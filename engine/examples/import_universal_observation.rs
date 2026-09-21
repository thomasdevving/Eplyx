//! Generic offline conversion of a frozen execution payload and raw source
//! envelopes into a schema-2 observation plus bundle-local evidence store.
use std::{collections::BTreeMap, io::Read, path::PathBuf};

use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use eplyx_engine::{
    dependencies::{DependencyDiscovery, DependencyManifest, ProgramDependency, ProgramSource},
    ingest::accounts,
    message::{ArchiveProvenance, LutResolutionProof},
    replay::hash_bytes,
    universal::{
        evidence::{
            AccountBoundary, AccountObservation, ChunkedAccountObservation, EvidenceKind,
            EvidenceRef, EvidenceStore,
        },
        execution::{InnerGroup, InnerInstruction, ReturnData},
        model::{
            AccountSeed, ExecutionInput, ExpectedAccountSource, ExpectedHistoricalOutcome,
            FidelityProfile, InstructionAssignment, InstructionRole, ProgramBinaryEvidence,
            ReplayObservationV2, RuntimeContext, SemanticTarget, WatchedAccount,
        },
    },
    versions::{ProgramLoader, LEGACY_BPF_LOADER_ID, UPGRADEABLE_LOADER_ID},
};
use serde_json::Value;

fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value.as_str().with_context(|| format!("missing {name}"))
}
fn bytes(value: &Value, name: &str) -> Result<Vec<u8>> {
    Ok(BASE64_STANDARD.decode(string(value, name)?)?)
}

fn content_ref(store: &EvidenceStore, reference: &EvidenceRef) -> Result<Option<EvidenceRef>> {
    match reference.kind {
        EvidenceKind::AccountObservation => {
            Ok(serde_json::from_slice::<AccountObservation>(&store.get(reference)?)?.content)
        }
        EvidenceKind::ChunkedAccountObservation => Ok(Some(
            serde_json::from_slice::<ChunkedAccountObservation>(&store.get(reference)?)?.content,
        )),
        _ => anyhow::bail!("account source reference has wrong category"),
    }
}

fn source(
    store: &EvidenceStore,
    item: &Value,
    address: &str,
    slot: u64,
    boundary: AccountBoundary,
    provider: &ArchiveProvenance,
) -> Result<EvidenceRef> {
    match string(&item["kind"], "source kind")? {
        "full" => AccountObservation::capture(
            store,
            address,
            slot,
            boundary,
            provider.clone(),
            &bytes(&item["raw_base64"], "full raw response")?,
        ),
        "chunked" => {
            ensure!(
                boundary == AccountBoundary::BeforeTransaction,
                "chunked source requires predecessor-slot context"
            );
            let receipts = item["receipts_base64"]
                .as_array()
                .context("chunk receipts")?
                .iter()
                .map(|value| bytes(value, "chunk receipt"))
                .collect::<Result<Vec<_>>>()?;
            let slices = item["slices"]
                .as_array()
                .context("chunk slices")?
                .iter()
                .map(|part| {
                    Ok((
                        part["offset"].as_u64().context("chunk offset")?,
                        bytes(&part["raw_base64"], "chunk raw response")?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            ChunkedAccountObservation::capture(
                store,
                address,
                slot,
                provider.clone(),
                &receipts,
                &slices,
            )
        }
        _ => anyhow::bail!("unknown historical account source kind"),
    }
}

fn inner(meta: &Value) -> Result<Vec<InnerGroup>> {
    meta["innerInstructions"]
        .as_array()
        .context("validator CPI groups")?
        .iter()
        .map(|group| {
            Ok(InnerGroup {
                outer_index: group["index"].as_u64().context("outer index")? as usize,
                instructions: group["instructions"]
                    .as_array()
                    .context("CPI instructions")?
                    .iter()
                    .map(|ix| {
                        Ok(InnerInstruction {
                            program_id_index: u8::try_from(
                                ix["programIdIndex"].as_u64().context("program index")?,
                            )?,
                            accounts: ix["accounts"]
                                .as_array()
                                .context("account indexes")?
                                .iter()
                                .map(|v| Ok(u8::try_from(v.as_u64().context("account index")?)?))
                                .collect::<Result<Vec<_>>>()?,
                            data: bs58::decode(string(&ix["data"], "CPI data")?).into_vec()?,
                            stack_height: u8::try_from(
                                ix["stackHeight"].as_u64().context("stack height")?,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            })
        })
        .collect()
}

fn build(import: &Value, output: &std::path::Path) -> Result<ReplayObservationV2> {
    std::fs::create_dir_all(output)?;
    let store = EvidenceStore::at(output.join("evidence"));
    let payload = &import["payload"];
    let frozen = &payload["frozen"];
    let raw = &frozen["result"];
    let genesis = string(&frozen["genesis"], "genesis")?;
    let slot = raw["slot"].as_u64().context("transaction slot")?;
    let signature = string(&raw["transaction"]["signatures"][0], "signature")?;
    let provider: ArchiveProvenance = serde_json::from_value(import["provider"].clone())?;
    ensure!(provider.genesis_hash == genesis, "provider genesis differs");
    let proof: LutResolutionProof = serde_json::from_value(payload["proof"].clone())?;
    let frozen_transaction = store.put(EvidenceKind::Transaction, &serde_json::to_vec(raw)?)?;
    let lookup_tables = frozen["evidence"]
        .as_array()
        .context("LUT evidence")?
        .iter()
        .map(|item| {
            let address = string(&item["pubkey"], "LUT address")?;
            let item_provider: ArchiveProvenance =
                serde_json::from_value(item["provider"].clone())?;
            AccountObservation::capture(
                &store,
                address,
                slot,
                AccountBoundary::EndOfExecutionSlot,
                item_provider,
                &bytes(&item["raw_response_base64"], "LUT response")?,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let execution = ExecutionInput::V0 {
        message: proof.native_v0_message.clone(),
        frozen_transaction,
        lookup_tables,
        slot_hashes: None,
        claimed_proof: proof,
    };
    let resolved = execution.resolve(&store, genesis)?;
    ensure!(
        resolved.transaction.signature == signature && resolved.transaction.slot == slot,
        "frozen transaction identity differs"
    );
    let mut seed_sources = BTreeMap::new();
    for item in import["seed_sources"].as_array().context("seed sources")? {
        let address = string(&item["address"], "source address")?;
        ensure!(
            seed_sources.insert(address.to_string(), item).is_none(),
            "duplicate seed source"
        );
    }
    let mut account_seeds = Vec::new();
    let mut runtime_seeds = Vec::new();
    let mut seed_refs = BTreeMap::new();
    let mut seed_accounts = BTreeMap::new();
    for item in payload["seeds"].as_array().context("seed accounts")? {
        let address = string(&item["address"], "seed address")?;
        let runtime_or_lut = item["kind"] == "runtime" || item["kind"] == "lut";
        let boundary = if runtime_or_lut {
            AccountBoundary::EndOfExecutionSlot
        } else {
            AccountBoundary::BeforeTransaction
        };
        let source_value = seed_sources
            .get(address)
            .context("seed has no raw historical source")?;
        let reference = source(&store, source_value, address, slot, boundary, &provider)?;
        let actual = match reference.kind {
            EvidenceKind::ChunkedAccountObservation => Some(ChunkedAccountObservation::resolve(
                &store, &reference, address, slot, genesis,
            )?),
            _ => AccountObservation::resolve(&store, &reference, address, slot, boundary, genesis)?,
        }
        .context("seed source is absent")?;
        let expected = accounts::normalize(&item["account"])?;
        ensure!(
            actual == expected
                && hash_bytes(&actual.data) == string(&item["data_sha256"], "seed data hash")?,
            "seed content differs from source"
        );
        let seed = AccountSeed {
            address: address.into(),
            boundary,
            observation: reference.clone(),
        };
        if item["kind"] == "runtime" {
            runtime_seeds.push(seed);
        } else {
            account_seeds.push(seed);
        }
        seed_refs.insert(address.to_string(), reference);
        seed_accounts.insert(address.to_string(), actual);
    }
    let mut absent_pre_accounts = Vec::new();
    for item in import["absent_sources"]
        .as_array()
        .context("absence sources")?
    {
        let address = string(&item["address"], "absent address")?;
        ensure!(
            payload["absent"]
                .as_array()
                .context("absence list")?
                .iter()
                .any(|a| a == address),
            "absence source not in frozen input"
        );
        let reference = source(
            &store,
            item,
            address,
            slot,
            AccountBoundary::BeforeTransaction,
            &provider,
        )?;
        ensure!(
            AccountObservation::resolve(
                &store,
                &reference,
                address,
                slot,
                AccountBoundary::BeforeTransaction,
                genesis
            )?
            .is_none(),
            "absence source contains an account"
        );
        absent_pre_accounts.push(AccountSeed {
            address: address.into(),
            boundary: AccountBoundary::BeforeTransaction,
            observation: reference,
        });
    }
    ensure!(
        absent_pre_accounts.len() == payload["absent"].as_array().unwrap().len(),
        "absence source set incomplete"
    );
    let mut post_sources = BTreeMap::new();
    for item in import["post_sources"].as_array().context("post sources")? {
        let address = string(&item["address"], "post address")?;
        ensure!(
            post_sources.insert(address.to_string(), item).is_none(),
            "duplicate post source"
        );
    }
    let watched_accounts = payload["watch"]
        .as_array()
        .context("watch list")?
        .iter()
        .map(|address| {
            let address = string(address, "watched address")?;
            let (source, content) = if let Some(post) = post_sources.get(address) {
                let reference = source(
                    &store,
                    post,
                    address,
                    slot,
                    AccountBoundary::EndOfExecutionSlot,
                    &provider,
                )?;
                let content = content_ref(&store, &reference)?;
                (ExpectedAccountSource::Archived(reference), content)
            } else {
                let reference = seed_refs
                    .get(address)
                    .context("watched account lacks post or pre source")?
                    .clone();
                let content = content_ref(&store, &reference)?;
                (ExpectedAccountSource::PreRetained(reference), content)
            };
            Ok(WatchedAccount {
                address: address.into(),
                source,
                expected_post_content: content,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut binaries = Vec::new();
    let mut dependencies = Vec::new();
    for program in payload["programs"].as_array().context("program manifest")? {
        let id = string(&program["program_id"], "program ID")?;
        let source: ProgramSource = serde_json::from_value(program["source"].clone())?;
        let provenance = &program["provenance"];
        let loader: Option<ProgramLoader> = provenance["loader"]
            .as_str()
            .map(|value| serde_json::from_value(Value::String(value.into())))
            .transpose()?;
        let deployed_slot = provenance["deploy_slot"]
            .as_u64()
            .or(provenance["deployment_slot"].as_u64());
        let observed_slot = program["observed_slot"]
            .as_u64()
            .or(provenance["observed_slot"].as_u64());
        let discovered_by =
            serde_json::from_value::<Vec<DependencyDiscovery>>(program["discovered_by"].clone())?;
        let mut dependency = ProgramDependency {
            program_id: id.into(),
            source,
            loader,
            deployed_slot,
            binary_sha256: None,
            binary_len: None,
            observed_slot,
            discovered_by,
            note: None,
        };
        if source == ProgramSource::HistoricalMainnet {
            let executable = seed_accounts
                .get(id)
                .context("historical program header not seeded")?;
            let (elf, programdata_address, programdata_account, loader_id) =
                match loader.context("historical loader missing")? {
                    ProgramLoader::Legacy => {
                        (executable.data.clone(), None, None, LEGACY_BPF_LOADER_ID)
                    }
                    ProgramLoader::Upgradeable => {
                        let pd_address =
                            string(&provenance["programdata_address"], "ProgramData address")?;
                        let pd = seed_accounts
                            .get(pd_address)
                            .context("ProgramData bytes not seeded")?;
                        ensure!(pd.data.len() >= 45, "ProgramData header truncated");
                        (
                            pd.data[45..].to_vec(),
                            Some(pd_address.to_string()),
                            Some(
                                seed_refs
                                    .get(pd_address)
                                    .context("ProgramData source missing")?
                                    .clone(),
                            ),
                            UPGRADEABLE_LOADER_ID,
                        )
                    }
                };
            let digest = hash_bytes(&elf);
            ensure!(
                digest == string(&provenance["sha256"], "ELF hash")?,
                "historical ELF hash differs"
            );
            let elf_ref = store.put(EvidenceKind::ProgramBinary, &elf)?;
            dependency.binary_sha256 = Some(digest);
            dependency.binary_len = Some(elf.len() as u64);
            binaries.push(ProgramBinaryEvidence {
                program_id: id.into(),
                loader: loader_id.into(),
                programdata_address,
                deployment_slot: deployed_slot,
                upgrade_authority: provenance["upgrade_authority"].as_str().map(str::to_string),
                elf: elf_ref,
                executable_account: seed_refs.get(id).context("program source missing")?.clone(),
                programdata_account,
            });
        }
        dependencies.push(dependency);
    }
    let meta = &raw["meta"];
    let logs = meta["logMessages"]
        .as_array()
        .context("validator logs")?
        .iter()
        .map(|v| string(v, "log").map(str::to_string))
        .collect::<Result<Vec<_>>>()?;
    let return_data = if meta["returnData"].is_null() {
        None
    } else {
        Some(ReturnData {
            program: string(&meta["returnData"]["programId"], "return program")?.into(),
            data: bytes(&meta["returnData"]["data"][0], "return bytes")?,
        })
    };
    let expected = ExpectedHistoricalOutcome {
        success: meta["err"].is_null(),
        error: (!meta["err"].is_null()).then(|| meta["err"].to_string()),
        fee: meta["fee"].as_u64().context("fee")?,
        logs,
        inner_instructions: inner(meta)?,
        return_data,
        watched_accounts,
    };
    let roles = import["instruction_roles"]
        .as_array()
        .context("instruction roles")?
        .iter()
        .enumerate()
        .map(|(outer_index, value)| {
            Ok(InstructionAssignment {
                outer_index,
                role: serde_json::from_value::<InstructionRole>(value.clone())?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let target_outer_index = import["target_outer_index"]
        .as_u64()
        .context("target index")? as usize;
    let program_id = resolved
        .transaction
        .instructions
        .get(target_outer_index)
        .context("target instruction missing")?
        .program
        .clone();
    let mut record = ReplayObservationV2 { schema_version: 2, id: String::new(),
        protocol: string(&import["protocol"], "protocol")?.into(), program_id: program_id.clone(),
        genesis_hash: genesis.into(), signature: signature.into(), slot, execution,
        target: SemanticTarget { program_id, outer_index: target_outer_index,
            instruction_identity: string(&import["target_identity"], "target identity")?.into() },
        instruction_roles: roles, account_seeds, absent_pre_accounts, binaries,
        dependencies: DependencyManifest { programs: dependencies },
        runtime: RuntimeContext { sysvars: runtime_seeds, feature_profile: "LiteSVM 0.16.0 mainnet".into(),
            slot_hashes_policy: "materiality_checked_default".into(), signature_check: false,
            blockhash_check: false, instructions_rule: "runtime_generated_from_complete_message".into(),
            provenance: "U3F three-profile SlotHashes materiality control; exact Clock/Rent/EpochSchedule account observations".into() },
        expected, fidelity_profile: FidelityProfile::CompleteExecutionV2 };
    record.id = record.identity()?;
    record.resolve(&store)?;
    let records = output.join("records");
    std::fs::create_dir_all(&records)?;
    let path = records.join(format!("{}.json", record.id));
    ensure!(!path.exists(), "observation identity already exists");
    std::fs::write(path, serde_json::to_vec(&record)?)?;
    Ok(record)
}

fn main() -> Result<()> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("output directory required")?,
    );
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let import: Value = serde_json::from_str(&input)?;
    let record = build(&import, &output)?;
    println!(
        "{}",
        serde_json::json!({"id":record.id,"signature":record.signature,"slot":record.slot})
    );
    Ok(())
}
