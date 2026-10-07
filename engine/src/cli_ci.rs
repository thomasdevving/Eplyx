//! The CI gate and program-upgrade ChangeSpec commands.
use super::{ChangeProgramUpgradeArgs, CiCheckArgs, Format};
use anyhow::{Context, Result};
use std::process::ExitCode;

/// The CI gate: a pinned bundle, a candidate, and a declaration file.
///
/// Returns an exit code rather than an error for a completed review, so a
/// failing gate is distinguishable from an analysis that could not run.
pub(crate) fn ci_check(args: CiCheckArgs) -> Result<ExitCode> {
    use eplyx_engine::ci;

    let outcome = match &args.change_spec {
        None => ci::check(
            &args.bundle,
            args.candidate
                .as_deref()
                .expect("clap requires --candidate without --change-spec"),
            args.expectations.as_deref(),
        ),
        Some(path) => eplyx_engine::change::ChangeSpec::load(path)
            .map_err(ci::CheckError::Configuration)
            .and_then(|spec| {
                let source = match (&args.candidate, &args.artifacts) {
                    (Some(file), _) => Some(eplyx_engine::change::CandidateSource::File(file)),
                    (None, Some(store)) => {
                        Some(eplyx_engine::change::CandidateSource::Store(store))
                    }
                    (None, None) => None,
                };
                ci::check_change(
                    &args.bundle,
                    &ci::ChangeInput::Spec {
                        spec: &spec,
                        source,
                    },
                    args.expectations.as_deref(),
                )
            }),
    };
    let report = match outcome {
        Ok(report) => report,
        Err(error) => {
            // A preflight abort produces no analysis, but it must still answer
            // in the format that was asked for. A consumer parsing JSON should
            // not have to scrape stderr to learn why a run produced nothing.
            let code = error.exit_code();
            match args.format {
                Format::Json => {
                    let report_schema = std::fs::read(args.bundle.join("bundle.json"))
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                        .and_then(|manifest| manifest["schema_version"].as_u64())
                        .filter(|version| *version == 2)
                        .unwrap_or(u64::from(eplyx_engine::ci::CI_REPORT_SCHEMA));
                    let body = serde_json::json!({
                        "schema_version": report_schema,
                        "status": "error",
                        "exit_code": code,
                        "error": format!("{error}"),
                    });
                    let rendered = serde_json::to_string_pretty(&body)?;
                    match &args.out {
                        Some(path) => {
                            std::fs::write(path, format!("{rendered}\n"))?;
                            eprintln!("wrote {}", path.display());
                        }
                        None => println!("{rendered}"),
                    }
                }
                Format::Text => {
                    eprintln!("EPLYX UPGRADE CHECK\n\nAnalysis could not run.\n\n{error}");
                }
            }
            return Ok(ExitCode::from(code));
        }
    };

    let rendered = match args.format {
        Format::Json => serde_json::to_string_pretty(&report)?,
        Format::Text => render_ci(&report),
    };
    match &args.out {
        Some(path) => {
            std::fs::write(path, format!("{rendered}\n"))?;
            println!("wrote {}", path.display());
        }
        None => println!("{rendered}"),
    }
    Ok(ExitCode::from(report.exit_code()))
}

pub(crate) fn change_program_upgrade(args: ChangeProgramUpgradeArgs) -> Result<ExitCode> {
    use eplyx_engine::change::{Activation, Change, ChangeSpec, ExecutableArtifact};

    let candidate = std::fs::read(&args.candidate)
        .with_context(|| format!("reading {}", args.candidate.display()))?;
    let mut spec = ChangeSpec::program_upgrade(&args.program, &candidate);
    match &mut spec.change {
        Change::ProgramUpgrade {
            target,
            replaces,
            expected_upgrade_authority,
            ..
        } => {
            target.programdata_address = args.programdata;
            *expected_upgrade_authority = args.upgrade_authority;
            if let Some(path) = &args.replaces {
                let bytes =
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
                *replaces = Some(ExecutableArtifact::of(&bytes));
            }
        }
        Change::TokenMigration(_)
        | Change::LifecycleChange(_)
        | Change::ProtocolParameterChange(_) => {
            unreachable!("program-upgrade constructor")
        }
    }
    if args.activation_slot.is_some() || args.activation_unix_timestamp.is_some() {
        spec.activation = Some(Activation {
            slot: args.activation_slot,
            unix_timestamp: args.activation_unix_timestamp,
        });
    }
    spec.metadata.label = args.label;
    spec.validate()?;
    if let Some(store) = &args.store {
        eplyx_engine::universal::evidence::EvidenceStore::at(store).put(
            eplyx_engine::universal::evidence::EvidenceKind::ProgramBinary,
            &candidate,
        )?;
    }
    let document = spec.to_document()?;
    match &args.out {
        Some(path) => {
            std::fs::write(path, format!("{document}\n"))?;
            eprintln!("wrote {} ({})", path.display(), spec.id()?);
        }
        None => println!("{document}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn render_ci(report: &eplyx_engine::ci::CiReport) -> String {
    use eplyx_engine::review::ReviewStatus;
    use std::fmt::Write as _;

    let mut text = String::from("EPLYX UPGRADE CHECK\n===================\n\n");
    let _ = writeln!(text, "Baseline:   {}", report.bundle.baseline_sha256);
    let _ = writeln!(text, "Candidate:  {}", report.candidate.sha256);
    if let Some(change) = &report.change {
        match &change.change {
            eplyx_engine::change::BoundChange::ProgramUpgrade {
                target_program_id, ..
            } => {
                let _ = writeln!(
                    text,
                    "Change:     {} ({} of {})",
                    change.change_spec_id,
                    change.kind().as_str(),
                    target_program_id
                );
            }
            eplyx_engine::change::BoundChange::ProtocolParameterChange { target, .. } => {
                let _ = writeln!(text, "Parameter mint: {}", target.config_account);
            }
            eplyx_engine::change::BoundChange::LifecycleChange { asset_mint, .. } => {
                println!("Lifecycle asset: {asset_mint}");
            }
            eplyx_engine::change::BoundChange::TokenMigration {
                source_mint,
                destination_mint,
                ..
            } => {
                let _ = writeln!(
                    text,
                    "Change:     {} (token migration {} to {})",
                    change.change_spec_id, source_mint, destination_mint
                );
            }
        }
        if let Some(eplyx_engine::change::Delivery::SquadsV4(delivery)) = change.delivery() {
            let _ = writeln!(
                text,
                "Delivery:   Squads #{} of {} (message {}). This report does not verify the \
                 proposal; run `eplyx governance squads verify`.",
                delivery.transaction_index, delivery.multisig, delivery.message_sha256
            );
        }
    }
    let _ = writeln!(
        text,
        "Bundle:     {} ({} @{})",
        report.bundle.sha256, report.bundle.adapter, report.bundle.adapter_version
    );
    let _ = writeln!(
        text,
        "\nCorpus:     {} validated historical observations",
        report.bundle.record_count
    );
    if let Some(proof) = &report.replay_proof {
        let contract = if proof.proof_contract_versions.is_empty() {
            String::new()
        } else {
            format!(", proof contracts {:?}", proof.proof_contract_versions)
        };
        let _ = writeln!(
            text,
            "Replay/proof: {} ({:?}, {} observations{})",
            proof.status, proof.profile, proof.observations, contract
        );
        if let Some(boundary) = &proof.boundary_proof {
            let _ = writeln!(text, "Boundary proof: {boundary}");
        }
    }
    if let Some(binding) = &report.semantic_binding {
        let _ = writeln!(
            text,
            "Semantic binding: exact source-to-ELF verified = {}",
            binding.exact_source_to_elf_verified
        );
        for observation in &binding.observations {
            let _ = writeln!(
                text,
                "  {}: {}",
                observation.observation_id,
                observation.binding.level()
            );
        }
    }

    let _ = writeln!(text, "\nCOVERAGE");
    for subject in &report.coverage {
        let _ = writeln!(
            text,
            "  {:<58} {:>3} observations",
            subject.subject, subject.observations
        );
    }

    let _ = writeln!(text, "\nRESULTS");
    let _ = writeln!(
        text,
        "  expected                     {:>3}",
        report.summary.expected
    );
    let _ = writeln!(
        text,
        "  unexpected                   {:>3}",
        report.summary.unexpected
    );
    let _ = writeln!(
        text,
        "  expected but exceeded        {:>3}",
        report.summary.expected_but_exceeded
    );
    let _ = writeln!(
        text,
        "  stale declarations           {:>3}",
        report.summary.stale
    );
    let _ = writeln!(
        text,
        "  unevaluable declarations     {:>3}",
        report.summary.unevaluable
    );

    if !report.findings.is_empty() {
        let _ = writeln!(text, "\nFINDINGS");
        for finding in &report.findings {
            // Severity and review status are two separate statements, and the
            // report keeps them side by side rather than folding one into the
            // other.
            let _ = writeln!(
                text,
                "  {} / {}",
                finding.severity.as_str(),
                finding.status.as_str().to_uppercase()
            );
            let _ = writeln!(text, "    {}", finding.fingerprint);
            let _ = writeln!(
                text,
                "    affected {} of {} observations that can measure it, {} entities",
                finding.observations.len(),
                finding.covered_observations,
                finding.entities.len()
            );
            if let Some(bps) = finding.max_relative_delta_bps {
                let _ = writeln!(text, "    largest change {bps} bps");
            }
            if let Some(reason) = &finding.reason {
                let _ = writeln!(text, "    declared: {reason}");
            }
            for breach in &finding.breaches {
                let _ = writeln!(text, "    exceeds: {breach:?}");
            }
            if let Some(cause) = &finding.unevaluable {
                let _ = writeln!(text, "    cannot judge: {cause:?}");
            }
        }
    }

    if !report.unmatched.is_empty() {
        let _ = writeln!(text, "\nDECLARATIONS THAT MATCHED NOTHING");
        for entry in &report.unmatched {
            let _ = writeln!(text, "  {}", entry.status.as_str().to_uppercase());
            let _ = writeln!(text, "    {}", entry.fingerprint);
            let _ = writeln!(text, "    declared: {}", entry.reason);
            let _ = writeln!(
                text,
                "    {} observations in this corpus can measure it",
                entry.covered_observations
            );
            if entry.status == ReviewStatus::Unevaluable {
                let _ = writeln!(
                    text,
                    "    Eplyx cannot prove whether this declaration still applies."
                );
            }
        }
    }

    let _ = writeln!(
        text,
        "\nGATE: {}",
        if report.summary.passed {
            "PASSED".to_string()
        } else {
            format!(
                "FAILED ({})",
                report
                    .summary
                    .failure_reasons
                    .iter()
                    .map(|reason| reason.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    let _ = write!(text, "exit {}", report.summary.exit_code);
    text
}
