//! Bounded current observations. No execution facts or lifecycle policy are accepted here.
use super::{decode, rpc::SolanaRpc, AssetDescriptor};
use anyhow::{ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
use std::{collections::BTreeSet, io::Write, path::Path};

const MAINNET: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const DECODER: &str = "spl-token-2022-interface/3.1.1; current-observation/v1";
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub method: String,
    pub params: Value,
    pub started_at: String,
    pub completed_at: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub schema_version: u32,
    pub asset: AssetDescriptor,
    pub rpc_origin: String,
    pub started_at: String,
    pub completed_at: String,
    pub decoder: String,
    pub observations: Vec<Observation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<InspectionSelection>,
}
fn record(
    rpc: &impl SolanaRpc,
    records: &mut Vec<Observation>,
    method: &str,
    params: Value,
) -> Option<Value> {
    let started_at = now();
    let (result, error) = match rpc.call(method, params.clone()) {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e.to_string())),
    };
    records.push(Observation {
        method: method.into(),
        params,
        started_at,
        completed_at: now(),
        result: result.clone(),
        error,
    });
    result
}
fn config() -> Value {
    json!({"encoding":"base64","commitment":"finalized"})
}

pub fn capture(asset: AssetDescriptor, rpc: &impl SolanaRpc) -> Result<Capture> {
    capture_with_selection(asset, rpc, None)
}
fn capture_with_selection(
    asset: AssetDescriptor,
    rpc: &impl SolanaRpc,
    selection: Option<InspectionSelection>,
) -> Result<Capture> {
    let _: Address = asset.mint.parse().context("invalid asset mint")?;
    let mut c = Capture {
        schema_version: if selection.as_ref().is_some_and(|s| s.public_owner.is_some()) {
            3
        } else if selection.is_some() {
            2
        } else {
            1
        },
        asset,
        rpc_origin: "configured_read_only_provider".into(),
        started_at: now(),
        completed_at: String::new(),
        decoder: if selection.as_ref().is_some_and(|s| s.public_owner.is_some()) {
            WALLET_DECODER
        } else {
            DECODER
        }
        .into(),
        observations: vec![],
        selection,
    };
    eprintln!("CURRENT_STAGE:Validating mainnet identity");
    let genesis = record(rpc, &mut c.observations, "getGenesisHash", json!([]));
    if genesis.as_ref().and_then(Value::as_str) == Some(MAINNET) {
        eprintln!("CURRENT_STAGE:Fetching current mint state");
        let mint = record(
            rpc,
            &mut c.observations,
            "getAccountInfo",
            json!([c.asset.mint, config()]),
        );
        if let Some(owner) = c.selection.as_ref().and_then(|s| s.public_owner.clone()) {
            if let Some(response) = mint
                .as_ref()
                .filter(|m| decode::decode_mint(&m["value"]).is_ok())
            {
                let mut cfg = config();
                cfg["minContextSlot"] = json!(context_slot(response)?);
                eprintln!("CURRENT_STAGE:Checking public owner address");
                if let Some(owner_response) = record(
                    rpc,
                    &mut c.observations,
                    "getAccountInfo",
                    json!([owner, cfg]),
                ) {
                    if owner_boundary(&owner_response["value"])["accepted"] == true {
                        let mut cfg = config();
                        cfg["minContextSlot"] =
                            json!(context_slot(&owner_response)?.max(context_slot(response)?));
                        eprintln!("CURRENT_STAGE:Looking up token accounts");
                        record(
                            rpc,
                            &mut c.observations,
                            "getTokenAccountsByOwner",
                            json!([owner, {"mint":c.asset.mint}, cfg]),
                        );
                    }
                }
            }
        } else if c.selection.as_ref().is_none_or(|s| s.sample_accounts)
            && mint
                .as_ref()
                .is_some_and(|m| decode::decode_mint(&m["value"]).is_ok())
        {
            eprintln!("CURRENT_STAGE:Fetching a bounded account sample");
            if let Some(sample) = record(
                rpc,
                &mut c.observations,
                "getTokenLargestAccounts",
                json!([c.asset.mint,{"commitment":"finalized"}]),
            ) {
                if let Ok(addresses) = sample_addresses(&sample) {
                    let mut cfg = config();
                    cfg["minContextSlot"] = json!(context_slot(&sample)?);
                    record(
                        rpc,
                        &mut c.observations,
                        "getMultipleAccounts",
                        json!([addresses, cfg]),
                    );
                }
            }
        }
    }
    c.completed_at = now();
    Ok(c)
}
fn context_slot(v: &Value) -> Result<u64> {
    v["context"]["slot"]
        .as_u64()
        .context("missing response context")
}
fn sample_addresses(v: &Value) -> Result<Vec<String>> {
    context_slot(v)?;
    let entries = v["value"].as_array().context("invalid account sample")?;
    ensure!(entries.len() <= 20, "account sample exceeds budget");
    let mut seen = BTreeSet::new();
    entries
        .iter()
        .map(|e| {
            let a = e["address"].as_str().context("sample address missing")?;
            let _: Address = a.parse()?;
            ensure!(seen.insert(a), "duplicate sample account");
            Ok(a.to_owned())
        })
        .collect()
}
fn checked<'a>(c: &'a Capture, i: usize, method: &str, params: Value) -> Result<Option<&'a Value>> {
    let r = c
        .observations
        .get(i)
        .context("missing acquisition record")?;
    ensure!(
        r.method == method && r.params == params,
        "acquisition request binding mismatch"
    );
    let start = chrono::DateTime::parse_from_rfc3339(&r.started_at)?;
    let end = chrono::DateTime::parse_from_rfc3339(&r.completed_at)?;
    ensure!(
        start <= end
            && start >= chrono::DateTime::parse_from_rfc3339(&c.started_at)?
            && end <= chrono::DateTime::parse_from_rfc3339(&c.completed_at)?,
        "invalid retrieval interval"
    );
    ensure!(
        r.result.is_some() != r.error.is_some(),
        "ambiguous acquisition outcome"
    );
    Ok(r.result.as_ref())
}
/// Regenerate every displayed fact from the retained original responses, without RPC.
/// No deserialized analytical statuses, proofs or policy inputs exist in this format.
pub fn evaluate(c: &Capture) -> Result<Value> {
    if c.schema_version == 3 {
        return evaluate_wallet(c);
    }
    if c.schema_version == 2 {
        return evaluate_selected(c);
    }
    ensure!(
        c.selection.is_none(),
        "unexpected selection in legacy capture"
    );
    evaluate_legacy(c)
}
fn evaluate_legacy(c: &Capture) -> Result<Value> {
    ensure!(
        c.schema_version == 1 && c.decoder == DECODER,
        "unsupported observation version"
    );
    ensure!(
        (2..=4).contains(&c.observations.len()),
        "incomplete or excessive acquisition records"
    );
    let _: Address = c.asset.mint.parse()?;
    let genesis = checked(c, 0, "getGenesisHash", json!([]))?
        .context("mainnet identity acquisition failed")?;
    ensure!(genesis == MAINNET, "wrong cluster");
    if let Some(expected) = &c.asset.expected_genesis_hash {
        ensure!(expected == MAINNET, "asset cluster mismatch");
    }
    let mint_response = checked(c, 1, "getAccountInfo", json!([c.asset.mint, config()]))?
        .context("mint acquisition failed")?;
    let mint = decode::decode_mint(&mint_response["value"])?;
    if let Some(expected) = &c.asset.expected_token_program {
        ensure!(
            expected == &mint.token_program,
            "asset token program mismatch"
        );
    }
    let mint_slot = context_slot(mint_response)?;
    let mut accounts = vec![];
    let mut slots = vec![mint_slot];
    let mut gaps = vec![];
    let mut sample_status = "Unavailable";
    if c.observations.len() >= 3 {
        if let Some(sample) = checked(
            c,
            2,
            "getTokenLargestAccounts",
            json!([c.asset.mint,{"commitment":"finalized"}]),
        )? {
            let addresses = sample_addresses(sample)?;
            let slot = context_slot(sample)?;
            slots.push(slot);
            let mut cfg = config();
            cfg["minContextSlot"] = json!(slot);
            if let Some(batch) = checked(c, 3, "getMultipleAccounts", json!([addresses, cfg]))? {
                let batch_slot = context_slot(batch)?;
                ensure!(batch_slot >= slot, "sample capture predates discovery");
                slots.push(batch_slot);
                let values = batch["value"].as_array().context("invalid account batch")?;
                ensure!(values.len() == addresses.len(), "incomplete account batch");
                for (address, raw) in addresses.iter().zip(values) {
                    match decode::decode_token_account(raw,&mint.token_program,&c.asset.mint,mint.decimals) {
                        Ok(state)=>accounts.push(json!({"address":address,"state":state,"slot":batch_slot.to_string(),"authority_classification":"Unknown"})),
                        Err(_)=>gaps.push(format!("Sample account {address} is absent, changed, or unsupported by this decoder; original bytes are retained.")),
                    }
                }
                sample_status = "Sampled";
            } else {
                gaps.push("The account sample was discovered, but its account data could not be retrieved.".into());
            }
        } else {
            ensure!(
                c.observations.len() == 3,
                "unexpected records after failed sample"
            );
            gaps.push("The provider could not return the largest-account sample within the request budget. Holdings coverage is unavailable; mint findings remain valid.".into());
        }
    } else {
        gaps.push("Account sampling was not completed.".into());
    }
    let paths = [
        "OfficialTransition",
        "Redemption",
        "SecondaryMarketExit",
        "Transfer",
        "Withdrawal",
    ];
    Ok(json!({
        "schema_version":1,"kind":"current-inspection","asset":c.asset,
        "identity_provenance":"Saved issuer reference; independently decoded current mint. Issuer reference was not refreshed.",
        "acquisition":{"started_at":c.started_at,"completed_at":c.completed_at,"rpc_origin":c.rpc_origin,"genesis_hash":MAINNET,"commitment":"finalized","context_slots":slots.iter().map(u64::to_string).collect::<Vec<_>>(),"consistency":"Separate RPC observations, not an atomic validator bank","decoder":c.decoder},
        "mint":mint,"mint_slot":mint_slot.to_string(),"accounts":accounts,
        "discovery":{"status":sample_status,"complete_population":false,"maximum_sample_accounts":20,"gaps":gaps},
        "paths":paths.map(|p|json!({"path":p,"status":"NotTested","reason":"Fresh execution is not wired for this observation milestone."})),
        "lifecycle_event":null,"readiness":null,"authorization":false,"funds_moved":false,
        "local_execution_performed":false,"signer_assumed_locally":false,"signer_possession_known":false,
        "limitations":["Largest-account sampling is not a holder population or wallet lookup.","No protocol position, market route, token transfer, withdrawal or official conversion was tested.","No lifecycle event or assurance policy was selected; this is not a readiness verdict.","Mint supply, public account balances and withheld fees are separate quantities; no reconciliation across RPC contexts is claimed.","Amounts are unscaled token quantities, not company shares or valuations. Confidential balances are not known zero. Extension presence alone is not active behavior."]
    }))
}
pub fn save(c: &Capture, path: &Path) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(serde_json::to_string_pretty(c)?.as_bytes())?;
    file.sync_all()?;
    Ok(())
}
pub fn replay(path: &Path) -> Result<Value> {
    let bytes = crate::lifecycle::artifact::read(path)?;
    ensure!(bytes.len() <= 10 * 1024 * 1024, "capture exceeds budget");
    evaluate(&serde_json::from_slice(&bytes)?)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceAssertion {
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CatalogueReference {
    pub version: String,
    pub source_url: String,
    pub retrieved_at: String,
    pub content_sha256: String,
    pub assertions: Vec<SourceAssertion>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InspectionSelection {
    pub cluster: String,
    pub mint: String,
    pub reference: Option<CatalogueReference>,
    pub sample_accounts: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_owner: Option<String>,
}
impl InspectionSelection {
    pub fn validate(&self) -> Result<()> {
        let _: Address = self
            .mint
            .parse()
            .context("Enter a valid Solana token address")?;
        ensure!(self.cluster == "solana-mainnet", "unsupported cluster");
        if let Some(owner) = &self.public_owner {
            let _: Address = owner
                .parse()
                .context("Enter a valid Solana public owner address")?;
            ensure!(
                !self.sample_accounts,
                "wallet lookup cannot request a global sample"
            );
        }
        if let Some(r) = &self.reference {
            let hex = |s: &str| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            };
            ensure!(
                hex(&r.version) && hex(&r.content_sha256),
                "invalid catalogue digest"
            );
            ensure!(
                r.source_url == "https://prestocks.com/products",
                "unsupported identity source"
            );
            chrono::DateTime::parse_from_rfc3339(&r.retrieved_at)?;
            ensure!(
                !r.assertions.is_empty() && r.assertions.len() <= 100,
                "invalid source assertions"
            );
            for a in &r.assertions {
                ensure!(
                    a.mint == self.mint
                        && !a.name.trim().is_empty()
                        && a.name.len() <= 640
                        && !a.symbol.trim().is_empty()
                        && a.symbol.len() <= 640,
                    "catalogue identity mismatch"
                );
            }
        }
        Ok(())
    }
}
pub fn capture_selected(selection: InspectionSelection, rpc: &impl SolanaRpc) -> Result<Capture> {
    selection.validate()?;
    let asset = AssetDescriptor {
        name: selection
            .reference
            .as_ref()
            .map(|r| r.assertions[0].name.clone())
            .unwrap_or_else(|| "Custom token address".into()),
        mint: selection.mint.clone(),
        expected_token_program: None,
        expected_genesis_hash: Some(MAINNET.into()),
        verification: vec![],
    };
    capture_with_selection(asset, rpc, Some(selection))
}
fn evaluate_selected(c: &Capture) -> Result<Value> {
    ensure!(
        c.selection
            .as_ref()
            .is_none_or(|s| s.public_owner.is_none()),
        "wallet requires capture version 3"
    );
    let selection = c.selection.as_ref().context("missing selected identity")?;
    selection.validate()?;
    ensure!(
        c.decoder == DECODER
            && c.asset.mint == selection.mint
            && c.asset.expected_genesis_hash.as_deref() == Some(MAINNET)
            && c.asset.expected_token_program.is_none()
            && c.asset.verification.is_empty(),
        "selected asset binding mismatch"
    );
    let expected_name = selection
        .reference
        .as_ref()
        .map(|r| r.assertions[0].name.as_str())
        .unwrap_or("Custom token address");
    ensure!(
        c.asset.name == expected_name,
        "selected display identity mismatch"
    );
    ensure!(
        (2..=4).contains(&c.observations.len()),
        "incomplete acquisition"
    );
    ensure!(
        checked(c, 0, "getGenesisHash", json!([]))? == Some(&json!(MAINNET)),
        "mainnet identity acquisition failed"
    );
    let response = checked(c, 1, "getAccountInfo", json!([selection.mint, config()]))?
        .context("account acquisition failed")?;
    let slot = context_slot(response)?;
    ensure!(
        selection.sample_accounts || c.observations.len() == 2,
        "unrequested sample observations"
    );
    // Validate request/interval bindings before optional decoding can produce a partial result.
    if c.observations.len() >= 3 {
        let sample = checked(
            c,
            2,
            "getTokenLargestAccounts",
            json!([selection.mint,{"commitment":"finalized"}]),
        )?;
        if c.observations.len() == 4 {
            let sample = sample.context("records after failed sample")?;
            let addresses = sample_addresses(sample)?;
            let mut cfg = config();
            cfg["minContextSlot"] = json!(context_slot(sample)?);
            checked(c, 3, "getMultipleAccounts", json!([addresses, cfg]))?;
        }
    }
    let raw = &response["value"];
    let decoded = decode::decode_mint(raw);
    let mut legacy = c.clone();
    legacy.schema_version = 1;
    legacy.selection = None;
    let mut result = if decoded.is_ok() {
        match evaluate_legacy(&legacy) {
            Ok(r) => r,
            Err(error) => {
                legacy.observations.truncate(2);
                let mut r = evaluate_legacy(&legacy)?;
                r["discovery"]["gaps"] = json!([
                    "Token details were retrieved. The account sample could not be loaded."
                ]);
                r["discovery"]["technical_boundary"] = json!(error.to_string());
                r
            }
        }
    } else {
        ensure!(c.observations.len() == 2, "sample without supported mint");
        json!({"schema_version":2,"kind":"current-inspection","asset":c.asset,
            "acquisition":{"started_at":c.started_at,"completed_at":c.completed_at,"rpc_origin":c.rpc_origin,"genesis_hash":MAINNET,"commitment":"finalized","context_slots":[slot.to_string()],"consistency":"Separate RPC observations, not an atomic validator bank","decoder":c.decoder},
            "mint":null,"mint_slot":slot.to_string(),"accounts":[],"discovery":{"status":"NotRequested","complete_population":false,"maximum_sample_accounts":20,"gaps":[]},
            "paths":(["OfficialTransition","Redemption","SecondaryMarketExit","Transfer","Withdrawal"].map(|p|json!({"path":p,"status":"NotTested","reason":"No execution requested or performed."}))),
            "lifecycle_event":null,"readiness":null,"authorization":false,"funds_moved":false,"local_execution_performed":false,"signer_assumed_locally":false,"signer_possession_known":false,
            "limitations":["This inspection does not establish stock issuer association, lifecycle readiness or execution assurance."]})
    };
    result["schema_version"] = json!(2);
    result["selection"] = json!(selection);
    result["execution_performed"] = json!(false);
    result["identity_provenance"] = json!(if selection.reference.is_some() {
        "Referenced by the PreStocks product source; on-chain account inspected independently. This source reference is not execution proof or legal verification."
    } else {
        "Stock/issuer association unconfirmed. A token name or symbol is not issuer evidence."
    });
    result["inspection"] = match decoded {
        Ok(_) => {
            json!({"status":"Completed","account_type":"Mint","message":"Current token information retrieved"})
        }
        Err(error) => account_boundary(raw, &error.to_string()),
    };
    result["account_observation"] = if raw.is_null() {
        Value::Null
    } else {
        json!({"owner":raw["owner"],"executable":raw["executable"],"space":raw["space"].as_u64().map(|n| n.to_string()),"slot":slot.to_string()})
    };
    if !selection.sample_accounts {
        result["discovery"]["status"] = json!("NotRequested");
        result["discovery"]["gaps"] = json!([]);
    }
    result["discovery"]["requested"] = json!(selection.sample_accounts);
    result["discovery"]["method"] = json!("getTokenLargestAccounts + getMultipleAccounts");
    result["discovery"]["sample_count"] = if result["discovery"]["status"] == "Sampled" {
        json!(result["accounts"].as_array().map(Vec::len))
    } else {
        Value::Null
    };
    result["discovery"]["technical_errors"] = json!(c
        .observations
        .iter()
        .filter_map(|o| o
            .error
            .as_ref()
            .map(|e| json!({"method":o.method,"error":e})))
        .collect::<Vec<_>>());
    let metadata = result["mint"]["extensions"]
        .as_array()
        .and_then(|extensions| {
            extensions
                .iter()
                .find(|e| e["extension_type"] == "TokenMetadata")
        })
        .map(|e| e["config"].clone())
        .unwrap_or(Value::Null);
    let mut mismatches = vec![];
    if !metadata.is_null() && metadata["mint"] != selection.mint {
        mismatches.push("On-chain metadata refers to a different mint".to_string());
    }
    if let Some(reference) = &selection.reference {
        for a in &reference.assertions {
            if !metadata.is_null() && (metadata["name"] != a.name || metadata["symbol"] != a.symbol)
            {
                mismatches.push(format!(
                    "Source name/symbol ({}/{}) differs from on-chain metadata",
                    a.name, a.symbol
                ));
            }
            if !result["mint"].is_null() && result["mint"]["decimals"] != a.decimals {
                mismatches.push("Source decimals differ from the inspected mint".into());
            }
        }
        if reference.assertions.len() > 1 {
            mismatches.push("Multiple distinct source assertions retained for this mint".into());
        }
    }
    result["onchain_metadata"] = metadata;
    result["identity_mismatches"] = json!(mismatches);
    Ok(result)
}
fn account_boundary(raw: &Value, error: &str) -> Value {
    use solana_program_pack::Pack;
    use spl_token_2022_interface::{
        extension::{StateWithExtensions, StateWithExtensionsMut},
        state::{Account, Mint},
    };
    if raw.is_null() {
        return json!({"status":"Unavailable","account_type":"Missing","message":"No account was returned for this address."});
    }
    let program = raw["owner"].as_str().unwrap_or("");
    if raw["executable"] == true {
        return json!({"status":"Unsupported","account_type":"Program","message":"This address is an executable program, not a token mint."});
    }
    if program != decode::LEGACY_PROGRAM && program != decode::TOKEN_2022_PROGRAM {
        return json!({"status":"Unsupported","account_type":"UnsupportedOwner","message":"This account uses a program the current inspector does not support."});
    }
    if let Ok(mut bytes) = decode::account_bytes(raw, program) {
        let holding = if program == decode::LEGACY_PROGRAM {
            Account::unpack(&bytes).is_ok()
        } else {
            StateWithExtensions::<Account>::unpack(&bytes).is_ok()
        };
        if holding {
            return json!({"status":"Unsupported","account_type":"TokenAccount","message":"This address is a token-holding account, not a token mint."});
        }
        let base = if program == decode::LEGACY_PROGRAM {
            Mint::unpack_unchecked(&bytes).ok()
        } else {
            StateWithExtensions::<Mint>::unpack(&bytes)
                .ok()
                .map(|s| s.base)
                .or_else(|| {
                    StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut bytes)
                        .ok()
                        .map(|s| s.base)
                })
        };
        if let Some(base) = base {
            return json!({"status":"Partial","account_type":"Mint","message":"Mint base fields retrieved; full token configuration is outside the current decoder boundary.","technical_boundary":error,"base_fields":{"is_initialized":base.is_initialized,"decimals":base.decimals,"raw_supply":base.supply.to_string(),"decimal_supply":decode::decimal_amount(base.supply,base.decimals),"mint_authority":Option::<Address>::from(base.mint_authority).map(|v|v.to_string()),"freeze_authority":Option::<Address>::from(base.freeze_authority).map(|v|v.to_string())},"extensions":null});
        }
    }
    json!({"status":"Unsupported","account_type":"UnknownTokenLayout","message":"This token-program account could not be decoded as a supported mint.","technical_boundary":error})
}

const WALLET_DECODER: &str = "spl-token-2022-interface/3.1.1; current-wallet/v1";
const MAX_WALLET_ACCOUNTS: usize = 1000;

/// A public owner authority is not an authenticated wallet or a known signer.
fn owner_boundary(raw: &Value) -> Value {
    if raw.is_null() {
        return json!({"accepted":true,"kind":"Absent","message":"No funded account observed at this owner address. Signing access is unknown."});
    }
    if raw["executable"] == true {
        return json!({"accepted":false,"kind":"Program","message":"This address is an executable program rather than a wallet owner. Enter a public owner address."});
    }
    let program = raw["owner"].as_str().unwrap_or("");
    if program == decode::LEGACY_PROGRAM || program == decode::TOKEN_2022_PROGRAM {
        use solana_program_pack::Pack;
        use spl_token_2022_interface::{extension::StateWithExtensions, state::Account};
        if let Ok(bytes) = decode::account_bytes(raw, program) {
            let base = if bytes.len() == Account::LEN {
                Account::unpack_unchecked(&bytes).ok()
            } else {
                StateWithExtensions::<Account>::unpack(&bytes)
                    .ok()
                    .map(|s| s.base)
            };
            if let Some(base) = base {
                return json!({"accepted":false,"kind":"TokenAccount","suggested_owner":base.owner.to_string(),"message":"This address appears to be a token account rather than a wallet owner. Enter its recorded owner address instead."});
            }
        }
        let boundary = account_boundary(raw, "unsupported owner layout");
        return json!({"accepted":false,"kind":boundary["account_type"],"message":"This address is a mint or another token-program account rather than a supported public owner address."});
    }
    json!({"accepted":true,"kind":"PossibleOwnerAuthority","runtime_owner":raw["owner"],"message":"Possible owner authority; human identity, authority control and signer possession are not established."})
}

#[derive(Debug, Serialize)]
struct WalletAccount {
    address: String,
    state: decode::TokenAccountState,
    slot: u64,
    evidence: String,
}
#[derive(Debug, Serialize)]
struct WalletObservation {
    submitted_owner: String,
    selected_mint: String,
    status: String,
    owner_inspection: Value,
    acquisition: Value,
    token_accounts: Vec<WalletAccount>,
    account_count: Option<usize>,
    decoded_account_count: usize,
    public_balance_total_raw: Option<String>,
    known_public_balance_subtotal_raw: Option<String>,
    confidential_balance: String,
    coverage: Value,
    gaps: Vec<Value>,
    limitations: Vec<String>,
}
fn evaluate_wallet(c: &Capture) -> Result<Value> {
    ensure!(c.decoder == WALLET_DECODER, "unsupported wallet decoder");
    let selection = c.selection.as_ref().context("missing wallet selection")?;
    selection.validate()?;
    let owner = selection
        .public_owner
        .as_ref()
        .context("missing public owner")?;
    ensure!(
        (2..=4).contains(&c.observations.len()),
        "incomplete or excessive wallet records"
    );
    // Reuse identity/mint derivation without accepting any wallet facts from JSON.
    let mut base = c.clone();
    base.schema_version = 2;
    base.decoder = DECODER.into();
    base.selection.as_mut().unwrap().public_owner = None;
    base.observations.truncate(2);
    let mut result = evaluate_selected(&base)?;
    let mint = decode::decode_mint(
        &c.observations[1]
            .result
            .as_ref()
            .context("mint acquisition failed")?["value"],
    )
    .context("wallet inspection requires a supported current mint")?;
    let mint_slot = result["mint_slot"]
        .as_str()
        .context("missing mint slot")?
        .parse::<u64>()?;
    let mut cfg = config();
    cfg["minContextSlot"] = json!(mint_slot);
    let owner_response = checked(c, 2, "getAccountInfo", json!([owner, cfg]))?;
    let mut wallet = WalletObservation {
        submitted_owner: owner.clone(), selected_mint: selection.mint.clone(), status: "Unavailable".into(),
        owner_inspection: json!({"accepted":false,"kind":"Unavailable","message":"The public owner address could not be inspected. Holdings are unknown."}),
        acquisition: json!({"method":"getTokenAccountsByOwner","query":{"owner":owner,"mint":selection.mint},"status":"NotRequested","context_slot":null,"evidence":"observations[3]"}),
        token_accounts: vec![], account_count: None, decoded_account_count: 0,
        public_balance_total_raw: None, known_public_balance_subtotal_raw: None,
        confidential_balance: "Unknown; excluded from public quantities".into(),
        coverage: json!({"scope":"Owner-scoped token accounts for selected mint","complete_response_decoded":false,"maximum_accounts":MAX_WALLET_ACCOUNTS,"response_byte_limit":2097152,"all_assets":false,"protocol_positions_checked":false}),
        gaps: vec![],
        limitations: vec!["Protocol positions were not checked in this analysis.".into(),"Transfers, market exits, withdrawals, lifecycle transition and redemption were not tested.".into(),"Public owner association does not prove human identity, legal ownership or signing access.".into(),"Public base balances exclude encrypted balances and withheld fees; display scaling and interest conversion are not applied.".into(),"Separate finalized RPC contexts are not an atomic bank or a guarantee of later availability.".into()],
    };
    if let Some(response) = owner_response {
        let owner_slot = context_slot(response)?;
        ensure!(owner_slot >= mint_slot, "owner observation predates mint");
        result["acquisition"]["context_slots"]
            .as_array_mut()
            .unwrap()
            .push(json!(owner_slot.to_string()));
        wallet.owner_inspection = owner_boundary(&response["value"]);
        if wallet.owner_inspection["accepted"] == true {
            cfg["minContextSlot"] = json!(owner_slot);
            let lookup = checked(
                c,
                3,
                "getTokenAccountsByOwner",
                json!([owner,{"mint":selection.mint},cfg]),
            )?;
            if let Some(lookup) = lookup {
                let decoded = (|| -> Result<()> {
                    let slot = context_slot(lookup)?;
                    ensure!(
                        slot >= owner_slot,
                        "wallet lookup predates owner inspection"
                    );
                    wallet.acquisition["context_slot"] = json!(slot.to_string());
                    result["acquisition"]["context_slots"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(slot.to_string()));
                    let rows = lookup["value"]
                        .as_array()
                        .context("invalid owner lookup response")?;
                    ensure!(
                        rows.len() <= MAX_WALLET_ACCOUNTS,
                        "owner response exceeds account budget; no truncated result accepted"
                    );
                    let mut seen = BTreeSet::new();
                    // Validate keys before accumulating; duplicate rows must not inflate totals.
                    for row in rows {
                        let address = row["pubkey"]
                            .as_str()
                            .context("missing token account address")?;
                        let _: Address = address.parse()?;
                        ensure!(seen.insert(address), "duplicate owner token account");
                    }
                    wallet.account_count = Some(rows.len());
                    let mut total = 0u128;
                    for (index, row) in rows.iter().enumerate() {
                        let address = row["pubkey"].as_str().unwrap();
                        let state = decode::decode_token_account(
                            &row["account"],
                            &mint.token_program,
                            &selection.mint,
                            mint.decimals,
                        )
                        .and_then(|s| {
                            ensure!(
                                &s.owner == owner,
                                "returned account has a different recorded owner"
                            );
                            Ok(s)
                        });
                        match state {
                            Ok(state) => {
                                total = total.checked_add(state.raw_balance.parse::<u64>()? as u128).context("public balance total overflow")?;
                                wallet.token_accounts.push(WalletAccount{address:address.into(),state,slot,evidence:format!("observations[3].result.value[{index}]")});
                            },
                            Err(error) => wallet.gaps.push(json!({"address":address,"reason":error.to_string(),"evidence":format!("observations[3].result.value[{index}]")})),
                        }
                    }
                    wallet
                        .token_accounts
                        .sort_by(|a, b| a.address.cmp(&b.address));
                    wallet.decoded_account_count = wallet.token_accounts.len();
                    wallet.known_public_balance_subtotal_raw = Some(total.to_string());
                    wallet.status = if wallet.gaps.is_empty() {
                        "Completed"
                    } else {
                        "Partial"
                    }
                    .into();
                    wallet.acquisition["status"] = json!("Succeeded");
                    if wallet.gaps.is_empty() {
                        wallet.public_balance_total_raw = Some(total.to_string());
                        wallet.coverage["complete_response_decoded"] = json!(true);
                    }
                    Ok(())
                })();
                if let Err(error) = decoded {
                    wallet.gaps.push(json!({"reason":error.to_string()}));
                    wallet.acquisition["status"] = json!("InvalidResponse");
                }
            } else {
                wallet.acquisition["status"] = json!("Unavailable");
                wallet.gaps.push(json!({"reason":c.observations[3].error}));
            }
        } else {
            ensure!(c.observations.len() == 3, "lookup after rejected owner");
            wallet.status = "InvalidOwner".into();
        }
    } else {
        ensure!(c.observations.len() == 3, "lookup without owner inspection");
        wallet.gaps.push(json!({"reason":c.observations[2].error}));
    }
    result["schema_version"] = json!(3);
    result["selection"] = json!(selection);
    result["acquisition"]["decoder"] = json!(c.decoder);
    result["acquisition"]["rpc_methods"] =
        json!(c.observations.iter().map(|o| &o.method).collect::<Vec<_>>());
    result["inspection"]["status"] = json!(match wallet.status.as_str() {
        "Completed" => "Completed",
        "Partial" => "Partial",
        "InvalidOwner" => "Unsupported",
        _ => "Unavailable",
    });
    result["limitations"] = json!(wallet.limitations);
    result["wallet_observation"] = serde_json::to_value(wallet)?;
    Ok(result)
}
