//! The hosted service's durable, content-addressed program store.
//!
//! ```text
//! /data/artifacts
//!   programs/<sha256>        immutable executable bytes, addressed by their hash
//!   tmp/<random>             in-flight writes; never addressable; swept at startup
//!   quarantine/<sha256>.<n>  an object found not to match its address, kept aside
//! ```
//!
//! The layout is the engine's evidence-store layout (`programs/<sha256>`), so
//! the same object can be named by `EvidenceRef` and resolved by
//! `CandidateSource::Store` if a caller wants to. What this module adds is the
//! part a long-lived multi-run store needs and a per-run scratch store did not:
//!
//! - **Atomic creation.** Bytes are written to `tmp/`, synced, re-read and
//!   verified, and only then hard-linked into `programs/`. A link either appears
//!   whole or not at all, so a crash mid-write can never leave a partial object
//!   under a valid address. Linking (rather than renaming) fails if the address
//!   already exists, which is what makes the store write-once.
//! - **Deduplication.** Identical bytes have one address and one object; a second
//!   upload of them verifies the existing object and reuses it.
//! - **Hash-on-read.** Every read checks existence, length and SHA-256 before a
//!   byte is handed out. An object whose name looks like its hash is not thereby
//!   trusted.
//! - **No caller-controlled paths.** A path is only ever built from a validated
//!   canonical hash: 64 lowercase hex characters, nothing else.
//!
//! Nothing here deletes a program. Retention is "every artefact is kept": the
//! only deletion paths in the service are a run's scratch work directory and
//! stale temporary files, neither of which is addressable.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use eplyx_engine::change::ExecutableArtifact;
use eplyx_engine::replay::hash_bytes;

/// A stored executable, named by content. The same shape as the change spec's
/// `candidate`, so "the registry's artefact equals the spec's candidate" is a
/// plain equality rather than a translation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub sha256: String,
    pub len: u64,
}

impl ArtifactRef {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            sha256: hash_bytes(bytes),
            len: bytes.len() as u64,
        }
    }

    pub fn matches(&self, artifact: &ExecutableArtifact) -> bool {
        self.sha256 == artifact.sha256 && self.len == artifact.len
    }
}

impl From<&ExecutableArtifact> for ArtifactRef {
    fn from(artifact: &ExecutableArtifact) -> Self {
        Self {
            sha256: artifact.sha256.clone(),
            len: artifact.len,
        }
    }
}

/// A content hash as this store accepts it: exactly the spelling `hash_bytes`
/// produces. Anything else is refused before it becomes a path segment, so a
/// hash string read from a change spec can never name a different file.
pub fn canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// What a `put` did. `created` is false when identical bytes were already held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub reference: ArtifactRef,
    pub created: bool,
}

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    root: PathBuf,
    max_program_bytes: usize,
}

impl ArtifactStore {
    pub fn open(root: impl Into<PathBuf>, max_program_bytes: usize) -> Result<Self> {
        let root = root.into();
        for directory in ["programs", "tmp", "quarantine"] {
            fs::create_dir_all(root.join(directory))
                .with_context(|| format!("creating {}", root.join(directory).display()))?;
        }
        Ok(Self {
            root,
            max_program_bytes,
        })
    }

    /// The directory in the engine's evidence layout, for `CandidateSource::Store`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn program_path(&self, sha256: &str) -> Result<PathBuf> {
        ensure!(
            canonical_sha256(sha256),
            "artifact hash {sha256:?} is not a canonical sha256"
        );
        Ok(self.root.join("programs").join(sha256))
    }

    /// Store an executable, or verify the identical one already held.
    ///
    /// ```text
    /// tmp/<random>  write → fsync → re-read + verify
    ///   → hard link to programs/<sha256>   (fails if it exists: write-once)
    ///   → fsync programs/                  (the link is durable)
    ///   → remove tmp/<random>
    /// ```
    ///
    /// Two concurrent uploads of the same bytes race only on the link: one
    /// creates it, the other finds it and verifies it. Neither can observe a
    /// partial object, because none is ever addressable.
    pub fn put_program(&self, bytes: &[u8]) -> Result<Stored> {
        ensure!(!bytes.is_empty(), "an empty program is not an executable");
        ensure!(
            bytes.len() <= self.max_program_bytes,
            "program exceeds the {} byte artifact limit",
            self.max_program_bytes
        );
        let reference = ArtifactRef::of(bytes);
        let destination = self.program_path(&reference.sha256)?;
        if destination.exists() {
            match self.get_program(&reference) {
                Ok(_) => {
                    return Ok(Stored {
                        reference,
                        created: false,
                    })
                }
                // An object that does not match its own address is damage, not
                // content. It is moved aside rather than trusted or silently
                // overwritten, and the verified bytes take its address.
                Err(_) => self.quarantine(&destination, &reference.sha256)?,
            }
        }

        let temporary = self.root.join("tmp").join(format!(
            "{}-{:016x}",
            &reference.sha256[..16],
            rand::random::<u64>()
        ));
        let result = self.promote(&temporary, &destination, bytes, &reference);
        fs::remove_file(&temporary).ok();
        result
    }

    fn promote(
        &self,
        temporary: &Path,
        destination: &Path,
        bytes: &[u8],
        reference: &ArtifactRef,
    ) -> Result<Stored> {
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temporary)
                .with_context(|| format!("creating {}", temporary.display()))?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        // Verified from disk, not from memory: the object that becomes
        // addressable is the one that was actually written.
        let written = fs::read(temporary)?;
        ensure!(
            ArtifactRef::of(&written) == *reference,
            "the written artifact does not verify"
        );
        match fs::hard_link(temporary, destination) {
            Ok(()) => {
                sync_directory(destination.parent().expect("programs directory"));
                Ok(Stored {
                    reference: reference.clone(),
                    created: true,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // Another upload of the same bytes won the race.
                self.get_program(reference)
                    .context("an identical artifact appeared concurrently but does not verify")?;
                Ok(Stored {
                    reference: reference.clone(),
                    created: false,
                })
            }
            Err(error) => Err(error).context("promoting the artifact"),
        }
    }

    fn quarantine(&self, path: &Path, sha256: &str) -> Result<()> {
        let aside = self
            .root
            .join("quarantine")
            .join(format!("{sha256}.{}", crate::registry::now_unix_seconds()));
        fs::rename(path, &aside)
            .with_context(|| format!("quarantining the damaged artifact {sha256}"))?;
        eprintln!("artifact {sha256} did not match its address; moved to quarantine");
        Ok(())
    }

    /// Read an executable, proving it is the one named.
    ///
    /// Existence, then length from metadata (so a truncated or bloated object is
    /// refused before it is read), then the SHA-256 of the bytes actually read.
    pub fn get_program(&self, reference: &ArtifactRef) -> Result<Vec<u8>> {
        let path = self.program_path(&reference.sha256)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                bail!("artifact {} is not held by this service", reference.sha256)
            }
            Err(error) => return Err(error).context("reading artifact metadata"),
        };
        ensure!(
            metadata.is_file() && metadata.len() == reference.len,
            "artifact {} is {} bytes on disk, not the {} it is named with",
            reference.sha256,
            metadata.len(),
            reference.len
        );
        let bytes = fs::read(&path).context("reading the artifact")?;
        let actual = ArtifactRef::of(&bytes);
        ensure!(
            actual == *reference,
            "artifact {} does not match its address (read {} / {} bytes)",
            reference.sha256,
            actual.sha256,
            actual.len
        );
        Ok(bytes)
    }

    /// The verified identity of a held artifact, or nothing. Never a path.
    pub fn describe(&self, sha256: &str) -> Result<Option<ArtifactRef>> {
        let path = self.program_path(sha256)?;
        let Ok(metadata) = fs::metadata(&path) else {
            return Ok(None);
        };
        let reference = ArtifactRef {
            sha256: sha256.to_string(),
            len: metadata.len(),
        };
        self.get_program(&reference)?;
        Ok(Some(reference))
    }

    /// Remove in-flight writes a previous process left behind.
    ///
    /// Only safe while no write can be in progress, which is why it runs at
    /// startup, under the data directory's exclusive lock. `tmp/` is never
    /// addressable, so nothing removed here was ever an artefact.
    pub fn sweep_temporary(&self) -> Result<usize> {
        let mut removed = 0;
        for entry in fs::read_dir(self.root.join("tmp"))?.flatten() {
            if fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// Make a directory entry durable. Best effort: not every platform can open a
/// directory for syncing, and the file contents themselves are already synced.
pub fn sync_directory(directory: &Path) {
    if let Ok(handle) = fs::File::open(directory) {
        handle.sync_all().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ArtifactStore) {
        let scratch = tempfile::tempdir().unwrap();
        let store = ArtifactStore::open(scratch.path().join("artifacts"), 1024).unwrap();
        (scratch, store)
    }

    #[test]
    fn identical_bytes_are_one_artifact() {
        let (_scratch, store) = store();
        let first = store.put_program(b"program one").unwrap();
        let second = store.put_program(b"program one").unwrap();
        assert!(first.created);
        assert!(!second.created, "identical bytes were stored twice");
        assert_eq!(first.reference, second.reference);
        let other = store.put_program(b"program two").unwrap();
        assert_ne!(other.reference.sha256, first.reference.sha256);
        assert_eq!(
            fs::read_dir(store.root().join("programs")).unwrap().count(),
            2
        );
    }

    #[test]
    fn a_read_proves_existence_length_and_hash() {
        let (_scratch, store) = store();
        let stored = store.put_program(b"program").unwrap().reference;
        assert_eq!(store.get_program(&stored).unwrap(), b"program");

        let path = store.program_path(&stored.sha256).unwrap();
        // Same length, different bytes.
        fs::write(&path, b"prograM").unwrap();
        assert!(store.get_program(&stored).is_err());
        // Different length.
        fs::write(&path, b"program!").unwrap();
        assert!(store.get_program(&stored).is_err());
        fs::remove_file(&path).unwrap();
        let missing = store.get_program(&stored).unwrap_err();
        assert!(format!("{missing:#}").contains("not held"));
        // A wrong length in the reference is refused even over good bytes.
        store.put_program(b"program").unwrap();
        let mut lying = stored.clone();
        lying.len += 1;
        assert!(store.get_program(&lying).is_err());
    }

    #[test]
    fn a_damaged_object_is_quarantined_and_replaced_by_verified_bytes() {
        let (_scratch, store) = store();
        let stored = store.put_program(b"program").unwrap().reference;
        let path = store.program_path(&stored.sha256).unwrap();
        fs::write(&path, b"damaged").unwrap();
        let again = store.put_program(b"program").unwrap();
        assert!(again.created);
        assert_eq!(store.get_program(&stored).unwrap(), b"program");
        assert_eq!(
            fs::read_dir(store.root().join("quarantine"))
                .unwrap()
                .count(),
            1
        );
    }

    /// A write that never completed is not an artefact, and never becomes one.
    #[test]
    fn a_partial_write_is_never_addressable() {
        let (_scratch, store) = store();
        let bytes = b"complete program bytes";
        let reference = ArtifactRef::of(bytes);
        // What a crash mid-write leaves: a truncated file in tmp/.
        let partial = store.root().join("tmp").join("crashed-write");
        fs::write(&partial, &bytes[..5]).unwrap();
        assert!(store.get_program(&reference).is_err());
        assert_eq!(store.describe(&reference.sha256).unwrap(), None);
        assert_eq!(store.sweep_temporary().unwrap(), 1);
        assert!(!partial.exists());
        // And a completed write leaves nothing behind in tmp/.
        store.put_program(bytes).unwrap();
        assert_eq!(fs::read_dir(store.root().join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn concurrent_identical_uploads_agree() {
        let (_scratch, store) = store();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || store.put_program(b"shared program").unwrap())
            })
            .collect();
        let results: Vec<Stored> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|s| s.created).count(), 1);
        assert!(results.iter().all(|s| s.reference == results[0].reference));
        assert_eq!(
            store.get_program(&results[0].reference).unwrap(),
            b"shared program"
        );
        assert_eq!(fs::read_dir(store.root().join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn only_canonical_hashes_become_paths() {
        let (_scratch, store) = store();
        for bad in [
            "../../etc/passwd",
            &"A".repeat(64),
            &"a".repeat(63),
            &format!("{}/", "a".repeat(63)),
            "",
        ] {
            assert!(store.program_path(bad).is_err(), "accepted {bad:?}");
            assert!(store.describe(bad).is_err());
        }
    }

    #[test]
    fn limits_are_enforced() {
        let (_scratch, store) = store();
        assert!(store.put_program(b"").is_err());
        assert!(store.put_program(&[0_u8; 1025]).is_err());
        assert!(store.put_program(&[0_u8; 1024]).is_ok());
    }
}
