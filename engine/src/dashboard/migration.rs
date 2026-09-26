//! Dashboard views for Token Migration V1 runs. Like every dashboard view, these
//! select and count engine fields and reuse the engine gate evaluator; nothing is
//! replayed, executed or decided here. Executable proposal and state input stay separate.
use super::view::{DetailContext, Run};
use crate::{
    local_store::{replay_inputs, SavedMigrationCounterexample, MIGRATION_COUNTEREXAMPLE_KIND},
    migration::gate::{self as package_gate, Policy},
    migration::search::{self, Counterexample, SearchResult},
};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const KIND: &str = "token_migration";

pub fn is_migration(report: Option<&Value>, change_spec: Option<&Value>) -> bool {
    report.is_some_and(|r| r["transition_kind"] == KIND)
        || change_spec.is_some_and(|m| m["change"]["kind"] == "token_migration")
}

fn status_counts<'a>(items: impl Iterator<Item = &'a Value>, field: &str) -> Value {
    let mut counts = std::collections::BTreeMap::<String, u64>::new();
    for item in items {
        if let Some(status) = item[field].as_str() {
            *counts.entry(status.into()).or_default() += 1;
        }
    }
    json!(counts)
}

fn search_brief(run: &Run) -> Value {
    let Some(search) = &run.migration_search else {
        return if run.search_sha256.is_some() {
            json!({"state": "Unreadable"})
        } else {
            Value::Null
        };
    };
    let observed = search
        .counterexamples
        .iter()
        .filter(|c| matches!(c, Counterexample::MigrationObserved { .. }))
        .count();
    json!({
        "state": "Recorded",
        "conclusion": search.conclusion,
        "observed": observed,
        "derived": search.counterexamples.len() - observed,
        "total": search.counterexamples.len(),
        "budget": search.budget,
        "explored_dimensions": search.explored_dimensions,
        "has_derived_domain": !search.domains.is_empty(),
        "sha256": run.search_sha256,
    })
}

/// A run summary with the shared list fields plus a `migration` block.
pub fn summary(run: &Run) -> Value {
    let report = run.report.as_ref().unwrap_or(&Value::Null);
    let change_spec = run.change_spec.as_ref().unwrap_or(&Value::Null);
    let meta = run.metadata.as_ref();
    let change = &change_spec["change"];
    let invariants = report["invariants"].as_array();
    let axis = |name: &str| report["readiness"][name]["status"].clone();
    json!({
        "id": run.id,
        "kind": KIND,
        "state": super::view::state(run),
        "problems": run.problems,
        "timestamp": meta.map(|m| m.timestamp.clone()),
        "run_source": meta.and_then(|m| m.run_source),
        "eplyx_version": meta.map(|m| m.eplyx_version.clone()),
        "engine_binary_sha256": meta.map(|m| m.engine_binary_sha256.clone()),
        "git": {
            "commit": meta.and_then(|m| m.git_commit.clone()),
            "branch": meta.and_then(|m| m.git_branch.clone()),
            "dirty": meta.and_then(|m| m.git_dirty),
        },
        "candidate_program_sha256": meta.map(|m| m.candidate_program_sha256.clone()).or_else(|| report["candidate_program_sha256"].as_str().map(str::to_owned)),
        "analysis_input_sha256": meta.map(|m| m.analysis_input_sha256.clone()).or_else(|| report["analysis_input_sha256"].as_str().map(str::to_owned)),
        "state_input_sha256": report["state_input_sha256"],
        "gate": {
            "policy": report["gate_policy"].as_str().map(str::to_owned).or_else(|| meta.map(|m| m.gate_policy.clone())),
            "outcome": report["gate_outcome"].as_str().map(str::to_owned).or_else(|| meta.map(|m| m.gate_outcome.clone())),
            "reasons": report["gate_reasons"].as_array().map(Vec::len),
            "reason_codes": report["gate_reason_codes"],
        },
        "conversion": Value::Null,
        "readiness": {
            "mechanism": axis("mechanism"),
            "funding": axis("funding"),
            "population": axis("population"),
            "reconciliation": axis("reconciliation"),
            "analytical": report["declared_preflight_status"],
        },
        "official_transition": report["official_transition"],
        "stress": {
            "selected": report["coverage"]["stress"]["frozen_cases"],
            "executed": report["coverage"]["stress"]["executed"],
            "behaving": report["coverage"]["stress"]["behaving"],
            "deviating": report["coverage"]["stress"]["deviating"],
        },
        "population": {
            "token_accounts_observed": report["impact"]["population"]["token_accounts"],
            "positive_balance_accounts_observed": report["impact"]["population"]["positive_balance_accounts"],
            "enumeration": report["impact"]["population"]["enumeration"],
        },
        "invariants": {
            "total": invariants.map(Vec::len),
            "counts": invariants.map_or(Value::Null, |items| status_counts(items.iter(), "status")),
        },
        "transition": {
            "source_mint": report["migration"]["source"]["mint"].as_str().or(change["source"]["mint"].as_str()),
            "replacement_mint": report["migration"]["destination"]["mint"].as_str().or(change["destination"]["mint"].as_str()),
            "adapter": report["adapter"].as_str().or(change_spec["adapter"].as_str()),
        },
        "migration": {
            "state": report["state"]["world_kind"],
            "cluster": report["state"]["cluster"],
            "source": report["migration"]["source"],
            "destination": report["migration"]["destination"],
            "disposition": report["migration"]["source_disposition"],
            "funding": report["migration"]["destination_funding"]["kind"],
            "attempted": report["coverage"]["rehearsal"]["attempted"],
            "migrated": report["coverage"]["rehearsal"]["migrated"],
            "reconciliation": report["reconciliation"]["status"],
            "required_reserve_raw": report["reconciliation"]["required_reserve_raw"],
            "available_reserve_raw": report["reconciliation"]["available_reserve_raw"],
            "unsigned_units": report["unsigned_plan"]["units"],
        },
        "evaluated_at": report["evaluated_at"],
        "capture_timestamp": Value::Null,
        "search": search_brief(run),
    })
}

fn cx_brief(c: &Counterexample) -> Value {
    let id = search::counterexample_id(c).ok();
    match c {
        Counterexample::MigrationObserved {
            source_account,
            observed_balance_raw,
            finding,
            signature,
            planned_class,
            provenance,
            ..
        } => json!({
            "id": id, "kind": "Observed", "transition_kind": KIND, "key": format!("observed:{source_account}"),
            "account": source_account, "observed_amount_raw": observed_balance_raw,
            "expected": "Migrate", "finding": finding, "planned_class": planned_class, "failure": signature, "provenance": provenance,
        }),
        Counterexample::MigrationDerived {
            source_account,
            dimension,
            case_kind,
            derived_value_raw,
            expected,
            finding,
            signature,
            minimized,
            boundary,
            provenance,
            derived_world_sha256,
            message_sha256,
            ..
        } => json!({
            "id": id, "kind": "Derived", "transition_kind": KIND, "key": format!("derived:{dimension:?}:{source_account}"),
            "account": source_account, "dimension": dimension, "case_kind": case_kind,
            "derived_value_raw": derived_value_raw, "minimized": minimized, "boundary": boundary,
            "expected": expected, "finding": finding, "failure": signature, "provenance": provenance,
            "derived_world_sha256": derived_world_sha256, "message_sha256": message_sha256,
        }),
    }
}

fn gate_views(report: &Value, search: Option<&SearchResult>) -> Value {
    let saved: Option<package_gate::DeploymentGate> =
        serde_json::from_value(report["deployment_gate"].clone()).ok();
    let evaluate = |policy: Policy| match package_gate::evaluate_migration(report, policy) {
        Ok(gate) => json!(gate),
        Err(error) => json!({"error": format!("{error:#}")}),
    };
    let with_search = |policy: Policy| {
        search.map(|result| {
            match package_gate::evaluate_migration_with_counterexamples(report, policy, result) {
                Ok(gate) => json!(gate),
                Err(error) => json!({"error": format!("{error:#}")}),
            }
        })
    };
    let consistent = saved.as_ref().map(|gate| {
        package_gate::evaluate_migration(report, gate.policy)
            .is_ok_and(|expected| &expected == gate)
    });
    json!({
        "saved": report["deployment_gate"],
        "consistent_with_engine": consistent,
        "policies": [
            {"policy": "block-only", "preflight": evaluate(Policy::BlockOnly), "with_search": with_search(Policy::BlockOnly)},
            {"policy": "strict", "preflight": evaluate(Policy::Strict), "with_search": with_search(Policy::Strict)},
        ],
        "basis": "Evaluated by the engine's deployment gate over the saved report.json and, where recorded, the saved migration search. The dashboard does not replay evidence; `eplyx migration gate` and `eplyx migration reproduce` do.",
    })
}

fn command(label: &str, text: String) -> Value {
    json!({"label": label, "command": text})
}

pub fn detail(run: &Run, context: DetailContext, counterexample_files: &[Value]) -> Value {
    let id = run.id.as_str();
    let mut detail = summary(run);
    let report = run.report.clone().unwrap_or(Value::Null);
    let change_spec = run.change_spec.clone().unwrap_or(Value::Null);
    let saved_ids: BTreeSet<&str> = counterexample_files
        .iter()
        .filter(|c| c["parent_run"] == id)
        .filter_map(|c| c["id"].as_str())
        .collect();
    let search_detail = run.migration_search.as_ref().map(|search| {
        json!({
            "version": search.version,
            "conclusion": search.conclusion,
            "domains": search.domains,
            "explored_dimensions": search.explored_dimensions,
            "budget": search.budget,
            "stopping_condition": search.stopping_condition,
            "trace": search.trace,
            "counterexamples": search.counterexamples.iter().map(|c| {
                let mut brief = cx_brief(c);
                let saved = brief["id"].as_str().is_some_and(|cx| saved_ids.contains(cx));
                brief["saved"] = json!(saved);
                brief
            }).collect::<Vec<_>>(),
            "saved_files": saved_ids.len(),
            "sha256": run.search_sha256,
        })
    });
    let mut commands = vec![
        command("Summarise this run", format!("eplyx show {id}")),
        command(
            "Replay offline and gate (recorded policy)",
            format!("eplyx migration gate --run {id}"),
        ),
        command(
            "Replay offline under strict",
            format!("eplyx migration gate --run {id} --policy strict"),
        ),
        command(
            "Export the unsigned execution plan",
            format!("eplyx migration plan --run {id} --out unsigned-plan.json"),
        ),
    ];
    if run.migration_search.is_none() {
        commands.push(command(
            "Search this run",
            format!("eplyx migration search --run {id}"),
        ));
    }
    let answers = json!([
        {"question": "Can this proposed migration execute for the tested states?", "status": report["readiness"]["mechanism"]["status"], "detail": format!("{} of {} attempted holders migrated; {} of {} stress cases behave as specified.", super::view::display_value(&report["coverage"]["rehearsal"]["migrated"]), super::view::display_value(&report["coverage"]["rehearsal"]["attempted"]), super::view::display_value(&report["coverage"]["stress"]["behaving"]), super::view::display_value(&report["coverage"]["stress"]["executed"]))},
        {"question": "How much of the captured population is covered?", "status": report["readiness"]["population"]["status"], "detail": format!("{} token accounts, {} with a balance; enumeration {}.", super::view::display_value(&report["impact"]["population"]["token_accounts"]), super::view::display_value(&report["impact"]["population"]["positive_balance_accounts"]), report["impact"]["population"]["enumeration"].as_str().unwrap_or(""))},
        {"question": "Which accounts or classes cannot migrate?", "status": report["readiness"]["population"]["status"], "detail": report["readiness"]["population"]["codes"]},
        {"question": "Does source → destination accounting reconcile?", "status": report["readiness"]["reconciliation"]["status"], "detail": report["reconciliation"]["status"]},
        {"question": "Is the destination funding sufficient?", "status": report["readiness"]["funding"]["status"], "detail": json!({"required_raw": report["reconciliation"]["required_reserve_raw"], "available_raw": report["reconciliation"]["available_reserve_raw"], "codes": report["readiness"]["funding"]["codes"]})},
        {"question": "What are the concrete failures or counterexamples?", "status": if run.migration_search.as_ref().is_some_and(|s| !s.counterexamples.is_empty()) || super::view::quantity(&report["coverage"]["stress"]["deviating"]).unwrap_or(0) > 0 { "Found" } else { "None found" }, "detail": json!({"deviating_stress_cases": report["coverage"]["stress"]["deviating"], "counterexamples": run.migration_search.as_ref().map(|s| s.counterexamples.len())})},
        {"question": "What are the evidence limitations?", "status": "Stated", "detail": report["limitations"]},
        {"question": "How can a failure be reproduced?", "status": "Offline", "detail": "`eplyx migration reproduce <cx-id>` replays the saved run and search in the local VM with no RPC."},
    ]);
    let mut hashes = json!({
        "analysis_input_sha256": report["analysis_input_sha256"],
        "candidate_program_sha256": report["candidate_program_sha256"],
        "state_input_sha256": report["state_input_sha256"],
        "change_spec_id": report["change_spec_id"],
        "world_sha256": report["state"]["world_sha256"],
        "search_sha256": run.search_sha256,
        "engine_binary_sha256": run.metadata.as_ref().map(|m| m.engine_binary_sha256.clone()),
    });
    // Each result artifact's digest as its own row, keyed by file name.
    if let (Some(target), Some(artifacts)) =
        (hashes.as_object_mut(), report["artifacts"].as_object())
    {
        for (name, digest) in artifacts {
            target.insert(name.clone(), digest.clone());
        }
    }
    let extra = json!({
        "answers": answers,
        "release": {
            "program_id": change_spec["change"]["mechanism"]["program_id"],
            "packaged_artifact": change_spec["change"]["mechanism"]["artifact"],
            "change_spec_schema_version": change_spec["schema_version"],
            "adapter": report["adapter"],
            "change": change_spec["change"],
            "invariant_definitions": run.invariants.as_ref().unwrap_or(&Value::Null),
            "provenance": report["provenance"],
            "deployment_origin": report["deployment_origin"],
            "local_run_id": run.id,
        },
        "migration_detail": {
            "migration": report["migration"],
            "state": report["state"],
            "impact": report["impact"],
            "reconciliation": report["reconciliation"],
            "compatibility": report["compatibility"],
            "authority": report["authority"],
            "coverage": report["coverage"],
            "execution": report["execution"],
            "readiness": report["readiness"],
            "unsigned_plan": report["unsigned_plan"],
            "limitations": report["limitations"],
        },
        "search_detail": search_detail,
        "invariant_results": report["invariants"],
        "gate_detail": gate_views(&report, run.migration_search.as_ref()),
        "evidence": {
            "artifacts": super::view::artifacts_for_kind(context.artifacts, true),
            "bindings": context.bindings,
            "hashes": hashes,
            "commands": commands,
        },
    });
    if let (Some(target), Some(source)) = (detail.as_object_mut(), extra.as_object()) {
        target.extend(source.clone());
    }
    detail
}

pub fn counterexample_fields(saved: &SavedMigrationCounterexample, file_id: &str) -> Value {
    let computed = search::counterexample_id(&saved.counterexample).ok();
    let identity_verified = computed.as_deref() == Some(file_id)
        && saved.id == file_id
        && saved.kind == MIGRATION_COUNTEREXAMPLE_KIND
        && saved.replay_inputs == replay_inputs(&saved.parent_run);
    let mut view = cx_brief(&saved.counterexample);
    let (tp, cps, world) = match &saved.counterexample {
        Counterexample::MigrationObserved {
            analysis_input_sha256,
            candidate_program_sha256,
            world_sha256,
            ..
        }
        | Counterexample::MigrationDerived {
            analysis_input_sha256,
            candidate_program_sha256,
            world_sha256,
            ..
        } => (
            analysis_input_sha256,
            candidate_program_sha256,
            world_sha256,
        ),
    };
    view["id"] = json!(file_id);
    view["claim"] = json!(saved.counterexample.claim());
    view["parent_run"] = json!(saved.parent_run);
    view["package_run"] = json!(saved.parent_run);
    view["search_sha256"] = json!(saved.search_sha256);
    view["analysis_input_sha256"] = json!(tp);
    view["candidate_program_sha256"] = json!(cps);
    view["world_sha256"] = json!(world);
    view["identity_verified"] = json!(identity_verified);
    view["state"] = json!(if identity_verified {
        "Valid"
    } else {
        "IdentityMismatch"
    });
    view["reproduce"] = json!(format!("eplyx migration reproduce {file_id}"));
    view["limitations"] = json!(match &saved.counterexample {
        Counterexample::MigrationObserved { limitations, .. }
        | Counterexample::MigrationDerived { limitations, .. } => limitations,
    });
    view
}

pub fn counterexample_detail_for(
    saved: &SavedMigrationCounterexample,
    summary: &Value,
    parent: &Run,
) -> Result<Value> {
    let mut view = summary.clone();
    view["replay_inputs"] = json!({
        "input": format!(".eplyx/{}", saved.replay_inputs.input),
        "result": format!(".eplyx/{}", saved.replay_inputs.result),
        "search": format!(".eplyx/{}", saved.replay_inputs.search),
    });
    view["search_artifact_matches"] =
        json!(parent.search_sha256.as_deref() == Some(saved.search_sha256.as_str()));
    view["search_context"] = json!(parent.migration_search.as_ref().map(|search| json!({
        "conclusion": search.conclusion,
        "domains": search.domains,
        "budget": search.budget,
        "total": search.counterexamples.len(),
    })));
    if let Counterexample::MigrationDerived {
        mutations,
        reserve_override_raw,
        transaction_variant,
        ..
    } = &saved.counterexample
    {
        view["reproduction_inputs"] = json!({"mutations": mutations, "reserve_override_raw": reserve_override_raw, "transaction_variant": transaction_variant});
    }
    view["raw"] = serde_json::to_value(saved)?;
    Ok(view)
}

fn field(group: &str, label: &str, left: Value, right: Value) -> Value {
    json!({"group": group, "label": label, "changed": left != right, "left": left, "right": right})
}

/// Semantic comparison of two migration runs. Index 0 stays the candidate hash.
pub fn compare(a: &Run, b: &Run) -> Result<Value> {
    let (ra, rb) = (
        a.report.clone().unwrap_or(Value::Null),
        b.report.clone().unwrap_or(Value::Null),
    );
    let pick = |r: &Value, path: &[&str]| -> Value {
        let mut v = r;
        for p in path {
            v = &v[*p];
        }
        v.clone()
    };
    let pair = |group: &str, label: &str, path: &[&str]| {
        field(group, label, pick(&ra, path), pick(&rb, path))
    };
    let inputs = vec![
        pair(
            "Candidate",
            "Candidate program hash",
            &["candidate_program_sha256"],
        ),
        pair(
            "Candidate",
            "Analytical input hash",
            &["analysis_input_sha256"],
        ),
        pair(
            "Candidate",
            "State descriptor hash",
            &["state_input_sha256"],
        ),
        pair("Candidate", "Specification hash", &["change_spec_id"]),
        pair("Candidate", "Adapter", &["adapter"]),
        pair("Candidate", "Program ID", &["program_id"]),
        pair("Migration", "Source asset", &["migration", "source"]),
        pair(
            "Migration",
            "Destination asset",
            &["migration", "destination"],
        ),
        pair(
            "Migration",
            "Conversion terms",
            &["migration", "conversion", "declared"],
        ),
        pair(
            "Migration",
            "Effective raw terms",
            &["migration", "conversion", "effective_raw_terms"],
        ),
        pair("Migration", "Window", &["migration", "window"]),
        pair(
            "Migration",
            "Source disposition",
            &["migration", "source_disposition"],
        ),
        pair(
            "Migration",
            "Destination funding",
            &["migration", "destination_funding"],
        ),
        pair("Migration", "Eligibility", &["migration", "eligibility"]),
        field(
            "Policy",
            "Declared invariants",
            a.invariants.clone().unwrap_or(Value::Null),
            b.invariants.clone().unwrap_or(Value::Null),
        ),
        pair("Policy", "Gate policy", &["gate_policy"]),
    ];
    let results = vec![
        pair("State", "World", &["state", "world_kind"]),
        pair("State", "World digest", &["state", "world_sha256"]),
        pair(
            "Population",
            "Token accounts",
            &["impact", "population", "token_accounts"],
        ),
        pair(
            "Population",
            "Positive balances",
            &["impact", "population", "positive_balance_accounts"],
        ),
        pair("Population", "Classes", &["impact", "classes"]),
        pair(
            "Rehearsal",
            "Attempted holders",
            &["coverage", "rehearsal", "attempted"],
        ),
        pair(
            "Rehearsal",
            "Migrated holders",
            &["coverage", "rehearsal", "migrated"],
        ),
        pair(
            "Rehearsal",
            "Stress cases behaving",
            &["coverage", "stress", "behaving"],
        ),
        pair(
            "Rehearsal",
            "Stress cases deviating",
            &["coverage", "stress", "deviating"],
        ),
        pair(
            "Readiness",
            "Mechanism",
            &["readiness", "mechanism", "status"],
        ),
        pair("Readiness", "Funding", &["readiness", "funding", "status"]),
        pair(
            "Readiness",
            "Population",
            &["readiness", "population", "status"],
        ),
        pair(
            "Readiness",
            "Reconciliation",
            &["readiness", "reconciliation", "status"],
        ),
        pair(
            "Accounting",
            "Reconciliation status",
            &["reconciliation", "status"],
        ),
        pair(
            "Accounting",
            "Required reserve",
            &["reconciliation", "required_reserve_raw"],
        ),
        pair(
            "Accounting",
            "Available reserve",
            &["reconciliation", "available_reserve_raw"],
        ),
        pair("Readiness", "Official transition", &["official_transition"]),
    ];
    let invariant_map = |r: &Value| -> std::collections::BTreeMap<String, Value> {
        r["invariants"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| Some((i["invariant_type"].as_str()?.to_owned(), i.clone())))
            .collect()
    };
    let (ia, ib) = (invariant_map(&ra), invariant_map(&rb));
    let kinds: BTreeSet<&String> = ia.keys().chain(ib.keys()).collect();
    let invariants: Vec<Value> = kinds
        .into_iter()
        .map(|kind| {
            let (l, r) = (ia.get(kind), ib.get(kind));
            let status = |v: Option<&Value>| v.map_or(Value::Null, |i| i["status"].clone());
            json!({
                "invariant_id": l.or(r).map_or(Value::Null, |i| i["invariant_id"].clone()),
                "invariant_type": kind,
                "severity": l.or(r).map_or(Value::Null, |i| i["severity"].clone()),
                "left": status(l), "right": status(r), "changed": status(l) != status(r),
                "left_explanation": l.map(|i| i["explanation"].clone()),
                "right_explanation": r.map(|i| i["explanation"].clone()),
            })
        })
        .collect();
    let set = |r: &Value, field: &str| -> BTreeSet<String> {
        r[field]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect()
    };
    let (ga, gb) = (set(&ra, "gate_reasons"), set(&rb, "gate_reasons"));
    let (ca, cb) = (set(&ra, "gate_reason_codes"), set(&rb, "gate_reason_codes"));
    let meta = |run: &Run, f: fn(&crate::local_store::Metadata) -> Value| {
        run.metadata.as_ref().map_or(Value::Null, f)
    };
    let git = vec![
        field(
            "Git",
            "Run source",
            meta(a, |m| json!(m.run_source)),
            meta(b, |m| json!(m.run_source)),
        ),
        field(
            "Git",
            "Commit",
            meta(a, |m| json!(m.git_commit)),
            meta(b, |m| json!(m.git_commit)),
        ),
        field(
            "Git",
            "Branch",
            meta(a, |m| json!(m.git_branch)),
            meta(b, |m| json!(m.git_branch)),
        ),
        field(
            "Git",
            "Uncommitted changes",
            meta(a, |m| json!(m.git_dirty)),
            meta(b, |m| json!(m.git_dirty)),
        ),
        field(
            "Tooling",
            "Eplyx version",
            meta(a, |m| json!(m.eplyx_version)),
            meta(b, |m| json!(m.eplyx_version)),
        ),
        field(
            "Tooling",
            "Engine binary hash",
            meta(a, |m| json!(m.engine_binary_sha256)),
            meta(b, |m| json!(m.engine_binary_sha256)),
        ),
    ];
    Ok(json!({
        "kind": KIND,
        "left": summary(a),
        "right": summary(b),
        "inputs": inputs,
        "results": results,
        "invariants": invariants,
        "gate": {
            "left": {"outcome": ra["gate_outcome"], "policy": ra["gate_policy"]},
            "right": {"outcome": rb["gate_outcome"], "policy": rb["gate_policy"]},
            "changed": ra["gate_outcome"] != rb["gate_outcome"] || ra["gate_policy"] != rb["gate_policy"],
            "reasons_added": gb.difference(&ga).collect::<Vec<_>>(),
            "reasons_removed": ga.difference(&gb).collect::<Vec<_>>(),
            "reasons_kept": ga.intersection(&gb).count(),
            "codes_added": cb.difference(&ca).collect::<Vec<_>>(),
            "codes_removed": ca.difference(&cb).collect::<Vec<_>>(),
        },
        "git": git,
        "counterexamples": counterexample_diff(a, b),
        "causality": "Differences are listed side by side. Eplyx does not infer which input change caused which result change.",
    }))
}

fn semantic_key(c: &Counterexample) -> String {
    match c {
        Counterexample::MigrationObserved { source_account, .. } => {
            format!("observed:{source_account}")
        }
        Counterexample::MigrationDerived {
            source_account,
            dimension,
            ..
        } => {
            format!("derived:{dimension:?}:{source_account}")
        }
    }
}

/// Whether two searches ran under identical conditions: same search version,
/// same rehearsal world, same declared domains and the same budget maxima.
fn conditions(a: Option<&SearchResult>, b: Option<&SearchResult>) -> (bool, Vec<String>) {
    let (Some(a), Some(b)) = (a, b) else {
        return (
            false,
            vec!["At least one run has no recorded migration search.".into()],
        );
    };
    let mut differences = vec![];
    if a.version != b.version {
        differences.push(format!("Search version {} vs {}", a.version, b.version));
    }
    if a.world_sha256 != b.world_sha256 {
        differences
            .push("The rehearsal worlds differ (different captured or fixture state).".into());
    }
    if a.domains != b.domains {
        differences.push("The declared search domains differ.".into());
    }
    if (a.budget.max_probes, a.budget.max_minimization)
        != (b.budget.max_probes, b.budget.max_minimization)
    {
        differences.push("The search budgets differ.".into());
    }
    (differences.is_empty(), differences)
}

fn counterexample_diff(a: &Run, b: &Run) -> Value {
    let (comparable, differences) =
        conditions(a.migration_search.as_ref(), b.migration_search.as_ref());
    let index = |run: &Run| -> std::collections::BTreeMap<String, Value> {
        let mut map = std::collections::BTreeMap::new();
        for c in run.migration_search.iter().flat_map(|s| &s.counterexamples) {
            let base = semantic_key(c);
            let (mut key, mut n) = (base.clone(), 1);
            while map.contains_key(&key) {
                n += 1;
                key = format!("{base}#{n}");
            }
            map.insert(key, cx_brief(c));
        }
        map
    };
    let (left, right) = (index(a), index(b));
    // A derived state counts as re-executed only when B's trace ran the same
    // dimension and value and it behaved as specified.
    let retested_derived = |brief: &Value| -> bool {
        b.migration_search.as_ref().is_some_and(|s| {
            s.parent_source_account.as_deref() == brief["account"].as_str()
                && s.trace.iter().any(|p| {
                    serde_json::to_value(p.dimension).ok().as_ref() == Some(&brief["dimension"])
                        && p.value_raw.as_ref().map(|v| json!(v))
                            == Some(brief["derived_value_raw"].clone())
                        && p.kind == brief["case_kind"]
                        && p.derived_world_sha256 == brief["derived_world_sha256"]
                        && p.message_sha256 == brief["message_sha256"]
                        && serde_json::to_value(p.expected).ok().as_ref()
                            == Some(&brief["expected"])
                        && p.behaves_as_specified
                })
        })
    };
    let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    let mut counts = std::collections::BTreeMap::<&str, u64>::new();
    let mut items = vec![];
    for key in keys {
        let (l, r) = (left.get(key), right.get(key));
        let account = l.or(r).and_then(|v| v["account"].as_str()).unwrap_or("");
        let observed = key.starts_with("observed:");
        let (status, note) = match (l, r) {
            (Some(l), Some(r)) => {
                if (
                    l["finding"].clone(),
                    l["failure"]["error_name"].clone(),
                    l["derived_value_raw"].clone(),
                ) != (
                    r["finding"].clone(),
                    r["failure"]["error_name"].clone(),
                    r["derived_value_raw"].clone(),
                ) {
                    (
                        "changed",
                        "Found in both runs; the finding, failure or value differs.".to_string(),
                    )
                } else {
                    (
                        "persistent",
                        "Found in both runs with the same finding and failure.".into(),
                    )
                }
            }
            (None, Some(_)) => {
                if comparable {
                    (
                        "new",
                        "Run A searched under identical conditions and did not find this.".into(),
                    )
                } else {
                    (
                        "only_right",
                        "Run A did not test this state under identical conditions.".into(),
                    )
                }
            }
            (Some(l), None) => {
                if b.migration_search.is_none() {
                    (
                        "only_left",
                        "Run B has no recorded search, so this was not re-tested.".into(),
                    )
                } else if comparable
                    && (if observed {
                        b.report.as_ref().is_some_and(|r| {
                            r["execution"]["units"].as_array().is_some_and(|units| {
                                units.iter().any(|u| {
                                    u["source_account"] == l["account"]
                                        && u["outcome"] == "Migrated"
                                })
                            })
                        })
                    } else {
                        retested_derived(l)
                    })
                {
                    ("resolved", "Run B re-executed this exact state under identical search conditions and it behaved as specified. This conclusion covers only that exact state.".into())
                } else {
                    ("only_left", "Absent from run B, which did not re-execute this state under identical conditions.".into())
                }
            }
            (None, None) => continue,
        };
        *counts.entry(status).or_default() += 1;
        items.push(json!({"key": key, "kind": if observed { "Observed" } else { "Derived" }, "account": account, "status": status, "note": note, "left": l, "right": r}));
    }
    json!({
        "left_total": left.len(),
        "right_total": right.len(),
        "comparable": comparable,
        "differences": differences,
        "counts": counts,
        "items": items,
        "identity": "Matched by source token account (and search dimension for derived states). Local cx_ IDs bind the engine run, so they never match across runs.",
    })
}
