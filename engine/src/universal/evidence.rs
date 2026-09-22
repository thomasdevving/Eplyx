//! Bundle-local content-addressed evidence with temporal observations.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ingest::accounts, message::ArchiveProvenance, replay::hash_bytes, types::AccountSnapshot,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    AccountContent,
    AccountObservation,
    DerivedAccount,
    AccountChunk,
    ChunkedAccountObservation,
    Checkpoint,
    ClosureProof,
    Execution,
    ResponseTemplate,
    Transaction,
    ProgramBinary,
    Runtime,
    Validator,
}

impl EvidenceKind {
    fn directory(self) -> &'static str {
        match self {
            Self::AccountContent => "accounts/content",
            Self::AccountObservation => "accounts/observations",
            Self::DerivedAccount => "accounts/derived",
            Self::AccountChunk => "accounts/chunks",
            Self::ChunkedAccountObservation => "accounts/chunked-observations",
            Self::Checkpoint => "checkpoints",
            Self::ClosureProof => "closure",
            Self::Execution => "execution",
            Self::ResponseTemplate => "accounts/responses",
            Self::Transaction => "transactions",
            Self::ProgramBinary => "programs",
            Self::Runtime => "runtime",
            Self::Validator => "validator",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub sha256: String,
}

impl EvidenceRef {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.sha256.len() == 64 && self.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid evidence hash"
        );
        Ok(())
    }
}

pub struct EvidenceStore {
    root: PathBuf,
}

impl EvidenceStore {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, reference: &EvidenceRef) -> Result<PathBuf> {
        reference.validate()?;
        Ok(self
            .root
            .join(reference.kind.directory())
            .join(&reference.sha256))
    }

    pub fn put(&self, kind: EvidenceKind, bytes: &[u8]) -> Result<EvidenceRef> {
        let reference = EvidenceRef {
            kind,
            sha256: hash_bytes(bytes),
        };
        let path = self.path(&reference)?;
        fs::create_dir_all(path.parent().expect("evidence category directory"))?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure!(
                    self.get(&reference)? == bytes,
                    "existing evidence object differs from content hash"
                );
            }
            Err(error) => return Err(error.into()),
        }
        Ok(reference)
    }

    pub fn get(&self, reference: &EvidenceRef) -> Result<Vec<u8>> {
        let path = self.path(reference)?;
        let bytes = fs::read(&path)
            .with_context(|| format!("missing evidence object {}", path.display()))?;
        ensure!(
            hash_bytes(&bytes) == reference.sha256,
            "evidence object hash differs: {}",
            path.display()
        );
        Ok(bytes)
    }
}

/// The requested historical boundary is part of observation identity, while
/// identical account content may be shared by observations at different slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountBoundary {
    BeforeTransaction,
    BeforeTargetExecution,
    EndOfExecutionSlot,
}

impl AccountBoundary {
    fn requested_slot(self, transaction_slot: u64) -> Result<u64> {
        match self {
            Self::BeforeTransaction => transaction_slot
                .checked_sub(1)
                .context("transaction has no predecessor slot"),
            Self::BeforeTargetExecution => Ok(transaction_slot),
            Self::EndOfExecutionSlot => Ok(transaction_slot),
        }
    }
}

/// The raw provider response is split around its base64 account-data string.
/// Reassembling it from the shared account content reproduces the original
/// response byte for byte, including whitespace and field order, so the wire
/// hash remains independently verifiable without repeating multi-MiB data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ResponseTemplate {
    prefix_base64: String,
    suffix_base64: String,
    raw_sha256: String,
    split_data: bool,
}

impl ResponseTemplate {
    fn from_raw(raw: &[u8], content: Option<&AccountSnapshot>) -> Result<Self> {
        let Some(content) = content else {
            return Ok(Self::whole(raw));
        };
        Self::from_raw_data(raw, &content.data)
    }

    fn from_raw_data(raw: &[u8], data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Ok(Self::whole(raw));
        }
        let needle = serde_json::to_vec(&BASE64_STANDARD.encode(data))?;
        let text = std::str::from_utf8(raw).context("archived response is not UTF-8 JSON")?;
        let pattern = std::str::from_utf8(&needle).expect("JSON base64 string is UTF-8");
        let positions: Vec<_> = text
            .match_indices(pattern)
            .map(|(i, _)| i)
            .take(2)
            .collect();
        ensure!(
            positions.len() == 1,
            "cannot uniquely split archived account data"
        );
        let start = positions[0];
        Ok(Self {
            prefix_base64: BASE64_STANDARD.encode(&raw[..start]),
            suffix_base64: BASE64_STANDARD.encode(&raw[start + needle.len()..]),
            raw_sha256: hash_bytes(raw),
            split_data: true,
        })
    }

    fn whole(raw: &[u8]) -> Self {
        Self {
            prefix_base64: BASE64_STANDARD.encode(raw),
            suffix_base64: String::new(),
            raw_sha256: hash_bytes(raw),
            split_data: false,
        }
    }

    fn restore(&self, content: Option<&AccountSnapshot>) -> Result<Vec<u8>> {
        self.restore_data(content.map(|a| a.data.as_slice()))
    }

    fn restore_data(&self, data: Option<&[u8]>) -> Result<Vec<u8>> {
        let mut raw = BASE64_STANDARD.decode(&self.prefix_base64)?;
        if self.split_data {
            let data = data.context("split response has no account content")?;
            raw.extend_from_slice(&serde_json::to_vec(&BASE64_STANDARD.encode(data))?);
        }
        raw.extend_from_slice(&BASE64_STANDARD.decode(&self.suffix_base64)?);
        ensure!(
            hash_bytes(&raw) == self.raw_sha256,
            "archived response bytes differ"
        );
        Ok(raw)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountChunk {
    pub offset: u64,
    pub data: EvidenceRef,
    pub response_template: EvidenceRef,
    pub raw_response_sha256: String,
}

/// A historical account captured by exact-slot `dataSlice` requests. Each
/// interval is tied back to the retained request receipt, not merely to a
/// declared offset. Complete raw bytes are reconstructed and hashed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkedAccountObservation {
    pub address: String,
    pub transaction_slot: u64,
    pub requested_slot: u64,
    pub returned_context_slot: u64,
    pub provider: ArchiveProvenance,
    pub receipts: Vec<EvidenceRef>,
    pub content: EvidenceRef,
    pub chunks: Vec<AccountChunk>,
    pub identity: String,
}

impl ChunkedAccountObservation {
    fn identity(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.identity.clear();
        Ok(hash_bytes(&serde_json::to_vec(&copy)?))
    }

    pub fn capture(
        store: &EvidenceStore,
        address: &str,
        transaction_slot: u64,
        provider: ArchiveProvenance,
        receipt_raws: &[Vec<u8>],
        slices: &[(u64, Vec<u8>)],
    ) -> Result<EvidenceRef> {
        provider.validate()?;
        address.parse::<solana_address::Address>()?;
        let requested = transaction_slot
            .checked_sub(1)
            .context("transaction has no predecessor slot")?;
        ensure!(
            !receipt_raws.is_empty(),
            "historical account has no request receipt"
        );
        let mut receipts = Vec::new();
        let mut receipt_values = Vec::new();
        for raw in receipt_raws {
            let receipt_value: Value = serde_json::from_slice(raw)?;
            ensure!(
                receipt_genesis(&receipt_value) == Some(provider.genesis_hash.as_str())
                    && receipt_value["provider"].as_str() == Some(provider.scheme_host.as_str()),
                "archive receipt provider or genesis differs"
            );
            receipts.push(store.put(EvidenceKind::Validator, raw)?);
            receipt_values.push(receipt_value);
        }
        let mut ordered = slices.to_vec();
        ordered.sort_by_key(|(offset, _)| *offset);
        ensure!(!ordered.is_empty(), "chunked account has no slices");
        let mut chunks = Vec::new();
        let mut assembled = Vec::new();
        let mut metadata: Option<AccountSnapshot> = None;
        let mut space = None;
        for (offset, raw) in ordered {
            ensure!(
                offset == assembled.len() as u64,
                "missing or overlapping historical account byte interval"
            );
            let parsed: Value = serde_json::from_slice(&raw)?;
            ensure!(
                parsed.get("error").is_none()
                    && parsed["result"]["context"]["slot"].as_u64() == Some(requested),
                "chunk response uses wrong historical context"
            );
            let value = &parsed["result"]["value"];
            let part = accounts::normalize(value)?;
            let size = value["space"]
                .as_u64()
                .context("chunk response missing full account space")?;
            if let Some(previous) = &metadata {
                ensure!(
                    previous.owner == part.owner
                        && previous.lamports == part.lamports
                        && previous.executable == part.executable
                        && previous.rent_epoch == part.rent_epoch
                        && space == Some(size),
                    "historical account chunk metadata differs"
                );
            } else {
                metadata = Some(part.clone());
                space = Some(size);
            }
            let raw_hash = hash_bytes(&raw);
            ensure!(
                receipt_values.iter().any(|receipt| receipt_contains(
                    receipt, address, requested, offset, &raw_hash
                )),
                "chunk has no matching exact-slot dataSlice request receipt"
            );
            let data = store.put(EvidenceKind::AccountChunk, &part.data)?;
            let template = ResponseTemplate::from_raw_data(&raw, &part.data)?;
            let response_template = store.put(
                EvidenceKind::ResponseTemplate,
                &serde_json::to_vec(&template)?,
            )?;
            assembled.extend_from_slice(&part.data);
            chunks.push(AccountChunk {
                offset,
                data,
                response_template,
                raw_response_sha256: raw_hash,
            });
        }
        ensure!(
            space == Some(assembled.len() as u64),
            "historical account chunks do not cover full space"
        );
        let mut full = metadata.expect("nonempty slices");
        full.data = assembled;
        let content = store.put(EvidenceKind::AccountContent, &serde_json::to_vec(&full)?)?;
        let mut observation = Self {
            address: address.into(),
            transaction_slot,
            requested_slot: requested,
            returned_context_slot: requested,
            provider,
            receipts,
            content,
            chunks,
            identity: String::new(),
        };
        observation.identity = observation.identity()?;
        store.put(
            EvidenceKind::ChunkedAccountObservation,
            &serde_json::to_vec(&observation)?,
        )
    }

    pub fn resolve(
        store: &EvidenceStore,
        reference: &EvidenceRef,
        address: &str,
        transaction_slot: u64,
        genesis: &str,
    ) -> Result<AccountSnapshot> {
        ensure!(
            reference.kind == EvidenceKind::ChunkedAccountObservation,
            "chunked account reference has wrong kind"
        );
        let observation: Self = serde_json::from_slice(&store.get(reference)?)?;
        ensure!(
            observation.identity == observation.identity()?,
            "chunked observation identity differs"
        );
        let requested = transaction_slot
            .checked_sub(1)
            .context("transaction has no predecessor slot")?;
        ensure!(
            observation.address == address
                && observation.transaction_slot == transaction_slot
                && observation.requested_slot == requested
                && observation.returned_context_slot == requested,
            "chunked account address or historical context differs"
        );
        observation.provider.validate()?;
        ensure!(
            observation.provider.genesis_hash == genesis,
            "chunked account genesis differs"
        );
        ensure!(
            !observation.receipts.is_empty()
                && observation
                    .receipts
                    .iter()
                    .all(|r| r.kind == EvidenceKind::Validator)
                && observation.content.kind == EvidenceKind::AccountContent,
            "chunked account evidence category differs"
        );
        let receipts = observation
            .receipts
            .iter()
            .map(|reference| {
                let receipt: Value = serde_json::from_slice(&store.get(reference)?)?;
                ensure!(
                    receipt_genesis(&receipt) == Some(genesis)
                        && receipt["provider"].as_str()
                            == Some(observation.provider.scheme_host.as_str()),
                    "chunked account receipt identity differs"
                );
                Ok(receipt)
            })
            .collect::<Result<Vec<_>>>()?;
        let full: AccountSnapshot = serde_json::from_slice(&store.get(&observation.content)?)?;
        let mut assembled = Vec::new();
        ensure!(
            !observation.chunks.is_empty(),
            "chunked account has no slices"
        );
        for chunk in &observation.chunks {
            ensure!(
                chunk.offset == assembled.len() as u64
                    && chunk.data.kind == EvidenceKind::AccountChunk
                    && chunk.response_template.kind == EvidenceKind::ResponseTemplate,
                "chunk offset or evidence category differs"
            );
            let part = store.get(&chunk.data)?;
            let template: ResponseTemplate =
                serde_json::from_slice(&store.get(&chunk.response_template)?)?;
            let raw = template.restore_data(Some(&part))?;
            ensure!(
                hash_bytes(&raw) == chunk.raw_response_sha256
                    && receipts.iter().any(|receipt| receipt_contains(
                        receipt,
                        address,
                        requested,
                        chunk.offset,
                        &chunk.raw_response_sha256
                    )),
                "chunk response/receipt identity differs"
            );
            let parsed: Value = serde_json::from_slice(&raw)?;
            ensure!(
                parsed.get("error").is_none()
                    && parsed["result"]["context"]["slot"].as_u64() == Some(requested),
                "chunk response has wrong context"
            );
            let value = &parsed["result"]["value"];
            let slice = accounts::normalize(value)?;
            ensure!(
                slice.data == part
                    && slice.owner == full.owner
                    && slice.lamports == full.lamports
                    && slice.executable == full.executable
                    && slice.rent_epoch == full.rent_epoch
                    && value["space"].as_u64() == Some(full.data.len() as u64),
                "chunk content or metadata differs"
            );
            assembled.extend_from_slice(&part);
        }
        ensure!(
            assembled == full.data,
            "reassembled historical account differs from content"
        );
        Ok(full)
    }
}

fn receipt_contains(
    receipt: &Value,
    address: &str,
    slot: u64,
    offset: u64,
    response_sha256: &str,
) -> bool {
    let entries = receipt["requests"]
        .as_array()
        .or_else(|| receipt["attempts"].as_array());
    entries.is_some_and(|requests| {
        requests.iter().any(|request| {
            request["method"] == "getAccountInfo"
                && request
                    .get("status")
                    .map(|status| status == "success")
                    .unwrap_or_else(|| request.get("failure_class").is_some_and(Value::is_null))
                && request["params"][0] == address
                && request["params"][1]["slot"].as_u64() == Some(slot)
                && request["params"][1]["dataSlice"]["offset"]
                    .as_u64()
                    .unwrap_or(0)
                    == offset
                && (request["response_sha256"] == response_sha256
                    || request["body_sha256"] == response_sha256)
        })
    })
}

fn receipt_genesis(receipt: &Value) -> Option<&str> {
    receipt["genesis"]
        .as_str()
        .or_else(|| receipt["qualification"]["genesis_hash"].as_str())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountObservation {
    pub address: String,
    pub transaction_slot: u64,
    pub boundary: AccountBoundary,
    pub requested_slot: u64,
    pub returned_context_slot: u64,
    pub provider: ArchiveProvenance,
    pub content: Option<EvidenceRef>,
    pub response_template: EvidenceRef,
    pub raw_response_sha256: String,
    pub identity: String,
}

impl AccountObservation {
    fn identity(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.identity.clear();
        Ok(hash_bytes(&serde_json::to_vec(&copy)?))
    }

    pub fn capture(
        store: &EvidenceStore,
        address: &str,
        transaction_slot: u64,
        boundary: AccountBoundary,
        provider: ArchiveProvenance,
        raw: &[u8],
    ) -> Result<EvidenceRef> {
        ensure!(
            boundary != AccountBoundary::BeforeTargetExecution,
            "archived account cannot claim before-target execution"
        );
        provider.validate()?;
        address.parse::<solana_address::Address>()?;
        let requested = boundary.requested_slot(transaction_slot)?;
        let parsed: Value =
            serde_json::from_slice(raw).context("invalid archived account response")?;
        ensure!(
            parsed.get("error").is_none(),
            "archived account response reports an error"
        );
        let context = parsed["result"]["context"]["slot"]
            .as_u64()
            .context("archived response has no context slot")?;
        ensure!(
            context == requested,
            "archive returned the wrong historical context"
        );
        let value = &parsed["result"]["value"];
        let account = if value.is_null() {
            None
        } else {
            Some(accounts::normalize(value)?)
        };
        let content = account
            .as_ref()
            .map(|a| store.put(EvidenceKind::AccountContent, &serde_json::to_vec(a)?))
            .transpose()?;
        let template = ResponseTemplate::from_raw(raw, account.as_ref())?;
        let response_template = store.put(
            EvidenceKind::ResponseTemplate,
            &serde_json::to_vec(&template)?,
        )?;
        let mut observation = Self {
            address: address.into(),
            transaction_slot,
            boundary,
            requested_slot: requested,
            returned_context_slot: context,
            provider,
            content,
            response_template,
            raw_response_sha256: hash_bytes(raw),
            identity: String::new(),
        };
        observation.identity = observation.identity()?;
        store.put(
            EvidenceKind::AccountObservation,
            &serde_json::to_vec(&observation)?,
        )
    }

    pub fn resolve(
        store: &EvidenceStore,
        reference: &EvidenceRef,
        address: &str,
        transaction_slot: u64,
        boundary: AccountBoundary,
        genesis: &str,
    ) -> Result<Option<AccountSnapshot>> {
        Ok(Self::resolve_with_raw(
            store,
            reference,
            address,
            transaction_slot,
            boundary,
            genesis,
        )?
        .0)
    }

    pub fn resolve_with_raw(
        store: &EvidenceStore,
        reference: &EvidenceRef,
        address: &str,
        transaction_slot: u64,
        boundary: AccountBoundary,
        genesis: &str,
    ) -> Result<(Option<AccountSnapshot>, Vec<u8>)> {
        ensure!(
            boundary != AccountBoundary::BeforeTargetExecution,
            "archived account cannot claim before-target execution"
        );
        ensure!(
            reference.kind == EvidenceKind::AccountObservation,
            "account observation reference has wrong kind"
        );
        let observation: Self = serde_json::from_slice(&store.get(reference)?)?;
        ensure!(
            observation.identity == observation.identity()?,
            "account observation identity differs"
        );
        ensure!(
            observation.address == address
                && observation.transaction_slot == transaction_slot
                && observation.boundary == boundary,
            "account address or historical boundary differs"
        );
        let requested = boundary.requested_slot(transaction_slot)?;
        ensure!(
            observation.requested_slot == requested
                && observation.returned_context_slot == requested,
            "account observation uses the wrong historical context"
        );
        ensure!(
            observation.provider.genesis_hash == genesis,
            "account observation genesis differs"
        );
        observation.provider.validate()?;
        let content = observation
            .content
            .as_ref()
            .map(|r| {
                ensure!(
                    r.kind == EvidenceKind::AccountContent,
                    "account content reference has wrong kind"
                );
                serde_json::from_slice::<AccountSnapshot>(&store.get(r)?).map_err(Into::into)
            })
            .transpose()?;
        ensure!(
            observation.response_template.kind == EvidenceKind::ResponseTemplate,
            "response template has wrong kind"
        );
        let template: ResponseTemplate =
            serde_json::from_slice(&store.get(&observation.response_template)?)?;
        let raw = template.restore(content.as_ref())?;
        ensure!(
            hash_bytes(&raw) == observation.raw_response_sha256,
            "account wire response hash differs"
        );
        let parsed: Value = serde_json::from_slice(&raw)?;
        ensure!(
            parsed.get("error").is_none()
                && parsed["result"]["context"]["slot"].as_u64() == Some(requested),
            "archived account response context differs"
        );
        let value = &parsed["result"]["value"];
        let verified = if value.is_null() {
            None
        } else {
            Some(accounts::normalize(value)?)
        };
        ensure!(
            verified == content,
            "archived account response differs from shared content"
        );
        Ok((content, raw))
    }
}
