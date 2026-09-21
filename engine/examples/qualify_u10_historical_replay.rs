//! Offline qualification of the frozen U10 historical replay candidate.
//!
//! This is deliberately an evidence-bound experiment, not a product replay
//! schema. All execution behavior stays in the protocol-independent backend.

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use eplyx_engine::{
    message::{self, ArchiveProvenance, FrozenV0, HistoricalAccountEvidence},
    replay::hash_bytes,
    types::AccountSnapshot,
    universal::{
        execution::{
            ExecutionBackend, ExecutionRequest, HistoricalRuntimeEvidence, InnerGroup,
            InnerInstruction, LiteSvmBackend, ReturnData, RuntimeProfile, SlotHashesVariant,
        },
        model::ResolvedMessage,
    },
};
use serde_json::{json, Value};
use solana_hash::Hash;

const SLOT: u64 = 448_760_958;
const PRE_SLOT: u64 = SLOT - 1;
const NONCE: &str = "HCyytQceq1kmeMEmKmDutWM74M1c7CMK8nASjbSJWd94";
const EXPECTED_NONCE: &str = "GU8hFD4frrKz5zkwqU4d7wq2Ynor89uNp4dCTZy7cizE";
const ENVIRONMENT_BLOCKHASH: &str = "A2uVDFYNS3HNaYMus1fM6FztNkanJsV74K66FF4mDjZi";
const SYSVAR_OWNER: &str = "Sysvar1111111111111111111111111111111111111";
const RECENT_BLOCKHASHES: &str = "SysvarRecentB1ockHashes11111111111111111111";
const SLOT_HASHES: &str = "SysvarS1otHashes111111111111111111111111111";

fn read_json(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn snapshot(value: &Value) -> Result<AccountSnapshot> {
    Ok(AccountSnapshot {
        lamports: value["lamports"].as_u64().context("account lamports")?,
        owner: value["owner"].as_str().context("account owner")?.into(),
        data: BASE64_STANDARD.decode(value["data"][0].as_str().context("account data")?)?,
        executable: value["executable"]
            .as_bool()
            .context("account executable")?,
        rent_epoch: value["rentEpoch"].as_u64().context("account rent epoch")?,
    })
}

fn archived_account(slot: u64, address: &str) -> Result<Option<AccountSnapshot>> {
    let raw = read_json(format!(
        "docs/examples/phase-u7-archive/{slot}-{address}.body"
    ))?;
    raw["result"]["value"]
        .as_object()
        .map(|_| snapshot(&raw["result"]["value"]))
        .transpose()
}

fn acquired_sysvar(name: &str, manifest: &Value) -> Result<(String, AccountSnapshot)> {
    let row = manifest["sysvars"]
        .as_array()
        .context("sysvar acquisition rows")?
        .iter()
        .find(|row| row["name"].as_str() == Some(name))
        .with_context(|| format!("missing acquired sysvar {name}"))?;
    Ok((
        row["address"].as_str().context("sysvar address")?.into(),
        AccountSnapshot {
            lamports: row["lamports"].as_u64().context("sysvar lamports")?,
            owner: row["owner"].as_str().context("sysvar owner")?.into(),
            data: fs::read(row["account_file"].as_str().context("sysvar file")?)?,
            executable: row["executable"].as_bool().context("sysvar executable")?,
            rent_epoch: row["rent_epoch"].as_u64().context("sysvar rent epoch")?,
        },
    ))
}

fn validator_inner(meta: &Value) -> Result<Vec<InnerGroup>> {
    meta["innerInstructions"]
        .as_array()
        .context("validator inner instructions")?
        .iter()
        .map(|group| {
            Ok(InnerGroup {
                outer_index: group["index"].as_u64().context("outer index")? as usize,
                instructions: group["instructions"]
                    .as_array()
                    .context("inner instruction group")?
                    .iter()
                    .map(|ix| {
                        Ok(InnerInstruction {
                            program_id_index: u8::try_from(
                                ix["programIdIndex"].as_u64().context("program index")?,
                            )?,
                            accounts: ix["accounts"]
                                .as_array()
                                .context("inner accounts")?
                                .iter()
                                .map(|v| Ok(u8::try_from(v.as_u64().context("account index")?)?))
                                .collect::<Result<_>>()?,
                            data: bs58::decode(ix["data"].as_str().context("inner data")?)
                                .into_vec()?,
                            stack_height: u8::try_from(
                                ix["stackHeight"].as_u64().context("stack height")?,
                            )?,
                        })
                    })
                    .collect::<Result<_>>()?,
            })
        })
        .collect()
}

fn nonce_value(account: &AccountSnapshot) -> Result<String> {
    ensure!(account.data.len() == 80, "nonce account has wrong size");
    Ok(Hash::new_from_array(account.data[40..72].try_into()?).to_string())
}

fn main() -> Result<()> {
    let mutation = std::env::args().nth(1);
    let input = read_json("docs/examples/phase-u9-analysis/lut-reconstruction-input.json")?;
    let row = input
        .as_array()
        .and_then(|rows| rows.first())
        .context("frozen reconstruction input")?;
    let genesis = row["genesis"].as_str().context("genesis")?;
    let frozen = FrozenV0::from_rpc(&row["result"], genesis)?;
    let tables = row["evidence"]
        .as_array()
        .context("LUT evidence")?
        .iter()
        .map(|raw| {
            let provider: ArchiveProvenance = serde_json::from_value(raw["provider"].clone())?;
            HistoricalAccountEvidence::from_response(
                raw["pubkey"].as_str().context("LUT address")?,
                SLOT,
                provider,
                &BASE64_STANDARD.decode(
                    raw["raw_response_base64"]
                        .as_str()
                        .context("LUT response")?,
                )?,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let proven = message::reconstruct(&frozen, &tables, None)
        .map_err(|error| anyhow::anyhow!("LUT proof stage {}: {}", error.stage, error.detail))?;
    let resolved = ResolvedMessage {
        message: proven.versioned_message(),
        transaction: proven.transaction().clone(),
        account_keys: proven
            .proof()
            .full_account_keys
            .iter()
            .map(|key| key.address.clone())
            .collect(),
    };

    let closure = read_json("docs/examples/phase-u8-analysis/closure.json")?;
    let watched = closure["validation_outputs"]
        .as_array()
        .context("validation outputs")?
        .iter()
        .map(|address| {
            address
                .as_str()
                .context("validation address")
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>>>()?;

    let mut seeds = BTreeMap::new();
    let mut absent = Vec::new();
    for address in &resolved.account_keys {
        if address == RECENT_BLOCKHASHES {
            continue;
        }
        match archived_account(PRE_SLOT, address)? {
            Some(account) => {
                seeds.insert(address.clone(), account);
            }
            None => absent.push(address.clone()),
        }
    }
    // Native v0 sanitization happens inside LiteSVM as well, so the same
    // historical tables used to prove the key vector must be available there.
    for table in &tables {
        seeds.insert(table.pubkey.clone(), table.account(SLOT, genesis)?);
    }

    let acquisition = read_json("docs/examples/phase-u9-acquisition/acquisition.json")?;
    for row in acquisition["programdata"]
        .as_array()
        .context("ProgramData acquisition")?
    {
        seeds.insert(
            row["programdata_address"]
                .as_str()
                .context("ProgramData address")?
                .into(),
            AccountSnapshot {
                lamports: row["lamports"].as_u64().context("ProgramData lamports")?,
                owner: row["owner"].as_str().context("ProgramData owner")?.into(),
                data: fs::read(row["account_file"].as_str().context("ProgramData file")?)?,
                executable: false,
                rent_epoch: row["rent_epoch"]
                    .as_u64()
                    .context("ProgramData rent epoch")?,
            },
        );
    }

    let mut sysvars = BTreeMap::new();
    for name in ["Clock", "Rent", "EpochSchedule"] {
        let (address, account) = acquired_sysvar(name, &acquisition)?;
        sysvars.insert(address, account);
    }
    sysvars.insert(
        RECENT_BLOCKHASHES.into(),
        AccountSnapshot {
            lamports: 42_706_560,
            owner: SYSVAR_OWNER.into(),
            data: fs::read(
                "docs/examples/phase-u9-analysis/runtime/RecentBlockhashes.pre-transaction.bin",
            )?,
            executable: false,
            rent_epoch: u64::MAX,
        },
    );
    sysvars.insert(
        SLOT_HASHES.into(),
        AccountSnapshot {
            lamports: 143_487_360,
            owner: SYSVAR_OWNER.into(),
            data: fs::read("docs/examples/phase-u9-analysis/runtime/SlotHashes.account.bin")?,
            executable: false,
            rent_epoch: u64::MAX,
        },
    );

    let mut expected_accounts = BTreeMap::new();
    for address in &watched {
        expected_accounts.insert(address.clone(), archived_account(SLOT, address)?);
    }
    let seed_snapshot_id = hash_bytes(&serde_json::to_vec(&("eplyx-phase-u10-seeds-v1", &seeds))?);
    let checkpoint_snapshot_id = hash_bytes(&serde_json::to_vec(&(
        "eplyx-phase-u10-checkpoint-v1",
        &expected_accounts,
    ))?);
    match mutation.as_deref() {
        Some("nonce-account") => seeds.get_mut(NONCE).context("nonce seed")?.data[40] ^= 1,
        Some("historical-bpf") => {
            let address = acquisition["programdata"][0]["programdata_address"]
                .as_str()
                .context("first ProgramData address")?;
            seeds.get_mut(address).context("ProgramData seed")?.data[45] ^= 1;
        }
        Some("s-checkpoint") => {
            expected_accounts
                .get_mut(&watched[0])
                .context("first checkpoint output")?
                .as_mut()
                .context("present checkpoint output")?
                .lamports += 1
        }
        _ => {}
    }
    ensure!(
        seed_snapshot_id
            == hash_bytes(&serde_json::to_vec(&("eplyx-phase-u10-seeds-v1", &seeds,))?),
        "frozen seed snapshot identity differs"
    );
    ensure!(
        checkpoint_snapshot_id
            == hash_bytes(&serde_json::to_vec(&(
                "eplyx-phase-u10-checkpoint-v1",
                &expected_accounts,
            ))?),
        "checkpoint S snapshot identity differs"
    );

    let feature_profile = if mutation.as_deref() == Some("feature-profile") {
        "mutated-feature-profile"
    } else {
        "LiteSVM 0.16.0 mainnet"
    };
    let environment_blockhash = if mutation.as_deref() == Some("environment-blockhash") {
        Hash::new_from_array([42; 32]).to_string()
    } else {
        ENVIRONMENT_BLOCKHASH.into()
    };
    let historical = HistoricalRuntimeEvidence::new(
        environment_blockhash.clone(),
        RuntimeProfile::sysvar_snapshot_hash(&sysvars)?,
        feature_profile.into(),
        "agave-4.2.2-native-system-compute".into(),
        "phase-u9-runtime-profile:eb8ab40820255573c0b2d4534783eda74b1525056afd34696052be9cde734852"
            .into(),
    )?;
    let mut profile = RuntimeProfile::resolve(
        Some(&historical),
        &sysvars,
        feature_profile,
        false,
        false,
        "runtime_generated_from_complete_message",
        "historical",
    )?;
    if mutation.as_deref() == Some("runtime-profile-identity") {
        profile.profile_id.replace_range(..1, "0");
    }
    if mutation.as_deref() == Some("recent-blockhashes") {
        sysvars
            .get_mut(RECENT_BLOCKHASHES)
            .context("RecentBlockhashes sysvar")?
            .data[8] ^= 1;
    }

    let execution = LiteSvmBackend.execute(&ExecutionRequest {
        message: &resolved,
        seeds: &seeds,
        absent_pre_accounts: &absent,
        watched: &watched,
        runtime_sysvars: &sysvars,
        clock: None,
        programs_to_load: &[],
        runtime_profile: &profile,
        unlimited_logs: true,
        slot_hashes: SlotHashesVariant::BackendDefault,
        require_complete_state: true,
    })?;

    let post_nonce = execution.post_accounts[NONCE]
        .as_ref()
        .context("nonce absent after execution")?;
    let actual_nonce = nonce_value(post_nonce)?;
    let nonce_matches = actual_nonce == EXPECTED_NONCE;
    ensure!(
        nonce_matches,
        "RuntimeConfigurationMismatch: durable nonce differs"
    );

    let meta = &row["result"]["meta"];
    let validator_logs = meta["logMessages"]
        .as_array()
        .context("validator logs")?
        .iter()
        .map(|line| line.as_str().context("validator log").map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    let validator_inner = validator_inner(meta)?;
    let validator_return = if meta["returnData"].is_null() {
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
    let local_inner = execution
        .inner_instructions
        .iter()
        .filter(|group| !group.instructions.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let fidelity = json!({
        "outcome": execution.success == meta["err"].is_null(),
        "error": execution.error.is_none() == meta["err"].is_null(),
        "fee": execution.fee == meta["fee"].as_u64().context("validator fee")?,
        "compute_units": execution.compute_units == meta["computeUnitsConsumed"].as_u64().context("validator compute units")?,
        "logs": execution.logs == validator_logs,
        "cpi": local_inner == validator_inner,
        "return_data": match validator_return { Some(ref expected) => expected == &execution.return_data, None => execution.return_data.data.is_empty() },
    });

    let mut reconciliation = Vec::new();
    for address in &watched {
        let expected = expected_accounts
            .get(address)
            .context("checkpoint output missing")?;
        let actual = execution
            .post_accounts
            .get(address)
            .context("watched output missing")?;
        reconciliation.push(json!({
            "address": address,
            "matched": expected == actual,
            "expected_present": expected.is_some(),
            "actual_present": actual.is_some(),
        }));
    }

    let output = json!({
        "schema": "eplyx.phase-u10.historical-replay.v1",
        "slot": SLOT,
        "signature": resolved.transaction.signature,
        "message_version": resolved.transaction.version,
        "transaction_recent_blockhash": resolved.transaction.recent_blockhash,
        "environment_blockhash": environment_blockhash,
        "mutation": mutation,
        "historical_runtime_evidence_id": historical.evidence_id,
        "runtime_profile_id": profile.profile_id,
        "runtime_sysvar_snapshot_hash": profile.sysvar_snapshot_hash,
        "seed_snapshot_id": seed_snapshot_id,
        "checkpoint_snapshot_id": checkpoint_snapshot_id,
        "lut_proof_id": proven.proof().proof_id,
        "historical_programdata_accounts": acquisition["programdata"].as_array().map_or(0, Vec::len),
        "execution": execution,
        "nonce": {"expected": EXPECTED_NONCE, "actual": actual_nonce, "matched": nonce_matches},
        "validator_fidelity": fidelity,
        "account_reconciliation": reconciliation,
        "execution_evidence_sha256": hash_bytes(&serde_json::to_vec(&execution)?),
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
