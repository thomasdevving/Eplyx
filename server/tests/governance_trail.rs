//! Product-layer tests over the existing simulated Squads verifier/attester.
use anyhow::Result;
use base64::Engine;
use eplyx_engine::{
    change::ChangeSpec,
    governance::{
        self,
        attestation::{attest_squads_upgrade, DeploymentAttestation},
        simulated, squads, BindingOutcome, Commitment, GovernanceBinding, SquadsProposalRef,
    },
    ingest::rpc::RpcProvider,
    replay::hash_bytes,
    standard_programs::upgradeable_loader::{self as loader, encode},
};
use eplyx_server::{registry::Registry, storage::Storage};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PROJECT: &str = "proj_trail";
fn registry() -> (tempfile::TempDir, Registry) {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::new(Storage::open(dir.path()).unwrap());
    (dir, registry)
}
fn check(world: &simulated::World, spec: &ChangeSpec) -> GovernanceBinding {
    governance::verify_squads_upgrade(
        world,
        &SquadsProposalRef {
            multisig: simulated::multisig().to_string(),
            transaction_index: simulated::TRANSACTION_INDEX,
        },
        spec,
        Commitment::Finalized,
    )
    .unwrap()
}
fn page(registry: &Registry, spec: &ChangeSpec, cursor: Option<&str>, limit: usize) -> Value {
    serde_json::to_value(
        registry
            .governance_trail(PROJECT, spec, cursor, None, limit)
            .unwrap(),
    )
    .unwrap()
}
fn proof_path(
    registry: &Registry,
    spec: &ChangeSpec,
    proof: &DeploymentAttestation,
) -> std::path::PathBuf {
    registry
        .storage()
        .project_governance_dir(PROJECT, &spec.id().unwrap())
        .unwrap()
        .join("attestations")
        .join(format!("{}.json", proof.attestation_id.as_ref().unwrap()))
}

#[test]
fn mixed_history_pages_past_twenty_without_rpc_and_legacy_bytes_survive() {
    let (_dir, r) = registry();
    let world = simulated::World::new();
    let binding = check(&world, &world.spec());
    let spec = binding.bound_spec(&world.spec()).unwrap().unwrap();
    r.save_governance_spec(PROJECT, &spec).unwrap();
    r.record_governance_check(PROJECT, &binding).unwrap();
    let proof = attest_squads_upgrade(&world, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
    // A retained pre-occurrence G2 file. GET must not migrate or rewrite it.
    let path = proof_path(&r, &spec, &proof);
    let bytes = proof.to_document().unwrap().into_bytes();
    r.storage().write_bytes(&path, &bytes).unwrap();
    let legacy = page(&r, &spec, None, 20);
    assert_eq!(legacy["events"][1]["legacy"], true);
    assert!(legacy["events"][1]["recorded_at_unix_seconds"].is_null());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let dir = r
        .storage()
        .project_governance_dir(PROJECT, &spec.id().unwrap())
        .unwrap();
    assert!(!dir.join("attestation-occurrences").exists());
    // Later hosted requests for this identical seal retain legacy provenance.
    let mut expected = vec![legacy["events"][0]["event_id"].as_str().unwrap().to_owned()];
    for _ in 0..24 {
        let check = r
            .record_governance_check(PROJECT, &binding)
            .unwrap()
            .pop()
            .unwrap();
        expected.push(check.check_id);
        let o = r.record_deployment_attestation(PROJECT, &proof).unwrap();
        expected.push(o.occurrence_id);
    }
    expected.push(legacy["events"][1]["event_id"].as_str().unwrap().into());
    let reads = world.reads().len();
    let mut cursor = None;
    let mut found = vec![];
    loop {
        let p = page(&r, &spec, cursor.as_deref(), 7);
        assert_eq!(p, page(&r, &spec, cursor.as_deref(), 7));
        for event in p["events"].as_array().unwrap() {
            found.push(event["event_id"].as_str().unwrap().to_owned());
        }
        cursor = p["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(found, expected);
    assert_eq!(found.len(), 50);
    assert_eq!(
        found.iter().collect::<std::collections::HashSet<_>>().len(),
        50
    );
    assert_eq!(world.reads().len(), reads);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn later_stale_check_does_not_replace_the_binding_used_by_g2() {
    let (_dir, r) = registry();
    let world = simulated::World::new();
    let a = check(&world, &world.spec());
    let spec = a.bound_spec(&world.spec()).unwrap().unwrap();
    r.record_governance_check(PROJECT, &a).unwrap();
    world.edit(|s| s.buffer_bytes.push(99));
    let b = check(&world, &spec);
    assert_eq!(b.outcome, BindingOutcome::StaleArtifact);
    r.record_governance_check(PROJECT, &b).unwrap();
    let proof = attest_squads_upgrade(&world, &spec, &a, simulated::CANDIDATE_ELF).unwrap();
    r.record_deployment_attestation(PROJECT, &proof).unwrap();
    let p = page(&r, &spec, None, 20);
    assert_eq!(p["events"][0]["binding"]["outcome"], "matched");
    assert_eq!(p["events"][1]["binding"]["outcome"], "stale_artifact");
    assert_eq!(
        p["events"][2]["attestation"]["binding_id"],
        a.binding_id.unwrap()
    );
    assert_eq!(p["events"][2]["attestation"]["outcome"], "not_executed");
    assert!(p["events"][2]["attestation"]["execution"].is_null());
}

// Reseal deliberate inconsistent statements to exercise cross-object identity,
// independently of the sealed-content hash check.
fn reseal(mut proof: DeploymentAttestation) -> DeploymentAttestation {
    proof.attestation_id = None;
    proof.attestation_id = Some(hash_bytes(
        &serde_json::to_vec(&("eplyx-squads-deployment-attestation-v1", &proof)).unwrap(),
    ));
    proof
}

#[test]
fn tampered_indexes_seals_and_resealed_cross_object_identities_fail_closed() {
    for mutation in [
        "check",
        "occurrence",
        "binding_other_change",
        "candidate",
        "target",
        "proposal",
        "message",
        "seal",
        "binding_seal",
    ] {
        let (_dir, r) = registry();
        let world = simulated::World::new();
        let binding = check(&world, &world.spec());
        let spec = binding.bound_spec(&world.spec()).unwrap().unwrap();
        let checks = r.record_governance_check(PROJECT, &binding).unwrap();
        let mut proof =
            attest_squads_upgrade(&world, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
        let root = spec.id().unwrap();
        let dir = r.storage().project_governance_dir(PROJECT, &root).unwrap();
        let mut other_spec = world.spec();
        other_spec.activation = Some(eplyx_engine::change::Activation {
            slot: Some(123),
            unix_timestamp: None,
        });
        let other = check(&world, &other_spec);
        let other_id = other.binding_id.as_ref().unwrap();
        r.storage()
            .write_bytes(
                &dir.join("bindings").join(format!("{other_id}.json")),
                other.to_document().unwrap().as_bytes(),
            )
            .unwrap();
        match mutation {
            "binding_other_change" => proof.binding_id = other_id.clone(),
            "candidate" => proof.candidate.sha256 = "a".repeat(64),
            "target" => proof.target_program = simulated::outsider().to_string(),
            "proposal" => proof.proposal = simulated::outsider().to_string(),
            "message" => proof.message_sha256 = "b".repeat(64),
            _ => {}
        }
        proof = reseal(proof);
        let mut occurrence = r.record_deployment_attestation(PROJECT, &proof).unwrap();
        match mutation {
            "check" => {
                let mut c = checks.last().unwrap().clone();
                c.binding_id = other_id.clone();
                r.storage()
                    .write_json(&dir.join("checks").join(format!("{}.json", c.check_id)), &c)
                    .unwrap();
            }
            "occurrence" => {
                // A valid different proof is present, but uses a different binding.
                let mut other_proof = proof.clone();
                other_proof.binding_id = other_id.clone();
                other_proof = reseal(other_proof);
                r.storage()
                    .write_bytes(
                        &proof_path(&r, &spec, &other_proof),
                        other_proof.to_document().unwrap().as_bytes(),
                    )
                    .unwrap();
                occurrence.attestation_id = other_proof.attestation_id.unwrap();
                r.storage()
                    .write_json(
                        &dir.join("attestation-occurrences")
                            .join(format!("{}.json", occurrence.occurrence_id)),
                        &occurrence,
                    )
                    .unwrap();
            }
            "seal" => {
                let path = proof_path(&r, &spec, &proof);
                proof.observed_slot = Some(1);
                r.storage().write_json(&path, &proof).unwrap();
            }
            "binding_seal" => {
                let mut b = binding.clone();
                b.observation.slot = Some(1);
                r.storage()
                    .write_json(
                        &dir.join("bindings")
                            .join(format!("{}.json", b.binding_id.as_ref().unwrap())),
                        &b,
                    )
                    .unwrap();
            }
            _ => {}
        }
        assert!(
            r.governance_trail(PROJECT, &spec, None, None, 100).is_err(),
            "accepted {mutation}"
        );
    }
}

// Execution fixture mirrors the engine’s existing simulated G2 scenario.
struct Scenario {
    proposal: Value,
    vault_transaction: Value,
    program: Value,
    programdata: Value,
    transaction: Value,
    block: Value,
    signatures: Value,
}

fn account(owner: impl ToString, data: Vec<u8>, executable: bool) -> Value {
    json!({"owner": owner.to_string(), "lamports": 1_000_000, "executable": executable,
        "rentEpoch": u64::MAX, "data": [base64::prelude::BASE64_STANDARD.encode(data), "base64"]})
}

impl RpcProvider for Scenario {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        Ok(match method {
            "getMultipleAccounts" => {
                json!({"context":{"slot":7001}, "value":[self.program,self.programdata]})
            }
            "getAccountInfo" => {
                json!({"context":{"slot":7001}, "value":if params[0] == self.transaction["transaction"]["message"]["accountKeys"][3] { &self.vault_transaction } else { &self.proposal }})
            }
            "getSignaturesForAddress" => self.signatures.clone(),
            "getTransaction" => self.transaction.clone(),
            "getBlock" => self.block.clone(),
            _ => anyhow::bail!("unexpected RPC {method}"),
        })
    }
}

fn setup() -> (ChangeSpec, GovernanceBinding, Scenario) {
    let world = simulated::World::new();
    let analysed = world.spec();
    let binding = governance::verify_squads_upgrade(
        &world,
        &SquadsProposalRef {
            multisig: simulated::multisig().to_string(),
            transaction_index: simulated::TRANSACTION_INDEX,
        },
        &analysed,
        Commitment::Finalized,
    )
    .unwrap();
    assert_eq!(binding.outcome, BindingOutcome::Matched);
    let spec = binding.bound_spec(&analysed).unwrap().unwrap();
    let delivery = binding.observation.delivery.as_ref().unwrap();
    let upgrade = binding.observation.upgrade.as_ref().unwrap();
    let mut proposal = world.state().proposal().clone();
    proposal.status = squads::ProposalStatus::Executed {
        timestamp: 1_790_000_100,
    };
    let proposal_account = account(
        squads::program(),
        simulated::anchor_account(squads::PROPOSAL_DISCRIMINATOR, &proposal, 96),
        false,
    );
    let vault_transaction_account = account(
        squads::program(),
        simulated::anchor_account(
            squads::VAULT_TRANSACTION_DISCRIMINATOR,
            world.state().transaction(),
            0,
        ),
        false,
    );
    let program_account = account(
        loader::id(),
        encode::program(&upgrade.programdata.parse().unwrap()),
        true,
    );
    let programdata_account = account(
        loader::id(),
        encode::programdata(7000, Some(simulated::vault()), simulated::CANDIDATE_ELF),
        false,
    );
    let message = binding.observation.message.as_ref().unwrap();
    let mut keys = vec![
        squads::SQUADS_V4_PROGRAM_ID.to_string(),
        delivery.multisig.clone(),
        delivery.proposal.clone(),
        delivery.transaction.clone(),
        simulated::member(1).to_string(),
    ];
    for key in &message.account_keys {
        if !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    let index = |key: &str| keys.iter().position(|k| k == key).unwrap();
    let top_accounts: Vec<usize> = [
        &delivery.multisig,
        &delivery.proposal,
        &delivery.transaction,
        &simulated::member(1).to_string(),
    ]
    .iter()
    .map(|key| index(key))
    .chain(message.account_keys.iter().map(|key| index(key)))
    .collect();
    let loader_accounts: Vec<usize> = message.instructions[0]
        .account_indexes
        .iter()
        .map(|i| index(&message.account_keys[*i as usize]))
        .collect();
    let tx = json!({"slot":7000, "meta":{"err":null,"innerInstructions":[{"index":0,"instructions":[
        {"programIdIndex":index(&loader::id().to_string()),"accounts":loader_accounts,"data":bs58::encode(encode::upgrade()).into_string(),"stackHeight":2}
    ]}]},"transaction":{"message":{"accountKeys":keys,"recentBlockhash":"blockhash","instructions":[
        {"programIdIndex":0,"accounts":top_accounts,"data":bs58::encode(Sha256::digest(b"global:vault_transaction_execute")[..8].to_vec()).into_string()}
    ]},"signatures":["execution"]}});
    let block_keys: Vec<Value> = keys.iter().map(|key| json!({"pubkey":key,"writable": key == &upgrade.program || key == &upgrade.programdata})).collect();
    let block = json!({"transactions":[{"transaction":{"signatures":["execution"],"accountKeys":block_keys},"meta":{"err":null}}]});
    let scenario = Scenario {
        proposal: proposal_account,
        vault_transaction: vault_transaction_account,
        program: program_account,
        programdata: programdata_account,
        transaction: tx,
        block,
        signatures: json!([{"signature":"execution","err":null}]),
    };
    (spec, binding, scenario)
}

#[test]
fn execution_attribution_match_mismatch_and_supersession_stay_separate_observations() {
    let (_dir, r) = registry();
    let (spec, binding, mut rpc) = setup();
    r.record_governance_check(PROJECT, &binding).unwrap();
    let record = |expected: &str| {
        let a = attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
        assert_eq!(serde_json::to_value(a.outcome).unwrap(), expected);
        r.record_deployment_attestation(PROJECT, &a).unwrap();
        a
    };
    let matched = record("deployed_match");
    assert!(matched.execution.is_some());
    assert_eq!(
        matched.deployed.as_ref().unwrap().prefix_matches,
        Some(true)
    );
    rpc.programdata = account(
        loader::id(),
        encode::programdata(7000, Some(simulated::vault()), b"wrong bytes"),
        false,
    );
    let mismatch = attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
    assert_eq!(
        serde_json::to_value(mismatch.outcome).unwrap(),
        "deployed_mismatch"
    );
    r.record_deployment_attestation(PROJECT, &mismatch).unwrap();
    rpc.programdata = account(
        loader::id(),
        encode::programdata(7002, Some(simulated::vault()), b"later bytes"),
        false,
    );
    let superseded =
        attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
    assert_eq!(
        serde_json::to_value(superseded.outcome).unwrap(),
        "superseded"
    );
    r.record_deployment_attestation(PROJECT, &superseded)
        .unwrap();
    rpc.programdata = account(
        loader::id(),
        encode::programdata(7000, Some(simulated::vault()), simulated::CANDIDATE_ELF),
        false,
    );
    rpc.block = Value::Null;
    let unprovable =
        attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
    assert_eq!(
        serde_json::to_value(unprovable.outcome).unwrap(),
        "unverifiable"
    );
    assert!(unprovable.execution.is_some());
    assert_eq!(unprovable.deployed.as_ref().unwrap().prefix_matches, None);
    r.record_deployment_attestation(PROJECT, &unprovable)
        .unwrap();
    // Executed Proposal status without a matching transaction is still not proof.
    rpc.transaction = Value::Null;
    let no_execution =
        attest_squads_upgrade(&rpc, &spec, &binding, simulated::CANDIDATE_ELF).unwrap();
    assert!(no_execution.execution.is_none());
    r.record_deployment_attestation(PROJECT, &no_execution)
        .unwrap();
    let p = page(&r, &spec, None, 20);
    let events = p["events"].as_array().unwrap();
    assert_eq!(events.len(), 6);
    assert_eq!(events[0]["binding"]["outcome"], "matched");
    for (i, outcome) in [
        "deployed_match",
        "deployed_mismatch",
        "superseded",
        "unverifiable",
        "unverifiable",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(events[i + 1]["attestation"]["outcome"], *outcome);
        assert_eq!(
            events[i + 1]["attestation"]["binding_id"],
            binding.binding_id.as_ref().unwrap().as_str()
        );
    }
    assert_eq!(
        events[1]["attestation"],
        serde_json::to_value(matched).unwrap()
    );
}
