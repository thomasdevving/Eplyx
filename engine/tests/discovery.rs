use anyhow::{anyhow, Result};
use eplyx_engine::{
    discovery::{self, *},
    ingest::{self, rpc::RpcProvider, transactions::HistoricalTransaction, IngestManifest},
    replay::ReplayStateSource,
    types::{AccountMetaSpec, InstructionSpec},
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

const PROGRAM: &str = "8opHzTAnfzRpPEx21XtnrVTX28YQuCpAjcn1PczScKh";
const OTHER: &str = "11111111111111111111111111111111";

fn meta(byte: u8) -> Vec<AccountMetaSpec> {
    vec![
        AccountMetaSpec {
            address: format!("account-{byte}-a"),
            is_signer: true,
            is_writable: true,
        },
        AccountMetaSpec {
            address: format!("account-{byte}-b"),
            is_signer: false,
            is_writable: false,
        },
    ]
}

fn tx(index: u64, shape: u8, cpi: bool, success: bool) -> HistoricalTransaction {
    let target = InstructionSpec {
        program: PROGRAM.into(),
        accounts: meta(shape),
        data: vec![shape, 2, 3],
    };
    let wrapper = InstructionSpec {
        program: OTHER.into(),
        accounts: meta(shape),
        data: vec![9],
    };
    HistoricalTransaction {
        signature: format!("signature-{index:04}"),
        slot: 100 + index,
        block_time: Some(1_700_000_000 + index as i64),
        version: "legacy".into(),
        recent_blockhash: "blockhash".into(),
        payer: format!("account-{shape}-a"),
        account_keys: meta(shape),
        loaded_address_count: 0,
        instructions: vec![if cpi { wrapper } else { target.clone() }],
        inner_instructions: if cpi { vec![target] } else { vec![] },
        inner_instruction_frames: vec![],
        success,
        error: (!success).then(|| json!({"InstructionError":[0,"Custom"]})),
        fee: 5_000,
        compute_units: Some(10_000 + index * 1_000),
        pre_balances: None,
        post_balances: None,
        pre_token_balances: None,
        post_token_balances: None,
        native_value_lamports: Some(index * 1_000_000),
        logs: vec![format!("log-{shape}")],
    }
}

fn manifest(transactions: Vec<HistoricalTransaction>) -> IngestManifest {
    IngestManifest {
        schema_version: 1,
        program_id: PROGRAM.into(),
        genesis_hash: "mainnet-genesis".into(),
        start_slot: 100,
        end_slot: 200,
        transactions,
    }
}

fn build(transactions: Vec<HistoricalTransaction>, max: u64) -> DiscoveryCorpus {
    discovery::build(
        &manifest(transactions),
        SelectionPolicy {
            max_records: max,
            ..Default::default()
        },
        Some("endpoint-hash".into()),
        1,
        3,
        |_| Some(ReplayStateSource::CurrentApproximation),
    )
    .unwrap()
}

#[test]
fn instruction_fingerprints_are_deterministic_and_structural() {
    let instruction = tx(1, 7, false, true).instructions.remove(0);
    let first = instruction_fingerprint(&instruction);
    assert_eq!(first, instruction_fingerprint(&instruction));
    let mut changed = instruction.clone();
    changed.accounts[1].is_writable = true;
    assert_ne!(first.id, instruction_fingerprint(&changed).id);
    assert_eq!(first.data_prefix_hex, "070203");
}

#[test]
fn direct_cpi_and_unknown_are_distinguished() {
    assert_eq!(
        interaction_type(&tx(1, 1, false, true), PROGRAM),
        InteractionType::DirectInteraction
    );
    assert_eq!(
        interaction_type(&tx(1, 1, true, true), PROGRAM),
        InteractionType::CpiInteraction
    );
    assert_eq!(
        interaction_type(&tx(1, 1, false, true), OTHER),
        InteractionType::Unknown
    );
}

#[test]
fn clustering_is_deterministic_and_groups_matching_shapes() {
    let corpus = build(vec![tx(2, 1, false, true), tx(1, 1, false, true)], 10);
    assert_eq!(corpus.clusters.len(), 1);
    assert_eq!(corpus.clusters[0].occurrences, 2);
    let reversed = build(vec![tx(1, 1, false, true), tx(2, 1, false, true)], 10);
    assert_eq!(corpus.clusters, reversed.clusters);
}

#[test]
fn rare_shapes_receive_rarity_reasons_and_higher_scores() {
    let mut transactions: Vec<_> = (1..=6).map(|i| tx(i, 1, false, true)).collect();
    transactions.push(tx(7, 2, false, true));
    let corpus = build(transactions, 7);
    let rare = corpus
        .selected
        .iter()
        .find(|item| item.interaction.instruction_fingerprints[0].data_prefix_hex == "020203")
        .unwrap();
    assert!(rare
        .selection_reasons
        .iter()
        .any(|reason| reason.starts_with("rare_")));
    assert_eq!(rare.score.rarity, 3000);
}

#[test]
fn high_compute_and_native_movement_are_ranked_by_distribution() {
    let corpus = build((1..=10).map(|i| tx(i, 1, false, true)).collect(), 10);
    let highest = corpus
        .selected
        .iter()
        .find(|item| item.interaction.signature == "signature-0010")
        .unwrap();
    assert_eq!(highest.score.compute, 2000);
    assert_eq!(highest.score.native_value, 1000);
    assert!(highest
        .selection_reasons
        .iter()
        .any(|reason| reason.starts_with("high_compute_")));
}

#[test]
fn historical_failures_are_prioritized_but_not_called_bugs() {
    let corpus = build(
        vec![
            tx(1, 1, false, true),
            tx(2, 1, false, true),
            tx(3, 1, false, false),
        ],
        3,
    );
    let failed = corpus
        .selected
        .iter()
        .find(|item| !item.interaction.success)
        .unwrap();
    assert_eq!(failed.score.failure, 1800);
    assert!(failed
        .selection_reasons
        .contains(&"historical_transaction_failed".into()));
}

#[test]
fn deduplication_caps_near_identical_buckets() {
    let corpus = build((1..=30).map(|i| tx(i, 1, false, true)).collect(), 30);
    // Distribution/temporal bands retain diversity, but no identical bucket
    // contributes an unbounded run.
    let mut buckets = std::collections::BTreeMap::new();
    for item in &corpus.selected {
        let key = (
            item.interaction.cluster_id.clone(),
            item.score.compute / 400,
            item.score.native_value / 200,
            item.interaction.slot / 10,
        );
        *buckets.entry(key).or_insert(0) += 1;
    }
    assert!(corpus.selected.len() < 30);
}

#[test]
fn stratified_selection_keeps_rare_failure_cpi_and_high_compute_examples() {
    let mut transactions: Vec<_> = (1..=15).map(|i| tx(i, 1, false, true)).collect();
    transactions.push(tx(16, 2, false, false));
    transactions.push(tx(17, 3, true, true));
    let corpus = build(transactions, 6);
    assert!(corpus.selected.iter().any(|item| !item.interaction.success));
    assert!(corpus
        .selected
        .iter()
        .any(|item| item.interaction.interaction_type == InteractionType::CpiInteraction));
    assert!(corpus
        .selected
        .iter()
        .any(|item| item.score.compute >= 1800));
}

#[test]
fn repeated_selection_is_byte_identical() {
    let transactions: Vec<_> = (1..=12)
        .map(|i| tx(i, (i % 3) as u8, i % 5 == 0, i % 4 != 0))
        .collect();
    let a = build(transactions.clone(), 8);
    let b = build(transactions, 8);
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
}

#[test]
fn replay_eligibility_preserves_phase_four_boundaries() {
    let direct = tx(1, 1, false, true);
    assert_eq!(
        replay_eligibility(
            &direct,
            PROGRAM,
            Some(&ReplayStateSource::ControlledSnapshot)
        ),
        ReplayEligibility::HistoricalStateReady
    );
    assert_eq!(
        replay_eligibility(&direct, PROGRAM, Some(&ReplayStateSource::Reconstructed)),
        ReplayEligibility::ReconstructedReady
    );
    assert_eq!(
        replay_eligibility(
            &direct,
            PROGRAM,
            Some(&ReplayStateSource::CurrentApproximation)
        ),
        ReplayEligibility::ApproximateOnly
    );
    assert_eq!(
        replay_eligibility(&direct, PROGRAM, None),
        ReplayEligibility::MissingState
    );
    assert_eq!(
        replay_eligibility(&tx(2, 1, true, true), PROGRAM, None),
        ReplayEligibility::UnsupportedCpi
    );
    // A v0 message that resolved no lookup tables carries exactly the static
    // key list and replays like a legacy one, so it stays eligible.
    let mut v0_without_lookups = direct.clone();
    v0_without_lookups.version = "v0".into();
    assert_eq!(
        replay_eligibility(
            &v0_without_lookups,
            PROGRAM,
            Some(&ReplayStateSource::ControlledSnapshot)
        ),
        ReplayEligibility::HistoricalStateReady
    );
    // One that resolved a table does not: the addresses are normalized for
    // inspection, but the lookup itself is never executed.
    let mut v0_with_lookups = direct;
    v0_with_lookups.version = "v0".into();
    v0_with_lookups.loaded_address_count = 2;
    assert_eq!(
        replay_eligibility(&v0_with_lookups, PROGRAM, None),
        ReplayEligibility::UnsupportedTransaction
    );
}

#[test]
fn selected_records_preserve_source_provenance() {
    let corpus = build(vec![tx(1, 1, false, true)], 1);
    let item = &corpus.selected[0].interaction;
    assert_eq!(item.signature, item.provenance.source_signature);
    assert_eq!(item.slot, item.provenance.source_slot);
    assert_eq!(item.provenance.genesis_hash, "mainnet-genesis");
    assert_eq!(item.provenance.program_id, PROGRAM);
    assert_eq!(
        item.provenance.state_source,
        Some(ReplayStateSource::CurrentApproximation)
    );
    assert!(item.provenance.account_observation_context_slots.is_empty());
}

#[test]
fn discovery_corpus_and_manifest_round_trip() {
    let corpus = build(vec![tx(1, 1, false, true), tx(2, 2, true, true)], 2);
    let bytes = serde_json::to_vec_pretty(&corpus).unwrap();
    let restored: DiscoveryCorpus = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored, corpus);
    discovery::validate(&restored).unwrap();
    assert!(String::from_utf8(bytes)
        .unwrap()
        .contains("Discovery corpus only"));
}

#[test]
fn observations_are_not_silently_equated_with_unique_entities() {
    let corpus = build(vec![tx(1, 1, false, true), tx(2, 1, false, true)], 2);
    assert_eq!(corpus.statistics.observation_count, 2);
    assert_eq!(corpus.statistics.unique_economic_entities, None);
    let mut interactions: Vec<_> = corpus
        .selected
        .iter()
        .map(|item| item.interaction.clone())
        .collect();
    for item in &mut interactions {
        item.metadata.economic_entity_id = Some("position-a".into());
    }
    assert_eq!(economic_entity_statistics(&interactions), (2, Some(1)));
}

struct FlakyRpc {
    calls: AtomicU32,
    fail_until: u32,
}

impl RpcProvider for FlakyRpc {
    fn call(&self, _method: &str, _params: Value) -> Result<Value> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if call <= self.fail_until {
            Err(anyhow!("temporary provider failure"))
        } else {
            Ok(json!("ok"))
        }
    }
}

#[test]
fn provider_retries_are_bounded_and_eventually_succeed() {
    let provider = FlakyRpc {
        calls: AtomicU32::new(0),
        fail_until: 2,
    };
    let retry = ingest::rpc::RetryingRpc {
        provider: &provider,
        max_retries: 3,
        base_backoff: Duration::ZERO,
    };
    assert_eq!(retry.call("method", json!([])).unwrap(), json!("ok"));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 3);

    let provider = FlakyRpc {
        calls: AtomicU32::new(0),
        fail_until: 10,
    };
    let retry = ingest::rpc::RetryingRpc {
        provider: &provider,
        max_retries: 2,
        base_backoff: Duration::ZERO,
    };
    assert!(retry.call("method", json!([])).is_err());
    assert_eq!(provider.calls.load(Ordering::Relaxed), 3);
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "discovery-tests-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("thread")
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn current_state_provider_never_promotes_samples_to_historical_state() {
    let temp = Temp::new();
    let corpus = build(vec![tx(1, 1, false, true)], 1);
    let item = &corpus.selected[0].interaction;
    let path = temp
        .0
        .join("accounts")
        .join(format!("{}.json", item.signature));
    ingest::write_json(
        &path,
        &json!({"schema_version":1,"state_source":"current_approximation","samples":[{"response":{"value":[{"lamports":1},null]}}]}),
    )
    .unwrap();
    let provider = CurrentApproximationProvider {
        cache_root: temp.0.clone(),
    };
    let state = provider.load_pre_state(item).unwrap();
    assert_eq!(
        state.state_source,
        Some(ReplayStateSource::CurrentApproximation)
    );
    assert_eq!((state.accounts_observed, state.accounts_missing), (1, 1));
    assert!(state.pre_state_hash.is_none());
}

#[test]
fn coverage_reports_factual_cluster_and_failure_counts() {
    let corpus = build(
        vec![
            tx(1, 1, false, true),
            tx(2, 1, false, true),
            tx(3, 2, false, false),
        ],
        3,
    );
    assert_eq!(corpus.coverage.clusters_represented, 2);
    assert_eq!(corpus.coverage.clusters_total, 2);
    assert_eq!(corpus.coverage.historical_failures_discovered, 1);
    assert_eq!(corpus.coverage.historical_failures_selected, 1);
}
