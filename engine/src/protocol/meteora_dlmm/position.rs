//! Bounded PositionV2 (8120 bytes) and RemoveLiquidityByRange2 from the pinned official IDL.
use crate::path::position::*;
use crate::{
    executor::{execute_probe_message, LoadedProgram},
    lifecycle::frozen::selection::canonical,
    lifecycle::{
        decode,
        exposure::{meteora_dlmm as dlmm, sha256},
        policy::LifecycleScenario,
        LifecycleSnapshot,
    },
    path::{meteora_dlmm as shared, ProbeClock, ProbeMessage},
    types::{AccountMetaSpec, NamedAccount},
};
use crate::{
    lifecycle::resolution::{PathStatus, SignerAssumption},
    path::ExitPathType,
    types::{AccountSnapshot, InstructionSpec},
};
use anyhow::{ensure, Context, Result};
use serde_json::json;
use serde_json::Value;
use solana_address::Address;
use solana_clock::Clock;
use solana_instruction::{AccountMeta, Instruction};
use solana_message::Message;
use std::collections::BTreeMap;

/// Sealed execution result, constructed only by the protocol executor.
#[derive(Clone, Debug)]
pub struct VerifiedWithdrawal {
    scope: WithdrawalScope,
    path_type: ExitPathType,
    status: PathStatus,
}
impl VerifiedWithdrawal {
    pub fn scope(&self) -> &WithdrawalScope {
        &self.scope
    }
    pub fn path_type(&self) -> ExitPathType {
        self.path_type
    }
    pub fn status(&self) -> PathStatus {
        self.status
    }
}
pub const POSITION_DISCRIMINATOR: [u8; 8] = [117, 176, 212, 199, 245, 180, 133, 182];
pub const REMOVE_BY_RANGE2: [u8; 8] = [204, 2, 195, 145, 53, 145, 145, 205];
const SYSTEM: &str = "11111111111111111111111111111111";
const POSITION_LEN: usize = 8120;
#[derive(Clone, Debug)]
struct PositionState {
    pool: String,
    owner: String,
    lower: i32,
    upper: i32,
    shares: Vec<u128>,
    pending: [u64; 2],
    complete: Vec<[u128; 2]>,
    lock: u64,
    fields: Value,
}
fn slice(b: &[u8], n: usize, len: usize) -> Result<&[u8]> {
    b.get(n..n + len)
        .context("truncated position/protocol data")
}
fn key(b: &[u8], n: usize) -> Result<String> {
    Ok(Address::new_from_array(slice(b, n, 32)?.try_into()?).to_string())
}
fn u64_at(b: &[u8], n: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(slice(b, n, 8)?.try_into()?))
}
fn u128_at(b: &[u8], n: usize) -> Result<u128> {
    Ok(u128::from_le_bytes(slice(b, n, 16)?.try_into()?))
}
fn i32_at(b: &[u8], n: usize) -> Result<i32> {
    Ok(i32::from_le_bytes(slice(b, n, 4)?.try_into()?))
}
fn position_state(raw: &Value) -> Result<PositionState> {
    let b = decode::account_bytes(raw, dlmm::PROGRAM_ID)?;
    ensure!(
        b.len() == POSITION_LEN && b[..8] == POSITION_DISCRIMINATOR,
        "not a complete supported PositionV2 account; pool vault is not a position"
    );
    let lower = i32_at(&b, 7912)?;
    let upper = i32_at(&b, 7916)?;
    ensure!(
        lower <= upper && i64::from(upper) - i64::from(lower) < 70,
        "unsupported position bin range"
    );
    let shares = (0..70)
        .map(|i| u128_at(&b, 72 + i * 16))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        shares[(upper - lower + 1) as usize..]
            .iter()
            .all(|v| *v == 0),
        "liquidity outside encoded position range"
    );
    let mut pending = [0u64; 2];
    let mut complete = Vec::new();
    for i in 0..70 {
        let o = 4552 + i * 48;
        complete.push([u128_at(&b, o)?, u128_at(&b, o + 16)?]);
        for (side, total) in pending.iter_mut().enumerate() {
            *total = total
                .checked_add(u64_at(&b, o + 32 + side * 8)?)
                .context("pending fee overflow")?;
        }
    }
    let pool = key(&b, 8)?;
    let owner = key(&b, 40)?;
    let lock = u64_at(&b, 7992)?;
    let fields = json!({"type":"PositionV2","lower_bin_id":lower,"upper_bin_id":upper,
        "owner":owner,"pool":pool,"last_updated_at":i64::from_le_bytes(slice(&b,7920,8)?.try_into()?).to_string(),
        "total_claimed_fee_raw":[u64_at(&b,7928)?.to_string(),u64_at(&b,7936)?.to_string()],
        "total_claimed_rewards_raw":[u64_at(&b,7944)?.to_string(),u64_at(&b,7952)?.to_string()],
        "operator":key(&b,7960)?,"lock_release_point":lock.to_string(),"fee_owner":key(&b,8001)?,
        "version":b[8033],"permissionless_operation_bits":b[8034],
        "pending_fees_raw":pending.map(|x|x.to_string()),"liquidity_shares":shares.iter().map(ToString::to_string).collect::<Vec<_>>()});
    ensure!(b[8033] <= 1, "unsupported position version");
    Ok(PositionState {
        pool,
        owner,
        lower,
        upper,
        shares,
        pending,
        complete,
        lock,
        fields,
    })
}
struct Batch {
    raws: BTreeMap<String, Value>,
    slot: u64,
    at: String,
}
fn batch(fixture: &[u8], genesis: &str) -> Result<Batch> {
    let c: Value = serde_json::from_slice(fixture)?;
    let records = c["records"].as_array().context("missing capture records")?;
    ensure!(
        records.len() == 3
            && records[0]["request"]["method"] == "getGenesisHash"
            && records[0]["response"]["result"] == genesis,
        "wrong chain or incomplete capture"
    );
    ensure!(
        records[0]["request"]["params"] == json!([]),
        "wrong genesis request"
    );
    for record in &records[1..] {
        ensure!(
            record["request"]["method"] == "getMultipleAccounts"
                && record["request"]["params"][1]["encoding"] == "base64"
                && record["request"]["params"][1]["commitment"] == "finalized"
                && record["response"].get("error").is_none(),
            "capture is not full finalized account state"
        );
    }
    let first_slot = records[1]["response"]["result"]["context"]["slot"]
        .as_u64()
        .context("missing initial bank")?;
    let final_record = &records[2];
    let slot = final_record["response"]["result"]["context"]["slot"]
        .as_u64()
        .context("missing final bank")?;
    ensure!(
        slot >= first_slot && final_record["request"]["params"][1]["minContextSlot"] == first_slot,
        "incoherent capture contexts"
    );
    let keys = final_record["request"]["params"][0]
        .as_array()
        .context("missing capture addresses")?;
    let values = final_record["response"]["result"]["value"]
        .as_array()
        .context("missing capture accounts")?;
    ensure!(
        keys.len() == values.len() && keys.len() <= 100,
        "incomplete bounded account batch"
    );
    let mut raws = BTreeMap::new();
    for (k, v) in keys.iter().zip(values) {
        ensure!(
            raws.insert(
                k.as_str().context("invalid account address")?.to_string(),
                v.clone()
            )
            .is_none(),
            "duplicate captured account"
        );
    }
    Ok(Batch {
        raws,
        slot,
        at: c["captured_at"]
            .as_str()
            .context("missing capture time")?
            .into(),
    })
}
fn get<'a>(raws: &'a BTreeMap<String, Value>, address: &str) -> Result<&'a Value> {
    let v = raws
        .get(address)
        .with_context(|| format!("missing required public account {address}"))?;
    ensure!(!v.is_null(), "missing required public account {address}");
    Ok(v)
}
fn derived(seed: &[u8], pool: &str) -> Result<String> {
    Ok(Address::find_program_address(
        &[seed, pool.parse::<Address>()?.as_ref()],
        &dlmm::PROGRAM_ID.parse()?,
    )
    .0
    .to_string())
}
fn bin_address(pool: &str, index: i64) -> Result<String> {
    Ok(Address::find_program_address(
        &[
            b"bin_array",
            pool.parse::<Address>()?.as_ref(),
            &index.to_le_bytes(),
        ],
        &dlmm::PROGRAM_ID.parse()?,
    )
    .0
    .to_string())
}
fn destination(owner: &str, mint: &str, program: &str) -> Result<String> {
    Ok(Address::find_program_address(
        &[
            owner.parse::<Address>()?.as_ref(),
            program.parse::<Address>()?.as_ref(),
            mint.parse::<Address>()?.as_ref(),
        ],
        &shared::ATA_PROGRAM.parse()?,
    )
    .0
    .to_string())
}
fn authority_model(owner: &str, raw: &Value) -> Result<AuthorityModel> {
    let addr: Address = owner.parse()?;
    Ok(if !addr.is_on_curve() {
        AuthorityModel::PDA
    } else if raw.is_null() {
        AuthorityModel::Unknown
    } else if raw["owner"] == SYSTEM
        && raw["executable"] == false
        && decode::raw_account_bytes(raw)?.is_empty()
    {
        AuthorityModel::DirectSigner
    } else {
        AuthorityModel::ProgramControlled
    })
}
fn bind_position(state: &PositionState, pool: &str, authority: &str) -> Result<()> {
    ensure!(state.pool == pool, "position must be bound to exact pool");
    ensure!(
        state.owner == authority,
        "position authority does not match withdrawal account plan"
    );
    Ok(())
}
#[derive(Clone)]
struct Bin {
    address: String,
    offset: usize,
    supply: u128,
    amounts: [u64; 2],
    fees: [u128; 2],
}
fn bin(raws: &BTreeMap<String, Value>, pool: &str, id: i32) -> Result<Bin> {
    let index = i64::from(id).div_euclid(70);
    let address = bin_address(pool, index)?;
    let b = decode::account_bytes(get(raws, &address)?, dlmm::PROGRAM_ID)?;
    ensure!(
        b.len() == shared::BIN_ARRAY_LEN
            && b[..8] == [92, 142, 92, 220, 5, 148, 70, 181]
            && i64::from_le_bytes(slice(&b, 8, 8)?.try_into()?) == index
            && key(&b, 24)? == pool
            && b[16] <= 3,
        "wrong bin array/pool/index/layout"
    );
    let offset = 56 + (id.rem_euclid(70) as usize) * 144;
    Ok(Bin {
        address,
        offset,
        supply: u128_at(&b, offset + 32)?,
        amounts: [u64_at(&b, offset)?, u64_at(&b, offset + 8)?],
        fees: [u128_at(&b, offset + 80)?, u128_at(&b, offset + 96)?],
    })
}
fn proportional(amount: u64, shares: u128, supply: u128) -> Result<u64> {
    if shares == 0 {
        return Ok(0);
    }
    ensure!(
        supply > 0 && shares <= supply,
        "position shares exceed actual bin liquidity supply"
    );
    // Checked bounded arithmetic: unrepresentable products never become invented amounts.
    Ok(u64::try_from(
        u128::from(amount)
            .checked_mul(shares)
            .context("unsupported proportional arithmetic overflow")?
            / supply,
    )?)
}
fn exposure(state: &PositionState, raws: &BTreeMap<String, Value>) -> Result<([u64; 2], [u64; 2])> {
    let mut amounts = [0u64; 2];
    let mut fees = state.pending;
    for (i, share) in state
        .shares
        .iter()
        .enumerate()
        .take((state.upper - state.lower + 1) as usize)
    {
        let b = bin(raws, &state.pool, state.lower + i as i32)?;
        for side in 0..2 {
            amounts[side] = amounts[side]
                .checked_add(proportional(b.amounts[side], *share, b.supply)?)
                .context("exposure overflow")?;
            if *share > 0 {
                let delta = b.fees[side]
                    .checked_sub(state.complete[i][side])
                    .context("fee accumulator underflow")?;
                let accrued = (*share >> 64)
                    .checked_mul(delta)
                    .context("fee multiplication overflow")?
                    >> 64;
                fees[side] = fees[side]
                    .checked_add(u64::try_from(accrued)?)
                    .context("accrued fee overflow")?;
            }
        }
    }
    Ok((amounts, fees))
}
pub fn discover(
    snapshot: &LifecycleSnapshot,
    scenario: &LifecycleScenario,
    discovery: &[u8],
    fixture: &[u8],
) -> Result<ProtocolPosition> {
    snapshot.validate()?;
    scenario.validate()?;
    discover_validated(snapshot, scenario, discovery, fixture)
}
fn discover_validated(
    snapshot: &LifecycleSnapshot,
    scenario: &LifecycleScenario,
    discovery: &[u8],
    fixture: &[u8],
) -> Result<ProtocolPosition> {
    ensure!(
        snapshot.asset.mint == scenario.policy.asset_mint,
        "asset/policy mismatch"
    );
    let batch = batch(fixture, &snapshot.source.genesis_hash)?;
    let candidates = batch
        .raws
        .iter()
        .filter(|(_, r)| {
            r["owner"] == dlmm::PROGRAM_ID
                && decode::raw_account_bytes(r)
                    .is_ok_and(|b| b.len() == POSITION_LEN && b[..8] == POSITION_DISCRIMINATOR)
        })
        .collect::<Vec<_>>();
    ensure!(
        candidates.len() == 1,
        "exactly one selected real position required in execution batch"
    );
    let (address, raw) = candidates[0];
    let state = position_state(raw)?;
    let known = snapshot
        .exposures
        .as_ref()
        .context("verified pool snapshot required")?
        .protocol_exposures
        .iter()
        .find(|e| e.pool_address == state.pool)
        .context("position pool is not an existing verified venue")?;
    ensure!(
        batch.slot >= known.discovered_at_slot,
        "capture predates pool verification"
    );
    let pool = dlmm::decode_pool(
        &state.pool,
        get(&batch.raws, &state.pool)?,
        &snapshot.asset.mint,
    )?;
    let discovery: Value = serde_json::from_slice(discovery)?;
    ensure!(
        discovery["request"]["method"] == "getProgramAccounts"
            && discovery["request"]["params"][0] == dlmm::PROGRAM_ID
            && discovery["request"]["params"][1]["commitment"] == "finalized"
            && discovery["request"]["params"][1]["encoding"] == "base64"
            && discovery["request"]["params"][1]["withContext"] == true,
        "invalid position discovery request"
    );
    let filters = discovery["request"]["params"][1]["filters"]
        .as_array()
        .context("missing bounded discovery filters")?;
    ensure!(
        filters.len() == 2
            && filters[1]["memcmp"]["offset"] == 8
            && filters[1]["memcmp"]["bytes"] == state.pool
            && filters[0]["memcmp"]["offset"] == 0,
        "discovery not scoped to exact pool/position discriminator"
    );
    // Decode returned bytes, rather than trusting the RPC discriminator filter alone.
    let items = discovery["response"]["result"]["value"]
        .as_array()
        .context("missing real discovery response")?;
    let original = items
        .iter()
        .find(|v| v["pubkey"] == *address)
        .context("position absent from real pool discovery")?;
    let original_state = position_state(&original["account"])?;
    bind_position(&original_state, &state.pool, &state.owner)?;
    ensure!(
        original_state.lower == state.lower && original_state.upper == state.upper,
        "position geometry changed between discovery and capture"
    );
    let discovery_slot = discovery["response"]["result"]["context"]["slot"]
        .as_u64()
        .context("missing discovery slot")?;
    ensure!(
        batch.slot >= discovery_slot,
        "execution capture predates discovery"
    );
    let model = authority_model(
        &state.owner,
        batch.raws.get(&state.owner).unwrap_or(&Value::Null),
    )?;
    let (amounts, fees) = exposure(&state, &batch.raws)?;
    let side = pool
        .mints
        .iter()
        .position(|m| m.to_string() == snapshot.asset.mint)
        .context("missing lifecycle asset")?;
    ensure!(
        amounts[side] > 0,
        "selected position lacks positive SPACEX-side principal exposure"
    );
    Ok(ProtocolPosition {
        schema_version: 1,
        position_id: address.clone(),
        protocol: "MeteoraDlmm".into(),
        pool: state.pool.clone(),
        authority: state.owner.clone(),
        authority_model: model,
        signer: SignerAssumption {
            authority: state.owner.clone(),
            signer_possession_known: false,
            signer_assumed_locally: model == AuthorityModel::DirectSigner,
            wording: "Encoded position owner is retained; local message signing privilege is assumed only for a DirectSigner. Private-key possession and real authorization are not proved.".into(),
        },
        assets: pool.mints.map(|m| m.to_string()),
        lower_bin_id: state.lower,
        upper_bin_id: state.upper,
        liquidity_shares: state.shares.iter().map(ToString::to_string).collect(),
        principal_exposure_raw: amounts.map(|v| v.to_string()),
        pending_fees_raw: state.pending.map(|v| v.to_string()),
        calculated_accrued_fees_raw: fees.map(|v| v.to_string()),
        position_account_fields: state.fields,
        snapshot_sha256: sha256(snapshot.to_json()?.as_bytes()),
        scenario_sha256: scenario.sha256()?,
        discovery_sha256: sha256(canonical(&discovery)?.as_bytes()),
        fixture_sha256: sha256(fixture),
        captured_slot: batch.slot,
        captured_at: batch.at,
        raw_position_sha256: sha256(&decode::raw_account_bytes(raw)?),
        selection_reason: format!("Bounded discovery in one already verified pool returned {} position account(s). The selected captured PositionV2 has positive lifecycle-asset principal, an encoded owner, a two-array range and existing owner destinations. Selection precedes withdrawal execution; success is not a selection condition.", items.len()),
        lifecycle_status: scenario.policy.status_at(scenario.policy.effective_at),
        policy_evaluated_at: scenario.policy.effective_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        limitations: vec![
            "Exposure is floor(bin public principal × position shares / bin supply) per bin, excluding accrued fees and destination transfer fees; execution outputs are independent observations.".into(),
            "Frozen lifecycle population supplies pool/policy provenance; all execution balances use the fresh single finalized batch, not a historical population bank.".into(),
            "Metadata, mint delegation and pool-vault authority do not establish LP ownership; encoded position owner is the authority proof.".into(),
        ],
    })
}
struct Prepared {
    batch: Batch,
    state: PositionState,
    pool: dlmm::DecodedPool,
    clock: Clock,
    programs: Vec<LoadedProgram>,
    program_evidence: Vec<ProgramEvidence>,
    accounts: Vec<NamedAccount>,
    local: NamedAccount,
    watch: Vec<String>,
    instructions: Vec<Instruction>,
    message: Message,
    destinations: [String; 2],
    mint_configs: [decode::MintConfig; 2],
}
fn prepare(
    position: &ProtocolPosition,
    probe: &WithdrawalProbe,
    fixture: &[u8],
    genesis: &str,
) -> Result<Prepared> {
    let batch = batch(fixture, genesis)?;
    ensure!(
        batch.slot == position.captured_slot && sha256(fixture) == position.fixture_sha256,
        "position fixture fingerprint/bank mismatch"
    );
    ensure!(
        probe.position_id == position.position_id && probe.pool == position.pool,
        "withdrawal scope does not match exact position/pool"
    );
    let raw = get(&batch.raws, &position.position_id)?;
    let state = position_state(raw)?;
    ensure!(
        sha256(&decode::raw_account_bytes(raw)?) == position.raw_position_sha256,
        "position raw evidence changed"
    );
    bind_position(&state, &probe.pool, &probe.authority)?;
    ensure!(
        position.authority_model == AuthorityModel::DirectSigner,
        "unsupported position authority model: no issuer/admin/PDA caller may be assumed"
    );
    ensure!(
        state.owner == position.authority
            && position.signer.authority == state.owner
            && !position.signer.signer_possession_known
            && position.signer.signer_assumed_locally,
        "retained signer assumptions must remain explicit; no known-key claim"
    );
    ensure!(
        state.lower == position.lower_bin_id
            && state.upper == position.upper_bin_id
            && state
                .shares
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                == position.liquidity_shares,
        "position accounting mismatch"
    );
    ensure!(
        probe.lower_bin_id >= state.lower
            && probe.upper_bin_id <= state.upper
            && probe.lower_bin_id <= probe.upper_bin_id
            && probe.bps_to_remove > 0
            && probe.bps_to_remove <= 10000,
        "invalid exact withdrawal range/fraction"
    );
    ensure!(
        probe.compute_unit_limit > 0 && probe.compute_unit_limit <= 1400000,
        "invalid compute budget"
    );
    let owner = get(&batch.raws, &state.owner)?;
    ensure!(
        authority_model(&state.owner, owner)? == AuthorityModel::DirectSigner
            && position.authority_model == AuthorityModel::DirectSigner,
        "unsupported position authority model: no issuer/admin/PDA caller may be assumed"
    );
    let pool = dlmm::decode_pool(
        &state.pool,
        get(&batch.raws, &state.pool)?,
        &position.assets[0],
    )?;
    ensure!(
        pool.mints.map(|m| m.to_string()) == position.assets,
        "position assets mismatch"
    );
    let clock = shared::clock(get(&batch.raws, shared::CLOCK)?)?;
    ensure!(
        clock.slot == batch.slot,
        "captured Clock/final bank mismatch"
    );
    ensure!(
        state.lock == 0,
        "locked position requires additional lock/activation semantics"
    );
    let (principal, fees) = exposure(&state, &batch.raws)?;
    ensure!(
        principal.map(|v| v.to_string()) == position.principal_exposure_raw
            && state.pending.map(|v| v.to_string()) == position.pending_fees_raw
            && fees.map(|v| v.to_string()) == position.calculated_accrued_fees_raw,
        "position exposure/fee evidence mismatch"
    );
    let bitmap = derived(b"bitmap", &state.pool)?;
    let bm = decode::account_bytes(get(&batch.raws, &bitmap)?, dlmm::PROGRAM_ID)?;
    ensure!(
        bm.len() == 1576
            && bm[..8] == [80, 111, 124, 113, 55, 237, 18, 5]
            && key(&bm, 8)? == state.pool,
        "missing or wrong bitmap-extension state"
    );
    // This supported slice requires the observed initialized bitmap, rather than fabricating absence.
    let mint_configs = [
        decode::decode_mint(get(&batch.raws, &position.assets[0])?)?,
        decode::decode_mint(get(&batch.raws, &position.assets[1])?)?,
    ];
    let destinations = [
        destination(&state.owner, &position.assets[0], &pool.token_programs[0])?,
        destination(&state.owner, &position.assets[1], &pool.token_programs[1])?,
    ];
    for side in 0..2 {
        let mint = &mint_configs[side];
        ensure!(
            mint.token_program == pool.token_programs[side],
            "wrong mint program"
        );
        ensure!(
            !mint
                .extensions
                .iter()
                .any(
                    |e| (e.extension_type == "Pausable" && e.config["paused"] == true)
                        || (e.extension_type == "TransferHook" && !e.config["programId"].is_null())
                ),
            "paused mint or active hook requires unsupported extra state"
        );
        for (address, authority) in [
            (&destinations[side], &state.owner),
            (&pool.vaults[side].to_string(), &state.pool),
        ] {
            let token = decode::decode_token_account(
                get(&batch.raws, address)?,
                &pool.token_programs[side],
                &position.assets[side],
                mint.decimals,
            )?;
            ensure!(
                token.owner == *authority && token.is_initialized && !token.is_frozen,
                "wrong destination/reserve authority/mint/state"
            );
            ensure!(
                !token
                    .extensions
                    .iter()
                    .any(|e| e.extension_type == "ConfidentialTransferAccount"),
                "confidential account execution is outside this slice"
            );
        }
    }
    let mut programs = Vec::new();
    let mut program_evidence = Vec::new();
    for id in [
        dlmm::PROGRAM_ID,
        pool.token_programs[0].as_str(),
        pool.token_programs[1].as_str(),
        shared::MEMO_PROGRAM,
    ] {
        if programs
            .iter()
            .any(|p: &LoadedProgram| p.program_id.to_string() == id)
        {
            continue;
        }
        let raw = get(&batch.raws, id)?;
        let pd = shared::programdata_address(raw)?;
        let mut deployment = None;
        let mut upgrade = None;
        let bytes = if let Some(ref pd) = pd {
            ensure!(
                Address::find_program_address(
                    &[id.parse::<Address>()?.as_ref()],
                    &shared::UPGRADEABLE_LOADER.parse()?
                )
                .0
                .to_string()
                    == *pd,
                "noncanonical loader ProgramData link"
            );
            let pr = get(&batch.raws, pd)?;
            ensure!(
                pr["owner"] == shared::UPGRADEABLE_LOADER && pr["executable"] == false,
                "invalid ProgramData runtime owner/state"
            );
            let b = decode::raw_account_bytes(pr)?;
            let programdata = crate::standard_programs::upgradeable_loader::decode_programdata(&b)?;
            ensure!(
                !programdata.bytes.is_empty() && programdata.deploy_slot < batch.slot,
                "program deployed after capture"
            );
            deployment = Some(programdata.deploy_slot);
            upgrade = programdata.upgrade_authority.map(|key| key.to_string());
            programdata.bytes
        } else {
            decode::raw_account_bytes(raw)?
        };
        ensure!(bytes.starts_with(b"\x7fELF"), "missing deployed ELF bytes");
        let loader = raw["owner"].as_str().context("missing loader")?.to_string();
        program_evidence.push(ProgramEvidence {
            program: id.into(),
            loader: loader.clone(),
            programdata: pd,
            deployment_slot: deployment,
            upgrade_authority: upgrade,
            elf_sha256: sha256(&bytes),
        });
        programs.push(LoadedProgram {
            program_id: id.parse()?,
            loader: loader.parse()?,
            bytes,
        });
    }
    let event =
        Address::find_program_address(&[b"__event_authority"], &dlmm::PROGRAM_ID.parse()?).0;
    // Null event-authority account is legitimate; the address is still the verified PDA.
    ensure!(
        batch.raws.contains_key(&event.to_string()),
        "event authority absence was not captured"
    );
    let mut metas = vec![
        AccountMeta::new(position.position_id.parse()?, false),
        AccountMeta::new(state.pool.parse()?, false),
        AccountMeta::new(bitmap.parse()?, false),
        AccountMeta::new(destinations[0].parse()?, false),
        AccountMeta::new(destinations[1].parse()?, false),
        AccountMeta::new(pool.vaults[0], false),
        AccountMeta::new(pool.vaults[1], false),
        AccountMeta::new_readonly(pool.mints[0], false),
        AccountMeta::new_readonly(pool.mints[1], false),
        AccountMeta::new_readonly(state.owner.parse()?, true),
        AccountMeta::new_readonly(pool.token_programs[0].parse()?, false),
        AccountMeta::new_readonly(pool.token_programs[1].parse()?, false),
        AccountMeta::new_readonly(shared::MEMO_PROGRAM.parse()?, false),
        AccountMeta::new_readonly(event, false),
        AccountMeta::new_readonly(dlmm::PROGRAM_ID.parse()?, false),
    ];
    for index in
        i64::from(probe.lower_bin_id).div_euclid(70)..=i64::from(probe.upper_bin_id).div_euclid(70)
    {
        metas.push(AccountMeta::new(
            bin_address(&state.pool, index)?.parse()?,
            false,
        ));
    }
    let data = [
        REMOVE_BY_RANGE2.to_vec(),
        probe.lower_bin_id.to_le_bytes().to_vec(),
        probe.upper_bin_id.to_le_bytes().to_vec(),
        probe.bps_to_remove.to_le_bytes().to_vec(),
        0u32.to_le_bytes().to_vec(),
    ]
    .concat();
    let instructions = vec![
        Instruction {
            program_id: "ComputeBudget111111111111111111111111111111".parse()?,
            accounts: vec![],
            data: [vec![2], probe.compute_unit_limit.to_le_bytes().to_vec()].concat(),
        },
        Instruction {
            program_id: dlmm::PROGRAM_ID.parse()?,
            accounts: metas,
            data,
        },
    ];
    let message = Message::new(&instructions, Some(&shared::payer()));
    ensure!(
        !batch.raws.contains_key(&shared::payer().to_string()),
        "local fee payer collides with captured state"
    );
    let local = NamedAccount {
        label: "local-fee-payer".into(),
        address: shared::payer().to_string(),
        account: AccountSnapshot {
            lamports: 1000000000,
            owner: SYSTEM.into(),
            data: vec![],
            executable: false,
            rent_epoch: 0,
        },
    };
    let mut accounts = batch
        .raws
        .iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(a, v)| shared::captured_account(a, v))
        .collect::<Result<Vec<_>>>()?;
    accounts.push(local.clone());
    let watch = batch
        .raws
        .iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(a, _)| a.clone())
        .collect();
    Ok(Prepared {
        batch,
        state,
        pool,
        clock,
        programs,
        program_evidence,
        accounts,
        local,
        watch,
        instructions,
        message,
        destinations,
        mint_configs,
    })
}
fn scope(position: &ProtocolPosition, probe: &WithdrawalProbe) -> WithdrawalScope {
    WithdrawalScope {
        position_id: probe.position_id.clone(),
        pool: probe.pool.clone(),
        authority: probe.authority.clone(),
        fixture_sha256: position.fixture_sha256.clone(),
        lower_bin_id: probe.lower_bin_id,
        upper_bin_id: probe.upper_bin_id,
        bps_to_remove: probe.bps_to_remove,
    }
}

fn exact_token_conservation(
    user: u64,
    withheld: u64,
    reserve: u64,
    bin: u64,
    expected: u64,
    fee: u64,
) -> Result<()> {
    ensure!(
        user.checked_add(withheld) == Some(reserve)
            && reserve == bin
            && bin == expected
            && withheld == fee,
        "withdrawal user/reserve/bin/principal/transfer-fee conservation mismatch"
    );
    Ok(())
}
fn exact_liquidity_conservation(
    before: u128,
    after: u128,
    supply_before: u128,
    supply_after: u128,
    removed: u128,
) -> Result<()> {
    ensure!(
        before.checked_sub(after) == Some(removed)
            && supply_before.checked_sub(supply_after) == Some(removed),
        "withdrawal position/bin liquidity-share conservation mismatch"
    );
    Ok(())
}
fn execution_status(success: bool, rollback: Option<bool>) -> PathStatus {
    if success {
        PathStatus::Proven
    } else if rollback == Some(true) {
        PathStatus::Failed
    } else {
        PathStatus::Indeterminate
    }
}
pub fn execute(
    position: &ProtocolPosition,
    probe: &WithdrawalProbe,
    snapshot: &LifecycleSnapshot,
    scenario: &LifecycleScenario,
    discovery: &[u8],
    fixture: &[u8],
) -> Result<WithdrawalReport> {
    snapshot.validate()?;
    scenario.validate()?;
    ensure!(
        position.schema_version == 1 && position.protocol == "MeteoraDlmm",
        "unsupported position artifact"
    );
    ensure!(
        position.snapshot_sha256 == sha256(snapshot.to_json()?.as_bytes())
            && position.scenario_sha256 == scenario.sha256()?,
        "position population/policy fingerprints differ"
    );
    ensure!(
        position.fixture_sha256 == sha256(fixture)
            && position.discovery_sha256
                == sha256(canonical(&serde_json::from_slice::<Value>(discovery)?)?.as_bytes()),
        "position raw artifact fingerprint mismatch"
    );
    let scope = scope(position, probe);
    let mut report = WithdrawalReport {
        schema_version: 1,
        position: position.clone(),
        probe: probe.clone(),
        scope: scope.clone(),
        status: PathStatus::Indeterminate,
        execution_attempted: false,
        precondition_error: None,
        captured_clock: None,
        account_evidence: vec![],
        programs: vec![],
        instructions: vec![],
        normalized_message: None,
        local_accounts: vec![],
        execution: None,
        token_reconciliation: vec![],
        bin_reconciliation: vec![],
        post_position_fields: None,
        position_retained: None,
        all_position_liquidity_removed: None,
        fees_claimed_raw: None,
        rollback_verified: None,
        watched_state_changes: BTreeMap::new(),
        paths: vec![],
        limitations: vec![
            "Only native liquidity principal is removed. No claim-fee, reward, close-position, swap, redemption or official conversion instruction is executed.".into(),
            "Original position owner signer privilege is assumed locally; signature verification and recent-blockhash validation are disabled. No private key possession or issuer/protocol-admin signing is claimed.".into(),
            "Captured deployed programs execute in fresh LiteSVM 0.16 with captured Clock and mainnet feature/default sysvar profile, not a complete validator bank.".into(),
            "One position/range/fraction/pool/captured bank proves no peers, other ranges, future liquidity or simultaneous portfolio capacity. RPC hashes are observations, not signed inclusion proofs.".into(),
            "A synthetic fee payer alone supplies local lamports; no protocol reserves, shares, token amounts or authority state is fabricated. No transaction is broadcast.".into(),
        ],
    };
    let prepared = prepare(position, probe, fixture, &snapshot.source.genesis_hash).and_then(|p| {
        // Re-derive all discovery/accounting facts before trusting a serialized position.
        let independently = discover_validated(snapshot, scenario, discovery, fixture)?;
        ensure!(
            independently == *position,
            "position artifact does not match independently decoded production evidence"
        );
        Ok(p)
    });
    let p = match prepared {
        Ok(p) => p,
        Err(e) => {
            let reason = format!("{e:#}");
            report.status = if reason.contains("unsupported position authority model")
                || reason.contains("locked position")
                || reason.contains("active hook")
                || reason.contains("confidential account")
            {
                PathStatus::Unsupported
            } else {
                PathStatus::Indeterminate
            };
            report.precondition_error = Some(reason);
            let proof = VerifiedWithdrawal {
                scope: scope.clone(),
                path_type: ExitPathType::Withdrawal,
                status: report.status,
            };
            report.paths = resolve_position_paths(&scope, Some(&proof));
            return Ok(report);
        }
    };
    report.captured_clock = Some(ProbeClock::from(&p.clock));
    report.programs = p.program_evidence.clone();
    let keys: Vec<_> = p.batch.raws.keys().cloned().collect();
    let capture: Value = serde_json::from_slice(fixture)?;
    let captured_keys = capture["records"][2]["request"]["params"][0]
        .as_array()
        .context("missing keys")?;
    for address in keys {
        let raw = &p.batch.raws[&address];
        let index = captured_keys
            .iter()
            .position(|k| k == &address)
            .context("missing key")?;
        report.account_evidence.push(AccountEvidence {
            address,
            pointer: format!("/records/2/response/result/value/{index}"),
            slot: p.batch.slot,
            exists: !raw.is_null(),
            runtime_owner: raw["owner"].as_str().map(str::to_string),
            raw_data_sha256: if raw.is_null() {
                None
            } else {
                Some(sha256(&decode::raw_account_bytes(raw)?))
            },
        });
    }
    report.local_accounts.push(p.local.clone());
    report.instructions = p
        .instructions
        .iter()
        .map(|ix| InstructionSpec {
            program: ix.program_id.to_string(),
            accounts: ix
                .accounts
                .iter()
                .map(|m| AccountMetaSpec {
                    address: m.pubkey.to_string(),
                    is_signer: m.is_signer,
                    is_writable: m.is_writable,
                })
                .collect(),
            data: ix.data.clone(),
        })
        .collect();
    report.normalized_message = Some(serde_json::to_value(ProbeMessage::from(&p.message))?);
    let execution = execute_probe_message(
        &p.accounts,
        &p.watch,
        p.clock.clone(),
        &p.programs,
        p.message.clone(),
    )?;
    report.execution_attempted = true;
    let mut rollback = true;
    for address in &p.watch {
        let before = shared::captured_account(address, &p.batch.raws[address])?.account;
        let after = execution
            .post_accounts
            .get(address)
            .context("watched account disappeared")?;
        if before != *after {
            rollback = false;
            report.watched_state_changes.insert(address.clone(),json!({"before_sha256":sha256(&before.data),"after_sha256":sha256(&after.data),"data_ranges":shared::data_changes(address,&before.data,&after.data),"lamports_before":before.lamports.to_string(),"lamports_after":after.lamports.to_string()}));
        }
    }
    if !execution.success {
        report.rollback_verified = Some(rollback);
        report.status = execution_status(false, report.rollback_verified);
    } else {
        reconcile(&p, probe, &execution, &mut report)?;
        report.status = execution_status(true, None);
    }
    let proof = VerifiedWithdrawal {
        scope: scope.clone(),
        path_type: ExitPathType::Withdrawal,
        status: report.status,
    };
    report.paths = resolve_position_paths(&scope, Some(&proof));
    report.execution = Some(execution.into());
    Ok(report)
}
fn reconcile(
    p: &Prepared,
    probe: &WithdrawalProbe,
    e: &crate::executor::ProbeTransactionExecution,
    report: &mut WithdrawalReport,
) -> Result<()> {
    let post_raws = e
        .post_accounts
        .iter()
        .map(|(a, v)| (a.clone(), account_json(v)))
        .collect::<BTreeMap<_, _>>();
    let post = position_state(get(&post_raws, &probe.position_id)?)?;
    bind_position(&post, &p.state.pool, &p.state.owner)?;
    ensure!(
        post.lower == p.state.lower && post.upper == p.state.upper,
        "position range/owner/pool changed"
    );
    let mut totals = [0u64; 2];
    for i in 0..70 {
        let share = p.state.shares[i];
        let id = p.state.lower + i as i32;
        if id > p.state.upper {
            ensure!(post.shares[i] == share, "out-of-range shares changed");
            continue;
        }
        let before = bin(&p.batch.raws, &p.state.pool, id)?;
        let after = bin(&post_raws, &p.state.pool, id)?;
        let removed = if id >= probe.lower_bin_id && id <= probe.upper_bin_id {
            share
                .checked_mul(u128::from(probe.bps_to_remove))
                .context("removed-share arithmetic overflow")?
                / 10000
        } else {
            0
        };
        exact_liquidity_conservation(share, post.shares[i], before.supply, after.supply, removed)?;
        let mut amounts = [0u64; 2];
        for (side, total) in totals.iter_mut().enumerate() {
            amounts[side] = proportional(before.amounts[side], removed, before.supply)?;
            ensure!(
                before.amounts[side].checked_sub(after.amounts[side]) == Some(amounts[side]),
                "bin principal removal mismatch"
            );
            *total = total
                .checked_add(amounts[side])
                .context("withdrawal amount overflow")?;
        }
        // Only amount/supply fields may change in the captured bin arrays for this operation.
        let before_bytes = decode::raw_account_bytes(get(&p.batch.raws, &before.address)?)?;
        let after_bytes = decode::raw_account_bytes(get(&post_raws, &after.address)?)?;
        ensure!(
            before_bytes[before.offset + 16..before.offset + 32]
                == after_bytes[after.offset + 16..after.offset + 32]
                && before_bytes[before.offset + 48..before.offset + 144]
                    == after_bytes[after.offset + 48..after.offset + 144],
            "unexplained bin price/fee/order-state mutation"
        );
        report.bin_reconciliation.push(BinReconciliation {
            bin_id: id,
            bin_array: before.address,
            shares_before: share.to_string(),
            shares_after: post.shares[i].to_string(),
            removed_shares: removed.to_string(),
            supply_before: before.supply.to_string(),
            supply_after: after.supply.to_string(),
            amounts_before: before.amounts.map(|v| v.to_string()),
            amounts_after: after.amounts.map(|v| v.to_string()),
            principal_removed_raw: amounts.map(|v| v.to_string()),
        });
    }
    // No peer bin in either captured array may inherit this position's removal.
    for index in i64::from(p.state.lower).div_euclid(70)..=i64::from(p.state.upper).div_euclid(70) {
        let address = bin_address(&p.state.pool, index)?;
        let before = decode::raw_account_bytes(get(&p.batch.raws, &address)?)?;
        let after = decode::raw_account_bytes(get(&post_raws, &address)?)?;
        ensure!(
            before[..56] == after[..56],
            "bin-array identity/header changed"
        );
        for local in 0..70 {
            let id = index * 70 + local;
            if id < i64::from(p.state.lower) || id > i64::from(p.state.upper) {
                let offset = 56 + (local as usize) * 144;
                ensure!(
                    before[offset..offset + 144] == after[offset..offset + 144],
                    "unselected peer bin changed"
                );
            }
        }
    }
    let old_pool = decode::raw_account_bytes(get(&p.batch.raws, &p.state.pool)?)?;
    let new_pool = decode::raw_account_bytes(get(&post_raws, &p.state.pool)?)?;
    ensure!(
        old_pool[216..232] == new_pool[216..232],
        "protocol fee accumulators changed unexpectedly"
    );
    let pairs = crate::evidence::current::pair(&p.accounts, &p.watch, e)?;
    for (side, total) in totals.iter().enumerate() {
        let mint = &p.pool.mints[side].to_string();
        let pr = &p.pool.token_programs[side];
        let decimals = p.mint_configs[side].decimals;
        let reserve = p.pool.vaults[side].to_string();
        let dest = &p.destinations[side];
        let user_amounts = crate::evidence::current::token_amounts(&pairs, dest, pr, mint)?;
        let reserve_amounts = crate::evidence::current::token_amounts(&pairs, &reserve, pr, mint)?;
        let complete = |v: crate::evidence::current::TokenAmounts| -> Result<(u64, u64, u64, u64)> {
            ensure!(
                v.delta.is_some(),
                "token account changed existence during principal removal"
            );
            Ok((
                v.before.context("opening amount")?,
                v.after.context("closing amount")?,
                v.withheld_before.context("opening fee")?,
                v.withheld_after.context("closing fee")?,
            ))
        };
        let (ub, ua, wb, wa) = complete(user_amounts)?;
        let (rb, ra, rwb, rwa) = complete(reserve_amounts)?;
        ensure!(
            rwb == rwa,
            "reserve withheld fees changed on outgoing transfer"
        );
        let user = ua.checked_sub(ub).context("negative user credit")?;
        let withheld = wa
            .checked_sub(wb)
            .context("negative destination withheld fee")?;
        let debit = rb.checked_sub(ra).context("negative reserve debit")?;
        let fee = shared::transfer_fee(
            &decode::raw_account_bytes(get(&p.batch.raws, mint)?)?,
            p.clock.epoch,
            *total,
        )?;
        exact_token_conservation(user, withheld, debit, *total, *total, fee)?;
        ensure!(
            decode::raw_account_bytes(get(&p.batch.raws, mint)?)?
                == decode::raw_account_bytes(get(&post_raws, mint)?)?,
            "mint account changed during withdrawal"
        );
        report.token_reconciliation.push(TokenReconciliation {
            mint: mint.clone(),
            destination: dest.clone(),
            reserve,
            destination_before_raw: ub.to_string(),
            destination_after_raw: ua.to_string(),
            user_credit_raw: user.to_string(),
            destination_withheld_before_raw: wb.to_string(),
            destination_withheld_after_raw: wa.to_string(),
            withheld_credit_raw: withheld.to_string(),
            reserve_before_raw: rb.to_string(),
            reserve_after_raw: ra.to_string(),
            reserve_debit_raw: debit.to_string(),
            protocol_calculated_principal_raw: total.to_string(),
            bin_principal_decrease_raw: total.to_string(),
            transfer_fee_raw: fee.to_string(),
            decimal_user_credit: decode::decimal_amount(user, decimals),
        });
    }
    ensure!(
        post.fields["total_claimed_fee_raw"] == p.state.fields["total_claimed_fee_raw"]
            && post.fields["total_claimed_rewards_raw"]
                == p.state.fields["total_claimed_rewards_raw"],
        "unrequested fee/reward claim occurred"
    );
    // Fee checkpoints update as shares are removed; claimable fees remain in the position.
    if probe.lower_bin_id == p.state.lower && probe.upper_bin_id == p.state.upper {
        let (_, fees) = exposure(&p.state, &p.batch.raws)?;
        ensure!(
            post.pending == fees,
            "retained accrued fee accounting mismatch"
        );
    }
    report.post_position_fields = Some(post.fields);
    report.position_retained = Some(true);
    report.all_position_liquidity_removed = Some(post.shares.iter().all(|s| *s == 0));
    report.fees_claimed_raw = Some(["0".into(), "0".into()]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> PositionState {
        PositionState {
            pool: "pool-a".into(),
            owner: "owner-a".into(),
            lower: -2,
            upper: 1,
            shares: vec![1; 70],
            pending: [0; 2],
            complete: vec![[0; 2]; 70],
            lock: 0,
            fields: Value::Null,
        }
    }
    #[test]
    fn pool_vault_authority_cannot_grant_lp_ownership() {
        assert!(bind_position(&state(), "pool-a", "pool-a").is_err());
    }
    #[test]
    fn position_is_bound_to_exact_pool() {
        assert!(bind_position(&state(), "pool-b", "owner-a").is_err());
    }
    #[test]
    fn wrong_position_authority_is_rejected() {
        assert!(bind_position(&state(), "pool-a", "other-owner").is_err());
    }
    #[test]
    fn successful_token_deltas_require_exact_conservation() {
        exact_token_conservation(995, 5, 1000, 1000, 1000, 5).unwrap();
        for (u, w, r, b, e, f) in [
            (996, 5, 1000, 1000, 1000, 5),
            (995, 4, 1000, 1000, 1000, 5),
            (995, 5, 1001, 1000, 1000, 5),
            (995, 5, 1000, 999, 1000, 5),
            (995, 5, 1000, 1000, 999, 5),
            (995, 5, 1000, 1000, 1000, 4),
        ] {
            assert!(exact_token_conservation(u, w, r, b, e, f).is_err());
        }
    }
    #[test]
    fn position_shares_and_bin_supply_reconcile_exactly() {
        exact_liquidity_conservation(100, 0, 300, 200, 100).unwrap();
        assert!(exact_liquidity_conservation(100, 1, 300, 200, 100).is_err());
        assert!(exact_liquidity_conservation(100, 0, 300, 201, 100).is_err());
    }
    #[test]
    fn failed_withdrawal_never_becomes_proven() {
        assert_eq!(execution_status(false, Some(true)), PathStatus::Failed);
        assert_eq!(
            execution_status(false, Some(false)),
            PathStatus::Indeterminate
        );
    }
    #[test]
    fn proportional_rounding_and_overflow_are_explicit() {
        assert_eq!(proportional(10, 3, 4).unwrap(), 7);
        assert_eq!(proportional(10, 0, 0).unwrap(), 0);
        assert!(proportional(10, 5, 4).is_err());
        assert!(proportional(u64::MAX, u128::MAX, u128::MAX).is_err());
    }
}

#[cfg(test)]
#[path = "position_proof_tests.rs"]
mod proof_tests;
