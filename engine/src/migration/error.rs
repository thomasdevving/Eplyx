//! Stable CLI classification without matching human-readable error strings.
use anyhow::Result;
#[derive(Debug)]
pub struct Incompatible(pub &'static str);
impl std::fmt::Display for Incompatible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Incompatible {}
pub fn compatible(condition: bool, message: &'static str) -> Result<()> {
    if !condition {
        return Err(Incompatible(message).into());
    }
    Ok(())
}
pub fn exit_code(error: &anyhow::Error) -> u8 {
    if error.downcast_ref::<Incompatible>().is_some() {
        4
    } else {
        2
    }
}
