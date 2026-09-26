//! Deterministic RPC fixtures, not live-mainnet tests.
use anyhow::{bail, Result};
use eplyx_engine::lifecycle::{current, rpc::SolanaRpc, AssetDescriptor};
use serde_json::{json, Value};
use std::sync::Mutex;

struct Rpc {
    calls: Mutex<Vec<String>>,
    wrong_chain: bool,
    fail_mint: bool,
}
impl SolanaRpc for Rpc {
    fn call(&self, method: &str, _params: Value) -> Result<Value> {
        self.calls.lock().unwrap().push(method.into());
        match method {
            "getGenesisHash" => Ok(json!(if self.wrong_chain {
                "wrong"
            } else {
                "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
            })),
            "getAccountInfo" => {
                if self.fail_mint {
                    bail!("RPC unavailable");
                }
                let fixture: Value = serde_json::from_str(include_str!(
                    "../../fixtures/lifecycle/main-t7/fixtures/token-2022-spacex-mint.json"
                ))?;
                Ok(json!({"context":{"slot":448335802},"value":fixture["account"]}))
            }
            "getTokenLargestAccounts" => {
                bail!("RPC getTokenLargestAccounts failed with code Some(429)")
            }
            _ => panic!("unexpected RPC {method}"),
        }
    }
}
fn fixture() -> (AssetDescriptor, Rpc) {
    (
        serde_json::from_str(include_str!(
            "../../fixtures/path/sta/assets/prestocks-spacex.json"
        ))
        .unwrap(),
        Rpc {
            calls: Mutex::new(vec![]),
            wrong_chain: false,
            fail_mint: false,
        },
    )
}
#[test]
fn fresh_capture_calls_rpc_again_and_rate_limit_preserves_only_valid_mint_findings() {
    let (asset, rpc) = fixture();
    let first = current::capture(asset.clone(), &rpc).unwrap();
    let second = current::capture(asset, &rpc).unwrap();
    assert_eq!(rpc.calls.lock().unwrap().len(), 6);
    for c in [&first, &second] {
        let r = current::evaluate(c).unwrap();
        assert_eq!(r["discovery"]["status"], "Unavailable");
        assert_eq!(r["discovery"]["complete_population"], false);
        assert!(r["discovery"]["gaps"].as_array().unwrap().len() == 1);
        assert!(r["mint"]["extensions"].as_array().unwrap().len() > 5);
        assert!(r["mint"]["raw_supply"].is_string());
        assert!(r["readiness"].is_null());
        assert_eq!(r["authorization"], false);
        assert_eq!(r["local_execution_performed"], false);
        assert!(r["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["status"] == "NotTested"));
    }
}
#[test]
fn offline_replay_redecodes_and_retains_original_rpc_evidence() {
    let (asset, rpc) = fixture();
    let c = current::capture(asset, &rpc).unwrap();
    let path = std::env::temp_dir().join(format!("eplyx-current-{}.json", std::process::id()));
    current::save(&c, &path).unwrap();
    assert!(current::save(&c, &path).is_err());
    assert_eq!(
        current::evaluate(&c).unwrap(),
        current::replay(&path).unwrap()
    );
    std::fs::remove_file(path).unwrap();
    assert_eq!(rpc.calls.lock().unwrap().len(), 3);
}
#[test]
fn wrong_cluster_and_missing_mint_never_fall_back_to_demo() {
    for (wrong_chain, fail_mint) in [(true, false), (false, true)] {
        let (asset, mut rpc) = fixture();
        rpc.wrong_chain = wrong_chain;
        rpc.fail_mint = fail_mint;
        let c = current::capture(asset, &rpc).unwrap();
        assert!(current::evaluate(&c).is_err());
        assert_eq!(
            rpc.calls.lock().unwrap().len(),
            if wrong_chain { 1 } else { 2 }
        );
    }
}
#[test]
fn changed_scope_commitment_program_or_injected_proof_fails_closed() {
    let (asset, rpc) = fixture();
    let c = current::capture(asset, &rpc).unwrap();
    let mut other = c.clone();
    other.asset.mint = "11111111111111111111111111111111".into();
    assert!(current::evaluate(&other).is_err());
    let mut other = c.clone();
    other.observations[1].params[1]["commitment"] = json!("processed");
    assert!(current::evaluate(&other).is_err());
    let mut other = c.clone();
    other.observations[1].result.as_mut().unwrap()["value"]["owner"] =
        json!("11111111111111111111111111111111");
    assert!(current::evaluate(&other).is_err());
    let mut other = serde_json::to_value(&c).unwrap();
    other["paths"] = json!([{"status":"Proven"}]);
    assert!(serde_json::from_value::<current::Capture>(other).is_err());
}
#[test]
fn acquisition_interval_and_decoder_version_are_checked() {
    let (asset, rpc) = fixture();
    let c = current::capture(asset, &rpc).unwrap();
    let mut other = c.clone();
    other.decoder = "untrusted".into();
    assert!(current::evaluate(&other).is_err());
    let mut other = c;
    other.completed_at = "2000-01-01T00:00:00Z".into();
    assert!(current::evaluate(&other).is_err());
}

struct SampleRpc {
    stale_batch: bool,
    missing_account: bool,
}
impl SolanaRpc for SampleRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        use solana_address::Address;
        use solana_program_pack::Pack;
        use spl_token_2022_interface::state::{Account, AccountState};
        let (asset, rpc) = fixture();
        let address = Address::new_from_array([9; 32]).to_string();
        match method {
            "getTokenLargestAccounts" => {
                Ok(json!({"context":{"slot":448335810},"value":[{"address":address}]}))
            }
            "getMultipleAccounts" => {
                assert_eq!(params[0], json!([address]));
                assert_eq!(params[1]["minContextSlot"], 448335810);
                let mut bytes = vec![0; Account::LEN];
                Account::pack(
                    Account {
                        mint: asset.mint.parse()?,
                        owner: Address::new_from_array([8; 32]),
                        amount: u64::MAX,
                        state: AccountState::Frozen,
                        ..Account::default()
                    },
                    &mut bytes,
                )?;
                let raw = if self.missing_account {
                    Value::Null
                } else {
                    json!({"owner":asset.expected_token_program,"data":[STANDARD.encode(bytes),"base64"],"executable":false,"space":Account::LEN})
                };
                Ok(
                    json!({"context":{"slot":if self.stale_batch {448335809}else{448335811}},"value":[raw]}),
                )
            }
            _ => rpc.call(method, params),
        }
    }
}
#[test]
fn sampled_token_state_preserves_exact_amounts_and_never_becomes_population_or_authority_proof() {
    let (asset, _) = fixture();
    let c = current::capture(
        asset,
        &SampleRpc {
            stale_batch: false,
            missing_account: false,
        },
    )
    .unwrap();
    let r = current::evaluate(&c).unwrap();
    assert_eq!(
        r["accounts"][0]["state"]["raw_balance"],
        u64::MAX.to_string()
    );
    assert_eq!(r["accounts"][0]["state"]["is_frozen"], true);
    assert_eq!(r["accounts"][0]["authority_classification"], "Unknown");
    assert_eq!(r["discovery"]["complete_population"], false);
    assert_eq!(r["discovery"]["status"], "Sampled");
}
#[test]
fn missing_sample_account_is_a_gap_and_stale_capture_is_rejected() {
    let (asset, _) = fixture();
    let c = current::capture(
        asset.clone(),
        &SampleRpc {
            stale_batch: false,
            missing_account: true,
        },
    )
    .unwrap();
    let r = current::evaluate(&c).unwrap();
    assert!(r["accounts"].as_array().unwrap().is_empty());
    assert_eq!(r["discovery"]["gaps"].as_array().unwrap().len(), 1);
    let c = current::capture(
        asset,
        &SampleRpc {
            stale_batch: true,
            missing_account: false,
        },
    )
    .unwrap();
    assert!(current::evaluate(&c).is_err());
}

fn selected(mint: &str, sample: bool) -> current::InspectionSelection {
    current::InspectionSelection {
        cluster: "solana-mainnet".into(),
        mint: mint.into(),
        reference: None,
        sample_accounts: sample,
        public_owner: None,
    }
}
#[test]
fn dynamic_mint_is_bound_to_requests_capture_replay_and_no_saved_evidence() {
    let (_, rpc) = fixture();
    let mint = "So11111111111111111111111111111111111111112";
    let c = current::capture_selected(selected(mint, false), &rpc).unwrap();
    assert_eq!(c.observations[1].params[0], mint);
    let r = current::evaluate(&c).unwrap();
    assert_eq!(r["asset"]["mint"], mint);
    assert_eq!(r["selection"]["mint"], mint);
    assert_eq!(r["discovery"]["status"], "NotRequested");
    assert!(r["discovery"]["sample_count"].is_null());
    assert!(r["identity_provenance"]
        .as_str()
        .unwrap()
        .contains("unconfirmed"));
    // This deterministic fixture deliberately contains another mint's metadata: retained, never identity authority.
    assert!(!r["identity_mismatches"].as_array().unwrap().is_empty());
    for key in ["lifecycle_event", "readiness"] {
        assert!(r[key].is_null());
    }
    for key in [
        "execution_performed",
        "local_execution_performed",
        "authorization",
        "funds_moved",
    ] {
        assert_eq!(r[key], false);
    }
    let mut altered = c.clone();
    altered.asset.mint = "11111111111111111111111111111111".into();
    assert!(current::evaluate(&altered).is_err());
    let mut altered = c.clone();
    altered.observations[1].params[0] = json!("11111111111111111111111111111111");
    assert!(current::evaluate(&altered).is_err());
    let mut altered = c.clone();
    altered.selection.as_mut().unwrap().cluster = "devnet".into();
    assert!(current::evaluate(&altered).is_err());
    assert!(selected("not a public key", false).validate().is_err());
}
#[test]
fn selected_catalogue_identity_and_original_source_assertions_remain_pinned() {
    let (asset, rpc) = fixture();
    let mut selection = selected(&asset.mint, true);
    selection.reference = Some(current::CatalogueReference {
        version: "a".repeat(64),
        source_url: "https://prestocks.com/products".into(),
        retrieved_at: "2026-09-19T00:00:00Z".into(),
        content_sha256: "b".repeat(64),
        assertions: vec![current::SourceAssertion {
            mint: asset.mint.clone(),
            name: "Different source name".into(),
            symbol: "SOURCE".into(),
            decimals: 8,
        }],
    });
    let c = current::capture_selected(selection.clone(), &rpc).unwrap();
    let r = current::evaluate(&c).unwrap();
    assert_eq!(r["selection"], json!(selection));
    assert_eq!(r["discovery"]["status"], "Unavailable");
    assert!(r["discovery"]["sample_count"].is_null());
    assert_eq!(r["inspection"]["status"], "Completed");
    assert!(!r["identity_mismatches"].as_array().unwrap().is_empty());
    selection.reference.as_mut().unwrap().assertions[0].mint =
        "So11111111111111111111111111111111111111112".into();
    assert!(selection.validate().is_err());
}
#[test]
fn nonmint_accounts_and_unsupported_configuration_retain_observations_without_fake_mint() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use solana_program_pack::Pack;
    use spl_token_2022_interface::state::{Account, AccountState, Mint};
    let (asset, rpc) = fixture();
    let original = current::capture_selected(selected(&asset.mint, false), &rpc).unwrap();
    let mut account = vec![0; Account::LEN];
    Account::pack(
        Account {
            mint: asset.mint.parse().unwrap(),
            state: AccountState::Initialized,
            ..Account::default()
        },
        &mut account,
    )
    .unwrap();
    let mut uninitialized = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            decimals: 9,
            supply: 1,
            is_initialized: false,
            ..Mint::default()
        },
        &mut uninitialized,
    )
    .unwrap();
    for (raw, kind) in [
        (Value::Null, "Missing"),
        (
            json!({"owner":"11111111111111111111111111111111","executable":false,"data":["","base64"],"space":0}),
            "UnsupportedOwner",
        ),
        (
            json!({"owner":"11111111111111111111111111111111","executable":true}),
            "Program",
        ),
        (
            json!({"owner":asset.expected_token_program,"executable":false,"data":[STANDARD.encode(account),"base64"],"space":Account::LEN}),
            "TokenAccount",
        ),
        (
            json!({"owner":"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA","executable":false,"data":[STANDARD.encode(uninitialized),"base64"],"space":Mint::LEN}),
            "Mint",
        ),
    ] {
        let mut c = original.clone();
        c.observations[1].result.as_mut().unwrap()["value"] = raw;
        let r = current::evaluate(&c).unwrap();
        assert!(r["mint"].is_null());
        assert_eq!(r["inspection"]["account_type"], kind);
        assert!(r["readiness"].is_null());
        if kind == "Mint" {
            assert_eq!(r["inspection"]["base_fields"]["is_initialized"], false);
            assert_eq!(
                r["inspection"]["base_fields"]["decimal_supply"],
                "0.000000001"
            );
        }
    }
}
#[test]
fn cross_mint_sample_is_a_scoped_gap_never_another_assets_balance() {
    let (asset, _) = fixture();
    let mut c = current::capture_selected(
        selected(&asset.mint, true),
        &SampleRpc {
            stale_batch: false,
            missing_account: false,
        },
    )
    .unwrap();
    let other = "So11111111111111111111111111111111111111112";
    c.selection.as_mut().unwrap().mint = other.into();
    c.asset.mint = other.into();
    c.observations[1].params[0] = json!(other);
    c.observations[2].params[0] = json!(other);
    let r = current::evaluate(&c).unwrap();
    assert_eq!(r["inspection"]["status"], "Completed");
    assert_eq!(r["discovery"]["status"], "Sampled");
    assert_eq!(r["discovery"]["sample_count"], 0);
    assert_eq!(r["accounts"], json!([]));
    assert!(!r["discovery"]["gaps"].as_array().unwrap().is_empty());
    assert_eq!(r["discovery"]["complete_population"], false);
}
