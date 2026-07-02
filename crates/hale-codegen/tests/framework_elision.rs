//! v1.x-FRAMEWORK: chunked-class child instantiation strips
//! redundant framework calls when the child + parent shape
//! permits it. Two elisions guarded here:
//!
//! 1. **Empty-accept elision**: when the parent's `accept(c: T)`
//!    body is `{ }`, the `parent.accept(...)` call at the child
//!    statement-position instantiation site is skipped. The
//!    children-array append still fires.
//!
//! 2. **Subregion arena elision**: when the child's locus body
//!    is provably non-allocating (`locus_arena_elidable`
//!    predicate — no slots, no bus, no closures, no failure
//!    handler, all method bodies non-allocating), the child's
//!    `__arena` field borrows the parent's arena directly
//!    instead of calling `lotus_arena_create_subregion`.
//!
//! Combined effect: per-child cost in `coord_with_churn`-style
//! accept-in-a-loop drops by ~75% (the only remaining per-child
//! op is the `lotus_arena_alloc` for the child struct itself).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-framework-elision-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ));
    p
}

fn dump_ir(src: &str, tag: &str) -> String {
    let bin = unique_path(tag, "bin");
    let ir = bin.with_extension("ll");
    let program = hale_syntax::parse_source(src).expect("parse");
    std::env::set_var("LOTUS_DUMP_IR", "1");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");
    let ir_text = std::fs::read_to_string(&ir).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);
    ir_text
}

#[test]
fn empty_accept_call_is_elided() {
    // Coord declares `accept(w: Worker) { }` — empty body. The
    // child-instantiation site in `Coord.run` should NOT emit a
    // `call void @Coord.accept(...)`.
    let src = r#"
        locus Worker {
            params { n: Int = 0; }
        }
        locus Coord : projection chunked {
            params { batch: Int = 3; }
            accept(w: Worker) { }
            run() {
                let mut i = 0;
                while i < self.batch {
                    Worker { n: i };
                    i = i + 1;
                }
            }
        }
        fn main() {
            Coord { batch: 3 };
        }
    "#;
    let ir = dump_ir(src, "empty-accept");
    // The function definition is still emitted (Pass C is
    // unconditional), but no caller should reference it.
    assert!(
        ir.contains("define void @Coord.accept"),
        "Coord.accept fn should still be defined",
    );
    assert!(
        !ir.contains("call void @Coord.accept"),
        "elided empty-body accept must not be called per child; IR snippet near the loop:\n{}",
        &ir[..ir.len().min(2000)],
    );
}

#[test]
fn non_empty_accept_still_dispatched() {
    // Sibling case: accept body has a real statement. The call
    // must still fire.
    let src = r#"
        locus Worker {
            params { n: Int = 0; }
        }
        locus Coord : projection chunked {
            params { batch: Int = 3; total: Int = 0; }
            accept(w: Worker) {
                self.total = self.total + 1;
            }
            run() {
                let mut i = 0;
                while i < self.batch {
                    Worker { n: i };
                    i = i + 1;
                }
            }
        }
        fn main() {
            Coord { batch: 3 };
        }
    "#;
    let ir = dump_ir(src, "real-accept");
    assert!(
        ir.contains("call void @Coord.accept"),
        "non-empty accept must still be called per child",
    );
}

#[test]
fn subregion_call_elided_for_elidable_child() {
    // Worker has no slots, no bus, no closures, no method bodies
    // → `locus_arena_elidable` is true. With the Subregion-path
    // elision the per-child loop body should NOT contain a
    // `call ptr @lotus_arena_create_subregion`. The child
    // struct still allocates via `lotus_arena_alloc`.
    let src = r#"
        locus Worker {
            params { n: Int = 0; }
        }
        locus Coord : projection chunked {
            params { batch: Int = 3; }
            accept(w: Worker) { }
            run() {
                let mut i = 0;
                while i < self.batch {
                    Worker { n: i };
                    i = i + 1;
                }
            }
        }
        fn main() {
            Coord { batch: 3 };
        }
    "#;
    let ir = dump_ir(src, "subregion-elide");

    // Find the Coord.run body and check only its hot loop. The
    // method body itself opens a scratch subregion at entry
    // (bus-arena reclaim 2026-05-21), so a single
    // `lotus_arena_create_subregion` call IS expected — but it
    // lives at function entry, not inside the per-iteration
    // while.body. The elision check is about per-child subregion
    // creation, so scope the assertion to the hot loop block.
    let coord_run_start = ir.find("define void @Coord.run").expect("Coord.run defined");
    let coord_run_end = ir[coord_run_start..]
        .find("\n}")
        .map(|i| coord_run_start + i)
        .unwrap_or(ir.len());
    let coord_run_body = &ir[coord_run_start..coord_run_end];
    // Carve out the while.body block (per-iteration code).
    let body_start = coord_run_body
        .find("while.body:")
        .expect("Coord.run has while.body");
    let body_after = &coord_run_body[body_start..];
    let body_end = body_after
        .find("\nwhile.cond:")
        .or_else(|| body_after.find("\nwhile.end:"))
        .map(|i| body_start + i)
        .unwrap_or(coord_run_body.len());
    let while_body = &coord_run_body[body_start..body_end];
    assert!(
        !while_body.contains("@lotus_arena_create_subregion"),
        "per-iteration Worker instantiation must not call \
         lotus_arena_create_subregion (child is arena_elidable); \
         while.body:\n{}",
        while_body,
    );
    assert!(
        while_body.contains("@lotus_child_struct_alloc"),
        "Coord.run while.body must still allocate the Worker struct \
         via lotus_child_struct_alloc (the recycling front of \
         lotus_arena_alloc for accept'd children, 2026-07-01)",
    );
}
