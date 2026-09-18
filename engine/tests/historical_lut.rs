//! Synthetic slot-rule controls and separately acquired frozen production LUT
//! envelopes. Production tests read docs/examples/phase-u3b2-lut offline; their
//! table bytes never come from transaction loadedAddresses. The explicitly
//! transformed warmup fixture uses real bytes with a synthetic bank context.
use anyhow::Result;
use base64::{prelude::BASE64_STANDARD, Engine};
use eplyx_engine::{
    ingest::rpc::RpcProvider,
    message::{self, ArchiveProvenance, FrozenV0, HistoricalAccountEvidence, HistoricalVisibility},
    replay::hash_bytes,
};
use serde_json::{json, Value};
use solana_address::Address;
use solana_address_lookup_table_interface::{
    program,
    state::{AddressLookupTable, LookupTableMeta},
};
use solana_hash::Hash;
use solana_slot_hashes::SlotHashes;
use std::{borrow::Cow, collections::BTreeMap};
const GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const SLOT: u64 = 100;
fn key(n: u8) -> String {
    Address::new_from_array([n; 32]).to_string()
}
fn provenance() -> ArchiveProvenance {
    ArchiveProvenance {
        scheme_host: "https://synthetic.invalid".into(),
        genesis_hash: GENESIS.into(),
        visibility: HistoricalVisibility::FinalizedEndOfExecutionSlot,
        validation_artifact_sha256: hash_bytes(
            b"synthetic validated archive fixture; not production",
        ),
    }
}
fn envelope(slot: u64, data: &[u8], owner: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":1, "result":{"context":{"slot":slot}, "value":{"owner":owner, "lamports":10_000_000, "executable":false, "rentEpoch":0,"data":[BASE64_STANDARD.encode(data),"base64"]}}})).unwrap()
}
fn table_bytes(keys: &[String], meta: LookupTableMeta) -> Vec<u8> {
    AddressLookupTable {
        meta,
        addresses: Cow::Owned(keys.iter().map(|k| k.parse().unwrap()).collect()),
    }
    .serialize_for_tests()
    .unwrap()
}
fn evidence(pubkey: &str, keys: &[String], meta: LookupTableMeta) -> HistoricalAccountEvidence {
    HistoricalAccountEvidence::from_response(
        pubkey,
        SLOT,
        provenance(),
        &envelope(SLOT, &table_bytes(keys, meta), &program::id().to_string()),
    )
    .unwrap()
}
fn fixture() -> (Value, Vec<HistoricalAccountEvidence>) {
    let a = vec![key(13), key(12), key(11), key(15), key(14)];
    let b = vec![key(24), key(23), key(22), key(21)];
    let rpc = json!({"slot":SLOT,"blockTime":0,"version":0,"transaction":{"signatures":[bs58::encode([1;64]).into_string()], "message":{"header":{"numRequiredSignatures":1,"numReadonlySignedAccounts":0,"numReadonlyUnsignedAccounts":1},"accountKeys":[key(1),key(2),key(3)],"recentBlockhash":Hash::new_from_array([4;32]).to_string(),"addressTableLookups":[{"accountKey":key(30),"writableIndexes":[0,1,2],"readonlyIndexes":[3,4]},{"accountKey":key(31),"writableIndexes":[0,1],"readonlyIndexes":[2,3]}],"instructions":[{"programIdIndex":2,"accounts":[0,1,3,5,6,8,11,3],"data":bs58::encode([5,4,3,2,1]).into_string()}]}},"meta":{"err":null,"fee":5000,"loadedAddresses":{"writable":[a[0],a[1],a[2],b[0],b[1]],"readonly":[a[3],a[4],b[2],b[3]]},"innerInstructions":[],"logMessages":[]}});
    (
        rpc,
        vec![
            evidence(&key(30), &a, LookupTableMeta::default()),
            evidence(&key(31), &b, LookupTableMeta::default()),
        ],
    )
}
fn prove(
    raw: &Value,
    tables: &[HistoricalAccountEvidence],
) -> std::result::Result<message::ProvenV0, message::LutProofFailure> {
    message::reconstruct(&FrozenV0::from_rpc(raw, GENESIS).unwrap(), tables, None)
}
fn replace_table(tables: &mut [HistoricalAccountEvidence], pos: usize, meta: LookupTableMeta) {
    let old = &tables[pos];
    let account = old.account(SLOT, GENESIS).unwrap();
    let table = AddressLookupTable::deserialize(&account.data).unwrap();
    let keys = table
        .addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    tables[pos] = evidence(&old.pubkey, &keys, meta);
}
#[test]
fn independent_resolution_preserves_writable_readonly_order() {
    let (raw, tables) = fixture();
    let result = prove(&raw, &tables);
    assert!(
        result.is_ok(),
        "assertion: exact independent reconstruction: {:?}",
        result.as_ref().err()
    );
    let p = result.unwrap();
    assert_eq!(
        p.proof()
            .resolved_writable
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![key(13), key(12), key(11), key(24), key(23)],
        "writable order"
    );
    assert_eq!(
        p.proof()
            .resolved_readonly
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![key(15), key(14), key(22), key(21)],
        "readonly order"
    );
    assert_eq!(p.proof().tables.len(), 2, "second table retained");
    assert!(
        matches!(
            p.versioned_message(),
            solana_message::VersionedMessage::V0(_)
        ),
        "native v0 retained"
    );
}
#[test]
fn single_writable_and_readonly_and_many_addresses() {
    for (w, r) in [(1, 0), (0, 1), (1, 1), (25, 30)] {
        let (mut raw, _) = fixture();
        let keys = (40..40 + w + r).map(|n| key(n as u8)).collect::<Vec<_>>();
        raw["transaction"]["message"]["addressTableLookups"] = json!([{"accountKey":key(30),"writableIndexes":(0..w).collect::<Vec<_>>(),"readonlyIndexes":(w..w+r).collect::<Vec<_>>()}]);
        raw["transaction"]["message"]["instructions"][0]["accounts"] = json!([0, 2, 3 + w + r - 1]);
        raw["meta"]["loadedAddresses"] = json!({"writable":keys[..w],"readonly":keys[w..]});
        let p = prove(
            &raw,
            &[evidence(&key(30), &keys, LookupTableMeta::default())],
        )
        .expect("single table order");
        assert_eq!(p.proof().full_account_keys.len(), 3 + w + r);
    }
}
#[test]
fn official_metadata_decode_and_raw_hashes_are_stable() {
    let (raw, mut tables) = fixture();
    let meta = LookupTableMeta {
        last_extended_slot: 90,
        last_extended_slot_start_index: 2,
        authority: Some(key(90).parse().unwrap()),
        ..LookupTableMeta::default()
    };
    replace_table(&mut tables, 0, meta.clone());
    let p = prove(&raw, &tables).unwrap();
    assert_eq!(p.proof().tables[0].metadata, meta, "official metadata");
    assert_eq!(p.proof().tables[0].addresses.len(), 5);
    let copy: HistoricalAccountEvidence =
        serde_json::from_slice(&serde_json::to_vec(&tables[0]).unwrap()).unwrap();
    assert_eq!(tables[0], copy, "stable hash roundtrip");
    assert_eq!(
        tables[0].raw_account_sha256,
        hash_bytes(&copy.account(SLOT, GENESIS).unwrap().data)
    );
}
#[test]
fn wrong_owner_and_malformed_and_empty_table_fail_closed() {
    let (raw, tables) = fixture();
    for (bytes, owner) in [
        (table_bytes(&[key(13)], LookupTableMeta::default()), key(99)),
        (vec![0; 5], program::id().to_string()),
        (
            table_bytes(&[], LookupTableMeta::default()),
            program::id().to_string(),
        ),
    ] {
        let mut bad = tables.clone();
        bad[0] = HistoricalAccountEvidence::from_response(
            &key(30),
            SLOT,
            provenance(),
            &envelope(SLOT, &bytes, &owner),
        )
        .unwrap();
        let e = prove(&raw, &bad).expect_err("invalid owner/codec/index must reject");
        assert!(e.stage == 2 || e.stage == 3, "correct decode/lookup stage");
    }
}
#[test]
fn wrong_owner_is_rejected_before_address_comparison() {
    let (raw, mut tables) = fixture();
    let account = tables[0].account(SLOT, GENESIS).unwrap();
    tables[0] = HistoricalAccountEvidence::from_response(
        &key(30),
        SLOT,
        provenance(),
        &envelope(SLOT, &account.data, &key(99)),
    )
    .unwrap();
    let result = prove(&raw, &tables);
    assert!(result.is_err(), "assertion: wrong LUT owner accepted");
    assert_eq!(result.unwrap_err().stage, 2, "owner assertion");
}
#[test]
fn same_slot_extension_only_exposes_old_prefix() {
    let (raw, mut tables) = fixture();
    replace_table(
        &mut tables,
        0,
        LookupTableMeta {
            last_extended_slot: SLOT,
            last_extended_slot_start_index: 2,
            ..LookupTableMeta::default()
        },
    );
    let result = prove(&raw, &tables);
    assert!(result.is_err(), "assertion: same-slot suffix accepted");
    assert_eq!(result.unwrap_err().stage, 3, "warmup assertion");
    let (mut raw, mut tables) = fixture();
    raw["transaction"]["message"]["addressTableLookups"][0]["writableIndexes"] = json!([0]);
    raw["transaction"]["message"]["addressTableLookups"][0]["readonlyIndexes"] = json!([1]);
    raw["transaction"]["message"]["instructions"][0]["accounts"] = json!([0, 3, 6]);
    raw["meta"]["loadedAddresses"] =
        json!({"writable":[key(13),key(24),key(23)],"readonly":[key(12),key(22),key(21)]});
    replace_table(
        &mut tables,
        0,
        LookupTableMeta {
            last_extended_slot: SLOT,
            last_extended_slot_start_index: 2,
            ..LookupTableMeta::default()
        },
    );
    assert!(
        prove(&raw, &tables).is_ok(),
        "same-slot old prefix must remain usable"
    );
}
#[test]
fn writable_and_readonly_out_of_range_indexes_reject() {
    for field in ["writableIndexes", "readonlyIndexes"] {
        let (mut raw, tables) = fixture();
        raw["transaction"]["message"]["addressTableLookups"][0][field][0] = json!(255);
        assert_eq!(
            prove(&raw, &tables)
                .expect_err("out-of-range index accepted")
                .stage,
            3,
            "no clamping"
        );
    }
}
#[test]
fn deactivation_uses_exact_historical_slot_hashes() {
    let (raw, mut tables) = fixture();
    replace_table(
        &mut tables,
        0,
        LookupTableMeta {
            deactivation_slot: SLOT,
            ..LookupTableMeta::default()
        },
    );
    assert!(
        prove(&raw, &tables).is_ok(),
        "same-slot deactivation still usable"
    );
    replace_table(
        &mut tables,
        0,
        LookupTableMeta {
            deactivation_slot: 90,
            ..LookupTableMeta::default()
        },
    );
    assert_eq!(
        prove(&raw, &tables)
            .expect_err("missing SlotHashes accepted")
            .stage,
        2
    );
    let frozen = FrozenV0::from_rpc(&raw, GENESIS).unwrap();
    let hashes = SlotHashes::new(&[
        (99, Hash::new_from_array([9; 32])),
        (90, Hash::new_from_array([8; 32])),
    ]);
    let slot_evidence = HistoricalAccountEvidence::from_response(
        message::SLOT_HASHES_ID,
        SLOT,
        provenance(),
        &envelope(
            SLOT,
            &wincode::serialize(&hashes).unwrap(),
            message::SYSVAR_OWNER,
        ),
    )
    .unwrap();
    assert!(
        message::reconstruct(&frozen, &tables, Some(&slot_evidence)).is_ok(),
        "cooldown canonical usable"
    );
    replace_table(
        &mut tables,
        0,
        LookupTableMeta {
            deactivation_slot: 80,
            ..LookupTableMeta::default()
        },
    );
    assert_eq!(
        message::reconstruct(&frozen, &tables, Some(&slot_evidence))
            .expect_err("inactive accepted")
            .stage,
        2
    );
}
#[test]
fn future_table_metadata_rejects() {
    for meta in [
        LookupTableMeta {
            last_extended_slot: 101,
            ..LookupTableMeta::default()
        },
        LookupTableMeta {
            deactivation_slot: 101,
            ..LookupTableMeta::default()
        },
    ] {
        let (raw, mut tables) = fixture();
        replace_table(&mut tables, 0, meta);
        assert_eq!(
            prove(&raw, &tables)
                .expect_err("future metadata accepted")
                .stage,
            2
        );
    }
}
#[test]
fn rpc_loaded_metadata_is_never_a_table_proof() {
    let (raw, _) = fixture();
    let result = prove(&raw, &[]);
    assert!(result.is_err(), "assertion: RPC-only message accepted");
    assert_eq!(result.unwrap_err().stage, 1, "proof mandatory");
}
#[test]
fn rpc_address_length_order_and_pubkey_mismatch_reject() {
    for kind in 0..3 {
        let (mut raw, tables) = fixture();
        let a = raw["meta"]["loadedAddresses"]["writable"]
            .as_array_mut()
            .unwrap();
        match kind {
            0 => {
                a.swap(0, 1);
            }
            1 => {
                a[0] = json!(key(100));
            }
            _ => {
                a.pop();
            }
        }
        // Length mismatch is already refused by the existing normalizer.
        if kind == 2 {
            assert!(FrozenV0::from_rpc(&raw, GENESIS).is_err());
        } else {
            assert_eq!(
                prove(&raw, &tables).expect_err("mismatch accepted").stage,
                4
            );
        }
    }
}
#[test]
fn full_key_order_and_loaded_privileges_are_exact() {
    let (raw, tables) = fixture();
    let frozen = FrozenV0::from_rpc(&raw, GENESIS).unwrap();
    let p = prove(&raw, &tables).unwrap();
    let expected = vec![
        key(1),
        key(2),
        key(3),
        key(13),
        key(12),
        key(11),
        key(24),
        key(23),
        key(15),
        key(14),
        key(22),
        key(21),
    ];
    assert_eq!(
        p.proof()
            .full_account_keys
            .iter()
            .map(|k| k.address.clone())
            .collect::<Vec<_>>(),
        expected,
        "static/writable/readonly vector"
    );
    assert!(
        p.proof().full_account_keys[3..]
            .iter()
            .all(|k| !k.is_signer),
        "loaded must not sign"
    );
    assert!(p.proof().full_account_keys[3..8]
        .iter()
        .all(|k| k.is_writable));
    assert!(p.proof().full_account_keys[8..]
        .iter()
        .all(|k| !k.is_writable));
    let mut forged = p.proof().clone();
    forged.full_account_keys[3].is_signer = true;
    assert!(
        message::validate_proof(&frozen, &tables, None, &forged).is_err(),
        "forged signer caught"
    );
    let mut shifted = p.proof().clone();
    shifted.resolved_writable.rotate_left(1);
    assert!(
        message::validate_proof(&frozen, &tables, None, &shifted).is_err(),
        "shifted loaded vector caught"
    );
}
#[test]
fn duplicates_are_rejected_without_deduplication() {
    for mode in 0..3 {
        let (mut raw, mut tables) = fixture();
        if mode == 0 {
            raw["transaction"]["message"]["accountKeys"][1] = json!(key(13));
        } else if mode == 1 {
            raw["transaction"]["message"]["addressTableLookups"][0]["writableIndexes"][1] =
                json!(0);
            raw["meta"]["loadedAddresses"]["writable"][1] = json!(key(13));
        } else {
            tables[1] = evidence(
                &key(31),
                &[key(13), key(23), key(22), key(21)],
                LookupTableMeta::default(),
            );
            raw["meta"]["loadedAddresses"]["writable"][3] = json!(key(13));
        }
        assert_eq!(
            prove(&raw, &tables).expect_err("duplicate accepted").stage,
            5,
            "official duplicate rule"
        );
    }
}
#[test]
fn table_identity_binds_pubkey_bytes_slot_and_genesis() {
    let a = evidence(&key(30), &[key(13)], LookupTableMeta::default());
    let b = evidence(&key(31), &[key(13)], LookupTableMeta::default());
    assert_eq!(a.raw_account_sha256, b.raw_account_sha256);
    assert_ne!(a.evidence_id, b.evidence_id, "pubkey bound to identity");
    let c = evidence(&key(30), &[key(14)], LookupTableMeta::default());
    assert_ne!(a.evidence_id, c.evidence_id, "bytes bound");
    assert!(
        a.account(SLOT + 1, GENESIS).is_err(),
        "execution slot bound"
    );
    assert!(
        a.account(SLOT, &Hash::new_from_array([9; 32]).to_string())
            .is_err(),
        "genesis bound"
    );
    let mut corrupt = a;
    corrupt.raw_account_sha256 = c.raw_account_sha256;
    assert!(corrupt.account(SLOT, GENESIS).is_err());
}
#[test]
fn reversed_input_and_parallel_reconstruction_are_byte_identical() {
    let (raw, mut tables) = fixture();
    let a = serde_json::to_vec(prove(&raw, &tables).unwrap().proof()).unwrap();
    tables.reverse();
    let b = serde_json::to_vec(prove(&raw, &tables).unwrap().proof()).unwrap();
    assert_eq!(a, b, "input enumeration irrelevant");
    std::thread::scope(|scope| {
        let jobs = (0..4)
            .map(|_| {
                scope.spawn(|| serde_json::to_vec(prove(&raw, &tables).unwrap().proof()).unwrap())
            })
            .collect::<Vec<_>>();
        for job in jobs {
            assert_eq!(a, job.join().unwrap(), "concurrency determinism");
        }
    });
}
struct ChangingArchive {
    old: Vec<u8>,
    current: Vec<u8>,
}
impl RpcProvider for ChangingArchive {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        if method == "getGenesisHash" {
            return Ok(json!(GENESIS));
        }
        let slot = params[1]["slot"].as_u64().unwrap_or(101);
        let data = if slot == SLOT {
            &self.old
        } else {
            &self.current
        };
        Ok(
            serde_json::from_slice::<Value>(&envelope(slot, data, &program::id().to_string()))?
                ["result"]
                .clone(),
        )
    }
}
#[test]
fn acquisition_never_substitutes_current_state_for_historical() {
    let archive = ChangingArchive {
        old: table_bytes(&[key(13)], LookupTableMeta::default()),
        current: table_bytes(
            &[key(14), key(15)],
            LookupTableMeta {
                last_extended_slot: 101,
                ..LookupTableMeta::default()
            },
        ),
    };
    let result = message::acquire_historical_account(&archive, &key(30), SLOT, provenance());
    assert!(
        result.is_ok(),
        "assertion: exact historical acquisition: {:?}",
        result.as_ref().err()
    );
    let a = result.unwrap();
    assert_eq!(
        a.raw_account_sha256,
        hash_bytes(&archive.old),
        "historical bytes used"
    );
    assert!(
        HistoricalAccountEvidence::from_response(
            &key(30),
            SLOT,
            provenance(),
            &envelope(101, &archive.current, &program::id().to_string())
        )
        .is_err(),
        "current response rejected"
    );
}
#[test]
fn compiled_indices_are_validated_by_official_v0_sanitizer() {
    for (field, index) in [("accounts", 255), ("programIdIndex", 3)] {
        let (mut raw, _) = fixture();
        if field == "accounts" {
            raw["transaction"]["message"]["instructions"][0][field][0] = json!(index);
        } else {
            raw["transaction"]["message"]["instructions"][0][field] = json!(index);
        }
        assert!(
            FrozenV0::from_rpc(&raw, GENESIS).is_err(),
            "official compiled index assertion"
        );
    }
}
#[test]
fn real_five_table_55_key_compiled_fixture_with_synthetic_table_bytes() {
    let root = eplyx_engine::repo_root().join("docs/examples/phase-u3-baseline");
    let signature =
        "47XZ8dNPUBxMTnJ3zrxeCKt37PRsckvhKzPSutdUp2NavU1UTTNYRd7rnvEP3pSmHQszrLW3cdB56zTw3kyrwZUf";
    let raw: Value = serde_json::from_slice(
        &std::fs::read(root.join(format!("transactions/{signature}.json"))).unwrap(),
    )
    .unwrap();
    let raw = &raw["result"];
    let slot = raw["slot"].as_u64().unwrap();
    let frozen = FrozenV0::from_rpc(raw, GENESIS).unwrap();
    let loaded = &raw["meta"]["loadedAddresses"];
    let mut wi = 0;
    let mut ri = 0;
    let mut tables = Vec::new();
    for lookup in &frozen.native_message().address_table_lookups {
        let mut mapping = BTreeMap::new();
        for (indexes, field, cursor) in [
            (&lookup.writable_indexes, "writable", &mut wi),
            (&lookup.readonly_indexes, "readonly", &mut ri),
        ] {
            for i in indexes {
                mapping.insert(*i, loaded[field][*cursor].as_str().unwrap().to_owned());
                *cursor += 1;
            }
        }
        let len = usize::from(*mapping.keys().max().unwrap()) + 1;
        let mut keys = vec![key(180); len];
        for (i, address) in mapping {
            keys[usize::from(i)] = address;
        }
        let evidence = HistoricalAccountEvidence::from_response(
            &lookup.account_key.to_string(),
            slot,
            provenance(),
            &envelope(
                slot,
                &table_bytes(&keys, LookupTableMeta::default()),
                &program::id().to_string(),
            ),
        )
        .unwrap();
        tables.push(evidence);
    }
    let result = message::reconstruct(&frozen, &tables, None);
    assert!(
        result.is_ok(),
        "assertion: real index fixture with SYNTHETIC state: {:?}",
        result.as_ref().err()
    );
    let p = result.unwrap();
    assert_eq!(p.proof().tables.len(), 5, "all five tables");
    assert_eq!(
        p.proof().resolved_writable.len() + p.proof().resolved_readonly.len(),
        55,
        "55 loaded keys"
    );
    for (compiled, expected) in frozen
        .native_message()
        .instructions
        .iter()
        .zip(&p.transaction().instructions)
    {
        assert_eq!(
            p.proof().full_account_keys[usize::from(compiled.program_id_index)].address,
            expected.program
        );
        for (index, meta) in compiled.accounts.iter().zip(&expected.accounts) {
            assert_eq!(
                &p.proof().full_account_keys[usize::from(*index)],
                meta,
                "real compiled account identity"
            );
        }
    }
    assert!(p.proof().compiled_instructions_match);
    use eplyx_engine::protocol::{kamino::KaminoKlendAdapter, ProtocolAdapter};
    assert!(
        KaminoKlendAdapter
            .accept(p.transaction())
            .unwrap_err()
            .to_string()
            .contains("lookup tables"),
        "ordinary path retains old gate"
    );
    assert_eq!(
        KaminoKlendAdapter
            .accept_reconstructed_message(&p)
            .unwrap_err()
            .to_string(),
        "replay selects successfully captured original transactions",
        "unchanged next blocker"
    );
}
#[test]
fn provenance_omits_credentials_and_lut_core_is_protocol_neutral() {
    for url in [
        "https://key@host",
        "https://host/token",
        "https://host?apikey=x",
    ] {
        let mut p = provenance();
        p.scheme_host = url.into();
        assert!(HistoricalAccountEvidence::from_response(
            &key(30),
            SLOT,
            p,
            &envelope(
                SLOT,
                &table_bytes(&[key(13)], LookupTableMeta::default()),
                &program::id().to_string()
            )
        )
        .is_err());
    }
    let source = include_str!("../src/message.rs");
    assert!(
        !source.contains("KLend") && !source.contains("kamino"),
        "generic source boundary"
    );
}

#[test]
fn deactivation_rejects_wrong_slot_hashes_owner_and_bank_context() {
    let (raw, mut tables) = fixture();
    replace_table(
        &mut tables,
        0,
        LookupTableMeta {
            deactivation_slot: 90,
            ..LookupTableMeta::default()
        },
    );
    let frozen = FrozenV0::from_rpc(&raw, GENESIS).unwrap();
    for (owner, slots) in [
        (key(99), vec![99, 90]),
        (message::SYSVAR_OWNER.into(), vec![100, 90]),
        (message::SYSVAR_OWNER.into(), vec![99, 99, 90]),
    ] {
        let hashes = SlotHashes::new(
            &slots
                .into_iter()
                .map(|s| (s, Hash::new_from_array([8; 32])))
                .collect::<Vec<_>>(),
        );
        let e = HistoricalAccountEvidence::from_response(
            message::SLOT_HASHES_ID,
            SLOT,
            provenance(),
            &envelope(SLOT, &wincode::serialize(&hashes).unwrap(), &owner),
        )
        .unwrap();
        let result = message::reconstruct(&frozen, &tables, Some(&e));
        assert!(result.is_err(), "invalid SlotHashes must fail closed");
        assert_eq!(result.unwrap_err().stage, 2);
    }
}

#[test]
fn duplicate_extra_missing_and_corrupt_table_artifacts_fail_closed() {
    let (raw, tables) = fixture();
    let mut duplicate = tables.clone();
    duplicate.push(tables[0].clone());
    assert_eq!(prove(&raw, &duplicate).unwrap_err().stage, 1);
    let mut extra = tables.clone();
    extra.push(evidence(&key(77), &[key(78)], LookupTableMeta::default()));
    assert_eq!(prove(&raw, &extra).unwrap_err().stage, 1);
    assert_eq!(prove(&raw, &tables[..1]).unwrap_err().stage, 1);
    let mut corrupt = tables.clone();
    corrupt[0].raw_response_sha256 = "0".repeat(64);
    assert_eq!(prove(&raw, &corrupt).unwrap_err().stage, 2);
}

// These fixtures are acquired historical production envelopes, kept separately
// from synthetic byte fixtures. No transport or current-state query runs here.
fn real_production_case(
    table_count: usize,
    loaded_count: usize,
) -> (Value, Vec<HistoricalAccountEvidence>) {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/examples/phase-u3b2-lut");
    let targets: Value =
        serde_json::from_slice(&std::fs::read(root.join("targets.json")).unwrap()).unwrap();
    assert_eq!(
        targets["sample_fingerprint"],
        "b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af"
    );
    let target = targets["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| {
            t["address_table_lookups"].as_array().unwrap().len() == table_count
                && t["rpc_loaded_writable"].as_array().unwrap().len()
                    + t["rpc_loaded_readonly"].as_array().unwrap().len()
                    == loaded_count
        })
        .unwrap();
    let sample = root.join("../phase-u3-baseline");
    let raw_bytes = std::fs::read(sample.join(target["capture_file"].as_str().unwrap())).unwrap();
    assert_eq!(hash_bytes(&raw_bytes), target["capture_sha256"]);
    let raw: Value = serde_json::from_slice(&raw_bytes).unwrap();
    let capture: Value =
        serde_json::from_slice(&std::fs::read(root.join("acquisition.json")).unwrap()).unwrap();
    let provider: ArchiveProvenance = serde_json::from_value(capture["provider"].clone()).unwrap();
    let validation =
        std::fs::read(root.join(capture["archive_validation_file"].as_str().unwrap())).unwrap();
    assert_eq!(hash_bytes(&validation), provider.validation_artifact_sha256);
    let slot = target["execution_slot"].as_u64().unwrap();
    let tables = target["address_table_lookups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|lookup| {
            let row = capture["requests"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["table_pubkey"] == lookup["accountKey"] && r["execution_slot"] == slot)
                .unwrap();
            assert_eq!(row["status"], "success");
            let body = std::fs::read(root.join(row["response_file"].as_str().unwrap())).unwrap();
            assert_eq!(hash_bytes(&body), row["response_sha256"]);
            let evidence = HistoricalAccountEvidence::from_response(
                lookup["accountKey"].as_str().unwrap(),
                slot,
                provider.clone(),
                &body,
            )
            .unwrap();
            assert_eq!(evidence.raw_account_sha256, row["decoded_account_sha256"]);
            let account = evidence.account(slot, GENESIS).unwrap();
            assert_eq!(account.owner, program::id().to_string());
            assert!(AddressLookupTable::deserialize(&account.data).is_ok());
            evidence
        })
        .collect();
    (raw["result"].clone(), tables)
}
fn assert_real_proof(table_count: usize, loaded_count: usize) {
    let (raw, tables) = real_production_case(table_count, loaded_count);
    let frozen = FrozenV0::from_rpc(&raw, GENESIS).unwrap();
    let result = message::reconstruct(&frozen, &tables, None);
    assert!(
        result.is_ok(),
        "assertion: real historical proof failed: {:?}",
        result.as_ref().err()
    );
    let proven = result.unwrap();
    let p = proven.proof();
    assert_eq!(
        p.tables.len(),
        table_count,
        "assertion: all historical tables contribute"
    );
    let writable: Vec<_> = p
        .resolved_writable
        .iter()
        .map(ToString::to_string)
        .collect();
    let readonly: Vec<_> = p
        .resolved_readonly
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(json!(writable), raw["meta"]["loadedAddresses"]["writable"]);
    assert_eq!(json!(readonly), raw["meta"]["loadedAddresses"]["readonly"]);
    assert_eq!(writable.len() + readonly.len(), loaded_count);
    let normalized = eplyx_engine::ingest::transactions::normalize(&raw).unwrap();
    assert_eq!(
        p.full_account_keys, normalized.account_keys,
        "assertion: complete requested key privileges"
    );
    assert!(p.compiled_instructions_match);
    for (ix, expected) in frozen
        .native_message()
        .instructions
        .iter()
        .zip(&normalized.instructions)
    {
        assert_eq!(
            p.full_account_keys[usize::from(ix.program_id_index)].address,
            expected.program
        );
        let accounts: Vec<_> = ix
            .accounts
            .iter()
            .map(|i| p.full_account_keys[usize::from(*i)].clone())
            .collect();
        assert_eq!(accounts, expected.accounts);
        assert_eq!(ix.data, expected.data);
    }
    let roundtrip = serde_json::from_slice(&serde_json::to_vec(p).unwrap()).unwrap();
    assert!(message::validate_proof(&frozen, &tables, None, &roundtrip).is_ok());
    assert!(matches!(
        proven.versioned_message(),
        solana_message::VersionedMessage::V0(_)
    ));
}
#[test]
fn real_single_table_historical_proof() {
    assert_real_proof(1, 13);
}
#[test]
fn real_five_table_historical_proof() {
    assert_real_proof(5, 52);
}
#[test]
fn real_five_table_55_loaded_historical_proof() {
    assert_real_proof(5, 55);
}
#[test]
fn real_historical_bytes_cannot_be_replaced_by_rpc_metadata() {
    let (raw, _) = real_production_case(5, 55);
    let result = prove(&raw, &[]);
    assert!(
        result.is_err(),
        "assertion: frozen RPC metadata substituted for historical evidence"
    );
    assert_eq!(result.unwrap_err().stage, 1);
}
#[test]
fn real_wrong_historical_bytes_are_rejected() {
    let (raw, mut tables) = real_production_case(5, 55);
    let account = tables[0]
        .account(raw["slot"].as_u64().unwrap(), GENESIS)
        .unwrap();
    let table = AddressLookupTable::deserialize(&account.data).unwrap();
    let mut addresses = table.addresses.to_vec();
    let requested = raw["transaction"]["message"]["addressTableLookups"][0]["writableIndexes"][0]
        .as_u64()
        .unwrap() as usize;
    addresses[requested] = key(99).parse().unwrap();
    let bytes = AddressLookupTable {
        meta: table.meta,
        addresses: Cow::Owned(addresses),
    }
    .serialize_for_tests()
    .unwrap();
    tables[0] = HistoricalAccountEvidence::from_response(
        &tables[0].pubkey,
        raw["slot"].as_u64().unwrap(),
        tables[0].provider.clone(),
        &envelope(
            raw["slot"].as_u64().unwrap(),
            &bytes,
            &program::id().to_string(),
        ),
    )
    .unwrap();
    let result = prove(&raw, &tables);
    assert!(
        result.is_err(),
        "assertion: wrong historical bytes accepted"
    );
    assert_eq!(result.unwrap_err().stage, 4);
}

#[test]
fn real_table_bytes_in_explicit_synthetic_warmup_context() {
    // Reuse real table bytes at their metadata boundary in a synthetic message
    // and synthetic envelope context. This is a visibility regression, never a
    // historical proof for another production slot or another transaction.
    let (real, tables) = real_production_case(1, 13);
    let account = tables[0]
        .account(real["slot"].as_u64().unwrap(), GENESIS)
        .unwrap();
    let table = AddressLookupTable::deserialize(&account.data).unwrap();
    let boundary = table.meta.last_extended_slot;
    let index = table.meta.last_extended_slot_start_index;
    assert!((index as usize) < table.addresses.len());
    let (mut raw, _) = fixture();
    raw["transaction"]["message"]["addressTableLookups"] =
        json!([{"accountKey":tables[0].pubkey,"writableIndexes":[index],"readonlyIndexes":[]}]);
    raw["transaction"]["message"]["instructions"][0]["accounts"] = json!([0, 3]);
    raw["meta"]["loadedAddresses"] =
        json!({"writable":[table.addresses[index as usize].to_string()],"readonly":[]});
    for (slot, expected_stage) in [
        (boundary - 1, Some(2)),
        (boundary, Some(3)),
        (boundary + 1, None),
    ] {
        raw["slot"] = json!(slot);
        let evidence = HistoricalAccountEvidence::from_response(
            &tables[0].pubkey,
            slot,
            tables[0].provider.clone(),
            &envelope(slot, &account.data, &program::id().to_string()),
        )
        .unwrap();
        let result = prove(&raw, &[evidence]);
        match expected_stage {
            Some(stage) => {
                assert!(
                    result.is_err(),
                    "assertion: real-byte visibility boundary ignored"
                );
                assert_eq!(result.unwrap_err().stage, stage);
            }
            None => assert!(
                result.is_ok(),
                "assertion: next-slot real-byte suffix should be usable"
            ),
        }
    }
}
