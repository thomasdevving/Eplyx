//! Bounded current DLMM discovery. Select one layout-compatible route deterministically;
//! verify its complete current fixture without replacing failed selected routes.
use super::{
    current::{exact_amount, CheckRequest},
    meteora_dlmm as adapter,
    token_transfer::TransferContext,
    CapturedExecutionFixture,
};
use crate::lifecycle::{decode, exposure::meteora_dlmm as dlmm, rpc::SolanaRpc, RpcEvidence};
use anyhow::{ensure, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};
fn config(slot: u64) -> Value {
    json!({"encoding":"base64","commitment":"finalized","minContextSlot":slot})
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
pub struct Prepared {
    pub parameters: adapter::SwapParameters,
    pub fixture: CapturedExecutionFixture,
    pub minimum_slot: u64,
    pub discovery: Value,
}
pub enum Preparation {
    NoRoute(Value),
    Ready(Box<Prepared>),
}
pub fn prepare(
    c: &TransferContext,
    amount: u64,
    request: &CheckRequest,
    rpc: &impl SolanaRpc,
) -> Result<Preparation> {
    ensure!(
        c.program == decode::TOKEN_2022_PROGRAM,
        "current DLMM adapter requires Token-2022 input"
    );
    ensure!(
        rpc.call("getGenesisHash", json!([]))?.as_str() == Some(&c.genesis_hash),
        "wrong discovery chain"
    );
    eprintln!("CURRENT_STAGE:Discovering a supported current route");
    // One X-side scan only. Other directions, venues and larger/extended routes remain unexamined.
    let mut cfg = config(c.minimum_slot);
    cfg["withContext"] = true.into();
    cfg["filters"] = json!([{"dataSize":904},{"memcmp":{"offset":88,"bytes":c.mint}}]);
    let scan = rpc.call("getProgramAccounts", json!([dlmm::PROGRAM_ID, cfg]))?;
    let scan_slot = adapter::slot(&scan)?;
    ensure!(
        scan_slot >= c.minimum_slot,
        "discovery predates wallet observation"
    );
    let rows = scan["value"]
        .as_array()
        .context("missing bounded route scan")?;
    ensure!(rows.len() <= 64, "route scan exceeds 64 candidate budget");
    let output = request
        .output_mint
        .as_ref()
        .context("paired mint required")?;
    let mut compatible = std::collections::BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for row in rows {
        let pool = row["pubkey"].as_str().context("missing pool address")?;
        ensure!(seen.insert(pool), "duplicate discovery pool");
        let parameters = adapter::SwapParameters {
            pool: pool.into(),
            input_mint: c.mint.clone(),
            output_mint: output.clone(),
            input_amount_raw: amount.to_string(),
            minimum_output_raw: String::new(),
            amount_reason: String::new(),
        };
        if adapter::route_current(&parameters, &c.source, &c.owner, &row["account"]).is_ok() {
            compatible.insert(pool.to_string(), parameters);
        }
    }
    let mut discovery = json!({"program":dlmm::PROGRAM_ID,"scan_slot":scan_slot.to_string(),"maximum_candidates":64,"observed_candidates":rows.len(),"compatible_layout_candidates":compatible.len(),"selection_rule":"Lexicographically first layout-compatible X-to-Y pool; final dependencies verified for this one candidate. No substitution after selection.","scope":"One bounded Meteora DLMM PermissionlessV2 version 1 scan for selected mint as Token-2022 X and requested legacy Y, at most four internal-bitmap bin arrays. Not a global market search or best-route ranking.","global_exitability_established":false});
    let Some((_, mut parameters)) = compatible.pop_first() else {
        return Ok(Preparation::NoRoute(discovery));
    };
    discovery["selected_pool"] = parameters.pool.clone().into();
    let output_response = rpc.call("getAccountInfo", json!([output, config(scan_slot)]))?;
    let output_slot = adapter::slot(&output_response)?;
    ensure!(output_slot >= scan_slot, "old paired mint context");
    let output_config = decode::decode_mint(&output_response["value"])?;
    ensure!(
        output_config.token_program == decode::LEGACY_PROGRAM,
        "unsupported paired mint program"
    );
    parameters.minimum_output_raw = exact_amount(
        request
            .minimum_output_decimal
            .as_deref()
            .context("explicit minimum output required")?,
        output_config.decimals,
    ).map_err(|e|anyhow::anyhow!("InvalidRequest: minimum output must use the captured paired mint precision ({} decimals): {e}",output_config.decimals))?
    .to_string();
    parameters.amount_reason="Exact user-selected base token input and explicit minimum paired-token output for this local test; no quote or best execution guarantee.".into();
    discovery["output_decimals"] = output_config.decimals.into();
    eprintln!("CURRENT_STAGE:Preparing current state");
    let mut evidence = vec![];
    let genesis = record(rpc, &mut evidence, "getGenesisHash", json!([]))?;
    ensure!(
        genesis.as_str() == Some(&c.genesis_hash),
        "wrong execution chain"
    );
    let initial = record(
        rpc,
        &mut evidence,
        "getAccountInfo",
        json!([parameters.pool, config(output_slot)]),
    )?;
    let route = adapter::route_current(&parameters, &c.source, &c.owner, &initial["value"])?;
    let initial_slot = adapter::slot(&initial)?;
    ensure!(initial_slot >= output_slot, "old initial pool");
    let headers = record(
        rpc,
        &mut evidence,
        "getMultipleAccounts",
        json!([adapter::program_ids(), config(initial_slot)]),
    )?;
    let pd = headers["value"]
        .as_array()
        .context("missing program headers")?
        .iter()
        .map(adapter::programdata_address)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    record(
        rpc,
        &mut evidence,
        "getMultipleAccounts",
        json!([
            adapter::account_plan(&route, &pd),
            config(adapter::slot(&headers)?)
        ]),
    )?;
    Ok(Preparation::Ready(Box::new(Prepared {
        parameters,
        minimum_slot: output_slot,
        discovery,
        fixture: CapturedExecutionFixture {
            schema_version: 1,
            decoder_revision: dlmm::SDK_REVISION.into(),
            captured_at: Utc::now(),
            rpc_origin: "configured_read_only_provider".into(),
            evidence,
        },
    })))
}
