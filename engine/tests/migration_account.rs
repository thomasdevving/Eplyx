#[path = "common/migration.rs"]
mod common;
use common::*;
use eplyx_engine::{
    ingest::rpc::RpcProvider,
    lifecycle::current,
    migration::{account, world::World},
    path::current::AmountMode,
};
use serde_json::{json, Value};
struct Rpc {
    world: World,
    source: String,
    owner: String,
    mint: String,
    replacement: String,
}
impl RpcProvider for Rpc {
    fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let value = match method {
            "getGenesisHash" => return Ok(json!(eplyx_engine::migration::world::MAINNET_GENESIS)),
            "getAccountInfo" => self.world.rpc_value(params[0].as_str().unwrap()),
            "getMultipleAccounts" => json!(params[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| self.world.rpc_value(a.as_str().unwrap()))
                .collect::<Vec<_>>()),
            "getTokenAccountsByOwner" => {
                json!([{"pubkey":self.source,"account":self.world.rpc_value(&self.source)}])
            }
            _ => anyhow::bail!("unexpected observation method"),
        };
        Ok(json!({"context":{"slot":self.world.clock.slot},"value":value}))
    }
}
fn setup() -> (Rpc, String, account::Request) {
    let recipe = recipe(
        json!({"schemaVersion":1,"programs":"pinnedMainnetCapture","populationMint":"source","description":"Mock current candidate acquisition using synthetic state and frozen deployed programs","id":"current-candidate","clock":{"slot":"500000000","epoch":"0","unixTimestamp":"100"},"wallets":[{"label":"holder","lamports":"1000000000"}],"mints":[{"label":"source","tokenProgram":LEGACY,"decimals":0,"mintAuthority":{"label":"holder"}},{"label":"destination","tokenProgram":LEGACY,"decimals":0,"mintAuthority":{"label":"holder"}}],"tokenAccounts":[{"label":"holding","mint":"source","owner":{"label":"holder"},"layout":"associated","amount":"100"}]}),
    );
    let spec = spec(&recipe, LEGACY, LEGACY, json!({}));
    let rpc = Rpc {
        world: world(&recipe, &spec),
        source: address(&recipe, "holding"),
        owner: address(&recipe, "holder"),
        mint: address(&recipe, "source"),
        replacement: address(&recipe, "destination"),
    };
    let wallet = serde_json::to_string(
        &current::capture_selected(
            current::InspectionSelection {
                cluster: "solana-mainnet".into(),
                mint: rpc.mint.clone(),
                reference: None,
                sample_accounts: false,
                public_owner: Some(rpc.owner.clone()),
            },
            &rpc,
        )
        .unwrap(),
    )
    .unwrap();
    let request = account::Request {
        source: rpc.source.clone(),
        replacement_mint: rpc.replacement.clone(),
        amount_mode: AmountMode::Custom,
        amount_decimal: Some("10".into()),
        numerator: "1".into(),
        denominator: "2".into(),
        rounding: eplyx_engine::migration::spec::Rounding::Floor,
        fee_bps: 100,
        reserve_raw: "100".into(),
    };
    (rpc, wallet, request)
}
fn replay(c: &account::Capture) -> Value {
    let bytes = serde_json::to_vec(c).unwrap();
    account::replay(
        &bytes,
        "observation-a",
        "candidate-a",
        &c.wallet_sha256,
        &reference(),
    )
    .unwrap()
    .value()
    .clone()
}
#[test]
fn exact_current_candidate_replays_with_unchanged_source_and_no_official_claim() {
    let (rpc, wallet, request) = setup();
    let c = account::capture(
        wallet,
        request,
        "observation-a".into(),
        "candidate-a".into(),
        &reference(),
        &rpc,
    )
    .unwrap();
    let v = replay(&c);
    assert_eq!(v["status"], "Proven", "{v}");
    assert_eq!(v["execution"]["reconciled"], true);
    assert_eq!(v["amount_raw"], "10");
    assert_eq!(v["plan_unit"]["source_balance_raw"], "100");
    assert_eq!(v["official_transition"], "NotTested");
    assert_eq!(v["signer_possession_known"], false);
    assert_eq!(v, replay(&c));
}
#[test]
fn final_drift_and_unavailable_clock_never_adjust_or_execute() {
    let (rpc, wallet, request) = setup();
    let mut c = account::capture(
        wallet,
        request,
        "observation-a".into(),
        "candidate-a".into(),
        &reference(),
        &rpc,
    )
    .unwrap();
    let last = c.observations.last_mut().unwrap();
    let index = last.params[0]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a == &rpc.source)
        .unwrap();
    last.result.as_mut().unwrap()["value"][index]["lamports"] = json!(1);
    let v = replay(&c);
    assert_eq!(v["status"], "Indeterminate");
    assert_eq!(v["execution_performed"], false);
    assert_eq!(v["amount_raw"], "10");
    assert_eq!(v["reason"], "SourceStateChanged");
    c.observations.pop();
    assert_eq!(replay(&c)["execution_performed"], false);
}
#[test]
fn typed_terms_refuse_forged_status_program_and_other_wallet_account() {
    let (_, wallet, mut request) = setup();
    request.source = request.replacement_mint.clone();
    assert!(account::validate(&wallet, &request).is_err());
    let mut value = serde_json::to_value(&request).unwrap();
    for key in ["program", "status", "instructions", "rpc_url", "path"] {
        value[key] = json!("forged");
        assert!(serde_json::from_value::<account::Request>(value.clone()).is_err());
        value.as_object_mut().unwrap().remove(key);
    }
}
