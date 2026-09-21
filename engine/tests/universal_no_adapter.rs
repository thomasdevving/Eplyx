use std::path::PathBuf;

use eplyx_engine::{
    ci::semantic_coverage_failure,
    protocol::adapter_for,
    review::{FailureReason, ObservationCoverage},
    universal::{
        evidence::EvidenceStore,
        model::{InstructionRole, ReplayObservationV2},
        pipeline,
    },
};

#[test]
fn historical_dependency_replays_without_a_semantic_adapter() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/examples/phase-u4-kamino");
    let mut record: ReplayObservationV2 =
        serde_json::from_slice(
            &std::fs::read(root.join(
                "records/e56202c8fdcd443d1deba455ab04f2bc6f16689f83a7c107bb98765ec2da626f.json",
            ))
            .expect("frozen T1 record"),
        )
        .expect("schema-2 observation");
    let store = EvidenceStore::at(root.join("evidence"));
    let original = record
        .resolve(&store)
        .expect("historical evidence resolves");
    let scope = "HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ";
    let scope_index = original
        .message
        .transaction
        .instructions
        .iter()
        .position(|ix| ix.program == scope)
        .expect("Scope executed in complete envelope");
    record.program_id = scope.into();
    record.protocol = "uninterpreted-scope".into();
    record.target.program_id = scope.into();
    record.target.outer_index = scope_index;
    record.target.instruction_identity = "uninterpreted-execution-dependency".into();
    record.instruction_roles.iter_mut().for_each(|role| {
        if role.role == InstructionRole::SemanticTarget {
            role.role = InstructionRole::TargetPrerequisite;
        }
        if role.outer_index == scope_index {
            role.role = InstructionRole::SemanticTarget;
        }
    });
    record.id = record.identity().expect("new observation identity");
    assert!(
        adapter_for(scope).is_none(),
        "the dependency has no economic adapter"
    );
    let resolved = record
        .resolve(&store)
        .expect("unknown protocol still resolves");
    let (execution, fidelity) = pipeline::baseline(&record, &resolved)
        .expect("unknown protocol executes with full historical fidelity");
    assert!(
        execution.success && fidelity.matched(),
        "semantic absence cannot veto replay"
    );
    assert_eq!(
        semantic_coverage_failure(&[ObservationCoverage {
            observation_id: record.id.clone(),
            subjects: Vec::new(),
        }]),
        Some(FailureReason::NoSemanticCoverage),
        "a faithful replay without an adapter must report missing semantic coverage"
    );
}
