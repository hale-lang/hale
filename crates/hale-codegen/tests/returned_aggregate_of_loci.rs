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
/// MB (~4.9 KB a call). GH #1570: the instance's own struct stayed, built
/// in the program-lifetime payload arena (~170 bytes a call); it now
/// goes back at the end of the instance's reclaim, so 200k discarded
/// calls and 200k `let`-bound ones leave RSS flat (under 8 bytes a call,
/// page noise).
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

    let churn = |n: u32, m: u32| {
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
    @unbounded
    fn bind(n: Int) {{
        let mut i = 0;
        while i < n {{ let w = mkw(i); i = i + 1 + len(w.name) - len(w.name); }}
    }}
    run() {{
        self.churn(1000);
        self.bind(1000);
        print("base_statm=");
        println(std::io::fs::read_file("/proc/self/statm") or "");
        self.churn({n});
        print("grown_statm=");
        println(std::io::fs::read_file("/proc/self/statm") or "");
        self.bind({m});
        print("bound_statm=");
        println(std::io::fs::read_file("/proc/self/statm") or "");
    }}
}}
fn main() {{ App {{ }}; }}
"#
        )
    };
    let src = churn(100_000, 200_000);
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
    assert!(per_call <= 8, "rss grew {grown} bytes over 200k discarded factory calls ({per_call} a call)");
    let bound = read("bound_statm=") - read("grown_statm=");
    let per_bind = bound / 200_000;
    assert!(per_bind <= 8, "rss grew {bound} bytes over 200k let-bound factory calls ({per_bind} a call)");
}

/// GH #1569: a fn value returning a locus is a fresh factory however it
/// was bound — a loop variable over an ascribed array or an array
/// parameter, its ascribed rebinding, a tuple pattern's name, a `let` of
/// an `if` or of a method's call — and each instance a call through one
/// builds is torn down once: a discarded one at the statement, a bound
/// one with its binding (in a loop, before the next iteration builds
/// its own). Sanitized.
#[test]
fn a_fn_value_bound_any_way_builds_instances_owned_once() {
    let src = r#"locus W { params { name: String = "w"; } dissolve() { println(f"W.dissolve {self.name}"); } }
fn mkw() -> W { return W { name: "made" }; }
fn mka() -> W { return W { name: "a" }; }
fn over(fs: [fn() -> W; 2]) { for g in fs { g(); } println("over"); }
@unbounded
fn loops() {
    let fs: [fn() -> W; 2] = [mkw, mka];
    for g in fs { let h: fn() -> W = g; let w = h(); println(f"held {w.name}"); }
    println("loops");
}
fn tuples() {
    let (f, n) = (mkw, 1);
    let w = f();
    let g: fn() -> W = f;
    g();
    println(f"tuple {w.name} {n}");
}
main locus App {
    fn pk() -> fn() -> W { return mka; }
    run() {
        let f = if true { mkw } else { mka };
        let a = f();
        let m = self.pk();
        let b = m();
        m();
        println(f"run {a.name} {b.name}");
    }
}
fn main() { over([mkw, mka]); loops(); tuples(); App { }; }
"#;
    let bin = harness::unique_bin("hale_fn_value_bound_any_way");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && !stderr.contains("Sanitizer"), "{stdout}\n{stderr}");
    assert_eq!(
        stdout,
        "W.dissolve made\nW.dissolve a\nover\n\
         held made\nW.dissolve made\nheld a\nloops\nW.dissolve a\n\
         W.dissolve made\ntuple made 1\nW.dissolve made\n\
         W.dissolve a\nrun made a\nW.dissolve a\nW.dissolve made\n"
    );
}

/// GH #1569: a locus the callee builds and hands back inside a tuple
/// beside parts of other types (`fn mk2() -> (W, Int)`) is owned as a
/// factory's direct return: by the binding (torn down at the frame's
/// flush), in a method and in `main` and through a fn value; at the
/// statement when the result is discarded; by a holder whose param holds
/// it, and so is one written at the holder's literal (`(W { }, 5)`),
/// replaced in place when a method writes the whole tuple. It was
/// nobody's: LeakSanitizer found its arena. Each torn down once,
/// sanitized.
#[test]
fn a_locus_a_callee_builds_inside_a_tuple_is_owned_like_a_factorys() {
    let src = r#"locus W { params { name: String = "w"; } dissolve() { println(f"W.dissolve {self.name}"); } }
fn mk2(n: String) -> (W, Int) { return (W { name: n }, 1); }
fn mkp(n: String) -> (Int, W, W) { return (2, W { name: n }, W { name: f"{n}2" }); }
locus Hold {
    params { p: (W, Int) = mk2("held"); t: (W, Int) = (W { name: "lit" }, 5); }
    fn swap() { self.t = (W { name: "lit2" }, 6); self.t.1 = 7; }
    dissolve() { println(f"Hold.dissolve {self.p.1} {self.t.1}"); }
}
main locus App {
    params { h: Hold = Hold { }; }
    run() {
        let t = mk2("method");
        let f = mk2;
        let u = f("value");
        mk2("dropped");
        f("dropped2");
        let q = mkp("three");
        self.h.swap();
        println(f"run {t.0.name} {u.0.name} {q.1.name} {q.2.name} {t.1}");
    }
}
fn main() { let t = mk2("main"); println(f"main {t.0.name}"); App { }; }
"#;
    let bin = harness::unique_bin("hale_tuple_built_part");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && !stderr.contains("Sanitizer"), "{stdout}\n{stderr}");
    assert_eq!(
        stdout,
        "main main\nW.dissolve dropped\nW.dissolve dropped2\nW.dissolve lit\n\
         run method value three three2 1\nW.dissolve three2\nW.dissolve three\nW.dissolve value\nW.dissolve method\n\
         Hold.dissolve 1 7\nW.dissolve held\nW.dissolve lit2\nW.dissolve main\n"
    );
}

/// GH #1569: a fresh factory is the greatest fixpoint over the calls fns
/// hand back: fns handing back each other's calls, one building at its
/// base case (`cyc1` builds a `W` at 0, else returns `cyc2(..)`, which
/// returns `cyc1(..)`), build every instance they return — a cycle with
/// no way out returns nothing — so either may be taken as a value, and
/// each instance a call builds is torn down once, bound or discarded.
/// Sanitized.
#[test]
fn recursive_fresh_factories_build_what_they_return() {
    let src = r#"locus W { params { name: String = "w"; } dissolve() { println(f"W.dissolve {self.name}"); } }
fn cyc1(n: Int) -> W { if n == 0 { return W { name: f"base{n}" }; } return cyc2(n - 1); }
fn cyc2(n: Int) -> W { return cyc1(n); }
fn main() {
    let f = cyc1;
    let a = f(3);
    f(2);
    let g = cyc2;
    let b = g(1);
    cyc1(4);
    println(f"{a.name} {b.name}");
}
"#;
    let bin = harness::unique_bin("hale_recursive_factories");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && !stderr.contains("Sanitizer"), "{stdout}\n{stderr}");
    assert_eq!(stdout, "W.dissolve base0\nW.dissolve base0\nbase0 base0\nW.dissolve base0\nW.dissolve base0\n");
}

/// GH #1570 review: the released structs are kept a list per size, so a loop
/// that hands back more distinct struct sizes than a scan would reach reuses
/// every one of them. Twelve locus types of twelve sizes, each built by a
/// factory and dropped at the statement, 20k turns: RSS stays flat (a single
/// list scanned to its first eight blocks carved a new struct for the sizes
/// past them every turn). The same program runs clean under ASan with chunk
/// recycling off, where each struct is allocated and freed on its own.
#[test]
fn a_loop_over_many_returned_struct_sizes_reuses_every_size() {
    const KINDS: usize = 12;
    let program = |turns: u32| {
        let mut src = String::new();
        for k in 0..KINDS {
            let fields: Vec<String> = (0..=k).map(|f| format!("f{f}: String = \"x\";")).collect();
            // a birth that writes its arena, as the one-size test's `W` does (a
            // locus with no arena keeps a fresh String field elsewhere)
            src.push_str(&format!(
                "locus W{k} {{ params {{ {} }} birth() {{ self.f0 = f\"{{self.f0}} born\"; }} dissolve() {{ }} }}\n",
                fields.join(" ")
            ));
            src.push_str(&format!("fn mk{k}(n: Int) -> W{k} {{ return W{k} {{ f0: f\"made {{n}}\" }}; }}\n"));
        }
        let calls: Vec<String> = (0..KINDS).map(|k| format!("mk{k}(i);")).collect();
        src.push_str(&format!(
            r#"main locus App {{
    @unbounded
    fn churn(n: Int) {{
        let mut i = 0;
        while i < n {{ {} i = i + 1; }}
    }}
    run() {{
        self.churn(500);
        print("base_statm=");
        println(std::io::fs::read_file("/proc/self/statm") or "");
        self.churn({turns});
        print("grown_statm=");
        println(std::io::fs::read_file("/proc/self/statm") or "");
    }}
}}
fn main() {{ App {{ }}; }}
"#,
            calls.join(" ")
        ));
        src
    };

    let bin = harness::unique_bin("hale_returned_sizes_rss");
    build_opts::build_source(&program(20_000), &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let read = |key: &str| -> i64 {
        let line = stdout.lines().find_map(|l| l.strip_prefix(key)).unwrap_or_else(|| panic!("missing {key}: {stdout}"));
        harness::statm_resident_bytes(line)
    };
    let grown = read("grown_statm=") - read("base_statm=");
    let per_turn = grown / 20_000;
    assert!(per_turn <= 32, "rss grew {grown} bytes over 20k turns of {KINDS} returned sizes ({per_turn} a turn)");

    let bin = harness::unique_bin("hale_returned_sizes_asan");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(&program(200), &bin, &options).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && !stderr.contains("Sanitizer"), "{}\n{stderr}", String::from_utf8_lossy(&out.stdout));
}
