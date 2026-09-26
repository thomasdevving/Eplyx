//! The registered `token_migration_v1` adapter ABI.
//!
//! The adapter fixes the instruction layout, account order, configuration layout,
//! PDA seeds and success log of a migration candidate. It does not fix the program
//! ID or the program bytes: the operator declares the address they intend to deploy
//! under and supplies the exact SBF build, and only VM execution plus exact
//! reconciliation can create evidence for that build. Everything here is derived
//! deterministically from the specification and captured identities; the browser,
//! config and package supply no account metas, instructions or transaction bytes.
use super::{
    economics::EffectiveTerms,
    spec::{
        DestinationFunding, HolderAuthorization, MigrationAuthority, Reserve, ResolvedMigration,
        SourceDisposition, TokenMigrationV1,
    },
    world::associated_token_address,
};
use crate::standard_programs::token::ATA_PROGRAM;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use solana_address::Address;
use solana_instruction::{account_meta::AccountMeta, Instruction};

pub const ADAPTER: &str = "token_migration_v1";
pub const ADAPTER_VERSION: u32 = 1;
pub const REFERENCE_PROGRAM_ID: &str = "Akajkkga5U8d6xkVQUMXjnbdSuMtGAHBLbsr9dUvQGsj";
pub const REFERENCE_PROGRAM_PREIMAGE: &str = "sha256(\"eplyx-token-migration-v1\")";
pub const REFERENCE_ARTIFACT: &str = "artifacts/eplyx_token_migration.so";
/// Candidates use MAIN’s upgradeable-loader execution model in the local VM.
pub const CANDIDATE_LOADER: &str = crate::standard_programs::token::UPGRADEABLE_LOADER;
pub const CONFIG_LEN: usize = 308;
pub const CONFIG_LAYOUT_VERSION: u8 = 1;
pub const MIGRATE_TAG: u8 = 1;
pub const CONFIG_SEED: &[u8] = b"eplyx-migration-config";
pub const AUTHORITY_SEED: &[u8] = b"eplyx-migration-authority";
pub const LOG_PREFIX: &str = "EPLYX_TOKEN_MIGRATION v1";
pub const COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

/// Error names of the ABI, by stable code. An independent copy of the program table.
pub fn error_name(code: u32) -> Option<&'static str> {
    const NAMES: [&str; 31] = [
        "MalformedInstruction",
        "ZeroAmount",
        "ConfigNotOwned",
        "ConfigLayout",
        "AuthorityMismatch",
        "MintMismatch",
        "TokenProgramMismatch",
        "MigrationNotActive",
        "MigrationWindowClosed",
        "SourceAccountMismatch",
        "UnauthorizedHolderAuthority",
        "HolderNotSigner",
        "InsufficientSourceBalance",
        "InvalidTerms",
        "UnsupportedRounding",
        "ArithmeticOverflow",
        "ZeroOutput",
        "OutputBelowMinimum",
        "FundingAccountMismatch",
        "InsufficientReserve",
        "MintAuthorityMismatch",
        "DestinationMismatch",
        "DestinationOwnerMismatch",
        "SourceNotConsumedExactly",
        "SupplyNotReducedExactly",
        "EscrowNotCreditedExactly",
        "ReserveNotReleasedExactly",
        "DestinationNotCreditedExactly",
        "SupplyNotIncreasedExactly",
        "EscrowMismatch",
        "MigrationAuthorityNotSigner",
    ];
    usize::try_from(code)
        .ok()
        .and_then(|c| c.checked_sub(1))
        .and_then(|i| NAMES.get(i).copied())
}

/// Addresses the adapter derives for one specification and program ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overlay {
    pub program_id: String,
    pub config: String,
    pub config_bump: u8,
    pub migration_authority: String,
    pub authority_kind: String,
    pub authority_bump: u8,
    /// Reserve vault (reserve transfer) or destination mint (mint-to).
    pub funding_account: String,
    pub reserve_vault: Option<String>,
    pub reserve_origin: Option<String>,
    pub escrow_vault: Option<String>,
}

fn digest_bytes(digest: &str) -> Result<[u8; 32]> {
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "expected a sha-256 digest"
    );
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&digest[i * 2..i * 2 + 2], 16)?;
    }
    Ok(bytes)
}

pub fn derive(spec: &TokenMigrationV1, change_spec_id: &str, program_id: &str) -> Result<Overlay> {
    let program: Address = program_id.parse().context("invalid candidate program ID")?;
    let (config, config_bump) =
        Address::find_program_address(&[CONFIG_SEED, &digest_bytes(change_spec_id)?], &program);
    let (authority, authority_kind, authority_bump) = match &spec.authorities.migration_authority {
        MigrationAuthority::ProgramDerived => {
            let (pda, bump) =
                Address::find_program_address(&[AUTHORITY_SEED, config.as_ref()], &program);
            (pda.to_string(), "ProgramDerived", bump)
        }
        MigrationAuthority::External { address } => (address.clone(), "External", 0),
    };
    let (funding_account, reserve_vault, reserve_origin) = match &spec.destination_funding {
        DestinationFunding::ReserveTransfer { reserve } => {
            let (vault, origin) = match reserve {
                Reserve::Proposed { .. } => (
                    associated_token_address(
                        &authority,
                        &spec.destination.token_program,
                        &spec.destination.mint,
                    )?,
                    "Proposed",
                ),
                Reserve::Observed { account } => (account.clone(), "Observed"),
            };
            (vault.clone(), Some(vault), Some(origin.to_string()))
        }
        DestinationFunding::MintTo => (spec.destination.mint.clone(), None, None),
    };
    let escrow_vault = match spec.source_disposition {
        SourceDisposition::Burn => None,
        SourceDisposition::Escrow => Some(associated_token_address(
            &authority,
            &spec.source.token_program,
            &spec.source.mint,
        )?),
    };
    Ok(Overlay {
        program_id: program.to_string(),
        config: config.to_string(),
        config_bump,
        migration_authority: authority,
        authority_kind: authority_kind.into(),
        authority_bump,
        funding_account,
        reserve_vault,
        reserve_origin,
        escrow_vault,
    })
}

/// Exact bytes of the candidate configuration account, packed from the ABI layout.
pub fn config_bytes(
    spec: &TokenMigrationV1,
    resolved: &ResolvedMigration,
    overlay: &Overlay,
    change_spec_id: &str,
) -> Result<Vec<u8>> {
    let key = |address: &str| -> Result<[u8; 32]> { Ok(address.parse::<Address>()?.to_bytes()) };
    let EffectiveTerms {
        numerator,
        denominator,
        rounding,
        fee_bps,
        minimum_output,
    } = resolved.terms;
    let mut data = vec![0u8; CONFIG_LEN];
    data[0] = CONFIG_LAYOUT_VERSION;
    data[1..33].copy_from_slice(&key(&spec.source.mint)?);
    data[33..65].copy_from_slice(&key(&spec.destination.mint)?);
    data[65..97].copy_from_slice(&key(&spec.source.token_program)?);
    data[97..129].copy_from_slice(&key(&spec.destination.token_program)?);
    data[129..161].copy_from_slice(&key(&overlay.migration_authority)?);
    data[161] = u8::from(overlay.authority_kind == "External");
    data[162] = overlay.authority_bump;
    data[163] = spec.source.decimals;
    data[164] = spec.destination.decimals;
    data[165..173].copy_from_slice(&numerator.to_le_bytes());
    data[173..181].copy_from_slice(&denominator.to_le_bytes());
    data[181] = rounding.code();
    data[182..184].copy_from_slice(&fee_bps.to_le_bytes());
    data[184..192].copy_from_slice(&minimum_output.to_le_bytes());
    data[192] = u8::from(spec.source_disposition == SourceDisposition::Escrow);
    data[193] = u8::from(spec.destination_funding == DestinationFunding::MintTo);
    data[194..226].copy_from_slice(&key(&overlay.funding_account)?);
    if let Some(escrow) = &overlay.escrow_vault {
        data[226..258].copy_from_slice(&key(escrow)?);
    }
    let (basis, activation, deadline) = match (resolved.activation, resolved.deadline) {
        (None, None) => (0u8, 0u64, u64::MAX),
        (a, d) => {
            let slot = a.or(d).map(|(is_slot, _)| is_slot).unwrap_or(true);
            (
                if slot { 1 } else { 2 },
                a.map_or(0, |(_, v)| v),
                d.map_or(u64::MAX, |(_, v)| v),
            )
        }
    };
    data[258] = basis;
    data[259..267].copy_from_slice(&activation.to_le_bytes());
    data[267..275].copy_from_slice(&deadline.to_le_bytes());
    let mut authorization = 0u8;
    for (flag, allowed) in [
        (1u8, HolderAuthorization::Owner),
        (2, HolderAuthorization::Delegate),
        (4, HolderAuthorization::PermanentDelegate),
    ] {
        if spec.allows(allowed) {
            authorization |= flag;
        }
    }
    data[275] = authorization;
    data[276..308].copy_from_slice(&digest_bytes(change_spec_id)?);
    Ok(data)
}

/// One account of an instruction descriptor, with the role it plays.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountDescriptor {
    pub address: String,
    pub role: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

/// A serializable instruction. The VM rehearsal and the unsigned execution plan use
/// the same descriptors, so instruction construction cannot diverge between them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionDescriptor {
    pub program: String,
    pub purpose: String,
    pub accounts: Vec<AccountDescriptor>,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
}

impl InstructionDescriptor {
    pub fn instruction(&self) -> Result<Instruction> {
        Ok(Instruction {
            program_id: self.program.parse()?,
            accounts: self
                .accounts
                .iter()
                .map(|a| {
                    let key: Address = a.address.parse()?;
                    Ok(if a.is_writable {
                        AccountMeta::new(key, a.is_signer)
                    } else {
                        AccountMeta::new_readonly(key, a.is_signer)
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            data: self.data.clone(),
        })
    }
}

fn meta(address: &str, role: &str, is_signer: bool, is_writable: bool) -> AccountDescriptor {
    AccountDescriptor {
        address: address.into(),
        role: role.into(),
        is_signer,
        is_writable,
    }
}

pub fn compute_budget() -> InstructionDescriptor {
    InstructionDescriptor {
        program: COMPUTE_BUDGET_PROGRAM.into(),
        purpose: "SetComputeUnitLimit".into(),
        accounts: vec![],
        data: [vec![2], COMPUTE_UNIT_LIMIT.to_le_bytes().to_vec()].concat(),
    }
}

/// Create the holder's canonical destination ATA if it does not exist. The token
/// program is part of the derivation and of the instruction.
pub fn create_destination(
    payer: &str,
    destination: &str,
    owner: &str,
    mint: &str,
    token_program: &str,
) -> Result<InstructionDescriptor> {
    ensure!(
        associated_token_address(owner, token_program, mint)? == destination,
        "destination is not the canonical ATA for this token program"
    );
    Ok(InstructionDescriptor {
        program: ATA_PROGRAM.into(),
        purpose: "CreateIdempotent destination associated token account".into(),
        accounts: vec![
            meta(payer, "relayer", true, true),
            meta(destination, "holder-destination", false, true),
            meta(owner, "holder-owner", false, false),
            meta(mint, "destination-mint", false, false),
            meta(SYSTEM_PROGRAM, "system-program", false, false),
            meta(token_program, "destination-token-program", false, false),
        ],
        data: vec![1],
    })
}

/// How the holder authority signs the Migrate instruction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HolderSigning {
    /// The authority account itself signs.
    Direct { authority: String },
    /// An SPL multisig account is the authority; the listed members sign.
    Multisig {
        multisig: String,
        signers: Vec<String>,
    },
}

pub struct MigrateAccounts<'a> {
    pub source: &'a str,
    pub destination: &'a str,
    pub holder: &'a HolderSigning,
}

pub fn migrate(
    spec: &TokenMigrationV1,
    overlay: &Overlay,
    accounts: &MigrateAccounts<'_>,
    amount: u64,
) -> Result<InstructionDescriptor> {
    ensure!(amount > 0, "a migration amount must be positive");
    let mint_to = spec.destination_funding == DestinationFunding::MintTo;
    let external = overlay.authority_kind == "External";
    let mut metas = vec![
        meta(&overlay.config, "migration-config", false, false),
        meta(accounts.source, "holder-source", false, true),
        meta(&spec.source.mint, "source-mint", false, true),
    ];
    match accounts.holder {
        HolderSigning::Direct { authority } => {
            metas.push(meta(authority, "holder-authority", true, false))
        }
        HolderSigning::Multisig { multisig, .. } => {
            metas.push(meta(multisig, "holder-multisig", false, false))
        }
    }
    metas.extend([
        meta(accounts.destination, "holder-destination", false, true),
        meta(&spec.destination.mint, "destination-mint", false, mint_to),
        meta(
            &overlay.migration_authority,
            "migration-authority",
            external,
            false,
        ),
        meta(
            &spec.source.token_program,
            "source-token-program",
            false,
            false,
        ),
        meta(
            &spec.destination.token_program,
            "destination-token-program",
            false,
            false,
        ),
    ]);
    if let Some(reserve) = &overlay.reserve_vault {
        metas.push(meta(reserve, "reserve-vault", false, true));
    }
    if let Some(escrow) = &overlay.escrow_vault {
        metas.push(meta(escrow, "escrow-vault", false, true));
    }
    if let HolderSigning::Multisig { signers, .. } = accounts.holder {
        if signers.is_empty() {
            bail!("a multisig authority needs signers");
        }
        for signer in signers {
            metas.push(meta(signer, "multisig-signer", true, false));
        }
    }
    Ok(InstructionDescriptor {
        program: overlay.program_id.clone(),
        purpose: "Migrate".into(),
        accounts: metas,
        data: [vec![MIGRATE_TAG], amount.to_le_bytes().to_vec()].concat(),
    })
}

/// The exact success log line the ABI requires for a reconciled migration.
pub fn success_log(
    spec: &TokenMigrationV1,
    consumed: u64,
    fee: u64,
    converted: u64,
    output: u64,
) -> String {
    format!(
        "{LOG_PREFIX} consumed={consumed} fee={fee} converted={converted} output={output} disposition={} funding={}",
        match spec.source_disposition {
            SourceDisposition::Burn => "burn",
            SourceDisposition::Escrow => "escrow",
        },
        match spec.destination_funding {
            DestinationFunding::ReserveTransfer { .. } => "reserve",
            DestinationFunding::MintTo => "mint",
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_program_id_is_the_published_preimage() {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(b"eplyx-token-migration-v1");
        let expected = Address::try_from(digest.as_slice()).unwrap();
        assert_eq!(expected.to_string(), REFERENCE_PROGRAM_ID);
    }

    #[test]
    fn error_table_is_one_based_and_bounded() {
        assert_eq!(error_name(1), Some("MalformedInstruction"));
        assert_eq!(error_name(20), Some("InsufficientReserve"));
        assert_eq!(error_name(31), Some("MigrationAuthorityNotSigner"));
        assert_eq!(error_name(0), None);
        assert_eq!(error_name(32), None);
    }

    #[test]
    fn overlay_depends_on_program_spec_and_token_program() {
        let spec = crate::migration::spec::tests::example();
        let digest = "aa".repeat(32);
        let a = derive(&spec, &digest, REFERENCE_PROGRAM_ID).unwrap();
        let b = derive(&spec, &"bb".repeat(32), REFERENCE_PROGRAM_ID).unwrap();
        assert_ne!(a.config, b.config);
        assert_ne!(a.migration_authority, b.migration_authority);
        let mut other = spec.clone();
        other.destination.token_program =
            crate::standard_programs::token::TOKEN_2022_PROGRAM.into();
        let c = derive(&other, &digest, REFERENCE_PROGRAM_ID).unwrap();
        assert_eq!(a.migration_authority, c.migration_authority);
        assert_ne!(a.reserve_vault, c.reserve_vault);
        let resolved = spec.resolve().unwrap();
        let bytes = config_bytes(&spec, &resolved, &a, &digest).unwrap();
        assert_eq!(bytes.len(), CONFIG_LEN);
        assert_eq!(bytes[258], 1, "slot window basis");
        assert_eq!(bytes[275], 3, "owner and delegate authorization");
    }
}
