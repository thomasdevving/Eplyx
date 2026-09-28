//! Bounded schema-1 qualification. Coordinates existing evidence predicates.
mod contract;
use crate::{
    bundle,
    corpus_store::CorpusStore,
    discovery,
    historical::{HistoricalStateProvider, ProtocolArchiveProvider},
    ingest::{self, rpc::RpcProvider},
    replay::{hash_bytes, ReplayRecord},
    select,
};
use anyhow::{Context, Result};
pub use contract::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub struct Providers<'a> {
    pub transactions: &'a dyn RpcProvider,
    pub accounts: &'a dyn RpcProvider,
    pub blocks: &'a dyn RpcProvider,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub signature: String,
    pub slot: u64,
    pub eligibility: Option<discovery::ReplayEligibility>,
    pub reasons: Vec<String>,
    pub attempted: bool,
    pub result: String,
    pub rejection: Option<Code>,
    pub record_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub schema_version: u32,
    pub tool_version: String,
    pub tool_commit: String,
    pub scope: Option<Scope>,
    pub adapter: Option<Value>,
    pub genesis: Option<String>,
    pub capabilities: BTreeMap<String, String>,
    pub discovery: Option<Value>,
    pub ledger: Vec<LedgerEntry>,
    pub acquisition_counts: BTreeMap<String, usize>,
    pub cohorts: BTreeMap<String, Value>,
    pub acquired_actions: BTreeMap<String, usize>,
    pub evaluable_subjects: BTreeMap<String, Vec<String>>,
    pub observed_semantic_population: Value,
    pub selection: Option<select::SelectedCorpus>,
    pub fidelity: Vec<bundle::FidelityOutcome>,
    pub bundle: Option<bundle::BundleManifest>,
    pub bundle_verified: bool,
    pub coverage_acceptance_requested: bool,
    pub coverage_accepted: bool,
    pub limitations: Vec<bundle::BundledLimitation>,
    pub states: Vec<State>,
    pub state: State,
    pub blockers: Vec<Code>,
    pub next_action: String,
    pub exit_code: u8,
}
impl Receipt {
    fn new() -> Self {
        Self {
            schema_version: 1,
            tool_version: crate::build_info::VERSION.into(),
            tool_commit: crate::build_info::COMMIT.into(),
            scope: None,
            adapter: None,
            genesis: None,
            capabilities: BTreeMap::new(),
            discovery: None,
            ledger: vec![],
            acquisition_counts: BTreeMap::new(),
            cohorts: BTreeMap::new(),
            acquired_actions: BTreeMap::new(),
            evaluable_subjects: BTreeMap::new(),
            observed_semantic_population: json!({"status":"unknown"}),
            selection: None,
            fidelity: vec![],
            bundle: None,
            bundle_verified: false,
            coverage_acceptance_requested: false,
            coverage_accepted: false,
            limitations: vec![],
            states: vec![State::ScopeUnqualified],
            state: State::ScopeUnqualified,
            blockers: vec![],
            next_action: "qualify the declared scope".into(),
            exit_code: 24,
        }
    }
    fn advance(&mut self, state: State) {
        self.state = state;
        self.states.push(state);
    }
    fn save(&mut self, out: &Path) -> Result<()> {
        self.acquisition_counts = [
            ("considered", self.ledger.len()),
            (
                "attempted",
                self.ledger.iter().filter(|e| e.attempted).count(),
            ),
            (
                "accepted",
                self.ledger.iter().filter(|e| e.record_id.is_some()).count(),
            ),
            (
                "refused",
                self.ledger.iter().filter(|e| e.rejection.is_some()).count(),
            ),
            (
                "budget_excluded",
                self.ledger
                    .iter()
                    .filter(|e| e.result == "budget_excluded")
                    .count(),
            ),
            (
                "not_shortlisted",
                self.ledger
                    .iter()
                    .filter(|e| e.result == "not_shortlisted")
                    .count(),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect();
        ingest::write_json(&out.join("receipt.json"), self)
    }
    fn limit(&mut self, code: &str, detail: &str) {
        let limitation = bundle::BundledLimitation {
            code: code.into(),
            detail: detail.into(),
        };
        if !self.limitations.contains(&limitation) {
            self.limitations.push(limitation);
        }
    }
}

/// A fresh root is mandatory. Provider calls happen only after local preflight.
/// Raw errors are never serialized or printed by this boundary.
pub fn prepare(config: &Config, out: &Path, providers: Providers<'_>) -> Result<Receipt> {
    std::fs::create_dir(out).context("qualification requires a fresh output directory")?;
    let mut receipt = Receipt::new();
    receipt.coverage_acceptance_requested = config.accept_coverage;
    receipt.save(out)?;
    if let Err(error) = pipeline(config, out, providers, &mut receipt) {
        let code = error
            .downcast_ref::<Code>()
            .copied()
            .unwrap_or(Code::InternalFailure);
        receipt.blockers.push(code);
        let (state, exit) = code.failure_state();
        receipt.advance(state);
        receipt.exit_code = exit;
        receipt.next_action = code.next_action().into();
    }
    ingest::write_json(&out.join("acquisition-ledger.json"), &receipt.ledger)?;
    receipt.save(out)?;
    Ok(receipt)
}

fn pipeline(
    config: &Config,
    out: &Path,
    providers: Providers<'_>,
    receipt: &mut Receipt,
) -> Result<()> {
    if safe_scope(&config.scope) {
        receipt.scope = Some(config.scope.clone());
    }
    if let Some(path) = &config.prepared_corpus {
        let manifest: Value =
            ingest::read_json(&path.join("manifest.json")).context(Code::InvalidConfiguration)?;
        return Err(if manifest["schema_version"] == 2 {
            Code::OutsideSchema1QualificationContract
        } else {
            Code::PreparedCorpusOutsideAcquisitionContract
        }
        .into());
    }
    let adapter = preflight(config)?;
    receipt.adapter = Some(json!({"name":adapter.name(),"version":adapter.adapter_version()}));
    let observed = match &config.observed {
        Some(path) => {
            let measured: MeasuredPopulation =
                ingest::read_json(path).context(Code::InvalidObservedPopulation)?;
            measured.validate(&config.scope, adapter)?;
            receipt.limit("external_semantic_measurement", "Semantic classifier provenance and completeness are caller declarations; binding is checked but the measurement is not independently verified.");
            if matches!(measured.completeness, Completeness::PartialDeclaredWindow) {
                receipt.limit("partial_semantic_population", "The supplied semantic measurement declares only partial coverage of the window.");
            }
            receipt.observed_semantic_population =
                json!({"status":"measured", "artifact":measured});
            measured.counts
        }
        None => {
            receipt.limit(
                "observed_semantic_population_unknown",
                "No independently measured semantic population was supplied; absence is not zero.",
            );
            BTreeMap::new()
        }
    };
    receipt.save(out)?;
    for (name, rpc) in [
        ("transaction_genesis", providers.transactions),
        ("account_genesis", providers.accounts),
        ("block_genesis", providers.blocks),
    ] {
        receipt
            .capabilities
            .insert(name.into(), "probe_started".into());
        receipt.save(out)?;
        ingest::rpc::require_mainnet_genesis(rpc).context(Code::ProviderGenesisFailed)?;
        receipt
            .capabilities
            .insert(name.into(), "mainnet_matched".into());
    }
    receipt.genesis = Some(ingest::rpc::MAINNET_GENESIS.into());
    let scope = &config.scope;
    receipt
        .capabilities
        .insert("transaction_history".into(), "probe_started".into());
    receipt.save(out)?;
    // No current-account sampling is needed to rank transaction structure.
    let mut manifest = ingest::discover_bounded(
        providers.transactions,
        &scope.program,
        scope.start_slot,
        scope.end_slot,
        Some(scope.discovery_limit),
    )
    .context(Code::DiscoveryFailed)?;
    // Unreadable RPC diagnostics are not evidence and may contain provider data.
    for unreadable in &mut manifest.unreadable_transactions {
        unreadable.reason = "unsupported_transaction_version".into();
    }
    let discovery = discovery::build(
        &manifest,
        discovery::SelectionPolicy {
            max_records: scope.discovery_limit as u64,
            ..Default::default()
        },
        None,
        1,
        0,
        |_| None,
    )
    .context(Code::DiscoveryFailed)?;
    ingest::write_json(&out.join("discovery/manifest.json"), &manifest)?;
    ingest::write_json(&out.join("discovery/corpus.json"), &discovery)?;
    std::fs::write(
        out.join("discovery/report.txt"),
        discovery::render_text(&discovery),
    )?;
    receipt.capabilities.insert(
        "transaction_history".into(),
        "bounded_scan_completed".into(),
    );
    receipt.discovery = Some(
        json!({"sha256":hash_bytes(&serde_json::to_vec(&discovery)?), "requested_start":scope.start_slot,"requested_end":scope.end_slot,
        "first_normalized_slot":manifest.transactions.first().map(|t|t.slot),"last_normalized_slot":manifest.transactions.last().map(|t|t.slot),
        "statistics":discovery.statistics, "unreadable":manifest.unreadable_transactions.len(),
        "direct_interactions":manifest.transactions.iter().filter(|t|discovery::interaction_type(t,&scope.program)==discovery::InteractionType::DirectInteraction).count()}),
    );
    let chosen: BTreeMap<_, _> = discovery
        .selected
        .iter()
        .map(|s| (s.interaction.signature.as_str(), s))
        .collect();
    for tx in &manifest.transactions {
        let code = transaction_blocker(tx, adapter);
        let selected = chosen.get(tx.signature.as_str());
        receipt.ledger.push(LedgerEntry {
            signature: tx.signature.clone(),
            slot: tx.slot,
            eligibility: Some(discovery::replay_eligibility(tx, &scope.program, None)),
            reasons: selected
                .map(|s| s.selection_reasons.clone())
                .unwrap_or_else(|| vec!["outside_structural_shortlist".into()]),
            attempted: false,
            result: if code.is_some() {
                "refused"
            } else if selected.is_some() {
                "queued"
            } else {
                "not_shortlisted"
            }
            .into(),
            rejection: code,
            record_id: None,
        });
    }
    for tx in &manifest.unreadable_transactions {
        receipt.ledger.push(LedgerEntry {
            signature: tx.signature.clone(),
            slot: tx.slot,
            eligibility: None,
            reasons: vec!["observed_but_not_normalized".into()],
            attempted: false,
            result: "refused".into(),
            rejection: Some(Code::UnsupportedTransaction),
            record_id: None,
        });
    }
    // The structural selector's stable ranking is the queue order. Every other
    // observed signature remains in the ledger, including budget exclusions.
    let queue: Vec<String> = discovery
        .selected
        .iter()
        .map(|s| s.interaction.signature.clone())
        .collect();
    ingest::write_json(&out.join("candidate-queue.json"), &queue)?;
    let store = CorpusStore::open(out.join("acquired"))?;
    let mut records = Vec::new();
    let mut attempts = 0;
    let provider = ProtocolArchiveProvider {
        transaction_rpc: providers.transactions,
        account_archive_rpc: providers.accounts,
        block_rpc: Some(providers.blocks),
        program_id: &scope.program,
    };
    receipt
        .capabilities
        .insert("exact_slot_accounts".into(), "not_yet_tested".into());
    receipt
        .capabilities
        .insert("block_history".into(), "not_yet_tested".into());
    receipt.save(out)?;
    for signature in queue {
        let index = receipt
            .ledger
            .iter()
            .position(|e| e.signature == signature)
            .expect("queue from manifest");
        if receipt.ledger[index].rejection.is_some() {
            continue;
        }
        if attempts >= scope.acquisition_budget {
            receipt.ledger[index].result = "budget_excluded".into();
            continue;
        }
        attempts += 1;
        receipt.ledger[index].attempted = true;
        receipt.ledger[index].result = "attempt_started".into();
        receipt.save(out)?;
        match provider.acquire_exact(&signature) {
            Err(error) => {
                let code = acquisition_code(&error);
                receipt.ledger[index].result = "refused".into();
                receipt.ledger[index].rejection = Some(code);
            }
            Ok(acquired) => {
                let record = &acquired.record;
                // Bind discovery and finalized acquisition, not just the signature.
                if record.transaction.slot != receipt.ledger[index].slot
                    || record.genesis_hash != ingest::rpc::MAINNET_GENESIS
                    || record.program_id != scope.program
                {
                    receipt.ledger[index].result = "refused".into();
                    receipt.ledger[index].rejection = Some(Code::EvidenceIdentityMismatch);
                } else {
                    store.insert(record)?;
                    store.publish()?;
                    persist_binary(out, &record.current_program_sha256, &acquired.v1_program)?;
                    for bytes in acquired.dependency_binaries.values() {
                        persist_binary(out, &hash_bytes(bytes), bytes)?;
                    }
                    receipt.ledger[index].result = "acquired".into();
                    receipt.ledger[index].record_id = Some(record.id.clone());
                    receipt.capabilities.insert(
                        "exact_slot_accounts".into(),
                        "demonstrated_for_acquired_records_only".into(),
                    );
                    receipt.capabilities.insert(
                        "block_history".into(),
                        "screened_acquired_slots_only".into(),
                    );
                    records.push(acquired.record);
                }
            }
        }
        receipt.save(out)?;
    }
    ingest::write_json(&out.join("acquisition-ledger.json"), &receipt.ledger)?;
    if records.is_empty() {
        return Err(Code::NoExactHistoricalRecords.into());
    }
    receipt.advance(State::EvidenceAcquired);
    receipt.limit("acquisition_losses", "Refused, unreadable, non-shortlisted and budget-excluded observations remain in the ledger; they are not absent production activity.");
    receipt.limit("schema1_runtime_assumption", "LiteSVM mainnet profile; no exact historical feature inventory. V1 fidelity bounds only the selected observations.");
    receipt.limit("selector_protocol_specific_dimensions", "The existing selector's legacy pool and amount features use Stake Pool keys; novelty is not uniformly protocol-neutral.");
    // Every acquired record participates in cohort validation before any scope filtering.
    for record in &records {
        let cohort = cohort(record)?;
        let key = hash_bytes(&serde_json::to_vec(&cohort)?);
        receipt.cohorts.entry(key).or_insert(cohort);
    }
    receipt.save(out)?;
    if receipt.cohorts.len() != 1 {
        return Err(Code::IncompatibleEvidenceCohorts.into());
    }
    let mut eligible = Vec::new();
    for record in &records {
        let action = adapter.semantic_action(&record.transaction).as_str();
        *receipt.acquired_actions.entry(action.into()).or_default() += 1;
        let subjects: Vec<String> = adapter
            .evaluable_subjects(&record.transaction, &record.accounts)
            .iter()
            .map(ToString::to_string)
            .collect();
        receipt
            .evaluable_subjects
            .insert(record.id.clone(), subjects.clone());
        if scope.actions.iter().any(|a| a == action) {
            let requested: Vec<_> = scope
                .subjects
                .iter()
                .filter(|s| subject_action(adapter.name(), s.action.as_str()) == Some(action))
                .collect();
            if requested.is_empty() || requested.iter().any(|s| !subjects.contains(&s.to_string()))
            {
                return Err(Code::MissingEvaluableSubject.into());
            }
            eligible.push(record.clone());
        } else {
            receipt.limit("acquired_action_outside_scope", "Acquired actions outside the declared scope were retained but excluded from selection.");
        }
    }
    if eligible.is_empty() {
        return Err(Code::NoScopedRecords.into());
    }
    for action in &scope.actions {
        if !eligible
            .iter()
            .any(|r| adapter.semantic_action(&r.transaction).as_str() == action)
        {
            receipt.limit(
                &format!("{action}_has_no_replayable_observations"),
                "Declared action has no acquired record in this window.",
            );
        }
    }
    receipt.limit("selector_population_is_acquired", "The selector's legacy replay_eligible population counts describe acquired records before fidelity; only the per-record V1 outcomes establish baseline matches.");
    let selected =
        select::select(&eligible, scope.target_size, &observed).context(Code::SelectionFailed)?;
    let keep: BTreeSet<_> = selected.selected.iter().map(|r| r.id.as_str()).collect();
    let selected_records: Vec<_> = eligible
        .into_iter()
        .filter(|r| keep.contains(r.id.as_str()))
        .collect();
    for limitation in &selected.limitations {
        receipt.limit(&limitation.code, &limitation.detail);
    }
    let covered: BTreeSet<String> = selected_records
        .iter()
        .flat_map(|r| receipt.evaluable_subjects[&r.id].clone())
        .collect();
    let missing_subject = scope
        .subjects
        .iter()
        .any(|s| !covered.contains(&s.to_string()));
    if missing_subject {
        receipt.limit("declared_subject_not_selected", "At least one declared CI subject is not evaluable in the selected corpus; rescope or acquire more evidence.");
    }
    ingest::write_json(&out.join("selection.json"), &selected)?;
    receipt.selection = Some(selected.clone());
    receipt.advance(State::BundleCandidate);
    receipt.fidelity = selected_records
        .iter()
        .map(|record| bundle::FidelityOutcome {
            record_id: record.id.clone(),
            fidelity: None,
            failures: vec!["not_run".into()],
        })
        .collect();
    receipt.save(out)?;
    let baseline = out
        .join("binaries")
        .join(format!("{}.so", selected_records[0].current_program_sha256));
    let dependencies = out.join("dependencies");
    std::fs::create_dir(&dependencies)?;
    for dep in records[0]
        .dependencies
        .loadable()
        .filter(|d| d.program_id != scope.program)
    {
        let hash = dep
            .binary_sha256
            .as_ref()
            .context(Code::DependencyResolutionFailed)?;
        std::fs::copy(
            out.join("binaries").join(format!("{hash}.so")),
            dependencies.join(dep.artifact_file_name()),
        )?;
    }
    let inputs = bundle::BundleInputs {
        records: &selected_records,
        baseline: &baseline,
        dependencies: &dependencies,
        selection_policy: Some(selected.selection_policy.clone()),
        selection_policy_version: Some(selected.selection_policy_version),
        limitations: receipt.limitations.clone(),
        validation: bundle::Validation::AgainstBaseline,
    };
    let mut checkpoint_error = false;
    let built = bundle::build_observed(inputs, &out.join("bundle"), &mut |result| {
        if let Some(outcome) = receipt
            .fidelity
            .iter_mut()
            .find(|o| o.record_id == result.record_id)
        {
            *outcome = result;
        } else {
            checkpoint_error = true;
        }
        if receipt.save(out).is_err() {
            checkpoint_error = true;
        }
    });
    if checkpoint_error {
        return Err(Code::InternalFailure.into());
    }
    let built = built.context(Code::BaselineFidelityOrBuildFailed)?;
    // Deliberate second open: compare authoritative identities with the exact
    // selection and the builder result, never infer verification from directory existence.
    let reopened =
        bundle::CiBundle::open(out.join("bundle")).context(Code::BundleVerificationFailed)?;
    let ids: BTreeSet<_> = reopened
        .manifest()
        .record_ids
        .iter()
        .map(String::as_str)
        .collect();
    let expected_ids: BTreeSet<_> = selected_records.iter().map(|r| r.id.as_str()).collect();
    if reopened.manifest() != built.manifest()
        || ids != expected_ids
        || reopened.manifest().program_id != scope.program
        || reopened.adapter().name != adapter.name()
        || reopened.adapter().version != adapter.adapter_version()
        || reopened.adapter().limitations != receipt.limitations
    {
        return Err(Code::BundleVerificationFailed.into());
    }
    receipt.bundle = Some(reopened.manifest().clone());
    receipt.bundle_verified = true;
    receipt.advance(State::BundleVerified);
    receipt.exit_code = 0;
    receipt.coverage_accepted = config.accept_coverage && !missing_subject;
    if missing_subject {
        receipt.blockers.push(Code::MissingSelectedSubject);
    }
    if config.accept_coverage && !missing_subject {
        receipt.advance(State::ReadyForActivation);
        receipt.exit_code = 20;
        receipt.next_action =
            "register and deliberately activate through the separate administrative flow".into();
    } else {
        receipt.advance(State::CoverageReviewRequired);
        receipt.next_action="review scope, ledger, runtime assumptions and limitations; explicitly accept coverage or rescope in a fresh run".into();
    }
    Ok(())
}

fn persist_binary(out: &Path, hash: &str, bytes: &[u8]) -> Result<()> {
    if hash_bytes(bytes) != hash {
        return Err(Code::EvidenceIdentityMismatch.into());
    }
    std::fs::create_dir_all(out.join("binaries"))?;
    std::fs::write(out.join("binaries").join(format!("{hash}.so")), bytes)?;
    Ok(())
}

/// Excludes observation slots and discovery routes, but retains deployment,
/// loader and binary identities, including the target and builtins.
pub fn cohort(record: &ReplayRecord) -> Result<Value> {
    record.validate().context(Code::EvidenceIdentityMismatch)?;
    let mut programs = record.dependencies.programs.clone();
    for dep in &mut programs {
        dep.observed_slot = None;
        dep.discovered_by.clear();
        dep.note = None;
    }
    programs.sort_by(|a, b| a.program_id.cmp(&b.program_id));
    Ok(
        json!({"baseline_sha256":record.current_program_sha256,"programs":programs,"runtime_profile":"schema1_litesvm_mainnet","genesis":record.genesis_hash}),
    )
}
