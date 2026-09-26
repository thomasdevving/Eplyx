#![allow(dead_code)]
#[path = "migration.rs"]
pub mod base;
use anyhow::{bail, Result};
use base::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use eplyx_engine::{
    ingest::rpc::RpcProvider as SolanaRpc,
    migration::{
        self,
        input::{self, Config, StateSource, ValidatedInput},
        invariants::MigrationInvariant,
        planner::RehearsalClockPolicy,
        population_types::StressBudget,
        world::{World, MAINNET_GENESIS},
    },
};
use serde_json::{json, Value};
use std::{
    str::FromStr,
    sync::{Mutex, OnceLock},
};
const READ_ONLY: [&str; 4] = [
    "getGenesisHash",
    "getAccountInfo",
    "getProgramAccounts",
    "getMultipleAccounts",
];

/// A provider over one immutable world. Every response carries a context slot at
/// or above the request's `minContextSlot`.
pub struct WorldRpc {
    pub world: World,
    pub genesis: String,
    pub slot: u64,
    pub calls: Mutex<Vec<(String, Value)>>,
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
                let mut rows: Vec<Value> = self
                    .world
                    .accounts
                    .iter()
                    .filter(|(_, a)| a.account.owner == program && a.account.data.starts_with(&mint))
                    .map(|(address, a)| {
                        json!({"pubkey": address, "account": eplyx_engine::migration::world::rpc_value(&a.account)})
                    })
                    .collect();
                rows.reverse();
                json!({"context": context, "value": rows})
            }
            other => bail!("the mock provider refuses {other}"),
        })
    }
}

fn fixture() -> &'static (migration::fixture::Recipe, World) {
    static FIXTURE: OnceLock<(migration::fixture::Recipe, World)> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let mut holders=vec![json!({"label":"issuer","lamports":"1000000000"})];let mut accounts=vec![];
        for i in 0..35 {holders.push(json!({"label":format!("w{i}"),"lamports":"1000000000"}));accounts.push(json!({"label":format!("s{i}"),"mint":"source","owner":{"label":format!("w{i}")},"layout":"associated","amount":((i+1)*1000).to_string()}));}
        accounts.push(json!({"label":"pool-source","mint":"source","owner":{"label":"pool"},"layout":"associated","amount":"90000"}));
        accounts.push(json!({"label":"empty","mint":"source","owner":{"label":"issuer"},"layout":"associated","amount":"0"}));
        let recipe=recipe(json!({"schemaVersion":1,"id":"current-mock","description":"T4 synthetic mock population","programs":"pinnedMainnetCapture","clock":{"slot":"448600000","unixTimestamp":"1790000000","epoch":"900"},"populationMint":"source","wallets":holders,"programOwned":[{"label":"pool"}],"mints":[{"label":"source","tokenProgram":LEGACY,"decimals":6,"mintAuthority":{"label":"issuer"},"freezeAuthority":{"label":"issuer"}},{"label":"destination","tokenProgram":T22,"decimals":6,"mintAuthority":{"label":"issuer"},"freezeAuthority":{"label":"issuer"}}],"tokenAccounts":accounts}));
        let spec=spec(&recipe,LEGACY,T22,json!({}));let world=world(&recipe,&spec);(recipe,world)
    })
}
pub struct Prepared {
    pub root: tempfile::TempDir,
    pub input: ValidatedInput,
    pub population: Vec<u8>,
    pub budget: StressBudget,
    pub discovery: World,
    pub rpc: WorldRpc,
}
pub fn prepare(reserve: &str) -> Prepared {
    let (recipe, world) = fixture();
    let spec = spec(
        recipe,
        LEGACY,
        T22,
        json!({"destinationFunding":{"kind":"reserveTransfer","reserve":{"kind":"proposed","fundedRaw":reserve}}}),
    );
    let root = tempfile::tempdir().unwrap();
    let input = input::assemble(
        &root.path().join("input"),
        &spec,
        migration::adapter::REFERENCE_PROGRAM_ID,
        &reference(),
        &Config {
            state: StateSource::MainnetCapture,
            rehearsal_clock: RehearsalClockPolicy::Captured,
            max_rehearsal_units: 100,
            max_captured_holders: 100,
        },
        None,
        MigrationInvariant::recommended(),
    )
    .unwrap();
    let rpc = WorldRpc {
        world: world.clone(),
        genesis: MAINNET_GENESIS.into(),
        slot: world.clock.slot,
        calls: Mutex::new(vec![]),
    };
    let budget = StressBudget {
        max_selected_cases: 3,
        ..StressBudget::default()
    };
    let pop = migration::population::capture(
        spec.source.mint.clone(),
        "current-run".into(),
        "stress-1".into(),
        budget.clone(),
        &rpc,
    )
    .unwrap();
    let population = serde_json::to_vec(&pop).unwrap();
    let observation = migration::population::evaluate_bytes(&population, &budget).unwrap();
    let overlay =
        migration::adapter::derive(&spec, input.change_spec_id(), input.program_id()).unwrap();
    let cap = migration::capture::capture(&spec, &overlay, &observation, 100, "current-run", &rpc)
        .unwrap();
    let discovery = migration::capture::world(
        &spec,
        &population,
        &serde_json::to_vec(&cap).unwrap(),
        &budget,
    )
    .unwrap();
    Prepared {
        root,
        input,
        population,
        budget,
        discovery,
        rpc,
    }
}
impl Prepared {
    pub fn plan(&self) -> migration::current::FrozenPlan {
        migration::current::freeze(
            &self.input,
            &self.population,
            &self.budget,
            &self.discovery,
            "2026-01-01T00:00:00.000Z",
        )
        .unwrap()
    }
    pub fn run(&self, rpc: &impl SolanaRpc) -> migration::current::CurrentReport {
        migration::current::run_with(
            &self.input,
            &self.population,
            &self.budget,
            &self.discovery,
            &self.root.path().join("current"),
            rpc,
        )
        .unwrap()
    }
    pub fn replay(&self) -> migration::current::CurrentReport {
        migration::current::replay(&self.input, &self.root.path().join("current"), &self.budget)
            .unwrap()
    }
}
pub fn data(raw: &mut Value, f: impl FnOnce(&mut Vec<u8>)) {
    let mut bytes = STANDARD.decode(raw["data"][0].as_str().unwrap()).unwrap();
    f(&mut bytes);
    raw["data"][0] = STANDARD.encode(&bytes).into();
    raw["space"] = json!(bytes.len());
}
pub fn token(raw: &mut Value, f: impl FnOnce(&mut spl_token_2022_interface::state::Account)) {
    use solana_program_pack::Pack;
    data(raw, |bytes| {
        let mut account =
            spl_token_2022_interface::state::Account::unpack_from_slice(bytes).unwrap();
        f(&mut account);
        account.pack_into_slice(bytes);
    });
}
pub struct Alter<'a, F> {
    pub rpc: &'a WorldRpc,
    pub change: F,
}
impl<F: Fn(&Value, &mut Value) + Sync> SolanaRpc for Alter<'_, F> {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut response = self.rpc.call(method, params.clone())?;
        if method == "getMultipleAccounts" {
            (self.change)(&params, &mut response);
        }
        Ok(response)
    }
}
pub fn source_at<'a>(
    params: &Value,
    response: &'a mut Value,
    address: &str,
) -> Option<&'a mut Value> {
    let i = params[0].as_array()?.iter().position(|v| v == address)?;
    response["value"].as_array_mut()?.get_mut(i)
}
