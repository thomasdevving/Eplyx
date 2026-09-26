//! Pinned Meteora DLMM LbPair identity and custody decoding; no execution claim.
use crate::standard_programs::token::{self as decode, LEGACY_PROGRAM, TOKEN_2022_PROGRAM};
use anyhow::{ensure, Context, Result};
use borsh::BorshDeserialize;
use serde_json::{json, Value};
use solana_address::Address;
pub const PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
pub const SDK_REVISION: &str = "576919e3e4368e542c402f000b4264724f7f23ec";
pub const ADAPTER_VERSION: &str = "1";
pub const DISCRIMINATOR: [u8; 8] = [33, 11, 49, 98, 181, 101, 177, 13];
pub const ACCOUNT_LEN: usize = 904;

// The IDL zero-copy layout includes explicit padding. Opaque regions are fields
// irrelevant to custody discovery (fees/rewards/bins); complete bytes are retained.
#[derive(BorshDeserialize)]
struct LbPairLayout {
    _parameters: [u8; 32],
    _v_parameters: [u8; 32],
    bump_seed: [u8; 1],
    bin_step_seed: [u8; 2],
    pair_type: u8,
    active_id: i32,
    bin_step: u16,
    status: u8,
    require_base_factor_seed: u8,
    _base_factor_seed: [u8; 2],
    activation_type: u8,
    creator_pool_on_off_control: u8,
    token_x_mint: [u8; 32],
    token_y_mint: [u8; 32],
    reserve_x: [u8; 32],
    reserve_y: [u8; 32],
    _protocol_fee: [u8; 16],
    _padding_1: [u8; 32],
    _reward_infos: [u8; 288],
    _oracle: [u8; 32],
    _bin_array_bitmap: [u8; 128],
    _last_updated_at: i64,
    _padding_2: [u8; 32],
    _pre_activation_swap_address: [u8; 32],
    base_key: [u8; 32],
    _activation_point: u64,
    _pre_activation_duration: u64,
    _padding_3: [u8; 8],
    _padding_4: u64,
    _creator: [u8; 32],
    token_mint_x_program_flag: u8,
    token_mint_y_program_flag: u8,
    version: u8,
    _reserved: [u8; 21],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedPool {
    pub pool: Address,
    pub mints: [Address; 2],
    pub vaults: [Address; 2],
    pub token_programs: [String; 2],
    pub decoded_fields: Value,
}

pub fn decode_pool(pool: &str, raw: &Value, target_mint: &str) -> Result<DecodedPool> {
    let pool: Address = pool.parse().context("invalid DLMM pool address")?;
    let bytes = decode::account_bytes(raw, PROGRAM_ID)
        .context("candidate is not a non-executable DLMM-owned pool")?;
    ensure!(
        bytes.len() == ACCOUNT_LEN,
        "unsupported DLMM account length (expected 904)"
    );
    ensure!(
        bytes[..8] == DISCRIMINATOR,
        "candidate lacks DLMM LbPair discriminator"
    );
    let layout =
        LbPairLayout::try_from_slice(&bytes[8..]).context("malformed DLMM LbPair layout")?;
    ensure!(
        layout.version == 1 && layout.pair_type == 3,
        "unsupported DLMM state version / pair type (only version 1 PermissionlessV2)"
    );
    ensure!(
        layout.status <= 1
            && layout.activation_type <= 1
            && layout.require_base_factor_seed <= 1
            && layout.creator_pool_on_off_control <= 1,
        "invalid DLMM enum/boolean field"
    );
    ensure!(
        layout.bin_step > 0 && layout.bin_step_seed == layout.bin_step.to_le_bytes(),
        "invalid DLMM bin-step seed"
    );
    let mints = [
        Address::new_from_array(layout.token_x_mint),
        Address::new_from_array(layout.token_y_mint),
    ];
    let vaults = [
        Address::new_from_array(layout.reserve_x),
        Address::new_from_array(layout.reserve_y),
    ];
    ensure!(
        mints[0] != mints[1] && vaults[0] != vaults[1],
        "duplicate DLMM assets/vaults"
    );
    ensure!(
        mints.iter().any(|m| m.to_string() == target_mint),
        "candidate pool does not contain lifecycle mint"
    );
    let program: Address = PROGRAM_ID.parse()?;
    let (min, max) = if layout.token_x_mint < layout.token_y_mint {
        (&layout.token_x_mint, &layout.token_y_mint)
    } else {
        (&layout.token_y_mint, &layout.token_x_mint)
    };
    // Official deriveLbPairWithPresetParamWithIndexKey: base_key stores the
    // preset parameter key for this pool type. Verify both address and bump.
    let (derived_pool, bump) =
        Address::find_program_address(&[&layout.base_key, min, max], &program);
    ensure!(
        derived_pool == pool && layout.bump_seed[0] == bump,
        "DLMM pool PDA / bump mismatch"
    );
    for i in 0..2 {
        let (reserve, _) =
            Address::find_program_address(&[pool.as_ref(), mints[i].as_ref()], &program);
        ensure!(
            reserve == vaults[i],
            "DLMM reserve is not canonical pool/mint PDA"
        );
    }
    let token_programs = [
        layout.token_mint_x_program_flag,
        layout.token_mint_y_program_flag,
    ]
    .map(|flag| match flag {
        0 => Ok(LEGACY_PROGRAM.to_string()),
        1 => Ok(TOKEN_2022_PROGRAM.to_string()),
        _ => Err(anyhow::anyhow!("unsupported DLMM token-program flag")),
    });
    let [x, y] = token_programs;
    Ok(DecodedPool {
        pool,
        mints,
        vaults,
        token_programs: [x?, y?],
        decoded_fields: json!({
        "account_type":"LbPair", "state_version":layout.version,"pair_type":"PermissionlessV2",
        "pool_bump":bump,"base_key":Address::new_from_array(layout.base_key).to_string(),
        "bin_step":layout.bin_step,"active_bin_id":layout.active_id,
        "status":if layout.status==0 {"Enabled"} else {"Disabled"},
        "token_program_flags":[layout.token_mint_x_program_flag,layout.token_mint_y_program_flag]}),
    })
}
