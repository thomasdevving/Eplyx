//! Settlement semantics for the one frozen U13.3 direct Drift transaction.
//!
//! Layout: pinned IDL `sdk/src/idl/drift.json` at 73d22383e621040cd11b94375b6bd2f728f7537e.
//! Arithmetic: pinned `math/spot_balance.rs`, `controller/spot_balance.rs`,
//! `controller/position.rs`, `controller/amm.rs`, and `controller/pnl.rs`.
//! This adapter does not infer semantics from arbitrary Drift-owned bytes.

use std::{collections::BTreeMap, ops::Range};

use anyhow::{ensure, Result};

use super::{EconomicChange, ProtocolAdapter, SemanticAccount};
use crate::{
    executor::ExecutionResult,
    ingest::transactions::HistoricalTransaction,
    semantic_binding::{CorroboratedFact, RepositoryInterface, SemanticBinding, SourceBlob},
    semantics::{
        ActionId, ChangeKind, EvaluableSubject, FindingDomain, FindingFingerprint, NamedFinding,
        SemanticSubject, SemanticValue,
    },
    types::{AccountSnapshot, NamedAccount},
    universal::execution::ExecutionEvidence,
};

pub const PROGRAM_ID: &str = "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH";
const SLOT: u64 = 409_942_000;
const SIGNATURE: &str =
    "2BD3UJFPUPJbxoJMAntwyZrzLjzKv3yeuQERTu2rPRZLf4H8Wdjihsh6xChwC4Vxruxy9qyrrknmnvi7UJqCHRSK";
const COMPUTE_BUDGET: &str = "ComputeBudget111111111111111111111111111111";
const INSTRUCTION: [u8; 10] = [0x2b, 0x3d, 0xea, 0x2d, 0x0f, 0x5f, 0x98, 0x99, 3, 0];
const ROLES: [&str; 14] = [
    "5zpq7DvB6UdFFvpmBPspGPNfUGoBRRCE2HHg5u3gxcsN", // State
    "JE9m89yHHiCGzzL2FAeeZgHKAFwjkW4Qp1GfjegWnojR", // User
    "maCYbwXrJDnnP5ft3ySoH4ogwv5yFph8fFacgMa51de",  // signer
    "GXWqPpjQpdz7KZw9p7f5PX2eGxHAhvpNXiviFkAB8zXg", // quote vault
    "9VCioxmni2gDLv11qufWzT3RDERhQE4iY5Gf7NTfYyAV",
    "3m6i4RFWEDw2Ft4tFHPJtYgmpPe21k56M3FHeWYrgGBz",
    "HN7qfUNM5Q7gQTwyEucmYdCF4CjwUrspj3DbNQ4V8P52",
    "CXZhzKePYajrZgZyrzgvHYFKK3c5tNgDrRobAgySo8Nb",
    "93FG52TzNKCnMiasV14Ba34BYcHDb9p4zK4GjZnLwqWR",
    "6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3", // quote SpotMarket
    "3x85u7SWkmmr7YQGYhtjARgxwegTLJgkSLRprfXod6rh",
    "HpR1bLcW6rsXrBigRhW18WnQNwAVBq7wBPhteRKGBU5z",
    "7QAtMC3AaAc91W4XuwYXM1Mtffq9h9Z8dTxcJrKRHu1z", // PerpMarket 3
    "25Eax9W8SA3wpCQFhJEGyHhQ2NDHEshZEDzyMNtthR8D",
];

// Offsets include the eight-byte Anchor discriminator. The account sizes and
// fixed array slots are from the retained IDL, not guessed from changed bytes.
const USER_DISC: [u8; 8] = [0x9f, 0x75, 0x5f, 0xe3, 0xef, 0x97, 0x3a, 0xec];
const PERP_DISC: [u8; 8] = [0x0a, 0xdf, 0x0c, 0x2c, 0x6b, 0xf5, 0x37, 0xf7];
const SPOT_DISC: [u8; 8] = [0x64, 0xb1, 0x08, 0x6b, 0xa8, 0x41, 0x41, 0x27];
const USER_LEN: usize = 4376;
const PERP_LEN: usize = 1216;
const SPOT_LEN: usize = 776;
const USER_SPOT_SCALED: usize = 104; // spotPositions[0].scaledBalance
const USER_PERP_QUOTE: usize = 536; // perpPositions[1].quoteAssetAmount
const USER_PERP_SETTLED: usize = 576; // perpPositions[1].settledPnl
const USER_TOTAL_SETTLED: usize = 4296;
const PERP_AMM_QUOTE: usize = 384;
const PERP_POOL_SCALED: usize = 976;
const PERP_USERS: usize = 1156;
const SPOT_REVENUE_SCALED: usize = 256;
const SPOT_DEPOSIT_SCALED: usize = 432;
const SPOT_DEPOSIT_INTEREST: usize = 464;
const SPOT_BORROW_SCALED: usize = 448;

pub struct DriftSettlePnlAdapter;

fn shape(tx: &HistoricalTransaction) -> Result<()> {
    ensure!(
        tx.signature == SIGNATURE && tx.slot == SLOT && tx.version == "v0" && tx.success,
        "only the frozen successful direct settlePnl witness is supported"
    );
    ensure!(
        tx.loaded_address_count == 0 && tx.instructions.len() == 3,
        "settlePnl message shape differs"
    );
    ensure!(
        tx.instructions[..2]
            .iter()
            .all(|ix| ix.program == COMPUTE_BUDGET),
        "settlePnl companion instructions differ"
    );
    let ix = &tx.instructions[2];
    ensure!(
        ix.program == PROGRAM_ID && ix.data == INSTRUCTION && ix.accounts.len() == ROLES.len(),
        "settlePnl instruction differs"
    );
    ensure!(
        ix.accounts
            .iter()
            .zip(ROLES)
            .all(|(meta, address)| meta.address == address),
        "settlePnl account roles differ"
    );
    ensure!(
        !ix.accounts[0].is_writable
            && ix.accounts[1].is_writable
            && ix.accounts[2].is_signer
            && !ix.accounts[3].is_writable
            && ix.accounts[9].is_writable
            && ix.accounts[12].is_writable,
        "settlePnl access roles differ"
    );
    ensure!(
        tx.inner_instructions.is_empty()
            && tx.pre_token_balances.as_ref().is_some_and(Vec::is_empty)
            && tx.post_token_balances.as_ref().is_some_and(Vec::is_empty),
        "settlePnl has unsupported CPI or token flow"
    );
    Ok(())
}

fn checked(account: &AccountSnapshot, len: usize, disc: [u8; 8]) -> Option<&[u8]> {
    (account.owner == PROGRAM_ID && account.data.len() == len && account.data[..8] == disc)
        .then_some(account.data.as_slice())
}
fn bytes<const N: usize>(data: &[u8], offset: usize) -> Option<[u8; N]> {
    data.get(offset..offset + N)?.try_into().ok()
}
fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes(data, offset)?))
}
fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes(data, offset)?))
}
fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes(data, offset)?))
}
fn i64_at(data: &[u8], offset: usize) -> Option<i64> {
    Some(i64::from_le_bytes(bytes(data, offset)?))
}
fn u128_at(data: &[u8], offset: usize) -> Option<u128> {
    Some(u128::from_le_bytes(bytes(data, offset)?))
}
fn i128_at(data: &[u8], offset: usize) -> Option<i128> {
    Some(i128::from_le_bytes(bytes(data, offset)?))
}
fn address_at(data: &[u8], offset: usize) -> Option<String> {
    Some(solana_address::Address::new_from_array(bytes(data, offset)?).to_string())
}

#[derive(Clone, Copy)]
struct UserState {
    scaled: u64,
    quote: i64,
    settled: i64,
    total_settled: i64,
}
#[derive(Clone, Copy)]
struct PerpState {
    amm_quote: i128,
    pool: u128,
    users: u32,
}
#[derive(Clone, Copy)]
struct SpotState {
    revenue: u128,
    deposits: u128,
    borrows: u128,
    interest: u128,
}

fn user(account: &AccountSnapshot) -> Option<UserState> {
    let d = checked(account, USER_LEN, USER_DISC)?;
    // The frozen quote deposit is slot 0; the market-3 perp position is slot 1.
    if u16_at(d, 136)? != 0
        || d[138] != 0
        || u16_at(d, 612)? != 3
        || i64_at(d, 528)? != 0
        || u64_at(d, 584)? != 0
        || u64_at(d, 592)? != 0
        || bytes::<32>(d, 8)? == [0; 32]
    {
        return None;
    }
    Some(UserState {
        scaled: u64_at(d, USER_SPOT_SCALED)?,
        quote: i64_at(d, USER_PERP_QUOTE)?,
        settled: i64_at(d, USER_PERP_SETTLED)?,
        total_settled: i64_at(d, USER_TOTAL_SETTLED)?,
    })
}

fn perp(account: &AccountSnapshot) -> Option<PerpState> {
    let d = checked(account, PERP_LEN, PERP_DISC)?;
    if address_at(d, 8)? != ROLES[12]
        || u16_at(d, 1160)? != 3
        || u16_at(d, 1166)? != 0
        || u16_at(d, 992)? != 0
    {
        return None;
    }
    Some(PerpState {
        amm_quote: i128_at(d, PERP_AMM_QUOTE)?,
        pool: u128_at(d, PERP_POOL_SCALED)?,
        users: u32_at(d, PERP_USERS)?,
    })
}

fn spot(account: &AccountSnapshot) -> Option<SpotState> {
    let d = checked(account, SPOT_LEN, SPOT_DISC)?;
    if address_at(d, 8)? != ROLES[9]
        || address_at(d, 104)? != ROLES[3]
        || u32_at(d, 680)? != 6
        || u16_at(d, 684)? != 0
        || u16_at(d, 272)? != 0
    {
        return None;
    }
    Some(SpotState {
        revenue: u128_at(d, SPOT_REVENUE_SCALED)?,
        deposits: u128_at(d, SPOT_DEPOSIT_SCALED)?,
        borrows: u128_at(d, SPOT_BORROW_SCALED)?,
        interest: u128_at(d, SPOT_DEPOSIT_INTEREST)?,
    })
}

fn pre_account<'a>(
    accounts: &'a [NamedAccount],
    label: &str,
    address: &str,
) -> Option<&'a AccountSnapshot> {
    let item = accounts
        .iter()
        .find(|item| item.label == label && item.address == address)?;
    Some(&item.account)
}
fn post_account<'a>(result: &'a ExecutionResult, label: &str) -> Option<&'a AccountSnapshot> {
    result.accounts.get(label)
}

// get_token_amount(Deposit) from pinned math/spot_balance.rs: floor(scaled *
// interest / 10^(19 - decimals)). The quote market has six decimals, so the
// denominator is 10^13. Both balances use target-post interest to exclude
// the interest update that precedes settlement in the same transaction.
fn quote_tokens(scaled: u64, interest: u128) -> Option<i128> {
    let tokens = u128::from(scaled).checked_mul(interest)? / 10_000_000_000_000;
    i128::try_from(tokens).ok()
}

fn evaluate(accounts: &[NamedAccount], result: &ExecutionResult) -> Option<i64> {
    if !result.success {
        return None;
    }
    let pre_user = user(pre_account(accounts, "user", ROLES[1])?)?;
    let post_user = user(post_account(result, "user")?)?;
    let pre_perp = perp(pre_account(accounts, "perp-market", ROLES[12])?)?;
    let post_perp = perp(post_account(result, "perp-market")?)?;
    let pre_spot = spot(pre_account(accounts, "quote-spot-market", ROLES[9])?)?;
    let post_spot = spot(post_account(result, "quote-spot-market")?)?;
    if post_spot.interest == 0
        || pre_spot.interest == 0
        || post_spot.borrows != pre_spot.borrows
        || post_spot.deposits.checked_sub(pre_spot.deposits)
            != post_spot.revenue.checked_sub(pre_spot.revenue)
    {
        return None;
    }
    let settled = quote_tokens(post_user.scaled, post_spot.interest)?
        .checked_sub(quote_tokens(pre_user.scaled, post_spot.interest)?)?;
    let settled_i64 = i64::try_from(settled).ok()?;
    let scaled_delta = i128::from(post_user.scaled) - i128::from(pre_user.scaled);
    let expected_scaled =
        settled.unsigned_abs().checked_mul(10_000_000_000_000)? / post_spot.interest;
    if scaled_delta.unsigned_abs() != expected_scaled
        || i128::try_from(post_perp.pool)
            .ok()?
            .checked_sub(i128::try_from(pre_perp.pool).ok()?)?
            != -scaled_delta
        || i128::from(post_user.settled) - i128::from(pre_user.settled) != settled
        || i128::from(post_user.total_settled) - i128::from(pre_user.total_settled) != settled
        || i128::from(pre_user.quote) - i128::from(post_user.quote) != settled
        || pre_perp.amm_quote.checked_sub(post_perp.amm_quote)? != settled
    {
        return None;
    }
    let expected_users = i64::from(pre_perp.users) + i64::from(pre_user.quote == 0)
        - i64::from(post_user.quote == 0);
    if i64::from(post_perp.users) != expected_users {
        return None;
    }
    Some(settled_i64)
}

fn source_interface() -> RepositoryInterface {
    RepositoryInterface {
        repository: "https://github.com/velocity-exchange/protocol-v2".into(),
        commit: "73d22383e621040cd11b94375b6bd2f728f7537e".into(),
        source_blobs: [
            (
                "programs/drift/src/lib.rs",
                "1862893e79b9e34f7ee2f3df08e5a2ccd641eedd",
            ),
            (
                "sdk/src/idl/drift.json",
                "7232b2925ffcee1f65ad85a0fddbf830add64f12",
            ),
            (
                "programs/drift/src/controller/pnl.rs",
                "3aae2e82a40cf274c9f9ac519f3efb6a99c38275",
            ),
            (
                "programs/drift/src/controller/amm.rs",
                "ea334200c138481f36a77c93058d5831ba2da4a4",
            ),
            (
                "programs/drift/src/controller/position.rs",
                "6b6ec530d529718ffd3d3726f62596d226133492",
            ),
            (
                "programs/drift/src/controller/spot_balance.rs",
                "f8e1b625e91629e7ab1842b72a67809c8c70b49b",
            ),
            (
                "programs/drift/src/math/spot_balance.rs",
                "da5261a48708e1c96c79864c0ec4fa37aea21592",
            ),
            (
                "programs/drift/src/math/constants.rs",
                "ad91a592a2e0b8deffe2cf85122eaecc73444611",
            ),
        ]
        .into_iter()
        .map(|(path, git_blob_sha1)| SourceBlob {
            path: path.into(),
            git_blob_sha1: git_blob_sha1.into(),
        })
        .collect(),
    }
}

impl ProtocolAdapter for DriftSettlePnlAdapter {
    fn name(&self) -> &'static str {
        "drift-settle-pnl"
    }
    fn program_id(&self) -> &'static str {
        PROGRAM_ID
    }
    fn adapter_version(&self) -> u32 {
        1
    }
    fn accept_instruction_contract(&self, transaction: &HistoricalTransaction) -> Result<()> {
        shape(transaction)
    }
    fn label(&self, tx: &HistoricalTransaction, index: usize) -> String {
        let address = tx.account_keys.get(index).map(|key| key.address.as_str());
        match address {
            Some(value) if value == ROLES[1] => "user".into(),
            Some(value) if value == ROLES[9] => "quote-spot-market".into(),
            Some(value) if value == ROLES[12] => "perp-market".into(),
            _ => format!("account-{index}"),
        }
    }
    fn decode(&self, _account: &AccountSnapshot) -> Option<SemanticAccount> {
        None
    }
    fn prove_boundaries(
        &self,
        _tx: &HistoricalTransaction,
        _pre: &[NamedAccount],
        _post: &[NamedAccount],
    ) -> Result<Vec<String>> {
        anyhow::bail!("settlePnl requires the universal contract-3 derived target boundary")
    }
    fn interpret(
        &self,
        _accounts: &[NamedAccount],
        _v1: &ExecutionResult,
        _v2: &ExecutionResult,
    ) -> Vec<EconomicChange> {
        Vec::new()
    }
    fn action_id(&self, tx: &HistoricalTransaction) -> Option<ActionId> {
        shape(tx).ok()?;
        ActionId::new("settle_pnl").ok()
    }
    fn semantic_binding(
        &self,
        tx: &HistoricalTransaction,
        pre: &BTreeMap<String, AccountSnapshot>,
        baseline_elf: &[u8],
        baseline: &ExecutionEvidence,
    ) -> Result<SemanticBinding> {
        if shape(tx).is_err() {
            return Ok(SemanticBinding::ManualOrUnknown);
        }
        let source = source_interface();
        let accounts = [
            ("user", ROLES[1]),
            ("perp-market", ROLES[12]),
            ("quote-spot-market", ROLES[9]),
        ]
        .into_iter()
        .map(|(label, address)| {
            Ok(NamedAccount {
                label: label.into(),
                address: address.into(),
                account: pre
                    .get(address)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("missing {label} pre-state"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
        let post = baseline
            .post_accounts
            .iter()
            .filter_map(|(address, value)| {
                let label = match address.as_str() {
                    value if value == ROLES[1] => "user",
                    value if value == ROLES[12] => "perp-market",
                    value if value == ROLES[9] => "quote-spot-market",
                    _ => return None,
                };
                Some((label.into(), value.clone()?))
            })
            .collect::<BTreeMap<String, AccountSnapshot>>();
        let result = ExecutionResult {
            version: "semantic-binding".into(),
            success: baseline.success,
            error: baseline.error.clone(),
            compute_units: Some(baseline.compute_units),
            fee: baseline.fee,
            logs: Vec::new(),
            cpi_calls: Vec::new(),
            accounts: post,
        };
        ensure!(
            evaluate(&accounts, &result).is_some_and(|amount| amount > 0),
            "historical state does not corroborate positive internal settlement"
        );
        Ok(SemanticBinding::ExecutionCorroboratedExternalInterface {
            source,
            facts: vec![
                CorroboratedFact::InstructionShape,
                CorroboratedFact::SignerRole,
                CorroboratedFact::BaselineExecutionSuccess,
                CorroboratedFact::AnchorAccountDiscriminator,
                CorroboratedFact::MarketIndexLinkage,
                CorroboratedFact::AccountRoleRelationship,
                CorroboratedFact::InternalAccountingDirection,
            ],
            historical_elf_sha256: crate::replay::hash_bytes(baseline_elf),
            execution_evidence_sha256: crate::universal::pipeline::evidence_hash(baseline)?,
        })
    }
    fn evaluable_subjects(
        &self,
        tx: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<EvaluableSubject> {
        if shape(tx).is_err() {
            return Vec::new();
        }
        let (Some(protocol), Some(action)) = (self.protocol_id(), self.action_id(tx)) else {
            return Vec::new();
        };
        let subject = |domain, name: &str| EvaluableSubject {
            protocol: protocol.clone(),
            action: action.clone(),
            domain,
            subject: SemanticSubject::new(name).expect("static subject"),
        };
        let mut out = vec![subject(FindingDomain::Execution, "transaction")];
        if pre_account(accounts, "user", ROLES[1])
            .and_then(user)
            .is_some()
            && pre_account(accounts, "perp-market", ROLES[12])
                .and_then(perp)
                .is_some()
            && pre_account(accounts, "quote-spot-market", ROLES[9])
                .and_then(spot)
                .is_some()
        {
            out.push(subject(FindingDomain::Economic, "pnl_settled"));
        }
        out
    }
    fn decoded_sources_of(&self, subject: &str) -> &'static [(&'static str, &'static str)] {
        if subject == "pnl_settled" {
            &[("user", "settlement"), ("perp-market", "settlement")]
        } else {
            &[]
        }
    }
    fn decoded_byte_ranges(&self, label: &str) -> &'static [Range<usize>] {
        match label {
            "user" => &[104..112, 536..544, 576..584, 4296..4304],
            "perp-market" => &[384..400, 976..992, 1156..1160],
            _ => &[],
        }
    }
    fn named_findings(
        &self,
        tx: &HistoricalTransaction,
        accounts: &[NamedAccount],
        baseline: &ExecutionResult,
        candidate: &ExecutionResult,
    ) -> Vec<NamedFinding> {
        if shape(tx).is_err() {
            return Vec::new();
        }
        let (Some(protocol), Some(action)) = (self.protocol_id(), self.action_id(tx)) else {
            return Vec::new();
        };
        let fingerprint = |domain, subject: &str, change| FindingFingerprint {
            protocol: protocol.clone(),
            action: action.clone(),
            domain,
            subject: SemanticSubject::new(subject).expect("static subject"),
            change,
        };
        if baseline.success && !candidate.success {
            return vec![NamedFinding {
                fingerprint: fingerprint(
                    FindingDomain::Execution,
                    "transaction",
                    ChangeKind::NowReverts,
                ),
                baseline: None,
                candidate: None,
                relative_delta_bps: None,
                severity: crate::diff::Severity::Critical,
            }];
        }
        let (Some(before), Some(after)) =
            (evaluate(accounts, baseline), evaluate(accounts, candidate))
        else {
            return Vec::new(); // malformed state remains structural/undeclarable
        };
        if before == after {
            return Vec::new();
        }
        vec![NamedFinding {
            fingerprint: fingerprint(
                FindingDomain::Economic,
                "pnl_settled",
                ChangeKind::from_delta(i128::from(after) - i128::from(before)),
            ),
            baseline: Some(SemanticValue::signed_quantity(i128::from(before), 6)),
            candidate: Some(SemanticValue::signed_quantity(i128::from(after), 6)),
            relative_delta_bps: None,
            severity: crate::diff::Severity::High,
        }]
    }
}
