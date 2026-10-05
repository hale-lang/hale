//! The builtin error types are declared once (F.40 phase 4, S8):
//! `hale_types::builtin_types::BUILTIN_TYPES`, which the checker injects
//! from and lowering builds its structs from.
//!
//! Before it, each was written twice, as the resolver's injection and as
//! one of lowering's nine builders, and the two agreed field for field on
//! the eight both had; `ClosureViolation` was lowering's alone. The test
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
