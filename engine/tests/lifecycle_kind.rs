//! Lifecycle proposals carry declared terms without an executable or upgrade target.
use eplyx_engine::{
    change::{Activation, CandidateSource, Change, ChangeKind, ChangeSpec, ChangeTarget},
    lifecycle::{
        policy::{LifecycleScenario, LifecycleStatus},
        spec::{Eligibility, Ratio},
    },
    migration::spec::Rounding,
};

fn scenario() -> LifecycleScenario {
    serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/scenarios/spacex-transition.json"
    ))
    .unwrap()
}

#[test]
fn notice_policy_creates_a_real_non_executable_change() {
    let scenario = scenario();
    let spec = ChangeSpec::lifecycle(&scenario).unwrap();
    assert_eq!(spec.kind(), ChangeKind::LifecycleChange);
    assert!(matches!(spec.target(), ChangeTarget::LifecycleAsset(_)));
    assert!(spec.candidate().is_none() && spec.target_program_id().is_none());
    assert!(spec.as_program_upgrade().is_none() && spec.as_token_migration().is_none());
    assert!(spec
        .resolve(CandidateSource::Bytes(b"unrelated elf"))
        .is_err());
    assert!(spec.with_delivery(None).is_err());
    let change = spec.as_lifecycle().unwrap();
    assert_eq!(change.asset.mint, scenario.policy.asset_mint);
    assert!(change.ratio.is_none());
    let proposal = serde_json::to_value(&spec).unwrap();
    assert!(proposal["change"].get("ratio").is_none());
    assert_eq!(change.eligibility, Eligibility::Unknown);
    let binding = spec.bind_lifecycle(&scenario).unwrap();
    assert_eq!(binding.change_spec_id, spec.id().unwrap());
    assert!(binding.target_program_id().is_none() && binding.candidate_sha256().is_none());
    let json = serde_json::to_value(binding).unwrap();
    assert_eq!(json["kind"], "lifecycle_change");
    assert!(json.get("candidate_sha256").is_none());
}

#[test]
fn metadata_is_outside_identity_but_every_declared_semantic_term_is_inside() {
    let original = ChangeSpec::lifecycle(&scenario()).unwrap();
    let id = original.id().unwrap();
    let mut display = original.clone();
    display.metadata.label = Some("Another display label".into());
    display.metadata.source = Some("Local description".into());
    assert_eq!(display.id().unwrap(), id);
    let mut variants = vec![];
    let mut time = original.clone();
    time.activation.as_mut().unwrap().unix_timestamp = Some(1);
    variants.push(time);
    for field in 0..6 {
        let mut spec = original.clone();
        let Change::LifecycleChange(change) = &mut spec.change else {
            unreachable!()
        };
        match field {
            0 => change.before = LifecycleStatus::Unknown,
            1 => change.after = LifecycleStatus::Unknown,
            2 => change.deadline.as_mut().unwrap().unix_timestamp += 1,
            3 => {
                change.deadline.as_mut().unwrap().after =
                    LifecycleStatus::PostDeadlineTransitionRequired
            }
            4 => {
                change.eligibility = Eligibility::Declared {
                    policy: "External documented entitlement".into(),
                }
            }
            _ => {
                change.ratio = Some(Ratio {
                    numerator: "1".into(),
                    denominator: "2".into(),
                    rounding: Rounding::Floor,
                    fee_bps: 0,
                })
            }
        }
        variants.push(spec);
    }
    for spec in variants {
        assert_ne!(spec.id().unwrap(), id);
    }
    assert_eq!(
        ChangeSpec::parse(original.to_document().unwrap().as_bytes())
            .unwrap()
            .id()
            .unwrap(),
        id
    );
}

#[test]
fn activation_policy_and_source_disagreement_cannot_bind() {
    let original = scenario();
    let spec = ChangeSpec::lifecycle(&original).unwrap();
    for field in 0..4 {
        let mut changed = original.clone();
        match field {
            0 => changed.policy.effective_at += chrono::Duration::seconds(1),
            1 => changed.policy.before = LifecycleStatus::Unknown,
            2 => changed.sources[0].reference.push_str("-different"),
            _ => {
                changed.policy.deadline.as_mut().unwrap().after =
                    LifecycleStatus::PostDeadlineTransitionRequired
            }
        }
        assert!(spec.bind_lifecycle(&changed).is_err());
    }
    let mut changed = spec.clone();
    changed.activation = None;
    assert!(changed.validate().is_err());
    changed.activation = Some(Activation {
        slot: Some(1),
        unix_timestamp: None,
    });
    assert!(changed.validate().is_err());
    let mut bytes = serde_json::to_value(&spec).unwrap();
    bytes["change"]["invented_result"] = true.into();
    assert!(ChangeSpec::parse(&serde_json::to_vec(&bytes).unwrap()).is_err());
}

#[test]
fn complete_provenance_is_required_for_additional_ratio_and_eligibility_declarations() {
    let mut spec = ChangeSpec::lifecycle(&scenario()).unwrap();
    let Change::LifecycleChange(change) = &mut spec.change else {
        unreachable!()
    };
    change.destination = Some(eplyx_engine::lifecycle::spec::Asset {
        mint: "So11111111111111111111111111111111111111112".into(),
    });
    change.ratio = Some(Ratio {
        numerator: "1".into(),
        denominator: "2".into(),
        rounding: Rounding::Floor,
        fee_bps: 0,
    });
    change.eligibility = Eligibility::Declared {
        policy: "Terms supplied by operator".into(),
    };
    assert!(spec.validate().is_err());
    let Change::LifecycleChange(change) = &mut spec.change else {
        unreachable!()
    };
    change.sources[0].supports.extend([
        "/change/destination".into(),
        "/change/ratio".into(),
        "/change/eligibility".into(),
    ]);
    change.sources[0].supports.sort();
    change.sources[0].supports.dedup();
    spec.validate().unwrap();
    let Change::LifecycleChange(change) = &mut spec.change else {
        unreachable!()
    };
    change.ratio.as_mut().unwrap().denominator = "0".into();
    assert!(spec.validate().is_err());
}
