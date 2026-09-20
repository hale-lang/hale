//! GH #815 — a locus created in a LOOP is reclaimed when its slot
//! is reused, not only the last one at scope exit.
//!
//! A deferred-dissolve slot is one alloca per instantiation SITE,
//! and so is the locus struct it points at: both are hoisted to the
//! enclosing fn's entry block, so a site inside a loop rewrites the
//! same two slots every iteration. The scope-exit flush loads the
//! slot ONCE and therefore only ever saw the LAST instance —
//! `dissolve()` never ran for the other 31, and their arenas, their
//! `@form` buffers and their whole child trees were still live at
//! process exit. Pre-existing for `let` inside a loop (the rule was
//! recorded as "fn-level, not block-level, in v0"); PR #814 made
//! the same slot the owner of an unowned literal in EXPRESSION
//! position, which put the shape on the common path.
//!
//! Control arriving at a site for a second time is the proof that
//! the previous occupant is dead — the name it was bound to is
//! about to be rebound — so the instantiation now reclaims that
//! occupant first, with the same per-entry spine the flush emits
//! (drain → `__dissolve_closures` → dissolve → child cascade →
//! arena_destroy). The slot's NULL-self guard makes the first pass
//! a no-op, and the flush still owns the last instance, so a `let`
//! read AFTER the loop still names a live locus.
//!
//! The counts below are the assertion: N iterations must produce N
//! `dissolve()` calls at every level of the tree, not one. They are
//! stdout counts rather than a sanitizer verdict so the test bites
//! without one — but the same programs also run under
//! `LOTUS_ASAN=1` (`build_executable` reads the flag at codegen
//! time), where the 31 unreclaimed arenas are what LeakSanitizer
//! reports and a leak turns every `status.success()` here red. Note
//! GH #816: the chunk pool masks USE-AFTER-FREE from ASan, not
//! leaks — LSan does see these.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Compile `src`, run it, return its stdout. Asserts a clean exit —
/// which under `LOTUS_ASAN=1` is also the leak oracle.
fn run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("gh815_{}", name));
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

/// A two-level tree, so the count proves the CASCADE reaches the
/// child on the per-iteration path and not just the holder. The
/// leaf's `@form(vec)` grandchild allocates outside the arena (the
/// vec buffer is a realloc), which is what makes an unreclaimed
/// instance visible to LeakSanitizer as well as to stdout.
const TREE: &str = r#"
    type Row { v: Int = 0; }
    @form(vec)
    locus Rows { capacity { heap rows of Row; } }
    locus Leaf {
        params { rows: Rows = Rows { }; }
        birth() { self.rows.push(Row { v: 7 }); }
        dissolve() { println("leaf dissolved"); }
    }
    locus Holder {
        params { leaf: Leaf = Leaf { }; tag: String = "x"; }
        dissolve() { println("holder dissolved"); }
    }
"#;

fn program(body: &str) -> String {
    format!("{TREE}\n{body}\n")
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// The issue's reproducer: `let h = Holder { ... };` in a loop.
/// 32 instances, 32 dissolves — 31 as their slot is reused and the
/// last at fn exit.
#[test]
fn let_bound_locus_in_a_loop_dissolves_every_iteration() {
    let out = run(
        "let_loop",
        &program(
            r#"
            fn main() {
                let mut i = 0;
                while i < 32 {
                    let h = Holder { tag: "loop" };
                    i = i + 1;
                }
                println("done");
            }
        "#,
        ),
    );
    assert_eq!(count(&out, "holder dissolved"), 32, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 32, "got:\n{out}");
}

/// The expression-position twin (PR #814 gave it the same owner):
/// a literal nothing binds, read for a field inside the loop.
#[test]
fn expression_position_literal_in_a_loop_dissolves_every_iteration() {
    let out = run(
        "expr_loop",
        &program(
            r#"
            fn main() {
                let mut i = 0;
                while i < 32 {
                    println("t=", Holder { tag: "loop" }.tag);
                    i = i + 1;
                }
                println("done");
            }
        "#,
        ),
    );
    assert_eq!(count(&out, "t=loop"), 32, "got:\n{out}");
    assert_eq!(count(&out, "holder dissolved"), 32, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 32, "got:\n{out}");
}

/// The control: a bare `Holder { ... };` STATEMENT never goes
/// through expression lowering and never took a deferred slot, so
/// it kept its fire-and-forget teardown at the statement boundary
/// all along. Its count is unchanged — the fix must not turn the
/// eager path into a deferred one.
#[test]
fn bare_statement_control_is_unchanged() {
    let out = run(
        "bare_stmt",
        &program(
            r#"
            fn main() {
                let mut i = 0;
                while i < 32 {
                    Holder { tag: "loop" };
                    println("t=loop");
                    i = i + 1;
                }
                println("done");
            }
        "#,
        ),
    );
    assert_eq!(count(&out, "holder dissolved"), 32, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 32, "got:\n{out}");
    // Eager, so every dissolve lands BEFORE `done` — none is left
    // for the fn-exit flush.
    let at_done = out.find("done").expect("printed done");
    let after = &out[at_done..];
    assert_eq!(count(after, "holder dissolved"), 0, "got:\n{out}");
}

/// The last instance still belongs to the fn-exit flush, so a
/// `let` binding is readable AFTER the loop that made it: `let` is
/// fn-scoped, not block-scoped, and this fix does not change that.
/// (If the reclaim happened at the loop's back edge instead, this
/// read would be a use-after-free.)
#[test]
fn the_last_instance_outlives_the_loop() {
    let out = run(
        "last_outlives",
        &program(
            r#"
            fn main() {
                let mut i = 0;
                while i < 8 {
                    let h = Holder { tag: "kept" };
                    i = i + 1;
                }
                let h2 = Holder { tag: "after" };
                println("tag=", h2.tag);
                println("done");
            }
        "#,
        ),
    );
    assert!(out.contains("tag=after"), "got:\n{out}");
    // 8 from the loop + the one bound after it.
    assert_eq!(count(&out, "holder dissolved"), 9, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 9, "got:\n{out}");
    let at_done = out.find("done").expect("printed done");
    // Exactly two are left for the flush: the loop's last instance
    // and the one bound after it.
    assert_eq!(
        count(&out[at_done..], "holder dissolved"),
        2,
        "got:\n{out}"
    );
}

/// Nested loops: the inner site's slot is reused across BOTH loops
/// (it is one alloca in the fn's entry block, not one per outer
/// iteration), so its count is the product, and the outer site's is
/// the outer count.
#[test]
fn nested_loops_reclaim_at_every_level() {
    let out = run(
        "nested",
        &program(
            r#"
            locus Marker {
                params { tag: String = "m"; }
                dissolve() { println("outer dissolved"); }
            }
            fn main() {
                let mut i = 0;
                while i < 4 {
                    let mut j = 0;
                    while j < 8 {
                        let inner = Holder { tag: "inner" };
                        j = j + 1;
                    }
                    let outer = Marker { tag: "outer" };
                    i = i + 1;
                }
                println("done");
            }
        "#,
        ),
    );
    assert_eq!(count(&out, "holder dissolved"), 32, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 32, "got:\n{out}");
    assert_eq!(count(&out, "outer dissolved"), 4, "got:\n{out}");
}

/// Two `let`s of the SAME NAME in one fn, no loop: each is its own
/// instantiation site, so each mints its own slot and BOTH are torn
/// down at fn exit in reverse declaration order. A rebinding is not
/// a slot reuse — nothing here goes through the per-iteration path,
/// and the counts pin that it stays that way.
#[test]
fn rebinding_the_same_name_without_a_loop_dissolves_both() {
    let out = run(
        "rebind",
        &program(
            r#"
            fn main() {
                let h = Holder { tag: "first" };
                let h = Holder { tag: "second" };
                println("done");
            }
        "#,
        ),
    );
    assert_eq!(count(&out, "holder dissolved"), 2, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 2, "got:\n{out}");
    let at_done = out.find("done").expect("printed done");
    // Both belong to the flush — neither reclaims the other.
    assert_eq!(
        count(&out[at_done..], "holder dissolved"),
        2,
        "got:\n{out}"
    );
}

/// A loop whose instance is handed to an owner. The owner's F.29
/// `__locus_ref_owned_mask` bit stays CLEAR for an externally-
/// provided override, so the owner's cascade skips the handle and
/// the per-iteration reclaim is the only teardown: exactly one
/// dissolve per instance, at each level, never two.
///
/// One ordering note the counts deliberately do not pin: within an
/// iteration the reclaims run in SITE order (`moved`, then
/// `keeper`), where the fn-exit flush runs in reverse push order
/// (`keeper`, then `moved`). Each site can only reclaim its own
/// previous occupant, at its own point in the body. That is not a
/// double dissolve and not a use-after-free — the mask keeps the
/// two teardowns disjoint — but a `dissolve()` body that reads a
/// sibling handle it does not own reads the sibling's NEXT
/// instance on the per-iteration path.
#[test]
fn an_instance_moved_into_an_owner_does_not_double_dissolve() {
    let out = run(
        "moved",
        &program(
            r#"
            locus Keeper {
                params { held: Holder = Holder { tag: "own" }; }
                dissolve() { println("keeper dissolved"); }
            }
            fn main() {
                let mut i = 0;
                while i < 8 {
                    let moved = Holder { tag: "moved" };
                    let keeper = Keeper { held: moved };
                    i = i + 1;
                }
                println("done");
            }
        "#,
        ),
    );
    // The override replaces the `Holder { tag: "own" }` default, so
    // there is exactly one Holder per iteration and one Keeper.
    assert_eq!(count(&out, "keeper dissolved"), 8, "got:\n{out}");
    assert_eq!(count(&out, "holder dissolved"), 8, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 8, "got:\n{out}");
}

/// The other deferred slot with the same shape (GH #383): a `let`
/// bound to a proven-fresh locus FACTORY uses the binding's own
/// alloca as its dissolve slot, which `alloca_for` also hoists to
/// the entry block. In a loop it leaked exactly the same way.
#[test]
fn a_factory_result_bound_in_a_loop_dissolves_every_iteration() {
    let out = run(
        "factory_loop",
        &program(
            r#"
            fn make() -> Holder {
                let h = Holder { tag: "made" };
                return h;
            }
            fn main() {
                let mut i = 0;
                while i < 16 {
                    let w = make();
                    i = i + 1;
                }
                println("done");
            }
        "#,
        ),
    );
    assert_eq!(count(&out, "holder dissolved"), 16, "got:\n{out}");
    assert_eq!(count(&out, "leaf dissolved"), 16, "got:\n{out}");
}

// === GH #826: the pinned residue ===================================
//
// One shape is excluded from the reclaim above: a PINNED locus. Its
// deferred entry carries a `pthread_t`, so reclaiming it at slot
// reuse means joining the previous thread — a behaviour change #815
// deliberately left alone. `placement { }` is main-only, so the
// reachable shape is a main locus with a pinned entry instantiated
// inside a loop, and it leaked N-1 arenas and orphaned N-1 threads
// (LSan: "Direct leak of N objects ... lotus_arena_create_labeled").
//
// `check_pinned_locus_in_loop` now rejects that program with a
// located diagnostic. `build_executable` does NOT run the checker,
// so codegen keeps a backstop: refuse the lowering rather than emit
// the leak. This is that backstop.

/// A main locus that pins a field, instantiated inside a loop.
fn pinned_in_loop_src(loop_body: &str) -> String {
    format!(
        r#"
        locus Worker {{
            params {{ id: Int = 0; }}
            run() {{ println("worker ", self.id); }}
        }}

        main locus App {{
            params {{
                w: Worker = Worker {{ id: 1 }};
            }}
            placement {{
                w: pinned;
            }}
            run() {{ println("app"); }}
        }}

        fn main() {{
            let mut i = 0;
            while i < 4 {{
                {}
                i = i + 1;
            }}
            return 0;
        }}
    "#,
        loop_body
    )
}

#[test]
fn codegen_refuses_a_pinned_locus_lowered_inside_a_loop() {
    let src = pinned_in_loop_src("App { };");
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin("gh826_pinned_loop");
    let err = build_executable(&program, &bin)
        .expect_err("a pinned locus in a loop must not build");
    let _ = std::fs::remove_file(&bin);
    let msg = err.to_string();
    assert!(
        msg.contains("pinned locus `Worker` is instantiated inside a loop"),
        "expected the GH #826 backstop, got: {msg}"
    );
    assert!(
        msg.contains("Instantiate it once outside the loop"),
        "the backstop should name the fix: {msg}"
    );
}

/// The control that keeps the backstop honest: the same program with
/// the instantiation hoisted out of the loop still builds and runs.
/// Without it, "refuses" could mean "refuses every pinned program".
#[test]
fn a_pinned_locus_outside_a_loop_still_builds() {
    let src = r#"
        locus Worker {
            params { id: Int = 0; }
            run() { println("worker ", self.id); }
        }

        main locus App {
            params {
                w: Worker = Worker { id: 1 };
            }
            placement {
                w: pinned;
            }
            run() { println("app"); }
        }

        fn main() {
            App { };
            return 0;
        }
    "#;
    let out = run("gh826_pinned_no_loop", src);
    assert_eq!(count(&out, "worker 1"), 1, "got:\n{out}");
    assert_eq!(count(&out, "app"), 1, "got:\n{out}");
}
