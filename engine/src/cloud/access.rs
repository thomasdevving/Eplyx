//! Eplyx access: the hosted Solana RPC, and the password that unlocks it.
//!
//! Every command that reads the chain resolves its endpoint the same way: an
//! explicit flag, then the named environment variables, then the Eplyx hosted
//! RPC at `<server>/v1/rpc`. A developer therefore needs no provider of their
//! own; bringing one is still an explicit override.
//!
//! The hosted RPC needs the access password: `EPLYX_ACCESS_PASSWORD`, else the
//! one `eplyx access` verified and saved owner-only beside the cloud
//! credentials. A saved password is only ever sent to the server it was saved
//! for. It is transport configuration: sent as a bearer header over curl's
//! stdin, never in process arguments, a cache key, a record or a report.
//! Analysis and replay never read it; only commands that capture chain state do.
use super::credentials::{config_dir, normalize_server, read_private, write_private};
use super::SERVER_ENV;
use crate::ingest::rpc::{HttpRpc, RpcProvider, MAINNET_GENESIS};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

pub const PASSWORD_ENV: &str = "EPLYX_ACCESS_PASSWORD";
/// Where the hosted RPC lives unless `EPLYX_URL` or `eplyx access --server`
/// names another Eplyx service.
pub const DEFAULT_SERVER: &str = "https://eplyx.dev";
pub const RPC_PATH: &str = "/v1/rpc";
pub const FILE: &str = "access.json";
/// The hosted RPC is a relay, so it is one hop slower than a provider; large
/// block and account reads need more than the direct default.
const HOSTED_TIMEOUT_SECONDS: u64 = 90;

const NO_ACCESS: &str = "no Solana RPC is configured. Run `eplyx access` and enter your Eplyx \
     access password to use the hosted Eplyx RPC, or bring your own with --rpc-url / SOLANA_RPC_URL";

/// The password `eplyx access` saved, and the only server it is sent to.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    schema_version: u32,
    server: String,
    password: String,
}

/// 8 to 256 printable ASCII characters with no spaces, quotes or backslashes:
/// exactly what can travel in a bearer header unescaped.
pub fn validate_password(password: &str) -> Result<()> {
    ensure!(
        (8..=256).contains(&password.len())
            && password
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\'),
        "an Eplyx access password is 8-256 printable characters, without spaces, quotes or backslashes"
    );
    Ok(())
}

/// A resolved read-only Solana endpoint.
#[derive(Clone)]
pub struct Endpoint {
    url: String,
    hosted: Option<Hosted>,
}

#[derive(Clone)]
struct Hosted {
    server: String,
    password: Option<String>,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Both forms can carry a credential: a provider key in the URL, or the
        // access password. Neither is ever formatted.
        f.write_str(match self.hosted {
            Some(_) => "Endpoint(Eplyx hosted RPC)",
            None => "Endpoint(own RPC, redacted)",
        })
    }
}

impl Endpoint {
    /// A developer's own provider.
    pub fn own(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            hosted: None,
        }
    }

    /// The hosted RPC on `server`. A missing password is not an error here:
    /// an offline, cache-only run needs the address and never the password.
    pub fn hosted(server: &str, password: Option<String>) -> Result<Self> {
        let server = normalize_server(server)?;
        if let Some(password) = &password {
            validate_password(password)?;
        }
        Ok(Self {
            url: format!("{server}{RPC_PATH}"),
            hosted: Some(Hosted { server, password }),
        })
    }

    /// The request URL. Never contains the access password, so it is safe to
    /// hash into a cache namespace.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn is_hosted(&self) -> bool {
        self.hosted.is_some()
    }

    /// The Eplyx service origin, for the hosted RPC only.
    pub fn server(&self) -> Option<&str> {
        self.hosted.as_ref().map(|hosted| hosted.server.as_str())
    }

    /// A transport for this endpoint, authenticated for the hosted RPC.
    pub fn transport(&self) -> Result<HttpRpc> {
        let rpc = HttpRpc::new(self.url.clone())?;
        match &self.hosted {
            None => Ok(rpc),
            Some(Hosted { password: None, .. }) => bail!(NO_ACCESS),
            Some(Hosted {
                password: Some(password),
                ..
            }) => rpc
                .with_bearer(password)?
                .with_timeout(HOSTED_TIMEOUT_SECONDS),
        }
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// An explicit flag, then `env` in order, then the Eplyx hosted RPC.
pub fn resolve(explicit: Option<String>, env: &[&str]) -> Result<Endpoint> {
    match explicit.or_else(|| env.iter().find_map(|name| var(name))) {
        Some(url) => Ok(Endpoint::own(url)),
        None => hosted(None),
    }
}

/// The hosted RPC: `server`, else `EPLYX_URL`, else the saved server, else
/// eplyx.dev; with `EPLYX_ACCESS_PASSWORD`, else the password saved for that
/// same server.
pub fn hosted(server: Option<&str>) -> Result<Endpoint> {
    let saved = load()?;
    let server = match server.map(str::to_owned).or_else(|| var(SERVER_ENV)) {
        Some(server) => normalize_server(&server)?,
        None => saved
            .as_ref()
            .map_or_else(|| DEFAULT_SERVER.to_owned(), |saved| saved.server.clone()),
    };
    let password = var(PASSWORD_ENV).or_else(|| {
        saved
            .filter(|saved| saved.server == server)
            .map(|saved| saved.password)
    });
    Endpoint::hosted(&server, password)
}

/// Every access password this process could send, for leak scanning.
pub fn known_passwords() -> Vec<String> {
    let mut passwords: Vec<String> = var(PASSWORD_ENV).into_iter().collect();
    if let Ok(Some(saved)) = load() {
        passwords.push(saved.password);
    }
    passwords
}

pub fn path() -> Result<PathBuf> {
    Ok(config_dir()?.join(FILE))
}

fn load() -> Result<Option<Saved>> {
    let Some(bytes) = read_private(&path()?, "run `chmod 600` on it or `eplyx access --forget`")?
    else {
        return Ok(None);
    };
    let saved: Saved = serde_json::from_slice(&bytes)
        .context("invalid Eplyx access file; run `eplyx access --forget`")?;
    ensure!(saved.schema_version == 1, "unsupported Eplyx access file");
    normalize_server(&saved.server)?;
    validate_password(&saved.password)?;
    Ok(Some(saved))
}

/// Prove the endpoint answers for Solana mainnet with this password.
fn verify(endpoint: &Endpoint) -> Result<()> {
    let genesis = endpoint
        .transport()?
        .call("getGenesisHash", serde_json::json!([]))?;
    ensure!(
        genesis.as_str() == Some(MAINNET_GENESIS),
        "the Eplyx hosted RPC answered for a network other than Solana mainnet"
    );
    Ok(())
}

/// Read the password without echoing it where the terminal allows.
fn read_password(from_stdin: bool) -> Result<String> {
    let stdin = std::io::stdin();
    let interactive = !from_stdin && stdin.is_terminal();
    if interactive {
        eprint!("Eplyx access password: ");
        let _ = std::io::stderr().flush();
    }
    let hidden = interactive && echo(false);
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if hidden {
        echo(true);
        eprintln!();
    }
    read.context("reading the access password")?;
    let password = line.trim_end_matches(['\r', '\n']).to_owned();
    ensure!(!password.is_empty(), "no access password was entered");
    Ok(password)
}

/// Toggle terminal echo. `false` when it could not be changed.
fn echo(on: bool) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("stty")
            .arg(if on { "echo" } else { "-echo" })
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(not(unix))]
    {
        let _ = on;
        false
    }
}

/// `eplyx access`: verify a password against the hosted RPC and save it.
/// `--status` re-checks what is configured; `--forget` removes the saved one.
pub fn command(
    server: Option<&str>,
    password_stdin: bool,
    status: bool,
    forget: bool,
) -> Result<u8> {
    let file = path()?;
    if forget {
        match std::fs::remove_file(&file) {
            Ok(()) => println!("Removed the saved Eplyx access password."),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                println!("No Eplyx access password was saved.")
            }
            Err(error) => return Err(error.into()),
        }
        if var(PASSWORD_ENV).is_some() {
            println!("{PASSWORD_ENV} is still set in this environment.");
        }
        return Ok(0);
    }
    if status {
        let endpoint = hosted(server)?;
        let server = endpoint.server().unwrap_or(DEFAULT_SERVER).to_owned();
        if !endpoint
            .hosted
            .as_ref()
            .is_some_and(|h| h.password.is_some())
        {
            println!("Locked: no access password for {server}. Run `eplyx access` to unlock the hosted RPC.");
            return Ok(1);
        }
        return Ok(match verify(&endpoint) {
            Ok(()) => {
                println!(
                    "Unlocked: Solana mainnet reads go through the Eplyx hosted RPC at {server}."
                );
                0
            }
            Err(error) => {
                println!("Not usable: {error:#}");
                1
            }
        });
    }
    let server = match server.map(str::to_owned).or_else(|| var(SERVER_ENV)) {
        Some(server) => normalize_server(&server)?,
        None => DEFAULT_SERVER.to_owned(),
    };
    let password = read_password(password_stdin)?;
    validate_password(&password)?;
    let endpoint = Endpoint::hosted(&server, Some(password.clone()))?;
    verify(&endpoint)
        .with_context(|| format!("could not unlock the Eplyx hosted RPC at {server}"))?;
    write_private(
        &file,
        &serde_json::to_vec_pretty(&Saved {
            schema_version: 1,
            server: server.clone(),
            password,
        })?,
    )?;
    println!(
        "Access granted. Eplyx now reads Solana mainnet through its hosted RPC at {server};\n\
         no RPC provider or SOLANA_RPC_URL is needed. Saved to {} (owner-only).",
        file.display()
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_must_fit_a_bearer_header_unescaped() {
        for good in ["judges-2026", "Sol4na!Demo#", &"x".repeat(256)] {
            assert!(validate_password(good).is_ok(), "{good}");
        }
        for bad in [
            "short",
            "has space here",
            "quote\"inside",
            "back\\slash1",
            "new\nline12",
            "ünïcödé-pass",
            &"x".repeat(257),
        ] {
            assert!(validate_password(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_hosted_endpoint_never_formats_or_hashes_its_password() {
        let endpoint = Endpoint::hosted("https://Eplyx.dev/", Some("judges-2026".into())).unwrap();
        assert_eq!(endpoint.url(), "https://eplyx.dev/v1/rpc");
        assert_eq!(endpoint.server(), Some("https://eplyx.dev"));
        assert!(endpoint.is_hosted());
        assert!(!format!("{endpoint:?}").contains("judges-2026"));
        assert!(endpoint.transport().is_ok());
        let own = Endpoint::own("https://rpc.example/?api-key=secret");
        assert!(!own.is_hosted());
        assert!(!format!("{own:?}").contains("secret"));
    }

    #[test]
    fn a_locked_hosted_endpoint_resolves_but_cannot_send() {
        // Offline, cache-only runs need the address and never the password.
        let endpoint = Endpoint::hosted(DEFAULT_SERVER, None).unwrap();
        assert_eq!(endpoint.url(), "https://eplyx.dev/v1/rpc");
        let Err(error) = endpoint.transport() else {
            panic!("a locked endpoint sent a request");
        };
        let error = error.to_string();
        assert!(error.contains("eplyx access"), "{error}");
        assert!(Endpoint::hosted("http://eplyx.dev", Some("judges-2026".into())).is_err());
    }

    #[test]
    fn an_explicit_endpoint_wins_over_the_hosted_rpc() {
        let endpoint = resolve(
            Some("https://own.example/rpc".into()),
            &["EPLYX_TEST_UNSET_RPC_VARIABLE"],
        )
        .unwrap();
        assert!(!endpoint.is_hosted());
        assert_eq!(endpoint.url(), "https://own.example/rpc");
    }
}
