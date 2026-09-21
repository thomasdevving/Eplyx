//! Experimental native-v0 execution of a sealed envelope. No production admission.
use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use eplyx_engine::{
    ingest::accounts,
    message::{self, ArchiveProvenance, FrozenV0, HistoricalAccountEvidence},
    protocol::kamino::envelope,
    replay::hash_bytes,
};
use litesvm::LiteSVM;
use serde_json::{json, Value};
use solana_account::Account;
use solana_address::Address;
use solana_address_lookup_table_interface::state::AddressLookupTable;
use solana_clock::Clock;
use solana_hash::Hash;
use solana_message::VersionedMessage;
use solana_slot_hashes::SlotHashes;
use solana_transaction::versioned::VersionedTransaction;
use std::{collections::BTreeMap, io::Read, time::Instant};

fn account(value: &Value) -> Result<Account> {
    let a = accounts::normalize(value)?;
    Ok(Account {
        lamports: a.lamports,
        data: a.data,
        owner: a.owner.parse()?,
        executable: a.executable,
        rent_epoch: a.rent_epoch,
    })
}
fn snapshot(a: &Account) -> Value {
    json!({"lamports":a.lamports,"owner":a.owner.to_string(),"executable":a.executable,"rentEpoch":a.rent_epoch,
           "data":[BASE64_STANDARD.encode(&a.data),"base64"],"data_sha256":hash_bytes(&a.data)})
}
fn execute(input: &Value, variant: &str) -> Result<Value> {
    let tick = Instant::now();
    let raw = &input["frozen"];
    let frozen = FrozenV0::from_rpc(&raw["result"], raw["genesis"].as_str().context("genesis")?)?;
    let mut evidence = Vec::new();
    for item in raw["evidence"].as_array().context("LUT evidence")? {
        let provider: ArchiveProvenance = serde_json::from_value(item["provider"].clone())?;
        let body = BASE64_STANDARD.decode(
            item["raw_response_base64"]
                .as_str()
                .context("raw LUT response")?,
        )?;
        evidence.push(HistoricalAccountEvidence::from_response(
            item["pubkey"].as_str().context("LUT key")?,
            frozen.execution_slot(),
            provider,
            &body,
        )?);
    }
    let proven = message::reconstruct(&frozen, &evidence, None)
        .map_err(|e| anyhow::anyhow!("LUT proof: {}", e.detail))?;
    ensure!(
        serde_json::to_value(proven.proof())? == input["proof"],
        "sealed historical proof differs"
    );
    let admitted = envelope::admit(&proven).context("envelope admission failed")?;
    admitted.check_instruction_sequence(&proven.transaction().instructions)?;
    let original_message = proven.versioned_message();
    let native = match &original_message {
        VersionedMessage::V0(m) => m,
        _ => anyhow::bail!("native v0 required"),
    };
    ensure!(
        native.instructions.len() == proven.transaction().instructions.len(),
        "complete original envelope required"
    );
    let slot = frozen.execution_slot();
    let mut seeds = BTreeMap::new();
    for item in input["seeds"].as_array().context("seed accounts")? {
        let key: Address = item["address"].as_str().context("seed address")?.parse()?;
        ensure!(!seeds.contains_key(&key), "duplicate seed account");
        let expected_slot = if item["kind"] == "runtime" || item["kind"] == "lut" {
            slot
        } else {
            slot - 1
        };
        ensure!(
            item["slot"].as_u64() == Some(expected_slot),
            "current/intermediate/post state cannot seed ordinary pre-state"
        );
        let value = account(&item["account"])?;
        ensure!(
            item["data_sha256"].as_str() == Some(hash_bytes(&value.data).as_str()),
            "seed bytes/hash differ"
        );
        if let Some(file) = item["provenance"]["file"].as_str() {
            let raw = std::fs::read(file)?;
            ensure!(
                item["provenance"]["sha256"].as_str() == Some(hash_bytes(&raw).as_str()),
                "retained seed response hash differs"
            );
            let raw: Value = serde_json::from_slice(&raw)?;
            ensure!(raw["result"]["context"]["slot"].as_u64() == Some(expected_slot)
                && account(&raw["result"]["value"])? == value,
                "historical seed differs from retained raw response; post/current/intermediate injection rejected");
        }
        if item["kind"] == "lut" {
            let source = evidence
                .iter()
                .find(|e| e.pubkey == key.to_string())
                .context("unproven runtime LUT seed")?;
            let source = source.account(slot, raw["genesis"].as_str().context("genesis")?)?;
            ensure!(
                source.data == value.data
                    && source.owner == value.owner.to_string()
                    && source.lamports == value.lamports
                    && source.executable == value.executable
                    && source.rent_epoch == value.rent_epoch,
                "runtime LUT bytes differ from historical proof"
            );
        }
        seeds.insert(key, value);
    }
    for key in &proven.transaction().account_keys {
        if key.address == "Sysvar1nstructions1111111111111111111111111" {
            continue;
        }
        ensure!(
            seeds.contains_key(&key.address.parse()?)
                || input["absent"]
                    .as_array()
                    .context("absence evidence")?
                    .iter()
                    .any(|a| a.as_str() == Some(&key.address)),
            "required pre-state missing: {}",
            key.address
        );
    }
    for (index, key) in proven.transaction().account_keys.iter().enumerate() {
        if key.address == "Sysvar1nstructions1111111111111111111111111" {
            continue;
        }
        let actual = seeds
            .get(&key.address.parse()?)
            .map(|a| a.lamports)
            .unwrap_or(0);
        ensure!(
            actual
                == proven
                    .transaction()
                    .pre_balances
                    .as_ref()
                    .context("validator pre-balances")?[index],
            "seed lamports contradict original validator boundary"
        );
    }
    for program in input["programs"].as_array().context("binary manifest")? {
        let id: Address = program["program_id"]
            .as_str()
            .context("program id")?
            .parse()?;
        let a = seeds
            .get(&id)
            .context("historical program account missing")?;
        if program["source"] == "builtin" {
            continue;
        }
        let bytes = if let Some(pd) = program["provenance"]["programdata_address"].as_str() {
            let pd: Address = pd.parse()?;
            ensure!(
                a.data.get(4..36) == Some(pd.as_ref()),
                "ProgramData identity differs"
            );
            let data = &seeds.get(&pd).context("ProgramData account missing")?.data;
            ensure!(data.len() >= 45, "truncated ProgramData");
            let deploy = u64::from_le_bytes(data[4..12].try_into()?);
            ensure!(
                deploy < slot
                    && program["provenance"]["deployment_slot"]
                        .as_u64()
                        .or(program["provenance"]["deploy_slot"].as_u64())
                        == Some(deploy),
                "historical deployment context differs"
            );
            &data[45..]
        } else {
            a.data.as_slice()
        };
        ensure!(
            program["provenance"]["sha256"].as_str() == Some(hash_bytes(bytes).as_str()),
            "historical ELF hash differs"
        );
    }
    let mut svm = LiteSVM::new()
        .with_sigverify(false)
        .with_blockhash_check(false)
        .with_log_bytes_limit(None);
    // Preserve historical runtime bytes; set_account refreshes the corresponding sysvar cache.
    for item in input["seeds"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["kind"] == "runtime")
    {
        let key: Address = item["address"].as_str().unwrap().parse()?;
        svm.set_account(key, seeds[&key].clone())
            .map_err(|e| anyhow::anyhow!("historical sysvar {key}: {e:?}"))?;
    }
    let clock = svm.get_sysvar::<Clock>();
    ensure!(
        clock.slot == slot
            && clock.unix_timestamp
                == proven
                    .transaction()
                    .block_time
                    .context("historical block time")?,
        "historical Clock differs"
    );
    match variant {
        "default" => (),
        "empty" => svm.set_sysvar(&SlotHashes::new(&[])),
        "different" => svm.set_sysvar(&SlotHashes::new(&[
            (slot - 1, Hash::new_from_array([17; 32])),
            (slot - 7, Hash::new_from_array([99; 32])),
        ])),
        _ => anyhow::bail!("unknown SlotHashes experiment"),
    }
    let hashes = svm.get_sysvar::<SlotHashes>();
    // ProgramData before executable headers; load_program then uses the captured ELF directly.
    // No synthetic deployment metadata or authority replaces the captured accounts.
    for executable in [false, true] {
        for (key, value) in &seeds {
            if value.executable == executable {
                svm.set_account(*key, value.clone())
                    .map_err(|e| anyhow::anyhow!("seed {key}: {e:?}"))?;
            }
        }
    }
    let mut writable = Vec::new();
    let mut readonly = Vec::new();
    let mut tables = Vec::new();
    for lookup in &native.address_table_lookups {
        let table_account = svm
            .get_account(&lookup.account_key)
            .context("LUT absent from bank")?;
        let table = AddressLookupTable::deserialize(&table_account.data)?;
        ensure!(
            table.meta.deactivation_slot == u64::MAX,
            "minimality experiment restricted to active LUTs"
        );
        writable.extend(table.lookup(slot, &lookup.writable_indexes, &hashes)?);
        readonly.extend(table.lookup(slot, &lookup.readonly_indexes, &hashes)?);
        tables.push(json!({"address":lookup.account_key.to_string(),"deactivation_slot":table.meta.deactivation_slot,"last_extended_slot":table.meta.last_extended_slot}));
    }
    let vector: Vec<String> = native
        .account_keys
        .iter()
        .chain(&writable)
        .chain(&readonly)
        .map(ToString::to_string)
        .collect();
    ensure!(
        vector
            == proven
                .transaction()
                .account_keys
                .iter()
                .map(|k| k.address.clone())
                .collect::<Vec<_>>(),
        "runtime LUT account vector differs from sealed proof"
    );
    let signatures = raw["result"]["transaction"]["signatures"]
        .as_array()
        .context("signatures")?
        .iter()
        .map(|s| {
            s.as_str()
                .context("signature")?
                .parse()
                .map_err(anyhow::Error::from)
        })
        .collect::<Result<Vec<_>>>()?;
    let skip_scope = input["control"] == "skip_scope";
    ensure!(
        input["control"].is_null() || skip_scope,
        "unknown causal intervention"
    );
    let mut submitted_message = original_message.clone();
    if skip_scope {
        // Deliberately nonhistorical negative control. This never changes the
        // sealed original or produces a baseline/production admission claim.
        let VersionedMessage::V0(message) = &mut submitted_message else {
            unreachable!()
        };
        ensure!(
            message.account_keys[message.instructions[0].program_id_index as usize].to_string()
                == "HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ",
            "causal control requires original Scope at index zero"
        );
        message.instructions.remove(0);
        ensure!(
            message.instructions == native.instructions[1..],
            "causal control changed later target/dependency bytes"
        );
    } else {
        ensure!(
            submitted_message == proven.versioned_message(),
            "message changed before execution"
        );
    }
    let seeded_pre_accounts: BTreeMap<_, _> = seeds
        .keys()
        .map(|key| (key.to_string(), svm.get_account(key).as_ref().map(snapshot)))
        .collect();
    let tx = VersionedTransaction {
        signatures,
        message: submitted_message.clone(),
    };
    let preparation_seconds = tick.elapsed().as_secs_f64();
    let tick = Instant::now();
    let (success, error, meta) = match svm.send_transaction(tx) {
        Ok(meta) => (true, None, meta),
        Err(failed) => (false, Some(format!("{:?}", failed.err)), failed.meta),
    };
    let execution_seconds = tick.elapsed().as_secs_f64();
    let mut post = BTreeMap::new();
    for key in input["watch"].as_array().context("watch set")? {
        let key = key.as_str().context("watched key")?;
        post.insert(key, svm.get_account(&key.parse()?).as_ref().map(snapshot));
    }
    let inner: Vec<_> = meta.inner_instructions.iter().enumerate().map(|(outer, group)| json!({"outer_index":outer,"instructions":group.iter().map(|i|json!({"program_id_index":i.instruction.program_id_index,"accounts":i.instruction.accounts,"data_hex":i.instruction.data.iter().map(|b|format!("{b:02x}")).collect::<String>(),"stack_height":i.stack_height})).collect::<Vec<_>>()})).collect();
    let mut output = json!({"variant":variant,"preparation_seconds":preparation_seconds,"execution_seconds":execution_seconds,
       "seeded_pre_accounts":seeded_pre_accounts,
       "slot_hashes":hashes.iter().map(|(slot,hash)|json!([slot,hash.to_string()])).collect::<Vec<_>>(),
       "evidence":{"proof_id":proven.proof().proof_id,"native_message":native,"account_vector":vector,"tables":tables,
       "success":success,"error":error,"compute_units":meta.compute_units_consumed,"fee":meta.fee,"logs":meta.logs,
       "inner_instructions":inner,"return_data":{"program":meta.return_data.program_id.to_string(),"data_base64":BASE64_STANDARD.encode(meta.return_data.data)},"post_accounts":post},
       "original_instructions_executed":!skip_scope,"production_replay_eligible":false});
    if skip_scope {
        output["diagnostic_control"] = json!("skip_scope; not a baseline or production replay");
        output["evidence"]["native_message"] = serde_json::to_value(submitted_message)?;
    }
    Ok(output)
}
fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let input: Value = serde_json::from_str(&input)?;
    let variant = input["variant"].as_str().context("variant")?;
    println!("{}", execute(&input, variant)?);
    Ok(())
}
