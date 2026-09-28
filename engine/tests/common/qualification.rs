//! Qualification uses the existing CPI fixture and real reference/token SBF.
use super::*;
use eplyx_engine::qualification::{self as q, Code, Config, Providers, State};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Frozen {
    archive: Archive,
    raw: Vec<Value>,
    calls: AtomicUsize,
    slot_mismatch: bool,
    secret_error: bool,
}
impl Frozen {
    fn new() -> Self {
        let (record, v1, _, dependencies) = local_scenario();
        let post = record.execute(&v1, &dependencies).unwrap();
        let mut archive = Archive::new(419_472_000, None);
        let mut raw = raw_transaction();
        raw["transaction"]["message"]["accountKeys"] = json!(record
            .transaction
            .account_keys
            .iter()
            .map(|k| &k.address)
            .collect::<Vec<_>>());
        for account in &record.accounts {
            let value = |a: &AccountSnapshot| {
                let mut value = account_json(a.lamports, &a.owner, &a.data, a.executable);
                value["rentEpoch"] = json!(a.rent_epoch);
                value
            };
            archive
                .accounts
                .insert((account.address.clone(), SLOT - 1), value(&account.account));
            archive.accounts.insert(
                (account.address.clone(), SLOT),
                value(&post.accounts[&account.label]),
            );
        }
        let token = std::fs::read(committed_dependencies().join(format!("{TOKEN}.so"))).unwrap();
        for slot in [SLOT - 1, SLOT] {
            for (id, data, bytes, deploy) in [
                (STAKE_POOL, address(100), v1.bytes.as_slice(), 370_300_186),
                (TOKEN, address(101), token.as_slice(), 419_472_000),
            ] {
                let (program, programdata) = upgradeable_program(&data, deploy, bytes);
                archive.accounts.insert((id.into(), slot), program);
                archive.accounts.insert((data, slot), programdata);
            }
        }
        Self {
            archive,
            raw: vec![raw],
            calls: AtomicUsize::new(0),
            slot_mismatch: false,
            secret_error: false,
        }
    }
    fn add_incompatible(&mut self, dependency: bool) {
        let mut raw = self.raw[0].clone();
        raw["slot"] = json!(SLOT + 2);
        raw["transaction"]["signatures"][0] = json!(bs58::encode([8u8; 64]).into_string());
        for ((key, slot), value) in self.archive.accounts.clone() {
            self.archive.accounts.insert((key, slot + 2), value);
        }
        let (id, data) = if dependency {
            (TOKEN, address(101))
        } else {
            (STAKE_POOL, address(100))
        };
        // A different deployment identity, even when its ELF bytes match, must
        // require a cohort decision. The bundle must not choose the largest.
        let value = self.archive.accounts.get_mut(&(data, SLOT + 1)).unwrap();
        let mut bytes = base64::prelude::BASE64_STANDARD
            .decode(value["data"][0].as_str().unwrap())
            .unwrap();
        bytes[4..12].copy_from_slice(&(SLOT - 10).to_le_bytes());
        value["data"][0] = json!(encode(&bytes));
        assert!(self.archive.accounts.contains_key(&(id.into(), SLOT + 1)));
        self.raw.push(raw);
    }
}
impl RpcProvider for Frozen {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.secret_error {
            anyhow::bail!("https://private.invalid/token=SECRET /secret/config");
        }
        match method {
            "getGenesisHash" => Ok(json!(eplyx_engine::ingest::rpc::MAINNET_GENESIS)),
            "getSignaturesForAddress" => {
                if params[1].get("before").is_some() {
                    Ok(json!([]))
                } else {
                    Ok(json!(self.raw.iter().rev().map(|r|json!({"signature":r["transaction"]["signatures"][0],"slot":r["slot"]})).collect::<Vec<_>>()))
                }
            }
            "getTransaction" => Ok(self
                .raw
                .iter()
                .find(|r| r["transaction"]["signatures"][0] == params[0])
                .unwrap()
                .clone()),
            "getBlock" => {
                let mut block = self.archive.block.clone();
                let raw = self.raw.iter().find(|r| r["slot"] == params[0]).unwrap();
                // The fixture block normally places the target at index 1;
                // locate it by its original signature, even with a conflict.
                for entry in block["transactions"].as_array_mut().unwrap() {
                    if entry["transaction"]["signatures"][0] == signature() {
                        entry["transaction"]["signatures"][0] =
                            raw["transaction"]["signatures"][0].clone();
                    }
                }
                Ok(block)
            }
            "getAccountInfo" => {
                let mut value = self.archive.call(method, params)?;
                if self.slot_mismatch {
                    value["context"]["slot"] = json!(1);
                }
                Ok(value)
            }
            _ => anyhow::bail!("unexpected frozen request {method}"),
        }
    }
}
fn config() -> Config {
    serde_json::from_value(json!({"schema_version":1,"scope":{
        "program":STAKE_POOL,"actions":["deposit"],"subjects":[{"protocol":"spl-stake-pool","action":"deposit_sol","domain":"economic","subject":"pool_tokens_received"}],
        "start_slot":SLOT,"end_slot":SLOT+2,"discovery_limit":10,"acquisition_budget":10,"target_size":10,"replay_schema":1,"runtime_profile":"schema1_litesvm_mainnet"},
        "providers":{"transaction_url_env":"SECRET_URL","account_url_env":"SECRET_URL","block_url_env":"SECRET_URL"}})).unwrap()
}
fn run(c: &Config, f: &dyn RpcProvider, out: &std::path::Path) -> q::Receipt {
    q::prepare(
        c,
        out,
        Providers {
            transactions: f,
            accounts: f,
            blocks: f,
        },
    )
    .unwrap()
}
#[test]
fn qualification_real_acquisition_fidelity_verify_and_frozen_cache_are_deterministic() {
    let fixture = Frozen::new();
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let rpc = eplyx_engine::ingest::CachedRpc {
        provider: &fixture,
        root: cache.clone(),
    };
    let first = run(&config(), &rpc, &temp.path().join("first"));
    assert_eq!(
        first.state,
        State::CoverageReviewRequired,
        "{}",
        serde_json::to_string_pretty(&first).unwrap()
    );
    assert!(first.bundle_verified);
    assert_eq!(
        first.observed_semantic_population,
        json!({"status":"unknown"})
    );
    let selection = first.selection.as_ref().unwrap();
    assert_eq!(selection.selected.len(), 1);
    let bundle = first.bundle.as_ref().unwrap();
    assert_eq!(bundle.record_ids, vec![selection.selected[0].id.clone()]);
    assert_eq!(
        bundle.baseline_program_sha256,
        hash_bytes(&artifact("fixture_stake_pool_reference.so").bytes)
    );
    assert_eq!(bundle.dependencies.len(), 1);
    assert_eq!(bundle.dependencies[0].program_id, TOKEN);
    assert_eq!(
        first.adapter.as_ref().unwrap(),
        &json!({"name":"spl-stake-pool","version":3})
    );
    assert_eq!(first.fidelity.len(), 1);
    assert_eq!(first.fidelity[0].fidelity, Some(ReplayFidelity::Matched));
    assert_eq!(first.fidelity[0].record_id, selection.selected[0].id);
    assert_eq!(
        first.ledger[0].record_id.as_ref(),
        Some(&selection.selected[0].id)
    );
    let offline = eplyx_engine::ingest::CachedRpc {
        provider: &eplyx_engine::ingest::rpc::OfflineRpc,
        root: cache,
    };
    let second = run(&config(), &offline, &temp.path().join("second"));
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    assert_eq!(
        std::fs::read(temp.path().join("first/bundle/bundle.json")).unwrap(),
        std::fs::read(temp.path().join("second/bundle/bundle.json")).unwrap()
    );
    let mut accepted = config();
    accepted.accept_coverage = true;
    let third = run(&accepted, &offline, &temp.path().join("accepted"));
    assert_eq!(third.state, State::ReadyForActivation);
    assert_eq!(third.exit_code, 20);
    assert_eq!(third.bundle.unwrap().bundle_sha256, bundle.bundle_sha256);
    let text = serde_json::to_string(&first).unwrap();
    assert!(!text.contains("SECRET") && !text.contains(temp.path().to_str().unwrap()));
    // Verify corruption cannot survive reopening.
    std::fs::write(temp.path().join("first/bundle/binaries/current.so"), b"bad").unwrap();
    assert!(eplyx_engine::bundle::CiBundle::open(temp.path().join("first/bundle")).is_err());
}
#[test]
fn qualification_local_support_failures_make_no_provider_requests() {
    let fixture = Frozen::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cases = vec![];
    let mut c = config();
    c.scope.program = SYSTEM.into();
    cases.push((c, Code::MissingAdapter));
    let mut c = config();
    c.scope.actions = vec!["swap".into()];
    cases.push((c, Code::UnsupportedAction));
    let mut c = config();
    c.scope.subjects[0].subject =
        eplyx_engine::semantics::SemanticSubject::new("not_evaluable").unwrap();
    cases.push((c, Code::MissingEvaluableSubject));
    let mut c = config();
    c.scope.replay_schema = 2;
    cases.push((c, Code::OutsideSchema1QualificationContract));
    let mut c = config();
    c.scope.runtime_profile = "invented".into();
    cases.push((c, Code::UnsupportedRuntime));
    let mut c = config();
    c.scope.program = eplyx_engine::protocol::orca::PROGRAM_ID.into();
    cases.push((c, Code::OutsideSchema1QualificationContract));
    for (i, (c, code)) in cases.iter().enumerate() {
        let r = run(c, &fixture, &temp.path().join(i.to_string()));
        assert_eq!(r.blockers, vec![*code]);
        assert!(!r.bundle_verified);
    }
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn qualification_acquisition_refusals_stay_in_ledger_without_secrets() {
    let temp = tempfile::tempdir().unwrap();
    for (i, code) in [
        Code::ArchiveSlotMismatch,
        Code::SameSlotConflict,
        Code::TargetExecutableUnavailable,
        Code::DependencyResolutionFailed,
        Code::CreationClosureUnsupported,
        Code::UnsupportedLookupTables,
        Code::UnsupportedCpi,
        Code::UnsupportedTransaction,
        Code::HistoricalAccountUnavailable,
    ]
    .into_iter()
    .enumerate()
    {
        let mut fixture = Frozen::new();
        match code {
            Code::ArchiveSlotMismatch => fixture.slot_mismatch = true,
            Code::SameSlotConflict => fixture.archive.block = block(Some(&reserve())),
            Code::TargetExecutableUnavailable => {
                fixture
                    .archive
                    .accounts
                    .remove(&(STAKE_POOL.into(), SLOT - 1));
            }
            Code::DependencyResolutionFailed => {
                fixture.archive.accounts.remove(&(TOKEN.into(), SLOT - 1));
            }
            Code::CreationClosureUnsupported => {
                fixture.archive.accounts.remove(&(pool(), SLOT));
            }
            Code::UnsupportedLookupTables => {
                fixture.raw[0]["version"] = json!(0);
                fixture.raw[0]["transaction"]["message"]["addressTableLookups"] =
                    json!([{"accountKey":address(221),"writableIndexes":[0],"readonlyIndexes":[]}]);
                fixture.raw[0]["meta"]["preBalances"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(0));
                fixture.raw[0]["meta"]["postBalances"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(0));
                fixture.raw[0]["meta"]["loadedAddresses"] =
                    json!({"writable":[address(222)],"readonly":[]});
            }
            Code::UnsupportedTransaction => {
                fixture.raw[0]["meta"]["err"] = json!({"InstructionError":[0,"InvalidArgument"]});
            }
            Code::HistoricalAccountUnavailable => {
                fixture.archive.accounts.remove(&(pool(), SLOT));
                fixture.archive.accounts.remove(&(pool(), SLOT - 1));
            }
            Code::UnsupportedCpi => {
                fixture.raw[0]["meta"]["innerInstructions"][0]["instructions"][0]["stackHeight"] =
                    json!(3);
            }
            _ => unreachable!(),
        }
        let r = run(&config(), &fixture, &temp.path().join(i.to_string()));
        assert!(!r.bundle_verified, "{code:?}");
        assert_eq!(r.ledger.len(), 1, "{code:?}: {:?}", r.blockers);
        assert_eq!(r.ledger[0].rejection, Some(code));
    }
    let mut f = Frozen::new();
    f.secret_error = true;
    let r = run(&config(), &f, &temp.path().join("secret"));
    let text = serde_json::to_string(&r).unwrap();
    for secret in ["SECRET", "private.invalid", "/secret/config"] {
        assert!(!text.contains(secret));
    }
    assert_eq!(r.blockers, vec![Code::ProviderGenesisFailed]);
}
#[test]
fn qualification_incompatible_baseline_and_dependency_deployments_block_before_selection() {
    let temp = tempfile::tempdir().unwrap();
    for dependency in [false, true] {
        let mut f = Frozen::new();
        f.add_incompatible(dependency);
        let r = run(&config(), &f, &temp.path().join(dependency.to_string()));
        assert_eq!(
            r.blockers,
            vec![Code::IncompatibleEvidenceCohorts],
            "{}",
            serde_json::to_string_pretty(&r).unwrap()
        );
        assert_eq!(r.cohorts.len(), 2);
        assert!(r.selection.is_none());
        assert!(!r.bundle_verified);
    }
}
#[test]
fn qualification_does_not_drop_v1_mismatch_or_mark_it_verified() {
    let mut f = Frozen::new();
    // Change an opaque, unmeasured byte of the declared historical post-state.
    // Boundary acquisition permits it; authoritative V1 replay must refuse it.
    let value = f.archive.accounts.get_mut(&(pool(), SLOT)).unwrap();
    let mut bytes = base64::prelude::BASE64_STANDARD
        .decode(value["data"][0].as_str().unwrap())
        .unwrap();
    bytes[1] ^= 1;
    value["data"][0] = json!(encode(&bytes));
    let temp = tempfile::tempdir().unwrap();
    let r = run(&config(), &f, &temp.path().join("run"));
    assert_eq!(r.blockers, vec![Code::BaselineFidelityOrBuildFailed]);
    assert!(r.selection.is_some());
    assert_eq!(r.fidelity.len(), 1);
    assert!(!r.bundle_verified);
    assert!(r.bundle.is_none());
    assert!(!r.states.contains(&State::BundleVerified));
    assert!(!r.fidelity[0].failures.is_empty());
}
#[test]
fn qualification_measured_zero_yield_is_explicit_and_blocks_acceptance_of_missing_subject() {
    let temp = tempfile::tempdir().unwrap();
    let f = Frozen::new();
    let mut c = config();
    c.scope.actions.push("withdraw".into());
    c.scope.subjects.push(serde_json::from_value(json!({"protocol":"spl-stake-pool","action":"withdraw_sol","domain":"economic","subject":"sol_received_by_user"})).unwrap());
    c.accept_coverage = true;
    let observed = temp.path().join("observed.json");
    let artifact = json!({"schema_version":1,"program":STAKE_POOL,"adapter":"spl-stake-pool","adapter_version":3,"classifier":"reviewed-v1","actions":["deposit","withdraw"],"start_slot":SLOT,"end_slot":SLOT+2,"counts":{"deposit":1,"withdraw":12},"completeness":"complete_declared_window","provenance_sha256":"a".repeat(64),"unknown_interactions":0,"unsupported_interactions":0,"failed_interactions":0});
    eplyx_engine::ingest::write_json(&observed, &artifact).unwrap();
    c.observed = Some(observed.clone());
    let r = run(&c, &f, &temp.path().join("measured"));
    assert!(r.bundle_verified);
    assert_eq!(r.state, State::CoverageReviewRequired);
    assert!(r
        .limitations
        .iter()
        .any(|l| l.code == "withdraw_has_no_replayable_observations"));
    assert!(r.blockers.contains(&Code::MissingSelectedSubject));
    for (i, invalid) in [
        json!({"direct_interaction":1}),
        {
            let mut a = artifact.clone();
            a["counts"]["deposit"] = json!(-1);
            a
        },
        {
            let mut a = artifact.clone();
            a["counts"]["deposit"] = json!(1.5);
            a
        },
        {
            let mut a = artifact.clone();
            a["start_slot"] = json!(1);
            a
        },
    ]
    .into_iter()
    .enumerate()
    {
        eplyx_engine::ingest::write_json(&observed, &invalid).unwrap();
        let before = f.calls.load(Ordering::SeqCst);
        let r = run(&c, &f, &temp.path().join(format!("invalid-{i}")));
        assert_eq!(r.blockers, vec![Code::InvalidObservedPopulation]);
        assert_eq!(before, f.calls.load(Ordering::SeqCst));
    }
}

#[test]
fn qualification_prepared_schema2_and_existing_output_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let f = Frozen::new();
    let mut c = config();
    let prepared = temp.path().join("prepared");
    eplyx_engine::ingest::write_json(
        &prepared.join("manifest.json"),
        &json!({"schema_version":2}),
    )
    .unwrap();
    c.prepared_corpus = Some(prepared);
    let out = temp.path().join("run");
    let r = run(&c, &f, &out);
    assert_eq!(r.blockers, vec![Code::OutsideSchema1QualificationContract]);
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    let before = std::fs::read(out.join("receipt.json")).unwrap();
    assert!(q::prepare(
        &c,
        &out,
        Providers {
            transactions: &f,
            accounts: &f,
            blocks: &f
        }
    )
    .is_err());
    assert_eq!(before, std::fs::read(out.join("receipt.json")).unwrap());
}
#[test]
fn qualification_coordination_guards_selection_verification_and_admin_separation() {
    // Bounded architecture guards supplement the real acquisition/fidelity tests.
    let source = include_str!("../../src/qualification/mod.rs");
    assert_eq!(
        source.matches("select::select(").count(),
        1,
        "one authoritative selection"
    );
    assert!(source.contains("validation: bundle::Validation::AgainstBaseline"));
    assert!(source.contains("bundle::CiBundle::open(out.join(\"bundle\"))"));
    for forbidden in [
        "Validation::Skip",
        "register_bundle(",
        "activate_bundle(",
        "Registry::",
        "eplyx_server",
        "-mainnet-v1.so",
    ] {
        assert!(!source.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn qualification_cli_exit_json_and_config_redaction() {
    let temp = tempfile::tempdir().unwrap();
    let spec = temp.path().join("secret-config.json");
    let mut value = serde_json::to_value(config().scope).unwrap();
    value["program"] = json!(SYSTEM);
    eplyx_engine::ingest::write_json(&spec,&json!({"schema_version":1,"scope":value,"providers":{"transaction_url_env":"MISSING_PROVIDER_ENV","account_url_env":"MISSING_PROVIDER_ENV","block_url_env":"MISSING_PROVIDER_ENV"}})).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["bundle", "prepare", "--spec"])
        .arg(&spec)
        .arg("--out")
        .arg(temp.path().join("output"))
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(21),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let receipt: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(receipt["state"], "support_blocked");
    assert_eq!(receipt["blockers"], json!(["missing_adapter"]));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("secret-config"));
    std::fs::write(&spec, b"{\"https://provider.invalid/SECRET\":").unwrap();
    let invalid = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["bundle", "prepare", "--spec"])
        .arg(&spec)
        .arg("--out")
        .arg(temp.path().join("invalid"))
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(23));
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("SECRET"));
}
