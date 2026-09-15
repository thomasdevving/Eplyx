//! Durable, offline replay and fidelity gate. RPC is never called here.
use crate::{
    executor::{execute_in_environment, ExecutionResult, ProgramVersion},
    ingest::transactions::HistoricalTransaction,
    types::{AccountSnapshot, Category, Fixture, NamedAccount},
    Report,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_clock::Clock;
use solana_message::{compiled_instruction::CompiledInstruction, Message, MessageHeader};
use std::collections::{BTreeMap, BTreeSet};

pub const REPLAY_SCHEMA: u32 = 1;
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
    pub assumptions: Vec<String>,
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    crate::hexfmt::encode(&Sha256::digest(bytes))
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
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.schema_version == REPLAY_SCHEMA,
            "unsupported replay schema"
        );
        anyhow::ensure!(
            self.program_id == crate::fixture_program_id().to_string(),
            "only the controlled lending program is supported for replay"
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
        anyhow::ensure!(
            self.transaction.version == "legacy",
            "v0 is normalized but execution is not yet supported"
        );
        anyhow::ensure!(
            self.transaction.instructions.len() == 1
                && self.transaction.instructions[0].program == self.program_id,
            "replay currently requires one top-level controlled-program instruction"
        );
        anyhow::ensure!(
            self.transaction.inner_instructions.is_empty(),
            "controlled exact replay currently excludes CPI transactions"
        );
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
        for key in &self.transaction.account_keys {
            if key.address != self.program_id {
                anyhow::ensure!(
                    addresses.contains(key.address.as_str()),
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
                !account.account.executable && account.address != self.program_id,
                "executable accounts must be supplied by pinned runtime/program binary"
            );
        }
        let fixture = self.fixture();
        anyhow::ensure!(
            fixture
                .account("position")
                .and_then(|a| crate::interpret::position_economics(&a.account.data))
                .is_some(),
            "missing controlled position valuation"
        );
        self.message()?;
        Ok(())
    }
    pub fn fixture(&self) -> Fixture {
        Fixture {
            id: self.id.clone(),
            category: Category::Boundary,
            scenario: "historical controlled interaction".into(),
            notes: "Capital represents replay observations, not unique positions or deployed TVL"
                .into(),
            keypairs: vec![],
            accounts: self.accounts.clone(),
            fee_payer: self.transaction.payer.clone(),
            signers: vec![],
            instruction: self.transaction.instructions[0].clone(),
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
    pub fn execute(&self, program: &ProgramVersion) -> Result<ExecutionResult> {
        self.validate()?;
        execute_in_environment(
            &self.fixture(),
            &self.program_id.parse()?,
            program,
            self.clock.clock(),
            Some(self.message()?),
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
        state_hash(&accounts)
    }
    pub fn fidelity(&self, result: &ExecutionResult) -> Result<ReplayFidelity> {
        if self.state_source == ReplayStateSource::CurrentApproximation {
            return Ok(ReplayFidelity::Approximate);
        }
        let Some(original) = &self.original else {
            return Ok(ReplayFidelity::Unknown);
        };
        if result.success != original.success
            || result.fee != original.fee
            || self.post_hash(result)? != original.post_state_hash
        {
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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayReport {
    pub schema_version: u32,
    pub observations: Vec<ReplayObservation>,
    pub analysis: Report,
}

pub fn compare(
    records: &[ReplayRecord],
    v1: &ProgramVersion,
    v2: &ProgramVersion,
) -> Result<ReplayReport> {
    anyhow::ensure!(!records.is_empty(), "empty replay corpus");
    let mut ids = BTreeSet::new();
    let mut fixtures = Vec::new();
    let mut diffs = Vec::new();
    let mut observations = Vec::new();
    for record in records {
        anyhow::ensure!(ids.insert(&record.id), "duplicate replay ID");
        anyhow::ensure!(
            record.current_program_sha256 == hash_bytes(&v1.bytes),
            "V1 binary differs from captured program"
        );
        let original = record
            .execute(v1)
            .with_context(|| format!("V1 replay {}", record.id))?;
        let fidelity = record.fidelity(&original)?;
        anyhow::ensure!(
            matches!(fidelity, ReplayFidelity::Exact | ReplayFidelity::Matched),
            "replay {} fidelity {:?}; candidate execution withheld",
            record.id,
            fidelity
        );
        let candidate = record.execute(v2)?;
        observations.push(ReplayObservation {
            id: record.id.clone(),
            source_signature: record.transaction.signature.clone(),
            source_slot: record.transaction.slot,
            state_source: record.state_source.clone(),
            fidelity,
            pre_state_hash: record.pre_state_hash.clone(),
            post_v1_state_hash: record.post_hash(&original)?,
            post_v2_state_hash: record.post_hash(&candidate)?,
        });
        let fixture = record.fixture();
        diffs.push(crate::diff::compare(&fixture, original, candidate));
        fixtures.push(fixture);
    }
    Ok(ReplayReport {
        schema_version: REPLAY_SCHEMA,
        observations,
        analysis: Report::new(
            records[0].program_id.clone(),
            v1.label.clone(),
            v2.label.clone(),
            &fixtures,
            diffs,
        ),
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

/// State snapshots in external captures are keyed by public address.
pub type AccountMap = BTreeMap<String, AccountSnapshot>;
