//! Report rendering: human-readable text and machine-readable JSON.
//!
//! The JSON form is the contract a CI gate would eventually read; the text form
//! is what a developer reads in a terminal. Both are produced from the same
//! `StateDiff` values, so they cannot disagree.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::diff::{Classification, Difference, Severity, StateDiff};
use crate::types::Category;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategorySummary {
    pub tested: usize,
    pub identical: usize,
    pub compute_only: usize,
    pub changed: usize,
    pub critical: usize,
}

/// Compute is tracked on its own axis; see `diff::Classification`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComputeSummary {
    pub fixtures_with_delta: usize,
    pub min_pct: f64,
    pub max_pct: f64,
    /// Fixtures whose compute moved far enough to be an operational risk.
    pub above_regression_threshold: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    pub summary: Summary,
    pub diffs: Vec<StateDiff>,
}

impl Report {
    pub fn new(
        program_id: String,
        v1_artifact: String,
        v2_artifact: String,
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

        let mut min_pct = f64::INFINITY;
        let mut max_pct = f64::NEG_INFINITY;

        for diff in &diffs {
            let entry = summary
                .by_category
                .entry(diff.category.as_str().to_string())
                .or_default();
            entry.tested += 1;

            if let Some((_, _, pct)) = diff.compute_delta() {
                summary.compute.fixtures_with_delta += 1;
                min_pct = min_pct.min(pct);
                max_pct = max_pct.max(pct);
                if pct.abs() >= crate::diff::COMPUTE_REGRESSION_PCT {
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
            summary.compute.min_pct = min_pct;
            summary.compute.max_pct = max_pct;
        }

        // Empty categories only add noise.
        summary.by_category.retain(|_, v| v.tested > 0);

        Self {
            program_id,
            v1_artifact,
            v2_artifact,
            summary,
            diffs,
        }
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
        Difference::ComputeChanged { v1, v2, delta, pct } => vec![format!(
            "{tag:<8}  compute units  {v1} -> {v2}  (delta {delta}, {pct:+.2}%)"
        )],
    }
}

fn render_fixture(diff: &StateDiff, indent: &str) -> String {
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
    out
}

pub fn render_text(report: &Report) -> String {
    let s = &report.summary;
    let mut out = String::new();

    out.push_str("UPGRADE IMPACT REPORT\n");
    out.push_str("=====================\n\n");
    out.push_str(&format!("program:  {}\n", report.program_id));
    out.push_str(&format!("V1:       {}\n", report.v1_artifact));
    out.push_str(&format!("V2:       {}\n\n", report.v2_artifact));

    out.push_str(&format!("Fixtures tested:    {}\n", s.fixtures_tested));
    out.push_str(&format!(
        "Outcome identical:  {}   (state, balances, result and CPI shape unchanged)\n",
        s.outcome_identical
    ));
    out.push_str(&format!("Outcome changed:    {}\n", s.changed));
    out.push_str(&format!("  critical:         {}\n", s.critical));
    out.push_str(&format!("  high:             {}\n", s.high));
    out.push_str(&format!("  warning:          {}\n", s.warning));

    out.push_str("\nCompute units (tracked separately - any recompilation moves these)\n");
    if s.compute.fixtures_with_delta == 0 {
        out.push_str("  no differences\n");
    } else {
        out.push_str(&format!(
            "  {} of {} fixtures differ, range {:+.2}% .. {:+.2}%\n",
            s.compute.fixtures_with_delta, s.fixtures_tested, s.compute.min_pct, s.compute.max_pct
        ));
        out.push_str(&format!(
            "  above the {:.0}% operational-risk threshold: {}\n",
            crate::diff::COMPUTE_REGRESSION_PCT,
            s.compute.above_regression_threshold
        ));
    }

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

    let critical = report.critical();
    if !critical.is_empty() {
        out.push_str(&format!(
            "\n\nCRITICAL  ({} fixtures)\n{}\n\n",
            critical.len(),
            "-".repeat(70)
        ));
        for diff in &critical {
            out.push_str(&render_fixture(diff, ""));
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
            out.push_str(&render_fixture(diff, ""));
            out.push('\n');
        }
    }

    if s.critical > 0 {
        out.push_str(&format!(
            "\nVERDICT: {} critical economic regression(s) detected. Do not deploy V2.\n",
            s.critical
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
                    "  {account}: collateral {} SOL (${}), debt ${}, health {}, liquidatable {}\n",
                    economics.collateral_sol,
                    economics.collateral_value_usd,
                    economics.debt_usd,
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
