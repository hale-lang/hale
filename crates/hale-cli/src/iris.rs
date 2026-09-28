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
//!   hale iris inspect <artifact> [url]  artifact-side inspector
//!   hale iris --where                   print the cache directory
//!   hale iris --build-only              materialize + build, print the binary
//!
//! `hale run --observe <target>` sets `LOTUS_OBS=1` on the program and
//! launches `hale iris` beside it for the program's lifetime — with
//! its own stdout, and never outliving the `hale` that started it
//! (GH #905).

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
/// cache's build lock, and return its binary. `hale dna` builds its
/// own programs from the same cache the same way (GH #566 F7: two
/// commands racing to build one binary exec'd a half-written file —
/// `Permission denied`).
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
    // The DNA toolchain cache is restored from a GitHub Actions cache
    // entry before this runs; a truncated or zero-byte file from an
    // incomplete save (or an interrupted earlier build) is still
    // `is_file()`, and handing its path to a caller that execs it is
    // worse than a slow rebuild. A present, non-empty file is trusted;
    // anything else is removed and treated as absent.
    match std::fs::metadata(&bin_path) {
        Ok(m) if m.is_file() && m.len() > 0 => return Ok(bin_path),
        Ok(_) => {
            let _ = std::fs::remove_file(&bin_path);
        }
        Err(_) => {}
    }
    let me = std::env::current_exe().map_err(|e| format!("hale iris: cannot locate the hale binary: {e}"))?;
    eprintln!("hale iris: building {what} ({} @ {})", seed, root.display());
    let mut build = Command::new(&me);
    build.arg("build").arg(root.join(seed)).stdin(Stdio::null()).stdout(Stdio::null());
    crate::dies_with_us(&mut build);
    let status = build.status().map_err(|e| format!("hale iris: build failed to start: {e}"))?;
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

fn exec(bin: &Path, args: &[String]) -> ExitCode {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    // fuse-hl is our child, not a peer: `hale iris` is a supervisor
    // that only waits. Without this, killing `hale iris` (which is
    // what `hale run --observe` does when the program ends, and what
    // a parent-death signal does when it is killed) left fuse-hl
    // running and holding every descriptor it inherited — GH #905.
    crate::dies_with_us(&mut cmd);
    let name = bin.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    match cmd.status() {
        // Every way out says why (GH #578): a child killed by a signal
        // has no exit code, and this used to return 1 without a word —
        // the "exits 1 silently" a loaded CI shard kept seeing. It exits
        // as a shell would, 128 + the signal.
        Ok(st) => match st.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(c) => {
                eprintln!("hale iris: {name} exited with status {c}");
                ExitCode::from(c.clamp(0, 255) as u8)
            }
            None => {
                use std::os::unix::process::ExitStatusExt;
                let sig = st.signal().unwrap_or(0);
                eprintln!(
                    "hale iris: {name} was killed by signal {sig} ({}){}",
                    signal_name(sig),
                    if st.core_dumped() { ", core dumped" } else { "" }
                );
                ExitCode::from((128 + sig).clamp(0, 255) as u8)
            }
        },
        Err(e) => {
            eprintln!("hale iris: cannot exec {}: {e}", bin.display());
            ExitCode::from(1)
        }
    }
}

/// The name of a signal a child can die from, for the line that says so.
fn signal_name(sig: i32) -> &'static str {
    match sig {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGBUS => "SIGBUS",
        libc::SIGABRT => "SIGABRT",
        libc::SIGKILL => "SIGKILL, often the kernel's out-of-memory killer",
        libc::SIGTERM => "SIGTERM",
        libc::SIGINT => "SIGINT",
        libc::SIGHUP => "SIGHUP",
        libc::SIGILL => "SIGILL",
        libc::SIGFPE => "SIGFPE",
        libc::SIGPIPE => "SIGPIPE",
        _ => "another signal",
    }
}

/// `hale iris ...` — `args` are the words after `iris`.
pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => {
            eprintln!("usage: hale iris [port] [artifact.json] [--diff <a.topology> <b.topology> | --diff <diff.json>]");
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
            exec(&bin, &args[1..])
        }
        _ => {
            // GH #527 B5: `--diff a b` diffs the pair HERE (one
            // engine, `hale_types::topology_diff`) and hands the
            // document to fuse-hl; `--diff doc.json` hands a ready
            // one over. With a pair and no artifact positional, the
            // B side is the artifact the law view runs against.
            let mut positional: Vec<String> = Vec::new();
            let mut diff_paths: Vec<String> = Vec::new();
            let mut it = args.iter();
            while let Some(a) = it.next() {
                if a == "--diff" {
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
            exec(&bin, &fargs)
        }
    }
}

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
///
/// The session is strictly ancillary to the run, so (GH #905):
///
///   * it does not outlive us — see [`crate::dies_with_us`];
///   * it does not write to OUR stdout. The program's stdout is the
///     command's output, and a caller consuming it through a pipe
///     must see EOF when we exit. An observer sharing that
///     descriptor both interleaves its chatter into the program's
///     output and, orphaned, holds the pipe open forever. It gets
///     its own pipe instead, which we relay to stderr — the session
///     still says what it is doing, on the stream diagnostics belong
///     on. The relay is a second, portable layer under the
///     parent-death signal: when we die the read end closes, so the
///     session's writes fail even where `prctl` does not exist.
pub fn spawn_session() -> Option<std::process::Child> {
    let me = std::env::current_exe().ok()?;
    let mut cmd = Command::new(me);
    cmd.arg("iris").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::dies_with_us(&mut cmd);
    match cmd.spawn() {
        Ok(mut c) => {
            if let Some(out) = c.stdout.take() {
                relay_to_stderr(out);
            }
            if let Some(err) = c.stderr.take() {
                relay_to_stderr(err);
            }
            Some(c)
        }
        Err(e) => {
            eprintln!("hale run --observe: could not launch hale iris: {e}");
            None
        }
    }
}

/// Copy one of the session's streams to ours until it ends. The
/// thread is detached: it retires at EOF, which the session's death
/// guarantees, and the process exiting under it is the other way out.
fn relay_to_stderr<R: std::io::Read + Send + 'static>(mut src: R) {
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut src, &mut std::io::stderr());
    });
}
