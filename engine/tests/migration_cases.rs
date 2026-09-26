//! Token Migration V1 end-to-end cases A–H over synthetic worlds built with the real
//! captured SPL Token / Token-2022 / ATA programs:
//! spec → package → fixture → plan → VM rehearsal → invariants → search →
//! counterexample → offline replay (reproduce) → gate, plus the unsigned plan
//! re-executed from its serialized descriptors in a fresh local VM.
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    migration::gate::Policy,
    migration::{
        execute,
        invariants::MigrationInvariant,
        pipeline,
        search::{self, Counterexample, SearchFinding},
        unsigned::{self, UnsignedPlan},
    },
};
use migration_common::*;
use serde_json::{json, Value};

struct Case {
    report: Value,
    strict: Value,
    search: search::SearchResult,
    cross: Vec<unsigned::CrossCheck>,
}

fn run_case(
    label: &str,
    recipe_value: Value,
    source: &str,
    destination: &str,
    patch: Value,
    program: &[u8],
) -> Case {
    let recipe = recipe(recipe_value);
    let spec = spec(&recipe, source, destination, patch);
    let (root, validated) = package(
        label,
        &recipe,
        &spec,
        program,
        MigrationInvariant::recommended(),
    );
    let result = root.join("result");
    let report = pipeline::run(
        validated.root(),
        &result,
        Policy::BlockOnly,
        pipeline::Isolation::InProcess,
    )
    .unwrap();
    assert_eq!(
        pipeline::replay(validated.root(), &result).unwrap(),
        report,
        "{label}: offline replay"
    );
    let strict =
        pipeline::replay_with_policy(validated.root(), &result, Some(Policy::Strict)).unwrap();
    let search_dir = root.join("search");
    let outcome = search::run(validated.root(), &result, &search_dir).unwrap();
    assert_eq!(
        search::replay(validated.root(), &result, &search_dir).unwrap(),
        outcome,
        "{label}: search replay"
    );
    let plan: UnsignedPlan =
        serde_json::from_slice(&std::fs::read(result.join(pipeline::UNSIGNED)).unwrap()).unwrap();
    let b = pipeline::bindings(&result).unwrap();
    let world = pipeline::world_for(&validated, &result, &b).unwrap();
    let programs =
        execute::programs(&world, &spec, validated.program_id(), validated.candidate()).unwrap();
    let cross = unsigned::cross_check(&plan, &world, &programs).unwrap();
    assert!(
        cross.iter().all(|c| c.matches_rehearsal),
        "{label}: unsigned plan diverges from rehearsal: {cross:#?}"
    );
    let recorded = &report["unsigned_plan"]["cross_check"];
    assert_eq!(recorded["executed_units"], cross.len(), "{label}");
    assert_eq!(recorded["matching_units"], cross.len(), "{label}");
    assert_eq!(report["official_transition"], "NotTested");
    assert_eq!(report["funds_moved"], false);
    assert_t0(label, &report, serde_json::to_value(&outcome).unwrap());
    Case {
        report,
        strict,
        search: outcome,
        cross,
    }
}

fn base(
    id: &str,
    source_program: &str,
    destination_program: &str,
    source_ext: Value,
    destination_ext: Value,
    accounts: Value,
) -> Value {
    json!({
        "schemaVersion": 1, "id": id, "description": id,
        "programs": "pinnedMainnetCapture",
        "clock": {"slot": "5000", "unixTimestamp": "1760000000", "epoch": "400"},
        "populationMint": "source",
        "wallets": wallets(&["alice", "bob", "carol", "dave", "issuer", "m1", "m2", "m3", "hook"]),
        "multisigs": [{"label": "treasury", "tokenProgram": source_program, "threshold": 2, "signers": ["m1", "m2", "m3"]}],
        "programOwned": [{"label": "pool"}],
        "mints": [
            {"label": "source", "tokenProgram": source_program, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}, "extensions": source_ext},
            {"label": "destination", "tokenProgram": destination_program, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}, "extensions": destination_ext}
        ],
        "tokenAccounts": accounts
    })
}

fn holders() -> Value {
    json!([
        {"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "2500000"},
        {"label": "bob-source", "mint": "source", "owner": {"label": "bob"}, "layout": "associated", "amount": "300001"},
        {"label": "bob-destination", "mint": "destination", "owner": {"label": "bob"}, "layout": "associated", "amount": "0"},
        {"label": "treasury-source", "mint": "source", "owner": {"label": "treasury"}, "layout": "associated", "amount": "70000"}
    ])
}

fn gate(report: &Value) -> &str {
    report["gate_outcome"].as_str().unwrap()
}

#[test]
fn case_a_legacy_to_legacy() {
    let c = run_case(
        "case-a",
        base("case-a", LEGACY, LEGACY, json!([]), json!([]), holders()),
        LEGACY,
        LEGACY,
        json!({}),
        &reference(),
    );
    assert_eq!(
        c.report["declared_preflight_status"], "Ready",
        "{:#}",
        c.report["readiness"]
    );
    assert_eq!(c.report["reconciliation"]["status"], "FullyReconciled");
    assert_eq!(gate(&c.report), "Pass");
    assert_eq!(gate(&c.strict), "Pass");
    assert!(c.search.counterexamples.is_empty());
    assert_eq!(c.cross.len(), 3);
}

#[test]
fn case_b_legacy_to_token_2022_with_destination_transfer_fee() {
    let c = run_case(
        "case-b",
        base(
            "case-b",
            LEGACY,
            T22,
            json!([]),
            json!([{"kind": "transferFee", "bps": 25, "maximumFee": "1000"}]),
            holders(),
        ),
        LEGACY,
        T22,
        json!({}),
        &reference(),
    );
    assert_eq!(
        c.report["declared_preflight_status"], "Ready",
        "{:#}",
        c.report["readiness"]
    );
    let rec = &c.report["reconciliation"];
    assert_ne!(
        rec["destination_withheld_fee_raw"], "0",
        "holders receive net of the Token-2022 transfer fee"
    );
    let findings = c.report["compatibility"]["mint_findings"].to_string();
    assert!(findings.contains("TRANSFER_FEE_WITHHELD"), "{findings}");
    assert!(c.search.counterexamples.is_empty());
}

#[test]
fn case_c_token_2022_supported_extension_subset_with_escrow() {
    let source_ext = json!([
        {"kind": "transferFee", "bps": 10, "maximumFee": "500"},
        {"kind": "permanentDelegate", "delegate": {"label": "issuer"}},
        {"kind": "metadataPointer", "authority": {"label": "issuer"}},
        {"kind": "interestBearing", "rateBps": 50},
        {"kind": "mintCloseAuthority", "authority": {"label": "issuer"}}
    ]);
    let destination_ext = json!([{"kind": "transferFee", "bps": 30, "maximumFee": "100000"}, {"kind": "metadataPointer", "authority": {"label": "issuer"}}]);
    let mut accounts = holders();
    accounts.as_array_mut().unwrap().push(json!({"label": "carol-source", "mint": "source", "owner": {"label": "carol"}, "layout": "associated", "amount": "999", "extensions": ["memoTransfer"]}));
    let c = run_case(
        "case-c",
        base("case-c", T22, T22, source_ext, destination_ext, accounts),
        T22,
        T22,
        json!({"sourceDisposition": {"kind": "escrow"}, "conversion": {"ratioBasis": "raw", "numerator": "3", "denominator": "2", "rounding": "floor", "fee": {"kind": "sourceBps", "bps": 50}, "minimumOutputRaw": "1"}}),
        &reference(),
    );
    assert_eq!(
        c.report["readiness"]["mechanism"]["status"], "Ready",
        "{:#}",
        c.report["readiness"]
    );
    assert_eq!(
        c.report["readiness"]["population"]["status"], "Ready",
        "{:#}",
        c.report["readiness"]
    );
    let rec = &c.report["reconciliation"];
    assert_ne!(
        rec["source_escrow_withheld_fee_raw"], "0",
        "escrow receives net of the source transfer fee"
    );
    assert_eq!(rec["source_burned_raw"], "0");
    assert!(
        c.search.counterexamples.is_empty(),
        "{:#?}",
        c.search.counterexamples
    );
}

#[test]
fn case_d_insufficient_destination_reserve() {
    let c = run_case(
        "case-d",
        base("case-d", LEGACY, LEGACY, json!([]), json!([]), holders()),
        LEGACY,
        LEGACY,
        json!({"destinationFunding": {"kind": "reserveTransfer", "reserve": {"kind": "proposed", "fundedRaw": "1200000"}}}),
        &reference(),
    );
    assert_eq!(c.report["readiness"]["funding"]["status"], "Blocked");
    assert!(c.report["gate_reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("INSUFFICIENT_RESERVE")));
    assert_eq!(gate(&c.report), "Block");
    assert_eq!(gate(&c.strict), "Block");
    assert!(c
        .search
        .counterexamples
        .iter()
        .any(|x| x.finding() == SearchFinding::EligibleHolderNotMigrated));
    assert!(c.search.trace.iter().any(|p| p.kind == "PopulationReserve"
        && p.signature.error_name.as_deref() == Some("InsufficientReserve")));
}

#[test]
fn case_e_rounding_boundary() {
    let accounts = json!([
        {"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "2"},
        {"label": "bob-source", "mint": "source", "owner": {"label": "bob"}, "layout": "associated", "amount": "3"},
        {"label": "carol-source", "mint": "source", "owner": {"label": "carol"}, "layout": "associated", "amount": "1000"}
    ]);
    let c = run_case(
        "case-e",
        base("case-e", LEGACY, T22, json!([]), json!([]), accounts),
        LEGACY,
        T22,
        json!({"conversion": {"ratioBasis": "raw", "numerator": "1", "denominator": "3", "rounding": "floor", "fee": {"kind": "none"}, "minimumOutputRaw": "1"}}),
        &reference(),
    );
    let classes = c.report["impact"]["classes"].to_string();
    assert!(
        classes.contains("OutputBelowMinimum"),
        "2 raw converts to 0 under floor: {classes}"
    );
    let rec = &c.report["reconciliation"];
    assert_eq!(rec["rounding_denominator"], "3");
    assert_ne!(
        rec["rounding_delta_numerator"], "0",
        "floor keeps dust back: {rec:#}"
    );
    let stress = c.report["execution"]["stress_cases"].as_array().unwrap();
    for kind in [
        "RoundingExact",
        "RoundingDust",
        "BelowMinimumAmount",
        "MinimumMigratableAmount",
    ] {
        let case = stress
            .iter()
            .find(|s| s["kind"] == kind)
            .unwrap_or_else(|| panic!("missing {kind}"));
        assert_eq!(case["behaves_as_specified"], true, "{case:#}");
    }
    assert_eq!(gate(&c.report), "Warn");
}

#[test]
fn case_f_frozen_and_unsupported_holders() {
    let mut accounts = holders();
    accounts.as_array_mut().unwrap().extend([
        json!({"label": "carol-source", "mint": "source", "owner": {"label": "carol"}, "layout": "associated", "amount": "5000", "frozen": true}),
        json!({"label": "pool-source", "mint": "source", "owner": {"label": "pool"}, "layout": "associated", "amount": "8000"}),
        json!({"label": "dave-source", "mint": "source", "owner": {"label": "dave"}, "layout": "associated", "amount": "6000", "extensions": ["cpiGuard"]}),
    ]);
    let c = run_case(
        "case-f",
        base("case-f", T22, T22, json!([]), json!([]), accounts),
        T22,
        T22,
        json!({}),
        &reference(),
    );
    let codes = c.report["readiness"]["population"]["codes"].to_string();
    for code in [
        "SOURCE_FROZEN",
        "AUTHORITY_PATH_UNAVAILABLE",
        "UNSUPPORTED_TOKEN_EXTENSION",
    ] {
        assert!(codes.contains(code), "{code} missing from {codes}");
    }
    assert_eq!(c.report["readiness"]["mechanism"]["status"], "Ready");
    assert_eq!(
        gate(&c.report),
        "Warn",
        "unsupported holders are incomplete, not failures"
    );
    assert_eq!(gate(&c.strict), "Block");
    assert_eq!(
        c.report["reconciliation"]["status"], "ReconciledForExecutedUnits",
        "never FullyReconciled with stranded holders"
    );
}

#[test]
fn case_g_authority_mismatch() {
    // The specification expects a source mint authority the captured mint does not have.
    let wrong = "11111111111111111111111111111112";
    let c = run_case(
        "case-g",
        base("case-g", LEGACY, LEGACY, json!([]), json!([]), holders()),
        LEGACY,
        LEGACY,
        json!({"authorities": {"migrationAuthority": {"kind": "programDerived"}, "expected": {"sourceMintAuthority": {"kind": "address", "address": wrong}}, "feePayer": {"kind": "relayer"}}}),
        &reference(),
    );
    assert!(c.report["gate_reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("REQUIRED_AUTHORITY_MISMATCH")));
    assert_eq!(
        gate(&c.report),
        "Block",
        "a violated blocking invariant blocks under block-only"
    );
    let stress = c.report["execution"]["stress_cases"].as_array().unwrap();
    let wrong_signer = stress.iter().find(|s| s["kind"] == "WrongSigner").unwrap();
    assert_eq!(wrong_signer["error_name"], "UnauthorizedHolderAuthority");
    // Mint-to without the migration authority as mint authority: no path, never assumed.
    let mint = run_case(
        "case-g-mint",
        base("case-g-mint", LEGACY, T22, json!([]), json!([]), holders()),
        LEGACY,
        T22,
        json!({"destinationFunding": {"kind": "mintTo"}}),
        &reference(),
    );
    assert_eq!(mint.report["readiness"]["funding"]["status"], "Blocked");
    assert!(mint.report["readiness"]["funding"]["codes"]
        .to_string()
        .contains("MINT_AUTHORITY_MISMATCH"));
    assert!(
        mint.cross.is_empty(),
        "no unsigned unit is emitted without a funding path"
    );
}

#[test]
fn case_h_unsupported_token_2022_extension_semantics() {
    let destination_ext = json!([{"kind": "transferHook", "program": {"label": "hook"}}]);
    let c = run_case(
        "case-h",
        base("case-h", LEGACY, T22, json!([]), destination_ext, holders()),
        LEGACY,
        T22,
        json!({}),
        &reference(),
    );
    let findings = c.report["compatibility"]["mint_findings"].to_string();
    assert!(
        findings.contains("TRANSFER_HOOK_ACCOUNTS_REQUIRED")
            && findings.contains("RequiresProtocolSpecificHandling"),
        "{findings}"
    );
    assert!(c.report["readiness"]["population"]["codes"]
        .to_string()
        .contains("MINT_STATE_BLOCKS_MIGRATION"));
    assert_eq!(
        c.report["coverage"]["rehearsal"]["attempted"], 0,
        "unsupported semantics are never executed as legacy transfers"
    );
    assert_eq!(gate(&c.strict), "Block");
    // Paused source mint.
    let paused = run_case(
        "case-h-paused",
        {
            let mut v = base(
                "case-h-paused",
                T22,
                LEGACY,
                json!([{"kind": "pausable", "authority": {"label": "issuer"}}]),
                json!([]),
                holders(),
            );
            v["postSteps"] = json!([{"kind": "pause", "mint": "source"}]);
            v
        },
        T22,
        LEGACY,
        json!({}),
        &reference(),
    );
    assert!(paused.report["compatibility"]["mint_findings"]
        .to_string()
        .contains("MINT_PAUSED"));
}

#[test]
fn defect_counterexample_is_reproducible_from_the_saved_search() {
    let c = run_case(
        "case-defect",
        base(
            "case-defect",
            LEGACY,
            LEGACY,
            json!([]),
            json!([]),
            holders(),
        ),
        LEGACY,
        LEGACY,
        json!({"conversion": {"ratioBasis": "raw", "numerator": "1", "denominator": "2", "rounding": "floor", "fee": {"kind": "sourceBps", "bps": 33}, "minimumOutputRaw": "1"}}),
        &candidate("eplyx_token_migration_defect_fee_ceiling"),
    );
    assert_eq!(c.report["readiness"]["mechanism"]["status"], "Blocked");
    assert!(c.search.counterexamples.iter().any(|x| matches!(
        x,
        Counterexample::MigrationObserved {
            finding: SearchFinding::ReconciliationMismatch,
            ..
        }
    )));
    assert_eq!(gate(&c.report), "Block");
}
