//! Exact read-only provider responses retained for offline re-evaluation.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub method: String,
    pub params: Value,
    pub started_at: String,
    pub completed_at: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}

/// A successful raw read, indexed in its parent transcript. It proves nothing by itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcEvidence {
    pub id: usize,
    pub method: String,
    pub params: Value,
    pub result: Value,
}
