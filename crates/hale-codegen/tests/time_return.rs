//! A fn that returns `Time` lowers like one that returns `Duration`.
//!
//! `Time` is a point over `Duration` (U4, GH #1415), an `i64` count.
//! `declare_locus_methods` listed `CodegenTy::Time` with the
//! pointer-returned types, so a locus method (or mode) returning one
//! was declared `ptr`; a call that read the result as an integer —
//! `println(c.start())`, `c.start() == Time(3ns)` — panicked inkwell
//! ("Found PointerValue ... but expected the IntValue variant"). Free
//! fns were declared right and never panicked. Each shape builds and
//! runs here under `LOTUS_ASAN=1` as well (`support/sanitize.rs`), so
//! a stray pointer-sized return would also show as a sanitizer report.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/sanitize.rs"]
mod sanitize;

fn run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("lotus_test_time_return_{}", name));
    build_opts::build_source(src, &bin, &sanitize::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "{name}: non-zero exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status,
    );
    stdout
}

const CLOCK: &str = r#"
    type E { kind: String; }
    locus Clock {
        params { base: Time = Time(3ns); }
        fn at(d: Duration) -> Time { return self.base + d; }
        fn start() -> Time { return self.base; }
        fn checked(ok: Bool) -> Time fallible(E) {
            if !ok { fail E { kind: "no" }; }
            return self.base + 1s;
        }
        mode bulk() -> Time { return self.base; }
    }
"#;

#[test]
fn a_method_returning_time_is_printed() {
    let out = run(
        "method",
        &format!("{CLOCK}\nfn main() {{ let c = Clock {{ }}; println(c.at(5s)); println(c.start()); }}"),
    );
    assert_eq!(
        out,
        "1970-01-01T00:00:05.000000003Z\n1970-01-01T00:00:00.000000003Z\n"
    );
}

#[test]
fn a_free_fn_returning_time_is_printed() {
    let out = run(
        "free",
        "fn later(t: Time, d: Duration) -> Time { return t + d; }\n\
         fn main() { println(later(Time(1ns), 5s)); }",
    );
    assert_eq!(out, "1970-01-01T00:00:05.000000001Z\n");
}

#[test]
fn a_fallible_method_returning_time_takes_both_paths() {
    let out = run(
        "fallible",
        &format!(
            "{CLOCK}\nfn main() {{ let c = Clock {{ }}; \
             println(c.checked(true) or Time(0ns)); \
             println(c.checked(false) or Time(0ns)); }}"
        ),
    );
    assert_eq!(
        out,
        "1970-01-01T00:00:01.000000003Z\n1970-01-01T00:00:00Z\n"
    );
}

#[test]
fn a_mode_returning_time_is_compared() {
    let out = run(
        "mode",
        &format!(
            "{CLOCK}\nfn main() {{ let c = Clock {{ }}; \
             println(c.bulk() == Time(3ns)); }}"
        ),
    );
    assert_eq!(out, "true\n");
}
