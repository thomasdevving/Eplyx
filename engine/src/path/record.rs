//! Untrusted report records. Deserializing these never constructs VM evidence.
use crate::types::AccountSnapshot;
use std::collections::BTreeMap;
/// Actual inner instruction payloads for consequence probes (including event CPI).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecordedInnerInstruction {
    pub program: String,
    pub stack_height: u8,
    pub accounts: Vec<String>,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecordedExecution {
    pub success: bool,
    pub error: Option<String>,
    #[serde(with = "crate::numfmt::u64_string")]
    pub compute_units: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub transaction_fee_lamports: u64,
    pub logs: Vec<String>,
    pub inner_instructions: Vec<RecordedInnerInstruction>,
    pub post_accounts: BTreeMap<String, AccountSnapshot>,
}

impl From<crate::executor::ProbeTransactionExecution> for RecordedExecution {
    fn from(x: crate::executor::ProbeTransactionExecution) -> Self {
        Self {
            success: x.success,
            error: x.error,
            compute_units: x.compute_units,
            transaction_fee_lamports: x.transaction_fee_lamports,
            logs: x.logs,
            inner_instructions: x
                .inner_instructions
                .into_iter()
                .map(|i| RecordedInnerInstruction {
                    program: i.program,
                    stack_height: i.stack_height,
                    accounts: i.accounts,
                    data: i.data,
                })
                .collect(),
            post_accounts: x.post_accounts,
        }
    }
}
