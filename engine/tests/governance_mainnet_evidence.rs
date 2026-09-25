//! Phase G1.1: the committed real-mainnet Squads evidence, re-checked offline.
//!
//! `docs/examples/phase-g1-1-squads-mainnet/` holds raw mainnet account bytes
//! and the sealed bindings the `eplyx` binary produced against mainnet. This
//! decodes the real bytes through the production decoder and re-verifies every
//! sealed record, so a layout or identity regression is caught without a
//! network. It does not re-read the chain; `eplyx governance squads verify`
//! does that.

use std::path::{Path, PathBuf};

use base64::Engine;
use eplyx_engine::change::{CandidateSource, ChangeSpec, Delivery};
use eplyx_engine::governance::squads::{self, ProposalStatusKind};
use eplyx_engine::governance::{BindingOutcome, GovernanceBinding};
use solana_address::Address;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/examples/phase-g1-1-squads-mainnet")
}

fn binding(path: &Path) -> GovernanceBinding {
    GovernanceBinding::parse(&std::fs::read(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()))
}

fn raw(role: &str) -> (Address, Vec<u8>) {
    let file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("raw-accounts.json")).unwrap()).unwrap();
    let account = file["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["role"] == role)
        .unwrap_or_else(|| panic!("no {role}"));
    assert_eq!(account["owner"], squads::SQUADS_V4_PROGRAM_ID);
    (
        account["address"].as_str().unwrap().parse().unwrap(),
        base64::prelude::BASE64_STANDARD
            .decode(account["data_base64"].as_str().unwrap())
            .unwrap(),
    )
}

/// Real Multisig, VaultTransaction and Proposal bytes decode under the pinned
/// layouts, derive to their own addresses, and hash to the message the
/// mainnet binding sealed.
#[test]
fn real_squads_accounts_decode_and_derive() {
    for witness in ["primary", "second"] {
        let sealed = binding(&root().join(format!("witness-{witness}/bind.json")));
        let delivery = sealed.observation.delivery.as_ref().unwrap();

        let (multisig_key, data) = raw(&format!("{witness}/multisig"));
        let multisig = squads::decode_multisig(&data).unwrap();
        assert_eq!(
            squads::multisig_address(&Address::from(multisig.create_key)),
            (multisig_key, multisig.bump)
        );

        let (transaction_key, data) = raw(&format!("{witness}/vault_transaction"));
        let (transaction, message) = squads::decode_vault_transaction(&data).unwrap();
        assert_eq!(Address::from(transaction.multisig), multisig_key);
        assert_eq!(
            squads::transaction_address(&multisig_key, transaction.index),
            (transaction_key, transaction.bump)
        );
        assert_eq!(
            squads::vault_signer(
                &multisig_key,
                transaction.vault_index,
                transaction.vault_bump
            ),
            Some(squads::vault_address(&multisig_key, transaction.vault_index).0)
        );
        assert_eq!(
            squads::message_hash_of_bytes(&message),
            delivery.message_sha256
        );
        assert_eq!(
            squads::derive_delivery(
                &multisig_key,
                transaction.vault_index,
                transaction.index,
                delivery.message_sha256.clone()
            ),
            *delivery
        );

        let (proposal_key, data) = raw(&format!("{witness}/proposal"));
        let proposal = squads::decode_proposal(&data).unwrap();
        assert_eq!(
            squads::proposal_address(&multisig_key, transaction.index),
            (proposal_key, proposal.bump)
        );
        assert_eq!(proposal.transaction_index, transaction.index);
        assert_eq!(proposal.status.kind(), ProposalStatusKind::Active);
    }
}

/// A Proposal last written before Squads inserted `Executing` at variant 4
/// (commit 8416203, 2023-06-23) stores the old `Executed { timestamp }` there.
/// The pinned layout cannot read it, and the decoder refuses rather than
/// guessing: G1 fails closed on it.
#[test]
fn a_legacy_proposal_status_fails_closed() {
    let (_, data) = raw("legacy/proposal");
    assert_eq!(data[8 + 32 + 8], 4, "old-enum variant 4");
    let timestamp = i64::from_le_bytes(data[49..57].try_into().unwrap());
    assert!(
        (1_680_000_000..1_700_000_000).contains(&timestamp),
        "a 2023 timestamp where the current layout has none: {timestamp}"
    );
    assert!(squads::decode_proposal(&data).is_err());
}

/// Every sealed mainnet binding still verifies, names its slot, and ties to
/// the committed specs and candidate bytes.
#[test]
fn the_mainnet_bindings_are_sealed_and_consistent() {
    for witness in ["primary", "second", "large"] {
        let dir = root().join(format!("witness-{witness}"));
        let bind = binding(&dir.join("bind.json"));
        assert_eq!(bind.outcome, BindingOutcome::Matched, "{witness}");
        assert!(bind
            .statement
            .contains(&format!("slot {}", bind.observation.slot.unwrap())));
        let acquired = ChangeSpec::load(&dir.join("acquired-change-spec.json")).unwrap();
        assert!(acquired.delivery().is_none());
        assert_eq!(acquired.id().unwrap(), bind.analysed_change_spec_id);
        assert_eq!(
            acquired.candidate(),
            &bind.observation.buffer.as_ref().unwrap().artifact
        );
        let bound = bind.bound_spec(&acquired).unwrap().unwrap();
        assert_ne!(bound.id().unwrap(), acquired.id().unwrap());
        assert_eq!(bound.candidate(), acquired.candidate());
        assert_eq!(bound.target_program_id(), acquired.target_program_id());
        let Some(Delivery::SquadsV4(delivery)) = bound.delivery() else {
            unreachable!()
        };
        assert_eq!(Some(delivery), bind.observation.delivery.as_ref());

        if witness == "large" {
            continue; // bytes not committed: 2.5 MB of a third party's program
        }
        let committed = ChangeSpec::load(&dir.join("bound-change-spec.json")).unwrap();
        assert_eq!(committed.id().unwrap(), bound.id().unwrap());
        let resolved = acquired
            .resolve(CandidateSource::Store(&dir.join("store")))
            .unwrap();
        assert!(resolved.bytes().starts_with(b"\x7fELF"));

        let mut slots = vec![bind.observation.slot.unwrap()];
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("verify-") {
                let verify = binding(&entry.path());
                assert_eq!(verify.outcome, BindingOutcome::Matched, "{witness}/{name}");
                assert_eq!(verify.analysed_change_spec_id, bound.id().unwrap());
                assert_eq!(verify.observation.delivery, bind.observation.delivery);
                assert_eq!(verify.observation.buffer, bind.observation.buffer);
                slots.push(verify.observation.slot.unwrap());
            }
        }
        assert!(slots.len() >= 2, "{witness} was re-verified");
    }
}

/// The live negative controls ran against the same real proposal and none
/// defaulted to a match.
#[test]
fn the_mainnet_negative_controls_failed_closed() {
    let dir = root().join("witness-primary/negative-controls");
    for (name, outcome, code) in [
        (
            "wrong-target",
            BindingOutcome::DifferentProposal,
            "target_program_differs",
        ),
        (
            "wrong-candidate",
            BindingOutcome::StaleArtifact,
            "candidate_differs",
        ),
        (
            "wrong-index-existing",
            BindingOutcome::Unverifiable,
            "proposal_reference_differs",
        ),
        (
            "wrong-index-absent",
            BindingOutcome::Unverifiable,
            "proposal_reference_differs",
        ),
        (
            "wrong-multisig",
            BindingOutcome::Unverifiable,
            "proposal_reference_differs",
        ),
    ] {
        let control = binding(&dir.join(format!("{name}.json")));
        assert_eq!(control.outcome, outcome, "{name}");
        assert!(control.reasons.iter().any(|r| r.code == code), "{name}");
    }
}
