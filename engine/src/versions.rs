//! Historical program version resolution.
//!
//! Phase 6 pinned V1 by reading an immutable legacy-loader program account: the
//! executable bytes live directly in the program account, and legacy-loader
//! deployments cannot be replaced, so "the bytes at slot S" and "the bytes now"
//! are the same question. Neither holds for an upgradeable program.
//!
//! Under the upgradeable loader the program account holds only a pointer to a
//! ProgramData account, and that account is rewritten in place on every upgrade.
//! Resolving a version therefore means reading ProgramData *at a slot*, which is
//! exactly what a slot-addressable account archive provides.
//!
//! The header also records the slot of the deployment that produced the current
//! bytes. That field is what makes upgrade discovery cheap: as a function of the
//! query slot it is a monotone non-decreasing step function, so a binary search
//! over slots recovers the upgrade boundaries without scanning any blocks.

use crate::{
    ingest::{accounts, rpc::RpcProvider},
    replay::hash_bytes,
    types::AccountSnapshot,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const LEGACY_BPF_LOADER_ID: &str = "BPFLoader2111111111111111111111111111111111";
pub const UPGRADEABLE_LOADER_ID: &str = "BPFLoaderUpgradeab1e11111111111111111111111";
/// Owner of a program the validator implements itself. There is no artefact to
/// read: the semantics live in the runtime, not in an account.
pub const NATIVE_LOADER_ID: &str = "NativeLoader1111111111111111111111111111111";

/// `UpgradeableLoaderState` discriminants, as serialized by the loader.
const STATE_PROGRAM: u32 = 2;
const STATE_PROGRAM_DATA: u32 = 3;

/// `UpgradeableLoaderState::Program` is a 4-byte tag followed by the ProgramData
/// address. The loader sizes the account at exactly this.
const PROGRAM_STATE_LEN: usize = 36;

/// `UpgradeableLoaderState::ProgramData` is a 4-byte tag, the deployment slot,
/// and an optional upgrade authority. Executable bytes begin immediately after.
const PROGRAMDATA_HEADER_LEN: usize = 45;

/// Raw bytes per archive request when reading a large ProgramData account.
/// Mainnet ProgramData accounts run to 10 MiB, which is past what a single
/// base64 JSON-RPC response handles comfortably, so reads are chunked. The
/// chunk boundary is part of the cache key, so changing it re-fetches.
const CHUNK: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramLoader {
    /// Bytes live in the program account and can never be replaced.
    Legacy,
    /// Bytes live in a separate ProgramData account and are rewritten on upgrade.
    Upgradeable,
}

/// One program version, as it existed at one slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployedProgram {
    pub program_id: String,
    pub loader: ProgramLoader,
    /// Slot this version was read at, not the slot it was deployed at.
    pub observed_slot: u64,
    /// Slot of the deployment that produced these bytes. Upgradeable only:
    /// legacy-loader accounts do not record it.
    pub deploy_slot: Option<u64>,
    pub upgrade_authority: Option<String>,
    pub programdata_address: Option<String>,
    pub sha256: String,
    /// The buffer the loader hands the VM: for the upgradeable loader this is
    /// ProgramData minus its 45-byte header, including any trailing padding the
    /// deployer reserved. Trimming it would change the artefact away from the
    /// bytes mainnet actually executed.
    #[serde(skip)]
    pub elf: Vec<u8>,
}

impl DeployedProgram {
    /// Whether two resolutions refer to the same deployed bytes.
    pub fn same_bytes_as(&self, other: &Self) -> bool {
        self.sha256 == other.sha256
    }
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

/// Read one account at an exact slot. The archive must honour the slot rather
/// than silently answering at its own commitment; anything else would quietly
/// substitute current state for history.
fn account_at(rpc: &dyn RpcProvider, address: &str, slot: u64) -> Result<AccountSnapshot> {
    let response = rpc.call(
        "getAccountInfo",
        json!([address, {"encoding":"base64","commitment":"finalized","slot":slot}]),
    )?;
    anyhow::ensure!(
        response["context"]["slot"].as_u64() == Some(slot),
        "account archive did not honor exact requested slot {slot} for {address}"
    );
    accounts::normalize(&response["value"])
        .with_context(|| format!("historical account {address} at slot {slot}"))
}

/// Read `length` bytes at `offset` from one account at an exact slot.
fn slice_at(
    rpc: &dyn RpcProvider,
    address: &str,
    slot: u64,
    offset: usize,
    length: usize,
) -> Result<(AccountSnapshot, u64)> {
    let response = rpc.call(
        "getAccountInfo",
        json!([address, {
            "encoding":"base64",
            "commitment":"finalized",
            "slot":slot,
            "dataSlice":{"offset":offset,"length":length},
        }]),
    )?;
    anyhow::ensure!(
        response["context"]["slot"].as_u64() == Some(slot),
        "account archive did not honor exact requested slot {slot} for {address}"
    );
    // `space` is the account's full data length, independent of the slice, and
    // is how a chunked read learns how far it has to go.
    let space = response["value"]["space"]
        .as_u64()
        .context("archive response omitted account data length")?;
    let snapshot = accounts::normalize(&response["value"])
        .with_context(|| format!("historical account {address} at slot {slot}"))?;
    Ok((snapshot, space))
}

/// Read a whole account at an exact slot, in bounded chunks.
///
/// The first request doubles as the header read, so a small account costs one
/// request and a 10 MiB one costs twenty rather than a single oversized body.
fn whole_account_at(
    rpc: &dyn RpcProvider,
    address: &str,
    slot: u64,
) -> Result<(AccountSnapshot, Vec<u8>)> {
    let (head, space) = slice_at(rpc, address, slot, 0, CHUNK)?;
    let space =
        usize::try_from(space).context("account larger than this platform's address space")?;
    let mut data = head.data.clone();
    while data.len() < space {
        let (chunk, chunk_space) = slice_at(rpc, address, slot, data.len(), CHUNK)?;
        anyhow::ensure!(
            chunk_space as usize == space,
            "account {address} changed length mid-read at slot {slot}"
        );
        anyhow::ensure!(
            !chunk.data.is_empty(),
            "archive returned an empty chunk for {address} at slot {slot}"
        );
        data.extend_from_slice(&chunk.data);
    }
    anyhow::ensure!(
        data.len() == space,
        "chunked read of {address} produced {} bytes, expected {space}",
        data.len()
    );
    Ok((
        AccountSnapshot {
            data: data.clone(),
            ..head
        },
        data,
    ))
}

/// What kind of program an address held at one slot.
///
/// The distinction matters for replay: a loader-owned program has bytes that can
/// be read and re-executed, while a runtime-implemented one does not, and
/// substituting a stand-in for the latter would change historical semantics
/// silently. Classification is therefore taken from the account's owner at the
/// slot in question rather than from a hard-coded list of addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgramResolution {
    /// Implemented by the validator; no artefact exists to load.
    Native,
    /// Loader-owned, with exact bytes for the deployment live at the slot.
    Deployed(Box<DeployedProgram>),
}

/// Classify and, where applicable, resolve the program live at `slot`.
pub fn resolve_program_at(
    rpc: &dyn RpcProvider,
    program_id: &str,
    slot: u64,
) -> Result<ProgramResolution> {
    let program = account_at(rpc, program_id, slot)?;
    anyhow::ensure!(
        program.executable,
        "account {program_id} was not executable at slot {slot}"
    );
    if program.owner == NATIVE_LOADER_ID {
        return Ok(ProgramResolution::Native);
    }
    resolve_deployed(rpc, program_id, slot, program)
        .map(|program| ProgramResolution::Deployed(Box::new(program)))
}

/// Resolve the program version that was live at `slot`.
pub fn resolve_at(rpc: &dyn RpcProvider, program_id: &str, slot: u64) -> Result<DeployedProgram> {
    let program = account_at(rpc, program_id, slot)?;
    anyhow::ensure!(
        program.executable,
        "account {program_id} was not executable at slot {slot}"
    );
    resolve_deployed(rpc, program_id, slot, program)
}

fn resolve_deployed(
    rpc: &dyn RpcProvider,
    program_id: &str,
    slot: u64,
    program: AccountSnapshot,
) -> Result<DeployedProgram> {
    if program.owner == LEGACY_BPF_LOADER_ID {
        anyhow::ensure!(
            program.data.starts_with(b"\x7fELF"),
            "legacy-loader program {program_id} does not contain SBF ELF bytes at slot {slot}"
        );
        return Ok(DeployedProgram {
            program_id: program_id.into(),
            loader: ProgramLoader::Legacy,
            observed_slot: slot,
            deploy_slot: None,
            upgrade_authority: None,
            programdata_address: None,
            sha256: hash_bytes(&program.data),
            elf: program.data,
        });
    }

    anyhow::ensure!(
        program.owner == UPGRADEABLE_LOADER_ID,
        "program {program_id} uses unsupported loader {} at slot {slot}",
        program.owner
    );
    anyhow::ensure!(
        program.data.len() == PROGRAM_STATE_LEN && u32_at(&program.data, 0) == Some(STATE_PROGRAM),
        "program account {program_id} is not UpgradeableLoaderState::Program at slot {slot}"
    );
    let programdata_address = bs58::encode(&program.data[4..PROGRAM_STATE_LEN]).into_string();

    let (programdata, data) = whole_account_at(rpc, &programdata_address, slot)?;
    anyhow::ensure!(
        programdata.owner == UPGRADEABLE_LOADER_ID && !programdata.executable,
        "ProgramData {programdata_address} is not owned by the upgradeable loader at slot {slot}"
    );
    anyhow::ensure!(
        u32_at(&data, 0) == Some(STATE_PROGRAM_DATA),
        "account {programdata_address} is not UpgradeableLoaderState::ProgramData at slot {slot}"
    );
    let deploy_slot = u64_at(&data, 4).context("ProgramData header is truncated")?;
    let upgrade_authority = match data.get(12) {
        Some(1) => Some(
            bs58::encode(
                data.get(13..PROGRAMDATA_HEADER_LEN)
                    .context("ProgramData upgrade authority is truncated")?,
            )
            .into_string(),
        ),
        // A `None` authority means the deployment was made immutable. That is a
        // stronger guarantee than the legacy loader's, not a weaker one.
        Some(0) => None,
        _ => anyhow::bail!("ProgramData {programdata_address} has an invalid authority flag"),
    };
    let elf = data
        .get(PROGRAMDATA_HEADER_LEN..)
        .context("ProgramData contains no executable bytes")?
        .to_vec();
    anyhow::ensure!(
        elf.starts_with(b"\x7fELF"),
        "ProgramData {programdata_address} does not contain SBF ELF bytes at slot {slot}"
    );
    anyhow::ensure!(
        deploy_slot <= slot,
        "ProgramData {programdata_address} reports a deployment at slot {deploy_slot}, \
         which is later than the queried slot {slot}"
    );

    Ok(DeployedProgram {
        program_id: program_id.into(),
        loader: ProgramLoader::Upgradeable,
        observed_slot: slot,
        deploy_slot: Some(deploy_slot),
        upgrade_authority,
        programdata_address: Some(programdata_address),
        sha256: hash_bytes(&elf),
        elf,
    })
}

/// Read just the deployment slot recorded at `slot`, without pulling bytecode.
///
/// Upgrade search calls this once per binary-search probe, so it deliberately
/// reads the 45-byte header alone rather than megabytes of ELF.
pub fn deploy_slot_at(rpc: &dyn RpcProvider, programdata_address: &str, slot: u64) -> Result<u64> {
    let (snapshot, _) = slice_at(rpc, programdata_address, slot, 0, PROGRAMDATA_HEADER_LEN)?;
    anyhow::ensure!(
        u32_at(&snapshot.data, 0) == Some(STATE_PROGRAM_DATA),
        "account {programdata_address} is not ProgramData at slot {slot}"
    );
    u64_at(&snapshot.data, 4).context("ProgramData header is truncated")
}

/// One upgrade boundary: the program changed between `before` and `at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeBoundary {
    pub programdata_address: String,
    /// Last slot at which the previous deployment was live.
    pub before: u64,
    /// First slot at which the new deployment was live. This equals the
    /// deployment slot the loader recorded.
    pub at: u64,
    pub previous_deploy_slot: u64,
    pub deploy_slot: u64,
}

/// Find every upgrade boundary in `(low, high]` by bisection.
///
/// The recorded deployment slot only ever increases with the query slot, so a
/// range whose endpoints agree contains no upgrade and needs no further probing.
/// That turns "find the upgrades in a 20-million-slot window" into a handful of
/// 45-byte reads per boundary instead of a block scan.
pub fn find_upgrades(
    rpc: &dyn RpcProvider,
    programdata_address: &str,
    low: u64,
    high: u64,
) -> Result<Vec<UpgradeBoundary>> {
    anyhow::ensure!(low < high, "upgrade search needs a non-empty slot range");
    let low_deploy = deploy_slot_at(rpc, programdata_address, low)?;
    let high_deploy = deploy_slot_at(rpc, programdata_address, high)?;
    let mut found = Vec::new();
    bisect(
        rpc,
        programdata_address,
        low,
        low_deploy,
        high,
        high_deploy,
        &mut found,
    )?;
    found.sort_by_key(|boundary| boundary.at);
    Ok(found)
}

fn bisect(
    rpc: &dyn RpcProvider,
    programdata_address: &str,
    low: u64,
    low_deploy: u64,
    high: u64,
    high_deploy: u64,
    found: &mut Vec<UpgradeBoundary>,
) -> Result<()> {
    if low_deploy == high_deploy {
        return Ok(());
    }
    if low + 1 == high {
        found.push(UpgradeBoundary {
            programdata_address: programdata_address.into(),
            before: low,
            at: high,
            previous_deploy_slot: low_deploy,
            deploy_slot: high_deploy,
        });
        return Ok(());
    }
    let mid = low + (high - low) / 2;
    let mid_deploy = deploy_slot_at(rpc, programdata_address, mid)?;
    bisect(
        rpc,
        programdata_address,
        low,
        low_deploy,
        mid,
        mid_deploy,
        found,
    )?;
    bisect(
        rpc,
        programdata_address,
        mid,
        mid_deploy,
        high,
        high_deploy,
        found,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Deterministic archive stand-in: a step function from slot to deployment.
    struct FakeArchive {
        /// (first slot at which it is live, deployment slot) pairs, ascending.
        deployments: Vec<(u64, u64)>,
        probes: AtomicUsize,
    }

    impl FakeArchive {
        fn deploy_for(&self, slot: u64) -> u64 {
            self.deployments
                .iter()
                .rev()
                .find(|(live_from, _)| slot >= *live_from)
                .map(|(_, deploy)| *deploy)
                .expect("slot precedes the first deployment")
        }
    }

    impl RpcProvider for FakeArchive {
        fn call(&self, method: &str, params: Value) -> Result<Value> {
            assert_eq!(method, "getAccountInfo");
            self.probes.fetch_add(1, Ordering::Relaxed);
            let slot = params[1]["slot"].as_u64().expect("slot selector");
            let mut data = Vec::new();
            data.extend_from_slice(&STATE_PROGRAM_DATA.to_le_bytes());
            data.extend_from_slice(&self.deploy_for(slot).to_le_bytes());
            data.push(0);
            data.resize(PROGRAMDATA_HEADER_LEN, 0);
            Ok(json!({
                "context": {"slot": slot},
                "value": {
                    "data": [base64_encode(&data), "base64"],
                    "owner": UPGRADEABLE_LOADER_ID,
                    "lamports": 1_u64,
                    "executable": false,
                    "rentEpoch": 0_u64,
                    "space": data.len() as u64,
                },
            }))
        }
    }

    fn base64_encode(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::prelude::BASE64_STANDARD.encode(bytes)
    }

    fn archive(deployments: &[(u64, u64)]) -> FakeArchive {
        FakeArchive {
            deployments: deployments.to_vec(),
            probes: AtomicUsize::new(0),
        }
    }

    #[test]
    fn a_range_without_an_upgrade_reports_no_boundary() {
        let rpc = archive(&[(0, 100)]);
        assert!(find_upgrades(&rpc, "pd", 1_000, 2_000).unwrap().is_empty());
    }

    #[test]
    fn bisection_locates_an_upgrade_to_the_exact_slot() {
        let rpc = archive(&[(0, 100), (1_500, 1_500)]);
        let found = find_upgrades(&rpc, "pd", 1_000, 2_000).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].before, 1_499);
        assert_eq!(found[0].at, 1_500);
        assert_eq!(found[0].previous_deploy_slot, 100);
        assert_eq!(found[0].deploy_slot, 1_500);
    }

    #[test]
    fn bisection_locates_every_upgrade_in_a_range() {
        let rpc = archive(&[(0, 10), (1_200, 1_200), (1_700, 1_700), (1_900, 1_900)]);
        let found = find_upgrades(&rpc, "pd", 1_000, 2_000).unwrap();
        assert_eq!(
            found.iter().map(|b| b.at).collect::<Vec<_>>(),
            vec![1_200, 1_700, 1_900]
        );
    }

    /// The point of bisection is that cost tracks boundaries, not slot count.
    #[test]
    fn searching_a_wide_range_costs_far_less_than_scanning_it() {
        let rpc = archive(&[(0, 10), (500_000, 500_000)]);
        let found = find_upgrades(&rpc, "pd", 0, 1_000_000).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].at, 500_000);
        let probes = rpc.probes.load(Ordering::Relaxed);
        assert!(
            probes < 64,
            "expected a logarithmic probe count, used {probes}"
        );
    }

    #[test]
    fn an_empty_range_is_rejected_rather_than_silently_returning_nothing() {
        let rpc = archive(&[(0, 10)]);
        assert!(find_upgrades(&rpc, "pd", 2_000, 2_000).is_err());
    }
}
