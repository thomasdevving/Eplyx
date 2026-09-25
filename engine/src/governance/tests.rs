//! Every mutation a Squads binding must catch, driven through the real reads,
//! decoders and checks against [`simulated::World`].

use super::simulated::{self, World};
use super::squads::{self, *};
use super::*;
use crate::change::Change;
use crate::standard_programs::upgradeable_loader::{self as loader, encode};

fn request() -> SquadsProposalRef {
    SquadsProposalRef {
        multisig: simulated::multisig().to_string(),
        transaction_index: simulated::TRANSACTION_INDEX,
    }
}

fn check(world: &World, spec: &ChangeSpec) -> GovernanceBinding {
    verify_squads_upgrade(world, &request(), spec, Commitment::Finalized).expect("a binding")
}

fn codes(binding: &GovernanceBinding) -> Vec<&str> {
    binding.reasons.iter().map(|r| r.code.as_str()).collect()
}

fn assert_outcome(binding: &GovernanceBinding, outcome: BindingOutcome, code: &str) {
    assert_eq!(binding.outcome, outcome, "{:#?}", binding.reasons);
    assert!(
        codes(binding).contains(&code),
        "expected reason {code}, got {:?}",
        codes(binding)
    );
}

// ------------------------------------------------------------------ matched

#[test]
fn a_healthy_proposal_matches_and_yields_the_bound_spec() {
    let world = World::new();
    let analysed = World::analysed_spec();
    let binding = check(&world, &analysed);
    assert_eq!(
        binding.outcome,
        BindingOutcome::Matched,
        "{:#?}",
        binding.reasons
    );
    assert!(binding.reasons.is_empty());

    // The observation is complete and at one slot.
    let observed = &binding.observation;
    let slot = observed.slot.expect("slot");
    assert!(slot > observed.message_read_slot.unwrap());
    assert_eq!(observed.accounts.len(), 6);
    assert_eq!(
        observed.buffer.as_ref().unwrap().artifact,
        ExecutableArtifact::of(simulated::CANDIDATE_ELF)
    );
    assert_eq!(
        observed.proposal.as_ref().unwrap().status,
        ProposalStatusKind::Active
    );
    let upgrade = observed.upgrade.as_ref().unwrap();
    assert_eq!(upgrade.authority, simulated::vault().to_string());
    assert_eq!(upgrade.buffer, simulated::buffer().to_string());
    assert!(binding.statement.contains(&format!("at slot {slot}")));
    assert!(binding.statement.contains("Re-verify"));
    assert!(!binding.statement.to_lowercase().contains("never diverge"));

    // Unbound input: the binding names the governance-bound identity, which
    // is a different change, and hands back exactly that spec.
    let bound = binding.bound_spec(&analysed).unwrap().expect("bound spec");
    assert_eq!(Some(bound.id().unwrap()), binding.bound_change_spec_id);
    assert_ne!(bound.id().unwrap(), analysed.id().unwrap());
    assert_eq!(bound, world.bound_spec());

    // The bound spec matches too, and now both identities agree.
    let rebound = check(&world, &bound);
    assert_eq!(
        rebound.outcome,
        BindingOutcome::Matched,
        "{:#?}",
        rebound.reasons
    );
    assert_eq!(
        rebound.bound_change_spec_id.as_deref(),
        Some(rebound.analysed_change_spec_id.as_str())
    );
}

#[test]
fn stated_target_expectations_are_proved_consistently() {
    let world = World::new();
    let mut spec = World::analysed_spec();
    let Change::ProgramUpgrade {
        target,
        expected_upgrade_authority,
        replaces,
        ..
    } = &mut spec.change;
    target.programdata_address =
        Some(loader::programdata_address(&simulated::program()).to_string());
    *expected_upgrade_authority = Some(simulated::vault().to_string());
    *replaces = Some(ExecutableArtifact::of(simulated::DEPLOYED_ELF));
    assert_eq!(check(&world, &spec).outcome, BindingOutcome::Matched);

    let mut other_authority = spec.clone();
    let Change::ProgramUpgrade {
        expected_upgrade_authority,
        ..
    } = &mut other_authority.change;
    *expected_upgrade_authority = Some(simulated::outsider().to_string());
    assert_outcome(
        &check(&world, &other_authority),
        BindingOutcome::DifferentProposal,
        "expected_authority_differs",
    );

    let mut other_baseline = spec;
    let Change::ProgramUpgrade { replaces, .. } = &mut other_baseline.change;
    *replaces = Some(ExecutableArtifact::of(b"\x7fELF some other deployment"));
    assert_outcome(
        &check(&world, &other_baseline),
        BindingOutcome::DifferentProposal,
        "replaced_executable_differs",
    );
}

// ------------------------------------------------------- 1-5: identity

#[test]
fn m01_a_different_multisig_is_a_different_proposal() {
    let bound = World::new().bound_spec();
    // The same message at the same index, in another multisig.
    let other = World::new();
    let mut create = [0u8; 32];
    create[0] = 99;
    let other_create = solana_address::Address::from(create);
    let other_multisig = squads::multisig_address(&other_create).0;
    let request = SquadsProposalRef {
        multisig: other_multisig.to_string(),
        transaction_index: simulated::TRANSACTION_INDEX,
    };
    let binding = verify_squads_upgrade(&other, &request, &bound, Commitment::Finalized).unwrap();
    assert!(binding.outcome < BindingOutcome::StaleArtifact);
    assert!(codes(&binding).contains(&"proposal_reference_differs"));
    assert_ne!(binding.outcome, BindingOutcome::Matched);
}

#[test]
fn m02_a_different_transaction_index_is_a_different_proposal() {
    let world = World::new();
    let bound = world.bound_spec();
    // Transaction 43 carries the identical message: still not the proposal the
    // analysis is bound to.
    world.edit(|s| {
        let message = s.message().clone();
        s.add_transaction(43, message);
    });
    let request = SquadsProposalRef {
        transaction_index: 43,
        ..request()
    };
    let binding = verify_squads_upgrade(&world, &request, &bound, Commitment::Finalized).unwrap();
    assert_outcome(
        &binding,
        BindingOutcome::DifferentProposal,
        "proposal_reference_differs",
    );
    // And an index that was never created is unverifiable, not matched.
    let missing = SquadsProposalRef {
        transaction_index: 44,
        ..request
    };
    let binding = verify_squads_upgrade(
        &world,
        &missing,
        &World::analysed_spec(),
        Commitment::Finalized,
    )
    .unwrap();
    assert_outcome(&binding, BindingOutcome::Unverifiable, "account_missing");
}

#[test]
fn m03_a_wrong_transaction_pda_is_refused() {
    // A spec naming a transaction address that is not the derivation.
    let mut spec = World::new().bound_spec();
    let Change::ProgramUpgrade {
        delivery: Some(Delivery::SquadsV4(delivery)),
        ..
    } = &mut spec.change
    else {
        unreachable!()
    };
    delivery.transaction = squads::transaction_address(&simulated::multisig(), 41)
        .0
        .to_string();
    assert!(spec.validate().is_err());
    assert!(ChangeSpec::parse(serde_json::to_string(&spec).unwrap().as_bytes()).is_err());

    // An account at the derived address that claims another index or bump.
    let world = World::new();
    world.edit(|s| s.transaction().index = 41);
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "index_mismatch",
    );
    let world = World::new();
    world.edit(|s| s.transaction().bump = s.transaction().bump.wrapping_sub(1));
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "pda_mismatch",
    );
}

#[test]
fn m04_a_wrong_proposal_pda_is_refused() {
    let mut spec = World::new().bound_spec();
    let Change::ProgramUpgrade {
        delivery: Some(Delivery::SquadsV4(delivery)),
        ..
    } = &mut spec.change
    else {
        unreachable!()
    };
    delivery.proposal = squads::proposal_address(&simulated::multisig(), 43)
        .0
        .to_string();
    assert!(spec.validate().is_err());

    let world = World::new();
    world.edit(|s| s.proposal().transaction_index = 43);
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "index_mismatch",
    );
    let world = World::new();
    world.edit(|s| {
        s.proposals.remove(&simulated::TRANSACTION_INDEX);
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "account_missing",
    );
}

#[test]
fn m05_transaction_and_proposal_must_belong_to_the_multisig() {
    let world = World::new();
    world.edit(|s| s.proposal().multisig = simulated::outsider().to_bytes());
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "multisig_mismatch",
    );
    let world = World::new();
    world.edit(|s| s.transaction().multisig = simulated::outsider().to_bytes());
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "multisig_mismatch",
    );
}

// ------------------------------------------------------- 6-8: target

#[test]
fn m06_a_different_target_program_is_a_different_proposal() {
    let world = World::new();
    let spec =
        ChangeSpec::program_upgrade(&simulated::outsider().to_string(), simulated::CANDIDATE_ELF);
    assert_outcome(
        &check(&world, &spec),
        BindingOutcome::DifferentProposal,
        "target_program_differs",
    );
}

#[test]
fn m07_a_different_programdata_is_refused() {
    let world = World::new();
    let mut spec = World::analysed_spec();
    let Change::ProgramUpgrade { target, .. } = &mut spec.change;
    target.programdata_address = Some(simulated::outsider().to_string());
    assert_outcome(
        &check(&world, &spec),
        BindingOutcome::DifferentProposal,
        "programdata_differs",
    );

    // A message whose ProgramData is not the loader derivation cannot execute.
    let world = World::new();
    world.edit(|s| s.message().account_keys[1] = simulated::outsider().to_bytes());
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::UnsupportedProposal,
        "programdata_not_derived",
    );
}

#[test]
fn m08_a_different_buffer_address_is_a_different_message() {
    let bound = World::new().bound_spec();
    let world = World::new();
    let other_buffer = simulated::outsider();
    world.edit(|s| {
        s.message().account_keys[3] = other_buffer.to_bytes();
        let account = s.accounts().get(&simulated::buffer()).cloned().flatten();
        s.overrides.insert(other_buffer, account);
    });
    let binding = check(&world, &bound);
    assert_outcome(
        &binding,
        BindingOutcome::DifferentProposal,
        "message_differs",
    );
    assert_eq!(
        binding.observation.upgrade.as_ref().unwrap().buffer,
        other_buffer.to_string()
    );
}

// ------------------------------------------------------- 9-11: artefact

#[test]
fn m09_a_rewritten_buffer_is_a_stale_artifact() {
    let world = World::new();
    let bound = world.bound_spec();
    world.edit(|s| s.buffer_bytes = b"\x7fELF\x02\x01\x01 rewritten after analysis".to_vec());
    let binding = check(&world, &bound);
    assert_outcome(&binding, BindingOutcome::StaleArtifact, "candidate_differs");
    assert!(binding.statement.contains("no longer matches"));
    // The message did not change: the identity it carries is still the bound one.
    assert_eq!(binding.bound_change_spec_id, Some(bound.id().unwrap()));
}

#[test]
fn m10_a_buffer_authority_outside_the_vault_is_never_bound() {
    let world = World::new();
    world.edit(|s| s.buffer_authority = Some(simulated::outsider()));
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::AuthorityMismatch,
        "buffer_authority_not_vault",
    );
    let world = World::new();
    world.edit(|s| s.buffer_authority = None);
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::AuthorityMismatch,
        "buffer_authority_not_vault",
    );
}

#[test]
fn m11_an_upgrade_authority_outside_the_vault_is_never_bound() {
    let world = World::new();
    world.edit(|s| s.programdata_authority = Some(simulated::outsider()));
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::AuthorityMismatch,
        "upgrade_authority_not_vault",
    );
    let world = World::new();
    world.edit(|s| s.programdata_authority = None);
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::AuthorityMismatch,
        "upgrade_authority_not_vault",
    );
}

// ------------------------------------------------------- 12-16: shape

fn system_transfer(message: &mut VaultTransactionMessage) {
    // Append the System program as a readonly key and a transfer from the vault.
    message
        .account_keys
        .push(solana_address::Address::default().to_bytes());
    let system = (message.account_keys.len() - 1) as u8;
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend(1_000_000u64.to_le_bytes());
    message.instructions.push(CompiledInstruction {
        program_id_index: system,
        account_indexes: vec![0, 4],
        data,
    });
}

#[test]
fn m12_an_upgrade_plus_a_transfer_is_unsupported() {
    let world = World::new();
    world.edit(|s| system_transfer(s.message()));
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::UnsupportedProposal,
        "instruction_count",
    );
}

#[test]
fn m13_two_upgrades_are_unsupported() {
    let world = World::new();
    world.edit(|s| {
        let upgrade = s.message().instructions[0].clone();
        s.message().instructions.push(upgrade);
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::UnsupportedProposal,
        "instruction_count",
    );
    // An authority change alongside the upgrade, likewise.
    let world = World::new();
    world.edit(|s| {
        let mut set_authority = s.message().instructions[0].clone();
        set_authority.data = encode::instruction(
            &solana_loader_v3_interface::instruction::UpgradeableLoaderInstruction::SetAuthority,
        );
        set_authority.account_indexes = vec![1, 0];
        s.message().instructions.push(set_authority);
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::UnsupportedProposal,
        "instruction_count",
    );
}

#[test]
fn m14_other_loader_instructions_are_unsupported() {
    use solana_loader_v3_interface::instruction::UpgradeableLoaderInstruction as I;
    for (instruction, code) in [
        (I::SetAuthority, "not_upgrade_instruction"),
        (I::Close { tombstone: false }, "not_upgrade_instruction"),
        (
            I::ExtendProgram {
                additional_bytes: 8,
            },
            "not_upgrade_instruction",
        ),
        (
            I::Upgrade {
                close_buffer: false,
            },
            "upgrade_keeps_buffer",
        ),
    ] {
        let world = World::new();
        world.edit(|s| s.message().instructions[0].data = encode::instruction(&instruction));
        assert_outcome(
            &check(&world, &World::analysed_spec()),
            BindingOutcome::UnsupportedProposal,
            code,
        );
    }
    // The explicit close_buffer = true form means what the legacy form means.
    let world = World::new();
    world.edit(|s| {
        s.message().instructions[0].data = encode::instruction(&I::Upgrade { close_buffer: true })
    });
    assert_eq!(
        check(&world, &World::analysed_spec()).outcome,
        BindingOutcome::Matched
    );
    // Account order is the loader's, not a heuristic over names: swap buffer
    // and spill and the spill account *is* the buffer, which does not exist.
    let world = World::new();
    world.edit(|s| s.message().instructions[0].account_indexes.swap(2, 3));
    let swapped = check(&world, &World::analysed_spec());
    assert_outcome(&swapped, BindingOutcome::Unverifiable, "buffer_missing");
    assert_eq!(
        swapped.observation.upgrade.as_ref().unwrap().buffer,
        simulated::spill().to_string()
    );
    // A sysvar where ProgramData belongs breaks the loader's writability contract.
    let world = World::new();
    world.edit(|s| s.message().instructions[0].account_indexes.swap(0, 4));
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::UnsupportedProposal,
        "upgrade_account_shape",
    );
    // A readonly buffer cannot be consumed.
    let world = World::new();
    world.edit(|s| s.message().num_writable_non_signers = 2);
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::UnsupportedProposal,
        "upgrade_account_shape",
    );
    // A second signer the vault does not control.
    let world = World::new();
    world.edit(|s| {
        let message = s.message();
        message
            .account_keys
            .insert(1, simulated::outsider().to_bytes());
        message.num_signers = 2;
        for ix in &mut message.instructions {
            ix.program_id_index += 1;
            for i in &mut ix.account_indexes {
                if *i >= 1 {
                    *i += 1;
                }
            }
        }
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::UnsupportedProposal,
        "signer_shape",
    );
}

#[test]
fn m15_an_altered_message_is_a_different_proposal_whatever_the_memo() {
    // Squads logs a memo and never stores it; the message is the only identity.
    let world = World::new();
    let bound = world.bound_spec();
    world.edit(|s| s.message().account_keys[4] = simulated::outsider().to_bytes());
    let binding = check(&world, &bound);
    assert_outcome(
        &binding,
        BindingOutcome::DifferentProposal,
        "message_differs",
    );
    // Unbound, the altered proposal still binds on its own terms — to a
    // different identity than the original.
    let altered = check(&world, &World::analysed_spec());
    assert_eq!(altered.outcome, BindingOutcome::Matched);
    assert_ne!(altered.bound_change_spec_id, Some(bound.id().unwrap()));
}

#[test]
fn m16_lookup_tables_are_unsupported_and_part_of_identity() {
    let with_table = |table: u8| {
        let world = World::new();
        world.edit(|s| {
            s.message().address_table_lookups.push(AddressTableLookup {
                account_key: [table; 32],
                writable_indexes: vec![],
                readonly_indexes: vec![0],
            })
        });
        world
    };
    let a = check(&with_table(1), &World::analysed_spec());
    assert_outcome(
        &a,
        BindingOutcome::UnsupportedProposal,
        "address_table_lookups",
    );
    let b = check(&with_table(2), &World::analysed_spec());
    assert_ne!(
        a.observation.delivery.unwrap().message_sha256,
        b.observation.delivery.unwrap().message_sha256,
        "a changed lookup table must change the message identity"
    );
}

// ------------------------------------------------------- 17: status

#[test]
fn m17_status_is_reported_and_never_identity() {
    let world = World::new();
    let bound = world.bound_spec();
    let active = check(&world, &bound);
    for (status, kind) in [
        (
            ProposalStatus::Approved { timestamp: 1 },
            ProposalStatusKind::Approved,
        ),
        (
            ProposalStatus::Cancelled { timestamp: 2 },
            ProposalStatusKind::Cancelled,
        ),
        (
            ProposalStatus::Rejected { timestamp: 3 },
            ProposalStatusKind::Rejected,
        ),
        (
            ProposalStatus::Draft { timestamp: 4 },
            ProposalStatusKind::Draft,
        ),
    ] {
        world.edit(|s| s.proposal().status = status.clone());
        let binding = check(&world, &bound);
        assert_eq!(binding.outcome, BindingOutcome::Matched);
        assert_eq!(binding.observation.proposal.as_ref().unwrap().status, kind);
        assert_eq!(binding.bound_change_spec_id, active.bound_change_spec_id);
        assert_eq!(world.bound_spec().id().unwrap(), bound.id().unwrap());
        assert!(
            binding.statement.contains(status_word(kind)),
            "{}",
            binding.statement
        );
    }
    // A stale Active proposal is reported as stale.
    world.edit(|s| {
        s.proposal().status = ProposalStatus::Active { timestamp: 5 };
        s.multisig.stale_transaction_index = simulated::TRANSACTION_INDEX;
    });
    let stale = check(&world, &bound);
    assert!(stale.observation.proposal.as_ref().unwrap().stale);
    assert!(stale.statement.contains("stale"));

    // Executed: the buffer is gone, which is said, not guessed around.
    world.edit(|s| {
        s.proposal().status = ProposalStatus::Executed { timestamp: 6 };
        s.buffer_exists = false;
    });
    let executed = check(&world, &bound);
    assert_outcome(&executed, BindingOutcome::Unverifiable, "buffer_missing");
    assert!(executed.reasons[0].detail.contains("executed"));
}

// ------------------------------------------------------- 18: evidence

#[test]
fn m18_malformed_rpc_accounts_are_unverifiable() {
    let tx = squads::transaction_address(&simulated::multisig(), simulated::TRANSACTION_INDEX).0;
    let world = World::new();
    world.edit(|s| {
        let mut account = s.accounts()[&tx].clone().unwrap();
        account.owner = solana_address::Address::default().to_string();
        s.overrides.insert(tx, Some(account));
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "wrong_owner",
    );

    let world = World::new();
    world.edit(|s| {
        let mut account = s.accounts()[&tx].clone().unwrap();
        account.data[0] ^= 1;
        s.overrides.insert(tx, Some(account));
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "malformed_account",
    );

    let world = World::new();
    world.edit(|s| {
        let mut account = s.accounts()[&tx].clone().unwrap();
        account.data.push(1);
        s.overrides.insert(tx, Some(account));
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "malformed_account",
    );

    // A Proposal where a Multisig should be: right owner, wrong discriminator.
    let world = World::new();
    world.edit(|s| {
        let proposal = s.accounts()
            [&squads::proposal_address(&simulated::multisig(), simulated::TRANSACTION_INDEX).0]
            .clone();
        s.overrides.insert(simulated::multisig(), proposal);
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "malformed_account",
    );

    // A buffer that is not a loader Buffer.
    let world = World::new();
    world.edit(|s| {
        let programdata = s.accounts()[&loader::programdata_address(&simulated::program())].clone();
        s.overrides.insert(simulated::buffer(), programdata);
    });
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "buffer_malformed",
    );

    // A non-canonical vault bump would sign as some other address.
    let world = World::new();
    world.edit(|s| s.transaction().vault_bump = s.transaction().vault_bump.wrapping_sub(1));
    assert_outcome(
        &check(&world, &World::analysed_spec()),
        BindingOutcome::Unverifiable,
        "pda_mismatch",
    );

    // No transport at all.
    let world = World::new();
    world.edit(|s| s.fail = true);
    let binding = check(&world, &World::analysed_spec());
    assert_outcome(&binding, BindingOutcome::Unverifiable, "rpc_unavailable");
    assert_eq!(binding.observation.slot, None);
    assert!(binding.statement.contains("no chain read completed"));
}

#[test]
fn m19_tampered_evidence_is_refused() {
    let binding = check(&World::new(), &World::analysed_spec());
    let document = binding.to_document().unwrap();
    assert_eq!(
        GovernanceBinding::parse(document.as_bytes()).unwrap(),
        binding
    );

    let original: serde_json::Value = serde_json::from_str(&document).unwrap();
    for (pointer, value) in [
        ("/observation/slot", serde_json::json!(1)),
        (
            "/observation/buffer/artifact/sha256",
            serde_json::json!("00".repeat(32)),
        ),
        ("/outcome", serde_json::json!("matched")),
        ("/statement", serde_json::json!("everything is fine")),
        ("/bound_change_spec_id", serde_json::json!("11".repeat(32))),
    ] {
        let mut edited = original.clone();
        *edited.pointer_mut(pointer).unwrap() = value;
        if edited == original {
            continue;
        }
        assert!(
            GovernanceBinding::parse(edited.to_string().as_bytes()).is_err(),
            "{pointer} edit was accepted"
        );
    }
    // An unsealed record is not evidence.
    let mut unsealed = original.clone();
    unsealed.as_object_mut().unwrap().remove("binding_id");
    assert!(GovernanceBinding::parse(unsealed.to_string().as_bytes()).is_err());

    // Resealing a lie is still refused: the outcome must follow the reasons.
    let stale = {
        let world = World::new();
        world.edit(|s| s.buffer_bytes = b"\x7fELF other".to_vec());
        check(&world, &World::analysed_spec())
    };
    let mut forged = stale.clone();
    forged.outcome = BindingOutcome::Matched;
    let forged = forged.seal().unwrap();
    assert!(GovernanceBinding::parse(forged.to_document().unwrap().as_bytes()).is_err());
}

#[test]
fn m20_a_recheck_rereads_the_chain_and_never_returns_a_cached_match() {
    let world = World::new();
    let bound = world.bound_spec();
    let first = check(&world, &bound);
    assert_eq!(first.outcome, BindingOutcome::Matched);
    let reads = world.reads().len();

    world.edit(|s| s.buffer_bytes = b"\x7fELF\x02\x01\x01 swapped before execution".to_vec());
    let second = check(&world, &bound);
    assert_eq!(second.outcome, BindingOutcome::StaleArtifact);
    assert_eq!(
        world.reads().len(),
        reads + 2,
        "the recheck must read the chain again"
    );
    assert!(second.observation.slot > first.observation.slot);
    assert_ne!(second.binding_id, first.binding_id);
}

// ------------------------------------------------------- identity freezes

#[test]
fn unbound_specs_keep_their_c1_identity() {
    assert_eq!(
        ChangeSpec::program_upgrade(
            "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
            b"candidate elf"
        )
        .id()
        .unwrap(),
        "b5a894cdbec6251f73b4224a294579fe1af9e316232949fa92da852468900bf3"
    );
    let document = World::analysed_spec().to_document().unwrap();
    assert!(!document.contains("delivery"));
}

#[test]
fn the_squads_message_hash_is_frozen() {
    let world = World::new();
    let message = world.state().message().clone();
    // Encoded by hand, independently of the Borsh derive.
    let mut bytes = vec![1u8, 1, 4];
    bytes.extend(8u32.to_le_bytes());
    for key in &message.account_keys {
        bytes.extend(key);
    }
    bytes.extend(1u32.to_le_bytes());
    bytes.push(7);
    bytes.extend(7u32.to_le_bytes());
    bytes.extend([1, 2, 3, 4, 5, 6, 0]);
    bytes.extend(4u32.to_le_bytes());
    bytes.extend([3, 0, 0, 0]);
    bytes.extend(0u32.to_le_bytes());
    assert_eq!(borsh::to_vec(&message).unwrap(), bytes);
    assert_eq!(
        squads::message_hash(&message).unwrap(),
        FROZEN_MESSAGE_HASH,
        "the canonical message encoding moved: every governance-bound spec now names another proposal"
    );
}

pub(super) const FROZEN_MESSAGE_HASH: &str =
    "b8694179bdb02136eda583bd5d999023ca93fb92af4166b456ee0471437dcd86";

#[test]
fn a_bound_spec_round_trips_and_every_delivery_field_moves_its_id() {
    let world = World::new();
    let bound = world.bound_spec();
    let parsed = ChangeSpec::parse(bound.to_document().unwrap().as_bytes()).unwrap();
    assert_eq!(parsed.id().unwrap(), bound.id().unwrap());

    let Some(Delivery::SquadsV4(delivery)) = bound.delivery().cloned() else {
        unreachable!()
    };
    let mut ids = std::collections::BTreeSet::from([
        bound.id().unwrap(),
        World::analysed_spec().id().unwrap(),
    ]);
    let variants = [
        squads::derive_delivery(
            &simulated::multisig(),
            1,
            delivery.transaction_index,
            delivery.message_sha256.clone(),
        ),
        squads::derive_delivery(
            &simulated::multisig(),
            0,
            delivery.transaction_index + 1,
            delivery.message_sha256.clone(),
        ),
        squads::derive_delivery(
            &simulated::outsider(),
            0,
            delivery.transaction_index,
            delivery.message_sha256.clone(),
        ),
        squads::derive_delivery(
            &simulated::multisig(),
            0,
            delivery.transaction_index,
            "cd".repeat(32),
        ),
    ];
    for variant in variants {
        let spec = World::analysed_spec().with_delivery(Some(Delivery::SquadsV4(variant)));
        spec.validate().unwrap();
        assert!(ids.insert(spec.id().unwrap()));
    }

    // Unknown fields in a delivery are refused, as everywhere in a spec.
    let mut document: serde_json::Value =
        serde_json::from_str(&bound.to_document().unwrap()).unwrap();
    document["change"]["delivery"]["proposal_status"] = "active".into();
    assert!(ChangeSpec::parse(document.to_string().as_bytes()).is_err());
    let mut document: serde_json::Value =
        serde_json::from_str(&bound.to_document().unwrap()).unwrap();
    document["change"]["delivery"]["provider"] = "realms".into();
    assert!(ChangeSpec::parse(document.to_string().as_bytes()).is_err());
}

#[test]
fn the_bundle_binding_is_untouched_by_a_delivery() {
    use crate::change::{BaselineTarget, TargetEvidence, TargetLoader};
    let world = World::new();
    let bound = world.bound_spec();
    let baseline = BaselineTarget {
        program_id: simulated::program().to_string(),
        executable: ExecutableArtifact::of(simulated::DEPLOYED_ELF),
        loader: TargetEvidence::Proven(TargetLoader::Upgradeable),
        programdata_address: TargetEvidence::Unproven,
        upgrade_authorities: TargetEvidence::Unproven,
    };
    let binding = bound.bind(&baseline).unwrap();
    assert_eq!(binding.change_spec_id, bound.id().unwrap());
    assert_eq!(binding.delivery, bound.delivery().cloned());
    let unbound = World::analysed_spec().bind(&baseline).unwrap();
    assert!(!serde_json::to_string(&unbound)
        .unwrap()
        .contains("delivery"));
}

// ------------------------------------------------------------------ fixtures

/// Committed engine output: the stored vault transaction and one binding per
/// outcome. The frontend renders these, and
/// `scripts/verify-squads-binding.mjs` recomputes the message hash, the bound
/// change ID and every binding ID from them without this crate. Regenerate
/// with `make governance-fixtures`.
#[test]
fn governance_fixtures_are_current() {
    use base64::Engine;
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-g1-squads-binding");
    let write = std::env::var_os("EPLYX_WRITE_GOVERNANCE_FIXTURES").is_some();
    let expect = |name: &str, contents: String| {
        let path = directory.join(name);
        if write {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(&path, &contents).unwrap();
        } else {
            let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| {
                panic!(
                    "{} is missing; run make governance-fixtures",
                    path.display()
                )
            });
            assert_eq!(
                committed, contents,
                "{name} is stale; run make governance-fixtures"
            );
        }
    };

    let world = World::new();
    let analysed = World::analysed_spec();
    let bound = world.bound_spec();
    let transaction =
        squads::transaction_address(&simulated::multisig(), simulated::TRANSACTION_INDEX).0;
    let account = world.state().accounts()[&transaction].clone().unwrap();
    let Some(Delivery::SquadsV4(delivery)) = bound.delivery().cloned() else {
        unreachable!()
    };
    expect(
        "vault-transaction.json",
        serde_json::to_string_pretty(&serde_json::json!({
            "note": "Simulated Squads V4 VaultTransaction account (one loader Upgrade). Not a mainnet account.",
            "address": transaction.to_string(),
            "owner": account.owner,
            "data_base64": base64::prelude::BASE64_STANDARD.encode(&account.data),
            "message_hash_domain": squads::MESSAGE_HASH_DOMAIN,
            "message_sha256": delivery.message_sha256,
            "delivery": delivery,
        }))
        .unwrap()
            + "\n",
    );
    expect(
        "analysed-change-spec.json",
        analysed.to_document().unwrap() + "\n",
    );
    expect(
        "bound-change-spec.json",
        bound.to_document().unwrap() + "\n",
    );

    type Mutation = Box<dyn Fn(&World)>;
    let outcomes: [(&str, Mutation); 5] = [
        ("binding-matched.json", Box::new(|_| {})),
        (
            "binding-stale-artifact.json",
            Box::new(|w| {
                w.edit(|s| {
                    s.buffer_bytes = b"\x7fELF\x02\x01\x01 rewritten after analysis".to_vec()
                })
            }),
        ),
        (
            "binding-authority-mismatch.json",
            Box::new(|w| w.edit(|s| s.buffer_authority = Some(simulated::outsider()))),
        ),
        (
            "binding-unsupported.json",
            Box::new(|w| w.edit(|s| system_transfer(s.message()))),
        ),
        (
            "binding-cancelled.json",
            Box::new(|w| {
                w.edit(|s| {
                    s.proposal().status = ProposalStatus::Cancelled {
                        timestamp: 1_790_000_600,
                    }
                })
            }),
        ),
    ];
    for (name, mutate) in outcomes {
        let world = World::new();
        mutate(&world);
        let binding = check(&world, &bound);
        GovernanceBinding::parse(binding.to_document().unwrap().as_bytes()).unwrap();
        expect(name, binding.to_document().unwrap() + "\n");
    }
}

/// G1.1: a load-balanced mainnet endpoint answered the consistency read with
/// -32016 from a node behind the first read. That is waited out, still under
/// the `minContextSlot` floor; any other error is not retried.
#[test]
fn a_lagging_node_is_waited_for_and_never_accepted_older() {
    struct Lagging<'a> {
        world: &'a World,
        calls: std::sync::atomic::AtomicUsize,
        lag: usize,
        code: &'static str,
    }
    impl crate::ingest::rpc::RpcProvider for Lagging<'_> {
        fn call(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> anyhow::Result<serde_json::Value> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if params[1].get("minContextSlot").is_some() && n <= self.lag {
                anyhow::bail!(
                    "RPC {method} returned error code {}, transaction error null, program logs null",
                    self.code
                );
            }
            self.world.call(method, params)
        }
    }
    let world = World::new();
    let lagging = Lagging {
        world: &world,
        calls: 0.into(),
        lag: 2,
        code: "-32016",
    };
    let binding = verify_squads_upgrade(
        &lagging,
        &request(),
        &World::analysed_spec(),
        Commitment::Finalized,
    )
    .unwrap();
    assert_eq!(
        binding.outcome,
        BindingOutcome::Matched,
        "{:#?}",
        binding.reasons
    );
    assert!(binding.observation.slot >= binding.observation.message_read_slot);

    let world = World::new();
    let other = Lagging {
        world: &world,
        calls: 0.into(),
        lag: 2,
        code: "-32005",
    };
    let binding = verify_squads_upgrade(
        &other,
        &request(),
        &World::analysed_spec(),
        Commitment::Finalized,
    )
    .unwrap();
    assert_outcome(&binding, BindingOutcome::Unverifiable, "rpc_unavailable");
}
