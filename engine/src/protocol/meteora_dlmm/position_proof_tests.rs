use super::*;
fn scope() -> WithdrawalScope {
    WithdrawalScope {
        position_id: "position-a".into(),
        pool: "pool-a".into(),
        authority: "authority-a".into(),
        fixture_sha256: "bank-a".into(),
        lower_bin_id: -102,
        upper_bin_id: -53,
        bps_to_remove: 10000,
    }
}
fn proof() -> VerifiedWithdrawal {
    VerifiedWithdrawal {
        scope: scope(),
        path_type: ExitPathType::Withdrawal,
        status: PathStatus::Proven,
    }
}
#[test]
fn direct_holder_evidence_cannot_prove_withdrawal() {
    let mut p = proof();
    p.path_type = ExitPathType::SecondaryMarketExit;
    assert_eq!(
        resolve_position_paths(&scope(), Some(&p))[4].status,
        PathStatus::NotTested
    );
    p.path_type = ExitPathType::Transfer;
    assert_eq!(
        resolve_position_paths(&scope(), Some(&p))[4].status,
        PathStatus::NotTested
    );
}
#[test]
fn withdrawal_never_proves_official_transition_or_redemption() {
    let rows = resolve_position_paths(&scope(), Some(&proof()));
    assert_eq!(rows[0].status, PathStatus::NotTested);
    assert_eq!(rows[1].status, PathStatus::Unsupported);
    assert_eq!(rows[4].status, PathStatus::Proven);
}
#[test]
fn withdrawal_evidence_cannot_inherit_to_another_position() {
    let mut other = scope();
    other.position_id = "position-b".into();
    assert_eq!(
        resolve_position_paths(&other, Some(&proof()))[4].status,
        PathStatus::NotTested
    );
}
#[test]
fn pool_authority_bank_range_and_fraction_are_independent_scopes() {
    for field in 0..6 {
        let mut other = scope();
        match field {
            0 => other.pool = "pool-b".into(),
            1 => other.authority = "authority-b".into(),
            2 => other.fixture_sha256 = "bank-b".into(),
            3 => other.lower_bin_id += 1,
            4 => other.upper_bin_id -= 1,
            _ => other.bps_to_remove = 5000,
        }
        assert_eq!(
            resolve_position_paths(&other, Some(&proof()))[4].status,
            PathStatus::NotTested
        );
    }
}
#[test]
fn vault_presence_and_discovery_without_execution_remain_not_tested() {
    assert_eq!(
        resolve_position_paths(&scope(), None)[4].status,
        PathStatus::NotTested
    );
}
#[test]
fn generic_position_model_contains_no_asset_or_issuer_literals() {
    let text = include_str!("../../path/position/mod.rs");
    for name in ["PreANxuX", "SPACEX", "PreStocks", "prestocks", "741ZXY"] {
        assert!(!text.contains(name));
    }
}
