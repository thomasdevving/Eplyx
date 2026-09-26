//! Read-only loopback dashboard. Configuration is optional and RPC is never read.
use super::cli_local::config::MigrationConfig;
use anyhow::{Context, Result};
use eplyx_engine::{build_info, dashboard::Dashboard};
use serde_json::json;
use std::{
    io::Write,
    path::Path,
    process::{Command, ExitCode},
};

pub fn execute(config: &Path, no_open: bool, port: Option<u16>) -> Result<ExitCode> {
    let absolute = if config.is_absolute() {
        config.to_owned()
    } else {
        std::env::current_dir()?.join(config)
    };
    let root = absolute
        .parent()
        .context("configuration has no project root")?
        .canonicalize()?;
    let context = if absolute.exists() {
        let parsed = super::cli_local::bounded_read(&absolute, 1024 * 1024).and_then(|bytes| {
            Ok(toml::from_str::<MigrationConfig>(std::str::from_utf8(
                &bytes,
            )?)?)
        });
        match parsed {
            Ok(config) => {
                json!({"state":"Valid","name":config.project.name,"gate_policy":config.gate.policy})
            }
            Err(_) => {
                json!({"state":"Invalid","error":"The project configuration could not be parsed."})
            }
        }
    } else {
        json!({"state":"Missing"})
    };
    let dashboard = Dashboard::bind(
        &root,
        port,
        json!({"version":build_info::VERSION,"config":context}),
    )?;
    let (name, runs, counterexamples) = dashboard.banner()?;
    let url = dashboard.url();
    println!("Eplyx dashboard\nProject: {name}\nRuns: {runs}\nCounterexamples: {counterexamples}\n{url}\nRead-only local analysis. Press Ctrl+C to stop.");
    std::io::stdout().flush()?;
    if !no_open {
        #[cfg(target_os = "macos")]
        let mut opener = Command::new("/usr/bin/open");
        #[cfg(target_os = "windows")]
        let mut opener = {
            let mut c = Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut opener = Command::new("xdg-open");
        let _ = opener.arg(&url).spawn();
    }
    dashboard.serve()?;
    Ok(ExitCode::SUCCESS)
}
