//! Deterministic public-wallet acquisition fixtures; no live RPC.
use anyhow::{bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use eplyx_engine::lifecycle::{current, decode, rpc::SolanaRpc};
use serde_json::{json, Value};
use solana_address::Address;
use solana_program_pack::Pack;
use spl_token_2022_interface::{
    extension::{
        confidential_transfer::ConfidentialTransferAccount, BaseStateWithExtensionsMut,
        ExtensionType, StateWithExtensionsMut,
    },
    state::{Account, AccountState, Mint},
};
use std::sync::Mutex;
fn address(n: u8) -> String {
    Address::new_from_array([n; 32]).to_string()
}
fn selection() -> current::InspectionSelection {
    current::InspectionSelection {
        cluster: "solana-mainnet".into(),
        mint: address(1),
        reference: None,
        sample_accounts: false,
        public_owner: Some(address(2)),
    }
}
fn raw(bytes: Vec<u8>, program: &str) -> Value {
    json!({"owner":program,"executable":false,"space":bytes.len(),"data":[STANDARD.encode(bytes),"base64"]})
}
struct Rpc {
    calls: Mutex<Vec<(String, Value)>>,
    program: &'static str,
    rows: usize,
    fail: bool,
    owner: Value,
    bad: Option<&'static str>,
    confidential: bool,
}
impl Default for Rpc {
    fn default() -> Self {
        Self {
            calls: Mutex::new(vec![]),
            program: decode::TOKEN_2022_PROGRAM,
            rows: 1,
            fail: false,
            owner: Value::Null,
            bad: None,
            confidential: false,
        }
    }
}
impl SolanaRpc for Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        match method {
            "getGenesisHash" => Ok(json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
            "getAccountInfo" if params[0] == address(1) => {
                let mut b = vec![0; Mint::LEN];
                Mint::pack(
                    Mint {
                        is_initialized: true,
                        decimals: 9,
                        ..Mint::default()
                    },
                    &mut b,
                )?;
                Ok(json!({"context":{"slot":10},"value":raw(b,self.program)}))
            }
            "getAccountInfo" => Ok(json!({"context":{"slot":11},"value":self.owner})),
            "getTokenAccountsByOwner" => {
                assert_eq!(
                    params,
                    json!([address(2),{"mint":address(1)},{"commitment":"finalized","encoding":"base64","minContextSlot":11}])
                );
                if self.fail {
                    bail!("RPC getTokenAccountsByOwner HTTP status 429 Too Many Requests")
                }
                let rows = (0..self.rows)
                    .map(|i| {
                        let base = Account {
                            mint: address(if i == 0 && self.bad == Some("mint") {
                                9
                            } else {
                                1
                            })
                            .parse()
                            .unwrap(),
                            owner: address(if i == 0 && self.bad == Some("owner") {
                                9
                            } else {
                                2
                            })
                            .parse()
                            .unwrap(),
                            amount: u64::MAX,
                            state: AccountState::Frozen,
                            delegate: Some(Address::new_from_array([3; 32])).into(),
                            delegated_amount: u64::MAX,
                            close_authority: Some(Address::new_from_array([4; 32])).into(),
                            ..Account::default()
                        };
                        let mut bytes = if self.confidential {
                            vec![
                                0;
                                ExtensionType::try_calculate_account_len::<Account>(&[
                                    ExtensionType::ConfidentialTransferAccount
                                ])
                                .unwrap()
                            ]
                        } else {
                            vec![0; Account::LEN]
                        };
                        if self.confidential {
                            let mut s =
                                StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut bytes)
                                    .unwrap();
                            s.init_extension::<ConfidentialTransferAccount>(true)
                                .unwrap();
                            s.base = base;
                            s.pack_base();
                            s.init_account_type().unwrap();
                        } else {
                            Account::pack(base, &mut bytes).unwrap();
                        }
                        if i == 0 && self.bad == Some("decode") {
                            bytes.truncate(12);
                        }
                        json!({"pubkey":address(10+i as u8),"account":raw(bytes,self.program)})
                    })
                    .collect::<Vec<_>>();
                Ok(json!({"context":{"slot":12},"value":rows}))
            }
            _ => panic!("Unexpected RPC {method}"),
        }
    }
}
fn run(rpc: &Rpc) -> (current::Capture, Value) {
    let c = current::capture_selected(selection(), rpc).unwrap();
    let r = current::evaluate(&c).unwrap();
    (c, r)
}
#[test]
fn one_and_multiple_legacy_and_token2022_accounts_preserve_exact_state() {
    for program in [decode::LEGACY_PROGRAM, decode::TOKEN_2022_PROGRAM] {
        for rows in [1, 2] {
            let rpc = Rpc {
                program,
                rows,
                ..Rpc::default()
            };
            let (c, r) = run(&rpc);
            let w = &r["wallet_observation"];
            assert_eq!(w["account_count"], rows);
            assert_eq!(w["status"], "Completed");
            assert_eq!(
                w["public_balance_total_raw"],
                ((u64::MAX as u128) * (rows as u128)).to_string()
            );
            for a in w["token_accounts"].as_array().unwrap() {
                assert_eq!(a["state"]["raw_balance"], u64::MAX.to_string());
                assert_eq!(a["state"]["delegate"], address(3));
                assert_eq!(a["state"]["delegated_amount"], u64::MAX.to_string());
                assert_eq!(a["state"]["close_authority"], address(4));
                assert_eq!(a["state"]["is_frozen"], true);
            }
            assert_eq!(r["mint"]["token_program"], program);
            assert_eq!(c.schema_version, 3);
            assert_eq!(rpc.calls.lock().unwrap().len(), 4);
        }
    }
}
#[test]
fn empty_success_is_distinct_from_rpc_failure() {
    let (_, empty) = run(&Rpc {
        rows: 0,
        ..Rpc::default()
    });
    assert_eq!(empty["wallet_observation"]["status"], "Completed");
    assert_eq!(empty["wallet_observation"]["public_balance_total_raw"], "0");
    assert_eq!(empty["wallet_observation"]["account_count"], 0);
    let (_, failed) = run(&Rpc {
        fail: true,
        ..Rpc::default()
    });
    assert_eq!(failed["wallet_observation"]["status"], "Unavailable");
    assert!(failed["wallet_observation"]["public_balance_total_raw"].is_null());
    assert!(failed["wallet_observation"]["account_count"].is_null());
}
#[test]
fn invalid_owner_rejected_before_any_rpc() {
    let rpc = Rpc::default();
    let mut s = selection();
    s.public_owner = Some("not an address".into());
    assert!(current::capture_selected(s, &rpc).is_err());
    assert!(rpc.calls.lock().unwrap().is_empty());
}
#[test]
fn programs_mints_and_token_accounts_are_not_reinterpreted() {
    let mut b = vec![0; Account::LEN];
    Account::pack(
        Account {
            mint: Address::new_from_array([1; 32]),
            owner: Address::new_from_array([7; 32]),
            state: AccountState::Initialized,
            ..Account::default()
        },
        &mut b,
    )
    .unwrap();
    let mut m = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            is_initialized: true,
            ..Mint::default()
        },
        &mut m,
    )
    .unwrap();
    for (owner, kind) in [
        (json!({"executable":true}), "Program"),
        (raw(b, decode::TOKEN_2022_PROGRAM), "TokenAccount"),
        (raw(m, decode::LEGACY_PROGRAM), "Mint"),
    ] {
        let rpc = Rpc {
            owner,
            ..Rpc::default()
        };
        let (_, r) = run(&rpc);
        let w = &r["wallet_observation"];
        assert_eq!(w["status"], "InvalidOwner");
        assert_eq!(w["owner_inspection"]["kind"], kind);
        if kind == "TokenAccount" {
            assert_eq!(w["owner_inspection"]["suggested_owner"], address(7));
        }
        assert_eq!(rpc.calls.lock().unwrap().len(), 3);
        assert!(w["account_count"].is_null());
    }
}
#[test]
fn wrong_mint_owner_and_decoder_gaps_preserve_other_rows_without_claiming_total() {
    for bad in ["mint", "owner", "decode"] {
        let (_, r) = run(&Rpc {
            rows: 2,
            bad: Some(bad),
            ..Rpc::default()
        });
        let w = &r["wallet_observation"];
        assert_eq!(w["status"], "Partial");
        assert_eq!(w["decoded_account_count"], 1);
        assert!(w["public_balance_total_raw"].is_null());
        assert_eq!(w["known_public_balance_subtotal_raw"], u64::MAX.to_string());
        assert_eq!(w["gaps"].as_array().unwrap().len(), 1);
    }
}
#[test]
fn confidential_balance_remains_unknown_and_extensions_retained() {
    let (_, r) = run(&Rpc {
        confidential: true,
        ..Rpc::default()
    });
    let w = &r["wallet_observation"];
    assert!(w["confidential_balance"]
        .as_str()
        .unwrap()
        .starts_with("Unknown"));
    assert_eq!(
        w["token_accounts"][0]["state"]["extensions"][0]["extension_type"],
        "ConfidentialTransferAccount"
    );
    assert_eq!(w["public_balance_total_raw"], u64::MAX.to_string());
}
#[test]
fn replay_and_refresh_never_inherit_execution_or_lifecycle() {
    let rpc = Rpc::default();
    let (c, r) = run(&rpc);
    let (_, second) = run(&rpc);
    assert_eq!(
        rpc.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "getTokenAccountsByOwner")
            .count(),
        2
    );
    for result in [&r, &second] {
        assert!(result["lifecycle_event"].is_null());
        assert!(result["readiness"].is_null());
        assert_eq!(result["execution_performed"], false);
        assert!(result["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["status"] == "NotTested"));
    }
    let path = std::env::temp_dir().join(format!("wallet-{}.json", std::process::id()));
    current::save(&c, &path).unwrap();
    assert!(current::save(&c, &path).is_err());
    assert_eq!(current::replay(&path).unwrap(), r);
    std::fs::remove_file(path).unwrap();
    assert_eq!(rpc.calls.lock().unwrap().len(), 8);
}
#[test]
fn exact_owner_mint_context_and_decoder_bindings_fail_closed() {
    let (c, _) = run(&Rpc::default());
    for key in ["owner", "mint", "context", "decoder", "version"] {
        let mut other = c.clone();
        match key {
            "owner" => other.selection.as_mut().unwrap().public_owner = Some(address(9)),
            "mint" => other.selection.as_mut().unwrap().mint = address(9),
            "context" => other.observations[3].params[2]["minContextSlot"] = json!(1),
            "decoder" => other.decoder = "unknown".into(),
            _ => other.schema_version = 2,
        };
        assert!(current::evaluate(&other).is_err());
    }
}
#[test]
fn malformed_duplicate_and_stale_response_never_establish_zero_holdings() {
    let (c, _) = run(&Rpc::default());
    for mode in ["duplicate", "stale", "malformed"] {
        let mut other = c.clone();
        let lookup = other.observations[3].result.as_mut().unwrap();
        match mode {
            "duplicate" => {
                let row = lookup["value"][0].clone();
                lookup["value"].as_array_mut().unwrap().push(row);
            }
            "stale" => lookup["context"]["slot"] = json!(1),
            _ => lookup["value"] = Value::Null,
        };
        let r = current::evaluate(&other).unwrap();
        assert_eq!(r["wallet_observation"]["status"], "Unavailable");
        assert!(r["wallet_observation"]["public_balance_total_raw"].is_null());
    }
}
#[test]
fn owner_lookup_failure_and_mint_failure_remain_unknown() {
    let (c, _) = run(&Rpc::default());
    let mut other = c.clone();
    other.observations.truncate(3);
    other.observations[2].result = None;
    other.observations[2].error = Some("unavailable".into());
    assert_eq!(
        current::evaluate(&other).unwrap()["wallet_observation"]["status"],
        "Unavailable"
    );
    let mut other = c;
    other.observations[1].result = None;
    other.observations[1].error = Some("unavailable".into());
    assert!(current::evaluate(&other).is_err());
}
