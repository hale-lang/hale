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
//! of the view lives in `tests/hale/str_byte_view_test.hl`), with the
//! text `for` sources' (GH #1995, behaviour in
//! `tests/hale/str_split_sources_test.hl`).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let bin = harness::unique_bin(&format!("hale_test_stdlib_str_{}", name));
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
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
    let bin = harness::unique_bin("hale_test_stdlib_str_collision");
    let result = build_opts::build_source(src, &bin, &build_opts::options());
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

/// The scaling tests below share one way of timing (GH #2115).
///
/// Each guards a quadratic shape that came back as an O(n) cost per
/// element: a `strlen`, a copy of the rest of the text. A count of
/// allocations or copied bytes would see a copy and miss a `strlen`, and
/// `std::diag::heap_alloc_count` answers -1 on macOS, so they measure
/// time, held so that a loaded runner cannot fake a ratio:
///
///  * the time is the process's CPU time, libc's `clock()` (microseconds:
///    POSIX fixes `CLOCKS_PER_SEC` at 1,000,000), not the wall clock. A
///    runner with more work than cores takes the CPU away from a long
///    sample far more often than from a short one, which faked the 10x
///    of #2115 on a 0.6 ms base; it does not add to the CPU time;
///  * the cases run in rounds, every case once per round, so what load
///    still costs (caches, memory bandwidth, the clock rate) lands on the
///    small and the large size of a pair alike;
///  * a sample is `reps` calls back to back, milliseconds of work, so the
///    clock's microsecond is noise far below the bound;
///  * a pair's growth is the median, over the rounds, of the large
///    sample's time per call over the small one's, so no one round
///    decides it;
///  * the bound stays where it separates the shapes: 4x the input is 4x
///    the time when linear and 16x when quadratic, held under 10x.
///
/// A sample that passes a second stops its repeats, and a round that
/// passes three the rounds, so a regression costs seconds, not minutes.
/// Each sample prints `<label> got=<answer> ns=<total> reps=<calls>`;
/// `work(kind, s, n)` is the test's own dispatch.
const SAMPLE_HL: &str = r#"
    @ffi("c") fn clock() -> Int;
    fn sample(kind: Int, s: String, n: Int, reps: Int, label: String) -> Int {
        let mut got = 0;
        let mut done = 0;
        let mut ns = 0;
        let t0 = clock();
        while done < reps && ns < 1000000000 {
            got = work(kind, s, n);
            done = done + 1;
            ns = (clock() - t0) * 1000;
        }
        println(label, " got=", got, " ns=", ns, " reps=", done);
        return ns;
    }
"#;

/// The samples a [`SAMPLE_HL`] program printed.
struct Samples {
    stdout: String,
}

impl Samples {
    fn run(name: &str, program: &str) -> Samples {
        let (stdout, status) = build_and_run(name, &format!("{SAMPLE_HL}\n{program}"));
        assert!(status.success(), "build/run failed: {:?}", stdout);
        Samples { stdout }
    }

    /// `(got, ns, reps)` of each of `label`'s samples, in round order.
    fn of(&self, label: &str) -> Vec<(i64, i64, i64)> {
        let field = |line: &str, key: &str| -> i64 {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(key))
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("no {key} in {line:?}"))
        };
        let rows: Vec<_> = self
            .stdout
            .lines()
            .filter(|l| l.split_whitespace().next() == Some(label))
            .map(|l| (field(l, "got="), field(l, "ns="), field(l, "reps=")))
            .collect();
        assert!(!rows.is_empty(), "no {label} sample in:\n{}", self.stdout);
        rows
    }

    /// What every one of `label`'s calls answered.
    fn got(&self, label: &str) -> i64 {
        let rows = self.of(label);
        let got = rows[0].0;
        assert!(rows.iter().all(|r| r.0 == got), "{label} answered differently across rounds: {rows:?}");
        got
    }

    /// `label`'s time per call in each round, ns.
    fn per_call(&self, label: &str) -> Vec<f64> {
        self.of(label).iter().map(|&(_, ns, reps)| ns.max(1) as f64 / reps.max(1) as f64).collect()
    }

    /// `label`'s median time per call, ns.
    fn median(&self, label: &str) -> f64 {
        median(self.per_call(label))
    }

    /// The median over the rounds of `large`'s time per call over
    /// `small`'s.
    fn growth(&self, small: &str, large: &str) -> f64 {
        let (s, l) = (self.per_call(small), self.per_call(large));
        median(s.iter().zip(&l).map(|(s, l)| l / s).collect())
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    assert!(!v.is_empty());
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let m = v.len() / 2;
    if v.len() % 2 == 1 { v[m] } else { (v[m - 1] + v[m]) / 2.0 }
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
/// Timed as [`SAMPLE_HL`] says: 8, 16 and 32 MiB, four scans a sample,
/// nine rounds. It deliberately does NOT run the quadratic loop at
/// 4 MiB.
#[test]
fn byte_view_scan_scales_linearly() {
    let s = Samples::run(
        "byte_view_scaling",
        r#"
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
        fn work(kind: Int, s: String, n: Int) -> Int { return scan(s); }
        fn main() {
            let one = std::str::repeat("abcdefgh", 1048576);
            let two = std::str::repeat("abcdefgh", 2097152);
            let four = std::str::repeat("abcdefgh", 4194304);
            let mut k = 0;
            while k < 9 {
                let mut round = sample(0, one, 0, 4, "sz8");
                round = round + sample(0, two, 0, 4, "sz16");
                round = round + sample(0, four, 0, 4, "sz32");
                k = if round > 3000000000 { 9 } else { k + 1 };
            }
        }
    "#,
    );

    // Every byte was actually visited — 'e' is one byte in eight.
    assert_eq!(s.got("sz8"), 1048576, "8 MiB scan visited 8 MiB");
    assert_eq!(s.got("sz16"), 2097152, "16 MiB scan visited 16 MiB");
    assert_eq!(s.got("sz32"), 4194304, "32 MiB scan visited 32 MiB");

    let (ns1, ns2, ns4) = (s.median("sz8"), s.median("sz16"), s.median("sz32"));
    let growth = s.growth("sz8", "sz32");
    println!("byte view 8 MiB {ns1:.0} ns, 16 MiB {ns2:.0} ns, 32 MiB {ns4:.0} ns ({growth:.1}x)");

    // Absolute cap: 32 MiB of safe byte access is milliseconds of work.
    // Five seconds is hundreds of times the measured cost and still far
    // below the minutes a quadratic scan needs.
    assert!(
        ns4 < 5e9,
        "32 MiB scan took {ns4:.0} ns — a linear scan is milliseconds, so this \
         is the quadratic shape #720 reported"
    );

    // Shape: 4x the input for under 10x the time. Measured on an M1:
    // 1.6 / 3.3 / 6.6 ms a scan at 8 / 16 / 32 MiB (4.0x); the quadratic
    // form is 16x or worse. The point of the bound is
    // to separate O(n) from O(n^2), not to police a constant factor.
    assert!(
        growth < 10.0,
        "32 MiB ({ns4:.0} ns) should cost under 10x 8 MiB ({ns1:.0} ns), took {growth:.1}x — 16 MiB \
         was {ns2:.0} ns; a per-access cost that grows with the input is back"
    );
}

/// `std::str::find` over a view pays the distance to each hit, so a
/// line-at-a-time scan is linear in the text, where `index_of_from`,
/// which measures the whole String on every call, made it quadratic
/// (16 MB in 32,000 lines took 4.7 s; this takes about 3 ms). Timed as
/// [`SAMPLE_HL`] says: 4 and 16 MB, eight scans a sample.
#[test]
fn a_find_line_scan_scales_linearly() {
    let s = Samples::run(
        "find_line_scan_scaling",
        r#"
        fn lines(s: String) -> Int {
            let v = std::str::bytes_view(s);
            let mut k = 0;
            let mut nl = std::str::find(v, "\n", 0);
            while nl >= 0 { k = k + 1; nl = std::str::find(v, "\n", nl + 1); }
            return k;
        }
        fn work(kind: Int, s: String, n: Int) -> Int { return lines(s); }
        fn main() {
            let row = std::str::repeat("x", 499) + "\n";
            let small = std::str::repeat(row, 8192);
            let large = std::str::repeat(row, 32768);
            let mut k = 0;
            while k < 9 {
                let round = sample(0, small, 0, 8, "sz4") + sample(0, large, 0, 8, "sz16");
                k = if round > 3000000000 { 9 } else { k + 1 };
            }
        }
    "#,
    );
    assert_eq!(s.got("sz4"), 8192, "every line of 4 MB was found");
    assert_eq!(s.got("sz16"), 32768, "every line of 16 MB was found");
    let (ns4, ns16) = (s.median("sz4"), s.median("sz16"));
    let growth = s.growth("sz4", "sz16");
    println!("find lines 4 MB {ns4:.0} ns, 16 MB {ns16:.0} ns ({growth:.1}x)");
    // Quadratic, 4x the text is 16x the time, and 16 MB took seconds.
    assert!(ns16 < 1e9, "a 16 MB line scan took {ns16:.0} ns: the quadratic shape");
    assert!(growth < 10.0, "4x the text took {growth:.1}x the time ({ns4:.0} -> {ns16:.0} ns): not linear");
}

/// GH #1995 S2: `for w in std::str::words(s)` is one scan, linear in the
/// text, where the hand loop it replaces is quadratic: each turn of
/// `while len(rest) > 0 { … rest = rest[(sp + 1)..len(rest)]; }` measures
/// the rest of the text and copies it, so 4x the text is 16x the work
/// (and the copies stay in the frame's arena until it returns).
///
/// Ratios, not absolute times, timed as [`SAMPLE_HL`] says: each form at
/// two sizes 4x apart, its growth held under 10x (4x is linear, 16x
/// quadratic) and under the hand loop's, measured in the same rounds. The
/// source runs at 1 and 4 MiB, the hand loop at 256 KiB and 1 MiB, so
/// the hand loop's copies stay near 128 MiB a call; words are 4 KiB for
/// the same reason. Measured on an M1, per call: the source 0.65 /
/// 2.6 ms (4.0x), the hand loop 0.54 / 7.5 ms (13.6x).
///
/// A chain over the source (GH #1995 S2) is held to the same bound, both
/// a counting one and `max`, whose element-valued terminal keeps the best
/// part's offsets rather than the part.
#[test]
fn words_source_scales_linearly_where_the_hand_loop_does_not() {
    let s = Samples::run(
        "words_source_scaling",
        r#"
        fn hand(s: String) -> Int {
            let mut rest = s;
            let mut n = 0;
            while len(rest) > 0 {
                let sp = std::str::index_of(rest, " ");
                let w = if sp < 0 { rest } else { rest[0..sp] };
                if len(w) > 0 { n = n + 1; }
                if sp < 0 { rest = ""; } else { rest = rest[(sp + 1)..len(rest)]; }
            }
            return n;
        }
        fn source(s: String) -> Int {
            let mut n = 0;
            for w in std::str::words(s) {
                if len(w) > 0 { n = n + 1; }
            }
            return n;
        }
        // GH #1995 S2, chain sources: the same scan as a chain, and the
        // element-valued terminal that keeps the best part's offsets
        fn chain(s: String) -> Int {
            return std::str::words(s).filter(len(it) > 0).count();
        }
        fn longest(s: String) -> Int {
            let w = std::str::words(s).max(len(it)) or "";
            return len(w);
        }
        fn work(kind: Int, s: String, n: Int) -> Int {
            return if kind == 1 { hand(s) } else if kind == 2 { chain(s) } else if kind == 3 { longest(s) } else { source(s) };
        }
        fn main() {
            let word = std::str::repeat("x", 4095) + " ";
            let quarter = std::str::repeat(word, 64);
            let one = std::str::repeat(word, 256);
            let four = std::str::repeat(word, 1024);
            let mut k = 0;
            while k < 9 {
                let mut round = sample(0, one, 0, 8, "source1m");
                round = round + sample(0, four, 0, 8, "source4m");
                round = round + sample(1, quarter, 0, 8, "hand256k");
                round = round + sample(1, one, 0, 8, "hand1m");
                round = round + sample(2, one, 0, 8, "chain1m");
                round = round + sample(2, four, 0, 8, "chain4m");
                round = round + sample(3, one, 0, 8, "max1m");
                round = round + sample(3, four, 0, 8, "max4m");
                k = if round > 3000000000 { 9 } else { k + 1 };
            }
        }
    "#,
    );
    assert_eq!(s.got("source1m"), 256, "every word of 1 MiB");
    assert_eq!(s.got("source4m"), 1024, "every word of 4 MiB");
    assert_eq!(s.got("hand256k"), 64, "the hand loop's words of 256 KiB");
    assert_eq!(s.got("hand1m"), 256, "the hand loop's words of 1 MiB");
    let (s1, s4) = (s.median("source1m"), s.median("source4m"));
    let (h1, h4) = (s.median("hand256k"), s.median("hand1m"));
    let source_growth = s.growth("source1m", "source4m");
    let hand_growth = s.growth("hand256k", "hand1m");
    println!(
        "source 1 MiB {s1:.0} ns, 4 MiB {s4:.0} ns ({source_growth:.1}x); \
         hand 256 KiB {h1:.0} ns, 1 MiB {h4:.0} ns ({hand_growth:.1}x)"
    );
    assert!(
        source_growth < 10.0,
        "4x the text took {source_growth:.1}x the time through `words` ({s1:.0} -> {s4:.0} ns): not linear"
    );
    assert!(
        source_growth < hand_growth,
        "`words` grew {source_growth:.1}x over 4x the text, the quadratic hand loop {hand_growth:.1}x \
         ({h1:.0} -> {h4:.0} ns): the source should grow slower"
    );
    assert!(s1 < h4, "`words` over 1 MiB took {s1:.0} ns, the hand loop {h4:.0} ns: the source should be faster");
    // a chain over the source is the same scan (GH #1995 S2): linear, and
    // `max` keeps offsets, not the parts
    assert_eq!(s.got("chain1m"), 256, "the chain counts every word of 1 MiB");
    assert_eq!(s.got("chain4m"), 1024, "the chain counts every word of 4 MiB");
    assert_eq!(s.got("max1m"), 4095, "max picks a 4095-byte word");
    for label in ["chain", "max"] {
        let (small, large) = (format!("{label}1m"), format!("{label}4m"));
        let (one, four) = (s.median(&small), s.median(&large));
        let growth = s.growth(&small, &large);
        println!("{label} 1 MiB {one:.0} ns, 4 MiB {four:.0} ns ({growth:.1}x)");
        assert!(growth < 10.0, "4x the text took {growth:.1}x the time through a `{label}` chain ({one:.0} -> {four:.0} ns): not linear");
        assert!(growth < hand_growth, "a `{label}` chain grew {growth:.1}x, the quadratic hand loop {hand_growth:.1}x: it should grow slower");
    }
}

/// GH #1995 S2 (review of #2060): `min` and `max` over a text source keep
/// the best part where it lies. A long first word followed by many short
/// ones is the shape that would copy the winner out once per later part
/// (quadratic when the two are the same size); keyed and bare, each runs
/// at two sizes 4x apart, timed as [`SAMPLE_HL`] says, and its growth is
/// held under 10x.
#[test]
fn min_max_over_a_text_source_never_recopy_the_best_part() {
    let s = Samples::run(
        "text_min_max_scaling",
        r#"
        fn keyed(s: String) -> Int {
            let w = std::str::words(s).max(len(it)) or "";
            return len(w);
        }
        fn bare(s: String) -> Int {
            let w = std::str::words(s).max() or "";
            return len(w);
        }
        fn work(kind: Int, s: String, n: Int) -> Int {
            return if kind == 0 { keyed(s) } else { bare(s) };
        }
        fn main() {
            let small = std::str::repeat("x", 131072) + " " + std::str::repeat("a ", 131072);
            let large = std::str::repeat("x", 524288) + " " + std::str::repeat("a ", 524288);
            let mut k = 0;
            while k < 9 {
                let mut round = sample(0, small, 0, 1, "keyed1");
                round = round + sample(0, large, 0, 1, "keyed4");
                round = round + sample(1, small, 0, 1, "bare1");
                round = round + sample(1, large, 0, 1, "bare4");
                k = if round > 3000000000 { 9 } else { k + 1 };
            }
        }
    "#,
    );
    assert_eq!(s.got("keyed1"), 131072, "max(len(it)) picks the long word");
    assert_eq!(s.got("keyed4"), 524288, "max(len(it)) picks the long word");
    assert_eq!(s.got("bare1"), 131072, "max() picks the long word");
    assert_eq!(s.got("bare4"), 524288, "max() picks the long word");
    for label in ["keyed", "bare"] {
        let (small, large) = (format!("{label}1"), format!("{label}4"));
        let (one, four) = (s.median(&small), s.median(&large));
        let growth = s.growth(&small, &large);
        println!("{label}: {one:.0} ns, 4x the text {four:.0} ns ({growth:.1}x)");
        assert!(growth < 10.0, "`{label}` max over 4x the text took {growth:.1}x ({one:.0} -> {four:.0} ns): the best part is copied again per part");
    }
}

/// GH #1947: `len` and slicing on a String local or parameter measure its
/// text once per value it holds, so the byte loop `while i < len(s) {
/// s[i..(i + 1)] … }` and the build loop `while len(out) < n { out += …
/// }` are linear. Both were quadratic: the condition's `strlen` and the
/// slice's clamp walked the whole text on every iteration (13.2 s on
/// 1 MiB, spec/stdlib.md § Linear byte scanning), and so did `len(out)`
/// over a text the appends had grown.
///
/// Ratios, not absolute times, timed as [`SAMPLE_HL`] says: each loop at
/// two sizes 4x apart, its growth held under 10x (4x is linear, 16x
/// quadratic) and under the growth of a control measured in the same
/// rounds: the same byte loop over a String read from a record's field on
/// every use, which keeps no length and still pays the walk. Measured on
/// an M1, per call: the byte loop 1.5 / 6.1 ms at 128 / 512 KiB (4.1x),
/// the build loop 0.29 / 1.1 ms to 256 KiB / 1 MiB (3.9x), the control
/// 12 / 184 ms at 16 / 64 KiB (15.4x).
#[test]
fn len_and_slice_of_a_local_scale_linearly() {
    let s = Samples::run(
        "len_slice_scaling",
        r#"
        type Held { text: String; }
        fn scan(s: String) -> Int {
            let mut count = 0;
            let mut i = 0;
            while i < len(s) {
                let c = s[i..(i + 1)];
                if c == "e" { count = count + 1; }
                i = i + 1;
            }
            return count;
        }
        fn scan_field(h: Held) -> Int {
            let mut count = 0;
            let mut i = 0;
            while i < len(h.text) {
                let c = h.text[i..(i + 1)];
                if c == "e" { count = count + 1; }
                i = i + 1;
            }
            return count;
        }
        fn grow(n: Int) -> Int {
            let mut out = "";
            while len(out) < n {
                out += "abcdefgh";
            }
            return len(out);
        }
        fn work(kind: Int, s: String, n: Int) -> Int {
            return if kind == 0 { scan(s) } else if kind == 1 { scan_field(Held { text: s }) } else { grow(n) };
        }
        fn main() {
            let small = std::str::repeat("abcdefgh", 16384);
            let large = std::str::repeat("abcdefgh", 65536);
            let field_small = std::str::repeat("abcdefgh", 2048);
            let field_large = std::str::repeat("abcdefgh", 8192);
            let mut k = 0;
            while k < 7 {
                let mut round = sample(0, small, 0, 4, "scan128k");
                round = round + sample(0, large, 0, 4, "scan512k");
                round = round + sample(2, "", 262144, 16, "grow256k");
                round = round + sample(2, "", 1048576, 16, "grow1m");
                round = round + sample(1, field_small, 0, 1, "field16k");
                round = round + sample(1, field_large, 0, 1, "field64k");
                k = if round > 3000000000 { 7 } else { k + 1 };
            }
        }
    "#,
    );
    assert_eq!(s.got("scan128k"), 16384, "every byte of 128 KiB was read");
    assert_eq!(s.got("scan512k"), 65536, "every byte of 512 KiB was read");
    assert_eq!(s.got("grow256k"), 262144, "the build stopped at 256 KiB");
    assert_eq!(s.got("grow1m"), 1048576, "the build stopped at 1 MiB");
    assert_eq!(s.got("field16k"), 2048, "the control read every byte of 16 KiB");
    assert_eq!(s.got("field64k"), 8192, "the control read every byte of 64 KiB");
    let growth = |small: &str, large: &str| (s.median(small), s.median(large), s.growth(small, large));
    let (s1, s4, scan_growth) = growth("scan128k", "scan512k");
    let (g1, g4, grow_growth) = growth("grow256k", "grow1m");
    let (f1, f4, field_growth) = growth("field16k", "field64k");
    println!(
        "scan {s1:.0} -> {s4:.0} ns ({scan_growth:.1}x); grow {g1:.0} -> {g4:.0} ns ({grow_growth:.1}x); \
         control {f1:.0} -> {f4:.0} ns ({field_growth:.1}x)"
    );
    assert!(scan_growth < 10.0, "4x the text took {scan_growth:.1}x the time in the byte loop ({s1:.0} -> {s4:.0} ns): not linear");
    assert!(grow_growth < 10.0, "4x the text took {grow_growth:.1}x the time in the build loop ({g1:.0} -> {g4:.0} ns): not linear");
    assert!(
        scan_growth < field_growth && grow_growth < field_growth,
        "the byte loop grew {scan_growth:.1}x and the build loop {grow_growth:.1}x, the control that measures on \
         every use {field_growth:.1}x ({f1:.0} -> {f4:.0} ns): a local's length should be measured once"
    );
}

/// `cp_at` / `cp_size` at an offset past the end of a String answer -1 and
/// read nothing past its terminator. They used to load the byte at the
/// offset whatever it was: past the end of an arena string that is a read
/// out of bounds, which AddressSanitizer reports (or the load faults).
#[test]
fn a_code_point_read_past_the_end_reads_nothing_under_asan() {
    let src = r#"
        fn main() {
            let s = std::str::repeat("h", 3) + "é";
            println(std::str::cp_at(s, 5), " ", std::str::cp_size(s, 5));
            println(std::str::cp_at(s, 6), " ", std::str::cp_size(s, 6));
            println(std::str::cp_at(s, 4000000), " ", std::str::cp_size(s, 4000000));
            println(std::str::cp_at(s, 3), " ", std::str::cp_size(s, 3));
            println(std::str::cp_at(s, 9223372036854775807), " ", std::str::cp_size(s, 1099511627776));
        }
    "#;
    let bin = harness::unique_bin("cp_past_end_asan");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("asan build");
    let out = std::process::Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && !stderr.contains("AddressSanitizer"), "{:?}: {stderr}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "-1 -1\n-1 -1\n-1 -1\n233 2\n-1 -1\n");
}

