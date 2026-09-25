//! Governance binding: is the proposal people are approving the change Eplyx
//! analysed?
//!
//! ```text
//! GOVERNANCE PROPOSAL  ==  ANALYSED CHANGESPEC        (at slot S)
//! ```
//!
//! One provider and one shape: a Squads V4 vault transaction whose stored
//! message is exactly one loader-v3 `Upgrade`, with no address lookup tables.
//! Anything else is [`BindingOutcome::UnsupportedProposal`], never a partial
//! match.
//!
//! # Three identities, not one
//!
//! * **Governance message** — the Squads vault transaction message. Immutable
//!   once created, and hashed exactly as stored ([`squads::message_hash`]).
//!   It names a *buffer account*.
//! * **Candidate artefact** — the SHA-256 of the bytes in that buffer. The
//!   message does not commit to them: a buffer can be rewritten by its
//!   authority until the upgrade executes.
//! * **Freshness** — the slot at which the buffer was read and found to hold
//!   the candidate. A binding is a statement *at a slot*, never a statement
//!   that the proposal can never diverge.
//!
//! What narrows the gap between the second and third: the loader requires the
//! buffer's authority to equal the upgrade authority when `Upgrade` executes,
//! and only the authority can write, re-assign or close a buffer. So a buffer
//! whose authority is the Squads vault can change afterwards only through
//! another transaction that vault signs — another approved proposal of the
//! same multisig. A buffer with any other authority can change with no vote at
//! all, and is never reported as matched.
//!
//! This module reads and verifies. It never signs, approves, rejects,
//! cancels, executes or creates anything, and holds no key.

pub mod attestation;
pub mod simulated;
pub mod squads;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;

use crate::change::{ChangeSpec, Delivery, ExecutableArtifact, SquadsV4Delivery};
use crate::ingest::{accounts, rpc::RpcProvider};
use crate::replay::hash_bytes;
use crate::standard_programs::upgradeable_loader::{self as loader, LoaderInstruction};
use crate::types::AccountSnapshot;

pub const BINDING_SCHEMA: u32 = 1;
const BINDING_DOMAIN: &str = "eplyx-governance-binding-v1";
pub const BINDING_KIND: &str = "squads_v4_program_upgrade";

/// The proposal a caller asks about. Only the two facts Squads derives every
/// other address from: nothing user-supplied is trusted as an address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SquadsProposalRef {
    pub multisig: String,
    pub transaction_index: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Commitment {
    Confirmed,
    /// The default: an observation that cannot be rolled back.
    #[default]
    Finalized,
}

impl Commitment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
        }
    }
}

/// What the binding found. Deliberately not a boolean.
///
/// Ordered by precedence: when several apply, the first one here heads the
/// result and every reason is still listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingOutcome {
    /// Account, RPC or consistency evidence was missing or corrupt, so no
    /// answer was obtained.
    Unverifiable,
    /// A valid Squads proposal outside the bounded one-upgrade shape.
    UnsupportedProposal,
    /// The proposal's message, target or reference is not the analysed spec's.
    DifferentProposal,
    /// The message still matches, but the buffer does not hold the analysed
    /// candidate at the observed slot.
    StaleArtifact,
    /// Message and bytes match, but an authority is not the Squads vault: the
    /// buffer could change without a vote, or the vault could not execute it.
    AuthorityMismatch,
    /// Exact message, target and current buffer bytes match the analysed spec.
    Matched,
}

impl BindingOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unverifiable => "unverifiable",
            Self::UnsupportedProposal => "unsupported_proposal",
            Self::DifferentProposal => "different_proposal",
            Self::StaleArtifact => "stale_artifact",
            Self::AuthorityMismatch => "authority_mismatch",
            Self::Matched => "matched",
        }
    }

    /// `eplyx governance` exit codes. 0 is the only pass. 1 means a binding
    /// was evaluated and the answer is no; 2 that no answer was obtained; 4
    /// that the proposal is outside what this build can bind.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Matched => 0,
            Self::StaleArtifact | Self::DifferentProposal | Self::AuthorityMismatch => 1,
            Self::Unverifiable => 2,
            Self::UnsupportedProposal => 4,
        }
    }
}

/// One reason, with a stable machine code and a sentence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingReason {
    pub outcome: BindingOutcome,
    pub code: String,
    pub detail: String,
}

/// Where the decoder's layouts come from. Recorded, never proved: Eplyx does
/// not claim the deployed Squads program was built from this source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderProvenance {
    pub squads_program_id: String,
    pub source_repository: String,
    pub source_revision: String,
    pub program_crate_version: String,
    pub decoder: String,
    pub message_hash_domain: String,
    pub loader_interface: String,
}

impl DecoderProvenance {
    pub fn current() -> Self {
        Self {
            squads_program_id: squads::SQUADS_V4_PROGRAM_ID.into(),
            source_repository: squads::SOURCE_REPOSITORY.into(),
            source_revision: squads::SOURCE_REVISION.into(),
            program_crate_version: squads::PROGRAM_CRATE_VERSION.into(),
            decoder: squads::DECODER.into(),
            message_hash_domain: squads::MESSAGE_HASH_DOMAIN.into(),
            loader_interface: "solana-loader-v3-interface 8.0.1".into(),
        }
    }
}

/// What the analysed spec says, copied so the evidence reads on its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedUpgrade {
    pub target_program_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub programdata_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_upgrade_authority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<ExecutableArtifact>,
    pub candidate: ExecutableArtifact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<SquadsV4Delivery>,
}

impl ExpectedUpgrade {
    fn of(spec: &ChangeSpec) -> Self {
        let crate::change::Change::ProgramUpgrade {
            target,
            candidate,
            replaces,
            expected_upgrade_authority,
            delivery,
        } = &spec.change;
        Self {
            target_program_id: target.program_id.clone(),
            programdata_address: target.programdata_address.clone(),
            expected_upgrade_authority: expected_upgrade_authority.clone(),
            replaces: replaces.clone(),
            candidate: candidate.clone(),
            delivery: delivery.as_ref().map(|Delivery::SquadsV4(d)| d.clone()),
        }
    }
}

/// Multisig facts that bear on whether a proposal can proceed. Observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultisigState {
    pub threshold: u16,
    pub time_lock_seconds: u32,
    pub transaction_index: u64,
    pub stale_transaction_index: u64,
    pub members: u32,
    /// A controlled multisig's config authority can change members and
    /// threshold without a vote. It cannot execute vault transactions.
    pub config_authority: Option<String>,
}

/// Proposal state. Mutable, observed, and never part of a change's identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalState {
    pub status: squads::ProposalStatusKind,
    pub status_timestamp: Option<i64>,
    pub approvals: u32,
    pub rejections: u32,
    pub cancellations: u32,
    /// `transaction_index <= stale_transaction_index`: Squads refuses new
    /// votes. An already-approved vault transaction can still execute.
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionView {
    pub program_id_index: u8,
    pub account_indexes: Vec<u8>,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupView {
    pub account_key: String,
    pub writable_indexes: Vec<u8>,
    pub readonly_indexes: Vec<u8>,
}

/// The stored message, field for field. What `message_sha256` hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageView {
    pub num_signers: u8,
    pub num_writable_signers: u8,
    pub num_writable_non_signers: u8,
    pub account_keys: Vec<String>,
    pub instructions: Vec<InstructionView>,
    pub address_table_lookups: Vec<LookupView>,
}

impl MessageView {
    fn of(message: &squads::VaultTransactionMessage) -> Self {
        Self {
            num_signers: message.num_signers,
            num_writable_signers: message.num_writable_signers,
            num_writable_non_signers: message.num_writable_non_signers,
            account_keys: message
                .account_keys
                .iter()
                .map(|key| Address::from(*key).to_string())
                .collect(),
            instructions: message
                .instructions
                .iter()
                .map(|ix| InstructionView {
                    program_id_index: ix.program_id_index,
                    account_indexes: ix.account_indexes.clone(),
                    data: ix.data.clone(),
                })
                .collect(),
            address_table_lookups: message
                .address_table_lookups
                .iter()
                .map(|lookup| LookupView {
                    account_key: Address::from(lookup.account_key).to_string(),
                    writable_indexes: lookup.writable_indexes.clone(),
                    readonly_indexes: lookup.readonly_indexes.clone(),
                })
                .collect(),
        }
    }
}

/// The loader `Upgrade` the message would execute, resolved by position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeView {
    pub program: String,
    pub programdata: String,
    pub buffer: String,
    pub spill: String,
    pub authority: String,
}

/// The program as deployed at the observed slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentProgram {
    pub upgrade_authority: Option<String>,
    pub deploy_slot: u64,
    /// ProgramData after its header, padding kept, as `versions` reads it.
    pub executable: ExecutableArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BufferView {
    pub authority: Option<String>,
    /// Every byte `Upgrade` would deploy.
    pub artifact: ExecutableArtifact,
}

/// One account as read, by digest. Provider-neutral: the normalized account,
/// never the provider's JSON, so two providers agreeing produce one digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountDigest {
    pub role: String,
    pub address: String,
    pub present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Decimal string, as every lamport count in Eplyx.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lamports: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_len: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_sha256: Option<String>,
}

impl AccountDigest {
    fn of(role: &str, address: &Address, account: Option<&AccountSnapshot>) -> Self {
        Self {
            role: role.into(),
            address: address.to_string(),
            present: account.is_some(),
            owner: account.map(|a| a.owner.clone()),
            lamports: account.map(|a| a.lamports.to_string()),
            executable: account.map(|a| a.executable),
            data_len: account.map(|a| a.data.len() as u64),
            data_sha256: account.map(|a| hash_bytes(&a.data)),
        }
    }
}

/// Everything read from the chain, at one context slot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SquadsObservation {
    /// Context slot of the single atomic read every fact below comes from.
    /// `None` when no read completed.
    pub slot: Option<u64>,
    /// Context slot of the first read, which located the message.
    pub message_read_slot: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<SquadsV4Delivery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multisig: Option<MultisigState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<ProposalState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<MessageView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade: Option<UpgradeView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_program: Option<CurrentProgram>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer: Option<BufferView>,
    pub accounts: Vec<AccountDigest>,
}

/// The durable, content-addressed result of one binding check.
///
/// `binding_id` is the SHA-256 of `("eplyx-governance-binding-v1", body)`,
/// where the body is this record without its id. It proves the record was not
/// edited after it was written. It does not prove the record true: only
/// re-reading the chain does that, which is why `verify` never consults a
/// stored binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceBinding {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    pub kind: String,
    pub decoder: DecoderProvenance,
    pub commitment: Commitment,
    pub request: SquadsProposalRef,
    /// The spec this check was asked about.
    pub analysed_change_spec_id: String,
    /// The analysed spec delivered by the observed proposal: the identity an
    /// analysis must carry for its verdict to be a verdict about this
    /// proposal. Equal to `analysed_change_spec_id` exactly when the analysed
    /// spec was already bound to this proposal. `None` when the proposal could
    /// not be decoded.
    pub bound_change_spec_id: Option<String>,
    pub expected: ExpectedUpgrade,
    pub observation: SquadsObservation,
    pub outcome: BindingOutcome,
    pub reasons: Vec<BindingReason>,
    pub statement: String,
}

impl GovernanceBinding {
    fn compute_id(&self) -> Result<String> {
        let mut body = self.clone();
        body.binding_id = None;
        Ok(hash_bytes(&serde_json::to_vec(&(BINDING_DOMAIN, &body))?))
    }

    fn seal(mut self) -> Result<Self> {
        self.binding_id = Some(self.compute_id()?);
        Ok(self)
    }

    /// The id, recomputed. A stored record whose id disagrees is refused.
    pub fn id(&self) -> Result<String> {
        self.compute_id()
    }

    /// Parse a stored binding and require it to be exactly what was sealed.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let binding: Self =
            serde_json::from_slice(bytes).context("parsing a governance binding")?;
        ensure!(
            binding.schema_version == BINDING_SCHEMA,
            "governance binding schema {} is not supported",
            binding.schema_version
        );
        let stated = binding
            .binding_id
            .as_deref()
            .context("a stored governance binding must commit to its id")?;
        let derived = binding.compute_id()?;
        ensure!(
            stated == derived,
            "governance binding states id {stated} but its contents identify {derived}"
        );
        ensure!(
            binding.outcome == outcome_of(&binding.reasons),
            "governance binding outcome {} disagrees with its own reasons",
            binding.outcome.as_str()
        );
        Ok(binding)
    }

    pub fn to_document(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// The governance-bound spec, when this check found the analysed change
    /// delivered by a decodable proposal. What an analysis must be run on for
    /// its report to name this proposal.
    pub fn bound_spec(&self, analysed: &ChangeSpec) -> Result<Option<ChangeSpec>> {
        ensure!(
            analysed.id()? == self.analysed_change_spec_id,
            "this binding was not computed for change {}",
            analysed.id()?
        );
        let Some(delivery) = &self.observation.delivery else {
            return Ok(None);
        };
        let bound = analysed.with_delivery(Some(Delivery::SquadsV4(delivery.clone())));
        ensure!(
            Some(bound.id()?) == self.bound_change_spec_id,
            "the binding's bound change id is not the analysed spec delivered by its proposal"
        );
        Ok(Some(bound))
    }
}

fn outcome_of(reasons: &[BindingReason]) -> BindingOutcome {
    reasons
        .iter()
        .map(|reason| reason.outcome)
        .min()
        .unwrap_or(BindingOutcome::Matched)
}

// ---------------------------------------------------------------- the check

struct Reasons(Vec<BindingReason>);

impl Reasons {
    fn push(&mut self, outcome: BindingOutcome, code: &str, detail: impl Into<String>) {
        self.0.push(BindingReason {
            outcome,
            code: code.into(),
            detail: detail.into(),
        });
    }
    fn unverifiable(&mut self, code: &str, detail: impl Into<String>) {
        self.push(BindingOutcome::Unverifiable, code, detail);
    }
    fn unsupported(&mut self, code: &str, detail: impl Into<String>) {
        self.push(BindingOutcome::UnsupportedProposal, code, detail);
    }
    fn different(&mut self, code: &str, detail: impl Into<String>) {
        self.push(BindingOutcome::DifferentProposal, code, detail);
    }
}

/// How often the consistency read waits for a lagging node. Bounded: a
/// provider that never catches up is `unverifiable`, as before.
const MIN_CONTEXT_SLOT_RETRIES: u32 = 5;

/// JSON-RPC -32016, "Minimum context slot has not been reached". The one
/// error a retry can cure; every other failure stays a failure.
fn is_min_context_slot_not_reached(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("-32016") || text.contains("Minimum context slot has not been reached")
}

/// `getMultipleAccounts`: every account at one context slot.
fn read_accounts(
    rpc: &dyn RpcProvider,
    keys: &[Address],
    commitment: Commitment,
    min_context_slot: Option<u64>,
) -> Result<(u64, Vec<Option<AccountSnapshot>>)> {
    let mut config = json!({"encoding": "base64", "commitment": commitment.as_str()});
    if let Some(slot) = min_context_slot {
        config["minContextSlot"] = json!(slot);
    }
    let keys_text: Vec<String> = keys.iter().map(Address::to_string).collect();
    let params = json!([keys_text, config]);
    let mut attempt = 0;
    let response = loop {
        match rpc.call("getMultipleAccounts", params.clone()) {
            // A load-balanced endpoint can route the consistency read to a node
            // that has not reached the first read's slot yet (G1.1 saw this on
            // mainnet). Wait for it; never accept the older view instead.
            Err(error)
                if min_context_slot.is_some()
                    && attempt < MIN_CONTEXT_SLOT_RETRIES
                    && is_min_context_slot_not_reached(&error) =>
            {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(400 * attempt as u64));
            }
            other => break other?,
        }
    };
    let slot = response["context"]["slot"]
        .as_u64()
        .context("getMultipleAccounts response carries no context slot")?;
    if let Some(minimum) = min_context_slot {
        ensure!(
            slot >= minimum,
            "RPC answered at slot {slot}, before the slot {minimum} it was asked not to precede"
        );
    }
    let values = response["value"]
        .as_array()
        .context("getMultipleAccounts response carries no account list")?;
    ensure!(
        values.len() == keys.len(),
        "getMultipleAccounts returned {} accounts for {} keys",
        values.len(),
        keys.len()
    );
    let accounts = values
        .iter()
        .zip(keys)
        .map(|(value, key)| match value {
            Value::Null => Ok(None),
            value => accounts::normalize(value)
                .map(Some)
                .with_context(|| format!("account {key}")),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((slot, accounts))
}

/// A decoded upgrade, or why the message is not one.
fn resolve_upgrade(
    message: &squads::VaultTransactionMessage,
    ephemeral_signers: usize,
    vault: &Address,
    reasons: &mut Reasons,
) -> Option<UpgradeView> {
    let before = reasons.0.len();
    if !message.address_table_lookups.is_empty() {
        reasons.unsupported(
            "address_table_lookups",
            format!(
                "the message loads accounts from {} address lookup table(s), which are resolved \
                 against mutable tables at execution; G1 binds only messages with static keys",
                message.address_table_lookups.len()
            ),
        );
    }
    if ephemeral_signers != 0 {
        reasons.unsupported(
            "ephemeral_signers",
            format!("the transaction declares {ephemeral_signers} ephemeral signer(s); an upgrade needs none"),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    if !message.account_keys.iter().all(|key| seen.insert(*key)) {
        reasons.unsupported(
            "duplicate_account_keys",
            "the message lists an account key twice",
        );
    }
    if message.num_signers != 1 || message.key(0).as_ref() != Some(vault) {
        reasons.unsupported(
            "signer_shape",
            format!(
                "the only signer must be the vault {vault}; the message declares {} signer(s){}",
                message.num_signers,
                message
                    .key(0)
                    .map(|key| format!(", first key {key}"))
                    .unwrap_or_default()
            ),
        );
    }
    if message.instructions.len() != 1 {
        reasons.unsupported(
            "instruction_count",
            format!(
                "the message carries {} instructions; only a message that is exactly one \
                 loader Upgrade is bound",
                message.instructions.len()
            ),
        );
    }
    if reasons.0.len() != before {
        return None;
    }
    let instruction = &message.instructions[0];
    let program = usize::from(instruction.program_id_index);
    if message.key(program) != Some(loader::id()) {
        reasons.unsupported(
            "not_loader_program",
            format!(
                "the instruction calls {}, not the upgradeable loader {}",
                message
                    .key(program)
                    .map(|key| key.to_string())
                    .unwrap_or_else(|| format!("key index {program}")),
                loader::id()
            ),
        );
        return None;
    }
    match loader::decode_instruction(&instruction.data) {
        LoaderInstruction::Upgrade => {}
        LoaderInstruction::UpgradeKeepingBuffer => {
            reasons.unsupported(
                "upgrade_keeps_buffer",
                "Upgrade { close_buffer: false } behaves differently before and after SIMD-0430; \
                 only the legacy form, or close_buffer = true, is bound",
            );
            return None;
        }
        LoaderInstruction::Other(name) => {
            reasons.unsupported(
                "not_upgrade_instruction",
                format!("the loader instruction is {name}, not Upgrade"),
            );
            return None;
        }
        LoaderInstruction::Undecodable => {
            reasons.unsupported(
                "not_upgrade_instruction",
                format!(
                    "the loader instruction data {} is not a canonical Upgrade encoding",
                    crate::hexfmt::encode(&instruction.data)
                ),
            );
            return None;
        }
    }
    let indexes: Vec<usize> = instruction
        .account_indexes
        .iter()
        .map(|i| usize::from(*i))
        .collect();
    let distinct: std::collections::BTreeSet<_> = indexes.iter().collect();
    let key = |position: usize| message.key(indexes[position]);
    let shape_ok = indexes.len() == loader::UPGRADE_ACCOUNTS
        && distinct.len() == indexes.len()
        && indexes.iter().all(|i| *i < message.account_keys.len())
        && [
            loader::UPGRADE_PROGRAMDATA,
            loader::UPGRADE_PROGRAM,
            loader::UPGRADE_BUFFER,
            loader::UPGRADE_SPILL,
        ]
        .iter()
        .all(|p| message.is_static_writable(indexes[*p]) && !message.is_signer(indexes[*p]))
        && [loader::UPGRADE_RENT, loader::UPGRADE_CLOCK]
            .iter()
            .all(|p| !message.is_static_writable(indexes[*p]) && !message.is_signer(indexes[*p]))
        && key(loader::UPGRADE_RENT) == Some(loader::rent_sysvar())
        && key(loader::UPGRADE_CLOCK) == Some(loader::clock_sysvar())
        && indexes[loader::UPGRADE_AUTHORITY] == 0
        && !message.is_static_writable(program)
        && !message.is_signer(program);
    if !shape_ok {
        reasons.unsupported(
            "upgrade_account_shape",
            format!(
                "Upgrade must reference [ProgramData w, Program w, Buffer w, spill w, Rent, Clock, \
                 vault signer] as seven distinct static keys; it references indexes {indexes:?}"
            ),
        );
        return None;
    }
    let mut referenced: std::collections::BTreeSet<usize> = indexes.iter().copied().collect();
    referenced.insert(program);
    if referenced.len() != message.account_keys.len() {
        reasons.unsupported(
            "unreferenced_account_keys",
            format!(
                "the message lists {} keys but the Upgrade uses {}; unused keys are not part of \
                 the bounded shape",
                message.account_keys.len(),
                referenced.len()
            ),
        );
        return None;
    }
    let program_key = key(loader::UPGRADE_PROGRAM)?;
    let programdata = key(loader::UPGRADE_PROGRAMDATA)?;
    if programdata != loader::programdata_address(&program_key) {
        reasons.unsupported(
            "programdata_not_derived",
            format!(
                "ProgramData {programdata} is not the loader's ProgramData for {program_key} ({})",
                loader::programdata_address(&program_key)
            ),
        );
        return None;
    }
    Some(UpgradeView {
        program: program_key.to_string(),
        programdata: programdata.to_string(),
        buffer: key(loader::UPGRADE_BUFFER)?.to_string(),
        spill: key(loader::UPGRADE_SPILL)?.to_string(),
        authority: key(loader::UPGRADE_AUTHORITY)?.to_string(),
    })
}

fn owned_by(account: &AccountSnapshot, owner: &Address) -> bool {
    account.owner == owner.to_string()
}

/// Check the proposal `request` names against the analysed `spec`, reading
/// everything fresh from `rpc`.
///
/// Errors only for a malformed request or spec. Every chain-side problem —
/// an RPC failure, a missing or corrupt account, a PDA that does not derive —
/// is a typed [`BindingOutcome::Unverifiable`] inside the returned binding, so
/// the evidence of *why* is kept. Nothing cached is ever consulted.
pub fn verify_squads_upgrade(
    rpc: &dyn RpcProvider,
    request: &SquadsProposalRef,
    spec: &ChangeSpec,
    commitment: Commitment,
) -> Result<GovernanceBinding> {
    spec.validate()?;
    let multisig: Address = request
        .multisig
        .parse()
        .map_err(|_| anyhow::anyhow!("multisig {:?} is not a base58 address", request.multisig))?;
    ensure!(
        multisig.to_string() == request.multisig,
        "multisig {:?} is not canonically encoded",
        request.multisig
    );
    ensure!(
        request.transaction_index > 0,
        "Squads transaction indexes start at 1"
    );
    let expected = ExpectedUpgrade::of(spec);
    let mut reasons = Reasons(Vec::new());
    let mut observation = SquadsObservation::default();

    let squads_program = squads::program();
    let (transaction_key, transaction_bump) =
        squads::transaction_address(&multisig, request.transaction_index);
    let (proposal_key, proposal_bump) =
        squads::proposal_address(&multisig, request.transaction_index);

    if let Some(bound) = &expected.delivery {
        if bound.multisig != request.multisig
            || bound.transaction_index != request.transaction_index
        {
            reasons.different(
                "proposal_reference_differs",
                format!(
                    "the analysed change is bound to transaction {} of multisig {}, not \
                     transaction {} of multisig {}",
                    bound.transaction_index,
                    bound.multisig,
                    request.transaction_index,
                    request.multisig
                ),
            );
        }
    }

    // ---- read 1: find the message ---------------------------------------
    let squads_keys = [multisig, transaction_key, proposal_key];
    let first = match read_accounts(rpc, &squads_keys, commitment, None) {
        Ok(read) => read,
        Err(error) => {
            reasons.unverifiable("rpc_unavailable", format!("{error:#}"));
            return finish(
                spec,
                request,
                commitment,
                expected,
                observation,
                reasons,
                None,
            );
        }
    };
    observation.message_read_slot = Some(first.0);
    let first_transaction = first.1[1].clone();

    let decoded_transaction = match &first_transaction {
        None => {
            reasons.unverifiable(
                "account_missing",
                format!(
                    "no vault transaction exists at {transaction_key} (transaction {} of {multisig})",
                    request.transaction_index
                ),
            );
            None
        }
        Some(account) if !owned_by(account, &squads_program) => {
            reasons.unverifiable(
                "wrong_owner",
                format!(
                    "vault transaction {transaction_key} is owned by {}, not Squads V4",
                    account.owner
                ),
            );
            None
        }
        Some(account) => match squads::decode_vault_transaction(&account.data) {
            Ok(decoded) => Some(decoded),
            Err(error) => {
                reasons.unverifiable("malformed_account", format!("{error:#}"));
                None
            }
        },
    };

    let mut upgrade = None;
    let mut vault = None;
    if let Some((transaction, message_bytes)) = &decoded_transaction {
        if Address::from(transaction.multisig) != multisig {
            reasons.unverifiable(
                "multisig_mismatch",
                format!(
                    "vault transaction {transaction_key} belongs to multisig {}",
                    Address::from(transaction.multisig)
                ),
            );
        }
        if transaction.index != request.transaction_index {
            reasons.unverifiable(
                "index_mismatch",
                format!(
                    "vault transaction {transaction_key} records index {}",
                    transaction.index
                ),
            );
        }
        if transaction.bump != transaction_bump {
            reasons.unverifiable(
                "pda_mismatch",
                format!(
                    "vault transaction {transaction_key} records bump {}, not its canonical {transaction_bump}",
                    transaction.bump
                ),
            );
        }
        let (canonical_vault, _) = squads::vault_address(&multisig, transaction.vault_index);
        if squads::vault_signer(&multisig, transaction.vault_index, transaction.vault_bump)
            != Some(canonical_vault)
        {
            reasons.unverifiable(
                "pda_mismatch",
                format!(
                    "vault bump {} does not sign as vault {} of {multisig}",
                    transaction.vault_bump, transaction.vault_index
                ),
            );
        }
        vault = Some(canonical_vault);
        let message_sha256 = squads::message_hash_of_bytes(message_bytes);
        let delivery = squads::derive_delivery(
            &multisig,
            transaction.vault_index,
            request.transaction_index,
            message_sha256,
        );
        observation.message = Some(MessageView::of(&transaction.message));
        upgrade = resolve_upgrade(
            &transaction.message,
            transaction.ephemeral_signer_bumps.len(),
            &canonical_vault,
            &mut reasons,
        );
        if let Some(bound) = &expected.delivery {
            if bound.multisig == delivery.multisig
                && bound.transaction_index == delivery.transaction_index
            {
                if bound.vault_index != delivery.vault_index {
                    reasons.different(
                        "vault_differs",
                        format!(
                            "the analysed change names vault {}, the proposal executes as vault {}",
                            bound.vault_index, delivery.vault_index
                        ),
                    );
                }
                if bound.message_sha256 != delivery.message_sha256 {
                    reasons.different(
                        "message_differs",
                        format!(
                            "the proposal's message hashes to {}, the analysed change names {}",
                            delivery.message_sha256, bound.message_sha256
                        ),
                    );
                }
            }
        }
        observation.delivery = Some(delivery);
        observation.upgrade = upgrade.clone();
    }

    // ---- read 2: everything, at one slot ---------------------------------
    let mut keys = squads_keys.to_vec();
    let target_keys = upgrade.as_ref().map(|u| {
        [
            u.program.parse::<Address>().expect("decoded address"),
            u.programdata.parse::<Address>().expect("decoded address"),
            u.buffer.parse::<Address>().expect("decoded address"),
        ]
    });
    if let Some(target) = &target_keys {
        keys.extend_from_slice(target);
    }
    let (slot, accounts) = match read_accounts(rpc, &keys, commitment, Some(first.0)) {
        Ok(read) => read,
        Err(error) => {
            reasons.unverifiable("rpc_unavailable", format!("{error:#}"));
            return finish(
                spec,
                request,
                commitment,
                expected,
                observation,
                reasons,
                None,
            );
        }
    };
    observation.slot = Some(slot);
    let roles = [
        "multisig",
        "vault_transaction",
        "proposal",
        "program",
        "programdata",
        "buffer",
    ];
    observation.accounts = keys
        .iter()
        .zip(&accounts)
        .zip(roles)
        .map(|((key, account), role)| AccountDigest::of(role, key, account.as_ref()))
        .collect();
    if accounts[1].as_ref().map(|a| &a.data) != first_transaction.as_ref().map(|a| &a.data) {
        reasons.unverifiable(
            "inconsistent_read",
            "the vault transaction changed between the two reads; Squads never rewrites a stored message",
        );
    }

    // Multisig: real, at the address asked about, and consistent.
    match &accounts[0] {
        None => reasons.unverifiable(
            "account_missing",
            format!("no multisig exists at {multisig}"),
        ),
        Some(account) if !owned_by(account, &squads_program) => reasons.unverifiable(
            "wrong_owner",
            format!(
                "multisig {multisig} is owned by {}, not Squads V4",
                account.owner
            ),
        ),
        Some(account) => match squads::decode_multisig(&account.data) {
            Err(error) => reasons.unverifiable("malformed_account", format!("{error:#}")),
            Ok(state) => {
                let (derived, bump) = squads::multisig_address(&Address::from(state.create_key));
                if derived != multisig || bump != state.bump {
                    reasons.unverifiable(
                        "pda_mismatch",
                        format!("multisig {multisig} is not the Squads PDA of its own create key"),
                    );
                }
                if state.transaction_index < request.transaction_index {
                    reasons.unverifiable(
                        "index_mismatch",
                        format!(
                            "multisig {multisig} has created {} transactions; transaction {} cannot exist",
                            state.transaction_index, request.transaction_index
                        ),
                    );
                }
                let config_authority = Address::from(state.config_authority);
                observation.multisig = Some(MultisigState {
                    threshold: state.threshold,
                    time_lock_seconds: state.time_lock,
                    transaction_index: state.transaction_index,
                    stale_transaction_index: state.stale_transaction_index,
                    members: state.members.len() as u32,
                    config_authority: (state.config_authority != [0u8; 32])
                        .then(|| config_authority.to_string()),
                });
            }
        },
    }

    // Proposal: exists, belongs to this transaction, and is read for status.
    match &accounts[2] {
        None => reasons.unverifiable(
            "account_missing",
            format!(
                "no proposal exists at {proposal_key} for transaction {}",
                request.transaction_index
            ),
        ),
        Some(account) if !owned_by(account, &squads_program) => reasons.unverifiable(
            "wrong_owner",
            format!(
                "proposal {proposal_key} is owned by {}, not Squads V4",
                account.owner
            ),
        ),
        Some(account) => match squads::decode_proposal(&account.data) {
            Err(error) => reasons.unverifiable("malformed_account", format!("{error:#}")),
            Ok(proposal) => {
                if Address::from(proposal.multisig) != multisig {
                    reasons.unverifiable(
                        "multisig_mismatch",
                        format!(
                            "proposal {proposal_key} belongs to multisig {}",
                            Address::from(proposal.multisig)
                        ),
                    );
                }
                if proposal.transaction_index != request.transaction_index {
                    reasons.unverifiable(
                        "index_mismatch",
                        format!(
                            "proposal {proposal_key} records transaction {}",
                            proposal.transaction_index
                        ),
                    );
                }
                if proposal.bump != proposal_bump {
                    reasons.unverifiable(
                        "pda_mismatch",
                        format!(
                            "proposal {proposal_key} records bump {}, not its canonical {proposal_bump}",
                            proposal.bump
                        ),
                    );
                }
                observation.proposal = Some(ProposalState {
                    status: proposal.status.kind(),
                    status_timestamp: proposal.status.timestamp(),
                    approvals: proposal.approved.len() as u32,
                    rejections: proposal.rejected.len() as u32,
                    cancellations: proposal.cancelled.len() as u32,
                    stale: observation
                        .multisig
                        .as_ref()
                        .is_some_and(|m| request.transaction_index <= m.stale_transaction_index),
                });
            }
        },
    }

    if let (Some(upgrade), Some(vault)) = (&upgrade, vault) {
        let loader_id = loader::id();
        let program_account = &accounts[3];
        let programdata_account = &accounts[4];
        let buffer_account = &accounts[5];

        // The target, against what the spec states.
        if upgrade.program != expected.target_program_id {
            reasons.different(
                "target_program_differs",
                format!(
                    "the proposal upgrades {}, the analysed change targets {}",
                    upgrade.program, expected.target_program_id
                ),
            );
        }
        if let Some(stated) = &expected.programdata_address {
            if &upgrade.programdata != stated {
                reasons.different(
                    "programdata_differs",
                    format!(
                        "the proposal writes ProgramData {}, the analysed change names {stated}",
                        upgrade.programdata
                    ),
                );
            }
        }
        if let Some(stated) = &expected.expected_upgrade_authority {
            if *stated != vault.to_string() {
                reasons.different(
                    "expected_authority_differs",
                    format!(
                        "the analysed change expects upgrade authority {stated}; the proposal \
                         upgrades as vault {vault}"
                    ),
                );
            }
        }

        match program_account {
            Some(account)
                if owned_by(account, &loader_id)
                    && account.executable
                    && loader::decode_program(&account.data)
                        .ok()
                        .map(|a| a.to_string())
                        == Some(upgrade.programdata.clone()) => {}
            Some(_) => reasons.unverifiable(
                "program_malformed",
                format!(
                    "{} is not an upgradeable-loader program whose ProgramData is {}",
                    upgrade.program, upgrade.programdata
                ),
            ),
            None => reasons.unverifiable(
                "account_missing",
                format!("program {} does not exist", upgrade.program),
            ),
        }

        match programdata_account {
            Some(account) if owned_by(account, &loader_id) => {
                match loader::decode_programdata(&account.data) {
                    Ok(programdata) => {
                        let current = ExecutableArtifact::of(&programdata.bytes);
                        if programdata.upgrade_authority != Some(vault) {
                            reasons.push(
                                BindingOutcome::AuthorityMismatch,
                                "upgrade_authority_not_vault",
                                format!(
                                    "{}'s upgrade authority is {}, not the Squads vault {vault}; \
                                     the vault cannot execute this upgrade",
                                    upgrade.program,
                                    programdata
                                        .upgrade_authority
                                        .map(|a| a.to_string())
                                        .unwrap_or_else(|| "none (immutable)".into())
                                ),
                            );
                        }
                        if let Some(replaced) = &expected.replaces {
                            if replaced != &current {
                                reasons.different(
                                    "replaced_executable_differs",
                                    format!(
                                        "the program deployed now is {} ({} bytes), not the {} \
                                         ({} bytes) the analysed change replaces",
                                        current.sha256, current.len, replaced.sha256, replaced.len
                                    ),
                                );
                            }
                        }
                        observation.current_program = Some(CurrentProgram {
                            upgrade_authority: programdata.upgrade_authority.map(|a| a.to_string()),
                            deploy_slot: programdata.deploy_slot,
                            executable: current,
                        });
                    }
                    Err(error) => reasons.unverifiable(
                        "program_malformed",
                        format!("ProgramData {}: {error:#}", upgrade.programdata),
                    ),
                }
            }
            Some(account) => reasons.unverifiable(
                "wrong_owner",
                format!(
                    "ProgramData {} is owned by {}",
                    upgrade.programdata, account.owner
                ),
            ),
            None => reasons.unverifiable(
                "account_missing",
                format!("ProgramData {} does not exist", upgrade.programdata),
            ),
        }

        let executed = observation
            .proposal
            .as_ref()
            .is_some_and(|p| p.status == squads::ProposalStatusKind::Executed);
        match buffer_account {
            None => reasons.unverifiable(
                "buffer_missing",
                if executed {
                    format!(
                        "buffer {} no longer exists: the proposal was executed and Upgrade \
                         consumed it, so its bytes cannot be checked here",
                        upgrade.buffer
                    )
                } else {
                    format!(
                        "buffer {} does not exist; the proposal cannot execute",
                        upgrade.buffer
                    )
                },
            ),
            Some(account) if !owned_by(account, &loader_id) => reasons.unverifiable(
                "buffer_malformed",
                format!(
                    "buffer {} is owned by {}, not the upgradeable loader",
                    upgrade.buffer, account.owner
                ),
            ),
            Some(account) => match loader::decode_buffer(&account.data) {
                Err(error) => reasons.unverifiable(
                    "buffer_malformed",
                    format!("buffer {}: {error:#}", upgrade.buffer),
                ),
                Ok(buffer) => {
                    let artifact = ExecutableArtifact::of(&buffer.bytes);
                    if artifact != expected.candidate {
                        reasons.push(
                            BindingOutcome::StaleArtifact,
                            "candidate_differs",
                            format!(
                                "buffer {} holds {} ({} bytes), not the analysed candidate {} ({} bytes)",
                                upgrade.buffer,
                                artifact.sha256,
                                artifact.len,
                                expected.candidate.sha256,
                                expected.candidate.len
                            ),
                        );
                    } else if !buffer.bytes.starts_with(b"\x7fELF") {
                        reasons.unverifiable(
                            "buffer_not_elf",
                            format!("buffer {} does not hold SBF ELF bytes", upgrade.buffer),
                        );
                    }
                    if buffer.authority != Some(vault) {
                        reasons.push(
                            BindingOutcome::AuthorityMismatch,
                            "buffer_authority_not_vault",
                            format!(
                                "buffer {}'s authority is {}, not the Squads vault {vault}: its \
                                 bytes can be rewritten without any vote",
                                upgrade.buffer,
                                buffer
                                    .authority
                                    .map(|a| a.to_string())
                                    .unwrap_or_else(|| "none".into())
                            ),
                        );
                    }
                    observation.buffer = Some(BufferView {
                        authority: buffer.authority.map(|a| a.to_string()),
                        artifact,
                    });
                }
            },
        }
    }

    let bound = observation
        .delivery
        .as_ref()
        .map(|d| spec.with_delivery(Some(Delivery::SquadsV4(d.clone()))).id())
        .transpose()?;
    finish(
        spec,
        request,
        commitment,
        expected,
        observation,
        reasons,
        bound,
    )
}

/// The analysed spec a proposal implies, from the chain: its target and the
/// bytes its buffer holds *now*, with the bytes returned for the caller to
/// store content-addressed.
///
/// Explicit and separate from [`verify_squads_upgrade`], because it turns
/// chain-acquired bytes into a candidate. It asserts nothing about them: the
/// spec it returns still has to be analysed, and then bound and verified like
/// any other — a buffer rewritten in between is a stale artefact then.
pub fn acquire_squads_candidate(
    rpc: &dyn RpcProvider,
    request: &SquadsProposalRef,
    commitment: Commitment,
) -> Result<(ChangeSpec, Vec<u8>)> {
    let multisig: Address = request
        .multisig
        .parse()
        .map_err(|_| anyhow::anyhow!("multisig {:?} is not a base58 address", request.multisig))?;
    let transaction_key = squads::transaction_address(&multisig, request.transaction_index).0;
    let (_, accounts) = read_accounts(rpc, &[transaction_key], commitment, None)?;
    let account = accounts[0]
        .as_ref()
        .with_context(|| format!("no vault transaction exists at {transaction_key}"))?;
    ensure!(
        owned_by(account, &squads::program()),
        "vault transaction {transaction_key} is not owned by Squads V4"
    );
    let (transaction, _) = squads::decode_vault_transaction(&account.data)?;
    ensure!(
        Address::from(transaction.multisig) == multisig
            && transaction.index == request.transaction_index,
        "vault transaction {transaction_key} is not transaction {} of {multisig}",
        request.transaction_index
    );
    let vault = squads::vault_address(&multisig, transaction.vault_index).0;
    let mut reasons = Reasons(Vec::new());
    let upgrade = resolve_upgrade(
        &transaction.message,
        transaction.ephemeral_signer_bumps.len(),
        &vault,
        &mut reasons,
    );
    let Some(upgrade) = upgrade else {
        let why: Vec<String> = reasons.0.into_iter().map(|r| r.detail).collect();
        anyhow::bail!(
            "the proposal is not a bounded program upgrade: {}",
            why.join("; ")
        );
    };
    let buffer_key: Address = upgrade.buffer.parse()?;
    let (_, accounts) = read_accounts(rpc, &[buffer_key], commitment, None)?;
    let account = accounts[0]
        .as_ref()
        .with_context(|| format!("buffer {buffer_key} does not exist"))?;
    ensure!(
        owned_by(account, &loader::id()),
        "buffer {buffer_key} is not owned by the upgradeable loader"
    );
    let buffer = loader::decode_buffer(&account.data)?;
    ensure!(
        buffer.bytes.starts_with(b"\x7fELF"),
        "buffer {buffer_key} does not hold SBF ELF bytes"
    );
    Ok((
        ChangeSpec::program_upgrade(&upgrade.program, &buffer.bytes),
        buffer.bytes,
    ))
}

fn finish(
    spec: &ChangeSpec,
    request: &SquadsProposalRef,
    commitment: Commitment,
    expected: ExpectedUpgrade,
    observation: SquadsObservation,
    reasons: Reasons,
    bound_change_spec_id: Option<String>,
) -> Result<GovernanceBinding> {
    let outcome = outcome_of(&reasons.0);
    let mut binding = GovernanceBinding {
        schema_version: BINDING_SCHEMA,
        binding_id: None,
        kind: BINDING_KIND.into(),
        decoder: DecoderProvenance::current(),
        commitment,
        request: request.clone(),
        analysed_change_spec_id: spec.id()?,
        bound_change_spec_id,
        expected,
        observation,
        outcome,
        reasons: reasons.0,
        statement: String::new(),
    };
    binding.statement = statement(&binding);
    binding.seal()
}

/// The sentence every surface shows. Always names the slot; never claims the
/// proposal cannot diverge later.
pub fn statement(binding: &GovernanceBinding) -> String {
    let proposal = format!(
        "Squads transaction #{} of multisig {}",
        binding.request.transaction_index, binding.request.multisig
    );
    let at = match binding.observation.slot {
        Some(slot) => format!("at slot {slot} ({})", binding.commitment.as_str()),
        None => "(no chain read completed)".to_string(),
    };
    let status = binding
        .observation
        .proposal
        .as_ref()
        .map(|p| {
            let mut text = format!(" Proposal status: {}", status_word(p.status));
            if p.stale {
                text.push_str(", stale (no further votes accepted)");
            }
            text.push('.');
            text
        })
        .unwrap_or_default();
    let head = binding
        .reasons
        .iter()
        .find(|reason| reason.outcome == binding.outcome)
        .map(|reason| reason.detail.trim_end_matches('.'))
        .unwrap_or("");
    match binding.outcome {
        BindingOutcome::Matched => format!(
            "{proposal} matched analysed change {} {at}: the same program upgrade, and its buffer \
             held the analysed candidate {}. The buffer's authority is the Squads vault, so its \
             bytes can change again only through another transaction that vault executes. \
             Re-verify immediately before approving or executing.{status}",
            binding
                .bound_change_spec_id
                .as_deref()
                .unwrap_or(&binding.analysed_change_spec_id),
            binding.expected.candidate.sha256,
        ),
        BindingOutcome::StaleArtifact => format!(
            "{proposal} still names the analysed upgrade, but {at} its buffer no longer matches the \
             candidate Eplyx analysed: {head}.{status}"
        ),
        BindingOutcome::AuthorityMismatch => format!(
            "{proposal} matches the analysed upgrade {at}, but is not bound: {head}.{status}"
        ),
        BindingOutcome::DifferentProposal => {
            format!("{proposal} is not the analysed change {at}: {head}.{status}")
        }
        BindingOutcome::UnsupportedProposal => format!(
            "{proposal}, read {at}, is outside what Eplyx can bind (exactly one loader Upgrade, no \
             lookup tables): {head}.{status}"
        ),
        BindingOutcome::Unverifiable => {
            format!("Eplyx could not verify {proposal} {at}: {head}.{status}")
        }
    }
}

fn status_word(status: squads::ProposalStatusKind) -> &'static str {
    use squads::ProposalStatusKind::*;
    match status {
        Draft => "Draft",
        Active => "Active",
        Rejected => "Rejected",
        Approved => "Approved",
        Executing => "Executing",
        Executed => "Executed",
        Cancelled => "Cancelled",
    }
}

#[cfg(test)]
mod tests;
