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

/// The file under a root naming the compose projects found rooted in it,
/// one per line, kept until their teardown is confirmed — as
/// `dna::scratch_compose_down` keeps it: a volume carries its project's
/// name but not the directory it ran from.
pub const COMPOSE_FILE: &str = ".dna-compose-projects";

fn keep_scratch() -> bool {
    std::env::var("HALE_DNA_KEEP_SCRATCH").is_ok_and(|v| !v.is_empty() && v != "0")
}

enum Docker {
    Answered(String),
    // no CLI, or no daemon: nothing compose started can be running there
    Absent,
    Failed(String),
}

fn docker(args: &[&str]) -> Docker {
    match Command::new("docker").args(args).output() {
        Err(_) => Docker::Absent,
        Ok(o) if o.status.success() => Docker::Answered(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).into_owned();
            if err.contains("Cannot connect to the Docker daemon") || err.contains("Is the docker daemon running") {
                Docker::Absent
            } else {
                Docker::Failed(format!("docker {}: {}", args.join(" "), err.trim()))
            }
        }
    }
}

/// Take down every compose project whose working directory is `dir` or
/// under it — its containers, networks and volumes — as
/// `dna::scratch_done` does for a fixture's root ([`compose_down_all`]).
#[allow(dead_code)]
pub fn compose_down(dir: &Path) -> Result<(), String> {
    compose_down_all(&[dir.to_path_buf()])
}

/// Take down every compose project rooted in one of `roots`: a host with
/// no owner's DSN brings its knowledge database up from the project's
/// `dna/compose.yaml`, and nothing else takes it down. A project is found
/// by the `com.docker.compose.project.working_dir` label on any container,
/// running or not, or by name in the root's [`COMPOSE_FILE`]; `down` by
/// project name needs no compose file, and a teardown is confirmed by the
/// project's label on containers and on volumes. `Err` names what failed
/// or survived, and the projects stay named in the root's file.
/// `HALE_DNA_KEEP_SCRATCH` keeps everything, as it keeps the directory;
/// nothing is done where there is no docker to ask.
pub fn compose_down_all(roots: &[PathBuf]) -> Result<(), String> {
    if keep_scratch() || roots.is_empty() {
        return Ok(());
    }
    let listed = match docker(&[
        "ps",
        "-a",
        "--filter",
        "label=com.docker.compose.project",
        "--format",
        "{{.Label \"com.docker.compose.project\"}}|{{.Label \"com.docker.compose.project.working_dir\"}}",
    ]) {
        Docker::Absent => return Ok(()),
        Docker::Failed(why) => return Err(why),
        Docker::Answered(out) => out,
    };
    let mut failed = Vec::new();
    for root in roots {
        let text = root.display().to_string();
        let under = format!("{text}/");
        let mut projects: Vec<String> = listed
            .lines()
            .filter_map(|l| l.split_once('|'))
            .filter(|(p, wd)| !p.is_empty() && (*wd == text || wd.starts_with(&under)))
            .map(|(p, _)| p.to_string())
            .collect();
        let file = root.join(COMPOSE_FILE);
        projects.extend(std::fs::read_to_string(&file).unwrap_or_default().lines().filter(|l| !l.is_empty()).map(str::to_string));
        projects.sort();
        projects.dedup();
        if projects.is_empty() {
            continue;
        }
        let _ = std::fs::create_dir_all(root);
        let _ = std::fs::write(&file, projects.iter().map(|p| format!("{p}\n")).collect::<String>());
        let mut left = Vec::new();
        for project in &projects {
            if let Docker::Failed(why) = docker(&["compose", "-p", project, "down", "-v", "--remove-orphans"]) {
                left.push(why);
            }
            let label = format!("label=com.docker.compose.project={project}");
            for (what, args) in [("containers", vec!["ps", "-a", "-q", "--filter", &label]), ("volumes", vec!["volume", "ls", "-q", "--filter", &label])] {
                match docker(&args) {
                    Docker::Answered(out) if !out.trim().is_empty() => left.push(format!("compose project {project} still has {what} after its teardown")),
                    Docker::Failed(why) => left.push(why),
                    _ => {}
                }
            }
        }
        if left.is_empty() {
            let _ = std::fs::remove_file(&file);
        } else {
            failed.extend(left);
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
