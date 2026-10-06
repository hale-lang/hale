//! GH #1076: lowering and the unit dialect's declarations and literals.
//!
//! A `unit` declaration and a scalar `type` lower to no code: the
//! checker judges them. A quantity literal (U3) is the constant its row
//! in the typed bodies' `conversions` column says, the count the checker
//! converted into the denomination the literal flows into; a literal with
//! no row (a build that skips the check, as this harness does, of a
//! literal the check refuses) is a missing required row, refused with an
//! error naming it and where it is, never a panic and never a silent
//! count in the wrong denomination.

use hale_codegen::CodegenError;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

#[test]
fn declarations_lower_to_no_code() {
    let src = "unit cent;\nunit USD = 100 cent;\ntype Money = quantity Int in cent;\n\
               type Session = distinct Int { range: 0..64; }\nfn main() { println(1); }\n";
    let bin = harness::unique_bin(&format!("hale_test_unit_dialect_decls_{}", std::process::id()));
    let built = build_opts::build_source(src, &bin, &build_opts::options());
    let _ = std::fs::remove_file(&bin);
    if let Err(e) = built {
        panic!("a program that only declares units and scalars builds: {e}");
    }
}

#[test]
fn a_quantity_literal_with_no_row_is_refused_where_it_is_written() {
    // No `unit bp`: the check refuses the literal and records no row.
    let src = "fn main() {\n    let fee = 3bp;\n    println(1);\n}\n";
    let bin = harness::unique_bin(&format!("hale_test_unit_dialect_literal_{}", std::process::id()));
    let err = build_opts::build_source(src, &bin, &build_opts::options()).expect_err("a literal with no row");
    let _ = std::fs::remove_file(&bin);
    let got = match err {
        CodegenError::UnsupportedAt(msg, span) => format!("{msg} @ {}", span.slice(src)),
        other => panic!("expected a located refusal, got {other}"),
    };
    assert_eq!(
        got,
        "quantity literal `3bp` has no required `expression_typing` row: the checker converts a quantity literal \
         into the denomination it flows into, and lowering emits that count @ 3bp"
    );
}
