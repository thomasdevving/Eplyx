//! `eplyx` - command line entry point.

mod cli_bundle;
mod cli_ci;
mod cli_compare;
mod cli_dashboard;
mod cli_governance;
mod cli_historical;
mod cli_interaction;
mod cli_lifecycle;
mod cli_local;
mod cli_parameter;
mod cli_path;
mod cli_qualification;
mod cli_rollout;

use std::path::PathBuf;
static LONG_VERSION: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(eplyx_engine::build_info::long_version);
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "eplyx",
    version = eplyx_engine::build_info::VERSION,
    long_version = LONG_VERSION.as_str(),
    about = "Local analysis of Solana program upgrades and token migrations",
    long_about = "Executes identical transactions against two builds of the same Solana program \
                  over a corpus of account states, and reports what changed solely because the \
                  program version changed. Token migration commands rehearse a proposed mechanism \
                  against separately captured or synthetic state in a local VM."
)]
struct Cli {
    /// Project configuration for migration and local store commands.
    #[arg(long, global = true, default_value = "eplyx.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Analyze a qualified local program-upgrade and parameter interaction.
    Interaction {
        #[command(subcommand)]
        command: cli_interaction::Command,
    },
    #[command(hide = true)]
    InteractionWorker { encoded: String },
    /// Rehearse a bounded installed program upgrade and configuration rollout order.
    Rollout {
        #[command(subcommand)]
        command: cli_rollout::Command,
    },
    #[command(hide = true)]
    RolloutWorker { encoded: String },
    /// Analyze one active Token-2022 transfer-fee parameter counterfactual.
    Parameter {
        #[command(subcommand)]
        command: cli_parameter::Command,
    },
    #[command(hide = true)]
    ParameterWorker { encoded: String },
    /// Sign in through a browser-approved device code.
    Login {
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        no_open: bool,
    },
    /// Revoke the CLI access token and remove its local credential entry.
    Logout {
        #[arg(long)]
        server: Option<String>,
    },
    /// Link a local store to a private hosted project.
    Link {
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        create: Option<String>,
        #[arg(long)]
        unlink: bool,
        #[arg(long)]
        force: bool,
    },
    /// Upload exact, privacy-checked analytical artifacts. Never executes.
    Sync {
        #[arg(long, conflicts_with_all=["latest","run_id"])]
        run: Option<String>,
        #[arg(value_name = "RUN", conflicts_with = "latest")]
        run_id: Option<String>,
        #[arg(long)]
        latest: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Browse saved analytical records on loopback. Executes no analysis.
    Dashboard {
        #[arg(long)]
        no_open: bool,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Check one exact current path against captured deployed code.
    Path {
        #[command(subcommand)]
        command: cli_path::PathCommand,
    },
    /// Capture or replay bounded read-only current observations.
    Observe {
        #[command(subcommand)]
        command: cli_path::ObserveCommand,
    },
    #[command(hide = true)]
    PathWorker { request: String },
    /// Analyse declared lifecycle changes.
    Lifecycle {
        #[command(subcommand)]
        command: cli_lifecycle::LifecycleCommand,
    },
    #[command(hide = true)]
    LifecycleWorker { request: String },
    /// Analyse and replay token migrations.
    Migration {
        #[command(subcommand)]
        command: cli_local::MigrationCommand,
    },
    /// Create a token migration project.
    Init {
        #[arg(long, required = true)]
        migration: bool,
        #[arg(long)]
        fixture: bool,
        #[arg(long)]
        force: bool,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// Validate project configuration and inputs without contacting a provider.
    Doctor {
        #[arg(long, hide = true)]
        offline: bool,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// List saved local run history.
    Runs {
        #[arg(long)]
        json: bool,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// Show saved local run history.
    Show {
        id: String,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// Show binary identity and supported schemas without a project.
    Version {
        #[arg(long)]
        json: bool,
    },
    #[command(hide = true)]
    FinishMigrationPreflight {
        input: PathBuf,
        #[arg(long)]
        result: PathBuf,
    },
    #[command(hide = true)]
    MigrationWorker(cli_local::WorkerArgs),
    /// Run the corpus against both program builds and report the differences.
    Compare(CompareArgs),
    /// Ingest program activity via standard Solana RPC into a local cache.
    Ingest(IngestArgs),
    /// Discover, classify, and select representative public-program activity.
    Discover(DiscoverArgs),
    /// Acquire exact slot-addressed historical state for a bounded mainnet path.
    Historical {
        #[command(subcommand)]
        command: HistoricalCommand,
    },
    /// Resolve historically deployed program versions from a slot-addressable archive.
    Versions {
        #[command(subcommand)]
        command: VersionsCommand,
    },
    /// Inspect standard-RPC history capabilities without assuming account archives.
    Rpc {
        #[command(subcommand)]
        command: RpcCommand,
    },
    /// Build or inspect discovery artifacts without contacting RPC.
    Discovery {
        #[command(subcommand)]
        command: DiscoveryCommand,
    },
    /// Select captured transactions into a durable offline replay corpus.
    Corpus {
        #[command(subcommand)]
        command: CorpusCommand,
    },
    /// Check a candidate program against a pinned bundle. The CI gate.
    Ci {
        #[command(subcommand)]
        command: CiCommand,
    },
    /// Describe a proposed change as a content-addressed change spec.
    Change {
        #[command(subcommand)]
        command: ChangeCommand,
    },
    /// Bind a governance proposal to an analysed change. Read-only: never
    /// signs, approves, rejects, cancels or executes anything.
    Governance {
        #[command(subcommand)]
        command: GovernanceCommand,
    },
    /// Assemble and verify the offline CI bundle a gate runs against.
    Bundle {
        #[command(subcommand)]
        command: BundleCommand,
    },
    /// Isolated local-validator setup and snapshot capture for the demo.
    Controlled {
        #[command(subcommand)]
        command: ControlledCommand,
    },
    /// Write the generated fixture corpus to disk as JSON.
    Generate(GenerateArgs),
    /// Replay a single fixture, or a regression group and its minimized
    /// counterexample, and print a detailed side-by-side view.
    Reproduce(ReproduceArgs),
    /// List the fixtures in the corpus.
    List(ListArgs),
}

#[derive(Clone, Copy, ValueEnum, serde::Serialize, serde::Deserialize)]
enum Format {
    Text,
    Json,
}

#[derive(Parser)]
struct CompareArgs {
    /// Durable replay corpus JSON. Runs offline with a V1 fidelity gate.
    #[arg(long)]
    corpus: Option<PathBuf>,
    /// Path to the V1 program artefact.
    #[arg(long, alias = "current")]
    v1: Option<PathBuf>,
    /// Path to the V2 program artefact.
    #[arg(long, alias = "candidate")]
    v2: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Restrict the run to one fixture ID.
    #[arg(long)]
    fixture: Option<String>,
    /// Restrict the run to one category.
    #[arg(long)]
    category: Option<String>,
    /// Write the report to a file instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Exit non-zero when critical regressions are found (for CI gates).
    #[arg(long)]
    fail_on_critical: bool,
    /// Skip counterexample minimization, which re-executes candidate states.
    #[arg(long)]
    no_minimize: bool,
    /// Directory holding the historical dependency binaries a replay corpus
    /// pins. Defaults to a `dependencies` directory beside the corpus file.
    #[arg(long)]
    dependencies: Option<PathBuf>,
}

#[derive(Parser)]
struct GenerateArgs {
    /// Directory to write fixture JSON into.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Parser)]
struct ReproduceArgs {
    /// Fixture ID (e.g. boundary-position-017) or regression group ID
    /// (e.g. newly-liquidatable, or regression-group:newly-liquidatable).
    target: String,
    #[arg(long, alias = "current")]
    v1: Option<PathBuf>,
    #[arg(long, alias = "candidate")]
    v2: Option<PathBuf>,
    /// Skip minimization when reproducing a regression group.
    #[arg(long)]
    no_minimize: bool,
}

#[derive(Parser)]
struct ListArgs {
    #[arg(long)]
    category: Option<String>,
}

#[derive(Parser)]
struct IngestArgs {
    #[arg(long)]
    program: String,
    /// Falls back to SOLANA_RPC_URL; endpoint is never persisted in reports.
    #[arg(long)]
    rpc_url: Option<String>,
    #[arg(long)]
    start_slot: u64,
    #[arg(long)]
    end_slot: u64,
    /// Use a separate cache directory per chain/endpoint.
    #[arg(long)]
    cache: PathBuf,
}

#[derive(Parser)]
struct DiscoverArgs {
    #[arg(long)]
    program: String,
    /// Falls back to SOLANA_RPC_URL; never persisted.
    #[arg(long)]
    rpc_url: Option<String>,
    /// Origin header for endpoints with an allowlist. Falls back to
    /// SOLANA_RPC_ORIGIN and is never persisted.
    #[arg(long)]
    rpc_origin: Option<String>,
    /// Defaults to a bounded 5,000-slot window ending at cached getSlot.
    #[arg(long)]
    start_slot: Option<u64>,
    #[arg(long)]
    end_slot: Option<u64>,
    /// Maximum normalized source interactions (newest in the window).
    #[arg(long, default_value_t = 500)]
    limit: usize,
    /// Maximum interactions selected into the discovery corpus.
    #[arg(long, default_value_t = 250)]
    corpus_size: u64,
    /// Output session directory containing cache, corpus JSON, and text report.
    #[arg(long)]
    output: PathBuf,
    /// Maximum concurrent transaction-history requests. Current account samples
    /// remain serialized for conservative public-provider behavior.
    #[arg(long, default_value_t = 1)]
    concurrency: u64,
    #[arg(long, default_value_t = 3)]
    retries: u32,
    #[arg(long, default_value_t = 250)]
    backoff_ms: u64,
}

#[derive(Subcommand)]
enum HistoricalCommand {
    /// Build one HistoricalStateReady System-transfer/Memo replay record and V1 artifact.
    Acquire(HistoricalAcquireArgs),
}

#[derive(Parser)]
struct HistoricalAcquireArgs {
    #[arg(long)]
    signature: String,
    /// Program whose upgrade is under test. Selects a protocol adapter and the
    /// historically deployed V1 binary. Omitted, the bounded Phase 6
    /// System-transfer/Memo contract is used.
    #[arg(long)]
    program: Option<String>,
    /// Standard transaction-history RPC; falls back to SOLANA_RPC_URL.
    #[arg(long)]
    transaction_rpc_url: Option<String>,
    /// Slot-addressable account archive; falls back to SOLANA_ARCHIVE_RPC_URL,
    /// then the transaction RPC URL.
    #[arg(long)]
    archive_rpc_url: Option<String>,
    /// Block source for same-slot interference screening; falls back to
    /// SOLANA_BLOCK_RPC_URL, then the transaction RPC URL.
    ///
    /// A protocol whose contract admits cross-program invocation is screened
    /// whether or not this is given, because it cannot be acquired without the
    /// evidence. For any other protocol, screening happens only when a source is
    /// named: the boundary proof those paths were established under does not
    /// depend on it, and a block is megabytes of response.
    #[arg(long)]
    block_rpc_url: Option<String>,
    /// Optional Origin header for allowlisted demo endpoints; falls back to
    /// SOLANA_RPC_ORIGIN and is never persisted.
    #[arg(long)]
    rpc_origin: Option<String>,
    /// Session directory containing immutable transport cache and durable output.
    #[arg(long)]
    output: PathBuf,
    /// Refuse transport access and require every request to exist in the cache.
    #[arg(long)]
    offline: bool,
}

#[derive(Subcommand)]
enum VersionsCommand {
    /// Resolve the program version that was live at one slot.
    Resolve(VersionsResolveArgs),
    /// Locate upgrade boundaries in a slot range by bisecting deployment slots.
    Upgrades(VersionsUpgradesArgs),
}

/// Transport and cache options shared by the version-resolution commands.
#[derive(Parser)]
struct ArchiveArgs {
    /// Slot-addressable account archive; falls back to SOLANA_ARCHIVE_RPC_URL,
    /// then SOLANA_RPC_URL.
    #[arg(long)]
    archive_rpc_url: Option<String>,
    /// Optional Origin header for allowlisted demo endpoints; never persisted.
    #[arg(long)]
    rpc_origin: Option<String>,
    /// Session directory holding the immutable transport cache.
    #[arg(long)]
    output: PathBuf,
    /// Refuse transport access and require every request to exist in the cache.
    #[arg(long)]
    offline: bool,
}

#[derive(Parser)]
struct VersionsResolveArgs {
    #[arg(long)]
    program: String,
    /// Slot to resolve the version at.
    #[arg(long)]
    slot: u64,
    /// Write the resolved executable bytes here.
    #[arg(long)]
    out: Option<PathBuf>,
    #[command(flatten)]
    archive: ArchiveArgs,
}

#[derive(Parser)]
struct VersionsUpgradesArgs {
    #[arg(long)]
    program: String,
    #[arg(long)]
    start_slot: u64,
    #[arg(long)]
    end_slot: u64,
    #[command(flatten)]
    archive: ArchiveArgs,
}

#[derive(Subcommand)]
enum RpcCommand {
    Inspect {
        #[arg(long)]
        rpc_url: Option<String>,
        /// Optional address used to probe signature and transaction history.
        #[arg(long)]
        program: Option<String>,
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
}

#[derive(Subcommand)]
enum DiscoveryCommand {
    /// Select a discovery corpus from an existing ingestion cache.
    Build {
        #[arg(long)]
        cache: PathBuf,
        /// Optional Phase 4 snapshots, validated before exact-ready labeling.
        #[arg(long)]
        snapshots: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 250)]
        corpus_size: u64,
    },
}
#[derive(Parser)]
struct CorpusSelectArgs {
    /// Directory holding a durable corpus: manifest.json, records/, corpus.json.
    #[arg(long)]
    corpus: PathBuf,
    /// How many observations to select. Never padded by duplication.
    #[arg(long, default_value_t = 10)]
    target_size: usize,
    /// Optional discovery counts as JSON, e.g. {"deposit":681,"withdraw":200}.
    /// An absent count is reported as not measured rather than as zero.
    #[arg(long)]
    observed: Option<String>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Subcommand)]
enum CorpusCommand {
    /// Publish the deterministic index for an existing immutable record store.
    Publish {
        #[arg(long)]
        corpus: PathBuf,
    },
    Build {
        #[arg(long)]
        cache: PathBuf,
        #[arg(long)]
        snapshots: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Select a deterministic production-derived regression corpus.
    Select(CorpusSelectArgs),
}
#[derive(Subcommand)]
enum CiCommand {
    /// Replay a pinned corpus against a candidate and gate on the result.
    Check(CiCheckArgs),
}

#[derive(Parser)]
struct CiCheckArgs {
    /// Directory holding the pinned, offline CI bundle.
    #[arg(long)]
    bundle: PathBuf,
    /// The candidate program artefact under test. Alone, it stands for an
    /// upgrade of the bundle's program to exactly these bytes. With
    /// --change-spec, it supplies the bytes the spec names and must be them.
    #[arg(long, required_unless_present = "change_spec")]
    candidate: Option<PathBuf>,
    /// The proposed change, as written by `eplyx change program-upgrade`.
    #[arg(long)]
    change_spec: Option<PathBuf>,
    /// Content-addressed store holding the spec's candidate at
    /// `programs/<sha256>`, instead of --candidate.
    #[arg(long, requires = "change_spec", conflicts_with = "candidate")]
    artifacts: Option<PathBuf>,
    /// Declared intentional changes. Omitted, nothing is declared and every
    /// finding is unexpected, which is the correct default.
    #[arg(long)]
    expectations: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Write the report to a file instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Subcommand)]
enum GovernanceCommand {
    /// Squads V4 vault transactions carrying one loader-v3 program upgrade.
    Squads {
        #[command(subcommand)]
        command: SquadsCommand,
    },
}

#[derive(Subcommand)]
enum SquadsCommand {
    /// Check a proposal against an analysed change spec and write the
    /// governance-bound spec an analysis must carry to name the proposal.
    Bind(SquadsBindArgs),
    /// Re-read a governance-bound spec's proposal and buffer now. Run it
    /// immediately before approving or executing.
    Verify(SquadsVerifyArgs),
    /// Derive the analysed spec a proposal implies from the chain, storing
    /// the buffer's current bytes as a content-addressed candidate.
    Acquire(SquadsAcquireArgs),
    /// Verify execution and deployed bytes using a sealed pre-execution G1
    /// binding and the content-addressed P2 candidate store.
    Attest(SquadsAttestArgs),
}

#[derive(Parser)]
struct SquadsAttestArgs {
    #[arg(long)]
    change_spec: PathBuf,
    #[arg(long)]
    binding: PathBuf,
    /// P2 content-addressed store root containing programs/<sha256>.
    #[arg(long)]
    artifacts: PathBuf,
    #[arg(long)]
    evidence_out: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[command(flatten)]
    chain: SquadsChainArgs,
}

#[derive(Parser)]
struct SquadsChainArgs {
    /// JSON-RPC endpoint; falls back to SOLANA_RPC_URL. Never persisted.
    #[arg(long)]
    rpc_url: Option<String>,
    #[arg(long, value_enum, default_value_t = CommitmentArg::Finalized)]
    commitment: CommitmentArg,
}

#[derive(Clone, Copy, ValueEnum)]
enum CommitmentArg {
    Confirmed,
    Finalized,
}

#[derive(Parser)]
struct SquadsBindArgs {
    #[arg(long)]
    multisig: String,
    #[arg(long)]
    transaction_index: u64,
    /// The analysed change spec (bound or not).
    #[arg(long)]
    change_spec: PathBuf,
    /// Write the governance-bound spec here. Written only on `matched`.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Write the sealed binding evidence here, whatever the outcome.
    #[arg(long)]
    evidence_out: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[command(flatten)]
    chain: SquadsChainArgs,
}

#[derive(Parser)]
struct SquadsVerifyArgs {
    /// A governance-bound change spec (one with a Squads `delivery`).
    #[arg(long)]
    change_spec: PathBuf,
    /// Assert the proposal is this multisig's; defaults to the spec's.
    #[arg(long)]
    multisig: Option<String>,
    /// Assert the proposal is this transaction; defaults to the spec's.
    #[arg(long)]
    transaction_index: Option<u64>,
    #[arg(long)]
    evidence_out: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[command(flatten)]
    chain: SquadsChainArgs,
}

#[derive(Parser)]
struct SquadsAcquireArgs {
    #[arg(long)]
    multisig: String,
    #[arg(long)]
    transaction_index: u64,
    /// Content-addressed artifact store to put the buffer's bytes in.
    #[arg(long)]
    store: PathBuf,
    /// Write the (unbound) analysed spec here.
    #[arg(long)]
    out: PathBuf,
    #[command(flatten)]
    chain: SquadsChainArgs,
}

#[derive(Subcommand)]
enum ChangeCommand {
    Lifecycle(cli_lifecycle::ChangeArgs),
    /// Describe a token migration with its exact candidate bytes.
    TokenMigration(cli_local::ChangeArgs),
    /// Write the change spec for upgrading one program to one executable.
    ProgramUpgrade(ChangeProgramUpgradeArgs),
}

#[derive(Parser)]
struct ChangeProgramUpgradeArgs {
    /// The program being upgraded.
    #[arg(long)]
    program: String,
    /// The proposed executable. Named in the spec by content hash only.
    #[arg(long)]
    candidate: PathBuf,
    /// Also require the target to keep its bytes in this ProgramData account.
    #[arg(long)]
    programdata: Option<String>,
    /// Also require the baseline to be exactly this executable.
    #[arg(long)]
    replaces: Option<PathBuf>,
    /// Also require the baseline to prove this upgrade authority.
    #[arg(long)]
    upgrade_authority: Option<String>,
    #[arg(long)]
    activation_slot: Option<u64>,
    #[arg(long)]
    activation_unix_timestamp: Option<i64>,
    /// Display label. Not part of the spec's identity.
    #[arg(long)]
    label: Option<String>,
    /// Also store the candidate in this content-addressed artifact store.
    #[arg(long)]
    store: Option<PathBuf>,
    /// Write the spec here instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Subcommand)]
enum BundleCommand {
    /// Qualify a bounded schema-1 corpus from declared scope and historical providers.
    Prepare(cli_qualification::PrepareArgs),
    /// Assemble an offline-executable bundle from a validated corpus.
    Build(BundleBuildArgs),
    /// Re-hash every byte a bundle pins and report what it covers.
    Verify(BundleVerifyArgs),
}

#[derive(Parser)]
struct BundleBuildArgs {
    /// Directory holding a durable corpus: manifest.json, records/, corpus.json.
    #[arg(long)]
    corpus: PathBuf,
    /// The V1 binary every record was validated against.
    #[arg(long)]
    baseline: PathBuf,
    /// Directory holding the dependency artefacts the records pin. Defaults to
    /// a `dependencies` directory beside the corpus.
    #[arg(long)]
    dependencies: Option<PathBuf>,
    /// Select down to this many observations first. Omitted, every validated
    /// record in the corpus is bundled.
    #[arg(long)]
    target_size: Option<usize>,
    /// Optional discovery counts as JSON, e.g. {"deposit":681,"withdraw":200}.
    #[arg(long)]
    observed: Option<String>,
    #[arg(long)]
    out: PathBuf,
}

#[derive(Parser)]
struct BundleVerifyArgs {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Subcommand)]
enum ControlledCommand {
    Prepare {
        #[arg(long)]
        dir: PathBuf,
    },
    Capture {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        snapshots: PathBuf,
        #[arg(long)]
        rpc_url: String,
        #[arg(long)]
        current: PathBuf,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let args: Vec<String> = std::env::args().collect();
            let json = args.iter().any(|a| a == "--format=json")
                || args
                    .windows(2)
                    .any(|a| a[0] == "--format" && a[1] == "json");
            if json && error.exit_code() != 0 {
                println!(
                    "{}",
                    serde_json::json!({"error":{"code":2,"message":error.to_string()}})
                );
                return Ok(ExitCode::from(2));
            }
            error.print()?;
            return Ok(ExitCode::from(error.exit_code() as u8));
        }
    };
    match cli.command {
        Command::Login { server, no_open } => Ok(ExitCode::from(
            eplyx_engine::cloud::commands::login(server.as_deref(), !no_open)?,
        )),
        Command::Logout { server } => Ok(ExitCode::from(eplyx_engine::cloud::commands::logout(
            server.as_deref(),
        )?)),
        Command::Link {
            server,
            project,
            workspace,
            create,
            unlink,
            force,
        } => {
            let root = cloud_project_root(&cli.config)?;
            Ok(ExitCode::from(eplyx_engine::cloud::commands::link(
                &root.join(".eplyx"),
                eplyx_engine::cloud::commands::LinkArgs {
                    server: server.as_deref(),
                    project: project.as_deref(),
                    workspace: workspace.as_deref(),
                    create: create.as_deref(),
                    unlink,
                    force,
                },
            )?))
        }
        Command::Sync {
            run,
            run_id,
            latest,
            dry_run,
            json,
        } => {
            let root = cloud_project_root(&cli.config)?;
            Ok(ExitCode::from(eplyx_engine::cloud::commands::sync(
                &root,
                &root.join(".eplyx"),
                eplyx_engine::cloud::commands::SyncArgs {
                    run: run.as_deref().or(run_id.as_deref()),
                    latest,
                    dry_run,
                    json,
                },
            )?))
        }
        Command::Dashboard { no_open, port } => cli_dashboard::execute(&cli.config, no_open, port),
        Command::Path { command } => cli_path::execute(cli_path::Request::Path(command)),
        Command::Observe { command } => cli_path::execute(cli_path::Request::Observe(command)),
        Command::PathWorker { request } => cli_path::worker(&request),
        Command::Lifecycle { command } => cli_lifecycle::execute(command),
        Command::LifecycleWorker { request } => cli_lifecycle::worker(&request),
        Command::Change {
            command: ChangeCommand::Lifecycle(args),
        } => cli_lifecycle::change(args),
        Command::Migration { command } => Ok(cli_local::execute(&cli.config, command)),
        Command::Init {
            migration: _,
            fixture,
            force,
            format,
        } => Ok(cli_local::init(&cli.config, fixture, force, format)),
        Command::Doctor { offline: _, format } => Ok(cli_local::doctor(&cli.config, format)),
        Command::Runs { json, format } => Ok(cli_local::runs(
            &cli.config,
            if json { Format::Json } else { format },
        )),
        Command::Show { id, format } => Ok(cli_local::show(&cli.config, &id, format)),
        Command::Version { json } => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&eplyx_engine::build_info::json())?
                );
            } else {
                println!("eplyx {}", eplyx_engine::build_info::long_version());
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::FinishMigrationPreflight { input, result } => {
            eplyx_engine::local_store::verify_offline_environment()?;
            eplyx_engine::migration::pipeline::finish(&input, &result)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::MigrationWorker(args) => Ok(cli_local::worker_entry(args)),
        Command::Change {
            command: ChangeCommand::TokenMigration(args),
        } => Ok(cli_local::change(args)),
        Command::Compare(args) => cli_compare::compare(args),
        Command::Ingest(args) => cli_historical::ingest_command(args),
        Command::Discover(args) => cli_historical::discover_command(args),
        Command::Historical { command } => match command {
            HistoricalCommand::Acquire(args) => cli_historical::historical_acquire(args),
        },
        Command::Versions { command } => match command {
            VersionsCommand::Resolve(args) => cli_historical::versions_resolve(args),
            VersionsCommand::Upgrades(args) => cli_historical::versions_upgrades(args),
        },
        Command::Rpc { command } => match command {
            RpcCommand::Inspect {
                rpc_url,
                program,
                format,
            } => cli_historical::rpc_inspect(rpc_url, program, format),
        },
        Command::Discovery { command } => match command {
            DiscoveryCommand::Build {
                cache,
                snapshots,
                out,
                corpus_size,
            } => cli_historical::discovery_build(cache, snapshots, out, corpus_size),
        },
        Command::Corpus {
            command: CorpusCommand::Select(select_args),
        } => cli_bundle::corpus_select(select_args),
        Command::Corpus {
            command: CorpusCommand::Publish { corpus },
        } => cli_bundle::corpus_publish(corpus),
        Command::Ci {
            command: CiCommand::Check(check_args),
        } => cli_ci::ci_check(check_args),
        Command::Change {
            command: ChangeCommand::ProgramUpgrade(change_args),
        } => cli_ci::change_program_upgrade(change_args),
        Command::Governance {
            command: GovernanceCommand::Squads { command },
        } => match command {
            SquadsCommand::Bind(args) => cli_governance::squads_bind(args),
            SquadsCommand::Verify(args) => cli_governance::squads_verify(args),
            SquadsCommand::Acquire(args) => cli_governance::squads_acquire(args),
            SquadsCommand::Attest(args) => cli_governance::squads_attest(args),
        },
        Command::Bundle {
            command: BundleCommand::Prepare(args),
        } => cli_qualification::run(args),
        Command::Bundle {
            command: BundleCommand::Build(build_args),
        } => cli_bundle::bundle_build(build_args),
        Command::Bundle {
            command: BundleCommand::Verify(verify_args),
        } => cli_bundle::bundle_verify(verify_args),
        Command::Corpus {
            command:
                CorpusCommand::Build {
                    cache,
                    snapshots,
                    out,
                    limit,
                },
        } => cli_bundle::corpus_build(cache, snapshots, out, limit),
        Command::Controlled { command } => cli_historical::controlled(command),
        Command::Interaction { command } => cli_interaction::run(command),
        Command::InteractionWorker { encoded } => cli_interaction::worker(&encoded),
        Command::Rollout { command } => cli_rollout::run(command),
        Command::RolloutWorker { encoded } => cli_rollout::worker(&encoded),
        Command::Parameter { command } => cli_parameter::run(command),
        Command::ParameterWorker { encoded } => cli_parameter::worker(&encoded),
        Command::Generate(args) => cli_compare::generate(args),
        Command::Reproduce(args) => cli_compare::reproduce(args),
        Command::List(args) => cli_compare::list(args),
    }
}

fn cloud_project_root(config: &std::path::Path) -> Result<PathBuf> {
    let absolute = if config.is_absolute() {
        config.to_owned()
    } else {
        std::env::current_dir()?.join(config)
    };
    Ok(absolute
        .parent()
        .context("configuration has no project root")?
        .canonicalize()?)
}
