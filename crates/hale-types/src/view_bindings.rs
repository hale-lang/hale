//! Bindings that view a field another write can replace under them.
//!
//! A binding of a String or Bytes read from a place (`let s =
//! self.f.s;`) holds the field's own pointer: lowering copies a record
//! read from a place (GH #713) but not a String, so the hot path of
//! reading a field into a local costs nothing. The pointer is only good
//! while the field keeps that value. A write to the field replaces it:
//! a same-length String or Bytes assign overwrites the bytes in place
//! (spec/memory.md rule 7), and a replace that does not fit, a whole
//! record store, or a payload enum's, retires the old block, reusable
//! after the next activation-boundary flush (rule 6). A callee that
//! replaces the field runs its flush when it returns, so a local held
//! across the call read whatever reused the block next — `let s =
//! self.f.s; self.put("NEW"); s` printed a later value, and under ASan
//! read freed memory. A match arm's payload binding of a field
//! (`match self.e { E::A(s) => .. }`), a `for` element of an array
//! field, a tuple let's elements and a bare local reassigned from a
//! field are the same view.
//!
//! This pass finds the bindings for which that can happen, so lowering
//! binds an owned copy of exactly those and leaves every other one a
//! view. A value views the places it may have been read from, named by
//! their field names, its keys:
//!
//! - a field read views its field and what it is read from, when that
//!   views a place or is a handle (`self`, a handle parameter or local,
//!   a locus literal, a call declared to return one, any field read); a
//!   field of a record value the frame holds itself views nothing a
//!   write can replace (a write through a local root replaces the
//!   field's pointer, GH #993);
//! - a `match` arm's payload, an `if`, block or `or` result, and a
//!   tuple, array or record literal view what they carry out (a
//!   literal's binding copies the parts that view a place where the
//!   literal is built, not the literal whole);
//! - an enum constructor views its arguments and a form's read
//!   (`self.v.get(i)`) its cell; any other call's result is copied out
//!   of the callee's frame (a method's into the caller's arena, a
//!   stdlib leaf's built fresh) and views nothing a write may replace —
//!   though, for the local-place rule below, it may be a place's value
//!   when its callee returns a part of a parameter or a cell's read
//!   (`Summaries::ret_views`).
//!
//! A binding needs its own copy when some use of it comes after (or as
//! an argument of) a write that may change one of its keys, and so
//! does an operand held while a later sibling is evaluated (an earlier
//! argument, a receiver, a binary operator's left side, a literal's
//! earlier field) when that sibling's evaluation may change one:
//!
//! - an assignment `x.f... = v` (any root) writes key `f`;
//! - a call writes what its callee writes, transitively: a locus
//!   method by name, over every locus declaring one (the receiver's
//!   type is not known here), a free fn by name, and a method called
//!   on a field (`self.v.set(..)`, a form) writes that field too;
//! - a publish, an explicit `yield` and a stdlib leaf that publishes
//!   or parks on an `async_io` pool (where another delivery to the same
//!   locus runs meanwhile) write what any bus handler writes;
//! - a call the pass cannot positively identify — through a fn value
//!   (a local, a parameter, a field whose type is a fn or cannot be
//!   classified), of a name no body declares, a recovery statement —
//!   may write anything. Unknown never means "writes nothing".
//!
//! Every question about a type (a handle? a fn value? a form?) is
//! answered by `Bodies::classify` over the resolved type — aliases
//! followed through imports and minted names — and a type it cannot
//! place answers the way that costs a copy.
//!
//! In a loop, a write anywhere in the body reaches a use anywhere in it
//! when the variable was declared before the loop, wherever in the
//! loop the instance holding the value was assigned. Field names stand
//! in for fields of any type, so the answer only errs toward a copy.
//!
//! # Values read from a form's cell
//!
//! A value read from a `@form(hashmap)` that reads in place (`get`,
//! `entry_at`; a map with no `sync =` or `sync = none`) or from a
//! `@form(lru_cache)` (`get`) is the cell's bytes copied into the
//! reader's frame: its top-level fields are the reader's own, its
//! nested blocks (a nested record, a tuple, an out-of-line array, a
//! payload enum, and what they hold) are still the cell's. The
//! language rule is value semantics: a value you read is yours, and
//! writing into it never changes the cell (spec/forms.md § Reading a
//! cell). The pass keeps the read shared, and decides where a write
//! needs a copy:
//!
//! - a read's value carries a marker key naming the form's place (its
//!   receiver spelled as a path, `self.m`, a local by its variable);
//!   a field, an element, a `match` payload or a `for` element of it
//!   carries an in-cell marker instead — that value is the cell's
//!   block itself, its top level included; `for e in m.entries` over
//!   such a map is one too;
//! - a write into a nested part of the local the read is bound to
//!   (`c.inner.k = 7`, `c.big[j] = x`) copies that part into the
//!   frame at the part's first write (copy on write: a flag per local
//!   and part, cleared where the local is bound to a read), unless the
//!   write is part of a get-modify-set: the local is set back into the
//!   same form (`m.set(c)`, or a literal that keys the same cell), on
//!   every path after the write, before any read that may see the cell
//!   (another read of a form that may be this one, a call that may
//!   read a form, a delivery the bus runs) and with the local not used
//!   after the set. Then the write lands in the cell's block, which the
//!   set keeps, and no one can tell — the idiom stays allocation-free;
//! - a value in-cell that is written at all, a local the read is bound
//!   to by any other binding, or one passed (whole or by a part) to a
//!   call that may write through its parameter (a body that assigns
//!   under a parameter, transitively; a callee the pass cannot identify
//!   may), binds an owned copy instead, as a viewed binding does;
//! - each copy this rule adds is reported with its reason
//!   ([`OwnedViews::cell_copies`]), which the allocation survey turns
//!   into a site of its own, so the hot-path lint and `@budget` see it.
//!
//! # Locals bound from a place
//!
//! The same rule holds for any place, not only a cell (spec/types.md §
//! A local bound from a place is yours): an array, a tuple's element, a
//! payload, a `for` element — any compound value bound from a field, an
//! element, a local or a parameter without a copy — is the place's own
//! block, and a write into the local must never land there. The pass
//! marks such a value with a place marker (the place spelled as a path)
//! and decides:
//!
//! - a write into a local (`a[i] = x`, `x.k = 1`, at any depth) whose
//!   instance carries a marker and was not copied at its binding (an
//!   owned view; a struct `let` or `=` of a place, GH #713 — known by
//!   the write's shape, `v.name..`, or a declared type) copies the whole
//!   value at the first write: a flag per local, set at every binding
//!   (`let`, `=`, tuple `let`, `for`, `match` payload), so one copy
//!   serves every later write. A struct literal is fresh at its top
//!   level: only a write below a field whose init holds a place's value
//!   copies, and only that field;
//! - unless it is a get-modify-store of a self field (`let mut v =
//!   self.f; v.x[i] = y; self.f = v;`): every binding of the local is a
//!   `let`/`=` of that one field, the whole store back follows on every
//!   path, nothing that may read the field comes between, no other
//!   binding views it and the local is not used after. The store keeps
//!   the block the field already holds, so the idiom copies nothing;
//! - a local handed to a call that may write through that parameter is
//!   bound as an owned copy (a parameter is a reference by design, GH
//!   #1550: a callee writing through its own parameter is seen). Which
//!   parameter a body writes through is summarized by position
//!   (`Summaries::param_masks`): an assignment rooted at it, or handing
//!   it on to a callee's written one — never a local bound from it,
//!   which this rule copies at its write; a primitive or handle
//!   parameter is never written into. The cell-read rule keeps its
//!   broader per-body flag (`writes_params`) as a second net;
//! - the other direction: a binding of a local's or a parameter's value
//!   that is live across a write into that value in place (a slot of
//!   the block it holds, or anything under it written by a call) is
//!   bound as an owned copy;
//! - a part of a local assigned from a place (`h.arr = r.arr`, `xs[0] =
//!   m.get(k) or d`) binds the local anew: a part marker for the field
//!   (an element: the whole local), cleared again after the store, so the
//!   next write through it copies that part;
//! - a call's result is a place's value when a callee may return a part
//!   of a parameter (the argument handed there), a cell's read, or what
//!   an unidentified callee returns (`Summaries::ret_views`, by parameter
//!   position, transitively; a method on `self` or on a value of a known
//!   locus type resolves to that locus's method, and a locus's own
//!   storage is copied out on return);
//! - a `@form(vec)`'s `.items` walk, a map's `.entries` and a walk of
//!   `self.children` keep visiting in place, and a value known to be a
//!   primitive (a literal, an operator's result, a declared or inferred
//!   scalar return, an `if`, `match` or block carrying one out, a tuple
//!   element of one) or a handle carries no marker.
//!
//! The copies are reported like the cell rule's, with `PLACE_*` reasons.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use hale_syntax::ast::*;
use hale_syntax::Span;

/// The binding sites lowering binds an owned copy at. A `match` is
/// named by its span (a `MatchStmt` has no id); every other site by its
/// snapshot id.
#[derive(Debug, Clone, Default)]
pub struct OwnedViews {
    sites: HashSet<Site>,
    /// Assignments (by id) that write into a part of a local whose
    /// part another name may still reach: a nested part (a field name)
    /// of a local bound to a form cell's read, or the whole value (`""`)
    /// of a local bound from a place. The part is copied into the frame
    /// first, if the local's flag for it says it is not the frame's yet.
    cow_writes: HashMap<u32, String>,
    /// The bindings of those locals — a `let`, a bare `=`, a tuple
    /// `let`, a `for` (by id), a `match`'s payloads (by its span) — and
    /// the name each binds, with the parts their flags track: `true`
    /// when the value bound there is the frame's own already (a fresh
    /// value, or an owned copy).
    cow_binds: HashMap<(Site, String), Vec<(String, bool)>>,
    /// Every copy the cell-read rule adds, with its reason.
    cell_copies: Vec<CellCopy>,
    /// The bindings (by id) the cell-read rule owns: a `for` over a
    /// map's entries copies each entry it binds only for this rule.
    cell_owned: HashSet<u32>,
}

/// A copy the cell-read rule adds: where (a write's statement, a
/// binding's statement, an argument), and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellCopy {
    /// The program (an index into the programs the pass ran over) whose
    /// body holds it.
    pub program: usize,
    /// A statement's span (a write's, a binding's), or with `operand`
    /// an argument's.
    pub span: Span,
    pub operand: bool,
    pub reason: &'static str,
}

/// The reasons a write into a value read from a form's cell copies.
pub mod reasons {
    /// Another read of the form (or a call or delivery that may read
    /// one) comes between the write and the set.
    pub const READ_BEFORE_SET: &str = "copy: the cell is read again before the set";
    /// The local is used after the set that would have kept the write.
    pub const OUTLIVES_SET: &str = "copy: the value outlives the set";
    /// No set of the local into its form follows the write.
    pub const NO_SET: &str = "copy: the value is not set back into its map";
    /// The set may not run after the write: it is in a branch or a
    /// loop the write is not, or a path leaves before it.
    pub const NOT_EVERY_PATH: &str = "copy: the set is not on every path after the write";
    /// The set may key another cell than the one read.
    pub const OTHER_KEY: &str = "copy: the set may key another cell";
    /// The value, or a part of it, is passed to a call that may write
    /// through its parameter.
    pub const CALL_WRITES: &str = "copy: the value is passed to a call that may write through its parameter";
    /// A binding that is the cell's own block (a part of the value, an
    /// element, a payload, an iteration's entry) is written.
    pub const VIEW_WRITTEN: &str = "copy: a binding into the cell is written";
    /// A local bound from a place (a field, an element, a local, a
    /// parameter, a `match` payload, a `for` element) is written into:
    /// the value is copied at its first write (spec/types.md § A local
    /// bound from a place is yours).
    pub const PLACE_WRITTEN: &str = "copy: written local bound from a place";
    /// A local bound from a place is handed to a call that may write
    /// through its parameter: the local is bound as an owned copy.
    pub const PLACE_CALL_WRITES: &str =
        "copy: a local bound from a place is passed to a call that may write through its parameter";

    /// The local or parameter a local was read from is written into
    /// while the local is live (`let a = r.names; r.names[0] = x;`): the
    /// local is bound as an owned copy.
    pub const PLACE_SOURCE_WRITTEN: &str = "copy: what the local was read from is written while it is live";

    /// Is the reason the local-place rule's (`PLACE_*`), not the
    /// cell-read rule's?
    pub fn is_place(reason: &str) -> bool {
        reason == PLACE_WRITTEN || reason == PLACE_CALL_WRITES || reason == PLACE_SOURCE_WRITTEN
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Site {
    /// A `let`, a tuple `let`, a bare-local `=` or a `for`, by id.
    Node(u32),
    /// A `match`'s payload bindings, by the match's span.
    Match(u32, u32),
    /// A call's argument or receiver read from a place the callee may
    /// replace (the callee reads its parameter after), by its span. An
    /// argument is passed by reference — a callee may write a record
    /// through its parameter, and the caller sees it — so only an
    /// immutable leaf (a String, a Bytes) is passed as a copy.
    Arg(u32, u32),
    /// A value position — a literal's part, a binary operator's or an
    /// index's operand — read from a place a later operand may replace:
    /// copied whole where it is evaluated.
    Part(u32, u32),
}

impl OwnedViews {
    /// Run the pass over every body of `programs` (lowering's merged
    /// program; the survey's checked programs and the stdlib's analysis
    /// copy); `renames` is the build's import table (a qualified path to
    /// the declaration it names).
    pub fn of(programs: &[&Program], renames: &BTreeMap<Vec<String>, String>) -> Self {
        let mut out = OwnedViews::default();
        let found = analyze(programs, renames);
        for (site, _) in found.sites {
            out.sites.insert(site);
        }
        out.cow_writes = found.cow_writes;
        out.cow_binds = found.cow_binds;
        out.cell_copies = found.cell_copies;
        out.cell_owned = found.cell_owned;
        out
    }

    /// Does the cell-read rule own the binding (a `let`, a `for`, by id)
    /// with this id: a value read from a form's cell that is written
    /// into, or handed to a call that may write through it?
    pub fn cell_owned(&self, id: NodeId) -> bool {
        !id.is_none() && self.cell_owned.contains(&id.0)
    }

    /// Does the `let`, tuple `let`, bare-local `=` or `for` with this id
    /// bind an owned copy?
    pub fn node(&self, id: NodeId) -> bool {
        !id.is_none() && self.sites.contains(&Site::Node(id.0))
    }

    /// Do this match's payload bindings bind owned copies?
    pub fn match_arms(&self, m: &MatchStmt) -> bool {
        self.sites.contains(&Site::Match(m.span.start.0, m.span.end.0))
    }

    /// The part a write (an assignment, by id) into a local copies
    /// first, when the local's flag for it says the frame does not own
    /// it yet: a field name (a nested part of a form cell's read), or
    /// `""`, the whole value (a local bound from a place).
    pub fn cow_write(&self, id: NodeId) -> Option<&str> {
        if id.is_none() {
            return None;
        }
        self.cow_writes.get(&id.0).map(|s| s.as_str())
    }

    /// The parts whose flags a binding of `name` (a `let`, a bare `=`,
    /// a tuple `let`, a `for`, by id) sets, each with the value it sets:
    /// `true`, the bound value is the frame's own already.
    pub fn cow_bind(&self, id: NodeId, name: &str) -> Option<&[(String, bool)]> {
        if id.is_none() || self.cow_binds.is_empty() {
            return None;
        }
        self.cow_binds.get(&(Site::Node(id.0), name.to_string())).map(|v| v.as_slice())
    }

    /// The parts whose flags this match's payload binding of `name`
    /// sets, as [`OwnedViews::cow_bind`].
    pub fn cow_match_bind(&self, m: &MatchStmt, name: &str) -> Option<&[(String, bool)]> {
        if self.cow_binds.is_empty() {
            return None;
        }
        self.cow_binds.get(&(Site::Match(m.span.start.0, m.span.end.0), name.to_string())).map(|v| v.as_slice())
    }

    /// Every copy the cell-read rule adds, in body order: the copy on
    /// write at a write, an owned binding, a copied argument.
    pub fn cell_copies(&self) -> &[CellCopy] {
        &self.cell_copies
    }

    /// Is `e` an operand read from a place a later write may replace,
    /// copied where it is evaluated? `Some(true)`: a call's argument or
    /// receiver, copied only when it is an immutable leaf (String,
    /// Bytes) — a record passed by reference stays the caller's, which
    /// the callee may write through; `Some(false)`: a value position,
    /// copied whole.
    pub fn operand(&self, e: &Expr) -> Option<bool> {
        if self.sites.is_empty() {
            return None;
        }
        let span = e.span();
        if self.sites.contains(&Site::Part(span.start.0, span.end.0)) {
            Some(false)
        } else if self.sites.contains(&Site::Arg(span.start.0, span.end.0)) {
            Some(true)
        } else {
            None
        }
    }
}

/// What a body may write: every field name, or these.
#[derive(Debug, Clone, Default, PartialEq)]
struct Writes {
    top: bool,
    /// Field names by their `Bodies::field_id`, a bit each: the
    /// summaries' fixpoint unions these for every call edge of every
    /// body, round after round.
    bits: Vec<u64>,
}

impl Writes {
    fn top() -> Self {
        Writes { top: true, bits: Vec::new() }
    }
    fn field(id: usize) -> Self {
        let mut w = Writes::default();
        let (i, b) = (id / 64, id % 64);
        w.bits.resize(i + 1, 0);
        w.bits[i] |= 1 << b;
        w
    }
    fn add(&mut self, o: &Writes) {
        self.top |= o.top;
        if self.top {
            self.bits.clear();
            return;
        }
        if self.bits.len() < o.bits.len() {
            self.bits.resize(o.bits.len(), 0);
        }
        for (a, b) in self.bits.iter_mut().zip(&o.bits) {
            *a |= *b;
        }
    }
    fn is_empty(&self) -> bool {
        !self.top && self.bits.iter().all(|w| *w == 0)
    }
    fn has(&self, id: usize) -> bool {
        self.bits.get(id / 64).is_some_and(|w| w & (1 << (id % 64)) != 0)
    }
    /// May these writes replace what `keys` views? A place marker names
    /// no place a write replaces (the local-place rule reads it): a value
    /// carrying nothing else is touched by nothing, an unknown write
    /// included.
    fn touches(&self, keys: &BTreeSet<String>, p: &Bodies<'_>) -> bool {
        (self.top && keys.iter().any(|k| !is_place_marker(k)))
            || keys.iter().any(|k| p.known_field(k).is_some_and(|id| self.has(id)))
    }
}

/// The field names a value may view, or `None` for a value that
/// views no place.
type Keys = Option<BTreeSet<String>>;

fn union(a: Keys, b: Keys) -> Keys {
    match (a, b) {
        (Some(mut a), Some(b)) => {
            a.extend(b);
            Some(a)
        }
        (a, None) => a,
        (None, b) => b,
    }
}

/// What a call names: resolved against the program's summaries once
/// they are complete.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Callee {
    Method(String),
    Fn(String),
    /// Every bus handler: what a publish, a `yield` or a park may run.
    Deliveries,
}

/// Every body of the program, with the summaries the pass reads.
struct Bodies<'a> {
    bodies: Vec<(&'a [Param], &'a Block)>,
    /// Each body's program, by its index in the programs the pass runs
    /// over.
    body_program: Vec<usize>,
    /// Method name → the summaries' index of each body declaring it.
    methods: BTreeMap<String, Vec<usize>>,
    fns: BTreeMap<String, Vec<usize>>,
    /// Params fields by name, each declared type (`None`: inferred from
    /// its default), classified once the declarations are in.
    field_types: Vec<(String, Option<&'a TypeExpr>)>,
    /// Field names a call `x.name(..)` may call as a value: a fn type, or
    /// a type this pass cannot classify.
    fn_fields: BTreeSet<String>,
    /// Bus handler names: a publish, a `yield` or a park may run any
    /// of them before it returns.
    handlers: BTreeSet<String>,
    /// `@form` loci, and every params field naming one (by its type's
    /// last segment): a form's read (`self.v.get(i)`) may hand back a
    /// view of its cell, which the form's own writes retire.
    form_loci: BTreeSet<String>,
    /// Records and enums, by every name they answer to: values the frame
    /// holds, not handles.
    records: BTreeSet<String>,
    enums: BTreeSet<String>,
    /// Struct declarations, by every name they answer to: a value of
    /// one bound by `let` (or a bare `=`) from a place is a copy
    /// already (spec/types.md § A struct binding is a copy).
    structs: BTreeSet<String>,
    /// Field names every declaration of which holds a struct; a
    /// primitive (a number, a `Bool`, a `String`, `Bytes`).
    record_fields: BTreeSet<String>,
    scalar_fields: BTreeSet<String>,
    /// Field names every declaration of which holds an array, a tuple
    /// or an enum: a block that is neither a struct nor a primitive.
    compound_fields: BTreeSet<String>,
    /// Field names every declaration of which holds a handle (a locus,
    /// an interface, a perspective, a form): a binding of one is the
    /// handle, which a write through it means to reach.
    handle_fields: BTreeSet<String>,
    /// Type aliases, by their normalized names: classified by what they
    /// name, never by their spelling.
    aliases: BTreeMap<String, &'a TypeExpr>,
    /// The build's import table: `lib::Kid` names the declaration minted
    /// for the library's `Kid`.
    renames: BTreeMap<Vec<String>, String>,
    /// Every enum variant's name: a call of one constructs a value that
    /// holds its arguments.
    variants: BTreeSet<String>,
    form_fields: BTreeSet<String>,
    /// Field names every declaration of which holds an `@form` locus.
    form_only_fields: BTreeSet<String>,
    /// The `@form` loci whose reads hand back the cell's nested blocks:
    /// a `@form(hashmap)` with no `sync =` or `sync = none`, a
    /// `@form(lru_cache)`. Every other form reads an owned copy.
    view_form_loci: BTreeSet<String>,
    /// Field names some declaration of which may hold such a form (or a
    /// type the pass cannot place).
    view_form_fields: BTreeSet<String>,
    /// Every capacity slot's `indexed_by` field: the field a set keys a
    /// cell by.
    key_fields: BTreeSet<String>,
    /// Field names every declaration of which is laid out inline in its
    /// record: a write into one is the record's own.
    inline_fields: BTreeSet<String>,
    /// The scalar declarations (`quantity`, `distinct`, `point`, a
    /// refinement), and those of them lowering lays out as an `Int`
    /// (`crate::units::typed_scalar_names`), by every name they answer
    /// to.
    scalar_decls: Vec<(&'a str, &'a ScalarDecl)>,
    /// Each scalar declaration's base type, by every name it answers to:
    /// a typed scalar is what its base is (a primitive, for the
    /// local-place rule: nothing writes into one).
    scalar_bases: BTreeMap<String, &'a TypeExpr>,
    /// Each struct's declared fields, by every name the struct answers
    /// to (a suffix shared by two structs merges them: a field then has
    /// both declarations' types, and is classified by neither alone).
    struct_fields: BTreeMap<String, Vec<(String, &'a TypeExpr)>>,
    /// Each locus's methods by (the locus, by every name it answers to;
    /// the method) → their bodies, and the locus each body is a member
    /// of: a call on `self`, or on a value of a known locus type, names
    /// that locus's method only.
    locus_methods: BTreeMap<(String, String), Vec<usize>>,
    body_locus: Vec<Option<String>>,
    /// Each body's declared return type (`None`: none, or no fn).
    body_ret: Vec<Option<&'a TypeExpr>>,
    int_scalars: BTreeSet<String>,
    /// Loci, interfaces and perspectives: a value of one of these is a
    /// handle, whose field reads view the locus's storage.
    loci: BTreeSet<String>,
    /// Every fn and method name with each body's declared return type
    /// (`None`: returns nothing), classified once the declarations are
    /// in.
    returns: Vec<(String, Option<&'a TypeExpr>)>,
    /// Fn and method names some body of which may return a handle; every
    /// body of which declares a primitive return (a typed scalar too).
    handle_fns: BTreeSet<String>,
    scalar_fns: BTreeSet<String>,
    /// Fn and method names every body of which returns nothing, a
    /// primitive or a handle: a result no write goes into.
    plain_fns: BTreeSet<String>,
    /// Every field name a write names, numbered for `Writes`' bits.
    field_ids: std::cell::RefCell<BTreeMap<String, usize>>,
}

impl Bodies<'_> {
    fn field_id(&self, name: &str) -> usize {
        let mut ids = self.field_ids.borrow_mut();
        let n = ids.len();
        *ids.entry(name.to_string()).or_insert(n)
    }
    fn known_field(&self, name: &str) -> Option<usize> {
        self.field_ids.borrow().get(name).copied()
    }
}

/// The names a declaration answers to. An imported seed's declaration
/// is minted `__lib_<alias>_<stem>_<Name>`, and a use spells it either
/// that way (inside the seed, after the mangle), through the import
/// table (`lib::Name`, `Bodies::path_name`), or — in a body the mangle
/// did not reach — as `Name`: the minted name answers to every suffix
/// after an underscore too. A suffix shared with an unrelated
/// declaration only makes the answer err toward a copy.
fn decl_names(name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if name.starts_with("__lib_") {
        for (i, _) in name.match_indices('_') {
            if i + 1 < name.len() && !name[i + 1..].starts_with('_') {
                out.push(name[i + 1..].to_string());
            }
        }
    }
    out
}

impl<'a> Bodies<'a> {
    /// The declaration a path names, by its normalized name: a qualified
    /// path through the import table (or the stdlib's), a bare name as
    /// written.
    fn path_name(&self, segs: &[Ident]) -> Option<String> {
        let last = segs.last()?;
        if segs.len() > 1 {
            let key: Vec<String> = segs.iter().map(|s| s.name.clone()).collect();
            let refs: Vec<&str> = key.iter().map(|s| s.as_str()).collect();
            if let Some(m) = self.renames.get(&key) {
                return Some(m.clone());
            }
            if let Some(m) = crate::ownership::stdlib_mangled_for_path(&refs) {
                return Some(m.to_string());
            }
        }
        Some(last.name.clone())
    }

    /// Does the call path name an enum variant (`E::A`, `lib::E::A`)?
    fn is_ctor(&self, q: &QualifiedName) -> bool {
        q.segments.len() > 1 && q.segments.last().is_some_and(|s| self.variants.contains(&s.name))
    }

    /// The one place a type is classified: `ty` with its aliases
    /// followed (`type Child = Kid;`, through any import or minted
    /// name), a mode projection taken off. Every question the pass asks
    /// of a type — a handle? a fn value? a form? — reads this.
    /// A type it cannot place (a generic parameter, a name it does not
    /// know) is `Unknown`, which every question answers the way that
    /// costs a copy rather than a use after a retire.
    fn classify(&self, ty: &TypeExpr) -> Class {
        let mut cur = ty;
        for _ in 0..16 {
            match cur {
                TypeExpr::Projection { inner, .. } => cur = inner,
                TypeExpr::Named { path, .. } => {
                    let Some(name) = self.path_name(&path.segments) else { return Class::Unknown };
                    if let Some(next) = self.aliases.get(&name) {
                        cur = next;
                        continue;
                    }
                    return self.classify_name(&name);
                }
                TypeExpr::Primitive(..) => return Class::Scalar,
                // A fixed array of scalars and a bounded list are laid
                // out inline in the record that holds them: no block of
                // their own. The element is read as lowering lays it out
                // (`array_inline_spec`): through its aliases, a typed
                // scalar as the `Int` it is.
                TypeExpr::Array { elem, .. } if self.inline_scalar(elem) => return Class::Inline,
                TypeExpr::Bounded { .. } => return Class::Inline,
                TypeExpr::Tuple(..) | TypeExpr::Array { .. } => return Class::Value,
                TypeExpr::Function { .. } => return Class::FnValue,
                TypeExpr::Perspective { .. } => return Class::Handle,
            }
        }
        Class::Unknown
    }

    /// Is `ty`, its aliases followed, a scalar lowering stores inline in
    /// a fixed array (`Int`, `Float`, `Bool`, `Decimal`, `Duration`, a
    /// scalar declaration lowered as `Int`)? Anything it cannot place is
    /// not: a block, which costs a copy.
    fn inline_scalar(&self, ty: &TypeExpr) -> bool {
        let mut cur = ty;
        for _ in 0..16 {
            match cur {
                TypeExpr::Primitive(p, _) => return crate::ty::inline_array_scalar(*p),
                TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() => {
                    let Some(name) = self.path_name(&path.segments) else { return false };
                    if let Some(next) = self.aliases.get(&name) {
                        cur = next;
                        continue;
                    }
                    return self.int_scalars.contains(&name);
                }
                _ => return false,
            }
        }
        false
    }

    /// The classes of a tuple type's elements, its aliases followed;
    /// `None` for any other type.
    fn tuple_elems(&self, ty: &TypeExpr) -> Option<Vec<Class>> {
        let mut cur = ty;
        for _ in 0..16 {
            match cur {
                TypeExpr::Projection { inner, .. } => cur = inner,
                TypeExpr::Named { path, .. } => {
                    let name = self.path_name(&path.segments)?;
                    cur = self.aliases.get(&name)?;
                }
                TypeExpr::Tuple(es, ..) => return Some(es.iter().map(|e| self.classify(e)).collect()),
                _ => return None,
            }
        }
        None
    }

    /// The locus `ty` names, its aliases followed; `None` for any other
    /// type.
    fn named_locus(&self, ty: &TypeExpr) -> Option<String> {
        let mut cur = ty;
        for _ in 0..16 {
            match cur {
                TypeExpr::Projection { inner, .. } => cur = inner,
                TypeExpr::Named { path, .. } => {
                    let name = self.path_name(&path.segments)?;
                    if let Some(next) = self.aliases.get(&name) {
                        cur = next;
                        continue;
                    }
                    return self.loci.contains(&name).then_some(name);
                }
                _ => return None,
            }
        }
        None
    }

    /// The struct `ty` names, its aliases followed; `None` for any other
    /// type.
    fn named_struct(&self, ty: &TypeExpr) -> Option<String> {
        let mut cur = ty;
        for _ in 0..16 {
            match cur {
                TypeExpr::Projection { inner, .. } => cur = inner,
                TypeExpr::Named { path, .. } => {
                    let name = self.path_name(&path.segments)?;
                    if let Some(next) = self.aliases.get(&name) {
                        cur = next;
                        continue;
                    }
                    return self.structs.contains(&name).then_some(name);
                }
                _ => return None,
            }
        }
        None
    }

    /// The class of the field `field` of the struct `st` (every
    /// declaration the name answers to agreeing), or `None`.
    fn struct_field_class(&self, st: &str, field: &str) -> Option<Class> {
        let classes: Vec<Class> =
            self.struct_fields.get(st)?.iter().filter(|(f, _)| f == field).map(|(_, t)| self.classify(t)).collect();
        classes.first().copied().filter(|c| classes.iter().all(|x| x == c))
    }

    /// A declared name's class (aliases already followed).
    fn classify_name(&self, name: &str) -> Class {
        if self.form_loci.contains(name) {
            Class::Form(self.view_form_loci.contains(name))
        } else if self.loci.contains(name) {
            Class::Handle
        } else if self.structs.contains(name) {
            Class::Record
        } else if let Some(base) = self.scalar_bases.get(name) {
            match self.classify(base) {
                Class::Scalar => Class::Scalar,
                _ => Class::Value,
            }
        } else if self.records.contains(name) || self.enums.contains(name) {
            Class::Value
        } else {
            Class::Unknown
        }
    }

    /// The class a literal or constructor path names, its aliases
    /// followed.
    /// The struct a literal's or constructor's path names, its aliases
    /// followed.
    fn struct_of_path(&self, path: &QualifiedName) -> Option<String> {
        let name = self.path_name(&path.segments)?;
        match self.aliases.get(&name) {
            Some(t) => self.named_struct(t),
            None => self.structs.contains(&name).then_some(name),
        }
    }

    /// Is the field `field` of the struct `st` surely a primitive or a
    /// handle (nothing a write into the struct's field path could reach
    /// through it)?
    fn plain_field(&self, st: Option<&str>, field: &str) -> bool {
        let class = st.and_then(|st| self.struct_field_class(st, field));
        match class {
            Some(c) => matches!(c, Class::Scalar | Class::Handle | Class::Form(_)),
            None => self.scalar_fields.contains(field) || self.handle_fields.contains(field),
        }
    }

    fn classify_path(&self, path: &QualifiedName) -> Class {
        let Some(name) = self.path_name(&path.segments) else { return Class::Unknown };
        match self.aliases.get(&name) {
            Some(t) => self.classify(t),
            None => self.classify_name(&name),
        }
    }
}

/// The method names of the `@form` loci, synthesized by lowering (no
/// body): what they write is the field they are called on.
/// The bare `bounded[T; N]` intrinsics, whose first argument is the
/// bounded field they write (`push(p.path_vals, v)`).
const BOUNDED_OPS: &[&str] = &["count", "clear", "truncate", "push", "at", "pop"];

const FORM_OPS: &[&str] = &[
    "get", "set", "pop", "push", "remove", "insert", "contains", "has", "len", "is_empty", "is_full", "key_at",
    "entry_at", "sort", "bump", "put", "clear",
];

/// What a call's target is, as far as the pass can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// A fn or method of the program, with a body and so a summary.
    Fn(String),
    Method(String),
    /// A builtin or stdlib leaf: writes nothing of the program's (or
    /// what the bus handlers write, when it publishes or parks).
    Std,
    /// An enum variant: builds a value holding its arguments.
    Ctor,
    /// A form's synthesized method.
    FormOp,
    /// A bare `bounded[T; N]` intrinsic: writes its first argument.
    Bounded,
    /// Nothing the pass knows: may write anything, may return a handle.
    Unknown,
}

impl Bodies<'_> {
    /// The target of a call through `callee` (not a local fn value);
    /// `on_form`: its receiver is known to be an `@form` locus, whose
    /// synthesized methods take a name before any method of another
    /// locus that shares it.
    fn target(&self, callee: &Expr, on_form: bool) -> Target {
        use crate::builtin_sigs::bare_builtin_sig;
        use crate::stdlib_surface::{builtin_effects, row};
        match callee {
            Expr::Ident(id) => {
                let n = id.name.as_str();
                if self.fns.contains_key(n) {
                    Target::Fn(n.to_string())
                } else if BOUNDED_OPS.contains(&n) {
                    Target::Bounded
                } else if self.variants.contains(n) {
                    Target::Ctor
                } else if bare_builtin_sig(n).is_some() || builtin_effects(n).is_some() || row(&[n]).is_some() {
                    Target::Std
                } else {
                    Target::Unknown
                }
            }
            Expr::Path(q) => {
                let segs: Vec<&str> = q.segments.iter().map(|s| s.name.as_str()).collect();
                if self.is_ctor(q) {
                    return Target::Ctor;
                }
                if segs.first() == Some(&"std") || row(&segs).is_some() {
                    return Target::Std;
                }
                match self.path_name(&q.segments) {
                    Some(n) if self.fns.contains_key(&n) => Target::Fn(n),
                    // `Type::method(..)`
                    _ if self.methods.contains_key(*segs.last().unwrap_or(&"")) => {
                        Target::Method(segs.last().unwrap_or(&"").to_string())
                    }
                    _ => Target::Unknown,
                }
            }
            Expr::Field { name, .. } | Expr::Path2 { name, .. } => {
                if on_form && FORM_OPS.contains(&name.name.as_str()) {
                    Target::FormOp
                } else if self.fn_fields.contains(&name.name) {
                    // A fn-valued (or unclassified) field of that name: a
                    // value call, whatever methods share the name.
                    Target::Unknown
                } else if self.methods.contains_key(&name.name) {
                    Target::Method(name.name.clone())
                } else {
                    // A form's synthesized method only on a receiver known
                    // to hold a form: by its name alone it may be a record's
                    // fn-valued member, or anything else.
                    Target::Unknown
                }
            }
            _ => Target::Unknown,
        }
    }

    /// May a call of `t` return a handle?
    fn returns_handle(&self, t: &Target) -> bool {
        match t {
            Target::Fn(n) | Target::Method(n) => self.handle_fns.contains(n),
            Target::Std | Target::Ctor | Target::FormOp | Target::Bounded => false,
            Target::Unknown => true,
        }
    }
}

/// What a type is, as the pass needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A scalar, String or Bytes, enum, tuple or array: a value the
    /// frame holds (its fields view nothing another write replaces).
    Value,
    /// A struct: a value, which a `let` from a place binds as a copy.
    Record,
    /// A primitive (a number, a `Bool`, a `String`, `Bytes`): a value
    /// nothing writes into, only replaces.
    Scalar,
    /// A value laid out inline in the record holding it (a fixed array
    /// of scalars, a bounded list): a write into it writes the record's
    /// own bytes.
    Inline,
    /// A locus, an interface, a perspective: its field reads view the
    /// locus's storage.
    Handle,
    /// An `@form` locus: a handle whose reads may hand back a cell;
    /// `true`, one whose reads leave the cell's nested blocks shared.
    Form(bool),
    /// A fn type: calling one may run anything.
    FnValue,
    /// A type the pass cannot place: treated as a handle, and called as
    /// a value it may run anything.
    Unknown,
}

impl Class {
    fn may_be_handle(self) -> bool {
        matches!(self, Class::Handle | Class::Form(_) | Class::Unknown)
    }
    /// May a value of this type be a form whose reads leave the cell's
    /// nested blocks shared?
    fn may_be_view_form(self) -> bool {
        matches!(self, Class::Form(true) | Class::Unknown)
    }
    fn is_form(self) -> bool {
        matches!(self, Class::Form(_))
    }
    fn may_be_fn(self) -> bool {
        matches!(self, Class::FnValue | Class::Unknown)
    }
}

fn collect<'a>(items: &'a [TopDecl], p: &mut Bodies<'a>) {
    for item in items {
        match item {
            TopDecl::Fn(f) => {
                // An imported seed's fn is minted under a mangled name and
                // called by its own (`lib::name`, `name`).
                for n in decl_names(&f.name.name) {
                    p.fns.entry(n.clone()).or_default().push(p.bodies.len());
                    p.returns.push((n, f.ret.as_ref()));
                }
                p.body_ret.resize(p.bodies.len(), None);
                p.body_ret.push(f.ret.as_ref());
                p.bodies.push((&f.params, &f.body));
            }
            TopDecl::Interface(i) => {
                p.loci.extend(decl_names(&i.name.name));
            }
            TopDecl::Locus(l) => {
                p.loci.extend(decl_names(&l.name.name));
                if let Some(form) = &l.form {
                    p.form_loci.extend(decl_names(&l.name.name));
                    // A map that reads in place (no lock a reader takes)
                    // and an LRU cache hand back the cell's nested blocks.
                    use crate::form_rows::{sync_config, Discipline, SyncConfig};
                    let in_place = match form.name.name.as_str() {
                        "hashmap" => !matches!(sync_config(form), SyncConfig::Explicit(d) if d != Discipline::None),
                        "lru_cache" => true,
                        _ => false,
                    };
                    if in_place {
                        p.view_form_loci.extend(decl_names(&l.name.name));
                    }
                }
                for m in &l.members {
                    if let LocusMember::Capacity(c) = m {
                        p.key_fields.extend(c.slots.iter().filter_map(|s| s.indexed_by.as_ref().map(|i| i.name.clone())));
                    }
                    match m {
                        LocusMember::Fn(f) => {
                            p.methods.entry(f.name.name.clone()).or_default().push(p.bodies.len());
                            for n in decl_names(&l.name.name) {
                                p.locus_methods.entry((n, f.name.name.clone())).or_default().push(p.bodies.len());
                            }
                            p.body_locus.resize(p.bodies.len(), None);
                            p.body_locus.push(Some(l.name.name.clone()));
                            p.body_ret.resize(p.bodies.len(), None);
                            p.body_ret.push(f.ret.as_ref());
                            p.bodies.push((&f.params, &f.body));
                            p.returns.push((f.name.name.clone(), f.ret.as_ref()));
                        }
                        LocusMember::Lifecycle(d) => {
                            p.body_locus.resize(p.bodies.len(), None);
                            p.body_locus.push(Some(l.name.name.clone()));
                            p.bodies.push((&d.params, &d.body))
                        }
                        LocusMember::Mode(d) => p.bodies.push((&d.params, &d.body)),
                        LocusMember::Failure(d) => p.bodies.push((&d.params, &d.body)),
                        LocusMember::Params(pb) => {
                            for d in &pb.params {
                                p.field_types.push((d.name.name.clone(), d.ty.as_ref()));
                            }
                        }
                        LocusMember::Bus(b) => {
                            for bm in &b.members {
                                if let BusMember::Subscribe { handler, .. } = bm {
                                    p.handlers.insert(handler.name.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            TopDecl::Perspective(d) => {
                p.loci.extend(decl_names(&d.name.name));
                for m in &d.members {
                    if let PerspectiveMember::Fn(f) = m {
                        p.methods.entry(f.name.name.clone()).or_default().push(p.bodies.len());
                        p.bodies.push((&f.params, &f.body));
                        p.returns.push((f.name.name.clone(), f.ret.as_ref()));
                    }
                }
            }
            TopDecl::Module(m) => collect(&m.items, p),
            TopDecl::Type(td) => {
                match &td.body {
                    TypeDeclBody::Enum(vs) => {
                        p.variants.extend(vs.iter().map(|v| v.name.name.clone()));
                        p.enums.extend(decl_names(&td.name.name));
                    }
                    TypeDeclBody::Struct(fields) => {
                        p.records.extend(decl_names(&td.name.name));
                        p.structs.extend(decl_names(&td.name.name));
                        // A record's members are fields a call
                        // `x.name(..)` may run as a value, like a locus's.
                        for f in fields {
                            p.field_types.push((f.name.name.clone(), Some(&f.ty)));
                        }
                        for n in decl_names(&td.name.name) {
                            p.struct_fields
                                .entry(n)
                                .or_default()
                                .extend(fields.iter().map(|f| (f.name.name.clone(), &f.ty)));
                        }
                    }
                    TypeDeclBody::Scalar(sd) => {
                        p.records.extend(decl_names(&td.name.name));
                        p.scalar_decls.push((td.name.name.as_str(), sd));
                        for n in decl_names(&td.name.name) {
                            p.scalar_bases.insert(n, &sd.base);
                        }
                    }
                    TypeDeclBody::Alias(_) => {}
                }
                if let TypeDeclBody::Alias(t) = &td.body {
                    for n in decl_names(&td.name.name) {
                        p.aliases.insert(n, t);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The flagged sites, each with the name it binds (the first name of a
/// tuple `let`, a match's first payload binding): for the tests.
#[cfg(test)]
fn owned_view_sites(program: &Program) -> Vec<(Site, String)> {
    owned_view_sites_with(program, &BTreeMap::new())
}

#[cfg(test)]
fn owned_view_sites_with(program: &Program, renames: &BTreeMap<Vec<String>, String>) -> Vec<(Site, String)> {
    analyze(&[program], renames).sites
}

/// What the pass finds: the owned sites, and the cell-read rule's copies.
#[derive(Default)]
struct Found {
    sites: Vec<(Site, String)>,
    cow_writes: HashMap<u32, String>,
    cow_binds: HashMap<(Site, String), Vec<(String, bool)>>,
    cell_copies: Vec<CellCopy>,
    cell_owned: HashSet<u32>,
}

/// Per body, what its calls reach, transitively: the fields it may
/// write, whether it may read a form's cell, whether it may write
/// through a parameter.
struct Summaries {
    writes: Vec<Writes>,
    reads: Vec<bool>,
    writes_params: Vec<bool>,
    /// Per body, which of its parameters (a bit by position; the last
    /// bit stands for every one past it) it may write through by an
    /// assignment rooted at the parameter itself, or by handing it to a
    /// callee that may write through that one, transitively. A local
    /// bound from a parameter is not the parameter (it is copied at its
    /// first write, the local-place rule), so it writes none.
    param_masks: Vec<u64>,
    /// Per body, whether what it returns may be (or hold) a block a
    /// caller's name reaches: a parameter's part, a self field's, a form
    /// cell's, or what a callee that does returns. A call of one hands
    /// back a place's value, not a fresh one.
    ret_views: Vec<RetView>,
}

fn analyze(programs: &[&Program], renames: &BTreeMap<Vec<String>, String>) -> Found {
    let mut p = Bodies {
        bodies: Vec::new(),
        body_program: Vec::new(),
        methods: BTreeMap::new(),
        fns: BTreeMap::new(),
        fn_fields: BTreeSet::new(),
        handlers: BTreeSet::new(),
        form_loci: BTreeSet::new(),
        records: BTreeSet::new(),
        enums: BTreeSet::new(),
        structs: BTreeSet::new(),
        record_fields: BTreeSet::new(),
        scalar_fields: BTreeSet::new(),
        compound_fields: BTreeSet::new(),
        handle_fields: BTreeSet::new(),
        field_types: Vec::new(),
        aliases: BTreeMap::new(),
        renames: renames.clone(),
        variants: BTreeSet::new(),
        form_fields: BTreeSet::new(),
        form_only_fields: BTreeSet::new(),
        view_form_loci: BTreeSet::new(),
        view_form_fields: BTreeSet::new(),
        key_fields: BTreeSet::new(),
        inline_fields: BTreeSet::new(),
        scalar_decls: Vec::new(),
        scalar_bases: BTreeMap::new(),
        struct_fields: BTreeMap::new(),
        locus_methods: BTreeMap::new(),
        body_locus: Vec::new(),
        body_ret: Vec::new(),
        int_scalars: BTreeSet::new(),
        loci: BTreeSet::new(),
        returns: Vec::new(),
        handle_fns: BTreeSet::new(),
        scalar_fns: BTreeSet::new(),
        plain_fns: BTreeSet::new(),
        field_ids: std::cell::RefCell::new(BTreeMap::new()),
    };
    for (i, program) in programs.iter().enumerate() {
        collect(&program.items, &mut p);
        p.body_program.resize(p.bodies.len(), i);
    }
    let scalars: BTreeMap<&str, &ScalarDecl> = p.scalar_decls.iter().copied().collect();
    p.int_scalars = crate::units::typed_scalar_names(&scalars).into_iter().flat_map(decl_names).collect();
    // Every classification below reads `classify`: a fn or method name
    // some body of which may return a handle; a field a call may run as
    // a value; a field holding a form. A field with no declared type
    // (inferred from its default) is unclassified.
    let handle_fns: BTreeSet<String> = p
        .returns
        .iter()
        .filter(|(_, r)| r.is_some_and(|t| p.classify(t).may_be_handle()))
        .map(|(n, _)| n.clone())
        .collect();
    let classes: Vec<(String, Class)> = p
        .field_types
        .iter()
        .map(|(f, t)| (f.clone(), t.map_or(Class::Unknown, |t| p.classify(t))))
        .collect();
    p.handle_fns = handle_fns;
    let not_scalar_fn: BTreeSet<&String> =
        p.returns.iter().filter(|(_, r)| !r.is_some_and(|t| p.classify(t) == Class::Scalar)).map(|(n, _)| n).collect();
    p.scalar_fns =
        p.returns.iter().filter(|(n, _)| !not_scalar_fn.contains(n)).map(|(n, _)| n.clone()).collect();
    let not_plain_fn: BTreeSet<&String> = p
        .returns
        .iter()
        .filter(|(_, r)| r.is_some_and(|t| !matches!(p.classify(t), Class::Scalar | Class::Handle | Class::Form(_))))
        .map(|(n, _)| n)
        .collect();
    p.plain_fns = p.returns.iter().filter(|(n, _)| !not_plain_fn.contains(n)).map(|(n, _)| n.clone()).collect();
    p.fn_fields = classes.iter().filter(|(_, c)| c.may_be_fn()).map(|(f, _)| f.clone()).collect();
    p.form_fields = classes.iter().filter(|(_, c)| c.is_form() || *c == Class::Unknown).map(|(f, _)| f.clone()).collect();
    p.view_form_fields = classes.iter().filter(|(_, c)| c.may_be_view_form()).map(|(f, _)| f.clone()).collect();
    let not_form: BTreeSet<&String> = classes.iter().filter(|(_, c)| !c.is_form()).map(|(f, _)| f).collect();
    p.form_only_fields =
        classes.iter().filter(|(f, c)| c.is_form() && !not_form.contains(f)).map(|(f, _)| f.clone()).collect();
    let not_record: BTreeSet<&String> = classes.iter().filter(|(_, c)| *c != Class::Record).map(|(f, _)| f).collect();
    p.record_fields =
        classes.iter().filter(|(f, c)| *c == Class::Record && !not_record.contains(f)).map(|(f, _)| f.clone()).collect();
    let not_scalar: BTreeSet<&String> = classes.iter().filter(|(_, c)| *c != Class::Scalar).map(|(f, _)| f).collect();
    p.scalar_fields =
        classes.iter().filter(|(f, c)| *c == Class::Scalar && !not_scalar.contains(f)).map(|(f, _)| f.clone()).collect();
    let not_compound: BTreeSet<&String> = classes.iter().filter(|(_, c)| *c != Class::Value).map(|(f, _)| f).collect();
    p.compound_fields =
        classes.iter().filter(|(f, c)| *c == Class::Value && !not_compound.contains(f)).map(|(f, _)| f.clone()).collect();
    let not_handle: BTreeSet<&String> =
        classes.iter().filter(|(_, c)| !matches!(c, Class::Handle | Class::Form(_))).map(|(f, _)| f).collect();
    p.handle_fields = classes
        .iter()
        .filter(|(f, c)| matches!(c, Class::Handle | Class::Form(_)) && !not_handle.contains(f))
        .map(|(f, _)| f.clone())
        .collect();
    let not_inline: BTreeSet<&String> = classes.iter().filter(|(_, c)| *c != Class::Inline).map(|(f, _)| f).collect();
    p.inline_fields =
        classes.iter().filter(|(f, c)| *c == Class::Inline && !not_inline.contains(f)).map(|(f, _)| f.clone()).collect();

    // Each body's own writes and the calls it makes, then the fixpoint.
    struct Direct {
        writes: Writes,
        callees: BTreeSet<Callee>,
        reads: bool,
        writes_params: bool,
        param_callees: BTreeSet<Callee>,
        param_mask: u64,
        param_passes: Vec<(Callee, usize, usize)>,
        ret_src: Src,
        ret_marks: BTreeMap<String, (Vec<usize>, Vec<Src>)>,
    }
    let mut direct: Vec<Direct> = Vec::with_capacity(p.bodies.len());
    p.body_locus.resize(p.bodies.len(), None);
    p.body_ret.resize(p.bodies.len(), None);
    for (b, (params, body)) in p.bodies.iter().enumerate() {
        let mut w = Walk::new(&p, None, params);
        w.cur_locus = p.body_locus[b].clone();
        let tail = w.block(body);
        if let Some(t) = &body.tail {
            w.note_return(t, tail);
        }
        let ret_src = w.result_views();
        let ret_marks = std::mem::take(&mut w.ret_marks);
        direct.push(Direct {
            writes: w.own,
            callees: w.callees,
            reads: w.reads,
            writes_params: w.writes_params,
            param_callees: w.param_callees,
            param_mask: w.param_mask,
            param_passes: w.param_passes,
            ret_src,
            ret_marks,
        });
    }
    // Each body's callees, resolved once (a method name to every body
    // declaring it), then unioned until nothing changes.
    let resolve_all = |callees: &BTreeSet<Callee>| -> Vec<usize> {
        let mut e: Vec<usize> = callees.iter().flat_map(|c| p.resolve(c)).collect();
        e.sort_unstable();
        e.dedup();
        e
    };
    let edges: Vec<Vec<usize>> = direct.iter().map(|d| resolve_all(&d.callees)).collect();
    let param_edges: Vec<Vec<usize>> = direct.iter().map(|d| resolve_all(&d.param_callees)).collect();
    let mut reads: Vec<bool> = direct.iter().map(|d| d.reads).collect();
    let mut writes_params: Vec<bool> = direct.iter().map(|d| d.writes_params).collect();
    let mut param_masks: Vec<u64> = direct.iter().map(|d| d.param_mask).collect();
    let mut ret_views: Vec<RetView> = vec![RetView::default(); direct.len()];
    let param_passes: Vec<Vec<(Vec<usize>, usize, usize)>> = direct
        .iter()
        .map(|d| d.param_passes.iter().map(|(c, k, m)| (p.resolve(c), *k, *m)).collect())
        .collect();
    let mut summary: Vec<Writes> = direct.iter().map(|d| d.writes.clone()).collect();
    loop {
        let mut changed = false;
        for i in 0..summary.len() {
            if !reads[i] && edges[i].iter().any(|&j| reads[j]) {
                reads[i] = true;
                changed = true;
            }
            if !writes_params[i] && param_edges[i].iter().any(|&j| writes_params[j]) {
                writes_params[i] = true;
                changed = true;
            }
            let next = eval_src(&direct[i].ret_src, &direct[i].ret_marks, &ret_views, 0);
            let cur = ret_views[i];
            let merged = RetView {
                params: cur.params | next.params,
                other: cur.other || next.other,
                pparams: cur.pparams | next.pparams,
                pother: cur.pother || next.pother,
            };
            if merged != cur {
                ret_views[i] = merged;
                changed = true;
            }
            for (bodies, k, m) in &param_passes[i] {
                let bit = param_bit(*m);
                if param_masks[i] & bit == 0 && bodies.iter().any(|&j| param_masks[j] & param_bit(*k) != 0) {
                    param_masks[i] |= bit;
                    changed = true;
                }
            }
            if summary[i].top {
                continue;
            }
            let mut next = summary[i].clone();
            for &j in &edges[i] {
                if j != i {
                    next.add(&summary[j]);
                    if next.top {
                        break;
                    }
                }
            }
            if next != summary[i] {
                summary[i] = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let summaries = Summaries { writes: summary, reads, writes_params, param_masks, ret_views };

    let mut out = Found::default();
    for (b, (params, body)) in p.bodies.iter().enumerate() {
        let mut w = Walk::new(&p, Some(&summaries), params);
        w.cur_locus = p.body_locus[b].clone();
        w.block(body);
        let before = out.cell_copies.len();
        w.finish(&mut out);
        for c in &mut out.cell_copies[before..] {
            c.program = p.body_program[b];
        }
    }
    out
}

/// What `src` may be, over the callees' current summaries: its own
/// parameter bits and other, and for each call it may be the result of,
/// what that call's callees return — a parameter of theirs read as the
/// argument it was handed there.
fn eval_src(src: &Src, marks: &BTreeMap<String, (Vec<usize>, Vec<Src>)>, views: &[RetView], depth: usize) -> RetView {
    let mut out = RetView { params: src.params, other: src.other, pparams: src.pparams, pother: src.pother };
    if depth > 8 {
        out.other |= !src.calls.is_empty();
        return out;
    }
    for mark in &src.calls {
        let Some((bodies, args)) = marks.get(mark) else {
            out.other = true;
            continue;
        };
        for &j in bodies {
            let rv = views[j];
            out.other |= rv.other;
            out.pother |= rv.pother;
            for (k, arg) in args.iter().enumerate() {
                let (whole, part) = (rv.params & param_bit(k) != 0, rv.pparams & param_bit(k) != 0);
                if !(whole || part) {
                    continue;
                }
                let a = eval_src(arg, marks, views, depth + 1);
                if whole {
                    out.params |= a.params;
                    out.other |= a.other;
                    out.pparams |= a.pparams;
                    out.pother |= a.pother;
                } else {
                    // Returned as a part of a fresh value: whatever the
                    // argument was, it is only a part now.
                    out.pparams |= a.params | a.pparams;
                    out.pother |= a.other || a.pother;
                }
            }
        }
    }
    out
}

/// A parameter's bit in a body's parameter mask: one per position, the
/// last standing for every parameter past it.
fn param_bit(i: usize) -> u64 {
    1u64 << i.min(63)
}

impl Bodies<'_> {
    fn resolve(&self, c: &Callee) -> Vec<usize> {
        let m = match c {
            Callee::Method(n) => self.methods.get(n),
            Callee::Fn(n) => self.fns.get(n),
            Callee::Deliveries => {
                return self.handlers.iter().filter_map(|h| self.methods.get(h)).flatten().copied().collect();
            }
        };
        m.cloned().unwrap_or_default()
    }
}

/// One binding of a variable: its site, the place keys it views, and
/// where it was made.
struct Inst {
    var: usize,
    site: Site,
    name: String,
    keys: BTreeSet<String>,
    at: u32,
    /// A literal's binding: the parts that view a place, copied where
    /// the literal is built instead of the literal copied whole.
    parts: Option<Vec<Site>>,
    /// The binding's statement, where a copy it binds is reported.
    span: Span,
    /// A `let` or a bare `=` of a place (a local, a field, an
    /// element): a struct bound there is a copy already (GH #713,
    /// GH #992).
    place_rhs: bool,
    /// A literal's binding: the parts that hold any place's value,
    /// which the local-place rule copies where the literal is built
    /// when it owns the binding.
    place_parts: Option<Vec<Site>>,
}

/// The marker key of a form cell's read: the value's top level is the
/// reader's own, its nested blocks the cell's. Followed by the form's
/// place and, after `KEY_SEP`, the read's key (a local, by its
/// variable) or `-`. No field name contains it.
const CELL: &str = "\u{1}cell\u{1}";
/// The marker key of a value inside a cell's blocks (a part of a
/// read's value, an element, a payload, an iteration's entry): all of
/// it is the cell's. Followed by the form's place.
const IN_CELL: &str = "\u{1}in\u{1}";
const KEY_SEP: char = '\u{2}';
/// The marker key of a value that may be (or hold) a block another
/// name reaches: a place's value read without a copy — a field, an
/// element, a local, a parameter — or what carries one out (a `match`
/// payload, a literal holding it, an `if` or block result). Followed by
/// the place spelled as a path (`self.r.names`, `@3` a local, `$p` a
/// parameter), or `?`. Unlike the field-name keys it says nothing about
/// what a later write may REPLACE: only that a write INTO the value
/// would land in a block someone else reads (the local-place rule).
const AT: &str = "\u{1}at\u{1}";

/// The marker key of a struct literal's field that holds a place's
/// value (`R { inner: self.inner }`): the literal is fresh, its top level
/// the frame's own, but a write below that field lands in the place's
/// block. Followed by the field's name and, after `KEY_SEP`, the place
/// (as `AT` spells it), one marker per place the init may hold.
const PART: &str = "\u{1}part\u{1}";

/// A value's part markers: each field and the place it may hold.
fn part_markers(keys: &BTreeSet<String>) -> impl Iterator<Item = (&str, &str)> {
    keys.iter().filter_map(|k| {
        let rest = k.strip_prefix(PART)?;
        let mut it = rest.splitn(2, KEY_SEP);
        Some((it.next().unwrap_or(""), it.next().unwrap_or("?")))
    })
}


/// Where a value may come from, for a body's return summary: which of
/// the body's parameters it may be (a part of), whether it may be
/// anything else a caller's name reaches (a self field's, a cell's, a
/// handle's field), and which calls' results it may be (by their result
/// markers, resolved against the callees' summaries).
#[derive(Debug, Clone, Default, PartialEq)]
struct Src {
    params: u64,
    other: bool,
    /// The same, for a value that is fresh at its top level and only
    /// holds such a block in a part (a struct literal's field).
    pparams: u64,
    pother: bool,
    calls: BTreeSet<String>,
}

/// What a body's result may be (`Summaries::ret_views`): a part of the
/// parameters by these bits, or something else of the caller's — the
/// result itself (`params`, `other`), or only a part of a result fresh
/// at its top level (`pparams`, `pother`: a write into its own fields is
/// the caller's, one below a field may not be).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct RetView {
    params: u64,
    other: bool,
    pparams: u64,
    pother: bool,
}

/// The marker key of a call's result, on the summary pass: which
/// callee it came from, so a body that returns it returns what that
/// callee does (`Summaries::ret_views`). Followed by the callee.
const RET: &str = "\u{1}ret\u{1}";

fn is_place_marker(k: &str) -> bool {
    k.starts_with(AT) || k.starts_with(PART) || k.starts_with(RET)
}

/// A struct literal's keys with its place markers turned into part
/// markers: one per field whose init may hold a place's value.
fn literal_part_markers(keys: Keys, parts: &[(String, Vec<String>)]) -> Keys {
    // The real keys stay (a cell's markers among them); the whole
    // literal's place markers become its parts'.
    let mut ks: BTreeSet<String> = real(&keys).unwrap_or_default();
    for (f, places) in parts {
        ks.extend(places.iter().map(|q| format!("{PART}{f}{KEY_SEP}{q}")));
    }
    (!ks.is_empty()).then_some(ks)
}

/// The keys without their place markers: what the replaced-view
/// analysis and the cell-read rule read, as before the local-place
/// rule added the markers.
fn real(keys: &Keys) -> Keys {
    keys.as_ref().and_then(|ks| {
        let ks: BTreeSet<String> = ks.iter().filter(|k| !is_place_marker(k)).cloned().collect();
        (!ks.is_empty()).then_some(ks)
    })
}

/// The place marker of `place` (`None`: an unnamed place).
fn at(place: Option<String>) -> Keys {
    Some(BTreeSet::from([format!("{AT}{}", place.unwrap_or_else(|| "?".to_string()))]))
}

/// Does the value carry a place marker?
fn has_at(keys: &BTreeSet<String>) -> bool {
    keys.iter().any(|k| k.starts_with(AT))
}

/// The places a value's markers name.
fn at_places(keys: &BTreeSet<String>) -> impl Iterator<Item = &str> {
    keys.iter().filter_map(|k| k.strip_prefix(AT))
}

/// Do two places overlap: one is the other, or a part of it?
fn places_overlap(a: &str, b: &str) -> bool {
    let under = |x: &str, y: &str| x == y || x.strip_prefix(y).is_some_and(|r| r.starts_with('.'));
    a == "?" || b == "?" || under(a, b) || under(b, a)
}

/// The form methods whose result may be a cell of a form that reads in
/// place.
const CELL_READS: &[&str] = &["get", "entry_at"];
/// The form methods that read a cell's value (and so would see a write
/// made into it).
const VALUE_READS: &[&str] = &["get", "entry_at", "peek", "pop"];
/// The form methods that store a value as a cell.
const CELL_STORES: &[&str] = &["set", "put"];

/// A value's keys with every cell marker turned in-cell: a part of a
/// read's value is the cell's own block.
fn in_cell(keys: Keys) -> Keys {
    keys.map(|ks| {
        ks.into_iter()
            .map(|k| match k.strip_prefix(CELL) {
                Some(rest) => format!("{IN_CELL}{}", rest.split(KEY_SEP).next().unwrap_or("")),
                None => k,
            })
            .collect()
    })
}

/// A value's keys with its cell markers dropped: a record bound from a
/// local is copied (spec/types.md § A struct binding is a copy).
fn without_cells(keys: Keys) -> Keys {
    keys.and_then(|ks| {
        let ks: BTreeSet<String> = ks.into_iter().filter(|k| !k.starts_with(CELL)).collect();
        (!ks.is_empty()).then_some(ks)
    })
}

/// One binding of a variable, whatever it binds: where it is, whether a
/// flag can be set there (a `let` or a bare `=`), and its instance.
struct Bind {
    site: Site,
    at: u32,
    /// A `let` or a bare `=`: lowering can set a copy-on-write flag
    /// there.
    flaggable: bool,
    inst: Option<usize>,
}

/// An assignment under a local: `v.f.. = x` (`first` the field) or
/// `v[i].. = x` (`first` `None`), `depth` segments deep.
struct AssignWrite {
    var: usize,
    first: Option<String>,
    depth: usize,
    id: u32,
    span: Span,
    at: u32,
    /// The blocks the assignment is inside, innermost last.
    blocks: Vec<u32>,
}

/// What a form's store (`m.set(x)`, `c.put(x)`) is handed.
enum StoreArg {
    /// A local.
    Var(usize),
    /// A record literal: each init of a key field, as what it reads.
    Literal(Vec<KeyInit>),
    Other,
}

enum KeyInit {
    /// `id: v.id`: the same field of a local.
    FieldOf(usize),
    /// `id: k`: a local or a parameter, by its place.
    Var(String),
    Other,
}

/// A form's store: where, into which place, of what, and the block it
/// is a statement of (`None`: inside an expression).
struct StoreAt {
    at: u32,
    place: String,
    arg: StoreArg,
    block: Option<u32>,
}

/// A whole-value store of a local into a self field (`self.f = v;`):
/// where, the field's place, the local, and the block it is a
/// statement of.
struct SelfStore {
    at: u32,
    place: String,
    var: usize,
    block: Option<u32>,
}

/// A write event: the writes, and the positions it spans (a call's
/// arguments are inside it: a callee reads its parameters after it
/// writes).
struct WriteAt {
    writes: Writes,
    start: u32,
    at: u32,
}

struct Walk<'p, 'a> {
    p: &'p Bodies<'a>,
    /// `None` on the summary pass: collect `own` and `callees` only.
    summary: Option<&'p Summaries>,
    own: Writes,
    callees: BTreeSet<Callee>,
    pos: u32,
    scopes: Vec<BTreeMap<String, Option<usize>>>,
    /// Per variable, its instances.
    vars: Vec<Vec<usize>>,
    /// Per variable, whether it may hold a handle, whose field reads
    /// view a locus's storage; a record value of the frame's own is
    /// never replaced under a binding of its field.
    handles: Vec<bool>,
    /// The parameters that are handles.
    handle_params: BTreeSet<String>,
    /// Per variable, whether it holds an `@form` locus (bound from a
    /// form's literal or declared as one); and the form parameters.
    forms: Vec<bool>,
    form_params: BTreeSet<String>,
    /// Per variable, where it was declared: a loop that starts after
    /// it carries its value round the back edge, whichever instance of
    /// it holds the value.
    declared: Vec<u32>,
    insts: Vec<Inst>,
    /// Per variable, the positions it is read at.
    uses: Vec<Vec<u32>>,
    writes: Vec<WriteAt>,
    loops: Vec<(u32, u32)>,
    /// What `Callee::Deliveries` writes, once the summaries are in.
    deliveries: Option<Writes>,
    /// Arguments passed to a call that may replace what they view.
    args: Vec<(Site, String)>,
    /// The cell-read rule's facts. On the summary pass: whether the body
    /// itself may read a form's cell or write through a parameter, and
    /// the callees it passes a parameter to.
    reads: bool,
    writes_params: bool,
    param_callees: BTreeSet<Callee>,
    /// Every parameter's name.
    params: BTreeSet<String>,
    /// Per variable, whether it may hold (a part of) the caller's value a
    /// parameter refers to (`Walk::rooted_at_param`).
    from_param: Vec<bool>,
    /// Per variable, whether it may hold a form whose reads leave the
    /// cell's nested blocks shared; and such parameters.
    view_forms: Vec<bool>,
    view_form_params: BTreeSet<String>,
    /// Per variable, every binding of it.
    binds: Vec<Vec<Bind>>,
    /// The blocks being walked, innermost last, and the next block's id.
    block_stack: Vec<u32>,
    next_block: u32,
    /// The call a statement is (`m.set(c);`), by its span, while it is
    /// walked.
    stmt_call: Option<(u32, u32)>,
    assigns: Vec<AssignWrite>,
    stores: Vec<StoreAt>,
    /// Reads of a cell: where, and the form's place (`None`: any form).
    cell_reads: Vec<(u32, Option<String>)>,
    /// Where a path may leave: `true` a `break` or `continue`, `false` a
    /// `return`, `fail`, `violate`, `terminate` or `or raise`.
    exits: Vec<(u32, bool)>,
    /// Locals passed (whole or by a part, the part's depth) to a call
    /// that may write through its parameter.
    passed: Vec<(usize, usize)>,
    /// Arguments of no local copied for such a call, by span.
    passed_copies: Vec<Span>,
    /// Parameters rebound (`k = ..`), and where.
    param_rebinds: Vec<(String, u32)>,
    /// The local-place rule's facts. Per variable: its name, whether it
    /// is known to hold a struct, whether it surely holds a handle; the
    /// parameters that surely hold one, and those that hold a struct.
    names: Vec<String>,
    record_hint: Vec<bool>,
    handle_only: Vec<bool>,
    /// Per variable, whether it surely holds a primitive (nothing writes
    /// into one); and such parameters.
    scalar_hint: Vec<bool>,
    scalar_params: BTreeSet<String>,
    /// Per variable, whether it is known to hold an array, a tuple or an
    /// enum read from a place (a field every declaration of which holds
    /// one, or a local that does).
    compound_hint: Vec<bool>,
    /// Per variable, the struct it is known to hold (a literal's, a
    /// declared type's, a parameter's); and such parameters.
    var_struct: Vec<Option<String>>,
    param_structs: BTreeMap<String, String>,
    handle_only_params: BTreeSet<String>,
    record_params: BTreeSet<String>,
    /// Locals handed to a call that may write through its parameter,
    /// with whether that parameter is declared a struct.
    place_passed: Vec<(usize, bool, Option<String>)>,
    /// Every field read, where and its name; every call that may read
    /// the program's fields (a fn or method of it, one the pass cannot
    /// identify, a delivery), where.
    field_reads: Vec<(u32, String)>,
    opaque_calls: Vec<u32>,
    self_stores: Vec<SelfStore>,
    /// Writes into a local's or a parameter's value in place: the block
    /// written, spelled as the markers spell it, where, and whether
    /// anything below it may be written too (a call that may write
    /// through it) or only a slot of the block itself (an assignment).
    root_writes: Vec<(String, u32, bool)>,
    /// On the summary pass: the parameters the body writes through
    /// itself (`Summaries::param_masks`), and each parameter it hands to
    /// a callee (the callee, the argument's position, the parameter's).
    param_mask: u64,
    param_passes: Vec<(Callee, usize, usize)>,
    /// Each parameter's position.
    param_index: BTreeMap<String, usize>,
    /// On the summary pass: what each `return` (and the body's tail)
    /// hands back — its keys and the local it names — and the callee
    /// each result marker stands for.
    returns: Vec<(Keys, Option<usize>)>,
    ret_marks: BTreeMap<String, (Vec<usize>, Vec<Src>)>,
    /// The locus this body is a member of, and the parameters of a known
    /// locus type: a method call on `self` or on one of those resolves
    /// to that locus's method alone (for what its result may be).
    cur_locus: Option<String>,
    param_loci: BTreeMap<String, String>,
    var_locus: Vec<Option<String>>,
}

fn pattern_names<'e>(p: &'e Pattern, out: &mut Vec<&'e Ident>) {
    match p {
        Pattern::Binding(id) => out.push(id),
        Pattern::Constructor { args, .. } => args.iter().for_each(|a| pattern_names(a, out)),
        Pattern::Tuple(ps, _) => ps.iter().for_each(|a| pattern_names(a, out)),
        Pattern::Literal(..) | Pattern::Wildcard(_) => {}
    }
}

impl<'p, 'a> Walk<'p, 'a> {
    fn new(p: &'p Bodies<'a>, summary: Option<&'p Summaries>, params: &[Param]) -> Self {
        let mut top = BTreeMap::new();
        let mut handle_params = BTreeSet::new();
        let mut form_params = BTreeSet::new();
        let mut view_form_params = BTreeSet::new();
        let mut handle_only_params = BTreeSet::new();
        let mut record_params = BTreeSet::new();
        let mut scalar_params = BTreeSet::new();
        for prm in params {
            top.insert(prm.name.name.clone(), None);
            let class = p.classify(&prm.ty);
            if matches!(class, Class::Handle | Class::Form(_)) {
                handle_only_params.insert(prm.name.name.clone());
            }
            if class == Class::Record {
                record_params.insert(prm.name.name.clone());
            }
            if class == Class::Scalar {
                scalar_params.insert(prm.name.name.clone());
            }
            if class.may_be_handle() {
                handle_params.insert(prm.name.name.clone());
            }
            if class.is_form() {
                form_params.insert(prm.name.name.clone());
            }
            if class.may_be_view_form() {
                view_form_params.insert(prm.name.name.clone());
            }
        }
        Walk {
            p,
            summary,
            own: Writes::default(),
            callees: BTreeSet::new(),
            pos: 0,
            scopes: vec![top],
            vars: Vec::new(),
            handles: Vec::new(),
            handle_params,
            forms: Vec::new(),
            form_params,
            declared: Vec::new(),
            insts: Vec::new(),
            uses: Vec::new(),
            writes: Vec::new(),
            loops: Vec::new(),
            args: Vec::new(),
            deliveries: None,
            reads: false,
            writes_params: false,
            param_callees: BTreeSet::new(),
            params: params.iter().map(|prm| prm.name.name.clone()).collect(),
            from_param: Vec::new(),
            view_forms: Vec::new(),
            view_form_params,
            binds: Vec::new(),
            block_stack: Vec::new(),
            next_block: 0,
            stmt_call: None,
            assigns: Vec::new(),
            stores: Vec::new(),
            cell_reads: Vec::new(),
            exits: Vec::new(),
            passed: Vec::new(),
            passed_copies: Vec::new(),
            param_rebinds: Vec::new(),
            names: Vec::new(),
            record_hint: Vec::new(),
            handle_only: Vec::new(),
            scalar_hint: Vec::new(),
            scalar_params,
            compound_hint: Vec::new(),
            var_struct: Vec::new(),
            param_structs: params
                .iter()
                .filter_map(|prm| p.named_struct(&prm.ty).map(|s| (prm.name.name.clone(), s)))
                .collect(),
            handle_only_params,
            record_params,
            place_passed: Vec::new(),
            field_reads: Vec::new(),
            opaque_calls: Vec::new(),
            self_stores: Vec::new(),
            root_writes: Vec::new(),
            param_mask: 0,
            param_passes: Vec::new(),
            param_index: params.iter().enumerate().map(|(i, prm)| (prm.name.name.clone(), i)).collect(),
            returns: Vec::new(),
            ret_marks: BTreeMap::new(),
            cur_locus: None,
            var_locus: Vec::new(),
            param_loci: params
                .iter()
                .filter_map(|prm| p.named_locus(&prm.ty).map(|l| (prm.name.name.clone(), l)))
                .collect(),
        }
    }

    fn tick(&mut self) -> u32 {
        self.pos += 1;
        self.pos
    }

    /// `Some(Some(var))` a local, `Some(None)` a parameter (or a
    /// binding this pass does not track), `None` not a local at all.
    fn lookup(&self, name: &str) -> Option<Option<usize>> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn declare(&mut self, name: &str) -> usize {
        let v = self.vars.len();
        self.vars.push(Vec::new());
        self.handles.push(false);
        self.forms.push(false);
        self.view_forms.push(false);
        self.from_param.push(false);
        self.binds.push(Vec::new());
        self.declared.push(self.pos);
        self.uses.push(Vec::new());
        self.names.push(name.to_string());
        self.record_hint.push(false);
        self.handle_only.push(false);
        self.scalar_hint.push(false);
        self.compound_hint.push(false);
        self.var_struct.push(None);
        self.var_locus.push(None);
        self.scopes.last_mut().expect("a scope").insert(name.to_string(), Some(v));
        v
    }

    /// The keys every instance of the variable `v` views.
    fn var_keys(&self, v: usize) -> Keys {
        let mut all = BTreeSet::new();
        for &i in &self.vars[v] {
            all.extend(self.insts[i].keys.iter().cloned());
        }
        (!all.is_empty()).then_some(all)
    }

    /// A binding no flag can be set at (a `match` payload, a `for`
    /// element, a tuple `let`'s name).
    fn bind(&mut self, var: usize, site: Site, name: &str, keys: Option<BTreeSet<String>>, span: Span) {
        self.bind_parts(var, site, name, keys, (None, None), span, false, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_parts(
        &mut self,
        var: usize,
        site: Site,
        name: &str,
        keys: Option<BTreeSet<String>>,
        (parts, place_parts): (Option<Vec<Site>>, Option<Vec<Site>>),
        span: Span,
        flaggable: bool,
        place_rhs: bool,
    ) {
        let at = self.tick();
        let Some(keys) = keys else {
            self.binds[var].push(Bind { site, at, flaggable, inst: None });
            return;
        };
        // Read out of a place, it may be a child's handle. (A value
        // carrying only a place marker is the local-place rule's: it
        // views nothing a write may replace.)
        if keys.iter().any(|k| !is_place_marker(k)) {
            self.handles[var] = true;
        }
        let i = self.insts.len();
        self.insts.push(Inst { var, site, name: name.to_string(), keys, at, parts, span, place_rhs, place_parts });
        self.vars[var].push(i);
        self.binds[var].push(Bind { site, at, flaggable, inst: Some(i) });
    }

    /// The keys of `receiver.name`, the receiver's being `keys`: a field
    /// read views its field when the receiver views a place or holds a
    /// handle; a field of a record value the frame holds views nothing
    /// another write can replace (a write through a local root replaces
    /// the field's pointer, GH #993).
    ///
    /// Its place marker (the local-place rule): a field of a place is
    /// the place `receiver.name`; a field of any other value that may
    /// be shared is shared as that value is; a field every declaration
    /// of which holds a handle is the handle, and carries none.
    fn field_keys(&self, receiver: &Expr, keys: Keys, name: &Ident) -> Keys {
        let marker = if self.p.handle_fields.contains(&name.name) || self.p.scalar_fields.contains(&name.name) {
            None
        } else if let Some(rp) = self.place_of(receiver) {
            at(Some(format!("{rp}.{}", name.name)))
        } else {
            keys.as_ref().filter(|k| has_at(k)).map(|k| k.iter().filter(|k| k.starts_with(AT)).cloned().collect())
        };
        let keys = real(&keys);
        if keys.is_none() && !self.is_handle(receiver) {
            return marker;
        }
        let mut keys = in_cell(keys).unwrap_or_default();
        keys.insert(name.name.clone());
        union(Some(keys), marker)
    }

    /// A struct literal bound to a local: its top level is fresh, so its
    /// place markers become part markers, one per field whose init may
    /// hold a place's value (a field, an element, a local, a parameter,
    /// or a value that carries one out). Any other value's keys as given.
    fn struct_literal_keys(&self, value: &Expr, keys: Keys) -> Keys {
        let Expr::Struct { inits, path, .. } = value else { return keys };
        let st = self.p.struct_of_path(path);
        // Every place the literal as a whole may hold: what an init the
        // pass cannot read without walking may hold.
        let whole: Vec<String> = keys
            .iter()
            .flatten()
            .filter_map(|k| k.strip_prefix(AT).map(str::to_string))
            .chain(keys.iter().flatten().filter_map(|k| {
                k.strip_prefix(PART).map(|r| r.split(KEY_SEP).nth(1).unwrap_or("?").to_string())
            }))
            .collect();
        let places_of = |ks: &Keys| -> Vec<String> {
            ks.iter()
                .flatten()
                .filter_map(|k| match k.strip_prefix(AT) {
                    Some(p) => Some(p.to_string()),
                    None => k.strip_prefix(PART).map(|r| r.split(KEY_SEP).nth(1).unwrap_or("?").to_string()),
                })
                .collect()
        };
        let parts: Vec<(String, Vec<String>)> = inits
            .iter()
            .map(|i| {
                let plain = self.p.plain_field(st.as_deref(), &i.name.name);
                let places = match &i.value {
                    _ if plain => Vec::new(),
                    Expr::Ident(_) | Expr::Field { .. } | Expr::Index { .. } => places_of(&self.part_keys(&i.value)),
                    Expr::Literal(..) | Expr::Binary { .. } | Expr::Unary { .. } | Expr::Range { .. } => Vec::new(),
                    // A call's result is fresh unless the call may hand back a
                    // place's value (`?ret`): then whatever the literal holds.
                    Expr::Call { callee, .. } if !matches!(callee.as_ref(), Expr::Path(q) if self.p.is_ctor(q)) => {
                        if whole.iter().any(|q| q == "?ret") {
                            whole.clone()
                        } else {
                            Vec::new()
                        }
                    }
                    _ => whole.clone(),
                };
                (i.name.name.clone(), places)
            })
            .collect();
        literal_part_markers(keys, &parts)
    }

    /// The bodies a call of `c` may run, its receiver read: `self`'s
    /// locus's method, or a known locus type's, when the receiver says
    /// which; every body of the name otherwise.
    fn callee_bodies(&self, c: &Callee, receiver: Option<&Expr>) -> Vec<usize> {
        if let Callee::Method(m) = c {
            let locus = match receiver {
                Some(Expr::KwSelf(_)) => self.cur_locus.clone(),
                Some(Expr::Ident(id)) => match self.lookup(&id.name) {
                    Some(None) => self.param_loci.get(&id.name).cloned(),
                    Some(Some(v)) => self.var_locus[v].clone(),
                    None => None,
                },
                _ => None,
            };
            if let Some(bodies) = locus.and_then(|l| self.p.locus_methods.get(&(l, m.clone()))) {
                return bodies.clone();
            }
        }
        self.p.resolve(c)
    }

    /// What a `return` (or the body's tail) hands back, for the summary.
    fn note_return(&mut self, e: &Expr, keys: Keys) {
        if self.summary.is_some() {
            return;
        }
        // A literal is fresh: only its parts' place markers can reach a
        // caller's block (its real keys name fields read for a primitive
        // or handle part, which its part markers leave out).
        let keys = if let Expr::Struct { .. } = e {
            self.struct_literal_keys(e, keys).map(|ks| ks.into_iter().filter(|k| is_place_marker(k)).collect())
        } else {
            keys
        };
        let local = match e {
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => Some(v),
                _ => None,
            },
            _ => None,
        };
        self.returns.push((keys, local));
    }

    /// May what this body returns be a block a caller's name reaches,
    /// and which callees' results does it return? A local returned is
    /// judged by its instances: one copied at its binding (a struct `let`
    /// of a place) or a primitive returns nothing of the caller's; a
    /// local's own place (`@n`) is the frame's.
    fn result_views(&self) -> Src {
        let mut record = self.record_hint.clone();
        for w in &self.assigns {
            if w.first.as_ref().is_some_and(|f| !f.chars().all(|c| c.is_ascii_digit())) {
                record[w.var] = true;
            }
        }
        let mut out = Src::default();
        for (keys, local) in &self.returns {
            match local {
                Some(v) => {
                    if self.scalar_hint[*v] {
                        continue;
                    }
                    for &i in &self.vars[*v] {
                        let inst = &self.insts[i];
                        if record[*v] && inst.place_rhs {
                            continue;
                        }
                        self.src_of(&inst.keys, inst.place_parts.is_some(), &mut out);
                    }
                }
                None => {
                    if let Some(ks) = keys {
                        self.src_of(ks, false, &mut out);
                    }
                }
            }
        }
        out
    }

    /// Where a value with these keys may come from (`Src`); `literal`, a
    /// fresh literal's: only its markers count. A local's own place
    /// (`@n`) is the frame's (its sources are among the keys already). A
    /// locus's storage (`self`'s fields, a handle's) is copied into the
    /// caller's arena on the way out, so it is none of the caller's; a
    /// form cell's nested blocks are not (a read leaves them shared), nor
    /// is what an unidentified call returned.
    fn src_of(&self, ks: &BTreeSet<String>, literal: bool, out: &mut Src) {
        let place = |p: &str, part: bool, out: &mut Src| {
            if let Some(rest) = p.strip_prefix('$') {
                let name = rest.split('.').next().unwrap_or("");
                // A field read through a handle parameter is that locus's
                // storage, which a free fn hands back uncopied.
                if rest.contains('.') && self.handle_params.contains(name) {
                    if part {
                        out.pother = true;
                    } else {
                        out.other = true;
                    }
                    return;
                }
                match (self.param_index.get(name), part) {
                    (Some(&i), false) => out.params |= param_bit(i),
                    (Some(&i), true) => out.pparams |= param_bit(i),
                    (None, false) => out.other = true,
                    (None, true) => out.pother = true,
                }
            } else if p.starts_with('?') {
                if part {
                    out.pother = true;
                } else {
                    out.other = true;
                }
            }
        };
        for k in ks {
            if literal && !is_place_marker(k) {
                continue;
            }
            if k.starts_with(RET) {
                out.calls.insert(k.clone());
            } else if let Some(p) = k.strip_prefix(AT) {
                place(p, false, out);
            } else if let Some(r) = k.strip_prefix(PART) {
                place(r.split(KEY_SEP).nth(1).unwrap_or("?"), true, out);
            } else if k.starts_with(CELL) || k.starts_with(IN_CELL) {
                out.other = true;
            }
        }
    }

    /// `v.f.. = value` / `v[i].. = value`: when `value` may hold another
    /// name's block (a place marker, a part marker's place, a cell read),
    /// a new instance of `v` binds it — a part marker for the first field
    /// of the path, or a whole marker when the path starts with an index.
    /// Owning it copies `value` where the assignment evaluates it.
    fn bind_assigned_part(&mut self, v: usize, target: &LValue, value: &Expr, keys: &Keys, id: u32, span: Span) {
        if self.known_scalar(None, value) || self.surely_handle(None, value) {
            return;
        }
        if let Some(LValueSeg::Field(f)) = target.tail.first() {
            if target.tail.len() == 1 && self.p.plain_field(self.var_struct[v].as_deref(), &f.name) {
                return;
            }
        }
        let Some(ks) = keys else { return };
        let mut places: Vec<String> = ks
            .iter()
            .filter_map(|k| {
                if let Some(p) = k.strip_prefix(AT) {
                    Some(p.to_string())
                } else if let Some(r) = k.strip_prefix(PART) {
                    Some(r.split(KEY_SEP).nth(1).unwrap_or("?").to_string())
                } else if k.starts_with(CELL) || k.starts_with(IN_CELL) {
                    Some("?".to_string())
                } else {
                    None
                }
            })
            .collect();
        places.sort();
        places.dedup();
        if places.is_empty() {
            return;
        }
        if self.rooted_at_param(value) {
            self.from_param[v] = true;
        }
        let markers: BTreeSet<String> = match target.tail.first() {
            Some(LValueSeg::Field(f)) => places.iter().map(|q| format!("{PART}{}{KEY_SEP}{q}", f.name)).collect(),
            _ => places.iter().map(|q| format!("{AT}{q}")).collect(),
        };
        let all = markers;
        let vspan = value.span();
        let parts = (None, Some(vec![Site::Part(vspan.start.0, vspan.end.0)]));
        let name = self.names[v].clone();
        self.bind_parts(v, Site::Node(id), &name, Some(all), parts, span, true, false);
    }

    /// The field right below the local an argument is read from
    /// (`lit.arr[0]` → `arr`); `None` when the local is passed whole or
    /// indexed first.
    fn first_field(&self, a: &Expr) -> Option<String> {
        match a {
            Expr::Field { receiver, name, .. } if matches!(receiver.as_ref(), Expr::Ident(_)) => Some(name.name.clone()),
            Expr::Field { receiver, .. } | Expr::Index { receiver, .. } => self.first_field(receiver),
            _ => None,
        }
    }

    /// The local or parameter place an argument hands a callee (`r`,
    /// `r.names`, `r.rows[i]` as `r.rows`), spelled as the markers spell
    /// it; `None` for any other argument.
    fn arg_place(&self, a: &Expr) -> Option<String> {
        match a {
            Expr::Index { receiver, .. } => self.arg_place(receiver),
            Expr::Field { receiver, .. } if matches!(receiver.as_ref(), Expr::Index { .. }) => self.arg_place(receiver),
            other => self.place_of(other).filter(|p| !p.starts_with("self")),
        }
    }

    /// The place marker of a read of the local or parameter `name`
    /// (`None`: one that surely holds a handle, or no local at all).
    fn ident_at(&self, name: &str) -> Keys {
        match self.lookup(name) {
            Some(Some(v)) if !self.handle_only[v] && !self.scalar_hint[v] => at(Some(format!("@{v}"))),
            Some(None)
                if self.params.contains(name)
                    && !self.handle_only_params.contains(name)
                    && !self.scalar_params.contains(name) =>
            {
                at(Some(format!("${name}")))
            }
            _ => None,
        }
    }

    /// Does `value`, bound to a fresh local, surely hold a handle (its
    /// declared type, a field every declaration of which holds one, a
    /// local or a parameter that does, a locus literal)?
    fn surely_handle(&self, ty: Option<&TypeExpr>, value: &Expr) -> bool {
        if let Some(t) = ty {
            return matches!(self.p.classify(t), Class::Handle | Class::Form(_));
        }
        match value {
            Expr::Field { name, .. } => self.p.handle_fields.contains(&name.name),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.handle_only[v],
                Some(None) => self.handle_only_params.contains(&id.name),
                None => false,
            },
            Expr::Struct { path, .. } => matches!(self.p.classify_path(path), Class::Handle | Class::Form(_)),
            Expr::KwSelf(_) => true,
            _ => false,
        }
    }

    /// Is `value`, bound to a fresh local, surely a primitive (its
    /// declared type, a literal, an operator's result, a field every
    /// declaration of which holds one, a local or a parameter that is)?
    fn known_scalar(&self, ty: Option<&TypeExpr>, value: &Expr) -> bool {
        if let Some(t) = ty {
            return self.p.classify(t) == Class::Scalar;
        }
        match value {
            Expr::Literal(..) | Expr::Binary { .. } | Expr::Unary { .. } => true,
            Expr::Call { callee, args, .. } => self.call_returns_scalar(callee, args),
            Expr::Or { inner, disposition, .. } => {
                self.known_scalar(None, inner)
                    && match disposition {
                        OrDisposition::Substitute(x) => self.known_scalar(None, x),
                        _ => true,
                    }
            }
            Expr::Field { name, .. } => self.p.scalar_fields.contains(&name.name),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.scalar_hint[v],
                Some(None) => self.scalar_params.contains(&id.name),
                None => false,
            },
            // What an `if`, a `match` or a block carries out: a primitive
            // when every way out is one (an arm's payload names are out
            // of scope here, so an arm reading one is not known).
            Expr::If(i) => self.if_scalar(i),
            Expr::Block(b) => self.block_scalar(b),
            Expr::Match(m) => {
                !m.arms.is_empty()
                    && m.arms.iter().all(|a| match &a.body {
                        MatchArmBody::Expr(x) => self.known_scalar(None, x),
                        MatchArmBody::Block(b) => self.block_scalar(b),
                    })
            }
            _ => false,
        }
    }

    /// Does a call surely return a primitive: a builtin or a stdlib leaf
    /// whose signature says so, or a fn or method every body of which
    /// declares one (a typed scalar included)?
    fn call_returns_scalar(&self, callee: &Expr, args: &[Expr]) -> bool {
        use crate::builtin_sigs::{bare_builtin_sig, Returns};
        use crate::stdlib_surface::{signature_for, SigTy};
        if let Expr::Ident(id) = callee {
            if self.lookup(&id.name).is_some() {
                return false;
            }
            if let Some(sig) = bare_builtin_sig(&id.name) {
                return match sig.returns {
                    Returns::Prim(_) => true,
                    Returns::Operand => args.first().is_some_and(|a| self.known_scalar(None, a)),
                };
            }
        }
        if let Expr::Path(q) = callee {
            let segs: Vec<&str> = q.segments.iter().map(|s| s.name.as_str()).collect();
            if let Some(sig) = signature_for(&segs) {
                return matches!(
                    sig.ret,
                    SigTy::Int
                        | SigTy::Uint
                        | SigTy::Float
                        | SigTy::Bool
                        | SigTy::Str
                        | SigTy::Bytes
                        | SigTy::Decimal
                        | SigTy::Duration
                        | SigTy::Time
                );
            }
        }
        // A method on `self` or on a value of a known locus type is that
        // locus's, whatever a form's synthesized method of the name does.
        if let Expr::Field { receiver, name, .. } = callee {
            let c = Callee::Method(name.name.clone());
            let bodies = self.callee_bodies(&c, Some(receiver));
            let resolved = match receiver.as_ref() {
                Expr::KwSelf(_) => self.cur_locus.is_some(),
                Expr::Ident(id) => match self.lookup(&id.name) {
                    Some(None) => self.param_loci.contains_key(&id.name),
                    Some(Some(v)) => self.var_locus[v].is_some(),
                    None => false,
                },
                _ => false,
            };
            if resolved && !bodies.is_empty() {
                return bodies
                    .iter()
                    .all(|&j| self.p.body_ret.get(j).copied().flatten().is_some_and(|t| self.p.classify(t) == Class::Scalar));
            }
        }
        match self.p.target(callee, self.on_form(callee)) {
            Target::Fn(n) | Target::Method(n) => self.p.scalar_fns.contains(&n),
            _ => false,
        }
    }

    /// The element classes of a tuple-typed value: a field's declared
    /// type (its struct's when the receiver's struct is known, else every
    /// declaration of the name agreeing), a call's declared return.
    fn tuple_classes(&self, value: &Expr) -> Option<Vec<Class>> {
        match value {
            Expr::Field { receiver, name, .. } => {
                let st = match receiver.as_ref() {
                    Expr::Ident(id) => match self.lookup(&id.name) {
                        Some(Some(v)) => self.var_struct[v].clone(),
                        Some(None) => self.param_structs.get(&id.name).cloned(),
                        None => None,
                    },
                    _ => None,
                };
                let tys: Vec<&TypeExpr> = match st.as_ref().and_then(|st| self.p.struct_fields.get(st)) {
                    Some(fields) => fields.iter().filter(|(f, _)| *f == name.name).map(|(_, t)| *t).collect(),
                    None => self.p.field_types.iter().filter(|(f, _)| *f == name.name).filter_map(|(_, t)| *t).collect(),
                };
                let all: Vec<Vec<Class>> = tys.iter().filter_map(|t| self.p.tuple_elems(t)).collect();
                (all.len() == tys.len() && !all.is_empty() && all.iter().all(|c| *c == all[0])).then(|| all[0].clone())
            }
            _ => None,
        }
    }

    /// Is a block's value (its tail) surely a primitive?
    fn block_scalar(&self, b: &Block) -> bool {
        b.tail.as_ref().is_some_and(|t| self.known_scalar(None, t))
    }

    fn if_scalar(&self, i: &IfStmt) -> bool {
        self.block_scalar(&i.then_block)
            && match i.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => self.block_scalar(b),
                Some(ElseBranch::ElseIf(e)) => self.if_scalar(e),
                None => false,
            }
    }

    /// Is `value`, bound to a fresh local, known to be a struct (its
    /// declared type, a field every declaration of which holds one, a
    /// local or a parameter that is)?
    fn known_record(&self, ty: Option<&TypeExpr>, value: &Expr) -> bool {
        if let Some(t) = ty {
            return self.p.classify(t) == Class::Record;
        }
        match value {
            Expr::Field { name, .. } => self.p.record_fields.contains(&name.name),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.record_hint[v],
                Some(None) => self.record_params.contains(&id.name),
                None => false,
            },
            Expr::Struct { path, .. } => self.p.classify_path(path) == Class::Record,
            _ => false,
        }
    }

    /// Is the receiver of the method call `callee` known to be an
    /// `@form` locus: a form-typed field, a form parameter, a local bound
    /// from a form's literal or declared as one?
    fn on_form(&self, callee: &Expr) -> bool {
        let (Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. }) = callee else { return false };
        match receiver.as_ref() {
            Expr::Field { name, .. } => self.p.form_only_fields.contains(&name.name),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.forms[v],
                Some(None) => self.form_params.contains(&id.name),
                None => false,
            },
            _ => false,
        }
    }

    /// The place `e` names, spelled as a path: `self`, a local (by its
    /// variable, `@n`), a parameter (`$name`), a field of one. `None`:
    /// no place a path names (a call's result, a literal).
    fn place_of(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::KwSelf(_) => Some("self".to_string()),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => Some(format!("@{v}")),
                Some(None) => Some(format!("${}", id.name)),
                None => None,
            },
            Expr::Field { receiver, name, .. } => Some(format!("{}.{}", self.place_of(receiver)?, name.name)),
            _ => None,
        }
    }

    /// May `e` be a form whose reads leave the cell's nested blocks
    /// shared? A field some declaration of which holds one (or a type
    /// the pass cannot place), a local or a parameter that may, and any
    /// other value that may be a handle.
    fn may_be_view_form(&self, e: &Expr) -> bool {
        match e {
            Expr::Field { name, .. } => self.p.view_form_fields.contains(&name.name),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.view_forms[v],
                Some(None) => self.view_form_params.contains(&id.name),
                None => false,
            },
            Expr::KwSelf(_) => false,
            Expr::Struct { path, .. } => self.p.classify_path(path).may_be_view_form(),
            other => self.is_handle(other),
        }
    }

    /// May `e` be any `@form` locus?
    fn may_be_form(&self, e: &Expr) -> bool {
        match e {
            Expr::Field { name, .. } => self.p.form_fields.contains(&name.name),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.forms[v] || self.view_forms[v] || self.handles[v],
                Some(None) => self.handle_params.contains(&id.name),
                None => false,
            },
            Expr::KwSelf(_) => false,
            other => self.is_handle(other),
        }
    }

    /// The local an expression is read from (`v`, `v.f`, `v[i].g`) and
    /// how many segments below it.
    fn root_local(&self, e: &Expr) -> Option<(usize, usize)> {
        match e {
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => Some((v, 0)),
                _ => None,
            },
            Expr::Field { receiver, .. } | Expr::Index { receiver, .. } => {
                self.root_local(receiver).map(|(v, d)| (v, d + 1))
            }
            _ => None,
        }
    }

    /// The parameter `e` is, or is read from (`p`, `p.f`, `p[i]`), by
    /// position — not through a local, which the local-place rule copies
    /// at its first write.
    fn direct_param(&self, e: &Expr) -> Option<usize> {
        match e {
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(None) => self.param_index.get(&id.name).copied(),
                _ => None,
            },
            Expr::Field { receiver, .. } | Expr::Index { receiver, .. } => self.direct_param(receiver),
            _ => None,
        }
    }

    /// May `e` be (a part of) the caller's value a parameter refers to:
    /// a parameter, a read from one (`p.f`, `p[i]`), a local bound from
    /// one (`let a = p.big;`, a `for` element of `p.big`, a `match`
    /// payload of `p.e`), or what an `or`, `if`, `match` or block may
    /// carry out of one? A write through such a value, or handing it to
    /// a callee that writes through its parameter, writes through the
    /// parameter: an argument is passed by reference.
    fn rooted_at_param(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(None) => self.params.contains(&id.name),
                Some(Some(v)) => self.from_param[v],
                None => false,
            },
            Expr::Field { receiver, .. } | Expr::Index { receiver, .. } => self.rooted_at_param(receiver),
            Expr::Or { inner, disposition, .. } => {
                self.rooted_at_param(inner)
                    || matches!(disposition, OrDisposition::Substitute(x) if self.rooted_at_param(x))
            }
            Expr::Block(b) => self.block_rooted_at_param(b),
            Expr::If(i) => self.if_rooted_at_param(i),
            Expr::Match(m) => m.arms.iter().any(|a| match &a.body {
                MatchArmBody::Expr(x) => self.rooted_at_param(x),
                MatchArmBody::Block(b) => self.block_rooted_at_param(b),
            }),
            _ => false,
        }
    }

    fn block_rooted_at_param(&self, b: &Block) -> bool {
        b.tail.as_ref().is_some_and(|t| self.rooted_at_param(t))
    }

    fn if_rooted_at_param(&self, i: &IfStmt) -> bool {
        self.block_rooted_at_param(&i.then_block)
            || match i.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => self.block_rooted_at_param(b),
                Some(ElseBranch::ElseIf(e)) => self.if_rooted_at_param(e),
                None => false,
            }
    }

    /// May `e` be a handle? `self`, a handle parameter, a local that
    /// may hold one, a locus literal, a call of a fn or method declared
    /// to return one, and any field read (a field may hold a child).
    fn is_handle(&self, e: &Expr) -> bool {
        match e {
            Expr::KwSelf(_) => true,
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.handles[v],
                Some(None) => self.handle_params.contains(&id.name),
                None => false,
            },
            Expr::Struct { path, .. } => self.p.classify_path(path).may_be_handle(),
            Expr::Field { .. } | Expr::Index { .. } => true,
            Expr::Call { callee, .. } => match callee.as_ref() {
                // A fn value held in a local or a parameter.
                Expr::Ident(id) if self.lookup(&id.name).is_some() => true,
                c => self.p.returns_handle(&self.p.target(c, self.on_form(c))),
            },
            Expr::If(_) | Expr::Match(_) | Expr::Block(_) | Expr::Or { .. } => true,
            _ => false,
        }
    }

    /// For a tuple, array or record literal, the parts that view a
    /// place (nested literals walked through), by span: an owned copy
    /// of each where the literal is built leaves the literal owning
    /// what it holds, where a copy of the whole would also copy every
    /// scalar and fresh part. `None` for any other expression.
    fn literal_parts(&self, e: &Expr) -> Option<Vec<Site>> {
        self.literal_parts_by(e, false)
    }

    /// `literal_parts`; with `places`, a part counts that holds any
    /// place's value (the local-place rule's markers too), not only a
    /// place a write may replace.
    fn literal_parts_by(&self, e: &Expr, places: bool) -> Option<Vec<Site>> {
        fn walk(w: &Walk<'_, '_>, e: &Expr, out: &mut Vec<Site>, places: bool) {
            match e {
                Expr::Tuple(es, _) | Expr::Array(es, _) => es.iter().for_each(|x| walk(w, x, out, places)),
                Expr::Struct { inits, .. } => inits.iter().for_each(|i| walk(w, &i.value, out, places)),
                Expr::ArrayRepeat { val, .. } => walk(w, val, out, places),
                // What these carry out is known only inside them: the
                // part is copied whole.
                Expr::Match(_) | Expr::If(_) | Expr::Block(_) | Expr::Or { .. } => {
                    let span = e.span();
                    out.push(Site::Part(span.start.0, span.end.0));
                }
                _ => {
                    let keys = w.part_keys(e);
                    let counts = if places {
                        keys.as_ref().is_some_and(|k| !k.is_empty())
                    } else {
                        real(&keys).is_some()
                    };
                    if counts {
                        let span = e.span();
                        out.push(Site::Part(span.start.0, span.end.0));
                    }
                }
            }
        }
        if !matches!(e, Expr::Tuple(..) | Expr::Array(..) | Expr::Struct { .. } | Expr::ArrayRepeat { .. }) {
            return None;
        }
        let mut out = Vec::new();
        walk(self, e, &mut out, places);
        Some(out)
    }

    /// The keys of a field read, a local or an enum constructor, read
    /// without walking (`expr` has walked it already).
    fn part_keys(&self, e: &Expr) -> Keys {
        match e {
            Expr::Field { receiver, name, .. } => self.field_keys(receiver, self.part_keys(receiver), name),
            Expr::Index { receiver, .. } => in_cell(self.part_keys(receiver)),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => union(self.var_keys(v), self.ident_at(&id.name)),
                _ => self.ident_at(&id.name),
            },
            Expr::Call { callee, args, .. } => match callee.as_ref() {
                Expr::Path(q) if self.p.is_ctor(q) =>
                {
                    args.iter().fold(None, |acc, a| union(acc, self.part_keys(a)))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn write(&mut self, writes: Writes, start: Option<u32>) {
        let at = self.tick();
        self.own.add(&writes);
        if !writes.is_empty() {
            self.writes.push(WriteAt { writes, start: start.unwrap_or(at), at });
        }
    }

    /// Walk a block; the keys of its value (its tail's).
    fn block(&mut self, b: &Block) -> Keys {
        self.scopes.push(BTreeMap::new());
        self.block_stack.push(self.next_block);
        self.next_block += 1;
        for s in &b.stmts {
            self.stmt(s);
        }
        let keys = b.tail.as_ref().and_then(|t| self.expr(t));
        self.block_stack.pop();
        self.scopes.pop();
        keys
    }

    /// A call or a delivery that may read any of the program's fields.
    fn opaque(&mut self) {
        let at = self.tick();
        self.opaque_calls.push(at);
    }

    fn exit(&mut self, jump: bool) {
        let at = self.tick();
        self.exits.push((at, jump));
    }

    /// A read that may see a cell of the form at `place` (`None`: of any
    /// form).
    fn cell_read(&mut self, place: Option<String>) {
        self.reads = true;
        let at = self.tick();
        self.cell_reads.push((at, place));
    }

    fn in_loop(&mut self, f: impl FnOnce(&mut Self)) {
        let start = self.tick();
        f(self);
        let end = self.tick();
        self.loops.push((start, end));
    }

    /// Walk a statement; the keys of its value when it is an
    /// expression, `if`, `match` or block statement.
    fn stmt(&mut self, s: &Stmt) -> Keys {
        match s {
            Stmt::Let { name, ty, value, id, span, .. } => {
                let mut keys = self.expr(value);
                // A record bound from a local is a copy (GH #713): what
                // a read of a cell bound there, it is no more.
                if matches!(value, Expr::Ident(_)) {
                    keys = without_cells(keys);
                }
                keys = self.struct_literal_keys(value, keys);
                let parts = (self.literal_parts(value), self.literal_parts_by(value, true));
                // A binding declared as a handle is one, whatever the
                // value's own spelling says.
                let handle = self.is_handle(value) || ty.as_ref().is_some_and(|t| self.p.classify(t).may_be_handle());
                let (form, view_form) = match (ty, value) {
                    (Some(t), _) => {
                        let c = self.p.classify(t);
                        (c.is_form(), c.may_be_view_form())
                    }
                    (None, Expr::Struct { path, .. }) => {
                        let c = self.p.classify_path(path);
                        (c.is_form(), c.may_be_view_form())
                    }
                    (None, other) => (false, self.may_be_view_form(other)),
                };
                let surely_handle = self.surely_handle(ty.as_ref(), value);
                let record = self.known_record(ty.as_ref(), value);
                let scalar = self.known_scalar(ty.as_ref(), value);
                let place_rhs = matches!(value, Expr::Ident(_) | Expr::Field { .. } | Expr::Index { .. });
                let from_param = self.rooted_at_param(value);
                let locus_name = match (ty, value) {
                    (Some(t), _) => self.p.named_locus(t),
                    (None, Expr::Struct { path, .. }) => self.p.path_name(&path.segments).filter(|n| self.p.loci.contains(n)),
                    _ => None,
                };
                let struct_name = match (ty, value) {
                    (Some(t), _) => self.p.named_struct(t),
                    (None, Expr::Struct { path, .. }) => self.p.struct_of_path(path),
                    (None, Expr::Ident(id)) => match self.lookup(&id.name) {
                        Some(Some(u)) => self.var_struct[u].clone(),
                        Some(None) => self.param_structs.get(&id.name).cloned(),
                        None => None,
                    },
                    _ => None,
                };
                let compound = match value {
                    Expr::Field { name, .. } => self.p.compound_fields.contains(&name.name),
                    Expr::Ident(id) => matches!(self.lookup(&id.name), Some(Some(u)) if self.compound_hint[u]),
                    _ => false,
                };
                // A primitive (or a handle) holds no block a write could
                // reach: it carries no place marker (the local-place rule),
                // as lowering copies nothing for it.
                if scalar || surely_handle {
                    keys = real(&keys);
                }
                let v = self.declare(&name.name);
                self.from_param[v] = from_param;
                self.handles[v] = handle;
                self.forms[v] = form;
                self.view_forms[v] = view_form;
                self.handle_only[v] = surely_handle;
                self.record_hint[v] = record;
                self.scalar_hint[v] = scalar;
                self.compound_hint[v] = compound;
                self.var_struct[v] = struct_name;
                self.var_locus[v] = locus_name;
                self.bind_parts(v, Site::Node(id.0), &name.name, keys, parts, *span, true, place_rhs);
            }
            Stmt::LetTuple { names, value, id, span, .. } => {
                let keys = in_cell(self.expr(value));
                let from_param = self.rooted_at_param(value);
                // Each element known to be a primitive carries no place
                // marker: a literal's element by its own value, a field's
                // by the tuple type it is declared as.
                let scalars: Vec<bool> = match value {
                    Expr::Tuple(es, _) => es.iter().map(|e| self.known_scalar(None, e)).collect(),
                    _ => {
                        let classes = self.tuple_classes(value);
                        (0..names.len()).map(|i| classes.as_ref().is_some_and(|c| c.get(i) == Some(&Class::Scalar))).collect()
                    }
                };
                for (i, n) in names.iter().enumerate() {
                    let v = self.declare(&n.name);
                    self.from_param[v] = from_param;
                    let scalar = scalars.get(i).copied().unwrap_or(false);
                    self.scalar_hint[v] = scalar;
                    let k = if scalar { real(&keys) } else { keys.clone() };
                    self.bind(v, Site::Node(id.0), &n.name, k, *span);
                }
            }
            Stmt::Assign { target, value, id, span, .. } => {
                for seg in &target.tail {
                    if let LValueSeg::Index(e) = seg {
                        self.expr(e);
                    }
                }
                let mut keys = self.expr(value);
                let first = target.tail.iter().find_map(|seg| match seg {
                    LValueSeg::Field(f) => Some(f.name.as_str()),
                    LValueSeg::Index(_) => None,
                });
                let head = self.lookup(&target.head.name);
                if target.tail.is_empty() {
                    // A bare local rebound: a new instance of it.
                    if let Some(Some(v)) = head {
                        if self.is_handle(value) {
                            self.handles[v] = true;
                        }
                        if self.may_be_view_form(value) {
                            self.view_forms[v] = true;
                        }
                        if self.rooted_at_param(value) {
                            self.from_param[v] = true;
                        }
                        if matches!(value, Expr::Ident(_)) {
                            keys = without_cells(keys);
                        }
                        keys = self.struct_literal_keys(value, keys);
                        let parts = (self.literal_parts(value), self.literal_parts_by(value, true));
                        let place_rhs = matches!(value, Expr::Ident(_) | Expr::Field { .. } | Expr::Index { .. });
                        self.bind_parts(v, Site::Node(id.0), &target.head.name, keys, parts, *span, true, place_rhs);
                    } else if matches!(head, Some(None)) {
                        let at = self.tick();
                        self.param_rebinds.push((target.head.name.clone(), at));
                    }
                } else {
                    if let Some(f) = first {
                        let id = self.p.field_id(f);
                        self.write(Writes::field(id), None);
                    } else {
                        // `x[i] = v` on a local or a parameter: the element
                        // a binding of `x[j]` views.
                        self.use_name(&target.head.name);
                        let id = self.p.field_id(&target.head.name);
                        self.write(Writes::field(id), None);
                    }
                    // The place written in place, for the local-place rule:
                    // a binding of it (or of a part of it) made before
                    // would see the write.
                    // The block an assignment writes a slot of is what the
                    // path names short of its last segment; a marker
                    // stops at an index, and so does this.
                    if let Some(root) = self.place_of(&Expr::Ident(target.head.clone())).filter(|r| r != "self") {
                        let mut place = root;
                        for seg in &target.tail[..target.tail.len() - 1] {
                            match seg {
                                LValueSeg::Field(f) => place = format!("{place}.{}", f.name),
                                LValueSeg::Index(_) => break,
                            }
                        }
                        let at = self.tick();
                        self.root_writes.push((place, at, false));
                    }
                    match head {
                        // A write under a local: what the cell-read rule
                        // decides about, once the body is walked.
                        Some(Some(v)) => {
                            // A local that may be (a part of) the caller's
                            // value: the write reaches the caller.
                            if self.from_param[v] {
                                self.writes_params = true;
                            }
                            let first = match target.tail.first() {
                                Some(LValueSeg::Field(f)) => Some(f.name.clone()),
                                _ => None,
                            };
                            let at = self.tick();
                            self.assigns.push(AssignWrite {
                                var: v,
                                first,
                                depth: target.tail.len(),
                                id: id.0,
                                span: *span,
                                at,
                                blocks: self.block_stack.clone(),
                            });
                        }
                        // A write under a parameter reaches the caller's
                        // value (an argument is passed by reference).
                        Some(None) if self.params.contains(&target.head.name) => {
                            self.writes_params = true;
                            if let Some(&i) = self.param_index.get(&target.head.name) {
                                self.param_mask |= param_bit(i);
                            }
                        }
                        _ => {}
                    }
                    // A place's (or a cell's) value assigned into a part of
                    // a local: the local holds that block now, as a literal
                    // field init would. The assignment binds the part anew
                    // (its flag cleared after the store): a field path marks
                    // the field (`h.f = r.arr` → a part marker), an element
                    // the whole local (`xs[0] = <cell read>`).
                    if let Some(Some(v)) = head {
                        self.bind_assigned_part(v, target, value, &keys, id.0, *span);
                    }
                    // A place's value assigned into a part of a parameter: the
                    // caller's value would hold that block, and a later write
                    // through either reaches the other (`h.arr = r.arr;
                    // h.arr[0].k = 7` with both parameters). The value is
                    // stored as a copy, as a struct field's is (GH #992).
                    // A handle parameter's field is that locus's storage, and
                    // a store into it from outside the locus's methods is not
                    // yet owned by the locus (the value stays where the caller's
                    // arena holds it): a value spelled as a place (a local, a
                    // field, an element), a String included, is a copy, so the
                    // locus and the caller never share it (a later same-length
                    // overwrite by the locus would otherwise rewrite the
                    // caller's String). Nothing else is (a String carried by an
                    // `if` or a call, a store through a handle local): GH #1561.
                    // The clone costs: a free fn storing a String parameter
                    // into a handle's field 4M times ran 7 s and 62 MB longer.
                    let param_head = matches!(head, Some(None)) && self.params.contains(&target.head.name);
                    let handle_head = self.handle_only_params.contains(&target.head.name);
                    let marked = keys.as_ref().is_some_and(|k| k.iter().any(|k| k.starts_with(AT) || k.starts_with(PART)));
                    let place_value = matches!(value, Expr::Ident(_) | Expr::Field { .. } | Expr::Index { .. });
                    if param_head
                        && self.summary.is_some()
                        && !self.surely_handle(None, value)
                        && ((marked && !self.known_scalar(None, value)) || (handle_head && place_value))
                    {
                        let vspan = value.span();
                        self.args.push((Site::Part(vspan.start.0, vspan.end.0), target.head.name.clone()));
                    }
                    // `self.f.. = v`, a local stored whole into a self
                    // field: what a get-modify-store sets back.
                    let fields: Option<Vec<&str>> = target
                        .tail
                        .iter()
                        .map(|seg| match seg {
                            LValueSeg::Field(f) => Some(f.name.as_str()),
                            LValueSeg::Index(_) => None,
                        })
                        .collect();
                    if let (true, Some(fields), Expr::Ident(vid)) = (target.head.name == "self", fields, value) {
                        if let Some(Some(var)) = self.lookup(&vid.name) {
                            let place = format!("self.{}", fields.join("."));
                            let block = self.block_stack.last().copied();
                            self.self_stores.push(SelfStore { at: self.pos, place, var, block });
                        }
                    }
                }
            }
            Stmt::If(i) => return self.if_stmt(i),
            Stmt::Match(m) => return self.match_stmt(m),
            Stmt::For { name, iter, body, id, span, .. } => {
                // The loop holds the array it walks from before its
                // first iteration to its last, reading the next element
                // at the top of each: the binding is the array's, made
                // before the loop, and the element is that binding.
                let mut keys = in_cell(self.expr(iter));
                // A form's walk (`m.entries`, `v.items`) reads its cells;
                // a map that reads in place hands each entry as the cell
                // itself. A `@form(vec)` walk visits its cells in place
                // (spec/forms.md), and a walk of `self.children` hands
                // out handles: neither element is a local-place value.
                if let Expr::Field { receiver, name: pseudo, .. } = iter {
                    if pseudo.name == "children" && matches!(receiver.as_ref(), Expr::KwSelf(_)) {
                        keys = real(&keys);
                    }
                    if matches!(pseudo.name.as_str(), "entries" | "items") && self.may_be_form(receiver) {
                        keys = real(&keys);
                        let place = self.place_of(receiver);
                        if pseudo.name == "entries" && self.may_be_view_form(receiver) {
                            let marker = format!("{IN_CELL}{}", place.clone().unwrap_or_else(|| "?".to_string()));
                            keys = union(keys, Some(BTreeSet::from([marker])));
                        }
                        self.cell_read(place);
                    }
                }
                let from_param = self.rooted_at_param(iter);
                self.scopes.push(BTreeMap::new());
                let v = self.declare(&name.name);
                self.from_param[v] = from_param;
                self.bind(v, Site::Node(id.0), &name.name, keys, *span);
                self.in_loop(|w| {
                    let at = w.tick();
                    w.uses[v].push(at);
                    w.block(body);
                });
                self.scopes.pop();
            }
            Stmt::While { cond, body, .. } => self.in_loop(|w| {
                w.expr(cond);
                w.block(body);
            }),
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    let keys = self.expr(e);
                    self.note_return(e, keys);
                }
                self.exit(false);
            }
            Stmt::Fail { value, .. } => {
                self.expr(value);
                self.exit(false);
            }
            Stmt::Violate { payload, .. } => {
                if let Some(e) = payload {
                    self.expr(e);
                }
                self.exit(false);
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
                // A direct-dispatch subscriber runs before the publish
                // returns.
                self.opaque();
                let w = self.deliveries();
                self.write(w, None);
                self.delivery_reads();
            }
            Stmt::Yield(_) => {
                self.opaque();
                let w = self.deliveries();
                self.write(w, None);
                self.delivery_reads();
            }
            Stmt::Reperspective { .. } => {
                self.opaque();
                self.write(Writes::top(), None);
                self.cell_read(None);
            }
            Stmt::Recovery { args, .. } => {
                args.iter().for_each(|a| {
                    self.expr(a);
                });
                self.opaque();
                self.write(Writes::top(), None);
                self.cell_read(None);
            }
            Stmt::ShmWrite { max, binding, body, .. } => {
                self.expr(max);
                self.scopes.push(BTreeMap::new());
                self.scopes.last_mut().expect("a scope").insert(binding.name.clone(), None);
                self.block(body);
                self.scopes.pop();
            }
            Stmt::Block(b) => return self.block(b),
            Stmt::Expr(e) => {
                // A call written as a statement: a store there runs
                // wherever the statement does.
                let saved = self.stmt_call.take();
                if let Expr::Call { span, .. } = e {
                    self.stmt_call = Some((span.start.0, span.end.0));
                }
                let keys = self.expr(e);
                self.stmt_call = saved;
                return keys;
            }
            Stmt::Break(_) | Stmt::Continue(_) => self.exit(true),
            Stmt::Terminate(_) => self.exit(false),
        }
        None
    }

    /// Walk an `if`; the keys of its value, either branch's.
    fn if_stmt(&mut self, i: &IfStmt) -> Keys {
        self.expr(&i.cond);
        let then = self.block(&i.then_block);
        let other = match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(e)) => self.if_stmt(e),
            None => None,
        };
        union(then, other)
    }

    /// Walk a `match`; the keys of its value, any arm's.
    fn match_stmt(&mut self, m: &MatchStmt) -> Keys {
        let keys = in_cell(self.expr(&m.scrutinee));
        let from_param = self.rooted_at_param(&m.scrutinee);
        let mut out = None;
        let site = Site::Match(m.span.start.0, m.span.end.0);
        for arm in &m.arms {
            self.scopes.push(BTreeMap::new());
            let mut names = Vec::new();
            pattern_names(&arm.pattern, &mut names);
            for n in names {
                let v = self.declare(&n.name);
                self.from_param[v] = from_param;
                self.bind(v, site, &n.name, keys.clone(), m.span);
            }
            if let Some(g) = &arm.guard {
                self.expr(g);
            }
            let arm_keys = match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b),
            };
            out = union(out, arm_keys);
            self.scopes.pop();
        }
        out
    }

    /// Walk an `or`'s disposition; the keys of a substitute value.
    fn disposition(&mut self, d: &OrDisposition) -> Keys {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                self.scopes.push(BTreeMap::new());
                self.scopes.last_mut().expect("a scope").insert("err".into(), None);
                let keys = self.expr(e);
                self.scopes.pop();
                if matches!(d, OrDisposition::Substitute(_)) {
                    keys
                } else {
                    self.exit(false);
                    None
                }
            }
            OrDisposition::Raise(_) => {
                self.exit(false);
                None
            }
            OrDisposition::Discard(_) | OrDisposition::Wait(_) => None,
        }
    }

    fn use_name(&mut self, name: &str) {
        if let Some(Some(v)) = self.lookup(name) {
            let at = self.tick();
            self.uses[v].push(at);
        }
    }

    /// Walk an expression; the keys of the places its value may view
    /// (`None`: it views none). A field read views its field, and
    /// whatever the value it is read from views; a local, what its
    /// instances view; a tuple, array or record literal, what its
    /// parts view; a block, `if`, `match` or `or`, what any value it
    /// may produce views. A call's result may hand back what its
    /// arguments and its receiver view (a stdlib leaf may return its
    /// argument, an enum constructor holds it), so it views their
    /// keys; arithmetic and concatenation produce fresh values.
    fn expr(&mut self, e: &Expr) -> Keys {
        match e {
            Expr::Ident(id) => {
                self.use_name(&id.name);
                match self.lookup(&id.name) {
                    Some(Some(v)) => union(self.var_keys(v), self.ident_at(&id.name)),
                    _ => self.ident_at(&id.name),
                }
            }
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => None,
            Expr::Binary { left, right, .. } => {
                self.operands(&[left, right]);
                None
            }
            Expr::Unary { operand, .. } => {
                self.expr(operand);
                None
            }
            Expr::Call { callee, args, span, .. } => self.call(callee, args, *span),
            Expr::Field { receiver, name, .. } => {
                let keys = self.expr(receiver);
                let at = self.tick();
                self.field_reads.push((at, name.name.clone()));
                self.field_keys(receiver, keys, name)
            }
            Expr::Path2 { receiver, .. } => {
                self.expr(receiver);
                None
            }
            Expr::Index { receiver, index, .. } => {
                let mut keys = self.operands(&[receiver, index]);
                in_cell(keys.swap_remove(0))
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                let ops: Vec<&Expr> = es.iter().collect();
                self.operands(&ops).into_iter().fold(None, union)
            }
            Expr::Struct { inits, .. } => {
                let ops: Vec<&Expr> = inits.iter().map(|i| &i.value).collect();
                self.operands(&ops).into_iter().fold(None, union)
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Sum(x, _) | Expr::Prod(x, _) => {
                self.expr(x);
                None
            }
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
                None
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
                None
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => {
                let keys = self.expr(inner);
                let other = self.disposition(disposition);
                union(keys, other)
            }
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Keys {
        let start = self.tick();
        let mut writes = Writes::default();
        let mut ctor = false;
        let mut form_op = false;
        let mut receiver: Option<&Expr> = None;
        // What the callee may do through its parameters and to a form's
        // cells: `Some` a callee with a summary, by name.
        let mut summarized: Option<Callee> = None;
        // A callee the pass cannot identify: it may do anything.
        let mut unknown = false;
        let mut bounded = false;
        // A target the pass cannot positively identify may write
        // anything: missed precision costs a copy, never a use after a
        // retire.
        match callee {
            Expr::Ident(id) if self.lookup(&id.name).is_some() => {
                // A fn value held in a local or a parameter.
                self.use_name(&id.name);
                writes = Writes::top();
                unknown = true;
            }
            Expr::Ident(_) | Expr::Path(_) | Expr::Field { .. } | Expr::Path2 { .. } => {
                if let Expr::Field { receiver: r, .. } | Expr::Path2 { receiver: r, .. } = callee {
                    receiver = Some(r);
                    // A method of a field's value (a form's `set`, a
                    // child's) may change what the field holds.
                    if let Expr::Field { name: f, .. } = r.as_ref() {
                        writes.add(&Writes::field(self.p.field_id(&f.name)));
                    }
                }
                match self.p.target(callee, self.on_form(callee)) {
                    Target::Fn(n) => {
                        writes.add(&self.callee(Callee::Fn(n.clone())));
                        summarized = Some(Callee::Fn(n));
                    }
                    Target::Method(n) => {
                        writes.add(&self.callee(Callee::Method(n.clone())));
                        summarized = Some(Callee::Method(n));
                    }
                    Target::Std => {
                        let segs: Vec<&str> = match callee {
                            Expr::Ident(id) => vec![id.name.as_str()],
                            Expr::Path(q) => q.segments.iter().map(|s| s.name.as_str()).collect(),
                            _ => Vec::new(),
                        };
                        writes.add(&self.std_leaf(&segs));
                    }
                    // An enum constructor holds its arguments; a fn's or a
                    // stdlib leaf's result is copied out of its frame.
                    Target::Ctor => ctor = true,
                    Target::FormOp => form_op = true,
                    Target::Bounded => {
                        // `count` and `at` read the list; the rest write it.
                        bounded = !matches!(callee, Expr::Ident(id) if matches!(id.name.as_str(), "count" | "at"));
                        // The bounded field it is handed.
                        if let Some(Expr::Field { name: f, .. }) = args.first() {
                            writes.add(&Writes::field(self.p.field_id(&f.name)));
                        }
                    }
                    Target::Unknown => {
                        writes = Writes::top();
                        unknown = true;
                    }
                }
            }
            other => {
                self.expr(other);
                writes = Writes::top();
                unknown = true;
            }
        }
        // The receiver and the arguments, in the order they are
        // evaluated: each is held while the later ones run.
        let mut ops: Vec<&Expr> = Vec::with_capacity(args.len() + 1);
        ops.extend(receiver);
        ops.extend(args.iter());
        let mut keys = self.operands(&ops);
        // The callee runs once its operands are in: it may read any of
        // the program's fields.
        if unknown || summarized.is_some() {
            self.opaque();
        }
        // What a form's method hands back is a cell's read or a fresh
        // value, never the receiver's own block: no place marker.
        let operand_keys: Keys = keys.iter().cloned().fold(None, union);
        let recv_keys = if receiver.is_some() { real(&keys.remove(0)) } else { None };
        let mut result: Keys = None;
        // A form's read may hand back its cell; any other method's
        // result is copied into the caller's arena.
        if let Some(Expr::Field { name: f, .. }) = receiver {
            if self.p.form_fields.contains(&f.name) {
                result = recv_keys.clone();
            }
        }
        if form_op {
            // A form held in a local or a parameter: its cells are a place
            // of their own, named after it, which its writes (`set`,
            // `remove`, `pop`) retire and its reads may view.
            if let Some(Expr::Ident(id)) = receiver {
                let key = format!("@{}", id.name);
                writes.add(&Writes::field(self.p.field_id(&key)));
                let mut k = BTreeSet::new();
                k.insert(key);
                result = union(result, Some(k));
            }
            result = union(result, recv_keys.clone());
        }
        // A call whose result may be what a caller's name reaches (it
        // returns a parameter's part, a self field's, a cell's, or what a
        // callee that does returns — `Summaries::ret_views`), or a callee
        // the pass cannot identify: the result is a place's value, which
        // may be what the call was handed. On the summary pass the result
        // only names its callee.
        if !form_op && !ctor {
            let plain = matches!(&summarized, Some(Callee::Fn(n) | Callee::Method(n)) if self.p.plain_fns.contains(n));
            if let (None, Some(c), false) = (self.summary, &summarized, plain) {
                let bodies = self.callee_bodies(c, receiver);
                let srcs: Vec<Src> = keys
                    .iter()
                    .map(|k| {
                        let mut s = Src::default();
                        if let Some(ks) = k {
                            self.src_of(ks, false, &mut s);
                        }
                        s
                    })
                    .collect();
                let mark = format!("{RET}{}", self.ret_marks.len());
                self.ret_marks.insert(mark.clone(), (bodies, srcs));
                result = union(result, Some(BTreeSet::from([mark])));
            }
            // What the callees return: something else of the caller's (or
            // an unidentified callee's anything), or the arguments handed
            // for the parameters they return a part of.
            let markers = |ks: &Keys| -> Keys {
                ks.as_ref().map(|ks| {
                    ks.iter()
                        .filter_map(|k| {
                            if k.starts_with(AT) || k.starts_with(PART) || k.starts_with(IN_CELL) {
                                Some(k.clone())
                            } else if let Some(rest) = k.strip_prefix(CELL) {
                                // A cell's read handed back: the cell's block
                                // itself now, its kind kept, so the cell-read
                                // rule copies it where it is written or
                                // handed to a writer.
                                Some(format!("{IN_CELL}{}", rest.split(KEY_SEP).next().unwrap_or("")))
                            } else {
                                None
                            }
                        })
                        .collect()
                })
            };
            if let (Some(s), false) = (self.summary, plain) {
                let rv = if unknown {
                    RetView { other: true, ..RetView::default() }
                } else if let Some(c) = &summarized {
                    self.callee_bodies(c, receiver).into_iter().fold(RetView::default(), |a, j| {
                        let v = s.ret_views[j];
                        RetView {
                            params: a.params | v.params,
                            other: a.other || v.other,
                            pparams: a.pparams | v.pparams,
                            pother: a.pother || v.pother,
                        }
                    })
                } else {
                    RetView::default()
                };
                // A part's markers: every place a marker names, as a part
                // marker any field matches (`*`).
                let as_parts = |ks: Keys| -> Keys {
                    ks.map(|ks| {
                        ks.iter()
                            .map(|k| {
                                let place = match k.strip_prefix(AT) {
                                    Some(p) => p.to_string(),
                                    None => k.strip_prefix(PART).and_then(|r| r.split(KEY_SEP).nth(1)).unwrap_or("?").to_string(),
                                };
                                format!("{PART}*{KEY_SEP}{place}")
                            })
                            .collect()
                    })
                };
                if rv.other {
                    result = union(result, union(markers(&operand_keys), at(Some("?ret".to_string()))));
                } else {
                    for (k, ak) in keys.iter().enumerate() {
                        if rv.params & param_bit(k) != 0 {
                            result = union(result, markers(ak));
                        }
                    }
                    if rv.pother {
                        result = union(result, as_parts(union(markers(&operand_keys), at(Some("?ret".to_string())))));
                    }
                    for (k, ak) in keys.iter().enumerate() {
                        if rv.pparams & param_bit(k) != 0 {
                            result = union(result, as_parts(markers(ak)));
                        }
                    }
                }
            }
        }
        // The cell-read rule: a read of a form's cell, a store into one.
        let method = match callee {
            Expr::Field { name, .. } | Expr::Path2 { name, .. } => Some(name.name.as_str()),
            _ => None,
        };
        if let (Some(r), Some(m)) = (receiver, method) {
            let on_any_form = form_op || self.may_be_form(r);
            if on_any_form && VALUE_READS.contains(&m) {
                self.cell_read(self.place_of(r));
            }
            if CELL_READS.contains(&m) && self.may_be_view_form(r) {
                // The value read is the cell's bytes: its nested blocks
                // the cell's. The key, when it is a local, is what a
                // literal set back keys the same cell by.
                let place = self.place_of(r).unwrap_or_else(|| format!("?{start}"));
                let key = match (m, args.first()) {
                    ("get", Some(a @ Expr::Ident(_))) => self.place_of(a).unwrap_or_else(|| "-".to_string()),
                    _ => "-".to_string(),
                };
                let marker = format!("{CELL}{place}{KEY_SEP}{key}");
                result = union(result, Some(BTreeSet::from([marker])));
            }
            if CELL_STORES.contains(&m) && self.may_be_view_form(r) && args.len() == 1 {
                let place = self.place_of(r).unwrap_or_else(|| format!("?{start}"));
                let arg = self.store_arg(&args[0]);
                let block = (self.stmt_call == Some((span.start.0, span.end.0)))
                    .then(|| self.block_stack.last().copied())
                    .flatten();
                // Placed at the store's write, below.
                self.stores.push(StoreAt { at: 0, place, arg, block });
            }
        }
        // What the callee may do with the arguments it is handed: write
        // through them (a parameter is a reference to the caller's value),
        // and read a form's cell.
        let callee_reads = unknown || summarized.as_ref().is_some_and(|c| self.summarized(c, |s, j| s.reads[j]));
        if callee_reads {
            self.cell_read(None);
        }
        for (i, a) in args.iter().enumerate() {
            if form_op || ctor {
                continue;
            }
            let may_write = unknown
                || (bounded && i == 0)
                || summarized.as_ref().is_some_and(|c| self.summarized(c, |s, j| s.writes_params[j]));
            if self.summary.is_none() {
                // The summary pass: a parameter handed on, by position
                // (a bounded intrinsic or a callee the pass cannot
                // identify writes it).
                if let Some(m) = self.direct_param(a) {
                    if unknown || (bounded && i == 0) {
                        self.param_mask |= param_bit(m);
                    } else if let Some(c) = &summarized {
                        self.param_passes.push((c.clone(), i, m));
                    }
                }
                // A bounded intrinsic on a local bound from a parameter
                // writes the caller's list: no first-class value, it is
                // never copied.
                if bounded && i == 0 && self.direct_param(a).is_none() && self.rooted_at_param(a) {
                    self.param_mask = u64::MAX;
                }
                if self.rooted_at_param(a) {
                    if unknown || (bounded && i == 0) {
                        self.writes_params = true;
                    } else if let Some(c) = &summarized {
                        self.param_callees.insert(c.clone());
                    }
                }
                continue;
            }
            if !may_write {
                continue;
            }
            // The local-place rule: a local handed to a call that may
            // write through it (a bounded intrinsic's list is no
            // first-class value, and is not copied).
            // The parameter's declared class in every body the call may
            // reach that writes through a parameter (a method name names
            // every locus's method of it): a primitive one (a String, a
            // number) is never written into, the callee can only rebind
            // it, and a handle is a reference by design; a struct one
            // says the local passed whole is a struct.
            // Only the bodies that write through this very parameter count
            // (`Summaries::param_masks`); a callee the pass cannot identify
            // may write through any.
            let classes: Vec<Class> = summarized.as_ref().map_or(Vec::new(), |c| {
                let s = self.summary.expect("the second pass");
                self.p
                    .resolve(c)
                    .into_iter()
                    .filter(|&j| s.param_masks[j] & param_bit(i) != 0)
                    .map(|j| self.p.bodies[j].0.get(i).map_or(Class::Unknown, |prm| self.p.classify(&prm.ty)))
                    .collect()
            });
            let only = |ok: &dyn Fn(Class) -> bool| !classes.is_empty() && classes.iter().all(|&c| ok(c));
            let into = !(bounded && i == 0)
                && (unknown || !classes.is_empty())
                && !only(&|c| matches!(c, Class::Scalar | Class::Handle | Class::Form(_)));
            if let (Some((v, _)), true) = (self.root_local(a), into) {
                let record_param = only(&|c| matches!(c, Class::Scalar | Class::Record));
                self.place_passed.push((v, record_param && matches!(a, Expr::Ident(_)), self.first_field(a)));
            }
            if let Some(place) = self.arg_place(a).filter(|_| into) {
                let at = self.tick();
                self.root_writes.push((place, at, true));
            }
            match self.root_local(a) {
                // A bounded intrinsic writes the list inline in its
                // record: only one below the local's top level is in a
                // nested block.
                Some((v, depth)) if !(bounded && depth < 2) => self.passed.push((v, depth)),
                Some(_) => {}
                None => {
                    // A read of a cell handed straight to the call: copied
                    // where it is evaluated.
                    let k = keys.get(i).cloned().flatten();
                    if let Some(k) = k.filter(|ks| ks.iter().any(|k| k.starts_with(CELL) || k.starts_with(IN_CELL))) {
                        self.flag_operand(a, &k, false);
                        self.passed_copies.push(a.span());
                    }
                }
            }
        }
        for (a, keys) in args.iter().zip(keys) {
            // An argument the callee may replace before it reads its
            // parameter. A local passed on is its own binding's use,
            // inside the call: that binding answers for it. A form's
            // synthesized method copies its arguments in before it
            // replaces a cell.
            if form_op {
                continue;
            }
            if let (Some(k), true, false) = (&keys, self.summary.is_some(), matches!(a, Expr::Ident(_))) {
                if writes.touches(k, self.p) {
                    self.flag_operand(a, k, true);
                }
            }
            if ctor {
                result = union(result, keys);
            }
        }
        // A form's synthesized method has consumed its arguments by the
        // time it writes: the write spans no argument.
        self.write(writes, if form_op { None } else { Some(start) });
        if let Some(store) = self.stores.last_mut().filter(|s| s.at == 0) {
            store.at = self.pos;
        }
        result
    }

    /// What a store is handed: a local, a literal (its key fields'
    /// inits), or something else.
    fn store_arg(&self, a: &Expr) -> StoreArg {
        match a {
            Expr::Ident(_) => match self.root_local(a) {
                Some((v, 0)) => StoreArg::Var(v),
                _ => StoreArg::Other,
            },
            Expr::Struct { inits, .. } => StoreArg::Literal(
                inits
                    .iter()
                    .filter(|i| self.p.key_fields.contains(&i.name.name))
                    .map(|i| match &i.value {
                        Expr::Field { receiver, name, .. } if name.name == i.name.name => match self.root_local(receiver) {
                            Some((v, 0)) => KeyInit::FieldOf(v),
                            _ => KeyInit::Other,
                        },
                        Expr::Ident(_) => match self.place_of(&i.value) {
                            Some(u) => KeyInit::Var(u),
                            None => KeyInit::Other,
                        },
                        _ => KeyInit::Other,
                    })
                    .collect(),
            ),
            _ => StoreArg::Other,
        }
    }

    /// Does some body `c` names answer `f` (over the summaries)? On the
    /// summary pass, nothing is known yet: no.
    fn summarized(&self, c: &Callee, f: impl Fn(&Summaries, usize) -> bool) -> bool {
        let Some(s) = self.summary else { return false };
        self.p.resolve(c).into_iter().any(|j| f(s, j))
    }

    /// A publish or a `yield`: the handlers it may run may read a cell.
    fn delivery_reads(&mut self) {
        let reads = self.summarized(&Callee::Deliveries, |s, j| s.reads[j]);
        self.callees.insert(Callee::Deliveries);
        if reads {
            self.cell_read(None);
        }
    }

    /// Walk operands evaluated in order and held until the last is: an
    /// operand that views a place is copied where it is evaluated when a
    /// later operand's evaluation may replace that place (`take(match
    /// self.e { .. }, self.reset())`, `self.f.s + self.renamed()`, a
    /// record literal's fields). The keys of each.
    /// A call's receiver and arguments are operands too: one a later
    /// sibling's evaluation replaces is copied whole, a compound
    /// included — the place it referenced is gone before the callee
    /// starts, so there is nothing for the callee to write through.
    fn operands(&mut self, ops: &[&Expr]) -> Vec<Keys> {
        let first_write = self.writes.len();
        let mut keys = Vec::with_capacity(ops.len());
        let mut ends = Vec::with_capacity(ops.len());
        for op in ops {
            keys.push(self.expr(op));
            ends.push(self.pos);
        }
        if self.summary.is_some() && ops.len() > 1 {
            for i in 0..ops.len() - 1 {
                let Some(k) = &keys[i] else { continue };
                let hit = self.writes[first_write..]
                    .iter()
                    .any(|w| w.at > ends[i] && w.writes.touches(k, self.p));
                if hit {
                    let k = k.clone();
                    // A later sibling replaced the place before the call
                    // begins: there is no live place for a callee to write
                    // through, so even a compound argument is copied whole.
                    self.flag_operand(ops[i], &k, false);
                }
            }
        }
        keys
    }

    /// Copy `a`, an operand that views `keys`, where it is evaluated: a
    /// literal by its parts that view a place, anything else whole.
    fn flag_operand(&mut self, a: &Expr, keys: &BTreeSet<String>, by_ref: bool) {
        let name = keys.iter().find(|k| !is_place_marker(k)).cloned().unwrap_or_default();
        match self.literal_parts(a) {
            Some(parts) => self.args.extend(parts.into_iter().map(|s| (s, name.clone()))),
            None => {
                let span = a.span();
                let site = if by_ref { Site::Arg(span.start.0, span.end.0) } else { Site::Part(span.start.0, span.end.0) };
                self.args.push((site, name));
            }
        }
    }

    /// What a delivery the bus runs before returning may write: every
    /// handler's writes.
    fn deliveries(&mut self) -> Writes {
        if let Some(w) = &self.deliveries {
            return w.clone();
        }
        let w = self.callee(Callee::Deliveries);
        if self.summary.is_some() {
            self.deliveries = Some(w.clone());
        }
        w
    }

    /// What a builtin or stdlib leaf writes: what the bus handlers
    /// write when it publishes or parks on an `async_io` pool, else
    /// nothing of the program's.
    fn std_leaf(&mut self, segs: &[&str]) -> Writes {
        use crate::stdlib_surface::{effects_for, parks_on_async_io, EffectSet};
        if parks_on_async_io(segs)
            || effects_for(segs).is_some_and(|e| !e.is_unclassified() && e.contains(EffectSet::PUBLISH))
        {
            self.opaque();
            let w = self.deliveries();
            self.delivery_reads();
            return w;
        }
        Writes::default()
    }

    fn callee(&mut self, c: Callee) -> Writes {
        let mut w = Writes::default();
        if let Some(summary) = self.summary {
            for j in self.p.resolve(&c) {
                w.add(&summary.writes[j]);
            }
        }
        self.callees.insert(c);
        w
    }

    /// The instances whose value some use reads after a write that may
    /// change it: the sites to bind owned, and which instances they are.
    fn flagged(&self) -> (Vec<(Site, String)>, Vec<bool>) {
        let mut out = self.args.clone();
        let mut owned = vec![false; self.insts.len()];
        for (i, inst) in self.insts.iter().enumerate() {
            // A place marker is no key a write replaces.
            if inst.keys.iter().all(|k| is_place_marker(k)) {
                continue;
            }
            let hit = self.writes.iter().any(|w| {
                w.at > inst.at
                    && w.writes.touches(&inst.keys, self.p)
                    && self.uses[inst.var].iter().any(|&u| {
                        u >= w.start
                            || self.loops.iter().any(|&(ls, le)| {
                                ls > self.declared[inst.var] && ls <= u.min(w.start) && u.max(w.at) <= le
                            })
                    })
            });
            if hit {
                owned[i] = true;
                self.own_inst(inst, &mut out);
            }
        }
        (out, owned)
    }

    fn own_inst(&self, inst: &Inst, out: &mut Vec<(Site, String)>) {
        match &inst.parts {
            Some(parts) => out.extend(parts.iter().map(|s| (*s, inst.name.clone()))),
            None => out.push((inst.site, inst.name.clone())),
        }
    }

    /// The cell markers an instance carries: each read's form place and
    /// key; and whether it carries an in-cell one.
    fn markers(&self, i: usize) -> (Vec<(String, String)>, bool) {
        let mut cells = Vec::new();
        let mut inside = false;
        for k in &self.insts[i].keys {
            if let Some(rest) = k.strip_prefix(CELL) {
                let mut it = rest.splitn(2, KEY_SEP);
                let place = it.next().unwrap_or("").to_string();
                let key = it.next().unwrap_or("-").to_string();
                cells.push((place, key));
            } else if k.starts_with(IN_CELL) {
                inside = true;
            }
        }
        (cells, inside)
    }

    /// Is `at` inside a loop that `outer` is not (a loop containing `at`
    /// whose body does not hold `outer`)?
    fn loops_holding_not(&self, at: u32, outer: u32) -> Vec<(u32, u32)> {
        self.loops.iter().copied().filter(|&(ls, le)| ls < at && at <= le && !(ls < outer && outer <= le)).collect()
    }

    /// Does the store `st` set the cell the local `v` was read from (its
    /// reads `cells`), after the write `w`, so that the write lands in a
    /// block the store keeps and no one sees it between? `Err` the
    /// reason a copy is needed.
    fn keeps_write(&self, w: &AssignWrite, st: &StoreAt, cells: &[(String, String)], first_bind: u32) -> Result<(), &'static str> {
        let v = w.var;
        let key_written = self.assigns.iter().any(|a| a.var == v && a.depth == 1 && a.first.as_ref().is_some_and(|f| self.p.key_fields.contains(f)));
        // The same cell: the local itself (its key field unwritten), or a
        // literal keyed by the local's key field or by the read's key.
        match &st.arg {
            StoreArg::Var(x) if *x == v => {
                if key_written {
                    return Err(reasons::OTHER_KEY);
                }
            }
            StoreArg::Literal(inits) => {
                let rebound = |u: &str| match u.strip_prefix('@').and_then(|n| n.parse::<usize>().ok()) {
                    Some(x) => self.binds[x].iter().any(|b| b.at > first_bind && b.at < st.at),
                    None => self.param_rebinds.iter().any(|(n, at)| u.strip_prefix('$') == Some(n.as_str()) && *at > first_bind && *at < st.at),
                };
                let ok = !inits.is_empty()
                    && inits.iter().all(|k| match k {
                        KeyInit::FieldOf(x) => *x == v && !key_written,
                        KeyInit::Var(u) => cells.iter().all(|(_, key)| key == u) && !rebound(u),
                        KeyInit::Other => false,
                    });
                if !ok {
                    return Err(reasons::OTHER_KEY);
                }
            }
            _ => return Err(reasons::NO_SET),
        }
        // On every path after the write: a statement of a block the write
        // is in, with no way out between.
        if !st.block.is_some_and(|b| w.blocks.contains(&b)) {
            return Err(reasons::NOT_EVERY_PATH);
        }
        let around = self.loops_holding_not(w.at, st.at);
        let leaves = self.exits.iter().any(|&(at, jump)| {
            (at > w.at && at < st.at) || (!jump && around.iter().any(|&(ls, le)| ls < at && at <= le))
        });
        if leaves {
            return Err(reasons::NOT_EVERY_PATH);
        }
        // No read that may see the cell between the write and the store.
        let place = &cells[0].0;
        let last = |p: &str| p.rsplit('.').next().unwrap_or(p).to_string();
        let local = |p: &str| p.starts_with('@') || p.starts_with('$') || p.starts_with('?');
        let may_alias = |q: &Option<String>| match q {
            None => true,
            Some(q) => q == place || last(q) == last(place) || local(q) || local(place),
        };
        let seen = self.cell_reads.iter().any(|(at, q)| {
            may_alias(q) && ((*at > w.at && *at < st.at) || around.iter().any(|&(ls, le)| ls < *at && *at <= le))
        });
        if seen {
            return Err(reasons::READ_BEFORE_SET);
        }
        // The local is done with at the store: a later use would read
        // the cell's block (or a later replace's retire of it).
        if self.uses[v].iter().any(|&u| u > st.at) {
            return Err(reasons::OUTLIVES_SET);
        }
        // A form held in a local, rebound before the store, is another map.
        if let Some(fv) = place.strip_prefix('@').and_then(|n| n.split('.').next()).and_then(|n| n.parse::<usize>().ok()) {
            if fv < self.binds.len() && self.binds[fv].iter().any(|b| b.at > first_bind && b.at < st.at) {
                return Err(reasons::OTHER_KEY);
            }
        }
        Ok(())
    }

    /// The pass's answers for this body: the owned sites, and the
    /// cell-read rule's copies on write, flags and reports.
    fn finish(&self, found: &mut Found) {
        let (mut out, mut owned) = self.flagged();
        let mut reported: BTreeSet<usize> = BTreeSet::new();
        // An instance owned for the cell-read rule, reported at its
        // binding.
        struct Owning<'o> {
            out: &'o mut Vec<(Site, String)>,
            owned: &'o mut Vec<bool>,
            reported: &'o mut BTreeSet<usize>,
        }
        let own = |o: &mut Owning<'_>, i: usize, reason: &'static str, found: &mut Found| {
            if !o.owned[i] {
                o.owned[i] = true;
                self.own_inst(&self.insts[i], o.out);
            }
            if let Site::Node(id) = self.insts[i].site {
                found.cell_owned.insert(id);
            }
            if o.reported.insert(i) {
                found.cell_copies.push(CellCopy { program: 0, span: self.insts[i].span, operand: false, reason });
            }
        };
        let marked = |i: usize| {
            let (cells, inside) = self.markers(i);
            !cells.is_empty() || inside
        };
        // A local read from a cell (or into one) handed to a call that may
        // write through its parameter: owned from its binding.
        let mut o = Owning { out: &mut out, owned: &mut owned, reported: &mut reported };
        for &(v, _) in &self.passed {
            for &i in &self.vars[v] {
                if marked(i) {
                    own(&mut o, i, reasons::CALL_WRITES, found);
                }
            }
        }
        for span in &self.passed_copies {
            found.cell_copies.push(CellCopy { program: 0, span: *span, operand: true, reason: reasons::CALL_WRITES });
        }
        let mut cow_binds: HashMap<(Site, String), BTreeMap<String, bool>> = HashMap::new();
        for w in &self.assigns {
            let v = w.var;
            let insts: Vec<usize> = self.vars[v].iter().copied().filter(|&i| marked(i)).collect();
            if insts.is_empty() {
                continue;
            }
            let inside = insts.iter().any(|&i| self.markers(i).1);
            let flaggable = self.binds[v].iter().all(|b| b.flaggable);
            // A write into a value that is the cell's block itself, an
            // element written through a read's array, or a read bound
            // where no flag can be kept: the binding is owned.
            if inside || w.first.is_none() || !flaggable {
                for &i in &insts {
                    own(&mut o, i, reasons::VIEW_WRITTEN, found);
                }
                continue;
            }
            // A write to the read's own top level is the frame's, and so
            // is one into a part laid out inline in it.
            if w.depth < 2 || w.first.as_ref().is_some_and(|f| self.p.inline_fields.contains(f)) {
                continue;
            }
            let field = w.first.clone().expect("a field write");
            let shared: Vec<usize> = insts.iter().copied().filter(|&i| !o.owned[i]).collect();
            if shared.is_empty() {
                // Owned at its binding already (it is used after a write
                // of its form): the write is the frame's.
                for &i in &insts {
                    if o.reported.insert(i) {
                        found.cell_copies.push(CellCopy { program: 0, span: self.insts[i].span, operand: false, reason: reasons::OUTLIVES_SET });
                    }
                }
                continue;
            }
            let cells: Vec<(String, String)> = shared.iter().flat_map(|&i| self.markers(i).0).collect();
            let first_bind = shared.iter().map(|&i| self.insts[i].at).min().unwrap_or(0);
            let one_place = cells.iter().all(|(p, _)| *p == cells[0].0);
            let verdict = if !one_place {
                Err(reasons::NO_SET)
            } else {
                let mut verdict = Err(reasons::NO_SET);
                for st in self.stores.iter().filter(|st| st.at > w.at && st.place == cells[0].0) {
                    match self.keeps_write(w, st, &cells, first_bind) {
                        Ok(()) => {
                            verdict = Ok(());
                            break;
                        }
                        // The nearest store's reason, unless a later one
                        // keeps the write.
                        Err(r) => {
                            if verdict == Err(reasons::NO_SET) {
                                verdict = Err(r);
                            }
                        }
                    }
                }
                verdict
            };
            let Err(reason) = verdict else { continue };
            found.cow_writes.insert(w.id, field.clone());
            found.cell_copies.push(CellCopy { program: 0, span: w.span, operand: false, reason });
            for b in &self.binds[v] {
                let Site::Node(_) = b.site else { continue };
                let shared_here = b.inst.is_some_and(|i| shared.contains(&i));
                // A flag says the frame owns the part: a binding of a
                // fresh value, or of a copy, sets it; a read's sets it
                // clear.
                let entry =
                    cow_binds.entry((b.site, self.names[v].clone())).or_default().entry(field.clone()).or_insert(!shared_here);
                *entry = *entry && !shared_here;
            }
        }
        // The local-place rule: a local bound from a place is the frame's
        // own value. A write into it (at any depth) copies the whole value
        // at the first write, behind a flag per local; a local handed to a
        // call that may write through its parameter is bound as a copy.
        // A local the cell-read rule decides about is left to it.
        let mut record = self.record_hint.clone();
        for w in &self.assigns {
            // A write `v.name..` names a struct's field: `v` holds one.
            if w.first.as_ref().is_some_and(|f| !f.chars().all(|c| c.is_ascii_digit())) {
                record[w.var] = true;
            }
        }
        for &(v, record_param, _) in &self.place_passed {
            if record_param {
                record[v] = true;
            }
        }
        let cell_var = |v: usize| self.vars[v].iter().any(|&i| marked(i));
        // An instance the local-place rule may act on: not the cell rule's,
        // not copied at its binding already (the replaced-view rule's copy,
        // or a struct `let` of a place, GH #713), not a primitive.
        let open = |owned: &[bool], i: usize| {
            let inst = &self.insts[i];
            let v = inst.var;
            !marked(i) && !owned[i] && !(record[v] && inst.place_rhs) && !self.scalar_hint[v]
        };
        // Its value whole may be another name's block (a place marker), or
        // a field of it may (a struct literal's part markers).
        let whole = |i: usize| has_at(&self.insts[i].keys);
        let part_of = |i: usize, f: Option<&str>| {
            part_markers(&self.insts[i].keys).any(|(g, _)| g == "*" || f.is_none_or(|f| f == g))
        };
        // Owned for the local-place rule: a literal by every part that
        // holds a place's value, anything else whole.
        let own_place = |o: &mut Owning<'_>, i: usize, reason: &'static str, found: &mut Found| {
            let inst = &self.insts[i];
            if !o.owned[i] {
                o.owned[i] = true;
                match &inst.place_parts {
                    Some(parts) => o.out.extend(parts.iter().map(|s| (*s, inst.name.clone()))),
                    None => o.out.push((inst.site, inst.name.clone())),
                }
            }
            if let Site::Node(id) = inst.site {
                found.cell_owned.insert(id);
            }
            if o.reported.insert(i) {
                found.cell_copies.push(CellCopy { program: 0, span: inst.span, operand: false, reason });
            }
        };
        // Every marker kind meets every decision below: a write into the
        // local (whole: copy on write; a literal's part: copy the part on
        // write), a call that may write through what it is handed (whole
        // or a part), and an in-place write of the place it was read from
        // while it is live.
        //
        // The last first: a binding of a local's or a parameter's value
        // (or a part of it) that is live across a write into that value in
        // place sees the write, so it is bound as an owned copy instead.
        for &(ref written, at, deep) in &self.root_writes {
            let mut hit = Vec::new();
            // A binding of the written block or of a block holding it sees
            // a slot written; one of a block below it, only when anything
            // below may be written. (A slot replaced leaves the block it
            // held as it was.)
            let sees = |q: &str| {
                if deep {
                    places_overlap(q, written)
                } else {
                    q == "?" || written == q || written.strip_prefix(q).is_some_and(|r| r.starts_with('.'))
                }
            };
            for (i, inst) in self.insts.iter().enumerate() {
                if inst.at >= at {
                    continue;
                }
                let v = inst.var;
                // A call writing anything under the place: only a binding
                // known to hold a block that is neither a struct (copied at
                // its `let`) nor a primitive is owned for it, or a literal's
                // part holding the place.
                let by_whole = at_places(&inst.keys).any(sees) && (!deep || self.compound_hint[v]);
                let by_part = part_markers(&inst.keys).any(|(_, q)| sees(q));
                if !(by_whole || by_part) || !open(o.owned, i) {
                    continue;
                }
                // The written local's own instances are its rule's.
                if written.strip_prefix('@').and_then(|r| r.split('.').next()).and_then(|n| n.parse::<usize>().ok()) == Some(v) {
                    continue;
                }
                let live = self.uses[v].iter().any(|&u| {
                    u > at || self.loops.iter().any(|&(ls, le)| ls > self.declared[v] && ls <= u.min(at) && u.max(at) <= le)
                });
                if live {
                    hit.push(i);
                }
            }
            for i in hit {
                own_place(&mut o, i, reasons::PLACE_SOURCE_WRITTEN, found);
            }
        }
        // A local handed to a call that may write through what it is
        // handed: whole, or a literal's part it is handed (or every part,
        // the literal handed whole).
        for (v, _, first) in &self.place_passed {
            for &i in &self.vars[*v] {
                if open(o.owned, i) && (whole(i) || part_of(i, first.as_deref())) {
                    own_place(&mut o, i, reasons::PLACE_CALL_WRITES, found);
                }
            }
        }
        for w in &self.assigns {
            let v = w.var;
            // The instance a part assignment binds is bound after that
            // very write.
            let after = |i: usize| !(self.insts[i].at > w.at && self.insts[i].span == w.span);
            let mut shared: Vec<usize> =
                self.vars[v].iter().copied().filter(|&i| after(i) && open(o.owned, i) && whole(i)).collect();
            let mut part = String::new();
            if shared.is_empty() {
                // A struct literal's field holding a place's value: a write
                // below that field copies the field (`st.inner.k = 1`).
                let Some(f) = w.first.as_ref().filter(|_| w.depth >= 2) else { continue };
                shared = self.vars[v].iter().copied().filter(|&i| after(i) && open(o.owned, i) && part_of(i, Some(f))).collect();
                if shared.is_empty() {
                    continue;
                }
                part = f.clone();
            }
            // Read from a cell under another binding: no one flag can
            // serve both rules, so the place's instances are copied whole.
            if cell_var(v) {
                for i in shared {
                    own_place(&mut o, i, reasons::PLACE_WRITTEN, found);
                }
                continue;
            }
            if part.is_empty() && self.store_back_keeps(w, &shared) {
                continue;
            }
            found.cow_writes.insert(w.id, part.clone());
            found.cell_copies.push(CellCopy { program: 0, span: w.span, operand: false, reason: reasons::PLACE_WRITTEN });
            for b in &self.binds[v] {
                let shared_here = b.inst.is_some_and(|i| shared.contains(&i));
                let entry =
                    cow_binds.entry((b.site, self.names[v].clone())).or_default().entry(part.clone()).or_insert(!shared_here);
                *entry = *entry && !shared_here;
            }
        }
        for (key, fields) in cow_binds {
            found.cow_binds.insert(key, fields.into_iter().collect());
        }
        found.sites.extend(out);
    }

    /// Does a write `w` into a local bound from a self field (`shared`,
    /// its instances that still view the field) stay in the field's own
    /// block, unseen? The get-modify-store idiom: `let mut v = self.f;
    /// v.x[i] = y; self.f = v;` — every binding of the local is a
    /// `let` (or `=`) of that one field, a whole store of the local back
    /// into the same field follows the write on every path, nothing that
    /// may read the field (a read of it or of a field holding it, a
    /// call of the program's, a delivery) comes between, no other
    /// binding views the field, and the local is not used after the
    /// store. The store keeps the block the field already holds at each
    /// place (GH #1542), so the write is what the field would have held.
    fn store_back_keeps(&self, w: &AssignWrite, shared: &[usize]) -> bool {
        let v = w.var;
        let mut place: Option<&str> = None;
        for &i in &self.vars[v] {
            let inst = &self.insts[i];
            let mut places = at_places(&inst.keys);
            let (Some(p), None) = (places.next(), places.next()) else { return false };
            if !inst.place_rhs || !p.starts_with("self.") || place.is_some_and(|q| q != p) {
                return false;
            }
            place = Some(p);
        }
        let Some(place) = place else { return false };
        let first_bind = shared.iter().map(|&i| self.insts[i].at).min().unwrap_or(0);
        let segs: Vec<&str> = place.split('.').skip(1).collect();
        // No other binding views the field (it would see the write).
        let others = self.insts.iter().any(|o| {
            o.var != v
                && at_places(&o.keys).any(|q| places_overlap(q, place))
                && self.uses[o.var].iter().any(|&u| u > first_bind)
        });
        if others {
            return false;
        }
        self.self_stores.iter().filter(|st| st.var == v && st.place == place && st.at > w.at).any(|st| {
            if !st.block.is_some_and(|b| w.blocks.contains(&b)) {
                return false;
            }
            let around = self.loops_holding_not(w.at, st.at);
            let within = |at: u32| (at > w.at && at < st.at) || around.iter().any(|&(ls, le)| ls < at && at <= le);
            let leaves = self.exits.iter().any(|&(at, jump)| {
                (at > w.at && at < st.at) || (!jump && around.iter().any(|&(ls, le)| ls < at && at <= le))
            });
            !leaves
                && !self.field_reads.iter().any(|(at, n)| within(*at) && segs.contains(&n.as_str()))
                && !self.opaque_calls.iter().any(|&at| within(at))
                && !self.uses[v].iter().any(|&u| u > st.at)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(src: &str) -> Vec<String> {
        let program = hale_syntax::parse_source(src).expect("parses");
        let mut names: Vec<String> = owned_view_sites(&program).into_iter().map(|(_, n)| n).collect();
        names.sort();
        names
    }

    const LOCUS: &str = r#"
        type F { s: String = ""; n: Int = 0; }
        locus H {
            params { f: F = F { }; g: F = F { }; count: Int = 0; }
            fn put(t: String) { self.f = F { s: t }; }
            fn bump() { self.count = self.count + 1; }
            fn via() { self.put("x"); }
            fn body() { BODY }
        }
    "#;

    fn with(body: &str) -> Vec<String> {
        flagged(&LOCUS.replace("BODY", body))
    }

    #[test]
    fn a_local_held_across_a_replacing_call_is_owned() {
        assert_eq!(with(r#"let s = self.f.s; self.put("n"); println(s);"#), ["s"]);
        assert_eq!(with(r#"let s = self.f.s; self.via(); println(s);"#), ["s"]);
    }

    #[test]
    fn a_local_with_no_replacing_call_after_it_stays_a_view() {
        assert!(with(r#"let s = self.f.s; println(s); self.put("n");"#).is_empty());
        assert!(with(r#"let s = self.f.s; self.bump(); println(s);"#).is_empty());
        assert!(with(r#"let s = self.g.s; self.put("n"); println(s);"#).is_empty());
    }

    #[test]
    fn a_direct_store_reaches_a_later_use() {
        assert_eq!(with(r#"let s = self.f.s; self.f.s = "abc"; println(s);"#), ["s"]);
    }

    #[test]
    fn a_local_passed_to_the_writing_call_is_owned() {
        assert_eq!(with(r#"let s = self.f.s; self.put(s);"#), ["s"]);
    }

    #[test]
    fn a_loop_carries_a_later_write_to_an_earlier_use() {
        assert_eq!(
            with(r#"let s = self.f.s; let mut i = 0; while i < 2 { println(s); self.put("n"); i = i + 1; }"#),
            ["s"]
        );
        // Bound inside the loop: each iteration's binding is fresh.
        assert!(with(r#"let mut i = 0; while i < 2 { let s = self.f.s; println(s); self.put("n"); i = i + 1; }"#)
            .is_empty());
    }

    #[test]
    fn an_alias_of_a_view_is_a_view() {
        assert_eq!(with(r#"let s = self.f.s; let t = s; self.put("n"); println(t);"#), ["t"]);
    }

    #[test]
    fn a_field_passed_to_a_call_that_replaces_it_is_owned() {
        assert_eq!(with(r#"self.put(self.f.s);"#), ["f"]);
        assert!(with(r#"self.put(self.g.s);"#).is_empty());
    }

    #[test]
    fn a_match_payload_and_a_for_element_are_views() {
        assert_eq!(with(r#"for x in self.f.s { self.put("n"); println(x); }"#), ["x"]);
        // The next element is read after the write.
        assert_eq!(with(r#"for x in self.f.s { println(x); self.put("n"); }"#), ["x"]);
        assert!(with(r#"for x in self.f.s { println(x); self.bump(); }"#).is_empty());
        let src = r#"
            type E = enum { A(String), B };
            locus H {
                params { e: E = E::B; }
                fn set(t: String) { self.e = E::A(t); }
                fn body() {
                    match self.e { E::A(s) -> { self.set("n"); println(s); }, E::B -> { }, }
                }
            }
        "#;
        assert_eq!(flagged(src), ["s"]);
    }

    #[test]
    fn a_value_carried_out_of_a_match_if_or_block_is_owned() {
        let src = r#"
            type E = enum { A(String), B };
            locus H {
                params { e: E = E::B; flag: Bool = true; }
                fn clear() { self.e = E::B; }
                fn matched() -> String {
                    let held = match self.e { E::A(s) -> s, E::B -> "", };
                    self.clear();
                    return held;
                }
                fn cond() -> String {
                    let m = match self.e { E::A(s) -> s, E::B -> "", };
                    let held = if self.flag { m } else { "" };
                    self.clear();
                    return held;
                }
                fn block() -> String {
                    let held = { let t = match self.e { E::A(s) -> s, E::B -> "", }; t };
                    self.clear();
                    return held;
                }
                fn early() -> String {
                    let held = match self.e { E::A(s) -> s, E::B -> "", };
                    let out = held + "!";
                    self.clear();
                    return out;
                }
            }
        "#;
        let got = owned_view_sites(&hale_syntax::parse_source(src).expect("parses"));
        let lets = got.iter().filter(|(site, _)| matches!(site, Site::Node(_))).count();
        assert_eq!(lets, 3, "{got:?}");
    }

    #[test]
    fn a_literal_or_a_call_holding_a_view_is_a_view() {
        assert_eq!(with(r#"let t = (self.f.s, 1); self.put("n"); println(t.0);"#), ["t"]);
        assert_eq!(with(r#"let r = F { s: self.f.s }; self.put("n"); println(r.s);"#), ["r"]);
        // A call's result is copied out of its frame (a method's into
        // the caller's arena, a stdlib leaf's built fresh); concatenation
        // is a fresh value.
        assert!(with(r#"let t = std::str::trim(self.f.s); self.put("n"); println(t);"#).is_empty());
        assert!(with(r#"let t = self.via_name(); self.put("n"); println(t);"#).is_empty());
        assert!(with(r#"let t = self.f.s + "!"; self.put("n"); println(t);"#).is_empty());
    }

    #[test]
    fn an_outer_local_rebound_in_a_loop_carries_round_the_back_edge() {
        assert_eq!(
            with(r#"let mut held = ""; let mut i = 0; while i < 2 { println(held); held = self.f.s; self.put("n"); i = i + 1; }"#),
            ["held"]
        );
        // Rebound after the write: the next iteration reads it before
        // the next write.
        assert!(with(r#"let mut held = ""; let mut i = 0; while i < 2 { println(held); self.put("n"); held = self.f.s; i = i + 1; }"#)
            .is_empty());
        // A for, and a loop nested in another.
        assert_eq!(
            with(r#"let mut held = ""; for k in [1, 2] { println(held); held = self.f.s; self.put("n"); }"#),
            ["held"]
        );
        assert_eq!(
            with(r#"let mut held = ""; let mut i = 0; while i < 2 { let mut j = 0; while j < 2 { println(held); held = self.f.s; self.put("n"); j = j + 1; } i = i + 1; }"#),
            ["held"]
        );
    }

    #[test]
    fn a_field_of_a_record_the_frame_holds_is_not_a_view() {
        let src = r#"
            type F { s: String = ""; }
            locus K { params { name: String = ""; } fn rename() { self.name = "x"; } }
            locus H {
                params { f: F = F { }; }
                fn put() { self.f = F { s: "n" }; }
                fn make() -> F { return F { s: "m" }; }
                fn record() { let r = self.make(); let s = r.s; self.put(); println(s); }
                fn handle() { let k = K { }; let n = k.name; k.rename(); println(n); }
            }
        "#;
        assert_eq!(flagged(src), ["n"]);
    }

    #[test]
    fn an_operand_held_across_a_later_operands_write_is_owned() {
        let src = r#"
            type E = enum { A(String), B };
            type R { a: String = ""; n: Int = 0; }
            fn take(s: String, n: Int) -> String { return s + "!"; }
            locus H {
                params { e: E = E::B; f: R = R { }; }
                fn reset() -> Int { self.e = E::B; self.f = R { }; return 0; }
                fn other() -> Int { return 1; }
                fn arg() -> String { return take(match self.e { E::A(s) -> s, E::B -> "", }, self.reset()); }
                fn binary() -> String { return self.f.a + to_string(self.reset()); }
                fn literal() -> R { return R { a: self.f.a, n: self.reset() }; }
                fn fine() -> String { return take(self.f.a, self.other()); }
            }
        "#;
        let got = owned_view_sites(&hale_syntax::parse_source(src).expect("parses"));
        assert_eq!(got.len(), 3, "{got:?}");
    }

    #[test]
    fn a_handle_is_known_by_its_resolved_type() {
        // Through an alias, through a binding's declared type, and by an
        // imported seed's minted name.
        let src = r#"
            type E = enum { A(String), B };
            locus Kid { params { e: E = E::B; } fn clear() { self.e = E::B; } }
            locus __lib_x_f_Ward { params { e: E = E::B; } fn clear() { self.e = E::B; } }
            type Child = Kid;
            fn child(k: Kid) -> Child { return k; }
            fn ward(w: x::Ward) -> x::Ward { return w; }
            fn opaque(k: Kid) -> Kid { return k; }
            locus H {
                params { k: Kid = Kid { }; w: x::Ward = x::Ward { }; }
                fn aliased() -> String { let c = child(self.k); let held = match c.e { E::A(s) -> s, E::B -> "", }; c.clear(); return held; }
                fn declared() -> String { let c: Kid = opaque(self.k); let held = match c.e { E::A(s) -> s, E::B -> "", }; c.clear(); return held; }
                fn imported() -> String { let c = ward(self.w); let held = match c.e { E::A(s) -> s, E::B -> "", }; c.clear(); return held; }
            }
        "#;
        let renames: BTreeMap<Vec<String>, String> =
            [(vec!["x".to_string(), "Ward".to_string()], "__lib_x_f_Ward".to_string())].into_iter().collect();
        let got = owned_view_sites_with(&hale_syntax::parse_source(src).expect("parses"), &renames);
        let lets = got.iter().filter(|(site, _)| matches!(site, Site::Node(_))).count();
        assert_eq!(lets, 3, "{got:?}");
    }

    #[test]
    fn a_callee_it_cannot_identify_writes_anything() {
        // An unknown free fn, an unknown method, a fn-valued field through
        // an alias and a field of a generic parameter's type: each may
        // write the field the binding views.
        let src = r#"
            type F { s: String = ""; }
            type Callback = fn(Int) -> Int;
            locus Hook<T> { params { cb: T; } }
            locus H {
                params { f: F = F { }; cb: Callback = nothing; k: Hook<Int> = Hook { cb: 0 }; }
                fn unknown_fn() -> String { let s = self.f.s; mystery(1); return s; }
                fn unknown_method() -> String { let s = self.f.s; self.k.vanish(); return s; }
                fn aliased_field() -> String { let s = self.f.s; self.cb(1); return s; }
                fn generic_field() -> String { let s = self.f.s; self.k.cb(1); return s; }
                fn known() -> String { let s = self.f.s; println("x"); return s; }
            }
        "#;
        let got = owned_view_sites(&hale_syntax::parse_source(src).expect("parses"));
        let lets = got.iter().filter(|(site, _)| matches!(site, Site::Node(_))).count();
        assert_eq!(lets, 4, "{got:?}");
    }

    /// Every question the pass asks of a type goes through `classify`:
    /// no `TypeExpr` variant is matched anywhere else, so a new spelling
    /// (an alias, an import, a minted name) cannot be classified one way
    /// in one place and another way in the next.
    #[test]
    fn types_are_classified_in_one_place() {
        let src = include_str!("view_bindings.rs");
        let body_start = src.find("    fn classify(&self, ty: &TypeExpr) -> Class {").expect("classify");
        let body_end = src.find("    /// A declared name's class").expect("classify_name");
        let needle = ["Type", "Expr::"].concat();
        for (i, _) in src.match_indices(&needle) {
            assert!(
                (body_start..body_end).contains(&i),
                "a type is matched outside `classify`, at byte {i}: {}",
                &src[i..src.len().min(i + 60)]
            );
        }
    }

    #[test]
    fn a_forms_read_is_a_view_of_its_cell() {
        // A form's synthesized `set` is the form's, not some other
        // locus's method named `set`; its read views the cell its later
        // write retires, and its arguments are copied in before it writes.
        let src = r#"
            type C { id: Int = 0; s: String = ""; }
            @form(hashmap)
            locus M { capacity { pool cells of C indexed_by id; } }
            locus Other { params { n: Int = 0; } fn set(c: C) { mystery(); } }
            fn rmw() {
                let m = M { };
                let e = m.get(1) or C { };
                m.set(C { id: 1, s: e.s });
            }
            fn held() {
                let m = M { };
                let e = m.get(1) or C { };
                m.set(C { id: 1, s: "x" });
                println(e.s);
            }
        "#;
        assert_eq!(flagged(src), ["e"]);
    }

    #[test]
    fn a_member_named_like_a_form_op_is_a_value_call() {
        // `pop` on a record member is no form's `pop`: only a receiver
        // known to hold a form takes a form's synthesized method.
        let src = r#"
            type F { s: String = ""; }
            type Hooks { pop: fn(Int) -> Int; }
            locus H {
                params { f: F = F { }; hooks: Hooks = Hooks { pop: one }; }
                fn hold() -> String { let s = self.f.s; self.hooks.pop(1); return s; }
            }
            fn one(x: Int) -> Int { return x; }
        "#;
        assert_eq!(flagged(src), ["s"]);
    }

    #[test]
    fn an_enum_constructor_holds_its_argument() {
        let src = r#"
            type E = enum { A(String), B };
            type F { s: String = ""; }
            locus H {
                params { f: F = F { }; }
                fn put() { self.f = F { s: "n" }; }
                fn body() { let e = E::A(self.f.s); self.put(); println(e); }
            }
        "#;
        assert_eq!(flagged(src), ["e"]);
    }

    #[test]
    fn a_publish_runs_the_handlers_writes() {
        let src = r#"
            type F { s: String = ""; }
            locus H {
                params { f: F = F { }; g: F = F { }; }
                bus { subscribe "t" as on_t of type String; }
                fn on_t(m: String) { self.f = F { s: m }; }
                fn held() { let s = self.f.s; "t" <- "x"; println(s); }
                fn other() { let s = self.g.s; "t" <- "x"; println(s); }
            }
        "#;
        assert_eq!(flagged(src), ["s"]);
    }

    #[test]
    fn a_fn_value_may_write_anything() {
        let src = r#"
            type F { s: String = ""; }
            locus H {
                params { g: F = F { }; }
                fn hold(cb: fn(Int) -> Int) { let s = self.g.s; cb(1); println(s); }
            }
        "#;
        assert_eq!(flagged(src), ["s"]);
        // A yield with no handler in the program runs nothing.
        assert!(with(r#"let s = self.g.s; yield; println(s);"#).is_empty());
    }

    /// The cell-read rule's copies in `body` (a fn of the program below,
    /// `m` a map that reads in place), each as its statement's text and
    /// its reason.
    fn cell_copies(body: &str) -> Vec<(String, &'static str)> {
        let src = CELLS.replace("BODY", body);
        let program = hale_syntax::parse_source(&src).expect("parses");
        let found = analyze(&[&program], &BTreeMap::new());
        // The local-place rule's copies (in the helpers' bodies) are its
        // own tests'.
        found
            .cell_copies
            .iter()
            .filter(|c| !reasons::is_place(c.reason))
            .map(|c| (src[c.span.start.0 as usize..c.span.end.0 as usize].to_string(), c.reason))
            .collect()
    }

    const CELLS: &str = r#"
        type In { s: String = ""; k: Int = 0; }
        type Count = Int;
        type Hits = distinct Int;
        type Rec { id: Int = 0; body: String = ""; inner: In = In { }; big: [In; 4] = [In { }; 4]; counts: [Int; 4] = [0, 0, 0, 0];
            tallies: [Count; 2] = [0, 0]; hits: [Hits; 2] = [Hits(0), Hits(0)]; }
        @form(hashmap)
        locus RecMap { capacity { pool items of Rec indexed_by id; } }
        @form(hashmap, sync = serialized)
        locus Deep { capacity { pool items of Rec indexed_by id; } }
        fn peek(mm: RecMap, k: Int) -> Int { let c = mm.get(k) or Rec { }; return c.inner.k; }
        fn poke(r: Rec) { r.inner.k = 9; }
        fn poke_alias(r: Rec) { let mut a = r.big; a[0].k = 7; }
        fn poke_for(r: Rec) { for x in r.big { x.k = 7; } }
        fn poke_arr(xs: [In; 4]) { xs[0].k = 7; }
        fn poke_hand(r: Rec) { let a = r.big; poke_arr(a); }
        fn read_only(r: Rec) { let a = r.big; for x in a { println(x.s); } }
        fn show(r: Rec) { println(r.body); }
        fn body(m: RecMap, d: Deep, k: Int, flag: Bool) { BODY }
    "#;

    #[test]
    fn a_get_modify_set_keeps_the_read_shared() {
        // The idiom: the write lands in the cell's block, which the set
        // keeps; nothing reads the cell between.
        assert!(cell_copies("let mut e = m.get(k) or Rec { id: k }; e.big[1] = In { k: 2 }; e.inner.k = 3; m.set(e);").is_empty());
        // A literal keyed by the read's key, or by the local's key field.
        assert!(cell_copies("let mut e = m.get(k) or Rec { id: k }; e.inner.k = 3; m.set(Rec { id: k, inner: e.inner });").is_empty());
        assert!(cell_copies("let mut e = m.get(k) or Rec { id: k }; e.inner.k = 3; m.set(Rec { id: e.id, inner: e.inner });").is_empty());
        // A write to the read's own top level is the frame's, and so is
        // one into a part laid out inline in it.
        assert!(cell_copies("let mut e = m.get(k) or Rec { }; e.body = \"x\"; e.id = 4; println(e.body);").is_empty());
        assert!(cell_copies("let mut e = m.get(k) or Rec { }; e.counts[1] = 4; println(e.counts[1]);").is_empty());
        // Whatever the element is spelled as: an alias of a scalar, a
        // scalar declaration lowered as an `Int`.
        assert!(cell_copies("let mut e = m.get(k) or Rec { }; e.tallies[1] = 4; println(e.tallies[1]);").is_empty());
        assert!(cell_copies("let mut e = m.get(k) or Rec { }; e.hits[1] = Hits(4); println(e.id);").is_empty());
        // A map that reads a deep copy hands back the frame's own value.
        assert!(cell_copies("let mut e = d.get(k) or Rec { }; e.inner.k = 3; println(e.inner.k);").is_empty());
        // The write in a loop, the set after it.
        assert!(cell_copies("let mut e = m.get(k) or Rec { id: k }; let mut i = 0; while i < 4 { e.big[i] = In { k: i }; i = i + 1; } m.set(e);").is_empty());
    }

    #[test]
    fn a_write_the_cell_could_show_copies_on_write() {
        use reasons::*;
        assert_eq!(
            cell_copies("let mut e = m.get(k) or Rec { }; e.inner.k = 3; println(e.inner.k);"),
            [("e.inner.k = 3;".to_string(), NO_SET)]
        );
        assert_eq!(
            cell_copies("let mut e = m.get(k) or Rec { id: k }; e.inner.k = 3; if flag { m.set(e); }"),
            [("e.inner.k = 3;".to_string(), NOT_EVERY_PATH)]
        );
        assert_eq!(
            cell_copies("let mut e = m.get(k) or Rec { id: k }; e.inner.k = 3; if flag { return; } m.set(e);"),
            [("e.inner.k = 3;".to_string(), NOT_EVERY_PATH)]
        );
        assert_eq!(
            cell_copies("let mut e = m.get(k) or Rec { id: k }; e.inner.k = 3; let n = peek(m, 2); m.set(e);"),
            [("e.inner.k = 3;".to_string(), READ_BEFORE_SET)]
        );
        assert_eq!(
            cell_copies("let mut e = m.get(k) or Rec { id: k }; e.inner.k = 3; m.set(Rec { id: 7, inner: e.inner });"),
            [("e.inner.k = 3;".to_string(), OTHER_KEY)]
        );
        assert_eq!(
            cell_copies("let mut e = m.get(k) or Rec { id: k }; e.id = 8; e.inner.k = 3; m.set(e);"),
            [("e.inner.k = 3;".to_string(), OTHER_KEY)]
        );
    }

    #[test]
    fn a_read_used_after_its_set_is_owned_from_its_binding() {
        // The set is a write of the map the local is used after: the
        // binding is an owned copy, and the write is the frame's.
        let got = cell_copies("let mut e = m.get(k) or Rec { id: k }; e.inner.k = 3; m.set(e); println(e.inner.k);");
        assert_eq!(got, [("let mut e = m.get(k) or Rec { id: k };".to_string(), reasons::OUTLIVES_SET)]);
    }

    #[test]
    fn a_binding_into_the_cell_written_or_handed_to_a_writer_is_owned() {
        use reasons::*;
        assert_eq!(
            cell_copies("let e = m.get(k) or Rec { }; poke(e);"),
            [("let e = m.get(k) or Rec { };".to_string(), CALL_WRITES)]
        );
        // A callee that writes nothing through its parameter shares it.
        assert!(cell_copies("let e = m.get(k) or Rec { }; show(e);").is_empty());
        assert!(cell_copies("let e = m.get(k) or Rec { }; read_only(e);").is_empty());
        // A callee writes through its parameter by a local bound from a
        // part of it, by a `for` element of it, and by handing that local
        // to another writer.
        for callee in ["poke_alias", "poke_for", "poke_hand"] {
            assert_eq!(
                cell_copies(&format!("let e = m.get(k) or Rec {{ }}; {callee}(e);")),
                [("let e = m.get(k) or Rec { };".to_string(), CALL_WRITES)],
                "{callee}"
            );
            assert_eq!(
                cell_copies(&format!("{callee}(m.get(k) or Rec {{ }});")),
                [("m.get(k) or Rec { }".to_string(), CALL_WRITES)],
                "{callee}"
            );
        }
        assert_eq!(cell_copies("poke(m.get(k) or Rec { });"), [("m.get(k) or Rec { }".to_string(), CALL_WRITES)]);
        assert_eq!(
            cell_copies("let e = m.get(k) or Rec { }; for x in e.big { x.k = 1; }"),
            [("for x in e.big { x.k = 1; }".to_string(), VIEW_WRITTEN)]
        );
        assert_eq!(
            cell_copies("for e in m.entries { e.id = 1; }"),
            [("for e in m.entries { e.id = 1; }".to_string(), VIEW_WRITTEN)]
        );
    }

    /// The local-place rule's copies in `body` (a method of `H` below),
    /// each as its statement's text and its reason.
    fn place_copies(body: &str) -> Vec<(String, &'static str)> {
        let src = PLACES.replace("BODY", body);
        let program = hale_syntax::parse_source(&src).expect("parses");
        let found = analyze(&[&program], &BTreeMap::new());
        let start = src.find("fn body(").expect("the body") as u32;
        found
            .cell_copies
            .iter()
            .filter(|c| reasons::is_place(c.reason) && c.span.start.0 >= start)
            .map(|c| (src[c.span.start.0 as usize..c.span.end.0 as usize].to_string(), c.reason))
            .collect()
    }

    const PLACES: &str = r#"
        type In { k: Int = 0; s: String = ""; }
        type P = enum { A(In), B };
        type R { arr: [In; 2] = [In { }, In { }]; inner: In = In { }; e: P = P::B; }
        @form(vec)
        locus V { capacity { heap cells of In; } }
        fn poke(a: [In; 2]) { a[0].k = 1; }
        fn look(a: [In; 2]) -> Int { return a[0].k; }
        fn poke_second(a: [In; 2], b: [In; 2]) { b[0].k = 1; }
        fn poke_via(a: [In; 2]) { poke(a); }
        fn poke_copy(a: [In; 2]) { let mut c = a; c[0].k = 1; }
        fn poke_rec(r: R) { r.arr[0].k = 8; }
        type Out { flag: Bool = false; o: In = In { }; }
        fn mk(r: R) -> Out { return Out { o: r.arr[0] }; }
        fn whole(r: R) -> In { return r.arr[0]; }
        fn one() -> Int { return 1; }
        type Hits = distinct Int;
        fn hits() -> Hits { return Hits(1); }
        locus H {
            params { r: R = R { }; arr: [In; 2] = [In { }, In { }]; v: V = V { }; }
            fn other() -> Int { return 1; }
            fn body(p: R, cb: fn(Int) -> Int) { BODY }
        }
    "#;

    #[test]
    fn a_write_into_a_local_bound_from_a_place_copies_on_write() {
        use reasons::*;
        assert_eq!(place_copies("let mut a = self.r.arr; a[0].k = 3;"), [("a[0].k = 3;".to_string(), PLACE_WRITTEN)]);
        assert_eq!(place_copies("let mut a = p.arr; a[1] = In { };"), [("a[1] = In { };".to_string(), PLACE_WRITTEN)]);
        assert_eq!(
            place_copies("match self.r.e { P::A(i) -> { i.k = 1; }, P::B -> { }, }"),
            [("i.k = 1;".to_string(), PLACE_WRITTEN)]
        );
        assert_eq!(place_copies("for x in p.arr { x.k = 1; }"), [("x.k = 1;".to_string(), PLACE_WRITTEN)]);
        // A struct literal is fresh: only a write below a field holding a
        // place's value copies, and only that field.
        assert!(place_copies("let mut l = R { arr: self.arr }; l.inner.k = 2; l.e = P::B;").is_empty());
        assert_eq!(place_copies("let mut l = R { arr: self.arr }; l.arr[0].k = 2;"), [("l.arr[0].k = 2;".to_string(), PLACE_WRITTEN)]);
    }

    #[test]
    fn a_read_or_a_struct_let_of_a_place_copies_nothing_more() {
        // Reads stay views; a struct `let` of a place is a copy already
        // (GH #713); a vec's walk visits its cells in place.
        assert!(place_copies("let a = self.r.arr; let n = look(a); println(a[0].k + n);").is_empty());
        assert!(place_copies("let mut c = self.r; c.inner.k = 1; c.arr[0].k = 2;").is_empty());
        assert!(place_copies("for x in self.v.items { x.k = 1; }").is_empty());
        // A place marker is no key a write replaces: an operand that
        // carries only one is not copied for a later operand's unknown
        // call (which may write anything).
        let src = PLACES.replace("BODY", "let n = look(p.arr, mystery(1)); println(n);");
        let program = hale_syntax::parse_source(&src).expect("parses");
        assert!(analyze(&[&program], &BTreeMap::new()).sites.is_empty());
    }

    #[test]
    fn a_get_modify_store_of_a_self_field_stays_in_the_field() {
        use reasons::*;
        assert!(place_copies("let mut a = self.arr; a[0].k = 3; a[1].s = \"x\"; self.arr = a;").is_empty());
        // Something may read the field between the write and the store.
        assert_eq!(
            place_copies("let mut a = self.arr; a[0].k = 3; let n = self.other(); self.arr = a;"),
            [("a[0].k = 3;".to_string(), PLACE_WRITTEN)]
        );
        assert_eq!(
            place_copies("let mut a = self.arr; a[0].k = 3; println(self.arr[0].k); self.arr = a;"),
            [("a[0].k = 3;".to_string(), PLACE_WRITTEN)]
        );
        // Used after the store, which replaces the field: the binding is
        // an owned copy already (the replaced-view rule), so the write is
        // the frame's.
        let src = PLACES.replace("BODY", "let mut a = self.arr; a[0].k = 3; self.arr = a; println(a[0].k);");
        let program = hale_syntax::parse_source(&src).expect("parses");
        assert_eq!(analyze(&[&program], &BTreeMap::new()).sites.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["a"]);
        assert!(place_copies("let mut a = self.arr; a[0].k = 3; self.arr = a; println(a[0].k);").is_empty());
        // The store not on every path.
        assert_eq!(
            place_copies("let mut a = self.arr; a[0].k = 3; if p.inner.k > 0 { self.arr = a; }"),
            [("a[0].k = 3;".to_string(), PLACE_WRITTEN)]
        );
        // Another binding views the field and would see the write.
        assert_eq!(
            place_copies("let b = self.arr; let mut a = self.arr; a[0].k = 3; self.arr = a; println(b[0].k);"),
            [("a[0].k = 3;".to_string(), PLACE_WRITTEN)]
        );
    }

    #[test]
    fn a_local_handed_to_a_writer_or_outliving_a_write_of_its_source_is_owned() {
        use reasons::*;
        assert_eq!(
            place_copies("let a = self.r.arr; poke(a);"),
            [("let a = self.r.arr;".to_string(), PLACE_CALL_WRITES)]
        );
        // Only the parameter a callee writes through, transitively, and
        // not through a local bound from it (which copies on its write).
        assert!(place_copies("let a = self.r.arr; poke_second(a, [In { }, In { }]);").is_empty());
        assert_eq!(
            place_copies("let a = self.r.arr; poke_via(a);"),
            [("let a = self.r.arr;".to_string(), PLACE_CALL_WRITES)]
        );
        assert!(place_copies("let a = self.r.arr; poke_copy(a);").is_empty());
        assert_eq!(
            place_copies("let a = p.arr; p.arr[0].k = 4; println(a[0].k);"),
            [("let a = p.arr;".to_string(), PLACE_SOURCE_WRITTEN)]
        );
        // A slot of the source replaced leaves the block it held alone.
        assert!(place_copies("let a = p.arr; p.arr = [In { }, In { }]; println(a[0].k);").is_empty());
    }

    /// Every place-marker kind — a whole value read from a place (`AT`),
    /// a struct literal's field holding one (`PART`) — meets every copy
    /// decision: a write into the local, a call that may write through
    /// what it is handed (the local whole, or the part), and an in-place
    /// write of the source while the local is live. Each pair copies.
    #[test]
    fn every_place_marker_meets_every_copy_decision() {
        use reasons::*;
        let kinds = [("whole", "let mut v = self.r.arr;", "v[0]", "v"), ("part", "let mut v = R { arr: p.arr };", "v.arr[0]", "v.arr")];
        for (kind, bind, elem, part) in kinds {
            let write = format!("{bind} {elem}.k = 3;");
            let got = place_copies(&write);
            assert_eq!(got.len(), 1, "{kind} write: {got:?}");
            assert_eq!(got[0].1, PLACE_WRITTEN, "{kind} write");
            // A call handed the local whole, and handed the shared part.
            for passed in ["poke_rec(v);".to_string(), format!("poke({part});")] {
                if kind == "whole" && passed.starts_with("poke_rec") {
                    continue; // an array is no record
                }
                let got = place_copies(&format!("{bind} {passed}"));
                assert_eq!(got, [(bind.to_string(), PLACE_CALL_WRITES)], "{kind} handed to {passed}");
            }
        }
        // The source written in place while the local is live, for both
        // kinds (a parameter's array as the source).
        for bind in ["let v = p.arr;", "let v = R { arr: p.arr };"] {
            let got = place_copies(&format!("{bind} p.arr[0].k = 4; println(v.inner.k);"));
            assert_eq!(got, [(bind.to_string(), PLACE_SOURCE_WRITTEN)], "{bind}");
        }
    }

    /// A primitive never carries a place marker, however it is known: a
    /// call's declared return, a typed scalar, a builtin's or a stdlib
    /// leaf's signature, an alias of one. Lowering copies nothing for one,
    /// and the pass reports nothing.
    #[test]
    fn a_primitive_is_never_a_place_copy() {
        for body in [
            "let a = one(); let b = a; cb(b);",
            "let a = hits(); let b = a; cb(1); poke_second(self.arr, self.arr); println(b);",
            "let a = len(\"x\"); let b = a; cb(b);",
            "let a = std::str::upper(\"x\"); let b = a; cb(1); println(b);",
            "let a = one() or 2; let b = a; cb(b);",
        ] {
            assert!(place_copies(body).is_empty(), "{body}: {:?}", place_copies(body));
        }
    }

    /// A call's result fresh at its top level but holding a parameter's
    /// block in a field (`mk` returns a literal around `r.arr[0]`): a write
    /// into its own fields copies nothing, a write below that field copies
    /// the field; a result that is the parameter's block itself copies at
    /// any write (CI's dna/core verify gate: `x.reconciled = true` on such
    /// a result was a reported copy in a loop).
    #[test]
    fn a_result_holding_a_place_in_a_part_copies_only_below_it() {
        use reasons::*;
        assert!(place_copies("let mut x = mk(p); x.flag = true;").is_empty());
        assert_eq!(place_copies("let mut x = mk(p); x.o.k = 7;"), [("x.o.k = 7;".to_string(), PLACE_WRITTEN)]);
        assert_eq!(place_copies("let mut y = whole(p); y.k = 7;"), [("y.k = 7;".to_string(), PLACE_WRITTEN)]);
    }
}
