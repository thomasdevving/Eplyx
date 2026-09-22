//! Provenance of an adapter's *meaning*, separate from execution fidelity.
//! A checked historical ELF proves what ran; it does not prove that a source
//! tree compiled to that ELF. These types deliberately keep those claims apart.

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

use crate::{replay::hash_bytes, universal::execution::ExecutionEvidence};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBlob {
    pub path: String,
    pub git_blob_sha1: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInterface {
    pub repository: String,
    pub commit: String,
    pub source_blobs: Vec<SourceBlob>,
}

impl RepositoryInterface {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.repository.starts_with("https://") && !self.source_blobs.is_empty(),
            "semantic repository source is incomplete"
        );
        ensure!(
            is_lower_hex(&self.commit, 40),
            "semantic source commit is not a Git SHA-1"
        );
        ensure!(
            self.source_blobs
                .iter()
                .all(|blob| !blob.path.is_empty() && is_lower_hex(&blob.git_blob_sha1, 40)),
            "semantic source blob identity is incomplete"
        );
        Ok(())
    }
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// The exact relationships checked against one retained historical execution.
/// The adapter must recompute these checks from resolved evidence at build and
/// open time; serializing a check never makes it authoritative on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorroboratedFact {
    InstructionShape,
    SignerRole,
    TokenProgramOwnership,
    MintIdentity,
    TokenAccountAuthority,
    PoolMintVaultRelationship,
    ObservedFlowDirection,
    BaselineExecutionSuccess,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum SemanticBinding {
    /// Reserved for a reproducible build witness checked against the historical
    /// ELF. No such verifier exists yet, so validation rejects this claim.
    ExactVerifiedBuild {
        source: RepositoryInterface,
        build_evidence_sha256: String,
    },
    StandardProgramInterface {
        program_id: String,
    },
    ExecutionCorroboratedExternalInterface {
        source: RepositoryInterface,
        facts: Vec<CorroboratedFact>,
        historical_elf_sha256: String,
        execution_evidence_sha256: String,
    },
    RepositorySourceClaim {
        source: RepositoryInterface,
    },
    ManualOrUnknown,
}

impl SemanticBinding {
    pub fn level(&self) -> &'static str {
        match self {
            Self::ExactVerifiedBuild { .. } => "exact_verified_build",
            Self::StandardProgramInterface { .. } => "standard_program_interface",
            Self::ExecutionCorroboratedExternalInterface { .. } => {
                "execution_corroborated_external_interface"
            }
            Self::RepositorySourceClaim { .. } => "repository_source_claim",
            Self::ManualOrUnknown => "manual_or_unknown",
        }
    }

    pub fn exact_source_to_elf_verified(&self) -> bool {
        matches!(self, Self::ExactVerifiedBuild { .. })
    }

    pub fn validate(
        &self,
        program_id: &str,
        baseline_elf: &[u8],
        baseline: &ExecutionEvidence,
    ) -> Result<()> {
        match self {
            Self::ExactVerifiedBuild { .. } => anyhow::bail!(
                "exact semantic build provenance requires a reproducible source-to-ELF verifier"
            ),
            Self::StandardProgramInterface {
                program_id: claimed,
            } => {
                ensure!(
                    claimed == program_id
                        && [
                            crate::standard_programs::spl_token::PROGRAM_ID,
                            crate::standard_programs::token2022::PROGRAM_ID,
                            crate::standard_programs::system::PROGRAM_ID
                        ]
                        .contains(&claimed.as_str()),
                    "standard-program semantic binding names an unsupported program"
                );
            }
            Self::ExecutionCorroboratedExternalInterface {
                source,
                facts,
                historical_elf_sha256,
                execution_evidence_sha256,
            } => {
                source.validate()?;
                ensure!(
                    !facts.is_empty() && facts.windows(2).all(|pair| pair[0] < pair[1]),
                    "semantic corroboration facts must be unique and sorted"
                );
                ensure!(
                    historical_elf_sha256 == &hash_bytes(baseline_elf)
                        && execution_evidence_sha256
                            == &crate::universal::pipeline::evidence_hash(baseline)?,
                    "semantic corroboration is bound to another ELF or execution"
                );
            }
            Self::RepositorySourceClaim { source } => source.validate()?,
            Self::ManualOrUnknown => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationSemanticBinding {
    pub observation_id: String,
    pub binding: SemanticBinding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticBindingReport {
    pub observations: Vec<ObservationSemanticBinding>,
    /// Explicit in the report so a hard High/Critical finding cannot be read
    /// as source-to-bytecode verified merely because replay was matched.
    pub exact_source_to_elf_verified: bool,
}

impl SemanticBindingReport {
    pub fn new(observations: Vec<ObservationSemanticBinding>) -> Self {
        let exact_source_to_elf_verified = !observations.is_empty()
            && observations
                .iter()
                .all(|item| item.binding.exact_source_to_elf_verified());
        Self {
            observations,
            exact_source_to_elf_verified,
        }
    }
}
