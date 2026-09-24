//! Convert retained U13 raw receipts into ordinary contract-3 product inputs.
//! No feasibility result is read. All commitments are freshly executed.
use anyhow::{ensure, Context, Result};
use eplyx_engine::{
    corpus_store::CorpusStore,
    dependencies::{DependencyManifest, ProgramDependency, ProgramSource},
    message::{self, ArchiveProvenance, FrozenV0, HistoricalAccountEvidence},
    replay::hash_bytes,
    universal::{
        checkpoint::{CheckpointAccount, ObservedCheckpointV1},
        evidence::{
            AccountBoundary, AccountObservation, ChunkedAccountObservation, EvidenceKind,
            EvidenceRef, EvidenceStore,
        },
        execution::{
            ExecutionRequest, HistoricalRuntimeEvidence, LiteSvmBackend, RuntimeProfile,
            SlotHashesVariant,
        },
        historical_features::{HistoricalFeatureReceipt, HistoricalFeatureSetEvidence},
        model::*,
        sequence::{self, HistoricalSequenceClosureProofV1, SequenceEntryV1},
    },
    versions::{ProgramLoader, UPGRADEABLE_LOADER_ID},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
const SLOT: u64 = 409942000;
const PROGRAM: &str = "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH";
const PD: &str = "7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP";
const NATIVE: &str = "ComputeBudget111111111111111111111111111111";
const GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const U13: &str = "docs/examples/phase-u13-drift-witness/raw";
const U1: &str = "docs/examples/phase-u13-1-causal-closure";
const U2: &str = "docs/examples/phase-u13-2a-runtime";
const U3: &str = "docs/examples/phase-u13-2c-sequence";
fn value(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn capture(
    store: &EvidenceStore,
    provider: &ArchiveProvenance,
    address: &str,
    boundary: AccountBoundary,
    path: impl AsRef<Path>,
) -> Result<EvidenceRef> {
    AccountObservation::capture(
        store,
        address,
        SLOT,
        boundary,
        provider.clone(),
        &fs::read(path)?,
    )
}
fn account_path(slot: u64, key: &str) -> PathBuf {
    let path = PathBuf::from(format!("{U1}/raw/account-{slot}-{key}.json"));
    if path.exists() {
        path
    } else if key == PROGRAM {
        format!("{U13}/accounts/{slot}-program.json").into()
    } else {
        format!("{U13}/accounts/{slot}-{key}.json").into()
    }
}
fn main() -> Result<()> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("output corpus directory required")?,
    );
    ensure!(
        !output.exists() || fs::read_dir(&output)?.next().is_none(),
        "output must be empty"
    );
    let store = EvidenceStore::at(output.join("evidence"));
    let provider: ArchiveProvenance = serde_json::from_value(
        value("docs/examples/phase-u9-analysis/lut-reconstruction-input.json")?[0]["evidence"][0]
            ["provider"]
            .clone(),
    )?;
    let closure = value(format!("{U1}/closure.json"))?;
    let block_bytes = fs::read(format!("{U13}/block-{SLOT}.json"))?;
    ensure!(
        hash_bytes(&block_bytes) == closure["source"]["sha256"],
        "frozen block hash changed"
    );
    let block: Value = serde_json::from_slice(&block_bytes)?;
    let full_block = store.put(EvidenceKind::Validator, &block_bytes)?;
    let outputs = closure["transactions"][0]["declaredWritableAccounts"]
        .as_array()
        .context("outputs")?
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let plan = sequence::plan(
        &block,
        73,
        &outputs,
        &BTreeMap::from([(PROGRAM.into(), PD.into())]),
    )?;
    ensure!(
        plan.indexes == [73, 428, 431, 438, 1245] && plan.terminal.len() == 9,
        "qualified closure changed"
    );
    let mut seeds = BTreeMap::new();
    let mut refs = BTreeMap::new();
    for key in &plan.parent {
        if key == NATIVE || key == PD {
            continue;
        }
        let reference = capture(
            &store,
            &provider,
            key,
            AccountBoundary::BeforeTransaction,
            account_path(SLOT - 1, key),
        )?;
        let account = AccountObservation::resolve(
            &store,
            &reference,
            key,
            SLOT,
            AccountBoundary::BeforeTransaction,
            GENESIS,
        )?
        .context("parent absent")?;
        seeds.insert(key.clone(), account);
        refs.insert(key.clone(), reference);
    }
    let acquisition = value(format!("{U1}/acquisition.json"))?;
    let rows = acquisition["receipts"]
        .as_array()
        .context("acquisition receipts")?
        .iter()
        .filter(|r| r["params"][1]["dataSlice"].is_object())
        .collect::<Vec<_>>();
    let receipts = json!({"genesis":GENESIS,"provider":provider.scheme_host,"requests":rows.iter().map(|r|json!({"method":"getAccountInfo","status":"success","params":r["params"],"response_sha256":r["sha256"]})).collect::<Vec<_>>()});
    let slices = rows
        .iter()
        .map(|r| {
            Ok((
                r["params"][1]["dataSlice"]["offset"]
                    .as_u64()
                    .context("offset")?,
                fs::read(format!(
                    "{U1}/raw/{}",
                    r["name"].as_str().context("slice file")?
                ))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let pd_ref = ChunkedAccountObservation::capture(
        &store,
        PD,
        SLOT,
        provider.clone(),
        &[serde_json::to_vec(&receipts)?],
        &slices,
    )?;
    let pd = ChunkedAccountObservation::resolve(&store, &pd_ref, PD, SLOT, GENESIS)?;
    let elf = fs::read(format!("{U2}/historical-drift.so"))?;
    ensure!(
        pd.data[45..] == elf,
        "historical ELF differs from ProgramData"
    );
    let deploy = u64::from_le_bytes(pd.data[4..12].try_into()?);
    let authority = (pd.data[12] != 0).then(|| {
        solana_address::Address::new_from_array(pd.data[13..45].try_into().unwrap()).to_string()
    });
    seeds.insert(PD.into(), pd);
    refs.insert(PD.into(), pd_ref.clone());
    let elf_ref = store.put(EvidenceKind::ProgramBinary, &elf)?;
    let binary = ProgramBinaryEvidence {
        program_id: PROGRAM.into(),
        loader: UPGRADEABLE_LOADER_ID.into(),
        programdata_address: Some(PD.into()),
        deployment_slot: Some(deploy),
        upgrade_authority: authority,
        elf: elf_ref.clone(),
        executable_account: refs[PROGRAM].clone(),
        programdata_account: Some(pd_ref),
    };
    let start_checkpoint = ObservedCheckpointV1 {
        schema_version: 1,
        slot: SLOT - 1,
        accounts: refs
            .iter()
            .map(|(address, observation)| CheckpointAccount {
                address: address.clone(),
                observation: observation.clone(),
            })
            .collect(),
    }
    .store(&store)?;
    let mut terminal_entries = Vec::new();
    for key in &plan.terminal {
        terminal_entries.push(CheckpointAccount {
            address: key.clone(),
            observation: capture(
                &store,
                &provider,
                key,
                AccountBoundary::EndOfExecutionSlot,
                account_path(SLOT, key),
            )?,
        });
    }
    let terminal_checkpoint = ObservedCheckpointV1 {
        schema_version: 1,
        slot: SLOT,
        accounts: terminal_entries,
    }
    .store(&store)?;
    let mut sysvars = BTreeMap::new();
    let mut runtime_seeds = Vec::new();
    for (name, key) in [
        ("Clock", "SysvarC1ock11111111111111111111111111111111"),
        ("Rent", "SysvarRent111111111111111111111111111111111"),
        (
            "EpochSchedule",
            "SysvarEpochSchedu1e111111111111111111111111",
        ),
    ] {
        let reference = capture(
            &store,
            &provider,
            key,
            AccountBoundary::EndOfExecutionSlot,
            format!("{U2}/raw/sysvar-{name}.json"),
        )?;
        sysvars.insert(
            key.to_string(),
            AccountObservation::resolve(
                &store,
                &reference,
                key,
                SLOT,
                AccountBoundary::EndOfExecutionSlot,
                GENESIS,
            )?
            .context("sysvar absent")?,
        );
        runtime_seeds.push(AccountSeed {
            address: key.into(),
            boundary: AccountBoundary::EndOfExecutionSlot,
            observation: reference,
        });
    }
    let audit = value("docs/examples/phase-u13-2b-feature-universe/audit.json")?;
    let feature_receipts = audit["observations"]
        .as_array()
        .context("features")?
        .iter()
        .map(|r| {
            Ok(HistoricalFeatureReceipt {
                id: r["id"].as_str().context("feature ID")?.into(),
                source_sha256: r["responseSha256"].as_str().context("feature hash")?.into(),
                source_response: fs::read_to_string(
                    r["source"].as_str().context("feature source")?,
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let historical = HistoricalRuntimeEvidence::new(
        "EK66pFxagY8mA6VjRmNnBqhZwcQ44jFztfNVfueaDWgM".into(),
        RuntimeProfile::sysvar_snapshot_hash(&sysvars)?,
        "LiteSVM 0.16.0 historical evidence".into(),
        "agave-4.2.2-native-system-compute".into(),
        "U13.2B exact-slot account archive receipts".into(),
    )?
    .with_historical_feature_set(HistoricalFeatureSetEvidence::new(SLOT, feature_receipts)?)?;
    let runtime_ref = store.put(EvidenceKind::Runtime, &serde_json::to_vec(&historical)?)?;
    let runtime = RuntimeContext {
        sysvars: runtime_seeds,
        feature_profile: historical.feature_profile.clone(),
        slot_hashes_policy: "materiality_checked_default".into(),
        signature_check: false,
        blockhash_check: false,
        instructions_rule: "runtime_generated_from_complete_message".into(),
        provenance: "retained historical runtime account receipts".into(),
        historical_evidence: Some(historical.clone()),
        historical_evidence_ref: Some(runtime_ref.clone()),
    };
    let profile = RuntimeProfile::resolve(
        Some(&historical),
        &sysvars,
        &runtime.feature_profile,
        false,
        false,
        &runtime.instructions_rule,
        &runtime.slot_hashes_policy,
    )?;
    let lut_receipts = value(format!("{U3}/lut-acquisition.json"))?;
    let mut lut_refs = BTreeMap::new();
    let mut tables = BTreeMap::new();
    for row in lut_receipts["receipts"]
        .as_array()
        .context("LUT receipts")?
    {
        let key = row["address"].as_str().context("LUT key")?;
        let raw = fs::read(format!(
            "{U3}/raw/{}",
            row["name"].as_str().context("LUT file")?
        ))?;
        lut_refs.insert(
            key.to_owned(),
            AccountObservation::capture(
                &store,
                key,
                SLOT,
                AccountBoundary::EndOfExecutionSlot,
                provider.clone(),
                &raw,
            )?,
        );
        tables.insert(
            key.to_owned(),
            HistoricalAccountEvidence::from_response(key, SLOT, provider.clone(), &raw)?,
        );
    }
    let mut entries = Vec::new();
    let mut messages = Vec::new();
    for index in &plan.indexes {
        let mut raw = block["result"]["transactions"][*index].clone();
        raw["slot"] = json!(SLOT);
        raw["transactionIndex"] = json!(index);
        raw["blockTime"] = block["result"]["blockTime"].clone();
        let frozen = FrozenV0::from_rpc(&raw, GENESIS)?;
        let tables_used = frozen
            .native_message()
            .address_table_lookups
            .iter()
            .map(|l| tables[&l.account_key.to_string()].clone())
            .collect::<Vec<_>>();
        let proven = message::reconstruct(&frozen, &tables_used, None)
            .map_err(|e| anyhow::anyhow!("LUT: {e:?}"))?;
        let execution = ExecutionInput::V0 {
            message: frozen.native_message().clone(),
            frozen_transaction: store.put(EvidenceKind::Transaction, &serde_json::to_vec(&raw)?)?,
            lookup_tables: frozen
                .native_message()
                .address_table_lookups
                .iter()
                .map(|l| lut_refs[&l.account_key.to_string()].clone())
                .collect(),
            slot_hashes: None,
            claimed_proof: proven.proof().clone(),
        };
        let message = execution.resolve(&store, GENESIS)?;
        ensure!(
            raw["meta"]["err"].is_null()
                && raw["meta"]["innerInstructions"]
                    .as_array()
                    .context("inner")?
                    .is_empty()
                && raw["meta"]["returnData"].is_null(),
            "conversion envelope changed"
        );
        let expected = ExpectedHistoricalOutcome {
            success: true,
            error: None,
            fee: raw["meta"]["fee"].as_u64().context("fee")?,
            compute_units: raw["meta"]["computeUnitsConsumed"].as_u64(),
            logs: serde_json::from_value(raw["meta"]["logMessages"].clone())?,
            inner_instructions: vec![],
            return_data: None,
            watched_accounts: vec![],
        };
        entries.push(SequenceEntryV1 {
            transaction_index: *index,
            signature: message.transaction.signature.clone(),
            execution,
            full_account_keys: message.account_keys.clone(),
            expected,
            runtime_profile_id: profile.profile_id.clone(),
            frontier_digest: String::new(),
        });
        messages.push(message);
    }
    let frontier = seeds.keys().cloned().collect::<Vec<_>>();
    let runs = LiteSvmBackend.execute_sequence(
        &ExecutionRequest {
            message: &messages[0],
            seeds: &seeds,
            absent_pre_accounts: &[],
            watched: &frontier,
            runtime_sysvars: &sysvars,
            clock: None,
            programs_to_load: &[],
            runtime_profile: &profile,
            unlimited_logs: true,
            slot_hashes: SlotHashesVariant::BackendDefault,
            recent_blockhashes: Default::default(),
            require_complete_state: false,
        },
        &messages,
    )?;
    for (entry, run) in entries.iter_mut().zip(&runs) {
        entry.frontier_digest = sequence::frontier_digest(&run.post_accounts)?;
    }
    let mut target_execution = runs[0].clone();
    target_execution
        .post_accounts
        .retain(|k, _| outputs.contains(k));
    let target_execution_ref = store.put(
        EvidenceKind::Execution,
        &serde_json::to_vec(&target_execution)?,
    )?;
    let proof = HistoricalSequenceClosureProofV1 {
        schema_version: 1,
        slot: SLOT,
        target_transaction_index: 73,
        target_signature: entries[0].signature.clone(),
        full_block,
        start_checkpoint: start_checkpoint.clone(),
        terminal_checkpoint: terminal_checkpoint.clone(),
        runtime_evidence: runtime_ref,
        program_binaries: vec![binary.clone()],
        entries: entries.clone(),
        dependency_edges: plan.edges,
        terminal_frontier: plan.terminal,
        target_execution: target_execution_ref.clone(),
    };
    let closure_proof = proof.store(&store)?;
    let mut expected = entries[0].expected.clone();
    expected.watched_accounts = outputs
        .iter()
        .map(|key| {
            Ok(WatchedAccount {
                address: key.clone(),
                expected_post_content: target_execution.post_accounts[key]
                    .as_ref()
                    .map(|a| store.put(EvidenceKind::AccountContent, &serde_json::to_vec(a)?))
                    .transpose()?,
                source: ExpectedAccountSource::DerivedTargetBoundary,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let discovered = eplyx_engine::dependencies::discover(&messages[0].transaction, None, PROGRAM);
    let dependencies = DependencyManifest {
        programs: discovered
            .into_iter()
            .map(|(program_id, discovered_by)| {
                let native = program_id == NATIVE;
                ProgramDependency {
                    program_id,
                    source: if native {
                        ProgramSource::Builtin
                    } else {
                        ProgramSource::HistoricalMainnet
                    },
                    loader: (!native).then_some(ProgramLoader::Upgradeable),
                    deployed_slot: (!native).then_some(deploy),
                    binary_sha256: (!native).then(|| elf_ref.sha256.clone()),
                    binary_len: (!native).then_some(elf.len() as u64),
                    observed_slot: Some(SLOT - 1),
                    discovered_by,
                    note: None,
                }
            })
            .collect(),
    };
    let mut record = ReplayObservationV2 {
        schema_version: 2,
        id: String::new(),
        protocol: "unknown".into(),
        program_id: PROGRAM.into(),
        genesis_hash: GENESIS.into(),
        signature: entries[0].signature.clone(),
        slot: SLOT,
        execution: entries[0].execution.clone(),
        target: SemanticTarget {
            program_id: PROGRAM.into(),
            outer_index: 2,
            instruction_identity: hash_bytes(&serde_json::to_vec(
                &messages[0].transaction.instructions[2],
            )?),
        },
        instruction_roles: (0..3)
            .map(|i| InstructionAssignment {
                outer_index: i,
                role: if i == 2 {
                    InstructionRole::SemanticTarget
                } else {
                    InstructionRole::StandardCompanion
                },
            })
            .collect(),
        account_seeds: refs
            .into_iter()
            .map(|(address, observation)| AccountSeed {
                address,
                boundary: AccountBoundary::BeforeTransaction,
                observation,
            })
            .collect(),
        absent_pre_accounts: vec![],
        binaries: vec![binary],
        dependencies,
        runtime,
        expected,
        fidelity_profile: FidelityProfile::CheckpointedExecutionV1,
        checkpointed_execution: Some(CheckpointedExecutionProof {
            proof_contract_version: 3,
            start_checkpoint,
            terminal_checkpoint,
            closure_proof,
            deterministic_execution: target_execution_ref,
            validation_outputs: outputs,
        }),
    };
    record.id = record.identity()?;
    let corpus = CorpusStore::open(output)?;
    corpus.insert_v2(&record)?;
    corpus.publish_v2()?;
    println!("{}", record.id);
    Ok(())
}
