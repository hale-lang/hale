//! GH #890 — a `placement { }` entry a factory-built field drops.
//!
//! The A/B in the issue is one program with only the default's
//! spelling changed:
//!
//! ```text
//! a: Worker = Worker { };      // pinned: `a` runs on its own thread
//! a: Worker = make_worker();   // silently NOT pinned
//! ```
//!
//! The literal spelling printed `app run` before the worker's lines
//! and carried one more `pthread_create` call site in the binary; the
//! factory spelling ran in strict declaration order with no extra
//! thread and no diagnostic. The entry is an override on the NEXT
//! locus literal lowered for the field
//! (`placement_for_next_locus_instantiation` plus the parallel pool
//! and NUMA-node slots); a call lowers none, and the next field's turn
//! through the params-init loop resets it.
//!
//! Applying it afterwards is not on the table: the pinned path is
//! "spawn a pthread that runs the whole lifecycle on it", and the
//! factory's literal has already run birth and `run()` before the
//! value comes back. So the shape is refused.
//!
//! `check_placement_entry_consumed` refuses it with a located
//! diagnostic — that is the user-facing half, tested in
//! `hale-types/tests/placement.rs`. This file pins the codegen
//! backstop: `build_executable` does not run the checker, so an
//! embedder that bypasses it must be refused here rather than emit a
//! program whose stated placement quietly does nothing.

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn app(default: &str, extra_fn: &str) -> String {
    format!(
        r#"
        locus Worker {{
            run() {{ print("worker"); }}
        }}

        {extra_fn}

        main locus App {{
            params {{
                a: Worker = {default};
                b: Int = 7;
            }}
            placement {{
                a: pinned;
            }}
            run() {{ print("app run"); }}
        }}

        fn main() {{ App {{ }}; }}
        "#
    )
}

#[test]
fn factory_default_under_a_placement_entry_is_refused() {
    let src = app(
        "make_worker()",
        "fn make_worker() -> Worker { Worker { } }",
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin("hale_placement_factory_890");
    let err = build_executable(&program, &bin)
        .expect_err("a dropped placement must not build");
    let _ = std::fs::remove_file(&bin);
    let msg = err.to_string();
    assert!(
        msg.contains("placement") && msg.contains("not a locus literal"),
        "expected the GH #890 refusal, got: {msg}"
    );
}

#[test]
fn locus_literal_default_under_a_placement_entry_still_builds() {
    // The control half of the A/B — the same program with the default
    // spelled as the literal, which is the form that carries the
    // entry. The backstop must not touch it.
    let src = app("Worker { }", "");
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin("hale_placement_literal_890");
    build_executable(&program, &bin).expect("build");
    let out = std::process::Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app run") && stdout.contains("worker"),
        "both halves should run: {stdout:?}"
    );
}

/// GH #921 A3, commit 5 (PR #919's note): the placement override
/// must not outlive the field it was looked up for.
///
/// It used to be a slot the NEXT locus literal lowered took. The
/// params-init loop set it per field, and for every field but the
/// LAST the next turn through the loop reset it — so a placed field
/// whose initialiser never reaches `lower_locus_instantiation` left a
/// pinned `ScheduleClass` behind and the next instantiation anywhere
/// took it. `DefaultInit::Const` is that shape: `const_param` lowers
/// no expression, and the GH #890 backstop only fires for an
/// initialiser it can see, so a `placement { }` entry on a
/// scalar-defaulted field is neither consumed nor refused. The
/// checker rejects placement on a non-locus field, but
/// `build_executable` does not run the checker — which is what this
/// whole file is about.
///
/// The oracle is the GH #826 refusal: a PINNED locus instantiated
/// inside a loop cannot build, because its thread's join record is
/// one slot per site. So if the stale override reaches the loop's
/// literal the build is refused, and the fix is the build succeeding.
/// The program is assembled without a raw string literal so
/// `hale_corpus::embedded` does not harvest it.
#[test]
fn a_placement_entry_does_not_reach_a_later_instantiation() {
    let src = [
        "locus Worker {\n",
        "    run() { print(\"worker\"); }\n",
        "}\n",
        "locus Other {\n",
        "    params { n: Int = 0; }\n",
        "    fn v() -> Int { return self.n + 1; }\n",
        "}\n",
        // `slot` is the LAST params field and carries the entry, and
        // its default is a scalar — so nothing lowers an expression
        // for it and nothing resets the override after it.
        "main locus App {\n",
        "    params {\n",
        "        w: Worker = Worker { };\n",
        "        slot: Int = 5;\n",
        "    }\n",
        "    placement {\n",
        "        slot: pinned;\n",
        "    }\n",
        "    run() { print(\"app run\"); }\n",
        "}\n",
        "fn main() {\n",
        "    App { };\n",
        "    let mut i = 0;\n",
        "    while i < 2 {\n",
        "        println(\"o=\", Other { n: i }.v());\n",
        "        i = i + 1;\n",
        "    }\n",
        "}\n",
    ]
    .concat();
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin("hale_placement_slot_scope_921");
    let built = build_executable(&program, &bin);
    assert!(
        built.is_ok(),
        "a placement entry on an earlier field must not pin a later \
         instantiation — the GH #826 loop refusal is the tell: {:?}",
        built.err()
    );
    let out = std::process::Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("o=1") && stdout.contains("o=2"),
        "both iterations should run: {stdout:?}"
    );
}
