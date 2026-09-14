//! Structural comparison of two execution results.
//!
//! Protocol-agnostic: this module compares success, balances, account bytes,
//! CPI shape and compute. Where an account decodes to a known type it defers to
//! `interpret` for field names and economic meaning; where it does not, it falls
//! back to a raw byte difference rather than staying silent.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::executor::ExecutionResult;
use crate::interpret::{self, Decoded, FieldValue};
use crate::types::{Category, Fixture};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::High => "HIGH",
            Severity::Critical => "CRITICAL",
        }
    }
}

/// Compute deltas below this magnitude are background noise: any recompilation
/// moves compute a little, and a few hundred CU on a 200k budget changes
/// nothing operationally.
pub const COMPUTE_NOISE_PCT: f64 = 5.0;

/// Above this, a compute change is an operational risk in its own right - the
/// transaction may still succeed in isolation while becoming fragile inside a
/// larger transaction or under a tighter budget.
pub const COMPUTE_REGRESSION_PCT: f64 = 30.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Difference {
    /// The transaction succeeded under one version and failed under the other.
    SuccessChanged {
        v1_success: bool,
        v2_success: bool,
        v1_error: Option<String>,
        v2_error: Option<String>,
    },
    /// A position crossed the liquidation threshold in one version only.
    LiquidationStatusChanged {
        account: String,
        v1: bool,
        v2: bool,
        v1_health: String,
        v2_health: String,
    },
    /// A named field of a decoded account differs.
    FieldChanged {
        account: String,
        field: String,
        v1: String,
        v2: String,
        delta: Option<i128>,
        consequence: Option<String>,
    },
    /// An account changed but could not be decoded; reported as raw bytes.
    RawDataChanged {
        account: String,
        offset: usize,
        v1: String,
        v2: String,
    },
    /// Lamport balance of a watched account differs.
    BalanceChanged {
        account: String,
        v1: u64,
        v2: u64,
        delta: i128,
    },
    /// The cross-program invocation sequence differs.
    CpiChanged { v1: Vec<String>, v2: Vec<String> },
    ComputeChanged {
        v1: u64,
        v2: u64,
        delta: i64,
        pct: f64,
    },
}

impl Difference {
    pub fn severity(&self) -> Severity {
        match self {
            // Either direction is critical: a withdrawal that starts failing
            // strands users; one that starts succeeding bypasses a guard.
            Difference::SuccessChanged { .. } => Severity::Critical,
            Difference::LiquidationStatusChanged { .. } => Severity::Critical,
            Difference::BalanceChanged { .. } => Severity::High,
            Difference::CpiChanged { .. } => Severity::High,
            Difference::FieldChanged { field, .. } => match field.as_str() {
                "health_factor" | "collateral_amount" | "debt_amount" => Severity::High,
                _ => Severity::Warning,
            },
            Difference::RawDataChanged { .. } => Severity::Warning,
            Difference::ComputeChanged { pct, .. } => {
                let magnitude = pct.abs();
                if magnitude < COMPUTE_NOISE_PCT {
                    Severity::Info
                } else if magnitude < COMPUTE_REGRESSION_PCT {
                    Severity::Warning
                } else {
                    Severity::High
                }
            }
        }
    }

    pub fn is_compute_only(&self) -> bool {
        matches!(self, Difference::ComputeChanged { .. })
    }
}

/// Headline bucket for a fixture.
///
/// Compute is deliberately kept off this axis. Every recompilation moves
/// compute units, so folding them in would classify the entire corpus as
/// "changed" and bury the question the tool exists to answer: did anything
/// change for users, positions or capital? Compute is reported separately, and
/// is still escalated on its own merits when the shift is large enough to be an
/// operational risk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// Nothing at all differed, compute included.
    Identical,
    /// State, balances, outcome and CPI shape are identical; only compute moved.
    ComputeOnly,
    /// Something observable to a user or a position changed.
    Changed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateDiff {
    pub fixture_id: String,
    pub category: Category,
    pub scenario: String,
    pub notes: String,
    pub differences: Vec<Difference>,
    pub v1: ExecutionResult,
    pub v2: ExecutionResult,
}

impl StateDiff {
    /// Severity across every difference, compute included.
    pub fn max_severity(&self) -> Option<Severity> {
        self.differences.iter().map(Difference::severity).max()
    }

    /// Differences that are observable as state, balances, outcome or CPI shape.
    pub fn outcome_differences(&self) -> Vec<&Difference> {
        self.differences
            .iter()
            .filter(|d| !d.is_compute_only())
            .collect()
    }

    /// Severity of the behavioural change alone.
    pub fn outcome_severity(&self) -> Option<Severity> {
        self.outcome_differences()
            .into_iter()
            .map(Difference::severity)
            .max()
    }

    pub fn compute_delta(&self) -> Option<(u64, u64, f64)> {
        self.differences.iter().find_map(|d| match d {
            Difference::ComputeChanged { v1, v2, pct, .. } => Some((*v1, *v2, *pct)),
            _ => None,
        })
    }

    pub fn classification(&self) -> Classification {
        if self.differences.is_empty() {
            return Classification::Identical;
        }
        if self.outcome_differences().is_empty() {
            return Classification::ComputeOnly;
        }
        Classification::Changed
    }

    pub fn is_critical(&self) -> bool {
        self.outcome_severity() == Some(Severity::Critical)
    }

    /// Behavioural differences that carry weight, worst first.
    pub fn material_differences(&self) -> Vec<&Difference> {
        let mut out: Vec<&Difference> = self
            .outcome_differences()
            .into_iter()
            .filter(|d| d.severity() > Severity::Info)
            .collect();
        out.sort_by_key(|d| std::cmp::Reverse(d.severity()));
        out
    }
}

fn first_difference_offset(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}

/// Compare two executions of the same fixture.
pub fn compare(fixture: &Fixture, v1: ExecutionResult, v2: ExecutionResult) -> StateDiff {
    let mut differences = Vec::new();

    if v1.success != v2.success {
        differences.push(Difference::SuccessChanged {
            v1_success: v1.success,
            v2_success: v2.success,
            v1_error: v1.error.clone(),
            v2_error: v2.error.clone(),
        });
    }

    let labels: BTreeSet<&String> = v1.accounts.keys().chain(v2.accounts.keys()).collect();
    for label in labels {
        let (before, after) = match (v1.accounts.get(label), v2.accounts.get(label)) {
            (Some(a), Some(b)) => (a, b),
            // An account existing under one version only is a structural change
            // worth surfacing as a raw difference.
            (a, b) => {
                differences.push(Difference::RawDataChanged {
                    account: label.clone(),
                    offset: 0,
                    v1: a
                        .map(|x| format!("{} lamports", x.lamports))
                        .unwrap_or("<absent>".into()),
                    v2: b
                        .map(|x| format!("{} lamports", x.lamports))
                        .unwrap_or("<absent>".into()),
                });
                continue;
            }
        };

        if before.lamports != after.lamports {
            differences.push(Difference::BalanceChanged {
                account: label.clone(),
                v1: before.lamports,
                v2: after.lamports,
                delta: after.lamports as i128 - before.lamports as i128,
            });
        }

        if before.data == after.data {
            continue;
        }

        let decoded_before = interpret::decode(&before.data);
        let decoded_after = interpret::decode(&after.data);

        let comparable = !matches!(decoded_before, Decoded::Opaque)
            && std::mem::discriminant(&decoded_before) == std::mem::discriminant(&decoded_after);

        if !comparable {
            differences.push(Difference::RawDataChanged {
                account: label.clone(),
                offset: first_difference_offset(&before.data, &after.data),
                v1: crate::hexfmt::encode(&before.data),
                v2: crate::hexfmt::encode(&after.data),
            });
            continue;
        }

        let fields_before = interpret::fields(&decoded_before);
        let fields_after = interpret::fields(&decoded_after);
        for ((name, value_before), (_, value_after)) in
            fields_before.iter().zip(fields_after.iter())
        {
            if value_before == value_after {
                continue;
            }
            let delta = match (value_before.numeric(), value_after.numeric()) {
                (Some(a), Some(b)) => Some(b - a),
                _ => None,
            };
            differences.push(Difference::FieldChanged {
                account: label.clone(),
                field: (*name).to_string(),
                v1: value_before.render(),
                v2: value_after.render(),
                delta,
                consequence: interpret::explain(name, value_before, value_after),
            });

            // The headline economic event: a threshold crossing.
            if let (FieldValue::Health(a), FieldValue::Health(b)) = (value_before, value_after) {
                let was = interpret::is_liquidatable(*a);
                let now = interpret::is_liquidatable(*b);
                if was != now {
                    differences.push(Difference::LiquidationStatusChanged {
                        account: label.clone(),
                        v1: was,
                        v2: now,
                        v1_health: interpret::format_health(*a),
                        v2_health: interpret::format_health(*b),
                    });
                }
            }
        }
    }

    let cpi_v1: Vec<String> = v1
        .cpi_calls
        .iter()
        .map(|c| format!("{}@{}", c.program, c.stack_height))
        .collect();
    let cpi_v2: Vec<String> = v2
        .cpi_calls
        .iter()
        .map(|c| format!("{}@{}", c.program, c.stack_height))
        .collect();
    if cpi_v1 != cpi_v2 {
        differences.push(Difference::CpiChanged {
            v1: cpi_v1,
            v2: cpi_v2,
        });
    }

    if let (Some(a), Some(b)) = (v1.compute_units, v2.compute_units) {
        if a != b {
            let pct = if a == 0 {
                100.0
            } else {
                (b as f64 - a as f64) / a as f64 * 100.0
            };
            differences.push(Difference::ComputeChanged {
                v1: a,
                v2: b,
                delta: b as i64 - a as i64,
                pct,
            });
        }
    }

    StateDiff {
        fixture_id: fixture.id.clone(),
        category: fixture.category,
        scenario: fixture.scenario.clone(),
        notes: fixture.notes.clone(),
        differences,
        v1,
        v2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering_is_meaningful() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }

    #[test]
    fn small_compute_drift_is_noise() {
        let difference = Difference::ComputeChanged {
            v1: 10_000,
            v2: 10_100,
            delta: 100,
            pct: 1.0,
        };
        assert_eq!(difference.severity(), Severity::Info);
    }

    #[test]
    fn large_compute_drift_escalates() {
        let difference = Difference::ComputeChanged {
            v1: 140_000,
            v2: 240_000,
            delta: 100_000,
            pct: 71.4,
        };
        assert_eq!(difference.severity(), Severity::High);
    }

    #[test]
    fn compute_only_diffs_are_not_behavioural_changes() {
        let execution = |cu: u64| crate::executor::ExecutionResult {
            version: "x".into(),
            success: true,
            error: None,
            compute_units: Some(cu),
            fee: 5000,
            logs: vec![],
            cpi_calls: vec![],
            accounts: Default::default(),
        };
        let diff = StateDiff {
            fixture_id: "f".into(),
            category: Category::Healthy,
            scenario: "s".into(),
            notes: "n".into(),
            differences: vec![Difference::ComputeChanged {
                v1: 4205,
                v2: 3968,
                delta: -237,
                pct: -5.64,
            }],
            v1: execution(4205),
            v2: execution(3968),
        };
        assert_eq!(diff.classification(), Classification::ComputeOnly);
        assert!(!diff.is_critical());
        assert_eq!(diff.outcome_severity(), None);
    }
}
