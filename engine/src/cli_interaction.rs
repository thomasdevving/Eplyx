//! Local interaction commands run in the existing credential-free child.
use anyhow::{ensure, Result};
use clap::{Subcommand, ValueEnum};
use eplyx_engine::{change::ChangeSpec, interaction, lifecycle::artifact};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::ExitCode};
#[derive(Clone, Copy, ValueEnum, Serialize, Deserialize)]
pub enum Format {
    Json,
    Text,
}
#[derive(Subcommand, Serialize, Deserialize)]
pub enum Command {
    /// Execute the bounded V1/V2 × C0/C1 experiment from retained local evidence.
    Analyse {
        #[arg(long)]
        upgrade: PathBuf,
        #[arg(long)]
        parameter: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        record_id: String,
        #[arg(long)]
        candidate: PathBuf,
        /// New artifact directory; an existing path is never overwritten.
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Check portable artifact integrity and derivations without running a VM.
    Verify {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Verify and rerun the complete experiment offline in fresh VM banks.
    Reproduce {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
}
pub fn run(command: Command) -> Result<ExitCode> {
    let encoded = serde_json::to_string(&command)?;
    ensure!(
        encoded.len() < 64 * 1024,
        "interaction command exceeds bound"
    );
    let status = eplyx_engine::local_store::offline_command(&std::env::current_exe()?)
        .arg("interaction-worker")
        .arg(encoded)
        .status()?;
    Ok(ExitCode::from(status.code().unwrap_or(2) as u8))
}
pub fn worker(encoded: &str) -> Result<ExitCode> {
    eplyx_engine::local_store::verify_offline_environment()?;
    ensure!(
        encoded.len() < 64 * 1024,
        "interaction command exceeds bound"
    );
    let command: Command = serde_json::from_str(encoded)?;
    let (report, format, operation) = match command {
        Command::Analyse {
            upgrade,
            parameter,
            bundle,
            record_id,
            candidate,
            out,
            format,
        } => {
            ensure!(!out.exists(), "output directory must be fresh");
            let bounded = |path: PathBuf, limit: u64| -> Result<Vec<u8>> {
                ensure!(
                    std::fs::symlink_metadata(&path)?.len() <= limit,
                    "interaction input exceeds existing local bound"
                );
                artifact::read(path)
            };
            let upgrade = ChangeSpec::parse(&bounded(
                upgrade,
                eplyx_engine::migration::input::MAX_CHANGE_BYTES,
            )?)?;
            let parameter = ChangeSpec::parse(&bounded(
                parameter,
                eplyx_engine::migration::input::MAX_CHANGE_BYTES,
            )?)?;
            let bundle = eplyx_engine::bundle::CiBundle::open(bundle)?;
            let historical = eplyx_engine::parameter_change::stake_pool::Input::from_bundle(
                &bundle, &record_id,
            )?;
            let input = interaction::Input::new(
                &upgrade,
                &parameter,
                historical,
                bounded(candidate, eplyx_engine::migration::input::MAX_PROGRAM_BYTES)?,
            )?;
            let report = interaction::analyse(&input)?;
            interaction::save(&report, &out)?;
            (report, format, "analysed")
        }
        Command::Verify { artifact, format } => (interaction::load(&artifact)?, format, "verified"),
        Command::Reproduce { artifact, format } => {
            let report = interaction::load(&artifact)?;
            interaction::reproduce(&report)?;
            (report, format, "reproduced")
        }
    };
    let receipt = serde_json::json!({"operation":operation,"status":report.status,"analysis_input_sha256":report.analysis_input_sha256,"report_sha256":report.report_sha256,"upgrade_change_spec_id":report.input.upgrade.id()?,"parameter_change_spec_id":report.input.parameter.id()?,"offline":true,"vm_execution_requested":operation!="verified","recipient_effects":report.effects["recipient_account_credit_raw"]});
    match format {
        Format::Json => println!("{}", serde_json::to_string(&receipt)?),
        Format::Text => println!("{}", serde_json::to_string_pretty(&receipt)?),
    };
    Ok(ExitCode::SUCCESS)
}
