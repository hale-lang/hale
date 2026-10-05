//! F.40 phase 4, S1: a `std::` call at statement position is the
//! expression form's call with its value dropped.
//!
//! `lower_stdlib_path_call` keeps only the arms a statement answers
//! differently and its own fallibility refusal; every other path falls
//! through to `lower_stdlib_path_call_expr`. Before, its last arm was
//! "stdlib path `..` — not implemented", so three kinds of statement
//! call changed, each pinned here:
//!
//! 1. a path only the expression form had an arm for now lowers;
//! 2. a function with a Hale body named by `hale_stdlib::PATH_RENAMES`
//!    now lowers through the expression form's fallback to that body
//!    (one that returns no value is still refused there, so
//!    `std::process::adopt`, Unit, keeps its own statement arm);
//! 3. `std::str::parse_int` and `parse_float`, bare, get the expression
//!    form's fallibility refusal.

#[path = "support/harness.rs"]
mod harness;

fn checks_clean(source: &str) -> hale_syntax::ast::Program {
    let program = hale_syntax::parse_source(source).expect("parses");
    let errors: Vec<String> = hale_types::check_program(&program)
        .iter()
        .filter(|d| d.is_error())
        .map(|d| format!("{d:?}"))
        .collect();
    assert!(errors.is_empty(), "the checker refuses it:\n  {}", errors.join("\n  "));
    program
}

/// Build `program` and run it, returning its IR and its stdout.
fn build_and_run(program: &hale_syntax::ast::Program, name: &str) -> (String, String) {
    let bin = harness::unique_bin(name);
    let ir = harness::build_ir_text(program, &bin).unwrap_or_else(|e| panic!("does not lower: {e:?}"));
    let out = std::process::Command::new(&bin).output().expect("runs");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "exit {:?}: {}", out.status, String::from_utf8_lossy(&out.stderr));
    (ir, String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The body of `define .. @main(`, for asserting what main itself calls.
fn main_body(ir: &str) -> &str {
    let start = ir.find("@main(").expect("a main");
    let end = ir[start..].find("\n}\n").map_or(ir.len(), |n| start + n);
    &ir[start..end]
}

/// Kind 1: `std::str::contains` has an expression arm and had no
/// statement arm, so a bare call was "stdlib path `std::str::contains`
/// — not implemented". It now lowers to the expression arm's call.
#[test]
fn a_path_only_the_expression_form_had_an_arm_for_lowers_at_statement_position() {
    let program = checks_clean(
        "fn main() {\n    std::str::contains(\"abc\", \"b\");\n    println(\"after\");\n}\n",
    );
    let (ir, stdout) = build_and_run(&program, "stmt_expression_only_path");
    assert!(
        main_body(&ir).contains("@lotus_str_contains("),
        "main does not call the expression arm's runtime function"
    );
    assert_eq!(stdout, "after\n");
}

/// Kind 2: `std::log::kv` is a `PATH_RENAMES` row naming the Hale body
/// `__std_log_kv`, and no dispatcher has an arm for it; a bare call was
/// "not implemented". It now reaches the expression form's fallback,
/// which calls the body, and the value is dropped.
#[test]
fn a_hale_body_named_by_path_renames_lowers_at_statement_position() {
    let program = checks_clean(
        "fn main() {\n    std::log::kv(\"k\", \"v\");\n    println(\"after\");\n}\n",
    );
    let (ir, stdout) = build_and_run(&program, "stmt_path_renames_body");
    assert!(main_body(&ir).contains("@__std_log_kv("), "main does not call the Hale body");
    assert_eq!(stdout, "after\n");
}

/// Kind 3: bare `std::str::parse_int` / `parse_float` at statement
/// position were "not implemented" (the statement form's refusal list
/// never named them). They now get the expression form's refusal, in
/// its words. The checker refuses the bare call first (its signature
/// row is fallible), so only an unchecked build reaches this.
#[test]
fn bare_parse_int_and_parse_float_get_the_expression_forms_refusal() {
    for path in ["std::str::parse_int", "std::str::parse_float"] {
        let program =
            hale_syntax::parse_source(&format!("fn main() {{\n    {path}(\"1\");\n}}\n")).expect("parses");
        let bin = harness::unique_bin("stmt_bare_parse");
        let err = harness::build_ir_text(&program, &bin).expect_err("a bare parse is refused");
        let _ = std::fs::remove_file(&bin);
        let text = format!("{err}");
        assert!(
            text.contains(&format!(
                "`{path}` returns a fallible value — address the error with \
                 `or raise`, `or <substitute>`, or `or self.handle(err)`"
            )),
            "{path}: {text}"
        );
    }
}

/// A path no dispatcher lowers at statement position fails as it did
/// before the fold, in the statement's words: the fall-through must not
/// hand a statement the expression form's "in expression position".
/// The four paths the checker knows and only the `or` form lowers are
/// the ones a checked program can reach this with.
#[test]
fn a_statement_no_dispatcher_lowers_keeps_the_statement_wording() {
    for path in [
        "std::io::tcp::set_recv_timeout",
        "std::io::tcp::set_send_timeout",
        "std::io::tls::set_nodelay",
        "std::io::tls::set_rx_timestamps",
    ] {
        let program =
            hale_syntax::parse_source(&format!("fn main() {{\n    {path}(3, 5);\n}}\n")).expect("parses");
        let bin = harness::unique_bin("stmt_not_implemented");
        let err = harness::build_ir_text(&program, &bin).expect_err("no dispatcher lowers it bare");
        let _ = std::fs::remove_file(&bin);
        let text = format!("{err}");
        assert!(text.contains(&format!("stdlib path `{path}` — not implemented")), "{path}: {text}");
        assert!(!text.contains("in expression position"), "{path}: {text}");
    }
}
