//! Hosted Eplyx CI API.
//!
//! A thin transport layer over the engine. There is no second implementation of
//! comparison, semantics, expectations or review here, and the hosted result is
//! the same deterministic `CiReport` the local CLI produces for the same three
//! inputs.
//!
//! # Two workflows, deliberately separated
//!
//! ```text
//! PERIODIC / ADMINISTRATIVE          PER PULL REQUEST
//!
//! mainnet                            candidate.so
//!   ↓ historical acquisition           + expected-changes.toml
//! validated corpus                     + the project's active bundle
//!   ↓ selection                              ↓
//! CI bundle                          eplyx ci check
//!   ↓ operator activates                     ↓
//! project points at it               pass / fail
//! ```
//!
//! The left column needs an archive endpoint, takes minutes and is reviewed
//! when it changes. The right column needs no credentials at all. A pull request
//! never touches the left column, which is why a check can be fast, offline, and
//! stable enough that a green result last week means something today.
//!
//! # Candidate code is built outside Eplyx
//!
//! A security boundary, not a convenience. This service never clones a
//! repository, never runs a build script, never compiles uploaded source and
//! never executes a Dockerfile. GitHub Actions builds the `.so`; only those
//! bytes are uploaded, and they are executed solely inside the replay VM the
//! engine already sandboxes.

mod api;
mod config;
mod project;
mod registry;
mod storage;

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::api::AppState;
use crate::config::Config;
use crate::project::{generate_token, Project};
use crate::registry::Registry;
use crate::storage::Storage;

#[derive(Parser)]
#[command(
    name = "eplyx-server",
    about = "Hosted Eplyx CI API and its operator commands"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the API. The default when no subcommand is given.
    Serve,
    /// Operator-only administration. Never reachable over HTTP: a project's CI
    /// token can run checks, and nothing else. Replacing the bundle a project
    /// is measured against is not something a CI credential may do.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Create a project and print its CI token once.
    CreateProject {
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        program_id: String,
    },
    /// Verify a bundle and install it under its content hash.
    InstallBundle {
        #[arg(long)]
        path: std::path::PathBuf,
    },
    /// Point a project at an installed bundle. Deliberately a separate step
    /// from installing one: a corpus change moves what every pull request is
    /// measured against, so a human chooses when that happens.
    ActivateBundle {
        #[arg(long)]
        project: String,
        #[arg(long)]
        bundle: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::from_env()?;
    let storage = Storage::open(&config.data_dir)
        .with_context(|| format!("opening data directory {}", config.data_dir.display()))?;
    let registry = Registry::new(storage);

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(config, registry),
        Command::Admin { command } => admin(command, &registry),
    }
}

fn admin(command: AdminCommand, registry: &Registry) -> Result<()> {
    match command {
        AdminCommand::CreateProject {
            id,
            name,
            program_id,
        } => {
            let token = generate_token();
            let project = Project::new(&id, &name, &program_id, &token)?;
            registry.save_project(&project)?;
            // Printed once and never stored in this form. There is no endpoint
            // that can hand it back.
            println!("project {id} created for program {program_id}");
            println!("\nCI token (shown once, store it as the EPLYX_TOKEN secret):\n\n  {token}\n");
            println!("No bundle is active yet. Install one and activate it before checks can run.");
        }
        AdminCommand::InstallBundle { path } => {
            let sha256 = registry.install_bundle(&path)?;
            println!("installed bundle {sha256}");
            println!("It is not active. Activate it deliberately:");
            println!("  eplyx-server admin activate-bundle --project <id> --bundle {sha256}");
        }
        AdminCommand::ActivateBundle { project, bundle } => {
            registry.activate_bundle(&project, &bundle)?;
            println!("project {project} now checks against bundle {bundle}");
        }
    }
    Ok(())
}

fn serve(config: Config, registry: Registry) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let bind = config.bind;
        let state = Arc::new(AppState {
            runs: tokio::sync::Semaphore::new(config.max_concurrent_runs),
            config,
            registry,
        });
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .with_context(|| format!("binding {bind}"))?;
        // The data directory is printed; nothing else about the configuration
        // is, and no credential exists in this process to print.
        eprintln!("eplyx-server listening on {bind}");
        axum::serve(listener, api::router(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .context("serving")
    })
}
