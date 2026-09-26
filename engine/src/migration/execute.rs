//! VM migration execution through the existing LiteSVM backend.
//!
//! For every executed unit: restore the exact bank (world accounts + deterministic
//! proposed overlay), load the exact candidate bytes and the captured deployed token
//! programs, pin the rehearsal Clock, construct the migration from adapter
//! descriptors, execute, record logs and errors, read the post-state of every
//! account the transaction references, reconcile it exactly against the planner,
//! and verify rollback on failure. A transaction can only modify accounts it
//! references, so checking every referenced writable account is a complete check
//! for unintended changes. Nothing is signed and nothing leaves this process.
use super::{
    adapter::{self, HolderSigning, InstructionDescriptor, MigrateAccounts},
    authority::HolderAuthority,
    planner::{ImpactClass, MigrationPlan, MigrationUnit},
    spec::{DestinationFunding, SourceDisposition, TokenMigrationV1},
    world::{World, WorldClock, WorldOrigin, SYSTEM_PROGRAM},
};
use crate::{
    evidence::paths::PathStatus,
    executor::{LoadedProgram, ProbeInnerInstruction},
    replay::hash_bytes as sha256,
    standard_programs::token as decode,
    standard_programs::token::proposed_token_account,
    standard_programs::token::{ATA_PROGRAM, CLOCK},
    types::AccountSnapshot,
};
use anyhow::{anyhow, ensure, Context, Result};
use litesvm::LiteSVM;
use serde::{Deserialize, Serialize};
use solana_account::Account;
use solana_address::Address;
use solana_message::Message;
use solana_transaction::Transaction;
use std::collections::{BTreeMap, BTreeSet};

pub const EXECUTOR_VERSION: &str = "eplyx-migration-executor/v1";
/// Lamports of synthesized proposed overlay accounts. Proposed, not observed.
pub const PROPOSED_LAMPORTS: u64 = 10_000_000;
pub const RELAYER_LAMPORTS: u64 = 1_000_000_000_000;

/// Where one bank account comes from. Origins never mix and are never upgraded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BankOrigin {
    World {
        origin: WorldOrigin,
    },
    /// Deterministically derived rollout state the operator proposes.
    Proposed {
        derivation: String,
    },
    /// A synthetic local fee payer / relayer. Never chain state.
    LocalRelayer,
    /// A typed local mutation of an identified parent account (stress/search).
    Derived {
        parent: Box<BankOrigin>,
        mutation: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BankAccount {
    pub account: AccountSnapshot,
    pub origin: BankOrigin,
}

/// The complete pre-execution bank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bank {
    pub accounts: BTreeMap<String, BankAccount>,
    pub clock: WorldClock,
    pub clock_basis: String,
    pub relayer: String,
}

pub fn relayer() -> Address {
    super::fixture::label_address("eplyx-migration-rehearsal", "local-relayer")
}

impl Bank {
    /// World + proposed overlay (+ local relayer), with the rehearsal Clock.
    pub fn build(
        world: &World,
        spec: &TokenMigrationV1,
        plan: &MigrationPlan,
        config_bytes: &[u8],
    ) -> Result<Self> {
        Self::build_filtered(world, spec, plan, config_bytes, None)
    }

    /// As [`Bank::build`], keeping only `keys` of the world (for an isolated
    /// transaction, its message accounts). Proposed overlay accounts are always built.
    pub fn build_filtered(
        world: &World,
        spec: &TokenMigrationV1,
        plan: &MigrationPlan,
        config_bytes: &[u8],
        keys: Option<&BTreeSet<String>>,
    ) -> Result<Self> {
        let mut accounts: BTreeMap<String, BankAccount> = world
            .accounts
            .iter()
            .filter(|(address, _)| address.as_str() != CLOCK)
            .filter(|(address, _)| keys.is_none_or(|keys| keys.contains(*address)))
            .map(|(address, entry)| {
                (
                    address.clone(),
                    BankAccount {
                        account: entry.account.clone(),
                        origin: BankOrigin::World {
                            origin: entry.origin.clone(),
                        },
                    },
                )
            })
            .collect();
        let overlay = &plan.overlay;
        let mut proposed = vec![(
            overlay.config.clone(),
            AccountSnapshot {
                lamports: PROPOSED_LAMPORTS,
                owner: overlay.program_id.clone(),
                data: config_bytes.to_vec(),
                executable: false,
                rent_epoch: 0,
            },
            "Candidate configuration: token_migration_v1 layout packed from the exact specification; PDA([\"eplyx-migration-config\", spec sha-256], candidate program)".to_string(),
        )];
        if overlay.reserve_origin.as_deref() == Some("Proposed") {
            let vault = overlay.reserve_vault.clone().context("reserve vault")?;
            let funded = match &plan.reserve_override_raw {
                Some(raw) => raw.parse()?,
                None => spec
                    .resolve()?
                    .proposed_reserve
                    .context("proposed reserve funding")?,
            };
            let mint = world
                .snapshot(&spec.destination.mint)
                .context("destination mint")?;
            proposed.push((
                vault,
                AccountSnapshot {
                    lamports: PROPOSED_LAMPORTS,
                    owner: spec.destination.token_program.clone(),
                    data: proposed_token_account(
                        &mint.data,
                        &spec.destination.mint.parse()?,
                        &overlay.migration_authority.parse()?,
                        funded,
                    )?,
                    executable: false,
                    rent_epoch: 0,
                },
                format!("Proposed reserve vault funded with {funded} raw destination units; ATA(migration authority, destination token program, destination mint)"),
            ));
        }
        if let Some(escrow) = &overlay.escrow_vault {
            let mint = world.snapshot(&spec.source.mint).context("source mint")?;
            proposed.push((
                escrow.clone(),
                AccountSnapshot {
                    lamports: PROPOSED_LAMPORTS,
                    owner: spec.source.token_program.clone(),
                    data: proposed_token_account(
                        &mint.data,
                        &spec.source.mint.parse()?,
                        &overlay.migration_authority.parse()?,
                        0,
                    )?,
                    executable: false,
                    rent_epoch: 0,
                },
                "Proposed empty escrow vault; ATA(migration authority, source token program, source mint)".to_string(),
            ));
        }
        for (address, account, derivation) in proposed {
            ensure!(
                !accounts.contains_key(&address),
                "proposed overlay account {address} collides with world state"
            );
            let derived_reserve = plan.reserve_override_raw.is_some()
                && overlay.reserve_vault.as_deref() == Some(address.as_str());
            let origin = if derived_reserve {
                BankOrigin::Derived {
                    parent: Box::new(BankOrigin::Proposed { derivation }),
                    mutation: format!(
                        "ProposedReserveFunding {}",
                        plan.reserve_override_raw.as_deref().unwrap_or_default()
                    ),
                }
            } else {
                BankOrigin::Proposed { derivation }
            };
            accounts.insert(address, BankAccount { account, origin });
        }
        let relayer = relayer().to_string();
        ensure!(
            !accounts.contains_key(&relayer),
            "local relayer collides with world state"
        );
        accounts.insert(
            relayer.clone(),
            BankAccount {
                account: AccountSnapshot {
                    lamports: RELAYER_LAMPORTS,
                    owner: SYSTEM_PROGRAM.into(),
                    data: vec![],
                    executable: false,
                    rent_epoch: 0,
                },
                origin: BankOrigin::LocalRelayer,
            },
        );
        Ok(Self {
            accounts,
            clock: plan.rehearsal_clock.clock,
            clock_basis: plan.rehearsal_clock.basis.clone(),
            relayer,
        })
    }

    pub fn get(&self, address: &str) -> Option<&AccountSnapshot> {
        self.accounts.get(address).map(|a| &a.account)
    }

    /// Keep only the accounts an isolated transaction references. Nothing else can
    /// be read or written by it, so the execution is identical and seeding is cheap.
    pub fn restrict_to(&mut self, instructions: &[InstructionDescriptor]) {
        let mut keys = message_keys(instructions);
        keys.insert(self.relayer.clone());
        self.accounts.retain(|address, _| keys.contains(address));
    }
}

/// Every account and program an instruction list references.
pub fn message_keys(instructions: &[InstructionDescriptor]) -> BTreeSet<String> {
    instructions
        .iter()
        .flat_map(|ix| {
            ix.accounts
                .iter()
                .map(|a| a.address.clone())
                .chain(std::iter::once(ix.program.clone()))
        })
        .collect()
}

impl Bank {
    #[doc(hidden)]
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }
}

/// Executables the VM loads: captured token/ATA programs and the exact candidate.
pub fn programs(
    world: &World,
    spec: &TokenMigrationV1,
    program_id: &str,
    candidate: &crate::change::ResolvedCandidate,
) -> Result<Vec<LoadedProgram>> {
    let ids: BTreeSet<&str> = [
        spec.source.token_program.as_str(),
        spec.destination.token_program.as_str(),
        ATA_PROGRAM,
    ]
    .into();
    ensure!(
        !ids.contains(program_id),
        "the candidate program ID collides with a token program"
    );
    let mut loaded: Vec<LoadedProgram> = ids
        .iter()
        .map(|id| world.program(id))
        .collect::<Result<_>>()?;
    ensure!(
        candidate.bytes().starts_with(b"\x7fELF"),
        "the candidate is not an SBF ELF"
    );
    ensure!(
        world.get(program_id).is_none(),
        "the candidate program ID already exists in the world"
    );
    loaded.push(LoadedProgram {
        program_id: program_id.parse()?,
        loader: adapter::CANDIDATE_LOADER.parse()?,
        bytes: candidate.bytes().to_vec(),
    });
    Ok(loaded)
}

/// The actual VM loader input must carry exactly the packaged candidate bytes.
pub fn assert_candidate(
    programs: &[LoadedProgram],
    program_id: &str,
    expected_sha256: &str,
) -> Result<()> {
    let candidate: Vec<_> = programs
        .iter()
        .filter(|p| p.program_id.to_string() == program_id)
        .collect();
    ensure!(
        candidate.len() == 1
            && candidate[0].loader.to_string() == adapter::CANDIDATE_LOADER
            && sha256(&candidate[0].bytes) == expected_sha256,
        "the VM candidate bytes differ from the validated package"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Executed and exactly reconciled against the specification.
    Migrated,
    /// Executed and failed; every referenced account rolled back.
    Rejected,
    /// Executed successfully but the state does not match the specification.
    ReconciliationMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationCheck {
    pub check: String,
    pub expected: String,
    pub observed: String,
    pub holds: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedDeltas {
    pub source_debit_raw: String,
    pub source_supply_decrease_raw: String,
    pub escrow_gross_credit_raw: String,
    pub escrow_withheld_fee_raw: String,
    pub reserve_debit_raw: String,
    pub destination_supply_increase_raw: String,
    pub destination_net_credit_raw: String,
    pub destination_withheld_fee_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureSignature {
    /// `candidate`, `token-program`, `ata-program`, `runtime` or `unknown`.
    pub stage: String,
    pub instruction_index: Option<u8>,
    pub program: Option<String>,
    pub error: String,
    /// The ABI name for candidate custom errors, e.g. `InsufficientReserve`.
    pub error_name: Option<String>,
    pub relevant_log: Option<String>,
    pub rollback_verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitExecution {
    pub unit_id: String,
    pub source_account: String,
    pub amount_raw: String,
    pub status: PathStatus,
    pub outcome: Outcome,
    pub instructions: Vec<InstructionDescriptor>,
    pub message_sha256: String,
    pub pre_state_sha256: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub compute_units: u64,
    pub logs_sha256: String,
    pub candidate_log: Option<String>,
    pub failure: Option<FailureSignature>,
    pub deltas: ObservedDeltas,
    pub reconciliation: Vec<ReconciliationCheck>,
    pub reconciled: bool,
    pub changed_accounts: Vec<String>,
    pub unexpected_changes: Vec<String>,
    pub assumed_signers: Vec<String>,
}

/// Build the unit's instructions from adapter descriptors.
pub fn unit_instructions(
    spec: &TokenMigrationV1,
    plan: &MigrationPlan,
    unit: &MigrationUnit,
    relayer: &str,
) -> Result<(Vec<InstructionDescriptor>, Vec<String>)> {
    let authority = match &unit.authority {
        super::authority::AuthorityPath::Available { authority, .. } => authority,
        _ => anyhow::bail!("unit {} has no available authority path", unit.unit_id),
    };
    let (holder, mut signers) = match authority {
        HolderAuthority::OwnerMultisig {
            owner,
            threshold,
            signers,
        } => {
            let chosen: Vec<String> = signers
                .iter()
                .take(usize::from(*threshold))
                .cloned()
                .collect();
            (
                HolderSigning::Multisig {
                    multisig: owner.clone(),
                    signers: chosen.clone(),
                },
                chosen,
            )
        }
        other => (
            HolderSigning::Direct {
                authority: other.authority().to_string(),
            },
            vec![other.authority().to_string()],
        ),
    };
    let amount: u64 = unit.amount_raw.parse()?;
    let mut instructions = vec![adapter::compute_budget()];
    if unit.destination.action == "CreateAssociated" {
        instructions.push(adapter::create_destination(
            relayer,
            &unit.destination.address,
            &unit.owner,
            &spec.destination.mint,
            &spec.destination.token_program,
        )?);
    }
    instructions.push(adapter::migrate(
        spec,
        &plan.overlay,
        &MigrateAccounts {
            source: &unit.source_account,
            destination: &unit.destination.address,
            holder: &holder,
        },
        amount,
    )?);
    if plan.overlay.authority_kind == "External" {
        signers.push(plan.overlay.migration_authority.clone());
    }
    signers.push(relayer.to_string());
    Ok((instructions, signers))
}

/// One LiteSVM session over a bank. Sequential units share state, exactly like a
/// rollout executing transactions one after another at the pinned Clock.
pub struct Session {
    svm: LiteSVM,
    pub program_id: String,
}

impl Session {
    pub fn new(bank: &Bank, programs: &[LoadedProgram], program_id: &str) -> Result<Self> {
        let mut svm = LiteSVM::new()
            .with_sigverify(false)
            .with_blockhash_check(false)
            .with_transaction_history(0);
        for program in programs {
            svm.add_program_with_loader(program.program_id, &program.bytes, program.loader)
                .map_err(|e| anyhow!("cannot load executable {}: {e:?}", program.program_id))?;
        }
        for (address, entry) in &bank.accounts {
            if entry.account.executable {
                // Program headers are loaded through the program cache above.
                if programs
                    .iter()
                    .any(|p| p.program_id.to_string() == *address)
                {
                    continue;
                }
            }
            svm.set_account(address.parse()?, to_account(&entry.account)?)
                .map_err(|e| anyhow!("cannot seed bank account {address}: {e:?}"))?;
        }
        svm.set_sysvar(&bank.clock.clock());
        Ok(Self {
            svm,
            program_id: program_id.into(),
        })
    }

    pub fn account(&self, address: &str) -> Option<AccountSnapshot> {
        let key: Address = address.parse().ok()?;
        self.svm.get_account(&key).map(|a| from_account(&a))
    }

    pub fn set_account(&mut self, address: &str, account: &AccountSnapshot) -> Result<()> {
        self.svm
            .set_account(address.parse()?, to_account(account)?)
            .map_err(|e| anyhow!("cannot set account {address}: {e:?}"))
    }

    pub fn set_clock(&mut self, clock: &WorldClock) {
        self.svm.set_sysvar(&clock.clock());
    }

    /// Execute descriptors as one transaction; report pre/post of every account key.
    pub fn execute(
        &mut self,
        instructions: &[InstructionDescriptor],
        payer: &str,
    ) -> Result<RawExecution> {
        let built = instructions
            .iter()
            .map(InstructionDescriptor::instruction)
            .collect::<Result<Vec<_>>>()?;
        let message = Message::new(&built, Some(&payer.parse()?));
        let keys: Vec<String> = message.account_keys.iter().map(|k| k.to_string()).collect();
        let pre: BTreeMap<String, Option<AccountSnapshot>> =
            keys.iter().map(|k| (k.clone(), self.account(k))).collect();
        let message_sha256 = sha256(&message.serialize());
        let (success, error, meta) = match self
            .svm
            .send_transaction(Transaction::new_unsigned(message.clone()))
        {
            Ok(meta) => (true, None, meta),
            Err(failure) => (false, Some(failure.err), failure.meta),
        };
        let post: BTreeMap<String, Option<AccountSnapshot>> =
            keys.iter().map(|k| (k.clone(), self.account(k))).collect();
        let mut inner = Vec::new();
        for outer in &meta.inner_instructions {
            for ix in outer {
                let i = &ix.instruction;
                inner.push(ProbeInnerInstruction {
                    program: keys
                        .get(usize::from(i.program_id_index))
                        .cloned()
                        .context("inner instruction program index is outside the message")?,
                    stack_height: ix.stack_height,
                    accounts: i
                        .accounts
                        .iter()
                        .map(|a| {
                            keys.get(usize::from(*a))
                                .cloned()
                                .context("inner instruction account index is outside the message")
                        })
                        .collect::<Result<Vec<_>>>()?,
                    data: i.data.clone(),
                });
            }
        }
        Ok(RawExecution {
            success,
            error: error.map(|e| format!("{e:?}")),
            instruction_index: None,
            fee: meta.fee,
            payer: payer.to_string(),
            logs: meta.logs,
            compute_units: meta.compute_units_consumed,
            inner,
            pre,
            post,
            message_sha256,
            keys,
        }
        .with_index())
    }
}

/// Raw facts of one transaction.
pub struct RawExecution {
    pub success: bool,
    pub error: Option<String>,
    pub instruction_index: Option<u8>,
    /// Transaction fee charged to the fee payer, even when the transaction fails.
    pub fee: u64,
    pub payer: String,
    pub logs: Vec<String>,
    pub compute_units: u64,
    pub inner: Vec<ProbeInnerInstruction>,
    pub pre: BTreeMap<String, Option<AccountSnapshot>>,
    pub post: BTreeMap<String, Option<AccountSnapshot>>,
    pub message_sha256: String,
    pub keys: Vec<String>,
}

impl RawExecution {
    fn with_index(mut self) -> Self {
        if let Some(error) = &self.error {
            if let Some(rest) = error.strip_prefix("InstructionError(") {
                self.instruction_index = rest.split(',').next().and_then(|n| n.trim().parse().ok());
            }
        }
        self
    }
    /// Whether a failed transaction left every referenced account as it was, except
    /// the fee payer's exact fee debit (Solana charges fees for failed transactions).
    pub fn rolled_back(&self) -> bool {
        self.keys.iter().all(|k| {
            let (pre, post) = (
                self.pre.get(k).cloned().flatten(),
                self.post.get(k).cloned().flatten(),
            );
            if *k == self.payer {
                match (pre, post) {
                    (Some(a), Some(b)) => {
                        a.data == b.data
                            && a.owner == b.owner
                            && a.executable == b.executable
                            && a.lamports.checked_sub(b.lamports) == Some(self.fee)
                    }
                    (a, b) => a == b,
                }
            } else {
                pre == post
            }
        })
    }
    pub fn changed(&self) -> Vec<String> {
        self.keys
            .iter()
            .filter(|k| self.pre.get(*k) != self.post.get(*k))
            .cloned()
            .collect()
    }
    pub fn pre_state_sha256(&self) -> String {
        let rows: Vec<(String, Option<String>)> = self
            .pre
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_ref().map(|a| {
                        sha256(
                            format!(
                                "{}:{}:{}:{}",
                                a.lamports,
                                a.owner,
                                a.executable,
                                sha256(&a.data)
                            )
                            .as_bytes(),
                        )
                    }),
                )
            })
            .collect();
        crate::canonical::digest(&rows).unwrap_or_default()
    }
}

fn to_account(snapshot: &AccountSnapshot) -> Result<Account> {
    Ok(Account {
        lamports: snapshot.lamports,
        data: snapshot.data.clone(),
        owner: snapshot.owner.parse()?,
        executable: snapshot.executable,
        rent_epoch: snapshot.rent_epoch,
    })
}

fn from_account(account: &Account) -> AccountSnapshot {
    AccountSnapshot {
        lamports: account.lamports,
        owner: account.owner.to_string(),
        data: account.data.clone(),
        executable: account.executable,
        rent_epoch: account.rent_epoch,
    }
}

/// Classify a failure: which instruction and program failed, and the ABI name.
pub fn failure_signature(
    raw: &RawExecution,
    instructions: &[InstructionDescriptor],
    program_id: &str,
    rollback_verified: bool,
) -> FailureSignature {
    let error = raw.error.clone().unwrap_or_default();
    let index = raw.instruction_index;
    let top = index.and_then(|i| instructions.get(usize::from(i)));
    // The program that raised the error logs "failed" first; callers that
    // propagate it log their own "failed" lines afterwards.
    let failed_program = raw
        .logs
        .iter()
        .find(|l| l.ends_with("failed") || l.contains(" failed: "))
        .and_then(|l| l.strip_prefix("Program "))
        .and_then(|l| l.split(' ').next())
        .map(str::to_string)
        .or_else(|| top.map(|t| t.program.clone()));
    let custom = error
        .split("Custom(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .and_then(|n| n.parse::<u32>().ok());
    let stage = match failed_program.as_deref() {
        Some(p) if p == program_id => "candidate",
        Some(p) if p == ATA_PROGRAM => "ata-program",
        Some(p) if p == decode::LEGACY_PROGRAM || p == decode::TOKEN_2022_PROGRAM => {
            "token-program"
        }
        Some(_) => "runtime",
        None => "unknown",
    };
    let error_name = match (stage, custom) {
        ("candidate", Some(code)) => adapter::error_name(code).map(str::to_string),
        ("token-program", Some(code)) => token_error_name(code).map(str::to_string),
        _ => None,
    };
    let relevant_log = raw
        .logs
        .iter()
        .rev()
        .find(|l| l.starts_with("Program log: Error") || l.contains("custom program error"))
        .cloned();
    FailureSignature {
        stage: stage.into(),
        instruction_index: index,
        program: failed_program,
        error,
        error_name,
        relevant_log,
        rollback_verified,
    }
}

/// SPL Token / Token-2022 `TokenError` names (spl-token-2022-interface 3.1.1
/// numbering; the legacy program shares codes 0-19).
pub fn token_error_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "NotRentExempt",
        1 => "InsufficientFunds",
        2 => "InvalidMint",
        3 => "MintMismatch",
        4 => "OwnerMismatch",
        5 => "FixedSupply",
        6 => "AlreadyInUse",
        7 => "InvalidNumberOfProvidedSigners",
        8 => "InvalidNumberOfRequiredSigners",
        9 => "UninitializedState",
        10 => "NativeNotSupported",
        11 => "NonNativeHasBalance",
        12 => "InvalidInstruction",
        13 => "InvalidState",
        14 => "Overflow",
        15 => "AuthorityTypeNotSupported",
        16 => "MintCannotFreeze",
        17 => "AccountFrozen",
        18 => "MintDecimalsMismatch",
        19 => "NonNativeNotSupported",
        20 => "ExtensionTypeMismatch",
        21 => "ExtensionBaseMismatch",
        31 => "MintRequiredForTransfer",
        32 => "FeeMismatch",
        34 => "ImmutableOwner",
        36 => "NoMemo",
        37 => "NonTransferable",
        42 => "CpiGuardTransferBlocked",
        43 => "CpiGuardBurnBlocked",
        48 => "ExtensionNotFound",
        67 => "MintPaused",
        _ => return None,
    })
}

/// Execute one planned unit in a session and reconcile it exactly.
pub fn execute_unit(
    session: &mut Session,
    spec: &TokenMigrationV1,
    plan: &MigrationPlan,
    unit: &MigrationUnit,
    relayer: &str,
) -> Result<UnitExecution> {
    let (instructions, assumed_signers) = unit_instructions(spec, plan, unit, relayer)?;
    let raw = session.execute(&instructions, relayer)?;
    reconcile(spec, plan, unit, &instructions, assumed_signers, &raw)
}

pub fn reconcile(
    spec: &TokenMigrationV1,
    plan: &MigrationPlan,
    unit: &MigrationUnit,
    instructions: &[InstructionDescriptor],
    assumed_signers: Vec<String>,
    raw: &RawExecution,
) -> Result<UnitExecution> {
    let program_id = plan.overlay.program_id.as_str();
    let pre = |a: &str| raw.pre.get(a).cloned().flatten();
    let post = |a: &str| raw.post.get(a).cloned().flatten();
    let tokens = |s: &Option<AccountSnapshot>| {
        s.as_ref()
            .map(|s| decode::account_amounts(&s.owner, &s.data))
            .transpose()
            .map(|v| v.unwrap_or((0, 0)))
    };
    let mint_supply = |s: &Option<AccountSnapshot>| {
        s.as_ref()
            .map(|s| decode::mint_base(&s.owner, &s.data).map(|m| m.supply))
            .transpose()
            .map(|v| v.unwrap_or(0))
    };
    let (source_before, _) = tokens(&pre(&unit.source_account))?;
    let (source_after, _) = tokens(&post(&unit.source_account))?;
    let (dest_before, dest_fee_before) = tokens(&pre(&unit.destination.address))?;
    let (dest_after, dest_fee_after) = tokens(&post(&unit.destination.address))?;
    let source_supply_decrease = mint_supply(&pre(&spec.source.mint))? as i128
        - mint_supply(&post(&spec.source.mint))? as i128;
    let dest_supply_increase = mint_supply(&post(&spec.destination.mint))? as i128
        - mint_supply(&pre(&spec.destination.mint))? as i128;
    let (escrow_gross, escrow_fee) = match &plan.overlay.escrow_vault {
        Some(e) => {
            let (a0, f0) = tokens(&pre(e))?;
            let (a1, f1) = tokens(&post(e))?;
            (
                (a1 as i128 + f1 as i128) - (a0 as i128 + f0 as i128),
                f1 as i128 - f0 as i128,
            )
        }
        None => (0, 0),
    };
    let reserve_debit = match &plan.overlay.reserve_vault {
        Some(r) => tokens(&pre(r))?.0 as i128 - tokens(&post(r))?.0 as i128,
        None => 0,
    };
    let deltas = ObservedDeltas {
        source_debit_raw: (source_before as i128 - source_after as i128).to_string(),
        source_supply_decrease_raw: source_supply_decrease.to_string(),
        escrow_gross_credit_raw: escrow_gross.to_string(),
        escrow_withheld_fee_raw: escrow_fee.to_string(),
        reserve_debit_raw: reserve_debit.to_string(),
        destination_supply_increase_raw: dest_supply_increase.to_string(),
        destination_net_credit_raw: (dest_after as i128 - dest_before as i128).to_string(),
        destination_withheld_fee_raw: (dest_fee_after as i128 - dest_fee_before as i128)
            .to_string(),
    };
    let changed = raw.changed();
    let candidate_log = raw
        .logs
        .iter()
        .find(|l| l.contains(adapter::LOG_PREFIX))
        .map(|l| l.trim_start_matches("Program log: ").to_string());
    let message_sha256 = raw.message_sha256.clone();
    let pre_state_sha256 = raw.pre_state_sha256();
    let logs_sha256 = sha256(raw.logs.join("\n").as_bytes());
    if !raw.success {
        let rollback = raw.rolled_back();
        let failure = failure_signature(raw, instructions, program_id, rollback);
        return Ok(UnitExecution {
            unit_id: unit.unit_id.clone(),
            source_account: unit.source_account.clone(),
            amount_raw: unit.amount_raw.clone(),
            status: PathStatus::Failed,
            outcome: Outcome::Rejected,
            instructions: instructions.to_vec(),
            message_sha256,
            pre_state_sha256,
            compute_units: raw.compute_units,
            logs_sha256,
            candidate_log,
            failure: Some(failure),
            deltas,
            reconciliation: vec![ReconciliationCheck {
                check: "failed transaction rolled back every referenced account".into(),
                expected: format!(
                    "no referenced account changed except the {} lamport fee debit",
                    raw.fee
                ),
                observed: if rollback {
                    "rolled back".into()
                } else {
                    changed.join(",")
                },
                holds: rollback,
            }],
            reconciled: false,
            unexpected_changes: if rollback {
                vec![]
            } else {
                changed
                    .iter()
                    .filter(|a| **a != raw.payer)
                    .cloned()
                    .collect()
            },
            changed_accounts: changed,
            assumed_signers,
        });
    }
    // Success: every quantity must match the planner's exact expectation.
    let mut checks = vec![];
    let mut check = |name: &str, expected: String, observed: String| {
        let holds = expected == observed;
        checks.push(ReconciliationCheck {
            check: name.into(),
            expected,
            observed,
            holds,
        });
    };
    let quote = unit.quote.clone();
    let expected = unit.expected.clone();
    match (&quote, &expected) {
        (Some(q), Some(e)) => {
            let consumed: u64 = q.consumed_raw.parse().unwrap_or(0);
            let fee: u64 = q.fee_raw.parse().unwrap_or(0);
            let converted: u64 = q.converted_raw.parse().unwrap_or(0);
            let output: u64 = q.output_raw.parse().unwrap_or(0);
            check(
                "candidate success log",
                adapter::success_log(spec, consumed, fee, converted, output),
                candidate_log.clone().unwrap_or_else(|| "absent".into()),
            );
            check(
                "source debit",
                e.source_debit_raw.clone(),
                deltas.source_debit_raw.clone(),
            );
            match spec.source_disposition {
                SourceDisposition::Burn => check(
                    "source supply burned",
                    e.source_supply_decrease_raw.clone(),
                    deltas.source_supply_decrease_raw.clone(),
                ),
                SourceDisposition::Escrow => {
                    check(
                        "escrow gross credit",
                        q.consumed_raw.clone(),
                        deltas.escrow_gross_credit_raw.clone(),
                    );
                    check(
                        "escrow withheld transfer fee",
                        e.escrow_transfer_fee_raw.clone(),
                        deltas.escrow_withheld_fee_raw.clone(),
                    );
                    check(
                        "source supply unchanged",
                        "0".into(),
                        deltas.source_supply_decrease_raw.clone(),
                    );
                }
            }
            match spec.destination_funding {
                DestinationFunding::ReserveTransfer { .. } => {
                    check(
                        "reserve release",
                        e.reserve_debit_raw.clone(),
                        deltas.reserve_debit_raw.clone(),
                    );
                    check(
                        "destination supply unchanged",
                        "0".into(),
                        deltas.destination_supply_increase_raw.clone(),
                    );
                }
                DestinationFunding::MintTo => check(
                    "destination minted",
                    e.destination_supply_increase_raw.clone(),
                    deltas.destination_supply_increase_raw.clone(),
                ),
            }
            check(
                "destination net credit",
                e.destination_credit_raw.clone(),
                deltas.destination_net_credit_raw.clone(),
            );
            check(
                "destination withheld transfer fee",
                e.destination_transfer_fee_raw.clone(),
                deltas.destination_withheld_fee_raw.clone(),
            );
            let invoked = raw
                .logs
                .iter()
                .any(|l| *l == format!("Program {program_id} invoke [1]"));
            check(
                "candidate invoked at depth 1",
                "true".into(),
                invoked.to_string(),
            );
            let cpi = |program: &str, tags: &[u8]| {
                raw.inner.iter().any(|i| {
                    i.program == program
                        && i.stack_height >= 2
                        && i.data.first().is_some_and(|t| tags.contains(t))
                })
            };
            let disposition_cpi = cpi(
                &spec.source.token_program,
                match spec.source_disposition {
                    SourceDisposition::Burn => &[15],
                    SourceDisposition::Escrow => &[12],
                },
            );
            let funding_cpi = cpi(
                &spec.destination.token_program,
                match spec.destination_funding {
                    DestinationFunding::ReserveTransfer { .. } => &[12],
                    DestinationFunding::MintTo => &[14],
                },
            );
            check(
                "source disposition CPI into the source token program",
                "true".into(),
                disposition_cpi.to_string(),
            );
            check(
                "destination funding CPI into the destination token program",
                "true".into(),
                funding_cpi.to_string(),
            );
            let config_unchanged = pre(&plan.overlay.config) == post(&plan.overlay.config);
            check(
                "candidate configuration unchanged",
                "true".into(),
                config_unchanged.to_string(),
            );
        }
        _ => check(
            "only units the specification plans as migratable may migrate",
            "planned migratable unit with exact quote".into(),
            format!("{:?}", unit.class),
        ),
    }
    // Changes outside the expected set are unintended.
    let mut allowed: BTreeSet<String> = [
        unit.source_account.clone(),
        unit.destination.address.clone(),
        plan.overlay.funding_account.clone(),
        spec.source.mint.clone(),
    ]
    .into();
    allowed.extend(plan.overlay.escrow_vault.clone());
    allowed.insert(super::execute::relayer().to_string());
    let unexpected: Vec<String> = changed
        .iter()
        .filter(|a| !allowed.contains(*a))
        .cloned()
        .collect();
    check(
        "no unintended account changes",
        "none".into(),
        if unexpected.is_empty() {
            "none".into()
        } else {
            unexpected.join(",")
        },
    );
    let reconciled = checks.iter().all(|c| c.holds);
    Ok(UnitExecution {
        unit_id: unit.unit_id.clone(),
        source_account: unit.source_account.clone(),
        amount_raw: unit.amount_raw.clone(),
        status: if reconciled {
            PathStatus::Proven
        } else {
            PathStatus::Failed
        },
        outcome: if reconciled {
            Outcome::Migrated
        } else {
            Outcome::ReconciliationMismatch
        },
        instructions: instructions.to_vec(),
        message_sha256,
        pre_state_sha256,
        compute_units: raw.compute_units,
        logs_sha256,
        candidate_log,
        failure: None,
        deltas,
        reconciliation: checks,
        reconciled,
        changed_accounts: changed,
        unexpected_changes: unexpected,
        assumed_signers,
    })
}

/// Whether the planner expects this unit to be attempted in a rehearsal.
pub fn attempted(unit: &MigrationUnit) -> bool {
    matches!(
        unit.class,
        ImpactClass::Migratable | ImpactClass::InsufficientReserve
    )
}
