//! The teardown spines' obligations per target (F.40 phase 3, P3 3 of
//! 3; `notes/f40-capability-matrix.md` § 3.3): the matrix selects them,
//! the lifecycle plan orders them.
//!
//! Each spine is a program shape whose first teardown is that spine's:
//! the eager main-locus dissolve (a statement literal), the deferred
//! main-locus entry (the literal let-bound in a fn that is not `main`),
//! and `fn main`'s fall-through, `return` and test-failure exits (the
//! literal let-bound in `main`). Each is built in four variants that
//! change one behaviour at a time: a pool field only, an `or wait`
//! publish on a transport-bound topic only, an `or wait` publish on a
//! local `on_full: fail` topic with no binding only, and none of them.
//!
//! - **Host**: every variant builds. The ones that run (all but the
//!   bound one, whose binding has no peer) run under the lifecycle
//!   trace, and the spine under test owes exactly what the host's cells
//!   select for the program: the wait-abort always, the pool join where
//!   the program has a pool. The order checked is the plan's
//!   (`LifecyclePlan::process_order` over the five spines' process
//!   rows), read from the program's plan, never spelled here, and both
//!   come before the main locus's cascade. No reclaim
//!   spine and no dissolve cascade owes a process-wide obligation.
//! - **wasm32**: the pool and bound variants are refused at their
//!   placement entry and binding. The local and bare ones build, call
//!   neither the pool join nor the ingress quiesce anywhere, call the
//!   wait-abort in every spine (no proof omits it yet), import no
//!   `pthread_create`, `epoll_*` or `eventfd` (the thread and async_io
//!   startup the cells reject), and run to their end under node.
//! - **IR**, both targets: within every basic block the three calls
//!   respect the plan's edges (a spine's head is one block, so this is
//!   each spine's own order, the ingress quiesce included, which the
//!   trace does not carry), and each module's call counts are pinned
//!   per shape, variant and target in [`PINNED`].

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use std::collections::BTreeSet;

use hale_codegen::{build_executable_with_options, BuildOptions, CodegenError, CompileTarget};
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_types::capability::{LoweringCells, Obligation, TargetClass};
use hale_types::lifecycle::trace::{self, Trace};
use hale_types::lifecycle::{ObligationKind, Point, Spine};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/wasm_module.rs"]
mod wasm_module;

const DEADLINE: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    Eager,
    Deferred,
    FallThrough,
    Return,
    TestFailure,
}

const SHAPES: &[Shape] = &[Shape::Eager, Shape::Deferred, Shape::FallThrough, Shape::Return, Shape::TestFailure];

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::Eager => "eager",
            Shape::Deferred => "deferred",
            Shape::FallThrough => "fall_through",
            Shape::Return => "return",
            Shape::TestFailure => "test_failure",
        }
    }

    /// The spine whose teardown runs first.
    fn spine(self) -> Spine {
        match self {
            Shape::Eager => Spine::EagerTeardown,
            Shape::Deferred => Spine::DeferredMainEntry,
            Shape::FallThrough => Spine::MainFallThrough,
            Shape::Return => Spine::MainReturn,
            Shape::TestFailure => Spine::MainTestFailure,
        }
    }

    fn main(self) -> &'static str {
        match self {
            Shape::Eager => "fn main() {\n    App { };\n    println(\"end\");\n}\n",
            Shape::Deferred => "fn start() {\n    let app = App { };\n}\n\nfn main() {\n    start();\n    println(\"end\");\n}\n",
            Shape::FallThrough => "fn main() {\n    let app = App { };\n    println(\"end\");\n}\n",
            Shape::Return => "fn main() {\n    let app = App { };\n    println(\"end\");\n    return;\n}\n",
            Shape::TestFailure => {
                "fn main() {\n    let app = App { };\n    println(\"end\");\n    std::test::assert(false, \"the failure exit\");\n}\n"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Variant {
    Pool,
    Bound,
    Local,
    Bare,
}

const VARIANTS: &[Variant] = &[Variant::Pool, Variant::Bound, Variant::Local, Variant::Bare];

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Variant::Pool => "pool",
            Variant::Bound => "bound",
            Variant::Local => "local",
            Variant::Bare => "bare",
        }
    }

    fn app(self) -> &'static str {
        match self {
            Variant::Pool => {
                "locus Worker {\n    run() { println(\"worker\"); }\n}\n\n\
                 main locus App {\n    params { w: Worker = Worker { }; }\n    \
                 placement { w: cooperative(pool = side); }\n    run() { println(\"app\"); }\n}\n"
            }
            Variant::Bound => {
                "type E { n: Int = 0; }\ntopic Evt { payload: E; subject: \"cells.bound\"; }\n\n\
                 main locus App {\n    bus { publish Evt; }\n    \
                 bindings { Evt: unix(\"/tmp/hale-cells-no-peer.sock\", role: connect); }\n    \
                 run() { Evt <- E { n: 1 } or wait; }\n}\n"
            }
            Variant::Local => {
                "type E { n: Int = 0; }\ntopic Evt {\n    payload: E;\n    subject: \"cells.local\";\n    \
                 bounded(2);\n    on_full: fail;\n}\n\n\
                 main locus App {\n    bus { publish Evt; }\n    run() { Evt <- E { n: 1 } or wait; println(\"app\"); }\n}\n"
            }
            Variant::Bare => "main locus App {\n    run() { println(\"app\"); }\n}\n",
        }
    }

    fn pools(self) -> bool {
        self == Variant::Pool
    }
}

fn source(shape: Shape, variant: Variant) -> String {
    format!("{}\n{}", variant.app(), shape.main())
}

/// The module's teardown call counts: ingress quiesce, wait-abort, pool
/// join, per (shape, variant, target); `None` where the target refuses
/// the program.
type Counts = Option<(usize, usize, usize)>;

/// The call counts, pinned. The selection is the cells' (wasm32 owes no
/// quiesce and no join); the count is how many spines a shape emits:
/// eager, its head and the fall-through's; deferred, the entry's and
/// the fall-through's; fall-through and `return`, the exit's and the
/// deferred entry's in its frame (the fall-through after `return;` is
/// unreachable and dropped); test failure, both the fall-through and
/// the failure exit, each with the deferred entry's.
const PINNED: &[(&str, &str, &str, Counts)] = &[
    ("eager", "pool", "host", Some((2, 2, 2))),
    ("eager", "bound", "host", Some((2, 2, 0))),
    ("eager", "local", "host", Some((2, 2, 0))),
    ("eager", "bare", "host", Some((2, 2, 0))),
    ("eager", "pool", "wasm32", None),
    ("eager", "bound", "wasm32", None),
    ("eager", "local", "wasm32", Some((0, 2, 0))),
    ("eager", "bare", "wasm32", Some((0, 2, 0))),
    ("deferred", "pool", "host", Some((2, 2, 2))),
    ("deferred", "bound", "host", Some((2, 2, 0))),
    ("deferred", "local", "host", Some((2, 2, 0))),
    ("deferred", "bare", "host", Some((2, 2, 0))),
    ("deferred", "pool", "wasm32", None),
    ("deferred", "bound", "wasm32", None),
    ("deferred", "local", "wasm32", Some((0, 2, 0))),
    ("deferred", "bare", "wasm32", Some((0, 2, 0))),
    ("fall_through", "pool", "host", Some((2, 2, 2))),
    ("fall_through", "bound", "host", Some((2, 2, 0))),
    ("fall_through", "local", "host", Some((2, 2, 0))),
    ("fall_through", "bare", "host", Some((2, 2, 0))),
    ("fall_through", "pool", "wasm32", None),
    ("fall_through", "bound", "wasm32", None),
    ("fall_through", "local", "wasm32", Some((0, 2, 0))),
    ("fall_through", "bare", "wasm32", Some((0, 2, 0))),
    ("return", "pool", "host", Some((2, 2, 2))),
    ("return", "bound", "host", Some((2, 2, 0))),
    ("return", "local", "host", Some((2, 2, 0))),
    ("return", "bare", "host", Some((2, 2, 0))),
    ("return", "pool", "wasm32", None),
    ("return", "bound", "wasm32", None),
    ("return", "local", "wasm32", Some((0, 2, 0))),
    ("return", "bare", "wasm32", Some((0, 2, 0))),
    ("test_failure", "pool", "host", Some((4, 4, 4))),
    ("test_failure", "bound", "host", Some((4, 4, 0))),
    ("test_failure", "local", "host", Some((4, 4, 0))),
    ("test_failure", "bare", "host", Some((4, 4, 0))),
    ("test_failure", "pool", "wasm32", None),
    ("test_failure", "bound", "wasm32", None),
    ("test_failure", "local", "wasm32", Some((0, 4, 0))),
    ("test_failure", "bare", "wasm32", Some((0, 4, 0))),
];

const QUIESCE: &str = "@lotus_bus_ingress_quiesce(";
const ABORT: &str = "@lotus_bus_wait_abort_all(";
const JOIN: &str = "@lotus_coop_pool_shutdown_all(";

fn obligation_of(line: &str) -> Option<Obligation> {
    let t = line.trim_start();
    if t.starts_with("declare") || !t.contains("call ") {
        return None;
    }
    if t.contains(QUIESCE) {
        Some(Obligation::IngressQuiesce)
    } else if t.contains(ABORT) {
        Some(Obligation::WaitAbort)
    } else if t.contains(JOIN) {
        Some(Obligation::PoolJoin)
    } else {
        None
    }
}

/// Each basic block's sequence of the three calls, in the block's order.
fn block_sequences(ir: &str) -> Vec<Vec<Obligation>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for l in ir.lines() {
        let t = l.trim_end();
        let label = !t.starts_with(' ') && !t.starts_with(';') && t.ends_with(':')
            || (!t.starts_with(' ') && t.contains(':') && t.contains("; preds"));
        if label || t.starts_with("define ") || t == "}" {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if let Some(o) = obligation_of(t) {
            cur.push(o);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn counts(ir: &str) -> (usize, usize, usize) {
    let n = |o: Obligation| ir.lines().filter(|l| obligation_of(l) == Some(o)).count();
    (n(Obligation::IngressQuiesce), n(Obligation::WaitAbort), n(Obligation::PoolJoin))
}

/// The five teardown spines.
const TEARDOWN_SPINES: &[Spine] =
    &[Spine::EagerTeardown, Spine::DeferredMainEntry, Spine::MainFallThrough, Spine::MainReturn, Spine::MainTestFailure];

/// The order the program's plan places the three obligations in, per
/// teardown spine (`LifecyclePlan::process_order`), as pairs: `(a, b)`
/// where some spine owes `a` before `b`. No two spines disagree.
fn plan_pairs(src: &str) -> BTreeSet<(Obligation, Obligation)> {
    let program = hale_syntax::parse_source(src).expect("parse");
    let snap = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())).unwrap_or_else(|_| panic!("no snapshot"));
    let plan = snap.demand_lifecycle().unwrap_or_else(|_| panic!("the lifecycle plan is blocked"));
    let mut pairs = BTreeSet::new();
    for &spine in TEARDOWN_SPINES {
        let order: Vec<Obligation> =
            plan.process_order(spine).expect("one order").iter().filter_map(|s| s.kind.capability()).collect();
        for (i, a) in order.iter().enumerate() {
            for b in &order[i + 1..] {
                pairs.insert((*a, *b));
            }
        }
    }
    for (a, b) in &pairs {
        assert!(!pairs.contains(&(*b, *a)), "the plan orders {} and {} both ways", a.name(), b.name());
    }
    pairs
}

/// A block's calls respect every order the plan states between two of
/// them: the first `a` comes ahead of the first `b`.
fn respects_the_plan(seq: &[Obligation], pairs: &BTreeSet<(Obligation, Obligation)>) -> Result<(), String> {
    for (before, after) in pairs {
        let b = seq.iter().position(|o| o == before);
        let a = seq.iter().position(|o| o == after);
        if let (Some(b), Some(a)) = (b, a) {
            if b > a {
                return Err(format!("{} before {}, which the plan orders after it", after.name(), before.name()));
            }
        }
    }
    Ok(())
}

struct Built {
    ir: String,
    err: Option<CodegenError>,
    bin: std::path::PathBuf,
}

fn build(src: &str, target: CompileTarget, trace: bool, name: &str) -> Built {
    let program = hale_syntax::parse_source(src).unwrap_or_else(|e| panic!("{name}: parse: {e:?}\n{src}"));
    let bin = harness::unique_bin(name);
    let out = if target == CompileTarget::Wasm32 { bin.with_extension("wasm") } else { bin.clone() };
    let ll = bin.with_extension("ll");
    let opts = BuildOptions { target, dump_ir: Some(ll.clone()), lifecycle_trace: trace, ..build_opts::options() };
    let err = build_executable_with_options(&program, &out, &[], &opts).err();
    let ir = std::fs::read_to_string(&ll).unwrap_or_default();
    let _ = std::fs::remove_file(&ll);
    Built { ir, err, bin: out }
}

fn cleanup(bin: &Path) {
    for p in [bin.to_path_buf(), bin.with_extension("mjs"), bin.with_extension("wasm")] {
        let _ = std::fs::remove_file(p);
    }
}

struct Ran {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run(cmd: &mut Command) -> Ran {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    let start = Instant::now();
    loop {
        if child.try_wait().expect("wait").is_some() {
            break;
        }
        if start.elapsed() > DEADLINE {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let out = child.wait_with_output().expect("output");
    Ran {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code(),
    }
}

/// A wasm module's imported names, from its import section (id 2).
fn wasm_imports(bytes: &[u8]) -> Option<Vec<String>> {
    Some(wasm_module::imports(bytes)?.into_iter().map(|i| i.name).collect())
}

fn tool(name: &str) -> Option<String> {
    [name.to_string(), format!("{name}-18")].into_iter().find(|c| Command::new(c).arg("--version").output().is_ok())
}

fn seq_of(t: &Trace, kind: ObligationKind, spine: Spine, point: Point) -> Option<u64> {
    t.events
        .iter()
        .find(|e| e.kind == kind && e.spine == Some(spine) && e.point.satisfies(point))
        .map(|e| e.seq)
}

/// The host: the trace's spine owes what the cells select, in the
/// plan's order, before the main locus's cascade; no reclaim spine or
/// cascade owes a process-wide obligation.
fn check_host_trace(shape: Shape, variant: Variant, t: &Trace, pairs: &BTreeSet<(Obligation, Obligation)>) -> Result<(), String> {
    let cells = LoweringCells::of(TargetClass::of(&hale_types::target::TargetSpec::host()).expect("a host column"));
    let spine = shape.spine();
    let abort = seq_of(t, ObligationKind::WaitAbort, spine, Point::Completed);
    let join_entered = seq_of(t, ObligationKind::PoolJoin, spine, Point::Entered);
    let join_done = seq_of(t, ObligationKind::PoolJoin, spine, Point::Completed);
    if abort.is_some() != cells.emits(Obligation::WaitAbort) {
        return Err(format!("WaitAbort@{}: {abort:?}", spine.name()));
    }
    let owes_join = cells.emits(Obligation::PoolJoin) && variant.pools();
    if join_done.is_some() != owes_join {
        return Err(format!("PoolJoin@{}: {join_done:?}, owed {owes_join}", spine.name()));
    }
    // The plan's traced order: the abort completes before the join is
    // entered.
    if pairs.contains(&(Obligation::WaitAbort, Obligation::PoolJoin)) {
        if let (Some(a), Some(j)) = (abort, join_entered) {
            if a > j {
                return Err(format!("PoolJoin@{} entered before WaitAbort completed", spine.name()));
            }
        }
    } else if variant.pools() {
        return Err(format!("the plan does not order WaitAbort before PoolJoin on {}", spine.name()));
    }
    // Both before the main locus's dissolve.
    let app_dissolve = t
        .events
        .iter()
        .find(|e| e.kind == ObligationKind::Dissolve && e.decl.as_deref() == Some("App"))
        .map(|e| e.seq);
    for (what, s) in [("WaitAbort", abort), ("PoolJoin", join_done)] {
        if let (Some(s), Some(d)) = (s, app_dissolve) {
            if s > d {
                return Err(format!("{what}@{} after App's dissolve", spine.name()));
            }
        }
    }
    for e in &t.events {
        if matches!(e.kind, ObligationKind::WaitAbort | ObligationKind::PoolJoin)
            && matches!(e.spine, Some(Spine::Reclaim | Spine::Cascade | Spine::DeferredEntry))
        {
            return Err(format!("{}@{} owes no process-wide obligation", e.kind.name(), e.spine.map(Spine::name).unwrap_or("-")));
        }
    }
    Ok(())
}

#[test]
fn every_spine_owes_what_its_target_selects_in_the_plans_order() {
    if tool("clang").is_none() || tool("wasm-ld").is_none() {
        eprintln!("SKIP: the wasm32 builds need clang and wasm-ld, and one is missing");
        return;
    }
    let node = tool("node");
    if node.is_none() {
        eprintln!("SKIP (the node runs only): node is missing");
    }
    let mut failures = Vec::new();
    let mut observed = Vec::new();
    for &shape in SHAPES {
        for &variant in VARIANTS {
            let src = source(shape, variant);
            let id = format!("{}/{}", shape.name(), variant.name());
            let pairs = plan_pairs(&src);
            for (target, tname) in [(CompileTarget::Native, "host"), (CompileTarget::Wasm32, "wasm32")] {
                let name = format!("hale_cells_{}_{}_{}", shape.name(), variant.name(), tname);
                let b = build(&src, target, false, &name);
                let got: Counts = if b.err.is_none() { Some(counts(&b.ir)) } else { None };
                observed.push(format!("(\"{}\", \"{}\", \"{tname}\", {got:?}),", shape.name(), variant.name()));
                let pinned = PINNED
                    .iter()
                    .find(|(s, v, t, _)| *s == shape.name() && *v == variant.name() && *t == tname)
                    .map(|p| p.3);
                if pinned != Some(got) {
                    failures.push(format!("{id} {tname}: counts {got:?}, pinned {pinned:?}"));
                }
                // The refusals: wasm32 refuses the pool at its placement
                // entry and the binding at its entry.
                if let Some(e) = &b.err {
                    let refused = matches!(e, CodegenError::CapabilityRefused(..));
                    let msg = e.to_string();
                    let expected = target == CompileTarget::Wasm32
                        && match variant {
                            Variant::Pool => msg.contains("placement entry `w`"),
                            Variant::Bound => msg.contains("Evt"),
                            _ => false,
                        };
                    if !(refused && expected) {
                        failures.push(format!("{id} {tname}: unexpected refusal: {msg}"));
                    }
                    cleanup(&b.bin);
                    continue;
                }
                for seq in block_sequences(&b.ir) {
                    if let Err(why) = respects_the_plan(&seq, &pairs) {
                        failures.push(format!("{id} {tname}: a block emits {seq:?}: {why}"));
                    }
                }
                if target == CompileTarget::Wasm32 {
                    let bytes = std::fs::read(&b.bin).unwrap_or_default();
                    let imports = wasm_imports(&bytes).unwrap_or_else(|| panic!("{id}: not a wasm module"));
                    let program = hale_syntax::parse_source(&src).expect("built above");
                    if let Err(e) = wasm_module::backstop(&format!("target_lifecycle_cells::{id}"), &program, &b.bin) {
                        failures.push(e);
                    }
                    // The thread and async_io startup the cells reject. (The
                    // import backstop, below, holds the whole import list:
                    // the runtime's thread joins and waits are compiled out
                    // of a wasm32 module, P3 T7.)
                    for forbidden in ["pthread_create", "epoll_", "eventfd"] {
                        if let Some(i) = imports.iter().find(|i| i.starts_with(forbidden)) {
                            failures.push(format!("{id} wasm32: the module imports `{i}`"));
                        }
                    }
                    if let Some(node) = &node {
                        let r = run(Command::new(node).arg(b.bin.with_extension("mjs")));
                        let finished = r.stdout.lines().any(|l| l == "end");
                        let ok = r.code == Some(0) || shape == Shape::TestFailure;
                        if !(finished && ok) {
                            failures.push(format!("{id} wasm32 under node: {:?}\n{}\n{}", r.code, r.stdout, r.stderr));
                        }
                    }
                }
                cleanup(&b.bin);
            }
            // The host's run, under the trace (not the bound variant: its
            // binding has no peer).
            if variant != Variant::Bound {
                let name = format!("hale_cells_{}_{}_trace", shape.name(), variant.name());
                let b = build(&src, CompileTarget::Native, true, &name);
                if let Some(e) = &b.err {
                    failures.push(format!("{id} host trace build: {e}"));
                    continue;
                }
                let r = run(&mut Command::new(&b.bin));
                cleanup(&b.bin);
                let want = if shape == Shape::TestFailure { Some(1) } else { Some(0) };
                if r.code != want || !r.stdout.lines().any(|l| l == "end") {
                    failures.push(format!("{id} host run: {:?}\n{}\n{}", r.code, r.stdout, r.stderr));
                    continue;
                }
                match trace::parse(&r.stderr) {
                    Ok(t) => {
                        if let Err(why) = check_host_trace(shape, variant, &t, &pairs) {
                            failures.push(format!("{id} host trace: {why}"));
                        }
                    }
                    Err(e) => failures.push(format!("{id} host trace does not parse: {e}")),
                }
            }
        }
    }
    eprintln!("observed counts:\n{}", observed.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
