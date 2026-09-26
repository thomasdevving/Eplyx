//! The unsigned, deterministic migration execution plan.
//!
//! The clean boundary between evidence/planning and live execution. The plan lists,
//! per migratable holder, the exact instruction descriptors the VM rehearsal
//! executed, the signer roles each needs, the preconditions and the expected state
//! deltas, plus the setup the rollout requires first (deploy the exact candidate,
//! create its configuration, fund the reserve or hand over mint authority). It has
//! no recent blockhash, no signatures and no key material, and Eplyx never submits
//! it: a future wallet, issuer or relayer tool would have to sign and send it, after
//! re-checking every precondition against then-current state.
use super::{
    adapter::{self, InstructionDescriptor},
    authority::RequiredSigner,
    execute::{self, UnitExecution},
    planner::{ExpectedDeltas, ImpactClass, MigrationPlan},
    spec::{DestinationFunding, MigrationAuthority, Reserve, TokenMigrationV1},
};
use crate::replay::hash_bytes as sha256;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const UNSIGNED_SCHEMA: u32 = 1;
pub const KIND: &str = "eplyx-unsigned-migration-plan";
pub const NOTICE: &str = "UNSIGNED EXECUTION PLAN. Not a transaction, not signed, contains no private keys and was never submitted. Rehearsal results do not show that a mainnet migration happened.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupRequirement {
    pub step: String,
    pub performed_by: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "optional_hex")]
    pub data: Option<Vec<u8>>,
    /// Owner program of `data` (configuration account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_program: Option<String>,
    /// A token account the rollout must create and fund before migrating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_account: Option<SetupTokenAccount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupTokenAccount {
    pub address: String,
    pub mint: String,
    pub token_program: String,
    pub owner: String,
    pub amount_raw: String,
}

mod optional_hex {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => s.serialize_some(&crate::hexfmt::encode(bytes)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let text: Option<String> = Option::deserialize(d)?;
        text.map(|t| crate::hexfmt::decode(&t).map_err(serde::de::Error::custom))
            .transpose()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsignedUnit {
    pub unit_id: String,
    pub source_account: String,
    pub owner: String,
    pub amount_raw: String,
    pub instructions: Vec<InstructionDescriptor>,
    pub fee_payer_role: String,
    pub required_signers: Vec<RequiredSigner>,
    pub preconditions: Vec<String>,
    pub expected: Option<ExpectedDeltas>,
    /// The VM rehearsal of these exact descriptors, for cross-checking.
    pub rehearsal: Option<RehearsalCrossCheck>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RehearsalCrossCheck {
    pub outcome: String,
    pub message_sha256: String,
    pub relayer_in_rehearsal: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsignedPlan {
    pub schema_version: u32,
    pub kind: String,
    pub notice: String,
    pub analysis_input_sha256: String,
    pub candidate_program_sha256: String,
    pub change_spec_id: String,
    pub plan_sha256: String,
    pub world_sha256: String,
    pub program_id: String,
    pub recent_blockhash: String,
    /// The Clock the rehearsal pinned (a derived activation Clock is labelled).
    pub rehearsal_clock: super::planner::RehearsalClock,
    pub setup_requirements: Vec<SetupRequirement>,
    pub units: Vec<UnsignedUnit>,
    pub excluded_units: BTreeMap<String, usize>,
    pub limitations: Vec<String>,
}

impl UnsignedPlan {
    pub fn sha256(&self) -> Result<String> {
        crate::canonical::digest(self)
    }
}

pub fn build(
    spec: &TokenMigrationV1,
    plan: &MigrationPlan,
    analysis_input_sha256: &str,
    config_bytes: &[u8],
    rehearsal: &[UnitExecution],
) -> Result<UnsignedPlan> {
    let overlay = &plan.overlay;
    let mut setup = vec![
        SetupRequirement {
            step: "Deploy the exact candidate program".into(),
            performed_by: "operator".into(),
            detail: format!(
                "Deploy the SBF build with SHA-256 {} at program ID {}. Rehearsal loaded it under {}; an upgradeable deployment may meter compute differently.",
                plan.candidate_program_sha256, overlay.program_id, adapter::CANDIDATE_LOADER
            ),
            address: Some(overlay.program_id.clone()),
            data_sha256: Some(plan.candidate_program_sha256.clone()),
            data: None,
            owner_program: None,
            token_account: None,
        },
        SetupRequirement {
            step: "Create the migration configuration account".into(),
            performed_by: "operator".into(),
            detail: format!(
                "The candidate must hold exactly these {} bytes at the configuration PDA, owned by the candidate program. The v1 ABI has no initializer instruction; creating it is part of deployment.",
                config_bytes.len()
            ),
            address: Some(overlay.config.clone()),
            data_sha256: Some(sha256(config_bytes)),
            data: Some(config_bytes.to_vec()),
            owner_program: Some(overlay.program_id.clone()),
            token_account: None,
        },
    ];
    match &spec.destination_funding {
        DestinationFunding::ReserveTransfer { reserve } => setup.push(SetupRequirement {
            step: "Fund the reserve vault".into(),
            performed_by: "issuer or operator".into(),
            detail: match reserve {
                Reserve::Proposed { funded_raw } => format!(
                    "Create the reserve vault (ATA of the migration authority for the destination mint under {}) and fund it with at least {} raw; the specification proposes {funded_raw} raw.",
                    spec.destination.token_program, plan.funding.required_for_migratable_raw
                ),
                Reserve::Observed { account } => format!(
                    "The observed reserve {account} must hold at least {} raw and stay owned by the migration authority.",
                    plan.funding.required_for_migratable_raw
                ),
            },
            address: overlay.reserve_vault.clone(),
            data_sha256: None,
            data: None,
            owner_program: None,
            token_account: match (reserve, &overlay.reserve_vault) {
                (Reserve::Proposed { .. }, Some(vault)) => Some(SetupTokenAccount {
                    address: vault.clone(),
                    mint: spec.destination.mint.clone(),
                    token_program: spec.destination.token_program.clone(),
                    owner: overlay.migration_authority.clone(),
                    amount_raw: plan
                        .funding
                        .available_raw
                        .clone()
                        .unwrap_or_else(|| "0".into()),
                }),
                _ => None,
            },
        }),
        DestinationFunding::MintTo => setup.push(SetupRequirement {
            step: "Hand destination mint authority to the migration authority".into(),
            performed_by: "issuer (current destination mint authority)".into(),
            detail: format!(
                "SetAuthority(MintTokens) on {} to {}. Eplyx never assumes it holds this authority.",
                spec.destination.mint, overlay.migration_authority
            ),
            address: Some(spec.destination.mint.clone()),
            data_sha256: None,
            data: None,
            owner_program: None,
            token_account: None,
        }),
    }
    if let Some(escrow) = &overlay.escrow_vault {
        setup.push(SetupRequirement {
            step: "Create the escrow vault".into(),
            performed_by: "operator".into(),
            detail: format!(
                "Create the escrow vault (ATA of the migration authority for the source mint under {}).",
                spec.source.token_program
            ),
            address: Some(escrow.clone()),
            data_sha256: None,
            data: None,
            owner_program: None,
            token_account: Some(SetupTokenAccount {
                address: escrow.clone(),
                mint: spec.source.mint.clone(),
                token_program: spec.source.token_program.clone(),
                owner: overlay.migration_authority.clone(),
                amount_raw: "0".into(),
            }),
        });
    }
    if let MigrationAuthority::External { address } = &spec.authorities.migration_authority {
        setup.push(SetupRequirement {
            step: "Arrange the external migration authority co-signature".into(),
            performed_by: "external migration authority".into(),
            detail: format!("{address} must co-sign every migration transaction. Key possession is unknown to Eplyx."),
            address: Some(address.clone()),
            data_sha256: None,
            data: None,
            owner_program: None,
            token_account: None,
        });
    }
    let rehearsed: BTreeMap<&str, &UnitExecution> =
        rehearsal.iter().map(|e| (e.unit_id.as_str(), e)).collect();
    let relayer = execute::relayer().to_string();
    let mut units = vec![];
    let mut excluded: BTreeMap<String, usize> = BTreeMap::new();
    for unit in &plan.units {
        if unit.class != ImpactClass::Migratable {
            *excluded.entry(format!("{:?}", unit.class)).or_default() += 1;
            continue;
        }
        let (instructions, _) = execute::unit_instructions(spec, plan, unit, &relayer)?;
        let mut preconditions = vec![
            format!(
                "Source {} holds exactly {} raw of {} under {} and is not frozen.",
                unit.source_account, unit.amount_raw, spec.source.mint, spec.source.token_program
            ),
            format!(
                "The migration window is open at execution time (rehearsed at slot {}, unix {}).",
                plan.rehearsal_clock.clock.slot, plan.rehearsal_clock.clock.unix_timestamp
            ),
        ];
        preconditions.push(match unit.destination.action.as_str() {
            "CreateAssociated" => format!(
                "Destination ATA {} does not exist yet or is created idempotently for owner {} under {}.",
                unit.destination.address, unit.owner, spec.destination.token_program
            ),
            _ => format!(
                "Destination {} exists, is owned by {} and is not frozen.",
                unit.destination.address, unit.owner
            ),
        });
        if let Some(cumulative) = &unit.cumulative_output_raw {
            preconditions.push(format!(
                "The reserve still holds this unit's output when it executes; in plan order the reserve released through this unit is {cumulative} raw."
            ));
        }
        units.push(UnsignedUnit {
            unit_id: unit.unit_id.clone(),
            source_account: unit.source_account.clone(),
            owner: unit.owner.clone(),
            amount_raw: unit.amount_raw.clone(),
            instructions,
            fee_payer_role: "relayer".into(),
            required_signers: unit.required_signers.clone(),
            preconditions,
            expected: unit.expected.clone(),
            rehearsal: rehearsed
                .get(unit.unit_id.as_str())
                .map(|e| RehearsalCrossCheck {
                    outcome: format!("{:?}", e.outcome),
                    message_sha256: e.message_sha256.clone(),
                    relayer_in_rehearsal: relayer.clone(),
                }),
        });
    }
    Ok(UnsignedPlan {
        schema_version: UNSIGNED_SCHEMA,
        kind: KIND.into(),
        notice: NOTICE.into(),
        analysis_input_sha256: analysis_input_sha256.into(),
        candidate_program_sha256: plan.candidate_program_sha256.clone(),
        change_spec_id: plan.change_spec_id.clone(),
        plan_sha256: plan.sha256()?,
        world_sha256: plan.world_sha256.clone(),
        program_id: overlay.program_id.clone(),
        recent_blockhash: "to be supplied by the executor at signing time".into(),
        rehearsal_clock: plan.rehearsal_clock.clone(),
        setup_requirements: setup,
        units,
        excluded_units: excluded,
        limitations: vec![
            "The relayer address in each instruction is the rehearsal's synthetic relayer; an executor substitutes its own fee payer, which changes the message digest.".into(),
            "Preconditions were true in the rehearsal bank only; an executor must re-verify them against then-current chain state.".into(),
            "Required signers are roles and addresses; Eplyx does not know whether anyone holds their keys.".into(),
        ],
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossCheck {
    pub unit_id: String,
    pub outcome: String,
    pub message_sha256: String,
    pub matches_rehearsal: bool,
}

/// Execute a serialized unsigned plan in a fresh local VM built only from the world
/// and the plan's own setup requirements, then compare every rehearsed unit's
/// message and outcome with the rehearsal. This checks that the descriptors alone reproduce the VM model; it
/// never signs, and it is not a submission.
pub fn cross_check(
    plan: &UnsignedPlan,
    world: &super::world::World,
    programs: &[crate::executor::LoadedProgram],
) -> Result<Vec<CrossCheck>> {
    use super::execute::{Bank, BankAccount, BankOrigin, Session};
    use crate::types::AccountSnapshot;
    use anyhow::Context;
    let mut accounts: BTreeMap<String, BankAccount> = world
        .accounts
        .iter()
        .filter(|(address, _)| address.as_str() != crate::standard_programs::token::CLOCK)
        .map(|(address, entry)| {
            (
                address.clone(),
                BankAccount {
                    account: entry.account.clone(),
                    origin: BankOrigin::World {
                        origin: entry.origin.clone(),
                    },
                },
            )
        })
        .collect();
    for setup in &plan.setup_requirements {
        let proposed = |derivation: &str| BankOrigin::Proposed {
            derivation: format!("Unsigned plan setup: {derivation}"),
        };
        if let (Some(address), Some(data), Some(owner)) =
            (&setup.address, &setup.data, &setup.owner_program)
        {
            accounts.insert(
                address.clone(),
                BankAccount {
                    account: AccountSnapshot {
                        lamports: super::execute::PROPOSED_LAMPORTS,
                        owner: owner.clone(),
                        data: data.clone(),
                        executable: false,
                        rent_epoch: 0,
                    },
                    origin: proposed(&setup.step),
                },
            );
        }
        if let Some(token) = &setup.token_account {
            let mint = world
                .snapshot(&token.mint)
                .context("setup token account mint is not in the world")?;
            accounts.insert(
                token.address.clone(),
                BankAccount {
                    account: AccountSnapshot {
                        lamports: super::execute::PROPOSED_LAMPORTS,
                        owner: token.token_program.clone(),
                        data: crate::standard_programs::token::proposed_token_account(
                            &mint.data,
                            &token.mint.parse()?,
                            &token.owner.parse()?,
                            token.amount_raw.parse()?,
                        )?,
                        executable: false,
                        rent_epoch: 0,
                    },
                    origin: proposed(&setup.step),
                },
            );
        }
    }
    let relayer = super::execute::relayer().to_string();
    accounts.insert(
        relayer.clone(),
        BankAccount {
            account: AccountSnapshot {
                lamports: super::execute::RELAYER_LAMPORTS,
                owner: crate::migration::world::SYSTEM_PROGRAM.into(),
                data: vec![],
                executable: false,
                rent_epoch: 0,
            },
            origin: BankOrigin::LocalRelayer,
        },
    );
    let bank = Bank {
        accounts,
        clock: plan.rehearsal_clock.clock,
        clock_basis: plan.rehearsal_clock.basis.clone(),
        relayer: relayer.clone(),
    };
    let mut session = Session::new(&bank, programs, &plan.program_id)?;
    let mut results = vec![];
    // Units beyond the rehearsal budget have nothing to compare with.
    for unit in plan.units.iter().filter(|u| u.rehearsal.is_some()) {
        let raw = session.execute(&unit.instructions, &relayer)?;
        let outcome = if raw.success { "Migrated" } else { "Rejected" };
        let matches = unit.rehearsal.as_ref().is_some_and(|r| {
            r.message_sha256 == raw.message_sha256
                && (r.outcome == outcome || (r.outcome == "ReconciliationMismatch" && raw.success))
        });
        results.push(CrossCheck {
            unit_id: unit.unit_id.clone(),
            outcome: outcome.into(),
            message_sha256: raw.message_sha256,
            matches_rehearsal: matches,
        });
    }
    Ok(results)
}
