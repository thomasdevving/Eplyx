//! Historical DepositSol, with proposed state produced only by deployed SetFee.
use super::{binding, failed, Failure, Operation, Status, KIND, REPORT_SCHEMA};
use crate::{
    bundle::CiBundle,
    change::ChangeSpec,
    executor::{self, ExecutionResult, LoadedProgram, ProbeTransactionExecution, ProgramVersion},
    path::{ProbeClock, ProbeMessage},
    protocol::stake_pool as adapter,
    replay::{self, ReplayRecord, ReplayStateSource},
    types::{AccountSnapshot, NamedAccount},
};
use anyhow::{ensure, Context, Result};
use borsh::BorshDeserialize;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_clock::Clock;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_message::Message;
use spl_stake_pool::state::{AccountType, Fee, FeeType, StakePool};
use std::collections::{BTreeMap, BTreeSet};

pub const OPERATION: &str = "spl_stake_pool_sol_deposit_fee_v1";
pub const INPUT_KIND: &str = "spl_stake_pool_historical_deposit_v1";
/// Qualification binds the complete layout and manager signer boundary to this
/// retained deployment. Other deployments need an explicit new qualification.
const QUALIFIED_STAKE_POOL_ELF: &str =
    "ec2dfefaa70d560754a0000f39bd2cabc192b895d36205b3c428f601b6e1d7e1";
mod canonical_u64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
        use serde::de::Error;
        let text = String::deserialize(d)?;
        let value = text.parse::<u64>().map_err(D::Error::custom)?;
        if text != value.to_string() {
            return Err(D::Error::custom("canonical u64 decimal string required"));
        }
        Ok(value)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RationalFee {
    #[serde(with = "canonical_u64")]
    pub numerator: u64,
    #[serde(with = "canonical_u64")]
    pub denominator: u64,
}
impl From<Fee> for RationalFee {
    fn from(f: Fee) -> Self {
        Self {
            numerator: f.numerator,
            denominator: f.denominator,
        }
    }
}
impl From<&RationalFee> for Fee {
    fn from(f: &RationalFee) -> Self {
        Self {
            numerator: f.numerator,
            denominator: f.denominator,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedCurrent {
    pub account_data_sha256: String,
    #[serde(with = "canonical_u64")]
    pub numerator: u64,
    #[serde(with = "canonical_u64")]
    pub denominator: u64,
    pub sol_referral_fee_percent: u8,
    #[serde(with = "canonical_u64")]
    pub last_update_epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramEvidence {
    pub program_id: String,
    pub loader: String,
    pub elf_sha256: String,
    #[serde(with = "crate::hexfmt")]
    pub elf: Vec<u8>,
}
/// No proposed bytes or claimed instruction result. The observed record and
/// its pinned historical deployments are the only user-action inputs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub schema_version: u32,
    pub kind: String,
    pub record: ReplayRecord,
    pub record_sha256: String,
    pub programs: Vec<ProgramEvidence>,
    pub source_bundle_sha256: String,
}
impl Input {
    pub fn from_bundle(bundle: &CiBundle, record_id: &str) -> Result<Self> {
        let record = bundle
            .records()
            .iter()
            .find(|r| r.id == record_id)
            .context("explicit historical record not in retained bundle")?
            .clone();
        let mut programs = Vec::new();
        for p in record.dependencies.loadable() {
            let elf = std::fs::read(if p.program_id == record.program_id {
                bundle.baseline()
            } else {
                bundle.dependencies().join(p.artifact_file_name())
            })?;
            programs.push(ProgramEvidence {
                program_id: p.program_id.clone(),
                loader: match p.loader.context("historical loader missing")? {
                    crate::versions::ProgramLoader::Upgradeable => {
                        crate::versions::UPGRADEABLE_LOADER_ID
                    }
                    crate::versions::ProgramLoader::Legacy => crate::versions::LEGACY_BPF_LOADER_ID,
                }
                .into(),
                elf_sha256: replay::hash_bytes(&elf),
                elf,
            });
        }
        let input = Self {
            schema_version: 1,
            kind: INPUT_KIND.into(),
            record_sha256: crate::canonical::digest(&record)?,
            record,
            programs,
            source_bundle_sha256: bundle.manifest().bundle_sha256.clone(),
        };
        input.validate()?;
        Ok(input)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.kind == INPUT_KIND
                && self.record_sha256 == crate::canonical::digest(&self.record)?,
            "config_evidence_missing: historical record commitment/schema mismatch"
        );
        ensure!(
            self.record.program_id == adapter::PROGRAM_ID,
            "downstream_action_unsupported: Stake Pool record required"
        );
        self.record.validate()?;
        ensure!(matches!(self.record.state_source, ReplayStateSource::HistoricalArchive | ReplayStateSource::ControlledSnapshot) && self.record.original.is_some(),"config_evidence_missing: retained historical boundary or labelled controlled snapshot and original outcome required");
        let ix = deposit(&self.record)?;
        ensure!(
            ix.accounts.len() == 10 && ix.data.len() == 9 && amount(&self.record)? > 0,
            "downstream_action_unsupported: supported nonzero ten-account DepositSol required"
        );
        ensure!(
            self.programs.len() == self.record.dependencies.loadable().count(),
            "config_evidence_missing: exact pinned executable set required"
        );
        let mut unique = BTreeSet::new();
        for p in &self.programs {
            ensure!(unique.insert(&p.program_id), "duplicate program evidence");
            let d = self
                .record
                .dependencies
                .get(&p.program_id)
                .context("unrelated program bytes")?;
            let loader = match d.loader.context("missing loader")? {
                crate::versions::ProgramLoader::Upgradeable => {
                    crate::versions::UPGRADEABLE_LOADER_ID
                }
                crate::versions::ProgramLoader::Legacy => crate::versions::LEGACY_BPF_LOADER_ID,
            };
            ensure!(
                p.loader == loader
                    && Some(&p.elf_sha256) == d.binary_sha256.as_ref()
                    && p.elf_sha256 == replay::hash_bytes(&p.elf)
                    && Some(p.elf.len() as u64) == d.binary_len,
                "config_evidence_missing: historical ELF/loader identity mismatch"
            );
        }
        ensure!(
            self.programs
                .iter()
                .any(|p| p.program_id == adapter::PROGRAM_ID
                    && p.elf_sha256 == self.record.current_program_sha256),
            "historical baseline missing"
        );
        ensure!(self.record.current_program_sha256==QUALIFIED_STAKE_POOL_ELF,"config_execution_unavailable: deployed Stake Pool layout/manager boundary has not been qualified");
        Ok(())
    }
    pub(crate) fn loaded(&self) -> Result<Vec<LoadedProgram>> {
        self.programs
            .iter()
            .map(|p| {
                Ok(LoadedProgram {
                    program_id: p.program_id.parse()?,
                    loader: p.loader.parse()?,
                    bytes: p.elf.clone(),
                })
            })
            .collect()
    }
    pub(crate) fn execute(&self, record: &ReplayRecord) -> Result<ExecutionResult> {
        self.execute_code(record, None)
    }
    pub(crate) fn execute_code(
        &self,
        record: &ReplayRecord,
        candidate: Option<&[u8]>,
    ) -> Result<ExecutionResult> {
        // Resolve the same verified historical dependency manifest in a private
        // temporary directory. No provider or active bundle participates.
        let temp = tempfile::tempdir()?;
        for p in &self.programs {
            std::fs::write(temp.path().join(format!("{}.so", p.program_id)), &p.elf)?;
        }
        let dependencies = replay::load_dependencies(std::slice::from_ref(record), temp.path())?;
        let program = self
            .programs
            .iter()
            .find(|p| p.program_id == adapter::PROGRAM_ID)
            .context("baseline missing")?;
        record.execute(
            &ProgramVersion {
                label: "historical-baseline".into(),
                bytes: candidate.unwrap_or(&program.elf).to_vec(),
            },
            &dependencies,
        )
    }
}
fn deposit(record: &ReplayRecord) -> Result<&crate::types::InstructionSpec> {
    let mut instructions = record
        .transaction
        .instructions
        .iter()
        .filter(|ix| ix.program == adapter::PROGRAM_ID);
    let ix = instructions
        .next()
        .context("downstream_action_unsupported: DepositSol absent")?;
    ensure!(
        instructions.next().is_none() && ix.data.first() == Some(&14),
        "downstream_action_unsupported: exactly one DepositSol required"
    );
    Ok(ix)
}
fn amount(record: &ReplayRecord) -> Result<u64> {
    Ok(u64::from_le_bytes(
        deposit(record)?
            .data
            .get(1..9)
            .context("DepositSol amount absent")?
            .try_into()?,
    ))
}
fn named<'a>(record: &'a ReplayRecord, address: &str) -> Result<&'a NamedAccount> {
    record
        .accounts
        .iter()
        .find(|a| a.address == address)
        .context("retained role account missing")
}
pub(crate) fn pool(record: &ReplayRecord) -> Result<&NamedAccount> {
    named(
        record,
        &deposit(record)?
            .accounts
            .first()
            .context("pool meta absent")?
            .address,
    )
}
/// Decode every official typed field, retain the exact trailing bytes, require
/// a byte-identical roundtrip, and cross-check Eplyx's qualified partial reader.
pub(crate) fn decode(account: &AccountSnapshot) -> Result<(StakePool, Vec<u8>)> {
    ensure!(
        account.owner == adapter::PROGRAM_ID && !account.executable,
        "invalid pool envelope"
    );
    let mut bytes = account.data.as_slice();
    let state = StakePool::deserialize(&mut bytes)
        .context("incompatible complete StakePool Borsh layout")?;
    ensure!(
        state.account_type == AccountType::StakePool,
        "pool not initialized"
    );
    let encoded = borsh::to_vec(&state)?;
    ensure!(
        encoded == account.data[..account.data.len() - bytes.len()],
        "pool typed roundtrip differs"
    );
    let partial =
        adapter::StakePool::decode(&account.data).context("partial pool decoder mismatch")?;
    ensure!(
        partial.sol_deposit_fee.numerator == state.sol_deposit_fee.numerator
            && partial.sol_deposit_fee.denominator == state.sol_deposit_fee.denominator
            && partial.sol_referral_fee == state.sol_referral_fee
            && partial.pool_token_supply == state.pool_token_supply
            && partial.total_lamports == state.total_lamports
            && partial.last_update_epoch == state.last_update_epoch,
        "pool decoder disagreement"
    );
    Ok((state, bytes.to_vec()))
}
pub(crate) fn values(spec: &ChangeSpec) -> Result<(&ExpectedCurrent, &RationalFee)> {
    spec.validate()?;
    match &super::proposal(spec)?.operation {
        Operation::SplStakePoolSolDepositFeeV1 {
            expected_current,
            proposed_fee,
        } => Ok((expected_current, proposed_fee)),
        _ => anyhow::bail!("unsupported_config_field: Stake Pool operation required"),
    }
}
pub(crate) fn prepare(
    spec: &ChangeSpec,
    input: &Input,
) -> std::result::Result<ConfigPlan, Failure> {
    let (expected, fee) = values(spec).map_err(|e| failed(Status::UnsupportedConfigField, e))?;
    input.validate().map_err(|e| {
        failed(
            if e.to_string().starts_with("downstream_action_unsupported:") {
                Status::DownstreamActionUnsupported
            } else if e.to_string().starts_with("config_execution_unavailable:") {
                Status::ConfigExecutionUnavailable
            } else {
                Status::ConfigEvidenceMissing
            },
            e,
        )
    })?;
    let observed = pool(&input.record).map_err(|e| failed(Status::ConfigEvidenceMissing, e))?;
    let target = &super::proposal(spec)
        .map_err(|e| failed(Status::UnsupportedConfigField, e))?
        .target;
    if target.config_account != observed.address
        || target.program_id != observed.account.owner
        || expected.account_data_sha256 != replay::hash_bytes(&observed.account.data)
    {
        return Err(failed(
            Status::CurrentStateMismatch,
            "observed historical pool target/owner/data hash differs from declaration",
        ));
    }
    let (state, _) =
        decode(&observed.account).map_err(|e| failed(Status::ConfigExecutionUnavailable, e))?;
    if expected.numerator != state.sol_deposit_fee.numerator
        || expected.denominator != state.sol_deposit_fee.denominator
        || expected.sol_referral_fee_percent != state.sol_referral_fee
        || expected.last_update_epoch != state.last_update_epoch
    {
        return Err(failed(
            Status::CurrentStateMismatch,
            "observed rational/referral/last-update epoch differs from declaration",
        ));
    }
    config_plan(input, fee, &state).map_err(|e| failed(Status::ConfigExecutionUnavailable, e))
}
pub(crate) struct ConfigPlan {
    pub(crate) accounts: Vec<NamedAccount>,
    pub(crate) watch: Vec<String>,
    pub(crate) message: Message,
    instruction: Instruction,
    manager: String,
    payer: String,
    pub(crate) clock: Clock,
}
fn config_plan(input: &Input, fee: &RationalFee, state: &StakePool) -> Result<ConfigPlan> {
    let observed = pool(&input.record)?;
    let manager = state.manager.to_string();
    let payer = bs58::encode([77u8; 32]).into_string();
    ensure!(
        manager != payer
            && !input
                .record
                .transaction
                .account_keys
                .iter()
                .any(|a| a.address == payer),
        "simulation payer collides with historical message"
    );
    // Qualified processor check_manager reads only key/is_signer, not owner,
    // data or lamports. This account is an explicit simulation assumption.
    let mut accounts = input.record.accounts.clone();
    ensure!(!accounts.iter().any(|a|a.address==manager),"config_execution_unavailable: separate historical manager evidence not qualified for this boundary");
    accounts.push(NamedAccount {
        label: "assumed-manager".into(),
        address: manager.clone(),
        account: AccountSnapshot {
            lamports: 1_000_000,
            owner: adapter::SYSTEM_PROGRAM_ID.into(),
            data: vec![],
            executable: false,
            rent_epoch: 0,
        },
    });
    accounts.push(NamedAccount {
        label: "simulation-payer".into(),
        address: payer.clone(),
        account: AccountSnapshot {
            lamports: 10_000_000,
            owner: adapter::SYSTEM_PROGRAM_ID.into(),
            data: vec![],
            executable: false,
            rent_epoch: u64::MAX,
        },
    });
    let official = spl_stake_pool::instruction::set_fee(
        &spl_stake_pool::id(),
        &observed.address.parse()?,
        &state.manager,
        FeeType::SolDeposit(fee.into()),
    );
    let instruction = Instruction {
        program_id: official.program_id.to_string().parse()?,
        accounts: official
            .accounts
            .iter()
            .map(|meta| {
                Ok(AccountMeta {
                    pubkey: meta.pubkey.to_string().parse()?,
                    is_signer: meta.is_signer,
                    is_writable: meta.is_writable,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        data: official.data,
    };
    let message = Message::new(std::slice::from_ref(&instruction), Some(&payer.parse()?));
    let c = &input.record.clock;
    Ok(ConfigPlan {
        watch: accounts.iter().map(|a| a.address.clone()).collect(),
        accounts,
        message,
        instruction,
        manager,
        payer,
        clock: Clock {
            slot: c.slot,
            epoch: c.epoch,
            epoch_start_timestamp: c.epoch_start_timestamp,
            leader_schedule_epoch: c.leader_schedule_epoch,
            unix_timestamp: c.unix_timestamp,
        },
    })
}
pub(crate) fn config_commitment(input: &Input, plan: &ConfigPlan) -> Value {
    json!({"origin":"simulated_configuration_instruction","message":ProbeMessage::from(&plan.message),"instruction":{"program_id":plan.instruction.program_id.to_string(),"accounts":plan.instruction.accounts.iter().map(|a|json!({"address":a.pubkey.to_string(),"is_signer":a.is_signer,"is_writable":a.is_writable})).collect::<Vec<_>>(),"data_hex":crate::hexfmt::encode(&plan.instruction.data)},"pre_accounts":plan.accounts,"watch":plan.watch,"clock":ProbeClock::from(&plan.clock),"programs":program_commitments(input),"manager_assumption":{"origin":"assumed_simulation_only","address":plan.manager,"signer":true,"observed":false,"key_possession_established":false,"boundary":"Qualified SetFee/check_manager checks only manager key and signer privilege; account owner, data and balance are simulation-only."},"fee_payer":{"origin":"assumed_simulation_only","address":plan.payer,"propagated_to_user_action":false},"runtime":runtime()})
}
pub(crate) fn verify_config(
    input: &Input,
    fee: &RationalFee,
    plan: &ConfigPlan,
    x: &ProbeTransactionExecution,
) -> Result<AccountSnapshot> {
    ensure!(
        x.success && x.error.is_none(),
        "config execution not successful"
    );
    ensure!(
        x.inner_instructions.is_empty(),
        "unexpected configuration CPI"
    );
    ensure!(
        x.post_accounts.keys().cloned().collect::<BTreeSet<_>>()
            == plan.watch.iter().cloned().collect(),
        "missing/extra configuration post accounts"
    );
    let observed = pool(&input.record)?;
    let next = x
        .post_accounts
        .get(&observed.address)
        .context("post pool absent")?;
    let (before, trailing) = decode(&observed.account)?;
    let (after, post_trailing) = decode(next)?;
    let mut expected = before;
    expected.sol_deposit_fee = fee.into();
    ensure!(
        after == expected && trailing == post_trailing,
        "unrelated typed pool field or trailing bytes changed"
    );
    let mut restored = next.clone();
    restored.data = observed.account.data.clone();
    ensure!(
        restored == observed.account && next.data.len() == observed.account.data.len(),
        "pool envelope/length changed"
    );
    ensure!(
        x.transaction_fee_lamports == u64::from(plan.message.header.num_required_signatures) * 5000,
        "unexpected modeled configuration fee"
    );
    for a in &plan.accounts {
        if a.address == observed.address {
            continue;
        }
        let mut want = a.account.clone();
        if a.address == plan.payer {
            want.lamports = want
                .lamports
                .checked_sub(x.transaction_fee_lamports)
                .context("fee exceeds payer")?;
        }
        ensure!(
            x.post_accounts[&a.address] == want,
            "unexpected non-pool configuration change: {}",
            a.address
        );
    }
    Ok(next.clone())
}
pub(crate) fn verify_config_rejection(
    plan: &ConfigPlan,
    x: &ProbeTransactionExecution,
) -> Result<()> {
    ensure!(
        !x.success && x.error.is_some() && x.inner_instructions.is_empty(),
        "configuration rejection outcome missing or inconsistent"
    );
    ensure!(
        x.transaction_fee_lamports == u64::from(plan.message.header.num_required_signatures) * 5000,
        "configuration rejection fee mismatch"
    );
    ensure!(
        x.post_accounts.len() == plan.accounts.len(),
        "configuration rollback accounts absent"
    );
    for a in &plan.accounts {
        let mut expected = a.account.clone();
        if a.address == plan.payer {
            expected.lamports = expected
                .lamports
                .checked_sub(x.transaction_fee_lamports)
                .context("configuration rollback payer fee")?;
        }
        ensure!(
            x.post_accounts.get(&a.address) == Some(&expected),
            "configuration rejection did not roll back {}",
            a.address
        );
    }
    Ok(())
}
fn program_commitments(input: &Input) -> Value {
    json!(input
        .programs
        .iter()
        .map(|p| json!({"program_id":p.program_id,"loader":p.loader,"elf_sha256":p.elf_sha256}))
        .collect::<Vec<_>>())
}
pub fn runtime() -> Value {
    json!({"backend":"LiteSVM 0.16","profile":"schema1_litesvm_mainnet","signature_verification":false,"recent_blockhash_verification":false,"clock_advanced":false,"revision":OPERATION,"official_interface":"spl-stake-pool=2.0.3","lock_sha256":replay::hash_bytes(include_bytes!("../../../Cargo.lock")),"executor_sha256":replay::hash_bytes(include_bytes!("../executor.rs")),"replay_sha256":replay::hash_bytes(include_bytes!("../replay.rs")),"operation_source_sha256":replay::hash_bytes(include_bytes!("stake_pool.rs"))})
}
pub(crate) fn action_commitment(input: &Input, record: &ReplayRecord) -> Result<Value> {
    Ok(
        json!({"accounts":record.accounts,"pre_state_hash":record.pre_state_hash,"transaction":record.transaction,"message":ProbeMessage::from(&record.message()?),"clock":record.clock,"dependencies":record.dependencies,"programs":program_commitments(input),"watch":record.fixture().watch,"assumptions":record.assumptions,"runtime":runtime()}),
    )
}
pub(crate) fn proposed_record(input: &Input, next: &AccountSnapshot) -> Result<ReplayRecord> {
    let mut record = input.record.clone();
    let address = &deposit(&record)?.accounts[0].address.clone();
    let index = record
        .accounts
        .iter()
        .position(|a| &a.address == address)
        .context("pool absent")?;
    record.accounts[index].account = next.clone();
    record.pre_state_hash = replay::state_hash(&record.accounts)?;
    let original = action_commitment(input, &input.record)?;
    let mut restored = action_commitment(input, &record)?;
    restored["accounts"][index]["account"] =
        serde_json::to_value(&input.record.accounts[index].account)?;
    restored["pre_state_hash"] = input.record.pre_state_hash.clone().into();
    ensure!(
        restored == original,
        "paired commitments differ beyond verified pool"
    );
    Ok(record)
}
/// Unique-account credit reconciliation. Aliased roles carry combined observed
/// credit, never an invented independently measured referral/recipient split.
pub fn reconcile(record: &ReplayRecord, x: &ExecutionResult) -> Result<Value> {
    let ix = deposit(record)?;
    ensure!(
        x.accounts.len() == record.accounts.len(),
        "missing action post accounts"
    );
    ensure!(
        x.fee == record.transaction.fee,
        "action transaction fee differs from retained message"
    );
    if !x.success {
        for a in &record.accounts {
            let mut expected = a.account.clone();
            if a.address == record.transaction.payer {
                expected.lamports = expected
                    .lamports
                    .checked_sub(x.fee)
                    .context("rollback payer")?;
            }
            ensure!(
                x.accounts.get(&a.label) == Some(&expected),
                "rejected action did not roll back watched state"
            );
        }
        return Ok(
            json!({"reconciled":true,"success":false,"rollback_verified":true,"action_transaction_fee_lamports":x.fee.to_string()}),
        );
    }
    ensure!(x.error.is_none(), "successful action has error");
    let pre_pool = pool(record)?;
    let (before, _) = decode(&pre_pool.account)?;
    let pool_post = x
        .accounts
        .get(&pre_pool.label)
        .context("pool post absent")?;
    let (after, post_trailing) = decode(pool_post)?;
    let mut expected_pool = before.clone();
    expected_pool.total_lamports = after.total_lamports;
    expected_pool.pool_token_supply = after.pool_token_supply;
    ensure!(
        after == expected_pool && decode(&pre_pool.account)?.1 == post_trailing,
        "unrelated action pool state changed"
    );
    let mut restored_pool = pool_post.clone();
    restored_pool.data = pre_pool.account.data.clone();
    ensure!(
        restored_pool == pre_pool.account && pool_post.data.len() == pre_pool.account.data.len(),
        "action pool envelope changed"
    );
    let delta = |a: u64, b: u64| i128::from(b) - i128::from(a);
    let mut token_roles: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (i, role) in [(4, "recipient"), (5, "manager_fee"), (6, "referral_fee")] {
        token_roles
            .entry(ix.accounts[i].address.clone())
            .or_default()
            .push(role);
    }
    let mut credits = Vec::new();
    let mut sum = 0i128;
    let mut recipient_credit = 0;
    let mut manager_credit = None;
    let mut referral_credit = None;
    for (address, roles) in &token_roles {
        let a = named(record, address)?;
        let post = x.accounts.get(&a.label).context("token post missing")?;
        ensure!(
            a.account.owner == adapter::TOKEN_PROGRAM_ID && post.owner == a.account.owner,
            "token role owner mismatch"
        );
        let before =
            crate::standard_programs::token::account_base(&a.account.owner, &a.account.data)?;
        let after = crate::standard_programs::token::account_base(&post.owner, &post.data)?;
        ensure!(
            before.mint == ix.accounts[7].address && after.mint == before.mint,
            "role mint mismatch"
        );
        let credit = delta(before.amount, after.amount);
        ensure!(credit >= 0, "negative minted account credit");
        sum += credit;
        let mut restored = post.clone();
        restored.data[64..72].copy_from_slice(&a.account.data[64..72]);
        ensure!(
            restored == a.account,
            "unrelated token account state changed"
        );
        if roles.contains(&"recipient") {
            recipient_credit = credit;
        }
        if roles.len() == 1 && roles.contains(&"manager_fee") {
            manager_credit = Some(credit.to_string());
        }
        if roles.len() == 1 && roles.contains(&"referral_fee") {
            referral_credit = Some(credit.to_string());
        }
        credits.push(json!({"address":address,"roles":roles,"credit_raw":credit.to_string(),"independently_measured_role":roles.len()==1}));
    }
    let mint = named(record, &ix.accounts[7].address)?;
    let mint_post = x.accounts.get(&mint.label).context("mint post missing")?;
    let mint_delta = delta(
        crate::standard_programs::token::mint_base(&mint.account.owner, &mint.account.data)?.supply,
        crate::standard_programs::token::mint_base(&mint_post.owner, &mint_post.data)?.supply,
    );
    let mut restored_mint = mint_post.clone();
    restored_mint.data[36..44].copy_from_slice(&mint.account.data[36..44]);
    ensure!(
        restored_mint == mint.account,
        "unrelated mint state changed"
    );
    let supply_delta = delta(before.pool_token_supply, after.pool_token_supply);
    let reserve = named(record, &ix.accounts[2].address)?;
    let reserve_delta = delta(
        reserve.account.lamports,
        x.accounts
            .get(&reserve.label)
            .context("reserve post missing")?
            .lamports,
    );
    ensure!(
        sum == mint_delta && mint_delta == supply_delta && mint_delta > 0,
        "pool token supply/account credit reconciliation failed"
    );
    ensure!(
        reserve_delta == i128::from(amount(record)?)
            && delta(before.total_lamports, after.total_lamports) == reserve_delta,
        "reserve/pool lamport reconciliation failed"
    );
    let payer = named(record, &record.transaction.payer)?;
    let payer_debit = i128::from(payer.account.lamports)
        - i128::from(
            x.accounts
                .get(&payer.label)
                .context("payer post missing")?
                .lamports,
        );
    let net: i128 = record
        .accounts
        .iter()
        .map(|a| {
            x.accounts
                .get(&a.label)
                .map(|p| delta(a.account.lamports, p.lamports))
                .context("post account absent")
        })
        .collect::<Result<Vec<_>>>()?
        .iter()
        .sum();
    ensure!(
        net == -i128::from(x.fee),
        "lamport ledger does not reconcile to action transaction fee"
    );
    for a in &record.accounts {
        if a.address == pre_pool.address
            || a.address == mint.address
            || token_roles.contains_key(&a.address)
        {
            continue;
        }
        let post = x.accounts.get(&a.label).context("action post missing")?;
        let mut restored = post.clone();
        restored.lamports = a.account.lamports;
        ensure!(
            restored == a.account,
            "unrelated action account bytes/envelope changed"
        );
    }
    Ok(
        json!({"reconciled":true,"success":true,"recipient_account_credit_raw":recipient_credit.to_string(),"recipient_credit_contains_aliased_roles":token_roles[&ix.accounts[4].address].len()>1,"manager_fee_account_credit_raw":manager_credit,"referral_account_credit_raw":referral_credit,"unique_token_account_credits":credits,"mint_supply_delta_raw":mint_delta.to_string(),"pool_token_supply_delta_raw":supply_delta.to_string(),"reserve_lamport_delta":reserve_delta.to_string(),"pool_total_lamports_delta":reserve_delta.to_string(),"funding_payer_lamport_debit":payer_debit.to_string(),"funding_payer_debit_excluding_transaction_fee":(payer_debit-i128::from(x.fee)).to_string(),"action_transaction_fee_lamports":x.fee.to_string(),"config_transaction_fee_included":false,"fee_split_limitation":"Aliased role credit is measured once; no independent role split is asserted."}),
    )
}
fn side(record: &ReplayRecord, x: &ExecutionResult) -> Result<Value> {
    let (r, e) = match reconcile(record, x) {
        Ok(v) => (v, None),
        Err(e) => (Value::Null, Some(e.to_string())),
    };
    Ok(
        json!({"origin":"simulated_user_action_result","pre_accounts":record.accounts,"execution":x,"execution_sha256":crate::canonical::digest(x)?,"reconciliation":r,"reconciliation_error":e}),
    )
}
fn consequences(b: &Value, p: &Value) -> Result<(Status, Vec<Value>)> {
    if b["reconciliation"]["reconciled"] != true || p["reconciliation"]["reconciled"] != true {
        return Ok((Status::ReconciliationFailed, vec![]));
    }
    let bs = b["execution"]["success"] == true;
    let ps = p["execution"]["success"] == true;
    if bs != ps {
        return Ok((
            Status::SemanticConsequenceObserved,
            vec![
                json!({"fingerprint":format!("spl-stake-pool/deposit_sol/execution/transaction/{}",if ps{"now_succeeds"}else{"now_reverts"}),"baseline":bs,"proposed":ps}),
            ],
        ));
    }
    if !bs {
        return Ok((Status::ExecutionRejected, vec![]));
    }
    let bv = b["reconciliation"]["recipient_account_credit_raw"]
        .as_str()
        .context("recipient credit missing")?
        .parse::<i128>()?;
    let pv = p["reconciliation"]["recipient_account_credit_raw"]
        .as_str()
        .context("recipient credit missing")?
        .parse::<i128>()?;
    if bv == pv {
        return Ok((Status::NoObservedConsequence, vec![]));
    }
    Ok((
        Status::SemanticConsequenceObserved,
        vec![
            json!({"fingerprint":format!("spl-stake-pool/deposit_sol/economic/pool_tokens_received/{}",if pv>bv{"increased"}else{"decreased"}),"baseline_raw":bv.to_string(),"proposed_raw":pv.to_string(),"delta_raw":(pv-bv).to_string()}),
        ],
    ))
}
fn preamble(spec: &ChangeSpec, input: &Input) -> Result<Value> {
    let origin = if input.record.state_source == ReplayStateSource::ControlledSnapshot {
        "labelled_synthetic_control"
    } else {
        "observed_historical_record"
    };
    Ok(
        json!({"schema":REPORT_SCHEMA,"kind":KIND,"operation":OPERATION,"change":binding(spec)?,"retained_input":input,"analysis_input_sha256":crate::canonical::digest(input)?,"runtime":runtime(),"authorization":false,"funds_moved":false,"on_chain_update":false,"execution_performed":false,"findings":[],"observed_current_state":{"origin":origin,"record_id":input.record.id,"signature":input.record.transaction.signature,"slot":input.record.transaction.slot.to_string(),"record_sha256":input.record_sha256,"source_bundle_sha256":input.source_bundle_sha256,"pool":pool(&input.record).ok(),"programs":program_commitments(input)},"proposed_declaration":{"origin":"validated_changespec","change_spec_id":spec.id()?,"operation":super::proposal(spec)?.operation},"limitations":["One retained historical DepositSol or explicitly labelled synthetic control; no live configuration, population-wide impact, fairness or valuation claim.","Same historical deployed program and dependencies on both sides. Independent user-action VMs differ only by the verified instruction-produced pool account.","SetFee is simulated with an assumed manager signer and separate assumed payer. No key possession, externally signable authority or governance approval is established.","The retained schema-1 Clock/runtime is fixed, including its retained epoch; no time advancement or slot-accurate validator feature reconstruction.","Pool-token credits are reconciled from actual VM account deltas. Aliased roles are counted once and do not establish an independent role split."]}),
    )
}
fn failure_report(
    spec: &ChangeSpec,
    input: &Input,
    f: &Failure,
    config: Option<Value>,
    baseline: Option<Value>,
) -> Result<Value> {
    let mut r = preamble(spec, input)?;
    r["status"] = serde_json::to_value(f.status)?;
    r["failure"] = json!({"status":f.status,"detail":f.detail});
    if let Some(config) = config {
        r["simulated_config_instruction"] = config;
    }
    if let Some(baseline) = baseline {
        r["baseline"] = baseline;
    }
    super::seal(r)
}
fn config_value(input: &Input, plan: &ConfigPlan, x: &ProbeTransactionExecution) -> Result<Value> {
    let mut v = config_commitment(input, plan);
    v["execution"] = serde_json::to_value(x)?;
    v["execution_sha256"] = crate::canonical::digest(x)?.into();
    Ok(v)
}
fn finish(
    spec: &ChangeSpec,
    input: &Input,
    plan: &ConfigPlan,
    x: &ProbeTransactionExecution,
    next: &AccountSnapshot,
    b: Value,
    p: Value,
) -> Result<Value> {
    let mut r = preamble(spec, input)?;
    let (_, fee) = values(spec)?;
    ensure!(
        verify_config(input, fee, plan, x)? == *next,
        "post configuration pool differs"
    );
    let proposed = proposed_record(input, next)?;
    r["simulated_config_instruction"] = config_value(input, plan, x)?;
    r["simulated_proposed_pre_state"] = json!({"origin":"instruction_produced_pool_state","parent_pool_data_sha256":replay::hash_bytes(&pool(&input.record)?.account.data),"pool_data_sha256":replay::hash_bytes(&next.data),"pool":next,"configuration_execution_sha256":crate::canonical::digest(x)?,"unrelated_typed_fields_envelope_and_trailing_bytes_preserved":true});
    r["baseline_fidelity"] = json!({"status":input.record.fidelity(&serde_json::from_value(b["execution"].clone())?)?,"failures":input.record.fidelity_failures(&serde_json::from_value(b["execution"].clone())?)?});
    ensure!(
        r["baseline_fidelity"]["failures"] == json!([]),
        "baseline fidelity failed"
    );
    r["baseline_execution_commitment"] = action_commitment(input, &input.record)?;
    r["proposed_execution_commitment"] = action_commitment(input, &proposed)?;
    let (status, findings) = consequences(&b, &p)?;
    r["status"] = serde_json::to_value(status)?;
    r["findings"] = json!(findings);
    r["baseline"] = b;
    r["proposed"] = p;
    r["execution_performed"] = true.into();
    super::seal(r)
}
pub fn analyze(spec: &ChangeSpec, input: &Input) -> Result<Value> {
    execute_analysis(spec, input, false)
}
fn execute_analysis(spec: &ChangeSpec, input: &Input, configuration_first: bool) -> Result<Value> {
    values(spec)?;
    let plan = match prepare(spec, input) {
        Ok(v) => v,
        Err(f) => return failure_report(spec, input, &f, None, None),
    };
    // Initial qualification runs the observed baseline first. Reproduction
    // executes configuration, baseline, then proposed in three fresh VMs.
    let programs = input.loaded()?;
    let execute_config = || {
        executor::execute_probe_message(
            &plan.accounts,
            &plan.watch,
            plan.clock.clone(),
            &programs,
            plan.message.clone(),
        )
    };
    let early_config = if configuration_first {
        Some(execute_config())
    } else {
        None
    };
    let baseline = match input.execute(&input.record) {
        Ok(x) => x,
        Err(e) => {
            return failure_report(
                spec,
                input,
                &failed(Status::ExecutionUnavailable, e),
                None,
                None,
            )
        }
    };
    let b = side(&input.record, &baseline)?;
    if !input.record.fidelity_failures(&baseline)?.is_empty() {
        return failure_report(
            spec,
            input,
            &failed(
                Status::ConfigEvidenceMissing,
                "original baseline replay fidelity mismatch",
            ),
            None,
            Some(b),
        );
    }
    let x = match early_config.unwrap_or_else(execute_config) {
        Ok(x) => x,
        Err(e) => {
            return failure_report(
                spec,
                input,
                &failed(Status::ConfigExecutionUnavailable, e),
                None,
                Some(b),
            )
        }
    };
    let config = config_value(input, &plan, &x)?;
    if !x.success {
        if let Err(e) = verify_config_rejection(&plan, &x) {
            return failure_report(
                spec,
                input,
                &failed(Status::PostConfigStateMismatch, e),
                Some(config),
                Some(b),
            );
        }
        return failure_report(
            spec,
            input,
            &failed(
                Status::ConfigExecutionRejected,
                x.error.as_deref().unwrap_or("SetFee rejected"),
            ),
            Some(config),
            Some(b),
        );
    }
    let (_, fee) = values(spec)?;
    let next = match verify_config(input, fee, &plan, &x) {
        Ok(next) => next,
        Err(e) => {
            return failure_report(
                spec,
                input,
                &failed(Status::PostConfigStateMismatch, e),
                Some(config),
                Some(b),
            )
        }
    };
    let proposed = proposed_record(input, &next)?;
    let p = match input.execute(&proposed) {
        Ok(p) => p,
        Err(e) => {
            return failure_report(
                spec,
                input,
                &failed(Status::ExecutionUnavailable, e),
                Some(config),
                Some(b),
            )
        }
    };
    finish(spec, input, &plan, &x, &next, b, side(&proposed, &p)?)
}
/// Reconstruct provenance, preservation and all derived results without a VM.
fn verify_current(spec: &ChangeSpec, report: &Value) -> Result<()> {
    let mut unsealed = report.clone();
    let hash = unsealed
        .as_object_mut()
        .context("report object required")?
        .remove("report_sha256")
        .context("report seal missing")?;
    ensure!(
        hash == crate::canonical::digest(&unsealed)?,
        "report seal mismatch"
    );
    let input: Input = serde_json::from_value(report["retained_input"].clone())?;
    let plan = match prepare(spec, &input) {
        Ok(v) => v,
        Err(f) => {
            ensure!(
                *report == failure_report(spec, &input, &f, None, None)?,
                "pre-execution failure mismatch"
            );
            return Ok(());
        }
    };
    let b = if report.get("baseline").is_some() {
        let x: ExecutionResult = serde_json::from_value(report["baseline"]["execution"].clone())?;
        Some(side(&input.record, &x)?)
    } else {
        None
    };
    let x = if report.get("simulated_config_instruction").is_some() {
        let x: ProbeTransactionExecution =
            serde_json::from_value(report["simulated_config_instruction"]["execution"].clone())?;
        ensure!(
            report["simulated_config_instruction"] == config_value(&input, &plan, &x)?,
            "config message/assumptions/execution mismatch"
        );
        Some(x)
    } else {
        None
    };
    if report["execution_performed"] == true {
        let x = x.context("configuration result absent")?;
        let (_, fee) = values(spec)?;
        let next = verify_config(&input, fee, &plan, &x)?;
        let proposed = proposed_record(&input, &next)?;
        let p: ExecutionResult = serde_json::from_value(report["proposed"]["execution"].clone())?;
        ensure!(
            *report
                == finish(
                    spec,
                    &input,
                    &plan,
                    &x,
                    &next,
                    b.context("baseline absent")?,
                    side(&proposed, &p)?
                )?,
            "pool report cross-object/result mismatch"
        );
        return Ok(());
    }
    let status: Status = serde_json::from_value(report["status"].clone())?;
    let (_, fee) = values(spec)?;
    let detail = match (status, x.as_ref(), b.as_ref()) {
        (Status::ConfigExecutionRejected, Some(x), Some(b)) => {
            verify_config_rejection(&plan, x)?;
            ensure!(
                input
                    .record
                    .fidelity_failures(&serde_json::from_value(b["execution"].clone())?)?
                    .is_empty(),
                "invalid configuration rejection/baseline"
            );
            x.error
                .as_deref()
                .context("rejection error absent")?
                .to_string()
        }
        (Status::PostConfigStateMismatch, Some(x), Some(_)) => {
            let error = if x.success {
                verify_config(&input, fee, &plan, x).err()
            } else {
                verify_config_rejection(&plan, x).err()
            };
            error
                .context("claimed post-state mismatch is false")?
                .to_string()
        }
        (Status::ConfigEvidenceMissing, None, Some(b)) => {
            ensure!(
                !input
                    .record
                    .fidelity_failures(&serde_json::from_value(b["execution"].clone())?)?
                    .is_empty(),
                "baseline mismatch is false"
            );
            "original baseline replay fidelity mismatch".into()
        }
        (Status::ExecutionUnavailable, None, None)
        | (Status::ConfigExecutionUnavailable, None, Some(_))
        | (Status::ExecutionUnavailable, Some(_), Some(_)) => report["failure"]["detail"]
            .as_str()
            .context("missing infrastructure error")?
            .to_string(),
        _ => anyhow::bail!("inadmissible recorded failure distinction"),
    };
    ensure!(
        *report
            == failure_report(
                spec,
                &input,
                &failed(status, detail),
                x.as_ref()
                    .map(|x| config_value(&input, &plan, x))
                    .transpose()?,
                b
            )?,
        "failure evidence mismatch"
    );
    Ok(())
}
// Reviewed compatibility: only the visibility/generalized execution seam changed.
// All runtime pins and every other receipt field remain checked. Archived source
// commitments are preserved, never silently upgraded on disk.
const RETAINED_SOURCE: &str = "63aa307015b17f79d6b07e71f024f624eb2388ad15e6ba8264a5b3fa2a9a9a87";
fn replace_runtime(value: &mut Value, from: &Value, to: &Value) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "runtime" {
                    ensure!(*child == *from, "inconsistent retained runtime");
                    *child = to.clone();
                } else {
                    replace_runtime(child, from, to)?;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_runtime(item, from, to)?;
            }
        }
        _ => (),
    }
    Ok(())
}
fn runtime_compatible(report: &Value) -> Result<()> {
    let mut recorded = report["runtime"].clone();
    if recorded == runtime() {
        return Ok(());
    }
    ensure!(
        recorded["operation_source_sha256"] == RETAINED_SOURCE,
        "unreviewed retained Stake Pool source"
    );
    recorded["operation_source_sha256"] = runtime()["operation_source_sha256"].clone();
    ensure!(
        recorded == runtime(),
        "retained Stake Pool runtime pins differ"
    );
    Ok(())
}
pub fn verify(spec: &ChangeSpec, report: &Value) -> Result<()> {
    let mut normalized = report.clone();
    let seal = normalized
        .as_object_mut()
        .context("report object required")?
        .remove("report_sha256")
        .context("report seal missing")?;
    ensure!(
        seal == crate::canonical::digest(&normalized)?,
        "report seal mismatch"
    );
    runtime_compatible(report)?;
    replace_runtime(&mut normalized, &report["runtime"], &runtime())?;
    verify_current(spec, &super::seal(normalized)?)
}
pub fn reproduce(spec: &ChangeSpec, report: &Value) -> Result<()> {
    verify(spec, report)?;
    let input: Input = serde_json::from_value(report["retained_input"].clone())?;
    let mut rebuilt = execute_analysis(spec, &input, true)?;
    rebuilt.as_object_mut().unwrap().remove("report_sha256");
    replace_runtime(&mut rebuilt, &runtime(), &report["runtime"])?;
    ensure!(
        *report == super::seal(rebuilt)?,
        "offline configuration/action reproduction differs"
    );
    Ok(())
}

#[cfg(test)]
mod qualification {
    use super::*;
    fn retained() -> Input {
        Input::from_bundle(
            &CiBundle::open("../deploy/bundle").unwrap(),
            "mainnet-spl-stake-pool-151010f709e113e7",
        )
        .unwrap()
    }
    #[test]
    fn real_manager_checks_and_assumed_envelope_boundary() {
        let input = retained();
        let (state, _) = decode(&pool(&input.record).unwrap().account).unwrap();
        let fee = RationalFee {
            numerator: 1,
            denominator: 100,
        };
        for mode in [
            "correct",
            "missing_signer",
            "wrong_manager",
            "different_envelope",
        ] {
            let mut plan = config_plan(&input, &fee, &state).unwrap();
            match mode {
                "missing_signer" => plan.instruction.accounts[1].is_signer = false,
                "wrong_manager" => {
                    let key = bs58::encode([91; 32]).into_string();
                    plan.accounts
                        .iter_mut()
                        .find(|a| a.address == plan.manager)
                        .unwrap()
                        .address = key.clone();
                    if let Some(address) = plan.watch.iter_mut().find(|a| **a == plan.manager) {
                        *address = key.clone();
                    }
                    plan.instruction.accounts[1].pubkey = key.parse().unwrap();
                }
                "different_envelope" => {
                    let a = &mut plan
                        .accounts
                        .iter_mut()
                        .find(|a| a.address == plan.manager)
                        .unwrap()
                        .account;
                    a.owner = bs58::encode([92; 32]).into_string();
                    a.data = vec![17; 8];
                    a.lamports = 2_000_000;
                }
                _ => {}
            }
            plan.message = Message::new(
                std::slice::from_ref(&plan.instruction),
                Some(&plan.payer.parse().unwrap()),
            );
            let x = executor::execute_probe_message(
                &plan.accounts,
                &plan.watch,
                plan.clock.clone(),
                &input.loaded().unwrap(),
                plan.message.clone(),
            )
            .unwrap();
            if mode == "correct" || mode == "different_envelope" {
                assert!(x.success, "{mode}: {:?}", x.logs);
                verify_config(&input, &fee, &plan, &x).unwrap();
            } else {
                assert!(!x.success, "{mode}");
                assert_eq!(
                    x.post_accounts[&pool(&input.record).unwrap().address],
                    pool(&input.record).unwrap().account
                );
                assert!(
                    x.logs
                        .iter()
                        .any(|l| l.contains(if mode == "missing_signer" {
                            "signature missing"
                        } else {
                            "Incorrect manager"
                        })),
                    "{:?}",
                    x.logs
                );
            }
        }
    }
    #[test]
    fn official_variable_options_and_future_states_are_preserved_by_real_set_fee() {
        let mut input = retained();
        let a = input
            .record
            .accounts
            .iter_mut()
            .find(|a| a.label == "stake-pool")
            .unwrap();
        let (mut state, _) = decode(&a.account).unwrap();
        state.preferred_deposit_validator_vote_address = Some(state.manager);
        state.preferred_withdraw_validator_vote_address = Some(state.staker);
        state.next_epoch_fee = spl_stake_pool::state::FutureEpoch::One(Fee {
            denominator: 123,
            numerator: 7,
        });
        state.next_stake_withdrawal_fee = spl_stake_pool::state::FutureEpoch::Two(Fee {
            denominator: 456,
            numerator: 8,
        });
        state.next_sol_withdrawal_fee = spl_stake_pool::state::FutureEpoch::One(Fee {
            denominator: 789,
            numerator: 9,
        });
        // Labelled layout control, never retained observed evidence.
        a.account.data = borsh::to_vec(&state).unwrap();
        a.account.data.extend_from_slice(&[0xAB; 64]);
        let plan = config_plan(
            &input,
            &RationalFee {
                numerator: 2,
                denominator: 100,
            },
            &state,
        )
        .unwrap();
        let x = executor::execute_probe_message(
            &plan.accounts,
            &plan.watch,
            plan.clock.clone(),
            &input.loaded().unwrap(),
            plan.message.clone(),
        )
        .unwrap();
        assert!(x.success, "{:?}", x.logs);
        verify_config(
            &input,
            &RationalFee {
                numerator: 2,
                denominator: 100,
            },
            &plan,
            &x,
        )
        .unwrap();
        for malformed in [vec![1; 200], vec![0; 611]] {
            let mut a = pool(&input.record).unwrap().account.clone();
            a.data = malformed;
            assert!(decode(&a).is_err());
        }
    }
    fn controlled(mut input: Input) -> Input {
        input.record.id = "labelled-synthetic-deposit-control".into();
        input.record.state_source = ReplayStateSource::ControlledSnapshot;
        input.record.pre_state_hash = replay::state_hash(&input.record.accounts).unwrap();
        let x = input.execute(&input.record).unwrap();
        assert!(x.success, "{:?}", x.logs);
        input.record.transaction.logs = x.logs.clone();
        input.record.transaction.compute_units = x.compute_units;
        input.record.original = Some(replay::OriginalExecution {
            success: x.success,
            fee: x.fee,
            post_state_hash: input.record.post_hash(&x).unwrap(),
            post_accounts: vec![],
            cpi_invocations: replay::cpi_graph(&x.cpi_calls),
        });
        input.record_sha256 = crate::canonical::digest(&input.record).unwrap();
        input
    }
    fn declaration(input: &Input, fee: RationalFee) -> ChangeSpec {
        let pool = pool(&input.record).unwrap();
        let (s, _) = decode(&pool.account).unwrap();
        ChangeSpec {
            schema_version: 1,
            change_spec_id: None,
            activation: None,
            metadata: Default::default(),
            change: crate::change::Change::ProtocolParameterChange(Box::new(
                super::super::ParameterChange {
                    target: super::super::ConfigTarget {
                        program_id: adapter::PROGRAM_ID.into(),
                        config_account: pool.address.clone(),
                    },
                    operation: Operation::SplStakePoolSolDepositFeeV1 {
                        expected_current: ExpectedCurrent {
                            account_data_sha256: replay::hash_bytes(&pool.account.data),
                            numerator: s.sol_deposit_fee.numerator,
                            denominator: s.sol_deposit_fee.denominator,
                            sol_referral_fee_percent: s.sol_referral_fee,
                            last_update_epoch: s.last_update_epoch,
                        },
                        proposed_fee: fee,
                    },
                },
            )),
        }
    }
    #[test]
    fn labelled_rounding_no_consequence_and_actual_downstream_rejection_rollback() {
        let mut input = retained();
        let (state, _) = decode(&pool(&input.record).unwrap().account).unwrap();
        let fee = RationalFee {
            numerator: 1,
            denominator: u64::MAX,
        };
        let plan = config_plan(&input, &fee, &state).unwrap();
        let x = executor::execute_probe_message(
            &plan.accounts,
            &plan.watch,
            plan.clock.clone(),
            &input.loaded().unwrap(),
            plan.message.clone(),
        )
        .unwrap();
        let next = verify_config(&input, &fee, &plan, &x).unwrap();
        input
            .record
            .accounts
            .iter_mut()
            .find(|a| a.label == "stake-pool")
            .unwrap()
            .account = next;
        let input = controlled(input);
        let spec = declaration(
            &input,
            RationalFee {
                numerator: 2,
                denominator: u64::MAX,
            },
        );
        let r = analyze(&spec, &input).unwrap();
        assert_eq!(r["status"], "no_observed_consequence");
        assert_eq!(
            r["observed_current_state"]["origin"],
            "labelled_synthetic_control"
        );
        assert_eq!(
            r["baseline"]["reconciliation"]["manager_fee_account_credit_raw"],
            "1"
        );
        reproduce(&spec, &r).unwrap();
        let mut input = retained();
        input
            .record
            .accounts
            .iter_mut()
            .find(|a| a.label == "manager-fee")
            .unwrap()
            .account
            .data[64..72]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        let input = controlled(input);
        let spec = declaration(
            &input,
            RationalFee {
                numerator: 1,
                denominator: 100,
            },
        );
        let r = analyze(&spec, &input).unwrap();
        assert_eq!(r["status"], "reconciliation_failed");
        assert!(r["findings"].as_array().unwrap().is_empty());
        assert_eq!(
            r["proposed"]["reconciliation_error"],
            "negative minted account credit"
        );
        reproduce(&spec, &r).unwrap();
        // A readonly manager destination is harmless for the zero-fee baseline,
        // but the nonzero-fee CPI must reject privilege escalation and roll back.
        let mut input = retained();
        let manager = deposit(&input.record).unwrap().accounts[5].address.clone();
        let old_keys = input.record.transaction.account_keys.clone();
        for key in &mut input.record.transaction.account_keys {
            if key.address == manager {
                key.is_writable = false;
            }
        }
        input
            .record
            .transaction
            .account_keys
            .sort_by_key(|key| (!key.is_signer, !key.is_writable));
        let order: Vec<usize> = input
            .record
            .transaction
            .account_keys
            .iter()
            .map(|key| {
                old_keys
                    .iter()
                    .position(|old| old.address == key.address)
                    .unwrap()
            })
            .collect();
        for balances in [
            &mut input.record.transaction.pre_balances,
            &mut input.record.transaction.post_balances,
        ]
        .into_iter()
        .flatten()
        {
            let old = balances.clone();
            *balances = order.iter().map(|index| old[*index]).collect();
        }
        for balances in [
            &mut input.record.transaction.pre_token_balances,
            &mut input.record.transaction.post_token_balances,
        ]
        .into_iter()
        .flatten()
        {
            for balance in balances {
                balance.account_index = order
                    .iter()
                    .position(|index| *index == balance.account_index)
                    .unwrap();
            }
        }
        for ix in input
            .record
            .transaction
            .instructions
            .iter_mut()
            .chain(&mut input.record.transaction.inner_instructions)
        {
            for meta in &mut ix.accounts {
                if meta.address == manager {
                    meta.is_writable = false;
                }
            }
        }
        let input = controlled(input);
        let spec = declaration(
            &input,
            RationalFee {
                numerator: 1,
                denominator: 100,
            },
        );
        let r = analyze(&spec, &input).unwrap();
        assert_eq!(
            r["status"], "semantic_consequence_observed",
            "baseline={} proposed={}",
            r["baseline"]["reconciliation_error"], r["proposed"]["reconciliation_error"]
        );
        assert_eq!(r["proposed"]["execution"]["success"], false);
        assert_eq!(r["proposed"]["reconciliation"]["rollback_verified"], true);
        assert_eq!(
            r["findings"][0]["fingerprint"],
            "spl-stake-pool/deposit_sol/execution/transaction/now_reverts"
        );
        assert_eq!(r["findings"].as_array().unwrap().len(), 1);
        reproduce(&spec, &r).unwrap();
    }
}
