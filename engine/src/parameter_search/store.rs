//! Portable private local CAS. Exact evidence is deduplicated, never sanitized
//! by rewriting it. Only the separate public summary passes privacy scanning.
use super::{Input, Report, SCHEMA};
use crate::{
    canonical,
    lifecycle::artifact,
    universal::evidence::{EvidenceKind, EvidenceRef, EvidenceStore},
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path};
const MAX_TOTAL: usize = 512 * 1024 * 1024;
fn real_path(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        #[cfg(target_os = "macos")]
        if (p == Path::new("/var") && p.canonicalize()? == Path::new("/private/var"))
            || (p == Path::new("/tmp") && p.canonicalize()? == Path::new("/private/tmp"))
        {
            continue;
        }
        ensure!(!p.is_symlink(), "symlinked search artifact path");
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
pub fn begin(root: &Path, input: &Input) -> Result<()> {
    real_path(root)?;
    ensure!(!root.exists(), "search output must be a new directory");
    // Validate the entire parent/domain before creating an output or executing.
    super::input_identity(input)?;
    fs::create_dir(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    }
    write_new(&root.join("incomplete.json"), canonical::document(&json!({"schema":SCHEMA,"completion_status":"incomplete","reason":"completed manifest not yet published; interrupted runs remain incomplete"}))?.as_bytes())
}
// Recursively externalize large subtrees; repeated fixture/account/base64 bytes
// become one content object even across many original/derived paired reports.
fn externalize(store: &EvidenceStore, v: &mut Value) -> Result<()> {
    match v {
        Value::Object(map) => {
            for child in map.values_mut() {
                externalize(store, child)?;
            }
        }
        Value::Array(items) if items.iter().all(|v| !v.is_array() && !v.is_object()) => (),
        Value::Array(items) => {
            for child in items {
                externalize(store, child)?;
            }
        }
        _ => (),
    }
    let bytes = canonical::document(v)?;
    if bytes.len() >= 1024 {
        ensure!(
            bytes.len() <= artifact::MAX_BYTES as usize,
            "CAS object exceeds bound"
        );
        let reference = store.put(EvidenceKind::Execution, bytes.as_bytes())?;
        *v = json!({"parameter_search_cas":reference});
    }
    Ok(())
}
fn hydrate(store: &EvidenceStore, v: &mut Value, depth: usize, total: &mut usize) -> Result<()> {
    ensure!(depth <= 96, "search CAS nesting exceeds bound");
    if let Some(map) = v.as_object() {
        if let Some(value) = map.get("parameter_search_cas") {
            ensure!(map.len() == 1, "invalid search CAS reference");
            let r: EvidenceRef = serde_json::from_value(value.clone())?;
            ensure!(
                r.kind == EvidenceKind::Execution,
                "wrong search CAS evidence kind"
            );
            let path = store.path(&r)?;
            real_path(&path)?;
            let bytes = artifact::read(path)?;
            ensure!(
                crate::replay::hash_bytes(&bytes) == r.sha256,
                "search CAS content mismatch"
            );
            *total = total
                .checked_add(bytes.len())
                .context("artifact size overflow")?;
            ensure!(
                *total <= MAX_TOTAL,
                "hydrated search exceeds aggregate byte bound"
            );
            *v = serde_json::from_slice(&bytes)?;
            return hydrate(store, v, depth + 1, total);
        }
    }
    match v {
        Value::Object(map) => {
            for child in map.values_mut() {
                hydrate(store, child, depth + 1, total)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                hydrate(store, child, depth + 1, total)?;
            }
        }
        _ => (),
    }
    Ok(())
}
pub fn save(root: &Path, report: &Report) -> Result<()> {
    real_path(root)?;
    ensure!(
        root.join("incomplete.json").is_file() && !root.join("manifest.json").exists(),
        "missing fresh incomplete artifact"
    );
    super::verify(report)?;
    ensure!(
        canonical::document(report)?.len() <= MAX_TOTAL,
        "search report exceeds aggregate byte bound"
    );
    let store = EvidenceStore::at(root.join("evidence"));
    let mut value = serde_json::to_value(report)?;
    externalize(&store, &mut value)?;
    write_new(
        &root.join("summary.json"),
        canonical::document(&super::receipt(report)?)?.as_bytes(),
    )?;
    write_new(&root.join("report.md"), super::render(report)?.as_bytes())?;
    // Atomic completion marker; no interrupted partial manifest can look complete.
    write_new(
        &root.join("manifest.pending"),
        canonical::document(&json!({"schema":SCHEMA,"completion_status":"complete",
        "report_sha256":report.report_sha256,"report":value}))?
        .as_bytes(),
    )?;
    fs::rename(root.join("manifest.pending"), root.join("manifest.json"))?;
    Ok(())
}
pub fn load(root: &Path) -> Result<Report> {
    real_path(root)?;
    ensure!(
        root.join("manifest.json").is_file(),
        "incomplete search artifact: completed manifest absent"
    );
    let manifest: Value = serde_json::from_slice(&artifact::read(root.join("manifest.json"))?)?;
    ensure!(
        manifest["schema"] == SCHEMA
            && manifest["completion_status"] == "complete"
            && manifest.as_object().is_some_and(|m| m.len() == 4),
        "invalid completed search manifest"
    );
    let mut value = manifest["report"].clone();
    hydrate(
        &EvidenceStore::at(root.join("evidence")),
        &mut value,
        0,
        &mut 0,
    )?;
    let report: Report = serde_json::from_value(value)?;
    ensure!(
        manifest["report_sha256"] == report.report_sha256,
        "search manifest report binding differs"
    );
    super::verify(&report)?;
    ensure!(
        artifact::read(root.join("summary.json"))?
            == canonical::document(&super::receipt(&report)?)?.as_bytes(),
        "public search summary differs from verified report"
    );
    ensure!(
        artifact::read(root.join("report.md"))? == super::render(&report)?.as_bytes(),
        "readable search report differs from verified report"
    );
    Ok(report)
}
