//! Deterministic state-corpus generation for the fixture lending protocol.
//!
//! This is the one module besides `interpret` that knows what a lending position
//! *is*. In a later phase the corpus would be derived from mainnet state; the
//! fixture format it emits is the same either way, which is the point of keeping
//! this seam clean.
//!
//! Generation is a pure function of the fixture index - no RNG, no clock, no
//! filesystem - so the corpus is bit-identical on every machine and every run.
//!
//! # Corpus design
//!
//! The seeded V2 regression truncates *fractional* SOL when valuing collateral.
//! The corpus is therefore built around that fault line deliberately:
//!
//! * positions holding a whole number of SOL are unaffected, and form the
//!   majority (they are what ordinary round-number deposits produce);
//! * positions holding fractional SOL are mis-valued by up to ~1 SOL of
//!   collateral, but only *change outcome* if they sit near a threshold;
//! * the boundary families are arithmetically tuned so that a known, exact set
//!   of fixtures crosses a threshold under V2 and no others do.
//!
//! The expected classifications are asserted in `engine/tests/upgrade_diff.rs`,
//! so a change in the program that shifted these numbers would fail the suite
//! rather than silently rewriting the baseline.

use fixture_lending_interface::{
    reference, LendingInstruction, Market, Position, ACCOUNT_TAG_MARKET, ACCOUNT_TAG_POSITION,
    MARKET_LEN, POSITION_LEN, VAULT_SEED,
};
use solana_address::Address;
use solana_keypair::Keypair;
use solana_rent::Rent;
use solana_signer::Signer;

use crate::types::{
    AccountMetaSpec, AccountSnapshot, Category, Fixture, InstructionSpec, KeypairSpec, NamedAccount,
};

pub const SOL: u64 = 1_000_000_000;
pub const USD: u64 = 1_000_000;

/// $100.00 per SOL. A round price keeps the worked examples in the README exact.
pub const PRICE: u64 = 100 * USD;
pub const LIQUIDATION_THRESHOLD_BPS: u16 = 8_000;
pub const MAX_LTV_BPS: u16 = 7_500;

pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

const OWNER_FUNDING: u64 = 5_000 * SOL;
const PAYER_FUNDING: u64 = 100 * SOL;
const LIQUIDATOR_FUNDING: u64 = 1_000 * SOL;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Refresh,
    Withdraw(u64),
    Deposit(u64),
    Borrow(u64),
    Repay(u64),
    Liquidate(u64),
}

impl Action {
    fn describe(self) -> String {
        match self {
            Action::Refresh => "refresh position".to_string(),
            Action::Withdraw(a) => format!("withdraw {} SOL collateral", fmt_sol(a)),
            Action::Deposit(a) => format!("deposit {} SOL collateral", fmt_sol(a)),
            Action::Borrow(a) => format!("borrow {} USD", fmt_usd(a)),
            Action::Repay(a) => format!("repay {} USD", fmt_usd(a)),
            Action::Liquidate(a) => format!("liquidate, repaying {} USD", fmt_usd(a)),
        }
    }

    fn encode(self) -> Vec<u8> {
        let instruction = match self {
            Action::Refresh => LendingInstruction::RefreshPosition,
            Action::Withdraw(amount) => LendingInstruction::WithdrawCollateral { amount },
            Action::Deposit(amount) => LendingInstruction::DepositCollateral { amount },
            Action::Borrow(amount) => LendingInstruction::Borrow { amount },
            Action::Repay(amount) => LendingInstruction::Repay { amount },
            Action::Liquidate(repay_amount) => LendingInstruction::Liquidate { repay_amount },
        };
        borsh::to_vec(&instruction).expect("instruction encoding is infallible")
    }
}

pub fn fmt_sol(lamports: u64) -> String {
    format!("{}.{:09}", lamports / SOL, lamports % SOL)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub fn fmt_usd(micro: u64) -> String {
    format!("{}.{:06}", micro / USD, micro % USD)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

struct Spec {
    id: String,
    category: Category,
    notes: String,
    collateral: u64,
    debt: u64,
    action: Action,
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Deterministic 32-byte seed for a (fixture, role) pair. Any 32 bytes are a
/// valid ed25519 seed, so no rejection sampling is needed.
fn seed_for(id: &str, role: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for chunk in 0..4usize {
        let hash = fnv1a(format!("{id}/{role}/{chunk}").as_bytes());
        out[chunk * 8..(chunk + 1) * 8].copy_from_slice(&hash.to_le_bytes());
    }
    out
}

fn keypair_spec(id: &str, role: &str) -> (KeypairSpec, Address) {
    let seed = seed_for(id, role);
    let keypair = Keypair::new_from_array(seed);
    let address = keypair.pubkey();
    (
        KeypairSpec {
            label: role.to_string(),
            seed: seed.to_vec(),
            address: address.to_string(),
        },
        address,
    )
}

fn system_account(lamports: u64) -> AccountSnapshot {
    AccountSnapshot {
        lamports,
        owner: SYSTEM_PROGRAM.to_string(),
        data: Vec::new(),
        executable: false,
        rent_epoch: 0,
    }
}

fn build(spec: Spec, program_id: &Address) -> Fixture {
    let rent = Rent::default();
    let id = spec.id;

    let (payer_kp, payer) = keypair_spec(&id, "payer");
    let (owner_kp, owner) = keypair_spec(&id, "owner");
    let (authority_kp, authority) = keypair_spec(&id, "authority");
    let (liquidator_kp, liquidator) = keypair_spec(&id, "liquidator");

    // Market and position are program-owned data accounts that never sign, so
    // their addresses are taken straight from the seed rather than a keypair.
    let market = Address::new_from_array(seed_for(&id, "market"));
    let position = Address::new_from_array(seed_for(&id, "position"));
    let (vault, _bump) = Address::find_program_address(&[VAULT_SEED, market.as_ref()], program_id);

    let market_state = Market {
        tag: ACCOUNT_TAG_MARKET,
        version: 1,
        authority: authority.to_bytes(),
        vault: vault.to_bytes(),
        collateral_price: PRICE,
        liquidation_threshold_bps: LIQUIDATION_THRESHOLD_BPS,
        max_ltv_bps: MAX_LTV_BPS,
        position_count: 1,
    };

    // The cached health factor in the *initial* state is computed with the
    // reference (correct) math: the position is modelled as having last been
    // touched by V1, which is exactly the situation an upgrade walks into.
    let position_state = Position {
        tag: ACCOUNT_TAG_POSITION,
        version: 1,
        owner: owner.to_bytes(),
        market: market.to_bytes(),
        collateral_amount: spec.collateral,
        debt_amount: spec.debt,
        collateral_price: PRICE,
        liquidation_threshold_bps: LIQUIDATION_THRESHOLD_BPS,
        max_ltv_bps: MAX_LTV_BPS,
        health_factor: reference::health_factor(
            spec.collateral,
            spec.debt,
            PRICE,
            LIQUIDATION_THRESHOLD_BPS,
        ),
        last_update_slot: 0,
    };

    let program_owner = program_id.to_string();
    let mut accounts = vec![
        NamedAccount {
            label: "market".into(),
            address: market.to_string(),
            account: AccountSnapshot {
                lamports: rent.minimum_balance(MARKET_LEN),
                owner: program_owner.clone(),
                data: borsh::to_vec(&market_state).unwrap(),
                executable: false,
                rent_epoch: 0,
            },
        },
        NamedAccount {
            label: "position".into(),
            address: position.to_string(),
            account: AccountSnapshot {
                lamports: rent.minimum_balance(POSITION_LEN),
                owner: program_owner.clone(),
                data: borsh::to_vec(&position_state).unwrap(),
                executable: false,
                rent_epoch: 0,
            },
        },
        NamedAccount {
            label: "vault".into(),
            address: vault.to_string(),
            // The vault custodies every lamport of collateral, plus its own rent.
            account: AccountSnapshot {
                lamports: rent.minimum_balance(0) + spec.collateral,
                owner: program_owner,
                data: Vec::new(),
                executable: false,
                rent_epoch: 0,
            },
        },
        NamedAccount {
            label: "owner".into(),
            address: owner.to_string(),
            account: system_account(OWNER_FUNDING),
        },
        NamedAccount {
            label: "payer".into(),
            address: payer.to_string(),
            account: system_account(PAYER_FUNDING),
        },
    ];

    let mut keypairs = vec![payer_kp, owner_kp, authority_kp];
    let mut watch = vec![
        "position".to_string(),
        "vault".to_string(),
        "owner".to_string(),
    ];

    let (metas, signers) = match spec.action {
        Action::Refresh => (
            vec![
                AccountMetaSpec {
                    address: position.to_string(),
                    is_signer: false,
                    is_writable: true,
                },
                AccountMetaSpec {
                    address: market.to_string(),
                    is_signer: false,
                    is_writable: false,
                },
            ],
            vec![],
        ),
        Action::Withdraw(_) => (
            vec![
                AccountMetaSpec {
                    address: position.to_string(),
                    is_signer: false,
                    is_writable: true,
                },
                AccountMetaSpec {
                    address: market.to_string(),
                    is_signer: false,
                    is_writable: false,
                },
                AccountMetaSpec {
                    address: vault.to_string(),
                    is_signer: false,
                    is_writable: true,
                },
                AccountMetaSpec {
                    address: owner.to_string(),
                    is_signer: true,
                    is_writable: true,
                },
            ],
            vec!["owner".to_string()],
        ),
        Action::Deposit(_) => (
            vec![
                AccountMetaSpec {
                    address: position.to_string(),
                    is_signer: false,
                    is_writable: true,
                },
                AccountMetaSpec {
                    address: market.to_string(),
                    is_signer: false,
                    is_writable: false,
                },
                AccountMetaSpec {
                    address: vault.to_string(),
                    is_signer: false,
                    is_writable: true,
                },
                AccountMetaSpec {
                    address: owner.to_string(),
                    is_signer: true,
                    is_writable: true,
                },
                AccountMetaSpec {
                    address: SYSTEM_PROGRAM.to_string(),
                    is_signer: false,
                    is_writable: false,
                },
            ],
            vec!["owner".to_string()],
        ),
        Action::Borrow(_) | Action::Repay(_) => (
            vec![
                AccountMetaSpec {
                    address: position.to_string(),
                    is_signer: false,
                    is_writable: true,
                },
                AccountMetaSpec {
                    address: market.to_string(),
                    is_signer: false,
                    is_writable: false,
                },
                AccountMetaSpec {
                    address: owner.to_string(),
                    is_signer: true,
                    is_writable: false,
                },
            ],
            vec!["owner".to_string()],
        ),
        Action::Liquidate(_) => {
            accounts.push(NamedAccount {
                label: "liquidator".into(),
                address: liquidator.to_string(),
                account: system_account(LIQUIDATOR_FUNDING),
            });
            keypairs.push(liquidator_kp);
            watch.push("liquidator".to_string());
            (
                vec![
                    AccountMetaSpec {
                        address: position.to_string(),
                        is_signer: false,
                        is_writable: true,
                    },
                    AccountMetaSpec {
                        address: market.to_string(),
                        is_signer: false,
                        is_writable: false,
                    },
                    AccountMetaSpec {
                        address: vault.to_string(),
                        is_signer: false,
                        is_writable: true,
                    },
                    AccountMetaSpec {
                        address: liquidator.to_string(),
                        is_signer: true,
                        is_writable: true,
                    },
                ],
                vec!["liquidator".to_string()],
            )
        }
    };

    Fixture {
        id,
        category: spec.category,
        scenario: spec.action.describe(),
        notes: spec.notes,
        keypairs,
        accounts,
        fee_payer: "payer".to_string(),
        signers,
        instruction: InstructionSpec {
            program: program_id.to_string(),
            accounts: metas,
            data: spec.action.encode(),
        },
        watch,
    }
}

/// Debt that puts a position at exactly `target_health` under the reference math.
fn debt_for_health(collateral: u64, target_health: u64) -> u64 {
    let adjusted = reference::adjusted_collateral(collateral, PRICE, LIQUIDATION_THRESHOLD_BPS);
    u64::try_from(adjusted * 1_000_000 / target_health as u128).unwrap_or(u64::MAX)
}

fn specs() -> Vec<Spec> {
    let mut out = Vec::new();

    // ---- whole-SOL families: V1 and V2 agree exactly ----------------------
    // Collateral is always an exact multiple of 1 SOL, before and after the
    // action, so the V2 truncation has nothing to truncate.

    for n in 1..=40u64 {
        let collateral = (10 + n * 5) * SOL;
        let sol = collateral / SOL;
        let debt = sol * 20 * USD; // ~20% LTV
        let action = match n % 5 {
            0 => Action::Refresh,
            1 => Action::Withdraw((1 + n % 3) * SOL),
            2 => Action::Borrow(50 * USD),
            3 => Action::Repay(10 * USD),
            _ => Action::Deposit(2 * SOL),
        };
        out.push(Spec {
            id: format!("healthy-{n:03}"),
            category: Category::Healthy,
            notes: "Low leverage, whole-SOL collateral. Expected identical under both builds."
                .into(),
            collateral,
            debt,
            action,
        });
    }

    for n in 1..=25u64 {
        let collateral = (20 + n * 4) * SOL;
        let sol = collateral / SOL;
        let debt = sol * 45 * USD; // ~45% LTV
        let action = match n % 3 {
            0 => Action::Refresh,
            1 => Action::Withdraw(2 * SOL),
            _ => Action::Borrow(100 * USD),
        };
        out.push(Spec {
            id: format!("moderate-{n:03}"),
            category: Category::Moderate,
            notes: "Mid leverage, whole-SOL collateral. Expected identical under both builds."
                .into(),
            collateral,
            debt,
            action,
        });
    }

    for n in 1..=12u64 {
        let collateral = n * SOL;
        let debt = n * 30 * USD;
        out.push(Spec {
            id: format!("small-{n:03}"),
            category: Category::Small,
            notes: "Dust-scale position, whole-SOL collateral.".into(),
            collateral,
            debt,
            action: if n % 2 == 0 {
                Action::Refresh
            } else {
                Action::Repay(5 * USD)
            },
        });
    }

    for n in 1..=12u64 {
        let collateral = (1_000 + n * 500) * SOL;
        let sol = collateral / SOL;
        let debt = sol * 40 * USD;
        out.push(Spec {
            id: format!("large-{n:03}"),
            category: Category::Large,
            notes: "Large position; also exercises u128 headroom in the valuation path.".into(),
            collateral,
            debt,
            action: if n % 2 == 0 {
                Action::Refresh
            } else {
                Action::Withdraw(10 * SOL)
            },
        });
    }

    // ---- fractional families: V2 mis-values, outcome usually unchanged -----

    for n in 1..=15u64 {
        let collateral = (50 + n) * SOL + (n % 9 + 1) * 100_000_000;
        let sol = collateral / SOL;
        let debt = sol * 50 * USD; // health ~1.6
        out.push(Spec {
            id: format!("fractional-{n:03}"),
            category: Category::Fractional,
            notes: "Fractional-SOL collateral with comfortable headroom: the cached health \
                    factor shifts under V2 but no threshold is crossed."
                .into(),
            collateral,
            debt,
            action: Action::Refresh,
        });
    }

    for n in 1..=8u64 {
        let collateral = (80 + n) * SOL + 500_000_000;
        let target_health = 1_020_000 + n * 20_000; // 1.02 .. 1.18
        out.push(Spec {
            id: format!("near-liquidation-{n:03}"),
            category: Category::NearLiquidation,
            notes: "Fractional collateral close to, but not at, the liquidation threshold.".into(),
            collateral,
            debt: debt_for_health(collateral, target_health),
            action: Action::Refresh,
        });
    }

    // ---- boundary families: arithmetically tuned to straddle a threshold ---
    //
    // Collateral is fixed at 99.5 SOL, so at $100/SOL and an 80% threshold:
    //   risk-adjusted collateral, V1 = 99.5 * 100 * 0.80 = $7,960.00
    //   risk-adjusted collateral, V2 = 99   * 100 * 0.80 = $7,920.00
    // A position is healthy under V1 but liquidatable under V2 exactly when its
    // debt falls in ($7,920.00, $7,960.00].

    for n in 1..=20u64 {
        let debt = 7_760_000_000 + n * 10_000_000; // $7,770 .. $7,960
        out.push(Spec {
            id: format!("boundary-position-{n:03}"),
            category: Category::Boundary,
            notes: "Health factor recomputed against an unchanged price. Fixtures 017-020 sit \
                    inside the ($7,920, $7,960] window where V1 reports healthy and V2 reports \
                    liquidatable."
                .into(),
            collateral: 99_500_000_000,
            debt,
            action: Action::Refresh,
        });
    }

    // Withdrawal gate. Starting collateral is a whole 100 SOL (so the initial
    // state is identical under both builds); the withdrawal itself creates the
    // fractional remainder that V2 mis-values.
    //   post-withdrawal borrow capacity, V1 = 99.5 * 100 * 0.75 = $7,462.50
    //   post-withdrawal borrow capacity, V2 = 99   * 100 * 0.75 = $7,425.00
    for n in 1..=6u64 {
        let debt = 7_400_000_000 + n * 10_000_000; // $7,410 .. $7,460
        out.push(Spec {
            id: format!("withdraw-boundary-{n:03}"),
            category: Category::WithdrawBoundary,
            notes: "Withdrawal of 0.5 SOL. Fixtures 003-006 carry debt above V2's reduced \
                    borrow capacity, so the same withdrawal reverts under V2."
                .into(),
            collateral: 100 * SOL,
            debt,
            action: Action::Withdraw(500_000_000),
        });
    }

    // Liquidation eligibility: V1 rejects the liquidation as healthy, V2 permits
    // it and the owner's collateral is seized.
    for n in 1..=3u64 {
        let debt = 7_930_000_000 + n * 10_000_000; // $7,940 .. $7,960
        out.push(Spec {
            id: format!("liquidation-boundary-{n:03}"),
            category: Category::LiquidationBoundary,
            notes: "Third-party liquidation attempt. V1 rejects it (PositionHealthy); under V2 \
                    the same transaction succeeds and seizes collateral."
                .into(),
            collateral: 99_500_000_000,
            debt,
            action: Action::Liquidate(1_000_000_000),
        });
    }

    out
}

/// Generate the full corpus. Pure and deterministic.
pub fn generate(program_id: &Address) -> Vec<Fixture> {
    specs()
        .into_iter()
        .map(|spec| build(spec, program_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_is_large_enough_and_ids_are_unique() {
        let fixtures = generate(&crate::fixture_program_id());
        assert!(
            fixtures.len() >= 100,
            "corpus has {} fixtures",
            fixtures.len()
        );
        let mut ids: Vec<&str> = fixtures.iter().map(|f| f.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "fixture ids must be unique");
    }

    #[test]
    fn generation_is_deterministic() {
        let program_id = crate::fixture_program_id();
        assert_eq!(generate(&program_id), generate(&program_id));
    }

    #[test]
    fn boundary_window_is_where_we_think_it_is() {
        // V1-adjusted collateral for 99.5 SOL is $7,960; V2's is $7,920.
        let collateral = 99_500_000_000;
        assert_eq!(
            reference::adjusted_collateral(collateral, PRICE, LIQUIDATION_THRESHOLD_BPS),
            7_960_000_000
        );
        // boundary-position-017 carries $7,930 of debt: inside the window.
        let health =
            reference::health_factor(collateral, 7_930_000_000, PRICE, LIQUIDATION_THRESHOLD_BPS);
        assert!(!reference::is_liquidatable(health), "healthy under V1");
        assert_eq!(health, 1_003_783);
    }
}
