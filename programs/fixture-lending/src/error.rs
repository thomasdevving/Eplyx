//! Error plumbing.
//!
//! The error *codes* live in `fixture-lending-interface` so that the host-side
//! engine can decode them, and so V1 and V2 cannot drift. The orphan rule
//! prevents implementing `From<LendingError> for ProgramError` here, so the
//! conversion is an explicit helper instead.

pub use fixture_lending_interface::LendingError;
use solana_program::program_error::ProgramError;

pub fn to_program_error(error: LendingError) -> ProgramError {
    ProgramError::Custom(error as u32)
}

/// Convenience for `Result<T, LendingError>` in a `ProgramResult` context.
pub trait IntoProgramResult<T> {
    fn or_program_err(self) -> Result<T, ProgramError>;
}

impl<T> IntoProgramResult<T> for Result<T, LendingError> {
    fn or_program_err(self) -> Result<T, ProgramError> {
        self.map_err(to_program_error)
    }
}
