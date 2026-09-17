//! `eplyx` - command line entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use eplyx_engine::{
    compare_all, corpus, default_artifact, fixture_program_id, load_versions, report::render_text,
    report::Report, types::Category,
};

#[derive(Parser)]
#[command(
    name = "eplyx",
    about = "Deterministic differential execution for Solana program upgrades",
    long_about = "Executes identical transactions against two builds of the same Solana program \
                  over a corpus of account states, and reports what changed solely because the \
                  program version changed."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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

#[derive(Clone, Copy, ValueEnum)]
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
    /// The candidate program artefact under test.
    #[arg(long)]
    candidate: PathBuf,
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
enum BundleCommand {
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
fn endpoint(explicit: Option<String>) -> Result<String> {
    explicit
        .or_else(|| std::env::var("SOLANA_RPC_URL").ok())
        .context("provide --rpc-url or SOLANA_RPC_URL")
}

/// Build a cached archive transport. The endpoint namespaces the cache by hash
/// so credentials never reach disk and two endpoints never share entries.
fn archive_transport(
    args: &ArchiveArgs,
) -> Result<(
    Box<dyn eplyx_engine::ingest::rpc::RpcProvider>,
    std::path::PathBuf,
)> {
    use eplyx_engine::ingest::rpc::{HttpRpc, OfflineRpc, RpcProvider};
    let url = args
        .archive_rpc_url
        .clone()
        .or_else(|| std::env::var("SOLANA_ARCHIVE_RPC_URL").ok())
        .or_else(|| std::env::var("SOLANA_RPC_URL").ok())
        .context("provide --archive-rpc-url, SOLANA_ARCHIVE_RPC_URL or SOLANA_RPC_URL")?;
    let origin = args
        .rpc_origin
        .clone()
        .or_else(|| std::env::var("SOLANA_RPC_ORIGIN").ok());
    let root = args
        .output
        .join("cache")
        .join("accounts")
        .join(eplyx_engine::replay::hash_bytes(url.as_bytes()));
    let transport: Box<dyn RpcProvider> = if args.offline {
        Box::new(OfflineRpc)
    } else {
        let rpc = HttpRpc::new(url)?;
        match origin {
            Some(origin) => Box::new(rpc.with_origin(origin)?),
            None => Box::new(rpc),
        }
    };
    Ok((transport, root))
}

fn versions_resolve(args: VersionsResolveArgs) -> Result<ExitCode> {
    let (transport, root) = archive_transport(&args.archive)?;
    let retrying = eplyx_engine::ingest::rpc::RetryingRpc {
        provider: transport.as_ref(),
        max_retries: 5,
        base_backoff: std::time::Duration::from_millis(500),
    };
    let rpc = eplyx_engine::ingest::CachedRpc {
        provider: &retrying,
        root,
    };
    let start = std::time::Instant::now();
    let resolved = eplyx_engine::versions::resolve_at(&rpc, &args.program, args.slot)?;
    if let Some(out) = &args.out {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = out.with_extension("so.tmp");
        std::fs::write(&temporary, &resolved.elf)?;
        std::fs::rename(temporary, out)?;
    }
    println!(
        "Program:           {}\nObserved at slot:  {}\nLoader:            {:?}\nDeployed at slot:  {}\nUpgrade authority: {}\nProgramData:       {}\nExecutable bytes:  {}\nSHA-256:           {}\nResolution:        {} ms{}",
        resolved.program_id,
        resolved.observed_slot,
        resolved.loader,
        resolved
            .deploy_slot
            .map(|slot| slot.to_string())
            .unwrap_or_else(|| "n/a (legacy loader records none)".into()),
        resolved
            .upgrade_authority
            .clone()
            .unwrap_or_else(|| "none (immutable)".into()),
        resolved
            .programdata_address
            .clone()
            .unwrap_or_else(|| "n/a".into()),
        resolved.elf.len(),
        resolved.sha256,
        start.elapsed().as_millis(),
        args.out
            .as_ref()
            .map(|p| format!("\nArtifact:          {}", p.display()))
            .unwrap_or_default(),
    );
    Ok(ExitCode::SUCCESS)
}

fn versions_upgrades(args: VersionsUpgradesArgs) -> Result<ExitCode> {
    let (transport, root) = archive_transport(&args.archive)?;
    let retrying = eplyx_engine::ingest::rpc::RetryingRpc {
        provider: transport.as_ref(),
        max_retries: 5,
        base_backoff: std::time::Duration::from_millis(500),
    };
    let rpc = eplyx_engine::ingest::CachedRpc {
        provider: &retrying,
        root,
    };
    let start = std::time::Instant::now();
    // Resolving at the end slot is what yields the ProgramData address; the
    // search itself then reads only 45-byte headers.
    let head = eplyx_engine::versions::resolve_at(&rpc, &args.program, args.end_slot)?;
    let programdata = head
        .programdata_address
        .context("upgrade search requires an upgradeable-loader program")?;
    let boundaries =
        eplyx_engine::versions::find_upgrades(&rpc, &programdata, args.start_slot, args.end_slot)?;
    println!(
        "Program:     {}\nProgramData: {}\nRange:       {}..={}\nUpgrades:    {}",
        args.program,
        programdata,
        args.start_slot,
        args.end_slot,
        boundaries.len()
    );
    for boundary in &boundaries {
        println!(
            "  upgrade at slot {} (previous deployment {} was live through slot {})",
            boundary.at, boundary.previous_deploy_slot, boundary.before
        );
    }
    println!("Search: {} ms", start.elapsed().as_millis());
    Ok(ExitCode::SUCCESS)
}

fn historical_acquire(args: HistoricalAcquireArgs) -> Result<ExitCode> {
    use eplyx_engine::{
        historical::HistoricalStateProvider,
        ingest::rpc::{HttpRpc, OfflineRpc, RpcProvider},
    };
    let transaction_url = endpoint(args.transaction_rpc_url)?;
    let archive_url = args
        .archive_rpc_url
        .or_else(|| std::env::var("SOLANA_ARCHIVE_RPC_URL").ok())
        .unwrap_or_else(|| transaction_url.clone());
    let origin = args
        .rpc_origin
        .or_else(|| std::env::var("SOLANA_RPC_ORIGIN").ok());
    let transport = |url: String| -> Result<Box<dyn RpcProvider>> {
        if args.offline {
            return Ok(Box::new(OfflineRpc));
        }
        let rpc = HttpRpc::new(url)?;
        Ok(match origin.clone() {
            Some(origin) => Box::new(rpc.with_origin(origin)?),
            None => Box::new(rpc),
        })
    };
    let explicit_block_source =
        args.block_rpc_url.is_some() || std::env::var("SOLANA_BLOCK_RPC_URL").is_ok();
    let block_url = args
        .block_rpc_url
        .or_else(|| std::env::var("SOLANA_BLOCK_RPC_URL").ok())
        .unwrap_or_else(|| transaction_url.clone());
    let transaction_transport = transport(transaction_url.clone())?;
    let archive_transport = transport(archive_url.clone())?;
    let block_transport = transport(block_url.clone())?;
    // Retry sits inside the cache: a cache hit never consumes retry budget, and
    // a shared public endpoint's rate limiting does not abort an acquisition
    // that is otherwise complete.
    let transaction_retrying = eplyx_engine::ingest::rpc::RetryingRpc {
        provider: transaction_transport.as_ref(),
        max_retries: 5,
        base_backoff: std::time::Duration::from_millis(500),
    };
    let archive_retrying = eplyx_engine::ingest::rpc::RetryingRpc {
        provider: archive_transport.as_ref(),
        max_retries: 5,
        base_backoff: std::time::Duration::from_millis(500),
    };
    let block_retrying = eplyx_engine::ingest::rpc::RetryingRpc {
        provider: block_transport.as_ref(),
        max_retries: 5,
        base_backoff: std::time::Duration::from_millis(500),
    };
    let cache = args.output.join("cache");
    let transaction_rpc = eplyx_engine::ingest::CachedRpc {
        provider: &transaction_retrying,
        root: cache
            .join("transactions")
            .join(eplyx_engine::replay::hash_bytes(transaction_url.as_bytes())),
    };
    let archive_rpc = eplyx_engine::ingest::CachedRpc {
        provider: &archive_retrying,
        root: cache
            .join("accounts")
            .join(eplyx_engine::replay::hash_bytes(archive_url.as_bytes())),
    };
    let block_rpc = eplyx_engine::ingest::CachedRpc {
        provider: &block_retrying,
        root: cache
            .join("blocks")
            .join(eplyx_engine::replay::hash_bytes(block_url.as_bytes())),
    };
    let start = std::time::Instant::now();
    // Screening reads a whole block, which is megabytes. Spend it where the
    // guarantee needs it - a CPI contract cannot be acquired without it - or
    // where the caller asked for it explicitly.
    let screen_slot = explicit_block_source
        || args
            .program
            .as_deref()
            .and_then(eplyx_engine::protocol::adapter_for)
            .is_some_and(|adapter| adapter.supports_cpi());
    // With a program named, the adapter seam owns the contract and the V1
    // binary is resolved from history. Without one, the bounded Memo path is
    // used unchanged.
    let acquired = match &args.program {
        Some(program) => eplyx_engine::historical::ProtocolArchiveProvider {
            transaction_rpc: &transaction_rpc,
            account_archive_rpc: &archive_rpc,
            block_rpc: screen_slot.then_some(&block_rpc as &dyn RpcProvider),
            program_id: program,
        }
        .acquire_exact(&args.signature)?,
        None => eplyx_engine::historical::SlotAccountArchiveProvider {
            transaction_rpc: &transaction_rpc,
            account_archive_rpc: &archive_rpc,
        }
        .acquire_exact(&args.signature)?,
    };
    let snapshots = args.output.join("snapshots");
    eplyx_engine::ingest::write_json(
        &snapshots.join(format!("{}.json", args.signature)),
        &acquired.record,
    )?;
    // Durable, append-only: the record lands under its own stable id and the
    // canonical index is rebuilt from what is stored, so acquiring a second
    // observation into the same directory accumulates rather than replaces.
    let store = eplyx_engine::corpus_store::CorpusStore::open(&args.output)?;
    let insert = store.insert(&acquired.record)?;
    let manifest = store.publish()?;
    let artifact_name = match eplyx_engine::protocol::adapter_for(&acquired.record.program_id) {
        Some(adapter) => format!("{}-mainnet-v1.so", adapter.name()),
        None => "memo-mainnet-v1.so".to_string(),
    };
    let v1_path = args.output.join(artifact_name);
    let write_artifact = |path: &std::path::Path, bytes: &[u8]| -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("so.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    };
    write_artifact(&v1_path, &acquired.v1_program)?;
    // Dependency binaries live beside the corpus, addressed by program ID, so
    // the comparison runs with no transport at all once acquisition is done.
    let dependency_dir =
        eplyx_engine::replay::dependency_directory(&args.output.join("corpus.json"));
    for (program_id, bytes) in &acquired.dependency_binaries {
        write_artifact(&dependency_dir.join(format!("{program_id}.so")), bytes)?;
    }
    let value_line = match eplyx_engine::protocol::adapter_for(&acquired.record.program_id) {
        Some(adapter) => format!("Protocol: {}", adapter.name()),
        None => format!(
            "Native transfer: {} lamports",
            acquired.record.native_transfer_lamports().unwrap_or(0)
        ),
    };
    println!(
        // Deliberately not "Exact". Acquisition establishes that exact
        // historical state was obtained, its boundaries proved and its slot
        // screened - it does not run the V1 fidelity gate, which happens at
        // comparison time and yields `Matched` for an archive record. `Exact`
        // stays reserved for the controlled-snapshot contract, where every
        // proof condition is actually held.
        "Historical mainnet replay record: {} at slot {}\nState source: historical_archive \
         (V1 post-state fidelity is checked at comparison time)\n{}\nV1 SHA-256: {}\n\
         Corpus: {}\nV1 artifact: {}\nAcquisition: {} ms{}",
        acquired.record.transaction.signature,
        acquired.record.transaction.slot,
        value_line,
        acquired.record.current_program_sha256,
        args.output.join("corpus.json").display(),
        v1_path.display(),
        start.elapsed().as_millis(),
        if args.offline {
            " (offline cache only)"
        } else {
            ""
        },
    );
    println!(
        "Corpus: {} record(s) ({}), canonical hash {}",
        manifest.record_count,
        match insert {
            eplyx_engine::corpus_store::Insert::Added => "this observation added",
            eplyx_engine::corpus_store::Insert::AlreadyPresent => "already present, unchanged",
        },
        manifest.canonical_hash
    );
    render_dependencies(&acquired.record);
    if let Some(screening) = &acquired.record.slot_screening {
        println!(
            "Same-slot screening: {} required account(s) against {} transactions in slot {}; \
             no conflicts (target at index {})",
            screening.required_accounts.len(),
            screening.transactions_in_slot,
            screening.slot,
            screening.target_index
        );
    }
    if !acquired
        .record
        .transaction
        .inner_instruction_frames
        .is_empty()
    {
        print!(
            "\nCPI graph recorded from validator metadata:\n{}",
            eplyx_engine::replay::render_cpi_graph(
                &acquired.record.program_id,
                &acquired.record.transaction.inner_instruction_frames
            )
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Print the dependency manifest: which binaries executed, and where each came
/// from. A replay that cannot say this is not evidence of anything.
fn render_dependencies(record: &eplyx_engine::replay::ReplayRecord) {
    if record.dependencies.programs.is_empty() {
        return;
    }
    println!("\nProgram dependencies:");
    for program in &record.dependencies.programs {
        println!(
            "  {:44} {:18} {}",
            program.program_id,
            program.source.as_str(),
            match (&program.binary_sha256, program.deployed_slot) {
                (Some(hash), Some(slot)) => format!("deployed at slot {slot}, sha256 {hash}"),
                (Some(hash), None) => format!("sha256 {hash}"),
                _ => program
                    .note
                    .clone()
                    .unwrap_or_else(|| "provided by the runtime".into()),
            }
        );
        println!(
            "  {:44} discovered by: {}",
            "",
            program
                .discovered_by
                .iter()
                .map(|how| how.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}
fn ingest_command(args: IngestArgs) -> Result<ExitCode> {
    let start = std::time::Instant::now();
    let url = endpoint(args.rpc_url)?;
    // Endpoint namespace prevents cache reuse across networks without storing
    // credentials. Changing credentials creates a new transport cache.
    let namespace = eplyx_engine::replay::hash_bytes(url.as_bytes());
    let rpc = eplyx_engine::ingest::rpc::HttpRpc::new(url)?;
    let cached = eplyx_engine::ingest::CachedRpc {
        provider: &rpc,
        root: args.cache.join(namespace),
    };
    let manifest = eplyx_engine::ingest::ingest(
        &cached,
        &args.cache,
        &args.program,
        args.start_slot,
        args.end_slot,
    )?;
    println!(
        "Ingested {} transactions in {} ms; slots {}..{}; current account samples are APPROXIMATE",
        manifest.transactions.len(),
        start.elapsed().as_millis(),
        args.start_slot,
        args.end_slot
    );
    Ok(ExitCode::SUCCESS)
}

fn discover_command(args: DiscoverArgs) -> Result<ExitCode> {
    anyhow::ensure!(args.limit > 0, "--limit must be positive");
    anyhow::ensure!(args.concurrency > 0, "--concurrency must be positive");
    let overall = std::time::Instant::now();
    let url = endpoint(args.rpc_url)?;
    let namespace = eplyx_engine::replay::hash_bytes(url.as_bytes());
    let mut rpc = eplyx_engine::ingest::rpc::HttpRpc::new(url)?;
    if let Some(origin) = args
        .rpc_origin
        .clone()
        .or_else(|| std::env::var("SOLANA_RPC_ORIGIN").ok())
    {
        rpc = rpc.with_origin(origin)?;
    }
    let retrying = eplyx_engine::ingest::rpc::RetryingRpc {
        provider: &rpc,
        max_retries: args.retries,
        base_backoff: std::time::Duration::from_millis(args.backoff_ms),
    };
    let cache_root = args.output.join("cache");
    let cache_metrics = eplyx_engine::ingest::CacheMetrics::default();
    let cached = eplyx_engine::ingest::MeasuredCachedRpc {
        provider: &retrying,
        root: cache_root.join(&namespace),
        metrics: &cache_metrics,
    };
    use eplyx_engine::ingest::rpc::RpcProvider;
    let end_slot = match args.end_slot {
        Some(slot) => slot,
        None => cached
            .call("getSlot", serde_json::json!([{"commitment":"confirmed"}]))?
            .as_u64()
            .context("getSlot returned no slot")?,
    };
    let start_slot = args
        .start_slot
        .unwrap_or_else(|| end_slot.saturating_sub(5_000));
    anyhow::ensure!(
        start_slot <= end_slot,
        "start slot must not exceed end slot"
    );

    let ingest_start = std::time::Instant::now();
    let manifest = eplyx_engine::ingest::ingest_bounded_with_concurrency(
        &cached,
        &cache_root,
        &args.program,
        start_slot,
        end_slot,
        Some(args.limit),
        usize::try_from(args.concurrency).context("--concurrency is too large")?,
    )?;
    let ingest_ms = ingest_start.elapsed().as_millis();
    anyhow::ensure!(
        !manifest.transactions.is_empty(),
        "no target-program interactions found in the requested window"
    );
    let selection_start = std::time::Instant::now();
    let policy = eplyx_engine::discovery::SelectionPolicy {
        max_records: args.corpus_size,
        ..Default::default()
    };
    let corpus = eplyx_engine::discovery::build_from_cache(
        &cache_root,
        policy,
        Some(namespace),
        args.concurrency,
        args.retries as u64,
    )?;
    let selection_ms = selection_start.elapsed().as_millis();
    eplyx_engine::ingest::write_json(&args.output.join("discovery-corpus.json"), &corpus)?;
    let report = eplyx_engine::discovery::render_text(&corpus);
    std::fs::write(args.output.join("discovery-report.txt"), &report)?;
    print!("{report}");
    let normalized_per_second =
        (manifest.transactions.len() as u128).saturating_mul(1000) / ingest_ms.max(1);
    eprintln!(
        "Performance: discovery/RPC {ingest_ms} ms; normalize+cluster+rank+select {selection_ms} ms; total {} ms; {} normalized interactions/s; cache {} hits / {} misses (transport request groups)",
        overall.elapsed().as_millis(),
        normalized_per_second,
        cache_metrics.hits(),
        cache_metrics.misses(),
    );
    Ok(ExitCode::SUCCESS)
}

fn rpc_inspect(
    rpc_url: Option<String>,
    program: Option<String>,
    format: Format,
) -> Result<ExitCode> {
    let rpc = eplyx_engine::ingest::rpc::HttpRpc::new(endpoint(rpc_url)?)?;
    use eplyx_engine::ingest::rpc::RpcProvider;
    let genesis_hash = rpc.call("getGenesisHash", serde_json::json!([]))?;
    let first_available_block = rpc.call("getFirstAvailableBlock", serde_json::json!([]))?;
    let version = rpc.call("getVersion", serde_json::json!([]))?;
    let mut old_signatures = "not_probed";
    let mut historical_transactions = "not_probed";
    if let Some(program) = program {
        program.parse::<solana_address::Address>()?;
        let page = rpc.call(
            "getSignaturesForAddress",
            serde_json::json!([program,{"limit":1,"commitment":"confirmed"}]),
        )?;
        old_signatures = if page.as_array().is_some() {
            "yes"
        } else {
            "unknown"
        };
        if let Some(signature) = page
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item["signature"].as_str())
        {
            let tx = rpc.call("getTransaction",serde_json::json!([signature,{"encoding":"json","commitment":"confirmed","maxSupportedTransactionVersion":0}]))?;
            historical_transactions = if tx.is_null() {
                "sample_unavailable"
            } else {
                "sample_available"
            };
        }
    }
    let inspection = serde_json::json!({
        "genesis_hash": genesis_hash,
        "first_available_block": first_available_block,
        "node_version": version,
        "signature_history": old_signatures,
        "sample_transaction_metadata": historical_transactions,
        "arbitrary_historical_account_snapshots": "not_available_via_standard_solana_rpc",
        "note": "Transaction archives and arbitrary historical account bytes are separate capabilities."
    });
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&inspection)?),
        Format::Text => println!(
            "EPLYX RPC INSPECTION\nGenesis hash: {}\nFirst available block: {}\nSignature history: {}\nSample transaction metadata: {}\nArbitrary historical account snapshots: not available via standard Solana RPC\n\nTransaction archives and arbitrary historical account bytes are separate capabilities.",
            inspection["genesis_hash"], first_available_block, old_signatures, historical_transactions
        ),
    }
    Ok(ExitCode::SUCCESS)
}

fn parse_category(name: &str) -> Result<Category> {
    Category::ALL
        .into_iter()
        .find(|c| c.as_str() == name)
        .ok_or_else(|| {
            anyhow!(
                "unknown category {name:?}; valid categories: {}",
                Category::ALL
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
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
    let cli = Cli::parse();
    match cli.command {
        Command::Compare(args) => compare(args),
        Command::Ingest(args) => ingest_command(args),
        Command::Discover(args) => discover_command(args),
        Command::Historical { command } => match command {
            HistoricalCommand::Acquire(args) => historical_acquire(args),
        },
        Command::Versions { command } => match command {
            VersionsCommand::Resolve(args) => versions_resolve(args),
            VersionsCommand::Upgrades(args) => versions_upgrades(args),
        },
        Command::Rpc { command } => match command {
            RpcCommand::Inspect {
                rpc_url,
                program,
                format,
            } => rpc_inspect(rpc_url, program, format),
        },
        Command::Discovery { command } => match command {
            DiscoveryCommand::Build {
                cache,
                snapshots,
                out,
                corpus_size,
            } => {
                let start = std::time::Instant::now();
                let policy = eplyx_engine::discovery::SelectionPolicy {
                    max_records: corpus_size,
                    ..Default::default()
                };
                let corpus = match snapshots {
                    Some(snapshots) => eplyx_engine::discovery::build_from_cache_and_snapshots(
                        &cache, &snapshots, policy, None, 1, 0,
                    )?,
                    None => eplyx_engine::discovery::build_from_cache(&cache, policy, None, 1, 0)?,
                };
                eplyx_engine::ingest::write_json(&out, &corpus)?;
                println!("{}", eplyx_engine::discovery::render_text(&corpus));
                eprintln!(
                    "Offline discovery selection: {} interactions in {} ms",
                    corpus.statistics.transactions_normalized,
                    start.elapsed().as_millis()
                );
                Ok(ExitCode::SUCCESS)
            }
        },
        Command::Corpus {
            command: CorpusCommand::Select(select_args),
        } => corpus_select(select_args),
        Command::Ci {
            command: CiCommand::Check(check_args),
        } => ci_check(check_args),
        Command::Bundle {
            command: BundleCommand::Build(build_args),
        } => bundle_build(build_args),
        Command::Bundle {
            command: BundleCommand::Verify(verify_args),
        } => bundle_verify(verify_args),
        Command::Corpus {
            command:
                CorpusCommand::Build {
                    cache,
                    snapshots,
                    out,
                    limit,
                },
        } => {
            let start = std::time::Instant::now();
            let count = eplyx_engine::ingest::build_corpus(&cache, &snapshots, &out, limit)?;
            println!(
                "Built {count} replay records in {} ms: {}",
                start.elapsed().as_millis(),
                out.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Controlled { command } => {
            match command {
                ControlledCommand::Prepare { dir } => {
                    eplyx_engine::ingest::controlled::prepare(&dir)?
                }
                ControlledCommand::Capture {
                    dir,
                    snapshots,
                    rpc_url,
                    current,
                } => {
                    anyhow::ensure!(
                        rpc_url.starts_with("http://127.0.0.1:")
                            || rpc_url.starts_with("http://localhost:"),
                        "controlled capture is local-validator only"
                    );
                    let rpc = eplyx_engine::ingest::rpc::HttpRpc::new(rpc_url)?;
                    let (start, end) = eplyx_engine::ingest::controlled::capture(
                        &rpc, &dir, &snapshots, &current,
                    )?;
                    eplyx_engine::ingest::write_json(
                        &dir.join("window.json"),
                        &serde_json::json!({"start_slot":start,"end_slot":end}),
                    )?;
                    println!("Captured three controlled interactions; slots {start}..{end}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Generate(args) => generate(args),
        Command::Reproduce(args) => reproduce(args),
        Command::List(args) => list(args),
    }
}

fn compare(args: CompareArgs) -> Result<ExitCode> {
    if let Some(path) = &args.corpus {
        anyhow::ensure!(
            args.fixture.is_none() && args.category.is_none(),
            "select replay records with corpus build --limit"
        );
        let records = eplyx_engine::replay::load_corpus(path)?;
        let (v1, v2) = load_versions(
            &args.v1.clone().unwrap_or_else(|| default_artifact("v1")),
            &args.v2.clone().unwrap_or_else(|| default_artifact("v2")),
        )?;
        let dependency_dir = args
            .dependencies
            .clone()
            .unwrap_or_else(|| eplyx_engine::replay::dependency_directory(path));
        let load_start = std::time::Instant::now();
        let dependencies = eplyx_engine::replay::load_dependencies(&records, &dependency_dir)?;
        let dependency_load = load_start.elapsed().as_micros();
        let start = std::time::Instant::now();
        let report =
            eplyx_engine::replay::compare_with_dependencies(&records, &v1, &v2, &dependencies)?;
        let elapsed = start.elapsed().as_micros();
        let rendered = match args.format {
            Format::Json => serde_json::to_string_pretty(&report)?,
            Format::Text => {
                let mut text=String::from("OFFLINE TRUSTED REPLAY\nCapital represents interaction observations, not unique TVL. Native transfers are reported separately without fiat estimates.\n");
                for item in &report.observations {
                    text.push_str(&format!("{} | slot {} | {:?} | fidelity {:?}\n  source {}\n  pre {}\n  V1  {} (matches original)\n  V2  {}\n",item.id,item.source_slot,item.state_source,item.fidelity,item.source_signature,item.pre_state_hash,item.post_v1_state_hash,item.post_v2_state_hash));
                }
                if let Some(native) = &report.native_impact {
                    text.push_str(&format!("Native transfer represented: {} lamports\nCandidate prevented:          {} lamports\n",native.v1_transferred_lamports,native.candidate_prevented_lamports));
                }
                for item in &report.observations {
                    if item.dependency_programs.is_empty() {
                        continue;
                    }
                    text.push_str(&format!("\nPROGRAM DEPENDENCIES ({})\n", item.id));
                    for program in &item.dependency_programs {
                        text.push_str(&format!(
                            "  {:44} {:18} {}\n",
                            program.program_id,
                            program.source.as_str(),
                            match (&program.binary_sha256, program.deployed_slot) {
                                (Some(hash), Some(slot)) =>
                                    format!("deployed at slot {slot}, sha256 {hash}"),
                                (Some(hash), None) => format!("sha256 {hash}"),
                                _ => "provided by the runtime".into(),
                            }
                        ));
                    }
                }
                for item in &report.observations {
                    if item.original_cpi_graph.is_empty() && item.cpi_graph_v1.is_empty() {
                        continue;
                    }
                    text.push_str(&format!("\nCPI GRAPH ({})\n", item.id));
                    text.push_str("  mainnet, from validator metadata:\n");
                    for line in eplyx_engine::replay::render_cpi_graph(
                        &report.analysis.program_id,
                        &item.original_cpi_graph,
                    )
                    .lines()
                    {
                        text.push_str(&format!("    {line}\n"));
                    }
                    text.push_str(&format!(
                        "  V1 replay reproduced it: {}\n  V2 invocation graph: {}\n",
                        if item.cpi_graph_v1 == item.original_cpi_graph {
                            "yes"
                        } else {
                            "no"
                        },
                        if item.cpi_graph_changed {
                            "differs from V1; a changed invocation graph is reported, \
                             not scored"
                        } else {
                            "identical to V1"
                        }
                    ));
                }
                let summarized: Vec<_> = report
                    .observations
                    .iter()
                    .filter(|item| !item.economic_summary.is_empty())
                    .collect();
                if !summarized.is_empty() {
                    text.push_str("\nPROTOCOL RESULT\n");
                    for item in summarized {
                        text.push_str(&format!("  {}\n", item.id));
                        for row in &item.economic_summary {
                            text.push_str(&format!(
                                "    {:26} V1 {:>22}   V2 {:>22}{}\n",
                                row.field,
                                row.v1,
                                row.v2,
                                row.delta
                                    .filter(|delta| !delta.is_zero())
                                    .map(|delta| format!("   delta {delta}"))
                                    .unwrap_or_default()
                            ));
                        }
                    }
                }
                let economic: Vec<_> = report
                    .observations
                    .iter()
                    .flat_map(|item| {
                        item.economic_changes
                            .iter()
                            .map(move |change| (item.id.as_str(), change))
                    })
                    .collect();
                text.push_str("\nPROTOCOL ECONOMICS\n");
                if economic.is_empty() {
                    text.push_str("  No protocol-level field differs between the two builds.\n");
                } else {
                    text.push_str(&format!(
                        "  {} of {} observation(s) changed economically. Whether a change is \
                         intended is not classified here.\n",
                        report.economic_findings,
                        report.observations.len()
                    ));
                    for (id, change) in economic {
                        text.push_str(&format!(
                            "  {id}\n    {} ({}) {}\n      V1 {}\n      V2 {}{}\n",
                            change.account_label,
                            change.account_kind,
                            change.field,
                            change.v1,
                            change.v2,
                            change
                                .delta
                                .map(|delta| format!("\n      delta {delta}"))
                                .unwrap_or_default(),
                        ));
                    }
                }
                let analysis = render_text(&report.analysis).replace(
                    "synthetic corpus, valued from fixture state",
                    "replay observations, valued from captured state",
                );
                // The fixture protocol's position/USD aggregation is meaningless
                // for an adapter-owned record: there are no positions to value,
                // and printing zeroed collateral would read as a measurement.
                text.push_str(
                    &if records.iter().any(|record| record.adapter().is_some()) {
                        analysis
                        .split("ECONOMIC COVERAGE")
                        .next()
                        .unwrap_or(&analysis)
                        .to_string()
                        + "(Position and USD aggregation belongs to the fixture lending protocol \
                           and is omitted: this record's economics are protocol token amounts, \
                           reported above.)\n\n"
                        + analysis
                            .split_once("VERDICT:")
                            .map(|(_, rest)| format!("VERDICT:{rest}"))
                            .unwrap_or_default()
                            .as_str()
                    } else {
                        analysis
                    },
                );
                text
            }
        };
        if let Some(out) = &args.out {
            std::fs::write(out, rendered)?;
        } else {
            println!("{rendered}");
        }
        let total_micros = |select: fn(&eplyx_engine::replay::ReplayTiming) -> u128| {
            report.timings.iter().map(select).sum::<u128>()
        };
        eprintln!(
            "Replay performance: dependency load {dependency_load} us for {} binary(ies); \
             V1 replay {} us; V2 replay {} us; comparison total {elapsed} us; \
             {} VM executions; {} accounts loaded; {} program dependencies \
             ({} loaded from history, {} runtime-provided)",
            dependencies.programs().len(),
            total_micros(|timing| timing.v1_micros),
            total_micros(|timing| timing.v2_micros),
            records.len() * 2,
            records
                .iter()
                .map(|record| record.accounts.len())
                .sum::<usize>(),
            records
                .iter()
                .map(|record| record.dependencies.programs.len())
                .sum::<usize>(),
            records
                .iter()
                .flat_map(|record| record.dependencies.loadable())
                .count(),
            records
                .iter()
                .flat_map(|record| record.dependencies.programs.iter())
                .filter(
                    |program| program.source == eplyx_engine::dependencies::ProgramSource::Builtin
                )
                .count(),
        );
        // A protocol-level economic change is a gate failure even when the
        // protocol-agnostic classifier only saw bytes move.
        return Ok(
            if args.fail_on_critical
                && (report.analysis.summary.critical > 0 || report.economic_findings > 0)
            {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            },
        );
    }
    let program_id = fixture_program_id();
    let mut fixtures = corpus::generate(&program_id);

    if let Some(id) = &args.fixture {
        fixtures.retain(|f| &f.id == id);
        if fixtures.is_empty() {
            return Err(anyhow!("no fixture with id {id:?}"));
        }
    }
    if let Some(name) = &args.category {
        let category = parse_category(name)?;
        fixtures.retain(|f| f.category == category);
    }

    let v1_path = args.v1.unwrap_or_else(|| default_artifact("v1"));
    let v2_path = args.v2.unwrap_or_else(|| default_artifact("v2"));
    let (v1, v2) = load_versions(&v1_path, &v2_path)?;

    let diffs = compare_all(&fixtures, &program_id, &v1, &v2)?;
    let mut report = Report::new(
        program_id.to_string(),
        v1_path.display().to_string(),
        v2_path.display().to_string(),
        &fixtures,
        diffs,
    );
    if !args.no_minimize {
        eplyx_engine::minimize_clusters(
            &mut report,
            &fixtures,
            &program_id,
            &v1,
            &v2,
            eplyx_engine::shrink::ShrinkConfig::default(),
        )?;
    }

    let rendered = match args.format {
        Format::Text => render_text(&report),
        Format::Json => report.to_json()?,
    };

    match &args.out {
        Some(path) => {
            std::fs::write(path, &rendered)
                .with_context(|| format!("writing report to {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => println!("{rendered}"),
    }

    if args.fail_on_critical && report.summary.critical > 0 {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn generate(args: GenerateArgs) -> Result<ExitCode> {
    let program_id = fixture_program_id();
    let fixtures = corpus::generate(&program_id);
    let out = args
        .out
        .unwrap_or_else(|| eplyx_engine::repo_root().join("fixtures/states"));
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;

    for fixture in &fixtures {
        let path = out.join(format!("{}.json", fixture.id));
        std::fs::write(&path, serde_json::to_string_pretty(fixture)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }

    let index: Vec<_> = fixtures
        .iter()
        .map(|f| {
            serde_json::json!({
                "id": f.id,
                "category": f.category.as_str(),
                "scenario": f.scenario,
            })
        })
        .collect();
    let index_path = out.join("index.json");
    std::fs::write(&index_path, serde_json::to_string_pretty(&index)? + "\n")?;

    println!("wrote {} fixtures to {}", fixtures.len(), out.display());
    Ok(ExitCode::SUCCESS)
}

fn reproduce(args: ReproduceArgs) -> Result<ExitCode> {
    let program_id = fixture_program_id();
    let fixtures = corpus::generate(&program_id);
    let v1_path = args.v1.unwrap_or_else(|| default_artifact("v1"));
    let v2_path = args.v2.unwrap_or_else(|| default_artifact("v2"));
    let (v1, v2) = load_versions(&v1_path, &v2_path)?;

    let target = args
        .target
        .strip_prefix("regression-group:")
        .unwrap_or(&args.target)
        .to_string();

    // A single fixture needs two executions; a regression group needs the whole
    // corpus, so try the cheap resolution first.
    if let Some(fixture) = fixtures.iter().find(|f| f.id == target) {
        let diff = eplyx_engine::compare_fixture(fixture, &program_id, &v1, &v2)?;
        println!("{}", eplyx_engine::report::render_reproduction(&diff));
        return Ok(ExitCode::SUCCESS);
    }

    let diffs = compare_all(&fixtures, &program_id, &v1, &v2)?;
    let mut report = Report::new(
        program_id.to_string(),
        v1_path.display().to_string(),
        v2_path.display().to_string(),
        &fixtures,
        diffs,
    );
    if !args.no_minimize {
        eplyx_engine::minimize_clusters(
            &mut report,
            &fixtures,
            &program_id,
            &v1,
            &v2,
            eplyx_engine::shrink::ShrinkConfig::default(),
        )?;
    }

    match report.cluster(&target) {
        Some(cluster) => {
            println!(
                "{}",
                eplyx_engine::report::render_cluster_reproduction(&report, cluster)
            );
            Ok(ExitCode::SUCCESS)
        }
        None => Err(anyhow!(
            "no fixture or regression group {target:?}\n\navailable regression groups:\n{}",
            report
                .clusters
                .iter()
                .map(|c| format!(
                    "  {:<34} {} fixtures{}",
                    c.id,
                    c.fixture_count(),
                    if c.critical { "  [CRITICAL]" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join("\n")
        )),
    }
}

fn list(args: ListArgs) -> Result<ExitCode> {
    let program_id = fixture_program_id();
    let mut fixtures = corpus::generate(&program_id);
    if let Some(name) = &args.category {
        let category = parse_category(name)?;
        fixtures.retain(|f| f.category == category);
    }
    for fixture in &fixtures {
        println!(
            "{:<28} {:<22} {}",
            fixture.id,
            fixture.category.as_str(),
            fixture.scenario
        );
    }
    println!("\n{} fixtures", fixtures.len());
    Ok(ExitCode::SUCCESS)
}

/// The CI gate: a pinned bundle, a candidate, and a declaration file.
///
/// Returns an exit code rather than an error for a completed review, so a
/// failing gate is distinguishable from an analysis that could not run.
fn ci_check(args: CiCheckArgs) -> Result<ExitCode> {
    use eplyx_engine::ci;

    let report = match ci::check(&args.bundle, &args.candidate, args.expectations.as_deref()) {
        Ok(report) => report,
        Err(error) => {
            // A preflight abort produces no analysis, but it must still answer
            // in the format that was asked for. A consumer parsing JSON should
            // not have to scrape stderr to learn why a run produced nothing.
            let code = error.exit_code();
            match args.format {
                Format::Json => {
                    let body = serde_json::json!({
                        "schema_version": eplyx_engine::ci::CI_REPORT_SCHEMA,
                        "status": "error",
                        "exit_code": code,
                        "error": format!("{error}"),
                    });
                    let rendered = serde_json::to_string_pretty(&body)?;
                    match &args.out {
                        Some(path) => {
                            std::fs::write(path, format!("{rendered}\n"))?;
                            eprintln!("wrote {}", path.display());
                        }
                        None => println!("{rendered}"),
                    }
                }
                Format::Text => {
                    eprintln!("EPLYX UPGRADE CHECK\n\nAnalysis could not run.\n\n{error}");
                }
            }
            return Ok(ExitCode::from(code));
        }
    };

    let rendered = match args.format {
        Format::Json => serde_json::to_string_pretty(&report)?,
        Format::Text => render_ci(&report),
    };
    match &args.out {
        Some(path) => {
            std::fs::write(path, format!("{rendered}\n"))?;
            println!("wrote {}", path.display());
        }
        None => println!("{rendered}"),
    }
    Ok(ExitCode::from(report.exit_code()))
}

fn render_ci(report: &eplyx_engine::ci::CiReport) -> String {
    use eplyx_engine::review::ReviewStatus;
    use std::fmt::Write as _;

    let mut text = String::from("EPLYX UPGRADE CHECK\n===================\n\n");
    let _ = writeln!(text, "Baseline:   {}", report.bundle.baseline_sha256);
    let _ = writeln!(text, "Candidate:  {}", report.candidate.sha256);
    let _ = writeln!(
        text,
        "Bundle:     {} ({} @{})",
        report.bundle.sha256, report.bundle.adapter, report.bundle.adapter_version
    );
    let _ = writeln!(
        text,
        "\nCorpus:     {} validated historical observations",
        report.bundle.record_count
    );

    let _ = writeln!(text, "\nCOVERAGE");
    for subject in &report.coverage {
        let _ = writeln!(
            text,
            "  {:<58} {:>3} observations",
            subject.subject, subject.observations
        );
    }

    let _ = writeln!(text, "\nRESULTS");
    let _ = writeln!(
        text,
        "  expected                     {:>3}",
        report.summary.expected
    );
    let _ = writeln!(
        text,
        "  unexpected                   {:>3}",
        report.summary.unexpected
    );
    let _ = writeln!(
        text,
        "  expected but exceeded        {:>3}",
        report.summary.expected_but_exceeded
    );
    let _ = writeln!(
        text,
        "  stale declarations           {:>3}",
        report.summary.stale
    );
    let _ = writeln!(
        text,
        "  unevaluable declarations     {:>3}",
        report.summary.unevaluable
    );

    if !report.findings.is_empty() {
        let _ = writeln!(text, "\nFINDINGS");
        for finding in &report.findings {
            // Severity and review status are two separate statements, and the
            // report keeps them side by side rather than folding one into the
            // other.
            let _ = writeln!(
                text,
                "  {} / {}",
                finding.severity.as_str(),
                finding.status.as_str().to_uppercase()
            );
            let _ = writeln!(text, "    {}", finding.fingerprint);
            let _ = writeln!(
                text,
                "    affected {} of {} observations that can measure it, {} entities",
                finding.observations.len(),
                finding.covered_observations,
                finding.entities.len()
            );
            if let Some(bps) = finding.max_relative_delta_bps {
                let _ = writeln!(text, "    largest change {bps} bps");
            }
            if let Some(reason) = &finding.reason {
                let _ = writeln!(text, "    declared: {reason}");
            }
            for breach in &finding.breaches {
                let _ = writeln!(text, "    exceeds: {breach:?}");
            }
            if let Some(cause) = &finding.unevaluable {
                let _ = writeln!(text, "    cannot judge: {cause:?}");
            }
        }
    }

    if !report.unmatched.is_empty() {
        let _ = writeln!(text, "\nDECLARATIONS THAT MATCHED NOTHING");
        for entry in &report.unmatched {
            let _ = writeln!(text, "  {}", entry.status.as_str().to_uppercase());
            let _ = writeln!(text, "    {}", entry.fingerprint);
            let _ = writeln!(text, "    declared: {}", entry.reason);
            let _ = writeln!(
                text,
                "    {} observations in this corpus can measure it",
                entry.covered_observations
            );
            if entry.status == ReviewStatus::Unevaluable {
                let _ = writeln!(
                    text,
                    "    Eplyx cannot prove whether this declaration still applies."
                );
            }
        }
    }

    let _ = writeln!(
        text,
        "\nGATE: {}",
        if report.summary.passed {
            "PASSED".to_string()
        } else {
            format!(
                "FAILED ({})",
                report
                    .summary
                    .failure_reasons
                    .iter()
                    .map(|reason| reason.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    let _ = write!(text, "exit {}", report.summary.exit_code);
    text
}

/// Assemble an offline-executable CI bundle.
fn bundle_build(args: BundleBuildArgs) -> Result<ExitCode> {
    use eplyx_engine::bundle;

    let store = eplyx_engine::corpus_store::CorpusStore::open(&args.corpus)?;
    let all = store.load()?;
    let dependencies = args
        .dependencies
        .clone()
        .unwrap_or_else(|| args.corpus.join("dependencies"));

    // Selecting first keeps the bundle small enough to run on every pull
    // request; the policy and its stated limitations travel with it, so the
    // gate can report what the corpus does not cover.
    let (records, policy, policy_version, limitations) = match args.target_size {
        Some(target) => {
            let observed: eplyx_engine::select::ObservedCounts = match &args.observed {
                Some(text) => {
                    serde_json::from_str(text).context("--observed must be a JSON object")?
                }
                None => Default::default(),
            };
            let selected = eplyx_engine::select::select(&all, target, &observed)?;
            let keep: std::collections::BTreeSet<&str> =
                selected.selected.iter().map(|s| s.id.as_str()).collect();
            let records: Vec<_> = all
                .iter()
                .filter(|r| keep.contains(r.id.as_str()))
                .cloned()
                .collect();
            let limitations = selected
                .limitations
                .iter()
                .map(|l| bundle::BundledLimitation {
                    code: l.code.clone(),
                    detail: l.detail.clone(),
                })
                .collect();
            (
                records,
                Some(selected.selection_policy),
                Some(selected.selection_policy_version),
                limitations,
            )
        }
        None => (all, None, None, Vec::new()),
    };

    let built = bundle::build(
        bundle::BundleInputs {
            records: &records,
            baseline: &args.baseline,
            dependencies: &dependencies,
            selection_policy: policy,
            selection_policy_version: policy_version,
            limitations,
            // A corpus is acquired until something reproduces it. This is the
            // first step holding the baseline and every dependency, so it is
            // the first step that can make "validated" true.
            validation: bundle::Validation::AgainstBaseline,
        },
        &args.out,
    )?;
    print_bundle(&built);
    println!("\nwrote {}", args.out.display());
    Ok(ExitCode::SUCCESS)
}

/// Re-hash every byte a bundle pins.
fn bundle_verify(args: BundleVerifyArgs) -> Result<ExitCode> {
    let bundle = eplyx_engine::bundle::CiBundle::open(&args.bundle)?;
    match args.format {
        Format::Json => println!("{}", serde_json::to_string_pretty(bundle.manifest())?),
        Format::Text => {
            print_bundle(&bundle);
            println!("\nEvery hash verified against the bytes on disk.");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_bundle(bundle: &eplyx_engine::bundle::CiBundle) {
    let manifest = bundle.manifest();
    let adapter = bundle.adapter();
    println!("EPLYX CI BUNDLE");
    println!("===============\n");
    println!("program:           {}", manifest.program_id);
    println!(
        "adapter:           {}@{}{}",
        adapter.name,
        adapter.version,
        if adapter.supports_cpi {
            ", cross-program invocation"
        } else {
            ""
        }
    );
    if let (Some(policy), Some(version)) = (
        &manifest.selection_policy,
        manifest.selection_policy_version,
    ) {
        println!("selection policy:  {policy} v{version}");
    }
    println!("\nbaseline sha256:   {}", manifest.baseline_program_sha256);
    println!("corpus sha256:     {}", manifest.corpus_sha256);
    println!("bundle sha256:     {}", manifest.bundle_sha256);
    println!(
        "\nrecords:           {} validated historical observations",
        manifest.record_count
    );
    println!(
        "production window: slots {} -> {}",
        manifest.source_slot_range.first, manifest.source_slot_range.last
    );
    println!("\nCOVERAGE");
    for action in &adapter.actions {
        println!(
            "  {:<16} {:>3} tested",
            action.semantic_action, action.observations
        );
    }
    if !manifest.dependencies.is_empty() {
        println!("\nPINNED DEPENDENCIES");
        for dependency in &manifest.dependencies {
            println!(
                "  {}  {}  {} bytes",
                dependency.program_id,
                &dependency.sha256[..16],
                dependency.len
            );
        }
    }
    if !adapter.limitations.is_empty() {
        println!("\nKNOWN LIMITATIONS");
        for limitation in &adapter.limitations {
            println!("  - {}", limitation.code);
            println!("    {}", limitation.detail);
        }
    }
}

/// Select a deterministic regression corpus from validated replay records.
fn corpus_select(args: CorpusSelectArgs) -> Result<ExitCode> {
    let store = eplyx_engine::corpus_store::CorpusStore::open(&args.corpus)?;
    let records = store.load()?;
    anyhow::ensure!(
        !records.is_empty(),
        "no validated records in {}",
        args.corpus.display()
    );
    let observed: eplyx_engine::select::ObservedCounts = match &args.observed {
        Some(text) => serde_json::from_str(text).context("--observed must be a JSON object")?,
        None => Default::default(),
    };
    let corpus = eplyx_engine::select::select(&records, args.target_size, &observed)?;
    let rendered = match args.format {
        Format::Text => eplyx_engine::select::render(&corpus),
        Format::Json => serde_json::to_string_pretty(&corpus)?,
    };
    match &args.out {
        Some(path) => {
            std::fs::write(path, &rendered)
                .with_context(|| format!("writing {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => println!("{rendered}"),
    }
    Ok(ExitCode::SUCCESS)
}
