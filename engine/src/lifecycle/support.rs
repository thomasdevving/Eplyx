//! Lifecycle support policy over MAIN's tolerant standard-token decoder.
//! Unknown or malformed extensions remain visible to generic inspection but do
//! not enter a lifecycle observation whose semantics claim to be understood.
pub use crate::standard_programs::token::{
    account_bytes, decimal_amount, raw_account_bytes, MintConfig, TokenAccountState,
    TokenExtension, LEGACY_PROGRAM, TOKEN_2022_PROGRAM,
};
use anyhow::{ensure, Result};
use serde_json::Value;
fn known(extensions: &[TokenExtension]) -> Result<()> {
    ensure!(
        extensions
            .iter()
            .all(|e| !matches!(e.extension_type.as_str(), "Malformed" | "Unrecognized")),
        "Unverifiable lifecycle token semantics: unsupported or malformed extension"
    );
    Ok(())
}
pub fn decode_mint(raw: &Value) -> Result<MintConfig> {
    let mint = crate::standard_programs::token::decode_mint(raw)?;
    known(&mint.extensions)?;
    Ok(mint)
}
pub fn decode_token_account(
    raw: &Value,
    program: &str,
    mint: &str,
    decimals: u8,
) -> Result<TokenAccountState> {
    let account =
        crate::standard_programs::token::decode_token_account(raw, program, mint, decimals)?;
    known(&account.extensions)?;
    Ok(account)
}
