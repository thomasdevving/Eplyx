//! Entity-only presets reuse the existing exact-scope requirement evaluator.
//! Mobility, candidate-plan and full-transition assurance stay separate gates.
use super::*;
use crate::lifecycle::resolution::current::VerifiedCurrentPaths;
use serde_json::{json, Value};

/// Which question this evaluation answers. They never substitute for each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Can this exact tested amount move or be sold?
    Mobility,
    /// Does this supplied conversion plan work for this exact account and amount?
    CandidatePlan,
    /// Everything mobility requires, plus an independently proven official conversion.
    FullTransition,
}
pub(crate) fn evaluate_selected(
    paths: &VerifiedCurrentPaths,
    conversions: &[ConversionFact],
    policy: &crate::lifecycle::policy::AssetLifecyclePolicy,
    scope: &crate::lifecycle::resolution::current::CurrentEntityScope<'_>,
    at: &str,
    gate: Gate,
) -> Result<Value> {
    let crate::lifecycle::resolution::current::CurrentEntityScope {
        run,
        wallet_hash,
        source,
        owner,
        scenario_hash,
        ..
    } = *scope;
    let entity = format!("current:{run}:{source}");
    let base = EvidenceScope {
        asset_mint: policy.asset_mint.clone(),
        entity_id: entity.clone(),
        state_shape: StateShape::DirectTokenAccount,
        authority: owner.into(),
        exact_amount_raw: None,
        range: None,
        bps_to_remove: None,
        venue: None,
        context_id: None,
        captured_slot: None,
        clock: None,
        capture_context: None,
        captured_state_sha256: Some(wallet_hash.into()),
        fixture_sha256: None,
        source_before_raw: None,
        scenario_sha256: scenario_hash.into(),
    };
    let mut facts = vec![];
    for row in paths.rows() {
        for context in &row.contexts {
            for a in &context.attempts {
                let mut scope = base.clone();
                scope.exact_amount_raw = Some(a.exact_input_raw.clone());
                scope.context_id = Some(a.context_id.clone());
                scope.captured_slot = a.clock.as_ref().map(|c| c.slot);
                scope.clock = a.clock;
                scope.capture_context = Some(a.capture_context);
                scope.captured_state_sha256 = a.captured_state_sha256.clone();
                scope.fixture_sha256 = a.fixture_sha256.clone();
                scope.source_before_raw = a.source_before_raw.clone();
                facts.push(PathFact {scope,path_type:a.path_type,status:a.status,execution_attempted:a.execution_attempted,
            reconciled:a.status==PathStatus::Proven || a.rollback_verified==Some(true),rollback_verified:a.rollback_verified,
            signer:a.signer.clone(),evidence_ids:vec![a.case_id.clone()],reason:"Exact current-run replay; original check retains destination, route, minimum output and fees".into()});
            }
        }
    }
    let condition = |path, scope| PathCondition {
        path_type: path,
        scope,
        accepted_statuses: vec![PathStatus::Proven],
        blocking_statuses: vec![],
        allow_local_signer_assumption: true,
    };
    let requirement = |id: &str, label: &str, condition| {
        ReadinessRequirement{id:id.into(),label:label.into(),target:RequirementTarget::Entity{entity_id:entity.clone()},required:true,condition,
        rollout_assumption:"Selected account only, under captured-state and assumed local signing; no future liquidity assurance".into(),
        remediation_requirement:"Run an available exact-scope local check, or independently establish and execute the conversion mechanism; then re-evaluate".into()}
    };
    let mut requirements = vec![requirement(
        "observed-and-isolated",
        "Selected current account observed; exact evidence isolation",
        RequirementCondition::EvidenceIsolation,
    )];
    if gate == Gate::CandidatePlan {
        // The gate pins whatever the supplied plan actually executed against. With
        // no candidate execution the pinned digests match nothing and stay Incomplete.
        let fact = conversions.first();
        requirements.push(requirement(
            "candidate-conversion",
            "The supplied candidate conversion plan executed and reconciled for this exact account and amount",
            RequirementCondition::CandidateConversion {
                scope: Box::new(fact.map(|f| f.scope.clone()).unwrap_or_else(|| base.clone())),
                plan_sha256: fact.map(|f| f.plan_sha256.clone()).unwrap_or_else(|| "0".repeat(64)),
                program_sha256: fact
                    .map(|f| f.program_sha256.clone())
                    .unwrap_or_else(|| "0".repeat(64)),
                replacement_mint: fact
                    .map(|f| f.replacement_mint.clone())
                    .unwrap_or_else(|| "no-candidate-replacement-supplied".into()),
            },
        ));
    } else {
        let mut mobility: Vec<_> = facts
            .iter()
            .filter(|f| {
                matches!(
                    f.path_type,
                    ExitPathType::Transfer | ExitPathType::SecondaryMarketExit
                )
            })
            .map(|f| condition(f.path_type, f.scope.clone()))
            .collect();
        if mobility.is_empty() {
            mobility = vec![
                condition(ExitPathType::Transfer, base.clone()),
                condition(ExitPathType::SecondaryMarketExit, base.clone()),
            ];
        }
        requirements.push(requirement(
            "mobility",
            "At least one selected exact mobility test is Proven",
            RequirementCondition::Path { any_of: mobility },
        ));
        if gate == Gate::FullTransition {
            requirements.push(requirement(
                "official-conversion",
                "Actual replacement-token conversion must be independently Proven",
                RequirementCondition::Path {
                    any_of: vec![condition(ExitPathType::OfficialTransition, base.clone())],
                },
            ));
        }
    }
    let assurance=LifecycleReadinessPolicy {schema_version:1,id:match gate {Gate::FullTransition=>"current-full-transition-v1",Gate::CandidatePlan=>"current-candidate-conversion-v1",Gate::Mobility=>"current-mobility-v1"}.into(),policy_type:PolicyType::DemoAssurancePolicy,
        not_issuer_policy:true,description:"Selected entity demonstration assurance; successful exact tested amounts do not prove whole-balance or future actionability. Missing and failed alternatives remain incomplete, not blocking.".into(),
        asset_mint:policy.asset_mint.clone(),scenario_sha256:scenario_hash.into(),evaluated_scope:EvaluatedScope::DemoEntityReadiness,
        evidence_manifest:ArtifactRef{file:format!("run:{run}"),sha256:wallet_hash.into()},requirements};
    let evidence = VerifiedReadinessEvidence {
        asset_mint: policy.asset_mint.clone(),
        scenario_sha256: scenario_hash.into(),
        lifecycle_event: policy.clone(),
        policy_evaluated_at: at.into(),
        path_facts: facts,
        conversion_facts: conversions.to_vec(),
        complete_exits: vec![],
        isolation_verified: true,
        stress: None,
        evidence_refs: vec![EvidenceReference {
            id: run.into(),
            artifact: ArtifactRef {
                file: format!("{run}.capture.json"),
                sha256: wallet_hash.into(),
            },
            description: "Successfully decoded selected current account".into(),
        }],
        population: PopulationEvidence {
            token_account_entities: 0,
            positive_balance_entities: 0,
            distinct_owner_authorities: 0,
            entities_with_measured_amount: 0,
            represented_amount_raw: "0".into(),
            covered_amount_raw: "0".into(),
            without_evidence_raw: "0".into(),
            account_types: BTreeMap::new(),
            execution_status_counts: BTreeMap::new(),
            evidence_ids: vec![],
            entities: vec![],
        },
    };
    let report = evaluate(&assurance, &evidence)?;
    // Do not publish the historical report's population fields for a current entity.
    let mut value = json!({"scope":match gate {Gate::CandidatePlan=>"CandidateConversionPlanReadiness",_=>"SelectedEntityPreflightReadiness"},
        "status":report.overall_status,"policy":report.policy,"policy_sha256":report.policy_semantic_sha256,
        "findings":report.findings,"entity":report.entity_readiness,"path_evidence":report.path_evidence,
        "population_readiness":null,"authorization":false});
    if gate == Gate::CandidatePlan {
        value["conversion_evidence"] = serde_json::to_value(conversions)?;
        value["official_transition_established"] = false.into();
        value["question"] = "Does this supplied conversion plan work for this exact selected current account and amount?".into();
    }
    Ok(value)
}
