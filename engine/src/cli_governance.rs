//! Squads governance binding and deployment attestation commands.
use super::{
    CommitmentArg, Format, SquadsAcquireArgs, SquadsAttestArgs, SquadsBindArgs, SquadsChainArgs,
    SquadsVerifyArgs,
};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::ExitCode;

fn squads_rpc(args: &SquadsChainArgs) -> Result<eplyx_engine::ingest::rpc::HttpRpc> {
    let url = args
        .rpc_url
        .clone()
        .or_else(|| std::env::var("SOLANA_RPC_URL").ok())
        .context("--rpc-url or SOLANA_RPC_URL is required")?;
    eplyx_engine::ingest::rpc::HttpRpc::new(url)
}

fn squads_commitment(args: &SquadsChainArgs) -> eplyx_engine::governance::Commitment {
    match args.commitment {
        CommitmentArg::Confirmed => eplyx_engine::governance::Commitment::Confirmed,
        CommitmentArg::Finalized => eplyx_engine::governance::Commitment::Finalized,
    }
}

pub(crate) fn squads_attest(args: SquadsAttestArgs) -> Result<ExitCode> {
    use eplyx_engine::change::CandidateSource;
    use eplyx_engine::governance::attestation::{attest_squads_upgrade, DeploymentOutcome};
    use eplyx_engine::governance::GovernanceBinding;
    anyhow::ensure!(
        matches!(args.chain.commitment, CommitmentArg::Finalized),
        "deployment attestation requires finalized RPC evidence"
    );
    let spec = eplyx_engine::change::ChangeSpec::load(&args.change_spec)?;
    let binding = GovernanceBinding::parse(&std::fs::read(&args.binding)?)?;
    let candidate = spec.resolve(CandidateSource::Store(&args.artifacts))?;
    let rpc = squads_rpc(&args.chain)?;
    let attestation = attest_squads_upgrade(&rpc, &spec, &binding, candidate.bytes())?;
    let document = attestation.to_document()?;
    if let Some(path) = &args.evidence_out {
        std::fs::write(path, format!("{document}\n"))
            .with_context(|| format!("writing {}", path.display()))?;
    }
    match args.format {
        Format::Json => println!("{document}"),
        _ => {
            println!("EPLYX DEPLOYMENT ATTESTATION\n============================\n");
            println!("Result:    {:?}", attestation.outcome);
            println!(
                "Proposal:  Squads #{} of {}",
                attestation.transaction_index, attestation.multisig
            );
            if let Some(execution) = &attestation.execution {
                println!(
                    "Execution: {} at slot {} index {:?}",
                    execution.signature, execution.slot, execution.transaction_index
                );
            }
            for reason in &attestation.reasons {
                println!("  - {reason}");
            }
            println!(
                "Evidence:  {}",
                attestation.attestation_id.as_deref().unwrap_or("unsealed")
            );
        }
    }
    Ok(ExitCode::from(match attestation.outcome {
        DeploymentOutcome::DeployedMatch => 0,
        DeploymentOutcome::DeployedMismatch => 1,
        DeploymentOutcome::Superseded
        | DeploymentOutcome::NotExecuted
        | DeploymentOutcome::Unverifiable => 2,
        DeploymentOutcome::Unsupported => 4,
    }))
}

fn render_binding(binding: &eplyx_engine::governance::GovernanceBinding) -> String {
    use std::fmt::Write as _;
    let mut text = String::from("EPLYX GOVERNANCE BINDING\n========================\n\n");
    let _ = writeln!(text, "Result:     {}", binding.outcome.as_str());
    let _ = writeln!(
        text,
        "Proposal:   Squads #{} of {}",
        binding.request.transaction_index, binding.request.multisig
    );
    let observation = &binding.observation;
    if let Some(slot) = observation.slot {
        let _ = writeln!(
            text,
            "Observed:   slot {slot} ({})",
            binding.commitment.as_str()
        );
    }
    if let Some(proposal) = &observation.proposal {
        let _ = writeln!(
            text,
            "Status:     {:?}{}",
            proposal.status,
            if proposal.stale { " (stale)" } else { "" }
        );
    }
    let _ = writeln!(text, "Analysed:   {}", binding.analysed_change_spec_id);
    if let Some(bound) = &binding.bound_change_spec_id {
        let _ = writeln!(text, "Bound:      {bound}");
    }
    if let Some(delivery) = &observation.delivery {
        let _ = writeln!(text, "Message:    {}", delivery.message_sha256);
    }
    if let Some(upgrade) = &observation.upgrade {
        let _ = writeln!(text, "Program:    {}", upgrade.program);
        let _ = writeln!(text, "Buffer:     {}", upgrade.buffer);
    }
    if let Some(buffer) = &observation.buffer {
        let _ = writeln!(
            text,
            "Buffer ELF: {} ({} bytes)",
            buffer.artifact.sha256, buffer.artifact.len
        );
    }
    let _ = writeln!(
        text,
        "Candidate:  {} ({} bytes)",
        binding.expected.candidate.sha256, binding.expected.candidate.len
    );
    for reason in &binding.reasons {
        let _ = writeln!(text, "  - [{}] {}", reason.code, reason.detail);
    }
    let _ = writeln!(text, "\n{}", binding.statement);
    if let Some(id) = &binding.binding_id {
        let _ = writeln!(text, "\nEvidence:   {id}");
    }
    text
}

fn emit_binding(
    binding: &eplyx_engine::governance::GovernanceBinding,
    format: Format,
    evidence_out: Option<&PathBuf>,
) -> Result<()> {
    let document = binding.to_document()?;
    if let Some(path) = evidence_out {
        std::fs::write(path, format!("{document}\n"))
            .with_context(|| format!("writing {}", path.display()))?;
    }
    match format {
        Format::Json => println!("{document}"),
        _ => print!("{}", render_binding(binding)),
    }
    Ok(())
}

pub(crate) fn squads_bind(args: SquadsBindArgs) -> Result<ExitCode> {
    use eplyx_engine::governance::{self, BindingOutcome, SquadsProposalRef};
    let spec = eplyx_engine::change::ChangeSpec::load(&args.change_spec)?;
    let rpc = squads_rpc(&args.chain)?;
    let request = SquadsProposalRef {
        multisig: args.multisig,
        transaction_index: args.transaction_index,
    };
    let binding =
        governance::verify_squads_upgrade(&rpc, &request, &spec, squads_commitment(&args.chain))?;
    emit_binding(&binding, args.format, args.evidence_out.as_ref())?;
    if binding.outcome == BindingOutcome::Matched {
        let bound = binding
            .bound_spec(&spec)?
            .context("a matched binding always carries its proposal")?;
        if let Some(path) = &args.out {
            std::fs::write(path, format!("{}\n", bound.to_document()?))?;
            eprintln!(
                "wrote {} ({}); analyse it so a report names this proposal",
                path.display(),
                bound.id()?
            );
        }
    }
    Ok(ExitCode::from(binding.outcome.exit_code()))
}

pub(crate) fn squads_verify(args: SquadsVerifyArgs) -> Result<ExitCode> {
    use eplyx_engine::change::Delivery;
    use eplyx_engine::governance::{self, SquadsProposalRef};
    let spec = eplyx_engine::change::ChangeSpec::load(&args.change_spec)?;
    let Some(Delivery::SquadsV4(delivery)) = spec.delivery() else {
        anyhow::bail!(
            "{} names no Squads delivery; bind it first with `eplyx governance squads bind`",
            args.change_spec.display()
        );
    };
    let request = SquadsProposalRef {
        multisig: args.multisig.unwrap_or_else(|| delivery.multisig.clone()),
        transaction_index: args.transaction_index.unwrap_or(delivery.transaction_index),
    };
    let rpc = squads_rpc(&args.chain)?;
    let binding =
        governance::verify_squads_upgrade(&rpc, &request, &spec, squads_commitment(&args.chain))?;
    emit_binding(&binding, args.format, args.evidence_out.as_ref())?;
    Ok(ExitCode::from(binding.outcome.exit_code()))
}

pub(crate) fn squads_acquire(args: SquadsAcquireArgs) -> Result<ExitCode> {
    use eplyx_engine::governance::{self, SquadsProposalRef};
    let rpc = squads_rpc(&args.chain)?;
    let request = SquadsProposalRef {
        multisig: args.multisig,
        transaction_index: args.transaction_index,
    };
    let (spec, bytes) =
        governance::acquire_squads_candidate(&rpc, &request, squads_commitment(&args.chain))?;
    eplyx_engine::universal::evidence::EvidenceStore::at(&args.store).put(
        eplyx_engine::universal::evidence::EvidenceKind::ProgramBinary,
        &bytes,
    )?;
    std::fs::write(&args.out, format!("{}\n", spec.to_document()?))?;
    let candidate = spec
        .candidate()
        .context("acquired change names no executable candidate")?;
    eprintln!(
        "stored the buffer's current bytes as candidate {} ({} bytes) and wrote {} ({}). \
         Analyse it, then `governance squads bind` to name the proposal.",
        candidate.sha256,
        candidate.len,
        args.out.display(),
        spec.id()?
    );
    Ok(ExitCode::SUCCESS)
}
