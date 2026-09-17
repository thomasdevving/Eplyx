//! The economic interpreter.
//!
//! The diff engine can tell you that byte 82 of an account changed. That is
//! true and nearly useless. This module turns raw account bytes into named
//! fields and then into economic statements - "this position is now
//! liquidatable" - which is the output the product is actually about.
//!
//! It is the second of the two protocol-aware modules (the other is `corpus`).
//! Everything it knows comes from the shared wire-format crate, so a future
//! protocol adapter would implement this same shape against an IDL.

use borsh::BorshDeserialize;
use fixture_lending_interface::{
    reference, Market, Position, ACCOUNT_TAG_MARKET, ACCOUNT_TAG_POSITION, COLLATERAL_DECIMALS,
    DEBT_DECIMALS, DEBT_PRICE_MICRO_USD, HEALTH_INFINITE, HEALTH_SCALE, MARKET_LEN, POSITION_LEN,
};

use crate::money::{SignedUsd, Usd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoded {
    Market(Box<Market>),
    Position(Box<Position>),
    /// Not a recognised account type; the diff engine falls back to raw bytes.
    Opaque,
}

pub fn decode(data: &[u8]) -> Decoded {
    match data.first() {
        Some(&ACCOUNT_TAG_POSITION) if data.len() >= POSITION_LEN => {
            Position::try_from_slice(&data[..POSITION_LEN])
                .map(|p| Decoded::Position(Box::new(p)))
                .unwrap_or(Decoded::Opaque)
        }
        Some(&ACCOUNT_TAG_MARKET) if data.len() >= MARKET_LEN => {
            Market::try_from_slice(&data[..MARKET_LEN])
                .map(|m| Decoded::Market(Box::new(m)))
                .unwrap_or(Decoded::Opaque)
        }
        _ => Decoded::Opaque,
    }
}

/// Render a `HEALTH_SCALE` fixed-point health factor.
pub fn format_health(health: u64) -> String {
    if health == HEALTH_INFINITE {
        return "inf (no debt)".to_string();
    }
    format!("{}.{:06}", health / HEALTH_SCALE, health % HEALTH_SCALE)
}

pub fn format_usd(micro: u64) -> String {
    format!("{}.{:06}", micro / 1_000_000, micro % 1_000_000)
}

pub fn format_usd_u128(micro: u128) -> String {
    format!("{}.{:06}", micro / 1_000_000, micro % 1_000_000)
}

pub fn format_sol(lamports: u64) -> String {
    format!(
        "{}.{:09}",
        lamports / 1_000_000_000,
        lamports % 1_000_000_000
    )
}

pub fn is_liquidatable(health: u64) -> bool {
    reference::is_liquidatable(health)
}

/// A field of a decoded account, as the diff engine sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Unsigned(u128),
    Text(String),
    /// Health factor, carried distinctly so the reporter can render it as a
    /// decimal and reason about the 1.0 threshold.
    Health(u64),
}

impl FieldValue {
    pub fn render(&self) -> String {
        match self {
            FieldValue::Unsigned(v) => v.to_string(),
            FieldValue::Text(v) => v.clone(),
            FieldValue::Health(v) => format_health(*v),
        }
    }

    pub fn numeric(&self) -> Option<i128> {
        match self {
            FieldValue::Unsigned(v) => i128::try_from(*v).ok(),
            FieldValue::Health(v) => Some(*v as i128),
            FieldValue::Text(_) => None,
        }
    }
}

fn address(bytes: &[u8; 32]) -> String {
    solana_address::Address::new_from_array(*bytes).to_string()
}

/// Named fields of a decoded account, in a stable order.
pub fn fields(decoded: &Decoded) -> Vec<(&'static str, FieldValue)> {
    match decoded {
        Decoded::Position(p) => vec![
            ("owner", FieldValue::Text(address(&p.owner))),
            ("market", FieldValue::Text(address(&p.market))),
            (
                "collateral_amount",
                FieldValue::Unsigned(p.collateral_amount as u128),
            ),
            ("debt_amount", FieldValue::Unsigned(p.debt_amount as u128)),
            (
                "collateral_price",
                FieldValue::Unsigned(p.collateral_price as u128),
            ),
            (
                "liquidation_threshold_bps",
                FieldValue::Unsigned(p.liquidation_threshold_bps as u128),
            ),
            ("max_ltv_bps", FieldValue::Unsigned(p.max_ltv_bps as u128)),
            ("health_factor", FieldValue::Health(p.health_factor)),
            (
                "last_update_slot",
                FieldValue::Unsigned(p.last_update_slot as u128),
            ),
        ],
        Decoded::Market(m) => vec![
            ("authority", FieldValue::Text(address(&m.authority))),
            ("vault", FieldValue::Text(address(&m.vault))),
            (
                "collateral_price",
                FieldValue::Unsigned(m.collateral_price as u128),
            ),
            (
                "liquidation_threshold_bps",
                FieldValue::Unsigned(m.liquidation_threshold_bps as u128),
            ),
            ("max_ltv_bps", FieldValue::Unsigned(m.max_ltv_bps as u128)),
            (
                "position_count",
                FieldValue::Unsigned(m.position_count as u128),
            ),
        ],
        Decoded::Opaque => Vec::new(),
    }
}

/// The economic view of a single position.
///
/// The raw inputs are read straight out of the position account rather than
/// duplicated into the fixture: `collateral_amount`, `debt_amount` and
/// `collateral_price` are already part of the on-chain state, and the decimal
/// exponents and the debt peg are protocol-level constants in the shared
/// interface crate. Only the *derived* values are new.
///
/// Every value here is computed with integer arithmetic. See [`crate::money`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PositionEconomics {
    // --- inputs, as they appear in account state ---
    pub collateral_amount: u64,
    pub collateral_decimals: u8,
    pub collateral_price_usd: Usd,
    pub debt_amount: u64,
    pub debt_decimals: u8,
    pub debt_price_usd: Usd,

    // --- normalised values ---
    pub collateral_value_usd: Usd,
    pub debt_value_usd: Usd,
    /// Collateral minus debt. Negative when the position is underwater.
    pub net_value_usd: SignedUsd,

    // --- risk ---
    pub health_factor: u64,
    pub health_display: String,
    pub liquidatable: bool,

    // --- presentation ---
    pub collateral_display: String,
}

pub fn economics(position: &Position) -> PositionEconomics {
    let collateral_value = Usd::from_micro(reference::value_micro_usd(
        position.collateral_amount,
        COLLATERAL_DECIMALS,
        position.collateral_price,
    ));
    let debt_value = Usd::from_micro(reference::value_micro_usd(
        position.debt_amount,
        DEBT_DECIMALS,
        DEBT_PRICE_MICRO_USD,
    ));

    PositionEconomics {
        collateral_amount: position.collateral_amount,
        collateral_decimals: COLLATERAL_DECIMALS,
        collateral_price_usd: Usd::from_micro(position.collateral_price as u128),
        debt_amount: position.debt_amount,
        debt_decimals: DEBT_DECIMALS,
        debt_price_usd: Usd::from_micro(DEBT_PRICE_MICRO_USD as u128),

        collateral_value_usd: collateral_value,
        debt_value_usd: debt_value,
        net_value_usd: collateral_value.signed_sub(debt_value),

        health_factor: position.health_factor,
        health_display: format_health(position.health_factor),
        liquidatable: is_liquidatable(position.health_factor),

        collateral_display: format!("{} SOL", format_sol(position.collateral_amount)),
    }
}

/// Name of the instruction an encoded payload invokes.
///
/// Borsh encodes an enum as a leading u8 discriminant, so the first byte of the
/// instruction data identifies the action without decoding the whole payload.
pub fn instruction_name(data: &[u8]) -> &'static str {
    match data.first() {
        Some(0) => "initialize_market",
        Some(1) => "create_position",
        Some(2) => "deposit_collateral",
        Some(3) => "borrow",
        Some(4) => "repay",
        Some(5) => "withdraw_collateral",
        Some(6) => "liquidate",
        Some(7) => "refresh_position",
        Some(8) => "set_price",
        _ => "unknown",
    }
}

/// Decode a position out of raw account data, if it is one.
pub fn position_economics(data: &[u8]) -> Option<PositionEconomics> {
    match decode(data) {
        Decoded::Position(position) => Some(economics(&position)),
        _ => None,
    }
}

/// Plain-language consequence of a field moving, where one exists.
pub fn explain(field: &str, before: &FieldValue, after: &FieldValue) -> Option<String> {
    match (field, before, after) {
        ("health_factor", FieldValue::Health(a), FieldValue::Health(b)) => {
            let was = is_liquidatable(*a);
            let now = is_liquidatable(*b);
            if was != now {
                Some(if now {
                    "position crosses below the liquidation threshold".to_string()
                } else {
                    "position rises above the liquidation threshold".to_string()
                })
            } else if b < a {
                Some("position is closer to liquidation".to_string())
            } else {
                Some("position is further from liquidation".to_string())
            }
        }
        ("collateral_amount", FieldValue::Unsigned(a), FieldValue::Unsigned(b)) => {
            let delta = *b as i128 - *a as i128;
            Some(format!(
                "collateral differs by {} lamports for the same instruction",
                delta
            ))
        }
        ("debt_amount", FieldValue::Unsigned(a), FieldValue::Unsigned(b)) => {
            let delta = *b as i128 - *a as i128;
            Some(format!("recorded debt differs by {delta} micro-USD"))
        }
        _ => None,
    }
}

/// How many leading bytes a decoded layout actually describes.
///
/// Used by the generic diff to prove that everything past the struct is still
/// compared: a decoder that covers a prefix must not be mistaken for one that
/// covers the account.
pub fn decoded_len(decoded: &Decoded) -> usize {
    match decoded {
        Decoded::Market(_) => MARKET_LEN,
        Decoded::Position(_) => POSITION_LEN,
        Decoded::Opaque => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_formats_as_decimal() {
        assert_eq!(format_health(1_003_783), "1.003783");
        assert_eq!(format_health(998_738), "0.998738");
        assert_eq!(format_health(HEALTH_INFINITE), "inf (no debt)");
    }

    #[test]
    fn liquidation_threshold_is_exactly_one() {
        assert!(!is_liquidatable(HEALTH_SCALE));
        assert!(is_liquidatable(HEALTH_SCALE - 1));
    }

    #[test]
    fn valuation_uses_account_state_and_protocol_constants() {
        // 99.5 SOL at $100.00 against $7,930.00 of debt.
        let position = Position {
            tag: ACCOUNT_TAG_POSITION,
            version: 1,
            owner: [1u8; 32],
            market: [2u8; 32],
            collateral_amount: 99_500_000_000,
            debt_amount: 7_930_000_000,
            collateral_price: 100_000_000,
            liquidation_threshold_bps: 8_000,
            max_ltv_bps: 7_500,
            health_factor: 1_003_783,
            last_update_slot: 0,
        };
        let economics = economics(&position);

        assert_eq!(
            economics.collateral_value_usd,
            Usd::from_micro(9_950_000_000)
        );
        assert_eq!(economics.collateral_value_usd.format_dollars(), "$9,950.00");
        assert_eq!(economics.debt_value_usd, Usd::from_micro(7_930_000_000));
        assert_eq!(economics.debt_value_usd.format_dollars(), "$7,930.00");
        assert_eq!(economics.net_value_usd.format_dollars(), "$2,020.00");
        assert_eq!(economics.collateral_decimals, COLLATERAL_DECIMALS);
        assert_eq!(economics.debt_decimals, DEBT_DECIMALS);
        assert!(!economics.liquidatable);
    }

    #[test]
    fn an_underwater_position_has_negative_net_value() {
        let position = Position {
            tag: ACCOUNT_TAG_POSITION,
            version: 1,
            owner: [1u8; 32],
            market: [2u8; 32],
            collateral_amount: 1_000_000_000, // 1 SOL = $100
            debt_amount: 250_000_000,         // $250
            collateral_price: 100_000_000,
            liquidation_threshold_bps: 8_000,
            max_ltv_bps: 7_500,
            health_factor: 320_000,
            last_update_slot: 0,
        };
        let economics = economics(&position);
        assert!(economics.net_value_usd.is_negative());
        assert_eq!(economics.net_value_usd.to_plain_string(), "-150.000000");
        assert!(economics.liquidatable);
    }

    #[test]
    fn instruction_names_match_the_wire_encoding() {
        use fixture_lending_interface::LendingInstruction;
        let cases = [
            (LendingInstruction::RefreshPosition, "refresh_position"),
            (
                LendingInstruction::WithdrawCollateral { amount: 1 },
                "withdraw_collateral",
            ),
            (
                LendingInstruction::Liquidate { repay_amount: 1 },
                "liquidate",
            ),
            (LendingInstruction::Borrow { amount: 1 }, "borrow"),
        ];
        for (instruction, expected) in cases {
            let encoded = borsh::to_vec(&instruction).unwrap();
            assert_eq!(instruction_name(&encoded), expected);
        }
        assert_eq!(instruction_name(&[]), "unknown");
    }

    #[test]
    fn opaque_data_decodes_to_opaque() {
        assert_eq!(decode(&[]), Decoded::Opaque);
        assert_eq!(decode(&[9, 9, 9]), Decoded::Opaque);
    }
}
