//! Deterministic balance bucketing, shape grouping and bounded test selection,
//! frozen into an immutable plan before any capture or execution happens.
//!
//! Selection reads only freshly captured population facts. No execution outcome
//! is visible to this module, so a case can never be chosen, replaced, re-aimed
//! or dropped because of what it later did.
use super::{
    current_classify as classify,
    current_classify::ShapeDimensions,
    population::PopulationObservation,
    population_types::{
        AuthorityResolutionCompleteness, EnumerationCompleteness, StressBudget, StressEntity,
    },
};
use crate::{
    canonical::{digest, document as canonical},
    evidence::authority::EntityType,
    standard_programs::token as decode,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

pub const SELECTOR_VERSION: &str = "eplyx-migration-current-select/v1";
pub const SELECTOR_VERSION_V2: &str = "eplyx-migration-current-select/v2";
pub const PLAN_KIND: &str = "migration-current-plan";
pub const ORDERING_RULE: &str = "Within any group: observed raw public balance descending, then token account address ascending. Fully deterministic and independent of execution outcomes.";
pub const BUCKET_METHOD: &str = "Rank quartiles over the positive-balance accounts observed in this exact capture, ordered by raw balance ascending then address ascending. Buckets order tests; they are not economic classes and carry no valuation.";

pub fn eligibility_key(e: &Eligibility) -> String {
    serde_json::to_value(e)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{e:?}"))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BucketBoundary {
    pub bucket: u8,
    pub entities: usize,
    pub min_raw: String,
    pub max_raw: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BalanceBuckets {
    pub method: String,
    pub bucket_count: u8,
    pub population: usize,
    pub boundaries: Vec<BucketBoundary>,
    pub note: String,
}
pub const BUCKET_NOTE: &str = "Selected across observed balance buckets. These cases are not a statistical sample and not representative holders.";

/// Deterministic quartile assignment over the fresh positive-balance population.
pub fn buckets(
    observation: &PopulationObservation,
) -> Result<(BalanceBuckets, BTreeMap<String, u8>)> {
    let mut ranked: Vec<(u64, &str)> = observation
        .positive_entities()
        .map(|e| Ok((e.balance()?, e.token_account.as_str())))
        .collect::<Result<Vec<_>>>()?;
    ranked.sort();
    let n = ranked.len();
    let mut assignment = BTreeMap::new();
    let mut grouped: BTreeMap<u8, Vec<u64>> = BTreeMap::new();
    // The loop body only runs when the population is nonempty, so n > 0 here.
    for (rank, (balance, address)) in ranked.iter().enumerate() {
        let bucket = ((rank * 4) / n) as u8;
        assignment.insert((*address).to_string(), bucket);
        grouped.entry(bucket).or_default().push(*balance);
    }
    let boundaries = grouped
        .into_iter()
        .map(|(bucket, values)| BucketBoundary {
            bucket,
            entities: values.len(),
            min_raw: values.first().copied().unwrap_or(0).to_string(),
            max_raw: values.last().copied().unwrap_or(0).to_string(),
        })
        .collect();
    Ok((
        BalanceBuckets {
            method: BUCKET_METHOD.into(),
            bucket_count: 4,
            population: n,
            boundaries,
            note: BUCKET_NOTE.into(),
        },
        assignment,
    ))
}

/// One classified positive-balance entity, before selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Classification {
    pub entity_id: String,
    pub token_account: String,
    pub state_shape_sha256: String,
    pub eligibility: Eligibility,
    pub balance_bucket: u8,
    pub balance_raw: String,
}

struct Classified<'a> {
    entity: &'a StressEntity,
    balance: u64,
    bucket: u8,
    shape: String,
    dimensions: ShapeDimensions,
    eligibility: Eligibility,
    reason: String,
}

fn classify_all<'a>(
    observation: &'a PopulationObservation,
    assignment: &BTreeMap<String, u8>,
) -> Result<Vec<Classified<'a>>> {
    let mint = observation
        .mint_config
        .as_ref()
        .context("population has no decoded mint configuration to classify against")?;
    observation
        .positive_entities()
        .map(|entity| {
            let dimensions = classify::dimensions(entity, mint)?;
            let (eligibility, reason) = classify::eligibility(&dimensions);
            Ok(Classified {
                entity,
                balance: entity.balance()?,
                bucket: *assignment.get(&entity.token_account).unwrap_or(&0),
                shape: classify::shape_key(&dimensions)?,
                dimensions,
                eligibility,
                reason,
            })
        })
        .collect()
}

/// Deterministic digest over the complete classification of every positive
/// entity. Binding this into the plan pins all candidate classifications exactly
/// without publishing a multi-million-row listing; replay recomputes and compares.
fn classification_digest(
    rows: &[Classified<'_>],
    selector_version: &str,
) -> Result<(String, Vec<Classification>)> {
    let mut full: Vec<Classification> = rows
        .iter()
        .map(|c| Classification {
            entity_id: c.entity.entity_id.clone(),
            token_account: c.entity.token_account.clone(),
            state_shape_sha256: c.shape.clone(),
            eligibility: c.eligibility,
            balance_bucket: c.bucket,
            balance_raw: c.balance.to_string(),
        })
        .collect();
    full.sort_by(|a, b| a.token_account.cmp(&b.token_account));
    Ok((
        digest(&(selector_version, classify::CLASSIFIER_VERSION, &full))?,
        full,
    ))
}

pub(crate) fn case_plan(base: &CandidatePlan, entity: &StressEntity) -> CandidatePlan {
    let mut plan = base.clone();
    plan.source_account = Some(entity.token_account.clone());
    plan
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StressTestPlan {
    pub schema_version: u32,
    pub kind: String,
    pub stress_id: String,
    pub run_id: String,
    pub asset_mint: String,
    pub population_capture_sha256: String,
    pub enumeration_completeness: EnumerationCompleteness,
    pub authority_resolution_completeness: AuthorityResolutionCompleteness,
    pub candidate_plan: CandidatePlan,
    pub candidate_plan_sha256: String,
    pub candidate_program_sha256: String,
    pub classifier_version: String,
    pub selector_version: String,
    pub ordering_rule: String,
    pub selection_strategy: Vec<String>,
    pub budget: StressBudget,
    pub buckets: BalanceBuckets,
    /// Pins every candidate classification exactly; recomputed during replay.
    pub classification_sha256: String,
    pub positive_balance_entities: usize,
    pub state_shapes: Vec<ShapeGroup>,
    pub eligibility_counts: BTreeMap<String, usize>,
    pub selected: Vec<SelectedCase>,
    pub frozen_at: String,
    pub limitations: Vec<String>,
}

impl StressTestPlan {
    pub fn sha256(&self) -> Result<String> {
        digest(self)
    }
    /// Write the frozen plan in its canonical form, so the digest of the file on
    /// disk is exactly [`Self::sha256`]. The service hashes the file and the
    /// engine hashes the structure; both must arrive at the same value or the
    /// frozen-plan binding would mean two different things.
    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = canonical(self)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(bytes.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }
    /// Re-derive the whole plan from the population capture and assert it is
    /// unchanged. Expected classifications, selected cases and selection reasons
    /// cannot be rewritten after execution and still pass this.
    pub fn validate(
        &self,
        observation: &PopulationObservation,
        candidate_plan: &CandidatePlan,
        candidate_program_sha256: &str,
    ) -> Result<()> {
        let rebuilt = build(
            observation,
            candidate_plan,
            candidate_program_sha256,
            &self.frozen_at,
        )?;
        ensure!(
            *self == rebuilt,
            "the stress plan differs from the deterministic pre-execution selection over its population capture"
        );
        Ok(())
    }
}

pub const SELECTION_STRATEGY: [&str; 3] = [
    "1. State-shape coverage: one executable case for each distinct discovered state shape, shape key ascending.",
    "2. Balance-bucket coverage: one executable case for each observed balance bucket not yet covered anywhere, bucket ascending.",
    "3. Remaining budget: highest observed public balance first.",
];

/// Build the immutable plan. Deterministic in its inputs and blind to results.
pub fn build(
    observation: &PopulationObservation,
    candidate_plan: &CandidatePlan,
    candidate_program_sha256: &str,
    frozen_at: &str,
) -> Result<StressTestPlan> {
    let rebound = observation.schema_version == 2;
    let selector_version = if rebound {
        SELECTOR_VERSION_V2
    } else {
        SELECTOR_VERSION
    };
    candidate_plan.validate()?;
    ensure!(observation.schema_version == 2, "coherent final rebinding requires population schema 2; archived captures cannot inherit proof");
    observation.budget.validate()?;
    ensure!(
        candidate_plan.source_mint == observation.mint,
        "the candidate plan's source mint is not the mint of this population capture"
    );
    ensure!(
        candidate_program_sha256.len() == 64,
        "a stress plan must pin the candidate program build"
    );
    chrono::DateTime::parse_from_rfc3339(frozen_at).context("invalid plan freeze timestamp")?;
    let decimals = observation
        .mint_config
        .as_ref()
        .context("a stress plan requires a decoded current mint")?
        .decimals;
    let (buckets, assignment) = buckets(observation)?;
    let rows = classify_all(observation, &assignment)?;
    let (classification_sha256, _) = classification_digest(&rows, selector_version)?;

    let mut eligibility_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut shape_members: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        *eligibility_counts
            .entry(eligibility_key(&row.eligibility))
            .or_insert(0) += 1;
        shape_members
            .entry(row.shape.clone())
            .or_default()
            .push(index);
    }
    // Deterministic ordering inside every group, independent of provider order.
    let order = |a: &Classified<'_>, b: &Classified<'_>| {
        b.balance
            .cmp(&a.balance)
            .then_with(|| a.entity.token_account.cmp(&b.entity.token_account))
    };
    for members in shape_members.values_mut() {
        members.sort_by(|x, y| order(&rows[*x], &rows[*y]));
    }

    // ---- Selection. Three phases, executed before any capture or execution. ----
    let budget = observation.budget.max_selected_cases;
    let mut taken: BTreeSet<usize> = BTreeSet::new();
    let mut covered_buckets: BTreeSet<u8> = BTreeSet::new();
    let mut picks: Vec<(usize, SelectionReason, String)> = vec![];

    for (shape, members) in &shape_members {
        if picks.len() >= budget {
            break;
        }
        let Some(&index) = members
            .iter()
            .find(|i| rows[**i].eligibility == Eligibility::ExecutableCandidate)
        else {
            continue;
        };
        taken.insert(index);
        covered_buckets.insert(rows[index].bucket);
        picks.push((
            index,
            SelectionReason::NewStateShape,
            format!(
                "First executable case for state shape {shape}, which covers {} observed positive-balance accounts.",
                members.len()
            ),
        ));
    }

    let mut global: Vec<usize> = (0..rows.len())
        .filter(|i| rows[*i].eligibility == Eligibility::ExecutableCandidate)
        .collect();
    global.sort_by(|x, y| order(&rows[*x], &rows[*y]));

    for bucket in 0..buckets.bucket_count {
        if picks.len() >= budget {
            break;
        }
        if covered_buckets.contains(&bucket) {
            continue;
        }
        let Some(&index) = global
            .iter()
            .find(|i| !taken.contains(i) && rows[**i].bucket == bucket)
        else {
            continue;
        };
        taken.insert(index);
        covered_buckets.insert(bucket);
        picks.push((
            index,
            SelectionReason::NewBalanceBucket,
            format!("First executable case in observed balance bucket {bucket}, which no earlier case covered."),
        ));
    }

    for &index in &global {
        if picks.len() >= budget {
            break;
        }
        if taken.contains(&index) {
            continue;
        }
        taken.insert(index);
        picks.push((
            index,
            SelectionReason::HighestRemainingBalance,
            "Remaining stress budget, highest observed public balance first.".into(),
        ));
    }

    let mut selected = Vec::with_capacity(picks.len());
    for (order_index, (index, reason, detail)) in picks.into_iter().enumerate() {
        let row = &rows[index];
        let amount_decimal = decode::decimal_amount(row.balance, decimals);
        let plan = case_plan(candidate_plan, row.entity);
        selected.push(SelectedCase {
            case_id: format!("case-{order_index:02}"),
            selection_order: order_index,
            selection_reason: reason,
            selection_detail: detail,
            entity_id: row.entity.entity_id.clone(),
            token_account: row.entity.token_account.clone(),
            authority: row.entity.authority.clone(),
            authority_model: row.entity.authority_model.clone(),
            state_shape_sha256: row.shape.clone(),
            shape_label: classify::shape_label(&row.dimensions),
            balance_bucket: row.bucket,
            observed_balance_raw: row.balance.to_string(),
            discovery_slot: row.entity.token_account_evidence.slot,
            selected_amount_raw: row.balance.to_string(),
            selected_amount_decimal: amount_decimal,
            amount_policy: if rebound {
                FULL_AT_FINAL_POLICY
            } else {
                FULL_BALANCE_POLICY
            }
            .into(),
            amount_capped: false,
            case_plan_sha256: plan.sha256()?,
            case_plan: plan,
        });
    }

    let chosen: BTreeMap<&str, &SelectedCase> = selected
        .iter()
        .map(|c| (c.state_shape_sha256.as_str(), c))
        .fold(BTreeMap::new(), |mut m, (k, v)| {
            m.entry(k).or_insert(v);
            m
        });
    let _ = chosen;
    let mut state_shapes = Vec::with_capacity(shape_members.len());
    for (shape, members) in &shape_members {
        let first = &rows[members[0]];
        let mut balance_buckets: BTreeMap<u8, usize> = BTreeMap::new();
        let mut represented: BTreeMap<String, u64> = BTreeMap::new();
        for &i in members {
            *balance_buckets.entry(rows[i].bucket).or_insert(0) += 1;
            represented.insert(rows[i].entity.entity_id.clone(), rows[i].balance);
        }
        let selected_entity_ids: Vec<String> = selected
            .iter()
            .filter(|c| c.state_shape_sha256 == *shape)
            .map(|c| c.entity_id.clone())
            .collect();
        state_shapes.push(ShapeGroup {
            state_shape_sha256: shape.clone(),
            shape_label: classify::shape_label(&first.dimensions),
            dimensions: first.dimensions.clone(),
            eligibility: first.eligibility,
            eligibility_reason: first.reason.clone(),
            entities_in_shape: members.len(),
            represented_raw: super::population_types::sum_once(&represented),
            balance_buckets,
            highest_balance_entities: members
                .iter()
                .take(MAX_SHAPE_EXAMPLES)
                .map(|i| rows[*i].entity.token_account.clone())
                .collect(),
            entities_selected: selected_entity_ids.len(),
            selected_entity_ids,
        });
    }

    let mut limitations = vec![
        if rebound {
            "This plan freezes selected identities, discovery observations and FullAtFinalCapture amount policy before results. Each positive final amount is resolved and bound before its one local VM execution.".to_string()
        } else {
            "This plan was frozen before any capture or execution. Selected cases, amounts and expected classifications may not be rewritten afterwards.".to_string()
        },
        "A state shape prioritizes and describes tests. It is never a proof equivalence class: no tested entity establishes anything about its peers.".to_string(),
        "Balance buckets order tests across the observed range. They are not economic classes and the selected cases are not a representative sample.".to_string(),
        "Only the registered repository candidate mechanism is executed, always locally, against a freshly captured bank plus an explicitly proposed rollout overlay.".to_string(),
    ];
    if observation.enumeration.completeness != EnumerationCompleteness::CompleteForQuery {
        limitations.push("Population discovery is incomplete for this capture, so this plan covers only the trustworthy observed subset.".into());
    }
    if observation.authority_resolution.completeness != AuthorityResolutionCompleteness::Complete {
        limitations.push("Some authority models were not resolved. Those accounts are outside the executable candidate set and receive no assumed signer.".into());
    }

    Ok(StressTestPlan {
        schema_version: if rebound { 2 } else { 1 },
        kind: PLAN_KIND.into(),
        stress_id: observation.stress_id.clone(),
        run_id: observation.run_id.clone(),
        asset_mint: observation.mint.clone(),
        population_capture_sha256: observation.capture_sha256.clone(),
        enumeration_completeness: observation.enumeration.completeness,
        authority_resolution_completeness: observation.authority_resolution.completeness,
        candidate_plan_sha256: candidate_plan.sha256()?,
        candidate_plan: candidate_plan.clone(),
        candidate_program_sha256: candidate_program_sha256.into(),
        classifier_version: classify::CLASSIFIER_VERSION.into(),
        selector_version: selector_version.into(),
        ordering_rule: ORDERING_RULE.into(),
        selection_strategy: SELECTION_STRATEGY.iter().map(|s| s.to_string()).collect(),
        budget: observation.budget.clone(),
        buckets,
        classification_sha256,
        positive_balance_entities: rows.len(),
        state_shapes,
        eligibility_counts,
        selected,
        frozen_at: frozen_at.into(),
        limitations,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Eligibility {
    ExecutableCandidate,
    CaptureRequired,
    Unsupported,
    Invalid,
}

/// Frozen execution intent references the MAIN proposal and state; it contains no executable bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatePlan {
    pub change_spec_id: String,
    pub analysis_input_sha256: String,
    pub source_mint: String,
    pub source_account: Option<String>,
    pub amount_policy: String,
}
impl CandidatePlan {
    pub fn new(input: &super::input::ValidatedInput) -> Self {
        Self {
            change_spec_id: input.change_spec_id().into(),
            analysis_input_sha256: input.analysis_input_sha256().into(),
            source_mint: input.spec().source.mint.clone(),
            source_account: None,
            amount_policy: FULL_AT_FINAL_POLICY.into(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        for value in [&self.change_spec_id, &self.analysis_input_sha256] {
            ensure!(
                value.len() == 64
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "invalid migration intent digest"
            );
        }
        let _: solana_address::Address = self.source_mint.parse()?;
        if let Some(address) = &self.source_account {
            let _: solana_address::Address = address.parse()?;
        }
        ensure!(
            self.amount_policy == FULL_AT_FINAL_POLICY,
            "amount policy changed"
        );
        Ok(())
    }
    pub fn sha256(&self) -> Result<String> {
        digest(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionReason {
    /// First executable case for a distinct discovered state shape.
    NewStateShape,
    /// First executable case in a balance bucket not yet covered anywhere.
    NewBalanceBucket,
    /// Remaining budget, highest observed public balance first.
    HighestRemainingBalance,
}

/// One exact test case, frozen before capture and execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedCase {
    pub case_id: String,
    pub selection_order: usize,
    pub selection_reason: SelectionReason,
    pub selection_detail: String,
    pub entity_id: String,
    pub token_account: String,
    pub authority: String,
    pub authority_model: EntityType,
    pub state_shape_sha256: String,
    pub shape_label: String,
    pub balance_bucket: u8,
    pub observed_balance_raw: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub discovery_slot: u64,
    pub selected_amount_raw: String,
    pub selected_amount_decimal: String,
    pub amount_policy: String,
    /// Explicit: this milestone never silently caps a selected amount.
    pub amount_capped: bool,
    /// The operator's plan with this exact source account and amount. Same terms,
    /// same mechanism, same replacement asset; a distinct digest per case.
    pub case_plan: CandidatePlan,
    pub case_plan_sha256: String,
}
pub const FULL_BALANCE_POLICY: &str = "FullObservedPublicBalance: the entire public balance observed for this exact account in the frozen population capture";
pub const FULL_AT_FINAL_POLICY: &str = "FullAtFinalCapture: freeze the selected account and policy before execution, then use its entire positive public balance from the verified final account batch";

/// A group of entities sharing execution-relevant characteristics.
///
/// Deliberately carries no status: a shape is exercised, never proven. Execution
/// outcomes live on exact cases and are joined into a separate coverage view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeGroup {
    pub state_shape_sha256: String,
    pub shape_label: String,
    pub dimensions: classify::ShapeDimensions,
    pub eligibility: Eligibility,
    pub eligibility_reason: String,
    pub entities_in_shape: usize,
    pub represented_raw: String,
    pub balance_buckets: BTreeMap<u8, usize>,
    /// Bounded example listing, highest observed balance first. Not a selection.
    pub highest_balance_entities: Vec<String>,
    pub entities_selected: usize,
    pub selected_entity_ids: Vec<String>,
}
pub const MAX_SHAPE_EXAMPLES: usize = 10;
