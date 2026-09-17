//! Expectation review: measured reality, then the team's declaration about it.
//!
//! Eplyx measures what changed. The team declares whether that change was
//! intended. These are separate statements and the review keeps them separate:
//! a finding **never** loses its severity by being expected.
//!
//! ```text
//! CRITICAL / EXPECTED               declared, and inside its bounds
//! WARNING  / UNEXPECTED             nothing declares it
//! CRITICAL / EXPECTED_BUT_EXCEEDED  declared, but larger than declared
//! ```
//!
//! So there is no path from `critical` to `info`. A reviewer reading the report
//! still sees that withdrawals stopped working; what the declaration adds is
//! that somebody signed their name to it.
//!
//! # Stale and unevaluable are different failures
//!
//! When a declaration matches nothing, there are two very different reasons:
//!
//! ```text
//! the candidate no longer does it      -> STALE
//! this corpus cannot tell you          -> UNEVALUABLE
//! ```
//!
//! Both fail the gate, and they must not be confused. Reporting a coverage gap
//! as "your intentional change was reverted" would send a team looking through
//! their diff for something that is not there. Distinguishing them is why the
//! adapter reports [`EvaluableSubject`]s — what each observation is *able* to
//! measure — alongside the findings themselves. Without that the two are only
//! separable by guessing.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::diff::Severity;
use crate::expectations::{ExpectationFile, ExpectedChange};
use crate::semantics::{
    relative_delta, EvaluableSubject, FindingFingerprint, NamedFinding, RelativeDelta,
    UndefinedBound,
};

/// One finding, as measured on one observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedFinding {
    pub observation_id: String,
    /// Which economic entity this observation belonged to, where the adapter
    /// names one. Bounds on affected entities need it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    pub finding: NamedFinding,
}

/// What one observation was able to measure, whether or not anything changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationCoverage {
    pub observation_id: String,
    pub subjects: Vec<EvaluableSubject>,
}

/// How a declaration and reality line up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    /// Declared, and within every bound the declaration set.
    Expected,
    /// Measured, and nothing declares it.
    Unexpected,
    /// Declared, but the impact is larger or wider than declared.
    ExpectedButExceeded,
    /// Declared, measurable by this corpus, and not happening. The declaration
    /// leaves permission behind for behaviour that no longer exists.
    Stale,
    /// Declared, and this corpus cannot say whether it happened.
    Unevaluable,
}

impl ReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expected => "expected",
            Self::Unexpected => "unexpected",
            Self::ExpectedButExceeded => "expected_but_exceeded",
            Self::Stale => "stale",
            Self::Unevaluable => "unevaluable",
        }
    }

    /// Everything except [`ReviewStatus::Expected`] fails the gate.
    pub fn passes(self) -> bool {
        matches!(self, Self::Expected)
    }
}

/// A bound a declaration set, and what was actually measured against it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "bound", rename_all = "snake_case")]
pub enum BoundBreach {
    RelativeDelta { limit_bps: u32, observed_bps: i64 },
    AffectedObservations { limit: usize, observed: usize },
    AffectedEntities { limit: usize, observed: usize },
}

/// Why a declaration could not be judged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum UnevaluableCause {
    /// No observation in this corpus can measure the declared subject.
    SubjectNotCovered { subject: String },
    /// Some matching observation's bound has no defined value, so the bound
    /// as a whole cannot be said to hold.
    BoundUndefined {
        detail: String,
        /// How many matching observations could not be judged.
        unevaluable: usize,
        /// How many matching observations there were in total.
        matching: usize,
    },
}

/// One fingerprint, aggregated across the observations that produced it.
///
/// `severity` is **descriptive metadata**. It is not part of the finding's
/// identity, and the gate never consults it: every non-`Expected` status fails
/// regardless of severity, and an `Expected` one passes regardless of severity.
/// That separation is deliberate, because the current mapping is coarse - a 1 bp
/// change and a 3,000 bp change are both `High` - and a better severity model
/// must be able to land later without invalidating a single existing
/// `expected-changes.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedFinding {
    pub fingerprint: String,
    /// What the diff layer rated this. Never altered by review status.
    pub severity: Severity,
    pub status: ReviewStatus,
    pub observations: Vec<String>,
    pub entities: Vec<String>,
    /// How many observations in the corpus can measure this subject at all.
    ///
    /// Reported beside `observations` so a change that is real but *not
    /// uniform* is visible: "3 of 4 withdrawals now revert" reads very
    /// differently from "withdrawals now revert", and a declaration bounded at
    /// 4 would accept both.
    pub covered_observations: usize,
    /// Largest relative change seen, by magnitude, across the observations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_relative_delta_bps: Option<i64>,
    /// The declaration's stated reason, when one matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaches: Vec<BoundBreach>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unevaluable: Option<UnevaluableCause>,
}

/// A declaration that matched no finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmatchedExpectation {
    pub fingerprint: String,
    pub reason: String,
    pub status: ReviewStatus,
    /// How many observations could have measured this declaration. Zero is
    /// exactly why an unmatched declaration is unevaluable rather than stale.
    pub covered_observations: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unevaluable: Option<UnevaluableCause>,
}

/// Why the gate failed, so a caller can turn it into an exit code and a team
/// can tell "this upgrade contains a finding" from "Eplyx could not complete
/// the analysis".
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// The corpus and adapter produced no semantic coverage at all, so a pass
    /// would mean "we did not look", not "nothing changed".
    NoSemanticCoverage,
    /// A change the engine detected but the public vocabulary cannot name, so
    /// it cannot be declared and must not be silently accepted.
    UndeclarableChange,
    /// A declaration this corpus cannot judge. Eplyx cannot prove whether the
    /// approval still applies.
    UnevaluableExpectation,
    /// A declaration for behaviour that no longer happens. The approval file
    /// contains something that is no longer true.
    StaleExpectation,
    /// A change nothing declared, or one larger than declared. The candidate
    /// did something nobody approved.
    UndeclaredChange,
}

impl FailureReason {
    /// The process exit code this reason summarises to.
    ///
    /// Three different actions for a team, so three different codes:
    /// 1 is "the candidate did something you did not approve", 3 is "your
    /// approval file contains something that no longer happens", and 5 is
    /// "Eplyx cannot prove whether your approval still applies". Codes 2 and 4
    /// belong to failures that happen *before* review - a configuration or
    /// fidelity error, and a bundle or baseline incompatibility - and abort it.
    pub fn exit_code(self) -> u8 {
        match self {
            // The analysis could not be performed meaningfully, which is an
            // analysis failure rather than a verdict on the candidate.
            Self::NoSemanticCoverage => 2,
            Self::UnevaluableExpectation => 5,
            Self::StaleExpectation => 3,
            // Both are "the candidate did something nobody approved". One could
            // have been declared and was not; the other cannot be declared at
            // all yet.
            Self::UndeclaredChange | Self::UndeclarableChange => 1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoSemanticCoverage => "no_semantic_coverage",
            Self::UndeclarableChange => "undeclarable_change",
            Self::UnevaluableExpectation => "unevaluable_expectation",
            Self::StaleExpectation => "stale_expectation",
            Self::UndeclaredChange => "undeclared_change",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub findings: Vec<ReviewedFinding>,
    pub unmatched: Vec<UnmatchedExpectation>,
    /// Every reason the gate failed, most significant first. Empty means pass.
    ///
    /// All of them are reported. The exit code is only a deterministic summary
    /// of this list, never a replacement for it.
    pub failures: Vec<FailureReason>,
}

impl Review {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    /// One code for a shell, chosen by precedence: coverage uncertainty outranks
    /// a stale declaration, which outranks an undeclared change.
    ///
    /// Uncertainty wins because it says the analysis was incomplete, and a team
    /// that cannot tell an incomplete analysis from a clean finding will
    /// eventually act on the wrong one. The full list stays in `failures`.
    pub fn exit_code(&self) -> u8 {
        self.failures
            .first()
            .map(|reason| reason.exit_code())
            .unwrap_or(0)
    }

    pub fn count(&self, status: ReviewStatus) -> usize {
        self.findings.iter().filter(|f| f.status == status).count()
            + self.unmatched.iter().filter(|e| e.status == status).count()
    }
}

/// One fingerprint's worth of measurements, before review.
#[derive(Default)]
struct Aggregate {
    observations: BTreeSet<String>,
    entities: BTreeSet<String>,
    severity: Option<Severity>,
    /// Largest magnitude among the observations that *could* be measured.
    max_delta_bps: Option<i64>,
    /// Observations whose relative delta has no defined value, and why.
    ///
    /// Kept per observation rather than as a single flag because the report
    /// has to say how much of the evidence was unjudgeable: "1 of 10" is a
    /// different situation from "10 of 10".
    undefined: BTreeMap<String, UndefinedBound>,
}

/// Review measured findings against declared expectations.
///
/// `coverage` is what the corpus could have measured. It is what makes
/// [`ReviewStatus::Stale`] and [`ReviewStatus::Unevaluable`] separable, and
/// passing an empty slice means every unmatched declaration is unevaluable —
/// which is the honest answer when nothing reported its coverage.
pub fn review(
    observed: &[ObservedFinding],
    coverage: &[ObservationCoverage],
    expectations: &ExpectationFile,
) -> Review {
    let declared = expectations.by_fingerprint();

    // What this corpus is able to speak about at all, and how widely. Counted
    // from what each observation declares it can measure, never from what
    // happened to differ.
    let mut measurable: BTreeMap<EvaluableSubject, usize> = BTreeMap::new();
    for observation in coverage {
        for subject in observation
            .subjects
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
        {
            *measurable.entry(subject).or_default() += 1;
        }
    }

    let mut aggregates: BTreeMap<FindingFingerprint, Aggregate> = BTreeMap::new();
    for entry in observed {
        let aggregate = aggregates
            .entry(entry.finding.fingerprint.clone())
            .or_default();
        aggregate.observations.insert(entry.observation_id.clone());
        if let Some(entity) = &entity_of(entry) {
            aggregate.entities.insert(entity.clone());
        }
        aggregate.severity = Some(match aggregate.severity {
            Some(existing) if existing >= entry.finding.severity => existing,
            _ => entry.finding.severity,
        });
        match measured_delta(&entry.finding) {
            Some(RelativeDelta::Bps(bps)) => {
                let larger = aggregate
                    .max_delta_bps
                    .is_none_or(|existing| bps.abs() > existing.abs());
                if larger {
                    aggregate.max_delta_bps = Some(bps);
                }
            }
            Some(RelativeDelta::Undefined(cause)) => {
                aggregate
                    .undefined
                    .insert(entry.observation_id.clone(), cause);
            }
            None => {}
        }
    }

    let mut findings = Vec::new();
    let mut matched: BTreeSet<FindingFingerprint> = BTreeSet::new();

    for (fingerprint, aggregate) in &aggregates {
        // Matching is exact on all five parts. A declaration about
        // `pool_tokens_received/decreased` cannot absorb a finding about
        // `sol_received_by_user/decreased`, or the same subject increasing:
        // a new regression must stay visible next to a declared one.
        let declaration = declared.get(fingerprint).copied();
        let (status, breaches, unevaluable) = match declaration {
            None => (ReviewStatus::Unexpected, Vec::new(), None),
            Some(change) => judge(change, aggregate),
        };
        if declaration.is_some() {
            matched.insert(fingerprint.clone());
        }
        findings.push(ReviewedFinding {
            fingerprint: fingerprint.to_string(),
            severity: aggregate.severity.unwrap_or(Severity::Info),
            status,
            observations: aggregate.observations.iter().cloned().collect(),
            entities: aggregate.entities.iter().cloned().collect(),
            covered_observations: measurable
                .get(&fingerprint.evaluable_subject())
                .copied()
                .unwrap_or_default(),
            max_relative_delta_bps: aggregate.max_delta_bps,
            reason: declaration.map(|c| c.reason.clone()),
            breaches,
            unevaluable,
        });
    }

    let mut unmatched = Vec::new();
    for change in &expectations.changes {
        let fingerprint = change.fingerprint();
        if matched.contains(&fingerprint) {
            continue;
        }
        let subject = fingerprint.evaluable_subject();
        let covered = measurable.get(&subject).copied().unwrap_or_default();
        // The distinction this whole module exists for.
        let (status, cause) = if covered > 0 {
            (ReviewStatus::Stale, None)
        } else {
            (
                ReviewStatus::Unevaluable,
                Some(UnevaluableCause::SubjectNotCovered {
                    subject: subject.to_string(),
                }),
            )
        };
        unmatched.push(UnmatchedExpectation {
            fingerprint: fingerprint.to_string(),
            reason: change.reason.clone(),
            status,
            covered_observations: covered,
            unevaluable: cause,
        });
    }

    // Canonical order, like the findings. Traversing the declaration file's own
    // order would let two semantically identical expectation files produce
    // reports that differ only in arrangement, and a diff between two runs
    // should mean something.
    unmatched.sort_by(|a, b| a.fingerprint.cmp(&b.fingerprint));

    let mut failures = Vec::new();
    let failed =
        |status: ReviewStatus, findings: &[ReviewedFinding], un: &[UnmatchedExpectation]| {
            findings.iter().any(|f| f.status == status) || un.iter().any(|e| e.status == status)
        };
    // Precedence order, so `failures.first()` is the exit code.
    if failed(ReviewStatus::Unevaluable, &findings, &unmatched) {
        failures.push(FailureReason::UnevaluableExpectation);
    }
    if failed(ReviewStatus::Stale, &findings, &unmatched) {
        failures.push(FailureReason::StaleExpectation);
    }
    if failed(ReviewStatus::Unexpected, &findings, &unmatched)
        || failed(ReviewStatus::ExpectedButExceeded, &findings, &unmatched)
    {
        failures.push(FailureReason::UndeclaredChange);
    }

    Review {
        findings,
        unmatched,
        failures,
    }
}

fn entity_of(entry: &ObservedFinding) -> Option<String> {
    entry.entity.clone()
}

/// The relative size of one finding, where both sides were measured.
fn measured_delta(finding: &NamedFinding) -> Option<RelativeDelta> {
    match (&finding.baseline, &finding.candidate) {
        (Some(baseline), Some(candidate)) => Some(relative_delta(baseline, candidate)),
        // A finding that reports a delta without both sides is taken at its
        // word; one that reports neither has no relative measure at all.
        _ => finding
            .relative_delta_bps
            .map(RelativeDelta::Bps)
            .or(Some(RelativeDelta::Undefined(UndefinedBound::NotMeasured))),
    }
}

/// Check one declaration's bounds against what was measured.
fn judge(
    change: &ExpectedChange,
    aggregate: &Aggregate,
) -> (ReviewStatus, Vec<BoundBreach>, Option<UnevaluableCause>) {
    let mut breaches = Vec::new();

    if let Some(limit) = change.max_delta_bps {
        // A bound passes only if *every* matching observation is evaluable and
        // every matching observation is within it. Nine measurable observations
        // at 12 bps do not vouch for a tenth whose baseline was zero: that one
        // is unjudged, so the bound is unproven rather than satisfied.
        if let Some((_, cause)) = aggregate.undefined.iter().next() {
            return (
                ReviewStatus::Unevaluable,
                breaches,
                Some(UnevaluableCause::BoundUndefined {
                    detail: cause.as_str().to_string(),
                    unevaluable: aggregate.undefined.len(),
                    matching: aggregate.observations.len(),
                }),
            );
        }
        match aggregate.max_delta_bps {
            Some(observed) if observed.unsigned_abs() > u64::from(limit) => {
                breaches.push(BoundBreach::RelativeDelta {
                    limit_bps: limit,
                    observed_bps: observed,
                });
            }
            Some(_) => {}
            None => {
                return (
                    ReviewStatus::Unevaluable,
                    breaches,
                    Some(UnevaluableCause::BoundUndefined {
                        detail: UndefinedBound::NotMeasured.as_str().to_string(),
                        unevaluable: aggregate.observations.len(),
                        matching: aggregate.observations.len(),
                    }),
                );
            }
        }
    }

    if let Some(limit) = change.max_affected_observations {
        let observed = aggregate.observations.len();
        if observed > limit {
            breaches.push(BoundBreach::AffectedObservations { limit, observed });
        }
    }
    if let Some(limit) = change.max_affected_entities {
        let observed = aggregate.entities.len();
        if observed > limit {
            breaches.push(BoundBreach::AffectedEntities { limit, observed });
        }
    }

    let status = if breaches.is_empty() {
        ReviewStatus::Expected
    } else {
        ReviewStatus::ExpectedButExceeded
    };
    (status, breaches, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expectations::ExpectationFile;
    use crate::semantics::{FindingFingerprint, SemanticValue};

    fn fingerprint(text: &str) -> FindingFingerprint {
        text.parse().expect("valid fingerprint")
    }

    const SHARES: &str = "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased";
    const REVERTS: &str = "spl-stake-pool/withdraw_sol/execution/transaction/now_reverts";

    fn finding(text: &str, severity: Severity, values: Option<(u64, u64)>) -> NamedFinding {
        let (baseline, candidate) = match values {
            Some((b, c)) => (
                Some(SemanticValue::quantity(b, 9)),
                Some(SemanticValue::quantity(c, 9)),
            ),
            None => (None, None),
        };
        NamedFinding {
            fingerprint: fingerprint(text),
            baseline,
            candidate,
            relative_delta_bps: None,
            severity,
        }
    }

    fn observed(
        text: &str,
        severity: Severity,
        values: Option<(u64, u64)>,
        observation: &str,
        entity: &str,
    ) -> ObservedFinding {
        ObservedFinding {
            observation_id: observation.to_string(),
            entity: Some(entity.to_string()),
            finding: finding(text, severity, values),
        }
    }

    fn coverage(observation: &str, subjects: &[&str]) -> ObservationCoverage {
        ObservationCoverage {
            observation_id: observation.to_string(),
            subjects: subjects
                .iter()
                .map(|s| fingerprint(s).evaluable_subject())
                .collect(),
        }
    }

    /// Strip the file-level header so a fixture can be concatenated.
    fn declarations_only(text: &str) -> String {
        text.lines()
            .filter(|line| {
                !line.starts_with("version =") && !line.starts_with("semantic_schema_version =")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn expectations(toml: &str) -> ExpectationFile {
        ExpectationFile::parse(toml).expect("valid expectations")
    }

    const DECLARE_SHARES: &str = r#"
version = 1
semantic_schema_version = 2
[[change]]
protocol = "spl-stake-pool"
action   = "deposit_sol"
domain   = "economic"
subject  = "pool_tokens_received"
change   = "decreased"
max_delta_bps             = 25
max_affected_observations = 10
reason = "Approved deposit fee increase"
"#;

    const DECLARE_REMOVAL: &str = r#"
version = 1
semantic_schema_version = 2
[[change]]
protocol = "spl-stake-pool"
action   = "withdraw_sol"
domain   = "execution"
subject  = "transaction"
change   = "now_reverts"
max_affected_observations = 16
reason = "WithdrawSol is intentionally removed in upgrade v3"
"#;

    #[test]
    fn an_undeclared_change_is_unexpected_and_fails() {
        let review = review(
            &[observed(
                SHARES,
                Severity::Warning,
                Some((100_000, 99_800)),
                "o1",
                "e1",
            )],
            &[coverage("o1", &[SHARES])],
            &ExpectationFile::empty(),
        );
        assert_eq!(review.findings[0].status, ReviewStatus::Unexpected);
        assert_eq!(review.failures, vec![FailureReason::UndeclaredChange]);
        assert!(!review.passed());
    }

    #[test]
    fn a_declared_change_inside_its_bounds_passes() {
        let review = review(
            &[observed(
                SHARES,
                Severity::Warning,
                Some((100_000, 99_800)),
                "o1",
                "e1",
            )],
            &[coverage("o1", &[SHARES])],
            &expectations(DECLARE_SHARES),
        );
        assert_eq!(review.findings[0].status, ReviewStatus::Expected);
        assert_eq!(review.findings[0].max_relative_delta_bps, Some(-20));
        assert_eq!(
            review.findings[0].reason.as_deref(),
            Some("Approved deposit fee increase")
        );
        assert!(review.passed());
    }

    /// Eplyx measures reality; the team says whether it was intended. A
    /// declaration adds a signature, it does not reclassify the finding.
    #[test]
    fn an_expected_finding_keeps_its_severity() {
        let review = review(
            &[observed(REVERTS, Severity::Critical, None, "o1", "e1")],
            &[coverage("o1", &[REVERTS])],
            &expectations(DECLARE_REMOVAL),
        );
        let reviewed = &review.findings[0];
        assert_eq!(reviewed.status, ReviewStatus::Expected);
        assert_eq!(
            reviewed.severity,
            Severity::Critical,
            "critical must never become info because somebody expected it"
        );
        assert!(review.passed(), "an acknowledged critical change can pass");
    }

    #[test]
    fn a_change_larger_than_declared_fails() {
        // 100.000 -> 99.000 is 100 bps, well past the declared 25.
        let review = review(
            &[observed(
                SHARES,
                Severity::Warning,
                Some((100_000, 99_000)),
                "o1",
                "e1",
            )],
            &[coverage("o1", &[SHARES])],
            &expectations(DECLARE_SHARES),
        );
        let reviewed = &review.findings[0];
        assert_eq!(reviewed.status, ReviewStatus::ExpectedButExceeded);
        assert_eq!(
            reviewed.breaches,
            vec![BoundBreach::RelativeDelta {
                limit_bps: 25,
                observed_bps: -100
            }]
        );
        assert_eq!(review.failures, vec![FailureReason::UndeclaredChange]);
    }

    #[test]
    fn a_change_wider_than_declared_fails() {
        let declaration = DECLARE_SHARES.replace(
            "max_affected_observations = 10",
            "max_affected_observations = 1",
        );
        let review = review(
            &[
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_900)),
                    "o1",
                    "e1",
                ),
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_900)),
                    "o2",
                    "e2",
                ),
            ],
            &[coverage("o1", &[SHARES]), coverage("o2", &[SHARES])],
            &expectations(&declaration),
        );
        assert_eq!(review.findings[0].status, ReviewStatus::ExpectedButExceeded);
        assert_eq!(
            review.findings[0].breaches,
            vec![BoundBreach::AffectedObservations {
                limit: 1,
                observed: 2
            }]
        );
    }

    /// The failure this whole module is shaped around. A team declares that
    /// WithdrawSol may be removed; the same candidate also quietly gives
    /// depositors fewer shares. The second must not disappear into the first.
    #[test]
    fn a_declaration_does_not_absorb_a_different_finding() {
        let review = review(
            &[
                observed(REVERTS, Severity::Critical, None, "o1", "e1"),
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_880)),
                    "o2",
                    "e2",
                ),
            ],
            &[coverage("o1", &[REVERTS]), coverage("o2", &[SHARES])],
            &expectations(DECLARE_REMOVAL),
        );

        let by_fingerprint: BTreeMap<&str, &ReviewedFinding> = review
            .findings
            .iter()
            .map(|f| (f.fingerprint.as_str(), f))
            .collect();

        assert_eq!(by_fingerprint[REVERTS].status, ReviewStatus::Expected);
        assert_eq!(by_fingerprint[REVERTS].severity, Severity::Critical);
        assert_eq!(by_fingerprint[SHARES].status, ReviewStatus::Unexpected);
        assert!(
            !review.passed(),
            "the undeclared regression must fail the gate"
        );
    }

    /// Nor may a declaration about one direction absorb the other.
    #[test]
    fn the_opposite_direction_is_a_different_finding() {
        let increased = SHARES.replace("decreased", "increased");
        let review = review(
            &[observed(
                &increased,
                Severity::Warning,
                Some((100_000, 100_100)),
                "o1",
                "e1",
            )],
            &[coverage("o1", &[&increased])],
            &expectations(DECLARE_SHARES),
        );
        assert_eq!(review.findings[0].status, ReviewStatus::Unexpected);
    }

    // ---- stale vs unevaluable -------------------------------------------

    /// The corpus can measure the declared subject, and it is not happening.
    /// The declaration leaves permission behind for behaviour that is gone.
    #[test]
    fn a_declaration_for_behaviour_that_stopped_is_stale() {
        let review = review(
            &[],
            &[coverage("o1", &[REVERTS])],
            &expectations(DECLARE_REMOVAL),
        );
        assert_eq!(review.unmatched.len(), 1);
        assert_eq!(review.unmatched[0].status, ReviewStatus::Stale);
        assert!(review.unmatched[0].unevaluable.is_none());
        assert_eq!(review.failures, vec![FailureReason::StaleExpectation]);
    }

    /// The same absent finding, but the corpus cannot measure the subject at
    /// all. Reporting this as stale would send a team hunting through their
    /// diff for a reversion that never happened.
    #[test]
    fn a_declaration_the_corpus_cannot_measure_is_unevaluable() {
        let review = review(
            &[],
            &[coverage("o1", &[SHARES])],
            &expectations(DECLARE_REMOVAL),
        );
        assert_eq!(review.unmatched[0].status, ReviewStatus::Unevaluable);
        assert_eq!(
            review.unmatched[0].unevaluable,
            Some(UnevaluableCause::SubjectNotCovered {
                subject: "spl-stake-pool/withdraw_sol/execution/transaction".to_string()
            })
        );
        assert_eq!(review.failures, vec![FailureReason::UnevaluableExpectation]);
    }

    /// The two are never conflated, and they never share a failure reason.
    #[test]
    fn stale_and_unevaluable_are_reported_separately() {
        let both = format!(
            "{}\n{}",
            DECLARE_REMOVAL.trim(),
            declarations_only(DECLARE_SHARES).trim()
        );
        let review = review(&[], &[coverage("o1", &[REVERTS])], &expectations(&both));
        let statuses: BTreeSet<ReviewStatus> = review.unmatched.iter().map(|e| e.status).collect();
        assert_eq!(
            statuses,
            BTreeSet::from([ReviewStatus::Stale, ReviewStatus::Unevaluable])
        );
        assert_eq!(
            review.failures,
            vec![
                FailureReason::UnevaluableExpectation,
                FailureReason::StaleExpectation
            ],
            "reported most significant first"
        );
        assert_eq!(review.exit_code(), 5, "coverage uncertainty outranks stale");
    }

    /// With nothing reporting coverage, every unmatched declaration is
    /// unevaluable — the honest answer, never stale by default.
    #[test]
    fn without_coverage_nothing_is_called_stale() {
        let review = review(&[], &[], &expectations(DECLARE_REMOVAL));
        assert_eq!(review.unmatched[0].status, ReviewStatus::Unevaluable);
    }

    /// A bound with no denominator is not satisfied, it is unjudgeable.
    #[test]
    fn a_zero_baseline_makes_a_relative_bound_unevaluable() {
        let review = review(
            &[observed(
                SHARES,
                Severity::Warning,
                Some((0, 500)),
                "o1",
                "e1",
            )],
            &[coverage("o1", &[SHARES])],
            &expectations(DECLARE_SHARES),
        );
        let reviewed = &review.findings[0];
        assert_eq!(reviewed.status, ReviewStatus::Unevaluable);
        assert_eq!(
            reviewed.unevaluable,
            Some(UnevaluableCause::BoundUndefined {
                detail: "baseline quantity is zero".to_string(),
                unevaluable: 1,
                matching: 1,
            })
        );
        assert_eq!(review.failures, vec![FailureReason::UnevaluableExpectation]);
        assert!(!review.passed());
    }

    /// The bound applies per observation, so the widest one decides.
    #[test]
    fn the_largest_magnitude_across_observations_is_what_is_bounded() {
        let review = review(
            &[
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_950)),
                    "o1",
                    "e1",
                ),
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_700)),
                    "o2",
                    "e2",
                ),
            ],
            &[coverage("o1", &[SHARES]), coverage("o2", &[SHARES])],
            &expectations(DECLARE_SHARES),
        );
        assert_eq!(review.findings[0].max_relative_delta_bps, Some(-30));
        assert_eq!(review.findings[0].status, ReviewStatus::ExpectedButExceeded);
    }

    #[test]
    fn observations_and_entities_are_aggregated_deterministically() {
        let review = review(
            &[
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_990)),
                    "o2",
                    "e2",
                ),
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_990)),
                    "o1",
                    "e1",
                ),
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_990)),
                    "o3",
                    "e1",
                ),
            ],
            &[coverage("o1", &[SHARES])],
            &expectations(DECLARE_SHARES),
        );
        assert_eq!(review.findings[0].observations, ["o1", "o2", "o3"]);
        assert_eq!(
            review.findings[0].entities,
            ["e1", "e2"],
            "deduplicated and sorted"
        );
    }

    /// A clean upgrade against a file that declares nothing is the ordinary
    /// passing case, and it must not require an expectation file to exist.
    #[test]
    fn no_findings_and_no_declarations_passes() {
        let review = review(&[], &[coverage("o1", &[SHARES])], &ExpectationFile::empty());
        assert!(review.passed());
        assert!(review.findings.is_empty());
        assert!(review.unmatched.is_empty());
    }

    /// Nine measurable observations do not vouch for a tenth that could not be
    /// judged. The bound is unproven, not satisfied.
    #[test]
    fn one_unevaluable_observation_makes_the_whole_bound_unevaluable() {
        let mut observations: Vec<ObservedFinding> = (0..9)
            .map(|i| {
                observed(
                    SHARES,
                    Severity::Warning,
                    // 12 bps, comfortably inside the declared 25.
                    Some((100_000, 99_880)),
                    &format!("o{i}"),
                    &format!("e{i}"),
                )
            })
            .collect();
        observations.push(observed(
            SHARES,
            Severity::Warning,
            Some((0, 500)),
            "o9",
            "e9",
        ));

        let review = review(
            &observations,
            &[coverage("o0", &[SHARES])],
            &expectations(DECLARE_SHARES),
        );
        let reviewed = &review.findings[0];
        assert_eq!(
            reviewed.status,
            ReviewStatus::Unevaluable,
            "must not pass on the strength of the nine that were measurable"
        );
        assert_eq!(
            reviewed.unevaluable,
            Some(UnevaluableCause::BoundUndefined {
                detail: "baseline quantity is zero".to_string(),
                unevaluable: 1,
                matching: 10,
            })
        );
        assert_eq!(review.exit_code(), 5);
    }

    /// The same rule for every way a bound can be undefined.
    #[test]
    fn any_undefined_bound_cause_blocks_the_whole_bound() {
        for (label, bad) in [
            (
                "incomparable scales",
                NamedFinding {
                    fingerprint: fingerprint(SHARES),
                    baseline: Some(SemanticValue::quantity(100_000, 9)),
                    candidate: Some(SemanticValue::quantity(99_900, 6)),
                    relative_delta_bps: None,
                    severity: Severity::Warning,
                },
            ),
            (
                "not a quantity",
                NamedFinding {
                    fingerprint: fingerprint(SHARES),
                    baseline: Some(SemanticValue::Flag { value: true }),
                    candidate: Some(SemanticValue::Flag { value: false }),
                    relative_delta_bps: None,
                    severity: Severity::Warning,
                },
            ),
            (
                "not measured",
                NamedFinding {
                    fingerprint: fingerprint(SHARES),
                    baseline: None,
                    candidate: None,
                    relative_delta_bps: None,
                    severity: Severity::Warning,
                },
            ),
        ] {
            let observations = vec![
                observed(
                    SHARES,
                    Severity::Warning,
                    Some((100_000, 99_880)),
                    "o0",
                    "e0",
                ),
                ObservedFinding {
                    observation_id: "o1".to_string(),
                    entity: Some("e1".to_string()),
                    finding: bad,
                },
            ];
            let review = review(
                &observations,
                &[coverage("o0", &[SHARES])],
                &expectations(DECLARE_SHARES),
            );
            assert_eq!(
                review.findings[0].status,
                ReviewStatus::Unevaluable,
                "{label} must block the bound"
            );
        }
    }

    /// A declaration with no relative bound is unaffected by an undefined one:
    /// the rule guards `max_delta_bps`, it does not invent a bound.
    #[test]
    fn an_undefined_delta_does_not_block_a_declaration_without_a_delta_bound() {
        let declaration = DECLARE_SHARES.replace("max_delta_bps             = 25\n", "");
        let review = review(
            &[observed(
                SHARES,
                Severity::Warning,
                Some((0, 500)),
                "o1",
                "e1",
            )],
            &[coverage("o1", &[SHARES])],
            &expectations(&declaration),
        );
        assert_eq!(review.findings[0].status, ReviewStatus::Expected);
        assert!(review.passed());
    }

    #[test]
    fn exit_codes_are_distinct_and_stable() {
        assert_eq!(FailureReason::UndeclaredChange.exit_code(), 1);
        assert_eq!(FailureReason::StaleExpectation.exit_code(), 3);
        assert_eq!(FailureReason::UnevaluableExpectation.exit_code(), 5);

        let passing = review(&[], &[coverage("o1", &[SHARES])], &ExpectationFile::empty());
        assert_eq!(passing.exit_code(), 0);
    }

    /// Severity is descriptive metadata. The gate runs on review status alone,
    /// so a better severity model can land later without invalidating a single
    /// existing expectation file.
    #[test]
    fn the_gate_never_consults_severity() {
        for severity in [
            Severity::Info,
            Severity::Warning,
            Severity::High,
            Severity::Critical,
        ] {
            let undeclared = review(
                &[observed(
                    SHARES,
                    severity,
                    Some((100_000, 99_800)),
                    "o1",
                    "e1",
                )],
                &[coverage("o1", &[SHARES])],
                &ExpectationFile::empty(),
            );
            assert!(
                !undeclared.passed(),
                "{severity:?} undeclared must fail like any other"
            );
            assert_eq!(undeclared.exit_code(), 1);
            assert_eq!(
                undeclared.findings[0].severity, severity,
                "and is preserved"
            );
        }

        // The converse: the most severe finding there is passes when it is
        // declared and inside its bounds.
        let declared = review(
            &[observed(REVERTS, Severity::Critical, None, "o1", "e1")],
            &[coverage("o1", &[REVERTS])],
            &expectations(DECLARE_REMOVAL),
        );
        assert!(declared.passed());
        assert_eq!(declared.findings[0].severity, Severity::Critical);
    }

    /// A change that is real but not uniform must not read as uniform. Three of
    /// four withdrawals reverting is inside a bound of four, and the team still
    /// needs to see that one production path still works.
    #[test]
    fn coverage_and_affected_counts_are_both_reported() {
        let observed: Vec<ObservedFinding> = (0..3)
            .map(|i| {
                observed(
                    REVERTS,
                    Severity::Critical,
                    None,
                    &format!("w{i}"),
                    &format!("e{i}"),
                )
            })
            .collect();
        let coverage: Vec<ObservationCoverage> = (0..4)
            .map(|i| coverage(&format!("w{i}"), &[REVERTS]))
            .collect();

        let review = review(&observed, &coverage, &expectations(DECLARE_REMOVAL));
        let reviewed = &review.findings[0];
        assert_eq!(reviewed.observations.len(), 3, "affected");
        assert_eq!(reviewed.covered_observations, 4, "measurable");
        assert_eq!(
            reviewed.status,
            ReviewStatus::Expected,
            "3 is inside the declared bound of 16"
        );
        assert!(review.passed());
    }

    /// Precedence chooses one exit code; it must never remove a failure from
    /// the report.
    #[test]
    fn precedence_summarises_without_filtering() {
        let all_three = format!(
            "{}\n{}\n{}",
            DECLARE_REMOVAL.trim(),
            // stale: covered, not happening
            declarations_only(DECLARE_SHARES).trim(),
            r#"
[[change]]
protocol = "spl-stake-pool"
action   = "deposit_sol"
domain   = "economic"
subject  = "sol_received_by_user"
change   = "increased"
reason   = "declared for a subject this corpus cannot measure"
"#
            .trim()
        );
        let undeclared = SHARES.replace("decreased", "increased");
        let review = review(
            // an undeclared change, plus a declaration for withdraw_sol that is
            // covered but not happening, plus one nothing can measure
            &[observed(
                &undeclared,
                Severity::High,
                Some((100_000, 100_500)),
                "o1",
                "e1",
            )],
            &[coverage("o1", &[SHARES, REVERTS])],
            &expectations(&all_three),
        );

        assert_eq!(
            review.failures,
            vec![
                FailureReason::UnevaluableExpectation,
                FailureReason::StaleExpectation,
                FailureReason::UndeclaredChange,
            ],
            "all three survive into the report"
        );
        assert_eq!(
            review.exit_code(),
            5,
            "the shell gets one deterministic code"
        );
        assert_eq!(review.count(ReviewStatus::Unexpected), 1);
        assert_eq!(review.count(ReviewStatus::Stale), 2);
        assert_eq!(review.count(ReviewStatus::Unevaluable), 1);
    }

    /// Two declaration files that declare the same things in a different order
    /// are the same contract, and must produce the same report.
    #[test]
    fn declaration_order_does_not_reach_the_report() {
        let removal = declarations_only(DECLARE_REMOVAL);
        let shares = declarations_only(DECLARE_SHARES);
        let header = "version = 1\nsemantic_schema_version = 2\n";
        let one = expectations(&format!("{header}{removal}\n{shares}"));
        let other = expectations(&format!("{header}{shares}\n{removal}"));

        let coverage = [coverage("o1", &[REVERTS])];
        let a = review(&[], &coverage, &one);
        let b = review(&[], &coverage, &other);
        assert_eq!(a, b, "the same contract, written in a different order");
    }

    #[test]
    fn a_review_survives_a_json_round_trip() {
        let review = review(
            &[observed(
                SHARES,
                Severity::Warning,
                Some((100_000, 99_000)),
                "o1",
                "e1",
            )],
            &[coverage("o1", &[SHARES])],
            &expectations(DECLARE_SHARES),
        );
        let json = serde_json::to_string(&review).unwrap();
        assert_eq!(serde_json::from_str::<Review>(&json).unwrap(), review);
    }
}
