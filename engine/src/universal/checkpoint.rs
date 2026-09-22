//! Generic evidence and validation for checkpoint-derived execution proofs.
//!
//! A checkpoint profile does not claim that every transaction write was
//! observed.  It starts from an observed predecessor checkpoint, executes a
//! proven message, and reconciles the derived outputs with an independently
//! observed terminal checkpoint.  The closure proof below establishes why no
//! other committed transaction can account for those outputs.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    evidence::{AccountBoundary, AccountObservation, EvidenceKind, EvidenceRef, EvidenceStore},
    model::{ReplayObservationV2, ResolvedMessage},
};
use crate::types::AccountSnapshot;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointAccount {
    pub address: String,
    pub observation: EvidenceRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedCheckpointV1 {
    pub schema_version: u32,
    pub slot: u64,
    pub accounts: Vec<CheckpointAccount>,
}

impl ObservedCheckpointV1 {
    pub fn store(&self, store: &EvidenceStore) -> Result<EvidenceRef> {
        ensure!(self.schema_version == 1, "wrong checkpoint schema");
        store.put(EvidenceKind::Checkpoint, &serde_json::to_vec(self)?)
    }

    pub fn resolve(
        store: &EvidenceStore,
        reference: &EvidenceRef,
        transaction_slot: u64,
        boundary: AccountBoundary,
        genesis: &str,
    ) -> Result<BTreeMap<String, Option<AccountSnapshot>>> {
        ensure!(
            reference.kind == EvidenceKind::Checkpoint,
            "checkpoint reference has wrong evidence category"
        );
        let checkpoint: Self = serde_json::from_slice(&store.get(reference)?)?;
        ensure!(checkpoint.schema_version == 1, "wrong checkpoint schema");
        let expected_slot = match boundary {
            AccountBoundary::BeforeTransaction => transaction_slot
                .checked_sub(1)
                .context("transaction has no predecessor slot")?,
            AccountBoundary::EndOfExecutionSlot | AccountBoundary::BeforeTargetExecution => {
                transaction_slot
            }
        };
        ensure!(
            checkpoint.slot == expected_slot,
            "checkpoint uses the wrong slot"
        );
        let mut accounts = BTreeMap::new();
        for entry in &checkpoint.accounts {
            ensure!(
                accounts
                    .insert(
                        entry.address.clone(),
                        AccountObservation::resolve(
                            store,
                            &entry.observation,
                            &entry.address,
                            transaction_slot,
                            boundary,
                            genesis,
                        )?,
                    )
                    .is_none(),
                "checkpoint contains a duplicate account"
            );
        }
        ensure!(!accounts.is_empty(), "checkpoint contains no accounts");
        Ok(accounts)
    }
}

/// An account image derived from retained validator/runtime evidence rather
/// than represented as a directly observed point-in-time RPC account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivedAccountEvidenceV1 {
    pub schema_version: u32,
    pub address: String,
    pub transaction_slot: u64,
    pub boundary: AccountBoundary,
    pub content: EvidenceRef,
    pub sources: Vec<EvidenceRef>,
    pub derivation: String,
}

impl DerivedAccountEvidenceV1 {
    pub fn store(&self, store: &EvidenceStore) -> Result<EvidenceRef> {
        ensure!(self.schema_version == 1, "wrong derived-account schema");
        store.put(EvidenceKind::DerivedAccount, &serde_json::to_vec(self)?)
    }

    pub fn resolve(
        store: &EvidenceStore,
        reference: &EvidenceRef,
        address: &str,
        transaction_slot: u64,
        boundary: AccountBoundary,
    ) -> Result<AccountSnapshot> {
        ensure!(
            reference.kind == EvidenceKind::DerivedAccount,
            "derived account reference has wrong evidence category"
        );
        let evidence: Self = serde_json::from_slice(&store.get(reference)?)?;
        ensure!(
            evidence.schema_version == 1
                && evidence.address == address
                && evidence.transaction_slot == transaction_slot
                && evidence.boundary == boundary,
            "derived account identity differs"
        );
        ensure!(
            evidence.content.kind == EvidenceKind::AccountContent,
            "derived account content has wrong evidence category"
        );
        ensure!(
            !evidence.sources.is_empty() && !evidence.derivation.is_empty(),
            "derived account provenance is incomplete"
        );
        for source in &evidence.sources {
            store.get(source)?;
        }
        Ok(serde_json::from_slice(&store.get(&evidence.content)?)?)
    }
}

/// Reconstructive account evidence. The enum, source references and result are
/// all part of the content-addressed evidence object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccountDerivationV2 {
    YellowstoneAccountImageV1 {
        event: EvidenceRef,
    },
    RecentBlockhashesPreTargetV1 {
        event: EvidenceRef,
        tail_block: EvidenceRef,
        acquisition_receipt: EvidenceRef,
        tail_slot: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivedAccountEvidenceV2 {
    pub schema_version: u32,
    pub address: String,
    pub transaction_slot: u64,
    pub boundary: AccountBoundary,
    pub content: EvidenceRef,
    pub derivation: AccountDerivationV2,
}

fn protobuf_varint(bytes: &[u8], offset: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*offset).context("truncated Yellowstone varint")?;
        *offset += 1;
        ensure!(shift != 63 || byte <= 1, "Yellowstone varint overflow");
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    anyhow::bail!("Yellowstone varint too long")
}

fn protobuf_field_optional(bytes: &[u8], wanted: u64) -> Result<Option<Vec<u8>>> {
    let mut offset = 0;
    let mut found = None;
    while offset < bytes.len() {
        let tag = protobuf_varint(bytes, &mut offset)?;
        let wire = tag & 7;
        let value = match wire {
            0 => {
                let start = offset;
                protobuf_varint(bytes, &mut offset)?;
                &bytes[start..offset]
            }
            1 => {
                let end = offset.checked_add(8).context("protobuf length overflow")?;
                let v = bytes
                    .get(offset..end)
                    .context("truncated protobuf fixed64")?;
                offset = end;
                v
            }
            2 => {
                let length = usize::try_from(protobuf_varint(bytes, &mut offset)?)?;
                let end = offset
                    .checked_add(length)
                    .context("protobuf length overflow")?;
                let v = bytes.get(offset..end).context("truncated protobuf bytes")?;
                offset = end;
                v
            }
            5 => {
                let end = offset.checked_add(4).context("protobuf length overflow")?;
                let v = bytes
                    .get(offset..end)
                    .context("truncated protobuf fixed32")?;
                offset = end;
                v
            }
            _ => anyhow::bail!("unsupported Yellowstone protobuf wire type"),
        };
        if tag >> 3 == wanted {
            ensure!(found.is_none(), "duplicate Yellowstone field");
            found = Some(value.to_vec());
        }
    }
    Ok(found)
}

fn protobuf_field(bytes: &[u8], wanted: u64) -> Result<Vec<u8>> {
    protobuf_field_optional(bytes, wanted)?
        .with_context(|| format!("Yellowstone field {wanted} missing"))
}

fn protobuf_number(bytes: &[u8], wanted: u64) -> Result<u64> {
    let raw = protobuf_field(bytes, wanted)?;
    let mut offset = 0;
    let value = protobuf_varint(&raw, &mut offset)?;
    ensure!(offset == raw.len(), "invalid Yellowstone numeric field");
    Ok(value)
}

fn yellowstone_account(
    store: &EvidenceStore,
    source: &EvidenceRef,
    address: &str,
    slot: u64,
) -> Result<AccountSnapshot> {
    ensure!(
        source.kind == EvidenceKind::Validator,
        "Yellowstone source category differs"
    );
    let frame = store.get(source)?;
    let update = protobuf_field(&frame, 2)?;
    ensure!(
        protobuf_number(&update, 2)? == slot,
        "Yellowstone event slot differs"
    );
    let info = protobuf_field(&update, 1)?;
    ensure!(
        protobuf_field_optional(&info, 8)?.is_none(),
        "runtime account event unexpectedly carries a transaction signature"
    );
    let key = protobuf_field(&info, 1)?;
    ensure!(
        key.len() == 32 && bs58::encode(key).into_string() == address,
        "Yellowstone event address differs"
    );
    let owner = protobuf_field(&info, 3)?;
    ensure!(owner.len() == 32, "Yellowstone event owner length differs");
    let executable = match protobuf_field_optional(&info, 4)? {
        Some(raw) => {
            let mut offset = 0;
            let value = protobuf_varint(&raw, &mut offset)?;
            ensure!(offset == raw.len(), "invalid Yellowstone executable field");
            value
        }
        None => 0,
    };
    ensure!(executable <= 1, "Yellowstone executable flag differs");
    Ok(AccountSnapshot {
        owner: bs58::encode(owner).into_string(),
        lamports: protobuf_number(&info, 2)?,
        data: protobuf_field(&info, 6)?,
        executable: executable == 1,
        rent_epoch: protobuf_number(&info, 5)?,
    })
}

impl DerivedAccountEvidenceV2 {
    pub fn store(&self, store: &EvidenceStore) -> Result<EvidenceRef> {
        ensure!(
            self.schema_version == 2,
            "wrong reconstructive derived-account schema"
        );
        store.put(EvidenceKind::DerivedAccount, &serde_json::to_vec(self)?)
    }

    pub fn resolve(
        store: &EvidenceStore,
        reference: &EvidenceRef,
        address: &str,
        slot: u64,
        boundary: AccountBoundary,
    ) -> Result<AccountSnapshot> {
        ensure!(
            reference.kind == EvidenceKind::DerivedAccount,
            "derived account reference has wrong evidence category"
        );
        let evidence: Self = serde_json::from_slice(&store.get(reference)?)?;
        ensure!(
            evidence.schema_version == 2
                && evidence.address == address
                && evidence.transaction_slot == slot
                && evidence.boundary == boundary
                && boundary == AccountBoundary::BeforeTargetExecution,
            "reconstructive derived account identity differs"
        );
        ensure!(
            evidence.content.kind == EvidenceKind::AccountContent,
            "derived account content has wrong category"
        );
        let result = match &evidence.derivation {
            AccountDerivationV2::YellowstoneAccountImageV1 { event } => {
                ensure!(
                    address == "SysvarS1otHashes111111111111111111111111111",
                    "direct-event derivation has unsupported runtime address"
                );
                let account = yellowstone_account(store, event, address, slot)?;
                ensure!(
                    account.data.len() == 20_488
                        && u64::from_le_bytes(account.data[..8].try_into()?) == 512
                        && u64::from_le_bytes(account.data[8..16].try_into()?)
                            == slot
                                .checked_sub(1)
                                .context("slot-hashes predecessor missing")?,
                    "slot-hashes event layout or head differs"
                );
                account
            }
            AccountDerivationV2::RecentBlockhashesPreTargetV1 {
                event,
                tail_block,
                acquisition_receipt,
                tail_slot,
            } => {
                ensure!(
                    address == "SysvarRecentB1ockHashes11111111111111111111",
                    "recent-blockhash derivation has wrong sysvar address"
                );
                ensure!(
                    *tail_slot < slot
                        && tail_block.kind == EvidenceKind::Validator
                        && acquisition_receipt.kind == EvidenceKind::Validator,
                    "recent-blockhash source category or slot differs"
                );
                let mut account = yellowstone_account(store, event, address, slot)?;
                ensure!(
                    account.data.len() == 6008
                        && u64::from_le_bytes(account.data[..8].try_into()?) == 150,
                    "recent-blockhash event layout differs"
                );
                let tail_bytes = store.get(tail_block)?;
                let tail: Value = serde_json::from_slice(&tail_bytes)?;
                let receipt: Value = serde_json::from_slice(&store.get(acquisition_receipt)?)?;
                ensure!(
                    receipt["receipts"]
                        .as_array()
                        .is_some_and(|rows| rows.iter().any(|row| row["method"] == "getBlock"
                            && row["params"][0].as_u64() == Some(*tail_slot)
                            && row["params"][1]["commitment"] == "finalized"
                            && row["http_status"].as_u64() == Some(200)
                            && row["rpc_error"].is_null()
                            && row["transport_error"].is_null()
                            && row["response_sha256"].as_str()
                                == Some(tail_block.sha256.as_str()))),
                    "tail block has no matching retained acquisition receipt"
                );
                let block = &tail["result"];
                ensure!(
                    block["parentSlot"].as_u64() == tail_slot.checked_sub(1)
                        && receipt["recent_blockhash_tail_anchor"]["slot"].as_u64()
                            == Some(*tail_slot)
                        && receipt["recent_blockhash_tail_anchor"]["blockhash"]
                            == block["blockhash"]
                        && receipt["recent_blockhash_tail_anchor"]["previous_blockhash"]
                            == block["previousBlockhash"],
                    "recent-blockhash tail parent relationship differs"
                );
                let tail_hash = bs58::decode(
                    block["blockhash"]
                        .as_str()
                        .context("tail blockhash missing")?,
                )
                .into_vec()?;
                let predecessor = bs58::decode(
                    block["previousBlockhash"]
                        .as_str()
                        .context("tail previous blockhash missing")?,
                )
                .into_vec()?;
                ensure!(
                    tail_hash.len() == 32
                        && predecessor.len() == 32
                        && account.data[8 + 149 * 40..8 + 149 * 40 + 32] == tail_hash,
                    "recent-blockhash tail link differs"
                );
                let fee = account.data[8 + 149 * 40 + 32..].to_vec();
                let mut data = account.data[..8].to_vec();
                data.extend_from_slice(&account.data[8 + 40..8 + 150 * 40]);
                data.extend_from_slice(&predecessor);
                data.extend_from_slice(&fee);
                account.data = data;
                account
            }
        };
        let stored: AccountSnapshot = serde_json::from_slice(&store.get(&evidence.content)?)?;
        ensure!(
            result == stored,
            "derived account content differs from reconstructed sources"
        );
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailedTransactionRollbackRule {
    FeePayerAndDurableNonceOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailedOverlap {
    pub transaction_index: usize,
    pub signature: String,
    pub fee_payer: String,
    pub durable_nonce_accounts: Vec<String>,
}

/// Minimal protocol-neutral closure facts.  The account-mode block is parsed
/// again during every resolution; these fields select the target and retain
/// the failed-transaction rollback facts that account-mode blocks cannot
/// derive by themselves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionClosureProofV1 {
    pub schema_version: u32,
    pub slot: u64,
    pub parent_slot: u64,
    pub blockhash: String,
    pub transaction_count: usize,
    pub target_transaction_index: usize,
    pub target_signature: String,
    pub execution_inputs: Vec<String>,
    pub validation_outputs: Vec<String>,
    pub failed_overlaps: Vec<FailedOverlap>,
    pub rollback_rule: FailedTransactionRollbackRule,
    pub block_accounts_evidence: EvidenceRef,
    pub conflict_census_evidence: EvidenceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_block_evidence: Option<EvidenceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_provenance: Option<RollbackProvenanceV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackProvenanceV1 {
    pub semantics_version: u32,
    pub upstream_commit: String,
    pub runtime_evidence: EvidenceRef,
    pub source_provenance: EvidenceRef,
    pub rollback_source: EvidenceRef,
    pub transaction_processor_source: EvidenceRef,
}

impl RollbackProvenanceV1 {
    fn verify(&self, store: &EvidenceStore, record: &ReplayObservationV2) -> Result<()> {
        ensure!(
            self.semantics_version == 1
                && self.upstream_commit == "965aee8e55d45ac3ca72e48f15945bfca4a32804",
            "unsupported rollback semantics provenance"
        );
        ensure!(
            self.runtime_evidence.kind == EvidenceKind::Runtime
                && record.runtime.historical_evidence_ref.as_ref() == Some(&self.runtime_evidence),
            "rollback runtime provenance differs"
        );
        store.get(&self.runtime_evidence)?;
        for (reference, hash) in [
            (
                &self.source_provenance,
                "abdbcc9f67430cafde752a4908c86696a7921db689e7a7ccf858e7a66257205c",
            ),
            (
                &self.rollback_source,
                "05fc22177dd28df5cc1708cb968ca88e77c5ea4bce4e6e23852c2f76ae440c52",
            ),
            (
                &self.transaction_processor_source,
                "a41d5b466b13c0aa2f63a42d05ded092ad8ef8399109bb85d15546f7cdc09509",
            ),
        ] {
            ensure!(
                reference.kind == EvidenceKind::Validator && reference.sha256 == hash,
                "rollback source provenance differs"
            );
            store.get(reference)?;
        }
        let manifest: Value = serde_json::from_slice(&store.get(&self.source_provenance)?)?;
        ensure!(
            manifest["commit"].as_str() == Some(self.upstream_commit.as_str())
                && manifest["files"].as_array().is_some_and(|files| files
                    .iter()
                    .any(|file| file["sha256"].as_str()
                        == Some(self.rollback_source.sha256.as_str()))
                    && files.iter().any(|file| file["sha256"].as_str()
                        == Some(self.transaction_processor_source.sha256.as_str()))),
            "rollback source manifest differs"
        );
        Ok(())
    }
}

fn full_account_keys(transaction: &Value) -> Result<Vec<String>> {
    let mut keys = transaction["transaction"]["message"]["accountKeys"]
        .as_array()
        .context("full block static keys missing")?
        .iter()
        .map(|key| {
            key.as_str()
                .context("full block static key invalid")
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>>>()?;
    let loaded = &transaction["meta"]["loadedAddresses"];
    for class in ["writable", "readonly"] {
        for key in loaded[class]
            .as_array()
            .context("full block loaded addresses missing")?
        {
            keys.push(
                key.as_str()
                    .context("full block loaded address invalid")?
                    .to_owned(),
            );
        }
    }
    Ok(keys)
}

fn full_durable_nonce_accounts(transaction: &Value, keys: &[String]) -> Result<Vec<String>> {
    let message = &transaction["transaction"]["message"];
    let first = message["instructions"]
        .as_array()
        .context("full block instructions missing")?
        .first();
    let Some(first) = first else {
        return Ok(Vec::new());
    };
    let program_index = usize::try_from(
        first["programIdIndex"]
            .as_u64()
            .context("full block instruction program missing")?,
    )?;
    let program = keys
        .get(program_index)
        .context("full block program index out of range")?;
    let data = bs58::decode(
        first["data"]
            .as_str()
            .context("full block instruction data missing")?,
    )
    .into_vec()?;
    if program != "11111111111111111111111111111111" || data != [4, 0, 0, 0] {
        return Ok(Vec::new());
    }
    let accounts = first["accounts"]
        .as_array()
        .context("nonce instruction accounts missing")?;
    ensure!(
        accounts.len() == 3,
        "nonce instruction account count differs"
    );
    let index = usize::try_from(
        accounts[0]
            .as_u64()
            .context("nonce account index missing")?,
    )?;
    Ok(vec![keys
        .get(index)
        .context("nonce account index out of range")?
        .clone()])
}

fn account_keys(transaction: &Value) -> Result<Vec<(String, bool)>> {
    transaction["transaction"]["accountKeys"]
        .as_array()
        .context("closure transaction account keys missing")?
        .iter()
        .map(|key| {
            Ok((
                key["pubkey"]
                    .as_str()
                    .context("closure account address missing")?
                    .to_string(),
                key["writable"]
                    .as_bool()
                    .context("closure account writable flag missing")?,
            ))
        })
        .collect()
}

fn signature(transaction: &Value) -> Result<&str> {
    transaction["transaction"]["signatures"][0]
        .as_str()
        .context("closure transaction signature missing")
}

fn successful(transaction: &Value) -> Result<bool> {
    Ok(transaction["meta"]["err"].is_null())
}

impl TransactionClosureProofV1 {
    pub fn store(&self, store: &EvidenceStore) -> Result<EvidenceRef> {
        ensure!(
            self.schema_version == 1 || self.schema_version == 2,
            "wrong closure proof schema"
        );
        store.put(EvidenceKind::ClosureProof, &serde_json::to_vec(self)?)
    }

    pub fn verify(
        store: &EvidenceStore,
        reference: &EvidenceRef,
        record: &ReplayObservationV2,
        message: &ResolvedMessage,
        validation_outputs: &[String],
        checkpoint_contract: u32,
    ) -> Result<()> {
        ensure!(
            reference.kind == EvidenceKind::ClosureProof,
            "closure proof reference has wrong evidence category"
        );
        let proof: Self = serde_json::from_slice(&store.get(reference)?)?;
        ensure!(
            proof.schema_version == checkpoint_contract,
            "closure proof contract differs"
        );
        let full_block = if checkpoint_contract == 2 {
            ensure!(
                proof.rollback_rule == FailedTransactionRollbackRule::FeePayerAndDurableNonceOnly,
                "unsupported failed-transaction rollback rule"
            );
            proof
                .rollback_provenance
                .as_ref()
                .context("rollback semantics provenance missing")?
                .verify(store, record)?;
            let reference = proof
                .full_block_evidence
                .as_ref()
                .context("full transaction evidence missing")?;
            ensure!(
                reference.kind == EvidenceKind::Validator,
                "full transaction evidence category differs"
            );
            Some(serde_json::from_slice::<Value>(&store.get(reference)?)?)
        } else {
            None
        };
        ensure!(
            proof.slot == record.slot
                && proof.target_signature == record.signature
                && proof.execution_inputs == message.account_keys
                && proof.validation_outputs == validation_outputs,
            "closure target, inputs, or validation outputs differ"
        );
        let input_set = proof
            .execution_inputs
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        ensure!(
            input_set.len() == proof.execution_inputs.len(),
            "closure execution inputs are not unique"
        );
        let output_set = proof
            .validation_outputs
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        ensure!(
            output_set.len() == proof.validation_outputs.len() && output_set.is_subset(&input_set),
            "closure validation output set is invalid"
        );

        ensure!(
            proof.block_accounts_evidence.kind == EvidenceKind::Validator
                && proof.conflict_census_evidence.kind == EvidenceKind::Validator,
            "closure source evidence has wrong category"
        );
        let block: Value = serde_json::from_slice(&store.get(&proof.block_accounts_evidence)?)?;
        let result = &block["result"];
        let transactions = result["transactions"]
            .as_array()
            .context("closure block transactions missing")?;
        ensure!(
            result["parentSlot"].as_u64() == Some(proof.parent_slot)
                && result["blockhash"].as_str() == Some(proof.blockhash.as_str())
                && transactions.len() == proof.transaction_count
                && proof.target_transaction_index < transactions.len(),
            "closure block identity differs"
        );
        let target = &transactions[proof.target_transaction_index];
        let full_transactions = if let Some(full) = &full_block {
            let body = &full["result"];
            let rows = body["transactions"]
                .as_array()
                .context("full block transactions missing")?;
            ensure!(
                body["parentSlot"].as_u64() == Some(proof.parent_slot)
                    && body["blockhash"].as_str() == Some(proof.blockhash.as_str())
                    && rows.len() == transactions.len(),
                "full block identity differs"
            );
            for (parsed, complete) in transactions.iter().zip(rows) {
                ensure!(
                    signature(parsed)? == signature(complete)?,
                    "full block transaction order differs"
                );
            }
            Some(rows)
        } else {
            None
        };
        ensure!(
            signature(target)? == proof.target_signature
                && account_keys(target)?
                    .into_iter()
                    .map(|(address, _)| address)
                    .collect::<Vec<_>>()
                    == proof.execution_inputs,
            "closure target transaction differs"
        );

        let census: Value = serde_json::from_slice(&store.get(&proof.conflict_census_evidence)?)?;
        ensure!(
            census["slot"].as_u64() == Some(proof.slot)
                && census["target_transaction_index"].as_u64()
                    == Some(proof.target_transaction_index as u64)
                && census["target_signature"].as_str() == Some(proof.target_signature.as_str()),
            "closure conflict census target differs"
        );
        let census_accounts = census["accounts"]
            .as_array()
            .context("closure conflict census accounts missing")?;
        for address in &proof.execution_inputs {
            let row = census_accounts
                .iter()
                .find(|row| row["address"].as_str() == Some(address))
                .with_context(|| format!("closure census omitted execution input {address}"))?;
            ensure!(
                row["earlier_writable_transaction_indexes"]
                    .as_array()
                    .is_some_and(Vec::is_empty),
                "an earlier transaction can modify a target input"
            );
        }

        let annotations = proof
            .failed_overlaps
            .iter()
            .map(|overlap| (overlap.transaction_index, overlap))
            .collect::<BTreeMap<_, _>>();
        ensure!(
            annotations.len() == proof.failed_overlaps.len(),
            "duplicate failed-overlap annotation"
        );
        let census_overlaps = census["overlap_transactions"]
            .as_array()
            .context("closure conflict census overlaps missing")?;
        let mut actual_failed_overlaps = BTreeSet::new();
        for (index, transaction) in transactions.iter().enumerate() {
            let keys = account_keys(transaction)?;
            let writable_outputs = keys
                .iter()
                .enumerate()
                .filter(|(_, (address, writable))| {
                    *writable && output_set.contains(address.as_str())
                })
                .collect::<Vec<_>>();
            if index < proof.target_transaction_index {
                ensure!(
                    keys.iter().all(|(address, writable)| {
                        !*writable || !input_set.contains(address.as_str())
                    }),
                    "an earlier transaction declares a target input writable"
                );
                continue;
            }
            if index == proof.target_transaction_index || writable_outputs.is_empty() {
                continue;
            }
            ensure!(
                !successful(transaction)?,
                "a successful later transaction modifies a validation output"
            );
            actual_failed_overlaps.insert(index);
            let annotation = annotations
                .get(&index)
                .context("failed output overlap lacks rollback evidence")?;
            let reconstructed_nonce = if let Some(rows) = full_transactions {
                let complete = &rows[index];
                ensure!(
                    complete["version"].as_u64() == Some(0),
                    "unsupported failed-overlap message version"
                );
                let resolved = full_account_keys(complete)?;
                ensure!(
                    resolved
                        == keys
                            .iter()
                            .map(|(address, _)| address.clone())
                            .collect::<Vec<_>>(),
                    "full block resolved account keys differ"
                );
                full_durable_nonce_accounts(complete, &resolved)?
            } else {
                annotation.durable_nonce_accounts.clone()
            };
            ensure!(
                annotation.durable_nonce_accounts == reconstructed_nonce,
                "failed overlap nonce classification differs from transaction"
            );
            ensure!(
                signature(transaction)? == annotation.signature
                    && keys.first().map(|(address, _)| address.as_str())
                        == Some(annotation.fee_payer.as_str()),
                "failed overlap transaction identity differs"
            );
            let census_row = census_overlaps
                .iter()
                .find(|row| row["transaction_index"].as_u64() == Some(index as u64))
                .context("failed overlap missing from conflict census")?;
            if checkpoint_contract == 2 {
                ensure!(
                    census_row["message_version"].as_u64() == Some(0),
                    "failed overlap census message version differs"
                );
            }
            let census_nonce = census_row["durable_nonce_accounts"]
                .as_array()
                .context("failed overlap nonce census missing")?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .context("invalid nonce account")
                        .map(str::to_string)
                })
                .collect::<Result<Vec<_>>>()?;
            ensure!(
                census_row["success"].as_bool() == Some(false)
                    && census_row["signature"].as_str() == Some(annotation.signature.as_str())
                    && census_row["fee_payer"].as_str() == Some(annotation.fee_payer.as_str())
                    && census_nonce == reconstructed_nonce,
                "failed overlap rollback census differs"
            );
            if checkpoint_contract == 2 {
                let overlap_accounts = census_row["overlap_accounts"]
                    .as_array()
                    .context("failed overlap account classifications missing")?;
                for row in overlap_accounts {
                    let address = row["address"]
                        .as_str()
                        .context("overlap account address missing")?;
                    ensure!(
                        row["is_fee_payer"].as_bool() == Some(address == annotation.fee_payer)
                            && row["is_durable_nonce"].as_bool()
                                == Some(reconstructed_nonce.iter().any(|nonce| nonce == address)),
                        "failed overlap account classification differs from transaction"
                    );
                }
            }
            ensure!(
                !output_set.contains(annotation.fee_payer.as_str())
                    && annotation
                        .durable_nonce_accounts
                        .iter()
                        .all(|address| !output_set.contains(address.as_str())),
                "failed transaction can commit a fee-payer or nonce output mutation"
            );
            let pre = transaction["meta"]["preBalances"]
                .as_array()
                .context("failed overlap pre-balances missing")?;
            let post = transaction["meta"]["postBalances"]
                .as_array()
                .context("failed overlap post-balances missing")?;
            ensure!(
                pre.len() == keys.len() && post.len() == keys.len(),
                "failed overlap balance vector differs"
            );
            for (key_index, _) in writable_outputs {
                ensure!(
                    pre[key_index].as_u64() == post[key_index].as_u64(),
                    "failed ordinary output mutation did not roll back"
                );
            }
        }
        ensure!(
            actual_failed_overlaps == annotations.keys().copied().collect(),
            "failed-overlap annotations do not match the block"
        );
        Ok(())
    }
}
