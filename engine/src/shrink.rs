//! Deterministic counterexample minimization.
//!
//! Given a fixture that reproduces a regression, this searches for a smaller
//! position that reproduces *the same* regression, so a developer reads
//! "0.5 SOL against $1 of debt" rather than "99.5 SOL against $7,930".
//!
//! # What it is
//!
//! A bounded delta-debugging search. Each candidate is executed against both
//! program builds exactly as any other fixture would be, and is accepted only if
//! its [`RegressionSignature`] still matches the original. The move set, the
//! order moves are tried in, and the probe budget are all fixed, so the same
//! input always yields the same minimized case.
//!
//! # What it is not
//!
//! It is not formal verification and it is not a proof of minimality. It finds a
//! small *witness*, not the smallest one: the search is greedy, it stops at a
//! configured granularity, and it only varies two numeric fields. A case it
//! cannot shrink further is a local stopping point, not a proven boundary.

use anyhow::Result;
use borsh::BorshDeserialize;
use fixture_lending_interface::{reference, Position, POSITION_LEN};
use solana_address::Address;
use solana_rent::Rent;

use crate::cluster::{signature, MinimizedCase, RegressionSignature};
use crate::diff;
use crate::executor::{execute, ProgramVersion};
use crate::impact;
use crate::interpret::{format_health, is_liquidatable};
use crate::money::Usd;
use crate::types::Fixture;

/// Search limits. Deliberately small: minimization runs on every comparison, and
/// a counterexample that takes a minute to find would simply be turned off.
#[derive(Clone, Copy, Debug)]
pub struct ShrinkConfig {
    /// Maximum candidate executions per cluster. Each probe runs both builds.
    pub max_probes: usize,
    /// Smallest collateral step, in lamports. 0.001 SOL: below this the case
    /// stops being something a reader can picture.
    pub collateral_granularity: u64,
    /// Smallest debt step, in micro-USD. $0.01.
    pub debt_granularity: u64,
}

impl Default for ShrinkConfig {
    fn default() -> Self {
        Self {
            max_probes: 96,
            collateral_granularity: 1_000_000,
            debt_granularity: 10_000,
        }
    }
}

fn quantize(value: u64, granularity: u64) -> u64 {
    if granularity == 0 {
        return value;
    }
    (value / granularity) * granularity
}

fn read_position(fixture: &Fixture) -> Option<Position> {
    let account = fixture.account("position")?;
    Position::try_from_slice(&account.account.data[..POSITION_LEN.min(account.account.data.len())])
        .ok()
}

/// Build a variant of `base` with different collateral and debt.
///
/// The position's cached health factor is recomputed with the reference
/// arithmetic and the vault balance is adjusted to match, so the candidate is a
/// *self-consistent* state rather than an arbitrary edit. Nothing else changes:
/// the instruction, signers, program and every other account are untouched.
pub fn variant(base: &Fixture, collateral: u64, debt: u64) -> Option<Fixture> {
    let mut position = read_position(base)?;
    position.collateral_amount = collateral;
    position.debt_amount = debt;
    position.health_factor = reference::health_factor(
        collateral,
        debt,
        position.collateral_price,
        position.liquidation_threshold_bps,
    );
    let encoded = borsh::to_vec(&position).ok()?;

    let mut fixture = base.clone();
    let vault_floor = Rent::default().minimum_balance(0);
    for account in &mut fixture.accounts {
        match account.label.as_str() {
            "position" => {
                account.account.data[..encoded.len()].copy_from_slice(&encoded);
            }
            "vault" => {
                // The vault custodies exactly the collateral plus its own rent.
                account.account.lamports = vault_floor.checked_add(collateral)?;
            }
            _ => {}
        }
    }
    Some(fixture)
}

struct Search<'a> {
    program_id: &'a Address,
    v1: &'a ProgramVersion,
    v2: &'a ProgramVersion,
    target: RegressionSignature,
    config: ShrinkConfig,
    probes: usize,
    reductions: usize,
}

impl Search<'_> {
    /// Execute a candidate and report whether it reproduces the same regression.
    fn reproduces(&mut self, candidate: &Fixture) -> Result<bool> {
        self.probes += 1;
        let v1 = execute(candidate, self.program_id, self.v1)?;
        let v2 = execute(candidate, self.program_id, self.v2)?;
        let state_diff = diff::compare(candidate, v1, v2);
        let Some(economics) = impact::evaluate(candidate, &state_diff) else {
            return Ok(false);
        };
        Ok(signature(candidate, &state_diff, &economics) == self.target)
    }

    fn budget_left(&self) -> bool {
        self.probes < self.config.max_probes
    }

    /// Try one candidate; adopt it if it preserves the regression.
    fn try_candidate(
        &mut self,
        base: &Fixture,
        state: &mut (u64, u64),
        collateral: u64,
        debt: u64,
    ) -> Result<bool> {
        if !self.budget_left() {
            return Ok(false);
        }
        let collateral = quantize(collateral, self.config.collateral_granularity);
        let debt = quantize(debt, self.config.debt_granularity);
        if collateral < self.config.collateral_granularity || debt < self.config.debt_granularity {
            return Ok(false);
        }
        if (collateral, debt) == *state {
            return Ok(false);
        }
        let Some(candidate) = variant(base, collateral, debt) else {
            return Ok(false);
        };
        if self.reproduces(&candidate)? {
            *state = (collateral, debt);
            self.reductions += 1;
            return Ok(true);
        }
        Ok(false)
    }
}

/// Shrink a fixture while preserving its regression class.
///
/// Returns `None` when the fixture carries no position, or when no configured
/// simplification preserves the regression.
pub fn minimize(
    base: &Fixture,
    target: RegressionSignature,
    program_id: &Address,
    v1: &ProgramVersion,
    v2: &ProgramVersion,
    config: ShrinkConfig,
) -> Result<Option<MinimizedCase>> {
    anyhow::ensure!(
        config.collateral_granularity > 0 && config.debt_granularity > 0,
        "minimization granularity must be positive"
    );
    let Some(original) = read_position(base) else {
        return Ok(None);
    };

    let mut search = Search {
        program_id,
        v1,
        v2,
        target,
        config,
        probes: 0,
        reductions: 0,
    };
    let mut state = (original.collateral_amount, original.debt_amount);

    // Collateral and debt are coupled: near a threshold neither can move on its
    // own without leaving the regression class. Scaling both together breaks
    // that deadlock, and the single-field descents then refine the result.
    // Ratios are tried largest-reduction first, with 3/4 and 7/8 catching the
    // cases where halving overshoots.
    const RATIOS: [(u128, u128); 3] = [(1, 2), (3, 4), (7, 8)];

    loop {
        let before = state;

        for (numerator, denominator) in RATIOS {
            // Repeat a ratio while it keeps working before moving to a gentler one.
            loop {
                let collateral = (state.0 as u128 * numerator / denominator) as u64;
                let debt = (state.1 as u128 * numerator / denominator) as u64;
                if !search.try_candidate(base, &mut state, collateral, debt)? {
                    break;
                }
            }
        }

        // Single-field descent with a halving step, which needs no monotonicity
        // assumption: a rejected step is simply halved and retried.
        let mut step = state.0 / 2;
        while step >= search.config.collateral_granularity && search.budget_left() {
            let (collateral, debt) = (state.0.saturating_sub(step), state.1);
            if !search.try_candidate(base, &mut state, collateral, debt)? {
                step /= 2;
            }
        }

        let mut step = state.1 / 2;
        while step >= search.config.debt_granularity && search.budget_left() {
            let (collateral, debt) = (state.0, state.1.saturating_sub(step));
            if !search.try_candidate(base, &mut state, collateral, debt)? {
                step /= 2;
            }
        }

        if state == before || !search.budget_left() {
            break;
        }
    }

    if search.reductions == 0 {
        return Ok(None);
    }

    // Re-execute the winner so the reported observations come from a real run
    // rather than from whatever the last probe happened to be.
    let minimized = variant(base, state.0, state.1).expect("variant of a valid fixture");
    let result_v1 = execute(&minimized, program_id, v1)?;
    let result_v2 = execute(&minimized, program_id, v2)?;

    let health = |result: &crate::executor::ExecutionResult| -> Option<u64> {
        result
            .accounts
            .get("position")
            .and_then(|snapshot| crate::interpret::position_economics(&snapshot.data))
            .map(|economics| economics.health_factor)
    };
    let outcome = |result: &crate::executor::ExecutionResult| -> String {
        if result.success {
            "success".to_string()
        } else {
            format!(
                "reverted: {}",
                result.error.as_deref().unwrap_or("unknown error")
            )
        }
    };

    let v1_health = health(&result_v1);
    let v2_health = health(&result_v2);

    Ok(Some(MinimizedCase {
        derived_from: base.id.clone(),
        collateral_lamports: state.0,
        collateral_display: format!(
            "{}.{:09} SOL",
            state.0 / 1_000_000_000,
            state.0 % 1_000_000_000
        ),
        debt_micro_usd: state.1,
        debt_display: Usd::from_micro(state.1 as u128).format_dollars(),
        collateral_value_usd: Usd::from_micro(reference::collateral_value(
            state.0,
            original.collateral_price,
        )),
        debt_value_usd: Usd::from_micro(reference::debt_value(state.1)),
        v1_health: v1_health.map(format_health).unwrap_or_else(|| "n/a".into()),
        v2_health: v2_health.map(format_health).unwrap_or_else(|| "n/a".into()),
        v1_liquidatable: v1_health.map(is_liquidatable).unwrap_or(false),
        v2_liquidatable: v2_health.map(is_liquidatable).unwrap_or(false),
        v1_outcome: outcome(&result_v1),
        v2_outcome: outcome(&result_v2),
        probes: search.probes,
        reductions: search.reductions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantization_rounds_down_to_readable_steps() {
        assert_eq!(quantize(99_500_123_456, 1_000_000), 99_500_000_000);
        assert_eq!(quantize(7_930_004_321, 10_000), 7_930_000_000);
        assert_eq!(quantize(5, 0), 5);
        assert_eq!(quantize(999, 1_000), 0);
    }

    #[test]
    fn a_variant_stays_self_consistent() {
        let program_id = crate::fixture_program_id();
        let fixtures = crate::corpus::generate(&program_id);
        let base = fixtures
            .iter()
            .find(|f| f.id == "boundary-position-017")
            .expect("flagship fixture");

        let candidate = variant(base, 1_500_000_000, 100_000_000).expect("variant");
        let position = read_position(&candidate).expect("position");

        assert_eq!(position.collateral_amount, 1_500_000_000);
        assert_eq!(position.debt_amount, 100_000_000);
        // Cached health recomputed with the reference arithmetic.
        assert_eq!(
            position.health_factor,
            reference::health_factor(1_500_000_000, 100_000_000, position.collateral_price, 8_000)
        );
        // Vault balance tracks the collateral it custodies.
        let vault = candidate.account("vault").expect("vault");
        assert_eq!(
            vault.account.lamports,
            Rent::default().minimum_balance(0) + 1_500_000_000
        );
        // Everything else is untouched.
        assert_eq!(candidate.instruction, base.instruction);
        assert_eq!(candidate.keypairs, base.keypairs);
        assert_eq!(candidate.id, base.id);
    }
}
