//! Paired parameter analysis and reproduction in the existing empty-environment child.
use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use eplyx_engine::{change::ChangeSpec, parameter_change as engine};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::ExitCode};
#[derive(Subcommand, Serialize, Deserialize)]
pub enum Command {
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
    let status = eplyx_engine::local_store::offline_command(&std::env::current_exe()?)
        .arg("parameter-worker")
        .arg(request)
        .status()?;
    Ok(ExitCode::from(status.code().unwrap_or(2) as u8))
}
pub fn worker(encoded: &str) -> Result<ExitCode> {
    eplyx_engine::local_store::verify_offline_environment()?;
    ensure!(encoded.len() < 64 * 1024, "parameter command exceeds bound");
    let Request { command, source } = serde_json::from_str(encoded)?;
    match command {
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
