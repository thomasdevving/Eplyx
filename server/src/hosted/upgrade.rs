//! Program-upgrade orchestration keeps the original engine report bytes while
//! moving execution out of the process that holds identity/provider credentials.
use crate::{artifacts::ArtifactRef, registry::RunOutcome};
use anyhow::{ensure, Result};
use eplyx_engine::{
    change::{CandidateSource, ChangeSpec},
    ci,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    bundle: PathBuf,
    spec: ChangeSpec,
    candidate: ArtifactRef,
    expectations: Option<ArtifactRef>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "outcome", deny_unknown_fields)]
enum Output {
    Reported { report: Box<ci::CiReport> },
    Aborted { exit_code: u8, detail: String },
}
pub fn execute(directory: &Path) -> Result<()> {
    eplyx_engine::local_store::verify_offline_environment()?;
    let mut bytes = vec![];
    std::fs::File::open(directory.join("request.json"))?
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 128 * 1024, "worker request exceeds bound");
    let request: Request = serde_json::from_slice(&bytes)?;
    let candidate = super::worker::member(directory, "candidate.so", &request.candidate)?;
    let expectations = request
        .expectations
        .as_ref()
        .map(|r| super::worker::member(directory, "expected-changes.toml", r))
        .transpose()?;
    let input = ci::ChangeInput::Spec {
        spec: &request.spec,
        source: Some(CandidateSource::Bytes(&candidate)),
    };
    let output = match ci::check_change(
        &request.bundle,
        &input,
        expectations
            .as_ref()
            .map(|_| directory.join("expected-changes.toml"))
            .as_deref(),
    ) {
        Ok(report) => Output::Reported {
            report: Box::new(report),
        },
        Err(error) => Output::Aborted {
            exit_code: error.exit_code(),
            detail: format!("{error:#}"),
        },
    };
    let bytes = serde_json::to_vec(&output)?;
    ensure!(
        bytes.len() <= 64 * 1024 * 1024,
        "worker report exceeds bound"
    );
    std::fs::write(directory.join("upgrade-output.json"), bytes)?;
    Ok(())
}
pub fn run(
    binary: &Path,
    directory: &Path,
    bundle: &Path,
    spec: &ChangeSpec,
    candidate: &[u8],
    expectations: Option<&Path>,
) -> Result<RunOutcome> {
    std::fs::create_dir_all(directory)?;
    std::fs::write(directory.join("candidate.so"), candidate)?;
    let expectations = expectations.map(std::fs::read).transpose()?;
    let request = Request {
        bundle: bundle.to_owned(),
        spec: spec.clone(),
        candidate: ArtifactRef::of(candidate),
        expectations: expectations.as_ref().map(|b| ArtifactRef::of(b)),
    };
    if let Some(bytes) = expectations {
        std::fs::write(directory.join("expected-changes.toml"), bytes)?;
    }
    std::fs::write(
        directory.join("request.json"),
        serde_json::to_vec(&request)?,
    )?;
    let mut child = Command::new(binary)
        .arg("offline-upgrade")
        .arg(directory)
        .env_clear()
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    super::process::wait(&mut child, Duration::from_secs(600))?;
    let mut bytes = vec![];
    std::fs::File::open(directory.join("upgrade-output.json"))?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 64 * 1024 * 1024,
        "worker report exceeds bound"
    );
    Ok(match serde_json::from_slice(&bytes)? {
        Output::Reported { report } => {
            let markdown = eplyx_engine::ci_markdown::render(&report);
            RunOutcome::Reported { report, markdown }
        }
        Output::Aborted { exit_code, detail } => RunOutcome::PreflightAbort { exit_code, detail },
    })
}
