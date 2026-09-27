//! One current wallet account, typed proposed terms and the registered MAIN
//! migration candidate. Acquisition never executes; replay rebuilds every
//! account and instruction from the transcript before granting evidence.
use super::{adapter, coherence, execute, planner, spec, world};
use crate::{
    change::ChangeSpec,
    ingest::{
        observation::{Observation, RpcEvidence},
        rpc::RpcProvider,
    },
    lifecycle::{current, decode},
    path::current::{exact_amount, AmountMode},
    replay::hash_bytes,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub source: String,
    pub replacement_mint: String,
    pub amount_mode: AmountMode,
    pub amount_decimal: Option<String>,
    pub numerator: String,
    pub denominator: String,
    pub rounding: spec::Rounding,
    pub fee_bps: u16,
    pub reserve_raw: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub schema_version: u32,
    pub run_id: String,
    pub check_id: String,
    pub wallet_capture: String,
    pub wallet_sha256: String,
    pub request: Request,
    pub observations: Vec<Observation>,
}
/// No Deserialize or public constructor: only exact replay can create proof.
pub struct Verified {
    value: Value,
}
impl Verified {
    pub fn value(&self) -> &Value {
        &self.value
    }
}
struct Scope {
    mint: decode::MintConfig,
    source: decode::TokenAccountState,
    raw: Value,
    owner: String,
    floor: u64,
    amount: u64,
}
fn scope(wallet: &str, request: &Request) -> Result<Scope> {
    ensure!(
        wallet.len() <= 10 * 1024 * 1024,
        "wallet capture exceeds bound"
    );
    let capture: current::Capture = serde_json::from_str(wallet)?;
    let result = current::evaluate(&capture)?;
    ensure!(
        capture.schema_version == 3,
        "owner-scoped observation required"
    );
    let owner = capture
        .selection
        .as_ref()
        .and_then(|s| s.public_owner.clone())
        .context("owner required")?;
    let mint: decode::MintConfig = serde_json::from_value(result["mint"].clone())?;
    let _: solana_address::Address = request.replacement_mint.parse()?;
    ensure!(
        request.replacement_mint != capture.asset.mint,
        "replacement must differ"
    );
    let selected = result["wallet_observation"]["token_accounts"]
        .as_array()
        .context("wallet accounts")?
        .iter()
        .find(|a| a["address"] == request.source)
        .context("source outside observation")?;
    let source: decode::TokenAccountState = serde_json::from_value(selected["state"].clone())?;
    ensure!(
        source.owner == owner && source.mint == capture.asset.mint,
        "source identity mismatch"
    );
    let raw = capture
        .observations
        .iter()
        .filter_map(|o| o.result.as_ref())
        .filter_map(|r| r["value"].as_array())
        .flatten()
        .find(|r| r["pubkey"] == request.source)
        .context("source bytes unavailable")?["account"]
        .clone();
    let balance = source.raw_balance.parse::<u64>()?;
    let amount = match request.amount_mode {
        AmountMode::Full => {
            ensure!(
                request.amount_decimal.is_none(),
                "full amount has no decimal"
            );
            balance
        }
        AmountMode::Custom => exact_amount(
            request
                .amount_decimal
                .as_deref()
                .context("custom amount required")?,
            mint.decimals,
        )?,
    };
    ensure!(
        amount > 0 && amount <= balance,
        "amount exceeds observed balance"
    );
    for term in [
        &request.numerator,
        &request.denominator,
        &request.reserve_raw,
    ] {
        let value = term.parse::<u64>()?;
        ensure!(value.to_string() == *term, "noncanonical integer term");
    }
    ensure!(
        request.numerator != "0" && request.denominator != "0" && request.fee_bps <= 10000,
        "invalid conversion terms"
    );
    let floor = capture
        .observations
        .iter()
        .filter_map(|o| o.result.as_ref())
        .filter_map(|r| r["context"]["slot"].as_u64())
        .max()
        .unwrap_or(0);
    Ok(Scope {
        mint,
        source,
        raw,
        owner,
        floor,
        amount,
    })
}
pub fn validate(wallet: &str, request: &Request) -> Result<()> {
    scope(wallet, request).map(|_| ())
}
fn proposal(
    scope: &Scope,
    request: &Request,
    destination: &decode::MintConfig,
    candidate: &[u8],
) -> Result<ChangeSpec> {
    let fee = if request.fee_bps == 0 {
        json!({"kind":"none"})
    } else {
        json!({"kind":"source_bps","bps":request.fee_bps})
    };
    let terms: spec::TokenMigrationV1 = serde_json::from_value(json!({"version":1,
      "source":{"mint":scope.source.mint,"token_program":scope.mint.token_program,"decimals":scope.mint.decimals},
      "destination":{"mint":request.replacement_mint,"token_program":destination.token_program,"decimals":destination.decimals},
      "conversion":{"ratio_basis":"raw","numerator":request.numerator,"denominator":request.denominator,"rounding":request.rounding,"fee":fee,"minimum_output_raw":"1"},
      "window":{},"eligibility":{"amount_policy":{"exact_raw":{"amount_raw":scope.amount.to_string()}},"minimum_source_balance_raw":"1","holder_authorization":["owner"],"owner_authority_classes":["wallet"],"excluded_accounts":[]},
      "source_disposition":{"kind":"burn"},"destination_funding":{"kind":"reserve_transfer","reserve":{"kind":"proposed","funded_raw":request.reserve_raw}},
      "authorities":{"migration_authority":{"kind":"program_derived"},"expected":{},"fee_payer":{"kind":"relayer"}}}))?;
    ChangeSpec::token_migration(terms, adapter::REFERENCE_PROGRAM_ID, candidate)
}
fn addresses(scope: &Scope, change: &ChangeSpec) -> Result<Vec<String>> {
    let terms = change
        .as_token_migration()
        .context("migration kind")?
        .evaluation_spec(change.activation.as_ref())?;
    let overlay = adapter::derive(&terms, &change.id()?, adapter::REFERENCE_PROGRAM_ID)?;
    let mut addresses: BTreeSet<_> = super::capture::identity_addresses(&terms, &overlay)
        .into_iter()
        .collect();
    addresses.extend([
        scope.owner.clone(),
        crate::standard_programs::token::CLOCK.into(),
        world::associated_token_address(
            &scope.owner,
            &terms.destination.token_program,
            &terms.destination.mint,
        )?,
    ]);
    Ok(addresses.into_iter().collect())
}
fn record(
    rpc: &impl RpcProvider,
    out: &mut Vec<Observation>,
    method: &str,
    params: Value,
) -> Option<Value> {
    let started_at = chrono::Utc::now().to_rfc3339();
    let response = rpc.call(method, params.clone());
    let (result, error) = match response {
        Ok(v) => (Some(v), None),
        Err(_) => (None, Some("Read-only observation unavailable".into())),
    };
    out.push(Observation {
        method: method.into(),
        params,
        started_at,
        completed_at: chrono::Utc::now().to_rfc3339(),
        result: result.clone(),
        error,
    });
    result
}
fn final_addresses(
    scope: &Scope,
    request: &Request,
    change: &ChangeSpec,
    discovery: &Value,
) -> Result<Vec<String>> {
    let initial = addresses(scope, change)?;
    let mut result: BTreeSet<String> = initial
        .iter()
        .cloned()
        .chain([request.source.clone()])
        .collect();
    let values = discovery["value"].as_array().context("identity values")?;
    ensure!(values.len() == initial.len(), "incomplete identities");
    for raw in values {
        if raw["executable"] == true
            && raw["owner"] == crate::standard_programs::token::UPGRADEABLE_LOADER
        {
            result.insert(
                crate::standard_programs::upgradeable_loader::decode_program(
                    &decode::raw_account_bytes(raw)?,
                )?
                .to_string(),
            );
        }
    }
    ensure!(result.len() <= 100, "account bound");
    Ok(result.into_iter().collect())
}
pub fn capture(
    wallet: String,
    request: Request,
    run_id: String,
    check_id: String,
    candidate: &[u8],
    rpc: &impl RpcProvider,
) -> Result<Capture> {
    let scope = scope(&wallet, &request)?;
    let mut c = Capture {
        schema_version: 1,
        run_id,
        check_id,
        wallet_sha256: hash_bytes(wallet.as_bytes()),
        wallet_capture: wallet,
        request,
        observations: vec![],
    };
    if record(rpc, &mut c.observations, "getGenesisHash", json!([]))
        != Some(json!(world::MAINNET_GENESIS))
    {
        return Ok(c);
    }
    let Some(dest) = record(
        rpc,
        &mut c.observations,
        "getAccountInfo",
        json!([c.request.replacement_mint, coherence::config(scope.floor)]),
    ) else {
        return Ok(c);
    };
    let Ok(destination) = decode::decode_mint(&dest["value"]) else {
        return Ok(c);
    };
    let change = proposal(&scope, &c.request, &destination, candidate)?;
    let floor = dest["context"]["slot"]
        .as_u64()
        .context("destination context")?;
    ensure!(floor >= scope.floor, "destination predates observation");
    let initial = addresses(&scope, &change)?;
    let Some(discovery) = record(
        rpc,
        &mut c.observations,
        "getMultipleAccounts",
        json!([initial, coherence::config(floor)]),
    ) else {
        return Ok(c);
    };
    let floor2 = discovery["context"]["slot"]
        .as_u64()
        .context("identity context")?;
    ensure!(floor2 >= floor, "identities predate destination");
    let final_set = final_addresses(&scope, &c.request, &change, &discovery)?;
    coherence::capture_final(&final_set, floor2, |params| {
        record(rpc, &mut c.observations, "getMultipleAccounts", params)
    });
    Ok(c)
}
pub fn replay(
    bytes: &[u8],
    run: &str,
    check: &str,
    wallet_hash: &str,
    candidate: &[u8],
) -> Result<Verified> {
    ensure!(bytes.len() <= 128 * 1024 * 1024, "candidate capture bound");
    let c: Capture = serde_json::from_slice(bytes)?;
    ensure!(
        c.schema_version == 1
            && c.run_id == run
            && c.check_id == check
            && c.wallet_sha256 == wallet_hash
            && hash_bytes(c.wallet_capture.as_bytes()) == wallet_hash,
        "candidate capture binding mismatch"
    );
    let scope = scope(&c.wallet_capture, &c.request)?;
    let mut result = json!({"schema_version":1,"kind":"current-candidate","run_id":run,"check_id":check,"wallet_capture_sha256":wallet_hash,"execution_capture_sha256":hash_bytes(bytes),"source":c.request.source,"mint":scope.source.mint,"owner":scope.owner,"replacement_mint":c.request.replacement_mint,"amount_raw":scope.amount.to_string(),"status":"Indeterminate","execution_performed":false,"authorization":false,"funds_moved":false,"signer_possession_known":false,"signer_assumed_locally":false,"official_transition":"NotTested","candidate_program_sha256":hash_bytes(candidate),"plan_provenance":"UserProposedCandidate","plan_sha256":crate::canonical::digest(&(&c.request,&c.wallet_sha256,hash_bytes(candidate)))?,"limitations":["Candidate plan only; proposed reserve is declared, never observed issuer funding.","Exact captured account and amount under assumed local signing; no official transition or population readiness."]});
    let evaluated = (|| -> Result<()> {
        ensure!(
            (4..=6).contains(&c.observations.len()),
            "final acquisition incomplete"
        );
        let mut previous = None;
        let evidence = c
            .observations
            .iter()
            .enumerate()
            .map(|(id, o)| {
                let start = chrono::DateTime::parse_from_rfc3339(&o.started_at)?;
                let end = chrono::DateTime::parse_from_rfc3339(&o.completed_at)?;
                ensure!(
                    start <= end && previous.is_none_or(|p| p <= start),
                    "capture intervals invalid"
                );
                previous = Some(end);
                ensure!(o.error.is_none(), "final acquisition unavailable");
                Ok(RpcEvidence {
                    id,
                    method: o.method.clone(),
                    params: o.params.clone(),
                    result: o.result.clone().context("missing observation")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            evidence[0].method == "getGenesisHash"
                && evidence[0].params == json!([])
                && evidence[0].result == world::MAINNET_GENESIS,
            "mainnet identity unavailable"
        );
        ensure!(
            evidence[1].method == "getAccountInfo"
                && evidence[1].params
                    == json!([c.request.replacement_mint, coherence::config(scope.floor)]),
            "destination request changed"
        );
        let destination = decode::decode_mint(&evidence[1].result["value"])?;
        let change = proposal(&scope, &c.request, &destination, candidate)?;
        let terms = change
            .as_token_migration()
            .context("migration kind")?
            .evaluation_spec(change.activation.as_ref())?;
        let floor = evidence[1].result["context"]["slot"]
            .as_u64()
            .context("destination slot")?;
        let floor2 = evidence[2].result["context"]["slot"]
            .as_u64()
            .context("identity slot")?;
        ensure!(
            floor >= scope.floor
                && floor2 >= floor
                && evidence[2].method == "getMultipleAccounts"
                && evidence[2].params
                    == json!([addresses(&scope, &change)?, coherence::config(floor)]),
            "identity discovery changed"
        );
        let final_set = final_addresses(&scope, &c.request, &change, &evidence[2].result)?;
        let context = coherence::verify_after(&evidence, 3, &final_set, floor2, &c.request.source)?;
        let last = evidence.last().context("final batch")?;
        let values = last.result["value"].as_array().context("final values")?;
        let raw: BTreeMap<_, _> = final_set.iter().zip(values).collect();
        ensure!(raw[&c.request.source] == &scope.raw, "SourceStateChanged");
        let mut accounts = BTreeMap::new();
        let mut absent = vec![];
        for (index, (address, value)) in final_set.iter().zip(values).enumerate() {
            if value.is_null() {
                absent.push(address.clone())
            } else {
                accounts.insert(
                    address.clone(),
                    world::WorldAccount {
                        account: world::snapshot_from_rpc(value)?,
                        origin: world::WorldOrigin::Observed {
                            artifact: "candidate.capture.json".into(),
                            record: evidence.len() - 1,
                            pointer: format!(
                                "/observations/{}/result/value/{index}",
                                evidence.len() - 1
                            ),
                            slot: context.final_context_slot,
                        },
                    },
                );
            }
        }
        let clock = world::WorldClock::from_bytes(&decode::raw_account_bytes(
            raw[&crate::standard_programs::token::CLOCK.to_string()],
        )?)?;
        let world = world::World {
            kind: world::WorldKind::ObservedCapture,
            cluster: "solana-mainnet".into(),
            genesis_hash: world::MAINNET_GENESIS.into(),
            clock,
            observed_slots: Some((context.final_context_slot, context.final_context_slot)),
            accounts,
            inspected_absent: absent,
            population: world::PopulationIndex {
                source_mint: terms.source.mint.clone(),
                token_accounts: vec![c.request.source.clone()],
                enumeration_completeness: "Partial".into(),
                authority_resolution_completeness: "Complete".into(),
                undecoded_accounts: vec![],
            },
            limitations: vec![
                "One exact final batch; not a population or historical validator bank".into(),
            ],
            derived_from: None,
        };
        world.validate()?;
        let id = change.id()?;
        let hash = hash_bytes(candidate);
        let planned = planner::plan(&planner::PlanInput {
            spec: &terms,
            change_spec_id: &id,
            world: &world,
            program_id: adapter::REFERENCE_PROGRAM_ID,
            candidate_program_sha256: &hash,
            clock_policy: planner::RehearsalClockPolicy::Captured,
            reserve_override: None,
            focus: Some(&c.request.source),
        })?;
        let unit = planned.unit(&c.request.source).context("selected unit")?;
        result["change_spec"] = serde_json::to_value(&change)?;
        result["execution_plan_sha256"] = planned.sha256()?.into();
        result["world_sha256"] = world.sha256()?.into();
        result["execution_context"] = serde_json::to_value(context)?;
        result["plan_unit"] = serde_json::to_value(unit)?;
        result["clock"] = serde_json::to_value(world.clock)?;
        if !execute::attempted(unit) {
            result["status"] = json!("Unsupported");
            result["reason"] = json!(format!("Candidate precondition: {:?}", unit.class));
            return Ok(());
        }
        let resolved = change.resolve(crate::change::CandidateSource::Bytes(candidate))?;
        let config = adapter::config_bytes(&terms, &terms.resolve()?, &planned.overlay, &id)?;
        let bank = execute::Bank::build(&world, &terms, &planned, &config)?;
        let programs = execute::programs(&world, &terms, adapter::REFERENCE_PROGRAM_ID, &resolved)?;
        execute::assert_candidate(&programs, adapter::REFERENCE_PROGRAM_ID, &hash)?;
        let mut session = execute::Session::new(&bank, &programs, adapter::REFERENCE_PROGRAM_ID)?;
        let execution = execute::execute_unit(&mut session, &terms, &planned, unit, &bank.relayer)?;
        result["status"] = json!(if execution.outcome == execute::Outcome::Migrated
            && execution.reconciled
        {
            "Proven"
        } else {
            "Failed"
        });
        result["execution_performed"] = true.into();
        result["signer_assumed_locally"] = true.into();
        result["execution"] = serde_json::to_value(execution)?;
        Ok(())
    })();
    if let Err(error) = evaluated {
        result["reason"] = format!("{error:#}").into();
    }
    Ok(Verified { value: result })
}

/// Reconstruct the proposal identity from typed terms and the independently observed destination.
pub fn declared_change(capture: &Capture, candidate: &[u8]) -> Result<Option<ChangeSpec>> {
    let scope = scope(&capture.wallet_capture, &capture.request)?;
    let Some(record) = capture.observations.get(1) else {
        return Ok(None);
    };
    ensure!(
        record.method == "getAccountInfo"
            && record.params
                == json!([
                    capture.request.replacement_mint,
                    coherence::config(scope.floor)
                ]),
        "destination request mismatch"
    );
    let Some(result) = &record.result else {
        return Ok(None);
    };
    let Ok(destination) = decode::decode_mint(&result["value"]) else {
        return Ok(None);
    };
    proposal(&scope, &capture.request, &destination, candidate).map(Some)
}
