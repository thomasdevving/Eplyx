use std::{
    fs,
    path::{Path, PathBuf},
};

use eplyx_engine::{
    bundle::AdapterMetadata,
    ci,
    corpus_store::CorpusStore,
    protocol::{orca::OrcaSwapV2Adapter, ProtocolAdapter},
    replay::hash_bytes,
    semantic_binding::{SemanticBinding, SemanticBindingReport},
    universal::{
        bundle::{BundleManifestV2, UniversalBundle},
        evidence::EvidenceStore,
        pipeline,
    },
};

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u11-2-checkpointed-corpus")
}

fn bundle() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u12-1-semantic-binding-bundle")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn rehash_metadata(root: &Path, metadata: &AdapterMetadata) -> String {
    let bytes = serde_json::to_vec_pretty(metadata).unwrap();
    fs::write(root.join("adapters/metadata.json"), &bytes).unwrap();
    let mut manifest: BundleManifestV2 =
        serde_json::from_slice(&fs::read(root.join("bundle.json")).unwrap()).unwrap();
    let previous = manifest.bundle_sha256.clone();
    manifest.adapter_metadata_sha256 = hash_bytes(&bytes);
    manifest.bundle_sha256.clear();
    manifest.bundle_sha256 = hash_bytes(&serde_json::to_vec(&manifest).unwrap());
    fs::write(
        root.join("bundle.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    assert_ne!(
        manifest.bundle_sha256, previous,
        "provenance changes bundle identity"
    );
    manifest.bundle_sha256
}

#[test]
fn frozen_binding_is_corroborated_by_historical_state_and_execution() {
    let root = corpus();
    let record = CorpusStore::open(&root)
        .unwrap()
        .load_v2()
        .unwrap()
        .pop()
        .unwrap();
    let input = record
        .resolve(&EvidenceStore::at(root.join("evidence")))
        .unwrap();
    let (baseline, _) = pipeline::baseline(&record, &input).unwrap();
    let adapter = OrcaSwapV2Adapter;
    let binding = adapter
        .semantic_binding(
            &input.message.transaction,
            &input.seeds,
            &input.baseline_elf,
            &baseline,
        )
        .unwrap();
    assert_eq!(binding.level(), "execution_corroborated_external_interface");
    assert!(!binding.exact_source_to_elf_verified());
    binding
        .validate(&record.program_id, &input.baseline_elf, &baseline)
        .unwrap();

    let mut wrong_pool = input.seeds.clone();
    let pool = wrong_pool
        .get_mut(&input.message.transaction.instructions[4].accounts[4].address)
        .unwrap();
    pool.data[133] ^= 1; // Vault A address in the pinned repository layout.
    assert!(adapter
        .semantic_binding(
            &input.message.transaction,
            &wrong_pool,
            &input.baseline_elf,
            &baseline
        )
        .is_err());

    let mut wrong_owner = input.seeds.clone();
    let vault_b = &input.message.transaction.instructions[4].accounts[10].address;
    wrong_owner.get_mut(vault_b).unwrap().owner = "11111111111111111111111111111111".into();
    assert!(adapter
        .semantic_binding(
            &input.message.transaction,
            &wrong_owner,
            &input.baseline_elf,
            &baseline
        )
        .is_err());

    let mut wrong_mint = input.seeds.clone();
    wrong_mint.get_mut(vault_b).unwrap().data[0] ^= 1;
    assert!(adapter
        .semantic_binding(
            &input.message.transaction,
            &wrong_mint,
            &input.baseline_elf,
            &baseline
        )
        .is_err());
    let mut wrong_authority = input.seeds.clone();
    wrong_authority.get_mut(vault_b).unwrap().data[32] ^= 1;
    assert!(adapter
        .semantic_binding(
            &input.message.transaction,
            &wrong_authority,
            &input.baseline_elf,
            &baseline
        )
        .is_err());

    let mut wrong_flow = baseline.clone();
    let user_b = &input.message.transaction.instructions[4].accounts[9].address;
    wrong_flow
        .post_accounts
        .get_mut(user_b)
        .unwrap()
        .as_mut()
        .unwrap()
        .data[64..72]
        .copy_from_slice(&input.seeds[user_b].data[64..72]);
    assert!(adapter
        .semantic_binding(
            &input.message.transaction,
            &input.seeds,
            &input.baseline_elf,
            &wrong_flow
        )
        .is_err());

    let mut wrong_shape = input.message.transaction.clone();
    wrong_shape.instructions[4].data[0] ^= 1;
    assert_eq!(
        adapter
            .semantic_binding(&wrong_shape, &input.seeds, &input.baseline_elf, &baseline)
            .unwrap()
            .level(),
        "manual_or_unknown"
    );
    let mut wrong_signer = input.message.transaction.clone();
    wrong_signer.instructions[4].accounts[3].is_signer = false;
    assert_eq!(
        adapter
            .semantic_binding(&wrong_signer, &input.seeds, &input.baseline_elf, &baseline)
            .unwrap()
            .level(),
        "manual_or_unknown"
    );
    let mut failed_execution = baseline.clone();
    failed_execution.success = false;
    assert!(adapter
        .semantic_binding(
            &input.message.transaction,
            &input.seeds,
            &input.baseline_elf,
            &failed_execution
        )
        .is_err());

    let mut opposite = input.message.transaction.clone();
    opposite.instructions[4].data[41] = 1;
    opposite.instructions[3].accounts.clear();
    opposite.instructions[5].accounts.clear();
    let lower = adapter
        .semantic_binding(&opposite, &input.seeds, &input.baseline_elf, &baseline)
        .unwrap();
    assert_eq!(lower.level(), "repository_source_claim");
    let shown = serde_json::to_string(&SemanticBindingReport::new(vec![
        eplyx_engine::semantic_binding::ObservationSemanticBinding {
            observation_id: record.id,
            binding: lower,
        },
    ]))
    .unwrap();
    assert!(shown.contains("repository_source_claim"));
    assert!(shown.contains("\"exact_source_to_elf_verified\":false"));
}

#[test]
fn exact_build_claim_requires_a_verifier() {
    let root = corpus();
    let record = CorpusStore::open(&root)
        .unwrap()
        .load_v2()
        .unwrap()
        .pop()
        .unwrap();
    let input = record
        .resolve(&EvidenceStore::at(root.join("evidence")))
        .unwrap();
    let (baseline, _) = pipeline::baseline(&record, &input).unwrap();
    let adapter = OrcaSwapV2Adapter;
    let source = match adapter
        .semantic_binding(
            &input.message.transaction,
            &input.seeds,
            &input.baseline_elf,
            &baseline,
        )
        .unwrap()
    {
        SemanticBinding::ExecutionCorroboratedExternalInterface { source, .. } => source,
        other => panic!("unexpected binding: {other:?}"),
    };
    let false_exact = SemanticBinding::ExactVerifiedBuild {
        source,
        build_evidence_sha256: hash_bytes(&input.baseline_elf),
    };
    assert!(false_exact
        .validate(&record.program_id, &input.baseline_elf, &baseline)
        .is_err());
}

#[test]
fn metadata_mutation_changes_identity_and_cannot_retain_assurance() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("bundle");
    copy_tree(&bundle(), &root);
    let old = UniversalBundle::open(&root).unwrap();
    let mut metadata = old.adapter.clone();
    let binding = &mut metadata.semantic_bindings.as_mut().unwrap()[0].binding;
    let source = match binding {
        SemanticBinding::ExecutionCorroboratedExternalInterface { source, .. } => source.clone(),
        other => panic!("unexpected binding: {other:?}"),
    };
    *binding = SemanticBinding::RepositorySourceClaim { source };
    rehash_metadata(&root, &metadata);
    assert!(
        UniversalBundle::open(&root).is_err(),
        "a rehashed weaker claim cannot pass reconstruction"
    );

    metadata.semantic_bindings = None;
    rehash_metadata(&root, &metadata);
    let unclassified = UniversalBundle::open(&root).unwrap();
    assert!(unclassified.adapter.semantic_bindings.is_none());
    let report = ci::check(&root, &root.join("binaries/current.so"), None).unwrap();
    assert!(
        report.semantic_binding.is_none(),
        "removing provenance removes the assurance claim"
    );
}

#[test]
fn new_and_legacy_reports_keep_three_distinct_answers() {
    let root = bundle();
    let report = ci::check(&root, &root.join("binaries/current.so"), None).unwrap();
    assert_eq!(report.replay_proof.as_ref().unwrap().status, "matched");
    assert_eq!(report.coverage.len(), 4);
    let binding = report.semantic_binding.as_ref().unwrap();
    assert_eq!(
        binding.observations[0].binding.level(),
        "execution_corroborated_external_interface"
    );
    assert!(!binding.exact_source_to_elf_verified);
    assert_eq!(report.exit_code(), 0);
    let source = match &binding.observations[0].binding {
        SemanticBinding::ExecutionCorroboratedExternalInterface { source, .. } => source.clone(),
        other => panic!("unexpected binding: {other:?}"),
    };
    let mut repository_only_report = report.clone();
    repository_only_report.semantic_binding = Some(SemanticBindingReport::new(vec![
        eplyx_engine::semantic_binding::ObservationSemanticBinding {
            observation_id: binding.observations[0].observation_id.clone(),
            binding: SemanticBinding::RepositorySourceClaim { source },
        },
    ]));
    let json = serde_json::to_value(&repository_only_report).unwrap();
    assert_eq!(
        json["semantic_binding"]["observations"][0]["binding"]["level"],
        "repository_source_claim"
    );
    assert_eq!(
        json["semantic_binding"]["exact_source_to_elf_verified"],
        false
    );
    assert_eq!(json["replay_proof"]["status"], "matched");
    assert_eq!(json["coverage"].as_array().unwrap().len(), 4);

    let legacy = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u12-orca-semantic-bundle");
    let old = ci::check(&legacy, &legacy.join("binaries/current.so"), None).unwrap();
    assert_eq!(old.coverage.len(), 4);
    assert!(old.semantic_binding.is_none());
    let replay_only = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u11-2-checkpointed-bundle");
    let none = ci::check(&replay_only, &replay_only.join("binaries/current.so"), None).unwrap();
    assert!(none.semantic_binding.is_none());
    assert_eq!(none.bundle.adapter, "none");
    assert_eq!(none.exit_code(), 2);
}
