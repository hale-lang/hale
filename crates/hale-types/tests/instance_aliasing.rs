//! One locus instance shared by two differently-placed towers (#333,
//! the `InstanceId` half of #334).
//!
//! F.31 keeps a locus's methods on its own pool's thread, but reasons
//! per FIELD DECLARATION: for `self.s.bump()` inside `WorkerA`, `s`
//! inherits WorkerA's pool, and WorkerB reasons identically about its
//! own `s`. Neither is wrong about its own field — they are wrong
//! about each other, and nothing related the two declarations back to
//! the single object they both name.
//!
//! Measured consequence before this: two pinned workers each doing
//! 100k increments on one shared locus produced ~140k of 200k, with
//! `hale check` reporting `ok`.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::error::DiagKind;
use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

const SHARED: &str = "\
locus Shared { params { n: Int = 0; } fn bump() { self.n = self.n + 1; } }
locus A { params { s: Shared = Shared { }; } run() { self.s.bump(); } }
locus B { params { s: Shared = Shared { }; } run() { self.s.bump(); } }
";

#[test]
fn aliasing_into_two_pools_is_reported() {
    let src = format!(
        "{SHARED}
main locus App {{
    params {{ sh: Shared = Shared {{ }};
             a: A = A {{ s: self.sh }};
             b: B = B {{ s: self.sh }}; }}
    placement {{ a: pinned(core = 0); b: pinned(core = 1); }}
}}
fn main() {{ App {{ }}; }}"
    );
    let ds = diags(&src);
    let hit = ds
        .iter()
        .find(|m| m.contains("is shared by"))
        .unwrap_or_else(|| panic!("cross-pool aliasing must be reported: {:?}", ds));
    assert!(
        hit.contains("self.sh") && hit.contains('a') && hit.contains('b'),
        "the report must name the instance and both holders: {}",
        hit
    );
}

/// Sharing within ONE pool is ordinary composition — handler
/// serialization already orders the accesses.
#[test]
fn aliasing_within_one_pool_is_fine() {
    let src = format!(
        "{SHARED}
main locus App {{
    params {{ sh: Shared = Shared {{ }};
             a: A = A {{ s: self.sh }};
             b: B = B {{ s: self.sh }}; }}
}}
fn main() {{ App {{ }}; }}"
    );
    assert!(
        !diags(&src).iter().any(|m| m.contains("is shared by")),
        "same-pool sharing must not be reported"
    );
}

#[test]
fn a_single_holder_is_fine_however_it_is_placed() {
    let src = format!(
        "{SHARED}
main locus App {{
    params {{ sh: Shared = Shared {{ }}; a: A = A {{ s: self.sh }}; }}
    placement {{ a: pinned(core = 0); }}
}}
fn main() {{ App {{ }}; }}"
    );
    assert!(
        !diags(&src).iter().any(|m| m.contains("is shared by")),
        "one holder cannot race with itself"
    );
}

/// A WARNING, not an error. The sanctioned way to share across pools
/// is a `@form(..., sync = ...)` locus, and a plain locus whose
/// mutable state sits entirely behind such fields is a legitimate
/// design — two applications in a downstream fleet do exactly that.
/// Distinguishing those needs the declared shared-locus surface
/// discussed on #333; until then this reports without failing builds.
#[test]
fn the_report_does_not_fail_the_build() {
    let src = format!(
        "{SHARED}
main locus App {{
    params {{ sh: Shared = Shared {{ }};
             a: A = A {{ s: self.sh }};
             b: B = B {{ s: self.sh }}; }}
    placement {{ a: pinned(core = 0); b: pinned(core = 1); }}
}}
fn main() {{ App {{ }}; }}"
    );
    let program = parse_source(&src).expect("parse");
    let ds = entries::check_program(&program);
    assert!(
        ds.iter().any(|d| d.message.contains("is shared by")),
        "the aliasing should still be surfaced"
    );
    assert!(
        !ds.iter().any(|d| {
            d.message.contains("is shared by")
                && matches!(d.kind, DiagKind::Type)
        }),
        "but as a warning, not a type error"
    );
}

// ---- GH #984: a borrow across thread domains --------------------------
//
// A handle stored by name into a locus-carrying field is a borrow (GH
// #730), and the lifetime pass decides only whether it outlives its holder.
// The thread-domain axis is the cross-pool hand-over rule of GH #1566
// (spec/types.md § "One instance is never shared across pools"), kept apart
// from the lifetime table so a program is told which axis it is wrong on.
// These pin #984's acceptance against it.

const BORROWED: &str = "\
locus Store { params { n: Int = 0; } fn bump() -> Int { self.n = self.n + 1; return self.n; } }
locus Reader { params { store: Store; } fn read() -> Int { return self.store.bump(); } }
";

fn app(placement: &str) -> String {
    format!(
        "{BORROWED}
main locus App {{
    params {{ store: Store = Store {{ }}; reader: Reader = Reader {{ store: self.store }}; }}
    placement {{ {placement} }}
    run() {{ }}
}}
fn main() {{ App {{ }}; }}"
    )
}

#[test]
fn gh984_a_borrow_between_two_instances_of_one_pool_checks_clean() {
    let ds = diags(&app("store: cooperative(pool = io); reader: cooperative(pool = io);"));
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn gh984_a_borrow_across_a_placement_boundary_is_refused_naming_both_domains() {
    for (placement, there, here) in [
        ("store: cooperative(pool = io);", "cooperative(pool = io)", "cooperative(pool = main)"),
        ("store: pinned(core = 0); reader: cooperative(pool = io);", "pinned", "cooperative(pool = io)"),
    ] {
        let ds = diags(&app(placement));
        let hit = ds
            .iter()
            .find(|m| m.starts_with("cross-pool alias:"))
            .unwrap_or_else(|| panic!("{placement}: the borrow across domains must be refused: {ds:?}"));
        assert!(hit.contains("`self.store`") && hit.contains("`Reader`"), "the witness names the handle and its holder: {hit}");
        assert!(hit.contains(there) && hit.contains(here), "{placement}: both domains are named: {hit}");
        assert!(!ds.iter().any(|m| m.contains("does not outlive")), "the lifetime axis says nothing: {ds:?}");
    }
}

#[test]
fn gh984_a_synced_type_crossing_is_admitted() {
    let src = "\
type Row { id: Int; v: Int = 0; }
@form(hashmap, sync = serialized)
locus Table { capacity { pool rows of Row indexed_by id; } }
locus Reader { params { t: Table; } fn read() -> Int { return self.t.len(); } }
main locus App {
    params { t: Table = Table { }; reader: Reader = Reader { t: self.t }; }
    placement { t: cooperative(pool = io); }
    run() { println(self.reader.read()); }
}
fn main() { App { }; }";
    let ds = diags(src);
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn gh984_a_bus_delivered_copy_is_not_a_borrow() {
    let src = "\
type Snap { n: Int; }
topic Snaps { payload: Snap; subject: \"t.snaps\"; }
locus Store {
    params { n: Int = 0; }
    bus { publish Snaps; }
    run() { self.n = self.n + 1; Snaps <- Snap { n: self.n }; }
}
locus Reader {
    params { last: Snap = Snap { n: 0 }; }
    bus { subscribe Snaps as on_snap; }
    fn on_snap(s: Snap) { self.last = s; }
}
main locus App {
    params { store: Store = Store { }; reader: Reader = Reader { }; }
    placement { store: cooperative(pool = io); }
    run() { }
}
fn main() { App { }; }";
    let ds = diags(src);
    assert!(ds.is_empty(), "{ds:?}");
}
