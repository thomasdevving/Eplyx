//! Deterministic selection of a production-derived regression corpus.
//!
//! Turns validated historical replay records into a small, diverse set suitable
//! for candidate V1/V2 analysis. It optimizes for regression-test usefulness,
//! not for reproducing production's traffic distribution - and it says so,
//! because those are different claims and only one of them is true here.
//!
//! # Three populations, never collapsed
//!
//! * `observed`        - interactions discovery saw on mainnet
//! * `replay_eligible` - those the exact historical contract can replay
//! * `selected`        - those this policy chose
//!
//! A selected corpus may deliberately oversample a rare-but-replayable action.
//! Reporting all three side by side is what stops "we could not replay it" from
//! being read as "it does not happen".
//!
//! # Determinism
//!
//! Records are canonicalised by observation ID before anything else, so input
//! order and concurrency cannot reach the result. Every score component is an
//! integer, every tie breaks on ID, and the canonical output carries no clock,
//! endpoint or duration.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::protocol::{adapter_for, SemanticAction};
use crate::replay::{hash_bytes, ReplayRecord};

/// Bumped whenever selection rules change meaning. Recorded in the manifest so
/// a corpus built under different rules cannot look identical to an older one.
pub const SELECTION_POLICY_VERSION: u32 = 1;
pub const SELECTED_CORPUS_SCHEMA: u32 = 1;

/// Why one observation was chosen. Derived from rules, never from prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionReason {
    /// First record representing its semantic action.
    SemanticActionCoverage,
    NewEconomicEntity,
    NewPool,
    /// Sits close to a protocol-defined economic threshold.
    BoundaryProximity,
    /// Largest or smallest amount in its action.
    AmountTail,
    /// Extends the time span the corpus covers.
    TemporalDiversity,
    /// An interaction shape no selected record had yet.
    StructuralNovelty,
    /// Compute consumption unlike the rest of its action.
    ComputeOutlier,
    /// Ordinary traffic, kept so the corpus is not only edge cases.
    CommonPath,
}

impl SelectionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SemanticActionCoverage => "semantic_action_coverage",
            Self::NewEconomicEntity => "new_economic_entity",
            Self::NewPool => "new_pool",
            Self::BoundaryProximity => "boundary_proximity",
            Self::AmountTail => "amount_tail",
            Self::TemporalDiversity => "temporal_diversity",
            Self::StructuralNovelty => "structural_novelty",
            Self::ComputeOutlier => "compute_outlier",
            Self::CommonPath => "common_path",
        }
    }
}

/// Integer score components. No floating point: these feed a canonical hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionScore {
    /// Higher for an action with fewer replayable observations.
    pub semantic_rarity: u32,
    /// Closeness to a protocol boundary, in basis points inverted.
    pub boundary_proximity: u32,
    /// Distance from its action's median amount, in basis points.
    pub amount_tail: u32,
    /// Compute distance from its action's median, in basis points.
    pub compute_outlier: u32,
    pub total: u32,
}

/// One chosen observation, with everything needed to justify it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedObservation {
    pub id: String,
    pub signature: String,
    pub slot: u64,
    pub semantic_action: String,
    pub economic_entity: Option<String>,
    pub pool: Option<String>,
    pub amount: Option<u64>,
    pub compute_units: Option<u64>,
    pub selection_reasons: Vec<SelectionReason>,
    pub score: SelectionScore,
}

/// Counts for one semantic action across the three populations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPopulation {
    pub semantic_action: String,
    /// `None` when discovery counts were not supplied for this run: an absent
    /// measurement is reported as absent, never as zero.
    pub observed: Option<usize>,
    pub replay_eligible: usize,
    pub selected: usize,
}

/// A structured statement about what this corpus cannot cover.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageLimitation {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortfall {
    pub requested: usize,
    pub available: usize,
    pub selected: usize,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedCorpus {
    pub schema_version: u32,
    pub program_id: String,
    pub protocol: Option<String>,
    pub adapter_version: Option<u32>,
    pub selection_policy: String,
    pub selection_policy_version: u32,
    pub target_size: usize,
    pub populations: Vec<ActionPopulation>,
    pub selected: Vec<SelectedObservation>,
    pub limitations: Vec<CoverageLimitation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortfall: Option<Shortfall>,
    pub selected_corpus_sha256: String,
}

impl SelectedCorpus {
    pub fn unique_entities(&self) -> usize {
        self.selected
            .iter()
            .filter_map(|s| s.economic_entity.as_ref())
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn unique_pools(&self) -> usize {
        self.selected
            .iter()
            .filter_map(|s| s.pool.as_ref())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// Everything the selector needs about one record, derived once.
struct Candidate {
    id: String,
    signature: String,
    slot: u64,
    action: SemanticAction,
    entity: Option<String>,
    pool: Option<String>,
    amount: Option<u64>,
    compute: Option<u64>,
    /// Smallest boundary distance the adapter reports, in basis points.
    boundary_bps: Option<u32>,
    /// Shape fingerprint: which account labels the record touches.
    shape: String,
}

fn describe(record: &ReplayRecord) -> Candidate {
    let adapter = adapter_for(&record.program_id);
    let action = adapter
        .map(|a| a.semantic_action(&record.transaction))
        .unwrap_or(SemanticAction::Unknown);
    let entity = adapter
        .and_then(|a| a.economic_entity_id(&record.transaction, &record.accounts))
        .map(|e| e.id);
    let features: BTreeMap<String, u128> = adapter
        .map(|a| a.state_features(&record.transaction, &record.accounts))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| f.value.as_integer().map(|v| (f.name, v)))
        .collect();
    let amount = features
        .get("deposit_lamports")
        .or_else(|| features.get("pool_tokens_burned"))
        .and_then(|v| u64::try_from(*v).ok());
    let boundary_bps = adapter
        .map(|a| a.boundaries(&record.transaction, &record.accounts))
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.distance_bps)
        .min();
    let pool = record
        .accounts
        .iter()
        .find(|a| a.label == "stake-pool")
        .map(|a| a.address.clone());
    let mut labels: Vec<&str> = record.accounts.iter().map(|a| a.label.as_str()).collect();
    labels.sort_unstable();
    // The invocation signature is part of the interaction's shape, not a
    // separate dimension. A referral deposit mints twice where an ordinary one
    // mints once; both name the same accounts, so labels alone report them as
    // the same structure and a corpus can cover every label set while leaving
    // the second-mint path untested. The signature is archive evidence the
    // record already carries, so reading it here adds no replay requirement.
    // A record with no original outcome has an unknown signature, which is not
    // the same structure as one observed to invoke nothing: the two must not
    // share a shape key.
    let invocations = match &record.original {
        Some(original) => original
            .cpi_invocations
            .iter()
            .map(|c| {
                let discriminant = c
                    .discriminant
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".to_string());
                format!("{}:{}:{}", c.program, discriminant, c.account_count)
            })
            .collect::<Vec<_>>()
            .join(","),
        None => "unknown".to_string(),
    };
    Candidate {
        id: record.id.clone(),
        signature: record.transaction.signature.clone(),
        slot: record.transaction.slot,
        action,
        entity,
        pool,
        amount,
        compute: record.transaction.compute_units,
        boundary_bps,
        shape: format!("{}|{}", labels.join("+"), invocations),
    }
}

/// Distance from the median, in basis points, saturating at 10_000.
///
/// Capped so every score component shares one scale. Uncapped, a whale deposit
/// 800x the median scores in the millions and silently becomes the only
/// component that matters, which would make the other dimensions decorative.
/// Anything at or beyond twice the median is already maximally tail.
fn tail_bps(value: u64, median: u64) -> u32 {
    if median == 0 {
        return 0;
    }
    let gap = value.abs_diff(median) as u128;
    u32::try_from(gap.saturating_mul(10_000) / median as u128)
        .unwrap_or(u32::MAX)
        .min(10_000)
}

fn median(mut values: Vec<u64>) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

fn score(
    candidate: &Candidate,
    eligible_for_action: usize,
    total: usize,
    medians: (u64, u64),
) -> SelectionScore {
    // Rarer actions score higher, so a small replayable population is not
    // drowned out by a large one.
    let semantic_rarity = if eligible_for_action == 0 {
        0
    } else {
        u32::try_from(10_000 * total as u128 / (eligible_for_action as u128 * total.max(1) as u128))
            .unwrap_or(0)
            .min(10_000)
    };
    let boundary_proximity = candidate
        .boundary_bps
        .map(|bps| 10_000_u32.saturating_sub(bps.min(10_000)))
        .unwrap_or(0);
    let amount_tail = candidate
        .amount
        .map(|a| tail_bps(a, medians.0))
        .unwrap_or(0);
    let compute_outlier = candidate
        .compute
        .map(|c| tail_bps(c, medians.1))
        .unwrap_or(0);
    SelectionScore {
        semantic_rarity,
        boundary_proximity,
        amount_tail,
        compute_outlier,
        total: semantic_rarity
            .saturating_add(boundary_proximity)
            .saturating_add(amount_tail)
            .saturating_add(compute_outlier),
    }
}

/// Discovery counts, when a run has them. Keyed by semantic action.
pub type ObservedCounts = BTreeMap<String, usize>;

/// Select a corpus from validated replay records.
///
/// Stratified rather than top-N: sorting every record by one scalar and taking
/// the first N collapses diversity, which is the property a regression corpus
/// most needs. Each round claims records for a specific reason, and only the
/// remainder is filled by score.
pub fn select(
    records: &[ReplayRecord],
    target_size: usize,
    observed: &ObservedCounts,
) -> Result<SelectedCorpus> {
    anyhow::ensure!(target_size > 0, "--target-size must be greater than zero");

    // Canonical order first: everything downstream is order-independent because
    // the input is normalised here, not because callers are careful.
    let mut candidates: Vec<Candidate> = records.iter().map(describe).collect();
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    anyhow::ensure!(
        candidates.windows(2).all(|w| w[0].id != w[1].id),
        "duplicate observation id in the eligible set"
    );

    let total = candidates.len();
    let mut per_action: BTreeMap<String, usize> = BTreeMap::new();
    for candidate in &candidates {
        *per_action
            .entry(candidate.action.as_str().to_string())
            .or_default() += 1;
    }
    let amount_median = median(candidates.iter().filter_map(|c| c.amount).collect());
    let compute_median = median(candidates.iter().filter_map(|c| c.compute).collect());
    let scores: BTreeMap<&str, SelectionScore> = candidates
        .iter()
        .map(|c| {
            let n = per_action
                .get(c.action.as_str())
                .copied()
                .unwrap_or(total.max(1));
            (
                c.id.as_str(),
                score(c, n, total, (amount_median, compute_median)),
            )
        })
        .collect();

    let mut chosen: BTreeMap<String, Vec<SelectionReason>> = BTreeMap::new();
    let mut entities: BTreeSet<String> = BTreeSet::new();
    let mut pools: BTreeSet<String> = BTreeSet::new();
    let mut shapes: BTreeSet<String> = BTreeSet::new();

    // Ordering within a round: score descending, then id. Both are total, so
    // the result cannot depend on how the vector happened to be arranged.
    let ranked = |pool: &[&Candidate]| -> Vec<String> {
        let mut v: Vec<&&Candidate> = pool.iter().collect();
        v.sort_by(|a, b| {
            scores[b.id.as_str()]
                .total
                .cmp(&scores[a.id.as_str()].total)
                .then(a.id.cmp(&b.id))
        });
        v.into_iter().map(|c| c.id.clone()).collect()
    };
    let by_id: BTreeMap<&str, &Candidate> = candidates.iter().map(|c| (c.id.as_str(), c)).collect();

    let take = |id: &str,
                reason: SelectionReason,
                chosen: &mut BTreeMap<String, Vec<SelectionReason>>,
                entities: &mut BTreeSet<String>,
                pools: &mut BTreeSet<String>,
                shapes: &mut BTreeSet<String>| {
        let candidate = by_id[id];
        let mut reasons = vec![reason];
        // Credit every kind of novelty this record actually adds, not only the
        // round that happened to claim it. A record chosen for its entity that
        // also brings a new pool and sits near a boundary should say so.
        if candidate
            .entity
            .as_ref()
            .is_some_and(|e| !entities.contains(e))
        {
            reasons.push(SelectionReason::NewEconomicEntity);
        }
        if candidate.pool.as_ref().is_some_and(|p| !pools.contains(p)) {
            reasons.push(SelectionReason::NewPool);
        }
        if candidate.boundary_bps.is_some_and(|b| b <= 2_000) {
            reasons.push(SelectionReason::BoundaryProximity);
        }
        if !shapes.contains(&candidate.shape) {
            reasons.push(SelectionReason::StructuralNovelty);
        }
        // A record at or past twice its action's median amount is a value tail.
        if scores[candidate.id.as_str()].amount_tail >= 10_000 {
            reasons.push(SelectionReason::AmountTail);
        }
        if scores[candidate.id.as_str()].compute_outlier >= 10_000 {
            reasons.push(SelectionReason::ComputeOutlier);
        }
        let entry = chosen.entry(id.to_string()).or_default();
        for reason in reasons {
            if !entry.contains(&reason) {
                entry.push(reason);
            }
        }
        entry.sort_unstable();
        entry.dedup();
        if let Some(e) = &candidate.entity {
            entities.insert(e.clone());
        }
        if let Some(p) = &candidate.pool {
            pools.insert(p.clone());
        }
        shapes.insert(candidate.shape.clone());
    };

    // 1. Every semantic action that has a replayable observation gets one.
    for action in per_action.keys() {
        if chosen.len() >= target_size {
            break;
        }
        let pool: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| c.action.as_str() == action)
            .collect();
        if let Some(id) = ranked(&pool).into_iter().next() {
            take(
                &id,
                SelectionReason::SemanticActionCoverage,
                &mut chosen,
                &mut entities,
                &mut pools,
                &mut shapes,
            );
        }
    }

    // 2. Unique economic entities: a hundred interactions with one position are
    //    not a hundred independent exposures.
    //
    //    Ranked so that, among records bringing a new entity, one that also
    //    brings a new pool or sits near a boundary is taken first. With more
    //    entities than slots this round would otherwise consume the whole
    //    target and starve every dimension after it.
    loop {
        if chosen.len() >= target_size {
            break;
        }
        let mut best: Option<(u32, u32, String)> = None;
        for candidate in &candidates {
            if chosen.contains_key(&candidate.id) {
                continue;
            }
            if !candidate
                .entity
                .as_ref()
                .is_some_and(|e| !entities.contains(e))
            {
                continue;
            }
            let compounding =
                u32::from(candidate.pool.as_ref().is_some_and(|p| !pools.contains(p)))
                    + u32::from(candidate.boundary_bps.is_some_and(|b| b <= 2_000))
                    + u32::from(!shapes.contains(&candidate.shape));
            let key = (
                compounding,
                scores[candidate.id.as_str()].total,
                candidate.id.clone(),
            );
            if best.as_ref().is_none_or(|current| {
                (key.0, key.1) > (current.0, current.1)
                    || ((key.0, key.1) == (current.0, current.1) && key.2 < current.2)
            }) {
                best = Some(key);
            }
        }
        let Some((_, _, id)) = best else { break };
        take(
            &id,
            SelectionReason::NewEconomicEntity,
            &mut chosen,
            &mut entities,
            &mut pools,
            &mut shapes,
        );
    }

    // 3. Unique pools.
    for id in ranked(&candidates.iter().collect::<Vec<_>>()) {
        if chosen.len() >= target_size {
            break;
        }
        if chosen.contains_key(&id) {
            continue;
        }
        if by_id[id.as_str()]
            .pool
            .as_ref()
            .is_some_and(|p| !pools.contains(p))
        {
            take(
                &id,
                SelectionReason::NewPool,
                &mut chosen,
                &mut entities,
                &mut pools,
                &mut shapes,
            );
        }
    }

    // 4. Boundary states: closest to a protocol-defined threshold.
    let mut near: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.boundary_bps.is_some_and(|b| b <= 2_000))
        .collect();
    near.sort_by_key(|c| (c.boundary_bps.unwrap_or(u32::MAX), c.id.clone()));
    for candidate in near {
        if chosen.len() >= target_size {
            break;
        }
        if !chosen.contains_key(&candidate.id) {
            take(
                &candidate.id.clone(),
                SelectionReason::BoundaryProximity,
                &mut chosen,
                &mut entities,
                &mut pools,
                &mut shapes,
            );
        }
    }

    // 5. Amount tails, per action, so a large action cannot own both ends.
    for action in per_action.keys() {
        let mut pool: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| c.action.as_str() == action && c.amount.is_some())
            .collect();
        pool.sort_by_key(|c| (c.amount.unwrap_or(0), c.id.clone()));
        for candidate in [pool.first(), pool.last()].into_iter().flatten() {
            if chosen.len() >= target_size {
                break;
            }
            if !chosen.contains_key(&candidate.id) {
                take(
                    &candidate.id.clone(),
                    SelectionReason::AmountTail,
                    &mut chosen,
                    &mut entities,
                    &mut pools,
                    &mut shapes,
                );
            }
        }
    }

    // 6. Structural novelty: an interaction shape nothing selected covers yet.
    for id in ranked(&candidates.iter().collect::<Vec<_>>()) {
        if chosen.len() >= target_size {
            break;
        }
        if chosen.contains_key(&id) {
            continue;
        }
        if !shapes.contains(&by_id[id.as_str()].shape) {
            take(
                &id,
                SelectionReason::StructuralNovelty,
                &mut chosen,
                &mut entities,
                &mut pools,
                &mut shapes,
            );
        }
    }

    // 7. Temporal spread: widen the window the corpus covers.
    let mut by_slot: Vec<&Candidate> = candidates.iter().collect();
    by_slot.sort_by_key(|c| (c.slot, c.id.clone()));
    for candidate in [by_slot.first(), by_slot.last()].into_iter().flatten() {
        if chosen.len() >= target_size {
            break;
        }
        if !chosen.contains_key(&candidate.id) {
            take(
                &candidate.id.clone(),
                SelectionReason::TemporalDiversity,
                &mut chosen,
                &mut entities,
                &mut pools,
                &mut shapes,
            );
        }
    }

    // 8. Remaining slots: ordinary traffic, by score then id.
    for id in ranked(&candidates.iter().collect::<Vec<_>>()) {
        if chosen.len() >= target_size {
            break;
        }
        if !chosen.contains_key(&id) {
            take(
                &id,
                SelectionReason::CommonPath,
                &mut chosen,
                &mut entities,
                &mut pools,
                &mut shapes,
            );
        }
    }

    let mut selected: Vec<SelectedObservation> = chosen
        .iter()
        .map(|(id, reasons)| {
            let c = by_id[id.as_str()];
            SelectedObservation {
                id: c.id.clone(),
                signature: c.signature.clone(),
                slot: c.slot,
                semantic_action: c.action.as_str().to_string(),
                economic_entity: c.entity.clone(),
                pool: c.pool.clone(),
                amount: c.amount,
                compute_units: c.compute,
                selection_reasons: reasons.clone(),
                score: scores[id.as_str()],
            }
        })
        .collect();
    selected.sort_by(|a, b| a.id.cmp(&b.id));

    let populations = per_action
        .iter()
        .map(|(action, eligible)| ActionPopulation {
            semantic_action: action.clone(),
            observed: observed.get(action).copied(),
            replay_eligible: *eligible,
            selected: selected
                .iter()
                .filter(|s| &s.semantic_action == action)
                .count(),
        })
        .collect::<Vec<_>>();

    let shortfall = (target_size > total).then(|| Shortfall {
        requested: target_size,
        available: total,
        selected: selected.len(),
        detail: "insufficient validated observations to satisfy the requested target size; \
                 records are never duplicated to fill a target"
            .into(),
    });

    let program_id = records
        .first()
        .map(|r| r.program_id.clone())
        .unwrap_or_default();
    let adapter = adapter_for(&program_id);
    let mut corpus = SelectedCorpus {
        schema_version: SELECTED_CORPUS_SCHEMA,
        program_id,
        protocol: adapter.map(|a| a.name().to_string()),
        adapter_version: adapter.map(|a| a.adapter_version()),
        selection_policy: "stratified-diversity".into(),
        selection_policy_version: SELECTION_POLICY_VERSION,
        target_size,
        populations,
        selected,
        limitations: limitations(observed, &per_action),
        shortfall,
        selected_corpus_sha256: String::new(),
    };
    // Hash the canonical content, with the hash field empty so it cannot
    // depend on itself.
    corpus.selected_corpus_sha256 = hash_bytes(&serde_json::to_vec(&corpus)?);
    Ok(corpus)
}

/// What this corpus cannot speak to. Stated structurally so a consumer can act
/// Limits that hold for every corpus built under this replay contract,
/// whatever was selected from it.
///
/// They travel with a bundle even when no selection ran, because a bundle that
/// has lost them is a bundle that overclaims — and a passing CI report that
/// omits them is the one place a reader most needs to see them.
pub fn contract_limitations() -> Vec<CoverageLimitation> {
    vec![
        CoverageLimitation {
            code: "failed_original_transactions_unsupported".into(),
            detail: "Transactions that failed on mainnet are observed but not replayed, so no \
                     failure path is represented."
                .into(),
        },
        CoverageLimitation {
            code: "account_creation_paths_unsupported".into(),
            detail: "Interactions that create a token account are outside the exact historical \
                     contract and are excluded."
                .into(),
        },
        CoverageLimitation {
            code: "replay_runtime_rent_semantics_unsupported".into(),
            detail: "Transactions crediting an account that stays below its rent-exempt minimum \
                     are accepted by mainnet and refused by the replay runtime, so they are \
                     excluded. Tip accounts are the common case."
                .into(),
        },
        CoverageLimitation {
            code: "address_lookup_tables_unsupported".into(),
            detail: "A v0 message that actually resolves addresses through a lookup table is \
                     outside the replay contract and is excluded, so any interaction shape that \
                     requires one is unrepresented."
                .into(),
        },
    ]
}

/// on it rather than having to read prose.
fn limitations(
    observed: &ObservedCounts,
    eligible: &BTreeMap<String, usize>,
) -> Vec<CoverageLimitation> {
    let mut out = contract_limitations();
    // Iterate the observed side, not the eligible side. An action seen in
    // production with *zero* replayable records is the most severe bias there
    // is, and looping over what survived would skip exactly that case.
    for (action, observed_count) in observed {
        if *observed_count == 0 {
            continue;
        }
        let eligible_count = eligible.get(action).copied().unwrap_or(0);
        if eligible_count == 0 {
            out.push(CoverageLimitation {
                code: format!("{action}_has_no_replayable_observations"),
                detail: format!(
                    "{observed_count} {action} interactions were observed in production and none \
                     are replayable under the exact historical contract. This corpus says nothing \
                     about {action}; that is not the same as {action} being unaffected."
                ),
            });
            continue;
        }
        let eligible_count = &eligible_count;
        let share_bps = (*eligible_count as u128 * 10_000) / *observed_count as u128;
        if share_bps < 2_000 {
            out.push(CoverageLimitation {
                code: format!("{}_replayability_materially_below_observed", action),
                detail: format!(
                    "{eligible_count} of {observed_count} observed {action} interactions are \
                     replayable under the exact historical contract ({}.{:02}%). The corpus \
                     under-represents this action relative to production.",
                    share_bps / 100,
                    share_bps % 100
                ),
            });
        }
    }
    out
}

/// Terminal rendering. Populations are always shown side by side, because the
/// gap between them is the most important thing the report says.
pub fn render(corpus: &SelectedCorpus) -> String {
    let mut out = String::new();
    out.push_str("EPLYX VALIDATED PRODUCTION-DERIVED CORPUS\n");
    out.push_str("=========================================\n\n");
    out.push_str(&format!(
        "protocol:          {}\n",
        corpus.protocol.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!(
        "adapter version:   {}\n",
        corpus.adapter_version.unwrap_or(0)
    ));
    out.push_str(&format!(
        "selection policy:  {} v{}\n\n",
        corpus.selection_policy, corpus.selection_policy_version
    ));

    let eligible: usize = corpus.populations.iter().map(|p| p.replay_eligible).sum();
    out.push_str(&format!("Eligible observations:   {eligible}\n"));
    out.push_str(&format!(
        "Selected observations:   {}\n",
        corpus.selected.len()
    ));
    if let Some(shortfall) = &corpus.shortfall {
        out.push_str(&format!(
            "\nRequested: {}\nAvailable: {}\nSelected:  {}\n\nCoverage limitation:\n  {}\n",
            shortfall.requested, shortfall.available, shortfall.selected, shortfall.detail
        ));
    }

    out.push_str("\nPOPULATIONS  (observed -> replayable -> selected)\n");
    out.push_str(&format!(
        "  {:<16}{:>10}{:>12}{:>10}\n",
        "action", "observed", "replayable", "selected"
    ));
    for p in &corpus.populations {
        out.push_str(&format!(
            "  {:<16}{:>10}{:>12}{:>10}\n",
            p.semantic_action,
            p.observed
                .map(|n| n.to_string())
                .unwrap_or_else(|| "not measured".into()),
            p.replay_eligible,
            p.selected
        ));
    }
    out.push_str(
        "\n  These are three different populations. A selected corpus may deliberately\n  \
         oversample a rare-but-replayable action; it does not reproduce production's\n  \
         traffic distribution.\n",
    );

    out.push_str(&format!(
        "\nDIVERSITY\n  Unique entities        {:>4}\n  Unique pools           {:>4}\n",
        corpus.unique_entities(),
        corpus.unique_pools()
    ));

    out.push_str("\nSELECTED OBSERVATIONS\n");
    for s in &corpus.selected {
        out.push_str(&format!(
            "  {:<14} slot {:<11} {}\n",
            s.semantic_action,
            s.slot,
            &s.signature[..24.min(s.signature.len())]
        ));
        out.push_str(&format!(
            "      reasons: {}\n",
            s.selection_reasons
                .iter()
                .map(|r| r.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    out.push_str("\nCOVERAGE LIMITATIONS\n");
    for limitation in &corpus.limitations {
        out.push_str(&format!(
            "  - {}\n    {}\n",
            limitation.code, limitation.detail
        ));
    }
    out.push_str(&format!(
        "\nCorpus SHA-256:\n  {}\n",
        corpus.selected_corpus_sha256
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real committed mainnet record, varied along one axis at a time.
    ///
    /// Addresses are remapped consistently across the message keys, the
    /// instruction metas and the account snapshots, so the adapter derives the
    /// entity and pool the test intends rather than whatever the fixture held.
    fn base() -> ReplayRecord {
        serde_json::from_str(include_str!(
            "../../docs/examples/mainnet-stake-pool-record.json"
        ))
        .expect("committed mainnet record")
    }

    /// The second supported action, so action coverage can be tested rather
    /// than assumed. A deposit and a withdrawal decode through different paths
    /// and name different accounts, and no synthetic edit to one produces the
    /// other.
    fn withdraw() -> ReplayRecord {
        serde_json::from_str(include_str!(
            "../../docs/examples/mainnet-stake-pool-withdraw-record.json"
        ))
        .expect("committed mainnet withdraw record")
    }

    fn address(tag: u8, seed: u8) -> String {
        let mut bytes = [seed; 32];
        bytes[0] = tag;
        solana_address::Address::new_from_array(bytes).to_string()
    }

    fn remap(record: &mut ReplayRecord, label: &str, to: &str) {
        let Some(from) = record
            .accounts
            .iter()
            .find(|a| a.label == label)
            .map(|a| a.address.clone())
        else {
            return;
        };
        for account in &mut record.accounts {
            if account.address == from {
                account.address = to.to_string();
            }
        }
        for key in &mut record.transaction.account_keys {
            if key.address == from {
                key.address = to.to_string();
            }
        }
        for instruction in &mut record.transaction.instructions {
            for meta in &mut instruction.accounts {
                if meta.address == from {
                    meta.address = to.to_string();
                }
            }
        }
    }

    /// `n` records differing in observation id, slot, entity and pool.
    fn population(n: u8) -> Vec<ReplayRecord> {
        (0..n)
            .map(|i| {
                let mut r = base();
                r.id = format!("obs-{i:03}");
                r.transaction.slot = 400_000_000 + u64::from(i);
                r.transaction.signature = format!("sig-{i:03}");
                remap(&mut r, "destination-pool-token", &address(1, i));
                // Two pools across the population, so pool novelty is scarcer
                // than entity novelty and the two can be told apart.
                remap(&mut r, "stake-pool", &address(2, i % 2));
                r
            })
            .collect()
    }

    fn ids(corpus: &SelectedCorpus) -> Vec<String> {
        corpus.selected.iter().map(|s| s.id.clone()).collect()
    }

    #[test]
    fn the_same_input_selects_the_same_observations() {
        let records = population(8);
        let a = select(&records, 4, &Default::default()).unwrap();
        let b = select(&records, 4, &Default::default()).unwrap();
        assert_eq!(ids(&a), ids(&b));
        assert_eq!(a.selected_corpus_sha256, b.selected_corpus_sha256);
    }

    #[test]
    fn input_order_cannot_reach_the_result() {
        let records = population(8);
        let mut shuffled = records.clone();
        shuffled.reverse();
        // A rotation as well, so the test is not only sensitive to reversal.
        let mut rotated = records.clone();
        rotated.rotate_left(3);

        let expected = select(&records, 4, &Default::default()).unwrap();
        for variant in [shuffled, rotated] {
            let actual = select(&variant, 4, &Default::default()).unwrap();
            assert_eq!(ids(&actual), ids(&expected));
            assert_eq!(
                actual.selected_corpus_sha256,
                expected.selected_corpus_sha256
            );
        }
    }

    #[test]
    fn concurrent_construction_produces_the_same_corpus() {
        let records = population(10);
        let expected = select(&records, 5, &Default::default()).unwrap();
        let results: Vec<SelectedCorpus> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|offset| {
                    let mut shuffled = records.clone();
                    shuffled.rotate_left(offset);
                    scope.spawn(move || select(&shuffled, 5, &Default::default()).unwrap())
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for actual in results {
            assert_eq!(actual, expected, "concurrency must not reach the result");
        }
    }

    #[test]
    fn the_canonical_encoding_is_byte_identical_across_runs() {
        let records = population(6);
        let a = serde_json::to_vec(&select(&records, 3, &Default::default()).unwrap()).unwrap();
        let b = serde_json::to_vec(&select(&records, 3, &Default::default()).unwrap()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn an_observation_cannot_be_selected_twice() {
        let corpus = select(&population(12), 12, &Default::default()).unwrap();
        let mut seen = ids(&corpus);
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(before, seen.len());
    }

    #[test]
    fn a_target_larger_than_the_population_reports_a_shortfall() {
        let corpus = select(&population(5), 25, &Default::default()).unwrap();
        let shortfall = corpus.shortfall.expect("shortfall must be reported");
        assert_eq!(shortfall.requested, 25);
        assert_eq!(shortfall.available, 5);
        assert_eq!(shortfall.selected, 5);
        assert_eq!(corpus.selected.len(), 5, "records are never duplicated");
    }

    #[test]
    fn a_target_the_population_can_satisfy_reports_no_shortfall() {
        let corpus = select(&population(5), 5, &Default::default()).unwrap();
        assert!(corpus.shortfall.is_none());
    }

    #[test]
    fn an_invalid_target_fails_cleanly() {
        assert!(select(&population(3), 0, &Default::default()).is_err());
    }

    /// Coverage of an action comes before any amount of depth within one. A
    /// corpus that tests deposits nine ways and never withdraws has not tested
    /// the upgrade.
    #[test]
    fn every_action_with_an_observation_is_represented() {
        let mut records = population(9);
        let mut w = withdraw();
        w.id = "obs-withdraw".to_string();
        records.push(w);

        let actions: BTreeSet<String> = select(&records, 2, &Default::default())
            .unwrap()
            .selected
            .iter()
            .map(|s| s.semantic_action.clone())
            .collect();
        assert_eq!(actions.len(), 2, "one deposit and one withdrawal");

        // And the populations report both, whatever the target.
        let corpus = select(&records, 2, &Default::default()).unwrap();
        assert_eq!(corpus.populations.len(), 2);
        for row in &corpus.populations {
            assert!(row.selected >= 1, "{} unrepresented", row.semantic_action);
        }
    }

    #[test]
    fn entity_diversity_is_preferred() {
        // Ten distinct entities, five slots: every selected record should carry
        // a different one.
        let corpus = select(&population(10), 5, &Default::default()).unwrap();
        assert_eq!(corpus.unique_entities(), 5);
    }

    #[test]
    fn pool_diversity_is_preferred_within_entity_selection() {
        // The population spans two pools; a corpus of two must cover both
        // rather than taking the highest-scoring pair from one.
        let corpus = select(&population(10), 2, &Default::default()).unwrap();
        assert_eq!(corpus.unique_pools(), 2);
    }

    /// The referral-fee deposit in the production corpus mints twice where an
    /// ordinary deposit mints once, while naming the same accounts. It is the
    /// only record of its shape, and it is the only one the regressed candidate
    /// diverges from structurally rather than numerically. A selection that
    /// covers every account label set but not that path would report the
    /// upgrade as a value change when it is also a control-flow change.
    #[test]
    fn a_singleton_invocation_signature_is_covered() {
        let mut records = population(12);
        let referral = &mut records[7];
        let original = referral.original.as_mut().expect("original outcome");
        let extra = original
            .cpi_invocations
            .last()
            .cloned()
            .expect("the record invokes at least once");
        original.cpi_invocations.push(extra);
        let odd_one_out = referral.id.clone();

        let corpus = select(&records, 6, &Default::default()).unwrap();
        assert!(
            ids(&corpus).contains(&odd_one_out),
            "the only record of its invocation shape must be selected"
        );
    }

    #[test]
    fn selection_reasons_are_stable_and_non_empty() {
        let records = population(8);
        let a = select(&records, 4, &Default::default()).unwrap();
        let b = select(&records, 4, &Default::default()).unwrap();
        for (x, y) in a.selected.iter().zip(b.selected.iter()) {
            assert_eq!(x.selection_reasons, y.selection_reasons);
            assert!(
                !x.selection_reasons.is_empty(),
                "every record must justify itself"
            );
        }
    }

    // ---- bias reporting -------------------------------------------------

    #[test]
    fn a_large_observed_to_replayable_gap_is_surfaced() {
        let observed: ObservedCounts = [("deposit".to_string(), 681_usize)].into_iter().collect();
        let corpus = select(&population(4), 4, &observed).unwrap();
        let population_row = corpus
            .populations
            .iter()
            .find(|p| p.semantic_action == "deposit")
            .expect("deposit population");
        assert_eq!(population_row.observed, Some(681));
        assert_eq!(population_row.replay_eligible, 4);
        assert!(corpus
            .limitations
            .iter()
            .any(|l| l.code == "deposit_replayability_materially_below_observed"));
    }

    /// "We could not replay it" and "it does not happen" are different claims.
    /// An unmeasured population is reported as unmeasured, never as zero.
    #[test]
    fn an_unmeasured_observed_population_is_not_reported_as_zero() {
        let corpus = select(&population(4), 4, &Default::default()).unwrap();
        for row in &corpus.populations {
            assert_eq!(row.observed, None);
        }
        // And with nothing observed recorded, no under-representation claim is
        // made either, because there is nothing to compare against.
        assert!(!corpus
            .limitations
            .iter()
            .any(|l| l.code.ends_with("_replayability_materially_below_observed")));
    }

    #[test]
    fn contract_limitations_are_always_stated() {
        let corpus = select(&population(3), 3, &Default::default()).unwrap();
        for code in [
            "failed_original_transactions_unsupported",
            "account_creation_paths_unsupported",
            "replay_runtime_rent_semantics_unsupported",
        ] {
            assert!(
                corpus.limitations.iter().any(|l| l.code == code),
                "{code} must always be stated"
            );
        }
    }

    #[test]
    fn the_policy_version_is_recorded() {
        let corpus = select(&population(3), 3, &Default::default()).unwrap();
        assert_eq!(corpus.selection_policy_version, SELECTION_POLICY_VERSION);
        assert_eq!(corpus.selection_policy, "stratified-diversity");
        assert_eq!(
            corpus.adapter_version,
            crate::protocol::adapter_for(&base().program_id).map(|a| a.adapter_version()),
        );
    }
}
