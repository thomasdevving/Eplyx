use std::{collections::BTreeMap, path::PathBuf};

use eplyx_engine::{
    ci::{self, semantic_result},
    corpus_store::CorpusStore,
    executor::ExecutionResult,
    protocol::{
        orca::OrcaSwapV2Adapter, ProtocolAdapter, SemanticEvaluation, SemanticEvaluationContext,
    },
    standard_programs::{spl_token, token2022},
    types::{AccountSnapshot, NamedAccount},
    universal::{evidence::EvidenceStore, pipeline},
};

fn fixture() -> (
    eplyx_engine::ingest::transactions::HistoricalTransaction,
    Vec<NamedAccount>,
    ExecutionResult,
) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u11-2-checkpointed-corpus");
    let record = CorpusStore::open(&root)
        .unwrap()
        .load_v2()
        .unwrap()
        .pop()
        .unwrap();
    let resolved = record
        .resolve(&EvidenceStore::at(root.join("evidence")))
        .unwrap();
    let tx = resolved.message.transaction.clone();
    let adapter = OrcaSwapV2Adapter;
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
            label: labels.get(address).unwrap().clone(),
            address: address.clone(),
            account: resolved
                .seeds
                .get(address)
                .cloned()
                .unwrap_or_else(|| AccountSnapshot {
                    lamports: 0,
                    owner: "11111111111111111111111111111111".into(),
                    data: Vec::new(),
                    executable: false,
                    rent_epoch: 0,
                }),
        })
        .collect();
    let (baseline, _) = pipeline::baseline(&record, &resolved).unwrap();
    let result = semantic_result(&baseline, &labels, &resolved.message.account_keys).unwrap();
    (tx, pre, result)
}

fn mutate_amount(result: &mut ExecutionResult, label: &str, change: i64) {
    let account = result.accounts.get_mut(label).unwrap();
    let offset = spl_token::ACCOUNT_AMOUNT;
    let amount = u64::from_le_bytes(account.data[offset..offset + 8].try_into().unwrap());
    let next = amount.checked_add_signed(change).unwrap();
    account.data[offset..offset + 8].copy_from_slice(&next.to_le_bytes());
}

fn amount_of(account: &AccountSnapshot) -> u64 {
    u64::from_le_bytes(
        account.data[spl_token::ACCOUNT_AMOUNT..spl_token::ACCOUNT_AMOUNT + 8]
            .try_into()
            .unwrap(),
    )
}

fn set_amount(account: &mut AccountSnapshot, value: u64) {
    account.data[spl_token::ACCOUNT_AMOUNT..spl_token::ACCOUNT_AMOUNT + 8]
        .copy_from_slice(&value.to_le_bytes());
}

#[test]
fn frozen_direct_swap_has_only_proven_economic_subjects() {
    let (tx, pre, baseline) = fixture();
    let adapter = OrcaSwapV2Adapter;
    assert_eq!(
        tx.instructions
            .iter()
            .filter(|ix| ix.program == adapter.program_id())
            .count(),
        1
    );
    assert_eq!(
        pre.iter()
            .find(|a| a.label == "vault-a")
            .unwrap()
            .account
            .owner,
        spl_token::PROGRAM_ID
    );
    assert_eq!(
        pre.iter()
            .find(|a| a.label == "vault-b")
            .unwrap()
            .account
            .owner,
        token2022::PROGRAM_ID
    );
    assert_eq!(
        pre.iter()
            .find(|a| a.label == "user-token-b")
            .unwrap()
            .account
            .owner,
        token2022::PROGRAM_ID
    );
    assert_eq!(
        adapter
            .evaluable_subjects(&tx, &pre)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![
            "orca-whirlpool/swap_v2/execution/transaction",
            "orca-whirlpool/swap_v2/economic/user_input_spent",
            "orca-whirlpool/swap_v2/economic/vault_a_tokens_out",
            "orca-whirlpool/swap_v2/economic/vault_b_tokens_in",
        ]
    );
    assert!(adapter
        .named_findings(&tx, &pre, &baseline, &baseline)
        .is_empty());
    assert!(baseline
        .accounts
        .get("user-token-a")
        .is_none_or(|account| account.data.is_empty())); // closed WSOL account
}

#[test]
fn evaluation_contract_reports_four_subjects_or_typed_failure() {
    let (tx, pre, baseline) = fixture();
    let adapter = OrcaSwapV2Adapter;
    let context = |transaction, candidate| SemanticEvaluationContext {
        transaction,
        pre: &pre,
        baseline: &baseline,
        candidate,
    };
    let SemanticEvaluation::Evaluated {
        subjects, findings, ..
    } = adapter
        .evaluate_semantics(&context(&tx, &baseline))
        .unwrap()
    else {
        panic!("frozen swap must evaluate")
    };
    assert_eq!(subjects.len(), 4);
    assert!(findings.is_empty());
    let mut wrong = tx.clone();
    wrong.instructions[4].data[0] ^= 1;
    assert!(matches!(
        adapter
            .evaluate_semantics(&context(&wrong, &baseline))
            .unwrap(),
        SemanticEvaluation::Unsupported
    ));
    let mut malformed = baseline.clone();
    malformed
        .accounts
        .get_mut("user-token-b")
        .unwrap()
        .data
        .truncate(50);
    assert!(matches!(
        adapter
            .evaluate_semantics(&context(&tx, &malformed))
            .unwrap(),
        SemanticEvaluation::Unevaluable { .. }
    ));
    let mut changed = baseline.clone();
    mutate_amount(&mut changed, "user-token-b", -1);
    let SemanticEvaluation::Evaluated {
        subjects, findings, ..
    } = adapter.evaluate_semantics(&context(&tx, &changed)).unwrap()
    else {
        panic!("changed swap must evaluate")
    };
    assert_eq!(subjects.len(), 4);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].fingerprint.subject.as_str(), "user_input_spent");
}

#[test]
fn bounded_swap_shape_and_malformed_token_accounts_fail_closed() {
    let (tx, pre, baseline) = fixture();
    let adapter = OrcaSwapV2Adapter;
    let mut wrong = tx.clone();
    wrong.instructions[4].data[0] ^= 1;
    assert!(
        adapter.evaluable_subjects(&wrong, &pre).is_empty(),
        "wrong discriminator"
    );
    let mut exact_out = tx.clone();
    exact_out.instructions[4].data[40] = 0;
    assert!(
        adapter.evaluable_subjects(&exact_out, &pre).is_empty(),
        "exact-output behavior is outside this frozen shape"
    );
    let mut fewer = tx.clone();
    fewer.instructions[4].accounts.pop();
    assert!(
        adapter.evaluable_subjects(&fewer, &pre).is_empty(),
        "missing role"
    );
    let mut aliases = tx.clone();
    aliases.instructions[4].accounts[9] = aliases.instructions[4].accounts[7].clone();
    assert!(
        adapter.evaluable_subjects(&aliases, &pre).is_empty(),
        "aliased role"
    );
    let mut companion_write = tx.clone();
    companion_write.instructions[5]
        .accounts
        .push(tx.instructions[4].accounts[10].clone());
    assert!(
        adapter
            .evaluable_subjects(&companion_write, &pre)
            .is_empty(),
        "another outer instruction could change the measured vault"
    );
    let mut malformed = pre.clone();
    malformed
        .iter_mut()
        .find(|a| a.label == "user-token-b")
        .unwrap()
        .account
        .data
        .truncate(50);
    assert_eq!(
        adapter.evaluable_subjects(&tx, &malformed).len(),
        3,
        "malformed user amount is unsupported, never zero"
    );
    assert!(adapter
        .named_findings(&tx, &malformed, &baseline, &baseline)
        .is_empty());
    let mut wrong_owner = pre.clone();
    wrong_owner
        .iter_mut()
        .find(|a| a.label == "user-token-b")
        .unwrap()
        .account
        .owner = spl_token::PROGRAM_ID.into();
    assert_eq!(
        adapter.evaluable_subjects(&tx, &wrong_owner).len(),
        3,
        "Token-2022 cannot be decoded as SPL Token"
    );
    let mut malformed_candidate = baseline.clone();
    malformed_candidate
        .accounts
        .get_mut("user-token-b")
        .unwrap()
        .data
        .truncate(50);
    assert!(
        adapter
            .named_findings(&tx, &pre, &baseline, &malformed_candidate)
            .is_empty(),
        "malformed candidate amount is not interpreted as zero"
    );
}

#[test]
fn balance_evidence_drives_stable_directional_findings_not_logs() {
    let (tx, pre, baseline) = fixture();
    let adapter = OrcaSwapV2Adapter;
    let fingerprint = |candidate: &ExecutionResult| {
        adapter
            .named_findings(&tx, &pre, &baseline, candidate)
            .iter()
            .map(|finding| finding.fingerprint.to_string())
            .collect::<Vec<_>>()
    };
    let mut candidate = baseline.clone();
    mutate_amount(&mut candidate, "user-token-b", -1);
    assert_eq!(
        fingerprint(&candidate),
        ["orca-whirlpool/swap_v2/economic/user_input_spent/increased"]
    );
    let input_finding = adapter
        .named_findings(&tx, &pre, &baseline, &candidate)
        .remove(0);
    assert_eq!(
        input_finding.baseline,
        Some(eplyx_engine::semantics::SemanticValue::quantity(
            31_217_749, 8
        ))
    );
    mutate_amount(&mut candidate, "user-token-b", 2);
    assert_eq!(
        fingerprint(&candidate),
        ["orca-whirlpool/swap_v2/economic/user_input_spent/decreased"]
    );
    let mut candidate = baseline.clone();
    mutate_amount(&mut candidate, "vault-a", -1);
    assert_eq!(
        fingerprint(&candidate),
        ["orca-whirlpool/swap_v2/economic/vault_a_tokens_out/increased"]
    );
    let mut candidate = baseline.clone();
    mutate_amount(&mut candidate, "vault-b", -1);
    assert_eq!(
        fingerprint(&candidate),
        ["orca-whirlpool/swap_v2/economic/vault_b_tokens_in/decreased"]
    );
    let mut candidate = baseline.clone();
    candidate.success = false;
    assert_eq!(
        fingerprint(&candidate),
        ["orca-whirlpool/swap_v2/execution/transaction/now_reverts"]
    );
    let mut candidate = baseline.clone();
    candidate
        .logs
        .push("Program data: false swap event claims a different amount".into());
    assert!(
        fingerprint(&candidate).is_empty(),
        "logs cannot assert economic quantities"
    );
    mutate_amount(&mut candidate, "user-token-b", -1);
    assert_eq!(
        fingerprint(&candidate),
        ["orca-whirlpool/swap_v2/economic/user_input_spent/increased"],
        "account state wins when logs claim another amount"
    );
}

#[test]
fn reversing_direction_requires_the_other_user_account() {
    let (mut tx, mut pre, mut baseline) = fixture();
    let adapter = OrcaSwapV2Adapter;
    tx.instructions[4].data[41] = 1;
    assert!(
        adapter.evaluable_subjects(&tx, &pre).is_empty(),
        "the frozen companion creates A, so a reversed swap cannot claim transaction-level A input"
    );
    tx.instructions[3].program = "ComputeBudget111111111111111111111111111111".into();
    tx.instructions[3].accounts.clear();
    tx.instructions[5].program = "ComputeBudget111111111111111111111111111111".into();
    tx.instructions[5].accounts.clear();
    let uncovered = adapter.evaluable_subjects(&tx, &pre);
    assert_eq!(uncovered.len(), 3);
    assert!(!uncovered
        .iter()
        .any(|subject| subject.subject.as_str() == "user_input_spent"));
    assert!(adapter
        .named_findings(&tx, &pre, &baseline, &baseline)
        .is_empty());

    // Controlled complete A-to-B state: changing the direction makes A the
    // user's input, A the receiving vault, and B the sending vault.
    let mut user_a = pre
        .iter()
        .find(|a| a.label == "vault-a")
        .unwrap()
        .account
        .clone();
    let authority = bs58::decode(&tx.instructions[4].accounts[3].address)
        .into_vec()
        .unwrap();
    user_a.data[spl_token::ACCOUNT_OWNER..spl_token::ACCOUNT_OWNER + 32]
        .copy_from_slice(&authority);
    let initial_user_a = amount_of(&user_a);
    if let Some(existing) = pre.iter_mut().find(|a| a.label == "user-token-a") {
        existing.account = user_a.clone();
    } else {
        pre.push(NamedAccount {
            label: "user-token-a".into(),
            address: tx.instructions[4].accounts[7].address.clone(),
            account: user_a.clone(),
        });
    }
    set_amount(&mut user_a, initial_user_a - 10);
    baseline.accounts.insert("user-token-a".into(), user_a);
    let a_pre = amount_of(&pre.iter().find(|a| a.label == "vault-a").unwrap().account);
    let b_pre = amount_of(&pre.iter().find(|a| a.label == "vault-b").unwrap().account);
    set_amount(baseline.accounts.get_mut("vault-a").unwrap(), a_pre + 10);
    set_amount(baseline.accounts.get_mut("vault-b").unwrap(), b_pre - 20);
    let names = adapter
        .evaluable_subjects(&tx, &pre)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(names.contains(&"orca-whirlpool/swap_v2/economic/vault_a_tokens_in".to_string()));
    assert!(names.contains(&"orca-whirlpool/swap_v2/economic/vault_b_tokens_out".to_string()));
    let mut candidate = baseline.clone();
    mutate_amount(&mut candidate, "user-token-a", -1);
    assert_eq!(
        adapter
            .named_findings(&tx, &pre, &baseline, &candidate)
            .iter()
            .map(|finding| finding.fingerprint.to_string())
            .collect::<Vec<_>>(),
        ["orca-whirlpool/swap_v2/economic/user_input_spent/increased"]
    );
}

#[test]
fn replay_only_bundle_stays_adapter_free_after_registration() {
    let bundle = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u11-2-checkpointed-bundle");
    let report = ci::check(&bundle, &bundle.join("binaries/current.so"), None).unwrap();
    assert_eq!(report.bundle.adapter, "none");
    assert_eq!(report.replay_proof.as_ref().unwrap().status, "matched");
    assert!(report.coverage.is_empty());
    assert_eq!(report.exit_code(), 2);
}

#[test]
fn semantic_bundle_replays_with_bounded_coverage() {
    let bundle = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u12-orca-semantic-bundle");
    let report = ci::check(&bundle, &bundle.join("binaries/current.so"), None).unwrap();
    assert_eq!(report.bundle.adapter, "orca-whirlpool");
    assert_eq!(report.replay_proof.as_ref().unwrap().status, "matched");
    assert_eq!(
        report
            .replay_proof
            .as_ref()
            .unwrap()
            .proof_contract_versions,
        [2]
    );
    assert_eq!(report.coverage.len(), 4);
    assert!(report.findings.is_empty());
    assert!(report.failures.is_empty());
    assert_eq!(report.exit_code(), 0);
}
