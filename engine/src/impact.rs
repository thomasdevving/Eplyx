//! Economic impact aggregation.
//!
//! The diff engine answers *did behaviour change?* This module answers *how much
//! does that change matter?* - how much collateral and debt the corpus
//! represents, how much of it sits in positions whose behaviour changed, and how
//! much became newly liquidatable.
//!
//! # Scope of the claim
//!
//! Every number here is derived deterministically from fixture account state by
//! integer arithmetic. None of it is an estimate, a projection, or a statement
//! about real deployed capital: this phase operates entirely on the controlled
//! synthetic corpus. The vocabulary is chosen to keep that distinction visible -
//! "collateral represented", "affected collateral", "newly liquidatable
//! collateral". There is no `capital_at_risk` field, because the engine has not
//! proven anything that deserves that name.
//!
//! # Layering
//!
//! This is a protocol-aware module, alongside `corpus` and `interpret`. It reads
//! the generic [`StateDiff`] but understands what a *position* is. `diff.rs` and
//! `executor.rs` remain free of any lending concept.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::diff::{Difference, StateDiff};
use crate::interpret::{position_economics, PositionEconomics};
use crate::money::{SignedUsd, Usd};
use crate::types::{Category, Fixture};

/// Label of the account a fixture's position lives in.
const POSITION_LABEL: &str = "position";

/// What changing behaviour means economically for one position.
///
/// Derived from observed differences only - never from parsing the scenario
/// text, which would couple this to instruction naming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicConsequence {
    /// Healthy under V1, liquidatable under V2.
    NewlyLiquidatable,
    /// Liquidatable under V1, healthy under V2.
    NoLongerLiquidatable,
    /// The transaction succeeded under V1 and reverts under V2 - in this corpus,
    /// a withdrawal the user can no longer make.
    TransactionNowReverts,
    /// The transaction was rejected under V1 and succeeds under V2 - in this
    /// corpus, a liquidation the protocol previously refused to permit.
    TransactionNowSucceeds,
    /// Balances or position fields differ without crossing a threshold or
    /// changing the transaction outcome.
    ValueChanged,
}

impl EconomicConsequence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewlyLiquidatable => "newly_liquidatable",
            Self::NoLongerLiquidatable => "no_longer_liquidatable",
            Self::TransactionNowReverts => "transaction_now_reverts",
            Self::TransactionNowSucceeds => "transaction_now_succeeds",
            Self::ValueChanged => "value_changed",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::NewlyLiquidatable => "position becomes newly liquidatable",
            Self::NoLongerLiquidatable => "position is no longer liquidatable",
            Self::TransactionNowReverts => "transaction succeeds under V1 and reverts under V2",
            Self::TransactionNowSucceeds => {
                "transaction is rejected under V1 and succeeds under V2"
            }
            Self::ValueChanged => "resulting values differ without a threshold crossing",
        }
    }

    pub const ALL: [EconomicConsequence; 5] = [
        Self::NewlyLiquidatable,
        Self::NoLongerLiquidatable,
        Self::TransactionNowReverts,
        Self::TransactionNowSucceeds,
        Self::ValueChanged,
    ];
}

/// Whether a position's capital counts as affected.
///
/// Compute-only differences never count. Recompilation always moves compute
/// units - including for the 89 fixtures whose state is byte-identical - so
/// letting compute mark capital as affected would report the entire corpus as
/// economically impacted, which would be false.
pub fn is_economically_affected(diff: &StateDiff) -> bool {
    diff.differences
        .iter()
        .any(|difference| !difference.is_compute_only())
}

/// Economic consequences observed for one fixture, deduplicated and ordered.
pub fn consequences(diff: &StateDiff) -> Vec<EconomicConsequence> {
    let mut out = Vec::new();
    for difference in &diff.differences {
        match difference {
            Difference::LiquidationStatusChanged { v1, v2, .. } => {
                out.push(if !*v1 && *v2 {
                    EconomicConsequence::NewlyLiquidatable
                } else {
                    EconomicConsequence::NoLongerLiquidatable
                });
            }
            Difference::SuccessChanged {
                v1_success,
                v2_success,
                ..
            } => {
                out.push(if *v1_success && !*v2_success {
                    EconomicConsequence::TransactionNowReverts
                } else {
                    EconomicConsequence::TransactionNowSucceeds
                });
            }
            _ => {}
        }
    }
    if out.is_empty() && is_economically_affected(diff) {
        out.push(EconomicConsequence::ValueChanged);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The economic record for one fixture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureEconomics {
    pub fixture_id: String,
    pub category: Category,
    /// Valuation of the position as the corpus holds it, before the transaction
    /// runs. This is the basis for every aggregate: it is what the corpus
    /// *represents*, valued by the correct (reference) arithmetic rather than by
    /// either candidate build.
    pub baseline: PositionEconomics,
    /// Post-execution valuation under each build, where the position survived.
    pub v1_post: Option<PositionEconomics>,
    pub v2_post: Option<PositionEconomics>,
    pub affected: bool,
    pub critical: bool,
    pub consequences: Vec<EconomicConsequence>,
}

impl FixtureEconomics {
    pub fn is_newly_liquidatable(&self) -> bool {
        self.consequences
            .contains(&EconomicConsequence::NewlyLiquidatable)
    }
}

/// Build the economic record for one fixture, if it carries a position account.
pub fn evaluate(fixture: &Fixture, diff: &StateDiff) -> Option<FixtureEconomics> {
    let baseline = fixture
        .account(POSITION_LABEL)
        .and_then(|named| position_economics(&named.account.data))?;

    Some(FixtureEconomics {
        fixture_id: fixture.id.clone(),
        category: fixture.category,
        baseline,
        v1_post: diff
            .v1
            .accounts
            .get(POSITION_LABEL)
            .and_then(|snapshot| position_economics(&snapshot.data)),
        v2_post: diff
            .v2
            .accounts
            .get(POSITION_LABEL)
            .and_then(|snapshot| position_economics(&snapshot.data)),
        affected: is_economically_affected(diff),
        critical: diff.is_critical(),
        consequences: consequences(diff),
    })
}

/// Positions and capital grouped under one heading.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapitalGroup {
    pub positions: usize,
    pub collateral_value_usd: Usd,
    pub debt_value_usd: Usd,
}

impl CapitalGroup {
    fn add(&mut self, economics: &FixtureEconomics) {
        self.positions += 1;
        self.collateral_value_usd = self
            .collateral_value_usd
            .saturating_add(economics.baseline.collateral_value_usd);
        self.debt_value_usd = self
            .debt_value_usd
            .saturating_add(economics.baseline.debt_value_usd);
    }
}

/// Corpus-wide economic impact.
///
/// All values are baseline valuations (see [`FixtureEconomics::baseline`]) and
/// describe the synthetic corpus only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicImpactSummary {
    /// Fixtures that carried a valuable position. Fixtures without one are
    /// excluded from every total rather than counted as zero.
    pub positions_valued: usize,

    // --- what the corpus covers ---
    pub total_collateral_value_usd: Usd,
    pub total_debt_value_usd: Usd,
    pub total_net_value_usd: SignedUsd,

    // --- what changed ---
    pub affected: CapitalGroup,
    pub critical: CapitalGroup,
    pub newly_liquidatable: CapitalGroup,

    /// Every consequence, including those with no positions, so that a consumer
    /// sees a stable set of keys.
    pub by_consequence: BTreeMap<String, CapitalGroup>,
}

impl EconomicImpactSummary {
    pub fn unaffected_positions(&self) -> usize {
        self.positions_valued
            .saturating_sub(self.affected.positions)
    }
}

/// Aggregate per-fixture records into the corpus summary.
pub fn summarize(entries: &[FixtureEconomics]) -> EconomicImpactSummary {
    let mut summary = EconomicImpactSummary {
        positions_valued: entries.len(),
        ..Default::default()
    };
    for consequence in EconomicConsequence::ALL {
        summary
            .by_consequence
            .insert(consequence.as_str().to_string(), CapitalGroup::default());
    }

    for economics in entries {
        summary.total_collateral_value_usd = summary
            .total_collateral_value_usd
            .saturating_add(economics.baseline.collateral_value_usd);
        summary.total_debt_value_usd = summary
            .total_debt_value_usd
            .saturating_add(economics.baseline.debt_value_usd);

        if economics.affected {
            summary.affected.add(economics);
        }
        if economics.critical {
            summary.critical.add(economics);
        }
        if economics.is_newly_liquidatable() {
            summary.newly_liquidatable.add(economics);
        }
        for consequence in &economics.consequences {
            if let Some(group) = summary.by_consequence.get_mut(consequence.as_str()) {
                group.add(economics);
            }
        }
    }

    summary.total_net_value_usd = summary
        .total_collateral_value_usd
        .signed_sub(summary.total_debt_value_usd);
    summary
}

/// Build per-fixture records for a whole run. Fixtures and diffs are matched by
/// ID rather than by position, so ordering differences cannot silently misalign
/// a valuation with the wrong diff.
pub fn evaluate_all(fixtures: &[Fixture], diffs: &[StateDiff]) -> Vec<FixtureEconomics> {
    let by_id: BTreeMap<&str, &Fixture> = fixtures.iter().map(|f| (f.id.as_str(), f)).collect();
    diffs
        .iter()
        .filter_map(|diff| {
            by_id
                .get(diff.fixture_id.as_str())
                .and_then(|fixture| evaluate(fixture, diff))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::ExecutionResult;
    use crate::types::AccountSnapshot;
    use fixture_lending_interface::{Position, ACCOUNT_TAG_POSITION};

    fn position(collateral: u64, debt: u64, health: u64) -> Position {
        Position {
            tag: ACCOUNT_TAG_POSITION,
            version: 1,
            owner: [1u8; 32],
            market: [2u8; 32],
            collateral_amount: collateral,
            debt_amount: debt,
            collateral_price: 100_000_000,
            liquidation_threshold_bps: 8_000,
            max_ltv_bps: 7_500,
            health_factor: health,
            last_update_slot: 0,
        }
    }

    fn snapshot(p: &Position) -> AccountSnapshot {
        AccountSnapshot {
            lamports: 1_000_000,
            owner: "11111111111111111111111111111111".into(),
            data: borsh::to_vec(p).unwrap(),
            executable: false,
            rent_epoch: 0,
        }
    }

    fn execution(p: &Position) -> ExecutionResult {
        let mut accounts = std::collections::BTreeMap::new();
        accounts.insert(POSITION_LABEL.to_string(), snapshot(p));
        ExecutionResult {
            version: "x".into(),
            success: true,
            error: None,
            compute_units: Some(1000),
            fee: 5000,
            logs: vec![],
            cpi_calls: vec![],
            accounts,
        }
    }

    fn diff_with(differences: Vec<Difference>, v1: &Position, v2: &Position) -> StateDiff {
        StateDiff {
            fixture_id: "f".into(),
            category: Category::Boundary,
            scenario: "s".into(),
            notes: "n".into(),
            differences,
            v1: execution(v1),
            v2: execution(v2),
        }
    }

    fn economics_for(diff: &StateDiff, baseline: &Position) -> FixtureEconomics {
        FixtureEconomics {
            fixture_id: diff.fixture_id.clone(),
            category: diff.category,
            baseline: crate::interpret::economics(baseline),
            v1_post: None,
            v2_post: None,
            affected: is_economically_affected(diff),
            critical: diff.is_critical(),
            consequences: consequences(diff),
        }
    }

    #[test]
    fn compute_only_differences_do_not_affect_capital() {
        let p = position(99_500_000_000, 7_930_000_000, 1_003_783);
        let diff = diff_with(
            vec![Difference::ComputeChanged {
                v1: 100_000,
                v2: 105_000,
                delta: 5_000,
                pct_bps: 500,
            }],
            &p,
            &p,
        );
        assert!(!is_economically_affected(&diff));
        assert!(consequences(&diff).is_empty());

        let summary = summarize(&[economics_for(&diff, &p)]);
        assert_eq!(summary.affected.positions, 0);
        assert_eq!(summary.affected.collateral_value_usd, Usd::ZERO);
        assert_eq!(summary.affected.debt_value_usd, Usd::ZERO);
        // The position is still counted as represented.
        assert_eq!(summary.positions_valued, 1);
        assert_eq!(
            summary.total_collateral_value_usd,
            Usd::from_micro(9_950_000_000)
        );
    }

    #[test]
    fn unchanged_fixtures_do_not_affect_capital() {
        let p = position(100_000_000_000, 5_000_000_000, 1_600_000);
        let diff = diff_with(vec![], &p, &p);
        assert!(!is_economically_affected(&diff));
        let summary = summarize(&[economics_for(&diff, &p)]);
        assert_eq!(summary.affected.positions, 0);
        assert_eq!(summary.unaffected_positions(), 1);
    }

    #[test]
    fn a_threshold_crossing_contributes_and_is_counted_as_newly_liquidatable() {
        let p = position(99_500_000_000, 7_930_000_000, 1_003_783);
        let diff = diff_with(
            vec![Difference::LiquidationStatusChanged {
                account: POSITION_LABEL.into(),
                v1: false,
                v2: true,
                v1_health: "1.003783".into(),
                v2_health: "0.998738".into(),
            }],
            &p,
            &p,
        );
        assert!(is_economically_affected(&diff));
        assert_eq!(
            consequences(&diff),
            vec![EconomicConsequence::NewlyLiquidatable]
        );

        let summary = summarize(&[economics_for(&diff, &p)]);
        assert_eq!(summary.newly_liquidatable.positions, 1);
        assert_eq!(
            summary.newly_liquidatable.collateral_value_usd,
            Usd::from_micro(9_950_000_000)
        );
        assert_eq!(
            summary.newly_liquidatable.debt_value_usd,
            Usd::from_micro(7_930_000_000)
        );
        assert_eq!(summary.affected.positions, 1);
    }

    #[test]
    fn a_reverting_transaction_is_affected_but_not_newly_liquidatable() {
        let p = position(100_000_000_000, 7_430_000_000, 1_070_000);
        let diff = diff_with(
            vec![Difference::SuccessChanged {
                v1_success: true,
                v2_success: false,
                v1_error: None,
                v2_error: Some("LtvExceeded".into()),
            }],
            &p,
            &p,
        );
        assert_eq!(
            consequences(&diff),
            vec![EconomicConsequence::TransactionNowReverts]
        );
        let summary = summarize(&[economics_for(&diff, &p)]);
        assert_eq!(summary.affected.positions, 1);
        assert_eq!(summary.newly_liquidatable.positions, 0);
    }

    #[test]
    fn a_balance_change_alone_is_a_value_change() {
        let p = position(50_000_000_000, 1_000_000_000, 4_000_000);
        let diff = diff_with(
            vec![Difference::BalanceChanged {
                account: "owner".into(),
                v1: 10,
                v2: 20,
                delta: 10,
            }],
            &p,
            &p,
        );
        assert_eq!(consequences(&diff), vec![EconomicConsequence::ValueChanged]);
        assert!(is_economically_affected(&diff));
    }

    #[test]
    fn aggregate_equals_the_sum_of_included_positions() {
        let a = position(10_000_000_000, 1_000_000_000, 8_000_000); // $1,000 / $1,000
        let b = position(20_000_000_000, 2_000_000_000, 8_000_000); // $2,000 / $2,000
        let c = position(30_000_000_000, 3_000_000_000, 8_000_000); // $3,000 / $3,000

        let changed = Difference::BalanceChanged {
            account: "owner".into(),
            v1: 1,
            v2: 2,
            delta: 1,
        };
        let entries = vec![
            economics_for(&diff_with(vec![changed.clone()], &a, &a), &a),
            economics_for(&diff_with(vec![], &b, &b), &b),
            economics_for(&diff_with(vec![changed], &c, &c), &c),
        ];
        let summary = summarize(&entries);

        let expected_total: u128 = entries
            .iter()
            .map(|e| e.baseline.collateral_value_usd.micro())
            .sum();
        assert_eq!(summary.total_collateral_value_usd.micro(), expected_total);

        let expected_affected: u128 = entries
            .iter()
            .filter(|e| e.affected)
            .map(|e| e.baseline.collateral_value_usd.micro())
            .sum();
        assert_eq!(
            summary.affected.collateral_value_usd.micro(),
            expected_affected
        );
        assert_eq!(summary.affected.positions, 2);
        // a ($1,000) + c ($3,000)
        assert_eq!(
            summary.affected.collateral_value_usd,
            Usd::from_micro(4_000_000_000)
        );
        assert_eq!(
            summary.total_net_value_usd,
            Usd::from_micro(6_000_000_000).signed_sub(Usd::from_micro(6_000_000_000))
        );
    }

    #[test]
    fn consequence_keys_are_always_present() {
        let summary = summarize(&[]);
        for consequence in EconomicConsequence::ALL {
            assert!(summary.by_consequence.contains_key(consequence.as_str()));
        }
    }
}
