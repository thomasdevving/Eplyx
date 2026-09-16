//! Which executable programs a historical transaction actually needs.
//!
//! Through Phase 7 the answer was trivial, because the supported contract
//! excluded CPI: the only program that ran was the one under test, and every
//! other message key was state. A transaction that invokes other programs makes
//! the question real, and getting it wrong is silent. Load today's build of a
//! dependency into a replay of last quarter's transaction and the replay still
//! succeeds - it just stops being a replay of what happened.
//!
//! So dependencies are discovered from evidence rather than assumed, and each
//! one is pinned to the deployment that was live at the transaction's slot.
//! Programs the validator implements itself are recorded as such and no artefact
//! is substituted for them, because there is nothing to substitute: their
//! semantics live in the runtime, and a stand-in would change history quietly.

use crate::{
    ingest::{rpc::RpcProvider, transactions::HistoricalTransaction},
    protocol::ProtocolAdapter,
    versions::{self, ProgramLoader, ProgramResolution},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How a required program came to light.
///
/// Recorded because the discovery routes have different strength: a top-level
/// instruction is in the signed message, while a log line is the validator
/// reporting what actually executed. Keeping them distinct is what lets a
/// reader see that nothing was assumed into the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyDiscovery {
    /// Named as the program of a top-level instruction.
    TopLevelInstruction,
    /// Named as the program of an inner (CPI) instruction in validator metadata.
    InnerInstruction,
    /// Observed invoking in the captured execution log.
    ExecutionLog,
    /// Declared by the protocol adapter as part of its supported contract.
    AdapterDeclared,
    /// The program whose upgrade is under test.
    ProgramUnderTest,
}

impl DependencyDiscovery {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TopLevelInstruction => "top-level instruction",
            Self::InnerInstruction => "inner instruction",
            Self::ExecutionLog => "execution log",
            Self::AdapterDeclared => "adapter declared",
            Self::ProgramUnderTest => "program under test",
        }
    }
}

/// Where the bytes that execute for one program come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramSource {
    /// The validator implements this program. Nothing is loaded, and nothing
    /// may be substituted for it.
    Builtin,
    /// Exact bytes of the deployment that was live at the transaction's slot.
    HistoricalMainnet,
    /// Replaced at execution time by the artefact under test.
    CandidateOverride,
    /// Required, recognized, and outside what this build can reproduce.
    Unsupported,
}

impl ProgramSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::HistoricalMainnet => "historical-mainnet",
            Self::CandidateOverride => "candidate-override",
            Self::Unsupported => "unsupported",
        }
    }
}

/// One program the replay environment has to provide.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramDependency {
    pub program_id: String,
    pub source: ProgramSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<ProgramLoader>,
    /// Slot of the deployment that produced these bytes, where the loader
    /// records one. Legacy-loader deployments cannot be replaced, so they have
    /// no deployment slot and need none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_slot: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_len: Option<u64>,
    /// Slot the version was read at: the transaction's predecessor slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_slot: Option<u64>,
    pub discovered_by: Vec<DependencyDiscovery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ProgramDependency {
    /// File name for this dependency's artefact in a replay bundle.
    ///
    /// Addressed by program ID rather than by a human name so that a bundle
    /// cannot quietly pair one program's bytes with another's manifest entry.
    pub fn artifact_file_name(&self) -> String {
        format!("{}.so", self.program_id)
    }

    pub fn requires_artifact(&self) -> bool {
        self.source == ProgramSource::HistoricalMainnet
    }
}

/// The complete set of programs one replay needs, in a canonical order.
///
/// Ordered by program ID rather than by discovery order so that two runs over
/// the same transaction produce byte-identical manifests.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyManifest {
    pub programs: Vec<ProgramDependency>,
}

impl DependencyManifest {
    pub fn get(&self, program_id: &str) -> Option<&ProgramDependency> {
        self.programs
            .iter()
            .find(|program| program.program_id == program_id)
    }

    pub fn unsupported(&self) -> Vec<&ProgramDependency> {
        self.programs
            .iter()
            .filter(|program| program.source == ProgramSource::Unsupported)
            .collect()
    }

    /// Dependencies whose bytes must be supplied to execute.
    pub fn loadable(&self) -> impl Iterator<Item = &ProgramDependency> {
        self.programs
            .iter()
            .filter(|program| program.requires_artifact())
    }
}

/// Parse `Program <id> invoke [n]` out of one captured log line.
///
/// Log scraping is a discovery route only. Nothing is executed because a log
/// mentioned it; the manifest still has to resolve the program at the slot, and
/// the fidelity gate still has to reproduce the original outcome.
fn invoked_in_log(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("Program ")?;
    let (address, tail) = rest.split_once(' ')?;
    tail.starts_with("invoke [").then_some(address)
}

/// Every program the transaction is evidenced to need, with its provenance.
///
/// Deliberately a union of independent routes. Top-level metas alone are not
/// sufficient - that is the whole point of a CPI-aware replay - and neither are
/// inner instructions, because a program that is invoked but whose metadata was
/// truncated would vanish from the set.
pub fn discover(
    transaction: &HistoricalTransaction,
    adapter: Option<&dyn ProtocolAdapter>,
    program_under_test: &str,
) -> Vec<(String, Vec<DependencyDiscovery>)> {
    let mut found: BTreeMap<String, Vec<DependencyDiscovery>> = BTreeMap::new();
    let mut record = |address: &str, how: DependencyDiscovery| {
        let entry = found.entry(address.to_string()).or_default();
        if !entry.contains(&how) {
            entry.push(how);
        }
    };
    record(program_under_test, DependencyDiscovery::ProgramUnderTest);
    for instruction in &transaction.instructions {
        record(
            &instruction.program,
            DependencyDiscovery::TopLevelInstruction,
        );
    }
    for instruction in &transaction.inner_instructions {
        record(&instruction.program, DependencyDiscovery::InnerInstruction);
    }
    for line in &transaction.logs {
        if let Some(address) = invoked_in_log(line) {
            // A log line is free-form text from the program itself further in,
            // so only accept something that is actually an address.
            if address.parse::<solana_address::Address>().is_ok() {
                record(address, DependencyDiscovery::ExecutionLog);
            }
        }
    }
    if let Some(adapter) = adapter {
        for address in adapter.dependency_programs() {
            record(address, DependencyDiscovery::AdapterDeclared);
        }
    }
    found.into_iter().collect()
}

/// Resolve every discovered dependency at the transaction's predecessor slot.
///
/// `slot` is `S-1` rather than `S`: the binaries that executed are the ones live
/// when the slot began. Reading at `S` would pick up an upgrade landing in the
/// same slot, which is exactly the class of error this phase exists to rule out.
pub fn resolve(
    rpc: &dyn RpcProvider,
    transaction: &HistoricalTransaction,
    adapter: Option<&dyn ProtocolAdapter>,
    program_under_test: &str,
    slot: u64,
) -> Result<(DependencyManifest, BTreeMap<String, Vec<u8>>)> {
    let mut programs = Vec::new();
    let mut binaries = BTreeMap::new();
    for (program_id, discovered_by) in discover(transaction, adapter, program_under_test) {
        let resolution = versions::resolve_program_at(rpc, &program_id, slot)
            .with_context(|| format!("resolving dependency {program_id} at slot {slot}"))?;
        match resolution {
            ProgramResolution::Native => programs.push(ProgramDependency {
                program_id,
                source: ProgramSource::Builtin,
                loader: None,
                deployed_slot: None,
                binary_sha256: None,
                binary_len: None,
                observed_slot: Some(slot),
                discovered_by,
                note: Some(
                    "owned by the native loader at this slot; the validator implements it \
                     and no artefact is loaded"
                        .into(),
                ),
            }),
            ProgramResolution::Deployed(deployed) => {
                binaries.insert(program_id.clone(), deployed.elf.clone());
                programs.push(ProgramDependency {
                    program_id,
                    source: ProgramSource::HistoricalMainnet,
                    loader: Some(deployed.loader),
                    deployed_slot: deployed.deploy_slot,
                    binary_sha256: Some(deployed.sha256.clone()),
                    binary_len: Some(deployed.elf.len() as u64),
                    observed_slot: Some(slot),
                    discovered_by,
                    note: None,
                });
            }
        }
    }
    Ok((DependencyManifest { programs }, binaries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AccountMetaSpec, InstructionSpec};

    fn instruction(program: &str) -> InstructionSpec {
        InstructionSpec {
            program: program.into(),
            accounts: vec![AccountMetaSpec {
                address: "11111111111111111111111111111111".into(),
                is_signer: false,
                is_writable: false,
            }],
            data: vec![],
        }
    }

    fn transaction(outer: &[&str], inner: &[&str], logs: &[&str]) -> HistoricalTransaction {
        HistoricalTransaction {
            signature: "sig".into(),
            slot: 10,
            block_time: Some(0),
            version: "legacy".into(),
            recent_blockhash: "hash".into(),
            payer: "payer".into(),
            account_keys: vec![],
            loaded_address_count: 0,
            instructions: outer.iter().copied().map(instruction).collect(),
            inner_instructions: inner.iter().copied().map(instruction).collect(),
            inner_instruction_frames: inner
                .iter()
                .enumerate()
                .map(|(position, program)| {
                    crate::ingest::transactions::CpiFrame::new(
                        0,
                        2,
                        (*program).to_string(),
                        1,
                        &[u8::try_from(position).unwrap_or(0)],
                    )
                })
                .collect(),
            success: true,
            error: None,
            fee: 5000,
            compute_units: None,
            pre_balances: None,
            post_balances: None,
            pre_token_balances: None,
            post_token_balances: None,
            native_value_lamports: None,
            logs: logs.iter().map(|line| line.to_string()).collect(),
        }
    }

    const SYSTEM: &str = "11111111111111111111111111111111";
    const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
    const STAKE_POOL: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";

    /// The point of the phase: a program reached only through CPI is still a
    /// program the replay has to load.
    #[test]
    fn a_cpi_only_program_is_discovered() {
        let found = discover(
            &transaction(&[STAKE_POOL], &[SYSTEM, TOKEN], &[]),
            None,
            STAKE_POOL,
        );
        let ids: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&TOKEN), "{ids:?}");
        assert!(ids.contains(&SYSTEM), "{ids:?}");
    }

    #[test]
    fn discovery_routes_are_recorded_and_merged() {
        let found = discover(
            &transaction(
                &[STAKE_POOL],
                &[SYSTEM],
                &[&format!("Program {SYSTEM} invoke [2]")],
            ),
            None,
            STAKE_POOL,
        );
        let system = found
            .iter()
            .find(|(id, _)| id == SYSTEM)
            .expect("system discovered");
        assert_eq!(
            system.1,
            vec![
                DependencyDiscovery::InnerInstruction,
                DependencyDiscovery::ExecutionLog
            ]
        );
        let pool = found
            .iter()
            .find(|(id, _)| id == STAKE_POOL)
            .expect("program under test discovered");
        assert_eq!(
            pool.1,
            vec![
                DependencyDiscovery::ProgramUnderTest,
                DependencyDiscovery::TopLevelInstruction
            ]
        );
    }

    /// Log text past the first frame is program-authored, so the scraper must
    /// not turn arbitrary words into dependencies.
    #[test]
    fn log_scraping_ignores_program_authored_text() {
        let found = discover(
            &transaction(
                &[STAKE_POOL],
                &[],
                &[
                    "Program log: Instruction: DepositSol",
                    "Program log: Program totally-not-an-address invoke [2]",
                    "Program SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy consumed 11464 of 392513 compute units",
                ],
            ),
            None,
            STAKE_POOL,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, STAKE_POOL);
    }

    #[test]
    fn the_manifest_orders_dependencies_canonically() {
        let found = discover(&transaction(&[TOKEN, SYSTEM], &[], &[]), None, STAKE_POOL);
        let ids: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn an_artifact_name_is_addressed_by_program_id() {
        let dependency = ProgramDependency {
            program_id: TOKEN.into(),
            source: ProgramSource::HistoricalMainnet,
            loader: None,
            deployed_slot: None,
            binary_sha256: None,
            binary_len: None,
            observed_slot: None,
            discovered_by: vec![],
            note: None,
        };
        assert_eq!(dependency.artifact_file_name(), format!("{TOKEN}.so"));
        assert!(dependency.requires_artifact());
    }
}
