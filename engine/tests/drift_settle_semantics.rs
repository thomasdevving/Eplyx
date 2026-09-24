use std::{collections::BTreeMap, path::PathBuf};

use eplyx_engine::{
    ci::semantic_result,
    corpus_store::CorpusStore,
    executor::ExecutionResult,
    protocol::{drift::DriftSettlePnlAdapter, ProtocolAdapter},
    semantic_binding::{ObservationSemanticBinding, SemanticBinding, SemanticBindingReport},
    semantics::{ChangeKind, SemanticValue},
    types::NamedAccount,
    universal::{evidence::EvidenceStore, pipeline},
};

fn fixture() -> (
    eplyx_engine::ingest::transactions::HistoricalTransaction,
    Vec<NamedAccount>,
    ExecutionResult,
    eplyx_engine::universal::execution::ExecutionEvidence,
    BTreeMap<String, eplyx_engine::types::AccountSnapshot>,
    Vec<u8>,
) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u13-3-sequence-corpus");
    let record = CorpusStore::open(&root)
        .unwrap()
        .load_v2()
        .unwrap()
        .pop()
        .unwrap();
    let resolved = record
        .resolve(&EvidenceStore::at(root.join("evidence")))
        .unwrap();
    let (baseline, _) = pipeline::baseline(&record, &resolved).unwrap();
    let adapter = DriftSettlePnlAdapter;
    let tx = resolved.message.transaction.clone();
    let labels = resolved
        .message
        .account_keys
        .iter()
        .enumerate()
        .map(|(index, address)| (address.clone(), adapter.label(&tx, index)))
        .collect::<BTreeMap<_, _>>();
    let pre = resolved
        .watched
        .iter()
        .map(|address| NamedAccount {
            label: labels[address].clone(),
            address: address.clone(),
            account: resolved.seeds[address].clone(),
        })
        .collect();
    let result = semantic_result(&baseline, &labels, &resolved.message.account_keys).unwrap();
    (
        tx,
        pre,
        result,
        baseline,
        resolved.seeds,
        resolved.baseline_elf,
    )
}

#[test]
fn frozen_target_has_signed_pnl_subject_and_corroborated_binding() {
    let (tx, pre, baseline, evidence, seeds, elf) = fixture();
    let adapter = DriftSettlePnlAdapter;
    adapter.accept_instruction_contract(&tx).unwrap();
    let subjects = adapter.evaluable_subjects(&tx, &pre);
    assert_eq!(subjects.len(), 2);
    assert_eq!(
        subjects[1].to_string(),
        "drift-settle-pnl/settle_pnl/economic/pnl_settled"
    );
    assert!(adapter
        .named_findings(&tx, &pre, &baseline, &baseline)
        .is_empty());
    let binding = adapter
        .semantic_binding(&tx, &seeds, &elf, &evidence)
        .unwrap();
    assert!(matches!(
        binding,
        SemanticBinding::ExecutionCorroboratedExternalInterface { .. }
    ));
    assert!(!binding.exact_source_to_elf_verified());
    binding
        .validate(adapter.program_id(), &elf, &evidence)
        .unwrap();
}

fn set_i64(result: &mut ExecutionResult, label: &str, offset: usize, value: i64) {
    result.accounts.get_mut(label).unwrap().data[offset..offset + 8]
        .copy_from_slice(&value.to_le_bytes());
}
fn set_u64(result: &mut ExecutionResult, label: &str, offset: usize, value: u64) {
    result.accounts.get_mut(label).unwrap().data[offset..offset + 8]
        .copy_from_slice(&value.to_le_bytes());
}
fn set_u128(result: &mut ExecutionResult, label: &str, offset: usize, value: u128) {
    result.accounts.get_mut(label).unwrap().data[offset..offset + 16]
        .copy_from_slice(&value.to_le_bytes());
}
fn set_i128(result: &mut ExecutionResult, label: &str, offset: usize, value: i128) {
    result.accounts.get_mut(label).unwrap().data[offset..offset + 16]
        .copy_from_slice(&value.to_le_bytes());
}
fn read<const N: usize>(pre: &[NamedAccount], label: &str, offset: usize) -> [u8; N] {
    pre.iter().find(|a| a.label == label).unwrap().account.data[offset..offset + N]
        .try_into()
        .unwrap()
}
fn settlement(pre: &[NamedAccount], baseline: &ExecutionResult, amount: i64) -> ExecutionResult {
    let mut candidate = baseline.clone();
    let interest = u128::from_le_bytes(
        candidate.accounts["quote-spot-market"].data[464..480]
            .try_into()
            .unwrap(),
    );
    let pre_scaled = u64::from_le_bytes(read(pre, "user", 104));
    let pre_tokens = u128::from(pre_scaled) * interest / 10_000_000_000_000;
    let rough = amount.unsigned_abs() as u128 * 10_000_000_000_000 / interest;
    let scaled_delta = rough;
    let next_scaled = if amount >= 0 {
        u128::from(pre_scaled) + scaled_delta
    } else {
        u128::from(pre_scaled) - scaled_delta
    };
    assert_eq!(
        (next_scaled * interest / 10_000_000_000_000) as i128 - pre_tokens as i128,
        i128::from(amount)
    );
    set_u64(&mut candidate, "user", 104, next_scaled as u64);
    let pre_quote = i64::from_le_bytes(read(pre, "user", 536));
    set_i64(&mut candidate, "user", 536, pre_quote - amount);
    set_i64(
        &mut candidate,
        "user",
        576,
        i64::from_le_bytes(read(pre, "user", 576)) + amount,
    );
    set_i64(
        &mut candidate,
        "user",
        4296,
        i64::from_le_bytes(read(pre, "user", 4296)) + amount,
    );
    let pre_pool = u128::from_le_bytes(read(pre, "perp-market", 976));
    let next_pool = if amount >= 0 {
        pre_pool - scaled_delta
    } else {
        pre_pool + scaled_delta
    };
    set_u128(&mut candidate, "perp-market", 976, next_pool);
    let pre_amm = i128::from_le_bytes(read(pre, "perp-market", 384));
    set_i128(
        &mut candidate,
        "perp-market",
        384,
        pre_amm - i128::from(amount),
    );
    let pre_users = u32::from_le_bytes(read(pre, "perp-market", 1156));
    let next_users = if pre_quote - amount == 0 {
        pre_users - 1
    } else {
        pre_users
    };
    candidate.accounts.get_mut("perp-market").unwrap().data[1156..1160]
        .copy_from_slice(&next_users.to_le_bytes());
    candidate
}

#[test]
fn controlled_settlement_changes_have_stable_signed_fingerprints() {
    let (tx, pre, baseline, ..) = fixture();
    let adapter = DriftSettlePnlAdapter;
    for (amount, change) in [
        (303, ChangeKind::Increased),
        (103, ChangeKind::Decreased),
        (-203, ChangeKind::Decreased),
    ] {
        let candidate = settlement(&pre, &baseline, amount);
        let findings = adapter.named_findings(&tx, &pre, &baseline, &candidate);
        assert_eq!(findings.len(), 1, "settlement {amount}");
        assert_eq!(findings[0].fingerprint.change, change);
        assert_eq!(
            findings[0].baseline,
            Some(SemanticValue::signed_quantity(203, 6))
        );
        assert_eq!(
            findings[0].candidate,
            Some(SemanticValue::signed_quantity(i128::from(amount), 6))
        );
        assert_eq!(
            findings[0].fingerprint.to_string(),
            format!(
                "drift-settle-pnl/settle_pnl/economic/pnl_settled/{}",
                change.as_str()
            )
        );
    }
}

#[test]
fn malformed_and_inconsistent_candidate_state_never_becomes_zero() {
    let (tx, pre, baseline, evidence, seeds, elf) = fixture();
    let adapter = DriftSettlePnlAdapter;
    for label in ["user", "perp-market", "quote-spot-market"] {
        let mut candidate = baseline.clone();
        candidate.accounts.get_mut(label).unwrap().data.truncate(7);
        assert!(adapter
            .named_findings(&tx, &pre, &baseline, &candidate)
            .is_empty());
        let mut candidate = baseline.clone();
        candidate.accounts.get_mut(label).unwrap().owner =
            "11111111111111111111111111111111".into();
        assert!(adapter
            .named_findings(&tx, &pre, &baseline, &candidate)
            .is_empty());
    }
    for address in [
        "JE9m89yHHiCGzzL2FAeeZgHKAFwjkW4Qp1GfjegWnojR",
        "7QAtMC3AaAc91W4XuwYXM1Mtffq9h9Z8dTxcJrKRHu1z",
        "6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3",
    ] {
        let mut malformed_pre = seeds.clone();
        malformed_pre.get_mut(address).unwrap().data.truncate(7);
        assert!(adapter
            .semantic_binding(&tx, &malformed_pre, &elf, &evidence)
            .is_err());
    }
    let mut wrong_market = baseline.clone();
    wrong_market.accounts.get_mut("perp-market").unwrap().data[1160] = 4;
    assert!(adapter
        .named_findings(&tx, &pre, &baseline, &wrong_market)
        .is_empty());
    for (label, offset) in [("user", 136), ("user", 612), ("quote-spot-market", 684)] {
        let mut wrong_index = baseline.clone();
        wrong_index.accounts.get_mut(label).unwrap().data[offset] ^= 1;
        assert!(adapter
            .named_findings(&tx, &pre, &baseline, &wrong_index)
            .is_empty());
    }
    let mut inconsistent = settlement(&pre, &baseline, 303);
    set_i64(&mut inconsistent, "user", 576, 0);
    assert!(adapter
        .named_findings(&tx, &pre, &baseline, &inconsistent)
        .is_empty());
}

#[test]
fn logs_do_not_override_state_and_revert_is_an_execution_finding() {
    let (tx, pre, baseline, ..) = fixture();
    let adapter = DriftSettlePnlAdapter;
    let mut changed_logs = baseline.clone();
    changed_logs
        .logs
        .push("SettlePnlRecord pnl: 999999999".into());
    assert!(adapter
        .named_findings(&tx, &pre, &baseline, &changed_logs)
        .is_empty());
    let mut revert = baseline.clone();
    revert.success = false;
    let findings = adapter.named_findings(&tx, &pre, &baseline, &revert);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].fingerprint.change, ChangeKind::NowReverts);
}

#[test]
fn unknown_raw_change_is_outside_explained_ranges_and_provenance_downgrade_is_visible() {
    let (tx, pre, baseline, evidence, seeds, elf) = fixture();
    let adapter = DriftSettlePnlAdapter;
    let mut candidate = baseline.clone();
    candidate.accounts.get_mut("user").unwrap().data[4360] ^= 1;
    assert!(adapter
        .named_findings(&tx, &pre, &baseline, &candidate)
        .is_empty());
    assert!(!adapter
        .decoded_byte_ranges("user")
        .iter()
        .any(|range| range.contains(&4360)));
    let mut combined = settlement(&pre, &baseline, 303);
    combined.accounts.get_mut("user").unwrap().data[4360] ^= 1;
    assert_eq!(
        adapter
            .named_findings(&tx, &pre, &baseline, &combined)
            .len(),
        1
    );
    let binding = adapter
        .semantic_binding(&tx, &seeds, &elf, &evidence)
        .unwrap();
    let source = match binding {
        SemanticBinding::ExecutionCorroboratedExternalInterface { source, .. } => source,
        _ => panic!("historical binding must be corroborated"),
    };
    let report = SemanticBindingReport::new(vec![ObservationSemanticBinding {
        observation_id: "frozen-witness".into(),
        binding: SemanticBinding::RepositorySourceClaim { source },
    }]);
    assert_eq!(
        report.observations[0].binding.level(),
        "repository_source_claim"
    );
    assert!(!report.exact_source_to_elf_verified);
}

#[test]
fn signed_settlement_value_round_trips_without_float_conversion() {
    let value = SemanticValue::signed_quantity(-203, 6);
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(json, r#"{"kind":"signed_quantity","quantity":"-0.000203"}"#);
    assert_eq!(serde_json::from_str::<SemanticValue>(&json).unwrap(), value);
}

#[test]
fn ordinary_ci_keeps_semantic_and_frozen_none_bundles_distinct() {
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/examples");
    for (directory, adapter, exit, coverage) in [
        ("phase-u14-drift-semantic-bundle", "drift-settle-pnl", 0, 2),
        ("phase-u13-3-sequence-bundle", "none", 2, 0),
    ] {
        let root = docs.join(directory);
        let report =
            eplyx_engine::ci::check(&root, &root.join("binaries/current.so"), None).unwrap();
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["bundle"]["adapter"], adapter);
        assert_eq!(value["replay_proof"]["status"], "matched");
        assert_eq!(
            value["replay_proof"]["proof_contract_versions"],
            serde_json::json!([3])
        );
        assert_eq!(value["summary"]["exit_code"], exit);
        assert_eq!(value["coverage"].as_array().unwrap().len(), coverage);
        if adapter == "none" {
            assert!(value["semantic_binding"].is_null());
            assert_eq!(
                value["summary"]["failure_reasons"],
                serde_json::json!(["no_semantic_coverage"])
            );
        } else {
            assert_eq!(value["findings"].as_array().unwrap().len(), 0);
            assert_eq!(
                value["semantic_binding"]["observations"][0]["binding"]["level"],
                "execution_corroborated_external_interface"
            );
            assert_eq!(
                value["semantic_binding"]["exact_source_to_elf_verified"],
                false
            );
        }
    }
}
