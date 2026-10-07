//! Bounded program-upgrade rollout rehearsal.
//!
//! One question: does changing the order of an actual installed loader-v3
//! program upgrade and the qualified Stake Pool `SetFee(SolDeposit)` change the
//! final retained `DepositSol`? Five fixed scenarios each start from the same
//! restored S0. Within a scenario every successful step hands its byte-bearing
//! world to the next; across scenarios nothing is carried.
//!
//! The upgrade is the loader's own `Upgrade` instruction over seeded Program,
//! ProgramData and Buffer accounts, never an executable swap. LiteSVM 0.16
//! makes an upgraded program callable in the same slot (its post-transaction
//! account sync reloads the Program with `effective_slot = slot`), so the
//! rollout model itself refuses any program-invoking step before
//! `deploy_slot + DELAY_VISIBILITY_SLOT_OFFSET` and advances only `Clock.slot`,
//! explicitly, to cross that boundary.
//!
//! The report is a pure reduction of retained executions: verification repeats
//! the reduction without a VM, reproduction re-executes every step.
pub mod artifact;
pub mod world;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_message::Message;

pub use solana_program_runtime::program_cache_entry::DELAY_VISIBILITY_SLOT_OFFSET;
use world::{AccountStore, Entry, Execution, State, STATE_VERSION};

use crate::{
    canonical,
    change::{ChangeSpec, ExecutableArtifact},
    executor::{
        CpiCall, ExecutionResult, LoadedProgram, ProbeInnerInstruction, ProbeTransactionExecution,
    },
    parameter_change::stake_pool as s,
    path::ProbeMessage,
    protocol::stake_pool as adapter,
    replay::{self, AccountStateSource, ReplayClock, ReplayFidelity, ReplayRecord},
    standard_programs::upgradeable_loader as loader,
    types::AccountSnapshot,
};

pub const INPUT_VERSION: &str = "eplyx-rollout-rehearsal-input-v1";
pub const REVISION: &str = "eplyx-rollout-rehearsal-v1";
pub const REPORT_SCHEMA: &str = "eplyx-rollout-rehearsal-report-v1";
pub const SCENARIO_VERSION: &str = "eplyx-rollout-scenario-v1";
/// The constructed rollout counterexample (programs/fixture-stake-pool-rollout-
/// candidate). Not an upstream release; qualified only for this contract.
pub const CANDIDATE_SHA256: &str =
    "64612be0d9dde5cb4f24d1542572f56ff59b1fa66552329f71227dcfd329c019";
pub const CANDIDATE_LEN: u64 = 134_320;
pub const CANDIDATE_PROFILE: &str = "constructed-rollout-stricter-sol-deposit-fee-v1";
pub const SAME_CODE_PROFILE: &str = "same-code-historical-v1";
const TOKEN_SHA: &str = "8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697";
/// Declared mainnet-beta EpochSchedule: fixed-length epochs, no warmup. The
/// retained schema-1 Clock carries epoch 0 and is never rewritten; this is the
/// schedule against which a slot transition is refused if it would cross into
/// another epoch.
pub const MAINNET_SLOTS_PER_EPOCH: u64 = 432_000;
const LIMITS: &[&str] = &[
    "Bounded rollout rehearsal under one retained Stake Pool world and explicit signer assumptions. Not a mainnet deployment, governance approval, proof that any authority possesses keys, exact validator-bank equivalence, generic transaction sequencing or complete protocol release safety.",
    "The candidate is a constructed rollout counterexample, not an upstream release and not intended for deployment.",
    "S0's Program and ProgramData envelopes are reconstructed from retained evidence (program ID, loader, deployment slot, ELF bytes with padding); their lamports are the default rent-exempt minimum and their upgrade authority is a simulation key. The Buffer holding the candidate is assumed to have been written beforehand.",
    "Signature verification and recent-blockhash age are disabled locally; signer privileges are checked by the loader and the program. The upgrade authority, upgrade payer, manager and configuration payer are simulation assumptions.",
    "Only Clock.slot advances, by exactly the loader's visibility offset; Clock.epoch, timestamps and other sysvars keep the retained schema-1 values. No ExtendProgram, Squads, governance, migration or other rollout step is executed.",
    "Configuration and upgrade transaction fees are excluded from user-action economics. Aliased token roles are counted once. Read-only verification establishes internal consistency, not independent proof that a VM ran; reproduction reruns the VM.",
];
const METRICS: &[&str] = &[
    "recipient_account_credit_raw",
    "manager_fee_account_credit_raw",
    "referral_account_credit_raw",
    "mint_supply_delta_raw",
    "pool_token_supply_delta_raw",
    "reserve_lamport_delta",
    "pool_total_lamports_delta",
    "funding_payer_lamport_debit",
    "funding_payer_debit_excluding_transaction_fee",
    "action_transaction_fee_lamports",
];

fn simulated(byte: u8) -> String {
    bs58::encode([byte; 32]).into_string()
}
/// Assumed upgrade authority written into S0's ProgramData and the Buffer.
pub fn upgrade_authority() -> String {
    simulated(81)
}
/// Assumed upgrade fee payer; also the spill account that receives the
/// closed Buffer's lamports.
pub fn upgrade_payer() -> String {
    simulated(82)
}
/// Address of the assumed pre-written Buffer holding the candidate.
pub fn buffer_address() -> String {
    simulated(83)
}

// ---------------------------------------------------------------------------
// Input and admission
// ---------------------------------------------------------------------------

/// Everything the analysis reads. Proposals keep their own identities; this
/// input binds both plus the retained world and candidate bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub upgrade: ChangeSpec,
    pub parameter: ChangeSpec,
    pub historical: s::Input,
    pub candidate: s::ProgramEvidence,
}

impl Input {
    pub fn new(
        upgrade: &ChangeSpec,
        parameter: &ChangeSpec,
        historical: s::Input,
        candidate: Vec<u8>,
    ) -> Result<Self> {
        let normalize = |spec: &ChangeSpec| -> Result<ChangeSpec> {
            spec.validate()?;
            let mut copy = spec.clone();
            copy.change_spec_id = Some(spec.id()?);
            Ok(copy)
        };
        let u = upgrade
            .as_program_upgrade()
            .context("a program_upgrade ChangeSpec is required")?;
        ensure!(
            *u.candidate == ExecutableArtifact::of(&candidate),
            "candidate_identity_mismatch: the supplied bytes ({}, {} bytes) are not the upgrade ChangeSpec's candidate ({}, {} bytes)",
            replay::hash_bytes(&candidate),
            candidate.len(),
            u.candidate.sha256,
            u.candidate.len
        );
        ensure!(
            parameter.as_program_upgrade().is_none(),
            "a protocol_parameter_change ChangeSpec is required"
        );
        Ok(Self {
            upgrade: normalize(upgrade)?,
            parameter: normalize(parameter)?,
            candidate: s::ProgramEvidence {
                program_id: historical.record.program_id.clone(),
                loader: crate::versions::UPGRADEABLE_LOADER_ID.into(),
                elf_sha256: replay::hash_bytes(&candidate),
                elf: candidate,
            },
            historical,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    DepositSol,
    SetFee,
    Upgrade,
    AdvanceToVisibleSlot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioName {
    Control,
    UpgradeControl,
    ConfigControl,
    OrderA,
    OrderB,
    /// Only through [`Prepared::run_sequence`]: a bounded sequence of the same
    /// supported step kinds, used to examine the model's own guards.
    Probe,
}

impl ScenarioName {
    pub fn steps(self) -> &'static [StepKind] {
        use StepKind::*;
        match self {
            Self::Control => &[DepositSol],
            Self::UpgradeControl => &[Upgrade, AdvanceToVisibleSlot, DepositSol],
            Self::ConfigControl => &[SetFee, DepositSol],
            Self::OrderA => &[Upgrade, AdvanceToVisibleSlot, SetFee, DepositSol],
            Self::OrderB => &[SetFee, Upgrade, AdvanceToVisibleSlot, DepositSol],
            Self::Probe => &[],
        }
    }
}

pub const SCENARIOS: [ScenarioName; 5] = [
    ScenarioName::Control,
    ScenarioName::UpgradeControl,
    ScenarioName::ConfigControl,
    ScenarioName::OrderA,
    ScenarioName::OrderB,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capacity {
    pub programdata_address: String,
    pub programdata_account_sha256: String,
    pub programdata_data_len: u64,
    pub capacity_bytes: u64,
    pub required_bytes: u64,
    pub sufficient: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocker {
    pub reason: String,
    pub detail: String,
}

/// Facts derived from inputs alone, before any VM runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preflight {
    pub capacity: Capacity,
    pub candidate_profile: Option<String>,
    pub upgrade_blocker: Option<Blocker>,
}

/// Admitted input, the S0 world and the three supported messages.
pub struct Prepared {
    pub input: Input,
    pub contract: Value,
    pub analysis_input_id: String,
    pub s0: State,
    pub s0_accounts: AccountStore,
    pub preflight: Preflight,
    plan: s::ConfigPlan,
    dependencies: Vec<LoadedProgram>,
    upgrade_message: Message,
    deposit_message: Message,
    v1: Vec<u8>,
}

fn runtime() -> Value {
    json!({
        "backend": "LiteSVM 0.16",
        "profile": "LiteSVM::new mainnet features; sigverify=false; blockhash_check=false; fresh VM per step",
        "revision": REVISION,
        "shared_parameter_runtime": s::runtime(),
        "rollout_source_sha256": replay::hash_bytes(include_bytes!("mod.rs")),
        "world_source_sha256": replay::hash_bytes(include_bytes!("world.rs")),
    })
}

pub fn visibility_contract() -> Value {
    json!({
        "rule": "an upgrade executed at slot N is callable from slot N + DELAY_VISIBILITY_SLOT_OFFSET",
        "delay_visibility_slot_offset": DELAY_VISIBILITY_SLOT_OFFSET,
        "backend_behaviour": "LiteSVM 0.16 reloads the upgraded Program at effective_slot = N after the transaction, i.e. immediately; the rollout model compensates by refusing any target-program step while clock.slot < deploy_slot + offset",
        "transition": "advance_to_visible_slot changes Clock.slot only; Clock.epoch, epoch_start_timestamp, leader_schedule_epoch and unix_timestamp keep their retained values",
        "epoch_schedule": {"slots_per_epoch": MAINNET_SLOTS_PER_EPOCH, "warmup": false, "source": "declared mainnet-beta schedule"},
        "epoch_boundary": "a transition whose source and target slots lie in different epochs is refused as unsupported_rollout_clock_transition",
    })
}

/// The explicit next-slot transition. Pure; refused rather than approximated.
pub fn visibility_transition(
    clock: &ReplayClock,
    deploy_slot: u64,
) -> std::result::Result<ReplayClock, Blocker> {
    let target = deploy_slot
        .checked_add(DELAY_VISIBILITY_SLOT_OFFSET)
        .ok_or_else(|| Blocker {
            reason: "unsupported_rollout_clock_transition".into(),
            detail: "visible slot overflows".into(),
        })?;
    if target <= clock.slot {
        return Err(Blocker {
            reason: "unsupported_rollout_clock_transition".into(),
            detail: format!(
                "no pending visibility boundary: deploy slot {deploy_slot} is already visible at slot {}",
                clock.slot
            ),
        });
    }
    if clock.slot / MAINNET_SLOTS_PER_EPOCH != target / MAINNET_SLOTS_PER_EPOCH {
        return Err(Blocker {
            reason: "unsupported_rollout_clock_transition".into(),
            detail: format!(
                "advancing from slot {} to {target} crosses a mainnet epoch boundary ({} -> {}); epoch-dependent runtime state is not modelled",
                clock.slot,
                clock.slot / MAINNET_SLOTS_PER_EPOCH,
                target / MAINNET_SLOTS_PER_EPOCH
            ),
        });
    }
    let mut next = clock.clone();
    next.slot = target;
    Ok(next)
}

fn rent_exempt(len: usize) -> u64 {
    solana_rent::Rent::default().minimum_balance(len).max(1)
}

fn system_account(lamports: u64, rent_epoch: u64) -> AccountSnapshot {
    AccountSnapshot {
        lamports,
        owner: adapter::SYSTEM_PROGRAM_ID.into(),
        data: vec![],
        executable: false,
        rent_epoch,
    }
}

fn upgrade_instruction(program_id: &str, authority: &str, signs: bool) -> Result<Instruction> {
    let program: Address = program_id.parse()?;
    let mut accounts = vec![AccountMeta::new_readonly(Address::default(), false); 7];
    accounts[loader::UPGRADE_PROGRAMDATA] =
        AccountMeta::new(loader::programdata_address(&program), false);
    accounts[loader::UPGRADE_PROGRAM] = AccountMeta::new(program, false);
    accounts[loader::UPGRADE_BUFFER] = AccountMeta::new(buffer_address().parse()?, false);
    accounts[loader::UPGRADE_SPILL] = AccountMeta::new(upgrade_payer().parse()?, false);
    accounts[loader::UPGRADE_RENT] = AccountMeta::new_readonly(loader::rent_sysvar(), false);
    accounts[loader::UPGRADE_CLOCK] = AccountMeta::new_readonly(loader::clock_sysvar(), false);
    accounts[loader::UPGRADE_AUTHORITY] = AccountMeta::new_readonly(authority.parse()?, signs);
    Ok(Instruction {
        program_id: loader::id(),
        accounts,
        // The legacy four-byte form: closes the Buffer on every runtime.
        data: loader::encode::upgrade(),
    })
}

fn config_instruction(input: &Input, manager: &str, signs: bool) -> Result<Instruction> {
    let (_, fee) = s::values(&input.parameter)?;
    let pool = s::pool(&input.historical.record)?;
    let official = spl_stake_pool::instruction::set_fee(
        &spl_stake_pool::id(),
        &pool.address.parse()?,
        &manager.parse()?,
        spl_stake_pool::state::FeeType::SolDeposit(fee.into()),
    );
    Ok(Instruction {
        program_id: official.program_id.to_string().parse()?,
        accounts: official
            .accounts
            .iter()
            .map(|meta| {
                Ok(AccountMeta {
                    pubkey: meta.pubkey.to_string().parse()?,
                    is_signer: meta.is_signer && (signs || meta.pubkey.to_string() != manager),
                    is_writable: meta.is_writable,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        data: official.data,
    })
}

impl Prepared {
    /// Strict admission. Unsupported proposal shapes and unqualified retained
    /// scope are errors; an upgrade that cannot be installed (capacity, an
    /// unqualified candidate) is a typed blocker the report carries.
    pub fn new(input: Input) -> Result<Self> {
        ensure!(
            canonical::document(&input)?.len() as u64 <= crate::lifecycle::artifact::MAX_BYTES,
            "rollout input exceeds local byte bound"
        );
        ensure!(
            input.candidate.elf.len() as u64 <= crate::migration::input::MAX_PROGRAM_BYTES
                && input
                    .historical
                    .programs
                    .iter()
                    .all(|p| p.elf.len() as u64 <= crate::migration::input::MAX_PROGRAM_BYTES),
            "program exceeds existing local executable bound"
        );
        ensure!(
            input.upgrade.to_document()?.len() as u64 <= crate::migration::input::MAX_CHANGE_BYTES
                && input.parameter.to_document()?.len() as u64
                    <= crate::migration::input::MAX_CHANGE_BYTES,
            "proposal exceeds existing local change bound"
        );
        input.upgrade.validate()?;
        input.parameter.validate()?;
        let u = input
            .upgrade
            .as_program_upgrade()
            .context("a program_upgrade ChangeSpec is required")?;
        ensure!(
            input.upgrade.activation.is_none(),
            "unsupported_upgrade_expectation: activation timing is not evaluated by this rehearsal"
        );
        ensure!(
            u.delivery.is_none(),
            "unsupported_upgrade_expectation: governance delivery (Squads) is not executed by this rehearsal"
        );
        ensure!(
            u.expected_upgrade_authority.is_none(),
            "unsupported_upgrade_expectation: the rehearsal's upgrade authority is a simulation assumption and cannot prove a stated authority"
        );
        let record = &input.historical.record;
        input.historical.validate()?;
        ensure!(
            u.target.program_id == record.program_id,
            "upgrade target differs from the retained program"
        );
        let program: Address = record.program_id.parse()?;
        let programdata = loader::programdata_address(&program).to_string();
        if let Some(stated) = &u.target.programdata_address {
            ensure!(
                *stated == programdata,
                "upgrade ChangeSpec ProgramData differs from the loader derivation"
            );
        }
        ensure!(
            input.candidate.program_id == record.program_id
                && input.candidate.loader == crate::versions::UPGRADEABLE_LOADER_ID
                && input.candidate.elf_sha256 == replay::hash_bytes(&input.candidate.elf)
                && *u.candidate == ExecutableArtifact::of(&input.candidate.elf),
            "candidate_identity_mismatch: candidate bytes/hash/length/loader differ from the upgrade ChangeSpec"
        );
        let v1 = input
            .historical
            .programs
            .iter()
            .find(|p| p.program_id == record.program_id)
            .context("historical V1 bytes absent")?;
        if let Some(replaces) = u.replaces {
            ensure!(
                *replaces == ExecutableArtifact::of(&v1.elf),
                "upgrade replaces differs from retained V1"
            );
        }
        let plan = s::prepare(&input.parameter, &input.historical)
            .map_err(|e| anyhow::anyhow!("{:?}: {}", e.status, e.detail))?;
        let (pool, _) = s::decode(&s::pool(record)?.account)?;
        ensure!(
            pool.sol_referral_fee == 0
                && pool.sol_deposit_authority.is_none()
                && pool.token_program_id.to_string() == adapter::TOKEN_PROGRAM_ID
                && record.clock.epoch == 0,
            "unqualified scope: requires zero referral, ungated legacy Token DepositSol and fixed epoch zero"
        );
        let allowed_dependencies = [
            adapter::PROGRAM_ID,
            adapter::TOKEN_PROGRAM_ID,
            adapter::SYSTEM_PROGRAM_ID,
            adapter::COMPUTE_BUDGET_PROGRAM_ID,
        ];
        ensure!(
            record.dependencies.programs.len() == 4
                && record
                    .dependencies
                    .programs
                    .iter()
                    .all(|p| allowed_dependencies.contains(&p.program_id.as_str()))
                && input.historical.programs.len() == 2,
            "unqualified dependency set: only reviewed target, Token, System and ComputeBudget"
        );
        let token = input
            .historical
            .programs
            .iter()
            .find(|p| p.program_id == adapter::TOKEN_PROGRAM_ID)
            .context("pinned Token dependency absent")?;
        ensure!(
            token.elf_sha256 == TOKEN_SHA && token.loader == crate::versions::UPGRADEABLE_LOADER_ID,
            "unqualified pinned Token dependency"
        );
        let deploy_slot = record
            .dependencies
            .get(&record.program_id)
            .and_then(|d| d.deployed_slot)
            .context("retained V1 deployment slot absent")?;
        ensure!(
            deploy_slot < record.clock.slot,
            "retained V1 deployment slot is not before the retained Clock"
        );
        let dependencies: Vec<LoadedProgram> = input
            .historical
            .loaded()?
            .into_iter()
            .filter(|p| p.program_id != program)
            .collect();
        let deposit_message = record.message()?;
        let upgrade_message = Message::new(
            &[upgrade_instruction(
                &record.program_id,
                &upgrade_authority(),
                true,
            )?],
            Some(&upgrade_payer().parse()?),
        );

        // S0: retained record accounts and absences, the configuration plan's
        // assumed manager and payer, and the installed-program envelope.
        let mut store = AccountStore::new();
        let mut accounts = BTreeMap::new();
        for a in &plan.accounts {
            ensure!(
                accounts
                    .insert(
                        a.address.clone(),
                        world::put(&mut store, Some(a.account.clone()))?
                    )
                    .is_none(),
                "duplicate S0 account {}",
                a.address
            );
        }
        for a in &record.acquisitions {
            if a.source == AccountStateSource::AbsentAtBothBoundaries {
                ensure!(
                    accounts
                        .insert(a.address.clone(), Entry::KnownAbsent)
                        .is_none(),
                    "retained absence collides with a present account"
                );
            }
        }
        let authority: Address = upgrade_authority().parse()?;
        let pd_data = loader::encode::programdata(deploy_slot, Some(authority), &v1.elf);
        let program_data = loader::encode::program(&programdata.parse()?);
        let buffer_data = loader::encode::buffer(Some(authority), &input.candidate.elf);
        let installation = [
            (
                programdata.clone(),
                AccountSnapshot {
                    lamports: rent_exempt(pd_data.len()),
                    owner: loader::id().to_string(),
                    data: pd_data,
                    executable: false,
                    rent_epoch: u64::MAX,
                },
            ),
            (
                record.program_id.clone(),
                AccountSnapshot {
                    lamports: rent_exempt(program_data.len()),
                    owner: loader::id().to_string(),
                    data: program_data,
                    executable: true,
                    rent_epoch: u64::MAX,
                },
            ),
            (
                buffer_address(),
                AccountSnapshot {
                    lamports: rent_exempt(buffer_data.len()),
                    owner: loader::id().to_string(),
                    data: buffer_data,
                    executable: false,
                    rent_epoch: u64::MAX,
                },
            ),
            (upgrade_authority(), system_account(1_000_000, 0)),
            (upgrade_payer(), system_account(10_000_000, u64::MAX)),
        ];
        for (address, account) in installation {
            ensure!(
                accounts
                    .insert(address.clone(), world::put(&mut store, Some(account))?)
                    .is_none(),
                "simulation installation account {address} collides with retained state"
            );
        }
        let s0 = State {
            version: STATE_VERSION.into(),
            clock: record.clock.clone(),
            accounts,
        };
        let pd_account = s0
            .account(&store, &programdata)?
            .context("S0 ProgramData")?
            .clone();
        let capacity = Capacity {
            programdata_address: programdata.clone(),
            programdata_account_sha256: world::account_id(&pd_account)?,
            programdata_data_len: pd_account.data.len() as u64,
            capacity_bytes: (pd_account.data.len()
                - solana_loader_v3_interface::state::UpgradeableLoaderState::size_of_programdata_metadata())
                as u64,
            required_bytes: input.candidate.elf.len() as u64,
            sufficient: false,
        };
        let capacity = Capacity {
            sufficient: capacity.required_bytes <= capacity.capacity_bytes,
            ..capacity
        };
        let candidate_profile = if input.candidate.elf_sha256 == record.current_program_sha256 {
            Some(SAME_CODE_PROFILE.to_string())
        } else if input.candidate.elf_sha256 == CANDIDATE_SHA256
            && input.candidate.elf.len() as u64 == CANDIDATE_LEN
        {
            Some(CANDIDATE_PROFILE.to_string())
        } else {
            None
        };
        let upgrade_blocker = if !capacity.sufficient {
            Some(Blocker {
                reason: "programdata_capacity_insufficient".into(),
                detail: format!(
                    "candidate needs {} bytes; ProgramData {} holds {} bytes after its header. ExtendProgram must run first; this rehearsal never resizes ProgramData.",
                    capacity.required_bytes, capacity.programdata_address, capacity.capacity_bytes
                ),
            })
        } else if candidate_profile.is_none() {
            Some(Blocker {
                reason: "candidate_not_qualified".into(),
                detail: "no compiled rollout profile admits this executable; it is not installed or executed".into(),
            })
        } else {
            None
        };
        let preflight = Preflight {
            capacity,
            candidate_profile,
            upgrade_blocker,
        };
        let mut prepared = Self {
            contract: Value::Null,
            analysis_input_id: String::new(),
            s0,
            s0_accounts: store,
            preflight,
            plan,
            dependencies,
            upgrade_message,
            deposit_message,
            v1: v1.elf.clone(),
            input,
        };
        for message in [
            &prepared.deposit_message,
            &prepared.plan.message,
            &prepared.upgrade_message,
        ] {
            prepared.keys_known(&prepared.s0, message)?;
        }
        prepared.contract = prepared.build_contract()?;
        prepared.analysis_input_id = canonical::digest(&(INPUT_VERSION, &prepared.contract))?;
        Ok(prepared)
    }

    fn build_contract(&self) -> Result<Value> {
        let record = &self.input.historical.record;
        let account_sha = |address: &str| -> Result<Value> {
            Ok(match self.s0.accounts.get(address) {
                Some(Entry::Present { account_sha256 }) => json!(account_sha256),
                _ => Value::Null,
            })
        };
        let mut upgrade = self.input.upgrade.clone();
        upgrade.metadata = Default::default();
        let mut parameter = self.input.parameter.clone();
        parameter.metadata = Default::default();
        Ok(json!({
            "version": INPUT_VERSION,
            "execution_revision": REVISION,
            "question": QUESTION,
            "upgrade_change_spec_id": self.input.upgrade.id()?,
            "parameter_change_spec_id": self.input.parameter.id()?,
            "proposals": {"upgrade": upgrade, "parameter": parameter},
            "retained_record": {
                "id": record.id,
                "record_sha256": self.input.historical.record_sha256,
                "source_bundle_sha256": self.input.historical.source_bundle_sha256,
                "signature": record.transaction.signature,
                "slot": record.transaction.slot,
            },
            "target_program_id": record.program_id,
            "s0_state_id": self.s0.id()?,
            "installation": {
                "loader": loader::id().to_string(),
                "program_account_sha256": account_sha(&record.program_id)?,
                "programdata_address": self.preflight.capacity.programdata_address,
                "programdata_account_sha256": self.preflight.capacity.programdata_account_sha256,
                "programdata_capacity_bytes": self.preflight.capacity.capacity_bytes,
                "installed_v1": ExecutableArtifact::of(&self.v1),
                "deploy_slot_origin": "retained dependency manifest deployed_slot",
                "envelope_origin": "reconstructed: rent-exempt default lamports, simulation upgrade authority",
            },
            "candidate": {
                "artifact": ExecutableArtifact::of(&self.input.candidate.elf),
                "profile": self.preflight.candidate_profile,
                "origin": "constructed rollout counterexample; not upstream release; not intended for deployment",
            },
            "buffer": {"address": buffer_address(), "account_sha256": account_sha(&buffer_address())?, "authority": upgrade_authority()},
            "config_account": s::pool(record)?.address,
            "configuration": s::config_commitment(&self.input.historical, &self.plan),
            "action": s::action_commitment(&self.input.historical, record)?,
            "upgrade_message": ProbeMessage::from(&self.upgrade_message),
            "dependencies": self.dependencies.iter().map(|p| json!({"program_id": p.program_id.to_string(), "loader": p.loader.to_string(), "elf": ExecutableArtifact::of(&p.bytes)})).collect::<Vec<_>>(),
            "runtime": runtime(),
            "clock": record.clock,
            "visibility": visibility_contract(),
            "assumptions": assumptions(),
            "scenarios": SCENARIOS.iter().map(|n| json!({"name": n, "steps": n.steps()})).collect::<Vec<_>>(),
            "closure": self.s0.closure(),
            "metrics": METRICS,
        }))
    }

    /// Every message key is a declared world account, a pinned dependency, a
    /// builtin or a runtime sysvar. Nothing else may supply state.
    fn keys_known(&self, state: &State, message: &Message) -> Result<()> {
        let allowed: BTreeSet<String> = self
            .dependencies
            .iter()
            .map(|p| p.program_id.to_string())
            .chain(
                [
                    adapter::SYSTEM_PROGRAM_ID,
                    adapter::COMPUTE_BUDGET_PROGRAM_ID,
                    crate::versions::UPGRADEABLE_LOADER_ID,
                    adapter::CLOCK_SYSVAR_ID,
                    "SysvarRent111111111111111111111111111111111",
                ]
                .map(String::from),
            )
            .collect();
        for key in &message.account_keys {
            let key = key.to_string();
            ensure!(
                state.accounts.contains_key(&key) || allowed.contains(&key),
                "message key {key} has no declared state, absence proof or pinned program"
            );
        }
        Ok(())
    }

    pub fn upgrade_message(&self) -> &Message {
        &self.upgrade_message
    }
    pub fn deposit_message(&self) -> &Message {
        &self.deposit_message
    }
    pub fn config_message(&self) -> &Message {
        &self.plan.message
    }
    pub fn dependencies(&self) -> &[LoadedProgram] {
        &self.dependencies
    }

    /// Run a bounded sequence of supported step kinds from S0 in fresh VMs.
    /// The five scenarios are the only sequences analysis declares; this
    /// exists to examine the model's own guards (e.g. same-slot visibility).
    pub fn run_sequence(&self, steps: &[StepKind]) -> Result<(ScenarioReport, Evidence)> {
        let mut source = VmSource { prepared: self };
        let mut runner = Runner::new(self, &mut source);
        let scenario = runner.scenario(ScenarioName::Probe, steps, None)?;
        Ok((scenario, runner.evidence()))
    }

    /// One step over an arbitrary closure-compatible state, optionally with a
    /// variant signer boundary. For examining rejections; never part of a report.
    pub fn probe(
        &self,
        kind: StepKind,
        state: &State,
        accounts: &AccountStore,
        variant: Variant,
    ) -> Result<(StepRecord, Option<State>, Evidence)> {
        let mut source = VmSource { prepared: self };
        let mut runner = Runner::new(self, &mut source);
        let mut state = state.clone();
        runner.accounts.extend(accounts.clone());
        if let Variant::Signer { address, .. } = &variant {
            state
                .accounts
                .entry(address.clone())
                .or_insert(Entry::KnownAbsent);
        }
        let (record, next) = runner.step(ScenarioName::Probe, 0, kind, &state, &variant)?;
        Ok((record, next, runner.evidence()))
    }
}

/// A different signer for Upgrade (authority) or SetFee (manager), and
/// whether that key signs. `Declared` is the analysed message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Variant {
    Declared,
    Signer { address: String, signs: bool },
}

pub const QUESTION: &str = "Does changing the order of an actual installed program upgrade and the qualified SetFee(SolDeposit) configuration change alter the final retained DepositSol outcome?";

fn assumptions() -> Value {
    json!([
        {"id": "upgrade_authority_assumed", "address": upgrade_authority(), "origin": "assumed_simulation_only", "signer": true, "observed": false, "key_possession_established": false, "boundary": "Written into S0's ProgramData and Buffer headers; the actual mainnet upgrade authority is not observed here."},
        {"id": "upgrade_payer_assumed", "address": upgrade_payer(), "origin": "assumed_simulation_only", "role": "upgrade fee payer and spill recipient", "propagated_to_user_action": false},
        {"id": "buffer_prewritten", "address": buffer_address(), "origin": "assumed_simulation_only", "boundary": "Buffer holds exactly the candidate bytes under the assumed authority; its Write history is not executed."},
        {"id": "manager_signer_assumed", "origin": "assumed_simulation_only", "boundary": "Reused unchanged from the qualified SetFee(SolDeposit) contract; key possession not established."},
        {"id": "configuration_payer_assumed", "origin": "assumed_simulation_only", "propagated_to_user_action": false},
    ])
}

// ---------------------------------------------------------------------------
// Report types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Verified,
    Rejected,
    Unsupported,
    EvidenceGap,
    HandoffFailure,
    UnexpectedWrite,
    ReconciliationFailed,
    ExecutionUnavailable,
    NotExecuted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledProgram {
    pub program_id: String,
    pub programdata_address: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub deploy_slot: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub visible_from_slot: u64,
    pub upgrade_authority: Option<String>,
    pub capacity_bytes: u64,
    /// SHA-256 of every byte after the header: what the loader hands the VM.
    pub executable_sha256: String,
    /// `historical_v1`, `candidate` (exact prefix, all-zero padding) or
    /// `unrecognized`.
    pub identity: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepRecord {
    pub index: usize,
    pub step: StepKind,
    pub outcome: StepOutcome,
    pub reason: Option<String>,
    pub detail: Option<String>,
    pub before_state_id: Option<String>,
    pub execution_id: Option<String>,
    pub after_state_id: Option<String>,
    pub clock_before: Option<ReplayClock>,
    pub clock_after: Option<ReplayClock>,
    pub installed_program_after: Option<InstalledProgram>,
    pub derived: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioReport {
    pub name: ScenarioName,
    pub scenario_id: String,
    pub declared_steps: Vec<StepKind>,
    pub initial_state_id: String,
    pub final_state_id: String,
    pub completed: bool,
    pub steps: Vec<StepRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorStatus {
    Matched,
    Failed,
    NotEstablished,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub status: AnchorStatus,
    pub reason: Option<String>,
    pub detail: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchors {
    pub baseline_world_fidelity: Anchor,
    pub installed_overlay: Anchor,
    /// The existing candidate-overlay execution (fresh bank, executable
    /// added directly) of the same V2 bytes over S0's retained action.
    pub overlay_execution: Option<ExecutionResult>,
    pub overlay_unavailable: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonStatus {
    NoOrderEffectObserved,
    RolloutOrderEffectObserved,
    RolloutNotEstablished,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub value: Option<String>,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricContrast {
    pub order_a: Quantity,
    pub order_b: Quantity,
    pub differs: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepContrast {
    pub step: StepKind,
    pub order_a_index: usize,
    pub order_b_index: usize,
    pub order_a: StepOutcome,
    pub order_b: StepOutcome,
    /// Outcome or the step's established result (installed executable,
    /// verified pool, reconciled action) differs.
    pub differs: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comparison {
    pub status: ComparisonStatus,
    pub finding: Option<String>,
    pub statement: String,
    pub order_scenario_ids: [String; 2],
    pub all_steps_executed: BTreeMap<String, bool>,
    pub final_state_ids: [String; 2],
    pub final_states_differ: bool,
    pub final_state_differences: BTreeSet<String>,
    pub step_contrasts: Vec<StepContrast>,
    pub first_divergence: Option<StepContrast>,
    pub final_action_outcomes: [StepOutcome; 2],
    pub final_action_executed_differently: Option<bool>,
    pub final_action_metrics: BTreeMap<String, MetricContrast>,
    pub established: Vec<String>,
    pub unavailable: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceIndex {
    pub states: BTreeSet<String>,
    pub executions: BTreeSet<String>,
    pub accounts: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: String,
    pub question: String,
    pub analysis_input_id: String,
    pub upgrade_change_spec_id: String,
    pub parameter_change_spec_id: String,
    pub preflight: Preflight,
    pub visibility: Value,
    pub assumptions: Value,
    pub anchors: Anchors,
    pub scenarios: Vec<ScenarioReport>,
    pub comparison: Comparison,
    pub limitations: Vec<String>,
    pub evidence: EvidenceIndex,
    pub report_sha256: String,
}

impl Report {
    pub fn scenario(&self, name: ScenarioName) -> Option<&ScenarioReport> {
        self.scenarios.iter().find(|s| s.name == name)
    }
}

/// Byte-bearing evidence, content addressed. Every report reference resolves
/// here and nothing else is retained.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    pub states: BTreeMap<String, State>,
    pub executions: BTreeMap<String, Execution>,
    pub accounts: AccountStore,
}

impl Evidence {
    pub fn index(&self) -> EvidenceIndex {
        EvidenceIndex {
            states: self.states.keys().cloned().collect(),
            executions: self.executions.keys().cloned().collect(),
            accounts: self.accounts.keys().cloned().collect(),
        }
    }
}

/// A completed analysis: the input it was derived from, the identity contract,
/// the sealed report and its evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analysis {
    pub input: Input,
    pub contract: Value,
    pub report: Report,
    pub evidence: Evidence,
}

// ---------------------------------------------------------------------------
// Execution sources
// ---------------------------------------------------------------------------

enum Attempt<T> {
    Executed(T),
    Unavailable(String),
}

trait Source {
    fn execute(
        &mut self,
        scenario: ScenarioName,
        index: usize,
        state: &State,
        accounts: &mut AccountStore,
        message: &Message,
    ) -> Option<Attempt<Execution>>;
    fn overlay(&mut self) -> Option<Attempt<ExecutionResult>>;
}

struct VmSource<'a> {
    prepared: &'a Prepared,
}

impl Source for VmSource<'_> {
    fn execute(
        &mut self,
        _: ScenarioName,
        _: usize,
        state: &State,
        accounts: &mut AccountStore,
        message: &Message,
    ) -> Option<Attempt<Execution>> {
        Some(
            match world::execute(state, accounts, &self.prepared.dependencies, message) {
                Ok(x) => Attempt::Executed(x),
                Err(e) => Attempt::Unavailable(format!("{e:#}")),
            },
        )
    }
    fn overlay(&mut self) -> Option<Attempt<ExecutionResult>> {
        let h = &self.prepared.input.historical;
        Some(
            match h.execute_code(&h.record, Some(&self.prepared.input.candidate.elf)) {
                Ok(x) => Attempt::Executed(x),
                Err(e) => Attempt::Unavailable(format!("{e:#}")),
            },
        )
    }
}

/// Retained executions only. A missing or inconsistent object makes the
/// reduction differ, which verification reports; nothing is repaired.
struct RetainedSource<'a> {
    report: &'a Report,
    evidence: &'a Evidence,
}

impl Source for RetainedSource<'_> {
    fn execute(
        &mut self,
        scenario: ScenarioName,
        index: usize,
        _: &State,
        accounts: &mut AccountStore,
        _: &Message,
    ) -> Option<Attempt<Execution>> {
        let step = self.report.scenario(scenario)?.steps.get(index)?;
        if let Some(id) = &step.execution_id {
            let x = self.evidence.executions.get(id)?.clone();
            for entry in x.post.values() {
                if let Entry::Present { account_sha256 } = entry {
                    accounts.insert(
                        account_sha256.clone(),
                        self.evidence.accounts.get(account_sha256)?.clone(),
                    );
                }
            }
            return Some(Attempt::Executed(x));
        }
        (step.outcome == StepOutcome::ExecutionUnavailable)
            .then(|| Attempt::Unavailable(step.detail.clone().unwrap_or_default()))
    }
    fn overlay(&mut self) -> Option<Attempt<ExecutionResult>> {
        let a = &self.report.anchors;
        match (&a.overlay_execution, &a.overlay_unavailable) {
            (Some(x), None) => Some(Attempt::Executed(x.clone())),
            (None, Some(e)) => Some(Attempt::Unavailable(e.clone())),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

struct Runner<'a> {
    p: &'a Prepared,
    source: &'a mut dyn Source,
    states: BTreeMap<String, State>,
    executions: BTreeMap<String, Execution>,
    accounts: AccountStore,
}

fn record(index: usize, step: StepKind, outcome: StepOutcome) -> StepRecord {
    StepRecord {
        index,
        step,
        outcome,
        reason: None,
        detail: None,
        before_state_id: None,
        execution_id: None,
        after_state_id: None,
        clock_before: None,
        clock_after: None,
        installed_program_after: None,
        derived: Value::Null,
    }
}

impl StepRecord {
    fn because(mut self, reason: &str, detail: impl ToString) -> Self {
        self.reason = Some(reason.into());
        self.detail = Some(detail.to_string());
        self
    }
}

pub fn installed_program(
    state: &State,
    accounts: &AccountStore,
    program_id: &str,
    v1: &[u8],
    candidate: &[u8],
) -> Result<InstalledProgram> {
    let program = state
        .account(accounts, program_id)?
        .context("target Program account absent")?;
    ensure!(
        program.owner == loader::id().to_string() && program.executable,
        "target Program is not an executable loader-v3 account"
    );
    let programdata_address = loader::decode_program(&program.data)?;
    let pd = state
        .account(accounts, &programdata_address.to_string())?
        .context("ProgramData absent")?;
    ensure!(
        pd.owner == loader::id().to_string() && !pd.executable,
        "ProgramData envelope is not loader-owned data"
    );
    let decoded = loader::decode_programdata(&pd.data)?;
    let bytes = &decoded.bytes;
    let identity = if bytes == v1 {
        "historical_v1"
    } else if bytes.len() >= candidate.len()
        && bytes[..candidate.len()] == *candidate
        && bytes[candidate.len()..].iter().all(|b| *b == 0)
    {
        "candidate"
    } else {
        "unrecognized"
    };
    Ok(InstalledProgram {
        program_id: program_id.into(),
        programdata_address: programdata_address.to_string(),
        deploy_slot: decoded.deploy_slot,
        visible_from_slot: decoded
            .deploy_slot
            .checked_add(DELAY_VISIBILITY_SLOT_OFFSET)
            .context("visible slot overflow")?,
        upgrade_authority: decoded.upgrade_authority.map(|a| a.to_string()),
        capacity_bytes: bytes.len() as u64,
        executable_sha256: replay::hash_bytes(bytes),
        identity: identity.into(),
    })
}

/// Convert a world execution to the action result the existing qualified
/// reconciliation and fidelity gates read, over the record's labelled accounts.
fn action_result(
    record: &ReplayRecord,
    x: &Execution,
    accounts: &AccountStore,
) -> Result<ExecutionResult> {
    let mut by_label = BTreeMap::new();
    for a in &record.accounts {
        if let Some(Entry::Present { account_sha256 }) = x.post.get(&a.address) {
            by_label.insert(
                a.label.clone(),
                accounts
                    .get(account_sha256)
                    .context("post account bytes not retained")?
                    .clone(),
            );
        }
    }
    let keys = &x.message.account_keys;
    let cpi_calls = x
        .inner_instructions
        .iter()
        .flat_map(|group| {
            group.instructions.iter().map(|inner| {
                let index = usize::from(inner.program_id_index);
                CpiCall {
                    program: keys
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("<unresolved index {index}>")),
                    stack_height: inner.stack_height,
                    outer_index: u8::try_from(group.outer_index).unwrap_or(u8::MAX),
                    account_count: u8::try_from(inner.accounts.len()).unwrap_or(u8::MAX),
                    data_len: u32::try_from(inner.data.len()).unwrap_or(u32::MAX),
                    discriminant: inner.data.first().copied(),
                }
            })
        })
        .collect();
    Ok(ExecutionResult {
        version: "rollout-installed-world".into(),
        success: x.success,
        error: x.error.clone(),
        compute_units: Some(x.compute_units),
        fee: x.transaction_fee_lamports,
        logs: x.logs.clone(),
        cpi_calls,
        accounts: by_label,
    })
}

/// The configuration contract's own execution shape over its watch set.
fn config_execution(
    plan: &s::ConfigPlan,
    x: &Execution,
    accounts: &AccountStore,
) -> Result<ProbeTransactionExecution> {
    let mut post_accounts = BTreeMap::new();
    for address in &plan.watch {
        if let Some(Entry::Present { account_sha256 }) = x.post.get(address) {
            post_accounts.insert(
                address.clone(),
                accounts
                    .get(account_sha256)
                    .context("post account bytes not retained")?
                    .clone(),
            );
        }
    }
    let keys = &x.message.account_keys;
    let mut inner_instructions = Vec::new();
    for group in &x.inner_instructions {
        for inner in &group.instructions {
            inner_instructions.push(ProbeInnerInstruction {
                program: keys
                    .get(usize::from(inner.program_id_index))
                    .context("CPI program index")?
                    .clone(),
                stack_height: inner.stack_height,
                accounts: inner
                    .accounts
                    .iter()
                    .map(|i| {
                        keys.get(usize::from(*i))
                            .cloned()
                            .context("CPI account index")
                    })
                    .collect::<Result<_>>()?,
                data: inner.data.clone(),
            });
        }
    }
    Ok(ProbeTransactionExecution {
        success: x.success,
        error: x.error.clone(),
        compute_units: x.compute_units,
        transaction_fee_lamports: x.transaction_fee_lamports,
        logs: x.logs.clone(),
        inner_instructions,
        post_accounts,
    })
}

/// A rejected transaction must leave every closure row unchanged except the
/// fee payer, debited exactly the reported fee.
fn verify_rollback(
    before: &State,
    after: &State,
    accounts: &AccountStore,
    payer: &str,
    fee: u64,
) -> Result<()> {
    let changed = world::changed(before, after);
    ensure!(
        changed.iter().all(|a| a == payer),
        "rejected transaction changed {changed:?}"
    );
    let pre = before.account(accounts, payer)?.context("payer absent")?;
    let post = after
        .account(accounts, payer)?
        .context("payer absent after")?;
    let mut expected = pre.clone();
    expected.lamports = pre.lamports.checked_sub(fee).context("fee exceeds payer")?;
    ensure!(
        *post == expected,
        "rejected transaction payer debit is not exactly its fee"
    );
    Ok(())
}

impl<'a> Runner<'a> {
    fn new(p: &'a Prepared, source: &'a mut dyn Source) -> Self {
        let mut states = BTreeMap::new();
        states.insert(p.s0.id().expect("S0 id"), p.s0.clone());
        Self {
            p,
            source,
            states,
            executions: BTreeMap::new(),
            accounts: p.s0_accounts.clone(),
        }
    }

    fn evidence(self) -> Evidence {
        Evidence {
            states: self.states,
            executions: self.executions,
            accounts: self.accounts,
        }
    }

    fn retain(&mut self, state: State) -> Result<String> {
        let id = state.id()?;
        self.states.insert(id.clone(), state);
        Ok(id)
    }

    fn installed(&self, state: &State) -> Result<InstalledProgram> {
        installed_program(
            state,
            &self.accounts,
            &self.p.input.historical.record.program_id,
            &self.p.v1,
            &self.p.input.candidate.elf,
        )
    }

    fn scenario(
        &mut self,
        name: ScenarioName,
        steps: &[StepKind],
        gate: Option<&str>,
    ) -> Result<ScenarioReport> {
        let initial = self.p.s0.clone();
        let initial_id = initial.id()?;
        let scenario_id =
            canonical::digest(&(SCENARIO_VERSION, &self.p.analysis_input_id, name, steps))?;
        let mut state = initial;
        let mut final_state_id = initial_id.clone();
        let mut blocked: Option<String> = gate.map(str::to_string);
        let mut records = Vec::new();
        for (index, kind) in steps.iter().enumerate() {
            if let Some(why) = &blocked {
                records.push(
                    record(index, *kind, StepOutcome::NotExecuted).because("not_executed", why),
                );
                continue;
            }
            let (r, next) = self.step(name, index, *kind, &state, &Variant::Declared)?;
            if let Some(id) = &r.after_state_id {
                final_state_id = id.clone();
            }
            match (r.outcome, next) {
                (StepOutcome::Verified, Some(next)) => state = next,
                (outcome, _) => {
                    blocked = Some(format!(
                        "prerequisite step {index} ({}) was {}",
                        serde_json::to_value(kind)?.as_str().unwrap_or_default(),
                        serde_json::to_value(outcome)?.as_str().unwrap_or_default()
                    ))
                }
            }
            records.push(r);
        }
        Ok(ScenarioReport {
            name,
            scenario_id,
            declared_steps: steps.to_vec(),
            initial_state_id: initial_id,
            final_state_id,
            completed: records.iter().all(|r| r.outcome == StepOutcome::Verified),
            steps: records,
        })
    }

    /// One step. `Ok` always carries a typed record; `Err` is reserved for a
    /// retained object that does not bind to the request (verification
    /// failure) or an internal invariant.
    fn step(
        &mut self,
        scenario: ScenarioName,
        index: usize,
        kind: StepKind,
        state: &State,
        variant: &Variant,
    ) -> Result<(StepRecord, Option<State>)> {
        let mut r = record(index, kind, StepOutcome::Verified);
        r.before_state_id = Some(state.id()?);
        r.clock_before = Some(state.clock.clone());
        let installed = self.installed(state);
        if kind == StepKind::AdvanceToVisibleSlot {
            let installed = match installed {
                Ok(i) => i,
                Err(e) => {
                    r.outcome = StepOutcome::EvidenceGap;
                    return Ok((r.because("installed_program_unreadable", e), None));
                }
            };
            return Ok(
                match visibility_transition(&state.clock, installed.deploy_slot) {
                    Ok(clock) => {
                        let next = State {
                            version: STATE_VERSION.into(),
                            clock: clock.clone(),
                            accounts: state.accounts.clone(),
                        };
                        r.after_state_id = Some(self.retain(next.clone())?);
                        r.clock_after = Some(clock.clone());
                        r.installed_program_after = Some(installed.clone());
                        r.derived = json!({
                            "transition": "advance_to_visible_slot",
                            "from_slot": state.clock.slot.to_string(),
                            "to_slot": clock.slot.to_string(),
                            "deploy_slot": installed.deploy_slot.to_string(),
                            "fields_changed": ["slot"],
                            "mainnet_epoch": (clock.slot / MAINNET_SLOTS_PER_EPOCH).to_string(),
                            "accounts_unchanged": true,
                        });
                        (r, Some(next))
                    }
                    Err(b) => {
                        r.outcome = StepOutcome::Unsupported;
                        (r.because(&b.reason, b.detail), None)
                    }
                },
            );
        }
        let installed = match installed {
            Ok(i) => i,
            Err(e) => {
                r.outcome = StepOutcome::EvidenceGap;
                return Ok((r.because("installed_program_unreadable", e), None));
            }
        };
        let message = match (kind, variant) {
            (StepKind::Upgrade, Variant::Declared) => self.p.upgrade_message.clone(),
            (StepKind::Upgrade, Variant::Signer { address, signs }) => Message::new(
                &[upgrade_instruction(
                    &self.p.input.historical.record.program_id,
                    address,
                    *signs,
                )?],
                Some(&upgrade_payer().parse()?),
            ),
            (StepKind::SetFee, Variant::Declared) => self.p.plan.message.clone(),
            (StepKind::SetFee, Variant::Signer { address, signs }) => Message::new(
                &[config_instruction(&self.p.input, address, *signs)?],
                Some(&self.p.plan.message.account_keys[0]),
            ),
            (StepKind::DepositSol, _) => self.p.deposit_message.clone(),
            (StepKind::AdvanceToVisibleSlot, _) => unreachable!(),
        };
        if let Err(e) = self.p.keys_known(state, &message) {
            r.outcome = StepOutcome::EvidenceGap;
            return Ok((r.because("undeclared_account", e), None));
        }
        // Preconditions: blockers, visibility, and the handoff the reused
        // contracts are qualified for.
        match kind {
            StepKind::Upgrade => {
                if let Some(b) = &self.p.preflight.upgrade_blocker {
                    r.outcome = StepOutcome::Unsupported;
                    r.derived = json!({"capacity": self.p.preflight.capacity});
                    return Ok((r.because(&b.reason, &b.detail), None));
                }
            }
            StepKind::SetFee | StepKind::DepositSol => {
                if state.clock.slot < installed.visible_from_slot {
                    r.outcome = StepOutcome::Unsupported;
                    r.derived = json!({"installed_program": installed});
                    return Ok((
                        r.because(
                            "visibility_boundary_not_crossed",
                            format!(
                                "the program installed at slot {} is not callable on mainnet until slot {}; this step was requested at slot {}. LiteSVM 0.16 would execute it immediately, which is not mainnet-equivalent, so the step is not executed",
                                installed.deploy_slot, installed.visible_from_slot, state.clock.slot
                            ),
                        ),
                        None,
                    ));
                }
                if let Err(e) = self.handoff(kind, state, variant) {
                    r.outcome = StepOutcome::HandoffFailure;
                    return Ok((r.because("handoff_outside_qualified_contract", e), None));
                }
            }
            StepKind::AdvanceToVisibleSlot => unreachable!(),
        }
        let x = match self
            .source
            .execute(scenario, index, state, &mut self.accounts, &message)
        {
            None => {
                r.outcome = StepOutcome::ExecutionUnavailable;
                return Ok((
                    r.because("execution_unavailable", "no execution available"),
                    None,
                ));
            }
            Some(Attempt::Unavailable(detail)) => {
                r.outcome = StepOutcome::ExecutionUnavailable;
                return Ok((r.because("execution_unavailable", detail), None));
            }
            Some(Attempt::Executed(x)) => x,
        };
        ensure!(
            x.version == world::EXECUTION_VERSION
                && x.before_state_id == state.id()?
                && x.message == ProbeMessage::from(&message)
                && x.clock == state.clock,
            "retained execution does not bind to its step's state, message and Clock"
        );
        for entry in x.post.values() {
            if let Entry::Present { account_sha256 } = entry {
                let account = self
                    .accounts
                    .get(account_sha256)
                    .context("execution post account not retained")?;
                ensure!(
                    world::account_id(account)? == *account_sha256,
                    "retained account bytes differ from their digest"
                );
            }
        }
        let after = world::after(state, &x)?;
        let execution_id = x.id()?;
        self.executions.insert(execution_id.clone(), x.clone());
        r.execution_id = Some(execution_id);
        r.after_state_id = Some(self.retain(after.clone())?);
        r.clock_after = Some(after.clock.clone());
        r.installed_program_after = self.installed(&after).ok();
        if !x.outside_writes.is_empty() {
            r.outcome = StepOutcome::UnexpectedWrite;
            return Ok((
                r.because(
                    "write_outside_declared_closure",
                    format!("{:?}", x.outside_writes),
                ),
                None,
            ));
        }
        let verdict = match kind {
            StepKind::Upgrade => self.verify_upgrade(state, &after, &x, variant),
            StepKind::SetFee => self.verify_config(state, &after, &x, variant),
            StepKind::DepositSol => self.verify_deposit(state, &after, &x),
            StepKind::AdvanceToVisibleSlot => unreachable!(),
        };
        let (outcome, reason, detail, derived) = verdict;
        r.outcome = outcome;
        r.reason = reason;
        r.detail = detail;
        r.derived = derived;
        let next = (outcome == StepOutcome::Verified).then_some(after);
        Ok((r, next))
    }

    /// The reused SetFee and DepositSol contracts are qualified from the
    /// retained accounts. Only a pool produced by a verified SetFee in this
    /// scenario may differ from S0, and only for the action.
    fn handoff(&self, kind: StepKind, state: &State, variant: &Variant) -> Result<()> {
        let record = &self.p.input.historical.record;
        let pool = s::pool(record)?.address.clone();
        let s0 = &self.p.s0;
        let same = |address: &str| -> Result<bool> {
            Ok(state.accounts.get(address) == s0.accounts.get(address))
        };
        match kind {
            StepKind::SetFee => {
                for a in &self.p.plan.accounts {
                    ensure!(
                        same(&a.address)?,
                        "configuration account {} differs from S0",
                        a.address
                    );
                }
                if let Variant::Signer { address, .. } = variant {
                    ensure!(
                        state.accounts.contains_key(address),
                        "variant signer outside closure"
                    );
                }
            }
            StepKind::DepositSol => {
                for a in &record.accounts {
                    if a.address == pool {
                        let current = state
                            .account(&self.accounts, &pool)?
                            .context("pool absent")?;
                        if *current != a.account {
                            let (before, trailing) = s::decode(&a.account)?;
                            let (after, after_trailing) = s::decode(current)?;
                            let mut expected = before;
                            expected.sol_deposit_fee = after.sol_deposit_fee;
                            ensure!(
                                after == expected && trailing == after_trailing,
                                "pool differs from S0 beyond its SOL deposit fee"
                            );
                        }
                    } else {
                        ensure!(
                            same(&a.address)?,
                            "action account {} differs from S0",
                            a.address
                        );
                    }
                }
                for a in &record.acquisitions {
                    if a.source == AccountStateSource::AbsentAtBothBoundaries {
                        ensure!(
                            same(&a.address)?,
                            "retained absence {} no longer holds",
                            a.address
                        );
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn verify_upgrade(
        &self,
        before: &State,
        after: &State,
        x: &Execution,
        variant: &Variant,
    ) -> (StepOutcome, Option<String>, Option<String>, Value) {
        let result = (|| -> Result<(StepOutcome, Option<String>, Option<String>, Value)> {
            let payer = upgrade_payer();
            if !x.success {
                verify_rollback(
                    before,
                    after,
                    &self.accounts,
                    &payer,
                    x.transaction_fee_lamports,
                )?;
                let error = x.error.clone().unwrap_or_default();
                let loader_state_error = x.logs.iter().any(|l| {
                    l.contains("Invalid Program account")
                        || l.contains("Invalid ProgramData account")
                        || l.contains("Invalid Buffer account")
                });
                let reason = if error.contains("InvalidAccountData") && !loader_state_error {
                    "candidate_installation_failed"
                } else {
                    "upgrade_rejected"
                };
                return Ok((
                    StepOutcome::Rejected,
                    Some(reason.into()),
                    Some(error),
                    json!({"rollback_verified": true, "upgrade_transaction_fee_lamports": x.transaction_fee_lamports.to_string(), "variant": variant != &Variant::Declared}),
                ));
            }
            ensure!(
                x.error.is_none()
                    && x.inner_instructions
                        .iter()
                        .all(|g| g.instructions.is_empty()),
                "successful Upgrade with error or CPI"
            );
            let record = &self.p.input.historical.record;
            let pd = self.p.preflight.capacity.programdata_address.clone();
            let buffer = buffer_address();
            let changed = world::changed(before, after);
            ensure!(
                changed
                    .iter()
                    .all(|a| *a == pd || *a == buffer || *a == payer),
                "Upgrade changed accounts outside ProgramData, Buffer and spill/payer: {changed:?}"
            );
            let pd_before = before
                .account(&self.accounts, &pd)?
                .context("ProgramData before")?;
            let pd_after = after
                .account(&self.accounts, &pd)?
                .context("ProgramData after")?;
            let buffer_before = before
                .account(&self.accounts, &buffer)?
                .context("Buffer before")?;
            ensure!(
                after.account(&self.accounts, &buffer)?.is_none(),
                "Buffer not closed"
            );
            let staged = loader::decode_buffer(&buffer_before.data)?;
            let old = loader::decode_programdata(&pd_before.data)?;
            let new = loader::decode_programdata(&pd_after.data)?;
            ensure!(
                new.deploy_slot == before.clock.slot
                    && new.upgrade_authority == old.upgrade_authority
                    && pd_after.data.len() == pd_before.data.len()
                    && pd_after.owner == pd_before.owner
                    && !pd_after.executable
                    && new.bytes[..staged.bytes.len()] == staged.bytes[..]
                    && new.bytes[staged.bytes.len()..].iter().all(|b| *b == 0),
                "ProgramData is not exactly the Buffer bytes, zero padding, new slot and same authority"
            );
            ensure!(
                pd_after.lamports == rent_exempt(pd_after.data.len()),
                "ProgramData is not funded to exactly rent exemption"
            );
            let payer_before = before.account(&self.accounts, &payer)?.context("payer")?;
            let payer_after = after
                .account(&self.accounts, &payer)?
                .context("payer after")?;
            let spill = (pd_before.lamports + buffer_before.lamports)
                .checked_sub(pd_after.lamports)
                .context("negative spill")?;
            let mut expected = payer_before.clone();
            expected.lamports = (payer_before.lamports + spill)
                .checked_sub(x.transaction_fee_lamports)
                .context("payer fee")?;
            ensure!(
                *payer_after == expected,
                "spill/payer ledger does not reconcile"
            );
            let installed = self.installed(after)?;
            ensure!(
                installed.identity != "unrecognized" && installed.program_id == record.program_id,
                "installed executable is not the staged candidate"
            );
            Ok((
                StepOutcome::Verified,
                None,
                None,
                json!({
                    "loader_instruction": "Upgrade (legacy four-byte encoding, closes Buffer)",
                    "installed_program": installed,
                    "previous_deploy_slot": old.deploy_slot.to_string(),
                    "programdata_capacity_bytes": new.bytes.len(),
                    "candidate_bytes": staged.bytes.len(),
                    "zero_padding_bytes": new.bytes.len() - staged.bytes.len(),
                    "buffer_closed": true,
                    "spill_credit_lamports": spill.to_string(),
                    "upgrade_transaction_fee_lamports": x.transaction_fee_lamports.to_string(),
                    "upgrade_authority": {"address": upgrade_authority(), "origin": "assumed_simulation_only", "signer": true},
                    "visible_from_slot": installed.visible_from_slot.to_string(),
                }),
            ))
        })();
        result.unwrap_or_else(|e| {
            (
                StepOutcome::ReconciliationFailed,
                Some("installed_state_mismatch".into()),
                Some(format!("{e:#}")),
                Value::Null,
            )
        })
    }

    fn verify_config(
        &self,
        before: &State,
        after: &State,
        x: &Execution,
        variant: &Variant,
    ) -> (StepOutcome, Option<String>, Option<String>, Value) {
        let plan = &self.p.plan;
        let payer = plan.message.account_keys[0].to_string();
        let result = (|| -> Result<(StepOutcome, Option<String>, Option<String>, Value)> {
            let probe = config_execution(plan, x, &self.accounts)?;
            let changed = world::changed(before, after);
            if !x.success {
                if variant == &Variant::Declared {
                    s::verify_config_rejection(plan, &probe)?;
                }
                verify_rollback(
                    before,
                    after,
                    &self.accounts,
                    &payer,
                    x.transaction_fee_lamports,
                )?;
                return Ok((
                    StepOutcome::Rejected,
                    Some("config_execution_rejected".into()),
                    x.error.clone(),
                    json!({"rollback_verified": true, "config_transaction_fee_lamports": x.transaction_fee_lamports.to_string(), "variant": variant != &Variant::Declared}),
                ));
            }
            ensure!(
                variant == &Variant::Declared,
                "a variant signer configuration succeeded"
            );
            let pool = s::pool(&self.p.input.historical.record)?.address.clone();
            ensure!(
                changed.iter().all(|a| *a == pool || *a == payer),
                "SetFee changed closure accounts beyond pool and payer: {changed:?}"
            );
            let (_, fee) = s::values(&self.p.input.parameter)?;
            let next = s::verify_config(&self.p.input.historical, fee, plan, &probe)?;
            Ok((
                StepOutcome::Verified,
                None,
                None,
                json!({
                    "verified_pool_sha256": world::account_id(&next)?,
                    "proposed_fee": fee,
                    "preservation_verified": true,
                    "config_transaction_fee_lamports": x.transaction_fee_lamports.to_string(),
                    "manager_assumption": "assumed_simulation_only",
                }),
            ))
        })();
        result.unwrap_or_else(|e| {
            (
                StepOutcome::ReconciliationFailed,
                Some("post_config_state_mismatch".into()),
                Some(format!("{e:#}")),
                Value::Null,
            )
        })
    }

    fn deposit_record(&self, state: &State) -> Result<ReplayRecord> {
        let h = &self.p.input.historical;
        let pool = s::pool(&h.record)?;
        let current = state
            .account(&self.accounts, &pool.address)?
            .context("pool absent")?;
        if *current == pool.account {
            Ok(h.record.clone())
        } else {
            s::proposed_record(h, current)
        }
    }

    fn verify_deposit(
        &self,
        before: &State,
        after: &State,
        x: &Execution,
    ) -> (StepOutcome, Option<String>, Option<String>, Value) {
        let result = (|| -> Result<(StepOutcome, Option<String>, Option<String>, Value)> {
            let record = self.deposit_record(before)?;
            let result = action_result(&record, x, &self.accounts)?;
            let writable: BTreeSet<String> =
                record.accounts.iter().map(|a| a.address.clone()).collect();
            let changed = world::changed(before, after);
            if let Some(extra) = changed.iter().find(|a| !writable.contains(*a)) {
                return Ok((
                    StepOutcome::UnexpectedWrite,
                    Some("write_outside_action_accounts".into()),
                    Some(extra.clone()),
                    Value::Null,
                ));
            }
            let fee =
                s::RationalFee::from(s::decode(&s::pool(&record)?.account)?.0.sol_deposit_fee);
            let mut derived = json!({"pool_sol_deposit_fee": fee});
            if before.id()? == self.p.s0.id()? {
                let fidelity = crate::universal::fidelity::compare_v1(&record, &result)?;
                derived["historical_fidelity"] =
                    json!({"status": fidelity.status, "failures": fidelity.failures});
            }
            let reconciliation = match s::reconcile(&record, &result) {
                Ok(v) => v,
                Err(e) => {
                    return Ok((
                        StepOutcome::ReconciliationFailed,
                        Some("action_reconciliation_failed".into()),
                        Some(format!("{e:#}")),
                        derived,
                    ))
                }
            };
            derived["reconciliation"] = reconciliation;
            derived["compute_units"] = x.compute_units.to_string().into();
            Ok(if x.success {
                (StepOutcome::Verified, None, None, derived)
            } else {
                (
                    StepOutcome::Rejected,
                    Some("action_rejected".into()),
                    x.error.clone(),
                    derived,
                )
            })
        })();
        result.unwrap_or_else(|e| {
            (
                StepOutcome::ReconciliationFailed,
                Some("action_reconciliation_failed".into()),
                Some(format!("{e:#}")),
                Value::Null,
            )
        })
    }
}

// ---------------------------------------------------------------------------
// Anchors and comparison
// ---------------------------------------------------------------------------

fn deposit_step(s: &ScenarioReport) -> Option<&StepRecord> {
    s.steps.iter().find(|r| r.step == StepKind::DepositSol)
}

fn baseline_anchor(control: &ScenarioReport) -> Anchor {
    let Some(step) = deposit_step(control) else {
        return Anchor {
            status: AnchorStatus::NotEstablished,
            reason: Some("control_missing".into()),
            detail: Value::Null,
        };
    };
    let fidelity = &step.derived["historical_fidelity"];
    let matched = step.outcome == StepOutcome::Verified
        && fidelity["status"] == json!(ReplayFidelity::Matched)
        && fidelity["failures"] == json!([]);
    Anchor {
        status: if matched {
            AnchorStatus::Matched
        } else {
            AnchorStatus::Failed
        },
        reason: (!matched).then(|| "baseline_world_fidelity_failed".into()),
        detail: json!({
            "step_outcome": step.outcome,
            "historical_fidelity": fidelity,
            "execution_id": step.execution_id,
            "contract": "S0 -> DepositSol through the seeded installed-program world must reproduce the retained historical outcome (Matched)",
        }),
    }
}

fn installed_overlay_anchor(
    p: &Prepared,
    evidence: &Evidence,
    upgrade_control: &ScenarioReport,
    overlay: &Option<Attempt<ExecutionResult>>,
) -> Result<Anchor> {
    let not_established = |reason: &str, detail: Value| Anchor {
        status: AnchorStatus::NotEstablished,
        reason: Some(reason.into()),
        detail,
    };
    let overlay = match overlay {
        None => return Ok(not_established("overlay_not_executed", Value::Null)),
        Some(Attempt::Unavailable(e)) => {
            return Ok(not_established("overlay_execution_unavailable", json!(e)))
        }
        Some(Attempt::Executed(x)) => x,
    };
    let Some(step) = deposit_step(upgrade_control).filter(|s| s.execution_id.is_some()) else {
        return Ok(not_established(
            "installed_deposit_not_executed",
            Value::Null,
        ));
    };
    let record = &p.input.historical.record;
    let x = &evidence.executions[step.execution_id.as_ref().unwrap()];
    let installed = action_result(record, x, &evidence.accounts)?;
    let installed_reconciliation = s::reconcile(record, &installed).ok();
    let overlay_reconciliation = s::reconcile(record, overlay).ok();
    let mut mismatches = Vec::new();
    let mut check = |field: &str, equal: bool| {
        if !equal {
            mismatches.push(field.to_string());
        }
    };
    check("success", installed.success == overlay.success);
    check("error", installed.error == overlay.error);
    check("transaction_fee", installed.fee == overlay.fee);
    check(
        "compute_units",
        installed.compute_units == overlay.compute_units,
    );
    check("logs", installed.logs == overlay.logs);
    check("cpi_shape", installed.cpi_calls == overlay.cpi_calls);
    check("watched_post_state", installed.accounts == overlay.accounts);
    check(
        "reconciliation",
        installed_reconciliation.is_some() && installed_reconciliation == overlay_reconciliation,
    );
    let pre_state = evidence
        .states
        .get(step.before_state_id.as_ref().context("before")?)
        .context("installed pre-action state")?;
    let pre_equal = record.accounts.iter().all(|a| {
        pre_state
            .account(&evidence.accounts, &a.address)
            .ok()
            .flatten()
            == Some(&a.account)
    });
    check("pre_action_accounts", pre_equal);
    let identity = step
        .installed_program_after
        .as_ref()
        .map(|i| {
            i.identity == "candidate"
                || (i.identity == "historical_v1" && p.input.candidate.elf == p.v1)
        })
        .unwrap_or(false);
    check("executable_identity", identity);
    let matched = mismatches.is_empty();
    Ok(Anchor {
        status: if matched {
            AnchorStatus::Matched
        } else {
            AnchorStatus::Failed
        },
        reason: (!matched).then(|| "installed_overlay_behavior_mismatch".into()),
        detail: json!({
            "contract": "same V2 bytes, same retained action message and pre-action accounts; installed (ProgramData seeded, upgraded by the loader, executed after the visibility boundary) versus the existing candidate overlay (executable added directly, S0)",
            "compared": ["success","error","transaction_fee","compute_units","logs","cpi_shape","watched_post_state","reconciliation","pre_action_accounts","executable_identity"],
            "not_compared": "complete world bytes: ProgramData, deployment slot, Buffer, authority/payer balances and Clock.slot legitimately differ",
            "mismatches": mismatches,
            "installed_execution_id": step.execution_id,
            "installed_clock_slot": x.clock.slot.to_string(),
            "overlay_clock_slot": record.clock.slot.to_string(),
            "overlay_executable": ExecutableArtifact::of(&p.input.candidate.elf),
            "installed_program": step.installed_program_after,
        }),
    })
}

fn quantity(step: Option<&StepRecord>, metric: &str) -> Quantity {
    match step {
        Some(s) if s.outcome == StepOutcome::Verified => {
            match s.derived["reconciliation"][metric].as_str() {
                Some(v) => Quantity {
                    value: Some(v.into()),
                    unavailable_reason: None,
                },
                None => Quantity {
                    value: None,
                    unavailable_reason: Some(
                        "aliased role lacks an independently measured split".into(),
                    ),
                },
            }
        }
        Some(s) => Quantity {
            value: None,
            unavailable_reason: Some(format!(
                "final action {}: {}",
                serde_json::to_value(s.outcome)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default(),
                s.detail.clone().unwrap_or_default()
            )),
        },
        None => Quantity {
            value: None,
            unavailable_reason: Some("final action absent".into()),
        },
    }
}

/// The comparable result a step established, beyond its outcome.
fn established_result(step: &StepRecord) -> Value {
    match step.step {
        StepKind::Upgrade => step.derived["installed_program"]["executable_sha256"].clone(),
        StepKind::SetFee => step.derived["verified_pool_sha256"].clone(),
        StepKind::DepositSol => step.derived["reconciliation"].clone(),
        StepKind::AdvanceToVisibleSlot => Value::Null,
    }
}

fn compare(
    anchors: &Anchors,
    scenarios: &[ScenarioReport],
    evidence: &Evidence,
) -> Result<Comparison> {
    let get = |n: ScenarioName| scenarios.iter().find(|s| s.name == n).context("scenario");
    let a = get(ScenarioName::OrderA)?;
    let b = get(ScenarioName::OrderB)?;
    let mut step_contrasts = Vec::new();
    for (ia, sa) in a.steps.iter().enumerate() {
        if sa.step == StepKind::AdvanceToVisibleSlot {
            continue;
        }
        let (ib, sb) = b
            .steps
            .iter()
            .enumerate()
            .find(|(_, s)| s.step == sa.step)
            .context("orders declare the same step kinds")?;
        step_contrasts.push(StepContrast {
            step: sa.step,
            order_a_index: ia,
            order_b_index: ib,
            order_a: sa.outcome,
            order_b: sb.outcome,
            differs: sa.outcome != sb.outcome
                || (sa.outcome == StepOutcome::Verified
                    && established_result(sa) != established_result(sb)),
        });
    }
    let classified = |s: &ScenarioReport| {
        let mut stopped = false;
        s.steps.iter().all(|r| match (stopped, r.outcome) {
            (false, StepOutcome::Verified) => true,
            (false, StepOutcome::Rejected) => {
                stopped = true;
                r.derived["rollback_verified"] == true || r.step == StepKind::DepositSol
            }
            (true, StepOutcome::NotExecuted) => true,
            _ => false,
        })
    };
    let controls = [
        ScenarioName::Control,
        ScenarioName::UpgradeControl,
        ScenarioName::ConfigControl,
    ];
    let mut unavailable = Vec::new();
    if anchors.baseline_world_fidelity.status != AnchorStatus::Matched {
        unavailable.push("baseline world fidelity anchor (S0 -> DepositSol) not matched".into());
    }
    if anchors.installed_overlay.status != AnchorStatus::Matched {
        unavailable.push(format!(
            "installed-V2 versus overlay-V2 anchor: {}",
            anchors.installed_overlay.reason.clone().unwrap_or_default()
        ));
    }
    for c in controls {
        let s = get(c)?;
        if !s.completed {
            unavailable.push(format!("control scenario {c:?} did not complete"));
        }
    }
    for s in [a, b] {
        if !classified(s) {
            let bad = s
                .steps
                .iter()
                .find(|r| {
                    !matches!(
                        r.outcome,
                        StepOutcome::Verified | StepOutcome::NotExecuted | StepOutcome::Rejected
                    )
                })
                .map(|r| {
                    format!(
                        "{:?} step {} {:?} ({})",
                        s.name,
                        r.index,
                        r.outcome,
                        r.reason.clone().unwrap_or_default()
                    )
                })
                .unwrap_or_else(|| format!("{:?} is not fully classified", s.name));
            unavailable.push(bad);
        }
    }
    let da = deposit_step(a);
    let db = deposit_step(b);
    let final_action_metrics: BTreeMap<String, MetricContrast> = METRICS
        .iter()
        .map(|m| {
            let qa = quantity(da, m);
            let qb = quantity(db, m);
            let differs = match (&qa.value, &qb.value) {
                (Some(x), Some(y)) => Some(x != y),
                _ => None,
            };
            (
                (*m).to_string(),
                MetricContrast {
                    order_a: qa,
                    order_b: qb,
                    differs,
                },
            )
        })
        .collect();
    let outcomes = [
        da.map(|s| s.outcome).unwrap_or(StepOutcome::NotExecuted),
        db.map(|s| s.outcome).unwrap_or(StepOutcome::NotExecuted),
    ];
    let executed = |o: StepOutcome| matches!(o, StepOutcome::Verified | StepOutcome::Rejected);
    let final_action_executed_differently =
        (executed(outcomes[0]) && executed(outcomes[1])).then(|| {
            outcomes[0] != outcomes[1]
                || established_result(da.unwrap()) != established_result(db.unwrap())
        });
    let state_a = evidence.states.get(&a.final_state_id).context("final A")?;
    let state_b = evidence.states.get(&b.final_state_id).context("final B")?;
    let final_state_differences = world::changed(state_a, state_b);
    let first_divergence = step_contrasts.iter().find(|c| c.differs).cloned();
    let mut established =
        vec!["every step's typed outcome, before/after state and execution identity".to_string()];
    let (status, finding, statement) = if !unavailable.is_empty() {
        (
            ComparisonStatus::RolloutNotEstablished,
            None,
            format!(
                "The rollout comparison was not established: {}.",
                unavailable.join("; ")
            ),
        )
    } else if let Some(d) = &first_divergence {
        established
            .push("both orders fully classified with verified rollbacks for rejected steps".into());
        let word = |o: StepOutcome| {
            serde_json::to_value(o)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default()
        };
        match d.step {
            StepKind::SetFee => (
                ComparisonStatus::RolloutOrderEffectObserved,
                Some("rollout/order/configuration_installation_outcome_differs".to_string()),
                format!(
                    "Under the pinned world and explicit signer assumptions, changing rollout order changed whether the proposed fee configuration could be installed: SetFee was {} in order A (upgrade first) and {} in order B (configuration first).",
                    word(d.order_a), word(d.order_b)
                ),
            ),
            StepKind::Upgrade => (
                ComparisonStatus::RolloutOrderEffectObserved,
                Some("rollout/order/upgrade_installation_outcome_differs".to_string()),
                format!(
                    "Under the pinned world and explicit signer assumptions, changing rollout order changed the program upgrade's installation: {} in order A and {} in order B.",
                    word(d.order_a), word(d.order_b)
                ),
            ),
            _ => (
                ComparisonStatus::RolloutOrderEffectObserved,
                Some(if d.order_a == d.order_b {
                    "rollout/order/final_action_economics_differ".to_string()
                } else {
                    "rollout/order/final_action_outcome_differs".to_string()
                }),
                "Under the pinned world and explicit signer assumptions, both orders installed every declared step but the final DepositSol differed.".to_string(),
            ),
        }
    } else {
        established.push("both orders reached reconciled final DepositSol results".into());
        (
            ComparisonStatus::NoOrderEffectObserved,
            None,
            "Under the pinned world and explicit signer assumptions, both rollout orders reached semantically equivalent final user-action outcomes.".to_string(),
        )
    };
    for (metric, c) in &final_action_metrics {
        if c.differs.is_none() {
            unavailable.push(format!(
                "final-action metric {metric}: {}",
                c.order_a
                    .unavailable_reason
                    .clone()
                    .or_else(|| c.order_b.unavailable_reason.clone())
                    .unwrap_or_default()
            ));
        }
    }
    Ok(Comparison {
        status,
        finding,
        statement,
        order_scenario_ids: [a.scenario_id.clone(), b.scenario_id.clone()],
        all_steps_executed: scenarios
            .iter()
            .map(|s| {
                (
                    serde_json::to_value(s.name)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default(),
                    s.steps
                        .iter()
                        .all(|r| r.outcome != StepOutcome::NotExecuted),
                )
            })
            .collect(),
        final_state_ids: [a.final_state_id.clone(), b.final_state_id.clone()],
        final_states_differ: a.final_state_id != b.final_state_id,
        final_state_differences,
        step_contrasts,
        first_divergence,
        final_action_outcomes: outcomes,
        final_action_executed_differently,
        final_action_metrics,
        established,
        unavailable,
    })
}

// ---------------------------------------------------------------------------
// Assembly, verification and reproduction
// ---------------------------------------------------------------------------

fn assemble(p: &Prepared, source: &mut dyn Source) -> Result<(Report, Evidence)> {
    let mut runner = Runner::new(p, source);
    let control = runner.scenario(ScenarioName::Control, ScenarioName::Control.steps(), None)?;
    let baseline = baseline_anchor(&control);
    let gate = (baseline.status != AnchorStatus::Matched).then_some(
        "baseline_world_fidelity_failed: S0 -> DepositSol did not reproduce the retained outcome",
    );
    let overlay = if gate.is_none() && p.preflight.upgrade_blocker.is_none() {
        runner.source.overlay()
    } else {
        None
    };
    let mut scenarios = vec![control];
    for name in &SCENARIOS[1..] {
        scenarios.push(runner.scenario(*name, name.steps(), gate)?);
    }
    let evidence = runner.evidence();
    let installed_overlay = if p.preflight.upgrade_blocker.is_some() {
        Anchor {
            status: AnchorStatus::NotEstablished,
            reason: p
                .preflight
                .upgrade_blocker
                .as_ref()
                .map(|b| b.reason.clone()),
            detail: Value::Null,
        }
    } else {
        installed_overlay_anchor(p, &evidence, &scenarios[1], &overlay)?
    };
    let anchors = Anchors {
        baseline_world_fidelity: baseline,
        installed_overlay,
        overlay_execution: match &overlay {
            Some(Attempt::Executed(x)) => Some(x.clone()),
            _ => None,
        },
        overlay_unavailable: match &overlay {
            Some(Attempt::Unavailable(e)) => Some(e.clone()),
            _ => None,
        },
    };
    let comparison = compare(&anchors, &scenarios, &evidence)?;
    let mut report = Report {
        schema: REPORT_SCHEMA.into(),
        question: QUESTION.into(),
        analysis_input_id: p.analysis_input_id.clone(),
        upgrade_change_spec_id: p.input.upgrade.id()?,
        parameter_change_spec_id: p.input.parameter.id()?,
        preflight: p.preflight.clone(),
        visibility: visibility_contract(),
        assumptions: assumptions(),
        anchors,
        scenarios,
        comparison,
        limitations: LIMITS.iter().map(|s| s.to_string()).collect(),
        evidence: evidence.index(),
        report_sha256: String::new(),
    };
    report.report_sha256 = report_digest(&report)?;
    Ok((report, evidence))
}

pub fn report_digest(report: &Report) -> Result<String> {
    let mut v = serde_json::to_value(report)?;
    v.as_object_mut()
        .context("report object")?
        .remove("report_sha256");
    canonical::digest(&v)
}

/// Execute all five scenarios in fresh VMs from retained bytes.
pub fn analyse(input: &Input) -> Result<Analysis> {
    let p = Prepared::new(input.clone())?;
    let mut source = VmSource { prepared: &p };
    let (report, evidence) = assemble(&p, &mut source)?;
    Ok(Analysis {
        input: p.input.clone(),
        contract: p.contract.clone(),
        report,
        evidence,
    })
}

/// Reduce retained executions again, without a VM and without checking the
/// seal: the analysis these executions actually support. Missing objects are
/// carried as unavailable, never repaired.
pub fn reduce(analysis: &Analysis) -> Result<Analysis> {
    let p = Prepared::new(analysis.input.clone())?;
    let mut source = RetainedSource {
        report: &analysis.report,
        evidence: &analysis.evidence,
    };
    let (report, evidence) = assemble(&p, &mut source)?;
    Ok(Analysis {
        input: p.input.clone(),
        contract: p.contract.clone(),
        report,
        evidence,
    })
}

/// Read-only: rebuild the input contract and reduce the retained executions
/// again. No VM, provider or repair. Any tampered or inconsistent object makes
/// the reduction differ.
pub fn verify(analysis: &Analysis) -> Result<()> {
    ensure!(
        analysis.report.report_sha256 == report_digest(&analysis.report)?,
        "rollout report seal mismatch"
    );
    ensure!(
        analysis.evidence.index() == analysis.report.evidence,
        "retained evidence set differs from the report's index"
    );
    for (id, state) in &analysis.evidence.states {
        ensure!(state.id()? == *id, "state object differs from its id");
    }
    for (id, x) in &analysis.evidence.executions {
        ensure!(x.id()? == *id, "execution object differs from its id");
    }
    for (id, a) in &analysis.evidence.accounts {
        ensure!(
            world::account_id(a)? == *id,
            "account object differs from its id"
        );
    }
    let rebuilt = reduce(analysis)?;
    ensure!(
        rebuilt.contract == analysis.contract
            && rebuilt.report.analysis_input_id == analysis.report.analysis_input_id,
        "rollout input identity differs from the retained contract"
    );
    ensure!(
        rebuilt.report == analysis.report,
        "rollout steps, handoffs, classifications, anchors or comparison differ from the retained report"
    );
    ensure!(
        rebuilt.evidence == analysis.evidence,
        "rollout evidence differs from the reduction of retained executions"
    );
    Ok(())
}

/// Verification first, then every scenario again in fresh VMs; the complete
/// report and evidence must be identical.
pub fn reproduce(analysis: &Analysis) -> Result<Analysis> {
    verify(analysis)?;
    let again = analyse(&analysis.input)?;
    if again != *analysis {
        bail!("offline rollout reproduction differs from the retained analysis");
    }
    Ok(again)
}

/// Count of fresh VM executions an analysis performed (steps plus overlay).
pub fn vm_executions(report: &Report) -> usize {
    report
        .scenarios
        .iter()
        .flat_map(|s| &s.steps)
        .filter(|r| r.execution_id.is_some())
        .count()
        + usize::from(report.anchors.overlay_execution.is_some())
}
