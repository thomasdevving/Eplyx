//! Step 16B: explicit ProgramData preparation for the rollout rehearsal —
//! read-only capacity preflight, the loader's own ExtendProgram, Upgrade into
//! the resized ProgramData, and the Step 16A order comparison on top.
//!
//! Every execution runs real SBF bytecode under the pinned loader-v3: retained
//! V1 and Token from deploy/bundle, and the tracked constructed oversized
//! rollout counterexample (fixtures/rollout), reproducible with
//! scripts/build-stake-pool-oversized-rollout-candidate.sh. No outcome is
//! hard-coded into the analyzer; every asserted result is what the VM produced.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use eplyx_engine::{
    bundle::CiBundle,
    change::{Change, ChangeSpec},
    parameter_change::{stake_pool, Operation},
    rollout::{
        self, artifact,
        preparation::{self, Plan, PreflightStatus, Preparation},
        world::{self, Entry},
        AnchorStatus, ComparisonStatus, ScenarioName, StepKind, StepOutcome, Variant,
    },
    standard_programs::upgradeable_loader as loader,
};

const RECORD: &str = "mainnet-spl-stake-pool-151010f709e113e7";
const PROGRAMDATA: &str = "EmiU8AQkB2sswTxVB6aCmsAJftoowZGGDXuytm6X65R3";
const POOL: &str = "CV6bkrUksMwcEC4jfLTJsbHwF3Y2YurZdWWua95Fpbtd";
const REQUIRED: u64 = 65_920;
const SLOT: u64 = 447_850_493;

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

fn oversized() -> Vec<u8> {
    std::fs::read(root().join("fixtures/rollout/fixture_stake_pool_oversized_rollout_v2.so"))
        .expect("tracked oversized candidate missing; scripts/build-stake-pool-oversized-rollout-candidate.sh reproduces it")
}

fn fitting() -> Vec<u8> {
    std::fs::read(root().join("fixtures/rollout/fixture_stake_pool_rollout_v2.so")).unwrap()
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

fn base(candidate: Vec<u8>) -> rollout::Input {
    let h = historical();
    let upgrade = ChangeSpec::program_upgrade(&h.record.program_id, &candidate);
    rollout::Input::new(&upgrade, &parameter(1, 100), h, candidate).unwrap()
}

fn extend(n: u64) -> Preparation {
    Preparation::ExtendProgram {
        additional_bytes: n,
    }
}

fn input(candidate: Vec<u8>, preparation: Preparation) -> rollout::Input {
    base(candidate).with_preparation(preparation)
}

fn prepared(preparation: Preparation) -> rollout::Prepared {
    rollout::Prepared::new(input(oversized(), preparation)).unwrap()
}

/// The declared rollout with exactly the required extension.
fn exact() -> &'static rollout::Analysis {
    static A: OnceLock<rollout::Analysis> = OnceLock::new();
    A.get_or_init(|| rollout::analyse(&input(oversized(), extend(REQUIRED))).unwrap())
}

/// The declared rollout without any preparation.
fn missing() -> &'static rollout::Analysis {
    static A: OnceLock<rollout::Analysis> = OnceLock::new();
    A.get_or_init(|| rollout::analyse(&input(oversized(), Preparation::None)).unwrap())
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

fn account<'a>(
    a: &'a rollout::Analysis,
    id: &Option<String>,
    address: &str,
) -> &'a eplyx_engine::types::AccountSnapshot {
    state(a, id)
        .account(&a.evidence.accounts, address)
        .unwrap()
        .unwrap()
}

fn metric(r: &rollout::StepRecord, name: &str) -> String {
    r.derived["reconciliation"][name]
        .as_str()
        .unwrap()
        .to_string()
}

/// Independent of the analyzer: SIMD-0194 rent, 128-byte storage overhead.
fn rent_minimum(len: u64) -> u64 {
    ((128 + len) * 6960).max(1)
}

fn reseal(a: &mut rollout::Analysis) {
    a.report.evidence = a.evidence.index();
    a.report.report_sha256 = rollout::report_digest(&a.report).unwrap();
}

// ---------------------------------------------------------------------------
// Preflight and plan
// ---------------------------------------------------------------------------

#[test]
fn preflight_derives_capacity_from_the_loader_account_format() {
    let fits = rollout::Prepared::new(input(fitting(), Preparation::None)).unwrap();
    let f = &fits.programdata_preflight;
    assert_eq!(f.status, PreflightStatus::ReadyWithoutExtension);
    assert!(f.candidate_fits);
    assert_eq!(f.minimum_additional_bytes_required, 0);

    let f = prepared(Preparation::None).programdata_preflight;
    assert_eq!(f.status, PreflightStatus::ExtensionRequired);
    assert_eq!(f.reason.as_deref(), Some("programdata_extension_required"));
    assert_eq!(f.programdata_address, PROGRAMDATA);
    assert_eq!(f.programdata_account_len, 1_080_509);
    assert_eq!(f.programdata_metadata_len, 45);
    assert_eq!(f.executable_capacity_bytes, 1_080_464);
    assert_eq!(f.current_executable.len, 1_080_464);
    assert_eq!(
        f.current_executable.sha256,
        eplyx_engine::replay::hash_bytes(&v1())
    );
    assert_eq!(f.candidate.len, 1_146_384);
    assert_eq!(f.candidate.sha256, rollout::OVERSIZED_CANDIDATE_SHA256);
    assert!(!f.candidate_fits);
    assert_eq!(f.minimum_additional_bytes_required, REQUIRED);
    assert!(f.funding.is_none(), "nothing declared, nothing funded");
    assert_eq!(
        f.loader_rules["upgrade_authority_signature_required"],
        false
    );

    let f = prepared(extend(REQUIRED)).programdata_preflight;
    assert_eq!(f.status, PreflightStatus::DeclaredExtensionSufficient);
    assert_eq!(f.surplus_capacity_bytes, Some(0));
    assert_eq!(f.extended_account_len, Some(1_080_509 + REQUIRED));
    let funding = f.funding.unwrap();
    assert_eq!(
        funding.minimum_balance_after,
        rent_minimum(1_080_509 + REQUIRED)
    );
    assert_eq!(funding.programdata_lamports_before, rent_minimum(1_080_509));
    assert_eq!(
        funding.required_funding_lamports,
        rent_minimum(1_080_509 + REQUIRED) - rent_minimum(1_080_509)
    );
    assert_eq!(funding.payer_origin, "assumed_simulation_only");
    assert!(funding.payer_covers_funding_and_fee);

    let f = prepared(extend(REQUIRED - 1)).programdata_preflight;
    assert_eq!(f.status, PreflightStatus::DeclaredExtensionInsufficient);
    assert_eq!(f.shortfall_bytes, Some(1));

    let f = prepared(extend(REQUIRED + 20_480)).programdata_preflight;
    assert_eq!(f.status, PreflightStatus::DeclaredExtensionSufficient);
    assert_eq!(f.surplus_capacity_bytes, Some(20_480));

    for (n, reason) in [
        (0, "zero_extension"),
        (4_096, "below_minimum_extension"),
        (10 * 1024 * 1024, "programdata_size_overflow"),
        (u64::from(u32::MAX) + 1, "programdata_size_overflow"),
    ] {
        let f = prepared(extend(n)).programdata_preflight;
        assert_eq!(f.status, PreflightStatus::ExtensionUnsupported, "{n}");
        assert_eq!(f.reason.as_deref(), Some(reason), "{n}");
    }
}

#[test]
fn rent_identity_matches_the_runtime_rent_sysvar() {
    let svm = litesvm::LiteSVM::new();
    let runtime = svm.get_sysvar::<solana_rent::Rent>();
    for len in [0usize, 1_080_509, 1_146_429, 1_166_909] {
        assert_eq!(
            runtime.minimum_balance(len),
            preparation::minimum_balance(len as u64).unwrap(),
            "{len}"
        );
        assert_eq!(runtime.minimum_balance(len), rent_minimum(len as u64));
    }
}

#[test]
fn plan_documents_are_strict_and_versioned() {
    let ok = Plan::parse(
        br#"{"schema_version":1,"programdata_preparation":{"kind":"extend_program","additional_bytes":"65920"}}"#,
    )
    .unwrap();
    assert_eq!(ok.programdata_preparation, extend(REQUIRED));
    assert_eq!(
        Plan::parse(br#"{"schema_version":1,"programdata_preparation":{"kind":"none"}}"#)
            .unwrap()
            .programdata_preparation,
        Preparation::None
    );
    for bad in [
        r#"{"schema_version":1,"programdata_preparation":{"kind":"extend_program","additional_bytes":65920}}"#,
        r#"{"schema_version":1,"programdata_preparation":{"kind":"extend_program","additional_bytes":"065920"}}"#,
        r#"{"schema_version":1,"programdata_preparation":{"kind":"extend_program","additional_bytes":"-1"}}"#,
        r#"{"schema_version":1,"programdata_preparation":{"kind":"extend_program","additional_bytes":"65920","payer":"x"}}"#,
        r#"{"schema_version":1,"programdata_preparation":{"kind":"none","additional_bytes":"1"}}"#,
        r#"{"schema_version":1,"programdata_preparation":{"kind":"auto"}}"#,
        r#"{"schema_version":2,"programdata_preparation":{"kind":"none"}}"#,
        r#"{"schema_version":1,"programdata_preparation":{"kind":"none"},"steps":[]}"#,
        r#"{"schema_version":1}"#,
    ] {
        assert!(Plan::parse(bad.as_bytes()).is_err(), "{bad}");
    }
}

// ---------------------------------------------------------------------------
// The loader's ExtendProgram
// ---------------------------------------------------------------------------

#[test]
fn extend_program_executes_and_every_delta_reconciles() {
    let a = exact();
    let r = step(a, ScenarioName::PreparationControl, 0);
    assert_eq!(r.step, StepKind::ExtendProgram);
    assert_eq!(r.outcome, StepOutcome::Verified, "{:?}", r.detail);
    let x = &a.evidence.executions[r.execution_id.as_ref().unwrap()];
    assert!(x
        .logs
        .iter()
        .any(|l| l.contains("Extended ProgramData account by 65920 bytes")));
    // Exactly the interface's metas: payer (fee payer, signer), ProgramData,
    // Program, System, loader. No upgrade authority participates.
    let keys = &x.message.account_keys;
    assert_eq!(keys[0], preparation::extension_payer());
    assert_eq!(x.message.required_signatures, 1);
    assert!(!keys.contains(&rollout::upgrade_authority()));
    let ix = &x.message.instructions[0];
    assert_eq!(
        keys[usize::from(ix.program_index)],
        loader::id().to_string()
    );
    let metas: Vec<_> = ix
        .account_indices
        .iter()
        .map(|i| keys[usize::from(*i)].clone())
        .collect();
    assert_eq!(
        metas,
        [
            PROGRAMDATA.to_string(),
            historical().record.program_id,
            "11111111111111111111111111111111".to_string(),
            preparation::extension_payer()
        ]
    );
    let mut data = 6u32.to_le_bytes().to_vec();
    data.extend_from_slice(&(REQUIRED as u32).to_le_bytes());
    assert_eq!(ix.data, data, "official ExtendProgram encoding");

    let before = state(a, &r.before_state_id);
    let after = state(a, &r.after_state_id);
    let pd_before = account(a, &r.before_state_id, PROGRAMDATA);
    let pd_after = account(a, &r.after_state_id, PROGRAMDATA);
    assert_eq!(
        pd_after.data.len(),
        pd_before.data.len() + REQUIRED as usize
    );
    let old = loader::decode_programdata(&pd_before.data).unwrap();
    let new = loader::decode_programdata(&pd_after.data).unwrap();
    assert_eq!(old.bytes, v1());
    assert_eq!(new.bytes[..old.bytes.len()], old.bytes[..]);
    assert!(new.bytes[old.bytes.len()..].iter().all(|b| *b == 0));
    assert_eq!(new.upgrade_authority, old.upgrade_authority);
    assert_eq!(old.deploy_slot, 429_882_117);
    assert_eq!(
        new.deploy_slot, SLOT,
        "ExtendProgram redeploys at Clock.slot"
    );
    // Rent and funding, against the independent formula.
    let funding = rent_minimum(1_080_509 + REQUIRED) - rent_minimum(1_080_509);
    assert_eq!(pd_before.lamports, rent_minimum(1_080_509));
    assert_eq!(pd_after.lamports, rent_minimum(1_080_509 + REQUIRED));
    let payer_before = account(a, &r.before_state_id, &preparation::extension_payer());
    let payer_after = account(a, &r.after_state_id, &preparation::extension_payer());
    assert_eq!(x.transaction_fee_lamports, 5_000);
    assert_eq!(
        payer_after.lamports,
        payer_before.lamports - funding - x.transaction_fee_lamports
    );
    assert_eq!(r.derived["funding_lamports"], funding.to_string());
    // One System transfer CPI of exactly the funding.
    let inner: Vec<_> = x
        .inner_instructions
        .iter()
        .flat_map(|g| &g.instructions)
        .collect();
    assert_eq!(inner.len(), 1);
    assert_eq!(
        keys[usize::from(inner[0].program_id_index)],
        "11111111111111111111111111111111"
    );
    // Only ProgramData and the payer changed; the Program relationship holds.
    assert_eq!(
        world::changed(before, after),
        [PROGRAMDATA.to_string(), preparation::extension_payer()].into()
    );
    let program = account(a, &r.after_state_id, &historical().record.program_id);
    assert_eq!(
        loader::decode_program(&program.data).unwrap().to_string(),
        PROGRAMDATA
    );
    // No candidate bytes are installed by ExtendProgram.
    assert_eq!(r.derived["candidate_installed"], false);
    assert_eq!(
        r.installed_program_after.as_ref().unwrap().identity,
        "historical_v1"
    );
    assert_eq!(
        after.accounts[&rollout::buffer_address()],
        before.accounts[&rollout::buffer_address()]
    );
}

#[test]
fn extension_alone_preserves_the_installed_v1_action() {
    let a = exact();
    let anchor = a.report.anchors.extension_only.as_ref().unwrap();
    assert_eq!(anchor.status, AnchorStatus::Matched, "{:?}", anchor.detail);
    assert_eq!(anchor.detail["mismatches"], serde_json::json!([]));
    let base = step(a, ScenarioName::Control, 0);
    let ext = step(a, ScenarioName::PreparationControl, 2);
    assert_eq!(
        base.derived["reconciliation"],
        ext.derived["reconciliation"]
    );
    assert_eq!(base.derived["compute_units"], ext.derived["compute_units"]);
    assert_eq!(metric(ext, "recipient_account_credit_raw"), "760985008");
    assert_eq!(ext.clock_before.as_ref().unwrap().slot, SLOT + 1);
}

#[test]
fn missing_extension_is_an_explicit_prerequisite_and_never_inserted() {
    let a = missing();
    assert_eq!(
        a.report.comparison.status,
        ComparisonStatus::RolloutPreconditionMissing
    );
    assert_eq!(
        a.report.comparison.finding.as_deref(),
        Some("rollout/precondition/programdata_extension_required")
    );
    assert_eq!(
        a.report.comparison.statement,
        "The declared rollout cannot install the candidate because the retained ProgramData account is too small. At least 65920 additional bytes are required."
    );
    // The declared plan is executed as declared: no step anywhere extends.
    for s in &a.report.scenarios {
        assert!(!s.declared_steps.contains(&StepKind::ExtendProgram));
    }
    for s in [
        ScenarioName::UpgradeControl,
        ScenarioName::OrderA,
        ScenarioName::OrderB,
    ] {
        let up = a
            .report
            .scenario(s)
            .unwrap()
            .steps
            .iter()
            .find(|r| r.step == StepKind::Upgrade)
            .unwrap();
        assert_eq!(up.outcome, StepOutcome::Unsupported);
        assert_eq!(up.reason.as_deref(), Some("programdata_extension_required"));
        assert!(
            up.execution_id.is_none(),
            "no impossible Upgrade is executed"
        );
        assert_eq!(up.derived["additional_bytes_required"], REQUIRED);
        assert_eq!(up.derived["executable_capacity_bytes"], 1_080_464);
        assert_eq!(up.derived["programdata_address"], PROGRAMDATA);
    }
    // No state mutation of ProgramData anywhere.
    let s0 = &a.report.scenarios[0].initial_state_id;
    for st in a.evidence.states.values() {
        assert_eq!(
            st.accounts[PROGRAMDATA],
            a.evidence.states[s0].accounts[PROGRAMDATA]
        );
    }
    assert_eq!(
        a.report.programdata_preflight.as_ref().unwrap().status,
        PreflightStatus::ExtensionRequired
    );
    // The analysis with the extension also shows what happens without it.
    let p1 = step(exact(), ScenarioName::UpgradeWithoutPreparation, 0);
    assert_eq!(p1.outcome, StepOutcome::Unsupported);
    assert_eq!(p1.reason.as_deref(), Some("programdata_extension_required"));
    rollout::verify(a).unwrap();
}

#[test]
fn too_small_extension_executes_but_stays_insufficient() {
    let a = rollout::analyse(&input(oversized(), extend(REQUIRED - 1))).unwrap();
    use StepOutcome::*;
    assert_eq!(
        outcomes(&a, ScenarioName::UpgradeControl),
        [Verified, Verified, Unsupported, NotExecuted, NotExecuted]
    );
    let up = step(&a, ScenarioName::UpgradeControl, 2);
    assert_eq!(
        up.reason.as_deref(),
        Some("declared_extension_insufficient")
    );
    assert_eq!(up.derived["additional_bytes_required"], 1);
    assert_eq!(up.derived["extended_in_this_scenario"], true);
    assert!(up.execution_id.is_none());
    // The successful extension is retained, not rolled back: the rollout is
    // multi-transaction.
    let ext = step(&a, ScenarioName::UpgradeControl, 0);
    assert_eq!(
        account(&a, &ext.after_state_id, PROGRAMDATA).data.len(),
        1_080_509 + REQUIRED as usize - 1
    );
    assert_eq!(
        a.report
            .scenario(ScenarioName::UpgradeControl)
            .unwrap()
            .final_state_id,
        step(&a, ScenarioName::UpgradeControl, 1)
            .after_state_id
            .clone()
            .unwrap()
    );
    assert_eq!(
        outcomes(&a, ScenarioName::OrderB),
        [
            Verified,
            Verified,
            Verified,
            Unsupported,
            NotExecuted,
            NotExecuted
        ]
    );
    assert_eq!(
        a.report.comparison.status,
        ComparisonStatus::RolloutPreconditionMissing
    );
    assert_eq!(
        a.report.comparison.finding.as_deref(),
        Some("rollout/precondition/declared_extension_insufficient")
    );
    assert!(a
        .report
        .comparison
        .statement
        .starts_with("The declared ProgramData extension of 65919 bytes executed"));
    for m in a.report.comparison.final_action_metrics.values() {
        assert!(m.order_a.value.is_none() && m.order_b.value.is_none());
    }
}

#[test]
fn larger_extension_leaves_unused_capacity_that_is_not_semantic() {
    let a = rollout::analyse(&input(oversized(), extend(REQUIRED + 20_480))).unwrap();
    let up = step(&a, ScenarioName::UpgradeControl, 2);
    assert_eq!(up.outcome, StepOutcome::Verified, "{:?}", up.detail);
    assert_eq!(up.derived["zero_padding_bytes"], 20_480);
    assert_eq!(
        up.installed_program_after.as_ref().unwrap().identity,
        "candidate"
    );
    assert_eq!(
        a.report.anchors.installed_overlay.status,
        AnchorStatus::Matched
    );
    assert_eq!(
        a.report.anchors.extension_only.as_ref().unwrap().status,
        AnchorStatus::Matched
    );
    assert_eq!(
        a.report.comparison.status,
        ComparisonStatus::RolloutOrderEffectObserved
    );
    assert_eq!(
        a.report.comparison.final_action_metrics,
        exact().report.comparison.final_action_metrics
    );
}

fn s0_with(
    p: &rollout::Prepared,
    address: &str,
    edit: impl FnOnce(&mut eplyx_engine::types::AccountSnapshot),
) -> (world::State, world::AccountStore) {
    let mut store = p.s0_accounts.clone();
    let mut a = p.s0.account(&store, address).unwrap().unwrap().clone();
    edit(&mut a);
    let s = p.s0.with_account(&mut store, address, Some(a)).unwrap();
    (s, store)
}

#[test]
fn rejected_extensions_roll_back_with_typed_loader_evidence() {
    // Zero, below SIMD-0431's minimum, and beyond the 10 MiB account maximum.
    for (n, reason, error) in [
        (0, "extend_program_rejected", "InvalidInstructionData"),
        (4_096, "extend_program_rejected", "InvalidArgument"),
        (
            10 * 1024 * 1024,
            "programdata_size_overflow",
            "InvalidRealloc",
        ),
    ] {
        let p = prepared(extend(n));
        let (r, next, e) = p
            .probe(
                StepKind::ExtendProgram,
                &p.s0,
                &p.s0_accounts,
                Variant::Declared,
            )
            .unwrap();
        assert_eq!(r.outcome, StepOutcome::Rejected, "{n}: {:?}", r.detail);
        assert_eq!(r.reason.as_deref(), Some(reason), "{n}");
        assert!(
            r.detail.as_ref().unwrap().contains(error),
            "{n}: {:?}",
            r.detail
        );
        assert_eq!(r.derived["rollback_verified"], true);
        assert!(next.is_none());
        let after = &e.states[r.after_state_id.as_ref().unwrap()];
        assert_eq!(after.accounts[PROGRAMDATA], p.s0.accounts[PROGRAMDATA]);
        assert_eq!(
            world::changed(&p.s0, after),
            [preparation::extension_payer()].into()
        );
    }
    // Not a u32: the declared plan cannot be encoded, so it is refused.
    let error = rollout::analyse(&input(oversized(), extend(u64::from(u32::MAX) + 1)))
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("programdata_size_overflow"), "{error}");
}

#[test]
fn insufficient_payer_rejects_without_touching_programdata() {
    let p = prepared(extend(REQUIRED));
    let (s, store) = s0_with(&p, &preparation::extension_payer(), |a| {
        a.lamports = 1_000_000;
    });
    let (r, next, e) = p
        .probe(StepKind::ExtendProgram, &s, &store, Variant::Declared)
        .unwrap();
    assert_eq!(r.outcome, StepOutcome::Rejected, "{:?}", r.detail);
    assert_eq!(r.reason.as_deref(), Some("extension_funding_insufficient"));
    assert!(next.is_none());
    let after = &e.states[r.after_state_id.as_ref().unwrap()];
    assert_eq!(after.accounts[PROGRAMDATA], s.accounts[PROGRAMDATA]);
    let payer = after
        .account(&e.accounts, &preparation::extension_payer())
        .unwrap()
        .unwrap();
    assert_eq!(payer.lamports, 1_000_000 - 5_000, "only the fee is charged");
    // In a scenario, nothing after a rejected extension runs.
    let (scenario, _) = {
        let p = rollout::Prepared::new(input(oversized(), extend(0))).unwrap();
        p.run_sequence(&[
            StepKind::ExtendProgram,
            StepKind::AdvanceToVisibleSlot,
            StepKind::Upgrade,
        ])
        .unwrap()
    };
    assert_eq!(scenario.steps[0].outcome, StepOutcome::Rejected);
    assert!(scenario.steps[1..]
        .iter()
        .all(|r| r.outcome == StepOutcome::NotExecuted && r.execution_id.is_none()));
}

#[test]
fn a_wrong_program_relationship_is_rejected_by_the_loader() {
    let p = prepared(extend(REQUIRED));
    let other: solana_address::Address = "11111111111111111111111111111112".parse().unwrap();
    let (s, store) = s0_with(&p, &historical().record.program_id, |a| {
        a.data = loader::encode::program(&other);
    });
    let (r, _, e) = p
        .probe(StepKind::ExtendProgram, &s, &store, Variant::Declared)
        .unwrap();
    assert_eq!(r.outcome, StepOutcome::Rejected, "{:?}", r.detail);
    assert_eq!(r.reason.as_deref(), Some("extend_program_rejected"));
    let x = &e.executions[r.execution_id.as_ref().unwrap()];
    assert!(x
        .logs
        .iter()
        .any(|l| l.contains("Program account does not match ProgramData account")));
}

// ---------------------------------------------------------------------------
// Visibility and the upgrade after extension
// ---------------------------------------------------------------------------

#[test]
fn extension_redeploys_so_the_existing_visibility_rule_applies() {
    let p = prepared(extend(REQUIRED));
    // Same-slot action after an extension is not mainnet-equivalent.
    let (s, _) = p
        .run_sequence(&[StepKind::ExtendProgram, StepKind::DepositSol])
        .unwrap();
    assert_eq!(s.steps[0].outcome, StepOutcome::Verified);
    assert_eq!(s.steps[1].outcome, StepOutcome::Unsupported);
    assert_eq!(
        s.steps[1].reason.as_deref(),
        Some("visibility_boundary_not_crossed")
    );
    // The loader itself refuses an Upgrade in the extension's slot.
    let (s, e) = p
        .run_sequence(&[StepKind::ExtendProgram, StepKind::Upgrade])
        .unwrap();
    assert_eq!(s.steps[1].outcome, StepOutcome::Rejected);
    let x = &e.executions[s.steps[1].execution_id.as_ref().unwrap()];
    assert!(x
        .logs
        .iter()
        .any(|l| l.contains("Program was deployed in this block already")));
    // One next-slot transition after each redeploying step, nothing more.
    let a = exact();
    let slots: Vec<u64> = a
        .report
        .scenario(ScenarioName::UpgradeControl)
        .unwrap()
        .steps
        .iter()
        .map(|r| r.clock_before.as_ref().unwrap().slot)
        .collect();
    assert_eq!(slots, [SLOT, SLOT, SLOT + 1, SLOT + 1, SLOT + 2]);
    for i in [1, 3] {
        assert_eq!(
            step(a, ScenarioName::UpgradeControl, i).derived["fields_changed"],
            serde_json::json!(["slot"])
        );
    }
}

#[test]
fn upgrade_installs_the_oversized_candidate_into_the_resized_programdata() {
    let a = exact();
    let up = step(a, ScenarioName::UpgradeControl, 2);
    assert_eq!(up.outcome, StepOutcome::Verified, "{:?}", up.detail);
    // Its input is the extended world, not S0.
    assert_eq!(
        up.before_state_id,
        step(a, ScenarioName::UpgradeControl, 1).after_state_id
    );
    let pd_before = account(a, &up.before_state_id, PROGRAMDATA);
    let pd_after = account(a, &up.after_state_id, PROGRAMDATA);
    assert_eq!(pd_before.data.len(), 1_146_429);
    assert_eq!(pd_after.data.len(), 1_146_429);
    let new = loader::decode_programdata(&pd_after.data).unwrap();
    assert_eq!(new.bytes, oversized(), "exactly fills the resized capacity");
    assert_eq!(new.deploy_slot, SLOT + 1);
    assert_eq!(up.derived["previous_deploy_slot"], (SLOT).to_string());
    assert_eq!(
        new.upgrade_authority.unwrap().to_string(),
        rollout::upgrade_authority()
    );
    // Spill is the Buffer's lamports; the extension funding stays in ProgramData.
    let buffer = account(a, &up.before_state_id, &rollout::buffer_address());
    assert_eq!(
        up.derived["spill_credit_lamports"],
        buffer.lamports.to_string()
    );
    assert_eq!(pd_after.lamports, pd_before.lamports);
    assert_eq!(
        state(a, &up.after_state_id).accounts[&rollout::buffer_address()],
        Entry::KnownAbsent
    );
    let installed = up.installed_program_after.as_ref().unwrap();
    assert_eq!(installed.identity, "candidate");
    assert_eq!(
        installed.executable_sha256,
        rollout::OVERSIZED_CANDIDATE_SHA256
    );
    assert_eq!(installed.visible_from_slot, SLOT + 2);
}

#[test]
fn installed_oversized_v2_matches_its_overlay() {
    let a = exact();
    let anchor = &a.report.anchors.installed_overlay;
    assert_eq!(anchor.status, AnchorStatus::Matched, "{:?}", anchor.detail);
    assert_eq!(
        anchor.detail["overlay_executable"]["sha256"],
        rollout::OVERSIZED_CANDIDATE_SHA256
    );
    let overlay = a.report.anchors.overlay_execution.as_ref().unwrap();
    assert!(overlay
        .logs
        .iter()
        .any(|l| l.contains("Rollout fixture: DepositSol")));
    assert_eq!(
        a.report.anchors.baseline_world_fidelity.status,
        AnchorStatus::Matched
    );
}

// ---------------------------------------------------------------------------
// Ordering and inherited configuration
// ---------------------------------------------------------------------------

#[test]
fn order_effect_survives_programdata_preparation() {
    let a = exact();
    use StepOutcome::*;
    assert_eq!(outcomes(a, ScenarioName::Control), [Verified]);
    assert_eq!(
        outcomes(a, ScenarioName::UpgradeWithoutPreparation),
        [Unsupported, NotExecuted, NotExecuted]
    );
    assert_eq!(outcomes(a, ScenarioName::PreparationControl), [Verified; 3]);
    assert_eq!(outcomes(a, ScenarioName::UpgradeControl), [Verified; 5]);
    assert_eq!(outcomes(a, ScenarioName::ConfigControl), [Verified; 2]);
    assert_eq!(
        outcomes(a, ScenarioName::OrderA),
        [
            Verified,
            Verified,
            Verified,
            Verified,
            Rejected,
            NotExecuted
        ]
    );
    assert_eq!(outcomes(a, ScenarioName::OrderB), [Verified; 6]);
    let k = step(a, ScenarioName::OrderA, 4);
    assert_eq!(k.reason.as_deref(), Some("config_execution_rejected"));
    let x = &a.evidence.executions[k.execution_id.as_ref().unwrap()];
    assert!(x
        .logs
        .iter()
        .any(|l| l.contains("SolDeposit fee exceeds V2 maximum 1/200")));
    let c = &a.report.comparison;
    assert_eq!(c.status, ComparisonStatus::RolloutOrderEffectObserved);
    assert_eq!(
        c.finding.as_deref(),
        Some("rollout/order/configuration_installation_outcome_differs")
    );
    assert!(c.statement.contains(
        "SetFee(1/100) was accepted before the upgrade and rejected after it, while the configuration installed before the upgrade remained byte-identical"
    ));
    let d = c.first_divergence.as_ref().unwrap();
    assert_eq!(
        (d.step, d.order_a, d.order_b),
        (StepKind::SetFee, Rejected, Verified)
    );
    // Extension and upgrade established identical results in both orders.
    for s in &c.step_contrasts {
        if matches!(s.step, StepKind::ExtendProgram | StepKind::Upgrade) {
            assert!(!s.differs, "{:?}", s.step);
        }
    }
    let b = step(a, ScenarioName::OrderB, 5);
    assert_eq!(
        b.derived["pool_sol_deposit_fee"],
        serde_json::json!({"numerator": "1", "denominator": "100"})
    );
    for (name, value) in [
        ("recipient_account_credit_raw", "753375157"),
        ("manager_fee_account_credit_raw", "7609851"),
        ("mint_supply_delta_raw", "760985008"),
        ("pool_token_supply_delta_raw", "760985008"),
        ("reserve_lamport_delta", "822000000"),
        ("pool_total_lamports_delta", "822000000"),
        ("funding_payer_lamport_debit", "822014000"),
        ("action_transaction_fee_lamports", "14000"),
    ] {
        assert_eq!(metric(b, name), value, "{name}");
        assert!(c.final_action_metrics[name].order_a.value.is_none());
    }
    assert!(c.final_action_metrics["referral_account_credit_raw"]
        .order_b
        .value
        .is_none());
    for word in ["safe", "unsafe", "exploitable", "approved"] {
        assert!(!c.statement.to_lowercase().contains(word), "{word}");
    }
}

#[test]
fn the_inherited_configuration_is_byte_identical_through_extend_and_upgrade() {
    let a = exact();
    let inherited = a
        .report
        .comparison
        .inherited_configuration
        .as_ref()
        .unwrap();
    assert_eq!(inherited["identical_until_final_action"], true);
    let configured = step(a, ScenarioName::OrderB, 0);
    let pool = account(a, &configured.after_state_id, POOL).clone();
    assert_eq!(
        configured.derived["verified_pool_sha256"],
        world::account_id(&pool).unwrap()
    );
    for i in 1..=4 {
        let r = step(a, ScenarioName::OrderB, i);
        assert_eq!(*account(a, &r.after_state_id, POOL), pool, "step {i}");
        if r.execution_id.is_some() {
            assert!(
                !world::changed(state(a, &r.before_state_id), state(a, &r.after_state_id))
                    .contains(POOL)
            );
        }
    }
    assert_eq!(
        *account(a, &step(a, ScenarioName::OrderB, 5).before_state_id, POOL),
        pool
    );
}

#[test]
fn scenarios_restore_s0_and_hand_off_byte_bearing_worlds() {
    let a = exact();
    let s0 = a.report.scenarios[0].initial_state_id.clone();
    for s in &a.report.scenarios {
        assert_eq!(s.initial_state_id, s0);
        let mut previous = s0.clone();
        for r in &s.steps {
            if r.outcome == StepOutcome::NotExecuted || r.before_state_id.is_none() {
                continue;
            }
            assert_eq!(r.before_state_id.as_ref(), Some(&previous), "{:?}", s.name);
            if let Some(id) = &r.after_state_id {
                previous = id.clone();
            }
        }
    }
    for st in a.evidence.states.values() {
        for e in st.accounts.values() {
            if let Entry::Present { account_sha256 } = e {
                assert!(a.evidence.accounts.contains_key(account_sha256));
            }
        }
    }
    assert_eq!(
        a.contract["version"],
        rollout::INPUT_VERSION_V2,
        "a declared preparation is input v2"
    );
}

// ---------------------------------------------------------------------------
// Identity, evidence, verify and reproduce
// ---------------------------------------------------------------------------

#[test]
fn identity_binds_the_declared_preparation_and_not_results() {
    let none = prepared(Preparation::None);
    let exact_p = prepared(extend(REQUIRED));
    let larger = prepared(extend(REQUIRED + 20_480));
    let ids = [
        &none.analysis_input_id,
        &exact_p.analysis_input_id,
        &larger.analysis_input_id,
    ];
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
    for key in [
        "preparation",
        "programdata_preflight",
        "extension_message",
        "extension_payer",
    ] {
        assert!(exact_p.contract.get(key).is_some(), "{key}");
    }
    assert!(exact_p.contract["runtime"]["rent"].is_object());
    assert!(exact_p.contract.get("comparison").is_none());
    // A v1 input is still the Step 16A family.
    let v1 = rollout::Prepared::new(base(fitting())).unwrap();
    assert_eq!(v1.contract["version"], rollout::INPUT_VERSION);
    assert!(v1.contract.get("preparation").is_none());
}

#[test]
fn tampered_preparation_evidence_fails_verification() {
    let a = exact();
    rollout::verify(a).unwrap();
    // Declared extension.
    let mut t = a.clone();
    t.input.preparation = Some(extend(REQUIRED + 1));
    assert!(rollout::verify(&t).is_err());
    // Preflight claim, resealed.
    let mut t = a.clone();
    t.report
        .programdata_preflight
        .as_mut()
        .unwrap()
        .minimum_additional_bytes_required = 1;
    reseal(&mut t);
    assert!(rollout::verify(&t).is_err());
    // Resized ProgramData bytes inside the extension's execution, re-keyed.
    let rekey = |t: &mut rollout::Analysis,
                 scenario: ScenarioName,
                 index: usize,
                 edit: &dyn Fn(&mut eplyx_engine::types::AccountSnapshot),
                 address: &str| {
        let old = step(t, scenario, index).execution_id.clone().unwrap();
        let mut x = t.evidence.executions[&old].clone();
        let Entry::Present { account_sha256 } = x.post[address].clone() else {
            panic!()
        };
        let mut acct = t.evidence.accounts[&account_sha256].clone();
        edit(&mut acct);
        x.post.insert(
            address.into(),
            world::put(&mut t.evidence.accounts, Some(acct)).unwrap(),
        );
        let id = x.id().unwrap();
        t.evidence.executions.insert(id.clone(), x);
        for s in &mut t.report.scenarios {
            for r in &mut s.steps {
                if r.execution_id.as_deref() == Some(old.as_str()) {
                    r.execution_id = Some(id.clone());
                }
            }
        }
        reseal(t);
    };
    let mut t = a.clone();
    rekey(
        &mut t,
        ScenarioName::PreparationControl,
        0,
        &|pd| {
            let last = pd.data.len() - 1;
            pd.data[last] = 1;
        },
        PROGRAMDATA,
    );
    assert!(rollout::verify(&t).is_err());
    // Payer delta.
    let mut t = a.clone();
    rekey(
        &mut t,
        ScenarioName::PreparationControl,
        0,
        &|payer| payer.lamports += 1,
        &preparation::extension_payer(),
    );
    assert!(rollout::verify(&t).is_err());
    // Upgrade input: point the upgrade at S0 instead of the extended world.
    let mut t = a.clone();
    let s0 = t.report.scenarios[0].initial_state_id.clone();
    t.report
        .scenarios
        .iter_mut()
        .find(|s| s.name == ScenarioName::UpgradeControl)
        .unwrap()
        .steps[2]
        .before_state_id = Some(s0);
    reseal(&mut t);
    assert!(rollout::verify(&t).is_err());
    // Scenario chain: drop the extension from order B's record.
    let mut t = a.clone();
    t.report
        .scenarios
        .iter_mut()
        .find(|s| s.name == ScenarioName::OrderB)
        .unwrap()
        .steps
        .remove(1);
    reseal(&mut t);
    assert!(rollout::verify(&t).is_err());
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "eplyx-rollout-16b-{}-{name}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn artifact_round_trips_and_reproduces() {
    let a = exact();
    let base = temp("artifact");
    let dir = base.join("rollout");
    let manifest = artifact::save(a, &dir).unwrap();
    assert_eq!(artifact::load(&dir).unwrap(), *a);
    // The resized ProgramData is retained byte for byte, once.
    let pd = account(
        a,
        &step(a, ScenarioName::PreparationControl, 0).after_state_id,
        PROGRAMDATA,
    );
    let id = world::account_id(pd).unwrap();
    assert!(manifest.objects.contains_key(&format!("accounts/{id}")));
    let distinct: std::collections::BTreeSet<_> = manifest.objects.values().collect();
    assert_eq!(
        distinct.len(),
        std::fs::read_dir(dir.join("objects")).unwrap().count()
    );
    let readable = std::fs::read_to_string(dir.join("report.md")).unwrap();
    assert!(readable.contains("## ProgramData preparation"));
    assert!(readable.contains("Extension-only control"));
    let again = rollout::reproduce(a).unwrap();
    assert_eq!(again, *a);
    let vm = rollout::vm_executions(&a.report);
    assert_eq!(
        vm,
        a.report
            .scenarios
            .iter()
            .flat_map(|s| &s.steps)
            .filter(|r| r.execution_id.is_some())
            .count()
            + 1
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn step_16a_identities_are_unchanged_under_the_reviewed_source() {
    // The Step 16A analysis (commit 0e90d79): input v1, counterexample
    // candidate, SetFee(1/100). Its artifact bound these identities; this
    // build's v1 path must reproduce them exactly.
    let input = base(fitting());
    let source = rollout::SourceIdentity {
        rollout: rollout::STEP_16A_ROLLOUT_SOURCE_SHA256.into(),
        world: rollout::STEP_16A_WORLD_SOURCE_SHA256.into(),
        preparation: rollout::SourceIdentity::current().preparation,
    };
    let a = rollout::analyse_with_source(&input, source).unwrap();
    assert_eq!(
        a.report.analysis_input_id,
        "ffafb0ad8fed57add04807e2b1ce89dae177d4a3d529f2a80dcb64aa0e16d50b"
    );
    assert_eq!(
        a.report.report_sha256,
        "74454af97f1b5df3b781594836166dece5ca1816a20a9cd4f4f714be0e1ec3dd"
    );
    rollout::verify(&a).unwrap();
    // An unreviewed prior source is refused.
    let forged = rollout::SourceIdentity {
        rollout: "00".repeat(32),
        world: rollout::STEP_16A_WORLD_SOURCE_SHA256.into(),
        preparation: rollout::SourceIdentity::current().preparation,
    };
    assert!(rollout::analyse_with_source(&input, forged).is_err());
    // The Step 16A identity cannot label a v2 analysis.
    let v2 = rollout::SourceIdentity {
        rollout: rollout::STEP_16A_ROLLOUT_SOURCE_SHA256.into(),
        world: rollout::STEP_16A_WORLD_SOURCE_SHA256.into(),
        preparation: rollout::SourceIdentity::current().preparation,
    };
    assert!(rollout::analyse_with_source(&input.with_preparation(Preparation::None), v2).is_err());
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn eplyx(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(args)
        .output()
        .unwrap()
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

#[test]
fn cli_preflights_analyses_verifies_and_reproduces_offline() {
    let base = temp("cli");
    let upgrade = base.join("upgrade.json");
    std::fs::write(
        &upgrade,
        ChangeSpec::program_upgrade(&historical().record.program_id, &oversized())
            .to_document()
            .unwrap(),
    )
    .unwrap();
    let plan = base.join("plan.json");
    std::fs::write(
        &plan,
        br#"{"schema_version":1,"programdata_preparation":{"kind":"extend_program","additional_bytes":"65920"}}"#,
    )
    .unwrap();
    let candidate = root().join("fixtures/rollout/fixture_stake_pool_oversized_rollout_v2.so");
    let parameter = root().join("docs/examples/stake-pool-parameter-change.json");
    let bundle = root().join("deploy/bundle");
    let common = |command: &str| -> Vec<String> {
        [
            "rollout",
            command,
            "--upgrade",
            upgrade.to_str().unwrap(),
            "--parameter",
            parameter.to_str().unwrap(),
            "--bundle",
            bundle.to_str().unwrap(),
            "--record-id",
            RECORD,
            "--candidate",
            candidate.to_str().unwrap(),
            "--format",
            "json",
        ]
        .map(String::from)
        .to_vec()
    };
    let run = |args: Vec<String>| {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        eplyx(&refs)
    };
    // Preflight: no plan evaluates `none`; no VM; nothing written.
    let o = run(common("preflight"));
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let f: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(f["vm_executions"], 0);
    assert_eq!(f["programdata_preflight"]["status"], "extension_required");
    assert_eq!(
        f["programdata_preflight"]["minimum_additional_bytes_required"],
        REQUIRED
    );
    let mut args = common("preflight");
    args.extend(["--plan".into(), plan.to_str().unwrap().into()]);
    let f: serde_json::Value = serde_json::from_slice(&run(args).stdout).unwrap();
    assert_eq!(
        f["programdata_preflight"]["status"],
        "declared_extension_sufficient"
    );
    // A plan with an unknown field is a structured refusal.
    let bad = base.join("bad.json");
    std::fs::write(
        &bad,
        br#"{"schema_version":1,"programdata_preparation":{"kind":"none"},"auto":true}"#,
    )
    .unwrap();
    let mut args = common("preflight");
    args.extend(["--plan".into(), bad.to_str().unwrap().into()]);
    let o = run(args);
    assert_eq!(o.status.code(), Some(2));
    let e: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("invalid_preparation_plan"));

    let out = base.join("rollout");
    let mut args = common("analyse");
    args.extend([
        "--plan".into(),
        plan.to_str().unwrap().into(),
        "--out".into(),
        out.to_str().unwrap().into(),
    ]);
    let o = run(args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let receipt: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(receipt["status"], "rollout_order_effect_observed");
    assert_eq!(receipt["preflight_status"], "declared_extension_sufficient");
    assert_eq!(receipt["anchor_extension_only"], "matched");
    assert_eq!(receipt["report_sha256"], exact().report.report_sha256);

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

    // Reproduce from a moved copy with no network at all (Linux namespace);
    // the negative control shows the namespace really has no route out.
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
        let denied = Command::new("unshare")
            .args([
                "-rn",
                "python3",
                "-c",
                "import socket; socket.create_connection(('1.1.1.1', 443), timeout=2)",
            ])
            .output()
            .expect("unshare is required on Linux to qualify network-denied reproduction");
        assert!(
            !denied.status.success(),
            "network is reachable in the namespace"
        );
        Command::new("unshare")
            .args(["-rn", env!("CARGO_BIN_EXE_eplyx")])
            .args(reproduce)
            .output()
            .unwrap()
    } else {
        eplyx(&reproduce)
    };
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let reproduced: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(reproduced["operation"], "reproduced");
    assert_eq!(reproduced["report_sha256"], receipt["report_sha256"]);
    assert_eq!(reproduced["vm_executions"], receipt["vm_executions"]);
    std::fs::remove_dir_all(base).unwrap();
}
