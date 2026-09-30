//! Durable offline analysis jobs. Observation happens before a job is accepted;
//! this module's input contract contains byte identities, never an RPC client,
//! credential, caller-supplied transaction or claimed execution status.
use crate::artifacts::ArtifactRef;
use chrono::{DateTime, Utc};
use eplyx_engine::migration::gate::Policy;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    ProtocolParameterChange {
        change: ArtifactRef,
        capture: ArtifactRef,
        parent_run: String,
    },
    CurrentStress {
        change: ArtifactRef,
        state_input: ArtifactRef,
        candidate: ArtifactRef,
        files: std::collections::BTreeMap<String, ArtifactRef>,
        budget: eplyx_engine::migration::population_types::StressBudget,
        parent_observation: String,
        wallet_sha256: String,
        stress_id: String,
        candidate_run_id: String,
    },
    CurrentCandidate {
        capture: ArtifactRef,
        candidate: ArtifactRef,
        change: Option<ArtifactRef>,
        parent_observation: String,
        wallet_sha256: String,
        check_id: String,
    },
    CurrentPreflight {
        bundle: ArtifactRef,
        candidate: Option<ArtifactRef>,
        parent_observation: String,
        wallet_sha256: String,
        preflight_id: String,
        scenario_sha256: String,
    },
    CurrentObservation {
        capture: ArtifactRef,
    },
    CurrentPath {
        capture: ArtifactRef,
        parent_observation: String,
        wallet_sha256: String,
        check_id: String,
    },
    MigrationOrder {
        change: ArtifactRef,
        world: ArtifactRef,
        candidate: ArtifactRef,
        parent_run: String,
        parent_input: ArtifactRef,
        parent_projection: ArtifactRef,
        source_a: String,
        source_b: String,
    },
    TokenMigration {
        change: ArtifactRef,
        state_input: ArtifactRef,
        /// The descriptor selects exactly fixture.json or world.json.
        state_artifact: ArtifactRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program_capture: Option<ArtifactRef>,
        candidate: ArtifactRef,
        policy: Policy,
    },
    LifecycleChange {
        change: ArtifactRef,
        snapshot: ArtifactRef,
        scenario: ArtifactRef,
        #[serde(default)]
        evidence: std::collections::BTreeMap<String, ArtifactRef>,
        before: DateTime<Utc>,
        at: DateTime<Utc>,
    },
}
impl Input {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ProtocolParameterChange { .. } => "protocol_parameter_change",
            Self::CurrentStress { .. } => "current_stress",
            Self::CurrentCandidate { .. } => "current_candidate",
            Self::CurrentPreflight { .. } => "current_preflight",
            Self::MigrationOrder { .. } => "migration_order",
            Self::TokenMigration { .. } => "token_migration",
            Self::LifecycleChange { .. } => "lifecycle_change",
            Self::CurrentObservation { .. } => "current_observation",
            Self::CurrentPath { .. } => "current_path",
        }
    }
    pub fn change(&self) -> Option<&ArtifactRef> {
        match self {
            Self::ProtocolParameterChange { change, .. } => Some(change),
            Self::CurrentStress { change, .. } => Some(change),
            Self::CurrentCandidate { change, .. } => change.as_ref(),
            Self::MigrationOrder { change, .. }
            | Self::TokenMigration { change, .. }
            | Self::LifecycleChange { change, .. } => Some(change),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_key: Option<String>,
    pub kind: String,
    /// Immutable input manifest in MAIN's document CAS. Every member is also
    /// retained and hash-verified before acceptance and again before execution.
    pub input: ArtifactRef,
    /// Result projection, filled exactly once when execution completes.
    pub projection: Option<ArtifactRef>,
}

pub mod catalogue;
pub mod observation;
pub mod order;
pub mod parameter;
mod process;
pub mod proposal;
pub mod stress;
pub mod upgrade;
pub mod worker;

use crate::{
    projection::Projection,
    registry::{now_unix_seconds, ChangeOrigin, Registry, RunChange, RunMetadata, RunStatus},
};
use anyhow::{ensure, Context, Result};
impl Registry {
    pub fn hosted_input(&self, metadata: &RunMetadata) -> Result<Input> {
        let job = metadata
            .hosted_analysis
            .as_ref()
            .context("run is not a hosted analysis")?;
        let input: Input = serde_json::from_slice(&self.document_bytes(&job.input)?)?;
        ensure!(input.kind() == job.kind, "job kind mismatch");
        Ok(input)
    }
    /// All inputs are in the authoritative CAS before the durable queue entry.
    pub fn create_hosted_analysis(&self, project: &str, input: Input) -> Result<RunMetadata> {
        self.create_hosted_analysis_with_key(project, input, None)
    }
    pub fn hosted_request(&self, project: &str, key: &str) -> Result<Option<RunMetadata>> {
        for id in self.project_run_ids(project)? {
            if !self.storage().run_dir(&id)?.join("metadata.json").exists() {
                continue;
            }
            let record = self.load_run(&id)?;
            if record.project_id == project
                && record
                    .hosted_analysis
                    .as_ref()
                    .is_some_and(|j| j.request_key.as_deref() == Some(key))
            {
                return Ok(Some(record));
            }
        }
        Ok(None)
    }
    pub fn create_hosted_analysis_with_key(
        &self,
        project: &str,
        input: Input,
        request_key: Option<String>,
    ) -> Result<RunMetadata> {
        let _guard = self.transitions.lock().unwrap_or_else(|h| h.into_inner());
        if let Some(key) = &request_key {
            ensure!(
                crate::storage::valid_id(key) && key.len() >= 16,
                "invalid request key"
            );
            if let Some(record) = self.hosted_request(project, key)? {
                ensure!(
                    serde_json::to_vec(&self.hosted_input(&record)?)?
                        == serde_json::to_vec(&input)?,
                    "request key conflict"
                );
                return Ok(record);
            }
        }
        let pending = self
            .project_run_ids(project)?
            .into_iter()
            .filter_map(|id| self.load_run(&id).ok())
            .filter(|r| !r.status.is_terminal())
            .count();
        ensure!(pending < 32, "project analysis queue is full");
        let project = self.load_project(project)?;
        ensure!(
            project.status != crate::project::ProjectStatus::Disabled,
            "project is disabled"
        );
        if let Input::MigrationOrder { parent_run, .. } = &input {
            ensure!(
                self.load_run(parent_run)?.project_id == project.project_id,
                "order parent project mismatch"
            );
        }
        if matches!(input, Input::ProtocolParameterChange { .. }) {
            parameter::validate_parent(self, &project.project_id, &input)?;
        }
        let validation = tempfile::tempdir()?;
        worker::stage(self, &input, validation.path())?;
        let spec = input
            .change()
            .map(|r| {
                self.document_bytes(r)
                    .and_then(|b| eplyx_engine::change::ChangeSpec::parse(&b))
            })
            .transpose()?;
        let change = spec
            .as_ref()
            .map(|s| RunChange::of(s, ChangeOrigin::Submitted))
            .transpose()?;
        let input_ref = self.document_ref(&serde_json::to_vec(&input)?)?;
        let candidate = match &input {
            Input::MigrationOrder { candidate, .. }
            | Input::TokenMigration { candidate, .. }
            | Input::CurrentCandidate { candidate, .. }
            | Input::CurrentStress { candidate, .. } => Some(candidate.clone()),
            _ => None,
        };
        if let Some(candidate) = &candidate {
            self.index_project_artifact(&project.project_id, &candidate.sha256)?;
        }
        let id = crate::ids::run();
        if let Some(spec) = &spec {
            self.save_change_spec(&id, spec)?;
        }
        let record = RunMetadata {
            order_failure: None,
            hosted_analysis: Some(Job {
                request_key,
                kind: input.kind().into(),
                input: input_ref,
                projection: None,
            }),
            analysis: None,
            run_id: id.clone(),
            project_id: project.project_id.clone(),
            status: RunStatus::Queued,
            bundle_sha256: String::new(),
            bundle_id: None,
            corpus_sha256: None,
            baseline_sha256: None,
            candidate_sha256: candidate
                .as_ref()
                .map(|c| c.sha256.clone())
                .unwrap_or_default(),
            candidate_artifact: candidate,
            change: change.clone(),
            expectations_sha256: None,
            attempts: vec![],
            adapter: None,
            adapter_version: None,
            semantic_schema_version: None,
            record_count: None,
            exit_code: None,
            report_available: false,
            detail: None,
            created_at_unix_seconds: now_unix_seconds(),
            started_at_unix_seconds: None,
            completed_at_unix_seconds: None,
        };
        self.index_run(&project.project_id, &id)?;
        if input.kind() != "migration_order" {
            if let Some(change) = &change {
                self.index_change(&project.project_id, &change.change_spec_id, &id)?;
            }
        }
        self.create_run(&record)?;
        Ok(record)
    }
    pub fn verify_hosted_projection(
        &self,
        record: &RunMetadata,
        projection: &Projection,
    ) -> Result<()> {
        let kind = projection.verified_kind()?;
        ensure!(
            projection.run_id == record.run_id,
            "projection run mismatch"
        );
        let input = self.hosted_input(record)?;
        ensure!(kind == input.kind(), "projection kind mismatch");
        if let Some(reference) = input.change() {
            let expected = self.document_bytes(reference)?;
            ensure!(
                projection
                    .change_spec
                    .as_ref()
                    .context("missing change")?
                    .text
                    .as_bytes()
                    == expected,
                "projection change mismatch"
            );
            let spec = eplyx_engine::change::ChangeSpec::parse(&expected)?;
            ensure!(
                record
                    .change
                    .as_ref()
                    .context("missing indexed change")?
                    .change_spec_id
                    == spec.id()?,
                "indexed change mismatch"
            );
        } else {
            ensure!(
                projection.change_spec.is_none() && record.change.is_none(),
                "observation cannot invent a proposal"
            );
        }
        ensure!(
            projection.search.is_none(),
            "execution cannot manufacture a search"
        );
        match input {
            Input::ProtocolParameterChange { .. } => {
                let report: serde_json::Value = serde_json::from_str(&projection.report.text)?;
                parameter::validate_parent(self, &record.project_id, &input)?;
                parameter::verify(self, &input, &report)?;
                let expected = RunChange::of(
                    &eplyx_engine::change::ChangeSpec::parse(
                        &self.document_bytes(input.change().context("missing change")?)?,
                    )?,
                    ChangeOrigin::Submitted,
                )?;
                ensure!(
                    record.change.as_ref() == Some(&expected),
                    "parameter index differs from proposal"
                );
            }
            Input::MigrationOrder { .. } => order::verify(self, record, projection)?,
            Input::CurrentStress {
                parent_observation,
                wallet_sha256,
                stress_id,
                files,
                candidate_run_id,
                ..
            } => {
                let report: serde_json::Value = serde_json::from_str(&projection.report.text)?;
                ensure!(
                    report["run_id"] == parent_observation
                        && report["stress_id"] == stress_id
                        && report["wallet_capture_sha256"] == wallet_sha256
                        && report["candidate_run_id"] == candidate_run_id
                        && report["report"]["population_capture_sha256"]
                            == files["population.capture.json"].sha256
                        && report["authorization"] == false
                        && report["funds_moved"] == false,
                    "stress projection binding mismatch"
                );
            }
            Input::CurrentCandidate {
                capture,
                candidate,
                parent_observation,
                wallet_sha256,
                check_id,
                ..
            } => {
                let report: serde_json::Value = serde_json::from_str(&projection.report.text)?;
                ensure!(
                    report["run_id"] == parent_observation
                        && report["check_id"] == check_id
                        && report["wallet_capture_sha256"] == wallet_sha256
                        && report["execution_capture_sha256"] == capture.sha256
                        && report["candidate_program_sha256"] == candidate.sha256
                        && report["official_transition"] == "NotTested"
                        && report["authorization"] == false
                        && report["funds_moved"] == false,
                    "candidate projection binding mismatch"
                );
            }
            Input::CurrentPreflight {
                bundle,
                parent_observation,
                wallet_sha256,
                preflight_id,
                scenario_sha256,
                ..
            } => {
                let report: serde_json::Value = serde_json::from_str(&projection.report.text)?;
                ensure!(
                    report["run_id"] == parent_observation
                        && report["preflight_id"] == preflight_id
                        && report["wallet_capture_sha256"] == wallet_sha256
                        && report["bundle_sha256"] == bundle.sha256
                        && report["scenario_sha256"] == scenario_sha256
                        && report["authorization"] == false
                        && report["funds_moved"] == false,
                    "preflight projection binding mismatch"
                );
            }
            Input::CurrentObservation { capture } => {
                let frozen: eplyx_engine::lifecycle::current::Capture = serde_json::from_slice(
                    &self
                        .artifacts()
                        .get(crate::artifacts::ArtifactClass::Capture, &capture)?,
                )?;
                let expected = eplyx_engine::canonical::document(
                    &eplyx_engine::lifecycle::current::evaluate(&frozen)?,
                )?;
                ensure!(
                    projection.report.text == expected,
                    "current observation result mismatch"
                );
            }
            Input::CurrentPath {
                capture,
                parent_observation,
                wallet_sha256,
                check_id,
            } => {
                let report: serde_json::Value = serde_json::from_str(&projection.report.text)?;
                ensure!(
                    report["run_id"] == parent_observation
                        && report["check_id"] == check_id
                        && report["wallet_capture_sha256"] == wallet_sha256
                        && report["execution_capture_sha256"] == capture.sha256,
                    "current path binding mismatch"
                );
                ensure!(
                    report["funds_moved"] == false
                        && report["authorization"] == false
                        && report["signer_possession_known"] == false,
                    "invalid current execution scope"
                );
            }
            Input::TokenMigration {
                state_input,
                candidate,
                ..
            } => {
                ensure!(
                    projection
                        .state_input
                        .as_ref()
                        .context("missing state")?
                        .text
                        .as_bytes()
                        == self.document_bytes(&state_input)?,
                    "projection state mismatch"
                );
                if let Some(reference) = &projection.migration_world {
                    let world: eplyx_engine::migration::world::World = serde_json::from_slice(
                        &self
                            .artifacts()
                            .get(crate::artifacts::ArtifactClass::Capture, reference)?,
                    )?;
                    world.validate()?;
                    let report: serde_json::Value = serde_json::from_str(&projection.report.text)?;
                    ensure!(
                        report["coverage"]["world"]["world_sha256"] == world.sha256()?,
                        "parent retained world mismatch"
                    );
                }
                let metadata: eplyx_engine::local_store::Metadata =
                    serde_json::from_str(&projection.metadata.text)?;
                ensure!(
                    metadata.candidate_program_sha256 == candidate.sha256,
                    "projection candidate mismatch"
                );
            }
            Input::LifecycleChange {
                snapshot,
                before,
                at,
                ..
            } => {
                let descriptor: serde_json::Value = serde_json::from_str(
                    &projection
                        .state_input
                        .as_ref()
                        .context("missing state")?
                        .text,
                )?;
                let frozen: eplyx_engine::lifecycle::LifecycleSnapshot = serde_json::from_slice(
                    &self
                        .artifacts()
                        .get(crate::artifacts::ArtifactClass::Capture, &snapshot)?,
                )?;
                ensure!(
                    descriptor["snapshot_sha256"]
                        == eplyx_engine::replay::hash_bytes(frozen.to_json()?.as_bytes())
                        && descriptor["before"] == serde_json::to_value(before)?
                        && descriptor["after"] == serde_json::to_value(at)?,
                    "projection observation mismatch"
                );
            }
        }
        Ok(())
    }
    pub(crate) fn completed_hosted_projection(&self, record: &RunMetadata) -> Option<Projection> {
        let reference: ArtifactRef = self
            .storage()
            .read_json(
                &self
                    .storage()
                    .run_dir(&record.run_id)
                    .ok()?
                    .join("completed-projection.json"),
            )
            .ok()?;
        let projection = serde_json::from_slice(&self.document_bytes(&reference).ok()?).ok()?;
        self.verify_hosted_projection(record, &projection).ok()?;
        Some(projection)
    }
}
