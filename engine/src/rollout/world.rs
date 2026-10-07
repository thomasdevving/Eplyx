//! The bounded rollout world: byte-bearing account states, content-addressed,
//! and one fresh-VM execution of one supported message over such a state.
//!
//! A state is the complete declared closure. A missing key is unknown, never
//! known absence: `KnownAbsent` is a positive claim carried from retained
//! evidence or produced by an execution (for example a loader Buffer closed to
//! zero lamports). Account bytes live once in an `AccountStore` keyed by their
//! canonical digest, so the 1 MiB ProgramData is stored once per distinct value
//! rather than once per state.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, ensure, Context, Result};
use litesvm::LiteSVM;
use serde::{Deserialize, Serialize};
use solana_account::Account;
use solana_address::Address;
use solana_message::Message;
use solana_transaction::Transaction;

use crate::{
    canonical,
    executor::LoadedProgram,
    replay::ReplayClock,
    types::AccountSnapshot,
    universal::execution::{InnerGroup, InnerInstruction},
};

pub const STATE_VERSION: &str = "eplyx-rollout-state-v1";
pub const EXECUTION_VERSION: &str = "eplyx-rollout-execution-v1";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "existence", deny_unknown_fields)]
pub enum Entry {
    Present { account_sha256: String },
    KnownAbsent,
}

/// One world: the Clock it executes under and every declared account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: String,
    pub clock: ReplayClock,
    pub accounts: BTreeMap<String, Entry>,
}

pub type AccountStore = BTreeMap<String, AccountSnapshot>;

pub fn account_id(account: &AccountSnapshot) -> Result<String> {
    canonical::digest(account)
}

impl State {
    pub fn id(&self) -> Result<String> {
        canonical::digest(self)
    }

    /// `Ok(None)` is known absence; an address outside the closure is an error.
    pub fn account<'a>(
        &self,
        store: &'a AccountStore,
        address: &str,
    ) -> Result<Option<&'a AccountSnapshot>> {
        match self.accounts.get(address) {
            Some(Entry::Present { account_sha256 }) => {
                Ok(Some(store.get(account_sha256).with_context(|| {
                    format!("account bytes {account_sha256} not retained")
                })?))
            }
            Some(Entry::KnownAbsent) => Ok(None),
            None => anyhow::bail!("account {address} is outside the declared rollout closure"),
        }
    }

    /// A copy with one closure account replaced. Used to build S0 and by tests
    /// that need a deliberately different world; never by step handoff.
    pub fn with_account(
        &self,
        store: &mut AccountStore,
        address: &str,
        account: Option<AccountSnapshot>,
    ) -> Result<Self> {
        ensure!(
            self.accounts.contains_key(address),
            "account {address} is outside the declared rollout closure"
        );
        let mut next = self.clone();
        next.accounts.insert(address.into(), put(store, account)?);
        Ok(next)
    }

    pub fn closure(&self) -> BTreeSet<String> {
        self.accounts.keys().cloned().collect()
    }
}

pub fn put(store: &mut AccountStore, account: Option<AccountSnapshot>) -> Result<Entry> {
    Ok(match account {
        Some(account) => {
            let account_sha256 = account_id(&account)?;
            store.insert(account_sha256.clone(), account);
            Entry::Present { account_sha256 }
        }
        None => Entry::KnownAbsent,
    })
}

/// Everything one VM execution produced over the closure. Post-state rows use
/// the same Present/KnownAbsent semantics as states; their bytes are in the
/// account store under `post`'s digests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    pub version: String,
    pub before_state_id: String,
    pub message: crate::path::ProbeMessage,
    pub clock: ReplayClock,
    pub success: bool,
    pub error: Option<String>,
    #[serde(with = "crate::numfmt::u64_string")]
    pub compute_units: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub transaction_fee_lamports: u64,
    pub logs: Vec<String>,
    pub inner_instructions: Vec<InnerGroup>,
    pub post: BTreeMap<String, Entry>,
    /// Accounts outside the closure whose bytes, envelope or existence changed
    /// in the VM's complete account census. Must be empty for any accepted step.
    pub outside_writes: BTreeSet<String>,
}

impl Execution {
    pub fn id(&self) -> Result<String> {
        canonical::digest(self)
    }
}

fn to_account(snapshot: &AccountSnapshot) -> Result<Account> {
    Ok(Account {
        lamports: snapshot.lamports,
        data: snapshot.data.clone(),
        owner: snapshot.owner.parse()?,
        executable: snapshot.executable,
        rent_epoch: snapshot.rent_epoch,
    })
}

fn from_account(account: &Account) -> AccountSnapshot {
    AccountSnapshot {
        lamports: account.lamports,
        owner: account.owner.to_string(),
        data: account.data.clone(),
        executable: account.executable,
        rent_epoch: account.rent_epoch,
    }
}

fn census(svm: &LiteSVM) -> BTreeMap<String, AccountSnapshot> {
    svm.accounts_db()
        .inner
        .iter()
        .map(|(address, account)| {
            (
                address.to_string(),
                from_account(&Account::from(account.clone())),
            )
        })
        .collect()
}

/// A fresh LiteSVM holding exactly `state`: Clock first, pinned dependency
/// programs, then every Present account (ProgramData before the executable
/// Program header, so the loader relationship is read from seeded bytes).
/// The seeded closure is read back and must equal the state byte for byte.
pub fn restore(
    state: &State,
    store: &AccountStore,
    dependencies: &[LoadedProgram],
) -> Result<LiteSVM> {
    let mut svm = LiteSVM::new()
        .with_sigverify(false)
        .with_blockhash_check(false);
    let c = &state.clock;
    svm.set_sysvar(&solana_clock::Clock {
        slot: c.slot,
        epoch_start_timestamp: c.epoch_start_timestamp,
        epoch: c.epoch,
        leader_schedule_epoch: c.leader_schedule_epoch,
        unix_timestamp: c.unix_timestamp,
    });
    for program in dependencies {
        ensure!(
            !state.accounts.contains_key(&program.program_id.to_string()),
            "a pinned dependency is also a world account"
        );
        svm.add_program_with_loader(program.program_id, &program.bytes, program.loader)
            .map_err(|e| anyhow!("load pinned dependency {}: {e:?}", program.program_id))?;
    }
    for executable in [false, true] {
        for (address, entry) in &state.accounts {
            if let Entry::Present { account_sha256 } = entry {
                let account = store
                    .get(account_sha256)
                    .context("seeded account bytes not retained")?;
                if account.executable == executable {
                    svm.set_account(address.parse()?, to_account(account)?)
                        .map_err(|e| anyhow!("seed world account {address}: {e:?}"))?;
                }
            }
        }
    }
    for (address, entry) in &state.accounts {
        let seeded = svm.get_account(&address.parse::<Address>()?);
        match entry {
            Entry::Present { account_sha256 } => ensure!(
                seeded.as_ref().map(from_account).as_ref() == store.get(account_sha256),
                "restored account {address} differs from its state bytes"
            ),
            Entry::KnownAbsent => ensure!(
                seeded.is_none(),
                "known-absent account {address} exists after restore"
            ),
        }
    }
    Ok(svm)
}

/// Execute one message over a restored state. Signature possession and
/// blockhash age are explicit local assumptions; signer privileges, loader and
/// program account checks execute. The VM is discarded afterwards: the only
/// thing carried forward is the byte-bearing post-state.
pub(crate) fn execute(
    state: &State,
    store: &mut AccountStore,
    dependencies: &[LoadedProgram],
    message: &Message,
) -> Result<Execution> {
    let mut svm = restore(state, store, dependencies)?;
    let before = census(&svm);
    let (success, error, meta) =
        match svm.send_transaction(Transaction::new_unsigned(message.clone())) {
            Ok(meta) => (true, None, meta),
            Err(failure) => (false, Some(format!("{:?}", failure.err)), failure.meta),
        };
    let after = census(&svm);
    let closure = state.closure();
    let outside_writes = before
        .keys()
        .chain(after.keys())
        .filter(|k| !closure.contains(*k) && before.get(*k) != after.get(*k))
        .cloned()
        .collect();
    let mut post = BTreeMap::new();
    for address in &closure {
        let account = svm
            .get_account(&address.parse::<Address>()?)
            .as_ref()
            .map(from_account);
        post.insert(address.clone(), put(store, account)?);
    }
    let inner_instructions = meta
        .inner_instructions
        .iter()
        .enumerate()
        .map(|(outer_index, group)| InnerGroup {
            outer_index,
            instructions: group
                .iter()
                .map(|inner| InnerInstruction {
                    program_id_index: inner.instruction.program_id_index,
                    accounts: inner.instruction.accounts.clone(),
                    data: inner.instruction.data.clone(),
                    stack_height: inner.stack_height,
                })
                .collect(),
        })
        .collect();
    Ok(Execution {
        version: EXECUTION_VERSION.into(),
        before_state_id: state.id()?,
        message: crate::path::ProbeMessage::from(message),
        clock: state.clock.clone(),
        success,
        error,
        compute_units: meta.compute_units_consumed,
        transaction_fee_lamports: meta.fee,
        logs: meta.logs,
        inner_instructions,
        post,
        outside_writes,
    })
}

/// The state an execution hands to the next step: identical Clock, closure
/// rows exactly as the execution left them.
pub fn after(before: &State, execution: &Execution) -> Result<State> {
    ensure!(
        execution.before_state_id == before.id()?
            && execution.post.keys().eq(before.accounts.keys()),
        "execution does not bind to this state and closure"
    );
    Ok(State {
        version: STATE_VERSION.into(),
        clock: before.clock.clone(),
        accounts: execution.post.clone(),
    })
}

/// Addresses whose closure rows differ between two states.
pub fn changed(before: &State, after: &State) -> BTreeSet<String> {
    before
        .accounts
        .iter()
        .filter(|(k, v)| after.accounts.get(*k) != Some(v))
        .map(|(k, _)| k.clone())
        .chain(
            after
                .accounts
                .keys()
                .filter(|k| !before.accounts.contains_key(*k))
                .cloned(),
        )
        .collect()
}
