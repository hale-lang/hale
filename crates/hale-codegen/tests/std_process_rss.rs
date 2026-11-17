//! `std::process::rss_bytes() -> Int` and
//! `std::process::peak_rss_bytes() -> Int`: the calling process's
//! resident set, current and peak, in bytes.
//!
//! `rss_bytes()` is the CURRENT resident size (`/proc/self/statm` on
//! Linux, mach `task_info` on macOS), the number
//! `hale_process_resident_memory_bytes` scrapes. Until GH #1906 it
//! was getrusage's `ru_maxrss * 1024`: the peak, and 1024x too large
//! on macOS, which reports `ru_maxrss` in bytes.
//!
//! `peak_rss_bytes()` is that high-water mark, with the platform's
//! unit fixed. A peak is not the program's own number when the program
//! was spawned: `execve` folds the pre-exec (fork-inherited,
//! copy-on-write) RSS high-water mark into `ru_maxrss`, so a child
//! reports at least its parent's RSS forever (GH #772; measured 4.7 MB
//! standalone, 411 MB from a parent holding 400 MB). The tests below
//! are spawned by a harness that runs LLVM builds in process, so they
//! only bound the peak from below by the current size, and from above
//! by what a unit error would give.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let bin = harness::unique_bin(&format!(
        "hale_std_process_rss_{}_{}",
        name,
        std::process::id()
    ));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

fn field(stdout: &str, key: &str) -> i64 {
    stdout
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .unwrap_or_else(|| panic!("missing {key} in {stdout:?}"))
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("{key} is not an Int ({e}): {stdout:?}"))
}

#[test]
fn rss_bytes_returns_nonzero_int() {
    // A process that has just started still has some RSS: at least
    // the pages of its own code it has touched.
    let src = r#"
        fn main() {
            print("rss=");
            println(std::process::rss_bytes());
            print("peak=");
            println(std::process::peak_rss_bytes());
        }
    "#;
    let (stdout, status) = build_and_run("nonzero", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(field(&stdout, "rss=") > 0, "rss_bytes returned <= 0: {stdout:?}");
    assert!(field(&stdout, "peak=") > 0, "peak_rss_bytes returned <= 0: {stdout:?}");
}

/// 32 MiB written and still held shows in the current size, the peak
/// is at least the current size, and both are bytes: a peak read in
/// the wrong unit (the macOS bug, KiB arithmetic on a byte count) puts
/// 32 MiB at 32 GiB.
#[test]
fn rss_bytes_is_current_and_peak_is_its_high_water_mark() {
    let src = r#"
        fn main() {
            let r0 = std::process::rss_bytes();
            let held = std::str::repeat("x", 33554432);
            let r1 = std::process::rss_bytes();
            let peak = std::process::peak_rss_bytes();
            print("r0=");
            println(r0);
            print("r1=");
            println(r1);
            print("peak=");
            println(peak);
            // keep `held` live past the readings
            print("held_len=");
            println(len(held));
        }
    "#;
    let (stdout, status) = build_and_run("current", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("held_len=33554432"), "{stdout:?}");
    let (r0, r1, peak) = (field(&stdout, "r0="), field(&stdout, "r1="), field(&stdout, "peak="));
    const MIB: i64 = 1 << 20;
    assert!(
        r1 - r0 >= 24 * MIB,
        "holding 32 MiB grew the current size by only {} KiB (r0={r0}, r1={r1})",
        (r1 - r0) / 1024,
    );
    assert!(r1 < 4096 * MIB, "the current size is not in bytes: {r1}");
    assert!(peak >= r1, "the peak ({peak}) is below the current size ({r1})");
    assert!(peak < 16384 * MIB, "the peak is not in bytes: {peak}");
}

/// A peak never goes down, whatever the program lets go between the
/// readings (here a 32 MiB string a returned call's frame held).
#[test]
fn peak_rss_bytes_never_goes_down() {
    let src = r#"
        fn fill() -> Int {
            let big = std::str::repeat("y", 33554432);
            return len(big);
        }
        fn main() {
            let p0 = std::process::peak_rss_bytes();
            let n = fill();
            let p1 = std::process::peak_rss_bytes();
            print("n=");
            println(n);
            print("p0=");
            println(p0);
            print("p1=");
            println(p1);
        }
    "#;
    let (stdout, status) = build_and_run("peak", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("n=33554432"), "{stdout:?}");
    let (p0, p1) = (field(&stdout, "p0="), field(&stdout, "p1="));
    assert!(p1 >= p0, "peak_rss_bytes went down: {p0} -> {p1}");
}
