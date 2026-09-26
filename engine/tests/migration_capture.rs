//! The mainnet-state path of a Token Migration V1 preflight, end to end, against a
//! recorded mock provider. The mock answers only read-only JSON-RPC methods from a
//! synthetic world built with the pinned mainnet token programs, so the captured
//! world can be compared with the same world rehearsed as a fixture. No network is
//! used; this checks capture, reconstruction and offline replay, not any real chain.
#[path = "common/migration.rs"]
mod migration_common;
use anyhow::{bail, Result};
use eplyx_engine::{
    ingest::rpc::RpcProvider as SolanaRpc,
    migration::gate::Policy,
    migration::population_types::StressBudget,
    migration::{
        input::{self as package, Config, StateSource},
        invariants::MigrationInvariant,
        pipeline::{self, Isolation, ObservedSource},
        planner::RehearsalClockPolicy,
        world::{World, MAINNET_GENESIS},
    },
};
use migration_common::*;
use serde_json::{json, Value};
use std::{str::FromStr, sync::Mutex};

const READ_ONLY: [&str; 4] = [
    "getGenesisHash",
    "getAccountInfo",
    "getProgramAccounts",
    "getMultipleAccounts",
];

/// A provider over one immutable world. Every response carries a context slot at
/// or above the request's `minContextSlot`.
struct WorldRpc {
    world: World,
    genesis: String,
    slot: u64,
    calls: Mutex<Vec<(String, Value)>>,
}

impl WorldRpc {
    fn context(&self, params: &Value) -> Value {
        let min = params
            .as_array()
            .and_then(|p| p.last())
            .and_then(|c| c["minContextSlot"].as_u64())
            .unwrap_or(0);
        json!({"slot": self.slot.max(min)})
    }
}

impl SolanaRpc for WorldRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        let context = self.context(&params);
        Ok(match method {
            "getGenesisHash" => json!(self.genesis),
            "getAccountInfo" => {
                json!({"context": context, "value": self.world.rpc_value(params[0].as_str().unwrap())})
            }
            "getMultipleAccounts" => {
                let values: Vec<Value> = params[0]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| self.world.rpc_value(a.as_str().unwrap()))
                    .collect();
                json!({"context": context, "value": values})
            }
            "getProgramAccounts" => {
                let program = params[0].as_str().unwrap();
                let filter = &params[1]["filters"][0]["memcmp"];
                assert_eq!(filter["offset"], 0, "the scan filters on the mint field");
                let mint = solana_address::Address::from_str(filter["bytes"].as_str().unwrap())
                    .unwrap()
                    .to_bytes();
                let rows: Vec<Value> = self
                    .world
                    .accounts
                    .iter()
                    .filter(|(_, a)| a.account.owner == program && a.account.data.starts_with(&mint))
                    .map(|(address, a)| {
                        json!({"pubkey": address, "account": eplyx_engine::migration::world::rpc_value(&a.account)})
                    })
                    .collect();
                json!({"context": context, "value": rows})
            }
            other => bail!("the mock provider refuses {other}"),
        })
    }
}

fn recipe() -> eplyx_engine::migration::fixture::Recipe {
    // The Clock sits after the pinned programs' deployment slots, as a real
    // finalized mainnet bank would.
    migration_common::recipe(json!({
        "schemaVersion": 1, "id": "capture-mock", "description": "mock mainnet capture",
        "programs": "pinnedMainnetCapture",
        "clock": {"slot": "448600000", "unixTimestamp": "1790000000", "epoch": "900"},
        "populationMint": "source",
        "wallets": wallets(&["alice", "bob", "carol", "issuer"]),
        "programOwned": [{"label": "pool"}],
        "mints": [
            {"label": "source", "tokenProgram": LEGACY, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}},
            {"label": "destination", "tokenProgram": T22, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}}
        ],
        "tokenAccounts": [
            {"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "1001"},
            {"label": "bob-source", "mint": "source", "owner": {"label": "bob"}, "layout": "associated", "amount": "20"},
            {"label": "bob-destination", "mint": "destination", "owner": {"label": "bob"}, "layout": "associated", "amount": "0"},
            {"label": "carol-source", "mint": "source", "owner": {"label": "carol"}, "layout": "associated", "amount": "33", "frozen": true},
            {"label": "pool-source", "mint": "source", "owner": {"label": "pool"}, "layout": "associated", "amount": "400"},
            {"label": "empty-source", "mint": "source", "owner": {"label": "issuer"}, "layout": "associated", "amount": "0"}
        ]
    }))
}

fn mainnet_package(
    label: &str,
    spec: &eplyx_engine::migration::spec::TokenMigrationV1,
) -> (std::path::PathBuf, package::ValidatedInput) {
    bounded_package(label, spec, 5000)
}

fn bounded_package(
    label: &str,
    spec: &eplyx_engine::migration::spec::TokenMigrationV1,
    max_captured_holders: usize,
) -> (std::path::PathBuf, package::ValidatedInput) {
    let root = temp_dir(label);
    std::fs::create_dir_all(&root).unwrap();
    let config = Config {
        state: StateSource::MainnetCapture,
        rehearsal_clock: RehearsalClockPolicy::Captured,
        max_rehearsal_units: 5000,
        max_captured_holders,
    };
    let validated = package::assemble(
        &root.join("package"),
        spec,
        eplyx_engine::migration::adapter::REFERENCE_PROGRAM_ID,
        &reference(),
        &config,
        None,
        MigrationInvariant::recommended(),
    )
    .unwrap();
    (root, validated)
}

fn classes(report: &Value) -> Vec<(String, u64)> {
    report["impact"]["classes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["class"].as_str().unwrap().into(),
                c["accounts"].as_u64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn mainnet_state_is_captured_read_only_rebuilt_and_replayed_offline() {
    let recipe = recipe();
    let spec = spec(&recipe, LEGACY, T22, json!({}));
    let world = world(&recipe, &spec);
    let rpc = WorldRpc {
        slot: world.clock.slot,
        world,
        genesis: MAINNET_GENESIS.into(),
        calls: Mutex::new(vec![]),
    };

    // The same world rehearsed as a synthetic fixture.
    let (fixture_root, fixture) = package(
        "capture-fixture",
        &recipe,
        &spec,
        &reference(),
        MigrationInvariant::recommended(),
    );
    let fixture_report = pipeline::run(
        fixture.root(),
        &fixture_root.join("result"),
        Policy::BlockOnly,
        Isolation::InProcess,
    )
    .unwrap();

    let (root, captured) = mainnet_package("capture-mainnet", &spec);
    let output = root.join("result");
    let report = pipeline::run_with(
        captured.root(),
        &output,
        Policy::BlockOnly,
        Isolation::InProcess,
        Some(ObservedSource {
            population: &rpc,
            execution: &rpc,
            budget: StressBudget::default(),
        }),
    )
    .unwrap();

    // Only read-only methods were used; nothing was built, signed or sent.
    let calls = rpc.calls.lock().unwrap();
    assert!(
        calls.iter().all(|(m, _)| READ_ONLY.contains(&m.as_str())),
        "{calls:?}"
    );
    // Every batch after the first asks for a context at or after the previous one.
    let mut floor = 0;
    for (method, params) in calls.iter().filter(|(m, _)| m == "getMultipleAccounts") {
        let min = params[1]["minContextSlot"].as_u64().unwrap_or(0);
        assert!(min >= floor, "{method} regressed its minContextSlot");
        floor = min;
    }

    assert_eq!(report["state"]["world_kind"], "ObservedCapture");
    assert_eq!(report["state"]["cluster"], "solana-mainnet");
    assert_eq!(fixture_report["state"]["world_kind"], "SyntheticFixture");
    assert!(report["coverage"]["world"]["observed_slots"].is_array());
    assert_eq!(
        classes(&report),
        classes(&fixture_report),
        "the captured world classifies every holder as the fixture world does"
    );
    assert_eq!(report["impact"]["population"]["token_accounts"], 5);
    assert_eq!(report["coverage"]["not_inspected"]["destinations"], 0);
    assert_eq!(report["coverage"]["not_inspected"]["owners"], 0);
    assert_eq!(
        report["reconciliation"]["status"],
        fixture_report["reconciliation"]["status"]
    );
    for equation in report["reconciliation"]["equations"].as_array().unwrap() {
        assert_eq!(equation["holds"], true, "{equation}");
    }
    assert_eq!(report["official_transition"], "NotTested");
    let saved = std::fs::read(output.join("report.json")).unwrap();
    for leak in ["mock-mainnet.invalid", "/tmp", "/private/", "/var/folders"] {
        assert!(
            !String::from_utf8_lossy(&saved).contains(leak),
            "report leaks {leak}"
        );
    }

    // Offline replay re-derives the identical report from the recorded captures.
    let replayed = pipeline::replay(captured.root(), &output).unwrap();
    assert_eq!(replayed, report);
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(fixture_root).unwrap();
}

#[test]
fn a_non_mainnet_provider_is_refused() {
    let recipe = recipe();
    let spec = spec(&recipe, LEGACY, T22, json!({}));
    let world = world(&recipe, &spec);
    let rpc = WorldRpc {
        slot: world.clock.slot,
        world,
        genesis: "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG".into(),
        calls: Mutex::new(vec![]),
    };
    let (root, captured) = mainnet_package("capture-devnet", &spec);
    let error = pipeline::run_with(
        captured.root(),
        &root.join("result"),
        Policy::BlockOnly,
        Isolation::InProcess,
        Some(ObservedSource {
            population: &rpc,
            execution: &rpc,
            budget: StressBudget::default(),
        }),
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("mainnet"),
        "unexpected error: {error:#}"
    );
    assert_eq!(eplyx_engine::migration::error::exit_code(&error), 4);
    assert!(!root.join("result/report.json").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_bounded_capture_reports_uninspected_holders_as_unverifiable() {
    let recipe = recipe();
    let spec = spec(&recipe, LEGACY, T22, json!({}));
    let world = world(&recipe, &spec);
    let rpc = WorldRpc {
        slot: world.clock.slot,
        world,
        genesis: MAINNET_GENESIS.into(),
        calls: Mutex::new(vec![]),
    };
    let (root, captured) = bounded_package("capture-bounded", &spec, 1);
    let report = pipeline::run_with(
        captured.root(),
        &root.join("result"),
        Policy::BlockOnly,
        Isolation::InProcess,
        Some(ObservedSource {
            population: &rpc,
            execution: &rpc,
            budget: StressBudget::default(),
        }),
    )
    .unwrap();
    let unverifiable = classes(&report)
        .into_iter()
        .find(|(class, _)| class == "UnverifiableDestination")
        .map(|(_, n)| n)
        .unwrap_or(0);
    assert_eq!(unverifiable, 1, "{:#}", report["impact"]["classes"]);
    // Four positive holders' owners, one inspected. The frozen and the
    // program-controlled holder keep their own class, but their uninspected
    // destinations are still counted rather than assumed.
    assert_eq!(report["coverage"]["not_inspected"]["destinations"], 3);
    assert_ne!(report["readiness"]["population"]["status"], "Ready");
    std::fs::remove_dir_all(root).unwrap();
}
