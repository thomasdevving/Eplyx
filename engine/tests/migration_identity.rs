//! MAIN proposal/state boundaries and fail-closed executable resolution.
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    change::{Activation, CandidateSource, ChangeKind, ChangeSpec, ChangeTarget},
    migration::{
        adapter, fixture::Recipe, input, invariants::MigrationInvariant, spec::TokenMigrationV1,
    },
    repo_root,
};
use migration_common::*;

fn minimal() -> (Recipe, TokenMigrationV1) {
    let root = repo_root().join("examples/migrations/minimal");
    (
        Recipe::parse(&std::fs::read(root.join("fixtures/world.json")).unwrap()).unwrap(),
        serde_json::from_slice(&std::fs::read(root.join("migration.json")).unwrap()).unwrap(),
    )
}

#[test]
fn main_identity_owns_terms_mechanism_and_activation_but_not_metadata_or_state() {
    let (recipe, terms) = minimal();
    let bytes = reference();
    let mut spec =
        ChangeSpec::token_migration(terms.clone(), adapter::REFERENCE_PROGRAM_ID, &bytes).unwrap();
    let id = spec.id().unwrap();
    assert_eq!(spec.kind(), ChangeKind::TokenMigration);
    assert!(matches!(spec.target(), ChangeTarget::Asset(_)));
    assert!(
        spec.target_program_id().is_none()
            && spec.as_program_upgrade().is_none()
            && spec.delivery().is_none()
    );
    assert_eq!(
        spec.as_token_migration()
            .unwrap()
            .evaluation_spec(spec.activation.as_ref())
            .unwrap(),
        terms
    );
    spec.metadata.label = Some("renamed".into());
    assert_eq!(spec.id().unwrap(), id);
    spec.activation = Some(Activation {
        slot: Some(1),
        unix_timestamp: Some(2),
    });
    assert!(
        spec.validate().is_err(),
        "migration activation has one time axis"
    );
    spec.activation = None;
    assert_ne!(spec.id().unwrap(), id);
    let (_, a) = package(
        "identity-full",
        &recipe,
        &terms,
        &bytes,
        MigrationInvariant::recommended(),
    );
    let (_, b) = package("identity-no-invariants", &recipe, &terms, &bytes, vec![]);
    assert_eq!(a.change_spec_id(), b.change_spec_id());
    assert_ne!(a.analysis_input_sha256(), b.analysis_input_sha256());
    assert!(a.state().config == b.state().config);
    let wire = serde_json::to_value(a.change()).unwrap();
    assert!(wire["change"].get("window").is_none());
    assert!(wire["change"].get("deadline").is_some());
    assert!(wire["change"]["mechanism"]["artifact"]["sha256"].is_string());
    let mut altered = bytes.clone();
    altered[100] ^= 1;
    assert!(a
        .change()
        .resolve(CandidateSource::Bytes(&altered))
        .is_err());
    assert!(a
        .change()
        .resolve(CandidateSource::Bytes(&bytes[..bytes.len() - 1]))
        .is_err());
    assert_eq!(
        a.change()
            .resolve(CandidateSource::Store(a.root()))
            .unwrap()
            .bytes(),
        bytes
    );
}

#[test]
fn state_and_executable_tampering_are_refused_before_any_capture() {
    let (recipe, terms) = minimal();
    let bytes = reference();
    let (_, input) = package(
        "identity-tamper",
        &recipe,
        &terms,
        &bytes,
        MigrationInvariant::recommended(),
    );
    let program = input.root().join("programs").join(input.program_sha256());
    let mut altered = bytes.clone();
    altered[100] ^= 1;
    std::fs::write(&program, altered).unwrap();
    assert!(input::load(input.root()).is_err());
    std::fs::write(program, bytes).unwrap();
    let fixture = input.root().join("fixture.json");
    let mut changed = std::fs::read(&fixture).unwrap();
    changed.push(b' ');
    std::fs::write(fixture, changed).unwrap();
    assert!(input::load(input.root()).is_err());
    let mut state = input.state().clone();
    if let input::StateSource::SyntheticFixture { recipe, .. } = &mut state.config.state {
        *recipe = "../fixture.json".into();
    }
    assert!(input::validate_state(&mut state).is_err());
}
