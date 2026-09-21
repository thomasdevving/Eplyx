//! Diagnostic timings for the offline schema-2 product path.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use eplyx_engine::{
    ci::semantic_result,
    protocol::adapter_for,
    replay::hash_bytes,
    types::{AccountSnapshot, NamedAccount},
    universal::{
        evidence::{
            AccountBoundary, AccountObservation, ChunkedAccountObservation, EvidenceKind,
            EvidenceStore,
        },
        execution::{ExecutionRequest, LiteSvmBackend, SlotHashesVariant},
        fidelity::compare_v2,
        model::ReplayObservationV2,
        pipeline,
    },
};
use serde_json::json;

fn hash_inventory(root: &Path) -> Result<(usize, u64)> {
    let mut files = 0;
    let mut bytes = 0;
    fn visit(path: &Path, files: &mut usize, bytes: &mut u64) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                visit(&entry.path(), files, bytes)?;
            } else {
                let content = fs::read(entry.path())?;
                anyhow::ensure!(
                    entry.file_name().to_string_lossy() == hash_bytes(&content),
                    "evidence content hash differs"
                );
                *files += 1;
                *bytes += content.len() as u64;
            }
        }
        Ok(())
    }
    visit(root, &mut files, &mut bytes)?;
    Ok((files, bytes))
}

fn materialize_accounts(record: &ReplayObservationV2, store: &EvidenceStore) -> Result<usize> {
    let mut count = 0;
    for seed in record.account_seeds.iter().chain(&record.runtime.sysvars) {
        match seed.observation.kind {
            EvidenceKind::AccountObservation => {
                AccountObservation::resolve(
                    store,
                    &seed.observation,
                    &seed.address,
                    record.slot,
                    seed.boundary,
                    &record.genesis_hash,
                )?;
            }
            EvidenceKind::ChunkedAccountObservation => {
                anyhow::ensure!(seed.boundary == AccountBoundary::BeforeTransaction);
                ChunkedAccountObservation::resolve(
                    store,
                    &seed.observation,
                    &seed.address,
                    record.slot,
                    &record.genesis_hash,
                )?;
            }
            _ => anyhow::bail!("wrong account evidence category"),
        }
        count += 1;
    }
    Ok(count)
}

fn ms(started: Instant) -> u128 {
    started.elapsed().as_millis()
}

fn main() -> Result<()> {
    let corpus = PathBuf::from(std::env::args().nth(1).context("corpus path required")?);
    let store = EvidenceStore::at(corpus.join("evidence"));
    let started = Instant::now();
    let (objects, hashed_bytes) = hash_inventory(store.root())?;
    let evidence_hash_ms = ms(started);
    let mut rows = Vec::new();
    for path in fs::read_dir(corpus.join("records"))? {
        let path = path?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let record: ReplayObservationV2 = serde_json::from_slice(&fs::read(path)?)?;
        let started = Instant::now();
        let resolved = record.resolve(&store)?;
        let resolution_ms = ms(started);
        let started = Instant::now();
        let account_count = materialize_accounts(&record, &store)?;
        let account_materialization_ms = ms(started);
        let request = ExecutionRequest {
            message: &resolved.message,
            seeds: &resolved.seeds,
            absent_pre_accounts: &resolved.absent_pre_accounts,
            watched: &resolved.watched,
            runtime_sysvars: &resolved.runtime_sysvars,
            clock: None,
            programs_to_load: &[],
            signature_check: record.runtime.signature_check,
            blockhash_check: record.runtime.blockhash_check,
            unlimited_logs: true,
            slot_hashes: SlotHashesVariant::BackendDefault,
            require_complete_state: true,
        };
        let (execution, backend) = LiteSvmBackend.execute_timed(&request)?;
        let started = Instant::now();
        let fidelity = compare_v2(&record.expected, &resolved.expected_accounts, &execution);
        let fidelity_ms = ms(started);
        anyhow::ensure!(fidelity.matched(), "historical fidelity differs");
        let started = Instant::now();
        pipeline::baseline(&record, &resolved)?;
        let full_materiality_profile_ms = ms(started);
        let started = Instant::now();
        let mut subjects = 0;
        if let Some(adapter) = adapter_for(&record.program_id) {
            let labels = resolved
                .message
                .account_keys
                .iter()
                .enumerate()
                .map(|(index, address)| {
                    (
                        address.clone(),
                        adapter.label(&resolved.message.transaction, index),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let pre = resolved
                .watched
                .iter()
                .map(|address| NamedAccount {
                    label: labels[address].clone(),
                    address: address.clone(),
                    account: resolved
                        .seeds
                        .get(address)
                        .cloned()
                        .unwrap_or(AccountSnapshot {
                            owner: "11111111111111111111111111111111".into(),
                            lamports: 0,
                            data: Vec::new(),
                            executable: false,
                            rent_epoch: 0,
                        }),
                })
                .collect::<Vec<_>>();
            let adapted = semantic_result(&execution, &labels, &resolved.message.account_keys)?;
            subjects = adapter
                .evaluable_subjects(&resolved.message.transaction, &pre)
                .len();
            anyhow::ensure!(
                adapter
                    .named_findings(&resolved.message.transaction, &pre, &adapted, &adapted)
                    .is_empty(),
                "baseline semantic self-findings"
            );
        }
        let semantic_ms = ms(started);
        rows.push(json!({"record_id":record.id,"account_count":account_count,
            "resolution_ms":resolution_ms,"account_materialization_ms":account_materialization_ms,
            "runtime_setup_ms":backend.runtime_setup.as_millis(),
            "explicit_program_registration_ms":backend.explicit_program_registration.as_millis(),
            "account_seeding_ms":backend.account_seeding.as_millis(),
            "transaction_execution_ms":backend.transaction_execution.as_millis(),
            "evidence_collection_ms":backend.evidence_collection.as_millis(),
            "fidelity_ms":fidelity_ms,"semantic_interpretation_ms":semantic_ms,
            "full_materiality_profile_ms":full_materiality_profile_ms,"semantic_subjects":subjects}));
    }
    rows.sort_by(|a, b| a["record_id"].as_str().cmp(&b["record_id"].as_str()));
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "evidence_hash_validation_ms":evidence_hash_ms,
            "evidence_objects":objects,"evidence_bytes":hashed_bytes,
            "notes":"Timings are diagnostic and overlap. Seeded ProgramData is lazily loaded by LiteSVM during transaction execution; explicit registration is zero for native V0.",
            "records":rows,
        }))?
    );
    Ok(())
}
