//! Synthetic read-only-provider fixtures for presentation tests. No network client.
use anyhow::{bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use eplyx_engine::{
    canonical,
    lifecycle::{
        current, policy::LifecycleScenario, rpc::SolanaRpc, AssetDescriptor, LifecycleStateSource,
        SolanaTokenAssetSource,
    },
    standard_programs::{spl_token, system},
};
use serde_json::{json, Value};
use solana_address::Address;
use solana_program_pack::Pack;
use spl_token_2022_interface::state::{Account, AccountState, Mint};
use std::path::PathBuf;
const MAINNET: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
fn address(n: u8) -> Address {
    Address::new_from_array([n; 32])
}
fn owner() -> Address {
    let mut b = [0x66; 32];
    b[0] = 0x58;
    Address::new_from_array(b)
}
fn raw(data: &[u8], program: &str) -> Value {
    json!({"owner":program,"data":[STANDARD.encode(data),"base64"],"lamports":10000000,"executable":false,"rentEpoch":u64::MAX,"space":data.len()})
}
fn mint() -> Value {
    let mut bytes = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            mint_authority: Default::default(),
            supply: 1234567,
            decimals: 6,
            is_initialized: true,
            freeze_authority: Default::default(),
        },
        &mut bytes,
    )
    .unwrap();
    raw(&bytes, spl_token::PROGRAM_ID)
}
fn token() -> Value {
    let mut bytes = vec![0; Account::LEN];
    Account::pack(
        Account {
            mint: address(1),
            owner: owner(),
            amount: 1234567,
            state: AccountState::Initialized,
            ..Account::default()
        },
        &mut bytes,
    )
    .unwrap();
    raw(&bytes, spl_token::PROGRAM_ID)
}
struct Mock;
impl SolanaRpc for Mock {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let contextual = |v| json!({"context":{"slot":100},"value":v});
        Ok(match method {
            "getGenesisHash" => json!(MAINNET),
            "getAccountInfo" => contextual(if params[0] == address(1).to_string() {
                mint()
            } else {
                raw(&[], system::PROGRAM_ID)
            }),
            "getProgramAccounts" => {
                contextual(json!([{"pubkey":address(3).to_string(),"account":token()}]))
            }
            "getMultipleAccounts" => contextual(json!([raw(&[], system::PROGRAM_ID)])),
            _ => bail!("unexpected fixture request"),
        })
    }
}
fn main() -> Result<()> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("new output directory required"))?,
    );
    std::fs::create_dir(&out)?;
    let snapshot = SolanaTokenAssetSource { rpc: Mock }.capture(AssetDescriptor {
        name: "Synthetic presentation asset".into(),
        mint: address(1).to_string(),
        expected_token_program: Some(spl_token::PROGRAM_ID.into()),
        expected_genesis_hash: Some(MAINNET.into()),
        verification: vec![],
    })?;
    snapshot.save(&out.join("snapshot.json"))?;
    let scenario: LifecycleScenario = serde_json::from_value(
        json!({"schema_version":1,"scenario_type":"lifecycle_change","id":"synthetic-dashboard-policy","scenario_version":"1","captured_at":"2026-09-26T00:00:00Z","change":{"description":"Synthetic declared lifecycle policy for presentation tests."},"policy":{"asset_mint":address(1).to_string(),"effective_at":"2026-09-27T00:00:00Z","before":"Active","after":"TransitionRequired","deadline":{"at":"2026-09-28T00:00:00Z","after":"Expired"},"successor":null},"sources":[{"id":"declared-fixture-policy","kind":"ScenarioAssumption","reference":"synthetic presentation fixture","description":"No issuer statement or live capture.","captured_at":"2026-09-26T00:00:00Z","supports":["/policy/effective_at","/policy/before","/policy/after","/policy/deadline"],"artifact":null,"content_sha256":null}]}),
    )?;
    scenario.validate()?;
    std::fs::write(out.join("scenario.json"), canonical::document(&scenario)?)?;
    let selection = current::InspectionSelection {
        cluster: "solana-mainnet".into(),
        mint: address(1).to_string(),
        reference: None,
        sample_accounts: false,
        public_owner: None,
    };
    let capture = current::capture_selected(selection, &Mock)?;
    current::save(&capture, &out.join("current.capture.json"))?;
    // The fixtures represent responses from Mock, never a contacted cluster.
    println!("Generated synthetic observation and lifecycle inputs.");
    Ok(())
}
