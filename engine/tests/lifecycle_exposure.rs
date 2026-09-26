//! Network-free protocol tests from genuine captured account bytes, minimized for tests.
use anyhow::{ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use eplyx_engine::lifecycle::{
    exposure::{
        self,
        meteora_dlmm::{self, MeteoraDlmmAdapter},
        AdapterRun,
    },
    normalize,
    rpc::SolanaRpc,
    AssetDescriptor, EntityType, LifecycleSnapshot, RpcEvidence,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/fixtures/meteora-dlmm-spacex.json"
    ))
    .unwrap()
}
fn base_from(f: &Value) -> LifecycleSnapshot {
    normalize(
        serde_json::from_value::<AssetDescriptor>(f["baseline"]["asset"].clone()).unwrap(),
        f["baseline"]["captured_at"].as_str().unwrap().into(),
        f["baseline"]["rpc_origin"].as_str().unwrap().into(),
        serde_json::from_value::<Vec<RpcEvidence>>(f["baseline"]["evidence"].clone()).unwrap(),
    )
    .unwrap()
}
fn run_from(f: &Value) -> AdapterRun {
    serde_json::from_value(f["adapter_run"].clone()).unwrap()
}
fn enriched() -> LifecycleSnapshot {
    let f = fixture();
    let mut base = base_from(&f);
    let graph = exposure::normalize_graph(&base, vec![run_from(&f)]).unwrap();
    base.schema_version = 2;
    base.exposures = Some(graph);
    base
}
fn mutate_bytes(raw: &mut Value, change: impl FnOnce(&mut Vec<u8>)) {
    let mut bytes = STANDARD.decode(raw["data"][0].as_str().unwrap()).unwrap();
    change(&mut bytes);
    raw["data"][0] = json!(STANDARD.encode(&bytes));
    raw["space"] = json!(bytes.len());
}

#[test]
fn genuine_dlmm_state_normalizes_pool_both_assets_authority_and_balances() {
    let f = fixture();
    let s = enriched();
    s.validate().unwrap();
    let g = s.exposures.unwrap();
    let p = &g.protocol_exposures[0];
    assert_eq!(p.product, "DLMM");
    assert_eq!(p.program_id, meteora_dlmm::PROGRAM_ID);
    assert_eq!(p.authority, p.pool_address);
    assert_eq!(p.decoded_pool["pair_type"], "PermissionlessV2");
    assert_eq!(p.discovered_at_slot, f["expected"]["verified_slot"]);
    assert_eq!(p.assets[0].vault, f["expected"]["target_vault"]);
    assert_eq!(
        p.assets[0].state.raw_balance,
        f["expected"]["target_raw_balance"]
    );
    assert!(p.assets[0].mint_config.is_token_2022);
    assert_eq!(
        p.assets[1].state.raw_balance,
        f["expected"]["paired_raw_balance"]
    );
    assert!(!p.assets[1].mint_config.is_token_2022);
    assert_eq!(p.assets[0].mint_config.extensions.len(), 10);
    assert!(p.assets[0]
        .state
        .extensions
        .iter()
        .any(|e| e.extension_type == "TransferFeeAmount"));
    assert!(p.position_model.is_none());
    assert_eq!(g.summary.verified_program_controlled, 1);
}

#[test]
fn candidate_without_target_mint_is_rejected() {
    let f = fixture();
    let run = run_from(&f);
    let other = solana_address::Address::new_from_array([42; 32]).to_string();
    let error = meteora_dlmm::decode_pool(
        &run.candidate_pool,
        &run.evidence[1].result["value"],
        &other,
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not contain"));
}

#[test]
fn malformed_pool_wrong_owner_discriminator_pda_reserve_or_version_never_classifies() {
    let f = fixture();
    let r = run_from(&f);
    let raw = &r.evidence[1].result["value"];
    for variant in 0..8 {
        let mut bad = raw.clone();
        if variant == 0 {
            bad["owner"] = json!("11111111111111111111111111111111");
        } else {
            mutate_bytes(&mut bad, |b| match variant {
                1 => b[0] ^= 1,
                2 => {
                    b.pop();
                }
                3 => b[72] ^= 1,
                4 => b[152] ^= 1,
                5 => b[882] = 2,
                6 => b[75] = 2,
                _ => b[880] = 2,
            });
        }
        assert!(
            meteora_dlmm::decode_pool(
                &r.candidate_pool,
                &bad,
                f["baseline"]["asset"]["mint"].as_str().unwrap()
            )
            .is_err(),
            "variant {variant}"
        );
    }
    assert!(meteora_dlmm::decode_pool(
        "11111111111111111111111111111111",
        raw,
        f["baseline"]["asset"]["mint"].as_str().unwrap()
    )
    .is_err());
}

#[test]
fn fake_vault_mint_authority_program_extension_and_missing_accounts_are_rejected() {
    let f = fixture();
    let b = base_from(&f);
    for variant in 0..7 {
        let mut r = run_from(&f);
        let values = &mut r.evidence[2].result["value"];
        match variant {
            0 => mutate_bytes(&mut values[1], |b| b[0] ^= 1),
            1 => mutate_bytes(&mut values[1], |b| b[32] ^= 1),
            2 => values[1]["owner"] = json!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
            3 => mutate_bytes(&mut values[1], |b| {
                b[166..168].copy_from_slice(&65000u16.to_le_bytes())
            }),
            4 => values[5]["executable"] = json!(false),
            5 => values[2] = Value::Null,
            _ => values[3]["space"] = json!(1),
        }
        assert!(
            exposure::normalize_graph(&b, vec![r]).is_err(),
            "variant {variant}"
        );
    }
}

#[test]
fn vault_link_retains_phase2_identity_classification_balance_and_historical_pool_proof() {
    let f = fixture();
    let b = base_from(&f);
    let before = b.to_json().unwrap();
    let g = exposure::normalize_graph(&b, vec![run_from(&f)]).unwrap();
    let account = &g.account_exposures[0];
    let entity = &b.entities[0];
    assert_eq!(account.phase2_entity_id, entity.id);
    assert_eq!(
        account.original_classification,
        EntityType::ProgramOwnedAuthority
    );
    assert_eq!(
        account.refinement.as_ref().unwrap().classification,
        "LiquidityVault"
    );
    let link = g.protocol_exposures[0].assets[0]
        .phase2_link
        .as_ref()
        .unwrap();
    assert_eq!(link.phase2_raw_balance, entity.state.raw_balance);
    assert_eq!(
        link.current_raw_balance,
        f["expected"]["target_raw_balance"]
    );
    assert_eq!(
        link.phase2_pool_evidence.account,
        g.protocol_exposures[0].pool_address
    );
    assert_eq!(g.summary.phase2_entities_refined, 1);
    assert_eq!(g.summary.phase2_unknown_refined, 0);
    assert_eq!(before, b.to_json().unwrap());
    assert_eq!(
        g.source_snapshot_sha256,
        exposure::sha256(before.as_bytes())
    );
    assert!(g
        .edges
        .iter()
        .any(|e| e.relationship == "CanonicalLiquidityVault"
            && e.to == format!("token-account:{}", entity.token_account)));
    assert!(g.edges.iter().all(
        |e| !e.evidence.is_empty() && e.evidence.iter().all(|p| p.raw_data_sha256.len() == 64)
    ));
}

#[test]
fn absent_phase2_vault_and_unproven_phase2_pool_role_cannot_be_refined() {
    let mut f = fixture();
    let r = run_from(&f);
    f["baseline"]["evidence"][2]["result"]["value"] = json!([]);
    f["baseline"]["evidence"].as_array_mut().unwrap().remove(3);
    f["baseline"]["evidence"][3]["id"] = json!(3);
    let empty = base_from(&f);
    assert!(exposure::normalize_graph(&empty, vec![r]).is_err());
    let mut f = fixture();
    let r = run_from(&f);
    mutate_bytes(
        &mut f["baseline"]["evidence"][3]["result"]["value"][0],
        |b| b[0] ^= 1,
    );
    let unproven = base_from(&f);
    unproven.validate().unwrap();
    assert!(exposure::normalize_graph(&unproven, vec![r]).is_err());
}

#[test]
fn schema1_compatibility_and_schema2_serialization_roundtrip_are_deterministic() {
    let f = fixture();
    let b = base_from(&f);
    let old = b.to_json().unwrap();
    assert!(!old.contains("\"exposures\""));
    let old_loaded: LifecycleSnapshot = serde_json::from_str(&old).unwrap();
    old_loaded.validate().unwrap();
    assert_eq!(old_loaded.to_json().unwrap(), old);
    let s = enriched();
    let encoded = s.to_json().unwrap();
    let decoded: LifecycleSnapshot = serde_json::from_str(&encoded).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded.to_json().unwrap(), encoded);
    let path = std::env::temp_dir().join(format!(
        "eplyx-exposure-roundtrip-{}.json",
        std::process::id()
    ));
    s.save(&path).unwrap();
    assert!(s.save(&path).is_err());
    assert_eq!(LifecycleSnapshot::load(&path).unwrap(), s);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn offline_replay_rebuilds_the_identical_graph_without_rpc() {
    let f = fixture();
    let b = base_from(&f);
    let r = run_from(&f);
    let first = exposure::normalize_graph(&b, vec![r.clone()]).unwrap();
    let second = exposure::normalize_graph(&b, vec![r]).unwrap();
    assert_eq!(first, second);
    first.validate(&b).unwrap();
    assert!(first
        .edges
        .windows(2)
        .all(|pair| (&pair[0].from, &pair[0].to, &pair[0].relationship)
            <= (&pair[1].from, &pair[1].to, &pair[1].relationship)));
}

#[test]
fn incomplete_cross_chain_old_context_unknown_adapter_and_tampered_proof_fail() {
    let f = fixture();
    let b = base_from(&f);
    for variant in 0..6 {
        let mut r = run_from(&f);
        match variant {
            0 => {
                r.evidence.pop();
            }
            1 => r.evidence[0].result = json!("another-chain"),
            2 => r.evidence[2].result["context"]["slot"] = json!(1),
            3 => r.adapter.version = "unsupported".into(),
            4 => r.evidence[2].params[0][0] = json!("11111111111111111111111111111111"),
            _ => {
                r.evidence[2].result["value"].as_array_mut().unwrap().pop();
            }
        }
        assert!(
            exposure::normalize_graph(&b, vec![r]).is_err(),
            "variant {variant}"
        );
    }
    let s = enriched();
    let mut bad = s.clone();
    bad.exposures.as_mut().unwrap().edges[0].evidence[0].raw_data_sha256 = "tampered".into();
    assert!(bad.validate().is_err());
    let mut bad = s.clone();
    bad.exposures.as_mut().unwrap().account_exposures[0].original_classification =
        EntityType::Unknown;
    assert!(bad.validate().is_err());
    let mut bad = s.clone();
    bad.schema_version = 1;
    assert!(bad.validate().is_err());
    let mut bad = s;
    bad.exposures.as_mut().unwrap().source_snapshot_sha256 = "tampered".into();
    assert!(bad.validate().is_err());
}

struct MockRpc {
    responses: Mutex<VecDeque<RpcEvidence>>,
}
impl SolanaRpc for MockRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let expected = self.responses.lock().unwrap().pop_front().unwrap();
        ensure!(
            method == expected.method && params == expected.params,
            "unexpected query"
        );
        Ok(expected.result)
    }
}
#[test]
fn source_uses_only_one_candidate_read_and_one_contextual_six_account_batch() {
    let f = fixture();
    let b = base_from(&f);
    let run = run_from(&f);
    let rpc = MockRpc {
        responses: Mutex::new(run.evidence.clone().into()),
    };
    let s = exposure::discover(&b, &MeteoraDlmmAdapter, &run.candidate_pool, &rpc).unwrap();
    s.validate().unwrap();
    assert!(rpc.responses.lock().unwrap().is_empty());
    assert_eq!(s.entities, b.entities);
    assert_eq!(s.evidence, b.evidence);
    assert_eq!(s.exposures.as_ref().unwrap().summary.pools_verified, 1);
}

#[test]
fn decoder_offsets_and_discriminator_match_pinned_official_zero_copy_idl() {
    let idl: Value = serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/fixtures/meteora-dlmm-layout.json"
    ))
    .unwrap();
    let types: BTreeMap<String, Value> = idl["types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().into(), t.clone()))
        .collect();
    fn size(t: &Value, types: &BTreeMap<String, Value>) -> usize {
        if let Some(s) = t.as_str() {
            return match s {
                "u8" => 1,
                "u16" => 2,
                "u32" | "i32" => 4,
                "u64" | "i64" => 8,
                "u128" => 16,
                "pubkey" => 32,
                _ => panic!("unsupported field"),
            };
        }
        if let Some(a) = t.get("array") {
            return size(&a[0], types) * a[1].as_u64().unwrap() as usize;
        }
        types[t["defined"]["name"].as_str().unwrap()]["type"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| size(&f["type"], types))
            .sum()
    }
    let mut offset = 8;
    let mut offsets = BTreeMap::new();
    for f in types["LbPair"]["type"]["fields"].as_array().unwrap() {
        offsets.insert(f["name"].as_str().unwrap(), offset);
        offset += size(&f["type"], &types);
    }
    assert_eq!(offset, meteora_dlmm::ACCOUNT_LEN);
    for (name, offset) in [
        ("bump_seed", 72),
        ("token_x_mint", 88),
        ("token_y_mint", 120),
        ("reserve_x", 152),
        ("reserve_y", 184),
        ("base_key", 784),
        ("token_mint_x_program_flag", 880),
        ("token_mint_y_program_flag", 881),
        ("version", 882),
    ] {
        assert_eq!(offsets[name], offset);
    }
    assert_eq!(idl["address"], meteora_dlmm::PROGRAM_ID);
    assert_eq!(idl["revision"], meteora_dlmm::SDK_REVISION);
    assert_eq!(
        idl["account"]["discriminator"],
        json!(meteora_dlmm::DISCRIMINATOR)
    );
    assert_eq!(
        types["PairType"]["type"]["variants"][3]["name"],
        "PermissionlessV2"
    );
    assert_eq!(
        types["TokenProgramFlags"]["type"]["variants"][1]["name"],
        "TokenProgram2022"
    );
}
