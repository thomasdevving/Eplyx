//! Thin rollout layer over original pinned evidence; no VM/RPC execution here.
#[path = "common/lifecycle.rs"]
mod fixture;
use eplyx_engine::{
    lifecycle::artifact::reference_root as repo_root,
    lifecycle::exposure::sha256,
    lifecycle::frozen::selection::{canonical, load},
    lifecycle::readiness::{EvaluatedScope, FindingEffect, ReadinessStatus},
    lifecycle::resolution::PathStatus,
    lifecycle::rollout::{
        self, CandidateAcceptance, CandidateRolloutPlan, EvidenceAssessment, GateCommandCompletion,
        GuardDisposition, ReasonCode, RolloutDemoCases, RolloutValidator,
        VerifiedCandidateAssessment,
    },
    path::ExitPathType,
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};
fn validator() -> &'static RolloutValidator {
    static V: OnceLock<RolloutValidator> = OnceLock::new();
    V.get_or_init(|| {
        RolloutValidator::load(&repo_root().join("probes/phase14-evidence-binding.json")).unwrap()
    })
}
fn plan(name: &str) -> CandidateRolloutPlan {
    load(&repo_root().join(format!("probes/phase14-plans/{name}.json"))).unwrap()
}
fn evaluate(p: &CandidateRolloutPlan) -> VerifiedCandidateAssessment {
    let result = validator().evaluate(p, &repo_root().join("probes/phase14-plans"));
    assert!(
        result.is_ok(),
        "phase14_verified_candidate_evaluation_succeeds: {:?}",
        result.as_ref().err()
    );
    result.unwrap()
}
fn positive() -> VerifiedCandidateAssessment {
    evaluate(&plan("principal-removal-positive-control"))
}
fn original_policy(p: &mut CandidateRolloutPlan) {
    let original = plan("lp-complete-exit");
    p.assurance_policy = original.assurance_policy;
    p.required_assurance_conditions = original.required_assurance_conditions;
}
fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("eplyx-phase14-{}-{name}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    p
}
#[test]
fn principal_removal_never_establishes_complete_exit() {
    let e = evaluate(&plan("lp-complete-exit"));
    let r = e.report();
    assert_eq!(r.candidate_acceptance, CandidateAcceptance::NotAccepted);
    for id in [
        "principal-operation",
        "principal-token-deltas",
        "zero-liquidity-shares",
    ] {
        assert_eq!(
            r.assessments
                .iter()
                .find(|a| a.assertion_id == id)
                .unwrap()
                .assessment,
            EvidenceAssessment::Supported
        );
    }
    let complete = r
        .assessments
        .iter()
        .find(|a| a.assertion_id == "complete-exit")
        .unwrap();
    assert_eq!(
        complete.assessment,
        EvidenceAssessment::Contradicted,
        "principal_withdrawal_is_not_complete_exit"
    );
    assert!(complete
        .reason_codes
        .contains(&ReasonCode::ProtocolAccruedFeesRemain));
    assert!(complete
        .reason_codes
        .contains(&ReasonCode::FeeCollectionNotTested));
    assert!(complete
        .reason_codes
        .contains(&ReasonCode::PositionClosureNotTested));
    assert!(complete
        .evidence_pointers
        .iter()
        .any(|p| p.source_pointer == "/post_position_fields/pending_fees_raw"));
    assert_eq!(r.readiness.overall_status, ReadinessStatus::Incomplete);
    assert!(r
        .readiness
        .path_evidence
        .iter()
        .any(|f| f.path_type == ExitPathType::Withdrawal && f.status == PathStatus::Proven));
}
#[test]
fn accrued_protocol_fees_and_destination_withheld_fees_are_distinct() {
    let e = evaluate(&plan("lp-complete-exit"));
    let r = e.report();
    let p = &r.position_observations[0];
    let mint = &r.readiness.asset_mint;
    assert_eq!(p.retained_protocol_accrued_fees_raw[mint], "126543");
    assert_eq!(p.destination_withheld_transfer_fees_raw[mint], "106485");
    let usdc = p
        .retained_protocol_accrued_fees_raw
        .keys()
        .find(|k| *k != mint)
        .unwrap();
    assert_eq!(p.retained_protocol_accrued_fees_raw[usdc], "113735");
    assert_eq!(p.destination_withheld_transfer_fees_raw[usdc], "0");
    assert_eq!(
        p.provenance,
        rollout::PositionObservationProvenance::OriginalLocalWithdrawalPostExecution
    );
    assert!(p.all_liquidity_shares_zero && p.position_account_retained);
    assert_eq!(p.fee_collection, PathStatus::NotTested);
    assert_eq!(p.position_closure, PathStatus::NotTested);
    assert_eq!(
        r.assessments
            .iter()
            .find(|a| a.assertion_id == "no-protocol-fees")
            .unwrap()
            .assessment,
        EvidenceAssessment::Contradicted
    );
    assert_eq!(
        r.assessments
            .iter()
            .find(|a| a.assertion_id == "fee-collection")
            .unwrap()
            .assessment,
        EvidenceAssessment::NotEstablished
    );
    assert_eq!(
        r.assessments
            .iter()
            .find(|a| a.assertion_id == "position-closure")
            .unwrap()
            .assessment,
        EvidenceAssessment::Contradicted
    );
}
#[test]
fn transfer_never_establishes_official_conversion() {
    let e = evaluate(&plan("transfer-official-conversion"));
    let r = e.report();
    assert_eq!(
        r.assessments
            .iter()
            .find(|a| a.assertion_id == "transfer-operation")
            .unwrap()
            .assessment,
        EvidenceAssessment::Supported
    );
    let conversion = r
        .assessments
        .iter()
        .find(|a| a.assertion_id == "official-from-transfer")
        .unwrap();
    assert_eq!(
        conversion.assessment,
        EvidenceAssessment::EvidenceSubstitution,
        "transfer_is_not_official_conversion"
    );
    assert!(conversion
        .reason_codes
        .contains(&ReasonCode::PathEvidenceSubstitution));
    assert!(conversion
        .historical_paths
        .iter()
        .any(|f| f.path_type == ExitPathType::Transfer
            && f.status == PathStatus::Proven
            && f.signer.signer_assumed_locally
            && !f.signer.signer_possession_known));
    assert!(conversion
        .historical_paths
        .iter()
        .any(|f| f.path_type == ExitPathType::OfficialTransition
            && f.status == PathStatus::NotTested
            && !f.execution_attempted));
    assert_eq!(r.readiness.overall_status, ReadinessStatus::Incomplete);
}
#[test]
fn missing_official_execution_never_becomes_failed() {
    for name in ["lp-complete-exit", "transfer-official-conversion"] {
        let e = evaluate(&plan(name));
        assert!(e
            .report()
            .readiness
            .path_evidence
            .iter()
            .filter(|f| f.path_type == ExitPathType::OfficialTransition)
            .all(|f| f.status == PathStatus::NotTested && !f.execution_attempted));
    }
}
#[test]
fn exact_required_failed_route_is_blocked() {
    let e = evaluate(&plan("required-failed-route"));
    let r = e.report();
    assert_eq!(
        r.readiness.overall_status,
        ReadinessStatus::Blocked,
        "required_exact_failure_blocks"
    );
    assert_eq!(r.readiness_exit_code, 1);
    assert_eq!(e.command_exit_code(), 1);
    let finding = r
        .readiness
        .findings
        .iter()
        .find(|f| f.requirement_id == "optional-failed-route")
        .unwrap();
    assert!(finding.required);
    assert_eq!(finding.effect, FindingEffect::Blocking);
    assert_eq!(
        r.assessments[0].assessment,
        EvidenceAssessment::Contradicted
    );
    assert!(r.assessments[0]
        .explanation
        .contains("This exact required route failed under this captured execution context."));
    let f = r.assessments[0]
        .historical_paths
        .iter()
        .find(|f| f.status == PathStatus::Failed)
        .unwrap();
    assert_eq!(f.scope.exact_amount_raw.as_deref(), Some("14577177576"));
    assert_eq!(f.scope.captured_slot, Some(448018480));
    assert_eq!(
        f.scope.venue.as_deref(),
        Some("22PthLk8TYnurtbWKRyECFd99cHHbfsbPNHfeMetzfZg")
    );
    assert!(f.reason.contains("BitmapExtensionAccountIsNotProvided") && f.reason.contains("6036"));
    assert_eq!(f.rollback_verified, Some(true));
    assert!(
        !r.policy_change
            .parent_requirement
            .as_ref()
            .unwrap()
            .required
    );
    assert!(
        r.policy_change
            .resulting_requirement
            .as_ref()
            .unwrap()
            .required
    );
}
#[test]
fn original_optional_route_policy_and_result_remain_identical() {
    let mut p = plan("required-failed-route");
    original_policy(&mut p);
    let e = evaluate(&p);
    let r = e.report();
    assert_eq!(r.readiness.overall_status, ReadinessStatus::Incomplete);
    assert_eq!(
        canonical(&r.readiness).unwrap(),
        std::fs::read_to_string(repo_root().join("reports/spacex-lifecycle-readiness.json"))
            .unwrap()
    );
    let f = r
        .readiness
        .findings
        .iter()
        .find(|f| f.requirement_id == "optional-failed-route")
        .unwrap();
    assert!(!f.required);
    assert_eq!(f.effect, FindingEffect::Informational);
}
#[test]
fn different_entity_amount_venue_bank_range_fraction_and_signer_never_inherit_proof() {
    for kind in 0..7 {
        let mut p = plan("principal-removal-positive-control");
        let a = &mut p.requested_actions[0];
        match kind {
            0 => a.scope.entity_id = "different-position".into(),
            1 => a.scope.exact_amount_raw = Some("1".into()),
            2 => a.scope.venue = Some("different-pool".into()),
            3 => {
                a.scope.captured_slot = Some(1);
                a.scope.clock.as_mut().unwrap().slot = 1;
            }
            4 => a.scope.range = Some([-101, -53]),
            5 => a.scope.bps_to_remove = Some(5000),
            6 => a.signer.signer_possession_known = true,
            _ => unreachable!(),
        }
        let e = evaluate(&p);
        let r = e.report();
        assert_eq!(
            r.candidate_acceptance,
            CandidateAcceptance::NotAccepted,
            "exact_scope_isolation_{kind}"
        );
        assert!(
            r.assessments
                .iter()
                .all(|a| a.assessment == EvidenceAssessment::ScopeMismatch),
            "mismatched_scope_never_inherits_{kind}"
        );
        assert_eq!(r.readiness.overall_status, ReadinessStatus::Ready);
        assert_eq!(r.evaluation_command_exit_code, 5);
        assert_eq!(
            r.guarded_workflow_disposition,
            GuardDisposition::RefusedCandidate
        );
    }
}
#[test]
fn truthful_principal_removal_is_accepted_and_narrowly_ready() {
    let e = positive();
    let r = e.report();
    assert_eq!(r.candidate_acceptance, CandidateAcceptance::Accepted);
    assert_eq!(r.readiness.overall_status, ReadinessStatus::Ready);
    assert_eq!(
        r.readiness.evaluated_scope,
        EvaluatedScope::DemoEntityReadiness
    );
    assert!(r
        .assessments
        .iter()
        .all(|a| a.assessment == EvidenceAssessment::Supported));
    assert!(r.readiness.rollout_readiness.is_none());
    assert_eq!(r.readiness.requirements.len(), 1);
    assert_eq!(r.readiness.requirements[0].id, "lp-principal-unwind");
    assert_ne!(
        r.readiness.policy.id,
        validator().counterfactual().frozen_readiness.policy.id
    );
    assert_eq!(
        validator().counterfactual().frozen_readiness.overall_status,
        ReadinessStatus::Incomplete
    );
    assert!(r
        .limitations
        .iter()
        .any(|s| s.contains("not complete-position exit")
            && s.contains("population rollout readiness")));
}
#[test]
fn pre_event_never_bypasses_future_target_requirements() {
    let world = validator().counterfactual();
    assert_eq!(
        world.scenarios[0].readiness.applicability,
        eplyx_engine::lifecycle::counterfactual::GateApplicability::PreEvent
    );
    let e = evaluate(&plan("lp-complete-exit"));
    let r = e.report();
    assert_eq!(r.candidate.target_view, world.scenarios[1].scenario);
    assert_eq!(r.readiness.overall_status, ReadinessStatus::Incomplete);
    assert_ne!(r.guarded_workflow_disposition, GuardDisposition::Permitted);
    assert_eq!(
        r.readiness.policy_evaluated_at,
        world.frozen_readiness.policy_evaluated_at
    );
    assert_eq!(
        r.position_observations[0].scope.clock,
        world.frozen_readiness.position_exit_evidence[0].scope.clock
    );
    let mut before = plan("lp-complete-exit");
    before.target_view = world.scenarios[0].scenario.clone();
    assert_eq!(
        evaluate(&before).report().readiness.overall_status,
        ReadinessStatus::Incomplete
    );
}
#[test]
fn incomplete_assurance_never_runs_the_stub() {
    let mut p = plan("principal-removal-positive-control");
    original_policy(&mut p);
    let e = evaluate(&p);
    assert_eq!(
        e.report().candidate_acceptance,
        CandidateAcceptance::Accepted
    );
    assert_eq!(
        e.report().readiness.overall_status,
        ReadinessStatus::Incomplete
    );
    let dir = temp("incomplete");
    let marker = dir.join("stub");
    let run = rollout::run_guarded_stub(
        &e,
        GateCommandCompletion::AssuranceEvaluation { exit_code: 5 },
        &marker,
    )
    .unwrap();
    assert!(
        !run.marker_created && !marker.exists(),
        "incomplete_never_authorizes_stub"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn blocked_and_unaccepted_candidates_never_run_the_stub() {
    let dir = temp("blocked");
    for (i, name) in [
        "required-failed-route",
        "lp-complete-exit",
        "transfer-official-conversion",
    ]
    .iter()
    .enumerate()
    {
        let e = evaluate(&plan(name));
        let marker = dir.join(format!("stub-{i}"));
        let run = rollout::run_guarded_stub(
            &e,
            GateCommandCompletion::AssuranceEvaluation {
                exit_code: e.report().readiness_exit_code,
            },
            &marker,
        )
        .unwrap();
        assert!(!run.marker_created && !marker.exists());
    }
    let mut p = plan("principal-removal-positive-control");
    p.assertions
        .push(eplyx_engine::lifecycle::rollout::CandidateAssertion {
            id: "unproved-exit".into(),
            action_id: "principal-removal".into(),
            claim: rollout::Claim::CompletePositionExit,
            evidence_ids: vec!["position-resolution".into()],
            statement: "Complete exit asserted under the narrow principal-removal policy.".into(),
        });
    let e = evaluate(&p);
    assert_eq!(e.report().readiness.overall_status, ReadinessStatus::Ready);
    assert_eq!(e.command_exit_code(), 5);
    let marker = dir.join("unaccepted-ready");
    assert!(
        !rollout::run_guarded_stub(
            &e,
            GateCommandCompletion::AssuranceEvaluation { exit_code: 0 },
            &marker
        )
        .unwrap()
        .marker_created
    );
    assert!(!marker.exists());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn analysis_completion_zero_never_authorizes_the_stub() {
    let e = positive();
    let dir = temp("analysis");
    let marker = dir.join("stub");
    let run = rollout::run_guarded_stub(
        &e,
        GateCommandCompletion::AnalysisCompleted { exit_code: 0 },
        &marker,
    )
    .unwrap();
    assert_eq!(
        run.disposition,
        GuardDisposition::RefusedCommandContext,
        "analysis_success_is_not_assurance_approval"
    );
    assert!(
        !run.marker_created && !marker.exists(),
        "analysis_exit_zero_never_authorizes_stub"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn narrowly_ready_positive_control_creates_only_a_temporary_marker() {
    let e = positive();
    let dir = temp("ready");
    let marker = dir.join("stub");
    let run = rollout::run_guarded_stub(
        &e,
        GateCommandCompletion::AssuranceEvaluation { exit_code: 0 },
        &marker,
    )
    .unwrap();
    assert!(run.marker_created && marker.is_file());
    let bytes = std::fs::read(&marker).unwrap();
    assert_eq!(run.marker_content_sha256, Some(sha256(&bytes)));
    assert!(String::from_utf8(bytes)
        .unwrap()
        .contains("principal-removal assurance only"));
    assert!(rollout::run_guarded_stub(
        &e,
        GateCommandCompletion::AssuranceEvaluation { exit_code: 0 },
        &marker
    )
    .is_err());
    assert!(
        !rollout::run_guarded_stub(
            &e,
            GateCommandCompletion::AssuranceEvaluation { exit_code: 5 },
            &dir.join("wrong-code")
        )
        .unwrap()
        .marker_created
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn candidate_and_evidence_ordering_is_canonical() {
    let mut p = plan("lp-complete-exit");
    for id in ["withdrawal-fixture", "withdrawal-discovery"] {
        p.referenced_evidence.push(
            validator()
                .counterfactual()
                .frozen_readiness
                .evidence_refs
                .iter()
                .find(|r| r.id == id)
                .unwrap()
                .clone(),
        );
        p.assertions[0].evidence_ids.push(id.into());
    }
    let first = evaluate(&p).to_json().unwrap();
    p.assertions.reverse();
    p.requested_actions.reverse();
    p.referenced_evidence.reverse();
    p.required_assurance_conditions.reverse();
    for c in &mut p.assertions {
        c.evidence_ids.reverse();
    }
    assert_eq!(evaluate(&p).to_json().unwrap(), first);
    let round: rollout::CandidateAssessment = serde_json::from_str(&first).unwrap();
    assert_eq!(canonical(&round).unwrap(), first);
}
#[test]
fn tampering_omitted_requirements_and_wrong_target_are_invalid() {
    let base = repo_root().join("probes/phase14-plans");
    let mut p = plan("principal-removal-positive-control");
    p.referenced_evidence[0].artifact.sha256 = "self-declared-proven".into();
    assert!(
        validator().evaluate(&p, &base).is_err(),
        "unverified_candidate_evidence_rejected"
    );
    let mut p = plan("principal-removal-positive-control");
    p.required_assurance_conditions.clear();
    assert!(validator().evaluate(&p, &base).is_err());
    let mut p = plan("principal-removal-positive-control");
    p.target_view.evaluation_time += chrono::Duration::seconds(1);
    assert!(validator().evaluate(&p, &base).is_err());
    let mut p = plan("principal-removal-positive-control");
    p.production_state_digest = "other-world".into();
    assert!(validator().evaluate(&p, &base).is_err());
    let mut p = plan("principal-removal-positive-control");
    p.assertions.push(p.assertions[0].clone());
    assert!(p.normalized().is_err());
    let mut p = plan("principal-removal-positive-control");
    let mut unused = p.requested_actions[0].clone();
    unused.id = "unassessed-action".into();
    p.requested_actions.push(unused);
    assert!(
        p.normalized().is_err(),
        "unassessed_requested_action_rejected"
    );
}
#[test]
fn valid_claim_without_cited_execution_is_not_established() {
    let mut p = plan("principal-removal-positive-control");
    for c in &mut p.assertions {
        c.evidence_ids.clear();
    }
    let e = evaluate(&p);
    assert_eq!(
        e.report().candidate_acceptance,
        CandidateAcceptance::NotAccepted
    );
    assert!(e
        .report()
        .assessments
        .iter()
        .all(|a| a.assessment == EvidenceAssessment::NotEstablished
            && a.reason_codes.contains(&ReasonCode::MissingCitedEvidence)));
}
#[test]
fn historical_artifact_bytes_are_preserved() {
    // The archive-wide source assertion is checked by the phase's STA audit.
    // In MAIN, every imported historical byte is pinned, including dependencies
    // added after T0's conservative static inventory.
    let root = eplyx_engine::repo_root().join("fixtures/lifecycle");
    let manifest: serde_json::Value = load(&root.join("provenance.json")).unwrap();
    let historical: serde_json::Value =
        load(&root.join("sta/reports/phase14-validation/historical-inputs-before.json")).unwrap();
    let files = manifest["files"].as_array().unwrap();
    assert!(!files.is_empty());
    for entry in files {
        let name = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join("sta").join(name)).unwrap();
        assert_eq!(bytes.len() as u64, entry["bytes"].as_u64().unwrap());
        let digest = sha256(&bytes);
        assert_eq!(
            digest,
            entry["sha256"].as_str().unwrap(),
            "historical_artifact_preserved: {name}"
        );
        if let Some(original) = historical["files_sha256"].get(name) {
            assert_eq!(
                digest,
                original.as_str().unwrap(),
                "original archive checksum: {name}"
            );
        }
    }
}
#[test]
fn demo_order_is_deterministic_and_only_positive_control_runs() {
    let mut cases: RolloutDemoCases =
        load(&repo_root().join("probes/phase14-demo-cases.json")).unwrap();
    let first = temp("demo-order-a");
    let second = temp("demo-order-b");
    let base = repo_root().join("probes");
    let a = rollout::run_demo(validator(), &cases, &base, &first).unwrap();
    cases.plans.reverse();
    let b = rollout::run_demo(validator(), &cases, &base, &second).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        a.cases
            .iter()
            .filter(|c| c.observation.marker_created)
            .count(),
        1
    );
    assert_eq!(
        a.cases
            .iter()
            .find(|c| c.observation.marker_created)
            .unwrap()
            .assessment
            .candidate
            .id,
        "principal-removal-positive-control"
    );
    assert_eq!(std::fs::read_dir(&first).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(&second).unwrap().count(), 1);
    std::fs::remove_dir_all(first).unwrap();
    std::fs::remove_dir_all(second).unwrap();
}
fn cli(command: &str, plan: &Path, binding: &Path, extra: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .current_dir(std::env::temp_dir())
        .env_remove("SOLANA_RPC_URL")
        .arg("lifecycle")
        .arg(command)
        .arg("--plan")
        .arg(plan)
        .arg("--binding")
        .arg(binding)
        .args(["--format", "json"])
        .args(extra)
        .output()
        .unwrap()
}
#[test]
fn offline_candidate_cli_codes_portability_and_output_protection() {
    let root = repo_root();
    let dir = temp("cli");
    let binding = root.join("probes/phase14-evidence-binding.json");
    let plan = root.join("probes/phase14-plans/principal-removal-positive-control.json");
    let out = dir.join("assessment.json");
    let run = cli(
        "evaluate-rollout",
        &plan,
        &binding,
        &[std::ffi::OsStr::new("--out"), out.as_os_str()],
    );
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, std::fs::read(&out).unwrap());
    assert_eq!(run.stdout, positive().to_json().unwrap().as_bytes());
    assert_eq!(
        cli(
            "evaluate-rollout",
            &plan,
            &binding,
            &[std::ffi::OsStr::new("--out"), out.as_os_str()]
        )
        .status
        .code(),
        Some(2)
    );
    assert_eq!(run.stdout, std::fs::read(&out).unwrap());
    let failed = root.join("probes/phase14-plans/required-failed-route.json");
    assert_eq!(
        cli("evaluate-rollout", &failed, &binding, &[])
            .status
            .code(),
        Some(1)
    );
    let lp = root.join("probes/phase14-plans/lp-complete-exit.json");
    let marker = dir.join("incomplete-stub");
    let blocked = cli(
        "guard-rollout",
        &lp,
        &binding,
        &[std::ffi::OsStr::new("--marker"), marker.as_os_str()],
    );
    assert_eq!(blocked.status.code(), Some(5));
    let result: rollout::GuardRun = serde_json::from_slice(&blocked.stdout).unwrap();
    assert!(!result.observation.marker_created && !marker.exists());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn malformed_and_verification_failure_cli_inputs_never_run_stub() {
    let root = repo_root();
    let dir = fixture::package("invalid-cli");
    let marker = dir.join("stub");
    let original = root.join("probes/phase14-plans/principal-removal-positive-control.json");
    let binding = root.join("probes/phase14-evidence-binding.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&original).unwrap()).unwrap();
    json["Proven"] = serde_json::json!(true);
    let malformed = dir.join("malformed.json");
    std::fs::write(&malformed, canonical(&json).unwrap()).unwrap();
    let extra = [std::ffi::OsStr::new("--marker"), marker.as_os_str()];
    let run = cli("guard-rollout", &malformed, &binding, &extra);
    assert_eq!(run.status.code(), Some(2));
    assert!(
        !marker.exists()
            && serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap()["error"]["code"]
                == 2
    );
    let mut b: rollout::RolloutEvidenceBinding = load(&binding).unwrap();
    b.parent_policy.sha256 = "tampered".into();
    let tampered = dir.join("binding.json");
    b.parent_policy.file = "policies/stocklana-spacex-preflight-v1.json".into();
    std::fs::write(&tampered, canonical(&b).unwrap()).unwrap();
    let run = cli("guard-rollout", &original, &tampered, &extra);
    assert_eq!(run.status.code(), Some(2));
    assert!(
        !marker.exists()
            && serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap()["error"]["code"]
                == 2
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("artifact digest mismatch")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn actual_analysis_cli_exit_zero_is_not_readiness_authorization() {
    let root = repo_root();
    let run = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .current_dir(&root)
        .env_remove("SOLANA_RPC_URL")
        .arg("lifecycle")
        .args(["compare-scenarios", "--format", "json", "--snapshot"])
        .arg(root.join("snapshots/spacex-exposure.json"))
        .arg("--scenario")
        .arg(root.join("scenarios/spacex-transition.json"))
        .arg("--readiness-policy")
        .arg(root.join("policies/stocklana-spacex-preflight-v1.json"))
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        run.stdout,
        std::fs::read(root.join("reports/spacex-counterfactual-lifecycle.json")).unwrap()
    );
    assert_eq!(
        rollout::guard_decision(
            CandidateAcceptance::Accepted,
            ReadinessStatus::Ready,
            GateCommandCompletion::AnalysisCompleted {
                exit_code: run.status.code().unwrap() as u8
            },
            true
        ),
        GuardDisposition::RefusedCommandContext
    );
}

#[test]
fn narrow_policy_never_authorizes_additional_supported_actions() {
    let mut p = plan("principal-removal-positive-control");
    let transfer = plan("transfer-official-conversion");
    p.requested_actions.push(
        transfer
            .requested_actions
            .iter()
            .find(|a| a.id == "token-movement")
            .unwrap()
            .clone(),
    );
    p.assertions.push(
        transfer
            .assertions
            .iter()
            .find(|a| a.id == "transfer-operation")
            .unwrap()
            .clone(),
    );
    p.referenced_evidence.extend(transfer.referenced_evidence);
    let e = evaluate(&p);
    let r = e.report();
    assert_eq!(r.candidate_acceptance, CandidateAcceptance::Accepted);
    assert_eq!(r.readiness.overall_status, ReadinessStatus::Ready);
    assert!(!r.requested_actions_covered_by_policy);
    assert_eq!(
        r.guarded_workflow_disposition,
        GuardDisposition::RefusedRequestedScope
    );
    assert_eq!(e.command_exit_code(), 5);
    let dir = temp("scope-guard");
    let marker = dir.join("stub");
    let run = rollout::run_guarded_stub(
        &e,
        GateCommandCompletion::AssuranceEvaluation { exit_code: 0 },
        &marker,
    )
    .unwrap();
    assert!(
        !run.marker_created && !marker.exists(),
        "narrow_policy_does_not_assure_extra_requested_actions"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
