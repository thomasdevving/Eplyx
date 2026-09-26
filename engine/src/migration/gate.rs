//! A deployment decision over verified analytical findings, never new evidence.
use super::requirements::{Result as InvariantResult, Severity, Status};
use anyhow::{ensure, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    #[default]
    BlockOnly,
    Strict,
}

impl Policy {
    pub fn name(self) -> &'static str {
        match self {
            Self::BlockOnly => "block-only",
            Self::Strict => "strict",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Pass,
    Warn,
    Block,
}

impl Outcome {
    pub fn exit_code(self) -> u8 {
        if self == Self::Block {
            1
        } else {
            0
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "PASS WITH WARNINGS",
            Self::Block => "BLOCKED",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentGate {
    pub policy: Policy,
    pub outcome: Outcome,
    pub analytical_readiness: String,
    pub reasons: Vec<String>,
    /// Machine-readable reason codes. Absent from fixed-ratio conversion reports,
    /// so their saved bytes replay unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reason_codes: Vec<String>,
}

/// Effects of invariant findings on a gate: (blocked, warning, reasons, effective
/// type/status pairs).
type InvariantEffects = (bool, bool, Vec<String>, Vec<(String, Status)>);

/// The shared invariant rule: a blocking Violated finding blocks under every
/// policy; a blocking Indeterminate finding blocks under strict and warns under
/// block-only; warning findings warn. Returns (blocked, warning, reasons, types).
fn invariant_effects(report: &Value, policy: Policy) -> Result<InvariantEffects> {
    let mut blocked = false;
    let mut warning = false;
    let mut reasons = Vec::new();
    let mut effective = Vec::new();
    if let Some(findings) = report.get("invariants") {
        let findings: Vec<InvariantResult> = serde_json::from_value(findings.clone())?;
        for finding in findings {
            match (finding.severity, finding.status) {
                (Severity::Blocking, Status::Violated) => blocked = true,
                (Severity::Blocking, Status::Indeterminate) if policy == Policy::Strict => {
                    blocked = true
                }
                (_, Status::Violated | Status::Indeterminate) => warning = true,
                _ => continue,
            }
            reasons.push(format!(
                "invariant {} ({}) is {:?}: {}",
                finding.invariant_id, finding.invariant_type, finding.status, finding.explanation
            ));
            effective.push((finding.invariant_type.clone(), finding.status));
        }
    }
    Ok((blocked, warning, reasons, effective))
}

pub fn migration_invariant_code(invariant_type: &str) -> &'static str {
    match invariant_type {
        "candidate_binary_matches_package" => "CANDIDATE_BINARY_MISMATCH",
        "mint_identity_matches_spec" => "MINT_IDENTITY_MISMATCH",
        "required_authorities_match" => "REQUIRED_AUTHORITY_MISMATCH",
        "migration_arithmetic_matches_spec" => "MIGRATION_ARITHMETIC_MISMATCH",
        "no_unintended_account_changes" => "UNINTENDED_ACCOUNT_CHANGE",
        "failed_migrations_roll_back" => "ROLLBACK_VIOLATION",
        "supply_reconciles" => "SUPPLY_RECONCILIATION_FAILED",
        "reserve_never_underflows" => "RESERVE_UNDERFLOW",
        "reserve_covers_eligible_holders" => "INSUFFICIENT_RESERVE",
        "window_rules_hold" => "MIGRATION_WINDOW_INVALID",
        "extension_semantics_honored" => "UNSUPPORTED_TOKEN_EXTENSION",
        "no_unsupported_execution_succeeds" => "UNEXPECTED_MIGRATION_SUCCESS",
        "no_stress_case_deviates" => "STRESS_CASE_DEVIATION",
        "all_positive_holders_migrate" => "STRANDED_HOLDERS",
        _ => "INVARIANT_NOT_SATISFIED",
    }
}

pub fn evaluate_migration(report: &Value, policy: Policy) -> Result<DeploymentGate> {
    ensure!(
        report["transition_kind"] == "token_migration",
        "not a token migration report"
    );
    let mut blocked = false;
    let mut incomplete = false;
    let mut reasons = Vec::new();
    let mut codes = std::collections::BTreeSet::new();
    let mut statuses = Vec::new();
    for axis in ["mechanism", "funding", "population", "reconciliation"] {
        let entry = &report["readiness"][axis];
        let status = entry["status"].as_str().unwrap_or("");
        ensure!(
            matches!(status, "Ready" | "Incomplete" | "Blocked"),
            "invalid migration readiness: {axis}"
        );
        statuses.push(status.to_string());
        if status != "Ready" {
            if status == "Blocked" {
                blocked = true;
            } else {
                incomplete = true;
            }
            reasons.push(format!("{axis}_readiness is {status}"));
            for code in entry["codes"].as_array().into_iter().flatten() {
                if let Some(code) = code.as_str() {
                    codes.insert(code.to_string());
                }
            }
            for reason in entry["reasons"].as_array().into_iter().flatten() {
                if let Some(reason) = reason.as_str() {
                    reasons.push(reason.to_string());
                }
            }
        }
    }
    ensure!(
        report["official_transition"] == "NotTested",
        "unexpected official transition claim"
    );
    ensure!(
        report["funds_moved"] == false,
        "unexpected funds movement claim"
    );
    let (invariant_blocked, invariant_warning, invariant_reasons, effective) =
        invariant_effects(report, policy)?;
    blocked |= invariant_blocked;
    reasons.extend(invariant_reasons);
    for (kind, _) in &effective {
        codes.insert(migration_invariant_code(kind).to_string());
    }
    let expected_analytical = if statuses.iter().any(|s| s == "Blocked") {
        "Blocked"
    } else if statuses.iter().all(|s| s == "Ready") {
        "Ready"
    } else {
        "Incomplete"
    };
    let analytical = report["declared_preflight_status"].as_str().unwrap_or("");
    ensure!(
        analytical == expected_analytical,
        "analytical readiness is inconsistent"
    );
    let outcome = if blocked || (policy == Policy::Strict && incomplete) {
        Outcome::Block
    } else if incomplete || invariant_warning {
        Outcome::Warn
    } else {
        Outcome::Pass
    };
    Ok(DeploymentGate {
        policy,
        outcome,
        analytical_readiness: analytical.into(),
        reasons,
        reason_codes: codes.into_iter().collect(),
    })
}

pub fn evaluate_migration_with_counterexamples(
    report: &Value,
    policy: Policy,
    search: &crate::migration::search::SearchResult,
) -> Result<DeploymentGate> {
    ensure!(
        report["analysis_input_sha256"] == search.analysis_input_sha256
            && report["candidate_program_sha256"] == search.candidate_program_sha256
            && report["state"]["world_sha256"] == search.world_sha256
            && report["artifacts"]["migration.plan.json"] == search.plan_sha256
            && report["artifacts"]["stress.plan.json"] == search.stress_plan_sha256
            && search.official_transition == "NotTested"
            && !search.funds_moved,
        "counterexample finding is outside the exact package run scope"
    );
    let mut gate = evaluate_migration(report, policy)?;
    if !search.counterexamples.is_empty() {
        gate.outcome = Outcome::Block;
        gate.reasons.push(format!(
            "CounterexampleFinding: {} counterexamples within the recorded search domains; search grants no population or official-transition proof",
            search.counterexamples.len()
        ));
        if !gate
            .reason_codes
            .iter()
            .any(|c| c == "COUNTEREXAMPLE_FOUND")
        {
            gate.reason_codes.push("COUNTEREXAMPLE_FOUND".into());
            gate.reason_codes.sort();
        }
    }
    Ok(gate)
}

/// MAIN: known violations are 1; a strict gate blocked solely by missing evidence is 5.
pub fn exit_code(report: &Value, gate: &DeploymentGate) -> Result<u8> {
    if gate.outcome != Outcome::Block {
        return Ok(0);
    }
    let findings: Vec<InvariantResult> = serde_json::from_value(report["invariants"].clone())?;
    let known = findings
        .iter()
        .any(|f| f.severity == Severity::Blocking && f.status == Status::Violated)
        || ["mechanism", "funding", "population", "reconciliation"]
            .iter()
            .any(|axis| report["readiness"][axis]["status"] == "Blocked");
    let violations: Vec<super::report::RequirementViolation> =
        serde_json::from_value(report["requirement_violations"].clone())?;
    Ok(if known || !violations.is_empty() {
        1
    } else {
        5
    })
}
