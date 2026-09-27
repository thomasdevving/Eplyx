//! Deterministic reconstructed current-run fixtures, not live acquisition acceptance.
use eplyx_engine::{
    canonical::digest,
    lifecycle::preflight::{self, *},
    lifecycle::{current as wallet, exposure::sha256},
    path::current as execution,
};
use serde_json::{json, Value};
#[path = "common/current_observation.rs"]
mod harness;
fn root() -> std::path::PathBuf {
    eplyx_engine::repo_root().join("fixtures/current/sta")
}
fn inputs(checks: bool) -> Inputs {
    let c: execution::Capture = serde_json::from_slice(
        &std::fs::read(root().join("reports/milestone4-validation/live-transfer.capture.json"))
            .unwrap(),
    )
    .unwrap();
    let wallet_hash = sha256(c.wallet_capture.as_bytes());
    let mut input = Inputs {
        run_id: "current-test-run".into(),
        preflight_id: "proposal-a".into(),
        created_at: "2030-01-01T00:00:00Z".parse().unwrap(),
        engine_sha256: "test-engine".into(),
        wallet_capture: c.wallet_capture.clone(),
        wallet_sha256: wallet_hash,
        request: Request {
            source: c.request.source,
            successor_mint: None,
            effective_at: "2031-01-01T00:00:00Z".parse().unwrap(),
            deadline: Some("2032-01-01T00:00:00Z".parse().unwrap()),
            post_deadline: Some(PostDeadline::TransitionStillRequired),
            assurance: Assurance::FullTransition,
            check_ids: vec![],
            conversion_check_id: None,
        },
        successor_capture: None,
        checks: vec![],
        conversion: None,
    };
    if checks {
        for label in ["live-transfer", "live-market"] {
            let mut c: execution::Capture = serde_json::from_slice(
                &std::fs::read(root().join(format!(
                    "reports/milestone4-validation/{label}.capture.json"
                )))
                .unwrap(),
            )
            .unwrap();
            if label == "live-market" {
                let w: wallet::Capture = serde_json::from_str(&input.wallet_capture).unwrap();
                c.observations[1].params[1]["minContextSlot"] =
                    w.observations[3].result.as_ref().unwrap()["context"]["slot"].clone();
            }
            c.run_id = input.run_id.clone();
            c.wallet_capture = input.wallet_capture.clone();
            c.wallet_capture_sha256 = input.wallet_sha256.clone();
            let capture = serde_json::to_string(&c).unwrap();
            let hash = sha256(capture.as_bytes());
            let result = execution::replay(
                capture.as_bytes(),
                &input.run_id,
                &c.check_id,
                &input.wallet_sha256,
                &hash,
            )
            .unwrap();
            input.request.check_ids.push(c.check_id.clone());
            input.checks.push(CheckEvidence {
                id: c.check_id,
                capture_sha256: hash,
                result_sha256: digest(result.value()).unwrap(),
                engine_sha256: "fixture-test".into(),
                capture,
            });
        }
    }
    input
}
fn evaluate(input: Inputs) -> anyhow::Result<Value> {
    let bundle = preflight::prepare(input)?;
    evaluate_bundle(&bundle)
}
fn evaluate_bundle(b: &Bundle) -> anyhow::Result<Value> {
    let bytes = serde_json::to_vec(b)?;
    preflight::replay(
        &bytes,
        &b.inputs.run_id,
        &b.inputs.preflight_id,
        &b.inputs.wallet_sha256,
        &b.scenario_sha256,
        &sha256(&bytes),
        Some(
            &std::fs::read(eplyx_engine::repo_root().join("artifacts/eplyx_token_migration.so"))
                .unwrap(),
        ),
    )
}
fn path(v: &Value, name: &str) -> Value {
    v["paths"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["path_type"] == name)
        .unwrap()["status"]
        .clone()
}
#[test]
fn proposed_provenance_never_becomes_issuer_assertion() {
    let v = evaluate(inputs(false)).unwrap();
    assert_eq!(
        v["proposed_change"]["source"], "UserProposed",
        "user proposal cannot become issuer assertion"
    );
    assert_eq!(v["proposed_change"]["issuer_relationship_verified"], false);
    assert!(v["proposed_change"]["conversion_ratio"].is_null());
    assert_eq!(v["authorization"], false);
}
#[test]
fn unchanged_current_bytes_at_all_proposed_times() {
    let i = inputs(false);
    let original = i.wallet_capture.clone();
    let b = preflight::prepare(i).unwrap();
    let v = evaluate_bundle(&b).unwrap();
    assert_eq!(b.inputs.wallet_capture, original);
    let views = v["views"].as_array().unwrap();
    assert_eq!(views[0]["lifecycle_status"], "Active");
    assert_eq!(views[0]["impact"], "Unaffected");
    assert_eq!(views[0]["gate_applicability"], "PreEvent");
    assert!(views[0]["readiness"].is_null());
    assert_eq!(views[1]["lifecycle_status"], "TransitionRequired");
    assert_eq!(views[1]["impact"], "RequiresTransition");
    assert_eq!(
        views[2]["lifecycle_status"],
        "PostDeadlineTransitionRequired"
    );
    for view in views {
        assert_eq!(
            view["balance_raw"], v["entity"]["balance_raw"],
            "proposed time cannot change current balance"
        );
        assert_eq!(view["account_sha256"], v["entity"]["account_sha256"]);
    }
    assert!(!v.to_string().contains("NoIssuerEntitlement"));
    assert!(v["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("not predicted")));
}
#[test]
fn saved_phase7_proof_cannot_enter_missing_current_checks() {
    let v = evaluate(inputs(false)).unwrap();
    assert_eq!(
        path(&v, "Transfer"),
        "NotTested",
        "historical proof cannot satisfy current path"
    );
    assert_eq!(path(&v, "SecondaryMarketExit"), "NotTested");
    assert_eq!(path(&v, "OfficialTransition"), "NotTested");
    assert_eq!(path(&v, "Withdrawal"), "NotApplicable");
    assert_eq!(path(&v, "Redemption"), "Unsupported");
    assert_eq!(
        v["views"][1]["readiness"]["mobility"]["status"],
        "Incomplete"
    );
}
#[test]
fn current_mobility_never_proves_official_conversion() {
    let v = evaluate(inputs(true)).unwrap();
    assert_eq!(path(&v, "Transfer"), "Proven");
    assert_eq!(path(&v, "SecondaryMarketExit"), "Proven");
    assert_eq!(
        path(&v, "OfficialTransition"),
        "NotTested",
        "mobility must not prove official conversion"
    );
    assert_eq!(v["views"][1]["readiness"]["mobility"]["status"], "Ready");
    assert_eq!(
        v["views"][1]["readiness"]["full_transition"]["status"],
        "Incomplete"
    );
    for p in v["views"][0]["paths"].as_array().unwrap() {
        assert_eq!(p["lifecycle_relevant"], false);
    }
    for p in v["views"][1]["paths"].as_array().unwrap() {
        assert_eq!(p["status"], path(&v, p["path"].as_str().unwrap()));
    }
}
#[test]
fn refreshed_run_and_other_entity_reject_existing_proof() {
    let i = inputs(true);
    let mut changed = i.clone();
    changed.run_id = "refreshed-run".into();
    assert!(
        evaluate(changed).is_err(),
        "refresh cannot inherit earlier run proof"
    );
    let mut changed = i.clone();
    changed.request.source = "11111111111111111111111111111111".into();
    assert!(
        evaluate(changed).is_err(),
        "other account cannot inherit proof"
    );
    let mut changed = i.clone();
    let mut wallet: wallet::Capture = serde_json::from_str(&changed.wallet_capture).unwrap();
    wallet.selection.as_mut().unwrap().public_owner =
        Some("11111111111111111111111111111111".into());
    changed.wallet_capture = serde_json::to_string(&wallet).unwrap();
    changed.wallet_sha256 = sha256(changed.wallet_capture.as_bytes());
    assert!(
        evaluate(changed).is_err(),
        "other owner cannot inherit proof"
    );
    let mut changed = i;
    changed.wallet_sha256 = "0".repeat(64);
    assert!(evaluate(changed).is_err());
}
#[test]
fn scenario_digest_cannot_reuse_readiness() {
    let b = preflight::prepare(inputs(false)).unwrap();
    let bytes = serde_json::to_vec(&b).unwrap();
    assert!(
        preflight::replay(
            &bytes,
            &b.inputs.run_id,
            &b.inputs.preflight_id,
            &b.inputs.wallet_sha256,
            &"0".repeat(64),
            &sha256(&bytes),
            None
        )
        .is_err(),
        "different scenario digest must reject readiness"
    );
    let mut b = b;
    b.scenario["source"] = "IssuerAsserted".into();
    b.scenario_sha256 = digest(&b.scenario).unwrap();
    assert!(evaluate_bundle(&b).is_err());
}
#[test]
fn entity_ready_never_becomes_population_ready() {
    let v = evaluate(inputs(true)).unwrap();
    assert_eq!(
        v["views"][1]["readiness"]["mobility"]["scope"],
        "SelectedEntityPreflightReadiness"
    );
    assert!(
        v["population_readiness"].is_null(),
        "selected entity must not grant population readiness"
    );
    assert!(v["views"][1]["readiness"]["mobility"]["population_readiness"].is_null());
    assert!(!v.to_string().contains("positive_balance_entities"));
}
#[test]
fn successor_is_independently_inspected_and_never_issuer_verified() {
    let mut i = inputs(false);
    i.request.successor_mint = Some("So11111111111111111111111111111111111111112".into());
    assert!(
        evaluate(i.clone()).is_err(),
        "successor needs separate inspection"
    );
    i.successor_capture = Some(
        std::fs::read_to_string(root().join("reports/milestone2-validation/custom.capture.json"))
            .unwrap(),
    );
    let v = evaluate(i.clone()).unwrap();
    assert_eq!(v["successor_verification"]["status"], "MintObserved");
    assert_eq!(
        v["successor_verification"]["issuer_relationship_verified"],
        false
    );
    assert!(v["successor_verification"]["conversion_ratio"].is_null());
    i.request.successor_mint = Some("11111111111111111111111111111111".into());
    i.successor_capture = Some(
        std::fs::read_to_string(root().join("reports/milestone2-validation/nonmint.capture.json"))
            .unwrap(),
    );
    let v = evaluate(i.clone()).unwrap();
    assert_eq!(v["preparation_status"], "InvalidScenario");
    assert_eq!(v["views"], json!([]));
    let mut c: wallet::Capture =
        serde_json::from_str(i.successor_capture.as_ref().unwrap()).unwrap();
    c.observations[1].result = None;
    c.observations[1].error = Some("bounded acquisition unavailable".into());
    i.successor_capture = Some(serde_json::to_string(&c).unwrap());
    assert_eq!(
        evaluate(i).unwrap()["preparation_status"],
        "PreparationIncomplete"
    );
}
#[test]
fn request_boundaries_and_offline_canonical_replay() {
    let i = inputs(false);
    let b = preflight::prepare(i.clone()).unwrap();
    assert_eq!(evaluate_bundle(&b).unwrap(), evaluate_bundle(&b).unwrap());
    let mut bad = i.clone();
    bad.request.effective_at = bad.created_at;
    assert!(preflight::prepare(bad).is_err());
    let mut bad = i.clone();
    bad.request.deadline = Some(bad.request.effective_at);
    assert!(preflight::prepare(bad).is_err());
    let mut bad = i;
    bad.request.post_deadline = None;
    assert!(preflight::prepare(bad).is_err());
}

#[test]
fn unavailable_current_check_is_indeterminate_not_failure_or_readiness() {
    let mut i = inputs(true);
    i.checks.truncate(1);
    i.request.check_ids = vec![i.checks[0].id.clone()];
    let c = &mut i.checks[0];
    let mut capture: execution::Capture = serde_json::from_str(&c.capture).unwrap();
    capture.observations.clear();
    c.capture = serde_json::to_string(&capture).unwrap();
    c.capture_sha256 = sha256(c.capture.as_bytes());
    let proof = execution::replay(
        c.capture.as_bytes(),
        &i.run_id,
        &c.id,
        &i.wallet_sha256,
        &c.capture_sha256,
    )
    .unwrap();
    c.result_sha256 = digest(proof.value()).unwrap();
    let v = evaluate(i).unwrap();
    assert_eq!(path(&v, "Transfer"), "Indeterminate");
    assert_eq!(
        v["views"][1]["readiness"]["mobility"]["status"],
        "Incomplete"
    );
}

#[test]
fn another_decoded_account_in_same_wallet_does_not_inherit_proof() {
    let mut i = inputs(true);
    i.checks.truncate(1);
    i.request.check_ids = vec![i.checks[0].id.clone()];
    let second = "123aUGPWa93jiga876U3rLdBP86JNFSoz9tSQWCAskMc";
    let mut w: wallet::Capture = serde_json::from_str(&i.wallet_capture).unwrap();
    let rows = w.observations[3].result.as_mut().unwrap()["value"]
        .as_array_mut()
        .unwrap();
    let mut row = rows[0].clone();
    row["pubkey"] = second.into();
    rows.push(row);
    i.wallet_capture = serde_json::to_string(&w).unwrap();
    i.wallet_sha256 = sha256(i.wallet_capture.as_bytes());
    let c = &mut i.checks[0];
    let mut capture: execution::Capture = serde_json::from_str(&c.capture).unwrap();
    capture.wallet_capture = i.wallet_capture.clone();
    capture.wallet_capture_sha256 = i.wallet_sha256.clone();
    c.capture = serde_json::to_string(&capture).unwrap();
    c.capture_sha256 = sha256(c.capture.as_bytes());
    let proof = execution::replay(
        c.capture.as_bytes(),
        &i.run_id,
        &c.id,
        &i.wallet_sha256,
        &c.capture_sha256,
    )
    .unwrap();
    c.result_sha256 = digest(proof.value()).unwrap();
    i.request.source = second.into();
    assert!(
        evaluate(i)
            .unwrap_err()
            .to_string()
            .contains("entity binding mismatch"),
        "a second decoded account cannot inherit its peer's proof"
    );
}

#[test]
fn exact_failed_market_is_not_globally_blocked_by_entity_preset() {
    let mut i = inputs(true);
    i.checks.remove(0);
    i.request.check_ids = vec![i.checks[0].id.clone()];
    let c = &mut i.checks[0];
    let mut capture: execution::Capture = serde_json::from_str(&c.capture).unwrap();
    capture.request.minimum_output_decimal = Some("1000000".into());
    c.capture = serde_json::to_string(&capture).unwrap();
    c.capture_sha256 = sha256(c.capture.as_bytes());
    let proof = execution::replay(
        c.capture.as_bytes(),
        &i.run_id,
        &c.id,
        &i.wallet_sha256,
        &c.capture_sha256,
    )
    .unwrap();
    assert_eq!(proof.value()["status"], "Failed");
    c.result_sha256 = digest(proof.value()).unwrap();
    let v = evaluate(i).unwrap();
    assert_eq!(path(&v, "SecondaryMarketExit"), "Failed");
    assert_eq!(
        v["views"][1]["readiness"]["mobility"]["status"],
        "Incomplete"
    );
    assert_eq!(path(&v, "OfficialTransition"), "NotTested");
}
mod candidate_plan {
    use super::*;
    use crate::harness;
    use eplyx_engine::migration::account as conversion;

    fn conversion_evidence(run: &str, input: &Inputs) -> (ConversionEvidence, String) {
        let rpc = harness::Rpc::new().with_wallet(&input.wallet_capture);
        let replacement = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string();
        let request = conversion::Request {
            source: input.request.source.clone(),
            replacement_mint: replacement.clone(),
            amount_mode: eplyx_engine::path::current::AmountMode::Custom,
            amount_decimal: Some("0.0000001".into()),
            numerator: "1".into(),
            denominator: "1".into(),
            rounding: eplyx_engine::migration::spec::Rounding::Floor,
            fee_bps: 0,
            reserve_raw: "1000000000".into(),
        };
        let program =
            std::fs::read(eplyx_engine::repo_root().join("artifacts/eplyx_token_migration.so"))
                .unwrap();
        let capture = conversion::capture(
            input.wallet_capture.clone(),
            request,
            run.into(),
            "candidate-check".into(),
            &program,
            &rpc,
        )
        .unwrap();
        let bytes = serde_json::to_vec(&capture).unwrap();
        let verified = conversion::replay(
            &bytes,
            run,
            "candidate-check",
            &input.wallet_sha256,
            &program,
        )
        .unwrap();
        let v = verified.value();
        assert_eq!(v["status"], "Proven", "{v}");
        (
            ConversionEvidence {
                id: "candidate-check".into(),
                capture_sha256: sha256(&bytes),
                result_sha256: digest(v).unwrap(),
                engine_sha256: "fixture-test".into(),
                plan_sha256: v["plan_sha256"].as_str().unwrap().into(),
                program_sha256: sha256(&program),
                capture: String::from_utf8(bytes).unwrap(),
            },
            replacement,
        )
    }
    fn successor_capture() -> String {
        let rpc = harness::Rpc::new();
        serde_json::to_string(
            &wallet::capture_selected(
                wallet::InspectionSelection {
                    cluster: "solana-mainnet".into(),
                    mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
                    reference: None,
                    sample_accounts: false,
                    public_owner: None,
                },
                &rpc,
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn with_conversion() -> Inputs {
        let mut input = inputs(false);
        let (evidence, replacement) = conversion_evidence(&input.run_id, &input);
        input.request.conversion_check_id = Some(evidence.id.clone());
        input.request.successor_mint = Some(replacement);
        input.successor_capture = Some(successor_capture());
        input.conversion = Some(evidence);
        input
    }
    fn readiness(v: &Value, gate: &str) -> Value {
        v["views"]
            .as_array()
            .unwrap()
            .iter()
            .find(|view| view["id"] == "ProposedActive")
            .unwrap()["readiness"][gate]["status"]
            .clone()
    }
    #[test]
    fn no_supplied_plan_leaves_conversion_untested_and_full_transition_incomplete() {
        let v = evaluate(inputs(true)).unwrap();
        assert_eq!(
            v["replacement_conversion"]["status"], "NotTested",
            "an unsupplied conversion plan cannot be tested"
        );
        assert!(v["replacement_conversion"]["message"]
            .as_str()
            .unwrap()
            .contains("No conversion mechanism was supplied"));
        assert_eq!(path(&v, "OfficialTransition"), "NotTested");
        assert_eq!(readiness(&v, "candidate_plan"), "Incomplete");
        assert_eq!(readiness(&v, "full_transition"), "Incomplete");
        assert_eq!(
            readiness(&v, "mobility"),
            "Ready",
            "mobility can still be ready without any conversion plan"
        );
    }
    #[test]
    fn a_proven_candidate_plan_makes_candidate_readiness_ready_and_nothing_else() {
        let v = evaluate(with_conversion()).unwrap();
        assert_eq!(v["replacement_conversion"]["status"], "Proven");
        assert_eq!(
            v["replacement_conversion"]["provenance"],
            "OperatorSupplied"
        );
        assert_eq!(
            readiness(&v, "candidate_plan"),
            "Ready",
            "candidate_plan_readiness_can_become_ready"
        );
        assert_eq!(
            path(&v, "OfficialTransition"),
            "NotTested",
            "candidate_proof_must_not_become_official_transition"
        );
        assert_eq!(
            v["replacement_conversion"]["official_transition"], "NotTested",
            "operator_supplied_proof_must_not_report_an_official_transition"
        );
        assert_eq!(
            readiness(&v, "full_transition"),
            "Incomplete",
            "candidate_ready_must_not_become_full_transition_ready"
        );
        assert_eq!(
            v["population_readiness"],
            Value::Null,
            "candidate_ready_must_not_become_population_ready"
        );
        assert_eq!(v["authorization"], false);
        assert_eq!(v["funds_moved"], false);
    }
    #[test]
    fn candidate_evidence_is_exact_to_its_run_plan_program_and_result() {
        let base = with_conversion();
        type InputEdit = (&'static str, Box<dyn Fn(&mut Inputs)>);
        let mutate: Vec<InputEdit> = vec![
            (
                "wrong result digest",
                Box::new(|i: &mut Inputs| {
                    i.conversion.as_mut().unwrap().result_sha256 = "0".repeat(64)
                }),
            ),
            (
                "wrong plan digest",
                Box::new(|i: &mut Inputs| {
                    i.conversion.as_mut().unwrap().plan_sha256 = "0".repeat(64)
                }),
            ),
            (
                "wrong candidate binary digest",
                Box::new(|i: &mut Inputs| {
                    i.conversion.as_mut().unwrap().program_sha256 = "0".repeat(64)
                }),
            ),
            (
                "wrong capture digest",
                Box::new(|i: &mut Inputs| {
                    i.conversion.as_mut().unwrap().capture_sha256 = "0".repeat(64)
                }),
            ),
            (
                "another run",
                Box::new(|i: &mut Inputs| i.run_id = "another-run".into()),
            ),
            (
                "a different proposed replacement",
                Box::new(|i: &mut Inputs| {
                    i.request.successor_mint =
                        Some("PreweJYECqtQwBtpxHL171nL2K6umo692gTm7Q3rpgF".into())
                }),
            ),
            (
                "unbound evidence",
                Box::new(|i: &mut Inputs| i.request.conversion_check_id = None),
            ),
        ];
        for (name, edit) in mutate {
            let mut input = base.clone();
            edit(&mut input);
            assert!(
                evaluate(input).is_err(),
                "exact_candidate_binding_must_be_required: {name}"
            );
        }
    }
    #[test]
    fn a_refreshed_run_cannot_inherit_candidate_conversion_proof() {
        let mut input = with_conversion();
        input.run_id = "refreshed-run".into();
        input.preflight_id = "proposal-b".into();
        assert!(
            evaluate(input).is_err(),
            "refresh_must_not_inherit_candidate_conversion_proof"
        );
        // The refreshed run without the candidate check is simply untested again.
        let mut fresh = inputs(false);
        fresh.run_id = "refreshed-run".into();
        let v = evaluate(fresh).unwrap();
        assert_eq!(v["replacement_conversion"]["status"], "NotTested");
    }
    #[test]
    fn mobility_evidence_can_never_stand_in_for_a_candidate_conversion() {
        let mut input = inputs(true);
        let (evidence, replacement) = conversion_evidence(&input.run_id, &input);
        input.request.successor_mint = Some(replacement);
        input.successor_capture = Some(successor_capture());
        // Transfer and market-exit proof is present; no conversion is selected.
        let v = evaluate(input.clone()).unwrap();
        assert_eq!(path(&v, "Transfer"), "Proven");
        assert_eq!(v["replacement_conversion"]["status"], "NotTested");
        assert_eq!(
            readiness(&v, "candidate_plan"),
            "Incomplete",
            "mobility_proof_must_not_satisfy_candidate_conversion"
        );
        // Selecting it as an ordinary execution check is rejected outright.
        input.request.check_ids.push(evidence.id.clone());
        input.checks.push(CheckEvidence {
            id: evidence.id,
            capture_sha256: evidence.capture_sha256,
            result_sha256: evidence.result_sha256,
            engine_sha256: evidence.engine_sha256,
            capture: evidence.capture,
        });
        assert!(
            evaluate(input).is_err(),
            "a candidate conversion capture is not an execution check"
        );
    }
}
