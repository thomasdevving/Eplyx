//! Universal execution evidence.
//!
//! What was *measured*, never what it *meant*. Nothing in this module or its
//! children may name a deposit, a withdrawal, a borrow, a health factor or a
//! liquidation. A token balance went from A to B; whether that was a user being
//! paid or a protocol taking a fee is a question for
//! [`crate::protocol::ProtocolAdapter`], which is the only layer entitled to
//! answer it.
//!
//! ## Why this layer exists
//!
//! Measured, at commit `029811d`: 46% of the second protocol adapter's
//! shared-method code was line-identical to the first's, and the most
//! duplicated method — `prove_boundaries`, 77% — was Solana machinery rather
//! than protocol knowledge. Adapter three would have paid for it again. The
//! primitives live here so that it does not.
//!
//! ## The invariant that constrains the API
//!
//! **No flow evidence is not evidence of no economic change.** A Drift
//! `settlePNL` moves value between a position and a pool with no transfer, no
//! mint, no burn and no lamport delta: every primitive in this module measures
//! exactly zero, and the economics are real. So [`FlowEvidence`] has no
//! `is_empty`, no `unchanged`, and no method whose name invites the reading
//! "nothing happened". The only accessor for an empty set is
//! [`FlowEvidence::no_flow_observed`], which says what it means and nothing
//! more. See `a_measured_absence_of_flow_is_not_an_absence_of_change`.
//!
//! ## What this layer refuses to do
//!
//! It performs no I/O. It reads no environment. It fetches no interface. Every
//! function here is a pure function of already-captured evidence, which is what
//! makes derivation replay-safe and a report reproducible.

pub mod account;
pub mod boundary;
pub mod cpi;
pub mod field;
pub mod labels;
pub mod native;
pub mod pairing;
pub mod token;

use crate::{executor::ExecutionResult, types::NamedAccount};
use serde::{Deserialize, Serialize};

// Evidence is *derived*, never read back from a file: it is recomputed from an
// execution every time. So these types serialize - for diagnostics and for a
// future report surface - and deliberately do not deserialize. A `Deserialize`
// on derived evidence would imply a wire format that could be handed to the
// engine in place of a measurement, which is exactly the substitution this
// whole design exists to prevent.

/// Where one piece of evidence came from.
///
/// Every evidence item carries this. Evidence divorced from its proof context
/// is an assertion, and the whole design of this tool is that an assertion is
/// not a measurement.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Provenance {
    /// The record or fixture this was measured from.
    pub record: String,
    /// The account it concerns, by its stable label.
    pub account_label: String,
    /// The account's address, where one is known. A fixture account has a
    /// label and an address; a synthesised one may have only the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Which decoder read it, and under which layout claim.
    pub decoder: DecoderIdentity,
    /// The instruction or invocation it is attributed to, where the evidence
    /// is attributable to one rather than to the transaction as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<InvocationOrigin>,
}

/// Which decoder produced a value, and what its layout claim rests on.
///
/// `version` is not decoration. A decoder whose understanding of a layout
/// changes produces different evidence from the same bytes, and evidence that
/// cannot say which decoder read it cannot be compared across builds.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct DecoderIdentity {
    pub name: &'static str,
    pub version: u32,
    pub provenance: crate::standard_programs::SchemaProvenance,
}

impl DecoderIdentity {
    pub const fn standard(name: &'static str, version: u32) -> Self {
        Self {
            name,
            version,
            provenance: crate::standard_programs::SchemaProvenance::StandardProgram,
        }
    }

    pub const fn manual(name: &'static str, version: u32) -> Self {
        Self {
            name,
            version,
            provenance: crate::standard_programs::SchemaProvenance::ManualInterface,
        }
    }
}

/// Which instruction or cross-program invocation evidence is attributed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InvocationOrigin {
    /// Index of the top-level instruction this sits under.
    pub instruction_index: u8,
    /// Invocation depth: 1 for a top-level instruction, 2 for its direct CPI.
    pub depth: u8,
}

/// Everything measured about one execution, before anything interprets it.
///
/// Assembled by [`derive`] from an execution result and the pre-state it ran
/// against. Deliberately has no verdict on it: there is no `is_clean`, no
/// `changed`, no severity. Those are judgements, and this type does not judge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniversalEvidence {
    pub execution: ExecutionFacts,
    pub accounts: Vec<account::AccountDelta>,
    pub native: Vec<native::LamportDelta>,
    pub flows: FlowEvidence,
    pub lifecycle: Vec<account::LifecycleEvent>,
    pub authority: Vec<token::AuthorityDelta>,
    pub cpi: cpi::CpiGraph,
    pub fields: Vec<field::TypedFieldDelta>,
}

/// Protocol-independent facts about whether and how a transaction ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionFacts {
    pub version: String,
    pub succeeded: bool,
    pub error: Option<String>,
    pub compute_units: Option<u64>,
    pub fee: u64,
    pub log_count: usize,
    pub invocation_count: usize,
}

impl ExecutionFacts {
    pub fn of(result: &ExecutionResult) -> Self {
        Self {
            version: result.version.clone(),
            succeeded: result.success,
            error: result.error.clone(),
            compute_units: result.compute_units,
            fee: result.fee,
            log_count: result.logs.len(),
            invocation_count: result.cpi_calls.len(),
        }
    }
}

/// Measured value movement: token balance deltas, mints, burns.
///
/// # This type cannot tell you that nothing happened
///
/// It has no `is_empty`. It has no `unchanged`. The only way to ask about an
/// empty set is [`FlowEvidence::no_flow_observed`], whose name is the whole
/// point: it reports that *this measurement found no movement*, which is a
/// statement about the measurement and not about the economics.
///
/// Drift's `settlePNL` is the case that fixes this design. Value moves between
/// a user's position and a per-market P&L pool; all collateral sits in one
/// global vault; settlement is accounting. Every primitive here measures zero
/// and a user's economic position has genuinely changed. An API that let a
/// caller write `if evidence.flows.is_empty() { /* nothing to report */ }`
/// would turn that into a clean bill of health, which is worse than reporting
/// nothing at all because it looks like coverage.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FlowEvidence {
    token_deltas: Vec<token::TokenBalanceDelta>,
    mint_deltas: Vec<token::MintSupplyDelta>,
    transfers: Vec<token::TokenTransferEvidence>,
    mints: Vec<token::MintEvidence>,
    burns: Vec<token::BurnEvidence>,
}

impl FlowEvidence {
    pub fn new(
        token_deltas: Vec<token::TokenBalanceDelta>,
        mint_deltas: Vec<token::MintSupplyDelta>,
    ) -> Self {
        Self {
            token_deltas,
            mint_deltas,
            transfers: Vec::new(),
            mints: Vec::new(),
            burns: Vec::new(),
        }
    }

    pub fn with_invocation_evidence(
        mut self,
        transfers: Vec<token::TokenTransferEvidence>,
        mints: Vec<token::MintEvidence>,
        burns: Vec<token::BurnEvidence>,
    ) -> Self {
        self.transfers = transfers;
        self.mints = mints;
        self.burns = burns;
        self
    }

    pub fn token_deltas(&self) -> &[token::TokenBalanceDelta] {
        &self.token_deltas
    }

    pub fn mint_deltas(&self) -> &[token::MintSupplyDelta] {
        &self.mint_deltas
    }

    pub fn transfers(&self) -> &[token::TokenTransferEvidence] {
        &self.transfers
    }

    pub fn mints(&self) -> &[token::MintEvidence] {
        &self.mints
    }

    pub fn burns(&self) -> &[token::BurnEvidence] {
        &self.burns
    }

    /// Whether this measurement found any value movement.
    ///
    /// **Read the name literally.** `true` means the flow primitives measured
    /// nothing. It does not mean nothing changed, it does not mean no economic
    /// change occurred, and it must never be used to decide that an observation
    /// needs no further evidence. A protocol whose economics are internal
    /// accounting returns `true` here on every observation while moving real
    /// money. Use it to describe coverage, never to conclude safety.
    pub fn no_flow_observed(&self) -> bool {
        self.token_deltas.is_empty()
            && self.mint_deltas.is_empty()
            && self.transfers.is_empty()
            && self.mints.is_empty()
            && self.burns.is_empty()
    }
}

/// Derive every protocol-independent primitive from one execution.
///
/// Pure: no RPC, no environment, no clock, no global state. `pre` is the state
/// the execution ran against; `result` is what came out. `record` names the
/// observation for provenance.
pub fn derive(record: &str, pre: &[NamedAccount], result: &ExecutionResult) -> UniversalEvidence {
    let accounts = account::pair(record, pre, result);
    let native = native::deltas(&accounts);
    let lifecycle = account::lifecycle(&accounts);
    let token_deltas = token::balance_deltas(&accounts);
    let mint_deltas = token::supply_deltas(&accounts);
    let authority = token::authority_deltas(&accounts);
    UniversalEvidence {
        execution: ExecutionFacts::of(result),
        native,
        flows: FlowEvidence::new(token_deltas, mint_deltas),
        lifecycle,
        authority,
        cpi: cpi::CpiGraph::build(&result.cpi_calls),
        fields: Vec::new(),
        accounts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AccountSnapshot;
    use std::collections::BTreeMap;

    pub(crate) fn snapshot(owner: &str, lamports: u64, data: Vec<u8>) -> AccountSnapshot {
        AccountSnapshot {
            lamports,
            owner: owner.into(),
            data,
            executable: false,
            rent_epoch: 0,
        }
    }

    pub(crate) fn named(label: &str, owner: &str, lamports: u64, data: Vec<u8>) -> NamedAccount {
        NamedAccount {
            label: label.into(),
            address: format!("address-of-{label}"),
            account: snapshot(owner, lamports, data),
        }
    }

    pub(crate) fn execution(accounts: Vec<(&str, AccountSnapshot)>) -> ExecutionResult {
        ExecutionResult {
            version: "v1".into(),
            success: true,
            error: None,
            compute_units: Some(1_000),
            fee: 5_000,
            logs: Vec::new(),
            cpi_calls: Vec::new(),
            accounts: accounts
                .into_iter()
                .map(|(label, snapshot)| (label.to_string(), snapshot))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    /// The hard invariant, stated as a test.
    ///
    /// A change wholly internal to a program-owned account produces no flow
    /// evidence whatsoever, and that must not be readable as "no economic
    /// change". This is the Drift `settlePNL` shape: a typed field moves, no
    /// token balance does, no lamport does, nothing is minted or burned.
    #[test]
    fn a_measured_absence_of_flow_is_not_an_absence_of_change() {
        let program = "dRiftyHA3jZfMwd1XerVmmvcMGCsBLzbAXnPabcdefgh";
        let mut before = vec![0_u8; 128];
        before[40..48].copy_from_slice(&1_000_u64.to_le_bytes());
        let mut after = before.clone();
        after[40..48].copy_from_slice(&1_500_u64.to_le_bytes());

        let pre = vec![named("position", program, 2_039_280, before)];
        let result = execution(vec![("position", snapshot(program, 2_039_280, after))]);
        let evidence = derive("settle-pnl", &pre, &result);

        // Every flow primitive measures nothing.
        assert!(evidence.flows.no_flow_observed());
        assert!(evidence.flows.token_deltas().is_empty());
        assert!(evidence.flows.mint_deltas().is_empty());
        assert!(evidence.native.is_empty(), "lamports did not move either");

        // And the account plainly changed. The evidence says so, through a
        // different primitive - which is exactly why the flow set's emptiness
        // may not stand in for a verdict.
        assert_eq!(evidence.accounts.len(), 1);
        assert!(
            evidence.accounts[0].data_changed(),
            "the position's bytes moved"
        );
        assert_eq!(evidence.accounts[0].first_data_difference(), Some(40));
    }

    /// `UniversalEvidence` exposes no verdict. If someone adds one, this test
    /// is the place the argument has to be had.
    #[test]
    fn universal_evidence_offers_no_overall_verdict() {
        let pre = vec![named(
            "a",
            "11111111111111111111111111111111",
            1,
            Vec::new(),
        )];
        let result = execution(vec![(
            "a",
            snapshot("11111111111111111111111111111111", 1, Vec::new()),
        )]);
        let evidence = derive("r", &pre, &result);
        // The only emptiness question the flow set answers is the narrow,
        // honestly-named one.
        assert!(evidence.flows.no_flow_observed());
        // Execution facts are facts, not judgements: "succeeded" is what the
        // runtime reported, and carries no claim about whether it was correct.
        assert!(evidence.execution.succeeded);
        assert_eq!(evidence.execution.compute_units, Some(1_000));
    }

    #[test]
    fn evidence_derivation_is_deterministic() {
        let pre = vec![
            named("b", "11111111111111111111111111111111", 5, vec![1, 2, 3]),
            named("a", "11111111111111111111111111111111", 7, vec![4]),
        ];
        let result = execution(vec![
            (
                "a",
                snapshot("11111111111111111111111111111111", 9, vec![4]),
            ),
            (
                "b",
                snapshot("11111111111111111111111111111111", 5, vec![1, 2, 4]),
            ),
        ]);
        let first = derive("r", &pre, &result);
        let second = derive("r", &pre, &result);
        assert_eq!(first, second);
        // Ordering is by label, not by input order, so two callers that
        // assembled their pre-state differently still compare equal.
        assert_eq!(
            first
                .accounts
                .iter()
                .map(|a| a.provenance.account_label.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }
}
