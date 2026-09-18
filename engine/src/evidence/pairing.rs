//! Pairing two executions of the same observation.
//!
//! The walk — for every account both runs produced, decode both sides, compare
//! the economic fields, emit what differs — was 84% line-identical between the
//! two adapters. It is mechanism: nothing in it knows what any field means.
//!
//! What stays with the adapter is the pair of hooks this takes: how to decode an
//! account, and how to render a decoded quantity in the units a reader expects.
//! Those are exactly the two things the adapters genuinely disagreed about.
//!
//! ## Why this module names protocol types
//!
//! [`SemanticAccount`] and [`EconomicChange`] live in [`crate::protocol`]
//! because they are the seam's vocabulary. They are containers, not knowledge:
//! neither one names a deposit, a fee or a health factor. Using them here keeps
//! the mechanism in the evidence layer without moving serde types that a bundle
//! already depends on.

use crate::{
    executor::ExecutionResult,
    protocol::{EconomicChange, FieldValue, SemanticAccount, TokenQuantity},
    types::{AccountSnapshot, NamedAccount},
};

/// Compare two executions field by field, under an adapter's decoder.
///
/// `decode` reads an account; `rescale` converts a decoded quantity into the
/// units a reader expects, given the field's name. An account either side fails
/// to decode is skipped — there is nothing to compare — and a field present on
/// only one side is skipped for the same reason.
///
/// Non-economic fields are not compared. They are decoded for context, and a
/// change in one must not drive an economic verdict.
pub fn compare_decoded<D, R>(
    accounts: &[NamedAccount],
    v1: &ExecutionResult,
    v2: &ExecutionResult,
    decode: D,
    rescale: R,
) -> Vec<EconomicChange>
where
    D: Fn(&AccountSnapshot) -> Option<SemanticAccount>,
    R: Fn(&str, TokenQuantity) -> TokenQuantity,
{
    let mut changes = Vec::new();
    for named in accounts {
        let (Some(after_v1), Some(after_v2)) =
            (v1.accounts.get(&named.label), v2.accounts.get(&named.label))
        else {
            continue;
        };
        let (Some(decoded_v1), Some(decoded_v2)) = (decode(after_v1), decode(after_v2)) else {
            continue;
        };
        for field in &decoded_v1.fields {
            if !field.economic {
                continue;
            }
            let Some(other) = decoded_v2.field(&field.name) else {
                continue;
            };
            if other.value == field.value {
                continue;
            }
            let render = |value: &FieldValue| match value.as_quantity() {
                Some(quantity) => rescale(&field.name, quantity).to_string(),
                None => value.render(),
            };
            let delta = match (field.value.as_quantity(), other.value.as_quantity()) {
                (Some(before), Some(after)) => {
                    rescale(&field.name, after).delta(rescale(&field.name, before))
                }
                _ => None,
            };
            changes.push(EconomicChange {
                account_label: named.label.clone(),
                account_kind: decoded_v1.kind.clone(),
                field: field.name.clone(),
                v1: render(&field.value),
                v2: render(&other.value),
                delta,
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        evidence::tests::{execution, named, snapshot},
        protocol::SemanticField,
    };

    const OWNER: &str = "11111111111111111111111111111111";

    /// A toy decoder: the first byte is an economic amount, the second a
    /// non-economic counter.
    fn decode(account: &AccountSnapshot) -> Option<SemanticAccount> {
        let amount = *account.data.first()?;
        let counter = *account.data.get(1)?;
        Some(SemanticAccount {
            kind: "toy".into(),
            fields: vec![
                SemanticField {
                    name: "amount".into(),
                    value: FieldValue::quantity(u64::from(amount), 0),
                    economic: true,
                },
                SemanticField {
                    name: "counter".into(),
                    value: FieldValue::Count(u64::from(counter)),
                    economic: false,
                },
            ],
        })
    }

    fn identity(_: &str, quantity: TokenQuantity) -> TokenQuantity {
        quantity
    }

    #[test]
    fn a_changed_economic_field_is_reported_with_its_delta() {
        let accounts = vec![named("a", OWNER, 1, vec![10, 0])];
        let v1 = execution(vec![("a", snapshot(OWNER, 1, vec![10, 0]))]);
        let v2 = execution(vec![("a", snapshot(OWNER, 1, vec![14, 0]))]);
        let changes = compare_decoded(&accounts, &v1, &v2, decode, identity);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].field, "amount");
        assert_eq!(changes[0].account_label, "a");
        assert_eq!(changes[0].account_kind, "toy");
        assert_eq!(changes[0].delta.unwrap().base_units, 4);
    }

    #[test]
    fn an_unchanged_field_produces_nothing() {
        let accounts = vec![named("a", OWNER, 1, vec![10, 0])];
        let same = execution(vec![("a", snapshot(OWNER, 1, vec![10, 0]))]);
        assert!(compare_decoded(&accounts, &same, &same, decode, identity).is_empty());
    }

    /// Compute and bookkeeping fields are decoded for context. A change in one
    /// must not drive an economic verdict.
    #[test]
    fn a_non_economic_field_is_never_compared() {
        let accounts = vec![named("a", OWNER, 1, vec![10, 0])];
        let v1 = execution(vec![("a", snapshot(OWNER, 1, vec![10, 0]))]);
        let v2 = execution(vec![("a", snapshot(OWNER, 1, vec![10, 99]))]);
        assert!(compare_decoded(&accounts, &v1, &v2, decode, identity).is_empty());
    }

    #[test]
    fn an_account_only_one_side_produced_is_skipped() {
        let accounts = vec![named("a", OWNER, 1, vec![10, 0])];
        let v1 = execution(vec![("a", snapshot(OWNER, 1, vec![10, 0]))]);
        let v2 = execution(Vec::new());
        assert!(compare_decoded(&accounts, &v1, &v2, decode, identity).is_empty());
    }

    #[test]
    fn an_account_that_does_not_decode_is_skipped() {
        let accounts = vec![named("a", OWNER, 1, vec![10, 0])];
        let v1 = execution(vec![("a", snapshot(OWNER, 1, vec![10, 0]))]);
        let v2 = execution(vec![("a", snapshot(OWNER, 1, Vec::new()))]);
        assert!(compare_decoded(&accounts, &v1, &v2, decode, identity).is_empty());
    }

    /// The hook that differed between the two adapters: one rescales every
    /// quantity by the mint's decimals, the other only a named list.
    #[test]
    fn the_rescale_hook_governs_how_a_quantity_renders() {
        let accounts = vec![named("a", OWNER, 1, vec![10, 0])];
        let v1 = execution(vec![("a", snapshot(OWNER, 1, vec![10, 0]))]);
        let v2 = execution(vec![("a", snapshot(OWNER, 1, vec![14, 0]))]);

        let raw = compare_decoded(&accounts, &v1, &v2, decode, identity);
        assert_eq!(raw[0].v1, "10");
        assert_eq!(raw[0].v2, "14");

        let scaled = compare_decoded(&accounts, &v1, &v2, decode, |_, quantity| {
            TokenQuantity::new(quantity.base_units, 9)
        });
        assert_eq!(scaled[0].v1, "0.000000010");
        assert_eq!(scaled[0].v2, "0.000000014");
        assert_eq!(scaled[0].delta.unwrap().base_units, 4);
    }

    #[test]
    fn a_selective_rescale_leaves_other_fields_alone() {
        let accounts = vec![named("a", OWNER, 1, vec![10, 0])];
        let v1 = execution(vec![("a", snapshot(OWNER, 1, vec![10, 0]))]);
        let v2 = execution(vec![("a", snapshot(OWNER, 1, vec![14, 0]))]);
        let selective = compare_decoded(&accounts, &v1, &v2, decode, |field, quantity| {
            if field == "something_else" {
                TokenQuantity::new(quantity.base_units, 9)
            } else {
                quantity
            }
        });
        assert_eq!(selective[0].v1, "10");
    }

    #[test]
    fn accounts_are_reported_in_the_order_they_were_supplied() {
        let accounts = vec![
            named("z", OWNER, 1, vec![1, 0]),
            named("a", OWNER, 1, vec![1, 0]),
        ];
        let v1 = execution(vec![
            ("a", snapshot(OWNER, 1, vec![1, 0])),
            ("z", snapshot(OWNER, 1, vec![1, 0])),
        ]);
        let v2 = execution(vec![
            ("a", snapshot(OWNER, 1, vec![2, 0])),
            ("z", snapshot(OWNER, 1, vec![2, 0])),
        ]);
        let changes = compare_decoded(&accounts, &v1, &v2, decode, identity);
        assert_eq!(
            changes
                .iter()
                .map(|c| c.account_label.as_str())
                .collect::<Vec<_>>(),
            vec!["z", "a"],
            "caller order is the reporting order, and it is stable"
        );
    }
}
