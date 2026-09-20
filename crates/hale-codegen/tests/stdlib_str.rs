//! m78: std::str — minimal string parsing primitives.
//!
//! The parse_int / parse_decimal behaviour tests moved to
//! `tests/hale/str_parse_*_test.hl`, written in Hale and run by
//! `hale test`. Two reasons, both concrete rather than stylistic:
//!
//!  * the expectation stops being transcribed. It was
//!    `stdout.contains("a=42")` here; it is
//!    `assert_eq_int(parse_int("42"), 42)` there — which is also
//!    STRICTER, since `contains("a=42")` passes on `a=421`.
//!  * `build_executable` (what these tests call) does NOT run the
//!    typechecker, so the programs here were compiled and run but
//!    never checked. `hale test` checks them. That gap is not
//!    hypothetical: the `err.kind` shape these tests exercise did
//!    not typecheck at all, and moving them found it.
//!
//! What stays here is what genuinely needs the Rust side: the two
//! DIAGNOSTIC tests, which assert on compiler output rather than
//! program behaviour — and, since GH #720, the ByteView SCALING
//! test, which needs a clock and megabytes of input (the behaviour
//! of the view lives in `tests/hale/str_byte_view_test.hl`).

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_stdlib_str_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn std_str_parse_error_qualified_path_resolves() {
    // v1.x polish (2026-05-20): `std::str::ParseError` resolves
    // to the same struct the stdlib's parse_* fns inject. Lets
    // users disambiguate explicitly in fn signatures and `as e`
    // bindings — useful when a project also has its own local
    // error types.
    let src = r#"
        fn handle(e: std::str::ParseError) -> Int {
            println("kind=", e.kind);
            println("input=", e.input);
            return -1;
        }
        fn main() {
            let v = std::str::parse_int("nope") or handle(err);
            println("v=", v);
        }
    "#;
    let (stdout, status) = build_and_run("qualified_path", src);
    assert!(status.success(), "build/run failed: {:?}", stdout);
    assert!(
        stdout.contains("kind=parse_int") && stdout.contains("input=nope"),
        "expected qualified-path handler to see stdlib ParseError fields, \
         got stdout: {:?}",
        stdout
    );
}

#[test]
fn std_str_parse_user_parse_error_collision_diagnoses_cleanly() {
    // v1.x polish (2026-05-20): when a user declares their own
    // `type ParseError` with non-stdlib-compatible fields, the
    // codegen previously panicked with `ParseError.kind field`.
    // Now it returns a clean diagnostic naming the fix paths.
    let src = r#"
        type ParseError { msg: String; venue: String; }
        fn handle(e: ParseError) -> Int { let _ = e; return -1; }
        fn main() {
            let v = std::str::parse_int("nope") or handle(err);
            let _ = v;
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_stdlib_str_collision");
    let result = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    let err = match result {
        Err(e) => e,
        Ok(()) => panic!("expected codegen failure, but build succeeded"),
    };
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("user-declared `type ParseError`")
            && msg.contains("std::str::ParseError"),
        "expected clean diag naming the qualified-path fix, got: {}",
        msg
    );
}

/// GH #720 — a ByteView scan is LINEAR in the input, not quadratic.
///
/// What this replaces was quadratic for a structural reason: a
/// checked one-byte slice `s[i..(i + 1)]` clamps its range with its
/// own `strlen`, so a bounded loop over an n-byte String did O(n)
/// work per byte. Measured on this machine at 1 MiB: 13.2 s for the
/// naive loop, 7.7 s with the length hoisted into a local (the
/// slice's own strlen is the dominant term, not `len`), against
/// ~29 us for the view.
///
/// The bite is therefore the absolute bound as much as the ratio:
/// the naive loop at 4 MiB would run for minutes, so a regression
/// that reintroduced a per-access strlen could not hide under the
/// 5-second cap. The ratio check is what catches a subtler one — a
/// per-access cost that is sublinear but not constant.
///
/// The program times each size nine times and reports the fastest,
/// so a scheduler hiccup inflates a measurement rather than the
/// ratio — CI runs this binary alongside every other one. It
/// deliberately does NOT run the quadratic loop at 4 MiB.
#[test]
fn byte_view_scan_scales_linearly() {
    let src = r#"
        fn scan(s: String) -> Int {
            let v = std::str::bytes_view(s);
            let mut count = 0;
            let mut i = 0;
            while i < v.n {
                let c = std::str::byte_at(v, i);
                if c == 101 { count = count + 1; }
                i = i + 1;
            }
            return count;
        }
        fn best_of_nine(s: String, label: String) {
            let mut best = -1;
            let mut counted = 0;
            let mut k = 0;
            while k < 9 {
                let t0 = std::time::monotonic_ns();
                let c = scan(s);
                let t1 = std::time::monotonic_ns();
                let ns = t1 - t0;
                counted = c;
                if best < 0 || ns < best { best = ns; }
                k = k + 1;
            }
            println(label, " count=", counted, " ns=", best);
        }
        fn main() {
            let one = std::str::repeat("abcdefgh", 1048576);
            let two = std::str::repeat("abcdefgh", 2097152);
            let four = std::str::repeat("abcdefgh", 4194304);
            best_of_nine(one, "sz8");
            best_of_nine(two, "sz16");
            best_of_nine(four, "sz32");
        }
    "#;
    let (stdout, status) = build_and_run("byte_view_scaling", src);
    assert!(status.success(), "build/run failed: {:?}", stdout);

    let read = |label: &str, key: &str| -> i64 {
        let line = stdout
            .lines()
            .find(|l| l.starts_with(label))
            .unwrap_or_else(|| panic!("no {} line in:\n{}", label, stdout));
        let after = line
            .split(key)
            .nth(1)
            .unwrap_or_else(|| panic!("no {} in {:?}", key, line));
        after
            .split_whitespace()
            .next()
            .unwrap_or_else(|| panic!("empty {} in {:?}", key, line))
            .parse()
            .unwrap_or_else(|e| panic!("bad {} in {:?}: {}", key, line, e))
    };

    // Every byte was actually visited — 'e' is one byte in eight.
    assert_eq!(read("sz8", "count="), 1048576, "8 MiB scan visited 8 MiB");
    assert_eq!(read("sz16", "count="), 2097152, "16 MiB scan visited 16 MiB");
    assert_eq!(read("sz32", "count="), 4194304, "32 MiB scan visited 32 MiB");

    let ns1 = read("sz8", "ns=");
    let ns2 = read("sz16", "ns=");
    let ns4 = read("sz32", "ns=");
    assert!(
        ns1 > 0 && ns2 > 0 && ns4 > 0,
        "monotonic clock gave a non-positive span: {} / {} / {}",
        ns1,
        ns2,
        ns4
    );

    // Absolute cap: four megabytes of safe byte access is
    // microseconds of work. Five seconds is ~40,000x the measured
    // cost and still far below the minutes a quadratic scan needs.
    assert!(
        ns4 < 5_000_000_000,
        "32 MiB scan took {} ns — a linear scan is a millisecond, so this \
         is the quadratic shape #720 reported",
        ns4
    );

    // Shape: 4x the input for under 10x the time. Measured 3.9x
    // locally (29 / 57 / 114 us at 1/2/4 MiB); the quadratic form is
    // 16x or worse. The sizes are 8/16/32 MiB so the fastest-of-nine
    // is hundreds of microseconds and a shared CI runner's jitter is a
    // small fraction of it — the first cut at 1/2/4 MiB and 6x read
    // 7.1x on one runner and passed on the next. The point of the
    // bound is to separate O(n) from O(n^2), not to police a constant
    // factor.
    assert!(
        ns4 < ns1 * 10,
        "32 MiB ({} ns) should cost under 10x 8 MiB ({} ns) — 16 MiB \
         was {} ns; a per-access cost that grows with the input is back",
        ns4,
        ns1,
        ns2
    );
}
