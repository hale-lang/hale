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

use std::path::PathBuf;
use std::process::ExitCode;

use super::{host_run, memory_migrate, nerves_migrate, project, MemoryPlan, NervesPlan};

/// The verb's help: stdout and exit 0 for `--help`, stderr and exit 2
/// for a usage error (GH #817).
pub(crate) fn usage_text() -> &'static str {
    "usage: hale habitat migrate [organism]\n\
     \n\
     The local habitat holds every organism's infrastructure on this machine:\n\
     one docker compose project, named for the habitat, with a service per\n\
     allocation (memory-<id>, nerves-<id>, senses-<id>, <id> the organism's\n\
     record identity), under ${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local.\n\
     It is long-lived: nothing tears it down when work ends, and two checkouts\n\
     of one organism are the same tenant.\n\
     \n\
     migrate [organism]   bring the organism's allocations up and migrate them:\n\
                          memory's schema and the nerves' stream move to this\n\
                          toolchain's, and the head's DSN is kept in the\n\
                          checkout's .hale/dna/memory.env. With\n\
                          HALE_DNA_MEMORY_DSN_OWNER or HALE_DNA_NATS_URL_OWNER\n\
                          set, that server of your own is migrated instead.\n\
                          Idempotent. The organism defaults to the current\n\
                          directory.\n\
     \n\
     Nothing else migrates or provisions: `hale dna dev`, `run`, `head`, `show`\n\
     and `work` run on what is up and migrated, and name this command when it\n\
     is not.\n"
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            print!("{}", usage_text());
            ExitCode::SUCCESS
        }
        Some("migrate") => migrate(args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))),
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
