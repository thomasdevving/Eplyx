//! Additive Phase 7: immutable selection -> capture -> measured evidence -> delta.
//! This module never changes the Phase 4–6 population or assurance contract.
pub mod discovery;
pub mod pipeline;
pub mod selector;

pub use crate::canonical::{digest, document as canonical};
pub use crate::lifecycle::artifact::load;
use crate::{
    lifecycle::frozen::coverage::{
        CaseStatus, CoverageAggregate, CoverageClassification, CoverageReport,
    },
    lifecycle::{EntityType, LifecycleSnapshot},
    path::ExitPathType,
};
use anyhow::{ensure, Result};
pub use selector::{expand, SelectorConfig};
use serde::{Deserialize, Serialize};
use std::path::Path;
pub fn save<T: Serialize>(v: &T, p: &Path) -> Result<()> {
    crate::path::save_json(v, p)
}
pub fn path_key(path: ExitPathType) -> String {
    serde_json::to_value(path).unwrap().as_str().unwrap().into()
}
pub fn type_key(t: &EntityType) -> String {
    serde_json::to_value(t).unwrap().as_str().unwrap().into()
}
pub fn classify(balance: u64, covered: u64, unsupported: bool) -> CoverageClassification {
    if balance > 0 && covered == balance {
        CoverageClassification::Proven
    } else if covered > 0 {
        CoverageClassification::PartiallyProven
    } else if unsupported {
        CoverageClassification::Unsupported
    } else {
        CoverageClassification::Untested
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmountPoint {
    pub raw: String,
    pub invalid_control: bool,
    pub rationale: String,
}
pub fn amount_matrix(balance: u64) -> Vec<AmountPoint> {
    if balance == 0 {
        return vec![];
    }
    let mut points = std::collections::BTreeMap::new();
    for percent in [1u64, 25, 50, 100] {
        let n = (u128::from(balance) * u128::from(percent) / 100).max(1) as u64;
        points.entry(n).or_insert_with(||format!("max(1,floor(observed public balance * {percent}/100)); directly measured point, no interpolation"));
    }
    let mut out: Vec<_> = points
        .into_iter()
        .map(|(n, rationale)| AmountPoint {
            raw: n.to_string(),
            invalid_control: false,
            rationale,
        })
        .collect();
    if let Some(n) = balance.checked_add(1) {
        out.push(AmountPoint{raw:n.to_string(),invalid_control:true,rationale:"observed balance + 1; invalid control excluded from positive assurance even if a later balance differs".into()});
    }
    out
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Eligibility {
    ExecutableCandidate,
    CaptureRequired,
    Unsupported,
    Invalid,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageGap {
    pub account_class: String,
    pub path_type: ExitPathType,
    pub entities: usize,
    pub represented_raw: String,
    pub already_measured_raw: String,
    pub without_evidence_raw: String,
    pub execution_supported: bool,
    pub reason: String,
    pub highest_balance_entities: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedGain {
    pub entities: usize,
    pub authorities: usize,
    pub represented_raw: String,
    pub entity_path_contexts: usize,
    pub venue: usize,
    pub path_type: usize,
    pub state_shape: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreComponents {
    #[serde(with = "crate::numfmt::u128_string")]
    pub entity: u128,
    #[serde(with = "crate::numfmt::u128_string")]
    pub amount: u128,
    #[serde(with = "crate::numfmt::u128_string")]
    pub class: u128,
    #[serde(with = "crate::numfmt::u128_string")]
    pub venue: u128,
    #[serde(with = "crate::numfmt::u128_string")]
    pub path: u128,
    #[serde(with = "crate::numfmt::u128_string")]
    pub entity_path_context: u128,
    #[serde(with = "crate::numfmt::u128_string")]
    pub state_shape: u128,
    #[serde(with = "crate::numfmt::u128_string")]
    pub feasibility: u128,
    #[serde(with = "crate::numfmt::u128_string")]
    pub redundancy_penalty: u128,
    #[serde(with = "crate::numfmt::i128_string")]
    pub total: i128,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateProbe {
    pub id: String,
    pub entity_id: String,
    pub authority: String,
    pub account_class: String,
    pub represented_raw: String,
    pub balance_bucket: u8,
    pub state_shape_sha256: String,
    pub path_type: ExitPathType,
    pub context_id: String,
    pub eligibility: Eligibility,
    pub reason: String,
    pub initial_score: Option<ScoreComponents>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedProbe {
    pub selection_order: usize,
    pub candidate: CandidateProbe,
    pub score: ScoreComponents,
    pub expected_gain: ExpectedGain,
    pub amount_matrix: Vec<AmountPoint>,
    pub fixture_reference: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionPlan {
    pub schema_version: u32,
    pub selector_version: String,
    pub snapshot_sha256: String,
    pub impact_sha256: String,
    pub coverage_sha256: String,
    pub baseline_plan_sha256: String,
    pub inventory_sha256: String,
    pub config: SelectorConfig,
    pub before: CoverageAggregate,
    pub candidate_count: usize,
    pub candidates: Vec<CandidateProbe>,
    pub rejected_contexts: Vec<discovery::VenueContext>,
    pub gaps: Vec<CoverageGap>,
    pub selected: Vec<SelectedProbe>,
    pub limitations: Vec<String>,
}
impl ExpansionPlan {
    pub fn validate(
        &self,
        s: &LifecycleSnapshot,
        b: &CoverageReport,
        inventory: &discovery::VenueInventory,
    ) -> Result<()> {
        ensure!(
            *self == expand(s, b, inventory, &self.config)?,
            "expansion plan differs from pre-execution deterministic selection"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReference {
    pub group_id: String,
    pub case_id: String,
    pub result_sha256: String,
    pub fixture_sha256: Option<String>,
    pub result_file: String,
    pub status: CaseStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integer_amount_matrix_boundaries_and_controls() {
        assert!(amount_matrix(0).is_empty());
        let one = amount_matrix(1);
        assert_eq!(
            one.iter().map(|p| p.raw.as_str()).collect::<Vec<_>>(),
            vec!["1", "2"]
        );
        assert!(one[1].invalid_control);
        let three = amount_matrix(3);
        assert_eq!(
            three.iter().map(|p| p.raw.as_str()).collect::<Vec<_>>(),
            vec!["1", "3", "4"]
        );
        let hundred = amount_matrix(100);
        assert_eq!(
            hundred.iter().map(|p| p.raw.as_str()).collect::<Vec<_>>(),
            vec!["1", "25", "50", "100", "101"]
        );
        let max = amount_matrix(u64::MAX);
        assert_eq!(max.len(), 4);
        assert_eq!(max.last().unwrap().raw, u64::MAX.to_string());
        assert!(!max.iter().any(|p| p.invalid_control));
    }
    #[test]
    fn full_and_partial_never_follow_selection_or_zero_balance() {
        assert_eq!(classify(100, 0, false), CoverageClassification::Untested);
        assert_eq!(
            classify(100, 25, false),
            CoverageClassification::PartiallyProven
        );
        assert_eq!(classify(100, 100, false), CoverageClassification::Proven);
        assert_ne!(classify(0, 0, false), CoverageClassification::Proven);
        assert_eq!(classify(100, 0, true), CoverageClassification::Unsupported);
    }
}
