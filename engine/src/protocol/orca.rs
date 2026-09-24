//! Bounded semantics for the direct, 15-account Whirlpools SwapV2 shape.
//!
//! Role order and arguments follow the pinned Orca source at commit
//! 408c945fef4c49ab70def4303377cfaf8f0f3c99 (`instructions/v2/swap.rs`).
//! The adapter reads token balances through the shared standard-program
//! decoders. It makes no claim about ticks, pricing, fees, or transient output
//! accounts that are closed before the transaction boundary.

use anyhow::{ensure, Result};

use super::{
    ProtocolAdapter, SemanticAction, SemanticEvaluation, SemanticEvaluationContext, SemanticField,
};
use crate::{
    executor::ExecutionResult,
    ingest::transactions::HistoricalTransaction,
    semantic_binding::{CorroboratedFact, RepositoryInterface, SemanticBinding, SourceBlob},
    semantics::{
        ActionId, ChangeKind, EvaluableSubject, FindingDomain, FindingFingerprint, NamedFinding,
        SemanticSubject, SemanticValue,
    },
    standard_programs::{spl_token, token2022, Decoded},
    types::{AccountSnapshot, InstructionSpec, NamedAccount},
};
use std::collections::BTreeMap;

pub const PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
// Anchor sha256("global:swap_v2")[..8].
const SWAP_V2: [u8; 8] = [0x2b, 0x04, 0xed, 0x0b, 0x1a, 0xc9, 0x1e, 0x62];
const WHIRLPOOL_ACCOUNT: [u8; 8] = [0x3f, 0x95, 0xd1, 0x0c, 0xe1, 0x80, 0x63, 0x09];
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

fn source_interface() -> RepositoryInterface {
    RepositoryInterface {
        repository: "https://github.com/orca-so/whirlpools".into(),
        commit: "408c945fef4c49ab70def4303377cfaf8f0f3c99".into(),
        source_blobs: vec![
            SourceBlob {
                path: "programs/whirlpool/src/instructions/v2/swap.rs".into(),
                git_blob_sha1: "2284b67082c0e1e60bd67b2f96065a627066292a".into(),
            },
            SourceBlob {
                path: "programs/whirlpool/src/state/whirlpool.rs".into(),
                git_blob_sha1: "bc02ce0140434ae3ef0414ae72ad5a33eb451e01".into(),
            },
        ],
    }
}

fn pool_address_at(data: &[u8], offset: usize) -> Option<String> {
    let bytes: [u8; 32] = data.get(offset..offset + 32)?.try_into().ok()?;
    Some(solana_address::Address::new_from_array(bytes).to_string())
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
    fn semantic_binding(
        &self,
        transaction: &HistoricalTransaction,
        pre: &BTreeMap<String, AccountSnapshot>,
        baseline_elf: &[u8],
        baseline: &crate::universal::execution::ExecutionEvidence,
    ) -> Result<SemanticBinding> {
        let shape = match swap(transaction) {
            Ok(shape) => shape,
            Err(_) => return Ok(SemanticBinding::ManualOrUnknown),
        };
        let source = source_interface();
        // The retained successful transaction is B -> A. An opposite-direction
        // shape may share the repository interface, but this one execution does
        // not independently corroborate its economic role mapping.
        if shape.a_to_b {
            return Ok(SemanticBinding::RepositorySourceClaim { source });
        }
        let role = |index: usize| &shape.instruction.accounts[index].address;
        let pool = pre
            .get(role(4))
            .ok_or_else(|| anyhow::anyhow!("missing historical Whirlpool state"))?;
        ensure!(
            pool.owner == PROGRAM_ID
                && pool.data.len() == 653
                && pool.data[..8] == WHIRLPOOL_ACCOUNT,
            "historical Whirlpool state does not match the claimed account layout"
        );
        // Offsets include the 8-byte Anchor account discriminator. These four
        // fields are from the pinned source layout, not a verified ELF build.
        for (offset, index) in [(101, 5), (133, 8), (181, 6), (213, 10)] {
            ensure!(
                pool_address_at(&pool.data, offset).as_deref() == Some(role(index)),
                "historical Whirlpool mint/vault relationship differs"
            );
        }
        let checked = |index: usize,
                       program: usize,
                       mint: usize,
                       authority: usize|
         -> Result<(u64, u64)> {
            let address = role(index);
            let start = pre
                .get(address)
                .ok_or_else(|| anyhow::anyhow!("missing measured token pre-state"))?;
            let end = baseline
                .post_accounts
                .get(address)
                .and_then(Option::as_ref)
                .ok_or_else(|| anyhow::anyhow!("missing measured token post-state"))?;
            let read = |account| {
                token_amount(account, role(program), role(mint), role(authority)).ok_or_else(|| {
                    anyhow::anyhow!("historical token role, owner, mint or authority differs")
                })
            };
            Ok((read(start)?, read(end)?))
        };
        let (user_b_pre, user_b_post) = checked(9, 1, 6, 3)?;
        let (vault_a_pre, vault_a_post) = checked(8, 0, 5, 4)?;
        let (vault_b_pre, vault_b_post) = checked(10, 1, 6, 4)?;
        ensure!(
            baseline.success
                && user_b_pre > user_b_post
                && vault_a_pre > vault_a_post
                && vault_b_post > vault_b_pre,
            "historical execution does not corroborate B-to-A token flow"
        );
        Ok(SemanticBinding::ExecutionCorroboratedExternalInterface {
            source,
            facts: vec![
                CorroboratedFact::InstructionShape,
                CorroboratedFact::SignerRole,
                CorroboratedFact::TokenProgramOwnership,
                CorroboratedFact::MintIdentity,
                CorroboratedFact::TokenAccountAuthority,
                CorroboratedFact::PoolMintVaultRelationship,
                CorroboratedFact::ObservedFlowDirection,
                CorroboratedFact::BaselineExecutionSuccess,
            ],
            historical_elf_sha256: crate::replay::hash_bytes(baseline_elf),
            execution_evidence_sha256: crate::universal::pipeline::evidence_hash(baseline)?,
        })
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
    fn evaluate_semantics(
        &self,
        context: &SemanticEvaluationContext<'_>,
    ) -> Result<SemanticEvaluation> {
        let SemanticEvaluationContext {
            transaction,
            pre,
            baseline,
            candidate,
        } = context;
        let Ok(shape) = swap(transaction) else {
            return Ok(SemanticEvaluation::Unsupported);
        };
        if !baseline.success {
            return Ok(SemanticEvaluation::Unevaluable {
                reason: "invalid Orca baseline execution".into(),
            });
        }
        let roles = [if shape.a_to_b { 7 } else { 9 }, 8, 10];
        let scales = roles.map(|role| decimals(transaction, &shape, role));
        let before = flows(pre, baseline, &shape);
        if before.iter().any(Option::is_none) || scales.iter().any(Option::is_none) {
            return Ok(SemanticEvaluation::Unevaluable {
                reason: "invalid Orca baseline token state or flow".into(),
            });
        }
        let protocol = self
            .protocol_id()
            .ok_or_else(|| anyhow::anyhow!("missing Orca protocol id"))?;
        let action = self
            .action_id(transaction)
            .ok_or_else(|| anyhow::anyhow!("missing Orca action id"))?;
        let subject = |domain, name: &str| EvaluableSubject {
            protocol: protocol.clone(),
            action: action.clone(),
            domain,
            subject: SemanticSubject::new(name).expect("static subject"),
        };
        let mut subjects = vec![subject(FindingDomain::Execution, "transaction")];
        subjects.extend(
            subject_names(shape.a_to_b)
                .into_iter()
                .map(|name| subject(FindingDomain::Economic, name)),
        );
        let fingerprint = |domain, name: &str, change| FindingFingerprint {
            protocol: subjects[0].protocol.clone(),
            action: subjects[0].action.clone(),
            domain,
            subject: SemanticSubject::new(name).expect("static subject"),
            change,
        };
        if !candidate.success {
            return Ok(SemanticEvaluation::Evaluated {
                subjects: vec![subjects[0].clone()],
                findings: vec![NamedFinding {
                    fingerprint: fingerprint(
                        FindingDomain::Execution,
                        "transaction",
                        ChangeKind::NowReverts,
                    ),
                    baseline: None,
                    candidate: None,
                    relative_delta_bps: None,
                    severity: crate::diff::Severity::Critical,
                }],
                explained: Vec::new(),
            });
        }
        let after = flows(pre, candidate, &shape);
        if after.iter().any(Option::is_none) {
            return Ok(SemanticEvaluation::Unevaluable {
                reason: "invalid Orca candidate token state or flow".into(),
            });
        }
        let mut findings = Vec::new();
        for (index, name) in subject_names(shape.a_to_b).into_iter().enumerate() {
            let prior = before[index].ok_or_else(|| anyhow::anyhow!("Orca baseline flow lost"))?;
            let next = after[index].ok_or_else(|| anyhow::anyhow!("Orca candidate flow lost"))?;
            let scale = scales[index].ok_or_else(|| anyhow::anyhow!("Orca decimals lost"))?;
            if prior != next {
                findings.push(NamedFinding {
                    fingerprint: fingerprint(
                        FindingDomain::Economic,
                        name,
                        ChangeKind::from_delta(i128::from(next) - i128::from(prior)),
                    ),
                    baseline: Some(SemanticValue::quantity(prior, scale)),
                    candidate: Some(SemanticValue::quantity(next, scale)),
                    relative_delta_bps: None,
                    severity: crate::diff::Severity::High,
                });
            }
        }
        Ok(SemanticEvaluation::Evaluated {
            subjects,
            findings,
            explained: Vec::new(),
        })
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
        match self.evaluate_semantics(&SemanticEvaluationContext {
            transaction,
            pre: accounts,
            baseline: v1,
            candidate: v2,
        }) {
            Ok(SemanticEvaluation::Evaluated { findings, .. }) => findings,
            _ => Vec::new(),
        }
    }
}
