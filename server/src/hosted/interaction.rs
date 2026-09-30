//! Project occurrences of the existing six-execution engine. Parent provenance
//! and CAS references stay outside the portable engine bytes.
use super::Input;
use crate::{
    api::{project_for, ApiError, ApiResult, Shared},
    artifacts::{ArtifactClass, ArtifactRef},
    projection::Projection,
    registry::{Registry, RunMetadata, RunStatus},
};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json,
};
use eplyx_engine::{
    canonical, change::ChangeSpec, cloud::contract::Artifact, interaction as engine,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path as FsPath};

pub const KIND: &str = "upgrade_parameter_interaction";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    EvidenceIntegrity,
    AdmissionRejected,
}
#[derive(Debug)]
pub struct EvidenceError;
impl std::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("interaction retained evidence could not be verified")
    }
}
impl std::error::Error for EvidenceError {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub project_id: String,
    pub parent_run: String,
    pub parent_input: ArtifactRef,
    pub input_contract: ArtifactRef,
    pub parent_projection: ArtifactRef,
    pub upgrade: ArtifactRef,
    pub parameter: ArtifactRef,
    pub upgrade_change_spec_id: String,
    pub parameter_change_spec_id: String,
    pub record_id: String,
    pub record_sha256: String,
    pub bundle_sha256: String,
    pub analysis_input_sha256: String,
    pub source_slot_range: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentInput {
    project_id: String,
    run_id: String,
    bundle_id: String,
    bundle_sha256: String,
    baseline_sha256: String,
    corpus_sha256: String,
    candidate: ArtifactRef,
    upgrade_change_spec_id: String,
    upgrade_document_sha256: String,
    expectations_sha256: Option<String>,
    source_slot_range: Value,
}
struct Parent {
    upgrade: ChangeSpec,
    candidate: Vec<u8>,
    input: Vec<u8>,
    projection: Vec<u8>,
    source_slot_range: Value,
}
fn parent(registry: &Registry, record: &RunMetadata) -> Result<Parent> {
    ensure!(
        record.hosted_analysis.is_none()
            && record.analysis.is_none()
            && matches!(
                record.status,
                RunStatus::Passed | RunStatus::Failed | RunStatus::Completed
            )
            && record.report_available,
        "completed retained program-upgrade parent required"
    );
    let indexed = record.change.as_ref().context("missing parent proposal")?;
    let upgrade = registry.load_change_spec(&record.run_id, indexed)?;
    let u = upgrade
        .as_program_upgrade()
        .context("program-upgrade parent required")?;
    let reference = record
        .candidate_artifact
        .as_ref()
        .context("missing parent executable")?;
    ensure!(
        reference.sha256 == u.candidate.sha256
            && reference.len == u.candidate.len
            && record.candidate_sha256 == reference.sha256,
        "parent candidate mismatch"
    );
    let candidate = registry
        .artifacts()
        .get(ArtifactClass::Program, reference)?;
    let projection = registry.load_run_artifact(&record.run_id, "report.json")?;
    let report: eplyx_engine::ci::CiReport = serde_json::from_slice(&projection)?;
    indexed.verify_report(&report)?;
    ensure!(
        report.bundle.sha256 == record.bundle_sha256
            && Some(&report.bundle.baseline_sha256) == record.baseline_sha256.as_ref()
            && Some(&report.bundle.corpus_sha256) == record.corpus_sha256.as_ref()
            && report.bundle.program_id == u.target.program_id
            && report.candidate.sha256 == reference.sha256
            && report.candidate.len == reference.len,
        "parent result input mismatch"
    );
    registry.load_expectations(record)?;
    let source_slot_range = serde_json::to_value(report.bundle.source_slot_range)?;
    let input = canonical::document(&ParentInput {
        project_id: record.project_id.clone(),
        run_id: record.run_id.clone(),
        bundle_id: record.bundle_id.clone().context("missing parent bundle")?,
        bundle_sha256: record.bundle_sha256.clone(),
        baseline_sha256: report.bundle.baseline_sha256,
        corpus_sha256: report.bundle.corpus_sha256,
        candidate: reference.clone(),
        expectations_sha256: record.expectations_sha256.clone(),
        source_slot_range: source_slot_range.clone(),
        upgrade_change_spec_id: upgrade.id()?,
        upgrade_document_sha256: eplyx_engine::replay::hash_bytes(
            upgrade.to_document()?.as_bytes(),
        ),
    })?
    .into_bytes();
    Ok(Parent {
        upgrade,
        candidate,
        input,
        projection,
        source_slot_range,
    })
}
fn selected(
    registry: &Registry,
    record: &RunMetadata,
    p: &Parent,
    record_id: &str,
    parameter: &ChangeSpec,
) -> Result<engine::Input> {
    ensure!(
        !record_id.is_empty(),
        "explicit historical record_id required"
    );
    let historical = super::parameter::historical_input(registry, record, record_id)?;
    let input = engine::Input::new(&p.upgrade, parameter, historical, p.candidate.clone())?;
    engine::prepare(&input)?;
    Ok(input)
}
/// Whitelisted input facts; no raw account, message or ELF bytes are served.
fn facts(input: &engine::Input, contract: &Value) -> Value {
    let param = input
        .parameter
        .as_protocol_parameter_change()
        .expect("prepared parameter");
    json!({"upgrade_change_spec_id":input.upgrade.id().ok(),"parameter_change_spec_id":input.parameter.id().ok(),
        "upgrade_change_spec":input.upgrade,"parameter_change_spec":input.parameter,
        "analysis_input_sha256":canonical::digest(contract).ok(),
        "record_id":input.historical.record.id,"record_sha256":input.historical.record_sha256,
        "bundle_sha256":input.historical.source_bundle_sha256,"pool":param.target.config_account,
        "slot":input.historical.record.clock.slot.to_string(),"operation":param.operation,
        "v1":{"sha256":input.historical.record.current_program_sha256,"provenance":"retained historical baseline"},
        "v2":{"sha256":input.candidate.elf_sha256,"loader":input.candidate.loader,"len":input.candidate.elf.len().to_string(),"profile":contract["profile"]},
        "dependencies":input.historical.programs.iter().map(|p|json!({"program_id":p.program_id,"loader":p.loader,"sha256":p.elf_sha256,"len":p.elf.len().to_string()})).collect::<Vec<_>>(),
        "runtime":contract["runtime"],"clock":contract["configuration"]["clock"],
        "manager_assumption":contract["configuration"]["manager_assumption"],"fee_payer":contract["configuration"]["fee_payer"],
        "semantics":contract["semantics"],"scope":"One retained DepositSol; fixed epoch 0. Six independent fresh banks. Constructed profiles are test code, not an upstream release. No loader Upgrade, rollout order, authority possession, atomicity or governance approval is established."})
}
fn reason(error: &anyhow::Error) -> (&'static str, &'static str) {
    let text = error.to_string();
    if text.contains("candidate_interpretation_unqualified") {
        (
            "candidate_unqualified",
            "The retained candidate has no reviewed interaction profile.",
        )
    } else if text.contains("unsupported upgrade activation/delivery") {
        ("unsupported_upgrade_expectations","The exact parent proposal contains activation, delivery, authority or ProgramData expectations that this analyzer does not evaluate.")
    } else if text.starts_with("CurrentStateMismatch:") {
        ("current_state_mismatch","The declared expected-current pool data or fee fields differ from the retained record.")
    } else if text.starts_with("UnsupportedConfigField:") {
        (
            "parameter_operation_unsupported",
            "Only the existing Stake Pool SolDeposit SetFee parameter operation is supported.",
        )
    } else if text.contains("expected") || text.contains("current") {
        ("proposal_or_record_unsupported","The selected proposal or retained record does not satisfy the current bounded admission contract.")
    } else {
        ("retained_evidence_unavailable","The retained parent, bundle, proposal, candidate, dependency or supported record could not be verified.")
    }
}
pub fn eligibility(registry: &Registry, record: &RunMetadata) -> Value {
    if record.hosted_analysis.is_some()
        || record.analysis.is_some()
        || record
            .change
            .as_ref()
            .is_none_or(|c| c.kind() != eplyx_engine::change::ChangeKind::ProgramUpgrade)
    {
        return json!({"eligible":false,"parent_run_id":record.run_id,"records":[],"reason_code":"wrong_parent_kind","reason":"Select an ordinary retained program-upgrade parent."});
    }
    if !matches!(
        record.status,
        RunStatus::Passed | RunStatus::Failed | RunStatus::Completed
    ) || !record.report_available
    {
        return json!({"eligible":false,"parent_run_id":record.run_id,"records":[],"reason_code":"parent_not_completed","reason":"The parent has no completed retained result."});
    }
    let attempted = (|| -> Result<Value> {
        let p = parent(registry, record)?;
        let bundle = registry.open_bundle(&record.bundle_sha256)?;
        let mut records = Vec::new();
        let mut unavailable = Vec::new();
        for r in bundle.records() {
            let attempt = (|| -> Result<Value> {
                let h = super::parameter::historical_input(registry, record, &r.id)?;
                let input = engine::Input::retained_control(&p.upgrade, h, p.candidate.clone())?;
                let contract = engine::prepare(&input)?;
                let mut v = facts(&input, &contract);
                v["source_slot_range"] = p.source_slot_range.clone();
                v.as_object_mut().unwrap().remove("parameter_change_spec");
                v.as_object_mut()
                    .unwrap()
                    .remove("parameter_change_spec_id");
                v.as_object_mut().unwrap().remove("analysis_input_sha256");
                Ok(v)
            })();
            match attempt {
                Ok(v) => records.push(v),
                Err(e) => {
                    let (code, reason) = reason(&e);
                    unavailable.push(json!({"record_id":r.id,"reason_code":code,"reason":reason}));
                }
            }
        }
        Ok(
            json!({"eligible":!records.is_empty(),"parent_run_id":record.run_id,"records":records,"unavailable_records":unavailable,"reason":if records.is_empty(){Some("No retained record meets the bounded interaction contract.")}else{None}}),
        )
    })();
    match attempted {
        Ok(v) => v,
        Err(e) => {
            let (code, reason) = reason(&e);
            json!({"eligible":false,"parent_run_id":record.run_id,"records":[],"reason_code":code,"reason":reason})
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub record_id: String,
    pub parameter_change_spec: ChangeSpec,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_key: String,
    pub record_id: String,
    pub parameter_change_spec: ChangeSpec,
}
pub fn preview(
    registry: &Registry,
    record: &RunMetadata,
    request: &PreviewRequest,
) -> Result<Value> {
    let p = parent(registry, record)?;
    let input = selected(
        registry,
        record,
        &p,
        &request.record_id,
        &request.parameter_change_spec,
    )?;
    let mut facts = facts(&input, &engine::prepare(&input)?);
    facts["source_slot_range"] = p.source_slot_range;
    Ok(facts)
}
pub fn create(registry: &Registry, record: &RunMetadata, request: &Request) -> Result<RunMetadata> {
    super::observation::key(&request.request_key)?;
    let p = parent(registry, record)?;
    let input = selected(
        registry,
        record,
        &p,
        &request.record_id,
        &request.parameter_change_spec,
    )?;
    let contract = engine::prepare(&input)?;
    let binding = Binding {
        project_id: record.project_id.clone(),
        parent_run: record.run_id.clone(),
        parent_input: registry.document_ref(&p.input)?,
        input_contract: registry.document_ref(canonical::document(&contract)?.as_bytes())?,
        parent_projection: registry.document_ref(&p.projection)?,
        upgrade: registry.document_ref(canonical::document(&input.upgrade)?.as_bytes())?,
        parameter: registry.document_ref(canonical::document(&input.parameter)?.as_bytes())?,
        upgrade_change_spec_id: input.upgrade.id()?,
        parameter_change_spec_id: input.parameter.id()?,
        record_id: request.record_id.clone(),
        record_sha256: input.historical.record_sha256.clone(),
        bundle_sha256: record.bundle_sha256.clone(),
        analysis_input_sha256: canonical::digest(&contract)?,
        source_slot_range: p.source_slot_range.clone(),
    };
    let capture = registry
        .artifacts()
        .put(
            ArtifactClass::Capture,
            canonical::document(&input)?.as_bytes(),
        )?
        .reference;
    registry.create_hosted_analysis_with_key(
        &record.project_id,
        Input::UpgradeParameterInteraction {
            capture,
            binding: Box::new(binding),
        },
        Some(request.request_key.clone()),
    )
}
/// Revalidation always resolves the pinned parent; never the active bundle.
pub fn accepted(registry: &Registry, input: &Input) -> Result<engine::Input> {
    let Input::UpgradeParameterInteraction {
        capture,
        binding: b,
    } = input
    else {
        anyhow::bail!("interaction input required")
    };
    let record = registry.load_run(&b.parent_run)?;
    ensure!(record.project_id == b.project_id, "cross-project parent");
    let p = parent(registry, &record)?;
    ensure!(
        registry.document_bytes(&b.parent_input)? == p.input
            && registry.document_bytes(&b.parent_projection)? == p.projection,
        "parent references changed"
    );
    let retained: engine::Input =
        serde_json::from_slice(&registry.artifacts().get(ArtifactClass::Capture, capture)?)?;
    let parameter = ChangeSpec::parse(&registry.document_bytes(&b.parameter)?)?;
    let expected = selected(registry, &record, &p, &b.record_id, &parameter)?;
    ensure!(
        retained == expected
            && registry.document_bytes(&b.upgrade)?
                == canonical::document(&expected.upgrade)?.as_bytes()
            && b.upgrade_change_spec_id == expected.upgrade.id()?
            && b.parameter_change_spec_id == parameter.id()?
            && b.source_slot_range == p.source_slot_range
            && b.bundle_sha256 == record.bundle_sha256
            && b.record_sha256 == expected.historical.record_sha256
            && registry.document_bytes(&b.input_contract)?
                == canonical::document(&engine::prepare(&expected)?)?.as_bytes()
            && b.analysis_input_sha256 == canonical::digest(&engine::prepare(&expected)?)?,
        "accepted interaction binding mismatch"
    );
    Ok(retained)
}
pub(super) fn stage(registry: &Registry, input: &Input, directory: &FsPath) -> Result<()> {
    let retained = accepted(registry, input).map_err(|_| EvidenceError)?;
    std::fs::write(
        directory.join("interaction-input.json"),
        canonical::document(&retained)?,
    )?;
    Ok(())
}
pub fn presentation(r: &engine::Report) -> Value {
    fn stage<T: Serialize>(s: &engine::Stage<T>) -> Value {
        let mut derived = s.derived.clone();
        if let Some(o) = derived.as_object_mut() {
            o.remove("verified_pool");
        }
        json!({"state":s.state,"input_sha256":s.input_sha256,"execution_sha256":s.execution_sha256,"parent":s.parent,
            "reason":s.reason,"infrastructure_error":s.infrastructure_error.as_ref().map(|_|"The stage could not execute in the offline worker. Original error evidence is retained in the portable artifact."),"derived":derived})
    }
    json!({"status":r.status,"analysis_input_sha256":r.analysis_input_sha256,"report_sha256":r.report_sha256,
        "facts":facts(&r.input,&r.input_contract),"admission":r.admission,"limitations":r.limitations,
        "k1":stage(&r.k1),"k2":stage(&r.k2),"r00":stage(&r.r00),"r01":stage(&r.r01),"r10":stage(&r.r10),"r11":stage(&r.r11),
        "measurements":r.measurements,"effects":r.effects})
}
pub(super) fn execute(input: &Input, directory: &FsPath) -> Result<Value> {
    let Input::UpgradeParameterInteraction { capture, binding } = input else {
        anyhow::bail!("interaction input required")
    };
    let retained: engine::Input = serde_json::from_slice(&super::worker::member(
        directory,
        "interaction-input.json",
        capture,
    )?)?;
    ensure!(
        canonical::digest(&engine::prepare(&retained)?)? == binding.analysis_input_sha256,
        "worker input identity mismatch"
    );
    let report = engine::analyse(&retained)?;
    engine::save(&report, &directory.join("interaction"))?;
    Ok(json!({"kind":KIND,"analysis":presentation(&report)}))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Portable {
    binding: Binding,
    files: BTreeMap<String, ArtifactRef>,
}
fn safe_member(name: &str) -> bool {
    matches!(name, "manifest.json" | "report.md")
        || name.strip_prefix("objects/").is_some_and(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}
fn materialize(registry: &Registry, p: &Portable, directory: &FsPath) -> Result<()> {
    ensure!(
        p.files.len() <= 34
            && p.files.contains_key("manifest.json")
            && p.files.contains_key("report.md"),
        "invalid artifact members"
    );
    let total = p.files.values().try_fold(0u64, |sum, r| {
        sum.checked_add(r.len).context("artifact size overflow")
    })?;
    ensure!(
        total <= eplyx_engine::lifecycle::artifact::MAX_BYTES,
        "artifact byte bound"
    );
    for (name, r) in &p.files {
        ensure!(safe_member(name), "unsafe artifact member");
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().context("member parent")?)?;
        std::fs::write(path, registry.artifacts().get(ArtifactClass::Capture, r)?)?;
    }
    Ok(())
}
pub(super) fn retain(
    registry: &Registry,
    input: &Input,
    directory: &FsPath,
    projection: &mut Projection,
) -> Result<()> {
    let Input::UpgradeParameterInteraction { binding, .. } = input else {
        return Ok(());
    };
    let root = directory.join("interaction");
    engine::load(&root)?;
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for name in ["manifest.json", "report.md"]
        .into_iter()
        .map(str::to_owned)
        .chain(
            std::fs::read_dir(root.join("objects"))?
                .map(|e| e.map(|e| format!("objects/{}", e.file_name().to_string_lossy())))
                .collect::<std::io::Result<Vec<_>>>()?,
        )
    {
        ensure!(
            safe_member(&name) && std::fs::symlink_metadata(root.join(&name))?.is_file(),
            "invalid artifact member"
        );
        let size = std::fs::symlink_metadata(root.join(&name))?.len();
        total = total.checked_add(size).context("artifact size overflow")?;
        ensure!(
            files.len() < 34 && total <= eplyx_engine::lifecycle::artifact::MAX_BYTES,
            "artifact member/byte bound"
        );
        let bytes = std::fs::read(root.join(&name))?;
        files.insert(
            name,
            registry
                .artifacts()
                .put(ArtifactClass::Capture, &bytes)?
                .reference,
        );
    }
    let portable = Portable {
        binding: (**binding).clone(),
        files,
    };
    let tmp = tempfile::tempdir()?;
    materialize(registry, &portable, tmp.path())?;
    engine::load(tmp.path())?;
    projection.bindings = Some(Artifact::new(canonical::document(&portable)?.into_bytes())?);
    Ok(())
}
pub fn verify(registry: &Registry, record: &RunMetadata, projection: &Projection) -> Result<()> {
    let input = registry.hosted_input(record)?;
    let Input::UpgradeParameterInteraction { binding, .. } = &input else {
        anyhow::bail!("interaction required")
    };
    ensure!(
        binding.project_id == record.project_id,
        "interaction project mismatch"
    );
    let expected = accepted(registry, &input)?;
    let portable: Portable = serde_json::from_str(
        &projection
            .bindings
            .as_ref()
            .context("missing portable binding")?
            .text,
    )?;
    ensure!(
        portable.binding == **binding,
        "portable parent/proposals mismatch"
    );
    let tmp = tempfile::tempdir()?;
    materialize(registry, &portable, tmp.path())?;
    let report = engine::load(tmp.path())?;
    ensure!(
        report.input == expected
            && report.analysis_input_sha256 == binding.analysis_input_sha256
            && serde_json::from_str::<Value>(&projection.report.text)?
                == json!({"kind":KIND,"analysis":presentation(&report)}),
        "artifact/result mismatch"
    );
    Ok(())
}
pub fn result(registry: &Registry, record: &RunMetadata) -> Result<Value> {
    let input = registry.hosted_input(record)?;
    let Input::UpgradeParameterInteraction { binding, .. } = &input else {
        anyhow::bail!("interaction required")
    };
    ensure!(
        binding.project_id == record.project_id,
        "interaction project mismatch"
    );
    let mut result = json!({"kind":KIND,"run_id":record.run_id,"project_id":record.project_id,"status":record.status,"binding":binding,"parent_run_id":binding.parent_run,"analysis":null,"artifact_available":false,"failure":null});
    // Even queued/result reads verify accepted retained evidence; no repair/VM.
    if accepted(registry, &input).is_err() {
        result["failure"] = json!({"kind":"evidence_integrity","detail":"Retained evidence could not be verified; no analytical conclusion is available."});
        return Ok(result);
    }
    if record.report_available {
        match registry.analytical_projection(record) {
            Ok(p) => {
                result["analysis"] =
                    serde_json::from_str::<Value>(&p.report.text)?["analysis"].clone();
                result["artifact_available"] = json!(true);
            }
            Err(_) => {
                result["failure"] = json!({"kind":"evidence_integrity","detail":"The retained portable artifact or projection could not be verified; no analytical conclusion is available."})
            }
        }
    } else if let Some(kind) = record
        .hosted_analysis
        .as_ref()
        .and_then(|j| j.interaction_failure.as_ref())
    {
        result["failure"] = json!({"kind":kind,"detail":"The accepted evidence or admission could not be verified; no analytical conclusion is available."});
    } else if record.status == RunStatus::ExecutionError {
        result["failure"] = json!({"kind":"internal_execution_failure","detail":"The worker could not complete; no analytical conclusion is available."});
    }
    Ok(result)
}
pub fn children(registry: &Registry, project: &str, parent: &str) -> Result<Vec<Value>> {
    let mut children = Vec::new();
    for id in registry.project_run_ids(project)? {
        let r = registry.load_run(&id)?;
        if r.hosted_analysis.as_ref().is_some_and(|j| j.kind == KIND) {
            if let Input::UpgradeParameterInteraction { binding, .. } = registry.hosted_input(&r)? {
                ensure!(
                    r.project_id == project && binding.project_id == project,
                    "child project mismatch"
                );
                if binding.parent_run == parent {
                    let mut child = result(registry, &r)?;
                    if !child["analysis"].is_null() {
                        child["analysis"] = json!({"status":child["analysis"]["status"],"analysis_input_sha256":child["analysis"]["analysis_input_sha256"],"report_sha256":child["analysis"]["report_sha256"]});
                    }
                    children.push(child);
                }
            }
        }
    }
    Ok(children)
}
async fn authorized(
    state: &Shared,
    headers: &HeaderMap,
    project: &str,
    id: &str,
) -> ApiResult<RunMetadata> {
    project_for(state, project, headers).await?;
    let r = state
        .registry
        .load_run(id)
        .map_err(|_| ApiError::not_found("run"))?;
    if r.project_id != project {
        return Err(ApiError::not_found("run"));
    }
    Ok(r)
}
fn invalid(error: anyhow::Error) -> ApiError {
    let (code, reason) = reason(&error);
    ApiError::bad_request(format!("{code}: {reason}"))
}
async fn available(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let r = authorized(&s, &headers, &project, &id).await?;
    Ok(Json(
        tokio::task::spawn_blocking(move || eligibility(&s.registry, &r))
            .await
            .map_err(|_| ApiError::internal("Eligibility could not be verified."))?,
    ))
}
async fn inspect(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
    Json(request): Json<PreviewRequest>,
) -> ApiResult<Json<Value>> {
    let r = authorized(&s, &headers, &project, &id).await?;
    Ok(Json(
        tokio::task::spawn_blocking(move || preview(&s.registry, &r, &request))
            .await
            .map_err(|_| ApiError::internal("Preview could not be verified."))?
            .map_err(invalid)?,
    ))
}
async fn submit(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
    Json(request): Json<Request>,
) -> ApiResult<Response> {
    let r = authorized(&s, &headers, &project, &id).await?;
    super::observation::key(&request.request_key).map_err(super::observation::invalid)?;
    let task = s.clone();
    let r = tokio::task::spawn_blocking(move || create(&task.registry, &r, &request))
        .await
        .map_err(|_| ApiError::internal("Interaction submission stopped."))?
        .map_err(|e| {
            if e.to_string().contains("request key conflict") {
                ApiError::new(StatusCode::CONFLICT, "request key conflict")
            } else {
                invalid(e)
            }
        })?;
    crate::worker::spawn(s, r.run_id.clone());
    super::observation::accepted(&r)
}
async fn list(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    authorized(&s, &headers, &project, &id).await?;
    Ok(Json(
        json!({"interactions":tokio::task::spawn_blocking(move||children(&s.registry,&project,&id)).await.map_err(|_|ApiError::internal("Interaction index unavailable."))?.map_err(|_|ApiError::internal("Interaction evidence unavailable."))?}),
    ))
}
async fn read(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let r = authorized(&s, &headers, &project, &id).await?;
    Ok(Json(
        tokio::task::spawn_blocking(move || result(&s.registry, &r))
            .await
            .map_err(|_| ApiError::internal("Interaction read stopped."))?
            .map_err(|_| ApiError::internal("Interaction evidence unavailable."))?,
    ))
}
async fn download(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let r = authorized(&s, &headers, &project, &id).await?;
    if !r.hosted_analysis.as_ref().is_some_and(|j| j.kind == KIND) {
        return Err(ApiError::not_found("interaction"));
    }
    if !r.report_available {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "No portable artifact is available.",
        ));
    }
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let p = s.registry.analytical_projection(&r)?;
        let portable: Portable =
            serde_json::from_str(&p.bindings.context("missing artifact")?.text)?;
        let tmp = tempfile::tempdir()?;
        materialize(&s.registry, &portable, &tmp.path().join("interaction"))?;
        super::order::archive(tmp.path(), "interaction")
    })
    .await
    .map_err(|_| ApiError::internal("Artifact packaging stopped."))?
    .map_err(|_| ApiError::internal("Retained artifact integrity could not be verified."))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-tar"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=interaction.tar",
            ),
        ],
        bytes,
    )
        .into_response())
}
pub fn router() -> axum::Router<Shared> {
    axum::Router::new()
        .route(
            "/v1/projects/{project}/runs/{id}/interactions/eligibility",
            get(available),
        )
        .route(
            "/v1/projects/{project}/runs/{id}/interactions/preview",
            post(inspect),
        )
        .route(
            "/v1/projects/{project}/runs/{id}/interactions",
            post(submit).get(list),
        )
        .route("/v1/projects/{project}/interactions/{id}", get(read))
        .route(
            "/v1/projects/{project}/interactions/{id}/artifact",
            get(download),
        )
}
