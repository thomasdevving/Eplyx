//! One captured issuer notice -> provenance-bound semantics, never execution assurance.
pub mod identity;
pub mod prestocks;
pub mod workflow;
use crate::{
    lifecycle::frozen::selection::{canonical, digest},
    lifecycle::notice::identity::MechanismType,
    lifecycle::resolution::{ArtifactRef, PathStatus},
    lifecycle::{
        exposure::sha256,
        policy::{
            AssetLifecyclePolicy, LifecycleDeadline, LifecycleScenario, LifecycleScenarioType,
            LifecycleSource, LifecycleSourceKind, LifecycleStatus, SuccessorAsset,
        },
    },
};
use anyhow::{ensure, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenanceClass {
    IssuerAsserted,
    OnChainVerified,
    DemoConfigured,
    Derived,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub source_digest: String,
    pub pointer: String,
    pub byte_range: Option<[usize; 2]>,
    pub exact_text: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field<T> {
    pub value: T,
    pub classifications: Vec<ProvenanceClass>,
    pub provenance: Vec<SourceLocation>,
}
impl<T> Field<T> {
    pub(super) fn new(value: T, class: ProvenanceClass, location: SourceLocation) -> Self {
        Self {
            value,
            classifications: vec![class],
            provenance: vec![location],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedSourceDocument {
    pub schema_version: u32,
    pub source_url: String,
    pub retrieved_at: DateTime<Utc>,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub raw_content: String,
    pub content_sha256: String,
    pub source_type: String,
    pub issuer_label: String,
    pub raw_content_ref: ArtifactRef,
    pub historical_capture: ArtifactRef,
    pub retrieved_at_pointer: String,
    pub transport_metadata_note: String,
}
impl CapturedSourceDocument {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && sha256(self.raw_content.as_bytes()) == self.content_sha256,
            "notice source digest mismatch"
        );
        ensure!(
            self.raw_content_ref.sha256 == self.content_sha256
                && !self.source_url.is_empty()
                && !self.issuer_label.is_empty(),
            "notice source metadata mismatch"
        );
        Ok(())
    }
    pub fn verify(&self, base: &Path) -> Result<()> {
        self.validate()?;
        ensure!(
            self.raw_content_ref.read(base)? == self.raw_content.as_bytes(),
            "notice raw artifact mismatch"
        );
        ensure!(
            self.http_status.is_none() && self.content_type.is_none(),
            "historical capture did not retain transport headers; status/MIME must remain Unknown"
        );
        let historical: serde_json::Value =
            serde_json::from_slice(&self.historical_capture.read(base)?)?;
        ensure!(
            historical.pointer(&self.retrieved_at_pointer)
                == Some(&serde_json::to_value(self.retrieved_at)?),
            "historical retrieval provenance mismatch"
        );
        let sources = historical["sources"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("historical source metadata missing"))?;
        ensure!(
            sources
                .iter()
                .any(|s| s["reference"] == self.source_url
                    && s["content_sha256"] == self.content_sha256),
            "historical source URL/digest mismatch"
        );
        Ok(())
    }
}
pub trait LifecycleEventSource {
    fn normalize(&self, source: &CapturedSourceDocument) -> Result<NormalizedLifecycleEvent>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleEventType {
    SuccessorTransition,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventAsset {
    pub name: Field<Option<String>>,
    pub symbol: Field<Option<String>>,
    pub issuer_asserted_mint: Field<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityStatus {
    Verified,
    Unknown,
    Mismatch,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetIdentity {
    pub asserted_mint: String,
    pub observed_mint: Option<String>,
    pub status: IdentityStatus,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub slot: Option<u64>,
    pub observed_name: Option<String>,
    pub observed_symbol: Option<String>,
    pub provenance: Vec<SourceLocation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedLifecycleEvent {
    pub schema_version: u32,
    pub event_id: String,
    pub adapter_version: String,
    pub source_url: String,
    pub source_content_sha256: String,
    pub captured_document_sha256: String,
    pub field_provenance: BTreeMap<String, ScenarioFieldBinding>,
    pub issuer: Field<String>,
    pub event_type: Field<LifecycleEventType>,
    pub source_asset: EventAsset,
    pub successor_asset: EventAsset,
    pub effective_at: Field<Option<DateTime<Utc>>>,
    pub deadline: Field<DateTime<Utc>>,
    pub deadline_wording: Field<String>,
    pub alternate_destination: Field<String>,
    pub conversion_ratio: Field<Option<String>>,
    pub official_mechanism: Field<MechanismType>,
    pub official_execution_status: Field<PathStatus>,
    pub issuer_assertions: BTreeMap<String, Field<String>>,
    pub source_identity: AssetIdentity,
    pub successor_identity: AssetIdentity,
}
impl NormalizedLifecycleEvent {
    pub fn to_json(&self) -> Result<String> {
        canonical(self)
    }
    pub fn semantic_sha256(&self) -> Result<String> {
        digest(
            &serde_json::json!({"issuer":self.issuer.value,"event_type":self.event_type.value,"source_name":self.source_asset.name.value,"source_mint":self.source_asset.issuer_asserted_mint.value,"successor_symbol":self.successor_asset.symbol.value,"successor_mint":self.successor_asset.issuer_asserted_mint.value,"deadline":self.deadline.value,"deadline_wording":self.deadline_wording.value,"alternate_destination":self.alternate_destination.value,"effective_at":self.effective_at.value,"ratio":self.conversion_ratio.value,"mechanism":self.official_mechanism.value,"execution":self.official_execution_status.value,"assertions":self.issuer_assertions.iter().map(|(k,v)|(k.clone(),v.value.clone())).collect::<BTreeMap<_,_>>()}),
        )
    }
    pub fn render_text(&self) -> String {
        format!("LIFECYCLE EVENT INGESTED\nIssuer: {}\nAsset: {}\nEvent: Successor transition\nSuccessor reference: {} or {}\nDeadline wording: {}\nNormalized exclusive minute cutoff: {}\nSource: {}\nSource identity: {:?}; successor identity: {:?}\nOfficial execution mechanism: {:?}; execution status: {:?}\nNo issuer fact establishes signing access, conversion execution or readiness.\n",self.issuer.value,self.source_asset.name.value.as_deref().unwrap_or("Unknown"),self.successor_asset.symbol.value.as_deref().unwrap_or("Unknown"),self.alternate_destination.value,self.deadline_wording.value,self.deadline.value.to_rfc3339_opts(chrono::SecondsFormat::Secs,true),self.source_url,self.source_identity.status,self.successor_identity.status,self.official_mechanism.value,self.official_execution_status.value)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeadlineInterpretation {
    ExclusiveStartOfMinute,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemoEvaluationConfig {
    pub schema_version: u32,
    pub id: String,
    pub effective_at: DateTime<Utc>,
    pub evaluate_at: DateTime<Utc>,
    pub before: LifecycleStatus,
    pub deadline_interpretation: DeadlineInterpretation,
    pub expiry_interpretation: LifecycleStatus,
    pub not_issuer_policy: bool,
    pub description: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioFieldBinding {
    pub classifications: Vec<ProvenanceClass>,
    pub provenance: Vec<SourceLocation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioBinding {
    pub schema_version: u32,
    pub event_sha256: String,
    pub config_sha256: String,
    pub scenario_sha256: String,
    pub fields: BTreeMap<String, ScenarioFieldBinding>,
}
/// Only an event freshly re-normalized and compared to captured source/identity
/// evidence can produce a scenario. Deserializing normalized JSON is insufficient.
pub struct VerifiedLifecycleEvent(pub(super) NormalizedLifecycleEvent);
impl VerifiedLifecycleEvent {
    pub fn event(&self) -> &NormalizedLifecycleEvent {
        &self.0
    }
    pub fn generate(
        &self,
        source: &CapturedSourceDocument,
        config: &DemoEvaluationConfig,
        config_artifact: &ArtifactRef,
    ) -> Result<(LifecycleScenario, ScenarioBinding)> {
        source.validate()?;
        let e = &self.0;
        ensure!(
            e.captured_document_sha256 == digest(source)?
                && e.source_content_sha256 == source.content_sha256,
            "event/source binding mismatch"
        );
        ensure!(
            config.schema_version == 1
                && config.not_issuer_policy
                && config.before == LifecycleStatus::Active
                && config.expiry_interpretation == LifecycleStatus::NoIssuerEntitlement
                && config.evaluate_at >= config.effective_at
                && config.effective_at < e.deadline.value
                && !config.id.is_empty(),
            "invalid explicit demo evaluation config"
        );
        ensure!(
            digest(config)? == config_artifact.sha256,
            "evaluation config digest mismatch"
        );
        ensure!(
            e.source_identity.status == IdentityStatus::Verified
                && e.successor_identity.status == IdentityStatus::Verified,
            "scenario requires independent source/successor identity verification"
        );
        ensure!(
            e.official_mechanism.value == MechanismType::Unknown
                && e.official_execution_status.value == PathStatus::NotTested
                && e.conversion_ratio.value.is_none(),
            "notice cannot grant an execution mechanism or invented conversion terms"
        );
        let scenario=LifecycleScenario{schema_version:1,scenario_type:LifecycleScenarioType::LifecycleChange,id:config.id.clone(),scenario_version:"1.0.0".into(),captured_at:source.retrieved_at,change:crate::lifecycle::policy::ScenarioDescription{description:"Apply captured issuer successor-transition assertions with an explicit demo boundary, minute-precision cutoff and issuer-policy expiration interpretation; no executable conversion is inferred.".into()},policy:AssetLifecyclePolicy{asset_mint:e.source_asset.issuer_asserted_mint.value.clone(),effective_at:config.effective_at,before:config.before,after:LifecycleStatus::TransitionRequired,deadline:Some(LifecycleDeadline{at:e.deadline.value,after:config.expiry_interpretation}),successor:Some(SuccessorAsset{mint:e.successor_asset.issuer_asserted_mint.value.clone(),description:"Issuer-linked successor identity independently matched to captured initialized mint state; no conversion mechanics or entitlement implied.".into()})},sources:vec![LifecycleSource{id:"captured-issuer-notice".into(),kind:LifecycleSourceKind::ExternalPolicy,reference:source.source_url.clone(),description:"Issuer transition and deadline wording; also allows any other token. Assertions do not prove an IPO, legal worthlessness or execution.".into(),captured_at:source.retrieved_at,supports:vec!["/policy/after".into(),"/policy/deadline".into(),"/policy/successor".into()],artifact:Some(source.raw_content_ref.file.clone()),content_sha256:Some(source.content_sha256.clone())},LifecycleSource{id:"explicit-demo-evaluation".into(),kind:LifecycleSourceKind::ScenarioAssumption,reference:format!("config:{}",config.id),description:config.description.clone(),captured_at:source.retrieved_at,supports:vec!["/policy/effective_at".into(),"/policy/before".into(),"/policy/deadline".into()],artifact:Some(config_artifact.file.clone()),content_sha256:Some(config_artifact.sha256.clone())}]};
        scenario.validate()?;
        let event_sha = digest(e)?;
        let config_sha = digest(config)?;
        let mut fields: BTreeMap<String, ScenarioFieldBinding> = BTreeMap::new();
        let config_loc = |pointer: &str| SourceLocation {
            source_digest: config_sha.clone(),
            pointer: pointer.into(),
            byte_range: None,
            exact_text: None,
        };
        let derived = |pointer: &str| ScenarioFieldBinding {
            classifications: vec![ProvenanceClass::Derived],
            provenance: vec![SourceLocation {
                source_digest: event_sha.clone(),
                pointer: pointer.into(),
                byte_range: None,
                exact_text: None,
            }],
        };
        for (ptr, event_ptr) in [
            ("/schema_version", "/schema_version"),
            ("/scenario_type", "/event_type"),
            ("/scenario_version", "/adapter_version"),
            ("/captured_at", "/captured_document_sha256"),
            ("/change/description", "/issuer_assertions"),
            ("/sources", "/source_url"),
            ("/policy/successor/description", "/successor_identity"),
        ] {
            fields.insert(ptr.into(), derived(event_ptr));
        }
        for (ptr, cfg_ptr) in [
            ("/id", "/id"),
            ("/policy/effective_at", "/effective_at"),
            ("/policy/before", "/before"),
            ("/policy/deadline/after", "/expiry_interpretation"),
        ] {
            fields.insert(
                ptr.into(),
                ScenarioFieldBinding {
                    classifications: vec![ProvenanceClass::DemoConfigured],
                    provenance: vec![config_loc(cfg_ptr)],
                },
            );
        }
        for (ptr, f) in [
            ("/policy/asset_mint", &e.source_asset.issuer_asserted_mint),
            (
                "/policy/successor/mint",
                &e.successor_asset.issuer_asserted_mint,
            ),
        ] {
            fields.insert(
                ptr.into(),
                ScenarioFieldBinding {
                    classifications: f.classifications.clone(),
                    provenance: f.provenance.clone(),
                },
            );
        }
        fields.insert(
            "/policy/after".into(),
            ScenarioFieldBinding {
                classifications: vec![ProvenanceClass::IssuerAsserted, ProvenanceClass::Derived],
                provenance: e.event_type.provenance.clone(),
            },
        );
        let mut provenance = e.deadline.provenance.clone();
        provenance.push(config_loc("/deadline_interpretation"));
        fields.insert(
            "/policy/deadline/at".into(),
            ScenarioFieldBinding {
                classifications: vec![
                    ProvenanceClass::IssuerAsserted,
                    ProvenanceClass::Derived,
                    ProvenanceClass::DemoConfigured,
                ],
                provenance,
            },
        );
        // Materialize provenance for every serialized leaf, including source
        // metadata and support-array elements inherited from their mapped parent.
        fn leaves(value: &serde_json::Value, pointer: &str, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Object(m) => {
                    for (k, v) in m {
                        leaves(v, &format!("{pointer}/{k}"), out)
                    }
                }
                serde_json::Value::Array(a) => {
                    for (i, v) in a.iter().enumerate() {
                        leaves(v, &format!("{pointer}/{i}"), out)
                    }
                }
                _ => out.push(pointer.into()),
            }
        }
        let mut pointers = Vec::new();
        leaves(&serde_json::to_value(&scenario)?, "", &mut pointers);
        for pointer in pointers {
            if !fields.contains_key(&pointer) {
                let parent = fields
                    .iter()
                    .filter(|(key, _)| pointer.starts_with(&format!("{key}/")))
                    .max_by_key(|(key, _)| key.len())
                    .map(|(_, b)| b.clone())
                    .ok_or_else(|| anyhow::anyhow!("unmapped scenario leaf: {pointer}"))?;
                fields.insert(pointer, parent);
            }
        }
        Ok((
            scenario.clone(),
            ScenarioBinding {
                schema_version: 1,
                event_sha256: event_sha,
                config_sha256: config_sha,
                scenario_sha256: scenario.sha256()?,
                fields,
            },
        ))
    }
}
#[cfg(test)]
mod tests;
