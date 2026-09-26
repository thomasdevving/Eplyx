//! Frozen current-state cases, coherent final recapture and exact offline rebinding.
//! Capture is read-only. Every VM runs one fixed account, independently; no peer
//! receives evidence and no result is added into population rollout capacity.
use super::{
    adapter, authority_resolution, capture, coherence, current_classify as classify,
    current_select::{self as select, CandidatePlan, Eligibility, SelectedCase, StressTestPlan},
    execute,
    input::ValidatedInput,
    planner::{self, PlanInput, RehearsalClockPolicy},
    population,
    population_types::StressBudget,
    world::{self, World, WorldAccount, WorldKind, WorldOrigin},
};
use crate::{
    canonical::{digest, document},
    evidence::{
        authority::{classify_authority, EntityType},
        paths::PathStatus,
    },
    ingest::{
        observation::{Observation, RpcEvidence},
        rpc::RpcProvider,
    },
    replay::hash_bytes,
    standard_programs::{
        clock::CapturedClock,
        token::{self as decode, ATA_PROGRAM, CLOCK},
        upgradeable_loader,
    },
};
use anyhow::{ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
};

pub const VERSION: &str = "eplyx-migration-current/v1";
pub const MAX_CASE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CAPTURE_BYTES: usize = 192 * 1024 * 1024;
const PLAN: &str = "current.plan.json";
const CAPTURE: &str = "current.capture.json";
const BINDINGS: &str = "current.bindings.json";
const REPORT: &str = "current.report.json";
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
pub(crate) fn read(path: &Path, max: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    ensure!(
        f.metadata()?.len() <= max as u64,
        "capture artifact exceeds bound"
    );
    let mut bytes = Vec::new();
    f.take(max as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= max, "capture artifact grew past bound");
    Ok(bytes)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountPlan {
    pub case_id: String,
    pub case_plan_sha256: String,
    pub addresses: Vec<String>,
    #[serde(with = "crate::numfmt::u64_string")]
    pub min_context_slot: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenPlan {
    pub version: String,
    pub selection: StressTestPlan,
    pub discovery_world_sha256: String,
    pub accounts: Vec<AccountPlan>,
    pub authority: authority_resolution::Plan,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseCapture {
    pub case_id: String,
    pub case_plan_sha256: String,
    pub account_plan_sha256: String,
    pub started_at: String,
    pub completed_at: String,
    pub observations: Vec<Observation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureBundle {
    pub version: String,
    pub plan_sha256: String,
    pub cases: Vec<CaseCapture>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bindings {
    plan_sha256: String,
    capture_sha256: String,
}

/// Reports and contexts can only be recomputed from raw capture, never deserialized.
#[derive(Clone, Debug, Serialize)]
// Construction is restricted inside this crate as well as outside it.
#[allow(clippy::manual_non_exhaustive)]
pub struct CaseResult {
    pub case_id: String,
    pub token_account: String,
    pub entity_id: String,
    pub selection_shape_sha256: String,
    pub selection_bucket: u8,
    pub selected_amount_raw: String,
    pub final_amount_raw: Option<String>,
    pub final_shape_sha256: Option<String>,
    pub classification: String,
    pub changed_fields: Vec<String>,
    pub selection_shape_preserved: bool,
    pub selection_bucket_preserved: bool,
    pub final_bucket_at_discovery_thresholds: Option<u8>,
    pub status: PathStatus,
    pub reason: Option<String>,
    pub execution_performed: bool,
    pub signer_assumed_locally: bool,
    pub signer_possession_known: bool,
    pub issuer_binding_established: bool,
    pub funds_moved: bool,
    pub official_transition: PathStatus,
    pub execution_context: Option<coherence::ExecutionContext>,
    pub execution_plan_sha256: Option<String>,
    pub final_world_sha256: Option<String>,
    pub execution: Option<execute::UnitExecution>,
    #[serde(skip)]
    _verified: (),
}
#[derive(Clone, Debug, Serialize)]
#[allow(clippy::manual_non_exhaustive)]
pub struct CurrentReport {
    pub version: String,
    pub change_spec_id: String,
    pub population_capture_sha256: String,
    pub plan_sha256: String,
    pub capture_sha256: String,
    pub authority_resolution: authority_resolution::Report,
    pub results: Vec<CaseResult>,
    pub coverage: Value,
    pub official_transition: PathStatus,
    pub limitations: Vec<String>,
    #[serde(skip)]
    _verified: (),
}

/// Exact bounded final set, fixed before recapture. Discovery bytes never fill gaps.
pub fn account_plan(
    input: &ValidatedInput,
    discovery: &World,
    case: &SelectedCase,
) -> Result<AccountPlan> {
    let spec = input.spec();
    let overlay = adapter::derive(spec, input.change_spec_id(), input.program_id())?;
    let mut addresses: BTreeSet<String> = capture::identity_addresses(spec, &overlay)
        .into_iter()
        .collect();
    addresses.extend([
        case.token_account.clone(),
        case.authority.clone(),
        CLOCK.into(),
        overlay.migration_authority,
        world::associated_token_address(
            &case.authority,
            &spec.destination.token_program,
            &spec.destination.mint,
        )?,
    ]);
    for id in [
        &spec.source.token_program,
        &spec.destination.token_program,
        &ATA_PROGRAM.to_string(),
    ] {
        let raw = discovery
            .snapshot(id)
            .context("program header missing from discovery")?;
        ensure!(raw.executable, "captured dependency is not executable");
        match raw.owner.as_str() {
            decode::UPGRADEABLE_LOADER => {
                addresses.insert(upgradeable_loader::decode_program(&raw.data)?.to_string());
            }
            world::BPF_LOADER_1 | world::BPF_LOADER_2 => {}
            _ => anyhow::bail!("unsupported captured program loader"),
        }
    }
    ensure!(
        addresses.len() <= 100,
        "final execution account set exceeds one complete batch"
    );
    Ok(AccountPlan {
        case_id: case.case_id.clone(),
        case_plan_sha256: case.case_plan_sha256.clone(),
        addresses: addresses.into_iter().collect(),
        min_context_slot: discovery
            .observed_slots
            .map(|s| s.1)
            .context("current analysis requires observed discovery slots")?
            .max(case.discovery_slot),
    })
}
fn validate_discovery(
    input: &ValidatedInput,
    observation: &population::PopulationObservation,
    discovery: &World,
) -> Result<()> {
    discovery.validate()?;
    ensure!(
        discovery.kind == WorldKind::ObservedCapture
            && discovery.genesis_hash == world::MAINNET_GENESIS
            && observation.acquisition.genesis_hash == world::MAINNET_GENESIS,
        "current analysis requires mainnet observations"
    );
    ensure!(
        discovery.population.source_mint == input.spec().source.mint
            && observation.mint == input.spec().source.mint,
        "population source mint mismatch"
    );
    Ok(())
}
pub fn freeze(
    input: &ValidatedInput,
    population_bytes: &[u8],
    budget: &StressBudget,
    discovery: &World,
    frozen_at: &str,
) -> Result<FrozenPlan> {
    let observation = population::evaluate_bytes(population_bytes, budget)?;
    validate_discovery(input, &observation, discovery)?;
    let selection = select::build(
        &observation,
        &CandidatePlan::new(input),
        input.program_sha256(),
        frozen_at,
    )?;
    freeze_selection(input, &observation, discovery, selection)
}
pub(crate) fn freeze_selection(
    input: &ValidatedInput,
    observation: &population::PopulationObservation,
    discovery: &World,
    selection: StressTestPlan,
) -> Result<FrozenPlan> {
    let accounts = selection
        .selected
        .iter()
        .map(|case| account_plan(input, discovery, case))
        .collect::<Result<_>>()?;
    Ok(FrozenPlan {
        version: VERSION.into(),
        selection,
        discovery_world_sha256: discovery.sha256()?,
        accounts,
        authority: authority_resolution::plan(observation)?,
    })
}

/// Call only after persisting the frozen plan. Serial read-only requests; at most
/// three attempts per case. Provider errors remain selected, explicitly unevaluated.
pub(crate) fn capture_frozen(plan: &FrozenPlan, rpc: &impl RpcProvider) -> Result<CaptureBundle> {
    let mut cases = vec![];
    let mut captured_bytes = 0usize;
    for account in &plan.accounts {
        let mut capture = CaseCapture {
            case_id: account.case_id.clone(),
            case_plan_sha256: account.case_plan_sha256.clone(),
            account_plan_sha256: digest(account)?,
            started_at: now(),
            completed_at: String::new(),
            observations: vec![],
        };
        let started_at = now();
        let genesis = rpc.call("getGenesisHash", json!([]));
        let completed_at = now();
        let (result, error) = match genesis {
            Ok(value) => (Some(value), None),
            Err(_) => (None, Some("ProviderReadFailed".into())),
        };
        let mainnet = result.as_ref().and_then(Value::as_str) == Some(world::MAINNET_GENESIS);
        capture.observations.push(Observation {
            method: "getGenesisHash".into(),
            params: json!([]),
            started_at,
            completed_at,
            result,
            error,
        });
        if !mainnet {
            capture.completed_at = now();
            cases.push(capture);
            continue;
        }
        coherence::capture_final(&account.addresses, account.min_context_slot, |params| {
            let started_at = now();
            let result = if captured_bytes >= MAX_CAPTURE_BYTES {
                Err(anyhow::anyhow!("capture byte budget exhausted"))
            } else {
                rpc.call("getMultipleAccounts", params.clone())
            };
            let completed_at = now();
            // Provider text may contain configuration; retain a stable error category only.
            let (result, error) = match result {
                Ok(value)
                    if serde_json::to_vec(&value).is_ok_and(|v| {
                        let size = v.len();
                        captured_bytes = captured_bytes.saturating_add(size);
                        size <= MAX_CASE_BYTES && captured_bytes <= MAX_CAPTURE_BYTES
                    }) =>
                {
                    (Some(value), None)
                }
                Ok(_) => (None, Some("ResponseByteLimit".into())),
                Err(_) => (None, Some("ProviderReadFailed".into())),
            };
            capture.observations.push(Observation {
                method: "getMultipleAccounts".into(),
                params,
                started_at,
                completed_at,
                result: result.clone(),
                error,
            });
            result
        });
        capture.completed_at = now();
        cases.push(capture);
    }
    let bundle = CaptureBundle {
        version: VERSION.into(),
        plan_sha256: digest(plan)?,
        cases,
    };
    ensure!(
        serde_json::to_vec(&bundle)?.len() <= MAX_CAPTURE_BYTES,
        "case capture bundle exceeds bound"
    );
    Ok(bundle)
}

fn empty_result(case: &SelectedCase) -> CaseResult {
    CaseResult {
        case_id: case.case_id.clone(),
        token_account: case.token_account.clone(),
        entity_id: case.entity_id.clone(),
        selection_shape_sha256: case.state_shape_sha256.clone(),
        selection_bucket: case.balance_bucket,
        selected_amount_raw: case.selected_amount_raw.clone(),
        final_amount_raw: None,
        final_shape_sha256: None,
        classification: "Indeterminate".into(),
        changed_fields: vec![],
        selection_shape_preserved: false,
        selection_bucket_preserved: false,
        final_bucket_at_discovery_thresholds: None,
        status: PathStatus::Indeterminate,
        reason: None,
        execution_performed: false,
        signer_assumed_locally: false,
        signer_possession_known: false,
        issuer_binding_established: false,
        funds_moved: false,
        official_transition: PathStatus::NotTested,
        execution_context: None,
        execution_plan_sha256: None,
        final_world_sha256: None,
        execution: None,
        _verified: (),
    }
}
fn held(mut result: CaseResult, classification: &str, reason: impl Into<String>) -> CaseResult {
    result.classification = classification.into();
    result.reason = Some(reason.into());
    result
}
fn frozen_bucket(plan: &StressTestPlan, amount: u64) -> Option<u8> {
    if amount == 0 {
        return None;
    }
    plan.buckets
        .boundaries
        .iter()
        .find(|b| b.max_raw.parse::<u64>().is_ok_and(|max| amount <= max))
        .or_else(|| plan.buckets.boundaries.last())
        .map(|b| b.bucket)
}

pub(crate) fn evaluate_case(
    input: &ValidatedInput,
    plan: &FrozenPlan,
    population: &population::Capture,
    observation: &population::PopulationObservation,
    case: &SelectedCase,
    account: &AccountPlan,
    capture: &CaseCapture,
) -> Result<CaseResult> {
    ensure!(
        case.case_id == capture.case_id
            && case.case_plan_sha256 == capture.case_plan_sha256
            && capture.account_plan_sha256 == digest(account)?
            && account.case_id == case.case_id,
        "capture substituted a frozen case"
    );
    ensure!(
        capture.observations.len() <= 1 + coherence::MAX_FINAL_ATTEMPTS,
        "final attempt budget exceeded"
    );
    let mut result = empty_result(case);
    let mut previous = chrono::DateTime::parse_from_rfc3339(&capture.started_at)?;
    let end = chrono::DateTime::parse_from_rfc3339(&capture.completed_at)?;
    ensure!(
        chrono::DateTime::parse_from_rfc3339(&plan.selection.frozen_at)? <= previous
            && previous <= end,
        "capture precedes frozen selection"
    );
    for record in &capture.observations {
        let start = chrono::DateTime::parse_from_rfc3339(&record.started_at)?;
        let finish = chrono::DateTime::parse_from_rfc3339(&record.completed_at)?;
        ensure!(
            previous <= start
                && start <= finish
                && finish <= end
                && record.result.is_some() != record.error.is_some(),
            "invalid capture interval/outcome"
        );
        previous = finish;
    }
    if capture.observations.is_empty() || capture.observations.iter().any(|r| r.error.is_some()) {
        return Ok(held(
            result,
            coherence::COHERENCE_FAILURE,
            "Final read unavailable; selected account retained without execution.",
        ));
    }
    let evidence = capture
        .observations
        .iter()
        .enumerate()
        .map(|(id, r)| RpcEvidence {
            id,
            method: r.method.clone(),
            params: r.params.clone(),
            result: r.result.clone().unwrap(),
        })
        .collect::<Vec<_>>();
    ensure!(
        evidence[0].method == "getGenesisHash"
            && evidence[0].params == json!([])
            && evidence[0].result == world::MAINNET_GENESIS,
        "final capture is not bound to Solana mainnet"
    );
    let context = match coherence::verify_after(
        &evidence,
        1,
        &account.addresses,
        account.min_context_slot,
        &case.token_account,
    ) {
        Ok(context) => context,
        Err(error) => {
            return Ok(held(
                result,
                coherence::COHERENCE_FAILURE,
                error.to_string(),
            ))
        }
    };
    let final_record = evidence.last().context("final record missing")?;
    let values = final_record.result["value"]
        .as_array()
        .context("final values missing")?;
    let raw: BTreeMap<&str, &Value> = account
        .addresses
        .iter()
        .map(String::as_str)
        .zip(values)
        .collect();
    let entity = observation
        .entities
        .iter()
        .find(|e| e.token_account == case.token_account)
        .context("selected entity missing")?;
    ensure!(
        classify::assumed_local_signer(&entity.authority_model, entity.authority_resolution),
        "non-wallet case cannot receive a wallet signer"
    );
    let pointer = entity
        .token_account_evidence
        .pointer
        .strip_suffix("/account")
        .context("rebinding requires exact schema 2 population pointer")?;
    let row = population
        .observations
        .get(entity.token_account_evidence.rpc_id)
        .and_then(|r| r.result.as_ref())
        .and_then(|r| r.pointer(pointer))
        .context("selected raw source missing")?;
    ensure!(
        row["pubkey"] == case.token_account,
        "population source pointer does not bind selected identity"
    );
    let original = &row["account"];
    let mint = observation
        .mint_config
        .as_ref()
        .context("source mint missing")?;
    let old = decode::decode_token_account(
        original,
        &mint.token_program,
        &plan.selection.asset_mint,
        mint.decimals,
    )?;
    ensure!(
        old == entity.state,
        "population state differs from pinned raw bytes"
    );
    let source = raw[case.token_account.as_str()];
    let spec = input.spec();
    let new = match decode::decode_token_account(
        source,
        &spec.source.token_program,
        &spec.source.mint,
        spec.source.decimals,
    ) {
        Ok(state) => state,
        Err(error) => {
            return Ok(held(
                result,
                "IdentityChanged",
                format!("Source identity or layout changed: {error:#}"),
            ))
        }
    };
    let final_mint = match decode::decode_mint(raw[spec.source.mint.as_str()]) {
        Ok(m)
            if m.decimals == spec.source.decimals
                && m.token_program == spec.source.token_program =>
        {
            m
        }
        _ => {
            return Ok(held(
                result,
                "IdentityChanged",
                "Source mint identity changed",
            ))
        }
    };
    result.execution_context = Some(context.clone());
    result.changed_fields = source_field_changes(original, source, &old, &new)?;
    if final_mint.raw_supply != mint.raw_supply {
        result.changed_fields.push("source_mint_supply".into());
    }
    if final_mint.extensions != mint.extensions {
        result.changed_fields.push("source_mint_extensions".into());
    }
    let amount: u64 = new.raw_balance.parse()?;
    let mut final_entity = entity.clone();
    final_entity.state = new.clone();
    let (final_authority, final_observation, reason) =
        classify_authority(&case.authority, raw[case.authority.as_str()])?;
    final_entity.authority_model = final_authority.clone();
    final_entity.authority_observation = final_observation;
    final_entity.classification_reason = reason;
    let dimensions = classify::dimensions(&final_entity, &final_mint)?;
    let shape = classify::shape_key(&dimensions)?;
    result.final_amount_raw = Some(amount.to_string());
    result.final_shape_sha256 = Some(shape.clone());
    result.final_bucket_at_discovery_thresholds = frozen_bucket(&plan.selection, amount);
    result.selection_shape_preserved = shape == case.state_shape_sha256;
    result.selection_bucket_preserved =
        result.final_bucket_at_discovery_thresholds == Some(case.balance_bucket);
    if new.owner != case.authority
        || amount == 0
        || final_authority != EntityType::WalletCompatible
        || classify::eligibility(&dimensions).0 != Eligibility::ExecutableCandidate
    {
        return Ok(held(
            result,
            "NoLongerExecutable",
            "Final state cannot execute the frozen wallet case; no peer or amount substitute.",
        ));
    }
    if decode::raw_account_bytes(original)? != decode::raw_account_bytes(source)? && old == new {
        return Ok(held(
            result,
            "UnclassifiedSourceDataChange",
            "Changed source bytes are not explained by decoded fields.",
        ));
    }
    let mut accounts = BTreeMap::new();
    let mut absent = vec![];
    for (index, (address, value)) in account.addresses.iter().zip(values).enumerate() {
        if value.is_null() {
            absent.push(address.clone());
        } else {
            accounts.insert(
                address.clone(),
                WorldAccount {
                    account: world::snapshot_from_rpc(value)?,
                    origin: WorldOrigin::Observed {
                        artifact: CAPTURE.into(),
                        record: evidence.len() - 1,
                        pointer: format!(
                            "/cases/{}/observations/{}/result/value/{index}",
                            case.selection_order,
                            evidence.len() - 1
                        ),
                        slot: context.final_context_slot,
                    },
                },
            );
        }
    }
    let clock = CapturedClock::from_bytes(&decode::raw_account_bytes(raw[CLOCK])?)?;
    let final_world=World{kind:WorldKind::ObservedCapture,cluster:"solana-mainnet".into(),genesis_hash:world::MAINNET_GENESIS.into(),clock,observed_slots:Some((context.final_context_slot,context.final_context_slot)),accounts,inspected_absent:absent,population:world::PopulationIndex{source_mint:spec.source.mint.clone(),token_accounts:vec![case.token_account.clone()],enumeration_completeness:"Partial".into(),authority_resolution_completeness:"Complete".into(),undecoded_accounts:vec![]},limitations:vec!["Exact final account batch and matching Clock; not a historical validator bank or population-wide proof.".into()],derived_from:None};
    let execution_plan = match planner::plan(&PlanInput {
        spec,
        change_spec_id: input.change_spec_id(),
        world: &final_world,
        program_id: input.program_id(),
        candidate_program_sha256: input.program_sha256(),
        clock_policy: RehearsalClockPolicy::Captured,
        reserve_override: None,
        focus: Some(&case.token_account),
    }) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(held(
                result,
                "FinalStateUnsupported",
                format!("Final migration inputs unavailable: {error:#}"),
            ))
        }
    };
    let unit = execution_plan
        .unit(&case.token_account)
        .context("exact final unit missing")?;
    if !execute::attempted(unit) {
        return Ok(held(
            result,
            "NoLongerExecutable",
            format!("Final migration precondition: {:?}", unit.class),
        ));
    }
    ensure!(
        unit.amount_raw == amount.to_string(),
        "resolved execution amount is not the exact final balance"
    );
    result.final_world_sha256 = Some(final_world.sha256()?);
    result.execution_plan_sha256 = Some(digest(&(
        VERSION,
        &case.case_plan_sha256,
        &capture.account_plan_sha256,
        digest(capture)?,
        &result.final_world_sha256,
        &context,
        input.program_sha256(),
        execution_plan.sha256()?,
        amount.to_string(),
    ))?);
    let config = adapter::config_bytes(
        spec,
        &spec.resolve()?,
        &execution_plan.overlay,
        input.change_spec_id(),
    )?;
    let relayer = execute::relayer().to_string();
    let (instructions, _) = execute::unit_instructions(spec, &execution_plan, unit, &relayer)?;
    let keys = execute::message_keys(&instructions);
    let bank =
        execute::Bank::build_filtered(&final_world, spec, &execution_plan, &config, Some(&keys))?;
    let programs =
        match execute::programs(&final_world, spec, input.program_id(), input.candidate()) {
            Ok(programs) => programs,
            Err(error) => {
                return Ok(held(
                    result,
                    "FinalStateUnsupported",
                    format!("Captured executable identity unavailable: {error:#}"),
                ))
            }
        };
    execute::assert_candidate(&programs, input.program_id(), input.program_sha256())?;
    let mut session = execute::Session::new(&bank, &programs, input.program_id())?;
    let execution =
        execute::execute_unit(&mut session, spec, &execution_plan, unit, &bank.relayer)?;
    result.status = if execution.outcome == execute::Outcome::Migrated {
        PathStatus::Proven
    } else {
        PathStatus::Failed
    };
    result.execution_performed = true;
    result.signer_assumed_locally = true;
    result.execution = Some(execution);
    result.classification =
        if result.selection_shape_preserved && result.selection_bucket_preserved {
            "ExecutableCurrentState"
        } else {
            "SelectionStateChangedButExecutable"
        }
        .into();
    Ok(result)
}

pub(crate) fn evaluate_frozen(
    input: &ValidatedInput,
    population_bytes: &[u8],
    budget: &StressBudget,
    plan: &FrozenPlan,
    captures: &CaptureBundle,
) -> Result<CurrentReport> {
    let observation = population::evaluate_bytes(population_bytes, budget)?;
    let population: population::Capture = serde_json::from_slice(population_bytes)?;
    ensure!(
        plan.version == VERSION
            && captures.version == VERSION
            && captures.plan_sha256 == digest(plan)?
            && captures.cases.len() == plan.selection.selected.len()
            && plan.accounts.len() == captures.cases.len(),
        "capture does not match frozen plan"
    );
    let authority_resolution =
        authority_resolution::resolve_bytes(&plan.authority, population_bytes, budget)?;
    let mut results = vec![];
    for ((case, account), capture) in plan
        .selection
        .selected
        .iter()
        .zip(&plan.accounts)
        .zip(&captures.cases)
    {
        results.push(evaluate_case(
            input,
            plan,
            &population,
            &observation,
            case,
            account,
            capture,
        )?);
    }
    let executed: Vec<_> = results.iter().filter(|r| r.execution_performed).collect();
    let exact: Vec<_> = executed.iter().map(|r| r.entity_id.clone()).collect();
    let shape_coverage:Vec<Value>=plan.selection.state_shapes.iter().map(|shape| {
        let selected:Vec<_>=results.iter().filter(|r|r.selection_shape_sha256==shape.state_shape_sha256).collect();
        let proven:Vec<_>=selected.iter().filter(|r|r.execution_performed && r.selection_shape_preserved).map(|r|r.entity_id.clone()).collect();
        json!({"state_shape_sha256":shape.state_shape_sha256,"entities_in_shape":shape.entities_in_shape,"selected":selected.len(),"exact_execution_entity_ids":proven,"scope":"Only exact listed final executions; no peer inherits evidence."})
    }).collect();
    Ok(CurrentReport{version:VERSION.into(),change_spec_id:input.change_spec_id().into(),population_capture_sha256:hash_bytes(population_bytes),plan_sha256:digest(plan)?,capture_sha256:digest(captures)?,authority_resolution,coverage:json!({"positive_balance_accounts_observed":observation.positive_entities().count(),"exact_accounts_selected":results.len(),"exact_accounts_executed":executed.len(),"exact_accounts_proven":results.iter().filter(|r|r.status==PathStatus::Proven).count(),"exact_accounts_failed":results.iter().filter(|r|r.status==PathStatus::Failed).count(),"exact_execution_entity_ids":exact,"selection_state_changed_but_executable":results.iter().filter(|r|r.classification=="SelectionStateChangedButExecutable").count(),"shape_coverage":shape_coverage,"population_rollout_readiness":"Incomplete","note":"Independent exact case executions are never summed into rollout capacity; selection is not a statistical sample."}),results,official_transition:PathStatus::NotTested,limitations:vec!["Current finalized observations, not a historical validator bank; no guarantee about later state.".into(),"Exact local candidate execution under assumed signing; key possession remains Unknown and OfficialTransition is NotTested.".into()],_verified:()})
}

/// Persist selection before any final RPC. Persist exact captures and bindings
/// before executing. New directories and files only; replay uses no provider.
pub fn capture_with(
    input: &ValidatedInput,
    population_bytes: &[u8],
    budget: &StressBudget,
    discovery: &World,
    output: &Path,
    rpc: &impl RpcProvider,
) -> Result<String> {
    let plan = freeze(input, population_bytes, budget, discovery, &now())?;
    std::fs::create_dir(output)?;
    write_new(&output.join(PLAN), document(&plan)?.as_bytes())?;
    write_new(&output.join("population.capture.json"), population_bytes)?;
    write_new(
        &output.join("discovery.world.json"),
        document(discovery)?.as_bytes(),
    )?;
    let capture = capture_frozen(&plan, rpc)?;
    write_new(&output.join(CAPTURE), document(&capture)?.as_bytes())?;
    write_new(
        &output.join(BINDINGS),
        document(&Bindings {
            plan_sha256: digest(&plan)?,
            capture_sha256: digest(&capture)?,
        })?
        .as_bytes(),
    )?;
    Ok(hash_bytes(&read(&output.join(BINDINGS), 4096)?))
}
pub fn run_with(
    input: &ValidatedInput,
    population_bytes: &[u8],
    budget: &StressBudget,
    discovery: &World,
    output: &Path,
    rpc: &impl RpcProvider,
) -> Result<CurrentReport> {
    capture_with(input, population_bytes, budget, discovery, output, rpc)?;
    finish(input, output, budget)
}
/// Rebuild selection, exact capture chain, final state and every VM outcome.
pub fn replay(
    input: &ValidatedInput,
    output: &Path,
    budget: &StressBudget,
) -> Result<CurrentReport> {
    let report = evaluate_saved(input, output, budget)?;
    ensure!(
        read(&output.join(REPORT), MAX_CAPTURE_BYTES)? == document(&report)?.as_bytes(),
        "current report differs from offline recomputation"
    );
    Ok(report)
}
pub fn finish(
    input: &ValidatedInput,
    output: &Path,
    budget: &StressBudget,
) -> Result<CurrentReport> {
    let report = evaluate_saved(input, output, budget)?;
    write_new(&output.join(REPORT), document(&report)?.as_bytes())?;
    Ok(report)
}
fn evaluate_saved(
    input: &ValidatedInput,
    output: &Path,
    budget: &StressBudget,
) -> Result<CurrentReport> {
    let population = read(
        &output.join("population.capture.json"),
        budget.max_artifact_bytes as usize,
    )?;
    let discovery: World = serde_json::from_slice(&read(
        &output.join("discovery.world.json"),
        512 * 1024 * 1024,
    )?)?;
    let bytes = read(&output.join(PLAN), 16 * 1024 * 1024)?;
    let plan: FrozenPlan = serde_json::from_slice(&bytes)?;
    let rebuilt = freeze(
        input,
        &population,
        budget,
        &discovery,
        &plan.selection.frozen_at,
    )?;
    ensure!(
        plan == rebuilt && bytes == document(&rebuilt)?.as_bytes(),
        "frozen current selection was rewritten"
    );
    let bindings: Bindings = serde_json::from_slice(&read(&output.join(BINDINGS), 4096)?)?;
    let captures = read(&output.join(CAPTURE), MAX_CAPTURE_BYTES)?;
    ensure!(
        bindings.plan_sha256 == hash_bytes(&bytes)
            && bindings.capture_sha256 == hash_bytes(&captures),
        "frozen capture digest mismatch"
    );
    let captures: CaptureBundle = serde_json::from_slice(&captures)?;
    let report = evaluate_frozen(input, &population, budget, &plan, &captures)?;
    Ok(report)
}
fn source_field_changes(
    original: &Value,
    final_raw: &Value,
    old: &crate::standard_programs::token::TokenAccountState,
    new: &crate::standard_programs::token::TokenAccountState,
) -> Result<Vec<String>> {
    let mut changed = Vec::new();
    for (key, label) in [
        ("mint", "source_mint"),
        ("owner", "recorded_authority"),
        ("raw_balance", "token_raw_amount"),
        ("delegate", "delegate"),
        ("delegated_amount", "delegated_amount"),
        ("account_state", "account_state"),
        ("close_authority", "close_authority"),
        ("native_reserve", "native_reserve"),
        ("extensions", "account_extensions"),
    ] {
        let before = serde_json::to_value(old)?;
        let after = serde_json::to_value(new)?;
        if before[key] != after[key] {
            changed.push(label.to_string());
            if key == "extensions" {
                let withheld = |value: &Value| {
                    value["extensions"]
                        .as_array()
                        .and_then(|extensions| {
                            extensions.iter().find(|extension| {
                                extension["extension_type"] == "TransferFeeAmount"
                            })
                        })
                        .map(|extension| extension["config"]["withheldAmount"].clone())
                };
                if withheld(&before) != withheld(&after) {
                    changed.push("withheld_amount".into());
                }
            }
        }
    }
    for (key, label) in [
        ("owner", "runtime_owner"),
        ("executable", "executable"),
        ("lamports", "lamports"),
        ("rentEpoch", "rent_epoch"),
        ("space", "space"),
    ] {
        if original[key] != final_raw[key] {
            changed.push(label.into());
        }
    }
    if original["data"] != final_raw["data"] {
        changed.push("raw_data_bytes".into());
    }
    Ok(changed)
}
