//! Bounded semantics for the direct, 15-account Whirlpools SwapV2 shape.
//!
//! Role order and arguments follow the pinned Orca source at commit
//! 408c945fef4c49ab70def4303377cfaf8f0f3c99 (`instructions/v2/swap.rs`).
//! The adapter reads token balances through the shared standard-program
//! decoders. It makes no claim about ticks, pricing, fees, or transient output
//! accounts that are closed before the transaction boundary.

use anyhow::{ensure, Result};

use super::{ProtocolAdapter, SemanticAction, SemanticField};
use crate::{
    executor::ExecutionResult,
    ingest::transactions::HistoricalTransaction,
    semantics::{
        ActionId, ChangeKind, EvaluableSubject, FindingDomain, FindingFingerprint, NamedFinding,
        SemanticSubject, SemanticValue,
    },
    standard_programs::{spl_token, token2022, Decoded},
    types::{AccountSnapshot, InstructionSpec, NamedAccount},
};

pub const PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
// Anchor sha256("global:swap_v2")[..8].
const SWAP_V2: [u8; 8] = [0x2b, 0x04, 0xed, 0x0b, 0x1a, 0xc9, 0x1e, 0x62];
const ROLE_LABELS: [&str; 15] = [
    "token-program-a",
    "token-program-b",
    "memo-program",
    "token-authority",
    "whirlpool",
    "token-mint-a",
    "token-mint-b",
    "user-token-a",
    "vault-a",
    "user-token-b",
    "vault-b",
    "tick-array-0",
    "tick-array-1",
    "tick-array-2",
    "oracle",
];

pub struct OrcaSwapV2Adapter;

struct Swap<'a> {
    instruction: &'a InstructionSpec,
    a_to_b: bool,
}

fn swap(transaction: &HistoricalTransaction) -> Result<Swap<'_>> {
    ensure!(
        transaction.version == "v0" && transaction.success,
        "only successful native-v0 SwapV2 observations are supported"
    );
    let mut orca = transaction
        .instructions
        .iter()
        .filter(|instruction| instruction.program == PROGRAM_ID);
    let instruction = orca
        .next()
        .ok_or_else(|| anyhow::anyhow!("no direct Orca instruction"))?;
    ensure!(orca.next().is_none(), "multiple Orca instructions");
    ensure!(
        instruction.accounts.len() == ROLE_LABELS.len(),
        "SwapV2 requires exactly 15 roles"
    );
    let data = &instruction.data;
    ensure!(
        data.len() == 43 && data[..8] == SWAP_V2 && data[42] == 0,
        "unsupported SwapV2 discriminator or remaining accounts"
    );
    ensure!(
        data[40] == 1 && data[41] <= 1,
        "only exact-input SwapV2 is supported"
    );
    let account = |index: usize| &instruction.accounts[index];
    ensure!(
        account(0).address == spl_token::PROGRAM_ID && account(1).address == token2022::PROGRAM_ID,
        "SwapV2 requires the proven SPL Token A / Token-2022 B pair"
    );
    ensure!(
        account(2).address == MEMO_PROGRAM_ID && account(3).is_signer,
        "SwapV2 memo or authority role differs"
    );
    ensure!(
        [4, 7, 8, 9, 10, 11, 12, 13, 14]
            .iter()
            .all(|index| account(*index).is_writable),
        "SwapV2 writable role differs"
    );
    let distinct = instruction
        .accounts
        .iter()
        .map(|account| &account.address)
        .collect::<std::collections::BTreeSet<_>>();
    ensure!(distinct.len() == ROLE_LABELS.len(), "SwapV2 roles alias");
    // A transaction-level boundary can attribute these three net changes to
    // SwapV2 only when no companion outer instruction can write them.
    let input = if data[41] == 1 { 7 } else { 9 };
    let measured = [input, 8, 10].map(|role| &account(role).address);
    ensure!(
        transaction
            .instructions
            .iter()
            .filter(|other| !std::ptr::eq(*other, instruction))
            .all(|other| other
                .accounts
                .iter()
                .all(|meta| !meta.is_writable || !measured.contains(&&meta.address))),
        "companion instruction can write a measured SwapV2 account"
    );
    // u64 amount, u64 threshold and u128 price limit occupy bytes 8..40.
    // They bound execution; account-state deltas, not those stated values,
    // measure what the run actually transferred. Byte 40 is exact-in/out.
    Ok(Swap {
        instruction,
        a_to_b: data[41] == 1,
    })
}

fn token_amount(
    account: &AccountSnapshot,
    program: &str,
    mint: &str,
    authority: &str,
) -> Option<u64> {
    if account.owner != program {
        return None;
    }
    let decoded = match program {
        spl_token::PROGRAM_ID => spl_token::decode_account(&account.data),
        token2022::PROGRAM_ID => match token2022::decode_account(&account.data) {
            Decoded::Decoded(value) if value.extensions.truncated_at.is_none() => {
                Decoded::Decoded(value.base)
            }
            _ => return None,
        },
        _ => return None,
    };
    match decoded {
        Decoded::Decoded(value)
            if value.mint == mint
                && value.owner == authority
                && value.state == spl_token::AccountState::Initialized =>
        {
            Some(value.amount)
        }
        _ => None, // malformed/unsupported account never becomes a zero balance
    }
}

fn account<'a>(accounts: &'a [NamedAccount], label: &str) -> Option<&'a AccountSnapshot> {
    Some(
        &accounts
            .iter()
            .find(|account| account.label == label)?
            .account,
    )
}

fn amount(
    accounts: &[NamedAccount],
    result: Option<&ExecutionResult>,
    shape: &Swap<'_>,
    role: usize,
) -> Option<u64> {
    let label = ROLE_LABELS[role];
    let snapshot = match result {
        Some(result) => result.accounts.get(label)?,
        None => account(accounts, label)?,
    };
    let side = if role == 7 || role == 8 { 0 } else { 1 };
    let owner = if role == 7 || role == 9 { 3 } else { 4 };
    token_amount(
        snapshot,
        &shape.instruction.accounts[side].address,
        &shape.instruction.accounts[5 + side].address,
        &shape.instruction.accounts[owner].address,
    )
}

fn decimals(transaction: &HistoricalTransaction, shape: &Swap<'_>, role: usize) -> Option<u8> {
    let side = if role == 7 || role == 8 { 0 } else { 1 };
    let mint = &shape.instruction.accounts[5 + side].address;
    let program = &shape.instruction.accounts[side].address;
    let mut observed = transaction
        .pre_token_balances
        .as_ref()?
        .iter()
        .filter(|balance| &balance.mint == mint && &balance.program_id == program)
        .map(|balance| balance.decimals);
    let first = observed.next()?;
    observed.all(|value| value == first).then_some(first)
}

fn flows(
    accounts: &[NamedAccount],
    result: &ExecutionResult,
    shape: &Swap<'_>,
) -> [Option<u64>; 3] {
    if !result.success {
        return [None; 3];
    }
    let input = if shape.a_to_b { 7 } else { 9 };
    let pair = |role| {
        Some((
            amount(accounts, None, shape, role)?,
            amount(accounts, Some(result), shape, role)?,
        ))
    };
    let spent = pair(input).and_then(|(pre, post)| pre.checked_sub(post));
    let a_flow = pair(8).and_then(|(pre, post)| {
        if shape.a_to_b {
            post.checked_sub(pre)
        } else {
            pre.checked_sub(post)
        }
    });
    let b_flow = pair(10).and_then(|(pre, post)| {
        if shape.a_to_b {
            pre.checked_sub(post)
        } else {
            post.checked_sub(pre)
        }
    });
    [spent, a_flow, b_flow]
}

fn subject_names(a_to_b: bool) -> [&'static str; 3] {
    if a_to_b {
        [
            "user_input_spent",
            "vault_a_tokens_in",
            "vault_b_tokens_out",
        ]
    } else {
        [
            "user_input_spent",
            "vault_a_tokens_out",
            "vault_b_tokens_in",
        ]
    }
}

impl ProtocolAdapter for OrcaSwapV2Adapter {
    fn name(&self) -> &'static str {
        "orca-whirlpool"
    }
    fn program_id(&self) -> &'static str {
        PROGRAM_ID
    }
    fn supports_cpi(&self) -> bool {
        true
    }
    fn adapter_version(&self) -> u32 {
        1
    }
    fn semantic_action(&self, transaction: &HistoricalTransaction) -> SemanticAction {
        if swap(transaction).is_ok() {
            SemanticAction::Swap
        } else {
            SemanticAction::Unknown
        }
    }
    fn accept_instruction_contract(&self, transaction: &HistoricalTransaction) -> Result<()> {
        swap(transaction).map(|_| ())
    }
    fn label(&self, transaction: &HistoricalTransaction, index: usize) -> String {
        let Some(address) = transaction.account_keys.get(index).map(|key| &key.address) else {
            return format!("account-{index}");
        };
        swap(transaction)
            .ok()
            .and_then(|shape| {
                shape
                    .instruction
                    .accounts
                    .iter()
                    .position(|role| &role.address == address)
            })
            .map(|role| ROLE_LABELS[role].to_string())
            .unwrap_or_else(|| format!("account-{index}"))
    }
    fn decode(&self, _account: &AccountSnapshot) -> Option<super::SemanticAccount> {
        None
    }
    fn prove_boundaries(
        &self,
        _transaction: &HistoricalTransaction,
        _pre: &[NamedAccount],
        _post: &[NamedAccount],
    ) -> Result<Vec<String>> {
        anyhow::bail!("direct SwapV2 requires the checkpointed universal boundary proof")
    }
    fn interpret(
        &self,
        _accounts: &[NamedAccount],
        _v1: &ExecutionResult,
        _v2: &ExecutionResult,
    ) -> Vec<super::EconomicChange> {
        Vec::new()
    }
    fn action_id(&self, transaction: &HistoricalTransaction) -> Option<ActionId> {
        swap(transaction).ok()?;
        ActionId::new("swap_v2").ok()
    }
    fn evaluable_subjects(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<EvaluableSubject> {
        let Ok(shape) = swap(transaction) else {
            return Vec::new();
        };
        let (Some(protocol), Some(action)) = (self.protocol_id(), self.action_id(transaction))
        else {
            return Vec::new();
        };
        let make = |domain, name: &str| EvaluableSubject {
            protocol: protocol.clone(),
            action: action.clone(),
            domain,
            subject: SemanticSubject::new(name).expect("static subject"),
        };
        let mut subjects = vec![make(FindingDomain::Execution, "transaction")];
        let input = if shape.a_to_b { 7 } else { 9 };
        for (role, name) in [input, 8, 10].into_iter().zip(subject_names(shape.a_to_b)) {
            if amount(accounts, None, &shape, role).is_some()
                && decimals(transaction, &shape, role).is_some()
            {
                subjects.push(make(FindingDomain::Economic, name));
            }
        }
        subjects
    }
    fn summarize(&self, accounts: &[NamedAccount], result: &ExecutionResult) -> Vec<SemanticField> {
        // The transaction is unavailable at this older trait method. Named
        // findings use the transaction-aware flow calculation below.
        let _ = (accounts, result);
        Vec::new()
    }
    fn named_findings(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
        v1: &ExecutionResult,
        v2: &ExecutionResult,
    ) -> Vec<NamedFinding> {
        let Ok(shape) = swap(transaction) else {
            return Vec::new();
        };
        let (Some(protocol), Some(action)) = (self.protocol_id(), self.action_id(transaction))
        else {
            return Vec::new();
        };
        let fingerprint = |domain, name: &str, change| FindingFingerprint {
            protocol: protocol.clone(),
            action: action.clone(),
            domain,
            subject: SemanticSubject::new(name).expect("static subject"),
            change,
        };
        if v1.success != v2.success {
            return vec![NamedFinding {
                fingerprint: fingerprint(
                    FindingDomain::Execution,
                    "transaction",
                    if v1.success {
                        ChangeKind::NowReverts
                    } else {
                        ChangeKind::NowSucceeds
                    },
                ),
                baseline: None,
                candidate: None,
                relative_delta_bps: None,
                severity: crate::diff::Severity::Critical,
            }];
        }
        let before = flows(accounts, v1, &shape);
        let after = flows(accounts, v2, &shape);
        subject_names(shape.a_to_b)
            .iter()
            .enumerate()
            .filter_map(|(index, name)| {
                let (Some(prior), Some(next)) = (before[index], after[index]) else {
                    return None;
                };
                let role = [if shape.a_to_b { 7 } else { 9 }, 8, 10][index];
                let scale = decimals(transaction, &shape, role)?;
                if prior == next {
                    return None;
                }
                Some(NamedFinding {
                    fingerprint: fingerprint(
                        FindingDomain::Economic,
                        name,
                        ChangeKind::from_delta(i128::from(next) - i128::from(prior)),
                    ),
                    baseline: Some(SemanticValue::quantity(prior, scale)),
                    candidate: Some(SemanticValue::quantity(next, scale)),
                    relative_delta_bps: None,
                    severity: crate::diff::Severity::High,
                })
            })
            .collect()
    }
}
