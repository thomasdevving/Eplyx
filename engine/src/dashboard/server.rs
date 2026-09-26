//! Loopback-only HTTP server for the local dashboard. It answers GET and HEAD
//! on fixed routes, resolves every ID through the index and never lets a
//! browser-supplied string name a filesystem path. It executes nothing.
use super::{
    assets,
    store::{Index, Store, CAPTURE_LIMIT, SMALL_LIMIT},
    view,
};
use crate::local_store::is_safe_id;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

pub const DEFAULT_PORT: u16 = 4173;
const PORT_ATTEMPTS: u16 = 20;
const MAX_HEAD: usize = 16 * 1024;
const MAX_CONNECTIONS: usize = 64;

/// Pages served by the single-page application shell.
const PAGES: [&str; 8] = [
    "runs",
    "counterexamples",
    "compare",
    "production",
    "invariants",
    "gate",
    "project",
    "",
];

pub struct Dashboard {
    listener: TcpListener,
    address: SocketAddr,
    state: Arc<State>,
}

struct State {
    store: Store,
    index: Mutex<Index>,
    context: Value,
    port: u16,
    active: AtomicUsize,
}

struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    host: Option<String>,
}

enum Body {
    Bytes(Vec<u8>),
    File(File, u64),
}

struct Response {
    status: u16,
    content_type: String,
    body: Body,
    headers: Vec<(String, String)>,
}

impl Response {
    fn json(status: u16, value: &Value) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8".into(),
            body: Body::Bytes(serde_json::to_vec(value).unwrap_or_default()),
            headers: Vec::new(),
        }
    }

    fn error(status: u16, message: &str) -> Self {
        Self::json(status, &json!({"error": message}))
    }

    fn bytes(content_type: &str, bytes: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: content_type.into(),
            body: Body::Bytes(bytes),
            headers: Vec::new(),
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// Presentation-only path: the home directory becomes `~` so screenshots and
/// shared views do not expose a local user name.
pub fn display_path(path: &Path) -> String {
    let text = plain_path(path).to_string_lossy().into_owned();
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(|variable| std::env::var(variable).ok())
        .find_map(|home| abbreviate_home(&text, &home))
        .unwrap_or(text)
}

/// Strip a Windows drive path's verbatim prefix for display, preserving UNC paths.
fn plain_path(path: &Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.into(),
        _ => path.into(),
    }
}

/// `~` plus the remainder when `path` lies strictly inside `home`, for either
/// separator, so Windows and Unix paths display the same way.
pub fn abbreviate_home(path: &str, home: &str) -> Option<String> {
    let home = home.trim_end_matches(['/', '\\']);
    let rest = path.strip_prefix(home).filter(|_| !home.is_empty())?;
    (rest.starts_with('/') || rest.starts_with('\\')).then(|| format!("~{rest}"))
}

fn allowed_target(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.' | b'?' | b'=' | b'&')
}

fn parse_target(target: &str) -> Option<(String, Vec<(String, String)>)> {
    if !target.starts_with('/') || !target.bytes().all(allowed_target) {
        return None; // Rejects percent-encoding, backslashes and spaces outright.
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path.contains("..") || path.contains("//") || query.contains('?') {
        return None;
    }
    let query = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key.to_owned(), value.to_owned())
        })
        .collect();
    Some((path.trim_end_matches('/').to_owned(), query))
}

fn read_request(stream: &mut TcpStream) -> Result<Request, u16> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        if buffer.len() > MAX_HEAD {
            return Err(413);
        }
        let read = stream.read(&mut chunk).map_err(|_| 400u16)?;
        if read == 0 {
            return Err(400);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_HEAD {
            return Err(413);
        }
    }
    let head = std::str::from_utf8(&buffer).map_err(|_| 400u16)?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target, version) = (first.next(), first.next(), first.next());
    let (Some(method), Some(target), Some(version)) = (method, target, version) else {
        return Err(400);
    };
    if !version.starts_with("HTTP/1.") || first.next().is_some() {
        return Err(400);
    }
    let (path, query) = parse_target(target).ok_or(400u16)?;
    let hosts: Vec<String> = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim().to_ascii_lowercase())
        .collect();
    if hosts.len() != 1 {
        return Err(400);
    }
    let host = hosts.into_iter().next();
    Ok(Request {
        method: method.to_owned(),
        path,
        query,
        host,
    })
}

fn write_response(stream: &mut TcpStream, head_only: bool, response: Response) -> io::Result<()> {
    let length = match &response.body {
        Body::Bytes(bytes) => bytes.len() as u64,
        Body::File(_, length) => *length,
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {length}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nCross-Origin-Resource-Policy: same-origin\r\nContent-Security-Policy: default-src 'self'; img-src 'self' data:; style-src 'self'; style-src-attr 'unsafe-inline'; script-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nConnection: close\r\n",
        response.status,
        reason(response.status),
        response.content_type,
    );
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if !head_only {
        match response.body {
            Body::Bytes(bytes) => stream.write_all(&bytes)?,
            Body::File(file, length) => {
                io::copy(&mut file.take(length), stream)?;
            }
        }
    }
    stream.flush()
}

impl State {
    /// Refresh the cache against the store and return consistent copies of
    /// its run and counterexample summaries, newest run first.
    fn snapshot(&self) -> Result<(Vec<Value>, Vec<Value>, usize)> {
        let mut index = self.index.lock().unwrap_or_else(|p| p.into_inner());
        if self.store.refresh(&mut index)? {
            if let Err(error) = self.store.save_index(&index) {
                eprintln!("dashboard cache not written: {error:#}");
            }
        }
        let (_, ignored_runs) = self.store.run_ids()?;
        let (_, ignored_cx) = self.store.counterexample_ids()?;
        let (_, ignored_repro) = self.store.reproduction_ids()?;
        // Newest first: reproduction IDs start with their UTC timestamp.
        let history: Vec<Value> = index
            .reproductions
            .values()
            .rev()
            .map(|e| e.summary.clone())
            .collect();
        let (runs, files) = view::assemble(
            index.runs.values().map(|e| e.summary.clone()).collect(),
            index
                .counterexamples
                .values()
                .map(|e| e.summary.clone())
                .collect(),
            &history,
        );
        Ok((runs, files, ignored_runs + ignored_cx + ignored_repro))
    }

    /// Reproduction records from the index, newest first. Call after snapshot.
    fn reproductions(&self) -> Vec<Value> {
        let index = self.index.lock().unwrap_or_else(|p| p.into_inner());
        index
            .reproductions
            .values()
            .rev()
            .map(|e| e.summary.clone())
            .collect()
    }

    fn project(&self) -> Result<Value> {
        let (runs, files, ignored) = self.snapshot()?;
        let project = self
            .store
            .json(&["project.json"], SMALL_LIMIT)
            .ok()
            .flatten()
            .unwrap_or(Value::Null);
        let root_name = self
            .store
            .root()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        let mut payload = view::project_payload(
            json!({
                "name": project["name"].as_str().map(str::to_owned).or_else(|| self.context["config"]["name"].as_str().map(str::to_owned)).or(root_name),
                "id": project["local_id"],
            }),
            self.context.clone(),
            json!({
                "path": ".eplyx/",
                "root_display": display_path(self.store.root()),
                "index": ".eplyx/cache/dashboard-index.json",
            }),
            &runs,
            &files,
            &self.reproductions(),
            ignored,
        );
        payload["cloud"] = json!({"linked": false});
        Ok(payload)
    }

    fn known_run(&self, id: &str) -> Result<bool> {
        if !is_safe_id(id, "run_") {
            return Ok(false);
        }
        self.snapshot()?;
        Ok(self
            .index
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .runs
            .contains_key(id))
    }

    fn route(&self, request: &Request) -> Result<Response> {
        let segments: Vec<&str> = request.path.trim_start_matches('/').split('/').collect();
        let query = |key: &str| {
            request
                .query
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        Ok(match segments.as_slice() {
            ["api", "project"] => Response::json(200, &self.project()?),
            ["api", "runs"] => {
                let (runs, _, ignored) = self.snapshot()?;
                Response::json(
                    200,
                    &json!({"runs": runs, "ignored_store_entries": ignored}),
                )
            }
            ["api", "runs", id] => {
                if !self.known_run(id)? {
                    return Ok(Response::error(404, "unknown run"));
                }
                let (runs, files, _) = self.snapshot()?;
                let mut detail = view::run_detail(&self.store, id, &files)?;
                if let Some(run) = runs.iter().find(|r| r["id"] == *id) {
                    detail["number"] = run["number"].clone();
                    detail["saved_counterexamples"] = run["saved_counterexamples"].clone();
                }
                let position = runs.iter().position(|r| r["id"] == *id);
                detail["previous_run"] = position
                    .and_then(|p| runs.get(p + 1))
                    .map_or(Value::Null, |r| r["id"].clone());
                detail["sync"] = position.map_or(Value::Null, |p| runs[p]["sync"].clone());
                Response::json(200, &detail)
            }
            ["api", "runs", id, "artifacts", name] => {
                let Some((member, kind)) = view::artifact(name) else {
                    return Ok(Response::error(404, "unknown artifact"));
                };
                if !self.known_run(id)? {
                    return Ok(Response::error(404, "unknown run"));
                }
                let mut parts = vec!["runs", *id];
                parts.extend(member.split('/'));
                match self.store.open_file(&parts, CAPTURE_LIMIT)? {
                    Some((file, length)) => {
                        let disposition = if query("download") == Some("1") {
                            "attachment"
                        } else {
                            "inline"
                        };
                        Response {
                            status: 200,
                            content_type: kind.into(),
                            body: Body::File(file, length),
                            headers: vec![(
                                "Content-Disposition".into(),
                                format!("{disposition}; filename=\"{id}-{name}\""),
                            )],
                        }
                    }
                    None => Response::error(404, "artifact not present in this run"),
                }
            }
            ["api", "counterexamples"] => {
                let (_, files, _) = self.snapshot()?;
                Response::json(200, &json!({"counterexamples": files}))
            }
            ["api", "counterexamples", id] | ["api", "counterexamples", id, "raw"] => {
                if !is_safe_id(id, "cx_") {
                    return Ok(Response::error(404, "unknown counterexample"));
                }
                let (runs, files, _) = self.snapshot()?;
                let Some(summary) = files.iter().find(|c| c["id"] == *id) else {
                    return Ok(Response::error(404, "unknown counterexample"));
                };
                if segments.len() == 4 {
                    let file = format!("{id}.json");
                    let bytes = self
                        .store
                        .read(&["counterexamples", &file], SMALL_LIMIT)?
                        .context("counterexample file missing")?;
                    let mut response = Response::bytes("application/json", bytes);
                    response.headers.push((
                        "Content-Disposition".into(),
                        format!("attachment; filename=\"{id}.json\""),
                    ));
                    return Ok(response);
                }
                let mut detail = view::counterexample_detail(&self.store, id, summary)?;
                detail["parent_summary"] = runs
                    .iter()
                    .find(|r| r["id"] == summary["parent_run"])
                    .cloned()
                    .unwrap_or(Value::Null);
                Response::json(200, &detail)
            }
            ["api", "compare"] => {
                let (Some(left), Some(right)) = (query("left"), query("right")) else {
                    return Ok(Response::error(400, "choose two runs: left and right"));
                };
                if !self.known_run(left)? || !self.known_run(right)? {
                    return Ok(Response::error(404, "unknown run"));
                }
                let (runs, _, _) = self.snapshot()?;
                let mut comparison = view::compare(&self.store, left, right)?;
                for (side, id) in [("left", left), ("right", right)] {
                    if let Some(run) = runs.iter().find(|r| r["id"] == id) {
                        comparison[side]["number"] = run["number"].clone();
                    }
                }
                Response::json(200, &comparison)
            }
            ["api", ..] => Response::error(404, "unknown API route"),
            ["assets", name] => match assets::get(name) {
                Some((kind, body)) => Response::bytes(kind, body.to_vec()),
                None => Response::error(404, "unknown asset"),
            },
            [page] if PAGES.contains(page) => Response::bytes(
                "text/html; charset=utf-8",
                assets::INDEX.as_bytes().to_vec(),
            ),
            ["runs", id] if is_safe_id(id, "run_") => Response::bytes(
                "text/html; charset=utf-8",
                assets::INDEX.as_bytes().to_vec(),
            ),
            ["counterexamples", id] if is_safe_id(id, "cx_") => Response::bytes(
                "text/html; charset=utf-8",
                assets::INDEX.as_bytes().to_vec(),
            ),
            _ => Response::error(404, "not found"),
        })
    }

    fn handle(&self, mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(60)));
        let request = match read_request(&mut stream) {
            Ok(request) => request,
            Err(status) => {
                let _ = write_response(
                    &mut stream,
                    false,
                    Response::error(status, "malformed request"),
                );
                return;
            }
        };
        let head_only = request.method == "HEAD";
        let allowed_hosts = [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
        ];
        let response = if !matches!(request.method.as_str(), "GET" | "HEAD") {
            let mut response = Response::error(405, "the dashboard is read-only");
            response.headers.push(("Allow".into(), "GET, HEAD".into()));
            response
        } else if !request
            .host
            .as_ref()
            .is_some_and(|host| allowed_hosts.contains(host))
        {
            // Rejects DNS-rebinding pages that resolve their own name to loopback.
            Response::error(403, "host not allowed")
        } else {
            self.route(&request).unwrap_or_else(|error| {
                eprintln!("dashboard request failed: {error:#}");
                Response::error(500, "the dashboard could not read this local artifact")
            })
        };
        let _ = write_response(&mut stream, head_only, response);
    }
}

impl Dashboard {
    /// Open the store, rebuild the cache if needed and bind loopback only.
    /// `Some(0)` asks the OS for a free port; `None` tries 4173 upward.
    pub fn bind(root: &Path, port: Option<u16>, context: Value) -> Result<Self> {
        let store = Store::open(root)?;
        let mut index = store.load_index();
        if store.refresh(&mut index)? {
            if let Err(error) = store.save_index(&index) {
                eprintln!("dashboard cache not written: {error:#}");
            }
        }
        let listener = match port {
            Some(port) => TcpListener::bind((Ipv4Addr::LOCALHOST, port))
                .with_context(|| format!("could not listen on 127.0.0.1:{port}"))?,
            None => (0..PORT_ATTEMPTS)
                .find_map(|offset| {
                    TcpListener::bind((Ipv4Addr::LOCALHOST, DEFAULT_PORT + offset)).ok()
                })
                .context("no free local port between 4173 and 4192; pass --port")?,
        };
        let address = listener.local_addr()?;
        Ok(Self {
            listener,
            address,
            state: Arc::new(State {
                store,
                index: Mutex::new(index),
                context,
                port: address.port(),
                active: AtomicUsize::new(0),
            }),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.address.port())
    }

    /// Project name, run count and saved counterexample count for the banner.
    pub fn banner(&self) -> Result<(String, usize, usize)> {
        let project = self.state.project()?;
        Ok((
            project["project"]["name"]
                .as_str()
                .unwrap_or("unnamed project")
                .to_owned(),
            project["stats"]["runs"].as_u64().unwrap_or(0) as usize,
            project["stats"]["counterexamples_saved"]
                .as_u64()
                .unwrap_or(0) as usize,
        ))
    }

    pub fn serve(self) -> Result<()> {
        for stream in self.listener.incoming() {
            let Ok(stream) = stream else { continue };
            let state = Arc::clone(&self.state);
            if state.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                state.active.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            std::thread::spawn(move || {
                state.handle(stream);
                state.active.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_reject_traversal_and_encoding() {
        for target in [
            "/api/runs/../project",
            "/api/runs/%2e%2e/x",
            "/api/runs/run_a%2Fb",
            "/assets/..%5c..%5cCargo.toml",
            "/api/runs/run_a\\..\\x",
            "//etc/passwd",
            "api/runs",
            "/api/runs?x=1?y",
            "/api/runs/run a",
        ] {
            assert!(parse_target(target).is_none(), "{target}");
        }
        let (path, query) = parse_target("/api/compare?left=run_a&right=run_b").unwrap();
        assert_eq!(path, "/api/compare");
        assert_eq!(
            query,
            vec![
                ("left".into(), "run_a".into()),
                ("right".into(), "run_b".into())
            ]
        );
    }

    #[test]
    fn verbatim_windows_paths_become_plain() {
        assert_eq!(
            plain_path(Path::new(r"\\?\C:\Users\dev\app")),
            Path::new(r"C:\Users\dev\app")
        );
        assert_eq!(
            plain_path(Path::new(r"\\?\UNC\server\share")),
            Path::new(r"\\?\UNC\server\share")
        );
        assert_eq!(
            plain_path(Path::new("/home/dev/app")),
            Path::new("/home/dev/app")
        );
    }

    #[test]
    fn home_abbreviation_handles_both_separators() {
        assert_eq!(
            abbreviate_home("/home/dev/work/app", "/home/dev/").as_deref(),
            Some("~/work/app")
        );
        assert_eq!(
            abbreviate_home("C:\\Users\\dev\\work\\app", "C:\\Users\\dev").as_deref(),
            Some("~\\work\\app")
        );
        assert_eq!(abbreviate_home("/home/devil/app", "/home/dev"), None);
        assert_eq!(abbreviate_home("/srv/app", ""), None);
    }
}
