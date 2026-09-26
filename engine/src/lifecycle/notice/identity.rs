//! Identity fields used by the generic notice workflow. No issuer investigation runs here.
use crate::{lifecycle::resolution::ArtifactRef, standard_programs::token::MintConfig};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawEvidence {
    pub artifact: ArtifactRef,
    pub pointer: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MintVerification {
    pub mint: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub slot: u64,
    pub runtime_owner: String,
    pub raw_data_sha256: String,
    pub configuration: MintConfig,
    pub evidence: RawEvidence,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub creation_slot_established: Option<u64>,
}

/// Projects only independently rechecked mint observations from an archived report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityReport {
    pub source_verification: MintVerification,
    pub successor_verification: MintVerification,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MechanismType {
    BurnMint,
    TransferClaim,
    Swap,
    Redemption,
    BackendMediated,
    Unknown,
}
