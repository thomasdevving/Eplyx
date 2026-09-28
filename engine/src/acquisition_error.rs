//! Typed context on existing acquisition failures. These tags do not change admission.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionError {
    TransactionUnavailable,
    AdapterRejected,
    TargetExecutableUnavailable,
    DependencyResolutionFailed,
    HistoricalAccountUnavailable,
    ArchiveSlotMismatch,
    CreationClosureUnsupported,
    SameSlotConflict,
    BlockHistoryUnavailable,
    BoundaryProofFailed,
    UnsupportedRuntime,
    InvalidReplayEvidence,
}
impl std::fmt::Display for AcquisitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ArchiveSlotMismatch => "account archive did not honor exact requested slot",
            Self::SameSlotConflict => "same-slot conflict prevents exact historical boundary proof",
            Self::CreationClosureUnsupported => {
                "account creation and closure are outside the supported contract"
            }
            Self::TransactionUnavailable => "historical transaction unavailable",
            Self::AdapterRejected => "adapter rejected the historical transaction",
            Self::TargetExecutableUnavailable => "historical target executable unavailable",
            Self::DependencyResolutionFailed => "historical dependency resolution failed",
            Self::HistoricalAccountUnavailable => "historical account unavailable",
            Self::BlockHistoryUnavailable => "same-slot block history unavailable",
            Self::BoundaryProofFailed => "historical boundary proof failed",
            Self::UnsupportedRuntime => "unsupported schema-1 runtime semantics",
            Self::InvalidReplayEvidence => "invalid replay evidence",
        })
    }
}
impl std::error::Error for AcquisitionError {}

impl AcquisitionError {
    /// Preserve the existing human diagnostic while adding a machine tag.
    pub(crate) fn attach(self, error: anyhow::Error) -> anyhow::Error {
        let detail = format!("{error:#}");
        error.context(self).context(detail)
    }
}
