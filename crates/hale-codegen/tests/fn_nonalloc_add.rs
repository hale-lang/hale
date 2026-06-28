//! Type-aware `Add` classification (2026-06-28).
//!
//! `fn_body_definitely_non_allocating` used to treat every `+` as possibly
//! String concat (allocating), because codegen had no type info — so a leaf
//! helper like `fn f(i: Int) -> Int { return i + 1; }` was classified
//! allocating and paid a per-call scratch-arena malloc/free (~10× the cost
//! of the same body using `*`). It's now seeded with the fn's numeric
//! params and classifies `Int/Uint/Float/Duration + …` as non-allocating.
//!
//! These tests pin the CORRECTNESS boundary the optimization must respect:
//! a numeric-Add helper computes correctly (and is cheap), and — critically
//! — a String-concat helper is NOT misclassified as non-allocating, which
//! would skip the subregion and strand the returned String in the caller's
//! arena. (The perf win itself is measured out-of-band; ASan coverage of
//! misclassification leaks lives in `corpus_oracle`.)

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_nonalloc_add_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{name}: non-zero exit {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn numeric_add_helper_computes_correctly() {
    // A leaf helper doing integer Adds (now non-allocating) — called in a
    // hot loop, the sum must be exact.
    let src = r#"
        fn step(i: Int) -> Int { return i + i + 1; }
        fn main() {
            let mut acc: Int = 0;
            let mut i: Int = 0;
            while i < 1000 { acc = acc + step(i); i = i + 1; }
            println(acc);
        }
    "#;
    // sum_{i=0}^{999} (2i+1) = 999*1000 + 1000 = 1000000
    assert_eq!(build_and_run("numeric", src).trim(), "1000000");
}

#[test]
fn float_and_let_add_helper_runs() {
    // Float params + a numeric `let` extending the scope.
    let src = r#"
        fn mix(x: Float, k: Int) -> Float { let n = k + 1; return x + 2.0; }
        fn main() { println(mix(40.5, 1)); }
    "#;
    assert_eq!(build_and_run("float", src).trim(), "42.5");
}

#[test]
fn string_concat_helper_not_misclassified() {
    // The correctness guard: a String-concat helper MUST stay classified
    // allocating (keep its scratch subregion). If the type-aware Add ever
    // wrongly marked String `+` non-allocating, the returned String would
    // be allocated in the caller's arena and clobbered across calls — this
    // loop, accumulating distinct results, would corrupt. Each line must be
    // exact.
    let src = r#"
        fn tag(n: Int) -> String { return "item-" + to_string(n) + "!"; }
        fn main() {
            let mut i: Int = 0;
            while i < 4 { println(tag(i * 10)); i = i + 1; }
        }
    "#;
    assert_eq!(
        build_and_run("strconcat", src).trim(),
        "item-0!\nitem-10!\nitem-20!\nitem-30!"
    );
}

#[test]
fn mixed_numeric_and_string_helper_is_correct() {
    // A fn that does BOTH a numeric Add and a String concat — must be
    // classified allocating (the String concat allocates) and produce the
    // right value.
    let src = r#"
        fn label(n: Int) -> String { let m = n + 1; return "n=" + to_string(m); }
        fn main() { println(label(41)); }
    "#;
    assert_eq!(build_and_run("mixed", src).trim(), "n=42");
}

// --- interprocedural propagation -----------------------------------------

#[test]
fn call_chain_of_numeric_helpers_computes() {
    // a → b → c, all numeric (b uses c's result in an Add). The whole chain
    // is non-allocating, so none pays a scratch — and the value is exact.
    let src = r#"
        fn c(i: Int) -> Int { return i + 1; }
        fn b(i: Int) -> Int { return c(i) * 2; }
        fn a(i: Int) -> Int { return b(i) + c(i); }
        fn main() {
            let mut acc: Int = 0;
            let mut i: Int = 0;
            while i < 100 { acc = acc + a(i); i = i + 1; }
            println(acc);
        }
    "#;
    // a(i) = (2*(i+1)) + (i+1) = 3*(i+1); sum_{0..99} 3(i+1) = 3 * 5050 = 15150
    assert_eq!(build_and_run("chain", src).trim(), "15150");
}

#[test]
fn fn_calling_allocating_helper_stays_correct() {
    // The interprocedural soundness guard: `wrap` calls `tag`, which String-
    // concats (allocating). `wrap` must therefore stay classified
    // allocating — if the fixpoint wrongly promoted it, the returned String
    // would be stranded in the caller's arena and clobbered across the loop.
    let src = r#"
        fn tag(n: Int) -> String { return "x" + to_string(n); }
        fn wrap(n: Int) -> String { return tag(n + 100); }
        fn main() {
            let mut i: Int = 0;
            while i < 4 { println(wrap(i)); i = i + 1; }
        }
    "#;
    assert_eq!(
        build_and_run("interp_alloc", src).trim(),
        "x100\nx101\nx102\nx103"
    );
}

#[test]
fn mutually_recursive_numeric_fns_converge() {
    // Mutual recursion over numeric scalars must converge to non-allocating
    // (the greatest-fixpoint property) and stay correct.
    let src = r#"
        fn ping(i: Int) -> Int { if i <= 0 { return 0; } return pong(i - 1) + 1; }
        fn pong(i: Int) -> Int { if i <= 0 { return 0; } return ping(i - 1) + 1; }
        fn main() { println(ping(10)); }
    "#;
    assert_eq!(build_and_run("mutrec", src).trim(), "10");
}
