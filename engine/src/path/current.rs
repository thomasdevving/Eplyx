//! Current execution evidence is constructed only by rebuilding and executing captured inputs.
//! Discovery remains immutable. No serialized execution classification is accepted as proof.
use super::{token_transfer, CapturedExecutionFixture, ExitPathType, ProbeClock, ProbeMessage};
use crate::{
    executor,
    lifecycle::resolution::PathStatus,
    lifecycle::{current as observation, decode, exposure::sha256, rpc::SolanaRpc},
};
use anyhow::{ensure, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
use std::{io::Write, path::Path, sync::Mutex};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRequest {
    pub path: ExitPathType,
    pub source: String,
    pub amount_mode: AmountMode,
    pub amount_decimal: Option<String>,
    pub recipient: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_output_decimal: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AmountMode {
    Full,
    Custom,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub schema_version: u32,
    pub run_id: String,
    pub check_id: String,
    pub wallet_capture: String,
    pub wallet_capture_sha256: String,
    pub request: CheckRequest,
    pub started_at: String,
    pub completed_at: String,
    pub rpc_origin: String,
    pub observations: Vec<observation::Observation>,
}
/// Deliberately not Deserialize: verified evidence must go through replay + reconciliation.
#[derive(Serialize)]
pub struct VerifiedExecution {
    result: Value,
}
impl VerifiedExecution {
    pub fn value(&self) -> &Value {
        &self.result
    }
}
fn now() -> String {
    Utc::now().to_rfc3339()
}
pub fn exact_amount(text: &str, decimals: u8) -> Result<u64> {
    ensure!(
        !text.is_empty() && text.len() <= 280,
        "invalid decimal amount"
    );
    let mut parts = text.split('.');
    let whole = parts.next().unwrap();
    let fraction = parts.next();
    ensure!(
        !whole.is_empty() && whole.bytes().all(|b| b.is_ascii_digit()) && parts.next().is_none(),
        "invalid decimal amount"
    );
    let fraction = match fraction {
        Some(f) => {
            ensure!(!f.is_empty(), "invalid decimal amount");
            f
        }
        None => "",
    };
    ensure!(
        fraction.len() <= decimals as usize && fraction.bytes().all(|b| b.is_ascii_digit()),
        "excess precision or invalid decimal amount"
    );
    let digits = format!(
        "{whole}{fraction}{}",
        "0".repeat(decimals as usize - fraction.len())
    );
    let amount = digits
        .parse::<u64>()
        .context("amount exceeds raw integer range")?;
    ensure!(amount > 0, "amount must be positive");
    Ok(amount)
}
struct Scope {
    context: token_transfer::TransferContext,
    amount: u64,
    source_raw: Value,
    unsupported: Option<String>,
}
fn unsupported(
    mint: &decode::MintConfig,
    source: &decode::TokenAccountState,
    recipient: Option<&decode::TokenAccountState>,
) -> Option<String> {
    if !source.is_initialized
        || source.is_frozen
        || recipient.is_some_and(|a| !a.is_initialized || a.is_frozen)
    {
        return Some("Uninitialized or frozen token account".into());
    }
    for e in &mint.extensions {
        if (e.extension_type == "Pausable" && e.config["paused"] == true)
            || (e.extension_type == "TransferHook" && !e.config["programId"].is_null())
            || ["NonTransferable"].contains(&e.extension_type.as_str())
        {
            return Some(format!(
                "Unsupported current transfer configuration: {}",
                e.extension_type
            ));
        }
    }
    for account in std::iter::once(source).chain(recipient) {
        if account
            .extensions
            .iter()
            .any(|e| e.extension_type.contains("Confidential"))
        {
            return Some("Confidential accounts are unsupported".into());
        }
    }
    if recipient.is_some_and(|a| {
        a.extensions.iter().any(|e| {
            e.extension_type == "MemoTransfer" && e.config["requireIncomingTransferMemos"] == true
        })
    }) {
        return Some("Recipient requires an unsupported incoming memo path".into());
    }
    None
}
fn scope(wallet: &str, request: &CheckRequest) -> Result<Scope> {
    ensure!(
        wallet.len() <= 10 * 1024 * 1024,
        "wallet capture exceeds budget"
    );
    let c: observation::Capture = serde_json::from_str(wallet)?;
    let result = observation::evaluate(&c)?;
    ensure!(
        c.schema_version == 3
            && matches!(
                request.path,
                ExitPathType::Transfer | ExitPathType::SecondaryMarketExit
            ),
        "unsupported check or observation scope"
    );
    let _: Address = request.source.parse().context("invalid source address")?;
    if request.path == ExitPathType::Transfer {
        let _: Address = request
            .recipient
            .parse()
            .context("invalid recipient token account")?;
        ensure!(
            request.source != request.recipient
                && request.output_mint.is_none()
                && request.minimum_output_decimal.is_none(),
            "distinct transfer recipient and no market parameters required"
        );
    } else {
        ensure!(
            request.recipient.is_empty(),
            "market destination is the owner's derived paired-token ATA"
        );
        let output = request
            .output_mint
            .as_ref()
            .context("paired mint required")?;
        let _: Address = output.parse().context("invalid paired mint")?;
        ensure!(output != &c.asset.mint, "paired mint must differ");
        let minimum = request
            .minimum_output_decimal
            .as_deref()
            .context("explicit minimum output required")?;
        ensure!(
            minimum.len() <= 280
                && minimum.bytes().any(|b| matches!(b, b'1'..=b'9'))
                && minimum.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                && minimum.matches('.').count() <= 1,
            "invalid minimum output"
        );
    }
    let account = result["wallet_observation"]["token_accounts"]
        .as_array()
        .context("wallet accounts unavailable")?
        .iter()
        .find(|a| a["address"] == request.source)
        .context("focused account was not discovered in this wallet run")?;
    let state: decode::TokenAccountState = serde_json::from_value(account["state"].clone())?;
    let mint: decode::MintConfig = serde_json::from_value(result["mint"].clone())?;
    let owner = c
        .selection
        .as_ref()
        .and_then(|s| s.public_owner.as_ref())
        .context("missing owner")?;
    ensure!(
        state.owner == *owner && state.mint == c.asset.mint,
        "focused account identity mismatch"
    );
    let balance = state.raw_balance.parse::<u64>()?;
    let amount = match request.amount_mode {
        AmountMode::Full => {
            ensure!(
                request.amount_decimal.is_none(),
                "full balance cannot include custom amount"
            );
            balance
        }
        AmountMode::Custom => exact_amount(
            request
                .amount_decimal
                .as_deref()
                .context("custom amount required")?,
            mint.decimals,
        )?,
    };
    ensure!(
        amount > 0 && amount <= balance,
        "amount must be positive and no greater than the observed public balance"
    );
    let raw = c.observations[3]
        .result
        .as_ref()
        .context("wallet lookup missing")?["value"]
        .as_array()
        .context("missing accounts")?
        .iter()
        .find(|a| a["pubkey"] == request.source)
        .context("source raw bytes missing")?["account"]
        .clone();
    let mut boundary = unsupported(&mint, &state, None);
    if request.path == ExitPathType::SecondaryMarketExit
        && mint.token_program != decode::TOKEN_2022_PROGRAM
    {
        boundary =
            Some("Current market adapter requires Token-2022 input and legacy paired asset".into());
    }
    let authority: Address = owner.parse()?;
    let owner_raw = &c.observations[2]
        .result
        .as_ref()
        .context("owner inspection missing")?["value"];
    if !authority.is_on_curve()
        || owner_raw.is_null()
        || owner_raw["owner"] != "11111111111111111111111111111111"
        || owner_raw["executable"] != false
        || !decode::raw_account_bytes(owner_raw)?.is_empty()
    {
        boundary = Some("Recorded authority is not a supported directly signing wallet".into());
    }
    Ok(Scope {
        context: token_transfer::TransferContext {
            genesis_hash: result["acquisition"]["genesis_hash"]
                .as_str()
                .context("missing genesis")?
                .into(),
            minimum_slot: account["slot"]
                .as_str()
                .context("missing discovery slot")?
                .parse::<u64>()?,
            mint: c.asset.mint,
            program: mint.token_program,
            decimals: mint.decimals,
            source: request.source.clone(),
            owner: owner.clone(),
            destination: request.recipient.clone(),
            destination_owner: None,
        },
        amount,
        source_raw: raw,
        unsupported: boundary,
    })
}
pub fn validate(wallet: &str, request: &CheckRequest) -> Result<Value> {
    let s = scope(wallet, request)?;
    Ok(
        json!({"amount_raw":s.amount.to_string(),"amount_decimal":decode::decimal_amount(s.amount,s.context.decimals),"status":if s.unsupported.is_some(){PathStatus::Unsupported}else{PathStatus::NotTested},"reason":s.unsupported}),
    )
}
/// Availability is derived from verified wallet bytes; it is never execution proof.
pub fn capabilities(wallet: &str) -> Result<Value> {
    let c: observation::Capture = serde_json::from_str(wallet)?;
    let r = observation::evaluate(&c)?;
    ensure!(c.schema_version == 3, "wallet observation required");
    let mint: decode::MintConfig = serde_json::from_value(r["mint"].clone())?;
    let mut accounts = vec![];
    for a in r["wallet_observation"]["token_accounts"]
        .as_array()
        .context("wallet accounts unavailable")?
    {
        let state: decode::TokenAccountState = serde_json::from_value(a["state"].clone())?;
        let raw = &c.observations[2]
            .result
            .as_ref()
            .context("owner observation missing")?["value"];
        let owner: Address = state.owner.parse()?;
        let reason = unsupported(&mint, &state, None).or_else(|| {
            if state.raw_balance == "0" {
                Some("No positive public balance".into())
            } else if !owner.is_on_curve()
                || raw["owner"] != "11111111111111111111111111111111"
                || raw["executable"] != false
                || decode::raw_account_bytes(raw).map_or(true, |b| !b.is_empty())
            {
                Some("Recorded authority is not a supported directly signing wallet".into())
            } else {
                None
            }
        });
        let market_reason = reason.clone().or_else(|| {
            (mint.token_program != decode::TOKEN_2022_PROGRAM)
                .then(|| "Current market adapter requires Token-2022 input".into())
        });
        accounts.push(json!({"source":a["address"],"checks":[{"path":ExitPathType::Transfer,"status":if reason.is_some(){PathStatus::Unsupported}else{PathStatus::NotTested},"available":reason.is_none(),"reason":reason},{"path":ExitPathType::SecondaryMarketExit,"status":if market_reason.is_some(){PathStatus::Unsupported}else{PathStatus::NotTested},"available":market_reason.is_none(),"reason":market_reason}]}));
    }
    Ok(
        json!({"wallet_capture_sha256":sha256(wallet.as_bytes()),"accounts":accounts,"execution_performed":false}),
    )
}
struct Recorder<'a, R> {
    rpc: &'a R,
    observations: Mutex<Vec<observation::Observation>>,
}
impl<R: SolanaRpc> SolanaRpc for Recorder<'_, R> {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let start = now();
        let value = self.rpc.call(method, params.clone());
        self.observations
            .lock()
            .unwrap()
            .push(observation::Observation {
                method: method.into(),
                params,
                started_at: start,
                completed_at: now(),
                result: value.as_ref().ok().cloned(),
                error: value.as_ref().err().map(ToString::to_string),
            });
        value
    }
}
pub fn capture(
    wallet: String,
    request: CheckRequest,
    run_id: String,
    check_id: String,
    rpc: &impl SolanaRpc,
) -> Result<Capture> {
    let s = scope(&wallet, &request)?;
    let recorder = Recorder {
        rpc,
        observations: Mutex::new(vec![]),
    };
    let started_at = now();
    if s.unsupported.is_none() {
        eprintln!("CURRENT_STAGE:Preparing current state");
        // Failed acquisition is retained as a non-executed check, never converted to Failed.
        if request.path == ExitPathType::Transfer {
            let _ = token_transfer::capture_current(&s.context, &recorder);
        } else {
            let _ = super::current_market::prepare(&s.context, s.amount, &request, &recorder);
        }
    }
    Ok(Capture {
        schema_version: 1,
        run_id,
        check_id,
        wallet_capture_sha256: sha256(wallet.as_bytes()),
        wallet_capture: wallet,
        request,
        started_at,
        completed_at: now(),
        rpc_origin: "configured_read_only_provider".into(),
        observations: recorder.observations.into_inner().unwrap(),
    })
}
pub fn save(capture: &Capture, path: &Path) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec(capture)?)?;
    file.sync_all()?;
    Ok(())
}
pub fn replay(
    bytes: &[u8],
    run_id: &str,
    check_id: &str,
    wallet_hash: &str,
    capture_hash: &str,
) -> Result<VerifiedExecution> {
    ensure!(
        bytes.len() <= 64 * 1024 * 1024 && sha256(bytes) == capture_hash,
        "execution capture digest mismatch"
    );
    let c: Capture = serde_json::from_slice(bytes)?;
    ensure!(
        c.schema_version == 1
            && c.run_id == run_id
            && c.check_id == check_id
            && c.wallet_capture_sha256 == wallet_hash
            && sha256(c.wallet_capture.as_bytes()) == wallet_hash,
        "execution run or wallet binding mismatch"
    );
    let s = scope(&c.wallet_capture, &c.request)?;
    let start = chrono::DateTime::parse_from_rfc3339(&c.started_at)?;
    let end = chrono::DateTime::parse_from_rfc3339(&c.completed_at)?;
    ensure!(
        start <= end
            && c.observations.len()
                <= if c.request.path == ExitPathType::Transfer {
                    4
                } else {
                    7
                },
        "invalid execution acquisition interval or budget"
    );
    let mut previous = start;
    for r in &c.observations {
        let a = chrono::DateTime::parse_from_rfc3339(&r.started_at)?;
        let b = chrono::DateTime::parse_from_rfc3339(&r.completed_at)?;
        ensure!(
            previous <= a && a <= b && b <= end && r.result.is_some() != r.error.is_some(),
            "invalid acquisition record"
        );
        previous = b;
    }
    let mut result = json!({"schema_version":1,"kind":"current-execution","run_id":run_id,"check_id":check_id,"wallet_capture_sha256":wallet_hash,"execution_capture_sha256":capture_hash,"path":c.request.path,"request":c.request,"mint":s.context.mint,"source":s.context.source,"recipient":s.context.destination,"owner":s.context.owner,"amount_raw":s.amount.to_string(),"amount_decimal":decode::decimal_amount(s.amount,s.context.decimals),"status":PathStatus::Indeterminate,"execution_performed":false,"local_execution_performed":false,"funds_moved":false,"signer_assumed_locally":false,"signer_possession_known":false,"lifecycle_event":null,"readiness":null,"authorization":false,"acquisition":{"started_at":c.started_at,"completed_at":c.completed_at,"rpc_origin":c.rpc_origin,"commitment":"finalized","discovery_slot":s.context.minimum_slot.to_string(),"consistency":"Discovery and execution are separate observations. Execution uses one final account batch, not a full validator bank."},"scope":"Only this exact source, mint, amount, recipient, fixture, deployed code, runtime and assumed owner signature. No other account, amount, venue or lifecycle path inherits this evidence."});
    let fail =
        |mut result: Value, status: PathStatus, reason: String| -> Result<VerifiedExecution> {
            result["status"] = serde_json::to_value(status)?;
            result["reason"] = reason.into();
            Ok(VerifiedExecution { result })
        };
    if let Some(reason) = s.unsupported {
        return fail(result, PathStatus::Unsupported, reason);
    }
    let mut market = None;
    let fixture = if c.request.path == ExitPathType::SecondaryMarketExit {
        let rpc = ReplayRpc {
            records: &c.observations,
            next: Mutex::new(0),
        };
        let preparation = super::current_market::prepare(&s.context, s.amount, &c.request, &rpc);
        ensure!(
            *rpc.next.lock().unwrap() == c.observations.len(),
            "unused market capture observations"
        );
        match preparation {
            Err(e) => {
                result["failure_class"] = if e.to_string().starts_with("InvalidRequest:") {
                    "InvalidRequest"
                } else {
                    "PublicStateUnavailable"
                }
                .into();
                return fail(
                    result,
                    PathStatus::Indeterminate,
                    format!("Required current route state could not be established: {e}"),
                );
            }
            Ok(super::current_market::Preparation::NoRoute(discovery)) => {
                result["discovery"] = discovery;
                return fail(
                    result,
                    PathStatus::Unsupported,
                    "No supported market-exit route was verified in this scan.".into(),
                );
            }
            Ok(super::current_market::Preparation::Ready(mut prepared)) => {
                prepared.fixture.captured_at = end.with_timezone(&Utc);
                result["discovery"] = prepared.discovery.clone();
                result["market_parameters"] = serde_json::to_value(&prepared.parameters)?;
                result["recipient"] = Value::Null;
                let fixture = prepared.fixture.clone();
                market = Some(prepared);
                fixture
            }
        }
    } else {
        if c.observations.len() != 4 || c.observations.iter().any(|r| r.error.is_some()) {
            return fail(result, PathStatus::Indeterminate, "Required public execution state could not be captured within the bounded request budget.".into());
        }
        let fixture = CapturedExecutionFixture {
            schema_version: 1,
            decoder_revision: token_transfer::CURRENT_REVISION.into(),
            captured_at: end.with_timezone(&Utc),
            rpc_origin: c.rpc_origin,
            evidence: c
                .observations
                .iter()
                .enumerate()
                .map(|(i, r)| crate::lifecycle::RpcEvidence {
                    id: i,
                    method: r.method.clone(),
                    params: r.params.clone(),
                    result: r.result.clone().unwrap(),
                })
                .collect(),
        };
        fixture
    };
    let batch = &fixture.evidence[3];
    let raw = |address: &str| -> Result<&Value> {
        let i = batch.params[0]
            .as_array()
            .context("missing plan")?
            .iter()
            .position(|a| a == address)
            .context("missing account")?;
        Ok(&batch.result["value"][i])
    };
    let final_source = raw(&s.context.source)?;
    if ["data", "owner", "executable", "lamports"]
        .iter()
        .any(|field| final_source[field] != s.source_raw[field])
    {
        return fail(result, PathStatus::Indeterminate, "The account changed while the check was being prepared. Refresh current state and reconfirm the check.".into());
    }
    // Build validates the complete transcript, loader links and account identities before execution.
    let discovery_capture: observation::Capture = serde_json::from_str(&c.wallet_capture)?;
    let original_mint = &discovery_capture.observations[1]
        .result
        .as_ref()
        .context("missing original mint")?["value"];
    let original_owner = &discovery_capture.observations[2]
        .result
        .as_ref()
        .context("missing original owner")?["value"];
    result["discovery_comparison"] = json!({"source_unchanged":true,"mint_data_changed":raw(&s.context.mint)?["data"]!=original_mint["data"],"owner_account_changed":raw(&s.context.owner)?!=original_owner,"execution_fixture_is_authoritative":true});
    let built = match &market {
        Some(m) => super::meteora_dlmm::build_current(
            &s.context.genesis_hash,
            m.minimum_slot,
            &m.parameters,
            &s.context.source,
            &s.context.owner,
            &fixture,
        ),
        None => token_transfer::build_current(&s.context, s.amount, &fixture),
    };
    let plan = match built {
        Ok(p) => p,
        Err(e) => {
            return fail(
                result,
                if [
                    "mint paused",
                    "active hook requires unsupported extra-account resolution",
                    "uninitialized/frozen transfer account",
                    "confidential transfer account unsupported",
                    "incoming memo requires an additional captured memo path",
                    "captured source authority not wallet-compatible",
                ]
                .contains(&e.to_string().as_str())
                {
                    PathStatus::Unsupported
                } else {
                    PathStatus::Indeterminate
                },
                format!("Execution precondition could not be established: {e}"),
            )
        }
    };
    let final_mint = decode::decode_mint(raw(&s.context.mint)?)?;
    let source = decode::decode_token_account(
        final_source,
        &s.context.program,
        &s.context.mint,
        s.context.decimals,
    )?;
    if let Some(m) = &market {
        ensure!(
            final_mint.decimals == s.context.decimals,
            "input decimal basis changed"
        );
        let paired = decode::decode_mint(raw(&m.parameters.output_mint)?)?;
        ensure!(
            Some(u64::from(paired.decimals)) == m.discovery["output_decimals"].as_u64(),
            "minimum-output decimal basis changed"
        );
        let ix = plan
            .message
            .instructions
            .last()
            .context("missing market instruction")?;
        result["recipient"] = plan.message.account_keys[ix.accounts[5] as usize]
            .to_string()
            .into();
        if let Some(reason) = unsupported(&final_mint, &source, None) {
            return fail(result, PathStatus::Unsupported, reason);
        }
    } else {
        let recipient = decode::decode_token_account(
            raw(&s.context.destination)?,
            &s.context.program,
            &s.context.mint,
            s.context.decimals,
        )?;
        if let Some(reason) = unsupported(&final_mint, &source, Some(&recipient)) {
            return fail(result, PathStatus::Unsupported, reason);
        }
    }
    result["amount_basis"] = "Unscaled base token units using captured mint decimals; scaled UI and interest-bearing conversions are not requested or applied.".into();
    result["execution_fixture_sha256"] = fixture.sha256()?.into();
    result["clock"] = serde_json::to_value(ProbeClock::from(&plan.clock))?;
    result["message"] = serde_json::to_value(ProbeMessage::from(&plan.message))?;
    result["account_evidence"] = serde_json::to_value(&plan.account_evidence)?;
    result["deployed_programs"] = json!(plan.programs.iter().map(|p| -> Result<Value> {
        let id=p.program_id.to_string();let pd=super::meteora_dlmm::programdata_address(raw(&id)?)?;
        let deployment_slot=if let Some(address)=&pd {let b=decode::raw_account_bytes(raw(address)?)?;Some(crate::standard_programs::upgradeable_loader::decode_programdata(&b)?.deploy_slot.to_string())}else{None};
        Ok(json!({"program":id,"loader":p.loader.to_string(),"programdata_address":pd,"deployment_slot":deployment_slot,"code_sha256":sha256(&p.bytes)}))
    }).collect::<Result<Vec<_>>>()?);
    result["account_plan"] = fixture.evidence[3].params[0].clone();
    result["pre_accounts"] = serde_json::to_value(
        plan.accounts
            .iter()
            .filter(|a| plan.watch.contains(&a.address))
            .collect::<Vec<_>>(),
    )?;
    result["assumptions"] = json!(["The recorded owner is assumed locally to sign; key possession and authorization remain unknown.","Signature and recent-blockhash verification are disabled in the existing LiteSVM 0.16 runtime. Only the fee payer is synthetic; token accounts and deployed program bytes are captured.","This is token movement only. No market sale, conversion, redemption, withdrawal or readiness is established."]);
    result["runtime_profile"] = json!({"backend":"LiteSVM 0.16","features":"pinned mainnet/default profile","signature_verification":false,"recent_blockhash_verification":false,"synthetic_fee_payer":super::meteora_dlmm::payer().to_string(),"validator_bank_reproduction":false});
    if market.is_some() {
        result["assumptions"][2] = "Only the selected route and exact input/minimum-output condition are tested. Any missing destination ATA is created by the captured real ATA program, funded by the synthetic fee payer. No conversion, redemption, withdrawal, global exitability or readiness is established.".into();
    }
    eprintln!("CURRENT_STAGE:Running local simulation");
    let execution = executor::execute_probe_message(
        &plan.accounts,
        &plan.watch,
        plan.clock.clone(),
        &plan.programs,
        plan.message.clone(),
    )?;
    eprintln!("CURRENT_STAGE:Checking result");
    result["execution_performed"] = true.into();
    result["local_execution_performed"] = true.into();
    result["signer_assumed_locally"] = true.into();
    result["execution"] = serde_json::to_value(&execution)?;
    let reconciled = match &market {
        Some(m) => super::meteora_dlmm::reconcile_current(&m.parameters, &plan, &execution),
        None => token_transfer::reconcile_current(
            &s.context.mint,
            &s.context.source,
            &s.context.destination,
            s.context.decimals,
            s.amount,
            &plan,
            &execution,
        ),
    };
    let mut deltas = match reconciled {
        Ok(d) => d,
        Err(e) => {
            return fail(
                result,
                PathStatus::Indeterminate,
                format!("Instruction ran, but exact reconciliation could not be established: {e}"),
            )
        }
    };
    if !execution.success {
        deltas.reconciled &= plan.watch.iter().all(|address| {
            execution.post_accounts.get(address)
                == plan
                    .accounts
                    .iter()
                    .find(|a| &a.address == address)
                    .map(|a| &a.account)
        });
    }
    result["status"] = serde_json::to_value(if deltas.reconciled {
        if execution.success {
            PathStatus::Proven
        } else {
            PathStatus::Failed
        }
    } else {
        PathStatus::Indeterminate
    })?;
    if !deltas.reconciled {
        result["reason"] = "The instruction ran, but exact economic/account deltas did not reconcile. No execution proof was granted.".into();
    }
    result["execution_performed"] = true.into();
    result["local_execution_performed"] = true.into();
    result["signer_assumed_locally"] = true.into();
    result["execution"] = serde_json::to_value(execution)?;
    let mut delta_value = serde_json::to_value(deltas)?;
    // Historical adapter report wording stays byte-compatible; current result names the actual program.
    if market.is_none() {
        delta_value["reconciliation"][0] = delta_value["reconciliation"][0]
            .as_str()
            .unwrap()
            .replace(
                "Actual Token-2022 transfer",
                &format!("Actual {} transfer", s.context.program),
            )
            .into();
    }
    result["reconciliation"] = delta_value;
    eprintln!("CURRENT_STAGE:Preparing evidence");
    Ok(VerifiedExecution { result })
}

/// A transcript reader, not a network client. Re-running preparation verifies every request and selection.
struct ReplayRpc<'a> {
    records: &'a [observation::Observation],
    next: Mutex<usize>,
}
impl SolanaRpc for ReplayRpc<'_> {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut next = self.next.lock().unwrap();
        let r = self
            .records
            .get(*next)
            .context("required captured response missing")?;
        ensure!(
            r.method == method && r.params == params,
            "market acquisition request binding mismatch"
        );
        *next += 1;
        r.result
            .clone()
            .context("bounded public-state request failed")
    }
}
