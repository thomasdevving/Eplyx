//! Local rollout rehearsal commands, run in the existing credential-free child.
//! The CLI constructs only the supported rollout shape; it never accepts
//! caller-supplied instructions, accounts, Clock, signers or results.
use anyhow::{ensure, Result};
use clap::{Subcommand, ValueEnum};
use eplyx_engine::{change::ChangeSpec, lifecycle::artifact, rollout};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::ExitCode};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum Format {
    Json,
    Text,
}

#[derive(Subcommand, Serialize, Deserialize)]
pub enum Command {
    /// Rehearse an installed loader-v3 upgrade against the qualified Stake Pool
    /// SetFee(SolDeposit) in both orders, then the retained DepositSol.
    Analyse {
        /// Existing program_upgrade ChangeSpec.
        #[arg(long)]
        upgrade: PathBuf,
        /// Existing protocol_parameter_change ChangeSpec.
        #[arg(long)]
        parameter: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        record_id: String,
        /// Candidate ELF; must be exactly the upgrade ChangeSpec's candidate.
        #[arg(long)]
        candidate: PathBuf,
        /// New artifact directory; an existing path is never overwritten.
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Check the portable artifact's bindings and derivations without a VM.
    Verify {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Verify, then rerun all five scenarios offline in fresh VMs.
    Reproduce {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
}

pub fn run(command: Command) -> Result<ExitCode> {
    let encoded = serde_json::to_string(&command)?;
    ensure!(encoded.len() < 64 * 1024, "rollout command exceeds bound");
    let status = eplyx_engine::local_store::offline_command(&std::env::current_exe()?)
        .arg("rollout-worker")
        .arg(encoded)
        .status()?;
    Ok(ExitCode::from(status.code().unwrap_or(2) as u8))
}

fn format_of(command: &Command) -> Format {
    match command {
        Command::Analyse { format, .. }
        | Command::Verify { format, .. }
        | Command::Reproduce { format, .. } => *format,
    }
}

pub fn worker(encoded: &str) -> Result<ExitCode> {
    eplyx_engine::local_store::verify_offline_environment()?;
    ensure!(encoded.len() < 64 * 1024, "rollout command exceeds bound");
    let command: Command = serde_json::from_str(encoded)?;
    let format = format_of(&command);
    match execute(command) {
        Ok(receipt) => {
            match format {
                Format::Json => println!("{}", serde_json::to_string(&receipt)?),
                Format::Text => println!("{}", serde_json::to_string_pretty(&receipt)?),
            }
            Ok(ExitCode::SUCCESS)
        }
        // A preflight refusal produces no analysis; under JSON it is still a
        // structured error carrying the same exit code.
        Err(error) if format == Format::Json => {
            println!(
                "{}",
                serde_json::json!({"error":{"code":2,"message":format!("{error:#}")}})
            );
            Ok(ExitCode::from(2))
        }
        Err(error) => Err(error),
    }
}

fn execute(command: Command) -> Result<serde_json::Value> {
    let (analysis, operation, vm) = match command {
        Command::Analyse {
            upgrade,
            parameter,
            bundle,
            record_id,
            candidate,
            out,
            ..
        } => {
            ensure!(!out.exists(), "output directory must be fresh");
            let bounded = |path: PathBuf, limit: u64| -> Result<Vec<u8>> {
                ensure!(
                    std::fs::symlink_metadata(&path)?.len() <= limit,
                    "rollout input exceeds existing local bound"
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
            let input = rollout::Input::new(
                &upgrade,
                &parameter,
                historical,
                bounded(candidate, eplyx_engine::migration::input::MAX_PROGRAM_BYTES)?,
            )?;
            let analysis = rollout::analyse(&input)?;
            rollout::artifact::save(&analysis, &out)?;
            let vm = rollout::vm_executions(&analysis.report);
            (analysis, "analysed", vm)
        }
        Command::Verify { artifact, .. } => (rollout::artifact::load(&artifact)?, "verified", 0),
        Command::Reproduce { artifact, .. } => {
            let analysis = rollout::artifact::load(&artifact)?;
            rollout::reproduce(&analysis)?;
            let vm = rollout::vm_executions(&analysis.report);
            (analysis, "reproduced", vm)
        }
    };
    let r = &analysis.report;
    Ok(serde_json::json!({
        "operation": operation,
        "status": r.comparison.status,
        "finding": r.comparison.finding,
        "statement": r.comparison.statement,
        "analysis_input_id": r.analysis_input_id,
        "report_sha256": r.report_sha256,
        "upgrade_change_spec_id": r.upgrade_change_spec_id,
        "parameter_change_spec_id": r.parameter_change_spec_id,
        "anchors": {
            "baseline_world_fidelity": r.anchors.baseline_world_fidelity.status,
            "installed_overlay": r.anchors.installed_overlay.status,
        },
        "upgrade_blocker": r.preflight.upgrade_blocker,
        "scenarios": r.scenarios.iter().map(|s| serde_json::json!({
            "name": s.name,
            "scenario_id": s.scenario_id,
            "steps": s.steps.iter().map(|x| serde_json::json!({"step": x.step, "outcome": x.outcome, "reason": x.reason})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "first_divergence": r.comparison.first_divergence,
        "offline": true,
        "vm_executions": vm,
    }))
}
