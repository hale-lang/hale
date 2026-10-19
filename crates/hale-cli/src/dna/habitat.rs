//! `hale habitat` — the acts of the machine's habitat (GH #1653).
//!
//! The local habitat is long-lived: one compose project per machine,
//! named for the habitat, holding a service per allocation — an
//! organism's memory, nerves and senses' store, named for the organism's
//! record identity and the role. Organisms are its tenants, reused from
//! run to run, and no command that works on an organism (`dev`, `run`,
//! `head`, `show`, `work`, the host's verbs) provisions or migrates
//! anything as a side effect: each reports what is missing and names the
//! act that supplies it. This module holds those acts. `migrate` brings
//! the organism's allocations up and moves memory's schema and the
//! nerves' stream to this toolchain's; `up`, `status` and `down` follow
//! with the habitat's genome.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use super::{host_run, memory_migrate, nerves_migrate, project, MemoryPlan, NervesPlan};

/// The verb's help: stdout and exit 0 for `--help`, stderr and exit 2
/// for a usage error (GH #817).
pub(crate) fn usage_text() -> &'static str {
    r#"usage: hale habitat migrate [organism]
       hale habitat sweep <root>

The local habitat holds every organism's infrastructure on this machine:
one docker compose project, named for the habitat, with a service per
allocation (memory-<id>, nerves-<id>, senses-<id>, <id> the organism's
record identity), under ${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local.
It is long-lived: nothing tears it down when work ends, and two checkouts
of one organism are the same tenant.

migrate [organism]   bring the organism's allocations up and migrate them:
                     memory's schema and the nerves' stream move to this
                     toolchain's, and the head's DSN is kept in the
                     checkout's .hale/dna/memory.env. With
                     HALE_DNA_MEMORY_DSN_OWNER or HALE_DNA_NATS_URL_OWNER
                     set, that server of your own is migrated instead.
                     Idempotent. The organism defaults to the current
                     directory.

sweep <root>         take down every compose project run from under <root>,
                     a test's habitat: its containers, networks and
                     volumes, confirmed gone by the project's label. The
                     projects are named in <root>/.dna-compose-projects
                     until they are, so a teardown that did not finish is
                     retried. Never the developer's habitat (under $HOME),
                     nor the one XDG_DATA_HOME names outside <root>. Exit 1
                     names what survived; nothing is done without docker.

Nothing else migrates or provisions: `hale dna dev`, `run`, `head`, `show`
and `work` run on what is up and migrated, and name this command when it
is not.
"#
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            print!("{}", usage_text());
            ExitCode::SUCCESS
        }
        Some("migrate") => migrate(args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))),
        Some("sweep") if args.len() == 2 => match down_under(Path::new(&args[1])) {
            Ok(()) => ExitCode::SUCCESS,
            Err(why) => {
                eprintln!("hale habitat sweep: {why}");
                ExitCode::from(1)
            }
        },
        _ => {
            eprint!("{}", usage_text());
            ExitCode::from(2)
        }
    }
}

fn migrate(dir: PathBuf) -> ExitCode {
    let root = match project(&dir) {
        Ok((root, _)) => root,
        Err(e) => {
            eprintln!("hale habitat migrate: {e}");
            return ExitCode::from(2);
        }
    };
    match memory_migrate(&root) {
        Ok(MemoryPlan::Roles { .. }) => println!("memory  migrated; the head's DSN is kept in {}", root.join(".hale/dna/memory.env").display()),
        Ok(MemoryPlan::NoDatabase(why)) => {
            eprintln!("hale habitat migrate: memory: {why}");
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("hale habitat migrate: memory: {e}");
            return ExitCode::from(1);
        }
    }
    match nerves_migrate(&root) {
        Ok(NervesPlan::Roles(_)) => println!("nerves  migrated; the organization's stream is in place"),
        Ok(NervesPlan::NoServer(why)) => {
            eprintln!("hale habitat migrate: nerves: {why}");
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("hale habitat migrate: nerves: {e}");
            return ExitCode::from(1);
        }
    }
    // the senses' store is the habitat's when memory and the nerves are:
    // an environment that names servers of its own names its own store.
    // Nothing reads it but the reflexes, so one that cannot come up is said
    let own_servers = ["HALE_DNA_MEMORY_DSN_OWNER", "HALE_DNA_NATS_URL_OWNER"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()));
    if !own_servers {
        match host_run("senses-up", &root, &[]) {
            Ok(out) if !out.starts_with("none: ") => println!("senses  up ({})", out.trim().trim_start_matches("HALE_DNA_SENSES_URL=")),
            Ok(out) => println!("senses  not up: {}", out.trim().strip_prefix("none: ").unwrap_or(out.trim())),
            Err(e) => println!("senses  not up: {e}"),
        }
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------
// the teardown of a test's habitat (GH #1653)
// ---------------------------------------------------------------

/// The file under a root naming the compose projects found rooted in it,
/// one per line, kept until their teardown is confirmed: a volume carries
/// its project's name but not the directory it ran from, so a project
/// whose containers went and whose volumes did not is found again by name.
/// `dna::scratch_compose_down` keeps the same file.
pub(crate) const COMPOSE_FILE: &str = ".dna-compose-projects";

/// What docker answered: its output, nothing to ask (no CLI or no daemon:
/// nothing compose started can be running there), or why it failed.
pub(crate) enum Docker {
    Answered(String),
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

/// The compose project of the local habitat that lives at `dir`, as
/// `dna::habitat_project_at` names it.
pub(crate) fn habitat_project_at(dir: &str) -> String {
    let h = openssl::sha::sha256(dir.as_bytes());
    format!("hale-habitat-local-{}", h[..4].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The projects of docker's `listed` (`<project>|<working dir>` lines)
/// rooted in `root`, and the ones `recorded` names, as
/// `dna::scratch_projects_in` finds them: never the developer's own
/// habitat (under `$HOME`), nor the one this process's `XDG_DATA_HOME`
/// names when it lives outside `root`. The machine's long-lived habitat is
/// no teardown's, whatever a file under a root says.
pub(crate) fn projects_rooted_in(listed: &str, root: &Path, recorded: &str) -> Vec<String> {
    let text = root.display().to_string();
    let under = format!("{text}/");
    let home = std::env::var("HOME").unwrap_or_default();
    let mine = habitat_project_at(&format!("{home}/.local/share/hale/habitat/local"));
    let data = std::env::var("XDG_DATA_HOME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| format!("{home}/.local/share"));
    let here = format!("{data}/hale/habitat/local");
    let current = if here == text || here.starts_with(&under) { None } else { Some(habitat_project_at(&here)) };
    let mut projects: Vec<String> = listed
        .lines()
        .filter_map(|l| l.split_once('|'))
        .filter(|(p, wd)| !p.is_empty() && (*wd == text || wd.starts_with(&under)))
        .map(|(p, _)| p.to_string())
        .chain(recorded.lines().filter(|l| !l.is_empty()).map(str::to_string))
        .filter(|p| *p != mine && Some(p) != current.as_ref())
        .collect();
    projects.sort();
    projects.dedup();
    projects
}

/// Take down every compose project rooted in `root` — its containers,
/// networks and volumes (`docker compose -p <project> down -v`, which needs
/// no compose file) — and confirm none of it is left, by the project's
/// label on containers and on volumes. The projects are named in
/// `root`'s [`COMPOSE_FILE`] before anything is taken down and the file
/// goes only once nothing is left, so a teardown that did not finish (a
/// volume docker would not remove) is found again and retried by the next.
/// `Err` names what failed or survived; the caller keeps `root`. The one
/// teardown `hale test`, the CLI tests' reap and
/// `scripts/compose-down-under.sh` share (`hale habitat sweep`).
pub(crate) fn down_under(root: &Path) -> Result<(), String> {
    down_under_with(root, &mut |args| docker(args))
}

pub(crate) fn down_under_with(root: &Path, docker: &mut dyn FnMut(&[&str]) -> Docker) -> Result<(), String> {
    let file = root.join(COMPOSE_FILE);
    let recorded = std::fs::read_to_string(&file).unwrap_or_default();
    let listed = match docker(&["ps", "-a", "--filter", "label=com.docker.compose.project", "--format", "{{.Label \"com.docker.compose.project\"}}|{{.Label \"com.docker.compose.project.working_dir\"}}"]) {
        Docker::Absent => return Ok(()),
        Docker::Failed(why) => return Err(why),
        Docker::Answered(out) => out,
    };
    let projects = projects_rooted_in(&listed, root, &recorded);
    if projects.is_empty() {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    let _ = std::fs::create_dir_all(root);
    std::fs::write(&file, projects.iter().map(|p| format!("{p}\n")).collect::<String>()).map_err(|e| format!("write {}: {e}", file.display()))?;
    let mut left = Vec::new();
    for project in &projects {
        if let Docker::Failed(why) = docker(&["compose", "-p", project, "down", "-v", "--remove-orphans"]) {
            left.push(why);
        }
        let label = format!("label=com.docker.compose.project={project}");
        for (what, args) in [("containers", ["ps", "-a", "-q", "--filter", label.as_str()].as_slice()), ("volumes", ["volume", "ls", "-q", "--filter", label.as_str()].as_slice())] {
            match docker(args) {
                Docker::Answered(out) if !out.trim().is_empty() => left.push(format!("compose project {project} still has {what} after its teardown")),
                Docker::Failed(why) => left.push(why),
                _ => {}
            }
        }
    }
    if left.is_empty() {
        let _ = std::fs::remove_file(&file);
        Ok(())
    } else {
        Err(left.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hale_habitat_sweep_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A teardown whose volume did not go keeps the project named under the
    /// root and fails; the next finds it by that name alone (its containers
    /// are gone, so no label points at the root any more), confirms it gone
    /// and only then lets the name go.
    #[test]
    fn a_volume_that_survives_is_retried_by_name() {
        let d = root("retry");
        let p = habitat_project_at(&format!("{}/data/hale/habitat/local", d.display()));
        let listing = format!("{p}|{}/data/hale/habitat/local/allocations/abc\n", d.display());
        let mut volume_left = true;
        let mut first = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(listing.clone()),
            ["compose", ..] => Docker::Answered(String::new()),
            ["ps", "-a", "-q", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(if volume_left { format!("{p}_memory-abc\n") } else { String::new() }),
            _ => Docker::Failed("unexpected".into()),
        };
        let err = down_under_with(&d, &mut first).unwrap_err();
        assert!(err.contains("still has volumes"), "{err}");
        assert_eq!(std::fs::read_to_string(d.join(COMPOSE_FILE)).unwrap(), format!("{p}\n"), "the project stays named");
        // the containers went: nothing labels the root any more, and the
        // volume is still there, so the next teardown fails again, by name
        let mut downs = Vec::new();
        let mut again = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(String::new()),
            ["compose", "-p", project, ..] => {
                downs.push(project.to_string());
                Docker::Answered(String::new())
            }
            ["ps", "-a", "-q", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(format!("{p}_memory-abc\n")),
            _ => Docker::Failed("unexpected".into()),
        };
        assert!(down_under_with(&d, &mut again).is_err());
        assert_eq!(downs, vec![p.clone()], "the recorded project is taken down again");
        assert!(d.join(COMPOSE_FILE).exists());
        // once the volume goes, the name goes
        volume_left = false;
        let mut done = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(if volume_left { "x\n".into() } else { String::new() }),
            _ => Docker::Answered(String::new()),
        };
        down_under_with(&d, &mut done).unwrap();
        assert!(!d.join(COMPOSE_FILE).exists(), "confirmed gone, the record goes");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The developer's habitat is never a teardown's, even named in a
    /// root's file.
    #[test]
    fn the_developers_habitat_is_never_taken_down() {
        let d = root("developer");
        let home = std::env::var("HOME").unwrap_or_default();
        let developers = habitat_project_at(&format!("{home}/.local/share/hale/habitat/local"));
        std::fs::write(d.join(COMPOSE_FILE), format!("{developers}\n")).unwrap();
        let mut touched = false;
        let mut dock = |args: &[&str]| {
            if args[0] == "compose" {
                touched = true;
            }
            Docker::Answered(if args[0] == "ps" && args.len() > 3 && args[2] == "--filter" && args[3] == "label=com.docker.compose.project" { format!("{developers}|{home}/.local/share/hale/habitat/local\n") } else { String::new() })
        };
        down_under_with(&d, &mut dock).unwrap();
        assert!(!touched, "nothing was taken down");
        let _ = std::fs::remove_dir_all(&d);
    }
}
