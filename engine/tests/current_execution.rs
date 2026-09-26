//! Deterministic reconstructed acquisition using frozen raw bytes and deployed code.
//! These tests execute the VM; none constitutes live acquisition acceptance.
use anyhow::{bail, Result};
use eplyx_engine::{
    lifecycle::{current as wallet, exposure::sha256, rpc::SolanaRpc},
    path::{current as check, CapturedExecutionFixture, ExitPathType},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Mutex};
struct Rpc {
    raw: BTreeMap<String, Value>,
    slot: u64,
    mint: String,
    source: String,
    owner: String,
    calls: Mutex<Vec<String>>,
}
impl Rpc {
    fn new() -> Self {
        let root = eplyx_engine::lifecycle::artifact::reference_root();
        let fixture: CapturedExecutionFixture = serde_json::from_slice(
            &std::fs::read(root.join("probes/phase7-captures/fixtures/group-0.json")).unwrap(),
        )
        .unwrap();
        let batch = &fixture.evidence[3];
        let raw: BTreeMap<_, _> = batch.params[0]
            .as_array()
            .unwrap()
            .iter()
            .zip(batch.result["value"].as_array().unwrap())
            .map(|(a, v)| (a.as_str().unwrap().to_string(), v.clone()))
            .collect();
        let mint = fixture.evidence[1].params[0].as_str().unwrap().to_string();
        let source = "741ZXYKzgjPZucRhPUQFK7kKAgP1ESJwimhumgGpuPHs".to_string();
        let owner = eplyx_engine::lifecycle::decode::decode_token_account(
            &raw[&source],
            eplyx_engine::lifecycle::decode::TOKEN_2022_PROGRAM,
            &mint,
            9,
        )
        .unwrap()
        .owner;
        Self {
            raw,
            slot: batch.result["context"]["slot"].as_u64().unwrap(),
            mint,
            source,
            owner,
            calls: Mutex::new(vec![]),
        }
    }
    fn request(&self) -> check::CheckRequest {
        check::CheckRequest {
            path: ExitPathType::Transfer,
            source: self.source.clone(),
            amount_mode: check::AmountMode::Custom,
            amount_decimal: Some("0.0000001".into()),
            recipient: "124XHuTYUNnCf2ABCNcCEdQFpHQ7Y6MnPe9NeouDB1az".into(),
            output_mint: None,
            minimum_output_decimal: None,
        }
    }
    fn wallet(&self) -> String {
        serde_json::to_string(
            &wallet::capture_selected(
                wallet::InspectionSelection {
                    cluster: "solana-mainnet".into(),
                    mint: self.mint.clone(),
                    reference: None,
                    sample_accounts: false,
                    public_owner: Some(self.owner.clone()),
                },
                self,
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn capture(&self) -> check::Capture {
        check::capture(
            self.wallet(),
            self.request(),
            "run-a".into(),
            "check-a".into(),
            self,
        )
        .unwrap()
    }
}
impl SolanaRpc for Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls.lock().unwrap().push(method.into());
        let value = match method {
            "getGenesisHash" => return Ok(json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
            "getAccountInfo" => self
                .raw
                .get(params[0].as_str().unwrap())
                .cloned()
                .unwrap_or(Value::Null),
            "getMultipleAccounts" => json!(params[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| self
                    .raw
                    .get(a.as_str().unwrap())
                    .cloned()
                    .unwrap_or(Value::Null))
                .collect::<Vec<_>>()),
            "getTokenAccountsByOwner" => {
                json!([{"pubkey":self.source,"account":self.raw[&self.source]}])
            }
            "getProgramAccounts" => json!(self
                .raw
                .iter()
                .filter(|(_, v)| v["owner"]
                    == eplyx_engine::lifecycle::exposure::meteora_dlmm::PROGRAM_ID
                    && v["space"] == 904)
                .map(|(a, v)| json!({"pubkey":a,"account":v}))
                .collect::<Vec<_>>()),
            _ => bail!("unexpected method"),
        };
        Ok(json!({"context":{"slot":self.slot},"value":value}))
    }
}
fn replay(c: &check::Capture) -> Result<Value> {
    let b = serde_json::to_vec(c)?;
    Ok(check::replay(
        &b,
        "run-a",
        "check-a",
        &sha256(c.wallet_capture.as_bytes()),
        &sha256(&b),
    )?
    .value()
    .clone())
}
#[test]
fn exact_decimal_parsing() {
    assert_eq!(
        check::exact_amount("18446744073.709551615", 9).unwrap(),
        u64::MAX
    );
    assert_eq!(check::exact_amount("0.000000001", 9).unwrap(), 1);
    for text in [
        "0",
        "-1",
        "1e2",
        "NaN",
        ".1",
        "1.",
        "1.0000000001",
        "18446744073.709551616",
        "1 0",
    ] {
        assert!(check::exact_amount(text, 9).is_err(), "{text}");
    }
}
#[test]
fn current_proof_requires_actual_execution_and_replays_identically() {
    let rpc = Rpc::new();
    let c = rpc.capture();
    let discovery = wallet::evaluate(&serde_json::from_str(&c.wallet_capture).unwrap()).unwrap();
    assert!(
        discovery["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["status"] == "NotTested"),
        "historical proof cannot enter fresh observations"
    );
    let r = replay(&c).unwrap();
    assert_eq!(
        r["status"], "Proven",
        "actual execution and reconciliation creates proof"
    );
    assert_eq!(r["execution_performed"], true);
    assert!(
        r["execution"]["compute_units"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    assert_eq!(r["reconciliation"]["input_debited_raw"], "100");
    assert_eq!(r["reconciliation"]["output_received_raw"], "99");
    assert_eq!(
        r["reconciliation"]["token_accounts"][1]["withheld_fee_change_raw"], "1",
        "fee must reconcile"
    );
    assert_eq!(
        r["signer_possession_known"], false,
        "assumption never proves possession"
    );
    assert_eq!(r["signer_assumed_locally"], true);
    assert_eq!(r["readiness"], Value::Null);
    let calls = rpc.calls.lock().unwrap().len();
    assert_eq!(
        r,
        replay(&c).unwrap(),
        "offline replay reexecutes identically"
    );
    assert_eq!(calls, rpc.calls.lock().unwrap().len());
}
#[test]
fn exact_account_amount_run_and_fixture_bindings() {
    let rpc = Rpc::new();
    let wallet = rpc.wallet();
    let mut request = rpc.request();
    request.source = solana_address::Address::new_from_array([3; 32]).to_string();
    assert!(
        check::validate(&wallet, &request).is_err(),
        "focused account mismatch must be rejected"
    );
    request = rpc.request();
    request.amount_decimal = Some("18446744073.709551615".into());
    assert!(
        check::validate(&wallet, &request).is_err(),
        "above balance must be rejected"
    );
    let c = rpc.capture();
    let b = serde_json::to_vec(&c).unwrap();
    for (run, id, wallet, digest) in [
        (
            "run-b",
            "check-a",
            sha256(c.wallet_capture.as_bytes()),
            sha256(&b),
        ),
        (
            "run-a",
            "check-b",
            sha256(c.wallet_capture.as_bytes()),
            sha256(&b),
        ),
        ("run-a", "check-a", "0".repeat(64), sha256(&b)),
        (
            "run-a",
            "check-a",
            sha256(c.wallet_capture.as_bytes()),
            "0".repeat(64),
        ),
    ] {
        assert!(
            check::replay(&b, run, id, &wallet, &digest).is_err(),
            "cross-run or cross-fixture proof must be rejected"
        );
    }
}
#[test]
fn missing_capture_never_promotes_to_proven() {
    let rpc = Rpc::new();
    let mut c = rpc.capture();
    c.observations.pop();
    let r = replay(&c).unwrap();
    assert_eq!(
        r["status"], "Indeterminate",
        "missing capture must not promote"
    );
    assert_eq!(r["execution_performed"], false);
}
#[test]
fn changed_discovery_source_requires_reconfirmation() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let rpc = Rpc::new();
    let mut c = rpc.capture();
    let batch = &mut c.observations[3];
    let i = batch.params[0]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a == &rpc.source)
        .unwrap();
    let raw = &mut batch.result.as_mut().unwrap()["value"][i];
    let mut data = STANDARD.decode(raw["data"][0].as_str().unwrap()).unwrap();
    let old = u64::from_le_bytes(data[64..72].try_into().unwrap());
    data[64..72].copy_from_slice(&(old + 1).to_le_bytes());
    raw["data"][0] = STANDARD.encode(data).into();
    let r = replay(&c).unwrap();
    assert_eq!(r["status"], "Indeterminate");
    assert!(r["reason"].as_str().unwrap().contains("account changed"));
    assert_eq!(r["execution_performed"], false);
}
fn edit_raw(raw: &mut Value, edit: impl FnOnce(&mut Vec<u8>)) {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut bytes = STANDARD.decode(raw["data"][0].as_str().unwrap()).unwrap();
    edit(&mut bytes);
    raw["space"] = json!(bytes.len());
    raw["data"][0] = STANDARD.encode(bytes).into();
}
#[test]
fn deterministic_actual_vm_failure_is_failed_with_rollback() {
    let mut rpc = Rpc::new();
    let recipient = rpc.request().recipient;
    // Explicit synthetic integration variant: overflowing recipient, never a live failure claim.
    edit_raw(rpc.raw.get_mut(&recipient).unwrap(), |b| {
        b[64..72].copy_from_slice(&u64::MAX.to_le_bytes())
    });
    let result = replay(&rpc.capture()).unwrap();
    assert_eq!(
        result["status"], "Failed",
        "actual instruction failure is classified Failed"
    );
    assert_eq!(result["execution_performed"], true);
    assert_eq!(result["execution"]["success"], false);
    assert_eq!(
        result["reconciliation"]["reconciled"], true,
        "failed VM execution must roll back watched state"
    );
    assert_eq!(result["reconciliation"]["input_debited_raw"], "0");
    assert_eq!(result["reconciliation"]["output_received_raw"], "0");
}
#[test]
fn unsupported_frozen_paused_hook_confidential_and_authority_never_execute() {
    use spl_token_2022_interface::{
        extension::{
            confidential_transfer::ConfidentialTransferAccount, pausable::PausableConfig,
            transfer_hook::TransferHook, BaseStateWithExtensionsMut, ExtensionType,
            StateWithExtensions, StateWithExtensionsMut,
        },
        state::{Account, Mint},
    };
    for case in ["frozen", "paused", "hook", "confidential", "authority"] {
        let mut rpc = Rpc::new();
        if case == "frozen" {
            edit_raw(rpc.raw.get_mut(&rpc.source).unwrap(), |b| b[108] = 2);
        }
        if case == "paused" || case == "hook" {
            edit_raw(rpc.raw.get_mut(&rpc.mint).unwrap(), |b| {
                let mut state = StateWithExtensionsMut::<Mint>::unpack(b).unwrap();
                if case == "paused" {
                    state.get_extension_mut::<PausableConfig>().unwrap().paused = true.into();
                } else {
                    state
                        .get_extension_mut::<TransferHook>()
                        .unwrap()
                        .program_id = Some(solana_address::Address::new_from_array([5; 32]))
                        .try_into()
                        .unwrap();
                }
            });
        }
        if case == "confidential" {
            edit_raw(rpc.raw.get_mut(&rpc.source).unwrap(), |b| {
                let base = StateWithExtensions::<Account>::unpack(b).unwrap().base;
                let mut bytes = vec![
                    0;
                    ExtensionType::try_calculate_account_len::<Account>(&[
                        ExtensionType::ConfidentialTransferAccount
                    ])
                    .unwrap()
                ];
                let mut a =
                    StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut bytes).unwrap();
                a.init_extension::<ConfidentialTransferAccount>(true)
                    .unwrap();
                a.base = base;
                a.pack_base();
                a.init_account_type().unwrap();
                *b = bytes;
            });
        }
        if case == "authority" {
            let mut c = rpc.capture();
            let batch = &mut c.observations[3];
            let i = batch.params[0]
                .as_array()
                .unwrap()
                .iter()
                .position(|a| a == &rpc.owner)
                .unwrap();
            batch.result.as_mut().unwrap()["value"][i]["owner"] = rpc.mint.clone().into();
            let r = replay(&c).unwrap();
            assert_eq!(
                r["status"], "Unsupported",
                "program authority cannot be impersonated"
            );
            assert_eq!(r["execution_performed"], false);
            continue;
        }
        let r = replay(&rpc.capture()).unwrap();
        assert_eq!(r["status"], "Unsupported", "unsupported {case}");
        assert_eq!(
            r["execution_performed"], false,
            "unsupported {case} must never assume signing"
        );
    }
}
#[test]
fn legacy_transfer_uses_deployed_legacy_code_and_another_mint() {
    use eplyx_engine::lifecycle::decode::LEGACY_PROGRAM;
    let mut rpc = Rpc::new();
    let root = eplyx_engine::lifecycle::artifact::reference_root();
    let f: CapturedExecutionFixture = serde_json::from_slice(
        &std::fs::read(root.join("probes/phase7-captures/fixtures/group-1.json")).unwrap(),
    )
    .unwrap();
    let e = &f.evidence[3];
    // Reconstruct a deterministic legacy bank using actual captured legacy deployed code.
    for (address, raw) in e.params[0]
        .as_array()
        .unwrap()
        .iter()
        .zip(e.result["value"].as_array().unwrap())
    {
        if !raw.is_null() {
            rpc.raw
                .entry(address.as_str().unwrap().into())
                .or_insert_with(|| raw.clone());
        }
    }
    let mint = solana_address::Address::new_from_array([7; 32]);
    let mut raw = rpc.raw.remove(&rpc.mint).unwrap();
    edit_raw(&mut raw, |b| b.truncate(82));
    raw["owner"] = LEGACY_PROGRAM.into();
    rpc.mint = mint.to_string();
    rpc.raw.insert(rpc.mint.clone(), raw);
    let recipient = rpc.request().recipient;
    for address in [&rpc.source, &recipient] {
        let raw = rpc.raw.get_mut(address).unwrap();
        edit_raw(raw, |b| {
            b.truncate(165);
            b[..32].copy_from_slice(mint.as_ref());
        });
        raw["owner"] = LEGACY_PROGRAM.into();
    }
    let r = replay(&rpc.capture()).unwrap();
    assert_eq!(
        r["status"], "Proven",
        "legacy VM transfer must succeed: execution={} deltas={}",
        r["execution"], r["reconciliation"]
    );
    assert_eq!(r["mint"], rpc.mint);
    assert_eq!(r["deployed_programs"][0]["program"], LEGACY_PROGRAM);
    assert_eq!(r["reconciliation"]["input_debited_raw"], "100");
    assert_eq!(r["reconciliation"]["output_received_raw"], "100");
    assert_eq!(
        r["reconciliation"]["token_accounts"][1]["withheld_fee_change_raw"],
        "0"
    );
}

fn market_corpus() -> (Rpc, check::CheckRequest) {
    let mut rpc = Rpc::new();
    let root = eplyx_engine::lifecycle::artifact::reference_root();
    let f: CapturedExecutionFixture = serde_json::from_slice(
        &std::fs::read(root.join("probes/phase7-captures/fixtures/group-1.json")).unwrap(),
    )
    .unwrap();
    let batch = &f.evidence[3];
    rpc.slot = batch.result["context"]["slot"].as_u64().unwrap();
    for (a, v) in batch.params[0]
        .as_array()
        .unwrap()
        .iter()
        .zip(batch.result["value"].as_array().unwrap())
    {
        rpc.raw.insert(a.as_str().unwrap().into(), v.clone());
    }
    let request = check::CheckRequest {
        path: ExitPathType::SecondaryMarketExit,
        source: rpc.source.clone(),
        recipient: String::new(),
        amount_mode: check::AmountMode::Full,
        amount_decimal: None,
        output_mint: Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into()),
        minimum_output_decimal: Some("0.0001".into()),
    };
    (rpc, request)
}
fn market_capture(rpc: &Rpc, request: check::CheckRequest) -> check::Capture {
    check::capture(rpc.wallet(), request, "run-a".into(), "check-a".into(), rpc).unwrap()
}
#[test]
fn current_market_executes_and_reconciles_one_exact_route_offline() {
    let (rpc, request) = market_corpus();
    let c = market_capture(&rpc, request);
    let r = replay(&c).unwrap();
    assert_eq!(
        r["status"],
        "Proven",
        "market result: {}",
        r.get("reason").unwrap_or(&Value::Null)
    );
    assert_eq!(r["reconciliation"]["reconciled"], true);
    assert_eq!(
        r["discovery"]["global_exitability_established"], false,
        "one route never proves global exitability"
    );
    assert_eq!(
        r["market_parameters"]["minimum_output_raw"], "100",
        "minimum output must bind to the instruction"
    );
    assert!(
        r["execution"]["inner_instructions"]
            .as_array()
            .unwrap()
            .len()
            > 1
    );
    assert_eq!(r, replay(&c).unwrap());
}
#[test]
fn market_minimum_failure_and_missing_bin_are_distinct() {
    let (mut rpc, mut request) = market_corpus();
    request.minimum_output_decimal = Some("1000000".into());
    let r = replay(&market_capture(&rpc, request.clone())).unwrap();
    assert_eq!(
        r["status"], "Failed",
        "supported min-out failure must come from the VM: {}",
        r["reason"]
    );
    assert_eq!(r["reconciliation"]["reconciled"], true);
    assert_eq!(r["reconciliation"]["input_debited_raw"], "0");
    let bin = rpc
        .raw
        .iter()
        .find(|(_, v)| v["space"] == 10136)
        .unwrap()
        .0
        .clone();
    rpc.raw.insert(bin, Value::Null);
    let r = replay(&market_capture(&rpc, request)).unwrap();
    assert_eq!(r["status"], "Indeterminate");
    assert_eq!(r["execution_performed"], false);
}
#[test]
fn no_supported_pool_is_bounded_and_never_global_absence() {
    let (mut rpc, request) = market_corpus();
    rpc.raw.retain(|_, v| v["space"] != 904);
    let r = replay(&market_capture(&rpc, request)).unwrap();
    assert_eq!(r["status"], "Unsupported");
    assert_eq!(
        r["discovery"]["global_exitability_established"], false,
        "empty bounded scan is not global absence"
    );
    assert_eq!(r["execution_performed"], false);
}

#[test]
fn current_request_cannot_import_claimed_proof_or_custom_programs() {
    let rpc = Rpc::new();
    let request = serde_json::to_value(rpc.request()).unwrap();
    for key in [
        "status",
        "evidence_sha256",
        "program_id",
        "instructions",
        "account_metas",
        "rpc_url",
        "transaction",
    ] {
        let mut altered = request.clone();
        altered[key] = json!("Proven");
        assert!(
            serde_json::from_value::<check::CheckRequest>(altered).is_err(),
            "caller-controlled proof or instruction fields are forbidden"
        );
    }
    let mut capture = rpc.capture();
    let mut wallet: Value = serde_json::from_str(&capture.wallet_capture).unwrap();
    wallet["asset"]["mint"] = "So11111111111111111111111111111111111111112".into();
    capture.wallet_capture = serde_json::to_string(&wallet).unwrap();
    capture.wallet_capture_sha256 = sha256(capture.wallet_capture.as_bytes());
    assert!(
        replay(&capture).is_err(),
        "cross-mint acquisition cannot satisfy proof"
    );
}
#[test]
fn availability_uses_engine_boundaries_without_granting_proof() {
    let rpc = Rpc::new();
    let r = check::capabilities(&rpc.wallet()).unwrap();
    assert_eq!(r["accounts"][0]["source"], rpc.source);
    assert_eq!(r["accounts"][0]["checks"][0]["status"], "NotTested");
    assert_eq!(r["accounts"][0]["checks"][0]["available"], true);
    assert_eq!(r["execution_performed"], false);
}

#[test]
fn current_cli_is_deterministic_isolated_and_refuses_overwrite() {
    use std::process::Command;
    let rpc = Rpc::new();
    let capture = rpc.capture();
    let bytes = serde_json::to_vec(&capture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("capture.json");
    let output = directory.path().join("result.json");
    std::fs::write(&input, &bytes).unwrap();
    let wallet_hash = sha256(capture.wallet_capture.as_bytes());
    let capture_hash = sha256(&bytes);
    let run = |save: bool| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_eplyx"));
        c.env_clear()
            .env("SOLANA_RPC_URL", "sentinel-no-network")
            .env("EPLYX_TOKEN", "sentinel-not-forwarded")
            .current_dir(directory.path())
            .args([
                "path",
                "replay",
                "--input",
                input.to_str().unwrap(),
                "--run-id",
                "run-a",
                "--check-id",
                "check-a",
                "--wallet-sha256",
                &wallet_hash,
                "--capture-sha256",
                &capture_hash,
                "--format",
                "json",
            ]);
        c.arg("--record").arg(directory.path());
        if save {
            c.arg("--out").arg(&output);
        }
        c.output().unwrap()
    };
    let first = run(true);
    let second = run(false);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(second.status.success());
    assert_eq!(sha256(&first.stdout), sha256(&second.stdout));
    assert_eq!(std::fs::read(&output).unwrap(), first.stdout);
    let value: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(value["status"], "Proven");
    assert!(value["execution"]["compute_units"].is_string());
    assert!(value["clock"]["slot"].is_string());
    assert!(!String::from_utf8_lossy(&first.stdout).contains("sentinel"));
    let store = eplyx_engine::dashboard::store::Store::open(directory.path()).unwrap();
    let ids = store.run_ids().unwrap().0;
    assert_eq!(
        ids.len(),
        2,
        "recording appends even when analytical bytes repeat"
    );
    for id in ids {
        let detail = eplyx_engine::dashboard::view::run_detail(&store, &id, &[]).unwrap();
        assert_eq!(detail["state"], "Complete");
        assert_eq!(detail["kind"], "current_path");
        assert_eq!(detail["status"], "Proven");
        assert!(detail["gate"]["outcome"].is_null());
        assert_eq!(detail["analysis"]["report"], value);
    }
    let repeat = run(true);
    assert_eq!(repeat.status.code(), Some(2));
    assert_eq!(std::fs::read(&output).unwrap(), first.stdout);
    let worker = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["path-worker", "{}"])
        .env_clear()
        .env("SECRET_SENTINEL", "not-forwarded")
        .output()
        .unwrap();
    assert_eq!(worker.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&worker.stderr).contains("empty environment"));
    assert!(!String::from_utf8_lossy(&worker.stderr).contains("not-forwarded"));
}

#[test]
fn route_budget_and_selected_failure_cannot_trigger_fallback() {
    use eplyx_engine::protocol::meteora_dlmm;
    use solana_address::Address;
    let (mut rpc, request) = market_corpus();
    let (original, raw) = rpc
        .raw
        .iter()
        .find(|(_, v)| v["space"] == 904)
        .map(|(a, v)| (a.clone(), v.clone()))
        .unwrap();
    let program: Address = meteora_dlmm::PROGRAM_ID.parse().unwrap();
    let mut selected = None;
    for seed in 0..256u16 {
        let mut candidate = raw.clone();
        let mut address = String::new();
        edit_raw(&mut candidate, |bytes| {
            let x: [u8; 32] = bytes[88..120].try_into().unwrap();
            let y: [u8; 32] = bytes[120..152].try_into().unwrap();
            let mut base = [0u8; 32];
            base[..2].copy_from_slice(&seed.to_le_bytes());
            let (lo, hi) = if x < y { (&x, &y) } else { (&y, &x) };
            let (pool, bump) = Address::find_program_address(&[&base, lo, hi], &program);
            address = pool.to_string();
            bytes[72] = bump;
            bytes[784..816].copy_from_slice(&base);
            for (offset, mint) in [(152, x), (184, y)] {
                bytes[offset..offset + 32].copy_from_slice(
                    Address::find_program_address(&[pool.as_ref(), &mint], &program)
                        .0
                        .as_ref(),
                );
            }
            bytes[552..584].copy_from_slice(
                Address::find_program_address(&[b"oracle", pool.as_ref()], &program)
                    .0
                    .as_ref(),
            );
        });
        if address < original {
            selected = Some((address, candidate));
            break;
        }
    }
    let (first, candidate) = selected.expect("derived compatible pool preceding the real route");
    rpc.raw.insert(first.clone(), candidate);
    // Both layouts are compatible; the first lacks its final dependencies.
    let result = replay(&market_capture(&rpc, request.clone())).unwrap();
    assert_eq!(result["discovery"]["selected_pool"], first);
    assert_eq!(result["discovery"]["compatible_layout_candidates"], 2);
    assert_eq!(result["status"], "Indeterminate");
    assert_eq!(result["execution_performed"], false);
    for n in 0..64u8 {
        rpc.raw
            .insert(Address::new_from_array([n; 32]).to_string(), raw.clone());
    }
    let result = replay(&market_capture(&rpc, request)).unwrap();
    assert_eq!(result["status"], "Indeterminate");
    assert_eq!(result["execution_performed"], false);
    assert!(result["reason"]
        .as_str()
        .unwrap()
        .contains("64 candidate budget"));
}
