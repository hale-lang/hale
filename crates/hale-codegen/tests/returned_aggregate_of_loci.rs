//! A locus a fn builds and hands back inside a tuple or an array
//! (`fn both(w: W) -> (H, W) { return (H { w: w }, w); }`) escapes the
//! fn's frame as one handed back whole does, so it is built where the
//! caller can still read it. It was a stack slot of the callee's: the
//! caller read `self.p.0.w.name` out of a dead frame — `(null)` in an
//! ordinary build, a SEGV in `strlen` under the sanitizer most runs,
//! once something reused the stack.
//!
//! Each shape is built with AddressSanitizer and run ten times: the
//! failure was intermittent.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn program(decl: &str, params: &str, run: &str) -> String {
    format!(
        r#"locus W {{ params {{ name: String = ""; }} }}
locus H {{ params {{ w: W; }} }}
locus K {{ params {{ a: H; b: H; }} }}
{decl}
main locus App {{
    params {{ {params} }}
    run() {{ {run} }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

#[test]
fn a_locus_handed_back_inside_an_aggregate_outlives_its_builder() {
    let shapes = [
        (
            "fn both(w: W) -> (H, W) { return (H { w: w }, w); }",
            r#"p: (H, W) = both(W { name: std::str::upper("both") });"#,
            "println(self.p.0.w.name); println(self.p.1.name);",
            "BOTH\nBOTH\n",
        ),
        (
            "fn both(w: W) -> (W, H) { return (w, H { w: w }); }",
            r#"p: (W, H) = both(W { name: std::str::upper("rev") });"#,
            "println(self.p.1.w.name); println(self.p.0.name);",
            "REV\nREV\n",
        ),
        (
            "fn two(w: W) -> (H, H) { return (H { w: w }, H { w: w }); }",
            r#"p: (H, H) = two(W { name: std::str::upper("two") });"#,
            "println(self.p.0.w.name); println(self.p.1.w.name);",
            "TWO\nTWO\n",
        ),
        (
            "fn kk(w: W) -> K { return K { a: H { w: w }, b: H { w: w } }; }",
            r#"k: K = kk(W { name: std::str::upper("kk") });"#,
            "println(self.k.a.w.name); println(self.k.b.w.name);",
            "KK\nKK\n",
        ),
        (
            "fn arr(w: W) -> [H; 2] { return [H { w: w }, H { w: w }]; }",
            r#"hs: [H; 2] = arr(W { name: std::str::upper("arr") });"#,
            "println(self.hs[0].w.name); println(self.hs[1].w.name);",
            "ARR\nARR\n",
        ),
    ];
    for (decl, params, run, want) in shapes {
        let src = program(decl, params, run);
        let bin = harness::unique_bin("hale_returned_aggregate_of_loci");
        let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
        build_opts::build_source(&src, &bin, &options).unwrap_or_else(|e| panic!("{decl}: build: {e:?}"));
        for i in 0..10 {
            let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
            let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && !stderr.contains("Sanitizer"), "{decl}, run {i}: {stdout}\n{stderr}");
            assert_eq!(stdout, want, "{decl}, run {i}");
        }
        let _ = std::fs::remove_file(&bin);
    }
}

/// A locus handed back through a fn value (a builder taken as a value,
/// and a fn handing back what a fn value builds) is the caller's: held
/// by a field it is torn down with the holder, bound or unbound it is
/// the frame's. It was nobody's, and LeakSanitizer found its arena.
#[test]
fn a_locus_built_through_a_fn_value_is_owned_and_reclaimed() {
    let src = r#"locus W { params { name: String = "w"; } dissolve() { println(f"bye {self.name}"); } }
locus R { params { w: W; } dissolve() { println("bye r"); } }
fn mk() -> R { return R { w: W { name: "r" } }; }
fn mk_w(n: String) -> W { return W { name: n }; }
fn via(f: fn() -> R) -> R { return f(); }
fn local() {
    let g: fn(String) -> W = mk_w;
    let a = g("bound");
    println(g("temporary").name);
}
main locus App {
    params { r: R = via(mk); }
    run() { local(); println("APP DONE"); }
}
fn main() { App { }; }
"#;
    let bin = harness::unique_bin("hale_fn_value_owned");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && !stderr.contains("Sanitizer"), "{stdout}\n{stderr}");
    // `R` and the `W` it holds both say "bye r", torn down with `App`
    for (who, n) in [("bye bound", 1), ("bye temporary", 1), ("bye r", 2)] {
        assert_eq!(stdout.matches(who).count(), n, "{who}: {stdout}");
    }
}

/// GH #1569: a locus a fn hands back inside a tuple or an array is
/// owned by whatever holds the aggregate — a field holding it, as a
/// tuple or array param built from a literal holds its elements: torn
/// down with the holder, once each, in element order; an argument a
/// part keeps goes with the last part keeping it. Under an unplaced, a
/// pool-placed and a pinned holder, sanitized, the five shapes.
#[test]
fn a_locus_handed_back_inside_an_aggregate_is_its_holders() {
    let shapes: [(&str, &str, &str, &[&str]); 5] = [
        (
            "fn both(w: W) -> (H, W) { return (H { w: w }, w); }",
            r#"p: (H, W) = both(W { name: std::str::upper("both") });"#,
            "println(self.p.0.w.name);",
            &["H.dissolve h", "W.dissolve BOTH"],
        ),
        (
            "fn rev(w: W) -> (W, H) { return (w, H { w: w }); }",
            r#"p: (W, H) = rev(W { name: std::str::upper("rev") });"#,
            "println(self.p.1.w.name);",
            &["W.dissolve REV", "H.dissolve h"],
        ),
        (
            r#"fn two(w: W) -> (H, H) { return (H { w: w, tag: "h0" }, H { w: w, tag: "h1" }); }"#,
            r#"p: (H, H) = two(W { name: std::str::upper("two") });"#,
            "println(self.p.1.w.name);",
            &["H.dissolve h0", "H.dissolve h1", "W.dissolve TWO"],
        ),
        (
            r#"fn kk(w: W) -> K { return K { a: H { w: w, tag: "ka" }, b: H { w: w, tag: "kb" } }; }"#,
            r#"k: K = kk(W { name: std::str::upper("kk") });"#,
            "println(self.k.b.w.name);",
            &["K.dissolve", "H.dissolve ka", "H.dissolve kb", "W.dissolve KK"],
        ),
        (
            r#"fn arr(w: W) -> [H; 2] { return [H { w: w, tag: "a0" }, H { w: w, tag: "a1" }]; }"#,
            r#"hs: [H; 2] = arr(W { name: std::str::upper("arr") });"#,
            "println(self.hs[1].w.name);",
            &["H.dissolve a0", "H.dissolve a1", "W.dissolve ARR"],
        ),
    ];
    for placement in ["", "placement { q: cooperative(pool = io); }", "placement { q: pinned; }"] {
        for (decl, params, run, torn) in shapes {
            let src = format!(
                r#"locus W {{ params {{ name: String = ""; }} dissolve() {{ println(f"W.dissolve {{self.name}}"); }} }}
locus H {{ params {{ w: W; tag: String = "h"; }} dissolve() {{ println(f"H.dissolve {{self.tag}}"); }} }}
locus K {{ params {{ a: H; b: H; }} dissolve() {{ println("K.dissolve"); }} }}
{decl}
locus Hold {{
    params {{ {params} }}
    run() {{ {run} }}
    dissolve() {{ println("Hold.dissolve"); }}
}}
main locus App {{
    params {{ q: Hold = Hold {{ }}; }}
    {placement}
    run() {{ std::time::sleep(50ms); println("APP DONE"); }}
}}
fn main() {{ App {{ }}; }}
"#
            );
            let what = format!("{decl} [{placement}]");
            let bin = harness::unique_bin("hale_aggregate_holder");
            let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
            build_opts::build_source(&src, &bin, &options).unwrap_or_else(|e| panic!("{what}: build: {e:?}"));
            let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
            let _ = std::fs::remove_file(&bin);
            let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && !stderr.contains("Sanitizer"), "{what}: {stdout}\n{stderr}");
            let dissolves: Vec<&str> = stdout.lines().filter(|l| l.contains(".dissolve") && !l.starts_with("Hold")).collect();
            assert_eq!(dissolves, torn, "{what}: {stdout}");
        }
    }
}

/// GH #1569: one a binding holds is the frame's — torn down at its flush
/// (the parts in reverse creation order, then the argument), once each;
/// in a loop, each iteration's before the next builds its own. A
/// discarded call's are torn down at the statement, the parts in order,
/// then the argument they kept.
#[test]
fn a_locus_handed_back_inside_an_aggregate_to_a_frame_is_the_frames() {
    let src = r#"locus W { params { name: String = ""; } dissolve() { println(f"W.dissolve {self.name}"); } }
locus H { params { w: W; tag: String = "h"; } dissolve() { println(f"H.dissolve {self.tag}"); } }
fn both(w: W) -> (H, W) { return (H { w: w }, w); }
fn two(w: W) -> (H, H) { return (H { w: w, tag: "h0" }, H { w: w, tag: "h1" }); }
fn bound() { let t = both(W { name: "let" }); println(t.0.w.name); }
fn dropped() { two(W { name: "drop" }); println("dropped"); }
@unbounded
fn looped() { let mut i = 0; while i < 3 { let t = two(W { name: "loop" }); i = i + 1; } println("looped"); }
fn main() { bound(); dropped(); looped(); println("end"); }
"#;
    let bin = harness::unique_bin("hale_aggregate_frame");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && !stderr.contains("Sanitizer"), "{stdout}\n{stderr}");
    let loop_turn = "H.dissolve h0\nH.dissolve h1\nW.dissolve loop\n";
    let want = format!(
        "let\nH.dissolve h\nW.dissolve let\nH.dissolve h0\nH.dissolve h1\nW.dissolve drop\ndropped\n{loop_turn}{loop_turn}looped\nH.dissolve h1\nH.dissolve h0\nW.dissolve loop\nend\n"
    );
    assert_eq!(stdout, want);
}

/// GH #1569: a discarded call that builds the locus it hands back — a
/// fresh factory called by name, or through a fn value (only a fresh
/// factory may be one) — is reclaimed at the statement, in `main` and in
/// a method alike. It was nobody's: LeakSanitizer found its arena, and
/// 200k calls of a `W` that writes its arena at birth grew RSS by ~980
/// MB (~4.9 KB a call). What stays is the instance's own struct, which a
/// fn handing back a locus builds in the program-lifetime payload arena
/// (a reclaim may still be pending on it after the statement): ~170
/// bytes a call here, under the budget of 512.
#[test]
fn a_discarded_factory_result_is_reclaimed_at_the_statement() {
    let traced = r#"locus W { params { name: String = "w"; } dissolve() { println(f"W.dissolve {self.name}"); } }
fn mkw(n: String) -> W { return W { name: n }; }
fn pick() -> fn(String) -> W { return mkw; }
main locus App {
    run() {
        mkw("named");
        let f = mkw;
        f("value");
        let g = pick();
        g("picked");
        println("APP DONE");
    }
}
fn main() { mkw("main"); App { }; }
"#;
    let bin = harness::unique_bin("hale_discarded_factory");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(traced, &bin, &options).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && !stderr.contains("Sanitizer"), "{stdout}\n{stderr}");
    assert_eq!(stdout, "W.dissolve main\nW.dissolve named\nW.dissolve value\nW.dissolve picked\nAPP DONE\n");

    let churn = |n: u32| {
        format!(
            r#"locus W {{ params {{ name: String = "w"; }} birth() {{ self.name = f"{{self.name}} born"; }} dissolve() {{ }} }}
fn mkw(n: Int) -> W {{ return W {{ name: f"made {{n}}" }}; }}
main locus App {{
    @unbounded
    fn churn(n: Int) {{
        let f = mkw;
        let mut i = 0;
        while i < n {{ mkw(i); f(i); i = i + 1; }}
    }}
    run() {{
        self.churn(1000);
        print("base_statm=");
        println(std::io::fs::read_file("/proc/self/statm") or "");
        self.churn({n});
        print("grown_statm=");
        println(std::io::fs::read_file("/proc/self/statm") or "");
    }}
}}
fn main() {{ App {{ }}; }}
"#
        )
    };
    let src = churn(100_000);
    let bin = harness::unique_bin("hale_discarded_factory_rss");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let read = |key: &str| -> i64 {
        let line = stdout.lines().find_map(|l| l.strip_prefix(key)).unwrap_or_else(|| panic!("missing {key}: {stdout}"));
        harness::statm_resident_bytes(line)
    };
    let grown = read("grown_statm=") - read("base_statm=");
    let per_call = grown / 200_000;
    assert!(per_call <= 512, "rss grew {grown} bytes over 200k discarded factory calls ({per_call} a call)");
}
