//! Whatever a DNA test starts under its temporary directory stops when
//! the test does — passing, or unwinding from a failed assertion (#637).
//! Hosts, bodies, knowledge services and the tools they run all name a
//! path under that directory on their command line.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Kill every process whose command line names a path under `dir`, and
/// take down every compose project run from under it ([`compose_down`]).
pub fn reap(dir: &Path) {
    compose_down(dir);
    kill_under(dir);
}

/// Take down every compose project whose working directory is `dir` or
/// under it — its containers, networks and volumes — as
/// `dna::scratch_done` does for a fixture's root: a host with no owner's
/// DSN brings its knowledge database up from the project's
/// `dna/compose.yaml`, and nothing else takes it down. Found by the
/// `com.docker.compose.project.working_dir` label on any container,
/// running or not; `down` by project name needs no compose file.
/// `HALE_DNA_KEEP_SCRATCH` keeps them, as it keeps the directory. Nothing
/// is done where docker is not there to ask.
#[allow(dead_code)]
pub fn compose_down(dir: &Path) {
    if std::env::var("HALE_DNA_KEEP_SCRATCH").is_ok_and(|v| !v.is_empty() && v != "0") {
        return;
    }
    let root = dir.display().to_string();
    let under = format!("{root}/");
    let Ok(out) = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            "label=com.docker.compose.project",
            "--format",
            "{{.Label \"com.docker.compose.project\"}}|{{.Label \"com.docker.compose.project.working_dir\"}}",
        ])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let mut projects: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('|'))
        .filter(|(p, wd)| !p.is_empty() && (*wd == root || wd.starts_with(&under)))
        .map(|(p, _)| p.to_string())
        .collect();
    projects.sort();
    projects.dedup();
    for project in projects {
        let _ = Command::new("docker").args(["compose", "-p", &project, "down", "-v", "--remove-orphans"]).output();
    }
}

fn kill_under(dir: &Path) {
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
