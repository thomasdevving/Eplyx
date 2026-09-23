use std::{fs, path::PathBuf};

use eplyx_engine::universal::{
    execution::{HistoricalRuntimeEvidence, RuntimeProfile},
    historical_features::{HistoricalFeatureReceipt, HistoricalFeatureSetEvidence},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fixture() -> (HistoricalFeatureSetEvidence, Value) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let audit: Value = serde_json::from_slice(
        &fs::read(root.join("docs/examples/phase-u13-2b-feature-universe/audit.json")).unwrap(),
    )
    .unwrap();
    let receipts = audit["observations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| HistoricalFeatureReceipt {
            id: row["id"].as_str().unwrap().into(),
            source_sha256: row["responseSha256"].as_str().unwrap().into(),
            source_response: fs::read_to_string(root.join(row["source"].as_str().unwrap()))
                .unwrap(),
        })
        .collect();
    (
        HistoricalFeatureSetEvidence::new(409942000, receipts).unwrap(),
        audit,
    )
}
fn rewrite_receipt(
    feature: &mut HistoricalFeatureSetEvidence,
    id: &str,
    f: impl FnOnce(&mut Value),
) {
    let row = feature.receipts.iter_mut().find(|r| r.id == id).unwrap();
    let mut json: Value = serde_json::from_str(&row.source_response).unwrap();
    f(&mut json);
    row.source_response = serde_json::to_string(&json).unwrap();
    row.source_sha256 = sha(row.source_response.as_bytes());
}
fn rebuilt_parent_identity_still_rejects(feature: HistoricalFeatureSetEvidence) {
    let mut evidence = HistoricalRuntimeEvidence::new(
        "EK66pFxagY8mA6VjRmNnBqhZwcQ44jFztfNVfueaDWgM".into(),
        RuntimeProfile::sysvar_snapshot_hash(&Default::default()).unwrap(),
        "LiteSVM 0.16.0 historical evidence".into(),
        "agave-4.2.2-native-system-compute".into(),
        "mutation-test".into(),
    )
    .unwrap();
    evidence.historical_feature_set = Some(feature);
    evidence.evidence_id = evidence.identity().unwrap();
    assert!(evidence.validate().is_err());
}

#[test]
fn historical_feature_mutations_fail_after_rebuilding_parent_identity() {
    let (valid, audit) = fixture();
    valid.validate().unwrap();
    assert_eq!(valid.active.len(), 275);
    let active = valid.active.keys().next().unwrap().clone();
    let inactive = audit["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["state"] == "inactive")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let absent = audit["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["state"] == "absent")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let future = audit["backendPostTargetFeatures"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // 1. Remove an active feature from the claimed set.
    let mut m = valid.clone();
    m.active.remove(&active);
    rebuilt_parent_identity_still_rejects(m);
    // 2. Activate an inactive feature without changing its raw archive bytes.
    let mut m = valid.clone();
    m.active.insert(inactive, 1);
    rebuilt_parent_identity_still_rejects(m);
    // 3. Activate an absent feature without an archive account.
    let mut m = valid.clone();
    m.active.insert(absent, 1);
    rebuilt_parent_identity_still_rejects(m);
    // 4. Mutate owner in the raw source, rebuilding its response hash and the
    // inventory identity. The claimed active set still contradicts the source.
    let mut m = valid.clone();
    rewrite_receipt(&mut m, &active, |r| {
        r["result"]["value"]["owner"] = json!("11111111111111111111111111111111")
    });
    m.inventory_id = HistoricalFeatureSetEvidence::new(m.target_slot, m.receipts.clone())
        .unwrap()
        .inventory_id;
    rebuilt_parent_identity_still_rejects(m);
    // 5. Mutate activation in the raw source with dependent source/inventory hashes rebuilt.
    let mut m = valid.clone();
    rewrite_receipt(&mut m, &active, |r| {
        let data = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            r["result"]["value"]["data"][0].as_str().unwrap(),
        )
        .unwrap();
        let mut data = data;
        data[1..9].copy_from_slice(&1u64.to_le_bytes());
        r["result"]["value"]["data"][0] = json!(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            data
        ));
    });
    m.inventory_id = HistoricalFeatureSetEvidence::new(m.target_slot, m.receipts.clone())
        .unwrap()
        .inventory_id;
    rebuilt_parent_identity_still_rejects(m);
    // 6. Remove the source/reference entirely.
    let mut m = valid.clone();
    m.receipts
        .iter_mut()
        .find(|r| r.id == active)
        .unwrap()
        .source_response
        .clear();
    rebuilt_parent_identity_still_rejects(m);
    // 7. Change inventory identity.
    let mut m = valid.clone();
    m.inventory_id = "0".repeat(64);
    rebuilt_parent_identity_still_rejects(m);
    // 8. Change the explicit feature-set hash.
    let mut m = valid.clone();
    m.feature_set_hash = "0".repeat(64);
    rebuilt_parent_identity_still_rejects(m);
    // 9. Enable one known post-target feature.
    let mut m = valid.clone();
    m.active.insert(future, 409942000);
    rebuilt_parent_identity_still_rejects(m);
}
