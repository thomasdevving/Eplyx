use super::discovery::VenueInventory;
use super::*;
use crate::{
    lifecycle::decode,
    path::record::RecordedExecution,
    path::{
        self as probe, CapturedExecutionFixture, ExecutionAccountEvidence, ExecutionDeltas,
        ExecutionProbe, ExecutionProbeSpec, ProbeAdapter, ProbeClock, ProbeMessage,
        ProbePrecondition,
    },
};
use anyhow::Context;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureContext {
    CurrentFinalizedProduction,
    ExistingCapturedState,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureBinding {
    pub group_id: String,
    pub fixture_file: String,
    pub fixture_sha256: Option<String>,
    pub capture_context: CaptureContext,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureManifest {
    pub schema_version: u32,
    pub plan_sha256: String,
    pub bindings: Vec<CaptureBinding>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityAssumption {
    pub pubkey: String,
    pub authority_model: String,
    pub on_curve: bool,
    pub captured_runtime_owner: Option<String>,
    pub captured_executable: Option<bool>,
    pub signer_possession_known: bool,
    pub signer_assumed_locally: bool,
    pub wording: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEvidence {
    pub schema_version: u32,
    pub plan_sha256: String,
    pub group_id: String,
    pub case_id: String,
    pub entity_id: String,
    pub path_type: ExitPathType,
    pub context_id: String,
    pub input_raw: String,
    pub invalid_control: bool,
    pub fixture_sha256: Option<String>,
    pub captured_state_sha256: Option<String>,
    pub capture_context: CaptureContext,
    pub source_before_raw: Option<String>,
    pub authority: AuthorityAssumption,
    pub status: CaseStatus,
    pub blocker: Option<String>,
    pub preconditions: Vec<ProbePrecondition>,
    pub assumptions: Vec<String>,
    pub vm_clock: Option<ProbeClock>,
    pub message: Option<ProbeMessage>,
    pub account_evidence: Vec<ExecutionAccountEvidence>,
    pub local_accounts: Vec<crate::types::NamedAccount>,
    pub execution: Option<RecordedExecution>,
    pub deltas: Option<ExecutionDeltas>,
    pub rollback_verified: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionIndex {
    pub schema_version: u32,
    pub plan_sha256: String,
    pub capture_manifest_sha256: String,
    pub results: Vec<EvidenceReference>,
}
fn check_plan_inputs(
    s: &LifecycleSnapshot,
    b: &CoverageReport,
    p: &ExpansionPlan,
    i: &VenueInventory,
) -> Result<()> {
    p.validate(s, b, i)?;
    ensure!(p.schema_version == 1, "unknown plan schema");
    Ok(())
}
fn source_raw<'a>(
    s: &LifecycleSnapshot,
    group: &SelectedProbe,
    f: &'a CapturedExecutionFixture,
) -> Option<&'a serde_json::Value> {
    let address = &s
        .entities
        .iter()
        .find(|e| e.id == group.candidate.entity_id)?
        .token_account;
    let e = f.evidence.get(3)?;
    let pos = e.params[0]
        .as_array()?
        .iter()
        .position(|v| v.as_str() == Some(address.as_str()))?;
    e.result["value"].get(pos)
}
fn read_source_amount(
    s: &LifecycleSnapshot,
    group: &SelectedProbe,
    f: &CapturedExecutionFixture,
) -> Option<String> {
    let raw = source_raw(s, group, f)?;
    decode::decode_token_account(
        raw,
        decode::TOKEN_2022_PROGRAM,
        &s.asset.mint,
        s.mint_config.decimals,
    )
    .ok()
    .map(|a| a.raw_balance)
}

fn validate_manifest(p: &ExpansionPlan, m: &CaptureManifest) -> Result<()> {
    ensure!(
        m.schema_version == 1
            && m.plan_sha256 == digest(p)?
            && m.bindings.len() == p.selected.len(),
        "capture manifest differs from immutable plan"
    );
    for (g, b) in p.selected.iter().zip(&m.bindings) {
        ensure!(
            b.group_id == g.candidate.id
                && b.fixture_file == g.fixture_reference
                && b.fixture_sha256.is_some() != b.error.is_some(),
            "invalid capture group binding"
        );
    }
    Ok(())
}
fn spec(
    s: &LifecycleSnapshot,
    p: &ExpansionPlan,
    g: &SelectedProbe,
    a: &AmountPoint,
    f: &CapturedExecutionFixture,
    i: &VenueInventory,
) -> Result<ExecutionProbeSpec> {
    let context = i
        .contexts
        .iter()
        .find(|c| c.id == g.candidate.context_id)
        .context("context absent")?;
    Ok(ExecutionProbeSpec {
        schema_version: 1,
        id: format!("group-{}-raw-{}", g.selection_order, a.raw),
        path_type: g.candidate.path_type,
        adapter: ProbeAdapter::MeteoraDlmm,
        target_entity: g.candidate.entity_id.clone(),
        pool: context.address.clone(),
        input_mint: s.asset.mint.clone(),
        output_mint: context.output_mint.clone(),
        input_amount_raw: a.raw.clone(),
        minimum_output_raw: "1".into(),
        amount_reason: a.rationale.clone(),
        snapshot_sha256: p.snapshot_sha256.clone(),
        scenario_sha256: String::new(),
        fixture: g.fixture_reference.clone(),
        fixture_sha256: f.sha256()?,
    })
}
pub fn measure(
    s: &LifecycleSnapshot,
    p: &ExpansionPlan,
    g: &SelectedProbe,
    a: &AmountPoint,
    binding: &CaptureBinding,
    f: Option<&CapturedExecutionFixture>,
    i: &VenueInventory,
) -> Result<ExecutionEvidence> {
    let source = s
        .entities
        .iter()
        .find(|e| e.id == g.candidate.entity_id)
        .context("source absent")?;
    let raw = f.and_then(|f| source_raw(s, g, f));
    let before = f.and_then(|f| read_source_amount(s, g, f));
    let authority_raw = f.and_then(|f| {
        let e = f.evidence.get(3)?;
        let n = e.params[0]
            .as_array()?
            .iter()
            .position(|v| v.as_str() == Some(source.state.owner.as_str()))?;
        e.result["value"].get(n)
    });
    let mut evidence = ExecutionEvidence {
        schema_version: 1,
        plan_sha256: digest(p)?,
        group_id: g.candidate.id.clone(),
        case_id: format!("group-{}-raw-{}", g.selection_order, a.raw),
        entity_id: source.id.clone(),
        path_type: g.candidate.path_type,
        context_id: g.candidate.context_id.clone(),
        input_raw: a.raw.clone(),
        invalid_control: a.invalid_control,
        fixture_sha256: binding.fixture_sha256.clone(),
        captured_state_sha256: raw
            .map(|r| digest(&(binding.fixture_sha256.clone(), r)))
            .transpose()?,
        capture_context: binding.capture_context,
        source_before_raw: before.clone(),
        authority: AuthorityAssumption {
            pubkey: source.state.owner.clone(),
            authority_model: type_key(&source.entity_type),
            on_curve: source.authority_observation.is_on_curve,
            captured_runtime_owner: authority_raw
                .and_then(|v| v["owner"].as_str())
                .map(str::to_string),
            captured_executable: authority_raw.and_then(|v| v["executable"].as_bool()),
            signer_possession_known: false,
            signer_assumed_locally: true,
            wording: "original owner locally assumed to sign".into(),
        },
        status: CaseStatus::Indeterminate,
        blocker: binding.error.clone(),
        preconditions: vec![],
        assumptions: vec![],
        vm_clock: None,
        message: None,
        account_evidence: vec![],
        local_accounts: vec![],
        execution: None,
        deltas: None,
        rollback_verified: None,
    };
    let Some(f) = f else {
        return Ok(evidence);
    };
    ensure!(
        binding.fixture_sha256.as_ref() == Some(&f.sha256()?),
        "fixture binding mismatch"
    );
    // The frozen plan is never repaired using later capture facts.
    if a.invalid_control && before.as_ref() != Some(&g.candidate.represented_raw) {
        evidence.blocker=Some("Captured source balance changed; frozen balance+1 control cannot validate the planned bound. No transaction executed.".into());
        return Ok(evidence);
    }
    let amount = a.raw.parse::<u64>()?;
    let planned = match g.candidate.path_type {
        ExitPathType::SecondaryMarketExit => {
            probe::meteora_dlmm::DexSwapExitProbe.build_execution(s, &spec(s, p, g, a, f, i)?, f)
        }
        ExitPathType::Transfer => {
            let context = i
                .contexts
                .iter()
                .find(|c| c.id == g.candidate.context_id)
                .context("context absent")?;
            probe::token_transfer::build(s, &g.candidate.entity_id, &context.address, amount, f)
        }
        _ => {
            evidence.status = CaseStatus::Unsupported;
            evidence.blocker = Some("No executable adapter for requested path".into());
            return Ok(evidence);
        }
    };
    let plan = match planned {
        Ok(p) => p,
        Err(e) => {
            evidence.blocker = Some(format!(
                "Execution precondition rejected before transaction: {e:#}"
            ));
            return Ok(evidence);
        }
    };
    evidence.preconditions = plan.preconditions.clone();
    evidence.assumptions = plan.assumptions.clone();
    // Phase 5 wording is kept in its artifacts; new capture evidence labels its own bank.
    evidence
        .assumptions
        .retain(|a| !a.starts_with("All executable route state uses one Phase 5"));
    evidence.assumptions.push(format!("Independent reset at {:?} bank slot {}. No historical lifecycle bank is implied; no simultaneous venue capacity is measured.",binding.capture_context,plan.clock.slot));
    evidence.vm_clock = Some(ProbeClock::from(&plan.clock));
    evidence.message = Some(ProbeMessage::from(&plan.message));
    evidence.account_evidence = plan.account_evidence.clone();
    evidence.local_accounts = plan
        .accounts
        .iter()
        .filter(|a| !plan.account_evidence.iter().any(|e| e.address == a.address))
        .cloned()
        .collect();
    let x = match crate::executor::execute_probe_message(
        &plan.accounts,
        &plan.watch,
        plan.clock.clone(),
        &plan.programs,
        plan.message.clone(),
    ) {
        Ok(x) => x,
        Err(e) => {
            evidence.blocker = Some(format!("Local runtime setup failed: {e:#}"));
            return Ok(evidence);
        }
    };
    let rollback = plan.watch.iter().all(|a| {
        x.post_accounts.get(a)
            == plan
                .accounts
                .iter()
                .find(|b| b.address == *a)
                .map(|b| &b.account)
    });
    evidence.rollback_verified = if x.success { None } else { Some(rollback) };
    let deltas =
        match g.candidate.path_type {
            ExitPathType::SecondaryMarketExit => probe::meteora_dlmm::DexSwapExitProbe
                .classify_result(&spec(s, p, g, a, f, i)?, &plan, &x),
            ExitPathType::Transfer => {
                let context = i
                    .contexts
                    .iter()
                    .find(|c| c.id == g.candidate.context_id)
                    .context("context absent")?;
                probe::token_transfer::reconcile(
                    s,
                    &g.candidate.entity_id,
                    &context.address,
                    amount,
                    &plan,
                    &x,
                )
            }
            _ => unreachable!(),
        };
    match deltas {
        Ok(d) => {
            evidence.status = if x.success && d.reconciled {
                CaseStatus::Succeeded
            } else if !x.success && rollback {
                CaseStatus::Failed
            } else {
                CaseStatus::Indeterminate
            };
            if evidence.status == CaseStatus::Indeterminate {
                evidence.blocker = Some(
                    "Actual execution lacks economic reconciliation or watched rollback".into(),
                );
            }
            evidence.deltas = Some(d);
        }
        Err(e) => {
            evidence.status = CaseStatus::Indeterminate;
            evidence.blocker = Some(format!("Actual execution reconciliation error: {e:#}"));
        }
    }
    evidence.execution = Some(x.into());
    Ok(evidence)
}
pub fn execute_plan(
    s: &LifecycleSnapshot,
    b: &CoverageReport,
    p: &ExpansionPlan,
    i: &VenueInventory,
    m: &CaptureManifest,
    capture_dir: &Path,
    out: &Path,
) -> Result<ExecutionIndex> {
    check_plan_inputs(s, b, p, i)?;
    validate_manifest(p, m)?;
    ensure!(!out.exists(), "execution output directory already exists");
    std::fs::create_dir_all(out)?;
    let mut refs = Vec::new();
    for (g, binding) in p.selected.iter().zip(&m.bindings) {
        let world = super::discovery::execution_world(s, i, Some(&g.candidate.context_id))?;
        let fixture = if let Some(h) = &binding.fixture_sha256 {
            let bytes = crate::lifecycle::artifact::read(capture_dir.join(&binding.fixture_file))
                .context("captured fixture missing")?;
            ensure!(
                crate::lifecycle::exposure::sha256(&bytes) == *h,
                "fixture file hash mismatch"
            );
            Some(serde_json::from_slice::<CapturedExecutionFixture>(&bytes)?)
        } else {
            None
        };
        for amount in &g.amount_matrix {
            let e = measure(&world, p, g, amount, binding, fixture.as_ref(), i)?;
            let hash = digest(&e)?;
            let file = format!("results/{hash}.json");
            save(&e, &out.join(&file))?;
            refs.push(EvidenceReference {
                group_id: g.candidate.id.clone(),
                case_id: e.case_id,
                result_sha256: hash,
                fixture_sha256: binding.fixture_sha256.clone(),
                result_file: file,
                status: e.status,
            });
        }
    }
    let index = ExecutionIndex {
        schema_version: 1,
        plan_sha256: digest(p)?,
        capture_manifest_sha256: digest(m)?,
        results: refs,
    };
    save(&index, &out.join("execution-index.json"))?;
    Ok(index)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssuranceMetrics {
    pub portfolio: CoverageAggregate,
    pub measured_entities: usize,
    pub measured_authorities: usize,
    pub secondary_market_venues: Vec<String>,
    pub measured_paths: Vec<String>,
    pub measured_classes: Vec<String>,
    pub by_path: BTreeMap<String, CoverageAggregate>,
    pub by_account_type: BTreeMap<String, CoverageAggregate>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealizedGain {
    pub group_id: String,
    pub expected: ExpectedGain,
    pub realized_represented_raw: String,
    pub newly_measured_entities: usize,
    pub new_venue_contexts: usize,
    pub new_path_contexts: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleCoverageDeltaReport {
    pub schema_version: u32,
    pub baseline_sha256: String,
    pub plan_sha256: String,
    pub capture_manifest_sha256: String,
    pub execution_index_sha256: String,
    pub before: AssuranceMetrics,
    pub after: AssuranceMetrics,
    pub represented_raw_gain: String,
    pub entity_updates: Vec<crate::lifecycle::frozen::coverage::EntityCoverage>,
    pub evidence: Vec<EvidenceReference>,
    pub gains: Vec<RealizedGain>,
    pub status_counts: BTreeMap<String, usize>,
    pub remaining_gaps: Vec<CoverageGap>,
    pub added_capabilities: Vec<String>,
    pub second_venue_outcome: String,
    pub official_transition: crate::lifecycle::consequence::LifecycleExecutionStatus,
    pub assumptions: Vec<String>,
}
fn aggregate<'a>(
    entities: impl Iterator<
        Item = (
            &'a crate::lifecycle::frozen::coverage::EntityCoverage,
            CoverageClassification,
            u64,
        ),
    >,
) -> CoverageAggregate {
    let mut a = CoverageAggregate::default();
    for c in [
        CoverageClassification::Proven,
        CoverageClassification::PartiallyProven,
        CoverageClassification::Untested,
        CoverageClassification::Unsupported,
    ] {
        a.classifications.insert(c, 0);
    }
    let mut total = 0u128;
    let mut covered = 0u128;
    let mut owners = BTreeSet::new();
    let mut measured = BTreeSet::new();
    for (e, c, n) in entities {
        let b = e.represented_balance_raw.parse::<u64>().unwrap();
        a.entities += 1;
        a.positive_balance_entities += usize::from(b > 0);
        *a.classifications.entry(c).or_default() += 1;
        owners.insert(&e.owner_authority);
        if n > 0 {
            measured.insert(&e.owner_authority);
        }
        total += u128::from(b);
        covered += u128::from(n);
    }
    a.distinct_owner_authorities = owners.len();
    a.authorities_with_measured_amount = measured.len();
    a.represented_amount_raw = total.to_string();
    a.represented_amount_covered_raw = covered.to_string();
    a.represented_amount_without_evidence_raw = (total - covered).to_string();
    a
}
fn metrics(
    entities: &[crate::lifecycle::frozen::coverage::EntityCoverage],
    measured: &BTreeSet<String>,
    owners: &BTreeSet<String>,
    venues: &BTreeSet<String>,
    paths: &BTreeSet<String>,
    classes: &BTreeSet<String>,
) -> AssuranceMetrics {
    let mut by_path = BTreeMap::new();
    let mut by_type = BTreeMap::new();
    if let Some(e) = entities.first() {
        for path in &e.paths {
            by_path.insert(
                path_key(path.path_type),
                aggregate(entities.iter().map(|e| {
                    let p = e
                        .paths
                        .iter()
                        .find(|p| p.path_type == path.path_type)
                        .unwrap();
                    (
                        e,
                        p.classification,
                        p.represented_amount_covered_raw.parse().unwrap(),
                    )
                })),
            );
        }
    }
    for kind in entities
        .iter()
        .map(|e| &e.account_type)
        .collect::<BTreeSet<_>>()
    {
        by_type.insert(
            kind.clone(),
            aggregate(
                entities
                    .iter()
                    .filter(|e| &e.account_type == kind)
                    .map(|e| {
                        (
                            e,
                            e.classification,
                            e.represented_amount_covered_raw.parse().unwrap(),
                        )
                    }),
            ),
        );
    }
    AssuranceMetrics {
        portfolio: aggregate(entities.iter().map(|e| {
            (
                e,
                e.classification,
                e.represented_amount_covered_raw.parse().unwrap(),
            )
        })),
        measured_entities: measured.len(),
        measured_authorities: owners.len(),
        secondary_market_venues: venues.iter().cloned().collect(),
        measured_paths: paths.iter().cloned().collect(),
        measured_classes: classes.iter().cloned().collect(),
        by_path,
        by_account_type: by_type,
    }
}
/// Results must be fresh-replayed before use. The update never changes selection.
pub fn update(
    s: &LifecycleSnapshot,
    b: &CoverageReport,
    p: &ExpansionPlan,
    i: &VenueInventory,
    m: &CaptureManifest,
    artifact_dirs: (&Path, &Path),
    index: &ExecutionIndex,
) -> Result<LifecycleCoverageDeltaReport> {
    let (capture_dir, result_dir) = artifact_dirs;
    check_plan_inputs(s, b, p, i)?;
    validate_manifest(p, m)?;
    ensure!(
        index.schema_version == 1
            && index.plan_sha256 == digest(p)?
            && index.capture_manifest_sha256 == digest(m)?
            && index.results.len()
                == p.selected
                    .iter()
                    .map(|g| g.amount_matrix.len())
                    .sum::<usize>(),
        "execution index differs from selected plan"
    );
    let mut verified = Vec::new();
    let mut ref_iter = index.results.iter();
    for (g, binding) in p.selected.iter().zip(&m.bindings) {
        let world = super::discovery::execution_world(s, i, Some(&g.candidate.context_id))?;
        let fixture = if let Some(h) = &binding.fixture_sha256 {
            let bytes = crate::lifecycle::artifact::read(capture_dir.join(&binding.fixture_file))?;
            ensure!(
                crate::lifecycle::exposure::sha256(&bytes) == *h,
                "fixture digest mismatch"
            );
            Some(serde_json::from_slice::<CapturedExecutionFixture>(&bytes)?)
        } else {
            None
        };
        for amount in &g.amount_matrix {
            let r = ref_iter.next().context("missing execution reference")?;
            let bytes = crate::lifecycle::artifact::read(result_dir.join(&r.result_file))?;
            ensure!(
                crate::lifecycle::exposure::sha256(&bytes) == r.result_sha256,
                "result file digest mismatch"
            );
            let e: ExecutionEvidence = serde_json::from_slice(&bytes)?;
            ensure!(
                digest(&e)? == r.result_sha256
                    && e.case_id == r.case_id
                    && e.group_id == r.group_id
                    && e.fixture_sha256 == r.fixture_sha256
                    && e.status == r.status,
                "result binding mismatch"
            );
            ensure!(
                e == measure(&world, p, g, amount, binding, fixture.as_ref(), i)?,
                "result disagrees with fresh offline execution"
            );
            verified.push(e);
        }
    }
    merge_verified(b, p, i, m, index, &verified)
}
// Kept crate-private: public callers must verify result files and fresh execution.
pub(crate) fn merge_verified(
    b: &CoverageReport,
    p: &ExpansionPlan,
    i: &VenueInventory,
    m: &CaptureManifest,
    index: &ExecutionIndex,
    verified: &[ExecutionEvidence],
) -> Result<LifecycleCoverageDeltaReport> {
    let mut entities = b.entities.clone();
    let positions: BTreeMap<_, _> = entities
        .iter()
        .enumerate()
        .map(|(n, e)| (e.entity_id.clone(), n))
        .collect();
    let mut measured = BTreeSet::new();
    let mut owners = BTreeSet::new();
    let mut venues = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut classes = BTreeSet::new();
    for e in &entities {
        if e.represented_amount_covered_raw != "0" {
            measured.insert(e.entity_id.clone());
            owners.insert(e.owner_authority.clone());
            classes.insert(e.representative_class.clone());
        }
    }
    for c in &b.cases {
        if c.status == CaseStatus::Succeeded {
            paths.insert(path_key(c.case.path_type));
            if c.case.path_type == ExitPathType::SecondaryMarketExit {
                if let Some(v) = &c.case.venue {
                    venues.insert(v.clone());
                }
            }
        }
    }
    let before = metrics(&entities, &measured, &owners, &venues, &paths, &classes);
    for row in &mut entities {
        if row.account_type == "WalletCompatible" {
            if let Some(path) = row
                .paths
                .iter_mut()
                .find(|p| p.path_type == ExitPathType::Transfer)
            {
                if path.classification == CoverageClassification::Unsupported {
                    path.classification = CoverageClassification::Untested;
                    path.reason =
                        "Phase 7 transfer adapter exists; no execution evidence is inferred."
                            .into();
                }
            }
        }
    }
    let mut gains = Vec::new();
    let mut statuses = BTreeMap::new();
    let mut changed = BTreeSet::new();
    for group in &p.selected {
        let amount_before = aggregate(entities.iter().map(|e| {
            (
                e,
                e.classification,
                e.represented_amount_covered_raw.parse().unwrap(),
            )
        }))
        .represented_amount_covered_raw
        .parse::<u128>()?;
        let n_before = measured.len();
        let v_before = venues.len();
        let paths_before = paths.len();
        for e in verified.iter().filter(|e| e.group_id == group.candidate.id) {
            *statuses.entry(format!("{:?}", e.status)).or_default() += 1;
            if e.status != CaseStatus::Succeeded || e.invalid_control {
                continue;
            }
            let d = e.deltas.as_ref().context("success without deltas")?;
            ensure!(
                d.reconciled
                    && e.execution.as_ref().is_some_and(|x| x.success)
                    && d.input_debited_raw == e.input_raw,
                "success without exact measured reconciliation"
            );
            let pos = *positions
                .get(&e.entity_id)
                .context("measured entity absent")?;
            let row = &mut entities[pos];
            measured.insert(e.entity_id.clone());
            owners.insert(row.owner_authority.clone());
            classes.insert(row.representative_class.clone());
            paths.insert(path_key(e.path_type));
            if e.path_type == ExitPathType::SecondaryMarketExit {
                venues.insert(e.context_id.clone());
            }
            if e.source_before_raw.as_ref() != Some(&row.represented_balance_raw) {
                continue;
            }
            let balance = row.represented_balance_raw.parse::<u64>()?;
            let path = row
                .paths
                .iter_mut()
                .find(|p| p.path_type == e.path_type)
                .context("path not in baseline requests")?;
            let n = d.input_debited_raw.parse::<u64>()?;
            let old = path.represented_amount_covered_raw.parse::<u64>()?;
            let max = old.max(n);
            ensure!(
                max <= balance,
                "amount evidence exceeds represented balance"
            );
            path.largest_successful_input_raw = path
                .largest_successful_input_raw
                .parse::<u64>()?
                .max(n)
                .to_string();
            path.represented_amount_covered_raw = max.to_string();
            path.represented_amount_without_evidence_raw = (balance - max).to_string();
            path.classification = classify(balance, max, false);
            path.successful_cases.push(e.case_id.clone());
            path.reason="Direct measured exact input at matching captured source balance. Independent cases use maxima, never interpolation or simultaneous capacity.".into();
            let covered = row
                .paths
                .iter()
                .map(|p| p.represented_amount_covered_raw.parse::<u64>().unwrap())
                .max()
                .unwrap_or(0);
            row.represented_amount_covered_raw = covered.to_string();
            row.represented_amount_without_evidence_raw = (balance - covered).to_string();
            row.classification = classify(balance, covered, false);
            changed.insert(pos);
        }
        let amount_after = aggregate(entities.iter().map(|e| {
            (
                e,
                e.classification,
                e.represented_amount_covered_raw.parse().unwrap(),
            )
        }))
        .represented_amount_covered_raw
        .parse::<u128>()?;
        gains.push(RealizedGain {
            group_id: group.candidate.id.clone(),
            expected: group.expected_gain.clone(),
            realized_represented_raw: (amount_after - amount_before).to_string(),
            newly_measured_entities: measured.len() - n_before,
            new_venue_contexts: venues.len() - v_before,
            new_path_contexts: paths.len() - paths_before,
        });
    }
    let after = metrics(&entities, &measured, &owners, &venues, &paths, &classes);
    let amount_gain = after
        .portfolio
        .represented_amount_covered_raw
        .parse::<u128>()?
        - before
            .portfolio
            .represented_amount_covered_raw
            .parse::<u128>()?;
    let mut remaining_gaps = p.gaps.clone();
    for gap in &mut remaining_gaps {
        let amount = entities
            .iter()
            .filter(|e| e.representative_class == gap.account_class)
            .filter_map(|e| e.paths.iter().find(|path| path.path_type == gap.path_type))
            .map(|path| path.represented_amount_covered_raw.parse::<u128>().unwrap())
            .sum::<u128>();
        gap.already_measured_raw = amount.to_string();
        gap.without_evidence_raw = (gap.represented_raw.parse::<u128>()? - amount).to_string();
    }
    let second_venue_outcome = if after.secondary_market_venues.len() > 1 {
        "SecondVenueExecuted".into()
    } else if i.additional_venue_runs.is_empty() {
        i.second_venue_outcome.clone()
    } else {
        "SecondVenueVerifiedButNoSuccessfulExecution".into()
    };
    Ok(LifecycleCoverageDeltaReport {
        schema_version: 2,
        baseline_sha256: p.coverage_sha256.clone(),
        plan_sha256: digest(p)?,
        capture_manifest_sha256: digest(m)?,
        execution_index_sha256: digest(index)?,
        before, after,
        represented_raw_gain: amount_gain.to_string(),
        entity_updates: changed.into_iter().map(|n|entities[n].clone()).collect(),
        evidence: index.results.clone(), gains, status_counts: statuses,
        remaining_gaps,
        added_capabilities: vec!["WalletCompatible + Token-2022 Transfer; capability changes prior Unsupported to Untested, never positive proof".into()],
        second_venue_outcome,
        official_transition: crate::lifecycle::consequence::LifecycleExecutionStatus::NotTested,
        assumptions: vec![
            "Population assurance uses the unchanged Phase 6 maximum/exact-input rules and matching source balances. Current-state measured entities/context points remain separate when captures differ.".into(),
            "Original owner locally assumed to sign. Signer possession/authorization, inclusion, future exit, lifecycle entitlement and full validator bank fidelity are not proved.".into(),
            "Independent amounts/venues/paths reset captured state. Use per-entity maxima and exact points, never liquidity sums, interpolation, class-peer or venue-brand inheritance.".into(),
            "Transfer is actual token movement only; it does not prove sale, redemption, LP/vault withdrawal or OfficialTransition.".into(),
            "Remaining gaps recompute measured/without-evidence amounts from updated individual paths. Unsupported gaps remain development guidance, never assurance.".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    struct Corpus {
        baseline: CoverageReport,
        plan: ExpansionPlan,
        inventory: VenueInventory,
        manifest: CaptureManifest,
        index: ExecutionIndex,
        evidence: Vec<ExecutionEvidence>,
    }
    fn corpus() -> &'static Corpus {
        static C: OnceLock<Corpus> = OnceLock::new();
        C.get_or_init(|| {
            let root = crate::lifecycle::artifact::reference_root();
            let mut baseline: CoverageReport =
                super::super::load(&root.join("reports/spacex-lifecycle-coverage.json")).unwrap();
            let mut plan: ExpansionPlan =
                super::super::load(&root.join("probes/spacex-lifecycle-expansion-plan.json"))
                    .unwrap();
            let inventory =
                super::super::load(&root.join("probes/spacex-phase7-venues-verified.json"))
                    .unwrap();
            let manifest =
                super::super::load(&root.join("probes/phase7-captures/capture-manifest.json"))
                    .unwrap();
            let index: ExecutionIndex =
                super::super::load(&root.join("reports/phase7-evidence/execution-index.json"))
                    .unwrap();
            let evidence = index
                .results
                .iter()
                .map(|r| {
                    super::super::load(&root.join("reports/phase7-evidence").join(&r.result_file))
                        .unwrap()
                })
                .collect();
            let selected: BTreeSet<_> = plan
                .selected
                .iter()
                .map(|g| g.candidate.entity_id.as_str())
                .collect();
            // Small private reducer corpus: actual measured evidence and real rows,
            // plus one untested peer. Public update validates the full population.
            let peer = baseline
                .entities
                .iter()
                .find(|e| {
                    e.account_type == "WalletCompatible"
                        && e.represented_balance_raw != "0"
                        && !selected.contains(e.entity_id.as_str())
                        && e.classification == CoverageClassification::Untested
                })
                .unwrap()
                .entity_id
                .clone();
            baseline.entities.retain(|e| {
                selected.contains(e.entity_id.as_str())
                    || e.classification == CoverageClassification::Proven
                    || e.entity_id == peer
            });
            plan.candidates.clear(); // Hash is local to pure reducer facts, not production.
            Corpus {
                baseline,
                plan,
                inventory,
                manifest,
                index,
                evidence,
            }
        })
    }
    fn reduce(e: &[ExecutionEvidence]) -> LifecycleCoverageDeltaReport {
        let c = corpus();
        merge_verified(&c.baseline, &c.plan, &c.inventory, &c.manifest, &c.index, e).unwrap()
    }
    #[test]
    fn independent_swaps_are_maxima_never_summed_capacity() {
        let c = corpus();
        let r = reduce(&c.evidence);
        let mut max: BTreeMap<String, u64> = BTreeMap::new();
        for e in &c.evidence {
            if e.status == CaseStatus::Succeeded
                && !e.invalid_control
                && e.source_before_raw.as_ref()
                    == c.baseline
                        .entities
                        .iter()
                        .find(|row| row.entity_id == e.entity_id)
                        .map(|row| &row.represented_balance_raw)
            {
                max.entry(e.entity_id.clone())
                    .and_modify(|n| *n = (*n).max(e.input_raw.parse().unwrap()))
                    .or_insert(e.input_raw.parse().unwrap());
            }
        }
        let mut expected = c
            .baseline
            .entities
            .iter()
            .map(|row| row.represented_amount_covered_raw.parse::<u128>().unwrap())
            .sum::<u128>();
        for (id, n) in max {
            let old = c
                .baseline
                .entities
                .iter()
                .find(|e| e.entity_id == id)
                .unwrap()
                .represented_amount_covered_raw
                .parse::<u64>()
                .unwrap();
            expected += u128::from(n.saturating_sub(old));
        }
        assert_eq!(
            r.after.portfolio.represented_amount_covered_raw,
            expected.to_string()
        );
    }
    #[test]
    fn a_class_sample_never_proves_its_peers() {
        let c = corpus();
        let r = reduce(&c.evidence);
        let selected: BTreeSet<_> = c
            .plan
            .selected
            .iter()
            .map(|g| g.candidate.entity_id.as_str())
            .collect();
        assert!(r
            .entity_updates
            .iter()
            .all(|e| selected.contains(e.entity_id.as_str())));
        assert_eq!(r.after.portfolio.entities, c.baseline.entities.len());
    }
    #[test]
    fn second_venue_has_no_brand_or_first_venue_inheritance() {
        let c = corpus();
        let mut evidence = c.evidence.clone();
        evidence.retain(|e| e.path_type == ExitPathType::Transfer);
        let r = reduce(&evidence);
        let before: BTreeSet<_> = c
            .baseline
            .cases
            .iter()
            .filter(|c| {
                c.status == CaseStatus::Succeeded
                    && c.case.path_type == ExitPathType::SecondaryMarketExit
            })
            .filter_map(|c| c.case.venue.clone())
            .collect();
        assert_eq!(
            r.after.secondary_market_venues,
            before.into_iter().collect::<Vec<_>>()
        );
    }
    #[test]
    fn execution_never_rewrites_expected_selection_gains() {
        let c = corpus();
        let r = reduce(&c.evidence);
        for (g, realized) in c.plan.selected.iter().zip(&r.gains) {
            assert_eq!(g.expected_gain, realized.expected);
        }
        assert_eq!(r.plan_sha256, digest(&c.plan).unwrap());
    }
    #[test]
    fn distinct_accounts_with_one_authority_stay_distinct() {
        let c = corpus();
        let rows = &c.baseline.entities;
        let measured = rows.iter().take(2).map(|e| e.entity_id.clone()).collect();
        let owners = ["one-authority".into()].into();
        let m = metrics(
            rows,
            &measured,
            &owners,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );
        assert_eq!(m.measured_entities, 2);
        assert_eq!(m.measured_authorities, 1);
        assert_eq!(m.portfolio.entities, rows.len());
    }
    #[test]
    fn failed_indeterminate_unsupported_and_controls_gain_nothing() {
        let c = corpus();
        for status in [
            CaseStatus::Failed,
            CaseStatus::Indeterminate,
            CaseStatus::Unsupported,
            CaseStatus::Untested,
        ] {
            let mut e = c.evidence.clone();
            for row in &mut e {
                row.status = status;
            }
            let r = reduce(&e);
            assert_eq!(r.represented_raw_gain, "0");
            assert_eq!(r.after.measured_entities, r.before.measured_entities);
        }
        let mut e = c.evidence.clone();
        for row in &mut e {
            row.invalid_control = true;
        }
        assert_eq!(reduce(&e).represented_raw_gain, "0");
    }
    #[test]
    fn smaller_repeated_points_and_larger_points_add_only_difference() {
        let c = corpus();
        let full = reduce(&c.evidence);
        let mut repeated = c.evidence.clone();
        repeated.extend(c.evidence.iter().cloned());
        let more = reduce(&repeated);
        assert_eq!(
            full.after.portfolio.represented_amount_covered_raw,
            more.after.portfolio.represented_amount_covered_raw
        );
        assert_eq!(full.after.measured_entities, more.after.measured_entities);
        let mut successes: Vec<_> = c
            .evidence
            .iter()
            .filter(|e| e.status == CaseStatus::Succeeded && !e.invalid_control)
            .cloned()
            .collect();
        successes.sort_by_key(|e| e.input_raw.parse::<u64>().unwrap());
        let one = reduce(&successes[..1]);
        assert!(
            one.after
                .portfolio
                .represented_amount_covered_raw
                .parse::<u128>()
                .unwrap()
                <= full
                    .after
                    .portfolio
                    .represented_amount_covered_raw
                    .parse()
                    .unwrap()
        );
    }
    #[test]
    fn official_transition_and_redemption_cannot_inherit_swap_or_transfer() {
        let c = corpus();
        let r = reduce(&c.evidence);
        assert_eq!(
            r.after.by_path["OfficialTransition"].represented_amount_covered_raw,
            "0"
        );
        assert_eq!(
            r.after.by_path["Redemption"].represented_amount_covered_raw,
            "0"
        );
        assert_eq!(
            r.after.by_path["Withdrawal"].represented_amount_covered_raw,
            "0"
        );
        assert!(r.entity_updates.iter().all(|e| e
            .paths
            .iter()
            .filter(|p| p.path_type == ExitPathType::OfficialTransition)
            .all(|p| p.classification == CoverageClassification::Unsupported)));
        assert_eq!(
            serde_json::to_value(r.official_transition).unwrap(),
            "NotTested"
        );
    }
    #[test]
    fn current_capture_cannot_be_labeled_a_historical_bank() {
        assert_eq!(
            serde_json::to_value(CaptureContext::CurrentFinalizedProduction).unwrap(),
            "CurrentFinalizedProduction"
        );
        assert!(serde_json::from_str::<CaptureContext>("\"HistoricalLifecycleBank\"").is_err());
        let c = corpus();
        assert!(c
            .evidence
            .iter()
            .all(|e| e.capture_context == CaptureContext::CurrentFinalizedProduction));
    }
    #[test]
    fn result_bindings_preserve_immutable_preexecution_plan() {
        let c = corpus();
        let root = crate::lifecycle::artifact::reference_root();
        let plan: ExpansionPlan =
            super::super::load(&root.join("probes/spacex-lifecycle-expansion-plan.json")).unwrap();
        let hash = digest(&plan).unwrap();
        assert!(c.evidence.iter().all(|e| e.plan_sha256 == hash
            && plan.selected.iter().any(|g| g.candidate.id == e.group_id
                && g.amount_matrix
                    .iter()
                    .any(|a| a.raw == e.input_raw && a.invalid_control == e.invalid_control))));
    }
}
