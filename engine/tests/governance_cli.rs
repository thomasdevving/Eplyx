//! `eplyx governance squads` through the built binary, over real HTTP.
//!
//! A local JSON-RPC endpoint answers from the simulated Squads world, so the
//! binary's own transport, decoders and exit codes are what is tested. The
//! binary is invoked directly: `cargo run` would replace its exit status.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use eplyx_engine::change::ChangeSpec;
use eplyx_engine::governance::simulated::{self, World};
use eplyx_engine::governance::squads::{AddressTableLookup, ProposalStatus};
use eplyx_engine::governance::{BindingOutcome, GovernanceBinding};
use eplyx_engine::ingest::rpc::RpcProvider;

/// Serve JSON-RPC from `world` until the test process exits.
fn serve(world: Arc<World>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let response = match world.call(
                request["method"].as_str().unwrap(),
                request["params"].clone(),
            ) {
                Ok(result) => serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": result}),
                Err(error) => serde_json::json!({"jsonrpc": "2.0", "id": 1,
                    "error": {"code": -32000, "message": error.to_string()}}),
            }
            .to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            );
        }
    });
    url
}

fn eplyx(args: &[&str], url: &str) -> (u8, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(args)
        .args(["--rpc-url", url])
        .env_remove("SOLANA_RPC_URL")
        .output()
        .expect("run eplyx");
    (
        output.status.code().expect("exit code") as u8,
        String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr),
    )
}

fn write_spec(dir: &Path, name: &str, spec: &ChangeSpec) -> String {
    let path = dir.join(name);
    std::fs::write(&path, spec.to_document().unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

fn evidence(path: &Path) -> GovernanceBinding {
    GovernanceBinding::parse(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn bind_then_verify_then_catch_a_rewritten_buffer() {
    let scratch = tempfile::tempdir().unwrap();
    let world = Arc::new(World::new());
    let url = serve(Arc::clone(&world));
    let analysed = write_spec(scratch.path(), "analysed.json", &World::analysed_spec());
    let bound_path = scratch.path().join("bound.json");
    let bind_evidence = scratch.path().join("bind.json");
    let index = simulated::TRANSACTION_INDEX.to_string();
    let multisig = simulated::multisig().to_string();

    let (code, output) = eplyx(
        &[
            "governance",
            "squads",
            "bind",
            "--multisig",
            &multisig,
            "--transaction-index",
            &index,
            "--change-spec",
            &analysed,
            "--out",
            bound_path.to_str().unwrap(),
            "--evidence-out",
            bind_evidence.to_str().unwrap(),
        ],
        &url,
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Result:     matched"), "{output}");
    let bound = ChangeSpec::load(&bound_path).unwrap();
    assert_eq!(bound.id().unwrap(), world.bound_spec().id().unwrap());
    assert_eq!(
        evidence(&bind_evidence).bound_change_spec_id,
        Some(bound.id().unwrap())
    );

    let verify_evidence = scratch.path().join("verify.json");
    let verify = [
        "governance",
        "squads",
        "verify",
        "--change-spec",
        bound_path.to_str().unwrap(),
        "--evidence-out",
        verify_evidence.to_str().unwrap(),
        "--format",
        "json",
    ];
    let (code, output) = eplyx(&verify, &url);
    assert_eq!(code, 0, "{output}");
    let first = evidence(&verify_evidence);
    assert_eq!(first.outcome, BindingOutcome::Matched);
    assert_eq!(first.analysed_change_spec_id, bound.id().unwrap());

    // The buffer's authority rewrites it before execution.
    world.edit(|s| s.buffer_bytes = b"\x7fELF\x02\x01\x01 swapped in after review".to_vec());
    let (code, output) = eplyx(&verify, &url);
    assert_eq!(code, 1, "{output}");
    let second = evidence(&verify_evidence);
    assert_eq!(second.outcome, BindingOutcome::StaleArtifact);
    assert!(second.observation.slot > first.observation.slot);

    // Asking about another transaction with a bound spec is a different proposal.
    let (code, output) = eplyx(
        &[
            "governance",
            "squads",
            "verify",
            "--change-spec",
            bound_path.to_str().unwrap(),
            "--transaction-index",
            "41",
        ],
        &url,
    );
    assert_ne!(code, 0, "{output}");
}

#[test]
fn unsupported_and_unverifiable_proposals_have_their_own_exit_codes() {
    let scratch = tempfile::tempdir().unwrap();
    let analysed = write_spec(scratch.path(), "analysed.json", &World::analysed_spec());
    let index = simulated::TRANSACTION_INDEX.to_string();
    let multisig = simulated::multisig().to_string();
    let bind = |url: &str| {
        eplyx(
            &[
                "governance",
                "squads",
                "bind",
                "--multisig",
                &multisig,
                "--transaction-index",
                &index,
                "--change-spec",
                &analysed,
                "--out",
                scratch.path().join("never.json").to_str().unwrap(),
            ],
            url,
        )
    };

    let lookup = Arc::new(World::new());
    lookup.edit(|s| {
        s.message().address_table_lookups.push(AddressTableLookup {
            account_key: [5; 32],
            writable_indexes: vec![],
            readonly_indexes: vec![0],
        })
    });
    let (code, output) = bind(&serve(lookup));
    assert_eq!(code, 4, "{output}");
    assert!(output.contains("address_table_lookups"), "{output}");

    let down = Arc::new(World::new());
    down.edit(|s| s.fail = true);
    let (code, output) = bind(&serve(down));
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("rpc_unavailable"), "{output}");

    let foreign = Arc::new(World::new());
    foreign.edit(|s| s.programdata_authority = Some(simulated::outsider()));
    let (code, output) = bind(&serve(foreign));
    assert_eq!(code, 1, "{output}");
    // No bound spec is written for anything but a match.
    assert!(!scratch.path().join("never.json").exists());

    let cancelled = Arc::new(World::new());
    cancelled.edit(|s| s.proposal().status = ProposalStatus::Cancelled { timestamp: 9 });
    let (code, output) = bind(&serve(cancelled));
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Cancelled"), "{output}");

    // verify refuses a spec with no delivery rather than guessing a proposal.
    let (code, output) = eplyx(
        &["governance", "squads", "verify", "--change-spec", &analysed],
        "http://127.0.0.1:9",
    );
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("names no Squads delivery"), "{output}");
}

#[test]
fn acquire_stores_the_buffer_bytes_by_content_and_writes_the_unbound_spec() {
    let scratch = tempfile::tempdir().unwrap();
    let world = Arc::new(World::new());
    let url = serve(Arc::clone(&world));
    let store = scratch.path().join("store");
    let out = scratch.path().join("acquired.json");
    let (code, output) = eplyx(
        &[
            "governance",
            "squads",
            "acquire",
            "--multisig",
            &simulated::multisig().to_string(),
            "--transaction-index",
            &simulated::TRANSACTION_INDEX.to_string(),
            "--store",
            store.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ],
        &url,
    );
    assert_eq!(code, 0, "{output}");
    let spec = ChangeSpec::load(&out).unwrap();
    assert_eq!(spec, {
        let mut expected = World::analysed_spec();
        expected.change_spec_id = spec.change_spec_id.clone();
        expected
    });
    assert!(spec.delivery().is_none());
    let resolved = spec
        .resolve(eplyx_engine::change::CandidateSource::Store(&store))
        .unwrap();
    assert_eq!(resolved.bytes(), simulated::CANDIDATE_ELF);
}
