//! Squads V4 accounts, addresses and the canonical message identity.
//!
//! Decoding only: nothing here reads the chain or decides whether a proposal
//! matches a change. Layouts are transcribed from the official source at
//! [`SOURCE_REVISION`] and checked against its generated SDK's discriminators;
//! they are a pinned interface, not a claim that the deployed program was
//! built from that source.
//!
//! Every Squads account is Anchor-serialized: an eight-byte discriminator
//! (`sha256("account:<Name>")[..8]`), then Borsh. `Vec<T>` is a `u32` length
//! and its items, `Option<T>` one tag byte, an enum one variant byte.

use anyhow::{bail, ensure, Context, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_address::Address;

use crate::change::SquadsV4Delivery;

/// The mainnet (and devnet) Squads V4 program. The only deployment this
/// decoder is pinned against.
pub const SQUADS_V4_PROGRAM_ID: &str = "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf";
pub const SOURCE_REPOSITORY: &str = "https://github.com/Squads-Protocol/v4";
/// `main` when G1 was written; `programs/squads_multisig_program` 2.1.0.
pub const SOURCE_REVISION: &str = "af94153ff77a28b6effe46b9c94baaa93742b48c";
pub const PROGRAM_CRATE_VERSION: &str = "2.1.0";
/// Names the transcription below. Bumped whenever a layout or a check changes
/// what a decoded account means.
pub const DECODER: &str = "eplyx-squads-v4-decoder-v1";
/// Domain of [`message_hash`]. Part of every delivery's identity.
pub const MESSAGE_HASH_DOMAIN: &str = "eplyx-squads-v4-vault-message-v1";

// `state/seeds.rs`.
const SEED_PREFIX: &[u8] = b"multisig";
const SEED_MULTISIG: &[u8] = b"multisig";
const SEED_PROPOSAL: &[u8] = b"proposal";
const SEED_TRANSACTION: &[u8] = b"transaction";
const SEED_VAULT: &[u8] = b"vault";

pub const MULTISIG_DISCRIMINATOR: [u8; 8] = [224, 116, 121, 186, 68, 161, 79, 236];
pub const VAULT_TRANSACTION_DISCRIMINATOR: [u8; 8] = [168, 250, 162, 100, 81, 14, 162, 207];
pub const PROPOSAL_DISCRIMINATOR: [u8; 8] = [26, 94, 189, 187, 116, 136, 53, 33];

pub fn program() -> Address {
    SQUADS_V4_PROGRAM_ID.parse().expect("valid program id")
}

fn parse(value: &str, what: &str) -> Result<Address> {
    let address: Address = value
        .parse()
        .map_err(|_| anyhow::anyhow!("{what} {value:?} is not a base58 address"))?;
    ensure!(
        address.to_string() == value,
        "{what} {value:?} is not canonically encoded"
    );
    Ok(address)
}

// ------------------------------------------------------------------ addresses

/// `[SEED_PREFIX, SEED_MULTISIG, create_key]`.
pub fn multisig_address(create_key: &Address) -> (Address, u8) {
    Address::find_program_address(
        &[SEED_PREFIX, SEED_MULTISIG, create_key.as_ref()],
        &program(),
    )
}

/// `[SEED_PREFIX, multisig, SEED_VAULT, vault_index]`.
pub fn vault_address(multisig: &Address, vault_index: u8) -> (Address, u8) {
    Address::find_program_address(
        &[SEED_PREFIX, multisig.as_ref(), SEED_VAULT, &[vault_index]],
        &program(),
    )
}

/// The address a vault transaction actually signs as: the vault seeds with the
/// bump the transaction *stored*, not a re-derived canonical one. Squads
/// executes with `create_program_address(.., [vault_bump])`, so a stored bump
/// that is not canonical would sign as some other address.
pub fn vault_signer(multisig: &Address, vault_index: u8, vault_bump: u8) -> Option<Address> {
    Address::create_program_address(
        &[
            SEED_PREFIX,
            multisig.as_ref(),
            SEED_VAULT,
            &[vault_index],
            &[vault_bump],
        ],
        &program(),
    )
    .ok()
}

/// `[SEED_PREFIX, multisig, SEED_TRANSACTION, index_le]`.
pub fn transaction_address(multisig: &Address, transaction_index: u64) -> (Address, u8) {
    Address::find_program_address(
        &[
            SEED_PREFIX,
            multisig.as_ref(),
            SEED_TRANSACTION,
            &transaction_index.to_le_bytes(),
        ],
        &program(),
    )
}

/// `[SEED_PREFIX, multisig, SEED_TRANSACTION, index_le, SEED_PROPOSAL]`.
pub fn proposal_address(multisig: &Address, transaction_index: u64) -> (Address, u8) {
    Address::find_program_address(
        &[
            SEED_PREFIX,
            multisig.as_ref(),
            SEED_TRANSACTION,
            &transaction_index.to_le_bytes(),
            SEED_PROPOSAL,
        ],
        &program(),
    )
}

/// The delivery a `(multisig, vault_index, transaction_index)` names, with the
/// message hash supplied by whoever read the message.
pub fn derive_delivery(
    multisig: &Address,
    vault_index: u8,
    transaction_index: u64,
    message_sha256: String,
) -> SquadsV4Delivery {
    SquadsV4Delivery {
        squads_program_id: SQUADS_V4_PROGRAM_ID.into(),
        multisig: multisig.to_string(),
        vault_index,
        vault: vault_address(multisig, vault_index).0.to_string(),
        transaction_index,
        transaction: transaction_address(multisig, transaction_index)
            .0
            .to_string(),
        proposal: proposal_address(multisig, transaction_index).0.to_string(),
        message_sha256,
    }
}

/// A delivery is well-formed only if every address in it is exactly the
/// derivation from its multisig and indexes. Nothing a user typed is trusted
/// as an address.
pub fn validate_delivery(delivery: &SquadsV4Delivery) -> Result<()> {
    ensure!(
        delivery.squads_program_id == SQUADS_V4_PROGRAM_ID,
        "Squads program {} is not the Squads V4 program this build decodes ({SQUADS_V4_PROGRAM_ID})",
        delivery.squads_program_id
    );
    let multisig = parse(&delivery.multisig, "multisig")?;
    for (value, what) in [
        (&delivery.vault, "vault"),
        (&delivery.transaction, "vault transaction"),
        (&delivery.proposal, "proposal"),
    ] {
        parse(value, what)?;
    }
    ensure!(
        delivery.transaction_index > 0,
        "Squads transaction indexes start at 1"
    );
    ensure!(
        is_sha256_hex(&delivery.message_sha256),
        "message_sha256 must be 64 lowercase hex characters"
    );
    let derived = derive_delivery(
        &multisig,
        delivery.vault_index,
        delivery.transaction_index,
        delivery.message_sha256.clone(),
    );
    ensure!(
        derived.vault == delivery.vault,
        "vault {} is not vault {} of multisig {} (derived {})",
        delivery.vault,
        delivery.vault_index,
        delivery.multisig,
        derived.vault
    );
    ensure!(
        derived.transaction == delivery.transaction,
        "vault transaction {} is not transaction {} of multisig {} (derived {})",
        delivery.transaction,
        delivery.transaction_index,
        delivery.multisig,
        derived.transaction
    );
    ensure!(
        derived.proposal == delivery.proposal,
        "proposal {} is not the proposal of transaction {} of multisig {} (derived {})",
        delivery.proposal,
        delivery.transaction_index,
        delivery.multisig,
        derived.proposal
    );
    Ok(())
}

pub(crate) fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ------------------------------------------------------------------- accounts

/// `state/multisig.rs`. Allocated with slack (`rent_collector` always
/// reserves 32 bytes), so a zero tail is normal.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Multisig {
    pub create_key: [u8; 32],
    pub config_authority: [u8; 32],
    pub threshold: u16,
    pub time_lock: u32,
    pub transaction_index: u64,
    pub stale_transaction_index: u64,
    pub rent_collector: Option<[u8; 32]>,
    pub bump: u8,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Member {
    pub key: [u8; 32],
    pub permissions: u8,
}

/// `state/vault_transaction.rs`. Sized exactly at creation.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct VaultTransaction {
    pub multisig: [u8; 32],
    pub creator: [u8; 32],
    pub index: u64,
    pub bump: u8,
    pub vault_index: u8,
    pub vault_bump: u8,
    pub ephemeral_signer_bumps: Vec<u8>,
    pub message: VaultTransactionMessage,
}

/// The message Squads executes on the vault's behalf, exactly as stored.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct VaultTransactionMessage {
    pub num_signers: u8,
    pub num_writable_signers: u8,
    pub num_writable_non_signers: u8,
    pub account_keys: Vec<[u8; 32]>,
    pub instructions: Vec<CompiledInstruction>,
    pub address_table_lookups: Vec<AddressTableLookup>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct CompiledInstruction {
    pub program_id_index: u8,
    pub account_indexes: Vec<u8>,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct AddressTableLookup {
    pub account_key: [u8; 32],
    pub writable_indexes: Vec<u8>,
    pub readonly_indexes: Vec<u8>,
}

impl VaultTransactionMessage {
    /// Squads' own `is_static_writable_index`.
    pub fn is_static_writable(&self, index: usize) -> bool {
        let signers = usize::from(self.num_signers);
        if index >= self.account_keys.len() {
            return false;
        }
        if index < usize::from(self.num_writable_signers) {
            return true;
        }
        index >= signers && index - signers < usize::from(self.num_writable_non_signers)
    }

    pub fn is_signer(&self, index: usize) -> bool {
        index < usize::from(self.num_signers)
    }

    pub fn key(&self, index: usize) -> Option<Address> {
        self.account_keys.get(index).map(|k| Address::from(*k))
    }
}

/// `state/proposal.rs`. Sized for the member count, so a zero tail is normal.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Proposal {
    pub multisig: [u8; 32],
    pub transaction_index: u64,
    pub status: ProposalStatus,
    pub bump: u8,
    pub approved: Vec<[u8; 32]>,
    pub rejected: Vec<[u8; 32]>,
    pub cancelled: Vec<[u8; 32]>,
}

/// Variant order is the wire format.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum ProposalStatus {
    Draft {
        timestamp: i64,
    },
    Active {
        timestamp: i64,
    },
    Rejected {
        timestamp: i64,
    },
    Approved {
        timestamp: i64,
    },
    /// Deprecated upstream; transient within one transaction.
    Executing,
    Executed {
        timestamp: i64,
    },
    Cancelled {
        timestamp: i64,
    },
}

/// Proposal status as an observation. Never part of a change's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatusKind {
    Draft,
    Active,
    Rejected,
    Approved,
    Executing,
    Executed,
    Cancelled,
}

impl ProposalStatusKind {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Rejected | Self::Executed | Self::Cancelled)
    }
}

impl ProposalStatus {
    pub fn kind(&self) -> ProposalStatusKind {
        match self {
            Self::Draft { .. } => ProposalStatusKind::Draft,
            Self::Active { .. } => ProposalStatusKind::Active,
            Self::Rejected { .. } => ProposalStatusKind::Rejected,
            Self::Approved { .. } => ProposalStatusKind::Approved,
            Self::Executing => ProposalStatusKind::Executing,
            Self::Executed { .. } => ProposalStatusKind::Executed,
            Self::Cancelled { .. } => ProposalStatusKind::Cancelled,
        }
    }

    pub fn timestamp(&self) -> Option<i64> {
        match self {
            Self::Draft { timestamp }
            | Self::Active { timestamp }
            | Self::Rejected { timestamp }
            | Self::Approved { timestamp }
            | Self::Executed { timestamp }
            | Self::Cancelled { timestamp } => Some(*timestamp),
            Self::Executing => None,
        }
    }
}

/// Read one Anchor account: discriminator, Borsh body, and a tail that is
/// either absent or allocation slack. A non-zero tail is bytes this layout
/// does not explain, and is refused rather than ignored.
fn anchor<T: BorshDeserialize>(data: &[u8], discriminator: [u8; 8], what: &str) -> Result<T> {
    ensure!(
        data.len() >= 8 && data[..8] == discriminator,
        "{what} does not carry the Squads V4 {what} discriminator"
    );
    let mut body = &data[8..];
    let value = T::deserialize(&mut body).with_context(|| format!("{what} is not decodable"))?;
    ensure!(
        body.iter().all(|b| *b == 0),
        "{what} has {} unexplained trailing bytes",
        body.len()
    );
    Ok(value)
}

pub fn decode_multisig(data: &[u8]) -> Result<Multisig> {
    anchor(data, MULTISIG_DISCRIMINATOR, "Multisig")
}

pub fn decode_proposal(data: &[u8]) -> Result<Proposal> {
    anchor(data, PROPOSAL_DISCRIMINATOR, "Proposal")
}

/// A vault transaction, and the exact bytes its message occupied.
///
/// The message bytes are re-serialized from the decoded value and required to
/// equal the stored slice, so the canonical form hashed below *is* the stored
/// form, not a re-rendering that two different stored messages could share.
pub fn decode_vault_transaction(data: &[u8]) -> Result<(VaultTransaction, Vec<u8>)> {
    let transaction: VaultTransaction =
        anchor(data, VAULT_TRANSACTION_DISCRIMINATOR, "VaultTransaction")?;
    let message = borsh::to_vec(&transaction.message)?;
    let start = 8 + 32 + 32 + 8 + 3 + 4 + transaction.ephemeral_signer_bumps.len();
    let stored = data
        .get(start..start + message.len())
        .context("VaultTransaction message is truncated")?;
    if stored != message.as_slice() {
        bail!("VaultTransaction message does not re-encode to its stored bytes");
    }
    Ok((transaction, message))
}

/// The canonical identity of a stored Squads message.
///
/// ```text
/// sha256( "eplyx-squads-v4-vault-message-v1" || 0x00 || borsh(VaultTransactionMessage) )
/// ```
///
/// The Borsh encoding is the one Squads stores: signer and writable counts,
/// every static key in order, every instruction's program index, account
/// indexes and data, and every lookup's table and index lists, each list
/// length-prefixed so no two messages share an encoding. Nothing rendered, no
/// memo (Squads logs the memo and never stores it) and no status.
pub fn message_hash(message: &VaultTransactionMessage) -> Result<String> {
    Ok(message_hash_of_bytes(&borsh::to_vec(message)?))
}

pub fn message_hash_of_bytes(encoded_message: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MESSAGE_HASH_DOMAIN.as_bytes());
    hasher.update([0u8]);
    hasher.update(encoded_message);
    crate::hexfmt::encode(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MULTISIG: &str = "7Z4hZjf1FswanPWW53pAVnS9ApCLXaT1dPupMEHDAKax";

    #[test]
    fn discriminators_are_the_anchor_convention() {
        for (name, pinned) in [
            ("Multisig", MULTISIG_DISCRIMINATOR),
            ("VaultTransaction", VAULT_TRANSACTION_DISCRIMINATOR),
            ("Proposal", PROPOSAL_DISCRIMINATOR),
        ] {
            let digest = Sha256::digest(format!("account:{name}").as_bytes());
            assert_eq!(digest[..8], pinned, "{name}");
        }
    }

    #[test]
    fn a_delivery_must_be_its_own_derivation() {
        let multisig: Address = MULTISIG.parse().unwrap();
        let good = derive_delivery(&multisig, 0, 42, "ab".repeat(32));
        validate_delivery(&good).unwrap();

        let mut wrong_transaction = good.clone();
        wrong_transaction.transaction =
            derive_delivery(&multisig, 0, 43, String::new()).transaction;
        assert!(validate_delivery(&wrong_transaction).is_err());

        let mut wrong_proposal = good.clone();
        wrong_proposal.proposal = derive_delivery(&multisig, 0, 41, String::new()).proposal;
        assert!(validate_delivery(&wrong_proposal).is_err());

        let mut wrong_vault = good.clone();
        wrong_vault.vault_index = 1;
        assert!(validate_delivery(&wrong_vault).is_err());

        let mut other_program = good.clone();
        other_program.squads_program_id = "GyhGAqjokLwF9UXdQ2dR5Zwiup242j4mX4J1tSMKyAmD".into();
        assert!(validate_delivery(&other_program).is_err());

        let mut index_zero = derive_delivery(&multisig, 0, 0, "ab".repeat(32));
        index_zero.transaction_index = 0;
        assert!(validate_delivery(&index_zero).is_err());

        let mut bad_hash = good;
        bad_hash.message_sha256 = "AB".repeat(32);
        assert!(validate_delivery(&bad_hash).is_err());
    }
}
