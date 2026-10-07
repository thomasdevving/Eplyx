//! Bounded program-upgrade rollout rehearsal: an actual loader-v3 Upgrade in a
//! seeded installed world, the qualified Stake Pool SetFee(SolDeposit), and the
//! retained DepositSol, in five independently restored scenarios.
//!
//! Every execution test runs real SBF bytecode: the retained historical V1 and
//! Token ELFs from deploy/bundle, and the tracked constructed rollout
//! counterexample (fixtures/rollout), reproducible with
//! scripts/build-stake-pool-rollout-candidate.sh. Nothing here hard-codes an
//! outcome into the analyzer; every asserted result is what the VM produced.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use eplyx_engine::{
    bundle::CiBundle,
    change::{Change, ChangeSpec},
    parameter_change::{stake_pool, Operation},
    replay::ReplayClock,
    rollout::{
        self, artifact,
        world::{self, Entry},
        AnchorStatus, ComparisonStatus, ScenarioName, StepKind, StepOutcome, Variant,
    },
    standard_programs::upgradeable_loader as loader,
};

const RECORD: &str = "mainnet-spl-stake-pool-151010f709e113e7";
const DEPOSITOR: &str = "9J63BougcyZ6dKhpr94vfQAdEZcNVb87GSuRPWmT26P8";
const WITHDRAW_AUTHORITY: &str = "ECVFhdhHYpVmHDUD7b6N4Y38HGL2GXHj4wnmqPmjZ4U4";
const MANAGER: &str = "8zVQTFGiwCZQSkNhedqMnWSddeGDrSahmM4JdZMceagx";

fn root() -> PathBuf {
    eplyx_engine::repo_root()
}

fn historical() -> stake_pool::Input {
    stake_pool::Input::from_bundle(
        &CiBundle::open(root().join("deploy/bundle")).unwrap(),
        RECORD,
    )
    .unwrap()
}

fn candidate() -> Vec<u8> {
    std::fs::read(root().join("fixtures/rollout/fixture_stake_pool_rollout_v2.so")).expect(
        "tracked rollout candidate missing; scripts/build-stake-pool-rollout-candidate.sh reproduces it",
    )
}

fn v1() -> Vec<u8> {
    std::fs::read(root().join("deploy/bundle/binaries/current.so")).unwrap()
}

fn parameter(numerator: u64, denominator: u64) -> ChangeSpec {
    let mut spec = ChangeSpec::parse(
        &std::fs::read(root().join("docs/examples/stake-pool-parameter-change.json")).unwrap(),
    )
    .unwrap();
    if let Change::ProtocolParameterChange(p) = &mut spec.change {
        if let Operation::SplStakePoolSolDepositFeeV1 { proposed_fee, .. } = &mut p.operation {
            proposed_fee.numerator = numerator;
            proposed_fee.denominator = denominator;
        }
    }
    spec.change_spec_id = None;
    spec
}

fn input_with(candidate: Vec<u8>, numerator: u64, denominator: u64) -> rollout::Input {
    let h = historical();
    let upgrade = ChangeSpec::program_upgrade(&h.record.program_id, &candidate);
    rollout::Input::new(&upgrade, &parameter(numerator, denominator), h, candidate).unwrap()
}

/// SetFee(1/100) with the stricter-fee rollout candidate.
fn counterexample() -> &'static rollout::Analysis {
    static A: OnceLock<rollout::Analysis> = OnceLock::new();
    A.get_or_init(|| rollout::analyse(&input_with(candidate(), 1, 100)).unwrap())
}

/// SetFee(1/1000), within the candidate's maximum: the compatible control.
fn compatible() -> &'static rollout::Analysis {
    static A: OnceLock<rollout::Analysis> = OnceLock::new();
    A.get_or_init(|| rollout::analyse(&input_with(candidate(), 1, 1000)).unwrap())
}

fn prepared() -> &'static rollout::Prepared {
    static P: OnceLock<rollout::Prepared> = OnceLock::new();
    P.get_or_init(|| rollout::Prepared::new(input_with(candidate(), 1, 100)).unwrap())
}

fn step(a: &rollout::Analysis, s: ScenarioName, i: usize) -> &rollout::StepRecord {
    &a.report.scenario(s).unwrap().steps[i]
}

fn outcomes(a: &rollout::Analysis, s: ScenarioName) -> Vec<StepOutcome> {
    a.report
        .scenario(s)
        .unwrap()
        .steps
        .iter()
        .map(|r| r.outcome)
        .collect()
}

fn state<'a>(a: &'a rollout::Analysis, id: &Option<String>) -> &'a world::State {
    &a.evidence.states[id.as_ref().unwrap()]
}

fn metric(r: &rollout::StepRecord, name: &str) -> String {
    r.derived["reconciliation"][name]
        .as_str()
        .unwrap()
        .to_string()
}

fn reseal(a: &mut rollout::Analysis) {
    a.report.evidence = a.evidence.index();
    a.report.report_sha256 = rollout::report_digest(&a.report).unwrap();
}

// ---------------------------------------------------------------------------
// Fidelity anchors
// ---------------------------------------------------------------------------

#[test]
fn s0_deposit_reproduces_the_retained_historical_outcome() {
    let a = counterexample();
    let control = step(a, ScenarioName::Control, 0);
    assert_eq!(control.outcome, StepOutcome::Verified);
    assert_eq!(control.derived["historical_fidelity"]["status"], "matched");
    assert_eq!(
        control.derived["historical_fidelity"]["failures"],
        serde_json::json!([])
    );
    assert_eq!(metric(control, "recipient_account_credit_raw"), "760985008");
    assert_eq!(metric(control, "action_transaction_fee_lamports"), "14000");
    assert_eq!(
        a.report.anchors.baseline_world_fidelity.status,
        AnchorStatus::Matched
    );
    // The control ran through the seeded installed-program world, not through
    // an executable added by the backend.
    let s0 = state(a, &control.before_state_id);
    let installed = control.installed_program_after.as_ref().unwrap();
    assert_eq!(installed.identity, "historical_v1");
    assert_eq!(installed.deploy_slot, 429_882_117);
    assert_eq!(
        installed.upgrade_authority.as_deref(),
        Some(rollout::upgrade_authority().as_str())
    );
    assert_eq!(s0.clock, historical().record.clock);
}

#[test]
fn installed_v2_matches_the_existing_candidate_overlay() {
    let a = counterexample();
    let anchor = &a.report.anchors.installed_overlay;
    assert_eq!(anchor.status, AnchorStatus::Matched, "{:?}", anchor.detail);
    assert_eq!(anchor.detail["mismatches"], serde_json::json!([]));
    for field in [
        "logs",
        "compute_units",
        "cpi_shape",
        "watched_post_state",
        "reconciliation",
    ] {
        assert!(anchor.detail["compared"]
            .as_array()
            .unwrap()
            .contains(&field.into()));
    }
    let overlay = a.report.anchors.overlay_execution.as_ref().unwrap();
    assert!(overlay
        .logs
        .iter()
        .any(|l| l.contains("Rollout fixture: DepositSol")));
    let installed = step(a, ScenarioName::UpgradeControl, 2);
    assert_eq!(installed.clock_before.as_ref().unwrap().slot, 447_850_494);
    assert_eq!(
        overlay.compute_units,
        Some(
            installed.derived["compute_units"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap()
        )
    );
}

#[test]
fn deliberate_overlay_mismatch_is_reported_not_hidden() {
    let mut a = counterexample().clone();
    let overlay = a.report.anchors.overlay_execution.as_mut().unwrap();
    overlay.compute_units = overlay.compute_units.map(|c| c + 1);
    overlay.logs.push("deliberately different".into());
    let reduced = rollout::reduce(&a).unwrap();
    let anchor = &reduced.report.anchors.installed_overlay;
    assert_eq!(anchor.status, AnchorStatus::Failed);
    assert_eq!(
        anchor.reason.as_deref(),
        Some("installed_overlay_behavior_mismatch")
    );
    assert_eq!(
        anchor.detail["mismatches"],
        serde_json::json!(["compute_units", "logs"])
    );
    assert_eq!(
        reduced.report.comparison.status,
        ComparisonStatus::RolloutNotEstablished
    );
    assert!(reduced.report.comparison.finding.is_none());
    // A retained artifact carrying that overlay does not verify.
    reseal(&mut a);
    assert!(rollout::verify(&a).is_err());
}

// ---------------------------------------------------------------------------
// The actual installed upgrade
// ---------------------------------------------------------------------------

#[test]
fn loader_upgrade_installs_the_candidate_bytes_and_metadata() {
    let a = counterexample();
    let p = prepared();
    let up = step(a, ScenarioName::UpgradeControl, 0);
    assert_eq!(up.outcome, StepOutcome::Verified, "{:?}", up.detail);
    let x = &a.evidence.executions[up.execution_id.as_ref().unwrap()];
    assert!(x.success && x.error.is_none());
    assert!(x.logs.iter().any(|l| l.contains("Upgraded program")));
    assert_eq!(x.message.account_keys[0], rollout::upgrade_payer());
    let before = state(a, &up.before_state_id);
    let after = state(a, &up.after_state_id);
    let pd_address =
        loader::programdata_address(&historical().record.program_id.parse().unwrap()).to_string();
    let pd_before = before
        .account(&a.evidence.accounts, &pd_address)
        .unwrap()
        .unwrap();
    let pd_after = after
        .account(&a.evidence.accounts, &pd_address)
        .unwrap()
        .unwrap();
    let old = loader::decode_programdata(&pd_before.data).unwrap();
    let new = loader::decode_programdata(&pd_after.data).unwrap();
    assert_eq!(old.deploy_slot, 429_882_117);
    assert_eq!(old.bytes, v1());
    assert_eq!(new.deploy_slot, 447_850_493);
    assert_eq!(new.upgrade_authority, old.upgrade_authority);
    assert_eq!(pd_after.data.len(), pd_before.data.len());
    let elf = candidate();
    assert_eq!(new.bytes[..elf.len()], elf[..]);
    assert!(new.bytes[elf.len()..].iter().all(|b| *b == 0));
    // The Program account is untouched; the Buffer is closed to known absence.
    let program = &historical().record.program_id;
    assert_eq!(before.accounts[program], after.accounts[program]);
    assert_eq!(
        after.accounts[&rollout::buffer_address()],
        Entry::KnownAbsent
    );
    let buffer = before
        .account(&a.evidence.accounts, &rollout::buffer_address())
        .unwrap()
        .unwrap();
    let payer_before = before
        .account(&a.evidence.accounts, &rollout::upgrade_payer())
        .unwrap()
        .unwrap();
    let payer_after = after
        .account(&a.evidence.accounts, &rollout::upgrade_payer())
        .unwrap()
        .unwrap();
    assert_eq!(
        payer_after.lamports,
        payer_before.lamports + buffer.lamports - x.transaction_fee_lamports
    );
    assert_eq!(
        world::changed(before, after),
        [
            pd_address.clone(),
            rollout::buffer_address(),
            rollout::upgrade_payer()
        ]
        .into()
    );
    let installed = up.installed_program_after.as_ref().unwrap();
    assert_eq!(installed.identity, "candidate");
    assert_eq!(installed.visible_from_slot, 447_850_494);
    assert_eq!(p.preflight.capacity.capacity_bytes, 1_080_464);
    assert_eq!(p.preflight.capacity.required_bytes, 134_320);
    assert!(p.preflight.upgrade_blocker.is_none());
    assert_eq!(
        p.preflight.candidate_profile.as_deref(),
        Some(rollout::CANDIDATE_PROFILE)
    );
}

#[test]
fn wrong_or_unsigned_upgrade_authority_is_rejected_by_the_loader() {
    let p = prepared();
    let other = bs58::encode([91u8; 32]).into_string();
    let (r, next, _) = p
        .probe(
            StepKind::Upgrade,
            &p.s0,
            &p.s0_accounts,
            Variant::Signer {
                address: other.clone(),
                signs: true,
            },
        )
        .unwrap();
    assert_eq!(r.outcome, StepOutcome::Rejected);
    assert_eq!(r.reason.as_deref(), Some("upgrade_rejected"));
    assert!(r.detail.as_ref().unwrap().contains("IncorrectAuthority"));
    assert_eq!(r.derived["rollback_verified"], true);
    assert!(next.is_none());

    // Buffer authority agrees with the signer; ProgramData's does not.
    let mut store = p.s0_accounts.clone();
    let mut buffer =
        p.s0.account(&store, &rollout::buffer_address())
            .unwrap()
            .unwrap()
            .clone();
    buffer.data = loader::encode::buffer(Some(other.parse().unwrap()), &candidate());
    let s =
        p.s0.with_account(&mut store, &rollout::buffer_address(), Some(buffer))
            .unwrap();
    let (r, _, e) = p
        .probe(
            StepKind::Upgrade,
            &s,
            &store,
            Variant::Signer {
                address: other,
                signs: true,
            },
        )
        .unwrap();
    assert_eq!(r.outcome, StepOutcome::Rejected);
    assert!(r.detail.as_ref().unwrap().contains("IncorrectAuthority"));
    let x = &e.executions[r.execution_id.as_ref().unwrap()];
    assert!(x
        .logs
        .iter()
        .any(|l| l.contains("Incorrect upgrade authority provided")));

    let (r, _, _) = p
        .probe(
            StepKind::Upgrade,
            &p.s0,
            &p.s0_accounts,
            Variant::Signer {
                address: rollout::upgrade_authority(),
                signs: false,
            },
        )
        .unwrap();
    assert_eq!(r.outcome, StepOutcome::Rejected);
    assert!(r
        .detail
        .as_ref()
        .unwrap()
        .contains("MissingRequiredSignature"));
    assert_eq!(r.derived["rollback_verified"], true);
}

#[test]
fn malformed_buffer_is_a_candidate_installation_failure() {
    let p = prepared();
    let mut store = p.s0_accounts.clone();
    let mut buffer =
        p.s0.account(&store, &rollout::buffer_address())
            .unwrap()
            .unwrap()
            .clone();
    buffer.data = loader::encode::buffer(
        Some(rollout::upgrade_authority().parse().unwrap()),
        &candidate()[..4096],
    );
    let s =
        p.s0.with_account(&mut store, &rollout::buffer_address(), Some(buffer))
            .unwrap();
    let (r, next, e) = p
        .probe(StepKind::Upgrade, &s, &store, Variant::Declared)
        .unwrap();
    assert_eq!(r.outcome, StepOutcome::Rejected);
    assert_eq!(r.reason.as_deref(), Some("candidate_installation_failed"));
    assert!(next.is_none());
    // Rolled back: the historical V1 remains installed.
    let after = &e.states[r.after_state_id.as_ref().unwrap()];
    assert_eq!(
        r.installed_program_after.as_ref().unwrap().identity,
        "historical_v1"
    );
    assert_eq!(world::changed(&s, after), [rollout::upgrade_payer()].into());
}

#[test]
fn candidate_hash_mismatch_is_refused_before_analysis() {
    let h = historical();
    let upgrade = ChangeSpec::program_upgrade(&h.record.program_id, &candidate());
    let mut other = candidate();
    other[100] ^= 1;
    let error = rollout::Input::new(&upgrade, &parameter(1, 100), h, other)
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("candidate_identity_mismatch"), "{error}");
}

#[test]
fn insufficient_programdata_capacity_is_a_typed_blocker() {
    let mut padded = candidate();
    padded.resize(1_080_465, 0);
    let a = rollout::analyse(&input_with(padded, 1, 100)).unwrap();
    let blocker = a.report.preflight.upgrade_blocker.as_ref().unwrap();
    assert_eq!(blocker.reason, "programdata_capacity_insufficient");
    let cap = &a.report.preflight.capacity;
    assert_eq!(cap.capacity_bytes, 1_080_464);
    assert_eq!(cap.required_bytes, 1_080_465);
    assert!(!cap.sufficient);
    assert_eq!(
        cap.programdata_address,
        "EmiU8AQkB2sswTxVB6aCmsAJftoowZGGDXuytm6X65R3"
    );
    for s in [
        ScenarioName::UpgradeControl,
        ScenarioName::OrderA,
        ScenarioName::OrderB,
    ] {
        let scenario = a.report.scenario(s).unwrap();
        let up = scenario
            .steps
            .iter()
            .find(|r| r.step == StepKind::Upgrade)
            .unwrap();
        assert_eq!(up.outcome, StepOutcome::Unsupported);
        assert_eq!(
            up.reason.as_deref(),
            Some("programdata_capacity_insufficient")
        );
        assert!(up.execution_id.is_none(), "never executed, never resized");
        assert!(scenario.steps[up.index + 1..]
            .iter()
            .all(|r| r.outcome == StepOutcome::NotExecuted && r.execution_id.is_none()));
    }
    // Controls that need no upgrade still establish.
    assert!(a.report.scenario(ScenarioName::Control).unwrap().completed);
    assert!(
        a.report
            .scenario(ScenarioName::ConfigControl)
            .unwrap()
            .completed
    );
    // Order B's configuration ran before the blocked upgrade.
    assert_eq!(
        step(&a, ScenarioName::OrderB, 0).outcome,
        StepOutcome::Verified
    );
    assert_eq!(
        a.report.anchors.installed_overlay.status,
        AnchorStatus::NotEstablished
    );
    assert!(a.report.anchors.overlay_execution.is_none());
    assert_eq!(
        a.report.comparison.status,
        ComparisonStatus::RolloutNotEstablished
    );
    rollout::verify(&a).unwrap();
}

#[test]
fn unqualified_candidate_is_never_installed() {
    let mut other = candidate();
    other.push(0);
    let a = rollout::analyse(&input_with(other, 1, 100)).unwrap();
    assert_eq!(
        a.report.preflight.upgrade_blocker.as_ref().unwrap().reason,
        "candidate_not_qualified"
    );
    assert!(a.report.preflight.capacity.sufficient);
    let up = step(&a, ScenarioName::UpgradeControl, 0);
    assert_eq!(up.outcome, StepOutcome::Unsupported);
    assert!(up.execution_id.is_none());
    assert_eq!(
        a.report.comparison.status,
        ComparisonStatus::RolloutNotEstablished
    );
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

#[test]
fn litesvm_exposes_an_upgrade_in_the_same_slot_which_the_model_refuses() {
    use solana_transaction::Transaction;
    let p = prepared();
    // Backend fact: one LiteSVM, Upgrade then DepositSol at the same slot. The
    // backend runs the new code immediately; mainnet would not.
    let mut svm = world::restore(&p.s0, &p.s0_accounts, p.dependencies()).unwrap();
    svm.send_transaction(Transaction::new_unsigned(p.upgrade_message().clone()))
        .unwrap();
    let meta = svm
        .send_transaction(Transaction::new_unsigned(p.deposit_message().clone()))
        .unwrap();
    assert!(meta
        .logs
        .iter()
        .any(|l| l.contains("Rollout fixture: DepositSol")));

    // The rollout model does not accept that as mainnet-equivalent.
    for next in [StepKind::DepositSol, StepKind::SetFee] {
        let (s, e) = p.run_sequence(&[StepKind::Upgrade, next]).unwrap();
        assert_eq!(s.steps[0].outcome, StepOutcome::Verified);
        assert_eq!(s.steps[1].outcome, StepOutcome::Unsupported);
        assert_eq!(
            s.steps[1].reason.as_deref(),
            Some("visibility_boundary_not_crossed")
        );
        assert!(s.steps[1].execution_id.is_none());
        assert_eq!(e.executions.len(), 1);
    }
    let (s, _) = p
        .run_sequence(&[
            StepKind::Upgrade,
            StepKind::AdvanceToVisibleSlot,
            StepKind::DepositSol,
        ])
        .unwrap();
    assert!(s.completed);
}

#[test]
fn visibility_transition_advances_only_the_slot_and_never_crosses_an_epoch() {
    let clock = ReplayClock {
        slot: 447_850_493,
        epoch_start_timestamp: 0,
        epoch: 0,
        leader_schedule_epoch: 0,
        unix_timestamp: 1_789_665_625,
    };
    let next = rollout::visibility_transition(&clock, clock.slot).unwrap();
    assert_eq!(
        next.slot,
        clock.slot + rollout::DELAY_VISIBILITY_SLOT_OFFSET
    );
    assert_eq!(rollout::DELAY_VISIBILITY_SLOT_OFFSET, 1);
    let mut same = next.clone();
    same.slot = clock.slot;
    assert_eq!(same, clock, "only the slot changes");

    let last = rollout::MAINNET_SLOTS_PER_EPOCH * 1037 - 1;
    let edge = ReplayClock {
        slot: last,
        ..clock.clone()
    };
    let refused = rollout::visibility_transition(&edge, last).unwrap_err();
    assert_eq!(refused.reason, "unsupported_rollout_clock_transition");
    assert!(refused.detail.contains("epoch boundary"));
    // Nothing pending: refused, never a silent no-op.
    assert!(rollout::visibility_transition(&clock, 429_882_117).is_err());

    let a = counterexample();
    let advance = step(a, ScenarioName::UpgradeControl, 1);
    assert_eq!(advance.outcome, StepOutcome::Verified);
    assert!(
        advance.execution_id.is_none(),
        "a transition, not a transaction"
    );
    assert_eq!(
        advance.derived["fields_changed"],
        serde_json::json!(["slot"])
    );
    let before = state(a, &advance.before_state_id);
    let after = state(a, &advance.after_state_id);
    assert_eq!(before.accounts, after.accounts);
    assert_eq!(after.clock.slot, before.clock.slot + 1);
    assert_eq!(after.clock.epoch, before.clock.epoch);
    assert_eq!(after.clock.unix_timestamp, before.clock.unix_timestamp);
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[test]
fn configuration_reuses_the_qualified_setfee_contract() {
    let a = counterexample();
    let k = step(a, ScenarioName::ConfigControl, 0);
    assert_eq!(k.outcome, StepOutcome::Verified);
    assert_eq!(k.derived["preservation_verified"], true);
    assert_eq!(k.derived["manager_assumption"], "assumed_simulation_only");
    assert_eq!(k.derived["config_transaction_fee_lamports"], "10000");
    assert_eq!(
        k.derived["proposed_fee"],
        serde_json::json!({"numerator": "1", "denominator": "100"})
    );
    let contract = &a.contract["configuration"];
    assert_eq!(
        contract["manager_assumption"]["origin"],
        "assumed_simulation_only"
    );
    assert_eq!(contract["manager_assumption"]["address"], MANAGER);
    assert_eq!(
        contract["manager_assumption"]["key_possession_established"],
        false
    );
    // Same S0, same SetFee: the configuration step of order B produced the
    // identical byte-bearing world as the configuration control.
    assert_eq!(
        step(a, ScenarioName::OrderB, 0).after_state_id,
        k.after_state_id
    );
    // The configuration payer and manager never enter the user action.
    let deposit = step(a, ScenarioName::ConfigControl, 1);
    assert_eq!(metric(deposit, "action_transaction_fee_lamports"), "14000");
    assert_eq!(metric(deposit, "manager_fee_account_credit_raw"), "7609851");
}

#[test]
fn wrong_or_unsigned_manager_rejects_under_v1_and_installed_v2() {
    let a = counterexample();
    let p = prepared();
    let v2 = state(a, &step(a, ScenarioName::OrderA, 1).after_state_id);
    // Historical V1 reports StakePoolError::WrongManager (6) and
    // SignatureMissing (7); the constructed V2 its own program errors.
    for (world_state, wrong, unsigned) in [
        (&p.s0, "Custom(6)", "Custom(7)"),
        (v2, "InvalidArgument", "MissingRequiredSignature"),
    ] {
        let store = &a.evidence.accounts;
        for (address, signs, expected) in [
            (bs58::encode([92u8; 32]).into_string(), true, wrong),
            (MANAGER.to_string(), false, unsigned),
        ] {
            let (r, _, _) = p
                .probe(
                    StepKind::SetFee,
                    world_state,
                    store,
                    Variant::Signer { address, signs },
                )
                .unwrap();
            assert_eq!(r.outcome, StepOutcome::Rejected, "{:?}", r.detail);
            assert_eq!(r.reason.as_deref(), Some("config_execution_rejected"));
            assert!(
                r.detail.as_ref().unwrap().contains(expected),
                "{:?}",
                r.detail
            );
            assert_eq!(r.derived["rollback_verified"], true);
        }
    }
}

// ---------------------------------------------------------------------------
// Ordering and the counterexample
// ---------------------------------------------------------------------------

#[test]
fn counterexample_rollout_order_changes_whether_the_fee_can_be_installed() {
    let a = counterexample();
    use StepOutcome::*;
    assert_eq!(outcomes(a, ScenarioName::Control), [Verified]);
    assert_eq!(outcomes(a, ScenarioName::UpgradeControl), [Verified; 3]);
    assert_eq!(outcomes(a, ScenarioName::ConfigControl), [Verified; 2]);
    assert_eq!(
        outcomes(a, ScenarioName::OrderA),
        [Verified, Verified, Rejected, NotExecuted]
    );
    assert_eq!(outcomes(a, ScenarioName::OrderB), [Verified; 4]);

    // V1 accepts SetFee(1%); installed V2 rejects it with its stricter maximum.
    let rejected = step(a, ScenarioName::OrderA, 2);
    assert_eq!(
        rejected.reason.as_deref(),
        Some("config_execution_rejected")
    );
    assert_eq!(rejected.clock_before.as_ref().unwrap().slot, 447_850_494);
    assert_eq!(rejected.derived["rollback_verified"], true);
    let x = &a.evidence.executions[rejected.execution_id.as_ref().unwrap()];
    assert!(x
        .logs
        .iter()
        .any(|l| l.contains("SolDeposit fee exceeds V2 maximum 1/200")));
    let accepted = &a.evidence.executions[step(a, ScenarioName::OrderB, 0)
        .execution_id
        .as_ref()
        .unwrap()];
    assert!(!accepted.logs.iter().any(|l| l.contains("Rollout fixture")));

    // The inherited 1% survives the upgrade and V2's DepositSol applies it.
    let b = step(a, ScenarioName::OrderB, 3);
    assert_eq!(
        b.derived["pool_sol_deposit_fee"],
        serde_json::json!({"numerator": "1", "denominator": "100"})
    );
    assert_eq!(metric(b, "recipient_account_credit_raw"), "753375157");
    assert_eq!(metric(b, "manager_fee_account_credit_raw"), "7609851");
    assert_eq!(metric(b, "mint_supply_delta_raw"), "760985008");
    let bx = &a.evidence.executions[b.execution_id.as_ref().unwrap()];
    assert!(bx
        .logs
        .iter()
        .any(|l| l.contains("Rollout fixture: DepositSol")));

    let c = &a.report.comparison;
    assert_eq!(c.status, ComparisonStatus::RolloutOrderEffectObserved);
    assert_eq!(
        c.finding.as_deref(),
        Some("rollout/order/configuration_installation_outcome_differs")
    );
    assert!(c.statement.starts_with(
        "Under the pinned world and explicit signer assumptions, changing rollout order changed whether the proposed fee configuration could be installed"
    ));
    let d = c.first_divergence.as_ref().unwrap();
    assert_eq!(
        (d.step, d.order_a, d.order_b),
        (StepKind::SetFee, Rejected, Verified)
    );
    assert_eq!(c.final_action_outcomes, [NotExecuted, Verified]);
    assert_eq!(c.final_action_executed_differently, None);
    assert!(c.final_states_differ);
    // Not executed is unavailable, never zero.
    for (name, m) in &c.final_action_metrics {
        assert!(m.order_a.value.is_none(), "{name}");
        assert!(m.order_a.unavailable_reason.is_some());
        assert_eq!(m.differs, None);
    }
    assert_eq!(rollout::vm_executions(&a.report), 11);
    for word in ["unsafe", "vulnerable", "approved", "safe to deploy"] {
        assert!(!c.statement.to_lowercase().contains(word));
    }
}

#[test]
fn compatible_fee_reaches_equivalent_outcomes_in_both_orders() {
    let a = compatible();
    for s in rollout::SCENARIOS {
        assert!(a.report.scenario(s).unwrap().completed, "{s:?}");
    }
    let c = &a.report.comparison;
    assert_eq!(
        c.status,
        ComparisonStatus::NoOrderEffectObserved,
        "{}",
        c.statement
    );
    assert!(c.finding.is_none());
    assert!(c.first_divergence.is_none());
    assert_eq!(c.final_action_executed_differently, Some(false));
    assert_eq!(
        c.final_action_metrics["recipient_account_credit_raw"].order_a,
        c.final_action_metrics["recipient_account_credit_raw"].order_b
    );
    assert_eq!(
        c.final_action_metrics["manager_fee_account_credit_raw"].differs,
        Some(false)
    );
    // Aliased referral is unavailable in both orders, never zero.
    assert_eq!(
        c.final_action_metrics["referral_account_credit_raw"].differs,
        None
    );
    // Same installed bytes and same verified pool in both orders.
    assert!(c.step_contrasts.iter().all(|s| !s.differs));
    assert_eq!(
        a.report.anchors.installed_overlay.status,
        AnchorStatus::Matched
    );
}

#[test]
fn scenarios_restore_s0_independently_and_hand_off_byte_bearing_worlds() {
    let a = counterexample();
    let s0 = a.report.scenarios[0].initial_state_id.clone();
    let mut ids = std::collections::BTreeSet::new();
    for s in &a.report.scenarios {
        assert_eq!(s.initial_state_id, s0);
        assert!(ids.insert(s.scenario_id.clone()));
        let mut previous = s0.clone();
        for r in &s.steps {
            if r.outcome == StepOutcome::NotExecuted {
                assert!(r.before_state_id.is_none() && r.after_state_id.is_none());
                continue;
            }
            assert_eq!(r.before_state_id.as_ref(), Some(&previous));
            if let Some(id) = &r.execution_id {
                let x = &a.evidence.executions[id];
                assert_eq!(&x.before_state_id, &previous);
                // The after state is exactly the execution's post rows.
                assert_eq!(state(a, &r.after_state_id).accounts, x.post);
            }
            previous = r.after_state_id.clone().unwrap();
        }
    }
    // Order A and B share the rollout root but are distinct scenario identities.
    let ids = &a.report.comparison.order_scenario_ids;
    assert_ne!(ids[0], ids[1]);
    // Every intermediate world is byte-bearing: each Present row resolves.
    for st in a.evidence.states.values() {
        for entry in st.accounts.values() {
            if let Entry::Present { account_sha256 } = entry {
                assert!(a.evidence.accounts.contains_key(account_sha256));
            }
        }
    }
    let pd = loader::programdata_address(&historical().record.program_id.parse().unwrap());
    let upgraded = state(a, &step(a, ScenarioName::OrderA, 0).after_state_id);
    assert_eq!(
        upgraded
            .account(&a.evidence.accounts, &pd.to_string())
            .unwrap()
            .unwrap()
            .data
            .len(),
        1_080_509
    );
}

#[test]
fn known_absence_is_not_unknown_state() {
    let p = prepared();
    assert_eq!(p.s0.accounts[DEPOSITOR], Entry::KnownAbsent);
    assert_eq!(p.s0.accounts[WITHDRAW_AUTHORITY], Entry::KnownAbsent);
    assert!(p.s0.account(&p.s0_accounts, DEPOSITOR).unwrap().is_none());
    assert!(p
        .s0
        .account(&p.s0_accounts, &bs58::encode([93u8; 32]).into_string())
        .is_err());
    // Removing an absence proof makes the action an evidence gap, not a run
    // over a default account.
    let mut s = p.s0.clone();
    s.accounts.remove(DEPOSITOR);
    let (r, _, e) = p
        .probe(StepKind::DepositSol, &s, &p.s0_accounts, Variant::Declared)
        .unwrap();
    assert_eq!(r.outcome, StepOutcome::EvidenceGap);
    assert_eq!(r.reason.as_deref(), Some("undeclared_account"));
    assert!(e.executions.is_empty());
}

#[test]
fn a_write_outside_the_declared_closure_fails_the_step() {
    let mut a = counterexample().clone();
    let scenario = a
        .report
        .scenarios
        .iter_mut()
        .find(|s| s.name == ScenarioName::OrderB)
        .unwrap();
    let old = scenario.steps[1].execution_id.clone().unwrap();
    let mut x = a.evidence.executions.remove(&old).unwrap();
    x.outside_writes
        .insert(bs58::encode([94u8; 32]).into_string());
    let id = x.id().unwrap();
    a.evidence.executions.insert(id.clone(), x);
    scenario.steps[1].execution_id = Some(id);
    let reduced = rollout::reduce(&a).unwrap();
    let order_b = reduced.report.scenario(ScenarioName::OrderB).unwrap();
    assert_eq!(order_b.steps[1].outcome, StepOutcome::UnexpectedWrite);
    assert!(order_b.steps[2..]
        .iter()
        .all(|r| r.outcome == StepOutcome::NotExecuted));
    assert_eq!(
        reduced.report.comparison.status,
        ComparisonStatus::RolloutNotEstablished
    );
}

#[test]
fn identities_bind_inputs_not_labels_or_results() {
    let p = prepared();
    let h = historical();
    let mut upgrade = ChangeSpec::program_upgrade(&h.record.program_id, &candidate());
    upgrade.metadata.label = Some("display only".into());
    let labelled = rollout::Prepared::new(
        rollout::Input::new(&upgrade, &parameter(1, 100), h, candidate()).unwrap(),
    )
    .unwrap();
    assert_eq!(labelled.analysis_input_id, p.analysis_input_id);
    let other = rollout::Prepared::new(input_with(candidate(), 1, 1000)).unwrap();
    assert_ne!(other.analysis_input_id, p.analysis_input_id);
    assert_eq!(other.s0, p.s0, "the proposal is not part of the world");
    assert_eq!(
        counterexample().report.analysis_input_id,
        p.analysis_input_id
    );
    // ChangeSpec identities are the proposals' own, unchanged.
    assert_eq!(
        counterexample().report.parameter_change_spec_id,
        parameter(1, 100).id().unwrap()
    );
    assert!(p.contract.get("report").is_none() && p.contract.get("comparison").is_none());
}

#[test]
fn same_code_upgrade_is_a_no_effect_control_at_exact_capacity() {
    let a = rollout::analyse(&input_with(v1(), 1, 100)).unwrap();
    assert_eq!(a.report.preflight.capacity.required_bytes, 1_080_464);
    assert!(a.report.preflight.capacity.sufficient);
    assert_eq!(
        a.report.preflight.candidate_profile.as_deref(),
        Some(rollout::SAME_CODE_PROFILE)
    );
    assert_eq!(
        a.report.comparison.status,
        ComparisonStatus::NoOrderEffectObserved,
        "{}",
        a.report.comparison.statement
    );
    assert_eq!(
        a.report.anchors.installed_overlay.status,
        AnchorStatus::Matched
    );
}

// ---------------------------------------------------------------------------
// Portable evidence
// ---------------------------------------------------------------------------

#[test]
fn verify_and_reproduce_the_retained_analysis() {
    let a = counterexample();
    rollout::verify(a).unwrap();
    assert_eq!(rollout::reproduce(a).unwrap(), *a);

    // Tampered retained execution: the reduction no longer supports the report.
    let mut t = a.clone();
    let deposit = step(&t, ScenarioName::OrderB, 3).clone();
    let old = deposit.execution_id.clone().unwrap();
    let mut x = t.evidence.executions.remove(&old).unwrap();
    let recipient = "EpomeRdrhUeAmVyECwima2gBpY3jUoA6cee89EcADaPT";
    let Entry::Present { account_sha256 } = x.post[recipient].clone() else {
        panic!()
    };
    let mut account = t.evidence.accounts[&account_sha256].clone();
    account.data[64] ^= 1;
    x.post.insert(
        recipient.into(),
        world::put(&mut t.evidence.accounts, Some(account)).unwrap(),
    );
    let id = x.id().unwrap();
    t.evidence.executions.insert(id.clone(), x);
    t.report
        .scenarios
        .iter_mut()
        .find(|s| s.name == ScenarioName::OrderB)
        .unwrap()
        .steps[3]
        .execution_id = Some(id);
    reseal(&mut t);
    assert!(rollout::verify(&t).is_err());

    // A retained execution whose logs were rewritten is internally consistent
    // for steps that do not read logs; only re-execution exposes it.
    let mut t = a.clone();
    // Order B's SetFee is content-identical to the configuration control's,
    // which keeps referencing the original object.
    let k = step(&t, ScenarioName::OrderB, 0).clone();
    let old = k.execution_id.clone().unwrap();
    let mut x = t.evidence.executions[&old].clone();
    x.logs.push("rewritten".into());
    let id = x.id().unwrap();
    t.evidence.executions.insert(id.clone(), x);
    let s = t
        .report
        .scenarios
        .iter_mut()
        .find(|s| s.name == ScenarioName::OrderB)
        .unwrap();
    s.steps[0].execution_id = Some(id);
    reseal(&mut t);
    rollout::verify(&t).unwrap();
    assert!(rollout::reproduce(&t).is_err());
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "eplyx-rollout-{}-{name}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rewrite_object(dir: &Path, name: &str, bytes: &[u8]) {
    let manifest_path = dir.join("manifest.json");
    let mut manifest: artifact::Manifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let hash = eplyx_engine::replay::hash_bytes(bytes);
    std::fs::write(dir.join("objects").join(&hash), bytes).unwrap();
    manifest.objects.insert(name.into(), hash);
    std::fs::write(
        &manifest_path,
        eplyx_engine::canonical::document(&manifest).unwrap(),
    )
    .unwrap();
}

#[test]
fn portable_artifact_round_trips_and_refuses_tampering() {
    let a = counterexample();
    let base = temp("artifact");
    let dir = base.join("rollout");
    let manifest = artifact::save(a, &dir).unwrap();
    assert!(artifact::save(a, &dir).is_err(), "never overwritten");
    assert_eq!(artifact::load(&dir).unwrap(), *a);
    for prefix in [
        "states/",
        "executions/",
        "accounts/",
        "programs/",
        "proposals/",
    ] {
        assert!(
            manifest.objects.keys().any(|k| k.starts_with(prefix)),
            "{prefix}"
        );
    }
    for name in ["input.json", "contract.json", "report.json", "report.md"] {
        assert!(manifest.objects.contains_key(name));
    }
    // Identical content is stored once.
    let distinct: std::collections::BTreeSet<_> = manifest.objects.values().collect();
    assert_eq!(
        distinct.len(),
        std::fs::read_dir(dir.join("objects")).unwrap().count()
    );

    // A flipped byte in a state object.
    let copy = base.join("flipped");
    copy_dir(&dir, &copy);
    let state_hash = manifest
        .objects
        .iter()
        .find(|(k, _)| k.starts_with("states/"))
        .unwrap()
        .1;
    let path = copy.join("objects").join(state_hash);
    let mut bytes = std::fs::read(&path).unwrap();
    let i = bytes.len() / 2;
    bytes[i] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(artifact::load(&copy).is_err());

    // A changed, resealed and re-indexed report.
    let copy = base.join("resealed");
    copy_dir(&dir, &copy);
    let mut report = a.report.clone();
    report.comparison.status = ComparisonStatus::NoOrderEffectObserved;
    report.comparison.finding = None;
    report.report_sha256 = rollout::report_digest(&report).unwrap();
    rewrite_object(
        &copy,
        "report.json",
        eplyx_engine::canonical::document(&report)
            .unwrap()
            .as_bytes(),
    );
    let error = artifact::load(&copy).unwrap_err().to_string();
    assert!(error.contains("differ"), "{error}");

    // An extra object the analysis does not derive.
    let copy = base.join("extra");
    copy_dir(&dir, &copy);
    rewrite_object(&copy, "states/unrelated", b"{}\n");
    assert!(artifact::load(&copy).is_err());
    std::fs::remove_dir_all(base).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to.join("objects")).unwrap();
    for name in ["manifest.json", "report.md"] {
        std::fs::copy(from.join(name), to.join(name)).unwrap();
    }
    for entry in std::fs::read_dir(from.join("objects")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join("objects").join(entry.file_name())).unwrap();
    }
}

// ---------------------------------------------------------------------------
// CLI: the built binary, its empty-environment worker, and offline reproduction
// ---------------------------------------------------------------------------

fn eplyx(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn cli_analyses_verifies_and_reproduces_offline() {
    let base = temp("cli");
    let upgrade = base.join("upgrade.json");
    std::fs::write(
        &upgrade,
        ChangeSpec::program_upgrade(&historical().record.program_id, &candidate())
            .to_document()
            .unwrap(),
    )
    .unwrap();
    let out = base.join("rollout");
    let candidate_path = root().join("fixtures/rollout/fixture_stake_pool_rollout_v2.so");
    let parameter_path = root().join("docs/examples/stake-pool-parameter-change.json");
    let bundle = root().join("deploy/bundle");
    let analyse = |out: &Path, candidate: &Path| {
        eplyx(&[
            "rollout",
            "analyse",
            "--upgrade",
            upgrade.to_str().unwrap(),
            "--parameter",
            parameter_path.to_str().unwrap(),
            "--bundle",
            bundle.to_str().unwrap(),
            "--record-id",
            RECORD,
            "--candidate",
            candidate.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--format",
            "json",
        ])
    };
    let o = analyse(&out, &candidate_path);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let receipt: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(receipt["operation"], "analysed");
    assert_eq!(receipt["status"], "rollout_order_effect_observed");
    assert_eq!(receipt["vm_executions"], 11);
    assert_eq!(
        receipt["report_sha256"],
        counterexample().report.report_sha256
    );
    // Fresh output only.
    let again = analyse(&out, &candidate_path);
    assert_eq!(again.status.code(), Some(2));

    let o = eplyx(&[
        "rollout",
        "verify",
        "--artifact",
        out.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(o.status.success());
    let verified: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(verified["vm_executions"], 0);
    assert_eq!(verified["report_sha256"], receipt["report_sha256"]);

    // Reproduce from a copy of the artifact alone, in the empty-environment
    // worker. On Linux the whole command also runs in a private network
    // namespace, so no network is reachable at all.
    let moved = base.join("moved");
    copy_dir(&out, &moved);
    let reproduce = [
        "rollout",
        "reproduce",
        "--artifact",
        moved.to_str().unwrap(),
        "--format",
        "json",
    ];
    let o = if cfg!(target_os = "linux") {
        Command::new("unshare")
            .args(["-rn", env!("CARGO_BIN_EXE_eplyx")])
            .args(reproduce)
            .output()
            .expect("unshare is required on Linux to qualify network-denied reproduction")
    } else {
        eplyx(&reproduce)
    };
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let reproduced: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(reproduced["operation"], "reproduced");
    assert_eq!(reproduced["vm_executions"], 11);
    assert_eq!(reproduced["report_sha256"], receipt["report_sha256"]);

    // A refused preflight is a structured error with exit 2.
    let mut wrong = candidate();
    wrong[200] ^= 1;
    let wrong_path = base.join("wrong.so");
    std::fs::write(&wrong_path, wrong).unwrap();
    let o = analyse(&base.join("refused"), &wrong_path);
    assert_eq!(o.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("candidate_identity_mismatch"));
    assert!(!base.join("refused").exists());
    std::fs::remove_dir_all(base).unwrap();
}
