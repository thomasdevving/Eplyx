use eplyx_engine::{
    change::{Change, ChangeMetadata, ChangeSpec},
    parameter_change::{
        self as p, ConfigTarget, ExpectedCurrent, Input, Operation, ParameterChange, Status,
    },
    path::{token_transfer, CapturedExecutionFixture},
    replay::hash_bytes,
    standard_programs::{token, token2022},
};
use serde_json::{json, Value};
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
#[test]
fn retained_deployed_elf_pair_and_offline_reproduction() {
    let input = retained();
    let spec = spec(&input, 200);
    let report = p::analyze(&spec, &input).unwrap();
    assert_eq!(report["status"], "semantic_consequence_observed");
    assert_eq!(
        report["baseline"]["reconciliation"]["output_received_raw"],
        "9950"
    );
    assert_eq!(
        report["proposed"]["reconciliation"]["output_received_raw"],
        "9800"
    );
    assert_eq!(
        report["baseline"]["reconciliation"]["token_accounts"][1]["withheld_fee_change_raw"],
        "50"
    );
    assert_eq!(
        report["proposed"]["reconciliation"]["token_accounts"][1]["withheld_fee_change_raw"],
        "200"
    );
    assert_eq!(report["findings"].as_array().unwrap().len(), 2);
    p::verify(&spec, &report).unwrap();
    p::reproduce(&spec, &report).unwrap();
    if let Some(directory) = std::env::var_os("EPLYX_PARAMETER_QUALIFICATION_OUT") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("input.json"),
            eplyx_engine::canonical::document(&input).unwrap(),
        )
        .unwrap();
        std::fs::write(directory.join("change.json"), spec.to_document().unwrap()).unwrap();
        std::fs::write(
            directory.join("report.json"),
            eplyx_engine::canonical::document(&report).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn no_op_and_tiny_rounded_transfer_are_successful_controls() {
    let mut input = retained();
    for (amount, bps) in [(10000, 50), (1, 200), (10000, 0), (10000, 10000)] {
        input.amount_raw = amount;
        let s = spec(&input, bps);
        let report = p::analyze(&s, &input).unwrap();
        p::reproduce(&s, &report).unwrap();
        assert_eq!(report["baseline"]["reconciliation"]["reconciled"], true);
        assert_eq!(report["proposed"]["reconciliation"]["reconciled"], true);
        if bps == 50 || amount == 1 {
            assert_eq!(report["status"], "no_observed_consequence");
        }
    }
}
#[test]
fn spec_identity_and_invalid_contracts() {
    let input = retained();
    let original = spec(&input, 200);
    let id = original.id().unwrap();
    assert_eq!(
        id,
        "7f24680b92d65edec79f97175be9a81cb35dbf8670581af13519c8f444da3a11"
    );
    assert_eq!(original.candidate(), None);
    assert_eq!(original.target_program_id(), None);
    assert_eq!(original.kind().as_str(), p::KIND);
    assert!(original.as_protocol_parameter_change().is_some());
    assert_eq!(
        eplyx_engine::build_info::json()["engine"]["change_specs"][p::KIND],
        json!([1])
    );
    let wire = serde_json::to_value(&original).unwrap();
    for path in [
        "/change/target/program_id",
        "/change/target/config_account",
        "/change/operation/expected_current/account_data_sha256",
        "/change/operation/expected_current/basis_points",
        "/change/operation/expected_current/schedule_epoch",
        "/change/operation/expected_current/maximum_fee_raw",
        "/change/operation/kind",
        "/change/operation/proposed_basis_points",
    ] {
        let mut v = wire.clone();
        let x = v.pointer_mut(path).unwrap();
        *x = if x.is_number() {
            json!(201)
        } else if path.ends_with("schedule_epoch") || path.ends_with("maximum_fee_raw") {
            json!("1")
        } else {
            json!("different")
        };
        // Identity is independent of evidence validation; unsupported identities are still different.
        if let Ok(s) = serde_json::from_value::<ChangeSpec>(v) {
            assert_ne!(s.id().unwrap(), id);
        } else {
            assert!(path.ends_with("/kind"));
        }
    }
    let mut cosmetic = original.clone();
    cosmetic.metadata.label = Some("label".into());
    assert_eq!(cosmetic.id().unwrap(), id);
    let mut invalid = wire.clone();
    invalid["activation"] = json!({"slot":1});
    assert!(ChangeSpec::parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
    invalid["activation"] = Value::Null;
    assert!(serde_json::from_value::<ChangeSpec>(invalid.clone()).is_err());
    let mut legacy =
        serde_json::to_value(ChangeSpec::program_upgrade(token2022::PROGRAM_ID, b"ELF")).unwrap();
    legacy["activation"] = Value::Null;
    assert!(serde_json::from_value::<ChangeSpec>(legacy).is_ok());
    invalid = wire.clone();
    invalid["change"]["operation"]["proposed_basis_points"] = json!(10001);
    assert!(ChangeSpec::parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
    invalid = wire.clone();
    invalid["change"]["operation"]["byte_offset"] = json!(42);
    assert!(ChangeSpec::parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
    assert_eq!(
        ChangeSpec::parse(original.to_document().unwrap().as_bytes())
            .unwrap()
            .id()
            .unwrap(),
        id
    );
}
#[test]
fn mandatory_stale_expectations_prevent_execution_and_pending_does_not_warp_clock() {
    let input = retained();
    let original = spec(&input, 200);
    for field in [
        "account_data_sha256",
        "basis_points",
        "schedule_epoch",
        "maximum_fee_raw",
    ] {
        let mut v = serde_json::to_value(&original).unwrap();
        let x = &mut v["change"]["operation"]["expected_current"][field];
        *x = match field {
            "basis_points" => json!(51),
            "account_data_sha256" => json!("0".repeat(64)),
            _ => json!("1"),
        };
        let s: ChangeSpec = serde_json::from_value(v).unwrap();
        let r = p::analyze(&s, &input).unwrap();
        assert_eq!(r["status"], "current_state_mismatch");
        assert_eq!(r["execution_performed"], false);
        p::verify(&s, &r).unwrap();
    }
    let bytes = std::fs::read(
        eplyx_engine::repo_root()
            .join("fixtures/current/sta/reports/milestone4-validation/live-transfer.capture.json"),
    )
    .unwrap();
    let pending = eplyx_engine::path::current::parameter_input(&bytes).unwrap();
    let s = spec(&pending, 200);
    let r = p::analyze(&s, &pending).unwrap();
    assert_eq!(r["status"], "schedule_not_active");
    assert_eq!(r["execution_performed"], false);
    p::reproduce(&s, &r).unwrap();
}
#[test]
fn exhaustive_preservation_endpoints_and_unknown_or_malformed_tlv_fail_closed() {
    let input = retained();
    let s = spec(&input, 200);
    let Change::ProtocolParameterChange(c) = &s.change else {
        unreachable!()
    };
    let (expected, _) = c.operation.values();
    let plan = input.validate().unwrap();
    let current = plan
        .accounts
        .iter()
        .find(|a| a.address == input.context.mint)
        .unwrap()
        .account
        .clone();
    for bps in [0, 50, 200, 10000] {
        let next = p::mutate(&current, expected, bps, plan.clock.epoch).unwrap();
        let mut restored = next.clone();
        let mut state = StateWithExtensionsMut::<Mint>::unpack(&mut restored.data).unwrap();
        state
            .get_extension_mut::<TransferFeeConfig>()
            .unwrap()
            .newer_transfer_fee
            .transfer_fee_basis_points = 50u16.into();
        assert_eq!(restored, current);
    }
    assert_eq!(
        p::mutate(&current, expected, 10001, plan.clock.epoch)
            .unwrap_err()
            .status,
        Status::InvalidProposedValue
    );
    assert_eq!(
        p::mutate(&current, expected, 200, expected.schedule_epoch - 1)
            .unwrap_err()
            .status,
        Status::ScheduleNotActive
    );
    for variant in 0..4 {
        let mut bad = current.clone();
        match variant {
            0 => bad.data.truncate(180),
            1 => {
                bad.data[166..168].copy_from_slice(&65000u16.to_le_bytes());
            }
            2 => {
                let mut entry = vec![];
                entry.extend_from_slice(&1u16.to_le_bytes());
                entry.extend_from_slice(&108u16.to_le_bytes());
                entry.extend_from_slice(&bad.data[170..278]);
                bad.data.extend(entry);
            }
            _ => bad.owner = "11111111111111111111111111111111".into(),
        }
        let mut e = expected.clone();
        e.account_data_sha256 = hash_bytes(&bad.data);
        assert_eq!(
            p::mutate(&bad, &e, 200, plan.clock.epoch)
                .unwrap_err()
                .status,
            Status::MutationUnsupported
        );
    }
}
#[test]
fn tampered_capture_derived_program_clock_accounts_spec_and_report_fail_closed() {
    let input = retained();
    let spec = spec(&input, 200);
    let report = p::analyze(&spec, &input).unwrap();
    for path in [
        "/derived_proposed_pre_state/proposed_mint_sha256",
        "/derived_proposed_pre_state/mint/data",
        "/change/change_spec_id",
        "/proposed_declaration/proposed_bps",
        "/shared_execution/programs/0/elf_sha256",
        "/shared_execution/clock/epoch",
        "/shared_execution/accounts/0/account/data",
        "/baseline/execution/logs",
        "/findings",
    ] {
        let mut tampered = report.clone();
        *tampered.pointer_mut(path).unwrap() = Value::Null;
        assert!(p::verify(&spec, &tampered).is_err(), "{path}");
    }
    let mut tampered = input.clone();
    tampered.fixture.evidence[3].result["value"][0]["data"][0] = json!("AAAA");
    assert!(tampered.validate().is_err());
    let mut s = spec.clone();
    let Change::ProtocolParameterChange(c) = &mut s.change else {
        unreachable!()
    };
    let Operation::Token2022ActiveNewerTransferFeeBasisPointsV1 {
        proposed_basis_points,
        ..
    } = &mut c.operation;
    *proposed_basis_points = 201;
    assert!(p::verify(&s, &report).is_err());
}

#[test]
fn official_tlv_field_isolation_survives_ordering_and_padding() {
    let input = retained();
    let plan = input.validate().unwrap();
    let current = plan
        .accounts
        .iter()
        .find(|a| a.address == input.context.mint)
        .unwrap()
        .account
        .clone();
    let s = spec(&input, 200);
    let Change::ProtocolParameterChange(c) = &s.change else {
        unreachable!()
    };
    let (expected, _) = c.operation.values();
    let mut at = 166;
    let mut entries = vec![];
    while at + 4 <= current.data.len() {
        let kind = u16::from_le_bytes(current.data[at..at + 2].try_into().unwrap());
        if kind == 0 {
            break;
        }
        let n = u16::from_le_bytes(current.data[at + 2..at + 4].try_into().unwrap()) as usize;
        entries.push(current.data[at..at + 4 + n].to_vec());
        at += 4 + n;
    }
    entries.reverse();
    let mut reordered = current.clone();
    reordered.data.truncate(166);
    for e in entries {
        reordered.data.extend(e);
    }
    reordered.data.extend([0; 16]);
    let mut expected = expected.clone();
    expected.account_data_sha256 = hash_bytes(&reordered.data);
    let mut next = p::mutate(&reordered, &expected, 200, plan.clock.epoch).unwrap();
    let mut state = StateWithExtensionsMut::<Mint>::unpack(&mut next.data).unwrap();
    state
        .get_extension_mut::<TransferFeeConfig>()
        .unwrap()
        .newer_transfer_fee
        .transfer_fee_basis_points = 50u16.into();
    assert_eq!(next, reordered);
}
#[test]
fn synthetic_fee_cap_rounding_endpoints_and_actual_failure_rollback_use_real_vm() {
    // Derived qualification worlds. The retained capture itself is never rewritten or relabeled.
    let input = retained();
    let mut plan = input.validate().unwrap();
    let mint_index = plan
        .accounts
        .iter()
        .position(|a| a.address == input.context.mint)
        .unwrap();
    let mint = &mut plan.accounts[mint_index].account;
    let mut state = StateWithExtensionsMut::<Mint>::unpack(&mut mint.data).unwrap();
    let fee = state.get_extension_mut::<TransferFeeConfig>().unwrap();
    fee.newer_transfer_fee.maximum_fee = 10u64.into();
    fee.newer_transfer_fee.transfer_fee_basis_points = 100u16.into();
    let expected = ExpectedCurrent {
        account_data_sha256: hash_bytes(&mint.data),
        basis_points: 100,
        schedule_epoch: 1032,
        maximum_fee_raw: 10,
    };
    let execute = |plan: &eplyx_engine::path::ProbeExecutionPlan| {
        eplyx_engine::executor::execute_probe_message(
            &plan.accounts,
            &plan.watch,
            plan.clock.clone(),
            &plan.programs,
            plan.message.clone(),
        )
        .unwrap()
    };
    let baseline = execute(&plan);
    assert!(baseline.success);
    let reconcile = |plan: &eplyx_engine::path::ProbeExecutionPlan,
                     x: &eplyx_engine::executor::ProbeTransactionExecution| {
        token_transfer::reconcile_current(
            &input.context.mint,
            &input.context.source,
            &input.context.destination,
            input.context.decimals,
            input.amount_raw,
            plan,
            x,
        )
        .unwrap()
    };
    let bd = reconcile(&plan, &baseline);
    assert!(bd.reconciled);
    assert_eq!(bd.output_received_raw, "9990");
    let next = p::mutate(
        &plan.accounts[mint_index].account,
        &expected,
        200,
        plan.clock.epoch,
    )
    .unwrap();
    plan.accounts[mint_index].account = next;
    let proposed = execute(&plan);
    let pd = reconcile(&plan, &proposed);
    assert_eq!(bd.input_debited_raw, pd.input_debited_raw);
    assert_eq!(bd.output_received_raw, pd.output_received_raw);
    assert_eq!(bd.token_accounts, pd.token_accounts);
    // Instruction is admitted, then deliberately given wrong decimals to qualify atomic VM rejection.
    *plan
        .message
        .instructions
        .last_mut()
        .unwrap()
        .data
        .last_mut()
        .unwrap() = input.context.decimals + 1;
    let rejected = execute(&plan);
    assert!(!rejected.success);
    let rollback = reconcile(&plan, &rejected);
    assert!(rollback.reconciled);
    assert_eq!(rollback.input_debited_raw, "0");
    assert_eq!(rollback.output_received_raw, "0");
}
#[test]
fn corrupted_withheld_balance_cannot_emit_economic_findings_even_with_resealed_report() {
    let input = retained();
    let spec = spec(&input, 200);
    let mut report = p::analyze(&spec, &input).unwrap();
    let mut x: eplyx_engine::executor::ProbeTransactionExecution =
        serde_json::from_value(report["proposed"]["execution"].clone()).unwrap();
    use spl_token_2022_interface::{extension::transfer_fee::TransferFeeAmount, state::Account};
    let account = x.post_accounts.get_mut(&input.context.destination).unwrap();
    let mut state = StateWithExtensionsMut::<Account>::unpack(&mut account.data).unwrap();
    state
        .get_extension_mut::<TransferFeeAmount>()
        .unwrap()
        .withheld_amount = 0u64.into();
    report["proposed"]["execution"] = serde_json::to_value(x).unwrap();
    report.as_object_mut().unwrap().remove("report_sha256");
    report["report_sha256"] = eplyx_engine::canonical::digest(&report).unwrap().into();
    assert!(p::verify(&spec, &report).is_err());
}

#[test]
fn local_cli_durable_store_reader_reproduction_and_overwrite_boundary() {
    let root = tempfile::tempdir().unwrap();
    let input = retained();
    let spec = spec(&input, 200);
    let input_path = root.path().join("input.json");
    let change_path = root.path().join("change.json");
    let report_path = root.path().join("report.json");
    std::fs::write(
        &input_path,
        eplyx_engine::canonical::document(&input).unwrap(),
    )
    .unwrap();
    std::fs::write(&change_path, spec.to_document().unwrap()).unwrap();
    let analyze = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
            .args(["parameter", "analyse", "--change"])
            .arg(&change_path)
            .arg("--input")
            .arg(&input_path)
            .arg("--out")
            .arg(&report_path)
            .arg("--record")
            .arg(root.path())
            .env("SOLANA_RPC_URL", "https://unused.invalid")
            .output()
            .unwrap()
    };
    let output = analyze();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["offline"], true);
    let id = receipt["run_id"].as_str().unwrap();
    let store = eplyx_engine::dashboard::store::Store::open(root.path()).unwrap();
    let run = eplyx_engine::dashboard::view::load(&store, id);
    assert_eq!(run.kind(), p::KIND);
    assert_eq!(eplyx_engine::dashboard::view::state(&run), "Complete");
    assert!(run.problems().is_empty(), "{:?}", run.problems());
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["parameter", "reproduce", "--change"])
        .arg(&change_path)
        .arg("--report")
        .arg(&report_path)
        .env("SOLANA_RPC_URL", "https://unused.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["reproduced"],
        true
    );
    assert!(!analyze().status.success());
    let mut report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    report["status"] = json!("no_observed_consequence");
    std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["parameter", "reproduce", "--change"])
        .arg(&change_path)
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
}
