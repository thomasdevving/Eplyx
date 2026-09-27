//! Shared, read-only views of MAIN analytical records. Parsing a saved claim never
//! replays it or grants execution proof. Local and hosted views use these same bytes.
use super::store::{Store, CAPTURE_LIMIT, REPORT_LIMIT, SEARCH_LIMIT, SMALL_LIMIT};
use crate::{
    local_store::{AnalyticalMetadata, Metadata, Reproduction, SavedMigrationCounterexample},
    migration::search::SearchResult,
    replay::hash_bytes as sha256,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub const ARTIFACTS: &[(&str, &str, &str, &str)] = &[
    (
        "report.json",
        "result/report.json",
        "application/json",
        "Engine report",
    ),
    (
        "report.md",
        "result/report.md",
        "text/plain; charset=utf-8",
        "Human report",
    ),
    (
        "metadata.json",
        "metadata.json",
        "application/json",
        "Local run metadata",
    ),
    (
        "bindings.json",
        "result/bindings.json",
        "application/json",
        "Evidence bindings",
    ),
    (
        "change_spec.json",
        "input/change.json",
        "application/json",
        "Change specification",
    ),
    (
        "state_input.json",
        "input/state.json",
        "application/json",
        "State descriptor",
    ),
    (
        "migration.plan.json",
        "result/migration.plan.json",
        "application/json",
        "Migration plan",
    ),
    (
        "migration.unsigned-plan.json",
        "result/migration.unsigned-plan.json",
        "application/json",
        "Unsigned execution plan",
    ),
    (
        "population.rehearsal.json",
        "result/population.rehearsal.json",
        "application/json",
        "Population rehearsal",
    ),
    (
        "stress.plan.json",
        "result/stress.plan.json",
        "application/json",
        "Frozen stress plan",
    ),
    (
        "stress.results.json",
        "result/stress.results.json",
        "application/json",
        "Stress results",
    ),
    (
        "migration-search.json",
        "search/migration-search.json",
        "application/json",
        "Counterexample search",
    ),
];
pub fn artifacts_for_kind(rows: Vec<Value>, migration: bool) -> Vec<Value> {
    rows.into_iter()
        .filter(|r| {
            migration
                || [
                    "report.json",
                    "report.md",
                    "metadata.json",
                    "change_spec.json",
                    "state_input.json",
                ]
                .contains(&r["name"].as_str().unwrap_or(""))
        })
        .collect()
}
pub fn artifact(name: &str) -> Option<(&'static str, &'static str)> {
    ARTIFACTS
        .iter()
        .find(|(public, ..)| *public == name)
        .map(|(_, path, kind, _)| (*path, *kind))
}

pub struct Run {
    pub(crate) id: String,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) analytical_metadata: Option<AnalyticalMetadata>,
    pub(crate) report: Option<Value>,
    pub(crate) change_spec: Option<Value>,
    pub(crate) state_input: Option<Value>,
    pub(crate) invariants: Option<Value>,
    pub(crate) migration_search: Option<SearchResult>,
    pub(crate) search_sha256: Option<String>,
    pub(crate) problems: Vec<String>,
}
#[derive(Clone, Copy, Default)]
pub struct RunBytes<'a> {
    pub metadata: Option<&'a [u8]>,
    pub report: Option<&'a [u8]>,
    pub change_spec: Option<&'a [u8]>,
    pub state_input: Option<&'a [u8]>,
    pub bindings: Option<&'a [u8]>,
    pub search: Option<&'a [u8]>,
}
impl Run {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn report(&self) -> Option<&Value> {
        self.report.as_ref()
    }
    pub fn migration_search(&self) -> Option<&SearchResult> {
        self.migration_search.as_ref()
    }
    pub fn search_sha256(&self) -> Option<&str> {
        self.search_sha256.as_deref()
    }
    pub fn problems(&self) -> &[String] {
        &self.problems
    }
    pub fn has_search(&self) -> bool {
        self.migration_search.is_some()
    }
    pub fn is_migration(&self) -> bool {
        // A current candidate check can carry a token-migration proposal while
        // reporting one exact account, not a population migration report.
        self.analytical_metadata.is_none()
            && super::migration::is_migration(self.report.as_ref(), self.change_spec.as_ref())
    }
    pub fn kind(&self) -> &str {
        if self.is_migration() {
            "token_migration"
        } else {
            self.analytical_metadata
                .as_ref()
                .map_or("unknown", |m| m.kind.as_str())
        }
    }
}
pub fn search_member(_: Option<&[u8]>) -> &'static str {
    "search/migration-search.json"
}
fn parts(id: &str, member: &str) -> Vec<String> {
    std::iter::once("runs".to_owned())
        .chain(std::iter::once(id.to_owned()))
        .chain(member.split('/').map(str::to_owned))
        .collect()
}

fn read_json(
    store: &Store,
    id: &str,
    member: &str,
    limit: u64,
    problems: &mut Vec<String>,
) -> Option<Value> {
    let owned = parts(id, member);
    let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
    match store.json(&borrowed, limit) {
        Ok(value) => value,
        Err(error) => {
            problems.push(format!("{member}: {error:#}"));
            None
        }
    }
}

fn read_member(
    store: &Store,
    id: &str,
    member: &str,
    limit: u64,
    problems: &mut Vec<String>,
) -> Option<Vec<u8>> {
    let owned = parts(id, member);
    let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
    match store.read(&borrowed, limit) {
        Ok(bytes) => bytes,
        Err(error) => {
            problems.push(format!("{member}: {error:#}"));
            None
        }
    }
}

pub fn load(store: &Store, id: &str) -> Run {
    let mut problems = Vec::new();
    let metadata = read_member(store, id, "metadata.json", SMALL_LIMIT, &mut problems);
    let report = read_member(store, id, "result/report.json", REPORT_LIMIT, &mut problems);
    let change_spec = read_member(store, id, "input/change.json", SMALL_LIMIT, &mut problems);
    let state_input = read_member(store, id, "input/state.json", SMALL_LIMIT, &mut problems);
    let bindings = read_member(
        store,
        id,
        "result/bindings.json",
        SMALL_LIMIT,
        &mut problems,
    );
    let search = read_member(store, id, search_member(None), SEARCH_LIMIT, &mut problems);
    parse(
        id,
        RunBytes {
            metadata: metadata.as_deref(),
            report: report.as_deref(),
            change_spec: change_spec.as_deref(),
            state_input: state_input.as_deref(),
            bindings: bindings.as_deref(),
            search: search.as_deref(),
        },
        problems,
    )
}
pub fn from_bytes(id: &str, bytes: RunBytes) -> Run {
    parse(id, bytes, Vec::new())
}
fn parse(id: &str, bytes: RunBytes, mut problems: Vec<String>) -> Run {
    let mut parse_json = |name: &str, bytes: Option<&[u8]>, limit: usize| -> Option<Value> {
        let bytes = bytes?;
        if bytes.len() > limit {
            problems.push(format!("{name}: exceeds read bound"));
            return None;
        }
        serde_json::from_slice(bytes)
            .map_err(|e| problems.push(format!("{name}: {e}")))
            .ok()
    };
    let meta = parse_json("metadata.json", bytes.metadata, SMALL_LIMIT as usize);
    let report = parse_json("report.json", bytes.report, REPORT_LIMIT as usize);
    let change_spec = parse_json("change_spec.json", bytes.change_spec, SMALL_LIMIT as usize);
    let state_input = parse_json("state_input.json", bytes.state_input, SMALL_LIMIT as usize);
    let bindings = parse_json("bindings.json", bytes.bindings, SMALL_LIMIT as usize);
    let search_value = parse_json("migration-search.json", bytes.search, SEARCH_LIMIT as usize);
    let (mut metadata, mut analytical_metadata) = (None, None);
    if let Some(meta) = meta {
        if meta["schema_version"] == crate::local_store::ANALYTICAL_METADATA_VERSION {
            analytical_metadata = serde_json::from_value::<AnalyticalMetadata>(meta)
                .map_err(|e| problems.push(format!("metadata.json: {e}")))
                .ok();
        } else {
            metadata = serde_json::from_value::<Metadata>(meta)
                .map_err(|e| problems.push(format!("metadata.json: {e}")))
                .ok();
        }
    }
    let migration_search = search_value.and_then(|v| {
        serde_json::from_value::<SearchResult>(v)
            .map_err(|e| problems.push(format!("migration-search.json: {e}")))
            .ok()
    });
    let search_sha256 = bytes.search.map(sha256);
    if let Some(m) = &metadata {
        if ![1, 2].contains(&m.schema_version) {
            problems.push("unsupported migration metadata version".into());
        }
        if m.run_id != id {
            problems.push("metadata run ID does not match its directory".into());
        }
        if let Some(r) = &report {
            if r["transition_kind"] != "token_migration" || r["schema_version"] != 1 {
                problems.push("unsupported migration report kind or schema".into());
            }
            for (field, expected) in [
                ("gate_policy", &m.gate_policy),
                ("gate_outcome", &m.gate_outcome),
                ("candidate_program_sha256", &m.candidate_program_sha256),
                ("change_spec_id", &m.change_spec_id),
                ("analysis_input_sha256", &m.analysis_input_sha256),
            ] {
                if r[field].as_str() != Some(expected.as_str()) {
                    problems.push(format!("metadata {field} differs from report.json"));
                }
            }
        }
    }
    if let Some(m) = &analytical_metadata {
        if m.run_id != id {
            problems.push("metadata run ID does not match its directory".into());
        }
        if ![
            "lifecycle_change",
            "current_observation",
            "current_path",
            "current_candidate",
            "current_preflight",
            "current_stress",
        ]
        .contains(&m.kind.as_str())
        {
            problems.push("unsupported analytical record kind".into());
        }
        if bytes.report.map(sha256).as_deref() != Some(m.report_sha256.as_str()) {
            problems.push("metadata report digest differs from report.json".into());
        }
        for (name, actual, expected) in [
            (
                "change_spec",
                bytes.change_spec,
                m.change_spec_sha256.as_ref(),
            ),
            (
                "state_input",
                bytes.state_input,
                m.state_input_sha256.as_ref(),
            ),
        ] {
            if actual.map(sha256).as_ref() != expected {
                problems.push(format!("metadata {name} digest mismatch"));
            }
        }
    }
    if let (Some(s), Some(r)) = (&migration_search, &report) {
        if r["analysis_input_sha256"] != s.analysis_input_sha256
            || r["candidate_program_sha256"] != s.candidate_program_sha256
            || r["state"]["world_sha256"] != s.world_sha256
        {
            problems
                .push("search belongs to a different analytical input, candidate or world".into());
        }
    }
    if let Some(m) = &metadata {
        match (bytes.change_spec, bytes.state_input) {
            (Some(change), Some(state)) => {
                match crate::migration::input::declared_identity(change, state) {
                    Ok((change, state, id)) => {
                        if id != m.analysis_input_sha256
                            || change.id().ok().as_deref() != Some(m.change_spec_id.as_str())
                        {
                            problems.push("analytical input identity mismatch".into());
                        }
                        if let Some(r) = &report {
                            if crate::canonical::digest(&state).ok().as_deref()
                                != r["state_input_sha256"].as_str()
                            {
                                problems.push("state descriptor identity mismatch".into());
                            }
                        }
                    }
                    Err(e) => problems.push(format!("analytical input: {e:#}")),
                }
            }
            _ => problems.push("analytical input documents missing".into()),
        }
    }
    if metadata.is_some() {
        match bindings.as_ref() {
            Some(b) => {
                if b["schema_version"] != 1 || b["kind"] != "token-migration" {
                    problems.push("unsupported migration binding".into());
                }
                if b.get("report_presentation_version")
                    .is_some_and(|v| v != 1 && v != 2)
                {
                    problems.push("unsupported report presentation version".into());
                }
                if let Some(r) = &report {
                    for key in [
                        "analysis_input_sha256",
                        "candidate_program_sha256",
                        "state_input_sha256",
                        "change_spec_id",
                        "gate_policy",
                    ] {
                        if b[key] != r[key] {
                            problems.push(format!("binding {key} differs from report.json"));
                        }
                    }
                }
            }
            None => problems.push("migration bindings missing".into()),
        }
    }
    if let (Some(m), Some(r)) = (&analytical_metadata, &report) {
        let valid = match m.kind.as_str() {
            "lifecycle_change" => {
                r["change"]["kind"] == "lifecycle_change" && r["impact"].is_object()
            }
            "current_observation" => {
                r["kind"] == "current-inspection" && r["execution_performed"] == false
            }
            "current_candidate" => {
                r["kind"] == "current-candidate" && r["official_transition"] == "NotTested"
            }
            "current_preflight" => {
                r["kind"] == "current-preflight" && r["population_readiness"].is_null()
            }
            "current_stress" => r["kind"] == "current-stress",
            "current_path" => {
                r["transition_kind"].is_null()
                    && r["change"]["kind"].is_null()
                    && r["kind"] != "current-inspection"
            }
            _ => false,
        };
        if !valid {
            problems.push("metadata kind differs from report.json".into());
        }
    }
    if let Some(s) = &migration_search {
        if bindings.as_ref().and_then(|b| b["run_id"].as_str()) != Some(s.parent_run.as_str()) {
            problems.push("search engine-run binding mismatch".into());
        }
    }
    let invariants = state_input
        .as_ref()
        .and_then(|s| s.get("invariants"))
        .cloned();
    Run {
        id: id.into(),
        metadata,
        analytical_metadata,
        report,
        change_spec,
        state_input,
        invariants,
        migration_search,
        search_sha256,
        problems,
    }
}
pub fn state(run: &Run) -> &'static str {
    if !run.problems.is_empty() {
        "Unreadable"
    } else if (run.metadata.is_none() && run.analytical_metadata.is_none()) || run.report.is_none()
    {
        "Unfinished"
    } else {
        "Complete"
    }
}
pub fn summary(run: &Run) -> Value {
    if run.is_migration() {
        return super::migration::summary(run);
    }
    let m = run.analytical_metadata.as_ref();
    let r = run.report.as_ref().unwrap_or(&Value::Null);
    json!({"id":run.id,"kind":run.kind(),"state":state(run),"problems":run.problems,
 "timestamp":m.map(|m|&m.timestamp),"run_source":m.map(|m|m.run_source),"eplyx_version":m.map(|m|&m.eplyx_version),"engine_binary_sha256":m.map(|m|&m.engine_binary_sha256),
 "gate":{"outcome":null,"policy":null},"git":{},"transition":{},"invariants":{},"readiness":{},"population":{},"stress":{},"search":null,
 "report_sha256":m.map(|m|&m.report_sha256),"status":r["status"],"claim_basis":"Saved engine output; this view does not re-execute it.","change":r["change"],"limitations":r["limitations"]})
}
pub fn run_summary(store: &Store, id: &str) -> Value {
    summary(&load(store, id))
}
pub struct DetailContext {
    pub bindings: Option<Value>,
    pub artifacts: Vec<Value>,
}
pub fn run_detail(store: &Store, id: &str, counterexamples: &[Value]) -> Result<Value> {
    let run = load(store, id);
    let bindings = read_json(
        store,
        id,
        "result/bindings.json",
        SMALL_LIMIT,
        &mut Vec::new(),
    );
    let sizes = ARTIFACTS
        .iter()
        .map(|(_, path, ..)| {
            let p = parts(id, path);
            let b: Vec<&str> = p.iter().map(String::as_str).collect();
            store
                .open_file(&b, CAPTURE_LIMIT)
                .ok()
                .flatten()
                .map(|(_, len)| len)
        })
        .collect::<Vec<_>>();
    Ok(run_detail_for(
        &run,
        DetailContext {
            bindings,
            artifacts: artifact_rows(id, &sizes),
        },
        counterexamples,
    ))
}
pub fn artifact_rows(id: &str, sizes: &[Option<u64>]) -> Vec<Value> {
    ARTIFACTS
        .iter()
        .zip(sizes.iter().chain(std::iter::repeat(&None)))
        .map(|((name, path, _, label), size)| {
            json!({"name": name, "label": label, "path": format!(".eplyx/runs/{id}/{path}"), "size": size})
        })
        .collect()
}

pub fn run_detail_for(run: &Run, context: DetailContext, counterexamples: &[Value]) -> Value {
    if run.is_migration() {
        return super::migration::detail(run, context, counterexamples);
    }
    let mut result = summary(run);
    result["analysis"] =
        json!({"report":run.report,"change_spec":run.change_spec,"state_input":run.state_input});
    result["evidence"] = json!({"artifacts":artifacts_for_kind(context.artifacts,false),"hashes":{"report_sha256":run.analytical_metadata.as_ref().map(|m|&m.report_sha256)},"bindings":context.bindings});
    result
}
fn load_counterexample(store: &Store, id: &str) -> Result<SavedMigrationCounterexample> {
    crate::local_store::safe_id(id, "cx_")?;
    let bytes = store
        .read(&["counterexamples", &format!("{id}.json")], SEARCH_LIMIT)?
        .context("counterexample missing")?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn counterexample_summary(store: &Store, id: &str) -> Value {
    let mut view = match load_counterexample(store, id) {
        Ok(saved) => super::migration::counterexample_fields(&saved, id),
        Err(e) => json!({"id":id,"state":"Unreadable","problems":[format!("{e:#}")]}),
    };
    view["saved_at_ms"] = json!(store.modified_millis(&["counterexamples", &format!("{id}.json")]));
    view
}
pub fn reproduction_summary(store: &Store, id: &str) -> Value {
    let file = format!("{id}.json");
    let parsed = store
        .read(&["reproductions", &file], SMALL_LIMIT)
        .and_then(|bytes| {
            let bytes = bytes.context("reproduction file missing")?;
            serde_json::from_slice::<Reproduction>(&bytes).context("invalid reproduction record")
        });
    match parsed {
        Ok(record) if record.id == id => {
            let mut value = json!(record);
            value["state"] = json!("Valid");
            value
        }
        Ok(_) => {
            json!({"id": id, "state": "IdentityMismatch", "problems": ["record ID does not match its file name"]})
        }
        Err(error) => json!({"id": id, "state": "Unreadable", "problems": [format!("{error:#}")]}),
    }
}

pub fn counterexample_detail(store: &Store, id: &str, summary: &Value) -> Result<Value> {
    let saved = load_counterexample(store, id)?;
    super::migration::counterexample_detail_for(&saved, summary, &load(store, &saved.parent_run))
}
pub fn compare(store: &Store, left: &str, right: &str) -> Result<Value> {
    compare_runs(&load(store, left), &load(store, right))
}
pub fn compare_runs(a: &Run, b: &Run) -> Result<Value> {
    ensure!(
        a.kind() == b.kind(),
        "Select two runs of the same kind to compare."
    );
    ensure!(
        state(a) == "Complete" && state(b) == "Complete",
        "Both runs must be complete and readable."
    );
    if a.is_migration() {
        return super::migration::compare(a, b);
    }
    Ok(
        json!({"kind":a.kind(),"left":summary(a),"right":summary(b),"inputs":[{"label":"State descriptor","left":a.state_input,"right":b.state_input,"changed":a.state_input!=b.state_input},{"label":"Change specification","left":a.change_spec,"right":b.change_spec,"changed":a.change_spec!=b.change_spec}],"results":[{"label":"Recorded output","left":a.report,"right":b.report,"changed":a.report!=b.report}],"invariants":[],"git":[],"gate":{},"counterexamples":{"items":[],"counts":{},"differences":[]},"causality":"Side-by-side saved results. No causal conclusion or new execution proof is inferred."}),
    )
}
pub fn assemble(
    run_summaries: Vec<Value>,
    counterexample_summaries: Vec<Value>,
    reproductions_newest_first: &[Value],
) -> (Vec<Value>, Vec<Value>) {
    let history: Vec<&Value> = reproductions_newest_first
        .iter()
        .filter(|r| r["state"] == "Valid")
        .collect();
    let mut files: Vec<Value> = counterexample_summaries
        .into_iter()
        .map(|mut file| {
            let mine: Vec<&&Value> = history
                .iter()
                .filter(|r| r["counterexample_id"] == file["id"])
                .collect();
            file["reproductions"] = json!({
                "count": mine.len(),
                "succeeded": mine.iter().filter(|r| r["outcome"] == "Reproduced").count(),
                "failed": mine.iter().filter(|r| r["outcome"] == "Failed").count(),
                "last_timestamp": mine.first().map(|r| r["timestamp"].clone()),
                "last_outcome": mine.first().map(|r| r["outcome"].clone()),
                "history": mine.iter().take(50).collect::<Vec<_>>(),
            });
            file
        })
        .collect();
    let mut runs: Vec<Value> = run_summaries
        .into_iter()
        .enumerate()
        .map(|(position, mut run)| {
            let id = run["id"].as_str().unwrap_or("").to_owned();
            let mine = files.iter().filter(|c| c["parent_run"] == id.as_str());
            let (mut total, mut observed) = (0, 0);
            for c in mine {
                total += 1;
                observed += usize::from(c["kind"] == "Observed");
            }
            run["number"] = json!(position + 1);
            run["saved_counterexamples"] =
                json!({"total": total, "observed": observed, "derived": total - observed});
            run
        })
        .collect();
    runs.reverse();
    for file in &mut files {
        let parent = runs.iter().find(|r| r["id"] == file["parent_run"]);
        file["parent"] = parent.map_or(Value::Null, |run| {
            json!({
                "number": run["number"], "gate": run["gate"]["outcome"], "timestamp": run["timestamp"],
                "source_mint": run["transition"]["source_mint"], "replacement_mint": run["transition"]["replacement_mint"],
                "candidate_program_sha256": run["candidate_program_sha256"],
            })
        });
    }
    let number = |c: &Value| c["parent"]["number"].as_u64().unwrap_or(0);
    files.sort_by(|a, b| {
        number(b)
            .cmp(&number(a))
            .then_with(|| a["kind"].as_str().cmp(&b["kind"].as_str()))
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    (runs, files)
}

/// The Overview/Project payload shared by the local and cloud dashboards.
pub fn project_payload(
    project: Value,
    context: Value,
    store: Value,
    runs: &[Value],
    files: &[Value],
    reproductions: &[Value],
    ignored: usize,
) -> Value {
    let latest = runs.iter().find(|r| r["state"] == "Complete").cloned();
    json!({
        "project": project,
        "context": context,
        "store": store,
        "stats": stats(runs, files, reproductions, ignored),
        "latest": latest,
        "recent_runs": runs.iter().take(6).collect::<Vec<_>>(),
        "recent_counterexamples": files.iter().take(6).collect::<Vec<_>>(),
        "gate_history": runs.iter().take(30).map(|r| json!({"id": r["id"], "number": r["number"], "outcome": r["gate"]["outcome"], "timestamp": r["timestamp"]})).collect::<Vec<_>>(),
    })
}

/// Local usage statistics from `.eplyx/` only; nothing is sent anywhere.
pub fn stats(
    runs: &[Value],
    counterexamples: &[Value],
    reproductions: &[Value],
    ignored: usize,
) -> Value {
    let valid: Vec<&Value> = reproductions
        .iter()
        .filter(|r| r["state"] == "Valid")
        .collect();
    let latest_reproduction = valid.iter().filter_map(|r| r["timestamp"].as_str()).max();
    let outcome = |name: &str| runs.iter().filter(|r| r["gate"]["outcome"] == name).count();
    let timestamps: Vec<&str> = runs
        .iter()
        .filter_map(|r| r["timestamp"].as_str())
        .collect();
    let branches: BTreeSet<&str> = runs
        .iter()
        .filter_map(|r| r["git"]["branch"].as_str())
        .collect();
    let mut kinds = Map::new();
    for kind in ["Observed", "Derived"] {
        kinds.insert(
            kind.into(),
            json!(counterexamples.iter().filter(|c| c["kind"] == kind).count()),
        );
    }
    json!({
        "runs": runs.len(),
        "preflights": runs.iter().filter(|r| r["state"] == "Complete").count(),
        "unfinished_or_unreadable": runs.iter().filter(|r| r["state"] != "Complete").count(),
        "searches": runs.iter().filter(|r| r["search"]["state"] == "Recorded").count(),
        "counterexamples_saved": counterexamples.len(),
        "counterexample_kinds": kinds,
        "passed": outcome("Pass"),
        "warned": outcome("Warn"),
        "blocked": outcome("Block"),
        "offline_reproductions": valid.len(),
        "reproductions_succeeded": valid.iter().filter(|r| r["outcome"] == "Reproduced").count(),
        "reproductions_failed": valid.iter().filter(|r| r["outcome"] == "Failed").count(),
        "latest_reproduction": latest_reproduction,
        "offline_reproductions_note": "Recorded by `eplyx migration reproduce` in .eplyx/reproductions/. Earlier unrecorded attempts are unknown.",
        "run_sources": {
            "local": runs.iter().filter(|r| r["run_source"] == "local").count(),
            "ci": runs.iter().filter(|r| r["run_source"] == "ci").count(),
            "imported": runs.iter().filter(|r| r["run_source"] == "imported").count(),
            "hosted": runs.iter().filter(|r| r["run_source"] == "hosted").count(),
            "not_recorded": runs.iter().filter(|r| r["run_source"].is_null()).count(),
        },
        "first_run": timestamps.iter().min(),
        "latest_run": timestamps.iter().max(),
        "branches": branches,
        "ignored_store_entries": ignored,
    })
}

/// Accept MAIN decimal strings and bounded presentation counters without rounding.
pub fn quantity(v: &Value) -> Option<u64> {
    v.as_str()
        .and_then(|v| v.parse().ok())
        .or_else(|| v.as_u64())
}
pub fn display_value(v: &Value) -> String {
    v.as_str().map(str::to_owned).unwrap_or_else(|| {
        if v.is_null() {
            "Not recorded".into()
        } else {
            v.to_string()
        }
    })
}
