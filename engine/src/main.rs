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
    /// Select captured transactions into a durable offline replay corpus.
    Corpus {
        #[command(subcommand)]
        command: CorpusCommand,
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
        let start = std::time::Instant::now();
        let report = eplyx_engine::replay::compare(&records, &v1, &v2)?;
        let elapsed = start.elapsed().as_micros();
        let rendered = match args.format {
            Format::Json => serde_json::to_string_pretty(&report)?,
            Format::Text => {
                let mut text=String::from("OFFLINE CONTROLLED REPLAY\nCapital represents interaction observations, not unique TVL.\n");
                for item in &report.observations {
                    text.push_str(&format!("{} | slot {} | {:?} | fidelity {:?}\n  source {}\n  pre {}\n  V1  {} (matches original)\n  V2  {}\n",item.id,item.source_slot,item.state_source,item.fidelity,item.source_signature,item.pre_state_hash,item.post_v1_state_hash,item.post_v2_state_hash));
                }
                text.push_str(&render_text(&report.analysis).replace(
                    "synthetic corpus, valued from fixture state",
                    "replay observations, valued from captured state",
                ));
                text
            }
        };
        if let Some(out) = &args.out {
            std::fs::write(out, rendered)?;
        } else {
            println!("{rendered}");
        }
        eprintln!(
            "Replay performance: total {elapsed} us; mean V1/V2 pair {} us; {} VM executions",
            elapsed / records.len() as u128,
            records.len() * 2
        );
        return Ok(
            if args.fail_on_critical && report.analysis.summary.critical > 0 {
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
