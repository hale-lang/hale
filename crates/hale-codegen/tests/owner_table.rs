//! The ownership pre-pass — GH #921 A2, read by lowering since A3.
//!
//! Two halves, both about `hale_codegen::ownership`:
//!
//!   1. **derivations** — small programs, one per syntactic position
//!      F.39 names, asserting the owner the table gives the
//!      locus-producing expression there. These are the contract
//!      lowering reads, so they are stated positively and not as
//!      "whatever lowering happens to do". They seed the
//!      fresh-factory fixpoint EMPTY on purpose, so a derivation
//!      never passes because `compute_fresh_locus_factories`
//!      happened to agree.
//!   2. **the build** — the same programs compiled. Reaching a locus
//!      instantiation the table has no row for is a `CodegenError`
//!      (F.39's rule, A3 commit 7), so a build that succeeds is the
//!      statement that every locus in the program was decided before
//!      lowering began. A4's four `KNOWN_OPEN` families were pinned
//!      here as shadow-mode DISAGREEMENTS and were A3's checklist;
//!      all four are closed, and each is a shape here naming the
//!      commit that closed it.
//!
//! Every program is assembled from ordinary `"…"` constants, never a
//! RAW string literal, because `hale_corpus::embedded` harvests raw
//! literals that look like a program out of every Rust file under a
//! `tests` directory and these twenty permutations are not a corpus.

use std::collections::BTreeMap;

use hale_codegen::ownership::{
    resolve_owners, Entry, ExprId, Owner, OwnerTable, ScopeKind,
};

#[path = "support/harness.rs"]
mod harness;

// ===================================================================
// Fixtures
// ===================================================================

/// The subject and the helpers every program leans on. `Subj` is a
/// plain locus; `Holder` holds one behind a locus-typed field and
/// `IHolder` behind an interface-typed one; `make` / `make2` are
/// proven-fresh factories.
const DECLS: &str = "
locus Subj {
    params { n: Int = 0; }
    dissolve() { println(\"D:subj\"); }
    fn probe() -> Int { return self.n + 1; }
}

interface Probe { fn probe() -> Int; }

locus Cfg {
    dissolve() { println(\"D:cfg\"); }
    fn seed() -> Int { return 1; }
}

locus Holder {
    params { c: Subj = Subj { n: 1 }; }
    fn peek() -> Int { return self.c.probe(); }
}

locus IHolder {
    params { c: Probe = Subj { n: 1 }; }
    fn peek() -> Int { return self.c.probe(); }
}

locus DHolder {
    params { c: Subj = make(1); }
    fn peek() -> Int { return self.c.probe(); }
}

fn make(n: Int) -> Subj { return Subj { n: n }; }

fn make2(n: Int) -> Subj { return Subj { n: n }; }

fn make_f(n: Int) -> Subj fallible(String) {
    if n < 0 { fail \"negative\"; }
    return Subj { n: n };
}

fn touch(t: Subj) -> Int { return t.probe(); }
";

/// Wrap payload statements in `fn main`.
fn program(payload: &str) -> String {
    [DECLS, "\nfn main() {\n", payload, "\n    println(\"end\");\n}\n"]
        .concat()
}

/// Wrap payload statements in a `while` body inside `fn main`, so the
/// innermost reclaim scope is one iteration.
fn program_in_loop(payload: &str) -> String {
    [
        DECLS,
        "\nfn main() {\n    let mut i = 0;\n    while i < 2 {\n",
        payload,
        "\n        i = i + 1;\n    }\n    println(\"end\");\n}\n",
    ]
    .concat()
}

fn table_of(src: &str) -> OwnerTable {
    let mut p = hale_syntax::parse_source(src)
        .unwrap_or_else(|e| panic!("the fixture does not parse: {e:?}\n{src}"));
    // An EMPTY seed on purpose: the table's own fixpoint has to find
    // the ordinary factories, so a derivation test never passes
    // because `compute_fresh_locus_factories` happened to agree.
    let seed: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    resolve_owners(&mut p, &seed, &[])
}

/// The one row written in `decl` at `position` naming `name`.
fn row<'t>(
    t: &'t OwnerTable,
    decl: &str,
    position: &str,
    name: &str,
) -> &'t Entry {
    let hits: Vec<&Entry> = t
        .rows()
        .map(|(_, e)| e)
        .filter(|e| e.decl == decl && e.position == position && e.name == name)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one `{name}` at `{position}` in `{decl}`; the \
         table has {}:\n{}",
        hits.len(),
        dump(t)
    );
    hits[0]
}

/// Every row written in `decl` at `position` naming `name`, in id
/// order — for the positions that decide more than one expression
/// (a carrier's arms, a composite's elements, an `or`'s branches).
fn rows<'t>(
    t: &'t OwnerTable,
    decl: &str,
    position: &str,
) -> Vec<&'t Entry> {
    t.rows()
        .map(|(_, e)| e)
        .filter(|e| e.decl == decl && e.position == position)
        .collect()
}

fn dump(t: &OwnerTable) -> String {
    let mut s = String::new();
    for (id, e) in t.rows() {
        s.push_str(&format!(
            "  #{} {:?} {} `{}` in `{}` ({})\n",
            id.0, e.owner, e.what, e.name, e.decl, e.position
        ));
    }
    for ((l, f), e) in t.borrowed_rows() {
        s.push_str(&format!("  borrowed {l}.{f}: {:?}\n", e.owner));
    }
    s
}

fn is_frame_temp(t: &OwnerTable, e: &Entry, want: ScopeKind) -> bool {
    match &e.owner {
        Owner::FrameTemp(s) => t.scope_kind(*s) == want,
        _ => false,
    }
}

// ===================================================================
// 1 — derivations, one per position
// ===================================================================

#[test]
fn a_let_of_a_literal_is_owned_by_the_binding() {
    let t = table_of(&program("    let a = Subj { n: 1 };\n    println(\"u=\", a.probe());"));
    let e = row(&t, "fn main", "`let` RHS", "Subj");
    assert!(matches!(e.owner, Owner::Binding(_)), "{:?}", e.owner);
}

#[test]
fn a_let_of_a_factory_call_is_owned_by_the_binding() {
    let t = table_of(&program("    let a = make(1);\n    println(\"u=\", a.probe());"));
    let e = row(&t, "fn main", "`let` RHS", "Subj");
    assert!(matches!(e.owner, Owner::Binding(_)), "{:?}", e.owner);
}

#[test]
fn an_assignment_to_a_local_is_owned_by_that_slot() {
    let t = table_of(&program(
        "    let mut a = make(1);\n    a = make2(2);\n    println(\"u=\", a.probe());",
    ));
    let e = row(&t, "fn main", "assignment RHS", "Subj");
    assert!(matches!(e.owner, Owner::Binding(_)), "{:?}", e.owner);
}

#[test]
fn a_bare_statement_literal_is_a_frame_temporary() {
    let t = table_of(&program("    Subj { n: 1 };"));
    let e = row(&t, "fn main", "bare statement", "Subj");
    assert!(is_frame_temp(&t, e, ScopeKind::Frame), "{:?}", e.owner);
}

#[test]
fn a_receiver_literal_is_a_frame_temporary() {
    let t = table_of(&program("    println(\"u=\", Subj { n: 1 }.probe());"));
    let e = row(&t, "fn main", "call receiver", "Subj");
    assert!(is_frame_temp(&t, e, ScopeKind::Frame), "{:?}", e.owner);
}

#[test]
fn an_argument_literal_is_a_frame_temporary() {
    let t = table_of(&program("    println(\"u=\", touch(Subj { n: 1 }));"));
    let e = row(&t, "fn main", "argument", "Subj");
    assert!(is_frame_temp(&t, e, ScopeKind::Frame), "{:?}", e.owner);
}

#[test]
fn a_field_read_receiver_is_a_frame_temporary() {
    let t = table_of(&program("    println(\"u=\", Subj { n: 1 }.n + 1);"));
    let e = row(&t, "fn main", "field read", "Subj");
    assert!(is_frame_temp(&t, e, ScopeKind::Frame), "{:?}", e.owner);
}

#[test]
fn a_returned_literal_is_the_callers() {
    let src = [
        DECLS,
        "\nfn produce() -> Subj { return Subj { n: 1 }; }\n",
        "fn main() { let a = produce(); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    let t = table_of(&src);
    let e = row(&t, "fn produce", "`return`", "Subj");
    assert_eq!(e.owner, Owner::Caller);
}

#[test]
fn a_returned_factory_call_is_the_callers() {
    let src = [
        DECLS,
        "\nfn produce() -> Subj { return make(1); }\n",
        "fn main() { let a = produce(); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    let t = table_of(&src);
    let e = row(&t, "fn produce", "`return`", "Subj");
    assert_eq!(e.owner, Owner::Caller);
}

#[test]
fn every_arm_of_a_returned_if_is_the_callers() {
    let src = [
        DECLS,
        "\nfn produce(c: Bool) -> Subj {\n",
        "    return if c { make(1) } else { make2(1) };\n}\n",
        "fn main() { let a = produce(true); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    let t = table_of(&src);
    let arms = rows(&t, "fn produce", "`return`");
    assert_eq!(arms.len(), 2, "{}", dump(&t));
    for a in arms {
        assert_eq!(a.owner, Owner::Caller, "{}", dump(&t));
    }
}

#[test]
fn every_arm_of_a_returned_match_is_the_callers() {
    let src = [
        DECLS,
        "\nfn produce(k: Int) -> Subj {\n",
        "    return match k { 0 -> make(1), _ -> make2(1) };\n}\n",
        "fn main() { let a = produce(0); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    let t = table_of(&src);
    let arms = rows(&t, "fn produce", "`match` arm");
    assert_eq!(arms.len(), 2, "{}", dump(&t));
    for a in arms {
        assert_eq!(a.owner, Owner::Caller, "{}", dump(&t));
    }
}

#[test]
fn a_returned_block_tail_is_the_callers() {
    let src = [
        DECLS,
        "\nfn produce() -> Subj { return { make(1) }; }\n",
        "fn main() { let a = produce(); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    let t = table_of(&src);
    let e = row(&t, "fn produce", "block tail", "Subj");
    assert_eq!(e.owner, Owner::Caller);
}

#[test]
fn a_param_field_literal_transfers_into_the_field() {
    let t = table_of(&program(
        "    let h = Holder { c: Subj { n: 1 } };\n    println(\"u=\", h.peek());",
    ));
    let e = row(&t, "fn main", "param-field initialiser", "Subj");
    assert!(
        matches!(&e.owner, Owner::Field { field, .. } if field == "c"),
        "{:?}",
        e.owner
    );
}

#[test]
fn a_param_field_factory_transfers_into_the_field() {
    let t = table_of(&program(
        "    let h = Holder { c: make(1) };\n    println(\"u=\", h.peek());",
    ));
    let e = row(&t, "fn main", "param-field initialiser", "Subj");
    assert!(
        matches!(&e.owner, Owner::Field { field, .. } if field == "c"),
        "{:?}",
        e.owner
    );
}

#[test]
fn both_branches_of_an_or_field_initialiser_transfer_into_the_field() {
    let t = table_of(&program(
        "    let h = Holder { c: make_f(1) or make2(1) };\n    println(\"u=\", h.peek());",
    ));
    let ok = row(&t, "fn main", "param-field initialiser", "Subj");
    let sub = row(&t, "fn main", "`or` substitute", "Subj");
    for e in [ok, sub] {
        assert!(
            matches!(&e.owner, Owner::Field { field, .. } if field == "c"),
            "{:?} — {}",
            e.owner,
            dump(&t)
        );
    }
}

#[test]
fn an_or_raise_field_initialiser_transfers_into_the_field() {
    let src = [
        DECLS,
        "\nfn mk() -> Holder fallible(String) {\n",
        "    let h = Holder { c: make_f(1) or raise };\n    return h;\n}\n",
        "fn main() { let h = mk() or raise; println(\"u=\", h.peek()); }\n",
    ]
    .concat();
    let t = table_of(&src);
    let e = row(&t, "fn mk", "param-field initialiser", "Subj");
    assert!(
        matches!(&e.owner, Owner::Field { field, .. } if field == "c"),
        "{:?}",
        e.owner
    );
}

#[test]
fn a_param_field_named_by_a_handle_is_borrowed() {
    let t = table_of(&program(
        "    let t = Subj { n: 1 };\n    let h = Holder { c: t };\n    println(\"u=\", h.peek());",
    ));
    let e = t
        .borrowed_entry("Holder", "c")
        .unwrap_or_else(|| panic!("no borrowed row:\n{}", dump(&t)));
    assert!(matches!(e.owner, Owner::Borrowed(_)), "{:?}", e.owner);
}

#[test]
fn a_params_block_default_is_owned_by_the_declaration() {
    let t = table_of(&program("    let h = DHolder { };\n    println(\"u=\", h.peek());"));
    let e = row(&t, "DHolder.params.c", "param-field initialiser", "Subj");
    assert_eq!(
        e.owner,
        Owner::Field { owner: ExprId::DECLARED, field: "c".to_string() }
    );
}

#[test]
fn a_carrier_let_rhs_is_a_frame_temporary_of_the_frame() {
    let t = table_of(&program(
        "    let c = true;\n    let a = if c { make(1) } else { make2(1) };\n    println(\"u=\", a.probe());",
    ));
    let arms = rows(&t, "fn main", "`let` RHS");
    assert_eq!(arms.len(), 2, "{}", dump(&t));
    for e in arms {
        assert!(
            is_frame_temp(&t, e, ScopeKind::Frame),
            "{:?} — {}",
            e.owner,
            dump(&t)
        );
    }
}

#[test]
fn the_same_carrier_inside_a_loop_is_a_per_iteration_temporary() {
    let t = table_of(&program_in_loop(
        "        let c = true;\n        let a = if c { make(1) } else { make2(1) };\n        println(\"u=\", a.probe());",
    ));
    let arms = rows(&t, "fn main", "`let` RHS");
    assert_eq!(arms.len(), 2, "{}", dump(&t));
    for e in arms {
        assert!(
            is_frame_temp(&t, e, ScopeKind::LoopIteration),
            "{:?} — {}",
            e.owner,
            dump(&t)
        );
    }
}

#[test]
fn a_composite_decides_each_element_separately() {
    let t = table_of(&program(
        "    let xs: [Subj; 2] = [make(1), make2(1)];\n    println(\"u=\", xs[0].probe());",
    ));
    let els = rows(&t, "fn main", "composite element");
    assert_eq!(els.len(), 2, "{}", dump(&t));
    for e in els {
        assert!(is_frame_temp(&t, e, ScopeKind::Frame), "{:?}", e.owner);
    }
}

#[test]
fn a_receiver_inside_a_field_initialiser_is_not_the_fields() {
    // GH #896: the flags hand this receiver the decision meant for
    // the field's value.
    let t = table_of(&program(
        "    let h = Holder { c: make(Cfg { }.seed()) };\n    println(\"u=\", h.peek());",
    ));
    let recv = row(&t, "fn main", "call receiver", "Cfg");
    assert!(
        is_frame_temp(&t, recv, ScopeKind::Frame),
        "the receiver is a temporary of the frame, not the field's: {:?}",
        recv.owner
    );
    let value = row(&t, "fn main", "param-field initialiser", "Subj");
    assert!(
        matches!(&value.owner, Owner::Field { field, .. } if field == "c"),
        "{:?}",
        value.owner
    );
}

/// The declarations behind the `perspective(P)`-typed field shape —
/// `ownership_matrix.rs`'s `persp_field_factory` position, added by
/// GH #921 A3 because no cell covered it.
const PERSP_DECLS: &str = "
perspective PRoute { fn pv() -> Int; }

locus PRouteV1 : serves PRoute {
    params { n: Int = 1; }
    dissolve() { println(\"D:proute\"); }
    fn pv() -> Int { return self.n; }
}

locus PHolder {
    params { r: perspective(PRoute) = PRouteV1 { }; }
    fn peek() -> Int { return self.r.pv(); }
}

fn makep() -> PRouteV1 { return PRouteV1 { }; }
";

#[test]
fn a_factory_into_a_perspective_field_is_the_fields() {
    // F.39 says a param field's initialiser is `Owner::Field` whether
    // the field is locus-, interface- or perspective-typed. Lowering
    // used to disagree for the third: the F.17 gate covered
    // `LocusRef` and `Interface` only, so the value took the GH #402
    // frame temporary and the owner's mask bit deliberately did not
    // claim it. GH #921 A3 commit 1 retires that gate with
    // `suppress_fresh_temp`, so the bit has to claim it.
    let src = [
        PERSP_DECLS,
        "\nfn main() {\n    let h = PHolder { r: makep() };\n",
        "    println(\"u=\", h.peek());\n}\n",
    ]
    .concat();
    let t = table_of(&src);
    let e = row(&t, "fn main", "param-field initialiser", "PRouteV1");
    assert!(
        matches!(&e.owner, Owner::Field { field, .. } if field == "r"),
        "{:?}",
        e.owner
    );
}

#[test]
fn every_locus_is_decided_in_a_factory_into_a_perspective_field() {
    let src = [
        PERSP_DECLS,
        "\nfn main() {\n    let h = PHolder { r: makep() };\n",
        "    println(\"u=\", h.peek());\n}\n",
    ]
    .concat();
    agrees(&src, "persp_field_factory");
}

#[test]
fn a_placed_field_is_a_placement_entry() {
    let src = concat!(
        "locus Worker {\n",
        "    params { n: Int = 0; }\n",
        "    run() { println(\"w\"); }\n",
        "}\n",
        "main locus App {\n",
        "    params { w: Worker = Worker { n: 1 }; }\n",
        "    placement { w: pinned; }\n",
        "    run() { println(\"app\"); }\n",
        "}\n",
    );
    let t = table_of(src);
    let e = row(&t, "App.params.w", "param-field initialiser", "Worker");
    assert_eq!(e.owner, Owner::Placement("w".to_string()));
}

#[test]
fn a_carrier_return_fn_is_a_proven_fresh_factory_in_the_table() {
    // The whole of the 105-cell carrier-return family.
    // `compute_fresh_locus_factories::collect` classifies the CARRIER
    // node and never its arms, so `produce` was not a factory and its
    // caller's binding did not own the result. The table flattens the
    // arms, and GH #921 A3 commit 1 folds its answer back into the
    // map lowering reads — the seed here stays EMPTY so this asserts
    // the table's own fixpoint and not the fold-back.
    let src = [
        DECLS,
        "\nfn produce(c: Bool) -> Subj {\n",
        "    return if c { make(1) } else { make2(1) };\n}\n",
        "fn main() { let a = produce(true); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    let t = table_of(&src);
    assert_eq!(t.extended_fresh_factory("produce"), Some("Subj"));
    let e = row(&t, "fn main", "`let` RHS", "Subj");
    assert!(matches!(e.owner, Owner::Binding(_)), "{:?}", e.owner);
}

#[test]
fn a_binding_the_frame_hands_back_is_the_callers() {
    // `binding_escapes_this_frame`'s carve-out, stated on the table
    // side: the `let` names it but the caller reclaims it.
    let src = [
        DECLS,
        "\nfn produce(c: Bool) -> Subj {\n",
        "    let t = if c { make(1) } else { make2(1) };\n    return t;\n}\n",
        "fn main() { let a = produce(true); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    let t = table_of(&src);
    let arms = rows(&t, "fn produce", "`let` RHS");
    assert_eq!(arms.len(), 2, "{}", dump(&t));
    for e in arms {
        assert_eq!(e.owner, Owner::Caller, "{}", dump(&t));
    }
}

#[test]
fn the_table_numbers_every_locus_producing_node_it_decides() {
    let t = table_of(&program(
        "    let a = Subj { n: 1 };\n    let b = make(1);\n    println(\"u=\", a.probe() + b.probe());",
    ));
    assert!(t.numbered() >= t.len() as u32);
    assert!(!t.is_empty());
    for (id, _) in t.rows() {
        assert_ne!(
            *id,
            ExprId::DECLARED,
            "a real row must not carry the declaration sentinel"
        );
    }
}

// ===================================================================
// 2 — the build
// ===================================================================

/// Build the program and hand back the refusal, if any.
fn build(src: &str, tag: &str) -> Option<String> {
    let p = hale_syntax::parse_source(src)
        .unwrap_or_else(|e| panic!("{tag}: does not parse: {e:?}\n{src}"));
    let bin = harness::unique_bin(&["ownertab_", tag].concat());
    let r = hale_codegen::build_executable(&p, &bin);
    let _ = std::fs::remove_file(&bin);
    match r {
        Ok(()) => None,
        Err(e) => Some(format!("{e:?}")),
    }
}

/// Every locus in this program is decided before lowering begins.
/// A row the table does not have is a `CodegenError` naming the
/// expression (F.39's rule, GH #921 A3 commit 7), so a clean build
/// IS the assertion.
fn agrees(src: &str, tag: &str) {
    if let Some(e) = build(src, tag) {
        panic!(
            "{tag}: this shape must build — a locus the owner table \
             has no row for is refused:\n{e}"
        );
    }
}

#[test]
fn every_locus_is_decided_in_the_shapes_the_matrix_is_green_on() {
    for (tag, payload) in [
        ("let_literal", "    let a = Subj { n: 1 };\n    println(\"u=\", a.probe());"),
        ("bare_stmt", "    Subj { n: 1 };"),
        ("receiver", "    println(\"u=\", Subj { n: 1 }.probe());"),
        ("arg", "    println(\"u=\", touch(Subj { n: 1 }));"),
        ("field_read", "    println(\"u=\", Subj { n: 1 }.n + 1);"),
        ("let_factory", "    let a = make(1);\n    println(\"u=\", a.probe());"),
        (
            "field_literal",
            "    let h = Holder { c: Subj { n: 1 } };\n    println(\"u=\", h.peek());",
        ),
        (
            "field_factory",
            "    let h = Holder { c: make(1) };\n    println(\"u=\", h.peek());",
        ),
        (
            "field_or_call",
            "    let h = Holder { c: make_f(1) or make2(1) };\n    println(\"u=\", h.peek());",
        ),
        ("field_default_factory", "    let h = DHolder { };\n    println(\"u=\", h.peek());"),
        (
            "iface_field_literal",
            "    let h = IHolder { c: Subj { n: 1 } };\n    println(\"u=\", h.peek());",
        ),
        (
            "iface_field_factory",
            "    let h = IHolder { c: make(1) };\n    println(\"u=\", h.peek());",
        ),
        (
            "carrier_let_outside_a_loop",
            "    let c = true;\n    let a = if c { make(1) } else { make2(1) };\n    println(\"u=\", a.probe());",
        ),
        (
            "composite_outside_a_loop",
            "    let xs: [Subj; 2] = [make(1), make2(1)];\n    println(\"u=\", xs[0].probe());",
        ),
        (
            "literal_in_a_loop",
            "    let mut i = 0;\n    while i < 2 {\n        let a = Subj { n: 1 };\n        println(\"u=\", a.probe());\n        i = i + 1;\n    }",
        ),
    ] {
        agrees(&program(payload), tag);
    }
}

/// GH #921 A3, commit 1 closed these: the carrier-return family
/// (`ownership_matrix.rs`'s 105 cells) and the per-iteration frame
/// temporary (its 25). Both were `disagrees` pins until the commit
/// that retired `suppress_fresh_temp`; they are green shapes now, and
/// the matrix holds their cells to it.
#[test]
fn every_locus_is_decided_in_the_families_commit_one_closed() {
    let carrier = [
        DECLS,
        "\nfn produce(c: Bool) -> Subj {\n",
        "    return if c { make(1) } else { make2(1) };\n}\n",
        "fn main() { let a = produce(true); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    agrees(&carrier, "carrier_return");

    // The `let`-named twin of the same program — the shape the
    // matrix's differential oracle compares against, and the one the
    // fixpoint had to learn to resolve through a binding.
    let carrier_twin = [
        DECLS,
        "\nfn produce(c: Bool) -> Subj {\n",
        "    let t = if c { make(1) } else { make2(1) };\n    return t;\n}\n",
        "fn main() { let a = produce(true); println(\"u=\", a.probe()); }\n",
    ]
    .concat();
    agrees(&carrier_twin, "carrier_return_twin");

    agrees(
        &program_in_loop(
            "        let c = true;\n        let a = if c { make(1) } else { make2(1) };\n        println(\"u=\", a.probe());",
        ),
        "frame_temp_per_iteration",
    );
    agrees(
        &program_in_loop(
            "        let xs: [Subj; 2] = [make(1), make2(1)];\n        println(\"u=\", xs[0].probe());",
        ),
        "composite_per_iteration",
    );
}

/// GH #896, folded into #921 — `ownership_matrix.rs`'s
/// `NESTED_RECEIVER_IN_FIELD_INIT`, 35 cells, closed by commit 3.
/// The receiver literal inside a locus-typed field's non-literal
/// initialiser took the parent-owned flag meant for the field's
/// value, so the frame that built it stood back and nobody reclaimed
/// it. `parent_owns_via_field` is the table's `Owner::Field` on the
/// node itself now, and the table decided the receiver and the
/// field's value separately.
#[test]
fn every_locus_is_decided_in_the_family_commit_three_closed() {
    agrees(
        &program(
            "    let h = Holder { c: make(Cfg { }.seed()) };\n    println(\"u=\", h.peek());",
        ),
        "nested_receiver",
    );
}

#[test]
fn every_locus_is_decided_in_a_returned_literal_and_a_returned_factory() {
    for (tag, produce) in [
        ("return_literal", "fn produce() -> Subj { return Subj { n: 1 }; }\n"),
        ("return_factory", "fn produce() -> Subj { return make(1); }\n"),
    ] {
        let src = [
            DECLS,
            "\n",
            produce,
            "fn main() { let a = produce(); println(\"u=\", a.probe()); }\n",
        ]
        .concat();
        agrees(&src, tag);
    }
}

/// GH #921 A3, PR #916's residue — `ownership_matrix.rs`'s
/// `OR_INTO_INTERFACE_FIELD`, 35 cells, closed by commit 4 and the
/// last family on the board. The ok value and the substitute both
/// belong to the field; the three field-ownership predicates
/// compared the factory's DECLARED locus with the FIELD's, which an
/// interface-typed field does not have, so neither branch got the
/// owner's mask bit and the frame had already stood back (F.17).
#[test]
fn every_locus_is_decided_in_the_family_commit_four_closed() {
    agrees(
        &program(
            "    let h = IHolder { c: make_f(1) or make2(1) };\n    println(\"u=\", h.peek());",
        ),
        "or_into_iface_field",
    );
}
