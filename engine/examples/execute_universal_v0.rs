//! Offline equivalence probe for the generic execution backend. This consumes
//! the retained U3F input shape but has no protocol-specific admission logic.
use std::{collections::BTreeMap, io::Read};

use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use eplyx_engine::{
    ingest::accounts,
    message::{ArchiveProvenance, LutResolutionProof},
    replay::hash_bytes,
    types::AccountSnapshot,
    universal::{
        evidence::{AccountBoundary, AccountObservation, EvidenceKind, EvidenceStore},
        execution::{
            ExecutionBackend, ExecutionRequest, LiteSvmBackend, RuntimeProfile, SlotHashesVariant,
        },
        model::ExecutionInput,
    },
};
use serde_json::{json, Value};
use solana_address_lookup_table_interface::state::AddressLookupTable;

fn snapshot(account: &AccountSnapshot) -> Value {
    json!({"lamports":account.lamports,"owner":account.owner,
        "executable":account.executable,"rentEpoch":account.rent_epoch,
        "data":[BASE64_STANDARD.encode(&account.data),"base64"],
        "data_sha256":hash_bytes(&account.data)})
}

fn run(input: &Value) -> Result<Value> {
    let raw = &input["frozen"];
    let genesis = raw["genesis"].as_str().context("genesis hash")?;
    let transaction = &raw["result"];
    let slot = transaction["slot"].as_u64().context("transaction slot")?;
    let proof: LutResolutionProof = serde_json::from_value(input["proof"].clone())?;
    let scratch =
        std::env::temp_dir().join(format!("eplyx-universal-probe-{}", std::process::id()));
    ensure!(!scratch.exists(), "probe scratch directory already exists");
    std::fs::create_dir_all(&scratch)?;
    let store = EvidenceStore::at(&scratch);
    let frozen_transaction =
        store.put(EvidenceKind::Transaction, &serde_json::to_vec(transaction)?)?;
    let lookup_tables = raw["evidence"]
        .as_array()
        .context("LUT evidence")?
        .iter()
        .map(|item| {
            let address = item["pubkey"].as_str().context("LUT address")?;
            let provider: ArchiveProvenance = serde_json::from_value(item["provider"].clone())?;
            let response = BASE64_STANDARD.decode(
                item["raw_response_base64"]
                    .as_str()
                    .context("LUT response")?,
            )?;
            AccountObservation::capture(
                &store,
                address,
                slot,
                AccountBoundary::EndOfExecutionSlot,
                provider,
                &response,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let execution_input = ExecutionInput::V0 {
        message: proof.native_v0_message.clone(),
        frozen_transaction,
        lookup_tables,
        slot_hashes: None,
        claimed_proof: proof.clone(),
    };
    let resolved = execution_input.resolve(&store, genesis)?;
    let mut seeds = BTreeMap::new();
    let mut runtime_sysvars = BTreeMap::new();
    for item in input["seeds"]
        .as_array()
        .context("historical account seeds")?
    {
        let address = item["address"]
            .as_str()
            .context("seed address")?
            .to_string();
        let account = accounts::normalize(&item["account"])?;
        ensure!(
            item["data_sha256"].as_str() == Some(hash_bytes(&account.data).as_str()),
            "seed data hash differs"
        );
        if item["kind"] == "runtime" {
            runtime_sysvars.insert(address.clone(), account.clone());
        }
        ensure!(
            seeds.insert(address, account).is_none(),
            "duplicate seed address"
        );
    }
    let watched = input["watch"]
        .as_array()
        .context("watch list")?
        .iter()
        .map(|value| value.as_str().context("watch address").map(str::to_string))
        .collect::<Result<Vec<_>>>()?;
    let absent = input["absent"]
        .as_array()
        .context("absence proof")?
        .iter()
        .map(|value| value.as_str().context("absent address").map(str::to_string))
        .collect::<Result<Vec<_>>>()?;
    let runtime_profile = RuntimeProfile::resolve(
        None,
        &runtime_sysvars,
        "LiteSVM 0.16.0 mainnet",
        false,
        false,
        "runtime_generated_from_complete_message",
        "materiality_checked_default",
    )?;
    let output = LiteSvmBackend.execute(&ExecutionRequest {
        message: &resolved,
        seeds: &seeds,
        absent_pre_accounts: &absent,
        watched: &watched,
        runtime_sysvars: &runtime_sysvars,
        clock: None,
        programs_to_load: &[],
        runtime_profile: &runtime_profile,
        unlimited_logs: true,
        slot_hashes: SlotHashesVariant::BackendDefault,
        recent_blockhashes: Default::default(),
        require_complete_state: true,
    })?;
    let tables = proof
        .native_v0_message
        .address_table_lookups
        .iter()
        .map(|lookup| {
            let account = seeds
                .get(&lookup.account_key.to_string())
                .context("LUT account not seeded")?;
            let table = AddressLookupTable::deserialize(&account.data)?;
            Ok(json!({"address":lookup.account_key.to_string(),
            "deactivation_slot":table.meta.deactivation_slot,
            "last_extended_slot":table.meta.last_extended_slot}))
        })
        .collect::<Result<Vec<_>>>()?;
    let inner = output.inner_instructions.iter().map(|group| json!({"outer_index":group.outer_index,
        "instructions":group.instructions.iter().map(|ix| json!({"program_id_index":ix.program_id_index,
            "accounts":ix.accounts,"data_hex":eplyx_engine::hexfmt::encode(&ix.data),
            "stack_height":ix.stack_height})).collect::<Vec<_>>() })).collect::<Vec<_>>();
    let post = output
        .post_accounts
        .iter()
        .map(|(address, account)| {
            (
                address.clone(),
                account.as_ref().map(snapshot).unwrap_or(Value::Null),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let result = json!({"evidence":{"proof_id":proof.proof_id,
        "native_message":proof.native_v0_message,
        "account_vector":resolved.account_keys,"tables":tables,
        "success":output.success,"error":output.error,"compute_units":output.compute_units,
        "fee":output.fee,"logs":output.logs,"inner_instructions":inner,
        "return_data":{"program":output.return_data.program,
            "data_base64":BASE64_STANDARD.encode(output.return_data.data)},
        "post_accounts":post},"original_instructions_executed":true});
    std::fs::remove_dir_all(&scratch)?;
    Ok(result)
}

fn main() -> Result<()> {
    let mut bytes = String::new();
    std::io::stdin().read_to_string(&mut bytes)?;
    let input: Value = serde_json::from_str(&bytes)?;
    println!("{}", run(&input)?);
    Ok(())
}
