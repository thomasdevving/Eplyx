//! An in-memory Squads V4 + loader-v3 world, answering JSON-RPC.
//!
//! Test and fixture support only. It exists so the real verifier — the same
//! reads, decoders and checks that run against mainnet — can be driven through
//! every mutation a binding must catch, deterministically and offline. It is
//! never a source of evidence: nothing outside tests and committed fixtures
//! constructs one.
//!
//! Accounts are encoded independently of the decoders: Borsh through the
//! derive on the transcribed layouts, loader state through the official
//! interface, discriminators as pinned bytes.

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::Result;
use base64::Engine;
use serde_json::{json, Value};
use solana_address::Address;

use super::squads::{self, *};
use crate::change::{ChangeSpec, Delivery};
use crate::ingest::rpc::RpcProvider;
use crate::standard_programs::upgradeable_loader::{self as loader, encode};
use crate::types::AccountSnapshot;

pub const CANDIDATE_ELF: &[u8] = b"\x7fELF\x02\x01\x01 analysed candidate program v2";
pub const DEPLOYED_ELF: &[u8] = b"\x7fELF\x02\x01\x01 deployed program v1";

fn key(seed: u8) -> Address {
    Address::from([seed; 32])
}

/// The fixed cast. Arbitrary but stable, so fixtures and IDs never move.
pub fn create_key() -> Address {
    key(11)
}
pub fn program() -> Address {
    key(21)
}
pub fn buffer() -> Address {
    key(22)
}
pub fn spill() -> Address {
    key(23)
}
pub fn creator() -> Address {
    key(24)
}
pub fn member(n: u8) -> Address {
    key(30 + n)
}
pub fn outsider() -> Address {
    key(40)
}
pub fn multisig() -> Address {
    squads::multisig_address(&create_key()).0
}
pub fn vault() -> Address {
    squads::vault_address(&multisig(), 0).0
}
pub const TRANSACTION_INDEX: u64 = 42;

/// The message a Squads client compiles for one upgrade with the vault as
/// payer: `[vault | ProgramData, Program, Buffer, spill | Rent, Clock, loader]`.
pub fn upgrade_message(
    vault: Address,
    program: Address,
    buffer: Address,
    spill: Address,
) -> VaultTransactionMessage {
    VaultTransactionMessage {
        num_signers: 1,
        num_writable_signers: 1,
        num_writable_non_signers: 4,
        account_keys: [
            vault,
            loader::programdata_address(&program),
            program,
            buffer,
            spill,
            loader::rent_sysvar(),
            loader::clock_sysvar(),
            loader::id(),
        ]
        .iter()
        .map(Address::to_bytes)
        .collect(),
        instructions: vec![CompiledInstruction {
            program_id_index: 7,
            account_indexes: vec![1, 2, 3, 4, 5, 6, 0],
            data: encode::upgrade(),
        }],
        address_table_lookups: vec![],
    }
}

pub fn anchor_account<T: borsh::BorshSerialize>(
    discriminator: [u8; 8],
    value: &T,
    slack: usize,
) -> Vec<u8> {
    let mut data = discriminator.to_vec();
    data.extend(borsh::to_vec(value).expect("borsh"));
    data.resize(data.len() + slack, 0);
    data
}

fn owned(owner: Address, data: Vec<u8>, executable: bool) -> AccountSnapshot {
    AccountSnapshot {
        lamports: 1_000_000 + data.len() as u64,
        owner: owner.to_string(),
        data,
        executable,
        rent_epoch: u64::MAX,
    }
}

/// The world's editable state. Each field is the decoded form; `accounts()`
/// encodes them, so a test mutates meaning and the bytes follow.
#[derive(Clone, Debug)]
pub struct State {
    /// The program the proposal upgrades, and the bytes its analysed spec names.
    pub program: Address,
    pub candidate: Vec<u8>,
    pub multisig: Multisig,
    pub transactions: BTreeMap<u64, VaultTransaction>,
    pub proposals: BTreeMap<u64, Proposal>,
    pub program_programdata: Address,
    pub programdata_authority: Option<Address>,
    pub deployed: Vec<u8>,
    pub buffer_authority: Option<Address>,
    pub buffer_bytes: Vec<u8>,
    pub buffer_exists: bool,
    /// Raw overrides, applied last: an account replaced or removed outright.
    pub overrides: BTreeMap<Address, Option<AccountSnapshot>>,
    pub slot: u64,
    pub fail: bool,
}

impl State {
    pub fn transaction(&mut self) -> &mut VaultTransaction {
        self.transactions
            .get_mut(&TRANSACTION_INDEX)
            .expect("the proposal transaction")
    }

    pub fn message(&mut self) -> &mut VaultTransactionMessage {
        &mut self.transaction().message
    }

    pub fn proposal(&mut self) -> &mut Proposal {
        self.proposals
            .get_mut(&TRANSACTION_INDEX)
            .expect("the proposal")
    }

    /// Add another transaction and proposal at `index` of the same multisig.
    pub fn add_transaction(&mut self, index: u64, message: VaultTransactionMessage) {
        let (_, bump) = squads::transaction_address(&multisig(), index);
        let (_, vault_bump) = squads::vault_address(&multisig(), 0);
        self.transactions.insert(
            index,
            VaultTransaction {
                multisig: multisig().to_bytes(),
                creator: creator().to_bytes(),
                index,
                bump,
                vault_index: 0,
                vault_bump,
                ephemeral_signer_bumps: vec![],
                message,
            },
        );
        let (_, proposal_bump) = squads::proposal_address(&multisig(), index);
        self.proposals.insert(
            index,
            Proposal {
                multisig: multisig().to_bytes(),
                transaction_index: index,
                status: ProposalStatus::Active {
                    timestamp: 1_790_000_000,
                },
                bump: proposal_bump,
                approved: vec![member(1).to_bytes()],
                rejected: vec![],
                cancelled: vec![],
            },
        );
        self.multisig.transaction_index = self.multisig.transaction_index.max(index);
    }

    pub fn accounts(&self) -> BTreeMap<Address, Option<AccountSnapshot>> {
        let squads_program = squads::program();
        let mut accounts = BTreeMap::new();
        accounts.insert(
            multisig(),
            Some(owned(
                squads_program,
                anchor_account(MULTISIG_DISCRIMINATOR, &self.multisig, 64),
                false,
            )),
        );
        for (index, transaction) in &self.transactions {
            accounts.insert(
                squads::transaction_address(&multisig(), *index).0,
                Some(owned(
                    squads_program,
                    anchor_account(VAULT_TRANSACTION_DISCRIMINATOR, transaction, 0),
                    false,
                )),
            );
        }
        for (index, proposal) in &self.proposals {
            accounts.insert(
                squads::proposal_address(&multisig(), *index).0,
                Some(owned(
                    squads_program,
                    anchor_account(PROPOSAL_DISCRIMINATOR, proposal, 96),
                    false,
                )),
            );
        }
        accounts.insert(
            self.program,
            Some(owned(
                loader::id(),
                encode::program(&self.program_programdata),
                true,
            )),
        );
        accounts.insert(
            loader::programdata_address(&self.program),
            Some(owned(
                loader::id(),
                encode::programdata(7_000, self.programdata_authority, &self.deployed),
                false,
            )),
        );
        if self.buffer_exists {
            accounts.insert(
                buffer(),
                Some(owned(
                    loader::id(),
                    encode::buffer(self.buffer_authority, &self.buffer_bytes),
                    false,
                )),
            );
        }
        for (address, account) in &self.overrides {
            accounts.insert(*address, account.clone());
        }
        accounts
    }
}

/// A chain that answers `getMultipleAccounts` from [`State`], advancing one
/// slot per read.
pub struct World {
    state: Mutex<State>,
    reads: Mutex<Vec<Vec<String>>>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// A healthy Active proposal: exactly one Upgrade of [`program`] from
    /// [`buffer`] holding [`CANDIDATE_ELF`], every authority the vault.
    pub fn new() -> Self {
        Self::upgrading(program(), CANDIDATE_ELF)
    }

    /// The same healthy proposal, upgrading `program` to `candidate`.
    pub fn upgrading(program: Address, candidate: &[u8]) -> Self {
        let (_, bump) = squads::multisig_address(&create_key());
        let mut state = State {
            program,
            candidate: candidate.to_vec(),
            multisig: Multisig {
                create_key: create_key().to_bytes(),
                config_authority: [0; 32],
                threshold: 2,
                time_lock: 0,
                transaction_index: 0,
                stale_transaction_index: 0,
                rent_collector: None,
                bump,
                members: (1..=3)
                    .map(|n| Member {
                        key: member(n).to_bytes(),
                        permissions: 7,
                    })
                    .collect(),
            },
            transactions: BTreeMap::new(),
            proposals: BTreeMap::new(),
            program_programdata: loader::programdata_address(&program),
            programdata_authority: Some(vault()),
            deployed: DEPLOYED_ELF.to_vec(),
            buffer_authority: Some(vault()),
            buffer_bytes: candidate.to_vec(),
            buffer_exists: true,
            overrides: BTreeMap::new(),
            slot: 310_000_000,
            fail: false,
        };
        state.add_transaction(
            TRANSACTION_INDEX,
            upgrade_message(vault(), program, buffer(), spill()),
        );
        Self {
            state: Mutex::new(state),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub fn edit(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.lock().expect("state"));
    }

    pub fn state(&self) -> State {
        self.state.lock().expect("state").clone()
    }

    /// Every key list read, in order.
    pub fn reads(&self) -> Vec<Vec<String>> {
        self.reads.lock().expect("reads").clone()
    }

    /// The analysed change the default world matches: the minimal upgrade spec.
    pub fn analysed_spec() -> ChangeSpec {
        ChangeSpec::program_upgrade(&program().to_string(), CANDIDATE_ELF)
    }

    /// The analysed change this world matches.
    pub fn spec(&self) -> ChangeSpec {
        let state = self.state();
        ChangeSpec::program_upgrade(&state.program.to_string(), &state.candidate)
    }

    /// The analysed spec bound to this world's proposal as it stands.
    pub fn bound_spec(&self) -> ChangeSpec {
        let mut state = self.state();
        let message = state.message().clone();
        self.spec()
            .with_delivery(Some(Delivery::SquadsV4(squads::derive_delivery(
                &multisig(),
                0,
                TRANSACTION_INDEX,
                squads::message_hash(&message).expect("hash"),
            ))))
    }
}

fn encode_account(account: &AccountSnapshot) -> Value {
    json!({
        "data": [base64::prelude::BASE64_STANDARD.encode(&account.data), "base64"],
        "owner": account.owner,
        "lamports": account.lamports,
        "executable": account.executable,
        "rentEpoch": account.rent_epoch,
        "space": account.data.len(),
    })
}

impl RpcProvider for World {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        anyhow::ensure!(
            matches!(method, "getMultipleAccounts" | "getAccountInfo"),
            "unexpected RPC {method}"
        );
        let mut state = self.state.lock().expect("state");
        anyhow::ensure!(
            !state.fail,
            "RPC transport failed for {method} (endpoint redacted)"
        );
        state.slot += 1;
        let accounts = state.accounts();
        let keys: Vec<String> = if method == "getMultipleAccounts" {
            let keys: Vec<String> = params[0]
                .as_array()
                .expect("key list")
                .iter()
                .map(|k| k.as_str().expect("key").to_string())
                .collect();
            self.reads.lock().expect("reads").push(keys.clone());
            keys
        } else {
            vec![params[0].as_str().expect("account key").to_string()]
        };
        let values: Vec<Value> = keys
            .iter()
            .map(|k| {
                let address: Address = k.parse().expect("address");
                match accounts.get(&address).cloned().flatten() {
                    Some(account) => encode_account(&account),
                    None => Value::Null,
                }
            })
            .collect();
        if method == "getAccountInfo" {
            Ok(json!({"context": {"slot": state.slot}, "value": values[0]}))
        } else {
            Ok(json!({"context": {"slot": state.slot}, "value": values}))
        }
    }
}
