//! Portfolio evidence coverage. Representative samples and independent VM probes
//! never imply class-wide assurance, historical execution, or simultaneous capacity.
use crate::{
    lifecycle::{
        consequence::{
            LifecycleExecutionStatus, LifecycleImpactClassification, LifecycleImpactReport,
        },
        exposure::sha256,
        EntityType, LifecycleSnapshot,
    },
    path::{
        self as probe, CapturedExecutionFixture, ExecutionProbeSpec, ExitPathType,
        ExitabilityReport, ProbeExecutionStatus,
    },
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CoverageClassification {
    Proven,
    PartiallyProven,
    Untested,
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaseStatus {
    Succeeded,
    Failed,
    Indeterminate,
    Untested,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageCase {
    pub id: String,
    pub target_entity: String,
    pub path_type: ExitPathType,
    pub venue: Option<String>,
    pub input_amount_raw: String,
    /// Inline spec, with fixture resolved relative to the coverage plan file.
    pub execution_probe: Option<ExecutionProbeSpec>,
    /// Optional prior result, independently verified by fresh offline execution.
    pub execution_report: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoveragePlan {
    pub schema_version: u32,
    pub snapshot_sha256: String,
    pub scenario_sha256: String,
    pub impact_sha256: String,
    pub requested_paths: Vec<ExitPathType>,
    pub cases: Vec<CoverageCase>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepresentativeClass {
    pub class: String,
    pub population_entities: usize,
    pub selected_entities: Vec<String>,
    pub selection_rule: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    pub case: CoverageCase,
    pub status: CaseStatus,
    pub reason: String,
    pub execution_report: Option<ExitabilityReport>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathCoverage {
    pub path_type: ExitPathType,
    pub classification: CoverageClassification,
    pub largest_successful_input_raw: String,
    pub represented_amount_covered_raw: String,
    pub represented_amount_without_evidence_raw: String,
    pub successful_cases: Vec<String>,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityCoverage {
    pub entity_id: String,
    pub owner_authority: String,
    pub account_type: String,
    pub representative_class: String,
    pub selected_representative: bool,
    pub impact_classification: LifecycleImpactClassification,
    pub represented_balance_raw: String,
    pub classification: CoverageClassification,
    pub represented_amount_covered_raw: String,
    pub represented_amount_without_evidence_raw: String,
    pub paths: Vec<PathCoverage>,
    pub official_transition: LifecycleExecutionStatus,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageAggregate {
    /// Holder unit is a token-account entity, never a human identity.
    pub entities: usize,
    pub positive_balance_entities: usize,
    pub distinct_owner_authorities: usize,
    pub authorities_with_measured_amount: usize,
    pub classifications: BTreeMap<CoverageClassification, usize>,
    pub represented_amount_raw: String,
    pub represented_amount_covered_raw: String,
    pub represented_amount_without_evidence_raw: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageReport {
    pub schema_version: u32,
    pub evaluated_at: chrono::DateTime<chrono::Utc>,
    pub asset_mint: String,
    pub snapshot_sha256: String,
    pub scenario_sha256: String,
    pub impact_sha256: String,
    pub plan_sha256: String,
    pub representatives: Vec<RepresentativeClass>,
    pub cases: Vec<CaseResult>,
    pub entities: Vec<EntityCoverage>,
    pub portfolio: CoverageAggregate,
    pub by_account_type: BTreeMap<String, CoverageAggregate>,
    pub by_path_type: BTreeMap<String, CoverageAggregate>,
    /// Venue cohorts contain explicitly requested entities only; overlap is explicit.
    pub by_venue: BTreeMap<String, CoverageAggregate>,
    pub official_transition: LifecycleExecutionStatus,
    pub limitations: Vec<String>,
}
fn json_hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(sha256(
        (serde_json::to_string_pretty(value)? + "\n").as_bytes(),
    ))
}
fn raw(value: &str) -> Result<u64> {
    let n = value.parse::<u64>().context("invalid raw token amount")?;
    ensure!(
        n.to_string() == value,
        "raw amount must use canonical unsigned decimal"
    );
    Ok(n)
}
fn key<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .expect("enum serialization")
        .as_str()
        .expect("unit enum")
        .into()
}
fn supported(entity: &EntityType, path: ExitPathType) -> bool {
    *entity == EntityType::WalletCompatible && path == ExitPathType::SecondaryMarketExit
}
/// Stable class representatives: first zero, smallest positive, largest positive.
/// No sample's execution is extrapolated to another entity in its class.
pub fn representatives(
    snapshot: &LifecycleSnapshot,
    impact: &LifecycleImpactReport,
) -> Result<Vec<RepresentativeClass>> {
    let classes: BTreeMap<_, _> = snapshot
        .entities
        .iter()
        .map(|e| {
            (
                e.id.as_str(),
                format!(
                    "{}/state={}/delegated={}/confidential={}",
                    key(&e.entity_type),
                    e.state.account_state,
                    e.state.has_active_delegate,
                    e.state
                        .extensions
                        .iter()
                        .any(|e| e.extension_type == "ConfidentialTransferAccount")
                ),
            )
        })
        .collect();
    let mut groups: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();
    for e in &impact.entities {
        let role = e.verified_role.as_deref().unwrap_or("unverified");
        groups
            .entry(format!(
                "{}/role={role}",
                classes
                    .get(e.entity_id.as_str())
                    .context("impact entity absent from snapshot")?
            ))
            .or_default()
            .push((raw(&e.balance.raw)?, e.entity_id.clone()));
    }
    Ok(groups.into_iter().map(|(class, mut entries)| {
        entries.sort();
        let mut selected = BTreeSet::new();
        if let Some(e) = entries.iter().find(|e| e.0 == 0) { selected.insert(e.1.clone()); }
        if let Some(e) = entries.iter().find(|e| e.0 > 0) { selected.insert(e.1.clone()); }
        if let Some(e) = entries.iter().rev().find(|e| e.0 > 0) { selected.insert(e.1.clone()); }
        RepresentativeClass { class, population_entities: entries.len(), selected_entities: selected.into_iter().collect(), selection_rule: "Lexicographic tie-break; first zero, smallest positive and largest positive public balance. No class-wide execution inference.".into() }
    }).collect())
}
/// Construct a portable matrix. Seed specs are varied only in real raw input amount;
/// every run resets the original captured world, never manufactures holder accounts.
pub fn build_plan(
    snapshot: &LifecycleSnapshot,
    impact: &LifecycleImpactReport,
    seeds: &[ExecutionProbeSpec],
    amounts: &[u64],
) -> Result<CoveragePlan> {
    impact.validate(snapshot)?;
    ensure!(
        !amounts.is_empty() && amounts.iter().all(|a| *a > 0),
        "positive amount matrix required"
    );
    let paths = vec![
        ExitPathType::OfficialTransition,
        ExitPathType::SecondaryMarketExit,
        ExitPathType::Redemption,
        ExitPathType::Withdrawal,
        ExitPathType::Transfer,
    ];
    let mut cases = Vec::new();
    let amounts: BTreeSet<_> = amounts.iter().copied().collect();
    for (seed_index, seed) in seeds.iter().enumerate() {
        ensure!(
            seed.snapshot_sha256 == impact.after.snapshot_sha256
                && seed.scenario_sha256 == impact.scenario_sha256,
            "seed differs from population/scenario"
        );
        ensure!(
            impact
                .entities
                .iter()
                .any(|e| e.entity_id == seed.target_entity),
            "seed target absent from population"
        );
        cases.push(CoverageCase {
            id: format!("seed-{seed_index}-baseline"),
            target_entity: seed.target_entity.clone(),
            path_type: seed.path_type,
            venue: Some(seed.pool.clone()),
            input_amount_raw: seed.input_amount_raw.clone(),
            execution_probe: Some(seed.clone()),
            execution_report: None,
        });
        for amount in &amounts {
            let mut spec = seed.clone();
            let id = format!("seed-{seed_index}-raw-{amount}");
            spec.id = id.clone();
            spec.input_amount_raw = amount.to_string();
            spec.amount_reason = format!("Phase 6 independent real raw-unit amount matrix: {amount}. Original captured state reset for every case; minimum output retained as a test threshold, no simultaneous capacity inference.");
            cases.push(CoverageCase {
                id,
                target_entity: seed.target_entity.clone(),
                path_type: seed.path_type,
                venue: Some(seed.pool.clone()),
                input_amount_raw: amount.to_string(),
                execution_probe: Some(spec),
                execution_report: None,
            });
        }
    }
    for r in representatives(snapshot, impact)? {
        for entity in r.selected_entities {
            let balance = raw(&impact
                .entities
                .iter()
                .find(|e| e.entity_id == entity)
                .context("representative missing")?
                .balance
                .raw)?;
            if balance == 0 {
                continue;
            }
            for path in &paths {
                if cases
                    .iter()
                    .any(|c| c.target_entity == entity && c.path_type == *path)
                {
                    continue;
                }
                cases.push(CoverageCase {
                    id: format!("representative:{entity}:{}", key(path)),
                    target_entity: entity.clone(),
                    path_type: *path,
                    venue: None,
                    input_amount_raw: balance
                        .min(*amounts.first().expect("nonempty amounts"))
                        .to_string(),
                    execution_probe: None,
                    execution_report: None,
                });
            }
        }
    }
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(CoveragePlan {
        schema_version: 1,
        snapshot_sha256: impact.after.snapshot_sha256.clone(),
        scenario_sha256: impact.scenario_sha256.clone(),
        impact_sha256: sha256(impact.to_json()?.as_bytes()),
        requested_paths: paths,
        cases,
    })
}
fn classification(balance: u64, covered: u64, unsupported: bool) -> CoverageClassification {
    if balance > 0 && covered == balance {
        CoverageClassification::Proven
    } else if covered > 0 {
        CoverageClassification::PartiallyProven
    } else if unsupported {
        CoverageClassification::Unsupported
    } else {
        CoverageClassification::Untested
    }
}
fn aggregate<'a>(
    entries: impl Iterator<Item = (&'a EntityCoverage, CoverageClassification, u64)>,
) -> Result<CoverageAggregate> {
    let mut a = CoverageAggregate::default();
    for c in [
        CoverageClassification::Proven,
        CoverageClassification::PartiallyProven,
        CoverageClassification::Untested,
        CoverageClassification::Unsupported,
    ] {
        a.classifications.insert(c, 0);
    }
    let mut owners = BTreeSet::new();
    let mut measured = BTreeSet::new();
    let mut total = 0u128;
    let mut covered = 0u128;
    for (e, c, n) in entries {
        let balance = raw(&e.represented_balance_raw)?;
        ensure!(n <= balance, "coverage exceeds represented balance");
        a.entities += 1;
        a.positive_balance_entities += usize::from(balance > 0);
        *a.classifications.entry(c).or_default() += 1;
        owners.insert(&e.owner_authority);
        if n > 0 {
            measured.insert(&e.owner_authority);
        }
        total = total
            .checked_add(u128::from(balance))
            .context("population amount overflow")?;
        covered = covered
            .checked_add(u128::from(n))
            .context("coverage amount overflow")?;
    }
    a.distinct_owner_authorities = owners.len();
    a.authorities_with_measured_amount = measured.len();
    a.represented_amount_raw = total.to_string();
    a.represented_amount_covered_raw = covered.to_string();
    a.represented_amount_without_evidence_raw = (total - covered).to_string();
    Ok(a)
}
/// Revalidate the full population and replay every supplied probe before joining it.
/// Missing fixtures and malformed/tampered reports are errors, never silent skips.
pub fn evaluate(
    snapshot: &LifecycleSnapshot,
    impact: &LifecycleImpactReport,
    plan: &CoveragePlan,
    plan_dir: &Path,
) -> Result<CoverageReport> {
    impact.validate(snapshot)?;
    ensure!(
        plan.schema_version == 1
            && plan.snapshot_sha256 == impact.after.snapshot_sha256
            && plan.scenario_sha256 == impact.scenario_sha256
            && plan.impact_sha256 == sha256(impact.to_json()?.as_bytes()),
        "coverage population/scenario/impact fingerprint mismatch"
    );
    ensure!(
        !plan.requested_paths.is_empty(),
        "requested paths must be explicit"
    );
    let mut paths = plan.requested_paths.clone();
    paths.sort_by_key(key);
    paths.dedup();
    ensure!(
        paths.len() == plan.requested_paths.len(),
        "duplicate requested path"
    );
    let population: BTreeMap<_, _> = impact
        .entities
        .iter()
        .map(|e| (e.entity_id.as_str(), e))
        .collect();
    let world: BTreeMap<_, _> = snapshot
        .entities
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();
    let mut cases = plan.cases.clone();
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    let mut ids = BTreeSet::new();
    let mut probe_ids = BTreeSet::new();
    let mut results = Vec::new();
    for case in cases {
        ensure!(
            !case.id.trim().is_empty() && ids.insert(case.id.clone()),
            "empty or duplicate case ID"
        );
        let entity = population
            .get(case.target_entity.as_str())
            .context("case entity absent from population")?;
        ensure!(
            paths.contains(&case.path_type),
            "case path absent from requested paths"
        );
        let amount = raw(&case.input_amount_raw)?;
        ensure!(amount > 0, "positive case input required");
        ensure!(
            case.venue.as_ref().is_none_or(|v| !v.trim().is_empty()),
            "empty venue"
        );
        let mut report = None;
        let (status, reason) = if let Some(spec) = &case.execution_probe {
            ensure!(probe_ids.insert(spec.id.clone()), "duplicate probe ID");
            ensure!(
                spec.target_entity == case.target_entity
                    && spec.path_type == case.path_type
                    && Some(&spec.pool) == case.venue.as_ref()
                    && spec.input_amount_raw == case.input_amount_raw,
                "case/probe target, path, venue or amount mismatch"
            );
            let bytes = crate::lifecycle::artifact::read(plan_dir.join(&spec.fixture))
                .context("missing captured execution fixture")?;
            ensure!(
                sha256(&bytes) == spec.fixture_sha256,
                "fixture file hash mismatch"
            );
            let fixture: CapturedExecutionFixture = serde_json::from_slice(&bytes)?;
            let replay = probe::run(
                snapshot,
                &impact.scenario,
                spec,
                &fixture,
                impact.after.evaluated_at,
            )?;
            if let Some(path) = &case.execution_report {
                let prior: ExitabilityReport = serde_json::from_slice(
                    &crate::lifecycle::artifact::read(plan_dir.join(path))?,
                )?;
                ensure!(
                    prior == replay,
                    "prior report differs from fresh offline execution"
                );
            }
            // Probe execution uses its own standard before-time. The population's
            // chosen before-time remains bound by impact_sha256; only its semantic
            // before-view may differ, never balances, evidence or the after-view.
            let mut expected = (*entity).clone();
            let observed = &replay.lifecycle_assessment.entity_impact;
            expected.pre_lifecycle_status = observed.pre_lifecycle_status;
            expected.pre_classification = observed.pre_classification;
            expected.economic_meaning_changed = observed.economic_meaning_changed;
            ensure!(
                expected == *observed,
                "probe state/after-view differs from population entity"
            );
            let status = match replay.execution_status {
                ProbeExecutionStatus::Succeeded => CaseStatus::Succeeded,
                ProbeExecutionStatus::Failed => CaseStatus::Failed,
                ProbeExecutionStatus::Indeterminate => CaseStatus::Indeterminate,
            };
            let reason = replay
                .blocker
                .clone()
                .unwrap_or_else(|| replay.lifecycle_assessment.conclusion.clone());
            report = Some(replay);
            (status, reason)
        } else {
            ensure!(
                case.execution_report.is_none(),
                "prior report requires a probe and fixture for replay"
            );
            if supported(&entity.entity_type, case.path_type) {
                (CaseStatus::Untested,"No captured executable route supplied for this entity/path/venue/amount. Representative selection does not prove execution.".into())
            } else {
                (CaseStatus::Unsupported,"Current engine supports only wallet-compatible owner-signed Meteora DLMM secondary-market probes. This capability limit does not establish that the asset cannot exit.".into())
            }
        };
        results.push(CaseResult {
            case,
            status,
            reason,
            execution_report: report,
        });
    }
    let reps = representatives(snapshot, impact)?;
    let selected: BTreeSet<_> = reps
        .iter()
        .flat_map(|r| r.selected_entities.iter())
        .collect();
    let mut entities = Vec::new();
    for (id, e) in population {
        let balance = raw(&e.balance.raw)?;
        let mut entity_paths = Vec::new();
        for path in &paths {
            let relevant: Vec<_> = results
                .iter()
                .filter(|r| r.case.target_entity == id && r.case.path_type == *path)
                .collect();
            let mut largest = 0;
            let mut covered = 0;
            let mut successful = Vec::new();
            for r in relevant {
                if r.status != CaseStatus::Succeeded {
                    continue;
                }
                let report = r
                    .execution_report
                    .as_ref()
                    .context("success without actual execution")?;
                let d = report
                    .deltas
                    .as_ref()
                    .context("success without reconciliation")?;
                ensure!(
                    d.reconciled && report.execution.as_ref().is_some_and(|x| x.success),
                    "unreconciled success"
                );
                let amount = raw(&d.input_debited_raw)?;
                ensure!(
                    amount == raw(&r.case.input_amount_raw)?,
                    "input debit mismatch"
                );
                largest = largest.max(amount);
                successful.push(r.case.id.clone());
                // Different bank balances remain explicit measured cases, but cannot
                // cover the earlier represented amount by inventing a time bridge.
                if report
                    .lifecycle_assessment
                    .source_balance_at_phase5_raw
                    .as_deref()
                    == Some(e.balance.raw.as_str())
                {
                    covered = covered.max(amount.min(balance));
                }
            }
            let unsupported = !supported(&e.entity_type, *path);
            entity_paths.push(PathCoverage { path_type:*path,classification:classification(balance,covered,unsupported),largest_successful_input_raw:largest.to_string(),represented_amount_covered_raw:covered.to_string(),represented_amount_without_evidence_raw:(balance-covered).to_string(),successful_cases:successful,reason:if largest>0 && covered==0 { "Measured execution has a different captured balance; no historical population amount coverage is inferred.".into() } else { "Coverage is the maximum successful exact input with matching source balance, under retained captured-bank/signer/runtime assumptions. Amounts and alternative paths are never added; failed probes do not prove stranded exposure.".into() } });
        }
        let covered = entity_paths
            .iter()
            .map(|p| raw(&p.represented_amount_covered_raw))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .unwrap_or(0);
        let unsupported = entity_paths
            .iter()
            .all(|p| p.classification == CoverageClassification::Unsupported);
        let source = world.get(id).context("population entity missing")?;
        entities.push(EntityCoverage {
            entity_id: id.into(),
            owner_authority: source.state.owner.clone(),
            account_type: key(&e.entity_type),
            representative_class: format!(
                "{}/state={}/delegated={}/confidential={}/role={}",
                key(&source.entity_type),
                source.state.account_state,
                source.state.has_active_delegate,
                source
                    .state
                    .extensions
                    .iter()
                    .any(|e| e.extension_type == "ConfidentialTransferAccount"),
                e.verified_role.as_deref().unwrap_or("unverified")
            ),
            selected_representative: selected.contains(&e.entity_id),
            impact_classification: e.impact_classification,
            represented_balance_raw: e.balance.raw.clone(),
            classification: classification(balance, covered, unsupported),
            represented_amount_covered_raw: covered.to_string(),
            represented_amount_without_evidence_raw: (balance - covered).to_string(),
            paths: entity_paths,
            official_transition: LifecycleExecutionStatus::NotTested,
        });
    }
    let portfolio = aggregate(entities.iter().map(|e| {
        (
            e,
            e.classification,
            raw(&e.represented_amount_covered_raw).expect("generated amount"),
        )
    }))?;
    let mut by_account_type = BTreeMap::new();
    for kind in entities
        .iter()
        .map(|e| e.account_type.clone())
        .collect::<BTreeSet<_>>()
    {
        by_account_type.insert(
            kind.clone(),
            aggregate(entities.iter().filter(|e| e.account_type == kind).map(|e| {
                (
                    e,
                    e.classification,
                    raw(&e.represented_amount_covered_raw).expect("generated amount"),
                )
            }))?,
        );
    }
    let mut by_path_type = BTreeMap::new();
    for path in &paths {
        by_path_type.insert(
            key(path),
            aggregate(entities.iter().map(|e| {
                let p = e
                    .paths
                    .iter()
                    .find(|p| p.path_type == *path)
                    .expect("requested path");
                (
                    e,
                    p.classification,
                    raw(&p.represented_amount_covered_raw).expect("generated amount"),
                )
            }))?,
        );
    }
    let mut by_venue = BTreeMap::new();
    for venue in results
        .iter()
        .filter_map(|r| r.case.venue.clone())
        .collect::<BTreeSet<_>>()
    {
        let members: BTreeSet<_> = results
            .iter()
            .filter(|r| r.case.venue.as_ref() == Some(&venue))
            .map(|r| r.case.target_entity.as_str())
            .collect();
        by_venue.insert(
            venue.clone(),
            aggregate(
                entities
                    .iter()
                    .filter(|e| members.contains(e.entity_id.as_str()))
                    .map(|e| {
                        let n = results
                            .iter()
                            .filter(|r| {
                                r.case.target_entity == e.entity_id
                                    && r.case.venue.as_ref() == Some(&venue)
                                    && r.status == CaseStatus::Succeeded
                            })
                            .filter_map(|r| r.execution_report.as_ref())
                            .filter(|r| {
                                r.lifecycle_assessment.source_balance_at_phase5_raw.as_ref()
                                    == Some(&e.represented_balance_raw)
                            })
                            .filter_map(|r| r.deltas.as_ref())
                            .map(|d| raw(&d.input_debited_raw).expect("verified amount"))
                            .max()
                            .unwrap_or(0)
                            .min(raw(&e.represented_balance_raw).expect("verified balance"));
                        let unsupported = results
                            .iter()
                            .filter(|r| {
                                r.case.target_entity == e.entity_id
                                    && r.case.venue.as_ref() == Some(&venue)
                            })
                            .all(|r| r.status == CaseStatus::Unsupported);
                        (
                            e,
                            classification(
                                raw(&e.represented_balance_raw).expect("verified balance"),
                                n,
                                unsupported,
                            ),
                            n,
                        )
                    }),
            )?,
        );
    }
    Ok(CoverageReport { schema_version:1,evaluated_at:impact.after.evaluated_at,asset_mint:impact.asset_mint.clone(),snapshot_sha256:plan.snapshot_sha256.clone(),scenario_sha256:plan.scenario_sha256.clone(),impact_sha256:plan.impact_sha256.clone(),plan_sha256:json_hash(plan)?,representatives:reps,cases:results,entities,portfolio,by_account_type,by_path_type,by_venue,official_transition:LifecycleExecutionStatus::NotTested,limitations:vec![
        "Proven means a positive represented public amount has a full exact-input local execution witness on at least one requested path under recorded signer/runtime assumptions. It never means lifecycle transition, real signing access, inclusion or future exitability.".into(),
        "PartiallyProven means a bounded exact-input witness below the represented amount. Smaller amounts, differences between tested amounts, and the remainder are not independently executed or guaranteed.".into(),
        "Population holder counts are token-account entities. Distinct owner authorities are reported separately; neither proves human identity. Zero public balances are never vacuously proven; confidential exposure is unquantified.".into(),
        "Represented amount coverage requires matching captured source balance and is a conditional evidence envelope, not execution of the historical Phase 4 bank. Complete case reports retain separate bank Clock, account evidence, signer assumptions and minimum-output thresholds.".into(),
        "Use maxima per entity across independently reset amounts, venues and paths. Aggregated measured amounts are evidence coverage only, never simultaneous pool capacity, a quote, valuation or promised proceeds. Venue/path cohorts overlap and must not be added.".into(),
        "Unsupported means the current adapter/signing model lacks the requested capability, not that all exits fail. Failed and indeterminate attempts remain explicit and give no positive amount coverage.".into(),
        "OfficialTransition remains NotTested. Vault tokens are counted once as population accounts; Phase 3 protocol observations are overlapping descriptions and are not added. LP/vault withdrawal and successor transition are not inferred.".into(),
        "No mainnet transaction, RPC acquisition, account fabrication, price feed or external valuation is performed by coverage. Only supplied captured fixtures execute in fresh local VMs; representative samples never cover their class peers.".into(),
    ] })
}
impl CoverageReport {
    pub fn to_json(&self) -> Result<String> {
        crate::canonical::document(self)
    }
    pub fn render_text(&self) -> String {
        format!("Lifecycle portfolio assurance (conditional local execution evidence)\nAsset: {}\nEntities: {} ({} positive public balances); distinct owner authorities: {}\nClassification: {:?}\nRepresented public amount: {} raw\nMeasured amount envelope: {} raw; without execution evidence: {} raw\nCases: {}; venue cohorts: {}\nOfficialTransition: NotTested\nAmounts are independent local witnesses, not simultaneous liquidity or future exit guarantees.\n",self.asset_mint,self.portfolio.entities,self.portfolio.positive_balance_entities,self.portfolio.distinct_owner_authorities,self.portfolio.classifications,self.portfolio.represented_amount_raw,self.portfolio.represented_amount_covered_raw,self.portfolio.represented_amount_without_evidence_raw,self.cases.len(),self.by_venue.len())
    }
    pub fn validate(
        &self,
        snapshot: &LifecycleSnapshot,
        impact: &LifecycleImpactReport,
        plan: &CoveragePlan,
        plan_dir: &Path,
    ) -> Result<()> {
        ensure!(
            *self == evaluate(snapshot, impact, plan, plan_dir)?,
            "coverage report differs from deterministic replay"
        );
        Ok(())
    }
}
