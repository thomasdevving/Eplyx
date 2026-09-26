//! Read-only capture of a bounded execution fixture. Replay never calls this module.
use super::{
    meteora_dlmm::{self as adapter},
    CapturedExecutionFixture, ExecutionProbeSpec, ExitPathType, ProbeAdapter,
};
use crate::lifecycle::{
    exposure::{meteora_dlmm::SDK_REVISION, sha256},
    policy::LifecycleScenario,
    rpc::SolanaRpc,
    EntityType, LifecycleSnapshot, RpcEvidence,
};
use anyhow::{ensure, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};

fn config(min: u64) -> Value {
    json!({"encoding":"base64","commitment":"finalized","minContextSlot":min})
}
fn record(
    rpc: &impl SolanaRpc,
    e: &mut Vec<RpcEvidence>,
    method: &str,
    params: Value,
) -> Result<Value> {
    let result = rpc.call(method, params.clone())?;
    e.push(RpcEvidence {
        id: e.len(),
        method: method.into(),
        params,
        result: result.clone(),
    });
    Ok(result)
}

pub fn capture(
    snapshot: &LifecycleSnapshot,
    scenario: &LifecycleScenario,
    target: Option<&str>,
    amount: u64,
    fixture_reference: String,
    rpc: &impl SolanaRpc,
) -> Result<(ExecutionProbeSpec, CapturedExecutionFixture)> {
    capture_at(
        snapshot,
        scenario,
        target,
        amount,
        fixture_reference,
        None,
        rpc,
    )
}

pub fn capture_at(
    snapshot: &LifecycleSnapshot,
    scenario: &LifecycleScenario,
    target: Option<&str>,
    amount: u64,
    fixture_reference: String,
    pool: Option<&str>,
    rpc: &impl SolanaRpc,
) -> Result<(ExecutionProbeSpec, CapturedExecutionFixture)> {
    snapshot.validate()?;
    scenario.validate()?;
    let entity = match target {
        Some(id) => snapshot.entities.iter().find(|e| e.id == id),
        None => snapshot.entities.iter().find(|e| {
            e.entity_type == EntityType::WalletCompatible
                && e.state
                    .raw_balance
                    .parse::<u64>()
                    .is_ok_and(|n| n >= amount)
        }),
    }
    .context("no matching positive wallet-compatible source entity")?;
    ensure!(
        entity.entity_type == EntityType::WalletCompatible && amount > 0,
        "source must be wallet-compatible and amount positive"
    );
    let exposures = &snapshot
        .exposures
        .as_ref()
        .context("verified exposure snapshot required")?
        .protocol_exposures;
    let exposure = match pool {
        Some(pool) => exposures
            .iter()
            .find(|e| e.pool_address == pool)
            .context("selected pool not in verified snapshot")?,
        None => {
            ensure!(
                exposures.len() == 1,
                "exactly one previously verified venue is supported without an explicit pool"
            );
            &exposures[0]
        }
    };
    let paired = exposure
        .assets
        .iter()
        .find(|a| a.mint != snapshot.asset.mint)
        .context("missing paired asset")?;
    let mut spec=ExecutionProbeSpec{schema_version:1,id:"dlmm-secondary-market-exit-v1".into(),path_type:ExitPathType::SecondaryMarketExit,adapter:ProbeAdapter::MeteoraDlmm,
        target_entity:entity.id.clone(),pool:exposure.pool_address.clone(),input_mint:snapshot.asset.mint.clone(),output_mint:paired.mint.clone(),
        input_amount_raw:amount.to_string(),minimum_output_raw:"1".into(),amount_reason:format!("A bounded {amount} raw-unit input smaller than the selected captured holder amount, using mint precision. Not the full wallet; no claim of available depth beyond this amount. Minimum output is one raw paired-token unit, a test threshold rather than a production slippage recommendation."),
        snapshot_sha256:sha256(snapshot.to_json()?.as_bytes()),scenario_sha256:scenario.sha256()?,fixture:fixture_reference,fixture_sha256:String::new()};
    let mut evidence = Vec::new();
    let genesis = record(rpc, &mut evidence, "getGenesisHash", json!([]))?;
    ensure!(
        genesis.as_str() == Some(&snapshot.source.genesis_hash),
        "execution RPC on different chain"
    );
    let initial = record(
        rpc,
        &mut evidence,
        "getAccountInfo",
        json!([
            spec.pool,
            config(
                exposures
                    .iter()
                    .map(|p| p.discovered_at_slot)
                    .max()
                    .context("no verified venue")?
            )
        ]),
    )?;
    let route = adapter::route(snapshot, &spec, &initial["value"])?;
    let initial_slot = adapter::slot(&initial)?;
    let headers = record(
        rpc,
        &mut evidence,
        "getMultipleAccounts",
        json!([adapter::program_ids(), config(initial_slot)]),
    )?;
    let programdata: Vec<_> = headers["value"]
        .as_array()
        .context("missing program headers")?
        .iter()
        .map(adapter::programdata_address)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    record(
        rpc,
        &mut evidence,
        "getMultipleAccounts",
        json!([
            adapter::account_plan(&route, &programdata),
            config(adapter::slot(&headers)?)
        ]),
    )?;
    let fixture = CapturedExecutionFixture {
        schema_version: 1,
        decoder_revision: SDK_REVISION.into(),
        captured_at: Utc::now(),
        rpc_origin: "configured_read_only_provider".into(),
        evidence,
    };
    spec.fixture_sha256 = fixture.sha256()?;
    Ok((spec, fixture))
}
