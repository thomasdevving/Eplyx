//! Shared bounded wait for the two offline worker entry points.
use anyhow::{ensure, Result};
use std::{
    process::Child,
    time::{Duration, Instant},
};
pub(super) fn wait(child: &mut Child, limit: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "offline worker failed");
            return Ok(());
        }
        if start.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("offline worker exceeded time bound");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    #[test]
    fn actual_timeout_reaps_child_and_nonzero_status_is_not_evidence() {
        let mut child = Command::new("/bin/sleep")
            .arg("30")
            .env_clear()
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        assert!(wait(&mut child, Duration::from_millis(10))
            .unwrap_err()
            .to_string()
            .contains("time bound"));
        assert!(child.try_wait().unwrap().is_some());
        let mut failed = Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .env_clear()
            .spawn()
            .unwrap();
        assert!(wait(&mut failed, Duration::from_secs(1)).is_err());
        let mut passed = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .env_clear()
            .spawn()
            .unwrap();
        wait(&mut passed, Duration::from_secs(1)).unwrap();
    }
}
