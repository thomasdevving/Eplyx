//! Experimental execution envelopes. Structural dependency admission is never
//! semantic coverage or runtime eligibility. Complete instruction order is kept.
use crate::{message::ProvenV0, types::InstructionSpec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionRole {
    SemanticTarget,
    TargetPrerequisite,
    ExecutionDependency,
    StandardCompanion,
    UnsupportedCompanion,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetObservation {
    pub program_id: String,
    pub outer_index: usize,
    pub action_id: String,
    pub instruction_identity: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeInstruction {
    pub outer_index: usize,
    pub instruction: InstructionSpec,
    pub identity: String,
    pub role: InstructionRole,
    pub semantic_supported: bool,
    pub required_state: Vec<String>,
    pub may_write: Vec<String>,
    pub consumed_by: Vec<usize>,
    pub historical_binary_required: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeBlocker {
    pub code: String,
    pub outer_index: Option<usize>,
    pub detail: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeAnalysis {
    pub signature: String,
    pub execution_slot: u64,
    pub message_proof_id: String,
    pub instructions: Vec<EnvelopeInstruction>,
    pub targets: Vec<TargetObservation>,
    pub blockers: Vec<EnvelopeBlocker>,
    pub structurally_classified: bool,
    pub envelope_admissible: bool,
    pub attribution: String,
}
/// Sealed structural admission. It cannot authorize execution: historically
/// pinned binaries, pre-state and runtime context remain separate requirements.
#[derive(Clone, Debug)]
pub struct AdmittedEnvelope {
    message: ProvenV0,
    analysis: EnvelopeAnalysis,
}
impl AdmittedEnvelope {
    pub fn analysis(&self) -> &EnvelopeAnalysis {
        &self.analysis
    }
    pub fn message(&self) -> &ProvenV0 {
        &self.message
    }
    /// Planning guard only. No reduced transaction may replace the original.
    pub fn check_instruction_sequence(
        &self,
        instructions: &[InstructionSpec],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            instructions == self.message.transaction().instructions,
            "envelope instruction sequence changed; no dependency stripping/reordering"
        );
        Ok(())
    }
    pub fn check_binary_manifest(
        &self,
        manifest: &crate::dependencies::DependencyManifest,
        binaries: &std::collections::BTreeMap<String, Vec<u8>>,
    ) -> anyhow::Result<()> {
        use crate::dependencies::ProgramSource;
        let pre_slot = self.analysis.execution_slot - 1;
        for (program, _) in crate::dependencies::discover(
            self.message.transaction(),
            None,
            &self.analysis.targets[0].program_id,
        ) {
            let entry = manifest
                .get(&program)
                .ok_or_else(|| anyhow::anyhow!("dependency_binary_unavailable: {program}"))?;
            let must_load = self
                .analysis
                .instructions
                .iter()
                .any(|i| i.instruction.program == program && i.historical_binary_required);
            anyhow::ensure!(
                !must_load || entry.source == ProgramSource::HistoricalMainnet,
                "non-native external dependency requires historical binary"
            );
            anyhow::ensure!(
                entry.observed_slot == Some(pre_slot),
                "current dependency binary is not historical pre-slot deployment"
            );
            if entry.source == ProgramSource::HistoricalMainnet {
                let bytes = binaries
                    .get(&program)
                    .ok_or_else(|| anyhow::anyhow!("pinned binary bytes absent"))?;
                anyhow::ensure!(
                    entry.binary_sha256.as_deref()
                        == Some(crate::replay::hash_bytes(bytes).as_str())
                        && entry.binary_len == Some(bytes.len() as u64)
                        && entry.deployed_slot.is_none_or(|s| s <= pre_slot),
                    "historical dependency ELF/hash/deployment differs"
                );
            } else {
                anyhow::ensure!(
                    entry.source == ProgramSource::Builtin,
                    "unsupported dependency source"
                );
            }
        }
        Ok(())
    }
    /// Context guard for acquisition records, not an account-byte proof or an
    /// execution entry point. Actual acquisition must also prove raw snapshots.
    pub fn check_seed_context(
        &self,
        sources: &[crate::replay::AccountAcquisition],
        screen: &crate::screening::SlotScreening,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            screen.slot == self.analysis.execution_slot
                && screen.target_signature == self.analysis.signature,
            "screening identity differs"
        );
        screen.ensure_unambiguous()?;
        for ix in self
            .analysis
            .instructions
            .iter()
            .filter(|ix| ix.role == InstructionRole::ExecutionDependency)
        {
            for address in ix
                .required_state
                .iter()
                .filter(|a| *a != &ix.instruction.program && !a.starts_with("Sysvar"))
            {
                anyhow::ensure!(
                    screen.required_accounts.contains(address),
                    "dependency account was not screened"
                );
                let source = sources
                    .iter()
                    .find(|s| &s.address == address)
                    .ok_or_else(|| anyhow::anyhow!("dependency_state_unavailable"))?;
                anyhow::ensure!(source.source==crate::replay::AccountStateSource::HistoricalArchive && source.context_slot==self.analysis.execution_slot-1,"dependency seed must be historical pre-transaction state, never intermediate/post/current state");
            }
        }
        Ok(())
    }
}
impl EnvelopeAnalysis {
    pub(crate) fn seal(self, message: &ProvenV0) -> Option<AdmittedEnvelope> {
        (self.envelope_admissible
            && self.blockers.is_empty()
            && self.instructions.len() == message.transaction().instructions.len()
            && self.instructions.iter().enumerate().all(|(i, row)| {
                row.outer_index == i && row.instruction == message.transaction().instructions[i]
            }))
        .then(|| AdmittedEnvelope {
            message: message.clone(),
            analysis: self,
        })
    }
}
