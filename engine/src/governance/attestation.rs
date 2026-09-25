//! Post-execution proof for the narrow G1 Squads V4 Upgrade shape.
//!
//! A G1 match is a pre-execution observation. This module uses its sealed
//! message commitment, then independently identifies the successful execute
//! instruction and its loader CPI. Current ProgramData is compared only when
//! its deployment slot and full-block ordering attribute it to that execution.

use std::collections::BTreeSet;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solana_address::Address;

use super::{squads, BindingOutcome, Commitment, GovernanceBinding};
use crate::change::{ChangeSpec, Delivery, ExecutableArtifact};
use crate::ingest::{accounts, rpc::RpcProvider};
use crate::replay::hash_bytes;
use crate::screening::{self, ConflictPosition};
use crate::standard_programs::upgradeable_loader::{self as loader, LoaderInstruction};

pub const SCHEMA: u32 = 1;
const DOMAIN: &str = "eplyx-squads-deployment-attestation-v1";
const EXECUTE_NAME: &str = "global:vault_transaction_execute";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentOutcome {
    DeployedMatch,
    DeployedMismatch,
    Superseded,
    NotExecuted,
    Unsupported,
    Unverifiable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEvidence {
    pub signature: String,
    pub slot: u64,
    pub transaction_index: Option<usize>,
    pub blockhash: String,
    pub success: bool,
    pub squads_instruction_index: usize,
    pub loader_inner_instruction_index: usize,
    pub loader_accounts: Vec<String>,
    pub loader_data: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramDataEvidence {
    pub account_len: u64,
    pub account_sha256: String,
    pub deploy_slot: u64,
    pub executable_len: u64,
    pub candidate_len: u64,
    pub candidate_prefix_sha256: Option<String>,
    pub prefix_matches: Option<bool>,
    pub zero_padding_len: Option<u64>,
    pub zero_padding: Option<bool>,
    pub later_same_slot_writer: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentAttestation {
    pub schema_version: u32,
    pub kind: String,
    pub attestation_id: Option<String>,
    pub change_spec_id: String,
    pub binding_id: String,
    pub commitment: Commitment,
    pub multisig: String,
    pub transaction_index: u64,
    pub proposal: String,
    pub vault_transaction: String,
    pub message_sha256: String,
    pub target_program: String,
    pub programdata: String,
    pub candidate: ExecutableArtifact,
    pub observed_slot: Option<u64>,
    pub execution: Option<ExecutionEvidence>,
    pub deployed: Option<ProgramDataEvidence>,
    pub outcome: DeploymentOutcome,
    pub reasons: Vec<String>,
}

impl DeploymentAttestation {
    fn compute_id(&self) -> Result<String> {
        let mut body = self.clone();
        body.attestation_id = None;
        Ok(hash_bytes(&serde_json::to_vec(&(DOMAIN, &body))?))
    }

    fn seal(mut self) -> Result<Self> {
        self.attestation_id = Some(self.compute_id()?);
        Ok(self)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value: Self = serde_json::from_slice(bytes)?;
        ensure!(
            value.schema_version == SCHEMA && value.kind == DOMAIN,
            "unsupported deployment attestation"
        );
        ensure!(
            value.attestation_id.as_deref() == Some(value.compute_id()?.as_str()),
            "deployment attestation contents do not match their sealed id"
        );
        Ok(value)
    }

    pub fn to_document(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

fn execute_discriminator() -> [u8; 8] {
    let digest = Sha256::digest(EXECUTE_NAME.as_bytes());
    digest[..8].try_into().expect("eight bytes")
}

fn keys(tx: &Value) -> Result<Vec<String>> {
    let static_keys = tx["transaction"]["message"]["accountKeys"]
        .as_array()
        .context("transaction lacks account keys")?;
    let mut result: Vec<String> = static_keys
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .context("account key is not a string")
        })
        .collect::<Result<_>>()?;
    for section in ["writable", "readonly"] {
        if let Some(loaded) = tx["meta"]["loadedAddresses"][section].as_array() {
            for key in loaded {
                result.push(
                    key.as_str()
                        .context("loaded address is not a string")?
                        .into(),
                );
            }
        }
    }
    Ok(result)
}

fn instruction<'a>(
    value: &'a Value,
    keys: &'a [String],
) -> Result<(&'a str, Vec<&'a str>, Vec<u8>)> {
    let program_index = value["programIdIndex"]
        .as_u64()
        .context("instruction lacks program index")? as usize;
    let program = keys
        .get(program_index)
        .context("instruction program index out of bounds")?;
    let accounts = value["accounts"]
        .as_array()
        .context("instruction lacks accounts")?
        .iter()
        .map(|v| {
            let index = v
                .as_u64()
                .context("instruction account index is not an integer")?
                as usize;
            Ok(keys
                .get(index)
                .context("instruction account index out of bounds")?
                .as_str())
        })
        .collect::<Result<Vec<_>>>()?;
    let data = bs58::decode(
        value["data"]
            .as_str()
            .context("instruction lacks base58 data")?,
    )
    .into_vec()?;
    Ok((program, accounts, data))
}

fn expected_loader_accounts(binding: &GovernanceBinding) -> Result<Vec<String>> {
    let message = binding
        .observation
        .message
        .as_ref()
        .context("G1 binding has no stored message")?;
    ensure!(
        message.address_table_lookups.is_empty() && message.instructions.len() == 1,
        "G1 message is outside the supported shape"
    );
    let ix = &message.instructions[0];
    ix.account_indexes
        .iter()
        .map(|index| {
            message
                .account_keys
                .get(*index as usize)
                .cloned()
                .context("G1 message account index out of bounds")
        })
        .collect()
}

struct MatchedExecution {
    top_index: usize,
    inner_index: usize,
    loader_accounts: Vec<String>,
    loader_data: String,
}

fn identify_execution(
    tx: &Value,
    signature: &str,
    binding: &GovernanceBinding,
) -> Result<Option<MatchedExecution>> {
    ensure!(
        tx["transaction"]["signatures"][0].as_str() == Some(signature),
        "RPC returned a transaction under a different signature"
    );
    if tx["meta"].get("err") != Some(&Value::Null) {
        return Ok(None);
    }
    let all_keys = keys(tx)?;
    let top = tx["transaction"]["message"]["instructions"]
        .as_array()
        .context("transaction lacks instructions")?;
    let message = binding
        .observation
        .message
        .as_ref()
        .context("G1 message missing")?;
    let upgrade = binding
        .observation
        .upgrade
        .as_ref()
        .context("G1 loader Upgrade missing")?;
    let expected_accounts = expected_loader_accounts(binding)?;
    let mut found = None;
    for (top_index, ix) in top.iter().enumerate() {
        let (program, accounts, data) = instruction(ix, &all_keys)?;
        if program != squads::SQUADS_V4_PROGRAM_ID || data.as_slice() != execute_discriminator() {
            continue;
        }
        if accounts.len() < 4
            || accounts[0] != binding.multisig()
            || accounts[1] != binding.proposal()
            || accounts[2] != binding.transaction()
        {
            continue;
        }
        // No LUT or ephemeral signers in G1. Squads validates these remaining
        // accounts against the message before it invokes anything.
        if accounts.len() != 4 + message.account_keys.len()
            || accounts[4..]
                != message
                    .account_keys
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
        {
            continue;
        }
        let groups = tx["meta"]["innerInstructions"]
            .as_array()
            .context("successful transaction lacks inner instructions")?;
        let group = groups
            .iter()
            .find(|g| g["index"].as_u64() == Some(top_index as u64))
            .context("Squads execute has no CPI group")?;
        let inner = group["instructions"]
            .as_array()
            .context("CPI group lacks instructions")?;
        let mut loader_found = None;
        for (inner_index, candidate) in inner.iter().enumerate() {
            let (callee, ix_accounts, ix_data) = instruction(candidate, &all_keys)?;
            if callee != loader::id().to_string() {
                continue;
            }
            // Direct CPI from the top-level Squads invocation. A loader call
            // deeper in another program is not this stored one-instruction
            // message being executed.
            if candidate["stackHeight"].as_u64() != Some(2) {
                continue;
            }
            if loader::decode_instruction(&ix_data) != LoaderInstruction::Upgrade {
                continue;
            }
            if ix_accounts
                != expected_accounts
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            {
                continue;
            }
            if ix_accounts.len() != loader::UPGRADE_ACCOUNTS
                || ix_accounts[0] != upgrade.programdata
                || ix_accounts[1] != upgrade.program
                || ix_accounts[2] != upgrade.buffer
                || ix_accounts[3] != upgrade.spill
                || ix_accounts[6] != upgrade.authority
            {
                continue;
            }
            ensure!(
                loader_found.is_none(),
                "execution {signature} contains multiple matching loader Upgrades"
            );
            loader_found = Some((
                inner_index,
                ix_accounts.iter().map(|s| s.to_string()).collect(),
                crate::hexfmt::encode(&ix_data),
            ));
        }
        let Some((inner_index, accounts, data)) = loader_found else {
            continue;
        };
        ensure!(
            found.is_none(),
            "execution {signature} contains multiple matching Squads execute instructions"
        );
        found = Some(MatchedExecution {
            top_index,
            inner_index,
            loader_accounts: accounts,
            loader_data: data,
        });
    }
    Ok(found)
}

trait BindingKeys {
    fn multisig(&self) -> &str;
    fn proposal(&self) -> &str;
    fn transaction(&self) -> &str;
}
impl BindingKeys for GovernanceBinding {
    fn multisig(&self) -> &str {
        &self.request.multisig
    }
    fn proposal(&self) -> &str {
        &self
            .observation
            .delivery
            .as_ref()
            .expect("validated precondition")
            .proposal
    }
    fn transaction(&self) -> &str {
        &self
            .observation
            .delivery
            .as_ref()
            .expect("validated precondition")
            .transaction
    }
}

fn read_current(
    rpc: &dyn RpcProvider,
    program: &str,
    programdata: &str,
) -> Result<(u64, Vec<crate::types::AccountSnapshot>)> {
    let result = rpc.call(
        "getMultipleAccounts",
        json!([[program, programdata], {"encoding":"base64", "commitment":"finalized"}]),
    )?;
    let slot = result["context"]["slot"]
        .as_u64()
        .context("account read has no context slot")?;
    let values = result["value"]
        .as_array()
        .context("account read has no values")?;
    ensure!(
        values.len() == 2,
        "account read returned {} values",
        values.len()
    );
    let accounts = values
        .iter()
        .map(accounts::normalize)
        .collect::<Result<Vec<_>>>()?;
    Ok((slot, accounts))
}

fn compare(data: &[u8], candidate: &[u8], deploy_slot: u64) -> ProgramDataEvidence {
    let region = loader::decode_programdata(data)
        .expect("decoded by caller")
        .bytes;
    let enough = region.len() >= candidate.len();
    let prefix = enough.then(|| &region[..candidate.len()]);
    let padding = enough.then(|| &region[candidate.len()..]);
    ProgramDataEvidence {
        account_len: data.len() as u64,
        account_sha256: hash_bytes(data),
        deploy_slot,
        executable_len: region.len() as u64,
        candidate_len: candidate.len() as u64,
        candidate_prefix_sha256: prefix.map(hash_bytes),
        prefix_matches: prefix.map(|p| p == candidate),
        zero_padding_len: padding.map(|p| p.len() as u64),
        zero_padding: padding.map(|p| p.iter().all(|b| *b == 0)),
        later_same_slot_writer: None,
    }
}

/// Re-read finalized chain evidence and the exact content-addressed candidate.
/// The caller supplies bytes only after its artefact store has checked them;
/// their hash and length are checked again here against the ChangeSpec.
pub fn attest_squads_upgrade(
    rpc: &dyn RpcProvider,
    spec: &ChangeSpec,
    binding: &GovernanceBinding,
    candidate_bytes: &[u8],
) -> Result<DeploymentAttestation> {
    ensure!(
        binding.binding_id.as_deref() == Some(binding.id()?.as_str()),
        "G1 binding is not sealed"
    );
    ensure!(
        binding.outcome == BindingOutcome::Matched,
        "G2 requires a matched pre-execution G1 binding"
    );
    ensure!(
        binding.commitment == Commitment::Finalized,
        "G2 requires a finalized G1 binding"
    );
    ensure!(
        binding.bound_change_spec_id.as_deref() == Some(spec.id()?.as_str()),
        "G1 binding does not identify this governance-bound ChangeSpec"
    );
    let Delivery::SquadsV4(delivery) = spec
        .delivery()
        .context("ChangeSpec has no Squads delivery")?;
    squads::validate_delivery(delivery)?;
    ensure!(
        binding.observation.delivery.as_ref() == Some(delivery)
            && binding
                .expected
                .delivery
                .as_ref()
                .is_none_or(|stated| stated == delivery),
        "G1 binding names a different delivery"
    );
    ensure!(
        binding.expected.candidate == *spec.candidate(),
        "G1 binding names a different candidate"
    );
    ensure!(
        ExecutableArtifact::of(candidate_bytes) == *spec.candidate(),
        "content-addressed candidate bytes do not match ChangeSpec"
    );
    let upgrade = binding
        .observation
        .upgrade
        .as_ref()
        .context("G1 binding has no Upgrade")?;
    let crate::change::Change::ProgramUpgrade { target, .. } = &spec.change;
    ensure!(
        upgrade.program == target.program_id
            && target
                .programdata_address
                .as_deref()
                .is_none_or(|address| address == upgrade.programdata),
        "G1 upgrade target differs from ChangeSpec"
    );
    let mut result = DeploymentAttestation {
        schema_version: SCHEMA,
        kind: DOMAIN.into(),
        attestation_id: None,
        change_spec_id: spec.id()?,
        binding_id: binding.id()?,
        commitment: Commitment::Finalized,
        multisig: delivery.multisig.clone(),
        transaction_index: delivery.transaction_index,
        proposal: delivery.proposal.clone(),
        vault_transaction: delivery.transaction.clone(),
        message_sha256: delivery.message_sha256.clone(),
        target_program: upgrade.program.clone(),
        programdata: upgrade.programdata.clone(),
        candidate: spec.candidate().clone(),
        observed_slot: None,
        execution: None,
        deployed: None,
        outcome: DeploymentOutcome::Unverifiable,
        reasons: vec![],
    };
    let (slot, current) = match read_current(rpc, &upgrade.program, &upgrade.programdata) {
        Ok(v) => v,
        Err(e) => {
            result
                .reasons
                .push(format!("current account read failed: {e:#}"));
            return result.seal();
        }
    };
    result.observed_slot = Some(slot);
    if current[0].owner != loader::id().to_string()
        || !current[0].executable
        || loader::decode_program(&current[0].data)
            .ok()
            .map(|k| k.to_string())
            != Some(upgrade.programdata.clone())
    {
        result
            .reasons
            .push("target Program no longer points to the expected ProgramData".into());
        return result.seal();
    }
    if current[1].owner != loader::id().to_string() {
        result
            .reasons
            .push("ProgramData is not loader-owned".into());
        return result.seal();
    }
    let pd = match loader::decode_programdata(&current[1].data) {
        Ok(v) => v,
        Err(e) => {
            result
                .reasons
                .push(format!("ProgramData does not decode: {e:#}"));
            return result.seal();
        }
    };
    // The post-execution VaultTransaction may have been consumed. Its current
    // bytes are therefore not used as the message commitment.
    let proposal = match rpc.call("getAccountInfo", json!([delivery.proposal, {"encoding":"base64", "commitment":"finalized", "minContextSlot": slot}])) {
        Ok(v) => v, Err(e) => { result.reasons.push(format!("Proposal read failed: {e:#}")); return result.seal(); }
    };
    if proposal["context"]["slot"]
        .as_u64()
        .is_none_or(|seen| seen < slot)
    {
        result
            .reasons
            .push("Proposal read did not meet the ProgramData context slot".into());
        return result.seal();
    }
    let Some(value) = proposal.get("value").filter(|v| !v.is_null()) else {
        result
            .reasons
            .push("Proposal account is unavailable".into());
        return result.seal();
    };
    let proposal_account = match accounts::normalize(value) {
        Ok(v) => v,
        Err(e) => {
            result
                .reasons
                .push(format!("Proposal account is malformed: {e:#}"));
            return result.seal();
        }
    };
    if proposal_account.owner != squads::SQUADS_V4_PROGRAM_ID {
        result
            .reasons
            .push("Proposal owner differs from Squads V4".into());
        return result.seal();
    }
    let proposal = match squads::decode_proposal(&proposal_account.data) {
        Ok(v) => v,
        Err(e) => {
            result
                .reasons
                .push(format!("Proposal does not decode: {e:#}"));
            return result.seal();
        }
    };
    if Address::from(proposal.multisig).to_string() != delivery.multisig
        || proposal.transaction_index != delivery.transaction_index
    {
        result
            .reasons
            .push("Proposal identity differs from the sealed G1 delivery".into());
        return result.seal();
    }
    if proposal.status.kind() != squads::ProposalStatusKind::Executed {
        result.outcome = DeploymentOutcome::NotExecuted;
        result
            .reasons
            .push(format!("Proposal status is {:?}", proposal.status.kind()));
        return result.seal();
    }
    let stored_transaction = match rpc.call("getAccountInfo", json!([delivery.transaction, {"encoding":"base64", "commitment":"finalized", "minContextSlot": slot}])) {
        Ok(v) => v, Err(e) => { result.reasons.push(format!("VaultTransaction read failed: {e:#}")); return result.seal(); }
    };
    if stored_transaction["context"]["slot"]
        .as_u64()
        .is_none_or(|seen| seen < slot)
    {
        result
            .reasons
            .push("VaultTransaction read did not meet the ProgramData context slot".into());
        return result.seal();
    }
    if let Some(value) = stored_transaction.get("value").filter(|v| !v.is_null()) {
        let account = match accounts::normalize(value) {
            Ok(v) => v,
            Err(e) => {
                result
                    .reasons
                    .push(format!("VaultTransaction account is malformed: {e:#}"));
                return result.seal();
            }
        };
        if account.owner != squads::SQUADS_V4_PROGRAM_ID {
            result
                .reasons
                .push("VaultTransaction owner differs from Squads V4".into());
            return result.seal();
        }
        let (transaction, message_bytes) = match squads::decode_vault_transaction(&account.data) {
            Ok(v) => v,
            Err(e) => {
                result
                    .reasons
                    .push(format!("VaultTransaction does not decode: {e:#}"));
                return result.seal();
            }
        };
        if Address::from(transaction.multisig).to_string() != delivery.multisig
            || transaction.index != delivery.transaction_index
        {
            result
                .reasons
                .push("VaultTransaction identity differs from sealed G1 delivery".into());
            return result.seal();
        }
        if !transaction.message.instructions.is_empty()
            && squads::message_hash_of_bytes(&message_bytes) != delivery.message_sha256
        {
            result.reasons.push(
                "retained VaultTransaction message differs from the sealed G1 commitment".into(),
            );
            return result.seal();
        }
    }
    let mut before: Option<String> = None;
    let mut found = None;
    for _ in 0..10 {
        let mut options = json!({"limit": 1000, "commitment": "finalized"});
        if let Some(cursor) = &before {
            options["before"] = json!(cursor);
        }
        let page = match rpc.call(
            "getSignaturesForAddress",
            json!([delivery.proposal, options]),
        ) {
            Ok(v) => v,
            Err(e) => {
                result
                    .reasons
                    .push(format!("execution signature search failed: {e:#}"));
                return result.seal();
            }
        };
        let Some(signatures) = page.as_array() else {
            result
                .reasons
                .push("signature search did not return a list".into());
            return result.seal();
        };
        for entry in signatures {
            if entry.get("err").is_none() {
                result
                    .reasons
                    .push("signature entry omits execution status".into());
                return result.seal();
            }
            if !entry["err"].is_null() {
                continue;
            }
            let Some(signature) = entry["signature"].as_str() else {
                result
                    .reasons
                    .push("signature entry has no signature".into());
                return result.seal();
            };
            let tx = match rpc.call("getTransaction", json!([signature, {"encoding":"json", "commitment":"finalized", "maxSupportedTransactionVersion": crate::ingest::MAX_SUPPORTED_TRANSACTION_VERSION}])) {
                Ok(v) if !v.is_null() => v, _ => continue,
            };
            let identified = match identify_execution(&tx, signature, binding) {
                Ok(v) => v,
                Err(e) => {
                    result
                        .reasons
                        .push(format!("transaction {signature} cannot be decoded: {e:#}"));
                    return result.seal();
                }
            };
            if let Some(matched) = identified {
                if found.is_some() {
                    result.reasons.push(
                        "multiple successful executions identify the same Squads proposal".into(),
                    );
                    return result.seal();
                }
                let Some(execution_slot) = tx["slot"].as_u64() else {
                    result
                        .reasons
                        .push("execution transaction lacks a slot".into());
                    return result.seal();
                };
                let Some(blockhash) = tx["transaction"]["message"]["recentBlockhash"].as_str()
                else {
                    result
                        .reasons
                        .push("execution transaction lacks a blockhash".into());
                    return result.seal();
                };
                let screening = screening::screen_later(
                    rpc,
                    execution_slot,
                    signature,
                    &BTreeSet::from([upgrade.program.clone(), upgrade.programdata.clone()]),
                );
                let transaction_index = screening.as_ref().ok().map(|s| s.target_index);
                found = Some((
                    ExecutionEvidence {
                        signature: signature.into(),
                        slot: execution_slot,
                        transaction_index,
                        blockhash: blockhash.into(),
                        success: true,
                        squads_instruction_index: matched.top_index,
                        loader_inner_instruction_index: matched.inner_index,
                        loader_accounts: matched.loader_accounts,
                        loader_data: matched.loader_data,
                    },
                    screening,
                ));
            }
        }
        if signatures.len() < 1000 {
            break;
        }
        before = signatures
            .last()
            .and_then(|v| v["signature"].as_str())
            .map(str::to_owned);
    }
    let Some((execution, screening)) = found else {
        result.reasons.push("Proposal is Executed but no matching successful Squads execute and loader Upgrade was found within the bounded signature search".into());
        return result.seal();
    };
    let execution_slot = execution.slot;
    result.execution = Some(execution);
    result.deployed = Some(ProgramDataEvidence {
        account_len: current[1].data.len() as u64,
        account_sha256: hash_bytes(&current[1].data),
        deploy_slot: pd.deploy_slot,
        executable_len: pd.bytes.len() as u64,
        candidate_len: candidate_bytes.len() as u64,
        candidate_prefix_sha256: None,
        prefix_matches: None,
        zero_padding_len: None,
        zero_padding: None,
        later_same_slot_writer: None,
    });
    if pd.deploy_slot > execution_slot {
        result.outcome = DeploymentOutcome::Superseded;
        result.reasons.push(format!("ProgramData was deployed again at slot {}, after this execution at slot {execution_slot}", pd.deploy_slot));
        return result.seal();
    }
    if pd.deploy_slot < execution_slot {
        result.reasons.push(format!(
            "ProgramData deployment slot {} precedes the verified Upgrade at {execution_slot}",
            pd.deploy_slot
        ));
        return result.seal();
    }
    let screening = match screening {
        Ok(v) => v,
        Err(e) => {
            result.reasons.push(format!(
                "complete finalized block could not prove same-slot ordering: {e:#}"
            ));
            return result.seal();
        }
    };
    let later = screening
        .conflicts
        .iter()
        .any(|c| c.position == Some(ConflictPosition::After));
    result
        .deployed
        .as_mut()
        .expect("present")
        .later_same_slot_writer = Some(later);
    if later {
        result
            .reasons
            .push("a later transaction in the same slot could write Program or ProgramData".into());
        return result.seal();
    }
    result.deployed = Some(compare(&current[1].data, candidate_bytes, pd.deploy_slot));
    result
        .deployed
        .as_mut()
        .expect("present")
        .later_same_slot_writer = Some(false);
    let compared = result.deployed.as_ref().expect("present");
    if compared.prefix_matches == Some(true) && compared.zero_padding == Some(true) {
        result.outcome = DeploymentOutcome::DeployedMatch;
        result
            .reasons
            .push("candidate prefix matches and every allocated byte after it is zero".into());
    } else {
        result.outcome = DeploymentOutcome::DeployedMismatch;
        result.reasons.push(
            "attributable ProgramData differs from the analysed candidate or has non-zero padding"
                .into(),
        );
    }
    result.seal()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::governance::{simulated, verify_squads_upgrade, SquadsProposalRef};
    use crate::standard_programs::upgradeable_loader::encode;
    use base64::Engine;

    struct Scenario {
        proposal: Value,
        vault_transaction: Value,
        program: Value,
        programdata: Value,
        transaction: Value,
        block: Value,
        signatures: Value,
    }

    fn account(owner: Address, data: Vec<u8>, executable: bool) -> Value {
        json!({"owner": owner.to_string(), "lamports": 1_000_000, "executable": executable,
            "rentEpoch": u64::MAX, "data": [base64::prelude::BASE64_STANDARD.encode(data), "base64"]})
    }

    impl RpcProvider for Scenario {
        fn call(&self, method: &str, params: Value) -> Result<Value> {
            Ok(match method {
                "getMultipleAccounts" => {
                    json!({"context":{"slot":7001}, "value":[self.program,self.programdata]})
                }
                "getAccountInfo" => {
                    json!({"context":{"slot":7001}, "value":if params[0] == self.transaction["transaction"]["message"]["accountKeys"][3] { &self.vault_transaction } else { &self.proposal }})
                }
                "getSignaturesForAddress" => self.signatures.clone(),
                "getTransaction" => self.transaction.clone(),
                "getBlock" => self.block.clone(),
                _ => anyhow::bail!("unexpected RPC {method}"),
            })
        }
    }

    fn setup() -> (ChangeSpec, GovernanceBinding, Scenario) {
        let world = simulated::World::new();
        let analysed = world.spec();
        let binding = verify_squads_upgrade(
            &world,
            &SquadsProposalRef {
                multisig: simulated::multisig().to_string(),
                transaction_index: simulated::TRANSACTION_INDEX,
            },
            &analysed,
            Commitment::Finalized,
        )
        .unwrap();
        assert_eq!(binding.outcome, BindingOutcome::Matched);
        let spec = binding.bound_spec(&analysed).unwrap().unwrap();
        let delivery = binding.observation.delivery.as_ref().unwrap();
        let upgrade = binding.observation.upgrade.as_ref().unwrap();
        let mut proposal = world.state().proposal().clone();
        proposal.status = squads::ProposalStatus::Executed {
            timestamp: 1_790_000_100,
        };
        let proposal_account = account(
            squads::program(),
            simulated::anchor_account(squads::PROPOSAL_DISCRIMINATOR, &proposal, 96),
            false,
        );
        let vault_transaction_account = account(
            squads::program(),
            simulated::anchor_account(
                squads::VAULT_TRANSACTION_DISCRIMINATOR,
                world.state().transaction(),
                0,
            ),
            false,
        );
        let program_account = account(
            loader::id(),
            encode::program(&upgrade.programdata.parse().unwrap()),
            true,
        );
        let programdata_account = account(
            loader::id(),
            encode::programdata(7000, Some(simulated::vault()), simulated::CANDIDATE_ELF),
            false,
        );
        let message = binding.observation.message.as_ref().unwrap();
        let mut keys = vec![
            squads::SQUADS_V4_PROGRAM_ID.to_string(),
            delivery.multisig.clone(),
            delivery.proposal.clone(),
            delivery.transaction.clone(),
            simulated::member(1).to_string(),
        ];
        for key in &message.account_keys {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        let index = |key: &str| keys.iter().position(|k| k == key).unwrap();
        let top_accounts: Vec<usize> = [
            &delivery.multisig,
            &delivery.proposal,
            &delivery.transaction,
            &simulated::member(1).to_string(),
        ]
        .iter()
        .map(|key| index(key))
        .chain(message.account_keys.iter().map(|key| index(key)))
        .collect();
        let loader_accounts: Vec<usize> = expected_loader_accounts(&binding)
            .unwrap()
            .iter()
            .map(|key| index(key))
            .collect();
        let tx = json!({"slot":7000, "meta":{"err":null,"innerInstructions":[{"index":0,"instructions":[
            {"programIdIndex":index(&loader::id().to_string()),"accounts":loader_accounts,"data":bs58::encode(encode::upgrade()).into_string(),"stackHeight":2}
        ]}]},"transaction":{"message":{"accountKeys":keys,"recentBlockhash":"blockhash","instructions":[
            {"programIdIndex":0,"accounts":top_accounts,"data":bs58::encode(execute_discriminator()).into_string()}
        ]},"signatures":["execution"]}});
        let block_keys: Vec<Value> = keys.iter().map(|key| json!({"pubkey":key,"writable": key == &upgrade.program || key == &upgrade.programdata})).collect();
        let block = json!({"transactions":[{"transaction":{"signatures":["execution"],"accountKeys":block_keys},"meta":{"err":null}}]});
        let scenario = Scenario {
            proposal: proposal_account,
            vault_transaction: vault_transaction_account,
            program: program_account,
            programdata: programdata_account,
            transaction: tx,
            block,
            signatures: json!([{"signature":"execution","err":null}]),
        };
        (spec, binding, scenario)
    }

    #[test]
    fn exact_execution_and_zero_padded_programdata_match() {
        let (spec, binding, mut rpc) = setup();
        let mut bytes = simulated::CANDIDATE_ELF.to_vec();
        bytes.extend([0; 13]);
        rpc.programdata = account(
            loader::id(),
            encode::programdata(7000, Some(simulated::vault()), &bytes),
            false,
        );
        let proof = attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
        assert_eq!(proof.outcome, DeploymentOutcome::DeployedMatch);
        assert_eq!(proof.deployed.as_ref().unwrap().zero_padding_len, Some(13));
        DeploymentAttestation::parse(proof.to_document().unwrap().as_bytes()).unwrap();
        let mut tampered = proof;
        tampered.reasons.push("fabricated".into());
        assert!(DeploymentAttestation::parse(tampered.to_document().unwrap().as_bytes()).is_err());
    }

    #[test]
    fn attributable_prefix_and_padding_mismatches_are_distinct_from_supersession() {
        let (spec, binding, mut rpc) = setup();
        let first = attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
        assert_eq!(first.outcome, DeploymentOutcome::DeployedMatch);
        let mut wrong = simulated::CANDIDATE_ELF.to_vec();
        wrong[10] ^= 1;
        rpc.programdata = account(
            loader::id(),
            encode::programdata(7000, Some(simulated::vault()), &wrong),
            false,
        );
        assert_eq!(
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF)
                .unwrap()
                .outcome,
            DeploymentOutcome::DeployedMismatch
        );
        let mut padded = simulated::CANDIDATE_ELF.to_vec();
        padded.extend([0, 0, 1]);
        rpc.programdata = account(
            loader::id(),
            encode::programdata(7000, Some(simulated::vault()), &padded),
            false,
        );
        assert_eq!(
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF)
                .unwrap()
                .outcome,
            DeploymentOutcome::DeployedMismatch
        );
        rpc.programdata = account(
            loader::id(),
            encode::programdata(7002, Some(simulated::vault()), &wrong),
            false,
        );
        let later = attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
        assert_eq!(later.outcome, DeploymentOutcome::Superseded);
        assert_eq!(later.deployed.unwrap().prefix_matches, None);
        assert_eq!(
            DeploymentAttestation::parse(first.to_document().unwrap().as_bytes())
                .unwrap()
                .outcome,
            DeploymentOutcome::DeployedMatch
        );
        rpc.programdata = account(
            loader::id(),
            encode::programdata(
                7000,
                Some(simulated::vault()),
                &simulated::CANDIDATE_ELF[..8],
            ),
            false,
        );
        assert_eq!(
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF)
                .unwrap()
                .outcome,
            DeploymentOutcome::DeployedMismatch
        );
    }

    #[test]
    fn same_slot_writer_wrong_execution_and_unexecuted_status_fail_closed() {
        let (spec, binding, mut rpc) = setup();
        let target = binding
            .observation
            .upgrade
            .as_ref()
            .unwrap()
            .programdata
            .clone();
        rpc.block["transactions"].as_array_mut().unwrap().push(json!({"transaction":{"signatures":["later"],"accountKeys":[{"pubkey":target,"writable":true}]},"meta":{"err":null}}));
        let blocked =
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
        assert_eq!(blocked.outcome, DeploymentOutcome::Unverifiable);
        assert_eq!(blocked.deployed.unwrap().prefix_matches, None);
        rpc.block["transactions"].as_array_mut().unwrap().pop();
        rpc.transaction["transaction"]["message"]["instructions"][0]["accounts"][1] = json!(4);
        assert_eq!(
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF)
                .unwrap()
                .outcome,
            DeploymentOutcome::Unverifiable
        );
        rpc.transaction = Value::Null;
        let mut proposal = squads::decode_proposal(
            &base64::prelude::BASE64_STANDARD
                .decode(rpc.proposal["data"][0].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        proposal.status = squads::ProposalStatus::Active {
            timestamp: 1_790_000_000,
        };
        rpc.proposal = account(
            squads::program(),
            simulated::anchor_account(squads::PROPOSAL_DISCRIMINATOR, &proposal, 96),
            false,
        );
        assert_eq!(
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF)
                .unwrap()
                .outcome,
            DeploymentOutcome::NotExecuted
        );
    }

    #[test]
    fn failed_execution_wrong_loader_and_stale_deploy_slot_never_match() {
        let (spec, binding, mut rpc) = setup();
        rpc.transaction["meta"]["err"] = json!({"InstructionError":[3,"Custom"]});
        assert_eq!(
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF)
                .unwrap()
                .outcome,
            DeploymentOutcome::Unverifiable
        );
        rpc.transaction["meta"]["err"] = Value::Null;
        rpc.transaction["meta"]["innerInstructions"][0]["instructions"][0]["accounts"][0] =
            json!(4);
        assert_eq!(
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF)
                .unwrap()
                .outcome,
            DeploymentOutcome::Unverifiable
        );
        let (_, _, original) = setup();
        rpc.transaction = original.transaction;
        rpc.programdata = account(
            loader::id(),
            encode::programdata(6999, Some(simulated::vault()), simulated::CANDIDATE_ELF),
            false,
        );
        assert_eq!(
            attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF)
                .unwrap()
                .outcome,
            DeploymentOutcome::Unverifiable
        );
    }
}
