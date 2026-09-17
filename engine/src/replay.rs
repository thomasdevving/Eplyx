//! Durable, offline replay and fidelity gate. RPC is never called here.
use crate::{
    dependencies::{DependencyManifest, ProgramSource},
    executor::{execute_in_environment, CpiCall, ExecutionResult, LoadedProgram, ProgramVersion},
    ingest::transactions::{CpiFrame, HistoricalTransaction},
    protocol::{self, EconomicObservation, ProtocolAdapter},
    screening::SlotScreening,
    types::{AccountSnapshot, Category, Fixture, InstructionSpec, NamedAccount},
    versions::{ProgramLoader, LEGACY_BPF_LOADER_ID, UPGRADEABLE_LOADER_ID},
    Report,
};
use std::fmt;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_clock::Clock;
use solana_message::{compiled_instruction::CompiledInstruction, Message, MessageHeader};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const REPLAY_SCHEMA: u32 = 1;
pub const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";
const SYSTEM_TRANSFER_DISCRIMINANT: u32 = 2;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayStateSource {
    ControlledSnapshot,
    Reconstructed,
    CurrentApproximation,
    HistoricalArchive,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayFidelity {
    Exact,
    Matched,
    Mismatch,
    Unknown,
    Approximate,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayClock {
    pub slot: u64,
    pub epoch_start_timestamp: i64,
    pub epoch: u64,
    pub leader_schedule_epoch: u64,
    pub unix_timestamp: i64,
}
impl ReplayClock {
    fn clock(&self) -> Clock {
        Clock {
            slot: self.slot,
            epoch_start_timestamp: self.epoch_start_timestamp,
            epoch: self.epoch,
            leader_schedule_epoch: self.leader_schedule_epoch,
            unix_timestamp: self.unix_timestamp,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalExecution {
    pub success: bool,
    pub fee: u64,
    pub post_state_hash: String,
    /// Per-account post-state, recorded so a failed gate can name the account
    /// that differs instead of only two hashes. Digests rather than bytes: the
    /// point is to localize a mismatch, not to double the size of every record.
    /// Empty in records written before this existed, which keeps them loadable
    /// and simply falls back to the hash-only message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_accounts: Vec<PostAccountDigest>,
    /// The invocation graph the validator recorded, in execution order.
    ///
    /// For a transaction with no CPI this is empty and the gate is unchanged
    /// from Phase 7. For one with CPI it is independent evidence of what ran:
    /// a replay can reproduce a post-state by accident far more easily than it
    /// can reproduce the same sequence of calls at the same depths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cpi_invocations: Vec<CpiFrame>,
}

/// A lamport credit the local runtime refuses but mainnet accepted.
///
/// Solana forbids an account that is below its rent-exempt minimum from being
/// credited while staying below it: `post_lamports` must not exceed
/// `pre_lamports`. Mainnet accepted the transactions this describes, and the
/// replay runtime rejects them, so the divergence is in runtime semantics
/// rather than in the reconstructed state. Recorded as its own reason so it
/// never again surfaces as an unexplained post-state mismatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RentPayingCredit {
    pub address: String,
    pub label: String,
    pub owner: String,
    pub pre_lamports: u64,
    pub post_lamports: u64,
    pub credited: u64,
    pub rent_exempt_minimum: u64,
}

impl fmt::Display for RentPayingCredit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}) owned by {}: {} -> {} lamports (credited {}), rent-exempt minimum {}",
            self.label,
            self.address,
            self.owner,
            self.pre_lamports,
            self.post_lamports,
            self.credited,
            self.rent_exempt_minimum
        )
    }
}

/// One account's post-state, reduced to what identifies a difference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostAccountDigest {
    pub label: String,
    pub address: String,
    pub owner: String,
    pub lamports: u64,
    pub data_len: u64,
    pub data_sha256: String,
}

impl PostAccountDigest {
    pub fn of(named: &NamedAccount) -> Self {
        Self {
            label: named.label.clone(),
            address: named.address.clone(),
            owner: named.account.owner.clone(),
            lamports: named.account.lamports,
            data_len: named.account.data.len() as u64,
            data_sha256: hash_bytes(&named.account.data),
        }
    }

    /// Human-readable differences against a replayed account, empty when they
    /// agree on everything this digest records.
    pub fn differences(&self, replayed: &AccountSnapshot) -> Vec<String> {
        let mut out = Vec::new();
        if self.owner != replayed.owner {
            out.push(format!("owner {} -> {}", self.owner, replayed.owner));
        }
        if self.lamports != replayed.lamports {
            let delta = replayed.lamports as i128 - self.lamports as i128;
            out.push(format!(
                "lamports {} -> {} ({delta:+})",
                self.lamports, replayed.lamports
            ));
        }
        if self.data_len != replayed.data.len() as u64 {
            out.push(format!(
                "data length {} -> {}",
                self.data_len,
                replayed.data.len()
            ));
        } else if self.data_sha256 != hash_bytes(&replayed.data) {
            out.push(format!("data differs ({} bytes)", self.data_len));
        }
        out
    }
}

/// How an account came to be in the required set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountDiscovery {
    /// Named by a top-level instruction's account metas.
    InstructionMeta,
    /// A message key no top-level instruction names. Legacy messages carry every
    /// key a CPI can reach, so these are exactly the accounts a top-level-only
    /// view would miss.
    MessageKey,
    /// Named by an inner instruction in validator metadata.
    InnerInstruction,
    /// Named by pool state the adapter decoded.
    AdapterDependency,
}

/// Where one account's replayed bytes came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStateSource {
    /// Read from a slot-addressable archive at the transaction's boundary.
    HistoricalArchive,
    /// Held nothing at either boundary, which the validator's balances confirm.
    /// The runtime materializes it, exactly as it did originally.
    AbsentAtBothBoundaries,
    /// Boundary established from the transaction's own `preBalances` and
    /// `postBalances` rather than from an archive snapshot.
    ///
    /// Admissible only for an account whose entire state *is* its lamport
    /// balance - System-owned, non-executable, zero-length data - because for
    /// such an account the validator's per-transaction balances are exact
    /// evidence of its boundary, and nothing else about it can have changed.
    /// This is what lets a transaction survive a same-slot write to a shared
    /// fee payer: the archive snapshot is ambiguous, the transaction metadata
    /// is not.
    TransactionBalanceMetadata,
}

/// Whether an account's boundary can rest on transaction balance metadata.
///
/// Deliberately narrow. Any data at all disqualifies it, which excludes nonce
/// accounts (System-owned but 80 bytes), token accounts, mints, stake accounts
/// and every program account. A non-System owner disqualifies it, which
/// excludes PDAs, sysvars and the Jito tip accounts. Both boundaries must agree,
/// so an account that another transaction allocated or reassigned mid-slot is
/// rejected rather than reconstructed.
pub fn reconstructable_from_balances(pre: &AccountSnapshot, post: &AccountSnapshot) -> bool {
    [pre, post].into_iter().all(|account| {
        account.owner == SYSTEM_PROGRAM_ID && !account.executable && account.data.is_empty()
    })
}

/// Provenance for one acquired account.
///
/// The bytes themselves live in [`ReplayRecord::accounts`]. This records how
/// they were obtained, so a reader can tell a snapshot proved at an exact slot
/// from one that was inferred, without re-running the acquisition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountAcquisition {
    pub address: String,
    pub label: String,
    pub discovered_by: Vec<AccountDiscovery>,
    pub source: AccountStateSource,
    /// Slot the pre-state was read at: the transaction's predecessor.
    pub context_slot: u64,
    pub method: String,
}

/// One invocation, normalized so the original and the replay compare directly.
pub fn cpi_graph(calls: &[CpiCall]) -> Vec<CpiFrame> {
    calls
        .iter()
        .map(|call| CpiFrame {
            outer_index: call.outer_index,
            stack_height: call.stack_height,
            program: call.program.clone(),
            account_count: call.account_count,
            data_len: call.data_len,
            discriminant: call.discriminant,
        })
        .collect()
}

/// Render an invocation graph as a tree, outer instruction by outer instruction.
pub fn render_cpi_graph(root: &str, frames: &[CpiFrame]) -> String {
    let mut text = format!(
        "{root}
"
    );
    for (position, frame) in frames.iter().enumerate() {
        let last = position + 1 == frames.len();
        text.push_str(&format!(
            "{} {} (instruction {}, depth {}, {} accounts, {} bytes{})
",
            if last { "└─" } else { "├─" },
            frame.program,
            frame.outer_index,
            frame.stack_height,
            frame.account_count,
            frame.data_len,
            frame
                .discriminant
                .map(|value| format!(", discriminant {value}"))
                .unwrap_or_default(),
        ));
    }
    if frames.is_empty() {
        text.push_str(
            "└─ (no cross-program invocation)
",
        );
    }
    text
}

/// Dependency binaries for one replay, checked against a record's manifest.
///
/// Bytes are addressed by program ID and verified by hash, so a bundle cannot
/// quietly pair one program's artefact with another's manifest entry, and a
/// stale or substituted binary is an error rather than a different replay.
#[derive(Clone, Debug, Default)]
pub struct DependencyBundle {
    programs: Vec<LoadedProgram>,
}

fn loader_address(loader: ProgramLoader) -> &'static str {
    match loader {
        ProgramLoader::Legacy => LEGACY_BPF_LOADER_ID,
        ProgramLoader::Upgradeable => UPGRADEABLE_LOADER_ID,
    }
}

impl DependencyBundle {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn programs(&self) -> &[LoadedProgram] {
        &self.programs
    }

    /// Load every artefact a manifest requires from `directory`.
    pub fn load(manifest: &DependencyManifest, directory: &Path) -> Result<Self> {
        let mut programs = Vec::new();
        for dependency in manifest.loadable() {
            let path = directory.join(dependency.artifact_file_name());
            let bytes = std::fs::read(&path).with_context(|| {
                format!(
                    "reading dependency artefact {} for program {}
                     run `eplyx historical acquire` to rebuild the replay bundle",
                    path.display(),
                    dependency.program_id
                )
            })?;
            let digest = hash_bytes(&bytes);
            anyhow::ensure!(
                Some(&digest) == dependency.binary_sha256.as_ref(),
                "dependency {} at {} hashes to {digest}, but the record pins {}",
                dependency.program_id,
                path.display(),
                dependency
                    .binary_sha256
                    .clone()
                    .unwrap_or_else(|| "nothing".into())
            );
            programs.push(LoadedProgram {
                program_id: dependency.program_id.parse()?,
                loader: dependency
                    .loader
                    .map(loader_address)
                    .unwrap_or(UPGRADEABLE_LOADER_ID)
                    .parse()?,
                bytes,
            });
        }
        Ok(Self { programs })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayRecord {
    pub schema_version: u32,
    pub id: String,
    pub program_id: String,
    pub genesis_hash: String,
    pub transaction: HistoricalTransaction,
    pub accounts: Vec<NamedAccount>,
    pub clock: ReplayClock,
    pub state_source: ReplayStateSource,
    pub pre_state_hash: String,
    pub original: Option<OriginalExecution>,
    pub current_program_sha256: String,
    /// Every executable program this replay needs, pinned to the deployment
    /// live at the transaction's slot. Empty for records written before CPI
    /// replay existed, which by construction needed nothing but their own
    /// program and the runtime.
    #[serde(default, skip_serializing_if = "manifest_is_empty")]
    pub dependencies: DependencyManifest,
    /// Provenance for each required account.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acquisitions: Vec<AccountAcquisition>,
    /// Evidence that no other transaction in the slot made a boundary ambiguous.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_screening: Option<SlotScreening>,
    pub assumptions: Vec<String>,
}

fn manifest_is_empty(manifest: &DependencyManifest) -> bool {
    manifest.programs.is_empty()
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    crate::hexfmt::encode(&Sha256::digest(bytes))
}
/// Accounts the runtime writes on its own schedule rather than the program
/// under test writing them.
///
/// Their *pre*-state is real input and stays under `state_hash`. Their
/// post-state is not evidence about the candidate: the Clock advances every
/// slot no matter what executed, so hashing it into the fidelity gate asks a
/// replay to reproduce mainnet's wall clock, which nothing can do. Listed
/// explicitly rather than matched on the `Sysvar` prefix, because a ground
/// vanity address could otherwise exclude itself from the gate.
const RUNTIME_MANAGED: [&str; 8] = [
    "SysvarC1ock11111111111111111111111111111111",
    "SysvarStakeHistory1111111111111111111111111",
    "SysvarRent111111111111111111111111111111111",
    "SysvarS1otHashes111111111111111111111111111",
    "SysvarRecentB1ockHashes11111111111111111111",
    "SysvarEpochSchedu1e111111111111111111111111",
    "SysvarEpochRewards1111111111111111111111111",
    "SysvarLastRestartS1ot1111111111111111111111",
];

pub fn is_runtime_managed(address: &str) -> bool {
    RUNTIME_MANAGED.contains(&address)
}

/// Hash of the accounts a replay is answerable for.
///
/// Identical to [`state_hash`] except that runtime-managed accounts are
/// dropped. Used for the post-state fidelity comparison on both sides - the
/// original and the replay - so the two are compared over the same set by
/// construction. A record naming no such account hashes exactly as before,
/// which is why existing corpora are unaffected.
pub fn outcome_hash(accounts: &[NamedAccount]) -> Result<String> {
    let retained: Vec<NamedAccount> = accounts
        .iter()
        .filter(|a| !is_runtime_managed(&a.address))
        .cloned()
        .collect();
    state_hash(&retained)
}

/// Canonical binary encoding with explicit lengths, sorted by address. Labels
/// are presentation only. Hash includes rent_epoch as well as required fields.
pub fn state_hash(accounts: &[NamedAccount]) -> Result<String> {
    let mut sorted: Vec<_> = accounts.iter().collect();
    sorted.sort_by(|a, b| a.address.cmp(&b.address));
    anyhow::ensure!(
        sorted.windows(2).all(|a| a[0].address != a[1].address),
        "duplicate account address"
    );
    let mut bytes = b"replay-account-state-v1\0".to_vec();
    for a in sorted {
        let key: solana_address::Address = a.address.parse()?;
        let owner: solana_address::Address = a.account.owner.parse()?;
        bytes.extend_from_slice(key.as_ref());
        bytes.extend_from_slice(owner.as_ref());
        bytes.extend_from_slice(&a.account.lamports.to_le_bytes());
        bytes.push(u8::from(a.account.executable));
        bytes.extend_from_slice(&a.account.rent_epoch.to_le_bytes());
        bytes.extend_from_slice(&(a.account.data.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&a.account.data);
    }
    Ok(hash_bytes(&bytes))
}
impl ReplayRecord {
    fn is_fixture_lending(&self) -> bool {
        self.program_id == crate::fixture_program_id().to_string()
    }

    fn is_memo_transfer(&self) -> bool {
        self.program_id == MEMO_PROGRAM_ID
    }

    /// The adapter owning this record's program, if one is compiled in.
    ///
    /// The fixture protocol and the bounded Memo path predate the seam and keep
    /// their inline contracts; everything added from Phase 7 onward arrives as
    /// an adapter instead of another branch here.
    pub fn adapter(&self) -> Option<&'static dyn ProtocolAdapter> {
        protocol::adapter_for(&self.program_id)
    }

    /// The instruction this record exists to replay.
    ///
    /// A transaction may be preceded by compute-budget instructions, which must
    /// be executed for fidelity but are not what the comparison is about.
    fn primary_instruction(&self) -> &InstructionSpec {
        self.transaction
            .instructions
            .iter()
            .find(|instruction| instruction.program == self.program_id)
            .unwrap_or(&self.transaction.instructions[0])
    }

    /// Native value with protocol meaning for the deliberately narrow mainnet
    /// adapter: one System Program transfer followed by one Memo instruction.
    pub fn native_transfer_lamports(&self) -> Option<u64> {
        if !self.is_memo_transfer() || self.transaction.instructions.len() != 2 {
            return None;
        }
        let instruction = &self.transaction.instructions[0];
        if instruction.program != SYSTEM_PROGRAM_ID || instruction.data.len() != 12 {
            return None;
        }
        let discriminant = u32::from_le_bytes(instruction.data[..4].try_into().ok()?);
        (discriminant == SYSTEM_TRANSFER_DISCRIMINANT).then(|| {
            u64::from_le_bytes(instruction.data[4..12].try_into().expect("length checked"))
        })
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.schema_version == REPLAY_SCHEMA,
            "unsupported replay schema"
        );
        anyhow::ensure!(
            self.is_fixture_lending() || self.is_memo_transfer() || self.adapter().is_some(),
            "unsupported replay execution contract"
        );
        anyhow::ensure!(
            self.transaction.success && self.transaction.error.is_none(),
            "replay currently selects successfully captured original transactions"
        );
        if let Some(original) = &self.original {
            anyhow::ensure!(
                original.success == self.transaction.success
                    && original.fee == self.transaction.fee,
                "original evidence differs from transaction metadata"
            );
        }
        // A v0 message that resolved no lookup tables has exactly the static
        // key list, and `message()` rebuilds both it and a legacy message into
        // the same legacy `Message`, so the two execute identically. One that
        // did resolve tables is still refused.
        crate::protocol::require_executable_message(&self.transaction)?;
        // A known runtime-semantics divergence, reported as itself rather than
        // as an unexplained post-state mismatch several stages later. Phase 9
        // does not emulate validator rent behaviour; it names the gap.
        let credits = self.rent_paying_credits();
        if !credits.is_empty() {
            anyhow::bail!(
                "unsupported_runtime_semantics: rent_paying_account_credited. Mainnet accepted \
                 this transaction, but the replay runtime refuses to credit an account that \
                 stays below its rent-exempt minimum, so V1 cannot reproduce the original \
                 outcome:\n  - {}",
                credits
                    .iter()
                    .map(RentPayingCredit::to_string)
                    .collect::<Vec<_>>()
                    .join("\n  - ")
            );
        }
        if let Some(adapter) = self.adapter() {
            adapter.accept(&self.transaction)?;
        } else if self.is_fixture_lending() {
            anyhow::ensure!(
                self.transaction.instructions.len() == 1
                    && self.transaction.instructions[0].program == self.program_id,
                "fixture replay requires one top-level controlled-program instruction"
            );
        } else {
            anyhow::ensure!(
                self.native_transfer_lamports()
                    .is_some_and(|amount| amount > 0),
                "mainnet Memo replay requires one System transfer followed by one Memo"
            );
            // A malformed record can name fewer keys than the contract needs.
            // Indexing it would abort the process; a record that does not meet
            // the contract is simply not replayable, and says so.
            anyhow::ensure!(
                self.transaction.account_keys.len() >= 2,
                "mainnet Memo replay requires at least two account keys, found {}",
                self.transaction.account_keys.len()
            );
            let system = &self.transaction.instructions[0];
            let memo = &self.transaction.instructions[1];
            anyhow::ensure!(
                system.accounts.len() == 2
                    && system.accounts[0].address == self.transaction.payer
                    && system.accounts[1].address == self.transaction.account_keys[1].address
                    && memo.program == self.program_id
                    && memo.accounts.is_empty(),
                "unsupported System-transfer/Memo instruction shape"
            );
            anyhow::ensure!(
                self.transaction.account_keys.len() == 4
                    && self.transaction.account_keys[2].address == SYSTEM_PROGRAM_ID
                    && self.transaction.account_keys[3].address == self.program_id,
                "mainnet Memo replay requires exactly payer, recipient, System, and Memo keys"
            );
            let pre = self
                .transaction
                .pre_balances
                .as_ref()
                .context("historical Memo replay requires validator pre-balances")?;
            let post = self
                .transaction
                .post_balances
                .as_ref()
                .context("historical Memo replay requires validator post-balances")?;
            anyhow::ensure!(
                pre.len() == 4 && post.len() == 4,
                "historical Memo balance evidence is incomplete"
            );
        }
        // CPI replay is opt-in per adapter. Everything written before it existed
        // keeps the narrower guarantee it was proved under rather than
        // inheriting a wider one.
        if !self.adapter().is_some_and(|adapter| adapter.supports_cpi()) {
            anyhow::ensure!(
                self.transaction.inner_instructions.is_empty(),
                "this replay contract excludes CPI transactions"
            );
        }
        self.validate_dependencies()?;
        // A contract that admits CPI depends on more accounts, with more uneven
        // evidence behind them: validator metadata pins lamports and token
        // balances and says nothing at all about a pool's internal bytes. The
        // boundary proof that sufficed for two data-empty System accounts is not
        // enough here, so the screening evidence is required rather than
        // optional.
        anyhow::ensure!(
            !self.adapter().is_some_and(|adapter| adapter.supports_cpi())
                || self.slot_screening.is_some(),
            "a CPI replay record must carry same-slot interference screening"
        );
        if let Some(screening) = &self.slot_screening {
            anyhow::ensure!(
                screening.slot == self.transaction.slot
                    && screening.target_signature == self.transaction.signature,
                "slot screening does not describe this transaction"
            );
            // The screening result is only as good as the set it screened. A
            // record that carries a clean verdict over an empty or narrowed
            // required set proves nothing about the accounts the replay
            // actually depends on, so the set is recomputed here rather than
            // taken on the record's word.
            let screened: BTreeSet<&str> = screening
                .required_accounts
                .iter()
                .map(String::as_str)
                .collect();
            // Which accounts need this proof is decided by *default*, not by
            // the record's own list. Deriving the set from `acquisitions`
            // meant deleting that list emptied the set and exempted every
            // account at once - untrusted input deciding what needs proof.
            //
            // So: every non-runtime-managed account must be screened, and only
            // an explicit provenance entry naming a source that carries its own
            // boundary evidence can excuse one. A missing entry excuses nothing.
            let exempt: BTreeSet<&str> = self
                .acquisitions
                .iter()
                .filter(|a| a.source != AccountStateSource::HistoricalArchive)
                .map(|a| a.address.as_str())
                .collect();
            for account in &self.accounts {
                if is_runtime_managed(&account.address) || exempt.contains(account.address.as_str())
                {
                    continue;
                }
                anyhow::ensure!(
                    screened.contains(account.address.as_str()),
                    "slot screening omits required account {} ({}); its clean verdict does not \
                     cover the state this replay depends on",
                    account.label,
                    account.address
                );
            }
            screening.ensure_unambiguous()?;
        }

        // Provenance describes the boundary this record was acquired at. A slot
        // that is not the transaction's predecessor describes a different
        // boundary, and a record cannot substantiate a claim its own fields
        // contradict.
        let predecessor = self.transaction.slot.saturating_sub(1);
        for acquisition in &self.acquisitions {
            anyhow::ensure!(
                acquisition.context_slot == predecessor
                    || acquisition.context_slot == self.transaction.slot,
                "account {} was acquired at slot {}, which is neither the transaction's slot {} \
                 nor its predecessor {predecessor}",
                acquisition.label,
                acquisition.context_slot,
                self.transaction.slot
            );
        }
        for dependency in &self.dependencies.programs {
            if let Some(observed) = dependency.observed_slot {
                anyhow::ensure!(
                    observed == predecessor || observed == self.transaction.slot,
                    "dependency {} was resolved at slot {observed}, which is neither the \
                     transaction's slot {} nor its predecessor {predecessor}",
                    dependency.program_id,
                    self.transaction.slot
                );
            }
            if let Some(deployed) = dependency.deployed_slot {
                anyhow::ensure!(
                    deployed <= self.transaction.slot,
                    "dependency {} claims a deployment at slot {deployed}, after the transaction \
                     at slot {}",
                    dependency.program_id,
                    self.transaction.slot
                );
            }
        }
        anyhow::ensure!(
            self.transaction
                .account_keys
                .first()
                .is_some_and(|k| k.address == self.transaction.payer
                    && k.is_signer
                    && k.is_writable),
            "invalid replay payer"
        );
        anyhow::ensure!(
            self.clock.slot == self.transaction.slot,
            "execution clock slot differs from transaction slot"
        );
        anyhow::ensure!(
            state_hash(&self.accounts)? == self.pre_state_hash,
            "pre-state hash mismatch"
        );
        let addresses: BTreeSet<_> = self.accounts.iter().map(|a| a.address.as_str()).collect();
        let labels: BTreeSet<_> = self.accounts.iter().map(|a| a.label.as_str()).collect();
        anyhow::ensure!(
            labels.len() == self.accounts.len(),
            "duplicate account label"
        );
        let keys: BTreeSet<_> = self
            .transaction
            .account_keys
            .iter()
            .map(|a| a.address.as_str())
            .collect();
        anyhow::ensure!(
            keys.len() == self.transaction.account_keys.len(),
            "duplicate message key"
        );
        // A key that is invoked as a program is supplied by the runtime or by a
        // pinned artefact, not by the snapshot set. With CPI in scope, "invoked"
        // has to include programs reached only through an inner instruction and
        // programs the dependency manifest pins: a key that executes is never
        // state, whichever route named it.
        let invoked: BTreeSet<&str> = self
            .transaction
            .instructions
            .iter()
            .chain(&self.transaction.inner_instructions)
            .map(|instruction| instruction.program.as_str())
            .chain(
                self.dependencies
                    .programs
                    .iter()
                    .map(|program| program.program_id.as_str()),
            )
            .collect();
        for key in &self.transaction.account_keys {
            let runtime_key = key.address == self.program_id
                || invoked.contains(key.address.as_str())
                || (self.is_memo_transfer() && key.address == SYSTEM_PROGRAM_ID);
            if !runtime_key {
                // A key with no snapshot is only acceptable when the validator
                // recorded it holding nothing on both sides: the runtime then
                // materializes an empty account, which is what really executed.
                let held_nothing = |balances: &Option<Vec<u64>>| {
                    balances
                        .as_ref()
                        .and_then(|values| {
                            self.transaction
                                .account_keys
                                .iter()
                                .position(|other| other.address == key.address)
                                .and_then(|index| values.get(index))
                        })
                        .is_some_and(|lamports| *lamports == 0)
                };
                anyhow::ensure!(
                    addresses.contains(key.address.as_str())
                        || (held_nothing(&self.transaction.pre_balances)
                            && held_nothing(&self.transaction.post_balances)),
                    "missing required replay account {}",
                    key.address
                );
            }
        }
        for account in &self.accounts {
            anyhow::ensure!(
                keys.contains(account.address.as_str()),
                "snapshot contains account outside message"
            );
            anyhow::ensure!(
                !account.account.executable,
                "snapshot account is executable"
            );
        }
        let fixture = self.fixture();
        if let Some(adapter) = self.adapter() {
            anyhow::ensure!(
                self.accounts
                    .iter()
                    .any(|account| adapter.decode(&account.account).is_some()),
                "no snapshot decodes as a {} account",
                adapter.name()
            );
        } else if self.is_fixture_lending() {
            anyhow::ensure!(
                fixture
                    .account("position")
                    .and_then(|a| crate::interpret::position_economics(&a.account.data))
                    .is_some(),
                "missing controlled position valuation"
            );
        } else {
            anyhow::ensure!(
                self.accounts.len() == 2
                    && self.accounts.iter().all(|account| {
                        account.account.owner == SYSTEM_PROGRAM_ID
                            && account.account.data.is_empty()
                    }),
                "Memo transfer replay supports two data-empty System accounts only"
            );
            let pre = self
                .transaction
                .pre_balances
                .as_ref()
                .expect("checked above");
            for account in &self.accounts {
                let index = self
                    .transaction
                    .account_keys
                    .iter()
                    .position(|key| key.address == account.address)
                    .context("snapshot account missing from message")?;
                anyhow::ensure!(
                    account.account.lamports == pre[index],
                    "archive pre-state differs from validator pre-balance for {}",
                    account.address
                );
            }
        }
        self.message()?;
        Ok(())
    }
    pub fn fixture(&self) -> Fixture {
        let mainnet_memo = self.is_memo_transfer();
        let adapter = self.adapter();
        Fixture {
            id: self.id.clone(),
            category: Category::Boundary,
            scenario: match (adapter, mainnet_memo) {
                (Some(adapter), _) => {
                    format!("historical mainnet {} interaction", adapter.name())
                }
                (None, true) => "historical mainnet System transfer with Memo".into(),
                (None, false) => "historical controlled interaction".into(),
            },
            notes: match (adapter.is_some(), mainnet_memo) {
                (true, _) => {
                    "Amounts are exact token base units; no fiat valuation is inferred".into()
                }
                (false, true) => {
                    "Native value is the exact transfer amount; no fiat valuation is inferred"
                        .into()
                }
                (false, false) => {
                    "Capital represents replay observations, not unique positions or deployed TVL"
                        .into()
                }
            },
            keypairs: vec![],
            accounts: self.accounts.clone(),
            fee_payer: self.transaction.payer.clone(),
            signers: vec![],
            instruction: self.primary_instruction().clone(),
            watch: self.accounts.iter().map(|a| a.label.clone()).collect(),
        }
    }
    fn message(&self) -> Result<Message> {
        let keys = &self.transaction.account_keys;
        let signed = keys.iter().take_while(|k| k.is_signer).count();
        anyhow::ensure!(
            keys[signed..].iter().all(|k| !k.is_signer),
            "signer keys must be contiguous"
        );
        for group in [&keys[..signed], &keys[signed..]] {
            let writable = group.iter().take_while(|k| k.is_writable).count();
            anyhow::ensure!(
                group[writable..].iter().all(|k| !k.is_writable),
                "invalid key privilege ordering"
            );
        }
        let index = |address: &str| -> Result<u8> {
            u8::try_from(
                keys.iter()
                    .position(|k| k.address == address)
                    .context("instruction key missing from message")?,
            )
            .context("too many keys")
        };
        let instructions = self
            .transaction
            .instructions
            .iter()
            .map(|ix| {
                for meta in &ix.accounts {
                    anyhow::ensure!(
                        keys.get(index(&meta.address)? as usize) == Some(meta),
                        "instruction privileges differ from message"
                    );
                }
                Ok(CompiledInstruction {
                    program_id_index: index(&ix.program)?,
                    accounts: ix
                        .accounts
                        .iter()
                        .map(|a| index(&a.address))
                        .collect::<Result<_>>()?,
                    data: ix.data.clone(),
                })
            })
            .collect::<Result<_>>()?;
        Ok(Message {
            header: MessageHeader {
                num_required_signatures: u8::try_from(signed)?,
                num_readonly_signed_accounts: u8::try_from(
                    keys[..signed].iter().filter(|k| !k.is_writable).count(),
                )?,
                num_readonly_unsigned_accounts: u8::try_from(
                    keys[signed..].iter().filter(|k| !k.is_writable).count(),
                )?,
            },
            account_keys: keys
                .iter()
                .map(|k| k.address.parse().map_err(Into::into))
                .collect::<Result<_>>()?,
            recent_blockhash: self.transaction.recent_blockhash.parse()?,
            instructions,
        })
    }
    /// Programs whose bytes this record pins and which are not the one under
    /// test. Order is the manifest's, which is canonical by program ID.
    pub fn dependency_ids(&self) -> Vec<&str> {
        self.dependencies
            .loadable()
            .map(|program| program.program_id.as_str())
            .filter(|id| *id != self.program_id)
            .collect()
    }

    fn validate_dependencies(&self) -> Result<()> {
        if self.dependencies.programs.is_empty() {
            // A record with no manifest must also have nothing to resolve: no
            // CPI, and no top-level program but its own and the runtime's.
            anyhow::ensure!(
                self.transaction.inner_instructions.is_empty(),
                "a CPI transaction requires a dependency manifest"
            );
            return Ok(());
        }
        let unsupported = self.dependencies.unsupported();
        anyhow::ensure!(
            unsupported.is_empty(),
            "replay requires {} program(s) this build cannot reproduce: {}",
            unsupported.len(),
            unsupported
                .iter()
                .map(|program| program.program_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let known: BTreeSet<&str> = self
            .dependencies
            .programs
            .iter()
            .map(|program| program.program_id.as_str())
            .collect();
        for (program_id, _) in
            crate::dependencies::discover(&self.transaction, self.adapter(), &self.program_id)
        {
            anyhow::ensure!(
                known.contains(program_id.as_str()),
                "program {program_id} executes in this transaction but the dependency \
                 manifest does not pin it"
            );
        }
        for program in &self.dependencies.programs {
            if program.source == ProgramSource::HistoricalMainnet {
                anyhow::ensure!(
                    program.binary_sha256.is_some(),
                    "dependency {} is pinned to history with no binary hash",
                    program.program_id
                );
                program.program_id.parse::<Address>().with_context(|| {
                    format!("dependency {} is not an address", program.program_id)
                })?;
            }
        }
        Ok(())
    }

    pub fn execute(
        &self,
        program: &ProgramVersion,
        dependencies: &DependencyBundle,
    ) -> Result<ExecutionResult> {
        self.validate()?;
        for required in self.dependency_ids() {
            anyhow::ensure!(
                dependencies
                    .programs()
                    .iter()
                    .any(|loaded| loaded.program_id.to_string() == required),
                "replay {} needs the historical binary for {required}; supply the replay \
                 bundle's dependency directory",
                self.id
            );
        }
        execute_in_environment(
            &self.fixture(),
            &self.program_id.parse()?,
            program,
            self.clock.clock(),
            Some(self.message()?),
            dependencies.programs(),
        )
    }
    pub fn post_hash(&self, result: &ExecutionResult) -> Result<String> {
        let accounts = self
            .accounts
            .iter()
            .map(|a| {
                Ok(NamedAccount {
                    label: a.label.clone(),
                    address: a.address.clone(),
                    account: result
                        .accounts
                        .get(&a.label)
                        .cloned()
                        .context("watched account missing after replay")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        outcome_hash(&accounts)
    }
    /// Credits to rent-paying accounts that the local runtime will refuse.
    ///
    /// Evidence required before reporting one: the account was already below
    /// its rent-exempt minimum, the transaction credited it, it stayed below
    /// the minimum, and the original transaction succeeded on mainnet. Absent
    /// any of those this returns nothing, so an ordinary rent-exempt account or
    /// a debit is never implicated.
    pub fn rent_paying_credits(&self) -> Vec<RentPayingCredit> {
        let Some(original) = &self.original else {
            return Vec::new();
        };
        if !original.success {
            return Vec::new();
        }
        let rent = solana_rent::Rent::default();
        let mut out = Vec::new();
        for before in &self.accounts {
            let Some(after) = original
                .post_accounts
                .iter()
                .find(|d| d.address == before.address)
            else {
                continue;
            };
            let minimum = rent.minimum_balance(before.account.data.len());
            let credited = after.lamports > before.account.lamports;
            let stays_rent_paying = before.account.lamports < minimum && after.lamports < minimum;
            // A resize would be a different transition; this reason is only for
            // the credit case, where size is unchanged.
            let same_size = after.data_len == before.account.data.len() as u64;
            if credited && stays_rent_paying && same_size {
                out.push(RentPayingCredit {
                    address: before.address.clone(),
                    label: before.label.clone(),
                    owner: before.account.owner.clone(),
                    pre_lamports: before.account.lamports,
                    post_lamports: after.lamports,
                    credited: after.lamports - before.account.lamports,
                    rent_exempt_minimum: minimum,
                });
            }
        }
        out
    }

    /// Why a replay failed the gate, in the order the criteria are checked.
    ///
    /// Reported rather than summarized because "mismatch" alone is unusable:
    /// a differing fee, a differing post-state and a differing invocation graph
    /// are three different problems with three different causes.
    pub fn fidelity_failures(&self, result: &ExecutionResult) -> Result<Vec<String>> {
        let Some(original) = &self.original else {
            return Ok(Vec::new());
        };
        let mut failures = Vec::new();
        if result.success != original.success {
            failures.push(format!(
                "outcome: original {}, replay {}{}",
                if original.success {
                    "succeeded"
                } else {
                    "failed"
                },
                if result.success {
                    "succeeded"
                } else {
                    "failed"
                },
                result
                    .error
                    .as_ref()
                    .map(|error| format!(" ({error})"))
                    .unwrap_or_default()
            ));
        }
        if result.fee != original.fee {
            failures.push(format!(
                "fee: original {} lamports, replay {} lamports",
                original.fee, result.fee
            ));
        }
        let replayed = cpi_graph(&result.cpi_calls);
        if replayed != original.cpi_invocations {
            failures.push(format!(
                "invocation graph: original made {} cross-program call(s), replay made {}",
                original.cpi_invocations.len(),
                replayed.len()
            ));
        }
        let hash = self.post_hash(result)?;
        if hash != original.post_state_hash {
            // Name the accounts, when the record carries enough to do so. A
            // runtime-managed account is reported as excluded rather than
            // omitted, so a reader can see it was considered.
            for digest in &original.post_accounts {
                let Some(replayed) = result.accounts.get(&digest.label) else {
                    failures.push(format!(
                        "post-state: {} ({}) missing from the replay",
                        digest.label, digest.address
                    ));
                    continue;
                };
                let differences = digest.differences(replayed);
                if differences.is_empty() {
                    continue;
                }
                failures.push(format!(
                    "post-state: {} ({}) {}{}",
                    digest.label,
                    digest.address,
                    differences.join(", "),
                    if is_runtime_managed(&digest.address) {
                        " [runtime-managed; excluded from the gate]"
                    } else {
                        ""
                    }
                ));
            }
            failures.push(format!(
                "post-state: original {}, replay {hash}",
                original.post_state_hash
            ));
        }
        Ok(failures)
    }

    pub fn fidelity(&self, result: &ExecutionResult) -> Result<ReplayFidelity> {
        if self.state_source == ReplayStateSource::CurrentApproximation {
            return Ok(ReplayFidelity::Approximate);
        }
        let Some(original) = &self.original else {
            return Ok(ReplayFidelity::Unknown);
        };
        let _ = original;
        if !self.fidelity_failures(result)?.is_empty() {
            return Ok(ReplayFidelity::Mismatch);
        }
        Ok(
            if self.state_source == ReplayStateSource::ControlledSnapshot {
                ReplayFidelity::Exact
            } else {
                ReplayFidelity::Matched
            },
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayObservation {
    pub id: String,
    pub source_signature: String,
    pub source_slot: u64,
    pub state_source: ReplayStateSource,
    pub fidelity: ReplayFidelity,
    pub pre_state_hash: String,
    pub post_v1_state_hash: String,
    pub post_v2_state_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_transfer_lamports: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_prevented_native_transfer_lamports: Option<u64>,
    /// Protocol-level differences between the two builds, from the adapter.
    /// Empty when the outcome is identical, which is the expected result for a
    /// behaviour-preserving upgrade.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub economic_changes: Vec<crate::protocol::EconomicChange>,
    /// Headline protocol quantities under both builds, reported whether or not
    /// they differ. A preserved economic outcome is a result, not an absence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub economic_summary: Vec<EconomicObservation>,
    /// The economic entity this observation belongs to, where the adapter names
    /// one. Bounds on affected entities are counted from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub economic_entity: Option<String>,
    /// What this observation is *able* to measure, from the adapter, derived
    /// from the observation's shape rather than from anything that differed.
    /// This is the evidence that separates a stale declaration from one this
    /// corpus simply cannot judge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluable_subjects: Vec<crate::semantics::EvaluableSubject>,
    /// Differences named in the finding vocabulary, for expectation review.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub named_findings: Vec<crate::semantics::NamedFinding>,
    /// Programs the replay environment supplied, and where their bytes came
    /// from. Hashes are repeated here so a report is self-contained evidence of
    /// which binaries executed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_programs: Vec<crate::dependencies::ProgramDependency>,
    /// The invocation graph each side produced, and the one mainnet recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub original_cpi_graph: Vec<CpiFrame>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cpi_graph_v1: Vec<CpiFrame>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cpi_graph_v2: Vec<CpiFrame>,
    /// Whether the two builds invoked the same programs in the same shape.
    ///
    /// Reported rather than scored: a changed invocation graph is a fact about
    /// the candidate, and whether it matters is a question about semantics that
    /// this layer deliberately does not answer.
    #[serde(default)]
    pub cpi_graph_changed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeReplayImpact {
    pub v1_transferred_lamports: u64,
    pub candidate_prevented_lamports: u64,
}
/// Wall-clock cost of one record's two executions.
///
/// Deliberately not serialized: a report has to be byte-identical across runs
/// for the determinism check to mean anything, and a timing never is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplayTiming {
    pub id_index: usize,
    pub v1_micros: u128,
    pub v2_micros: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayReport {
    pub schema_version: u32,
    pub observations: Vec<ReplayObservation>,
    pub analysis: Report,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_impact: Option<NativeReplayImpact>,
    /// Observations whose protocol-level economics differ between the builds.
    ///
    /// Kept separate from [`Report::summary`]'s counts because that classifier
    /// is protocol-agnostic by design: it sees a token account's bytes change
    /// and can only call it a raw-data difference. The adapter is what knows
    /// the bytes were a balance. Whether such a change is *intended* is a
    /// question this phase deliberately does not answer.
    #[serde(default)]
    pub economic_findings: usize,
    #[serde(skip)]
    pub timings: Vec<ReplayTiming>,
}

pub fn compare(
    records: &[ReplayRecord],
    v1: &ProgramVersion,
    v2: &ProgramVersion,
) -> Result<ReplayReport> {
    compare_with_dependencies(records, v1, v2, &DependencyBundle::empty())
}

/// Compare two builds over a replay corpus, with pinned dependency binaries.
///
/// The V1 fidelity gate is unchanged and unweakened: the candidate is executed
/// only after V1, running the binary that was actually deployed alongside the
/// dependency binaries that were actually deployed, reproduces the original
/// mainnet outcome - its success, its fee, its invocation graph and its
/// post-state. A failure aborts the comparison rather than downgrading it.
pub fn compare_with_dependencies(
    records: &[ReplayRecord],
    v1: &ProgramVersion,
    v2: &ProgramVersion,
    dependencies: &DependencyBundle,
) -> Result<ReplayReport> {
    anyhow::ensure!(!records.is_empty(), "empty replay corpus");
    let mut ids = BTreeSet::new();
    let mut fixtures = Vec::new();
    let mut diffs = Vec::new();
    let mut observations = Vec::new();
    let mut timings = Vec::new();
    let mut v1_transferred_lamports = 0_u64;
    let mut candidate_prevented_lamports = 0_u64;
    for record in records {
        anyhow::ensure!(ids.insert(&record.id), "duplicate replay ID");
        anyhow::ensure!(
            record.current_program_sha256 == hash_bytes(&v1.bytes),
            "V1 binary differs from captured program"
        );
        let v1_started = std::time::Instant::now();
        let original = record
            .execute(v1, dependencies)
            .with_context(|| format!("V1 replay {}", record.id))?;
        let v1_micros = v1_started.elapsed().as_micros();
        let fidelity = record.fidelity(&original)?;
        anyhow::ensure!(
            matches!(fidelity, ReplayFidelity::Exact | ReplayFidelity::Matched),
            "replay {} fidelity {fidelity:?}; candidate execution withheld.{}",
            record.id,
            record
                .fidelity_failures(&original)?
                .iter()
                .map(|failure| format!("\n  - {failure}"))
                .collect::<String>()
        );
        let v2_started = std::time::Instant::now();
        let candidate = record.execute(v2, dependencies)?;
        timings.push(ReplayTiming {
            id_index: observations.len(),
            v1_micros,
            v2_micros: v2_started.elapsed().as_micros(),
        });
        let native_transfer_lamports = record.native_transfer_lamports();
        let prevented = native_transfer_lamports.filter(|_| original.success && !candidate.success);
        v1_transferred_lamports = v1_transferred_lamports
            .checked_add(native_transfer_lamports.unwrap_or(0))
            .context("native transfer total overflow")?;
        candidate_prevented_lamports = candidate_prevented_lamports
            .checked_add(prevented.unwrap_or(0))
            .context("prevented native transfer total overflow")?;
        observations.push(ReplayObservation {
            id: record.id.clone(),
            source_signature: record.transaction.signature.clone(),
            source_slot: record.transaction.slot,
            state_source: record.state_source.clone(),
            fidelity,
            pre_state_hash: record.pre_state_hash.clone(),
            post_v1_state_hash: record.post_hash(&original)?,
            post_v2_state_hash: record.post_hash(&candidate)?,
            native_transfer_lamports,
            candidate_prevented_native_transfer_lamports: prevented,
            economic_changes: match record.adapter() {
                Some(adapter) => adapter.interpret(&record.accounts, &original, &candidate),
                None => Vec::new(),
            },
            economic_summary: match record.adapter() {
                Some(adapter) => protocol::pair_summaries(
                    &adapter.summarize(&record.accounts, &original),
                    &adapter.summarize(&record.accounts, &candidate),
                ),
                None => Vec::new(),
            },
            economic_entity: record
                .adapter()
                .and_then(|adapter| {
                    adapter.economic_entity_id(&record.transaction, &record.accounts)
                })
                .map(|entity| entity.id),
            evaluable_subjects: match record.adapter() {
                Some(adapter) => adapter.evaluable_subjects(&record.transaction, &record.accounts),
                None => Vec::new(),
            },
            named_findings: match record.adapter() {
                Some(adapter) => adapter.named_findings(
                    &record.transaction,
                    &record.accounts,
                    &original,
                    &candidate,
                ),
                None => Vec::new(),
            },
            dependency_programs: record.dependencies.programs.clone(),
            original_cpi_graph: record
                .original
                .as_ref()
                .map(|original| original.cpi_invocations.clone())
                .unwrap_or_default(),
            cpi_graph_changed: cpi_graph(&original.cpi_calls) != cpi_graph(&candidate.cpi_calls),
            cpi_graph_v1: cpi_graph(&original.cpi_calls),
            cpi_graph_v2: cpi_graph(&candidate.cpi_calls),
        });
        let fixture = record.fixture();
        // Who owns this record's economics decides whether the generic layer
        // may decode at all. A record with an adapter has one: the adapter
        // names its quantities, and the generic layer must compare bytes
        // without pretending to understand them. A record without one is the
        // fixture protocol or the bounded pre-adapter path, which does own the
        // lending layout and whose economics come from exactly that decoder.
        let decoder = match record.adapter() {
            Some(_) => crate::diff::FieldDecoder::None,
            None => crate::diff::FieldDecoder::FixtureLending,
        };
        diffs.push(crate::diff::compare_with_decoder(
            &fixture, original, candidate, decoder,
        ));
        fixtures.push(fixture);
    }
    let native_impact = (v1_transferred_lamports > 0).then_some(NativeReplayImpact {
        v1_transferred_lamports,
        candidate_prevented_lamports,
    });
    let economic_findings = observations
        .iter()
        .filter(|observation| !observation.economic_changes.is_empty())
        .count();
    Ok(ReplayReport {
        schema_version: REPLAY_SCHEMA,
        economic_findings,
        timings,
        observations,
        analysis: Report::new(
            records[0].program_id.clone(),
            v1.label.clone(),
            v2.label.clone(),
            &fixtures,
            diffs,
        ),
        native_impact,
    })
}

pub fn load_corpus(path: &std::path::Path) -> Result<Vec<ReplayRecord>> {
    let records: Vec<ReplayRecord> = serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("reading corpus {}", path.display()))?,
    )?;
    for record in &records {
        record.validate()?;
    }
    Ok(records)
}

/// Default location for a replay bundle's dependency artefacts: a `dependencies`
/// directory beside the corpus file that names them.
pub fn dependency_directory(corpus: &Path) -> std::path::PathBuf {
    corpus
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("dependencies")
}

/// Load the dependency artefacts every record in a corpus requires.
///
/// Records that pin nothing produce an empty bundle and never touch the
/// directory, which is what keeps Phase 4 through 7 corpora loadable with no
/// bundle present at all.
pub fn load_dependencies(records: &[ReplayRecord], directory: &Path) -> Result<DependencyBundle> {
    let mut merged = DependencyManifest::default();
    for record in records {
        // The program under test is supplied as the comparison's two artefacts,
        // not from the bundle: it is the one thing the run varies.
        for program in record
            .dependencies
            .loadable()
            .filter(|program| program.program_id != record.program_id)
        {
            match merged.get(&program.program_id) {
                Some(existing) => anyhow::ensure!(
                    existing.binary_sha256 == program.binary_sha256,
                    "records disagree about which binary {} was; a corpus cannot mix \
                     dependency versions in one comparison",
                    program.program_id
                ),
                None => merged.programs.push(program.clone()),
            }
        }
    }
    if merged.programs.is_empty() {
        return Ok(DependencyBundle::empty());
    }
    DependencyBundle::load(&merged, directory)
}

/// State snapshots in external captures are keyed by public address.
pub type AccountMap = BTreeMap<String, AccountSnapshot>;

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(owner: &str, data: Vec<u8>, executable: bool) -> AccountSnapshot {
        AccountSnapshot {
            lamports: 1_000,
            owner: owner.into(),
            data,
            executable,
            rent_epoch: 0,
        }
    }

    /// The only account whose whole state is its lamport balance.
    #[test]
    fn only_an_empty_system_account_may_rest_on_balance_metadata() {
        let payer = snapshot(SYSTEM_PROGRAM_ID, vec![], false);
        assert!(reconstructable_from_balances(&payer, &payer));
    }

    /// Every class the reconstruction must never admit. Each is excluded for a
    /// stated reason, and each is a real account shape this corpus encounters.
    #[test]
    fn ineligible_account_classes_cannot_enter_the_reconstruction_path() {
        let cases: Vec<(&str, AccountSnapshot)> = vec![
            // Non-System owner: the Jito tip account that produced the known
            // rent-state divergence. Program-owned and data-bearing.
            (
                "jito tip account",
                snapshot(
                    "T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt",
                    vec![0; 8],
                    false,
                ),
            ),
            (
                "token account",
                snapshot(
                    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                    vec![0; 165],
                    false,
                ),
            ),
            (
                "token-2022 account",
                snapshot(
                    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
                    vec![0; 165],
                    false,
                ),
            ),
            (
                "mint",
                snapshot(
                    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                    vec![0; 82],
                    false,
                ),
            ),
            (
                "stake account",
                snapshot(
                    "Stake11111111111111111111111111111111111111",
                    vec![0; 200],
                    false,
                ),
            ),
            (
                "pool PDA",
                snapshot(
                    "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
                    vec![0; 611],
                    false,
                ),
            ),
            (
                "clock sysvar",
                snapshot(
                    "Sysvar1111111111111111111111111111111111111",
                    vec![0; 40],
                    false,
                ),
            ),
            (
                "program account",
                snapshot(
                    "BPFLoaderUpgradeab1e11111111111111111111111",
                    vec![0; 36],
                    true,
                ),
            ),
            // System-owned but stateful: a durable nonce carries 80 bytes, so
            // its lamports do not describe it.
            (
                "durable nonce",
                snapshot(SYSTEM_PROGRAM_ID, vec![0; 80], false),
            ),
            // System-owned and empty but executable, which no real account is -
            // included so the executable guard is proved independently.
            (
                "executable system account",
                snapshot(SYSTEM_PROGRAM_ID, vec![], true),
            ),
        ];
        for (name, account) in cases {
            assert!(
                !reconstructable_from_balances(&account, &account),
                "{name} must not be reconstructable from balance metadata"
            );
        }
    }

    /// Both boundaries must agree. An account another transaction allocated or
    /// reassigned inside the slot is refused rather than reconstructed.
    #[test]
    fn a_boundary_disagreement_blocks_reconstruction() {
        let empty = snapshot(SYSTEM_PROGRAM_ID, vec![], false);
        let allocated = snapshot(SYSTEM_PROGRAM_ID, vec![0; 1], false);
        let reassigned = snapshot("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", vec![], false);
        assert!(
            !reconstructable_from_balances(&empty, &allocated),
            "allocated mid-slot"
        );
        assert!(
            !reconstructable_from_balances(&allocated, &empty),
            "freed mid-slot"
        );
        assert!(
            !reconstructable_from_balances(&empty, &reassigned),
            "reassigned mid-slot"
        );
        assert!(
            !reconstructable_from_balances(&reassigned, &empty),
            "reassigned mid-slot"
        );
    }

    /// A real account's post-state is exactly what the gate exists to check.
    #[test]
    fn a_changed_protocol_account_post_state_must_change_the_outcome_hash() {
        let pool = account(
            "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
            10,
            vec![1, 2, 3],
        );
        let moved = account(
            "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
            11,
            vec![1, 2, 3],
        );
        let rewritten = account(
            "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
            10,
            vec![1, 2, 4],
        );
        let base = outcome_hash(&[pool]).unwrap();
        assert_ne!(
            base,
            outcome_hash(&[moved]).unwrap(),
            "a lamport change must be caught"
        );
        assert_ne!(
            base,
            outcome_hash(&[rewritten]).unwrap(),
            "a data change must be caught"
        );
    }

    /// Pre-state integrity is unchanged: it still hashes every account, sysvars
    /// included, so a record whose Clock input was altered fails before any
    /// replay runs. The exclusion applies to the *post*-state only.
    #[test]
    fn a_changed_clock_pre_state_still_breaks_record_integrity() {
        let pool = account(
            "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
            10,
            vec![1, 2, 3],
        );
        let clock = account("SysvarC1ock11111111111111111111111111111111", 1, vec![4, 5]);
        let tampered = account("SysvarC1ock11111111111111111111111111111111", 1, vec![4, 6]);
        assert_ne!(
            state_hash(&[pool.clone(), clock]).unwrap(),
            state_hash(&[pool, tampered]).unwrap(),
            "pre-state hashing must still cover the sysvar the program reads"
        );
    }

    /// An account that merely looks like a sysvar must not exclude itself. Only
    /// the enumerated addresses are dropped, so a ground vanity address is
    /// still fully covered by the gate.
    #[test]
    fn a_vanity_lookalike_account_is_not_excluded() {
        // A real, valid address one byte away from the Clock sysvar: the
        // closest thing an attacker could grind to something that "looks like"
        // a sysvar. Exclusion is by exact identity, so it must not apply.
        let clock: solana_address::Address = "SysvarC1ock11111111111111111111111111111111"
            .parse()
            .unwrap();
        let mut bytes = clock.to_bytes();
        bytes[31] ^= 1;
        let lookalike = solana_address::Address::new_from_array(bytes).to_string();

        assert_ne!(lookalike, clock.to_string());
        assert!(
            !is_runtime_managed(&lookalike),
            "{lookalike} must not be excluded"
        );
        assert_ne!(
            outcome_hash(&[account(&lookalike, 1, vec![1])]).unwrap(),
            outcome_hash(&[account(&lookalike, 2, vec![1])]).unwrap(),
            "a sysvar lookalike must still be covered by the outcome hash"
        );
        // And the genuine Clock still is excluded, so the two are distinguished
        // by identity rather than by resemblance.
        assert!(is_runtime_managed(&clock.to_string()));
    }

    /// The exclusion list is part of what a fidelity result means. Changing it
    /// changes the claim every archive record makes, so it is pinned here: if
    /// this fails, bump `REPLAY_SCHEMA`, restate the assumption the adapters
    /// emit, and rebuild affected corpora deliberately.
    #[test]
    fn the_runtime_managed_list_is_pinned_to_the_recorded_schema() {
        assert_eq!(
            RUNTIME_MANAGED,
            [
                "SysvarC1ock11111111111111111111111111111111",
                "SysvarStakeHistory1111111111111111111111111",
                "SysvarRent111111111111111111111111111111111",
                "SysvarS1otHashes111111111111111111111111111",
                "SysvarRecentB1ockHashes11111111111111111111",
                "SysvarEpochSchedu1e111111111111111111111111",
                "SysvarEpochRewards1111111111111111111111111",
                "SysvarLastRestartS1ot1111111111111111111111",
            ],
            "the set of accounts excluded from the post-state gate changed"
        );
        assert_eq!(
            REPLAY_SCHEMA, 1,
            "schema must be bumped alongside the list above"
        );
    }

    fn account(address: &str, lamports: u64, data: Vec<u8>) -> NamedAccount {
        NamedAccount {
            label: address.to_string(),
            address: address.to_string(),
            account: AccountSnapshot {
                lamports,
                owner: "11111111111111111111111111111111".into(),
                data,
                executable: false,
                rent_epoch: 0,
            },
        }
    }

    /// The post-state gate must ask only about accounts the candidate is
    /// answerable for. A record naming no runtime-managed account has to hash
    /// exactly as before, or every corpus written by an earlier phase would
    /// silently change meaning.
    #[test]
    fn the_outcome_hash_drops_runtime_managed_accounts_only() {
        let pool = account(
            "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
            10,
            vec![1, 2, 3],
        );
        let clock = account("SysvarC1ock11111111111111111111111111111111", 1, vec![4, 5]);

        let plain = vec![pool.clone()];
        assert_eq!(
            outcome_hash(&plain).unwrap(),
            state_hash(&plain).unwrap(),
            "a record with no runtime-managed account must hash unchanged"
        );

        let with_clock = vec![pool.clone(), clock];
        assert_ne!(
            outcome_hash(&with_clock).unwrap(),
            state_hash(&with_clock).unwrap()
        );
        assert_eq!(
            outcome_hash(&with_clock).unwrap(),
            state_hash(&plain).unwrap()
        );

        // The clock's own value cannot move the outcome hash, which is the
        // point: no replay reproduces mainnet's wall clock.
        let advanced = account(
            "SysvarC1ock11111111111111111111111111111111",
            4_242,
            vec![9, 9],
        );
        assert_eq!(
            outcome_hash(&with_clock).unwrap(),
            outcome_hash(&[pool, advanced]).unwrap()
        );
    }

    #[test]
    fn runtime_managed_recognises_the_sysvars_a_withdrawal_reads() {
        assert!(is_runtime_managed(
            "SysvarC1ock11111111111111111111111111111111"
        ));
        assert!(is_runtime_managed(
            "SysvarStakeHistory1111111111111111111111111"
        ));
        // Matched by exact identity, not by prefix: a ground vanity address must
        // not be able to exclude itself from the fidelity gate.
        assert!(!is_runtime_managed(
            "SysvarFakeButGroundVanityAddress1111111111"
        ));
        assert!(!is_runtime_managed(
            "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy"
        ));
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    fn stake_pool_record() -> ReplayRecord {
        serde_json::from_str(include_str!(
            "../../docs/examples/mainnet-stake-pool-record.json"
        ))
        .expect("committed record")
    }

    /// The real acquired records must still pass, or the new checks are
    /// rejecting evidence rather than forgeries.
    #[test]
    fn the_committed_records_still_validate() {
        for raw in [
            include_str!("../../docs/examples/mainnet-stake-pool-record.json"),
            include_str!("../../docs/examples/mainnet-stake-pool-withdraw-record.json"),
            include_str!("../../docs/examples/mainnet-token2022-record.json"),
            include_str!("../../docs/examples/mainnet-replay-record.json"),
            include_str!("../../docs/examples/replay-record.json"),
        ] {
            let record: ReplayRecord = serde_json::from_str(raw).expect("parses");
            record.validate().expect("a real record validates");
        }
    }

    /// A clean verdict over a narrowed set proves nothing about the accounts the
    /// replay actually depends on.
    #[test]
    fn screening_that_covers_fewer_accounts_than_the_replay_needs_is_refused() {
        let mut record = stake_pool_record();
        record
            .slot_screening
            .as_mut()
            .expect("a screened record")
            .required_accounts
            .clear();
        let error = record.validate().expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("omits required account"),
            "{error:#}"
        );
    }

    /// Provenance that describes a different boundary cannot substantiate this
    /// record's claims.
    #[test]
    fn acquisition_at_an_unrelated_slot_is_refused() {
        let mut record = stake_pool_record();
        let slot = record.transaction.slot;
        record.acquisitions[0].context_slot = slot + 999;
        let error = record.validate().expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("was acquired at slot"),
            "{error:#}"
        );
    }

    #[test]
    fn a_dependency_resolved_at_an_unrelated_slot_is_refused() {
        let mut record = stake_pool_record();
        let slot = record.transaction.slot;
        for dependency in &mut record.dependencies.programs {
            if dependency.observed_slot.is_some() {
                dependency.observed_slot = Some(slot + 999);
                break;
            }
        }
        let error = record.validate().expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("was resolved at slot"),
            "{error:#}"
        );
    }

    /// A program cannot have been deployed after the transaction that used it.
    #[test]
    fn a_dependency_deployed_after_the_transaction_is_refused() {
        let mut record = stake_pool_record();
        let slot = record.transaction.slot;
        for dependency in &mut record.dependencies.programs {
            if dependency.deployed_slot.is_some() {
                dependency.deployed_slot = Some(slot + 1);
                break;
            }
        }
        let error = record.validate().expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("after the transaction"),
            "{error:#}"
        );
    }
}

#[cfg(test)]
mod provenance_absence {
    use super::*;

    /// The withdrawal record, whose `destination-lamports` boundary comes from
    /// transaction balances rather than the archive. It is therefore
    /// legitimately outside the screened set - and so it is exactly the record
    /// where an exemption can be abused.
    fn record() -> ReplayRecord {
        serde_json::from_str(include_str!(
            "../../docs/examples/mainnet-stake-pool-withdraw-record.json"
        ))
        .expect("committed record")
    }

    /// Deleting the provenance list must not exempt every account from proof.
    /// Deriving the "needs screening" set from that list let untrusted input
    /// decide what needed proving, so removing it removed the requirement.
    #[test]
    fn stripping_acquisitions_does_not_waive_screening() {
        let mut stripped = record();
        stripped.acquisitions.clear();
        let error = stripped.validate().expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("omits required account"),
            "{error:#}"
        );
    }

    /// The same record with both provenance and the screened set emptied: the
    /// combination the audit used, and it must still fail.
    #[test]
    fn stripping_provenance_and_the_screened_set_together_is_refused() {
        let mut stripped = record();
        stripped.acquisitions.clear();
        if let Some(screening) = stripped.slot_screening.as_mut() {
            screening.required_accounts.clear();
        }
        assert!(stripped.validate().is_err());
    }

    /// An account with an explicit non-archive provenance entry is still
    /// legitimately outside the screened set - the exemption exists, it just
    /// has to be claimed rather than inferred from silence.
    #[test]
    fn an_explicitly_non_archived_account_stays_exempt() {
        record()
            .validate()
            .expect("the real record still validates");
    }
}
