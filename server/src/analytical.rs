//! Analytical records in MAIN's registry. Postgres never holds result bytes or
//! a second run index. A synced record is retained evidence, never a VM job.
use crate::{
    artifacts::{ArtifactClass, ArtifactRef},
    registry::{now_unix_seconds, Registry, RunMetadata, RunStatus},
};
use anyhow::{ensure, Context, Result};
use eplyx_engine::cloud::contract::{
    self, Artifact, CounterexampleDocument, ReproductionDocument, RunDocument,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunSource {
    Hosted,
    Local,
    Ci,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyticalRun {
    pub kind: String,
    pub source: RunSource,
    pub source_run_id: String,
    pub local_project_id: String,
    pub core_sha256: String,
    /// Exact UTF-8 member bytes inside the immutable sync envelope, without search.
    pub document: ArtifactRef,
    pub submitted_by: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedDocument {
    pub id: String,
    pub parent_run: String,
    pub local_project_id: String,
    pub artifact: ArtifactRef,
    pub created_at_unix_seconds: u64,
}
#[derive(Clone, Debug)]
pub struct SyncResult {
    pub run_id: String,
    pub status: &'static str,
}

impl Registry {
    pub(crate) fn document_ref(&self, bytes: &[u8]) -> Result<ArtifactRef> {
        Ok(self
            .artifacts()
            .put(ArtifactClass::Document, bytes)?
            .reference)
    }
    pub fn document_bytes(&self, reference: &ArtifactRef) -> Result<Vec<u8>> {
        self.artifacts().get(ArtifactClass::Document, reference)
    }
    fn search_path(&self, run: &str) -> Result<std::path::PathBuf> {
        Ok(self.storage().run_dir(run)?.join("search-ref.json"))
    }
    fn attach_search(&self, run: &str, search: Option<&Artifact>) -> Result<bool> {
        let Some(search) = search else {
            return Ok(false);
        };
        let reference = self.document_ref(search.text.as_bytes())?;
        let path = self.search_path(run)?;
        if path.exists() {
            let old: ArtifactRef = self.storage().read_json(&path)?;
            ensure!(
                old == reference,
                "a different search already belongs to this run"
            );
            self.document_bytes(&old)?;
            return Ok(false);
        }
        self.storage().write_json(&path, &reference)?;
        Ok(true)
    }
    pub fn analytical_document(&self, metadata: &RunMetadata) -> Result<RunDocument> {
        let analysis = metadata
            .analysis
            .as_ref()
            .context("run has no analytical document")?;
        let mut document: RunDocument =
            serde_json::from_slice(&self.document_bytes(&analysis.document)?)?;
        let path = self.search_path(&metadata.run_id)?;
        if path.exists() {
            let reference: ArtifactRef = self.storage().read_json(&path)?;
            document.search = Some(Artifact::new(self.document_bytes(&reference)?)?);
        }
        let verified = document.verify()?;
        ensure!(
            verified.core_sha256 == analysis.core_sha256
                && document.run_id == analysis.source_run_id
                && document.local_project_id == analysis.local_project_id
                && verified.run.kind() == analysis.kind,
            "analytical registry identity mismatch"
        );
        Ok(document)
    }
    pub fn source_run(&self, project: &str, source_run: &str) -> Result<Option<RunMetadata>> {
        for id in self.project_run_ids(project)? {
            if !self.storage().run_dir(&id)?.join("metadata.json").exists() {
                continue;
            }
            let run = self.load_run(&id)?;
            if run.project_id == project
                && run
                    .analysis
                    .as_ref()
                    .is_some_and(|a| a.source_run_id == source_run)
            {
                return Ok(Some(run));
            }
        }
        Ok(None)
    }
    pub fn sync_run_document(
        &self,
        project: &str,
        document: &RunDocument,
        source: RunSource,
        by: &str,
    ) -> Result<SyncResult> {
        let verified = document.verify()?;
        let _guard = self.transitions.lock().unwrap_or_else(|p| p.into_inner());
        self.load_project(project)?;
        if let Some(existing) = self.source_run(project, &document.run_id)? {
            let old = existing.analysis.as_ref().context("not analytical")?;
            ensure!(
                old.core_sha256 == verified.core_sha256,
                "run identity already has different immutable bytes"
            );
            self.analytical_document(&existing)?;
            let attached = self.attach_search(&existing.run_id, document.search.as_ref())?;
            return Ok(SyncResult {
                run_id: existing.run_id,
                status: if attached {
                    "search_attached"
                } else {
                    "unchanged"
                },
            });
        }
        let mut core = document.clone();
        core.search = None;
        let reference = self.document_ref(&serde_json::to_vec(&core)?)?;
        let now = now_unix_seconds();
        let id = crate::ids::run();
        self.attach_search(&id, document.search.as_ref())?;
        let change = document
            .change_spec
            .as_ref()
            .map(|a| serde_json::from_str::<eplyx_engine::change::ChangeSpec>(&a.text))
            .transpose()?
            .map(|spec| {
                crate::registry::RunChange::of(&spec, crate::registry::ChangeOrigin::Submitted)
            })
            .transpose()?;
        let metadata = RunMetadata {
            hosted_analysis: None,
            analysis: Some(AnalyticalRun {
                kind: verified.run.kind().into(),
                source,
                source_run_id: document.run_id.clone(),
                local_project_id: document.local_project_id.clone(),
                core_sha256: verified.core_sha256,
                document: reference,
                submitted_by: by.into(),
            }),
            run_id: id.clone(),
            project_id: project.into(),
            status: RunStatus::Completed,
            bundle_sha256: String::new(),
            bundle_id: None,
            corpus_sha256: None,
            baseline_sha256: None,
            candidate_sha256: String::new(),
            change,
            candidate_artifact: None,
            expectations_sha256: None,
            attempts: vec![],
            adapter: None,
            adapter_version: None,
            semantic_schema_version: None,
            record_count: None,
            exit_code: None,
            report_available: true,
            detail: None,
            created_at_unix_seconds: now,
            started_at_unix_seconds: None,
            completed_at_unix_seconds: Some(now),
        };
        self.index_run(project, &id)?;
        if let Some(change) = &metadata.change {
            self.index_change(project, &change.change_spec_id, &id)?;
        }
        self.create_run(&metadata)?;
        Ok(SyncResult {
            run_id: id,
            status: "created",
        })
    }
    fn saved_path(&self, project: &str, kind: &str, id: &str) -> Result<std::path::PathBuf> {
        ensure!(
            matches!(kind, "counterexamples" | "reproductions") && crate::storage::valid_id(id),
            "invalid saved document address"
        );
        Ok(self
            .storage()
            .project_dir(project)?
            .join(kind)
            .join(format!("{id}.json")))
    }
    pub fn saved_document(&self, project: &str, kind: &str, id: &str) -> Result<SavedDocument> {
        let saved: SavedDocument = self
            .storage()
            .read_json(&self.saved_path(project, kind, id)?)?;
        ensure!(
            saved.id == id && self.load_run(&saved.parent_run)?.project_id == project,
            "saved document identity mismatch"
        );
        self.document_bytes(&saved.artifact)?;
        Ok(saved)
    }
    pub fn saved_documents(&self, project: &str, kind: &str) -> Result<Vec<SavedDocument>> {
        ensure!(
            matches!(kind, "counterexamples" | "reproductions"),
            "invalid document kind"
        );
        let path = self.storage().project_dir(project)?.join(kind);
        if !path.exists() {
            return Ok(vec![]);
        }
        let mut documents = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name.strip_suffix(".json") {
                documents.push(self.saved_document(project, kind, id)?);
            }
        }
        documents.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(documents)
    }
    fn save_document(
        &self,
        project: &str,
        kind: &str,
        id: &str,
        local_project_id: &str,
        parent_run: &str,
        artifact: &Artifact,
    ) -> Result<&'static str> {
        let reference = self.document_ref(artifact.text.as_bytes())?;
        let path = self.saved_path(project, kind, id)?;
        if path.exists() {
            let old = self.saved_document(project, kind, id)?;
            ensure!(
                old.parent_run == parent_run
                    && old.local_project_id == local_project_id
                    && old.artifact == reference,
                "saved document identity already has different immutable bytes"
            );
            return Ok("unchanged");
        }
        self.storage().write_json(
            &path,
            &SavedDocument {
                id: id.into(),
                parent_run: parent_run.into(),
                local_project_id: local_project_id.into(),
                artifact: reference,
                created_at_unix_seconds: now_unix_seconds(),
            },
        )?;
        Ok("created")
    }
    pub fn sync_counterexample_document(
        &self,
        project: &str,
        document: &CounterexampleDocument,
    ) -> Result<&'static str> {
        let checked = document.verify()?;
        let _guard = self.transitions.lock().unwrap_or_else(|p| p.into_inner());
        let parent = self
            .source_run(project, &checked.saved.parent_run)?
            .context("sync the parent run first")?;
        ensure!(
            parent
                .analysis
                .as_ref()
                .is_some_and(|a| a.local_project_id == document.local_project_id),
            "local project mismatch"
        );
        contract::bind_counterexample(&checked.saved, &self.analytical_document(&parent)?.view())?;
        self.save_document(
            project,
            "counterexamples",
            &document.counterexample_id,
            &document.local_project_id,
            &parent.run_id,
            &document.file,
        )
    }
    pub fn sync_reproduction_document(
        &self,
        project: &str,
        document: &ReproductionDocument,
    ) -> Result<&'static str> {
        let checked = document.verify()?;
        let _guard = self.transitions.lock().unwrap_or_else(|p| p.into_inner());
        let parent = self
            .saved_document(project, "counterexamples", &document.counterexample_id)
            .context("sync the parent counterexample first")?;
        ensure!(
            parent.local_project_id == document.local_project_id,
            "local project mismatch"
        );
        let saved = serde_json::from_slice(&self.document_bytes(&parent.artifact)?)?;
        contract::bind_reproduction(&checked, &saved)?;
        self.save_document(
            project,
            "reproductions",
            &document.reproduction_id,
            &document.local_project_id,
            &parent.parent_run,
            &document.file,
        )
    }
}
