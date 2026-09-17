//! Same-slot interference screening.
//!
//! A slot-addressable account archive answers "what did this account hold at the
//! end of slot N". A replay needs "what did this account hold immediately before
//! transaction T in slot S", and those are the same question only when no other
//! transaction in slot S touches the account. Phase 7 established that this is a
//! real constraint rather than a theoretical one, and caught violations after the
//! fact, through a boundary proof that failed.
//!
//! Failing late is adequate for two data-empty System accounts. It is not
//! adequate once a transaction depends on a dozen accounts reached through CPI,
//! because the proof available for each of them is uneven: validator metadata
//! pins lamports and token balances, and pins nothing at all about a pool's
//! internal bytes. So the block is read directly and the conflicting transaction
//! is named, which turns "the archive disagrees with metadata" into "transaction
//! X at index 127 wrote this account earlier in the slot".
//!
//! The criterion is deliberately blunt: any other transaction in the slot that
//! takes a required account as writable makes the boundary ambiguous, whether or
//! not it is visible to have changed anything. A writable account can be
//! rewritten with no lamport movement, and nothing in block metadata would show
//! it.

use crate::ingest::rpc::RpcProvider;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// Where a conflicting transaction sits relative to the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPosition {
    /// Executed earlier in the slot, so the archived state at `S-1` is no
    /// longer this transaction's pre-state.
    Before,
    /// Executed later in the slot, so the archived state at `S` is no longer
    /// this transaction's post-state and cannot serve as the fidelity reference.
    After,
}

impl ConflictPosition {
    fn explain(self) -> &'static str {
        match self {
            Self::Before => {
                "wrote the account earlier in the slot, so the archived end-of-S-1 state is \
                 not this transaction's pre-state"
            }
            Self::After => {
                "wrote the account later in the slot, so the archived end-of-S state is not \
                 this transaction's post-state"
            }
        }
    }
}

/// One account whose boundary cannot be reconstructed unambiguously.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountConflict {
    pub account: String,
    pub target_signature: String,
    pub target_index: usize,
    /// The other transaction, when the block identifies one. Absent only when
    /// the block itself could not establish ordering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicting_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicting_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<ConflictPosition>,
    pub reason: String,
}

impl std::fmt::Display for AccountConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.account, self.reason)
    }
}

/// The result of screening one slot for one transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotScreening {
    pub slot: u64,
    pub target_signature: String,
    pub target_index: usize,
    pub transactions_in_slot: usize,
    /// Accounts whose exact boundary state the replay depends on.
    pub required_accounts: Vec<String>,
    pub conflicts: Vec<AccountConflict>,
}

impl SlotScreening {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }

    /// Reject the candidate unless every required account has an unambiguous
    /// boundary. A conflict is an error, never an approximation.
    pub fn ensure_unambiguous(&self) -> Result<()> {
        if self.conflicts.is_empty() {
            return Ok(());
        }
        let detail = self
            .conflicts
            .iter()
            .map(|conflict| format!("\n  - {conflict}"))
            .collect::<String>();
        anyhow::bail!(
            "transaction {} in slot {} shares {} account(s) with other transactions in the \
             same slot, so its exact boundary state cannot be reconstructed. Select a \
             transaction whose required accounts are untouched elsewhere in its slot.{}",
            self.target_signature,
            self.slot,
            self.conflicts.len(),
            detail
        );
    }
}

/// Writable keys of one block transaction, including lookup-table resolutions.
fn writable_keys(transaction: &serde_json::Value) -> Result<BTreeSet<String>> {
    let keys = transaction["transaction"]["accountKeys"]
        .as_array()
        .context(
            "block transaction is missing resolved account keys; \
                  request getBlock with transactionDetails=accounts",
        )?;
    let mut writable = BTreeSet::new();
    for key in keys {
        let pubkey = key["pubkey"]
            .as_str()
            .context("block account key is not a string")?;
        // `writable` is reported per key and already accounts for the message
        // header and for any address-table resolution the validator performed.
        //
        // A key that does not report it is *not* read-only. This is a proof of
        // no interference, and an absent write flag is absent evidence, not
        // evidence of absence: defaulting it to false would let an incomplete
        // provider response certify a boundary as clean.
        let Some(is_writable) = key["writable"].as_bool() else {
            anyhow::bail!(
                "block evidence is incomplete: account {pubkey} reports no writable flag, so \
                 this block cannot prove whether it was written. Request getBlock with \
                 transactionDetails=accounts from a provider that reports it."
            );
        };
        if is_writable {
            writable.insert(pubkey.to_string());
        }
    }
    Ok(writable)
}

fn signature_of(transaction: &serde_json::Value) -> Option<&str> {
    transaction["transaction"]["signatures"]
        .as_array()?
        .first()?
        .as_str()
}

/// Screen `slot` for interference with `signature` over `required`.
///
/// Reads the block once. `transactionDetails=accounts` is used rather than
/// `full` because the writable set and the execution order are all the screen
/// needs, and the compact form is a fraction of the size.
pub fn screen(
    rpc: &dyn RpcProvider,
    slot: u64,
    signature: &str,
    required: &BTreeSet<String>,
) -> Result<SlotScreening> {
    let block = rpc.call(
        "getBlock",
        json!([
            slot,
            {
                "encoding": "json",
                "transactionDetails": "accounts",
                "rewards": false,
                "commitment": "finalized",
                // Screening has to *see* every transaction in the slot to know
                // whether one of them writes a required account. A block
                // containing a version this client will not deserialize is
                // refused wholesale, which would make a clean slot unscreenable.
                // Raising the ceiling changes only what the RPC will return;
                // which transactions may be *replayed* is decided by the
                // adapter's own message rule, not here.
                "maxSupportedTransactionVersion": 1
            }
        ]),
    )?;
    anyhow::ensure!(
        !block.is_null(),
        "block {slot} is unavailable, so same-slot interference cannot be ruled out"
    );
    let transactions = block["transactions"]
        .as_array()
        .context("block response contains no transactions")?;
    let target_index = transactions
        .iter()
        .position(|entry| signature_of(entry) == Some(signature))
        .with_context(|| {
            format!(
                "transaction {signature} is not in block {slot}; the slot is wrong \
                     or the block was replaced"
            )
        })?;

    // First writer wins the report: naming one conflicting transaction per
    // account keeps the rejection readable, and the earliest is the one that
    // actually invalidates the pre-state.
    let mut conflicts: BTreeMap<String, AccountConflict> = BTreeMap::new();
    for (index, entry) in transactions.iter().enumerate() {
        if index == target_index {
            continue;
        }
        let writable = writable_keys(entry)?;
        for account in writable.intersection(required) {
            let position = if index < target_index {
                ConflictPosition::Before
            } else {
                ConflictPosition::After
            };
            let conflict = AccountConflict {
                account: account.clone(),
                target_signature: signature.to_string(),
                target_index,
                conflicting_signature: signature_of(entry).map(str::to_string),
                conflicting_index: Some(index),
                position: Some(position),
                reason: format!("transaction at index {index} {}", position.explain()),
            };
            conflicts.entry(account.clone()).or_insert(conflict);
        }
    }

    Ok(SlotScreening {
        slot,
        target_signature: signature.to_string(),
        target_index,
        transactions_in_slot: transactions.len(),
        required_accounts: required.iter().cloned().collect(),
        conflicts: conflicts.into_values().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    struct StaticRpc(Value);
    impl RpcProvider for StaticRpc {
        fn call(&self, _method: &str, _params: Value) -> Result<Value> {
            Ok(self.0.clone())
        }
    }

    fn entry(signature: &str, writable: &[&str], readonly: &[&str]) -> Value {
        let keys: Vec<Value> = writable
            .iter()
            .map(|key| json!({"pubkey": key, "writable": true, "signer": false}))
            .chain(
                readonly
                    .iter()
                    .map(|key| json!({"pubkey": key, "writable": false, "signer": false})),
            )
            .collect();
        json!({"transaction": {"signatures": [signature], "accountKeys": keys}, "meta": {"err": null}})
    }

    fn required(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn an_untouched_slot_screens_clean() {
        let rpc = StaticRpc(json!({"transactions": [
            entry("other", &["unrelated"], &[]),
            entry("target", &["pool", "reserve"], &[]),
        ]}));
        let screening = screen(&rpc, 7, "target", &required(&["pool", "reserve"])).unwrap();
        assert!(screening.is_clean());
        assert_eq!(screening.target_index, 1);
        assert_eq!(screening.transactions_in_slot, 2);
        screening.ensure_unambiguous().unwrap();
    }

    /// The rejection has to name the account, the conflicting transaction and
    /// which side of the boundary it spoils - that is what makes it actionable.
    #[test]
    fn an_earlier_writer_invalidates_the_pre_state() {
        let rpc = StaticRpc(json!({"transactions": [
            entry("earlier", &["pool"], &[]),
            entry("target", &["pool"], &[]),
        ]}));
        let screening = screen(&rpc, 7, "target", &required(&["pool"])).unwrap();
        assert_eq!(screening.conflicts.len(), 1);
        let conflict = &screening.conflicts[0];
        assert_eq!(conflict.account, "pool");
        assert_eq!(conflict.conflicting_signature.as_deref(), Some("earlier"));
        assert_eq!(conflict.conflicting_index, Some(0));
        assert_eq!(conflict.position, Some(ConflictPosition::Before));
        let error = screening.ensure_unambiguous().unwrap_err().to_string();
        assert!(error.contains("pool"), "{error}");
        assert!(
            error.contains("earlier") || error.contains("index 0"),
            "{error}"
        );
    }

    #[test]
    fn a_later_writer_invalidates_the_post_state() {
        let rpc = StaticRpc(json!({"transactions": [
            entry("target", &["reserve"], &[]),
            entry("later", &["reserve"], &[]),
        ]}));
        let screening = screen(&rpc, 7, "target", &required(&["reserve"])).unwrap();
        assert_eq!(
            screening.conflicts[0].position,
            Some(ConflictPosition::After)
        );
        assert!(screening.ensure_unambiguous().is_err());
    }

    /// A shared read-only key is not interference: nothing writes it.
    #[test]
    fn a_shared_read_only_account_is_not_a_conflict() {
        let rpc = StaticRpc(json!({"transactions": [
            entry("other", &["unrelated"], &["mint"]),
            entry("target", &["pool"], &["mint"]),
        ]}));
        let screening = screen(&rpc, 7, "target", &required(&["pool", "mint"])).unwrap();
        assert!(screening.is_clean());
    }

    /// A transaction that a slot does not contain cannot be screened, and
    /// silently screening clean would be the worst possible answer.
    #[test]
    fn a_missing_target_is_an_error() {
        let rpc = StaticRpc(json!({"transactions": [entry("other", &["pool"], &[])]}));
        let error = screen(&rpc, 7, "target", &required(&["pool"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("not in block"), "{error}");
    }

    #[test]
    fn an_unavailable_block_is_an_error() {
        let rpc = StaticRpc(Value::Null);
        let error = screen(&rpc, 7, "target", &required(&["pool"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("unavailable"), "{error}");
    }

    /// Only one conflict is reported per account, so a hot account written by
    /// hundreds of transactions does not bury the rest of the report.
    #[test]
    fn one_conflict_is_reported_per_account() {
        let rpc = StaticRpc(json!({"transactions": [
            entry("first", &["tip"], &[]),
            entry("second", &["tip"], &[]),
            entry("target", &["tip"], &[]),
            entry("third", &["tip"], &[]),
        ]}));
        let screening = screen(&rpc, 7, "target", &required(&["tip"])).unwrap();
        assert_eq!(screening.conflicts.len(), 1);
        assert_eq!(
            screening.conflicts[0].conflicting_signature.as_deref(),
            Some("first")
        );
    }
}
