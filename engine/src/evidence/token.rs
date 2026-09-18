//! Token movement, measured.
//!
//! Every name here is a measurement: a balance went from A to B, a supply rose,
//! an authority changed. None of them is `deposit`, `withdrawal`, `payout` or
//! `fee` — those are interpretations, and a mint can be a receipt, a debt, an
//! LP share or a reward depending on a protocol this module has never heard of.
//!
//! Both token programs are covered. Which one owns an account decides which
//! decoder reads it, and the decoder's identity travels with the evidence: the
//! same 165 leading bytes mean the same thing under both programs, but a
//! Token-2022 account's withheld fee exists only under one of them.

use super::{account::AccountDelta, DecoderIdentity, Provenance};
use crate::standard_programs::{spl_token, token2022};

const SPL_TOKEN_DECODER: DecoderIdentity = DecoderIdentity::standard("spl-token", 1);
const TOKEN_2022_DECODER: DecoderIdentity = DecoderIdentity::standard("token-2022", 1);

/// Which token program an account belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TokenProgram {
    SplToken,
    Token2022,
}

impl TokenProgram {
    pub fn of(owner: &str) -> Option<Self> {
        match owner {
            spl_token::PROGRAM_ID => Some(Self::SplToken),
            token2022::PROGRAM_ID => Some(Self::Token2022),
            _ => None,
        }
    }

    pub fn address(self) -> &'static str {
        match self {
            Self::SplToken => spl_token::PROGRAM_ID,
            Self::Token2022 => token2022::PROGRAM_ID,
        }
    }

    fn decoder(self) -> DecoderIdentity {
        match self {
            Self::SplToken => SPL_TOKEN_DECODER,
            Self::Token2022 => TOKEN_2022_DECODER,
        }
    }

    /// Decode a token account under this program's rules.
    ///
    /// The two differ and the difference is load-bearing: a legacy account is
    /// exactly 165 bytes, while a Token-2022 account may carry extensions past
    /// them. Reading an extended buffer under the legacy decoder would be
    /// claiming one program's guarantee over another program's state.
    pub fn account_amount(self, data: &[u8]) -> Option<u64> {
        match self {
            Self::SplToken => spl_token::account_amount(data),
            Self::Token2022 => token2022::account_amount(data),
        }
    }

    pub fn account_mint(self, data: &[u8]) -> Option<String> {
        match self {
            Self::SplToken => spl_token::account_mint(data),
            Self::Token2022 => token2022::account_mint(data),
        }
    }

    pub fn mint_supply(self, data: &[u8]) -> Option<u64> {
        match self {
            Self::SplToken => spl_token::mint_supply(data),
            Self::Token2022 => token2022::mint_supply(data),
        }
    }

    pub fn mint_decimals(self, data: &[u8]) -> Option<u8> {
        match self {
            Self::SplToken => spl_token::mint_decimals(data),
            Self::Token2022 => token2022::mint_decimals(data),
        }
    }
}

/// One token account's balance across a boundary.
///
/// `delta` is `i128` so a full-range `u64` decrease is representable. It is
/// never rendered as a float and never crosses mints: a caller comparing two
/// deltas must check the mints agree, which is why `mint` is carried here
/// rather than assumed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenBalanceDelta {
    pub provenance: Provenance,
    pub mint: String,
    pub owner: String,
    pub before: u64,
    pub after: u64,
    pub delta: i128,
    pub token_program: TokenProgram,
}

impl TokenBalanceDelta {
    pub fn label(&self) -> &str {
        &self.provenance.account_label
    }

    pub fn increased(&self) -> bool {
        self.delta > 0
    }

    pub fn decreased(&self) -> bool {
        self.delta < 0
    }
}

/// A mint's supply across a boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MintSupplyDelta {
    pub provenance: Provenance,
    pub before: u64,
    pub after: u64,
    pub delta: i128,
    pub decimals: u8,
    pub token_program: TokenProgram,
}

/// A token transfer, where execution evidence makes one deterministically
/// attributable.
///
/// Every field is optional except the ones a transfer cannot exist without,
/// because this is populated from what is actually known. `net_amount` differs
/// from `gross_amount` only where a transfer fee was withheld, and the
/// difference is measured, never modelled.
///
/// Notably absent: any notion of "user" or "vault". Which side of a transfer is
/// a user is a protocol's judgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenTransferEvidence {
    pub source: String,
    pub destination: String,
    pub mint: Option<String>,
    pub authority: Option<String>,
    pub gross_amount: u64,
    pub net_amount: Option<u64>,
    pub withheld_fee: Option<u64>,
    pub token_program: TokenProgram,
    pub origin: Option<super::InvocationOrigin>,
}

/// Tokens created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MintEvidence {
    pub mint: String,
    pub destination: Option<String>,
    pub amount: u64,
    pub token_program: TokenProgram,
    pub origin: Option<super::InvocationOrigin>,
}

/// Tokens destroyed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BurnEvidence {
    pub mint: String,
    pub source: Option<String>,
    pub amount: u64,
    pub token_program: TokenProgram,
    pub origin: Option<super::InvocationOrigin>,
}

/// An authority or delegate field changing on a standard token account.
///
/// Limited on purpose to the fields the token programs define. An arbitrary
/// protocol's admin key living at some offset is not generalised here: this
/// layer would have to guess the layout, and guessing a layout from bytes is
/// the failure this repository already has a scar from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityDelta {
    pub provenance: Provenance,
    pub field: AuthorityField,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorityField {
    AccountOwner,
    Delegate,
    CloseAuthority,
    MintAuthority,
    FreezeAuthority,
    PermanentDelegate,
}

impl AuthorityField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccountOwner => "owner",
            Self::Delegate => "delegate",
            Self::CloseAuthority => "close_authority",
            Self::MintAuthority => "mint_authority",
            Self::FreezeAuthority => "freeze_authority",
            Self::PermanentDelegate => "permanent_delegate",
        }
    }
}

/// Decode a token account under whichever program owns it.
fn read_account(owner: &str, data: &[u8]) -> Option<(TokenProgram, String, String, u64)> {
    let program = TokenProgram::of(owner)?;
    let amount = program.account_amount(data)?;
    let mint = program.account_mint(data)?;
    let owner_key = match program {
        TokenProgram::SplToken => spl_token::decode_account(data).ok().map(|a| a.owner),
        TokenProgram::Token2022 => token2022::decode_account(data).ok().map(|a| a.base.owner),
    }?;
    Some((program, mint, owner_key, amount))
}

/// Balance deltas for every paired account that is a token account on both
/// sides.
///
/// An account that becomes a token account, or stops being one, is not a
/// balance delta — it is a lifecycle event, and reporting it here would invent
/// a "before" the account never had.
pub fn balance_deltas(deltas: &[AccountDelta]) -> Vec<TokenBalanceDelta> {
    deltas
        .iter()
        .filter_map(|delta| {
            let (before, after) = (delta.before.as_ref()?, delta.after.as_ref()?);
            let (program, mint, owner, opening) = read_account(&before.owner, &before.data)?;
            let (after_program, after_mint, _, closing) = read_account(&after.owner, &after.data)?;
            // A token account whose mint or program changed underneath is not a
            // balance movement; it is a different account wearing the same
            // address, and subtracting the two would be meaningless.
            if program != after_program || mint != after_mint {
                return None;
            }
            let mut provenance = delta.provenance.clone();
            provenance.decoder = program.decoder();
            Some(TokenBalanceDelta {
                provenance,
                mint,
                owner,
                before: opening,
                after: closing,
                delta: i128::from(closing) - i128::from(opening),
                token_program: program,
            })
        })
        .collect()
}

/// Supply deltas for every paired account that is a mint on both sides.
pub fn supply_deltas(deltas: &[AccountDelta]) -> Vec<MintSupplyDelta> {
    deltas
        .iter()
        .filter_map(|delta| {
            let (before, after) = (delta.before.as_ref()?, delta.after.as_ref()?);
            let program = TokenProgram::of(&before.owner)?;
            if TokenProgram::of(&after.owner) != Some(program) {
                return None;
            }
            let opening = program.mint_supply(&before.data)?;
            let closing = program.mint_supply(&after.data)?;
            let decimals = program.mint_decimals(&before.data)?;
            let mut provenance = delta.provenance.clone();
            provenance.decoder = program.decoder();
            Some(MintSupplyDelta {
                provenance,
                before: opening,
                after: closing,
                delta: i128::from(closing) - i128::from(opening),
                decimals,
                token_program: program,
            })
        })
        .collect()
}

/// Authority and delegate changes on standard token accounts and mints.
pub fn authority_deltas(deltas: &[AccountDelta]) -> Vec<AuthorityDelta> {
    let mut changes = Vec::new();
    for delta in deltas {
        let (Some(before), Some(after)) = (delta.before.as_ref(), delta.after.as_ref()) else {
            continue;
        };
        let Some(program) = TokenProgram::of(&before.owner) else {
            continue;
        };
        if TokenProgram::of(&after.owner) != Some(program) {
            continue;
        }
        let mut provenance = delta.provenance.clone();
        provenance.decoder = program.decoder();
        let mut push = |field, first: Option<String>, second: Option<String>| {
            if first != second {
                changes.push(AuthorityDelta {
                    provenance: provenance.clone(),
                    field,
                    before: first,
                    after: second,
                });
            }
        };

        let accounts = match program {
            TokenProgram::SplToken => (
                spl_token::decode_account(&before.data).ok(),
                spl_token::decode_account(&after.data).ok(),
            ),
            TokenProgram::Token2022 => (
                token2022::decode_account(&before.data).ok().map(|a| a.base),
                token2022::decode_account(&after.data).ok().map(|a| a.base),
            ),
        };
        if let (Some(opening), Some(closing)) = accounts {
            push(
                AuthorityField::AccountOwner,
                Some(opening.owner.clone()),
                Some(closing.owner.clone()),
            );
            push(AuthorityField::Delegate, opening.delegate, closing.delegate);
            push(
                AuthorityField::CloseAuthority,
                opening.close_authority,
                closing.close_authority,
            );
            continue;
        }

        let mints = match program {
            TokenProgram::SplToken => (
                spl_token::decode_mint(&before.data).ok(),
                spl_token::decode_mint(&after.data).ok(),
            ),
            TokenProgram::Token2022 => (
                token2022::decode_mint(&before.data).ok().map(|m| m.base),
                token2022::decode_mint(&after.data).ok().map(|m| m.base),
            ),
        };
        if let (Some(opening), Some(closing)) = mints {
            push(
                AuthorityField::MintAuthority,
                opening.mint_authority,
                closing.mint_authority,
            );
            push(
                AuthorityField::FreezeAuthority,
                opening.freeze_authority,
                closing.freeze_authority,
            );
            if program == TokenProgram::Token2022 {
                let delegate = |data: &[u8]| -> Option<String> {
                    let list = token2022::extensions(data);
                    let value = list.value(token2022::PERMANENT_DELEGATE)?;
                    match token2022::decode_extension(token2022::PERMANENT_DELEGATE, value) {
                        token2022::Extension::PermanentDelegate { delegate } => delegate,
                        _ => None,
                    }
                };
                push(
                    AuthorityField::PermanentDelegate,
                    delegate(&before.data),
                    delegate(&after.data),
                );
            }
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{
        account::pair,
        tests::{execution, named, snapshot},
    };

    const SPL: &str = spl_token::PROGRAM_ID;
    const T22: &str = token2022::PROGRAM_ID;

    fn token_account(mint: u8, owner: u8, amount: u64) -> Vec<u8> {
        let mut data = vec![0_u8; spl_token::ACCOUNT_LEN];
        data[0..32].copy_from_slice(&[mint; 32]);
        data[32..64].copy_from_slice(&[owner; 32]);
        data[64..72].copy_from_slice(&amount.to_le_bytes());
        data[108] = 1;
        data
    }

    fn mint_account(supply: u64, decimals: u8) -> Vec<u8> {
        let mut data = vec![0_u8; spl_token::MINT_LEN];
        data[36..44].copy_from_slice(&supply.to_le_bytes());
        data[44] = decimals;
        data[45] = 1;
        data
    }

    fn deltas_for(
        owner: &str,
        before: Vec<u8>,
        after: Vec<u8>,
        lamports: (u64, u64),
    ) -> Vec<AccountDelta> {
        let pre = vec![named("a", owner, lamports.0, before)];
        let result = execution(vec![("a", snapshot(owner, lamports.1, after))]);
        pair("r", &pre, &result)
    }

    #[test]
    fn a_token_increase_and_decrease_are_both_measured() {
        let up = balance_deltas(&deltas_for(
            SPL,
            token_account(1, 2, 100),
            token_account(1, 2, 250),
            (1, 1),
        ));
        assert_eq!(up[0].delta, 150);
        assert!(up[0].increased());

        let down = balance_deltas(&deltas_for(
            SPL,
            token_account(1, 2, 250),
            token_account(1, 2, 100),
            (1, 1),
        ));
        assert_eq!(down[0].delta, -150);
        assert!(down[0].decreased());
    }

    #[test]
    fn an_unchanged_balance_is_still_reported_as_a_measurement() {
        let same = balance_deltas(&deltas_for(
            SPL,
            token_account(1, 2, 100),
            token_account(1, 2, 100),
            (1, 1),
        ));
        assert_eq!(same.len(), 1, "a measured zero is a result");
        assert_eq!(same[0].delta, 0);
        assert!(!same[0].increased() && !same[0].decreased());
    }

    /// Subtracting two different assets is meaningless, not merely imprecise.
    #[test]
    fn a_mint_change_under_one_address_is_not_a_balance_delta() {
        let crossed = balance_deltas(&deltas_for(
            SPL,
            token_account(1, 2, 100),
            token_account(9, 2, 100),
            (1, 1),
        ));
        assert!(crossed.is_empty());
    }

    #[test]
    fn a_token_program_change_is_not_a_balance_delta() {
        let pre = vec![named("a", SPL, 1, token_account(1, 2, 100))];
        let result = execution(vec![("a", snapshot(T22, 1, token_account(1, 2, 400)))]);
        assert!(balance_deltas(&pair("r", &pre, &result)).is_empty());
    }

    #[test]
    fn an_owner_change_on_a_token_account_is_an_authority_delta() {
        let deltas = deltas_for(
            SPL,
            token_account(1, 2, 100),
            token_account(1, 7, 100),
            (1, 1),
        );
        let authority = authority_deltas(&deltas);
        assert_eq!(authority.len(), 1);
        assert_eq!(authority[0].field, AuthorityField::AccountOwner);
        assert_eq!(
            authority[0].after,
            Some(bs58::encode([7_u8; 32]).into_string())
        );
        // And the balance is separately unchanged: two facts, not one.
        assert_eq!(balance_deltas(&deltas)[0].delta, 0);
    }

    #[test]
    fn a_delegate_appearing_is_an_authority_delta() {
        let mut after = token_account(1, 2, 100);
        after[72..76].copy_from_slice(&1_u32.to_le_bytes());
        after[76..108].copy_from_slice(&[5_u8; 32]);
        let authority = authority_deltas(&deltas_for(SPL, token_account(1, 2, 100), after, (1, 1)));
        assert!(authority
            .iter()
            .any(|a| a.field == AuthorityField::Delegate && a.before.is_none()));
    }

    #[test]
    fn mint_supply_increases_and_decreases_are_measured() {
        let minted = supply_deltas(&deltas_for(
            SPL,
            mint_account(1_000, 9),
            mint_account(1_500, 9),
            (1, 1),
        ));
        assert_eq!(minted[0].delta, 500);
        assert_eq!(minted[0].decimals, 9);

        let burned = supply_deltas(&deltas_for(
            SPL,
            mint_account(1_500, 9),
            mint_account(1_000, 9),
            (1, 1),
        ));
        assert_eq!(burned[0].delta, -500);
    }

    #[test]
    fn a_mint_authority_change_is_an_authority_delta() {
        let mut after = mint_account(10, 6);
        after[0..4].copy_from_slice(&1_u32.to_le_bytes());
        after[4..36].copy_from_slice(&[3_u8; 32]);
        let authority = authority_deltas(&deltas_for(SPL, mint_account(10, 6), after, (1, 1)));
        assert!(authority
            .iter()
            .any(|a| a.field == AuthorityField::MintAuthority));
    }

    #[test]
    fn a_full_range_decrease_does_not_overflow() {
        let down = balance_deltas(&deltas_for(
            SPL,
            token_account(1, 2, u64::MAX),
            token_account(1, 2, 0),
            (1, 1),
        ));
        assert_eq!(down[0].delta, -i128::from(u64::MAX));

        let up = balance_deltas(&deltas_for(
            SPL,
            token_account(1, 2, 0),
            token_account(1, 2, u64::MAX),
            (1, 1),
        ));
        assert_eq!(up[0].delta, i128::from(u64::MAX));
    }

    #[test]
    fn a_created_token_account_yields_no_balance_delta() {
        let result = execution(vec![("fresh", snapshot(SPL, 1, token_account(1, 2, 500)))]);
        let deltas = pair("r", &[], &result);
        assert!(
            balance_deltas(&deltas).is_empty(),
            "a created account has no before to subtract from"
        );
    }

    #[test]
    fn a_closed_token_account_yields_no_balance_delta() {
        let pre = vec![named("gone", SPL, 1, token_account(1, 2, 500))];
        let deltas = pair("r", &pre, &execution(Vec::new()));
        assert!(balance_deltas(&deltas).is_empty());
    }

    #[test]
    fn token_2022_extended_accounts_are_measured_under_their_own_program() {
        let mut before = token_account(1, 2, 100);
        before.resize(token2022::ACCOUNT_TYPE_OFFSET, 0);
        before.push(2);
        before.extend_from_slice(&token2022::TRANSFER_FEE_AMOUNT.to_le_bytes());
        before.extend_from_slice(&8_u16.to_le_bytes());
        before.extend_from_slice(&50_u64.to_le_bytes());
        let mut after = before.clone();
        after[64..72].copy_from_slice(&400_u64.to_le_bytes());

        let deltas = balance_deltas(&deltas_for(T22, before, after, (1, 1)));
        assert_eq!(deltas[0].delta, 300);
        assert_eq!(deltas[0].token_program, TokenProgram::Token2022);
        assert_eq!(deltas[0].provenance.decoder.name, "token-2022");
    }

    #[test]
    fn the_decoder_that_read_a_delta_is_recorded_on_it() {
        let spl = balance_deltas(&deltas_for(
            SPL,
            token_account(1, 2, 1),
            token_account(1, 2, 2),
            (1, 1),
        ));
        assert_eq!(spl[0].provenance.decoder.name, "spl-token");
        assert_eq!(
            spl[0].provenance.decoder.provenance,
            crate::standard_programs::SchemaProvenance::StandardProgram
        );
    }

    #[test]
    fn a_non_token_account_produces_no_token_evidence() {
        let deltas = deltas_for(
            "11111111111111111111111111111111",
            vec![1, 2, 3],
            vec![1, 2, 4],
            (1, 1),
        );
        assert!(balance_deltas(&deltas).is_empty());
        assert!(supply_deltas(&deltas).is_empty());
        assert!(authority_deltas(&deltas).is_empty());
    }

    /// A malformed token account must not read as a zero balance.
    #[test]
    fn a_malformed_token_account_yields_no_delta_rather_than_zero() {
        let mut broken = token_account(1, 2, 100);
        broken[108] = 9; // an account state outside the enum
        let deltas = balance_deltas(&deltas_for(SPL, token_account(1, 2, 100), broken, (1, 1)));
        assert!(
            deltas.is_empty(),
            "an unreadable post-state is not a balance of zero"
        );
    }
}
