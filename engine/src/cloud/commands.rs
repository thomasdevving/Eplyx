//! `eplyx login`, `logout`, `link` and `sync`. These are the only commands
//! that read credentials or talk to the cloud; they run after analysis, read
//! the local store without changing canonical artifacts and never execute.
use super::{
    client::{explain, Client, CloudError},
    contract::{self, is_cloud_id, RunDocument, WHAT_IS_SYNCED},
    credentials::{self, Credentials, Entry},
    local::{self, CloudLink, RunSyncState, SyncStatus},
    privacy, PROJECT_ENV, SERVER_ENV, TOKEN_ENV,
};
use crate::{
    dashboard::{server::display_path, store::Store},
    local_store::{is_safe_id, SavedMigrationCounterexample},
};
use anyhow::{bail, ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, IsTerminal, Write},
    path::Path,
    time::{Duration, Instant},
};

/// Hosted Eplyx workspace used when no server is configured.
pub const DEFAULT_SERVER: Option<&str> = None;

/// Cloud settings from the process environment (CI uses these).
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub server: Option<String>,
    pub token: Option<String>,
    pub project: Option<String>,
}

impl Env {
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            server: var(SERVER_ENV),
            token: var(TOKEN_ENV),
            project: var(PROJECT_ENV),
        }
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn resolve_server(
    explicit: Option<&str>,
    env: &Env,
    link: Option<&CloudLink>,
    stored: &Credentials,
) -> Result<String> {
    let chosen = explicit
        .map(str::to_owned)
        .or_else(|| env.server.clone())
        .or_else(|| link.map(|l| l.server.clone()))
        .or_else(|| stored.default_server.clone())
        .or_else(|| DEFAULT_SERVER.map(str::to_owned))
        .context("no Eplyx cloud server configured; pass --server https://… or set EPLYX_URL")?;
    credentials::normalize_server(&chosen)
}

fn token_for(server: &str, env: &Env, stored: &Credentials) -> Result<String> {
    env.token
        .clone()
        .or_else(|| stored.servers.get(server).map(|e| e.token.clone()))
        .with_context(|| {
            format!("not signed in to {server}; run `eplyx login` (in CI, set EPLYX_TOKEN)")
        })
}

// ------------------------------------------------------------------- login

pub fn login(server: Option<&str>, open: bool) -> Result<u8> {
    let env = Env::from_process();
    let mut stored = credentials::load()?;
    let server = resolve_server(server, &env, None, &stored)?;
    let client = Client::new(&server, None)?;
    let label = format!(
        "eplyx CLI {} ({}-{})",
        crate::build_info::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let (_, start) = client
        .post("/v1/auth/device", &json!({"client": label}))
        .map_err(|e| explain(&e))?;
    let device_code = start["device_code"]
        .as_str()
        .context("invalid device response")?;
    let user_code = start["user_code"]
        .as_str()
        .context("invalid device response")?;
    let verification = start["verification_uri_complete"]
        .as_str()
        .or(start["verification_uri"].as_str())
        .context("invalid device response")?;
    ensure!(
        verification.starts_with(&format!("{server}/")),
        "device verification page is not on {server}"
    );
    let interval = start["interval"].as_u64().unwrap_or(3).clamp(1, 30);
    let expires = start["expires_in"].as_u64().unwrap_or(600).min(1800);
    println!("Sign in to Eplyx cloud at {server}\n\nOpen this page in your browser and confirm the code:\n  {verification}\n\nCode: {user_code}\n\nThe CLI never asks for your password. Waiting for approval… (Ctrl+C to cancel)");
    let _ = std::io::stdout().flush();
    if open && std::io::stdout().is_terminal() && std::env::var_os("CI").is_none() {
        crate::cloud::commands::open_browser(verification);
    }
    let deadline = Instant::now() + Duration::from_secs(expires);
    let mut wait = interval;
    loop {
        ensure!(
            Instant::now() < deadline,
            "the sign-in code expired; run `eplyx login` again"
        );
        std::thread::sleep(Duration::from_secs(wait));
        match client.post(
            "/v1/auth/device/token",
            &json!({"device_code": device_code}),
        ) {
            Ok((_, token)) => {
                let access = token["access_token"]
                    .as_str()
                    .context("invalid token response")?
                    .to_owned();
                let email = token["user"]["email"].as_str().unwrap_or("").to_owned();
                stored.servers.insert(
                    server.clone(),
                    Entry {
                        token: access,
                        user_email: email.clone(),
                        created_at: now(),
                    },
                );
                stored.default_server = Some(server.clone());
                let path = credentials::save(&stored)?;
                println!(
                    "\nSigned in as {email} on {server}.\nToken stored in {} (owner-only permissions). It is used only by `eplyx link` and `eplyx sync`.",
                    display_path(&path)
                );
                return Ok(0);
            }
            Err(CloudError::Status { message, .. }) if message == "authorization_pending" => {}
            Err(CloudError::Status { message, .. }) if message == "slow_down" => {
                wait = (wait + 2).min(30)
            }
            Err(CloudError::Status { message, .. }) if message == "access_denied" => {
                bail!("sign-in was denied in the browser")
            }
            Err(CloudError::Status { message, .. }) if message == "expired_token" => {
                bail!("the sign-in code expired; run `eplyx login` again")
            }
            Err(error) => return Err(explain(&error)),
        }
    }
}

pub fn logout(server: Option<&str>) -> Result<u8> {
    let env = Env::from_process();
    let mut stored = credentials::load()?;
    let server = match resolve_server(server, &Env::default(), None, &stored) {
        Ok(server) => server,
        Err(_) if stored.servers.is_empty() => {
            println!("Not signed in; nothing to remove.");
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    match stored.servers.remove(&server) {
        Some(entry) => {
            let revoked = Client::new(&server, Some(entry.token))
                .map(|client| client.delete("/v1/auth/token"))
                .map(|result| result.is_ok())
                .unwrap_or(false);
            if stored.default_server.as_deref() == Some(server.as_str()) {
                stored.default_server = stored.servers.keys().next().cloned();
            }
            credentials::save(&stored)?;
            println!(
                "Signed out of {server}. {}",
                if revoked {
                    "The token was revoked on the server and removed locally."
                } else {
                    "The token was removed locally; the server could not be reached to revoke it."
                }
            );
        }
        None => println!("No stored token for {server}; nothing to remove."),
    }
    if env.token.is_some() {
        println!(
            "Note: EPLYX_TOKEN is set in this environment and still applies until you unset it."
        );
    }
    println!("Local preflight, search, reproduce and the dashboard never needed the cloud and keep working.");
    Ok(0)
}

// -------------------------------------------------------------------- link

pub struct LinkArgs<'a> {
    pub server: Option<&'a str>,
    pub project: Option<&'a str>,
    pub workspace: Option<&'a str>,
    pub create: Option<&'a str>,
    pub unlink: bool,
    pub force: bool,
}

fn read_choice(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

pub fn link(base: &Path, args: LinkArgs) -> Result<u8> {
    let local = local::project(base)?;
    if args.unlink {
        local::write_link(base, None)?;
        println!(
            "Unlinked {} locally. Nothing was deleted in the cloud.",
            local.id
        );
        return Ok(0);
    }
    let env = Env::from_process();
    let stored = credentials::load()?;
    let server = resolve_server(args.server, &env, local.link.as_ref(), &stored)?;
    let client = Client::new(&server, Some(token_for(&server, &env, &stored)?))?;
    let chosen = match (
        args.project.map(str::to_owned).or(env.project.clone()),
        args.create,
    ) {
        (Some(project), _) => {
            ensure!(
                contract::is_project_id(&project),
                "invalid cloud project ID {project}"
            );
            project
        }
        (None, Some(name)) => create_project(&client, args.workspace, name, &local.id)?,
        (None, None) => choose_project(&client, args.workspace, &local.name, &local.id)?,
    };
    if let Some(existing) = &local.link {
        ensure!(
            args.force || (existing.project_id == chosen && existing.server == server),
            "this local project is already linked to {} on {}; pass --force to relink",
            existing.project_id,
            existing.server
        );
    }
    let project = client
        .get(&format!("/v1/projects/{chosen}/workspace"))
        .map_err(|e| explain(&e))?;
    let workspace_id = project["workspace"]["id"]
        .as_str()
        .filter(|id| is_cloud_id(id, "ws_"))
        .context("invalid project response")?
        .to_owned();
    client
        .post(
            &format!("/v1/projects/{chosen}/links"),
            &json!({"local_project_id": local.id}),
        )
        .map_err(|e| explain(&e))?;
    local::write_link(
        base,
        Some(&CloudLink {
            server: server.clone(),
            workspace_id,
            project_id: chosen.clone(),
            linked_at: now(),
        }),
    )?;
    println!(
        "Linked local project {} to {} / {}\n  {server}/p/{chosen}\n\nNothing was uploaded. Run `eplyx sync` to upload run metadata; `eplyx sync --dry-run` shows exactly what would be sent.",
        local.id,
        project["workspace"]["name"].as_str().unwrap_or("workspace"),
        project["project"]["name"].as_str().unwrap_or("project"),
    );
    Ok(0)
}

fn pick_workspace(workspaces: &[Value], requested: Option<&str>) -> Result<String> {
    if let Some(id) = requested {
        ensure!(is_cloud_id(id, "ws_"), "invalid workspace ID {id}");
        return Ok(id.to_owned());
    }
    match workspaces {
        [only] => only["id"]
            .as_str()
            .map(str::to_owned)
            .context("invalid workspace"),
        [] => bail!("you have no cloud workspace; create one in the web app first"),
        _ => bail!("you belong to several workspaces; pass --workspace ws_…"),
    }
}

fn create_project(
    client: &Client,
    workspace: Option<&str>,
    name: &str,
    local_id: &str,
) -> Result<String> {
    let listed = client.get("/v1/workspaces").map_err(|e| explain(&e))?;
    let workspaces = listed["workspaces"].as_array().cloned().unwrap_or_default();
    let workspace = pick_workspace(&workspaces, workspace)?;
    let (_, created) = client
        .post(
            &format!("/v1/workspaces/{workspace}/projects"),
            &json!({"name": name,"request_key":format!("link_{}",crate::replay::hash_bytes(&serde_json::to_vec(&(local_id,&workspace,name))?))}),
        )
        .map_err(|e| explain(&e))?;
    created["project"]["id"]
        .as_str()
        .map(str::to_owned)
        .context("invalid project response")
}

fn choose_project(
    client: &Client,
    workspace: Option<&str>,
    local_name: &str,
    local_id: &str,
) -> Result<String> {
    let listed = client.get("/v1/workspaces").map_err(|e| explain(&e))?;
    let workspaces = listed["workspaces"].as_array().cloned().unwrap_or_default();
    let mut options = Vec::new();
    for ws in &workspaces {
        if workspace.is_some_and(|w| ws["id"] != w) {
            continue;
        }
        for project in ws["projects"].as_array().into_iter().flatten() {
            options.push((
                project["id"].as_str().unwrap_or("").to_owned(),
                format!(
                    "{} / {}",
                    ws["name"].as_str().unwrap_or("?"),
                    project["name"].as_str().unwrap_or("?")
                ),
            ));
        }
    }
    if !std::io::stdin().is_terminal() {
        let listing = options
            .iter()
            .map(|(id, label)| format!("  {id}  {label}"))
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "choose a project non-interactively with --project proj_… or --create <name>{}",
            if listing.is_empty() {
                String::new()
            } else {
                format!("\nAvailable projects:\n{listing}")
            }
        );
    }
    println!("Link this local project to a cloud project:");
    for (n, (id, label)) in options.iter().enumerate() {
        println!("  {}) {label}  ({id})", n + 1);
    }
    let create_label = if local_name.is_empty() {
        "eplyx-project"
    } else {
        local_name
    };
    println!("  n) Create a new project named {create_label:?}");
    let answer = read_choice("Choice: ")?;
    if answer.eq_ignore_ascii_case("n") {
        return create_project(client, workspace, create_label, local_id);
    }
    let index: usize = answer
        .parse()
        .context("enter a number from the list or n")?;
    options
        .get(index.wrapping_sub(1))
        .map(|(id, _)| id.clone())
        .context("no such option")
}

// -------------------------------------------------------------------- sync

pub struct SyncArgs<'a> {
    pub run: Option<&'a str>,
    pub latest: bool,
    pub dry_run: bool,
    pub json: bool,
}

struct Plan {
    run: RunDocument,
    counterexamples: Vec<contract::CounterexampleDocument>,
    reproductions: Vec<contract::ReproductionDocument>,
}

/// Values that must never appear in an upload, known only on this machine.
fn local_secrets(env: &Env, stored: &Credentials) -> Vec<String> {
    let mut secrets: Vec<String> = stored.servers.values().map(|e| e.token.clone()).collect();
    secrets.extend(env.token.clone());
    secrets
}

fn build_plans(root: &Path, local_id: &str, args: &SyncArgs) -> Result<(Vec<Plan>, Vec<String>)> {
    let store = Store::open(root)?;
    let (all_runs, _) = store.run_ids()?;
    let complete: Vec<String> = all_runs
        .into_iter()
        .filter(|id| {
            store
                .read(&["runs", id, "metadata.json"], 64 * 1024)
                .ok()
                .flatten()
                .is_some()
        })
        .collect();
    let notes = Vec::new();
    let selected: Vec<String> = match (args.run, args.latest) {
        (Some(run), _) => {
            ensure!(is_safe_id(run, "run_"), "invalid run ID {run}");
            ensure!(
                complete.iter().any(|id| id == run),
                "{run} is not a complete local run (see `eplyx runs`)"
            );
            vec![run.to_owned()]
        }
        (None, true) => complete.last().cloned().into_iter().collect(),
        (None, false) => complete.clone(),
    };
    let mut saved: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (counterexamples, _) = store.counterexample_ids()?;
    for id in &counterexamples {
        let file = format!("{id}.json");
        let bytes = store
            .read(&["counterexamples", &file], 1024 * 1024)?
            .context("saved counterexample file is missing")?;
        let cx: SavedMigrationCounterexample =
            serde_json::from_slice(&bytes).context("saved counterexample file is invalid")?;
        saved.entry(cx.parent_run).or_default().push(id.clone());
    }
    let (reproductions, _) = store.reproduction_ids()?;
    let mut plans = Vec::new();
    for run in &selected {
        let document = contract::run_document(&store, local_id, run)?;
        document
            .verify()
            .with_context(|| format!("{run} cannot be synced"))?;
        let mut cx_docs = Vec::new();
        let mut cx_ids = BTreeSet::new();
        for cx in saved.get(run).into_iter().flatten() {
            let doc = contract::counterexample_document(&store, local_id, cx)?;
            doc.verify()
                .with_context(|| format!("counterexample {cx} cannot be synced"))?;
            cx_ids.insert(cx.clone());
            cx_docs.push(doc);
        }
        let mut repro_docs = Vec::new();
        for repro in &reproductions {
            let file = format!("{repro}.json");
            let bytes = store
                .read(&["reproductions", &file], 16 * 1024)?
                .context("saved reproduction file is missing")?;
            let record: crate::local_store::Reproduction =
                serde_json::from_slice(&bytes).context("saved reproduction file is invalid")?;
            if !cx_ids.contains(&record.counterexample_id) {
                continue;
            }
            let doc = contract::reproduction_document(&store, local_id, repro, &[])?;
            doc.verify()
                .with_context(|| format!("reproduction {repro} cannot be synced"))?;
            repro_docs.push(doc);
        }
        plans.push(Plan {
            run: document,
            counterexamples: cx_docs,
            reproductions: repro_docs,
        });
    }
    Ok((plans, notes))
}

fn check_secrets(plans: &[Plan], secrets: &[String]) -> Result<()> {
    for plan in plans {
        let mut texts = vec![serde_json::to_string(&plan.run)?];
        texts.extend(
            plan.counterexamples
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()?,
        );
        texts.extend(
            plan.reproductions
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()?,
        );
        for text in texts {
            privacy::scan_secrets(&plan.run.run_id, &text, secrets)?;
        }
    }
    Ok(())
}

pub fn sync(root: &Path, base: &Path, args: SyncArgs) -> Result<u8> {
    let local_project = local::project(base)?;
    let env = Env::from_process();
    let stored = credentials::load()?;
    let (plans, notes) = build_plans(root, &local_project.id, &args)?;
    check_secrets(&plans, &local_secrets(&env, &stored))?;
    if args.dry_run {
        return dry_run(&plans, &notes, args.json);
    }
    if plans.is_empty() {
        println!("No complete local runs to sync. Run `eplyx migration analyse` first.");
        return Ok(0);
    }
    let project = env
        .project
        .clone()
        .or_else(|| local_project.link.as_ref().map(|l| l.project_id.clone()))
        .context("this local project is not linked to a cloud project; run `eplyx link` (in CI, set EPLYX_PROJECT_ID)")?;
    ensure!(
        contract::is_project_id(&project),
        "invalid cloud project ID {project}"
    );
    let server = resolve_server(None, &env, local_project.link.as_ref(), &stored)?;
    let client = Client::new(&server, Some(token_for(&server, &env, &stored)?))?;
    let mut failed = false;
    for plan in &plans {
        let run = plan.run.run_id.clone();
        let previous = local::read_state(base, &run).ok().flatten();
        let mut state = RunSyncState {
            schema_version: 1,
            run_id: run.clone(),
            server: server.clone(),
            cloud_project_id: project.clone(),
            status: SyncStatus::Failed,
            core_sha256: Some(plan.run.core_sha256()?),
            search_sha256: plan.run.search.as_ref().map(|s| s.sha256.clone()),
            counterexamples_synced: 0,
            reproductions_synced: 0,
            last_attempt_at: now(),
            last_synced_at: previous.and_then(|p| p.last_synced_at),
            error: None,
            url: None,
        };
        println!("Syncing {run}");
        let outcome = sync_one(&client, &project, plan, &mut state);
        match outcome {
            Ok(url) => {
                state.status = SyncStatus::Synced;
                state.last_synced_at = Some(state.last_attempt_at.clone());
                state.url = Some(url.clone());
                println!("\n  Synced to:\n  {url}\n");
            }
            Err(error) => {
                failed = true;
                state.error = Some(format!("{error:#}").lines().next().unwrap_or("").to_owned());
                eprintln!("  ✗ {error:#}\n");
            }
        }
        if let Err(error) = local::write_state(base, &state) {
            eprintln!("warning: sync state not recorded: {error:#}");
        }
        if failed
            && state
                .error
                .as_deref()
                .is_some_and(|e| e.contains("could not reach"))
        {
            eprintln!("Stopped: the cloud is unreachable. Nothing local changed; run `eplyx sync` again to retry.");
            break;
        }
    }
    for note in notes {
        println!("note: {note}");
    }
    println!("Synced results are copies of local engine output. Viewing them never reruns RPC or execution.");
    Ok(if failed { 2 } else { 0 })
}

fn sync_one(
    client: &Client,
    project: &str,
    plan: &Plan,
    state: &mut RunSyncState,
) -> Result<String> {
    let (_, response) = client
        .post(&format!("/v1/projects/{project}/sync/runs"), &plan.run)
        .map_err(|e| explain(&e))?;
    let status = response["status"].as_str().unwrap_or("synced");
    println!(
        "  ✓ Run metadata ({})",
        match status {
            "created" => "uploaded",
            "unchanged" => "already synced, identical",
            "search_attached" => "search result attached",
            other => other,
        }
    );
    for doc in &plan.counterexamples {
        client
            .post(&format!("/v1/projects/{project}/sync/counterexamples"), doc)
            .map_err(|e| explain(&e))
            .with_context(|| format!("counterexample {}", doc.counterexample_id))?;
        state.counterexamples_synced += 1;
    }
    println!(
        "  ✓ {} counterexample{}",
        plan.counterexamples.len(),
        if plan.counterexamples.len() == 1 {
            ""
        } else {
            "s"
        }
    );
    for doc in &plan.reproductions {
        client
            .post(&format!("/v1/projects/{project}/sync/reproductions"), doc)
            .map_err(|e| explain(&e))
            .with_context(|| format!("reproduction record {}", doc.reproduction_id))?;
        state.reproductions_synced += 1;
    }
    println!(
        "  ✓ {} reproduction record{}",
        plan.reproductions.len(),
        if plan.reproductions.len() == 1 {
            ""
        } else {
            "s"
        }
    );
    Ok(response["url"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}/p/{project}/runs/{}", client.server(), plan.run.run_id)))
}

fn dry_run(plans: &[Plan], notes: &[String], as_json: bool) -> Result<u8> {
    if as_json {
        let documents: Vec<Value> = plans
            .iter()
            .map(|p| {
                json!({
                    "run": p.run,
                    "counterexamples": p.counterexamples,
                    "reproductions": p.reproductions,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&documents)?);
        return Ok(0);
    }
    println!("Eplyx sync — dry run (nothing is sent)\n\n{WHAT_IS_SYNCED}\n");
    for plan in plans {
        let bytes = serde_json::to_vec(&plan.run)?.len();
        println!(
            "{}  {} KB run document{} · {} counterexamples · {} reproduction records",
            plan.run.run_id,
            bytes / 1024,
            if plan.run.search.is_some() {
                " incl. search"
            } else {
                ""
            },
            plan.counterexamples.len(),
            plan.reproductions.len()
        );
    }
    if plans.is_empty() {
        println!("No complete local runs.");
    }
    for note in notes {
        println!("note: {note}");
    }
    println!("\nRun `eplyx sync --dry-run --json` to print the exact documents.");
    Ok(0)
}

pub fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    let _ = command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
