//! Exact read-only provider responses retained for offline re-evaluation.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub method: String,
    pub params: Value,
    pub started_at: String,
    pub completed_at: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}
