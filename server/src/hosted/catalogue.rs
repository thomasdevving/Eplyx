//! First-party identity assertions, parsed as JSON data. No downloaded code is
//! evaluated and a catalogue assertion never substitutes for observed mint state.
use crate::{api::Shared, artifacts::ArtifactRef, registry::Registry};
use anyhow::{ensure, Context, Result};
use axum::{extract::State, Json};
use eplyx_engine::{
    lifecycle::current::{CatalogueReference, InspectionSelection, SourceAssertion},
    replay::hash_bytes,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub const SOURCE_URL: &str = "https://prestocks.com/products";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub source_url: String,
    pub retrieved_at: String,
    pub content_sha256: String,
    pub content: String,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub cluster: String,
    pub mint: String,
    pub assertions: Vec<SourceAssertion>,
}
#[derive(Serialize)]
pub struct Catalogue {
    pub version: String,
    pub source_url: String,
    pub retrieved_at: String,
    pub content_sha256: String,
    pub source_status: &'static str,
    pub entries: Vec<Entry>,
}
fn products(value: &Value, out: &mut Vec<Value>) -> Result<()> {
    match value {
        Value::Array(items) => {
            for item in items {
                products(item, out)?;
            }
        }
        Value::Object(fields) => {
            if let Some(list) = fields.get("products") {
                out.extend(
                    list.as_array()
                        .context("malformed products")?
                        .iter()
                        .cloned(),
                );
                ensure!(out.len() <= 100, "catalogue exceeds product bound");
            } else {
                for item in fields.values() {
                    products(item, out)?;
                }
            }
        }
        _ => {}
    };
    Ok(())
}
pub fn parse_products(content: &str) -> Result<Vec<Entry>> {
    ensure!(
        content.len() <= 2 * 1024 * 1024,
        "catalogue content exceeds byte bound"
    );
    let mut found = Vec::new();
    for piece in content.split("self.__next_f.push(").skip(1) {
        let Some((encoded, _)) = piece.split_once(")</script>") else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<Value>(encoded) else {
            continue;
        };
        if record[0] != 1 {
            continue;
        }
        let Some(lines) = record[1].as_str() else {
            continue;
        };
        for line in lines.lines() {
            if let Some((_, encoded)) = line.split_once(':') {
                if let Ok(value) = serde_json::from_str::<Value>(encoded) {
                    products(&value, &mut found)?;
                }
            }
        }
    }
    ensure!(
        !found.is_empty() && found.len() <= 100,
        "official product payload absent or exceeds bound"
    );
    let mut entries: Vec<Entry> = vec![];
    for p in found {
        let text = |field: &str| -> Result<String> {
            let value = p[field]
                .as_str()
                .context("catalogue identity field missing")?;
            ensure!(
                !value.trim().is_empty() && value.chars().count() <= 160,
                "invalid catalogue field"
            );
            Ok(value.into())
        };
        let mint = text("splMint")?;
        let decimals = u8::try_from(p["decimals"].as_u64().context("invalid decimals")?)?;
        let assertion = SourceAssertion {
            mint: mint.clone(),
            name: text("name")?,
            symbol: text("symbol")?,
            decimals,
        };
        InspectionSelection {
            cluster: "solana-mainnet".into(),
            mint: mint.clone(),
            reference: None,
            sample_accounts: false,
            public_owner: None,
        }
        .validate()?;
        if let Some(entry) = entries.iter_mut().find(|e| e.mint == mint) {
            if !entry.assertions.contains(&assertion) {
                entry.assertions.push(assertion);
            }
        } else {
            entries.push(Entry {
                cluster: "solana-mainnet".into(),
                mint,
                assertions: vec![assertion],
            });
        }
    }
    Ok(entries)
}
pub fn from_capture(capture: &Capture) -> Result<Catalogue> {
    ensure!(
        capture.source_url == SOURCE_URL
            && hash_bytes(capture.content.as_bytes()) == capture.content_sha256,
        "invalid catalogue capture"
    );
    chrono::DateTime::parse_from_rfc3339(&capture.retrieved_at)?;
    // Preserve the pinned source's JSON field order for this existing identity.
    #[derive(Serialize)]
    struct Identity<'a> {
        source_url: &'a str,
        retrieved_at: &'a str,
        content_sha256: &'a str,
    }
    let version = hash_bytes(&serde_json::to_vec(&Identity {
        source_url: &capture.source_url,
        retrieved_at: &capture.retrieved_at,
        content_sha256: &capture.content_sha256,
    })?);
    Ok(Catalogue {
        version,
        source_url: capture.source_url.clone(),
        retrieved_at: capture.retrieved_at.clone(),
        content_sha256: capture.content_sha256.clone(),
        source_status: "SavedSource",
        entries: parse_products(&capture.content)?,
    })
}
fn directory(registry: &Registry) -> std::path::PathBuf {
    registry.storage().artifacts_root().join("catalogue")
}
pub fn import_capture(registry: &Registry, bytes: &[u8]) -> Result<Catalogue> {
    ensure!(
        bytes.len() <= 3 * 1024 * 1024,
        "catalogue capture exceeds byte bound"
    );
    let capture: Capture = serde_json::from_slice(bytes)?;
    let catalogue = from_capture(&capture)?;
    let reference = registry.document_ref(bytes)?;
    let file = directory(registry).join(format!("{}.json", catalogue.version));
    if file.exists() {
        let old: ArtifactRef = registry.storage().read_json(&file)?;
        ensure!(old == reference, "catalogue version is immutable");
    } else {
        registry.storage().write_json(&file, &reference)?;
    }
    registry.storage().write_json(
        &directory(registry).join("current.json"),
        &json!({"version":catalogue.version}),
    )?;
    Ok(catalogue)
}
pub fn load(registry: &Registry, version: &str) -> Result<Catalogue> {
    ensure!(
        crate::artifacts::canonical_sha256(version),
        "invalid catalogue version"
    );
    let reference: ArtifactRef = registry
        .storage()
        .read_json(&directory(registry).join(format!("{version}.json")))?;
    let catalogue = from_capture(&serde_json::from_slice(
        &registry.document_bytes(&reference)?,
    )?)?;
    ensure!(catalogue.version == version, "catalogue version mismatch");
    Ok(catalogue)
}
pub fn reference(registry: &Registry, version: &str, mint: &str) -> Result<CatalogueReference> {
    let catalogue = load(registry, version)?;
    let entry = catalogue
        .entries
        .into_iter()
        .find(|e| e.mint == mint)
        .context("catalogue mint mismatch")?;
    Ok(CatalogueReference {
        version: catalogue.version,
        source_url: catalogue.source_url,
        retrieved_at: catalogue.retrieved_at,
        content_sha256: catalogue.content_sha256,
        assertions: entry.assertions,
    })
}
pub async fn current(State(state): State<Shared>) -> Json<Value> {
    let loaded = (|| -> Result<Value> {
        let pointer: Value = state
            .registry
            .storage()
            .read_json(&directory(&state.registry).join("current.json"))?;
        Ok(serde_json::to_value(load(
            &state.registry,
            pointer["version"]
                .as_str()
                .context("catalogue pointer missing")?,
        )?)?)
    })();
    Json(loaded.unwrap_or_else(|_| json!({"source_status":"Unavailable","entries":[]})))
}
