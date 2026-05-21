//! `std::process::rss_bytes() -> Int` — observability primitive
//! backed by `getrusage(RUSAGE_SELF)`. Returns the peak resident
//! set size in bytes. Useful for fathom-style daemons that want
//! to assert their memory pressure stays bounded (and is the
//! workaround for read_file-of-/proc/self/statm returning empty
//! because synthesized files report st_size=0).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_std_process_rss_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn rss_bytes_returns_nonzero_int() {
    // A process that has just started still has some RSS — at
    // minimum the binary's code segment. getrusage is reliable
    // enough that we can assert > 0 without flakes.
    let src = r#"
        fn main() {
            let rss = std::process::rss_bytes();
            if rss > 0 {
                println("ok");
            } else {
                println("zero");
            }
        }
    "#;
    let (stdout, status) = build_and_run("nonzero", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("ok"), "rss_bytes returned <= 0: {:?}", stdout);
}

#[test]
fn rss_bytes_grows_with_allocations() {
    // Allocate a non-trivial amount of memory, then re-read rss.
    // The post-alloc reading should be >= the initial reading.
    // (Peak RSS is monotonic by definition; this also exercises
    // that the primitive returns a real measurement.)
    let src = r#"
        fn main() {
            let r0 = std::process::rss_bytes();
            // Force a sizeable allocation that survives to the
            // second measurement. 64 KiB per concat × 100 = ~6.4
            // MB resident if the allocator commits eagerly.
            let mut held = "seed";
            let mut i = 0;
            while i < 100 {
                let big = std::str::repeat("x", 65536);
                held = held + big;
                i = i + 1;
            }
            let r1 = std::process::rss_bytes();
            // Avoid emitting the bytes themselves (would confuse
            // the assertion below); just check ordering.
            if r1 >= r0 {
                println("nondecreasing");
            } else {
                println("decreased");
            }
            // Use `held` so it doesn't get optimized away.
            println("held_len=", to_string(len(held)));
        }
    "#;
    let (stdout, status) = build_and_run("grows", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("nondecreasing"),
        "rss_bytes went down (peak RSS shouldn't); stdout: {:?}",
        stdout,
    );
}
