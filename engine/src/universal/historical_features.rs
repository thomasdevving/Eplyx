//! Content-bound feature-account receipts for a historically initialized VM.
use std::collections::{BTreeMap, BTreeSet};

use agave_feature_set::{FeatureSet, FEATURE_NAMES};
use anyhow::{anyhow, ensure, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::replay::hash_bytes;

const FEATURE_OWNER: &str = "Feature111111111111111111111111111111111111";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalFeatureReceipt {
    pub id: String,
    /// SHA-256 of the exact retained JSON-RPC response bytes.
    pub source_sha256: String,
    /// Retained raw response. Validation never trusts a precomputed active list.
    pub source_response: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalFeatureSetEvidence {
    pub schema: String,
    pub target_slot: u64,
    pub inventory_id: String,
    pub feature_set_hash: String,
    pub active: BTreeMap<String, u64>,
    pub receipts: Vec<HistoricalFeatureReceipt>,
}

impl HistoricalFeatureSetEvidence {
    pub fn new(target_slot: u64, mut receipts: Vec<HistoricalFeatureReceipt>) -> Result<Self> {
        receipts.sort_by(|a, b| a.id.cmp(&b.id));
        let mut evidence = Self {
            schema: "eplyx-historical-feature-set-v1".into(),
            target_slot,
            inventory_id: String::new(),
            feature_set_hash: String::new(),
            active: BTreeMap::new(),
            receipts,
        };
        evidence.inventory_id = evidence.inventory_hash()?;
        let active = evidence.derive_active()?;
        evidence.feature_set_hash = evidence.active_hash(&active)?;
        evidence.active = active;
        Ok(evidence)
    }

    fn inventory_hash(&self) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            "eplyx-feature-inventory-v1",
            self.receipts
                .iter()
                .map(|r| (&r.id, &r.source_sha256))
                .collect::<Vec<_>>(),
        ))?))
    }

    fn active_hash(&self, active: &BTreeMap<String, u64>) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            "eplyx-historical-feature-set-v1",
            self.target_slot,
            &self.inventory_id,
            active,
        ))?))
    }

    fn derive_active(&self) -> Result<BTreeMap<String, u64>> {
        ensure!(
            self.schema == "eplyx-historical-feature-set-v1",
            "feature evidence schema"
        );
        ensure!(!self.receipts.is_empty(), "feature inventory empty");
        let mut ids = BTreeSet::new();
        let mut active = BTreeMap::new();
        let mut prior = None;
        for receipt in &self.receipts {
            receipt
                .id
                .parse::<solana_pubkey::Pubkey>()
                .map_err(|e| anyhow!("invalid feature ID {}: {e}", receipt.id))?;
            ensure!(
                prior.as_ref().is_none_or(|p: &String| p < &receipt.id),
                "feature inventory duplicate or unordered"
            );
            prior = Some(receipt.id.clone());
            ids.insert(receipt.id.clone());
            ensure!(
                hash_bytes(receipt.source_response.as_bytes()) == receipt.source_sha256,
                "feature source reference differs: {}",
                receipt.id
            );
            let response: Value = serde_json::from_str(&receipt.source_response)
                .with_context(|| format!("feature source JSON: {}", receipt.id))?;
            ensure!(
                response.get("error").is_none()
                    && response
                        .pointer("/result/context/slot")
                        .and_then(Value::as_u64)
                        == Some(self.target_slot),
                "feature source slot/error: {}",
                receipt.id
            );
            let Some(value) = response.pointer("/result/value").filter(|v| !v.is_null()) else {
                continue;
            };
            let owner = value
                .get("owner")
                .and_then(Value::as_str)
                .context("feature owner missing")?;
            let pair = value
                .get("data")
                .and_then(Value::as_array)
                .context("feature data missing")?;
            ensure!(
                pair.len() == 2 && pair[1].as_str() == Some("base64"),
                "feature data encoding"
            );
            let data = base64::engine::general_purpose::STANDARD
                .decode(pair[0].as_str().context("feature data string missing")?)?;
            ensure!(
                value.get("space").and_then(Value::as_u64) == Some(data.len() as u64),
                "feature account space differs: {}",
                receipt.id
            );
            if owner != FEATURE_OWNER {
                continue;
            }
            ensure!(
                value.get("executable").and_then(Value::as_bool) == Some(false) && data.len() == 9,
                "feature account layout: {}",
                receipt.id
            );
            match data[0] {
                0 => ensure!(
                    data[1..].iter().all(|b| *b == 0),
                    "inactive feature encoding"
                ),
                1 => {
                    let slot = u64::from_le_bytes(data[1..].try_into()?);
                    ensure!(
                        slot <= self.target_slot,
                        "future feature activation: {}",
                        receipt.id
                    );
                    active.insert(receipt.id.clone(), slot);
                }
                _ => anyhow::bail!("invalid feature variant: {}", receipt.id),
            }
        }
        ensure!(
            FEATURE_NAMES.keys().all(|id| ids.contains(&id.to_string())),
            "backend-known feature identity omitted"
        );
        Ok(active)
    }

    pub fn validate(&self) -> Result<()> {
        let active = self.derive_active()?;
        ensure!(
            self.inventory_id == self.inventory_hash()?,
            "feature inventory identity differs"
        );
        ensure!(
            self.active == active,
            "claimed active features differ from sources"
        );
        ensure!(
            self.feature_set_hash == self.active_hash(&active)?,
            "feature-set hash differs"
        );
        Ok(())
    }

    pub fn feature_set(&self) -> Result<FeatureSet> {
        self.validate()?;
        let mut set = FeatureSet::default();
        for (id, slot) in self.derive_active()? {
            set.activate(&id.parse()?, slot);
        }
        Ok(set)
    }
}
