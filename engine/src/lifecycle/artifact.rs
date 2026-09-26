//! Bounded local artifact reads. A package marker permits sibling references
//! inside one explicit root; references cannot escape it or follow symlinks.
use anyhow::{ensure, Context, Result};
use serde::de::DeserializeOwned;
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const MAX_BYTES: u64 = 128 * 1024 * 1024;

/// Local pinned reference package used by regression tests; never a hosted input.
pub fn reference_root() -> PathBuf {
    crate::repo_root().join("fixtures/lifecycle/main-t7")
}

pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let metadata = std::fs::symlink_metadata(path).context("missing lifecycle artifact")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "lifecycle artifact must be a regular file"
    );
    ensure!(
        metadata.len() <= MAX_BYTES,
        "lifecycle artifact exceeds byte bound"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "lifecycle artifact exceeds byte bound"
    );
    Ok(bytes)
}

pub fn member(base: &Path, relative: &str) -> Result<PathBuf> {
    let name = Path::new(relative);
    ensure!(
        !name.is_absolute() && !relative.is_empty(),
        "artifact reference must be relative"
    );
    let base = base.canonicalize()?;
    let root = base
        .ancestors()
        .find(|p| p.join(".lifecycle-root").is_file())
        .unwrap_or(&base);
    let mut path = base.clone();
    for part in name.components() {
        match part {
            Component::Normal(p) => {
                path.push(p);
                ensure!(
                    !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
                    "artifact reference traverses a symlink"
                );
            }
            Component::CurDir => (),
            Component::ParentDir => {
                ensure!(path != root, "artifact reference escapes package");
                path.pop();
            }
            _ => anyhow::bail!("invalid artifact reference"),
        }
        ensure!(path.starts_with(root), "artifact reference escapes package");
    }
    ensure!(path.is_file(), "missing lifecycle artifact");
    Ok(path)
}

pub fn read_relative(base: &Path, relative: &str) -> Result<Vec<u8>> {
    read(&member(base, relative)?)
}

pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&read(path)?)?)
}

/// Reports carry a package-relative reference, never a machine path.
pub fn display_path(path: &Path) -> String {
    if !path.is_absolute() {
        return path.to_string_lossy().replace('\\', "/");
    }
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = absolute
        .ancestors()
        .find(|p| p.join(".lifecycle-root").is_file());
    root.and_then(|r| absolute.strip_prefix(r).ok())
        .unwrap_or_else(|| {
            path.file_name()
                .map(Path::new)
                .unwrap_or_else(|| Path::new("artifact"))
        })
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_members_are_bounded_and_cannot_follow_links_or_escape() {
        let dir =
            std::env::temp_dir().join(format!("eplyx-lifecycle-artifact-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join(".lifecycle-root"), b"test package").unwrap();
        let nested = dir.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(dir.join("value.json"), b"{}").unwrap();
        assert_eq!(read_relative(&nested, "../value.json").unwrap(), b"{}");
        assert!(member(&nested, "../../value.json").is_err());
        assert!(member(&nested, dir.join("value.json").to_str().unwrap()).is_err());
        let large = dir.join("too-large");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(read(&large).unwrap_err().to_string().contains("byte bound"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("value.json"), nested.join("linked")).unwrap();
            assert!(member(&nested, "linked").is_err());
            assert!(read(nested.join("linked")).is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
