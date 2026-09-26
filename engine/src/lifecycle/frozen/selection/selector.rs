use super::discovery::{ContextKind, VenueInventory};
use super::*;
use anyhow::Context;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreWeights {
    #[serde(with = "crate::numfmt::u64_string")]
    pub entity: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub amount: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub class: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub venue: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub path: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub entity_path_context: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub state_shape: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub feasibility: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub redundancy_penalty: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectorConfig {
    pub selector_version: String,
    pub probe_budget: usize,
    pub max_distinct_entities: usize,
    pub max_groups_per_entity: usize,
    pub required_balance_buckets: Vec<u8>,
    pub minimum_distinct_paths: usize,
    pub amount_score_cap_raw: String,
    pub weights: ScoreWeights,
    pub amount_strategy: String,
}
impl Default for SelectorConfig {
    fn default() -> Self {
        Self {
            selector_version: "lifecycle-gap-v1".into(),
            probe_budget: 6,
            max_distinct_entities: 3,
            max_groups_per_entity: 2,
            required_balance_buckets: vec![0, 1, 3],
            minimum_distinct_paths: 2,
            amount_score_cap_raw: "1000000000".into(),
            weights: ScoreWeights {
                entity: 100,
                amount: 20,
                class: 40,
                venue: 60,
                path: 80,
                entity_path_context: 30,
                state_shape: 25,
                feasibility: 5,
                redundancy_penalty: 1000,
            },
            amount_strategy: "integer-fractions-1-25-50-100-plus-one-v1".into(),
        }
    }
}
impl SelectorConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.selector_version == "lifecycle-gap-v1"
                && self.amount_strategy == "integer-fractions-1-25-50-100-plus-one-v1",
            "unknown selector/amount strategy version"
        );
        ensure!(
            self.probe_budget > 0
                && self.probe_budget <= 10
                && self.max_distinct_entities > 0
                && self.max_distinct_entities <= self.probe_budget
                && self.max_groups_per_entity > 0
                && self.max_groups_per_entity <= 2,
            "bounded positive group/entity budget required"
        );
        ensure!(
            self.required_balance_buckets.iter().all(|b| *b < 4)
                && self
                    .required_balance_buckets
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    == self.required_balance_buckets.len()
                && self.required_balance_buckets.len() <= self.max_distinct_entities,
            "invalid quartile diversity constraint"
        );
        ensure!(
            self.minimum_distinct_paths <= 2 && self.minimum_distinct_paths <= self.probe_budget,
            "unsupported path diversity budget"
        );
        ensure!(
            self.amount_score_cap_raw.parse::<u64>()? > 0,
            "positive declared raw amount score cap required"
        );
        Ok(())
    }
}
#[derive(Default, Clone)]
struct VirtualCoverage {
    amounts: BTreeMap<String, u64>,
    path_amounts: BTreeMap<(String, String, String), u64>,
    authorities: BTreeSet<String>,
    classes: BTreeSet<String>,
    venues: BTreeSet<String>,
    paths: BTreeSet<String>,
    shapes: BTreeSet<String>,
}
fn state_shape(
    s: &crate::lifecycle::LifecycleEntity,
    bucket: u8,
    role: Option<&str>,
) -> Result<String> {
    let mut extensions: Vec<_> = s
        .state
        .extensions
        .iter()
        .map(|e| e.extension_type.clone())
        .collect();
    extensions.sort();
    digest(&serde_json::json!({
    "authority_type":s.entity_type,
    "initialized":s.state.is_initialized,
    "frozen":s.state.is_frozen,
    "delegate_present":s.state.delegate.is_some(),
    "active_delegation":s.state.has_active_delegate,
    "extension_flags":extensions,
    "on_curve":s.authority_observation.is_on_curve,
    "owner_runtime":s.authority_observation.runtime_owner,
    "owner_executable":s.authority_observation.executable,
    "role":role,
    "population_rank_quartile":bucket}
    ))
}
fn score(
    c: &CandidateProbe,
    v: &VirtualCoverage,
    cfg: &SelectorConfig,
) -> Result<(ScoreComponents, ExpectedGain)> {
    ensure!(
        matches!(
            c.eligibility,
            Eligibility::CaptureRequired | Eligibility::ExecutableCandidate
        ),
        "unsupported/invalid candidates cannot be scored"
    );
    let balance = c.represented_raw.parse::<u64>()?;
    let already = v.amounts.get(&c.entity_id).copied().unwrap_or(0);
    let raw_gain = balance.saturating_sub(already);
    let cap = cfg.amount_score_cap_raw.parse::<u64>()?;
    let path = path_key(c.path_type);
    let context_new =
        !v.path_amounts
            .contains_key(&(c.entity_id.clone(), path.clone(), c.context_id.clone()));
    let g = ExpectedGain {
        entities: usize::from(already == 0),
        authorities: usize::from(!v.authorities.contains(&c.authority)),
        represented_raw: raw_gain.to_string(),
        entity_path_contexts: usize::from(context_new),
        venue: usize::from(
            c.path_type == ExitPathType::SecondaryMarketExit && !v.venues.contains(&c.context_id),
        ),
        path_type: usize::from(!v.paths.contains(&path)),
        state_shape: usize::from(!v.shapes.contains(&c.state_shape_sha256)),
    };
    let w = &cfg.weights;
    let mut components = ScoreComponents {
        entity: u128::from(w.entity) * g.entities as u128,
        amount: u128::from(raw_gain.min(cap)) * u128::from(w.amount) / u128::from(cap),
        class: u128::from(w.class) * u128::from(!v.classes.contains(&c.account_class)),
        venue: u128::from(w.venue) * g.venue as u128,
        path: u128::from(w.path) * g.path_type as u128,
        entity_path_context: u128::from(w.entity_path_context) * g.entity_path_contexts as u128,
        state_shape: u128::from(w.state_shape) * g.state_shape as u128,
        feasibility: u128::from(w.feasibility)
            * u128::from(matches!(
                c.eligibility,
                Eligibility::ExecutableCandidate | Eligibility::CaptureRequired
            )),
        redundancy_penalty: if !context_new && raw_gain == 0 {
            u128::from(w.redundancy_penalty)
        } else {
            0
        },
        total: 0,
    };
    let total = [
        components.entity,
        components.amount,
        components.class,
        components.venue,
        components.path,
        components.entity_path_context,
        components.state_shape,
        components.feasibility,
    ]
    .into_iter()
    .try_fold(0u128, |a, x| a.checked_add(x).context("score overflow"))?;
    components.total = i128::try_from(total)? - i128::try_from(components.redundancy_penalty)?;
    Ok((components, g))
}
fn apply(v: &mut VirtualCoverage, c: &CandidateProbe) -> Result<()> {
    let b = c.represented_raw.parse()?;
    v.amounts
        .entry(c.entity_id.clone())
        .and_modify(|n| *n = (*n).max(b))
        .or_insert(b);
    v.path_amounts.insert(
        (
            c.entity_id.clone(),
            path_key(c.path_type),
            c.context_id.clone(),
        ),
        b,
    );
    v.authorities.insert(c.authority.clone());
    v.classes.insert(c.account_class.clone());
    if c.path_type == ExitPathType::SecondaryMarketExit {
        v.venues.insert(c.context_id.clone());
    }
    v.paths.insert(path_key(c.path_type));
    v.shapes.insert(c.state_shape_sha256.clone());
    Ok(())
}
/// All score and diversity inputs are observed population/current *baseline* facts.
/// New execution results are intentionally absent from this API.
pub fn expand(
    s: &LifecycleSnapshot,
    b: &CoverageReport,
    inventory: &VenueInventory,
    cfg: &SelectorConfig,
) -> Result<ExpansionPlan> {
    cfg.validate()?;
    ensure!(
        b.schema_version == 1 && b.asset_mint == s.asset.mint,
        "baseline population/mint mismatch"
    );
    let mut normalized = s.clone();
    normalized
        .entities
        .sort_by(|a, b| a.token_account.cmp(&b.token_account));
    ensure!(
        digest(&normalized)? == b.snapshot_sha256,
        "planner snapshot differs from baseline population"
    );
    let mut baseline = b.clone();
    baseline
        .entities
        .sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
    inventory.validate(&normalized, &baseline)?;
    let population: BTreeMap<_, _> = normalized
        .entities
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();
    let covered: BTreeMap<_, _> = baseline
        .entities
        .iter()
        .map(|e| (e.entity_id.as_str(), e))
        .collect();
    ensure!(
        population.len() == covered.len() && population.keys().eq(covered.keys()),
        "planner population/entity coverage mismatch"
    );
    let mut wallet_balances: Vec<_> = normalized
        .entities
        .iter()
        .filter(|e| e.entity_type == EntityType::WalletCompatible && e.state.raw_balance != "0")
        .map(|e| Ok((e.state.raw_balance.parse::<u64>()?, e.id.clone())))
        .collect::<Result<_>>()?;
    wallet_balances.sort();
    let buckets: BTreeMap<_, _> = wallet_balances
        .iter()
        .enumerate()
        .map(|(rank, (_, id))| (id.as_str(), ((rank * 4) / wallet_balances.len()) as u8))
        .collect();
    let mut virtual_before = VirtualCoverage::default();
    for e in &baseline.entities {
        let n = e.represented_amount_covered_raw.parse::<u64>()?;
        if n == 0 {
            continue;
        }
        let state = population[e.entity_id.as_str()];
        virtual_before.amounts.insert(e.entity_id.clone(), n);
        virtual_before.authorities.insert(e.owner_authority.clone());
        virtual_before
            .classes
            .insert(e.representative_class.clone());
        virtual_before.shapes.insert(state_shape(
            state,
            *buckets.get(e.entity_id.as_str()).unwrap_or(&0),
            Some(&e.representative_class),
        )?);
    }
    for result in &baseline.cases {
        if result.status != CaseStatus::Succeeded {
            continue;
        }
        let c = &result.case;
        virtual_before
            .path_amounts
            .entry((
                c.target_entity.clone(),
                path_key(c.path_type),
                c.venue.clone().unwrap_or_default(),
            ))
            .and_modify(|n| *n = (*n).max(c.input_amount_raw.parse().unwrap()))
            .or_insert(c.input_amount_raw.parse()?);
        if c.path_type == ExitPathType::SecondaryMarketExit {
            if let Some(v) = &c.venue {
                virtual_before.venues.insert(v.clone());
            }
        }
        virtual_before.paths.insert(path_key(c.path_type));
    }
    let mut candidates = Vec::new();
    type GapMembers<'a> = BTreeMap<
        (String, String),
        Vec<(
            &'a crate::lifecycle::frozen::coverage::EntityCoverage,
            u64,
            bool,
        )>,
    >;
    let mut gap_groups: GapMembers<'_> = BTreeMap::new();
    for (id, e) in &population {
        let amount = e.state.raw_balance.parse::<u64>()?;
        if amount == 0 {
            continue;
        }
        let prior = covered[id];
        for p in &prior.paths {
            let supported = e.entity_type == EntityType::WalletCompatible
                && matches!(
                    p.path_type,
                    ExitPathType::SecondaryMarketExit | ExitPathType::Transfer
                );
            gap_groups
                .entry((prior.representative_class.clone(), path_key(p.path_type)))
                .or_default()
                .push((prior, p.represented_amount_covered_raw.parse()?, supported));
        }
        for context in inventory.contexts.iter().filter(|c| c.execution_supported) {
            let path = match context.kind {
                ContextKind::MeteoraDlmm => ExitPathType::SecondaryMarketExit,
                ContextKind::Token2022Destination => ExitPathType::Transfer,
            };
            let (eligibility, reason) = if e.token_account == context.address {
                (
                    Eligibility::Invalid,
                    "Source and transfer recipient are the same account".into(),
                )
            } else if e.entity_type != EntityType::WalletCompatible || !context.execution_supported
            {
                (
                    Eligibility::Unsupported,
                    format!(
                        "Authority or context adapter unsupported: {}",
                        context.reason
                    ),
                )
            } else if !e.state.is_initialized
                || e.state.is_frozen
                || e.state
                    .extensions
                    .iter()
                    .any(|x| x.extension_type == "ConfidentialTransferAccount")
            {
                (Eligibility::Unsupported,"Observed initialized/unfrozen public-source preconditions are absent or confidential authority semantics unsupported".into())
            } else {
                (Eligibility::CaptureRequired,"Real positive observed source and supported concrete context; finalized source/destination/owner/code/route batch required before execution".into())
            };
            let (eligibility, reason) = if eligibility == Eligibility::CaptureRequired
                && baseline.cases.iter().any(|r| {
                    r.status == CaseStatus::Succeeded
                        && r.case.target_entity == *id
                        && r.case.path_type == path
                        && r.case.venue.as_deref() == Some(context.id.as_str())
                }) {
                (Eligibility::ExecutableCandidate,"Existing independently validated Phase 6 capture has all required executable state for this exact entity/path/context; redundant evidence is penalized".into())
            } else {
                (eligibility, reason)
            };
            let mut c = CandidateProbe {
                id: format!("{}:{}:{}", id, path_key(path), context.id),
                entity_id: (*id).into(),
                authority: e.state.owner.clone(),
                account_class: prior.representative_class.clone(),
                represented_raw: amount.to_string(),
                balance_bucket: *buckets.get(id).unwrap_or(&0),
                state_shape_sha256: state_shape(
                    e,
                    *buckets.get(id).unwrap_or(&0),
                    Some(&prior.representative_class),
                )?,
                path_type: path,
                context_id: context.id.clone(),
                eligibility,
                reason,
                initial_score: None,
            };
            if matches!(
                c.eligibility,
                Eligibility::CaptureRequired | Eligibility::ExecutableCandidate
            ) {
                c.initial_score = Some(score(&c, &virtual_before, cfg)?.0);
            }
            candidates.push(c);
        }
    }
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    let mut gaps = Vec::new();
    for ((class, path), mut members) in gap_groups {
        members.sort_by(|a, b| {
            b.0.represented_balance_raw
                .parse::<u64>()
                .unwrap()
                .cmp(&a.0.represented_balance_raw.parse::<u64>().unwrap())
                .then(a.0.entity_id.cmp(&b.0.entity_id))
        });
        let supported = members.iter().any(|x| x.2);
        let represented = members
            .iter()
            .map(|x| x.0.represented_balance_raw.parse::<u128>().unwrap())
            .sum::<u128>();
        let measured = members.iter().map(|x| u128::from(x.1)).sum::<u128>();
        gaps.push(CoverageGap {
            account_class: class,
            path_type: serde_json::from_value(serde_json::Value::String(path))?,
            entities: members.len(),
            represented_raw: represented.to_string(),
            already_measured_raw: measured.to_string(),
            without_evidence_raw: (represented - measured).to_string(),
            execution_supported: supported,
            reason: if supported {
                "Individual observed entity gaps; sample selection never proves class peers"
            } else {
                "High-value unsupported authority/path gap; not scored as executable"
            }
            .into(),
            highest_balance_entities: members
                .iter()
                .take(3)
                .map(|x| x.0.entity_id.clone())
                .collect(),
        });
    }
    let mut v = virtual_before;
    let mut selected = Vec::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut used = BTreeSet::new();
    let mut selected_paths = BTreeSet::new();
    while selected.len() < cfg.probe_budget {
        let required = cfg.required_balance_buckets.get(selected.len()).copied();
        let missing_path =
            if required.is_none() && selected_paths.len() < cfg.minimum_distinct_paths {
                Some(selected_paths.clone())
            } else {
                None
            };
        let mut ranked = Vec::new();
        for c in &candidates {
            if used.contains(&c.id)
                || !matches!(
                    c.eligibility,
                    Eligibility::CaptureRequired | Eligibility::ExecutableCandidate
                )
                || counts.get(&c.entity_id).copied().unwrap_or(0) >= cfg.max_groups_per_entity
            {
                continue;
            }
            if required.is_some_and(|q| c.balance_bucket != q || counts.contains_key(&c.entity_id))
            {
                continue;
            }
            if counts.len() >= cfg.max_distinct_entities && !counts.contains_key(&c.entity_id) {
                continue;
            }
            if missing_path
                .as_ref()
                .is_some_and(|p| p.contains(&path_key(c.path_type)))
            {
                continue;
            }
            let (score, gain) = score(c, &v, cfg)?;
            if score.total <= 0 {
                continue;
            }
            ranked.push((c, score, gain));
        }
        ranked.sort_by(|a, b| {
            b.1.total
                .cmp(&a.1.total)
                .then(
                    b.2.represented_raw
                        .parse::<u64>()
                        .unwrap()
                        .cmp(&a.2.represented_raw.parse::<u64>().unwrap()),
                )
                .then(a.0.entity_id.cmp(&b.0.entity_id))
                .then(a.0.context_id.cmp(&b.0.context_id))
                .then(path_key(a.0.path_type).cmp(&path_key(b.0.path_type)))
        });
        let Some((candidate, score, gain)) = ranked.into_iter().next() else {
            break;
        };
        apply(&mut v, candidate)?;
        used.insert(candidate.id.clone());
        *counts.entry(candidate.entity_id.clone()).or_default() += 1;
        selected_paths.insert(path_key(candidate.path_type));
        selected.push(SelectedProbe {
            selection_order: selected.len(),
            candidate: candidate.clone(),
            score,
            expected_gain: gain,
            amount_matrix: amount_matrix(candidate.represented_raw.parse()?),
            fixture_reference: format!("fixtures/group-{}.json", selected.len()),
        });
    }
    Ok(ExpansionPlan{
schema_version:1,
selector_version:cfg.selector_version.clone(),
snapshot_sha256:b.snapshot_sha256.clone(),
impact_sha256:b.impact_sha256.clone(),
coverage_sha256:digest(&baseline)?,
baseline_plan_sha256:b.plan_sha256.clone(),
inventory_sha256:digest(inventory)?,
config:cfg.clone(),
before:b.portfolio.clone(),
candidate_count:candidates.len(),
candidates,
rejected_contexts:inventory.contexts.iter().filter(|c|!c.execution_supported).cloned().collect(),
gaps,
selected,
limitations:vec!["Expected gains assume successful full exact-input probes at unchanged source balance. These are selection priorities, never assurance or capacity.".into(),
"Quartiles are positive wallet population rank / count, with (balance, entity ID) sorting. No economic meaning or price is assigned. Required buckets select small/lower-middle/large before score-only filling.".into(),
"Tie-break: score descending, marginal represented raw descending, entity ID ascending, context ID ascending, path ID ascending. No results, RNG or environment enter selection.".into(),
"Global amount gain uses per-entity maxima across selected independent paths/contexts. Context gains are separate; no class-peer inheritance or simultaneous liquidity claim.".into(),
"Unsupported/invalid candidates are retained but receive no executable score. New captures are current finalized observations, never historical lifecycle banks.".into()]}
)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate() -> CandidateProbe {
        CandidateProbe {
            id: "observed-fact".into(),
            entity_id: "entity-x".into(),
            authority: "authority-x".into(),
            account_class: "class-x".into(),
            represented_raw: "100".into(),
            balance_bucket: 0,
            state_shape_sha256: "shape-x".into(),
            path_type: ExitPathType::SecondaryMarketExit,
            context_id: "venue-x".into(),
            eligibility: Eligibility::CaptureRequired,
            reason: "Synthetic selector facts only; never execution evidence".into(),
            initial_score: None,
        }
    }
    #[test]
    fn predicted_independent_contexts_use_maxima() {
        let a = candidate();
        let mut v = VirtualCoverage::default();
        apply(&mut v, &a).unwrap();
        let mut b = a.clone();
        b.context_id = "venue-y".into();
        b.represented_raw = "30".into();
        apply(&mut v, &b).unwrap();
        assert_eq!(v.amounts["entity-x"], 100);
        let (s, g) = score(&b, &v, &SelectorConfig::default()).unwrap();
        assert_eq!(g.represented_raw, "0");
        assert_eq!(g.entities, 0);
        assert_eq!(g.venue, 0);
        assert!(s.total < 0);
    }
    #[test]
    fn unsupported_and_invalid_cannot_be_scored() {
        for eligibility in [Eligibility::Unsupported, Eligibility::Invalid] {
            let mut c = candidate();
            c.eligibility = eligibility;
            assert!(score(&c, &VirtualCoverage::default(), &SelectorConfig::default()).is_err());
        }
    }
    #[test]
    fn new_venue_adds_context_not_a_second_entity_amount() {
        let a = candidate();
        let mut v = VirtualCoverage::default();
        apply(&mut v, &a).unwrap();
        let mut b = a.clone();
        b.context_id = "venue-y".into();
        let (_, g) = score(&b, &v, &SelectorConfig::default()).unwrap();
        assert_eq!(g.represented_raw, "0");
        assert_eq!(g.entities, 0);
        assert_eq!(g.venue, 1);
        assert_eq!(g.entity_path_contexts, 1);
    }
    #[test]
    fn new_class_and_state_shape_have_declared_separate_scores() {
        let a = candidate();
        let mut v = VirtualCoverage::default();
        apply(&mut v, &a).unwrap();
        let mut b = a.clone();
        b.entity_id = "entity-y".into();
        b.authority = "authority-y".into();
        let (same, _) = score(&b, &v, &SelectorConfig::default()).unwrap();
        b.account_class = "different-class".into();
        b.state_shape_sha256 = "different-shape".into();
        let (different, _) = score(&b, &v, &SelectorConfig::default()).unwrap();
        assert_eq!(different.class, 40);
        assert_eq!(different.state_shape, 25);
        assert_eq!(different.total - same.total, 65);
    }
    #[test]
    fn amount_score_is_bounded_and_config_is_versioned() {
        let mut c = candidate();
        c.represented_raw = u64::MAX.to_string();
        let (s, _) = score(&c, &VirtualCoverage::default(), &SelectorConfig::default()).unwrap();
        assert_eq!(s.amount, 20);
        let cfg = SelectorConfig {
            selector_version: "opaque-ml".into(),
            ..SelectorConfig::default()
        };
        assert!(cfg.validate().is_err());
        let cfg = SelectorConfig {
            probe_budget: 17957,
            ..SelectorConfig::default()
        };
        assert!(cfg.validate().is_err());
    }
}
