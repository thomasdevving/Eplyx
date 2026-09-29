//! Real candidate + captured token programs; no manually manufactured outcomes.
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    change::ChangeSpec,
    migration::{
        adapter,
        execute::{self, Outcome},
        order::{self, AccountEvidence, ComparisonStatus, FailureKind, OrderError},
        order_store,
        world::World,
    },
};
use migration_common::*;
use serde_json::json;

fn fixture(reserve: u64) -> (ChangeSpec, World, Vec<u8>, [String; 2]) {
    let recipe = recipe(
        serde_json::from_str(include_str!(
            "../../fixtures/migration/order/shared-reserve.recipe.json"
        ))
        .unwrap(),
    );
    let spec = spec(
        &recipe,
        LEGACY,
        LEGACY,
        json!({"destinationFunding":{"kind":"reserveTransfer","reserve":{"kind":"proposed","fundedRaw":reserve.to_string()}}}),
    );
    let w = world(&recipe, &spec);
    let bytes = reference();
    let change = ChangeSpec::token_migration(spec, adapter::REFERENCE_PROGRAM_ID, &bytes).unwrap();
    (
        change,
        w,
        bytes,
        [
            address(&recipe, "alice-source"),
            address(&recipe, "bob-source"),
        ],
    )
}
fn run(f: &(ChangeSpec, World, Vec<u8>, [String; 2])) -> order::Analysis {
    order::analyse(&f.0, &f.1, &f.2, [&f.3[0], &f.3[1]]).unwrap()
}
#[test]
fn shared_reserve_effect_solos_handoff_creation_rollback_and_offline_reproduction() {
    let f = fixture(80);
    let a = run(&f);
    assert_eq!(
        a.comparison.status,
        ComparisonStatus::SharedReserveChangesSuccessfulUnit
    );
    assert_eq!(a.comparison.finding.as_deref(), Some(order::FINDING));
    assert_eq!(a.binding.change_spec_id, f.0.id().unwrap());
    assert_ne!(a.scenarios[2].case_id, a.scenarios[3].case_id);
    for solo in &a.scenarios[..2] {
        assert_eq!(solo.steps[0].execution.outcome, Outcome::Migrated);
    }
    let initial = &a.scenarios[0].initial_state_id;
    assert!(a.scenarios.iter().all(|s| &s.initial_state_id == initial));
    for scenario in &a.scenarios[2..] {
        assert_eq!(scenario.steps[0].execution.outcome, Outcome::Migrated);
        let rejected = &scenario.steps[1];
        assert_eq!(rejected.execution.outcome, Outcome::Rejected);
        assert!(
            rejected
                .execution
                .failure
                .as_ref()
                .unwrap()
                .rollback_verified
        );
        assert_eq!(
            rejected
                .execution
                .failure
                .as_ref()
                .unwrap()
                .error_name
                .as_deref(),
            Some("InsufficientReserve")
        );
        assert_eq!(scenario.steps[0].after_state_id, rejected.before_state_id);
        assert_eq!(
            rejected.observed_reserve_before_raw,
            rejected.observed_reserve_after_raw
        );
        assert!(!rejected.expected_funded);
        let before = &a.states[&rejected.before_state_id];
        let after = &a.states[&rejected.after_state_id];
        for (key, account) in &before.accounts {
            if *key != execute::relayer().to_string() {
                assert_eq!(Some(account), after.accounts.get(key));
            }
        }
        let unit = a
            .binding
            .units
            .iter()
            .find(|u| u.unit_id == scenario.steps[0].execution.unit_id)
            .unwrap();
        assert_eq!(
            a.states[initial].accounts[&unit.destination.address],
            AccountEvidence::KnownAbsent
        );
        assert!(matches!(
            before.accounts[&unit.destination.address],
            AccountEvidence::Present { .. }
        ));
    }
    assert_eq!(a, run(&f), "deterministic states and runs");
    // Directly restore intermediate bytes into a separate session and reproduce B.
    let spec =
        f.0.as_token_migration()
            .unwrap()
            .evaluation_spec(f.0.activation.as_ref())
            .unwrap();
    let candidate =
        f.0.resolve(eplyx_engine::change::CandidateSource::Bytes(&f.2))
            .unwrap();
    let programs =
        execute::programs(&f.1, &spec, adapter::REFERENCE_PROGRAM_ID, &candidate).unwrap();
    let plan = plan(&spec, &f.1, &f.2);
    let mut session = order::restore(
        &a.states[&a.scenarios[2].steps[0].after_state_id],
        &a.binding,
        &programs,
    )
    .unwrap();
    let replay = execute::execute_unit(
        &mut session,
        &spec,
        &plan,
        &a.binding.units[1],
        &execute::relayer().to_string(),
    )
    .unwrap();
    assert_eq!(replay, a.scenarios[2].steps[1].execution);
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().canonicalize().unwrap().join("case");
    let saved = order_store::save(&out, &f.0, &f.1, &f.2, [&f.3[0], &f.3[1]]).unwrap();
    assert_eq!(saved, order_store::reproduce(&out).unwrap());
    assert!(order_store::save(&out, &f.0, &f.1, &f.2, [&f.3[0], &f.3[1]]).is_err());
}
#[test]
fn sufficient_reserve_control() {
    let a = run(&fixture(110));
    assert_eq!(
        a.comparison.status,
        ComparisonStatus::NoSuccessfulUnitEffect
    );
    assert!(a.comparison.finding.is_none());
    for scenario in &a.scenarios {
        for step in &scenario.steps {
            assert_eq!(step.execution.outcome, Outcome::Migrated);
            assert!(step.execution.reconciled);
        }
    }
}
#[test]
fn population_shortfall_is_not_inherited_and_labels_do_not_identify() {
    let mut f = fixture(80);
    let spec =
        f.0.as_token_migration()
            .unwrap()
            .evaluation_spec(None)
            .unwrap();
    assert!(plan(&spec, &f.1, &f.2)
        .units
        .iter()
        .any(|u| u.class == eplyx_engine::migration::planner::ImpactClass::InsufficientReserve));
    let a = run(&f);
    assert!(a
        .binding
        .units
        .iter()
        .all(|u| u.class == eplyx_engine::migration::planner::ImpactClass::Migratable));
    f.0.metadata.label = Some("display only".into());
    f.1.limitations.push("display annotation".into());
    assert_eq!(a, run(&f));
}
fn kind(e: anyhow::Error) -> FailureKind {
    e.downcast_ref::<OrderError>().unwrap().kind
}
#[test]
fn duplicate_foreign_source_candidate_and_unsupported_funding_fail_closed() {
    let (mut change, w, bytes, sources) = fixture(80);
    assert_eq!(
        kind(order::analyse(&change, &w, &bytes, [&sources[0], &sources[0]]).unwrap_err()),
        FailureKind::UnsupportedComposition
    );
    assert_eq!(
        kind(
            order::analyse(
                &change,
                &w,
                &bytes,
                [&sources[0], adapter::REFERENCE_PROGRAM_ID]
            )
            .unwrap_err()
        ),
        FailureKind::EvidenceGap
    );
    let mut wrong = bytes.clone();
    wrong[100] ^= 1;
    assert_eq!(
        kind(order::analyse(&change, &w, &wrong, [&sources[0], &sources[1]]).unwrap_err()),
        FailureKind::EvidenceGap
    );
    let mut terms = change
        .as_token_migration()
        .unwrap()
        .evaluation_spec(None)
        .unwrap();
    terms.destination_funding = eplyx_engine::migration::spec::DestinationFunding::MintTo;
    change = ChangeSpec::token_migration(terms, adapter::REFERENCE_PROGRAM_ID, &bytes).unwrap();
    assert_eq!(
        kind(order::analyse(&change, &w, &bytes, [&sources[0], &sources[1]]).unwrap_err()),
        FailureKind::UnsupportedComposition
    );
}
#[test]
fn fixed_clock_window_and_clock_account_mismatch() {
    let (change, w, bytes, sources) = fixture(80);
    let mut terms = change
        .as_token_migration()
        .unwrap()
        .evaluation_spec(None)
        .unwrap();
    use eplyx_engine::migration::spec::WindowBoundary;
    terms.window.activation = Some(WindowBoundary::Slot {
        value: "1000".into(),
    });
    terms.window.deadline = Some(WindowBoundary::Slot {
        value: "1001".into(),
    });
    let active =
        ChangeSpec::token_migration(terms.clone(), adapter::REFERENCE_PROGRAM_ID, &bytes).unwrap();
    assert!(order::analyse(&active, &w, &bytes, [&sources[0], &sources[1]]).is_ok());
    terms.window.activation = Some(WindowBoundary::Slot {
        value: "1001".into(),
    });
    terms.window.deadline = Some(WindowBoundary::Slot {
        value: "1002".into(),
    });
    let future =
        ChangeSpec::token_migration(terms.clone(), adapter::REFERENCE_PROGRAM_ID, &bytes).unwrap();
    assert_eq!(
        kind(order::analyse(&future, &w, &bytes, [&sources[0], &sources[1]]).unwrap_err()),
        FailureKind::UnsupportedComposition
    );
    terms.window.activation = Some(WindowBoundary::Slot {
        value: "999".into(),
    });
    terms.window.deadline = Some(WindowBoundary::Slot {
        value: "1000".into(),
    });
    let expired =
        ChangeSpec::token_migration(terms, adapter::REFERENCE_PROGRAM_ID, &bytes).unwrap();
    assert_eq!(
        kind(order::analyse(&expired, &w, &bytes, [&sources[0], &sources[1]]).unwrap_err()),
        FailureKind::UnsupportedComposition
    );
    let mut bad = w;
    bad.clock.slot += 1;
    assert_eq!(
        kind(order::analyse(&change, &bad, &bytes, [&sources[0], &sources[1]]).unwrap_err()),
        FailureKind::EvidenceGap
    );
}

#[test]
fn identities_bind_bytes_absence_units_order_clock_world_candidate_and_programs() {
    let f = fixture(80);
    let a = run(&f);
    let id = a.binding.id().unwrap();
    let mut mutations = Vec::new();
    let mut b = a.binding.clone();
    b.clock.slot += 1;
    mutations.push(b);
    let mut b = a.binding.clone();
    b.world_id.replace_range(0..1, "f");
    b.world_content_sha256.push('0');
    mutations.push(b);
    let mut b = a.binding.clone();
    b.candidate.sha256.push('0');
    mutations.push(b);
    let mut b = a.binding.clone();
    b.runtime_id.push('0');
    mutations.push(b);
    let mut b = a.binding.clone();
    b.programs[0].artifact.sha256.push('0');
    mutations.push(b);
    let mut b = a.binding.clone();
    b.units[0].unit_id.push('x');
    mutations.push(b);
    let mut b = a.binding.clone();
    b.change_spec_id.push('0');
    mutations.push(b);
    for b in mutations {
        assert_ne!(id, b.id().unwrap());
    }
    let mut case = a.scenarios[2].case.clone();
    case.ordered_unit_ids.reverse();
    assert_eq!(case.id().unwrap(), a.scenarios[3].case_id);
    let state = &a.states[&a.scenarios[2].steps[0].after_state_id];
    let mut changed = state.clone();
    if let AccountEvidence::Present { account } =
        changed.accounts.get_mut(&a.binding.reserve).unwrap()
    {
        account.data[64] ^= 1;
    }
    assert_ne!(state.id().unwrap(), changed.id().unwrap());
    let mut changed = state.clone();
    changed
        .accounts
        .insert(a.binding.reserve.clone(), AccountEvidence::KnownAbsent);
    assert_ne!(state.id().unwrap(), changed.id().unwrap());
    changed.accounts.remove(&a.binding.units[1].source_account);
    assert_eq!(
        kind(
            order::validate_handoff(
                &changed,
                &[a.binding.units[1].source_account.clone()].into()
            )
            .unwrap_err()
        ),
        FailureKind::HandoffFailure
    );
}

#[test]
fn missing_authority_unknown_absence_and_dependency_fail_closed() {
    let f = fixture(80);
    let a = run(&f);
    let mut w = f.1.clone();
    w.accounts.remove(&a.binding.units[0].owner);
    assert_eq!(
        kind(order::analyse(&f.0, &w, &f.2, [&f.3[0], &f.3[1]]).unwrap_err()),
        FailureKind::EvidenceGap
    );
    let mut w = f.1.clone();
    w.accounts.remove(LEGACY);
    assert_eq!(
        kind(order::analyse(&f.0, &w, &f.2, [&f.3[0], &f.3[1]]).unwrap_err()),
        FailureKind::EvidenceGap
    );
    // Observed worlds cannot infer absence from missing rows, unlike a closed fixture.
    let mut w = f.1.clone();
    use eplyx_engine::migration::world::{WorldKind, WorldOrigin, MAINNET_GENESIS};
    w.kind = WorldKind::ObservedCapture;
    w.cluster = "solana-mainnet".into();
    w.genesis_hash = MAINNET_GENESIS.into();
    w.observed_slots = Some((1000, 1000));
    for entry in w.accounts.values_mut() {
        entry.origin = WorldOrigin::Observed {
            artifact: "bounded-test".into(),
            record: 0,
            pointer: "/value".into(),
            slot: 1000,
        };
    }
    w.inspected_absent = a
        .binding
        .closure
        .iter()
        .filter(|key| {
            !w.accounts.contains_key(*key) && **key != a.binding.units[1].destination.address
        })
        .cloned()
        .collect();
    assert!(w.absence_known(&a.binding.units[0].destination.address));
    assert!(!w.absence_known(&a.binding.units[1].destination.address));
    let error = order::analyse(&f.0, &w, &f.2, [&f.3[0], &f.3[1]]).unwrap_err();
    assert!(
        error.to_string().contains("UnverifiableDestination"),
        "{error:#}"
    );
    assert_eq!(kind(error), FailureKind::EvidenceGap);
    // A declared dependency mismatch is refused before restoring any handoff.
    let spec =
        f.0.as_token_migration()
            .unwrap()
            .evaluation_spec(None)
            .unwrap();
    let resolved =
        f.0.resolve(eplyx_engine::change::CandidateSource::Bytes(&f.2))
            .unwrap();
    let mut programs =
        execute::programs(&f.1, &spec, adapter::REFERENCE_PROGRAM_ID, &resolved).unwrap();
    programs[0].bytes[100] ^= 1;
    assert_eq!(
        kind(
            order::restore(
                &a.states[&a.scenarios[0].initial_state_id],
                &a.binding,
                &programs
            )
            .err()
            .unwrap()
        ),
        FailureKind::EvidenceGap
    );
}

#[test]
fn saved_intermediate_byte_and_existence_tampering_fails_even_with_rehashed_cas() {
    use eplyx_engine::universal::evidence::{EvidenceKind, EvidenceRef, EvidenceStore};
    let f = fixture(80);
    for absence in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().canonicalize().unwrap().join("case");
        let a = order_store::save(&out, &f.0, &f.1, &f.2, [&f.3[0], &f.3[1]]).unwrap();
        let store = EvidenceStore::at(out.join("evidence"));
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("case.json")).unwrap()).unwrap();
        let reference: EvidenceRef = serde_json::from_value(manifest["analysis"].clone()).unwrap();
        let mut analysis: serde_json::Value =
            serde_json::from_slice(&store.get(&reference).unwrap()).unwrap();
        let intermediate = &a.scenarios[2].steps[0].after_state_id;
        let row = &mut analysis["states"][intermediate]["accounts"][&a.binding.reserve];
        if absence {
            *row = json!({"existence":"KnownAbsent"});
        } else {
            let account_ref: EvidenceRef =
                serde_json::from_value(row["account"]["order_account_content"].clone()).unwrap();
            let mut account: eplyx_engine::types::AccountSnapshot =
                serde_json::from_slice(&store.get(&account_ref).unwrap()).unwrap();
            account.data[64] ^= 1;
            row["account"]["order_account_content"] = serde_json::to_value(
                store
                    .put(
                        EvidenceKind::AccountContent,
                        eplyx_engine::canonical::document(&account)
                            .unwrap()
                            .as_bytes(),
                    )
                    .unwrap(),
            )
            .unwrap();
        }
        manifest["analysis"] = serde_json::to_value(
            store
                .put(
                    EvidenceKind::Execution,
                    eplyx_engine::canonical::document(&analysis)
                        .unwrap()
                        .as_bytes(),
                )
                .unwrap(),
        )
        .unwrap();
        std::fs::write(
            out.join("case.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let error = order_store::reproduce(&out).unwrap_err();
        assert!(
            error.to_string().contains("intermediate state identity"),
            "{error:#}"
        );
    }
}

#[test]
fn saved_binding_tampering_cannot_be_repaired_from_live_state() {
    use eplyx_engine::universal::evidence::{EvidenceKind, EvidenceRef, EvidenceStore};
    let f = fixture(80);
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().canonicalize().unwrap().join("case");
    order_store::save(&out, &f.0, &f.1, &f.2, [&f.3[0], &f.3[1]]).unwrap();
    let store = EvidenceStore::at(out.join("evidence"));
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("case.json")).unwrap()).unwrap();
    let reference: EvidenceRef = serde_json::from_value(original["analysis"].clone()).unwrap();
    let analysis: serde_json::Value =
        serde_json::from_slice(&store.get(&reference).unwrap()).unwrap();
    for pointer in [
        "/binding/candidate/sha256",
        "/binding/world_id",
        "/binding/runtime_id",
        "/binding/clock/slot",
        "/binding/programs/0/artifact/sha256",
        "/binding/units/0/unit_id",
        "/binding/change_spec_id",
        "/scenarios/2/case/ordered_unit_ids/0",
    ] {
        let mut changed = analysis.clone();
        *changed.pointer_mut(pointer).unwrap() = json!("0");
        let mut manifest = original.clone();
        let binding: order::PairBinding =
            serde_json::from_value(changed["binding"].clone()).unwrap();
        manifest["binding_id"] = json!(binding.id().unwrap());
        manifest["analysis"] = serde_json::to_value(
            store
                .put(
                    EvidenceKind::Execution,
                    eplyx_engine::canonical::document(&changed)
                        .unwrap()
                        .as_bytes(),
                )
                .unwrap(),
        )
        .unwrap();
        std::fs::write(
            out.join("case.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(order_store::reproduce(&out).is_err(), "{pointer}");
    }
}

#[test]
fn external_signer_declaration_without_authority_evidence_is_not_executable() {
    let (change, w, bytes, sources) = fixture(80);
    let mut terms = change
        .as_token_migration()
        .unwrap()
        .evaluation_spec(None)
        .unwrap();
    // A declared but absent address cannot acquire signer privileges simply
    // because the VM's signature verifier is disabled.
    terms.authorities.migration_authority =
        eplyx_engine::migration::spec::MigrationAuthority::External {
            address: eplyx_engine::migration::fixture::label_address("order-missing", "signer")
                .to_string(),
        };
    let change = ChangeSpec::token_migration(terms, adapter::REFERENCE_PROGRAM_ID, &bytes).unwrap();
    let error = order::analyse(&change, &w, &bytes, [&sources[0], &sources[1]]).unwrap_err();
    assert_eq!(kind(error), FailureKind::EvidenceGap);
}

#[test]
fn solo_economic_mismatch_never_becomes_an_ordering_finding() {
    let (change, w, _, sources) = fixture(80);
    let mut terms = change
        .as_token_migration()
        .unwrap()
        .evaluation_spec(None)
        .unwrap();
    terms.conversion.fee = eplyx_engine::migration::spec::Fee::SourceBps { bps: 25 };
    let bytes = candidate("eplyx_token_migration_defect_fee_ceiling");
    let change = ChangeSpec::token_migration(terms, adapter::REFERENCE_PROGRAM_ID, &bytes).unwrap();
    let a = order::analyse(&change, &w, &bytes, [&sources[0], &sources[1]]).unwrap();
    assert_eq!(a.comparison.status, ComparisonStatus::NotEstablished);
    assert!(a.comparison.finding.is_none());
    for scenario in &a.scenarios {
        assert_eq!(scenario.steps.len(), 1, "mismatch must stop the handoff");
        assert_eq!(
            scenario.steps[0].execution.outcome,
            Outcome::ReconciliationMismatch
        );
        assert_eq!(scenario.stopped, Some(FailureKind::ReconciliationMismatch));
    }
}
