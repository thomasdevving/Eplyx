//! Naming a message's account keys.
//!
//! Labels are how diffs and reports refer to accounts, so they have to be
//! derived from the transaction's structure rather than from an address, and
//! they have to be unique within one record. Both adapters implemented that and
//! the implementations were 68% line-identical.
//!
//! The mechanism is universal; the bindings are not. An adapter says "the
//! account at position 4 of this instruction is the destination pool token
//! account"; this module turns a list of such statements into a stable set of
//! names.

/// One account's role, as an adapter binds it.
///
/// Order matters: the first binding for an address is the one that sticks. One
/// account can hold two roles — a depositor who also takes the referral
/// position is the common case — and the first role named is the one that
/// explains what it is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleBinding {
    pub address: String,
    pub role: String,
}

impl RoleBinding {
    pub fn new(address: impl Into<String>, role: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            role: role.into(),
        }
    }
}

/// The prefix an unclaimed key keeps.
///
/// Public because "is this key still unnamed?" is the test both the binding
/// loop and the fee-payer fallback make, and spelling it twice is how the two
/// drift apart.
pub const UNNAMED_PREFIX: &str = "key-";

fn is_unnamed(label: &str) -> bool {
    label.starts_with(UNNAMED_PREFIX)
}

/// Assign a label to every message key.
///
/// Every key starts as `key-{index}`. Each binding, in order, names the first
/// key with a matching address that is still unnamed. Finally key 0 — always
/// the fee payer — becomes `payer` if no role claimed it, so an authority that
/// also pays keeps the role that explains what it is doing.
///
/// A binding whose address is not a message key is ignored rather than being an
/// error: an adapter may bind roles from an instruction whose accounts were
/// resolved elsewhere, and a missing one leaves a positional label rather than
/// a wrong one.
pub fn assign<'a>(
    addresses: impl IntoIterator<Item = &'a str>,
    bindings: &[RoleBinding],
) -> Vec<String> {
    let addresses: Vec<&str> = addresses.into_iter().collect();
    let mut labels: Vec<String> = (0..addresses.len())
        .map(|index| format!("{UNNAMED_PREFIX}{index}"))
        .collect();

    for binding in bindings {
        let Some(index) = addresses
            .iter()
            .position(|address| *address == binding.address)
        else {
            continue;
        };
        if !is_unnamed(&labels[index]) {
            continue;
        }
        labels[index] = binding.role.clone();
    }

    if let Some(label) = labels.first_mut() {
        if is_unnamed(label) {
            *label = "payer".into();
        }
    }
    labels
}

/// The label at `index`, defaulting to the positional name.
///
/// Both adapters implemented this identically — six lines, 100% duplicated.
pub fn at(labels: &[String], index: usize) -> String {
    labels
        .get(index)
        .cloned()
        .unwrap_or_else(|| format!("{UNNAMED_PREFIX}{index}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbound_keys_keep_positional_labels() {
        assert_eq!(
            assign(["a", "b", "c"], &[]),
            // Key 0 is the fee payer; nothing claimed it.
            vec!["payer", "key-1", "key-2"]
        );
    }

    #[test]
    fn a_bound_key_takes_its_role() {
        let labels = assign(
            ["payer-key", "source-key", "dest-key"],
            &[
                RoleBinding::new("source-key", "source"),
                RoleBinding::new("dest-key", "destination"),
            ],
        );
        assert_eq!(labels, vec!["payer", "source", "destination"]);
    }

    /// The rule that keeps a label explaining what an account is doing: the
    /// first role named for an address is the one that sticks.
    #[test]
    fn the_first_role_named_for_an_address_wins() {
        let labels = assign(
            ["depositor", "other"],
            &[
                RoleBinding::new("depositor", "depositor"),
                RoleBinding::new("depositor", "referral-fee"),
            ],
        );
        assert_eq!(labels[0], "depositor");
    }

    /// An authority that also pays keeps the role, not the fallback.
    #[test]
    fn a_role_on_key_zero_beats_the_payer_fallback() {
        let labels = assign(
            ["authority", "other"],
            &[RoleBinding::new("authority", "authority")],
        );
        assert_eq!(labels[0], "authority");
    }

    #[test]
    fn a_binding_for_an_absent_address_is_ignored() {
        let labels = assign(["a", "b"], &[RoleBinding::new("missing", "source")]);
        assert_eq!(labels, vec!["payer", "key-1"]);
    }

    #[test]
    fn labels_are_unique_by_construction() {
        let labels = assign(
            ["a", "b", "c", "d"],
            &[
                RoleBinding::new("a", "source"),
                RoleBinding::new("b", "source-1"),
                RoleBinding::new("c", "destination"),
            ],
        );
        let unique: std::collections::BTreeSet<&String> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "{labels:?}");
    }

    #[test]
    fn an_index_past_the_end_falls_back_to_a_positional_name() {
        let labels = assign(["a"], &[]);
        assert_eq!(at(&labels, 0), "payer");
        assert_eq!(at(&labels, 7), "key-7");
    }

    #[test]
    fn an_empty_message_produces_no_labels() {
        assert!(assign(Vec::<&str>::new(), &[]).is_empty());
    }

    /// Two instructions of the same family: the adapter disambiguates by
    /// producing distinct role names, and this layer keeps them distinct.
    #[test]
    fn ordinal_suffixed_roles_stay_separate() {
        let labels = assign(
            ["payer-key", "s0", "d0", "s1", "d1"],
            &[
                RoleBinding::new("s0", "source-0"),
                RoleBinding::new("d0", "destination-0"),
                RoleBinding::new("s1", "source-1"),
                RoleBinding::new("d1", "destination-1"),
            ],
        );
        assert_eq!(
            labels,
            vec![
                "payer",
                "source-0",
                "destination-0",
                "source-1",
                "destination-1"
            ]
        );
    }
}
