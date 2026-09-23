//! Protocol-independent observation and execution-message identity.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use solana_message::{v0, Message, VersionedMessage};

use super::evidence::{
    AccountBoundary, AccountObservation, EvidenceKind, EvidenceRef, EvidenceStore,
};
use super::execution::{HistoricalRuntimeEvidence, InnerGroup, ReturnData};
use crate::{
    dependencies::DependencyManifest,
    ingest::transactions::HistoricalTransaction,
    message::{self, FrozenV0, HistoricalAccountEvidence, LutResolutionProof},
    replay::hash_bytes,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum ExecutionInput {
    Legacy {
        message: Message,
        transaction: HistoricalTransaction,
    },
    /// V1 records already flattened LUT-free v0 before this model existed.
    /// This compatibility variant cannot be used by a schema-2 observation.
    LegacyV1Compatibility {
        message: Message,
        transaction: HistoricalTransaction,
    },
    V0 {
        message: v0::Message,
        frozen_transaction: EvidenceRef,
        lookup_tables: Vec<EvidenceRef>,
        slot_hashes: Option<EvidenceRef>,
        claimed_proof: LutResolutionProof,
    },
}

/// Loaded keys are derived during resolution, never accepted from a record.
#[derive(Clone, Debug)]
pub struct ResolvedMessage {
    pub message: VersionedMessage,
    pub transaction: HistoricalTransaction,
    pub account_keys: Vec<String>,
}

impl ExecutionInput {
    /// The immutable validator envelope used by complete-profile fidelity.
    /// V1 legacy records predate this requirement; new message variants must
    /// provide it before they can be admitted as schema-2 observations.
    pub fn validator_transaction_ref(&self) -> Option<&EvidenceRef> {
        match self {
            Self::V0 {
                frozen_transaction, ..
            } => Some(frozen_transaction),
            Self::Legacy { .. } | Self::LegacyV1Compatibility { .. } => None,
        }
    }

    pub fn identity(&self) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            "eplyx-execution-input-v1",
            self,
        ))?))
    }

    pub fn resolve(&self, store: &EvidenceStore, genesis: &str) -> Result<ResolvedMessage> {
        match self {
            Self::Legacy {
                message,
                transaction,
            }
            | Self::LegacyV1Compatibility {
                message,
                transaction,
            } => {
                let compatible_v0 = matches!(self, Self::LegacyV1Compatibility { .. })
                    && transaction.version == "v0";
                ensure!(
                    (transaction.version == "legacy" || compatible_v0)
                        && transaction.loaded_address_count == 0,
                    "legacy input has a different message format"
                );
                ensure!(
                    message
                        .account_keys
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        == transaction
                            .account_keys
                            .iter()
                            .map(|k| k.address.clone())
                            .collect::<Vec<_>>(),
                    "legacy message key space differs from transaction"
                );
                ensure!(
                    message.recent_blockhash.to_string() == transaction.recent_blockhash,
                    "legacy recent blockhash differs"
                );
                ensure!(
                    message.instructions.len() == transaction.instructions.len(),
                    "legacy instruction count differs"
                );
                for (compiled, normalized) in
                    message.instructions.iter().zip(&transaction.instructions)
                {
                    let program = message
                        .account_keys
                        .get(usize::from(compiled.program_id_index))
                        .context("legacy program index out of range")?;
                    ensure!(
                        program.to_string() == normalized.program
                            && compiled.data == normalized.data,
                        "legacy compiled instruction differs"
                    );
                    let addresses = compiled
                        .accounts
                        .iter()
                        .map(|i| {
                            message
                                .account_keys
                                .get(usize::from(*i))
                                .map(ToString::to_string)
                                .context("legacy account index out of range")
                        })
                        .collect::<Result<Vec<_>>>()?;
                    ensure!(
                        addresses
                            == normalized
                                .accounts
                                .iter()
                                .map(|a| a.address.clone())
                                .collect::<Vec<_>>(),
                        "legacy compiled account indexes differ"
                    );
                }
                Ok(ResolvedMessage {
                    message: VersionedMessage::Legacy(message.clone()),
                    transaction: transaction.clone(),
                    account_keys: message
                        .account_keys
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                })
            }
            Self::V0 {
                message,
                frozen_transaction,
                lookup_tables,
                slot_hashes,
                claimed_proof,
            } => {
                ensure!(
                    frozen_transaction.kind == EvidenceKind::Transaction,
                    "frozen transaction reference has wrong kind"
                );
                let raw: serde_json::Value =
                    serde_json::from_slice(&store.get(frozen_transaction)?)?;
                let frozen = FrozenV0::from_rpc(&raw, genesis)?;
                ensure!(
                    frozen.native_message() == message,
                    "native v0 message differs from frozen transaction"
                );
                let slot = frozen.execution_slot();
                let read_account =
                    |reference: &EvidenceRef, address: &str| -> Result<HistoricalAccountEvidence> {
                        let observation: AccountObservation =
                            serde_json::from_slice(&store.get(reference)?)?;
                        let (_, raw) = AccountObservation::resolve_with_raw(
                            store,
                            reference,
                            address,
                            slot,
                            AccountBoundary::EndOfExecutionSlot,
                            genesis,
                        )?;
                        HistoricalAccountEvidence::from_response(
                            address,
                            slot,
                            observation.provider,
                            &raw,
                        )
                    };
                ensure!(
                    lookup_tables.len() == message.address_table_lookups.len(),
                    "v0 lookup evidence count differs from native message"
                );
                let mut tables = Vec::new();
                for (reference, descriptor) in
                    lookup_tables.iter().zip(&message.address_table_lookups)
                {
                    tables.push(read_account(
                        reference,
                        &descriptor.account_key.to_string(),
                    )?);
                }
                let slot_hashes_evidence = slot_hashes
                    .as_ref()
                    .map(|reference| read_account(reference, message::SLOT_HASHES_ID))
                    .transpose()?;
                let proven = message::validate_proof(
                    &frozen,
                    &tables,
                    slot_hashes_evidence.as_ref(),
                    claimed_proof,
                )
                .map_err(|error| {
                    anyhow::anyhow!("v0 LUT proof stage {}: {}", error.stage, error.detail)
                })?;
                ensure!(
                    proven.versioned_message() == VersionedMessage::V0(message.clone()),
                    "resolved native message differs"
                );
                Ok(ResolvedMessage {
                    message: proven.versioned_message(),
                    transaction: proven.transaction().clone(),
                    account_keys: proven
                        .proof()
                        .full_account_keys
                        .iter()
                        .map(|k| k.address.clone())
                        .collect(),
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FidelityProfile {
    HistoricalReplayV1,
    CompleteExecutionV2,
    CheckpointedExecutionV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionRole {
    SemanticTarget,
    TargetPrerequisite,
    ExecutionDependency,
    StandardCompanion,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticTarget {
    pub program_id: String,
    pub outer_index: usize,
    pub instruction_identity: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionAssignment {
    pub outer_index: usize,
    pub role: InstructionRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSeed {
    pub address: String,
    pub boundary: AccountBoundary,
    pub observation: EvidenceRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeContext {
    pub sysvars: Vec<AccountSeed>,
    pub feature_profile: String,
    pub slot_hashes_policy: String,
    pub signature_check: bool,
    pub blockhash_check: bool,
    pub instructions_rule: String,
    pub provenance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_evidence: Option<HistoricalRuntimeEvidence>,
    /// CAS copy of `historical_evidence`. Checkpoint-derived profiles require
    /// the independently inventoried object; older observations remain byte
    /// stable because this field is omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_evidence_ref: Option<EvidenceRef>,
}

/// A runtime limitation is about the replay backend or its evidence, never a
/// verdict that the Solana protocol itself is unsupported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum RuntimeCapability {
    SupportedByCurrentBackend,
    UnsupportedRuntimeFeature(String),
    InsufficientRuntimeEvidence(String),
}

impl RuntimeContext {
    pub fn capability(&self) -> RuntimeCapability {
        if let Some(evidence) = &self.historical_evidence {
            if let Err(error) = evidence.validate() {
                return RuntimeCapability::InsufficientRuntimeEvidence(error.to_string());
            }
            if evidence.feature_profile != self.feature_profile {
                return RuntimeCapability::InsufficientRuntimeEvidence(
                    "runtime feature profile binding".into(),
                );
            }
        }
        if self.feature_profile != "LiteSVM 0.16.0 mainnet"
            && !(self.feature_profile == "LiteSVM 0.16.0 historical evidence"
                && self
                    .historical_evidence
                    .as_ref()
                    .is_some_and(|e| e.historical_feature_set.is_some()))
        {
            return RuntimeCapability::UnsupportedRuntimeFeature(format!(
                "feature profile {}",
                self.feature_profile
            ));
        }
        if self.instructions_rule != "runtime_generated_from_complete_message" {
            return RuntimeCapability::UnsupportedRuntimeFeature(format!(
                "Instructions construction {}",
                self.instructions_rule
            ));
        }
        if self.signature_check || self.blockhash_check {
            return RuntimeCapability::UnsupportedRuntimeFeature(
                "signature or blockhash verification policy".into(),
            );
        }
        if self.slot_hashes_policy != "materiality_checked_default"
            && self.slot_hashes_policy != "historical"
        {
            return RuntimeCapability::UnsupportedRuntimeFeature(format!(
                "SlotHashes policy {}",
                self.slot_hashes_policy
            ));
        }
        if self.provenance.is_empty() {
            return RuntimeCapability::InsufficientRuntimeEvidence("runtime provenance".into());
        }
        for required in [
            "SysvarC1ock11111111111111111111111111111111",
            "SysvarRent111111111111111111111111111111111",
            "SysvarEpochSchedu1e111111111111111111111111",
        ] {
            if !self.sysvars.iter().any(|seed| seed.address == required) {
                return RuntimeCapability::InsufficientRuntimeEvidence(format!(
                    "required historical sysvar {required}"
                ));
            }
        }
        if self.slot_hashes_policy == "historical"
            && !self
                .sysvars
                .iter()
                .any(|seed| seed.address == crate::message::SLOT_HASHES_ID)
        {
            return RuntimeCapability::InsufficientRuntimeEvidence(
                "historical SlotHashes account".into(),
            );
        }
        RuntimeCapability::SupportedByCurrentBackend
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramBinaryEvidence {
    pub program_id: String,
    pub loader: String,
    pub programdata_address: Option<String>,
    pub deployment_slot: Option<u64>,
    pub upgrade_authority: Option<String>,
    pub elf: EvidenceRef,
    pub executable_account: EvidenceRef,
    pub programdata_account: Option<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedAccount {
    pub address: String,
    pub expected_post_content: Option<EvidenceRef>,
    pub source: ExpectedAccountSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "observation", rename_all = "snake_case")]
pub enum ExpectedAccountSource {
    Archived(EvidenceRef),
    PreRetained(EvidenceRef),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedHistoricalOutcome {
    pub success: bool,
    pub error: Option<String>,
    pub fee: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_units: Option<u64>,
    pub logs: Vec<String>,
    pub inner_instructions: Vec<InnerGroup>,
    pub return_data: Option<ReturnData>,
    pub watched_accounts: Vec<WatchedAccount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointedExecutionProof {
    #[serde(
        default = "legacy_checkpoint_contract",
        skip_serializing_if = "is_legacy_checkpoint_contract"
    )]
    pub proof_contract_version: u32,
    pub start_checkpoint: EvidenceRef,
    pub terminal_checkpoint: EvidenceRef,
    pub closure_proof: EvidenceRef,
    pub deterministic_execution: EvidenceRef,
    pub validation_outputs: Vec<String>,
}

fn legacy_checkpoint_contract() -> u32 {
    1
}
fn is_legacy_checkpoint_contract(version: &u32) -> bool {
    *version == 1
}

/// Schema 2 is a small manifest: account and binary bytes live in shared CAS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayObservationV2 {
    pub schema_version: u32,
    pub id: String,
    pub protocol: String,
    pub program_id: String,
    pub genesis_hash: String,
    pub signature: String,
    pub slot: u64,
    pub execution: ExecutionInput,
    pub target: SemanticTarget,
    pub instruction_roles: Vec<InstructionAssignment>,
    pub account_seeds: Vec<AccountSeed>,
    pub absent_pre_accounts: Vec<AccountSeed>,
    pub binaries: Vec<ProgramBinaryEvidence>,
    pub dependencies: DependencyManifest,
    pub runtime: RuntimeContext,
    pub expected: ExpectedHistoricalOutcome,
    pub fidelity_profile: FidelityProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpointed_execution: Option<CheckpointedExecutionProof>,
}

impl ReplayObservationV2 {
    pub fn identity(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.id.clear();
        Ok(hash_bytes(&serde_json::to_vec(&(
            "eplyx-replay-observation-v2",
            copy,
        ))?))
    }

    pub fn validate_identity(&self) -> Result<()> {
        ensure!(self.schema_version == 2, "wrong observation schema");
        match self.fidelity_profile {
            FidelityProfile::CompleteExecutionV2 => ensure!(
                self.checkpointed_execution.is_none(),
                "complete execution fidelity cannot claim checkpoint-derived provenance"
            ),
            FidelityProfile::CheckpointedExecutionV1 => ensure!(
                self.checkpointed_execution.is_some(),
                "checkpoint-derived fidelity requires its proof"
            ),
            FidelityProfile::HistoricalReplayV1 => {
                anyhow::bail!("new observations cannot use historical replay V1 fidelity")
            }
        }
        ensure!(
            !matches!(self.execution, ExecutionInput::LegacyV1Compatibility { .. }),
            "legacy v0 flattening is restricted to historical V1 records"
        );
        ensure!(self.id == self.identity()?, "observation identity differs");
        ensure!(
            self.target.program_id == self.program_id,
            "semantic target program differs"
        );
        let mut roles = self.instruction_roles.clone();
        roles.sort_by_key(|r| r.outer_index);
        ensure!(
            roles
                .iter()
                .enumerate()
                .all(|(i, role)| role.outer_index == i),
            "every outer instruction needs one ordered role"
        );
        ensure!(
            roles
                .get(self.target.outer_index)
                .is_some_and(|r| r.role == InstructionRole::SemanticTarget),
            "target outer index does not name a semantic target"
        );
        Ok(())
    }
}
