//! Isolated historical sequence feasibility. This does not create an observation.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{anyhow, ensure, Context, Result};
use base64::Engine;
use eplyx_engine::{
    ingest::accounts,
    message::{self, ArchiveProvenance, FrozenV0, HistoricalAccountEvidence},
    types::AccountSnapshot,
    universal::{
        execution::{HistoricalRuntimeEvidence, RuntimeProfile},
        historical_features::{HistoricalFeatureReceipt, HistoricalFeatureSetEvidence},
        model::ResolvedMessage,
    },
};
use litesvm::{InvocationInspectCallback, LiteSVM};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_address::Address;
use solana_hash::Hash;
use solana_message::VersionedMessage;
use solana_program_runtime::invoke_context::InvokeContext;
use solana_transaction::{sanitized::SanitizedTransaction, versioned::VersionedTransaction};

const ROOT: &str = "docs/examples/phase-u13-2c-sequence";
const U13: &str = "docs/examples/phase-u13-drift-witness/raw";
const U13_1: &str = "docs/examples/phase-u13-1-causal-closure";
const U13_2A: &str = "docs/examples/phase-u13-2a-runtime";
const U13_2B: &str = "docs/examples/phase-u13-2b-feature-universe";
const GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const PROGRAM: &str = "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH";
const PROGRAM_DATA: &str = "7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP";
const COMPUTE_BUDGET: &str = "ComputeBudget111111111111111111111111111111";
const SPOT: &str = "6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3";
const INDEXES: [usize; 5] = [73, 428, 431, 438, 1245];

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_json(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn snapshot(response: &Value) -> Result<AccountSnapshot> {
    accounts::normalize(&response["result"]["value"])
}
fn solana_account(value: &AccountSnapshot) -> Result<Account> {
    Ok(Account {
        lamports: value.lamports,
        data: value.data.clone(),
        owner: value.owner.parse()?,
        executable: value.executable,
        rent_epoch: value.rent_epoch,
    })
}
fn account_snapshot(value: &Account) -> AccountSnapshot {
    AccountSnapshot {
        lamports: value.lamports,
        data: value.data.clone(),
        owner: value.owner.to_string(),
        executable: value.executable,
        rent_epoch: value.rent_epoch,
    }
}
fn world_account(svm: &LiteSVM, key: &str) -> Result<AccountSnapshot> {
    let address: Address = key.parse()?;
    Ok(account_snapshot(
        &svm.get_account(&address)
            .with_context(|| format!("missing {key}"))?,
    ))
}

#[derive(Clone, Copy)]
struct EnvironmentBlockhash(Hash);
impl InvocationInspectCallback for EnvironmentBlockhash {
    fn before_invocation(
        &self,
        _svm: &LiteSVM,
        _tx: &SanitizedTransaction,
        _program_indices: &[u16],
        context: &mut InvokeContext<'_, '_>,
        _enable_register_tracing: bool,
    ) {
        context.environment_config.blockhash = self.0;
    }
    fn after_invocation(
        &self,
        _svm: &LiteSVM,
        _tx: &SanitizedTransaction,
        _program_indices: &[u16],
        _context: &InvokeContext<'_, '_>,
        _enable_register_tracing: bool,
    ) {
    }
}

fn profile() -> Result<(RuntimeProfile, BTreeMap<String, AccountSnapshot>)> {
    let audit = read_json(format!("{U13_2B}/audit.json"))?;
    let receipts = audit["observations"]
        .as_array()
        .context("feature observations")?
        .iter()
        .map(|row| -> Result<_> {
            Ok(HistoricalFeatureReceipt {
                id: row["id"].as_str().context("feature ID")?.into(),
                source_sha256: row["responseSha256"]
                    .as_str()
                    .context("response hash")?
                    .into(),
                source_response: fs::read_to_string(row["source"].as_str().context("source")?)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let features = HistoricalFeatureSetEvidence::new(409942000, receipts)?;
    let mut sysvars = BTreeMap::new();
    for (name, address) in [
        ("Clock", "SysvarC1ock11111111111111111111111111111111"),
        ("Rent", "SysvarRent111111111111111111111111111111111"),
        (
            "EpochSchedule",
            "SysvarEpochSchedu1e111111111111111111111111",
        ),
    ] {
        sysvars.insert(
            address.into(),
            snapshot(&read_json(format!("{U13_2A}/raw/sysvar-{name}.json"))?)?,
        );
    }
    let historical = HistoricalRuntimeEvidence::new(
        "EK66pFxagY8mA6VjRmNnBqhZwcQ44jFztfNVfueaDWgM".into(),
        RuntimeProfile::sysvar_snapshot_hash(&sysvars)?,
        "LiteSVM 0.16.0 historical evidence".into(),
        "agave-4.2.2-native-system-compute".into(),
        "U13.2B exact-slot account archive receipts".into(),
    )?
    .with_historical_feature_set(features)?;
    let profile = RuntimeProfile::resolve(
        Some(&historical),
        &sysvars,
        "LiteSVM 0.16.0 historical evidence",
        false,
        false,
        "runtime_generated_from_complete_message",
        "materiality_checked_default",
    )?;
    ensure!(
        profile.profile_id == "82d133c01dfdffd1d28d4115f969e7ecf73178c41829ccdc3b882910c270e639",
        "U13.2B profile changed"
    );
    Ok((profile, sysvars))
}

fn parent_seeds(closure: &Value) -> Result<BTreeMap<String, AccountSnapshot>> {
    let mut seeds = BTreeMap::new();
    for key in closure["parentStateFrontier"]
        .as_array()
        .context("parent frontier")?
    {
        let key = key.as_str().context("parent key")?;
        if key == COMPUTE_BUDGET || key == PROGRAM_DATA {
            continue;
        }
        let path = format!("{U13_1}/raw/account-409941999-{key}.json");
        let receipt = if Path::new(&path).exists() {
            path
        } else if key == PROGRAM {
            format!("{U13}/accounts/409941999-program.json")
        } else {
            format!("{U13}/accounts/409941999-{key}.json")
        };
        seeds.insert(key.into(), snapshot(&read_json(receipt)?)?);
    }
    let mut programdata = snapshot(&read_json(format!(
        "{U13}/accounts/409941999-programdata-header.json"
    ))?)?;
    let elf = fs::read(format!("{U13_2A}/historical-drift.so"))?;
    ensure!(
        sha(&elf) == "56bb3c1218ca4cc1158116c008439a48ca942f7e2898227494ce2769a7fdce5d",
        "historical ELF hash"
    );
    programdata.data.truncate(45);
    programdata.data.extend(elf);
    ensure!(
        sha(&programdata.data)
            == "ca89b99a4ce9cd09a35e7fd881b09099397543a9a1122e93f9a68b1349007728",
        "ProgramData hash"
    );
    seeds.insert(PROGRAM_DATA.into(), programdata);
    ensure!(seeds.len() == 23, "parent frontier count");
    Ok(seeds)
}

type ResolvedInputs = (Vec<ResolvedMessage>, Vec<Value>, Vec<Value>, Vec<Value>);
fn resolved_messages(closure: &Value) -> Result<ResolvedInputs> {
    let block = read_json(format!("{U13}/block-409942000.json"))?;
    ensure!(
        sha(&fs::read(format!("{U13}/block-409942000.json"))?)
            == closure["source"]["sha256"].as_str().context("block hash")?,
        "frozen block differs"
    );
    let acquisition = read_json(format!("{ROOT}/lut-acquisition.json"))?;
    let provider: ArchiveProvenance = serde_json::from_value(
        read_json("docs/examples/phase-u9-analysis/lut-reconstruction-input.json")?[0]["evidence"]
            [0]["provider"]
            .clone(),
    )?;
    ensure!(
        acquisition["slot"] == 409942000
            && acquisition["providerHost"]
                == provider
                    .scheme_host
                    .strip_prefix("https://")
                    .context("LUT provider scheme")?,
        "LUT acquisition provider or slot differs"
    );
    let mut tables = BTreeMap::new();
    let mut table_report = Vec::new();
    for row in acquisition["receipts"].as_array().context("LUT receipts")? {
        let key = row["address"].as_str().context("LUT address")?;
        let raw = fs::read(format!(
            "{ROOT}/raw/{}",
            row["name"].as_str().context("LUT file")?
        ))?;
        ensure!(
            sha(&raw) == row["responseSha256"],
            "LUT receipt hash differs"
        );
        let evidence =
            HistoricalAccountEvidence::from_response(key, 409942000, provider.clone(), &raw)?;
        ensure!(
            sha(&evidence.account(409942000, GENESIS)?.data)
                == row["accountDataSha256"].as_str().context("LUT data hash")?,
            "LUT account data hash differs"
        );
        let parent = snapshot(&read_json(format!(
            "{U13_1}/raw/account-409941999-{key}.json"
        ))?)?;
        ensure!(
            evidence.account(409942000, GENESIS)? == parent,
            "LUT changed from qualified parent frontier"
        );
        table_report.push(json!({"address":key,"responseSha256":row["responseSha256"],
            "dataSha256":row["accountDataSha256"],"parentEqualsExecutionSlot":true}));
        tables.insert(key.to_string(), evidence);
    }
    let mut messages = Vec::new();
    let mut validators = Vec::new();
    let mut resolution_report = Vec::new();
    for (position, index) in INDEXES.iter().enumerate() {
        ensure!(
            closure["transactionIndexes"][position] == *index,
            "closure sequence differs"
        );
        let mut raw = block["result"]["transactions"][*index].clone();
        raw["slot"] = json!(409942000u64);
        raw["transactionIndex"] = json!(index);
        raw["blockTime"] = block["result"]["blockTime"].clone();
        let frozen = FrozenV0::from_rpc(&raw, GENESIS)?;
        let referenced = frozen
            .native_message()
            .address_table_lookups
            .iter()
            .map(|l| {
                tables
                    .get(&l.account_key.to_string())
                    .cloned()
                    .context("missing LUT evidence")
            })
            .collect::<Result<Vec<_>>>()?;
        let proven = message::reconstruct(&frozen, &referenced, None)
            .map_err(|e| anyhow!("tx {index} LUT proof stage {}: {}", e.stage, e.detail))?;
        message::validate_proof(&frozen, &referenced, None, proven.proof()).map_err(|e| {
            anyhow!(
                "tx {index} LUT proof roundtrip stage {}: {}",
                e.stage,
                e.detail
            )
        })?;
        let full_keys = proven
            .proof()
            .full_account_keys
            .iter()
            .map(|k| k.address.clone())
            .collect::<Vec<_>>();
        let expected = &closure["transactions"][position];
        ensure!(
            proven.transaction().signature
                == expected["signature"].as_str().context("signature")?
                && proven.transaction().slot == 409942000
                && expected["index"] == *index
                && expected["messageVersion"] == 0
                && expected["success"] == true
                && expected["fee"] == raw["meta"]["fee"]
                && expected["accountReads"] == json!(full_keys)
                && expected["lutReferences"]
                    == json!(frozen
                        .native_message()
                        .address_table_lookups
                        .iter()
                        .map(|l| l.account_key.to_string())
                        .collect::<Vec<_>>()),
            "tx {index} differs from qualified closure"
        );
        ensure!(
            raw["meta"]["err"].is_null(),
            "closure tx {index} failed on validator"
        );
        let programs = proven
            .transaction()
            .instructions
            .iter()
            .map(|instruction| instruction.program.clone())
            .collect::<BTreeSet<_>>();
        ensure!(
            json!(programs) == expected["programIds"],
            "tx {index} invoked program census differs"
        );
        resolution_report.push(json!({"index":index,
            "signature":proven.transaction().signature,
            "slot":proven.transaction().slot,"version":0,
            "staticKeys":frozen.native_message().account_keys.iter()
                .map(ToString::to_string).collect::<Vec<_>>(),
            "lookups":frozen.native_message().address_table_lookups.iter().map(|lookup|
                json!({"accountKey":lookup.account_key.to_string(),
                    "writableIndexes":lookup.writable_indexes,
                    "readonlyIndexes":lookup.readonly_indexes})).collect::<Vec<_>>(),
            "loadedWritable":raw["meta"]["loadedAddresses"]["writable"],
            "loadedReadonly":raw["meta"]["loadedAddresses"]["readonly"],
            "fullAccountKeys":full_keys,
            "instructionSha256":sha(&serde_json::to_vec(&frozen.native_message().instructions)?),
            "instructionCount":frozen.native_message().instructions.len(),
            "invokedProgramIds":programs,"lookupProofId":proven.proof().proof_id}));
        messages.push(ResolvedMessage {
            message: proven.versioned_message(),
            transaction: proven.transaction().clone(),
            account_keys: full_keys,
        });
        validators.push(raw);
    }
    Ok((messages, validators, table_report, resolution_report))
}

fn make_world(
    profile: &RuntimeProfile,
    sysvars: &BTreeMap<String, AccountSnapshot>,
    seeds: &BTreeMap<String, AccountSnapshot>,
    default_features: bool,
) -> Result<LiteSVM> {
    profile.validate()?;
    ensure!(
        RuntimeProfile::sysvar_snapshot_hash(sysvars)? == profile.sysvar_snapshot_hash,
        "sysvar profile changed"
    );
    let mut svm = if default_features {
        LiteSVM::new()
    } else {
        LiteSVM::default()
            .with_feature_set(
                profile
                    .historical_feature_set
                    .as_ref()
                    .context("historical features")?
                    .feature_set()?,
            )
            .with_builtins()
            .with_lamports(1_000_000u64.wrapping_mul(1_000_000_000))
            .with_sysvars()
            .with_feature_accounts()
            .with_default_programs()
            .with_sigverify(true)
            .with_blockhash_check(true)
    }
    .with_sigverify(profile.signature_check)
    .with_blockhash_check(profile.recent_blockhash_check)
    .with_log_bytes_limit(None);
    svm.set_invocation_inspect_callback(EnvironmentBlockhash(
        profile
            .environment_blockhash
            .as_ref()
            .context("environment blockhash")?
            .parse()?,
    ));
    for (key, value) in sysvars {
        svm.set_account(key.parse()?, solana_account(value)?)
            .map_err(|e| anyhow!("sysvar {key}: {e:?}"))?;
    }
    for executable in [false, true] {
        for (key, value) in seeds {
            if value.executable == executable {
                svm.set_account(key.parse()?, solana_account(value)?)
                    .map_err(|e| anyhow!("seed {key}: {e:?}"))?;
            }
        }
    }
    Ok(svm)
}

#[allow(clippy::too_many_arguments)]
fn run(
    profile: &RuntimeProfile,
    sysvars: &BTreeMap<String, AccountSnapshot>,
    seeds: &BTreeMap<String, AccountSnapshot>,
    messages: &[ResolvedMessage],
    validators: &[Value],
    closure: &Value,
    default_features: bool,
    reset_spot_after: Option<usize>,
    order: &[usize],
) -> Result<Value> {
    let mut svm = make_world(profile, sysvars, seeds, default_features)?;
    let terminal = closure["terminalStateFrontier"]
        .as_array()
        .context("terminal frontier")?
        .iter()
        .map(|v| v.as_str().context("terminal key").map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    let state_frontier = closure["parentStateFrontier"]
        .as_array()
        .context("state frontier")?
        .iter()
        .map(|v| v.as_str().context("state key").map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    let mut rows = Vec::new();
    for (position, message_index) in order.iter().copied().enumerate() {
        let message = &messages[message_index];
        ensure!(message.transaction.slot == 409942000, "mixed runtime slot");
        ensure!(
            message
                .account_keys
                .iter()
                .all(|key| seeds.contains_key(key)
                    || sysvars.contains_key(key)
                    || key == COMPUTE_BUDGET),
            "unseeded message account"
        );
        let native = match &message.message {
            VersionedMessage::V0(v) => v.clone(),
            _ => anyhow::bail!("native v0 required"),
        };
        let tx = VersionedTransaction {
            signatures: vec![message.transaction.signature.parse()?],
            message: VersionedMessage::V0(native),
        };
        let (success, error, meta) = match svm.send_transaction(tx) {
            Ok(meta) => (true, None, meta),
            Err(failure) => (false, Some(format!("{:?}", failure.err)), failure.meta),
        };
        let validator = &validators[message_index]["meta"];
        let logs = validator["logMessages"]
            .as_array()
            .context("validator logs")?
            .iter()
            .map(|v| v.as_str().context("validator log").map(str::to_owned))
            .collect::<Result<Vec<_>>>()?;
        let inner_count = meta.inner_instructions.iter().map(Vec::len).sum::<usize>();
        let validator_inner_count = validator["innerInstructions"]
            .as_array()
            .context("validator inner rows")?
            .iter()
            .map(|group| group["instructions"].as_array().map_or(0, Vec::len))
            .sum::<usize>();
        let envelope_match = success == validator["err"].is_null()
            && meta.fee == validator["fee"].as_u64().context("validator fee")?
            && meta.compute_units_consumed
                == validator["computeUnitsConsumed"]
                    .as_u64()
                    .context("validator CU")?
            && meta.logs == logs
            && inner_count == validator_inner_count
            && meta.return_data.data.is_empty() == validator["returnData"].is_null();
        let spot = world_account(&svm, SPOT)?;
        let state = state_frontier
            .iter()
            .filter(|key| key.as_str() != COMPUTE_BUDGET)
            .map(|key| -> Result<_> {
                let value = world_account(&svm, key)?;
                Ok((
                    key.clone(),
                    json!({"accountSha256":sha(&serde_json::to_vec(&value)?),
                "dataSha256":sha(&value.data),"lamports":value.lamports}),
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        rows.push(
            json!({"index":INDEXES[message_index], "signature":message.transaction.signature,
            "success":success,"error":error,"fee":meta.fee,
            "computeUnits":meta.compute_units_consumed,"logs":meta.logs,
            "innerInstructionCount":inner_count,"returnDataBytes":meta.return_data.data.len(),
            "validatorSuccess":validator["err"].is_null(),"validatorFee":validator["fee"],
            "validatorComputeUnits":validator["computeUnitsConsumed"],
            "validatorLogs":logs,"validatorInnerInstructionCount":validator_inner_count,
            "validatorReturnDataPresent":!validator["returnData"].is_null(),
            "envelopeMatch":envelope_match,"derivedSpotDataSha256":sha(&spot.data),
            "derivedSpotAccountSha256":sha(&serde_json::to_vec(&spot)?),
            "derivedState":state}),
        );
        if reset_spot_after == Some(position) {
            svm.set_account(
                SPOT.parse()?,
                solana_account(seeds.get(SPOT).context("parent spot")?)?,
            )
            .map_err(|e| anyhow!("diagnostic SpotMarket reset: {e:?}"))?;
        }
    }
    let terminal_comparisons = terminal
        .iter()
        .map(|key| -> Result<_> {
            let path = format!("{U13_1}/raw/account-409942000-{key}.json");
            let receipt = if Path::new(&path).exists() {
                path
            } else {
                format!("{U13}/accounts/409942000-{key}.json")
            };
            let expected = snapshot(&read_json(receipt)?)?;
            let actual = world_account(&svm, key)?;
            Ok((
                key.clone(),
                json!({"fullAccountEqual":actual == expected,
            "actualAccountSha256":sha(&serde_json::to_vec(&actual)?),
            "expectedAccountSha256":sha(&serde_json::to_vec(&expected)?),
            "actualDataSha256":sha(&actual.data),
            "expectedDataSha256":sha(&expected.data),
            "actualLamports":actual.lamports,"expectedLamports":expected.lamports}),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(
        json!({"transactions":rows,"terminalComparisons":terminal_comparisons,
        "allEnvelopesMatch":rows.iter().all(|r| r["envelopeMatch"] == true),
        "allTerminalAccountsMatch":terminal_comparisons.values().all(|r| r["fullAccountEqual"] == true)}),
    )
}

fn control_result(result: &Value) -> Value {
    let first_envelope_mismatch = result["transactions"].as_array().and_then(|rows| {
        rows.iter()
            .find(|row| row["envelopeMatch"] != true)
            .map(|row| row["index"].clone())
    });
    let terminal_mismatches = result["terminalComparisons"]
        .as_object()
        .map(|accounts| {
            accounts
                .iter()
                .filter(|(_, row)| row["fullAccountEqual"] != true)
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({"allEnvelopesMatch":result["allEnvelopesMatch"],
        "allTerminalAccountsMatch":result["allTerminalAccountsMatch"],
        "firstEnvelopeMismatch":first_envelope_mismatch,
        "terminalMismatches":terminal_mismatches,
        "detectedByExecution":first_envelope_mismatch.is_some() || !terminal_mismatches.is_empty()})
}

fn order_violations(closure: &Value, order: &[usize]) -> Result<Vec<Value>> {
    let positions = order
        .iter()
        .enumerate()
        .map(|(position, message_index)| (INDEXES[*message_index], position))
        .collect::<BTreeMap<_, _>>();
    Ok(closure["dependencyEdges"]
        .as_array()
        .context("dependency edges")?
        .iter()
        .filter(|edge| {
            let from = edge["from"].as_u64().unwrap_or_default() as usize;
            let to = edge["to"].as_u64().unwrap_or_default() as usize;
            matches!((positions.get(&from), positions.get(&to)),
                (Some(left), Some(right)) if left >= right)
        })
        .cloned()
        .collect())
}

fn mutated_lut_rejected(validators: &[Value]) -> Result<Value> {
    let index = 428usize;
    let raw = &validators[1];
    let frozen = FrozenV0::from_rpc(raw, GENESIS)?;
    let lookup = &frozen.native_message().address_table_lookups[0];
    let key = lookup.account_key.to_string();
    let path = format!("{ROOT}/raw/account-409942000-{key}.json");
    let mut response = read_json(path)?;
    let data = response["result"]["value"]["data"][0]
        .as_str()
        .context("LUT base64")?;
    let mut bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
    let used_index = *lookup
        .writable_indexes
        .first()
        .or_else(|| lookup.readonly_indexes.first())
        .context("LUT used index")? as usize;
    let byte = 56 + used_index * 32;
    ensure!(byte < bytes.len(), "LUT mutation index out of bounds");
    bytes[byte] ^= 1;
    response["result"]["value"]["data"][0] =
        json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
    let provider: ArchiveProvenance = serde_json::from_value(
        read_json("docs/examples/phase-u9-analysis/lut-reconstruction-input.json")?[0]["evidence"]
            [0]["provider"]
            .clone(),
    )?;
    let mut evidence = Vec::new();
    for descriptor in &frozen.native_message().address_table_lookups {
        let address = descriptor.account_key.to_string();
        let body = if address == key {
            serde_json::to_vec(&response)?
        } else {
            fs::read(format!("{ROOT}/raw/account-409942000-{address}.json"))?
        };
        evidence.push(HistoricalAccountEvidence::from_response(
            &address,
            409942000,
            provider.clone(),
            &body,
        )?);
    }
    let failure = message::reconstruct(&frozen, &evidence, None)
        .err()
        .context("mutated LUT unexpectedly resolved")?;
    Ok(json!({"transactionIndex":index,"mutatedTable":key,
        "usedAddressIndex":used_index,"proofStage":failure.stage,
        "proofFailure":failure.detail,"detectedByResolution":true}))
}

fn main() -> Result<()> {
    let closure = read_json(format!("{U13_1}/closure.json"))?;
    ensure!(
        closure["transactionIndexes"] == json!(INDEXES),
        "frozen closure differs"
    );
    ensure!(
        closure["dependencyEdges"]
            .as_array()
            .is_some_and(|v| v.len() == 11),
        "dependency graph differs"
    );
    ensure!(
        closure["summary"]["includedSuccessful"] == 5
            && closure["summary"]["includedFailed"] == 0
            && closure["failedDeclaredOverlaps"]
                .as_array()
                .is_some_and(Vec::is_empty),
        "closure success/failure classification differs"
    );
    for key in [
        PROGRAM,
        PROGRAM_DATA,
        "EiWSskK5HXnBTptiS5DH6gpAJRVNQ3cAhTKBGaiaysAb",
        "Fpys8GRa5RBWfyeN7AaDUwFGD1zkDCA4z3t4CJLV8dfL",
    ] {
        ensure!(
            closure["accountWriterCensus"]
                .as_array()
                .context("writer census")?
                .iter()
                .any(|row| row["account"] == key
                    && row["committedWriterIndexes"]
                        .as_array()
                        .is_some_and(Vec::is_empty)),
            "historical program or LUT has a committed block writer"
        );
    }
    let (messages, validators, luts, resolutions) = resolved_messages(&closure)?;
    let (profile, sysvars) = profile()?;
    let seeds = parent_seeds(&closure)?;
    ensure!(
        order_violations(&closure, &[0, 1, 2, 3, 4])?.is_empty(),
        "qualified closure order violates its dependencies"
    );
    let baseline = run(
        &profile,
        &sysvars,
        &seeds,
        &messages,
        &validators,
        &closure,
        false,
        None,
        &[0, 1, 2, 3, 4],
    )?;
    ensure!(
        baseline["allEnvelopesMatch"] == true && baseline["allTerminalAccountsMatch"] == true,
        "baseline sequence did not reconcile"
    );
    let mut controls = BTreeMap::new();
    for (omitted, index) in INDEXES.iter().enumerate().skip(1) {
        let order = (0..5).filter(|i| *i != omitted).collect::<Vec<_>>();
        let result = run(
            &profile,
            &sysvars,
            &seeds,
            &messages,
            &validators,
            &closure,
            false,
            None,
            &order,
        )?;
        controls.insert(format!("omit_{index}"), control_result(&result));
    }
    let reordered = run(
        &profile,
        &sysvars,
        &seeds,
        &messages,
        &validators,
        &closure,
        false,
        None,
        &[0, 2, 1, 3, 4],
    )?;
    let violated_edges = order_violations(&closure, &[0, 2, 1, 3, 4])?;
    ensure!(violated_edges.len() == 4, "reorder edge count differs");
    controls.insert(
        "reorder_428_431".into(),
        json!({
        "behavioralComparison":control_result(&reordered),
        "dependencyOrderValid":false,"violatedEdges":violated_edges,
        "detectedByDependencyResolution":true}),
    );
    let mut changed_messages = messages.clone();
    match &mut changed_messages[1].message {
        VersionedMessage::V0(v0) => {
            v0.instructions
                .last_mut()
                .context("later instruction")?
                .data[0] ^= 1
        }
        _ => anyhow::bail!("native v0 required"),
    }
    let changed_message = run(
        &profile,
        &sysvars,
        &seeds,
        &changed_messages,
        &validators,
        &closure,
        false,
        None,
        &[0, 1, 2, 3, 4],
    )?;
    controls.insert(
        "mutate_428_instruction_data".into(),
        control_result(&changed_message),
    );
    controls.insert(
        "mutate_lut_used_by_428".into(),
        mutated_lut_rejected(&validators)?,
    );
    let mut changed_seeds = seeds.clone();
    changed_seeds
        .get_mut("3e5QUcAj1qWHRjtphaKVguitkZx6Rnun6CSCJibnwxZM")
        .context("later-only parent account")?
        .data[0] ^= 1;
    let changed_parent = run(
        &profile,
        &sysvars,
        &changed_seeds,
        &messages,
        &validators,
        &closure,
        false,
        None,
        &[0, 1, 2, 3, 4],
    )?;
    controls.insert(
        "mutate_later_only_parent_account".into(),
        control_result(&changed_parent),
    );
    let reset = run(
        &profile,
        &sysvars,
        &seeds,
        &messages,
        &validators,
        &closure,
        false,
        Some(0),
        &[0, 1, 2, 3, 4],
    )?;
    controls.insert("reset_spot_after_73".into(), control_result(&reset));
    let default = run(
        &profile,
        &sysvars,
        &seeds,
        &messages,
        &validators,
        &closure,
        true,
        None,
        &[0, 1, 2, 3, 4],
    )?;
    controls.insert(
        "current_default_features_diagnostic".into(),
        json!({"historicalFeatureEvidenceSatisfied":false,
            "behavioralComparison":control_result(&default),
            "completeExecutionEqual":default == baseline}),
    );
    let report = json!({"schema":"U13_2CSequenceFeasibilityV1",
        "classification":"feasibility_only","runtimeProfileId":profile.profile_id,
        "featureInventoryId":profile.historical_feature_set.as_ref().unwrap().inventory_id,
        "featureSetHash":profile.historical_feature_set.as_ref().unwrap().feature_set_hash,
        "sourceBlockSha256":closure["source"]["sha256"],
        "parentFrontier":closure["parentStateFrontier"],
        "dependencyEdges":closure["dependencyEdges"],
        "invokedProgramIds":closure["distinctProgramIds"],
        "luts":luts,"messageResolutions":resolutions,
        "baseline":baseline,"controls":controls});
    fs::write(
        format!("{ROOT}/feasibility.json"),
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "allEnvelopesMatch":baseline["allEnvelopesMatch"],
            "allTerminalAccountsMatch":baseline["allTerminalAccountsMatch"],
            "transactions":baseline["transactions"].as_array().unwrap().iter().map(|row|
                json!({"index":row["index"],"success":row["success"],
                    "fee":row["fee"],"computeUnits":row["computeUnits"],
                    "envelopeMatch":row["envelopeMatch"],
                    "spot":row["derivedSpotDataSha256"]})).collect::<Vec<_>>()
        }))?
    );
    Ok(())
}
