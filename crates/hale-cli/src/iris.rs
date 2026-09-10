//! `hale iris` — the embedded observer (GH #527 B3).
//!
//! The observer is a Hale program the `hale` binary carries as source
//! (`hale-iris`). This module materializes it into the toolchain-hashed
//! cache, builds it with THIS binary (`hale build` on the seed, so the
//! observer is compiled by the exact compiler that built the observed
//! program), and execs the result with the user's arguments. Nothing
//! here is a second scheduler or a second decoder: fuse-hl attaches
//! over the shm segment like any other consumer.
//!
//!   hale iris [port] [artifact]         fusion + HTTP/SSE at :port (8787)
//!     --diff <a.topology> <b.topology>  review view: diff the pair here
//!     --diff <diff.json>                … or carry a ready diff document
//!     --membrane <dir>                  control channel: publish verdicts and
//!                                       intent to an organism's sockets in <dir>
//!     --organism <status.json>          the organism's status projection (B7)
//!   hale iris inspect <artifact> [url]  artifact-side inspector
//!   hale iris --where                   print the cache directory
//!   hale iris --build-only              materialize + build, print the binary
//!
//! `hale run --observe <target>` sets `LOTUS_OBS=1` on the program and
//! launches `hale iris` beside it for the program's lifetime.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// Materialize the sources; build a seed if its binary is missing.
/// Returns the binary path.
fn ensure_built(seed: &str, bin: &str) -> Result<(PathBuf, PathBuf), String> {
    let root = hale_iris::materialize().map_err(|e| format!("hale iris: cannot materialize sources: {e}"))?;
    let bin_path = ensure_built_in(&root, seed, bin, "the observer")?;
    Ok((root, bin_path))
}

/// Build `seed` under the materialized cache `root` once, under the
/// cache's build lock, and return its binary. The DNA's membrane client
/// and surface share the cache with the observer and build the same
/// way (GH #566 F7: two commands racing to build one binary exec'd a
/// half-written file — `Permission denied`).
pub(crate) fn ensure_built_in(root: &Path, seed: &str, bin: &str, what: &str) -> Result<PathBuf, String> {
    let bin_path = root.join(bin);
    // One build per cache, ever: two `hale iris` (or a test shard's
    // five) racing to build the same seed into the same directory
    // would trample each other's objects. An exclusive flock on the
    // cache root serializes them; the loser finds the binary built.
    // The lock is taken BEFORE the existence check: the linker creates
    // the output before it is complete or executable, and a caller
    // that saw the file and exec'd it got `Permission denied`.
    let lock_path = root.join(".build.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("hale iris: cannot open {}: {e}", lock_path.display()))?;
    let _guard = BuildLock::acquire(&lock);
    if bin_path.is_file() {
        return Ok(bin_path);
    }
    let me = std::env::current_exe().map_err(|e| format!("hale iris: cannot locate the hale binary: {e}"))?;
    eprintln!("hale iris: building {what} ({} @ {})", seed, root.display());
    let status = Command::new(&me)
        .arg("build")
        .arg(root.join(seed))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|e| format!("hale iris: build failed to start: {e}"))?;
    if !status.success() {
        return Err(format!("hale iris: building {} failed ({status})", seed));
    }
    if !bin_path.is_file() {
        return Err(format!("hale iris: build produced no binary at {}", bin_path.display()));
    }
    Ok(bin_path)
}

/// An exclusive advisory lock held for the build; released on drop.
struct BuildLock<'a>(&'a std::fs::File);

impl<'a> BuildLock<'a> {
    fn acquire(f: &'a std::fs::File) -> Self {
        use std::os::unix::io::AsRawFd;
        // SAFETY: flock on a valid, open descriptor we own for the
        // guard's lifetime; EINTR is retried, other errors mean "no
        // lock", which degrades to the old racy behavior, never worse.
        loop {
            let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
            if r == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
        }
        BuildLock(f)
    }
}

impl Drop for BuildLock<'_> {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: same descriptor, still open.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn exec(bin: &Path, args: &[String], envs: &[(String, String)]) -> ExitCode {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    match cmd.status() {
        Ok(st) => match st.code() {
            Some(c) => ExitCode::from(c.clamp(0, 255) as u8),
            None => ExitCode::from(1),
        },
        Err(e) => {
            eprintln!("hale iris: cannot exec {}: {e}", bin.display());
            ExitCode::from(1)
        }
    }
}

/// `hale iris ...` — `args` are the words after `iris`.
pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => {
            eprintln!("usage: hale iris [port] [artifact.json] [--diff <a.topology> <b.topology> | --diff <diff.json>] [--membrane <dir>] [--organism <status.json>]");
            eprintln!("       hale iris inspect <artifact.json> [http://host:port]");
            eprintln!("       hale iris --where | --build-only");
            ExitCode::SUCCESS
        }
        Some("--where") => match hale_iris::cache_dir() {
            Some(d) => {
                println!("{}", d.display());
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("hale iris: no cache directory (set XDG_CACHE_HOME or HOME)");
                ExitCode::from(1)
            }
        },
        Some("--build-only") => match ensure_built(hale_iris::FUSE_SEED, hale_iris::FUSE_BIN) {
            Ok((_, bin)) => {
                println!("{}", bin.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some("inspect") => {
            let (_, bin) = match ensure_built(hale_iris::INSPECT_SEED, hale_iris::INSPECT_BIN) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            exec(&bin, &args[1..], &[])
        }
        _ => {
            // GH #527 B5: `--diff a b` diffs the pair HERE (one
            // engine, `hale_types::topology_diff`) and hands the
            // document to fuse-hl; `--diff doc.json` hands a ready
            // one over. With a pair and no artifact positional, the
            // B side is the artifact the law view runs against.
            let mut positional: Vec<String> = Vec::new();
            let mut diff_paths: Vec<String> = Vec::new();
            let mut membrane: Option<String> = None;
            let mut organism: Option<String> = None;
            let mut it = args.iter();
            while let Some(a) = it.next() {
                if a == "--organism" {
                    match it.next() {
                        Some(p) => organism = Some(p.clone()),
                        None => {
                            eprintln!("hale iris: --organism needs a status.json path");
                            return ExitCode::from(2);
                        }
                    }
                } else if a == "--membrane" {
                    match it.next() {
                        Some(d) => membrane = Some(d.clone()),
                        None => {
                            eprintln!("hale iris: --membrane needs a directory");
                            return ExitCode::from(2);
                        }
                    }
                } else if a == "--diff" {
                    for x in it.by_ref() {
                        if x.starts_with("--") {
                            break;
                        }
                        diff_paths.push(x.clone());
                        if diff_paths.len() == 2 {
                            break;
                        }
                    }
                } else if a.starts_with("--") {
                    eprintln!("hale iris: unknown flag `{a}`");
                    return ExitCode::from(2);
                } else {
                    positional.push(a.clone());
                }
            }
            let diff_doc = match diff_paths.len() {
                0 => None,
                1 => Some(diff_paths[0].clone()),
                _ => match write_pair_diff(&diff_paths[0], &diff_paths[1]) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        eprintln!("{e}");
                        return ExitCode::from(2);
                    }
                },
            };
            // GH #527 B6: the typed control channel. The observer's
            // publishes of `dna.review.verdict` / `dna.intent.offered`
            // are routed to the organism's listen sockets with a
            // LOTUS_BUS_CONFIG file — env-configured connect routes
            // (spec/runtime.md), so an observer started without an
            // organism has nothing to fail on, and one started WITH
            // `--membrane` refuses to boot if the sockets are not
            // there (a route that was asked for may not silently not
            // exist).
            let mut envs: Vec<(String, String)> = Vec::new();
            if let Some(dir) = &membrane {
                let dir = dir.trim_end_matches('/').to_string();
                let conf = std::env::temp_dir().join(format!("hale-iris-membrane-{}.conf", std::process::id()));
                let body = format!(
                    "# hale iris --membrane {dir}\n\
                     dna.review.verdict = unix://{dir}/{} : connect\n\
                     dna.intent.offered = unix://{dir}/{} : connect\n",
                    MEMBRANE_VERDICT_SOCK, MEMBRANE_INTENT_SOCK
                );
                if let Err(e) = std::fs::write(&conf, body) {
                    eprintln!("hale iris: cannot write {}: {e}", conf.display());
                    return ExitCode::from(2);
                }
                envs.push(("LOTUS_BUS_CONFIG".into(), conf.display().to_string()));
                envs.push(("HALE_IRIS_MEMBRANE".into(), dir.clone()));
                eprintln!("hale iris: membrane at {dir} (verdicts and intent publish to the organism)");
            }
            // GH #528 PR 28 (B7): the organism's status projection —
            // a JSON file `hale dna run` re-projects from the Journal —
            // is the observer's third source beside shm and the artifact.
            if let Some(p) = &organism {
                envs.push(("HALE_IRIS_DNA_STATUS".into(), p.clone()));
            }
            let (root, bin) = match ensure_built(hale_iris::FUSE_SEED, hale_iris::FUSE_BIN) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            // fuse-hl's positional argv: port, webroot, [artifact], [diff].
            let port = positional.first().cloned().unwrap_or_else(|| "8787".to_string());
            let artifact = positional
                .get(1)
                .cloned()
                .or_else(|| if diff_paths.len() == 2 { Some(diff_paths[1].clone()) } else { None });
            let mut fargs = vec![port, root.join(hale_iris::WEBROOT).display().to_string()];
            if let Some(doc) = diff_doc {
                fargs.push(artifact.unwrap_or_default());
                fargs.push(doc);
            } else if let Some(artifact) = artifact {
                fargs.push(artifact);
            }
            exec(&bin, &fargs, &envs)
        }
    }
}

/// The organism's membrane sockets, by name inside the membrane
/// directory (`dna/organism/main.hl` binds them under `/tmp`).
pub const MEMBRANE_VERDICT_SOCK: &str = "hale-dna.review.verdict.sock";
pub const MEMBRANE_INTENT_SOCK: &str = "hale-dna.intent.offered.sock";
/// The host's report on an observation window (GH #529 D6); the
/// organism binds it beside the two above.
pub const MEMBRANE_OBSERVED_SOCK: &str = "hale-dna.expression.observed.sock";
/// Pressure raised from outside (an operator, a metrics relay; GH #566 F3).
pub const MEMBRANE_PRESSURE_SOCK: &str = "hale-dna.pressure.raised.sock";

/// Diff two topology artifacts with the compiler's own engine and
/// write the document where fuse-hl can watch it.
fn write_pair_diff(a: &str, b: &str) -> Result<String, String> {
    let mut admitted = Vec::new();
    for p in [a, b] {
        let raw = std::fs::read_to_string(p).map_err(|e| format!("hale iris --diff: cannot read {p}: {e}"))?;
        admitted.push(hale_types::topology_diff::admit(p, &raw).map_err(|e| format!("hale iris --diff: {e}"))?);
    }
    let d = hale_types::topology_diff::diff(&admitted[0], &admitted[1]);
    let out = std::env::temp_dir().join(format!("hale-iris-diff-{}.json", std::process::id()));
    std::fs::write(&out, serde_json::to_string_pretty(&d).unwrap_or_default())
        .map_err(|e| format!("hale iris --diff: cannot write {}: {e}", out.display()))?;
    eprintln!(
        "hale iris: review view over {} -> {} ({})",
        a,
        b,
        d["classification"].as_str().unwrap_or("?")
    );
    Ok(out.display().to_string())
}

/// `hale run --observe`: an iris session for the program's lifetime.
/// Returns the child so the caller can reap it after the program.
pub fn spawn_session() -> Option<std::process::Child> {
    let me = std::env::current_exe().ok()?;
    match Command::new(me).arg("iris").stdin(Stdio::null()).spawn() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("hale run --observe: could not launch hale iris: {e}");
            None
        }
    }
}
