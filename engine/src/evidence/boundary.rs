//! Proving archived snapshots against validator-observed metadata.
//!
//! This was the most duplicated method in the two adapters — 77% line-identical
//! at commit `029811d`, and 248 lines in one of them — and almost none of it was
//! protocol knowledge. "Does the archive's lamport balance for this account
//! equal the validator's, at this account's index in the message?" is a question
//! about Solana, not about stake pools.
//!
//! ## Core proves *how*; the adapter says *what*
//!
//! The split is deliberate and the adapter keeps the half that can be wrong in
//! a protocol-specific way:
//!
//! - which token program a balance must be attributed to. Proving a balance
//!   against the wrong program proves nothing, and the two adapters genuinely
//!   require different ones.
//! - which accounts are exempt from read-only byte identity. Stake Pool exempts
//!   Clock and StakeHistory because its `WithdrawSol` names them and the runtime
//!   rewrites them every slot. Token-2022 exempts nothing, and must not: no
//!   instruction in its contract reads a sysvar, so one appearing there is a
//!   real anomaly. A prover that exempted sysvars unconditionally would quietly
//!   weaken the narrower contract.
//! - how a token account's amount and mint are read, which differs between the
//!   legacy 165-byte layout and Token-2022's extended one.
//! - any protocol accounting identity that corroborates state the validator
//!   does not observe. The stake pool's `total_lamports` against the reserve's
//!   observed change is the strongest evidence in that adapter and is
//!   irreducibly its own.
//!
//! ## Absent evidence is not absence of a problem
//!
//! Every check here fails closed. A snapshot whose address is not in the
//! message is an error, not a skip. An account the validator recorded a token
//! balance for that does not decode is an error, not an unproved account. A
//! proof that established nothing at all is an error, not a pass.

use crate::{
    ingest::transactions::{HistoricalTransaction, TokenBalance},
    types::NamedAccount,
};
use anyhow::{Context, Result};

/// Which side of the transaction a check is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// State at the end of slot S-1, which must be this transaction's input.
    Pre,
    /// State at the end of slot S, which must be this transaction's output.
    Post,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pre => "pre",
            Self::Post => "post",
        }
    }

    /// The slot boundary this side is read at.
    pub fn boundary(self) -> &'static str {
        match self {
            Self::Pre => "S-1",
            Self::Post => "S",
        }
    }

    /// How a mismatch on this side is named.
    ///
    /// Pre-state and post-state failures mean different things — one says the
    /// input was wrong, the other says the output cannot serve as a fidelity
    /// reference — so they are reported differently.
    pub fn mismatch_headline(self) -> &'static str {
        match self {
            Self::Pre => "slot-before state is not this transaction's pre-state",
            Self::Post => "slot-end state is not this transaction's post-state",
        }
    }
}

/// What one protocol requires of a boundary proof.
///
/// Everything here is a thing the generic prover genuinely cannot know. Nothing
/// here is a formula: an adapter supplies facts and readers, and the prover
/// performs every check.
pub struct BoundaryContract {
    /// The token program every validator-observed token balance must name.
    pub token_program: &'static str,
    /// How that program is named in a failure. "Token-2022"; "the SPL Token
    /// program".
    pub token_program_description: &'static str,
    /// How a token account is described when it fails to decode. "token
    /// account"; "base-layout token account".
    pub token_account_description: &'static str,
    /// Read a token account's base-unit amount under this protocol's rules.
    pub account_amount: fn(&[u8]) -> Option<u64>,
    /// Read a token account's mint under this protocol's rules.
    pub account_mint: fn(&[u8]) -> Option<String>,
    /// Addresses exempt from the read-only byte-identity check.
    ///
    /// Empty for a contract admitting no instruction that reads runtime-written
    /// state. An exemption is a hole in the proof and has to be asked for.
    pub read_only_exempt: &'static [&'static str],
    /// The sentence appended to a lamport mismatch, explaining what it means.
    pub interference_hint: fn(Side, u64) -> String,
}

/// What a generic proof established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryProof {
    /// Token accounts whose pre-state amount matched validator metadata.
    pub proved_token_accounts: usize,
    /// The universal assumptions, in the order they are reported. An adapter
    /// appends its own; it does not rewrite these.
    pub assumptions: Vec<String>,
}

/// Find an address's position among the message's account keys.
///
/// A snapshot the message does not name is an error. It cannot be checked
/// against anything, and carrying it forward would put an unproved account
/// inside an exactness claim.
pub fn index_of(transaction: &HistoricalTransaction, address: &str) -> Result<usize> {
    transaction
        .account_keys
        .iter()
        .position(|key| key.address == address)
        .with_context(|| format!("snapshot account {address} is not in the message"))
}

/// The validator's token balance for one message index, if it recorded one.
pub fn balance_at(balances: Option<&Vec<TokenBalance>>, index: usize) -> Option<&TokenBalance> {
    balances?
        .iter()
        .find(|balance| balance.account_index == index)
}

/// Prove both boundaries against validator-observed metadata.
///
/// Checks, in order, for each side and each snapshot:
///
/// 1. the snapshot's address is in the message;
/// 2. its lamports equal the validator's balance at that index;
/// 3. where the validator recorded a token balance, the snapshot decodes as a
///    token account, its amount equals the recorded one, the recorded program
///    is the contract's, and its mint equals the recorded mint.
///
/// Then, across the boundary: every read-only account not exempted is
/// byte-identical, and at least one token account was proved.
pub fn prove(
    contract: &BoundaryContract,
    transaction: &HistoricalTransaction,
    pre: &[NamedAccount],
    post: &[NamedAccount],
) -> Result<BoundaryProof> {
    let pre_balances = transaction
        .pre_balances
        .as_ref()
        .context("transaction metadata omitted pre-balances")?;
    let post_balances = transaction
        .post_balances
        .as_ref()
        .context("transaction metadata omitted post-balances")?;
    let mut proved_token_accounts = 0_usize;

    for (side, snapshots, lamports, token_balances) in [
        (
            Side::Pre,
            pre,
            pre_balances,
            transaction.pre_token_balances.as_ref(),
        ),
        (
            Side::Post,
            post,
            post_balances,
            transaction.post_token_balances.as_ref(),
        ),
    ] {
        for named in snapshots {
            let index = index_of(transaction, &named.address)?;
            // A mismatch here is almost always same-slot interference rather
            // than a bad archive: the archive answers with the state at the
            // *end* of slot S, so another transaction touching the same account
            // in that slot moves it away from this transaction's boundary.
            anyhow::ensure!(
                lamports.get(index) == Some(&named.account.lamports),
                "{} for {}: archive reports {} lamports at the {} boundary, validator \
                 metadata records {}. {}",
                side.mismatch_headline(),
                named.address,
                named.account.lamports,
                side.boundary(),
                lamports
                    .get(index)
                    .map(u64::to_string)
                    .unwrap_or_else(|| "nothing".into()),
                (contract.interference_hint)(side, transaction.slot)
            );

            let Some(balance) = balance_at(token_balances, index) else {
                continue;
            };
            // The validator recorded an exact base-unit amount for this
            // account. Anything the archive returned has to match it, which is
            // what rejects a snapshot taken on the wrong side of a same-slot
            // write.
            let decoded = (contract.account_amount)(&named.account.data).with_context(|| {
                format!(
                    "account {} has a token balance but does not decode as a {}",
                    named.address, contract.token_account_description
                )
            })?;
            anyhow::ensure!(
                decoded == balance.amount,
                "{}-state archive amount {decoded} differs from validator-observed {} for {}",
                side.as_str(),
                balance.amount,
                named.address
            );
            anyhow::ensure!(
                balance.program_id == contract.token_program,
                "account {} is owned by token program {}, not {}",
                named.address,
                balance.program_id,
                contract.token_program_description
            );
            anyhow::ensure!(
                (contract.account_mint)(&named.account.data).as_deref() == Some(&balance.mint),
                "account {} decodes to a different mint than the validator recorded",
                named.address
            );
            if side == Side::Pre {
                proved_token_accounts += 1;
            }
        }
    }

    prove_read_only_identity(contract, transaction, pre, post)?;

    anyhow::ensure!(
        proved_token_accounts > 0,
        "no token account balance could be proved against validator metadata"
    );

    Ok(BoundaryProof {
        proved_token_accounts,
        assumptions: vec![
            format!(
                "{proved_token_accounts} token account balance(s) at S-1 and S match the \
                 validator-observed pre/post token balances"
            ),
            "every snapshot's lamports match validator-observed pre/post balances".into(),
        ],
    })
}

/// Every read-only account is byte-identical across the boundary.
///
/// Validator metadata records balances, not data, so this is the only available
/// proof for an account the transaction does not write. It is stated as an
/// assumption rather than claimed as independent evidence.
fn prove_read_only_identity(
    contract: &BoundaryContract,
    transaction: &HistoricalTransaction,
    pre: &[NamedAccount],
    post: &[NamedAccount],
) -> Result<()> {
    for named in pre {
        let index = index_of(transaction, &named.address)?;
        if transaction.account_keys[index].is_writable {
            continue;
        }
        if contract.read_only_exempt.contains(&named.address.as_str()) {
            continue;
        }
        let after = post
            .iter()
            .find(|other| other.address == named.address)
            .context("read-only account missing from post-state")?;
        anyhow::ensure!(
            named.account.data == after.account.data && named.account.owner == after.account.owner,
            "read-only account {} changed across the transaction boundary",
            named.address
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ingest::transactions::HistoricalTransaction,
        standard_programs::spl_token,
        types::{AccountMetaSpec, AccountSnapshot},
    };

    const TOKEN: &str = spl_token::PROGRAM_ID;

    fn contract() -> BoundaryContract {
        BoundaryContract {
            token_program: TOKEN,
            token_program_description: "the SPL Token program",
            token_account_description: "base-layout token account",
            account_amount: spl_token::account_amount,
            account_mint: spl_token::account_mint,
            read_only_exempt: &[],
            interference_hint: |side, slot| {
                format!(
                    "Another transaction in slot {slot} wrote this account {} this one.",
                    if side == Side::Pre { "before" } else { "after" }
                )
            },
        }
    }

    fn token_bytes(mint: u8, amount: u64) -> Vec<u8> {
        let mut data = vec![0_u8; spl_token::ACCOUNT_LEN];
        data[0..32].copy_from_slice(&[mint; 32]);
        data[32..64].copy_from_slice(&[9_u8; 32]);
        data[64..72].copy_from_slice(&amount.to_le_bytes());
        data[108] = 1;
        data
    }

    fn mint_address(mint: u8) -> String {
        bs58::encode([mint; 32]).into_string()
    }

    fn named(address: &str, lamports: u64, owner: &str, data: Vec<u8>) -> NamedAccount {
        NamedAccount {
            label: address.into(),
            address: address.into(),
            account: AccountSnapshot {
                lamports,
                owner: owner.into(),
                data,
                executable: false,
                rent_epoch: 0,
            },
        }
    }

    /// A transaction with two accounts: a writable token account, and a
    /// read-only one.
    fn transaction(writable: bool) -> HistoricalTransaction {
        let mut tx: HistoricalTransaction = serde_json::from_str(
            r#"{
                "signature":"s","slot":100,"block_time":null,"version":"legacy",
                "recent_blockhash":"b","payer":"holder","account_keys":[],
                "instructions":[],"inner_instructions":[],"success":true,
                "error":null,"fee":5000,"compute_units":null,"logs":[]
            }"#,
        )
        .expect("fixture transaction");
        tx.account_keys = vec![
            AccountMetaSpec {
                address: "holder".into(),
                is_signer: true,
                is_writable: true,
            },
            AccountMetaSpec {
                address: "reference".into(),
                is_signer: false,
                is_writable: writable,
            },
        ];
        tx.pre_balances = Some(vec![2_039_280, 1_000_000]);
        tx.post_balances = Some(vec![2_039_280, 1_000_000]);
        tx.pre_token_balances = Some(vec![TokenBalance {
            account_index: 0,
            mint: mint_address(1),
            program_id: TOKEN.into(),
            amount: 500,
            decimals: 6,
        }]);
        tx.post_token_balances = Some(vec![TokenBalance {
            account_index: 0,
            mint: mint_address(1),
            program_id: TOKEN.into(),
            amount: 300,
            decimals: 6,
        }]);
        tx
    }

    fn sides() -> (Vec<NamedAccount>, Vec<NamedAccount>) {
        (
            vec![
                named("holder", 2_039_280, TOKEN, token_bytes(1, 500)),
                named("reference", 1_000_000, TOKEN, vec![7; 16]),
            ],
            vec![
                named("holder", 2_039_280, TOKEN, token_bytes(1, 300)),
                named("reference", 1_000_000, TOKEN, vec![7; 16]),
            ],
        )
    }

    #[test]
    fn an_exact_boundary_proves() {
        let (pre, post) = sides();
        let proof = prove(&contract(), &transaction(false), &pre, &post).expect("proves");
        assert_eq!(proof.proved_token_accounts, 1);
        assert!(proof.assumptions[0].contains("1 token account balance(s)"));
        assert!(proof.assumptions[1].contains("lamports match validator-observed"));
    }

    #[test]
    fn a_lamport_mismatch_names_the_side_and_the_account() {
        let (mut pre, post) = sides();
        pre[0].account.lamports = 999;
        let error = prove(&contract(), &transaction(false), &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(
            error.contains("slot-before state is not this transaction's pre-state"),
            "{error}"
        );
        assert!(error.contains("holder"), "{error}");
        assert!(
            error.contains("wrote this account before this one"),
            "{error}"
        );
    }

    #[test]
    fn a_post_side_lamport_mismatch_is_named_differently() {
        let (pre, mut post) = sides();
        post[0].account.lamports = 1;
        let error = prove(&contract(), &transaction(false), &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(
            error.contains("slot-end state is not this transaction's post-state"),
            "{error}"
        );
        assert!(
            error.contains("wrote this account after this one"),
            "{error}"
        );
    }

    #[test]
    fn a_token_amount_that_contradicts_the_validator_is_refused() {
        let (mut pre, post) = sides();
        pre[0].account.data = token_bytes(1, 501);
        let error = prove(&contract(), &transaction(false), &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(error.contains("archive amount 501"), "{error}");
        assert!(error.contains("validator-observed 500"), "{error}");
    }

    #[test]
    fn a_mint_that_contradicts_the_validator_is_refused() {
        let (mut pre, post) = sides();
        pre[0].account.data = token_bytes(2, 500);
        let error = prove(&contract(), &transaction(false), &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(error.contains("decodes to a different mint"), "{error}");
    }

    #[test]
    fn a_balance_recorded_under_another_token_program_is_refused() {
        let (pre, post) = sides();
        let mut tx = transaction(false);
        tx.pre_token_balances.as_mut().unwrap()[0].program_id =
            crate::standard_programs::token2022::PROGRAM_ID.into();
        let error = prove(&contract(), &tx, &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(error.contains("not the SPL Token program"), "{error}");
    }

    #[test]
    fn a_balance_on_an_account_that_does_not_decode_is_refused() {
        let (mut pre, post) = sides();
        pre[0].account.data = vec![0; 64];
        let error = prove(&contract(), &transaction(false), &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(
            error.contains("does not decode as a base-layout token account"),
            "{error}"
        );
    }

    #[test]
    fn a_read_only_account_that_changed_is_refused() {
        let (pre, mut post) = sides();
        post[1].account.data = vec![8; 16];
        let error = prove(&contract(), &transaction(false), &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(
            error.contains("read-only account reference changed across the transaction boundary"),
            "{error}"
        );
    }

    /// The owner half of the same check. A read-only account whose owner
    /// changed is as much a boundary failure as one whose data did.
    #[test]
    fn a_read_only_account_whose_owner_changed_is_refused() {
        let (pre, mut post) = sides();
        post[1].account.owner = "11111111111111111111111111111111".into();
        assert!(prove(&contract(), &transaction(false), &pre, &post).is_err());
    }

    #[test]
    fn a_writable_account_may_change() {
        let (pre, mut post) = sides();
        post[1].account.data = vec![8; 16];
        // Marked writable, so byte identity is not expected of it.
        prove(&contract(), &transaction(true), &pre, &post).expect("proves");
    }

    /// Exemption is opt-in. A contract that asks for none gets none, which is
    /// what keeps the Token-2022 guarantee from widening by accident.
    #[test]
    fn exemption_applies_only_where_the_contract_asks_for_it() {
        let (pre, mut post) = sides();
        post[1].account.data = vec![8; 16];
        assert!(prove(&contract(), &transaction(false), &pre, &post).is_err());

        let exempting = BoundaryContract {
            read_only_exempt: &["reference"],
            ..contract()
        };
        prove(&exempting, &transaction(false), &pre, &post).expect("exempted");
    }

    #[test]
    fn a_snapshot_the_message_does_not_name_is_an_error() {
        let (mut pre, post) = sides();
        pre.push(named("stranger", 1, TOKEN, Vec::new()));
        let error = prove(&contract(), &transaction(false), &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(error.contains("is not in the message"), "{error}");
    }

    #[test]
    fn a_read_only_account_absent_from_the_post_state_is_an_error() {
        let (pre, mut post) = sides();
        post.retain(|named| named.address != "reference");
        let error = prove(&contract(), &transaction(false), &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(error.contains("missing from post-state"), "{error}");
    }

    /// A proof that established nothing is a failure, not a pass.
    #[test]
    fn proving_no_token_account_at_all_is_refused() {
        let mut tx = transaction(false);
        tx.pre_token_balances = Some(Vec::new());
        tx.post_token_balances = Some(Vec::new());
        let (pre, post) = sides();
        let error = prove(&contract(), &tx, &pre, &post)
            .expect_err("must refuse")
            .to_string();
        assert!(
            error.contains("no token account balance could be proved"),
            "{error}"
        );
    }

    #[test]
    fn missing_validator_balances_are_an_error_not_an_empty_proof() {
        let mut tx = transaction(false);
        tx.pre_balances = None;
        let (pre, post) = sides();
        assert!(prove(&contract(), &tx, &pre, &post)
            .expect_err("must refuse")
            .to_string()
            .contains("omitted pre-balances"));
    }
}
