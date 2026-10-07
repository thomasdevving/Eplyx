//! Publish complete immutable bytes without exposing a partial file or
//! overwriting a concurrent writer. Shared by corpus records and evidence CAS.
use anyhow::{ensure, Context, Result};
use std::{fs, io::Write, path::Path};

/// True when created; false when identical bytes were already present.
pub(crate) fn write_once(path: &Path, bytes: &[u8]) -> Result<bool> {
    let parent = path.parent().context("immutable file has no parent")?;
    fs::create_dir_all(parent)?;
    let same_bytes = || -> Result<bool> {
        let stored = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        ensure!(
            stored == bytes,
            "immutable content differs at {}",
            path.display()
        );
        Ok(false)
    };
    if path.try_exists()? {
        return same_bytes();
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(path) {
        Ok(_) => {
            if let Ok(directory) = fs::File::open(parent) {
                let _ = directory.sync_all();
            }
            Ok(true)
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => same_bytes(),
        Err(error) => Err(error.error).with_context(|| format!("publishing {}", path.display())),
    }
}
