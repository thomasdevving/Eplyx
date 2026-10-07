//! Historical mainnet acquisition, version resolution, RPC ingestion and discovery commands.
use super::{
    ArchiveArgs, ControlledCommand, DiscoverArgs, Format, HistoricalAcquireArgs, IngestArgs,
    VersionsResolveArgs, VersionsUpgradesArgs,
};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::ExitCode;

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

pub(crate) fn versions_resolve(args: VersionsResolveArgs) -> Result<ExitCode> {
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

pub(crate) fn versions_upgrades(args: VersionsUpgradesArgs) -> Result<ExitCode> {
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

pub(crate) fn historical_acquire(args: HistoricalAcquireArgs) -> Result<ExitCode> {
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
pub(crate) fn ingest_command(args: IngestArgs) -> Result<ExitCode> {
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

pub(crate) fn discover_command(args: DiscoverArgs) -> Result<ExitCode> {
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
    // Counted, not hidden. A window is not free of what the endpoint refused to
    // show us, and a silent skip here would be an unmeasured population
    // reported as nothing.
    if !manifest.unreadable_transactions.is_empty() {
        eprintln!(
            "note: {} transaction(s) in this window are newer than this build \
             reads and were observed but not normalized",
            manifest.unreadable_transactions.len()
        );
        for unreadable in manifest.unreadable_transactions.iter().take(3) {
            eprintln!("  {} at slot {}", unreadable.signature, unreadable.slot);
        }
    }
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

pub(crate) fn rpc_inspect(
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

/// Select a discovery corpus from an existing ingestion cache.
pub(crate) fn discovery_build(
    cache: PathBuf,
    snapshots: Option<PathBuf>,
    out: PathBuf,
    corpus_size: u64,
) -> Result<ExitCode> {
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

pub(crate) fn controlled(command: ControlledCommand) -> Result<ExitCode> {
    match command {
        ControlledCommand::Prepare { dir } => eplyx_engine::ingest::controlled::prepare(&dir)?,
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
            let (start, end) =
                eplyx_engine::ingest::controlled::capture(&rpc, &dir, &snapshots, &current)?;
            eplyx_engine::ingest::write_json(
                &dir.join("window.json"),
                &serde_json::json!({"start_slot":start,"end_slot":end}),
            )?;
            println!("Captured three controlled interactions; slots {start}..{end}");
        }
    }
    Ok(ExitCode::SUCCESS)
}
