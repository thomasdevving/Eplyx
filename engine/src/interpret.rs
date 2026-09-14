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
    reference, Market, Position, ACCOUNT_TAG_MARKET, ACCOUNT_TAG_POSITION, HEALTH_INFINITE,
    HEALTH_SCALE, MARKET_LEN, POSITION_LEN,
};

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

/// A human- and machine-readable economic summary of a position.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PositionEconomics {
    pub collateral_lamports: u64,
    pub collateral_sol: String,
    pub collateral_value_usd: String,
    pub debt_usd: String,
    pub health_factor: u64,
    pub health_display: String,
    pub liquidatable: bool,
}

pub fn economics(position: &Position) -> PositionEconomics {
    PositionEconomics {
        collateral_lamports: position.collateral_amount,
        collateral_sol: format_sol(position.collateral_amount),
        collateral_value_usd: format_usd_u128(reference::collateral_value(
            position.collateral_amount,
            position.collateral_price,
        )),
        debt_usd: format_usd(position.debt_amount),
        health_factor: position.health_factor,
        health_display: format_health(position.health_factor),
        liquidatable: is_liquidatable(position.health_factor),
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
    fn opaque_data_decodes_to_opaque() {
        assert_eq!(decode(&[]), Decoded::Opaque);
        assert_eq!(decode(&[9, 9, 9]), Decoded::Opaque);
    }
}
