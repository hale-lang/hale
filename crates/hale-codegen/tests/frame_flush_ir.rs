//! The frame flush's IR, pinned (F.40 phase 3, L4; inventory rows C16,
//! C17, C21–C23).
//!
//! At one of `fn main`'s exits the flush owes the rest of its spine's
//! process rows, read from the plan (`LifecyclePlan::process_order`): the
//! head runs the rows before the frame's pre-drain, the flush the
//! pre-drain and the rows after it. Without a pool that is the ingress
//! quiesce, the pre-drain, then the wait-abort (line 7: a handler the
//! pre-drain runs may still wait); with one, the quiesce, the wait-abort,
//! the pool join, then the pre-drain, and no second abort. Any other fn's
//! flush drains only: it must not abort the waits program-wide.
//!
//! The entries follow, subscription-less pinned ones first in reverse,
//! then the rest in reverse push order, a locus's own pinned fields pushed
//! after its own entry (C14), so they are joined before its cascade. A
//! main-locus entry's head (the quiesce, the wait-abort, the pool join)
//! comes before the first of its own pinned joins, wherever the order puts
//! it, as the plan places it (`LifecyclePlan::entry_order`; line 7: a
//! pinned thread parked in a wait is joined only once the wait is
//! aborted), and the entry's own teardown does not run it again.
//!
//! The order is read from the control flow: a step comes before another
//! when the other's block is reachable from its block and not the reverse
//! (or, in one block, by line).

use std::collections::BTreeSet;
use std::path::PathBuf;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Build `src` with its IR dumped; the IR.
fn ir(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_frame_flush_{name}"));
    let ll: PathBuf = bin.with_extension("ll");
    let opts = hale_codegen::BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let text = std::fs::read_to_string(&ll).unwrap_or_else(|e| panic!("{name}: read the IR: {e}"));
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    text
}

/// One function's blocks: label, lines, successors.
struct Func {
    blocks: Vec<(String, Vec<String>, Vec<String>)>,
}

/// The function `name` of the module.
fn function(ir: &str, name: &str) -> Func {
    let head = format!("@{name}(");
    let mut lines = ir.lines().skip_while(|l| !(l.starts_with("define") && l.contains(&head)));
    assert!(lines.next().is_some(), "no function `{name}` in the IR");
    let mut blocks: Vec<(String, Vec<String>, Vec<String>)> = vec![("<entry>".into(), Vec::new(), Vec::new())];
    for l in lines {
        if l == "}" {
            break;
        }
        let label = l.split(';').next().unwrap_or("").trim_end();
        if !l.starts_with(' ') && label.ends_with(':') {
            let label = label.trim_end_matches(':').trim_matches('"').to_string();
            if blocks.len() == 1 && blocks[0].1.is_empty() {
                blocks[0].0 = label;
            } else {
                blocks.push((label, Vec::new(), Vec::new()));
            }
            continue;
        }
        let b = blocks.last_mut().expect("a block");
        let t = l.trim_start();
        if t.starts_with("br ") || t.starts_with("switch ") || t.starts_with('[') || t.starts_with("i64 ") {
            for part in t.split("label %").skip(1) {
                let target: String =
                    part.trim_start_matches('"').chars().take_while(|c| !matches!(c, ',' | ' ' | ']' | '"')).collect();
                b.2.push(target);
            }
        }
        b.1.push(l.to_string());
    }
    Func { blocks }
}

impl Func {
    fn reach(&self, from: usize) -> BTreeSet<usize> {
        let at = |t: &String| self.blocks.iter().position(|b| &b.0 == t);
        let mut seen = BTreeSet::new();
        let mut stack: Vec<usize> = self.blocks[from].2.iter().filter_map(at).collect();
        while let Some(b) = stack.pop() {
            if seen.insert(b) {
                stack.extend(self.blocks[b].2.iter().filter_map(at));
            }
        }
        seen
    }

    /// The (block, line) of every call to `callee`.
    fn calls(&self, callee: &str) -> Vec<(usize, usize)> {
        let pat = format!("@{callee}(");
        let mut out = Vec::new();
        for (b, (_, lines, _)) in self.blocks.iter().enumerate() {
            for (i, l) in lines.iter().enumerate() {
                if l.contains("call ") && l.contains(&pat) {
                    out.push((b, i));
                }
            }
        }
        out
    }

    /// The one block labelled `label`.
    fn block(&self, label: &str) -> (usize, usize) {
        let found: Vec<usize> = (0..self.blocks.len()).filter(|&b| self.blocks[b].0 == label).collect();
        assert_eq!(found.len(), 1, "one block `{label}`");
        (found[0], 0)
    }

    fn before(&self, a: (usize, usize), b: (usize, usize)) -> bool {
        if a.0 == b.0 {
            return a.1 < b.1;
        }
        self.reach(a.0).contains(&b.0) && !self.reach(b.0).contains(&a.0)
    }

    /// The calls of one block, in order, among `callees` (short names).
    fn sequence(&self, b: usize, callees: &[&str]) -> Vec<String> {
        self.blocks[b]
            .1
            .iter()
            .filter_map(|l| callees.iter().find(|c| l.contains("call ") && l.contains(&format!("@{c}("))))
            .map(|c| c.to_string())
            .collect()
    }
}

const PROCESS: &[&str] = &[
    "lotus_bus_ingress_quiesce",
    "lotus_bus_wait_abort_all",
    "lotus_coop_pool_shutdown_all",
    "lotus_bus_queue_drain",
];

/// A subscribing main locus (deferred to fn main's frame) owning a
/// subscription-less pinned field and a subscribing pinned one, a
/// let-bound literal after it, and, with `pool`, a field on a pool.
fn program(pool: bool, main: &str) -> String {
    let (field, placement) = if pool {
        ("        pooled: Pooled = Pooled { };\n", " pooled: cooperative(pool = io);")
    } else {
        ("", "")
    };
    format!(
        r#"type Fin {{ n: Int; }}
topic Finals {{ payload: Fin; subject: "frame.flush.ir.finals"; }}
type Tick {{ n: Int; }}
topic Ticks {{ payload: Tick; subject: "frame.flush.ir.ticks"; }}

locus Sink {{
    bus {{ subscribe Finals as on_final; }}
    fn on_final(f: Fin) {{ }}
}}

locus Spinner {{
    run() {{ }}
}}

locus Pooled {{
    run() {{ }}
}}

locus Kid {{
    params {{ n: Int = 1; }}
}}

main locus App {{
    params {{
        spinner: Spinner = Spinner {{ }};
        sink: Sink = Sink {{ }};
{field}    }}
    placement {{ spinner: pinned; sink: pinned;{placement} }}
    bus {{ subscribe Ticks as on_tick; }}
    fn on_tick(t: Tick) {{ }}
    run() {{ }}
}}

{main}"#
    )
}

/// The blocks of `main` holding a whole exit's process calls: those whose
/// sequence of process calls is exactly `want`.
fn exits(f: &Func, want: &[&str]) -> usize {
    (0..f.blocks.len()).filter(|&b| f.sequence(b, PROCESS) == want).count()
}

/// The entries' order: subscription-less pinned first, then reverse push
/// (the let-bound literal, then the owner's subscribing pinned field,
/// then the owner), each entry's first block its arena check.
fn assert_entry_order(f: &Func, what: &str) {
    let order = ["Spinner", "Kid", "Sink", "App"].map(|n| f.block(&format!("{n}.dissolve.arena_check")));
    for w in order.windows(2) {
        assert!(f.before(w[0], w[1]), "{what}: the flush's entries out of order: {:?}", order);
    }
}

#[test]
fn fn_main_fall_through_without_a_pool_drains_then_aborts() {
    let ir = ir("fall_through", &program(false, "fn main() {\n    App { };\n    let kid = Kid { };\n}\n"));
    let f = function(&ir, "main");
    // The exit's head, then its flush: the quiesce, the pre-drain, the
    // wait-abort, in one block.
    let want = ["lotus_bus_ingress_quiesce", "lotus_bus_queue_drain", "lotus_bus_wait_abort_all"];
    assert_eq!(exits(&f, &want), 1, "fall-through: one exit block calls {want:?}");
    let abort = f.calls("lotus_bus_wait_abort_all");
    let first = f.block("Spinner.dissolve.arena_check");
    assert!(abort.iter().any(|&a| f.before(a, first)), "the wait-abort comes before the first entry");
    assert_entry_order(&f, "fall-through");
}

#[test]
fn fn_main_fall_through_with_a_pool_joins_then_drains() {
    let ir = ir("fall_through_pool", &program(true, "fn main() {\n    App { };\n    let kid = Kid { };\n}\n"));
    let f = function(&ir, "main");
    // With a pool, the head aborts the waits ahead of its join; the flush
    // owes the pre-drain alone.
    let want = [
        "lotus_bus_ingress_quiesce",
        "lotus_bus_wait_abort_all",
        "lotus_coop_pool_shutdown_all",
        "lotus_bus_queue_drain",
    ];
    assert_eq!(exits(&f, &want), 1, "fall-through: one exit block calls {want:?}");
    assert_entry_order(&f, "fall-through with a pool");
}

#[test]
fn fn_main_return_flushes_as_the_fall_through_does() {
    let ir = ir("return", &program(false, "fn main() {\n    App { };\n    let kid = Kid { };\n    return 0;\n}\n"));
    let f = function(&ir, "main");
    let want = ["lotus_bus_ingress_quiesce", "lotus_bus_queue_drain", "lotus_bus_wait_abort_all"];
    assert_eq!(exits(&f, &want), 1, "return: one exit block calls {want:?}");
    assert_entry_order(&f, "return");
}

#[test]
fn fn_main_test_failure_flushes_as_the_fall_through_does() {
    let main = "fn main() {\n    App { };\n    let kid = Kid { };\n    std::test::assert(false, \"the failure exit\");\n}\n";
    let bare = ir("test_failure", &program(false, main));
    let f = function(&bare, "main");
    // The fall-through and the failure exit each run the whole sequence.
    let want = ["lotus_bus_ingress_quiesce", "lotus_bus_queue_drain", "lotus_bus_wait_abort_all"];
    assert_eq!(exits(&f, &want), 2, "fall-through and failure exits: each a block calling {want:?}");
    let want = [
        "lotus_bus_ingress_quiesce",
        "lotus_bus_wait_abort_all",
        "lotus_coop_pool_shutdown_all",
        "lotus_bus_queue_drain",
    ];
    let pooled = ir("test_failure_pool", &program(true, main));
    assert_eq!(exits(&function(&pooled, "main"), &want), 2, "with a pool: each exit a block calling {want:?}");
}

/// The deferred main entry's head, hoisted ahead of its own pinned joins
/// (L4's fifth part): in `want`'s order in one block, `App.head`, before
/// the first pinned entry the flush joins and before the other, and not
/// emitted again by App's own teardown. Before, App's entry ran it after
/// both joins, so a pinned field parked in an `or wait` hung the join
/// (`l07_or_wait_deferred_pinned_field.hl`).
fn assert_head_before_pinned_joins(f: &Func, want: &[&str], what: &str) {
    let head: Vec<usize> = (0..f.blocks.len()).filter(|&b| f.blocks[b].0 == "App.head").collect();
    assert_eq!(head.len(), 1, "{what}: one hoisted head block");
    assert_eq!(f.sequence(head[0], PROCESS), want, "{what}: the head's rows in the plan's order");
    for pinned in ["Spinner", "Sink"] {
        let entry = f.block(&format!("{pinned}.dissolve.arena_check"));
        assert!(f.before((head[0], 0), entry), "{what}: the head comes before {pinned}'s join");
    }
    let own = f.block("App.dissolve.process");
    let rows = ["lotus_bus_ingress_quiesce", "lotus_bus_wait_abort_all", "lotus_coop_pool_shutdown_all"];
    assert!(f.sequence(own.0, &rows).is_empty(), "{what}: App's own teardown runs no second head");
}

#[test]
fn a_deferred_main_entrys_head_comes_before_its_pinned_joins() {
    let main = "fn start() {\n    let app = App { };\n}\n\nfn main() {\n    start();\n}\n";
    let bare = ir("hoisted_head", &program(false, main));
    let f = function(&bare, "start");
    assert_head_before_pinned_joins(&f, &["lotus_bus_ingress_quiesce", "lotus_bus_wait_abort_all"], "start");
    let pooled = ir("hoisted_head_pool", &program(true, main));
    let f = function(&pooled, "start");
    let want = ["lotus_bus_ingress_quiesce", "lotus_bus_wait_abort_all", "lotus_coop_pool_shutdown_all"];
    assert_head_before_pinned_joins(&f, &want, "start with a pool");
    // Held by fn main's frame, after the exit's own head and pre-drain.
    let in_main = ir("hoisted_head_main", &program(true, "fn main() {\n    App { };\n    let kid = Kid { };\n}\n"));
    assert_head_before_pinned_joins(&function(&in_main, "main"), &want, "fn main");
}

/// A root with a pinned subscriber field of `replicas` replicas, built by
/// `make`: `{returns}` says whether `make` hands it back, or tears it
/// down at its own exit.
fn anchor_program(returns: bool, replicas: u32) -> String {
    let (sig, body, main) = if returns {
        (" -> App", "    return App { };\n", "fn main() {\n    let app = make();\n}\n")
    } else {
        ("", "    let app = App { };\n", "fn main() {\n    make();\n}\n")
    };
    let placement = if replicas == 1 { "pinned".to_string() } else { format!("pinned(replicas = {replicas})") };
    format!(
        "type Ping {{ n: Int; }}\ntopic Pings {{ payload: Ping; subject: \"frame.flush.ir.anchor\"; }}\n\n\
         locus Sink {{\n    bus {{ subscribe Pings as on_ping; }}\n    fn on_ping(p: Ping) {{ }}\n}}\n\n\
         main locus App {{\n    params {{ sink: Sink = Sink {{ }}; }}\n    placement {{ sink: {placement}; }}\n}}\n\n\
         fn make(){sig} {{\n{body}}}\n\n{main}"
    )
}

/// The replica index each of `f`'s `sink.anchor_record.*` addresses
/// names (`getelementptr inbounds [K x { i64, ptr }], ptr …, i32 0, i32
/// <replica>`), with its (block, line), in block order.
fn record_reads(f: &Func) -> Vec<(u32, (usize, usize))> {
    let mut out = Vec::new();
    for (b, (_, lines, _)) in f.blocks.iter().enumerate() {
        for (i, l) in lines.iter().enumerate() {
            if l.trim_start().starts_with("%sink.anchor_record.") {
                let replica = l.rsplit("i32 ").next().and_then(|t| t.split(|c: char| !c.is_ascii_digit()).next()).unwrap();
                out.push((replica.parse().unwrap(), (b, i)));
            }
        }
    }
    out
}

/// The stores of a function into `{name}` slots (`store …, ptr %{name}`).
fn stores_into(f: &Func, name: &str) -> usize {
    let pat = format!("ptr %{name}");
    f.blocks.iter().flat_map(|b| &b.1).filter(|l| l.trim_start().starts_with("store ") && l.contains(&pat)).count()
}

/// C52 (line 12): a root handed back to its caller keeps, per instance,
/// the join record of every replica of its pinned field (`[K x {i64
/// thread, ptr instance}]`), and its cascade, where the caller tears it
/// down, shuts down, joins and reclaims every one; the frame that built
/// it pushes no entry for any. The order is the building frame's flush
/// order: replica 0 (pushed last, by the field's own path), then K-1 down
/// to 1. Before the review of #1354 the record was a `__thread` in the
/// field's instance, so only replica 0 was reachable from the root and
/// the others were never joined (`l12_returned_root_pinned_replicas.hl`).
/// The control, a root `make` keeps, is the frame flush's as before: no
/// record, no cascade join, `make`'s flush joins every replica.
#[test]
fn a_returned_roots_pinned_field_is_joined_by_its_owners_teardown() {
    for k in [1u32, 3] {
        let returned = ir(&format!("anchor_returned_{k}"), &anchor_program(true, k));
        assert!(returned.contains(&format!("[{k} x {{ i64, ptr }}]")), "K = {k}: App keeps one record per replica");
        assert!(!returned.contains("__thread"), "K = {k}: no record in the field's own instance");
        let make = function(&returned, "make");
        assert!(make.calls("lotus_pinned_join").is_empty(), "K = {k}: make joins no thread");
        assert!(!make.blocks.iter().any(|b| b.0.starts_with("Sink.dissolve")), "K = {k}: make's flush has no entry for any Sink");
        assert_eq!(stores_into(&make, "Sink.record.thread"), k as usize, "K = {k}: every replica writes its thread");
        assert_eq!(stores_into(&make, "Sink.record.self"), k as usize, "K = {k}: every replica writes its instance");

        let main = function(&returned, "main");
        let joins = main.calls("lotus_pinned_join");
        assert_eq!(joins.len(), k as usize, "K = {k}: fn main's App teardown joins every replica");
        let app = main.block("App.dissolve.process");
        assert!(joins.iter().all(|&j| main.before(app, j)), "K = {k}: inside App's teardown");
        let reads = record_reads(&main);
        let order: Vec<u32> = reads.iter().map(|r| r.0).collect();
        let want: Vec<u32> = std::iter::once(0).chain((1..k).rev()).collect();
        assert_eq!(order, want, "K = {k}: the records are joined in the frame flush's order");
        for (w, j) in reads.windows(2).zip(joins.windows(2)) {
            assert!(main.before(w[0].1, w[1].1) && main.before(j[0], j[1]), "K = {k}: one replica's join before the next's");
        }
        for (r, j) in reads.iter().zip(&joins) {
            assert!(main.before(r.1, *j), "K = {k}: replica {}'s record is read before its join", r.0);
        }
    }

    for k in [1u32, 2] {
        let kept = ir(&format!("anchor_kept_{k}"), &anchor_program(false, k));
        assert!(!kept.contains("anchor_record") && !kept.contains("instance_join"), "K = {k}: a root the frame keeps carries no record");
        let make = function(&kept, "make");
        assert_eq!(make.calls("lotus_pinned_join").len(), k as usize, "K = {k}: make's flush joins every replica");
    }
}

#[test]
fn another_fns_flush_drains_and_aborts_no_wait() {
    let main = "fn helper() {\n    let kid = Kid { };\n}\n\nfn main() {\n    helper();\n    App { };\n}\n";
    let ir = ir("helper", &program(false, main));
    let f = function(&ir, "helper");
    let drain = f.calls("lotus_bus_queue_drain");
    let kid = f.block("Kid.dissolve.arena_check");
    assert!(drain.iter().any(|&d| f.before(d, kid)), "helper's flush pre-drains before its entry");
    assert!(f.calls("lotus_bus_wait_abort_all").is_empty(), "helper's flush aborts no wait");
}
