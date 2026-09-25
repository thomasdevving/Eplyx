//! Convert the retained checkpointed replay evidence into one reconstructive schema-2
//! corpus observation. The conversion executes and reconciles the transaction;
//! it never consumes the U10 success JSON.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use eplyx_engine::{
    corpus_store::CorpusStore,
    dependencies::{self, DependencyManifest, ProgramDependency, ProgramSource},
    message::{self, ArchiveProvenance, FrozenV0, HistoricalVisibility},
    replay::hash_bytes,
    types::AccountSnapshot,
    universal::{
        checkpoint::{
            AccountDerivationV2, CheckpointAccount, DerivedAccountEvidenceV2, FailedOverlap,
            FailedTransactionRollbackRule, ObservedCheckpointV1, RollbackProvenanceV1,
            TransactionClosureProofV1,
        },
        evidence::{
            AccountBoundary, AccountObservation, ChunkedAccountObservation, EvidenceKind,
            EvidenceRef, EvidenceStore,
        },
        execution::{
            ExecutionBackend, ExecutionRequest, HistoricalRuntimeEvidence, InnerGroup,
            InnerInstruction, LiteSvmBackend, ReturnData, RuntimeProfile, SlotHashesVariant,
        },
        model::{
            AccountSeed, CheckpointedExecutionProof, ExecutionInput, ExpectedAccountSource,
            ExpectedHistoricalOutcome, FidelityProfile, InstructionAssignment, InstructionRole,
            ProgramBinaryEvidence, ReplayObservationV2, ResolvedMessage, RuntimeContext,
            SemanticTarget, WatchedAccount,
        },
        pipeline,
    },
    versions::{ProgramLoader, LEGACY_BPF_LOADER_ID, UPGRADEABLE_LOADER_ID},
};
use serde_json::{json, Value};

const SLOT: u64 = 448_760_958;
const TARGET_INDEX: usize = 259;
const TARGET_OUTER_INDEX: usize = 4;
const TARGET_PROGRAM: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
const ENVIRONMENT_BLOCKHASH: &str = "A2uVDFYNS3HNaYMus1fM6FztNkanJsV74K66FF4mDjZi";
const RECENT_BLOCKHASHES: &str = "SysvarRecentB1ockHashes11111111111111111111";
const SLOT_HASHES: &str = "SysvarS1otHashes111111111111111111111111111";
const SYSVAR_OWNER: &str = "Sysvar1111111111111111111111111111111111111";

fn read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    Ok(fs::read(path)?)
}

fn value(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&read(path)?)?)
}

fn provider(genesis: &str) -> ArchiveProvenance {
    ArchiveProvenance {
        scheme_host: "https://solana-mainnet.g.alchemy.com".into(),
        genesis_hash: genesis.into(),
        visibility: HistoricalVisibility::FinalizedEndOfExecutionSlot,
        validation_artifact_sha256:
            "c21a90f2a47425dfa4f925c344bcbbd4d667367547bf3c559f74c35a121e95e4".into(),
    }
}

fn content_ref(store: &EvidenceStore, account: &AccountSnapshot) -> Result<EvidenceRef> {
    store.put(EvidenceKind::AccountContent, &serde_json::to_vec(account)?)
}

fn capture_account(
    store: &EvidenceStore,
    genesis: &str,
    address: &str,
    boundary: AccountBoundary,
    path: impl AsRef<Path>,
) -> Result<(EvidenceRef, Option<AccountSnapshot>)> {
    let raw = read(path)?;
    let reference =
        AccountObservation::capture(store, address, SLOT, boundary, provider(genesis), &raw)?;
    let account = AccountObservation::resolve(store, &reference, address, SLOT, boundary, genesis)?;
    Ok((reference, account))
}

fn derived_runtime_account(
    store: &EvidenceStore,
    address: &str,
    lamports: u64,
    data_path: &str,
    source_path: &str,
) -> Result<(EvidenceRef, AccountSnapshot)> {
    let account = AccountSnapshot {
        owner: SYSVAR_OWNER.into(),
        lamports,
        data: read(data_path)?,
        executable: false,
        rent_epoch: u64::MAX,
    };
    let content = content_ref(store, &account)?;
    let source = store.put(EvidenceKind::Validator, &read(source_path)?)?;
    let derivation = if address == RECENT_BLOCKHASHES {
        AccountDerivationV2::RecentBlockhashesPreTargetV1 {
            event: source,
            tail_block: store.put(
                EvidenceKind::Validator,
                &read(
                    "docs/examples/phase-u9-acquisition/raw/recent-blockhash-tail-448760809.json",
                )?,
            )?,
            acquisition_receipt: store.put(
                EvidenceKind::Validator,
                &read("docs/examples/phase-u9-acquisition/acquisition.json")?,
            )?,
            tail_slot: 448_760_809,
        }
    } else {
        AccountDerivationV2::YellowstoneAccountImageV1 { event: source }
    };
    let evidence = DerivedAccountEvidenceV2 {
        schema_version: 2,
        address: address.into(),
        transaction_slot: SLOT,
        boundary: AccountBoundary::BeforeTargetExecution,
        content,
        derivation,
    };
    let reference = evidence.store(store)?;
    ensure!(
        DerivedAccountEvidenceV2::resolve(
            store,
            &reference,
            address,
            SLOT,
            AccountBoundary::BeforeTargetExecution
        )? == account,
        "conversion account reconstruction differs"
    );
    Ok((reference, account))
}

fn validator_inner(meta: &Value) -> Result<Vec<InnerGroup>> {
    meta["innerInstructions"]
        .as_array()
        .context("validator inner instructions")?
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
                                .collect::<Result<_>>()?,
                            data: bs58::decode(ix["data"].as_str().context("CPI data")?)
                                .into_vec()?,
                            stack_height: u8::try_from(
                                ix["stackHeight"].as_u64().context("CPI stack height")?,
                            )?,
                        })
                    })
                    .collect::<Result<_>>()?,
            })
        })
        .collect()
}

fn checkpoint(
    store: &EvidenceStore,
    slot: u64,
    accounts: Vec<CheckpointAccount>,
) -> Result<EvidenceRef> {
    ObservedCheckpointV1 {
        schema_version: 1,
        slot,
        accounts,
    }
    .store(store)
}

fn standardized_chunk_receipt(acquisition: &Value, genesis: &str) -> Result<Vec<u8>> {
    let requests = acquisition["receipts"]
        .as_array()
        .context("acquisition receipts")?
        .iter()
        .filter(|row| row["method"] == "getAccountInfo")
        .map(|row| {
            json!({
                "method": "getAccountInfo",
                "status": "success",
                "params": row["params"],
                "response_sha256": row["response_sha256"],
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_vec(&json!({
        "genesis": genesis,
        "provider": "https://solana-mainnet.g.alchemy.com",
        "requests": requests,
    }))?)
}

fn closure_proof(
    store: &EvidenceStore,
    signature: &str,
    inputs: &[String],
    outputs: &[String],
    runtime_evidence: &EvidenceRef,
) -> Result<EvidenceRef> {
    let block_bytes = read("docs/examples/phase-u5-sample/first-candidate-screen-block.body")?;
    let conflict_bytes = read("docs/examples/phase-u8-analysis/conflict-table.json")?;
    let block: Value = serde_json::from_slice(&block_bytes)?;
    let conflict: Value = serde_json::from_slice(&conflict_bytes)?;
    ensure!(
        conflict["target_signature"].as_str() == Some(signature)
            && conflict["target_transaction_index"].as_u64() == Some(TARGET_INDEX as u64),
        "retained conflict census target differs"
    );
    let failed_overlaps = conflict["overlap_transactions"]
        .as_array()
        .context("conflict overlaps")?
        .iter()
        .filter(|row| row["success"].as_bool() == Some(false))
        .map(|row| {
            Ok(FailedOverlap {
                transaction_index: row["transaction_index"].as_u64().context("overlap index")?
                    as usize,
                signature: row["signature"]
                    .as_str()
                    .context("overlap signature")?
                    .into(),
                fee_payer: row["fee_payer"]
                    .as_str()
                    .context("overlap fee payer")?
                    .into(),
                durable_nonce_accounts: row["durable_nonce_accounts"]
                    .as_array()
                    .context("overlap nonce accounts")?
                    .iter()
                    .map(|v| v.as_str().context("nonce address").map(str::to_string))
                    .collect::<Result<_>>()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let block_accounts_evidence = store.put(EvidenceKind::Validator, &block_bytes)?;
    let conflict_census_evidence = store.put(EvidenceKind::Validator, &conflict_bytes)?;
    let transactions = block["result"]["transactions"]
        .as_array()
        .context("block transactions")?;
    TransactionClosureProofV1 {
        schema_version: 2,
        slot: SLOT,
        parent_slot: block["result"]["parentSlot"]
            .as_u64()
            .context("parent slot")?,
        blockhash: block["result"]["blockhash"]
            .as_str()
            .context("blockhash")?
            .into(),
        transaction_count: transactions.len(),
        target_transaction_index: TARGET_INDEX,
        target_signature: signature.into(),
        execution_inputs: inputs.to_vec(),
        validation_outputs: outputs.to_vec(),
        failed_overlaps,
        rollback_rule: FailedTransactionRollbackRule::FeePayerAndDurableNonceOnly,
        block_accounts_evidence,
        conflict_census_evidence,
        full_block_evidence: Some(store.put(
            EvidenceKind::Validator,
            &read("docs/examples/phase-u5-sample/blocks/448760958.body")?,
        )?),
        rollback_provenance: Some(RollbackProvenanceV1 {
            semantics_version: 1,
            upstream_commit: "965aee8e55d45ac3ca72e48f15945bfca4a32804".into(),
            runtime_evidence: runtime_evidence.clone(),
            source_provenance: store.put(
                EvidenceKind::Validator,
                &read("docs/examples/phase-u8-freeze/source/provenance.json")?,
            )?,
            rollback_source: store.put(
                EvidenceKind::Validator,
                &read("docs/examples/phase-u8-freeze/source/rollback-accounts.rs")?,
            )?,
            transaction_processor_source: store.put(
                EvidenceKind::Validator,
                &read("docs/examples/phase-u8-freeze/source/transaction-processor.rs")?,
            )?,
        }),
    }
    .store(store)
}

fn main() -> Result<()> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("output corpus directory required")?,
    );
    ensure!(
        !output.exists() || fs::read_dir(&output)?.next().is_none(),
        "output corpus directory must be empty"
    );
    let store = EvidenceStore::at(output.join("evidence"));

    let input = value("docs/examples/phase-u9-analysis/lut-reconstruction-input.json")?;
    let row = input
        .as_array()
        .and_then(|rows| rows.first())
        .context("retained reconstruction input")?;
    let genesis = row["genesis"].as_str().context("genesis hash")?;
    let frozen = FrozenV0::from_rpc(&row["result"], genesis)?;
    let frozen_transaction = store.put(
        EvidenceKind::Transaction,
        &serde_json::to_vec(&row["result"])?,
    )?;

    let mut lookup_tables = Vec::new();
    for raw in row["evidence"].as_array().context("LUT evidence")? {
        let address = raw["pubkey"].as_str().context("LUT address")?;
        let provider: ArchiveProvenance = serde_json::from_value(raw["provider"].clone())?;
        let bytes = BASE64_STANDARD.decode(
            raw["raw_response_base64"]
                .as_str()
                .context("LUT response")?,
        )?;
        let reference = AccountObservation::capture(
            &store,
            address,
            SLOT,
            AccountBoundary::EndOfExecutionSlot,
            provider,
            &bytes,
        )?;
        lookup_tables.push(reference);
    }
    let lut_accounts = lookup_tables
        .iter()
        .zip(row["evidence"].as_array().context("LUT evidence")?)
        .map(|(reference, raw)| {
            let address = raw["pubkey"].as_str().context("LUT address")?;
            let observation: eplyx_engine::universal::evidence::AccountObservation =
                serde_json::from_slice(&store.get(reference)?)?;
            let (_, wire) = AccountObservation::resolve_with_raw(
                &store,
                reference,
                address,
                SLOT,
                AccountBoundary::EndOfExecutionSlot,
                genesis,
            )?;
            message::HistoricalAccountEvidence::from_response(
                address,
                SLOT,
                observation.provider,
                &wire,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let proven = message::reconstruct(&frozen, &lut_accounts, None)
        .map_err(|error| anyhow::anyhow!("LUT proof stage {}: {}", error.stage, error.detail))?;
    let resolved_message = ResolvedMessage {
        message: proven.versioned_message(),
        transaction: proven.transaction().clone(),
        account_keys: proven
            .proof()
            .full_account_keys
            .iter()
            .map(|key| key.address.clone())
            .collect(),
    };
    ensure!(
        resolved_message.transaction.slot == SLOT,
        "target slot differs"
    );

    let mut seed_accounts = BTreeMap::new();
    let mut seed_refs = BTreeMap::new();
    let mut account_seeds = Vec::new();
    let mut absent_pre_accounts = Vec::new();
    let mut start_entries = Vec::new();
    for address in &resolved_message.account_keys {
        let path = format!(
            "docs/examples/phase-u7-archive/{}-{}.body",
            SLOT - 1,
            address
        );
        let (reference, account) = capture_account(
            &store,
            genesis,
            address,
            AccountBoundary::BeforeTransaction,
            path,
        )?;
        start_entries.push(CheckpointAccount {
            address: address.clone(),
            observation: reference.clone(),
        });
        match account {
            Some(account) => {
                seed_accounts.insert(address.clone(), account);
                seed_refs.insert(address.clone(), reference.clone());
                account_seeds.push(AccountSeed {
                    address: address.clone(),
                    boundary: AccountBoundary::BeforeTransaction,
                    observation: reference,
                });
            }
            None if address != RECENT_BLOCKHASHES => absent_pre_accounts.push(AccountSeed {
                address: address.clone(),
                boundary: AccountBoundary::BeforeTransaction,
                observation: reference,
            }),
            None => {}
        }
    }
    let start_checkpoint = checkpoint(&store, SLOT - 1, start_entries)?;

    for (reference, evidence) in lookup_tables.iter().zip(&lut_accounts) {
        let account = evidence.account(SLOT, genesis)?;
        seed_accounts.insert(evidence.pubkey.clone(), account);
        seed_refs.insert(evidence.pubkey.clone(), reference.clone());
        account_seeds.push(AccountSeed {
            address: evidence.pubkey.clone(),
            boundary: AccountBoundary::EndOfExecutionSlot,
            observation: reference.clone(),
        });
    }

    let acquisition = value("docs/examples/phase-u9-acquisition/acquisition.json")?;
    let chunk_receipt = standardized_chunk_receipt(&acquisition, genesis)?;
    for programdata in acquisition["programdata"]
        .as_array()
        .context("ProgramData rows")?
    {
        let address = programdata["programdata_address"]
            .as_str()
            .context("ProgramData address")?;
        let slices = programdata["chunks"]
            .as_array()
            .context("ProgramData chunks")?
            .iter()
            .map(|chunk| {
                Ok((
                    chunk["offset"].as_u64().context("chunk offset")?,
                    read(
                        chunk["response_file"]
                            .as_str()
                            .context("chunk response file")?,
                    )?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let reference = ChunkedAccountObservation::capture(
            &store,
            address,
            SLOT,
            provider(genesis),
            std::slice::from_ref(&chunk_receipt),
            &slices,
        )?;
        let account =
            ChunkedAccountObservation::resolve(&store, &reference, address, SLOT, genesis)?;
        ensure!(
            hash_bytes(&account.data)
                == programdata["account_sha256"]
                    .as_str()
                    .context("ProgramData hash")?,
            "assembled ProgramData differs from retained acquisition"
        );
        seed_accounts.insert(address.into(), account);
        seed_refs.insert(address.into(), reference.clone());
        account_seeds.push(AccountSeed {
            address: address.into(),
            boundary: AccountBoundary::BeforeTransaction,
            observation: reference,
        });
    }

    let mut runtime_sysvars = BTreeMap::new();
    let mut runtime_seeds = Vec::new();
    for sysvar in acquisition["sysvars"].as_array().context("sysvar rows")? {
        let Some(file) = sysvar["account_file"].as_str() else {
            continue;
        };
        let address = sysvar["address"].as_str().context("sysvar address")?;
        let raw_file = format!(
            "docs/examples/phase-u9-acquisition/raw/sysvar-{}-{}.json",
            SLOT,
            sysvar["name"].as_str().context("sysvar name")?
        );
        ensure!(
            Path::new(file).exists(),
            "retained sysvar account file missing"
        );
        let (reference, account) = capture_account(
            &store,
            genesis,
            address,
            AccountBoundary::EndOfExecutionSlot,
            raw_file,
        )?;
        let account = account.context("runtime sysvar absent")?;
        runtime_sysvars.insert(address.into(), account);
        runtime_seeds.push(AccountSeed {
            address: address.into(),
            boundary: AccountBoundary::EndOfExecutionSlot,
            observation: reference,
        });
    }
    let (recent_ref, recent) = derived_runtime_account(
        &store,
        RECENT_BLOCKHASHES,
        42_706_560,
        "docs/examples/phase-u9-analysis/runtime/RecentBlockhashes.pre-transaction.bin",
        "docs/examples/phase-u9-analysis/runtime/RecentBlockhashes.event.pb",
    )?;
    runtime_sysvars.insert(RECENT_BLOCKHASHES.into(), recent);
    runtime_seeds.push(AccountSeed {
        address: RECENT_BLOCKHASHES.into(),
        boundary: AccountBoundary::BeforeTargetExecution,
        observation: recent_ref,
    });
    let (slot_hashes_ref, slot_hashes) = derived_runtime_account(
        &store,
        SLOT_HASHES,
        143_487_360,
        "docs/examples/phase-u9-analysis/runtime/SlotHashes.account.bin",
        "docs/examples/phase-u9-analysis/runtime/SlotHashes.event.pb",
    )?;
    runtime_sysvars.insert(SLOT_HASHES.into(), slot_hashes);
    runtime_seeds.push(AccountSeed {
        address: SLOT_HASHES.into(),
        boundary: AccountBoundary::BeforeTargetExecution,
        observation: slot_hashes_ref,
    });

    let historical_runtime = HistoricalRuntimeEvidence::new(
        ENVIRONMENT_BLOCKHASH.into(),
        RuntimeProfile::sysvar_snapshot_hash(&runtime_sysvars)?,
        "LiteSVM 0.16.0 mainnet".into(),
        "agave-4.2.2-native-system-compute".into(),
        "phase-u9-runtime-profile:eb8ab40820255573c0b2d4534783eda74b1525056afd34696052be9cde734852"
            .into(),
    )?;
    let historical_runtime_ref = store.put(
        EvidenceKind::Runtime,
        &serde_json::to_vec(&historical_runtime)?,
    )?;
    let runtime_profile = RuntimeProfile::resolve(
        Some(&historical_runtime),
        &runtime_sysvars,
        "LiteSVM 0.16.0 mainnet",
        false,
        false,
        "runtime_generated_from_complete_message",
        "historical",
    )?;

    let closure = value("docs/examples/phase-u8-analysis/closure.json")?;
    let validation_outputs = closure["validation_outputs"]
        .as_array()
        .context("validation outputs")?
        .iter()
        .map(|v| {
            v.as_str()
                .context("validation output address")
                .map(str::to_string)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        validation_outputs.len() == 12,
        "validation output census differs"
    );
    let mut terminal_entries = Vec::new();
    let mut watched_accounts = Vec::new();
    let mut expected_accounts = BTreeMap::new();
    for address in &validation_outputs {
        let path = format!("docs/examples/phase-u7-archive/{}-{}.body", SLOT, address);
        let (reference, account) = capture_account(
            &store,
            genesis,
            address,
            AccountBoundary::EndOfExecutionSlot,
            path,
        )?;
        terminal_entries.push(CheckpointAccount {
            address: address.clone(),
            observation: reference.clone(),
        });
        let expected_post_content = account
            .as_ref()
            .map(|account| content_ref(&store, account))
            .transpose()?;
        watched_accounts.push(WatchedAccount {
            address: address.clone(),
            expected_post_content,
            source: ExpectedAccountSource::Archived(reference),
        });
        expected_accounts.insert(address.clone(), account);
    }
    let terminal_checkpoint = checkpoint(&store, SLOT, terminal_entries)?;

    let meta = &row["result"]["meta"];
    let logs = meta["logMessages"]
        .as_array()
        .context("validator logs")?
        .iter()
        .map(|v| v.as_str().context("validator log").map(str::to_string))
        .collect::<Result<Vec<_>>>()?;
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
                    .context("return data")?,
            )?,
        })
    };
    let expected = ExpectedHistoricalOutcome {
        success: meta["err"].is_null(),
        error: (!meta["err"].is_null()).then(|| meta["err"].to_string()),
        fee: meta["fee"].as_u64().context("validator fee")?,
        compute_units: Some(
            meta["computeUnitsConsumed"]
                .as_u64()
                .context("validator compute units")?,
        ),
        logs,
        inner_instructions: validator_inner(meta)?,
        return_data,
        watched_accounts,
    };

    let mut binaries = Vec::new();
    let mut dependencies_manifest = Vec::new();
    let programdata_by_program = acquisition["programdata"]
        .as_array()
        .context("ProgramData rows")?
        .iter()
        .map(|row| {
            Ok((
                row["program_id"]
                    .as_str()
                    .context("program ID")?
                    .to_string(),
                row,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    for (program_id, discovered_by) in
        dependencies::discover(&resolved_message.transaction, None, TARGET_PROGRAM)
    {
        let executable_ref = seed_refs.get(&program_id).cloned();
        let mut dependency = ProgramDependency {
            program_id: program_id.clone(),
            source: ProgramSource::Builtin,
            loader: None,
            deployed_slot: None,
            binary_sha256: None,
            binary_len: None,
            observed_slot: Some(SLOT - 1),
            discovered_by,
            note: None,
        };
        if let Some(programdata) = programdata_by_program.get(&program_id) {
            let elf = read(programdata["elf_file"].as_str().context("ELF file")?)?;
            let elf_ref = store.put(EvidenceKind::ProgramBinary, &elf)?;
            let programdata_address = programdata["programdata_address"]
                .as_str()
                .context("ProgramData address")?;
            dependency.source = ProgramSource::HistoricalMainnet;
            dependency.loader = Some(ProgramLoader::Upgradeable);
            dependency.deployed_slot = programdata["deployment_slot"].as_u64();
            dependency.binary_sha256 = Some(hash_bytes(&elf));
            dependency.binary_len = Some(elf.len() as u64);
            binaries.push(ProgramBinaryEvidence {
                program_id: program_id.clone(),
                loader: UPGRADEABLE_LOADER_ID.into(),
                programdata_address: Some(programdata_address.into()),
                deployment_slot: dependency.deployed_slot,
                upgrade_authority: programdata["upgrade_authority"]
                    .as_str()
                    .map(str::to_string),
                elf: elf_ref,
                executable_account: executable_ref.context("upgradeable program seed missing")?,
                programdata_account: Some(
                    seed_refs
                        .get(programdata_address)
                        .context("ProgramData seed reference missing")?
                        .clone(),
                ),
            });
        } else if seed_accounts
            .get(&program_id)
            .is_some_and(|account| account.owner == LEGACY_BPF_LOADER_ID)
        {
            let elf = seed_accounts[&program_id].data.clone();
            let elf_ref = store.put(EvidenceKind::ProgramBinary, &elf)?;
            dependency.source = ProgramSource::HistoricalMainnet;
            dependency.loader = Some(ProgramLoader::Legacy);
            dependency.binary_sha256 = Some(hash_bytes(&elf));
            dependency.binary_len = Some(elf.len() as u64);
            binaries.push(ProgramBinaryEvidence {
                program_id: program_id.clone(),
                loader: LEGACY_BPF_LOADER_ID.into(),
                programdata_address: None,
                deployment_slot: None,
                upgrade_authority: None,
                elf: elf_ref,
                executable_account: executable_ref.context("legacy program seed missing")?,
                programdata_account: None,
            });
        }
        dependencies_manifest.push(dependency);
    }

    let absent = absent_pre_accounts
        .iter()
        .map(|seed| seed.address.clone())
        .collect::<Vec<_>>();
    let execution_evidence = LiteSvmBackend.execute(&ExecutionRequest {
        message: &resolved_message,
        seeds: &seed_accounts,
        absent_pre_accounts: &absent,
        watched: &validation_outputs,
        runtime_sysvars: &runtime_sysvars,
        clock: None,
        programs_to_load: &[],
        runtime_profile: &runtime_profile,
        unlimited_logs: true,
        slot_hashes: SlotHashesVariant::BackendDefault,
        recent_blockhashes: Default::default(),
        require_complete_state: true,
    })?;
    ensure!(
        execution_evidence.post_accounts == expected_accounts,
        "derived execution does not reconcile with the terminal checkpoint"
    );
    let deterministic_execution = store.put(
        EvidenceKind::Execution,
        &serde_json::to_vec(&execution_evidence)?,
    )?;
    let closure_proof = closure_proof(
        &store,
        &resolved_message.transaction.signature,
        &resolved_message.account_keys,
        &validation_outputs,
        &historical_runtime_ref,
    )?;

    let execution = ExecutionInput::V0 {
        message: match resolved_message.message.clone() {
            solana_message::VersionedMessage::V0(message) => message,
            _ => anyhow::bail!("retained target is not native v0"),
        },
        frozen_transaction,
        lookup_tables,
        slot_hashes: None,
        claimed_proof: proven.proof().clone(),
    };
    let roles = resolved_message
        .transaction
        .instructions
        .iter()
        .enumerate()
        .map(|(outer_index, _)| InstructionAssignment {
            outer_index,
            role: if outer_index == TARGET_OUTER_INDEX {
                InstructionRole::SemanticTarget
            } else {
                InstructionRole::StandardCompanion
            },
        })
        .collect::<Vec<_>>();
    let instruction_identity = hash_bytes(&serde_json::to_vec(&(
        "eplyx-target-instruction-v1",
        &resolved_message.transaction.instructions[TARGET_OUTER_INDEX],
    ))?);
    let mut record = ReplayObservationV2 {
        schema_version: 2,
        id: String::new(),
        protocol: "orca-whirlpool".into(),
        program_id: TARGET_PROGRAM.into(),
        genesis_hash: genesis.into(),
        signature: resolved_message.transaction.signature.clone(),
        slot: SLOT,
        execution,
        target: SemanticTarget {
            program_id: TARGET_PROGRAM.into(),
            outer_index: TARGET_OUTER_INDEX,
            instruction_identity,
        },
        instruction_roles: roles,
        account_seeds,
        absent_pre_accounts,
        binaries,
        dependencies: DependencyManifest {
            programs: dependencies_manifest,
        },
        runtime: RuntimeContext {
            sysvars: runtime_seeds,
            feature_profile: "LiteSVM 0.16.0 mainnet".into(),
            slot_hashes_policy: "historical".into(),
            signature_check: false,
            blockhash_check: false,
            instructions_rule: "runtime_generated_from_complete_message".into(),
            provenance:
                "retained historical runtime evidence and deterministic pre-target reconstruction"
                    .into(),
            historical_evidence: Some(historical_runtime),
            historical_evidence_ref: Some(historical_runtime_ref),
        },
        expected,
        fidelity_profile: FidelityProfile::CheckpointedExecutionV1,
        checkpointed_execution: Some(CheckpointedExecutionProof {
            proof_contract_version: 2,
            start_checkpoint,
            terminal_checkpoint,
            closure_proof,
            deterministic_execution,
            validation_outputs,
        }),
    };
    record.id = record.identity()?;
    let corpus = CorpusStore::open(&output)?;
    corpus.insert_v2(&record)?;
    let manifest = corpus.publish_v2()?;
    let resolved = record.resolve(&store)?;
    let (_, fidelity) = pipeline::baseline(&record, &resolved)?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "observation_id": record.id,
            "corpus_id": manifest.canonical_hash,
            "fidelity_profile": record.fidelity_profile,
            "fidelity_matched": fidelity.matched(),
            "validation_outputs": record.checkpointed_execution.as_ref().map(|p| p.validation_outputs.len()),
            "runtime_profile_id": resolved.runtime_profile.profile_id,
        }))?
    );
    Ok(())
}
