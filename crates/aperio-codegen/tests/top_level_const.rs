//! G7 — top-level `const NAME: T = LITERAL;` codegens.
//!
//! Before G7, the parser and AST accepted `const FOO: Int = 7;`
//! at top level but codegen had no emission path. Same-seed
//! references to the const errored as `unknown identifier
//! `FOO``, and cross-seed `lib::FOO` references errored as
//! `unknown identifier `__lib_..._FOO`` (after the mangler had
//! renamed both decl and use to the same string but no value
//! had been registered).
//!
//! The fix registers every `TopDecl::Const` in `Cx::user_consts`
//! at top-decl collection time, evaluating the value expression
//! to a `ParamValue` via the same `param_value` helper that locus
//! `params { ... }` defaults use (so any literal supported there
//! is supported here, and a non-literal value errors with a
//! pointer to the same shape rule).
//!
//! The `Expr::Ident` lowering checks `user_consts` between locals
//! (which shadow) and user_fns; the `Expr::Path` lowering walks
//! `mangled_for_path` and consults `user_consts` so `lib::FOO`
//! reads work the same way intra-seed reads do.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_top_level_const_{}_{}",
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
fn top_const_int_string_bool_read_from_main() {
    // All three primitive shapes inline from main.
    let src = r#"
        const CAP:  Int    = 99;
        const TAG:  String = "hello";
        const FAST: Bool   = true;

        fn main() {
            println("cap=", to_string(CAP));
            println("tag=", TAG);
            if FAST { println("fast"); }
        }
    "#;
    let (stdout, status) = build_and_run("primitive_reads", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("cap=99"), "stdout: {:?}", stdout);
    assert!(stdout.contains("tag=hello"), "stdout: {:?}", stdout);
    assert!(stdout.contains("fast"), "stdout: {:?}", stdout);
}

#[test]
fn top_const_read_inside_locus_method() {
    // A locus method reads a same-seed top-level const directly.
    let src = r#"
        const LIMIT: Int = 42;

        locus L {
            fn run_it() {
                println("limit=", to_string(LIMIT));
            }
        }

        fn main() {
            let l = L { };
            l.run_it();
        }
    "#;
    let (stdout, status) = build_and_run("inside_locus", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("limit=42"), "stdout: {:?}", stdout);
}

#[test]
fn top_const_locals_shadow_globals() {
    // A local binding with the same name shadows the const —
    // matches the documented rule for user_fns vs locals.
    let src = r#"
        const CAP: Int = 99;

        fn main() {
            println("outer=", to_string(CAP));
            let CAP = 7;
            println("inner=", to_string(CAP));
        }
    "#;
    let (stdout, status) = build_and_run("locals_shadow", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("outer=99"), "stdout: {:?}", stdout);
    assert!(stdout.contains("inner=7"), "stdout: {:?}", stdout);
}

#[test]
fn top_const_non_literal_value_lowers_at_use_site() {
    // 2026-05-22 (F.1): non-literal const initializers (computed
    // expressions, struct literals, etc.) used to error with a
    // "value must be a literal" diagnostic. Now they're stored
    // as deferred Expr in `user_const_exprs` and re-lowered at
    // each use site (same lifetime as inlining the expression
    // by hand). The arithmetic case here folds at IR-build time
    // through the normal Binary lowering.
    let src = r#"
        const X: Int = 1 + 2;

        fn main() {
            println("x=", to_string(X));
        }
    "#;
    let (stdout, status) = build_and_run("non_lit_compute", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("x=3"), "stdout: {:?}", stdout);
}

#[test]
fn top_const_struct_literal_value_lowers_at_use_site() {
    // 2026-05-22 (F.1): struct literals are the load-bearing F.1
    // case — locus param defaults already accepted them; consts
    // didn't until this change. Verify the natural shape works.
    let src = r#"
        type Pair { a: Int = 0; b: Int = 0; }

        const ORIGIN: Pair = Pair { a: 3, b: 4 };

        fn main() {
            let p = ORIGIN;
            println("a=", to_string(p.a), " b=", to_string(p.b));
        }
    "#;
    let (stdout, status) = build_and_run("non_lit_struct", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("a=3 b=4"), "stdout: {:?}", stdout);
}
