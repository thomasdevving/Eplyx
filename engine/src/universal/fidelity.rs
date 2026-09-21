//! Fidelity is infrastructure policy and runs before semantic interpretation.
use std::collections::BTreeMap;

use anyhow::Result;

use super::{
    execution::ExecutionEvidence,
    model::{ExpectedHistoricalOutcome, FidelityProfile},
};
use crate::{
    executor::ExecutionResult,
    replay::{ReplayFidelity, ReplayRecord, ReplayStateSource},
    types::AccountSnapshot,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FidelityResult {
    pub profile: FidelityProfile,
    pub status: ReplayFidelity,
    pub failures: Vec<String>,
}

impl FidelityResult {
    pub fn matched(&self) -> bool {
        matches!(self.status, ReplayFidelity::Exact | ReplayFidelity::Matched)
    }
}

/// V1 continues to compare only what its frozen records captured. It does not
/// silently inherit the V2 policy or claim that it contains V2 evidence.
pub fn compare_v1(record: &ReplayRecord, local: &ExecutionResult) -> Result<FidelityResult> {
    let failures = record.fidelity_failures(local)?;
    let status = if record.state_source == ReplayStateSource::CurrentApproximation {
        ReplayFidelity::Approximate
    } else if record.original.is_none() {
        ReplayFidelity::Unknown
    } else if !failures.is_empty() {
        ReplayFidelity::Mismatch
    } else if record.state_source == ReplayStateSource::ControlledSnapshot {
        ReplayFidelity::Exact
    } else {
        ReplayFidelity::Matched
    };
    Ok(FidelityResult {
        profile: FidelityProfile::HistoricalReplayV1,
        status,
        failures,
    })
}

/// Compare the complete execution evidence retained by U3F. The only
/// representation exception is Agave's omission of empty return data.
pub fn compare_v2(
    expected: &ExpectedHistoricalOutcome,
    expected_accounts: &BTreeMap<String, Option<AccountSnapshot>>,
    local: &ExecutionEvidence,
) -> FidelityResult {
    let mut failures = Vec::new();
    if expected.success != local.success || expected.error != local.error {
        failures.push("outcome".into());
    }
    if expected.fee != local.fee {
        failures.push("fee".into());
    }
    if expected.logs != local.logs {
        failures.push("logs".into());
    }
    let actual_inner = local
        .inner_instructions
        .iter()
        .filter(|group| !group.instructions.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if expected.inner_instructions != actual_inner {
        failures.push("inner_instructions".into());
    }
    match &expected.return_data {
        Some(value) if value != &local.return_data => failures.push("return_data".into()),
        None if !local.return_data.data.is_empty() => failures.push("return_data".into()),
        _ => {}
    }
    if expected_accounts.len() != expected.watched_accounts.len()
        || local.post_accounts.len() != expected.watched_accounts.len()
    {
        failures.push("watched_account_set".into());
    }
    for watched in &expected.watched_accounts {
        let address = &watched.address;
        match (
            expected_accounts.get(address),
            local.post_accounts.get(address),
        ) {
            (Some(expected), Some(actual)) if expected == actual => {}
            (Some(Some(expected)), Some(Some(actual))) => {
                if expected.owner != actual.owner {
                    failures.push(format!("account:{address}:owner"));
                }
                if expected.lamports != actual.lamports {
                    failures.push(format!("account:{address}:lamports"));
                }
                if expected.executable != actual.executable {
                    failures.push(format!("account:{address}:executable"));
                }
                if expected.rent_epoch != actual.rent_epoch {
                    failures.push(format!("account:{address}:rent_epoch"));
                }
                if expected.data != actual.data {
                    failures.push(format!("account:{address}:raw_data"));
                }
            }
            _ => failures.push(format!("account:{address}:presence")),
        }
    }
    FidelityResult {
        profile: FidelityProfile::CompleteExecutionV2,
        status: if failures.is_empty() {
            ReplayFidelity::Matched
        } else {
            ReplayFidelity::Mismatch
        },
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universal::{
        evidence::{EvidenceKind, EvidenceRef},
        execution::{InnerGroup, ReturnData},
        model::{ExpectedAccountSource, WatchedAccount},
    };

    fn fixture() -> (
        ExpectedHistoricalOutcome,
        BTreeMap<String, Option<AccountSnapshot>>,
        ExecutionEvidence,
    ) {
        let address = "watched".to_string();
        let account = AccountSnapshot {
            owner: "owner".into(),
            lamports: 17,
            data: vec![1, 2, 3],
            executable: false,
            rent_epoch: 8,
        };
        let reference = EvidenceRef {
            kind: EvidenceKind::AccountObservation,
            sha256: "a".repeat(64),
        };
        let expected = ExpectedHistoricalOutcome {
            success: true,
            error: None,
            fee: 5000,
            logs: vec!["log".into()],
            inner_instructions: Vec::new(),
            return_data: None,
            watched_accounts: vec![WatchedAccount {
                address: address.clone(),
                expected_post_content: None,
                source: ExpectedAccountSource::Archived(reference),
            }],
        };
        let accounts = BTreeMap::from([(address.clone(), Some(account.clone()))]);
        let local = ExecutionEvidence {
            success: true,
            error: None,
            compute_units: 42,
            fee: 5000,
            logs: vec!["log".into()],
            inner_instructions: vec![InnerGroup {
                outer_index: 0,
                instructions: Vec::new(),
            }],
            return_data: ReturnData {
                program: "last program".into(),
                data: Vec::new(),
            },
            post_accounts: BTreeMap::from([(address, Some(account))]),
        };
        (expected, accounts, local)
    }

    #[test]
    fn complete_profile_checks_raw_data_and_return_representation() {
        let (expected, accounts, mut local) = fixture();
        assert!(
            compare_v2(&expected, &accounts, &local).matched(),
            "empty return representation is normalized"
        );
        local.return_data.data = vec![1];
        assert!(
            compare_v2(&expected, &accounts, &local)
                .failures
                .contains(&"return_data".into()),
            "nonempty return mismatch must fail"
        );
        local.return_data.data.clear();
        local
            .post_accounts
            .get_mut("watched")
            .unwrap()
            .as_mut()
            .unwrap()
            .data[2] = 4;
        assert!(
            compare_v2(&expected, &accounts, &local)
                .failures
                .contains(&"account:watched:raw_data".into()),
            "typed equality cannot hide a raw byte mismatch"
        );
        local.post_accounts.insert("watched".into(), None);
        assert!(
            compare_v2(&expected, &accounts, &local)
                .failures
                .contains(&"account:watched:presence".into()),
            "watched account lifecycle must be compared"
        );
    }
}
