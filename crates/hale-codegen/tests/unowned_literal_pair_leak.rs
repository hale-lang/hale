//! GH #750 — the teardown cascade has to reach the whole locus
//! tree, not just its first level.
//!
//! The reported shape was two unowned locus literals in one fn:
//!
//! ```text
//! println("a=", Holder { tag: "hello" }.tag);
//! println("b=", Holder { tag: "again" }.tag);
//! ```
//!
//! with `Holder` owning a child that owns a `@form(vec)` grandchild.
//! LeakSanitizer flagged the FIRST literal's arena, its chunk and
//! the grandchild's vec buffer; one literal alone, the `let`-bound
//! form, a method call on the literal and a childless `Holder` were
//! all clean.
//!
//! The cause is not the unowned literal and not the pairing. A
//! locus held as a param field is parent-owned: its instantiation
//! takes `lower_locus_instantiation`'s `parent_owns_via_field`
//! no-op branch and is never pushed onto a deferred-dissolve frame,
//! because the owner's teardown is supposed to cascade into it.
//! `emit_locus_field_drains` / `emit_locus_field_dissolves` only
//! ever walked ONE level, so a grandchild never drained, never ran
//! its `dissolve()` body, never had its capacity slots freed and
//! never had its arena destroyed — for every binding shape.
//!
//! What the pairing changes is only whether LeakSanitizer can SEE
//! it. The grandchild's arena pointer lives in the child's arena
//! chunk, and `lotus_arena_destroy` returns chunks to
//! `g_chunk_pool` with their bytes intact, so LSan still reaches
//! the grandchild's arena through the pooled chunk. Building a
//! second tree of the same shape recycles that chunk and
//! overwrites the last reference. Hence "the first of two leaks,
//! the last one never does" — and hence `let_bound_form_leaks_too`
//! below, where the same `let`-bound control leaks as soon as a
//! second call recycles the chunk.
//!
//! So the assertions here are on stdout, not only on the exit
//! status: a lifecycle body that prints makes the cascade's reach
//! and its ORDER observable without a sanitizer. Run the binary
//! under `LOTUS_ASAN=1` (`build_executable` reads the flag at
//! codegen time, as `ownership_bubble.rs` does) and the same
//! programs additionally prove the arenas and the vec buffer are
//! freed: a leak makes the child process exit non-zero and every
//! `status.success()` assertion below fails.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Compile `src`, run it, return its stdout. Asserts a clean exit —
/// which under `LOTUS_ASAN=1` is also the leak oracle.
fn run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("lotus_test_gh750_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "{name}: non-zero exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status,
    );
    stdout
}

/// The locus tree every program below shares: a `@form(vec)`
/// grandchild whose `birth` allocates OUTSIDE its arena (the vec
/// buffer is a realloc), a child that owns it, and a holder that
/// owns the child. Each level's `drain` / `dissolve` prints, so the
/// cascade's reach and order are stdout.
const TREE: &str = r#"
    type Row { v: Int = 0; }
    @form(vec)
    locus Rows { capacity { heap rows of Row; } }
    locus Leaf {
        params { rows: Rows = Rows { }; }
        birth() { self.rows.push(Row { v: 7 }); }
        drain() { println("leaf drained"); }
        dissolve() { println("leaf dissolved"); }
        fn count() -> Int { return self.rows.len(); }
    }
    locus Mid {
        params { leaf: Leaf = Leaf { }; }
        drain() { println("mid drained"); }
        dissolve() { println("mid dissolved"); }
        fn total() -> Int { return self.leaf.count(); }
    }
    locus Holder {
        params { m: Mid = Mid { }; tag: String = "x"; }
        drain() { println("holder drained"); }
        dissolve() { println("holder dissolved"); }
        fn label() -> String { return self.tag; }
    }
"#;

fn program(body: &str) -> String {
    format!("{TREE}\n{body}\n")
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// The issue's reproducer. Both literals' whole trees must be torn
/// down — under `LOTUS_ASAN=1` this is the leak assertion, and
/// without it the grandchild's `dissolve()` count is.
#[test]
fn two_unowned_literals_tear_down_both_trees() {
    let out = run(
        "two_reads",
        &program(
            r#"
            fn main() {
                println("a=", Holder { tag: "hello" }.tag);
                println("b=", Holder { tag: "again" }.tag);
            }
        "#,
        ),
    );
    assert!(out.contains("a=hello"), "got:\n{out}");
    assert!(out.contains("b=again"), "got:\n{out}");
    // Two trees, so two of everything. Pre-fix the leaf lines were
    // absent entirely — the cascade stopped at `Mid`.
    assert_eq!(count(&out, "leaf dissolved"), 2, "got:\n{out}");
    assert_eq!(count(&out, "leaf drained"), 2, "got:\n{out}");
    assert_eq!(count(&out, "mid dissolved"), 2, "got:\n{out}");
    assert_eq!(count(&out, "holder dissolved"), 2, "got:\n{out}");
}

/// The four controls from the issue, all of which were leak-free
/// before the fix — three of them only because nothing recycled
/// the chunk that still pointed at the grandchild's arena. The
/// grandchild's teardown was missing from all of them too.
#[test]
fn controls_tear_down_the_whole_tree_as_well() {
    // (1) one literal: the shape the issue reports as clean.
    let one = run(
        "one_read",
        &program(
            r#"
            fn main() {
                println("a=", Holder { tag: "hello" }.tag);
            }
        "#,
        ),
    );
    assert_eq!(count(&one, "leaf dissolved"), 1, "got:\n{one}");

    // (2) the let-bound form.
    let bound = run(
        "let_bound",
        &program(
            r#"
            fn main() {
                let h = Holder { tag: "hello" };
                println("a=", h.tag, " total=", h.m.total());
            }
        "#,
        ),
    );
    assert!(bound.contains("total=1"), "got:\n{bound}");
    assert_eq!(count(&bound, "leaf dissolved"), 1, "got:\n{bound}");

    // (3) a method call on the literal (the GH #710 shape: the call
    // owns the receiver, so it dissolves at fn-scope exit).
    let recv = run(
        "method_receiver",
        &program(
            r#"
            fn main() {
                println("a=", Holder { tag: "hello" }.label());
                println("b=", Holder { tag: "again" }.label());
            }
        "#,
        ),
    );
    assert!(recv.contains("a=hello"), "got:\n{recv}");
    assert_eq!(count(&recv, "leaf dissolved"), 2, "got:\n{recv}");

    // (4) a holder with no child at all: nothing to cascade into.
    let childless = run(
        "childless",
        &program(
            r#"
            locus Lonely {
                params { n: Int = 0; }
                dissolve() { println("lonely dissolved"); }
            }
            fn main() {
                println("a=", Lonely { n: 21 }.n);
                println("b=", Lonely { n: 22 }.n);
            }
        "#,
        ),
    );
    assert_eq!(count(&childless, "lonely dissolved"), 2, "got:\n{childless}");
    assert_eq!(count(&childless, "leaf dissolved"), 0, "got:\n{childless}");
}

/// The control that shows the leak was never about unowned
/// literals: a `let`-bound tree in a helper fn called twice. The
/// second call recycles the first call's chunk, so under
/// `LOTUS_ASAN=1` this shape reported the identical three leaks
/// before the fix.
#[test]
fn let_bound_form_leaks_too() {
    let out = run(
        "let_in_helper",
        &program(
            r#"
            fn once(t: String) {
                let h = Holder { tag: t };
                println("v=", h.tag, " total=", h.m.total());
            }
            fn main() {
                once("hello");
                once("again");
            }
        "#,
        ),
    );
    assert!(out.contains("v=hello total=1"), "got:\n{out}");
    assert!(out.contains("v=again total=1"), "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 2, "got:\n{out}");
}

/// A hot loop building a temporary per iteration: the unbounded
/// version of the leak. GH #711 made every EXPRESSION-position
/// literal fn-scope-owned like `let`, and GH #815 gave that owner a
/// per-iteration boundary — a deferred slot reclaims its previous
/// occupant before the next iteration overwrites it — so this
/// spelling is back to reclaiming its whole tree each time round:
/// the count is exactly the iteration count and (under ASan) nothing
/// accumulates.
#[test]
fn per_iteration_temporary_reclaims_its_whole_tree() {
    let out = run(
        "hot_loop",
        &program(
            r#"
            fn spin() {
                let mut i = 0;
                while i < 32 {
                    println("t=", Holder { tag: "loop" }.tag);
                    i = i + 1;
                }
            }
            fn main() {
                spin();
            }
        "#,
        ),
    );
    assert_eq!(count(&out, "t=loop"), 32, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 32, "got:\n{out}");
    assert_eq!(count(&out, "leaf drained"), 32, "got:\n{out}");
}

/// The cascade's ORDER, at every level. Drain runs depth-first
/// (grandchild, child, self — spec/runtime.md); dissolve runs
/// outermost-first, so an owner's `dissolve()` body can still read
/// the fields it is about to release; each level's arena is
/// destroyed after its children's, since it holds their structs.
#[test]
fn cascade_order_is_depth_first_drain_and_outer_first_dissolve() {
    let out = run(
        "order",
        &program(
            r#"
            fn main() {
                println("start");
                println("a=", Holder { tag: "hello" }.tag);
            }
        "#,
        ),
    );
    let lines: Vec<&str> = out.lines().collect();
    let at = |needle: &str| {
        lines
            .iter()
            .position(|l| *l == needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in:\n{out}"))
    };
    assert!(at("leaf drained") < at("mid drained"), "got:\n{out}");
    assert!(at("mid drained") < at("holder drained"), "got:\n{out}");
    assert!(at("holder drained") < at("holder dissolved"), "got:\n{out}");
    assert!(at("holder dissolved") < at("mid dissolved"), "got:\n{out}");
    assert!(at("mid dissolved") < at("leaf dissolved"), "got:\n{out}");
}

/// A four-level tree: the cascade is recursive, not two levels
/// hard-coded.
#[test]
fn cascade_reaches_a_four_level_tree() {
    let out = run(
        "four_deep",
        &program(
            r#"
            locus Top {
                params { h: Holder = Holder { tag: "deep" }; }
                dissolve() { println("top dissolved"); }
            }
            fn main() {
                Top { };
            }
        "#,
        ),
    );
    assert_eq!(count(&out, "top dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "holder dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "mid dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 1, "got:\n{out}");
}

/// The F.29 ownership gate still holds at depth. A child handed in
/// from outside (`Mid { leaf: external }`) is not the holder's to
/// tear down: its real owner runs the teardown at ITS scope exit,
/// exactly once. Recursing must not turn that into a double
/// dissolve — nor drop it.
#[test]
fn an_externally_provided_grandchild_dissolves_exactly_once() {
    let out = run(
        "external_child",
        &program(
            r#"
            fn main() {
                let shared = Leaf { };
                let m = Mid { leaf: shared };
                println("a=", Holder { m: m, tag: "ext" }.tag);
                println("done");
            }
        "#,
        ),
    );
    assert!(out.contains("a=ext"), "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "mid dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "holder dissolved"), 1, "got:\n{out}");
    // The literal is fn-scope-owned (GH #711), so all three are torn
    // down at fn-scope exit, after `done`: the holder first, whose
    // cascade skips the borrowed `m`; then `m` and the leaf it
    // borrowed in turn, each by its own owner, in reverse
    // declaration order.
    let lines: Vec<&str> = out.lines().collect();
    let at = |needle: &str| {
        lines
            .iter()
            .position(|l| *l == needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in:\n{out}"))
    };
    assert!(at("done") < at("holder dissolved"), "got:\n{out}");
    assert!(at("holder dissolved") < at("mid dissolved"), "got:\n{out}");
    assert!(at("mid dissolved") < at("leaf dissolved"), "got:\n{out}");
}
