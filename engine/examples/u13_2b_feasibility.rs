//! Isolated witness feasibility, never a ReplayObservationV2 or fidelity claim.
use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{ensure, Context, Result};
use eplyx_engine::{
    ingest::transactions,
    message::FrozenV0,
    types::AccountSnapshot,
    universal::{
        execution::{
            ExecutionBackend, ExecutionEvidence, ExecutionRequest, HistoricalRuntimeEvidence,
            LiteSvmBackend, RecentBlockhashesVariant, RuntimeProfile, SlotHashesVariant,
        },
        historical_features::{HistoricalFeatureReceipt, HistoricalFeatureSetEvidence},
        model::ResolvedMessage,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solana_message::VersionedMessage;

const ROOT: &str = "docs/examples/phase-u13-2b-feature-universe";
const WITNESS: &str = "docs/examples/phase-u13-drift-witness/raw/accounts";
const U13A: &str = "docs/examples/phase-u13-2a-runtime";
const GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const USER: &str = "JE9m89yHHiCGzzL2FAeeZgHKAFwjkW4Qp1GfjegWnojR";
const PERP: &str = "7QAtMC3AaAc91W4XuwYXM1Mtffq9h9Z8dTxcJrKRHu1z";
const SPOT: &str = "6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3";
const PROGRAM_DATA: &str = "7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP";

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fingerprint(e: &ExecutionEvidence) -> Value {
    json!({"success": e.success, "error": e.error, "fee": e.fee,
        "computeUnits": e.compute_units, "logsSha256": sha(e.logs.join("\n").as_bytes()),
        "innerInstructions": e.inner_instructions, "returnData": e.return_data,
        "outputs": e.post_accounts.iter().map(|(key, value)| {
            (key.clone(), value.as_ref().map(|a| json!({"lamports": a.lamports,
                "dataSha256": sha(&a.data)})))
        }).collect::<BTreeMap<_, _>>()})
}
fn read_json(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn account(value: &Value) -> Result<AccountSnapshot> {
    let value = &value["result"]["value"];
    let data = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        value["data"][0].as_str().context("account base64")?,
    )?;
    ensure!(
        value["space"]
            .as_u64()
            .is_some_and(|space| space >= data.len() as u64),
        "account space"
    );
    Ok(AccountSnapshot {
        lamports: value["lamports"].as_u64().context("lamports")?,
        owner: value["owner"].as_str().context("owner")?.into(),
        data,
        executable: value["executable"].as_bool().context("executable")?,
        rent_epoch: value["rentEpoch"].as_u64().context("rent epoch")?,
    })
}
fn main() -> Result<()> {
    let audit = read_json(format!("{ROOT}/audit.json"))?;
    let receipts = audit["observations"]
        .as_array()
        .context("feature observations")?
        .iter()
        .map(|row| -> Result<_> {
            let source = fs::read_to_string(row["source"].as_str().context("source path")?)?;
            Ok(HistoricalFeatureReceipt {
                id: row["id"].as_str().context("feature ID")?.into(),
                source_sha256: row["responseSha256"]
                    .as_str()
                    .context("source hash")?
                    .into(),
                source_response: source,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let feature_set = HistoricalFeatureSetEvidence::new(409942000, receipts)?;
    feature_set.validate()?;
    let target = read_json("docs/examples/phase-u13-drift-witness/raw/target-transaction.json")?;
    let frozen = FrozenV0::from_rpc(&target, GENESIS)?;
    let normalized = transactions::normalize(&target)?;
    let message = ResolvedMessage {
        message: VersionedMessage::V0(frozen.native_message().clone()),
        account_keys: frozen
            .native_message()
            .account_keys
            .iter()
            .map(ToString::to_string)
            .collect(),
        transaction: normalized,
    };
    let mut seeds = BTreeMap::new();
    for key in &message.account_keys {
        if key == "ComputeBudget111111111111111111111111111111" {
            continue;
        }
        let file = if key == "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH" {
            format!("{WITNESS}/409941999-program.json")
        } else {
            format!("{WITNESS}/409941999-{key}.json")
        };
        seeds.insert(key.clone(), account(&read_json(file)?)?);
    }
    let header = account(&read_json(format!(
        "{WITNESS}/409941999-programdata-header.json"
    ))?)?;
    let elf = fs::read(format!("{U13A}/historical-drift.so"))?;
    ensure!(
        sha(&elf) == "56bb3c1218ca4cc1158116c008439a48ca942f7e2898227494ce2769a7fdce5d",
        "historical ELF hash"
    );
    let mut programdata = header;
    programdata.data.truncate(45);
    programdata.data.extend(elf);
    ensure!(
        sha(&programdata.data)
            == "ca89b99a4ce9cd09a35e7fd881b09099397543a9a1122e93f9a68b1349007728",
        "historical ProgramData hash"
    );
    seeds.insert(PROGRAM_DATA.into(), programdata);
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
            account(&read_json(format!("{U13A}/raw/sysvar-{name}.json"))?)?,
        );
    }
    let historical = HistoricalRuntimeEvidence::new(
        "EK66pFxagY8mA6VjRmNnBqhZwcQ44jFztfNVfueaDWgM".into(),
        RuntimeProfile::sysvar_snapshot_hash(&sysvars)?,
        "LiteSVM 0.16.0 historical evidence".into(),
        "agave-4.2.2-native-system-compute".into(),
        "U13.2B exact-slot account archive receipts".into(),
    )?
    .with_historical_feature_set(feature_set)?;
    let profile = RuntimeProfile::resolve(
        Some(&historical),
        &sysvars,
        "LiteSVM 0.16.0 historical evidence",
        false,
        false,
        "runtime_generated_from_complete_message",
        "materiality_checked_default",
    )?;
    let watched = vec![USER.into(), PERP.into(), SPOT.into()];
    let absent = vec!["ComputeBudget111111111111111111111111111111".into()];
    let run = |p: &RuntimeProfile, slot, recent| {
        LiteSvmBackend.execute(&ExecutionRequest {
            message: &message,
            seeds: &seeds,
            absent_pre_accounts: &absent,
            watched: &watched,
            runtime_sysvars: &sysvars,
            clock: None,
            programs_to_load: &[],
            runtime_profile: p,
            unlimited_logs: true,
            slot_hashes: slot,
            recent_blockhashes: recent,
            require_complete_state: true,
        })
    };
    let base_slot = SlotHashesVariant::BackendDefault;
    let base_recent = RecentBlockhashesVariant::BackendDefault;
    let result = run(&profile, base_slot, base_recent)?;
    let empty_slots = run(&profile, SlotHashesVariant::Empty, base_recent)?;
    let different_slots = run(&profile, SlotHashesVariant::Different, base_recent)?;
    let empty_recent = run(&profile, base_slot, RecentBlockhashesVariant::Empty)?;
    let different_recent = run(&profile, base_slot, RecentBlockhashesVariant::Different)?;
    let named_evidence = HistoricalRuntimeEvidence::new(
        historical.environment_blockhash.clone(),
        historical.sysvar_snapshot_hash.clone(),
        "LiteSVM 0.16.0 mainnet".into(),
        historical.native_program_profile.clone(),
        "U13.2B current-mainnet control".into(),
    )?;
    let default_profile = RuntimeProfile::resolve(
        Some(&named_evidence),
        &sysvars,
        "LiteSVM 0.16.0 mainnet",
        false,
        false,
        "runtime_generated_from_complete_message",
        "materiality_checked_default",
    )?;
    let default_features = run(&default_profile, base_slot, base_recent)?;
    let mut future_set = historical
        .historical_feature_set
        .as_ref()
        .unwrap()
        .feature_set()?;
    let future_rows = audit["backendPostTargetFeatures"]
        .as_array()
        .context("post-target list")?;
    ensure!(future_rows.len() == 26, "post-target count");
    for row in future_rows {
        future_set.activate(
            &row["id"].as_str().context("future ID")?.parse()?,
            row["activationSlot"]
                .as_u64()
                .context("future activation")?,
        );
    }
    let target_plus_future = LiteSvmBackend.execute_diagnostic_feature_set(
        &ExecutionRequest {
            message: &message,
            seeds: &seeds,
            absent_pre_accounts: &absent,
            watched: &watched,
            runtime_sysvars: &sysvars,
            clock: None,
            programs_to_load: &[],
            runtime_profile: &profile,
            unlimited_logs: true,
            slot_hashes: base_slot,
            recent_blockhashes: base_recent,
            require_complete_state: true,
        },
        &future_set,
    )?;
    let changed_environment = HistoricalRuntimeEvidence::new(
        solana_hash::Hash::new_from_array([29; 32]).to_string(),
        historical.sysvar_snapshot_hash.clone(),
        historical.feature_profile.clone(),
        historical.native_program_profile.clone(),
        "U13.2B environment control".into(),
    )?
    .with_historical_feature_set(historical.historical_feature_set.as_ref().unwrap().clone())?;
    let changed_profile = RuntimeProfile::resolve(
        Some(&changed_environment),
        &sysvars,
        "LiteSVM 0.16.0 historical evidence",
        false,
        false,
        "runtime_generated_from_complete_message",
        "materiality_checked_default",
    )?;
    let different_environment = run(&changed_profile, base_slot, base_recent)?;
    let outputs = watched
        .iter()
        .map(|key| {
            let account = result.post_accounts[key]
                .as_ref()
                .context("watched account absent")?;
            Ok((
                key.clone(),
                json!({ "lamports": account.lamports, "dataSha256": sha(&account.data) }),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let safe_match = [USER, PERP]
        .into_iter()
        .map(|key| -> Result<_> {
            let expected = account(&read_json(format!("{WITNESS}/409942000-{key}.json"))?)?;
            let actual = result.post_accounts[key]
                .as_ref()
                .context("safe output absent")?;
            Ok((
                key.to_string(),
                json!({"fullAccountEqual": actual == &expected,
            "actualDataSha256": sha(&actual.data), "expectedDataSha256": sha(&expected.data)}),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let report = json!({
        "schema": "U13_2BTargetFeasibilityV1", "classification": "feasibility_only",
        "featureInventoryId": profile.historical_feature_set.as_ref().unwrap().inventory_id,
        "featureSetHash": profile.historical_feature_set.as_ref().unwrap().feature_set_hash,
        "runtimeProfileId": profile.profile_id,
        "success": result.success, "error": result.error,
        "fee": result.fee, "computeUnits": result.compute_units,
        "logs": result.logs, "innerInstructions": result.inner_instructions,
        "returnData": result.return_data, "outputs": outputs, "safeOutputComparison": safe_match,
        "controls": {
            "slotHashesEmptyEqual": result == empty_slots,
            "slotHashesDifferentEqual": result == different_slots,
            "recentBlockhashesEmptyEqual": result == empty_recent,
            "recentBlockhashesDifferentEqual": result == different_recent,
            "environmentBlockhashDifferentEqual": result == different_environment,
            "currentDefaultEqual": result == default_features,
            "targetPlus26Equal": result == target_plus_future,
            "currentDefault": fingerprint(&default_features),
            "targetPlus26": fingerprint(&target_plus_future),
            "differentEnvironment": fingerprint(&different_environment),
            "emptyRecentBlockhashes": fingerprint(&empty_recent),
            "differentRecentBlockhashes": fingerprint(&different_recent),
        },
        "validator": { "success": target["meta"]["err"].is_null(),
            "fee": target["meta"]["fee"], "computeUnits": target["meta"]["computeUnitsConsumed"],
            "innerInstructionRows": target["meta"]["innerInstructions"].as_array().map_or(0, Vec::len),
            "executedInnerInstructions": result.inner_instructions.iter().map(|g| g.instructions.len()).sum::<usize>(),
            "returnDataPresent": !target["meta"]["returnData"].is_null(),
            "returnDataBytes": result.return_data.data.len(),
            "logsEqual": result.logs == target["meta"]["logMessages"].as_array().unwrap()
                .iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>() }
    });
    fs::write(
        format!("{ROOT}/feasibility.json"),
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    println!(
        "{}",
        serde_json::to_string(&json!({"success": report["success"],
        "error": report["error"], "fee": report["fee"], "computeUnits": report["computeUnits"],
        "outputs": report["outputs"]}))?
    );
    Ok(())
}
