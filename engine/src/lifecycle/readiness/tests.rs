use super::*;
fn scope(entity: &str) -> EvidenceScope {
    EvidenceScope {
        asset_mint: "asset-a".into(),
        entity_id: entity.into(),
        state_shape: StateShape::DirectTokenAccount,
        authority: "owner-a".into(),
        exact_amount_raw: Some("100".into()),
        range: None,
        bps_to_remove: None,
        venue: Some("venue-a".into()),
        context_id: Some("route-a".into()),
        captured_slot: Some(1),
        clock: None,
        capture_context: Some(
            crate::lifecycle::frozen::selection::pipeline::CaptureContext::ExistingCapturedState,
        ),
        captured_state_sha256: Some("state-a".into()),
        fixture_sha256: Some("fixture-a".into()),
        source_before_raw: Some("100".into()),
        scenario_sha256: "scenario-a".into(),
    }
}
fn condition(path: ExitPathType, s: EvidenceScope) -> PathCondition {
    PathCondition {
        path_type: path,
        scope: s,
        accepted_statuses: vec![PathStatus::Proven],
        blocking_statuses: vec![PathStatus::Failed],
        allow_local_signer_assumption: true,
    }
}
fn requirement(path: ExitPathType, s: EvidenceScope) -> ReadinessRequirement {
    ReadinessRequirement {
        id: "path".into(),
        label: "Exact action".into(),
        target: RequirementTarget::Entity {
            entity_id: s.entity_id.clone(),
        },
        required: true,
        condition: RequirementCondition::Path {
            any_of: vec![condition(path, s)],
        },
        rollout_assumption: "This exact action is proven.".into(),
        remediation_requirement: "Obtain independently measured scoped evidence.".into(),
    }
}
fn fact(path: ExitPathType, status: PathStatus, s: EvidenceScope) -> PathFact {
    PathFact {
        signer: SignerAssumption {
            authority: s.authority.clone(),
            signer_possession_known: false,
            signer_assumed_locally: true,
            wording: "Controlled model, locally assumed signer.".into(),
        },
        scope: s,
        path_type: path,
        status,
        execution_attempted: status == PathStatus::Proven || status == PathStatus::Failed,
        reconciled: status == PathStatus::Proven,
        rollback_verified: if status == PathStatus::Failed {
            Some(true)
        } else {
            None
        },
        evidence_ids: vec!["controlled".into()],
        reason: "Controlled unit model, not a production VM observation.".into(),
    }
}
fn policy(r: Vec<ReadinessRequirement>) -> LifecycleReadinessPolicy {
    LifecycleReadinessPolicy {
        schema_version: 1,
        id: "generic-demo".into(),
        policy_type: PolicyType::DemoAssurancePolicy,
        not_issuer_policy: true,
        description: "Controlled generic policy".into(),
        asset_mint: "asset-a".into(),
        scenario_sha256: "scenario-a".into(),
        evaluated_scope: EvaluatedScope::DemoEntityReadiness,
        evidence_manifest: ArtifactRef {
            file: "controlled.json".into(),
            sha256: "controlled".into(),
        },
        requirements: r,
    }
}
fn evidence(facts: Vec<PathFact>) -> VerifiedReadinessEvidence {
    let at = "2026-01-01T00:00:00Z".parse().unwrap();
    VerifiedReadinessEvidence {
        conversion_facts: vec![],
        asset_mint: "asset-a".into(),
        scenario_sha256: "scenario-a".into(),
        lifecycle_event: crate::lifecycle::policy::AssetLifecyclePolicy {
            asset_mint: "asset-a".into(),
            effective_at: at,
            before: crate::lifecycle::policy::LifecycleStatus::Active,
            after: crate::lifecycle::policy::LifecycleStatus::TransitionRequired,
            deadline: None,
            successor: None,
        },
        policy_evaluated_at: "2026-01-01T00:00:00Z".into(),
        path_facts: facts,
        complete_exits: vec![],
        population: PopulationEvidence {
            token_account_entities: 2,
            positive_balance_entities: 2,
            distinct_owner_authorities: 2,
            entities_with_measured_amount: 1,
            represented_amount_raw: "200".into(),
            covered_amount_raw: "100".into(),
            without_evidence_raw: "100".into(),
            account_types: BTreeMap::new(),
            execution_status_counts: BTreeMap::new(),
            evidence_ids: vec!["controlled".into()],
            entities: vec![
                PopulationEntityFact {
                    entity_id: "entity-a".into(),
                    account_type: "WalletCompatible".into(),
                    balance_raw: "100".into(),
                    proven_full_amount_paths: vec![ExitPathType::Transfer],
                    proven_candidate_conversion: false,
                },
                PopulationEntityFact {
                    entity_id: "entity-b".into(),
                    account_type: "Unknown".into(),
                    balance_raw: "100".into(),
                    proven_full_amount_paths: vec![],
                    proven_candidate_conversion: false,
                },
            ],
        },
        evidence_refs: vec![],
        isolation_verified: true,
        stress: None,
    }
}
fn status(p: &LifecycleReadinessPolicy, e: &VerifiedReadinessEvidence) -> ReadinessStatus {
    evaluate(p, e).unwrap().overall_status
}
#[test]
fn missing_evidence_is_incomplete_not_ready() {
    assert_eq!(
        status(
            &policy(vec![requirement(
                ExitPathType::OfficialTransition,
                scope("entity-a")
            )]),
            &evidence(vec![])
        ),
        ReadinessStatus::Incomplete
    );
}
#[test]
fn not_tested_never_becomes_proven_or_failed() {
    let s = scope("entity-a");
    let mut explicit = policy(vec![requirement(
        ExitPathType::OfficialTransition,
        s.clone(),
    )]);
    if let RequirementCondition::Path { any_of } = &mut explicit.requirements[0].condition {
        any_of[0].blocking_statuses.push(PathStatus::NotTested);
    }
    let observed = evidence(vec![fact(
        ExitPathType::OfficialTransition,
        PathStatus::NotTested,
        s.clone(),
    )]);
    let report = evaluate(&explicit, &observed).unwrap();
    assert_eq!(report.overall_status, ReadinessStatus::Blocked);
    assert_eq!(report.path_evidence[0].status, PathStatus::NotTested);
    assert_eq!(
        status(
            &policy(vec![requirement(
                ExitPathType::OfficialTransition,
                s.clone()
            )]),
            &evidence(vec![fact(
                ExitPathType::OfficialTransition,
                PathStatus::NotTested,
                s
            )])
        ),
        ReadinessStatus::Incomplete
    );
}
#[test]
fn failed_required_path_blocks_only_when_explicit() {
    let s = scope("entity-a");
    let mut p = policy(vec![requirement(
        ExitPathType::SecondaryMarketExit,
        s.clone(),
    )]);
    let e = evidence(vec![fact(
        ExitPathType::SecondaryMarketExit,
        PathStatus::Failed,
        s,
    )]);
    assert_eq!(status(&p, &e), ReadinessStatus::Blocked);
    if let RequirementCondition::Path { any_of } = &mut p.requirements[0].condition {
        any_of[0].blocking_statuses.clear();
    }
    assert_eq!(status(&p, &e), ReadinessStatus::Incomplete);
}
#[test]
fn unsupported_is_a_boundary_until_policy_blocks() {
    let s = scope("entity-a");
    let mut p = policy(vec![requirement(ExitPathType::Redemption, s.clone())]);
    let e = evidence(vec![fact(
        ExitPathType::Redemption,
        PathStatus::Unsupported,
        s,
    )]);
    assert_eq!(status(&p, &e), ReadinessStatus::Incomplete);
    if let RequirementCondition::Path { any_of } = &mut p.requirements[0].condition {
        any_of[0].blocking_statuses.push(PathStatus::Unsupported);
    }
    assert_eq!(status(&p, &e), ReadinessStatus::Blocked);
}
#[test]
fn transfer_never_satisfies_official_transition() {
    let s = scope("entity-a");
    assert_eq!(
        status(
            &policy(vec![requirement(
                ExitPathType::OfficialTransition,
                s.clone()
            )]),
            &evidence(vec![fact(ExitPathType::Transfer, PathStatus::Proven, s)])
        ),
        ReadinessStatus::Incomplete
    );
}
#[test]
fn market_exit_never_satisfies_official_transition() {
    let s = scope("entity-a");
    assert_eq!(
        status(
            &policy(vec![requirement(
                ExitPathType::OfficialTransition,
                s.clone()
            )]),
            &evidence(vec![fact(
                ExitPathType::SecondaryMarketExit,
                PathStatus::Proven,
                s
            )])
        ),
        ReadinessStatus::Incomplete
    );
}
#[test]
fn withdrawal_never_satisfies_official_transition() {
    let mut s = scope("lp-a");
    s.state_shape = StateShape::ProtocolPosition;
    s.range = Some([-1, 1]);
    s.bps_to_remove = Some(10000);
    assert_eq!(
        status(
            &policy(vec![requirement(
                ExitPathType::OfficialTransition,
                s.clone()
            )]),
            &evidence(vec![fact(ExitPathType::Withdrawal, PathStatus::Proven, s)])
        ),
        ReadinessStatus::Incomplete
    );
}
#[test]
fn principal_is_not_complete_position_exit() {
    let mut s = scope("lp-a");
    s.state_shape = StateShape::ProtocolPosition;
    let mut r = requirement(ExitPathType::Withdrawal, s.clone());
    r.condition = RequirementCondition::CompletePositionExit {
        scope: Box::new(s.clone()),
        forbid_remaining_fees: false,
    };
    let mut e = evidence(vec![]);
    e.complete_exits.push(CompleteExitFact {
        scope: s,
        principal_removed_raw: BTreeMap::new(),
        owner_received_raw: BTreeMap::new(),
        destination_withheld_raw: BTreeMap::new(),
        principal_unwind: PathStatus::Proven,
        fee_collection: PathStatus::NotTested,
        position_closure: PathStatus::NotTested,
        residual_fees_raw: BTreeMap::from([("asset-a".into(), "126543".into())]),
        evidence_ids: vec!["controlled".into()],
    });
    let mut p = policy(vec![r]);
    assert_eq!(status(&p, &e), ReadinessStatus::Incomplete);
    if let RequirementCondition::CompletePositionExit {
        forbid_remaining_fees,
        ..
    } = &mut p.requirements[0].condition
    {
        *forbid_remaining_fees = true;
    }
    assert_eq!(status(&p, &e), ReadinessStatus::Blocked);
}
#[test]
fn evidence_cannot_inherit_between_entities() {
    let s = scope("entity-a");
    let mut other = s.clone();
    other.entity_id = "entity-b".into();
    assert_eq!(
        status(
            &policy(vec![requirement(ExitPathType::Transfer, s)]),
            &evidence(vec![fact(
                ExitPathType::Transfer,
                PathStatus::Proven,
                other
            )])
        ),
        ReadinessStatus::Incomplete
    );
}
#[test]
fn venue_context_bank_and_fixture_are_exact() {
    let s = scope("entity-a");
    let p = policy(vec![requirement(
        ExitPathType::SecondaryMarketExit,
        s.clone(),
    )]);
    for i in 0..5 {
        let mut other = s.clone();
        match i {
            0 => other.venue = Some("venue-b".into()),
            1 => other.context_id = Some("route-b".into()),
            2 => other.captured_slot = Some(2),
            3 => other.fixture_sha256 = Some("fixture-b".into()),
            _ => other.exact_amount_raw = Some("99".into()),
        }
        assert_eq!(
            status(
                &p,
                &evidence(vec![fact(
                    ExitPathType::SecondaryMarketExit,
                    PathStatus::Proven,
                    other
                )])
            ),
            ReadinessStatus::Incomplete
        );
    }
}
#[test]
fn sampled_entity_never_proves_population() {
    let s = scope("entity-a");
    let r = requirement(ExitPathType::Transfer, s.clone());
    let e = evidence(vec![fact(ExitPathType::Transfer, PathStatus::Proven, s)]);
    let mut p = policy(vec![r]);
    let entity = evaluate(&p, &e).unwrap();
    assert_eq!(entity.overall_status, ReadinessStatus::Ready);
    assert!(entity.rollout_readiness.is_none());
    p.evaluated_scope = EvaluatedScope::PopulationRolloutReadiness;
    p.requirements.push(ReadinessRequirement {
        id: "population".into(),
        target: RequirementTarget::Rollout,
        condition: RequirementCondition::PopulationCoverage {
            any_of_paths: vec![ExitPathType::Transfer],
        },
        ..p.requirements[0].clone()
    });
    let report = evaluate(&p, &e).unwrap();
    assert_eq!(report.overall_status, ReadinessStatus::Incomplete);
    assert_eq!(report.entity_readiness[0].status, ReadinessStatus::Ready);
    assert_eq!(
        report.rollout_readiness.unwrap().exact_entities_satisfied,
        1
    );
}
#[test]
fn unrelated_success_cannot_erase_required_failure() {
    let s = scope("entity-a");
    let mut failed = requirement(ExitPathType::SecondaryMarketExit, s.clone());
    failed.id = "required-failed".into();
    let p = policy(vec![requirement(ExitPathType::Transfer, s.clone()), failed]);
    assert_eq!(
        status(
            &p,
            &evidence(vec![
                fact(ExitPathType::Transfer, PathStatus::Proven, s.clone()),
                fact(ExitPathType::SecondaryMarketExit, PathStatus::Failed, s)
            ])
        ),
        ReadinessStatus::Blocked
    );
}
#[test]
fn optional_failed_route_is_informational() {
    let s = scope("entity-a");
    let mut optional = requirement(ExitPathType::SecondaryMarketExit, scope("large-holder"));
    optional.id = "optional".into();
    optional.required = false;
    let p = policy(vec![
        requirement(ExitPathType::Transfer, s.clone()),
        optional,
    ]);
    let r = evaluate(
        &p,
        &evidence(vec![
            fact(ExitPathType::Transfer, PathStatus::Proven, s),
            fact(
                ExitPathType::SecondaryMarketExit,
                PathStatus::Failed,
                scope("large-holder"),
            ),
        ]),
    )
    .unwrap();
    assert_eq!(r.overall_status, ReadinessStatus::Ready);
    assert_eq!(r.entity_readiness.len(), 1);
    assert_eq!(
        r.findings
            .iter()
            .find(|f| f.requirement_id == "optional")
            .unwrap()
            .effect,
        FindingEffect::Informational
    );
    assert!(!r.prevented_rollout_conditions[0].real_incident_claimed);
}
#[test]
fn signer_assumption_is_explicit_not_key_possession() {
    let s = scope("entity-a");
    let mut p = policy(vec![requirement(ExitPathType::Transfer, s.clone())]);
    let e = evidence(vec![fact(ExitPathType::Transfer, PathStatus::Proven, s)]);
    assert_eq!(status(&p, &e), ReadinessStatus::Ready);
    if let RequirementCondition::Path { any_of } = &mut p.requirements[0].condition {
        any_of[0].allow_local_signer_assumption = false;
    }
    assert_eq!(status(&p, &e), ReadinessStatus::Incomplete);
}
#[test]
fn policy_ordering_and_json_roundtrip_are_canonical() {
    let s = scope("entity-a");
    let mut r = requirement(ExitPathType::Transfer, s.clone());
    if let RequirementCondition::Path { any_of } = &mut r.condition {
        any_of.push(condition(ExitPathType::SecondaryMarketExit, s.clone()));
    }
    let mut b = r.clone();
    b.id = "second".into();
    let mut p = policy(vec![r, b]);
    let e = evidence(vec![fact(ExitPathType::Transfer, PathStatus::Proven, s)]);
    let expected = evaluate(&p, &e).unwrap().to_json().unwrap();
    p.requirements.reverse();
    for r in &mut p.requirements {
        if let RequirementCondition::Path { any_of } = &mut r.condition {
            any_of.reverse();
        }
    }
    assert_eq!(evaluate(&p, &e).unwrap().to_json().unwrap(), expected);
    let round: LifecycleReadinessReport = serde_json::from_str(&expected).unwrap();
    assert_eq!(round.to_json().unwrap(), expected);
}
#[test]
fn invalid_policy_cannot_contradict_accepted_proof() {
    let mut p = policy(vec![requirement(ExitPathType::Transfer, scope("entity-a"))]);
    if let RequirementCondition::Path { any_of } = &mut p.requirements[0].condition {
        any_of[0].blocking_statuses.push(PathStatus::Proven);
    }
    assert!(p.normalized().is_err());
    p.requirements[0].required = false;
    assert!(p.normalized().is_err());
}
#[test]
fn generic_asset_and_economic_policy_remain_separate() {
    let s = scope("entity-a");
    let p = policy(vec![requirement(ExitPathType::Transfer, s.clone())]);
    let mut e = evidence(vec![fact(ExitPathType::Transfer, PathStatus::Proven, s)]);
    e.lifecycle_event.after = crate::lifecycle::policy::LifecycleStatus::Expired;
    assert_eq!(status(&p, &e), ReadinessStatus::Ready);
    e.scenario_sha256 = "different".into();
    assert!(evaluate(&p, &e).is_err());
}
#[test]
fn status_exit_codes_are_stable() {
    assert_eq!(ReadinessStatus::Ready.exit_code(), 0);
    assert_eq!(ReadinessStatus::Blocked.exit_code(), 1);
    assert_eq!(ReadinessStatus::Incomplete.exit_code(), 5);
}
#[test]
fn missing_vm_or_reconciliation_never_grants_ready() {
    let s = scope("entity-a");
    let p = policy(vec![requirement(ExitPathType::Transfer, s.clone())]);
    let mut f = fact(ExitPathType::Transfer, PathStatus::Proven, s);
    f.execution_attempted = false;
    assert_eq!(
        status(&p, &evidence(vec![f.clone()])),
        ReadinessStatus::Incomplete
    );
    f.execution_attempted = true;
    f.reconciled = false;
    assert_eq!(status(&p, &evidence(vec![f])), ReadinessStatus::Incomplete);
}
#[test]
fn failed_claim_without_actual_rollback_is_not_blocker() {
    let s = scope("entity-a");
    let p = policy(vec![requirement(
        ExitPathType::SecondaryMarketExit,
        s.clone(),
    )]);
    let mut f = fact(ExitPathType::SecondaryMarketExit, PathStatus::Failed, s);
    f.rollback_verified = None;
    assert_eq!(status(&p, &evidence(vec![f])), ReadinessStatus::Incomplete);
}
