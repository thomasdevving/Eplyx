//! The Milestone 18 sync contract: what one run, counterexample or
//! reproduction record looks like on its way to a cloud workspace, and the
//! checks both the CLI (before upload) and the server (on receipt) apply.
//!
//! Documents carry exact artifact bytes plus their SHA-256, so the cloud binds
//! to the same digests the local replay uses. Nothing here grants evidence:
//! a synced result is a copy of what the local engine concluded.
use super::privacy;
use crate::{
    dashboard::{
        store::{Store, CAPTURE_LIMIT},
        view::{self, RunBytes},
    },
    local_store::{is_safe_id, replay_inputs, Reproduction, SavedMigrationCounterexample},
    replay::hash_bytes as sha256,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const RUN_SCHEMA: &str = "eplyx.sync.run.v1";
pub const COUNTEREXAMPLE_SCHEMA: &str = "eplyx.sync.counterexample.v1";
pub const REPRODUCTION_SCHEMA: &str = "eplyx.sync.reproduction.v1";

/// Request body bounds. JSON string escaping can grow artifact text, so these
/// sit above the sum of the member bounds below.
pub const MAX_RUN_BODY: usize = 12 * 1024 * 1024;
pub const MAX_COUNTEREXAMPLE_BODY: usize = 3 * 1024 * 1024;
pub const MAX_REPRODUCTION_BODY: usize = 64 * 1024;

const METADATA_LIMIT: usize = 64 * 1024;
/// The package preflight worker's own bound for a report handoff.
const REPORT_LIMIT: usize = 4 * 1024 * 1024;
/// The engine reads bindings with a 32 KiB bound.
const BINDINGS_LIMIT: usize = 32 * 1024;
const SEARCH_LIMIT: usize = 4 * 1024 * 1024;
const COUNTEREXAMPLE_LIMIT: usize = 1024 * 1024;
const REPRODUCTION_LIMIT: usize = 16 * 1024;

/// Plain-language statement of the default sync scope, shown by the CLI.
pub const WHAT_IS_SYNCED: &str = "Eplyx sync uploads exact UTF-8 bytes and SHA-256 for completed run metadata, report, available bindings, ChangeSpec, state descriptor and search, plus saved counterexamples and reproduction records. Public addresses in those files are included. It never uploads source code, program binaries, raw captures, report.md, eplyx.toml, environment variables, provider URLs, absolute machine paths or credentials. Leaky artifacts are refused without rewriting. Viewing a synced result does not execute an analysis or create new evidence.";

/// Exact UTF-8 bytes of one small artifact and their SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub sha256: String,
    pub text: String,
}

impl Artifact {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let text = String::from_utf8(bytes).context("artifact is not UTF-8")?;
        Ok(Self {
            sha256: sha256(text.as_bytes()),
            text,
        })
    }

    fn verify(&self, what: &str, limit: usize) -> Result<()> {
        ensure!(self.text.len() <= limit, "{what} exceeds its sync bound");
        ensure!(
            sha256(self.text.as_bytes()) == self.sha256,
            "{what} SHA-256 does not match its bytes"
        );
        privacy::scan(what, &self.text)?;
        let decoded: Value = serde_json::from_str(&self.text).context("artifact is not JSON")?;
        privacy::scan_json(what, &decoded)
    }
}

pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Cloud workspace/project IDs: a fixed prefix and 20 lowercase hex digits.
pub fn is_cloud_id(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|rest| {
        rest.len() == 20
            && rest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDocument {
    pub schema: String,
    pub local_project_id: String,
    pub run_id: String,
    pub metadata: Artifact,
    pub report: Artifact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_spec: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_input: Option<Artifact>,
    /// Present once `eplyx search` has run for this run. It may be attached to
    /// a synced run exactly once and never changed.
    #[serde(default)]
    pub search: Option<Artifact>,
    /// Sizes of allowlisted members that stay on the machine, for display
    /// only. Never part of the run identity.
    #[serde(default)]
    pub local_artifact_sizes: BTreeMap<String, Option<u64>>,
}

/// A run document that passed every check, with the engine's own summary.
pub struct VerifiedRun {
    pub core_sha256: String,
    pub search_sha256: Option<String>,
    pub metadata: Value,
    pub run: view::Run,
    pub summary: Value,
}

impl RunDocument {
    /// Identity of the immutable run: its IDs and the digests of every member
    /// except the append-only search attachment and the display-only sizes.
    pub fn core_sha256(&self) -> Result<String> {
        crate::canonical::digest(&json!({
            "schema":self.schema,"local_project_id":self.local_project_id,"run_id":self.run_id,
            "metadata_sha256":self.metadata.sha256,"report_sha256":self.report.sha256,
            "bindings_sha256":self.bindings.as_ref().map(|a| &a.sha256),
            "change_spec_sha256":self.change_spec.as_ref().map(|a| &a.sha256),
            "state_input_sha256":self.state_input.as_ref().map(|a| &a.sha256)
        }))
    }

    pub fn view(&self) -> view::Run {
        view::from_bytes(
            &self.run_id,
            RunBytes {
                metadata: Some(self.metadata.text.as_bytes()),
                report: Some(self.report.text.as_bytes()),
                bindings: self.bindings.as_ref().map(|a| a.text.as_bytes()),
                change_spec: self.change_spec.as_ref().map(|a| a.text.as_bytes()),
                state_input: self.state_input.as_ref().map(|a| a.text.as_bytes()),
                search: self.search.as_ref().map(|a| a.text.as_bytes()),
            },
        )
    }

    pub fn verify(&self) -> Result<VerifiedRun> {
        ensure!(self.schema == RUN_SCHEMA, "unsupported run document schema");
        ensure!(is_safe_id(&self.run_id, "run_"), "invalid local run ID");
        ensure!(
            is_safe_id(&self.local_project_id, "local_"),
            "invalid local project ID"
        );
        self.metadata.verify("metadata.json", METADATA_LIMIT)?;
        self.report.verify("report.json", REPORT_LIMIT)?;
        for (artifact, name, bound) in [
            (&self.bindings, "bindings.json", BINDINGS_LIMIT),
            (&self.change_spec, "change_spec.json", 1024 * 1024),
            (&self.state_input, "state_input.json", 1024 * 1024),
            (&self.search, "migration-search.json", SEARCH_LIMIT),
        ] {
            if let Some(a) = artifact {
                a.verify(name, bound)?;
            }
        }
        ensure!(
            self.local_artifact_sizes.len() <= view::ARTIFACTS.len() + 1
                && self
                    .local_artifact_sizes
                    .keys()
                    .all(|n| view::artifact(n).is_some() || n == "world.json"),
            "unknown local artifact name"
        );
        let metadata: Value =
            serde_json::from_str(&self.metadata.text).context("invalid metadata.json")?;
        let report: Value =
            serde_json::from_str(&self.report.text).context("invalid report.json")?;
        let run = self.view();
        ensure!(
            view::state(&run) == "Complete",
            "run is incomplete or inconsistent"
        );
        if run.is_migration() {
            ensure!(
                report["official_transition"] == "NotTested" && report["funds_moved"] == false,
                "report claims an official transition or moved funds"
            );
        } else {
            ensure!(
                self.search.is_none() && self.bindings.is_none(),
                "non-migration records cannot claim migration search or bindings"
            );
        }
        let summary = view::summary(&run);
        Ok(VerifiedRun {
            core_sha256: self.core_sha256()?,
            search_sha256: self.search.as_ref().map(|a| a.sha256.clone()),
            metadata,
            run,
            summary,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterexampleDocument {
    pub schema: String,
    pub local_project_id: String,
    pub counterexample_id: String,
    /// The saved `.eplyx/counterexamples/<id>.json` file, byte for byte.
    pub file: Artifact,
}

pub struct VerifiedCounterexample {
    pub saved: SavedMigrationCounterexample,
    pub summary: Value,
}

impl CounterexampleDocument {
    /// Checks that need no parent run: bounds, digest, privacy and the
    /// content-addressed `cx_` identity recomputed by the engine.
    pub fn verify(&self) -> Result<VerifiedCounterexample> {
        ensure!(
            self.schema == COUNTEREXAMPLE_SCHEMA,
            "unsupported counterexample document schema"
        );
        ensure!(
            is_safe_id(&self.local_project_id, "local_"),
            "invalid local project ID"
        );
        ensure!(
            is_safe_id(&self.counterexample_id, "cx_"),
            "invalid counterexample ID"
        );
        self.file
            .verify("saved counterexample", COUNTEREXAMPLE_LIMIT)?;
        let saved: SavedMigrationCounterexample =
            serde_json::from_str(&self.file.text).context("invalid saved counterexample")?;
        ensure!(
            saved.schema_version == 1
                && saved.kind == crate::local_store::MIGRATION_COUNTEREXAMPLE_KIND
                && saved.id == self.counterexample_id
                && crate::migration::search::counterexample_id(&saved.counterexample)?
                    == self.counterexample_id
                && is_safe_id(&saved.parent_run, "run_")
                && valid_digest(&saved.search_sha256)
                && saved.replay_inputs == replay_inputs(&saved.parent_run),
            "counterexample identity mismatch"
        );
        let summary =
            crate::dashboard::migration::counterexample_fields(&saved, &self.counterexample_id);
        ensure!(
            summary["state"] == "Valid",
            "counterexample identity mismatch"
        );
        Ok(VerifiedCounterexample { saved, summary })
    }
}

/// A counterexample belongs to exactly one synced run: the run's saved search
/// must have the recorded digest and contain this exact engine counterexample.
pub fn bind_counterexample(saved: &SavedMigrationCounterexample, parent: &view::Run) -> Result<()> {
    ensure!(
        parent.id() == saved.parent_run,
        "counterexample belongs to a different run"
    );
    ensure!(
        parent.search_sha256() == Some(saved.search_sha256.as_str()),
        "counterexample search digest does not match the synced run's search"
    );
    let present = parent
        .migration_search()
        .context("parent has no migration search")?
        .counterexamples
        .contains(&saved.counterexample);
    ensure!(
        present,
        "counterexample is absent from the parent run's search result"
    );
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReproductionDocument {
    pub schema: String,
    pub local_project_id: String,
    pub reproduction_id: String,
    pub counterexample_id: String,
    /// The reproduction record, byte for byte. Leaky text is refused.
    pub file: Artifact,
}

impl ReproductionDocument {
    pub fn verify(&self) -> Result<Reproduction> {
        ensure!(
            self.schema == REPRODUCTION_SCHEMA,
            "unsupported reproduction document schema"
        );
        ensure!(
            is_safe_id(&self.local_project_id, "local_")
                && is_safe_id(&self.reproduction_id, "repro_")
                && is_safe_id(&self.counterexample_id, "cx_"),
            "invalid reproduction IDs"
        );
        self.file
            .verify("reproduction record", REPRODUCTION_LIMIT)?;
        let record: Reproduction =
            serde_json::from_str(&self.file.text).context("invalid reproduction record")?;
        ensure!(
            record.id == self.reproduction_id
                && record.counterexample_id == self.counterexample_id
                && record.schema_version == 1
                && record
                    .id
                    .ends_with(self.counterexample_id.trim_start_matches("cx_"))
                && record
                    .parent_run
                    .as_deref()
                    .is_none_or(|run| is_safe_id(run, "run_"))
                && record.search_sha256.as_deref().is_none_or(valid_digest),
            "reproduction identity mismatch"
        );
        Ok(record)
    }
}

/// A reproduction record must name a synced counterexample, and any parent run
/// or search digest it recorded must be that counterexample's.
pub fn bind_reproduction(
    record: &Reproduction,
    saved: &SavedMigrationCounterexample,
) -> Result<()> {
    ensure!(
        record.counterexample_id == saved.id
            && record
                .parent_run
                .as_deref()
                .is_none_or(|run| run == saved.parent_run)
            && record
                .search_sha256
                .as_deref()
                .is_none_or(|digest| digest == saved.search_sha256.as_str()),
        "reproduction record is bound to a different counterexample"
    );
    use crate::{local_store::ReproductionOutcome, migration::search::Counterexample};
    if let Some(binding) = &record.binding {
        let (input, candidate, world, signature) = match &saved.counterexample {
            Counterexample::MigrationObserved {
                analysis_input_sha256,
                candidate_program_sha256,
                world_sha256,
                signature,
                ..
            }
            | Counterexample::MigrationDerived {
                analysis_input_sha256,
                candidate_program_sha256,
                world_sha256,
                signature,
                ..
            } => (
                analysis_input_sha256,
                candidate_program_sha256,
                world_sha256,
                signature,
            ),
        };
        ensure!(
            &binding.analysis_input_sha256 == input
                && &binding.candidate_program_sha256 == candidate
                && &binding.world_sha256 == world
                && binding.counterexample_kind == saved.counterexample.claim()
                && binding.finding == format!("{:?}", saved.counterexample.finding())
                && binding.expected_signature == serde_json::to_value(signature)?,
            "reproduction binding identities disagree"
        );
    }
    if record.outcome == ReproductionOutcome::Reproduced {
        let binding = record
            .binding
            .as_ref()
            .context("successful reproduction has no binding")?;
        ensure!(
            record.failure_signature_matched
                && record.no_rpc
                && record.error.is_none()
                && record.parent_run.as_deref() == Some(saved.parent_run.as_str())
                && record.search_sha256.as_deref() == Some(saved.search_sha256.as_str())
                && binding.reproduced_signature.as_ref() == Some(&binding.expected_signature)
                && binding.gate_outcome_with_finding.as_deref() == Some("Block"),
            "successful reproduction claims are inconsistent"
        );
    } else {
        ensure!(
            !record.failure_signature_matched,
            "failed reproduction claims a matching signature"
        );
    }
    Ok(())
}

/// Summary the dashboards show for one reproduction record.
pub fn reproduction_summary(record: &Reproduction) -> Value {
    let mut value = json!(record);
    value["state"] = json!("Valid");
    value
}

// ------------------------------------------------------------ local builders

fn member(store: &Store, parts: &[&str], limit: usize) -> Result<Option<Artifact>> {
    store
        .read(parts, limit as u64)?
        .map(Artifact::new)
        .transpose()
}

/// Build the document for one complete local run from guarded store reads.
pub fn run_document(store: &Store, local_project_id: &str, run_id: &str) -> Result<RunDocument> {
    ensure!(is_safe_id(run_id, "run_"), "invalid run ID");
    let required = |relative: &str, limit: usize| -> Result<Artifact> {
        let mut parts = vec!["runs", run_id];
        parts.extend(relative.split('/'));
        member(store, &parts, limit)?
            .with_context(|| format!("{run_id} has no {relative}; only complete runs sync"))
    };
    let metadata = required("metadata.json", METADATA_LIMIT)?;
    let report = required("result/report.json", REPORT_LIMIT)?;
    let optional = |relative: &str, limit: usize| -> Result<Option<Artifact>> {
        let mut parts = vec!["runs", run_id];
        parts.extend(relative.split('/'));
        member(store, &parts, limit)
    };
    let bindings = optional("result/bindings.json", BINDINGS_LIMIT)?;
    let change_spec = optional("input/change.json", 1024 * 1024)?;
    let state_input = optional("input/state.json", 1024 * 1024)?;
    let search = optional(view::search_member(None), SEARCH_LIMIT)?;
    let mut local_artifact_sizes: BTreeMap<String, Option<u64>> = view::ARTIFACTS
        .iter()
        .map(|(name, path, ..)| {
            let mut parts = vec!["runs", run_id];
            parts.extend(path.split('/'));
            let size = store
                .open_file(&parts, CAPTURE_LIMIT)
                .ok()
                .flatten()
                .map(|(_, len)| len);
            ((*name).to_owned(), size)
        })
        .collect();
    local_artifact_sizes.insert(
        "world.json".into(),
        store
            .open_file(&["runs", run_id, "input", "world.json"], CAPTURE_LIMIT)
            .ok()
            .flatten()
            .map(|(_, n)| n),
    );
    Ok(RunDocument {
        schema: RUN_SCHEMA.into(),
        local_project_id: local_project_id.into(),
        run_id: run_id.into(),
        metadata,
        report,
        bindings,
        change_spec,
        state_input,
        search,
        local_artifact_sizes,
    })
}

pub fn counterexample_document(
    store: &Store,
    local_project_id: &str,
    id: &str,
) -> Result<CounterexampleDocument> {
    ensure!(is_safe_id(id, "cx_"), "invalid counterexample ID");
    let file = format!("{id}.json");
    let file = member(store, &["counterexamples", &file], COUNTEREXAMPLE_LIMIT)?
        .context("counterexample file missing")?;
    Ok(CounterexampleDocument {
        schema: COUNTEREXAMPLE_SCHEMA.into(),
        local_project_id: local_project_id.into(),
        counterexample_id: id.into(),
        file,
    })
}

/// Build an exact reproduction document. Retain the source call shape, but
/// never rewrite error text: the approved MAIN sync contract refuses leaks.
pub fn reproduction_document(
    store: &Store,
    local_project_id: &str,
    id: &str,
    _roots: &[(String, &str)],
) -> Result<ReproductionDocument> {
    ensure!(is_safe_id(id, "repro_"), "invalid reproduction ID");
    let file = format!("{id}.json");
    let bytes = store
        .read(&["reproductions", &file], REPRODUCTION_LIMIT as u64)?
        .context("reproduction record missing")?;
    let record: Reproduction =
        serde_json::from_slice(&bytes).context("invalid reproduction record")?;
    ensure!(record.id == id, "reproduction record ID mismatch");
    let file = Artifact::new(bytes)?;
    file.verify("reproduction record", REPRODUCTION_LIMIT)?;
    Ok(ReproductionDocument {
        schema: REPRODUCTION_SCHEMA.into(),
        local_project_id: local_project_id.into(),
        reproduction_id: id.into(),
        counterexample_id: record.counterexample_id.clone(),
        file,
    })
}

/// MAIN opaque project identities, minted by the server. Local project IDs stay distinct.
pub fn is_project_id(id: &str) -> bool {
    id.strip_prefix("proj_").is_some_and(|s| {
        s.len() == 26
            && s.bytes()
                .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
    })
}
