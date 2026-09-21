//! Existing U2 interpretation after strict experimental native-v0 fidelity.
use anyhow::{ensure, Context, Result};
use eplyx_engine::{
    executor::ExecutionResult,
    ingest::{accounts, transactions::HistoricalTransaction},
    protocol::{kamino::KaminoKlendAdapter, ProtocolAdapter},
    replay::{outcome_hash, state_hash},
    types::{AccountSnapshot, NamedAccount},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read};
fn snapshot(value: &Value) -> Result<AccountSnapshot> {
    if value.is_null() {
        Ok(AccountSnapshot {
            owner: "11111111111111111111111111111111".into(),
            lamports: 0,
            data: vec![],
            executable: false,
            rent_epoch: 0,
        })
    } else {
        accounts::normalize(value)
    }
}
fn main() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: Value = serde_json::from_str(&raw)?;
    let tx: HistoricalTransaction = serde_json::from_value(input["transaction"].clone())?;
    let adapter = KaminoKlendAdapter;
    let mut pre = Vec::new();
    let mut post = Vec::new();
    let mut expected = Vec::new();
    let watch = input["watch"].as_array().context("watch")?;
    ensure!(
        watch.len() == tx.account_keys.len() - 1,
        "complete message watch set required except runtime Instructions"
    );
    for key in watch {
        let address = key.as_str().context("address")?;
        let index = tx
            .account_keys
            .iter()
            .position(|k| k.address == address)
            .context("unknown account")?;
        let label = adapter.label(&tx, index);
        for (field, target) in [
            ("pre", &mut pre),
            ("post", &mut post),
            ("expected_post", &mut expected),
        ] {
            let value = input[field]
                .get(address)
                .context("required account absent from record")?;
            if value.is_null() {
                ensure!(
                    input["absent"]
                        .as_array()
                        .context("absence evidence")?
                        .iter()
                        .any(|a| a == key),
                    "unproved missing account"
                );
            }
            target.push(NamedAccount {
                label: label.clone(),
                address: address.into(),
                account: snapshot(value)?,
            });
        }
    }
    ensure!(
        post == expected,
        "raw account differences prohibit semantic promotion"
    );
    ensure!(
        input["outcome"]["success"] == true && tx.success,
        "successful faithful baseline required"
    );
    ensure!(
        input["outcome"]["fee"].as_u64() == Some(tx.fee),
        "fee differs"
    );
    let result = ExecutionResult {
        version: "historical-native-v0".into(),
        success: true,
        error: None,
        fee: tx.fee,
        compute_units: input["outcome"]["compute_units"].as_u64(),
        logs: vec![],
        cpi_calls: vec![],
        accounts: post
            .iter()
            .map(|n| (n.label.clone(), n.account.clone()))
            .collect::<BTreeMap<_, _>>(),
    };
    let legacy_boundary = match adapter.prove_boundaries(&tx, &pre, &expected) {
        Ok(v) => json!({"passed":true,"assumptions":v}),
        Err(e) => json!({"passed":false,"error":format!("{e:#}")}),
    };
    let capabilities = adapter.evaluable_subjects(&tx, &pre);
    let summaries = adapter.summarize(&pre, &result);
    // The product report renders scalar fields through FieldValue::render.
    // Direct serde of its internally-tagged scalar variants is unsupported;
    // this experimental wrapper preserves that rendered value explicitly.
    let summaries: Vec<Value> = summaries
        .iter()
        .map(|field| {
            let value = match &field.value {
                eplyx_engine::protocol::FieldValue::Quantity { .. } => {
                    serde_json::to_value(&field.value).expect("quantity serialization")
                }
                eplyx_engine::protocol::FieldValue::Text(_) => {
                    json!({"type":"text","value":field.value.render()})
                }
                eplyx_engine::protocol::FieldValue::Address(_) => {
                    json!({"type":"address","value":field.value.render()})
                }
                eplyx_engine::protocol::FieldValue::Flag(_) => {
                    json!({"type":"flag","value":field.value.render()})
                }
                eplyx_engine::protocol::FieldValue::Count(_) => {
                    json!({"type":"count","value":field.value.render()})
                }
            };
            json!({"name":field.name,"value":value,"economic":field.economic})
        })
        .collect();
    let differences = adapter.named_findings(&tx, &pre, &result, &result);
    ensure!(
        differences.is_empty(),
        "baseline self-check emitted differences"
    );
    println!(
        "{}",
        json!({"kind":"existing_U2_evaluation_of_experimental_historical_observation",
        "transaction":tx.signature,"target_outer_index":input["target_outer_index"].as_u64().unwrap_or(5),"subjects":capabilities,"summary":summaries,
        "baseline_self_findings":differences,"pre_state_hash":state_hash(&pre)?,"post_state_hash":outcome_hash(&post)?,
        "legacy_boundary_prover":legacy_boundary,"ordinary_production_adapter_admission_changed":false})
    );
    Ok(())
}
