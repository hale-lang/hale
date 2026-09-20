//! GH #825: every bundle-level check looks inside `module { … }`.
//!
//! PR #823 (GH #764) made the hot-path allocation lint walk into a
//! module and, checking the siblings, found that every other
//! bundle-level check in `check.rs` had the identical top-level-only
//! shape: iterate `program.items`, match `TopDecl::Fn` /
//! `TopDecl::Locus`, no `TopDecl::Module` arm. A declaration one
//! brace deeper was invisible — and most of these are HARD errors,
//! so a program rejected at the top level was ACCEPTED with its loci
//! in a module.
//!
//! Nothing else in the frontend reads a module that way. The
//! resolver's `register_top_decls` recurses and keys `TopScope` by
//! the BARE name, the typecheck pass recurses, the effects manifest
//! recurses. A module is a namespace, not an analysis boundary.
//!
//! ## The shape of every test here
//!
//! One declaration text, checked twice: at the top level (the
//! CONTROL — this is the behavior that already worked and must not
//! move) and wrapped in `module inner { … }` (the REGRESSION — this
//! was silent). Both must produce the same message. Writing the
//! program once and wrapping it is deliberate: two hand-written
//! copies drift, and a drifted control proves nothing.
//!
//! The programs are plain `"…"` literals rather than `r#"…"#` on
//! purpose — `hale-corpus` harvests raw-string literals out of test
//! files into the corpus-wide properties, and these are
//! *deliberately-diagnostic* programs whose only job is to be
//! rejected here.

use hale_syntax::parse_source;
use hale_types::check_program;

/// `(is_error, message)` for every diagnostic, in order.
fn diags(src: &str) -> Vec<(bool, String)> {
    let prog = parse_source(src).unwrap_or_else(|e| {
        panic!("parse failed: {:?}\n--- source ---\n{}", e, src)
    });
    check_program(&prog)
        .into_iter()
        .map(|d| (d.is_error(), d.message))
        .collect()
}

/// The same declarations, one brace deeper. `fn main` stays outside:
/// it is the process entry point, not part of what is being hidden.
fn in_module(decls: &str) -> String {
    let body: String = decls
        .lines()
        .map(|l| {
            if l.trim().is_empty() {
                String::from("\n")
            } else {
                format!("    {}\n", l)
            }
        })
        .collect();
    format!("module inner {{\n{}}}\n", body)
}

const MAIN: &str = "fn main() { App { }; }\n";

/// Check `decls` at the top level and inside a module, and return
/// the messages matching `needle` from each.
fn control_and_nested(
    decls: &str,
    needle: &str,
) -> (Vec<(bool, String)>, Vec<(bool, String)>) {
    let flat = format!("{}\n{}", decls, MAIN);
    let nested = format!("{}\n{}", in_module(decls), MAIN);
    let pick = |src: &str| -> Vec<(bool, String)> {
        diags(src)
            .into_iter()
            .filter(|(_, m)| m.contains(needle))
            .collect()
    };
    (pick(&flat), pick(&nested))
}

/// The whole assertion, for a check whose finding is one diagnostic:
/// the control still fires, the module-nested program fires the SAME
/// message, and the severity is unchanged.
fn assert_module_matches_top_level(decls: &str, needle: &str) {
    let (flat, nested) = control_and_nested(decls, needle);
    assert_eq!(
        flat.len(),
        1,
        "the top-level CONTROL must produce exactly one `{}` finding \
         (if this fails the test program is wrong, not the walk); \
         got: {:?}",
        needle,
        flat
    );
    assert_eq!(
        nested.len(),
        1,
        "expected the same finding one brace deeper, inside \
         `module inner {{ … }}`; got: {:?}",
        nested
    );
    assert_eq!(
        nested[0], flat[0],
        "a module changes the namespace, not what the check has to \
         say: message and severity must match the top-level control"
    );
}

// ---- check_unowned_subscriber_locus --------------------------------
//
// A bus-subscribing locus instantiated unowned inside another
// locus's bus handler: the handler returns after each message, so
// the subscriber dissolves before it could receive a second one.
// A hard error — and it was silent with the two loci in a module.

const UNOWNED_SUBSCRIBER: &str = "\
type Ping { n: Int = 0; }

topic Tick { payload: Ping; }

locus Watcher {
    params { n: Int = 0; }
    bus { subscribe Tick as on_tick; }
    fn on_tick(p: Ping) { }
    run() { }
}

locus Hub {
    params { n: Int = 0; }
    bus { subscribe Tick as on_msg; }
    fn on_msg(p: Ping) {
        let w = Watcher { };
    }
    run() { }
}

main locus App {
    params { h: Hub = Hub { }; }
    run() { }
}
";

#[test]
fn unowned_subscriber_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        UNOWNED_SUBSCRIBER,
        "instantiated unowned",
    );
}

// ---- check_accept_release ------------------------------------------
//
// The closest sibling to the hot-path lint, and the one GH #825 was
// verified against first: a locus that accepts children and never
// releases them, whose `run()` loops forever, accumulates resident
// children until OOM. A warning — and it stayed a warning when the
// walk reached inside the module, which is the other half of the
// contract.

const ACCEPT_WITHOUT_RELEASE: &str = "\
locus Child {
    params { n: Int = 0; }
    run() { }
}

locus Pool {
    params { n: Int = 0; }
    accept(c: Child) { }
    run() {
        while true {
            std::time::sleep(10ms);
        }
    }
}

main locus App {
    params { p: Pool = Pool { }; }
    run() { println(\"hi\"); }
}
";

#[test]
fn accept_without_release_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        ACCEPT_WITHOUT_RELEASE,
        "declares no `release(",
    );
}

// ---- check_cooperative_pool_blocking -------------------------------
//
// A non-main cooperative subscriber whose `run()` makes a blocking
// call is a DEAD RECEIVER: the blocking call holds the pool's OS
// thread, so the dispatch that would deliver to its handlers never
// runs. A hard error, and silent inside a module.

const DEAD_RECEIVER: &str = "\
type Tick { n: Int; }

locus Gateway {
    bus { subscribe \"tick\" as on_tick of type Tick; }
    fn on_tick(t: Tick) { }
    run() { let n = std::io::tls::recv_into(0, 0, 64); }
}

locus Feed {
    bus { publish \"tick\" of type Tick; }
    run() { \"tick\" <- Tick { n: 1 }; }
}

main locus App {
    params {
        gw: Gateway = Gateway { };
        feed: Feed  = Feed { };
    }
    placement {
        gw: cooperative(pool = ws);
    }
}
";

#[test]
fn dead_receiver_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        DEAD_RECEIVER,
        "monopolizes the pool's thread",
    );
}

// ---- check_nested_long_running_child -------------------------------
//
// A non-main locus with work of its own, holding a params field of a
// locus type whose `run()` never returns: nested cooperative children
// share the parent's OS thread and the child's `run()` runs to
// completion first, so the parent never starts. A hard error.

const NESTED_LONG_RUNNING: &str = "\
locus Daemon {
    params { n: Int = 0; }
    run() {
        while true {
            std::time::sleep(1s);
        }
    }
}

locus Parent {
    params { d: Daemon = Daemon { }; }
    run() { println(\"work\"); }
}

main locus App {
    params { p: Parent = Parent { }; }
    run() { }
}
";

#[test]
fn nested_long_running_child_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        NESTED_LONG_RUNNING,
        "with a non-trivial `run()` body of its own",
    );
}

#[test]
fn a_top_level_parent_sees_a_module_nested_childs_run() {
    // The index half of the same defect, and the half a walk-only
    // fix would miss: the PARENT is at the top level, so the walk
    // always reached it — but `Daemon` lived in a module, so the
    // name → `&LocusDecl` index had no entry and the child resolved
    // as "not long-running".
    let src = "\
module inner {
    locus Daemon {
        params { n: Int = 0; }
        run() {
            while true {
                std::time::sleep(1s);
            }
        }
    }
}

locus Parent {
    params { d: Daemon = Daemon { }; }
    run() { println(\"work\"); }
}

main locus App {
    params { p: Parent = Parent { }; }
    run() { }
}

fn main() { App { }; }
";
    let ds = diags(src);
    assert!(
        ds.iter().any(|(is_err, m)| *is_err
            && m.contains("with a non-trivial `run()` body of its own")),
        "a top-level parent holding a module-nested long-running child \
         must be flagged; got: {:?}",
        ds
    );
}

// ---- compute_pool_of_locus_type / check_placement_single_thread ----
//
// The F.31 single-threaded-method invariant: a direct
// `self.<field>.method()` call whose receiver is placed on another
// pool is a hard error, because cross-pool coordination goes through
// the bus. The whole layer hangs off finding the `main locus`, and
// that lookup stopped at the top level — so a main locus inside a
// module produced an EMPTY pool map, which every caller reads as
// "nothing placed here" and returns on.

const CROSS_POOL_CALL: &str = "\
locus DB {
    fn query() { }
}

main locus App {
    params {
        db: DB = DB { };
    }
    placement {
        db: pinned;
    }
    run() {
        self.db.query();
    }
}
";

#[test]
fn cross_pool_call_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        CROSS_POOL_CALL,
        "cross-pool method call",
    );
}

#[test]
fn a_module_nested_main_locus_still_seeds_the_pool_map() {
    // `compute_pool_of_locus_type` is `pub` and is re-run outside
    // this pass (sync inference, the pre-codegen finalization), so
    // an empty map is not just a missing diagnostic — it is a
    // different answer to "where does this locus run".
    let nested = format!("{}\n{}", in_module(CROSS_POOL_CALL), MAIN);
    let prog = parse_source(&nested).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(String::new(), &prog);
    let bundle = hale_types::Bundle::new(programs);
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    let pools = hale_types::check::compute_pool_of_locus_type(&bundle, &top);
    assert!(
        pools.contains_key("App") && pools.contains_key("DB"),
        "a module-nested main locus must still seed the pool map; \
         got: {:?}",
        pools
    );
}

// ---- check_pool_affinity -------------------------------------------
//
// Two placement entries naming ONE pool with two different
// affinities contradict each other — a pool has one worker thread.
// A hard error, silent inside a module.

const CONTRADICTORY_AFFINITY: &str = "\
locus A { run() { } }
locus B { run() { } }

main locus App {
    params {
        a: A = A { };
        b: B = B { };
    }
    placement {
        a: cooperative(pool = io, core = 0);
        b: cooperative(pool = io, core = 1);
    }
}
";

#[test]
fn contradictory_pool_affinity_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        CONTRADICTORY_AFFINITY,
        "is given two different",
    );
}

// ---- check_bounded_bus ---------------------------------------------
//
// `bounded(N)` on a topic with no `on_full:` policy is a capacity
// with no declared behavior — a hard error. It reads a `topic`
// declaration, which is the one `TopDecl` variant none of the other
// walks in this file touch.

const BOUNDED_WITHOUT_POLICY: &str = "\
type E { n: Int = 0; }

topic Evt {
    payload: E;
    subject: \"evt\";
    bounded(12);
}

main locus App {
    params { n: Int = 0; }
    bus { publish Evt; }
    run() { }
}
";

#[test]
fn bounded_topic_without_policy_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        BOUNDED_WITHOUT_POLICY,
        "requires `on_full: fail;`",
    );
}

// ---- check_phase3_fallback_subscribers -----------------------------
//
// `on_unmatched: fallback` with no `where key == _` subscriber is a
// hard error: an unmatched-key publish would have nowhere to go.
// The topic side and the subscriber side are separate walks, so
// there are two regressions.

const FALLBACK_WITHOUT_CATCHALL: &str = "\
type Reading { sensor: Int = 0; v: Int = 0; }

topic Readings {
    payload: Reading;
    subject: \"sense.reading\";
    keyed_by sensor;
    on_unmatched: fallback;
}

main locus App {
    params { seen: Int = 0; }
    bus { subscribe Readings as on_r where key == replica; }
    fn on_r(r: Reading) { self.seen = self.seen + 1; }
}
";

#[test]
fn fallback_topic_without_catchall_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        FALLBACK_WITHOUT_CATCHALL,
        "no subscriber declares `where key == _`",
    );
}

#[test]
fn a_module_nested_catchall_subscriber_satisfies_a_top_level_topic() {
    // The subscriber-side walk, and the direction that matters most:
    // it must not turn a CORRECT program red. The topic is at the top
    // level and its catch-all subscriber is in a module — before the
    // fix the subscriber was invisible, so the topic was reported as
    // having no catch-all at all.
    let src = "\
type Reading { sensor: Int = 0; v: Int = 0; }

topic Readings {
    payload: Reading;
    subject: \"sense.reading\";
    keyed_by sensor;
    on_unmatched: fallback;
}

module inner {
    locus Catcher {
        params { seen: Int = 0; }
        bus { subscribe Readings as on_any where key == _; }
        fn on_any(r: Reading) { self.seen = self.seen + 1; }
    }
}

main locus App {
    params { c: Catcher = Catcher { }; }
    run() { }
}

fn main() { App { }; }
";
    let ds = diags(src);
    assert!(
        !ds.iter()
            .any(|(_, m)| m.contains("no subscriber declares `where key == _`")),
        "a module-nested catch-all subscriber satisfies the topic; got: \
         {:?}",
        ds
    );
}

// ---- check_main_and_bindings ---------------------------------------
//
// At most one `main` locus per bundle, and every `bindings` entry
// names a declared topic. The main count is the sharpest case in
// GH #825: a count that skips half the declarations is not a count,
// and hiding a second `main locus` in a module made the bundle look
// singular.

const TWO_MAINS: &str = "\
main locus App {
    params { n: Int = 0; }
    run() { }
}

main locus Other {
    params { n: Int = 0; }
    run() { }
}
";

#[test]
fn a_second_main_locus_inside_a_module_is_counted() {
    // Both mains at the top level: two findings (one per main).
    let flat = format!("{}\n{}", TWO_MAINS, MAIN);
    let flat_ds: Vec<_> = diags(&flat)
        .into_iter()
        .filter(|(_, m)| m.contains("more than one `main` locus"))
        .collect();
    assert_eq!(flat_ds.len(), 2, "top-level control: {:?}", flat_ds);

    // The second one one brace deeper is still a second one.
    let hidden = "\
main locus App {
    params { n: Int = 0; }
    run() { }
}

module inner {
    main locus Other {
        params { n: Int = 0; }
        run() { }
    }
}

fn main() { App { }; }
";
    let hidden_ds: Vec<_> = diags(hidden)
        .into_iter()
        .filter(|(_, m)| m.contains("more than one `main` locus"))
        .collect();
    assert_eq!(
        hidden_ds.len(),
        2,
        "a `main locus` inside a module is still a main locus; got: {:?}",
        hidden_ds
    );
    assert!(
        hidden_ds.iter().all(|(is_err, _)| *is_err),
        "and still an error: {:?}",
        hidden_ds
    );
}

const UNKNOWN_BOUND_TOPIC: &str = "\
main locus App {
    params { n: Int = 0; }
    bindings {
        Missing: unix(\"/tmp/hale-825.sock\", role: listen);
    }
    run() { }
}
";

#[test]
fn binding_on_an_unknown_topic_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        UNKNOWN_BOUND_TOPIC,
        "binding references unknown topic",
    );
}

#[test]
fn a_module_nested_advisory_stays_a_warning() {
    // Severity is part of the contract: reaching inside a module
    // must not promote an advisory to an error.
    let (flat, nested) =
        control_and_nested(ACCEPT_WITHOUT_RELEASE, "declares no `release(");
    assert!(!flat[0].0, "the control is a warning: {:?}", flat);
    assert!(!nested[0].0, "so is the nested one: {:?}", nested);
}
