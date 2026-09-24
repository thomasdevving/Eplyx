//! What proposed change is being evaluated.
//!
//! A [`ChangeSpec`] describes a proposal and nothing else: which on-chain
//! identity it targets, which exact artefact or value it proposes, and under
//! what activation conditions. It carries no simulation result, no finding and
//! no proof strategy. The evaluation relation is
//!
//! ```text
//! baseline world (bundle: proven PRE state + executable) + ChangeSpec → candidate world
//! ```
//!
//! and the two sides never share an object. The bundle stays the authority for
//! baseline bytes and state; a content-addressed store or an explicitly
//! supplied file stays the authority for candidate bytes. The spec only names
//! them by hash, and every consumer resolves and re-verifies.
//!
//! # Identity
//!
//! `change_spec_id` is the SHA-256 of the canonical JSON of
//! `("eplyx-change-spec-v1", schema_version, change, activation)`. Everything
//! that alters what would be executed or when it would take effect is inside
//! that tuple: the kind, the target, the candidate artefact, every expectation
//! about the target, and the activation conditions. [`ChangeMetadata`] — a
//! display label and a free-text source — is deliberately outside it, so
//! renaming a proposal never makes it a different proposal.
//!
//! Canonical JSON here is serde's output for these types: struct fields in
//! declaration order, no maps. Reordering or renaming a field is therefore an
//! identity break, and `a_frozen_spec_keeps_its_identity` pins one ID so the
//! break cannot happen silently. Every optional identifying field is skipped
//! when absent, which is what lets a later field (a governance binding, say)
//! be added without re-identifying every spec that does not use it.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::replay::hash_bytes;
use crate::universal::evidence::{EvidenceKind, EvidenceRef, EvidenceStore};

pub const CHANGE_SPEC_SCHEMA: u32 = 1;
const IDENTITY_DOMAIN: &str = "eplyx-change-spec-v1";

/// A proposed change, as the stable input contract between whoever proposes it
/// (a developer, CI, a governance flow, the API) and the engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSpec {
    pub schema_version: u32,
    /// Recomputed on every load. Present in a file only as a commitment: a
    /// value that disagrees with the fields beside it is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_spec_id: Option<String>,
    pub change: Change,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<Activation>,
    #[serde(default, skip_serializing_if = "ChangeMetadata::is_empty")]
    pub metadata: ChangeMetadata,
}

/// What is changing. The target lives inside each kind, not beside it, because
/// what a change targets differs by kind: a program for an upgrade, an asset
/// for a lifecycle transition.
///
/// Unknown fields are refused rather than dropped: a misspelled expectation
/// that vanished on parse would leave the identity of a proposal that no
/// longer says what its author wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    ProgramUpgrade {
        target: ProgramTarget,
        candidate: ExecutableArtifact,
        /// The executable this upgrade replaces. When stated, the baseline the
        /// bundle proves must be exactly this artefact.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replaces: Option<ExecutableArtifact>,
        /// The upgrade authority the proposer expects. When stated, every
        /// baseline observation must prove this authority.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_upgrade_authority: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    ProgramUpgrade,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProgramUpgrade => "program_upgrade",
        }
    }
}

/// The program being changed. Deliberately separate from the bytes proposed
/// for it: a buffer, a multisig transaction or a staged deployment can all
/// name the same target with different artefacts, and the same artefact can
/// be proposed for different targets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramTarget {
    pub program_id: String,
    /// When stated, the bundle must prove the target keeps its bytes in this
    /// ProgramData account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub programdata_address: Option<String>,
}

/// An executable named by content. Never by filename.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableArtifact {
    pub sha256: String,
    pub len: u64,
}

impl ExecutableArtifact {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            sha256: hash_bytes(bytes),
            len: bytes.len() as u64,
        }
    }

    fn validate(&self, what: &str) -> Result<()> {
        ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "{what} sha256 must be 64 lowercase hex characters"
        );
        ensure!(self.len > 0, "{what} is empty");
        Ok(())
    }
}

/// When a change takes effect. Generic across kinds; kind-specific timing such
/// as a claim deadline belongs to the kind.
///
/// Part of identity. The CI gate does not condition on it: a corpus is
/// historical, and replaying a candidate over it is counterfactual whenever
/// the change would activate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unix_timestamp: Option<i64>,
}

/// Cosmetic and provenance fields. Outside identity by design.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Where the proposal came from, in free text. Never a filename the
    /// identity could depend on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl ChangeMetadata {
    pub fn is_empty(&self) -> bool {
        self.label.is_none() && self.source.is_none()
    }
}

fn canonical_address(value: &str, what: &str) -> Result<()> {
    let parsed: solana_address::Address = value
        .parse()
        .map_err(|_| anyhow::anyhow!("{what} {value:?} is not a base58 address"))?;
    // One encoding per key, or two spellings of one target would be two IDs.
    ensure!(
        parsed.to_string() == value,
        "{what} {value:?} is not canonically encoded"
    );
    Ok(())
}

impl ChangeSpec {
    /// The spec `--candidate <ELF>` stands for: upgrade the bundle's program to
    /// these bytes, asserting nothing else. Deliberately minimal, so the same
    /// bytes proposed for the same program have one identity no matter which
    /// bundle, or which schema of bundle, evaluates them.
    pub fn program_upgrade(program_id: &str, candidate: &[u8]) -> Self {
        Self {
            schema_version: CHANGE_SPEC_SCHEMA,
            change_spec_id: None,
            change: Change::ProgramUpgrade {
                target: ProgramTarget {
                    program_id: program_id.to_string(),
                    programdata_address: None,
                },
                candidate: ExecutableArtifact::of(candidate),
                replaces: None,
                expected_upgrade_authority: None,
            },
            activation: None,
            metadata: ChangeMetadata::default(),
        }
    }

    pub fn kind(&self) -> ChangeKind {
        match self.change {
            Change::ProgramUpgrade { .. } => ChangeKind::ProgramUpgrade,
        }
    }

    /// The deterministic identity. Computed, never trusted from input.
    pub fn id(&self) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            IDENTITY_DOMAIN,
            self.schema_version,
            &self.change,
            &self.activation,
        ))?))
    }

    /// Well-formedness alone. Whether the proposal fits a baseline is
    /// [`ChangeSpec::bind`]'s question.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == CHANGE_SPEC_SCHEMA,
            "change spec schema {} is not supported; this build reads {CHANGE_SPEC_SCHEMA}",
            self.schema_version
        );
        match &self.change {
            Change::ProgramUpgrade {
                target,
                candidate,
                replaces,
                expected_upgrade_authority,
            } => {
                canonical_address(&target.program_id, "target program")?;
                if let Some(address) = &target.programdata_address {
                    canonical_address(address, "target ProgramData")?;
                }
                candidate.validate("candidate executable")?;
                if let Some(replaced) = replaces {
                    replaced.validate("replaced executable")?;
                }
                if let Some(authority) = expected_upgrade_authority {
                    canonical_address(authority, "expected upgrade authority")?;
                }
            }
        }
        if let Some(activation) = &self.activation {
            ensure!(
                activation.slot.is_some() || activation.unix_timestamp.is_some(),
                "activation states neither a slot nor a time"
            );
        }
        if let Some(recorded) = &self.change_spec_id {
            let derived = self.id()?;
            ensure!(
                recorded == &derived,
                "change spec states id {recorded} but its fields identify {derived}"
            );
        }
        Ok(())
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let spec: Self = serde_json::from_slice(bytes).context("parsing the change spec")?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("reading the change spec {}", path.display()))?;
        Self::parse(&bytes)
    }

    /// The file form: the spec with its identity committed beside it.
    pub fn to_document(&self) -> Result<String> {
        let mut document = self.clone();
        document.change_spec_id = Some(self.id()?);
        Ok(serde_json::to_string_pretty(&document)?)
    }

    fn candidate(&self) -> &ExecutableArtifact {
        match &self.change {
            Change::ProgramUpgrade { candidate, .. } => candidate,
        }
    }

    /// Check the proposal against the baseline a bundle proves.
    ///
    /// Every expectation the spec states must be proved by the baseline;
    /// evidence the bundle does not carry fails a stated expectation rather
    /// than satisfying it. Unstated expectations are not checked.
    pub fn bind(&self, baseline: &BaselineTarget) -> Result<ChangeBinding> {
        self.validate()?;
        let Change::ProgramUpgrade {
            target,
            candidate,
            replaces,
            expected_upgrade_authority,
        } = &self.change;
        ensure!(
            target.program_id == baseline.program_id,
            "change spec targets program {} but the bundle proves program {}",
            target.program_id,
            baseline.program_id
        );
        if let TargetEvidence::Proven(loader) = &baseline.loader {
            ensure!(
                *loader == TargetLoader::Upgradeable,
                "program {} is deployed under a non-upgradeable loader; a program upgrade cannot target it",
                baseline.program_id
            );
        }
        if let Some(expected) = &target.programdata_address {
            match &baseline.programdata_address {
                TargetEvidence::Proven(actual) => ensure!(
                    actual == expected,
                    "change spec names ProgramData {expected} but the bundle proves {actual}"
                ),
                TargetEvidence::Unproven => bail!(
                    "change spec names ProgramData {expected} but the bundle carries no ProgramData evidence for {}",
                    baseline.program_id
                ),
            }
        }
        if let Some(replaced) = replaces {
            ensure!(
                replaced == &baseline.executable,
                "change spec replaces executable {} ({} bytes) but the bundle baseline is {} ({} bytes)",
                replaced.sha256,
                replaced.len,
                baseline.executable.sha256,
                baseline.executable.len
            );
        }
        if let Some(expected) = expected_upgrade_authority {
            match &baseline.upgrade_authorities {
                TargetEvidence::Proven(observed) => ensure!(
                    observed.len() == 1 && observed.contains(&Some(expected.clone())),
                    "change spec expects upgrade authority {expected} but the baseline proves {observed:?}"
                ),
                TargetEvidence::Unproven => bail!(
                    "change spec expects upgrade authority {expected} but the bundle carries no authority evidence"
                ),
            }
        }
        Ok(ChangeBinding {
            change_spec_id: self.id()?,
            kind: self.kind(),
            target_program_id: target.program_id.clone(),
            candidate_sha256: candidate.sha256.clone(),
        })
    }

    /// Resolve the candidate bytes this spec names.
    ///
    /// The only way to obtain a [`ResolvedCandidate`], so execution can never
    /// run bytes the spec did not describe.
    pub fn resolve(&self, source: CandidateSource<'_>) -> Result<ResolvedCandidate> {
        self.validate()?;
        let expected = self.candidate();
        let bytes = match source {
            CandidateSource::Bytes(bytes) => bytes.to_vec(),
            CandidateSource::File(path) => std::fs::read(path)
                .with_context(|| format!("reading the candidate program {}", path.display()))?,
            CandidateSource::Store(root) => EvidenceStore::at(root)
                .get(&EvidenceRef {
                    kind: EvidenceKind::ProgramBinary,
                    sha256: expected.sha256.clone(),
                })
                .context("resolving the candidate executable from the artifact store")?,
        };
        let actual = ExecutableArtifact::of(&bytes);
        ensure!(
            &actual == expected,
            "candidate bytes are {} ({} bytes) but the change spec describes {} ({} bytes)",
            actual.sha256,
            actual.len,
            expected.sha256,
            expected.len
        );
        Ok(ResolvedCandidate {
            change_spec_id: self.id()?,
            bytes,
        })
    }
}

/// Where a spec's candidate bytes come from. Each is verified against the
/// spec's content hash; none is trusted by name.
#[derive(Clone, Copy, Debug)]
pub enum CandidateSource<'a> {
    Bytes(&'a [u8]),
    File(&'a Path),
    /// A content-addressed store in the evidence layout
    /// (`programs/<sha256>`).
    Store(&'a Path),
}

/// Candidate bytes proven to be the artefact a spec describes.
#[derive(Clone, Debug)]
pub struct ResolvedCandidate {
    change_spec_id: String,
    bytes: Vec<u8>,
}

impl ResolvedCandidate {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn change_spec_id(&self) -> &str {
        &self.change_spec_id
    }
}

/// What a bundle proves about the program a change would replace, in the
/// terms a spec can state expectations over. Built by the bundle's owner from
/// its own evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaselineTarget {
    pub program_id: String,
    pub executable: ExecutableArtifact,
    pub loader: TargetEvidence<TargetLoader>,
    pub programdata_address: TargetEvidence<String>,
    /// Every authority any observation proves, so a window across an
    /// authority change cannot satisfy an expectation of one of them.
    pub upgrade_authorities: TargetEvidence<BTreeSet<Option<String>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetEvidence<T> {
    Proven(T),
    Unproven,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TargetLoader {
    Upgradeable,
    NotUpgradeable,
}

/// The part of a spec a report carries, so a verdict always names what it is
/// a verdict about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeBinding {
    pub change_spec_id: String,
    pub kind: ChangeKind,
    pub target_program_id: String,
    pub candidate_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";
    const OTHER: &str = "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH";
    const PROGRAMDATA: &str = "7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP";
    const AUTHORITY: &str = "Ad21qwCb3C98M6UNqjGsZgR48549Spp7W1UWETV29cZ9";

    fn spec() -> ChangeSpec {
        ChangeSpec::program_upgrade(PROGRAM, b"candidate elf")
    }

    fn baseline() -> BaselineTarget {
        BaselineTarget {
            program_id: PROGRAM.into(),
            executable: ExecutableArtifact::of(b"baseline elf"),
            loader: TargetEvidence::Proven(TargetLoader::Upgradeable),
            programdata_address: TargetEvidence::Proven(PROGRAMDATA.into()),
            upgrade_authorities: TargetEvidence::Proven([Some(AUTHORITY.to_string())].into()),
        }
    }

    fn upgrade(
        spec: &mut ChangeSpec,
    ) -> (
        &mut ProgramTarget,
        &mut ExecutableArtifact,
        &mut Option<ExecutableArtifact>,
        &mut Option<String>,
    ) {
        let Change::ProgramUpgrade {
            target,
            candidate,
            replaces,
            expected_upgrade_authority,
        } = &mut spec.change;
        (target, candidate, replaces, expected_upgrade_authority)
    }

    /// The canonical encoding is part of the contract. If this fails, every
    /// stored spec and every report that names one now points elsewhere.
    #[test]
    fn a_frozen_spec_keeps_its_identity() {
        assert_eq!(
            spec().id().unwrap(),
            "b5a894cdbec6251f73b4224a294579fe1af9e316232949fa92da852468900bf3"
        );
    }

    #[test]
    fn the_same_semantic_spec_has_the_same_id() {
        let a = spec();
        let mut b = ChangeSpec::parse(a.to_document().unwrap().as_bytes()).unwrap();
        assert_eq!(a.id().unwrap(), b.id().unwrap());
        b.change_spec_id = None;
        assert_eq!(a.id().unwrap(), b.id().unwrap());
    }

    #[test]
    fn every_semantic_field_moves_the_id() {
        let base = spec().id().unwrap();
        let mut mutations: Vec<(&str, ChangeSpec)> = Vec::new();
        let mut s = spec();
        *upgrade(&mut s).1 = ExecutableArtifact::of(b"candidate elg");
        mutations.push(("candidate bytes", s));
        let mut s = spec();
        upgrade(&mut s).1.len += 1;
        mutations.push(("candidate length", s));
        let mut s = spec();
        upgrade(&mut s).0.program_id = OTHER.into();
        mutations.push(("target program", s));
        let mut s = spec();
        upgrade(&mut s).0.programdata_address = Some(PROGRAMDATA.into());
        mutations.push(("target ProgramData", s));
        let mut s = spec();
        *upgrade(&mut s).2 = Some(ExecutableArtifact::of(b"baseline elf"));
        mutations.push(("replaced executable", s));
        let mut s = spec();
        *upgrade(&mut s).3 = Some(AUTHORITY.into());
        mutations.push(("authority expectation", s));
        let mut s = spec();
        s.activation = Some(Activation {
            slot: Some(1),
            unix_timestamp: None,
        });
        mutations.push(("activation slot", s));
        let mut s = spec();
        s.activation = Some(Activation {
            slot: None,
            unix_timestamp: Some(1),
        });
        mutations.push(("activation time", s));
        let mut s = spec();
        s.schema_version += 1;
        mutations.push(("schema", s));
        let mut seen = BTreeSet::from([base]);
        for (what, mutated) in mutations {
            assert!(
                seen.insert(mutated.id().unwrap()),
                "{what} did not move the id"
            );
        }
    }

    #[test]
    fn cosmetic_metadata_does_not_move_the_id() {
        let mut labelled = spec();
        labelled.metadata = ChangeMetadata {
            label: Some("v2.1 release candidate".into()),
            source: Some("hand-written".into()),
        };
        assert_eq!(labelled.id().unwrap(), spec().id().unwrap());
        // The document keeps the label; it just does not identify.
        let document = labelled.to_document().unwrap();
        assert!(document.contains("v2.1 release candidate"));
        assert_eq!(ChangeSpec::parse(document.as_bytes()).unwrap(), {
            let mut expected = labelled.clone();
            expected.change_spec_id = Some(labelled.id().unwrap());
            expected
        });
    }

    #[test]
    fn a_stated_id_that_disagrees_is_refused() {
        let mut document: serde_json::Value =
            serde_json::from_str(&spec().to_document().unwrap()).unwrap();
        document["change"]["candidate"]["len"] = 999.into();
        let error = ChangeSpec::parse(document.to_string().as_bytes()).unwrap_err();
        assert!(format!("{error:#}").contains("identify"), "{error:#}");
    }

    #[test]
    fn unknown_fields_and_kinds_are_refused() {
        let document: serde_json::Value =
            serde_json::from_str(&spec().to_document().unwrap()).unwrap();
        for (path, value) in [
            (
                "/change/expected_upgrade_authorty",
                serde_json::json!(AUTHORITY),
            ),
            ("/change/target/loader", serde_json::json!("upgradeable")),
            ("/simulation", serde_json::json!({"passed": true})),
        ] {
            let mut mutated = document.clone();
            let (parent, key) = path.rsplit_once('/').unwrap();
            mutated
                .pointer_mut(if parent.is_empty() { "" } else { parent })
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(key.into(), value);
            assert!(
                ChangeSpec::parse(mutated.to_string().as_bytes()).is_err(),
                "{path} was accepted"
            );
        }
        let mut lifecycle = document.clone();
        lifecycle["change"]["kind"] = "lifecycle_change".into();
        assert!(ChangeSpec::parse(lifecycle.to_string().as_bytes()).is_err());
    }

    #[test]
    fn malformed_identities_are_refused() {
        let mut s = spec();
        upgrade(&mut s).1.sha256 = upgrade(&mut s).1.sha256.to_uppercase();
        assert!(s.validate().is_err(), "uppercase hash is a second spelling");
        let mut s = spec();
        upgrade(&mut s).0.program_id = "not-an-address".into();
        assert!(s.validate().is_err());
        let mut s = spec();
        upgrade(&mut s).1.len = 0;
        assert!(s.validate().is_err());
        let mut s = spec();
        s.activation = Some(Activation {
            slot: None,
            unix_timestamp: None,
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn binding_checks_every_stated_expectation() {
        let binding = spec().bind(&baseline()).unwrap();
        assert_eq!(binding.change_spec_id, spec().id().unwrap());
        assert_eq!(binding.kind, ChangeKind::ProgramUpgrade);
        assert_eq!(binding.target_program_id, PROGRAM);

        let mut wrong_target = spec();
        upgrade(&mut wrong_target).0.program_id = OTHER.into();
        assert!(wrong_target.bind(&baseline()).is_err());

        let mut stated = spec();
        upgrade(&mut stated).0.programdata_address = Some(PROGRAMDATA.into());
        *upgrade(&mut stated).2 = Some(ExecutableArtifact::of(b"baseline elf"));
        *upgrade(&mut stated).3 = Some(AUTHORITY.into());
        stated.bind(&baseline()).unwrap();

        let mut wrong_programdata = spec();
        upgrade(&mut wrong_programdata).0.programdata_address = Some(AUTHORITY.into());
        assert!(wrong_programdata.bind(&baseline()).is_err());

        let mut wrong_baseline = spec();
        *upgrade(&mut wrong_baseline).2 = Some(ExecutableArtifact::of(b"other baseline"));
        assert!(wrong_baseline.bind(&baseline()).is_err());

        let mut wrong_authority = spec();
        *upgrade(&mut wrong_authority).3 = Some(PROGRAMDATA.into());
        assert!(wrong_authority.bind(&baseline()).is_err());

        // A window spanning an authority change proves neither authority alone.
        let mut rotated = baseline();
        rotated.upgrade_authorities =
            TargetEvidence::Proven([Some(AUTHORITY.to_string()), None].into());
        assert!(stated.bind(&rotated).is_err());

        // Evidence the bundle does not carry never satisfies a stated expectation.
        let mut unproven = baseline();
        unproven.programdata_address = TargetEvidence::Unproven;
        unproven.upgrade_authorities = TargetEvidence::Unproven;
        assert!(stated.bind(&unproven).is_err());
        // An unstated one is not checked.
        spec().bind(&unproven).unwrap();

        let mut legacy = baseline();
        legacy.loader = TargetEvidence::Proven(TargetLoader::NotUpgradeable);
        assert!(spec().bind(&legacy).is_err());
    }

    #[test]
    fn resolution_verifies_bytes_against_the_spec() {
        let s = spec();
        assert_eq!(
            s.resolve(CandidateSource::Bytes(b"candidate elf"))
                .unwrap()
                .bytes(),
            b"candidate elf"
        );
        let error = s
            .resolve(CandidateSource::Bytes(b"candidate elg"))
            .unwrap_err();
        assert!(format!("{error:#}").contains("describes"), "{error:#}");

        let scratch = tempfile::tempdir().unwrap();
        let store = scratch.path().join("store");
        let missing = s.resolve(CandidateSource::Store(&store)).unwrap_err();
        assert!(
            format!("{missing:#}").contains("missing evidence object"),
            "{missing:#}"
        );
        EvidenceStore::at(&store)
            .put(EvidenceKind::ProgramBinary, b"candidate elf")
            .unwrap();
        let resolved = s.resolve(CandidateSource::Store(&store)).unwrap();
        assert_eq!(resolved.change_spec_id(), s.id().unwrap());

        // A store object whose bytes no longer match its address.
        let path = store
            .join("programs")
            .join(ExecutableArtifact::of(b"candidate elf").sha256);
        std::fs::write(&path, b"tampered").unwrap();
        assert!(s.resolve(CandidateSource::Store(&store)).is_err());

        let file = scratch.path().join("other.so");
        std::fs::write(&file, b"another build").unwrap();
        assert!(s.resolve(CandidateSource::File(&file)).is_err());
    }

    /// The top level carries nothing a program upgrade owns, so a second kind
    /// with a different target (an asset, say) fits beside it without
    /// inheriting program fields.
    #[test]
    fn the_top_level_is_kind_neutral() {
        let mut s = spec();
        s.activation = Some(Activation {
            slot: Some(1),
            unix_timestamp: Some(2),
        });
        s.metadata.label = Some("x".into());
        let document: serde_json::Value = serde_json::from_str(&s.to_document().unwrap()).unwrap();
        let top: BTreeSet<&str> = document
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            top,
            BTreeSet::from([
                "schema_version",
                "change_spec_id",
                "change",
                "activation",
                "metadata"
            ])
        );
        let activation: BTreeSet<&str> = document["activation"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(activation, BTreeSet::from(["slot", "unix_timestamp"]));
        for key in ["target", "candidate"] {
            assert!(document["change"].get(key).is_some());
        }
    }
}
