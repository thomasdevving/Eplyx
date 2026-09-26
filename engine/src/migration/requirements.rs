use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const EVALUATION_VERSION: &str = "eplyx-package-invariants/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Blocking,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Satisfied,
    Violated,
    Indeterminate,
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Result {
    pub invariant_id: String,
    pub invariant_type: String,
    pub severity: Severity,
    pub status: Status,
    pub scope: String,
    pub config: Value,
    pub evidence_refs: Vec<String>,
    pub explanation: String,
    pub evaluation_version: String,
}
