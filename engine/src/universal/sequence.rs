//! Contract 3: reconstruct a historical causal sequence, preserving the target
//! boundary separately from the independently observed terminal checkpoint.
use super::{
    checkpoint::ObservedCheckpointV1,
    evidence::{AccountBoundary, EvidenceKind, EvidenceRef, EvidenceStore},
    execution::{
        ExecutionEvidence, ExecutionRequest, LiteSvmBackend, RuntimeProfile, SlotHashesVariant,
    },
    fidelity::compare_execution,
    model::{
        AccountSeed, ExecutionInput, ExpectedAccountSource, ExpectedHistoricalOutcome,
        FidelityProfile, ReplayObservationV2, RuntimeCapability,
    },
    resolver::{
        resolve_account, verify_binary, verify_token_balances, verify_validator_outcome,
        ResolvedReplayInput,
    },
};
use crate::{
    dependencies::{self, ProgramSource},
    replay::hash_bytes,
    types::AccountSnapshot,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

type Frontier = BTreeMap<String, Option<AccountSnapshot>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceEntryV1 {
    pub transaction_index: usize,
    pub signature: String,
    pub execution: ExecutionInput,
    pub full_account_keys: Vec<String>,
    pub expected: ExpectedHistoricalOutcome,
    pub runtime_profile_id: String,
    pub frontier_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyEdge {
    pub from: usize,
    pub to: usize,
    pub account: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalSequenceClosureProofV1 {
    pub schema_version: u32,
    pub slot: u64,
    pub target_transaction_index: usize,
    pub target_signature: String,
    pub full_block: EvidenceRef,
    pub start_checkpoint: EvidenceRef,
    pub terminal_checkpoint: EvidenceRef,
    pub runtime_evidence: EvidenceRef,
    pub program_binaries: Vec<super::model::ProgramBinaryEvidence>,
    pub entries: Vec<SequenceEntryV1>,
    pub dependency_edges: Vec<DependencyEdge>,
    pub terminal_frontier: Vec<String>,
    /// Execution commitment for the distinguished target, never an authority.
    pub target_execution: EvidenceRef,
}

impl HistoricalSequenceClosureProofV1 {
    pub fn store(&self, store: &EvidenceStore) -> Result<EvidenceRef> {
        ensure!(self.schema_version == 1, "unsupported sequence schema");
        store.put(EvidenceKind::ClosureProof, &serde_json::to_vec(self)?)
    }
}

pub fn frontier_digest(frontier: &Frontier) -> Result<String> {
    Ok(hash_bytes(&serde_json::to_vec(&(
        "historical-sequence-frontier-v1",
        frontier,
    ))?))
}

#[derive(Clone, Debug)]
struct CensusRow {
    signature: String,
    keys: Vec<String>,
    inputs: BTreeSet<String>,
    writes: BTreeSet<String>,
    success: bool,
}

fn strings(value: &Value) -> Result<Vec<String>> {
    value
        .as_array()
        .context("block key array missing")?
        .iter()
        .map(|v| Ok(v.as_str().context("block key invalid")?.to_owned()))
        .collect()
}

fn census(block: &Value, programs: &BTreeMap<String, String>) -> Result<Vec<CensusRow>> {
    block["result"]["transactions"]
        .as_array()
        .context("full block transactions missing")?
        .iter()
        .map(|row| {
            let message = &row["transaction"]["message"];
            let mut keys = strings(&message["accountKeys"])?;
            let n = keys.len();
            let header = &message["header"];
            let signed = header["numRequiredSignatures"]
                .as_u64()
                .context("signature count")? as usize;
            let ro_signed = header["numReadonlySignedAccounts"]
                .as_u64()
                .context("readonly signed")? as usize;
            let ro_unsigned = header["numReadonlyUnsignedAccounts"]
                .as_u64()
                .context("readonly unsigned")? as usize;
            ensure!(
                signed > 0 && signed <= n && ro_signed <= signed && ro_unsigned <= n - signed,
                "invalid block message header"
            );
            let mut writes = keys
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    if *i < signed {
                        *i < signed - ro_signed
                    } else {
                        *i < n - ro_unsigned
                    }
                })
                .map(|(_, k)| k.clone())
                .collect::<BTreeSet<_>>();
            if let Some(loaded) = row["meta"]["loadedAddresses"].as_object() {
                let writable = strings(&loaded["writable"])?;
                writes.extend(writable.clone());
                keys.extend(writable);
                keys.extend(strings(&loaded["readonly"])?);
            }
            let mut inputs = keys.iter().cloned().collect::<BTreeSet<_>>();
            if let Some(lookups) = message["addressTableLookups"].as_array() {
                for lookup in lookups {
                    inputs.insert(lookup["accountKey"].as_str().context("lookup key")?.into());
                }
            }
            for key in &keys {
                if let Some(pd) = programs.get(key) {
                    inputs.insert(pd.clone());
                }
            }
            // This bounded sequence contract rejects relevant failed declarations.
            // Thus it never needs to assume persistent-write rollback semantics.
            let success = row["meta"]
                .get("err")
                .context("validator outcome missing")?
                .is_null();
            Ok(CensusRow {
                signature: row["transaction"]["signatures"][0]
                    .as_str()
                    .context("block signature")?
                    .into(),
                keys,
                inputs,
                writes,
                success,
            })
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub struct SequencePlan {
    pub indexes: Vec<usize>,
    pub edges: Vec<DependencyEdge>,
    pub parent: Vec<String>,
    pub terminal: Vec<String>,
}

/// U13.1 conservative last-writer closure, recomputed from raw block messages.
/// Failed overlapping declarations and unanchored outputs fail closed.
pub fn plan(
    block: &Value,
    target: usize,
    outputs: &[String],
    programs: &BTreeMap<String, String>,
) -> Result<SequencePlan> {
    plan_rows(&census(block, programs)?, target, outputs)
}

fn plan_rows(rows: &[CensusRow], target: usize, outputs: &[String]) -> Result<SequencePlan> {
    let target_row = rows.get(target).context("target index outside block")?;
    ensure!(
        !outputs.is_empty() && outputs.iter().all(|k| target_row.keys.contains(k)),
        "invalid target validation outputs"
    );
    let last = |key: &str, before: usize| (0..before).rev().find(|i| rows[*i].writes.contains(key));
    let mut included = BTreeSet::from([target]);
    for (i, row) in rows.iter().enumerate().skip(target + 1) {
        if outputs.iter().any(|k| row.writes.contains(k)) {
            included.insert(i);
        }
    }
    loop {
        let mut added = included.clone();
        for i in &included {
            ensure!(
                rows[*i].success,
                "unsupported failed overlapping sequence transaction"
            );
            for key in &rows[*i].inputs {
                if let Some(p) = last(key, *i) {
                    added.insert(p);
                }
            }
        }
        if added == included {
            break;
        }
        included = added;
    }
    let mut edges = Vec::new();
    let mut parent = BTreeSet::new();
    let mut terminal = BTreeSet::new();
    for i in &included {
        for key in &rows[*i].inputs {
            if let Some(p) = last(key, *i) {
                edges.push(DependencyEdge {
                    from: p,
                    to: *i,
                    account: key.clone(),
                });
            } else {
                parent.insert(key.clone());
            }
        }
        for key in &rows[*i].writes {
            let next = (*i + 1..rows.len()).find(|j| rows[*j].writes.contains(key));
            match next {
                None => {
                    terminal.insert(key.clone());
                }
                Some(j) => ensure!(included.contains(&j), "unanchored sequence terminal output"),
            }
        }
    }
    edges.sort();
    Ok(SequencePlan {
        indexes: included.into_iter().collect(),
        edges,
        parent: parent.into_iter().collect(),
        terminal: terminal.into_iter().collect(),
    })
}

pub(super) fn resolve(
    record: &ReplayObservationV2,
    store: &EvidenceStore,
) -> Result<ResolvedReplayInput> {
    let contract = record
        .checkpointed_execution
        .as_ref()
        .context("sequence contract missing")?;
    ensure!(
        record.fidelity_profile == FidelityProfile::CheckpointedExecutionV1
            && contract.proof_contract_version == 3,
        "sequence requires checkpoint contract 3"
    );
    ensure!(
        contract.closure_proof.kind == EvidenceKind::ClosureProof,
        "sequence evidence category differs"
    );
    let proof: HistoricalSequenceClosureProofV1 =
        serde_json::from_slice(&store.get(&contract.closure_proof)?)?;
    ensure!(
        proof.schema_version == 1
            && proof.slot == record.slot
            && proof.target_signature == record.signature
            && proof.start_checkpoint == contract.start_checkpoint
            && proof.terminal_checkpoint == contract.terminal_checkpoint
            && proof.target_execution == contract.deterministic_execution
            && proof.program_binaries == record.binaries,
        "sequence identity or checkpoint binding differs"
    );
    ensure!(
        proof.full_block.kind == EvidenceKind::Validator,
        "sequence full block category differs"
    );
    let block: Value = serde_json::from_slice(&store.get(&proof.full_block)?)?;
    ensure!(
        block["result"]["parentSlot"].as_u64() == record.slot.checked_sub(1),
        "sequence block parent slot differs"
    );
    let programs: BTreeMap<String, String> = record
        .binaries
        .iter()
        .filter_map(|b| {
            b.programdata_address
                .as_ref()
                .map(|pd| (b.program_id.clone(), pd.clone()))
        })
        .collect();
    let census = census(&block, &programs)?;
    let watched = record
        .expected
        .watched_accounts
        .iter()
        .map(|w| w.address.clone())
        .collect::<Vec<_>>();
    ensure!(
        watched == contract.validation_outputs
            && watched.iter().collect::<BTreeSet<_>>().len() == watched.len(),
        "sequence target output set differs"
    );
    let plan = plan_rows(&census, proof.target_transaction_index, &watched)?;
    ensure!(
        plan.indexes
            == proof
                .entries
                .iter()
                .map(|e| e.transaction_index)
                .collect::<Vec<_>>(),
        "sequence membership or canonical order differs"
    );
    ensure!(
        plan.edges == proof.dependency_edges,
        "sequence dependency edges differ"
    );
    ensure!(
        plan.terminal == proof.terminal_frontier,
        "sequence terminal frontier differs"
    );
    ensure!(
        census[proof.target_transaction_index].signature == record.signature,
        "sequence distinguished target identity differs"
    );
    let start: ObservedCheckpointV1 = serde_json::from_slice(&store.get(&proof.start_checkpoint)?)?;
    ensure!(
        proof.start_checkpoint.kind == EvidenceKind::Checkpoint
            && start.schema_version == 1
            && Some(start.slot) == record.slot.checked_sub(1),
        "sequence start checkpoint identity differs"
    );
    let mut seeds = BTreeMap::new();
    let mut refs = BTreeMap::new();
    let mut absent = Vec::new();
    for entry in &start.accounts {
        ensure!(
            refs.insert(entry.address.clone(), entry.observation.clone())
                .is_none(),
            "duplicate sequence start account"
        );
        ensure!(
            matches!(
                entry.observation.kind,
                EvidenceKind::AccountObservation | EvidenceKind::ChunkedAccountObservation
            ),
            "sequence start must be observed"
        );
        let seed = AccountSeed {
            address: entry.address.clone(),
            boundary: AccountBoundary::BeforeTransaction,
            observation: entry.observation.clone(),
        };
        match resolve_account(store, &seed, record.slot, &record.genesis_hash, 3)? {
            Some(account) => {
                seeds.insert(entry.address.clone(), account);
            }
            None => absent.push(entry.address.clone()),
        }
    }
    // Native runtime accounts are authorized by the historical runtime below.
    let native = record
        .dependencies
        .programs
        .iter()
        .filter(|p| p.source == ProgramSource::Builtin)
        .map(|p| p.program_id.clone())
        .collect::<BTreeSet<_>>();
    ensure!(
        refs.keys().cloned().collect::<BTreeSet<_>>()
            == plan
                .parent
                .iter()
                .filter(|k| !native.contains(*k))
                .cloned()
                .collect(),
        "sequence observed parent frontier differs"
    );
    let supplied = record
        .account_seeds
        .iter()
        .chain(&record.absent_pre_accounts)
        .map(|s| {
            ensure!(
                s.boundary == AccountBoundary::BeforeTransaction,
                "sequence seed boundary differs"
            );
            Ok((s.address.clone(), s.observation.clone()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    ensure!(
        supplied == refs
            && supplied.len() == record.account_seeds.len() + record.absent_pre_accounts.len(),
        "sequence account seeds differ from checkpoint"
    );
    ensure!(
        record
            .account_seeds
            .iter()
            .all(|seed| seeds.contains_key(&seed.address))
            && record
                .absent_pre_accounts
                .iter()
                .all(|seed| absent.contains(&seed.address)),
        "sequence seed presence differs from observation"
    );
    ensure!(
        record.runtime.capability() == RuntimeCapability::SupportedByCurrentBackend,
        "unsupported sequence runtime"
    );
    ensure!(
        proof.runtime_evidence.kind == EvidenceKind::Runtime
            && record.runtime.historical_evidence_ref.as_ref() == Some(&proof.runtime_evidence),
        "sequence runtime evidence binding differs"
    );
    let historical = record
        .runtime
        .historical_evidence
        .as_ref()
        .context("sequence historical runtime missing")?;
    ensure!(
        historical == &serde_json::from_slice(&store.get(&proof.runtime_evidence)?)?,
        "sequence historical runtime differs"
    );
    let mut sysvars = BTreeMap::new();
    for seed in &record.runtime.sysvars {
        ensure!(
            seed.observation.kind == EvidenceKind::AccountObservation
                && seed.boundary == AccountBoundary::EndOfExecutionSlot,
            "sequence runtime sysvar must be observed"
        );
        let account = resolve_account(store, seed, record.slot, &record.genesis_hash, 3)?
            .context("sequence runtime sysvar absent")?;
        ensure!(
            sysvars.insert(seed.address.clone(), account).is_none(),
            "duplicate runtime sysvar"
        );
    }
    ensure!(
        historical
            .historical_feature_set
            .as_ref()
            .is_some_and(|features| features.target_slot == record.slot),
        "sequence requires historical feature-set evidence"
    );
    let runtime = RuntimeProfile::resolve(
        Some(historical),
        &sysvars,
        &record.runtime.feature_profile,
        record.runtime.signature_check,
        record.runtime.blockhash_check,
        &record.runtime.instructions_rule,
        &record.runtime.slot_hashes_policy,
    )?;
    runtime.validate()?;
    let mut baseline = None;
    let mut binaries = BTreeSet::new();
    for binary in &record.binaries {
        ensure!(
            binaries.insert(binary.program_id.clone()),
            "duplicate sequence binary"
        );
        let elf = verify_binary(record, store, &seeds, &refs, binary)?;
        if binary.program_id == record.program_id {
            baseline = Some(elf);
        }
    }
    for dep in &record.dependencies.programs {
        ensure!(
            dep.observed_slot == record.slot.checked_sub(1),
            "sequence dependency slot differs"
        );
        match dep.source {
            ProgramSource::HistoricalMainnet => ensure!(
                binaries.contains(&dep.program_id),
                "sequence historical binary missing"
            ),
            ProgramSource::Builtin => {
                // Fail closed to the native programs supported by this profile.
                ensure!(
                    matches!(
                        dep.program_id.as_str(),
                        "ComputeBudget111111111111111111111111111111"
                            | "11111111111111111111111111111111"
                    ),
                    "unsupported sequence native dependency"
                );
            }
            _ => anyhow::bail!("sequence candidate substitution is forbidden"),
        }
    }
    let immutable = native
        .iter()
        .chain(binaries.iter())
        .chain(programs.values())
        .chain(sysvars.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for i in &plan.indexes {
        ensure!(
            census[*i].writes.is_disjoint(&immutable),
            "sequence changes executable or runtime identity"
        );
    }
    let mut messages = Vec::new();
    let mut discovered = BTreeSet::new();
    for entry in &proof.entries {
        let index = entry.transaction_index;
        ensure!(
            entry.runtime_profile_id == runtime.profile_id,
            "sequence runtime profile differs"
        );
        let message = entry.execution.resolve(store, &record.genesis_hash)?;
        ensure!(
            message.transaction.slot == record.slot
                && message.transaction.signature == entry.signature
                && entry.signature == census[index].signature
                && message.account_keys == entry.full_account_keys
                && message.account_keys == census[index].keys,
            "sequence transaction identity or key order differs"
        );
        let reference = entry
            .execution
            .validator_transaction_ref()
            .context("sequence raw transaction missing")?;
        let raw: Value = serde_json::from_slice(&store.get(reference)?)?;
        let mut block_row = block["result"]["transactions"][index].clone();
        block_row["slot"] = record.slot.into();
        block_row["transactionIndex"] = index.into();
        block_row["blockTime"] = block["result"]["blockTime"].clone();
        ensure!(
            raw == block_row,
            "sequence transaction differs from retained block"
        );
        ensure!(
            entry.expected.watched_accounts.is_empty()
                && entry.expected.compute_units == raw["meta"]["computeUnitsConsumed"].as_u64(),
            "sequence validator envelope coverage differs"
        );
        let mut envelope = record.clone();
        envelope.execution = entry.execution.clone();
        envelope.expected = entry.expected.clone();
        verify_validator_outcome(&envelope, store)?;
        discovered.extend(
            dependencies::discover(&message.transaction, None, &record.program_id)
                .into_iter()
                .map(|(id, _)| id),
        );
        for key in &census[index].inputs {
            ensure!(
                seeds.contains_key(key)
                    || absent.contains(key)
                    || native.contains(key)
                    || sysvars.contains_key(key),
                "sequence input has no observed seed"
            );
        }
        if index == proof.target_transaction_index {
            ensure!(
                entry.execution == record.execution,
                "sequence target message differs"
            );
            let mut expected = record.expected.clone();
            expected.watched_accounts.clear();
            ensure!(
                expected == entry.expected,
                "sequence target envelope differs"
            );
        }
        messages.push(message);
    }
    ensure!(
        discovered
            == record
                .dependencies
                .programs
                .iter()
                .map(|p| p.program_id.clone())
                .collect()
            && discovered.len() == record.dependencies.programs.len(),
        "sequence dependency set differs"
    );
    // LUT bytes used to reconstruct messages must equal the continuing world.
    // This bounded contract excludes in-sequence table changes.
    for entry in &proof.entries {
        if let ExecutionInput::V0 {
            message,
            lookup_tables,
            ..
        } = &entry.execution
        {
            for (lookup, reference) in message.address_table_lookups.iter().zip(lookup_tables) {
                let address = lookup.account_key.to_string();
                ensure!(
                    census.iter().all(|row| !row.writes.contains(&address)),
                    "sequence LUT deployment changed in slot"
                );
                let seed = AccountSeed {
                    address: address.clone(),
                    boundary: AccountBoundary::EndOfExecutionSlot,
                    observation: reference.clone(),
                };
                ensure!(
                    resolve_account(store, &seed, record.slot, &record.genesis_hash, 3)?.as_ref()
                        == seeds.get(&address),
                    "sequence LUT differs from parent world"
                );
            }
        }
    }
    let target_position = proof
        .entries
        .iter()
        .position(|e| e.transaction_index == proof.target_transaction_index)
        .context("sequence target missing")?;
    let target_message = &messages[target_position];
    ensure!(
        record.target.program_id == record.program_id
            && record.target.outer_index < target_message.transaction.instructions.len()
            && target_message.transaction.instructions[record.target.outer_index].program
                == record.program_id
            && record.instruction_roles.len() == target_message.transaction.instructions.len()
            && record
                .instruction_roles
                .iter()
                .enumerate()
                .all(|(i, r)| r.outer_index == i
                    && (r.role == super::model::InstructionRole::SemanticTarget)
                        == (i == record.target.outer_index)),
        "sequence semantic target differs"
    );
    let frontier = seeds
        .keys()
        .chain(absent.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let request = |variant| ExecutionRequest {
        message: &messages[0],
        seeds: &seeds,
        absent_pre_accounts: &absent,
        watched: &frontier,
        runtime_sysvars: &sysvars,
        clock: None,
        programs_to_load: &[],
        runtime_profile: &runtime,
        unlimited_logs: true,
        slot_hashes: variant,
        recent_blockhashes: Default::default(),
        require_complete_state: false,
    };
    ensure!(
        sysvars.keys().all(|key| !seeds.contains_key(key)),
        "sequence parent checkpoint overlaps runtime sysvars"
    );
    let native_accounts = LiteSvmBackend.native_accounts(
        &request(SlotHashesVariant::BackendDefault),
        &native.iter().cloned().collect::<Vec<_>>(),
    )?;
    let runs =
        LiteSvmBackend.execute_sequence(&request(SlotHashesVariant::BackendDefault), &messages)?;
    if record.runtime.slot_hashes_policy == "materiality_checked_default" {
        for variant in [SlotHashesVariant::Empty, SlotHashesVariant::Different] {
            ensure!(
                runs == LiteSvmBackend.execute_sequence(&request(variant), &messages)?,
                "sequence SlotHashes materiality differs"
            );
        }
    }
    let target_execution = verify_reconstruction(record, store, &proof, &runs)?;
    let mut before = seeds.clone();
    before.extend(native_accounts.clone());
    before.extend(sysvars.clone());
    for (message, run) in messages.iter().zip(&runs) {
        let mut after = before.clone();
        for (key, value) in &run.post_accounts {
            match value {
                Some(value) => {
                    after.insert(key.clone(), value.clone());
                }
                None => {
                    after.remove(key);
                }
            }
        }
        for (balances, state, boundary) in [
            (&message.transaction.pre_balances, &before, "pre"),
            (&message.transaction.post_balances, &after, "post"),
        ] {
            let balances = balances
                .as_ref()
                .context("sequence validator balances missing")?;
            ensure!(
                balances.len() == message.account_keys.len(),
                "sequence validator balance length differs"
            );
            for (key, balance) in message.account_keys.iter().zip(balances) {
                ensure!(
                    state.get(key).map_or(0, |account| account.lamports) == *balance,
                    "sequence validator {boundary} balance differs: {key}"
                );
            }
        }
        verify_token_balances(message, &before, true)?;
        verify_token_balances(message, &after, false)?;
        before = after;
    }

    let pre = if target_position == 0 {
        seeds
            .iter()
            .map(|(k, v)| (k.clone(), Some(v.clone())))
            .chain(absent.iter().map(|key| (key.clone(), None)))
            .collect()
    } else {
        runs[target_position - 1].post_accounts.clone()
    };
    let mut target_seeds = pre
        .iter()
        .filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v)))
        .collect::<BTreeMap<_, _>>();
    target_seeds.extend(sysvars.clone());
    // Native program state comes from the same historical runtime constructor.
    // The target executor requires an explicit account census.
    target_seeds.extend(native_accounts);
    let balances = target_message
        .transaction
        .pre_balances
        .as_ref()
        .context("target pre balances missing")?;
    for (key, balance) in target_message.account_keys.iter().zip(balances) {
        ensure!(
            target_seeds.get(key).map_or(0, |a| a.lamports) == *balance,
            "sequence target pre balance differs: {key}"
        );
    }
    verify_token_balances(target_message, &target_seeds, true)?;
    Ok(ResolvedReplayInput {
        message: target_message.clone(),
        seeds: target_seeds,
        runtime_sysvars: sysvars,
        runtime_profile: runtime,
        absent_pre_accounts: pre
            .into_iter()
            .filter_map(|(k, v)| v.is_none().then_some(k))
            .collect(),
        watched,
        expected_accounts: target_execution.post_accounts,
        baseline_elf: baseline.context("sequence target ELF missing")?,
    })
}

/// Validate freshly executed sequence results. Resolution always obtains these
/// from one historical VM; commitments never substitute for those executions.
pub fn verify_reconstruction(
    record: &ReplayObservationV2,
    store: &EvidenceStore,
    proof: &HistoricalSequenceClosureProofV1,
    runs: &[ExecutionEvidence],
) -> Result<ExecutionEvidence> {
    ensure!(
        runs.len() == proof.entries.len() && !runs.is_empty(),
        "sequence execution count differs"
    );
    let watched = &record
        .checkpointed_execution
        .as_ref()
        .context("sequence contract missing")?
        .validation_outputs;
    let target_position = proof
        .entries
        .iter()
        .position(|e| e.transaction_index == proof.target_transaction_index)
        .context("sequence target missing")?;
    for (entry, run) in proof.entries.iter().zip(runs) {
        let mut envelope = run.clone();
        envelope.post_accounts.clear();
        let fidelity = compare_execution(
            &entry.expected,
            &BTreeMap::new(),
            &envelope,
            record.fidelity_profile,
        );
        ensure!(
            fidelity.matched(),
            "sequence transaction {} validator envelope differs: {:?}",
            entry.transaction_index,
            fidelity.failures
        );
        ensure!(
            frontier_digest(&run.post_accounts)? == entry.frontier_digest,
            "sequence transaction {} intermediate frontier differs",
            entry.transaction_index
        );
    }
    let terminal = ObservedCheckpointV1::resolve(
        store,
        &proof.terminal_checkpoint,
        record.slot,
        AccountBoundary::EndOfExecutionSlot,
        &record.genesis_hash,
    )?;
    ensure!(
        terminal.keys().cloned().collect::<Vec<_>>() == proof.terminal_frontier,
        "sequence terminal checkpoint incomplete"
    );
    let final_state = &runs
        .last()
        .context("empty sequence execution")?
        .post_accounts;
    for (key, expected) in &terminal {
        ensure!(
            final_state.get(key) == Some(expected),
            "sequence terminal reconciliation differs: {key}"
        );
    }
    let mut target_execution = runs[target_position].clone();
    target_execution
        .post_accounts
        .retain(|key, _| watched.contains(key));
    ensure!(
        proof.target_execution.kind == EvidenceKind::Execution
            && target_execution
                == serde_json::from_slice::<ExecutionEvidence>(
                    &store.get(&proof.target_execution)?
                )?,
        "derived target execution commitment differs"
    );
    for watched in &record.expected.watched_accounts {
        ensure!(
            watched.source == ExpectedAccountSource::DerivedTargetBoundary,
            "sequence requires derived target boundary"
        );
        let content = watched
            .expected_post_content
            .as_ref()
            .map(|r| -> Result<AccountSnapshot> {
                ensure!(
                    r.kind == EvidenceKind::AccountContent,
                    "target content category differs"
                );
                Ok(serde_json::from_slice::<AccountSnapshot>(&store.get(r)?)?)
            })
            .transpose()?;
        ensure!(
            target_execution.post_accounts.get(&watched.address) == Some(&content),
            "derived target boundary commitment differs"
        );
    }
    Ok(target_execution)
}
