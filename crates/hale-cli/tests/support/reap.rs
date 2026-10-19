//! Whatever a DNA test starts under its temporary directory stops when
//! the test does — passing, or unwinding from a failed assertion (#637).
//! Hosts, bodies, knowledge services and the tools they run all name a
//! path under that directory on their command line.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Kill every process whose command line names a path under `dir`, and
/// take down every compose project run from under it ([`compose_down`]).
/// A teardown that fails fails the test (or, already unwinding, says so),
/// and the projects stay named under `dir` for the next attempt.
pub fn reap(dir: &Path) {
    kill_under(dir);
    if let Err(why) = compose_down(dir) {
        failed_teardown(dir, &why);
    }
}

/// Fail loudly on a teardown that did not finish: a panic, unless the
/// test is already unwinding from one, when it is said on stderr instead.
pub fn failed_teardown(dir: &Path, why: &str) {
    let said = format!("the compose projects rooted in {} did not come down; it is kept for another attempt:\n{why}", dir.display());
    if std::thread::panicking() {
        eprintln!("{said}");
    } else {
        panic!("{said}");
    }
}

fn keep_scratch() -> bool {
    std::env::var("HALE_DNA_KEEP_SCRATCH").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Take down every compose project whose working directory is `dir` or
/// under it — its containers, networks and volumes — as
/// `dna::scratch_done` does for a fixture's root ([`compose_down_all`]).
#[allow(dead_code)]
pub fn compose_down(dir: &Path) -> Result<(), String> {
    compose_down_all(&[dir.to_path_buf()])
}

/// Take down every compose project rooted in one of `roots`: a verb with
/// no owner's DSN brings the organism's memory up as an allocation of the
/// local habitat, which a test keeps under its root, and nothing else takes
/// it down. GH #1653: through `hale habitat sweep <root>`, the one teardown
/// `hale test` and `scripts/compose-down-under.sh` share — the projects are
/// named under the root (`.dna-compose-projects`) until they are confirmed
/// gone, by label on containers and on volumes, and the developer's habitat
/// is never one of them. `Err` names what failed or survived, and the
/// projects stay named in the root's file. `HALE_DNA_KEEP_SCRATCH` keeps
/// everything, as it keeps the directory.
pub fn compose_down_all(roots: &[PathBuf]) -> Result<(), String> {
    if keep_scratch() {
        return Ok(());
    }
    let mut failed = Vec::new();
    for root in roots {
        match Command::new(env!("CARGO_BIN_EXE_hale")).args(["habitat", "sweep"]).arg(root).output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => failed.push(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => failed.push(format!("hale habitat sweep {}: {e}", root.display())),
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("\n"))
    }
}

/// Kill every process whose command line names a path under `dir`.
pub fn kill_under(dir: &Path) {
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
