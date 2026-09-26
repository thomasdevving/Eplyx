//! Sequential population rehearsal and source → destination reconciliation.
//!
//! One VM holds the whole bank at the rehearsal Clock and executes every unit the
//! planner attempts (migratable units and predicted reserve shortfalls), in plan
//! order, as separate transactions — the way a rollout would, with the reserve
//! shared across holders. Totals are measured from bank state before and after the
//! whole rehearsal and must satisfy explicit equations. "Fully reconciled" is only
//! claimed when every equation holds *and* the population is completely enumerated,
//! every positive holder was attempted and none was left out by the budget.
use super::{
    adapter,
    execute::{self, Bank, Outcome, Session, UnitExecution},
    planner::MigrationPlan,
    spec::{DestinationFunding, SourceDisposition, TokenMigrationV1},
    world::World,
};
use crate::executor::LoadedProgram;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const REHEARSAL_VERSION: &str = "eplyx-migration-population-rehearsal/v1";
pub const DEFAULT_MAX_UNITS: usize = 5_000;
pub const MAX_UNITS: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Equation {
    pub name: String,
    pub left: String,
    pub right: String,
    pub holds: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationStatus {
    /// Every equation holds, the population is complete and every positive holder
    /// was attempted within the budget.
    FullyReconciled,
    /// Every equation holds over the executed units; the population scope is not
    /// complete (unsupported holders, partial enumeration or budget).
    ReconciledForExecutedUnits,
    /// At least one equation fails.
    Mismatch,
    /// No unit was executed.
    NothingExecuted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reconciliation {
    pub version: String,
    pub status: ReconciliationStatus,
    pub not_fully_reconciled_because: Vec<String>,
    pub captured_positive_source_raw: String,
    pub captured_eligible_source_raw: String,
    pub unmigratable_source_raw: BTreeMap<String, String>,
    pub attempted_units: usize,
    pub migrated_units: usize,
    pub rejected_units: usize,
    pub mismatched_units: usize,
    pub simulated_consumed_raw: String,
    pub source_burned_raw: String,
    pub source_escrowed_gross_raw: String,
    pub source_escrow_withheld_fee_raw: String,
    pub migration_fee_source_raw: String,
    pub destination_released_raw: String,
    pub destination_minted_raw: String,
    pub destination_credited_net_raw: String,
    pub destination_withheld_fee_raw: String,
    pub expected_output_raw: String,
    pub rounding_delta_numerator: String,
    pub rounding_denominator: String,
    pub required_reserve_raw: Option<String>,
    pub available_reserve_raw: Option<String>,
    pub remaining_reserve_raw: Option<String>,
    pub reserve_shortfall_raw: Option<String>,
    pub failed_attempted_source_raw: String,
    pub equations: Vec<Equation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationRehearsal {
    pub version: String,
    pub max_units: usize,
    pub truncated: bool,
    pub not_attempted_by_budget: usize,
    pub executions: Vec<UnitExecution>,
    pub reconciliation: Reconciliation,
}

fn amounts(data: Option<&crate::types::AccountSnapshot>) -> Result<(u128, u128)> {
    data.map(|a| {
        crate::standard_programs::token::account_amounts(&a.owner, &a.data)
            .map(|(amount, fee)| (u128::from(amount), u128::from(fee)))
    })
    .transpose()
    .map(|v| v.unwrap_or((0, 0)))
}
fn supply(data: Option<&crate::types::AccountSnapshot>) -> Result<u128> {
    data.map(|a| {
        crate::standard_programs::token::mint_base(&a.owner, &a.data).map(|m| u128::from(m.supply))
    })
    .transpose()
    .map(|v| v.unwrap_or(0))
}

fn sum(values: impl Iterator<Item = u128>) -> u128 {
    values.fold(0u128, |a, b| a.saturating_add(b))
}

pub fn rehearse(
    spec: &TokenMigrationV1,
    world: &World,
    plan: &MigrationPlan,
    programs: &[LoadedProgram],
    max_units: usize,
) -> Result<PopulationRehearsal> {
    let resolved = spec.resolve()?;
    let config = adapter::config_bytes(spec, &resolved, &plan.overlay, &plan.change_spec_id)?;
    let bank = Bank::build(world, spec, plan, &config)?;
    let mut session = Session::new(&bank, programs, &plan.overlay.program_id)?;
    let attempted: Vec<_> = plan
        .units
        .iter()
        .filter(|u| execute::attempted(u))
        .collect();
    let truncated = attempted.len() > max_units;
    let mut executions = Vec::new();
    for unit in attempted.iter().take(max_units) {
        executions.push(execute::execute_unit(
            &mut session,
            spec,
            plan,
            unit,
            &bank.relayer,
        )?);
    }
    let before = |a: &str| bank.get(a).cloned();
    let after = |a: &str| session.account(a);
    let source_supply_decrease = supply(before(&spec.source.mint).as_ref())?
        .saturating_sub(supply(after(&spec.source.mint).as_ref())?);
    let destination_minted = supply(after(&spec.destination.mint).as_ref())?
        .saturating_sub(supply(before(&spec.destination.mint).as_ref())?);
    let (escrow_gross, escrow_fee) = match &plan.overlay.escrow_vault {
        Some(e) => {
            let (a0, f0) = amounts(before(e).as_ref())?;
            let (a1, f1) = amounts(after(e).as_ref())?;
            ((a1 + f1).saturating_sub(a0 + f0), f1.saturating_sub(f0))
        }
        None => (0, 0),
    };
    let (reserve_before, reserve_after) = match &plan.overlay.reserve_vault {
        Some(r) => (
            Some(amounts(before(r).as_ref())?.0),
            Some(amounts(after(r).as_ref())?.0),
        ),
        None => (None, None),
    };
    let released = match (reserve_before, reserve_after) {
        (Some(b), Some(a)) => b.saturating_sub(a),
        _ => 0,
    };
    let migrated: Vec<&UnitExecution> = executions
        .iter()
        .filter(|e| e.outcome == Outcome::Migrated)
        .collect();
    let succeeded: Vec<&UnitExecution> = executions
        .iter()
        .filter(|e| e.outcome != Outcome::Rejected)
        .collect();
    let parse = |s: &str| s.parse::<i128>().unwrap_or(0).max(0) as u128;
    let consumed = sum(succeeded.iter().map(|e| parse(&e.deltas.source_debit_raw)));
    let credited = sum(succeeded
        .iter()
        .map(|e| parse(&e.deltas.destination_net_credit_raw)));
    let withheld = sum(succeeded
        .iter()
        .map(|e| parse(&e.deltas.destination_withheld_fee_raw)));
    let unit_of = |id: &str| plan.units.iter().find(|u| u.unit_id == id);
    let expected_output = sum(succeeded.iter().filter_map(|e| {
        unit_of(&e.unit_id)
            .and_then(|u| u.quote.as_ref())
            .map(|q| parse(&q.output_raw))
    }));
    let migration_fee = sum(succeeded.iter().filter_map(|e| {
        unit_of(&e.unit_id)
            .and_then(|u| u.quote.as_ref())
            .map(|q| parse(&q.fee_raw))
    }));
    let rounding: i128 = succeeded
        .iter()
        .filter_map(|e| unit_of(&e.unit_id).and_then(|u| u.quote.as_ref()))
        .map(|q| q.rounding_delta_numerator.parse::<i128>().unwrap_or(0))
        .sum();
    let failed_attempted = sum(executions
        .iter()
        .filter(|e| e.outcome == Outcome::Rejected)
        .map(|e| parse(&e.amount_raw)));
    let failed_debits = sum(executions
        .iter()
        .filter(|e| e.outcome == Outcome::Rejected)
        .map(|e| parse(&e.deltas.source_debit_raw)));
    let delivered = match spec.destination_funding {
        DestinationFunding::ReserveTransfer { .. } => released,
        DestinationFunding::MintTo => destination_minted,
    };
    let mut equations = vec![];
    let mut eq = |name: &str, left: u128, right: u128| {
        equations.push(Equation {
            name: name.into(),
            left: left.to_string(),
            right: right.to_string(),
            holds: left == right,
        })
    };
    match spec.source_disposition {
        SourceDisposition::Burn => eq(
            "source supply burned = Σ source debited",
            source_supply_decrease,
            consumed,
        ),
        SourceDisposition::Escrow => {
            eq(
                "escrow gross credit = Σ source debited",
                escrow_gross,
                consumed,
            );
            eq(
                "source supply unchanged by escrow",
                source_supply_decrease,
                0,
            );
        }
    }
    match spec.destination_funding {
        DestinationFunding::ReserveTransfer { .. } => {
            eq(
                "reserve released = Σ expected outputs",
                released,
                expected_output,
            );
            eq(
                "destination supply unchanged by reserve transfer",
                destination_minted,
                0,
            );
        }
        DestinationFunding::MintTo => eq(
            "destination minted = Σ expected outputs",
            destination_minted,
            expected_output,
        ),
    }
    eq(
        "Σ destination net credit + Σ destination withheld fee = delivered",
        credited + withheld,
        delivered,
    );
    eq("rejected units debited nothing", failed_debits, 0);
    let mut class_balances: BTreeMap<String, u128> = BTreeMap::new();
    let mut positive_total = 0u128;
    for unit in &plan.units {
        let balance = parse(&unit.source_balance_raw);
        positive_total += balance;
        if !execute::attempted(unit) {
            *class_balances
                .entry(format!("{:?}", unit.class))
                .or_default() += balance;
        }
    }
    let eligible = sum(plan
        .units
        .iter()
        .filter(|u| execute::attempted(u))
        .map(|u| parse(&u.source_balance_raw)));
    eq(
        "Σ eligible + Σ unmigratable = captured positive balance",
        eligible + sum(class_balances.values().copied()),
        positive_total,
    );
    // The planner sums the output of every eligible unit before it marks the
    // ones the reserve cannot cover, so this is already the full requirement.
    let required = parse(&plan.funding.required_for_migratable_raw);
    if let (Some(b), Some(a)) = (reserve_before, reserve_after) {
        eq(
            "available reserve − released = remaining reserve",
            b.saturating_sub(released),
            a,
        );
    }
    let mismatched = executions
        .iter()
        .filter(|e| e.outcome == Outcome::ReconciliationMismatch)
        .count();
    let all_hold = equations.iter().all(|e| e.holds) && mismatched == 0;
    let mut because = vec![];
    let complete = matches!(
        plan.population_enumeration.as_str(),
        "CompleteForQuery" | "CompleteSyntheticFixture"
    );
    if !complete {
        because.push(format!(
            "Population enumeration is {}; holders outside it are unknown.",
            plan.population_enumeration
        ));
    }
    if truncated {
        because.push(format!(
            "The rehearsal budget of {max_units} units left {} units unexecuted.",
            attempted.len() - max_units
        ));
    }
    let stranded: usize = plan
        .units
        .iter()
        .filter(|u| u.source_balance_raw != "0" && !execute::attempted(u))
        .count();
    if stranded > 0 {
        let holders = if stranded == 1 { "holder" } else { "holders" };
        because.push(format!(
            "{stranded} positive-balance {holders} cannot be attempted under the current mechanism."
        ));
    }
    let rejected = executions
        .iter()
        .filter(|e| e.outcome == Outcome::Rejected)
        .count();
    if rejected > 0 {
        because.push(format!(
            "{rejected} attempted units were rejected in the VM."
        ));
    }
    let status = if executions.is_empty() {
        ReconciliationStatus::NothingExecuted
    } else if !all_hold {
        ReconciliationStatus::Mismatch
    } else if because.is_empty() {
        ReconciliationStatus::FullyReconciled
    } else {
        ReconciliationStatus::ReconciledForExecutedUnits
    };
    let reserve_mode = plan.overlay.reserve_vault.is_some();
    Ok(PopulationRehearsal {
        version: REHEARSAL_VERSION.into(),
        max_units,
        truncated,
        not_attempted_by_budget: attempted.len().saturating_sub(max_units),
        reconciliation: Reconciliation {
            version: REHEARSAL_VERSION.into(),
            status,
            not_fully_reconciled_because: because,
            captured_positive_source_raw: positive_total.to_string(),
            captured_eligible_source_raw: eligible.to_string(),
            unmigratable_source_raw: class_balances
                .into_iter()
                .filter(|(_, v)| *v > 0)
                .map(|(k, v)| (k, v.to_string()))
                .collect(),
            attempted_units: executions.len(),
            migrated_units: migrated.len(),
            rejected_units: rejected,
            mismatched_units: mismatched,
            simulated_consumed_raw: consumed.to_string(),
            source_burned_raw: source_supply_decrease.to_string(),
            source_escrowed_gross_raw: escrow_gross.to_string(),
            source_escrow_withheld_fee_raw: escrow_fee.to_string(),
            migration_fee_source_raw: migration_fee.to_string(),
            destination_released_raw: released.to_string(),
            destination_minted_raw: destination_minted.to_string(),
            destination_credited_net_raw: credited.to_string(),
            destination_withheld_fee_raw: withheld.to_string(),
            expected_output_raw: expected_output.to_string(),
            rounding_delta_numerator: rounding.to_string(),
            rounding_denominator: plan.effective_terms.denominator.to_string(),
            required_reserve_raw: reserve_mode.then(|| required.to_string()),
            available_reserve_raw: reserve_before.map(|b| b.to_string()),
            remaining_reserve_raw: reserve_after.map(|a| a.to_string()),
            reserve_shortfall_raw: reserve_before.map(|b| required.saturating_sub(b).to_string()),
            failed_attempted_source_raw: failed_attempted.to_string(),
            equations,
        },
        executions,
    })
}
