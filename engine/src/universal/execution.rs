//! One historical execution contract for legacy and native v0 messages.
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use anyhow::{anyhow, ensure, Result};
use litesvm::{InvocationInspectCallback, LiteSVM};
use serde::{Deserialize, Serialize};
use solana_account::Account;
use solana_address::Address;
use solana_clock::Clock;
use solana_hash::Hash;
use solana_message::VersionedMessage;
use solana_program_runtime::invoke_context::InvokeContext;
use solana_slot_hashes::SlotHashes;
use solana_transaction::{
    sanitized::SanitizedTransaction, versioned::VersionedTransaction, Transaction,
};

use super::model::ResolvedMessage;
use crate::{executor::LoadedProgram, replay::hash_bytes, types::AccountSnapshot};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalRuntimeEvidence {
    pub evidence_id: String,
    pub environment_blockhash: String,
    pub sysvar_snapshot_hash: String,
    pub feature_profile: String,
    pub native_program_profile: String,
    pub provenance: String,
}

impl HistoricalRuntimeEvidence {
    pub fn new(
        environment_blockhash: String,
        sysvar_snapshot_hash: String,
        feature_profile: String,
        native_program_profile: String,
        provenance: String,
    ) -> Result<Self> {
        let mut evidence = Self {
            evidence_id: String::new(),
            environment_blockhash,
            sysvar_snapshot_hash,
            feature_profile,
            native_program_profile,
            provenance,
        };
        evidence.evidence_id = evidence.identity()?;
        Ok(evidence)
    }

    pub fn identity(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.evidence_id.clear();
        Ok(hash_bytes(&serde_json::to_vec(&(
            "eplyx-historical-runtime-evidence-v1",
            copy,
        ))?))
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.environment_blockhash.is_empty()
                && !self.sysvar_snapshot_hash.is_empty()
                && !self.feature_profile.is_empty()
                && !self.native_program_profile.is_empty()
                && !self.provenance.is_empty(),
            "historical runtime evidence is incomplete"
        );
        self.environment_blockhash
            .parse::<Hash>()
            .map_err(|error| anyhow!("historical environment blockhash: {error}"))?;
        ensure!(
            self.evidence_id == self.identity()?,
            "historical runtime evidence identity differs"
        );
        Ok(())
    }
}

/// Fully resolved runtime configuration. Historical evidence remains a distinct
/// input and is bound here only after its identity and referenced sysvars pass.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProfile {
    pub profile_id: String,
    pub historical_evidence_id: Option<String>,
    pub environment_blockhash: Option<String>,
    pub sysvar_snapshot_hash: String,
    pub feature_profile: String,
    pub native_program_profile: String,
    pub signature_check: bool,
    pub recent_blockhash_check: bool,
    pub instructions_rule: String,
    pub slot_hashes_policy: String,
}

impl RuntimeProfile {
    pub fn sysvar_snapshot_hash(
        runtime_sysvars: &BTreeMap<String, AccountSnapshot>,
    ) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            "eplyx-runtime-sysvars-v1",
            runtime_sysvars,
        ))?))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        evidence: Option<&HistoricalRuntimeEvidence>,
        runtime_sysvars: &BTreeMap<String, AccountSnapshot>,
        feature_profile: &str,
        signature_check: bool,
        recent_blockhash_check: bool,
        instructions_rule: &str,
        slot_hashes_policy: &str,
    ) -> Result<Self> {
        let sysvar_snapshot_hash = Self::sysvar_snapshot_hash(runtime_sysvars)?;
        let (historical_evidence_id, environment_blockhash, native_program_profile) =
            if let Some(evidence) = evidence {
                evidence.validate()?;
                ensure!(
                    evidence.sysvar_snapshot_hash == sysvar_snapshot_hash,
                    "historical runtime sysvar snapshot differs"
                );
                ensure!(
                    evidence.feature_profile == feature_profile,
                    "historical runtime feature profile differs"
                );
                (
                    Some(evidence.evidence_id.clone()),
                    Some(evidence.environment_blockhash.clone()),
                    evidence.native_program_profile.clone(),
                )
            } else {
                (None, None, "litesvm-default-native-programs".into())
            };
        let mut profile = Self {
            profile_id: String::new(),
            historical_evidence_id,
            environment_blockhash,
            sysvar_snapshot_hash,
            feature_profile: feature_profile.into(),
            native_program_profile,
            signature_check,
            recent_blockhash_check,
            instructions_rule: instructions_rule.into(),
            slot_hashes_policy: slot_hashes_policy.into(),
        };
        profile.profile_id = profile.identity()?;
        Ok(profile)
    }

    pub fn legacy(signature_check: bool, recent_blockhash_check: bool) -> Result<Self> {
        Self::resolve(
            None,
            &BTreeMap::new(),
            "LiteSVM 0.16.0 mainnet",
            signature_check,
            recent_blockhash_check,
            "runtime_generated_from_complete_message",
            "materiality_checked_default",
        )
    }

    pub fn identity(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.profile_id.clear();
        Ok(hash_bytes(&serde_json::to_vec(&(
            "eplyx-runtime-profile-v1",
            copy,
        ))?))
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.profile_id == self.identity()?,
            "resolved runtime profile identity differs"
        );
        if let Some(blockhash) = &self.environment_blockhash {
            blockhash
                .parse::<Hash>()
                .map_err(|error| anyhow!("runtime environment blockhash: {error}"))?;
        }
        ensure!(
            !self.sysvar_snapshot_hash.is_empty()
                && !self.feature_profile.is_empty()
                && !self.native_program_profile.is_empty()
                && self.instructions_rule == "runtime_generated_from_complete_message",
            "resolved runtime profile is incomplete"
        );
        match &self.historical_evidence_id {
            Some(_) => ensure!(
                self.environment_blockhash.is_some()
                    && self.feature_profile == "LiteSVM 0.16.0 mainnet"
                    && self.native_program_profile == "agave-4.2.2-native-system-compute",
                "unsupported historical runtime profile"
            ),
            None => ensure!(
                self.environment_blockhash.is_none()
                    && self.feature_profile == "LiteSVM 0.16.0 mainnet"
                    && self.native_program_profile == "litesvm-default-native-programs",
                "unsupported default runtime profile"
            ),
        }
        Ok(())
    }
}

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
    pub runtime_profile: &'a RuntimeProfile,
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

/// Injects the bank/environment blockhash without rewriting the historical
/// transaction's recent blockhash. LiteSVM's `latest_blockhash` is only an age
/// check input; native programs read this distinct value from InvokeContext.
#[derive(Clone, Copy)]
struct HistoricalEnvironmentBlockhash(Hash);

impl InvocationInspectCallback for HistoricalEnvironmentBlockhash {
    fn before_invocation(
        &self,
        _svm: &LiteSVM,
        _tx: &SanitizedTransaction,
        _program_indices: &[u16],
        invoke_context: &mut InvokeContext<'_, '_>,
        _enable_register_tracing: bool,
    ) {
        invoke_context.environment_config.blockhash = self.0;
    }

    fn after_invocation(
        &self,
        _svm: &LiteSVM,
        _tx: &SanitizedTransaction,
        _program_indices: &[u16],
        _invoke_context: &InvokeContext<'_, '_>,
        _enable_register_tracing: bool,
    ) {
    }
}

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
        request.runtime_profile.validate()?;
        ensure!(
            request.runtime_profile.sysvar_snapshot_hash
                == RuntimeProfile::sysvar_snapshot_hash(request.runtime_sysvars)?,
            "runtime sysvar snapshot differs from resolved profile"
        );
        let mut svm = LiteSVM::new()
            .with_sigverify(request.runtime_profile.signature_check)
            .with_blockhash_check(request.runtime_profile.recent_blockhash_check);
        if let Some(blockhash) = &request.runtime_profile.environment_blockhash {
            svm.set_invocation_inspect_callback(HistoricalEnvironmentBlockhash(blockhash.parse()?));
        }
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
