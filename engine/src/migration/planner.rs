//! The deterministic migration planner.
//!
//! Given a TokenMigrationV1 specification, a rehearsal world, the candidate program
//! identity and the adapter ABI, the planner produces a versioned, hashable plan: one
//! unit per source token account with its authority path, destination state, exact
//! expected arithmetic and post-state, and one impact class with machine-readable
//! reasons. The plan is execution intent and evidence input. It contains no keys,
//! signs nothing, and never marks an account migratable without an executable path.
use super::{
    adapter::{self, Overlay},
    authority::{self, AuthorityPath, OwnerClass, RequiredSigner, SignerRole},
    economics::{self, EffectiveTerms, QuoteRecord},
    extensions::{self, ExtensionFinding, Side},
    spec::{
        AuthorityExpectation, DestinationFunding, MigrationAuthority, Reserve, ResolvedMigration,
        SourceDisposition, TokenMigrationV1, WindowState,
    },
    world::{associated_token_address, World, WorldClock, WorldKind},
};
use crate::standard_programs::token::{self as decode, MintConfig, TokenAccountState};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PLAN_SCHEMA: u32 = 1;
pub const PLANNER_VERSION: &str = "eplyx-migration-planner/v1";

/// Which Clock the rehearsal pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RehearsalClockPolicy {
    /// The captured (or fixture) Clock exactly.
    Captured,
    /// The captured Clock moved forward to the declared activation boundary when
    /// the capture predates it. All account state stays as captured.
    Activation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RehearsalClock {
    pub basis: String,
    pub clock: WorldClock,
    #[serde(with = "crate::numfmt::u64_string")]
    pub captured_slot: u64,
    #[serde(with = "crate::numfmt::i64_string")]
    pub captured_unix_timestamp: i64,
}

/// Where one account ends up. Exactly one class per unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ImpactClass {
    /// Executable under the current mechanism with the resolved authority path.
    Migratable,
    ZeroBalance,
    /// Outside eligibility: below the minimum balance or explicitly excluded.
    OutsideEligibility,
    /// The exact economics yield zero or below-minimum output, or overflow.
    OutputBelowMinimum,
    Frozen,
    UninitializedSource,
    /// A Token-2022 extension or state the mechanism cannot faithfully execute.
    UnsupportedTokenSemantics,
    /// No authorization path the specification allows (program-controlled owner,
    /// ineligible owner class, partial delegation).
    AuthorityPathUnavailable,
    /// The owner could not be verified or was not inspected.
    UnverifiableAuthority,
    /// The destination account is frozen, foreign, would be created frozen, or
    /// requires handling the mechanism lacks.
    InvalidDestinationState,
    /// The destination account was not inspected; its state is unknown.
    UnverifiableDestination,
    /// The pinned Clock is outside the migration window.
    WindowConflict,
    /// Destination funding cannot be performed by the migration authority.
    FundingPathUnavailable,
    /// The declared reserve cannot cover this unit after earlier units in plan order.
    InsufficientReserve,
    /// The mint state itself prevents migration (paused, unsupported extension).
    MintStateBlocksMigration,
}

impl ImpactClass {
    pub fn code(self) -> &'static str {
        match self {
            Self::Migratable => "MIGRATABLE",
            Self::ZeroBalance => "ZERO_BALANCE",
            Self::OutsideEligibility => "OUTSIDE_ELIGIBILITY",
            Self::OutputBelowMinimum => "OUTPUT_BELOW_MINIMUM",
            Self::Frozen => "SOURCE_FROZEN",
            Self::UninitializedSource => "SOURCE_UNINITIALIZED",
            Self::UnsupportedTokenSemantics => "UNSUPPORTED_TOKEN_EXTENSION",
            Self::AuthorityPathUnavailable => "AUTHORITY_PATH_UNAVAILABLE",
            Self::UnverifiableAuthority => "AUTHORITY_UNVERIFIABLE",
            Self::InvalidDestinationState => "DESTINATION_STATE_INVALID",
            Self::UnverifiableDestination => "DESTINATION_NOT_INSPECTED",
            Self::WindowConflict => "MIGRATION_WINDOW_INVALID",
            Self::FundingPathUnavailable => "FUNDING_PATH_UNAVAILABLE",
            Self::InsufficientReserve => "INSUFFICIENT_RESERVE",
            Self::MintStateBlocksMigration => "MINT_STATE_BLOCKS_MIGRATION",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reason {
    pub code: String,
    pub detail: String,
}

fn reason(code: &str, detail: impl Into<String>) -> Reason {
    Reason {
        code: code.into(),
        detail: detail.into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationPlan {
    pub address: String,
    pub token_program: String,
    /// `Existing`, `CreateAssociated` or `NotInspected`.
    pub action: String,
    pub state: Option<String>,
    pub findings: Vec<ExtensionFinding>,
}

/// Exact expected state changes for a migratable unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedDeltas {
    pub source_debit_raw: String,
    pub source_after_raw: String,
    pub source_supply_decrease_raw: String,
    pub escrow_credit_raw: String,
    pub escrow_transfer_fee_raw: String,
    pub reserve_debit_raw: String,
    pub destination_supply_increase_raw: String,
    pub destination_credit_raw: String,
    pub destination_transfer_fee_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationUnit {
    pub unit_id: String,
    pub source_account: String,
    pub owner: String,
    pub source_balance_raw: String,
    pub source_state: String,
    pub delegate: Option<String>,
    pub delegated_amount_raw: String,
    pub account_findings: Vec<ExtensionFinding>,
    pub authority: AuthorityPath,
    pub destination: DestinationPlan,
    pub class: ImpactClass,
    pub reasons: Vec<Reason>,
    pub amount_raw: String,
    pub quote: Option<QuoteRecord>,
    pub expected: Option<ExpectedDeltas>,
    /// Position in funding order and cumulative output through this unit.
    pub funding_order: Option<usize>,
    pub cumulative_output_raw: Option<String>,
    pub required_signers: Vec<RequiredSigner>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub check: String,
    pub expected: String,
    pub observed: String,
    pub satisfied: bool,
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FundingPlan {
    pub mode: String,
    pub origin: String,
    pub account: String,
    pub available_raw: Option<String>,
    pub required_for_migratable_raw: String,
    pub path_available: bool,
    pub reasons: Vec<Reason>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassTotal {
    pub accounts: usize,
    pub balance_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationPlan {
    pub schema_version: u32,
    pub planner_version: String,
    pub adapter: String,
    pub change_spec_id: String,
    pub candidate_program_sha256: String,
    pub world_sha256: String,
    pub world_kind: WorldKind,
    pub rehearsal_clock: RehearsalClock,
    pub window_state: WindowState,
    pub effective_terms: EffectiveTerms,
    pub overlay: Overlay,
    pub identity_checks: Vec<Check>,
    pub mint_findings: Vec<ExtensionFinding>,
    pub funding: FundingPlan,
    pub units: Vec<MigrationUnit>,
    pub class_totals: BTreeMap<String, ClassTotal>,
    pub authority_totals: BTreeMap<String, ClassTotal>,
    pub population_enumeration: String,
    pub authority_resolution: String,
    pub undecoded_accounts: Vec<String>,
    /// A derived proposed-reserve amount replacing the specification's funding
    /// (stress/search only). Never present in a package's own plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserve_override_raw: Option<String>,
}

impl MigrationPlan {
    pub fn sha256(&self) -> Result<String> {
        crate::canonical::digest(self)
    }
    pub fn unit(&self, source_account: &str) -> Option<&MigrationUnit> {
        self.units
            .iter()
            .find(|u| u.source_account == source_account)
    }
}

pub fn rehearsal_clock(
    resolved: &ResolvedMigration,
    captured: &WorldClock,
    policy: RehearsalClockPolicy,
) -> RehearsalClock {
    let mut clock = *captured;
    let mut basis = "Captured".to_string();
    if policy == RehearsalClockPolicy::Activation {
        if let Some((is_slot, value)) = resolved.activation {
            if is_slot && captured.slot < value {
                clock.slot = value;
                basis = "DerivedAtActivationSlot".into();
            } else if !is_slot && u64::try_from(captured.unix_timestamp).unwrap_or(0) < value {
                clock.unix_timestamp = i64::try_from(value).unwrap_or(i64::MAX);
                basis = "DerivedAtActivationTimestamp".into();
            }
        }
    }
    RehearsalClock {
        basis,
        clock,
        captured_slot: captured.slot,
        captured_unix_timestamp: captured.unix_timestamp,
    }
}

fn expectation_check(
    name: &str,
    expectation: &Option<AuthorityExpectation>,
    observed: &Option<String>,
    migration_authority: &str,
) -> Option<Check> {
    let expectation = expectation.as_ref()?;
    let (expected, satisfied) = match expectation {
        AuthorityExpectation::None => ("none".to_string(), observed.is_none()),
        AuthorityExpectation::Address { address } => {
            (address.clone(), observed.as_deref() == Some(address))
        }
        AuthorityExpectation::MigrationAuthority => (
            migration_authority.to_string(),
            observed.as_deref() == Some(migration_authority),
        ),
    };
    Some(Check {
        check: name.into(),
        expected,
        observed: observed.clone().unwrap_or_else(|| "none".into()),
        satisfied,
        code: "REQUIRED_AUTHORITY_MISMATCH".into(),
    })
}

fn transfer_fee(mint_bytes: &[u8], epoch: u64, amount: u64) -> Result<u64> {
    crate::standard_programs::token::transfer_fee(mint_bytes, epoch, amount)
}

pub struct PlanInput<'a> {
    pub spec: &'a TokenMigrationV1,
    pub change_spec_id: &'a str,
    pub world: &'a World,
    pub program_id: &'a str,
    pub candidate_program_sha256: &'a str,
    pub clock_policy: RehearsalClockPolicy,
    /// Derived proposed-reserve funding for stress/search; `None` for the package plan.
    pub reserve_override: Option<u64>,
    /// Plan only this source account (an isolated stress or search case). The
    /// reserve is then judged for this unit alone. `None` plans the population.
    pub focus: Option<&'a str>,
}

pub fn plan(input: &PlanInput<'_>) -> Result<MigrationPlan> {
    let spec = input.spec;
    let world = input.world;
    world.validate()?;
    let resolved = spec.resolve()?;
    let overlay = adapter::derive(spec, input.change_spec_id, input.program_id)?;
    let clock = rehearsal_clock(&resolved, &world.clock, input.clock_policy);
    let window_state = resolved.window_state(clock.clock.slot, clock.clock.unix_timestamp);

    // Identity: spec-declared mints, token programs and decimals against captured state.
    let source_mint = world
        .mint(&spec.source.mint)
        .context("the source mint is not an initialized token mint in the world")?;
    let destination_mint = world
        .mint(&spec.destination.mint)
        .context("the destination mint is not an initialized token mint in the world")?;
    ensure!(
        world.population.source_mint == spec.source.mint,
        "the world population is for a different source mint"
    );
    let mut identity_checks = vec![];
    for (name, expected, observed) in [
        (
            "source token program",
            spec.source.token_program.clone(),
            source_mint.token_program.clone(),
        ),
        (
            "destination token program",
            spec.destination.token_program.clone(),
            destination_mint.token_program.clone(),
        ),
        (
            "source decimals",
            spec.source.decimals.to_string(),
            source_mint.decimals.to_string(),
        ),
        (
            "destination decimals",
            spec.destination.decimals.to_string(),
            destination_mint.decimals.to_string(),
        ),
    ] {
        identity_checks.push(Check {
            check: name.into(),
            satisfied: expected == observed,
            expected,
            observed,
            code: "MINT_IDENTITY_MISMATCH".into(),
        });
    }
    let expected = &spec.authorities.expected;
    let authority = overlay.migration_authority.as_str();
    identity_checks.extend(
        [
            expectation_check(
                "source mint authority",
                &expected.source_mint_authority,
                &source_mint.mint_authority,
                authority,
            ),
            expectation_check(
                "source freeze authority",
                &expected.source_freeze_authority,
                &source_mint.freeze_authority,
                authority,
            ),
            expectation_check(
                "destination mint authority",
                &expected.destination_mint_authority,
                &destination_mint.mint_authority,
                authority,
            ),
            expectation_check(
                "destination freeze authority",
                &expected.destination_freeze_authority,
                &destination_mint.freeze_authority,
                authority,
            ),
        ]
        .into_iter()
        .flatten(),
    );
    let identity_ok = identity_checks
        .iter()
        .filter(|c| c.code == "MINT_IDENTITY_MISMATCH")
        .all(|c| c.satisfied);
    ensure!(
        identity_ok,
        "captured mint identity differs from the specification: {}",
        identity_checks
            .iter()
            .filter(|c| !c.satisfied && c.code == "MINT_IDENTITY_MISMATCH")
            .map(|c| format!(
                "{} expected {} observed {}",
                c.check, c.expected, c.observed
            ))
            .collect::<Vec<_>>()
            .join("; ")
    );

    // Proposed overlay addresses must be free in the world, or the collision unknown.
    let mut overlay_reasons = vec![];
    for (role, address, proposed) in [
        ("migration config", Some(overlay.config.clone()), true),
        (
            "reserve vault",
            overlay.reserve_vault.clone(),
            overlay.reserve_origin.as_deref() == Some("Proposed"),
        ),
        ("escrow vault", overlay.escrow_vault.clone(), true),
    ] {
        let Some(address) = address else { continue };
        if !proposed {
            continue;
        }
        if world.get(&address).is_some() {
            anyhow::bail!("the proposed {role} {address} collides with an existing account");
        }
        if !world.absence_known(&address) {
            overlay_reasons.push(reason(
                "PROPOSED_ADDRESS_NOT_INSPECTED",
                format!("The proposed {role} {address} was not inspected; a collision cannot be excluded."),
            ));
        }
    }

    let mut mint_findings = extensions::classify_mint(spec, Side::Source, &source_mint);
    mint_findings.extend(extensions::classify_mint(
        spec,
        Side::Destination,
        &destination_mint,
    ));
    let mint_blockers: Vec<Reason> = mint_findings
        .iter()
        .filter(|f| !f.support.executable())
        .map(|f| {
            reason(
                &f.code,
                format!("{:?} {}: {}", f.side, f.extension, f.semantics),
            )
        })
        .collect();

    // Funding path.
    let destination_mint_bytes = world
        .snapshot(&spec.destination.mint)
        .context("destination mint bytes")?
        .data
        .clone();
    let source_mint_bytes = world
        .snapshot(&spec.source.mint)
        .context("source mint bytes")?
        .data
        .clone();
    let mut funding_reasons = overlay_reasons.clone();
    let (mode, origin, available) = match &spec.destination_funding {
        DestinationFunding::ReserveTransfer { reserve } => match reserve {
            Reserve::Proposed { .. } => (
                "ReserveTransfer",
                if input.reserve_override.is_some() {
                    "DerivedProposed"
                } else {
                    "Proposed"
                },
                input.reserve_override.or(resolved.proposed_reserve),
            ),
            Reserve::Observed { account } => {
                let state = world.token_account(
                    account,
                    &spec.destination.token_program,
                    &spec.destination.mint,
                    spec.destination.decimals,
                );
                match state {
                    Ok(Some(state)) => {
                        if state.owner != overlay.migration_authority {
                            funding_reasons.push(reason(
                                "RESERVE_AUTHORITY_MISMATCH",
                                format!("The observed reserve's token owner is {}, not the migration authority {}.", state.owner, overlay.migration_authority),
                            ));
                        }
                        if state.is_frozen {
                            funding_reasons.push(reason(
                                "RESERVE_FROZEN",
                                "The observed reserve account is frozen.",
                            ));
                        }
                        (
                            "ReserveTransfer",
                            "Observed",
                            state.raw_balance.parse().ok(),
                        )
                    }
                    Ok(None) => {
                        funding_reasons.push(reason(
                            if world.absence_known(account) {
                                "RESERVE_MISSING"
                            } else {
                                "RESERVE_NOT_INSPECTED"
                            },
                            format!("The observed reserve {account} is not in the world."),
                        ));
                        ("ReserveTransfer", "Observed", None)
                    }
                    Err(error) => {
                        funding_reasons.push(reason("RESERVE_INVALID", format!("{error:#}")));
                        ("ReserveTransfer", "Observed", None)
                    }
                }
            }
        },
        DestinationFunding::MintTo => {
            if destination_mint.mint_authority.as_deref() != Some(&overlay.migration_authority) {
                funding_reasons.push(reason(
                    "MINT_AUTHORITY_MISMATCH",
                    format!(
                        "Minting requires the destination mint authority to be the migration authority {}; the captured mint authority is {}. Eplyx does not assume issuer mint authority.",
                        overlay.migration_authority,
                        destination_mint.mint_authority.as_deref().unwrap_or("none (revoked)")
                    ),
                ));
            }
            ("MintTo", "DestinationMint", None)
        }
    };
    let funding_available = !funding_reasons
        .iter()
        .any(|r| r.code != "PROPOSED_ADDRESS_NOT_INSPECTED");

    // Units, in population order.
    let terms = resolved.terms;
    let minimum_balance = resolved.minimum_source_balance;
    let mut units = Vec::with_capacity(world.population.token_accounts.len());
    for source_account in &world.population.token_accounts {
        if input.focus.is_some_and(|focus| focus != source_account) {
            continue;
        }
        let state = world
            .token_account(
                source_account,
                &spec.source.token_program,
                &spec.source.mint,
                spec.source.decimals,
            )?
            .context("population account vanished")?;
        units.push(plan_unit(
            spec,
            world,
            &source_mint,
            &destination_mint,
            &destination_mint_bytes,
            &source_mint_bytes,
            source_account,
            &state,
            &terms,
            minimum_balance,
            &clock,
            window_state,
            &mint_blockers,
            funding_available,
            &funding_reasons,
        )?);
    }

    // Reserve sufficiency in deterministic plan order.
    let mut cumulative: u128 = 0;
    let mut required: u128 = 0;
    let mut order = 0;
    for unit in units.iter_mut() {
        if unit.class != ImpactClass::Migratable {
            continue;
        }
        let output: u128 = unit
            .quote
            .as_ref()
            .map(|q| q.output_raw.parse().unwrap_or(0))
            .unwrap_or(0);
        required += output;
        if mode == "ReserveTransfer" {
            // Sequential semantics: a unit the remaining reserve cannot cover fails
            // and consumes nothing, so a later, smaller unit may still fit.
            unit.funding_order = Some(order);
            order += 1;
            let next = cumulative + output;
            match available {
                Some(available) if next > u128::from(available) => {
                    unit.class = ImpactClass::InsufficientReserve;
                    unit.cumulative_output_raw = Some(next.to_string());
                    unit.reasons.push(reason(
                        "INSUFFICIENT_RESERVE",
                        format!(
                            "Earlier units in plan order leave {} raw of the {available} raw reserve; this unit needs {output} raw.",
                            u128::from(available) - cumulative
                        ),
                    ));
                }
                _ => {
                    cumulative = next;
                    unit.cumulative_output_raw = Some(cumulative.to_string());
                }
            }
        }
    }
    let mut class_totals: BTreeMap<String, (usize, u128)> = BTreeMap::new();
    let mut authority_totals: BTreeMap<String, (usize, u128)> = BTreeMap::new();
    for unit in &units {
        let balance: u128 = unit.source_balance_raw.parse().unwrap_or(0);
        let entry = class_totals.entry(format!("{:?}", unit.class)).or_default();
        entry.0 += 1;
        entry.1 += balance;
        if balance > 0 {
            let entry = authority_totals
                .entry(format!("{:?}", unit.authority.required()))
                .or_default();
            entry.0 += 1;
            entry.1 += balance;
        }
    }
    let totals = |m: BTreeMap<String, (usize, u128)>| {
        m.into_iter()
            .map(|(k, (accounts, balance))| {
                (
                    k,
                    ClassTotal {
                        accounts,
                        balance_raw: balance.to_string(),
                    },
                )
            })
            .collect()
    };
    Ok(MigrationPlan {
        schema_version: PLAN_SCHEMA,
        planner_version: PLANNER_VERSION.into(),
        adapter: adapter::ADAPTER.into(),
        change_spec_id: input.change_spec_id.into(),
        candidate_program_sha256: input.candidate_program_sha256.into(),
        world_sha256: world.sha256()?,
        world_kind: world.kind,
        rehearsal_clock: clock,
        window_state,
        effective_terms: terms,
        overlay: overlay.clone(),
        identity_checks,
        mint_findings,
        funding: FundingPlan {
            mode: mode.into(),
            origin: origin.into(),
            account: overlay.funding_account.clone(),
            available_raw: available.map(|a| a.to_string()),
            required_for_migratable_raw: required.to_string(),
            path_available: funding_available,
            reasons: funding_reasons,
        },
        units,
        class_totals: totals(class_totals),
        authority_totals: totals(authority_totals),
        population_enumeration: world.population.enumeration_completeness.clone(),
        authority_resolution: world.population.authority_resolution_completeness.clone(),
        undecoded_accounts: world.population.undecoded_accounts.clone(),
        reserve_override_raw: input.reserve_override.map(|r| r.to_string()),
    })
}

#[allow(clippy::too_many_arguments)]
fn plan_unit(
    spec: &TokenMigrationV1,
    world: &World,
    source_mint: &MintConfig,
    destination_mint: &MintConfig,
    destination_mint_bytes: &[u8],
    source_mint_bytes: &[u8],
    source_account: &str,
    state: &TokenAccountState,
    terms: &EffectiveTerms,
    minimum_balance: u64,
    clock: &RehearsalClock,
    window_state: WindowState,
    mint_blockers: &[Reason],
    funding_available: bool,
    funding_reasons: &[Reason],
) -> Result<MigrationUnit> {
    let balance: u64 = state.raw_balance.parse()?;
    let owner_value = world.rpc_value(&state.owner);
    let owner_raw = if owner_value.is_null() && !world.absence_known(&state.owner) {
        None
    } else {
        Some(&owner_value)
    };
    let authority = authority::resolve(
        spec,
        &spec.source.token_program,
        source_mint,
        state,
        owner_raw,
        balance,
    )?;
    let holder_is_owner = matches!(
        &authority,
        AuthorityPath::Available {
            authority: authority::HolderAuthority::OwnerWallet { .. }
                | authority::HolderAuthority::OwnerMultisig { .. },
            ..
        }
    );
    let account_findings = extensions::classify_account(spec, Side::Source, state, holder_is_owner);

    // Destination: the owner's canonical ATA for the destination token program.
    let destination_address = associated_token_address(
        &state.owner,
        &spec.destination.token_program,
        &spec.destination.mint,
    )?;
    let mut destination_reasons = vec![];
    let destination = match world.token_account(
        &destination_address,
        &spec.destination.token_program,
        &spec.destination.mint,
        spec.destination.decimals,
    ) {
        Ok(Some(dest)) => {
            let findings = extensions::classify_account(spec, Side::Destination, &dest, false);
            if dest.is_frozen {
                destination_reasons.push(reason(
                    "DESTINATION_FROZEN",
                    "The holder's destination account is frozen.",
                ));
            }
            if !dest.is_initialized {
                destination_reasons.push(reason(
                    "DESTINATION_UNINITIALIZED",
                    "The destination account is not initialized.",
                ));
            }
            if dest.owner != state.owner {
                destination_reasons.push(reason(
                    "DESTINATION_OWNER_MISMATCH",
                    "The destination account belongs to a different owner.",
                ));
            }
            for f in findings.iter().filter(|f| !f.support.executable()) {
                destination_reasons.push(reason(&f.code, f.semantics.clone()));
            }
            DestinationPlan {
                address: destination_address.clone(),
                token_program: spec.destination.token_program.clone(),
                action: "Existing".into(),
                state: Some(dest.account_state.clone()),
                findings,
            }
        }
        Ok(None) if world.absence_known(&destination_address) => {
            if destination_mint
                .extensions
                .iter()
                .any(|e| e.extension_type == "DefaultAccountState" && e.config["state"] == "Frozen")
            {
                destination_reasons.push(reason(
                    "DEFAULT_FROZEN_DESTINATION",
                    "The destination account would be created frozen by the mint's default state and could not receive tokens without a thaw.",
                ));
            }
            DestinationPlan {
                address: destination_address.clone(),
                token_program: spec.destination.token_program.clone(),
                action: "CreateAssociated".into(),
                state: None,
                findings: vec![],
            }
        }
        Ok(None) => DestinationPlan {
            address: destination_address.clone(),
            token_program: spec.destination.token_program.clone(),
            action: "NotInspected".into(),
            state: None,
            findings: vec![],
        },
        Err(error) => {
            destination_reasons.push(reason(
                "DESTINATION_NOT_A_TOKEN_ACCOUNT",
                format!("The canonical destination address holds an account that is not a destination token account: {error:#}"),
            ));
            DestinationPlan {
                address: destination_address.clone(),
                token_program: spec.destination.token_program.clone(),
                action: "Existing".into(),
                state: None,
                findings: vec![],
            }
        }
    };

    let quote = economics::quote(balance, terms);
    let mut reasons = vec![];
    let excluded = spec
        .eligibility
        .excluded_accounts
        .binary_search(&source_account.to_string())
        .is_ok();
    let class = if balance == 0 {
        ImpactClass::ZeroBalance
    } else if !state.is_initialized {
        ImpactClass::UninitializedSource
    } else if excluded {
        reasons.push(reason(
            "EXCLUDED_ACCOUNT",
            "The specification explicitly excludes this account.",
        ));
        ImpactClass::OutsideEligibility
    } else if balance < minimum_balance {
        reasons.push(reason(
            "BELOW_MINIMUM_BALANCE",
            format!("Balance {balance} is below the eligibility minimum {minimum_balance}."),
        ));
        ImpactClass::OutsideEligibility
    } else if window_state != WindowState::Open {
        reasons.push(reason(
            "MIGRATION_WINDOW_INVALID",
            format!(
                "The migration window is {window_state:?} at the pinned Clock (slot {}, unix {}).",
                clock.clock.slot, clock.clock.unix_timestamp
            ),
        ));
        ImpactClass::WindowConflict
    } else if !mint_blockers.is_empty() {
        reasons.extend(mint_blockers.iter().cloned());
        ImpactClass::MintStateBlocksMigration
    } else if state.is_frozen {
        reasons.push(reason(
            "SOURCE_FROZEN",
            "The source account is frozen; only the freeze authority can thaw it.",
        ));
        ImpactClass::Frozen
    } else if let Err(error) = &quote {
        reasons.push(reason(error.name(), format!("Exact economics: {error}.")));
        ImpactClass::OutputBelowMinimum
    } else if let Some(f) = account_findings.iter().find(|f| !f.support.executable()) {
        reasons.push(reason(&f.code, f.semantics.clone()));
        ImpactClass::UnsupportedTokenSemantics
    } else if let AuthorityPath::Unavailable {
        owner_class,
        code,
        reason: why,
        ..
    } = &authority
    {
        reasons.push(reason(code, why.clone()));
        if *owner_class == OwnerClass::Unknown {
            ImpactClass::UnverifiableAuthority
        } else {
            ImpactClass::AuthorityPathUnavailable
        }
    } else if destination.action == "NotInspected" {
        reasons.push(reason(
            "DESTINATION_NOT_INSPECTED",
            "The destination account was not inspected; its state is unknown.",
        ));
        ImpactClass::UnverifiableDestination
    } else if !destination_reasons.is_empty() {
        reasons.extend(destination_reasons);
        ImpactClass::InvalidDestinationState
    } else if !funding_available {
        reasons.extend(funding_reasons.iter().cloned());
        ImpactClass::FundingPathUnavailable
    } else {
        ImpactClass::Migratable
    };
    let quote_record = quote.as_ref().ok().map(|q| q.record());
    let expected = match (&quote, class) {
        (Ok(q), ImpactClass::Migratable) => {
            let escrow_fee = if spec.source_disposition == SourceDisposition::Escrow {
                transfer_fee(source_mint_bytes, clock.clock.epoch, q.consumed)?
            } else {
                0
            };
            let reserve = matches!(
                spec.destination_funding,
                DestinationFunding::ReserveTransfer { .. }
            );
            let destination_fee = if reserve {
                transfer_fee(destination_mint_bytes, clock.clock.epoch, q.output)?
            } else {
                0
            };
            Some(ExpectedDeltas {
                source_debit_raw: q.consumed.to_string(),
                source_after_raw: (balance - q.consumed).to_string(),
                source_supply_decrease_raw: if spec.source_disposition == SourceDisposition::Burn {
                    q.consumed
                } else {
                    0
                }
                .to_string(),
                escrow_credit_raw: if spec.source_disposition == SourceDisposition::Escrow {
                    q.consumed - escrow_fee
                } else {
                    0
                }
                .to_string(),
                escrow_transfer_fee_raw: escrow_fee.to_string(),
                reserve_debit_raw: if reserve { q.output } else { 0 }.to_string(),
                destination_supply_increase_raw: if reserve { 0 } else { q.output }.to_string(),
                destination_credit_raw: (q.output - destination_fee).to_string(),
                destination_transfer_fee_raw: destination_fee.to_string(),
            })
        }
        _ => None,
    };
    let mut required_signers = match &authority {
        AuthorityPath::Available { authority, .. } => authority.signers(),
        _ => vec![],
    };
    if class == ImpactClass::Migratable {
        if let MigrationAuthority::External { address } = &spec.authorities.migration_authority {
            required_signers.push(RequiredSigner::new(
                SignerRole::ExternalMigrationAuthority,
                address,
            ));
        }
        required_signers.push(RequiredSigner::new(SignerRole::Relayer, "relayer"));
    }
    Ok(MigrationUnit {
        unit_id: format!(
            "unit-{}",
            &crate::replay::hash_bytes(source_account.as_bytes())[..16]
        ),
        source_account: source_account.into(),
        owner: state.owner.clone(),
        source_balance_raw: state.raw_balance.clone(),
        source_state: state.account_state.clone(),
        delegate: state.delegate.clone(),
        delegated_amount_raw: state.delegated_amount.clone(),
        account_findings,
        authority,
        destination,
        class,
        reasons,
        amount_raw: balance.to_string(),
        quote: quote_record,
        expected,
        funding_order: None,
        cumulative_output_raw: None,
        required_signers,
    })
}

/// Decode helper for callers holding a unit's source account in a bank.
pub fn decode_source(
    spec: &TokenMigrationV1,
    raw: &serde_json::Value,
) -> Result<TokenAccountState> {
    decode::decode_token_account(
        raw,
        &spec.source.token_program,
        &spec.source.mint,
        spec.source.decimals,
    )
}
