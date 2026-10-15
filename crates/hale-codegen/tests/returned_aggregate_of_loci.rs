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
