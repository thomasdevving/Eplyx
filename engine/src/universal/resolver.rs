//! Offline, fail-closed conversion from a durable observation to VM inputs.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use serde_json::Value;

use super::{
    checkpoint::{
        DerivedAccountEvidenceV1, DerivedAccountEvidenceV2, ObservedCheckpointV1,
        TransactionClosureProofV1,
    },
    evidence::{
        AccountBoundary, AccountObservation, ChunkedAccountObservation, EvidenceKind, EvidenceRef,
        EvidenceStore,
    },
    execution::{ExecutionEvidence, InnerGroup, InnerInstruction, ReturnData, RuntimeProfile},
    model::{
        AccountSeed, ExpectedAccountSource, FidelityProfile, ReplayObservationV2, ResolvedMessage,
        RuntimeCapability,
    },
};
use crate::{
    dependencies::{self, ProgramSource},
    evidence::token::TokenProgram,
    types::AccountSnapshot,
    versions::{ProgramLoader, LEGACY_BPF_LOADER_ID, UPGRADEABLE_LOADER_ID},
};

pub struct ResolvedReplayInput {
    pub message: ResolvedMessage,
    pub seeds: BTreeMap<String, AccountSnapshot>,
    pub runtime_sysvars: BTreeMap<String, AccountSnapshot>,
    pub runtime_profile: RuntimeProfile,
    pub absent_pre_accounts: Vec<String>,
    pub watched: Vec<String>,
    pub expected_accounts: BTreeMap<String, Option<AccountSnapshot>>,
    pub baseline_elf: Vec<u8>,
}

fn resolve_account(
    store: &EvidenceStore,
    seed: &AccountSeed,
    slot: u64,
    genesis: &str,
    checkpoint_contract: u32,
) -> Result<Option<AccountSnapshot>> {
    match seed.observation.kind {
        EvidenceKind::AccountObservation => AccountObservation::resolve(
            store,
            &seed.observation,
            &seed.address,
            slot,
            seed.boundary,
            genesis,
        ),
        EvidenceKind::ChunkedAccountObservation => {
            ensure!(
                seed.boundary == AccountBoundary::BeforeTransaction,
                "chunked observation can only seed predecessor-slot state"
            );
            Ok(Some(ChunkedAccountObservation::resolve(
                store,
                &seed.observation,
                &seed.address,
                slot,
                genesis,
            )?))
        }
        EvidenceKind::DerivedAccount => Ok(Some(if checkpoint_contract == 2 {
            DerivedAccountEvidenceV2::resolve(
                store,
                &seed.observation,
                &seed.address,
                slot,
                seed.boundary,
            )?
        } else {
            DerivedAccountEvidenceV1::resolve(
                store,
                &seed.observation,
                &seed.address,
                slot,
                seed.boundary,
            )?
        })),
        _ => anyhow::bail!("account seed reference has wrong evidence category"),
    }
}

fn validator_inner(meta: &Value) -> Result<Vec<InnerGroup>> {
    meta["innerInstructions"]
        .as_array()
        .context("validator omitted CPI groups")?
        .iter()
        .map(|group| {
            Ok(InnerGroup {
                outer_index: group["index"].as_u64().context("CPI outer index")? as usize,
                instructions: group["instructions"]
                    .as_array()
                    .context("CPI instructions")?
                    .iter()
                    .map(|ix| {
                        Ok(InnerInstruction {
                            program_id_index: u8::try_from(
                                ix["programIdIndex"].as_u64().context("CPI program index")?,
                            )?,
                            accounts: ix["accounts"]
                                .as_array()
                                .context("CPI accounts")?
                                .iter()
                                .map(|v| {
                                    Ok(u8::try_from(v.as_u64().context("CPI account index")?)?)
                                })
                                .collect::<Result<Vec<_>>>()?,
                            data: bs58::decode(
                                ix["data"].as_str().context("CPI instruction data")?,
                            )
                            .into_vec()?,
                            stack_height: u8::try_from(
                                ix["stackHeight"].as_u64().context("CPI stack height")?,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            })
        })
        .collect()
}

fn verify_validator_outcome(record: &ReplayObservationV2, store: &EvidenceStore) -> Result<()> {
    let frozen_transaction = record
        .execution
        .validator_transaction_ref()
        .context("complete-profile observation requires a frozen validator transaction")?;
    let raw: Value = serde_json::from_slice(&store.get(frozen_transaction)?)?;
    let meta = &raw["meta"];
    ensure!(
        record.expected.success == meta["err"].is_null()
            && record.expected.fee == meta["fee"].as_u64().context("validator fee")?,
        "historical outcome or fee differs from frozen validator result"
    );
    let error = (!meta["err"].is_null()).then(|| meta["err"].to_string());
    ensure!(
        record.expected.error == error,
        "historical error differs from frozen validator result"
    );
    let logs = meta["logMessages"]
        .as_array()
        .context("validator logs")?
        .iter()
        .map(|v| v.as_str().context("validator log line").map(str::to_string))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        record.expected.logs == logs,
        "historical logs differ from frozen validator result"
    );
    ensure!(
        record.expected.inner_instructions == validator_inner(meta)?,
        "historical CPI groups differ from frozen validator result"
    );
    let return_data = if meta["returnData"].is_null() {
        None
    } else {
        Some(ReturnData {
            program: meta["returnData"]["programId"]
                .as_str()
                .context("return program")?
                .into(),
            data: BASE64_STANDARD.decode(
                meta["returnData"]["data"][0]
                    .as_str()
                    .context("return bytes")?,
            )?,
        })
    };
    ensure!(
        record.expected.return_data == return_data,
        "historical return data differs from frozen validator result"
    );
    if let Some(expected) = record.expected.compute_units {
        ensure!(
            meta["computeUnitsConsumed"].as_u64() == Some(expected),
            "historical compute units differ from frozen validator result"
        );
    }
    Ok(())
}

fn verify_deterministic_execution(
    record: &ReplayObservationV2,
    store: &EvidenceStore,
    reference: &EvidenceRef,
    expected_accounts: &BTreeMap<String, Option<AccountSnapshot>>,
) -> Result<()> {
    ensure!(
        reference.kind == EvidenceKind::Execution,
        "deterministic execution reference has wrong evidence category"
    );
    let execution: ExecutionEvidence = serde_json::from_slice(&store.get(reference)?)?;
    ensure!(
        execution.success == record.expected.success
            && execution.error == record.expected.error
            && execution.fee == record.expected.fee
            && record
                .expected
                .compute_units
                .is_some_and(|units| units == execution.compute_units)
            && execution.logs == record.expected.logs,
        "deterministic execution outcome differs from observation"
    );
    let inner = execution
        .inner_instructions
        .iter()
        .filter(|group| !group.instructions.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    ensure!(
        inner == record.expected.inner_instructions,
        "deterministic execution CPI differs from observation"
    );
    match &record.expected.return_data {
        Some(expected) => ensure!(
            &execution.return_data == expected,
            "deterministic execution return data differs"
        ),
        None => ensure!(
            execution.return_data.data.is_empty(),
            "deterministic execution has unexpected return data"
        ),
    }
    ensure!(
        &execution.post_accounts == expected_accounts,
        "derived validation-output post-state differs from terminal checkpoint"
    );
    Ok(())
}

fn verify_token_balances(
    message: &ResolvedMessage,
    accounts: &BTreeMap<String, AccountSnapshot>,
    pre: bool,
) -> Result<()> {
    let entries = if pre {
        &message.transaction.pre_token_balances
    } else {
        &message.transaction.post_token_balances
    };
    for balance in entries.as_deref().unwrap_or(&[]) {
        let address = &message
            .account_keys
            .get(balance.account_index)
            .context("validator token balance index out of range")?;
        let account = accounts
            .get(*address)
            .with_context(|| format!("token account {address} missing"))?;
        let program = TokenProgram::of(&account.owner)
            .context("validator token balance has non-token owner")?;
        ensure!(
            program.address() == balance.program_id,
            "token program owner differs from validator metadata"
        );
        ensure!(
            program.account_amount(&account.data) == Some(balance.amount)
                && program.account_mint(&account.data).as_deref() == Some(balance.mint.as_str()),
            "token account amount or mint differs from validator metadata"
        );
    }
    Ok(())
}

fn verify_binary(
    record: &ReplayObservationV2,
    store: &EvidenceStore,
    seeds: &BTreeMap<String, AccountSnapshot>,
    seed_refs: &BTreeMap<String, EvidenceRef>,
    binary: &super::model::ProgramBinaryEvidence,
) -> Result<Vec<u8>> {
    ensure!(
        binary.elf.kind == EvidenceKind::ProgramBinary,
        "historical binary has wrong evidence category"
    );
    let elf = store.get(&binary.elf)?;
    let executable = seeds
        .get(&binary.program_id)
        .context("historical program header missing")?;
    ensure!(
        seed_refs.get(&binary.program_id) == Some(&binary.executable_account)
            && executable.owner == binary.loader
            && executable.executable,
        "program header/loader identity differs"
    );
    let dependency = record
        .dependencies
        .get(&binary.program_id)
        .context("historical binary missing dependency manifest entry")?;
    ensure!(
        dependency.source == ProgramSource::HistoricalMainnet
            && dependency.binary_sha256.as_deref() == Some(binary.elf.sha256.as_str())
            && dependency.binary_len == Some(elf.len() as u64)
            && dependency.observed_slot == record.slot.checked_sub(1),
        "historical binary dependency identity differs"
    );
    match dependency
        .loader
        .context("historical binary loader missing")?
    {
        ProgramLoader::Legacy => {
            ensure!(
                binary.loader == LEGACY_BPF_LOADER_ID
                    && binary.programdata_address.is_none()
                    && binary.programdata_account.is_none()
                    && binary.deployment_slot.is_none()
                    && executable.data == elf,
                "legacy loader ELF/account identity differs"
            );
        }
        ProgramLoader::Upgradeable => {
            ensure!(
                binary.loader == UPGRADEABLE_LOADER_ID,
                "upgradeable loader identity differs"
            );
            let address = binary
                .programdata_address
                .as_ref()
                .context("ProgramData address missing")?;
            let source = binary
                .programdata_account
                .as_ref()
                .context("ProgramData observation missing")?;
            ensure!(
                seed_refs.get(address) == Some(source),
                "ProgramData observation identity differs"
            );
            let programdata = seeds.get(address).context("ProgramData seed missing")?;
            let key: solana_address::Address = address.parse()?;
            ensure!(
                executable.data.get(4..36) == Some(key.as_ref())
                    && programdata.owner == binary.loader
                    && !programdata.executable
                    && programdata.data.len() >= 45,
                "ProgramData linkage/loader differs"
            );
            let deploy = u64::from_le_bytes(programdata.data[4..12].try_into()?);
            ensure!(
                binary.deployment_slot == Some(deploy)
                    && dependency.deployed_slot == Some(deploy)
                    && deploy < record.slot
                    && programdata.data[45..] == elf,
                "ProgramData deployment or complete ELF bytes differ"
            );
            let authority = if programdata.data[12] == 0 {
                None
            } else {
                Some(
                    solana_address::Address::new_from_array(programdata.data[13..45].try_into()?)
                        .to_string(),
                )
            };
            ensure!(
                binary.upgrade_authority == authority,
                "ProgramData upgrade authority differs"
            );
        }
    }
    Ok(elf)
}

impl ReplayObservationV2 {
    pub fn resolve(&self, store: &EvidenceStore) -> Result<ResolvedReplayInput> {
        self.validate_identity()?;
        let checkpoint_contract = self
            .checkpointed_execution
            .as_ref()
            .map_or(1, |proof| proof.proof_contract_version);
        ensure!(
            checkpoint_contract == 1 || checkpoint_contract == 2,
            "unsupported checkpoint proof contract"
        );
        let message = self.execution.resolve(store, &self.genesis_hash)?;
        ensure!(
            message.transaction.signature == self.signature
                && message.transaction.slot == self.slot,
            "observation transaction identity differs"
        );
        ensure!(
            self.instruction_roles.len() == message.transaction.instructions.len()
                && self.target.outer_index < message.transaction.instructions.len()
                && message.transaction.instructions[self.target.outer_index].program
                    == self.program_id,
            "semantic target or complete envelope differs"
        );
        ensure!(
            self.instruction_roles
                .iter()
                .filter(|r| r.role == super::model::InstructionRole::SemanticTarget)
                .count()
                == 1,
            "this observation requires one explicit semantic target"
        );
        verify_validator_outcome(self, store)?;
        match (
            self.fidelity_profile,
            &self.runtime.historical_evidence,
            &self.runtime.historical_evidence_ref,
        ) {
            (FidelityProfile::CheckpointedExecutionV1, Some(inline), Some(reference)) => {
                ensure!(
                    reference.kind == EvidenceKind::Runtime,
                    "historical runtime reference has wrong evidence category"
                );
                let retained = serde_json::from_slice(&store.get(reference)?)?;
                ensure!(
                    inline == &retained,
                    "historical runtime evidence differs from CAS object"
                );
            }
            (FidelityProfile::CheckpointedExecutionV1, _, _) => {
                anyhow::bail!("checkpoint-derived profile requires retained runtime evidence")
            }
            (_, Some(inline), Some(reference)) => {
                ensure!(
                    reference.kind == EvidenceKind::Runtime,
                    "historical runtime reference has wrong evidence category"
                );
                let retained = serde_json::from_slice(&store.get(reference)?)?;
                ensure!(
                    inline == &retained,
                    "historical runtime evidence differs from CAS object"
                );
            }
            _ => {}
        }
        match self.runtime.capability() {
            RuntimeCapability::SupportedByCurrentBackend => {}
            RuntimeCapability::UnsupportedRuntimeFeature(feature) => {
                anyhow::bail!("unsupported_runtime_feature: {feature}")
            }
            RuntimeCapability::InsufficientRuntimeEvidence(evidence) => {
                anyhow::bail!("insufficient_runtime_evidence: {evidence}")
            }
        }
        let mut seeds = BTreeMap::new();
        let mut seed_refs = BTreeMap::new();
        for seed in &self.account_seeds {
            let account = resolve_account(
                store,
                seed,
                self.slot,
                &self.genesis_hash,
                checkpoint_contract,
            )?
            .context("present account seed is absent")?;
            ensure!(
                seeds.insert(seed.address.clone(), account).is_none()
                    && seed_refs
                        .insert(seed.address.clone(), seed.observation.clone())
                        .is_none(),
                "duplicate account seed"
            );
        }
        let mut runtime_sysvars = BTreeMap::new();
        for seed in &self.runtime.sysvars {
            let valid_boundary = seed.boundary == AccountBoundary::EndOfExecutionSlot
                || (self.fidelity_profile == FidelityProfile::CheckpointedExecutionV1
                    && seed.boundary == AccountBoundary::BeforeTargetExecution
                    && seed.observation.kind == EvidenceKind::DerivedAccount);
            ensure!(
                valid_boundary,
                "runtime sysvar requires proven execution context"
            );
            let account = resolve_account(
                store,
                seed,
                self.slot,
                &self.genesis_hash,
                checkpoint_contract,
            )?
            .context("runtime sysvar absent")?;
            ensure!(
                runtime_sysvars
                    .insert(seed.address.clone(), account.clone())
                    .is_none()
                    && seeds.insert(seed.address.clone(), account).is_none()
                    && seed_refs
                        .insert(seed.address.clone(), seed.observation.clone())
                        .is_none(),
                "duplicate runtime account"
            );
        }
        for required in [
            "SysvarC1ock11111111111111111111111111111111",
            "SysvarRent111111111111111111111111111111111",
            "SysvarEpochSchedu1e111111111111111111111111",
        ] {
            ensure!(
                runtime_sysvars.contains_key(required),
                "insufficient_runtime_evidence: required historical sysvar {required}"
            );
        }
        let mut absent = BTreeSet::new();
        for seed in &self.absent_pre_accounts {
            ensure!(
                seed.boundary == AccountBoundary::BeforeTransaction
                    && resolve_account(
                        store,
                        seed,
                        self.slot,
                        &self.genesis_hash,
                        checkpoint_contract
                    )?
                    .is_none()
                    && absent.insert(seed.address.clone()),
                "absence evidence differs"
            );
        }
        ensure!(
            message
                .account_keys
                .iter()
                .all(|key| seeds.contains_key(key)
                    || absent.contains(key)
                    || key == "Sysvar1nstructions1111111111111111111111111"),
            "required message account has no seed or absence evidence"
        );
        if let Some(balances) = &message.transaction.pre_balances {
            ensure!(
                balances.len() == message.account_keys.len(),
                "validator pre-balance vector length differs"
            );
            for (key, balance) in message.account_keys.iter().zip(balances) {
                if key == "Sysvar1nstructions1111111111111111111111111" {
                    continue;
                }
                ensure!(
                    seeds.get(key).map(|a| a.lamports).unwrap_or(0) == *balance,
                    "historical account seed contradicts validator pre-balance: {key}"
                );
            }
        } else {
            anyhow::bail!("validator pre-balance evidence missing");
        }
        verify_token_balances(&message, &seeds, true)?;
        let mut discovered = dependencies::discover(&message.transaction, None, &self.program_id)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<BTreeSet<_>>();
        let manifest_ids = self
            .dependencies
            .programs
            .iter()
            .map(|p| p.program_id.clone())
            .collect::<BTreeSet<_>>();
        ensure!(
            discovered == manifest_ids,
            "execution dependency set differs from transaction evidence"
        );
        discovered.clear();
        let mut baseline_elf = None;
        let mut binary_ids = BTreeSet::new();
        for binary in &self.binaries {
            ensure!(
                binary_ids.insert(binary.program_id.clone()),
                "duplicate historical binary"
            );
            let elf = verify_binary(self, store, &seeds, &seed_refs, binary)?;
            if binary.program_id == self.program_id {
                baseline_elf = Some(elf);
            }
        }
        for dependency in &self.dependencies.programs {
            ensure!(
                dependency.observed_slot == self.slot.checked_sub(1),
                "dependency observed at wrong historical slot"
            );
            match dependency.source {
                ProgramSource::HistoricalMainnet => ensure!(
                    binary_ids.contains(&dependency.program_id),
                    "required historical binary missing"
                ),
                ProgramSource::Builtin => {
                    let account = seeds
                        .get(&dependency.program_id)
                        .context("native program account missing")?;
                    ensure!(
                        account.owner == crate::versions::NATIVE_LOADER_ID && account.executable,
                        "builtin program identity differs"
                    );
                }
                _ => anyhow::bail!("unsupported_runtime_feature: dependency source"),
            }
        }
        let baseline_elf = baseline_elf.context("semantic target historical ELF missing")?;
        let mut expected_accounts = BTreeMap::new();
        let mut watched = Vec::new();
        let mut post_present = BTreeMap::new();
        for watched_account in &self.expected.watched_accounts {
            let address = &watched_account.address;
            ensure!(
                !expected_accounts.contains_key(address),
                "duplicate watched account"
            );
            let source = match &watched_account.source {
                ExpectedAccountSource::Archived(reference) => {
                    let seed = AccountSeed {
                        address: address.clone(),
                        boundary: AccountBoundary::EndOfExecutionSlot,
                        observation: reference.clone(),
                    };
                    resolve_account(
                        store,
                        &seed,
                        self.slot,
                        &self.genesis_hash,
                        checkpoint_contract,
                    )?
                }
                ExpectedAccountSource::PreRetained(reference) => {
                    ensure!(
                        seed_refs.get(address) == Some(reference),
                        "retained pre-state reference differs"
                    );
                    Some(
                        seeds
                            .get(address)
                            .context("retained pre-state account missing")?
                            .clone(),
                    )
                }
            };
            let content = watched_account
                .expected_post_content
                .as_ref()
                .map(|reference| {
                    ensure!(
                        reference.kind == EvidenceKind::AccountContent,
                        "expected account content category differs"
                    );
                    serde_json::from_slice::<AccountSnapshot>(&store.get(reference)?)
                        .map_err(Into::into)
                })
                .transpose()?;
            ensure!(
                source == content,
                "expected post-state content differs from historical source"
            );
            let index = message
                .account_keys
                .iter()
                .position(|key| key == address)
                .context("watched account not in message")?;
            let post_balance = message
                .transaction
                .post_balances
                .as_ref()
                .and_then(|values| values.get(index))
                .context("validator post-balance missing")?;
            ensure!(
                content.as_ref().map(|a| a.lamports).unwrap_or(0) == *post_balance,
                "expected watched account contradicts validator post-balance"
            );
            if let Some(account) = &content {
                post_present.insert(address.clone(), account.clone());
            }
            expected_accounts.insert(address.clone(), content);
            watched.push(address.clone());
        }
        verify_token_balances(&message, &post_present, false)?;
        match self.fidelity_profile {
            FidelityProfile::CompleteExecutionV2 => ensure!(
                self.expected.watched_accounts.len()
                    == message
                        .account_keys
                        .iter()
                        .filter(|k| *k != "Sysvar1nstructions1111111111111111111111111")
                        .count(),
                "complete watched account vector required"
            ),
            FidelityProfile::CheckpointedExecutionV1 => {
                let proof = self
                    .checkpointed_execution
                    .as_ref()
                    .context("checkpoint-derived proof missing")?;
                ensure!(
                    proof.validation_outputs == watched,
                    "validation-output set differs from watched accounts"
                );
                let start = ObservedCheckpointV1::resolve(
                    store,
                    &proof.start_checkpoint,
                    self.slot,
                    AccountBoundary::BeforeTransaction,
                    &self.genesis_hash,
                )?;
                ensure!(
                    start.keys().cloned().collect::<Vec<_>>()
                        == message
                            .account_keys
                            .iter()
                            .cloned()
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect::<Vec<_>>(),
                    "start checkpoint account set differs from execution inputs"
                );
                for (address, observed) in &start {
                    match observed {
                        Some(account) => ensure!(
                            seeds.get(address) == Some(account),
                            "execution seed differs from observed start checkpoint: {address}"
                        ),
                        None => ensure!(
                            absent.contains(address) || runtime_sysvars.contains_key(address),
                            "observed-absent start account was silently seeded: {address}"
                        ),
                    }
                }
                let terminal = ObservedCheckpointV1::resolve(
                    store,
                    &proof.terminal_checkpoint,
                    self.slot,
                    AccountBoundary::EndOfExecutionSlot,
                    &self.genesis_hash,
                )?;
                ensure!(
                    terminal == expected_accounts,
                    "terminal checkpoint differs from validation-output evidence"
                );
                TransactionClosureProofV1::verify(
                    store,
                    &proof.closure_proof,
                    self,
                    &message,
                    &proof.validation_outputs,
                    proof.proof_contract_version,
                )?;
                verify_deterministic_execution(
                    self,
                    store,
                    &proof.deterministic_execution,
                    &expected_accounts,
                )?;
            }
            FidelityProfile::HistoricalReplayV1 => unreachable!("identity validation rejects V1"),
        }
        let runtime_profile = RuntimeProfile::resolve(
            self.runtime.historical_evidence.as_ref(),
            &runtime_sysvars,
            &self.runtime.feature_profile,
            self.runtime.signature_check,
            self.runtime.blockhash_check,
            &self.runtime.instructions_rule,
            &self.runtime.slot_hashes_policy,
        )?;
        Ok(ResolvedReplayInput {
            message,
            seeds,
            runtime_sysvars,
            runtime_profile,
            absent_pre_accounts: absent.into_iter().collect(),
            watched,
            expected_accounts,
            baseline_elf,
        })
    }
}
