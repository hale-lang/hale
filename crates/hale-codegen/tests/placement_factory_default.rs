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
//! Rule 18 refuses it with a located diagnostic — the user-facing
//! half, tested in `hale-types/tests/placement.rs`. It is a law over
//! the placement table (`hale_types::lowering_laws`, F.40 phase 3, C7)
//! that the harness's lowering view demands too, since
//! `build_executable` does not run the checker: this file pins that a
//! harness build is refused before lowering, with the law's wording,
//! rather than emit a program whose stated placement quietly does
//! nothing. Lowering keeps no refusal of its own.

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

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
    let err = build_executable_with_options(&program, &bin, &[], &build_opts::options())
        .expect_err("a dropped placement must not build");
    let _ = std::fs::remove_file(&bin);
    let msg = err.to_string();
    assert!(
        msg.contains("placement entry `a` names a field no locus literal initialises: `a`'s default is a call"),
        "expected the rule 18 law's refusal, got: {msg}"
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
    build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("build");
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
/// no expression, and lowering's own GH #890 refusal only fired for an
/// initialiser it could see, so a `placement { }` entry on a
/// scalar-defaulted field was neither consumed nor refused by a
/// harness build, which skips the checker.
///
/// Since F.40 phase 3, C7 rule 18 is a law the harness demands too,
/// and it refuses this program before lowering: the scalar default is
/// not a locus literal. So no entry point lowers a placed field whose
/// initialiser lowers no literal, and the override's per-field scope
/// (the fix) has no program left to reach it. The program is pinned
/// as the law's: the shape that leaked the override is refused.
/// The program is assembled without a raw string literal so
/// `hale_corpus::embedded` does not harvest it.
#[test]
fn a_placement_entry_on_a_scalar_field_is_refused_before_lowering() {
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
    let err = build_executable_with_options(&program, &bin, &[], &build_opts::options())
        .expect_err("a placement entry on a scalar field must not reach lowering");
    let _ = std::fs::remove_file(&bin);
    let msg = err.to_string();
    assert!(
        msg.contains("placement entry `slot` names a field no locus literal initialises: `slot`'s default"),
        "expected the rule 18 law's refusal, got: {msg}"
    );
}

/// `Worker`, a factory for it, and `App` pinning its `w`, then `rest`.
/// Assembled without a raw string literal so the corpus does not harvest
/// the refused programs.
fn root_then(rest: &[&str]) -> String {
    let mut src = vec![
        "locus Worker { run() { print(\"worker\"); } }\n",
        "fn make_worker() -> Worker { Worker { } }\n",
        "main locus App {\n",
        "    params { w: Worker = Worker { }; }\n",
        "    placement { w: pinned; }\n",
        "    run() { print(\"app\"); }\n",
        "}\n",
    ];
    src.extend_from_slice(rest);
    src.concat()
}

const RULE_18_AT_SITE: &str =
    "placement entry `w` names a field no locus literal initialises: the value supplied for `w` here is a call";

fn harness_refusal(tag: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(tag);
    let err = build_executable_with_options(&program, &bin, &[], &build_opts::options())
        .expect_err("a dropped placement must not build");
    let _ = std::fs::remove_file(&bin);
    err.to_string()
}

/// The review of PR #1338: a root literal in another locus's params
/// default. Lowering expands it wherever `Holder` is built, and its
/// factory call takes no entry, so `w` would run unplaced: before C7
/// lowering refused it (the GH #890 backstop), and once that backstop
/// was deleted it built with no thread. The placement table records the
/// literal as the root's (`expanded`), and the law refuses it before
/// lowering.
#[test]
fn a_root_literal_in_a_params_default_overriding_a_placed_field_is_refused() {
    let msg = harness_refusal(
        "hale_placement_root_in_default_1338",
        &root_then(&[
            "locus Holder { params { app: App = App { w: make_worker() }; } }\n",
            "fn main() { Holder { }; }\n",
        ]),
    );
    assert!(msg.contains(RULE_18_AT_SITE), "expected the rule 18 law's refusal, got: {msg}");
}

/// The same literal two params defaults deep: `Outer`'s default builds a
/// `Holder`, whose default builds the root.
#[test]
fn a_root_literal_two_params_defaults_deep_is_refused() {
    let msg = harness_refusal(
        "hale_placement_root_two_deep_1338",
        &root_then(&[
            "locus Holder { params { app: App = App { w: make_worker() }; } }\n",
            "locus Outer { params { h: Holder = Holder { }; } }\n",
            "fn main() { Outer { }; }\n",
        ]),
    );
    assert!(msg.contains(RULE_18_AT_SITE), "expected the rule 18 law's refusal, got: {msg}");
}

/// A default that calls a free fn which builds the root: the fn's body is
/// a scope, so its literal is a construction of the root already.
#[test]
fn a_root_built_in_a_fn_a_params_default_calls_is_refused() {
    let msg = harness_refusal(
        "hale_placement_root_via_fn_1338",
        &root_then(&[
            "fn make_app() -> App { App { w: make_worker() } }\n",
            "locus Holder { params { app: App = make_app(); } }\n",
            "fn main() { Holder { }; }\n",
        ]),
    );
    assert!(msg.contains(RULE_18_AT_SITE), "expected the rule 18 law's refusal, got: {msg}");
}

/// The control: a root literal in a params default that spells the
/// placed field as a literal consumes the entry. It builds, its `w` gets
/// its own thread (one `pthread_create` of `Worker`'s pinned start, whose
/// readiness is awaited), and both run.
#[test]
fn a_root_literal_in_a_params_default_consuming_the_entry_is_pinned() {
    let stdout = builds_with_one_pinned_thread(
        "hale_placement_root_in_default_pinned_1338",
        &root_then(&[
            "locus Holder { params { app: App = App { w: Worker { } }; } }\n",
            "fn main() { Holder { }; }\n",
        ]),
    );
    assert!(stdout.contains("app") && stdout.contains("worker"), "both halves should run: {stdout:?}");
}

/// `src` builds, its `w` gets its own thread (one `pthread_create` of
/// `Worker`'s pinned start, whose readiness is awaited), and it runs to a
/// clean exit; its stdout.
fn builds_with_one_pinned_thread(tag: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(tag);
    let ll = bin.with_extension("ll");
    let opts = hale_codegen::BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
    build_executable_with_options(&program, &bin, &[], &opts).expect("build");
    let ir = std::fs::read_to_string(&ll).expect("read IR");
    let out = std::process::Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    let spawns: Vec<&str> = ir
        .lines()
        .filter(|l| l.contains("call i32 @pthread_create(") && l.contains("@__pinned_main_Worker"))
        .collect();
    assert_eq!(spawns.len(), 1, "one pinned thread for `w`: {spawns:?}");
    assert!(ir.contains("call void @lotus_pinned_start_await_ready("), "the pinned start is awaited");
    assert!(out.status.success(), "non-zero exit");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const RULE_17: &str = "locus `App` is instantiated inside a loop, but its `placement { }` block pins field `w`";

/// The review of PR #1338, rule 17: a root literal in another locus's
/// params default is built wherever that locus is, so `Holder { }` in a
/// loop builds `App` per iteration and spawns `w`'s pinned thread each
/// time, joining only the last. Before C7 lowering refused it (the
/// GH #826 backstop: a pinned instantiation with a loop open); the law
/// judged only the root's constructions' loops, so with the backstop
/// deleted it built. The expanded literal inherits the loop of the
/// literal that builds it.
#[test]
fn a_root_literal_in_a_params_default_built_in_a_loop_is_refused() {
    let msg = harness_refusal(
        "hale_placement_root_default_in_loop_1338",
        &root_then(&[
            "locus Holder { params { app: App = App { w: Worker { } }; } }\n",
            "fn main() { for i in 0..3 { Holder { }; } }\n",
        ]),
    );
    assert!(msg.contains(RULE_17), "expected the rule 17 law's refusal, got: {msg}");
}

/// Two params defaults deep: `Shell`'s default builds a `Holder`, whose
/// default builds the root, and `Shell { }` is built in a loop.
#[test]
fn a_root_literal_two_params_defaults_deep_built_in_a_loop_is_refused() {
    let msg = harness_refusal(
        "hale_placement_root_two_deep_in_loop_1338",
        &root_then(&[
            "locus Holder { params { app: App = App { w: Worker { } }; } }\n",
            "locus Shell { params { inner: Holder = Holder { }; } }\n",
            "fn main() { for i in 0..3 { Shell { }; } }\n",
        ]),
    );
    assert!(msg.contains(RULE_17), "expected the rule 17 law's refusal, got: {msg}");
}

/// The control: the same program with `Holder { }` built outside the
/// loop builds, with one pinned thread, and runs.
#[test]
fn a_root_literal_in_a_params_default_built_outside_a_loop_is_pinned() {
    let stdout = builds_with_one_pinned_thread(
        "hale_placement_root_default_out_of_loop_1338",
        &root_then(&[
            "locus Holder { params { app: App = App { w: Worker { } }; } }\n",
            "fn main() { Holder { }; for i in 0..3 { print(\"tick\"); } }\n",
        ]),
    );
    assert!(stdout.contains("app") && stdout.contains("worker"), "both halves should run: {stdout:?}");
    assert_eq!(stdout.matches("tick").count(), 3, "the loop ran: {stdout:?}");
}

/// Rule 17 at the positions lowering emits at every use: a const's
/// value (at each read), a type's field default (at each literal of the
/// type taking it), a closure's assertion (at each evaluation). Each
/// program below builds `App` once per iteration of `main`'s loop.
/// Before C7 lowering refused the const and the type default (the
/// GH #826 backstop saw the loop open where it re-lowered the literal)
/// and built the closure's, whose evaluation runs where no loop is
/// open; with the backstop deleted all of them built. The placement
/// table records no construction for these positions, so the law
/// refuses a pinned root written in one outright.
fn per_use_positions() -> [(&'static str, &'static str, Vec<&'static str>); 4] {
    [
        ("a `const`'s value", "const", vec!["const C: App = App { };\n", "fn main() { for i in 0..3 { let a = C; } }\n"]),
        (
            "a `const`'s value",
            "through_const",
            vec![
                "locus Holder { params { app: App = App { }; } }\n",
                "const C: Holder = Holder { };\n",
                "fn main() { for i in 0..3 { let h = C; } }\n",
            ],
        ),
        (
            "a type's field default",
            "type_default",
            vec![
                "fn n(a: App) -> Int { return 1; }\n",
                "type Box { k: Int = n(App { }); }\n",
                "fn main() { for i in 0..3 { let b = Box { }; } }\n",
            ],
        ),
        (
            "a closure's assertion",
            "closure",
            vec![
                "fn n(a: App) -> Int { return 1; }\n",
                "locus Holder { params { x: Int = 1; } closure c { n(App { }) ~~ self.x within 0; } }\n",
                "fn main() { for i in 0..3 { Holder { }; } }\n",
            ],
        ),
    ]
}

#[test]
fn a_pinned_root_in_a_position_emitted_at_every_use_is_refused() {
    for (position, tag, rest) in per_use_positions() {
        let msg = harness_refusal(&format!("hale_placement_root_per_use_{tag}_1338"), &root_then(&rest));
        let want = format!("locus `App` is built by this literal, written in {position}");
        assert!(msg.contains(&want), "{tag}: expected the rule 17 law's refusal, got: {msg}");
    }
}

/// The controls: the same programs with no `pinned` entry build and
/// run, building `App` once per iteration.
#[test]
fn a_root_with_no_pinned_entry_in_a_position_emitted_at_every_use_builds() {
    for (_, tag, rest) in per_use_positions() {
        let src = root_then(&rest);
        let unpinned = src.replace("    placement { w: pinned; }\n", "");
        assert_ne!(unpinned, src, "{tag}: the entry was removed");
        let program = hale_syntax::parse_source(&unpinned).expect("parse");
        let bin = harness::unique_bin(&format!("hale_placement_root_per_use_control_{tag}_1338"));
        build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("build");
        let out = std::process::Command::new(&bin).output().expect("run");
        let _ = std::fs::remove_file(&bin);
        assert!(out.status.success(), "{tag}: non-zero exit");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(stdout.matches("app").count(), 3, "{tag}: `App` built per iteration: {stdout:?}");
    }
}
