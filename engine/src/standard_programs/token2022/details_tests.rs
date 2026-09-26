use super::*;
use bytemuck::{bytes_of, Pod};
use solana_address::Address;
use spl_token_2022_interface::extension::{self as ext, Extension as SplExtension};

fn official<T: Pod + Default + SplExtension>() {
    let raw = T::default();
    let bytes = bytes_of(&raw);
    let kind: u16 = T::TYPE.into();
    let parsed = decode_checked_extension(kind, bytes)
        .ok()
        .expect("official layout");
    assert!(!matches!(parsed, Extension::Unrecognized { .. }));
    let mut overlong = bytes.to_vec();
    overlong.push(0);
    assert!(decode_checked_extension(kind, &overlong).is_malformed());
    if !bytes.is_empty() {
        assert!(decode_checked_extension(kind, &bytes[..bytes.len() - 1]).is_malformed());
    }
}

#[test]
fn every_fixed_extension_matches_the_pinned_official_layout() {
    official::<ext::transfer_fee::TransferFeeConfig>();
    official::<ext::transfer_fee::TransferFeeAmount>();
    official::<ext::mint_close_authority::MintCloseAuthority>();
    official::<ext::confidential_transfer::ConfidentialTransferMint>();
    official::<ext::confidential_transfer::ConfidentialTransferAccount>();
    official::<ext::default_account_state::DefaultAccountState>();
    official::<ext::immutable_owner::ImmutableOwner>();
    official::<ext::memo_transfer::MemoTransfer>();
    official::<ext::non_transferable::NonTransferable>();
    official::<ext::interest_bearing_mint::InterestBearingConfig>();
    official::<ext::cpi_guard::CpiGuard>();
    official::<ext::permanent_delegate::PermanentDelegate>();
    official::<ext::non_transferable::NonTransferableAccount>();
    official::<ext::transfer_hook::TransferHook>();
    official::<ext::transfer_hook::TransferHookAccount>();
    official::<ext::confidential_transfer_fee::ConfidentialTransferFeeConfig>();
    official::<ext::confidential_transfer_fee::ConfidentialTransferFeeAmount>();
    official::<ext::metadata_pointer::MetadataPointer>();
    official::<ext::group_pointer::GroupPointer>();
    official::<spl_token_group_interface::state::TokenGroup>();
    official::<ext::group_member_pointer::GroupMemberPointer>();
    official::<spl_token_group_interface::state::TokenGroupMember>();
    official::<ext::confidential_mint_burn::ConfidentialMintBurn>();
    official::<ext::scaled_ui_amount::ScaledUiAmountConfig>();
    official::<ext::pausable::PausableConfig>();
    official::<ext::pausable::PausableAccount>();
    official::<ext::permissioned_burn::PermissionedBurnConfig>();
}

#[test]
fn decoded_authorities_counters_and_schedules_come_from_the_official_fields() {
    let key = Address::new_from_array([3; 32]);
    let other = Address::new_from_array([4; 32]);
    let fee = ext::transfer_fee::TransferFeeConfig {
        transfer_fee_config_authority: key.into(),
        withdraw_withheld_authority: other.into(),
        withheld_amount: u64::MAX.into(),
        older_transfer_fee: ext::transfer_fee::TransferFee {
            epoch: 42.into(),
            maximum_fee: 7.into(),
            transfer_fee_basis_points: 25.into(),
        },
        newer_transfer_fee: ext::transfer_fee::TransferFee {
            epoch: 99.into(),
            maximum_fee: 19.into(),
            transfer_fee_basis_points: 150.into(),
        },
    };
    let decoded = decode_checked_extension(1, bytes_of(&fee)).ok().unwrap();
    assert_eq!(
        decoded,
        Extension::TransferFeeConfig {
            config_authority: Some(key.to_string()),
            withdraw_authority: Some(other.to_string()),
            withheld_amount: u64::MAX,
            older: TransferFee {
                epoch: 42,
                maximum_fee: 7,
                basis_points: 25
            },
            newer: TransferFee {
                epoch: 99,
                maximum_fee: 19,
                basis_points: 150
            },
        }
    );
    for (epoch, expected) in [(98, fee.older_transfer_fee), (99, fee.newer_transfer_fee)] {
        let schedule = decoded.transfer_fee(epoch).unwrap();
        for amount in [0, 1, 2, 66, 67, 100, 9999, u64::MAX] {
            assert_eq!(
                schedule.calculate_fee(amount),
                expected.calculate_fee(amount)
            );
        }
    }
    assert_eq!(
        serde_json::to_value(&decoded).unwrap()["withheld_amount"],
        u64::MAX.to_string()
    );
    let interest = ext::interest_bearing_mint::InterestBearingConfig {
        rate_authority: key.into(),
        initialization_timestamp: (-42).into(),
        pre_update_average_rate: (-7).into(),
        last_update_timestamp: 123.into(),
        current_rate: 25.into(),
    };
    assert_eq!(
        decode_checked_extension(10, bytes_of(&interest))
            .ok()
            .unwrap(),
        Extension::InterestBearingConfig {
            rate_authority: Some(key.to_string()),
            initialization_timestamp: -42,
            pre_update_average_rate: -7,
            last_update_timestamp: 123,
            current_rate: 25,
        }
    );
    let group = spl_token_group_interface::state::TokenGroup {
        update_authority: key.into(),
        mint: other,
        size: 9007199254740993_u64.into(),
        max_size: u64::MAX.into(),
    };
    assert_eq!(
        decode_checked_extension(21, bytes_of(&group)).ok().unwrap(),
        Extension::TokenGroup {
            update_authority: Some(key.to_string()),
            mint: other.to_string(),
            size: 9007199254740993,
            max_size: u64::MAX,
        }
    );
    let member = spl_token_group_interface::state::TokenGroupMember {
        mint: other,
        group: key,
        member_number: 91.into(),
    };
    assert_eq!(
        decode_checked_extension(23, bytes_of(&member))
            .ok()
            .unwrap(),
        Extension::TokenGroupMember {
            mint: other.to_string(),
            group: key.to_string(),
            member_number: 91,
        }
    );
}

#[test]
fn metadata_borsh_and_scaled_multiplier_bits_are_lossless() {
    let key = Address::new_from_array([8; 32]);
    let metadata = spl_token_metadata_interface::state::TokenMetadata {
        update_authority: key.into(),
        mint: key,
        name: "Fixture asset".into(),
        symbol: "FIX".into(),
        uri: "".into(),
        additional_metadata: vec![("key".into(), "value".into())],
    };
    let bytes = borsh::to_vec(&metadata).unwrap();
    assert_eq!(
        decode_checked_extension(19, &bytes).ok().unwrap(),
        Extension::TokenMetadata {
            update_authority: Some(key.to_string()),
            mint: key.to_string(),
            name: metadata.name,
            symbol: metadata.symbol,
            uri: metadata.uri,
            additional_metadata: metadata.additional_metadata,
        }
    );
    assert!(decode_checked_extension(19, &bytes[..bytes.len() - 1]).is_malformed());
    let mut scaled = ext::scaled_ui_amount::ScaledUiAmountConfig {
        authority: key.into(),
        ..Default::default()
    };
    scaled.multiplier.0 = 0x3ff0000000000000_u64.to_le_bytes();
    scaled.new_multiplier.0 = 0x4000000000000000_u64.to_le_bytes();
    scaled.new_multiplier_effective_timestamp = (-123).into();
    assert_eq!(
        decode_checked_extension(25, bytes_of(&scaled))
            .ok()
            .unwrap(),
        Extension::ScaledUiAmount {
            authority: Some(key.to_string()),
            multiplier_bits: 0x3ff0000000000000,
            new_multiplier_effective_timestamp: -123,
            new_multiplier_bits: 0x4000000000000000,
        }
    );
    for invalid in [
        0x7ff0000000000000_u64,
        0xfff0000000000000,
        0x7ff0000000000001,
    ] {
        scaled.new_multiplier.0 = invalid.to_le_bytes();
        assert!(decode_checked_extension(25, bytes_of(&scaled)).is_malformed());
    }
}

fn tlv(layout: Layout, entries: &[(u16, &[u8])]) -> Vec<u8> {
    let mut bytes = vec![0; TLV_START];
    bytes[ACCOUNT_TYPE_OFFSET] = if layout == Layout::Mint { 1 } else { 2 };
    for (kind, value) in entries {
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
        bytes.extend_from_slice(value);
    }
    bytes
}

#[test]
fn malformed_lists_and_unknown_entries_are_distinct_from_absence() {
    let mint = Layout::Mint;
    use solana_program_pack::Pack;
    let mut multisig_sized = vec![0; spl_token_2022_interface::state::Multisig::LEN];
    multisig_sized[ACCOUNT_TYPE_OFFSET] = 1;
    assert!(checked_extensions(&multisig_sized, mint).is_malformed());
    let valid = tlv(mint, &[(28, &[2; 32])]);
    assert!(matches!(
        checked_extensions(&valid, mint).ok().unwrap()[0],
        Extension::PermissionedBurn { .. }
    ));
    assert!(checked_extensions(&valid, Layout::Account).is_malformed());
    for entries in [
        vec![(7, &[][..])],
        vec![(9, &[][..]), (9, &[][..])],
        vec![(9, &[1][..])],
        vec![(14, &[0; 32][..])],
    ] {
        assert!(checked_extensions(&tlv(mint, &entries), mint).is_malformed());
    }
    let unknown = tlv(mint, &[(999, &[1, 2, 3])]);
    assert_eq!(
        checked_extensions(&unknown, mint).ok().unwrap(),
        vec![Extension::Unrecognized {
            kind: 999,
            bytes: vec![1, 2, 3]
        }]
    );
    let mut padded = valid.clone();
    padded.extend_from_slice(&[0; 7]);
    assert_eq!(
        checked_extensions(&padded, mint),
        checked_extensions(&valid, mint)
    );
    padded.push(1);
    assert!(checked_extensions(&padded, mint).is_malformed());
    let mut partial = valid;
    partial.push(1);
    assert!(checked_extensions(&partial, mint).is_malformed());
    assert!(checked_extensions(&tlv(mint, &[(1, &[1, 2])]), mint).is_malformed());
}

#[test]
fn confidential_fields_preserve_ciphertexts_and_large_counters() {
    let mut account = ext::confidential_transfer::ConfidentialTransferAccount {
        approved: true.into(),
        allow_non_confidential_credits: true.into(),
        ..Default::default()
    };
    account.elgamal_pubkey.0.fill(1);
    account.pending_balance_lo.0.fill(2);
    account.pending_balance_hi.0.fill(3);
    account.available_balance.0.fill(4);
    account.decryptable_available_balance.0.fill(5);
    account.pending_balance_credit_counter = 9007199254740993_u64.into();
    account.maximum_pending_balance_credit_counter = u64::MAX.into();
    account.expected_pending_balance_credit_counter = 27.into();
    account.actual_pending_balance_credit_counter = 26.into();
    use base64::{engine::general_purpose::STANDARD, Engine};
    assert_eq!(
        decode_checked_extension(5, bytes_of(&account))
            .ok()
            .unwrap(),
        Extension::ConfidentialTransferAccount {
            approved: true,
            elgamal_pubkey: STANDARD.encode([1; 32]),
            pending_balance_lo: STANDARD.encode([2; 64]),
            pending_balance_hi: STANDARD.encode([3; 64]),
            available_balance: STANDARD.encode([4; 64]),
            decryptable_available_balance: STANDARD.encode([5; 36]),
            allow_confidential_credits: false,
            allow_non_confidential_credits: true,
            pending_balance_credit_counter: 9007199254740993,
            maximum_pending_balance_credit_counter: u64::MAX,
            expected_pending_balance_credit_counter: 27,
            actual_pending_balance_credit_counter: 26,
        }
    );
}
