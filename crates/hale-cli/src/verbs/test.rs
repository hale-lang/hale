use std::hash::Hasher;
use std::collections::hash_map::DefaultHasher;
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::path::Path;
use std::path::PathBuf;
use crate::shared::process::RunScratch;
use crate::shared::options::collect_ffi_from_imports;
use crate::shared::diag::diag_file_name;
use std::env;
use std::fs;
use crate::shared::diag::json_escape;
use crate::shared::frontend::LoadMode;
use crate::shared::source::Disk;
use crate::shared::workspace::find_workspace_root;
use crate::shared::diag::render_blocked;
use crate::shared::diag::render_codegen_error;
use crate::shared::diag::render_authors;
use hale_frontend::snapshot::{Config, LoadError, Snapshot, Target};
/// Verdict for one `*_test.hl` file.
pub(crate) struct TestOutcome {
    pub(crate) file: PathBuf,
    pub(crate) passed: bool,
    /// Failure detail: the captured `ASSERTION FAILED …` lines, the
    /// nonzero-exit note, or the compile diagnostic. `None` on pass.
    pub(crate) message: Option<String>,
    pub(crate) elapsed_ms: u128,
}

/// Recursively collect `*_test.hl` files under `target`. A file
/// target is taken as-is (an explicitly-named file runs regardless
/// of suffix — the user asked for it); a directory is walked
/// depth-first, gathering only names ending in `_test.hl`. Entries
/// are visited in sorted order at every level for determinism.
pub(crate) fn collect_test_files(target: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if target.is_file() {
        out.push(target.to_path_buf());
        return Ok(());
    }
    if target.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(target)
            .map_err(|e| format!("{}: {}", target.display(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                // `vendor/` and dot-directories are skipped (spec/testing.md):
                // a toolchain-managed tree, and the DNA's `.hale/` — its
                // worktrees carry the application's own tests (GH #529 D7)
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name == "vendor" || name.starts_with('.') {
                    continue;
                }
                collect_test_files(&p, out)?;
            } else if p
                .file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.ends_with("_test.hl"))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
        return Ok(());
    }
    Err(format!("not a file or directory: {}", target.display()))
}

/// Compile one test file to a temporary native binary, returning
/// its path on success or a rendered diagnostic string on failure.
/// A compile/typecheck error is a test failure — it comes back as
/// the `Err` message. The same snapshot `hale run` loads (a build's
/// config, for the host), with the dev profile, stopping at the binary
/// so the caller can `.output()`-capture the run.
pub(crate) fn compile_test_binary(
    entry: &Path,
    scratch: &RunScratch,
) -> Result<PathBuf, String> {
    // F.40 phase 2.2b: one snapshot, and the lowering view demanded
    // from it after the check.
    let snap = match Snapshot::load(entry, LoadMode::WholeSeed, &Disk, Config::build(Target::host())) {
        Ok(s) => s,
        Err(LoadError::Load(f)) => return Err(f.text()),
        Err(LoadError::Refused(msg)) => return Err(msg),
    };
    let (sources, file_bases) = (snap.sources(), snap.file_bases());
    // Stage-2 FFI pickup, same as `hale build` (2026-07-18; closes
    // pond FRICTION "hale test cannot link @ffi libs"): a test that
    // imports an FFI-bearing lib (sqlite et al.) needs the lib's
    // hale.toml [ffi] link/csrc surface on the link line, or every
    // such test dies with undefined lotus_* references regardless
    // of the test's own correctness.
    let entry_dir = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut options = collect_ffi_from_imports(
        snap.entry_imports(),
        &entry_dir,
        find_workspace_root(entry).as_deref(),
    );
    // Tests are rebuilt every run — take the dev profile's build
    // latency win; the exit-code contract doesn't time anything.
    options.dev_profile = true;
    // The check, with the build's rules. The rename table reaches the
    // analysis through the snapshot's bundle: without it a cross-seed
    // call is an unresolved edge, so an effect assertion violated one
    // seed away compiles, links and ships — and "it built" must not be
    // weaker than "it checked" on a contract the compiler already knows
    // how to evaluate. Errors fail the test; warnings are not printed.
    let diags = match snap.demand_check() {
        Ok(c) => &c.diags,
        Err(b) => return Err(render_blocked(b, snap.demangler(), file_bases, sources)),
    };
    if diags.iter().any(|d| d.is_error()) {
        let mut msg = String::new();
        for d in diags.iter().filter(|d| d.is_error()) {
            msg.push_str(&render_authors(d, snap.demangler(), file_bases, sources));
            msg.push('\n');
        }
        return Err(msg.trim_end().to_string());
    }
    // GH #1009: files compile concurrently now, so the path must be
    // unique per build and not merely per file name. The pid keeps
    // two `hale test` processes apart; the process-local counter
    // keeps two builds of this one apart structurally (the hash of
    // the entry path alone could, in principle, collide). The `.o`
    // (and `.ll`/`.bc` under the dump knobs) codegen writes beside
    // it is derived from this path, so it is unique too.
    static TEST_BIN_NONCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let nonce = TEST_BIN_NONCE.fetch_add(1, Ordering::Relaxed);
    let mut h = DefaultHasher::new();
    h.write(entry.display().to_string().as_bytes());
    let bin = scratch.path(&format!("test_{}_{:016x}", nonce, h.finish()));
    let view = snap.demand_lowering().map_err(|b| render_blocked(b, snap.demangler(), file_bases, sources))?;
    options.site_sources = Some(crate::shared::options::site_sources(file_bases, sources));
    if let Err(e) = hale_codegen::build_resolved(view, &bin, &options) {
        // GH #848: the per-fixture failure message is the located
        // rendering `build` prints, so a test that will not compile
        // names the line to open — it used to be the `{:?}` of the
        // error, span struct and all.
        return Err(render_codegen_error(&e, file_bases, sources));
    }
    Ok(bin)
}

/// `-j N` / `--jobs N` / `HALE_TEST_JOBS=N`: a positive worker count.
pub(crate) fn parse_test_jobs(v: &str) -> Result<usize, ()> {
    match v.trim().parse::<usize>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(()),
    }
}

/// Stack for a `hale test` worker thread. The compiler's passes are
/// recursive (descent parser, typechecker, lowering) and a spawned
/// thread's default is 2 MiB, well under the main thread's (8 MiB
/// under the usual `ulimit -s`) that `hale build` compiles on; a large
/// program that builds there must not overflow on a worker. The
/// reservation is virtual — pages are committed only as the stack is
/// used.
pub(crate) const TEST_WORKER_STACK: usize = 256 << 20;

/// GH #1009: compile and run every file, up to `jobs` at once, and
/// hand the outcomes back in `files` order — the order the report
/// prints them in, whatever order the workers finished in.
///
/// Each worker takes the next index off a shared counter, so a slow
/// file holds up one worker, not the queue. What a worker touches is
/// its own: the test binary's path is unique per build (pid + counter,
/// `compile_test_binary`), codegen's intermediates derive from it, the
/// runtime-object cache writes through a unique temp name and an
/// atomic rename, and each test's stdout and stderr come back through
/// its own pipes (`Command::output`) into its own `TestOutcome` —
/// nothing a test prints reaches the terminal, so two tests cannot
/// interleave there. The runner changes no process-wide state: no
/// `set_current_dir`, no `set_var`; every child inherits the cwd and
/// the environment `hale test` was started with, as it did serially.
///
/// `jobs == 1` runs on the calling thread, one file after another —
/// exactly the loop this replaced.
pub(crate) fn run_test_files(files: &[PathBuf], jobs: usize) -> Vec<TestOutcome> {
    // One private directory for the whole run: every test binary, and
    // the objects codegen writes beside it, live in it and go with it
    // when it drops — after the last worker is done, on every path out.
    let scratch = match RunScratch::new("test") {
        Ok(s) => s,
        Err(e) => {
            return files
                .iter()
                .map(|f| TestOutcome {
                    file: f.to_path_buf(),
                    passed: false,
                    message: Some(e.clone()),
                    elapsed_ms: 0,
                })
                .collect();
        }
    };
    let scratch = &scratch;
    let jobs = jobs.min(files.len()).max(1);
    if jobs == 1 {
        return files
            .iter()
            .map(|f| run_one_test_file(f, scratch))
            .collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<TestOutcome>>> =
        files.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            std::thread::Builder::new()
                .name("hale-test-worker".to_string())
                .stack_size(TEST_WORKER_STACK)
                .spawn_scoped(s, || loop {
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    let Some(f) = files.get(idx) else { break };
                    let outcome = run_one_test_file(f, scratch);
                    *slots[idx].lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(outcome);
                })
                .expect("spawn a hale test worker thread");
        }
    });
    // A worker that panicked re-raises at the end of the scope above,
    // as the panic did on the serial loop, so every slot is filled here.
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("every test file has an outcome")
        })
        .collect()
}

/// A fresh vault directory for one test file's run (mode 700), under
/// `<tmp>/hale-test-vaults-<uid>/<pid>-<n>`: this user's root, this
/// process, a counter, so parallel files never share one. The root is
/// refused when it is not a directory this user owns (another user's, a
/// planted symlink), so a test's secrets never go where someone else can
/// reach them. The first call sweeps the vaults of processes that are
/// gone (a run killed before it removed its own).
pub(crate) fn test_vault_dir() -> Result<PathBuf, String> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let uid = 0;
    let root = test_vault_root();
    let _ = std::fs::create_dir(&root);
    let meta = std::fs::symlink_metadata(&root).map_err(|e| format!("test vault root {}: {e}", root.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if !meta.is_dir() || meta.uid() != uid {
            return Err(format!("test vault root {} is not a directory of this user's", root.display()));
        }
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("test vault root {}: {e}", root.display()))?;
    }
    #[cfg(not(unix))]
    let _ = meta;
    if n == 0 {
        let _ = sweep_dead_test_vaults(&root);
    }
    let dir = root.join(format!("{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).map_err(|e| format!("test vault {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("test vault {}: {e}", dir.display()))?;
    }
    Ok(dir)
}

/// The root every test vault sits under, this user's alone:
/// `<tmp>/hale-test-vaults-<uid>`. `hale test`'s vaults and the Rust
/// tests' (`support/vault.rs`) share it.
pub(crate) fn test_vault_root() -> PathBuf {
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let uid = 0;
    std::env::temp_dir().join(format!("hale-test-vaults-{uid}"))
}

/// Remove each `<pid>` / `<pid>-<n>` vault under `root` whose process
/// no longer runs. The Rust tests' `support/vault.rs` keeps its vaults
/// under the same root, one per test module and process, and sweeps them
/// through this one (`hale habitat sweep --test-vaults`). GH #1653: a
/// vault's habitat comes down first, and a vault whose habitat does not
/// is kept, its projects named in it, for the next sweep to retry. A
/// habitat whose vault is already gone (a toolchain before GH #1653
/// removes a gone process's vault without taking its habitat down, and
/// they share the root) is found by its compose working directory, under
/// a gone process's vault, and taken down by its project's name. `Err`
/// names what was kept.
pub(crate) fn sweep_dead_test_vaults(root: &Path) -> Result<(), String> {
    let listed = crate::dna::habitat::compose_projects_listed()?;
    sweep_dead_test_vaults_with(root, &listed, &|pid| process_gone(pid), &mut |dir| {
        if dir.exists() {
            habitat_down(dir)
        } else {
            crate::dna::habitat::down_under(dir)
        }
    })
}

/// Whether no process has `pid`.
fn process_gone(pid: i32) -> bool {
    #[cfg(unix)]
    return pid > 0 && unsafe { libc::kill(pid, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    #[cfg(not(unix))]
    return false;
}

/// The sweep over `root`, with docker's `listed` compose projects
/// (`<project>|<working dir>` lines), `gone` telling a gone process's pid
/// and `down` taking a vault's habitat down.
fn sweep_dead_test_vaults_with(root: &Path, listed: &str, gone: &dyn Fn(i32) -> bool, down: &mut dyn FnMut(&Path) -> Result<(), String>) -> Result<(), String> {
    let pid_of = |name: &str| name.split('-').next().unwrap_or("").parse::<i32>().ok();
    let mut vaults: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|entries| entries.flatten().filter(|e| pid_of(&e.file_name().to_string_lossy()).is_some_and(gone)).map(|e| e.path()).collect())
        .unwrap_or_default();
    // the habitats whose vault went before they came down
    let under = format!("{}/", root.display());
    for (_, wd) in listed.lines().filter_map(|l| l.split_once('|')) {
        let Some(rest) = wd.strip_prefix(&under) else { continue };
        let name = rest.split('/').next().unwrap_or("");
        let vault = root.join(name);
        if !name.is_empty() && pid_of(name).is_some_and(gone) && !vaults.contains(&vault) {
            vaults.push(vault);
        }
    }
    vaults.sort();
    let mut kept = Vec::new();
    for vault in vaults {
        match down(&vault) {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&vault);
            }
            Err(why) => kept.push(format!("{} is kept for the next sweep: {why}", vault.display())),
        }
    }
    if kept.is_empty() { Ok(()) } else { Err(kept.join("\n")) }
}

/// The local habitat a test file's programs run under (GH #1653): its
/// `XDG_DATA_HOME`, inside the file's vault directory, so an organism a
/// fixture births or migrates is a tenant of the file's own habitat and
/// never of the developer's, and it comes down with the vault.
pub(crate) fn test_habitat_data(vault: &Path) -> PathBuf {
    vault.join("data")
}

/// Take down every compose project run from under `dir` (a test file's
/// habitat lives there), as `hale habitat sweep` does: the one teardown
/// the CLI tests' reap and `scripts/compose-down-under.sh` share, which
/// names the projects under `dir` until they are confirmed gone and never
/// touches the developer's habitat. `Err` names what survived, and the
/// caller keeps `dir`.
pub(crate) fn habitat_down(dir: &Path) -> Result<(), String> {
    // a file whose programs started nothing in docker has nothing to ask
    if !test_habitat_data(dir).exists() && !dir.join(crate::dna::habitat::COMPOSE_FILE).exists() {
        return Ok(());
    }
    crate::dna::habitat::down_under(dir)
}

/// Compile and run one `_test.hl` file and judge it by the
/// `spec/testing.md` contract.
pub(crate) fn run_one_test_file(f: &Path, scratch: &RunScratch) -> TestOutcome {
    let start = std::time::Instant::now();
    let (passed, message) = match compile_test_binary(f, scratch) {
        Err(diag) => (false, Some(diag)),
        Ok(bin) => {
            let mut cmd = std::process::Command::new(&bin);
            // A test that builds a program (a child it signals, a
            // tool it drives) builds it with THIS toolchain, not
            // whichever `hale` PATH finds first; a caller's own
            // HALE_BIN wins (GH #1039).
            if std::env::var_os("HALE_BIN").is_none() {
                if let Ok(me) = std::env::current_exe() {
                    cmd.env("HALE_BIN", me);
                }
            }
            // Each test file runs with a vault of its own, made empty for
            // the run and removed after it: a test that provisions a secret,
            // or a fixture that runs `hale dna init` (which draws the
            // organism's), never writes into the developer's vault, and no
            // test reaches a real vault (`HALE_VAULT_ADDR`). Everything the
            // test starts inherits it.
            // `HALE_TEST_KEEP_VAULT=1` keeps it, and says where.
            let vault = match test_vault_dir() {
                Ok(v) => v,
                Err(why) => {
                    let _ = std::fs::remove_file(&bin);
                    return TestOutcome {
                        file: f.to_path_buf(),
                        passed: false,
                        message: Some(format!("no vault for the test: {why}")),
                        elapsed_ms: start.elapsed().as_millis(),
                    };
                }
            };
            cmd.env("HALE_VAULT_DIR", &vault).env_remove("HALE_VAULT_ADDR");
            // GH #1653: and its own local habitat, beside the vault: a
            // fixture's organisms are never tenants of the developer's
            cmd.env("XDG_DATA_HOME", test_habitat_data(&vault));
            let output = cmd.output();
            let _ = std::fs::remove_file(&bin);
            if std::env::var("HALE_TEST_KEEP_VAULT").is_ok_and(|v| v == "1") {
                eprintln!("hale test: {}'s vault and habitat are kept at {}", f.display(), vault.display());
            } else {
                match habitat_down(&vault) {
                    Ok(()) => {
                        let _ = std::fs::remove_dir_all(&vault);
                    }
                    Err(why) => eprintln!("hale test: {}: {why}; {} is kept for the next run's sweep", f.display(), vault.display()),
                }
            }
            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    // spec/testing.md: pass = exit 0 AND empty stdout.
                    if out.status.success() && out.stdout.is_empty() {
                        (true, None)
                    } else {
                        let mut m = String::new();
                        let body = stdout.trim_end();
                        if !body.is_empty() {
                            m.push_str(body);
                        }
                        if !out.status.success() {
                            if !m.is_empty() {
                                m.push('\n');
                            }
                            match out.status.code() {
                                Some(c) => {
                                    m.push_str(&format!("(exited with code {})", c))
                                }
                                None => m.push_str("(terminated by signal)"),
                            }
                        } else if !body.is_empty() {
                            // Exit 0 but produced output — a passing
                            // test must be silent (spec contract).
                            m = format!(
                                "test exited 0 but produced stdout \
                                 (a passing test must be silent):\n{}",
                                body
                            );
                        }
                        (false, Some(m))
                    }
                }
                Err(e) => {
                    (false, Some(format!("could not execute compiled test: {}", e)))
                }
            }
        }
    };
    TestOutcome {
        file: f.to_path_buf(),
        passed,
        message,
        elapsed_ms: start.elapsed().as_millis(),
    }
}

/// `hale test [file | dir] [-run <substr>] [--json]`.
///
/// Discovers `*_test.hl` files, compiles+runs each as an ordinary
/// Hale binary, and reports per the `spec/testing.md` exit-code
/// contract: PASS iff the process exits 0 with empty stdout; any
/// other outcome (nonzero exit, stdout, or a compile error) is a
/// FAIL. Exits SUCCESS when every test passes, `1` when any fails.
pub(crate) fn run_test(args: &[String]) -> ExitCode {
    let mut target: Option<PathBuf> = None;
    let mut run_filter: Option<String> = None;
    let mut json = false;
    let mut jobs: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        // Accept the spec's single-dash `-run` and the CLI's
        // `--`-convention `--run`, in both space- and `=`-separated
        // forms.
        if a == "-run" || a == "--run" {
            match args.get(i + 1) {
                Some(v) => {
                    run_filter = Some(v.clone());
                    i += 2;
                }
                None => {
                    eprintln!("hale test: {} requires a substring argument", a);
                    return ExitCode::from(2);
                }
            }
        } else if let Some(v) = a.strip_prefix("-run=").or_else(|| a.strip_prefix("--run=")) {
            run_filter = Some(v.to_string());
            i += 1;
        } else if a == "--json" {
            json = true;
            i += 1;
        } else if a == "-j" || a == "--jobs" {
            match args.get(i + 1).map(|v| parse_test_jobs(v)) {
                Some(Ok(n)) => {
                    jobs = Some(n);
                    i += 2;
                }
                Some(Err(())) => {
                    eprintln!(
                        "hale test: {} takes a positive integer, got `{}`",
                        a,
                        args[i + 1]
                    );
                    return ExitCode::from(2);
                }
                None => {
                    eprintln!("hale test: {} requires a job count", a);
                    return ExitCode::from(2);
                }
            }
        } else if let Some(v) = a
            .strip_prefix("--jobs=")
            .or_else(|| a.strip_prefix("-j="))
            .or_else(|| a.strip_prefix("-j").filter(|v| !v.is_empty()))
        {
            match parse_test_jobs(v) {
                Ok(n) => jobs = Some(n),
                Err(()) => {
                    eprintln!(
                        "hale test: -j/--jobs takes a positive integer, got `{}`",
                        v
                    );
                    return ExitCode::from(2);
                }
            }
            i += 1;
        } else if a.starts_with('-') {
            eprintln!("hale test: unknown flag `{}`", a);
            return ExitCode::from(2);
        } else if target.is_none() {
            target = Some(PathBuf::from(a));
            i += 1;
        } else {
            eprintln!("hale test: unexpected extra argument `{}`", a);
            return ExitCode::from(2);
        }
    }
    let target = target
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    // GH #1009: `-j` wins, then `HALE_TEST_JOBS`, then one worker per
    // available core.
    let jobs = match jobs {
        Some(n) => n,
        None => match env::var("HALE_TEST_JOBS") {
            Ok(v) if !v.is_empty() => match parse_test_jobs(&v) {
                Ok(n) => n,
                Err(()) => {
                    eprintln!(
                        "hale test: HALE_TEST_JOBS takes a positive integer, got `{}`",
                        v
                    );
                    return ExitCode::from(2);
                }
            },
            _ => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
        },
    };

    let mut files: Vec<PathBuf> = Vec::new();
    if let Err(e) = collect_test_files(&target, &mut files) {
        eprintln!("hale test: {}", e);
        return ExitCode::from(2);
    }
    files.sort();
    files.dedup();
    if let Some(sub) = &run_filter {
        files.retain(|f| f.to_string_lossy().contains(sub.as_str()));
    }

    if files.is_empty() {
        if json {
            println!("[]");
        } else if let Some(sub) = &run_filter {
            println!(
                "no `_test.hl` files matching `{}` under {}",
                sub,
                target.display()
            );
        } else {
            println!("no `_test.hl` files found under {}", target.display());
        }
        // Nothing to run is not an error.
        return ExitCode::SUCCESS;
    }

    let outcomes = run_test_files(&files, jobs);

    let passed = outcomes.iter().filter(|o| o.passed).count();
    let failed = outcomes.len() - passed;

    if json {
        let mut buf = String::from("[");
        for (idx, o) in outcomes.iter().enumerate() {
            if idx > 0 {
                buf.push(',');
            }
            // GH #867: the row's `file` is spelled by the rule every
            // `--json` record's `file` is spelled by — canonical,
            // absolute, symlinks resolved, no `..` (GH #822). The row
            // says which test ran rather than where an error is, but
            // a tool joining these rows to `check --json` records on
            // `file` needs the two to agree, and this one carried the
            // command line's spelling verbatim. The PASS/FAIL lines
            // below keep that spelling: a human reads them beside the
            // command they just typed.
            buf.push_str(&format!(
                "{{\"file\":\"{}\",\"status\":\"{}\"",
                json_escape(&diag_file_name(&o.file)),
                if o.passed { "pass" } else { "fail" }
            ));
            if let Some(m) = &o.message {
                buf.push_str(&format!(",\"message\":\"{}\"", json_escape(m)));
            }
            buf.push_str(&format!(",\"elapsed_ms\":{}}}", o.elapsed_ms));
        }
        buf.push(']');
        println!("{}", buf);
    } else {
        for o in &outcomes {
            if o.passed {
                println!("ok   {}", o.file.display());
            } else {
                println!("FAIL {}", o.file.display());
                if let Some(m) = &o.message {
                    for line in m.lines() {
                        println!("     {}", line);
                    }
                }
            }
        }
        println!();
        println!("{} passed, {} failed", passed, failed);
    }

    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod habitat_sweep_tests {
    use super::*;

    /// GH #1653: a gone process's vault whose habitat did not come down is
    /// kept for the next sweep, and goes once it does.
    #[test]
    fn a_vault_whose_habitat_survives_is_kept_for_the_next_sweep() {
        let root = std::env::temp_dir().join(format!("hale_vault_sweep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // no process has this pid: the vault is a gone one's
        let vault = root.join("2147483646-0");
        std::fs::create_dir_all(vault.join("data")).unwrap();
        let gone = |pid: i32| pid == 2147483646;
        assert!(sweep_dead_test_vaults_with(&root, "", &gone, &mut |_| Err("compose project p still has volumes after its teardown".into())).is_err());
        assert!(vault.exists(), "kept while its habitat survives");
        let mut asked = Vec::new();
        let swept = sweep_dead_test_vaults_with(&root, "", &gone, &mut |d| {
            asked.push(d.to_path_buf());
            Ok(())
        });
        assert!(swept.is_ok());
        assert_eq!(asked, vec![vault.clone()], "retried by the next sweep");
        assert!(!vault.exists(), "and removed once it came down");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// GH #1653: a habitat whose vault was removed before it came down (by
    /// a toolchain that did not take it down first) is found by its
    /// compose working directory under a gone process's vault, and only
    /// then: a live process's, or one outside the root, is left alone.
    #[test]
    fn a_habitat_whose_vault_is_gone_is_found_by_its_working_directory() {
        let root = std::env::temp_dir().join(format!("hale_vault_orphans_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let r = root.display();
        let listed = format!(
            "hale-habitat-local-aaaa|{r}/2147483646-dna_projects.dna_new.vault/data/hale/habitat/local/allocations/e1e8\n\
             hale-habitat-local-aaaa|{r}/2147483646-dna_projects.dna_new.vault/data/hale/habitat/local/allocations/f2f9\n\
             hale-habitat-local-bbbb|{r}/2147483645-0/data/hale/habitat/local/allocations/0000\n\
             hale-habitat-local-cccc|/elsewhere/2147483646-0/data\n\
             hale-habitat-local-dddd|{r}/not-a-pid/data\n"
        );
        let gone = |pid: i32| pid == 2147483646;
        let mut asked = Vec::new();
        let swept = sweep_dead_test_vaults_with(&root, &listed, &gone, &mut |d| {
            asked.push(d.to_path_buf());
            std::fs::create_dir_all(d).unwrap();
            Ok(())
        });
        assert!(swept.is_ok());
        assert_eq!(asked, vec![root.join("2147483646-dna_projects.dna_new.vault")], "the gone process's habitat, once");
        assert!(!root.join("2147483646-dna_projects.dna_new.vault").exists(), "and nothing of it is left");
        let _ = std::fs::remove_dir_all(&root);
    }
}
