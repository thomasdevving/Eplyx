//! Independent scratch copies: tests never mutate frozen reference bytes.
use std::path::{Path, PathBuf};
pub fn package(name: &str) -> PathBuf {
    let target =
        std::env::temp_dir().join(format!("eplyx-lifecycle-{name}-{}", std::process::id()));
    std::fs::create_dir(&target).unwrap();
    fn copy(source: &Path, target: &Path) {
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            if name == ".gitignore" {
                continue;
            }
            let destination = target.join(&name);
            if entry.file_type().unwrap().is_dir() {
                std::fs::create_dir(&destination).unwrap();
                copy(&entry.path(), &destination)
            } else {
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }
    copy(
        &eplyx_engine::lifecycle::artifact::reference_root(),
        &target,
    );
    target
}
