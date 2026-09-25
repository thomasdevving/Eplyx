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

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use eplyx_server::api::{self, AppState};
use eplyx_server::config::Config;
use eplyx_server::project::{generate_token, AdapterId, Project, ProjectToken};
use eplyx_server::registry::Registry;
use eplyx_server::storage::Storage;

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
    /// Create a project. Its id is minted here, never chosen.
    CreateProject {
        #[arg(long)]
        name: String,
        #[arg(long)]
        program_id: String,
        /// Defaults to whatever adapter this build speaks for the program,
        /// which is the only value the project may declare anyway.
        #[arg(long)]
        adapter: Option<String>,
    },
    /// List projects and where each one stands.
    ListProjects,
    /// Issue a project API token and print it once.
    CreateToken {
        #[arg(long)]
        project: String,
        #[arg(long, default_value = "operator")]
        label: String,
    },
    /// Verify a bundle and register it to a project.
    RegisterBundle {
        #[arg(long)]
        project: String,
        #[arg(long)]
        path: std::path::PathBuf,
    },
    /// Point a project at one of its registered bundles. Deliberately a
    /// separate step from registering one: a corpus change moves what every
    /// pull request is measured against, so a human chooses when that happens.
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
        Command::Serve => {
            // Held for the life of the process. Recovery below re-enqueues every
            // non-terminal run on this volume, which is only safe if nothing
            // else can be executing them.
            let _lock = registry.storage().lock_exclusive()?;
            let swept = registry.artifacts().sweep_temporary()?;
            if swept > 0 {
                eprintln!(
                    "removed {swept} incomplete artifact write(s) left by a previous process"
                );
            }
            let recovery = registry
                .recover_runs()
                .context("reconciling runs a previous process left unfinished")?;
            for (what, ids) in [
                ("re-enqueued", &recovery.requeued),
                (
                    "finalized from an already-written report",
                    &recovery.finalized,
                ),
                (
                    "could not be recovered and are now execution_error",
                    &recovery.failed,
                ),
            ] {
                if !ids.is_empty() {
                    eprintln!("{} run(s) {what}: {}", ids.len(), ids.join(", "));
                }
            }
            serve(config, registry, recovery.requeued)
        }
        Command::Admin { command } => admin(command, &registry),
    }
}

fn admin(command: AdminCommand, registry: &Registry) -> Result<()> {
    match command {
        AdminCommand::CreateProject {
            name,
            program_id,
            adapter,
        } => {
            let adapter_id = match adapter {
                Some(text) => AdapterId::try_from(text)?,
                None => AdapterId::for_program(&program_id),
            };
            let project_id = eplyx_server::ids::project();
            let project = Project::new(&project_id, &name, &program_id, adapter_id)?;
            registry.create_project(&project)?;
            println!("project {project_id} created for program {program_id}");
            println!("  adapter {}", project.adapter_id);
            if !project.adapter_id.speaks_semantics() {
                println!(
                    "  note: this build speaks no semantics for that program, so every check \n\
                     \twill report no semantic coverage and fail. That is the engine saying it \n\
                     \tdid not look, not that nothing is wrong."
                );
            }
            println!(
                "\nNo bundle is active yet. Register one and activate it before checks can run."
            );
        }
        AdminCommand::ListProjects => {
            for project in registry.list_projects()? {
                let active = project
                    .active_bundle
                    .as_ref()
                    .map(|bundle| bundle.bundle_id.clone())
                    .unwrap_or_else(|| "-".to_string());
                println!(
                    "{}  {:?}  {}  {}  {}",
                    project.project_id, project.status, project.adapter_id, active, project.name
                );
            }
        }
        AdminCommand::CreateToken { project, label } => {
            let secret = generate_token();
            let token = ProjectToken::new(&eplyx_server::ids::token(), &project, &label, &secret)?;
            registry.load_project(&project)?;
            registry.create_token(&token)?;
            // Printed once and never stored in this form. There is no endpoint
            // that can hand it back.
            println!("token {} created for project {project}", token.token_id);
            println!(
                "\nAPI token (shown once, store it as the EPLYX_TOKEN secret):\n\n  {secret}\n"
            );
        }
        AdminCommand::RegisterBundle { project, path } => {
            let record = registry.load_project(&project)?;
            let bundle = registry.register_bundle(&record, &path, None)?;
            println!(
                "registered bundle {} ({})",
                bundle.bundle_id, bundle.bundle_sha256
            );
            println!("It is not active. Activate it deliberately:");
            println!(
                "  eplyx-server admin activate-bundle --project {project} --bundle {}",
                bundle.bundle_id
            );
        }
        AdminCommand::ActivateBundle { project, bundle } => {
            let updated = registry.activate_bundle(&project, &bundle)?;
            println!(
                "project {project} now checks against bundle {bundle} ({:?})",
                updated.status
            );
        }
    }
    Ok(())
}

fn serve(config: Config, registry: Registry, requeued: Vec<String>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let bind = config.bind;
        let concurrency = config.max_concurrent_runs;
        let governance = match &config.governance_rpc_url {
            Some(url) => Some(
                Arc::new(eplyx_engine::ingest::rpc::HttpRpc::new(url.clone())?)
                    as Arc<dyn eplyx_engine::ingest::rpc::RpcProvider + Send + Sync>,
            ),
            None => None,
        };
        let state = Arc::new(AppState {
            runs: tokio::sync::Semaphore::new(config.max_concurrent_runs),
            config,
            registry,
            governance,
        });
        // Recovered work goes back behind the same semaphore as new work.
        eplyx_server::worker::resume(&state, &requeued);
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .with_context(|| format!("binding {bind}"))?;
        // The data directory is printed; nothing else about the configuration
        // is, and no credential exists in this process to print.
        eprintln!("eplyx-server listening on {bind}, {concurrency} concurrent run(s)");
        axum::serve(listener, api::router(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .context("serving")
    })
}
