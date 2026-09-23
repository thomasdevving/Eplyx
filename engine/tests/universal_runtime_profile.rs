use std::collections::BTreeMap;

use eplyx_engine::{
    ingest::transactions::HistoricalTransaction,
    types::AccountSnapshot,
    universal::{
        execution::{
            ExecutionBackend, ExecutionRequest, HistoricalRuntimeEvidence, LiteSvmBackend,
            RuntimeProfile, SlotHashesVariant,
        },
        model::ResolvedMessage,
    },
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_hash::Hash;
use solana_message::{Message, VersionedMessage};
use solana_system_interface::instruction::advance_nonce_account;

const RECENT: &str = "SysvarRecentB1ockHashes11111111111111111111";
const SYSVAR_OWNER: &str = "Sysvar1111111111111111111111111111111111111";
const SYSTEM: &str = "11111111111111111111111111111111";

fn durable_nonce(blockhash: &Hash) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(b"DURABLE_NONCE");
    hasher.update(blockhash.to_bytes());
    Hash::new_from_array(hasher.finalize().into())
}

fn nonce_data(authority: &Address, nonce: &Hash) -> Vec<u8> {
    let mut data = Vec::with_capacity(80);
    data.extend_from_slice(&1u32.to_le_bytes()); // current version
    data.extend_from_slice(&1u32.to_le_bytes()); // initialized state
    data.extend_from_slice(authority.as_ref());
    data.extend_from_slice(nonce.as_ref());
    data.extend_from_slice(&5_000u64.to_le_bytes());
    assert_eq!(data.len(), 80);
    data
}

fn recent_blockhashes_data(hash: &Hash) -> Vec<u8> {
    let mut data = Vec::with_capacity(48);
    data.extend_from_slice(&1u64.to_le_bytes());
    data.extend_from_slice(hash.as_ref());
    data.extend_from_slice(&5_000u64.to_le_bytes());
    data
}

fn resolved_message(
    nonce_address: Address,
    authority: Address,
    stored_nonce: Hash,
) -> ResolvedMessage {
    let instruction = advance_nonce_account(&nonce_address, &authority);
    let mut message = Message::new(&[instruction], Some(&authority));
    message.recent_blockhash = stored_nonce;
    let account_keys = message
        .account_keys
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    ResolvedMessage {
        message: VersionedMessage::Legacy(message),
        transaction: HistoricalTransaction {
            signature: String::new(),
            slot: 900,
            block_time: None,
            version: "legacy".into(),
            recent_blockhash: stored_nonce.to_string(),
            payer: authority.to_string(),
            account_keys: Vec::new(),
            loaded_address_count: 0,
            instructions: Vec::new(),
            inner_instructions: Vec::new(),
            inner_instruction_frames: Vec::new(),
            success: true,
            error: None::<Value>,
            fee: 5_000,
            compute_units: None,
            pre_balances: None,
            post_balances: None,
            pre_token_balances: None,
            post_token_balances: None,
            native_value_lamports: None,
            logs: Vec::new(),
        },
        account_keys,
    }
}

fn runtime_profile(
    environment_blockhash: Hash,
    sysvars: &BTreeMap<String, AccountSnapshot>,
) -> RuntimeProfile {
    let evidence = HistoricalRuntimeEvidence::new(
        environment_blockhash.to_string(),
        RuntimeProfile::sysvar_snapshot_hash(sysvars).unwrap(),
        "LiteSVM 0.16.0 mainnet".into(),
        "agave-4.2.2-native-system-compute".into(),
        "content-addressed-test-evidence".into(),
    )
    .unwrap();
    RuntimeProfile::resolve(
        Some(&evidence),
        sysvars,
        "LiteSVM 0.16.0 mainnet",
        false,
        false,
        "runtime_generated_from_complete_message",
        "historical",
    )
    .unwrap()
}

fn execute_nonce(environment_blockhash: Hash) -> (Hash, String) {
    let nonce_address = Address::new_from_array([9; 32]);
    let authority = Address::new_from_array([7; 32]);
    let prior_environment = Hash::new_from_array([3; 32]);
    let stored_nonce = durable_nonce(&prior_environment);
    let message = resolved_message(nonce_address, authority, stored_nonce);
    let mut seeds = BTreeMap::new();
    seeds.insert(
        nonce_address.to_string(),
        AccountSnapshot {
            lamports: 2_000_000,
            owner: SYSTEM.into(),
            data: nonce_data(&authority, &stored_nonce),
            executable: false,
            rent_epoch: u64::MAX,
        },
    );
    seeds.insert(
        authority.to_string(),
        AccountSnapshot {
            lamports: 1_000_000,
            owner: SYSTEM.into(),
            data: Vec::new(),
            executable: false,
            rent_epoch: u64::MAX,
        },
    );
    let mut sysvars = BTreeMap::new();
    sysvars.insert(
        RECENT.into(),
        AccountSnapshot {
            lamports: 1,
            owner: SYSVAR_OWNER.into(),
            data: recent_blockhashes_data(&environment_blockhash),
            executable: false,
            rent_epoch: u64::MAX,
        },
    );
    let profile = runtime_profile(environment_blockhash, &sysvars);
    let evidence = LiteSvmBackend
        .execute(&ExecutionRequest {
            message: &message,
            seeds: &seeds,
            absent_pre_accounts: &[],
            watched: &[nonce_address.to_string()],
            runtime_sysvars: &sysvars,
            clock: None,
            programs_to_load: &[],
            runtime_profile: &profile,
            unlimited_logs: true,
            slot_hashes: SlotHashesVariant::BackendDefault,
            recent_blockhashes: Default::default(),
            require_complete_state: false,
        })
        .unwrap();
    assert!(evidence.success, "{:?}", evidence.error);
    let data = &evidence.post_accounts[&nonce_address.to_string()]
        .as_ref()
        .unwrap()
        .data;
    let next_nonce = Hash::new_from_array(data[40..72].try_into().unwrap());
    (next_nonce, profile.profile_id)
}

#[test]
fn environment_blockhash_controls_exact_durable_nonce() {
    let first_environment = Hash::new_from_array([11; 32]);
    let second_environment = Hash::new_from_array([12; 32]);
    let (first_nonce, first_profile) = execute_nonce(first_environment);
    let (second_nonce, second_profile) = execute_nonce(second_environment);
    assert_eq!(first_nonce, durable_nonce(&first_environment));
    assert_eq!(second_nonce, durable_nonce(&second_environment));
    assert_ne!(first_nonce, second_nonce);
    assert_ne!(first_profile, second_profile);
}

#[test]
fn runtime_evidence_and_profile_mutations_fail_closed() {
    let mut sysvars = BTreeMap::new();
    sysvars.insert(
        RECENT.into(),
        AccountSnapshot {
            lamports: 1,
            owner: SYSVAR_OWNER.into(),
            data: recent_blockhashes_data(&Hash::new_from_array([5; 32])),
            executable: false,
            rent_epoch: u64::MAX,
        },
    );
    let mut evidence = HistoricalRuntimeEvidence::new(
        Hash::new_from_array([6; 32]).to_string(),
        RuntimeProfile::sysvar_snapshot_hash(&sysvars).unwrap(),
        "LiteSVM 0.16.0 mainnet".into(),
        "agave-4.2.2-native-system-compute".into(),
        "content-addressed-test-evidence".into(),
    )
    .unwrap();
    evidence.environment_blockhash = Hash::new_from_array([7; 32]).to_string();
    assert!(evidence.validate().is_err());

    let valid = HistoricalRuntimeEvidence::new(
        Hash::new_from_array([6; 32]).to_string(),
        RuntimeProfile::sysvar_snapshot_hash(&sysvars).unwrap(),
        "LiteSVM 0.16.0 mainnet".into(),
        "agave-4.2.2-native-system-compute".into(),
        "content-addressed-test-evidence".into(),
    )
    .unwrap();
    sysvars.get_mut(RECENT).unwrap().data[8] ^= 1;
    assert!(RuntimeProfile::resolve(
        Some(&valid),
        &sysvars,
        "LiteSVM 0.16.0 mainnet",
        false,
        false,
        "runtime_generated_from_complete_message",
        "historical",
    )
    .is_err());
}
