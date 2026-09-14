//! Report rendering: human-readable text and machine-readable JSON.
//!
//! The JSON form is the contract a CI gate would eventually read; the text form
//! is what a developer reads in a terminal. Both are produced from the same
//! `StateDiff` values, so they cannot disagree.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::cluster::{self, RegressionCluster};
use crate::diff::{Classification, Difference, Severity, StateDiff};
use crate::impact::{
    evaluate_all, summarize, EconomicConsequence, EconomicImpactSummary, FixtureEconomics,
};
use crate::types::{Category, Fixture};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategorySummary {
    pub tested: usize,
    pub identical: usize,
    pub compute_only: usize,
    pub changed: usize,
    pub critical: usize,
}

/// Compute is tracked on its own axis; see `diff::Classification`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeSummary {
    pub fixtures_with_delta: usize,
    /// Basis points; see [`crate::diff::format_bps`].
    pub min_pct_bps: i32,
    pub max_pct_bps: i32,
    /// Fixtures whose compute moved far enough to be an operational risk.
    pub above_regression_threshold: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub fixtures_tested: usize,
    /// No difference whatsoever, compute included.
    pub identical: usize,
    /// Behaviourally identical; only compute moved.
    pub compute_only: usize,
    /// Behaviourally identical, whether or not compute moved.
    pub outcome_identical: usize,
    pub changed: usize,
    pub critical: usize,
    pub high: usize,
    pub warning: usize,
    pub compute: ComputeSummary,
    pub by_category: BTreeMap<String, CategorySummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub program_id: String,
    pub v1_artifact: String,
    pub v2_artifact: String,
    /// Behavioural result: what changed.
    pub summary: Summary,
    /// Economic result: how much it matters, across the corpus.
    pub economics: EconomicImpactSummary,
    /// Per-position economic detail, parallel to `diffs` and matched by ID.
    pub fixture_economics: Vec<FixtureEconomics>,
    /// Findings grouped by shared trigger, with common conditions and a
    /// representative example. Minimized counterexamples are filled in by
    /// `crate::minimize_clusters`, which needs to execute and so cannot run
    /// during construction.
    pub clusters: Vec<RegressionCluster>,
    pub diffs: Vec<StateDiff>,
}

impl Report {
    pub fn new(
        program_id: String,
        v1_artifact: String,
        v2_artifact: String,
        fixtures: &[Fixture],
        diffs: Vec<StateDiff>,
    ) -> Self {
        let mut summary = Summary {
            fixtures_tested: diffs.len(),
            ..Default::default()
        };
        for category in Category::ALL {
            summary
                .by_category
                .insert(category.as_str().to_string(), CategorySummary::default());
        }

        let mut min_pct_bps = i32::MAX;
        let mut max_pct_bps = i32::MIN;

        for diff in &diffs {
            let entry = summary
                .by_category
                .entry(diff.category.as_str().to_string())
                .or_default();
            entry.tested += 1;

            if let Some((_, _, pct_bps)) = diff.compute_delta() {
                summary.compute.fixtures_with_delta += 1;
                min_pct_bps = min_pct_bps.min(pct_bps);
                max_pct_bps = max_pct_bps.max(pct_bps);
                if pct_bps.abs() >= crate::diff::COMPUTE_REGRESSION_BPS {
                    summary.compute.above_regression_threshold += 1;
                }
            }

            match diff.classification() {
                Classification::Identical => {
                    summary.identical += 1;
                    summary.outcome_identical += 1;
                    entry.identical += 1;
                }
                Classification::ComputeOnly => {
                    summary.compute_only += 1;
                    summary.outcome_identical += 1;
                    entry.compute_only += 1;
                }
                Classification::Changed => {
                    summary.changed += 1;
                    entry.changed += 1;
                    match diff.outcome_severity() {
                        Some(Severity::Critical) => {
                            summary.critical += 1;
                            entry.critical += 1;
                        }
                        Some(Severity::High) => summary.high += 1,
                        _ => summary.warning += 1,
                    }
                }
            }
        }

        if summary.compute.fixtures_with_delta > 0 {
            summary.compute.min_pct_bps = min_pct_bps;
            summary.compute.max_pct_bps = max_pct_bps;
        }

        // Empty categories only add noise.
        summary.by_category.retain(|_, v| v.tested > 0);

        let fixture_economics = evaluate_all(fixtures, &diffs);
        let economics = summarize(&fixture_economics);
        let clusters = cluster::build(fixtures, &diffs, &fixture_economics);

        Self {
            program_id,
            v1_artifact,
            v2_artifact,
            summary,
            economics,
            fixture_economics,
            clusters,
            diffs,
        }
    }

    pub fn cluster(&self, id: &str) -> Option<&RegressionCluster> {
        let id = id.strip_prefix("regression-group:").unwrap_or(id);
        self.clusters.iter().find(|c| c.id == id)
    }

    pub fn diff_for(&self, fixture_id: &str) -> Option<&StateDiff> {
        self.diffs.iter().find(|d| d.fixture_id == fixture_id)
    }

    pub fn economics_for(&self, fixture_id: &str) -> Option<&FixtureEconomics> {
        self.fixture_economics
            .iter()
            .find(|e| e.fixture_id == fixture_id)
    }

    pub fn critical(&self) -> Vec<&StateDiff> {
        self.diffs.iter().filter(|d| d.is_critical()).collect()
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}

fn render_difference(difference: &Difference) -> Vec<String> {
    let tag = difference.severity().as_str();
    match difference {
        Difference::SuccessChanged {
            v1_success,
            v2_success,
            v1_error,
            v2_error,
        } => {
            let describe = |ok: bool, err: &Option<String>| {
                if ok {
                    "success".to_string()
                } else {
                    format!("FAILED - {}", err.as_deref().unwrap_or("unknown error"))
                }
            };
            vec![format!(
                "{tag:<8}  transaction outcome\n              V1: {}\n              V2: {}",
                describe(*v1_success, v1_error),
                describe(*v2_success, v2_error)
            )]
        }
        Difference::LiquidationStatusChanged {
            account,
            v1,
            v2,
            v1_health,
            v2_health,
        } => vec![format!(
            "{tag:<8}  {account}.liquidatable  {v1} -> {v2}\n              health factor {v1_health} -> {v2_health}"
        )],
        Difference::FieldChanged {
            account,
            field,
            v1,
            v2,
            delta,
            consequence,
        } => {
            let mut line = format!("{tag:<8}  {account}.{field}  {v1} -> {v2}");
            if let Some(delta) = delta {
                line.push_str(&format!("  (delta {delta})"));
            }
            if let Some(consequence) = consequence {
                line.push_str(&format!("\n              {consequence}"));
            }
            vec![line]
        }
        Difference::RawDataChanged {
            account,
            offset,
            v1,
            v2,
        } => {
            let truncate = |s: &String| {
                if s.len() > 48 {
                    format!("{}...", &s[..48])
                } else {
                    s.clone()
                }
            };
            vec![format!(
                "{tag:<8}  {account} raw data differs at offset {offset}\n              V1: {}\n              V2: {}",
                truncate(v1),
                truncate(v2)
            )]
        }
        Difference::BalanceChanged {
            account,
            v1,
            v2,
            delta,
        } => vec![format!(
            "{tag:<8}  {account}.lamports  {v1} -> {v2}  (delta {delta})"
        )],
        Difference::CpiChanged { v1, v2 } => vec![format!(
            "{tag:<8}  CPI sequence changed\n              V1: {:?}\n              V2: {:?}",
            v1, v2
        )],
        Difference::ComputeChanged {
            v1,
            v2,
            delta,
            pct_bps,
        } => vec![format!(
            "{tag:<8}  compute units  {v1} -> {v2}  (delta {delta}, {})",
            crate::diff::format_bps(*pct_bps)
        )],
    }
}

fn render_fixture(diff: &StateDiff, economics: Option<&FixtureEconomics>, indent: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{indent}{}  [{}]\n",
        diff.fixture_id,
        diff.category.as_str()
    ));
    out.push_str(&format!("{indent}  action: {}\n", diff.scenario));
    for difference in diff.material_differences() {
        for line in render_difference(difference) {
            out.push_str(&format!("{indent}  {line}\n"));
        }
    }
    // Economic context, derived from fixture state only - no estimates.
    if let Some(economics) = economics {
        let baseline = &economics.baseline;
        out.push_str(&format!(
            "{indent}  position value: collateral {} ({}), debt {}, net {}\n",
            baseline.collateral_value_usd.format_dollars(),
            baseline.collateral_display,
            baseline.debt_value_usd.format_dollars(),
            baseline.net_value_usd.format_dollars(),
        ));
        for consequence in &economics.consequences {
            out.push_str(&format!(
                "{indent}  economic consequence: {}\n",
                consequence.describe()
            ));
        }
    }
    out
}

fn render_capital_row(label: &str, group: &crate::impact::CapitalGroup) -> String {
    format!(
        "  {:<28} {:>5}   collateral {:>16}   debt {:>16}\n",
        label,
        group.positions,
        group.collateral_value_usd.format_dollars(),
        group.debt_value_usd.format_dollars()
    )
}

/// One cluster, compact enough for a CI log.
fn render_cluster(index: usize, cluster: &RegressionCluster) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}. {:<38} {}\n",
        index + 1,
        cluster.id,
        if cluster.critical { "[CRITICAL]" } else { "" }
    ));
    out.push_str(&format!("   action:         {}\n", cluster.action));

    let shown: Vec<&str> = cluster
        .fixture_ids
        .iter()
        .take(3)
        .map(String::as_str)
        .collect();
    let remainder = cluster.fixture_count().saturating_sub(shown.len());
    out.push_str(&format!(
        "   fixtures:       {}  ({}{})\n",
        cluster.fixture_count(),
        shown.join(", "),
        if remainder > 0 {
            format!(", +{remainder} more")
        } else {
            String::new()
        }
    ));
    out.push_str(&format!(
        "   capital:        collateral {}   debt {}\n",
        cluster.collateral_value_usd.format_dollars(),
        cluster.debt_value_usd.format_dollars()
    ));

    out.push_str("   common conditions:\n");
    for condition in &cluster.common_conditions {
        out.push_str(&format!("     {} = {}\n", condition.field, condition.value));
    }
    for range in cluster.informative_ranges() {
        if range.min == range.max {
            out.push_str(&format!("     {} = {}\n", range.field, range.min_display));
        } else {
            out.push_str(&format!(
                "     {} in {} .. {}\n",
                range.field, range.min_display, range.max_display
            ));
        }
    }

    out.push_str(&format!(
        "   representative: {}\n",
        cluster.representative_fixture_id
    ));

    if let Some(case) = &cluster.minimized_counterexample {
        out.push_str(&format!(
            "   minimized counterexample ({} reductions in {} probes):\n",
            case.reductions, case.probes
        ));
        out.push_str(&format!(
            "     collateral: {} ({})\n",
            case.collateral_display,
            case.collateral_value_usd.format_dollars()
        ));
        out.push_str(&format!("     debt:       {}\n", case.debt_display));
        out.push_str(&format!(
            "     V1: {}, health {}, liquidatable {}\n",
            case.v1_outcome, case.v1_health, case.v1_liquidatable
        ));
        out.push_str(&format!(
            "     V2: {}, health {}, liquidatable {}\n",
            case.v2_outcome, case.v2_health, case.v2_liquidatable
        ));
    }
    out
}

/// Full detail for one cluster, used by `eplyx reproduce <cluster-id>`.
pub fn render_cluster_reproduction(report: &Report, cluster: &RegressionCluster) -> String {
    let mut out = String::new();
    out.push_str(&format!("REGRESSION GROUP  {}\n", cluster.id));
    out.push_str(&format!("{}\n\n", "=".repeat(70)));
    out.push_str(&format!(
        "consequences:   {}\n",
        cluster
            .consequences
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out.push_str(&format!("action:         {}\n", cluster.action));
    out.push_str(&format!(
        "difference kinds: {}\n",
        cluster.difference_kinds.join(", ")
    ));
    out.push_str(&format!("critical:       {}\n\n", cluster.critical));

    out.push_str(&format!(
        "Affected fixtures ({}):\n",
        cluster.fixture_count()
    ));
    for id in &cluster.fixture_ids {
        out.push_str(&format!("  {id}\n"));
    }

    out.push_str("\nCommon conditions:\n");
    for condition in &cluster.common_conditions {
        out.push_str(&format!("  {} = {}\n", condition.field, condition.value));
    }

    out.push_str("\nParameter ranges across the group:\n");
    for range in &cluster.ranges {
        out.push_str(&format!(
            "  {:<20} {} .. {}{}\n",
            range.field,
            range.min_display,
            range.max_display,
            if range.informative {
                ""
            } else {
                "   (too wide to be a condition)"
            }
        ));
    }

    out.push_str(&format!(
        "\nRepresentative fixture:\n  {}\n",
        cluster.representative_fixture_id
    ));
    if let Some(diff) = report.diff_for(&cluster.representative_fixture_id) {
        out.push_str(&format!("  action: {}\n", diff.scenario));
        for difference in diff.material_differences() {
            for line in render_difference(difference) {
                out.push_str(&format!("  {line}\n"));
            }
        }
    }

    match &cluster.minimized_counterexample {
        Some(case) => {
            out.push_str(&format!(
                "\nMinimized counterexample (from {}, {} reductions in {} probes):\n",
                case.derived_from, case.reductions, case.probes
            ));
            out.push_str(&format!(
                "  collateral: {} ({})\n",
                case.collateral_display,
                case.collateral_value_usd.format_dollars()
            ));
            out.push_str(&format!(
                "  debt:       {} ({})\n",
                case.debt_display,
                case.debt_value_usd.format_dollars()
            ));
            out.push_str(&format!(
                "\n  V1: {}\n      health {}, liquidatable {}\n",
                case.v1_outcome, case.v1_health, case.v1_liquidatable
            ));
            out.push_str(&format!(
                "  V2: {}\n      health {}, liquidatable {}\n",
                case.v2_outcome, case.v2_health, case.v2_liquidatable
            ));
            out.push_str(
                "\n  This is a witness, not a proof of minimality: the search is\n                   greedy, bounded, and varies only collateral and debt.\n",
            );
        }
        None => {
            out.push_str("\nNo minimized counterexample: no configured simplification preserved\nthe regression class.\n");
        }
    }
    out
}

pub fn render_text(report: &Report) -> String {
    let s = &report.summary;
    let e = &report.economics;
    let mut out = String::new();

    out.push_str("EPLYX UPGRADE IMPACT\n");
    out.push_str("====================\n\n");
    out.push_str(&format!("program:  {}\n", report.program_id));
    out.push_str(&format!("V1:       {}\n", report.v1_artifact));
    out.push_str(&format!("V2:       {}\n\n", report.v2_artifact));

    out.push_str("BEHAVIOUR\n");
    out.push_str(&format!("  Fixtures tested:    {}\n", s.fixtures_tested));
    out.push_str(&format!(
        "  Outcome identical:  {}   (state, balances, result and CPI shape unchanged)\n",
        s.outcome_identical
    ));
    out.push_str(&format!("  Outcome changed:    {}\n", s.changed));
    out.push_str(&format!("    critical:         {}\n", s.critical));
    out.push_str(&format!("    high:             {}\n", s.high));
    out.push_str(&format!("    warning:          {}\n", s.warning));

    out.push_str("\n  Compute units (tracked separately - any recompilation moves these)\n");
    if s.compute.fixtures_with_delta == 0 {
        out.push_str("    no differences\n");
    } else {
        out.push_str(&format!(
            "    {} of {} fixtures differ, range {} .. {}\n",
            s.compute.fixtures_with_delta,
            s.fixtures_tested,
            crate::diff::format_bps(s.compute.min_pct_bps),
            crate::diff::format_bps(s.compute.max_pct_bps)
        ));
        out.push_str(&format!(
            "    above the {} operational-risk threshold: {}\n",
            crate::diff::format_bps(crate::diff::COMPUTE_REGRESSION_BPS),
            s.compute.above_regression_threshold
        ));
    }

    out.push_str("\nECONOMIC COVERAGE  (synthetic corpus, valued from fixture state)\n");
    out.push_str(&format!(
        "  Positions valued:        {}\n",
        e.positions_valued
    ));
    out.push_str(&format!(
        "  Collateral represented:  {:>16}\n",
        e.total_collateral_value_usd.format_dollars()
    ));
    out.push_str(&format!(
        "  Debt represented:        {:>16}\n",
        e.total_debt_value_usd.format_dollars()
    ));
    out.push_str(&format!(
        "  Net represented:         {:>16}\n",
        e.total_net_value_usd.format_dollars()
    ));

    out.push_str("\nAFFECTED  (positions with a non-compute difference)\n");
    out.push_str(&render_capital_row("affected", &e.affected));
    out.push_str(&render_capital_row("of which critical", &e.critical));
    out.push_str(&format!(
        "  {:<28} {:>5}\n",
        "unaffected",
        e.unaffected_positions()
    ));

    out.push_str("\nBY ECONOMIC CONSEQUENCE\n");
    for consequence in EconomicConsequence::ALL {
        if let Some(group) = e.by_consequence.get(consequence.as_str()) {
            if group.positions > 0 {
                out.push_str(&render_capital_row(consequence.as_str(), group));
            }
        }
    }

    out.push_str("\nNEWLY LIQUIDATABLE  (healthy under V1, liquidatable under V2)\n");
    out.push_str(&format!(
        "  Positions:               {}\n",
        e.newly_liquidatable.positions
    ));
    out.push_str(&format!(
        "  Collateral:              {:>16}\n",
        e.newly_liquidatable.collateral_value_usd.format_dollars()
    ));
    out.push_str(&format!(
        "  Debt:                    {:>16}\n",
        e.newly_liquidatable.debt_value_usd.format_dollars()
    ));

    out.push_str("\nBy category\n");
    out.push_str(&format!(
        "  {:<22} {:>7} {:>10} {:>8} {:>9}\n",
        "category", "tested", "identical", "changed", "critical"
    ));
    for (name, c) in &s.by_category {
        out.push_str(&format!(
            "  {:<22} {:>7} {:>10} {:>8} {:>9}\n",
            name,
            c.tested,
            c.identical + c.compute_only,
            c.changed,
            c.critical
        ));
    }

    if !report.clusters.is_empty() {
        out.push_str(&format!(
            "\n\nREGRESSION CLUSTERS  ({})\n{}\n\n",
            report.clusters.len(),
            "-".repeat(70)
        ));
        for (index, cluster) in report.clusters.iter().enumerate() {
            out.push_str(&render_cluster(index, cluster));
            out.push('\n');
        }
    }

    let critical = report.critical();
    if !critical.is_empty() {
        out.push_str(&format!(
            "\n\nCRITICAL  ({} fixtures)\n{}\n\n",
            critical.len(),
            "-".repeat(70)
        ));
        for diff in &critical {
            out.push_str(&render_fixture(
                diff,
                report.economics_for(&diff.fixture_id),
                "",
            ));
            out.push_str(&format!("  why: {}\n\n", diff.notes));
        }
    }

    let other: Vec<&StateDiff> = report
        .diffs
        .iter()
        .filter(|d| d.classification() == Classification::Changed && !d.is_critical())
        .collect();
    if !other.is_empty() {
        out.push_str(&format!(
            "\nOTHER BEHAVIOURAL DIFFERENCES  ({} fixtures)\n{}\n\n",
            other.len(),
            "-".repeat(70)
        ));
        for diff in &other {
            out.push_str(&render_fixture(
                diff,
                report.economics_for(&diff.fixture_id),
                "",
            ));
            out.push('\n');
        }
    }

    if s.critical > 0 {
        out.push_str(&format!(
            "\nVERDICT: {} critical economic regression(s) detected. Do not deploy V2.\n",
            s.critical
        ));
        out.push_str(&format!(
            "         {} of {} positions affected; {} newly liquidatable ({} collateral).\n",
            e.affected.positions,
            e.positions_valued,
            e.newly_liquidatable.positions,
            e.newly_liquidatable.collateral_value_usd.format_dollars()
        ));
    } else if s.changed > 0 {
        out.push_str(
            "\nVERDICT: no threshold crossings, but behaviour changed. Review before deploying.\n",
        );
    } else {
        out.push_str("\nVERDICT: no behavioural differences detected across the corpus.\n");
    }
    out
}

/// Detailed single-fixture view, used by `eplyx reproduce`.
pub fn render_reproduction(diff: &StateDiff) -> String {
    let mut out = String::new();
    out.push_str(&format!("FIXTURE  {}\n", diff.fixture_id));
    out.push_str(&format!("{}\n\n", "=".repeat(70)));
    out.push_str(&format!("category: {}\n", diff.category.as_str()));
    out.push_str(&format!("action:   {}\n", diff.scenario));
    out.push_str(&format!("why:      {}\n\n", diff.notes));

    for (label, result) in [("V1", &diff.v1), ("V2", &diff.v2)] {
        out.push_str(&format!("--- {label} ({}) ---\n", result.version));
        out.push_str(&format!(
            "  outcome: {}\n",
            if result.success {
                "success".to_string()
            } else {
                format!("FAILED - {}", result.error.as_deref().unwrap_or("unknown"))
            }
        ));
        if let Some(cu) = result.compute_units {
            out.push_str(&format!("  compute units: {cu}\n"));
        }
        for (account, snapshot) in &result.accounts {
            if let crate::interpret::Decoded::Position(position) =
                crate::interpret::decode(&snapshot.data)
            {
                let economics = crate::interpret::economics(&position);
                out.push_str(&format!(
                    "  {account}: collateral {} ({}), debt {}, net {}, health {}, liquidatable {}\n",
                    economics.collateral_display,
                    economics.collateral_value_usd.format_dollars(),
                    economics.debt_value_usd.format_dollars(),
                    economics.net_value_usd.format_dollars(),
                    economics.health_display,
                    economics.liquidatable
                ));
            } else {
                out.push_str(&format!("  {account}: {} lamports\n", snapshot.lamports));
            }
        }
        out.push_str("  logs:\n");
        for line in &result.logs {
            out.push_str(&format!("    {line}\n"));
        }
        out.push('\n');
    }

    out.push_str("--- DIFFERENCES ---\n");
    if diff.differences.is_empty() {
        out.push_str("  none\n");
    }
    for difference in &diff.differences {
        for line in render_difference(difference) {
            out.push_str(&format!("  {line}\n"));
        }
    }
    out
}
