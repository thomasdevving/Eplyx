//! The expected-change contract: what a team declares it meant to do.
//!
//! This file is **not an allowlist**. `allow_critical = true` or
//! `ignore = ["WithdrawSol"]` would become a mechanism for clicking regressions
//! away within a week, and a file full of those is indistinguishable from
//! having no gate. Every declaration here is narrow by construction: it names
//! one exact [`FindingFingerprint`], it carries bounds, and it carries a reason.
//!
//! ```toml
//! version = 1
//! semantic_schema_version = 2
//!
//! [[change]]
//! protocol = "spl-stake-pool"
//! action   = "deposit_sol"
//! domain   = "economic"
//! subject  = "pool_tokens_received"
//! change   = "decreased"
//!
//! max_delta_bps             = 25
//! max_affected_observations = 10
//! max_affected_entities     = 8
//!
//! reason = "Approved deposit fee increase from 0.10% to 0.25%"
//! ```
//!
//! The identity is written out field by field rather than as one canonical
//! string. Structured input gives type validation, error messages that point at
//! the wrong field, and a migration path — and it keeps string parsing out of
//! the core logic. The canonical string is for *display*: reports, the CLI and
//! the GitHub summary all print
//! `spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::semantics::{
    ActionId, ChangeKind, FindingDomain, FindingFingerprint, ProtocolId, SemanticSubject,
    SEMANTIC_SCHEMA_VERSION,
};

/// Schema of the expectation file itself, as opposed to the vocabulary it uses.
pub const EXPECTATION_SCHEMA_VERSION: u32 = 1;

/// What an absent `semantic_schema_version` means.
///
/// Deliberately the literal 1 rather than [`SEMANTIC_SCHEMA_VERSION`]. A file
/// written today against schema 1 and left untouched must keep meaning schema 1
/// when the vocabulary moves to 2 — defaulting to "whatever is current" would
/// silently reinterpret every old file the moment a subject changed meaning,
/// which is the exact failure the version exists to prevent.
const ASSUMED_SEMANTIC_SCHEMA: u32 = 1;

/// One declared change.
///
/// `deny_unknown_fields` is load-bearing: a typo like `max_delta_bp` would
/// otherwise be ignored, turning a bounded declaration into an unbounded one
/// without anybody noticing.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedChange {
    pub protocol: ProtocolId,
    pub action: ActionId,
    pub domain: FindingDomain,
    pub subject: SemanticSubject,
    pub change: ChangeKind,

    /// Largest per-observation relative change permitted, in basis points,
    /// measured against the baseline value of the named subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_delta_bps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_affected_observations: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_affected_entities: Option<usize>,

    /// Why this change is intended. Required, and required to say something:
    /// an expectation nobody can explain is how a file accumulates permissions
    /// that outlive the decision behind them.
    pub reason: String,
}

impl ExpectedChange {
    pub fn fingerprint(&self) -> FindingFingerprint {
        FindingFingerprint {
            protocol: self.protocol.clone(),
            action: self.action.clone(),
            domain: self.domain,
            subject: self.subject.clone(),
            change: self.change,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectationFile {
    /// Schema of this file.
    pub version: u32,
    /// Vocabulary the declarations below key on. Absent means schema 1.
    #[serde(default = "assumed_semantic_schema")]
    pub semantic_schema_version: u32,
    #[serde(default, rename = "change")]
    pub changes: Vec<ExpectedChange>,
}

fn assumed_semantic_schema() -> u32 {
    ASSUMED_SEMANTIC_SCHEMA
}

impl ExpectationFile {
    /// An empty contract: nothing is declared, so every finding is unexpected.
    ///
    /// This is the correct default for a team that has not written the file
    /// yet, and it is the state a passing gate should normally be in.
    pub fn empty() -> Self {
        Self {
            version: EXPECTATION_SCHEMA_VERSION,
            semantic_schema_version: SEMANTIC_SCHEMA_VERSION,
            changes: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let file: Self = toml::from_str(text).context("reading the expectation file")?;
        file.validate()?;
        Ok(file)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    fn validate(&self) -> Result<()> {
        if self.version != EXPECTATION_SCHEMA_VERSION {
            bail!(
                "expectation file schema {} is not supported by this build (expected {EXPECTATION_SCHEMA_VERSION})",
                self.version
            );
        }
        // Expectations written against a different vocabulary are refused
        // rather than reinterpreted: the same subject name may no longer mean
        // the same quantity.
        if self.semantic_schema_version != SEMANTIC_SCHEMA_VERSION {
            bail!(
                "these expectations were written against semantic schema {}, and this build \
                 speaks {SEMANTIC_SCHEMA_VERSION}. Subject names may no longer mean the same \
                 thing, so they are not applied. Review and re-declare them.",
                self.semantic_schema_version
            );
        }

        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for (index, change) in self.changes.iter().enumerate() {
            if change.reason.trim().is_empty() {
                bail!(
                    "expectation {} ({}) has no reason. Every declared change must say why it \
                     is intended.",
                    index + 1,
                    change.fingerprint()
                );
            }
            if change.max_affected_observations == Some(0)
                || change.max_affected_entities == Some(0)
            {
                bail!(
                    "expectation {} ({}) permits zero affected observations or entities, which \
                     cannot be satisfied by a finding that exists. Remove the declaration \
                     instead.",
                    index + 1,
                    change.fingerprint()
                );
            }
            // A relative bound needs a magnitude to be relative to. On
            // `now_reverts` or `changed` there is none, so the declaration
            // could never be satisfied - better to say so here than to report
            // it as unevaluable on every run.
            if change.max_delta_bps.is_some()
                && !matches!(change.change, ChangeKind::Increased | ChangeKind::Decreased)
            {
                bail!(
                    "expectation {} ({}) sets max_delta_bps, but '{}' has no magnitude to \
                     measure. Relative bounds apply to 'increased' and 'decreased' only.",
                    index + 1,
                    change.fingerprint(),
                    change.change.as_str()
                );
            }
            // Two declarations for one fingerprint would make the review depend
            // on which was checked first.
            let canonical = change.fingerprint().to_string();
            if let Some(first) = seen.insert(canonical.clone(), index + 1) {
                bail!(
                    "expectations {first} and {} both declare {canonical}; one fingerprint has \
                     one declaration.",
                    index + 1
                );
            }
        }
        Ok(())
    }

    /// Declarations by fingerprint, for the review engine.
    pub fn by_fingerprint(&self) -> BTreeMap<FindingFingerprint, &ExpectedChange> {
        self.changes
            .iter()
            .map(|change| (change.fingerprint(), change))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPOSIT_FEE: &str = r#"
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
max_affected_entities     = 8
reason = "Approved deposit fee increase from 0.10% to 0.25%"
"#;

    #[test]
    fn a_declaration_parses_into_a_fingerprint() {
        let file = ExpectationFile::parse(DEPOSIT_FEE).expect("parses");
        assert_eq!(file.changes.len(), 1);
        assert_eq!(
            file.changes[0].fingerprint().to_string(),
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased"
        );
        assert_eq!(file.changes[0].max_delta_bps, Some(25));
    }

    /// A file that declares nothing is the normal state of a passing gate.
    #[test]
    fn a_file_with_no_declarations_is_valid() {
        let file =
            ExpectationFile::parse("version = 1\nsemantic_schema_version = 2\n").expect("parses");
        assert!(file.changes.is_empty());
    }

    /// A silently ignored bound turns a bounded declaration into an unbounded
    /// one, which is the whole failure mode this file exists to prevent.
    #[test]
    fn a_misspelled_field_is_an_error_not_an_ignored_bound() {
        let text = DEPOSIT_FEE.replace("max_delta_bps  ", "max_delta_bp   ");
        let error = ExpectationFile::parse(&text).expect_err("must refuse");
        assert!(format!("{error:#}").contains("max_delta_bp"), "{error:#}");
    }

    /// This is not an allowlist. There is no field that turns a severity off.
    #[test]
    fn allowlist_shaped_fields_do_not_exist() {
        for field in [
            "allow_critical = true",
            "ignore = [\"withdraw_sol\"]",
            "severity = \"info\"",
            "suppress = true",
        ] {
            let text = format!("{DEPOSIT_FEE}{field}\n");
            assert!(ExpectationFile::parse(&text).is_err(), "accepted {field:?}");
        }
    }

    #[test]
    fn a_declaration_without_a_reason_is_refused() {
        let text = DEPOSIT_FEE.replace(
            r#"reason = "Approved deposit fee increase from 0.10% to 0.25%""#,
            r#"reason = "   ""#,
        );
        let error = ExpectationFile::parse(&text).expect_err("must refuse");
        assert!(format!("{error:#}").contains("no reason"), "{error:#}");

        let missing = DEPOSIT_FEE.replace(
            r#"reason = "Approved deposit fee increase from 0.10% to 0.25%""#,
            "",
        );
        assert!(
            ExpectationFile::parse(&missing).is_err(),
            "reason is required"
        );
    }

    #[test]
    fn two_declarations_for_one_fingerprint_are_refused() {
        let text = format!("{DEPOSIT_FEE}\n{}", declarations_only(DEPOSIT_FEE));
        let error = ExpectationFile::parse(&text).expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("one fingerprint has one declaration"),
            "{error:#}"
        );
    }

    #[test]
    fn a_relative_bound_needs_a_magnitude() {
        let text = DEPOSIT_FEE
            .replace(r#"change   = "decreased""#, r#"change   = "now_reverts""#)
            .replace(r#"domain   = "economic""#, r#"domain   = "execution""#);
        let error = ExpectationFile::parse(&text).expect_err("must refuse");
        assert!(format!("{error:#}").contains("no magnitude"), "{error:#}");
    }

    #[test]
    fn a_bound_of_zero_cannot_be_satisfied_and_is_refused() {
        let text = DEPOSIT_FEE.replace(
            "max_affected_observations = 10",
            "max_affected_observations = 0",
        );
        let error = ExpectationFile::parse(&text).expect_err("must refuse");
        assert!(format!("{error:#}").contains("zero affected"), "{error:#}");
    }

    #[test]
    fn an_unsupported_file_schema_is_refused() {
        let text = DEPOSIT_FEE.replace("version = 1", "version = 99");
        assert!(ExpectationFile::parse(&text).is_err());
    }

    /// Expectations written against a different vocabulary are refused rather
    /// than reinterpreted: a subject name may no longer mean the same quantity.
    #[test]
    fn a_different_semantic_schema_is_refused() {
        let text = DEPOSIT_FEE.replace(
            "semantic_schema_version = 2",
            &format!("semantic_schema_version = {}", SEMANTIC_SCHEMA_VERSION + 1),
        );
        let error = ExpectationFile::parse(&text).expect_err("must refuse");
        let reported = format!("{error:#}");
        assert!(
            reported.contains(&format!("semantic schema {}", SEMANTIC_SCHEMA_VERSION + 1)),
            "{reported}"
        );
    }

    /// An absent `semantic_schema_version` means schema 1 forever, not
    /// "whatever this build happens to speak".
    ///
    /// The vocabulary has since moved to 2, so this is now observable end to
    /// end: a file written before the move and left untouched is *refused*
    /// rather than silently reinterpreted under the new meaning. That is the
    /// entire purpose of the constant being a literal.
    #[test]
    fn an_absent_semantic_schema_means_schema_one() {
        assert_eq!(ASSUMED_SEMANTIC_SCHEMA, 1);
        let without = DEPOSIT_FEE.replace("semantic_schema_version = 2\n", "");
        assert!(!without.contains("semantic_schema_version"));
        let error = ExpectationFile::parse(&without).expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("semantic schema 1"),
            "{error:#}"
        );
    }

    /// Strip the file-level header so a fixture can be concatenated onto
    /// another without duplicating keys.
    fn declarations_only(text: &str) -> String {
        text.lines()
            .filter(|line| {
                !line.starts_with("version =") && !line.starts_with("semantic_schema_version =")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_malformed_identity_is_rejected_by_the_vocabulary() {
        let text = DEPOSIT_FEE.replace(r#"domain   = "economic""#, r#"domain   = "made_up""#);
        assert!(ExpectationFile::parse(&text).is_err());
        let text = DEPOSIT_FEE.replace(
            r#"protocol = "spl-stake-pool""#,
            r#"protocol = "SPL Stake Pool""#,
        );
        assert!(ExpectationFile::parse(&text).is_err());
    }
}
