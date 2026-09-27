//! The scoped Eplyx access token, stored per server in the user's config
//! directory with owner-only permissions. It is read only by `eplyx login`,
//! `logout`, `link` and `sync`; analysis commands never open this file, and
//! no Solana key, RPC credential or third-party token is ever stored here.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const FILE: &str = "credentials.json";
const LIMIT: u64 = 64 * 1024;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub schema_version: u32,
    #[serde(default)]
    pub default_server: Option<String>,
    #[serde(default)]
    pub servers: BTreeMap<String, Entry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub token: String,
    pub user_email: String,
    pub created_at: String,
}

/// `EPLYX_CONFIG_DIR`, else `%APPDATA%\eplyx` on Windows, else
/// `$XDG_CONFIG_HOME/eplyx`, else `~/.config/eplyx`.
pub fn config_dir() -> Result<PathBuf> {
    let from = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = from("EPLYX_CONFIG_DIR") {
        return Ok(dir);
    }
    if cfg!(windows) {
        if let Some(dir) = from("APPDATA") {
            return Ok(dir.join("eplyx"));
        }
    }
    if let Some(dir) = from("XDG_CONFIG_HOME") {
        return Ok(dir.join("eplyx"));
    }
    let home = from("HOME")
        .or_else(|| from("USERPROFILE"))
        .context("cannot find a home directory for Eplyx credentials; set EPLYX_CONFIG_DIR")?;
    Ok(home.join(".config").join("eplyx"))
}

pub fn path() -> Result<PathBuf> {
    Ok(config_dir()?.join(FILE))
}

pub fn load() -> Result<Credentials> {
    load_from(&path()?)
}

fn guard_parent(file: &Path) -> Result<()> {
    let parent = file.parent().context("credentials path has no parent")?;
    // Ancestors such as the platform temporary root may be symlinks, but the
    // credential directory itself must never redirect a write to another target.
    if let Ok(meta) = fs::symlink_metadata(parent) {
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "credentials directory must be a real directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                meta.permissions().mode() & 0o022 == 0,
                "credentials directory is writable by other users"
            );
        }
    }
    Ok(())
}
pub fn load_from(file: &Path) -> Result<Credentials> {
    guard_parent(file)?;
    let meta = match fs::symlink_metadata(file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Credentials {
                schema_version: 1,
                ..Credentials::default()
            })
        }
        Err(error) => return Err(error.into()),
        Ok(meta) => meta,
    };
    ensure!(
        !meta.file_type().is_symlink() && meta.is_file(),
        "{} must be a regular file",
        file.display()
    );
    ensure!(meta.len() <= LIMIT, "credentials file is too large");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "{} is readable by other users; run `chmod 600` on it or `eplyx logout`",
            file.display()
        );
    }
    let mut bytes = Vec::new();
    fs::File::open(file)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "credentials file is too large");
    let credentials: Credentials =
        serde_json::from_slice(&bytes).context("invalid Eplyx credentials file")?;
    ensure!(
        credentials.schema_version == 1,
        "unsupported credentials file"
    );
    Ok(credentials)
}

pub fn save(credentials: &Credentials) -> Result<PathBuf> {
    let file = path()?;
    save_to(&file, credentials)?;
    Ok(file)
}

pub fn save_to(file: &Path, credentials: &Credentials) -> Result<()> {
    guard_parent(file)?;
    let dir = file.parent().context("credentials path has no parent")?;
    if !dir.exists() {
        fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    if let Ok(meta) = fs::symlink_metadata(file) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            bail!("refusing to replace {}", file.display());
        }
    }
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(&serde_json::to_vec_pretty(credentials)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(file).map_err(|e| e.error)?;
    if let Ok(parent) = fs::File::open(dir) {
        let _ = parent.sync_all();
    }
    Ok(())
}

/// Reduce a server URL to `scheme://host[:port]`. HTTPS is required except on
/// loopback, so a token is never sent in clear text to another host.
pub fn normalize_server(url: &str) -> Result<String> {
    let url = url.trim().trim_end_matches('/');
    let (scheme, rest) = url
        .split_once("://")
        .context("the Eplyx cloud URL must start with https://")?;
    ensure!(
        !rest.is_empty()
            && !rest.contains(['/', '?', '#', '@', ' ', '\\'])
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']')),
        "the Eplyx cloud URL must be an origin such as https://cloud.example"
    );
    let host = rest
        .rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(rest, |(host, _)| host);
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]");
    match scheme {
        "https" => {}
        "http" if loopback => {}
        _ => bail!("the Eplyx cloud URL must use https:// (plain http only on loopback)"),
    }
    Ok(format!("{scheme}://{}", rest.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls_are_origins_and_https_only_off_loopback() {
        assert_eq!(
            normalize_server("https://Cloud.Example/").unwrap(),
            "https://cloud.example"
        );
        assert_eq!(
            normalize_server("http://127.0.0.1:4300").unwrap(),
            "http://127.0.0.1:4300"
        );
        for bad in [
            "http://cloud.example",
            "https://cloud.example/api",
            "https://user:pw@cloud.example",
            "ftp://cloud.example",
            "cloud.example",
            "https://cloud.example?x=1",
        ] {
            assert!(normalize_server(bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn credentials_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("eplyx-cred-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join(FILE);
        let mut credentials = Credentials {
            schema_version: 1,
            ..Credentials::default()
        };
        credentials.servers.insert(
            "https://cloud.example".into(),
            Entry {
                token: "eplyx_u_test".into(),
                user_email: "dev@example.com".into(),
                created_at: "2026-09-25T00:00:00Z".into(),
            },
        );
        save_to(&file, &credentials).unwrap();
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(load_from(&file).unwrap().servers.len(), 1);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            load_from(&file).is_err(),
            "group/other readable file is refused"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
