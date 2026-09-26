//! RPC-shaped token observations projected exclusively through MAIN's shared
//! base and extension decoders. These views describe captured fields, not proof.
use super::{spl_token, token2022, Decoded};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;

pub const LEGACY_PROGRAM: &str = spl_token::PROGRAM_ID;
pub const TOKEN_2022_PROGRAM: &str = token2022::PROGRAM_ID;
pub const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const CLOCK: &str = "SysvarC1ock11111111111111111111111111111111";
pub const UPGRADEABLE_LOADER: &str = "BPFLoaderUpgradeab1e11111111111111111111111";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenExtension {
    pub extension_type: String,
    pub type_id: u16,
    pub config: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MintConfig {
    pub token_program: String,
    pub is_token_2022: bool,
    pub decimals: u8,
    pub raw_supply: String,
    pub decimal_supply: String,
    pub mint_authority: Option<String>,
    pub freeze_authority: Option<String>,
    pub is_initialized: bool,
    pub extensions: Vec<TokenExtension>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenAccountState {
    pub mint: String,
    pub owner: String,
    pub raw_balance: String,
    pub ui_balance: String,
    pub ui_balance_basis: String,
    pub account_state: String,
    pub delegate: Option<String>,
    pub delegated_amount: String,
    pub close_authority: Option<String>,
    pub native_reserve: Option<String>,
    pub is_frozen: bool,
    pub is_initialized: bool,
    pub has_active_delegate: bool,
    pub extensions: Vec<TokenExtension>,
}

pub fn decimal_amount(amount: u64, decimals: u8) -> String {
    let text = amount.to_string();
    if decimals == 0 {
        return text;
    }
    let padded = format!("{:0>width$}", text, width = usize::from(decimals) + 1);
    let split = padded.len() - usize::from(decimals);
    format!("{}.{}", &padded[..split], &padded[split..])
        .trim_end_matches('0')
        .trim_end_matches('.')
        .into()
}

pub fn raw_account_bytes(raw: &Value) -> Result<Vec<u8>> {
    ensure!(
        raw["data"][1] == "base64",
        "expected complete base64 account data"
    );
    let bytes = STANDARD.decode(
        raw["data"][0]
            .as_str()
            .context("missing base64 account data")?,
    )?;
    if let Some(space) = raw.get("space") {
        ensure!(
            space.as_u64() == Some(bytes.len() as u64),
            "truncated account data / space mismatch"
        );
    }
    Ok(bytes)
}
pub fn account_bytes(raw: &Value, program: &str) -> Result<Vec<u8>> {
    ensure!(
        raw["owner"] == program && raw["executable"] == false,
        "unexpected token account runtime owner or executable flag"
    );
    raw_account_bytes(raw)
}
fn parsed<T>(decoded: Decoded<T>) -> Result<T> {
    match decoded {
        Decoded::Decoded(value) => Ok(value),
        Decoded::Malformed(reason) => bail!("malformed token data: {reason}"),
        Decoded::Unsupported(reason) => bail!("unsupported token data: {reason}"),
        Decoded::NotApplicable => bail!("unexpected token layout"),
    }
}
fn camel(name: &str) -> String {
    let mut parts = name.split('_');
    let mut out = parts.next().unwrap_or_default().to_string();
    for part in parts {
        let mut chars = part.chars();
        if let Some(c) = chars.next() {
            out.extend(c.to_uppercase());
            out.extend(chars);
        }
    }
    out
}
fn camel_fields(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(k, v)| (camel(&k), camel_fields(v)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(camel_fields).collect()),
        other => other,
    }
}
fn extension_views(bytes: &[u8], layout: token2022::Layout) -> Result<Vec<TokenExtension>> {
    let typed = match token2022::checked_extensions(bytes, layout) {
        Decoded::Decoded(values) => values,
        Decoded::Malformed(reason) => {
            return Ok(vec![TokenExtension {
                extension_type: "Malformed".into(),
                type_id: 0,
                config: json!({"reason":reason.to_string()}),
            }])
        }
        _ => bail!("unexpected extension layout"),
    };
    let entries = token2022::extensions(bytes);
    let mut result = Vec::new();
    for ((kind, _), extension) in entries.entries.iter().zip(typed) {
        let mut config = camel_fields(serde_json::to_value(extension)?);
        config
            .as_object_mut()
            .context("extension fields")?
            .remove("type");
        let name: String = token2022::extension_name(*kind)
            .split('-')
            .map(|part| {
                let mut c = part.chars();
                c.next()
                    .map(|first| first.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            })
            .collect();
        if *kind == 1 {
            let values = config.as_object_mut().unwrap();
            for (old, new) in [
                ("configAuthority", "transferFeeConfigAuthority"),
                ("withdrawAuthority", "withdrawWithheldAuthority"),
                ("older", "olderTransferFee"),
                ("newer", "newerTransferFee"),
            ] {
                if let Some(value) = values.remove(old) {
                    values.insert(new.into(), value);
                }
            }
            for field in ["olderTransferFee", "newerTransferFee"] {
                if let Some(values) = config[field].as_object_mut() {
                    if let Some(value) = values.remove("basisPoints") {
                        values.insert("transferFeeBasisPoints".into(), value);
                    }
                }
            }
        }
        if *kind == 6 {
            let raw = config["state"].as_u64().context("default account state")?;
            config = json!({"state":match raw {0=>"Uninitialized",1=>"Initialized",2=>"Frozen",_=>unreachable!()},"rawState":raw});
        }
        result.push(TokenExtension {
            extension_type: name,
            type_id: *kind,
            config,
        });
    }
    result.sort_by_key(|e| e.type_id);
    Ok(result)
}

pub fn decode_mint(raw: &Value) -> Result<MintConfig> {
    let program = raw["owner"]
        .as_str()
        .context("missing mint runtime owner")?;
    let bytes = account_bytes(raw, program)?;
    let (base, extensions) = match program {
        LEGACY_PROGRAM => (parsed(spl_token::decode_mint(&bytes))?, vec![]),
        TOKEN_2022_PROGRAM => (
            parsed(token2022::decode_mint(&bytes))?.base,
            extension_views(&bytes, token2022::Layout::Mint)?,
        ),
        _ => bail!("unsupported token program"),
    };
    ensure!(base.is_initialized, "uninitialized token mint");
    Ok(MintConfig {
        token_program: program.into(),
        is_token_2022: program == TOKEN_2022_PROGRAM,
        decimals: base.decimals,
        raw_supply: base.supply.to_string(),
        decimal_supply: decimal_amount(base.supply, base.decimals),
        mint_authority: base.mint_authority,
        freeze_authority: base.freeze_authority,
        is_initialized: base.is_initialized,
        extensions,
    })
}
pub fn decode_token_account(
    raw: &Value,
    program: &str,
    mint: &str,
    decimals: u8,
) -> Result<TokenAccountState> {
    let bytes = account_bytes(raw, program)?;
    let (base, extensions) = match program {
        LEGACY_PROGRAM => (parsed(spl_token::decode_account(&bytes))?, vec![]),
        TOKEN_2022_PROGRAM => (
            parsed(token2022::decode_account(&bytes))?.base,
            extension_views(&bytes, token2022::Layout::Account)?,
        ),
        _ => bail!("unsupported token program"),
    };
    ensure!(
        base.mint == mint,
        "token account belongs to a different mint"
    );
    Ok(TokenAccountState {
        mint: base.mint,
        owner: base.owner,
        raw_balance: base.amount.to_string(),
        ui_balance: decimal_amount(base.amount, decimals),
        ui_balance_basis: "raw_balance / 10^decimals; no UI transform".into(),
        account_state: match base.state {
            spl_token::AccountState::Uninitialized => "Uninitialized",
            spl_token::AccountState::Initialized => "Initialized",
            spl_token::AccountState::Frozen => "Frozen",
        }
        .into(),
        has_active_delegate: base.delegate.is_some() && base.delegated_amount > 0,
        delegate: base.delegate,
        delegated_amount: base.delegated_amount.to_string(),
        close_authority: base.close_authority,
        native_reserve: base.native_reserve.map(|v| v.to_string()),
        is_frozen: base.state == spl_token::AccountState::Frozen,
        is_initialized: base.state != spl_token::AccountState::Uninitialized,
        extensions,
    })
}

pub fn transfer_fee(mint: &[u8], epoch: u64, amount: u64) -> Result<u64> {
    parsed(token2022::decode_mint(mint))?;
    for extension in parsed(token2022::checked_extensions(mint, token2022::Layout::Mint))? {
        if let Some(fee) = extension.transfer_fee(epoch) {
            return fee.calculate_fee(amount).context("transfer fee overflow");
        }
    }
    Ok(0)
}

/// Create proposed bytes using the official token packer, never as captured state.
pub fn proposed_token_account(
    mint_bytes: &[u8],
    mint: &Address,
    owner: &Address,
    amount: u64,
) -> Result<Vec<u8>> {
    use spl_token_2022_interface::{
        extension::{
            account_len, BaseStateWithExtensions, BaseStateWithExtensionsMut, StateWithExtensions,
            StateWithExtensionsMut,
        },
        state::{Account, AccountState, Mint},
    };
    let length = account_len::try_calculate_account_len_from_mint_data(mint_bytes, &[])?;
    let mut bytes = vec![0; length];
    let mint_state = StateWithExtensions::<Mint>::unpack(mint_bytes)?;
    let mut state = StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut bytes)?;
    account_len::try_for_each_required_init_account_extension(
        mint_state.get_tlv_data(),
        |extension| state.init_account_extension_from_type(extension),
    )?;
    state.base = Account {
        mint: *mint,
        owner: *owner,
        amount,
        state: AccountState::Initialized,
        ..Account::default()
    };
    state.pack_base();
    state.init_account_type()?;
    Ok(bytes)
}

/// The token program owns the layout; never infer it from a leading byte.
pub fn account_base(program: &str, bytes: &[u8]) -> Result<spl_token::TokenAccount> {
    match program {
        LEGACY_PROGRAM => parsed(spl_token::decode_account(bytes)),
        TOKEN_2022_PROGRAM => Ok(parsed(token2022::decode_account(bytes))?.base),
        _ => bail!("unsupported token program"),
    }
}
pub fn mint_base(program: &str, bytes: &[u8]) -> Result<spl_token::Mint> {
    match program {
        LEGACY_PROGRAM => parsed(spl_token::decode_mint(bytes)),
        TOKEN_2022_PROGRAM => Ok(parsed(token2022::decode_mint(bytes))?.base),
        _ => bail!("unsupported token program"),
    }
}
/// Exact public and withheld balances. Unknown or malformed extensions fail closed.
pub fn account_amounts(program: &str, bytes: &[u8]) -> Result<(u64, u64)> {
    let base = account_base(program, bytes)?;
    let mut withheld = 0;
    if program == TOKEN_2022_PROGRAM {
        for extension in parsed(token2022::checked_extensions(
            bytes,
            token2022::Layout::Account,
        ))? {
            match extension {
                token2022::Extension::TransferFeeAmount { withheld_amount } => {
                    withheld = withheld_amount
                }
                token2022::Extension::Unrecognized { .. } => {
                    bail!("unknown token account extension")
                }
                _ => {}
            }
        }
    }
    Ok((base.amount, withheld))
}
/// Explicitly derived/proposed state, packed through the official interface.
pub fn replace_amount(program: &str, bytes: &mut [u8], amount: u64) -> Result<()> {
    account_base(program, bytes)?;
    use spl_token_2022_interface::{extension::StateWithExtensionsMut, state::Account};
    let mut state = StateWithExtensionsMut::<Account>::unpack(bytes)?;
    state.base.amount = amount;
    state.pack_base();
    Ok(())
}
pub fn replace_supply(program: &str, bytes: &mut [u8], supply: u64) -> Result<()> {
    mint_base(program, bytes)?;
    use spl_token_2022_interface::{extension::StateWithExtensionsMut, state::Mint};
    let mut state = StateWithExtensionsMut::<Mint>::unpack(bytes)?;
    state.base.supply = supply;
    state.pack_base();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program_pack::Pack;
    use spl_token_2022_interface::state::{Account, AccountState, Mint, Multisig};

    fn rpc(bytes: &[u8], owner: &str) -> Value {
        json!({"owner":owner,"executable":false,"data":[STANDARD.encode(bytes),"base64"],"space":bytes.len()})
    }

    #[test]
    fn observations_keep_exact_large_amounts_and_refuse_foreign_layouts() {
        let mint = Address::new_from_array([7; 32]);
        let owner = Address::new_from_array([8; 32]);
        let mut bytes = vec![0; Account::LEN];
        Account::pack(
            Account {
                mint,
                owner,
                amount: u64::MAX,
                state: AccountState::Initialized,
                ..Account::default()
            },
            &mut bytes,
        )
        .unwrap();
        let raw = rpc(&bytes, LEGACY_PROGRAM);
        let decoded = decode_token_account(&raw, LEGACY_PROGRAM, &mint.to_string(), 9).unwrap();
        assert_eq!(decoded.raw_balance, u64::MAX.to_string());
        assert_eq!(decoded.ui_balance, "18446744073.709551615");
        assert_eq!(
            account_amounts(LEGACY_PROGRAM, &bytes).unwrap(),
            (u64::MAX, 0)
        );
        assert!(decode_token_account(&raw, TOKEN_2022_PROGRAM, &mint.to_string(), 9).is_err());
        assert!(decode_token_account(&raw, LEGACY_PROGRAM, &owner.to_string(), 9).is_err());
        let mut truncated = raw.clone();
        truncated["space"] = json!(bytes.len() + 1);
        assert!(decode_token_account(&truncated, LEGACY_PROGRAM, &mint.to_string(), 9).is_err());
    }

    #[test]
    fn unknown_and_malformed_extensions_remain_visible_and_amounts_fail_closed() {
        let mut bytes = vec![0; Account::LEN];
        Account::pack(
            Account {
                state: AccountState::Initialized,
                ..Account::default()
            },
            &mut bytes,
        )
        .unwrap();
        bytes.push(2); // official AccountType::Account
        bytes.extend_from_slice(&60000u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.push(9);
        let view = decode_token_account(
            &rpc(&bytes, TOKEN_2022_PROGRAM),
            TOKEN_2022_PROGRAM,
            &Address::default().to_string(),
            0,
        )
        .unwrap();
        assert_eq!(view.extensions[0].extension_type, "Unrecognized");
        assert_eq!(view.extensions[0].type_id, 60000);
        assert!(account_amounts(TOKEN_2022_PROGRAM, &bytes).is_err());
        bytes[Account::LEN + 3..Account::LEN + 5].copy_from_slice(&2u16.to_le_bytes());
        let view = decode_token_account(
            &rpc(&bytes, TOKEN_2022_PROGRAM),
            TOKEN_2022_PROGRAM,
            &Address::default().to_string(),
            0,
        )
        .unwrap();
        assert_eq!(view.extensions[0].extension_type, "Malformed");
        assert!(account_amounts(TOKEN_2022_PROGRAM, &bytes).is_err());
    }

    #[test]
    fn shared_multisig_and_proposed_account_packers_use_official_layouts() {
        let a = Address::new_from_array([1; 32]);
        let b = Address::new_from_array([2; 32]);
        let mut signers = [Address::default(); 11];
        signers[0] = a;
        signers[1] = b;
        let mut bytes = vec![0; Multisig::LEN];
        Multisig::pack(
            Multisig {
                m: 2,
                n: 2,
                is_initialized: true,
                signers,
            },
            &mut bytes,
        )
        .unwrap();
        assert!(matches!(
            spl_token::decode_multisig(&bytes),
            Decoded::Decoded(_)
        ));
        bytes[0] = 3;
        assert!(matches!(
            spl_token::decode_multisig(&bytes),
            Decoded::Malformed(_)
        ));
        let mut mint = vec![0; Mint::LEN];
        Mint::pack(
            Mint {
                is_initialized: true,
                ..Mint::default()
            },
            &mut mint,
        )
        .unwrap();
        let account = proposed_token_account(&mint, &a, &b, 9007199254740993).unwrap();
        assert_eq!(
            account_base(LEGACY_PROGRAM, &account).unwrap().amount,
            9007199254740993
        );
    }
}
