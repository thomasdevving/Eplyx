//! Presentation labels shared with the frontend; serialized engine codes stay unchanged.
use std::{collections::BTreeMap, sync::OnceLock};
pub fn label(code: &str) -> &str {
    static LABELS: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    LABELS
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../frontend/src/glossary.json"))
                .expect("presentation glossary must be valid")
        })
        .get(code)
        .map(String::as_str)
        .unwrap_or(code)
}

/// Apply glossary labels only to complete machine-code words in a human report.
/// Callers must keep JSON, evidence files, identities and command inputs untouched.
pub fn text(report: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(report.len());
    for chunk in report.split_inclusive(|c: char| !word(c)) {
        let code = chunk.trim_end_matches(|c: char| !word(c));
        out.push_str(label(code));
        out.push_str(&chunk[code.len()..]);
    }
    let old="A rehearsal that succeeds for tested states does not show that every possible holder or state is safe.";
    out.replace(old, label(old))
}

#[cfg(test)]
mod tests {
    #[test]
    fn presentation_changes_labels_without_matching_inside_identifiers() {
        assert_eq!(
            super::text("NotTested, Unknown; Proven. cx_NotTested_x"),
            "Not evaluated, Cannot be judged; Verified in the local VM. cx_NotTested_x"
        );
    }
}
