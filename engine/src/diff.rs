//! Structural comparison of two execution results.
//!
//! Protocol-agnostic: this module compares success, balances, account bytes,
//! CPI shape and compute. Where an account decodes to a known type it defers to
//! `interpret` for field names and economic meaning; where it does not, it falls
//! back to a raw byte difference rather than staying silent.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::executor::ExecutionResult;
use crate::interpret::{self, Decoded, FieldValue};
use crate::types::{Category, Fixture};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::High => "HIGH",
            Severity::Critical => "CRITICAL",
        }
    }
}

/// Compute deltas below this magnitude are background noise: any recompilation
/// moves compute a little, and a few hundred CU on a 200k budget changes
/// nothing operationally. Basis points: 500 = 5%.
pub const COMPUTE_NOISE_BPS: i32 = 500;

/// Above this, a compute change is an operational risk in its own right - the
/// transaction may still succeed in isolation while becoming fragile inside a
/// larger transaction or under a tighter budget. Basis points: 3000 = 30%.
pub const COMPUTE_REGRESSION_BPS: i32 = 3_000;

/// Render basis points as a signed percentage: `-523` becomes `-5.23%`.
pub fn format_bps(bps: i32) -> String {
    let sign = if bps < 0 { "-" } else { "+" };
    let magnitude = bps.unsigned_abs();
    format!("{sign}{}.{:02}%", magnitude / 100, magnitude % 100)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Difference {
    /// The transaction succeeded under one version and failed under the other.
    SuccessChanged {
        v1_success: bool,
        v2_success: bool,
        v1_error: Option<String>,
        v2_error: Option<String>,
    },
    /// A position crossed the liquidation threshold in one version only.
    LiquidationStatusChanged {
        account: String,
        v1: bool,
        v2: bool,
        v1_health: String,
        v2_health: String,
    },
    /// A named field of a decoded account differs.
    FieldChanged {
        account: String,
        field: String,
        v1: String,
        v2: String,
        /// i64 rather than i128: `Difference` is an internally-tagged enum, and
        /// serde buffers those through a representation with no 128-bit integer
        /// variant, so an i128 here would serialise but refuse to deserialise.
        /// The JSON shape is unchanged, and no account field in reach of this
        /// engine produces a delta outside i64.
        delta: Option<i64>,
        consequence: Option<String>,
    },
    /// An account changed but could not be decoded; reported as raw bytes.
    RawDataChanged {
        account: String,
        offset: usize,
        v1: String,
        v2: String,
    },
    /// Lamport balance of a watched account differs.
    BalanceChanged {
        account: String,
        /// Decimal strings. A lamport balance passes 2^53 well inside `u64` -
        /// a pool holding 15 million SOL is 1.5e16 - and past that a JSON
        /// number rounds in most parsers.
        #[serde(with = "crate::numfmt::u64_string")]
        v1: u64,
        #[serde(with = "crate::numfmt::u64_string")]
        v2: u64,
        /// See the note on `FieldChanged::delta`. A lamport delta is bounded by
        /// total supply, far inside i64.
        #[serde(with = "crate::numfmt::i64_string")]
        delta: i64,
    },
    /// The cross-program invocation sequence differs.
    CpiChanged { v1: Vec<String>, v2: Vec<String> },
    ComputeChanged {
        v1: u64,
        v2: u64,
        delta: i64,
        /// Relative change in basis points (100 = 1%).
        ///
        /// Integer rather than a float so the report is exactly reproducible and
        /// survives a JSON round trip: an `f64` here did not, which is the same
        /// class of silent loss the monetary types exist to avoid.
        pct_bps: i32,
    },
}

impl Difference {
    pub fn severity(&self) -> Severity {
        match self {
            // Either direction is critical: a withdrawal that starts failing
            // strands users; one that starts succeeding bypasses a guard.
            Difference::SuccessChanged { .. } => Severity::Critical,
            Difference::LiquidationStatusChanged { .. } => Severity::Critical,
            Difference::BalanceChanged { .. } => Severity::High,
            Difference::CpiChanged { .. } => Severity::High,
            Difference::FieldChanged { field, .. } => match field.as_str() {
                "health_factor" | "collateral_amount" | "debt_amount" => Severity::High,
                _ => Severity::Warning,
            },
            Difference::RawDataChanged { .. } => Severity::Warning,
            Difference::ComputeChanged { pct_bps, .. } => {
                let magnitude = pct_bps.unsigned_abs();
                if magnitude < COMPUTE_NOISE_BPS as u32 {
                    Severity::Info
                } else if magnitude < COMPUTE_REGRESSION_BPS as u32 {
                    Severity::Warning
                } else {
                    Severity::High
                }
            }
        }
    }

    pub fn is_compute_only(&self) -> bool {
        matches!(self, Difference::ComputeChanged { .. })
    }

    /// Stable tag identifying the shape of the difference. Matches the `kind`
    /// discriminator in the JSON schema.
    pub fn kind(&self) -> &'static str {
        match self {
            Difference::SuccessChanged { .. } => "success_changed",
            Difference::LiquidationStatusChanged { .. } => "liquidation_status_changed",
            Difference::FieldChanged { .. } => "field_changed",
            Difference::RawDataChanged { .. } => "raw_data_changed",
            Difference::BalanceChanged { .. } => "balance_changed",
            Difference::CpiChanged { .. } => "cpi_changed",
            Difference::ComputeChanged { .. } => "compute_changed",
        }
    }
}

/// Headline bucket for a fixture.
///
/// Compute is deliberately kept off this axis. Every recompilation moves
/// compute units, so folding them in would classify the entire corpus as
/// "changed" and bury the question the tool exists to answer: did anything
/// change for users, positions or capital? Compute is reported separately, and
/// is still escalated on its own merits when the shift is large enough to be an
/// operational risk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// Nothing at all differed, compute included.
    Identical,
    /// State, balances, outcome and CPI shape are identical; only compute moved.
    ComputeOnly,
    /// Something observable to a user or a position changed.
    Changed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateDiff {
    pub fixture_id: String,
    pub category: Category,
    pub scenario: String,
    pub notes: String,
    pub differences: Vec<Difference>,
    pub v1: ExecutionResult,
    pub v2: ExecutionResult,
}

impl StateDiff {
    /// Severity across every difference, compute included.
    pub fn max_severity(&self) -> Option<Severity> {
        self.differences.iter().map(Difference::severity).max()
    }

    /// Differences that are observable as state, balances, outcome or CPI shape.
    pub fn outcome_differences(&self) -> Vec<&Difference> {
        self.differences
            .iter()
            .filter(|d| !d.is_compute_only())
            .collect()
    }

    /// Severity of the behavioural change alone.
    pub fn outcome_severity(&self) -> Option<Severity> {
        self.outcome_differences()
            .into_iter()
            .map(Difference::severity)
            .max()
    }

    pub fn compute_delta(&self) -> Option<(u64, u64, i32)> {
        self.differences.iter().find_map(|d| match d {
            Difference::ComputeChanged {
                v1, v2, pct_bps, ..
            } => Some((*v1, *v2, *pct_bps)),
            _ => None,
        })
    }

    pub fn classification(&self) -> Classification {
        if self.differences.is_empty() {
            return Classification::Identical;
        }
        if self.outcome_differences().is_empty() {
            return Classification::ComputeOnly;
        }
        Classification::Changed
    }

    pub fn is_critical(&self) -> bool {
        self.outcome_severity() == Some(Severity::Critical)
    }

    /// Behavioural differences that carry weight, worst first.
    pub fn material_differences(&self) -> Vec<&Difference> {
        let mut out: Vec<&Difference> = self
            .outcome_differences()
            .into_iter()
            .filter(|d| d.severity() > Severity::Info)
            .collect();
        out.sort_by_key(|d| std::cmp::Reverse(d.severity()));
        out
    }
}

/// The shape of one invocation, in full.
///
/// Program and depth alone are not the shape. A candidate that keeps the same
/// programs at the same depths while changing which instruction it calls, how
/// many accounts it passes, or how much data it sends has changed its
/// invocation graph, and comparing only `program@depth` reported that as
/// identical. The record's own fidelity gate already compares the richer frame;
/// the V1/V2 diff was the one place still looking at less.
fn invocation_key(call: &crate::executor::CpiCall) -> String {
    format!(
        "{}@{}#{}:{}/{}b:{}",
        call.program,
        call.stack_height,
        call.outer_index,
        call.account_count,
        call.data_len,
        call.discriminant
            .map(|d| d.to_string())
            .unwrap_or_else(|| "-".to_string()),
    )
}

fn first_difference_offset(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}

/// Whether this comparison may decode account bytes into named fields, and
/// with whose layout.
///
/// Generic diffing owns bytes, balances, outcomes and invocation shape. It does
/// **not** own economics, and it must never guess a layout. Dispatching on a
/// leading discriminator byte alone is a guess: a real SPL Stake Pool state
/// account begins with `1`, which is also this repository's synthetic
/// `ACCOUNT_TAG_MARKET`, so it decoded as a lending `Market`, was compared over
/// the first 86 of its 611 bytes, and reported as identical while
/// `total_lamports` at offset 258 changed underneath.
///
/// So the decoder is supplied by the caller rather than inferred. The synthetic
/// lending corpus passes its own; a replay record passes [`FieldDecoder::None`],
/// because its protocol adapter owns that interpretation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldDecoder {
    /// Bytes are opaque. A change is reported as changed bytes, in full.
    None,
    /// The fixture lending layout, for the synthetic corpus that defines it.
    FixtureLending,
}

/// Compare two executions of the same fixture, decoding the synthetic lending
/// layout. For anything that is not the fixture protocol, use
/// [`compare_with_decoder`] with [`FieldDecoder::None`].
pub fn compare(fixture: &Fixture, v1: ExecutionResult, v2: ExecutionResult) -> StateDiff {
    compare_with_decoder(fixture, v1, v2, FieldDecoder::FixtureLending)
}

/// Compare two executions of the same fixture under an explicit decoder.
pub fn compare_with_decoder(
    fixture: &Fixture,
    v1: ExecutionResult,
    v2: ExecutionResult,
    decoder: FieldDecoder,
) -> StateDiff {
    let mut differences = Vec::new();

    if v1.success != v2.success {
        differences.push(Difference::SuccessChanged {
            v1_success: v1.success,
            v2_success: v2.success,
            v1_error: v1.error.clone(),
            v2_error: v2.error.clone(),
        });
    }

    let labels: BTreeSet<&String> = v1.accounts.keys().chain(v2.accounts.keys()).collect();
    for label in labels {
        let (before, after) = match (v1.accounts.get(label), v2.accounts.get(label)) {
            (Some(a), Some(b)) => (a, b),
            // An account existing under one version only is a structural change
            // worth surfacing as a raw difference.
            (a, b) => {
                differences.push(Difference::RawDataChanged {
                    account: label.clone(),
                    offset: 0,
                    v1: a
                        .map(|x| format!("{} lamports", x.lamports))
                        .unwrap_or("<absent>".into()),
                    v2: b
                        .map(|x| format!("{} lamports", x.lamports))
                        .unwrap_or("<absent>".into()),
                });
                continue;
            }
        };

        if before.lamports != after.lamports {
            differences.push(Difference::BalanceChanged {
                account: label.clone(),
                v1: before.lamports,
                v2: after.lamports,
                delta: (after.lamports as i128 - before.lamports as i128) as i64,
            });
        }

        for (field, a, b) in [
            ("owner", before.owner.clone(), after.owner.clone()),
            (
                "executable",
                before.executable.to_string(),
                after.executable.to_string(),
            ),
            (
                "rent_epoch",
                before.rent_epoch.to_string(),
                after.rent_epoch.to_string(),
            ),
        ] {
            if a != b {
                differences.push(Difference::FieldChanged {
                    account: label.clone(),
                    field: field.into(),
                    v1: a,
                    v2: b,
                    delta: None,
                    consequence: None,
                });
            }
        }

        if before.data == after.data {
            continue;
        }

        let (decoded_before, decoded_after) = match decoder {
            FieldDecoder::FixtureLending => (
                interpret::decode(&before.data),
                interpret::decode(&after.data),
            ),
            FieldDecoder::None => (Decoded::Opaque, Decoded::Opaque),
        };

        let comparable = !matches!(decoded_before, Decoded::Opaque)
            && std::mem::discriminant(&decoded_before) == std::mem::discriminant(&decoded_after);

        if !comparable {
            differences.push(Difference::RawDataChanged {
                account: label.clone(),
                offset: first_difference_offset(&before.data, &after.data),
                v1: crate::hexfmt::encode(&before.data),
                v2: crate::hexfmt::encode(&after.data),
            });
            continue;
        }

        // A decoded layout describes a prefix. Anything past it is still real
        // state, and reporting only the fields would let a change outside the
        // struct vanish from a comparison that called itself complete.
        let decoded_len = interpret::decoded_len(&decoded_before);
        if before.data.get(decoded_len..) != after.data.get(decoded_len..) {
            differences.push(Difference::RawDataChanged {
                account: label.clone(),
                offset: first_difference_offset(&before.data, &after.data),
                v1: crate::hexfmt::encode(&before.data),
                v2: crate::hexfmt::encode(&after.data),
            });
        }

        let fields_before = interpret::fields(&decoded_before);
        let fields_after = interpret::fields(&decoded_after);
        for ((name, value_before), (_, value_after)) in
            fields_before.iter().zip(fields_after.iter())
        {
            if value_before == value_after {
                continue;
            }
            let delta = match (value_before.numeric(), value_after.numeric()) {
                (Some(a), Some(b)) => i64::try_from(b - a).ok(),
                _ => None,
            };
            differences.push(Difference::FieldChanged {
                account: label.clone(),
                field: (*name).to_string(),
                v1: value_before.render(),
                v2: value_after.render(),
                delta,
                consequence: interpret::explain(name, value_before, value_after),
            });

            // The headline economic event: a threshold crossing.
            if let (FieldValue::Health(a), FieldValue::Health(b)) = (value_before, value_after) {
                let was = interpret::is_liquidatable(*a);
                let now = interpret::is_liquidatable(*b);
                if was != now {
                    differences.push(Difference::LiquidationStatusChanged {
                        account: label.clone(),
                        v1: was,
                        v2: now,
                        v1_health: interpret::format_health(*a),
                        v2_health: interpret::format_health(*b),
                    });
                }
            }
        }
    }

    let cpi_v1: Vec<String> = v1.cpi_calls.iter().map(invocation_key).collect();
    let cpi_v2: Vec<String> = v2.cpi_calls.iter().map(invocation_key).collect();
    if cpi_v1 != cpi_v2 {
        differences.push(Difference::CpiChanged {
            v1: cpi_v1,
            v2: cpi_v2,
        });
    }

    if let (Some(a), Some(b)) = (v1.compute_units, v2.compute_units) {
        if a != b {
            // Integer basis points: (b - a) / a, scaled by 10_000.
            let pct_bps = if a == 0 {
                10_000
            } else {
                (((b as i128 - a as i128) * 10_000) / a as i128) as i32
            };
            differences.push(Difference::ComputeChanged {
                v1: a,
                v2: b,
                delta: b as i64 - a as i64,
                pct_bps,
            });
        }
    }

    StateDiff {
        fixture_id: fixture.id.clone(),
        category: fixture.category,
        scenario: fixture.scenario.clone(),
        notes: fixture.notes.clone(),
        differences,
        v1,
        v2,
    }
}

#[cfg(test)]
mod tests {

    /// A real SPL Stake Pool state account begins with `1`, which is also this
    /// repository's synthetic `ACCOUNT_TAG_MARKET`, and is 611 bytes against a
    /// `MARKET_LEN` of 86. Decoding it as a lending `Market` compared the first
    /// 86 bytes and called the account identical while `total_lamports` at
    /// offset 258 changed underneath.
    #[test]
    fn a_foreign_account_is_not_decoded_as_a_lending_layout() {
        use fixture_lending_interface::{ACCOUNT_TAG_MARKET, MARKET_LEN};

        let mut before = vec![0_u8; 611];
        before[0] = ACCOUNT_TAG_MARKET;
        let mut after = before.clone();
        // Past the lending prefix, where a stake pool keeps its own fields.
        after[258] ^= 0x01;
        assert!(after.len() > MARKET_LEN && 258 > MARKET_LEN);

        let differences = data_differences(&before, &after, FieldDecoder::None);
        assert!(
            differences
                .iter()
                .any(|d| matches!(d, Difference::RawDataChanged { .. })),
            "changed bytes must be reported, got {differences:?}"
        );
    }

    /// Even where a decoder legitimately applies, it describes a prefix. Bytes
    /// past it are still state, and must not vanish because the struct ended.
    #[test]
    fn changes_past_a_decoded_prefix_are_still_reported() {
        use fixture_lending_interface::{ACCOUNT_TAG_MARKET, MARKET_LEN};

        let mut before = vec![0_u8; MARKET_LEN + 64];
        before[0] = ACCOUNT_TAG_MARKET;
        let mut after = before.clone();
        after[MARKET_LEN + 10] ^= 0xFF;

        let differences = data_differences(&before, &after, FieldDecoder::FixtureLending);
        assert!(
            differences
                .iter()
                .any(|d| matches!(d, Difference::RawDataChanged { .. })),
            "a suffix-only change must survive decoding, got {differences:?}"
        );
    }

    /// Drive one account's data through `compare` and return what it reported.
    fn data_differences(before: &[u8], after: &[u8], decoder: FieldDecoder) -> Vec<Difference> {
        let snapshot = |data: &[u8]| crate::types::AccountSnapshot {
            lamports: 1,
            owner: "11111111111111111111111111111111".to_string(),
            data: data.to_vec(),
            executable: false,
            rent_epoch: 0,
        };
        let mut fixture =
            crate::corpus::generate(&solana_address::Address::new_from_array([3; 32]))
                .into_iter()
                .next()
                .expect("a fixture");
        fixture.accounts = vec![crate::types::NamedAccount {
            label: "subject".into(),
            address: "SubjectAccount".into(),
            account: snapshot(before),
        }];

        let result = |data: &[u8]| {
            let mut accounts = std::collections::BTreeMap::new();
            accounts.insert("subject".to_string(), snapshot(data));
            crate::executor::ExecutionResult {
                version: "v".into(),
                success: true,
                error: None,
                compute_units: Some(1),
                fee: 0,
                logs: Vec::new(),
                cpi_calls: Vec::new(),
                accounts,
            }
        };
        compare_with_decoder(&fixture, result(before), result(after), decoder).differences
    }
    use super::*;

    #[test]
    fn severity_ordering_is_meaningful() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }

    #[test]
    fn small_compute_drift_is_noise() {
        let difference = Difference::ComputeChanged {
            v1: 10_000,
            v2: 10_100,
            delta: 100,
            pct_bps: 100,
        };
        assert_eq!(difference.severity(), Severity::Info);
    }

    #[test]
    fn large_compute_drift_escalates() {
        let difference = Difference::ComputeChanged {
            v1: 140_000,
            v2: 240_000,
            delta: 100_000,
            pct_bps: 7_140,
        };
        assert_eq!(difference.severity(), Severity::High);
    }

    #[test]
    fn compute_only_diffs_are_not_behavioural_changes() {
        let execution = |cu: u64| crate::executor::ExecutionResult {
            version: "x".into(),
            success: true,
            error: None,
            compute_units: Some(cu),
            fee: 5000,
            logs: vec![],
            cpi_calls: vec![],
            accounts: Default::default(),
        };
        let diff = StateDiff {
            fixture_id: "f".into(),
            category: Category::Healthy,
            scenario: "s".into(),
            notes: "n".into(),
            differences: vec![Difference::ComputeChanged {
                v1: 4205,
                v2: 3968,
                delta: -237,
                pct_bps: -564,
            }],
            v1: execution(4205),
            v2: execution(3968),
        };
        assert_eq!(diff.classification(), Classification::ComputeOnly);
        assert!(!diff.is_critical());
        assert_eq!(diff.outcome_severity(), None);
    }
}

#[cfg(test)]
mod invocation_shape {
    use super::*;
    use crate::executor::CpiCall;

    fn call(discriminant: u8, account_count: u8, data_len: u32) -> CpiCall {
        CpiCall {
            program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".into(),
            stack_height: 2,
            outer_index: 3,
            account_count,
            data_len,
            discriminant: Some(discriminant),
        }
    }

    /// Same program, same depth, different instruction. That is a changed
    /// invocation graph and must not compare as identical.
    #[test]
    fn a_changed_discriminant_is_a_changed_invocation() {
        assert_ne!(
            invocation_key(&call(2, 3, 9)),
            invocation_key(&call(3, 3, 9))
        );
    }

    #[test]
    fn account_count_and_data_length_are_part_of_the_shape() {
        assert_ne!(
            invocation_key(&call(2, 3, 9)),
            invocation_key(&call(2, 4, 9))
        );
        assert_ne!(
            invocation_key(&call(2, 3, 9)),
            invocation_key(&call(2, 3, 12))
        );
    }

    /// Which top-level instruction a call descends from is part of the shape
    /// too: moving a mint from one instruction to another is a real change.
    #[test]
    fn the_owning_instruction_is_part_of_the_shape() {
        let mut moved = call(2, 3, 9);
        moved.outer_index = 4;
        assert_ne!(invocation_key(&call(2, 3, 9)), invocation_key(&moved));
    }

    #[test]
    fn an_identical_call_compares_equal() {
        assert_eq!(
            invocation_key(&call(2, 3, 9)),
            invocation_key(&call(2, 3, 9))
        );
    }
}

#[cfg(test)]
mod large_value_serialization {
    use super::*;

    /// `Difference` is internally tagged, which restricts what its variants can
    /// carry. The string-serialized balances must still survive a round trip
    /// through that representation, or the report would serialize and refuse to
    /// deserialize.
    #[test]
    fn a_balance_past_two_to_the_fifty_third_round_trips() {
        let difference = Difference::BalanceChanged {
            account: "reserve".into(),
            v1: 15_000_000_000_000_000,
            v2: 15_000_000_000_000_001,
            delta: 1,
        };
        let json = serde_json::to_string(&difference).unwrap();
        assert!(json.contains(r#""15000000000000000""#), "{json}");
        assert_eq!(
            serde_json::from_str::<Difference>(&json).unwrap(),
            difference
        );
    }

    /// Reports written before this change must keep parsing.
    #[test]
    fn a_numeric_balance_is_still_read() {
        let json = r#"{"kind":"balance_changed","account":"a","v1":1,"v2":2,"delta":1}"#;
        let difference: Difference = serde_json::from_str(json).unwrap();
        assert!(matches!(
            difference,
            Difference::BalanceChanged { v1: 1, v2: 2, .. }
        ));
    }
}
