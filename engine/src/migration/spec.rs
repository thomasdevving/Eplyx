//! TokenMigrationV1: a purely declarative, versioned token migration specification.
//!
//! The specification states *what* the proposed migration is: source and destination
//! assets with their owning token programs, the exact conversion terms, an optional
//! window, explicit eligibility, how source tokens leave the holder, how destination
//! tokens are funded and which authorities the migration expects. It carries no
//! secrets, no RPC endpoint, no account metas, no transaction bytes and no claimed
//! status. Everything a rehearsal executes is reconstructed from these fields, the
//! captured state and the registered adapter ABI.
use crate::standard_programs::token::{LEGACY_PROGRAM, TOKEN_2022_PROGRAM};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use solana_address::Address;

pub const VERSION: u32 = 1;
pub const MAX_EXCLUDED_ACCOUNTS: usize = 256;

/// One side of the migration: an asset, the token program that owns it and the
/// decimals the operator expects. The planner re-reads all three from captured state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TokenSide {
    pub mint: String,
    pub token_program: String,
    pub decimals: u8,
}

impl TokenSide {
    pub fn program(&self) -> Result<TokenProgram> {
        TokenProgram::from_address(&self.token_program)
    }
}

/// The two token programs Eplyx models. They are never interchangeable: account
/// layouts, extensions, ATA derivation and instruction targets all depend on this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TokenProgram {
    SplToken,
    SplToken2022,
}

impl TokenProgram {
    pub fn from_address(address: &str) -> Result<Self> {
        match address {
            LEGACY_PROGRAM => Ok(Self::SplToken),
            TOKEN_2022_PROGRAM => Ok(Self::SplToken2022),
            other => bail!("unsupported token program {other}; expected {LEGACY_PROGRAM} or {TOKEN_2022_PROGRAM}"),
        }
    }
    pub fn address(self) -> &'static str {
        match self {
            Self::SplToken => LEGACY_PROGRAM,
            Self::SplToken2022 => TOKEN_2022_PROGRAM,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::SplToken => "SPL Token",
            Self::SplToken2022 => "Token-2022",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RatioBasis {
    /// numerator/denominator applies to raw base units directly.
    Raw,
    /// numerator/denominator applies to whole tokens; decimals are scaled exactly.
    Ui,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rounding {
    Floor,
    Ceiling,
}

impl Rounding {
    pub fn code(self) -> u8 {
        match self {
            Self::Floor => 0,
            Self::Ceiling => 1,
        }
    }
}

/// A migration fee, taken in source units before the ratio applies. It is never a
/// Token-2022 transfer fee; those are reported separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Fee {
    None,
    /// floor(consumed × bps / 10 000) source units are not converted.
    SourceBps {
        bps: u16,
    },
}

impl Fee {
    pub fn bps(self) -> u16 {
        match self {
            Self::None => 0,
            Self::SourceBps { bps } => bps,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Conversion {
    pub ratio_basis: RatioBasis,
    pub numerator: String,
    pub denominator: String,
    pub rounding: Rounding,
    pub fee: Fee,
    /// The smallest destination output the migration accepts; smaller outputs fail.
    pub minimum_output_raw: String,
}

/// A window boundary on the pinned Clock. Activation is inclusive, the deadline is
/// exclusive: a migration is open when `activation <= clock < deadline`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowBoundary {
    Slot { value: String },
    UnixTimestamp { value: String },
}

impl WindowBoundary {
    pub fn value(&self) -> Result<u64> {
        match self {
            Self::Slot { value } | Self::UnixTimestamp { value } => canonical_u64(value),
        }
    }
    pub fn is_slot(&self) -> bool {
        matches!(self, Self::Slot { .. })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Window {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<WindowBoundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<WindowBoundary>,
}

/// Which token-account authority may authorize consumption of a holder's source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HolderAuthorization {
    /// The token account owner signs (a wallet, or an SPL multisig threshold).
    Owner,
    /// An approved delegate with sufficient delegated amount signs.
    Delegate,
    /// The source mint's Token-2022 permanent delegate signs. This is an issuer
    /// capability outside Eplyx; it is never assumed to be available.
    PermanentDelegate,
}

/// Which kinds of owner authority the migration is intended to reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerAuthorityClass {
    Wallet,
    Multisig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmountPolicy {
    /// The holder's entire source balance at the pinned state is migrated.
    FullBalance,
    /// An explicit proposed amount per eligible source account. The original
    /// full-balance encoding is unchanged. Amounts above the observed balance
    /// are outside eligibility, never clamped or manufactured from a mutation.
    ExactRaw { amount_raw: String },
}
impl AmountPolicy {
    pub fn amount(&self, balance: u64) -> Result<u64> {
        match self {
            Self::FullBalance => Ok(balance),
            Self::ExactRaw { amount_raw } => canonical_u64(amount_raw),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Eligibility {
    pub amount_policy: AmountPolicy,
    pub minimum_source_balance_raw: String,
    pub holder_authorization: Vec<HolderAuthorization>,
    pub owner_authority_classes: Vec<OwnerAuthorityClass>,
    /// Explicitly excluded source token accounts, for example an issuer treasury.
    #[serde(default)]
    pub excluded_accounts: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceDisposition {
    /// Source tokens are burned through the source token program.
    Burn,
    /// Source tokens are transferred into an escrow vault of the migration authority.
    Escrow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reserve {
    /// A reserve vault the operator proposes to fund before rollout. It is derived
    /// deterministically and never described as observed chain state.
    Proposed {
        #[serde(rename = "funded_raw")]
        funded_raw: String,
    },
    /// An existing token account captured from the chain. Its token owner must be
    /// the migration authority.
    Observed { account: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DestinationFunding {
    /// Destination tokens are transferred from a pre-funded reserve.
    ReserveTransfer { reserve: Reserve },
    /// Destination tokens are minted. The destination mint authority must be the
    /// migration authority; Eplyx never assumes it owns issuer mint authority.
    MintTo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MigrationAuthority {
    /// A program-derived address of the candidate migration program. It signs
    /// through the candidate program and is assumed locally before deployment.
    ProgramDerived,
    /// An external key (issuer or operator) that must co-sign every migration.
    /// Eplyx assumes the signature locally; key possession stays unknown.
    External { address: String },
}

/// An asserted expectation about a captured authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthorityExpectation {
    /// The authority must be absent (revoked).
    None,
    Address {
        address: String,
    },
    /// The authority must equal the migration authority.
    MigrationAuthority,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct AuthorityExpectations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_mint_authority: Option<AuthorityExpectation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_freeze_authority: Option<AuthorityExpectation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_mint_authority: Option<AuthorityExpectation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_freeze_authority: Option<AuthorityExpectation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeePayer {
    /// A relayer role pays transaction fees and destination-account rent. It is
    /// a required signer of the unsigned plan; the rehearsal uses a synthetic payer.
    Relayer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Authorities {
    pub migration_authority: MigrationAuthority,
    #[serde(default)]
    pub expected: AuthorityExpectations,
    pub fee_payer: FeePayer,
}

/// The identifying proposal inside MAIN's ChangeSpec. State and evaluation policy
/// are separate inputs. Activation belongs to the enclosing ChangeSpec.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenMigration {
    pub source: TokenSide,
    pub destination: TokenSide,
    pub conversion: Conversion,
    pub eligibility: Eligibility,
    pub source_disposition: SourceDisposition,
    pub destination_funding: DestinationFunding,
    pub authorities: Authorities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<WindowBoundary>,
    pub mechanism: Mechanism,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mechanism {
    pub program_id: String,
    pub artifact: crate::change::ExecutableArtifact,
}
impl TokenMigration {
    /// A detached evaluation view, never a second identity or executable input.
    pub fn evaluation_spec(
        &self,
        activation: Option<&crate::change::Activation>,
    ) -> Result<TokenMigrationV1> {
        let activation = activation
            .map(|a| match (a.slot, a.unix_timestamp) {
                (Some(slot), None) => Ok(WindowBoundary::Slot {
                    value: slot.to_string(),
                }),
                (None, Some(time)) if time >= 0 => Ok(WindowBoundary::UnixTimestamp {
                    value: time.to_string(),
                }),
                _ => bail!("migration activation requires exactly one nonnegative time axis"),
            })
            .transpose()?;
        let terms = TokenMigrationV1 {
            version: VERSION,
            source: self.source.clone(),
            destination: self.destination.clone(),
            conversion: self.conversion.clone(),
            window: Window {
                activation,
                deadline: self.deadline.clone(),
            },
            eligibility: self.eligibility.clone(),
            source_disposition: self.source_disposition,
            destination_funding: self.destination_funding.clone(),
            authorities: self.authorities.clone(),
        };
        terms.validate()?;
        let program = address(&self.mechanism.program_id, "migration mechanism")?;
        for reserved in [
            &self.source.mint,
            &self.destination.mint,
            &self.source.token_program,
            &self.destination.token_program,
        ] {
            ensure!(
                program.to_string() != *reserved,
                "mechanism program collides with migration identity"
            );
        }
        ensure!(
            program.to_string() != crate::standard_programs::token::ATA_PROGRAM,
            "mechanism program collides with token account program"
        );
        Ok(terms)
    }
}

/// Construction/evaluation view of the terms and their resolved window.
/// It does not name executable bytes and is never a stored ChangeSpec identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TokenMigrationV1 {
    pub version: u32,
    pub source: TokenSide,
    pub destination: TokenSide,
    pub conversion: Conversion,
    #[serde(default)]
    pub window: Window,
    pub eligibility: Eligibility,
    pub source_disposition: SourceDisposition,
    pub destination_funding: DestinationFunding,
    pub authorities: Authorities,
}

pub fn canonical_u64(input: &str) -> Result<u64> {
    let parsed: u64 = input
        .parse()
        .with_context(|| format!("invalid integer {input:?}"))?;
    ensure!(
        parsed.to_string() == input,
        "integer {input:?} is not canonical"
    );
    Ok(parsed)
}

fn address(input: &str, what: &str) -> Result<Address> {
    let address: Address = input
        .parse()
        .with_context(|| format!("invalid {what} address {input:?}"))?;
    ensure!(address.to_string() == input, "noncanonical {what} address");
    Ok(address)
}

impl TokenMigrationV1 {
    /// Structural validation only. Captured state is checked by the planner.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == VERSION,
            "unsupported TokenMigration version"
        );
        let source = address(&self.source.mint, "source mint")?;
        let destination = address(&self.destination.mint, "destination mint")?;
        ensure!(
            source != destination,
            "source and destination mints must differ"
        );
        TokenProgram::from_address(&self.source.token_program)?;
        TokenProgram::from_address(&self.destination.token_program)?;
        crate::migration::economics::effective_terms(
            &self.conversion,
            self.source.decimals,
            self.destination.decimals,
        )
        .map_err(|error| anyhow::anyhow!("invalid conversion terms: {error}"))?;
        let minimum_balance = canonical_u64(&self.eligibility.minimum_source_balance_raw)?;
        if let AmountPolicy::ExactRaw { amount_raw } = &self.eligibility.amount_policy {
            ensure!(
                canonical_u64(amount_raw)? > 0,
                "exact migration amount must be positive"
            );
        }
        ensure!(
            minimum_balance >= 1,
            "minimumSourceBalanceRaw must be at least 1"
        );
        let authorization = &self.eligibility.holder_authorization;
        ensure!(
            !authorization.is_empty() && authorization.windows(2).all(|w| w[0] < w[1]),
            "holderAuthorization must be a nonempty, sorted list without duplicates"
        );
        let classes = &self.eligibility.owner_authority_classes;
        ensure!(
            !classes.is_empty() && classes.windows(2).all(|w| w[0] < w[1]),
            "ownerAuthorityClasses must be a nonempty, sorted list without duplicates"
        );
        let excluded = &self.eligibility.excluded_accounts;
        ensure!(
            excluded.len() <= MAX_EXCLUDED_ACCOUNTS && excluded.windows(2).all(|w| w[0] < w[1]),
            "excludedAccounts must be sorted, unique and at most {MAX_EXCLUDED_ACCOUNTS}"
        );
        for account in excluded {
            address(account, "excluded account")?;
        }
        self.validate_window()?;
        let external = match &self.authorities.migration_authority {
            MigrationAuthority::ProgramDerived => None,
            MigrationAuthority::External { address: key } => {
                let key = address(key, "external migration authority")?;
                ensure!(
                    key != source && key != destination,
                    "the migration authority cannot be a mint"
                );
                Some(key)
            }
        };
        match &self.destination_funding {
            DestinationFunding::ReserveTransfer { reserve } => match reserve {
                Reserve::Proposed { funded_raw } => {
                    canonical_u64(funded_raw).context("invalid proposed reserve fundedRaw")?;
                }
                Reserve::Observed { account } => {
                    let reserve = address(account, "observed reserve")?;
                    ensure!(
                        reserve != source && reserve != destination,
                        "the reserve account cannot be a mint"
                    );
                }
            },
            DestinationFunding::MintTo => {
                if let Some(expectation) = &self.authorities.expected.destination_mint_authority {
                    let consistent = match expectation {
                        AuthorityExpectation::MigrationAuthority => true,
                        AuthorityExpectation::Address { address: key } => {
                            external.is_some() && Some(address(key, "authority")?) == external
                        }
                        AuthorityExpectation::None => false,
                    };
                    ensure!(
                        consistent,
                        "mintTo requires the destination mint authority to be the migration authority"
                    );
                }
            }
        }
        for expectation in [
            &self.authorities.expected.source_mint_authority,
            &self.authorities.expected.source_freeze_authority,
            &self.authorities.expected.destination_mint_authority,
            &self.authorities.expected.destination_freeze_authority,
        ]
        .into_iter()
        .flatten()
        {
            if let AuthorityExpectation::Address { address: key } = expectation {
                address(key, "expected authority")?;
            }
        }
        Ok(())
    }

    fn validate_window(&self) -> Result<()> {
        let window = &self.window;
        for boundary in [&window.activation, &window.deadline].into_iter().flatten() {
            boundary.value()?;
        }
        if let (Some(activation), Some(deadline)) = (&window.activation, &window.deadline) {
            ensure!(
                activation.is_slot() == deadline.is_slot(),
                "activation and deadline must use the same clock basis"
            );
            ensure!(
                deadline.value()? > activation.value()?,
                "the migration deadline must follow its activation"
            );
        }
        if let Some(deadline) = &window.deadline {
            ensure!(deadline.value()? > 0, "a zero deadline can never be open");
        }
        Ok(())
    }

    pub fn allows(&self, authorization: HolderAuthorization) -> bool {
        self.eligibility
            .holder_authorization
            .contains(&authorization)
    }
    pub fn allows_owner_class(&self, class: OwnerAuthorityClass) -> bool {
        self.eligibility.owner_authority_classes.contains(&class)
    }

    /// Validate and resolve every typed value the planner and adapter consume.
    pub fn resolve(&self) -> Result<ResolvedMigration> {
        self.validate()?;
        let terms = crate::migration::economics::effective_terms(
            &self.conversion,
            self.source.decimals,
            self.destination.decimals,
        )
        .map_err(|error| anyhow::anyhow!("invalid conversion terms: {error}"))?;
        let boundary = |b: &Option<WindowBoundary>| -> Result<Option<(bool, u64)>> {
            b.as_ref()
                .map(|b| Ok((b.is_slot(), b.value()?)))
                .transpose()
        };
        Ok(ResolvedMigration {
            source_program: self.source.program()?,
            destination_program: self.destination.program()?,
            terms,
            minimum_source_balance: canonical_u64(&self.eligibility.minimum_source_balance_raw)?,
            proposed_reserve: match &self.destination_funding {
                DestinationFunding::ReserveTransfer {
                    reserve: Reserve::Proposed { funded_raw },
                } => Some(canonical_u64(funded_raw)?),
                _ => None,
            },
            activation: boundary(&self.window.activation)?,
            deadline: boundary(&self.window.deadline)?,
        })
    }
}

/// Typed values of a validated specification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedMigration {
    pub source_program: TokenProgram,
    pub destination_program: TokenProgram,
    pub terms: crate::migration::economics::EffectiveTerms,
    pub minimum_source_balance: u64,
    pub proposed_reserve: Option<u64>,
    /// (is_slot, value); activation inclusive.
    pub activation: Option<(bool, u64)>,
    /// (is_slot, value); deadline exclusive.
    pub deadline: Option<(bool, u64)>,
}

impl ResolvedMigration {
    /// Whether the window is open at the pinned Clock (activation inclusive,
    /// deadline exclusive). A negative Unix timestamp precedes every boundary.
    pub fn window_state(&self, slot: u64, unix_timestamp: i64) -> WindowState {
        let at = |is_slot: bool| -> Option<u64> {
            if is_slot {
                Some(slot)
            } else {
                u64::try_from(unix_timestamp).ok()
            }
        };
        if let Some((is_slot, value)) = self.activation {
            if at(is_slot).is_none_or(|now| now < value) {
                return WindowState::NotYetActive;
            }
        }
        if let Some((is_slot, value)) = self.deadline {
            if at(is_slot).is_some_and(|now| now >= value) {
                return WindowState::Closed;
            }
        }
        WindowState::Open
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowState {
    NotYetActive,
    Open,
    Closed,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn example() -> TokenMigrationV1 {
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "source": {"mint": "So11111111111111111111111111111111111111112", "token_program": LEGACY_PROGRAM, "decimals": 9},
            "destination": {"mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", "token_program": LEGACY_PROGRAM, "decimals": 6},
            "conversion": {"ratio_basis": "raw", "numerator": "1", "denominator": "2", "rounding": "floor", "fee": {"kind": "none"}, "minimum_output_raw": "1"},
            "window": {"activation": {"kind": "slot", "value": "100"}, "deadline": {"kind": "slot", "value": "200"}},
            "eligibility": {"amount_policy": "full_balance", "minimum_source_balance_raw": "1", "holder_authorization": ["owner", "delegate"], "owner_authority_classes": ["wallet", "multisig"]},
            "source_disposition": {"kind": "burn"},
            "destination_funding": {"kind": "reserve_transfer", "reserve": {"kind": "proposed", "funded_raw": "1000"}},
            "authorities": {"migration_authority": {"kind": "program_derived"}, "fee_payer": {"kind": "relayer"}}
        }))
        .unwrap()
    }

    #[test]
    fn example_round_trips_and_validates() {
        let spec = example();
        spec.validate().unwrap();
        let text = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            serde_json::from_str::<TokenMigrationV1>(&text).unwrap(),
            spec
        );
        assert_eq!(
            spec.resolve().unwrap().source_program,
            TokenProgram::SplToken
        );
    }

    #[test]
    fn unknown_fields_programs_and_noncanonical_integers_are_refused() {
        let mut value = serde_json::to_value(example()).unwrap();
        value["rpc_url"] = "https://example.invalid".into();
        assert!(serde_json::from_value::<TokenMigrationV1>(value).is_err());

        let mut spec = example();
        spec.source.token_program = "11111111111111111111111111111111".into();
        assert!(spec.validate().is_err());

        let mut spec = example();
        spec.conversion.numerator = "01".into();
        assert!(spec.validate().is_err());

        let mut spec = example();
        spec.destination.mint = spec.source.mint.clone();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn window_is_activation_inclusive_and_deadline_exclusive() {
        let spec = example().resolve().unwrap();
        assert_eq!(spec.window_state(99, 0), WindowState::NotYetActive);
        assert_eq!(spec.window_state(100, 0), WindowState::Open);
        assert_eq!(spec.window_state(199, 0), WindowState::Open);
        assert_eq!(spec.window_state(200, 0), WindowState::Closed);
        let mut reversed = example();
        reversed.window.deadline = Some(WindowBoundary::Slot {
            value: "100".into(),
        });
        assert!(reversed.validate().is_err());
        let mut mixed = example();
        mixed.window.deadline = Some(WindowBoundary::UnixTimestamp {
            value: "1760000000".into(),
        });
        assert!(mixed.validate().is_err());
    }

    #[test]
    fn authorization_lists_must_be_sorted_and_unique() {
        let mut spec = example();
        spec.eligibility.holder_authorization =
            vec![HolderAuthorization::Delegate, HolderAuthorization::Owner];
        assert!(spec.validate().is_err());
        let mut spec = example();
        spec.eligibility.owner_authority_classes = vec![];
        assert!(spec.validate().is_err());
    }

    #[test]
    fn mint_to_rejects_contradictory_mint_authority_expectation() {
        let mut spec = example();
        spec.destination_funding = DestinationFunding::MintTo;
        spec.authorities.expected.destination_mint_authority = Some(AuthorityExpectation::None);
        assert!(spec.validate().is_err());
        spec.authorities.expected.destination_mint_authority =
            Some(AuthorityExpectation::MigrationAuthority);
        spec.validate().unwrap();
    }
}
