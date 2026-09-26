//! Local project commands. Capture runs in the parent; VM work runs in empty-env children.
pub(super) mod config;
use super::Format;
use anyhow::{ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use clap::{Args, Subcommand, ValueEnum};
use eplyx_engine::{
    local_store::{
        self, replay_inputs, safe_id, Metadata, Reproduction, ReproductionBinding,
        ReproductionOutcome, RunSource, SavedMigrationCounterexample, METADATA_VERSION,
        MIGRATION_COUNTEREXAMPLE_KIND, REPRODUCTION_VERSION,
    },
    migration::{
        self,
        gate::{self, Policy},
        input, pipeline,
        search::{self, Counterexample},
        spec::TokenMigrationV1,
    },
    replay::hash_bytes as sha256,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

#[derive(Subcommand)]
pub enum MigrationCommand {
    /// Capture or build state and evaluate a migration in the local VM.
    Analyse {
        #[arg(long, value_enum)]
        policy: Option<Policy>,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// Search a saved analysis offline, or analyse the configured project first.
    Search {
        #[arg(long)]
        run: Option<String>,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// Replay the saved bytes and evaluate a gate offline.
    Gate {
        #[arg(long)]
        run: String,
        #[arg(long, value_enum)]
        policy: Option<Policy>,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// Verify and export an UNSIGNED execution plan.
    Plan {
        #[arg(long)]
        run: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// Replay a saved counterexample offline and append reproduction history.
    Reproduce {
        id: String,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
    /// Inspect public-label fixture addresses without a provider.
    Fixture {
        recipe: PathBuf,
        #[arg(long, value_enum, default_value_t=Format::Text)]
        format: Format,
    },
}
#[derive(Args)]
pub struct ChangeArgs {
    #[arg(long)]
    spec: PathBuf,
    #[arg(long)]
    mechanism: PathBuf,
    #[arg(long)]
    program_id: Option<String>,
    #[arg(long, conflicts_with = "activation_unix")]
    activation_slot: Option<u64>,
    #[arg(long, conflicts_with = "activation_slot")]
    activation_unix: Option<i64>,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, value_enum, default_value_t=Format::Text)]
    format: Format,
}
#[derive(Clone, Copy, ValueEnum)]
pub enum WorkerAction {
    Gate,
    Search,
    Reproduce,
    Plan,
}
#[derive(Args)]
pub struct WorkerArgs {
    #[arg(long)]
    base: PathBuf,
    #[arg(long)]
    id: String,
    #[arg(long, value_enum)]
    action: WorkerAction,
    #[arg(long, value_enum)]
    policy: Option<Policy>,
}
#[derive(Serialize, Deserialize)]
pub struct Response {
    pub exit_code: u8,
    pub data: Value,
    pub text: String,
}
impl Response {
    fn ok(data: Value, text: impl Into<String>) -> Self {
        Self {
            exit_code: 0,
            data,
            text: text.into(),
        }
    }
}
/// Configuration/invalid input defaults to 2; typed state incompatibility is 4.
fn error_response(error: anyhow::Error) -> Response {
    let code = migration::error::exit_code(&error);
    let message = format!("{error:#}");
    Response {
        exit_code: code,
        text: format!("error: {message}"),
        data: json!({"error":{"code":code,"message":message}}),
    }
}
pub fn emit(format: Format, result: Result<Response>) -> ExitCode {
    let response = result.unwrap_or_else(error_response);
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&response.data).expect("JSON value")
        ),
        Format::Text => println!("{}", response.text),
    }
    ExitCode::from(response.exit_code)
}
pub fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>> {
    ensure!(!path.is_symlink(), "local files must not be symlinks");
    let file = fs::File::open(path).context("required local file is missing")?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= max,
        "local file exceeds its size bound"
    );
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= max,
        "local file exceeds its size bound"
    );
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn root_and_config(path: &Path) -> Result<(PathBuf, PathBuf)> {
    let absolute = std::env::current_dir()?.join(path);
    let root = absolute
        .parent()
        .context("config parent missing")?
        .canonicalize()?;
    let file = root.join(absolute.file_name().context("config filename missing")?);
    if file.exists() {
        ensure!(
            !file.is_symlink() && file.is_file(),
            "config must be a regular project file"
        );
    }
    Ok((root, file))
}
fn read_config(path: &Path) -> Result<config::MigrationConfig> {
    config::MigrationConfig::parse(std::str::from_utf8(&bounded_read(path, 64 * 1024)?)?)
}
fn directory(path: &Path, create: bool) -> Result<()> {
    if !path.exists() && create {
        fs::create_dir(path)?;
    }
    ensure!(
        path.is_dir() && path.canonicalize()? == path,
        "store directory missing or symlinked"
    );
    Ok(())
}
fn store(root: &Path, name: Option<&str>) -> Result<PathBuf> {
    let base = root.join(".eplyx");
    directory(&base, name.is_some())?;
    for child in ["runs", "counterexamples", "reproductions", "cache", "sync"] {
        directory(&base.join(child), name.is_some())?;
    }
    if let Some(name) = name {
        let file = base.join("project.json");
        if !file.exists() {
            // A local checkout identity, never a hosted MAIN proj_ ID or a registry of evidence.
            let id = format!("local_{}", &sha256(root.to_string_lossy().as_bytes())[..20]);
            write_new(
                &file,
                &serde_json::to_vec_pretty(&json!({"schema_version":1,"local_id":id,"name":name}))?,
            )?;
        }
    }
    Ok(base)
}
fn run_dir(base: &Path, id: &str) -> Result<PathBuf> {
    safe_id(id, "run_")?;
    let path = base.join("runs").join(id);
    directory(&path, false)?;
    for child in ["input", "result"] {
        directory(&path.join(child), false)?;
    }
    validate_store_tree(&path)?;
    Ok(path)
}
// Library replay follows only fixed run members. Reject symlinked members and
// special files before a worker reads anything from a local run supplied on disk.
fn validate_store_tree(root: &Path) -> Result<()> {
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut members = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        ensure!(depth <= 16, "local run nesting exceeds its bound");
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            members += 1;
            ensure!(members <= 4096, "local run member count exceeds its bound");
            let kind = entry.file_type()?;
            ensure!(
                kind.is_file() || kind.is_dir(),
                "local run contains a symlink or special file"
            );
            if kind.is_dir() {
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    Ok(())
}
fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&bounded_read(
        path,
        64 * 1024 * 1024,
    )?)?)
}
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env_clear()
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().into())
}
fn stamp() -> String {
    Utc::now().format("%Y%m%d%H%M%S%9f").to_string()
}
fn analyse(
    root: &Path,
    config: &config::MigrationConfig,
    base: &Path,
    policy: Policy,
) -> Result<Response> {
    let hash = sha256(&config::candidate(root, config)?);
    let id = format!("run_{}_{}", stamp(), &hash[..12]);
    let path = base.join("runs").join(&id);
    fs::create_dir(&path)?;
    let input = config::input_in(root, config, &path.join("input"))?;
    let report = pipeline::run(
        &path.join("input"),
        &path.join("result"),
        policy,
        pipeline::Isolation::Process(std::env::current_exe()?),
    )?;
    let meta = config::metadata(
        root,
        &id,
        &input,
        policy,
        report["gate_outcome"]
            .as_str()
            .context("gate outcome missing")?,
    )?;
    write_new(
        &path.join("metadata.json"),
        &serde_json::to_vec_pretty(&meta)?,
    )?;
    Ok(Response {
        exit_code: pipeline::exit_code(&report)?,
        text: render(&id, &report),
        data: json!({"run_id":id,"report":report,"metadata":meta}),
    })
}
fn render(id: &str, report: &Value) -> String {
    let label =
        eplyx_engine::presentation::label(report["gate_outcome"].as_str().unwrap_or("NotTested"));
    let mut text = format!(
        "Eplyx migration analysis · {id}\nToken Migration V1 rehearsal (local VM)\n{label}\n"
    );
    for axis in ["mechanism", "funding", "population", "reconciliation"] {
        text += &format!(
            "{axis}: {}\n",
            report["readiness"][axis]["status"]
                .as_str()
                .unwrap_or("Not evaluated")
        );
    }
    text += &format!(
        "Reason codes: {}\nRehearsal only: no transaction was signed or sent and no funds moved.",
        report["gate_reason_codes"]
    );
    text
}
fn verify_metadata(base: &Path, id: &str) -> Result<PathBuf> {
    let path = run_dir(base, id)?;
    let meta: Metadata = serde_json::from_value(read_json(&path.join("metadata.json"))?)?;
    let input = input::load(&path.join("input"))?;
    ensure!(
        meta.schema_version == METADATA_VERSION && meta.run_id == id,
        "run metadata identity mismatch"
    );
    migration::error::compatible(
        meta.analysis_input_sha256 == input.analysis_input_sha256()
            && meta.candidate_program_sha256 == input.program_sha256()
            && meta.change_spec_id == input.change_spec_id(),
        "run metadata is bound to another analytical input",
    )?;
    Ok(path)
}
fn worker(base: &Path, id: &str, action: WorkerAction, policy: Option<Policy>) -> Result<Response> {
    let mut command = local_store::offline_command(&std::env::current_exe()?);
    command
        .arg("migration-worker")
        .arg("--base")
        .arg(base)
        .arg("--id")
        .arg(id)
        .arg("--action")
        .arg(
            action
                .to_possible_value()
                .context("worker action")?
                .get_name(),
        );
    if let Some(policy) = policy {
        command.arg("--policy").arg(policy.name());
    }
    let output = command
        .output()
        .context("could not start offline migration worker")?;
    let response: Response = serde_json::from_slice(&output.stdout)
        .context("offline worker returned an invalid response")?;
    ensure!(
        output.status.code() == Some(i32::from(response.exit_code)),
        "offline worker exit status mismatch"
    );
    Ok(response)
}
pub fn worker_entry(args: WorkerArgs) -> ExitCode {
    let result = (|| {
        local_store::verify_offline_environment()?;
        worker_inner(args)
    })();
    let response = result.unwrap_or_else(error_response);
    println!(
        "{}",
        serde_json::to_string(&response).expect("worker response")
    );
    ExitCode::from(response.exit_code)
}
fn worker_inner(args: WorkerArgs) -> Result<Response> {
    directory(&args.base, false)?;
    if matches!(args.action, WorkerAction::Reproduce) {
        return reproduce(&args.base, &args.id);
    }
    let path = verify_metadata(&args.base, &args.id)?;
    match args.action {
        WorkerAction::Gate => {
            let mut report = pipeline::replay_with_policy(
                &path.join("input"),
                &path.join("result"),
                args.policy,
            )?;
            if path.join("search").exists() {
                directory(&path.join("search"), false)?;
                let result = search::replay(
                    &path.join("input"),
                    &path.join("result"),
                    &path.join("search"),
                )?;
                let policy = args
                    .policy
                    .unwrap_or(pipeline::bindings(&path.join("result"))?.gate_policy);
                let gate = gate::evaluate_migration_with_counterexamples(&report, policy, &result)?;
                let code = gate::exit_code(&report, &gate)?;
                report["gate_outcome"] = serde_json::to_value(gate.outcome)?;
                report["gate_reason_codes"] = serde_json::to_value(&gate.reason_codes)?;
                report["deployment_gate"] = serde_json::to_value(gate)?;
                return Ok(Response {
                    exit_code: code,
                    text: render(&args.id, &report),
                    data: report,
                });
            }
            Ok(Response {
                exit_code: pipeline::exit_code(&report)?,
                text: render(&args.id, &report),
                data: report,
            })
        }
        WorkerAction::Plan => {
            pipeline::replay(&path.join("input"), &path.join("result"))?;
            let data = read_json(&path.join("result").join(pipeline::UNSIGNED))?;
            Ok(Response::ok(
                data.clone(),
                serde_json::to_string_pretty(&data)?,
            ))
        }
        WorkerAction::Search => {
            let destination = path.join("search");
            let result = if destination.exists() {
                directory(&destination, false)?;
                search::replay(&path.join("input"), &path.join("result"), &destination)?
            } else {
                search::run(&path.join("input"), &path.join("result"), &destination)?
            };
            let hash = sha256(&bounded_read(
                &destination.join(search::SEARCH_ARTIFACT),
                64 * 1024 * 1024,
            )?);
            for cx in &result.counterexamples {
                let saved = SavedMigrationCounterexample {
                    schema_version: 1,
                    kind: MIGRATION_COUNTEREXAMPLE_KIND.into(),
                    id: search::counterexample_id(cx)?,
                    parent_run: args.id.clone(),
                    search_sha256: hash.clone(),
                    replay_inputs: replay_inputs(&args.id),
                    counterexample: cx.clone(),
                };
                let file = args
                    .base
                    .join("counterexamples")
                    .join(format!("{}.json", saved.id));
                let bytes = serde_json::to_vec_pretty(&saved)?;
                if file.exists() {
                    ensure!(
                        bounded_read(&file, 64 * 1024 * 1024)? == bytes,
                        "existing counterexample differs; history is immutable"
                    );
                } else {
                    write_new(&file, &bytes)?;
                }
            }
            let findings = result
                .counterexamples
                .iter()
                .map(|c| format!("{} · {:?}", c.claim(), c.finding()))
                .collect::<Vec<_>>()
                .join("\n");
            let text=format!("Migration counterexample search · {}\n{}\n{findings}\nNo network used. Derived witnesses are local states, not observed mainnet failures.",args.id,result.conclusion);
            Ok(Response::ok(
                json!({"run_id":args.id,"search":result}),
                text,
            ))
        }
        WorkerAction::Reproduce => unreachable!(),
    }
}
pub fn execute(config_path: &Path, action: MigrationCommand) -> ExitCode {
    let format = match &action {
        MigrationCommand::Analyse { format, .. }
        | MigrationCommand::Search { format, .. }
        | MigrationCommand::Gate { format, .. }
        | MigrationCommand::Plan { format, .. }
        | MigrationCommand::Reproduce { format, .. }
        | MigrationCommand::Fixture { format, .. } => *format,
    };
    emit(
        format,
        (|| {
            if let MigrationCommand::Fixture { recipe, .. } = &action {
                let recipe = migration::fixture::Recipe::parse(&bounded_read(
                    recipe,
                    input::MAX_RECIPE_BYTES,
                )?)?;
                let mut labels: Vec<&str> = recipe
                    .wallets
                    .iter()
                    .map(|x| x.label.as_str())
                    .chain(recipe.program_owned.iter().map(|x| x.label.as_str()))
                    .chain(recipe.multisigs.iter().map(|x| x.label.as_str()))
                    .chain(recipe.mints.iter().map(|x| x.label.as_str()))
                    .chain(recipe.token_accounts.iter().map(|x| x.label.as_str()))
                    .collect();
                labels.sort();
                let addresses:Vec<Value>=labels.into_iter().map(|label|json!({"label":label,"address":migration::fixture::address_of(&recipe,label)})).collect();
                return Ok(Response::ok(
                    json!({"synthetic_fixture":recipe.id,"addresses":addresses}),
                    format!(
                        "Synthetic fixture {}\n{}",
                        recipe.id,
                        serde_json::to_string_pretty(&addresses)?
                    ),
                ));
            }
            let (root, path) = root_and_config(config_path)?;
            match action {
                MigrationCommand::Analyse { policy, .. } => {
                    let c = read_config(&path)?;
                    let base = store(&root, Some(&c.project.name))?;
                    analyse(&root, &c, &base, policy.unwrap_or(c.gate.policy))
                }
                MigrationCommand::Search { run, .. } => {
                    let c = read_config(&path)?;
                    let base = store(&root, Some(&c.project.name))?;
                    let id = match run {
                        Some(id) => id,
                        None => analyse(&root, &c, &base, c.gate.policy)?.data["run_id"]
                            .as_str()
                            .context("analysis run ID")?
                            .into(),
                    };
                    let expected = config::validate_ephemeral(&root, &c, &base)?;
                    let run = verify_metadata(&base, &id)?;
                    let saved = input::load(&run.join("input"))?;
                    migration::error::compatible(
                        expected.analysis_input_sha256() == saved.analysis_input_sha256(),
                        "run is incompatible with the current config, spec or candidate binary",
                    )?;
                    worker(&base, &id, WorkerAction::Search, None)
                }
                MigrationCommand::Gate { run, policy, .. } => {
                    worker(&store(&root, None)?, &run, WorkerAction::Gate, policy)
                }
                MigrationCommand::Reproduce { id, .. } => {
                    worker(&store(&root, None)?, &id, WorkerAction::Reproduce, None)
                }
                MigrationCommand::Plan { run, out, .. } => {
                    let response = worker(&store(&root, None)?, &run, WorkerAction::Plan, None)?;
                    if response.exit_code != 0 {
                        return Ok(response);
                    }
                    if let Some(out) = out {
                        write_new(&out, &serde_json::to_vec_pretty(&response.data)?)?;
                        Ok(Response::ok(json!({"run_id":run,"unsigned_plan":response.data}),"Wrote UNSIGNED execution plan. Re-check all preconditions against current state before use."))
                    } else {
                        Ok(response)
                    }
                }
                MigrationCommand::Fixture { .. } => unreachable!(),
            }
        })(),
    )
}
pub fn doctor(config_path: &Path, format: Format) -> ExitCode {
    emit(
        format,
        (|| {
            let mut rows = vec![(
                true,
                format!(
                    "Installed binary    eplyx {} ({}) — running Eplyx needs no Rust compiler",
                    eplyx_engine::build_info::VERSION,
                    eplyx_engine::build_info::short_commit()
                ),
            )];
            let (root, path) = root_and_config(config_path)?;
            match read_config(&path) {
            Ok(c)=> {
                rows.push((true,"Project config      eplyx.toml (token_migration_v1)".into()));
                let base=store(&root,Some(&c.project.name))?;
                rows.extend(config::doctor_lines(&root,&c,&base));
                rows.push((true,if c.uses_rpc(){"RPC: set SOLANA_RPC_URL for read-only capture; provider was not contacted"}else{"RPC not needed: the state is a synthetic fixture"}.into()));
            },
            Err(error)=> rows.push((false,format!("Project config      {error:#}; run `eplyx init --migration` for a template"))),
        }
            let code = if rows.iter().all(|(ok, _)| *ok) { 0 } else { 2 };
            let text = rows
                .iter()
                .map(|(ok, text)| format!("{} {text}", if *ok { "✓" } else { "✗" }))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Response {
                exit_code: code,
                data: json!({"checks":rows,"exit_code":code}),
                text,
            })
        })(),
    )
}
pub fn runs(config_path: &Path, format: Format) -> ExitCode {
    emit(
        format,
        (|| {
            let (root, _) = root_and_config(config_path)?;
            let base = store(&root, None)?;
            let mut records = Vec::new();
            for entry in fs::read_dir(base.join("runs"))? {
                let entry = entry?;
                let id = entry.file_name().to_string_lossy().into_owned();
                if !local_store::is_safe_id(&id, "run_") || !entry.file_type()?.is_dir() {
                    continue;
                }
                let meta = entry.path().join("metadata.json");
                if meta.is_file() {
                    records.push(read_json(&meta)?);
                }
            }
            records.sort_by(|a, b| a["run_id"].as_str().cmp(&b["run_id"].as_str()));
            Ok(Response::ok(
                json!({"runs":records,"history_only":true}),
                serde_json::to_string_pretty(&records)?,
            ))
        })(),
    )
}
pub fn show(config_path: &Path, id: &str, format: Format) -> ExitCode {
    emit(
        format,
        (|| {
            let (root, _) = root_and_config(config_path)?;
            let base = store(&root, None)?;
            let path = run_dir(&base, id)?;
            let report = read_json(&path.join("result/report.json"))?;
            Ok(Response::ok(
                json!({"metadata":read_json(&path.join("metadata.json"))?,"report":report,"history_only":true}),
                format!(
                    "Saved history; viewing it does not re-execute the analysis.\n{}",
                    if report["transition_kind"] == "token_migration" {
                        render(id, &report)
                    } else {
                        serde_json::to_string_pretty(&report)?
                    }
                ),
            ))
        })(),
    )
}
pub fn change(args: ChangeArgs) -> ExitCode {
    emit(
        args.format,
        (|| {
            let mut spec: TokenMigrationV1 =
                serde_json::from_slice(&bounded_read(&args.spec, input::MAX_CHANGE_BYTES)?)?;
            if let Some(value) = args.activation_slot {
                spec.window.activation = Some(migration::spec::WindowBoundary::Slot {
                    value: value.to_string(),
                });
            }
            if let Some(value) = args.activation_unix {
                spec.window.activation = Some(migration::spec::WindowBoundary::UnixTimestamp {
                    value: value.to_string(),
                });
            }
            let bytes = bounded_read(&args.mechanism, input::MAX_PROGRAM_BYTES)?;
            input::validate_sbf(&bytes)?;
            let change = eplyx_engine::change::ChangeSpec::token_migration(
                spec,
                args.program_id
                    .as_deref()
                    .unwrap_or(migration::adapter::REFERENCE_PROGRAM_ID),
                &bytes,
            )?;
            change.resolve(eplyx_engine::change::CandidateSource::Bytes(&bytes))?;
            write_new(
                &args.out,
                eplyx_engine::canonical::document(&change)?.as_bytes(),
            )?;
            Ok(Response::ok(
                serde_json::to_value(&change)?,
                format!("Wrote token_migration ChangeSpec {}", change.id()?),
            ))
        })(),
    )
}

/// Offline re-execution of the saved run and search, then the exact counterexample.
fn reproduce(base: &Path, id: &str) -> Result<Response> {
    safe_id(id, "cx_")?;
    let saved: SavedMigrationCounterexample = serde_json::from_value(read_json(
        &base.join("counterexamples").join(format!("{id}.json")),
    )?)?;
    let result = verify(base, id, &saved);
    let binding = ReproductionBinding {
        analysis_input_sha256: match &saved.counterexample {
            Counterexample::MigrationObserved {
                analysis_input_sha256,
                ..
            }
            | Counterexample::MigrationDerived {
                analysis_input_sha256,
                ..
            } => analysis_input_sha256.clone(),
        },
        candidate_program_sha256: match &saved.counterexample {
            Counterexample::MigrationObserved {
                candidate_program_sha256,
                ..
            }
            | Counterexample::MigrationDerived {
                candidate_program_sha256,
                ..
            } => candidate_program_sha256.clone(),
        },
        world_sha256: match &saved.counterexample {
            Counterexample::MigrationObserved { world_sha256, .. }
            | Counterexample::MigrationDerived { world_sha256, .. } => world_sha256.clone(),
        },
        counterexample_kind: saved.counterexample.claim().into(),
        finding: format!("{:?}", saved.counterexample.finding()),
        expected_signature: match &saved.counterexample {
            Counterexample::MigrationObserved { signature, .. }
            | Counterexample::MigrationDerived { signature, .. } => {
                serde_json::to_value(signature)?
            }
        },
        reproduced_signature: result.as_ref().ok().cloned().flatten(),
        gate_outcome_with_finding: result.as_ref().ok().map(|_| "Block".to_string()),
    };
    let directory = base.join("reproductions");
    if !directory.exists() {
        fs::create_dir(&directory)?;
    }
    let now = Utc::now();
    let record = Reproduction {
        schema_version: REPRODUCTION_VERSION,
        id: eplyx_engine::local_store::reproduction_id(&stamp(), id),
        counterexample_id: id.into(),
        parent_run: Some(saved.parent_run.clone()),
        search_sha256: Some(saved.search_sha256.clone()),
        timestamp: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        outcome: if result.is_ok() {
            ReproductionOutcome::Reproduced
        } else {
            ReproductionOutcome::Failed
        },
        failure_signature_matched: result.is_ok(),
        error: result
            .as_ref()
            .err()
            .map(|e| format!("{e:#}").replace(&base.to_string_lossy().into_owned(), ".eplyx")),
        eplyx_version: env!("CARGO_PKG_VERSION").into(),
        engine_binary_sha256: sha256(&fs::read(std::env::current_exe()?)?),
        no_rpc: std::env::vars_os().next().is_none(),
        binding: Some(binding),
    };
    let name = format!("{}.json", record.id);
    write_new(
        &directory.join(&name),
        serde_json::to_vec_pretty(&record)?.as_slice(),
    )?;
    result.map(|_| Response::ok(serde_json::to_value(&record).expect("record"), format!("Reproduced {id}: same finding and failure signature.\nNo network used.\nRecorded .eplyx/reproductions/{name}")))
}

fn verify(base: &Path, id: &str, saved: &SavedMigrationCounterexample) -> Result<Option<Value>> {
    ensure!(
        saved.schema_version == 1
            && saved.kind == MIGRATION_COUNTEREXAMPLE_KIND
            && saved.id == id
            && saved.replay_inputs == replay_inputs(&saved.parent_run)
            && search::counterexample_id(&saved.counterexample)? == id,
        "counterexample identity mismatch"
    );
    let path = verify_metadata(base, &saved.parent_run)?;
    let artifact = path.join("search").join(search::SEARCH_ARTIFACT);
    ensure!(
        sha256(&fs::read(&artifact)?) == saved.search_sha256,
        "search artifact digest mismatch"
    );
    let replayed = search::replay(
        &path.join("input"),
        &path.join("result"),
        &path.join("search"),
    )?;
    ensure!(
        replayed.counterexamples.contains(&saved.counterexample),
        "counterexample missing from offline replay"
    );
    Ok(match &saved.counterexample {
        Counterexample::MigrationObserved { signature, .. }
        | Counterexample::MigrationDerived { signature, .. } => {
            Some(serde_json::to_value(signature)?)
        }
    })
}

/// Init replaces only explicit templates with --force; the run store remains append-only.
pub fn init(config_path: &Path, fixture: bool, force: bool, format: Format) -> ExitCode {
    emit(
        format,
        (|| {
            let (root, path) = root_and_config(config_path)?;
            ensure!(
                force || !path.exists(),
                "eplyx.toml already exists; use init --force to replace templates"
            );
            let name = root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("my-migration");
            let state = if fixture {
                "[state]\nsource = \"fixture\"\nfixture = \"fixtures/world.json\"\n"
            } else {
                "[state]\nsource = \"mainnet\"\n"
            };
            let mut template=format!("# Edit migration.json, build the candidate, then run eplyx doctor.\n[project]\nname = {name:?}\n\n[transition]\nadapter = \"token_migration_v1\"\nspec = \"migration.json\"\n\n[program]\npath = \"./target/deploy/migration.so\"\n\n{state}\n[rehearsal]\nclock = \"activation\"\nmax_units = 5000\n\n[gate]\npolicy = \"block-only\"\n");
            for invariant in migration::invariants::MigrationInvariant::recommended() {
                template += &format!(
                    "\n[[invariants]]\ntype = {:?}\nseverity = {:?}\n",
                    invariant.kind(),
                    serde_json::to_value(invariant.severity())?
                        .as_str()
                        .context("invariant severity")?
                );
            }
            let mut spec: Value = serde_json::from_str(include_str!(
                "../../examples/migrations/minimal/migration.json"
            ))?;
            spec["source"]["mint"] = "".into();
            spec["destination"]["mint"] = "".into();
            spec["authorities"]["expected"] = json!({});
            let mut files = vec![
                (path, template.into_bytes()),
                (
                    root.join("migration.json"),
                    serde_json::to_vec_pretty(&spec)?,
                ),
            ];
            if fixture {
                let dir = root.join("fixtures");
                directory(&dir, true)?;
                files.push((
                    dir.join("world.json"),
                    include_bytes!("../../examples/migrations/minimal/fixtures/world.json")
                        .to_vec(),
                ));
            }
            // Validate every destination before writing any template.
            for (file, _) in &files {
                ensure!(!file.is_symlink(), "refusing to replace a template symlink");
                ensure!(
                    !file.exists() || file.is_file(),
                    "template destination is not a file"
                );
            }
            for (file, bytes) in files {
                if force {
                    fs::write(file, bytes)?;
                } else if !file.exists() {
                    write_new(&file, &bytes)?;
                }
            }
            store(&root, Some(name))?;
            let ignore = root.join(".gitignore");
            ensure!(!ignore.is_symlink(), "refusing to edit a gitignore symlink");
            let text = fs::read_to_string(&ignore).unwrap_or_default();
            // Keep MAIN's bundle and expected-changes paths available to version control.
            let entries = [
                ".eplyx/project.json",
                ".eplyx/runs/",
                ".eplyx/counterexamples/",
                ".eplyx/reproductions/",
                ".eplyx/cache/",
                ".eplyx/sync/",
            ];
            let mut f = fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(ignore)?;
            if !text.is_empty() && !text.ends_with('\n') {
                writeln!(f)?;
            }
            for entry in entries {
                if !text.lines().any(|line| line.trim() == entry) {
                    writeln!(f, "{entry}")?;
                }
            }
            Ok(Response::ok(json!({"initialized":true,"kind":"token_migration","fixture":fixture}),"Created migration project. Fill in the public mints in migration.json, build your candidate and run eplyx doctor."))
        })(),
    )
}
