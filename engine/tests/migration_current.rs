//! T4 source contract: fixed selection, final-state rebinding, bounded waves and replay.
#[path = "common/current.rs"]
mod common;
use common::*;
use eplyx_engine::{
    evidence::paths::PathStatus,
    migration::{current, current_select, execute::Outcome, observed_search, population},
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[test]
fn every_selected_case_executes_at_its_full_final_balance_and_replays() {
    let p = prepare("18000");
    let frozen = p.plan();
    let report = p.run(&p.rpc);
    assert_eq!(report.results.len(), 3);
    let total: u64 = report
        .results
        .iter()
        .map(|r| {
            r.execution
                .as_ref()
                .unwrap()
                .deltas
                .reserve_debit_raw
                .parse::<u64>()
                .unwrap()
        })
        .sum();
    assert!(
        total > 18000,
        "independent cases must not consume one shared reserve bank"
    );
    assert_eq!(report.coverage["exact_accounts_executed"], 3);
    for (case, r) in frozen.selection.selected.iter().zip(&report.results) {
        assert_eq!(r.token_account, case.token_account);
        assert_eq!(r.status, PathStatus::Proven);
        assert!(
            r.execution_performed
                && r.signer_assumed_locally
                && r.execution.as_ref().unwrap().reconciled
        );
        assert_eq!(
            r.final_amount_raw.as_deref(),
            Some(case.observed_balance_raw.as_str())
        );
        assert_eq!(
            r.execution.as_ref().unwrap().deltas.source_debit_raw,
            case.observed_balance_raw
        );
        assert_eq!(r.official_transition, PathStatus::NotTested);
        assert!(!r.signer_possession_known && !r.funds_moved && !r.issuer_binding_established);
        assert_eq!(r.classification, "ExecutableCurrentState");
    }
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(p.replay()).unwrap()
    );
    assert_eq!(
        report.coverage["population_rollout_readiness"],
        "Incomplete"
    );
    assert!(report
        .authority_resolution
        .cases
        .iter()
        .all(|c| !c.signer_assumed_locally && !c.execution_supported));
    assert!(
        report.coverage["positive_balance_accounts_observed"]
            .as_u64()
            .unwrap()
            > 3
    );
    let amounts: BTreeSet<_> = report
        .results
        .iter()
        .map(|r| r.final_amount_raw.clone())
        .collect();
    assert_eq!(amounts.len(), 3);
}
#[test]
fn full_at_final_capture_reports_bucket_drift_without_rewriting_selection() {
    let p = prepare("1000000000");
    let plan = p.plan();
    let source = plan.selection.selected[0].token_account.clone();
    let rpc = Alter {
        rpc: &p.rpc,
        change: |params: &Value, response: &mut Value| {
            if let Some(raw) = source_at(params, response, &source) {
                token(raw, |a| a.amount = 50);
            }
        },
    };
    let report = p.run(&rpc);
    let r = &report.results[0];
    assert_eq!(r.token_account, source);
    assert_eq!(
        r.selected_amount_raw,
        plan.selection.selected[0].observed_balance_raw
    );
    assert_eq!(r.final_amount_raw.as_deref(), Some("50"));
    assert_eq!(r.status, PathStatus::Proven);
    assert_eq!(r.classification, "SelectionStateChangedButExecutable");
    assert!(!r.selection_bucket_preserved);
    assert!(r.selection_shape_preserved);
    assert!(r.changed_fields.contains(&"token_raw_amount".into()));
    assert_eq!(r.execution.as_ref().unwrap().amount_raw, "50");
    assert_eq!(r.execution.as_ref().unwrap().deltas.source_debit_raw, "50");
    p.replay();
}
#[test]
fn executable_shape_drift_qualifies_discovery_coverage() {
    let p = prepare("1000000000");
    let source = p.plan().selection.selected[0].token_account.clone();
    let rpc = Alter {
        rpc: &p.rpc,
        change: |params: &Value, response: &mut Value| {
            if let Some(raw) = source_at(params, response, &source) {
                token(raw, |a| a.close_authority = Some(a.owner).into());
            }
        },
    };
    let report = p.run(&rpc);
    let r = &report.results[0];
    assert_eq!(r.status, PathStatus::Proven);
    assert!(!r.selection_shape_preserved);
    assert_eq!(r.classification, "SelectionStateChangedButExecutable");
    let row = report.coverage["shape_coverage"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["state_shape_sha256"] == r.selection_shape_sha256)
        .unwrap();
    assert!(!row["exact_execution_entity_ids"]
        .as_array()
        .unwrap()
        .contains(&json!(r.entity_id)));
    p.replay();
}
#[test]
fn underfunded_final_amount_fails_in_vm_with_exact_rollback() {
    let p = prepare("1");
    let report = p.run(&p.rpc);
    for r in &report.results {
        assert_eq!(r.status, PathStatus::Failed);
        let e = r.execution.as_ref().unwrap();
        assert_eq!(e.outcome, Outcome::Rejected);
        assert!(e.failure.as_ref().unwrap().rollback_verified);
        assert!(e.unexpected_changes.is_empty());
        assert!(e.reconciliation.iter().all(|c| c.holds));
        assert_eq!(e.amount_raw, r.final_amount_raw.as_deref().unwrap());
    }
    p.replay();
}
#[test]
fn lamports_only_change_keeps_final_bytes_and_executes() {
    let p = prepare("1000000000");
    let source = p.plan().selection.selected[0].token_account.clone();
    let rpc = Alter {
        rpc: &p.rpc,
        change: |params: &Value, response: &mut Value| {
            if let Some(raw) = source_at(params, response, &source) {
                raw["lamports"] = json!(123456789);
            }
        },
    };
    let report = p.run(&rpc);
    assert_eq!(report.results[0].status, PathStatus::Proven);
    assert!(report.results[0]
        .changed_fields
        .contains(&"lamports".into()));
    assert_eq!(report.results[0].classification, "ExecutableCurrentState");
    p.replay();
}
#[test]
fn missing_zero_frozen_authority_and_program_drift_never_execute_a_peer() {
    for variant in 0..5 {
        let p = prepare("1000000000");
        let plan = p.plan();
        let source = plan.selection.selected[0].token_account.clone();
        let rpc = Alter {
            rpc: &p.rpc,
            change: |params: &Value, response: &mut Value| {
                if let Some(raw) = source_at(params, response, &source) {
                    match variant {
                        0 => *raw = Value::Null,
                        1 => token(raw, |a| a.amount = 0),
                        2 => token(raw, |a| {
                            a.state = spl_token_2022_interface::state::AccountState::Frozen
                        }),
                        3 => token(raw, |a| {
                            a.owner = solana_address::Address::new_from_array([99; 32])
                        }),
                        _ => raw["owner"] = json!("11111111111111111111111111111111"),
                    }
                }
            },
        };
        let report = p.run(&rpc);
        let r = &report.results[0];
        assert_eq!(r.token_account, source);
        assert_eq!(r.status, PathStatus::Indeterminate);
        assert!(!r.execution_performed);
        assert_eq!(report.results.len(), plan.selection.selected.len());
        p.replay();
    }
}
#[test]
fn population_pointer_names_original_row_after_provider_reordering() {
    let p = prepare("1000000000");
    let pop: population::Capture = serde_json::from_slice(&p.population).unwrap();
    let obs = population::evaluate_bytes(&p.population, &p.budget).unwrap();
    for e in &obs.entities {
        let pointer = e
            .token_account_evidence
            .pointer
            .strip_suffix("/account")
            .unwrap();
        let row = pop.observations[e.token_account_evidence.rpc_id]
            .result
            .as_ref()
            .unwrap()
            .pointer(pointer)
            .unwrap();
        assert_eq!(row["pubkey"], e.token_account);
    }
}
#[test]
fn frozen_selection_and_capture_tampering_fail_offline_replay() {
    for variant in 0..3 {
        let p = prepare("1000000000");
        p.run(&p.rpc);
        let path = p.root.path().join("current").join(if variant == 0 {
            "current.plan.json"
        } else {
            "current.capture.json"
        });
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        if variant == 0 {
            value["selection"]["selected"][0]["token_account"] =
                value["selection"]["selected"][1]["token_account"].clone();
        } else if variant == 1 {
            value["cases"][0]["case_id"] = "peer".into();
        } else {
            value["cases"][0]["observations"][1]["result"]["context"]["slot"] = json!(1);
        }
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        assert!(current::replay(&p.input, &p.root.path().join("current"), &p.budget).is_err());
    }
}
#[test]
fn refreshed_population_cannot_inherit_frozen_selection() {
    let p = prepare("1000000000");
    let plan = p.plan();
    let mut pop: population::Capture = serde_json::from_slice(&p.population).unwrap();
    pop.run_id = "refreshed".into();
    let bytes = serde_json::to_vec(&pop).unwrap();
    let obs = population::evaluate_bytes(&bytes, &p.budget).unwrap();
    assert!(plan
        .selection
        .validate(
            &obs,
            &current_select::CandidatePlan::new(&p.input),
            p.input.program_sha256()
        )
        .is_err());
}
#[test]
fn observed_waves_freeze_before_capture_keep_failures_and_replay() {
    let p = prepare("1");
    let baseline = p.run(&p.rpc);
    let parent = p.root.path().join("current");
    let output = p.root.path().join("search");
    let search = observed_search::run_with(&p.input, &parent, &p.budget, &output, &p.rpc).unwrap();
    assert_eq!(search.additional_selected, 25);
    assert_eq!(search.counterexamples.len(), 25);
    let mut identities: BTreeSet<_> = baseline
        .results
        .iter()
        .map(|r| r.token_account.clone())
        .collect();
    for wave in &search.waves {
        for r in &wave.results {
            assert!(identities.insert(r.token_account.clone()));
            assert_eq!(r.status, PathStatus::Failed);
            assert!(
                r.execution
                    .as_ref()
                    .unwrap()
                    .failure
                    .as_ref()
                    .unwrap()
                    .rollback_verified
            );
        }
    }
    assert_eq!(
        serde_json::to_value(&search).unwrap(),
        serde_json::to_value(
            observed_search::replay(&p.input, &parent, &p.budget, &output).unwrap()
        )
        .unwrap()
    );
    let mut freeze: Value =
        serde_json::from_slice(&std::fs::read(output.join("wave-1.freeze.json")).unwrap()).unwrap();
    freeze["parent_plan_sha256"] = "0".repeat(64).into();
    std::fs::write(
        output.join("wave-1.freeze.json"),
        serde_json::to_vec(&freeze).unwrap(),
    )
    .unwrap();
    assert!(observed_search::replay(&p.input, &parent, &p.budget, &output).is_err());
}

#[test]
fn final_coherence_retry_is_monotonic_bounded_and_never_replaces_source() {
    use eplyx_engine::standard_programs::{clock::CapturedClock, token::CLOCK};
    for mode in 0..3 {
        let p = prepare("1000000000");
        let plan = p.plan();
        let source = plan.selection.selected[0].token_account.clone();
        let floor = p.rpc.slot;
        let rpc = Alter {
            rpc: &p.rpc,
            change: |params: &Value, response: &mut Value| {
                assert!(
                    p.root.path().join("current/current.plan.json").is_file(),
                    "selection must be durable before any final read"
                );
                let min = params[1]["minContextSlot"].as_u64().unwrap();
                let clock = source_at(params, response, CLOCK).unwrap();
                data(clock, |bytes| {
                    let mut c = CapturedClock::from_bytes(bytes).unwrap();
                    c.slot = if mode == 0 || min == floor {
                        min + 2
                    } else {
                        min
                    };
                    *bytes = c.bytes();
                });
                if mode == 2 && min > floor {
                    if let Some(raw) = source_at(params, response, &source) {
                        token(raw, |a| a.amount -= 1);
                    }
                }
            },
        };
        let report = p.run(&rpc);
        let r = &report.results[0];
        if mode == 1 {
            assert_eq!(r.status, PathStatus::Proven);
            let c = r.execution_context.as_ref().unwrap();
            assert_eq!(c.attempts.len(), 2);
            assert_eq!(c.attempts[1].min_context_slot, floor + 3);
            assert!(!c.atomic_single_slot);
        } else {
            assert_eq!(r.status, PathStatus::Indeterminate);
            assert!(!r.execution_performed);
            if mode == 2 {
                assert!(r.reason.as_deref().unwrap().contains("SourceStateChanged"));
            }
        }
        assert_eq!(r.token_account, source);
        assert_eq!(report.results.len(), 3);
        p.replay();
    }
}
#[test]
fn unavailable_final_capture_stays_selected_and_indeterminate() {
    struct Unavailable;
    impl eplyx_engine::ingest::rpc::RpcProvider for Unavailable {
        fn call(&self, method: &str, _: Value) -> anyhow::Result<Value> {
            if method == "getGenesisHash" {
                return Ok(json!(eplyx_engine::migration::world::MAINNET_GENESIS));
            }
            assert_eq!(method, "getMultipleAccounts");
            anyhow::bail!("fixture read failure")
        }
    }
    let p = prepare("1000000000");
    let report = p.run(&Unavailable);
    assert_eq!(report.results.len(), 3);
    assert!(report
        .results
        .iter()
        .all(|r| r.status == PathStatus::Indeterminate && !r.execution_performed));
    p.replay();
}
#[test]
fn healthy_observed_waves_make_no_counterexample_or_peer_claim() {
    let p = prepare("1000000000");
    p.run(&p.rpc);
    let parent = p.root.path().join("current");
    let output = p.root.path().join("search");
    let rpc = Alter {
        rpc: &p.rpc,
        change: |_: &Value, _: &mut Value| {
            assert!(output.join("wave-1.plan.json").is_file());
            assert!(output.join("wave-1.freeze.json").is_file());
        },
    };
    let result = observed_search::run_with(&p.input, &parent, &p.budget, &output, &rpc).unwrap();
    assert_eq!(result.additional_selected, 25);
    assert!(result.counterexamples.is_empty());
    assert_eq!(
        result.conclusion,
        eplyx_engine::migration::search::NO_FINDING
    );
    assert!(result
        .waves
        .iter()
        .flat_map(|w| &w.results)
        .all(|r| r.status == PathStatus::Proven && r.official_transition == PathStatus::NotTested));
}

#[test]
fn current_findings_feed_the_main_migration_gate_and_replay_offline() {
    use eplyx_engine::migration::{
        gate::Policy,
        pipeline::{self, Isolation, ObservedSource},
    };
    use eplyx_engine::standard_programs::token::CLOCK;
    let p = prepare("1000000000");
    let source = p.plan().selection.selected[0].token_account.clone();
    let rpc = Alter {
        rpc: &p.rpc,
        change: |params: &Value, response: &mut Value| {
            if params[0].as_array().unwrap().iter().any(|a| a == CLOCK) {
                if let Some(raw) = source_at(params, response, &source) {
                    token(raw, |a| a.amount = 100_000_000);
                }
            }
        },
    };
    let output = p.root.path().join("analysis");
    let report = pipeline::run_with(
        p.input.root(),
        &output,
        Policy::BlockOnly,
        Isolation::InProcess,
        Some(ObservedSource {
            population: &p.rpc,
            execution: &rpc,
            budget: p.budget.clone(),
        }),
    )
    .unwrap();
    assert_eq!(pipeline::exit_code(&report).unwrap(), 1);
    assert!(report["gate_reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("CURRENT_CASE_FAILED")));
    assert_eq!(
        report["current_state_guarantees"]["results"][0]["token_account"],
        source
    );
    assert_eq!(
        report["current_state_guarantees"]["results"][0]["status"],
        "Failed"
    );
    assert_eq!(pipeline::replay(p.input.root(), &output).unwrap(), report);
}
