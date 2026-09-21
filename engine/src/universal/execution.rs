//! One historical execution contract for legacy and native v0 messages.
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use anyhow::{anyhow, ensure, Result};
use litesvm::LiteSVM;
use serde::{Deserialize, Serialize};
use solana_account::Account;
use solana_address::Address;
use solana_clock::Clock;
use solana_hash::Hash;
use solana_message::VersionedMessage;
use solana_slot_hashes::SlotHashes;
use solana_transaction::{versioned::VersionedTransaction, Transaction};

use super::model::ResolvedMessage;
use crate::{executor::LoadedProgram, types::AccountSnapshot};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InnerInstruction {
    pub program_id_index: u8,
    pub accounts: Vec<u8>,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
    pub stack_height: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InnerGroup {
    pub outer_index: usize,
    pub instructions: Vec<InnerInstruction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnData {
    pub program: String,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionEvidence {
    pub success: bool,
    pub error: Option<String>,
    pub compute_units: u64,
    pub fee: u64,
    pub logs: Vec<String>,
    pub inner_instructions: Vec<InnerGroup>,
    pub return_data: ReturnData,
    /// Address-keyed and presence preserving; absent accounts remain `None`.
    pub post_accounts: BTreeMap<String, Option<AccountSnapshot>>,
}

pub struct ExecutionRequest<'a> {
    pub message: &'a ResolvedMessage,
    pub seeds: &'a BTreeMap<String, AccountSnapshot>,
    pub absent_pre_accounts: &'a [String],
    pub watched: &'a [String],
    /// Historical sysvars are seeded before ordinary accounts.
    pub runtime_sysvars: &'a BTreeMap<String, AccountSnapshot>,
    /// V1 may supply a reconstructed clock; V2 supplies exact sysvar bytes.
    pub clock: Option<Clock>,
    /// V1 loads pinned ELF files; V2 can execute historically seeded accounts.
    pub programs_to_load: &'a [LoadedProgram],
    pub signature_check: bool,
    pub blockhash_check: bool,
    pub unlimited_logs: bool,
    pub slot_hashes: SlotHashesVariant,
    /// V2 requires a full account census; V1 retains its historical contract.
    pub require_complete_state: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotHashesVariant {
    BackendDefault,
    Empty,
    Different,
}

pub trait ExecutionBackend {
    fn execute(&self, request: &ExecutionRequest<'_>) -> Result<ExecutionEvidence>;
}

pub struct LiteSvmBackend;

/// Local diagnostic timings. They are never part of evidence identity.
/// LiteSVM may lazily load seeded ProgramData while executing, so that cost is
/// included in `transaction_execution`, not `explicit_program_registration`.
#[derive(Clone, Debug, Default)]
pub struct BackendTimings {
    pub runtime_setup: Duration,
    pub explicit_program_registration: Duration,
    pub account_seeding: Duration,
    pub transaction_execution: Duration,
    pub evidence_collection: Duration,
}

fn account(snapshot: &AccountSnapshot) -> Result<Account> {
    Ok(Account {
        lamports: snapshot.lamports,
        data: snapshot.data.clone(),
        owner: snapshot.owner.parse()?,
        executable: snapshot.executable,
        rent_epoch: snapshot.rent_epoch,
    })
}

fn snapshot(account: &Account) -> AccountSnapshot {
    AccountSnapshot {
        lamports: account.lamports,
        owner: account.owner.to_string(),
        data: account.data.clone(),
        executable: account.executable,
        rent_epoch: account.rent_epoch,
    }
}

impl LiteSvmBackend {
    pub fn execute_timed(
        &self,
        request: &ExecutionRequest<'_>,
    ) -> Result<(ExecutionEvidence, BackendTimings)> {
        let mut timings = BackendTimings::default();
        let started = Instant::now();
        let mut svm = LiteSVM::new()
            .with_sigverify(request.signature_check)
            .with_blockhash_check(request.blockhash_check);
        if request.unlimited_logs {
            svm = svm.with_log_bytes_limit(None);
        }
        if let Some(clock) = &request.clock {
            svm.set_sysvar(clock);
        }
        for (address, value) in request.runtime_sysvars {
            svm.set_account(address.parse()?, account(value)?)
                .map_err(|e| anyhow!("historical sysvar {address}: {e:?}"))?;
        }
        match request.slot_hashes {
            SlotHashesVariant::BackendDefault => {}
            SlotHashesVariant::Empty => svm.set_sysvar(&SlotHashes::new(&[])),
            SlotHashesVariant::Different => {
                let slot = request.message.transaction.slot;
                ensure!(
                    slot >= 7,
                    "slot too early for diagnostic SlotHashes profile"
                );
                svm.set_sysvar(&SlotHashes::new(&[
                    (slot - 1, Hash::new_from_array([17; 32])),
                    (slot - 7, Hash::new_from_array([99; 32])),
                ]));
            }
        }
        timings.runtime_setup = started.elapsed();
        let started = Instant::now();
        for program in request.programs_to_load {
            svm.add_program_with_loader(program.program_id, &program.bytes, program.loader)
                .map_err(|e| anyhow!("load program {}: {e:?}", program.program_id))?;
        }
        timings.explicit_program_registration = started.elapsed();
        let started = Instant::now();
        // ProgramData before executable headers preserves upgradeable-loader
        // metadata and matches the historical native-v0 execution proof.
        for executable in [false, true] {
            for (address, value) in request.seeds {
                if value.executable == executable {
                    svm.set_account(address.parse()?, account(value)?)
                        .map_err(|e| anyhow!("seed account {address}: {e:?}"))?;
                }
            }
        }
        if request.require_complete_state {
            ensure!(
                request
                    .message
                    .account_keys
                    .iter()
                    .all(|key| request.seeds.contains_key(key)
                        || request.runtime_sysvars.contains_key(key)
                        || request.absent_pre_accounts.contains(key)
                        || key == "Sysvar1nstructions1111111111111111111111111"),
                "message key lacks a resolved seed or absence proof"
            );
        }
        timings.account_seeding = started.elapsed();
        let started = Instant::now();
        let (success, error, meta) = match &request.message.message {
            VersionedMessage::Legacy(message) => {
                let tx = Transaction::new_unsigned(message.clone());
                match svm.send_transaction(tx) {
                    Ok(meta) => (true, None, meta),
                    Err(failure) => (false, Some(format!("{:?}", failure.err)), failure.meta),
                }
            }
            VersionedMessage::V0(message) => {
                let signature = request.message.transaction.signature.parse()?;
                let tx = VersionedTransaction {
                    signatures: vec![signature],
                    message: VersionedMessage::V0(message.clone()),
                };
                match svm.send_transaction(tx) {
                    Ok(meta) => (true, None, meta),
                    Err(failure) => (false, Some(format!("{:?}", failure.err)), failure.meta),
                }
            }
            VersionedMessage::V1(_) => anyhow::bail!("unsupported_runtime_feature: message_v1"),
        };
        timings.transaction_execution = started.elapsed();
        let started = Instant::now();
        let mut post_accounts = BTreeMap::new();
        for address in request.watched {
            let key: Address = address.parse()?;
            ensure!(
                post_accounts
                    .insert(
                        address.clone(),
                        svm.get_account(&key).as_ref().map(snapshot)
                    )
                    .is_none(),
                "duplicate watched address"
            );
        }
        let inner_instructions = meta
            .inner_instructions
            .iter()
            .enumerate()
            .map(|(outer_index, group)| InnerGroup {
                outer_index,
                instructions: group
                    .iter()
                    .map(|inner| InnerInstruction {
                        program_id_index: inner.instruction.program_id_index,
                        accounts: inner.instruction.accounts.clone(),
                        data: inner.instruction.data.clone(),
                        stack_height: inner.stack_height,
                    })
                    .collect(),
            })
            .collect();
        let evidence = ExecutionEvidence {
            success,
            error,
            compute_units: meta.compute_units_consumed,
            fee: meta.fee,
            logs: meta.logs,
            inner_instructions,
            return_data: ReturnData {
                program: meta.return_data.program_id.to_string(),
                data: meta.return_data.data,
            },
            post_accounts,
        };
        timings.evidence_collection = started.elapsed();
        Ok((evidence, timings))
    }
}

impl ExecutionBackend for LiteSvmBackend {
    fn execute(&self, request: &ExecutionRequest<'_>) -> Result<ExecutionEvidence> {
        Ok(self.execute_timed(request)?.0)
    }
}
