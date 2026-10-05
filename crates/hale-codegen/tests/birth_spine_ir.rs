//! The birth spine's IR, pinned per shape (F.40 phase 3, L4).
//!
//! The instantiation emits its birth spine in the order the lifecycle
//! plan places it (`hale_types::lifecycle::spine`): registration, the
//! birth with its checks, readiness, the run's start. Each shape's IR is
//! held to that order here, step by step, in the function that emits it:
//! a root child and a nested child (inline in the instantiation), a
//! replica (each replica's instantiation), a pinned child (the
//! registration on the instantiating thread, the rest in its thread
//! function, with the `birth_check` before `run()`: C38), and a
//! cross-pool bubble's child (its create cell's dispatcher on the
//! owner's thread: the stitch to the owner, then the birth; C2). The release
//! IR the plan changes is exactly these steps: the readiness window
//! around a subscriber's birth (`lotus_bus_hold_delivery` before its
//! first registration, `lotus_bus_ready` after its birth and checks),
//! a baked direct publish's guard, and a pinned locus's checks and gate.
//! A pinned thread keeps its mailbox current through params and birth;
//! the per-instance window holds only its own unready deliveries.

use std::path::PathBuf;

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Build `src` with its IR dumped; the IR.
fn ir(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).unwrap_or_else(|e| panic!("{name}: parse: {e:?}"));
    let bin = harness::unique_bin(&format!("hale_birth_spine_{name}"));
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

/// The steps of `locus`'s birth spine a function body shows, in order:
/// each line naming one, read as its step.
fn steps(body: &str, locus: &str) -> Vec<&'static str> {
    let me = format!("%{locus}.self");
    body.lines()
        .filter_map(|l| {
            if l.contains("@lotus_bus_hold_delivery(") && l.contains(&me) {
                Some("hold")
            } else if l.contains("@lotus_bus_register") && l.contains(&me) {
                Some("register")
            } else if l.contains(&format!("@{locus}.birth(")) {
                Some("birth")
            } else if l.contains("@lotus_pool_start_post(") && l.contains(&format!("@__pool_birth_{locus}")) {
                Some("birth-job")
            } else if l.contains("%bcheck.has.handler = ") {
                Some("check")
            } else if l.contains("@lotus_bus_ready(") {
                Some("ready")
            } else if l.starts_with("pinned.gate.restart:") || l.starts_with("gate.restart:") {
                Some("gate")
            } else if l.contains(&format!("@{locus}.run(")) || l.contains(&format!("@__coop_pool_run_{locus}(")) {
                Some("run")
            } else {
                None
            }
        })
        .collect()
}

const SUB: &str = "type Ping { n: Int; }
topic Pings { payload: Ping; subject: \"birth.spine.ping\"; }
locus Sub {
    params { n: Int = 0; }
    bus { subscribe Pings as on_ping; publish Pings; }
    fn on_ping(p: Ping) { self.n = self.n + p.n; }
    birth() { println(\"ev sub-birth\"); }
    run() { println(\"ev sub-run\"); }
}
";

#[test]
fn a_root_child_registers_then_is_born_then_ready_then_runs() {
    let src = format!("{SUB}main locus App {{ params {{ s: Sub = Sub {{ }}; }} }}\nfn main() {{ App {{ }}; }}\n");
    let ir = ir("root_child", &src);
    assert_eq!(steps(body(&ir, "main"), "Sub"), ["hold", "register", "birth", "ready", "run"]);
}

#[test]
fn a_nested_child_reads_the_same_order() {
    let src = format!(
        "{SUB}locus Mid {{ params {{ s: Sub = Sub {{ }}; }} }}\nmain locus App {{ params {{ m: Mid = Mid {{ }}; }} }}\nfn main() {{ App {{ }}; }}\n"
    );
    let ir = ir("nested_child", &src);
    assert_eq!(steps(body(&ir, "main"), "Sub"), ["hold", "register", "birth", "ready", "run"]);
}

/// `replicas = K` is a pinned placement's: each replica registers with
/// its mailbox on the instantiating thread, and the thread function they
/// share births it, then readies it (the birth window closes on its thread), then runs it.
#[test]
fn each_replica_reads_the_same_order() {
    let src = format!(
        "{SUB}main locus App {{\n    params {{ s: Sub = Sub {{ }}; }}\n    placement {{ s: pinned(replicas = 2); }}\n}}\nfn main() {{ App {{ }}; }}\n"
    );
    let ir = ir("replica", &src);
    assert_eq!(steps(body(&ir, "main"), "Sub"), ["hold", "register", "hold", "register"]);
    assert_eq!(steps(body(&ir, "__pinned_main_Sub"), "Sub"), ["birth", "ready", "run"]);
}

/// C38: the pinned thread's birth spine, after the registration on the
/// instantiating thread: birth, the `birth_check`, readiness, the gate a
/// held check's decision is waited at, then the run.
#[test]
fn a_pinned_child_checks_its_birth_on_its_thread_before_its_run() {
    let src = format!(
        "{}main locus App {{\n    params {{ s: Sub = Sub {{ }}; }}\n    placement {{ s: pinned; }}\n    on_failure(c: Sub, err: ClosureViolation) {{ }}\n}}\nfn main() {{ App {{ }}; }}\n",
        SUB.replace(
            "    birth() {",
            "    closure fuse { captures: n; epoch inline; }\n    birth_check { self.n == 0 } -> violate fuse;\n    birth() {"
        )
    );
    let ir = ir("pinned_child", &src);
    assert_eq!(steps(body(&ir, "main"), "Sub"), ["hold", "register"], "the instantiating thread holds and registers");
    assert_eq!(
        steps(body(&ir, "__pinned_main_Sub"), "Sub"),
        ["birth", "check", "ready", "gate", "run"],
        "the thread births, checks, readies, gates, runs"
    );
}

/// Line 3 (L4's fifth part): a root child placed on a worker pool is born
/// on the pool's worker. The instantiating thread registers it, posts its
/// birth as a job (`__pool_birth_Sub`: the birth, then its `birth_check`)
/// and waits for it, then readies it, gates on a held check's decision
/// (which stays on the thread holding the owner's params open, so the job
/// never waits for it) and posts its run. Before, the birth and the check
/// were inline on the instantiating thread.
#[test]
fn a_pool_placed_child_is_born_on_its_pools_worker() {
    let src = format!(
        "{}main locus App {{\n    params {{ s: Sub = Sub {{ }}; }}\n    placement {{ s: cooperative(pool = side); }}\n    on_failure(c: Sub, err: ClosureViolation) {{ }}\n}}\nfn main() {{ App {{ }}; }}\n",
        SUB.replace(
            "    birth() {",
            "    closure fuse { captures: n; epoch inline; }\n    birth_check { self.n == 0 } -> violate fuse;\n    birth() {"
        )
    );
    let ir = ir("pool_child", &src);
    assert_eq!(steps(body(&ir, "main"), "Sub"), ["hold", "register", "birth-job", "ready", "gate", "run"]);
    assert_eq!(steps(body(&ir, "__pool_birth_Sub"), "Sub"), ["birth", "check"], "the worker births and checks");
}

/// The fifth shape, a cross-pool bubble: the consumer posts a create
/// cell, and the dispatcher on the owner's thread runs the child's birth
/// spine there, in the plan's order (line 5): the stitch to its owner
/// (`accept`, the children's push), then `birth()`. It used to birth the
/// child first and accept it after. A cross-pool child subscribes to
/// nothing and runs nothing, so that is the whole spine.
#[test]
fn a_crosspool_bubble_accepts_then_births_on_the_owners_thread() {
    let src = "locus Ship {
    params { hull: Int = 0; }
    contract { expose hull: Int; }
    birth() { println(\"ev ship-birth\"); }
}
locus Driver { run() { Ship { hull: 7 }; } }
main locus World {
    params { driver: Driver = Driver { }; }
    placement { driver: cooperative(pool = workers); }
    contract { consume hull: Int; }
    accept(s: Ship) { println(\"ev accept\"); }
    run() { std::time::sleep(50ms); }
}
fn main() { World { }; }
";
    let ir = ir("crosspool_bubble", src);
    let dispatch = body(&ir, "__xpool_dispatch_Ship_World");
    let at = |pat: &str| {
        dispatch.lines().position(|l| l.contains(pat)).unwrap_or_else(|| panic!("no `{pat}` in the dispatcher:\n{dispatch}"))
    };
    let accept = at("call void @World.accept(");
    let push = at("@lotus_children_push(");
    let birth = at("call void @Ship.birth(");
    assert!(accept < push && push < birth, "accept, the push, then birth:\n{dispatch}");
    assert_eq!(dispatch.matches("\n  br ").count(), 0, "one block: the order is the text's");
}
