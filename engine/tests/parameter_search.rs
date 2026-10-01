use eplyx_engine::{
    change::{Change, ChangeMetadata, ChangeSpec},
    parameter_change::{
        self as p, ConfigTarget, ExpectedCurrent, Input, Operation, ParameterChange,
    },
    path::{token_transfer, CapturedExecutionFixture},
    replay::hash_bytes,
    standard_programs::{token, token2022},
};
use serde_json::json;
use spl_token_2022_interface::{
    extension::{
        transfer_fee::TransferFeeConfig, BaseStateWithExtensions, BaseStateWithExtensionsMut,
        StateWithExtensions, StateWithExtensionsMut,
    },
    state::Mint,
};
fn retained() -> Input {
    let root = eplyx_engine::lifecycle::artifact::reference_root();
    let fixture: CapturedExecutionFixture = serde_json::from_slice(
        &std::fs::read(root.join("probes/phase7-captures/fixtures/group-0.json")).unwrap(),
    )
    .unwrap();
    let snapshot = eplyx_engine::lifecycle::LifecycleSnapshot::load(
        &root.join("snapshots/spacex-exposure.json"),
    )
    .unwrap();
    let source = "741ZXYKzgjPZucRhPUQFK7kKAgP1ESJwimhumgGpuPHs";
    let e = snapshot
        .entities
        .iter()
        .find(|e| e.token_account == source)
        .unwrap();
    Input {
        schema_version: 1,
        context: token_transfer::TransferContext {
            genesis_hash: snapshot.source.genesis_hash,
            minimum_slot: snapshot.source.max_observed_slot,
            mint: snapshot.asset.mint,
            program: token::TOKEN_2022_PROGRAM.into(),
            decimals: snapshot.mint_config.decimals,
            source: source.into(),
            owner: e.state.owner.clone(),
            destination: "124XHuTYUNnCf2ABCNcCEdQFpHQ7Y6MnPe9NeouDB1az".into(),
            destination_owner: None,
        },
        amount_raw: 10000,
        fixture_sha256: fixture.sha256().unwrap(),
        source_capture_sha256: None,
        fixture,
    }
}
fn spec(input: &Input, bps: u16) -> ChangeSpec {
    let plan = input.validate().unwrap();
    let mint = &plan
        .accounts
        .iter()
        .find(|a| a.address == input.context.mint)
        .unwrap()
        .account;
    let state = StateWithExtensions::<Mint>::unpack(&mint.data).unwrap();
    let fee = state.get_extension::<TransferFeeConfig>().unwrap();
    ChangeSpec {
        schema_version: 1,
        change_spec_id: None,
        activation: None,
        metadata: ChangeMetadata::default(),
        change: Change::ProtocolParameterChange(Box::new(ParameterChange {
            target: ConfigTarget {
                program_id: token2022::PROGRAM_ID.into(),
                config_account: input.context.mint.clone(),
            },
            operation: Operation::Token2022ActiveNewerTransferFeeBasisPointsV1 {
                expected_current: ExpectedCurrent {
                    account_data_sha256: hash_bytes(&mint.data),
                    basis_points: fee.newer_transfer_fee.transfer_fee_basis_points.into(),
                    schedule_epoch: fee.newer_transfer_fee.epoch.into(),
                    maximum_fee_raw: fee.newer_transfer_fee.maximum_fee.into(),
                },
                proposed_basis_points: bps,
            },
        })),
    }
}

use eplyx_engine::parameter_search as search;
use std::sync::OnceLock;
fn search_input() -> search::Input {
    static INPUT: OnceLock<search::Input> = OnceLock::new();
    INPUT.get_or_init(|| {
        let input = retained(); let change = spec(&input,200);
        let parent_report = p::analyze(&change,&input).unwrap();
        search::Input { change, parent_report, source_capture:None,
            spec: serde_json::from_value(json!({"schema_version":1,"dimension":"transfer_amount_raw",
                "min_raw":"1","max_raw":"10000","predicate":{"kind":"recipient_loss_exceeds","threshold_raw":"100"},
                "budget":{"max_evaluations":64,"max_refinements":16}})).unwrap() }
    }).clone()
}
fn small_report() -> search::Report {
    static REPORT: OnceLock<search::Report> = OnceLock::new();
    REPORT
        .get_or_init(|| {
            let mut input = search_input();
            input.spec.max_raw = 4;
            input.spec.budget.max_evaluations = 4;
            input.spec.budget.max_refinements = 0;
            search::search(&input).unwrap()
        })
        .clone()
}
fn reseal(r: &mut search::Report) {
    let mut v = serde_json::to_value(&*r).unwrap();
    v.as_object_mut().unwrap().remove("report_sha256");
    r.report_sha256 = eplyx_engine::canonical::digest(&v).unwrap();
}
#[test]
fn schema_canonical_exact_quantities_and_explicit_supported_bounds() {
    let input = search_input();
    let schema = serde_json::to_value(&input.spec).unwrap();
    assert_eq!(
        schema,
        json!({"schema_version":1,"dimension":"transfer_amount_raw","min_raw":"1","max_raw":"10000",
        "predicate":{"kind":"recipient_loss_exceeds","threshold_raw":"100"},"budget":{"max_evaluations":64,"max_refinements":16,"max_vm_calls":128}})
    );
    for invalid in [
        json!(0),
        json!("01"),
        json!("+1"),
        json!("18446744073709551616"),
        json!("-1"),
        json!("1.0"),
    ] {
        let mut v = schema.clone();
        v["min_raw"] = invalid;
        assert!(serde_json::from_value::<search::Spec>(v).is_err());
    }
    let mut s = input.spec.clone();
    s.min_raw = u64::MAX;
    s.max_raw = u64::MAX;
    s.predicate = search::Predicate::RecipientLossExceeds {
        threshold_raw: u64::MAX,
    };
    s.validate(u64::MAX).unwrap();
    let plan = serde_json::from_value::<Input>(input.parent_report["retained_input"].clone())
        .unwrap()
        .validate()
        .unwrap();
    let source = plan
        .accounts
        .iter()
        .find(|a| a.address == retained().context.source)
        .unwrap();
    let balance = token::account_amounts(token2022::PROGRAM_ID, &source.account.data)
        .unwrap()
        .0;
    for (lo, hi) in [(0, 1), (2, 1), (1, balance + 1)] {
        let mut s = input.spec.clone();
        s.min_raw = lo;
        s.max_raw = hi;
        let err = s.validate(balance).unwrap_err().to_string();
        assert!(err.contains("requested interval") && err.contains(&balance.to_string()));
    }
    for (evaluations, refinements, vm) in
        [(65, 0, 128), (1, 2, 2), (2, 17, 4), (2, 0, 129), (2, 0, 1)]
    {
        let mut s = input.spec.clone();
        s.budget = search::Budget {
            max_evaluations: evaluations,
            max_refinements: refinements,
            max_vm_calls: vm,
        };
        assert!(s.validate(balance).is_err());
    }
}
#[test]
fn deterministic_bounded_queue_rounding_caps_original_and_overflow() {
    let mut s = search_input().spec;
    s.max_raw = u64::MAX;
    for (b, p, cap) in [
        (50, 200, u64::MAX),
        (0, 10000, u64::MAX),
        (100, 200, 10),
        (0, 0, 0),
        (10000, 10000, u64::MAX),
    ] {
        let q = search::candidates(&s, 10000, b, p, cap);
        assert_eq!(q, search::candidates(&s, 10000, b, p, cap));
        assert!(q.len() < 256);
        assert_eq!(q[0].amount_raw, 1);
        assert_eq!(q[1].amount_raw, u64::MAX);
        assert!(q.iter().any(|c| c.amount_raw == 10000));
        let unique = q
            .iter()
            .map(|c| c.amount_raw)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), q.len());
        if cap == 10 {
            for n in [900, 901, 902, 450, 451, 452] {
                assert!(unique.contains(&n), "cap neighborhood {n}");
            }
        }
    }
    s.max_raw = 10000;
    let q = search::candidates(&s, 10000, 50, 200, u64::MAX);
    for n in [50, 51, 52, 200, 201, 202, 6732, 6733, 6734] {
        assert!(
            q.iter().any(|c| c.amount_raw == n),
            "rounding/threshold {n}"
        );
    }
}
#[test]
fn parent_and_single_dimension_provenance_fail_closed() {
    let input = search_input();
    let parent: Input =
        serde_json::from_value(input.parent_report["retained_input"].clone()).unwrap();
    let original = eplyx_engine::canonical::document(&parent).unwrap();
    let mut tagged = parent.clone();
    tagged.source_capture_sha256 = Some("a".repeat(64));
    let d = search::derive(&tagged, 1).unwrap();
    assert!(d.source_capture_sha256.is_none());
    assert_eq!(d.fixture_sha256, parent.fixture_sha256);
    assert_eq!(
        eplyx_engine::canonical::document(&d.fixture).unwrap(),
        eplyx_engine::canonical::document(&parent.fixture).unwrap()
    );
    assert_ne!(d.sha256().unwrap(), parent.sha256().unwrap());
    assert_eq!(
        eplyx_engine::canonical::document(&parent).unwrap(),
        original
    );
    for path in [
        "/retained_input/fixture/evidence/3/result/value/0/data/0",
        "/shared_execution/clock/epoch",
        "/shared_execution/programs/0/elf_sha256",
    ] {
        let mut tampered = input.clone();
        *tampered.parent_report.pointer_mut(path).unwrap() = json!("bad");
        assert!(search::input_identity(&tampered).is_err());
    }
    let mut missing = input.clone();
    missing.parent_report["retained_input"]["source_capture_sha256"] = json!("a".repeat(64));
    assert!(search::input_identity(&missing).is_err());
    let mut stale = input.clone();
    let mut wire = serde_json::to_value(&stale.change).unwrap();
    wire["change"]["operation"]["expected_current"]["basis_points"] = json!(51);
    stale.change = serde_json::from_value(wire).unwrap();
    stale.parent_report = p::analyze(&stale.change, &parent).unwrap();
    assert!(search::input_identity(&stale).is_err());
    let bytes = std::fs::read(
        eplyx_engine::repo_root()
            .join("fixtures/current/sta/reports/milestone4-validation/live-transfer.capture.json"),
    )
    .unwrap();
    let pending = eplyx_engine::path::current::parameter_input(&bytes).unwrap();
    let mut blocked = input.clone();
    blocked.change = spec(&pending, 200);
    blocked.parent_report = p::analyze(&blocked.change, &pending).unwrap();
    blocked.source_capture = Some(String::from_utf8(bytes).unwrap());
    assert!(search::input_identity(&blocked).is_err());
}
#[test]
fn real_retained_search_witness_fresh_pairs_offline_reproduction_and_budget() {
    let input = search_input();
    let before = eplyx_engine::canonical::document(&input).unwrap();
    let r = search::search(&input).unwrap();
    search::verify(&r).unwrap();
    search::reproduce(&r).unwrap();
    assert_eq!(eplyx_engine::canonical::document(&input).unwrap(), before);
    assert_eq!(
        r.summary["total_vm_calls"],
        2 * r.summary["unique_evaluated_amounts"].as_u64().unwrap()
    );
    assert!(r.summary["total_vm_calls"].as_u64().unwrap() <= 128);
    assert!(r.summary["refinement_evaluations"].as_u64().unwrap() <= 16);
    assert_eq!(r.summary["numeric_domain_fully_enumerated"], false);
    let known = r
        .ledger
        .iter()
        .find(|c| c.candidate.amount_raw == 10000)
        .unwrap();
    let o = known.outcome.as_ref().unwrap();
    assert_eq!(o.baseline_credit_raw.as_deref(), Some("9950"));
    assert_eq!(o.proposed_credit_raw.as_deref(), Some("9800"));
    assert_eq!(o.loss_raw.as_deref(), Some("150"));
    assert_eq!(o.predicate, "matched");
    assert!(o.anomaly.is_none());
    let tiny = r
        .ledger
        .iter()
        .find(|c| c.candidate.amount_raw == 1)
        .unwrap();
    assert_eq!(
        tiny.outcome.as_ref().unwrap().loss_raw.as_deref(),
        Some("0")
    );
    let parent: Input =
        serde_json::from_value(input.parent_report["retained_input"].clone()).unwrap();
    for c in r.ledger.iter().filter(|c| c.attempted) {
        let d: Input =
            serde_json::from_value(c.report.as_ref().unwrap()["retained_input"].clone()).unwrap();
        assert_eq!(d.fixture_sha256, parent.fixture_sha256);
        assert_eq!(
            c.report.as_ref().unwrap()["change"]["change_spec_id"],
            input.change.id().unwrap()
        );
    }
    let id = r.witnesses[0]["witness_sha256"].as_str().unwrap();
    assert_eq!(search::reproduce_witness(&r, id).unwrap()["vm_calls"], 2);
    let root = tempfile::tempdir().unwrap();
    let artifact = root.path().join("portable");
    search::store::begin(&artifact, &input).unwrap();
    search::store::save(&artifact, &r).unwrap();
    let loaded = search::store::load(&artifact).unwrap();
    assert_eq!(loaded.report_sha256, r.report_sha256);
    assert!(search::receipt(&r).is_ok());
    if let Some(out) = std::env::var_os("EPLYX_PARAMETER_SEARCH_OUT") {
        let out = std::path::PathBuf::from(out);
        search::store::begin(&out, &input).unwrap();
        search::store::save(&out, &r).unwrap();
        std::fs::write(out.join("change.json"), input.change.to_document().unwrap()).unwrap();
        std::fs::write(
            out.join("parent-report.json"),
            eplyx_engine::canonical::document(&input.parent_report).unwrap(),
        )
        .unwrap();
        std::fs::write(
            out.join("search-spec.json"),
            eplyx_engine::canonical::document(&input.spec).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn no_witness_controls_completion_exhaustion_and_independent_vm_budget() {
    let r = small_report();
    search::verify(&r).unwrap();
    search::reproduce(&r).unwrap();
    assert!(r.witnesses.is_empty());
    assert_eq!(r.summary["numeric_domain_fully_enumerated"], true);
    assert_eq!(r.summary["completion_status"], "candidate_plan_completed");
    for (max, vm, expected, matching) in [
        (1, 128, "budget_exhausted", false),
        (2, 128, "budget_exhausted", true),
        (64, 2, "execution_limit_reached", false),
        (64, 4, "execution_limit_reached", true),
    ] {
        let mut input = search_input();
        input.spec.budget.max_evaluations = max;
        input.spec.budget.max_refinements = 0;
        input.spec.budget.max_vm_calls = vm;
        let r = search::search(&input).unwrap();
        search::verify(&r).unwrap();
        assert_eq!(r.summary["completion_status"], expected);
        assert_eq!(!r.witnesses.is_empty(), matching);
        assert!(r.summary["total_vm_calls"].as_u64().unwrap() <= u64::from(vm));
    }
    let mut equal = search_input();
    let parent: Input =
        serde_json::from_value(equal.parent_report["retained_input"].clone()).unwrap();
    equal.change = spec(&parent, 50);
    equal.parent_report = p::analyze(&equal.change, &parent).unwrap();
    equal.spec.budget.max_evaluations = 2;
    equal.spec.budget.max_refinements = 0;
    let no_op = search::search(&equal).unwrap();
    assert!(no_op.witnesses.is_empty());
    assert_eq!(
        no_op.summary["maximum_loss_among_tested_successful_cases_raw"],
        "0"
    );
}
#[test]
fn resealed_cross_object_tampering_order_coverage_minimum_and_predicate_rejected() {
    let r = small_report();
    for edit in 0..10 {
        let mut t = r.clone();
        match edit {
            0 => t.summary["numeric_domain_fully_enumerated"] = json!(false),
            1 => t.ledger[0].outcome.as_mut().unwrap().predicate = "matched".into(),
            2 => t.execution_order.swap(0, 1),
            3 => {
                t.execution_order.pop();
            }
            4 => t.ledger[0].candidate.amount_raw = 2,
            5 => {
                t.input.spec.predicate =
                    search::Predicate::RecipientLossExceeds { threshold_raw: 0 }
            }
            6 => t.summary["minimality_claim"] = json!("globally smallest"),
            7 => t.ledger[0].candidate.reasons.push("invented".into()),
            8 => t.ledger[0].derivation["message"] = json!(null),
            _ => t.ledger[0].vm_calls = 1,
        };
        reseal(&mut t);
        assert!(search::verify(&t).is_err(), "tamper {edit}");
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("interrupted");
    search::store::begin(&path, &r.input).unwrap();
    assert!(search::store::load(&path)
        .unwrap_err()
        .to_string()
        .contains("incomplete"));
    search::store::save(&path, &r).unwrap();
    let summary = path.join("summary.json");
    let saved_summary = std::fs::read(&summary).unwrap();
    std::fs::write(&summary, b"{}").unwrap();
    assert!(search::store::load(&path).is_err());
    std::fs::write(&summary, saved_summary).unwrap();
    let root_manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("manifest.json")).unwrap()).unwrap();
    let cas_ref: eplyx_engine::universal::evidence::EvidenceRef =
        serde_json::from_value(root_manifest["report"]["parameter_search_cas"].clone()).unwrap();
    let cas = eplyx_engine::universal::evidence::EvidenceStore::at(path.join("evidence"));
    let object = cas.path(&cas_ref).unwrap();
    let saved = std::fs::read(&object).unwrap();
    std::fs::write(&object, b"{}").unwrap();
    assert!(search::store::load(&path).is_err());
    std::fs::write(&object, saved).unwrap();
    let manifest = path.join("manifest.json");
    let mut wire: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    wire["report_sha256"] = json!("0".repeat(64));
    std::fs::write(&manifest, serde_json::to_vec(&wire).unwrap()).unwrap();
    assert!(search::store::load(&path).is_err());
}
#[test]
fn rejection_rollback_reconciliation_and_unavailable_do_not_invent_zero_loss() {
    let input = search_input();
    let mut report = input.parent_report.clone();
    let mut plan = retained().validate().unwrap();
    let mi = plan
        .accounts
        .iter()
        .position(|a| a.address == retained().context.mint)
        .unwrap();
    let expected = input
        .change
        .as_protocol_parameter_change()
        .unwrap()
        .operation
        .values()
        .unwrap()
        .0;
    plan.accounts[mi].account =
        p::mutate(&plan.accounts[mi].account, expected, 200, plan.clock.epoch).unwrap();
    *plan
        .message
        .instructions
        .last_mut()
        .unwrap()
        .data
        .last_mut()
        .unwrap() += 1;
    let rejected = eplyx_engine::executor::execute_probe_message(
        &plan.accounts,
        &plan.watch,
        plan.clock.clone(),
        &plan.programs,
        plan.message.clone(),
    )
    .unwrap();
    assert!(!rejected.success);
    report["proposed"]["execution"] = serde_json::to_value(&rejected).unwrap();
    report["proposed"]["reconciliation"]["reconciled"] = json!(true);
    let o = search::outcome(&report, &input.spec.predicate).unwrap();
    assert_eq!(
        o.anomaly.as_deref(),
        Some("baseline_succeeds_proposed_rejects")
    );
    assert_eq!(o.predicate, "unavailable");
    assert!(o.loss_raw.is_none());
    // A rejected execution's changed watched bytes independently fail rollback.
    report["proposed"]["execution"]["post_accounts"][retained().context.destination.clone()]
        ["lamports"] = json!("1");
    assert_eq!(
        search::outcome(&report, &input.spec.predicate)
            .unwrap()
            .anomaly
            .as_deref(),
        Some("rollback_failure")
    );
    let mut report = input.parent_report.clone();
    report["proposed"]["reconciliation"]["reconciled"] = json!(false);
    let o = search::outcome(&report, &input.spec.predicate).unwrap();
    assert!(o.loss_raw.is_none());
    assert_eq!(o.anomaly.as_deref(), Some("reconciliation_failure"));
    report["execution_performed"] = json!(false);
    report["status"] = json!("execution_unavailable");
    assert_eq!(
        search::outcome(&report, &input.spec.predicate)
            .unwrap()
            .status,
        "execution_unavailable"
    );
    report["status"] = json!("downstream_action_unsupported");
    assert_eq!(
        search::outcome(&report, &input.spec.predicate)
            .unwrap()
            .status,
        "admission_unsupported"
    );
}
#[test]
fn cli_portable_worker_empty_environment_and_selected_witness() {
    let input = search_input();
    let root = tempfile::tempdir().unwrap();
    let artifact = root.path().join("cli-artifact");
    for (name, value) in [
        ("parent", input.parent_report.clone()),
        ("change", serde_json::to_value(&input.change).unwrap()),
        ("spec", {
            let mut s = input.spec.clone();
            s.budget.max_evaluations = 3;
            s.budget.max_refinements = 0;
            serde_json::to_value(s).unwrap()
        }),
    ] {
        std::fs::write(
            root.path().join(format!("{name}.json")),
            eplyx_engine::canonical::document(&value).unwrap(),
        )
        .unwrap();
    }
    let cli = |args: Vec<String>| {
        std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
            .env(
                "SOLANA_RPC_URL",
                "https://unreachable.invalid/?api-key=must-not-inherit",
            )
            .env("EPLYX_TOKEN", "must-not-inherit")
            .args(args)
            .output()
            .unwrap()
    };
    let out = cli(vec![
        "parameter".into(),
        "search".into(),
        "--change".into(),
        root.path().join("change.json").display().to_string(),
        "--parent-report".into(),
        root.path().join("parent.json").display().to_string(),
        "--spec".into(),
        root.path().join("spec.json").display().to_string(),
        "--out".into(),
        artifact.display().to_string(),
        "--format".into(),
        "json".into(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(receipt["summary"]["total_vm_calls"], 6);
    for command in ["verify-search", "reproduce-search"] {
        let o = cli(vec![
            "parameter".into(),
            command.into(),
            "--artifact".into(),
            artifact.display().to_string(),
            "--format".into(),
            "json".into(),
        ]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let o = cli(vec![
        "parameter".into(),
        "reproduce-witness".into(),
        "--artifact".into(),
        artifact.display().to_string(),
        "--witness".into(),
        receipt["witnesses"][0]["witness_sha256"]
            .as_str()
            .unwrap()
            .into(),
        "--format".into(),
        "json".into(),
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap()["vm_calls"],
        2
    );
    // Invoking the hidden worker with credentials is rejected before any action.
    let rejected = cli(vec!["parameter-worker".into(), "{}".into()]);
    assert!(!rejected.status.success());
}

fn labelled_control(cap: Option<u64>, withheld: Option<u64>, original: u64) -> Input {
    use base64::{prelude::BASE64_STANDARD, Engine};
    use spl_token_2022_interface::{extension::transfer_fee::TransferFeeAmount, state::Account};
    let mut input = retained();
    input.amount_raw = original;
    input.fixture.rpc_origin = "labelled_synthetic_fee_or_overflow_control".into();
    let addresses = input.fixture.evidence[3].params[0]
        .as_array()
        .unwrap()
        .clone();
    if let Some(cap) = cap {
        let i = addresses
            .iter()
            .position(|a| a == &input.context.mint)
            .unwrap();
        let v = &mut input.fixture.evidence[3].result["value"][i];
        let mut bytes = BASE64_STANDARD
            .decode(v["data"][0].as_str().unwrap())
            .unwrap();
        let mut state = StateWithExtensionsMut::<Mint>::unpack(&mut bytes).unwrap();
        state
            .get_extension_mut::<TransferFeeConfig>()
            .unwrap()
            .newer_transfer_fee
            .maximum_fee = cap.into();
        v["data"][0] = json!(BASE64_STANDARD.encode(bytes));
    }
    if let Some(withheld) = withheld {
        let i = addresses
            .iter()
            .position(|a| a == &input.context.destination)
            .unwrap();
        let v = &mut input.fixture.evidence[3].result["value"][i];
        let mut bytes = BASE64_STANDARD
            .decode(v["data"][0].as_str().unwrap())
            .unwrap();
        let mut state = StateWithExtensionsMut::<Account>::unpack(&mut bytes).unwrap();
        state
            .get_extension_mut::<TransferFeeAmount>()
            .unwrap()
            .withheld_amount = withheld.into();
        v["data"][0] = json!(BASE64_STANDARD.encode(bytes));
    }
    input.fixture_sha256 = input.fixture.sha256().unwrap();
    input
}
#[test]
fn labelled_cap_non_monotone_and_actual_overflow_rejection_search_controls() {
    let original = retained();
    let cap = labelled_control(Some(10), None, 10000);
    assert_ne!(cap.fixture_sha256, original.fixture_sha256);
    let mut input = search_input();
    input.change = spec(&cap, 200);
    input.parent_report = p::analyze(&input.change, &cap).unwrap();
    input.spec.max_raw = 2500;
    input.spec.predicate = search::Predicate::RecipientLossExceeds { threshold_raw: 2 };
    input.spec.budget.max_evaluations = 12;
    input.spec.budget.max_refinements = 3;
    let r = search::search(&input).unwrap();
    search::verify(&r).unwrap();
    assert!(r
        .witnesses
        .iter()
        .any(|w| w["outcome"]["predicate"] == "matched"));
    assert_eq!(
        r.ledger
            .iter()
            .find(|c| c.candidate.amount_raw == 2500)
            .unwrap()
            .outcome
            .as_ref()
            .unwrap()
            .predicate,
        "not_matched"
    );
    assert!(r.summary["minimality_claim"]
        .as_str()
        .unwrap()
        .contains("no global minimum"));
    // Existing real deployed program, no artificial bug program: only a labelled
    // test control puts withheld fees near u64 overflow. Production derivation
    // still changes only amount and never modifies this fixed test state.
    let overflow = labelled_control(None, Some(u64::MAX - 60), 1);
    input.change = spec(&overflow, 200);
    input.parent_report = p::analyze(&input.change, &overflow).unwrap();
    input.spec.max_raw = 10000;
    input.spec.budget.max_evaluations = 2;
    input.spec.budget.max_refinements = 0;
    let r = search::search(&input).unwrap();
    search::verify(&r).unwrap();
    let c = r
        .ledger
        .iter()
        .find(|c| c.candidate.amount_raw == 10000)
        .unwrap();
    assert_eq!(
        c.outcome.as_ref().unwrap().anomaly.as_deref(),
        Some("baseline_succeeds_proposed_rejects")
    );
    assert!(c.outcome.as_ref().unwrap().loss_raw.is_none());
    search::reproduce_witness(&r, r.witnesses[0]["witness_sha256"].as_str().unwrap()).unwrap();
}

#[test]
fn explicit_prior_runtime_receipt_compatibility_and_unknown_source_rejection() {
    let input = search_input();
    let mut report = input.parent_report.clone();
    report["runtime"]["derivation_source_sha256"] =
        json!("51643aa9ca1fd8dcba3d21bf8af8b9e45ac53f39b92a66616dade06b3445de74");
    report["shared_execution"]["runtime"] = report["runtime"].clone();
    report["shared_execution_sha256"] =
        eplyx_engine::canonical::digest(&report["shared_execution"])
            .unwrap()
            .into();
    report.as_object_mut().unwrap().remove("report_sha256");
    report["report_sha256"] = eplyx_engine::canonical::digest(&report).unwrap().into();
    p::verify(&input.change, &report).unwrap();
    p::reproduce(&input.change, &report).unwrap();
    report["runtime"]["derivation_source_sha256"] = json!("0".repeat(64));
    report.as_object_mut().unwrap().remove("report_sha256");
    report["report_sha256"] = eplyx_engine::canonical::digest(&report).unwrap().into();
    assert!(p::verify(&input.change, &report).is_err());
}
