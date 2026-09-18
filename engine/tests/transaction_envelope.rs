//! Frozen production messages and historical LUT evidence; transformed messages
//! and simulated acquisition records are controls, never production replays.
use eplyx_engine::{
    dependencies::{self, DependencyManifest, ProgramDependency, ProgramSource},
    envelope::InstructionRole,
    message::{self, ArchiveProvenance, FrozenV0, HistoricalAccountEvidence, ProvenV0},
    protocol::{
        kamino::{
            envelope::{self, ATA_ID, SCOPE_ID},
            KaminoKlendAdapter,
        },
        ProtocolAdapter,
    },
    replay::{hash_bytes, AccountAcquisition, AccountStateSource},
    screening::{AccountConflict, SlotScreening},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};
const GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/examples")
}
fn read(path: PathBuf) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn fixture(prefix: &str) -> (Value, Vec<HistoricalAccountEvidence>) {
    let targets = read(root().join("phase-u3b2-lut/targets.json"));
    let target = targets["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["signature"].as_str().unwrap().starts_with(prefix))
        .unwrap();
    let bytes = std::fs::read(
        root()
            .join("phase-u3-baseline")
            .join(target["capture_file"].as_str().unwrap()),
    )
    .unwrap();
    assert_eq!(
        hash_bytes(&bytes),
        target["capture_sha256"],
        "frozen raw transaction hash"
    );
    let raw: Value = serde_json::from_slice(&bytes).unwrap();
    let capture = read(root().join("phase-u3b2-lut/acquisition.json"));
    let provider: ArchiveProvenance = serde_json::from_value(capture["provider"].clone()).unwrap();
    let tables = target["address_table_lookups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            let r = capture["requests"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| {
                    r["table_pubkey"] == l["accountKey"]
                        && r["execution_slot"] == target["execution_slot"]
                })
                .unwrap();
            let bytes = std::fs::read(
                root()
                    .join("phase-u3b2-lut")
                    .join(r["response_file"].as_str().unwrap()),
            )
            .unwrap();
            assert_eq!(hash_bytes(&bytes), r["response_sha256"]);
            HistoricalAccountEvidence::from_response(
                l["accountKey"].as_str().unwrap(),
                target["execution_slot"].as_u64().unwrap(),
                provider.clone(),
                &bytes,
            )
            .unwrap()
        })
        .collect();
    (raw["result"].clone(), tables)
}
fn prove(raw: &Value, tables: &[HistoricalAccountEvidence]) -> ProvenV0 {
    message::reconstruct(&FrozenV0::from_rpc(raw, GENESIS).unwrap(), tables, None).unwrap()
}
fn primary() -> ProvenV0 {
    let (r, t) = fixture("5eLac");
    prove(&r, &t)
}
#[test]
fn four_real_primary_envelopes_are_classified_without_production_admission_change() {
    for prefix in ["5eLac", "55Edb", "5NKN", "49zi"] {
        let (r, t) = fixture(prefix);
        let p = prove(&r, &t);
        let a = envelope::analyse(&p);
        assert!(
            a.structurally_classified && a.envelope_admissible,
            "exact primary envelope: {:?}",
            a.blockers
        );
        assert_eq!(a.targets.len(), 1);
        assert_eq!(a.instructions.len(), p.transaction().instructions.len());
        let admitted = envelope::admit(&p).unwrap();
        admitted
            .check_instruction_sequence(&p.transaction().instructions)
            .unwrap();
        assert!(matches!(
            admitted.message().versioned_message(),
            solana_message::VersionedMessage::V0(_)
        ));
        assert!(KaminoKlendAdapter
            .accept(p.transaction())
            .unwrap_err()
            .to_string()
            .contains("lookup tables"));
        assert!(KaminoKlendAdapter
            .accept_reconstructed_message(&p)
            .unwrap_err()
            .to_string()
            .contains(SCOPE_ID));
    }
}
#[test]
fn real_scope_identity_bytes_accounts_and_tokens_are_exact() {
    let p = primary();
    let ix = &p.transaction().instructions[0];
    assert_eq!(ix.program, SCOPE_ID);
    assert_eq!(
        ix.data,
        vec![83, 186, 207, 131, 203, 254, 198, 130, 4, 0, 0, 0, 88, 1, 23, 1, 13, 0, 200, 1],
        "exact frozen Scope instruction bytes"
    );
    assert_eq!(
        ix.accounts
            .iter()
            .map(|a| a.address.as_str())
            .collect::<Vec<_>>(),
        vec![
            "3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH",
            "4zh6bmb77qX2CL7t5AJYCqa6YqFafbz3QJNeFvZjLowg",
            "6L6vUts9tYqxHVUCEFVc2mzZw6yxMn8C6a44cp5ga7e9",
            "Sysvar1nstructions1111111111111111111111111",
            SCOPE_ID,
            SCOPE_ID,
            SCOPE_ID,
            SCOPE_ID
        ],
        "every account position retained, including repeated self references"
    );
    for (prefix, expected) in [
        ("5eLac", vec![344, 279, 13, 456]),
        ("55Edb", vec![3, 455, 13, 456, 25, 459, 148, 461]),
        ("5NKN", vec![221, 3, 221, 455, 13, 456]),
        ("49zi", vec![416, 501]),
    ] {
        let (r, t) = fixture(prefix);
        let p = prove(&r, &t);
        assert_eq!(
            envelope::scope_tokens(&p.transaction().instructions[0]).unwrap(),
            expected,
            "tokens retained in original order without deduplication"
        );
    }
}
#[test]
fn real_scope_decoder_rejects_wrong_identity_data_arity_and_privileges() {
    let p = primary();
    let original = p.transaction().instructions[0].clone();
    for mode in 0..7 {
        let mut ix = original.clone();
        match mode {
            0 => ix.program = ATA_ID.into(),
            1 => ix.data[0] ^= 1,
            2 => {
                ix.accounts.pop();
            }
            3 => ix.accounts.push(ix.accounts[4].clone()),
            4 => {
                ix.data.pop();
            }
            5 => ix.accounts[0].is_writable = false,
            _ => ix.accounts[3].is_signer = true,
        }
        assert!(
            envelope::scope_tokens(&ix).is_err(),
            "Scope malformed shape {mode} rejected"
        );
    }
}
#[test]
fn dependency_has_no_scope_semantic_support() {
    let a = envelope::analyse(&primary());
    let scope = &a.instructions[0];
    assert_eq!(scope.role, InstructionRole::ExecutionDependency);
    assert!(
        !scope.semantic_supported,
        "assertion: execution dependency has no Scope semantic coverage"
    );
    assert!(a.targets.iter().all(|t| t.program_id != SCOPE_ID));
    assert_eq!(
        scope.consumed_by,
        vec![2, 3],
        "mutated price is used by later reserve refreshes"
    );
}
#[test]
fn target_signer_and_positive_amount_contract_is_preserved() {
    let (original, tables) = fixture("5eLac");
    for mode in 0..2 {
        let mut raw = original.clone();
        let target = &mut raw["transaction"]["message"]["instructions"][5];
        if mode == 0 {
            target["accounts"][0] = json!(1);
        } else {
            let mut bytes = bs58::decode(target["data"].as_str().unwrap())
                .into_vec()
                .unwrap();
            bytes[8..].fill(0);
            target["data"] = json!(bs58::encode(bytes).into_string());
        }
        assert!(
            envelope::admit(&prove(&raw, &tables)).is_none(),
            "unchanged U2 signer/nonzero amount requirement"
        );
    }
}
#[test]
fn arbitrary_external_program_cannot_replace_scope() {
    let (mut r, t) = fixture("5eLac");
    let i = r["transaction"]["message"]["instructions"][0]["programIdIndex"]
        .as_u64()
        .unwrap() as usize;
    r["transaction"]["message"]["accountKeys"][i] = json!(bs58::encode([199; 32]).into_string());
    let p = prove(&r, &t);
    assert!(
        envelope::admit(&p).is_none(),
        "assertion: arbitrary external program must not be admitted"
    );
}
#[test]
fn unknown_extra_companion_cannot_hide_behind_required_dependencies() {
    let (mut r, t) = fixture("5eLac");
    let mut extra = r["transaction"]["message"]["instructions"][0].clone();
    // An existing static key used as an unrecognized program in this explicitly
    // transformed message. All required original instructions remain present.
    extra["programIdIndex"] = json!(1);
    extra["accounts"] = json!([]);
    extra["data"] = json!(bs58::encode([9; 8]).into_string());
    r["transaction"]["message"]["instructions"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    assert!(
        envelope::admit(&prove(&r, &t)).is_none(),
        "assertion: unknown extra rejected with all required dependencies present"
    );
}
#[test]
fn scope_after_reserve_refresh_is_rejected() {
    let (mut r, t) = fixture("5eLac");
    let seq = r["transaction"]["message"]["instructions"]
        .as_array_mut()
        .unwrap();
    let scope = seq.remove(0);
    seq.insert(4, scope);
    assert!(
        envelope::admit(&prove(&r, &t)).is_none(),
        "assertion: Scope must precede its consuming refresh; original order is contractual"
    );
}
#[test]
fn complete_sequence_cannot_strip_or_reorder_dependency() {
    let p = primary();
    let admitted = envelope::admit(&p).unwrap();
    let mut seq = p.transaction().instructions.clone();
    seq.remove(0);
    assert!(
        admitted.check_instruction_sequence(&seq).is_err(),
        "assertion: Scope cannot be stripped from replay plan"
    );
    let mut seq = p.transaction().instructions.clone();
    seq.swap(2, 3);
    assert!(
        admitted.check_instruction_sequence(&seq).is_err(),
        "all original bytes/account references and positions preserved"
    );
}
#[test]
fn unexpected_known_extra_dependency_is_rejected() {
    let (mut r, t) = fixture("5eLac");
    let seq = r["transaction"]["message"]["instructions"]
        .as_array_mut()
        .unwrap();
    seq.push(seq.last().unwrap().clone());
    assert!(
        envelope::admit(&prove(&r, &t)).is_none(),
        "assertion: duplicate compute instruction outside observed exact envelope"
    );
    let (mut r, t) = fixture("5eLac");
    let seq = r["transaction"]["message"]["instructions"]
        .as_array_mut()
        .unwrap();
    let rr = seq[2].clone();
    seq.insert(3, rr.clone());
    seq.insert(4, rr);
    assert!(envelope::admit(&prove(&r,&t)).is_none(),"duplicated reserve refreshes cannot turn an eight-instruction envelope into a ten-instruction profile");
}
#[test]
fn real_multi_action_observations_preserve_both_outer_indices() {
    for prefix in ["3zsw", "37XJ"] {
        let (r, t) = fixture(prefix);
        let p = prove(&r, &t);
        let a = envelope::analyse(&p);
        assert_eq!(a.targets.len(), 2, "never select only the first action");
        assert_ne!(
            a.targets[0].outer_index, a.targets[1].outer_index,
            "durable distinct observation identities"
        );
    }
}
#[test]
fn real_multi_action_ids_are_not_merged() {
    let (r, t) = fixture("3zsw");
    let a = envelope::analyse(&prove(&r, &t));
    assert_eq!(
        a.targets[0].action_id,
        "deposit_reserve_liquidity_and_obligation_collateral"
    );
    assert_eq!(
        a.targets[1].action_id, "borrow_obligation_liquidity",
        "borrow must not be merged into deposit action ID"
    );
}
#[test]
fn real_multi_action_whole_transaction_delta_is_unevaluable() {
    let (r, t) = fixture("3zsw");
    let p = prove(&r, &t);
    let a = envelope::analyse(&p);
    assert_eq!(
        a.attribution, "unsupported_multi_action_attribution",
        "no whole-transaction delta attributed to one of two actions"
    );
    assert!(a
        .blockers
        .iter()
        .any(|b| b.code == "unsupported_multi_action_attribution"));
    assert!(envelope::admit(&p).is_none());
    assert!(
        KaminoKlendAdapter.action_id(p.transaction()).is_none(),
        "existing evaluator has no independent attribution"
    );
}
#[test]
fn real_failed_original_policy_survives_exact_lut_proof() {
    let (r, t) = fixture("47XZ");
    let p = prove(&r, &t);
    let a = envelope::analyse(&p);
    assert_eq!(
        a.blockers[0].code, "failed_original",
        "failed-original policy retains first priority"
    );
    assert!(
        a.blockers.iter().any(|b| b.code == "failed_original"),
        "assertion: failed-original remains an explicit rejection even with exact LUT proof"
    );
    assert!(envelope::admit(&p).is_none());
}
#[test]
fn primary_ata_existing_evidence_cannot_be_replaced_by_creation() {
    let (mut r, t) = fixture("5eLac");
    let p = prove(&r, &t);
    let ata = &p.transaction().instructions[1];
    assert_eq!(ata.data, vec![1]);
    assert_eq!(ata.program, ATA_ID);
    let index = p
        .transaction()
        .account_keys
        .iter()
        .position(|a| a.address == ata.accounts[1].address)
        .unwrap();
    r["meta"]["preBalances"][index] = json!(0);
    r["meta"]["preTokenBalances"]
        .as_array_mut()
        .unwrap()
        .retain(|b| b["accountIndex"] != json!(index));
    assert!(
        envelope::admit(&prove(&r, &t)).is_none(),
        "assertion: creation or unknown ATA lifecycle must not be treated as existing-account no-op"
    );
}
// The following are explicitly simulated provenance guards. They are not state
// bytes, binary proof or a controlled execution of Scope state effects.
fn seed_records(p: &ProvenV0) -> (Vec<AccountAcquisition>, SlotScreening) {
    let a = envelope::analyse(p);
    let mut keys = a
        .instructions
        .iter()
        .filter(|i| i.role == InstructionRole::ExecutionDependency)
        .flat_map(|i| {
            i.required_state
                .iter()
                .filter(|k| *k != &i.instruction.program && !k.starts_with("Sysvar"))
        })
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    let sources = keys
        .iter()
        .map(|k| AccountAcquisition {
            address: k.clone(),
            label: "synthetic provenance control; no account bytes".into(),
            discovered_by: vec![],
            source: AccountStateSource::HistoricalArchive,
            context_slot: a.execution_slot - 1,
            method: "simulated getAccountInfo".into(),
        })
        .collect();
    let screen = SlotScreening {
        slot: a.execution_slot,
        target_signature: a.signature,
        target_index: 0,
        transactions_in_slot: 1,
        required_accounts: keys,
        conflicts: vec![],
    };
    (sources, screen)
}
#[test]
fn current_or_intermediate_scope_state_is_rejected() {
    let p = primary();
    let admitted = envelope::admit(&p).unwrap();
    let (sources, screen) = seed_records(&p);
    admitted.check_seed_context(&sources, &screen).unwrap();
    for slot in [p.transaction().slot, p.transaction().slot + 100] {
        let mut changed = sources.clone();
        changed[0].context_slot = slot;
        assert!(
            admitted.check_seed_context(&changed, &screen).is_err(),
            "assertion: dependency seeds require S-1; intermediate/post/current state forbidden"
        );
    }
}
#[test]
fn same_slot_dependency_interference_stays_fail_closed() {
    let p = primary();
    let admitted = envelope::admit(&p).unwrap();
    let (sources, mut screen) = seed_records(&p);
    screen.conflicts.push(AccountConflict {
        account: sources[0].address.clone(),
        target_signature: p.transaction().signature.clone(),
        target_index: 0,
        conflicting_signature: Some("synthetic conflicting writer".into()),
        conflicting_index: Some(1),
        position: None,
        reason: "synthetic same-slot data write".into(),
    });
    assert!(
        admitted.check_seed_context(&sources, &screen).is_err(),
        "same-slot interference cannot be bypassed for Scope"
    );
}
fn binary_records(p: &ProvenV0) -> (DependencyManifest, BTreeMap<String, Vec<u8>>) {
    let bytes = b"synthetic hash guard bytes; not executable ELF".to_vec();
    let mut binaries = BTreeMap::new();
    let programs = dependencies::discover(
        p.transaction(),
        None,
        "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
    )
    .into_iter()
    .map(|(id, by)| {
        binaries.insert(id.clone(), bytes.clone());
        ProgramDependency {
            program_id: id,
            source: ProgramSource::HistoricalMainnet,
            loader: None,
            deployed_slot: Some(p.transaction().slot - 2),
            observed_slot: Some(p.transaction().slot - 1),
            binary_sha256: Some(hash_bytes(&bytes)),
            binary_len: Some(bytes.len() as u64),
            discovered_by: by,
            note: Some("synthetic provenance guard only".into()),
        }
    })
    .collect();
    (DependencyManifest { programs }, binaries)
}
#[test]
fn current_external_binary_is_rejected() {
    let p = primary();
    let admitted = envelope::admit(&p).unwrap();
    let (mut manifest, binaries) = binary_records(&p);
    admitted
        .check_binary_manifest(&manifest, &binaries)
        .unwrap();
    manifest
        .programs
        .iter_mut()
        .find(|p| p.program_id == SCOPE_ID)
        .unwrap()
        .observed_slot = Some(p.transaction().slot + 100);
    assert!(
        admitted
            .check_binary_manifest(&manifest, &binaries)
            .is_err(),
        "assertion: current external ELF cannot replace historical pre-slot deployment"
    );
}
#[test]
fn missing_or_tampered_dependency_binary_fails_closed() {
    let p = primary();
    let admitted = envelope::admit(&p).unwrap();
    let (mut manifest, mut binaries) = binary_records(&p);
    binaries.get_mut(SCOPE_ID).unwrap().push(0);
    assert!(
        admitted
            .check_binary_manifest(&manifest, &binaries)
            .is_err(),
        "hash/length mismatch rejected"
    );
    manifest.programs.retain(|p| p.program_id != SCOPE_ID);
    assert!(
        admitted
            .check_binary_manifest(&manifest, &binaries)
            .is_err(),
        "external dependency cannot be omitted"
    );
}
#[test]
fn serialized_analysis_cannot_seal_an_envelope() {
    let (mut raw, tables) = fixture("5eLac");
    let index = raw["transaction"]["message"]["instructions"][0]["programIdIndex"]
        .as_u64()
        .unwrap() as usize;
    raw["transaction"]["message"]["accountKeys"][index] =
        json!(bs58::encode([199; 32]).into_string());
    let p = prove(&raw, &tables);
    let a = envelope::analyse(&p);
    let mut value = serde_json::to_value(a).unwrap();
    value["envelope_admissible"] = json!(true);
    value["instructions"] = json!([]);
    // Analysis is diagnostic only; admission always reconstructs from ProvenV0.
    let _diagnostic: eplyx_engine::envelope::EnvelopeAnalysis =
        serde_json::from_value(value).unwrap();
    assert!(
        envelope::admit(&p).is_none(),
        "forged diagnostic flag cannot admit unsupported raw message"
    );
}
