//! A refusal lowering raises with no place of its own names the
//! expression being lowered when it was raised.
//!
//! `type Page { meta: Meta = { }; }` parses `{ }` as an empty block, and
//! lowering the default at a literal that leaves the field refuses it
//! ("block used as expression has no trailing expression"). The build
//! printed that as a bare `codegen error:` line, with no file, line or
//! column to look at (downstream handoff). `hale check` refuses the
//! program now (`corpus_check_build_agreement`), so this harness, which
//! lowers without the check, is what still reaches the refusal: it is
//! placed at the innermost expression of the user's source being
//! lowered, the default itself.

use hale_codegen::CodegenError;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

#[test]
fn a_positionless_refusal_is_placed_at_the_expression_being_lowered() {
    let src = "type Meta { count: Int = 0; }\n\
               type Page { meta: Meta = { }; }\n\
               fn main() { let p = Page { }; println(to_string(p.meta.count)); }\n";
    let bin = harness::unique_bin(&format!("hale_test_located_refusal_{}", std::process::id()));
    let err = build_opts::build_source(src, &bin, &build_opts::options()).expect_err("an empty block as a value");
    let _ = std::fs::remove_file(&bin);
    let (msg, span) = match err {
        CodegenError::UnsupportedAt(msg, span) => (msg, span),
        other => panic!("expected a located refusal, got {other:?}"),
    };
    assert_eq!(msg, "block used as expression has no trailing expression");
    assert_eq!(span.slice(src), "{ }", "placed at the default");
    assert_eq!(span.line_col(src), (2, 26), "line 2, column 26");
}
