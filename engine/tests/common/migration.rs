//! Shared Token Migration V1 test helpers: synthetic worlds built with the real
//! captured token programs, specs addressed to those worlds, and the reference and
//! defective candidate builds.
#![allow(dead_code)]
use eplyx_engine::{
    migration::{
        adapter,
        execute::{self, Bank, Session, UnitExecution},
        fixture::{self, FixtureContext, Recipe},
        planner::{self, MigrationPlan, PlanInput, RehearsalClockPolicy},
        spec::TokenMigrationV1,
        world::World,
    },
    replay::hash_bytes as sha256,
    standard_programs::token::{LEGACY_PROGRAM, TOKEN_2022_PROGRAM},
};
use serde_json::{json, Value};

pub const LEGACY: &str = LEGACY_PROGRAM;
pub const T22: &str = TOKEN_2022_PROGRAM;

pub fn candidate(name: &str) -> Vec<u8> {
    std::fs::read(eplyx_engine::repo_root().join(format!("artifacts/{name}.so")))
        .expect("run ./scripts/build-programs.sh")
}
pub fn reference() -> Vec<u8> {
    candidate("eplyx_token_migration")
}

pub fn recipe(value: Value) -> Recipe {
    let recipe: Recipe = serde_json::from_value(value).unwrap();
    recipe.validate().unwrap();
    recipe
}

pub fn address(recipe: &Recipe, label: &str) -> String {
    fixture::address_of(recipe, label)
}

/// A spec migrating `source` to `destination` of the recipe, with overrides merged.
pub fn spec(
    recipe: &Recipe,
    source_program: &str,
    destination_program: &str,
    patch: Value,
) -> TokenMigrationV1 {
    let mint = |label: &str| {
        recipe
            .mints
            .iter()
            .find(|m| m.label == label)
            .unwrap()
            .decimals
    };
    let mut value = json!({
        "version": 1,
        "source": {"mint": address(recipe, "source"), "tokenProgram": source_program, "decimals": mint("source")},
        "destination": {"mint": address(recipe, "destination"), "tokenProgram": destination_program, "decimals": mint("destination")},
        "conversion": {"ratioBasis": "raw", "numerator": "1", "denominator": "2", "rounding": "floor", "fee": {"kind": "none"}, "minimumOutputRaw": "1"},
        "eligibility": {"amountPolicy": "fullBalance", "minimumSourceBalanceRaw": "1", "holderAuthorization": ["owner", "delegate"], "ownerAuthorityClasses": ["wallet", "multisig"]},
        "sourceDisposition": {"kind": "burn"},
        "destinationFunding": {"kind": "reserveTransfer", "reserve": {"kind": "proposed", "fundedRaw": "1000000000000"}},
        "authorities": {"migrationAuthority": {"kind": "programDerived"}, "feePayer": {"kind": "relayer"}}
    });
    merge(&mut value, patch);
    let spec: TokenMigrationV1 = serde_json::from_value(snake_terms(value)).unwrap();
    spec.validate().unwrap();
    spec
}

pub fn merge(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                if v.is_object()
                    && t.get(&k).is_some_and(Value::is_object)
                    && !v.get("kind").is_some()
                {
                    merge(t.get_mut(&k).unwrap(), v);
                } else {
                    t.insert(k, v);
                }
            }
        }
        (t, p) => *t = p,
    }
}

pub fn change_spec_id(spec: &TokenMigrationV1) -> String {
    eplyx_engine::change::ChangeSpec::token_migration(
        spec.clone(),
        adapter::REFERENCE_PROGRAM_ID,
        &reference(),
    )
    .unwrap()
    .id()
    .unwrap()
}

/// Build a world whose recipe may reference the package's migration authority.
pub fn world(recipe: &Recipe, spec: &TokenMigrationV1) -> World {
    let overlay =
        adapter::derive(spec, &change_spec_id(spec), adapter::REFERENCE_PROGRAM_ID).unwrap();
    fixture::build(
        recipe,
        &FixtureContext {
            migration_authority: Some(overlay.migration_authority),
        },
    )
    .unwrap()
}

pub fn plan(spec: &TokenMigrationV1, world: &World, program: &[u8]) -> MigrationPlan {
    planner::plan(&PlanInput {
        spec,
        change_spec_id: &change_spec_id(spec),
        world,
        program_id: adapter::REFERENCE_PROGRAM_ID,
        candidate_program_sha256: &sha256(program),
        clock_policy: RehearsalClockPolicy::Activation,
        reserve_override: None,
        focus: None,
    })
    .unwrap()
}

/// Execute every attempted unit sequentially in one session.
pub fn rehearse(
    spec: &TokenMigrationV1,
    world: &World,
    plan: &MigrationPlan,
    program: &[u8],
) -> Vec<UnitExecution> {
    let resolved = spec.resolve().unwrap();
    let config =
        adapter::config_bytes(spec, &resolved, &plan.overlay, &plan.change_spec_id).unwrap();
    let bank = Bank::build(world, spec, plan, &config).unwrap();
    let programs = execute::programs(
        world,
        spec,
        adapter::REFERENCE_PROGRAM_ID,
        &resolved_candidate(spec, program),
    )
    .unwrap();
    execute::assert_candidate(&programs, adapter::REFERENCE_PROGRAM_ID, &sha256(program)).unwrap();
    let mut session = Session::new(&bank, &programs, adapter::REFERENCE_PROGRAM_ID).unwrap();
    plan.units
        .iter()
        .filter(|u| execute::attempted(u))
        .map(|u| execute::execute_unit(&mut session, spec, plan, u, &bank.relayer).unwrap())
        .collect()
}

pub fn wallets(labels: &[&str]) -> Value {
    Value::Array(
        labels
            .iter()
            .map(|l| json!({"label": l, "lamports": "1000000000"}))
            .collect(),
    )
}

pub fn temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "eplyx-migration-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Assemble a schema 3 package around a synthetic fixture recipe.
pub fn package(
    label: &str,
    recipe: &Recipe,
    spec: &TokenMigrationV1,
    program: &[u8],
    invariants: Vec<eplyx_engine::migration::invariants::MigrationInvariant>,
) -> (
    std::path::PathBuf,
    eplyx_engine::migration::input::ValidatedInput,
) {
    use eplyx_engine::migration::input::{self as package, Config, StateSource};
    let root = temp_dir(label);
    std::fs::create_dir_all(&root).unwrap();
    let recipe_bytes = eplyx_engine::canonical::document(recipe)
        .unwrap()
        .into_bytes();
    let config = Config {
        state: StateSource::SyntheticFixture {
            recipe: "fixture.json".into(),
            recipe_sha256: sha256(&recipe_bytes),
        },
        rehearsal_clock: RehearsalClockPolicy::Activation,
        max_rehearsal_units: 5000,
        max_captured_holders: 5000,
    };
    let dir = root.join("package");
    let validated = package::assemble(
        &dir,
        spec,
        adapter::REFERENCE_PROGRAM_ID,
        program,
        &config,
        Some(&recipe_bytes),
        invariants,
    )
    .unwrap();
    (root, validated)
}

pub fn resolved_candidate(
    spec: &TokenMigrationV1,
    bytes: &[u8],
) -> eplyx_engine::change::ResolvedCandidate {
    use eplyx_engine::change::{CandidateSource, ChangeSpec};
    ChangeSpec::token_migration(spec.clone(), adapter::REFERENCE_PROGRAM_ID, bytes)
        .unwrap()
        .resolve(CandidateSource::Bytes(bytes))
        .unwrap()
}

/// Translate only source terms in test construction. MAIN rejects legacy wire keys.
pub fn snake_terms(value: Value) -> Value {
    fn snake(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_ascii_uppercase() {
                out.push('_');
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(k, v)| (snake(&k), snake_terms(v)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(snake_terms).collect()),
        Value::String(s)
            if [
                "fullBalance",
                "sourceBps",
                "reserveTransfer",
                "programDerived",
                "unixTimestamp",
                "mintTo",
                "explicitSigner",
                "migrationAuthority",
            ]
            .contains(&s.as_str()) =>
        {
            Value::String(snake(&s))
        }
        other => other,
    }
}

/// T0's analytical contract. Only explicitly approved identity/encoding changes
/// are normalized; candidate bytes, statuses, equations and search findings remain.
pub fn assert_t0(label: &str, report: &Value, search: Value) {
    use eplyx_engine::migration::gate::{self, Policy};
    let after = json!({
        "label":label, "population":report["impact"]["population"], "classes":report["impact"]["classes"],
        "block_only":gate::evaluate_migration(report,Policy::BlockOnly).unwrap(),
        "strict":gate::evaluate_migration(report,Policy::Strict).unwrap(),
        "readiness":report["readiness"], "invariants":report["invariants"], "reconciliation":report["reconciliation"],
        "coverage":report["coverage"], "stress_cases":report["execution"]["stress_cases"],
        "unsigned_cross_check":report["unsigned_plan"]["cross_check"], "search":search,
        "world_sha256":report["state"]["world_sha256"], "change_spec_id":report["change_spec_id"],
        "candidate_program_sha256":report["candidate_program_sha256"],
        "official_transition":report["official_transition"], "funds_moved":report["funds_moved"],
    });
    let root = eplyx_engine::repo_root();
    let path = root.join("target/t3-reference");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join(format!("{label}.json")),
        eplyx_engine::canonical::document(&after).unwrap(),
    )
    .unwrap();
    let reference: Value = serde_json::from_slice(
        &std::fs::read(root.join(
            "docs/examples/phase-t0-stock-transition-integration/sta-migration-reference.json",
        ))
        .unwrap(),
    )
    .unwrap();
    let before = reference["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["label"] == label)
        .expect("frozen T0 case");
    // Candidate configuration and authority PDAs are seeded by the ChangeSpec ID.
    // Recompute both rather than masking arbitrary addresses in diagnostic prose.
    fn derived_addresses(id: &str, report: &Value) -> Vec<String> {
        let bytes: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let program = adapter::REFERENCE_PROGRAM_ID.parse().unwrap();
        let (config, _) = solana_address::Address::find_program_address(
            &[adapter::CONFIG_SEED, &bytes],
            &program,
        );
        let authority = solana_address::Address::find_program_address(
            &[adapter::AUTHORITY_SEED, config.as_ref()],
            &program,
        )
        .0
        .to_string();
        let reserve = eplyx_engine::migration::world::associated_token_address(
            &authority,
            report["migration"]["destination"]["token_program"]
                .as_str()
                .unwrap(),
            report["migration"]["destination"]["mint"].as_str().unwrap(),
        )
        .unwrap();
        vec![config.to_string(), authority, reserve]
    }
    let old_id = before["spec_sha256"].as_str().or_else(|| {
        (label == "spacex-demo-frozen")
            .then_some("0990126919d0d477d1d785b85a6d00b59340361d9568c9d513bce6334bb3f624")
    });
    let authorities: Vec<String> = [old_id, report["change_spec_id"].as_str()]
        .into_iter()
        .flatten()
        .flat_map(|id| derived_addresses(id, report))
        .collect();
    fn comparable(value: Value, authorities: &[String]) -> Value {
        match value {
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .filter(|(k, _)| {
                        ![
                            "world_sha256",
                            "derived_world_sha256",
                            "message_sha256",
                            "plan_sha256",
                            "stress_plan_sha256",
                            "transition_package_sha256",
                            "analysis_input_sha256",
                            "spec_sha256",
                            "change_spec_id",
                            "parent_run",
                            "evidence_refs",
                            "id",
                        ]
                        .contains(&k.as_str())
                    })
                    .map(|(k, v)| {
                        let v = if k == "observed_slots" {
                            match v {
                                Value::Array(xs) => Value::Array(
                                    xs.into_iter()
                                        .map(|v| {
                                            Value::String(
                                                v.as_str()
                                                    .map(str::to_string)
                                                    .unwrap_or_else(|| v.to_string()),
                                            )
                                        })
                                        .collect(),
                                ),
                                x => x,
                            }
                        } else {
                            v
                        };
                        (k, comparable(v, authorities))
                    })
                    .collect(),
            ),
            Value::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|v| comparable(v, authorities))
                    .collect(),
            ),
            Value::String(mut text) => {
                for authority in authorities {
                    text = text.replace(authority, "<change-derived-authority>");
                }
                Value::String(text)
            }
            other => other,
        }
    }
    let mut expected = before.clone();
    // The diagnostic frozen-world source export did not record its candidate;
    // its exact hash is independently pinned by the candidate build contract.
    if expected.get("candidate_program_sha256").is_none() {
        expected["candidate_program_sha256"] = after["candidate_program_sha256"].clone();
    }
    assert_eq!(
        comparable(after, &authorities),
        comparable(expected, &authorities),
        "{label}: T0 analytical contract"
    );
}
