//! Read-only migration state capture and offline observed-world reconstruction.
//!
//! Holders and their owner accounts come from the existing bounded population
//! capture (`getProgramAccounts` over the source token program plus owner batches).
//! This module adds finalized `getMultipleAccounts` batches for the destination mint,
//! both token programs and the ATA program with their ProgramData, the observed
//! reserve or proposed overlay addresses (to prove non-collision), holders'
//! canonical destination ATAs and, last, the Clock. Every request carries
//! `minContextSlot` from the previous response. Nothing is written or signed.
//!
//! Reconstruction is offline: every world account is the exact bytes at a recorded
//! record and JSON pointer, addresses returned as null are recorded as inspected and
//! absent, and the world is labelled a composite of finalized observations.
use super::{
    adapter::Overlay,
    spec::{MigrationAuthority, TokenMigrationV1},
    world::{
        associated_token_address, snapshot_from_rpc, PopulationIndex, World, WorldAccount,
        WorldClock, WorldKind, WorldOrigin, MAINNET_GENESIS,
    },
};
use crate::{
    ingest::{observation::Observation, rpc::RpcProvider as SolanaRpc},
    migration::{population, population_types::StressBudget},
    standard_programs::token as decode,
    standard_programs::token::{ATA_PROGRAM, CLOCK, UPGRADEABLE_LOADER},
};
use anyhow::{bail, ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const CAPTURE_KIND: &str = "token-migration-capture";
pub const CAPTURE_SCHEMA: u32 = 1;
pub const BATCH: usize = 100;
pub const POPULATION_ARTIFACT: &str = "population.capture.json";
pub const MIGRATION_ARTIFACT: &str = "migration.capture.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub schema_version: u32,
    pub kind: String,
    pub run_id: String,
    pub source_mint: String,
    pub destination_mint: String,
    pub rpc_origin: String,
    pub started_at: String,
    pub completed_at: String,
    pub observations: Vec<Observation>,
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn config(slot: u64) -> Value {
    json!({"encoding": "base64", "commitment": "finalized", "minContextSlot": slot})
}

fn record(
    rpc: &impl SolanaRpc,
    out: &mut Vec<Observation>,
    method: &str,
    params: Value,
) -> Result<Value> {
    let started_at = now();
    let response = rpc.call(method, params.clone());
    let completed_at = now();
    let (result, error) = match &response {
        Ok(value) => (Some(value.clone()), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    out.push(Observation {
        method: method.into(),
        params,
        started_at,
        completed_at,
        result,
        error,
    });
    response.with_context(|| format!("{method} failed"))
}

fn slot(value: &Value) -> Result<u64> {
    value["context"]["slot"]
        .as_u64()
        .context("missing RPC response context slot")
}

/// Addresses whose state the migration needs besides holders and owners.
pub fn identity_addresses(spec: &TokenMigrationV1, overlay: &Overlay) -> Vec<String> {
    let mut set: BTreeSet<String> = [
        spec.source.mint.clone(),
        spec.destination.mint.clone(),
        spec.source.token_program.clone(),
        spec.destination.token_program.clone(),
        ATA_PROGRAM.to_string(),
        overlay.config.clone(),
        overlay.program_id.clone(),
    ]
    .into();
    set.extend(overlay.reserve_vault.clone());
    set.extend(overlay.escrow_vault.clone());
    if let MigrationAuthority::External { address } = &spec.authorities.migration_authority {
        set.insert(address.clone());
    }
    set.into_iter().collect()
}

/// Canonical destination ATAs of the positive-balance holders, sorted, bounded.
pub fn destination_addresses(
    spec: &TokenMigrationV1,
    observation: &population::PopulationObservation,
    max_holders: usize,
) -> Result<(Vec<String>, usize)> {
    let owners: BTreeSet<String> = observation
        .positive_entities()
        .map(|e| e.authority.clone())
        .collect();
    let total = owners.len();
    let mut addresses = owners
        .into_iter()
        .take(max_holders)
        .map(|owner| {
            associated_token_address(
                &owner,
                &spec.destination.token_program,
                &spec.destination.mint,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    addresses.sort();
    addresses.dedup();
    Ok((addresses, total))
}

pub fn capture(
    spec: &TokenMigrationV1,
    overlay: &Overlay,
    observation: &population::PopulationObservation,
    max_holders: usize,
    run_id: &str,
    rpc: &impl SolanaRpc,
) -> Result<Capture> {
    let mut c = Capture {
        schema_version: CAPTURE_SCHEMA,
        kind: CAPTURE_KIND.into(),
        run_id: run_id.into(),
        source_mint: spec.source.mint.clone(),
        destination_mint: spec.destination.mint.clone(),
        rpc_origin: "configured-read-only-provider".into(),
        started_at: now(),
        completed_at: String::new(),
        observations: vec![],
    };
    let genesis = record(rpc, &mut c.observations, "getGenesisHash", json!([]))?;
    ensure!(
        genesis.as_str() == Some(MAINNET_GENESIS),
        "the RPC is not Solana mainnet"
    );
    let floor = observation.enumeration.enumeration_slot.unwrap_or_default();
    let identities = identity_addresses(spec, overlay);
    let first = record(
        rpc,
        &mut c.observations,
        "getMultipleAccounts",
        json!([identities, config(floor)]),
    )?;
    let mut min_slot = slot(&first)?;
    let values = first["value"].as_array().context("missing accounts")?;
    let mut programdata = BTreeSet::new();
    for value in values {
        if value["executable"] == true && value["owner"] == UPGRADEABLE_LOADER {
            let bytes = decode::raw_account_bytes(value)?;
            programdata.insert(
                crate::standard_programs::upgradeable_loader::decode_program(&bytes)?.to_string(),
            );
        }
    }
    if !programdata.is_empty() {
        let list: Vec<String> = programdata.into_iter().collect();
        let response = record(
            rpc,
            &mut c.observations,
            "getMultipleAccounts",
            json!([list, config(min_slot)]),
        )?;
        min_slot = slot(&response)?;
    }
    let (destinations, _) = destination_addresses(spec, observation, max_holders)?;
    for chunk in destinations.chunks(BATCH) {
        eprintln!("CURRENT_STAGE:Inspecting destination accounts");
        let response = record(
            rpc,
            &mut c.observations,
            "getMultipleAccounts",
            json!([chunk, config(min_slot)]),
        )?;
        min_slot = slot(&response)?;
    }
    record(
        rpc,
        &mut c.observations,
        "getMultipleAccounts",
        json!([[CLOCK], config(min_slot)]),
    )?;
    c.completed_at = now();
    Ok(c)
}

/// Accumulates exact observed accounts from capture records; the latest finalized
/// observation of an address wins and requested nulls are recorded as absent.
pub(crate) struct Collector {
    pub accounts: BTreeMap<String, (WorldAccount, u64)>,
    pub absent: BTreeSet<String>,
    pub slots: Vec<u64>,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            accounts: BTreeMap::new(),
            absent: BTreeSet::new(),
            slots: vec![],
        }
    }
    pub fn observe(
        &mut self,
        address: &str,
        raw: &Value,
        artifact: &str,
        record: usize,
        pointer: String,
        slot: u64,
    ) -> Result<()> {
        self.slots.push(slot);
        if raw.is_null() {
            if !self.accounts.contains_key(address) {
                self.absent.insert(address.to_string());
            }
            return Ok(());
        }
        self.absent.remove(address);
        let account = WorldAccount {
            account: snapshot_from_rpc(raw)?,
            origin: WorldOrigin::Observed {
                artifact: artifact.into(),
                record,
                pointer,
                slot,
            },
        };
        match self.accounts.get(address) {
            // The later finalized observation of the same address wins.
            Some((_, earlier)) if *earlier > slot => {}
            _ => {
                self.accounts.insert(address.to_string(), (account, slot));
            }
        }
        Ok(())
    }
    pub fn batch(
        &mut self,
        observation: &Observation,
        artifact: &str,
        record: usize,
    ) -> Result<()> {
        self.batch_at(observation, artifact, record, "")
    }
    /// As `batch`, with a JSON-pointer prefix for records nested in a bundle.
    pub fn batch_at(
        &mut self,
        observation: &Observation,
        artifact: &str,
        record: usize,
        prefix: &str,
    ) -> Result<()> {
        if observation.method != "getMultipleAccounts" {
            return Ok(());
        }
        let Some(result) = &observation.result else {
            return Ok(());
        };
        let addresses = observation.params[0]
            .as_array()
            .context("batch without addresses")?;
        let values = result["value"].as_array().context("batch without values")?;
        ensure!(addresses.len() == values.len(), "incomplete account batch");
        let context = slot(result)?;
        for (index, (address, value)) in addresses.iter().zip(values).enumerate() {
            self.observe(
                address.as_str().context("address")?,
                value,
                artifact,
                record,
                format!("{prefix}/result/value/{index}"),
                context,
            )?;
        }
        Ok(())
    }
}

/// Rebuild the observed world from the two exact capture artifacts, offline.
pub fn world(
    spec: &TokenMigrationV1,
    population_bytes: &[u8],
    migration_bytes: &[u8],
    budget: &StressBudget,
) -> Result<World> {
    let observation = population::evaluate_bytes(population_bytes, budget)?;
    ensure!(
        observation.mint == spec.source.mint,
        "the population capture is for a different source mint"
    );
    let population_capture: population::Capture = serde_json::from_slice(population_bytes)?;
    let migration: Capture = serde_json::from_slice(migration_bytes)?;
    ensure!(
        migration.schema_version == CAPTURE_SCHEMA
            && migration.kind == CAPTURE_KIND
            && migration.source_mint == spec.source.mint
            && migration.destination_mint == spec.destination.mint,
        "the migration capture does not match the specification"
    );
    ensure!(
        migration
            .observations
            .first()
            .and_then(|o| o.result.as_ref())
            .and_then(Value::as_str)
            == Some(MAINNET_GENESIS),
        "the migration capture is not from Solana mainnet"
    );
    for observation in &migration.observations {
        ensure!(
            observation.result.is_some() != observation.error.is_some(),
            "a capture record needs exactly one of result or error"
        );
        if let Some(error) = &observation.error {
            bail!("the migration capture recorded a provider failure: {error}");
        }
    }
    let mut c = Collector::new();
    for (record, o) in population_capture.observations.iter().enumerate() {
        let Some(result) = &o.result else { continue };
        match o.method.as_str() {
            "getAccountInfo" => {
                let address = o.params[0].as_str().context("address")?;
                c.observe(
                    address,
                    &result["value"],
                    POPULATION_ARTIFACT,
                    record,
                    "/result/value".into(),
                    slot(result)?,
                )?;
            }
            "getProgramAccounts" => {
                let context = slot(result)?;
                for (index, row) in result["value"].as_array().into_iter().flatten().enumerate() {
                    let address = row["pubkey"].as_str().context("row pubkey")?;
                    c.observe(
                        address,
                        &row["account"],
                        POPULATION_ARTIFACT,
                        record,
                        format!("/result/value/{index}/account"),
                        context,
                    )?;
                }
            }
            _ => c.batch(o, POPULATION_ARTIFACT, record)?,
        }
    }
    for (record, o) in migration.observations.iter().enumerate() {
        c.batch(o, MIGRATION_ARTIFACT, record)?;
    }
    let clock_record = migration
        .observations
        .last()
        .context("empty migration capture")?;
    ensure!(
        clock_record.params[0] == json!([CLOCK]),
        "the migration capture must end with the Clock"
    );
    let clock_raw = &clock_record.result.as_ref().context("Clock result")?["value"][0];
    let clock = WorldClock::from_bytes(&decode::raw_account_bytes(clock_raw)?)?;
    let mut token_accounts: Vec<String> = observation
        .entities
        .iter()
        .map(|e| e.token_account.clone())
        .collect();
    token_accounts.sort();
    let min = c.slots.iter().min().copied().unwrap_or(clock.slot);
    let max = c.slots.iter().max().copied().unwrap_or(clock.slot);
    let accounts: BTreeMap<String, WorldAccount> =
        c.accounts.into_iter().map(|(k, (a, _))| (k, a)).collect();
    let world = World {
        kind: WorldKind::ObservedCapture,
        cluster: "solana-mainnet".into(),
        genesis_hash: MAINNET_GENESIS.into(),
        clock,
        observed_slots: Some((min, max)),
        inspected_absent: c.absent.into_iter().filter(|a| !accounts.contains_key(a)).collect(),
        accounts,
        population: PopulationIndex {
            source_mint: spec.source.mint.clone(),
            token_accounts,
            enumeration_completeness: observation.enumeration.completeness.key().into(),
            authority_resolution_completeness: observation.authority_resolution.completeness.key().into(),
            undecoded_accounts: observation.undecoded.iter().map(|u| u.address.clone()).collect(),
        },
        limitations: vec![
            format!("Composite of finalized read-only observations between slots {min} and {max}; not a historical validator bank."),
            "Accounts absent from every request are unknown, not absent; only requested null responses count as absent.".into(),
        ],
        derived_from: None,
    };
    world.validate()?;
    Ok(world)
}
