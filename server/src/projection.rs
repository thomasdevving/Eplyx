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
