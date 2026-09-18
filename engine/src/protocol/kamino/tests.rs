//! Kamino KLend adapter tests.
//!
//! Two jobs. The first is ordinary: that the two supported actions are
//! recognised, bound, decoded and interpreted correctly, and that every shape
//! outside the contract is refused. The second is the architectural claim this
//! phase exists to test — that the adapter consumes Phase U1's evidence rather
//! than rebuilding it, and that a borrow's debt cannot be established from
//! token flow alone.

use super::*;
use crate::{
    ingest::transactions::HistoricalTransaction,
    standard_programs::spl_token,
    types::{AccountMetaSpec, AccountSnapshot, InstructionSpec, NamedAccount},
};
use state::fixtures::{address_of, ObligationBuilder, ReserveBuilder};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Address bytes used consistently across these tests.
mod at {
    pub const OWNER: u8 = 1;
    pub const OBLIGATION: u8 = 2;
    pub const MARKET: u8 = 3;
    pub const MARKET_AUTHORITY: u8 = 4;
    pub const RESERVE: u8 = 5;
    pub const LIQUIDITY_MINT: u8 = 6;
    pub const LIQUIDITY_SUPPLY: u8 = 7;
    pub const COLLATERAL_MINT: u8 = 8;
    pub const RESERVE_DESTINATION_COLLATERAL: u8 = 9;
    pub const USER_LIQUIDITY: u8 = 10;
    pub const PLACEHOLDER: u8 = 11;
    pub const FEE_RECEIVER: u8 = 12;
    pub const REFERRER_STATE: u8 = 13;
    pub const OTHER_RESERVE: u8 = 14;
    pub const OTHER_MARKET: u8 = 15;
    pub const OTHER_OBLIGATION: u8 = 16;
    pub const ORACLE: u8 = 17;
}

fn meta(byte: u8, signer: bool, writable: bool) -> AccountMetaSpec {
    AccountMetaSpec {
        address: address_of(byte),
        is_signer: signer,
        is_writable: writable,
    }
}

fn instruction(
    program: &str,
    discriminator: [u8; 8],
    amount: Option<u64>,
    accounts: Vec<AccountMetaSpec>,
) -> InstructionSpec {
    let mut data = discriminator.to_vec();
    if let Some(amount) = amount {
        data.extend_from_slice(&amount.to_le_bytes());
    }
    InstructionSpec {
        program: program.into(),
        accounts,
        data,
    }
}

fn refresh_reserve(reserve: u8, market: u8) -> InstructionSpec {
    instruction(
        PROGRAM_ID,
        REFRESH_RESERVE,
        None,
        vec![
            meta(reserve, false, true),
            meta(market, false, false),
            meta(at::ORACLE, false, false),
            meta(at::ORACLE, false, false),
            meta(at::ORACLE, false, false),
            meta(at::ORACLE, false, false),
        ],
    )
}

fn refresh_obligation(market: u8, obligation: u8) -> InstructionSpec {
    instruction(
        PROGRAM_ID,
        REFRESH_OBLIGATION,
        None,
        vec![meta(market, false, false), meta(obligation, false, true)],
    )
}

fn deposit_accounts() -> Vec<AccountMetaSpec> {
    vec![
        meta(at::OWNER, true, true),
        meta(at::OBLIGATION, false, true),
        meta(at::MARKET, false, false),
        meta(at::MARKET_AUTHORITY, false, false),
        meta(at::RESERVE, false, true),
        meta(at::LIQUIDITY_MINT, false, false),
        meta(at::LIQUIDITY_SUPPLY, false, true),
        meta(at::COLLATERAL_MINT, false, true),
        meta(at::RESERVE_DESTINATION_COLLATERAL, false, true),
        meta(at::USER_LIQUIDITY, false, true),
        meta(at::PLACEHOLDER, false, false),
        AccountMetaSpec {
            address: SPL_TOKEN_PROGRAM_ID.into(),
            is_signer: false,
            is_writable: false,
        },
        AccountMetaSpec {
            address: SPL_TOKEN_PROGRAM_ID.into(),
            is_signer: false,
            is_writable: false,
        },
        AccountMetaSpec {
            address: INSTRUCTION_SYSVAR_ID.into(),
            is_signer: false,
            is_writable: false,
        },
    ]
}

fn borrow_accounts() -> Vec<AccountMetaSpec> {
    vec![
        meta(at::OWNER, true, false),
        meta(at::OBLIGATION, false, true),
        meta(at::MARKET, false, false),
        meta(at::MARKET_AUTHORITY, false, false),
        meta(at::RESERVE, false, true),
        meta(at::LIQUIDITY_MINT, false, false),
        meta(at::LIQUIDITY_SUPPLY, false, true),
        meta(at::FEE_RECEIVER, false, true),
        meta(at::USER_LIQUIDITY, false, true),
        meta(at::REFERRER_STATE, false, true),
        AccountMetaSpec {
            address: SPL_TOKEN_PROGRAM_ID.into(),
            is_signer: false,
            is_writable: false,
        },
        AccountMetaSpec {
            address: INSTRUCTION_SYSVAR_ID.into(),
            is_signer: false,
            is_writable: false,
        },
    ]
}

/// A transaction carrying the two prerequisite refreshes and one action.
fn transaction(action: InstructionSpec, extra: Vec<InstructionSpec>) -> HistoricalTransaction {
    let mut tx: HistoricalTransaction = serde_json::from_str(
        r#"{"signature":"sig","slot":448000000,"block_time":null,"version":"legacy",
            "recent_blockhash":"blockhash","payer":"payer","account_keys":[],
            "instructions":[],"inner_instructions":[],"success":true,"error":null,
            "fee":5000,"compute_units":null,"logs":[]}"#,
    )
    .expect("skeleton");
    let mut instructions = vec![
        refresh_reserve(at::RESERVE, at::MARKET),
        refresh_obligation(at::MARKET, at::OBLIGATION),
    ];
    instructions.extend(extra);
    instructions.push(action);

    // Message keys: every address any instruction names, payer first.
    let mut keys: Vec<AccountMetaSpec> = vec![AccountMetaSpec {
        address: address_of(at::OWNER),
        is_signer: true,
        is_writable: true,
    }];
    for ix in &instructions {
        for meta in &ix.accounts {
            if !keys.iter().any(|k| k.address == meta.address) {
                keys.push(meta.clone());
            }
        }
    }
    tx.payer = address_of(at::OWNER);
    tx.account_keys = keys;
    tx.instructions = instructions;
    tx.pre_balances = Some(vec![0; tx.account_keys.len()]);
    tx.post_balances = Some(vec![0; tx.account_keys.len()]);
    tx.pre_token_balances = Some(Vec::new());
    tx.post_token_balances = Some(Vec::new());
    tx
}

fn deposit_transaction() -> HistoricalTransaction {
    transaction(
        instruction(PROGRAM_ID, DEPOSIT_V1, Some(1_000_000), deposit_accounts()),
        Vec::new(),
    )
}

fn borrow_transaction() -> HistoricalTransaction {
    transaction(
        instruction(PROGRAM_ID, BORROW_V1, Some(500_000), borrow_accounts()),
        Vec::new(),
    )
}

fn snapshot(owner: &str, data: Vec<u8>) -> AccountSnapshot {
    AccountSnapshot {
        lamports: 2_039_280,
        owner: owner.into(),
        data,
        executable: false,
        rent_epoch: 0,
    }
}

fn token_bytes(mint: u8, owner: u8, amount: u64) -> Vec<u8> {
    let mut data = vec![0_u8; spl_token::ACCOUNT_LEN];
    data[0..32].copy_from_slice(&[mint; 32]);
    data[32..64].copy_from_slice(&[owner; 32]);
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    data
}

fn mint_bytes(supply: u64, decimals: u8) -> Vec<u8> {
    let mut data = vec![0_u8; spl_token::MINT_LEN];
    data[36..44].copy_from_slice(&supply.to_le_bytes());
    data[44] = decimals;
    data[45] = 1;
    data
}

fn reserve_bytes() -> Vec<u8> {
    ReserveBuilder::new()
        .lending_market(at::MARKET)
        .liquidity_mint(at::LIQUIDITY_MINT)
        .supply_vault(at::LIQUIDITY_SUPPLY)
        .fee_vault(at::FEE_RECEIVER)
        .available(50_000_000)
        .borrowed_sf(1_000_u128 << fraction::FRACTION_BITS)
        .decimals(6)
        .collateral_mint(at::COLLATERAL_MINT)
        .collateral_supply(40_000_000)
        .collateral_vault(at::RESERVE_DESTINATION_COLLATERAL)
        .build()
}

fn named(label: &str, address_byte: u8, owner: &str, data: Vec<u8>) -> NamedAccount {
    NamedAccount {
        label: label.into(),
        address: address_of(address_byte),
        account: snapshot(owner, data),
    }
}

/// The pre-state a borrow runs against: obligation with an existing debt, the
/// reserve, the borrower's token account, the reserve vault and the fee vault.
fn borrow_accounts_state(existing_debt_sf: u128) -> Vec<NamedAccount> {
    vec![
        named(
            "obligation",
            at::OBLIGATION,
            PROGRAM_ID,
            ObligationBuilder::new()
                .lending_market(at::MARKET)
                .owner(at::OWNER)
                .has_debt(true)
                .deposit(0, at::OTHER_RESERVE, 10_000_000)
                .borrow(0, at::RESERVE, existing_debt_sf)
                .build(),
        ),
        named("borrow-reserve", at::RESERVE, PROGRAM_ID, reserve_bytes()),
        named(
            "user-destination-liquidity",
            at::USER_LIQUIDITY,
            SPL_TOKEN_PROGRAM_ID,
            token_bytes(at::LIQUIDITY_MINT, at::OWNER, 0),
        ),
        named(
            "reserve-source-liquidity",
            at::LIQUIDITY_SUPPLY,
            SPL_TOKEN_PROGRAM_ID,
            token_bytes(at::LIQUIDITY_MINT, at::MARKET_AUTHORITY, 50_000_000),
        ),
        named(
            "borrow-reserve-liquidity-fee-receiver",
            at::FEE_RECEIVER,
            SPL_TOKEN_PROGRAM_ID,
            token_bytes(at::LIQUIDITY_MINT, at::MARKET_AUTHORITY, 0),
        ),
    ]
}

fn deposit_accounts_state() -> Vec<NamedAccount> {
    vec![
        named(
            "obligation",
            at::OBLIGATION,
            PROGRAM_ID,
            ObligationBuilder::new()
                .lending_market(at::MARKET)
                .owner(at::OWNER)
                .deposit(0, at::RESERVE, 0)
                .build(),
        ),
        named("reserve", at::RESERVE, PROGRAM_ID, reserve_bytes()),
        named(
            "user-source-liquidity",
            at::USER_LIQUIDITY,
            SPL_TOKEN_PROGRAM_ID,
            token_bytes(at::LIQUIDITY_MINT, at::OWNER, 5_000_000),
        ),
        named(
            "reserve-liquidity-supply",
            at::LIQUIDITY_SUPPLY,
            SPL_TOKEN_PROGRAM_ID,
            token_bytes(at::LIQUIDITY_MINT, at::MARKET_AUTHORITY, 50_000_000),
        ),
        named(
            "reserve-collateral-mint",
            at::COLLATERAL_MINT,
            SPL_TOKEN_PROGRAM_ID,
            mint_bytes(40_000_000, 6),
        ),
    ]
}

/// Build an execution result from `(label, snapshot)` pairs.
fn execution(success: bool, accounts: Vec<(&str, AccountSnapshot)>) -> ExecutionResult {
    ExecutionResult {
        version: "v1".into(),
        success,
        error: None,
        compute_units: Some(120_000),
        fee: 5_000,
        logs: Vec::new(),
        cpi_calls: Vec::new(),
        accounts: accounts
            .into_iter()
            .map(|(label, s)| (label.to_string(), s))
            .collect::<BTreeMap<_, _>>(),
    }
}

/// A borrow outcome: how much the borrower received, and the debt recorded.
fn borrow_outcome(received: u64, fee: u64, debt_sf: u128) -> ExecutionResult {
    execution(
        true,
        vec![
            (
                "obligation",
                snapshot(
                    PROGRAM_ID,
                    ObligationBuilder::new()
                        .lending_market(at::MARKET)
                        .owner(at::OWNER)
                        .has_debt(true)
                        .deposit(0, at::OTHER_RESERVE, 10_000_000)
                        .borrow(0, at::RESERVE, debt_sf)
                        .build(),
                ),
            ),
            ("borrow-reserve", snapshot(PROGRAM_ID, reserve_bytes())),
            (
                "user-destination-liquidity",
                snapshot(
                    SPL_TOKEN_PROGRAM_ID,
                    token_bytes(at::LIQUIDITY_MINT, at::OWNER, received),
                ),
            ),
            (
                "reserve-source-liquidity",
                snapshot(
                    SPL_TOKEN_PROGRAM_ID,
                    token_bytes(
                        at::LIQUIDITY_MINT,
                        at::MARKET_AUTHORITY,
                        50_000_000 - received - fee,
                    ),
                ),
            ),
            (
                "borrow-reserve-liquidity-fee-receiver",
                snapshot(
                    SPL_TOKEN_PROGRAM_ID,
                    token_bytes(at::LIQUIDITY_MINT, at::MARKET_AUTHORITY, fee),
                ),
            ),
        ],
    )
}

fn deposit_outcome(deposited: u64, collateral: u64) -> ExecutionResult {
    execution(
        true,
        vec![
            (
                "obligation",
                snapshot(
                    PROGRAM_ID,
                    ObligationBuilder::new()
                        .lending_market(at::MARKET)
                        .owner(at::OWNER)
                        .deposit(0, at::RESERVE, collateral)
                        .build(),
                ),
            ),
            ("reserve", snapshot(PROGRAM_ID, reserve_bytes())),
            (
                "user-source-liquidity",
                snapshot(
                    SPL_TOKEN_PROGRAM_ID,
                    token_bytes(at::LIQUIDITY_MINT, at::OWNER, 5_000_000 - deposited),
                ),
            ),
            (
                "reserve-liquidity-supply",
                snapshot(
                    SPL_TOKEN_PROGRAM_ID,
                    token_bytes(
                        at::LIQUIDITY_MINT,
                        at::MARKET_AUTHORITY,
                        50_000_000 + deposited,
                    ),
                ),
            ),
            (
                "reserve-collateral-mint",
                snapshot(SPL_TOKEN_PROGRAM_ID, mint_bytes(40_000_000 + collateral, 6)),
            ),
        ],
    )
}

const ADAPTER: KaminoKlendAdapter = KaminoKlendAdapter;

fn subject_names(subjects: &[crate::semantics::EvaluableSubject]) -> Vec<String> {
    subjects
        .iter()
        .map(|s| s.subject.as_str().to_string())
        .collect()
}

fn finding_prints(findings: &[crate::semantics::NamedFinding]) -> Vec<String> {
    findings.iter().map(|f| f.fingerprint.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Recognition and admission
// ---------------------------------------------------------------------------

#[test]
fn both_supported_actions_are_recognised_by_discriminator() {
    assert_eq!(
        KlendOp::from_discriminator(&DEPOSIT_V1),
        Some(KlendOp::DepositV1)
    );
    assert_eq!(
        KlendOp::from_discriminator(&DEPOSIT_V2),
        Some(KlendOp::DepositV2)
    );
    assert_eq!(
        KlendOp::from_discriminator(&BORROW_V1),
        Some(KlendOp::BorrowV1)
    );
    assert_eq!(
        KlendOp::from_discriminator(&BORROW_V2),
        Some(KlendOp::BorrowV2)
    );
    // Recognition is by discriminator, never by name, and everything else in
    // KLend's 66-instruction surface stays unsupported.
    for other in [REFRESH_RESERVE, REFRESH_OBLIGATION, [0; 8], [255; 8]] {
        assert_eq!(KlendOp::from_discriminator(&other), None);
    }
    assert_eq!(KlendOp::from_discriminator(&[1, 2, 3]), None, "short data");
}

#[test]
fn the_two_forms_of_one_family_share_an_action_id_and_differ_in_arity() {
    assert_eq!(
        KlendOp::DepositV1.action_id(),
        KlendOp::DepositV2.action_id()
    );
    assert_eq!(KlendOp::BorrowV1.action_id(), KlendOp::BorrowV2.action_id());
    assert_ne!(
        KlendOp::DepositV1.action_id(),
        KlendOp::BorrowV1.action_id()
    );
    assert_eq!(KlendOp::DepositV1.account_count(), 14);
    assert_eq!(KlendOp::DepositV2.account_count(), 17);
    assert_eq!(KlendOp::BorrowV1.account_count(), 12);
    assert_eq!(KlendOp::BorrowV2.account_count(), 15);
    // The economic accounts sit at the same positions in both forms.
    for position in 0..12 {
        assert_eq!(
            KlendOp::BorrowV1.role(position),
            KlendOp::BorrowV2.role(position)
        );
    }
    assert_eq!(KlendOp::BorrowV2.role(14), Some("farms-program"));
}

#[test]
fn a_supported_deposit_and_borrow_are_accepted() {
    ADAPTER.accept(&deposit_transaction()).expect("deposit");
    ADAPTER.accept(&borrow_transaction()).expect("borrow");
}

/// §8. The prerequisite refreshes are required, not tolerated: KLend refuses a
/// stale reserve, and a borrow computed against an unrefreshed obligation is a
/// different computation from the one that ran.
#[test]
fn an_action_without_its_prerequisite_refreshes_is_refused() {
    let mut tx = borrow_transaction();
    tx.instructions
        .retain(|ix| ix.data.get(..8) != Some(&REFRESH_RESERVE[..]));
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("requires refreshReserve"), "{error}");

    let mut tx = borrow_transaction();
    tx.instructions
        .retain(|ix| ix.data.get(..8) != Some(&REFRESH_OBLIGATION[..]));
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("requires refreshObligation"), "{error}");
}

/// A refresh of *some other* reserve leaves this one stale. Presence is not
/// enough; the refresh has to name the accounts the action names.
#[test]
fn a_refresh_of_a_different_reserve_does_not_satisfy_the_prerequisite() {
    let mut tx = borrow_transaction();
    tx.instructions[0] = refresh_reserve(at::OTHER_RESERVE, at::MARKET);
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("requires refreshReserve"), "{error}");
}

#[test]
fn a_refresh_naming_a_different_market_is_refused() {
    let mut tx = borrow_transaction();
    tx.instructions[0] = refresh_reserve(at::RESERVE, at::OTHER_MARKET);
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("different lending market"), "{error}");

    let mut tx = borrow_transaction();
    tx.instructions[1] = refresh_obligation(at::OTHER_MARKET, at::OBLIGATION);
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("different lending market"), "{error}");
}

#[test]
fn a_refresh_of_a_different_obligation_does_not_satisfy_the_prerequisite() {
    let mut tx = borrow_transaction();
    tx.instructions[1] = refresh_obligation(at::MARKET, at::OTHER_OBLIGATION);
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("requires refreshObligation"), "{error}");
}

/// Measured against mainnet: `refreshObligation` declares two accounts and
/// carries three, because Anchor appends the obligation's reserves as
/// `remaining_accounts`. An adapter that trusted the IDL's arity would reject
/// every production refresh — this is the kind of gap only real data finds.
#[test]
fn a_production_shaped_refresh_obligation_with_extra_accounts_is_accepted() {
    let mut tx = borrow_transaction();
    tx.instructions[1]
        .accounts
        .push(meta(at::OTHER_RESERVE, false, true));
    ADAPTER
        .accept(&tx)
        .expect("remaining_accounts on a refresh are the obligation's own reserves");

    // The two declared positions are still checked.
    let mut tx = borrow_transaction();
    tx.instructions[1].accounts.truncate(1);
    assert!(ADAPTER.accept(&tx).is_err());
}

#[test]
fn a_failed_original_transaction_is_refused() {
    let mut tx = borrow_transaction();
    tx.success = false;
    assert!(ADAPTER.accept(&tx).is_err());
    let mut tx = borrow_transaction();
    tx.error = Some(serde_json::json!({"InstructionError": [2, "Custom"]}));
    assert!(ADAPTER.accept(&tx).is_err());
}

#[test]
fn an_unsupported_klend_instruction_alongside_the_action_is_refused() {
    let tx = transaction(
        instruction(PROGRAM_ID, BORROW_V1, Some(1), borrow_accounts()),
        vec![instruction(
            PROGRAM_ID,
            [99; 8],
            Some(1),
            vec![meta(at::OBLIGATION, false, true)],
        )],
    );
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("unsupported KLend instruction"), "{error}");
}

#[test]
fn an_unsupported_companion_program_is_refused() {
    let mut tx = borrow_transaction();
    tx.instructions.push(InstructionSpec {
        program: "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".into(),
        accounts: Vec::new(),
        data: vec![1],
    });
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("unsupported program"), "{error}");
}

#[test]
fn two_klend_actions_in_one_transaction_are_refused() {
    let tx = transaction(
        instruction(PROGRAM_ID, BORROW_V1, Some(1), borrow_accounts()),
        vec![instruction(
            PROGRAM_ID,
            DEPOSIT_V1,
            Some(1),
            deposit_accounts(),
        )],
    );
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("one KLend action"), "{error}");
}

#[test]
fn a_wrong_arity_or_unsigned_owner_or_zero_amount_is_refused() {
    let mut tx = borrow_transaction();
    tx.instructions.last_mut().unwrap().accounts.pop();
    assert!(ADAPTER
        .accept(&tx)
        .expect_err("arity")
        .to_string()
        .contains("outside the supported shape"));

    let mut tx = borrow_transaction();
    tx.instructions.last_mut().unwrap().accounts[0].is_signer = false;
    assert!(ADAPTER
        .accept(&tx)
        .expect_err("signer")
        .to_string()
        .contains("direct signer"));

    let tx = transaction(
        instruction(PROGRAM_ID, BORROW_V1, Some(0), borrow_accounts()),
        Vec::new(),
    );
    assert!(ADAPTER
        .accept(&tx)
        .expect_err("amount")
        .to_string()
        .contains("non-zero"));
}

#[test]
fn a_message_resolving_lookup_tables_is_refused() {
    let mut tx = borrow_transaction();
    tx.version = "v0".into();
    tx.loaded_address_count = 4;
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(error.contains("lookup table"), "{error}");
}

#[test]
fn cpi_outside_the_defined_programs_is_refused() {
    let mut tx = borrow_transaction();
    tx.inner_instruction_frames = vec![serde_json::from_value(serde_json::json!({
        "program": "FarmsPZpWu9i7Kky8tPN37rs2TpmMrAZrC7S7vJa91Hr",
        "stack_height": 2, "outer_index": 2, "account_count": 4, "data_len": 8
    }))
    .expect("frame")];
    tx.inner_instructions = vec![InstructionSpec {
        program: FARMS_PROGRAM_ID.into(),
        accounts: Vec::new(),
        data: vec![0; 8],
    }];
    // A V1 borrow does not reach Farms.
    let error = ADAPTER.accept(&tx).expect_err("must refuse").to_string();
    assert!(
        error.contains("unsupported cross-program invocation"),
        "{error}"
    );
}

#[test]
fn missing_validator_token_balances_are_refused() {
    let mut tx = borrow_transaction();
    tx.pre_token_balances = None;
    assert!(ADAPTER.accept(&tx).is_err());
}

// ---------------------------------------------------------------------------
// Role binding
// ---------------------------------------------------------------------------

#[test]
fn accounts_are_bound_to_their_klend_roles() {
    let tx = borrow_transaction();
    let labels: Vec<String> = (0..tx.account_keys.len())
        .map(|index| ADAPTER.label(&tx, index))
        .collect();
    assert!(labels.contains(&"obligation".to_string()));
    assert!(labels.contains(&"borrow-reserve".to_string()));
    assert!(labels.contains(&"user-destination-liquidity".to_string()));
    assert!(labels.contains(&"borrow-reserve-liquidity-fee-receiver".to_string()));
    // The owner signs and pays, and keeps the role that explains what it does.
    assert_eq!(labels[0], "owner");
    // Labels are unique, which is what makes them usable as a diff key.
    let unique: std::collections::BTreeSet<&String> = labels.iter().collect();
    assert_eq!(unique.len(), labels.len(), "{labels:?}");
}

#[test]
fn the_economic_entity_is_the_obligation_not_the_wallet() {
    let entity = ADAPTER
        .economic_entity_id(&borrow_transaction(), &[])
        .expect("entity");
    assert_eq!(entity.kind, "kamino-obligation");
    assert_eq!(entity.id, address_of(at::OBLIGATION));
}

#[test]
fn required_accounts_include_the_oracle_the_refresh_reads() {
    let required = ADAPTER.required_accounts(&borrow_transaction());
    assert!(required.contains(&address_of(at::ORACLE)), "{required:?}");
    assert!(required.contains(&address_of(at::OBLIGATION)));
    assert!(required.contains(&address_of(at::RESERVE)));
    // Programs and sysvars are not acquired as protocol state.
    assert!(!required.iter().any(|a| a == SPL_TOKEN_PROGRAM_ID));
    assert!(!required.iter().any(|a| a == INSTRUCTION_SYSVAR_ID));
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

#[test]
fn a_reserve_and_an_obligation_decode_into_protocol_terms() {
    let reserve = ADAPTER
        .decode(&snapshot(PROGRAM_ID, reserve_bytes()))
        .expect("reserve decodes");
    assert_eq!(reserve.kind, "kamino-reserve");
    assert_eq!(
        reserve.field("available_amount").unwrap().value.render(),
        "50000000"
    );
    // The scaled value is reported exactly, as text, never as a token amount.
    assert_eq!(
        reserve.field("borrowed_amount_sf").unwrap().value.render(),
        (1_000_u128 << fraction::FRACTION_BITS).to_string()
    );

    let obligation = ADAPTER
        .decode(&snapshot(
            PROGRAM_ID,
            ObligationBuilder::new()
                .lending_market(at::MARKET)
                .owner(at::OWNER)
                .borrow(2, at::RESERVE, 7 << fraction::FRACTION_BITS)
                .build(),
        ))
        .expect("obligation decodes");
    assert_eq!(obligation.kind, "kamino-obligation");
    // Per-slot naming, so a subject can name the slot it reads.
    assert!(obligation.field("borrow_2_amount_sf").is_some());
    assert!(
        obligation.field("borrow_0_amount_sf").is_none(),
        "empty slot"
    );
}

/// §23. The adapter must not parse token layouts itself. It decodes them
/// through the shared standard-program decoders, which is why an account under
/// *either* token program reads correctly.
#[test]
fn token_accounts_are_decoded_through_the_shared_standard_program_layer() {
    let spl = ADAPTER
        .decode(&snapshot(
            SPL_TOKEN_PROGRAM_ID,
            token_bytes(at::LIQUIDITY_MINT, at::OWNER, 4_242),
        ))
        .expect("spl token account");
    assert_eq!(spl.kind, "token-account");
    assert_eq!(spl.field("amount").unwrap().value.render(), "4242");

    // The same bytes with a Token-2022 extension, under the Token-2022 program.
    let mut extended = token_bytes(at::LIQUIDITY_MINT, at::OWNER, 4_242);
    extended.resize(crate::standard_programs::token2022::ACCOUNT_TYPE_OFFSET, 0);
    extended.push(2);
    let t22 = ADAPTER
        .decode(&snapshot(TOKEN_2022_PROGRAM_ID, extended))
        .expect("token-2022 account");
    assert_eq!(t22.kind, "token-account");
    assert_eq!(t22.field("amount").unwrap().value.render(), "4242");
}

#[test]
fn an_account_owned_by_another_program_is_not_decoded() {
    assert!(ADAPTER
        .decode(&snapshot("11111111111111111111111111111111", vec![1, 2, 3]))
        .is_none());
    // And a KLend-owned account this build does not model stays undecoded
    // rather than being read at whatever offsets fit.
    assert!(ADAPTER
        .decode(&snapshot(PROGRAM_ID, vec![7; 900]))
        .is_none());
}

// ---------------------------------------------------------------------------
// Deposit semantics
// ---------------------------------------------------------------------------

#[test]
fn a_deposit_reports_its_flows_and_the_position_it_creates() {
    let accounts = deposit_accounts_state();
    let fields = ADAPTER.summarize(&accounts, &deposit_outcome(1_000_000, 900_000));
    let value = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.value.render())
    };
    assert_eq!(value("liquidity_deposited").as_deref(), Some("1.000000"));
    assert_eq!(
        value("reserve_liquidity_received").as_deref(),
        Some("1.000000")
    );
    assert_eq!(value("collateral_minted").as_deref(), Some("900000"));
    assert_eq!(
        value("obligation_collateral_deposited").as_deref(),
        Some("900000")
    );
}

#[test]
fn a_deposit_declares_exactly_the_subjects_it_can_measure() {
    let subjects = subject_names(
        &ADAPTER.evaluable_subjects(&deposit_transaction(), &deposit_accounts_state()),
    );
    assert!(subjects.contains(&"transaction".to_string()));
    assert!(subjects.contains(&"liquidity_deposited".to_string()));
    assert!(subjects.contains(&"reserve_liquidity_received".to_string()));
    assert!(subjects.contains(&"obligation_collateral_deposited".to_string()));
    // Borrow subjects belong to the other action and must not appear.
    assert!(!subjects.contains(&"debt_increased".to_string()));
    assert!(!subjects.contains(&"origination_fee".to_string()));
    // §15: nothing about risk is declared, because nothing about risk is
    // implemented.
    assert!(!subjects.iter().any(|s| s.contains("health")));
    assert!(!subjects.iter().any(|s| s.contains("liquidation")));
}

#[test]
fn a_deposit_regression_that_short_changes_the_depositor_is_named() {
    let accounts = deposit_accounts_state();
    let baseline = deposit_outcome(1_000_000, 900_000);
    let candidate = deposit_outcome(1_000_000, 890_000);
    let prints = finding_prints(&ADAPTER.named_findings(
        &deposit_transaction(),
        &accounts,
        &baseline,
        &candidate,
    ));
    assert!(
        prints.contains(
            &"kamino-klend/deposit_reserve_liquidity_and_obligation_collateral/economic/\
               obligation_collateral_deposited/decreased"
                .to_string()
        ),
        "{prints:?}"
    );
}

// ---------------------------------------------------------------------------
// Borrow semantics — the reason this action is in the slice
// ---------------------------------------------------------------------------

#[test]
fn a_borrow_reports_the_liquidity_it_moved_and_the_debt_it_created() {
    let accounts = borrow_accounts_state(0);
    let fields = ADAPTER.summarize(
        &accounts,
        &borrow_outcome(495_000, 5_000, 500_000_u128 << fraction::FRACTION_BITS),
    );
    let value = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.value.render())
    };
    assert_eq!(value("liquidity_borrowed").as_deref(), Some("0.495000"));
    assert_eq!(
        value("reserve_liquidity_drawn").as_deref(),
        Some("0.500000")
    );
    assert_eq!(value("origination_fee").as_deref(), Some("0.005000"));
    // The debt is the whole borrow, not the net the borrower received.
    assert_eq!(value("debt_increased").as_deref(), Some("0.500000"));
    // And the exact scaled evidence sits beside it.
    assert_eq!(
        value("debt_increased_scaled").as_deref(),
        Some((500_000_i128 << fraction::FRACTION_BITS).to_string()).as_deref()
    );
}

/// §23, §41. Every token quantity goes through the shared decoder, which means
/// it goes through the token *program* check.
///
/// The bytes below are a perfectly well-formed SPL Token account, sitting in an
/// account owned by something that is not a token program. A reader that went
/// straight to the amount offset would report a borrower credit of 495,000; the
/// shared decoder refuses, because an account's layout is never inferred from
/// its bytes alone. Mutation 1 of the phase's suite is exactly this bypass, and
/// this is the test that catches it.
#[test]
fn a_token_shaped_account_under_a_foreign_program_yields_no_balance() {
    let mut accounts = borrow_accounts_state(0);
    let foreign = "Stake11111111111111111111111111111111111111";
    for named in accounts.iter_mut() {
        if named.label == "user-destination-liquidity" {
            named.account.owner = foreign.into();
        }
    }
    let mut outcome = borrow_outcome(495_000, 5_000, 500_000_u128 << fraction::FRACTION_BITS);
    outcome
        .accounts
        .get_mut("user-destination-liquidity")
        .unwrap()
        .owner = foreign.into();

    let fields = ADAPTER.summarize(&accounts, &outcome);
    assert!(
        !fields.iter().any(|f| f.name == "liquidity_borrowed"),
        "a token-shaped account owned by a non-token program is not a balance: {:?}",
        fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>()
    );
    // The debt, read from the obligation, is unaffected - so the absence above
    // is the token check working rather than the whole summary collapsing.
    assert!(fields.iter().any(|f| f.name == "debt_increased"));
}

/// The same guard on the deposit side, where the reserve's own vault is read.
#[test]
fn a_reserve_vault_under_a_foreign_program_yields_no_flow() {
    let mut accounts = deposit_accounts_state();
    for named in accounts.iter_mut() {
        if named.label == "reserve-liquidity-supply" {
            named.account.owner = "11111111111111111111111111111111".into();
        }
    }
    let mut outcome = deposit_outcome(1_000_000, 900_000);
    outcome
        .accounts
        .get_mut("reserve-liquidity-supply")
        .unwrap()
        .owner = "11111111111111111111111111111111".into();
    let fields = ADAPTER.summarize(&accounts, &outcome);
    assert!(!fields
        .iter()
        .any(|f| f.name == "reserve_liquidity_received"));
    assert!(fields.iter().any(|f| f.name == "liquidity_deposited"));
}

/// §33. **The central test of this phase's slice.**
///
/// Two candidates credit the borrower identically — same token flow, same fee,
/// same reserve draw — and record different debts. Every universal flow
/// primitive is equal between them. If token flow were sufficient to establish
/// the borrow semantic, this would report no change, and a candidate that
/// silently over-charges every borrower would pass.
#[test]
fn token_flow_alone_cannot_establish_the_borrow_semantic() {
    let accounts = borrow_accounts_state(0);
    let honest = borrow_outcome(495_000, 5_000, 500_000_u128 << fraction::FRACTION_BITS);
    let overcharging = borrow_outcome(495_000, 5_000, 600_000_u128 << fraction::FRACTION_BITS);

    // Premise: the flows really are identical.
    let flow = |result: &ExecutionResult| {
        ADAPTER
            .summarize(&accounts, result)
            .into_iter()
            .filter(|f| {
                matches!(
                    f.name.as_str(),
                    "liquidity_borrowed" | "reserve_liquidity_drawn" | "origination_fee"
                )
            })
            .map(|f| (f.name, f.value.render()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        flow(&honest),
        flow(&overcharging),
        "premise: the token flows are indistinguishable"
    );

    // And the semantics are not.
    let prints = finding_prints(&ADAPTER.named_findings(
        &borrow_transaction(),
        &accounts,
        &honest,
        &overcharging,
    ));
    assert_eq!(
        prints,
        vec!["kamino-klend/borrow_obligation_liquidity/economic/debt_increased/increased"],
        "the debt difference is the only evidence, and it is found"
    );
}

/// The converse: a candidate that hands the borrower less while recording the
/// same debt is also caught, through the flow side.
#[test]
fn a_borrower_short_changed_on_liquidity_is_named() {
    let accounts = borrow_accounts_state(0);
    let honest = borrow_outcome(495_000, 5_000, 500_000_u128 << fraction::FRACTION_BITS);
    let short = borrow_outcome(490_000, 10_000, 500_000_u128 << fraction::FRACTION_BITS);
    let prints =
        finding_prints(&ADAPTER.named_findings(&borrow_transaction(), &accounts, &honest, &short));
    assert!(
        prints.contains(
            &"kamino-klend/borrow_obligation_liquidity/economic/liquidity_borrowed/decreased"
                .to_string()
        ),
        "{prints:?}"
    );
    assert!(
        prints.contains(
            &"kamino-klend/borrow_obligation_liquidity/economic/origination_fee/increased"
                .to_string()
        ),
        "{prints:?}"
    );
}

/// A borrow added to an existing position measures the *increase*, not the
/// total. Converting each side before subtracting would lose a sub-unit here.
#[test]
fn a_borrow_against_an_existing_debt_measures_the_increase() {
    let existing = 1_234_567_u128 << fraction::FRACTION_BITS;
    let accounts = borrow_accounts_state(existing);
    let result = borrow_outcome(
        495_000,
        5_000,
        existing + (500_000_u128 << fraction::FRACTION_BITS),
    );
    let fields = ADAPTER.summarize(&accounts, &result);
    assert_eq!(
        fields
            .iter()
            .find(|f| f.name == "debt_increased")
            .unwrap()
            .value
            .render(),
        "0.500000"
    );
}

#[test]
fn a_borrow_declares_exactly_the_subjects_it_can_measure() {
    let subjects = subject_names(
        &ADAPTER.evaluable_subjects(&borrow_transaction(), &borrow_accounts_state(0)),
    );
    for expected in [
        "transaction",
        "liquidity_borrowed",
        "reserve_liquidity_drawn",
        "origination_fee",
        "debt_increased",
    ] {
        assert!(subjects.contains(&expected.to_string()), "{subjects:?}");
    }
    assert!(!subjects.contains(&"liquidity_deposited".to_string()));
}

/// §26. A derived subject needs the state its evaluator reads. Declaring
/// `debt_increased` for an observation whose obligation was not captured would
/// be claiming a measurement this adapter cannot make.
#[test]
fn a_derived_subject_is_not_declared_when_its_state_is_missing() {
    let mut accounts = borrow_accounts_state(0);
    accounts.retain(|named| named.label != "obligation");
    let subjects = subject_names(&ADAPTER.evaluable_subjects(&borrow_transaction(), &accounts));
    assert!(
        !subjects.contains(&"debt_increased".to_string()),
        "{subjects:?}"
    );
    // The flow subjects are still measurable and still declared.
    assert!(subjects.contains(&"liquidity_borrowed".to_string()));
}

#[test]
fn every_emitted_finding_is_covered_by_a_declared_subject() {
    for (tx, accounts, before, after) in [
        (
            borrow_transaction(),
            borrow_accounts_state(0),
            borrow_outcome(495_000, 5_000, 500_000_u128 << fraction::FRACTION_BITS),
            borrow_outcome(480_000, 9_000, 600_000_u128 << fraction::FRACTION_BITS),
        ),
        (
            deposit_transaction(),
            deposit_accounts_state(),
            deposit_outcome(1_000_000, 900_000),
            deposit_outcome(900_000, 800_000),
        ),
    ] {
        let declared = subject_names(&ADAPTER.evaluable_subjects(&tx, &accounts));
        for finding in ADAPTER.named_findings(&tx, &accounts, &before, &after) {
            assert!(
                declared.contains(&finding.fingerprint.subject.as_str().to_string()),
                "{} was emitted without being declared evaluable; declared: {declared:?}",
                finding.fingerprint
            );
        }
    }
}

/// A candidate that stops executing is one finding, not a cascade of quantity
/// changes beside it.
#[test]
fn a_reverting_candidate_reports_only_the_outcome() {
    let accounts = borrow_accounts_state(0);
    let findings = ADAPTER.named_findings(
        &borrow_transaction(),
        &accounts,
        &borrow_outcome(495_000, 5_000, 500_000_u128 << fraction::FRACTION_BITS),
        &execution(false, Vec::new()),
    );
    assert_eq!(
        finding_prints(&findings),
        vec!["kamino-klend/borrow_obligation_liquidity/execution/transaction/now_reverts"]
    );
    assert_eq!(findings[0].severity, crate::diff::Severity::Critical);
}

#[test]
fn a_preserved_outcome_produces_no_findings() {
    let accounts = borrow_accounts_state(0);
    let same = borrow_outcome(495_000, 5_000, 500_000_u128 << fraction::FRACTION_BITS);
    assert!(ADAPTER
        .named_findings(&borrow_transaction(), &accounts, &same, &same)
        .is_empty());
    let accounts = deposit_accounts_state();
    let same = deposit_outcome(1_000_000, 900_000);
    assert!(ADAPTER
        .named_findings(&deposit_transaction(), &accounts, &same, &same)
        .is_empty());
}

// ---------------------------------------------------------------------------
// Evidence wiring
// ---------------------------------------------------------------------------

/// A debt subject may read any of the five borrow slots, because the slot the
/// action touches is a property of the observation.
#[test]
fn a_debt_subject_names_every_slot_its_position_could_occupy() {
    let sources = ADAPTER.decoded_sources_of("debt_increased");
    assert_eq!(sources.len(), state::BORROW_SLOTS);
    assert!(sources.contains(&("obligation", "borrow_2_amount_sf")));
    // A flow subject reads exactly one field.
    assert_eq!(
        ADAPTER.decoded_sources_of("liquidity_borrowed"),
        &[("user-destination-liquidity", "amount")]
    );
    assert!(ADAPTER.decoded_sources_of("nothing_promoted").is_empty());
}

/// Every decoded field a promoted subject reads must be a field the adapter
/// actually emits, or the change it describes would be reported as
/// undeclarable even when a finding named it.
#[test]
fn every_decoded_source_names_a_field_the_adapter_emits() {
    let obligation = ADAPTER
        .decode(&snapshot(
            PROGRAM_ID,
            ObligationBuilder::new()
                .deposit(0, at::RESERVE, 1)
                .deposit(1, at::OTHER_RESERVE, 1)
                .deposit(2, at::RESERVE, 1)
                .deposit(3, at::RESERVE, 1)
                .deposit(4, at::RESERVE, 1)
                .deposit(5, at::RESERVE, 1)
                .deposit(6, at::RESERVE, 1)
                .deposit(7, at::RESERVE, 1)
                .borrow(0, at::RESERVE, 1)
                .borrow(1, at::RESERVE, 1)
                .borrow(2, at::RESERVE, 1)
                .borrow(3, at::RESERVE, 1)
                .borrow(4, at::RESERVE, 1)
                .build(),
        ))
        .expect("decodes");
    for (label, field) in ADAPTER
        .decoded_sources_of("debt_increased")
        .iter()
        .chain(ADAPTER.decoded_sources_of("obligation_collateral_deposited"))
    {
        assert_eq!(*label, "obligation");
        assert!(
            obligation.field(field).is_some(),
            "{field} is named as a source but never emitted"
        );
    }
}

/// The byte ranges the adapter claims to decode must actually contain the
/// fields it reads. A range table that drifts from the layout would let a real
/// change be suppressed as "accounted for".
#[test]
fn the_declared_byte_ranges_cover_the_fields_that_move() {
    let before = ObligationBuilder::new().borrow(1, at::RESERVE, 5).build();
    let mut after = before.clone();
    // Move borrows[1].borrowedAmountSf.
    let offset = 1208 + 200 + 88;
    after[offset..offset + 16].copy_from_slice(&9_u128.to_le_bytes());
    let differing: Vec<usize> = before
        .iter()
        .zip(after.iter())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert!(!differing.is_empty());
    let ranges = ADAPTER.decoded_byte_ranges("obligation");
    for offset in differing {
        assert!(
            ranges.iter().any(|r| r.contains(&offset)),
            "offset {offset} moved and no declared range covers it"
        );
    }
    // And an account the adapter does not interpret claims no ranges at all.
    assert!(ADAPTER
        .decoded_byte_ranges("referrer-token-state")
        .is_empty());
}

#[test]
fn the_interpretation_walk_reports_a_changed_debt_field() {
    let accounts = borrow_accounts_state(0);
    let v1 = borrow_outcome(495_000, 5_000, 500_000_u128 << fraction::FRACTION_BITS);
    let v2 = borrow_outcome(495_000, 5_000, 600_000_u128 << fraction::FRACTION_BITS);
    let changes = ADAPTER.interpret(&accounts, &v1, &v2);
    assert!(
        changes
            .iter()
            .any(|c| c.account_label == "obligation" && c.field == "borrow_0_amount_sf"),
        "{changes:?}"
    );
}

// ---------------------------------------------------------------------------
// Coarse classification, and the pressure it is under
// ---------------------------------------------------------------------------

/// §25. A borrow is not in the coarse vocabulary and is **not** forced into
/// `Withdraw`. Value leaves the protocol toward the user, which looks like a
/// withdrawal and is the opposite economically.
#[test]
fn a_borrow_is_reported_as_unknown_rather_than_misclassified() {
    assert_eq!(
        ADAPTER.semantic_action(&borrow_transaction()),
        SemanticAction::Unknown
    );
    assert_ne!(
        ADAPTER.semantic_action(&borrow_transaction()),
        SemanticAction::Withdraw
    );
    // The exact identity is not lost: it lives in the action id, which is what
    // an expectation keys on.
    assert_eq!(
        ADAPTER.action_id(&borrow_transaction()).unwrap().as_str(),
        "borrow_obligation_liquidity"
    );
    // A deposit is honestly a deposit.
    assert_eq!(
        ADAPTER.semantic_action(&deposit_transaction()),
        SemanticAction::Deposit
    );
}

#[test]
fn state_features_describe_the_pre_state_without_claiming_risk() {
    let features = ADAPTER.state_features(&borrow_transaction(), &borrow_accounts_state(0));
    let names: Vec<&str> = features.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"klend_action"));
    assert!(names.contains(&"reserve_available_liquidity"));
    assert!(names.contains(&"reserve_borrowed_base_units"));
    assert!(names.contains(&"obligation_borrow_positions"));
    assert!(!names.iter().any(|n| n.contains("health")));
}

#[test]
fn the_modelled_boundary_is_the_reserves_available_liquidity() {
    let boundaries = ADAPTER.boundaries(&borrow_transaction(), &borrow_accounts_state(0));
    assert_eq!(boundaries.len(), 1);
    assert_eq!(boundaries[0].name, "amount_against_reserve_liquidity");
    // 500_000 of 50_000_000 available is one percent of the way to exhausting it.
    assert_eq!(boundaries[0].distance_bps, 9_900);
}

#[test]
fn the_adapter_is_registered_and_declares_its_contract() {
    let adapter = crate::protocol::adapter_for(PROGRAM_ID).expect("registered");
    assert_eq!(adapter.name(), "kamino-klend");
    assert_eq!(adapter.adapter_version(), 1);
    assert!(adapter.supports_cpi());
    assert_eq!(
        adapter.protocol_id().map(|p| p.as_str().to_string()),
        Some("kamino-klend".to_string())
    );
}
