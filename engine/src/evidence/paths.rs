//! Independent status vocabulary for explicitly scoped execution paths.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathStatus {
    Proven,
    Failed,
    Indeterminate,
    Unsupported,
    NotTested,
    NotApplicable,
}
