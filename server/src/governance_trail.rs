//! Retained hosted observations. No RPC, proof construction, or mutable latest state.
use std::{collections::BTreeSet, path::Path};

use anyhow::{ensure, Context, Result};
use eplyx_engine::{
    change::{ChangeSpec, Delivery},
    governance::{
        attestation::DeploymentAttestation, BindingOutcome, Commitment, GovernanceBinding,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    artifacts::canonical_sha256,
    registry::{GovernanceCheck, Registry},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationOccurrence {
    pub occurrence_id: String,
    pub project_id: String,
    pub change_spec_id: String,
    pub binding_id: String,
    pub attestation_id: String,
    pub recorded_at_unix_seconds: u64,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrailEvent {
    GovernanceCheck {
        event_id: String,
        recorded_at_unix_seconds: u64,
        check: GovernanceCheck,
        binding: Box<GovernanceBinding>,
    },
    DeploymentAttestation {
        event_id: String,
        recorded_at_unix_seconds: Option<u64>,
        legacy: bool,
        occurrence: Option<AttestationOccurrence>,
        attestation: Box<DeploymentAttestation>,
    },
}

#[derive(Serialize)]
pub struct GovernanceTrail {
    pub project_id: String,
    pub change_spec_id: String,
    pub source_unbound_change_spec_id: Option<String>,
    pub candidate: Value,
    pub target: Value,
    pub delivery: Value,
    pub runs: Vec<Value>,
    pub runs_next_cursor: Option<String>,
    pub events: Vec<TrailEvent>,
    pub next_cursor: Option<String>,
}

// The prefix does not order time: gchk and gocc share the same monotonic mint.
// Legacy comes last in hash order, outside the recorded chronology.
pub fn event_key(id: &str) -> Result<String> {
    if let Some(hash) = id.strip_prefix("legacy_") {
        ensure!(canonical_sha256(hash), "invalid legacy event id");
        return Ok(format!("1_{hash}"));
    }
    let suffix = id
        .strip_prefix("gchk_")
        .or_else(|| id.strip_prefix("gocc_"))
        .context("invalid hosted event id")?;
    ensure!(
        suffix.len() == 26
            && suffix
                .bytes()
                .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b)),
        "invalid hosted event id"
    );
    Ok(format!("0_{suffix}_{id}"))
}

fn ids(path: &Path) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|v| v == "json") {
            let id = path
                .file_stem()
                .and_then(|v| v.to_str())
                .context("invalid evidence filename")?;
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

impl Registry {
    // If a legacy seal is requested again, retain its unknown-time existence
    // as well as the new occurrence. Written only by POST, never on GET.
    pub(crate) fn retain_legacy_attestation(
        &self,
        project: &str,
        root: &str,
        attestation: &str,
    ) -> Result<()> {
        let dir = self.storage().project_governance_dir(project, root)?;
        for id in ids(&dir.join("attestation-occurrences"))? {
            let o: AttestationOccurrence = self.storage().read_json(
                &dir.join("attestation-occurrences")
                    .join(format!("{id}.json")),
            )?;
            if o.attestation_id == attestation {
                return Ok(());
            }
        }
        let marker = dir
            .join("legacy-attestations")
            .join(format!("{attestation}.json"));
        if !marker.exists() {
            self.storage().write_json(&marker, &attestation)?;
        }
        Ok(())
    }

    /// Resolve the exact seal, without a latest-check search or history cap.
    pub fn governance_binding(
        &self,
        project: &str,
        root: &str,
        id: &str,
    ) -> Result<GovernanceBinding> {
        ensure!(canonical_sha256(id), "invalid binding id");
        let dir = self.storage().project_governance_dir(project, root)?;
        let binding = GovernanceBinding::parse(
            &self
                .storage()
                .read_bytes(&dir.join("bindings").join(format!("{id}.json")))?,
        )?;
        ensure!(
            binding.binding_id.as_deref() == Some(id),
            "binding filename identity differs"
        );
        Ok(binding)
    }

    fn check_binding_root(&self, spec: &ChangeSpec, binding: &GovernanceBinding) -> Result<()> {
        let root = spec.id()?;
        ensure!(
            binding.analysed_change_spec_id == root
                || binding.bound_change_spec_id.as_deref() == Some(root.as_str()),
            "binding is unrelated to trail root"
        );
        // Recover the asked-about spec by replacing only delivery. This also
        // verifies source/derived IDs, including negative rechecks whose new
        // observed message derives a different bound ID.
        let asked =
            spec.with_delivery(binding.expected.delivery.clone().map(Delivery::SquadsV4))?;
        ensure!(
            asked.id()? == binding.analysed_change_spec_id,
            "binding source identity differs"
        );
        let upgrade = asked
            .as_program_upgrade()
            .context("trail requires program upgrade")?;
        let expected = &binding.expected;
        ensure!(
            expected.candidate == *upgrade.candidate
                && expected.target_program_id == upgrade.target.program_id
                && expected.programdata_address == upgrade.target.programdata_address
                && expected.replaces == *upgrade.replaces
                && expected.expected_upgrade_authority == *upgrade.expected_upgrade_authority,
            "binding expected candidate/target differs"
        );
        let derived = binding.bound_spec(&asked)?;
        ensure!(
            derived.is_some() || binding.bound_change_spec_id.is_none(),
            "binding claims an absent derived identity"
        );
        Ok(())
    }

    fn trail_attestation(
        &self,
        project: &str,
        spec: &ChangeSpec,
        id: &str,
    ) -> Result<DeploymentAttestation> {
        ensure!(canonical_sha256(id), "invalid attestation id");
        let root = spec.id()?;
        let dir = self.storage().project_governance_dir(project, &root)?;
        let a = DeploymentAttestation::parse(
            &self
                .storage()
                .read_bytes(&dir.join("attestations").join(format!("{id}.json")))?,
        )?;
        ensure!(
            a.attestation_id.as_deref() == Some(id) && a.change_spec_id == root,
            "attestation root/id differs"
        );
        let b = self.governance_binding(project, &root, &a.binding_id)?;
        self.check_binding_root(spec, &b)?;
        ensure!(
            b.outcome == BindingOutcome::Matched
                && b.commitment == Commitment::Finalized
                && a.commitment == Commitment::Finalized
                && b.bound_change_spec_id.as_deref() == Some(root.as_str()),
            "G2 requires its exact finalized matched G1 binding"
        );
        let Delivery::SquadsV4(d) = spec.delivery().context("missing Squads delivery")?;
        let u = b
            .observation
            .upgrade
            .as_ref()
            .context("matched binding has no Upgrade")?;
        let target = spec
            .as_program_upgrade()
            .context("not a program upgrade")?
            .target;
        ensure!(
            b.observation.delivery.as_ref() == Some(d)
                && b.expected.delivery.as_ref().is_none_or(|v| v == d)
                && b.request.multisig == d.multisig
                && b.request.transaction_index == d.transaction_index
                && a.candidate == b.expected.candidate
                && Some(&a.candidate) == spec.candidate()
                && a.target_program == target.program_id
                && a.target_program == u.program
                && a.programdata == u.programdata
                && target
                    .programdata_address
                    .as_ref()
                    .is_none_or(|v| v == &a.programdata)
                && a.multisig == d.multisig
                && a.transaction_index == d.transaction_index
                && a.proposal == d.proposal
                && a.vault_transaction == d.transaction
                && a.message_sha256 == d.message_sha256,
            "attestation candidate/target/proposal/message disagrees with its G1 binding"
        );
        Ok(a)
    }

    pub fn governance_trail(
        &self,
        project: &str,
        spec: &ChangeSpec,
        cursor: Option<&str>,
        run_cursor: Option<&str>,
        limit: usize,
    ) -> Result<GovernanceTrail> {
        // Observe a completed G2 publication, not a seal between its write and
        // occurrence write. This lock changes no durable state.
        let _guard = self.transitions.lock().unwrap_or_else(|e| e.into_inner());
        let limit = limit.clamp(1, 100);
        let root = spec.id()?;
        ensure!(
            spec.delivery().is_some(),
            "trail root must be governance-bound"
        );
        let dir = self.storage().project_governance_dir(project, &root)?;
        let after = cursor.map(event_key).transpose()?;
        let mut candidates = Vec::new();
        for id in ids(&dir.join("checks"))? {
            ensure!(id.starts_with("gchk_"), "invalid check filename");
            candidates.push((event_key(&id)?, id));
        }
        // Scan only this change's small occurrence metadata, never all project
        // history or all proof bodies. Legacy discovery needs the referenced IDs.
        let mut referenced = BTreeSet::new();
        for id in ids(&dir.join("attestation-occurrences"))? {
            let o: AttestationOccurrence = self.storage().read_json(
                &dir.join("attestation-occurrences")
                    .join(format!("{id}.json")),
            )?;
            ensure!(
                id.starts_with("gocc_")
                    && o.occurrence_id == id
                    && o.project_id == project
                    && o.change_spec_id == root
                    && canonical_sha256(&o.binding_id)
                    && canonical_sha256(&o.attestation_id),
                "occurrence index identity differs"
            );
            // Do not allow a missing reference to hide a legacy object.
            ensure!(
                dir.join("attestations")
                    .join(format!("{}.json", o.attestation_id))
                    .is_file(),
                "occurrence attestation is missing"
            );
            referenced.insert(o.attestation_id);
            candidates.push((event_key(&id)?, id));
        }
        let mut retained_legacy = BTreeSet::new();
        for id in ids(&dir.join("legacy-attestations"))? {
            let stated: String = self
                .storage()
                .read_json(&dir.join("legacy-attestations").join(format!("{id}.json")))?;
            ensure!(
                canonical_sha256(&id)
                    && stated == id
                    && dir
                        .join("attestations")
                        .join(format!("{id}.json"))
                        .is_file(),
                "legacy index identity differs"
            );
            retained_legacy.insert(id);
        }
        for id in ids(&dir.join("attestations"))? {
            ensure!(canonical_sha256(&id), "invalid attestation filename");
            if !referenced.contains(&id) || retained_legacy.contains(&id) {
                let id = format!("legacy_{id}");
                candidates.push((event_key(&id)?, id));
            }
        }
        candidates.sort();
        candidates.retain(|(key, _)| after.as_ref().is_none_or(|after| key > after));
        let more = candidates.len() > limit;
        candidates.truncate(limit);
        let next_cursor = more.then(|| candidates.last().unwrap().1.clone());
        let mut events = Vec::new();
        let mut source = None;
        for (_, id) in candidates {
            if id.starts_with("gchk_") {
                let check: GovernanceCheck = self
                    .storage()
                    .read_json(&dir.join("checks").join(format!("{id}.json")))?;
                ensure!(
                    check.check_id == id
                        && check.project_id == project
                        && check.change_spec_id == root,
                    "check index identity differs"
                );
                let binding = self.governance_binding(project, &root, &check.binding_id)?;
                self.check_binding_root(spec, &binding)?;
                if binding.expected.delivery.is_none() {
                    source = Some(binding.analysed_change_spec_id.clone());
                }
                events.push(TrailEvent::GovernanceCheck {
                    event_id: id,
                    recorded_at_unix_seconds: check.checked_at_unix_seconds,
                    check,
                    binding: Box::new(binding),
                });
            } else {
                let occurrence: Option<AttestationOccurrence> = if id.starts_with("gocc_") {
                    Some(
                        self.storage().read_json(
                            &dir.join("attestation-occurrences")
                                .join(format!("{id}.json")),
                        )?,
                    )
                } else {
                    None
                };
                let attestation_id = occurrence
                    .as_ref()
                    .map(|o| o.attestation_id.as_str())
                    .unwrap_or_else(|| id.strip_prefix("legacy_").unwrap());
                let attestation = self.trail_attestation(project, spec, attestation_id)?;
                if let Some(o) = &occurrence {
                    ensure!(
                        o.occurrence_id == id
                            && o.project_id == project
                            && o.change_spec_id == root
                            && o.binding_id == attestation.binding_id,
                        "occurrence points at another attestation binding"
                    );
                }
                events.push(TrailEvent::DeploymentAttestation {
                    event_id: id,
                    recorded_at_unix_seconds: occurrence
                        .as_ref()
                        .map(|o| o.recorded_at_unix_seconds),
                    legacy: occurrence.is_none(),
                    occurrence,
                    attestation: Box::new(attestation),
                });
            }
        }
        let mut run_ids = self.change_run_ids(project, &root)?;
        run_ids.retain(|id| run_cursor.is_none_or(|cursor| id.as_str() < cursor));
        let runs_more = run_ids.len() > limit;
        run_ids.truncate(limit);
        let runs_next_cursor = runs_more.then(|| run_ids.last().unwrap().clone());
        let mut runs = Vec::new();
        for id in run_ids {
            let run = self.load_run(&id)?;
            ensure!(
                run.run_id == id && run.project_id == project,
                "run index project differs"
            );
            let change = run.change.as_ref().context("indexed run has no change")?;
            ensure!(
                change.change_spec_id == root && self.load_change_spec(&id, change)?.id()? == root,
                "run index change differs"
            );
            runs.push(json!({"run_id": id, "status": run.status, "exit_code": run.exit_code,
                "candidate_sha256": run.candidate_sha256, "bundle_sha256": run.bundle_sha256,
                "created_at_unix_seconds": run.created_at_unix_seconds, "completed_at_unix_seconds": run.completed_at_unix_seconds}));
        }
        Ok(GovernanceTrail {
            project_id: project.into(),
            change_spec_id: root,
            source_unbound_change_spec_id: source,
            candidate: serde_json::to_value(spec.candidate())?,
            target: serde_json::to_value(spec.as_program_upgrade().context("not upgrade")?.target)?,
            delivery: serde_json::to_value(spec.delivery())?,
            runs,
            runs_next_cursor,
            events,
            next_cursor,
        })
    }
}
