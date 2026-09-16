//! Exact historical-state acquisition for one deliberately bounded mainnet path.
//!
//! Standard Solana RPC supplies transaction metadata but not arbitrary old
//! account bytes. This adapter consumes a slot-addressable account archive and
//! proves its snapshots against validator pre/post balances before constructing
//! a replay record. Anything approximate or ambiguous is rejected.

use crate::{
    dependencies,
    ingest::{
        accounts,
        rpc::RpcProvider,
        transactions::{self, HistoricalTransaction},
    },
    replay::{
        hash_bytes, outcome_hash, state_hash, AccountAcquisition, AccountDiscovery,
        AccountStateSource, OriginalExecution, PostAccountDigest, ReplayClock, ReplayRecord,
        ReplayStateSource, MEMO_PROGRAM_ID, REPLAY_SCHEMA, SYSTEM_PROGRAM_ID,
    },
    screening,
    types::{AccountSnapshot, NamedAccount},
};
use anyhow::{Context, Result};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub use crate::versions::LEGACY_BPF_LOADER_ID;

#[derive(Clone, Debug)]
pub struct HistoricalAcquisition {
    pub record: ReplayRecord,
    /// Exact executable bytes read from the immutable historical program
    /// account. These become the V1 fidelity-gate artifact.
    pub v1_program: Vec<u8>,
    /// Exact bytes of every other program the replay has to load, keyed by
    /// program ID. Written beside the corpus so the comparison runs offline.
    pub dependency_binaries: BTreeMap<String, Vec<u8>>,
}

pub trait HistoricalStateProvider {
    fn acquire_exact(&self, signature: &str) -> Result<HistoricalAcquisition>;
}

/// Provider for archive APIs that extend `getAccountInfo` with an exact `slot`
/// selector. The transaction and state transports remain separate so users can
/// combine a standard transaction archive with a dedicated account archive.
pub struct SlotAccountArchiveProvider<'a> {
    pub transaction_rpc: &'a dyn RpcProvider,
    pub account_archive_rpc: &'a dyn RpcProvider,
}

fn account_at(rpc: &dyn RpcProvider, address: &str, slot: u64) -> Result<AccountSnapshot> {
    let response = rpc.call(
        "getAccountInfo",
        json!([address,{"encoding":"base64","commitment":"finalized","slot":slot}]),
    )?;
    anyhow::ensure!(
        response["context"]["slot"].as_u64() == Some(slot),
        "account archive did not honor exact requested slot {slot} for {address}"
    );
    accounts::normalize(&response["value"])
        .with_context(|| format!("historical account {address} at slot {slot}"))
}

/// Read an account that the message references but that may not exist.
///
/// A key with no account is ordinary on Solana - an unfunded signer is the
/// common case - and the runtime materializes it as an empty System-owned
/// account. Recording it as absent is more faithful than inventing a snapshot,
/// so this returns `None` and the caller proves the absence against metadata.
fn optional_account_at(
    rpc: &dyn RpcProvider,
    address: &str,
    slot: u64,
) -> Result<Option<AccountSnapshot>> {
    let response = rpc.call(
        "getAccountInfo",
        json!([address,{"encoding":"base64","commitment":"finalized","slot":slot}]),
    )?;
    anyhow::ensure!(
        response["context"]["slot"].as_u64() == Some(slot),
        "account archive did not honor exact requested slot {slot} for {address}"
    );
    if response["value"].is_null() {
        return Ok(None);
    }
    accounts::normalize(&response["value"])
        .with_context(|| format!("historical account {address} at slot {slot}"))
        .map(Some)
}

fn prove_account_boundaries(
    transaction: &HistoricalTransaction,
    index: usize,
    pre: &AccountSnapshot,
    post: &AccountSnapshot,
) -> Result<()> {
    let address = &transaction.account_keys[index].address;
    let pre_balances = transaction
        .pre_balances
        .as_ref()
        .context("transaction metadata omitted pre-balances")?;
    let post_balances = transaction
        .post_balances
        .as_ref()
        .context("transaction metadata omitted post-balances")?;
    anyhow::ensure!(
        pre_balances.get(index) == Some(&pre.lamports),
        "slot-before archive state does not equal transaction pre-state for {address}"
    );
    anyhow::ensure!(
        post_balances.get(index) == Some(&post.lamports),
        "slot-end archive state does not equal transaction post-state for {address}"
    );
    anyhow::ensure!(
        pre.owner == post.owner
            && pre.data == post.data
            && pre.executable == post.executable
            && pre.rent_epoch == post.rent_epoch,
        "unsupported non-lamport state change for {address}"
    );
    anyhow::ensure!(
        pre.owner == SYSTEM_PROGRAM_ID && pre.data.is_empty() && !pre.executable,
        "bounded Memo adapter accepts data-empty System accounts only"
    );
    Ok(())
}

impl HistoricalStateProvider for SlotAccountArchiveProvider<'_> {
    fn acquire_exact(&self, signature: &str) -> Result<HistoricalAcquisition> {
        anyhow::ensure!(
            bs58::decode(signature).into_vec()?.len() == 64,
            "invalid transaction signature"
        );
        let transaction_genesis = self
            .transaction_rpc
            .call("getGenesisHash", json!([]))?
            .as_str()
            .context("transaction RPC missing genesis hash")?
            .to_string();
        let archive_genesis = self
            .account_archive_rpc
            .call("getGenesisHash", json!([]))?
            .as_str()
            .context("account archive missing genesis hash")?
            .to_string();
        anyhow::ensure!(
            transaction_genesis == archive_genesis,
            "transaction and account providers are on different chains"
        );

        let raw = self.transaction_rpc.call(
            "getTransaction",
            json!([signature,{"encoding":"json","commitment":"finalized","maxSupportedTransactionVersion":0}]),
        )?;
        let transaction = transactions::normalize(&raw)?;
        anyhow::ensure!(
            transaction.signature == signature,
            "RPC returned a different transaction"
        );
        anyhow::ensure!(transaction.slot > 0, "transaction has no predecessor slot");

        // Reuse the durable record validator as the execution-contract gate.
        // A temporary shell record lets that gate reject versions, CPI, extra
        // programs, and non-System-transfer Memo shapes before archive fanout.
        anyhow::ensure!(
            transaction.version == "legacy"
                && transaction.success
                && transaction.inner_instructions.is_empty()
                && transaction.instructions.len() == 2
                && transaction.instructions[0].program == SYSTEM_PROGRAM_ID
                && transaction.instructions[1].program == MEMO_PROGRAM_ID,
            "transaction is outside the bounded legacy System-transfer/Memo contract"
        );

        let pre_slot = transaction.slot - 1;
        let program = account_at(self.account_archive_rpc, MEMO_PROGRAM_ID, pre_slot)?;
        anyhow::ensure!(
            program.owner == LEGACY_BPF_LOADER_ID && program.executable,
            "historical Memo program is not an immutable legacy-loader executable"
        );
        anyhow::ensure!(
            program.data.starts_with(b"\x7fELF"),
            "historical Memo program account does not contain SBF ELF bytes"
        );

        let mut pre_accounts = Vec::new();
        let mut post_accounts = Vec::new();
        for (index, label) in [(0_usize, "payer"), (1_usize, "recipient")] {
            let address = &transaction.account_keys[index].address;
            let pre = account_at(self.account_archive_rpc, address, pre_slot)?;
            let post = account_at(self.account_archive_rpc, address, transaction.slot)?;
            prove_account_boundaries(&transaction, index, &pre, &post)?;
            pre_accounts.push(NamedAccount {
                label: label.into(),
                address: address.clone(),
                account: pre,
            });
            post_accounts.push(NamedAccount {
                label: label.into(),
                address: address.clone(),
                account: post,
            });
        }

        let id_material = serde_json::to_vec(&(
            "historical-mainnet-memo-v1",
            &transaction_genesis,
            signature,
            transaction.slot,
        ))?;
        let record = ReplayRecord {
            schema_version: REPLAY_SCHEMA,
            id: format!("mainnet-memo-{}", &hash_bytes(&id_material)[..20]),
            program_id: MEMO_PROGRAM_ID.into(),
            genesis_hash: transaction_genesis,
            clock: ReplayClock {
                slot: transaction.slot,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: transaction.block_time.context("missing block time")?,
            },
            state_source: ReplayStateSource::HistoricalArchive,
            pre_state_hash: state_hash(&pre_accounts)?,
            original: Some(OriginalExecution {
                success: transaction.success,
                fee: transaction.fee,
                post_state_hash: outcome_hash(&post_accounts)?,
                post_accounts: post_accounts.iter().map(PostAccountDigest::of).collect(),
                cpi_invocations: Vec::new(),
            }),
            current_program_sha256: hash_bytes(&program.data),
            dependencies: Default::default(),
            acquisitions: Vec::new(),
            slot_screening: None,
            accounts: pre_accounts,
            transaction,
            assumptions: vec![
                "slot-addressable archive snapshots at S-1 and S match validator pre/post balances"
                    .into(),
                "bounded contract is one System transfer followed by account-free Memo; no CPI"
                    .into(),
                "both state accounts are data-empty System accounts; only lamports may change"
                    .into(),
                "Memo is an immutable legacy-loader executable; archived account data is exact V1 SBF"
                    .into(),
                "System and Memo do not read Clock; remaining runtime state uses pinned LiteSVM defaults"
                    .into(),
            ],
        };
        record.validate()?;
        Ok(HistoricalAcquisition {
            record,
            v1_program: program.data,
            dependency_binaries: BTreeMap::new(),
        })
    }
}

/// Adapter-driven acquisition for a stateful production protocol.
///
/// Phase 6's provider hard-coded one transaction shape and one immutable
/// program. This one delegates both questions: the adapter decides what it can
/// replay and what its accounts mean, and [`crate::versions`] resolves whichever
/// binary was actually deployed at the transaction's slot. That is what makes
/// V1 the code that really produced the recorded outcome rather than whatever
/// is deployed today, and it is why the fidelity gate downstream is a genuine
/// check instead of a tautology.
pub struct ProtocolArchiveProvider<'a> {
    pub transaction_rpc: &'a dyn RpcProvider,
    pub account_archive_rpc: &'a dyn RpcProvider,
    /// Block source for same-slot interference screening.
    ///
    /// Optional because the adapters that predate CPI replay were proved
    /// without it, through a boundary check that fails late. An adapter whose
    /// contract admits CPI has too many accounts with too uneven evidence for
    /// that to be enough, so acquisition refuses to proceed without a block.
    pub block_rpc: Option<&'a dyn RpcProvider>,
    /// Program whose upgrade is under test. Its adapter must be compiled in.
    pub program_id: &'a str,
}

impl HistoricalStateProvider for ProtocolArchiveProvider<'_> {
    fn acquire_exact(&self, signature: &str) -> Result<HistoricalAcquisition> {
        anyhow::ensure!(
            bs58::decode(signature).into_vec()?.len() == 64,
            "invalid transaction signature"
        );
        let adapter = crate::protocol::adapter_for(self.program_id)
            .with_context(|| format!("no protocol adapter for program {}", self.program_id))?;

        let genesis = |rpc: &dyn RpcProvider, label: &str| -> Result<String> {
            Ok(rpc
                .call("getGenesisHash", json!([]))?
                .as_str()
                .with_context(|| format!("{label} RPC missing genesis hash"))?
                .to_string())
        };
        let transaction_genesis = genesis(self.transaction_rpc, "transaction")?;
        let archive_genesis = genesis(self.account_archive_rpc, "account archive")?;
        anyhow::ensure!(
            transaction_genesis == archive_genesis,
            "transaction and account providers are on different chains"
        );
        if let Some(rpc) = self.block_rpc {
            anyhow::ensure!(
                genesis(rpc, "block")? == transaction_genesis,
                "block and transaction providers are on different chains"
            );
        }

        let raw = self.transaction_rpc.call(
            "getTransaction",
            json!([signature,{"encoding":"json","commitment":"finalized","maxSupportedTransactionVersion":0}]),
        )?;
        let transaction = transactions::normalize(&raw)?;
        anyhow::ensure!(
            transaction.signature == signature,
            "RPC returned a different transaction"
        );
        anyhow::ensure!(transaction.slot > 0, "transaction has no predecessor slot");
        // Reject the shape before spending archive requests on it.
        adapter.accept(&transaction)?;
        anyhow::ensure!(
            transaction
                .account_keys
                .iter()
                .any(|key| key.address == self.program_id),
            "transaction does not reference program {}",
            self.program_id
        );

        let pre_slot = transaction.slot - 1;
        // The version that was live when the original transaction executed.
        let v1 = crate::versions::resolve_at(self.account_archive_rpc, self.program_id, pre_slot)?;

        // Everything else that executes, pinned to the deployment live at the
        // same slot. Resolving these is what keeps the replay a replay: the
        // dependency an aggregator called last quarter is not necessarily the
        // binary sitting at that address today.
        let (manifest, dependency_binaries) = dependencies::resolve(
            self.account_archive_rpc,
            &transaction,
            Some(adapter),
            self.program_id,
            pre_slot,
        )?;

        // Every key that is not executed is state this replay has to reproduce.
        // The executed set is the union of every discovery route, not the
        // top-level instruction list: a program reached only through CPI is
        // still a program, and treating it as state would try to snapshot an
        // executable.
        let invoked: BTreeSet<&str> = manifest
            .programs
            .iter()
            .map(|program| program.program_id.as_str())
            .collect();
        let meta_named: BTreeSet<&str> = transaction
            .instructions
            .iter()
            .flat_map(|instruction| instruction.accounts.iter())
            .map(|meta| meta.address.as_str())
            .collect();
        let inner_named: BTreeSet<&str> = transaction
            .inner_instructions
            .iter()
            .flat_map(|instruction| instruction.accounts.iter())
            .map(|meta| meta.address.as_str())
            .collect();
        let adapter_named: BTreeSet<String> = adapter
            .required_accounts(&transaction)
            .into_iter()
            .collect();
        let mut pre_accounts = Vec::new();
        let mut post_accounts = Vec::new();
        let mut acquisitions = Vec::new();
        let mut absent: Vec<String> = Vec::new();
        for (index, key) in transaction.account_keys.iter().enumerate() {
            if invoked.contains(key.address.as_str()) || key.address == self.program_id {
                continue;
            }
            let label = adapter.label(&transaction, index);
            let mut discovered_by = Vec::new();
            if meta_named.contains(key.address.as_str()) {
                discovered_by.push(AccountDiscovery::InstructionMeta);
            }
            if inner_named.contains(key.address.as_str()) {
                discovered_by.push(AccountDiscovery::InnerInstruction);
            }
            if adapter_named.contains(&key.address) {
                discovered_by.push(AccountDiscovery::AdapterDependency);
            }
            if discovered_by.is_empty() {
                discovered_by.push(AccountDiscovery::MessageKey);
            }
            let pre = optional_account_at(self.account_archive_rpc, &key.address, pre_slot)?;
            let post =
                optional_account_at(self.account_archive_rpc, &key.address, transaction.slot)?;
            let (pre, post) = match (pre, post) {
                (Some(pre), Some(post)) => (pre, post),
                (None, None) => {
                    // Absent on both sides. The validator must agree it held
                    // nothing, otherwise the archive is missing real state.
                    let zero = |balances: Option<&Vec<u64>>| {
                        balances.and_then(|values| values.get(index)) == Some(&0)
                    };
                    anyhow::ensure!(
                        zero(transaction.pre_balances.as_ref())
                            && zero(transaction.post_balances.as_ref()),
                        "account {} is absent from the archive but the validator recorded a \
                         balance for it",
                        key.address
                    );
                    acquisitions.push(AccountAcquisition {
                        address: key.address.clone(),
                        label,
                        discovered_by,
                        source: AccountStateSource::AbsentAtBothBoundaries,
                        context_slot: pre_slot,
                        method: "getAccountInfo at exact slots S-1 and S returned no account; \
                                 validator balances confirm it held nothing"
                            .into(),
                    });
                    absent.push(key.address.clone());
                    continue;
                }
                _ => anyhow::bail!(
                    "account {} exists on only one side of the transaction boundary; \
                     account creation and closure are outside the supported contract",
                    key.address
                ),
            };
            anyhow::ensure!(
                !pre.executable && !post.executable,
                "state account {} is executable",
                key.address
            );

            // An account whose whole state is its lamport balance can take that
            // balance from the transaction's own metadata, which is exact for
            // this transaction even when the slot-boundary snapshot is not.
            // Everything else about it - owner, emptiness - still comes from the
            // archive and must agree on both sides.
            let balance_at = |balances: Option<&Vec<u64>>| -> Option<u64> {
                balances.and_then(|values| values.get(index)).copied()
            };
            let (mut pre, mut post) = (pre, post);
            let metadata = crate::replay::reconstructable_from_balances(&pre, &post)
                .then(|| {
                    balance_at(transaction.pre_balances.as_ref())
                        .zip(balance_at(transaction.post_balances.as_ref()))
                })
                .flatten();
            let source = match metadata {
                Some((pre_lamports, post_lamports)) => {
                    pre.lamports = pre_lamports;
                    post.lamports = post_lamports;
                    AccountStateSource::TransactionBalanceMetadata
                }
                None => AccountStateSource::HistoricalArchive,
            };
            acquisitions.push(AccountAcquisition {
                address: key.address.clone(),
                label: label.clone(),
                discovered_by,
                source,
                context_slot: pre_slot,
                method: match source {
                    AccountStateSource::TransactionBalanceMetadata => {
                        "System-owned, non-executable and empty at both archive boundaries; \
                         lamports taken from this transaction's preBalances/postBalances"
                            .into()
                    }
                    _ => "getAccountInfo at exact slot, honored by the archive, at S-1 and S"
                        .to_string(),
                },
            });
            pre_accounts.push(NamedAccount {
                label: label.clone(),
                address: key.address.clone(),
                account: pre,
            });
            post_accounts.push(NamedAccount {
                label,
                address: key.address.clone(),
                account: post,
            });
        }

        // Screen the slot before proving boundaries, so a rejection names the
        // conflicting transaction rather than only the account whose numbers
        // failed to line up.
        // Only accounts whose boundary rests on the archive need an
        // unambiguous slot. One reconstructed from transaction metadata is
        // already exact for this transaction, so another writer in the same
        // slot cannot spoil it and screening it would reject a record that is
        // fully evidenced.
        let reconstructed: BTreeSet<&str> = acquisitions
            .iter()
            .filter(|a| a.source == AccountStateSource::TransactionBalanceMetadata)
            .map(|a| a.address.as_str())
            .collect();
        let required: BTreeSet<String> = pre_accounts
            .iter()
            .map(|named| named.address.clone())
            .filter(|address| !reconstructed.contains(address.as_str()))
            .collect();
        let slot_screening = match self.block_rpc {
            Some(rpc) => {
                let screening = screening::screen(rpc, transaction.slot, signature, &required)?;
                screening.ensure_unambiguous()?;
                Some(screening)
            }
            None => {
                anyhow::ensure!(
                    !adapter.supports_cpi(),
                    "{} replay requires same-slot interference screening; supply a block \
                     source so conflicting transactions can be identified",
                    adapter.name()
                );
                None
            }
        };

        // The adapter proves its own snapshots and reports what the proof rests
        // on, so the record carries the limits of its own evidence.
        let mut assumptions =
            adapter.prove_boundaries(&transaction, &pre_accounts, &post_accounts)?;
        if !absent.is_empty() {
            assumptions.push(format!(
                "{} message key(s) held no account at either boundary and the validator \
                 recorded a zero balance for each; the runtime materializes them as empty \
                 System-owned accounts, matching the original execution",
                absent.len()
            ));
        }
        assumptions.push(format!(
            "V1 is the {} deployment live at slot {}, read from ProgramData at slot {}",
            match v1.deploy_slot {
                Some(slot) => format!("slot-{slot}"),
                None => "immutable legacy-loader".into(),
            },
            transaction.slot,
            pre_slot
        ));
        let loaded = manifest
            .programs
            .iter()
            .filter(|program| program.requires_artifact() && program.program_id != self.program_id)
            .count();
        let builtin = manifest
            .programs
            .iter()
            .filter(|program| program.source == dependencies::ProgramSource::Builtin)
            .count();
        if loaded + builtin > 0 {
            assumptions.push(format!(
                "{loaded} dependency program(s) are loaded from the deployment live at slot \
                 {pre_slot}, not from today's; {builtin} are implemented by the runtime and \
                 nothing is substituted for them"
            ));
        }
        if let Some(screening) = &slot_screening {
            assumptions.push(format!(
                "no other transaction among the {} in slot {} takes any of the {} required \
                 accounts as writable",
                screening.transactions_in_slot,
                transaction.slot,
                screening.required_accounts.len()
            ));
        }
        assumptions.push(
            "the runtime feature set is LiteSVM's mainnet snapshot rather than a slot-accurate \
             reconstruction; both builds run under the identical set, and V1 reproducing the \
             original outcome - its fee, its invocation graph and its post-state - is what \
             rules out a material difference"
                .into(),
        );

        let id_material = serde_json::to_vec(&(
            "historical-mainnet-protocol-v1",
            adapter.name(),
            &transaction_genesis,
            signature,
            transaction.slot,
        ))?;
        let record = ReplayRecord {
            schema_version: REPLAY_SCHEMA,
            id: format!(
                "mainnet-{}-{}",
                adapter.name(),
                &hash_bytes(&id_material)[..16]
            ),
            program_id: self.program_id.to_string(),
            genesis_hash: transaction_genesis,
            clock: ReplayClock {
                slot: transaction.slot,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: transaction.block_time.context("missing block time")?,
            },
            state_source: ReplayStateSource::HistoricalArchive,
            pre_state_hash: state_hash(&pre_accounts)?,
            original: Some(OriginalExecution {
                success: transaction.success,
                fee: transaction.fee,
                post_state_hash: outcome_hash(&post_accounts)?,
                post_accounts: post_accounts.iter().map(PostAccountDigest::of).collect(),
                cpi_invocations: transaction.inner_instruction_frames.clone(),
            }),
            current_program_sha256: v1.sha256.clone(),
            dependencies: manifest,
            acquisitions,
            slot_screening,
            accounts: pre_accounts,
            transaction,
            assumptions,
        };
        record.validate()?;
        let dependency_binaries = dependency_binaries
            .into_iter()
            .filter(|(program_id, _)| program_id != self.program_id)
            .collect();
        Ok(HistoricalAcquisition {
            record,
            v1_program: v1.elf,
            dependency_binaries,
        })
    }
}
