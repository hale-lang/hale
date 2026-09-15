//! Whatever a DNA test starts under its temporary directory stops when
//! the test does — passing, or unwinding from a failed assertion (#637).
//! Hosts, bodies, knowledge services and the tools they run all name a
//! path under that directory on their command line.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Kill every process whose command line names a path under `dir`.
pub fn reap(dir: &Path) {
    let needle = format!("{}/", dir.display());
    let me = std::process::id().to_string();
    let Ok(out) = Command::new("ps").args(["-eo", "pid=,args="]).output() else { return };
    let table = String::from_utf8_lossy(&out.stdout);
    let pids: Vec<String> = table
        .lines()
        .filter(|l| l.contains(&needle))
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .filter(|p| *p != me)
        .collect();
    if !pids.is_empty() {
        let _ = Command::new("kill").arg("-KILL").args(&pids).output();
    }
}

/// Reaps `dir` when dropped.
pub struct ReapOnDrop(pub PathBuf);

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        reap(&self.0);
    }
}
