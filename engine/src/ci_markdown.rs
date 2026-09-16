//! Markdown rendering of a [`CiReport`].
//!
//! Rendered from the report object alone — never by re-running analysis — so a
//! report fetched hours later says exactly what the run said. Lives in the
//! engine rather than in the server so the hosted service and the CLI cannot
//! drift into two different accounts of the same result.
//!
//! Two rules the wording obeys:
//!
//! A passing run never says an upgrade is *safe*. It says no unexpected
//! economic changes were detected across the tested corpus, which is the claim
//! the evidence actually supports.
//!
//! Severity and review status stay on separate lines of the same heading.
//! `CRITICAL / EXPECTED` is never collapsed to `INFO`: the change is still
//! critical, and what the declaration adds is that somebody signed for it.

use std::fmt::Write as _;

use crate::ci::CiReport;
use crate::review::{ReviewStatus, ReviewedFinding};

/// Render a completed check as GitHub-flavoured Markdown.
pub fn render(report: &CiReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## Eplyx Upgrade Impact\n");

    let _ = writeln!(
        out,
        "{}\n",
        if report.summary.passed {
            "### ✅ Upgrade check passed"
        } else {
            "### ❌ Upgrade check failed"
        }
    );

    // The green wording is load-bearing. "Safe" is a claim about the upgrade;
    // this is a claim about a corpus, and only about the corpus.
    let _ = writeln!(
        out,
        "{}\n",
        if !report.summary.passed {
            "Changes were found that are not declared, or that exceed what was declared."
        } else if report.summary.expected > 0 {
            "All observed changes matched the declared expectations and remained within \
             their configured bounds."
        } else {
            "No unexpected economic changes were detected across the tested validated \
             historical corpus."
        }
    );

    let _ = writeln!(out, "### Bundle\n");
    let _ = writeln!(out, "| | |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| Program | `{}` |", report.bundle.program_id);
    let _ = writeln!(out, "| Bundle | `{}` |", report.bundle.sha256);
    let _ = writeln!(out, "| Corpus | `{}` |", report.bundle.corpus_sha256);
    let _ = writeln!(out, "| Baseline | `{}` |", report.bundle.baseline_sha256);
    let _ = writeln!(out, "| Candidate | `{}` |", report.candidate.sha256);
    let _ = writeln!(
        out,
        "| Adapter | `{}` v{} |",
        report.bundle.adapter, report.bundle.adapter_version
    );
    let _ = writeln!(
        out,
        "| Semantic schema | v{} |",
        report.bundle.semantic_schema_version
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "### Corpus\n");
    let _ = writeln!(
        out,
        "{} validated historical observations, production slots {} → {}.\n",
        report.bundle.record_count,
        report.bundle.source_slot_range.first,
        report.bundle.source_slot_range.last
    );
    if !report.coverage.is_empty() {
        let _ = writeln!(out, "| Semantic subject | Observations |");
        let _ = writeln!(out, "|---|---:|");
        for subject in &report.coverage {
            let _ = writeln!(out, "| `{}` | {} |", subject.subject, subject.observations);
        }
        let _ = writeln!(out);
    }

    render_group(
        &mut out,
        report,
        ReviewStatus::Unexpected,
        "Unexpected changes",
        "Nothing in `expected-changes.toml` declares these.",
    );
    render_group(
        &mut out,
        report,
        ReviewStatus::ExpectedButExceeded,
        "Expected, but larger than declared",
        "Declared, but the measured impact is outside the declared bounds.",
    );
    render_group(
        &mut out,
        report,
        ReviewStatus::Expected,
        "Expected changes",
        "Declared in `expected-changes.toml` and within their bounds.",
    );
    render_group(
        &mut out,
        report,
        ReviewStatus::Unevaluable,
        "Changes that could not be judged",
        "A declared bound has no defined value for at least one matching observation.",
    );

    if !report.review.unmatched.is_empty() {
        let _ = writeln!(out, "### Expectation health\n");
        for entry in &report.review.unmatched {
            let _ = writeln!(
                out,
                "**{}** — `{}`  ",
                entry.status.as_str().to_uppercase(),
                entry.fingerprint
            );
            let _ = writeln!(out, "Declared: {}  ", entry.reason);
            match entry.status {
                ReviewStatus::Stale => {
                    let _ = writeln!(
                        out,
                        "{} observations in this corpus can measure it, and it is not \
                         happening. The declaration leaves permission behind for behaviour \
                         that no longer exists.\n",
                        entry.covered_observations
                    );
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "No observation in this corpus can measure it, so Eplyx cannot prove \
                         whether this declaration still applies.\n"
                    );
                }
            }
        }
    }

    if !report.bundle.limitations.is_empty() {
        let _ = writeln!(out, "### Coverage limitations\n");
        let _ = writeln!(
            out,
            "These hold whatever the result above says, including a passing one.\n"
        );
        for limitation in &report.bundle.limitations {
            let _ = writeln!(out, "- **{}** — {}", limitation.code, limitation.detail);
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(
        out,
        "---\n_Eplyx runs candidate Solana program upgrades against a pinned corpus of \
         validated historical production interactions and fails CI on undeclared economic \
         changes. Exit code {}._",
        report.summary.exit_code
    );
    out
}

fn render_group(
    out: &mut String,
    report: &CiReport,
    status: ReviewStatus,
    heading: &str,
    blurb: &str,
) {
    let group: Vec<&ReviewedFinding> = report
        .review
        .findings
        .iter()
        .filter(|finding| finding.status == status)
        .collect();
    if group.is_empty() {
        return;
    }
    let _ = writeln!(out, "### {heading}\n");
    let _ = writeln!(out, "{blurb}\n");
    for finding in group {
        // Severity and review status are independent axes and stay side by
        // side. Neither is ever folded into the other.
        let _ = writeln!(
            out,
            "**{} / {}**  ",
            finding.severity.as_str(),
            finding.status.as_str().to_uppercase()
        );
        let _ = writeln!(out, "`{}`  ", finding.fingerprint);
        let _ = writeln!(
            out,
            "Affected: {} of {} measurable observations, {} economic entities  ",
            finding.observations.len(),
            finding.covered_observations,
            finding.entities.len()
        );
        if let Some(bps) = finding.max_relative_delta_bps {
            let _ = writeln!(out, "Largest change: {bps} bps  ");
        }
        if let Some(reason) = &finding.reason {
            let _ = writeln!(out, "Declared: {reason}  ");
        }
        for breach in &finding.breaches {
            let _ = writeln!(out, "Exceeds: `{breach:?}`  ");
        }
        if let Some(cause) = &finding.unevaluable {
            let _ = writeln!(out, "Cannot judge: `{cause:?}`  ");
        }
        let _ = writeln!(out);
    }
}
