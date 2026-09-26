//! Bounded original-owner Token-2022 transfer to an actual observed token account.
//! Token movement is neither secondary-market exit nor issuer transition.
use super::captured::{self as shared, CLOCK, UPGRADEABLE_LOADER};
use super::*;
use crate::lifecycle::{
    decode::{self, TOKEN_2022_PROGRAM},
    rpc::SolanaRpc,
};
use anyhow::{ensure, Context};
use serde_json::{json, Value};
use solana_address::Address;
use solana_instruction::Instruction;
use std::collections::{BTreeMap, BTreeSet};
pub const REVISION: &str = "token-2022-transfer-checked-interface-3.1.1-v1";
fn config(slot: u64) -> Value {
    json!({"encoding":"base64","commitment":"finalized","minContextSlot":slot})
}
fn record(
    rpc: &impl SolanaRpc,
    e: &mut Vec<RpcEvidence>,
    method: &str,
    params: Value,
) -> Result<Value> {
    let result = rpc.call(method, params.clone())?;
    e.push(RpcEvidence {
        id: e.len(),
        method: method.into(),
        params,
        result: result.clone(),
    });
    Ok(result)
}

/// Generic captured transfer inputs. Historical snapshots are adapted by wrappers below.
pub struct TransferContext {
    pub genesis_hash: String,
    pub minimum_slot: u64,
    pub mint: String,
    pub program: String,
    pub decimals: u8,
    pub source: String,
    pub owner: String,
    pub destination: String,
    pub destination_owner: Option<String>,
}
pub const CURRENT_REVISION: &str = "spl-transfer-checked-current-v1";
fn historical_context(
    s: &LifecycleSnapshot,
    entity: &str,
    destination: &str,
) -> Result<TransferContext> {
    let source = s
        .entities
        .iter()
        .find(|e| e.id == entity)
        .context("source absent")?;
    ensure!(
        source.entity_type == crate::lifecycle::EntityType::WalletCompatible,
        "unsupported source authority model"
    );
    let receiver = s
        .entities
        .iter()
        .find(|e| e.token_account == destination)
        .context("recipient is not an actual population token account")?;
    Ok(TransferContext {
        genesis_hash: s.source.genesis_hash.clone(),
        minimum_slot: s.source.max_observed_slot,
        mint: s.asset.mint.clone(),
        program: TOKEN_2022_PROGRAM.into(),
        decimals: s.mint_config.decimals,
        source: source.token_account.clone(),
        owner: source.state.owner.clone(),
        destination: destination.into(),
        destination_owner: Some(receiver.state.owner.clone()),
    })
}
pub fn capture(
    s: &LifecycleSnapshot,
    entity: &str,
    destination: &str,
    rpc: &impl SolanaRpc,
) -> Result<CapturedExecutionFixture> {
    capture_context(&historical_context(s, entity, destination)?, REVISION, rpc)
}
pub fn build(
    s: &LifecycleSnapshot,
    entity: &str,
    destination: &str,
    amount: u64,
    f: &CapturedExecutionFixture,
) -> Result<ProbeExecutionPlan> {
    build_context(
        &historical_context(s, entity, destination)?,
        amount,
        f,
        REVISION,
    )
}
pub fn capture_current(
    c: &TransferContext,
    rpc: &impl SolanaRpc,
) -> Result<CapturedExecutionFixture> {
    capture_context(c, CURRENT_REVISION, rpc)
}
fn capture_context(
    c: &TransferContext,
    revision: &str,
    rpc: &impl SolanaRpc,
) -> Result<CapturedExecutionFixture> {
    let mut e = Vec::new();
    let genesis = record(rpc, &mut e, "getGenesisHash", json!([]))?;
    ensure!(
        genesis.as_str() == Some(&c.genesis_hash),
        "wrong transfer capture chain"
    );
    let mint = record(
        rpc,
        &mut e,
        "getAccountInfo",
        json!([c.mint, config(c.minimum_slot)]),
    )?;
    let headers = record(
        rpc,
        &mut e,
        "getMultipleAccounts",
        json!([[c.program], config(shared::slot(&mint)?)]),
    )?;
    let pd = shared::programdata_address(&headers["value"][0])?;
    let mut addresses: BTreeSet<String> = [
        c.source.clone(),
        c.owner.clone(),
        c.destination.clone(),
        c.mint.clone(),
        CLOCK.into(),
        c.program.clone(),
    ]
    .into();
    if let Some(pd) = pd {
        addresses.insert(pd);
    }
    record(
        rpc,
        &mut e,
        "getMultipleAccounts",
        json!([
            addresses.into_iter().collect::<Vec<_>>(),
            config(shared::slot(&headers)?)
        ]),
    )?;
    Ok(CapturedExecutionFixture {
        schema_version: 1,
        decoder_revision: revision.into(),
        captured_at: Utc::now(),
        rpc_origin: "configured_read_only_provider".to_string(),
        evidence: e,
    })
}
pub fn build_current(
    c: &TransferContext,
    amount: u64,
    f: &CapturedExecutionFixture,
) -> Result<ProbeExecutionPlan> {
    build_context(c, amount, f, CURRENT_REVISION)
}
fn build_context(
    c: &TransferContext,
    amount: u64,
    f: &CapturedExecutionFixture,
    revision: &str,
) -> Result<ProbeExecutionPlan> {
    ensure!(
        f.schema_version == 1 && f.decoder_revision == revision && f.evidence.len() == 4,
        "unsupported transfer fixture schema/transcript"
    );
    ensure!(
        c.source != c.destination && amount > 0,
        "distinct recipient and positive amount required"
    );
    ensure!(
        [decode::LEGACY_PROGRAM, TOKEN_2022_PROGRAM].contains(&c.program.as_str()),
        "unsupported token program"
    );
    let e = &f.evidence;
    for (i, r) in e.iter().enumerate() {
        ensure!(r.id == i, "bad RPC record IDs");
    }
    ensure!(
        e[0].method == "getGenesisHash"
            && e[0].params == json!([])
            && e[0].result.as_str() == Some(&c.genesis_hash),
        "wrong transfer chain"
    );
    ensure!(
        e[1].method == "getAccountInfo" && e[1].params == json!([c.mint, config(c.minimum_slot)]),
        "invalid mint capture request"
    );
    let mint_slot = shared::slot(&e[1].result)?;
    ensure!(mint_slot >= c.minimum_slot, "old transfer mint");
    ensure!(
        e[2].method == "getMultipleAccounts"
            && e[2].params == json!([[c.program], config(mint_slot)]),
        "invalid token header request"
    );
    let pd = shared::programdata_address(&e[2].result["value"][0])?;
    let header_slot = shared::slot(&e[2].result)?;
    ensure!(header_slot >= mint_slot, "old header context");
    let mut address_set: BTreeSet<String> = [
        c.source.clone(),
        c.owner.clone(),
        c.destination.clone(),
        c.mint.clone(),
        CLOCK.into(),
        c.program.clone(),
    ]
    .into();
    if let Some(pd) = &pd {
        address_set.insert(pd.clone());
    }
    let addresses: Vec<_> = address_set.into_iter().collect();
    ensure!(
        e[3].method == "getMultipleAccounts"
            && e[3].params == json!([addresses, config(header_slot)]),
        "invalid final transfer account batch"
    );
    let values = e[3].result["value"]
        .as_array()
        .context("missing final accounts")?;
    ensure!(values.len() == addresses.len(), "missing final accounts");
    let slot = shared::slot(&e[3].result)?;
    ensure!(slot >= header_slot, "old final transfer context");
    let raw: BTreeMap<_, _> = addresses
        .iter()
        .zip(values)
        .map(|(a, v)| (a.as_str(), v))
        .collect();
    let get = |a: &str| -> Result<&Value> {
        let r = *raw.get(a).context("missing transfer account")?;
        ensure!(!r.is_null(), "missing transfer account {a}");
        Ok(r)
    };
    let clock = shared::clock(get(CLOCK)?)?;
    ensure!(clock.slot == slot, "Clock mismatch");
    let mint = decode::decode_mint(get(&c.mint)?)?;
    ensure!(
        mint.token_program == c.program && mint.decimals == c.decimals,
        "wrong mint program"
    );
    ensure!(
        !mint
            .extensions
            .iter()
            .any(|x| x.extension_type == "Pausable" && x.config["paused"] == true),
        "mint paused"
    );
    ensure!(
        !mint
            .extensions
            .iter()
            .any(|x| x.extension_type == "TransferHook" && !x.config["programId"].is_null()),
        "active hook requires unsupported extra-account resolution"
    );
    let from = decode::decode_token_account(get(&c.source)?, &c.program, &c.mint, mint.decimals)?;
    let to =
        decode::decode_token_account(get(&c.destination)?, &c.program, &c.mint, mint.decimals)?;
    ensure!(
        from.owner == c.owner
            && c.destination_owner
                .as_ref()
                .is_none_or(|owner| &to.owner == owner),
        "wrong/changed source or destination authority"
    );
    ensure!(
        from.is_initialized && to.is_initialized && !from.is_frozen && !to.is_frozen,
        "uninitialized/frozen transfer account"
    );
    ensure!(
        ![&from, &to].iter().any(|a| a
            .extensions
            .iter()
            .any(|x| x.extension_type == "ConfidentialTransferAccount")),
        "confidential transfer account unsupported"
    );
    ensure!(
        !to.extensions
            .iter()
            .any(|x| x.extension_type == "MemoTransfer"
                && x.config["requireIncomingTransferMemos"] == true),
        "incoming memo requires an additional captured memo path"
    );
    ensure!(
        from.raw_balance.parse::<u64>()? >= amount,
        "insufficient input token balance"
    );
    let owner = get(&c.owner)?;
    let authority: Address = c.owner.parse()?;
    ensure!(
        authority.is_on_curve()
            && owner["owner"] == "11111111111111111111111111111111"
            && owner["executable"] == false
            && decode::raw_account_bytes(owner)?.is_empty(),
        "captured source authority not wallet-compatible"
    );
    let header = get(&c.program)?;
    ensure!(
        shared::programdata_address(header)? == pd,
        "program loader link changed"
    );
    let bytes = if let Some(pd) = pd {
        ensure!(
            Address::find_program_address(
                &[c.program.parse::<Address>()?.as_ref()],
                &UPGRADEABLE_LOADER.parse()?
            )
            .0
            .to_string()
                == pd,
            "noncanonical ProgramData"
        );
        let state = get(&pd)?;
        ensure!(
            state["owner"] == UPGRADEABLE_LOADER && state["executable"] == false,
            "wrong ProgramData owner/state"
        );
        let b = decode::raw_account_bytes(state)?;
        let programdata = crate::standard_programs::upgradeable_loader::decode_programdata(&b)?;
        ensure!(
            !programdata.bytes.is_empty() && programdata.deploy_slot < slot,
            "malformed/new ProgramData"
        );
        programdata.bytes
    } else {
        decode::raw_account_bytes(header)?
    };
    ensure!(bytes.starts_with(b"\x7fELF"), "no deployed ELF");
    let programs = vec![LoadedProgram {
        program_id: c.program.parse()?,
        loader: header["owner"].as_str().context("loader absent")?.parse()?,
        bytes,
    }];
    let mut accounts = Vec::new();
    let mut proofs = Vec::new();
    for (i, a) in addresses.iter().enumerate() {
        let r = get(a)?;
        accounts.push(shared::captured_account(a, r)?);
        proofs.push(ExecutionAccountEvidence {
            address: a.clone(),
            rpc_record: 3,
            pointer: format!("/value/{i}"),
            slot,
            exists: true,
            runtime_owner: r["owner"].as_str().map(str::to_string),
            raw_data_sha256: Some(sha256(&decode::raw_account_bytes(r)?)),
        });
    }
    ensure!(
        !addresses.contains(&shared::payer().to_string()),
        "payer collision"
    );
    accounts.push(NamedAccount {
        label: "local-fee-payer".into(),
        address: shared::payer().to_string(),
        account: crate::types::AccountSnapshot {
            lamports: 1_000_000_000,
            owner: "11111111111111111111111111111111".into(),
            data: vec![],
            executable: false,
            rent_epoch: 0,
        },
    });
    let budget = Instruction {
        program_id: "ComputeBudget111111111111111111111111111111".parse()?,
        accounts: vec![],
        data: [vec![2], 1_400_000u32.to_le_bytes().to_vec()].concat(),
    };
    let ix = spl_token_2022_interface::instruction::transfer_checked(
        &c.program.parse()?,
        &c.source.parse()?,
        &c.mint.parse()?,
        &c.destination.parse()?,
        &authority,
        &[],
        amount,
        mint.decimals,
    )?;
    Ok(ProbeExecutionPlan{
accounts,
watch:vec![c.source.clone(),
c.destination.clone(),
c.mint.clone()],
programs,
message:Message::new(&[budget,
ix],
Some(&shared::payer())),
clock,
preconditions:vec![ProbePrecondition{
name:"Coherent final transfer batch, loader/ELF/Clock, actual compatible accounts and public transfer authority".into(),
proven:true,
reason:"Verified raw RPC bytes, canonical token loader links, mint/owner/initialized/freeze/pause/hook/confidential/memo states and sufficient input".into()}
],
account_evidence:proofs,
assumptions:vec!["Original owner locally assumed to sign; possession and authorization are unknown. Signature and recent-blockhash verification are disabled locally.".into(),
"Actual deployed Token-2022 executes TransferChecked with captured epoch fees. Permanent delegate and active delegate remain unused; raw quantities exclude scaled display transforms.".into(),
"Fresh existing LiteSVM 0.16 backend with pinned mainnet/default runtime profile and synthetic fee payer; not a full validator bank. No token account or token balance is fabricated.".into(),
"Transfer is movement to a different actual observed recipient token account. No market sale, redemption, withdrawal, entitlement, or official successor transition is implied.".into()]}
)
}
fn amount(b: &[u8]) -> Result<(u64, u64)> {
    crate::standard_programs::token::account_amounts(TOKEN_2022_PROGRAM, b)
}
pub fn reconcile(
    s: &LifecycleSnapshot,
    entity: &str,
    destination: &str,
    input: u64,
    p: &ProbeExecutionPlan,
    x: &ProbeTransactionExecution,
) -> Result<ExecutionDeltas> {
    let source = s
        .entities
        .iter()
        .find(|e| e.id == entity)
        .context("source absent")?;
    reconcile_current(
        &s.asset.mint,
        &source.token_account,
        destination,
        s.mint_config.decimals,
        input,
        p,
        x,
    )
}
pub fn reconcile_current(
    mint: &str,
    source: &str,
    destination: &str,
    decimals: u8,
    input: u64,
    p: &ProbeExecutionPlan,
    x: &ProbeTransactionExecution,
) -> Result<ExecutionDeltas> {
    let before: BTreeMap<_, _> = p
        .accounts
        .iter()
        .map(|a| (a.address.as_str(), &a.account))
        .collect();
    let mut tokens = Vec::new();
    let mut changes = Vec::new();
    for address in [source, destination, mint] {
        let a = before[address];
        let b = x
            .post_accounts
            .get(address)
            .context("missing watched post-state")?;
        changes.push(AccountDataDelta {
            address: address.into(),
            before_sha256: sha256(&a.data),
            after_sha256: sha256(&b.data),
            changed_ranges: shared::data_changes(address, &a.data, &b.data),
        });
        if address == mint {
            continue;
        }
        let (n, f) = amount(&a.data)?;
        let (m, g) = amount(&b.data)?;
        let delta = i128::from(m) - i128::from(n);
        tokens.push(TokenAccountDelta {
            address: address.into(),
            mint: mint.to_owned(),
            before_raw: n.to_string(),
            after_raw: m.to_string(),
            change_raw: delta.to_string(),
            change_decimal_base_units: if delta < 0 {
                format!("-{}", decode::decimal_amount((-delta) as u64, decimals))
            } else {
                decode::decimal_amount(delta as u64, decimals)
            },
            withheld_fee_change_raw: (i128::from(g) - i128::from(f)).to_string(),
        });
    }
    let debit = tokens[0]
        .before_raw
        .parse::<u64>()?
        .checked_sub(tokens[0].after_raw.parse()?)
        .context("source increased")?;
    let credit = tokens[1]
        .after_raw
        .parse::<u64>()?
        .checked_sub(tokens[1].before_raw.parse()?)
        .context("destination decreased")?;
    let fee = shared::transfer_fee(&before[mint].data, p.clock.epoch, input)?;
    let rollback = p
        .watch
        .iter()
        .all(|a| x.post_accounts.get(a) == before.get(a.as_str()).copied());
    let reconciled = if x.success {
        debit == input
            && credit == input - fee
            && tokens[0].withheld_fee_change_raw == "0"
            && tokens[1].withheld_fee_change_raw == fee.to_string()
            && x.post_accounts[mint] == *before[mint]
            && (x
                .logs
                .iter()
                .any(|l| l.contains("Instruction: TransferChecked"))
                || (before[mint].owner == decode::LEGACY_PROGRAM
                    && p.message.instructions.last().is_some_and(|ix| {
                        p.message.account_keys[ix.program_id_index as usize].to_string()
                            == decode::LEGACY_PROGRAM
                            && ix.data
                                == [vec![12], input.to_le_bytes().to_vec(), vec![decimals]].concat()
                    })))
    } else {
        debit == 0 && credit == 0 && rollback
    };
    Ok(ExecutionDeltas{
input_debited_raw:debit.to_string(),
output_received_raw:credit.to_string(),
output_decimal_base_units:decode::decimal_amount(credit,
decimals),
token_accounts:tokens,
account_data:changes,
fees:None,
reconciled,
reconciliation:vec![format!("Actual Token-2022 transfer: source debit = destination public credit + destination withheld fee. Captured epoch {}; independently calculated transfer fee {} raw. Mint unchanged; no DLMM event/bin/venue delta is applicable to Transfer.",
p.clock.epoch,
fee),
format!("Watched rollback on failed execution: {rollback}; transaction fees are paid separately by the synthetic payer.")]}
)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use spl_token_2022_interface::{extension::transfer_fee::TransferFeeAmount, state::Account};
    use std::sync::OnceLock;
    struct Corpus {
        s: LifecycleSnapshot,
        entity: String,
        destination: String,
        fixture: CapturedExecutionFixture,
    }
    fn corpus() -> &'static Corpus {
        static C: OnceLock<Corpus> = OnceLock::new();
        C.get_or_init(|| {
            let root = crate::lifecycle::artifact::reference_root();
            let plan: crate::lifecycle::frozen::selection::ExpansionPlan =
                crate::lifecycle::frozen::selection::load(
                    &root.join("probes/spacex-lifecycle-expansion-plan.json"),
                )
                .unwrap();
            let group = plan
                .selected
                .iter()
                .find(|g| g.candidate.path_type == ExitPathType::Transfer)
                .unwrap();
            Corpus {
                s: LifecycleSnapshot::load(&root.join("snapshots/spacex-exposure.json")).unwrap(),
                entity: group.candidate.entity_id.clone(),
                destination: group
                    .candidate
                    .context_id
                    .trim_start_matches("token-transfer:")
                    .into(),
                fixture: crate::lifecycle::frozen::selection::load(
                    &root
                        .join("probes/phase7-captures")
                        .join(&group.fixture_reference),
                )
                .unwrap(),
            }
        })
    }
    fn raw_mut(f: &mut CapturedExecutionFixture, address: &str, edit: impl FnOnce(&mut Vec<u8>)) {
        let e = &mut f.evidence[3];
        let n = e.params[0]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v == address)
            .unwrap();
        let raw = &mut e.result["value"][n];
        let engine = base64::engine::general_purpose::STANDARD;
        let mut bytes = engine.decode(raw["data"][0].as_str().unwrap()).unwrap();
        edit(&mut bytes);
        raw["data"][0] = engine.encode(bytes).into();
    }
    fn successful() -> (
        ProbeExecutionPlan,
        ProbeTransactionExecution,
        ExecutionDeltas,
    ) {
        let c = corpus();
        let p = build(&c.s, &c.entity, &c.destination, 100, &c.fixture).unwrap();
        let x = crate::executor::execute_probe_message(
            &p.accounts,
            &p.watch,
            p.clock.clone(),
            &p.programs,
            p.message.clone(),
        )
        .unwrap();
        let d = reconcile(&c.s, &c.entity, &c.destination, 100, &p, &x).unwrap();
        (p, x, d)
    }
    #[test]
    fn deployed_token_2022_transfer_executes_and_fees_reconcile() {
        let (p, x, d) = successful();
        assert!(x.success);
        assert!(d.reconciled);
        assert_eq!(d.input_debited_raw, "100");
        assert_eq!(d.output_received_raw, "99");
        assert_eq!(d.token_accounts[1].withheld_fee_change_raw, "1");
        assert!(x.compute_units > 0);
        assert!(x
            .logs
            .iter()
            .any(|l| l.contains("Instruction: TransferChecked")));
        assert!(p.assumptions.iter().any(|a| a.contains("locally assumed")));
    }
    #[test]
    fn token_transfer_replay_is_identical() {
        let (p, x, d) = successful();
        let again = crate::executor::execute_probe_message(
            &p.accounts,
            &p.watch,
            p.clock.clone(),
            &p.programs,
            p.message.clone(),
        )
        .unwrap();
        assert_eq!(x, again);
        let c = corpus();
        assert_eq!(
            d,
            reconcile(&c.s, &c.entity, &c.destination, 100, &p, &again).unwrap()
        );
    }
    #[test]
    fn insufficient_input_is_a_precondition_rejection() {
        let c = corpus();
        let source = c.s.entities.iter().find(|e| e.id == c.entity).unwrap();
        let e = &c.fixture.evidence[3];
        let pos = e.params[0]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v == &source.token_account)
            .unwrap();
        let a = decode::decode_token_account(
            &e.result["value"][pos],
            TOKEN_2022_PROGRAM,
            &c.s.asset.mint,
            c.s.mint_config.decimals,
        )
        .unwrap();
        assert!(build(
            &c.s,
            &c.entity,
            &c.destination,
            a.raw_balance.parse::<u64>().unwrap() + 1,
            &c.fixture
        )
        .err()
        .unwrap()
        .to_string()
        .contains("insufficient"));
    }
    #[test]
    fn wrong_mint_authority_and_frozen_source_are_rejected() {
        let c = corpus();
        let source = c.s.entities.iter().find(|e| e.id == c.entity).unwrap();
        for variant in 0..3 {
            let mut f = c.fixture.clone();
            raw_mut(&mut f, &source.token_account, |b| match variant {
                0 => b[..32].fill(0),
                1 => b[32..64].fill(0),
                _ => b[108] = 2,
            });
            assert!(build(&c.s, &c.entity, &c.destination, 100, &f).is_err());
        }
    }
    #[test]
    fn paused_mint_and_active_hook_without_extra_accounts_are_rejected() {
        use spl_token_2022_interface::{
            extension::{
                pausable::PausableConfig, transfer_hook::TransferHook, BaseStateWithExtensionsMut,
                StateWithExtensionsMut,
            },
            state::Mint,
        };
        let c = corpus();
        for hook in [false, true] {
            let mut f = c.fixture.clone();
            raw_mut(&mut f, &c.s.asset.mint, |b| {
                let mut m = StateWithExtensionsMut::<Mint>::unpack(b).unwrap();
                if hook {
                    m.get_extension_mut::<TransferHook>().unwrap().program_id =
                        Some(Address::new_from_array([5; 32])).try_into().unwrap();
                } else {
                    m.get_extension_mut::<PausableConfig>().unwrap().paused = true.into();
                }
            });
            let error = build(&c.s, &c.entity, &c.destination, 100, &f)
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains(if hook { "active hook" } else { "paused" }),
                "{error}"
            );
        }
    }
    #[test]
    fn real_program_failure_rolls_back_watched_tokens_and_mint() {
        let c = corpus();
        let mut p = build(&c.s, &c.entity, &c.destination, 100, &c.fixture).unwrap();
        *p.message
            .instructions
            .last_mut()
            .unwrap()
            .data
            .last_mut()
            .unwrap() = c.s.mint_config.decimals + 1;
        let x = crate::executor::execute_probe_message(
            &p.accounts,
            &p.watch,
            p.clock.clone(),
            &p.programs,
            p.message.clone(),
        )
        .unwrap();
        assert!(!x.success);
        let d = reconcile(&c.s, &c.entity, &c.destination, 100, &p, &x).unwrap();
        assert!(d.reconciled);
        assert_eq!(d.input_debited_raw, "0");
        assert_eq!(d.output_received_raw, "0");
        assert!(p.watch.iter().all(|a| x.post_accounts.get(a)
            == p.accounts
                .iter()
                .find(|b| b.address == *a)
                .map(|b| &b.account)));
    }
    #[test]
    fn destination_must_be_real_distinct_and_same_mint() {
        let c = corpus();
        let source = c.s.entities.iter().find(|e| e.id == c.entity).unwrap();
        assert!(build(&c.s, &c.entity, &source.token_account, 100, &c.fixture).is_err());
        assert!(build(
            &c.s,
            &c.entity,
            "11111111111111111111111111111111",
            100,
            &c.fixture
        )
        .is_err());
        let mut f = c.fixture.clone();
        raw_mut(&mut f, &c.destination, |b| b[..32].fill(0));
        assert!(build(&c.s, &c.entity, &c.destination, 100, &f).is_err());
    }
    #[test]
    fn missing_withheld_fee_fails_exact_reconciliation() {
        use spl_token_2022_interface::extension::{
            BaseStateWithExtensionsMut, StateWithExtensionsMut,
        };
        let (p, mut x, _) = successful();
        let c = corpus();
        let destination = x.post_accounts.get_mut(&c.destination).unwrap();
        let mut state = StateWithExtensionsMut::<Account>::unpack(&mut destination.data).unwrap();
        state
            .get_extension_mut::<TransferFeeAmount>()
            .unwrap()
            .withheld_amount = 0u64.into();
        let delta = reconcile(&c.s, &c.entity, &c.destination, 100, &p, &x).unwrap();
        assert!(
            !delta.reconciled,
            "missing_destination_withheld_fee_must_reject_proof"
        );
    }
}
