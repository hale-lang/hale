//! Lowering reads the checker's answers from the typed-body table
//! (F.40 phase 3, E4) and refuses a site the table holds no row for, at
//! the site, rather than typing it again.

use hale_codegen::{build_resolved, CodegenError};
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_types::typed_bodies::TypedBodies;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The harness's view of `src` with its typed-body table taken out, and
/// lowered: every site that reads a row has none.
fn lower_without_a_table(tag: &str, src: &str) -> Result<(), CodegenError> {
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let mut view = snap.demand_lowering().unwrap_or_else(|b| panic!("lowering blocked: {:?}", b.refused)).clone();
    view.typed = TypedBodies::default();
    let bin = harness::unique_bin(&format!("typed_body_rows_{tag}"));
    let built = build_resolved(&view, &bin, &build_opts::options());
    let _ = std::fs::remove_file(&bin);
    built
}

/// An accumulator's element type is the closure's row: with none, the
/// slot is refused at the accumulated expression.
#[test]
fn an_accumulator_without_a_row_is_refused_at_its_expression() {
    let src = "locus Tracker {\n    params { delta: Int = 0; }\n    \
               closure band { sum(self.delta) ~~ 0 within 100; epoch tick; }\n    \
               run { }\n}\nfn main() { Tracker { }; }\n";
    match lower_without_a_table("acc", src) {
        Err(CodegenError::UnsupportedAt(msg, span)) => {
            assert!(msg.contains("has no typed-body row"), "{msg}");
            assert_eq!(&src[span.start.as_usize()..span.end.as_usize()], "self.delta");
        }
        other => panic!("expected a located refusal, got {other:?}"),
    }
}
