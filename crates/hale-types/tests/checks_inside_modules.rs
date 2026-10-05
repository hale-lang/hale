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
//! Where the wrapped text holds the seed's only `main locus`, the
//! module variant has no entry, and the check says so (F.40 phase 3,
//! L4: a `main locus` inside a module is not the entry, decision 2).
//! That refusal is one more diagnostic, never a replacement, so the
//! comparison is over every message: the module variant reports all of
//! the control's, and the refusal is its one addition. The one message
//! of the control's it does not get is rule 9's orphan-topic warning,
//! which only a program with an entry is judged for (E0).
//!
//! The programs are plain escaped string literals rather than raw
//! strings on purpose — `hale-corpus` harvests raw-string literals
//! out of test files into the corpus-wide properties, and these are
//! *deliberately-diagnostic* programs whose only job is to be
//! rejected here. (The file therefore contains no raw-string opener
//! at all, doc comments included: the harvester scans the text, not
//! the parsed tokens.)

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

/// The refusal of a seed whose only `main locus`, `App`, is in `module
/// inner`.
const REFUSED: &str = "the entry must be top-level: `main locus App` inside `module inner` is not the \
                       program's entry, and nothing else in the seed is — move it out of the module";

/// Rule 9's orphan-topic warnings: judged only in a closed world, a
/// program with an entry, which a seed whose only `main locus` is
/// module-nested is not (decision 2, F.40 phase 3, E0).
fn closed_world_only(d: &(bool, String)) -> bool {
    !d.0 && d.1.starts_with("bus topic `")
}

/// Check `decls` at the top level and inside a module, and return
/// the messages matching `needle` from each. Every message is compared
/// first: the module variant reports each of the control's (as many
/// times), and, where `decls` holds the seed's only `main locus`, the
/// refusal as its one addition, an error; otherwise nothing more. The
/// one message of the control's a seed with no entry does not get is
/// rule 9's orphan warning ([`closed_world_only`]).
fn control_and_nested(
    decls: &str,
    needle: &str,
) -> (Vec<(bool, String)>, Vec<(bool, String)>) {
    let flat = diags(&format!("{}\n{}", decls, MAIN));
    let nested = diags(&format!("{}\n{}", in_module(decls), MAIN));
    let only_main = decls.contains("main locus");
    let mut added = nested.clone();
    for d in flat.iter().filter(|d| !(only_main && closed_world_only(d))) {
        let at = added.iter().position(|n| n == d).unwrap_or_else(|| {
            panic!(
                "the module variant must report every message of the \
                 control's; missing {:?}\ncontrol: {:?}\nmodule: {:?}",
                d, flat, nested
            )
        });
        added.remove(at);
    }
    let want: Vec<(bool, String)> = if only_main {
        vec![(true, REFUSED.to_string())]
    } else {
        Vec::new()
    };
    assert_eq!(
        added, want,
        "beyond the control's messages, the module variant reports \
         the refusal of a module-nested only `main locus` and nothing \
         else\ncontrol: {:?}\nmodule: {:?}",
        flat, nested
    );
    let pick = |ds: Vec<(bool, String)>| -> Vec<(bool, String)> {
        ds.into_iter().filter(|(_, m)| m.contains(needle)).collect()
    };
    (pick(flat), pick(nested))
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

// The warning half reaches its blocking call through a helper: a free
// fn, or a method of the locus's own. The set of helpers that block is
// the effect rows' (F.40 phase 3, E2), which key a module's fns by
// their bare names as the resolver does, so the helper one brace deeper
// still blocks.

const BLOCKING_FREE_HELPER: &str = "\
fn pump() { let n = std::io::tls::recv_into(0, 0, 64); }

locus Worker {
    params { n: Int = 0; }
    run() { pump(); }
}

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: cooperative(pool = io); }
}
";

#[test]
fn a_blocking_free_helper_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        BLOCKING_FREE_HELPER,
        "reaches the blocking call `pump() (which makes a blocking call)`",
    );
}

#[test]
fn a_top_level_run_sees_a_module_nested_helper_block() {
    // The helper alone one brace deeper: the `run()` that calls it is
    // at the top level, so only the set says `pump` blocks.
    let src = "\
module inner {
    fn pump() { let n = std::io::tls::recv_into(0, 0, 64); }
}

locus Worker {
    params { n: Int = 0; }
    run() { pump(); }
}

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: cooperative(pool = io); }
}

fn main() { App { }; }
";
    let ds = diags(src);
    assert!(
        ds.iter().any(|(e, m)| !e
            && m.contains("reaches the blocking call `pump() (which makes a blocking call)`")),
        "a top-level run() calling a module-nested blocking helper must warn: {:?}",
        ds
    );
}

const BLOCKING_SELF_HELPER: &str = "\
locus Worker {
    params { n: Int = 0; }
    fn pump() { let n = std::io::tls::recv_into(0, 0, 64); }
    run() { self.pump(); }
}

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: cooperative(pool = io); }
}
";

#[test]
fn a_blocking_self_helper_inside_a_module_is_flagged() {
    assert_module_matches_top_level(
        BLOCKING_SELF_HELPER,
        "reaches the blocking call `self.pump() (which makes a blocking call)`",
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

// ---- the placement table / check_placement_single_thread -----------
//
// The F.31 single-threaded-method invariant: a direct
// `self.<field>.method()` call whose receiver is placed on another
// pool is a hard error, because cross-pool coordination goes through
// the bus. The whole layer hangs off the `main locus` lowering
// deploys, the entry row's lowering root. The placed locus may live
// in a module, and the walk must see it there; so may the `main
// locus` itself, which is then not the entry (F.40 phase 3, E0,
// decision 2) but is still the root lowering deploys (until L4).

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

const CROSS_POOL_NEEDLE: &str = "cross-pool method call";

/// The top-level `main locus` and a placed locus one brace deeper.
const CROSS_POOL_CALL_TO_A_NESTED_LOCUS: &str = "\
module inner {
    locus DB {
        fn query() { }
    }
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

fn main() { App { }; }
";

/// The entry row, and the declarations the placement table places: what
/// the F.31 rule judges instances of.
fn pool_map(src: &str) -> (hale_types::entry::EntryRow, Vec<String>) {
    let mut prog = parse_source(src).expect("parse");
    let ids = hale_types::snapshot::mint([("", &mut prog)], &[]);
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(String::new(), &prog);
    let mut bundle = hale_types::Bundle::new(programs);
    bundle.snapshot = ids;
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    let entry = hale_types::entry::entry_row(&bundle);
    let table = hale_types::placement::derive_placement(&bundle, &top, &entry);
    let placed: std::collections::BTreeSet<String> =
        table.instances.values().filter_map(|r| r.realizes.as_ref().map(|d| d.lowered.clone())).collect();
    (entry, placed.into_iter().collect())
}

#[test]
fn cross_pool_call_to_a_locus_inside_a_module_is_flagged() {
    let control: Vec<_> = diags(&format!("{}\n{}", CROSS_POOL_CALL, MAIN))
        .into_iter()
        .filter(|(_, m)| m.contains(CROSS_POOL_NEEDLE))
        .collect();
    assert_eq!(control.len(), 1, "the top-level control: {:?}", control);
    let nested: Vec<_> = diags(CROSS_POOL_CALL_TO_A_NESTED_LOCUS)
        .into_iter()
        .filter(|(_, m)| m.contains(CROSS_POOL_NEEDLE))
        .collect();
    assert_eq!(nested, control, "a placed locus in a module is still placed");
}

#[test]
fn a_top_level_main_locus_seeds_the_pool_map_with_a_module_nested_locus() {
    // The placement table is read outside this pass too (sync
    // inference), so a missing row is not just a missing diagnostic —
    // it is a different answer to "where does this locus run".
    let (_, pools) = pool_map(CROSS_POOL_CALL_TO_A_NESTED_LOCUS);
    assert_eq!(pools, ["App", "DB"], "the entry seeds the table");
}

/// E0, decision 2: a `main locus` inside a module is not the entry.
/// Lowering still deploys it as the root until it reads the entry
/// (F.40 phase 3, L4), so it still seeds the placement table (GH #825)
/// and its cross-pool call is refused as the top-level one is: the
/// table reads the row's lowering root, not its entry.
#[test]
fn a_module_nested_main_locus_is_not_the_entry_and_still_seeds_the_pool_map() {
    let nested = format!("{}\n{}", in_module(CROSS_POOL_CALL), MAIN);
    let (entry, pools) = pool_map(&nested);
    assert_eq!(entry.no_entry(), Some(hale_types::entry::NoEntry::OnlyModuleNested));
    assert_eq!(entry.lowering_root.as_ref().map(|m| m.name.as_str()), Some("App"));
    assert_eq!(pools, ["App", "DB"], "the lowering root seeds the table");
    assert_module_matches_top_level(CROSS_POOL_CALL, CROSS_POOL_NEEDLE);
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

// ---- check_instance_aliasing / locus_has_unsynchronized_state ------
//
// One locus instance reached by two main-locus fields placed on
// different pools, holding unsynchronized mutable state: two threads
// reach that state with nothing ordering them. Measured at ~30% lost
// writes with `hale check` reporting `ok`, which is what this
// warning exists to say.

const ALIASED_ACROSS_POOLS: &str = "\
locus Shared { params { n: Int = 0; } fn bump() { self.n = self.n + 1; } }
locus A { params { s: Shared = Shared { }; } run() { self.s.bump(); } }
locus B { params { s: Shared = Shared { }; } run() { self.s.bump(); } }

main locus App {
    params { sh: Shared = Shared { };
             a: A = A { s: self.sh };
             b: B = B { s: self.sh }; }
    placement { a: pinned(core = 0); b: pinned(core = 1); }
}
";

#[test]
fn cross_pool_aliasing_inside_a_module_is_flagged() {
    assert_module_matches_top_level(ALIASED_ACROSS_POOLS, "is shared by");
}

#[test]
fn a_module_nested_form_still_answers_the_synchronized_question() {
    // `locus_has_unsynchronized_state` builds a `@form` -> has-sync
    // map to decide whether the alias actually races. A form the
    // walk cannot see is ABSENT from that map, and absent reads as
    // "not an unsynchronized form" — so a module-nested
    // `@form(hashmap)` with no `sync` discipline silenced the
    // warning for an alias whose loci are all at the top level.
    let src = "\
type Entry { k: Int = 0; v: Int = 0; }

module inner {
    @form(hashmap, key = k, value = v)
    locus Reg { params { k: Int = 0; v: Int = 0; } }
}

locus Shared { params { r: Reg = Reg { }; } }
locus A { params { s: Shared = Shared { }; } run() { } }
locus B { params { s: Shared = Shared { }; } run() { } }

main locus App {
    params { sh: Shared = Shared { };
             a: A = A { s: self.sh };
             b: B = B { s: self.sh }; }
    placement { a: pinned(core = 0); b: pinned(core = 1); }
}

fn main() { App { }; }
";
    let ds = diags(src);
    assert!(
        ds.iter().any(|(_, m)| m.contains("is shared by")
            && m.contains("with no `sync` discipline")),
        "a module-nested unsynchronized form must still make the alias \
         a race; got: {:?}",
        ds
    );
}

// ---- the `or wait` bound-topic set (GH #255) ------------------------
//
// Not on GH #825's list, and the only one of these that fails the
// OTHER way: every walk above loses a finding when it stops at the
// top level, this one INVENTS one. `or wait` is legal exactly when
// the topic has a transport binding, and a `bindings { }` block the
// prelude cannot see reads as "no transport" — so a correct program
// with its binding one brace deeper did not compile.

const OR_WAIT_ON_A_BOUND_TOPIC: &str = "\
type E { n: Int = 0; }

topic Evt {
    payload: E;
    subject: \"evt\";
}

main locus App {
    params { n: Int = 0; }
    bindings {
        Evt: unix(\"/tmp/hale-825-evt.sock\", role: listen);
    }
    bus { publish Evt; }
    run() {
        Evt <- E { n: 1 } or wait;
    }
}
";

#[test]
fn or_wait_accepts_a_module_nested_binding() {
    let (flat, nested) = control_and_nested(
        OR_WAIT_ON_A_BOUND_TOPIC,
        "`or wait` requires the topic",
    );
    assert!(
        flat.is_empty(),
        "the top-level control is legal and must be accepted: {:?}",
        flat
    );
    assert!(
        nested.is_empty(),
        "a `bindings` block inside a module still binds the topic, so \
         the same `or wait` is still legal: {:?}",
        nested
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

// ---- check_entry_point_placement -----------------------------------
//
// The one check here whose finding exists ONLY at depth, which is why
// it does not go through `control_and_nested`: the ENTRY POINT is the
// exception to everything above. `spec/semantics.md` § "Declarations
// inside `module { }`" has always said a seed's entry point is its
// top-level `fn main`, and codegen looks for it in `program.items`
// and nowhere else — so a seed whose only `fn main` is one brace
// deeper checked clean and then failed to build with codegen's
// spanless "program has no `fn main()`". The ruling on GH #911
// (2026-09-20): check says so, where the declaration is.

const ENTRY_MSG: &str = "the entry point must be top-level";

#[test]
fn a_module_nested_main_is_refused() {
    let ds = diags("module inner {\n    fn main() { }\n}\n");
    let found: Vec<&(bool, String)> =
        ds.iter().filter(|(_, m)| m.contains(ENTRY_MSG)).collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one entry-point finding, got: {:?}",
        ds
    );
    assert!(found[0].0, "it is a hard error, as the build is: {:?}", found);
    assert!(
        found[0].1.contains("module inner"),
        "it names the module the entry point has to leave: {:?}",
        found
    );
}

#[test]
fn a_main_two_modules_deep_is_refused() {
    let ds = diags(
        "module outer {\n    module inner {\n        fn main() { }\n    }\n}\n",
    );
    let found: Vec<&(bool, String)> =
        ds.iter().filter(|(_, m)| m.contains(ENTRY_MSG)).collect();
    assert_eq!(found.len(), 1, "got: {:?}", ds);
    assert!(
        found[0].1.contains("module inner"),
        "the innermost module is the one it is written in: {:?}",
        found
    );
}

/// The `main locus` half of the same exception (F.40 phase 3, L4): a
/// `main locus` inside a module is not the entry (decision 2), so a seed
/// whose only one is module-nested has nothing for lowering to deploy,
/// and the check refuses it once, an error at the locus's name. The
/// same declarations at the top level are the entry, and say nothing of
/// it; so does a module-nested `main locus` beside a top-level one,
/// which rule 1 refuses instead.
#[test]
fn a_module_nested_only_main_locus_is_refused_at_its_name() {
    let decls = "main locus App {\n    params { n: Int = 0; }\n}\n";
    let refusals = |src: &str| -> Vec<hale_syntax::Diag> {
        let prog = parse_source(src).expect("parse");
        check_program(&prog).into_iter().filter(|d| d.message.contains("the entry must be top-level")).collect()
    };
    assert!(refusals(&format!("{}\n{}", decls, MAIN)).is_empty(), "the top-level control is the entry");
    let nested = format!("{}\n{}", in_module(decls), MAIN);
    let found = refusals(&nested);
    assert_eq!(found.len(), 1, "refused once: {:?}", found);
    assert!(found[0].is_error(), "an error: {:?}", found);
    assert_eq!(found[0].message, REFUSED);
    let at = nested.find("App {").expect("the name");
    assert_eq!((found[0].span.start.as_usize(), found[0].span.end.as_usize()), (at, at + "App".len()), "at the name");
    let both = format!("main locus Top {{ }}\n{}\nfn main() {{ Top {{ }}; }}\n", in_module(decls));
    assert!(refusals(&both).is_empty(), "a seed with an entry is not refused for its module-nested main");
    // Two modules deep, the path names both.
    let deep = format!("module outer {{\n{}}}\n{}", in_module(decls), MAIN);
    let found = refusals(&deep);
    assert_eq!(found.len(), 1, "refused once: {:?}", found);
    assert!(found[0].message.contains("inside `module outer::inner`"), "{:?}", found);
}

/// The control: a top-level `fn main` beside a module full of
/// declarations is the ordinary shape of every other test in this
/// file, and it stays silent.
#[test]
fn a_top_level_main_beside_a_module_is_accepted() {
    let src = format!(
        "{}{}",
        in_module("type Point { x: Int = 0; }\nfn bare() -> Int { return 1; }"),
        "fn main() { println(bare()); }\n"
    );
    let ds = diags(&src);
    assert!(
        !ds.iter().any(|(_, m)| m.contains(ENTRY_MSG)),
        "a module is a namespace, not a reason to move `fn main`: {:?}",
        ds
    );
}

/// A free fn inside a module that is NOT the entry point is exactly
/// what the rest of this file is about: first class, and silent.
#[test]
fn a_module_nested_fn_of_another_name_is_accepted() {
    let ds = diags(
        "module inner {\n    fn mainish() -> Int { return 1; }\n}\n\
         fn main() { println(mainish()); }\n",
    );
    assert!(
        !ds.iter().any(|(_, m)| m.contains(ENTRY_MSG)),
        "only `main` is the entry point: {:?}",
        ds
    );
}
