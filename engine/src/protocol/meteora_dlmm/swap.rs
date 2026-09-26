//! Actual DLMM swap2 construction and state/event reconciliation, pinned to its IDL.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use solana_address::Address;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_message::Message;
use std::collections::{BTreeMap, BTreeSet};

use crate::path::*;
use crate::{
    executor::{LoadedProgram, ProbeTransactionExecution},
    lifecycle::{exposure::sha256, LifecycleSnapshot},
    types::NamedAccount,
};
use crate::{
    lifecycle::{
        decode::{self, TokenAccountState, LEGACY_PROGRAM, TOKEN_2022_PROGRAM},
        exposure::meteora_dlmm::{self as dlmm, DecodedPool},
        EntityType,
    },
    types::AccountSnapshot,
};
use serde::Serialize;

pub const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub(crate) use crate::path::captured::{
    captured_account, clock, data_changes, programdata_address, slot,
};
pub use crate::path::captured::{payer, CLOCK, UPGRADEABLE_LOADER};

pub const SWAP2: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];
pub const EVENT_CPI: [u8; 8] = [0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d];
pub const SWAP_EVENT: [u8; 8] = [0x2e, 0x74, 0x52, 0xd7, 0x94, 0x1b, 0x54, 0x4d];
pub const BIN_ARRAY_LEN: usize = 10136;

pub struct DexSwapExitProbe;
pub(crate) struct Route {
    pub pool: DecodedPool,
    pub source: String,
    pub user: Address,
    pub destination: Address,
    pub oracle: Address,
    pub bitmap: Address,
    pub event_authority: Address,
    pub bins: Vec<(i64, Address)>,
}
pub fn program_ids() -> Vec<String> {
    [
        dlmm::PROGRAM_ID,
        TOKEN_2022_PROGRAM,
        LEGACY_PROGRAM,
        ATA_PROGRAM,
        MEMO_PROGRAM,
    ]
    .map(str::to_string)
    .to_vec()
}

fn u64_at(b: &[u8], n: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        b.get(n..n + 8).context("truncated u64")?.try_into()?,
    ))
}
fn key_at(b: &[u8], n: usize) -> Result<Address> {
    Ok(Address::new_from_array(
        b.get(n..n + 32)
            .context("truncated public key")?
            .try_into()?,
    ))
}

fn config(n: u64) -> Value {
    serde_json::json!({"encoding":"base64","commitment":"finalized","minContextSlot":n})
}
pub(crate) fn route(
    snapshot: &LifecycleSnapshot,
    spec: &ExecutionProbeSpec,
    raw: &Value,
) -> Result<Route> {
    let entity = snapshot
        .entities
        .iter()
        .find(|e| e.id == spec.target_entity)
        .context("missing source entity")?;
    ensure!(
        entity.entity_type == EntityType::WalletCompatible,
        "source must be wallet-compatible; a liquidity vault cannot act as a user wallet"
    );
    let graph = snapshot
        .exposures
        .as_ref()
        .context("verified exposure graph required")?;
    let verified = graph
        .protocol_exposures
        .iter()
        .find(|p| p.pool_address == spec.pool)
        .context("pool not in verified exposure graph")?;
    ensure!(
        verified.program_id == dlmm::PROGRAM_ID,
        "wrong pool program"
    );
    ensure!(
        spec.input_mint == snapshot.asset.mint,
        "snapshot mint mismatch"
    );
    let route = route_current(
        &SwapParameters::from(spec),
        &entity.token_account,
        &entity.state.owner,
        raw,
    )?;
    for i in 0..2 {
        ensure!(
            verified
                .assets
                .iter()
                .any(|a| a.mint == route.pool.mints[i].to_string()
                    && a.vault == route.pool.vaults[i].to_string()),
            "current reserves differ from verified venue"
        );
    }
    Ok(route)
}

pub(crate) fn route_current(
    spec: &SwapParameters,
    source: &str,
    owner: &str,
    raw: &Value,
) -> Result<Route> {
    let pool = dlmm::decode_pool(&spec.pool, raw, &spec.input_mint)?;
    ensure!(
        pool.mints[0].to_string() == spec.input_mint
            && pool.mints[1].to_string() == spec.output_mint,
        "pool/mint relationship does not match X-to-Y exit probe"
    );
    ensure!(
        pool.token_programs == [TOKEN_2022_PROGRAM.to_string(), LEGACY_PROGRAM.to_string()],
        "supported slice requires Token-2022 X and legacy Y"
    );
    ensure!(
        pool.decoded_fields["status"] == "Enabled",
        "DLMM pool is disabled"
    );
    let bytes = decode::account_bytes(raw, dlmm::PROGRAM_ID)?;
    let active = i32::from_le_bytes(bytes[76..80].try_into()?);
    let index = i64::from(active.div_euclid(70));
    ensure!(
        (-512..512).contains(&index),
        "active bin outside supported internal bitmap; extended routing is indeterminate"
    );
    let program: Address = dlmm::PROGRAM_ID.parse()?;
    let mut indices = Vec::new();
    for bit in (0..1024usize).rev() {
        let candidate = bit as i64 - 512;
        if candidate <= index && (u64_at(&bytes, 584 + bit / 64 * 8)? >> (bit % 64)) & 1 == 1 {
            indices.push(candidate);
            if indices.len() == 4 {
                break;
            }
        }
    }
    ensure!(
        !indices.is_empty(),
        "no initialized X-to-Y bin array in the supported bounded route"
    );
    let bins = indices
        .into_iter()
        .map(|i| {
            (
                i,
                Address::find_program_address(
                    &[b"bin_array", pool.pool.as_ref(), &i.to_le_bytes()],
                    &program,
                )
                .0,
            )
        })
        .collect();
    let oracle = Address::find_program_address(&[b"oracle", pool.pool.as_ref()], &program).0;
    ensure!(
        oracle == key_at(&bytes, 552)?,
        "pool oracle differs from canonical PDA"
    );
    let bitmap = Address::find_program_address(&[b"bitmap", pool.pool.as_ref()], &program).0;
    let event_authority = Address::find_program_address(&[b"__event_authority"], &program).0;
    let user: Address = owner.parse()?;
    let token: Address = LEGACY_PROGRAM.parse()?;
    let destination = Address::find_program_address(
        &[user.as_ref(), token.as_ref(), pool.mints[1].as_ref()],
        &ATA_PROGRAM.parse()?,
    )
    .0;
    Ok(Route {
        pool,
        source: source.into(),
        user,
        destination,
        oracle,
        bitmap,
        event_authority,
        bins,
    })
}
pub(crate) fn account_plan(route: &Route, programdata: &[String]) -> Vec<String> {
    let mut set: BTreeSet<String> = [
        route.pool.pool.to_string(),
        route.source.clone(),
        route.user.to_string(),
        route.destination.to_string(),
        route.oracle.to_string(),
        route.bitmap.to_string(),
        route.event_authority.to_string(),
        CLOCK.into(),
    ]
    .into_iter()
    .collect();
    set.extend(route.pool.vaults.map(|v| v.to_string()));
    set.extend(route.pool.mints.map(|v| v.to_string()));
    set.extend(route.bins.iter().map(|(_, p)| p.to_string()));
    set.extend(program_ids());
    set.extend(programdata.iter().cloned());
    set.into_iter().collect()
}

fn withheld(s: &TokenAccountState) -> Result<u64> {
    match s
        .extensions
        .iter()
        .find(|e| e.extension_type == "TransferFeeAmount")
    {
        None => Ok(0),
        Some(e) => match &e.config["withheldAmount"] {
            Value::String(s) => Ok(s.parse()?),
            v => v.as_u64().context("missing withheld fee amount"),
        },
    }
}
pub use crate::standard_programs::token::transfer_fee;

impl ExecutionProbe for DexSwapExitProbe {
    fn build_execution(
        &self,
        snapshot: &LifecycleSnapshot,
        spec: &ExecutionProbeSpec,
        fixture: &CapturedExecutionFixture,
    ) -> Result<ProbeExecutionPlan> {
        let min = snapshot
            .exposures
            .as_ref()
            .context("missing verified venue")?
            .protocol_exposures
            .iter()
            .map(|p| p.discovered_at_slot)
            .max()
            .context("missing verified slot")?;
        build_with_route(
            &snapshot.source.genesis_hash,
            min,
            &SwapParameters::from(spec),
            fixture,
            |raw| route(snapshot, spec, raw),
        )
    }

    fn classify_result(
        &self,
        spec: &ExecutionProbeSpec,
        plan: &ProbeExecutionPlan,
        execution: &ProbeTransactionExecution,
    ) -> Result<ExecutionDeltas> {
        reconcile_current(&SwapParameters::from(spec), plan, execution)
    }
}

/// Economic inputs shared by historical and current callers; no lifecycle assertion or saved proof.
#[derive(Clone, Debug, Serialize)]
pub struct SwapParameters {
    pub pool: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount_raw: String,
    pub minimum_output_raw: String,
    pub amount_reason: String,
}
impl From<&ExecutionProbeSpec> for SwapParameters {
    fn from(s: &ExecutionProbeSpec) -> Self {
        Self {
            pool: s.pool.clone(),
            input_mint: s.input_mint.clone(),
            output_mint: s.output_mint.clone(),
            input_amount_raw: s.input_amount_raw.clone(),
            minimum_output_raw: s.minimum_output_raw.clone(),
            amount_reason: s.amount_reason.clone(),
        }
    }
}
pub fn build_current(
    genesis_hash: &str,
    minimum_slot: u64,
    spec: &SwapParameters,
    source: &str,
    owner: &str,
    fixture: &CapturedExecutionFixture,
) -> Result<ProbeExecutionPlan> {
    build_with_route(genesis_hash, minimum_slot, spec, fixture, |raw| {
        route_current(spec, source, owner, raw)
    })
}
fn build_with_route(
    genesis_hash: &str,
    min: u64,
    spec: &SwapParameters,
    fixture: &CapturedExecutionFixture,
    route_fn: impl Fn(&Value) -> Result<Route>,
) -> Result<ProbeExecutionPlan> {
    ensure!(
        fixture.schema_version == 1 && fixture.decoder_revision == dlmm::SDK_REVISION,
        "unsupported execution fixture decoder"
    );
    ensure!(
        fixture.evidence.len() == 4,
        "incomplete execution RPC transcript"
    );
    for (i, e) in fixture.evidence.iter().enumerate() {
        ensure!(e.id == i, "noncanonical execution evidence ids");
    }
    let e = &fixture.evidence;
    ensure!(
        e[0].method == "getGenesisHash"
            && e[0].params == serde_json::json!([])
            && e[0].result.as_str() == Some(genesis_hash),
        "execution capture on wrong chain"
    );
    ensure!(
        e[1].method == "getAccountInfo"
            && e[1].params == serde_json::json!([spec.pool, config(min)]),
        "invalid initial pool request"
    );
    let initial_slot = slot(&e[1].result)?;
    ensure!(
        initial_slot >= min,
        "execution pool capture predates verified venue"
    );
    let initial = route_fn(&e[1].result["value"])?;
    ensure!(
        e[2].method == "getMultipleAccounts"
            && e[2].params == serde_json::json!([program_ids(), config(initial_slot)]),
        "invalid program-header request"
    );
    let headers = e[2].result["value"]
        .as_array()
        .context("missing program headers")?;
    ensure!(
        headers.len() == program_ids().len(),
        "incomplete program headers"
    );
    let pd: Vec<_> = headers
        .iter()
        .map(programdata_address)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let header_slot = slot(&e[2].result)?;
    ensure!(header_slot >= initial_slot, "old program header context");
    let addresses = account_plan(&initial, &pd);
    ensure!(
        e[3].method == "getMultipleAccounts"
            && e[3].params == serde_json::json!([addresses, config(header_slot)]),
        "invalid final execution account plan"
    );
    let values = e[3].result["value"]
        .as_array()
        .context("missing execution account batch")?;
    ensure!(
        values.len() == addresses.len(),
        "missing required execution accounts"
    );
    let final_slot = slot(&e[3].result)?;
    ensure!(final_slot >= header_slot, "old final execution context");
    let raws: BTreeMap<_, _> = addresses
        .iter()
        .zip(values)
        .map(|(a, r)| (a.as_str(), r))
        .collect();
    let get = |a: &str| -> Result<&Value> {
        let raw = *raws
            .get(a)
            .with_context(|| format!("missing required protocol account {a}"))?;
        ensure!(!raw.is_null(), "missing required protocol account {a}");
        Ok(raw)
    };
    let r = route_fn(get(&spec.pool)?)?;
    ensure!(
        account_plan(&r, &pd) == addresses,
        "pool/bin account plan changed during capture"
    );
    let clock = clock(get(CLOCK)?)?;
    ensure!(
        clock.slot == final_slot,
        "Clock does not match final captured bank"
    );
    let input = spec.input_amount_raw.parse::<u64>()?;
    let minimum = spec.minimum_output_raw.parse::<u64>()?;
    ensure!(
        input > 0 && minimum > 0 && !spec.amount_reason.trim().is_empty(),
        "bounded positive raw amounts and rationale required"
    );
    let mx = decode::decode_mint(get(&spec.input_mint)?)?;
    let my = decode::decode_mint(get(&spec.output_mint)?)?;
    ensure!(
        mx.token_program == TOKEN_2022_PROGRAM && my.token_program == LEGACY_PROGRAM,
        "wrong mint token programs"
    );
    ensure!(
        !mx.extensions
            .iter()
            .any(|x| x.extension_type == "Pausable" && x.config["paused"] == true),
        "input mint is paused"
    );
    ensure!(
        !mx.extensions
            .iter()
            .any(|x| x.extension_type == "TransferHook" && !x.config["programId"].is_null()),
        "active transfer hook is unsupported; required hook accounts cannot be proved"
    );
    let source = decode::decode_token_account(
        get(&r.source)?,
        TOKEN_2022_PROGRAM,
        &spec.input_mint,
        mx.decimals,
    )?;
    let authority = get(&r.user.to_string())?;
    ensure!(
        r.user.is_on_curve()
            && authority["owner"] == "11111111111111111111111111111111"
            && authority["executable"] == false
            && decode::raw_account_bytes(authority)?.is_empty(),
        "captured holder authority is no longer wallet-compatible"
    );
    ensure!(
        source.owner == r.user.to_string() && source.is_initialized && !source.is_frozen,
        "invalid/frozen source or changed owner"
    );
    ensure!(
        source.raw_balance.parse::<u64>()? >= input,
        "insufficient input token balance"
    );
    ensure!(
        !source
            .extensions
            .iter()
            .any(|x| x.extension_type == "ConfidentialTransferAccount"),
        "confidential source requires additional transfer approval proof"
    );
    let dest = raws[r.destination.to_string().as_str()];
    if !dest.is_null() {
        let d = decode::decode_token_account(dest, LEGACY_PROGRAM, &spec.output_mint, my.decimals)?;
        ensure!(
            d.owner == r.user.to_string() && d.is_initialized && !d.is_frozen,
            "destination cannot receive paired asset"
        );
    }
    for (i, m) in [(&spec.input_mint, &mx), (&spec.output_mint, &my)]
        .into_iter()
        .enumerate()
    {
        let vault = decode::decode_token_account(
            get(&r.pool.vaults[i].to_string())?,
            &m.1.token_program,
            m.0,
            m.1.decimals,
        )?;
        ensure!(
            vault.owner == spec.pool && vault.is_initialized && !vault.is_frozen,
            "wrong/frozen pool vault authority"
        );
    }
    for (index, address) in &r.bins {
        let b = decode::account_bytes(get(&address.to_string())?, dlmm::PROGRAM_ID)?;
        ensure!(
            b.len() == BIN_ARRAY_LEN
                && b[..8] == [92, 142, 92, 220, 5, 148, 70, 181]
                && i64::from_le_bytes(b[8..16].try_into()?) == *index
                && key_at(&b, 24)? == r.pool.pool,
            "malformed bin array or wrong pool/index"
        );
        ensure!(b[16] <= 3, "unsupported bin array version");
    }
    let oracle = decode::account_bytes(get(&r.oracle.to_string())?, dlmm::PROGRAM_ID)?;
    ensure!(
        oracle.len() >= 8 && oracle[..8] == [139, 194, 131, 179, 140, 179, 229, 244],
        "wrong oracle discriminator"
    );
    let bitmap = raws[r.bitmap.to_string().as_str()];
    if !bitmap.is_null() {
        let b = decode::account_bytes(bitmap, dlmm::PROGRAM_ID)?;
        ensure!(
            b.len() == 1576
                && b[..8] == [80, 111, 124, 113, 55, 237, 18, 5]
                && key_at(&b, 8)? == r.pool.pool,
            "wrong bitmap extension"
        );
    }
    let mut programs = Vec::new();
    for (i, id) in program_ids().iter().enumerate() {
        let raw = get(id)?;
        ensure!(
            programdata_address(raw)? == programdata_address(&headers[i])?,
            "programdata address changed during capture"
        );
        let bytes = if let Some(pd) = programdata_address(raw)? {
            ensure!(
                Address::find_program_address(
                    &[id.parse::<Address>()?.as_ref()],
                    &UPGRADEABLE_LOADER.parse()?
                )
                .0
                .to_string()
                    == pd,
                "ProgramData is not the canonical loader PDA"
            );
            let state = get(&pd)?;
            ensure!(
                state["owner"] == UPGRADEABLE_LOADER && state["executable"] == false,
                "invalid ProgramData owner/state"
            );
            let b = decode::raw_account_bytes(state)?;
            let programdata = crate::standard_programs::upgradeable_loader::decode_programdata(&b)?;
            ensure!(
                !programdata.bytes.is_empty() && programdata.deploy_slot < clock.slot,
                "malformed/new ProgramData"
            );
            programdata.bytes
        } else {
            decode::raw_account_bytes(raw)?
        };
        ensure!(bytes.starts_with(b"\x7fELF"), "captured program has no ELF");
        programs.push(LoadedProgram {
            program_id: id.parse()?,
            loader: raw["owner"].as_str().unwrap().parse()?,
            bytes,
        });
    }
    let mut accounts = Vec::new();
    let mut evidence = Vec::new();
    for (i, address) in addresses.iter().enumerate() {
        let raw = &values[i];
        evidence.push(ExecutionAccountEvidence {
            address: address.clone(),
            rpc_record: 3,
            pointer: format!("/value/{i}"),
            slot: final_slot,
            exists: !raw.is_null(),
            runtime_owner: raw["owner"].as_str().map(str::to_string),
            raw_data_sha256: if raw.is_null() {
                None
            } else {
                Some(sha256(&decode::raw_account_bytes(raw)?))
            },
        });
        if !raw.is_null() {
            accounts.push(captured_account(address, raw)?);
        }
    }
    ensure!(
        !addresses.contains(&payer().to_string()),
        "synthetic payer collides with captured accounts"
    );
    accounts.push(NamedAccount {
        label: "local-fee-payer".into(),
        address: payer().to_string(),
        account: AccountSnapshot {
            lamports: 1000000000,
            owner: "11111111111111111111111111111111".into(),
            data: vec![],
            executable: false,
            rent_epoch: 0,
        },
    });
    let mut instructions = vec![Instruction {
        program_id: "ComputeBudget111111111111111111111111111111".parse()?,
        accounts: vec![],
        data: [vec![2], 1400000u32.to_le_bytes().to_vec()].concat(),
    }];
    if dest.is_null() {
        instructions.push(Instruction {
            program_id: ATA_PROGRAM.parse()?,
            accounts: vec![
                AccountMeta::new(payer(), true),
                AccountMeta::new(r.destination, false),
                AccountMeta::new_readonly(r.user, false),
                AccountMeta::new_readonly(r.pool.mints[1], false),
                AccountMeta::new_readonly("11111111111111111111111111111111".parse()?, false),
                AccountMeta::new_readonly(LEGACY_PROGRAM.parse()?, false),
            ],
            data: vec![1],
        });
    }
    let program: Address = dlmm::PROGRAM_ID.parse()?;
    let mut metas = vec![
        AccountMeta::new(r.pool.pool, false),
        AccountMeta::new(if bitmap.is_null() { program } else { r.bitmap }, false),
        AccountMeta::new(r.pool.vaults[0], false),
        AccountMeta::new(r.pool.vaults[1], false),
        AccountMeta::new(r.source.parse()?, false),
        AccountMeta::new(r.destination, false),
        AccountMeta::new_readonly(r.pool.mints[0], false),
        AccountMeta::new_readonly(r.pool.mints[1], false),
        AccountMeta::new(r.oracle, false),
        AccountMeta::new_readonly(program, false),
        AccountMeta::new_readonly(r.user, true),
        AccountMeta::new_readonly(TOKEN_2022_PROGRAM.parse()?, false),
        AccountMeta::new_readonly(LEGACY_PROGRAM.parse()?, false),
        AccountMeta::new_readonly(MEMO_PROGRAM.parse()?, false),
        AccountMeta::new_readonly(r.event_authority, false),
        AccountMeta::new_readonly(program, false),
    ];
    metas.extend(r.bins.iter().map(|(_, a)| AccountMeta::new(*a, false)));
    let data = [
        SWAP2.to_vec(),
        input.to_le_bytes().to_vec(),
        minimum.to_le_bytes().to_vec(),
        0u32.to_le_bytes().to_vec(),
    ]
    .concat();
    instructions.push(Instruction {
        program_id: program,
        accounts: metas,
        data,
    });
    let message = Message::new(&instructions, Some(&payer()));
    let mut watch = vec![
        r.source.clone(),
        r.destination.to_string(),
        spec.pool.clone(),
        r.oracle.to_string(),
        spec.input_mint.clone(),
        spec.output_mint.clone(),
    ];
    watch.extend(r.pool.vaults.map(|v| v.to_string()));
    watch.extend(r.bins.iter().map(|(_, a)| a.to_string()));
    watch.sort();
    let names = [
        "Pool/mints/vaults match verified venue and canonical PDAs",
        "Input source is wallet-compatible and retains original owner",
        "Sufficient positive raw input balance",
        "Paired-token destination is initialized or created by captured ATA program",
        "Token-2022 mint is not paused",
        "Transfer hook absent/inactive",
        "Source/vault account states permit public transfers",
        "Relevant initialized bin arrays, oracle and optional bitmap are captured",
        "All deployed SBF and token program bytes captured with loader links",
        "Captured Clock matches the final finalized bank",
    ];
    let preconditions = names
        .map(|name| ProbePrecondition {
            name: name.into(),
            proven: true,
            reason:
                "Validated from final RPC raw account bytes and original exposure/source evidence"
                    .into(),
        })
        .to_vec();
    Ok(ProbeExecutionPlan{accounts,watch,programs,message,clock,preconditions,account_evidence:evidence,assumptions:vec![
            "Original holder owner is retained. Signature verification is disabled locally; owner message signer privilege is assumed, not proof of key possession or authorization".into(),
            "Recent-blockhash validation is disabled locally. A deterministic synthetic fee payer funds fees and any real ATA creation; no protocol/token source state is funded or modified".into(),
            "Fresh LiteSVM 0.16 mainnet feature profile and default rent/epoch scheduling/syscall environment are used; this is not a full mainnet validator bank reproduction".into(),
            "All executable route state uses one Phase 5 finalized batch context. Phase 2/3 snapshots are earlier provenance/link observations, not mixed execution balances".into(),
            "Raw token units are canonical. Permanent delegate is not used; default account state and scaled UI display do not override captured initialized accounts or raw transfer amounts".into(),
        ]})
}

fn signed_decimal(n: i128, decimals: u8) -> String {
    if n < 0 {
        format!("-{}", decode::decimal_amount((-n) as u64, decimals))
    } else {
        decode::decimal_amount(n as u64, decimals)
    }
}

pub fn reconcile_current(
    spec: &SwapParameters,
    plan: &ProbeExecutionPlan,
    ex: &ProbeTransactionExecution,
) -> Result<ExecutionDeltas> {
    let pre: BTreeMap<_, _> = plan
        .accounts
        .iter()
        .map(|a| (a.address.as_str(), &a.account))
        .collect();
    let raw_pool = &pre[spec.pool.as_str()].data;
    let vaults = [
        key_at(raw_pool, 152)?.to_string(),
        key_at(raw_pool, 184)?.to_string(),
    ];
    let swap_ix = plan
        .message
        .instructions
        .last()
        .context("missing swap instruction")?;
    let key =
        |index: usize| plan.message.account_keys[usize::from(swap_ix.accounts[index])].to_string();
    let source = key(4);
    let dest = key(5);
    let x = crate::standard_programs::token::mint_base(
        TOKEN_2022_PROGRAM,
        &pre[spec.input_mint.as_str()].data,
    )?;
    let y = crate::standard_programs::token::mint_base(
        LEGACY_PROGRAM,
        &pre[spec.output_mint.as_str()].data,
    )?;
    let mut tokens = Vec::new();
    for (address, mint, program, decimals) in [
        (&source, &spec.input_mint, TOKEN_2022_PROGRAM, x.decimals),
        (&dest, &spec.output_mint, LEGACY_PROGRAM, y.decimals),
        (&vaults[0], &spec.input_mint, TOKEN_2022_PROGRAM, x.decimals),
        (&vaults[1], &spec.output_mint, LEGACY_PROGRAM, y.decimals),
    ] {
        let decode_account = |a: &AccountSnapshot| -> Result<TokenAccountState> {
            use base64::Engine;
            decode::decode_token_account(
                &serde_json::json!({"owner":a.owner,"data":[base64::engine::general_purpose::STANDARD.encode(&a.data),"base64"],"space":a.data.len(),"executable":a.executable}),
                program,
                mint,
                decimals,
            )
        };
        let b = pre
            .get(address.as_str())
            .map(|a| decode_account(a))
            .transpose()?;
        let a = ex
            .post_accounts
            .get(address.as_str())
            .map(decode_account)
            .transpose()?;
        let before = b
            .as_ref()
            .map(|s| s.raw_balance.parse::<u64>())
            .transpose()?
            .unwrap_or(0);
        let after = a
            .as_ref()
            .map(|s| s.raw_balance.parse::<u64>())
            .transpose()?
            .unwrap_or(0);
        let w_before = b.as_ref().map(withheld).transpose()?.unwrap_or(0);
        let w_after = a.as_ref().map(withheld).transpose()?.unwrap_or(0);
        let change = i128::from(after) - i128::from(before);
        tokens.push(TokenAccountDelta {
            address: address.clone(),
            mint: mint.clone(),
            before_raw: before.to_string(),
            after_raw: after.to_string(),
            change_raw: change.to_string(),
            change_decimal_base_units: signed_decimal(change, decimals),
            withheld_fee_change_raw: (i128::from(w_after) - i128::from(w_before)).to_string(),
        });
    }
    let mut account_data = Vec::new();
    for address in &plan.watch {
        let before = pre
            .get(address.as_str())
            .map(|a| a.data.as_slice())
            .unwrap_or(&[]);
        let after = ex
            .post_accounts
            .get(address)
            .map(|a| a.data.as_slice())
            .unwrap_or(&[]);
        account_data.push(AccountDataDelta {
            address: address.clone(),
            before_sha256: sha256(before),
            after_sha256: sha256(after),
            changed_ranges: data_changes(address, before, after),
        });
    }
    let debit = -tokens[0].change_raw.parse::<i128>()?;
    let output = tokens[1].change_raw.parse::<i128>()?;
    ensure!(
        debit >= 0 && output >= 0,
        "unexpected negative input debit/output receipt"
    );
    let mut fees = None;
    let mut reconciled = tokens
        .iter()
        .all(|a| a.change_raw == "0" && a.withheld_fee_change_raw == "0");
    let mut reconciliation=vec!["Amounts reconcile public SPL account amounts; withheld fees are separate extension balances, not additional holder amounts".into()];
    if ex.success {
        let events: Vec<_> = ex
            .inner_instructions
            .iter()
            .filter(|ix| {
                ix.program == dlmm::PROGRAM_ID
                    && ix
                        .data
                        .starts_with(&[EVENT_CPI.to_vec(), SWAP_EVENT.to_vec()].concat())
            })
            .collect();
        ensure!(
            events.len() == 1,
            "actual Swap2Evt CPI evidence missing or duplicated"
        );
        let event = &events[0].data[16..];
        ensure!(event.len() == 147, "unsupported Swap2Evt Borsh length");
        ensure!(
            key_at(event, 0)?.to_string() == spec.pool
                && key_at(event, 32)?.to_string() == key(10)
                && event[72] == 1,
            "swap event mismatches pool/actor/direction"
        );
        let amount = u64_at(event, 89)?;
        let left = u64_at(event, 97)?;
        let event_output = u64_at(event, 105)?;
        let mm_fee = u64_at(event, 113)?;
        let protocol = u64_at(event, 121)?;
        let order_fee = u64_at(event, 129)?;
        let host = u64_at(event, 137)?;
        let on_input = event[145] != 0;
        let on_x = event[146] != 0;
        let fee = mm_fee
            .checked_add(protocol)
            .and_then(|f| f.checked_add(order_fee))
            .and_then(|f| f.checked_add(host))
            .context("swap fee overflow")?;
        let token_fee = transfer_fee(
            &pre[spec.input_mint.as_str()].data,
            plan.clock.epoch,
            debit as u64,
        )?;
        let pool_after = &ex.post_accounts[spec.pool.as_str()].data;
        let protocol_delta = i128::from(u64_at(pool_after, if on_x { 216 } else { 224 })?)
            - i128::from(u64_at(raw_pool, if on_x { 216 } else { 224 })?);
        let dx = tokens[2].change_raw.parse::<i128>()?;
        let dy = tokens[3].change_raw.parse::<i128>()?;
        let withheld = tokens[2].withheld_fee_change_raw.parse::<i128>()?;
        // Exact MM-only reserve conservation, independently of the swap event.
        // Order fills are deliberately not interpreted by this first bounded probe.
        let mut bin_x = 0i128;
        let mut bin_y = 0i128;
        let mut mm_only = order_fee == 0;
        for address in &plan.watch {
            let before = pre
                .get(address.as_str())
                .map(|a| a.data.as_slice())
                .unwrap_or(&[]);
            if before.len() != BIN_ARRAY_LEN || before[..8] != [92, 142, 92, 220, 5, 148, 70, 181] {
                continue;
            }
            let after = &ex.post_accounts[address].data;
            for bin in 0..70 {
                let n = 56 + bin * 144;
                bin_x += i128::from(u64_at(after, n)?) - i128::from(u64_at(before, n)?);
                bin_y += i128::from(u64_at(after, n + 8)?) - i128::from(u64_at(before, n + 8)?);
                for off in [48, 56, 64, 72, 112, 120, 128] {
                    mm_only &= u64_at(before, n + off)? == u64_at(after, n + off)?;
                }
            }
        }
        reconciled = debit == i128::from(spec.input_amount_raw.parse::<u64>()?)
            && output >= i128::from(spec.minimum_output_raw.parse::<u64>()?)
            && amount == debit as u64
            && left == 0
            && event_output == output as u64
            && host == 0
            && debit == dx + withheld
            && withheld == i128::from(token_fee)
            && output == -dy
            && protocol_delta == i128::from(protocol)
            && on_input == on_x
            && mm_only
            && dx == bin_x + if on_x { i128::from(fee) } else { 0 }
            && dy == bin_y + if on_x { 0 } else { i128::from(fee) };
        fees = Some(SwapFeeDeltas {
            fee_asset_mint: if on_x {
                spec.input_mint.clone()
            } else {
                spec.output_mint.clone()
            },
            token_2022_transfer_fee_raw: token_fee.to_string(),
            dlmm_swap_fee_raw: fee.to_string(),
            dlmm_protocol_fee_raw: protocol.to_string(),
            dlmm_liquidity_provider_fee_raw: mm_fee.to_string(),
            host_fee_raw: host.to_string(),
            active_epoch: plan.clock.epoch,
        });
        reconciliation.extend([format!("User input debit {debit} = X vault public delta {dx} + X vault withheld fee {withheld}"),
            format!("User output credit {output} = -Y vault public delta {dy}"),format!("Actual Swap2Evt protocol fee {protocol} = pool protocol accumulator delta {protocol_delta}; total fee {fee}; fee-on-input={on_input}"),
            format!("MM-only bins: X delta {bin_x}, Y delta {bin_y}; vault deltas equal bin deltas plus fee {fee} on the identified fee asset"),
            "DLMM fees remain within the relevant vault's public amount and are not added again to token conservation; Swap2Evt records MM/protocol/order components".into()]);
    } else {
        reconciliation.push("Failed atomic transaction has zero watched token/withheld deltas; only transaction fee payer may be charged".into());
    }
    Ok(ExecutionDeltas {
        input_debited_raw: debit.to_string(),
        output_received_raw: output.to_string(),
        output_decimal_base_units: decode::decimal_amount(output as u64, y.decimals),
        token_accounts: tokens,
        account_data,
        fees,
        reconciled,
        reconciliation,
    })
}
