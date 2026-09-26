//! Local deterministic SPL fixtures are test inputs only, never production holders.
use anyhow::{ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use eplyx_engine::lifecycle::{
    decode::{self, LEGACY_PROGRAM, TOKEN_2022_PROGRAM},
    rpc::SolanaRpc,
    AssetDescriptor, EntityType, LifecycleSnapshot, LifecycleStateSource, SolanaTokenAssetSource,
};
use serde_json::{json, Value};
use solana_address::Address;
use solana_program_pack::Pack;
use spl_token_2022_interface::{
    extension::{self as ext, BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut},
    state::{Account, AccountState, Mint},
};
use std::{collections::VecDeque, sync::Mutex};

fn address(n: u8) -> Address {
    Address::new_from_array([n; 32])
}
fn owner_address() -> Address {
    // Compressed Edwards base point: deterministic and provably on-curve.
    let mut bytes = [0x66; 32];
    bytes[0] = 0x58;
    Address::new_from_array(bytes)
}
fn mint() -> Mint {
    Mint {
        supply: 123456789,
        decimals: 6,
        is_initialized: true,
        mint_authority: Some(address(3)).into(),
        freeze_authority: Some(address(4)).into(),
    }
}
fn token() -> Account {
    Account {
        mint: address(1),
        owner: owner_address(),
        amount: 123456789,
        state: AccountState::Initialized,
        ..Account::default()
    }
}
fn raw(bytes: &[u8], program: &str) -> Value {
    json!({"owner":program,"data":[STANDARD.encode(bytes),"base64"],"executable":false,
        "lamports":10000000,"rentEpoch":18446744073709551615u64,"space":bytes.len()})
}
fn packed<S: Pack>(s: S) -> Vec<u8> {
    let mut bytes = vec![0; S::LEN];
    S::pack(s, &mut bytes).unwrap();
    bytes
}
fn mint2022() -> Value {
    let types = [
        ExtensionType::TransferHook,
        ExtensionType::PermanentDelegate,
        ExtensionType::DefaultAccountState,
        ExtensionType::TransferFeeConfig,
        ExtensionType::Pausable,
        ExtensionType::ConfidentialTransferMint,
        ExtensionType::ScaledUiAmount,
        ExtensionType::MetadataPointer,
    ];
    let mut bytes = vec![0; ExtensionType::try_calculate_account_len::<Mint>(&types).unwrap()];
    let mut state = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut bytes).unwrap();
    state
        .init_extension::<ext::transfer_hook::TransferHook>(false)
        .unwrap()
        .program_id = Some(address(5)).try_into().unwrap();
    state
        .init_extension::<ext::permanent_delegate::PermanentDelegate>(false)
        .unwrap()
        .delegate = Some(address(6)).try_into().unwrap();
    state
        .init_extension::<ext::default_account_state::DefaultAccountState>(false)
        .unwrap()
        .state = AccountState::Frozen.into();
    state
        .init_extension::<ext::transfer_fee::TransferFeeConfig>(false)
        .unwrap()
        .newer_transfer_fee
        .transfer_fee_basis_points = 25u16.into();
    state
        .init_extension::<ext::pausable::PausableConfig>(false)
        .unwrap()
        .paused = true.into();
    state
        .init_extension::<ext::confidential_transfer::ConfidentialTransferMint>(false)
        .unwrap()
        .auto_approve_new_accounts = true.into();
    let scale = state
        .init_extension::<ext::scaled_ui_amount::ScaledUiAmountConfig>(false)
        .unwrap();
    scale.multiplier = 2.0.into();
    scale.new_multiplier = 3.0.into();
    state
        .init_extension::<ext::metadata_pointer::MetadataPointer>(false)
        .unwrap()
        .metadata_address = Some(address(1)).try_into().unwrap();
    state.base = mint();
    state.pack_base();
    state.init_account_type().unwrap();
    raw(&bytes, TOKEN_2022_PROGRAM)
}
fn token2022() -> Value {
    let types = [
        ExtensionType::ImmutableOwner,
        ExtensionType::TransferFeeAmount,
        ExtensionType::TransferHookAccount,
        ExtensionType::PausableAccount,
        ExtensionType::ConfidentialTransferAccount,
    ];
    let mut bytes = vec![0; ExtensionType::try_calculate_account_len::<Account>(&types).unwrap()];
    let mut state = StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut bytes).unwrap();
    state
        .init_extension::<ext::immutable_owner::ImmutableOwner>(false)
        .unwrap();
    state
        .init_extension::<ext::transfer_fee::TransferFeeAmount>(false)
        .unwrap()
        .withheld_amount = 55u64.into();
    state
        .init_extension::<ext::transfer_hook::TransferHookAccount>(false)
        .unwrap();
    state
        .init_extension::<ext::pausable::PausableAccount>(false)
        .unwrap();
    state
        .init_extension::<ext::confidential_transfer::ConfidentialTransferAccount>(false)
        .unwrap();
    state.base = Account {
        amount: 0,
        state: AccountState::Frozen,
        delegate: Some(address(7)).into(),
        delegated_amount: 42,
        close_authority: Some(address(8)).into(),
        ..token()
    };
    state.pack_base();
    state.init_account_type().unwrap();
    raw(&bytes, TOKEN_2022_PROGRAM)
}

#[test]
fn legacy_mint_and_account_decoding() {
    let m = decode::decode_mint(&raw(&packed(mint()), LEGACY_PROGRAM)).unwrap();
    assert!(!m.is_token_2022);
    assert_eq!(m.decimal_supply, "123.456789");
    assert_eq!(m.mint_authority, Some(address(3).to_string()));
    assert!(m.extensions.is_empty());
    let a = decode::decode_token_account(
        &raw(&packed(token()), LEGACY_PROGRAM),
        LEGACY_PROGRAM,
        &address(1).to_string(),
        6,
    )
    .unwrap();
    assert_eq!(a.raw_balance, "123456789");
    assert_eq!(a.ui_balance, "123.456789");
    assert_eq!(a.owner, owner_address().to_string());
    assert!(a.is_initialized);
    assert!(!a.is_frozen);
}
#[test]
fn token2022_mint_extensions_are_decoded_not_assumed() {
    let m = decode::decode_mint(&mint2022()).unwrap();
    assert!(m.is_token_2022);
    assert_eq!(m.extensions.len(), 8);
    let config = |name: &str| {
        &m.extensions
            .iter()
            .find(|e| e.extension_type == name)
            .unwrap()
            .config
    };
    assert_eq!(config("TransferHook")["programId"], address(5).to_string());
    assert_eq!(
        config("PermanentDelegate")["delegate"],
        address(6).to_string()
    );
    assert_eq!(config("DefaultAccountState")["state"], "Frozen");
    assert_eq!(config("Pausable")["paused"], true);
    assert_eq!(
        config("ScaledUiAmount")["multiplierBits"],
        0x4000000000000000u64.to_string()
    );
    let bare = decode::decode_mint(&raw(&packed(mint()), TOKEN_2022_PROGRAM)).unwrap();
    assert!(bare.extensions.is_empty());
}
#[test]
fn delegated_frozen_zero_and_account_extensions_survive_normalization() {
    let a =
        decode::decode_token_account(&token2022(), TOKEN_2022_PROGRAM, &address(1).to_string(), 6)
            .unwrap();
    assert_eq!(a.extensions.len(), 5);
    assert_eq!(a.raw_balance, "0");
    assert_eq!(a.ui_balance, "0");
    assert!(a.is_frozen && a.is_initialized && a.has_active_delegate);
    assert_eq!(a.delegate, Some(address(7).to_string()));
    assert_eq!(a.delegated_amount, "42");
    assert_eq!(a.close_authority, Some(address(8).to_string()));
    let fee = a
        .extensions
        .iter()
        .find(|e| e.extension_type == "TransferFeeAmount")
        .unwrap();
    assert_eq!(fee.config["withheldAmount"], "55");
    assert!(a
        .extensions
        .iter()
        .any(|e| e.extension_type == "ConfidentialTransferAccount"));
}
#[test]
fn uninitialized_accounts_are_preserved_and_zero_allowance_is_not_active() {
    for program in [LEGACY_PROGRAM, TOKEN_2022_PROGRAM] {
        let base = Account {
            state: AccountState::Uninitialized,
            delegate: Some(address(7)).into(),
            delegated_amount: 0,
            ..token()
        };
        let a = decode::decode_token_account(
            &raw(&packed(base), program),
            program,
            &address(1).to_string(),
            6,
        )
        .unwrap();
        assert!(!a.is_initialized && !a.has_active_delegate && !a.is_frozen);
        assert!(a.delegate.is_some());
    }
}
#[test]
fn malformed_unsupported_and_wrong_mint_fail_explicitly() {
    assert!(decode::decode_mint(&raw(&[0; 81], TOKEN_2022_PROGRAM)).is_err());
    assert!(decode::decode_mint(&raw(&packed(mint()), &address(9).to_string())).is_err());
    assert!(decode::decode_mint(&raw(&packed(mint()), LEGACY_PROGRAM)).is_ok());
    assert!(
        decode::decode_token_account(&token2022(), LEGACY_PROGRAM, &address(1).to_string(), 6)
            .is_err()
    );
    assert!(decode::decode_token_account(
        &token2022(),
        TOKEN_2022_PROGRAM,
        &address(10).to_string(),
        6
    )
    .is_err());
    for variant in 0..6 {
        let mut bytes = STANDARD
            .decode(mint2022()["data"][0].as_str().unwrap())
            .unwrap();
        match variant {
            0 => bytes[166..168].copy_from_slice(&65000u16.to_le_bytes()), // unknown extension
            1 => bytes[168..170].copy_from_slice(&65000u16.to_le_bytes()), // truncated TLV
            2 => bytes[165] = 2,                                           // wrong account type
            3 => bytes.truncate(166), // all TLVs removed: valid empty allocation
            4 => bytes[166..168].copy_from_slice(&2u16.to_le_bytes()), // account extension on mint
            _ => bytes.push(99),      // garbage trailing data
        }
        if variant == 3 {
            assert!(decode::decode_mint(&raw(&bytes, TOKEN_2022_PROGRAM)).is_ok());
        } else {
            assert!(
                decode::decode_mint(&raw(&bytes, TOKEN_2022_PROGRAM)).is_err(),
                "variant {variant}"
            );
        }
    }
    let mut bytes = packed(mint());
    bytes[45] = 2;
    assert!(decode::decode_mint(&raw(&bytes, LEGACY_PROGRAM)).is_err());
}
#[test]
fn exact_decimal_amounts_do_not_use_floats_or_overflow() {
    assert_eq!(decode::decimal_amount(u64::MAX, 9), "18446744073.709551615");
    assert_eq!(decode::decimal_amount(1, 255).len(), 257);
    assert_eq!(decode::decimal_amount(0, 255), "0");
}

struct MockRpc {
    responses: Mutex<VecDeque<(&'static str, Value)>>,
}
impl SolanaRpc for MockRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let (expected, result) = self.responses.lock().unwrap().pop_front().unwrap();
        ensure!(expected == method, "unexpected method");
        if method == "getProgramAccounts" {
            assert!(params[1].get("dataSize").is_none());
            assert_eq!(params[1]["filters"].as_array().unwrap().len(), 1);
        }
        Ok(result)
    }
}
fn capture(owner_raw: Value) -> LifecycleSnapshot {
    let contextual = |slot: u64, value: Value| json!({"context":{"slot":slot},"value":value});
    let responses = VecDeque::from([
        ("getGenesisHash", json!("test-genesis")),
        ("getAccountInfo", contextual(100, mint2022())),
        (
            "getProgramAccounts",
            contextual(
                101,
                json!([{"pubkey":address(11).to_string(),"account":token2022()}]),
            ),
        ),
        ("getMultipleAccounts", contextual(102, json!([owner_raw]))),
        ("getAccountInfo", contextual(103, mint2022())),
    ]);
    SolanaTokenAssetSource {
        rpc: MockRpc {
            responses: Mutex::new(responses),
        },
    }
    .capture(AssetDescriptor {
        name: "Deterministic test asset".into(),
        mint: address(1).to_string(),
        expected_token_program: Some(TOKEN_2022_PROGRAM.into()),
        expected_genesis_hash: Some("test-genesis".into()),
        verification: vec![],
    })
    .unwrap()
}
#[test]
fn snapshot_json_roundtrip_and_offline_evidence_replay_are_deterministic() {
    let s = capture(Value::Null);
    let encoded = s.to_json().unwrap();
    let decoded: LifecycleSnapshot = serde_json::from_str(&encoded).unwrap();
    decoded.validate().unwrap();
    assert_eq!(encoded, decoded.to_json().unwrap());
    assert_eq!(decoded.summary.active_delegated, 1);
    assert_eq!(decoded.summary.frozen, 1);
    assert_eq!(decoded.summary.unknown, 1);
    assert_eq!(decoded.source.min_observed_slot, 100);
    assert_eq!(decoded.source.max_observed_slot, 103);
    assert_eq!(decoded.source.consistency, "MultipleRpcContexts");
    assert_eq!(decoded.entities[0].token_account_evidence.slot, 101);
    assert_eq!(decoded.entities[0].authority_evidence.slot, 102);
    let path = std::env::temp_dir().join(format!(
        "eplyx-snapshot-roundtrip-{}.json",
        std::process::id()
    ));
    s.save(&path).unwrap();
    assert!(s.save(&path).is_err());
    assert_eq!(LifecycleSnapshot::load(&path).unwrap(), s);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn entity_classification_uses_authority_evidence_and_does_not_guess_pda_programs() {
    let missing = capture(Value::Null);
    assert_eq!(missing.entities[0].entity_type, EntityType::Unknown);
    let program = capture(raw(&[4; 100], &address(12).to_string()));
    assert_eq!(
        program.entities[0].entity_type,
        EntityType::ProgramOwnedAuthority
    );
    let mut executable = raw(&[4; 100], &address(12).to_string());
    executable["executable"] = json!(true);
    assert_eq!(
        capture(executable).entities[0].entity_type,
        EntityType::Unknown
    );
    let system = capture(raw(&[], "11111111111111111111111111111111"));
    assert!(owner_address().is_on_curve());
    assert_eq!(system.entities[0].entity_type, EntityType::WalletCompatible);
    let special = capture(raw(&[0; 80], "11111111111111111111111111111111"));
    assert_eq!(special.entities[0].entity_type, EntityType::Unknown);
}

#[test]
fn spl_multisig_authority_preserves_threshold_and_signers() {
    use spl_token_2022_interface::state::Multisig;
    let mut m = Multisig {
        m: 2,
        n: 2,
        is_initialized: true,
        ..Multisig::default()
    };
    m.signers[0] = address(20);
    m.signers[1] = address(21);
    let s = capture(raw(&packed(m), LEGACY_PROGRAM));
    assert_eq!(s.entities[0].entity_type, EntityType::TokenMultisig);
    let config = s.entities[0]
        .authority_observation
        .multisig
        .as_ref()
        .unwrap();
    assert_eq!(config["required_signers"], 2);
    assert_eq!(config["signers"][1], address(21).to_string());
}

#[test]
fn duplicate_extensions_and_invalid_marker_lengths_are_rejected() {
    let mut bytes = STANDARD
        .decode(mint2022()["data"][0].as_str().unwrap())
        .unwrap();
    // First TransferHook has 64-byte payload, followed by PermanentDelegate.
    bytes[234..236].copy_from_slice(&14u16.to_le_bytes());
    assert!(decode::decode_mint(&raw(&bytes, TOKEN_2022_PROGRAM)).is_err());
    let mut bytes = STANDARD
        .decode(token2022()["data"][0].as_str().unwrap())
        .unwrap();
    bytes[168..170].copy_from_slice(&1u16.to_le_bytes());
    assert!(decode::decode_token_account(
        &raw(&bytes, TOKEN_2022_PROGRAM),
        TOKEN_2022_PROGRAM,
        &address(1).to_string(),
        6
    )
    .is_err());
}
#[test]
fn offline_replay_rejects_tampering_incomplete_evidence_and_bad_context() {
    let s = capture(Value::Null);
    let mut bad = s.clone();
    bad.entities[0].state.raw_balance = "999".into();
    assert!(bad.validate().is_err());
    let mut bad = s.clone();
    bad.evidence[3].result["value"] = json!([]);
    assert!(bad.validate().is_err());
    let mut bad = s.clone();
    bad.evidence[3].result["context"]["slot"] = json!(99);
    assert!(bad.validate().is_err());
    let mut bad = s.clone();
    bad.evidence[2].params[1]["filters"] = json!([]);
    assert!(bad.validate().is_err());
    let mut bad = s.clone();
    bad.asset.expected_genesis_hash = Some("other-chain".into());
    assert!(bad.validate().is_err());
    let mut bad = s;
    bad.schema_version = 2;
    assert!(bad.validate().is_err());
}
#[test]
fn rpc_url_credentials_are_not_retained_in_origin() {
    let captured = capture(serde_json::Value::Null);
    assert_eq!(captured.source.rpc_origin, "configured_read_only_provider");
    assert!(!captured.to_json().unwrap().contains("://"));
}

#[test]
fn real_spacex_mint_fixture_decodes_all_live_extensions_and_embedded_metadata() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/fixtures/token-2022-spacex-mint.json"
    ))
    .unwrap();
    let mint = decode::decode_mint(&fixture["account"]).unwrap();
    assert_eq!(mint.decimals, 9);
    assert_eq!(mint.raw_supply, "8742515833967");
    let names: Vec<_> = mint
        .extensions
        .iter()
        .map(|e| e.extension_type.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "TransferFeeConfig",
            "ConfidentialTransferMint",
            "DefaultAccountState",
            "PermanentDelegate",
            "TransferHook",
            "ConfidentialTransferFeeConfig",
            "MetadataPointer",
            "TokenMetadata",
            "ScaledUiAmount",
            "Pausable"
        ]
    );
    let metadata = mint
        .extensions
        .iter()
        .find(|e| e.extension_type == "TokenMetadata")
        .unwrap();
    assert_eq!(metadata.config["mint"], fixture["mint"]);
    assert_eq!(metadata.config["name"], "SpaceX PreStocks");
    assert_eq!(metadata.config["symbol"], "SPACEX");
    assert_eq!(
        metadata.config["uri"],
        "https://prestocks.com/metadata/spacex.json"
    );
    let hook = mint
        .extensions
        .iter()
        .find(|e| e.extension_type == "TransferHook")
        .unwrap();
    assert!(hook.config["programId"].is_null());
    let paused = mint
        .extensions
        .iter()
        .find(|e| e.extension_type == "Pausable")
        .unwrap();
    assert_eq!(paused.config["paused"], false);
}
