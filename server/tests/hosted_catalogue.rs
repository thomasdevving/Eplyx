use eplyx_server::{
    hosted::catalogue::{self, Capture},
    registry::Registry,
    storage::Storage,
};
use serde_json::{json, Value};
fn fixtures() -> (std::path::PathBuf, Value) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("fixtures/catalogue");
    let manifest =
        serde_json::from_slice(&std::fs::read(root.join("provenance.json")).unwrap()).unwrap();
    (root, manifest)
}
fn html(products: Value) -> String {
    format!(
        "<script>self.__next_f.push({})</script>",
        json!([1, format!("0:{}\n", json!({"products":products}))])
    )
}
#[test]
fn retained_first_party_bytes_keep_nine_canonical_mints_and_original_source_identity() {
    let (root, manifest) = fixtures();
    for item in manifest["files"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(item["file"].as_str().unwrap()))
            .expect("import catalogue fixtures");
        assert_eq!(bytes.len() as u64, item["bytes"].as_u64().unwrap());
        assert_eq!(eplyx_engine::replay::hash_bytes(&bytes), item["sha256"]);
        let capture: Capture = serde_json::from_slice(&bytes).unwrap();
        let result = catalogue::from_capture(&capture).unwrap();
        assert_eq!(result.entries.len(), 9);
        assert_eq!(format!("{}.json", result.version), item["file"]);
        assert_eq!(result.source_status, "SavedSource");
        let mut tampered = capture;
        tampered.content.push('!');
        assert!(catalogue::from_capture(&tampered).is_err());
    }
    assert!(catalogue::parse_products("<script>globalThis.bad=true</script>").is_err());
}
#[test]
fn malformed_fields_are_refused_and_conflicting_assertions_retained_without_merging_identities() {
    let one = json!({"splMint":"So11111111111111111111111111111111111111112","name":"Duplicate","symbol":"DUP","decimals":9});
    for field in ["splMint", "name", "symbol", "decimals"] {
        let mut bad = one.clone();
        bad.as_object_mut().unwrap().remove(field);
        assert!(catalogue::parse_products(&html(json!([bad]))).is_err());
    }
    for v in [json!(-1), json!(256), json!("9")] {
        let mut bad = one.clone();
        bad["decimals"] = v;
        assert!(catalogue::parse_products(&html(json!([bad]))).is_err());
    }
    let mut other = one.clone();
    other["splMint"] = json!("11111111111111111111111111111111");
    let mut conflict = one.clone();
    conflict["symbol"] = json!("OTHER");
    let entries = catalogue::parse_products(&html(json!([one, other, conflict, one]))).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].assertions.len(), 2);
    assert_eq!(entries[1].assertions[0].name, "Duplicate");
}
#[test]
fn versions_are_immutable_and_references_cannot_bind_another_mint() {
    let (root, manifest) = fixtures();
    let scratch = tempfile::tempdir().unwrap();
    let registry = Registry::new(Storage::open(scratch.path()).unwrap());
    let mut pins = vec![];
    for item in manifest["files"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(item["file"].as_str().unwrap())).unwrap();
        let c = catalogue::import_capture(&registry, &bytes).unwrap();
        let mint = c.entries[0].mint.clone();
        pins.push(catalogue::reference(&registry, &c.version, &mint).unwrap());
        assert!(catalogue::reference(
            &registry,
            &c.version,
            "So11111111111111111111111111111111111111112"
        )
        .is_err());
        assert!(catalogue::reference(&registry, "../arbitrary", &mint).is_err());
    }
    for pin in pins {
        assert_eq!(
            catalogue::reference(&registry, &pin.version, &pin.assertions[0].mint).unwrap(),
            pin
        );
    }
    assert!(catalogue::import_capture(&registry, b"invalid").is_err());
}
