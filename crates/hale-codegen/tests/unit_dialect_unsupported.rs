//! GH #1076, step U1: lowering and the unit dialect.
//!
//! A `unit` declaration and a scalar `type` lower to no code: the
//! checker judges them, and a program that passes the check holds no
//! value of a new type. A value of one (a quantity literal) is not
//! lowered yet; a build that skips the check (as this harness does) is
//! refused with an error naming it and where it is, never a panic and
//! never a silent skip.

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
fn a_quantity_literal_is_refused_by_name() {
    let src = "fn main() {\n    let fee = 3bp;\n    println(1);\n}\n";
    let bin = harness::unique_bin(&format!("hale_test_unit_dialect_literal_{}", std::process::id()));
    let err = build_opts::build_source(src, &bin, &build_opts::options()).expect_err("a quantity value is not lowered");
    let _ = std::fs::remove_file(&bin);
    let got = match err {
        CodegenError::UnsupportedAt(msg, span) => format!("{msg} @ {}", span.slice(src)),
        other => panic!("expected a located refusal, got {other}"),
    };
    assert_eq!(
        got,
        "quantity literal `3bp`: the unit dialect is not lowered yet (GH #1076); `hale check` refuses it @ 3bp"
    );
}
