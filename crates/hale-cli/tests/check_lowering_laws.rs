//! F.40 phase 3, C7: the laws that replaced lowering's backstops are
//! judged at the production entry points, located.
//!
//! Lowering used to refuse these shapes itself, without a span, and
//! `hale check` accepted them. The laws (`hale_types::lowering_laws`)
//! run among the check's rules, and `hale build` checks before it
//! lowers, so both verbs refuse with the law's span and wording. The
//! harness's entry, which skips the check, is pinned in
//! `hale-codegen/tests/harness_lowering_laws.rs`.
//!
//! The programs are plain escaped string literals: `hale-corpus`
//! harvests `r#"…"#` literals out of test files into the corpus-wide
//! properties, and these are written to be refused.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Rule 6: an adapter inline in `bindings { }` runs pinned, and this
/// one accepts children. The binding entry is line 6, column 16.
const ADAPTER_ACCEPTS: &str = "type Tick { n: Int; }\n\
     topic Beat { payload: Tick; subject: \"beat\"; }\n\
     locus Child { run() { } }\n\
     locus Sink { accept(c: Child) { } fn send(subject: String, bytes: Bytes) { } }\n\
     locus Pub { bus { publish Beat; } run() { Beat <- Tick { n: 1 }; } }\n\
     main locus App { params { p: Pub = Pub { }; } bindings { Beat: Sink { }; } }\n\
     fn main() { App { }; }\n";

const RULE_6: &str = "adapter binding for topic `Beat`: `Sink` runs on its own pinned thread";

/// Rule 17 (GH #826): a root that pins a field, built inside a loop. The
/// literal is line 4, column 21.
const PINNED_ROOT_IN_A_LOOP: &str = "locus Worker { run() { } }\n\
     main locus App { params { w: Worker = Worker { }; } placement { w: pinned; } }\n\
     fn main() {\n\
     \x20   for i in 0..3 { App { }; }\n\
     }\n";

const RULE_17: &str = "locus `App` is instantiated inside a loop, but its `placement { }` block pins field `w`";

/// A cross-pool spawn used as a value: `Driver` runs on pool `workers`,
/// `World` (main, a singleton) accepts `Ship`. The literal is line 2,
/// column 32.
const XPOOL_VALUE: &str = "locus Ship { params { hull: Int = 0; } }\n\
     locus Driver { run() { let s = Ship { hull: 7 }; } }\n\
     main locus World { params { driver: Driver = Driver { }; } placement { driver: cooperative(pool = workers); } \
     accept(s: Ship) { } run() { } }\n\
     fn main() { World { }; }\n";

const FIRE_AND_FORGET: &str = "cross-pool spawn `Ship{ }` is fire-and-forget";

/// A cross-pool spawn in another locus's params default (C3 rest):
/// `Driver` builds a `Holder` that leaves `s` to its default, which
/// lowering expands under `Driver`, where `Ship` is a cross-pool birth.
/// The default's literal is line 2, column 35.
const XPOOL_DEFAULT: &str = "locus Ship { params { hull: Int = 0; } }\n\
     locus Holder { params { s: Ship = Ship { hull: 1 }; } }\n\
     locus Driver { run() { Ship { hull: 7 }; Holder { }; } }\n\
     main locus World { params { driver: Driver = Driver { }; } placement { driver: cooperative(pool = workers); } \
     accept(s: Ship) { } run() { } }\n\
     fn main() { World { }; }\n";

const FIRE_AND_FORGET_DEFAULT: &str =
    "cross-pool spawn `Ship{ }` is fire-and-forget: it is the default of `Holder`'s param `s`, which is built in `Driver`";

/// The review of #1351: a cross-pool spawn in a fn's argument default.
/// `take()` leaves `s` to its default, which lowering expands at the call,
/// under `Driver`, where `Ship` is a cross-pool birth (lowering used to
/// pass a null pointer for it). The call is line 3, column 42.
const XPOOL_ARG_DEFAULT: &str = "locus Ship { params { hull: Int = 0; } }\n\
     fn take(s: Ship = Ship { hull: 1 }) { println(s.hull); }\n\
     locus Driver { run() { Ship { hull: 7 }; take(); } }\n\
     main locus World { params { driver: Driver = Driver { }; } placement { driver: cooperative(pool = workers); } \
     accept(s: Ship) { } run() { } }\n\
     fn main() { World { }; }\n";

/// The control: the same call with the argument supplied, a `Ship` built
/// on `World`'s thread and handed to `Driver`. `World` waits for the
/// bare spawn's `Ship` to arrive before it returns, as the cross-pool
/// bubble's own runtime tests do (a `World` that returns first is gone
/// when the post lands, defaults or none).
const XPOOL_ARG_SUPPLIED: &str = "locus Ship { params { hull: Int = 0; } }\n\
     fn take(s: Ship = Ship { hull: 1 }) { println(s.hull); }\n\
     locus Driver { params { got: Ship; } run() { Ship { hull: 7 }; take(self.got); } }\n\
     main locus World { params { driver: Driver = Driver { got: Ship { hull: 5 } }; } \
     placement { driver: cooperative(pool = workers); } accept(s: Ship) { } \
     mode harmonic() -> Int { let mut n: Int = 0; for child in self.children { n = n + 1; } return n; } \
     run() { let mut waited: Int = 0; while self.harmonic() < 1 && waited < 120 { std::time::sleep(100ms); \
     waited = waited + 1; } } }\n\
     fn main() { World { }; }\n";

/// A method's default, called on a receiver. The call is line 3, column 60.
const XPOOL_METHOD_DEFAULT: &str = "locus Ship { params { hull: Int = 0; } }\n\
     locus Tool { fn use_it(s: Ship = Ship { hull: 2 }) { println(s.hull); } }\n\
     locus Driver { run() { Ship { hull: 7 }; let t = Tool { }; t.use_it(); } }\n\
     main locus World { params { driver: Driver = Driver { }; } placement { driver: cooperative(pool = workers); } \
     accept(s: Ship) { } run() { } }\n\
     fn main() { World { }; }\n";

/// Transitive: `outer()` leaves `n` to `inner()`, which leaves `s` to the
/// `Ship`. The call in `Driver` is line 4, column 42.
const XPOOL_DEFAULT_CHAIN: &str = "locus Ship { params { hull: Int = 0; } }\n\
     fn inner(s: Ship = Ship { hull: 1 }) -> Int { return s.hull; }\n\
     fn outer(n: Int = inner()) { println(n); }\n\
     locus Driver { run() { Ship { hull: 7 }; outer(); } }\n\
     main locus World { params { driver: Driver = Driver { }; } placement { driver: cooperative(pool = workers); } \
     accept(s: Ship) { } run() { } }\n\
     fn main() { World { }; }\n";

/// Two callers of one default: `Driver` (pool `workers`) crosses, `Local`
/// (on `World`'s thread) does not.
const XPOOL_ARG_TWO_CALLERS: &str = "locus Ship { params { hull: Int = 0; } }\n\
     fn take(s: Ship = Ship { hull: 1 }) { println(s.hull); }\n\
     locus Driver { run() { Ship { hull: 7 }; take(); } }\n\
     locus Local { run() { take(); } }\n\
     main locus World { params { driver: Driver = Driver { }; local: Local = Local { }; } \
     placement { driver: cooperative(pool = workers); } accept(s: Ship) { } run() { } }\n\
     fn main() { World { }; }\n";

fn fire_and_forget_arg(callee: &str, param: &str) -> String {
    format!(
        "cross-pool spawn `Ship{{ }}` is fire-and-forget: it is the default of `{callee}`'s argument `{param}`, \
         expanded here, in `Driver`"
    )
}

/// Rule 18 (GH #890), the review of PR #1338: a root literal written in
/// another locus's params default overrides the placed field with a
/// factory call. The override is line 8, column 45.
const ROOT_IN_A_DEFAULT: &str = "locus Worker { run() { print(\"worker\"); } }\n\
     fn make_worker() -> Worker { Worker { } }\n\
     main locus App {\n\
     \x20   params { w: Worker = Worker { }; }\n\
     \x20   placement { w: pinned; }\n\
     \x20   run() { print(\"app\"); }\n\
     }\n\
     locus Holder { params { app: App = App { w: make_worker() }; } }\n\
     fn main() { Holder { }; }\n";

/// The same literal two params defaults deep (`Outer` builds `Holder`).
const ROOT_TWO_DEFAULTS_DEEP: &str = "locus Worker { run() { print(\"worker\"); } }\n\
     fn make_worker() -> Worker { Worker { } }\n\
     main locus App {\n\
     \x20   params { w: Worker = Worker { }; }\n\
     \x20   placement { w: pinned; }\n\
     \x20   run() { print(\"app\"); }\n\
     }\n\
     locus Holder { params { app: App = App { w: make_worker() }; } }\n\
     locus Outer { params { h: Holder = Holder { }; } }\n\
     fn main() { Outer { }; }\n";

const RULE_18: &str =
    "placement entry `w` names a field no locus literal initialises: the value supplied for `w` here is a call";

/// Rule 17, the review of PR #1338: a root literal in another locus's
/// params default inherits the loop of the literal that builds it. The
/// loop's `Holder { }` is line 9, column 21.
const ROOT_DEFAULT_IN_A_LOOP: &str = "locus Worker { run() { print(\"worker\"); } }\n\
     main locus App {\n\
     \x20   params { w: Worker = Worker { }; }\n\
     \x20   placement { w: pinned; }\n\
     \x20   run() { print(\"app\"); }\n\
     }\n\
     locus Holder { params { app: App = App { w: Worker { } }; } }\n\
     fn main() {\n\
     \x20   for i in 0..3 { Holder { }; }\n\
     }\n";

/// Two params defaults deep: the loop builds `Shell`, whose default builds
/// `Holder`. The loop's `Shell { }` is line 10, column 21.
const ROOT_DEFAULT_TWO_DEEP_IN_A_LOOP: &str = "locus Worker { run() { print(\"worker\"); } }\n\
     main locus App {\n\
     \x20   params { w: Worker = Worker { }; }\n\
     \x20   placement { w: pinned; }\n\
     \x20   run() { print(\"app\"); }\n\
     }\n\
     locus Holder { params { app: App = App { w: Worker { } }; } }\n\
     locus Shell { params { inner: Holder = Holder { }; } }\n\
     fn main() {\n\
     \x20   for i in 0..3 { Shell { }; }\n\
     }\n";

/// Rule 17, the positions lowering emits at every use: a root literal
/// in a const's value is lowered again at each read of the const, so
/// the loop's `C` builds `App` per iteration. The const's `App { }` is
/// line 7, column 16.
const ROOT_IN_A_CONST: &str = "locus Worker { run() { print(\"worker\"); } }\n\
     main locus App {\n\
     \x20   params { w: Worker = Worker { }; }\n\
     \x20   placement { w: pinned; }\n\
     \x20   run() { print(\"app\"); }\n\
     }\n\
     const C: App = App { };\n\
     fn main() {\n\
     \x20   for i in 0..3 { let a = C; }\n\
     }\n";

/// Through a params default: the const builds `Holder`, whose default
/// builds the root. The const's `Holder { }` is line 8, column 19.
const ROOT_THROUGH_A_CONST: &str = "locus Worker { run() { print(\"worker\"); } }\n\
     main locus App {\n\
     \x20   params { w: Worker = Worker { }; }\n\
     \x20   placement { w: pinned; }\n\
     \x20   run() { print(\"app\"); }\n\
     }\n\
     locus Holder { params { app: App = App { }; } }\n\
     const C: Holder = Holder { };\n\
     fn main() {\n\
     \x20   for i in 0..3 { let h = C; }\n\
     }\n";

/// A type's field default, lowered at each literal of the type that
/// takes it. The default's `App { }` is line 8, column 23.
const ROOT_IN_A_TYPE_DEFAULT: &str = "locus Worker { run() { print(\"worker\"); } }\n\
     main locus App {\n\
     \x20   params { w: Worker = Worker { }; }\n\
     \x20   placement { w: pinned; }\n\
     \x20   run() { print(\"app\"); }\n\
     }\n\
     fn n(a: App) -> Int { return 1; }\n\
     type Box { k: Int = n(App { }); }\n\
     fn main() {\n\
     \x20   for i in 0..3 { let b = Box { }; }\n\
     }\n";

/// A closure's assertion, lowered at each evaluation of the closure.
/// The assertion's `App { }` is line 8, column 53.
const ROOT_IN_A_CLOSURE: &str = "locus Worker { run() { print(\"worker\"); } }\n\
     main locus App {\n\
     \x20   params { w: Worker = Worker { }; }\n\
     \x20   placement { w: pinned; }\n\
     \x20   run() { print(\"app\"); }\n\
     }\n\
     fn n(a: App) -> Int { return 1; }\n\
     locus Holder { params { x: Int = 1; } closure c { n(App { }) ~~ self.x within 0; } }\n\
     fn main() {\n\
     \x20   for i in 0..3 { Holder { }; }\n\
     }\n";

const RULE_17_PER_USE: &str = "locus `App` is built by this literal, written in";

fn seed(tag: &str, src: &str) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_check_lowering_laws_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    std::fs::write(d.join("main.hl"), src).expect("write");
    d
}

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn check_refuses_a_pinned_adapter_that_accepts_at_the_binding_entry() {
    let d = seed("check", ADAPTER_ACCEPTS);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(!ok, "check must fail:\n{out}");
    assert!(out.contains(RULE_6) && out.contains("(rule 6)"), "{out}");
    assert!(out.contains(":6:"), "the span is the binding entry's line:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn build_refuses_it_at_the_check_with_the_same_span() {
    let d = seed("build", ADAPTER_ACCEPTS);
    let bin = d.join("out");
    let (ok, out) = hale(&["build", &d.to_string_lossy(), "-o", &bin.to_string_lossy()]);
    assert!(!ok, "build must fail:\n{out}");
    assert!(out.contains(RULE_6) && out.contains(":6:"), "{out}");
    assert!(!bin.exists(), "nothing was lowered");
    let _ = std::fs::remove_dir_all(&d);
}

/// `hale check` and `hale build` both refuse `src` with `message` at
/// `at` (`:line:col:`), and nothing is lowered.
fn both_verbs_refuse(tag: &str, src: &str, message: &str, at: &str) {
    for verb in ["check", "build"] {
        let d = seed(&format!("{tag}_{verb}"), src);
        let bin = d.join("out");
        let dir = d.to_string_lossy().to_string();
        let out_path = bin.to_string_lossy().to_string();
        let args: Vec<&str> = match verb {
            "check" => vec!["check", &dir],
            _ => vec!["build", &dir, "-o", &out_path],
        };
        let (ok, out) = hale(&args);
        assert!(!ok, "{verb} must fail:\n{out}");
        assert!(out.contains(message) && out.contains(at), "{verb}: located at the literal:\n{out}");
        assert!(!bin.exists(), "nothing was lowered");
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[test]
fn check_and_build_refuse_a_pinned_root_in_a_loop_at_the_literal() {
    both_verbs_refuse("rule17", PINNED_ROOT_IN_A_LOOP, RULE_17, ":4:21:");
}

#[test]
fn check_and_build_refuse_a_cross_pool_spawn_used_as_a_value_at_the_literal() {
    both_verbs_refuse("xpool", XPOOL_VALUE, FIRE_AND_FORGET, ":2:32:");
}

#[test]
fn check_and_build_refuse_a_cross_pool_spawn_in_another_locus_default_at_the_default() {
    both_verbs_refuse("xpool_default", XPOOL_DEFAULT, FIRE_AND_FORGET_DEFAULT, ":2:35:");
}

/// Lowering's own error for a value use that reaches it (the judgment the
/// law should have made): never what a user sees for a pinned shape.
const LOWERING_BACKSTOP: &str = "reached lowering as a value";

/// `hale check` admits `src`; `hale build` builds it with its IR dumped
/// beside the binary, which runs and prints `prints`. The IR, for a
/// caller to read.
fn admitted_builds_and_runs(tag: &str, src: &str, prints: &str) -> String {
    let d = seed(tag, src);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(ok, "check must pass:\n{out}");
    let bin = d.join("out");
    let build = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["build", &d.to_string_lossy(), "-o", &bin.to_string_lossy()])
        .env("LOTUS_DUMP_IR", "1")
        .current_dir(Path::new("/"))
        .output()
        .expect("hale build");
    assert!(build.status.success(), "build must pass:\n{}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "clean exit: {stdout:?} {:?} {}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(stdout.lines().any(|l| l.trim() == prints), "prints {prints}: {stdout:?}");
    let ir = std::fs::read_to_string(d.join("out.ll")).expect("the dumped IR");
    let _ = std::fs::remove_dir_all(&d);
    ir
}

/// The calls of `@callee` in `ir`.
fn calls_of<'a>(ir: &'a str, callee: &str) -> Vec<&'a str> {
    ir.lines().filter(|l| l.contains(" call ") && l.contains(&format!("@{callee}("))).collect()
}

#[test]
fn check_and_build_refuse_a_cross_pool_spawn_in_a_fn_argument_default_at_the_call() {
    both_verbs_refuse("xpool_arg", XPOOL_ARG_DEFAULT, &fire_and_forget_arg("take", "s"), ":3:42:");
    let d = seed("xpool_arg_backstop", XPOOL_ARG_DEFAULT);
    let bin = d.join("out");
    let build = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["build", &d.to_string_lossy(), "-o", &bin.to_string_lossy()])
        .env("LOTUS_DUMP_IR", "1")
        .current_dir(Path::new("/"))
        .output()
        .expect("hale build");
    let out = String::from_utf8_lossy(&build.stderr);
    assert!(!build.status.success(), "build must fail:\n{out}");
    assert!(!out.contains(LOWERING_BACKSTOP), "the check refuses it, not lowering:\n{out}");
    assert!(!d.join("out.ll").exists(), "no IR: nothing was lowered (it passed `ptr null` for `s`)");
    let _ = std::fs::remove_dir_all(&d);
}

/// The control: supplying the argument expands no default. It builds,
/// runs, prints the hull, and the call passes a pointer, not `null`.
#[test]
fn check_and_build_admit_the_call_that_supplies_the_argument() {
    let ir = admitted_builds_and_runs("xpool_arg_supplied", XPOOL_ARG_SUPPLIED, "5");
    let calls = calls_of(&ir, "take");
    assert_eq!(calls.len(), 1, "one call of `take`: {calls:?}");
    assert!(calls[0].contains("ptr %") && !calls[0].contains("null"), "a non-null argument: {}", calls[0]);
}

#[test]
fn check_and_build_refuse_a_cross_pool_spawn_in_a_method_argument_default_at_the_call() {
    both_verbs_refuse("xpool_method", XPOOL_METHOD_DEFAULT, &fire_and_forget_arg("Tool.use_it", "s"), ":3:60:");
}

#[test]
fn check_and_build_refuse_a_cross_pool_spawn_through_a_chain_of_defaults_at_the_callers_call() {
    both_verbs_refuse("xpool_chain", XPOOL_DEFAULT_CHAIN, &fire_and_forget_arg("inner", "s"), ":4:42:");
}

/// `Driver` on `World`'s own thread: nothing crosses, the default is
/// built where the call stands. Builds, runs, and passes a pointer.
#[test]
fn check_and_build_admit_an_argument_default_expanded_on_the_owners_domain() {
    let src = XPOOL_ARG_DEFAULT.replace("placement { driver: cooperative(pool = workers); } ", "");
    assert_ne!(src, XPOOL_ARG_DEFAULT, "the placement entry was removed");
    let ir = admitted_builds_and_runs("xpool_arg_same_domain", &src, "1");
    let calls = calls_of(&ir, "take");
    assert_eq!(calls.len(), 1, "one call of `take`: {calls:?}");
    assert!(!calls[0].contains("null"), "a non-null argument: {}", calls[0]);
}

/// One default, two callers: one diagnostic, at the call that crosses.
#[test]
fn check_and_build_refuse_only_the_caller_that_crosses() {
    both_verbs_refuse("xpool_two_callers", XPOOL_ARG_TWO_CALLERS, &fire_and_forget_arg("take", "s"), ":3:42:");
    let d = seed("xpool_two_callers_count", XPOOL_ARG_TWO_CALLERS);
    let (_, out) = hale(&["check", &d.to_string_lossy()]);
    assert_eq!(out.matches("is fire-and-forget").count(), 1, "one diagnostic:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn check_and_build_refuse_a_root_literal_in_a_params_default_at_the_override() {
    both_verbs_refuse("rule18_default", ROOT_IN_A_DEFAULT, RULE_18, ":8:45:");
}

#[test]
fn check_and_build_refuse_a_root_literal_two_params_defaults_deep_at_the_override() {
    both_verbs_refuse("rule18_two_deep", ROOT_TWO_DEFAULTS_DEEP, RULE_18, ":8:45:");
}

#[test]
fn check_and_build_refuse_a_root_literal_in_a_params_default_built_in_a_loop_at_the_loops_literal() {
    both_verbs_refuse("rule17_default", ROOT_DEFAULT_IN_A_LOOP, RULE_17, ":9:21:");
}

#[test]
fn check_and_build_refuse_a_root_literal_two_params_defaults_deep_built_in_a_loop_at_the_loops_literal() {
    both_verbs_refuse("rule17_two_deep", ROOT_DEFAULT_TWO_DEEP_IN_A_LOOP, RULE_17, ":10:21:");
}

/// The control: `Holder { }` hoisted out of the loop is admitted.
#[test]
fn check_admits_a_root_literal_in_a_params_default_built_outside_a_loop() {
    let src = ROOT_DEFAULT_IN_A_LOOP.replace("    for i in 0..3 { Holder { }; }\n", "    Holder { };\n    for i in 0..3 { }\n");
    assert_ne!(src, ROOT_DEFAULT_IN_A_LOOP, "the loop's literal was hoisted");
    let d = seed("rule17_default_control", &src);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(ok, "check must pass:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn check_and_build_refuse_a_pinned_root_in_a_const_at_the_consts_literal() {
    both_verbs_refuse("rule17_const", ROOT_IN_A_CONST, &format!("{RULE_17_PER_USE} a `const`'s value"), ":7:16:");
}

#[test]
fn check_and_build_refuse_a_pinned_root_built_through_a_const_at_the_consts_literal() {
    both_verbs_refuse(
        "rule17_through_const",
        ROOT_THROUGH_A_CONST,
        &format!("{RULE_17_PER_USE} a `const`'s value"),
        ":8:19:",
    );
}

#[test]
fn check_and_build_refuse_a_pinned_root_in_a_type_field_default_at_the_defaults_literal() {
    both_verbs_refuse(
        "rule17_type_default",
        ROOT_IN_A_TYPE_DEFAULT,
        &format!("{RULE_17_PER_USE} a type's field default"),
        ":8:23:",
    );
}

/// The positions outside a member body that lowering expands a default
/// in: `inner()` leaves `s` to a `Ship` that `Driver` posts. A closure's
/// assertion is evaluated under its locus (refused like a member body's
/// call); a `const`'s value and a type's field default are emitted at
/// every use, under whichever locus uses them, so they are refused at the
/// position whenever some locus posts the child. Lowering refused all
/// three alone, as a missing judgment, before.
fn xpool_position(decls: &str, driver: &str) -> String {
    format!(
        "locus Ship {{ params {{ hull: Int = 0; }} }}\n\
         fn inner(s: Ship = Ship {{ hull: 1 }}) -> Int {{ return s.hull; }}\n\
         {decls}locus Driver {{ {driver} }}\n\
         main locus World {{ params {{ driver: Driver = Driver {{ }}; }} \
         placement {{ driver: cooperative(pool = workers); }} accept(s: Ship) {{ }} run() {{ }} }}\n\
         fn main() {{ World {{ }}; }}\n"
    )
}

const PER_USE: &str = "cross-pool spawn `Ship{ }` is fire-and-forget: it is built through the defaults this leaves, in";

#[test]
fn check_and_build_refuse_a_cross_pool_default_reached_from_a_closure_assertion() {
    let src = xpool_position("", "params { x: Int = 1; } closure c { inner() ~~ self.x within 5; } run() { Ship { hull: 7 }; }");
    both_verbs_refuse("xpool_closure", &src, &fire_and_forget_arg("inner", "s"), ":3:51:");
}

#[test]
fn check_and_build_refuse_a_cross_pool_default_reached_from_a_const_value() {
    let src = xpool_position("const N: Int = inner();\n", "run() { Ship { hull: 7 }; println(N); }");
    both_verbs_refuse("xpool_const", &src, &format!("{PER_USE} a `const`'s value"), ":3:16:");
}

#[test]
fn check_and_build_refuse_a_cross_pool_default_reached_from_a_type_field_default() {
    let src = xpool_position("type Box { k: Int = inner(); }\n", "run() { Ship { hull: 7 }; let b = Box { }; println(b.k); }");
    both_verbs_refuse("xpool_type_default", &src, &format!("{PER_USE} a type's field default"), ":3:21:");
}

#[test]
fn check_and_build_refuse_a_pinned_root_in_a_closure_assertion_at_the_assertions_literal() {
    both_verbs_refuse("rule17_closure", ROOT_IN_A_CLOSURE, &format!("{RULE_17_PER_USE} a closure's assertion"), ":8:53:");
}

/// The controls: in each position, a root with no `pinned` entry is
/// admitted. The refusal is the pinned thread's, not the position's.
#[test]
fn check_admits_a_root_with_no_pinned_entry_in_a_position_emitted_at_every_use() {
    for (tag, src) in [
        ("const", ROOT_IN_A_CONST),
        ("through_const", ROOT_THROUGH_A_CONST),
        ("type_default", ROOT_IN_A_TYPE_DEFAULT),
        ("closure", ROOT_IN_A_CLOSURE),
    ] {
        let unpinned = src.replace("    placement { w: pinned; }\n", "");
        assert_ne!(unpinned, src, "{tag}: the entry was removed");
        let d = seed(&format!("rule17_per_use_control_{tag}"), &unpinned);
        let (ok, out) = hale(&["check", &d.to_string_lossy()]);
        assert!(ok, "{tag}: check must pass:\n{out}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Rule 18, the third review of PR #1338: `App`'s `w` defaults to a
/// factory that says when it runs, then `rest`. The default's call is
/// line 4, column 26.
fn factory_default_then(rest: &str) -> String {
    format!(
        "locus Worker {{ run() {{ print(\"worker\"); }} }}\n\
         fn make_worker() -> Worker {{ print(\"factory\"); Worker {{ }} }}\n\
         main locus App {{\n\
         \x20   params {{ w: Worker = make_worker(); }}\n\
         \x20   placement {{ w: pinned; }}\n\
         \x20   run() {{ print(\"app\"); }}\n\
         }}\n\
         {rest}"
    )
}

const RULE_18_AT_DEFAULT: &str =
    "placement entry `w` names a field no locus literal initialises: `w`'s default is a call";

/// The review's program: the root's only literal is expanded from
/// `Holder`'s default and spells `w` as a literal, so the factory
/// default is dead. Both verbs admit it, and the binary never calls the
/// factory.
#[test]
fn check_and_build_admit_a_factory_default_every_expanded_root_literal_overrides() {
    let src = factory_default_then(
        "locus Holder { params { app: App = App { w: Worker { } }; } }\nfn main() { Holder { }; }\n",
    );
    let d = seed("rule18_dead_default", &src);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(ok, "check must pass:\n{out}");
    let bin = d.join("out");
    let (ok, out) = hale(&["build", &d.to_string_lossy(), "-o", &bin.to_string_lossy()]);
    assert!(ok, "build must pass:\n{out}");
    let run = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "clean exit: {stdout:?}");
    assert!(!stdout.contains("factory"), "the dead default never runs: {stdout:?}");
    assert!(stdout.contains("worker") && stdout.contains("app"), "both halves run: {stdout:?}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The inverse: the expanded literal leaves `w` to the factory default.
#[test]
fn check_and_build_refuse_a_factory_default_an_expanded_root_literal_takes() {
    let src = factory_default_then("locus Holder { params { app: App = App { }; } }\nfn main() { Holder { }; }\n");
    both_verbs_refuse("rule18_live_expanded", &src, RULE_18_AT_DEFAULT, ":4:26:");
}

/// A construction in `main`'s body takes the default beside the
/// expanded literal that overrides it.
#[test]
fn check_and_build_refuse_a_factory_default_a_construction_takes_beside_an_overriding_expanded_literal() {
    let src = factory_default_then(
        "locus Holder { params { app: App = App { w: Worker { } }; } }\nfn main() { Holder { }; App { }; }\n",
    );
    both_verbs_refuse("rule18_live_mixed", &src, RULE_18_AT_DEFAULT, ":4:26:");
}

/// No literal of the root anywhere: the entry's template takes the
/// default.
#[test]
fn check_and_build_refuse_a_factory_default_no_literal_overrides() {
    let src = factory_default_then("fn main() { print(\"main\"); }\n");
    both_verbs_refuse("rule18_live_entry", &src, RULE_18_AT_DEFAULT, ":4:26:");
}

/// The only overriding literal is in the default of a `Holder` nothing
/// builds: no literal lowering emits builds the root, and the default is
/// judged as when none does.
#[test]
fn check_and_build_refuse_a_factory_default_overridden_only_in_an_unbuilt_holder() {
    let src = factory_default_then(
        "locus Holder { params { app: App = App { w: Worker { } }; } }\nfn main() { print(\"main\"); }\n",
    );
    both_verbs_refuse("rule18_live_unbuilt", &src, RULE_18_AT_DEFAULT, ":4:26:");
}
