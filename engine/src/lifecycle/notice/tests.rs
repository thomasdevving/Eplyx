use super::*;
use std::path::PathBuf;
use workflow::NoticeWorkflow;
fn base() -> PathBuf {
    crate::lifecycle::artifact::reference_root().join("probes")
}
fn workflow() -> NoticeWorkflow {
    NoticeWorkflow::load(&base().join("spacex-notice-workflow.json")).unwrap()
}
fn source() -> CapturedSourceDocument {
    workflow().source(&base()).unwrap()
}
fn parsed() -> NormalizedLifecycleEvent {
    prestocks::CapturedIssuerNotice
        .normalize(&source())
        .unwrap()
}
fn verified() -> NormalizedLifecycleEvent {
    workflow().normalize(&base()).unwrap().event().clone()
}
fn regenerated(mut s: CapturedSourceDocument, raw: String) -> CapturedSourceDocument {
    s.raw_content = raw;
    s.content_sha256 = sha256(s.raw_content.as_bytes());
    s.raw_content_ref.sha256 = s.content_sha256.clone();
    s
}
#[test]
fn source_digest_mismatch_is_rejected() {
    let mut s = source();
    s.raw_content.push(' ');
    assert!(
        s.validate().is_err(),
        "changed raw source must fail digest verification"
    );
}
#[test]
fn successor_mint_keeps_exact_provenance() {
    let s = source();
    let e = parsed();
    let p = &e.successor_asset.issuer_asserted_mint.provenance[0];
    let [a, b] = p.byte_range.unwrap();
    assert_eq!(p.exact_text.as_deref(), Some(&s.raw_content[a..b]));
    assert!(s.raw_content[a..b].ends_with(&e.successor_asset.issuer_asserted_mint.value));
    assert_eq!(p.source_digest, s.content_sha256);
}
#[test]
fn deadline_keeps_exact_provenance() {
    let s = source();
    let e = parsed();
    assert_eq!(e.deadline.value.to_rfc3339(), "2027-03-12T23:59:00+00:00");
    let p = &e.deadline.provenance[0];
    let [a, b] = p.byte_range.unwrap();
    assert_eq!(p.exact_text.as_deref(), Some(&s.raw_content[a..b]));
    assert!(s.raw_content[a..b].contains("11:59pm UTC on 12 March 2027"));
}
#[test]
fn missing_required_deadline_fails() {
    let s = source();
    let raw = s
        .raw_content
        .replace(" before 11:59pm UTC on 12 March 2027", " soon");
    assert!(prestocks::CapturedIssuerNotice
        .normalize(&regenerated(s, raw))
        .is_err());
}
#[test]
fn issuer_mint_never_verifies_itself() {
    let e = parsed();
    assert_eq!(
        e.successor_identity.status,
        IdentityStatus::Unknown,
        "published address alone is not an observed mint"
    );
    assert_eq!(e.successor_identity.observed_mint, None);
    assert_eq!(
        e.successor_asset.issuer_asserted_mint.classifications,
        vec![ProvenanceClass::IssuerAsserted]
    );
}
#[test]
fn independent_raw_chain_confirms_both_mints() {
    let e = verified();
    assert_eq!(e.source_identity.status, IdentityStatus::Verified);
    assert_eq!(e.successor_identity.status, IdentityStatus::Verified);
    assert_eq!(
        e.successor_identity.observed_symbol.as_deref(),
        Some("SPCXx")
    );
    assert_eq!(e.successor_identity.slot, Some(448067723));
    assert!(e
        .successor_asset
        .issuer_asserted_mint
        .classifications
        .contains(&ProvenanceClass::OnChainVerified));
}
#[test]
fn wrong_successor_chain_identity_is_rejected() {
    let w = workflow();
    let r: crate::lifecycle::notice::identity::IdentityReport =
        serde_json::from_slice(&w.identity_report.read(&base()).unwrap()).unwrap();
    let mut assertion = parsed().successor_asset.issuer_asserted_mint;
    assertion.value = r.source_verification.mint.clone();
    assert!(
        workflow::verify_identity(&mut assertion, &r.successor_verification, &base()).is_err(),
        "independent mint mismatch cannot be trusted"
    );
}
#[test]
fn issuer_wording_never_proves_official_transition() {
    let e = parsed();
    assert_eq!(
        e.official_execution_status.value,
        PathStatus::NotTested,
        "swap language supplies no transaction proof"
    );
}
#[test]
fn unknown_mechanism_stays_unknown() {
    assert_eq!(
        parsed().official_mechanism.value,
        MechanismType::Unknown,
        "no exact program or plan is stated"
    );
}
#[test]
fn unrelated_data_never_infers_conversion_ratio() {
    assert_eq!(
        parsed().conversion_ratio.value,
        None,
        "page price and mint decimals cannot imply a conversion ratio"
    );
}
#[test]
fn demo_time_never_becomes_issuer_assertion() {
    let w = workflow();
    let e = verified();
    assert_eq!(e.effective_at.value, None);
    let (_, b) = w.generate(&base(), &e).unwrap();
    assert_eq!(
        b.fields["/policy/effective_at"].classifications,
        vec![ProvenanceClass::DemoConfigured],
        "hypothetical boundary must remain demo configuration"
    );
}
#[test]
fn scenario_generation_is_deterministic() {
    let w = workflow();
    let e = verified();
    let (s, a) = w.generate(&base(), &e).unwrap();
    let (t, b) = w.generate(&base(), &e).unwrap();
    assert_eq!(s, t);
    assert_eq!(a, b);
    assert!(s.policy.successor.is_some());
}
#[test]
fn ordering_and_irrelevant_markup_preserve_semantics() {
    let s = source();
    let raw = s
        .raw_content
        .replace(
            "property=\"og:site_name\" content=\"PreStocks\"",
            "content='PreStocks' property='og:site_name'",
        )
        .replace(
            "<span class=\"block\">SpaceX PreStocks",
            "<!-- irrelevant --><span class='different'>SpaceX PreStocks",
        );
    let changed = prestocks::CapturedIssuerNotice
        .normalize(&regenerated(s, raw))
        .unwrap();
    assert_eq!(
        parsed().semantic_sha256().unwrap(),
        changed.semantic_sha256().unwrap()
    );
    assert_ne!(
        parsed().source_content_sha256,
        changed.source_content_sha256
    );
}
#[test]
fn altered_deadline_without_regeneration_is_rejected() {
    let w = workflow();
    let mut e = verified();
    e.deadline.value += chrono::Duration::days(1);
    assert!(
        w.verify_event(&base(), &e).is_err(),
        "normalized deadline cannot bypass source regeneration"
    );
}
#[test]
fn changed_mint_or_provenance_is_rejected() {
    let w = workflow();
    let mut e = verified();
    e.successor_asset.issuer_asserted_mint.provenance[0].pointer = "/other".into();
    assert!(w.verify_event(&base(), &e).is_err());
    e = verified();
    e.successor_asset.issuer_asserted_mint.value =
        e.source_asset.issuer_asserted_mint.value.clone();
    assert!(w.verify_event(&base(), &e).is_err());
}
#[test]
fn manual_scenario_or_binding_tampering_fails() {
    let w = workflow();
    let e = verified();
    let (mut s, b) = w.generate(&base(), &e).unwrap();
    s.policy.deadline.as_mut().unwrap().at += chrono::Duration::minutes(1);
    assert!(w.verify_scenario(&base(), &e, &s, &b).is_err());
    let (s, mut b) = w.generate(&base(), &e).unwrap();
    b.fields
        .get_mut("/policy/effective_at")
        .unwrap()
        .classifications = vec![ProvenanceClass::IssuerAsserted];
    assert!(w.verify_scenario(&base(), &e, &s, &b).is_err());
}
#[test]
fn alternate_destination_and_assertion_limits_are_preserved() {
    let e = parsed();
    assert_eq!(e.alternate_destination.value, "any other token");
    assert_eq!(e.source_asset.symbol.value, None);
    assert_eq!(
        e.issuer_assertions["public_listing"].classifications,
        vec![ProvenanceClass::IssuerAsserted]
    );
    assert_eq!(source().http_status, None);
    assert_eq!(source().content_type, None);
}
#[test]
fn script_strings_cannot_supply_notice_fields() {
    let s = source();
    let raw = s
        .raw_content
        .replace("tokens must be swapped into", "tokens might change");
    let mut s = regenerated(s, raw);
    s.raw_content
        .push_str("<script>SpaceX PreStocks tokens must be swapped into</script>");
    s.content_sha256 = sha256(s.raw_content.as_bytes());
    s.raw_content_ref.sha256 = s.content_sha256.clone();
    assert!(prestocks::CapturedIssuerNotice.normalize(&s).is_err());
}
#[test]
fn strict_event_and_config_parsing_rejects_unknown_fields() {
    let mut v = serde_json::to_value(parsed()).unwrap();
    v["invented"] = true.into();
    assert!(serde_json::from_value::<NormalizedLifecycleEvent>(v).is_err());
}

#[test]
fn missing_chain_artifact_cannot_grant_identity() {
    let w = workflow();
    let r: crate::lifecycle::notice::identity::IdentityReport =
        serde_json::from_slice(&w.identity_report.read(&base()).unwrap()).unwrap();
    let mut mint = r.successor_verification;
    mint.evidence.artifact.file = "missing-chain-account.json".into();
    let mut assertion = parsed().successor_asset.issuer_asserted_mint;
    assert!(
        workflow::verify_identity(&mut assertion, &mint, &base()).is_err(),
        "issuer and published metadata alone cannot establish a raw on-chain identity"
    );
}

#[test]
fn changed_economic_semantics_cannot_reuse_frozen_proofs() {
    let original =
        LifecycleScenario::load(&base().join("../scenarios/spacex-transition.json")).unwrap();
    let w = workflow();
    let e = verified();
    let (generated, _) = w.generate(&base(), &e).unwrap();
    workflow::check_compatibility(&original, &generated, &e).unwrap();
    let mut changed = generated.clone();
    changed.policy.effective_at += chrono::Duration::seconds(1);
    assert!(
        workflow::check_compatibility(&original, &changed, &e).is_err(),
        "changed boundary needs new proof/policy binding"
    );
    changed = generated.clone();
    changed.policy.deadline.as_mut().unwrap().at += chrono::Duration::days(1);
    assert!(
        workflow::check_compatibility(&original, &changed, &e).is_err(),
        "changed deadline needs new proof/policy binding"
    );
    changed = generated;
    changed.policy.successor.as_mut().unwrap().mint =
        e.source_asset.issuer_asserted_mint.value.clone();
    assert!(
        workflow::check_compatibility(&original, &changed, &e).is_err(),
        "different successor cannot inherit identity"
    );
}
