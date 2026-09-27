#![allow(dead_code, unused_imports)]
//! Deterministic reconstructed acquisition using frozen raw bytes and deployed code.
//! These tests execute the VM; none constitutes live acquisition acceptance.
use anyhow::{bail, Result};
use eplyx_engine::{
    lifecycle::{current as wallet, exposure::sha256, rpc::SolanaRpc},
    path::{current as check, CapturedExecutionFixture, ExitPathType},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Mutex};
pub struct Rpc {
    raw: BTreeMap<String, Value>,
    slot: u64,
    pub mint: String,
    pub source: String,
    pub owner: String,
    pub calls: Mutex<Vec<String>>,
}
impl Rpc {
    pub fn new() -> Self {
        let root = eplyx_engine::lifecycle::artifact::reference_root();
        let fixture: CapturedExecutionFixture = serde_json::from_slice(
            &std::fs::read(root.join("probes/phase7-captures/fixtures/group-0.json")).unwrap(),
        )
        .unwrap();
        let batch = &fixture.evidence[3];
        let mut raw: BTreeMap<_, _> = batch.params[0]
            .as_array()
            .unwrap()
            .iter()
            .zip(batch.result["value"].as_array().unwrap())
            .map(|(a, v)| (a.as_str().unwrap().to_string(), v.clone()))
            .collect();
        // Additional exact deployed accounts for the replacement mint and market
        // route. The provider is reconstructed; it is never a live observation.
        let market: eplyx_engine::path::current::Capture = serde_json::from_slice(
            &std::fs::read(eplyx_engine::repo_root().join(
                "fixtures/current/sta/reports/milestone4-validation/live-market.capture.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let mut slot = batch.result["context"]["slot"].as_u64().unwrap();
        for record in market.observations {
            if record.method != "getMultipleAccounts" {
                continue;
            }
            let Some(result) = record.result else {
                continue;
            };
            for (key, value) in record.params[0]
                .as_array()
                .unwrap()
                .iter()
                .zip(result["value"].as_array().unwrap())
            {
                if value.is_null() {
                    continue;
                }
                let key = key.as_str().unwrap().to_string();
                if key == eplyx_engine::standard_programs::token::CLOCK
                    && result["context"]["slot"].as_u64().unwrap() >= slot
                {
                    slot = result["context"]["slot"].as_u64().unwrap();
                    raw.insert(key, value.clone());
                } else {
                    raw.entry(key).or_insert_with(|| value.clone());
                }
            }
        }
        let mint = fixture.evidence[1].params[0].as_str().unwrap().to_string();
        let source = "741ZXYKzgjPZucRhPUQFK7kKAgP1ESJwimhumgGpuPHs".to_string();
        let owner = eplyx_engine::lifecycle::decode::decode_token_account(
            &raw[&source],
            eplyx_engine::lifecycle::decode::TOKEN_2022_PROGRAM,
            &mint,
            9,
        )
        .unwrap()
        .owner;
        Self {
            raw,
            slot,
            mint,
            source,
            owner,
            calls: Mutex::new(vec![]),
        }
    }
    pub fn with_wallet(mut self, wallet: &str) -> Self {
        let capture: wallet::Capture = serde_json::from_str(wallet).unwrap();
        for record in capture.observations {
            let Some(result) = record.result else {
                continue;
            };
            if record.method == "getAccountInfo" {
                self.raw.insert(
                    record.params[0].as_str().unwrap().into(),
                    result["value"].clone(),
                );
            }
            if record.method == "getTokenAccountsByOwner" {
                for row in result["value"].as_array().unwrap() {
                    self.raw.insert(
                        row["pubkey"].as_str().unwrap().into(),
                        row["account"].clone(),
                    );
                }
            }
        }
        self
    }
    pub fn request(&self) -> check::CheckRequest {
        check::CheckRequest {
            path: ExitPathType::Transfer,
            source: self.source.clone(),
            amount_mode: check::AmountMode::Custom,
            amount_decimal: Some("0.0000001".into()),
            recipient: "124XHuTYUNnCf2ABCNcCEdQFpHQ7Y6MnPe9NeouDB1az".into(),
            output_mint: None,
            minimum_output_decimal: None,
        }
    }
    fn wallet(&self) -> String {
        serde_json::to_string(
            &wallet::capture_selected(
                wallet::InspectionSelection {
                    cluster: "solana-mainnet".into(),
                    mint: self.mint.clone(),
                    reference: None,
                    sample_accounts: false,
                    public_owner: Some(self.owner.clone()),
                },
                self,
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn capture(&self) -> check::Capture {
        check::capture(
            self.wallet(),
            self.request(),
            "run-a".into(),
            "check-a".into(),
            self,
        )
        .unwrap()
    }
}
impl SolanaRpc for Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls.lock().unwrap().push(method.into());
        let value = match method {
            "getGenesisHash" => return Ok(json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
            "getAccountInfo" => self
                .raw
                .get(params[0].as_str().unwrap())
                .cloned()
                .unwrap_or(Value::Null),
            "getMultipleAccounts" => json!(params[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| self
                    .raw
                    .get(a.as_str().unwrap())
                    .cloned()
                    .unwrap_or(Value::Null))
                .collect::<Vec<_>>()),
            "getTokenAccountsByOwner" => {
                json!([{"pubkey":self.source,"account":self.raw[&self.source]}])
            }
            "getProgramAccounts"
                if params[0] == eplyx_engine::standard_programs::token::TOKEN_2022_PROGRAM
                    || params[0] == eplyx_engine::standard_programs::token::LEGACY_PROGRAM =>
            {
                json!(self
                    .raw
                    .iter()
                    .filter(
                        |(_, v)| eplyx_engine::lifecycle::decode::decode_token_account(
                            v,
                            params[0].as_str().unwrap(),
                            &self.mint,
                            9
                        )
                        .is_ok()
                    )
                    .map(|(a, v)| json!({"pubkey":a,"account":v}))
                    .collect::<Vec<_>>())
            }
            "getProgramAccounts" => json!(self
                .raw
                .iter()
                .filter(|(_, v)| v["owner"]
                    == eplyx_engine::lifecycle::exposure::meteora_dlmm::PROGRAM_ID
                    && v["space"] == 904)
                .map(|(a, v)| json!({"pubkey":a,"account":v}))
                .collect::<Vec<_>>()),
            _ => bail!("unexpected method"),
        };
        Ok(json!({"context":{"slot":self.slot},"value":value}))
    }
}
