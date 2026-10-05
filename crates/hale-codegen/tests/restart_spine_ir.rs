//! The restart and resume spines' IR, pinned per shape (F.40 phase 3,
//! L4).
//!
//! A restart is emitted in the order the lifecycle plan places its steps
//! (`LifecyclePlan::recovery_order`): the decision, once the handler has
//! returned; the restart's entry (the params as built put back for a
//! `restart_in_place`, the latch the failure raised lowered), which
//! begins the next incarnation; that incarnation's `birth()` with its
//! birth-epoch closures; its `run()`. Nothing is torn down: the instance
//! is the same one (C42). `__restart_<L>` holds the entry and the birth
//! for every caller; the decision and the run are the caller's, on the
//! spine that decides: the run gate of the instantiation, the posted
//! run's loop, a pinned locus's thread, the resume at settle. Each shape
//! is held here, step by step, in the function that emits it: a root
//! child, a nested child, each replica, a pinned child, a pool-placed
//! child (whose resumed `run()` is still called inline on the settling
//! thread: C43, known open), and `restart_in_place` under a generic
//! supervisor. A locus that declares no `run()` is resumed without one
//! (line 13, C48).

use std::path::PathBuf;

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Build `src` with its IR dumped; the IR.
fn ir(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).unwrap_or_else(|e| panic!("{name}: parse: {e:?}"));
    let bin = harness::unique_bin(&format!("hale_restart_spine_{name}"));
    let ll: PathBuf = bin.with_extension("ll");
    let opts = hale_codegen::BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
    build_executable_with_options(&program, &bin, &[], &opts).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let text = std::fs::read_to_string(&ll).unwrap_or_else(|e| panic!("{name}: read the IR: {e}"));
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    text
}

/// The body of the function `@name`.
fn body<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("@{name}(");
    let start = ir
        .lines()
        .scan(0usize, |at, l| {
            let here = *at;
            *at += l.len() + 1;
            Some((here, l))
        })
        .find(|(_, l)| l.starts_with("define") && l.contains(&head))
        .map(|(at, _)| at)
        .unwrap_or_else(|| panic!("no function {name} in the IR"));
    let rest = &ir[start..];
    &rest[..rest.find("\n}\n").map(|e| e + 2).unwrap_or(rest.len())]
}

/// The restart steps of `locus` a function body shows, in order: each
/// line naming one, read as its step.
fn steps(body: &str, locus: &str) -> Vec<&'static str> {
    body.lines()
        .filter(|l| !l.starts_with("define"))
        .filter_map(|l| {
            // A second value of one name carries LLVM's numeric suffix.
            if l.trim_start().starts_with("%restart.bumped") {
                Some("decide")
            } else if l.contains(&format!("call void @__restart_{locus}(")) {
                Some("restart")
            } else if l.starts_with("restart.reset:") {
                Some("restore")
            } else if l.contains("%restart.drain_requested.ptr = ") {
                Some("unlatch")
            } else if l.contains(&format!("@{locus}.birth(")) {
                Some("birth")
            } else if l.contains(&format!("@{locus}.__birth_closures(")) {
                Some("closures")
            } else if l.contains(&format!("@{locus}.run(")) || l.contains(&format!("@__coop_pool_run_{locus}(")) {
                Some("run")
            } else if l.contains(&format!("@__run_end_{locus}(")) {
                Some("end")
            } else {
                None
            }
        })
        .collect()
}

/// A worker whose birth-epoch closure fails on its first birth; every
/// shape below restarts it.
const WORKER: &str = "locus Worker {
    params { births: Int = 0; }
    closure second { self.births ~~ 2 within 0; epoch birth; }
    birth() { self.births = self.births + 1; }
    run() { println(\"ev worker-run\"); }
}
";

/// The pinned shapes' worker: a pinned locus declares no birth-epoch
/// closure (rule 6), so its first birth fails a `birth_check`.
const PINNED_WORKER: &str = "locus Worker {
    params { births: Int = 0; }
    closure fuse { captures: births; epoch inline; }
    birth_check { self.births >= 2 } -> violate fuse;
    birth() { self.births = self.births + 1; }
    run() { println(\"ev worker-run\"); }
}
";

/// `__restart_Worker`: the entry, then the next incarnation's birth and
/// its closures, the order every caller shares.
const RESTART_BODY: [&str; 4] = ["restore", "unlatch", "birth", "closures"];

/// `__resume_Worker`: the decision; the restart and the run it decides;
/// the run end of a run that had returned; the run of one that had not.
const RESUME_BODY: [&str; 4] = ["decide", "restart", "end", "run"];

/// `func`'s steps of `Worker`'s restart are `want`.
fn check(ir: &str, func: &str, want: &[&str]) {
    let b = body(ir, func);
    assert_eq!(steps(b, "Worker"), want, "{func}:\n{b}");
}

fn supervised(worker: &str, fields: &str, extra: &str) -> String {
    format!(
        "{worker}main locus App {{\n    params {{ {fields} }}\n{extra}    on_failure(c: Worker, err: ClosureViolation) {{ restart(c); }}\n}}\nfn main() {{ App {{ }}; }}\n"
    )
}

#[test]
fn a_root_child_is_decided_restarted_reborn_then_run() {
    let ir = ir("root_child", &supervised(WORKER, "w: Worker = Worker { };", ""));
    check(&ir, "__restart_Worker", &RESTART_BODY);
    check(&ir, "__resume_Worker", &RESUME_BODY);
    // The run gate of the instantiation: the first birth, then the
    // decision and the restart, before the run.
    check(&ir, "main", &["birth", "closures", "decide", "restart", "run"]);
    // The posted run's loop: the run, then the decision, the restart and
    // the run again; or the run end.
    check(&ir, "__coop_pool_run_Worker", &["run", "decide", "restart", "end"]);
}

#[test]
fn a_nested_child_reads_the_same_order() {
    let src = format!(
        "{WORKER}locus Mid {{\n    params {{ w: Worker = Worker {{ }}; }}\n    on_failure(c: Worker, err: ClosureViolation) {{ restart(c); }}\n}}\nmain locus App {{ params {{ m: Mid = Mid {{ }}; }} }}\nfn main() {{ App {{ }}; }}\n"
    );
    let ir = ir("nested_child", &src);
    check(&ir, "__restart_Worker", &RESTART_BODY);
    check(&ir, "__resume_Worker", &RESUME_BODY);
    check(&ir, "main", &["birth", "closures", "decide", "restart", "run"]);
}

/// A pinned child decides on its own thread: at the gate after its
/// birth and its `birth_check` (C38), and after each run. Its restart
/// has no birth-epoch closures to run.
#[test]
fn a_pinned_child_decides_on_its_thread() {
    let ir = ir("pinned_child", &supervised(PINNED_WORKER, "w: Worker = Worker { };", "    placement { w: pinned; }\n"));
    check(&ir, "__restart_Worker", &RESTART_BODY[..3]);
    check(&ir, "__pinned_main_Worker", &["birth", "decide", "restart", "run", "decide", "restart"]);
}

/// `replicas = K`: each replica's thread runs the one thread function,
/// so each reads the pinned child's order.
#[test]
fn each_replica_reads_the_same_order() {
    let ir = ir(
        "replica",
        &supervised(PINNED_WORKER, "w: Worker = Worker { };", "    placement { w: pinned(replicas = 2); }\n"),
    );
    check(&ir, "__restart_Worker", &RESTART_BODY[..3]);
    check(&ir, "__pinned_main_Worker", &["birth", "decide", "restart", "run", "decide", "restart"]);
}

/// A pool-placed child: its run is posted, and the posted run's loop
/// decides after it. Its resume still runs `run()` inline on the
/// settling thread (C43, known open), through the same wrapper.
#[test]
fn a_pool_placed_child_decides_in_its_posted_run() {
    let ir = ir(
        "pool_child",
        &supervised(WORKER, "w: Worker = Worker { };", "    placement { w: cooperative(pool = side); }\n"),
    );
    check(&ir, "__restart_Worker", &RESTART_BODY);
    check(&ir, "__resume_Worker", &RESUME_BODY);
    check(&ir, "__coop_pool_run_Worker", &["run", "decide", "restart", "end"]);
}

/// `restart_in_place` under a generic supervisor: each specialization's
/// handler asks the one `__restart_Worker`, whose entry puts the params
/// as built back before the birth.
#[test]
fn restart_in_place_under_a_generic_supervisor_restores_before_the_birth() {
    let src = format!(
        "{WORKER}locus Sup<T> {{\n    params {{ seed: T; w: Worker = Worker {{ }}; }}\n    on_failure(c: Worker, err: ClosureViolation) {{ restart_in_place(c); }}\n}}\nfn main() {{\n    let a: Sup<Int> = Sup {{ seed: 1 }};\n    let b: Sup<String> = Sup {{ seed: \"x\" }};\n}}\n"
    );
    let ir = ir("generic_in_place", &src);
    check(&ir, "__restart_Worker", &RESTART_BODY);
    let restart = body(&ir, "__restart_Worker");
    assert!(restart.contains("%restore.snap = "), "the entry restores the params as built:\n{restart}");
}

/// Line 13, C48: a locus that declares no `run()` is resumed without
/// one, as its first incarnation runs none; the restart is unchanged.
#[test]
fn a_locus_with_no_run_is_resumed_without_one() {
    let src = "locus Worker {
    params { births: Int = 0; }
    closure second { self.births ~~ 2 within 0; epoch birth; }
    birth() { self.births = self.births + 1; }
}
main locus App {
    params { c: Worker = Worker { }; }
    on_failure(c: Worker, err: ClosureViolation) { restart(c); }
}
fn main() { App { }; }
";
    let ir = ir("no_run", src);
    check(&ir, "__restart_Worker", &RESTART_BODY);
    check(&ir, "__resume_Worker", &["decide", "restart", "end"]);
}
