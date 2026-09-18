//! Phase U1 guards.
//!
//! Two jobs. First, hold the refactor to its claim: the real production bundle
//! still verifies, the baseline still passes, and the canonical report is the
//! *same bytes* it was before any of this moved. Second, hold the universal
//! evidence layer to the one invariant that constrains its whole API — that a
//! measured absence of flow is not an absence of economic change.
//!
//! The bundle and its baseline are committed, so these run offline with no
//! build step and no network.

use eplyx_engine::{
    ci,
    evidence::{self, account, cpi, native, token},
    executor::{CpiCall, ExecutionResult},
    standard_programs::{spl_token, token2022, Decoded},
    types::{AccountSnapshot, NamedAccount},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

fn bundle() -> &'static Path {
    Path::new("../deploy/bundle")
}

/// The canonical report's sha256, as recorded in `docs/production-pilot.md`
/// from the deployed pilot and reproduced by local `eplyx ci check --format
/// json` before Phase U1 moved a single line.
const BASELINE_REPORT_SHA256: &str =
    "7be26a66f3e98f9b96ba6f1270003c158d99968d9ed081c336a97f14782d2099";

fn canonical(report: &ci::CiReport) -> String {
    // Byte-for-byte the artefact `eplyx ci check --format json` writes, which
    // is what the hosted service serves and what a team diffs: pretty-printed
    // serde output plus the trailing newline the CLI emits. The newline is part
    // of the file, so it is part of the hash the pilot recorded.
    let json = serde_json::to_string_pretty(report).expect("report serializes");
    format!("{:x}", Sha256::digest(format!("{json}\n").as_bytes()))
}

/// §47. The production bundle, the real baseline, and the exact report bytes.
///
/// This is the test that would have caught the refactor going wrong anywhere
/// between decoding an account and rendering a verdict. If it fails, the
/// refactor changed behaviour and the correct response is to find out why, not
/// to update the constant.
#[test]
fn the_production_bundle_still_passes_with_byte_identical_output() {
    let baseline = bundle().join("binaries/current.so");
    assert!(
        baseline.exists(),
        "the committed pilot bundle is missing: {}",
        baseline.display()
    );
    let report = ci::check(bundle(), &baseline, None).expect("the gate reaches a verdict");

    assert!(
        report.summary.passed,
        "{:?}",
        report.summary.failure_reasons
    );
    assert_eq!(report.exit_code(), 0);
    assert_eq!(report.bundle.record_count, 10);
    assert_eq!(report.bundle.adapter, "spl-stake-pool");
    assert_eq!(report.bundle.adapter_version, 3);
    assert_eq!(
        report.bundle.sha256,
        "5e5b67ac13e4f6b8249348ad81db29885ee8ee897ba78f6de213b55793f4285f"
    );
    // Coverage is a property of the observations, not of what differed, so a
    // passing run still declares what it was able to measure.
    assert_eq!(report.coverage.len(), 6);
    assert!(
        !report.bundle.limitations.is_empty(),
        "a pass still carries the corpus's known limitations"
    );

    assert_eq!(
        canonical(&report),
        BASELINE_REPORT_SHA256,
        "the canonical report is no longer byte-identical to the pre-refactor one"
    );
}

/// §47. The known regression behaves identically: same exit code, same
/// fingerprints, same counts.
///
/// The candidate is locally built, so this states what it needs rather than
/// skipping — a silent skip is how a guard stops guarding.
#[test]
fn the_known_regression_still_fails_with_the_same_findings() {
    let candidate = Path::new("../artifacts/fixture_stake_pool_v2.so");
    assert!(
        candidate.exists(),
        "run ./scripts/build-stake-pool-candidate.sh first: {} is missing",
        candidate.display()
    );
    let report = ci::check(bundle(), candidate, None).expect("the gate reaches a verdict");

    assert!(!report.summary.passed);
    assert_eq!(report.exit_code(), 1);

    let fingerprints: Vec<String> = report
        .findings
        .iter()
        .map(|finding| finding.fingerprint.to_string())
        .collect();
    assert!(
        fingerprints
            .iter()
            .any(|f| f == "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased"),
        "{fingerprints:?}"
    );
    assert!(
        fingerprints
            .iter()
            .any(|f| f == "spl-stake-pool/withdraw_sol/execution/transaction/now_reverts"),
        "{fingerprints:?}"
    );
    assert_eq!(fingerprints.len(), 2, "{fingerprints:?}");
}

// ---------------------------------------------------------------------------
// The invariant the universal layer's API exists to protect
// ---------------------------------------------------------------------------

const SPL: &str = spl_token::PROGRAM_ID;

fn snapshot(owner: &str, lamports: u64, data: Vec<u8>) -> AccountSnapshot {
    AccountSnapshot {
        lamports,
        owner: owner.into(),
        data,
        executable: false,
        rent_epoch: 0,
    }
}

fn named(label: &str, owner: &str, lamports: u64, data: Vec<u8>) -> NamedAccount {
    NamedAccount {
        label: label.into(),
        address: format!("address-of-{label}"),
        account: snapshot(owner, lamports, data),
    }
}

fn execution(accounts: Vec<(&str, AccountSnapshot)>, cpi_calls: Vec<CpiCall>) -> ExecutionResult {
    ExecutionResult {
        version: "v1".into(),
        success: true,
        error: None,
        compute_units: Some(1_000),
        fee: 5_000,
        logs: Vec::new(),
        cpi_calls,
        accounts: accounts
            .into_iter()
            .map(|(label, s)| (label.to_string(), s))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn token_bytes(mint: u8, amount: u64) -> Vec<u8> {
    let mut data = vec![0_u8; spl_token::ACCOUNT_LEN];
    data[0..32].copy_from_slice(&[mint; 32]);
    data[32..64].copy_from_slice(&[2_u8; 32]);
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    data
}

/// §45, and the reason [`evidence::FlowEvidence`] is shaped the way it is.
///
/// A program-owned account whose internal accounting moved produces **no**
/// token delta, **no** mint delta and **no** lamport delta. That is the Drift
/// `settlePNL` shape. The universal layer must report the change through a
/// different primitive, and its flow set's emptiness must not be usable as a
/// verdict.
#[test]
fn an_empty_universal_flow_set_is_not_an_empty_change_set() {
    let program = "dRiftyHA3jZfMwd1XerVmmvcMGCsBLzbAXnPabcdefgh";
    let mut before = vec![0_u8; 256];
    before[64..72].copy_from_slice(&1_000_u64.to_le_bytes());
    let mut after = before.clone();
    after[64..72].copy_from_slice(&1_500_u64.to_le_bytes());

    let pre = vec![named("position", program, 2_039_280, before)];
    let result = execution(
        vec![("position", snapshot(program, 2_039_280, after))],
        Vec::new(),
    );
    let measured = evidence::derive("settle-pnl", &pre, &result);

    // Every flow primitive is empty.
    assert!(measured.flows.no_flow_observed());
    assert!(measured.flows.token_deltas().is_empty());
    assert!(measured.flows.mint_deltas().is_empty());
    assert!(measured.flows.transfers().is_empty());
    assert!(measured.flows.mints().is_empty());
    assert!(measured.flows.burns().is_empty());
    assert!(measured.native.is_empty());

    // And the state plainly moved.
    assert_eq!(measured.accounts.len(), 1);
    assert!(measured.accounts[0].data_changed());
    assert_eq!(measured.accounts[0].first_data_difference(), Some(64));

    // The bytes even sit where a token account's `amount` would, and they are
    // still not a token delta: the account is owned by neither token program,
    // so nothing in this layer will claim to have read a balance out of it.
    assert_eq!(
        token::TokenProgram::of(program),
        None,
        "a layout must never be inferred from bytes alone"
    );
}

/// The converse, so the test above is not passing for a trivial reason: when
/// there *is* flow, the same derivation measures it.
#[test]
fn flow_is_measured_when_there_is_flow() {
    let pre = vec![named("holder", SPL, 2_039_280, token_bytes(1, 1_000))];
    let result = execution(
        vec![("holder", snapshot(SPL, 2_039_280, token_bytes(1, 400)))],
        Vec::new(),
    );
    let measured = evidence::derive("transfer", &pre, &result);
    assert!(!measured.flows.no_flow_observed());
    assert_eq!(measured.flows.token_deltas()[0].delta, -600);
}

/// §48 mutation 5. A fee payer's balance change is not protocol value movement,
/// and the attribution is what says so.
#[test]
fn a_fee_payer_lamport_change_is_never_value_movement() {
    let pre = vec![named(
        "payer",
        "11111111111111111111111111111111",
        1_000_000,
        Vec::new(),
    )];
    let result = execution(
        vec![(
            "payer",
            snapshot("11111111111111111111111111111111", 995_000, Vec::new()),
        )],
        Vec::new(),
    );
    let measured = evidence::derive("r", &pre, &result);
    let mut deltas = measured.native;
    assert_eq!(deltas.len(), 1);
    native::attribute_fee_payer(&mut deltas, "payer");
    assert_eq!(deltas[0].attribution, native::Attribution::FeePayer);
    assert!(!deltas[0].may_be_value_movement());
}

/// §48 mutation 9. A created account has no "before", so its whole balance is
/// not a delta. Attributing a fresh rent deposit to the protocol as a transfer
/// is the failure this prevents.
#[test]
fn a_created_account_is_never_reported_as_a_modified_existing_one() {
    let result = execution(
        vec![("fresh", snapshot(SPL, 2_039_280, token_bytes(1, 500)))],
        Vec::new(),
    );
    let measured = evidence::derive("r", &[], &result);
    assert_eq!(measured.accounts[0].existence, account::Existence::Created);
    assert_eq!(measured.accounts[0].lamport_delta(), None);
    assert!(measured.native.is_empty());
    assert!(measured.flows.token_deltas().is_empty());
    assert!(matches!(
        measured.lifecycle.as_slice(),
        [account::LifecycleEvent::AccountCreated { .. }]
    ));
}

/// §48 mutation 4. Accounts pair by label, which is address-backed. Pairing by
/// index would compare a mint against a token account.
///
/// The labels sort in the opposite order to the vector on purpose. The
/// post-state is a label-keyed map, so with alphabetically-ordered input the
/// two strategies agree and the test proves nothing — which is exactly what
/// mutation testing caught the first time this was written.
#[test]
fn accounts_never_pair_by_position() {
    let pre = vec![
        named("zebra", SPL, 10, token_bytes(1, 100)),
        named("alpha", SPL, 20, token_bytes(1, 200)),
    ];
    let result = execution(
        vec![
            ("alpha", snapshot(SPL, 20, token_bytes(1, 250))),
            ("zebra", snapshot(SPL, 10, token_bytes(1, 100))),
        ],
        Vec::new(),
    );
    let measured = evidence::derive("r", &pre, &result);
    let deltas = measured.flows.token_deltas();
    let zebra = deltas.iter().find(|d| d.label() == "zebra").unwrap();
    let alpha = deltas.iter().find(|d| d.label() == "alpha").unwrap();
    // Pairing by position would report zebra as +150 and alpha as -100.
    assert_eq!(zebra.delta, 0);
    assert_eq!(alpha.delta, 50);
}

/// §48 mutation 7. A CPI child must attach to a caller under its own top-level
/// instruction, never to a sibling instruction's frame.
#[test]
fn a_cpi_child_never_attaches_across_top_level_instructions() {
    let calls = vec![
        CpiCall {
            program: "pool".into(),
            stack_height: 2,
            outer_index: 0,
            account_count: 10,
            data_len: 9,
            discriminant: Some(14),
        },
        CpiCall {
            program: "token".into(),
            stack_height: 3,
            outer_index: 1,
            account_count: 4,
            data_len: 9,
            discriminant: Some(7),
        },
    ];
    let graph = cpi::CpiGraph::build(&calls);
    assert_eq!(graph.nodes[1].parent, None);
    assert_eq!(graph.roots().count(), 2);
}

/// §48 mutations 1 and 10. One shared decoder, strict about length, and a
/// malformed account is never silently accepted as a balance.
#[test]
fn the_shared_token_decoder_is_strict_about_what_it_will_read() {
    // A real 611-byte stake-pool account begins with `1` and must never read as
    // a token account. This is the defect the repository already has a scar
    // from, and the length check is what prevents it.
    let mut pool = vec![0_u8; 611];
    pool[0] = 1;
    assert_eq!(spl_token::decode_account(&pool), Decoded::NotApplicable);
    assert_eq!(spl_token::account_amount(&pool), None);

    // A malformed account is visible as malformed, not accepted as zero.
    let mut broken = token_bytes(1, 500);
    broken[108] = 9;
    assert!(spl_token::decode_account(&broken).is_malformed());
    assert_eq!(spl_token::account_amount(&broken), None);

    // And the amount really is read at the layout's offset.
    assert_eq!(
        spl_token::account_amount(&token_bytes(1, 123_456)),
        Some(123_456)
    );
}

/// §48 mutation 8. The Stake Pool adapter reads token accounts through the
/// shared *strict* decoder, so a Token-2022 extended account is refused rather
/// than read under this adapter's narrower claim.
#[test]
fn the_stake_pool_adapter_does_not_read_extended_token_accounts() {
    use eplyx_engine::protocol::stake_pool;

    let legacy = token_bytes(1, 777);
    assert_eq!(stake_pool::token_account_amount(&legacy), Some(777));

    let mut extended = legacy.clone();
    extended.resize(token2022::ACCOUNT_TYPE_OFFSET, 0);
    extended.push(2);
    extended.extend_from_slice(&token2022::TRANSFER_FEE_AMOUNT.to_le_bytes());
    extended.extend_from_slice(&8_u16.to_le_bytes());
    extended.extend_from_slice(&50_u64.to_le_bytes());

    assert_eq!(
        stake_pool::token_account_amount(&extended),
        None,
        "the SPL Stake Pool contract admits legacy token accounts only"
    );
    // The Token-2022 adapter, whose contract does admit them, reads it.
    assert_eq!(
        eplyx_engine::protocol::token2022::token_account_amount(&extended),
        Some(777)
    );
}

/// §48 mutation 2. A withheld transfer fee is spendable value parked in an
/// account, and the shared decoder must surface it rather than dropping it.
#[test]
fn a_withheld_transfer_fee_is_never_dropped() {
    let mut extended = token_bytes(1, 100);
    extended.resize(token2022::ACCOUNT_TYPE_OFFSET, 0);
    extended.push(2);
    extended.extend_from_slice(&token2022::TRANSFER_FEE_AMOUNT.to_le_bytes());
    extended.extend_from_slice(&8_u16.to_le_bytes());
    extended.extend_from_slice(&4_242_u64.to_le_bytes());

    let list = token2022::extensions(&extended);
    let value = list
        .value(token2022::TRANSFER_FEE_AMOUNT)
        .expect("the withheld fee is present");
    assert_eq!(
        token2022::decode_extension(token2022::TRANSFER_FEE_AMOUNT, value),
        token2022::Extension::TransferFeeAmount {
            withheld_amount: 4_242
        }
    );

    // And the adapter reports it as an economic field.
    use eplyx_engine::protocol::{adapter_for, token2022 as adapter};
    let decoded = adapter_for(adapter::PROGRAM_ID)
        .expect("adapter")
        .decode(&snapshot(token2022::PROGRAM_ID, 1, extended))
        .expect("decodes");
    let field = decoded
        .field("withheld_transfer_fee")
        .expect("the withheld fee reaches the report");
    assert!(field.economic);
    assert_eq!(field.value.render(), "4242");
}

/// §48 mutation 3. A read-only account whose owner changed is a boundary
/// failure, exactly as one whose data changed is.
#[test]
fn a_read_only_owner_change_fails_the_boundary_proof() {
    use eplyx_engine::evidence::boundary::{self, BoundaryContract, Side};
    use eplyx_engine::ingest::transactions::{HistoricalTransaction, TokenBalance};
    use eplyx_engine::types::AccountMetaSpec;

    let contract = BoundaryContract {
        token_program: SPL,
        token_program_description: "the SPL Token program",
        token_account_description: "base-layout token account",
        account_amount: spl_token::account_amount,
        account_mint: spl_token::account_mint,
        read_only_exempt: &[],
        interference_hint: |side, slot| {
            format!(
                "slot {slot} {}",
                if side == Side::Pre { "before" } else { "after" }
            )
        },
    };

    let mut tx: HistoricalTransaction = serde_json::from_str(
        r#"{"signature":"s","slot":100,"block_time":null,"version":"legacy",
            "recent_blockhash":"b","payer":"holder","account_keys":[],
            "instructions":[],"inner_instructions":[],"success":true,
            "error":null,"fee":5000,"compute_units":null,"logs":[]}"#,
    )
    .expect("fixture");
    tx.account_keys = vec![
        AccountMetaSpec {
            address: "holder".into(),
            is_signer: true,
            is_writable: true,
        },
        AccountMetaSpec {
            address: "reference".into(),
            is_signer: false,
            is_writable: false,
        },
    ];
    tx.pre_balances = Some(vec![2_039_280, 1_000_000]);
    tx.post_balances = Some(vec![2_039_280, 1_000_000]);
    let balance = |amount| {
        Some(vec![TokenBalance {
            account_index: 0,
            mint: bs58::encode([1_u8; 32]).into_string(),
            program_id: SPL.into(),
            amount,
            decimals: 6,
        }])
    };
    tx.pre_token_balances = balance(500);
    tx.post_token_balances = balance(300);

    let at = |address: &str, owner: &str, lamports, data: Vec<u8>| NamedAccount {
        label: address.into(),
        address: address.into(),
        account: snapshot(owner, lamports, data),
    };
    let pre = vec![
        at("holder", SPL, 2_039_280, token_bytes(1, 500)),
        at("reference", SPL, 1_000_000, vec![7; 16]),
    ];
    let clean = vec![
        at("holder", SPL, 2_039_280, token_bytes(1, 300)),
        at("reference", SPL, 1_000_000, vec![7; 16]),
    ];
    boundary::prove(&contract, &tx, &pre, &clean).expect("a clean boundary proves");

    let mut owner_changed = clean.clone();
    owner_changed[1].account.owner = "11111111111111111111111111111111".into();
    let error = boundary::prove(&contract, &tx, &pre, &owner_changed)
        .expect_err("an owner change must fail the proof")
        .to_string();
    assert!(
        error.contains("read-only account reference changed"),
        "{error}"
    );
}
