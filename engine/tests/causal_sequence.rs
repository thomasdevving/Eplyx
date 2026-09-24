use eplyx_engine::{
    corpus_store::CorpusStore,
    replay::hash_bytes,
    types::AccountSnapshot,
    universal::{
        checkpoint::ObservedCheckpointV1,
        evidence::{AccountBoundary, AccountObservation, EvidenceKind, EvidenceRef, EvidenceStore},
        execution::{
            ExecutionBackend, ExecutionEvidence, ExecutionRequest, LiteSvmBackend,
            SlotHashesVariant,
        },
        model::{ExecutionInput, ReplayObservationV2},
        pipeline,
        sequence::{verify_reconstruction, HistoricalSequenceClosureProofV1},
    },
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/examples/phase-u13-3-sequence-corpus")
}
fn link_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            link_tree(&e.path(), &dest)
        } else {
            fs::hard_link(e.path(), dest).unwrap();
        }
    }
}
fn fixture() -> (
    ReplayObservationV2,
    HistoricalSequenceClosureProofV1,
    tempfile::TempDir,
    EvidenceStore,
) {
    let record = CorpusStore::open(root())
        .unwrap()
        .load_v2()
        .unwrap()
        .pop()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    link_tree(&root().join("evidence"), &temp.path().join("evidence"));
    let store = EvidenceStore::at(temp.path().join("evidence"));
    let proof = serde_json::from_slice(
        &store
            .get(
                &record
                    .checkpointed_execution
                    .as_ref()
                    .unwrap()
                    .closure_proof,
            )
            .unwrap(),
    )
    .unwrap();
    (record, proof, temp, store)
}
fn reject(
    mut record: ReplayObservationV2,
    proof: HistoricalSequenceClosureProofV1,
    store: &EvidenceStore,
    expected: &str,
) {
    record
        .checkpointed_execution
        .as_mut()
        .unwrap()
        .closure_proof = proof.store(store).unwrap();
    record.id = record.identity().unwrap();
    let error = record
        .resolve(store)
        .err()
        .expect("accepted mutation")
        .to_string();
    assert!(
        error.contains(expected),
        "expected {expected:?}, got {error}"
    );
}
fn omit(index: usize) {
    let (r, mut p, _t, s) = fixture();
    p.entries.retain(|e| e.transaction_index != index);
    reject(r, p, &s, "sequence membership or canonical order");
}
#[test]
fn omit_tx428() {
    omit(428)
}
#[test]
fn omit_tx431() {
    omit(431)
}
#[test]
fn omit_tx438() {
    omit(438)
}
#[test]
fn omit_tx1245() {
    omit(1245)
}
#[test]
fn reorder_428_and_431_despite_commuting_behavior() {
    let (r, mut p, _t, s) = fixture();
    p.entries.swap(1, 2);
    reject(r, p, &s, "sequence membership or canonical order");
}
#[test]
fn alter_dependency_edge_with_rebuilt_hashes() {
    let (r, mut p, _t, s) = fixture();
    p.dependency_edges[0].from += 1;
    reject(r, p, &s, "sequence dependency edges");
}
#[test]
fn alter_tx428_expected_validator_envelope() {
    let (r, mut p, _t, s) = fixture();
    p.entries[1].expected.logs.push("forged".into());
    reject(r, p, &s, "historical logs differ");
}
#[test]
fn alter_tx428_instruction_with_all_message_hashes_rebuilt() {
    let (r, mut p, _t, s) = fixture();
    let entry = &mut p.entries[1];
    let mut raw: Value = serde_json::from_slice(
        &s.get(entry.execution.validator_transaction_ref().unwrap())
            .unwrap(),
    )
    .unwrap();
    let data = raw["transaction"]["message"]["instructions"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()
        .get_mut("data")
        .unwrap();
    let mut bytes = bs58::decode(data.as_str().unwrap()).into_vec().unwrap();
    bytes[0] ^= 1;
    *data = bs58::encode(bytes).into_string().into();
    let mut block: Value = serde_json::from_slice(&s.get(&p.full_block).unwrap()).unwrap();
    let mut row = raw.clone();
    for key in ["slot", "transactionIndex", "blockTime"] {
        row.as_object_mut().unwrap().remove(key);
    }
    block["result"]["transactions"][428] = row;
    p.full_block = s
        .put(
            EvidenceKind::Validator,
            &serde_json::to_vec(&block).unwrap(),
        )
        .unwrap();
    let ExecutionInput::V0 { lookup_tables, .. } = &entry.execution else {
        panic!()
    };
    let tables = lookup_tables
        .iter()
        .map(|reference| {
            let observation: AccountObservation =
                serde_json::from_slice(&s.get(reference).unwrap()).unwrap();
            let (_, wire) = AccountObservation::resolve_with_raw(
                &s,
                reference,
                &observation.address,
                r.slot,
                AccountBoundary::EndOfExecutionSlot,
                &r.genesis_hash,
            )
            .unwrap();
            eplyx_engine::message::HistoricalAccountEvidence::from_response(
                &observation.address,
                r.slot,
                observation.provider,
                &wire,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let frozen = eplyx_engine::message::FrozenV0::from_rpc(&raw, &r.genesis_hash).unwrap();
    let proven = eplyx_engine::message::reconstruct(&frozen, &tables, None).unwrap();
    entry.execution = ExecutionInput::V0 {
        message: frozen.native_message().clone(),
        frozen_transaction: s
            .put(
                EvidenceKind::Transaction,
                &serde_json::to_vec(&raw).unwrap(),
            )
            .unwrap(),
        lookup_tables: lookup_tables.clone(),
        slot_hashes: None,
        claimed_proof: proven.proof().clone(),
    };
    reject(
        r,
        p,
        &s,
        "sequence transaction 428 validator envelope differs",
    );
}
fn changed_observation(
    r: &ReplayObservationV2,
    s: &EvidenceStore,
    reference: &EvidenceRef,
    boundary: AccountBoundary,
    mutate: impl FnOnce(&mut Value),
) -> EvidenceRef {
    let o: AccountObservation = serde_json::from_slice(&s.get(reference).unwrap()).unwrap();
    let (_, raw) = AccountObservation::resolve_with_raw(
        s,
        reference,
        &o.address,
        r.slot,
        boundary,
        &r.genesis_hash,
    )
    .unwrap();
    let mut raw: Value = serde_json::from_slice(&raw).unwrap();
    mutate(&mut raw);
    AccountObservation::capture(
        s,
        &o.address,
        r.slot,
        boundary,
        o.provider,
        &serde_json::to_vec(&raw).unwrap(),
    )
    .unwrap()
}
fn flip_data(raw: &mut Value) {
    use base64::Engine;
    let engine = base64::prelude::BASE64_STANDARD;
    let mut bytes = engine
        .decode(raw["result"]["value"]["data"][0].as_str().unwrap())
        .unwrap();
    bytes[0] ^= 1;
    raw["result"]["value"]["data"][0] = engine.encode(bytes).into();
}
#[test]
fn alter_later_lut() {
    use base64::{prelude::BASE64_STANDARD, Engine};
    let (r, mut p, _t, s) = fixture();
    let ExecutionInput::V0 {
        message,
        lookup_tables,
        ..
    } = &mut p.entries[1].execution
    else {
        panic!()
    };
    let descriptor = &message.address_table_lookups[0];
    let used = *descriptor
        .writable_indexes
        .first()
        .or_else(|| descriptor.readonly_indexes.first())
        .unwrap() as usize;
    lookup_tables[0] = changed_observation(
        &r,
        &s,
        &lookup_tables[0],
        AccountBoundary::EndOfExecutionSlot,
        |raw| {
            let mut bytes = BASE64_STANDARD
                .decode(raw["result"]["value"]["data"][0].as_str().unwrap())
                .unwrap();
            bytes[56 + used * 32] ^= 1;
            raw["result"]["value"]["data"][0] = BASE64_STANDARD.encode(bytes).into();
        },
    );
    reject(r, p, &s, "v0 LUT proof stage 4");
}

#[test]
fn alter_later_only_parent_account() {
    let (mut r, mut p, _t, s) = fixture();
    let key = "3e5QUcAj1qWHRjtphaKVguitkZx6Rnun6CSCJibnwxZM";
    let mut checkpoint: ObservedCheckpointV1 =
        serde_json::from_slice(&s.get(&p.start_checkpoint).unwrap()).unwrap();
    let account = checkpoint
        .accounts
        .iter_mut()
        .find(|a| a.address == key)
        .unwrap();
    account.observation = changed_observation(
        &r,
        &s,
        &account.observation,
        AccountBoundary::BeforeTransaction,
        flip_data,
    );
    r.account_seeds
        .iter_mut()
        .find(|a| a.address == key)
        .unwrap()
        .observation = account.observation.clone();
    p.start_checkpoint = checkpoint.store(&s).unwrap();
    r.checkpointed_execution.as_mut().unwrap().start_checkpoint = p.start_checkpoint.clone();
    reject(r, p, &s, "intermediate frontier differs");
}
#[test]
fn alter_stored_target_post_with_all_commitments_rebuilt() {
    let (mut r, mut p, _t, s) = fixture();
    let w = &mut r.expected.watched_accounts[2];
    let mut account: AccountSnapshot =
        serde_json::from_slice(&s.get(w.expected_post_content.as_ref().unwrap()).unwrap()).unwrap();
    account.data[0] ^= 1;
    w.expected_post_content = Some(
        s.put(
            EvidenceKind::AccountContent,
            &serde_json::to_vec(&account).unwrap(),
        )
        .unwrap(),
    );
    let mut execution: ExecutionEvidence =
        serde_json::from_slice(&s.get(&p.target_execution).unwrap()).unwrap();
    execution
        .post_accounts
        .insert(w.address.clone(), Some(account));
    p.target_execution = s
        .put(
            EvidenceKind::Execution,
            &serde_json::to_vec(&execution).unwrap(),
        )
        .unwrap();
    r.checkpointed_execution
        .as_mut()
        .unwrap()
        .deterministic_execution = p.target_execution.clone();
    reject(r, p, &s, "derived target execution commitment differs");
}
#[test]
fn alter_one_terminal_account() {
    let (mut r, mut p, _t, s) = fixture();
    let mut c: ObservedCheckpointV1 =
        serde_json::from_slice(&s.get(&p.terminal_checkpoint).unwrap()).unwrap();
    c.accounts[0].observation = changed_observation(
        &r,
        &s,
        &c.accounts[0].observation,
        AccountBoundary::EndOfExecutionSlot,
        |raw| raw["result"]["value"]["rentEpoch"] = 1.into(),
    );
    p.terminal_checkpoint = c.store(&s).unwrap();
    r.checkpointed_execution
        .as_mut()
        .unwrap()
        .terminal_checkpoint = p.terminal_checkpoint.clone();
    reject(r, p, &s, "terminal reconciliation differs");
}
#[test]
fn omit_terminal_frontier_account() {
    let (mut r, mut p, _t, s) = fixture();
    let mut c: ObservedCheckpointV1 =
        serde_json::from_slice(&s.get(&p.terminal_checkpoint).unwrap()).unwrap();
    c.accounts.pop();
    p.terminal_checkpoint = c.store(&s).unwrap();
    r.checkpointed_execution
        .as_mut()
        .unwrap()
        .terminal_checkpoint = p.terminal_checkpoint.clone();
    reject(r, p, &s, "terminal checkpoint incomplete");
}
#[test]
fn substitute_wrong_historical_elf() {
    let (mut r, mut p, _t, s) = fixture();
    let mut elf = s.get(&r.binaries[0].elf).unwrap();
    elf[100] ^= 1;
    r.binaries[0].elf = s.put(EvidenceKind::ProgramBinary, &elf).unwrap();
    r.dependencies
        .programs
        .iter_mut()
        .find(|d| d.program_id == r.program_id)
        .unwrap()
        .binary_sha256 = Some(r.binaries[0].elf.sha256.clone());
    p.program_binaries = r.binaries.clone();
    reject(
        r,
        p,
        &s,
        "ProgramData deployment or complete ELF bytes differ",
    );
}
#[test]
fn substitute_wrong_historical_feature_set_evidence() {
    let (mut r, mut p, _t, s) = fixture();
    let h = r.runtime.historical_evidence.as_mut().unwrap();
    h.historical_feature_set = None;
    h.feature_profile = "LiteSVM 0.16.0 mainnet".into();
    h.evidence_id = h.identity().unwrap();
    r.runtime.feature_profile = h.feature_profile.clone();
    p.runtime_evidence = s
        .put(EvidenceKind::Runtime, &serde_json::to_vec(h).unwrap())
        .unwrap();
    r.runtime.historical_evidence_ref = Some(p.runtime_evidence.clone());
    reject(
        r,
        p,
        &s,
        "sequence requires historical feature-set evidence",
    );
}
#[test]
fn target_identity_points_to_suffix() {
    let (r, mut p, _t, s) = fixture();
    p.target_transaction_index = 428;
    p.target_signature = p.entries[1].signature.clone();
    reject(r, p, &s, "sequence identity");
}
#[test]
fn candidate_substitution_in_suffix_is_rejected() {
    let (mut r, p, _t, s) = fixture();
    let mut wire = serde_json::to_value(p).unwrap();
    wire["entries"][1]["candidate_elf"] = serde_json::to_value(&r.binaries[0].elf).unwrap();
    r.checkpointed_execution.as_mut().unwrap().closure_proof = s
        .put(
            EvidenceKind::ClosureProof,
            &serde_json::to_vec(&wire).unwrap(),
        )
        .unwrap();
    r.id = r.identity().unwrap();
    assert!(r
        .resolve(&s)
        .err()
        .unwrap()
        .to_string()
        .contains("unknown field `candidate_elf`"));
}
#[test]
fn reset_spot_market_between_transactions() {
    let (r, mut p, _t, s) = fixture();
    let resolved = r.resolve(&s).unwrap();
    let messages = p
        .entries
        .iter()
        .map(|e| e.execution.resolve(&s, &r.genesis_hash).unwrap())
        .collect::<Vec<_>>();
    let start: ObservedCheckpointV1 =
        serde_json::from_slice(&s.get(&p.start_checkpoint).unwrap()).unwrap();
    let watched = start
        .accounts
        .iter()
        .map(|a| a.address.clone())
        .collect::<Vec<_>>();
    let mut seeds = resolved.seeds.clone();
    let key = "6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3";
    let parent = seeds[key].clone();
    let mut runs = Vec::new();
    for (i, message) in messages.iter().enumerate() {
        if i == 1 {
            seeds.insert(key.into(), parent.clone());
        }
        let run = LiteSvmBackend
            .execute(&ExecutionRequest {
                message,
                seeds: &seeds,
                absent_pre_accounts: &[],
                watched: &watched,
                runtime_sysvars: &resolved.runtime_sysvars,
                clock: None,
                programs_to_load: &[],
                runtime_profile: &resolved.runtime_profile,
                unlimited_logs: true,
                slot_hashes: SlotHashesVariant::BackendDefault,
                recent_blockhashes: Default::default(),
                require_complete_state: true,
            })
            .unwrap();
        seeds.extend(
            run.post_accounts
                .iter()
                .filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v))),
        );
        runs.push(run);
    }
    // Rebuild every intermediate commitment around the bad reset. Independent
    // terminal evidence must still reject the execution, even if its cache is
    // internally consistent and all historical envelopes match.
    for (entry, run) in p.entries.iter_mut().zip(&runs) {
        entry.frontier_digest =
            eplyx_engine::universal::sequence::frontier_digest(&run.post_accounts).unwrap();
    }
    assert!(verify_reconstruction(&r, &s, &p, &runs)
        .unwrap_err()
        .to_string()
        .contains("terminal reconciliation differs"));
}
#[test]
fn candidate_elf_is_substituted_only_into_target_execution() {
    use std::cell::RefCell;
    struct Spy {
        calls: RefCell<Vec<(String, String)>>,
        target: String,
        pd: String,
    }
    impl ExecutionBackend for Spy {
        fn execute(&self, request: &ExecutionRequest<'_>) -> anyhow::Result<ExecutionEvidence> {
            assert_eq!(request.message.transaction.signature, self.target);
            self.calls.borrow_mut().push((
                request.message.transaction.signature.clone(),
                hash_bytes(&request.seeds[&self.pd].data[45..]),
            ));
            LiteSvmBackend.execute(request)
        }
    }
    let (r, _p, _t, s) = fixture();
    let resolved = r.resolve(&s).unwrap();
    let mut candidate = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/dependencies/TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA.so"),
    )
    .unwrap();
    assert_ne!(candidate, resolved.baseline_elf);
    candidate.resize(resolved.baseline_elf.len(), 0);
    let spy = Spy {
        calls: RefCell::new(vec![]),
        target: r.signature.clone(),
        pd: r.binaries[0].programdata_address.clone().unwrap(),
    };
    let candidate_result = pipeline::execute_with_backend(&r, &resolved, &candidate, &spy).unwrap();
    assert!(!candidate_result.success);
    let calls = spy.calls.borrow();
    assert_eq!(calls.len(), 3);
    assert!(calls
        .iter()
        .all(|(_, hash)| hash == &hash_bytes(&candidate)));
    assert_eq!(resolved.baseline_elf, s.get(&r.binaries[0].elf).unwrap());
}
#[test]
fn ordinary_offline_bundle_reports_matched_contract_three_without_semantics() {
    let bundle = eplyx_engine::universal::bundle::UniversalBundle::open(
        root().with_file_name("phase-u13-3-sequence-bundle"),
    )
    .unwrap();
    assert_eq!(
        bundle.records[0]
            .checkpointed_execution
            .as_ref()
            .unwrap()
            .proof_contract_version,
        3
    );
    let (_, f) = pipeline::baseline(&bundle.records[0], &bundle.resolved[0]).unwrap();
    assert!(f.matched());
    assert_eq!(bundle.adapter.name, "none");
    assert_eq!(bundle.adapter.version, 0);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["ci", "check", "--bundle"])
        .arg(bundle.root())
        .arg("--candidate")
        .arg(bundle.baseline_path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["replay_proof"]["status"], "matched");
    assert_eq!(
        report["replay_proof"]["proof_contract_versions"],
        serde_json::json!([3])
    );
    assert_eq!(
        report["replay_proof"]["profile"],
        "checkpointed_execution_v1"
    );
    assert_eq!(
        report["summary"]["failure_reasons"],
        serde_json::json!(["no_semantic_coverage"])
    );
    assert!(report["replay_proof"]["boundary_proof"]
        .as_str()
        .unwrap()
        .contains("derived_target_boundary"));
}

// Controlled, protocol-neutral standard-program sequence. A transfer at the
// target credits X, a later transfer credits X again, and a third credits Y.
fn synthetic() -> (
    ReplayObservationV2,
    HistoricalSequenceClosureProofV1,
    tempfile::TempDir,
    EvidenceStore,
    String,
    String,
) {
    use base64::{prelude::BASE64_STANDARD, Engine};
    use eplyx_engine::{
        dependencies::{DependencyManifest, ProgramDependency, ProgramSource},
        message::{self, ArchiveProvenance, FrozenV0},
        universal::{
            checkpoint::CheckpointAccount,
            model::*,
            sequence::{self, SequenceEntryV1},
        },
        versions::{ProgramLoader, LEGACY_BPF_LOADER_ID},
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    let (mut r, _p, t, s) = fixture();
    let provider: ArchiveProvenance = serde_json::from_slice::<AccountObservation>(
        &s.get(&r.runtime.sysvars[0].observation).unwrap(),
    )
    .unwrap()
    .provider;
    let mut sysvars = BTreeMap::new();
    for seed in &r.runtime.sysvars {
        sysvars.insert(
            seed.address.clone(),
            AccountObservation::resolve(
                &s,
                &seed.observation,
                &seed.address,
                r.slot,
                seed.boundary,
                &r.genesis_hash,
            )
            .unwrap()
            .unwrap(),
        );
    }
    let runtime = eplyx_engine::universal::execution::RuntimeProfile::resolve(
        r.runtime.historical_evidence.as_ref(),
        &sysvars,
        &r.runtime.feature_profile,
        false,
        false,
        &r.runtime.instructions_rule,
        &r.runtime.slot_hashes_policy,
    )
    .unwrap();
    let key = |byte| solana_address::Address::new_from_array([byte; 32]).to_string();
    let payer = key(41);
    let source = key(42);
    let x = key(43);
    let y = key(44);
    let program = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_owned();
    let elf = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/dependencies/TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA.so"),
    )
    .unwrap();
    let mut seeds = BTreeMap::new();
    seeds.insert(
        payer.clone(),
        AccountSnapshot {
            lamports: 100_000_000,
            owner: "11111111111111111111111111111111".into(),
            executable: false,
            rent_epoch: 0,
            data: vec![],
        },
    );
    for (address, amount) in [(&source, 1000u64), (&x, 10), (&y, 0)] {
        let mut data = vec![0u8; 165];
        data[..32].fill(45);
        data[32..64].fill(41);
        data[64..72].copy_from_slice(&amount.to_le_bytes());
        data[108] = 1;
        seeds.insert(
            address.clone(),
            AccountSnapshot {
                lamports: 10_000_000,
                owner: program.clone(),
                executable: false,
                rent_epoch: 0,
                data,
            },
        );
    }
    seeds.insert(
        program.clone(),
        AccountSnapshot {
            lamports: 100_000_000_000,
            owner: LEGACY_BPF_LOADER_ID.into(),
            executable: true,
            rent_epoch: 0,
            data: elf.clone(),
        },
    );
    let capture = |address: &str, account: &AccountSnapshot, boundary: AccountBoundary| {
        let slot = if boundary == AccountBoundary::BeforeTransaction {
            r.slot - 1
        } else {
            r.slot
        };
        let raw = json!({"jsonrpc":"2.0","id":1,"result":{"context":{"slot":slot},"value":{"lamports":account.lamports,"owner":account.owner,"executable":account.executable,"rentEpoch":account.rent_epoch,"data":[BASE64_STANDARD.encode(&account.data),"base64"],"space":account.data.len()}}});
        AccountObservation::capture(
            &s,
            address,
            r.slot,
            boundary,
            provider.clone(),
            &serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap()
    };
    let refs = seeds
        .iter()
        .map(|(k, v)| (k.clone(), capture(k, v, AccountBoundary::BeforeTransaction)))
        .collect::<BTreeMap<_, _>>();
    let start = ObservedCheckpointV1 {
        schema_version: 1,
        slot: r.slot - 1,
        accounts: refs
            .iter()
            .map(|(address, observation)| CheckpointAccount {
                address: address.clone(),
                observation: observation.clone(),
            })
            .collect(),
    }
    .store(&s)
    .unwrap();
    let execution = |raw: &Value| {
        if raw["version"] == "legacy" {
            return ExecutionInput::capture_legacy_v2(&s, raw).unwrap();
        }
        let frozen = FrozenV0::from_rpc(raw, &r.genesis_hash).unwrap();
        let proven = message::reconstruct(&frozen, &[], None).unwrap();
        ExecutionInput::V0 {
            message: frozen.native_message().clone(),
            frozen_transaction: s
                .put(EvidenceKind::Transaction, &serde_json::to_vec(raw).unwrap())
                .unwrap(),
            lookup_tables: vec![],
            slot_hashes: None,
            claimed_proof: proven.proof().clone(),
        }
    };
    let mut raws = Vec::new();
    let mut messages = Vec::new();
    for (i, (destination, amount)) in [(&x, 7u64), (&x, 11), (&y, 3)].iter().enumerate() {
        let mut data = vec![3];
        data.extend(amount.to_le_bytes());
        let mut raw = json!({"slot":r.slot,"transactionIndex":i,"blockTime":0,"version":0,"transaction":{"signatures":[bs58::encode([i as u8+1;64]).into_string()],"message":{"header":{"numRequiredSignatures":1,"numReadonlySignedAccounts":0,"numReadonlyUnsignedAccounts":1},"accountKeys":[payer,source,destination,program],"recentBlockhash":solana_hash::Hash::new_from_array([61;32]).to_string(),"addressTableLookups":[],"instructions":[{"programIdIndex":3,"accounts":[1,2,0],"data":bs58::encode(data).into_string()}]}},"meta":{"err":null,"fee":5000,"loadedAddresses":{"writable":[],"readonly":[]},"innerInstructions":[],"logMessages":[],"preBalances":[100000000,10000000,10000000,100000000000u64],"postBalances":[99995000,10000000,10000000,100000000000u64]}});
        if i == 1 {
            raw["version"] = json!("legacy");
            raw["transaction"]["message"]
                .as_object_mut()
                .unwrap()
                .remove("addressTableLookups");
        }
        messages.push(execution(&raw).resolve(&s, &r.genesis_hash).unwrap());
        raws.push(raw);
    }
    let frontier = seeds.keys().cloned().collect::<Vec<_>>();
    let runs = LiteSvmBackend
        .execute_sequence(
            &ExecutionRequest {
                message: &messages[0],
                seeds: &seeds,
                absent_pre_accounts: &[],
                watched: &frontier,
                runtime_sysvars: &sysvars,
                clock: None,
                programs_to_load: &[],
                runtime_profile: &runtime,
                unlimited_logs: true,
                slot_hashes: SlotHashesVariant::BackendDefault,
                recent_blockhashes: Default::default(),
                require_complete_state: true,
            },
            &messages,
        )
        .unwrap();
    assert!(runs.iter().all(|run| run.success), "{runs:?}");
    let mut entries = Vec::new();
    for (i, (raw, run)) in raws.iter_mut().zip(&runs).enumerate() {
        raw["meta"]["logMessages"] = json!(run.logs);
        raw["meta"]["computeUnitsConsumed"] = json!(run.compute_units);
        raw["meta"]["fee"] = json!(run.fee);
        raw["meta"]["preBalances"] = json!(messages[i]
            .account_keys
            .iter()
            .map(|k| if i == 0 {
                seeds[k].lamports
            } else {
                runs[i - 1].post_accounts[k].as_ref().unwrap().lamports
            })
            .collect::<Vec<_>>());
        raw["meta"]["postBalances"] = json!(messages[i]
            .account_keys
            .iter()
            .map(|k| run.post_accounts[k].as_ref().unwrap().lamports)
            .collect::<Vec<_>>());
        entries.push(SequenceEntryV1 {
            transaction_index: i,
            signature: messages[i].transaction.signature.clone(),
            execution: execution(raw),
            full_account_keys: messages[i].account_keys.clone(),
            expected: ExpectedHistoricalOutcome {
                success: true,
                error: None,
                fee: run.fee,
                compute_units: Some(run.compute_units),
                logs: run.logs.clone(),
                inner_instructions: vec![],
                return_data: None,
                watched_accounts: vec![],
            },
            runtime_profile_id: runtime.profile_id.clone(),
            frontier_digest: sequence::frontier_digest(&run.post_accounts).unwrap(),
        });
    }
    let block = json!({"result":{"parentSlot":r.slot-1,"blockTime":0,"blockhash":solana_hash::Hash::new_from_array([62;32]).to_string(),"transactions":raws.iter().map(|raw|{let mut raw=raw.clone();for k in ["slot","transactionIndex","blockTime"]{raw.as_object_mut().unwrap().remove(k);}raw}).collect::<Vec<_>>()}});
    let outputs = vec![payer.clone(), source.clone(), x.clone()];
    let plan = sequence::plan(&block, 0, &outputs, &BTreeMap::new()).unwrap();
    assert_eq!(plan.indexes, [0, 1, 2]);
    let terminal = ObservedCheckpointV1 {
        schema_version: 1,
        slot: r.slot,
        accounts: plan
            .terminal
            .iter()
            .map(|k| CheckpointAccount {
                address: k.clone(),
                observation: capture(
                    k,
                    runs[2].post_accounts[k].as_ref().unwrap(),
                    AccountBoundary::EndOfExecutionSlot,
                ),
            })
            .collect(),
    }
    .store(&s)
    .unwrap();
    let elf_ref = s.put(EvidenceKind::ProgramBinary, &elf).unwrap();
    let binary = ProgramBinaryEvidence {
        program_id: program.clone(),
        loader: LEGACY_BPF_LOADER_ID.into(),
        programdata_address: None,
        deployment_slot: None,
        upgrade_authority: None,
        elf: elf_ref.clone(),
        executable_account: refs[&program].clone(),
        programdata_account: None,
    };
    let mut target = runs[0].clone();
    target.post_accounts.retain(|k, _| outputs.contains(k));
    let target_ref = s
        .put(
            EvidenceKind::Execution,
            &serde_json::to_vec(&target).unwrap(),
        )
        .unwrap();
    let proof = HistoricalSequenceClosureProofV1 {
        schema_version: 1,
        slot: r.slot,
        target_transaction_index: 0,
        target_signature: entries[0].signature.clone(),
        full_block: s
            .put(
                EvidenceKind::Validator,
                &serde_json::to_vec(&block).unwrap(),
            )
            .unwrap(),
        start_checkpoint: start.clone(),
        terminal_checkpoint: terminal.clone(),
        runtime_evidence: r.runtime.historical_evidence_ref.clone().unwrap(),
        program_binaries: vec![binary.clone()],
        entries: entries.clone(),
        dependency_edges: plan.edges,
        terminal_frontier: plan.terminal,
        target_execution: target_ref.clone(),
    };
    r.protocol = "synthetic-standard-transfer".into();
    r.program_id = program.clone();
    r.signature = entries[0].signature.clone();
    r.execution = entries[0].execution.clone();
    r.target = SemanticTarget {
        program_id: program.clone(),
        outer_index: 0,
        instruction_identity: hash_bytes(b"synthetic transfer"),
    };
    r.instruction_roles = vec![InstructionAssignment {
        outer_index: 0,
        role: InstructionRole::SemanticTarget,
    }];
    r.account_seeds = refs
        .into_iter()
        .map(|(address, observation)| AccountSeed {
            address,
            boundary: AccountBoundary::BeforeTransaction,
            observation,
        })
        .collect();
    r.absent_pre_accounts.clear();
    r.binaries = vec![binary];
    r.dependencies = DependencyManifest {
        programs: vec![ProgramDependency {
            program_id: program,
            source: ProgramSource::HistoricalMainnet,
            loader: Some(ProgramLoader::Legacy),
            deployed_slot: None,
            binary_sha256: Some(elf_ref.sha256),
            binary_len: Some(elf.len() as u64),
            observed_slot: Some(r.slot - 1),
            discovered_by: vec![],
            note: None,
        }],
    };
    r.expected = entries[0].expected.clone();
    r.expected.watched_accounts = outputs
        .iter()
        .map(|k| WatchedAccount {
            address: k.clone(),
            source: ExpectedAccountSource::DerivedTargetBoundary,
            expected_post_content: Some(
                s.put(
                    EvidenceKind::AccountContent,
                    &serde_json::to_vec(target.post_accounts[k].as_ref().unwrap()).unwrap(),
                )
                .unwrap(),
            ),
        })
        .collect();
    r.checkpointed_execution = Some(CheckpointedExecutionProof {
        proof_contract_version: 3,
        start_checkpoint: start,
        terminal_checkpoint: terminal,
        closure_proof: proof.store(&s).unwrap(),
        deterministic_execution: target_ref,
        validation_outputs: outputs,
    });
    r.id = r.identity().unwrap();
    (r, proof, t, s, x, y)
}
#[test]
fn synthetic_later_mutation_preserves_target_boundary_and_candidate_comparison() {
    let (r, p, _t, s, x, _y) = synthetic();
    assert!(matches!(
        p.entries[1].execution,
        ExecutionInput::LegacyV2 { .. }
    ));
    let resolved = r.resolve(&s).unwrap();
    let terminal = ObservedCheckpointV1::resolve(
        &s,
        &p.terminal_checkpoint,
        r.slot,
        AccountBoundary::EndOfExecutionSlot,
        &r.genesis_hash,
    )
    .unwrap();
    let amount = |a: &Option<AccountSnapshot>| {
        u64::from_le_bytes(a.as_ref().unwrap().data[64..72].try_into().unwrap())
    };
    assert_eq!(amount(&resolved.expected_accounts[&x]), 17);
    assert_eq!(amount(&terminal[&x]), 28);
    let (local, fidelity) = pipeline::baseline(&r, &resolved).unwrap();
    assert!(fidelity.matched());
    assert_eq!(amount(&local.post_accounts[&x]), 17);
    let candidate = pipeline::execute(&r, &resolved, &resolved.baseline_elf).unwrap();
    assert_eq!(candidate, local);
    assert_ne!(candidate.post_accounts[&x], terminal[&x]);
}

#[test]
fn synthetic_legacy_sequence_entry_rejects_wrong_block_index() {
    let (r, mut proof, _temp, store, _x, _y) = synthetic();
    let entry = &mut proof.entries[1];
    assert!(matches!(entry.execution, ExecutionInput::LegacyV2 { .. }));
    let mut raw: Value = serde_json::from_slice(
        &store
            .get(entry.execution.validator_transaction_ref().unwrap())
            .unwrap(),
    )
    .unwrap();
    raw["transactionIndex"] = serde_json::json!(2);
    entry.execution = ExecutionInput::capture_legacy_v2(&store, &raw).unwrap();
    reject(
        r,
        proof,
        &store,
        "sequence transaction differs from retained block",
    );
}
#[test]
fn synthetic_commuting_transfers_still_require_canonical_dependency_order() {
    let (r, mut p, _t, s, _x, _y) = synthetic();
    let resolved = r.resolve(&s).unwrap();
    let mut messages = p
        .entries
        .iter()
        .map(|e| e.execution.resolve(&s, &r.genesis_hash).unwrap())
        .collect::<Vec<_>>();
    let watched = resolved.seeds.keys().cloned().collect::<Vec<_>>();
    let execute = |messages: &[eplyx_engine::universal::model::ResolvedMessage]| {
        LiteSvmBackend
            .execute_sequence(
                &ExecutionRequest {
                    message: &messages[0],
                    seeds: &resolved.seeds,
                    absent_pre_accounts: &[],
                    watched: &watched,
                    runtime_sysvars: &resolved.runtime_sysvars,
                    clock: None,
                    programs_to_load: &[],
                    runtime_profile: &resolved.runtime_profile,
                    unlimited_logs: true,
                    slot_hashes: SlotHashesVariant::BackendDefault,
                    recent_blockhashes: Default::default(),
                    require_complete_state: true,
                },
                messages,
            )
            .unwrap()
    };
    let canonical = execute(&messages);
    messages.swap(1, 2);
    let reordered = execute(&messages);
    assert_eq!(
        canonical.last().unwrap().post_accounts,
        reordered.last().unwrap().post_accounts
    );
    assert!(reordered.iter().all(|run| run.success));
    p.entries.swap(1, 2);
    reject(r, p, &s, "sequence membership or canonical order");
}
#[test]
fn synthetic_independent_terminal_detector() {
    let (r, mut p, _t, s, x, y) = synthetic();
    let resolved = r.resolve(&s).unwrap();
    let messages = p.entries[..2]
        .iter()
        .map(|e| e.execution.resolve(&s, &r.genesis_hash).unwrap())
        .collect::<Vec<_>>();
    let start: ObservedCheckpointV1 =
        serde_json::from_slice(&s.get(&p.start_checkpoint).unwrap()).unwrap();
    let watched = start
        .accounts
        .iter()
        .map(|a| a.address.clone())
        .collect::<Vec<_>>();
    let runs = LiteSvmBackend
        .execute_sequence(
            &ExecutionRequest {
                message: &messages[0],
                seeds: &resolved.seeds,
                absent_pre_accounts: &[],
                watched: &watched,
                runtime_sysvars: &resolved.runtime_sysvars,
                clock: None,
                programs_to_load: &[],
                runtime_profile: &resolved.runtime_profile,
                unlimited_logs: true,
                slot_hashes: SlotHashesVariant::BackendDefault,
                recent_blockhashes: Default::default(),
                require_complete_state: true,
            },
            &messages,
        )
        .unwrap();
    let terminal = ObservedCheckpointV1::resolve(
        &s,
        &p.terminal_checkpoint,
        r.slot,
        AccountBoundary::EndOfExecutionSlot,
        &r.genesis_hash,
    )
    .unwrap();
    assert_eq!(runs[1].post_accounts[&x], terminal[&x]);
    assert_ne!(runs[1].post_accounts[&y], terminal[&y]);
    p.entries.pop();
    assert!(verify_reconstruction(&r, &s, &p, &runs)
        .unwrap_err()
        .to_string()
        .contains("terminal reconciliation differs"));
}

#[test]
fn generic_sequence_execution_has_no_witness_specific_branches() {
    for source in [
        include_str!("../src/universal/sequence.rs"),
        include_str!("../src/universal/execution.rs"),
        include_str!("../src/universal/pipeline.rs"),
        include_str!("../src/universal/resolver.rs"),
    ] {
        for forbidden in [
            "Drift",
            "settlePnl",
            "settle_pnl",
            "dRiftyHA39",
            "2BD3UJFP",
            "409942000",
            "market index 3",
        ] {
            assert!(
                !source.contains(forbidden),
                "witness-specific core source: {forbidden}"
            );
        }
    }
}

#[test]
fn extra_unproven_transaction_is_rejected() {
    let (r, mut p, _t, s) = fixture();
    let mut extra = p.entries[0].clone();
    extra.transaction_index += 1;
    p.entries.insert(1, extra);
    reject(r, p, &s, "sequence membership or canonical order");
}

#[test]
fn incompatible_suffix_runtime_is_rejected() {
    let (r, mut p, _t, s) = fixture();
    p.entries[1].runtime_profile_id = "different historical runtime".into();
    reject(r, p, &s, "sequence runtime profile differs");
}
