//! Offline frozen-world counterfactual invariants; no VM or network operations.
use eplyx_engine::{
    lifecycle::artifact::reference_root as repo_root,
    lifecycle::counterfactual::{
        compare, CounterfactualReport, DiffCategory, FrozenCounterfactualWorld, GateApplicability,
    },
    lifecycle::frozen::selection::{digest, load},
    lifecycle::readiness::{LifecycleReadinessReport, ReadinessStatus},
    lifecycle::resolution::PathStatus,
    lifecycle::{
        consequence::LifecycleImpactClassification as Impact, policy::LifecycleStatus,
        LifecycleSnapshot,
    },
    path::ExitPathType,
};
use std::{process::Command, sync::OnceLock};
fn world() -> &'static FrozenCounterfactualWorld {
    static W: OnceLock<FrozenCounterfactualWorld> = OnceLock::new();
    W.get_or_init(|| {
        let root = repo_root();
        FrozenCounterfactualWorld::load(
            &root.join("snapshots/spacex-exposure.json"),
            &root.join("scenarios/spacex-transition.json"),
            &root.join("policies/stocklana-spacex-preflight-v1.json"),
        )
        .unwrap()
    })
}
fn report() -> &'static CounterfactualReport {
    static R: OnceLock<CounterfactualReport> = OnceLock::new();
    R.get_or_init(|| {
        let evaluated = world().evaluate(&world().scenarios().unwrap());
        assert!(
            evaluated.is_ok(),
            "same_world_counterfactual_invariants: {:?}",
            evaluated.as_ref().err()
        );
        evaluated.unwrap()
    })
}
fn snapshot() -> &'static LifecycleSnapshot {
    static S: OnceLock<LifecycleSnapshot> = OnceLock::new();
    S.get_or_init(|| {
        LifecycleSnapshot::load(&repo_root().join("snapshots/spacex-exposure.json")).unwrap()
    })
}
#[test]
fn same_production_digest_before_and_after() {
    let r = report();
    assert!(r
        .scenarios
        .iter()
        .all(|s| s.production_state_digest == r.production_world.production_state_digest));
    assert_eq!(
        r.production_world.exposure_snapshot_sha256,
        digest(snapshot()).unwrap()
    );
    assert_eq!(
        r.production_world.snapshot_sha256,
        snapshot()
            .exposures
            .as_ref()
            .unwrap()
            .source_snapshot_sha256
    );
}
#[test]
fn lifecycle_crosses_transition_boundary() {
    let r = report();
    assert_eq!(r.scenarios[0].lifecycle_status, LifecycleStatus::Active);
    assert_eq!(
        r.scenarios[1].lifecycle_status,
        LifecycleStatus::TransitionRequired
    );
    assert_eq!(
        r.scenarios[1].scenario.evaluation_time,
        r.lifecycle_scenario.policy.effective_at
    );
    assert_eq!(
        r.comparisons[0].crossed_boundaries,
        vec!["/policy/effective_at"]
    );
}
#[test]
fn balances_never_change() {
    let r = report();
    let expected = digest(
        &snapshot()
            .entities
            .iter()
            .map(|e| (&e.id, &e.state.raw_balance))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    for s in &r.scenarios {
        assert_eq!(s.balances_sha256, expected);
        assert_eq!(s.direct_holder.raw_balance, "17621");
        assert_eq!(
            s.protocol_position.raw_balance,
            r.frozen_position.principal_exposure_raw[0]
        );
    }
    assert!(r
        .scenarios
        .windows(2)
        .all(
            |s| s[0].direct_holder.raw_balance == s[1].direct_holder.raw_balance
                && s[0].protocol_position.raw_balance == s[1].protocol_position.raw_balance
        ));
}
#[test]
fn holder_requires_transition_after_event() {
    let r = report();
    assert_eq!(r.scenarios[0].direct_holder.impact, Impact::Unaffected);
    assert_eq!(
        r.scenarios[1].direct_holder.impact,
        Impact::RequiresTransition
    );
    assert!(
        r.scenarios[1].entity_impacts[&Impact::RequiresTransition].contains(
            &r.population_entities
                .binary_search(&r.scenarios[1].direct_holder.entity_id)
                .unwrap()
        )
    );
}
#[test]
fn vault_semantics_change_without_byte_changes() {
    let r = report();
    let vault = snapshot()
        .exposures
        .as_ref()
        .unwrap()
        .account_exposures
        .iter()
        .find(|e| {
            e.refinement
                .as_ref()
                .is_some_and(|v| v.classification == "LiquidityVault")
        })
        .unwrap();
    assert!(r.scenarios[0].entity_impacts[&Impact::Unaffected].contains(
        &r.population_entities
            .binary_search(&vault.phase2_entity_id)
            .unwrap()
    ));
    assert!(
        r.scenarios[1].entity_impacts[&Impact::StaleExposure].contains(
            &r.population_entities
                .binary_search(&vault.phase2_entity_id)
                .unwrap()
        )
    );
    assert_eq!(r.scenarios[0].population.stale_protocol_exposures, 0);
    assert!(r.scenarios[1].population.stale_protocol_exposures > 0);
    assert!(r.comparisons.iter().all(|d| d.unchanged_onchain_state
        && !d.onchain_state_changed
        && !d.categories.contains(&DiffCategory::OnChainStateChanged)));
    assert!(r.comparisons.iter().all(|d| d.lifecycle_meaning_changed
        && d.categories
            .contains(&DiffCategory::LifecycleMeaningChanged)));
    for s in &r.scenarios {
        for (a, b) in r.scenarios[0]
            .protocol_exposures
            .iter()
            .zip(&s.protocol_exposures)
        {
            assert_eq!(
                a.raw_balance, b.raw_balance,
                "frozen_vault_balance_unchanged"
            );
            assert_eq!(
                a.raw_state_sha256, b.raw_state_sha256,
                "frozen_vault_bytes_unchanged"
            );
            assert_eq!(a.captured_slot, b.captured_slot);
        }
        assert_eq!(
            s.direct_holder.raw_state_sha256,
            r.scenarios[0].direct_holder.raw_state_sha256
        );
        assert_eq!(
            s.protocol_position.raw_state_sha256,
            r.frozen_position.raw_position_sha256
        );
    }
}
#[test]
fn zero_balance_never_becomes_public_exposure() {
    let r = report();
    let zeros = snapshot()
        .entities
        .iter()
        .filter(|e| {
            e.state.raw_balance == "0"
                && !e
                    .state
                    .extensions
                    .iter()
                    .any(|x| x.extension_type == "ConfidentialTransferAccount")
        })
        .collect::<Vec<_>>();
    assert!(!zeros.is_empty());
    for s in &r.scenarios {
        for e in &zeros {
            assert!(
                s.entity_impacts.get(&Impact::Unaffected).is_some_and(
                    |ids| ids.contains(&r.population_entities.binary_search(&e.id).unwrap())
                ),
                "zero_balance_has_no_public_economic_exposure: {}",
                e.id
            );
        }
        assert_eq!(s.population.unaffected_zero_balance_entities, zeros.len());
    }
}
#[test]
fn deadline_uses_exact_policy_status() {
    let r = report();
    let d = r.lifecycle_scenario.policy.deadline.as_ref().unwrap();
    assert_eq!(r.scenarios[2].scenario.evaluation_time, d.at);
    assert_eq!(r.scenarios[2].lifecycle_status, d.after);
    assert_eq!(d.after, LifecycleStatus::NoIssuerEntitlement);
    assert_eq!(r.scenarios[2].direct_holder.impact, Impact::StaleExposure);
    assert_eq!(
        r.scenarios[2].population.stale_entities,
        r.scenarios[2].population.positive_balance_entities
    );
    assert_eq!(
        r.comparisons[1].crossed_boundaries,
        vec!["/policy/deadline/at"]
    );
}
#[test]
fn historical_execution_evidence_is_immutable() {
    let r = report();
    let direct: serde_json::Value =
        load(&repo_root().join("reports/spacex-lifecycle-path-resolution-phase9.json")).unwrap();
    let position: serde_json::Value =
        load(&repo_root().join("reports/spacex-dlmm-withdrawal.json")).unwrap();
    assert_eq!(
        r.historical_direct_paths, direct["paths"],
        "historical_direct_matrix_unchanged"
    );
    assert_eq!(
        r.historical_position_paths, position["paths"],
        "historical_position_matrix_unchanged"
    );
    assert_eq!(
        serde_json::to_value(&r.frozen_position).unwrap(),
        position["position"],
        "frozen_position_is_pre_execution"
    );
    let proof = digest(&(
        r.historical_direct_paths.clone(),
        r.historical_position_paths.clone(),
        &r.frozen_readiness.path_evidence,
        &r.frozen_readiness.position_exit_evidence,
    ))
    .unwrap();
    assert!(r
        .scenarios
        .iter()
        .all(|s| s.historical_execution_evidence_sha256 == proof));
    world().validate(r).unwrap();
}
#[test]
fn transfer_stays_proven() {
    for s in &report().scenarios {
        assert_eq!(
            s.path_implications
                .iter()
                .find(|p| p.path_type == ExitPathType::Transfer
                    && p.entity_id == s.direct_holder.entity_id)
                .unwrap()
                .historical_status,
            PathStatus::Proven
        );
    }
}
#[test]
fn withdrawal_stays_proven() {
    for s in &report().scenarios {
        assert_eq!(
            s.path_implications
                .iter()
                .find(|p| p.path_type == ExitPathType::Withdrawal
                    && p.entity_id == s.protocol_position.entity_id)
                .unwrap()
                .historical_status,
            PathStatus::Proven
        );
        assert_eq!(s.protocol_position.raw_balance, "21296808");
    }
    assert_eq!(
        report().scenarios[0].protocol_position.impact,
        Impact::Unaffected
    );
    assert_eq!(
        report().scenarios[1].protocol_position.impact,
        Impact::StaleExposure
    );
    assert!(
        report().scenarios[1]
            .protocol_position
            .cause
            .contains("Decoded protocol position"),
        "position_explanation_uses_decoded_ownership_context"
    );
}
#[test]
fn official_transition_never_inherits_proof() {
    for s in &report().scenarios {
        assert!(s
            .path_implications
            .iter()
            .filter(|p| p.path_type == ExitPathType::OfficialTransition)
            .all(|p| p.historical_status == PathStatus::NotTested));
    }
}
#[test]
fn readiness_changes_only_with_policy_and_applicability() {
    let r = report();
    let original: LifecycleReadinessReport =
        load(&repo_root().join("reports/spacex-lifecycle-readiness.json")).unwrap();
    assert_eq!(r.frozen_readiness, original);
    assert_eq!(
        r.scenarios[0].readiness.applicability,
        GateApplicability::PreEvent
    );
    assert_eq!(r.scenarios[0].readiness.policy_result, None);
    assert!(r.scenarios[0].readiness.active_failure_mode_ids.is_empty());
    for s in &r.scenarios[1..] {
        assert_eq!(s.readiness.applicability, GateApplicability::Applicable);
        assert_eq!(s.readiness.policy_result, Some(original.overall_status));
        assert_eq!(s.readiness.policy_result, Some(ReadinessStatus::Incomplete));
        assert_eq!(
            s.readiness.active_failure_mode_ids,
            original
                .prevented_rollout_conditions
                .iter()
                .map(|m| m.requirement_id.clone())
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(r.scenarios[1].readiness, r.scenarios[2].readiness);
    assert!(!r.comparisons[1]
        .categories
        .contains(&DiffCategory::ReadinessChanged));
    assert!(r.comparisons[0]
        .changed_readiness_findings
        .contains(&"direct-official-transition".to_string()));
    let mut tampered = r.clone();
    tampered.scenarios[1].readiness.policy_result = Some(ReadinessStatus::Ready);
    assert!(
        world().validate(&tampered).is_err(),
        "unjustified_readiness_rejected"
    );
}
#[test]
fn different_worlds_are_non_comparable() {
    let r = report();
    let mut different = r.scenarios[1].clone();
    different.production_state_digest = "different-world".into();
    assert!(
        compare(&r.scenarios[0], &different, &r.lifecycle_scenario).is_err(),
        "different_production_digests_rejected"
    );
    let mut balances = r.scenarios[1].clone();
    balances.direct_holder.raw_balance = "17622".into();
    assert!(
        compare(&r.scenarios[0], &balances, &r.lifecycle_scenario).is_err(),
        "mutated_token_balance_rejected"
    );
    let mut proof = r.scenarios[1].clone();
    proof.historical_execution_evidence_sha256 = "changed-evidence".into();
    assert!(
        compare(&r.scenarios[0], &proof, &r.lifecycle_scenario).is_err(),
        "changed_execution_evidence_rejected"
    );
}
#[test]
fn scenario_order_is_deterministic() {
    let mut views = world().scenarios().unwrap();
    views.reverse();
    assert_eq!(
        world().evaluate(&views).unwrap().to_json().unwrap(),
        report().to_json().unwrap()
    );
}
#[test]
fn canonical_json_roundtrip_is_byte_deterministic() {
    let bytes = report().to_json().unwrap();
    let round: CounterfactualReport = serde_json::from_str(&bytes).unwrap();
    assert_eq!(round.to_json().unwrap(), bytes);
}
#[test]
fn offline_cli_reproduces_canonical_report() {
    let root = repo_root();
    let temp = std::env::temp_dir().join(format!("eplyx-phase13-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let out = temp.join("report.json");
    let _ = std::fs::remove_file(&out);
    let args = ["compare-scenarios", "--snapshot"];
    let mut c = Command::new(env!("CARGO_BIN_EXE_eplyx"));
    c.current_dir(std::env::temp_dir())
        .env_remove("SOLANA_RPC_URL")
        .arg("lifecycle")
        .args(args)
        .arg(root.join("snapshots/spacex-exposure.json"))
        .arg("--scenario")
        .arg(root.join("scenarios/spacex-transition.json"))
        .arg("--readiness-policy")
        .arg(root.join("policies/stocklana-spacex-preflight-v1.json"))
        .args(["--format", "json", "--out"])
        .arg(&out);
    let run = c.output().unwrap();
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, report().to_json().unwrap().as_bytes());
    assert_eq!(run.stdout, std::fs::read(&out).unwrap());
    assert_eq!(
        run.stdout,
        std::fs::read(root.join("reports/spacex-counterfactual-lifecycle.json")).unwrap()
    );
    let protected = c.output().unwrap();
    assert_eq!(protected.status.code(), Some(2));
    assert_eq!(run.stdout, std::fs::read(&out).unwrap());
    std::fs::remove_dir_all(temp).unwrap();
}
#[test]
fn population_counts_are_derived_and_proof_scope_is_preserved() {
    let r = report();
    for s in &r.scenarios {
        assert_eq!(
            s.population.positive_balance_entities,
            snapshot()
                .entities
                .iter()
                .filter(|e| e.state.raw_balance != "0")
                .count()
        );
        assert_eq!(
            s.entity_impacts.values().map(Vec::len).sum::<usize>(),
            snapshot().entities.len()
        );
        assert_eq!(
            s.summary.classifications[&Impact::RequiresTransition],
            s.entity_impacts
                .get(&Impact::RequiresTransition)
                .map_or(0, Vec::len)
        );
    }
    assert_eq!(r.scenarios[0].population.requiring_transition, 0);
    assert_eq!(
        r.scenarios[1].population.requiring_transition + r.scenarios[1].population.stale_entities,
        r.scenarios[1].population.positive_balance_entities
    );
    assert_eq!(
        r.comparisons[0]
            .changed_entities
            .iter()
            .map(|g| g.entity_indices.len())
            .sum::<usize>(),
        r.scenarios[1].population.positive_balance_entities
    );
    assert!(r.scenarios[0]
        .path_implications
        .iter()
        .all(|p| !p.lifecycle_relevant));
    for scenario in &r.scenarios {
        for path in &scenario.path_implications {
            if path.historical_status == PathStatus::NotApplicable {
                assert!(
                    !path.lifecycle_relevant && path.cause.contains("NotApplicable"),
                    "inapplicable_path_retains_contextual_attribution"
                );
            }
        }
    }
    assert!(r.scenarios[1]
        .path_implications
        .iter()
        .filter(|p| p.historical_status == PathStatus::Proven)
        .all(|p| p.lifecycle_relevant));
    assert!(r
        .frozen_readiness
        .path_evidence
        .iter()
        .filter(|p| p.status == PathStatus::Proven)
        .all(|p| !p.signer.signer_possession_known));
}
#[test]
fn altered_inputs_and_semantics_fail_regeneration() {
    let r = report();
    let mut tampered = r.clone();
    tampered.scenarios[1].direct_holder.impact = Impact::Unaffected;
    assert!(
        world().validate(&tampered).is_err(),
        "altered_holder_impact_rejected"
    );
    let mut tampered = r.clone();
    tampered.comparisons[0].onchain_state_changed = true;
    assert!(
        world().validate(&tampered).is_err(),
        "semantic_change_is_not_onchain_mutation"
    );
    let mut views = world().scenarios().unwrap();
    views[0].lifecycle_policy_sha256 = "other-policy".into();
    assert!(world().evaluate(&views).is_err());
    let duplicate = vec![views[1].clone(), views[1].clone()];
    assert!(world().evaluate(&duplicate).is_err());
}

#[test]
fn mutated_snapshot_input_is_rejected_by_pinned_identity() {
    let root = repo_root();
    let temp = std::env::temp_dir().join(format!("eplyx-phase13-input-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("mutated-snapshot.json");
    let mut raw: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("snapshots/spacex-exposure.json")).unwrap(),
    )
    .unwrap();
    let entity = raw["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["token_account"] == "741ZXYKzgjPZucRhPUQFK7kKAgP1ESJwimhumgGpuPHs")
        .unwrap();
    entity["state"]["raw_balance"] = serde_json::json!("17622");
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    let result = FrozenCounterfactualWorld::load(
        &path,
        &root.join("scenarios/spacex-transition.json"),
        &root.join("policies/stocklana-spacex-preflight-v1.json"),
    );
    assert!(result.is_err(), "mutated_production_snapshot_rejected");
    let error = result.err().unwrap().to_string();
    assert!(
        error.contains("named readiness input digest mismatch"),
        "pinned_snapshot_identity_rejects_changed_balance: {error}"
    );
    std::fs::remove_dir_all(temp).unwrap();
}
