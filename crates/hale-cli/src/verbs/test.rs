use std::hash::Hasher;
use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::path::Path;
use std::path::PathBuf;
use hale_syntax::ast::Program;
use crate::shared::process::RunScratch;
use crate::shared::options::collect_ffi_from_imports;
use crate::shared::diag::diag_file_name;
use std::env;
use std::fs;
use crate::shared::diag::json_escape;
use crate::shared::frontend::parse_with_imports;
use crate::shared::source::Disk;
use crate::shared::diag::render_codegen_error;
use crate::shared::diag::render_located;
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
/// the `Err` message. Mirrors `run_program`'s single-file pipeline
/// (parse_with_imports → check_bundle_opts → build) but stops at
/// the binary so the caller can `.output()`-capture the run.
pub(crate) fn compile_test_binary(
    entry: &Path,
    scratch: &RunScratch,
) -> Result<PathBuf, String> {
    let (mut program, renames, sources, file_bases, ctx) = match parse_with_imports(entry, &Disk) {
        Ok(x) => x,
        Err(errors) => {
            let mut msg = String::new();
            for e in &errors {
                msg.push_str(&e.render());
                msg.push('\n');
            }
            return Err(msg.trim_end().to_string());
        }
    };
    // F.40 phase 1.1b-iii: the snapshot. The file entry runs no desugar
    // before the check, so it mints straight after the load, seeded by
    // the source map `check` mints with.
    let entry_name = entry.display().to_string();
    let source_map = crate::shared::frontend::source_map(entry, &file_bases, &sources);
    let snapshot = hale_types::snapshot::mint([(entry_name.as_str(), &mut program)], &source_map);
    let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
    bundle_programs.insert(entry_name.clone(), &program);
    // The rename table must reach the analysis here too, not only in
    // `check`. Without it a cross-seed call is an unresolved edge, so
    // an effect assertion violated one seed away compiles, links and
    // ships — a downstream fleet gates on `build` across 109 binaries,
    // and "it built" must not be weaker than "it checked" on a
    // contract the compiler already knows how to evaluate.
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames.clone();
    // The map the snapshot minted with, as `check` hands it over.
    bundle.sources = source_map.clone();
    bundle.snapshot = snapshot;
    let diags = hale_types::check_bundle_for_build(&bundle, false);
    if diags.iter().any(|d| d.is_error()) {
        let mut msg = String::new();
        for d in diags.iter().filter(|d| d.is_error()) {
            msg.push_str(&render_located(d, &file_bases, &sources));
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
    // Stage-2 FFI pickup, same as `hale build` (2026-07-18; closes
    // pond FRICTION "hale test cannot link @ffi libs"): a test that
    // imports an FFI-bearing lib (sqlite et al.) needs the lib's
    // hale.toml [ffi] link/csrc surface on the link line, or every
    // such test dies with undefined lotus_* references regardless
    // of the test's own correctness.
    let mut options = collect_ffi_from_imports(
        &ctx.imports,
        &ctx.entry_dir,
        ctx.workspace_root.as_deref(),
    );
    // Tests are rebuilt every run — take the dev profile's build
    // latency win; the exit-code contract doesn't time anything.
    options.dev_profile = true;
    if let Err(e) = hale_types::resolved::resolve_program(
        &program,
        &source_map,
        &renames,
        options.api.as_deref(),
        options.api_roles.as_deref(),
    )
    .map_err(hale_codegen::CodegenError::Unsupported)
    .and_then(|resolved| {
        hale_codegen::build_resolved(resolved, &bin, &options)
    }) {
        // GH #848: the per-fixture failure message is the located
        // rendering `build` prints, so a test that will not compile
        // names the line to open — it used to be the `{:?}` of the
        // error, span struct and all.
        return Err(render_codegen_error(&e, &file_bases, &sources));
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
    let root = std::env::temp_dir().join(format!("hale-test-vaults-{uid}"));
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
        sweep_dead_test_vaults(&root);
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

/// Remove each `<pid>` / `<pid>-<n>` vault under `root` whose process
/// no longer runs. The Rust tests' `support/vault.rs` keeps its vaults
/// under the same root, one per test module and process, and sweeps them
/// the same way.
pub(crate) fn sweep_dead_test_vaults(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(pid) = name.split('-').next().unwrap_or("").parse::<i32>() else { continue };
        #[cfg(unix)]
        let gone = pid > 0
            && unsafe { libc::kill(pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        #[cfg(not(unix))]
        let gone = false;
        if gone {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
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
            let output = cmd.output();
            let _ = std::fs::remove_file(&bin);
            if std::env::var("HALE_TEST_KEEP_VAULT").is_ok_and(|v| v == "1") {
                eprintln!("hale test: {}'s vault is kept at {}", f.display(), vault.display());
            } else {
                let _ = std::fs::remove_dir_all(&vault);
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
