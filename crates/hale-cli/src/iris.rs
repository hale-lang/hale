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
    let root = hale_iris::materialize(&crate::build_env::host_cache_options()).map_err(|e| format!("hale iris: cannot materialize sources: {e}"))?;
    let bin_path = ensure_built_in(&root, seed, bin, "the observer")?;
    Ok((root, bin_path))
}

/// Build `seed` under the materialized cache `root` once and return its
/// binary. `hale dna` builds its own programs from the same cache the
/// same way; see [`ensure_seeds`] for how a build and a reader of the
/// cache stay out of each other's way (GH #566 F7).
pub(crate) fn ensure_built_in(root: &Path, seed: &str, bin: &str, what: &str) -> Result<PathBuf, String> {
    let built = ensure_seeds(root, &[(seed, bin)], &Report::Inherit(what), &|seed, out| hale_build(root, seed, out))?;
    Ok(built.into_iter().next().unwrap_or_else(|| root.join(bin)))
}

/// Build several seeds under the materialized cache `root`, side by side,
/// and return their binaries in the order given. A seed whose binary is
/// already there is not rebuilt. Each `hale build` is one single-threaded
/// process, so the head's four seeds build in parallel as
/// `dna/face/start.sh` builds them from a checkout (GH #1147); every
/// build is waited for even after one fails, and a failed build's output
/// is printed under its seed's name.
pub(crate) fn ensure_all_built_in(root: &Path, seeds: &[(&str, &str)], what: &str) -> Result<Vec<PathBuf>, String> {
    ensure_seeds(root, seeds, &Report::Logged(what), &|seed, out| {
        let mut build = hale_build(root, seed, out)?;
        // observation belongs to the application, never the head's parts
        build.env_remove("LOTUS_OBS");
        Ok(build)
    })
}

/// `hale build <root>/<seed> -o <out>` with THIS binary.
fn hale_build(root: &Path, seed: &str, out: &Path) -> Result<Command, String> {
    let me = std::env::current_exe().map_err(|e| format!("cannot locate the hale binary: {e}"))?;
    let mut build = Command::new(me);
    build.arg("build").arg(root.join(seed)).arg("-o").arg(out).stdin(Stdio::null()).stdout(Stdio::null());
    Ok(build)
}

/// What a build of `what` says while it runs and when it fails.
enum Report<'a> {
    /// One line as it starts, `hale iris: building <what> (<seed>)`; its
    /// own output on our stderr as it comes.
    Inherit(&'a str),
    /// One line per seed as it starts, `hale: building <what>: <seed>`;
    /// its output kept under `<root>/.build-logs` and printed only when
    /// it fails.
    Logged(&'a str),
}

/// The command that compiles `seed` (relative to the cache root) into
/// the file `out`.
type BuildCommand<'a> = dyn Fn(&str, &Path) -> Result<Command, String> + 'a;

/// A cached binary is trusted when it is a non-empty file. The DNA
/// toolchain cache is restored from a GitHub Actions cache entry before
/// this runs; a truncated or zero-byte file from an incomplete save is
/// still `is_file()`, and handing its path to a caller that execs it is
/// worse than a slow rebuild, so anything else is rebuilt.
fn is_built(bin: &Path) -> bool {
    matches!(std::fs::metadata(bin), Ok(m) if m.is_file() && m.len() > 0)
}

/// Make every `(seed, bin)` of `seeds` present under `root` and return
/// the binaries in the order given.
///
/// A binary only ever appears at its path complete: each build writes a
/// private sibling (`<bin>.tmp-<pid>-<n>`) and renames it into place once
/// it succeeded, so a reader that finds the path finds the whole file —
/// the guarantee GH #566 F7 needed (two commands racing to build one
/// binary exec'd a half-written one, `Permission denied`). That is what
/// lets the common case skip the lock: when every binary is already
/// there, this returns without touching it, so a host start never waits
/// behind `hale dna head` building its seeds cold into the same cache.
/// Only what is missing is built, under an exclusive flock on
/// `<root>/.build.lock` (one build per cache: two builds of one seed
/// would trample each other's work), and every binary is looked at again
/// once the lock is held, since the holder we waited for may have built
/// it. A wait for the lock says so, once.
fn ensure_seeds(root: &Path, seeds: &[(&str, &str)], report: &Report<'_>, command: &BuildCommand<'_>) -> Result<Vec<PathBuf>, String> {
    let bins: Vec<PathBuf> = seeds.iter().map(|(_, bin)| root.join(bin)).collect();
    if bins.iter().all(|b| is_built(b)) {
        return Ok(bins);
    }
    let _guard = BuildLock::acquire(root)?;
    let logs = root.join(".build-logs");
    let mut running = Vec::new();
    for ((seed, _), bin_path) in seeds.iter().zip(&bins) {
        if is_built(bin_path) {
            continue;
        }
        let tmp = tmp_sibling(bin_path);
        let mut build = command(seed, &tmp)?;
        let log_path = match report {
            Report::Inherit(what) => {
                eprintln!("hale iris: building {what} ({seed} @ {})", root.display());
                None
            }
            Report::Logged(what) => {
                std::fs::create_dir_all(&logs).map_err(|e| format!("{}: {e}", logs.display()))?;
                let log_path = logs.join(format!("{}.log", seed.replace('/', "_")));
                let log = std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
                eprintln!("hale: building {what}: {seed} @ {}", root.display());
                build.stderr(log);
                Some(log_path)
            }
        };
        crate::shared::process::dies_with_us(&mut build);
        let child = build.spawn().map_err(|e| format!("building {seed} failed to start: {e}"))?;
        running.push((*seed, bin_path, tmp, log_path, child));
    }
    let mut failed = Vec::new();
    for (seed, bin_path, tmp, log_path, mut child) in running {
        let outcome = match child.wait() {
            Err(e) => Err(format!("failed: {e}")),
            Ok(st) if !st.success() => Err(format!("failed ({st})")),
            Ok(_) if !is_built(&tmp) => Err(format!("produced no binary at {}", tmp.display())),
            Ok(_) => publish(&tmp, bin_path),
        };
        if let Err(why) = outcome {
            let _ = std::fs::remove_file(&tmp);
            if let Some(log_path) = &log_path {
                eprintln!("hale: ── {seed}: FAILED ({why})");
                eprint!("{}", std::fs::read_to_string(log_path).unwrap_or_default());
            }
            failed.push(format!("{seed} {why}"));
        }
    }
    if !failed.is_empty() {
        return Err(match report {
            Report::Logged(_) => format!("cannot build {} (its output is above)", failed.join(", ")),
            Report::Inherit(_) => format!("hale iris: building {}", failed.join(", ")),
        });
    }
    Ok(bins)
}

/// A build-private name beside `bin` (the same directory, so the rename
/// into place is atomic): `<bin>.tmp-<pid>-<n>`. `LOTUS_DUMP_IR` leaves a
/// build's IR at its output's `with_extension("ll")`, which for this name
/// is already `<bin>.ll`, where `scripts/warm-dna-cache.sh` reads it.
fn tmp_sibling(bin: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NONCE: AtomicU64 = AtomicU64::new(0);
    let name = bin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "bin".into());
    bin.with_file_name(format!("{name}.tmp-{}-{}", std::process::id(), NONCE.fetch_add(1, Ordering::Relaxed)))
}

/// Rename a finished build into place, with its `LOTUS_DUMP_IR` IR when
/// that did not already land where the binary's own would.
fn publish(tmp: &Path, bin: &Path) -> Result<(), String> {
    std::fs::rename(tmp, bin).map_err(|e| format!("cannot move the built binary to {}: {e}", bin.display()))?;
    let (ir, want) = (tmp.with_extension("ll"), bin.with_extension("ll"));
    if ir != want && ir.is_file() {
        let _ = std::fs::rename(&ir, &want);
    }
    Ok(())
}

/// The cache's exclusive advisory build lock, held until drop.
struct BuildLock(std::fs::File);

impl BuildLock {
    /// Take `<root>/.build.lock`. A holder that makes us wait (another
    /// `hale` building into this cache) is announced with one line, so a
    /// wait is never silent.
    fn acquire(root: &Path) -> Result<Self, String> {
        let lock_path = root.join(".build.lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| format!("cannot open {}: {e}", lock_path.display()))?;
        if flock(&lock, libc::LOCK_EX | libc::LOCK_NB) == Err(libc::EWOULDBLOCK) {
            eprintln!("hale: waiting for the build lock at {} (another hale is building into this cache)", root.display());
            let _ = flock(&lock, libc::LOCK_EX);
        }
        Ok(BuildLock(lock))
    }
}

/// `flock(2)` with EINTR retried; the errno when it was not taken. An
/// error other than a held lock means "no lock", which degrades to the
/// old racy behavior, never worse: no caller refuses on it.
fn flock(f: &std::fs::File, op: libc::c_int) -> Result<(), i32> {
    use std::os::unix::io::AsRawFd;
    loop {
        // SAFETY: flock on a valid, open descriptor the caller owns.
        let r = unsafe { libc::flock(f.as_raw_fd(), op) };
        if r == 0 {
            return Ok(());
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EINTR) => continue,
            e => return Err(e.unwrap_or(0)),
        }
    }
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: same descriptor, still open.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// `observer`: `bin` is fuse-hl, whose nonzero status is a failure worth a
/// line; `inspect`'s exit status is its verdict and speaks for itself.
fn exec(bin: &Path, args: &[String], observer: bool) -> ExitCode {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    // fuse-hl is our child, not a peer: `hale iris` is a supervisor
    // that only waits. Without this, killing `hale iris` (which is
    // what `hale run --observe` does when the program ends, and what
    // a parent-death signal does when it is killed) left fuse-hl
    // running and holding every descriptor it inherited — GH #905.
    crate::shared::process::dies_with_us(&mut cmd);
    let name = bin.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    match cmd.status() {
        // Every way out says why (GH #578): a child killed by a signal
        // has no exit code, and this used to return 1 without a word —
        // the "exits 1 silently" a loaded CI shard kept seeing. It exits
        // as a shell would, 128 + the signal.
        Ok(st) => match st.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(c) => {
                if observer {
                    eprintln!("hale iris: {name} exited with status {c}");
                }
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
        Some("--where") => match hale_iris::cache_dir(&crate::build_env::host_cache_options()) {
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
            exec(&bin, &args[1..], false)
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
            exec(&bin, &fargs, true)
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
///   * it does not outlive us — see [`crate::shared::process::dies_with_us`];
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
///
/// `port` is the session's (`hale run --observe=<port>`); `None` is
/// `hale iris`'s default.
pub fn spawn_session(port: Option<&str>) -> Option<std::process::Child> {
    let me = std::env::current_exe().ok()?;
    let mut cmd = Command::new(me);
    cmd.arg("iris");
    if let Some(port) = port {
        cmd.arg(port);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::shared::process::dies_with_us(&mut cmd);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// A fresh cache root of this test's own.
    fn root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("hale-iris-build-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("seed")).unwrap();
        root
    }

    /// Hold `<root>/.build.lock` from an open file of the test's own, as
    /// another `hale` building into the cache would.
    fn hold_lock(root: &Path) -> std::fs::File {
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(root.join(".build.lock")).unwrap();
        flock(&f, libc::LOCK_EX).unwrap();
        f
    }

    /// A build that takes its time: half the binary, a pause, the rest,
    /// and a line in `<root>/builds` per build that ran.
    fn slow_build(root: &Path) -> impl Fn(&str, &Path) -> Result<Command, String> + Send + Sync + 'static {
        let builds = root.join("builds");
        move |_seed: &str, out: &Path| {
            let mut c = Command::new("sh");
            c.arg("-c").arg("printf part > \"$1\"; sleep 0.5; printf rest >> \"$1\"; echo built >> \"$2\"").arg("sh").arg(out).arg(&builds);
            Ok(c)
        }
    }

    fn ensure_on_thread(root: &Path) -> mpsc::Receiver<Result<Vec<PathBuf>, String>> {
        let (tx, rx) = mpsc::channel();
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let build = slow_build(&root);
            let _ = tx.send(ensure_seeds(&root, &[("seed", "seed/bin")], &Report::Logged("the test"), &build));
        });
        rx
    }

    /// The bug: a host start waited, silently, for the whole of
    /// `hale dna head`'s cold build of seeds it did not need, because the
    /// lock was taken before anything was looked at. With every binary
    /// present the lock is never touched.
    #[test]
    fn present_binaries_are_returned_while_another_holds_the_build_lock() {
        let root = root("present");
        std::fs::write(root.join("seed/bin"), b"complete").unwrap();
        let _held = hold_lock(&root);
        let rx = ensure_on_thread(&root);
        let got = rx.recv_timeout(Duration::from_secs(1)).expect("blocked on the build lock with every binary present");
        assert_eq!(got.unwrap(), vec![root.join("seed/bin")]);
        assert!(!root.join("builds").exists(), "nothing was built");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A missing binary is still built under the lock: the caller waits for
    /// the holder, then builds, and the binary is complete.
    #[test]
    fn a_missing_binary_is_built_under_the_build_lock() {
        let root = root("missing");
        // a truncated restore is not trusted
        std::fs::write(root.join("seed/bin"), b"").unwrap();
        let held = hold_lock(&root);
        let rx = ensure_on_thread(&root);
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err(), "built while another held the build lock");
        assert!(!root.join("builds").exists(), "built while another held the build lock");
        drop(held);
        let got = rx.recv_timeout(Duration::from_secs(20)).expect("never built after the lock was released");
        assert_eq!(got.unwrap(), vec![root.join("seed/bin")]);
        assert_eq!(std::fs::read(root.join("seed/bin")).unwrap(), b"partrest");
        let leftovers: Vec<_> = std::fs::read_dir(root.join("seed")).unwrap().flatten().map(|e| e.file_name()).filter(|n| n != "bin").collect();
        assert!(leftovers.is_empty(), "the build's private file was left behind: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Two callers racing on one missing binary: one builds, the other
    /// finds it built once the lock is free, and nobody — the second
    /// caller or a reader polling the path — ever sees a partial file.
    #[test]
    fn a_concurrent_caller_never_sees_a_partial_binary() {
        let root = root("race");
        let bin = root.join("seed/bin");
        let first = ensure_on_thread(&root);
        std::thread::sleep(Duration::from_millis(100));
        let second = ensure_on_thread(&root);
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut results = Vec::new();
        while results.len() < 2 {
            assert!(std::time::Instant::now() < deadline, "the builds never finished");
            if let Ok(bytes) = std::fs::read(&bin) {
                assert_eq!(bytes, b"partrest", "a reader saw a partial binary");
            }
            for rx in [&first, &second] {
                if let Ok(r) = rx.try_recv() {
                    results.push(r);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        for r in results {
            assert_eq!(r.unwrap(), vec![bin.clone()]);
        }
        assert_eq!(std::fs::read(&bin).unwrap(), b"partrest");
        assert_eq!(std::fs::read_to_string(root.join("builds")).unwrap().lines().count(), 1, "the binary was built twice");
        let _ = std::fs::remove_dir_all(&root);
    }
}
