//! Offline tests: genuine protocol bytes plus explicitly synthetic direct accounts.
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Duration, Utc};
use eplyx_engine::{
    change::ChangeSpec,
    lifecycle::{
        consequence::{
            BalanceObservationBasis as B, LifecycleExecutionStatus,
            LifecycleImpactClassification as C, LifecycleImpactReport, TechnicalStateDiff,
        },
        exposure::{self, AdapterRun, EvidenceLayer},
        normalize,
        policy::{LifecycleScenario, LifecycleSourceKind, LifecycleStatus as S},
        AssetDescriptor, EntityType, LifecycleSnapshot, RpcEvidence,
    },
};
use serde_json::{json, Value};
use solana_address::Address;
use solana_program_pack::Pack;
use spl_token_2022_interface::{
    extension::{self as ext, BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut},
    state::{Account, AccountState},
};
use std::collections::BTreeMap;

fn time(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn address(n: u8) -> Address {
    Address::new_from_array([n; 32])
}
fn wallet() -> Address {
    let mut b = [0x66; 32];
    b[0] = 0x58;
    Address::new_from_array(b)
}
fn raw(bytes: &[u8], owner: &str) -> Value {
    json!({"owner":owner,"data":[STANDARD.encode(bytes),"base64"],"executable":false,
        "lamports":10000000,"rentEpoch":18446744073709551615u64,"space":bytes.len()})
}
fn snapshot() -> LifecycleSnapshot {
    let mut f: Value = serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/fixtures/meteora-dlmm-spacex.json"
    ))
    .unwrap();
    let mint: Address = f["baseline"]["asset"]["mint"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let program = f["baseline"]["evidence"][2]["params"][0]
        .as_str()
        .unwrap()
        .to_string();
    let mut owners = BTreeMap::new();
    let pool_owner = f["adapter_run"]["candidate_pool"]
        .as_str()
        .unwrap()
        .to_string();
    owners.insert(
        pool_owner,
        f["baseline"]["evidence"][3]["result"]["value"][0].clone(),
    );
    owners.insert(
        wallet().to_string(),
        raw(&[], "11111111111111111111111111111111"),
    );
    owners.insert(address(77).to_string(), Value::Null);
    for (id, amount, owner, confidential) in [
        (71, 1000000000, wallet(), false),
        (72, 2000000000, address(77), false),
        (73, 0, wallet(), false),
        (74, 0, wallet(), true),
    ] {
        let account = Account {
            mint,
            owner,
            amount,
            state: AccountState::Initialized,
            ..Account::default()
        };
        let bytes = if confidential {
            let mut bytes = vec![
                0;
                ExtensionType::try_calculate_account_len::<Account>(&[
                    ExtensionType::ConfidentialTransferAccount
                ])
                .unwrap()
            ];
            let mut state =
                StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut bytes).unwrap();
            state
                .init_extension::<ext::confidential_transfer::ConfidentialTransferAccount>(false)
                .unwrap();
            state.base = account;
            state.pack_base();
            state.init_account_type().unwrap();
            bytes
        } else {
            let mut bytes = vec![0; Account::LEN];
            Account::pack(account, &mut bytes).unwrap();
            bytes
        };
        f["baseline"]["evidence"][2]["result"]["value"]
            .as_array_mut()
            .unwrap()
            .push(json!({"pubkey":address(id).to_string(),"account":raw(&bytes,&program)}));
    }
    f["baseline"]["evidence"][3]["params"][0] = json!(owners.keys().collect::<Vec<_>>());
    f["baseline"]["evidence"][3]["result"]["value"] = json!(owners.values().collect::<Vec<_>>());
    let b = &f["baseline"];
    let mut s = normalize(
        serde_json::from_value::<AssetDescriptor>(b["asset"].clone()).unwrap(),
        b["captured_at"].as_str().unwrap().into(),
        b["rpc_origin"].as_str().unwrap().into(),
        serde_json::from_value::<Vec<RpcEvidence>>(b["evidence"].clone()).unwrap(),
    )
    .unwrap();
    let run: AdapterRun = serde_json::from_value(f["adapter_run"].clone()).unwrap();
    let graph = exposure::normalize_graph(&s, vec![run]).unwrap();
    s.schema_version = 2;
    s.exposures = Some(graph);
    s
}
fn scenario() -> LifecycleScenario {
    serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/scenarios/spacex-transition.json"
    ))
    .unwrap()
}
fn evaluate(
    s: &LifecycleSnapshot,
    p: &LifecycleScenario,
    at: DateTime<Utc>,
) -> LifecycleImpactReport {
    ChangeSpec::lifecycle(p)
        .unwrap()
        .compare_lifecycle(s, p, p.policy.effective_at - Duration::nanoseconds(1), at)
        .unwrap()
}
fn direct(
    r: &LifecycleImpactReport,
    n: u8,
) -> &eplyx_engine::lifecycle::consequence::LifecycleImpact {
    r.entities
        .iter()
        .find(|e| e.token_account == address(n).to_string())
        .unwrap()
}

#[test]
fn active_before_effective_time_preserves_meaning() {
    let p = scenario();
    let s = snapshot();
    let r = evaluate(&s, &p, p.policy.effective_at - Duration::nanoseconds(1));
    assert_eq!(r.before.lifecycle_status, S::Active);
    assert_eq!(r.after.lifecycle_status, S::Active);
    assert_eq!(direct(&r, 71).impact_classification, C::Unaffected);
    assert!(!r.diff.lifecycle_status_changed);
    assert_eq!(r.summary.economic_meaning_changed_entities, 0);
}

#[test]
fn exact_effective_boundary_changes_same_positive_wallet_without_mutating_bytes() {
    let p = scenario();
    let s = snapshot();
    let bytes = s.to_json().unwrap();
    let r = evaluate(&s, &p, p.policy.effective_at);
    let e = direct(&r, 71);
    assert_eq!(e.entity_type, EntityType::WalletCompatible);
    assert!(!e.role_uncertain);
    assert_eq!(e.balance.raw, "1000000000");
    assert_eq!(e.balance.decimal_base_units, "1");
    assert_eq!(e.pre_lifecycle_status, S::Active);
    assert_eq!(e.post_lifecycle_status, S::TransitionRequired);
    assert_eq!(e.pre_classification, C::Unaffected);
    assert_eq!(e.impact_classification, C::RequiresTransition);
    assert!(e.economic_meaning_changed);
    assert_eq!(e.execution_status, LifecycleExecutionStatus::NotTested);
    assert_eq!(s.to_json().unwrap(), bytes);
    assert_eq!(r.before.snapshot_sha256, r.after.snapshot_sha256);
    assert_eq!(r.diff.technical_state, TechnicalStateDiff::Unchanged);
}

#[test]
fn inclusive_deadline_ends_entitlement_without_claiming_stranding_or_available_conversion() {
    let p = scenario();
    let s = snapshot();
    let deadline = p.policy.deadline.as_ref().unwrap().at;
    assert_eq!(
        p.policy.status_at(deadline - Duration::nanoseconds(1)),
        S::TransitionRequired
    );
    let r = evaluate(&s, &p, deadline);
    assert_eq!(r.after.lifecycle_status, S::NoIssuerEntitlement);
    assert_eq!(direct(&r, 71).impact_classification, C::StaleExposure);
    assert_eq!(r.summary.classifications[&C::RequiresTransition], 0);
    assert!(!r.to_json().unwrap().contains("\"Stranded\""));
}

#[test]
fn zero_public_balance_is_unaffected_but_confidential_zero_is_unresolved() {
    let p = scenario();
    let r = evaluate(&snapshot(), &p, p.policy.effective_at);
    assert_eq!(direct(&r, 73).impact_classification, C::Unaffected);
    assert!(!direct(&r, 73).economic_meaning_changed);
    assert_eq!(direct(&r, 74).impact_classification, C::Unresolved);
    assert_eq!(r.summary.entities_evaluated, 5);
    assert_eq!(r.summary.positive_balance_entities, 3);
    assert_eq!(r.summary.zero_public_balance_entities, 2);
    assert_eq!(r.summary.zero_public_balance_unresolved, 1);
    assert_eq!(r.summary.classifications[&C::Unaffected], 1);
}

#[test]
fn verified_vault_is_stale_and_old_and_current_amounts_never_merge() {
    let p = scenario();
    let r = evaluate(&snapshot(), &p, p.policy.effective_at);
    let current = &r.protocol_observations[0];
    let original = r
        .entities
        .iter()
        .find(|e| e.entity_id == current.entity_id)
        .unwrap();
    assert_eq!(original.verified_role.as_deref(), Some("LiquidityVault"));
    assert!(!original.role_uncertain);
    assert_eq!(original.impact_classification, C::StaleExposure);
    assert_eq!(current.impact_classification, C::StaleExposure);
    assert_eq!(original.entity_type, EntityType::ProgramOwnedAuthority);
    assert_eq!(original.balance.raw, "678320992");
    assert_eq!(original.balance.basis, B::Phase2TokenAccount);
    assert_eq!(current.balance.raw, "682216229");
    assert_eq!(current.balance.decimal_base_units, "0.682216229");
    assert_eq!(current.balance.basis, B::Phase3VerifiedVault);
    assert_eq!(r.summary.phase2_public_raw_exposure, "3678320992");
    assert_eq!(
        r.summary.phase3_verified_vault_public_raw_exposure,
        "682216229"
    );
    assert_eq!(r.summary.stale_protocol_exposures, 1);
}

#[test]
fn positive_unknown_role_is_transition_required_without_inventing_role() {
    let p = scenario();
    let r = evaluate(&snapshot(), &p, p.policy.effective_at);
    let e = direct(&r, 72);
    assert_eq!(e.entity_type, EntityType::Unknown);
    assert!(e.role_uncertain);
    assert_eq!(e.verified_role, None);
    assert!(!e.authority_observation.account_exists);
    assert_eq!(e.impact_classification, C::RequiresTransition);
    assert_eq!(r.summary.unknown_role_positive_entities, 1);
}

#[test]
fn rpc_evidence_and_policy_assumptions_are_distinct_and_resolvable() {
    let p = scenario();
    let s = snapshot();
    let r = evaluate(&s, &p, p.policy.effective_at);
    let e = direct(&r, 71);
    assert!(e
        .onchain_evidence
        .iter()
        .all(|proof| proof.layer == EvidenceLayer::Lifecycle));
    for proof in &e.onchain_evidence {
        assert!(s.evidence[proof.reference.rpc_id]
            .result
            .pointer(&proof.reference.pointer)
            .is_some());
    }
    for evidence in &e.lifecycle_evidence {
        let source = r
            .scenario
            .sources
            .iter()
            .find(|s| s.id == evidence.source_id)
            .unwrap();
        assert!(evidence
            .policy_fields
            .iter()
            .all(|field| source.supports.contains(field)));
    }
    assert!(r
        .scenario
        .sources
        .iter()
        .any(|s| s.kind == LifecycleSourceKind::ExternalPolicy));
    assert!(r
        .scenario
        .sources
        .iter()
        .any(|s| s.kind == LifecycleSourceKind::ScenarioAssumption));
    assert_eq!(r.scenario_sha256, p.sha256().unwrap());
}

#[test]
fn same_frozen_world_produces_reproducible_before_after_diff_offline() {
    let p = scenario();
    let s = snapshot();
    let first = evaluate(&s, &p, p.policy.effective_at);
    let second = evaluate(&s, &p, p.policy.effective_at);
    assert_eq!(first, second);
    assert_eq!(first.to_json().unwrap(), second.to_json().unwrap());
    assert!(first.diff.lifecycle_status_changed);
    assert_eq!(first.diff.economic_meaning_changed_entities, 3);
    assert_eq!(first.diff.economic_meaning_changed_protocol_observations, 1);
    first.validate(&s).unwrap();
}

#[test]
fn deterministic_serialization_roundtrip_and_tamper_detection() {
    let p = scenario();
    let s = snapshot();
    let r = evaluate(&s, &p, p.policy.effective_at);
    let bytes = r.to_json().unwrap();
    let decoded: LifecycleImpactReport = serde_json::from_str(&bytes).unwrap();
    decoded.validate(&s).unwrap();
    assert_eq!(decoded.to_json().unwrap(), bytes);
    for variant in 0..5 {
        let mut bad = r.clone();
        match variant {
            0 => bad.summary.positive_balance_entities += 1,
            1 => bad.entities[0].balance.raw = "0".into(),
            2 => bad.entities[0].onchain_evidence[0].raw_data_sha256 = "changed".into(),
            3 => bad.entities[0].entity_type = EntityType::Unknown,
            _ => bad.scenario_sha256 = "changed".into(),
        }
        assert!(bad.validate(&s).is_err(), "variant {variant}");
    }
    let path = std::env::temp_dir().join(format!("eplyx-impact-{}.json", std::process::id()));
    r.save(&path).unwrap();
    assert!(r.save(&path).is_err());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn unknown_policy_optional_deadline_and_legacy_snapshot_remain_conservative() {
    let mut p = scenario();
    let mut s = snapshot();
    p.policy.after = S::Unknown;
    let unknown = evaluate(&s, &p, p.policy.effective_at);
    assert_eq!(direct(&unknown, 71).impact_classification, C::Unresolved);
    p.policy.after = S::TransitionRequired;
    p.policy.deadline = None;
    for source in &mut p.sources {
        source.supports.retain(|f| f != "/policy/deadline");
    }
    s.schema_version = 1;
    s.exposures = None;
    let r = evaluate(&s, &p, time("2030-01-01T00:00:00Z"));
    assert_eq!(r.after.lifecycle_status, S::TransitionRequired);
    assert!(r.protocol_observations.is_empty());
    assert_eq!(r.summary.classifications[&C::RequiresTransition], 3);
}

#[test]
fn malformed_provenance_wrong_asset_reversed_times_and_mismatched_change_fail() {
    let p = scenario();
    let s = snapshot();
    for variant in 0..6 {
        let mut bad = p.clone();
        match variant {
            0 => bad.sources.clear(),
            1 => bad.sources[0].id = bad.sources[1].id.clone(),
            2 => bad.policy.deadline.as_mut().unwrap().at = bad.policy.effective_at,
            3 => bad.sources[0].supports.push("/policy/not-a-field".into()),
            4 => bad.policy.asset_mint = "invalid-mint".into(),
            _ => bad.sources[0].content_sha256 = None,
        }
        assert!(bad.validate().is_err(), "variant {variant}");
    }
    let c = ChangeSpec::lifecycle(&p).unwrap();
    let mut wrong = p.clone();
    wrong.policy.asset_mint = address(99).to_string();
    assert!(c
        .compare_lifecycle(&s, &wrong, p.policy.effective_at, p.policy.effective_at)
        .is_err());
    assert!(c
        .compare_lifecycle(
            &s,
            &p,
            p.policy.effective_at + Duration::seconds(1),
            p.policy.effective_at
        )
        .is_err());
    wrong = p.clone();
    wrong.policy.before = S::Unknown; // A different semantic proposal; display descriptions are outside MAIN identity.
    assert!(c
        .compare_lifecycle(&s, &wrong, p.policy.effective_at, p.policy.effective_at)
        .is_err());
}

#[test]
fn local_policy_capture_load_checks_hash_without_network() {
    let root = eplyx_engine::lifecycle::artifact::reference_root();
    LifecycleScenario::load(&root.join("scenarios/spacex-transition.json")).unwrap();
    let directory = std::env::temp_dir().join(format!("eplyx-policy-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut p = scenario();
    p.sources[0].artifact = Some("notice.html".into());
    std::fs::write(directory.join("notice.html"), "tampered source").unwrap();
    std::fs::write(directory.join("scenario.json"), p.to_json().unwrap()).unwrap();
    assert!(LifecycleScenario::load(&directory.join("scenario.json"))
        .unwrap_err()
        .to_string()
        .contains("hash mismatch"));
    std::fs::remove_dir_all(directory).unwrap();
}
