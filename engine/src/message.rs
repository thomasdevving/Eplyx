//! Experimental historical v0 message proof. No protocol IDs, network transport,
//! ReplayRecord migration or legacy conversion. RPC metadata is a comparison
//! target only; every loaded key must first come from historical account bytes.
use crate::{
    ingest::{
        accounts,
        rpc::RpcProvider,
        transactions::{self, HistoricalTransaction},
    },
    replay::hash_bytes,
    types::AccountMetaSpec,
};
use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
use solana_address_lookup_table_interface::{
    program,
    state::{AddressLookupTable, LookupTableMeta},
};
use solana_hash::Hash;
use solana_message::{
    compiled_instruction::CompiledInstruction,
    v0::{self, LoadedAddresses, LoadedMessage, MessageAddressTableLookup},
    MessageHeader, VersionedMessage,
};
use solana_slot_hashes::SlotHashes;
use std::collections::{BTreeMap, HashSet};

pub const SLOT_HASHES_ID: &str = "SysvarS1otHashes111111111111111111111111111";
pub const SYSVAR_OWNER: &str = "Sysvar1111111111111111111111111111111111111";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalVisibility {
    FinalizedEndOfExecutionSlot,
}

/// Capability reference is to a separately retained archive-validation artifact,
/// not a declaration that a standard RPC honors the nonstandard slot parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveProvenance {
    pub scheme_host: String,
    pub genesis_hash: String,
    pub visibility: HistoricalVisibility,
    pub validation_artifact_sha256: String,
}
impl ArchiveProvenance {
    pub fn validate(&self) -> Result<()> {
        let host = self
            .scheme_host
            .strip_prefix("https://")
            .or_else(|| self.scheme_host.strip_prefix("http://"))
            .context("provider identity must be scheme + host")?;
        ensure!(
            !host.is_empty()
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c)),
            "provider identity must omit credentials, path and query"
        );
        self.genesis_hash
            .parse::<Hash>()
            .context("invalid genesis hash")?;
        ensure!(
            self.validation_artifact_sha256.len() == 64
                && self
                    .validation_artifact_sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit()),
            "archive capability validation artifact required"
        );
        Ok(())
    }
}

/// Raw envelope retained in full; its exact hash and decoded account hash are
/// distinct. Identity binds table address, bank context and provider semantics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalAccountEvidence {
    pub pubkey: String,
    pub execution_slot: u64,
    pub requested_slot: u64,
    pub returned_context_slot: u64,
    pub provider: ArchiveProvenance,
    pub raw_response_base64: String,
    pub raw_response_sha256: String,
    pub raw_account_sha256: String,
    pub evidence_id: String,
}
impl HistoricalAccountEvidence {
    pub fn from_response(
        pubkey: &str,
        execution_slot: u64,
        provider: ArchiveProvenance,
        raw: &[u8],
    ) -> Result<Self> {
        provider.validate()?;
        pubkey.parse::<Address>()?;
        let response: Value =
            serde_json::from_slice(raw).context("invalid historical RPC envelope")?;
        ensure!(
            response.get("error").is_none(),
            "historical RPC returned an error"
        );
        let result = response
            .get("result")
            .context("missing historical RPC result")?;
        let context = result["context"]["slot"]
            .as_u64()
            .context("missing archive context slot")?;
        ensure!(
            context == execution_slot,
            "archive did not honor end-of-execution-slot context"
        );
        let account = accounts::normalize(&result["value"])?;
        let mut evidence = Self {
            pubkey: pubkey.into(),
            execution_slot,
            requested_slot: execution_slot,
            returned_context_slot: context,
            provider,
            raw_response_base64: BASE64_STANDARD.encode(raw),
            raw_response_sha256: hash_bytes(raw),
            raw_account_sha256: hash_bytes(&account.data),
            evidence_id: String::new(),
        };
        evidence.evidence_id = evidence.identity()?;
        Ok(evidence)
    }
    fn identity(&self) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            "historical-account-evidence-v1",
            &self.pubkey,
            self.execution_slot,
            self.requested_slot,
            self.returned_context_slot,
            &self.provider,
            &self.raw_response_sha256,
            &self.raw_account_sha256,
        ))?))
    }
    pub fn account(
        &self,
        execution_slot: u64,
        genesis: &str,
    ) -> Result<crate::types::AccountSnapshot> {
        self.provider.validate()?;
        ensure!(
            self.execution_slot == execution_slot
                && self.requested_slot == execution_slot
                && self.returned_context_slot == execution_slot,
            "historical evidence execution slot differs"
        );
        ensure!(
            self.provider.genesis_hash == genesis,
            "historical evidence genesis differs"
        );
        let raw = BASE64_STANDARD.decode(&self.raw_response_base64)?;
        ensure!(
            hash_bytes(&raw) == self.raw_response_sha256 && self.identity()? == self.evidence_id,
            "historical evidence identity/hash differs"
        );
        let verified =
            Self::from_response(&self.pubkey, execution_slot, self.provider.clone(), &raw)?;
        ensure!(
            &verified == self,
            "historical evidence envelope/hash differs"
        );
        let envelope: Value = serde_json::from_slice(&raw)?;
        accounts::normalize(&envelope["result"]["value"])
    }
}

/// Reuses the existing provider-neutral RpcProvider and exact-slot archive
/// convention. Raw transport/envelope preservation belongs to the collector;
/// this API retains a canonical envelope when the existing transport returns a
/// decoded result. Neither path ever retries against current account state.
pub fn acquire_historical_account(
    rpc: &dyn RpcProvider,
    pubkey: &str,
    execution_slot: u64,
    provider: ArchiveProvenance,
) -> Result<HistoricalAccountEvidence> {
    let genesis = rpc.call("getGenesisHash", json!([]))?;
    ensure!(
        genesis.as_str() == Some(provider.genesis_hash.as_str()),
        "archive genesis differs"
    );
    let result = rpc.call(
        "getAccountInfo",
        json!([pubkey, {"encoding":"base64", "commitment":"finalized", "slot":execution_slot}]),
    )?;
    HistoricalAccountEvidence::from_response(
        pubkey,
        execution_slot,
        provider,
        &serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":1, "result":result}))?,
    )
}

/// A parallel representation preserves the native compiled v0 message without
/// changing normalized transaction serialization or frozen U3A derived bytes.
#[derive(Clone, Debug)]
pub struct FrozenV0 {
    signature: String,
    slot: u64,
    genesis: String,
    message: v0::Message,
    rpc_loaded: LoadedAddresses,
    normalized: HistoricalTransaction,
    rpc_result_sha256: String,
}
fn byte(value: &Value) -> Result<u8> {
    u8::try_from(value.as_u64().context("missing unsigned v0 field")?)
        .context("v0 index exceeds u8")
}
fn indexes(value: &Value) -> Result<Vec<u8>> {
    value
        .as_array()
        .context("missing v0 indexes")?
        .iter()
        .map(byte)
        .collect()
}
fn addresses(value: &Value) -> Result<Vec<Address>> {
    value
        .as_array()
        .context("missing v0 addresses")?
        .iter()
        .map(|v| {
            v.as_str()
                .context("missing v0 address")?
                .parse()
                .context("invalid v0 address")
        })
        .collect()
}
impl FrozenV0 {
    pub fn from_rpc(result: &Value, genesis: &str) -> Result<Self> {
        genesis.parse::<Hash>()?;
        ensure!(
            result["version"].as_u64() == Some(0),
            "historical LUT proof requires v0"
        );
        let raw = &result["transaction"]["message"];
        let header = &raw["header"];
        let message = v0::Message {
            header: MessageHeader {
                num_required_signatures: byte(&header["numRequiredSignatures"])?,
                num_readonly_signed_accounts: byte(&header["numReadonlySignedAccounts"])?,
                num_readonly_unsigned_accounts: byte(&header["numReadonlyUnsignedAccounts"])?,
            },
            account_keys: addresses(&raw["accountKeys"])?,
            recent_blockhash: raw["recentBlockhash"]
                .as_str()
                .context("missing blockhash")?
                .parse()?,
            instructions: raw["instructions"]
                .as_array()
                .context("missing compiled instructions")?
                .iter()
                .map(|ix| {
                    Ok(CompiledInstruction {
                        program_id_index: byte(&ix["programIdIndex"])?,
                        accounts: indexes(&ix["accounts"])?,
                        data: bs58::decode(ix["data"].as_str().context("missing compiled data")?)
                            .into_vec()?,
                    })
                })
                .collect::<Result<_>>()?,
            address_table_lookups: raw["addressTableLookups"]
                .as_array()
                .context("missing lookup descriptors")?
                .iter()
                .map(|l| {
                    Ok(MessageAddressTableLookup {
                        account_key: l["accountKey"]
                            .as_str()
                            .context("missing LUT pubkey")?
                            .parse()?,
                        writable_indexes: indexes(&l["writableIndexes"])?,
                        readonly_indexes: indexes(&l["readonlyIndexes"])?,
                    })
                })
                .collect::<Result<_>>()?,
        };
        message
            .sanitize()
            .map_err(|e| anyhow::anyhow!("official v0 sanitization: {e:?}"))?;
        let normalized = transactions::normalize(result)?;
        let rpc_loaded = LoadedAddresses {
            writable: addresses(&result["meta"]["loadedAddresses"]["writable"])?,
            readonly: addresses(&result["meta"]["loadedAddresses"]["readonly"])?,
        };
        Ok(Self {
            signature: normalized.signature.clone(),
            slot: normalized.slot,
            genesis: genesis.into(),
            message,
            rpc_loaded,
            normalized,
            rpc_result_sha256: hash_bytes(&serde_json::to_vec(result)?),
        })
    }
    pub fn native_message(&self) -> &v0::Message {
        &self.message
    }
    pub fn execution_slot(&self) -> u64 {
        self.slot
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableResolution {
    pub table_pubkey: String,
    pub evidence_id: String,
    pub raw_account_sha256: String,
    pub raw_response_sha256: String,
    pub provider: ArchiveProvenance,
    pub requested_slot: u64,
    pub returned_context_slot: u64,
    pub metadata: LookupTableMeta,
    pub addresses: Vec<Address>,
    pub active_addresses_len: usize,
    pub writable_indexes: Vec<u8>,
    pub readonly_indexes: Vec<u8>,
    pub resolved_writable: Vec<Address>,
    pub resolved_readonly: Vec<Address>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LutResolutionProof {
    pub schema_version: u32,
    pub signature: String,
    pub execution_slot: u64,
    pub genesis_hash: String,
    pub rpc_result_sha256: String,
    pub native_v0_message: v0::Message,
    pub tables: Vec<TableResolution>,
    pub slot_hashes_evidence_id: Option<String>,
    pub resolved_writable: Vec<Address>,
    pub resolved_readonly: Vec<Address>,
    /// Requested privileges, before runtime program/reserved-key demotion.
    pub full_account_keys: Vec<AccountMetaSpec>,
    pub compiled_instructions_match: bool,
    pub metadata_match: bool,
    pub proof_id: String,
}
/// Cannot be constructed by deserialization or a claimed proof flag. The native
/// message and immutable normalized transaction exist only after reconstruction.
#[derive(Clone, Debug)]
pub struct ProvenV0 {
    proof: LutResolutionProof,
    transaction: HistoricalTransaction,
}
impl ProvenV0 {
    pub fn proof(&self) -> &LutResolutionProof {
        &self.proof
    }
    pub fn transaction(&self) -> &HistoricalTransaction {
        &self.transaction
    }
    pub fn versioned_message(&self) -> VersionedMessage {
        VersionedMessage::V0(self.proof.native_v0_message.clone())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LutProofFailure {
    pub stage: u8,
    pub detail: String,
}
fn stage<T>(number: u8, result: Result<T>) -> std::result::Result<T, LutProofFailure> {
    result.map_err(|e| LutProofFailure {
        stage: number,
        detail: format!("{e:#}"),
    })
}
fn historical_slot_hashes(
    evidence: Option<&HistoricalAccountEvidence>,
    slot: u64,
    genesis: &str,
) -> Result<SlotHashes> {
    let evidence =
        evidence.context("historical SlotHashes evidence required for prior deactivation")?;
    ensure!(evidence.pubkey == SLOT_HASHES_ID, "wrong SlotHashes pubkey");
    let account = evidence.account(slot, genesis)?;
    ensure!(
        account.owner == SYSVAR_OWNER && !account.executable,
        "wrong SlotHashes owner"
    );
    let hashes: SlotHashes =
        wincode::deserialize(&account.data).context("invalid SlotHashes bytes")?;
    ensure!(
        hashes.len() <= solana_slot_hashes::MAX_ENTRIES
            && hashes.iter().all(|(s, _)| *s < slot)
            && hashes.windows(2).all(|p| p[0].0 > p[1].0),
        "invalid execution-bank SlotHashes order/context"
    );
    Ok(hashes)
}

pub fn reconstruct(
    frozen: &FrozenV0,
    evidence: &[HistoricalAccountEvidence],
    slot_hashes: Option<&HistoricalAccountEvidence>,
) -> std::result::Result<ProvenV0, LutProofFailure> {
    let mut by_key = BTreeMap::new();
    for item in evidence {
        stage(
            1,
            (|| {
                ensure!(
                    by_key.insert(item.pubkey.as_str(), item).is_none(),
                    "duplicate historical LUT evidence"
                );
                Ok(())
            })(),
        )?;
    }
    let wanted: HashSet<_> = frozen
        .message
        .address_table_lookups
        .iter()
        .map(|l| l.account_key.to_string())
        .collect();
    stage(
        1,
        (|| {
            ensure!(
                by_key.len() == wanted.len() && by_key.keys().all(|k| wanted.contains(*k)),
                "missing or extra historical LUT evidence"
            );
            Ok(())
        })(),
    )?;
    let mut tables = Vec::new();
    let mut loaded = LoadedAddresses::default();
    let mut used_slot_hashes = false;
    for lookup in &frozen.message.address_table_lookups {
        let key = lookup.account_key.to_string();
        let item = stage(
            1,
            by_key
                .get(key.as_str())
                .copied()
                .context("historical LUT state unavailable"),
        )?;
        let account = stage(2, item.account(frozen.slot, &frozen.genesis))?;
        let (table, hashes, active_addresses_len) = stage(
            2,
            (|| {
                ensure!(
                    account.owner == program::id().to_string()
                        && !account.executable
                        && account.lamports > 0,
                    "wrong LUT owner/executable/closed state"
                );
                let table = AddressLookupTable::deserialize(&account.data)
                    .map_err(|e| anyhow::anyhow!("official LUT decode: {e:?}"))?;
                ensure!(
                    table.addresses.len() <= 256
                        && usize::from(table.meta.last_extended_slot_start_index)
                            <= table.addresses.len(),
                    "invalid LUT metadata/address length"
                );
                ensure!(
                    table.meta.last_extended_slot <= frozen.slot
                        && (table.meta.deactivation_slot == u64::MAX
                            || table.meta.deactivation_slot <= frozen.slot),
                    "historical LUT metadata is from a future slot"
                );
                let hashes = if table.meta.deactivation_slot != u64::MAX
                    && table.meta.deactivation_slot < frozen.slot
                {
                    used_slot_hashes = true;
                    historical_slot_hashes(slot_hashes, frozen.slot, &frozen.genesis)?
                } else {
                    SlotHashes::default()
                };
                let active = table
                    .get_active_addresses_len(frozen.slot, &hashes)
                    .map_err(|e| anyhow::anyhow!("official LUT visibility: {e:?}"))?;
                Ok((table, hashes, active))
            })(),
        )?;
        let writable_indexes = lookup.writable_indexes.clone();
        let readonly_indexes = lookup.readonly_indexes.clone();
        let (writable, readonly) = stage(
            3,
            (|| {
                let writable = table
                    .lookup(frozen.slot, &writable_indexes, &hashes)
                    .map_err(|e| anyhow::anyhow!("writable lookup: {e:?}"))?;
                let readonly = table
                    .lookup(frozen.slot, &readonly_indexes, &hashes)
                    .map_err(|e| anyhow::anyhow!("readonly lookup: {e:?}"))?;
                Ok((writable, readonly))
            })(),
        )?;
        loaded.writable.extend_from_slice(&writable);
        loaded.readonly.extend_from_slice(&readonly);
        tables.push(TableResolution {
            table_pubkey: key,
            evidence_id: item.evidence_id.clone(),
            raw_account_sha256: item.raw_account_sha256.clone(),
            raw_response_sha256: item.raw_response_sha256.clone(),
            provider: item.provider.clone(),
            requested_slot: item.requested_slot,
            returned_context_slot: item.returned_context_slot,
            metadata: table.meta.clone(),
            addresses: table.addresses.to_vec(),
            active_addresses_len,
            writable_indexes,
            readonly_indexes,
            resolved_writable: writable,
            resolved_readonly: readonly,
        });
    }
    stage(
        4,
        (|| {
            ensure!(
                loaded == frozen.rpc_loaded,
                "independent loaded addresses differ from frozen RPC (length/order/pubkey)"
            );
            Ok(())
        })(),
    )?;
    let official_loaded = LoadedMessage::new_borrowed(&frozen.message, &loaded, &HashSet::new());
    let mut full = Vec::new();
    let header = &frozen.message.header;
    for (i, key) in official_loaded.account_keys().iter().enumerate() {
        let is_signer = official_loaded.is_signer(i);
        let is_writable = if i < frozen.message.account_keys.len() {
            if is_signer {
                i < usize::from(
                    header.num_required_signatures - header.num_readonly_signed_accounts,
                )
            } else {
                i < frozen.message.account_keys.len()
                    - usize::from(header.num_readonly_unsigned_accounts)
            }
        } else {
            i - frozen.message.account_keys.len() < loaded.writable.len()
        };
        full.push(AccountMetaSpec {
            address: key.to_string(),
            is_signer,
            is_writable,
        });
    }
    stage(
        5,
        (|| {
            ensure!(
                !official_loaded.has_duplicates(),
                "official runtime rejects AccountLoadedTwice; no deduplication"
            );
            ensure!(
                full == frozen.normalized.account_keys,
                "full key space/privileges differ from frozen normalization"
            );
            ensure!(
                full.iter()
                    .skip(frozen.message.account_keys.len())
                    .all(|k| !k.is_signer),
                "loaded address became signer"
            );
            Ok(())
        })(),
    )?;
    stage(
        6,
        (|| {
            ensure!(
                frozen.message.instructions.len() == frozen.normalized.instructions.len(),
                "compiled instruction count differs"
            );
            for (ix, expected) in frozen
                .message
                .instructions
                .iter()
                .zip(&frozen.normalized.instructions)
            {
                let program = full
                    .get(usize::from(ix.program_id_index))
                    .context("compiled program index out of range")?;
                let accounts = ix
                    .accounts
                    .iter()
                    .map(|i| {
                        full.get(usize::from(*i))
                            .cloned()
                            .context("compiled account index out of range")
                    })
                    .collect::<Result<Vec<_>>>()?;
                ensure!(
                    program.address == expected.program
                        && accounts == expected.accounts
                        && ix.data == expected.data,
                    "compiled instruction identity/privileges/data differ"
                );
            }
            Ok(())
        })(),
    )?;
    let mut proof = LutResolutionProof {
        schema_version: 1,
        signature: frozen.signature.clone(),
        execution_slot: frozen.slot,
        genesis_hash: frozen.genesis.clone(),
        rpc_result_sha256: frozen.rpc_result_sha256.clone(),
        native_v0_message: frozen.message.clone(),
        tables,
        slot_hashes_evidence_id: if used_slot_hashes {
            slot_hashes.map(|s| s.evidence_id.clone())
        } else {
            None
        },
        resolved_writable: loaded.writable,
        resolved_readonly: loaded.readonly,
        full_account_keys: full,
        compiled_instructions_match: true,
        metadata_match: true,
        proof_id: String::new(),
    };
    proof.proof_id = stage(
        7,
        serde_json::to_vec(&proof)
            .map(|bytes| hash_bytes(&bytes))
            .map_err(Into::into),
    )?;
    Ok(ProvenV0 {
        proof,
        transaction: frozen.normalized.clone(),
    })
}

/// Serialized status is not authority: rebuild from raw inputs and compare every
/// proof byte before returning the sealed executable-message value.
pub fn validate_proof(
    frozen: &FrozenV0,
    evidence: &[HistoricalAccountEvidence],
    slot_hashes: Option<&HistoricalAccountEvidence>,
    claimed: &LutResolutionProof,
) -> std::result::Result<ProvenV0, LutProofFailure> {
    let verified = reconstruct(frozen, evidence, slot_hashes)?;
    stage(
        7,
        (|| {
            ensure!(
                &verified.proof == claimed,
                "serialized LUT proof differs from independent reconstruction"
            );
            Ok(())
        })(),
    )?;
    Ok(verified)
}
