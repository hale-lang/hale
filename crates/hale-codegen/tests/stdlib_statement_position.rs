//! F.40 phase 4, S1: a `std::` call at statement position is the
//! expression form's call with its value dropped.
//!
//! S1 kept in the statement dispatcher only the arms a statement
//! answers differently and its own fallibility refusal, every other
//! path falling through to the expression dispatcher; since S3 both
//! positions are `lower_std_call` with the position a parameter, and an
//! arm that does not match on it is the value position's, its value
//! dropped. Before S1, the statement dispatcher's last arm was "stdlib
//! path `..` — not implemented", so three kinds of statement call
//! changed, each pinned here:
//!
//! 1. a path only the expression form had an arm for now lowers;
//! 2. a function with a Hale body named by `hale_stdlib::PATH_RENAMES`
//!    now lowers through the expression form's fallback to that body
//!    (one that returned no value was still refused there, so
//!    `std::process::adopt`, Unit, kept its own statement arm until S5,
//!    when a statement became a call of such a body and done);
//! 3. `std::str::parse_int` and `parse_float`, bare, got the expression
//!    form's fallibility refusal; since S5 every bare call of a fallible
//!    row gets one answer, read from the row.

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;

#[path = "support/harness.rs"]
mod harness;

fn checks_clean(source: &str) -> &str {
    let program = hale_syntax::parse_source(source).expect("parses");
    let errors: Vec<String> = entries::check_program(&program)
        .iter()
        .filter(|d| d.is_error())
        .map(|d| format!("{d:?}"))
        .collect();
    assert!(errors.is_empty(), "the checker refuses it:\n  {}", errors.join("\n  "));
    source
}

/// Build the program text and run it, returning its IR and its stdout.
fn build_and_run(program: &str, name: &str) -> (String, String) {
    let bin = harness::unique_bin(name);
    let ir = harness::build_source_ir_text(program, &bin).unwrap_or_else(|e| panic!("does not lower: {e:?}"));
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

/// F.40 phase 4, S5 (a classified correction): a stdlib function whose
/// Hale body, reached through `PATH_RENAMES`, returns nothing can be called
/// as a statement. The fallback was written for a value position and
/// answered "returns no value but is used in expression position" at a
/// statement too, which is why `std::process::adopt` kept a hand-kept
/// statement branch; that branch did nothing the rule does not, so it is
/// gone and `adopt`'s row is a rename like the other `process.hl` wrappers.
/// A value position keeps its refusal, worded for a value.
#[test]
fn a_renamed_body_that_returns_nothing_is_called_as_a_statement() {
    use hale_types::stdlib_surface::{row, Lower};
    assert_eq!(row(&["std", "process", "adopt"]).map(|r| r.lower), Some(Lower::Renamed));
    let program = checks_clean(
        "fn main() {\n    let a = std::process::Child { };\n    let b = std::process::Child { };\n    \
         std::process::adopt(a, b);\n    println(\"after\");\n}\n",
    );
    let (ir, stdout) = build_and_run(&program, "stmt_renamed_no_value_body");
    assert!(main_body(&ir).contains("@__std_process_adopt("), "main does not call the Hale body");
    assert_eq!(stdout, "after\n");
    let (text, _) = lowering_error(
        "fn main() {\n    let a = std::process::Child { };\n    let b = std::process::Child { };\n    \
         let v = std::process::adopt(a, b);\n}\n",
        "value_renamed_no_value_body",
    );
    assert_eq!(
        text,
        "unsupported in codegen v0: stdlib path `std::process::adopt` returns no value but is used in \
         expression position"
    );
}

/// Build `source` without the check, and return lowering's refusal: its
/// text and, when it is located, its span.
fn lowering_error(source: &str, name: &str) -> (String, Option<hale_syntax::Span>) {
    let bin = harness::unique_bin(name);
    let err = harness::build_source_ir_text(source, &bin).expect_err("lowering refuses it");
    let _ = std::fs::remove_file(&bin);
    let span = match &err {
        hale_codegen::CodegenError::UnsupportedAt(_, span) => Some(*span),
        _ => None,
    };
    (format!("{err}"), span)
}

/// F.40 phase 4, S5 (a classified correction): which stdlib calls
/// lowering refuses is the row's fallibility, and a call the check
/// refuses gets an internal error naming the row, at the callee, where
/// lowering used to refuse it in words of its own or answer "not
/// implemented" (both only in a build that skipped the check):
///
/// - a bare call of a function whose row can fail: `parse_int` was in
///   the lists ("returns a fallible value — address the error .."),
///   `parse_decimal` was not ("not implemented");
/// - an `or` over a function whose row cannot fail, with a signature:
///   `sqrt` was in the list ("is not a fallible call"), `regex::valid`
///   was not ("`or` over unknown path call").
///
/// An `or` over a function with no signature was what a checked program
/// could still reach, since the check typed the call `Unknown` (`SOL_SOCKET`
/// got "`or` over unknown path call"). S6 signed it, and S6's ruling signed
/// the last unsigned intrinsic rows a program may call, the seven
/// `std::io::mirror` cursor primitives, so the check refuses an `or` over
/// `std::io::mirror::__len` too, and lowering's answer is the internal error.
#[test]
fn which_stdlib_calls_lowering_refuses_is_read_from_the_row() {
    for (path, err, line) in [
        ("std::str::parse_int", "ParseError", "    std::str::parse_int(\"1\");\n"),
        ("std::str::parse_decimal", "ParseError", "    let d = std::str::parse_decimal(\"1\");\n"),
        ("std::io::tcp::set_nodelay", "IoError", "    std::io::tcp::set_nodelay(3, true);\n"),
    ] {
        let pos = if line.contains("let ") { "value" } else { "statement" };
        let source = format!("fn main() {{\n{line}}}\n");
        let (text, span) = lowering_error(&source, "stmt_bare_fallible_row");
        assert_eq!(
            text,
            format!(
                "unsupported in codegen v0: internal error: a bare call of `{path}` reached lowering at {pos} \
                 position, but its row says it can fail ({err}), and `hale check` refuses that \
                 call (GH #738): this build skipped the check"
            ),
        );
        let at = source.find(path).unwrap() as u32;
        assert_eq!(span.map(|s| (s.start.0, s.end.0)), Some((at, at + path.len() as u32)), "{path}");
    }
    for (path, call) in [
        ("std::math::sqrt", "std::math::sqrt(2.0) or 0.0"),
        ("std::regex::valid", "std::regex::valid(\"a\") or false"),
        ("std::io::mirror::__len", "std::io::mirror::__len(0) or 0"),
    ] {
        let source = format!("fn main() {{\n    let v = {call};\n    println(v);\n}}\n");
        let (text, span) = lowering_error(&source, "or_over_infallible_row");
        assert_eq!(
            text,
            format!(
                "unsupported in codegen v0: internal error: an `or` over `{path}` reached lowering, but its row \
                 says it cannot fail, and `hale check` refuses that `or`: this build skipped the check"
            ),
        );
        let at = source.find(path).unwrap() as u32;
        assert_eq!(span.map(|s| (s.start.0, s.end.0)), Some((at, at + path.len() as u32)), "{path}");
    }
}

/// F.40 phase 4, S5 (a classified correction): the nine fallible rows
/// that kept a bare arm no checked program reaches (it returned the value
/// directly, -1 or an Int status) have none. A bare call built without the
/// check is the same internal error as every other fallible row's.
#[test]
fn the_fallible_rows_dead_bare_arms_are_gone() {
    for (path, args) in [
        ("std::bytes::at", "std::bytes::from_string(\"ab\"), 0"),
        ("std::io::fs::file_size", "\"/x\""),
        ("std::io::fs::list_dir_at", "\"/x\", 0"),
        ("std::io::fs::list_dir_count", "\"/x\""),
        ("std::io::fs::mkdir", "\"/x\""),
        ("std::io::fs::read_bytes", "\"/x\""),
        ("std::io::fs::read_file", "\"/x\""),
        ("std::io::fs::write_file", "\"/x\", \"y\""),
        ("std::io::fs::write_file_append", "\"/x\", \"y\""),
    ] {
        let err = hale_types::stdlib_surface::signature_for(&path.split("::").collect::<Vec<_>>())
            .and_then(|s| s.fallible)
            .expect("a fallible row");
        let source = format!("fn main() {{\n    let v = {path}({args});\n}}\n");
        let (text, _) = lowering_error(&source, "dead_bare_arm");
        assert_eq!(
            text,
            format!(
                "unsupported in codegen v0: internal error: a bare call of `{path}` reached lowering \
                 at value position, but its row says it can fail ({err}), and `hale check` refuses \
                 that call (GH #738): this build skipped the check"
            ),
        );
    }
}

/// A path no arm lowers at statement position fails in the statement's
/// words, and at a value position in the value position's: "not
/// implemented" is worded where it is produced (S3), so a statement never
/// gets "in expression position". Since S5 a function lowered only under
/// `or` gets the bare-fallible answer instead (above), so this is a path
/// with no row.
#[test]
fn a_statement_no_dispatcher_lowers_keeps_the_statement_wording() {
    let path = "std::io::file::no_such_primitive";
    let (text, _) = lowering_error(&format!("fn main() {{\n    {path}(3, 5);\n}}\n"), "stmt_not_implemented");
    assert!(text.contains(&format!("stdlib path `{path}` — not implemented")), "{path}: {text}");
    assert!(!text.contains("in expression position"), "{path}: {text}");
    let (text, _) =
        lowering_error(&format!("fn main() {{\n    let x = {path}(3, 5);\n    println(x);\n}}\n"), "stmt_not_implemented");
    assert!(
        text.contains(&format!("stdlib path `{path}` in expression position — not implemented")),
        "{path}: {text}"
    );
}
