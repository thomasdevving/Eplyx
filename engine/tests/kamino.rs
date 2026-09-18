//! Phase U2 acceptance: does the third adapter actually consume Phase U1?
//!
//! The Kamino adapter's own unit tests prove it behaves correctly. These prove
//! the *architectural* claim, which is a different thing: that it consumes the
//! universal evidence layer rather than rebuilding it, and that it cannot
//! quietly start rebuilding it later.
//!
//! An architectural boundary nothing enforces is a comment. Two of the tests
//! below read the adapter's own source, which is the only way to assert the
//! absence of a reimplementation — a behavioural test passes just as happily
//! whether a token account was decoded by the shared decoder or by a private
//! copy of the same offsets.

use eplyx_engine::{
    evidence::token::TokenProgram,
    protocol::{adapter_for, kamino},
    standard_programs::{spl_token, token2022},
};

const ADAPTER_SOURCE: &str = include_str!("../src/protocol/kamino/mod.rs");
const STATE_SOURCE: &str = include_str!("../src/protocol/kamino/state.rs");
const FRACTION_SOURCE: &str = include_str!("../src/protocol/kamino/fraction.rs");

/// Source with comments and doc comments removed.
///
/// The prohibitions below are about code. A doc comment that *mentions* the
/// 165-byte token layout while explaining why the adapter does not parse it is
/// exactly the documentation this phase wants, and a naive scan would forbid it.
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// §23, §46. The adapter must not carry a token layout of its own.
///
/// The three numbers below are the SPL Token account length, the mint length,
/// and the amount offset. Their appearance in this adapter would mean a fourth
/// copy of a layout Phase U1 exists to own once.
#[test]
fn the_kamino_adapter_contains_no_token_layout_of_its_own() {
    for (name, source) in [
        ("mod.rs", ADAPTER_SOURCE),
        ("state.rs", STATE_SOURCE),
        ("fraction.rs", FRACTION_SOURCE),
    ] {
        let code = code_only(source);
        for forbidden in ["165", "= 82", "data[64..72]", "data[0..32]", "data[32..64]"] {
            assert!(
                !code.contains(forbidden),
                "{name} contains {forbidden:?}: the SPL Token layout belongs to \
                 standard_programs::spl_token, and a private copy is the machinery tax \
                 Phase U1 removed"
            );
        }
        // Token-2022's TLV walk likewise.
        for forbidden in [
            "ACCOUNT_TYPE_OFFSET:",
            "fn extensions(",
            "fn extension_name(",
        ] {
            assert!(
                !code.contains(forbidden),
                "{name} contains {forbidden:?}: Token-2022 extension parsing is shared"
            );
        }
    }
}

/// §9, §46. The adapter must not carry its own boundary proof or pairing walk.
///
/// Both were extracted in Phase U1 precisely because the first two adapters had
/// near-identical copies. A third copy here would mean the extraction bought
/// nothing for the case it was meant to serve.
#[test]
fn the_kamino_adapter_delegates_proof_and_pairing_to_the_shared_layer() {
    let code = code_only(ADAPTER_SOURCE);

    assert!(
        code.contains("boundary::prove("),
        "the adapter must prove boundaries through evidence::boundary"
    );
    assert!(
        code.contains("pairing::compare_decoded("),
        "the adapter must pair executions through evidence::pairing"
    );
    assert!(
        code.contains("labels::assign("),
        "the adapter must assign labels through evidence::labels"
    );

    // And must not have rebuilt them. These are the shapes the extracted code
    // had: a per-side loop over validator balances, and a hand-rolled index
    // lookup into the message's account keys.
    for forbidden in [
        "pre_balances",
        "post_balances",
        "account_keys\n.iter()\n.position(",
        "for named in pre",
        "for named in post",
    ] {
        assert!(
            !code.contains(forbidden),
            "the adapter appears to reimplement boundary proof ({forbidden:?}); \
             evidence::boundary owns it"
        );
    }
}

/// §32. No floating point anywhere in the Kamino path.
///
/// A scaled fraction is the one place a lending protocol most invites a float,
/// and converting `U68F60` through an `f64` would silently lose the low bits of
/// every large debt.
#[test]
fn the_kamino_path_uses_no_floating_point() {
    for (name, source) in [
        ("mod.rs", ADAPTER_SOURCE),
        ("state.rs", STATE_SOURCE),
        ("fraction.rs", FRACTION_SOURCE),
    ] {
        let code = code_only(source);
        for forbidden in ["f64", "f32", "as f", "powf", "sqrt"] {
            assert!(
                !code.contains(forbidden),
                "{name} contains {forbidden:?}; the debt path is integer throughout"
            );
        }
    }
}

/// §23. Behavioural confirmation of the same boundary: the adapter decodes a
/// token account under *either* token program, which it could only do by going
/// through the shared dispatch.
#[test]
fn the_adapter_reads_token_accounts_under_both_token_programs() {
    let adapter = adapter_for(kamino::PROGRAM_ID).expect("registered");

    let mut legacy = vec![0_u8; spl_token::ACCOUNT_LEN];
    legacy[0..32].copy_from_slice(&[6_u8; 32]);
    legacy[32..64].copy_from_slice(&[1_u8; 32]);
    legacy[64..72].copy_from_slice(&7_777_u64.to_le_bytes());
    legacy[108] = 1;

    let mut extended = legacy.clone();
    extended.resize(token2022::ACCOUNT_TYPE_OFFSET, 0);
    extended.push(2);
    extended.extend_from_slice(&token2022::TRANSFER_FEE_AMOUNT.to_le_bytes());
    extended.extend_from_slice(&8_u16.to_le_bytes());
    extended.extend_from_slice(&11_u64.to_le_bytes());

    let decode = |owner: &str, data: Vec<u8>| {
        adapter
            .decode(&eplyx_engine::types::AccountSnapshot {
                lamports: 1,
                owner: owner.into(),
                data,
                executable: false,
                rent_epoch: 0,
            })
            .and_then(|account| account.field("amount").map(|field| field.value.render()))
    };

    assert_eq!(
        decode(spl_token::PROGRAM_ID, legacy.clone()),
        Some("7777".into())
    );
    assert_eq!(
        decode(token2022::PROGRAM_ID, extended.clone()),
        Some("7777".into())
    );
    // And the narrower program refuses the wider layout, which is the shared
    // decoder's strictness showing through rather than the adapter's.
    assert_eq!(decode(spl_token::PROGRAM_ID, extended), None);
    assert_eq!(
        TokenProgram::of(spl_token::PROGRAM_ID)
            .unwrap()
            .account_amount(&legacy),
        Some(7_777)
    );
}

/// §27. Coverage is exact. A supported borrow says nothing about repaying,
/// liquidating, or withdrawing collateral, and the adapter must refuse every
/// KLend instruction outside its two families.
#[test]
fn the_adapter_claims_only_the_two_families_it_supports() {
    let adapter = adapter_for(kamino::PROGRAM_ID).expect("registered");
    // A transaction whose only KLend instruction is a repay is not accepted,
    // and not because of its shape — because the action is unrecognised.
    let repay: eplyx_engine::ingest::transactions::HistoricalTransaction =
        serde_json::from_str(&sample_transaction(
            // repayObligationLiquidity
            "[145, 178, 13, 225, 76, 240, 147, 72]",
        ))
        .expect("transaction");
    let error = adapter.accept(&repay).expect_err("must refuse").to_string();
    assert!(
        error.contains("no supported Kamino KLend action"),
        "{error}"
    );
}

fn sample_transaction(discriminator: &str) -> String {
    let disc: Vec<u8> = serde_json::from_str(discriminator).expect("discriminator");
    let mut data = disc;
    data.extend_from_slice(&1_000_u64.to_le_bytes());
    let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        r#"{{"signature":"sig","slot":1,"block_time":null,"version":"legacy",
             "recent_blockhash":"b","payer":"11111111111111111111111111111112",
             "account_keys":[{{"address":"11111111111111111111111111111112","is_signer":true,"is_writable":true}}],
             "instructions":[{{"program":"{program}","accounts":[],"data":"{hex}"}}],
             "inner_instructions":[],"success":true,"error":null,"fee":5000,
             "compute_units":null,"logs":[]}}"#,
        program = kamino::PROGRAM_ID,
    )
}

/// §7. Every supported discriminator is pinned to Anchor's own derivation.
///
/// The adapter's unit test compares `from_discriminator(&BORROW_V1)` against
/// `BorrowV1`, which is true however the constant is written — mutating the
/// constant keeps it passing. This computes the value independently, from the
/// instruction name the IDL declares, so the constants are pinned to the
/// protocol rather than to themselves. Mutation 6 of the phase's suite is
/// exactly that drift, and only this test catches it.
#[test]
fn every_supported_discriminator_matches_anchors_derivation() {
    use sha2::{Digest, Sha256};

    fn anchor_discriminator(snake_name: &str) -> [u8; 8] {
        let digest = Sha256::digest(format!("global:{snake_name}").as_bytes());
        digest[..8].try_into().expect("eight bytes")
    }

    // The four instructions this adapter admits, by the names `kamino_lending`
    // 1.25.0 declares for them.
    for (name, accepted) in [
        ("deposit_reserve_liquidity_and_obligation_collateral", true),
        (
            "deposit_reserve_liquidity_and_obligation_collateral_v2",
            true,
        ),
        ("borrow_obligation_liquidity", true),
        ("borrow_obligation_liquidity_v2", true),
    ] {
        let discriminator = anchor_discriminator(name);
        let mut data = discriminator.to_vec();
        data.extend_from_slice(&1_000_u64.to_le_bytes());
        assert_eq!(
            kamino::recognises(&data),
            accepted,
            "{name} -> {discriminator:?} is not recognised as a supported action"
        );
    }

    // The prerequisites are recognised as prerequisites, not as actions.
    for name in ["refresh_reserve", "refresh_obligation"] {
        let mut data = anchor_discriminator(name).to_vec();
        data.extend_from_slice(&1_000_u64.to_le_bytes());
        assert!(
            !kamino::recognises(&data),
            "{name} is a prerequisite, never an action"
        );
    }

    // And a sample of KLend instructions outside the slice stay unsupported.
    for name in [
        "repay_obligation_liquidity",
        "liquidate_obligation_and_redeem_reserve_collateral",
        "withdraw_obligation_collateral",
        "flash_borrow_reserve_liquidity",
        "init_obligation",
    ] {
        let mut data = anchor_discriminator(name).to_vec();
        data.extend_from_slice(&1_000_u64.to_le_bytes());
        assert!(!kamino::recognises(&data), "{name} must stay unsupported");
    }
}

/// §6. The interface this adapter was written against is recorded, and what it
/// is worth is recorded with it.
#[test]
fn the_interface_provenance_is_recorded_and_not_overclaimed() {
    let interface = kamino::state::INTERFACE;
    assert_eq!(interface.program_id, kamino::PROGRAM_ID);
    assert_eq!(interface.idl_name, "kamino_lending");
    assert_eq!(interface.idl_version, "1.25.0");
    assert_eq!(interface.idl_sha256.len(), 64);
    assert!(interface.idl_sha256.chars().all(|c| c.is_ascii_hexdigit()));

    // The claim is about an interface, never about bytes. Nothing in the
    // adapter may assert that source matches the deployed binary.
    let code = code_only(STATE_SOURCE);
    for forbidden in [
        "ExactVerifiedBuild",
        "verified_build",
        "source_hash_matches",
    ] {
        assert!(
            !code.contains(forbidden),
            "the adapter must not claim source-to-bytecode equivalence it has not established"
        );
    }
}

/// §25. The coarse vocabulary was not extended to make Kamino fit.
#[test]
fn no_new_coarse_semantic_action_was_added_for_kamino() {
    use eplyx_engine::protocol::SemanticAction;
    assert_eq!(
        SemanticAction::ALL.len(),
        14,
        "Phase U2 must not add a coarse action variant; the pressure is documented instead"
    );
    assert!(SemanticAction::parse("borrow").is_none());
    assert!(SemanticAction::parse("repay").is_none());
}

/// §4, §54. No adapter framework, DSL or contract v2 was introduced.
#[test]
fn no_adapter_specification_language_was_introduced() {
    let root = std::path::Path::new("../engine/src");
    for forbidden in ["contract", "adapter_spec", "dsl", "idl"] {
        let path = root.join(format!("protocol/{forbidden}.rs"));
        assert!(
            !path.exists(),
            "{} exists; Phase U2 builds an adapter, not a contract language",
            path.display()
        );
    }
    // And the adapter is compiled in, like the other two.
    assert_eq!(
        eplyx_engine::protocol::adapters().len(),
        3,
        "three adapters, all compiled in"
    );
}

// ---------------------------------------------------------------------------
// The layout, pinned to bytes mainnet actually holds
// ---------------------------------------------------------------------------

/// §1, §17. The offsets are checked against real deployed accounts, not against
/// the IDL that describes them.
///
/// The committed fixture is a matched pair captured at one slot: a KLend
/// reserve and an obligation that borrows from it. That pairing is what makes
/// this a check rather than a plausibility argument — an offset table that
/// drifted would have to drift *consistently across two different structures*
/// to keep the cross-references agreeing.
///
/// This is not a replay record. There is no transaction, no boundary proof and
/// no fidelity claim attached to it; it pins a layout and nothing else.
#[test]
fn the_layout_decodes_real_mainnet_accounts() {
    use base64::Engine as _;
    use eplyx_engine::protocol::kamino::state;

    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../docs/examples/mainnet-kamino-accounts.json"
    ))
    .expect("fixture parses");
    let bytes = |key: &str| {
        base64::prelude::BASE64_STANDARD
            .decode(fixture[key]["data_base64"].as_str().expect("base64"))
            .expect("decodes")
    };
    let reserve_address = fixture["reserve"]["address"].as_str().unwrap();
    let reserve = state::decode_reserve(&bytes("reserve"))
        .ok()
        .expect("a real reserve decodes");
    let obligation = state::decode_obligation(&bytes("obligation"))
        .ok()
        .expect("a real obligation decodes");

    // Both are owned by KLend and carry the lengths the layout declares.
    assert_eq!(bytes("reserve").len(), state::RESERVE_LEN);
    assert_eq!(bytes("obligation").len(), state::OBLIGATION_LEN);
    assert_eq!(
        fixture["reserve"]["owner"].as_str(),
        Some(kamino::PROGRAM_ID)
    );

    // The cross-reference: they name the same lending market, and the
    // obligation holds a borrow against exactly this reserve.
    assert_eq!(reserve.lending_market, obligation.lending_market);
    let position = obligation
        .borrow_against(reserve_address)
        .expect("the obligation borrows from this reserve");

    // And the quantities are sane at the reserve's own decimal count, which is
    // what confirms the scaled-fraction scale empirically rather than by
    // assertion: a wrong shift would put these orders of magnitude out.
    assert_eq!(reserve.mint_decimals, 6);
    let unit = 10_u128.pow(u32::from(reserve.mint_decimals));
    let reserve_borrowed = kamino::fraction::to_base_units(reserve.borrowed_amount_sf);
    let position_borrowed = kamino::fraction::to_base_units(position.borrowed_amount_sf);
    assert!(
        (1..1_000_000).contains(&(reserve_borrowed / unit)),
        "reserve borrowed {reserve_borrowed} is not a plausible token amount"
    );
    assert!(
        position_borrowed <= reserve_borrowed,
        "one obligation cannot owe more than the whole reserve has lent: \
         {position_borrowed} > {reserve_borrowed}"
    );
    // The reserve is live: it has liquidity, a collateral mint, and vaults.
    assert!(reserve.available_amount > 0);
    assert_ne!(reserve.liquidity_supply_vault, reserve.liquidity_fee_vault);
    assert_ne!(reserve.collateral_mint, reserve.liquidity_mint);
    assert!(obligation.has_debt);
    assert!(!obligation.deposits.is_empty());
}
