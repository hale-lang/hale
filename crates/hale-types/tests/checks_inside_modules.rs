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

#[test]
fn a_module_nested_advisory_stays_a_warning() {
    // Severity is part of the contract: reaching inside a module
    // must not promote an advisory to an error.
    let (flat, nested) =
        control_and_nested(ACCEPT_WITHOUT_RELEASE, "declares no `release(");
    assert!(!flat[0].0, "the control is a warning: {:?}", flat);
    assert!(!nested[0].0, "so is the nested one: {:?}", nested);
}
