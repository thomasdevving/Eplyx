//! Second-layer control resolution over a frozen, freshly captured population.
//!
//! The plan is derived before any adapter is attempted. The existing coarse
//! classification is immutable. An identified controller is never a signer.
use super::{
    population,
    population_types::{AuthorityResolution, StressEntity},
};
use crate::{
    canonical::digest,
    evidence::{
        authority::{classify_authority, EntityType},
        paths::PathStatus,
    },
    standard_programs::token as decode,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use solana_address::Address;
use std::collections::{BTreeMap, BTreeSet};

#[path = "authority_resolution_meteora.rs"]
mod meteora;

/// Trusted, narrowly scoped program control discovery. An adapter may explain
/// custody without providing an executable candidate-conversion path.
pub(super) trait AuthorityAdapter {
    fn can_resolve(&self, entity: &StressEntity) -> bool;
    fn resolve(
        &self,
        entity: &StressEntity,
        raw: &Value,
        mint: &str,
        path: &mut ControlPath,
    ) -> Result<AdapterOutcome>;
}

pub(super) struct AdapterOutcome {
    resolution: ResolutionStatus,
    conversion: PathStatus,
    reason: String,
}

pub const SELECTOR_VERSION: &str = "eplyx-authority-resolution-select/v1";
pub const RESOLVER_VERSION: &str = "eplyx-authority-resolution/v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub max_cases: usize,
    pub max_linked_accounts_per_case: usize,
    pub max_adapters_per_case: usize,
    pub max_executable_program_cases: usize,
    pub max_concurrent_rpc_requests: usize,
    pub max_concurrent_vm_executions: usize,
    #[serde(with = "crate::numfmt::u64_string")]
    pub rpc_timeout_seconds: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub vm_timeout_seconds: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub max_response_bytes: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub max_artifact_bytes: u64,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            max_cases: 20,
            max_linked_accounts_per_case: 8,
            max_adapters_per_case: 3,
            max_executable_program_cases: 5,
            max_concurrent_rpc_requests: 1,
            max_concurrent_vm_executions: 1,
            rpc_timeout_seconds: 120,
            vm_timeout_seconds: 120,
            max_response_bytes: 2 * 1024 * 1024,
            max_artifact_bytes: 4 * 1024 * 1024,
        }
    }
}
impl Budget {
    fn validate(&self) -> Result<()> {
        ensure!(
            *self == Self::default(),
            "authority-resolution bounds are server controlled"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub token_account: String,
    pub recorded_authority: String,
    pub initial_classification: EntityType,
    pub observed_balance_raw: String,
    pub control_type_hint: String,
    pub selection_reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub selector_version: String,
    pub population_digest: String,
    pub run_id: String,
    pub mint: String,
    pub budget: Budget,
    pub selected: Vec<Selection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionStatus {
    ResolvedDirectWallet,
    ResolvedProgramControlled,
    ResolvedPDA,
    ResolvedMultisig,
    ResolvedProtocolInternal,
    ResolvedNeedsPrivateAuthorization,
    ResolvedExecutionUnsupported,
    PartiallyResolved,
    Unresolved,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvocationKind {
    DirectSigner,
    ProgramInstruction,
    MultisigExecution,
    PDAProgramInvocation,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPath {
    pub token_account: String,
    pub recorded_authority: String,
    pub controller_program: Option<String>,
    pub controller_state: Option<String>,
    pub pda_derivation: Option<Value>,
    pub invocation_kind: InvocationKind,
    pub required_accounts: Vec<String>,
    pub required_signers: Vec<String>,
    pub multisig_threshold: Option<u8>,
    pub multisig_members: Vec<String>,
    pub preconditions: Vec<String>,
    pub evidence: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub selection: Selection,
    pub authority_exists: bool,
    pub on_curve: bool,
    pub runtime_owner: Option<String>,
    pub executable: Option<bool>,
    pub data_len: Option<usize>,
    pub raw_data_sha256: Option<String>,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub lamports: Option<u64>,
    pub resolution: ResolutionStatus,
    pub conversion: PathStatus,
    pub execution_supported: bool,
    pub signer_assumed_locally: bool,
    pub reason: String,
    pub control_path: ControlPath,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub accounts: usize,
    pub public_balance_raw: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub positive_balance_accounts_observed: usize,
    pub cases_selected: usize,
    pub population_initial: BTreeMap<String, Group>,
    /// Initial classifications of only the frozen selected subset.
    pub initial: BTreeMap<String, Group>,
    pub resolved: BTreeMap<String, Group>,
    pub unselected_non_wallet_accounts: usize,
    pub unselected_non_wallet_raw: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub resolver_version: String,
    pub plan_sha256: String,
    pub population_digest: String,
    pub cases: Vec<Case>,
    pub coverage: Coverage,
}

fn raw<'a>(capture: &'a population::Capture, entity: &StressEntity) -> Result<&'a Value> {
    let evidence = entity
        .authority_evidence
        .as_ref()
        .context("selected authority was never captured")?;
    capture
        .observations
        .get(evidence.rpc_id)
        .and_then(|r| r.result.as_ref())
        .and_then(|r| r.pointer(&evidence.pointer))
        .context("selected authority evidence pointer is missing")
}

fn hint(entity: &StressEntity) -> String {
    format!(
        "{:?}/{}",
        entity.authority_model,
        entity
            .authority_observation
            .runtime_owner
            .as_deref()
            .unwrap_or("absent")
    )
}

pub fn plan(observation: &population::PopulationObservation) -> Result<Plan> {
    let budget = Budget::default();
    let mut candidates: Vec<&StressEntity> = observation
        .positive_entities()
        // The second-layer resolver consumes only authority observations already
        // captured by the population pass. A budget-exhausted authority remains
        // in the unselected Unresolved population group; it is never selected
        // without actually fetching its recorded account.
        .filter(|e| {
            e.authority_model != EntityType::WalletCompatible
                && e.authority_resolution == AuthorityResolution::Resolved
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.balance()
            .unwrap_or(0)
            .cmp(&a.balance().unwrap_or(0))
            .then_with(|| a.token_account.cmp(&b.token_account))
    });
    let mut chosen = Vec::new();
    let mut seen_types = BTreeSet::new();
    let mut seen_accounts = BTreeSet::new();
    for entity in &candidates {
        let key = hint(entity);
        if seen_types.insert(key) && seen_accounts.insert(&entity.token_account) {
            chosen.push((*entity, "NewControlType"));
            if chosen.len() == budget.max_cases {
                break;
            }
        }
    }
    if chosen.len() < budget.max_cases {
        for entity in candidates {
            if seen_accounts.insert(&entity.token_account) {
                chosen.push((entity, "HighestPositiveBalance"));
                if chosen.len() == budget.max_cases {
                    break;
                }
            }
        }
    }
    let selected = chosen
        .into_iter()
        .map(|(entity, reason)| {
            Ok(Selection {
                token_account: entity.token_account.clone(),
                recorded_authority: entity.authority.clone(),
                initial_classification: entity.authority_model.clone(),
                observed_balance_raw: entity.balance()?.to_string(),
                control_type_hint: hint(entity),
                selection_reason: reason.into(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let frozen = Plan {
        selector_version: SELECTOR_VERSION.into(),
        population_digest: observation.capture_sha256.clone(),
        run_id: observation.run_id.clone(),
        mint: observation.mint.clone(),
        budget,
        selected,
    };
    ensure!(
        crate::canonical::document(&frozen)?.len() as u64 <= frozen.budget.max_artifact_bytes,
        "authority-resolution plan exceeds artifact bound"
    );
    Ok(frozen)
}

fn group(map: &mut BTreeMap<String, Group>, key: String, amount: u64) -> Result<()> {
    let item = map.entry(key).or_insert(Group {
        accounts: 0,
        public_balance_raw: "0".into(),
    });
    item.accounts += 1;
    item.public_balance_raw = item
        .public_balance_raw
        .parse::<u128>()?
        .checked_add(u128::from(amount))
        .context("authority coverage balance overflow")?
        .to_string();
    Ok(())
}

/// The only supported controller adapter here is a verified DLMM pool/vault
/// relationship. It identifies custody, not a way to impersonate the pool.
fn resolve_one(
    selection: &Selection,
    entity: &StressEntity,
    capture: &population::Capture,
    mint: &str,
) -> Result<Case> {
    ensure!(
        selection.recorded_authority == entity.authority
            && selection.initial_classification == entity.authority_model
            && selection.observed_balance_raw == entity.state.raw_balance,
        "initial authority classification or account scope changed"
    );
    let captured = if entity.authority_resolution == AuthorityResolution::Resolved {
        Some(raw(capture, entity)?)
    } else {
        None
    };
    if let Some(raw_authority) = captured {
        let (model, observation, reason) = classify_authority(&entity.authority, raw_authority)?;
        ensure!(
            model == entity.authority_model
                && observation == entity.authority_observation
                && reason == entity.classification_reason,
            "initial authority classification differs from captured authority bytes"
        );
    }
    let bytes = captured
        .filter(|r| !r.is_null())
        .map(decode::raw_account_bytes)
        .transpose()?;
    let lamports = captured
        .filter(|r| !r.is_null())
        .map(|r| {
            r["lamports"]
                .as_u64()
                .context("captured authority lamports missing")
        })
        .transpose()?;
    let mut control_path = ControlPath {
        token_account: entity.token_account.clone(),
        recorded_authority: entity.authority.clone(),
        controller_program: None,
        controller_state: None,
        pda_derivation: None,
        invocation_kind: InvocationKind::Unsupported,
        required_accounts: vec![],
        required_signers: vec![],
        multisig_threshold: None,
        multisig_members: vec![],
        preconditions: vec![],
        evidence: entity
            .authority_evidence
            .as_ref()
            .map(|e| vec![format!("population:{}:{}", e.rpc_id, e.pointer)])
            .unwrap_or_default(),
    };
    let (resolution, conversion, reason) = if entity.authority_resolution
        == AuthorityResolution::NotResolved
    {
        (
            ResolutionStatus::Unresolved,
            PathStatus::Indeterminate,
            "Authority lookup budget or provider stopped before inspection".into(),
        )
    } else if entity.authority_model == EntityType::TokenMultisig {
        let multisig = entity
            .authority_observation
            .multisig
            .as_ref()
            .context("multisig classification lacks decoded state")?;
        let threshold = multisig["required_signers"]
            .as_u64()
            .context("multisig threshold absent")?;
        let members: Vec<String> = multisig["signers"]
            .as_array()
            .context("multisig members absent")?
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect();
        ensure!(
            threshold > 0 && threshold as usize <= members.len(),
            "invalid multisig threshold"
        );
        control_path.controller_program = entity.authority_observation.runtime_owner.clone();
        control_path.controller_state = Some(entity.authority.clone());
        control_path.invocation_kind = InvocationKind::MultisigExecution;
        control_path.required_accounts = vec![entity.authority.clone()];
        control_path.multisig_threshold = Some(threshold as u8);
        control_path.multisig_members = members;
        control_path.preconditions = vec![format!(
            "At least {threshold} actual member approvals; private authorization unavailable"
        )];
        (
            ResolutionStatus::ResolvedNeedsPrivateAuthorization,
            PathStatus::Unsupported,
            format!("Exact SPL Token multisig decoded; {threshold} member approvals unavailable"),
        )
    } else if meteora::MeteoraDlmmAuthorityAdapter.can_resolve(entity) {
        let outcome = meteora::MeteoraDlmmAuthorityAdapter.resolve(
            entity,
            captured.context("selected DLMM authority evidence missing")?,
            mint,
            &mut control_path,
        )?;
        (outcome.resolution, outcome.conversion, outcome.reason)
    } else if entity.authority_model == EntityType::ProgramOwnedAuthority {
        (
            ResolutionStatus::PartiallyResolved,
            PathStatus::Indeterminate,
            "Authority runtime owner is known; no trusted control adapter proves a signing path"
                .into(),
        )
    } else {
        (
            ResolutionStatus::Unresolved,
            PathStatus::Indeterminate,
            "Account existence, curve status and runtime owner do not establish a controller or PDA seeds".into(),
        )
    };
    let on_curve = entity.authority.parse::<Address>()?.is_on_curve();
    Ok(Case {
        selection: selection.clone(),
        authority_exists: captured.is_some_and(|r| !r.is_null()),
        on_curve,
        runtime_owner: entity.authority_observation.runtime_owner.clone(),
        executable: entity.authority_observation.executable,
        data_len: bytes.as_ref().map(Vec::len),
        raw_data_sha256: bytes.as_ref().map(|b| crate::replay::hash_bytes(b)),
        lamports,
        resolution,
        conversion,
        execution_supported: false,
        signer_assumed_locally: false,
        reason,
        control_path,
    })
}

/// Rebuilds every claim from pinned RPC bytes. Serialized resolution is compared
/// to this result by the package replay; it never grants proof by itself.
fn resolve(
    plan: &Plan,
    observation: &population::PopulationObservation,
    capture: &population::Capture,
) -> Result<Report> {
    plan.budget.validate()?;
    ensure!(
        digest(&population::evaluate(capture, &observation.capture_sha256)?)?
            == digest(observation)?,
        "population facts differ from raw capture"
    );
    ensure!(
        *plan == self::plan(observation)?,
        "frozen authority-resolution plan differs from capture"
    );
    let entities: BTreeMap<&str, &StressEntity> = observation
        .entities
        .iter()
        .map(|e| (e.token_account.as_str(), e))
        .collect();
    let mut cases = Vec::new();
    let mut initial = BTreeMap::new();
    let mut resolved = BTreeMap::new();
    for key in [
        "ResolvedExecutable",
        "ResolvedExecutionUnsupported",
        "ResolvedNeedsPrivateAuthorization",
        "ResolvedProtocolInternal",
        "PartiallyResolved",
        "StillUnresolved",
    ] {
        resolved.insert(
            key.into(),
            Group {
                accounts: 0,
                public_balance_raw: "0".into(),
            },
        );
    }
    for selection in &plan.selected {
        let entity = entities
            .get(selection.token_account.as_str())
            .context("selected authority account vanished")?;
        let case = resolve_one(selection, entity, capture, &plan.mint)?;
        ensure!(
            !case.signer_assumed_locally && !case.execution_supported,
            "non-wallet authority cannot inherit a direct signer or execution proof"
        );
        let amount = entity.balance()?;
        group(
            &mut initial,
            format!("{:?}", selection.initial_classification),
            amount,
        )?;
        let outcome = match case.resolution {
            ResolutionStatus::ResolvedProgramControlled | ResolutionStatus::ResolvedPDA => {
                "ResolvedExecutable"
            }
            ResolutionStatus::ResolvedNeedsPrivateAuthorization => {
                "ResolvedNeedsPrivateAuthorization"
            }
            ResolutionStatus::ResolvedProtocolInternal => "ResolvedProtocolInternal",
            ResolutionStatus::ResolvedExecutionUnsupported | ResolutionStatus::ResolvedMultisig => {
                "ResolvedExecutionUnsupported"
            }
            ResolutionStatus::PartiallyResolved => "PartiallyResolved",
            _ => "StillUnresolved",
        };
        group(&mut resolved, outcome.into(), amount)?;
        cases.push(case);
    }
    let selected: BTreeSet<&str> = plan
        .selected
        .iter()
        .map(|s| s.token_account.as_str())
        .collect();
    let mut unselected_count = 0;
    let mut unselected_raw = 0u128;
    let mut population_initial = BTreeMap::new();
    for key in [
        "ProgramOwnedAuthority",
        "TokenMultisig",
        "Unknown",
        "Unresolved",
    ] {
        population_initial.insert(
            key.into(),
            Group {
                accounts: 0,
                public_balance_raw: "0".into(),
            },
        );
    }
    for entity in observation.positive_entities() {
        if entity.authority_model != EntityType::WalletCompatible {
            let key = if entity.authority_resolution == AuthorityResolution::NotResolved {
                "Unresolved".into()
            } else {
                format!("{:?}", entity.authority_model)
            };
            group(&mut population_initial, key, entity.balance()?)?;
            if !selected.contains(entity.token_account.as_str()) {
                unselected_count += 1;
                unselected_raw += u128::from(entity.balance()?);
            }
        }
    }
    let report = Report {
        resolver_version: RESOLVER_VERSION.into(),
        plan_sha256: digest(plan)?,
        population_digest: plan.population_digest.clone(),
        cases,
        coverage: Coverage {
            positive_balance_accounts_observed: observation
                .summary
                .positive_balance_accounts_observed,
            cases_selected: plan.selected.len(),
            population_initial,
            initial,
            resolved,
            unselected_non_wallet_accounts: unselected_count,
            unselected_non_wallet_raw: unselected_raw.to_string(),
        },
    };
    ensure!(
        crate::canonical::document(&report)?.len() as u64 <= plan.budget.max_artifact_bytes,
        "authority-resolution report exceeds artifact bound"
    );
    Ok(report)
}

/// Recompute population classifications from exact input bytes before resolution.
pub fn resolve_bytes(
    plan: &Plan,
    bytes: &[u8],
    budget: &super::population_types::StressBudget,
) -> Result<Report> {
    let observation = population::evaluate_bytes(bytes, budget)?;
    let capture = serde_json::from_slice(bytes)?;
    resolve(plan, &observation, &capture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::meteora_dlmm;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use solana_program_pack::Pack;
    use spl_token_2022_interface::state::Multisig;
    use std::path::PathBuf;

    fn fixture(name: &str) -> (population::Capture, population::PopulationObservation) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/migration")
            .join(match name {
                "milestone8-healthy-worker" => "spacex/population.capture.json",
                "milestone8-underfunded" => "current-reference/underfunded.population.capture.json",
                "milestone8-second-asset" => {
                    "current-reference/second-asset.population.capture.json"
                }
                _ => panic!("unknown frozen fixture"),
            });
        let bytes = std::fs::read(path).unwrap();
        let capture: population::Capture = serde_json::from_slice(&bytes).unwrap();
        let observed = population::evaluate_bytes(&bytes, &capture.budget).unwrap();
        (capture, observed)
    }

    #[test]
    fn frozen_plan_and_offline_resolution_preserve_non_wallet_boundaries() {
        let (capture, observed) = fixture("milestone8-healthy-worker");
        let frozen = plan(&observed).unwrap();
        assert_eq!(frozen.budget.max_cases, 20);
        assert_eq!(frozen.selected.len(), 20);
        let report = resolve(&frozen, &observed, &capture).unwrap();
        assert_eq!(report.cases.len(), frozen.selected.len());
        assert!(report.cases.iter().all(|c| !c.signer_assumed_locally));
        assert!(report.cases.iter().all(|c| !c.execution_supported));
        assert!(report
            .cases
            .iter()
            .all(|c| c.conversion != PathStatus::Proven));
        assert!(
            report.cases.iter().any(|c| {
                !c.on_curve
                    && c.resolution == ResolutionStatus::Unresolved
                    && c.control_path.controller_program.is_none()
            }),
            "off-curve alone must not resolve a PDA"
        );
        assert!(
            report
                .cases
                .iter()
                .any(|c| c.resolution == ResolutionStatus::ResolvedProtocolInternal),
            "one resolved pool cannot prove its peers"
        );
        let controlled: Vec<_> = report
            .cases
            .iter()
            .filter(|c| c.resolution == ResolutionStatus::ResolvedProtocolInternal)
            .collect();
        assert_eq!(
            controlled.len(),
            1,
            "one resolved pool cannot prove its peers"
        );
        let exact = controlled[0];
        assert_eq!(
            exact.control_path.pda_derivation.as_ref().unwrap()["derived_source_vault"],
            exact.selection.token_account
        );
        let entity = observed
            .entities
            .iter()
            .find(|e| e.token_account == exact.selection.token_account)
            .unwrap();
        let authority_bytes = raw(&capture, entity).unwrap();
        assert!(meteora_dlmm::decode_pool(
            &exact.selection.recorded_authority,
            authority_bytes,
            &observed.mint
        )
        .is_ok());
        let wrong_state = observed
            .entities
            .iter()
            .find(|e| e.token_account != exact.selection.recorded_authority)
            .unwrap();
        assert!(
            meteora_dlmm::decode_pool(&wrong_state.token_account, authority_bytes, &observed.mint)
                .is_err(),
            "wrong controller state / PDA seeds must be rejected"
        );
        let mut wrong_program = authority_bytes.clone();
        wrong_program["owner"] = Value::String("11111111111111111111111111111111".into());
        assert!(
            meteora_dlmm::decode_pool(
                &exact.selection.recorded_authority,
                &wrong_program,
                &observed.mint
            )
            .is_err(),
            "wrong controller program must be rejected"
        );
        let mut replaced = frozen.clone();
        replaced.selected[0] = frozen.selected[1].clone();
        assert!(resolve(&replaced, &observed, &capture).is_err());
        let mut wrong_population = frozen.clone();
        wrong_population.population_digest = "0".repeat(64);
        assert!(
            resolve(&wrong_population, &observed, &capture).is_err(),
            "refreshed population cannot inherit authority resolution"
        );
        let (refreshed_capture, refreshed) = fixture("milestone8-underfunded");
        assert_eq!(refreshed.mint, observed.mint);
        assert_ne!(refreshed.capture_sha256, observed.capture_sha256);
        assert!(
            resolve(&frozen, &refreshed, &refreshed_capture).is_err(),
            "refreshed population cannot inherit authority resolution"
        );
        let mut changed_capture = capture.clone();
        changed_capture.observations[0].result = Some(Value::Null);
        assert!(resolve(&frozen, &observed, &changed_capture).is_err());
    }

    #[test]
    fn second_asset_uses_same_selector_and_never_inherits_first_asset_control() {
        let (first_capture, first) = fixture("milestone8-healthy-worker");
        let (second_capture, second) = fixture("milestone8-second-asset");
        let first_plan = plan(&first).unwrap();
        let second_plan = plan(&second).unwrap();
        assert_ne!(first_plan.mint, second_plan.mint);
        assert_ne!(first_plan.population_digest, second_plan.population_digest);
        assert!(resolve(&first_plan, &second, &second_capture).is_err());
        assert!(resolve(&second_plan, &first, &first_capture).is_err());
        let report = resolve(&second_plan, &second, &second_capture).unwrap();
        assert!(report.cases.iter().all(|c| !c.signer_assumed_locally));
    }

    #[test]
    fn off_curve_is_not_pda_proof() {
        let program: Address = meteora_dlmm::PROGRAM_ID.parse().unwrap();
        let (off_curve, _) = Address::find_program_address(&[b"unrelated"], &program);
        assert!(!off_curve.is_on_curve());
        // A derivation for an unrelated seed does not bind a captured token
        // authority to controller state. No generic off-curve adapter exists.
        assert_ne!(
            Address::find_program_address(&[b"other"], &program).0,
            off_curve
        );
    }

    #[test]
    fn resolved_pool_never_proves_peer_accounts() {
        let (capture, observed) = fixture("milestone8-healthy-worker");
        let report = resolve(&plan(&observed).unwrap(), &observed, &capture).unwrap();
        let resolved = report
            .cases
            .iter()
            .filter(|c| c.resolution == ResolutionStatus::ResolvedProtocolInternal)
            .count();
        assert_eq!(resolved, 1, "one resolved pool cannot prove its peers");
    }

    #[test]
    fn refreshed_population_rejects_prior_resolution_plan() {
        let (capture, observed) = fixture("milestone8-healthy-worker");
        let frozen = plan(&observed).unwrap();
        let mut stale = frozen.clone();
        stale.population_digest = "0".repeat(64);
        assert!(
            resolve(&stale, &observed, &capture).is_err(),
            "refreshed population cannot inherit authority resolution"
        );
        let (refreshed_capture, refreshed) = fixture("milestone8-underfunded");
        assert!(
            resolve(&frozen, &refreshed, &refreshed_capture).is_err(),
            "refreshed population cannot inherit authority resolution"
        );
    }

    #[test]
    fn exact_spl_multisig_resolution_never_fabricates_approvals() {
        let (mut capture, observed) = fixture("milestone8-healthy-worker");
        let mut entity = observed
            .positive_entities()
            .find(|e| e.authority_evidence.is_some())
            .unwrap()
            .clone();
        let evidence = entity.authority_evidence.clone().unwrap();
        let mut multisig = Multisig {
            m: 2,
            n: 3,
            is_initialized: true,
            ..Multisig::default()
        };
        for index in 0..3 {
            multisig.signers[index] = Address::new_from_array([index as u8 + 20; 32]);
        }
        let mut data = vec![0; Multisig::LEN];
        multisig.pack_into_slice(&mut data);
        let raw_account = serde_json::json!({
            "data": [STANDARD.encode(data), "base64"],
            "owner": crate::standard_programs::token::LEGACY_PROGRAM,
            "executable": false,
            "lamports": 1_000_000,
        });
        *capture.observations[evidence.rpc_id]
            .result
            .as_mut()
            .unwrap()
            .pointer_mut(&evidence.pointer)
            .unwrap() = raw_account.clone();
        let (model, authority_observation, reason) =
            classify_authority(&entity.authority, &raw_account).unwrap();
        assert_eq!(model, EntityType::TokenMultisig);
        entity.authority_model = model;
        entity.authority_observation = authority_observation;
        entity.classification_reason = reason;
        let selected = Selection {
            token_account: entity.token_account.clone(),
            recorded_authority: entity.authority.clone(),
            initial_classification: EntityType::TokenMultisig,
            observed_balance_raw: entity.state.raw_balance.clone(),
            control_type_hint: hint(&entity),
            selection_reason: "NewControlType".into(),
        };
        let result = resolve_one(&selected, &entity, &capture, &observed.mint).unwrap();
        assert_eq!(
            result.resolution,
            ResolutionStatus::ResolvedNeedsPrivateAuthorization
        );
        assert_eq!(
            result.conversion,
            PathStatus::Unsupported,
            "multisig approvals are not conversion proof"
        );
        assert_eq!(result.control_path.multisig_threshold, Some(2));
        assert_eq!(result.control_path.multisig_members.len(), 3);
        assert!(result.control_path.required_signers.is_empty());
        assert!(!result.signer_assumed_locally);
        assert!(!result.execution_supported);
    }
}
