//! Account pairing, snapshot proof, and lifecycle.
//!
//! The pre/post pairing rules live here so that no adapter rebuilds them. They
//! are explicit rather than implied, because each has a wrong answer that looks
//! plausible:
//!
//! - accounts pair by **address-backed label**, never by position. Two
//!   executions may present their accounts in different orders, and pairing by
//!   index would compare a mint against a token account and report the
//!   difference as an economic change.
//! - an account present after and absent before is **created**, not modified
//!   from an empty one. A created account has no "before" to subtract from, and
//!   reporting its whole balance as a delta would attribute a fresh rent
//!   deposit to the protocol as a transfer.
//! - an account present before and absent after is **closed**.
//! - an account in neither is not evidence of anything and produces no item.

use super::{DecoderIdentity, Provenance};
use crate::{executor::ExecutionResult, types::AccountSnapshot, types::NamedAccount};
use std::collections::BTreeMap;

const PAIRING: DecoderIdentity = DecoderIdentity::standard("account-pairing", 1);

/// How an account existed on each side of the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existence {
    /// Present before and after.
    Throughout,
    /// Absent before, present after.
    Created,
    /// Present before, absent after.
    Closed,
}

/// One account, paired across a boundary.
///
/// `before` and `after` are `None` exactly where the account did not exist. A
/// caller that wants a delta must decide what a missing side means; nothing
/// here silently substitutes a zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountDelta {
    pub provenance: Provenance,
    pub existence: Existence,
    pub before: Option<AccountSnapshot>,
    pub after: Option<AccountSnapshot>,
}

impl AccountDelta {
    pub fn label(&self) -> &str {
        &self.provenance.account_label
    }

    /// Lamport change, or `None` where one side did not exist.
    ///
    /// Deliberately `None` rather than the full balance for a created account:
    /// see the module note. A creation is reported by [`LifecycleEvent`].
    pub fn lamport_delta(&self) -> Option<i128> {
        let (before, after) = (self.before.as_ref()?, self.after.as_ref()?);
        Some(i128::from(after.lamports) - i128::from(before.lamports))
    }

    pub fn data_changed(&self) -> bool {
        match (&self.before, &self.after) {
            (Some(before), Some(after)) => before.data != after.data,
            _ => false,
        }
    }

    pub fn owner_changed(&self) -> bool {
        match (&self.before, &self.after) {
            (Some(before), Some(after)) => before.owner != after.owner,
            _ => false,
        }
    }

    pub fn executable_changed(&self) -> bool {
        match (&self.before, &self.after) {
            (Some(before), Some(after)) => before.executable != after.executable,
            _ => false,
        }
    }

    pub fn data_len_changed(&self) -> bool {
        match (&self.before, &self.after) {
            (Some(before), Some(after)) => before.data.len() != after.data.len(),
            _ => false,
        }
    }

    /// Offset of the first differing byte, where both sides exist and differ.
    pub fn first_data_difference(&self) -> Option<usize> {
        let (before, after) = (self.before.as_ref()?, self.after.as_ref()?);
        before
            .data
            .iter()
            .zip(after.data.iter())
            .position(|(a, b)| a != b)
            .or_else(|| {
                (before.data.len() != after.data.len())
                    .then(|| before.data.len().min(after.data.len()))
            })
    }

    /// The owner on whichever side the account exists, preferring the later.
    pub fn owner(&self) -> Option<&str> {
        self.after
            .as_ref()
            .or(self.before.as_ref())
            .map(|snapshot| snapshot.owner.as_str())
    }
}

/// An account's existence or ownership changing.
///
/// Reported as a fact with no meaning attached. A created account may be a user
/// opening a position or a protocol parking rent; this layer does not know and
/// does not guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    AccountCreated {
        provenance: Provenance,
        lamports: u64,
        owner: String,
        data_len: usize,
    },
    AccountClosed {
        provenance: Provenance,
        lamports_before: u64,
        owner_before: String,
    },
    OwnerChanged {
        provenance: Provenance,
        before: String,
        after: String,
    },
    ExecutableChanged {
        provenance: Provenance,
        before: bool,
        after: bool,
    },
    DataLengthChanged {
        provenance: Provenance,
        before: usize,
        after: usize,
    },
}

impl LifecycleEvent {
    pub fn provenance(&self) -> &Provenance {
        match self {
            Self::AccountCreated { provenance, .. }
            | Self::AccountClosed { provenance, .. }
            | Self::OwnerChanged { provenance, .. }
            | Self::ExecutableChanged { provenance, .. }
            | Self::DataLengthChanged { provenance, .. } => provenance,
        }
    }

    pub fn label(&self) -> &str {
        &self.provenance().account_label
    }
}

/// Pair a pre-state against an execution's post-state.
///
/// Output is ordered by label, so two callers that assembled their inputs in
/// different orders produce identical evidence. Determinism here is what lets a
/// canonical report be compared byte for byte.
pub fn pair(record: &str, pre: &[NamedAccount], result: &ExecutionResult) -> Vec<AccountDelta> {
    // Keyed by label. The label is address-backed — the protocol adapter binds
    // it from the message's account keys — so this is address pairing expressed
    // in the vocabulary the rest of the engine already uses. Pairing by
    // position would be the bug; see the module note.
    let mut addresses: BTreeMap<&str, &str> = BTreeMap::new();
    let mut before: BTreeMap<&str, &AccountSnapshot> = BTreeMap::new();
    for named in pre {
        before.insert(named.label.as_str(), &named.account);
        addresses.insert(named.label.as_str(), named.address.as_str());
    }

    let labels: std::collections::BTreeSet<&str> = before
        .keys()
        .copied()
        .chain(result.accounts.keys().map(String::as_str))
        .collect();

    labels
        .into_iter()
        .filter_map(|label| {
            let opening = before.get(label).copied().cloned();
            let closing = result.accounts.get(label).cloned();
            let existence = match (&opening, &closing) {
                (Some(_), Some(_)) => Existence::Throughout,
                (None, Some(_)) => Existence::Created,
                (Some(_), None) => Existence::Closed,
                // In neither. Not evidence of anything.
                (None, None) => return None,
            };
            Some(AccountDelta {
                provenance: Provenance {
                    record: record.to_string(),
                    account_label: label.to_string(),
                    address: addresses.get(label).map(|a| a.to_string()),
                    decoder: PAIRING,
                    origin: None,
                },
                existence,
                before: opening,
                after: closing,
            })
        })
        .collect()
}

/// Every existence or ownership change among paired accounts.
pub fn lifecycle(deltas: &[AccountDelta]) -> Vec<LifecycleEvent> {
    let mut events = Vec::new();
    for delta in deltas {
        match (&delta.before, &delta.after) {
            (None, Some(after)) => events.push(LifecycleEvent::AccountCreated {
                provenance: delta.provenance.clone(),
                lamports: after.lamports,
                owner: after.owner.clone(),
                data_len: after.data.len(),
            }),
            (Some(before), None) => events.push(LifecycleEvent::AccountClosed {
                provenance: delta.provenance.clone(),
                lamports_before: before.lamports,
                owner_before: before.owner.clone(),
            }),
            (Some(before), Some(after)) => {
                if before.owner != after.owner {
                    events.push(LifecycleEvent::OwnerChanged {
                        provenance: delta.provenance.clone(),
                        before: before.owner.clone(),
                        after: after.owner.clone(),
                    });
                }
                if before.executable != after.executable {
                    events.push(LifecycleEvent::ExecutableChanged {
                        provenance: delta.provenance.clone(),
                        before: before.executable,
                        after: after.executable,
                    });
                }
                if before.data.len() != after.data.len() {
                    events.push(LifecycleEvent::DataLengthChanged {
                        provenance: delta.provenance.clone(),
                        before: before.data.len(),
                        after: after.data.len(),
                    });
                }
            }
            (None, None) => {}
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::tests::{execution, named, snapshot};

    const SYSTEM: &str = "11111111111111111111111111111111";

    /// The caller's order is deliberately *not* alphabetical here.
    ///
    /// An earlier version of this test used `first` and `second`, supplied in
    /// that order — and since the post-state is a label-keyed `BTreeMap`, index
    /// order and label order coincided, so pairing by position passed it. The
    /// labels below sort in the opposite order to the vector, which is what
    /// makes the two strategies produce different answers.
    #[test]
    fn accounts_pair_by_label_not_by_position() {
        let pre = vec![
            named("zebra", SYSTEM, 10, vec![1]),
            named("alpha", SYSTEM, 20, vec![2]),
        ];
        let result = execution(vec![
            ("alpha", snapshot(SYSTEM, 25, vec![2])),
            ("zebra", snapshot(SYSTEM, 10, vec![1])),
        ]);
        let deltas = pair("r", &pre, &result);
        assert_eq!(deltas.len(), 2);
        let zebra = deltas.iter().find(|d| d.label() == "zebra").unwrap();
        let alpha = deltas.iter().find(|d| d.label() == "alpha").unwrap();
        // Pairing by position would give zebra the +5 and alpha the 0.
        assert_eq!(zebra.lamport_delta(), Some(0));
        assert_eq!(alpha.lamport_delta(), Some(5));
        assert_eq!(zebra.before.as_ref().unwrap().data, vec![1]);
        assert_eq!(alpha.before.as_ref().unwrap().data, vec![2]);
    }

    #[test]
    fn a_created_account_is_not_a_modified_empty_one() {
        let pre: Vec<NamedAccount> = Vec::new();
        let result = execution(vec![("fresh", snapshot(SYSTEM, 2_039_280, vec![0; 165]))]);
        let deltas = pair("r", &pre, &result);
        assert_eq!(deltas[0].existence, Existence::Created);
        assert_eq!(deltas[0].before, None);
        // The rule that matters: no delta is invented. Reporting 2_039_280
        // lamports of "increase" would attribute a rent deposit to the protocol.
        assert_eq!(deltas[0].lamport_delta(), None);
        assert!(
            !deltas[0].data_changed(),
            "there is nothing to have changed"
        );

        let events = lifecycle(&deltas);
        assert!(matches!(
            events.as_slice(),
            [LifecycleEvent::AccountCreated { lamports, data_len, .. }]
                if *lamports == 2_039_280 && *data_len == 165
        ));
    }

    #[test]
    fn a_closed_account_is_reported_as_closed() {
        let pre = vec![named("gone", SYSTEM, 500, vec![7])];
        let result = execution(Vec::new());
        let deltas = pair("r", &pre, &result);
        assert_eq!(deltas[0].existence, Existence::Closed);
        assert_eq!(deltas[0].after, None);
        assert_eq!(deltas[0].lamport_delta(), None);
        assert!(matches!(
            lifecycle(&deltas).as_slice(),
            [LifecycleEvent::AccountClosed {
                lamports_before: 500,
                ..
            }]
        ));
    }

    #[test]
    fn an_owner_change_is_a_lifecycle_event() {
        let pre = vec![named("taken", SYSTEM, 1, vec![0])];
        let result = execution(vec![("taken", snapshot("TokenkegQ", 1, vec![0]))]);
        let deltas = pair("r", &pre, &result);
        assert!(deltas[0].owner_changed());
        assert!(matches!(
            lifecycle(&deltas).as_slice(),
            [LifecycleEvent::OwnerChanged { .. }]
        ));
    }

    #[test]
    fn an_executable_change_and_a_length_change_are_both_reported() {
        let pre = vec![named("prog", SYSTEM, 1, vec![0; 4])];
        let mut after = snapshot(SYSTEM, 1, vec![0; 8]);
        after.executable = true;
        let result = execution(vec![("prog", after)]);
        let deltas = pair("r", &pre, &result);
        assert!(deltas[0].executable_changed());
        assert!(deltas[0].data_len_changed());
        let events = lifecycle(&deltas);
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .any(|e| matches!(e, LifecycleEvent::ExecutableChanged { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, LifecycleEvent::DataLengthChanged { .. })));
    }

    #[test]
    fn an_account_in_neither_side_produces_nothing() {
        let deltas = pair("r", &[], &execution(Vec::new()));
        assert!(deltas.is_empty());
        assert!(lifecycle(&deltas).is_empty());
    }

    #[test]
    fn the_first_differing_offset_is_reported() {
        let pre = vec![named("a", SYSTEM, 1, vec![1, 2, 3, 4])];
        let result = execution(vec![("a", snapshot(SYSTEM, 1, vec![1, 2, 9, 4]))]);
        let deltas = pair("r", &pre, &result);
        assert_eq!(deltas[0].first_data_difference(), Some(2));
    }

    #[test]
    fn a_shortened_buffer_reports_the_truncation_point() {
        let pre = vec![named("a", SYSTEM, 1, vec![1, 2, 3, 4])];
        let result = execution(vec![("a", snapshot(SYSTEM, 1, vec![1, 2]))]);
        let deltas = pair("r", &pre, &result);
        assert_eq!(deltas[0].first_data_difference(), Some(2));
    }

    #[test]
    fn provenance_travels_with_every_delta() {
        let pre = vec![named("here", SYSTEM, 1, vec![0])];
        let result = execution(vec![("here", snapshot(SYSTEM, 2, vec![0]))]);
        let deltas = pair("record-42", &pre, &result);
        assert_eq!(deltas[0].provenance.record, "record-42");
        assert_eq!(deltas[0].provenance.account_label, "here");
        assert_eq!(
            deltas[0].provenance.address.as_deref(),
            Some("address-of-here")
        );
        assert_eq!(deltas[0].provenance.decoder.name, "account-pairing");
    }
}
