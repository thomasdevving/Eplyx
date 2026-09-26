//! Additional observed search waves. Freeze exact identities before each read;
//! selection uses only prior-wave outcomes, never this wave's partial successes.
use super::{
    current::{self, CaptureBundle, CaseResult, CurrentReport, FrozenPlan},
    current_classify as classify,
    current_select::{
        self as select, Eligibility, SelectedCase, SelectionReason, StressTestPlan,
        FULL_AT_FINAL_POLICY,
    },
    input::ValidatedInput,
    population,
    population_types::StressBudget,
    world::World,
};
use crate::{
    canonical::{digest, document},
    evidence::paths::PathStatus,
    ingest::rpc::RpcProvider,
    replay::hash_bytes,
    standard_programs::token as decode,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};
pub const VERSION: &str = "eplyx-migration-observed-search/v1";
pub const MAX_OBSERVED: usize = 25;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Freeze {
    version: String,
    wave: usize,
    analysis_input_sha256: String,
    parent_plan_sha256: String,
    wave_plan_sha256: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ObservedWitness {
    pub provenance: &'static str,
    pub witness_id: String,
    pub wave: usize,
    pub token_account: String,
    pub final_amount_raw: String,
    pub final_world_sha256: String,
    pub execution_plan_sha256: String,
    pub result_sha256: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ObservedSearch {
    pub version: String,
    pub max_additional_accounts: usize,
    pub additional_selected: usize,
    pub waves: Vec<CurrentReport>,
    pub counterexamples: Vec<ObservedWitness>,
    pub conclusion: String,
}
fn frozen(
    wave: usize,
    input: &ValidatedInput,
    parent: &FrozenPlan,
    plan: &FrozenPlan,
) -> Result<Freeze> {
    Ok(Freeze {
        version: VERSION.into(),
        wave,
        analysis_input_sha256: input.analysis_input_sha256().into(),
        parent_plan_sha256: digest(parent)?,
        wave_plan_sha256: digest(plan)?,
    })
}
pub(crate) fn select_wave(
    base: &StressTestPlan,
    population: &population::PopulationObservation,
    previous: &[CaseResult],
    wave: usize,
    frozen_at: &str,
) -> Result<StressTestPlan> {
    let mint = population
        .mint_config
        .as_ref()
        .context("missing mint for search selection")?;
    let (_, buckets) = select::buckets(population)?;
    let excluded: BTreeSet<&str> = previous.iter().map(|r| r.token_account.as_str()).collect();
    let mut covered: BTreeSet<String> = previous
        .iter()
        .filter(|r| r.execution_performed)
        .map(|r| {
            r.final_shape_sha256
                .clone()
                .unwrap_or_else(|| r.selection_shape_sha256.clone())
        })
        .collect();
    let failures: Vec<u64> = previous
        .iter()
        .filter(|r| r.status == PathStatus::Failed)
        .filter_map(|r| r.final_amount_raw.as_deref()?.parse().ok())
        .collect();
    let mut candidates = Vec::new();
    for entity in population.positive_entities() {
        if excluded.contains(entity.token_account.as_str()) {
            continue;
        }
        let dimensions = classify::dimensions(entity, mint)?;
        if classify::eligibility(&dimensions).0 != Eligibility::ExecutableCandidate {
            continue;
        }
        candidates.push((
            entity,
            entity.balance()?,
            classify::shape_key(&dimensions)?,
            classify::shape_label(&dimensions),
        ));
    }
    let mut selected = Vec::new();
    let cap = if wave == 3 { 5 } else { 10 };
    while selected.len() < cap && !candidates.is_empty() {
        candidates.sort_by(|a, b| {
            let key = |x: &(
                &crate::migration::population_types::StressEntity,
                u64,
                String,
                String,
            )| {
                (
                    if covered.contains(&x.2) { 1 } else { 0 },
                    failures
                        .iter()
                        .map(|f| f.abs_diff(x.1))
                        .min()
                        .unwrap_or(u64::MAX),
                    std::cmp::Reverse(x.1),
                    x.0.token_account.clone(),
                )
            };
            key(a).cmp(&key(b))
        });
        let (entity, balance, shape, label) = candidates.remove(0);
        let new_shape = covered.insert(shape.clone());
        let reason = if new_shape {
            SelectionReason::NewStateShape
        } else {
            SelectionReason::HighestRemainingBalance
        };
        let amount_decimal = decode::decimal_amount(balance, mint.decimals);
        let mut case_plan = base.candidate_plan.clone();
        case_plan.source_account = Some(entity.token_account.clone());
        case_plan.validate()?;
        let order = selected.len();
        selected.push(SelectedCase {
            case_id: format!("search-w{wave}-{order:02}"),
            selection_order: order,
            selection_reason: reason,
            selection_detail: match reason {
                SelectionReason::NewStateShape => "Untested exact account adds a discovered executable state shape.".into(),
                SelectionReason::HighestRemainingBalance if !failures.is_empty() => "Untested exact account is a nearest balance neighbor to a prior-wave executed failure.".into(),
                _ => "Untested exact account has an extreme balance in the supported class.".into(),
            },
            entity_id: entity.entity_id.clone(),
            token_account: entity.token_account.clone(),
            authority: entity.authority.clone(),
            authority_model: entity.authority_model.clone(),
            state_shape_sha256: shape,
            shape_label: label,
            balance_bucket: *buckets.get(&entity.token_account).context("missing frozen bucket")?,
            observed_balance_raw: balance.to_string(),
            discovery_slot: entity.token_account_evidence.slot,
            selected_amount_raw: balance.to_string(),
            selected_amount_decimal: amount_decimal,
            amount_policy: FULL_AT_FINAL_POLICY.into(),
            amount_capped: false,
            case_plan_sha256: case_plan.sha256()?,
            case_plan,
        });
    }
    let mut plan = base.clone();
    plan.selected = selected;
    plan.selector_version = VERSION.into();
    plan.ordering_rule = "Prior-wave outcomes only; new executable shape, nearest earlier failed balance, then highest balance and account address. Every selected identity is frozen before capture.".into();
    plan.selection_strategy = vec![plan.ordering_rule.clone()];
    plan.frozen_at = frozen_at.into();
    Ok(plan)
}

fn conclude(waves: Vec<CurrentReport>) -> Result<ObservedSearch> {
    let additional_selected = waves.iter().map(|w| w.results.len()).sum();
    ensure!(
        additional_selected <= MAX_OBSERVED,
        "observed wave budget exceeded"
    );
    let mut counterexamples = vec![];
    for (index, wave) in waves.iter().enumerate() {
        for r in &wave.results {
            if r.execution_performed && r.status == PathStatus::Failed {
                let result_sha256 = digest(r)?;
                counterexamples.push(ObservedWitness {
                    provenance: "Observed",
                    witness_id: format!("observed-{}", &result_sha256[..24]),
                    wave: index + 1,
                    token_account: r.token_account.clone(),
                    final_amount_raw: r
                        .final_amount_raw
                        .clone()
                        .context("executed amount missing")?,
                    final_world_sha256: r
                        .final_world_sha256
                        .clone()
                        .context("executed world missing")?,
                    execution_plan_sha256: r
                        .execution_plan_sha256
                        .clone()
                        .context("execution plan missing")?,
                    result_sha256,
                });
            }
        }
    }
    let conclusion = if counterexamples.is_empty() {
        super::search::NO_FINDING.into()
    } else {
        format!(
            "{} exact observed local VM failure witnesses; no population-wide inference.",
            counterexamples.len()
        )
    };
    Ok(ObservedSearch {
        version: VERSION.into(),
        max_additional_accounts: MAX_OBSERVED,
        additional_selected,
        waves,
        counterexamples,
        conclusion,
    })
}
fn compute(
    input: &ValidatedInput,
    parent_path: &Path,
    budget: &StressBudget,
    output: &Path,
    rpc: Option<&dyn RpcProvider>,
) -> Result<ObservedSearch> {
    let baseline = current::replay(input, parent_path, budget)?;
    let population_bytes = current::read(
        &parent_path.join("population.capture.json"),
        budget.max_artifact_bytes as usize,
    )?;
    let population = population::evaluate_bytes(&population_bytes, budget)?;
    let parent: FrozenPlan = serde_json::from_slice(&current::read(
        &parent_path.join("current.plan.json"),
        16 * 1024 * 1024,
    )?)?;
    let discovery: World = serde_json::from_slice(&current::read(
        &parent_path.join("discovery.world.json"),
        512 * 1024 * 1024,
    )?)?;
    let mut previous = baseline.results;
    let mut waves = vec![];
    for wave in 1..=3 {
        let plan_path = output.join(format!("wave-{wave}.plan.json"));
        let freeze_path = output.join(format!("wave-{wave}.freeze.json"));
        let capture_path = output.join(format!("wave-{wave}.capture.json"));
        let binding_path = output.join(format!("wave-{wave}.capture-sha256.json"));
        let frozen_at = if rpc.is_some() {
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        } else {
            let p: FrozenPlan =
                serde_json::from_slice(&current::read(&plan_path, 16 * 1024 * 1024)?)?;
            p.selection.frozen_at
        };
        let selection = select_wave(&parent.selection, &population, &previous, wave, &frozen_at)?;
        let empty = selection.selected.is_empty();
        let plan = current::freeze_selection(input, &population, &discovery, selection)?;
        let freeze = frozen(wave, input, &parent, &plan)?;
        let capture = if let Some(rpc) = rpc {
            current::write_new(&plan_path, document(&plan)?.as_bytes())?;
            current::write_new(&freeze_path, document(&freeze)?.as_bytes())?;
            // Both identities are synced before the first request for this wave.
            let c = current::capture_frozen(&plan, &Provider(rpc))?;
            current::write_new(&capture_path, document(&c)?.as_bytes())?;
            current::write_new(&binding_path, document(&digest(&c)?)?.as_bytes())?;
            c
        } else {
            ensure!(
                current::read(&plan_path, 16 * 1024 * 1024)? == document(&plan)?.as_bytes()
                    && current::read(&freeze_path, 16 * 1024)? == document(&freeze)?.as_bytes(),
                "observed wave selection or parent identity changed after freeze"
            );
            let bytes = current::read(&capture_path, current::MAX_CAPTURE_BYTES)?;
            let expected: String = serde_json::from_slice(&current::read(&binding_path, 4096)?)?;
            ensure!(
                hash_bytes(&bytes) == expected,
                "observed wave capture digest changed"
            );
            serde_json::from_slice::<CaptureBundle>(&bytes)?
        };
        let report = current::evaluate_frozen(input, &population_bytes, budget, &plan, &capture)?;
        previous.extend(report.results.iter().cloned());
        waves.push(report);
        if empty {
            break;
        }
    }
    conclude(waves)
}
struct Provider<'a>(&'a dyn RpcProvider);
impl RpcProvider for Provider<'_> {
    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        self.0.call(method, params)
    }
}
pub fn run_with(
    input: &ValidatedInput,
    parent: &Path,
    budget: &StressBudget,
    output: &Path,
    rpc: &impl RpcProvider,
) -> Result<ObservedSearch> {
    std::fs::create_dir(output)?;
    let result = compute(input, parent, budget, output, Some(rpc))?;
    current::write_new(
        &output.join("observed-search.json"),
        document(&result)?.as_bytes(),
    )?;
    Ok(result)
}
pub fn replay(
    input: &ValidatedInput,
    parent: &Path,
    budget: &StressBudget,
    output: &Path,
) -> Result<ObservedSearch> {
    let result = compute(input, parent, budget, output, None)?;
    ensure!(
        current::read(
            &output.join("observed-search.json"),
            current::MAX_CAPTURE_BYTES
        )? == document(&result)?.as_bytes(),
        "observed search result differs from offline recomputation"
    );
    Ok(result)
}
