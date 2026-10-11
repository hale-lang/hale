//! `hale dna head` — the head as a toolchain verb.
//!
//! The head (the project service serving the browser face, its
//! per-project API child, the stub OpenID provider and the reflexes) is
//! launched by `dna/face/start.sh`, which builds its four seeds from a
//! checkout. A person with an installed toolchain has none, so the
//! toolchain carries what the head is built from (`hale_dna::HEAD_FILES`:
//! the seeds, the face's assets and the launcher itself) the way it
//! carries the host. This verb materializes that tree into the toolchain
//! cache (`hale_iris::materialize`, keyed by `toolchain_hash`, which
//! covers the head's files), builds the seeds there once — side by side,
//! under the cache's build lock — and execs the materialized launcher
//! with every binary handed in, so the launcher builds nothing and its
//! `checkout` is the cache. A second `hale dna head` finds the binaries
//! built and only launches.
//!
//! With a project and no `HALE_DNA_MEMORY_DSN_HEAD`, the head's memory
//! DSN is derived as `hale dna memory migrate` derives it (the same
//! function, in this process), handed to the launcher on its
//! environment and never printed. Memory that cannot be reached is said
//! plainly, with `hale dna dev` as the way to bring it up; the head
//! starts anyway, with Knowledge unsupported.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

use super::{memory_migrate, MemoryPlan};

/// The verb's help: stdout and exit 0 for `--help`, stderr and exit 2
/// for a usage error (GH #817).
pub(crate) fn usage_text() -> &'static str {
    "usage: hale dna head [project] [--port P] [--api-port P] [--oidc-port P] [--commands-port P] [--source-drafts]\n\
     \n\
     Start the head: the project service serving the browser face at\n\
     http://127.0.0.1:<port>/ (8792), its per-project API child (--api-port,\n\
     8793; its commands on --commands-port, 8795), the stub OpenID provider you\n\
     sign in through (--oidc-port, 8794) and, with a project, the reflexes.\n\
     Without a project the browser opens on Projects, where one is created,\n\
     initialized or attached. --source-drafts enables Organization source\n\
     preparation.\n\
     \n\
     Needs no checkout: the head's sources, the face's assets and its launcher\n\
     are part of the toolchain, built once into the toolchain cache\n\
     (`hale iris --where` names it) and reused until the toolchain changes.\n\
     From a hale checkout, dna/face/start.sh starts the same head.\n\
     \n\
     With a project and no HALE_DNA_MEMORY_DSN_HEAD, the head's memory DSN is\n\
     derived as `hale dna memory migrate` derives it (never printed); when the\n\
     project's memory is not up, `hale dna dev <project>` brings it and the\n\
     nerves up, and until then Knowledge is unsupported. The rest of the\n\
     launcher's environment (HALE_DNA_HEAD_STATE, HALE_DNA_NATS_URL_HEAD, …)\n\
     is inherited as dna/face/start.sh --help describes.\n\
     \n\
     Ctrl-C stops the head; the API child, a local body and every run it\n\
     started keep running and are re-adopted by the next head.\n"
}

pub(crate) fn head_cmd(args: &[String]) -> ExitCode {
    let mut project: Option<String> = None;
    let mut passed: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => {
                print!("{}", usage_text());
                return ExitCode::SUCCESS;
            }
            "--port" | "--api-port" | "--oidc-port" | "--commands-port" => match it.next() {
                Some(v) => {
                    passed.push(a.clone());
                    passed.push(v.clone());
                }
                None => {
                    eprintln!("hale dna head: {a} needs a value");
                    return ExitCode::from(2);
                }
            },
            "--source-drafts" => passed.push(a.clone()),
            f if f.starts_with('-') => {
                eprintln!("hale dna head: `{f}` is not a flag of head\n{}", usage_text());
                return ExitCode::from(2);
            }
            p => {
                if project.is_some() {
                    eprintln!("hale dna head: one project at most\n{}", usage_text());
                    return ExitCode::from(2);
                }
                project = Some(p.to_string());
            }
        }
    }
    match launch(project.as_deref(), &passed) {
        // `launch` execs; it returns only when it could not
        Ok(never) => match never {},
        Err(e) => {
            eprintln!("hale dna head: {e}");
            ExitCode::from(1)
        }
    }
}

fn set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

enum Never {}

fn launch(project: Option<&str>, passed: &[String]) -> Result<Never, String> {
    let cache = hale_iris::materialize(&crate::build_env::host_cache_options()).map_err(|e| format!("cannot materialize the toolchain cache: {e}"))?;
    // The seeds the launcher would build: a binary handed in is the
    // launcher's to use (HALE_API_BIN …), and a fixture's trusted-local
    // session starts neither the provider nor the reflexes.
    let trusted_local = std::env::var("HALE_DNA_TRUSTED_LOCAL").is_ok_and(|v| v == "1");
    let mut wanted: Vec<(&str, &hale_dna::HeadSeed)> = Vec::new();
    if !set("HALE_API_BIN") {
        wanted.push(("HALE_API_BIN", &hale_dna::HEAD_API));
    }
    if !set("HALE_HEAD_BIN") {
        wanted.push(("HALE_HEAD_BIN", &hale_dna::HEAD_SERVICE));
    }
    if !trusted_local && !set("HALE_OIDC_BIN") {
        wanted.push(("HALE_OIDC_BIN", &hale_dna::HEAD_OIDC));
    }
    if !trusted_local && project.is_some() && !set("HALE_REFLEXES_BIN") {
        wanted.push(("HALE_REFLEXES_BIN", &hale_dna::HEAD_REFLEXES));
    }
    let mut seeds: Vec<(&str, &str)> = wanted.iter().map(|(_, s)| (s.seed, s.bin)).collect();
    // with a project, the host answers memory's migration here and the
    // launcher's `senses up` and `nerves migrate` (its verb program): its
    // two programs built beside the head, as any host verb builds both
    if project.is_some() {
        seeds.push((hale_dna::HOST_SEED, hale_dna::HOST_BIN));
        seeds.push((hale_dna::HOST_VERB_SEED, hale_dna::HOST_VERB_BIN));
    }
    let built = crate::iris::ensure_all_built_in(&cache, &seeds, "the head")?;
    let me = std::env::current_exe().map_err(|e| format!("cannot locate the hale binary: {e}"))?;
    let mut cmd = Command::new("bash");
    cmd.arg(cache.join(hale_dna::HEAD_LAUNCHER));
    if let Some(p) = project {
        cmd.arg(p);
    }
    cmd.args(passed).env("HALE_BIN", &me);
    for ((var, _), bin) in wanted.iter().zip(&built) {
        cmd.env(var, bin);
    }
    if let Some(p) = project {
        if !set("HALE_DNA_MEMORY_DSN_HEAD") {
            if let Some(dsn) = head_dsn(Path::new(p)) {
                cmd.env("HALE_DNA_MEMORY_DSN_HEAD", dsn);
            }
        }
    }
    // exec in place: the pid that ran `hale dna head` is the launcher, so
    // a Ctrl-C or a signal to it runs the launcher's cleanup
    use std::os::unix::process::CommandExt;
    let e = cmd.exec();
    Err(format!("cannot run {}: {e}", cache.join(hale_dna::HEAD_LAUNCHER).display()))
}

/// The head's memory DSN for `project`, as `hale dna memory migrate`
/// prints it (`HALE_DNA_MEMORY_DSN_HEAD`): the same function, applying
/// memory's schema with the owner's DSN when it is not applied yet, which
/// is idempotent. The DSN carries the head role's password, so it travels
/// on the launcher's environment alone. `None`, said plainly, when there
/// is no memory to reach — or when the project's compose memory is not
/// running: the head starts no database (`hale dna dev` does), so the
/// migration is asked only of memory that is already up.
fn head_dsn(project: &Path) -> Option<OsString> {
    let why = match memory_down(project) {
        Some(why) => why,
        None => match memory_migrate(project) {
            Ok(MemoryPlan::Roles { head, .. }) => {
                eprintln!("hale dna head: the head reads Knowledge from memory (its DSN derived as `hale dna memory migrate` derives it)");
                return Some(OsString::from(head));
            }
            Ok(MemoryPlan::NoDatabase(why)) => why,
            Err(e) => e,
        },
    };
    eprintln!("hale dna head: memory is not up: {why}");
    eprintln!(
        "hale dna head: `hale dna dev {}` brings the project's memory and nerves up; until then the head runs with Knowledge unsupported",
        project.display()
    );
    None
}

/// The head's memory DSN for `project`, derived as [`head_dsn`] derives it,
/// for a verb that reads memory as the head (`hale dna show`): `None`,
/// quietly, when memory is not up, for that verb to say so its own way.
pub(super) fn derived_head_dsn(project: &Path) -> Option<OsString> {
    if memory_down(project).is_some() {
        return None;
    }
    match memory_migrate(project) {
        Ok(MemoryPlan::Roles { head, .. }) => Some(OsString::from(head)),
        _ => None,
    }
}

/// Why memory is not up for `project`, or `None` when it is: an owner's
/// own server (HALE_DNA_MEMORY_DSN_OWNER) is the owner's to keep up and
/// is asked directly; otherwise the project's compose `knowledge-db` must
/// be running already, since migrating would bring it up.
fn memory_down(project: &Path) -> Option<String> {
    if set("HALE_DNA_MEMORY_DSN_OWNER") {
        return None;
    }
    let root = match super::project(project) {
        Ok((root, _)) => root,
        Err(e) => return Some(e),
    };
    let compose = root.join("dna/compose.yaml");
    if !compose.is_file() {
        return Some("no dna/compose.yaml and no HALE_DNA_MEMORY_DSN_OWNER".to_string());
    }
    let out = Command::new("docker").args(["compose", "-f"]).arg(&compose).args(["ps", "--status", "running", "--services"]).output();
    match out {
        Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == "knowledge-db") => None,
        Ok(_) => Some("the project's memory (compose's `knowledge-db`) is not running".to_string()),
        Err(e) => Some(format!("docker: {e}")),
    }
}
