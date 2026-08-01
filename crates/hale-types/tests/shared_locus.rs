//! `@shared locus` — a declared coordination primitive (#333).
//!
//! F.31 reasons per field declaration, so it cannot tell a deliberate
//! cross-pool registry from an accidental alias. Both look like one
//! instance reachable from two towers. `@shared` is how the author
//! says which one it is, and the restraints are what make that a
//! claim rather than a label.
//!
//! What it does NOT do is prove every field is synchronized.
//! `@form(vec)` has no sync discipline in v1 — a real registry in a
//! downstream fleet holds one `sync = serialized` map beside an
//! unsynchronized vec — so that proof is not yet expressible. The
//! annotation pins the shape and keeps the gap reviewable.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// Pub/sub is domain-oriented and names what a system MEANS; a
/// coordination primitive is mechanical and names what it DOES. A
/// locus is one or the other.
#[test]
fn a_shared_locus_may_not_be_a_bus_participant() {
    let ds = diags(
        "type P { n: Int; }\n\
         topic T { payload: P; }\n\
         @shared\n\
         locus R { params { n: Int = 0; } bus { subscribe T as on_t; }\n\
           fn on_t(p: P) { } }\n\
         locus Q { bus { publish T; } fn go() { T <- P { n: 1 }; } }\n\
         main locus App { params { r: R = R { }; q: Q = Q { }; } }\n\
         fn main() { App { }; }",
    );
    assert!(
        ds.iter().any(|m| m.contains("may not declare a `bus")),
        "a shared locus must not carry a bus block: {:?}",
        ds
    );
}

/// A bare `self.n = v` is exactly the race the annotation is supposed
/// to be ruling out; mutable state belongs to a field whose own form
/// carries a discipline.
#[test]
fn a_shared_locus_may_not_assign_its_own_fields() {
    let ds = diags(
        "@shared\n\
         locus R { params { n: Int = 0; } fn set(v: Int) { self.n = v; } }\n\
         main locus App { params { r: R = R { }; } }\n\
         fn main() { App { }; }",
    );
    assert!(
        ds.iter().any(|m| m.contains("may not assign to its own fields")),
        "a bare self-field assignment must be rejected: {:?}",
        ds
    );
}

const REGISTRY: &str = "\
type E { k: Int; v: Int; }
@form(hashmap, sync = serialized)
locus Counts { capacity { pool entries of E indexed_by k; } }
@shared
locus Registry {
    params { store: Counts = Counts { }; }
    fn record(e: E) { self.store.set(e); }
}
locus A { params { r: Registry = Registry { }; } run() { self.r.record(E { k: 1, v: 1 }); } }
locus B { params { r: Registry = Registry { }; } run() { self.r.record(E { k: 2, v: 2 }); } }
";

/// The point of the annotation: a declared coordination primitive
/// reached from two pools is its purpose, not an accident.
#[test]
fn a_declared_shared_locus_may_be_reached_from_two_pools() {
    let src = format!(
        "{REGISTRY}
main locus App {{
    params {{ reg: Registry = Registry {{ }};
             a: A = A {{ r: self.reg }};
             b: B = B {{ r: self.reg }}; }}
    placement {{ a: pinned(core = 0); b: pinned(core = 1); }}
}}
fn main() {{ App {{ }}; }}"
    );
    assert!(
        !diags(&src).iter().any(|m| m.contains("is shared by")),
        "declared sharing must not be reported as accidental"
    );
}

/// And the distinction has to cut both ways, or the annotation is
/// decoration: an UNdeclared locus in the identical shape is still
/// reported.
#[test]
fn an_undeclared_locus_in_the_same_shape_is_still_reported() {
    let src = format!(
        "{}
main locus App {{
    params {{ reg: Registry = Registry {{ }};
             a: A = A {{ r: self.reg }};
             b: B = B {{ r: self.reg }}; }}
    placement {{ a: pinned(core = 0); b: pinned(core = 1); }}
}}
fn main() {{ App {{ }}; }}",
        REGISTRY.replace("@shared\n", "")
    );
    assert!(
        diags(&src).iter().any(|m| m.contains("is shared by")),
        "without the annotation the aliasing must still be reported"
    );
}

/// The restraints apply to the annotation, not to every locus.
#[test]
fn an_ordinary_locus_keeps_its_bus_block_and_its_fields() {
    let ds = diags(
        "type P { n: Int; }\n\
         topic T { payload: P; }\n\
         locus R { params { n: Int = 0; } bus { subscribe T as on_t; }\n\
           fn on_t(p: P) { self.n = p.n; } }\n\
         locus Q { bus { publish T; } fn go() { T <- P { n: 1 }; } }\n\
         main locus App { params { r: R = R { }; q: Q = Q { }; } }\n\
         fn main() { App { }; }",
    );
    assert!(
        !ds.iter().any(|m| m.contains("may not")),
        "the restraints must not leak onto ordinary loci: {:?}",
        ds
    );
}
