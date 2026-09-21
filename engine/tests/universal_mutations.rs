use std::{fs, path::PathBuf};

use eplyx_engine::{
    evidence::token::TokenProgram,
    replay::ReplayRecord,
    universal::{
        evidence::{AccountBoundary, AccountObservation, EvidenceKind, EvidenceStore},
        fidelity::compare_v2,
        model::{
            ExecutionInput, FidelityProfile, InstructionRole, ReplayObservationV2,
            RuntimeCapability,
        },
        pipeline,
    },
};

fn fixture() -> (ReplayObservationV2, EvidenceStore) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/examples/phase-u4-kamino");
    let record =
        serde_json::from_slice(
            &fs::read(root.join(
                "records/e56202c8fdcd443d1deba455ab04f2bc6f16689f83a7c107bb98765ec2da626f.json",
            ))
            .expect("T1 record bytes"),
        )
        .expect("V2 record");
    (record, EvidenceStore::at(root.join("evidence")))
}

fn reidentify(record: &mut ReplayObservationV2) {
    record.id = record.identity().unwrap();
}

fn must_reject(name: &str, result: anyhow::Result<impl Sized>) {
    assert!(result.is_err(), "mutation {name} was accepted");
}

#[test]
fn historical_identity_and_evidence_mutations_fail_closed() {
    let (record, store) = fixture();
    let resolved = record
        .resolve(&store)
        .expect("control observation resolves");
    let (mut execution, fidelity) =
        pipeline::baseline(&record, &resolved).expect("control historical fidelity");
    assert!(fidelity.matched(), "control must match before mutation");

    // 1: A serialized loaded-key claim is never authority over the LUT proof.
    let mut forged = record.clone();
    let ExecutionInput::V0 { claimed_proof, .. } = &mut forged.execution else {
        panic!("native v0 required")
    };
    claimed_proof.full_account_keys.last_mut().unwrap().address =
        "11111111111111111111111111111111".into();
    reidentify(&mut forged);
    must_reject("forged_v0_loaded_address", forged.resolve(&store));

    // 2, 16, 17: Temporal boundary, address and current-state substitution.
    let seed = record
        .account_seeds
        .iter()
        .find(|s| s.observation.kind == EvidenceKind::AccountObservation)
        .expect("regular account observation");
    must_reject(
        "wrong_historical_boundary",
        AccountObservation::resolve(
            &store,
            &seed.observation,
            &seed.address,
            record.slot,
            AccountBoundary::EndOfExecutionSlot,
            &record.genesis_hash,
        ),
    );
    must_reject(
        "same_bytes_wrong_address",
        AccountObservation::resolve(
            &store,
            &seed.observation,
            "11111111111111111111111111111111",
            record.slot,
            AccountBoundary::BeforeTransaction,
            &record.genesis_hash,
        ),
    );
    must_reject(
        "current_state_instead_of_historical",
        AccountObservation::resolve(
            &store,
            &seed.observation,
            &seed.address,
            record.slot + 1,
            AccountBoundary::BeforeTransaction,
            &record.genesis_hash,
        ),
    );

    // 3: ProgramData is bound to its actual deployment identity.
    let mut deployment = record.clone();
    let binary = deployment
        .binaries
        .iter_mut()
        .find(|b| b.program_id == record.program_id)
        .unwrap();
    binary.deployment_slot = binary.deployment_slot.map(|slot| slot + 1);
    reidentify(&mut deployment);
    must_reject("wrong_programdata_deployment", deployment.resolve(&store));

    // 4 and 15: Neither a wrong hash nor a missing CAS path can be ignored.
    let mut absent_object = record.clone();
    absent_object.account_seeds[0].observation.sha256 = "0".repeat(64);
    reidentify(&mut absent_object);
    must_reject(
        "record_references_missing_cas_object",
        absent_object.resolve(&store),
    );
    let scratch =
        std::env::temp_dir().join(format!("eplyx-u4-cas-mutation-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let isolated = EvidenceStore::at(&scratch);
    let original = store.get(&seed.observation).unwrap();
    let path = isolated.path(&seed.observation).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut changed = original;
    changed[0] ^= 1;
    fs::write(path, changed).unwrap();
    must_reject(
        "evidence_content_hash_ignored",
        isolated.get(&seed.observation),
    );
    fs::remove_dir_all(&scratch).unwrap();

    // 5: Runtime treatment is part of the observation's stable identity.
    let mut runtime = record.clone();
    runtime.runtime.provenance.push_str("/forged");
    must_reject(
        "runtime_context_removed_from_identity",
        runtime.validate_identity(),
    );

    // 6 and 12: V2 refuses legacy-flattened v0 and the weaker V1 profile.
    let mut flattened = record.clone();
    flattened.execution = ExecutionInput::LegacyV1Compatibility {
        message: solana_message::Message::default(),
        transaction: resolved.message.transaction.clone(),
    };
    reidentify(&mut flattened);
    must_reject("v0_flattened_to_legacy", flattened.validate_identity());
    let mut wrong_profile = record.clone();
    wrong_profile.fidelity_profile = FidelityProfile::HistoricalReplayV1;
    reidentify(&mut wrong_profile);
    must_reject(
        "v2_forced_through_v1_fidelity",
        wrong_profile.validate_identity(),
    );
    let old = include_str!("../../docs/examples/mainnet-stake-pool-record.json");
    assert!(
        serde_json::from_str::<ReplayRecord>(old).is_ok(),
        "V1 record must remain readable"
    );
    assert!(
        serde_json::from_str::<ReplayObservationV2>(old).is_err(),
        "V1 Stake Pool evidence cannot be silently upgraded to the V2 profile"
    );

    // 7 and 8: Dependencies remain mandatory execution, never a second target.
    let mut dropped = record.clone();
    dropped
        .dependencies
        .programs
        .retain(|p| p.program_id != "HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ");
    reidentify(&mut dropped);
    must_reject("scope_dependency_dropped", dropped.resolve(&store));
    let mut promoted = record.clone();
    promoted
        .instruction_roles
        .iter_mut()
        .find(|r| r.role == InstructionRole::ExecutionDependency)
        .expect("execution dependency role")
        .role = InstructionRole::SemanticTarget;
    reidentify(&mut promoted);
    must_reject(
        "dependency_promoted_to_second_semantic_target",
        promoted.resolve(&store),
    );

    // 9: A changed historical expectation halts the pipeline before adapters.
    let mut bad_history = record.clone();
    bad_history
        .expected
        .logs
        .push("forged historical log".into());
    reidentify(&mut bad_history);
    must_reject(
        "semantic_adapter_before_fidelity",
        pipeline::baseline(&bad_history, &resolved),
    );

    // 10 and 11: Raw bytes are compared even when account metadata is equal.
    let address = execution
        .post_accounts
        .iter()
        .find(|(_, account)| account.as_ref().is_some_and(|a| a.data.len() > 4))
        .unwrap()
        .0
        .clone();
    execution
        .post_accounts
        .get_mut(&address)
        .unwrap()
        .as_mut()
        .unwrap()
        .data[0] ^= 1;
    let first = compare_v2(&record.expected, &resolved.expected_accounts, &execution);
    assert!(
        first
            .failures
            .contains(&format!("account:{address}:raw_data")),
        "post_state_raw_mismatch_ignored"
    );
    let account = execution
        .post_accounts
        .get_mut(&address)
        .unwrap()
        .as_mut()
        .unwrap();
    account.data[0] ^= 1;
    let last = account.data.len() - 1;
    account.data[last] ^= 1;
    let second = compare_v2(&record.expected, &resolved.expected_accounts, &execution);
    assert!(
        second
            .failures
            .contains(&format!("account:{address}:raw_data")),
        "typed_equality_must_not_hide_raw_tail_mismatch"
    );

    // 13: Actual Token-2022 extension state cannot be decoded as legacy SPL.
    let token2022 = resolved
        .seeds
        .values()
        .find(|a| TokenProgram::of(&a.owner) == Some(TokenProgram::Token2022) && a.data.len() > 165)
        .unwrap();
    assert!(
        TokenProgram::SplToken
            .account_amount(&token2022.data)
            .is_none()
            && TokenProgram::Token2022
                .account_amount(&token2022.data)
                .is_some(),
        "token_2022_account_decoded_as_legacy_token"
    );
}

#[test]
fn backend_capability_is_not_a_protocol_verdict() {
    let (record, _) = fixture();
    assert_eq!(
        record.runtime.capability(),
        RuntimeCapability::SupportedByCurrentBackend
    );
    let mut unknown_feature = record.runtime.clone();
    unknown_feature.feature_profile = "future validator feature set".into();
    assert!(
        matches!(
            unknown_feature.capability(),
            RuntimeCapability::UnsupportedRuntimeFeature(_)
        ),
        "backend limitation cannot be reported as protocol unsupported"
    );
    let mut missing = record.runtime.clone();
    missing.provenance.clear();
    assert!(
        matches!(
            missing.capability(),
            RuntimeCapability::InsufficientRuntimeEvidence(_)
        ),
        "missing runtime evidence needs its own status"
    );
    let mut unproved_hashes = record.runtime.clone();
    unproved_hashes.slot_hashes_policy = "historical".into();
    assert!(
        matches!(
            unproved_hashes.capability(),
            RuntimeCapability::InsufficientRuntimeEvidence(_)
        ),
        "historical SlotHashes policy requires historical bytes"
    );
}

#[test]
fn generic_replay_core_has_no_protocol_identity_branches() {
    for source in [
        include_str!("../src/universal/model.rs"),
        include_str!("../src/universal/execution.rs"),
        include_str!("../src/universal/evidence.rs"),
        include_str!("../src/universal/resolver.rs"),
        include_str!("../src/universal/fidelity.rs"),
        include_str!("../src/universal/pipeline.rs"),
        include_str!("../src/universal/bundle.rs"),
    ] {
        assert!(
            !source.contains("KLend2g3")
                && !source.contains("kamino::")
                && !source.contains("stake_pool::"),
            "executor_branch_on_kamino_or_stake_pool_id"
        );
    }
}
