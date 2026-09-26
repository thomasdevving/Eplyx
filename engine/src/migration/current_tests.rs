//! Deterministic tests for population discovery, classification, selection and
//! the non-inheritance invariants.
//!
//! Everything here is built from synthetic RPC transcripts, so it never needs a
//! network and never needs a deployed program. The tests that require actual SBF
//! execution live in `engine/tests/conversion_stress.rs`.
use super::{
    current_classify as classify,
    current_select::{
        self as select, CandidatePlan, Eligibility, SelectionReason, FULL_AT_FINAL_POLICY,
    },
    population::{self, Capture},
    population_types::{
        AuthorityResolution, AuthorityResolutionCompleteness, EnumerationCompleteness, StressBudget,
    },
};
use crate::{
    evidence::authority::EntityType, ingest::observation::Observation,
    standard_programs::token as decode,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use solana_address::Address;
use solana_program_pack::Pack;
use spl_token_2022_interface::state::{Account, AccountState, Mint};

const MAINNET: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const LEGACY: &str = decode::LEGACY_PROGRAM;
const SYSTEM: &str = "11111111111111111111111111111111";

/// Deterministic address with the requested curve property.
///
/// Only the first two bytes are searched; bytes 2..32 carry the seed, so two
/// different seeds can never collide no matter how far the search runs. A
/// wallet-compatible authority must be on-curve, and mints and token accounts
/// are off-curve, so both are needed.
fn address(seed: u8, want_on_curve: bool) -> Address {
    for n in 0..=u16::MAX {
        let mut bytes = [seed; 32];
        bytes[0] = (n & 0xff) as u8;
        bytes[1] = (n >> 8) as u8;
        let a = Address::new_from_array(bytes);
        if a.is_on_curve() == want_on_curve {
            return a;
        }
    }
    unreachable!("both curve properties occur in this range")
}
fn on_curve(seed: u8) -> Address {
    address(seed, true)
}
fn off_curve(seed: u8) -> Address {
    address(seed, false)
}
fn raw(owner: &str, data: Vec<u8>) -> Value {
    json!({
        "lamports": 2_039_280u64,
        "owner": owner,
        "executable": false,
        "rentEpoch": 0u64,
        "space": data.len(),
        "data": [STANDARD.encode(&data), "base64"],
    })
}
fn mint_account(decimals: u8, supply: u64) -> Value {
    let mut data = vec![0u8; Mint::LEN];
    Mint {
        mint_authority: None.into(),
        supply,
        decimals,
        is_initialized: true,
        freeze_authority: None.into(),
    }
    .pack_into_slice(&mut data);
    raw(LEGACY, data)
}
struct TokenAccount {
    owner: Address,
    amount: u64,
    state: AccountState,
    delegate: Option<Address>,
    delegated: u64,
    close_authority: Option<Address>,
}
impl TokenAccount {
    fn new(owner: Address, amount: u64) -> Self {
        Self {
            owner,
            amount,
            state: AccountState::Initialized,
            delegate: None,
            delegated: 0,
            close_authority: None,
        }
    }
    fn bytes(&self, mint: Address) -> Vec<u8> {
        let mut data = vec![0u8; Account::LEN];
        Account {
            mint,
            owner: self.owner,
            amount: self.amount,
            delegate: self.delegate.into(),
            state: self.state,
            is_native: None.into(),
            delegated_amount: self.delegated,
            close_authority: self.close_authority.into(),
        }
        .pack_into_slice(&mut data);
        data
    }
}
fn wallet_authority() -> Value {
    json!({"lamports": 1_000_000u64, "owner": SYSTEM, "executable": false,
        "rentEpoch": 0u64, "space": 0u64, "data": ["", "base64"]})
}
fn program_authority() -> Value {
    json!({"lamports": 1_000_000u64, "owner": "BPFLoaderUpgradeab1e11111111111111111111111",
        "executable": false, "rentEpoch": 0u64, "space": 8u64,
        "data": [STANDARD.encode([7u8; 8]), "base64"]})
}
fn observation(method: &str, params: Value, result: Option<Value>) -> Observation {
    Observation {
        method: method.into(),
        params,
        started_at: "2026-09-22T00:00:01.000Z".into(),
        completed_at: "2026-09-22T00:00:02.000Z".into(),
        error: result.is_none().then(|| "provider unavailable".to_string()),
        result,
    }
}

struct World {
    mint: Address,
    budget: StressBudget,
    accounts: Vec<(Address, Vec<u8>)>,
    authorities: Vec<(Address, Value)>,
    scan_ok: bool,
    resolve_authorities: bool,
}
impl World {
    /// Four wallet-compatible positive accounts, one program-owned positive
    /// account and one zero-balance account, over a legacy SPL mint.
    fn standard() -> Self {
        let mint = off_curve(3);
        let mut accounts = vec![];
        let mut authorities = vec![];
        for (i, amount) in [(0u8, 1_000u64), (1, 25_000), (2, 400_000), (3, 9_000_000)] {
            let owner = on_curve(10 + i * 7);
            accounts.push((
                off_curve(40 + i),
                TokenAccount::new(owner, amount).bytes(mint),
            ));
            authorities.push((owner, wallet_authority()));
        }
        let program_owner = on_curve(200);
        accounts.push((
            off_curve(60),
            TokenAccount::new(program_owner, 77_000).bytes(mint),
        ));
        authorities.push((program_owner, program_authority()));
        let empty_owner = on_curve(220);
        accounts.push((off_curve(70), TokenAccount::new(empty_owner, 0).bytes(mint)));
        Self {
            mint,
            budget: StressBudget::default(),
            accounts,
            authorities,
            scan_ok: true,
            resolve_authorities: true,
        }
    }
    fn capture(&self) -> Capture {
        let mut rows: Vec<(Address, Vec<u8>)> = self.accounts.clone();
        rows.sort_by_key(|(a, _)| a.to_string());
        let mut observations = vec![observation(
            "getGenesisHash",
            json!([]),
            Some(json!(MAINNET)),
        )];
        let mint_cfg = json!({"encoding":"base64","commitment":"finalized"});
        observations.push(observation(
            "getAccountInfo",
            json!([self.mint.to_string(), mint_cfg]),
            Some(json!({"context":{"slot":1000u64},"value":mint_account(6, 10_000_000)})),
        ));
        let scan_cfg = json!({"encoding":"base64","commitment":"finalized","minContextSlot":1000u64,
            "withContext":true,"filters":[{"memcmp":{"offset":0,"bytes":self.mint.to_string()}}]});
        if !self.scan_ok {
            observations.push(observation(
                "getProgramAccounts",
                json!([LEGACY, scan_cfg]),
                None,
            ));
            return self.finish(observations);
        }
        let value: Vec<Value> = rows
            .iter()
            .map(|(a, d)| json!({"pubkey": a.to_string(), "account": raw(LEGACY, d.clone())}))
            .collect();
        observations.push(observation(
            "getProgramAccounts",
            json!([LEGACY, scan_cfg]),
            Some(json!({"context":{"slot":1001u64},"value":value})),
        ));
        if self.resolve_authorities {
            // The transcript must match the deterministic plan exactly: distinct
            // positive-balance authorities, ascending, chunked by the budget.
            let mut wanted: Vec<String> = vec![];
            for (address, data) in &rows {
                let _ = address;
                let state = decode::decode_token_account(
                    &raw(LEGACY, data.clone()),
                    LEGACY,
                    &self.mint.to_string(),
                    6,
                )
                .unwrap();
                if state.raw_balance != "0" && !wanted.contains(&state.owner) {
                    wanted.push(state.owner);
                }
            }
            wanted.sort();
            for batch in wanted.chunks(self.budget.authority_batch_size) {
                let values: Vec<Value> = batch
                    .iter()
                    .map(|a| {
                        self.authorities
                            .iter()
                            .find(|(k, _)| k.to_string() == *a)
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null)
                    })
                    .collect();
                observations.push(observation(
                    "getMultipleAccounts",
                    json!([batch, {"encoding":"base64","commitment":"finalized","minContextSlot":1001u64}]),
                    Some(json!({"context":{"slot":1002u64},"value":values})),
                ));
            }
        }
        self.finish(observations)
    }
    fn finish(&self, observations: Vec<Observation>) -> Capture {
        Capture {
            schema_version: 2,
            kind: population::KIND.into(),
            run_id: "run-1".into(),
            stress_id: "stress-1".into(),
            mint: self.mint.to_string(),
            budget: self.budget.clone(),
            rpc_origin: "configured-read-only-provider".into(),
            started_at: "2026-09-22T00:00:00.000Z".into(),
            completed_at: "2026-09-22T00:00:03.000Z".into(),
            decoder: population::DECODER_V2.into(),
            observations,
        }
    }
}

fn candidate_plan(source_mint: &str) -> CandidatePlan {
    CandidatePlan {
        change_spec_id: "ab".repeat(32),
        analysis_input_sha256: "cd".repeat(32),
        source_mint: source_mint.into(),
        source_account: None,
        amount_policy: FULL_AT_FINAL_POLICY.into(),
    }
}
const PROGRAM: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FROZEN_AT: &str = "2026-09-22T00:01:00.000Z";

fn evaluated(world: &World) -> population::PopulationObservation {
    let capture = world.capture();
    let bytes = serde_json::to_vec(&capture).unwrap();
    population::evaluate_bytes(&bytes, &world.budget).unwrap()
}
fn planned(world: &World) -> (population::PopulationObservation, select::StressTestPlan) {
    let observation = evaluated(world);
    let plan = select::build(
        &observation,
        &candidate_plan(&world.mint.to_string()),
        PROGRAM,
        FROZEN_AT,
    )
    .unwrap();
    (observation, plan)
}

// ---------------------------------------------------------------- population

#[test]
fn fresh_population_cannot_be_supplied_by_a_historical_snapshot() {
    // A Phase 2 snapshot is a different, incompatible shape. There is no path
    // that turns saved account inventory into a current stress population.
    let historical = json!({
        "schema_version": 1, "asset": {"name":"x","mint":off_curve(3).to_string(),
        "expected_token_program": null, "expected_genesis_hash": null, "verification": []},
        "captured_at": "2020-01-01T00:00:00Z", "slot": 1, "entities": [], "evidence": []
    });
    let bytes = serde_json::to_vec(&historical).unwrap();
    let error = population::evaluate_bytes(&bytes, &StressBudget::default()).unwrap_err();
    assert!(
        error.to_string().contains("missing field") || error.to_string().contains("unknown field"),
        "a historical snapshot must not deserialize into a stress population: {error}"
    );
}

#[test]
fn complete_enumeration_and_authority_resolution_are_independent_axes() {
    let world = World::standard();
    let complete = evaluated(&world);
    assert_eq!(
        complete.enumeration.completeness,
        EnumerationCompleteness::CompleteForQuery
    );
    assert_eq!(
        complete.authority_resolution.completeness,
        AuthorityResolutionCompleteness::Complete
    );

    // Skipping authority lookups must leave the token-account enumeration itself
    // complete. "We do not know how many accounts exist" is a different finding
    // from "we have every account but not every authority model".
    let mut partial = World::standard();
    partial.resolve_authorities = false;
    let observation = evaluated(&partial);
    assert_eq!(
        observation.enumeration.completeness,
        EnumerationCompleteness::CompleteForQuery,
        "authority budget must never downgrade enumeration completeness"
    );
    assert_eq!(
        observation.authority_resolution.completeness,
        AuthorityResolutionCompleteness::NotPerformed
    );
    assert_eq!(
        observation.enumeration.rows_decoded,
        complete.enumeration.rows_decoded
    );
    assert!(!observation.fully_resolved());
}

#[test]
fn a_failed_scan_is_unavailable_and_never_a_complete_empty_population() {
    let mut world = World::standard();
    world.scan_ok = false;
    let observation = evaluated(&world);
    assert_eq!(
        observation.enumeration.completeness,
        EnumerationCompleteness::Unavailable
    );
    assert_eq!(observation.summary.token_accounts_observed, 0);
    assert!(observation
        .enumeration
        .gaps
        .iter()
        .any(|g| g.contains("Population discovery incomplete")));
    assert!(observation
        .limitations
        .iter()
        .any(|l| l.contains("not the token's holder population")));
}

#[test]
fn reaching_the_decode_budget_is_partial_not_complete() {
    let mut world = World::standard();
    world.budget.max_decoded_accounts = 3;
    world.budget.max_authority_lookups = 3;
    world.resolve_authorities = false;
    let observation = evaluated(&world);
    assert_eq!(
        observation.enumeration.completeness,
        EnumerationCompleteness::Partial
    );
    assert_eq!(observation.enumeration.rows_returned, 6);
    assert_eq!(observation.enumeration.rows_decoded, 3);
}

#[test]
fn zero_balance_is_observed_but_is_never_exposure() {
    let observation = evaluated(&World::standard());
    assert_eq!(observation.summary.token_accounts_observed, 6);
    assert_eq!(observation.summary.positive_balance_accounts_observed, 5);
    assert_eq!(observation.summary.zero_balance_accounts_observed, 1);
    // 1000 + 25000 + 400000 + 9000000 + 77000, with the zero account excluded.
    assert_eq!(observation.summary.observed_public_balance_raw, "9503000");
    assert_eq!(observation.positive_entities().count(), 5);
}

#[test]
fn an_unknown_balance_is_not_treated_as_zero() {
    let dimensions = shape_with(|d| {
        d.account_extension_types = vec!["ConfidentialTransferAccount".into()];
    });
    let (eligibility, reason) = classify::eligibility(&dimensions);
    assert_eq!(eligibility, Eligibility::Unsupported);
    assert!(reason.contains("unknown, not zero"));
}

#[test]
fn rows_for_another_mint_or_program_are_rejected_and_kept_separate() {
    let mut world = World::standard();
    let other_mint = off_curve(150);
    world.accounts.push((
        off_curve(80),
        TokenAccount::new(on_curve(30), 5_000).bytes(other_mint),
    ));
    world.resolve_authorities = false;
    let observation = evaluated(&world);
    assert_eq!(observation.summary.token_accounts_observed, 6);
    assert_eq!(observation.undecoded.len(), 1);
    assert_eq!(observation.summary.undecodable_rows_observed, 1);
    assert_eq!(
        observation.enumeration.completeness,
        EnumerationCompleteness::Partial
    );
    // An undecodable row is retained with its evidence, never counted as zero.
    assert!(observation.undecoded[0].raw_data_sha256.is_some());
    assert_eq!(observation.summary.observed_public_balance_raw, "9503000");
}

#[test]
fn several_accounts_of_one_authority_remain_separate_entities() {
    let mut world = World::standard();
    let shared = on_curve(10);
    world.accounts.push((
        off_curve(85),
        TokenAccount::new(shared, 1_234).bytes(world.mint),
    ));
    let observation = evaluated(&world);
    assert_eq!(observation.summary.token_accounts_observed, 7);
    assert_eq!(observation.summary.positive_balance_accounts_observed, 6);
    let ids: Vec<_> = observation
        .positive_entities()
        .filter(|e| e.authority == shared.to_string())
        .map(|e| e.entity_id.clone())
        .collect();
    assert_eq!(ids.len(), 2, "one authority, two distinct entities");
    assert_ne!(ids[0], ids[1]);
}

// ------------------------------------------------------------- classification

fn shape_with(f: impl FnOnce(&mut classify::ShapeDimensions)) -> classify::ShapeDimensions {
    let mut d = classify::ShapeDimensions {
        authority_model: "WalletCompatible".into(),
        authority_resolved: true,
        authority_on_curve: true,
        authority_account_exists: true,
        authority_runtime_owner: Some(SYSTEM.into()),
        authority_executable: Some(false),
        account_initialized: true,
        account_frozen: false,
        delegate_present: false,
        active_delegation: false,
        close_authority_present: false,
        account_extension_types: vec![],
        mint_token_program: LEGACY.into(),
        mint_paused: false,
        mint_transfer_hook_active: false,
        mint_transfer_fee_configured: false,
        mint_default_account_state: None,
        mint_permanent_delegate: false,
        mint_confidential_transfer: false,
        mint_confidential_mint_burn: false,
        mint_non_transferable: false,
        mint_undecodable_extension: false,
        balance_positive: true,
    };
    f(&mut d);
    d
}

#[test]
fn the_shape_key_is_deterministic_and_carries_no_identity_or_amount() {
    let a = shape_with(|_| {});
    let b = shape_with(|_| {});
    assert_eq!(
        classify::shape_key(&a).unwrap(),
        classify::shape_key(&b).unwrap()
    );
    // Every execution-relevant change must produce a different shape.
    for mutate in [
        |d: &mut classify::ShapeDimensions| d.account_frozen = true,
        |d: &mut classify::ShapeDimensions| d.active_delegation = true,
        |d: &mut classify::ShapeDimensions| d.delegate_present = true,
        |d: &mut classify::ShapeDimensions| d.close_authority_present = true,
        |d: &mut classify::ShapeDimensions| d.mint_transfer_hook_active = true,
        |d: &mut classify::ShapeDimensions| d.authority_model = "ProgramOwnedAuthority".into(),
    ] {
        let changed = shape_with(mutate);
        assert_ne!(
            classify::shape_key(&a).unwrap(),
            classify::shape_key(&changed).unwrap()
        );
    }
}

#[test]
fn balances_and_buckets_never_enter_the_shape_key() {
    let observation = evaluated(&World::standard());
    let mint = observation.mint_config.as_ref().unwrap();
    let wallets: Vec<_> = observation
        .positive_entities()
        .filter(|e| e.authority_model == EntityType::WalletCompatible)
        .collect();
    assert!(wallets.len() >= 2);
    let first = classify::dimensions(wallets[0], mint).unwrap();
    let second = classify::dimensions(wallets[1], mint).unwrap();
    assert_ne!(wallets[0].state.raw_balance, wallets[1].state.raw_balance);
    assert_eq!(
        classify::shape_key(&first).unwrap(),
        classify::shape_key(&second).unwrap(),
        "accounts differing only in balance share one state shape"
    );
}

#[test]
fn only_a_resolved_wallet_authority_is_an_executable_candidate() {
    assert_eq!(
        classify::eligibility(&shape_with(|_| {})).0,
        Eligibility::ExecutableCandidate
    );
    for (model, expected) in [
        ("ProgramOwnedAuthority", Eligibility::Unsupported),
        ("TokenMultisig", Eligibility::Unsupported),
        ("Unknown", Eligibility::Unsupported),
    ] {
        let d = shape_with(|d| d.authority_model = model.into());
        assert_eq!(classify::eligibility(&d).0, expected, "{model}");
    }
    let unresolved = shape_with(|d| {
        d.authority_resolved = false;
        d.authority_model = "Unknown".into();
    });
    assert_eq!(
        classify::eligibility(&unresolved).0,
        Eligibility::CaptureRequired
    );
    assert_eq!(
        classify::eligibility(&shape_with(|d| d.balance_positive = false)).0,
        Eligibility::Invalid
    );
    assert_eq!(
        classify::eligibility(&shape_with(|d| d.account_frozen = true)).0,
        Eligibility::Unsupported
    );
}

#[test]
fn no_authority_model_but_a_resolved_wallet_receives_an_assumed_signer() {
    assert!(classify::assumed_local_signer(
        &EntityType::WalletCompatible,
        AuthorityResolution::Resolved
    ));
    // An unresolved authority is unknown, even if its recorded model defaulted.
    assert!(!classify::assumed_local_signer(
        &EntityType::WalletCompatible,
        AuthorityResolution::NotResolved
    ));
    for model in [
        EntityType::ProgramOwnedAuthority,
        EntityType::TokenMultisig,
        EntityType::Unknown,
    ] {
        assert!(
            !classify::assumed_local_signer(&model, AuthorityResolution::Resolved),
            "{model:?} must never be given a wallet signer"
        );
    }
}

// ------------------------------------------------------------------ selection

#[test]
fn selection_is_deterministic_under_a_reordered_population() {
    let world = World::standard();
    let (_, plan) = planned(&world);
    let mut shuffled = World::standard();
    shuffled.accounts.reverse();
    shuffled.authorities.reverse();
    let (_, other) = planned(&shuffled);
    assert_eq!(plan.classification_sha256, other.classification_sha256);
    assert_eq!(plan.sha256().unwrap(), other.sha256().unwrap());
    assert_eq!(
        plan.selected
            .iter()
            .map(|c| c.token_account.clone())
            .collect::<Vec<_>>(),
        other
            .selected
            .iter()
            .map(|c| c.token_account.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn selection_covers_shapes_first_then_uncovered_balance_buckets() {
    let (_, plan) = planned(&World::standard());
    assert_eq!(
        plan.selected[0].selection_reason,
        SelectionReason::NewStateShape,
        "the first phase must be state-shape coverage"
    );
    let reasons: Vec<_> = plan.selected.iter().map(|c| c.selection_reason).collect();
    assert!(reasons.contains(&SelectionReason::NewBalanceBucket));
    // Every selected case records why it exists.
    assert!(plan.selected.iter().all(|c| !c.selection_detail.is_empty()));
    // Only executable candidates may be selected.
    let executable: Vec<_> = plan
        .state_shapes
        .iter()
        .filter(|s| s.eligibility == Eligibility::ExecutableCandidate)
        .map(|s| s.state_shape_sha256.clone())
        .collect();
    assert!(plan
        .selected
        .iter()
        .all(|c| executable.contains(&c.state_shape_sha256)));
}

#[test]
fn a_program_owned_account_is_never_selected_and_is_reported_as_unsupported() {
    let (_, plan) = planned(&World::standard());
    let unsupported: Vec<_> = plan
        .state_shapes
        .iter()
        .filter(|s| s.eligibility == Eligibility::Unsupported)
        .collect();
    assert_eq!(unsupported.len(), 1);
    assert_eq!(unsupported[0].entities_in_shape, 1);
    assert_eq!(unsupported[0].represented_raw, "77000");
    assert_eq!(unsupported[0].entities_selected, 0);
    assert!(unsupported[0]
        .eligibility_reason
        .contains("non-System runtime program"));
    assert_eq!(*plan.eligibility_counts.get("Unsupported").unwrap(), 1);
}

#[test]
fn the_full_observed_balance_is_bound_and_never_silently_capped() {
    let (observation, plan) = planned(&World::standard());
    for case in &plan.selected {
        let entity = observation
            .positive_entities()
            .find(|e| e.token_account == case.token_account)
            .unwrap();
        assert_eq!(case.selected_amount_raw, entity.state.raw_balance);
        assert_eq!(case.observed_balance_raw, entity.state.raw_balance);
        assert!(!case.amount_capped);
        assert_eq!(
            case.case_plan.amount_policy, FULL_AT_FINAL_POLICY,
            "the frozen case pins FullAtFinalCapture before any execution"
        );
        assert_eq!(
            case.selected_amount_decimal,
            decode::decimal_amount(case.selected_amount_raw.parse::<u64>().unwrap(), 6)
        );
    }
}

#[test]
fn every_case_plan_keeps_the_candidate_terms_and_gets_its_own_digest() {
    let base = candidate_plan(&World::standard().mint.to_string());
    let (_, plan) = planned(&World::standard());
    let mut digests = std::collections::BTreeSet::new();
    for case in &plan.selected {
        assert_eq!(case.case_plan.change_spec_id, base.change_spec_id);
        assert_eq!(
            case.case_plan.analysis_input_sha256,
            base.analysis_input_sha256
        );
        assert_eq!(case.case_plan.source_mint, base.source_mint);
        assert_eq!(
            case.case_plan.source_account.as_deref(),
            Some(case.token_account.as_str())
        );
        assert_eq!(case.case_plan_sha256, case.case_plan.sha256().unwrap());
        assert!(digests.insert(case.case_plan_sha256.clone()));
    }
    assert_eq!(plan.candidate_plan_sha256, base.sha256().unwrap());
}

#[test]
fn the_plan_binds_its_population_candidate_plan_and_candidate_program() {
    let world = World::standard();
    let (observation, plan) = planned(&world);
    let base = candidate_plan(&world.mint.to_string());
    plan.validate(&observation, &base, PROGRAM).unwrap();

    // A different candidate program build invalidates the frozen plan.
    assert!(plan.validate(&observation, &base, &"b".repeat(64)).is_err());

    // A different candidate plan invalidates it.
    let mut other = base.clone();
    other.change_spec_id = "ef".repeat(32);
    assert!(plan.validate(&observation, &other, PROGRAM).is_err());

    // A different population capture invalidates it: entity ids embed the
    // capture digest, so a refreshed world can never reuse this plan.
    let mut refreshed = World::standard();
    refreshed.accounts[0].1 = TokenAccount::new(on_curve(10), 1_001).bytes(refreshed.mint);
    let new_observation = evaluated(&refreshed);
    assert_ne!(new_observation.capture_sha256, observation.capture_sha256);
    assert!(plan.validate(&new_observation, &base, PROGRAM).is_err());
}

#[test]
fn a_refreshed_population_produces_a_new_world_with_no_inherited_proof() {
    let (first, first_plan) = planned(&World::standard());
    let mut refreshed = World::standard();
    refreshed.accounts[0].1 = TokenAccount::new(on_curve(10), 2_000).bytes(refreshed.mint);
    let (second, second_plan) = planned(&refreshed);
    assert_ne!(first.capture_sha256, second.capture_sha256);
    assert_ne!(first_plan.sha256().unwrap(), second_plan.sha256().unwrap());
    // Entity identity is scoped to its capture, so no case id or entity id from
    // the old world can address anything in the new one.
    let old: std::collections::BTreeSet<_> =
        first_plan.selected.iter().map(|c| &c.entity_id).collect();
    assert!(second_plan
        .selected
        .iter()
        .all(|c| !old.contains(&c.entity_id)));
}

#[test]
fn a_tampered_plan_cannot_survive_revalidation() {
    let world = World::standard();
    let (observation, plan) = planned(&world);
    let base = candidate_plan(&world.mint.to_string());

    // Rewriting an expected classification after the fact.
    let mut edited = plan.clone();
    edited.state_shapes[0].eligibility = Eligibility::Invalid;
    assert!(edited.validate(&observation, &base, PROGRAM).is_err());

    // Dropping a selected case, for instance one that later failed.
    let mut dropped = plan.clone();
    dropped.selected.pop();
    assert!(dropped.validate(&observation, &base, PROGRAM).is_err());

    // Swapping a selected case for a different account.
    let mut swapped = plan.clone();
    swapped.selected[0].token_account = off_curve(70).to_string();
    assert!(swapped.validate(&observation, &base, PROGRAM).is_err());

    // Lowering a selected amount.
    let mut lowered = plan.clone();
    lowered.selected[0].selected_amount_raw = "1".into();
    assert!(lowered.validate(&observation, &base, PROGRAM).is_err());
}

#[test]
fn balance_buckets_are_derived_from_this_capture_and_labelled_as_ordering_only() {
    let (_, plan) = planned(&World::standard());
    assert_eq!(plan.buckets.population, 5);
    assert_eq!(plan.buckets.bucket_count, 4);
    assert!(plan.buckets.note.contains("not a statistical sample"));
    assert!(plan.buckets.method.contains("not economic classes"));
    let total: usize = plan.buckets.boundaries.iter().map(|b| b.entities).sum();
    assert_eq!(total, 5);
}

// -------------------------------------------------- non-inheritance invariants
