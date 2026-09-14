//! Regression clustering and common-condition derivation.
//!
//! Eleven critical findings are not eleven bugs. This module groups findings
//! that share an underlying trigger, describes the conditions the group has in
//! common, and picks one representative fixture a developer can start from.
//!
//! Everything here is derived arithmetically from fixture state and observed
//! differences. There is no heuristic scoring, no natural-language model and no
//! sampling: the same corpus always produces the same clusters, in the same
//! order, with the same representatives.
//!
//! # Layering
//!
//! This sits *above* execution and diffing: it consumes [`StateDiff`] values and
//! never produces them. It is protocol-aware, alongside `corpus`, `interpret`
//! and `impact`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::diff::{Difference, StateDiff};
use crate::impact::{EconomicConsequence, FixtureEconomics};
use crate::interpret::{format_health, instruction_name};
use crate::money::Usd;
use crate::types::Fixture;

/// A numeric range is reported as a *condition* only when its members sit close
/// together. Twenty percent relative spread is the cutoff: wider than that and
/// "debt between $2,500 and $7,960" describes the corpus rather than the
/// trigger, which would be worse than saying nothing.
pub const INFORMATIVE_SPREAD_BPS: i128 = 2_000;

/// The unit a numeric range is measured in, which determines how it renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Lamports,
    MicroUsd,
    /// Fixed-point health factor, `HEALTH_SCALE` == 1.0.
    Health,
    Raw,
}

impl Metric {
    pub fn render(self, value: i128) -> String {
        match self {
            Metric::Lamports => {
                let lamports = value.max(0) as u64;
                format!(
                    "{}.{:09} SOL",
                    lamports / 1_000_000_000,
                    lamports % 1_000_000_000
                )
            }
            Metric::MicroUsd => Usd::from_micro(value.max(0) as u128).format_dollars(),
            Metric::Health => format_health(value.max(0) as u64),
            Metric::Raw => value.to_string(),
        }
    }
}

/// An exact value shared by every fixture in a cluster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub field: String,
    pub value: String,
}

/// The span a numeric parameter covers across a cluster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumericRange {
    pub field: String,
    pub metric: Metric,
    pub min: i128,
    pub max: i128,
    pub min_display: String,
    pub max_display: String,
    /// Whether the span is tight enough to describe a trigger rather than the
    /// corpus. Wide ranges are still reported here, but are not promoted to
    /// common conditions in the human-readable output.
    pub informative: bool,
}

impl NumericRange {
    fn new(field: &str, metric: Metric, values: &[i128]) -> Option<Self> {
        let min = *values.iter().min()?;
        let max = *values.iter().max()?;
        Some(NumericRange {
            field: field.to_string(),
            metric,
            min,
            max,
            min_display: metric.render(min),
            max_display: metric.render(max),
            informative: is_informative(min, max),
        })
    }
}

fn is_informative(min: i128, max: i128) -> bool {
    if min == max {
        return true;
    }
    let scale = max.abs().max(min.abs());
    if scale == 0 {
        return true;
    }
    ((max - min) * 10_000) / scale <= INFORMATIVE_SPREAD_BPS
}

/// The signature that decides whether two findings belong together.
///
/// Severity is deliberately absent: two unrelated regressions that happen to be
/// critical are not the same bug.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ClusterKey {
    consequences: Vec<EconomicConsequence>,
    action: &'static str,
    difference_kinds: Vec<&'static str>,
    changed_fields: Vec<String>,
    outcome_transition: (bool, bool),
    liquidation_transitions: Vec<(String, bool, bool)>,
}

fn cluster_key(fixture: &Fixture, diff: &StateDiff, economics: &FixtureEconomics) -> ClusterKey {
    let mut difference_kinds = BTreeSet::new();
    let mut changed_fields = BTreeSet::new();
    for difference in diff.outcome_differences() {
        difference_kinds.insert(difference.kind());
        if let Difference::FieldChanged { account, field, .. } = difference {
            changed_fields.insert(format!("{account}.{field}"));
        }
    }
    ClusterKey {
        consequences: economics.consequences.clone(),
        action: instruction_name(&fixture.instruction.data),
        difference_kinds: difference_kinds.into_iter().collect(),
        changed_fields: changed_fields.into_iter().collect(),
        outcome_transition: (diff.v1.success, diff.v2.success),
        liquidation_transitions: {
            let mut transitions: Vec<_> = diff
                .outcome_differences()
                .into_iter()
                .filter_map(|d| {
                    if let Difference::LiquidationStatusChanged {
                        account, v1, v2, ..
                    } = d
                    {
                        Some((account.clone(), *v1, *v2))
                    } else {
                        None
                    }
                })
                .collect();
            transitions.sort();
            transitions
        },
    }
}

/// Opaque, comparable identity of a finding's regression class.
///
/// `crate::shrink` uses this to decide whether a mutated fixture still
/// reproduces *the same* regression. Comparing signatures rather than, say,
/// severity is what stops the shrinker from silently wandering into a different
/// bug and reporting it as a minimized version of the original.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegressionSignature(ClusterKey);

/// Compute the regression class of a single finding.
pub fn signature(
    fixture: &Fixture,
    diff: &StateDiff,
    economics: &FixtureEconomics,
) -> RegressionSignature {
    RegressionSignature(cluster_key(fixture, diff, economics))
}

/// A minimized counterexample, produced by `crate::shrink`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimizedCase {
    /// Fixture the search started from.
    pub derived_from: String,
    pub collateral_lamports: u64,
    pub collateral_display: String,
    pub debt_micro_usd: u64,
    pub debt_display: String,
    pub collateral_value_usd: Usd,
    pub debt_value_usd: Usd,
    pub v1_health: String,
    pub v2_health: String,
    pub v1_liquidatable: bool,
    pub v2_liquidatable: bool,
    pub v1_outcome: String,
    pub v2_outcome: String,
    /// Executions spent searching. Reported so the cost is visible and the
    /// search is auditable.
    pub probes: usize,
    /// Reductions actually accepted.
    pub reductions: usize,
}

/// A group of findings that share a trigger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionCluster {
    /// Stable, human-typeable identifier, e.g. `newly-liquidatable`.
    pub id: String,
    pub consequences: Vec<EconomicConsequence>,
    pub action: String,
    pub difference_kinds: Vec<String>,
    pub critical: bool,
    pub fixture_ids: Vec<String>,
    pub collateral_value_usd: Usd,
    pub debt_value_usd: Usd,
    pub common_conditions: Vec<Condition>,
    pub ranges: Vec<NumericRange>,
    pub representative_fixture_id: String,
    pub minimized_counterexample: Option<MinimizedCase>,
}

impl RegressionCluster {
    pub fn fixture_count(&self) -> usize {
        self.fixture_ids.len()
    }

    /// Conditions plus the ranges tight enough to be worth stating.
    pub fn informative_ranges(&self) -> Vec<&NumericRange> {
        self.ranges.iter().filter(|r| r.informative).collect()
    }
}

/// Per-fixture values the cluster summarises.
struct Member<'a> {
    fixture: &'a Fixture,
    diff: &'a StateDiff,
    economics: &'a FixtureEconomics,
}

impl Member<'_> {
    fn v1_health(&self) -> Option<u64> {
        self.economics.v1_post.as_ref().map(|p| p.health_factor)
    }
    fn v2_health(&self) -> Option<u64> {
        self.economics.v2_post.as_ref().map(|p| p.health_factor)
    }
}

/// Count the properties that make a fixture an awkward example.
///
/// Used only for representative selection: a fixture with fewer unusual traits
/// is easier to reason about, so it wins.
fn edge_property_count(member: &Member<'_>) -> u32 {
    let baseline = &member.economics.baseline;
    let mut count = 0;
    if baseline.collateral_amount >= 1_000 * 1_000_000_000 {
        count += 1; // whale-scale position
    }
    if baseline.collateral_amount < 1_000_000_000 {
        count += 1; // sub-1-SOL dust
    }
    if baseline.debt_amount == 0 {
        count += 1; // no debt is a degenerate risk case
    }
    if !member.diff.v1.success {
        count += 1; // the baseline version already failed
    }
    count
}

/// Deterministic ordering key for representative selection, best first.
///
/// Follows the stated preference order: fewest awkward properties, then
/// smallest economic magnitude, then closest to the decision boundary, then
/// fixture ID as a total-order tie-breaker so the result never depends on
/// iteration order.
fn representative_rank(member: &Member<'_>) -> (u32, u128, u64, String) {
    let baseline = &member.economics.baseline;
    let magnitude = baseline
        .collateral_value_usd
        .micro()
        .saturating_add(baseline.debt_value_usd.micro());
    let boundary_distance = member
        .v1_health()
        .map(|health| health.abs_diff(fixture_lending_interface::HEALTH_SCALE))
        .unwrap_or(u64::MAX);
    (
        edge_property_count(member),
        magnitude,
        boundary_distance,
        member.fixture.id.clone(),
    )
}

fn slug(text: &str) -> String {
    text.replace('_', "-")
}

fn conditions_and_ranges(members: &[Member<'_>]) -> (Vec<Condition>, Vec<NumericRange>) {
    let mut conditions = Vec::new();

    // Categorical: reported only when every member agrees exactly.
    let mut shared = |field: &str, values: Vec<String>| {
        if let Some(first) = values.first() {
            if values.iter().all(|v| v == first) {
                conditions.push(Condition {
                    field: field.to_string(),
                    value: first.clone(),
                });
            }
        }
    };

    shared(
        "action",
        members
            .iter()
            .map(|m| instruction_name(&m.fixture.instruction.data).to_string())
            .collect(),
    );
    shared(
        "collateral_asset",
        members.iter().map(|_| "SOL".to_string()).collect(),
    );
    shared(
        "fractional_collateral",
        members
            .iter()
            .map(|m| (m.economics.baseline.collateral_amount % 1_000_000_000 != 0).to_string())
            .collect(),
    );
    shared(
        "collateral_price",
        members
            .iter()
            .map(|m| m.economics.baseline.collateral_price_usd.format_dollars())
            .collect(),
    );
    shared(
        "v1_liquidatable",
        members
            .iter()
            .filter_map(|m| m.economics.v1_post.as_ref())
            .map(|p| p.liquidatable.to_string())
            .collect(),
    );
    shared(
        "v2_liquidatable",
        members
            .iter()
            .filter_map(|m| m.economics.v2_post.as_ref())
            .map(|p| p.liquidatable.to_string())
            .collect(),
    );
    shared(
        "transaction_outcome",
        members
            .iter()
            .map(|m| {
                format!(
                    "V1 {} / V2 {}",
                    if m.diff.v1.success {
                        "success"
                    } else {
                        "revert"
                    },
                    if m.diff.v2.success {
                        "success"
                    } else {
                        "revert"
                    }
                )
            })
            .collect(),
    );

    let mut ranges = Vec::new();
    let mut push_range = |field: &str, metric: Metric, values: Vec<i128>| {
        if values.len() == members.len() {
            if let Some(range) = NumericRange::new(field, metric, &values) {
                ranges.push(range);
            }
        }
    };

    push_range(
        "collateral",
        Metric::Lamports,
        members
            .iter()
            .map(|m| m.economics.baseline.collateral_amount as i128)
            .collect(),
    );
    push_range(
        "debt",
        Metric::MicroUsd,
        members
            .iter()
            .map(|m| m.economics.baseline.debt_amount as i128)
            .collect(),
    );
    push_range(
        "collateral_value",
        Metric::MicroUsd,
        members
            .iter()
            .map(|m| m.economics.baseline.collateral_value_usd.micro() as i128)
            .collect(),
    );
    // Health factors are skipped where any member carries no debt, since an
    // infinite health factor would make the range meaningless rather than wide.
    let finite = |values: Vec<Option<u64>>| -> Vec<i128> {
        if values
            .iter()
            .any(|v| v.is_none() || *v == Some(fixture_lending_interface::HEALTH_INFINITE))
        {
            Vec::new()
        } else {
            values.into_iter().flatten().map(|v| v as i128).collect()
        }
    };
    push_range(
        "health_factor_v1",
        Metric::Health,
        finite(members.iter().map(|m| m.v1_health()).collect()),
    );
    push_range(
        "health_factor_v2",
        Metric::Health,
        finite(members.iter().map(|m| m.v2_health()).collect()),
    );

    (conditions, ranges)
}

/// Group every changed fixture into regression clusters.
///
/// Ordering is deterministic: critical clusters first, then by descending
/// fixture count, then by ID.
pub fn build(
    fixtures: &[Fixture],
    diffs: &[StateDiff],
    economics: &[FixtureEconomics],
) -> Vec<RegressionCluster> {
    let fixtures_by_id: BTreeMap<&str, &Fixture> =
        fixtures.iter().map(|f| (f.id.as_str(), f)).collect();
    let economics_by_id: BTreeMap<&str, &FixtureEconomics> = economics
        .iter()
        .map(|e| (e.fixture_id.as_str(), e))
        .collect();

    let mut groups: BTreeMap<ClusterKey, Vec<Member<'_>>> = BTreeMap::new();
    for diff in diffs {
        let Some(fixture) = fixtures_by_id.get(diff.fixture_id.as_str()) else {
            continue;
        };
        let Some(economics) = economics_by_id.get(diff.fixture_id.as_str()) else {
            continue;
        };
        if economics.consequences.is_empty() {
            continue; // unaffected: nothing to cluster
        }
        groups
            .entry(cluster_key(fixture, diff, economics))
            .or_default()
            .push(Member {
                fixture,
                diff,
                economics,
            });
    }

    // Base slugs come from the consequence set; qualify with the action only
    // where that would otherwise collide, so common ids stay short.
    let mut base_counts: BTreeMap<String, usize> = BTreeMap::new();
    for key in groups.keys() {
        let base = key
            .consequences
            .iter()
            .map(|c| slug(c.as_str()))
            .collect::<Vec<_>>()
            .join("+");
        *base_counts.entry(base).or_default() += 1;
    }

    let mut clusters = Vec::new();
    let mut used_ids = BTreeMap::<String, usize>::new();
    for (key, mut members) in groups {
        members.sort_by_key(|m| m.fixture.id.clone());

        let base = key
            .consequences
            .iter()
            .map(|c| slug(c.as_str()))
            .collect::<Vec<_>>()
            .join("+");
        let id = if base_counts.get(&base).copied().unwrap_or(0) > 1 {
            format!("{base}--{}", slug(key.action))
        } else {
            base
        };

        // Distinct field/transition signatures can share consequence and action.
        // BTreeMap traversal gives deterministic suffixes without merging them.
        let count = used_ids.entry(id.clone()).or_default();
        *count += 1;
        let id = if *count == 1 {
            id
        } else {
            format!("{id}--{count}")
        };
        let (common_conditions, ranges) = conditions_and_ranges(&members);
        let representative = members
            .iter()
            .min_by_key(|m| representative_rank(m))
            .expect("groups are never empty");

        clusters.push(RegressionCluster {
            id,
            consequences: key.consequences.clone(),
            action: key.action.to_string(),
            difference_kinds: key.difference_kinds.iter().map(|k| k.to_string()).collect(),
            critical: members.iter().any(|m| m.economics.critical),
            fixture_ids: members.iter().map(|m| m.fixture.id.clone()).collect(),
            collateral_value_usd: members
                .iter()
                .map(|m| m.economics.baseline.collateral_value_usd)
                .sum(),
            debt_value_usd: members
                .iter()
                .map(|m| m.economics.baseline.debt_value_usd)
                .sum(),
            common_conditions,
            ranges,
            representative_fixture_id: representative.fixture.id.clone(),
            minimized_counterexample: None,
        });
    }

    clusters.sort_by(|a, b| {
        b.critical
            .cmp(&a.critical)
            .then(b.fixture_count().cmp(&a.fixture_count()))
            .then(a.id.cmp(&b.id))
    });
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_values_are_always_informative() {
        assert!(is_informative(100, 100));
        assert!(is_informative(0, 0));
    }

    #[test]
    fn tight_ranges_are_informative_and_wide_ones_are_not() {
        // $7,920 .. $7,960 is a 0.5% spread around a boundary.
        assert!(is_informative(7_920_000_000, 7_960_000_000));
        // $2,500 .. $7,960 describes the corpus, not a trigger.
        assert!(!is_informative(2_500_000_000, 7_960_000_000));
        // Exactly at the 20% cutoff.
        assert!(is_informative(8_000, 10_000));
        assert!(!is_informative(7_999, 10_000));
    }

    #[test]
    fn metrics_render_in_their_own_units() {
        assert_eq!(Metric::Lamports.render(99_500_000_000), "99.500000000 SOL");
        assert_eq!(Metric::MicroUsd.render(7_930_000_000), "$7,930.00");
        assert_eq!(Metric::Health.render(1_003_783), "1.003783");
        assert_eq!(Metric::Raw.render(-5), "-5");
    }

    #[test]
    fn ranges_capture_min_and_max() {
        let range = NumericRange::new(
            "debt",
            Metric::MicroUsd,
            &[7_940_000_000, 7_930_000_000, 7_960_000_000],
        )
        .expect("range");
        assert_eq!(range.min, 7_930_000_000);
        assert_eq!(range.max, 7_960_000_000);
        assert_eq!(range.min_display, "$7,930.00");
        assert_eq!(range.max_display, "$7,960.00");
        assert!(range.informative);
    }
}
