//! Current observation and local execution use separate processes and capabilities.
use super::Format;
use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use eplyx_engine::{
    lifecycle::{artifact, current as observation, policy::LifecycleScenario, LifecycleSnapshot},
    path::{self, current, position},
};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::PathBuf, process::ExitCode};

#[derive(Subcommand, Serialize, Deserialize)]
pub enum PathCommand {
    /// Execute one pinned market-exit probe entirely offline.
    Probe {
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        probe: PathBuf,
        #[arg(long)]
        at: chrono::DateTime<chrono::Utc>,
        #[command(flatten)]
        output: Output,
    },
    /// Read-only capture of a frozen lifecycle market-exit probe's dependencies.
    CaptureProbe {
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        entity: Option<String>,
        #[arg(long)]
        pool: Option<String>,
        #[arg(long)]
        amount_raw: u64,
        #[arg(long)]
        fixture_out: PathBuf,
        #[arg(long)]
        probe_out: PathBuf,
    },
    DiscoverPosition(PositionArgs),
    ProbeWithdrawal {
        #[command(flatten)]
        inputs: PositionArgs,
        #[arg(long)]
        position: PathBuf,
        /// Optional exact range/fraction declaration; defaults to full principal removal.
        #[arg(long)]
        terms: Option<PathBuf>,
    },
    /// Available checks are not execution proof.
    Capabilities {
        #[arg(long)]
        input: PathBuf,
        #[command(flatten)]
        output: Output,
    },
    Validate {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[command(flatten)]
        output: Output,
    },
    /// Read-only dependency capture. Does not execute or submit a transaction.
    Capture {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        run_id: String,
        #[arg(long)]
        check_id: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Replay exact captured dependencies in the offline VM.
    Replay {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        run_id: String,
        #[arg(long)]
        check_id: String,
        #[arg(long)]
        wallet_sha256: String,
        #[arg(long)]
        capture_sha256: String,
        #[command(flatten)]
        output: Output,
    },
}
#[derive(Subcommand, Serialize, Deserialize)]
pub enum ObserveCommand {
    /// Capture a validated mint/owner selection through read-only RPC.
    Capture {
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Re-decode a saved observation. No execution classification is inherited.
    Replay {
        #[arg(long)]
        input: PathBuf,
        #[command(flatten)]
        output: Output,
    },
}
#[derive(Parser, Serialize, Deserialize)]
pub struct Output {
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[arg(long)]
    out: Option<PathBuf>,
}
#[derive(Parser, Serialize, Deserialize)]
pub struct PositionArgs {
    #[arg(long)]
    snapshot: PathBuf,
    #[arg(long)]
    scenario: PathBuf,
    #[arg(long)]
    discovery: PathBuf,
    #[arg(long)]
    fixture: PathBuf,
    #[command(flatten)]
    output: Output,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Request {
    Path(PathCommand),
    Observe(ObserveCommand),
}
impl Request {
    fn format(&self) -> Format {
        match self {
            Self::Path(PathCommand::Capture { .. } | PathCommand::CaptureProbe { .. })
            | Self::Observe(ObserveCommand::Capture { .. }) => Format::Json,
            Self::Path(
                PathCommand::Probe { output, .. }
                | PathCommand::Capabilities { output, .. }
                | PathCommand::Validate { output, .. }
                | PathCommand::Replay { output, .. },
            )
            | Self::Observe(ObserveCommand::Replay { output, .. }) => output.format,
            Self::Path(
                PathCommand::DiscoverPosition(inputs) | PathCommand::ProbeWithdrawal { inputs, .. },
            ) => inputs.output.format,
        }
    }
}
fn provider() -> Result<eplyx_engine::ingest::rpc::HttpRpc> {
    eplyx_engine::ingest::rpc::HttpRpc::new(
        std::env::var("SOLANA_RPC_URL").context("SOLANA_RPC_URL is required for capture")?,
    )
}
fn read_text(input: &std::path::Path) -> Result<String> {
    Ok(String::from_utf8(artifact::read(input)?)?)
}
pub fn execute(request: Request) -> Result<ExitCode> {
    let format = request.format();
    let result = (|| {
        match request {
            Request::Path(PathCommand::CaptureProbe {
                snapshot,
                scenario,
                entity,
                pool,
                amount_raw,
                fixture_out,
                probe_out,
            }) => {
                ensure!(
                    !fixture_out.exists() && !probe_out.exists() && fixture_out != probe_out,
                    "capture outputs must be new and distinct"
                );
                let parent = |p: &std::path::Path| {
                    p.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .canonicalize()
                };
                ensure!(
                    parent(&fixture_out)? == parent(&probe_out)?,
                    "probe and fixture outputs must share a directory"
                );
                let reference = fixture_out
                    .file_name()
                    .context("fixture filename required")?
                    .to_str()
                    .context("fixture filename must be UTF-8")?
                    .to_owned();
                let (spec, fixture) = path::capture::capture_at(
                    &LifecycleSnapshot::load(&snapshot)?,
                    &LifecycleScenario::load(&scenario)?,
                    entity.as_deref(),
                    amount_raw,
                    reference,
                    pool.as_deref(),
                    &provider()?,
                )?;
                eplyx_engine::lifecycle::frozen::selection::save(&fixture, &fixture_out)?;
                eplyx_engine::lifecycle::frozen::selection::save(&spec, &probe_out)?;
                println!(
                    "{}",
                    serde_json::json!({"captured":true,"execution_performed":false})
                );
                Ok(ExitCode::SUCCESS)
            }
            Request::Observe(ObserveCommand::Capture { selection, out }) => {
                ensure!(!out.exists(), "capture output already exists");
                let selected = artifact::load(&selection)?;
                let capture = observation::capture_selected(selected, &provider()?)?;
                observation::save(&capture, &out)?;
                // Return only a receipt; decoding runs through the offline surface.
                println!(
                    "{}",
                    serde_json::json!({"captured":true,"execution_performed":false})
                );
                Ok(ExitCode::SUCCESS)
            }
            Request::Path(PathCommand::Capture {
                input,
                request,
                run_id,
                check_id,
                out,
            }) => {
                ensure!(!out.exists(), "capture output already exists");
                let capture = current::capture(
                    read_text(&input)?,
                    artifact::load(&request)?,
                    run_id,
                    check_id,
                    &provider()?,
                )?;
                current::save(&capture, &out)?;
                println!(
                    "{}",
                    serde_json::json!({"captured":true,"execution_performed":false})
                );
                Ok(ExitCode::SUCCESS)
            }
            offline => {
                let encoded = serde_json::to_string(&offline)?;
                ensure!(
                    encoded.len() <= 64 * 1024,
                    "path request exceeds byte bound"
                );
                let status = eplyx_engine::local_store::offline_command(&std::env::current_exe()?)
                    .arg("path-worker")
                    .arg(encoded)
                    .status()?;
                Ok(ExitCode::from(status.code().unwrap_or(2) as u8))
            }
        }
    })();
    Ok(result.unwrap_or_else(|error| super::cli_local::emit(format, Err(error))))
}
fn emit<T: Serialize>(value: &T, text: &str, output: Output) -> Result<ExitCode> {
    let bytes = eplyx_engine::canonical::document(value)?;
    if let Some(path) = output.out {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .context("output must be new")?;
        file.write_all(bytes.as_bytes())?;
        file.sync_all()?;
    }
    match output.format {
        Format::Json => print!("{bytes}"),
        Format::Text => println!("{text}"),
    }
    Ok(ExitCode::SUCCESS)
}
pub fn worker(encoded: &str) -> Result<ExitCode> {
    eplyx_engine::local_store::verify_offline_environment()?;
    ensure!(
        encoded.len() <= 64 * 1024,
        "path request exceeds byte bound"
    );
    let request: Request = serde_json::from_str(encoded)?;
    let format = request.format();
    let result = (|| {
        let output = match &request {
            Request::Path(
                PathCommand::Probe { output, .. }
                | PathCommand::Capabilities { output, .. }
                | PathCommand::Validate { output, .. }
                | PathCommand::Replay { output, .. },
            )
            | Request::Observe(ObserveCommand::Replay { output, .. }) => Some(output),
            Request::Path(
                PathCommand::DiscoverPosition(inputs) | PathCommand::ProbeWithdrawal { inputs, .. },
            ) => Some(&inputs.output),
            _ => None,
        };
        ensure!(
            output
                .and_then(|o| o.out.as_ref())
                .is_none_or(|p| !p.exists()),
            "output must be new"
        );
        match request {
            Request::Path(PathCommand::Capture { .. } | PathCommand::CaptureProbe { .. })
            | Request::Observe(ObserveCommand::Capture { .. }) => {
                anyhow::bail!("capture unavailable inside an offline worker")
            }
            Request::Observe(ObserveCommand::Replay { input, output }) => emit(
                &observation::replay(&input)?,
                "Current observations decoded. Paths remain not evaluated.",
                output,
            ),
            Request::Path(PathCommand::Capabilities { input, output }) => emit(
                &current::capabilities(&read_text(&input)?)?,
                "Current check availability decoded; no execution performed.",
                output,
            ),
            Request::Path(PathCommand::Validate {
                input,
                request,
                output,
            }) => emit(
                &current::validate(&read_text(&input)?, &artifact::load(&request)?)?,
                "Current check terms validated; no execution performed.",
                output,
            ),
            Request::Path(PathCommand::Replay {
                input,
                run_id,
                check_id,
                wallet_sha256,
                capture_sha256,
                output,
            }) => {
                let verified = current::replay(
                    &artifact::read(&input)?,
                    &run_id,
                    &check_id,
                    &wallet_sha256,
                    &capture_sha256,
                )?;
                emit(
                    verified.value(),
                    "Current path check completed for the exact captured scope.",
                    output,
                )
            }
            Request::Path(PathCommand::Probe {
                snapshot,
                scenario,
                probe,
                at,
                output,
            }) => {
                let (spec, fixture) = path::load_probe(&probe)?;
                let report = path::run(
                    &LifecycleSnapshot::load(&snapshot)?,
                    &LifecycleScenario::load(&scenario)?,
                    &spec,
                    &fixture,
                    at,
                )?;
                emit(
                    &report,
                    "Captured market-exit check completed for the declared scope.",
                    output,
                )
            }
            Request::Path(PathCommand::DiscoverPosition(inputs)) => {
                let position = position::meteora_dlmm::discover(
                    &LifecycleSnapshot::load(&inputs.snapshot)?,
                    &LifecycleScenario::load(&inputs.scenario)?,
                    &artifact::read(&inputs.discovery)?,
                    &artifact::read(&inputs.fixture)?,
                )?;
                emit(
                    &position,
                    "Position decoded from captured protocol state.",
                    inputs.output,
                )
            }
            Request::Path(PathCommand::ProbeWithdrawal {
                inputs,
                position,
                terms,
            }) => {
                let selected = artifact::load(&position)?;
                let probe = match terms {
                    Some(terms) => artifact::load(&terms)?,
                    None => position::WithdrawalProbe::full(&selected),
                };
                let report = position::meteora_dlmm::execute(
                    &selected,
                    &probe,
                    &LifecycleSnapshot::load(&inputs.snapshot)?,
                    &LifecycleScenario::load(&inputs.scenario)?,
                    &artifact::read(&inputs.discovery)?,
                    &artifact::read(&inputs.fixture)?,
                )?;
                emit(&report, &report.render_text(), inputs.output)
            }
        }
    })();
    Ok(result.unwrap_or_else(|error| super::cli_local::emit(format, Err(error))))
}
