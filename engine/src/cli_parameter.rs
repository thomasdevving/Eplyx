//! Paired parameter analysis and reproduction in the existing empty-environment child.
use anyhow::{ensure, Context, Result};
use clap::{Subcommand, ValueEnum};
use eplyx_engine::{change::ChangeSpec, parameter_change as engine};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::ExitCode};
#[derive(Clone, Copy, ValueEnum, Serialize, Deserialize)]
pub enum Format {
    Text,
    Json,
}
#[derive(Subcommand, Serialize, Deserialize)]
pub enum Command {
    /// Search hypothetical TransferChecked amounts for one fixed verified proposal.
    Search {
        #[arg(long)]
        change: PathBuf,
        #[arg(long)]
        parent_report: PathBuf,
        #[arg(long)]
        spec: PathBuf,
        /// Original capture bytes required when the parent binds a request-bearing capture.
        #[arg(long)]
        capture: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Read-only portable search verification; runs no VM or RPC.
    VerifySearch {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Rerun the entire deterministic completed search offline.
    ReproduceSearch {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Rerun one retained witness pair, without rerunning the search.
    ReproduceWitness {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long)]
        witness: String,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Evaluate or repeat two or three explicitly selected distinct-source cases locally.
    Cases {
        #[command(subcommand)]
        command: CaseCommand,
    },
    /// Compare one supported parameter using retained operation-specific evidence.
    Analyse {
        #[arg(long)]
        change: PathBuf,
        #[arg(long, conflicts_with_all = ["input", "bundle"], required_unless_present_any = ["input", "bundle"])]
        capture: Option<PathBuf>,
        /// Exact retained transfer fixture/context input, for already captured transfer cases.
        #[arg(long, conflicts_with_all = ["capture", "bundle"])]
        input: Option<PathBuf>,
        /// Immutable historical bundle for the Stake Pool operation.
        #[arg(long, conflicts_with_all = ["capture", "input"], requires = "record_id")]
        bundle: Option<PathBuf>,
        /// Select one exact retained historical record; never the first deposit.
        #[arg(long, requires = "bundle")]
        record_id: Option<String>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        record: Option<PathBuf>,
    },
    /// Execute both sides again from the report's retained evidence, entirely offline.
    Reproduce {
        #[arg(long)]
        change: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
}
#[derive(Subcommand, Serialize, Deserialize)]
pub enum CaseCommand {
    /// Validate exact retained inputs and save a private, bounded request manifest; no execution.
    Prepare {
        #[arg(long)]
        change: PathBuf,
        #[arg(long, num_args = 2..=3)]
        input: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Run the existing analyzer independently for every admitted input and save complete evidence.
    Analyse {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Check every complete report and the evidence-linked summary without executing.
    Verify {
        #[arg(long)]
        package: PathBuf,
    },
    /// Independently repeat every complete report through existing offline reproduction.
    Reproduce {
        #[arg(long)]
        package: PathBuf,
    },
}
#[derive(Serialize, Deserialize)]
struct Request {
    command: Command,
    source: eplyx_engine::local_store::RunSource,
}
pub fn run(command: Command) -> Result<ExitCode> {
    let request = serde_json::to_string(&Request {
        command,
        source: eplyx_engine::local_store::RunSource::detect(),
    })?;
    ensure!(request.len() < 64 * 1024, "parameter command exceeds bound");
    let bounded_search = matches!(
        serde_json::from_str::<Request>(&request)?.command,
        Command::Search { .. }
            | Command::VerifySearch { .. }
            | Command::ReproduceSearch { .. }
            | Command::ReproduceWitness { .. }
    );
    let mut child = eplyx_engine::local_store::offline_command(&std::env::current_exe()?)
        .arg("parameter-worker")
        .arg(request)
        .spawn()?;
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if bounded_search && started.elapsed() >= std::time::Duration::from_secs(900) {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("parameter search worker timeout after 900 seconds; any artifact without a completed manifest remains incomplete, no counterexample asserted");
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    Ok(ExitCode::from(status.code().unwrap_or(2) as u8))
}
pub fn worker(encoded: &str) -> Result<ExitCode> {
    eplyx_engine::local_store::verify_offline_environment()?;
    ensure!(encoded.len() < 64 * 1024, "parameter command exceeds bound");
    let Request { command, source } = serde_json::from_str(encoded)?;
    match command {
        Command::Search {
            change,
            parent_report,
            spec,
            capture,
            out,
            format,
        } => {
            use eplyx_engine::{lifecycle::artifact, parameter_search as search};
            let input = search::Input {
                change: ChangeSpec::parse(&artifact::read(change)?)?,
                parent_report: serde_json::from_slice(&artifact::read(parent_report)?)?,
                spec: serde_json::from_slice(&artifact::read(spec)?)?,
                source_capture: capture
                    .map(|p| -> Result<String> { Ok(String::from_utf8(artifact::read(p)?)?) })
                    .transpose()?,
            };
            search::store::begin(&out, &input)?;
            let report = search::search(&input)?;
            search::store::save(&out, &report)?;
            emit_search(&report, format, "searched")?;
            Ok(ExitCode::SUCCESS)
        }
        Command::VerifySearch { artifact, format } => {
            let report = eplyx_engine::parameter_search::store::load(&artifact)?;
            emit_search(&report, format, "verified")?;
            Ok(ExitCode::SUCCESS)
        }
        Command::ReproduceSearch { artifact, format } => {
            let report = eplyx_engine::parameter_search::store::load(&artifact)?;
            eplyx_engine::parameter_search::reproduce(&report)?;
            emit_search(&report, format, "reproduced")?;
            Ok(ExitCode::SUCCESS)
        }
        Command::ReproduceWitness {
            artifact,
            witness,
            format,
        } => {
            let report = eplyx_engine::parameter_search::store::load(&artifact)?;
            let value = eplyx_engine::parameter_search::reproduce_witness(&report, &witness)?;
            match format {
                Format::Json => println!("{}", value),
                Format::Text => println!("{}", serde_json::to_string_pretty(&value)?),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Cases { command } => {
            use eplyx_engine::parameter_cases as cases;
            let receipt = match command {
                CaseCommand::Prepare { change, input, out } => {
                    let manifest = cases::prepare(&change, &input, &out)?;
                    serde_json::json!({"status":"selected_case_set_prepared","case_set_id":manifest.case_set_id,"offline":true})
                }
                CaseCommand::Analyse { manifest, out } => {
                    let summary = cases::analyze(&manifest, &out)?;
                    serde_json::json!({"status":if summary.counts.unavailable_or_failed == 0 {"selected_case_set_evaluated"} else {"case_set_evidence_blocked"},"case_set_id":summary.case_set_id,"result_sha256":summary.result_sha256,"counts":summary.counts,"offline":true})
                }
                CaseCommand::Verify { package } => cases::check(&package, false)?,
                CaseCommand::Reproduce { package } => cases::check(&package, true)?,
            };
            let blocked = receipt["status"] == "case_set_evidence_blocked";
            println!("{receipt}");
            Ok(ExitCode::from(if blocked { 2 } else { 0 }))
        }
        Command::Analyse {
            change,
            capture,
            input,
            bundle,
            record_id,
            out,
            record,
        } => {
            let spec = ChangeSpec::parse(&eplyx_engine::lifecycle::artifact::read(change)?)?;
            let report = match (capture, input, bundle) {
                (Some(path), None, None) => engine::analyze(
                    &spec,
                    &eplyx_engine::path::current::parameter_input(
                        &eplyx_engine::lifecycle::artifact::read(path)?,
                    )?,
                )?,
                (None, Some(path), None) => {
                    let bytes = eplyx_engine::lifecycle::artifact::read(path)?;
                    if matches!(
                        spec.as_protocol_parameter_change()
                            .context("parameter change required")?
                            .operation,
                        engine::Operation::SplStakePoolSolDepositFeeV1 { .. }
                    ) {
                        engine::stake_pool::analyze(&spec, &serde_json::from_slice(&bytes)?)?
                    } else {
                        engine::analyze(&spec, &serde_json::from_slice(&bytes)?)?
                    }
                }
                (None, None, Some(path)) => {
                    let bundle = eplyx_engine::bundle::CiBundle::open(path)?;
                    let input = engine::stake_pool::Input::from_bundle(
                        &bundle,
                        record_id
                            .as_deref()
                            .context("explicit record ID required")?,
                    )?;
                    engine::stake_pool::analyze(&spec, &input)?
                }
                _ => anyhow::bail!(
                    "select exactly one retained capture, typed input or historical bundle"
                ),
            };
            let document = eplyx_engine::canonical::document(&report)?;
            ensure!(!out.exists(), "output must be new");
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(out)?;
            file.write_all(document.as_bytes())?;
            file.sync_all()?;
            let mut receipt = serde_json::json!({"status":report["status"],"change_spec_id":spec.id()?,"report_sha256":report["report_sha256"],"offline":true});
            if let Some(project) = record {
                let metadata = eplyx_engine::local_store::save_analysis(
                    &project,
                    engine::KIND,
                    document.as_bytes(),
                    Some(spec.to_document()?.as_bytes()),
                    None,
                    source,
                )?;
                receipt["run_id"] = metadata.run_id.into();
            }
            println!("{}", receipt);
            Ok(ExitCode::from(if report["execution_performed"] == true {
                0
            } else {
                2
            }))
        }
        Command::Reproduce { change, report } => {
            let spec = ChangeSpec::parse(&eplyx_engine::lifecycle::artifact::read(change)?)?;
            let value = serde_json::from_slice(&eplyx_engine::lifecycle::artifact::read(report)?)
                .context("invalid report")?;
            engine::reproduce(&spec, &value)?;
            println!(
                "{}",
                serde_json::json!({"reproduced":true,"offline":true,"change_spec_id":spec.id()?,"report_sha256":value["report_sha256"]})
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn emit_search(
    report: &eplyx_engine::parameter_search::Report,
    format: Format,
    operation: &str,
) -> Result<()> {
    use eplyx_engine::parameter_search as search;
    match format {
        Format::Text => println!("{}", search::render(report)?),
        Format::Json => {
            let mut receipt = search::receipt(report)?;
            receipt["operation"] = operation.into();
            receipt["operation_vm_calls"] = if operation == "verified" {
                serde_json::json!(0)
            } else {
                report.summary["total_vm_calls"].clone()
            };
            println!("{}", receipt);
        }
    }
    Ok(())
}
