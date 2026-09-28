//! Shared test-harness primitives: collision-proof temp paths and
//! ports.
//!
//! ## Why this exists
//!
//! Nearly every codegen test compiles a `.hl` program to a native
//! binary and runs it, and each test file grew its own copy of that
//! helper — 115 declarations, 104 of them textually distinct. The
//! variation was almost entirely in *where the binary is written*,
//! and 131 files picked a path with no uniquifier at all, most of
//! them `temp_dir()/lotus_test_{name}`. Eleven files shared that
//! exact template; nine shared `lotus_{name}`.
//!
//! Nothing made those distinct. It worked only because the `name`
//! arguments happened not to overlap — one `build_and_run("basic", …)`
//! in the wrong file and two tests write and exec the same path,
//! which is `ETXTBSY` ("text file busy") under any parallel runner.
//!
//! That latent hazard is why `CLAUDE.md` mandated `--test-threads=1`.
//! The CI workflow, meanwhile, claimed nextest's process-per-test
//! made the shared paths safe — which is not true: process isolation
//! is not filesystem isolation, and two processes writing one path
//! are *more* concurrent than two threads, not less. Two documents
//! disagreed and neither described the real situation.
//!
//! [`unique_bin`] removes the hazard structurally rather than by
//! convention, and `harness_paths_are_unique.rs` fails the build if a
//! test reintroduces a hand-rolled temp path.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "build.rs"]
mod build_opts;

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Build `program` to `bin` with the PRE-optimization LLVM IR dumped
/// beside it, and hand back the IR text. The `.ll` is removed; the
/// binary is left in place, because several callers also run it.
///
/// ## Why this exists (GH #843)
///
/// Every IR-shape test in this suite used to wrap its build in
/// `std::env::set_var("LOTUS_DUMP_IR", "1")` / `remove_var`. The
/// environment is a process-global, and mutating it while another
/// thread reads it is undefined behavior — which under `cargo test`
/// (libtest runs tests as *threads* in one process) is every
/// concurrent build in the same binary, each of which reads
/// `LOTUS_DUMP_IR`, `LOTUS_LTO`, `LOTUS_ASAN` and friends. Worse
/// than the UB, the effect was *visible*: whichever test set the var
/// turned the dump on for every build racing it, and whichever test
/// removed it first turned the dump off under a test that was still
/// building — so an IR assertion could fail on a missing `.ll` with
/// nothing wrong in the compiler. Only nextest's process-per-test
/// hid it.
///
/// `BuildOptions::dump_ir` asks for the same dump through the API,
/// so the request is scoped to one build.
#[allow(dead_code)]
pub fn build_ir_text(
    program: &hale_syntax::ast::Program,
    bin: &Path,
) -> Result<String, hale_codegen::CodegenError> {
    let ll = bin.with_extension("ll");
    let options = hale_codegen::BuildOptions {
        dump_ir: Some(ll.clone()),
        ..build_opts::options()
    };
    hale_codegen::build_executable_with_options(program, bin, &[], &options)?;
    let text = std::fs::read_to_string(&ll)
        .expect("BuildOptions::dump_ir should have written the .ll");
    let _ = std::fs::remove_file(&ll);
    Ok(text)
}

/// Build `program` to `bin` with AddressSanitizer instrumentation
/// (`BuildOptions::asan`), and check the artifact really carries it.
///
/// The check is not ceremony. Every ASan test in this suite asserts
/// the *absence* of leak / UAF / overflow text in the run's output,
/// so a build that quietly came out uninstrumented passes all of
/// them — which is exactly what a change to *how* the knob is
/// requested could introduce without any test going red (GH #843,
/// where the knob moved off `LOTUS_ASAN` in the process
/// environment). An instrumented binary links the ASan runtime, so
/// its symbols are in the image.
#[allow(dead_code)]
pub fn build_asan(program: &hale_syntax::ast::Program, bin: &Path) {
    let options = hale_codegen::BuildOptions {
        asan: true,
        ..build_opts::options()
    };
    hale_codegen::build_executable_with_options(program, bin, &[], &options)
        .expect("asan build");
    const MARKER: &[u8] = b"__asan_init";
    let image = std::fs::read(bin).expect("read the built binary");
    assert!(
        image.windows(MARKER.len()).any(|w| w == MARKER),
        "the build was asked for AddressSanitizer but {} carries no \
         ASan runtime symbols — every `nothing was reported` \
         assertion downstream would pass vacuously",
        bin.display()
    );
}

/// Resident bytes out of a `/proc/self/statm` line — the memory a
/// compiled test program actually holds, as *it* reports it.
///
/// The program under test prints the raw line, which costs it one
/// statement and no helper:
///
/// ```text
/// print("rss_statm="); println(std::io::fs::read_file("/proc/self/statm") or "");
/// ```
///
/// and the test does the arithmetic here. `statm`'s second field is
/// resident pages; a 4096-byte page is assumed, as everywhere else
/// in this suite (Linux CI; the macOS job builds the workspace and
/// smoke-tests two programs, it does not run these tests). Panics
/// with the offending text if the line is missing or malformed —
/// a memory test that silently stops measuring is worse than one
/// that is red.
///
/// ## Why not `std::process::rss_bytes()` (GH #772)
///
/// `std::process::rss_bytes()` is `getrusage(RUSAGE_SELF).ru_maxrss`,
/// and that number is **not** the program's own. `fork()` gives the
/// child a copy-on-write duplicate of the parent's address space, so
/// the pre-exec child's RSS high-water mark is the *parent's* RSS;
/// `execve` then folds that mark into the new image's `ru_maxrss`
/// (`exec_mmap()` → `setmax_mm_hiwater_rss(&tsk->signal->maxrss,
/// old_mm)`) where it stays for the life of the process. A program
/// spawned by a test harness therefore reports at least the
/// harness's RSS forever after.
///
/// Measured with one unmodified binary: 4.7 MB standalone, 411 MB
/// when spawned from a parent holding 400 MB. Under `cargo test`
/// the parent is a libtest process running several in-process LLVM
/// builds at once, which is why memory assertions in this suite
/// moved with machine load (#772: 100 MB bound, 137–145 MB
/// observed) and why *relative* assertions between two spawned runs
/// collapse — both readings clamp to the same harness floor, so a
/// gap test goes red and a "stays flat" test goes vacuously green.
///
/// `/proc/self/statm` is read from the new image's `mm`, so it
/// carries none of that history. Note it is *current* RSS, not a
/// high-water mark: have the program print it while the memory
/// being measured is still live.
#[allow(dead_code)]
pub fn statm_resident_bytes(line: &str) -> i64 {
    let value = line.trim();
    let pages: i64 = value
        .split_whitespace()
        .nth(1)
        .unwrap_or_else(|| {
            panic!(
                "/proc/self/statm has no second field — the program \
                 could not read its own residency: {:?}",
                line
            )
        })
        .parse()
        .unwrap_or_else(|e| {
            panic!("statm resident field is not a number ({e}): {line:?}")
        });
    pages * 4096
}

/// A binary path no other test can collide with, in this process or
/// any other.
///
/// Three components, each covering a case the others don't:
///   * the caller's `name` — keeps the path readable when a test
///     leaves one behind;
///   * the **pid** — separates concurrent test *processes*, which is
///     what nextest actually gives us;
///   * a process-local **counter** — separates tests within one
///     process, which the pid alone does not (libtest threads).
///
/// In an area binary every test file includes this module for itself,
/// so there is one copy per file (`<area>::<file>::harness`) and each
/// copy counts from 0: two files asking for `unique_bin("basic")` would
/// draw the same path. There the file's module name joins the path. A
/// binary of one file (`<file>::harness`) keeps the plain shape.
pub fn unique_bin(name: &str) -> PathBuf {
    let module: Vec<&str> = module_path!().split("::").collect();
    let file = if module.len() >= 3 { format!("{}_", module[module.len() - 2]) } else { String::new() };
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale_t_{}{}_{}_{}",
        file,
        name,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    p
}
