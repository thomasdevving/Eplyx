//! Deterministic execution of one fixture against one program build.
//!
//! Every knob that could make two runs differ for reasons other than the program
//! version is pinned here: the clock sysvar, the initial account set, the fee
//! payer, the signer set and the instruction bytes. The program bytecode is the
//! only free variable.
//!
//! Execution goes through real SBF bytecode in the Solana VM. Nothing in this
//! module calls the fixture protocol's Rust functions directly - if it did, the
//! comparison would be testing the host build rather than the deployable
//! artefact.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_address::Address;
use solana_clock::Clock;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

use crate::types::{AccountSnapshot, Fixture};

/// Pinned execution environment. Both versions see exactly these values.
pub const FIXED_SLOT: u64 = 300_000_000;
pub const FIXED_EPOCH: u64 = 694;
pub const FIXED_UNIX_TIMESTAMP: i64 = 1_760_000_000;

/// One side of the comparison: a compiled program artefact.
#[derive(Clone, Debug)]
pub struct ProgramVersion {
    pub label: String,
    pub bytes: Vec<u8>,
}

impl ProgramVersion {
    pub fn from_file(label: impl Into<String>, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).with_context(|| {
            format!(
                "could not read program artefact {}\n\
                 run `./scripts/build-programs.sh` to compile V1 and V2",
                path.display()
            )
        })?;
        Ok(Self {
            label: label.into(),
            bytes,
        })
    }
}

/// One executable dependency the replay environment must provide.
///
/// The loader is carried because it is not cosmetic: the runtime charges
/// different loading costs and applies different verification per loader, so
/// loading a legacy-loader program under the upgradeable loader would shift
/// compute away from what mainnet actually metered.
#[derive(Clone, Debug)]
pub struct LoadedProgram {
    pub program_id: Address,
    pub loader: Address,
    pub bytes: Vec<u8>,
}

/// Actual inner instruction payloads for consequence probes (including event CPI).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProbeInnerInstruction {
    pub program: String,
    pub stack_height: u8,
    pub accounts: Vec<String>,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProbeTransactionExecution {
    pub success: bool,
    pub error: Option<String>,
    #[serde(with = "crate::numfmt::u64_string")]
    pub compute_units: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub transaction_fee_lamports: u64,
    pub logs: Vec<String>,
    pub inner_instructions: Vec<ProbeInnerInstruction>,
    pub post_accounts: BTreeMap<String, AccountSnapshot>,
}

/// The same fresh LiteSVM backend, for captured multi-instruction probe messages.
/// Signature possession and blockhash freshness are explicit local assumptions;
/// message signer privileges, program/account checks and deployed SBF CPIs execute.
pub fn execute_probe_message(
    accounts: &[crate::types::NamedAccount],
    watch: &[String],
    clock: Clock,
    programs: &[LoadedProgram],
    message: Message,
) -> Result<ProbeTransactionExecution> {
    let mut svm = LiteSVM::new()
        .with_sigverify(false)
        .with_blockhash_check(false);
    svm.set_sysvar(&clock);
    for program in programs {
        svm.add_program_with_loader(program.program_id, &program.bytes, program.loader)
            .map_err(|e| {
                anyhow!(
                    "cannot load captured executable {}: {e:?}",
                    program.program_id
                )
            })?;
    }
    for named in accounts {
        svm.set_account(
            named.address.parse::<Address>()?,
            to_account(&named.account)?,
        )
        .map_err(|e| anyhow!("cannot seed captured account {}: {e:?}", named.address))?;
    }
    let keys = message.account_keys.clone();
    let (success, error, meta) = match svm.send_transaction(Transaction::new_unsigned(message)) {
        Ok(meta) => (true, None, meta),
        Err(failure) => (false, Some(format!("{:?}", failure.err)), failure.meta),
    };
    let mut inner_instructions = Vec::new();
    for outer in &meta.inner_instructions {
        for inner in outer {
            let ix = &inner.instruction;
            inner_instructions.push(ProbeInnerInstruction {
                program: keys
                    .get(usize::from(ix.program_id_index))
                    .context("probe CPI program index out of bounds")?
                    .to_string(),
                stack_height: inner.stack_height,
                accounts: ix
                    .accounts
                    .iter()
                    .map(|i| {
                        keys.get(usize::from(*i))
                            .map(ToString::to_string)
                            .context("probe CPI account index out of bounds")
                    })
                    .collect::<Result<Vec<_>>>()?,
                data: ix.data.clone(),
            });
        }
    }
    let mut post_accounts = BTreeMap::new();
    for address in watch {
        if let Some(account) = svm.get_account(&address.parse::<Address>()?) {
            post_accounts.insert(address.clone(), from_account(&account));
        }
    }
    Ok(ProbeTransactionExecution {
        success,
        error,
        compute_units: meta.compute_units_consumed,
        transaction_fee_lamports: meta.fee,
        logs: meta.logs,
        inner_instructions,
        post_accounts,
    })
}

/// A single cross-program invocation observed during execution.
///
/// Carries enough of the invoked instruction to compare two graphs without
/// comparing the instruction data itself: the depth it ran at, which top-level
/// instruction it descends from, and the shape of the call. A changed
/// discriminant or account count is a different call even when the program and
/// the count are unchanged.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CpiCall {
    pub program: String,
    pub stack_height: u8,
    #[serde(default)]
    pub outer_index: u8,
    #[serde(default)]
    pub account_count: u8,
    #[serde(default)]
    pub data_len: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminant: Option<u8>,
}

/// Everything observable about one execution.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionResult {
    pub version: String,
    pub success: bool,
    pub error: Option<String>,
    pub compute_units: Option<u64>,
    pub fee: u64,
    pub logs: Vec<String>,
    pub cpi_calls: Vec<CpiCall>,
    /// Post-execution state of every watched account, keyed by label.
    pub accounts: BTreeMap<String, AccountSnapshot>,
}

fn to_account(snapshot: &AccountSnapshot) -> Result<Account> {
    Ok(Account {
        lamports: snapshot.lamports,
        data: snapshot.data.clone(),
        owner: snapshot
            .owner
            .parse()
            .map_err(|e| anyhow!("invalid owner address {:?}: {e}", snapshot.owner))?,
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

fn keypair_for(fixture: &Fixture, label: &str) -> Result<Keypair> {
    let spec = fixture
        .keypair(label)
        .ok_or_else(|| anyhow!("fixture {} has no keypair labelled {label:?}", fixture.id))?;
    let seed: [u8; 32] = spec
        .seed
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("keypair {label:?} seed must be 32 bytes"))?;
    Ok(Keypair::new_from_array(seed))
}

/// Render a transaction error, resolving fixture-protocol custom codes to names
/// so reports do not bottom out in `Custom(9)`.
fn format_error(error: &TransactionError) -> String {
    if let TransactionError::InstructionError(index, InstructionError::Custom(code)) = error {
        if let Some(named) = fixture_lending_interface::LendingError::from_code(*code) {
            return format!("instruction {index}: {} ({code})", named.name());
        }
        return format!("instruction {index}: Custom({code})");
    }
    format!("{error:?}")
}

/// Execute one fixture against one program build.
///
/// The VM is constructed fresh for every call, which is what gives the "reset to
/// identical initial state" guarantee: there is no state to reset because
/// nothing is carried between executions.
pub fn execute(
    fixture: &Fixture,
    program_id: &Address,
    program: &ProgramVersion,
) -> Result<ExecutionResult> {
    execute_in_environment(
        fixture,
        program_id,
        program,
        Clock {
            slot: FIXED_SLOT,
            epoch_start_timestamp: FIXED_UNIX_TIMESTAMP,
            epoch: FIXED_EPOCH,
            leader_schedule_epoch: FIXED_EPOCH + 1,
            unix_timestamp: FIXED_UNIX_TIMESTAMP,
        },
        None,
        &[],
    )
}

/// Historical replay supplies a normalized message without requiring wallet
/// keys. Signature and recent-blockhash checks are disabled only in this local
/// execution path; message signer privileges and runtime account checks remain.
pub fn execute_in_environment(
    fixture: &Fixture,
    program_id: &Address,
    program: &ProgramVersion,
    clock: Clock,
    historical_message: Option<Message>,
    dependencies: &[LoadedProgram],
) -> Result<ExecutionResult> {
    let historical = historical_message.is_some();
    let mut svm = LiteSVM::new()
        .with_sigverify(!historical)
        .with_blockhash_check(!historical);
    svm.set_sysvar(&clock);

    // Dependencies first, so the artefact under test always wins if a caller
    // passes a dependency entry for the program being compared. The candidate
    // is the one thing the run is allowed to vary.
    for dependency in dependencies {
        if dependency.program_id == *program_id {
            continue;
        }
        svm.add_program_with_loader(dependency.program_id, &dependency.bytes, dependency.loader)
            .map_err(|e| {
                anyhow!(
                    "failed to load dependency program {}: {e:?}",
                    dependency.program_id
                )
            })?;
    }

    svm.add_program(*program_id, &program.bytes)
        .map_err(|e| anyhow!("failed to load program {}: {e:?}", program.label))?;

    for named in &fixture.accounts {
        let address: Address = named
            .address
            .parse()
            .map_err(|e| anyhow!("invalid address {:?}: {e}", named.address))?;
        svm.set_account(address, to_account(&named.account)?)
            .map_err(|e| anyhow!("failed to seed account {:?}: {e:?}", named.label))?;
    }

    let transaction = if let Some(message) = historical_message {
        Transaction::new_unsigned(message)
    } else {
        fixture_transaction(fixture, svm.latest_blockhash())?
    };
    let account_keys = transaction.message.account_keys.clone();

    let (success, error, meta) = match svm.send_transaction(transaction) {
        Ok(meta) => (true, None, meta),
        Err(failure) => (false, Some(format_error(&failure.err)), failure.meta),
    };

    // Structured CPI capture from inner instructions rather than log scraping:
    // a change in the invocation shape is a regression class of its own.
    let mut cpi_calls = Vec::new();
    for (outer_index, outer) in meta.inner_instructions.iter().enumerate() {
        for inner in outer {
            let index = inner.instruction.program_id_index as usize;
            let program_key = account_keys
                .get(index)
                .map(|key| key.to_string())
                .unwrap_or_else(|| format!("<unresolved index {index}>"));
            cpi_calls.push(CpiCall {
                program: program_key,
                stack_height: inner.stack_height,
                outer_index: u8::try_from(outer_index).unwrap_or(u8::MAX),
                account_count: u8::try_from(inner.instruction.accounts.len()).unwrap_or(u8::MAX),
                data_len: u32::try_from(inner.instruction.data.len()).unwrap_or(u32::MAX),
                discriminant: inner.instruction.data.first().copied(),
            });
        }
    }

    let mut accounts = BTreeMap::new();
    for label in &fixture.watch {
        let named = fixture
            .account(label)
            .ok_or_else(|| anyhow!("fixture {} watches unknown account {label:?}", fixture.id))?;
        let address: Address = named.address.parse().unwrap();
        if let Some(account) = svm.get_account(&address) {
            accounts.insert(label.clone(), from_account(&account));
        }
    }

    Ok(ExecutionResult {
        version: program.label.clone(),
        success,
        error,
        compute_units: Some(meta.compute_units_consumed),
        fee: meta.fee,
        logs: meta.logs.clone(),
        cpi_calls,
        accounts,
    })
}

/// Build the controlled signed transaction; used by the local-validator capture
/// workflow as well as the synthetic fixture executor.
pub fn fixture_transaction(fixture: &Fixture, blockhash: solana_hash::Hash) -> Result<Transaction> {
    let fee_payer = keypair_for(fixture, &fixture.fee_payer)?;
    let mut signers: Vec<Keypair> = vec![fee_payer];
    for label in &fixture.signers {
        if *label == fixture.fee_payer {
            continue;
        }
        let candidate = keypair_for(fixture, label)?;
        if signers.iter().any(|s| s.pubkey() == candidate.pubkey()) {
            continue;
        }
        signers.push(candidate);
    }

    let metas = fixture
        .instruction
        .accounts
        .iter()
        .map(|meta| {
            let address: Address = meta
                .address
                .parse()
                .map_err(|e| anyhow!("invalid meta address {:?}: {e}", meta.address))?;
            Ok(AccountMeta {
                pubkey: address,
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let instruction = Instruction {
        program_id: fixture
            .instruction
            .program
            .parse()
            .map_err(|e| anyhow!("invalid instruction program id: {e}"))?,
        accounts: metas,
        data: fixture.instruction.data.clone(),
    };

    let payer_pubkey = signers[0].pubkey();
    let message = Message::new(&[instruction], Some(&payer_pubkey));
    let signer_refs: Vec<&Keypair> = signers.iter().collect();
    Ok(Transaction::new(&signer_refs, message, blockhash))
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use crate::types::NamedAccount;

    #[test]
    fn local_unsigned_probe_is_fresh_deterministic_and_reports_failure_without_state_leakage() {
        let payer = Address::new_from_array([31; 32]);
        let recipient = Address::new_from_array([32; 32]);
        let account = |address: Address, lamports| NamedAccount {
            label: "public-label fixture".into(),
            address: address.to_string(),
            account: AccountSnapshot {
                lamports,
                owner: solana_sdk_ids::system_program::ID.to_string(),
                data: vec![],
                executable: false,
                rent_epoch: 0,
            },
        };
        let accounts = vec![account(payer, 10_000_000), account(recipient, 1_000_000)];
        let watch = vec![payer.to_string(), recipient.to_string()];
        let execute = |amount| {
            execute_probe_message(
                &accounts,
                &watch,
                Clock {
                    slot: 42,
                    epoch: 3,
                    unix_timestamp: -7,
                    ..Clock::default()
                },
                &[],
                Message::new(
                    &[solana_system_interface::instruction::transfer(
                        &payer, &recipient, amount,
                    )],
                    Some(&payer),
                ),
            )
            .unwrap()
        };
        let first = execute(123);
        assert!(first.success, "{:?}", first.error);
        assert_eq!(first, execute(123));
        assert_eq!(
            first.post_accounts[&recipient.to_string()].lamports,
            1_000_123
        );
        assert_eq!(
            first.post_accounts[&payer.to_string()].lamports,
            10_000_000 - 123 - first.transaction_fee_lamports
        );
        let rejected = execute(u64::MAX);
        assert!(!rejected.success);
        assert!(rejected.error.is_some());
        assert_eq!(
            rejected.post_accounts[&recipient.to_string()],
            accounts[1].account
        );
        assert_eq!(first, execute(123));
        assert!(serde_json::to_value(&first).unwrap()["compute_units"].is_string());
        assert!(serde_json::to_value(&first).unwrap()["transaction_fee_lamports"].is_string());
    }
}
