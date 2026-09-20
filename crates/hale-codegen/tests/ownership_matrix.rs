//! The ownership shape matrix — GH #921 A4.
//!
//! ## Why a generator
//!
//! Every teardown leak and use-after-free the 2026-09 sweep fixed was
//! the same defect wearing a different outfit: locus ownership is
//! decided by one-shot lowering flags (`suppress_fresh_temp`,
//! `defer_next_locus_dissolve`, `instantiating_for_parent_field`,
//! `placement_for_next_locus_instantiation`, `or_field_owner_locus`,
//! `returns_this_locus`) and a flag was taken by the wrong node. Each
//! fix arrived with hand-written cases for the shape that was
//! reported, which is exactly the coverage shape that lets the next
//! outfit through: nobody writes the case for a combination nobody
//! has hit yet.
//!
//! So this file does not contain cases. It contains three axes and a
//! renderer:
//!
//!   * **position** — where the locus-producing expression is
//!     written (27 of them: `let`, bare statement, receiver,
//!     argument retained / dropped, field read, `return` of a
//!     literal / factory / carrier, param-field literal / factory /
//!     `or raise` / `or <literal>` / `or <call>` / default / nested
//!     receiver, the same against an INTERFACE-typed field and a
//!     factory into a `perspective(P)`-typed one, `if` / `match` /
//!     block tails, array and tuple elements);
//!   * **type** — the shape of the tree being instantiated (plain,
//!     a `@form(vec)` child, a grandchild, an interface-typed child,
//!     a `perspective(P)` child);
//!   * **context** — the frame the payload runs in (`fn main`, a
//!     free fn, a locus METHOD, a `while` loop of four iterations, a
//!     fn inside `module { }`, and an early-`return` guard both
//!     taken and not taken).
//!
//! The context matters as much as the position: `main`'s arena is
//! destroyed at process exit, so a leak there is invisible to a
//! sanitizer and `main` frames pass vacuously (PR #835's note). A
//! method frame is where the same defect is measurable.
//!
//! ## The four oracles
//!
//! Each cell is compiled and run under all four:
//!
//!   1. **tags** — every locus in the generated tree prints a line
//!      from its `dissolve()`, and the count per tag must equal the
//!      number of instantiations exactly. Too few is a leak; too
//!      many is a double teardown. This is the only oracle that
//!      sees a *missing* reclaim in `main`.
//!   2. **residency** — `LOTUS_ARENA_RESIDENCY=1` walks the
//!      runtime's registry of live top-level arenas at exit and
//!      reports them on an ordinary build; the matrix requires `0
//!      live arenas`.
//!   3. **ASan** — `harness::build_asan` (`BuildOptions::asan`,
//!      GH #843), whose runtime cflags carry
//!      `-DLOTUS_NO_CHUNK_POOL_DEFAULT=1` (GH #816, PR #875) so a
//!      recycled chunk's intact bytes cannot hide a
//!      use-after-free; `LOTUS_NO_CHUNK_POOL=1` is restated on the
//!      child so the oracle does not depend on that default.
//!   4. **differential** — for every position that has an inline
//!      spelling, the `let`-named spelling of the same program is
//!      generated too, and the two must print identical stdout.
//!      `spec/semantics.md` § "Dissolve timing rules" promises
//!      exactly that: "Naming the literal with `let` first and
//!      using it inline are the same program, in every position."
//!      A position where the spec says the two spellings are
//!      *different* programs — a bare statement literal, or a field
//!      initialiser, where a handle written inline is a transfer
//!      and a named one is borrowed (F.29) — declares no twin.
//!
//! A parse or check refusal is a generator bug and fails loudly; a
//! build refusal is recorded as an oracle failure so a cell can
//! name it.
//!
//! ## Expected failures
//!
//! [`KNOWN_OPEN`] names the cells that fail today, each with the
//! family it belongs to. They are **asserted to fail**, not skipped:
//! the matrix is green with them listed, and the day a fix closes
//! one, its cell goes green and this file goes red until the entry
//! is deleted. That is the regression test. Nothing here is fixed by
//! this file; test infrastructure only.
//!
//! A4 landed with 200 open cells in four families. GH #921 A3
//! closes them one commit at a time, and each commit deletes its
//! block.
//!
//! ## Size
//!
//! The full matrix (27 × 5 × 7 = 945 cells, ~100 s on seven threads)
//! is behind `HALE_MATRIX=full`, for a nightly job. The per-PR
//! default runs a deterministic sample (~90 cells, ~17 s) that
//! always contains one
//! representative cell per open POSITION and that cell's six axis
//! neighbours, at least one cell per position, per type and per
//! context, and is topped up by a fixed co-prime stride to
//! [`TARGET_SAMPLE`]. Per open position rather than per open cell
//! because a family fails for every type and, bar one, every
//! context.
//!
//! ## Corpus note
//!
//! The generated programs are assembled from ordinary `"…"` string
//! constants and `concat`, never a RAW string literal, because
//! `hale_corpus::embedded` harvests raw string literals that look
//! like a program out of every Rust file under a `tests` directory.
//! Hundreds of synthetic permutations are not a corpus — so this
//! file contains no raw-literal opener at all, not even in prose.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

// ===================================================================
// Expected failures
// ===================================================================

// The defect families still open. A cell's KNOWN_OPEN entry names
// the family it belongs to, so the Phase A commit that closes one
// deletes one block.

/// GH #921 A3, PR #916's residue. `or_field_owner_locus` compares
/// the factory's declared locus with the FIELD's, which an
/// interface-typed field does not have; PR #910 closed the
/// `LocusRef` twin only. The bare factory into the same field was
/// closed by GH #895, which is why `iface_field_factory` is green
/// beside this one.
const OR_INTO_INTERFACE_FIELD: &str =
    "`or <call>` into an INTERFACE-typed field leaves the ok value \
     unowned — the LocusRef twin was closed by PR #910, the contract \
     one was not (GH #921 A3, PR #916's residue)";

/// Cells that fail on `main` today, each naming its family. Every
/// entry is ASSERTED to fail — see the module docs.
///
/// The cell id is `<position>/<type>/<context>`.
const KNOWN_OPEN: &[(&str, &str)] = &[
    // `or <call>` into an INTERFACE-typed field (GH #921 A3, PR #916's residue).
    ("iface_field_or_call/plain/main", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/plain/free_fn", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/plain/method", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/plain/loop", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/plain/module", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/plain/guard_taken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/plain/guard_untaken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/vec_child/main", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/vec_child/free_fn", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/vec_child/method", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/vec_child/loop", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/vec_child/module", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/vec_child/guard_taken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/vec_child/guard_untaken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/grandchild/main", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/grandchild/free_fn", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/grandchild/method", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/grandchild/loop", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/grandchild/module", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/grandchild/guard_taken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/grandchild/guard_untaken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/iface_child/main", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/iface_child/free_fn", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/iface_child/method", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/iface_child/loop", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/iface_child/module", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/iface_child/guard_taken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/iface_child/guard_untaken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/persp_child/main", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/persp_child/free_fn", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/persp_child/method", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/persp_child/loop", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/persp_child/module", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/persp_child/guard_taken", OR_INTO_INTERFACE_FIELD),
    ("iface_field_or_call/persp_child/guard_untaken", OR_INTO_INTERFACE_FIELD),

    // GH #896.

    // A frame temporary in a `while` body. Found by this matrix; not filed.
];

/// How many cells the per-PR sample aims for.
const TARGET_SAMPLE: usize = 60;

/// A stride co-prime with the cell count (26 · 5 · 7 = 910 = 2·5·7·13),
/// so topping the sample up visits the space evenly and
/// deterministically.
const FILL_STRIDE: usize = 101;

/// Generated programs answer in milliseconds; the deadline exists so
/// a teardown defect that sends a program spinning fails the job
/// instead of wedging it.
const DEADLINE: Duration = Duration::from_secs(90);

// ===================================================================
// Axis 1 — the type (the shape of the tree being instantiated)
// ===================================================================

/// Every shape declares a locus called `Subj` with a `params` field
/// `n: Int` and a method `probe() -> Int` answering `n + 1`, so one
/// set of positions is written against all five.
struct Shape {
    id: &'static str,
    decls: &'static str,
    /// The `dissolve()` line each instantiation must print, once.
    tags: &'static [&'static str],
}

const SHAPE_PLAIN: &str = "
locus Subj {
    params { n: Int = 0; }
    dissolve() { println(\"D:subj\"); }
    fn probe() -> Int { return self.n + 1; }
}
";

/// A `@form(vec)` child: its buffer is a realloc outside the arena,
/// so a missed reclaim is bytes a sanitizer can see rather than only
/// a silent arena.
const SHAPE_VEC_CHILD: &str = "
type Row { v: Int = 0; }

@form(vec)
locus Rows {
    capacity { heap rows of Row; }
}

locus Subj {
    params { n: Int = 0; rows: Rows = Rows { }; }
    birth() { self.rows.push(Row { v: 3 }); }
    dissolve() { println(\"D:subj\"); }
    fn probe() -> Int { return self.n + self.rows.len(); }
}
";

/// Two levels under the subject, so the cascade has to reach past
/// the first (F.4 depth-first).
const SHAPE_GRANDCHILD: &str = "
locus Leaf {
    dissolve() { println(\"D:leaf\"); }
    fn v() -> Int { return 1; }
}

locus Mid {
    params { leaf: Leaf = Leaf { }; }
    dissolve() { println(\"D:mid\"); }
    fn v() -> Int { return self.leaf.v(); }
}

locus Subj {
    params { n: Int = 0; mid: Mid = Mid { }; }
    dissolve() { println(\"D:subj\"); }
    fn probe() -> Int { return self.n + self.mid.v(); }
}
";

/// A child held behind a CONTRACT, which the teardown cascade cannot
/// key on the field's declared type to find (GH #871 arm 1).
const SHAPE_IFACE_CHILD: &str = "
interface Inner { fn v() -> Int; }

locus Impl {
    dissolve() { println(\"D:impl\"); }
    fn v() -> Int { return 1; }
}

locus Subj {
    params { n: Int = 0; inner: Inner = Impl { }; }
    dissolve() { println(\"D:subj\"); }
    fn probe() -> Int { return self.n + self.inner.v(); }
}
";

/// A `perspective(P)` handle: the field holds the designated impl
/// for OWNERSHIP alone, since dispatch goes through the
/// program-global slot. Declarable without a `main locus`, as
/// `owned_child_arena_reclaim.rs` already relies on — so the brief's
/// "skip and say so" escape is not needed.
const SHAPE_PERSP_CHILD: &str = "
perspective Route { fn v() -> Int; }

locus RouteV1 : serves Route {
    dissolve() { println(\"D:routev1\"); }
    fn v() -> Int { return 1; }
}

locus Subj {
    params { n: Int = 0; r: perspective(Route) = RouteV1 { }; }
    dissolve() { println(\"D:subj\"); }
    fn probe() -> Int { return self.n + self.r.v(); }
}
";

const SHAPES: &[Shape] = &[
    Shape { id: "plain", decls: SHAPE_PLAIN, tags: &["D:subj"] },
    Shape { id: "vec_child", decls: SHAPE_VEC_CHILD, tags: &["D:subj"] },
    Shape {
        id: "grandchild",
        decls: SHAPE_GRANDCHILD,
        tags: &["D:subj", "D:mid", "D:leaf"],
    },
    Shape {
        id: "iface_child",
        decls: SHAPE_IFACE_CHILD,
        tags: &["D:subj", "D:impl"],
    },
    Shape {
        id: "persp_child",
        decls: SHAPE_PERSP_CHILD,
        tags: &["D:subj", "D:routev1"],
    },
];

/// Everything the positions lean on, shared by all five shapes.
///
/// Only `Subj` (and its own children) and `Cfg` print at dissolve:
/// the holders are deliberately silent so a cell's tag counts speak
/// about the subject alone.
const COMMON: &str = "
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

locus Keeper {
    params { c: Subj = Subj { n: 1 }; }
    fn churn() -> Int { return self.c.probe(); }
}

fn make(n: Int) -> Subj { return Subj { n: n }; }

fn make2(n: Int) -> Subj { return Subj { n: n }; }

fn make_f(n: Int) -> Subj fallible(String) {
    if n < 0 { fail \"negative\"; }
    return Subj { n: n };
}

fn touch(t: Subj) -> Int { return t.probe(); }

fn retain(t: Subj) -> Int {
    let k = Keeper { c: t };
    return k.churn();
}
";

// ===================================================================
// Axis 2 — the position
// ===================================================================

struct Position {
    id: &'static str,
    /// Top-level declarations this position needs (may be empty).
    decls: &'static str,
    /// Statements placed in the context's frame.
    stmts: &'static str,
    /// The `let`-named spelling, when the spec says the two
    /// spellings are the same program. `None` means no differential.
    twin_decls: Option<&'static str>,
    twin_stmts: Option<&'static str>,
    /// `Subj` instantiations per execution of the payload.
    instances: usize,
    /// Tags from loci that are not the subject, per execution.
    extra_tags: &'static [(&'static str, usize)],
    /// The payload writes `or raise`, so its frame must be fallible.
    needs_fallible: bool,
}

/// A position with no twin, one instance, no extra tags and a
/// non-fallible frame — the common case, spelled out because
/// functional record update is not available in a `const`.
const fn plain_position(
    id: &'static str,
    decls: &'static str,
    stmts: &'static str,
) -> Position {
    Position {
        id,
        decls,
        stmts,
        twin_decls: None,
        twin_stmts: None,
        instances: 1,
        extra_tags: &[],
        needs_fallible: false,
    }
}

/// A position whose inline spelling has a `let`-named twin the spec
/// calls the same program, with the payload statements differing and
/// the declarations shared.
const fn twinned_position(
    id: &'static str,
    stmts: &'static str,
    twin_stmts: &'static str,
) -> Position {
    Position {
        id,
        decls: "",
        stmts,
        twin_decls: None,
        twin_stmts: Some(twin_stmts),
        instances: 1,
        extra_tags: &[],
        needs_fallible: false,
    }
}

/// A `return` position: the returning fn is a free fn (a locus
/// METHOD may not return a locus at all), and the twin names the
/// returned value with `let` first.
const fn returning_position(
    id: &'static str,
    decls: &'static str,
    twin_decls: &'static str,
    stmts: &'static str,
) -> Position {
    Position {
        id,
        decls,
        stmts,
        twin_decls: Some(twin_decls),
        twin_stmts: Some(stmts),
        instances: 1,
        extra_tags: &[],
        needs_fallible: false,
    }
}

const POSITIONS: &[Position] = &[
    // --- the two anchors -------------------------------------
    plain_position(
        "let_literal",
        "",
        "let a = Subj { n: 1 };\nprintln(\"u=\", a.probe());",
    ),
    // A literal whose value is DISCARDED keeps its fire-and-forget
    // teardown at the statement boundary, so it has no `let` twin
    // by design.
    plain_position(
        "bare_stmt",
        "",
        "Subj { n: 1 };\nprintln(\"u=\", 2);",
    ),
    // --- expression positions (GH #710 / #711 / #812) --------
    twinned_position(
        "receiver",
        "println(\"u=\", Subj { n: 1 }.probe());",
        "let a = Subj { n: 1 };\nprintln(\"u=\", a.probe());",
    ),
    twinned_position(
        "arg_retained",
        "println(\"u=\", retain(Subj { n: 1 }));",
        "let a = Subj { n: 1 };\nprintln(\"u=\", retain(a));",
    ),
    twinned_position(
        "arg_dropped",
        "println(\"u=\", touch(Subj { n: 1 }));",
        "let a = Subj { n: 1 };\nprintln(\"u=\", touch(a));",
    ),
    twinned_position(
        "field_read",
        "println(\"u=\", Subj { n: 1 }.n + 1);",
        "let a = Subj { n: 1 };\nprintln(\"u=\", a.n + 1);",
    ),
    // --- carriers bound by a `let` (GH #883) -----------------
    //
    // These are already the `let` spelling, so they carry no twin.
    plain_position(
        "let_if_tail",
        "",
        "let c = true;\nlet a = if c { make(1) } else { make2(1) };\nprintln(\"u=\", a.probe());",
    ),
    plain_position(
        "let_match_tail",
        "",
        "let k = 0;\nlet a = match k { 0 -> make(1), _ -> make2(1) };\nprintln(\"u=\", a.probe());",
    ),
    plain_position(
        "let_block_tail",
        "",
        "let a = { make(1) };\nprintln(\"u=\", a.probe());",
    ),
    // --- composites: the site names the aggregate ------------
    Position {
        id: "array_element",
        decls: "",
        stmts: "let xs: [Subj; 2] = [make(1), make2(1)];\nprintln(\"u=\", xs[0].probe());",
        twin_decls: None,
        twin_stmts: None,
        instances: 2,
        extra_tags: &[],
        needs_fallible: false,
    },
    Position {
        id: "tuple_element",
        decls: "",
        stmts: "let pr: (Subj, Subj) = (make(1), make2(1));\nprintln(\"u=\", pr.0.probe());",
        twin_decls: None,
        twin_stmts: None,
        instances: 2,
        extra_tags: &[],
        needs_fallible: false,
    },
    // --- `return`: the CALLER owns it ------------------------
    //
    // The context frame here is the one that CONSUMES the handle,
    // which is where an unowned return is measurable; the returning
    // fn is always a free fn, because a locus method may not return
    // a locus value at all.
    returning_position(
        "return_literal",
        "fn produce() -> Subj {\n    return Subj { n: 1 };\n}\n",
        "fn produce() -> Subj {\n    let t = Subj { n: 1 };\n    return t;\n}\n",
        "let a = produce();\nprintln(\"u=\", a.probe());",
    ),
    returning_position(
        "return_factory",
        "fn produce() -> Subj {\n    return make(1);\n}\n",
        "fn produce() -> Subj {\n    let t = make(1);\n    return t;\n}\n",
        "let a = produce();\nprintln(\"u=\", a.probe());",
    ),
    returning_position(
        "return_if_tail",
        "fn produce(c: Bool) -> Subj {\n    return if c { make(1) } else { make2(1) };\n}\n",
        "fn produce(c: Bool) -> Subj {\n    let t = if c { make(1) } else { make2(1) };\n    return t;\n}\n",
        "let a = produce(true);\nprintln(\"u=\", a.probe());",
    ),
    returning_position(
        "return_match_tail",
        "fn produce(k: Int) -> Subj {\n    return match k { 0 -> make(1), _ -> make2(1) };\n}\n",
        "fn produce(k: Int) -> Subj {\n    let t = match k { 0 -> make(1), _ -> make2(1) };\n    return t;\n}\n",
        "let a = produce(0);\nprintln(\"u=\", a.probe());",
    ),
    returning_position(
        "return_block_tail",
        "fn produce() -> Subj {\n    return { make(1) };\n}\n",
        "fn produce() -> Subj {\n    let t = { make(1) };\n    return t;\n}\n",
        "let a = produce();\nprintln(\"u=\", a.probe());",
    ),
    // --- a locus-typed param field: an ownership TRANSFER ----
    //
    // No twins: `Holder { c: Subj { } }` and `let t = Subj { };
    // Holder { c: t }` are different programs on purpose — the first
    // transfers, the second hands in a handle the binding still owns
    // (F.29).
    plain_position(
        "field_literal",
        "",
        "let h = Holder { c: Subj { n: 1 } };\nprintln(\"u=\", h.peek());",
    ),
    plain_position(
        "field_factory",
        "",
        "let h = Holder { c: make(1) };\nprintln(\"u=\", h.peek());",
    ),
    Position {
        id: "field_factory_raise",
        decls: "",
        stmts: "let h = Holder { c: make_f(1) or raise };\nprintln(\"u=\", h.peek());",
        twin_decls: None,
        twin_stmts: None,
        instances: 1,
        extra_tags: &[],
        needs_fallible: true,
    },
    plain_position(
        "field_factory_or_lit",
        "",
        "let h = Holder { c: make_f(1) or Subj { n: 1 } };\nprintln(\"u=\", h.peek());",
    ),
    plain_position(
        "field_factory_or_call",
        "",
        "let h = Holder { c: make_f(1) or make2(1) };\nprintln(\"u=\", h.peek());",
    ),
    plain_position(
        "field_default_factory",
        "",
        "let h = DHolder { };\nprintln(\"u=\", h.peek());",
    ),
    // GH #896's shape: a receiver literal inside a locus-typed
    // field's NON-literal initialiser, where it can take the
    // parent-owned flag meant for the field's value.
    Position {
        id: "field_nested_receiver",
        decls: "",
        stmts: "let h = Holder { c: make(Cfg { }.seed()) };\nprintln(\"u=\", h.peek());",
        twin_decls: None,
        twin_stmts: None,
        instances: 1,
        extra_tags: &[("D:cfg", 1)],
        needs_fallible: false,
    },
    // --- the same, against an INTERFACE-typed field ----------
    plain_position(
        "iface_field_literal",
        "",
        "let h = IHolder { c: Subj { n: 1 } };\nprintln(\"u=\", h.peek());",
    ),
    plain_position(
        "iface_field_factory",
        "",
        "let h = IHolder { c: make(1) };\nprintln(\"u=\", h.peek());",
    ),
    plain_position(
        "iface_field_or_call",
        "",
        "let h = IHolder { c: make_f(1) or make2(1) };\nprintln(\"u=\", h.peek());",
    ),
    // --- and against a `perspective(P)`-typed field --------------
    //
    // GH #921 A3. This shape had NO cell: the `persp_child` type
    // axis puts a perspective field on the SUBJECT, and every field
    // position puts the subject behind a locus- or interface-typed
    // one, so "a factory's result stored into a perspective-typed
    // field" was never generated. It is the one place F.39 and
    // lowering disagreed outside the four families — the F.17 gate
    // covered `LocusRef` and `Interface` and not `Perspective`, so
    // the value took the GH #402 frame temporary while F.39 says the
    // field owns it. The impl holds the subject so the type axis
    // still bites.
    Position {
        id: "persp_field_factory",
        decls: PERSP_FIELD_DECLS,
        stmts: "let h = PHolder { r: makep() };\nprintln(\"u=\", h.peek());",
        twin_decls: None,
        twin_stmts: None,
        instances: 1,
        extra_tags: &[("D:proute", 1)],
        needs_fallible: false,
    },
];

/// The declarations `persp_field_factory` needs. A second
/// perspective, so the `persp_child` shape's own `Route` is
/// untouched, and an impl that holds the subject so the shape axis
/// is not vacuous for this position.
const PERSP_FIELD_DECLS: &str = "
perspective PRoute { fn pv() -> Int; }

locus PRouteV1 : serves PRoute {
    params { s: Subj = Subj { n: 1 }; }
    dissolve() { println(\"D:proute\"); }
    fn pv() -> Int { return self.s.probe(); }
}

locus PHolder {
    params { r: perspective(PRoute) = PRouteV1 { }; }
    fn peek() -> Int { return self.r.pv(); }
}

fn makep() -> PRouteV1 { return PRouteV1 { }; }
";


// ===================================================================
// Axis 3 — the context
// ===================================================================

struct Context {
    id: &'static str,
    /// How many times the payload runs per program.
    repeat: usize,
}

const CONTEXTS: &[Context] = &[
    Context { id: "main", repeat: 1 },
    Context { id: "free_fn", repeat: 1 },
    Context { id: "method", repeat: 1 },
    Context { id: "loop", repeat: 4 },
    Context { id: "module", repeat: 1 },
    Context { id: "guard_taken", repeat: 1 },
    Context { id: "guard_untaken", repeat: 1 },
];

/// Wrap a payload in a context's frame.
///
/// Returns `(top-level declarations, the statements `fn main` runs)`.
fn wrap(ctx: &Context, stmts: &str, fallible: bool) -> (String, String) {
    let fal = if fallible { " fallible(String)" } else { "" };
    let orr = if fallible { " or raise" } else { "" };
    let body = indent(stmts, 4);
    let body2 = indent(stmts, 8);
    match ctx.id {
        "main" => (String::new(), body),
        "free_fn" => (
            ["fn mx_work()", fal, " {\n", &body, "\n}\n"].concat(),
            ["    mx_work()", orr, ";"].concat(),
        ),
        "method" => (
            [
                "locus MxHost {\n    fn work() -> Int",
                fal,
                " {\n",
                &body2,
                "\n        return 0;\n    }\n}\n",
            ]
            .concat(),
            [
                "    let mx_h = MxHost { };\n    mx_h.work()",
                orr,
                ";",
            ]
            .concat(),
        ),
        "loop" => (
            String::new(),
            [
                "    let mut mx_i = 0;\n    while mx_i < 4 {\n",
                &body2,
                "\n        mx_i = mx_i + 1;\n    }",
            ]
            .concat(),
        ),
        "module" => (
            [
                "module mx {\n    fn mx_work()",
                fal,
                " {\n",
                &body2,
                "\n    }\n}\n",
            ]
            .concat(),
            ["    mx_work()", orr, ";"].concat(),
        ),
        "guard_taken" | "guard_untaken" => (
            [
                "fn mx_work(flag: Bool)",
                fal,
                " {\n",
                &body,
                "\n    if flag { return; }\n    println(\"tail\");\n}\n",
            ]
            .concat(),
            [
                "    mx_work(",
                if ctx.id == "guard_taken" { "true" } else { "false" },
                ")",
                orr,
                ";",
            ]
            .concat(),
        ),
        other => panic!("no frame template for context {other}"),
    }
}

fn indent(text: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    text.lines()
        .map(|l| [pad.as_str(), l].concat())
        .collect::<Vec<_>>()
        .join("\n")
}

// ===================================================================
// Cells
// ===================================================================

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Cell {
    position: usize,
    shape: usize,
    context: usize,
}

fn cell_id(c: Cell) -> String {
    [
        POSITIONS[c.position].id,
        "/",
        SHAPES[c.shape].id,
        "/",
        CONTEXTS[c.context].id,
    ]
    .concat()
}

fn all_cells() -> Vec<Cell> {
    let mut out = Vec::with_capacity(
        POSITIONS.len() * SHAPES.len() * CONTEXTS.len(),
    );
    for position in 0..POSITIONS.len() {
        for shape in 0..SHAPES.len() {
            for context in 0..CONTEXTS.len() {
                out.push(Cell { position, shape, context });
            }
        }
    }
    out
}

fn full_mode() -> bool {
    matches!(std::env::var("HALE_MATRIX").as_deref(), Ok("full"))
}

/// One `KNOWN_OPEN` cell per open POSITION — the first in matrix
/// order — which is the granularity the open defects actually have:
/// each of the four families fails for every type and (bar the
/// loop-only one) every context, so 200 cells are open but only ten
/// distinct positions are.
fn open_representatives(all: &[Cell]) -> Vec<Cell> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::new();
    for c in all {
        let id = cell_id(*c);
        if open_reason(&id).is_some()
            && seen.insert(POSITIONS[c.position].id)
        {
            out.push(*c);
        }
    }
    out
}

/// The per-PR sample: every open position's representative cell and
/// its six axis neighbours, one cell per position (which also covers
/// every type and every context), then a co-prime stride fill to
/// [`TARGET_SAMPLE`].
///
/// Note the deviation from the brief, which asked for the neighbours
/// of every `KNOWN_OPEN` cell: with 200 open cells that is most of
/// the matrix, and a "sample" that runs 400 of 910 cells buys
/// nothing over the full run. Per open POSITION is the same
/// neighbourhood at the granularity the defects have. The full
/// matrix, which holds every open cell to its verdict, is 100
/// seconds — `HALE_MATRIX=full`.
fn selected_cells() -> Vec<Cell> {
    let all = all_cells();
    if full_mode() {
        return all;
    }
    let index: BTreeMap<String, usize> = all
        .iter()
        .enumerate()
        .map(|(i, c)| (cell_id(*c), i))
        .collect();
    let at = |c: Cell| -> usize { index[&cell_id(c)] };

    let mut picked: BTreeSet<usize> = BTreeSet::new();

    // Every position at least once, walking the other two axes so
    // every type and every context appears too.
    for p in 0..POSITIONS.len() {
        picked.insert(at(Cell {
            position: p,
            shape: p % SHAPES.len(),
            context: p % CONTEXTS.len(),
        }));
    }

    // Every open position's representative, plus the cells one step
    // away on each axis — the neighbourhood a Phase A fix is most
    // likely to move.
    for (id, _) in KNOWN_OPEN {
        assert!(
            index.contains_key(*id),
            "KNOWN_OPEN names {id:?}, which is not a cell of this \
             matrix — a renamed axis value leaves a dangling \
             expected-failure"
        );
    }
    for c in open_representatives(&all) {
        picked.insert(at(c));
        for n in neighbours(c) {
            picked.insert(at(n));
        }
    }

    let mut k = 0;
    while picked.len() < TARGET_SAMPLE && k < all.len() {
        picked.insert((k * FILL_STRIDE) % all.len());
        k += 1;
    }

    picked.into_iter().map(|i| all[i]).collect()
}

/// The six cells one step away, one on each axis, wrapping.
fn neighbours(c: Cell) -> Vec<Cell> {
    let step = |i: usize, len: usize, up: bool| -> usize {
        if up { (i + 1) % len } else { (i + len - 1) % len }
    };
    let mut out = Vec::new();
    for up in [true, false] {
        out.push(Cell { position: step(c.position, POSITIONS.len(), up), ..c });
        out.push(Cell { shape: step(c.shape, SHAPES.len(), up), ..c });
        out.push(Cell { context: step(c.context, CONTEXTS.len(), up), ..c });
    }
    out
}

// ===================================================================
// Rendering
// ===================================================================

fn program_source(c: Cell, twin: bool) -> String {
    let shape = &SHAPES[c.shape];
    let position = &POSITIONS[c.position];
    let ctx = &CONTEXTS[c.context];
    let (pdecls, pstmts) = if twin {
        (
            position.twin_decls.unwrap_or(position.decls),
            position.twin_stmts.expect("a twin was asked for"),
        )
    } else {
        (position.decls, position.stmts)
    };
    let (cdecls, cmain) = wrap(ctx, pstmts, position.needs_fallible);
    [
        shape.decls,
        COMMON,
        pdecls,
        "\n",
        &cdecls,
        "\nfn main() {\n",
        &cmain,
        "\n    println(\"end\");\n}\n",
    ]
    .concat()
}

// ===================================================================
// Running
// ===================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Oracle {
    Check,
    Build,
    Tags,
    Residency,
    Asan,
    Differential,
}

struct Outcome {
    ran: Vec<Oracle>,
    failures: Vec<(Oracle, String)>,
}

struct RunOut {
    stdout: String,
    stderr: String,
    /// Empty when the process exited zero within the deadline.
    verdict: String,
}

/// Run `bin`, redirecting output to files so a large sanitizer
/// report on stderr can never deadlock a pipe, and killing it at
/// [`DEADLINE`].
fn run_bin(bin: &Path, envs: &[(&str, &str)]) -> RunOut {
    let out_path = bin.with_extension("out");
    let err_path = bin.with_extension("err");
    let out_file = File::create(&out_path).expect("create stdout file");
    let err_file = File::create(&err_path).expect("create stderr file");
    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::null()).stdout(out_file).stderr(err_file);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn the generated binary");
    let start = Instant::now();
    let verdict = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) if s.success() => break String::new(),
            Some(s) => break format!("exited {s:?}"),
            None if start.elapsed() > DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                break format!("did not finish within {DEADLINE:?}");
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let read = |p: &Path| -> String {
        let mut s = String::new();
        let _ = File::open(p).and_then(|mut f| f.read_to_string(&mut s));
        s
    };
    let stdout = read(&out_path);
    let stderr = read(&err_path);
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
    RunOut { stdout, stderr, verdict }
}

fn slug(id: &str) -> String {
    id.replace('/', "_")
}

/// What a `dissolve()` tag line looks like, as a whole line, so
/// `D:subj` never matches `D:subject`.
fn tag_count(stdout: &str, tag: &str) -> usize {
    stdout.lines().filter(|l| l.trim_end() == tag).count()
}

/// Markers that mean a sanitizer fired, regardless of exit code.
const SANITIZER_MARKERS: &[&str] = &[
    "ERROR: AddressSanitizer",
    "ERROR: LeakSanitizer",
    "Direct leak",
    "Indirect leak",
    "heap-use-after-free",
    "double-free",
    "attempting double-free",
    "attempting free on address which was not malloc",
    "heap-buffer-overflow",
    "stack-buffer-overflow",
    "SEGV on unknown address",
];

fn run_cell(c: Cell) -> Outcome {
    let id = cell_id(c);
    let shape = &SHAPES[c.shape];
    let position = &POSITIONS[c.position];
    let ctx = &CONTEXTS[c.context];
    let mut ran = Vec::new();
    let mut failures = Vec::new();

    let src = program_source(c, false);
    let program = hale_syntax::parse_source(&src).unwrap_or_else(|e| {
        panic!("{id}: the generator emitted unparsable Hale: {e:?}\n{src}")
    });

    // --- oracle 0a: the checker accepts it -------------------
    //
    // Not one of the four, but free, and it keeps the generator
    // honest: a program `check` refuses is not a measurement of
    // ownership.
    ran.push(Oracle::Check);
    let check_errors: Vec<String> = hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    if !check_errors.is_empty() {
        failures.push((
            Oracle::Check,
            format!("check refused it: {check_errors:?}"),
        ));
    }

    // --- oracle 0b: it builds --------------------------------
    ran.push(Oracle::Build);
    let bin = harness::unique_bin(&["ownmatrix_", &slug(&id)].concat());
    if let Err(e) = build_executable(&program, &bin) {
        failures.push((Oracle::Build, format!("build refused it: {e:?}")));
        return Outcome { ran, failures };
    }

    // --- oracles 1 + 2: one run answers both -----------------
    let run = run_bin(&bin, &[("LOTUS_ARENA_RESIDENCY", "1")]);
    let _ = std::fs::remove_file(&bin);

    ran.push(Oracle::Tags);
    if !run.verdict.is_empty() {
        failures.push((
            Oracle::Tags,
            format!("the program did not run cleanly: {}\nstdout:\n{}\nstderr:\n{}", run.verdict, run.stdout, run.stderr),
        ));
    } else {
        let mut expected: BTreeMap<&str, usize> = BTreeMap::new();
        for tag in shape.tags {
            *expected.entry(tag).or_default() +=
                position.instances * ctx.repeat;
        }
        for (tag, n) in position.extra_tags {
            *expected.entry(tag).or_default() += n * ctx.repeat;
        }
        let mut wrong = Vec::new();
        for (tag, want) in &expected {
            let got = tag_count(&run.stdout, tag);
            if got != *want {
                wrong.push(format!("{tag}: expected {want}, saw {got}"));
            }
        }
        // A tag nothing accounted for is as wrong as a missing one.
        for line in run.stdout.lines() {
            let l = line.trim_end();
            if l.starts_with("D:") && !expected.contains_key(l) {
                wrong.push(format!("{l}: printed but not expected at all"));
            }
        }
        if !run.stdout.contains("u=") {
            wrong.push(
                "the payload printed no `u=` line — the cell measured \
                 nothing"
                    .to_string(),
            );
        }
        if !run.stdout.lines().any(|l| l.trim_end() == "end") {
            wrong.push(
                "`main` did not reach its final `end` line".to_string(),
            );
        }
        if !wrong.is_empty() {
            failures.push((
                Oracle::Tags,
                format!("{}\nstdout:\n{}", wrong.join("; "), run.stdout),
            ));
        }
    }

    ran.push(Oracle::Residency);
    if run.verdict.is_empty() {
        match residency(&run.stderr) {
            None => failures.push((
                Oracle::Residency,
                format!(
                    "no `[arena_residency dump]` line — the knob was not \
                     honoured, so this oracle measured nothing\nstderr:\n{}",
                    run.stderr
                ),
            )),
            Some(0) => {}
            Some(n) => failures.push((
                Oracle::Residency,
                format!("{n} arena(s) still live at exit\nstderr:\n{}", run.stderr),
            )),
        }
    }

    // --- oracle 4: the two spellings are one program ---------
    if position.twin_stmts.is_some() {
        ran.push(Oracle::Differential);
        let tsrc = program_source(c, true);
        let tprogram = hale_syntax::parse_source(&tsrc).unwrap_or_else(|e| {
            panic!("{id}: the generator emitted an unparsable twin: {e:?}\n{tsrc}")
        });
        let tbin =
            harness::unique_bin(&["ownmatrix_twin_", &slug(&id)].concat());
        match build_executable(&tprogram, &tbin) {
            Err(e) => failures.push((
                Oracle::Differential,
                format!("the `let`-named twin does not build: {e:?}"),
            )),
            Ok(()) => {
                let trun = run_bin(&tbin, &[("LOTUS_ARENA_RESIDENCY", "1")]);
                let _ = std::fs::remove_file(&tbin);
                if !trun.verdict.is_empty() {
                    failures.push((
                        Oracle::Differential,
                        format!("the twin did not run cleanly: {}", trun.verdict),
                    ));
                } else if trun.stdout != run.stdout {
                    failures.push((
                        Oracle::Differential,
                        format!(
                            "the inline and `let`-named spellings are the \
                             same program by spec/semantics.md, and printed \
                             differently.\ninline:\n{}\nlet-named:\n{}",
                            run.stdout, trun.stdout
                        ),
                    ));
                }
            }
        }
    }

    // --- oracle 3: the sanitizer --------------------------------
    ran.push(Oracle::Asan);
    let abin = harness::unique_bin(&["ownmatrix_asan_", &slug(&id)].concat());
    harness::build_asan(&program, &abin);
    let arun = run_bin(
        &abin,
        &[
            ("ASAN_OPTIONS", "detect_leaks=1"),
            ("LOTUS_NO_CHUNK_POOL", "1"),
        ],
    );
    let _ = std::fs::remove_file(&abin);
    let report = [arun.stdout.as_str(), arun.stderr.as_str()].concat();
    let hits: Vec<&str> = SANITIZER_MARKERS
        .iter()
        .copied()
        .filter(|m| report.contains(m))
        .collect();
    if !hits.is_empty() {
        failures.push((
            Oracle::Asan,
            format!("the sanitizer reported {hits:?}\n{report}"),
        ));
    } else if !arun.verdict.is_empty() {
        failures.push((
            Oracle::Asan,
            format!("the instrumented build {}\n{report}", arun.verdict),
        ));
    }

    Outcome { ran, failures }
}

/// `[arena_residency dump] N live arenas, sorted by bytes desc:` —
/// written to stderr by an atexit hook in `lotus_arena.c`.
fn residency(stderr: &str) -> Option<usize> {
    stderr
        .lines()
        .find(|l| l.contains("[arena_residency dump]"))
        .and_then(|l| l.split_whitespace().nth(2))
        .and_then(|n| n.parse().ok())
}

// ===================================================================
// The shards
// ===================================================================

fn open_reason(id: &str) -> Option<&'static str> {
    KNOWN_OPEN.iter().find(|(k, _)| *k == id).map(|(_, r)| *r)
}

/// Run every selected cell in one context, and hold each to the
/// verdict `KNOWN_OPEN` declares for it.
fn run_context(ctx_id: &str) {
    let cells: Vec<Cell> = selected_cells()
        .into_iter()
        .filter(|c| CONTEXTS[c.context].id == ctx_id)
        .collect();
    let mut green = Vec::new();
    let mut unexpected_pass = Vec::new();
    let mut unexpected_fail = Vec::new();
    for c in cells {
        let id = cell_id(c);
        let outcome = run_cell(c);
        assert!(
            !outcome.ran.is_empty(),
            "{id}: no oracle ran — a cell that measures nothing is \
             worse than a missing one"
        );
        let summary: Vec<String> = outcome
            .failures
            .iter()
            .map(|(o, m)| format!("{o:?}: {m}"))
            .collect();
        match open_reason(&id) {
            Some(reason) if outcome.failures.is_empty() => {
                unexpected_pass.push(format!("{id}  (listed as: {reason})"));
            }
            Some(reason) => {
                println!("KNOWN_OPEN {id}\n  reason: {reason}\n  {}", summary.join("\n  "));
                green.push(id);
            }
            None if outcome.failures.is_empty() => {
                green.push(id);
            }
            None => {
                unexpected_fail.push(format!("{id}\n  {}", summary.join("\n  ")));
            }
        }
    }
    assert!(
        unexpected_fail.is_empty(),
        "{} cell(s) in context `{ctx_id}` failed an ownership oracle and \
         are not listed in KNOWN_OPEN:\n\n{}",
        unexpected_fail.len(),
        unexpected_fail.join("\n\n")
    );
    assert!(
        unexpected_pass.is_empty(),
        "{} cell(s) in context `{ctx_id}` are listed in KNOWN_OPEN and \
         now pass every oracle. That is the regression test firing: \
         delete the entry so the cell is held to its verdict from \
         here on.\n\n{}",
        unexpected_pass.len(),
        unexpected_pass.join("\n")
    );
    assert!(
        !green.is_empty(),
        "context `{ctx_id}` ran no cells at all — the sample dropped a \
         whole axis value"
    );
}

#[test]
fn matrix_in_main() {
    run_context("main");
}

#[test]
fn matrix_in_a_free_fn() {
    run_context("free_fn");
}

#[test]
fn matrix_in_a_method_frame() {
    run_context("method");
}

#[test]
fn matrix_in_a_loop() {
    run_context("loop");
}

#[test]
fn matrix_in_a_module() {
    run_context("module");
}

#[test]
fn matrix_behind_a_taken_guard() {
    run_context("guard_taken");
}

#[test]
fn matrix_behind_an_untaken_guard() {
    run_context("guard_untaken");
}

// ===================================================================
// The matrix itself
// ===================================================================

/// The matrix must be rectangular, big, and actually sampled — a
/// generator that silently produces two cells passes every oracle.
#[test]
fn the_matrix_is_rectangular_and_non_vacuous() {
    assert_eq!(POSITIONS.len(), 27, "position axis changed size");
    assert_eq!(SHAPES.len(), 5, "type axis changed size");
    assert_eq!(CONTEXTS.len(), 7, "context axis changed size");

    let all = all_cells();
    assert_eq!(all.len(), 27 * 5 * 7);
    let ids: BTreeSet<String> = all.iter().map(|c| cell_id(*c)).collect();
    assert_eq!(ids.len(), all.len(), "two cells share an id");

    for (id, reason) in KNOWN_OPEN {
        assert!(
            ids.contains(*id),
            "KNOWN_OPEN names {id:?}, which is not a cell — a renamed \
             axis value leaves a dangling expected-failure"
        );
        assert!(
            !reason.trim().is_empty(),
            "{id} is listed open with no reason"
        );
    }

    let sample = selected_cells();
    if full_mode() {
        assert_eq!(sample.len(), all.len());
    } else {
        assert!(
            sample.len() >= TARGET_SAMPLE,
            "the per-PR sample shrank to {} cells",
            sample.len()
        );
        assert!(
            sample.len() <= all.len() / 4,
            "the per-PR sample grew to {} of {} cells — the default \
             must stay cheap; if it has to be this big, run the full \
             matrix instead",
            sample.len(),
            all.len()
        );
        // Every open POSITION's representative and its neighbourhood
        // must be in it.
        let picked: BTreeSet<String> =
            sample.iter().map(|c| cell_id(*c)).collect();
        let reps = open_representatives(&all);
        let open_positions: BTreeSet<&str> = KNOWN_OPEN
            .iter()
            .map(|(id, _)| id.split('/').next().expect("cell id"))
            .collect();
        assert_eq!(
            reps.len(),
            open_positions.len(),
            "one representative per open position"
        );
        for c in reps {
            let id = cell_id(c);
            assert!(picked.contains(&id), "{id} represents an open \
                 position and is not sampled");
            for n in neighbours(c) {
                let nid = cell_id(n);
                assert!(
                    picked.contains(&nid),
                    "{nid} neighbours the open cell {id} and is not sampled"
                );
            }
        }
    }

    // Every axis value appears, or a whole family goes untested and
    // the matrix is green for the wrong reason.
    for axis in [
        (
            "position",
            POSITIONS.iter().map(|p| p.id).collect::<Vec<_>>(),
            sample.iter().map(|c| POSITIONS[c.position].id).collect::<Vec<_>>(),
        ),
        (
            "type",
            SHAPES.iter().map(|s| s.id).collect::<Vec<_>>(),
            sample.iter().map(|c| SHAPES[c.shape].id).collect::<Vec<_>>(),
        ),
        (
            "context",
            CONTEXTS.iter().map(|c| c.id).collect::<Vec<_>>(),
            sample.iter().map(|c| CONTEXTS[c.context].id).collect::<Vec<_>>(),
        ),
    ] {
        let (name, want, got) = axis;
        let seen: BTreeSet<&str> = got.into_iter().collect();
        for v in want {
            assert!(
                seen.contains(v),
                "the sample covers no cell with {name} `{v}`"
            );
        }
    }

    // And every cell must render to something a parser accepts —
    // cheap, and it catches a template that only breaks in a corner
    // of the space the sample does not reach.
    for c in &all {
        let src = program_source(*c, false);
        assert!(
            hale_syntax::parse_source(&src).is_ok(),
            "{} does not parse:\n{src}",
            cell_id(*c)
        );
        if POSITIONS[c.position].twin_stmts.is_some() {
            let tsrc = program_source(*c, true);
            assert!(
                hale_syntax::parse_source(&tsrc).is_ok(),
                "{}'s twin does not parse:\n{tsrc}",
                cell_id(*c)
            );
        }
    }

    // A differential that never runs is not an oracle.
    let with_twin =
        POSITIONS.iter().filter(|p| p.twin_stmts.is_some()).count();
    assert_eq!(
        with_twin, 9,
        "the differential covers the four expression positions and the \
         five `return` ones; if that set changed, say so here"
    );
}
