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
        ensure!(self.schema_version == 1, "wrong closure proof schema");
        store.put(EvidenceKind::ClosureProof, &serde_json::to_vec(self)?)
    }

    pub fn verify(
        store: &EvidenceStore,
        reference: &EvidenceRef,
        record: &ReplayObservationV2,
        message: &ResolvedMessage,
        validation_outputs: &[String],
    ) -> Result<()> {
        ensure!(
            reference.kind == EvidenceKind::ClosureProof,
            "closure proof reference has wrong evidence category"
        );
        let proof: Self = serde_json::from_slice(&store.get(reference)?)?;
        ensure!(proof.schema_version == 1, "wrong closure proof schema");
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
                    && census_nonce == annotation.durable_nonce_accounts,
                "failed overlap rollback census differs"
            );
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
