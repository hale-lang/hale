//! The builtin error types are declared once (F.40 phase 4, S8):
//! `hale_types::builtin_types::BUILTIN_TYPES`, which the checker injects
//! from and lowering builds its structs from.
//!
//! Before it, each was written twice, as the resolver's injection and as
//! one of lowering's nine builders, and the two agreed field for field on
//! the eight both had; `ClosureViolation` was lowering's alone, and typed
//! `Unknown` to the checker until S8 2 of 2 injected it. The first test
//! below holds the two readers to the one table: a program reading every
//! field of every builtin type, each through an operation only its row's
//! type allows (`len` of a String, `* 2` of an Int), checks clean and
//! lowers, and lowering lays each struct out as the row says.

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;

#[path = "support/harness.rs"]
mod harness;

use hale_syntax::ast::PrimType;
use hale_types::builtin_types::BUILTIN_TYPES;

/// A function per builtin type reading each of its fields.
fn reader_program() -> String {
    let mut src = String::new();
    for t in BUILTIN_TYPES {
        let reads: Vec<String> = t
            .fields
            .iter()
            .map(|(f, p)| match p {
                PrimType::String => format!("len(e.{f})"),
                PrimType::Int => format!("e.{f} * 2"),
                other => panic!("{}.{f}: {other:?}", t.name),
            })
            .collect();
        src.push_str(&format!("fn read_{0}(e: {0}) -> Int {{\n    return {1};\n}}\n\n", t.name, reads.join(" + ")));
    }
    src.push_str("fn main() {\n}\n");
    src
}

#[test]
fn every_builtin_type_is_read_as_its_row_says_by_the_check_and_by_lowering() {
    let src = reader_program();
    let program = hale_syntax::parse_source(&src).expect("parses");
    let errors: Vec<String> =
        entries::check_program(&program).iter().filter(|d| d.is_error()).map(|d| format!("{d:?}")).collect();
    assert!(errors.is_empty(), "the checker refuses:\n{src}\n  {}", errors.join("\n  "));

    let bin = harness::unique_bin("builtin_types");
    let ir = harness::build_source_ir_text(&src, &bin).unwrap_or_else(|e| panic!("does not lower: {e:?}\n{src}"));
    let _ = std::fs::remove_file(&bin);
    for t in BUILTIN_TYPES {
        let layout: Vec<&str> = t.fields.iter().map(|(_, p)| if *p == PrimType::Int { "i64" } else { "ptr" }).collect();
        let body = format!("type {{ {} }}", layout.join(", "));
        // `%type.IoError.0`: the stdlib's own `type IoError` (io_tcp.hl)
        // replaces the builtin, and LLVM numbers the second struct of a
        // name. Every declaration of the type has the row's layout.
        let decls: Vec<&str> = ir
            .lines()
            .filter(|l| {
                l.strip_prefix(&format!("%type.{}", t.name)).is_some_and(|rest| {
                    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
                    rest.starts_with(" = type")
                })
            })
            .collect();
        assert!(!decls.is_empty(), "lowering declares no `{}`", t.name);
        for d in decls {
            assert!(d.ends_with(&body), "lowering lays `{}` out otherwise: `{d}`, the row says `{body}`", t.name);
        }
    }
}

/// A coordinator whose `on_failure` reads its `ClosureViolation` through
/// `handler`, beside a worker whose closure fails at birth.
fn violation_program(handler: &str) -> String {
    let head = r#"
locus Worker {
    params {
        valid: Int = 0;
    }

    closure must_be_valid {
        self.valid ~~ 1 within 0;
        epoch birth;
    }
}

locus Coordinator {
    on_failure(w: Worker, err: ClosureViolation) {
"#;
    let tail = r#"        quarantine (w);
    }

    run() {
        Worker { valid: 0 };
    }
}

fn main() {
    Coordinator { };
}
"#;
    format!("{head}{handler}{tail}")
}

/// `(message, the text its span covers)` of each error the check finds.
fn located_errors(src: &str) -> Vec<(String, String)> {
    let program = hale_syntax::parse_source(src).expect("parses");
    entries::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message, src[d.span.start.0 as usize..d.span.end.0 as usize].to_string()))
        .collect()
}

/// F.40 phase 4, S8 2 of 2 (a classified correction): the checker injects
/// `ClosureViolation` like the other builtin types, so an `on_failure`
/// handler's reads of it are typed. Its fields are the ones lowering
/// builds, read with their types (`len` of a String, `* 2` of the Int),
/// and the program runs with the values lowering fills in. Before, the
/// checker typed every read `Unknown`.
#[test]
fn a_closure_violations_fields_are_typed_by_the_check() {
    let src = violation_program(
        "        let n = len(err.locus) + len(err.closure) + err.diff * 2;\n        println(err.locus, \" \", err.closure, \" \", n);\n",
    );
    assert_eq!(located_errors(&src), Vec::<(String, String)>::new(), "{src}");
    let bin = harness::unique_bin("closure_violation_read");
    let built = harness::build_source_ir_text(&src, &bin);
    assert!(built.is_ok(), "does not lower: {built:?}");
    let out = std::process::Command::new(&bin).output().expect("runs");
    let _ = std::fs::remove_file(&bin);
    // 6 + 13 + (0 - 1) * 2: `diff` is the closure's `left - right`.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Worker must_be_valid 17\n");
}

/// The same correction's refusal: a misspelled field of a
/// `ClosureViolation` is a located error, where the checker typed it
/// `Unknown` and passed it.
#[test]
fn a_misspelled_closure_violation_field_is_refused_by_the_check() {
    let src = violation_program("        println(err.closur);\n");
    assert_eq!(
        located_errors(&src),
        vec![(
            "no field `closur` on `ClosureViolation` — did you mean `closure`?".to_string(),
            "err.closur".to_string()
        )],
        "{src}"
    );
}
