//! Owned bytes for the shared analytical dashboard projection. Storage and
//! transport provenance live in the run registry, outside these engine bytes.
use anyhow::{ensure, Result};
use eplyx_engine::{
    cloud::contract::{Artifact, RunDocument},
    dashboard::view::{self, RunBytes},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A 128 MiB report plus JSON string escaping and bounded sidecars.
pub const MAX_PROJECTION_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    pub run_id: String,
    /// Retained offline migration runtime contract, outside engine artifact bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_world: Option<crate::artifacts::ArtifactRef>,
    pub metadata: Artifact,
    pub report: Artifact,
    pub change_spec: Option<Artifact>,
    pub state_input: Option<Artifact>,
    pub bindings: Option<Artifact>,
    pub search: Option<Artifact>,
    pub local_artifact_sizes: BTreeMap<String, Option<u64>>,
}
impl From<RunDocument> for Projection {
    fn from(doc: RunDocument) -> Self {
        Self {
            run_id: doc.run_id,
            migration_runtime_id: None,
            migration_world: None,
            metadata: doc.metadata,
            report: doc.report,
            change_spec: doc.change_spec,
            state_input: doc.state_input,
            bindings: doc.bindings,
            search: doc.search,
            local_artifact_sizes: doc.local_artifact_sizes,
        }
    }
}
impl Projection {
    pub fn view(&self) -> view::Run {
        view::from_bytes(
            &self.run_id,
            RunBytes {
                metadata: Some(self.metadata.text.as_bytes()),
                report: Some(self.report.text.as_bytes()),
                change_spec: self.change_spec.as_ref().map(|a| a.text.as_bytes()),
                state_input: self.state_input.as_ref().map(|a| a.text.as_bytes()),
                bindings: self.bindings.as_ref().map(|a| a.text.as_bytes()),
                search: self.search.as_ref().map(|a| a.text.as_bytes()),
            },
        )
    }
    /// Integrity and completeness of stored claims, never an execution proof.
    pub fn verify(&self) -> Result<()> {
        self.verified_kind().map(|_| ())
    }
    pub(crate) fn verified_kind(&self) -> Result<String> {
        for artifact in [
            Some(&self.metadata),
            Some(&self.report),
            self.change_spec.as_ref(),
            self.state_input.as_ref(),
            self.bindings.as_ref(),
            self.search.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            ensure!(
                artifact.sha256 == eplyx_engine::replay::hash_bytes(artifact.text.as_bytes()),
                "projection artifact digest mismatch"
            );
        }
        let metadata: serde_json::Value = serde_json::from_str(&self.metadata.text)?;
        if metadata["kind"] == "migration_order"
            || metadata["kind"] == crate::hosted::interaction::KIND
        {
            let m: eplyx_engine::local_store::AnalyticalMetadata =
                serde_json::from_str(&self.metadata.text)?;
            ensure!(
                m.schema_version == eplyx_engine::local_store::ANALYTICAL_METADATA_VERSION
                    && m.run_id == self.run_id
                    && m.report_sha256 == self.report.sha256
                    && m.change_spec_sha256 == self.change_spec.as_ref().map(|a| a.sha256.clone())
                    && m.state_input_sha256.is_none()
                    && self.state_input.is_none()
                    && self.search.is_none(),
                "order projection metadata mismatch"
            );
            let report: serde_json::Value = serde_json::from_str(&self.report.text)?;
            ensure!(
                report["kind"] == metadata["kind"]
                    && (!report["analysis"].is_null() || !report["failure"].is_null()),
                "invalid order result"
            );
            if m.kind == crate::hosted::interaction::KIND {
                ensure!(
                    self.change_spec.is_none()
                        && self.migration_runtime_id.is_none()
                        && self.migration_world.is_none()
                        && !report["analysis"].is_null(),
                    "interaction cannot invent a single proposal or migration context"
                );
            }
            return Ok(m.kind);
        }
        let parsed = self.view();
        ensure!(
            view::state(&parsed) == "Complete",
            "projection is incomplete or inconsistent: {}",
            view::summary(&parsed)["problems"]
        );
        Ok(parsed.kind().to_owned())
    }
}
impl crate::registry::Registry {
    pub fn analytical_projection(
        &self,
        metadata: &crate::registry::RunMetadata,
    ) -> Result<Projection> {
        let result: Projection = if let Some(job) = &metadata.hosted_analysis {
            let reference = job
                .projection
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("analysis has no completed projection"))?;
            let projection = serde_json::from_slice(&self.document_bytes(reference)?)?;
            self.verify_hosted_projection(metadata, &projection)?;
            projection
        } else {
            let projection: Projection = self.analytical_document(metadata)?.into();
            projection.verify()?;
            projection
        };
        Ok(result)
    }
}
