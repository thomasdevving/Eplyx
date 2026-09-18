//! Lamport movement, measured.
//!
//! The rule that shapes this module: **not every lamport delta is an economic
//! transfer.** A fee payer loses lamports on every transaction, including one
//! that does nothing; a rent-exempt account gains them when it is created; the
//! runtime writes sysvars every slot. Attributing any of those to a protocol
//! would produce a finding about the fee schedule and call it a change in user
//! outcomes.
//!
//! So a delta carries an [`Attribution`], and the ones that are not protocol
//! movement say so. The classification is conservative in the direction that
//! matters: a delta this layer cannot attribute is
//! [`Attribution::Unattributed`], never silently promoted to economic.

use super::{account::AccountDelta, Provenance};

/// Addresses the runtime manages, whose balances move for reasons no protocol
/// chose.
///
/// Sysvars are rewritten every slot. Excluding them is not a convenience: their
/// balances carry no information about what a transaction did, and including
/// them would make every observation differ.
const RUNTIME_MANAGED: [&str; 8] = [
    "SysvarC1ock11111111111111111111111111111111",
    "SysvarStakeHistory1111111111111111111111111",
    "SysvarRent111111111111111111111111111111111",
    "SysvarRecentB1ockHashes11111111111111111111",
    "SysvarEpochSchedu1e111111111111111111111111",
    "SysvarFees111111111111111111111111111111111",
    "SysvarS1otHashes111111111111111111111111111",
    "Sysvar1nstructions1111111111111111111111111",
];

pub fn is_runtime_managed(address: &str) -> bool {
    RUNTIME_MANAGED.contains(&address)
}

/// Why an account's lamports moved, as far as this layer can establish it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Attribution {
    /// The transaction's fee payer. Part of its balance change is the fee,
    /// which is not protocol economics — this is why the fixture corpus pays
    /// fees from an account that is never the position owner.
    FeePayer,
    /// A sysvar or other runtime-written account.
    RuntimeManaged,
    /// The account did not exist on one side, so the change is its creation or
    /// closure rather than a movement.
    Lifecycle,
    /// Lamports moved and this layer has not established why. The honest
    /// default: a protocol adapter may know, and this layer does not guess.
    Unattributed,
}

/// One account's lamport balance across a boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LamportDelta {
    pub provenance: Provenance,
    pub before: u64,
    pub after: u64,
    pub delta: i128,
    pub attribution: Attribution,
}

impl LamportDelta {
    pub fn label(&self) -> &str {
        &self.provenance.account_label
    }

    /// Whether this delta may be read as value the transaction moved.
    ///
    /// False for fee payers, runtime-managed accounts and lifecycle changes.
    /// True does **not** mean the movement is economically meaningful to a
    /// user — only that nothing known disqualifies it. That judgement belongs
    /// to a protocol adapter.
    pub fn may_be_value_movement(&self) -> bool {
        self.attribution == Attribution::Unattributed
    }
}

/// Lamport deltas for every paired account whose balance changed.
///
/// Accounts whose balance did not move are omitted: a lamport balance that
/// stayed put is the overwhelmingly common case and reporting all of them would
/// bury the ones that did.
pub fn deltas(accounts: &[AccountDelta]) -> Vec<LamportDelta> {
    accounts
        .iter()
        .filter_map(|account| {
            let address = account.provenance.address.as_deref().unwrap_or("");
            let (Some(before), Some(after)) = (account.before.as_ref(), account.after.as_ref())
            else {
                // A created or closed account's whole balance is not a delta.
                return None;
            };
            if before.lamports == after.lamports {
                return None;
            }
            let attribution = if is_runtime_managed(address) {
                Attribution::RuntimeManaged
            } else {
                Attribution::Unattributed
            };
            Some(LamportDelta {
                provenance: account.provenance.clone(),
                before: before.lamports,
                after: after.lamports,
                delta: i128::from(after.lamports) - i128::from(before.lamports),
                attribution,
            })
        })
        .collect()
}

/// Re-attribute the delta belonging to a named fee payer.
///
/// Separate from [`deltas`] because who paid is a property of the transaction,
/// not of the account. A caller that knows the fee payer applies it; one that
/// does not leaves every delta unattributed rather than guessing.
pub fn attribute_fee_payer(deltas: &mut [LamportDelta], fee_payer_label: &str) {
    for delta in deltas.iter_mut() {
        if delta.provenance.account_label == fee_payer_label {
            delta.attribution = Attribution::FeePayer;
        }
    }
}

/// Mark deltas whose account was created or closed.
pub fn attribute_lifecycle(deltas: &mut [LamportDelta], lifecycle_labels: &[&str]) {
    for delta in deltas.iter_mut() {
        if lifecycle_labels.contains(&delta.provenance.account_label.as_str()) {
            delta.attribution = Attribution::Lifecycle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{
        account::pair,
        tests::{execution, named, snapshot},
    };
    use crate::types::NamedAccount;

    const SYSTEM: &str = "11111111111111111111111111111111";

    fn account_at(label: &str, address: &str, lamports: u64) -> NamedAccount {
        NamedAccount {
            label: label.into(),
            address: address.into(),
            account: snapshot(SYSTEM, lamports, Vec::new()),
        }
    }

    #[test]
    fn increases_and_decreases_are_both_measured() {
        let pre = vec![named("a", SYSTEM, 1_000, Vec::new())];
        let up = deltas(&pair(
            "r",
            &pre,
            &execution(vec![("a", snapshot(SYSTEM, 1_500, Vec::new()))]),
        ));
        assert_eq!(up[0].delta, 500);
        let down = deltas(&pair(
            "r",
            &pre,
            &execution(vec![("a", snapshot(SYSTEM, 250, Vec::new()))]),
        ));
        assert_eq!(down[0].delta, -750);
    }

    #[test]
    fn an_unchanged_balance_produces_no_delta() {
        let pre = vec![named("a", SYSTEM, 1_000, Vec::new())];
        let same = deltas(&pair(
            "r",
            &pre,
            &execution(vec![("a", snapshot(SYSTEM, 1_000, Vec::new()))]),
        ));
        assert!(same.is_empty());
    }

    /// The rule the fixture corpus is built around: a fee payer's balance
    /// change is not protocol economics.
    #[test]
    fn a_fee_payer_delta_is_not_value_movement() {
        let pre = vec![named("payer", SYSTEM, 1_000_000, Vec::new())];
        let mut measured = deltas(&pair(
            "r",
            &pre,
            &execution(vec![("payer", snapshot(SYSTEM, 995_000, Vec::new()))]),
        ));
        assert!(
            measured[0].may_be_value_movement(),
            "unattributed until told"
        );
        attribute_fee_payer(&mut measured, "payer");
        assert_eq!(measured[0].attribution, Attribution::FeePayer);
        assert!(!measured[0].may_be_value_movement());
    }

    /// Sysvars are rewritten every slot; their balances say nothing about a
    /// transaction. This exclusion already exists in the boundary prover and
    /// must survive the move.
    #[test]
    fn runtime_managed_accounts_are_excluded_from_value_movement() {
        let clock = "SysvarC1ock11111111111111111111111111111111";
        let pre = vec![account_at("clock-sysvar", clock, 1_000)];
        let result = execution(vec![("clock-sysvar", snapshot(SYSTEM, 1_001, Vec::new()))]);
        let measured = deltas(&pair("r", &pre, &result));
        assert_eq!(measured[0].attribution, Attribution::RuntimeManaged);
        assert!(!measured[0].may_be_value_movement());
        assert!(is_runtime_managed(clock));
        assert!(!is_runtime_managed(SYSTEM));
    }

    #[test]
    fn a_created_account_produces_no_lamport_delta() {
        let result = execution(vec![("fresh", snapshot(SYSTEM, 2_039_280, Vec::new()))]);
        assert!(deltas(&pair("r", &[], &result)).is_empty());
    }

    #[test]
    fn a_lifecycle_attribution_can_be_applied_after_the_fact() {
        let pre = vec![named("a", SYSTEM, 10, Vec::new())];
        let mut measured = deltas(&pair(
            "r",
            &pre,
            &execution(vec![("a", snapshot(SYSTEM, 20, Vec::new()))]),
        ));
        attribute_lifecycle(&mut measured, &["a"]);
        assert_eq!(measured[0].attribution, Attribution::Lifecycle);
        assert!(!measured[0].may_be_value_movement());
    }

    #[test]
    fn a_full_range_change_does_not_overflow() {
        let pre = vec![named("a", SYSTEM, u64::MAX, Vec::new())];
        let measured = deltas(&pair(
            "r",
            &pre,
            &execution(vec![("a", snapshot(SYSTEM, 0, Vec::new()))]),
        ));
        assert_eq!(measured[0].delta, -i128::from(u64::MAX));
    }
}
