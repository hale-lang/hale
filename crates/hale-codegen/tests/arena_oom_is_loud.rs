//! A refused allocation aborts where it happens, and says so.
//!
//! `lotus_arena_alloc` returned NULL when the OS refused it a chunk, and
//! every caller took the NULL for a pointer: the process died later, in
//! a `memcpy` or a store, as a SIGSEGV whose backtrace names code that
//! did nothing wrong (found by GH #1208, where a head under a memory
//! limit "crashed in the JSON reader"). The allocator now aborts at the
//! failing call instead — SIGABRT, and a line on stderr naming the
//! arena, the size asked for and the address of the call — so
//! out-of-memory is reported by the allocation that met it.
//!
//! The NULLs that ARE part of a contract stay NULLs: a `fixed_size` slab
//! that is full, and an arena at its `chunk_byte_cap`, return NULL to
//! callers that route it (spec/memory.md); only the OS refusing memory
//! is fatal.
//!
//! The program is limited by `ulimit -v`, the address-space ceiling, so
//! it is the allocation of a chunk that fails, whatever the machine's RAM.

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

/// Grows one locus's memory without bound: each iteration concatenates
/// onto a string that lives until the method returns.
fn hog(iterations: u64) -> String {
    format!(
        r#"
        locus Hog {{
            params {{ _u: Int = 0; }}
            birth() {{
                let mut s = "";
                let mut i = 0;
                while i < {iterations} {{
                    s = s + "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
                    i = i + 1;
                }}
                println("done ", len(s));
            }}
        }}
        fn main() {{ Hog {{ }}; }}
    "#
    )
}

struct Ran {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

/// Build `src` and run it under a 128 MiB address-space limit. `exec`,
/// so the status is the program's own and not the shell's.
fn run_limited(tag: &str, src: &str) -> Ran {
    let bin = harness::unique_bin(&format!("arena-oom-{tag}"));
    build_opts::build_source(src, &bin, &build_opts::options())
        .expect("build");
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!("ulimit -v 131072; exec {}", bin.display()))
        .output()
        .expect("run under ulimit");
    let _ = std::fs::remove_file(&bin);
    Ran {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[test]
fn a_refused_chunk_aborts_at_the_allocation_and_names_it() {
    let ran = run_limited("hog", &hog(100_000_000));
    assert_eq!(
        ran.status.signal(),
        Some(libc_sigabrt()),
        "a refused allocation must abort (SIGABRT), not run on with NULL \
         (SIGSEGV is 11).\nstatus: {:?}\nstdout: {:?}\nstderr: {:?}",
        ran.status,
        ran.stdout,
        ran.stderr
    );
    let e = &ran.stderr;
    assert!(e.contains("lotus: out of memory"), "names the failure: {e}");
    assert!(e.contains("Hog"), "names the arena, by the locus that owns it: {e}");
    assert!(
        e.contains("requested") && e.contains("bytes"),
        "says how much the failing call asked for: {e}"
    );
    assert!(e.contains("requested by the call at 0x"), "gives the address of the call: {e}");
    // glibc resolves that address to the function that made the call —
    // here the string concatenation the loop is doing.
    #[cfg(target_env = "gnu")]
    assert!(e.contains("lotus_str_concat"), "names the caller: {e}");
}

/// The control: the same program, small enough to fit, is untouched —
/// the abort is the refusal's, not a new limit.
#[test]
fn the_same_program_that_fits_runs_to_the_end() {
    let ran = run_limited("fits", &hog(300));
    assert!(ran.status.success(), "{:?}\n{}", ran.status, ran.stderr);
    assert!(ran.stdout.contains("done 19200"), "{:?}", ran.stdout);
    assert!(!ran.stderr.contains("out of memory"), "{}", ran.stderr);
}

fn libc_sigabrt() -> i32 {
    6
}
