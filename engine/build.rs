//! Build identity for `eplyx --version`: semantic version comes from Cargo;
//! this adds only the source commit and target triple. No timestamp is
//! embedded, so identical sources produce identical version output.
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=EPLYX_BUILD_COMMIT");
    // A new commit changes HEAD or the checked-out branch ref.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    if let Ok(head) = std::fs::read_to_string("../.git/HEAD") {
        if let Some(reference) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=../.git/{reference}");
        }
    }
    let commit = std::env::var("EPLYX_BUILD_COMMIT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            let output = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .filter(|value| value.len() >= 7 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=EPLYX_BUILD_COMMIT={commit}");
    println!(
        "cargo:rustc-env=EPLYX_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".into())
    );
}
