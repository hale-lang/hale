//! GH #921 A2 — locus ownership resolved before lowering, in shadow
//! mode.
//!
//! `spec/decisions.md` F.39 is the design. The short version: locus
//! ownership is decided today by seven one-shot flags on `Cx`
//! (`suppress_fresh_temp`, `defer_next_locus_dissolve`,
//! `instantiating_for_parent_field`,
//! `placement_for_next_locus_instantiation`, `or_field_owner_locus`,
//! the `returns_this_locus` / `current_user_fn_ret` spoof, and the
//! field-ownership predicates), each consumed by "the next literal or
//! call lowered". Every teardown leak the 2026-09 sweep fixed was a
//! flag taken by the wrong node. This module computes the same
//! decisions ONCE, from syntactic position, into a side table keyed by
//! expression identity — and, in this PR, only CHECKS the table
//! against what the flags do. Nothing here changes what is emitted.
//!
//! ## The key
//!
//! [`ExprId`] is the [`NodeId`] this pass writes into the AST, in
//! pre-order over the merged, desugared program. It has to live in the
//! node because codegen lowers CLONES of every declaration —
//! `locus_decls` and `user_fn_decls` are `Vec`s of cloned decls, and
//! `lower_locus_instantiation` clones the whole `LocusInfo` (param
//! default expressions included) on every instantiation. A node's
//! ADDRESS is therefore not its identity. Neither is its span: the
//! stdlib is parsed in its own coordinate space and overlaps user file
//! ranges (the reason DWARF emission skips stdlib-mangled fns), so two
//! different expressions can carry one span.
//!
//! Only the two shapes that can PRODUCE a locus carry an id — a struct
//! literal and a call. A node codegen synthesises after this pass
//! (a generic specialisation's body, the `Stmt::Let` generic-path
//! rewrite when it does not carry the id over) keeps `NodeId::NONE`
//! and is reported as `unindexed` rather than as a missing decision.
//!
//! ## The derivation
//!
//! A *locus-producing* expression is a locus literal, a call to a
//! proven-fresh factory, an `or` whose branches are those, a carrier
//! (`if` / `match` / block) whose arm tails are those, or an element of
//! an array / tuple of those. The site decides; a carrier never
//! consumes the decision for its arms, and a composite never consumes
//! it for its elements — each branch, arm and element is decided
//! separately.
//!
//! | position | owner |
//! |---|---|
//! | `let x = …`, `x = …` | [`Owner::Binding`] |
//! | `return …` | [`Owner::Caller`] |
//! | a locus / interface / perspective param field's initialiser | [`Owner::Field`] |
//! | the same field initialised from a NAME | [`Owner::Borrowed`] |
//! | a `placement { }`'d field, a `bindings { }` transport | [`Owner::Placement`] |
//! | everything else — receiver, argument, operand, index, condition, bare statement | [`Owner::FrameTemp`] |
//!
//! A decision that passes THROUGH a delegating node — an `or`, a
//! carrier, a composite — is demoted from `Binding` to
//! `FrameTemp(innermost reclaim scope)`, because the binding names the
//! join and not the arm: the arm's value is materialised in a
//! temporary the frame reclaims, which is exactly what lowering does
//! and what GH #921 A4 asks A3 to keep doing with a per-iteration
//! slot. `Caller`, `Field` and `Placement` are not demoted — the value
//! really does leave the frame, or the field's mask bit really does
//! claim whichever branch ran (GH #853).
//!
//! The innermost reclaim scope is the enclosing frame, or the
//! enclosing LOOP body when there is one: GH #824 gave a `let` in a
//! loop a per-iteration slot, and A4 found that a carrier or composite
//! RHS — which takes the GH #402 frame temporary instead — still does
//! not have one. That difference is what [`ScopeKind`] carries, and it
//! is the whole of the fourth open family.
//!
//! ## The fresh-factory set
//!
//! The table derives factory calls from an EXTENDED set: the one
//! `compute_fresh_locus_factories` computes, plus every fn whose
//! return arms are fresh once `if` / `match` / block tails are
//! flattened, and every fn that hands back a BINDING of one.
//! `compute_fresh_locus_factories::collect` classifies the carrier
//! node and never its arms, which is why `return if c { make(1) }
//! else { make(2) }` was not a factory and its caller's binding did
//! not own the result — the 105-cell carrier-return family.
//!
//! GH #921 A3 commit 1 folds the extension back into the map lowering
//! reads ([`OwnerTable::extended_fresh_factories`], applied in
//! `lower_program`), so the two sides of every ownership decision are
//! computed once and cannot drift. It lives here rather than inside
//! `compute_fresh_locus_factories` because the flattening is the same
//! walk the table already does to decide each arm.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    Block, ElseBranch, Expr, FnDecl, IfStmt, LValueSeg, LocusDecl,
    LocusMember, MatchArmBody, MatchStmt, ModuleDecl, NodeId,
    OrDisposition, Param, ParamInit, Program, QualifiedName, Stmt,
    StructInit, TopDecl, TypeExpr,
};
use hale_syntax::Span;

// ===================================================================
// Ids
// ===================================================================

/// The identity of a locus-producing expression: the [`NodeId`] this
/// pass writes into the node. See the module docs for why it cannot be
/// an address or a span.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct ExprId(pub u32);

impl ExprId {
    /// Stands in for an expression that has no numbered node: the
    /// declaring locus's own `params { }` text as the owner of a
    /// field, and the NAME a borrowed field initialiser reads
    /// through (`Expr::Ident` carries no id — see the module docs).
    pub const DECLARED: ExprId = ExprId(u32::MAX);
}

/// A local slot a binding names, unique within the program.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct SlotId(pub u32);

/// A reclaim scope: a frame, or one iteration of a loop inside it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct ScopeId(pub u32);

/// What a [`ScopeId`] reclaims at.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum ScopeKind {
    /// The enclosing fn / method / lifecycle body: flushed once, at
    /// the frame's exit.
    Frame,
    /// One iteration of a `while` / `for` body: the slot is reclaimed
    /// when the next iteration reuses it (GH #815 / #824).
    LoopIteration,
}

// ===================================================================
// The owner
// ===================================================================

/// Who reclaims a locus-producing expression's value. F.39's enum.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Owner {
    /// `let x = …` / `x = …` into a local slot.
    Binding(SlotId),
    /// A param field's initialiser — locus, interface or perspective
    /// typed. `owner` is the literal that built the holder, or
    /// [`ExprId::DECLARED`] for the declaring locus's own default
    /// text.
    Field { owner: ExprId, field: String },
    /// An expression-position value nobody binds: a receiver, an
    /// argument, a field read, an operand, a bare statement. Dissolved
    /// at the scope's flush.
    FrameTemp(ScopeId),
    /// The value a `return` hands out, in every arm of a carrier.
    Caller,
    /// A handle passed in — an interface value from a parent's field,
    /// a designated perspective slot. Never torn down here (GH #730 /
    /// #731).
    Borrowed(ExprId),
    /// A `placement { }` entry's instance (pinned thread, program
    /// lifetime), including the bindings transport (GH #893).
    Placement(String),
}

/// What the owner means for WHEN the value is reclaimed — the
/// granularity both the table and the flags can answer at a locus
/// INSTANTIATION, and so the granularity the shadow compares there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Disposition {
    /// This frame reclaims it at a flush, or eagerly at the end of its
    /// own expression. `per_iteration` is true when that happens once
    /// per loop iteration rather than once per frame.
    Frame { per_iteration: bool },
    /// An owning locus reclaims it: a param field's mask bit, or an
    /// acceptor's `__children[]` spine.
    Owned,
    /// The caller reclaims it.
    Caller,
    /// A placement entry: pinned, program lifetime.
    Placement,
    /// A handle passed in; nothing in this frame reclaims it.
    Borrowed,
}

impl std::fmt::Display for Disposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Disposition::Frame { per_iteration: true } => {
                write!(f, "frame (per iteration)")
            }
            Disposition::Frame { per_iteration: false } => {
                write!(f, "frame (per frame)")
            }
            Disposition::Owned => write!(f, "owner field"),
            Disposition::Caller => write!(f, "caller"),
            Disposition::Placement => write!(f, "placement"),
            Disposition::Borrowed => write!(f, "borrowed"),
        }
    }
}

/// The finer question a FACTORY CALL site can answer on both sides.
/// The flags there decide one bit — "does this frame register a GH
/// #402 / #793 temporary for the value" — and, when it does, at what
/// granularity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TempVerdict {
    /// This frame registers a temporary. `per_iteration` is true when
    /// the slot carries GH #815's reuse teardown, so a loop reclaims
    /// every iteration's value rather than only the last.
    FrameTemp { per_iteration: bool },
    /// The site owns it — a binding, a param field, the caller, a
    /// placement entry.
    SiteOwned,
    /// Nothing registers anything for this value.
    Nobody,
}

impl std::fmt::Display for TempVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TempVerdict::FrameTemp { per_iteration: true } => {
                write!(f, "frame temporary (per iteration)")
            }
            TempVerdict::FrameTemp { per_iteration: false } => {
                write!(f, "frame temporary (per frame)")
            }
            TempVerdict::SiteOwned => write!(f, "owned by the site"),
            TempVerdict::Nobody => write!(f, "nobody"),
        }
    }
}

/// Where the value lowering is about to build came from.
///
/// Set by the arm of `lower_expr` / `lower_stmt` that dispatches to
/// `lower_locus_instantiation`, and consumed there like every other
/// one-shot, so a nested instantiation does not read its parent's
/// site.
#[derive(Clone, Copy, Debug)]
pub enum Site {
    /// A node this pass numbered.
    Expr(ExprId),
    /// A node codegen built itself, after the pass ran. Not a missing
    /// decision; the span is carried so a report can say WHICH node.
    Unindexed(Span),
    /// Lowering with no source expression at all: the `@export`
    /// singleton's prelude, a `reperspective` swap.
    Synthesized(&'static str),
}

/// What a row's expression IS. The three shapes that can put a
/// locus in a position: the literal that builds one, a call to a fn
/// proven to build one, and a NAME that reads one somebody else
/// holds.
///
/// Typed rather than spelled, because lowering asks the question:
/// the owner's `__locus_ref_owned_mask` bit used to be set by "a
/// LITERAL consumed the parent-owned flag", and GH #921 A3 commit 3
/// has to ask the table the same thing while the factory half is
/// still decided by the field-ownership predicates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Produced {
    Literal,
    FactoryCall,
    Handle,
}

impl std::fmt::Display for Produced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Produced::Literal => write!(f, "locus literal"),
            Produced::FactoryCall => write!(f, "factory call"),
            Produced::Handle => write!(f, "handle"),
        }
    }
}

/// One table row.
#[derive(Clone, Debug)]
pub struct Entry {
    pub owner: Owner,
    /// The innermost reclaim scope in effect at the expression.
    pub scope: ScopeId,
    /// What the expression is.
    pub what: Produced,
    /// The locus or callee the expression names.
    pub name: String,
    /// The syntactic position the decision came from.
    pub position: &'static str,
    /// The declaration the expression is written in.
    pub decl: String,
    /// For reporting only — never a key.
    pub span: Span,
}

// ===================================================================
// The table
// ===================================================================

/// The side table [`resolve_owners`] produces.
#[derive(Debug, Default)]
pub struct OwnerTable {
    entries: BTreeMap<ExprId, Entry>,
    /// `(owner locus, field)` -> the row for a field initialised from
    /// a NAME. Keyed separately because `Expr::Ident` carries no id.
    borrowed: BTreeMap<(String, String), Entry>,
    scope_kinds: Vec<ScopeKind>,
    /// The EXTENDED proven-fresh factory set: fn name -> locus.
    fresh: BTreeMap<String, String>,
    /// How many nodes the pass numbered.
    numbered: u32,
}

impl OwnerTable {
    /// The id this pass gave the node, or `None` when the node carries
    /// no id at all (a shape that cannot produce a locus) or was built
    /// after the pass ran.
    pub fn id_of(&self, e: &Expr) -> Option<ExprId> {
        node_id(e).filter(|n| !n.is_none()).map(|n| ExprId(n.0))
    }

    /// The row for this node, if the pass decided one.
    pub fn entry_of(&self, e: &Expr) -> Option<&Entry> {
        self.id_of(e).and_then(|id| self.entries.get(&id))
    }

    pub fn entry(&self, id: ExprId) -> Option<&Entry> {
        self.entries.get(&id)
    }

    /// The row for a param field initialised from a name.
    pub fn borrowed_entry(
        &self,
        locus: &str,
        field: &str,
    ) -> Option<&Entry> {
        self.borrowed.get(&(locus.to_string(), field.to_string()))
    }

    pub fn scope_kind(&self, s: ScopeId) -> ScopeKind {
        self.scope_kinds
            .get(s.0 as usize)
            .copied()
            .unwrap_or(ScopeKind::Frame)
    }

    /// The disposition a row implies at an instantiation.
    pub fn disposition(&self, entry: &Entry) -> Disposition {
        let per_iteration =
            self.scope_kind(entry.scope) == ScopeKind::LoopIteration;
        match &entry.owner {
            Owner::Binding(_) | Owner::FrameTemp(_) => {
                Disposition::Frame { per_iteration }
            }
            Owner::Field { .. } => Disposition::Owned,
            Owner::Caller => Disposition::Caller,
            Owner::Placement(_) => Disposition::Placement,
            Owner::Borrowed(_) => Disposition::Borrowed,
        }
    }

    /// What a row says about the frame-temporary question a factory
    /// call site asks. No row means the table says nothing has to
    /// reclaim this value.
    pub fn temp_verdict(&self, entry: Option<&Entry>) -> TempVerdict {
        match entry {
            None => TempVerdict::Nobody,
            Some(e) => match &e.owner {
                Owner::FrameTemp(s) => TempVerdict::FrameTemp {
                    per_iteration: self.scope_kind(*s)
                        == ScopeKind::LoopIteration,
                },
                Owner::Binding(_)
                | Owner::Field { .. }
                | Owner::Caller
                | Owner::Placement(_)
                | Owner::Borrowed(_) => TempVerdict::SiteOwned,
            },
        }
    }

    /// The locus a fn freshly returns under the EXTENDED rule (the
    /// carrier-return arms `compute_fresh_locus_factories` misses are
    /// in here and not in its map).
    pub fn extended_fresh_factory(&self, fn_name: &str) -> Option<&str> {
        self.fresh.get(fn_name).map(|s| s.as_str())
    }

    /// The whole extended set, `fn name -> locus`. GH #921 A3 folds
    /// it back into the map lowering reads, so the carrier-return
    /// arms are proven fresh on both sides of the decision.
    pub fn extended_fresh_factories(
        &self,
    ) -> impl Iterator<Item = (&String, &String)> {
        self.fresh.iter()
    }

    /// The row for the value an expression hands its site, looking
    /// THROUGH the delegating shapes that do not produce a locus of
    /// their own: an `or`'s ok value, a carrier's first arm tail, a
    /// composite's first element. Every leaf under a delegating node
    /// carries the same decision (the site's, demoted by
    /// [`Decision::through_delegate`]), so the first one answers for
    /// all of them.
    ///
    /// `Expr::Or` and the carriers carry no [`NodeId`] — only a
    /// literal and a call do — so a consumer handed a field
    /// initialiser has to reach the leaf to find the row at all.
    pub fn leaf_entry_of(&self, e: &Expr) -> Option<&Entry> {
        match e {
            Expr::Struct { .. } | Expr::Call { .. } => self.entry_of(e),
            Expr::Or { inner, .. } => self.leaf_entry_of(inner),
            Expr::If(_) | Expr::Match(_) | Expr::Block(_) => {
                let mut arms = Vec::new();
                return_arms(e, &mut arms);
                arms.first().and_then(|a| self.leaf_entry_of(a))
            }
            Expr::Array(parts, _) | Expr::Tuple(parts, _) => {
                parts.first().and_then(|p| self.leaf_entry_of(p))
            }
            _ => None,
        }
    }

    /// Does the row for this expression's value say an OWNING LOCUS
    /// reclaims it — a param field's mask bit, or a placement entry?
    /// The question the field-ownership predicates answer today.
    pub fn field_owns(&self, e: &Expr) -> bool {
        matches!(
            self.leaf_entry_of(e).map(|x| &x.owner),
            Some(Owner::Field { .. }) | Some(Owner::Placement(_))
        )
    }

    /// The same, restricted to a LITERAL — the half of the mask-bit
    /// decision `instantiating_for_parent_field` carried (GH #921 A3
    /// commit 3). The factory half is still the field-ownership
    /// predicates' until commit 4.
    pub fn field_owns_a_literal(&self, e: &Expr) -> bool {
        matches!(
            self.leaf_entry_of(e),
            Some(Entry {
                owner: Owner::Field { .. } | Owner::Placement(_),
                what: Produced::Literal,
                ..
            })
        )
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many expression nodes the pass numbered.
    pub fn numbered(&self) -> u32 {
        self.numbered
    }

    /// Every row, in id order — the unit tests' view.
    pub fn rows(&self) -> impl Iterator<Item = (&ExprId, &Entry)> {
        self.entries.iter()
    }

    /// Every borrowed-handle row, in `(locus, field)` order.
    pub fn borrowed_rows(
        &self,
    ) -> impl Iterator<Item = (&(String, String), &Entry)> {
        self.borrowed.iter()
    }

    /// How a row names itself in a disagreement line.
    pub fn describe(&self, id: ExprId) -> String {
        match self.entries.get(&id) {
            Some(e) => format!(
                "{} `{}` in {} ({}, bytes {}..{})",
                e.what,
                e.name,
                e.decl,
                e.position,
                e.span.start.0,
                e.span.end.0
            ),
            None => format!("expression #{}", id.0),
        }
    }
}

/// The id a node carries, for the two shapes that carry one.
fn node_id(e: &Expr) -> Option<NodeId> {
    match e {
        Expr::Struct { id, .. } | Expr::Call { id, .. } => Some(*id),
        _ => None,
    }
}

// ===================================================================
// The resolver
// ===================================================================

/// What a field of a locus can hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FieldKind {
    /// A locus, interface or perspective handle — the field can OWN a
    /// locus, so an initialiser that builds one transfers into it.
    Holder,
    /// Anything else.
    Value,
}

/// The decision a site makes, before it is recorded against a leaf.
#[derive(Clone, Debug)]
enum Decision {
    Binding(SlotId),
    Field { owner: ExprId, field: String },
    FrameTemp,
    Caller,
    Placement(String),
}

impl Decision {
    /// Passing through a delegating node — an `or`, a carrier, a
    /// composite — demotes a binding to a frame temporary of the
    /// innermost reclaim scope. See the module docs.
    fn through_delegate(&self) -> Decision {
        match self {
            Decision::Binding(_) => Decision::FrameTemp,
            other => other.clone(),
        }
    }
}

/// What `open_frame` puts aside while a frame is walked.
struct SavedFrame {
    decl: String,
    slots: BTreeMap<String, SlotId>,
    returned: BTreeSet<String>,
}

/// The names a body hands back with a bare `return x;` — the shape
/// `compute_returned_bindings` recognises, and the reason lowering
/// does not let the binding own the value.
fn collect_returned_names(b: &Block, out: &mut BTreeSet<String>) {
    for s in &b.stmts {
        collect_returned_names_stmt(s, out);
    }
    if let Some(Expr::Ident(i)) = b.tail.as_deref() {
        out.insert(i.name.clone());
    }
}

fn collect_returned_names_stmt(s: &Stmt, out: &mut BTreeSet<String>) {
    match s {
        Stmt::Return(Some(Expr::Ident(i)), _) => {
            out.insert(i.name.clone());
        }
        Stmt::If(i) => collect_returned_names_if(i, out),
        Stmt::Match(m) => {
            for a in &m.arms {
                if let MatchArmBody::Block(b) = &a.body {
                    collect_returned_names(b, out);
                }
            }
        }
        Stmt::While { body, .. }
        | Stmt::For { body, .. }
        | Stmt::ShmWrite { body, .. }
        | Stmt::Block(body) => collect_returned_names(body, out),
        _ => {}
    }
}

fn collect_returned_names_if(i: &IfStmt, out: &mut BTreeSet<String>) {
    collect_returned_names(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => collect_returned_names(b, out),
        Some(ElseBranch::ElseIf(n)) => collect_returned_names_if(n, out),
        None => {}
    }
}

struct Resolver {
    /// Cross-seed import renames, for resolving a path-qualified
    /// literal or callee to the merged program's mangled name.
    renames: Vec<(Vec<String>, String)>,
    /// Declared locus names (including the stdlib's mangled ones).
    loci: BTreeSet<String>,
    /// locus -> field -> kind.
    locus_fields: BTreeMap<String, BTreeMap<String, FieldKind>>,
    /// locus -> the fields its `placement { }` names.
    placed_fields: BTreeMap<String, BTreeSet<String>>,
    /// locus -> the child locus its `accept(c: C)` takes. A literal
    /// of `C` written inside one of its member bodies is retained on
    /// the acceptor's `__children[]` spine and reclaimed by its
    /// cascade, not by the frame that wrote it.
    accepts: BTreeMap<String, String>,
    /// The child type the locus whose member body is being walked
    /// accepts, if any.
    accepting: Option<String>,
    /// The EXTENDED proven-fresh factory set: fn name -> locus.
    fresh: BTreeMap<String, String>,
    table: OwnerTable,
    next_id: u32,
    next_slot: u32,
    /// The reclaim-scope stack; the last entry is the innermost.
    scopes: Vec<ScopeId>,
    /// Binding name -> slot, within the frame being walked.
    slots: BTreeMap<String, SlotId>,
    /// Names this frame hands back with a bare `return x;`. The
    /// binding does not own those — the caller does, which is the
    /// carve-out `binding_escapes_this_frame` makes in lowering.
    returned: BTreeSet<String>,
    /// The declaration being walked, for reporting.
    decl: String,
    /// The locus whose literal / params block is being walked, for the
    /// borrowed-handle index.
    owner_locus: String,
}

/// Build the owner table for `program`, numbering every
/// locus-producing expression node on the way.
///
/// `fresh_factories` is codegen's own `fresh_locus_factories` map (fn
/// name -> (locus, returned binding)); the table seeds its EXTENDED
/// set from it and adds the carrier-return fns that map misses.
///
/// The program is taken by `&mut` for the ids alone: nothing else
/// about it changes, and every other field the AST carries is left
/// exactly as parsed.
pub fn resolve_owners(
    program: &mut Program,
    fresh_factories: &BTreeMap<String, (String, Option<String>)>,
    import_renames: &[(Vec<String>, String)],
) -> OwnerTable {
    let mut loci = BTreeSet::new();
    let mut provisional: BTreeMap<String, BTreeMap<String, FieldKind>> =
        BTreeMap::new();
    let mut placed_fields: BTreeMap<String, BTreeSet<String>> =
        BTreeMap::new();
    collect_loci(
        &program.items,
        &mut loci,
        &mut provisional,
        &mut placed_fields,
    );
    // The field-kind pass needs the full locus set to tell a
    // locus-typed field from a record-typed one, so it runs second.
    let mut locus_fields: BTreeMap<String, BTreeMap<String, FieldKind>> =
        BTreeMap::new();
    refine_field_kinds(
        &program.items,
        &loci,
        &provisional,
        import_renames,
        &mut locus_fields,
    );
    let accepts = collect_accepts(&program.items, &loci, import_renames);

    let fresh = extend_fresh_factories(
        program,
        fresh_factories,
        &loci,
        import_renames,
    );

    let mut r = Resolver {
        renames: import_renames.to_vec(),
        loci,
        locus_fields,
        placed_fields,
        accepts,
        accepting: None,
        fresh: fresh.clone(),
        table: OwnerTable {
            entries: BTreeMap::new(),
            borrowed: BTreeMap::new(),
            scope_kinds: Vec::new(),
            fresh,
            numbered: 0,
        },
        next_id: 0,
        next_slot: 0,
        scopes: Vec::new(),
        slots: BTreeMap::new(),
        decl: String::new(),
        owner_locus: String::new(),
        returned: BTreeSet::new(),
    };
    r.walk_decls(&mut program.items, "");
    r.table.numbered = r.next_id;
    r.table
}

// -------------------------------------------------------------------
// Declaration-level collection
// -------------------------------------------------------------------

fn collect_loci(
    items: &[TopDecl],
    loci: &mut BTreeSet<String>,
    fields: &mut BTreeMap<String, BTreeMap<String, FieldKind>>,
    placed: &mut BTreeMap<String, BTreeSet<String>>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) => {
                loci.insert(l.name.name.clone());
                // Provisional: every param is a Value until
                // `refine_field_kinds` sees the whole locus set.
                let mut fs = BTreeMap::new();
                let mut placement = BTreeSet::new();
                for m in &l.members {
                    match m {
                        LocusMember::Params(p) => {
                            for pd in &p.params {
                                fs.insert(
                                    pd.name.name.clone(),
                                    FieldKind::Value,
                                );
                            }
                        }
                        LocusMember::Placement(pb) => {
                            for e in &pb.entries {
                                placement.insert(e.field.name.clone());
                            }
                        }
                        _ => {}
                    }
                }
                fields.insert(l.name.name.clone(), fs);
                placed.insert(l.name.name.clone(), placement);
            }
            TopDecl::Module(m) => collect_loci(&m.items, loci, fields, placed),
            _ => {}
        }
    }
}

fn refine_field_kinds(
    items: &[TopDecl],
    loci: &BTreeSet<String>,
    provisional: &BTreeMap<String, BTreeMap<String, FieldKind>>,
    renames: &[(Vec<String>, String)],
    out: &mut BTreeMap<String, BTreeMap<String, FieldKind>>,
) {
    let ifaces: BTreeSet<String> = interface_names(items);
    #[allow(clippy::too_many_arguments)]
    fn go(
        items: &[TopDecl],
        loci: &BTreeSet<String>,
        ifaces: &BTreeSet<String>,
        provisional: &BTreeMap<String, BTreeMap<String, FieldKind>>,
        renames: &[(Vec<String>, String)],
        out: &mut BTreeMap<String, BTreeMap<String, FieldKind>>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let mut fs = provisional
                        .get(&l.name.name)
                        .cloned()
                        .unwrap_or_default();
                    for m in &l.members {
                        if let LocusMember::Params(p) = m {
                            for pd in &p.params {
                                let kind = param_field_kind(
                                    pd.ty.as_ref(),
                                    &pd.init,
                                    loci,
                                    ifaces,
                                    renames,
                                );
                                fs.insert(pd.name.name.clone(), kind);
                            }
                        }
                    }
                    out.insert(l.name.name.clone(), fs);
                }
                TopDecl::Module(m) => {
                    go(&m.items, loci, ifaces, provisional, renames, out)
                }
                _ => {}
            }
        }
    }
    go(items, loci, &ifaces, provisional, renames, out);
}

/// locus -> the child locus type its `accept(c: C)` declares.
fn collect_accepts(
    items: &[TopDecl],
    loci: &BTreeSet<String>,
    renames: &[(Vec<String>, String)],
) -> BTreeMap<String, String> {
    use hale_syntax::ast::LifecycleKind;
    let mut out = BTreeMap::new();
    fn go(
        items: &[TopDecl],
        loci: &BTreeSet<String>,
        renames: &[(Vec<String>, String)],
        out: &mut BTreeMap<String, String>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        let LocusMember::Lifecycle(lc) = m else { continue };
                        if lc.kind != LifecycleKind::Accept {
                            continue;
                        }
                        let Some(p) = lc.params.first() else { continue };
                        let TypeExpr::Named { path, .. } = &p.ty else {
                            continue;
                        };
                        let segs = qname_segs(path);
                        let name = resolve_path(&segs, renames)
                            .filter(|n| loci.contains(n))
                            .or_else(|| {
                                path.segments
                                    .last()
                                    .map(|s| s.name.clone())
                                    .filter(|n| loci.contains(n))
                            });
                        if let Some(name) = name {
                            out.insert(l.name.name.clone(), name);
                        }
                    }
                }
                TopDecl::Module(m) => go(&m.items, loci, renames, out),
                _ => {}
            }
        }
    }
    go(items, loci, renames, &mut out);
    out
}

fn interface_names(items: &[TopDecl]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn go(items: &[TopDecl], out: &mut BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Interface(i) => {
                    out.insert(i.name.name.clone());
                }
                TopDecl::Perspective(p) => {
                    out.insert(p.name.name.clone());
                }
                TopDecl::Module(m) => go(&m.items, out),
                _ => {}
            }
        }
    }
    go(items, &mut out);
    out
}

fn param_field_kind(
    ty: Option<&TypeExpr>,
    init: &ParamInit,
    loci: &BTreeSet<String>,
    ifaces: &BTreeSet<String>,
    renames: &[(Vec<String>, String)],
) -> FieldKind {
    let names = |path: &QualifiedName| -> Vec<String> {
        let mut v = Vec::new();
        let segs = qname_segs(path);
        if let Some(n) = resolve_path(&segs, renames) {
            v.push(n);
        }
        if let Some(last) = path.segments.last() {
            v.push(last.name.clone());
        }
        v.push(qname_joined(path));
        v
    };
    if let Some(t) = ty {
        return match t {
            TypeExpr::Perspective { .. } => FieldKind::Holder,
            TypeExpr::Named { path, .. } => {
                // `std::http::RouteHandler` is declared as
                // `__StdHttpRouteHandler`, so the bare last segment
                // finds neither the locus nor the interface — the
                // stdlib's own field initialisers looked like plain
                // values until this resolved the path.
                if names(path)
                    .iter()
                    .any(|n| loci.contains(n) || ifaces.contains(n))
                {
                    FieldKind::Holder
                } else {
                    FieldKind::Value
                }
            }
            _ => FieldKind::Value,
        };
    }
    // An inferred param: the initialiser's shape decides.
    match init {
        ParamInit::Value(Expr::Struct { path, .. }) => {
            if names(path).iter().any(|n| loci.contains(n)) {
                FieldKind::Holder
            } else {
                FieldKind::Value
            }
        }
        _ => FieldKind::Value,
    }
}

/// A multi-segment path resolved to the single mangled name the
/// merged program declares, exactly as `compute_fresh_locus_factories`
/// and `Cx::mangled_for_path` resolve it: bundled `std::…` paths
/// through `hale_stdlib::PATH_RENAMES`, cross-seed imports through the
/// caller's rename list. Without this, every path-qualified stdlib
/// literal (`std::io::tcp::Stream { … }`) looked like a record
/// literal to the pre-pass and was never numbered.
fn resolve_path(
    segs: &[String],
    renames: &[(Vec<String>, String)],
) -> Option<String> {
    if segs.len() == 1 {
        return Some(segs[0].clone());
    }
    let refs: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
    if refs.first() == Some(&"std") {
        if let Some((_, m)) = hale_stdlib::PATH_RENAMES
            .iter()
            .find(|(p, _)| *p == refs.as_slice())
        {
            return Some((*m).to_string());
        }
    }
    renames
        .iter()
        .find(|(p, _)| p.len() == segs.len() && p.iter().eq(segs.iter()))
        .map(|(_, m)| m.clone())
}

fn qname_segs(q: &QualifiedName) -> Vec<String> {
    q.segments.iter().map(|s| s.name.clone()).collect()
}

fn qname_joined(q: &QualifiedName) -> String {
    q.segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join("::")
}

// -------------------------------------------------------------------
// The extended fresh-factory fixpoint
// -------------------------------------------------------------------

/// Flatten a returned expression into the values that can actually
/// reach the caller: a carrier contributes its arm TAILS, an `or`
/// contributes its ok value and its substitute, and a diverging
/// disposition contributes nothing.
fn return_arms<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
    match e {
        Expr::If(i) => if_arms(i, out),
        Expr::Match(m) => {
            for a in &m.arms {
                match &a.body {
                    MatchArmBody::Expr(x) => return_arms(x, out),
                    MatchArmBody::Block(b) => block_arms(b, out),
                }
            }
        }
        Expr::Block(b) => block_arms(b, out),
        Expr::Or { inner, disposition, .. } => {
            return_arms(inner, out);
            match disposition {
                OrDisposition::Substitute(rhs) => return_arms(rhs, out),
                OrDisposition::Raise(_)
                | OrDisposition::Fail(..)
                | OrDisposition::Discard(_)
                | OrDisposition::Wait(_) => {}
            }
        }
        other => out.push(other),
    }
}

fn if_arms<'e>(i: &'e IfStmt, out: &mut Vec<&'e Expr>) {
    block_arms(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => block_arms(b, out),
        Some(ElseBranch::ElseIf(nested)) => if_arms(nested, out),
        None => {}
    }
}

fn block_arms<'e>(b: &'e Block, out: &mut Vec<&'e Expr>) {
    if let Some(t) = &b.tail {
        return_arms(t, out);
    }
}

/// Seed from codegen's map and add the fns whose every return arm is
/// fresh once carriers are flattened.
fn extend_fresh_factories(
    program: &Program,
    base: &BTreeMap<String, (String, Option<String>)>,
    loci: &BTreeSet<String>,
    renames: &[(Vec<String>, String)],
) -> BTreeMap<String, String> {
    // `compute_fresh_locus_factories` does not check that what a fn
    // freshly returns is a LOCUS — `fn parse_url(..) -> std::http::Url`
    // is in its map though `Url` is a record — and the hooks that read
    // it filter on the lowered `CodegenTy::LocusRef` instead. The
    // table has no lowered type, so it filters here; without this
    // every record factory looked like a locus the flags forgot to
    // give an owner.
    let mut out: BTreeMap<String, String> = base
        .iter()
        .filter(|(_, (l, _))| loci.contains(l))
        .map(|(k, (l, _))| (k.clone(), l.clone()))
        .collect();
    let mut fns: Vec<&FnDecl> = Vec::new();
    collect_fns(&program.items, &mut fns);
    loop {
        let mut added = false;
        for f in &fns {
            if out.contains_key(&f.name.name) {
                continue;
            }
            let Some(l) = declared_ret_locus(f, loci, renames) else {
                continue;
            };
            let mut rets: Vec<&Expr> = Vec::new();
            collect_returns(&f.body, &mut rets);
            if rets.is_empty() {
                continue;
            }
            let mut arms: Vec<&Expr> = Vec::new();
            for r in &rets {
                return_arms(r, &mut arms);
            }
            if arms.is_empty() {
                continue;
            }
            // The binding shape: `let t = <carrier>; return t;` is the
            // same program as `return <carrier>;` and has to be the
            // same answer (spec/semantics.md "Dissolve timing rules"
            // — the `let`-named and inline spellings are one
            // program). The name is resolved through the fn's own
            // `let`s, once, and only when it is bound exactly once and
            // never re-assigned; anything else leaves the fn out of
            // the set, which is the old leak and never a double free.
            let lets = collect_lets(&f.body);
            let assigned = assigned_names(&f.body);
            let all_fresh = arms.iter().all(|a| {
                arm_is_fresh(a, &l, &out, renames, &lets, &assigned, 0)
            });
            if all_fresh {
                out.insert(f.name.name.clone(), l);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    out
}

/// Is this return arm a value the fn freshly built?
///
/// A literal of the declared locus, a call to a fn already proven to
/// build one, or a NAME bound once to either — including through a
/// carrier, which is the `let t = if c { make(1) } else { make2(1) };
/// return t;` spelling of the same program.
#[allow(clippy::too_many_arguments)]
fn arm_is_fresh(
    a: &Expr,
    l: &str,
    known: &BTreeMap<String, String>,
    renames: &[(Vec<String>, String)],
    lets: &[(String, &Expr)],
    assigned: &BTreeSet<String>,
    depth: u32,
) -> bool {
    if depth > 4 {
        return false;
    }
    match a {
        Expr::Struct { path, .. } => {
            let segs = qname_segs(path);
            resolve_path(&segs, renames).as_deref() == Some(l)
                || path
                    .segments
                    .last()
                    .map(|s| s.name == l)
                    .unwrap_or(false)
        }
        Expr::Call { callee, .. } => callee_fn_name(callee, renames)
            .and_then(|n| known.get(&n).cloned())
            .map(|cl| cl == l)
            .unwrap_or(false),
        Expr::Ident(i) => {
            if assigned.contains(&i.name) {
                return false;
            }
            let bound: Vec<&(String, &Expr)> =
                lets.iter().filter(|(n, _)| *n == i.name).collect();
            if bound.len() != 1 {
                return false;
            }
            let mut inner = Vec::new();
            return_arms(bound[0].1, &mut inner);
            !inner.is_empty()
                && inner.iter().all(|x| {
                    arm_is_fresh(
                        x,
                        l,
                        known,
                        renames,
                        lets,
                        assigned,
                        depth + 1,
                    )
                })
        }
        _ => false,
    }
}

/// Every `let` in a body, as `(name, RHS)`. A name bound twice is
/// listed twice, which [`arm_is_fresh`] treats as unresolvable.
fn collect_lets(b: &Block) -> Vec<(String, &Expr)> {
    let mut out = Vec::new();
    fn go<'e>(b: &'e Block, out: &mut Vec<(String, &'e Expr)>) {
        for s in &b.stmts {
            match s {
                Stmt::Let { name, value, .. } => {
                    out.push((name.name.clone(), value))
                }
                Stmt::If(i) => go_if(i, out),
                Stmt::Match(m) => {
                    for a in &m.arms {
                        if let MatchArmBody::Block(bb) = &a.body {
                            go(bb, out);
                        }
                    }
                }
                Stmt::While { body, .. }
                | Stmt::For { body, .. }
                | Stmt::ShmWrite { body, .. }
                | Stmt::Block(body) => go(body, out),
                _ => {}
            }
        }
    }
    fn go_if<'e>(i: &'e IfStmt, out: &mut Vec<(String, &'e Expr)>) {
        go(&i.then_block, out);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => go(b, out),
            Some(ElseBranch::ElseIf(n)) => go_if(n, out),
            None => {}
        }
    }
    go(b, &mut out);
    out
}

/// Names a body re-assigns with a bare `=`. A binding that moves
/// between names can be reached through two of them, so it is never
/// the fn's own fresh value.
fn assigned_names(b: &Block) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn go(b: &Block, out: &mut BTreeSet<String>) {
        for s in &b.stmts {
            match s {
                Stmt::Assign { target, .. } => {
                    out.insert(target.head.name.clone());
                }
                Stmt::If(i) => go_if(i, out),
                Stmt::Match(m) => {
                    for a in &m.arms {
                        if let MatchArmBody::Block(bb) = &a.body {
                            go(bb, out);
                        }
                    }
                }
                Stmt::While { body, .. }
                | Stmt::For { body, .. }
                | Stmt::ShmWrite { body, .. }
                | Stmt::Block(body) => go(body, out),
                _ => {}
            }
        }
    }
    fn go_if(i: &IfStmt, out: &mut BTreeSet<String>) {
        go(&i.then_block, out);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => go(b, out),
            Some(ElseBranch::ElseIf(n)) => go_if(n, out),
            None => {}
        }
    }
    go(b, &mut out);
    out
}

fn collect_fns<'a>(items: &'a [TopDecl], out: &mut Vec<&'a FnDecl>) {
    for item in items {
        match item {
            TopDecl::Fn(f) => out.push(f),
            TopDecl::Module(m) => collect_fns(&m.items, out),
            _ => {}
        }
    }
}

fn declared_ret_locus(
    f: &FnDecl,
    loci: &BTreeSet<String>,
    renames: &[(Vec<String>, String)],
) -> Option<String> {
    match f.ret.as_ref()? {
        TypeExpr::Named { path, .. } => {
            let segs = qname_segs(path);
            if let Some(n) = resolve_path(&segs, renames) {
                if loci.contains(&n) {
                    return Some(n);
                }
            }
            let last = path.segments.last()?.name.clone();
            if loci.contains(&last) {
                Some(last)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn collect_returns<'e>(b: &'e Block, out: &mut Vec<&'e Expr>) {
    for s in &b.stmts {
        collect_returns_stmt(s, out);
    }
    if let Some(t) = &b.tail {
        out.push(t);
    }
}

fn collect_returns_stmt<'e>(s: &'e Stmt, out: &mut Vec<&'e Expr>) {
    match s {
        Stmt::Return(Some(e), _) => out.push(e),
        Stmt::If(i) => collect_returns_if(i, out),
        Stmt::Match(m) => {
            for a in &m.arms {
                match &a.body {
                    MatchArmBody::Expr(_) => {}
                    MatchArmBody::Block(b) => collect_returns(b, out),
                }
            }
        }
        Stmt::While { body, .. }
        | Stmt::For { body, .. }
        | Stmt::ShmWrite { body, .. } => collect_returns(body, out),
        Stmt::Block(b) => collect_returns(b, out),
        _ => {}
    }
}

fn collect_returns_if<'e>(i: &'e IfStmt, out: &mut Vec<&'e Expr>) {
    collect_returns(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => collect_returns(b, out),
        Some(ElseBranch::ElseIf(n)) => collect_returns_if(n, out),
        None => {}
    }
}

/// The dotted / `::`-joined name of a callee, in the spellings the
/// parser produces. Mirrors `Cx::callee_fn_name` closely enough for
/// the table; a name the two disagree on shows up as a disagreement
/// rather than as silence.
pub(crate) fn callee_fn_name(
    callee: &Expr,
    renames: &[(Vec<String>, String)],
) -> Option<String> {
    fn segs(e: &Expr, out: &mut Vec<String>) -> bool {
        match e {
            Expr::Ident(i) => {
                out.push(i.name.clone());
                true
            }
            Expr::Path(q) => {
                out.extend(q.segments.iter().map(|i| i.name.clone()));
                true
            }
            Expr::Path2 { receiver, name, .. }
            | Expr::Field { receiver, name, .. } => {
                if !segs(receiver, out) {
                    return false;
                }
                out.push(name.name.clone());
                true
            }
            _ => false,
        }
    }
    let mut v = Vec::new();
    if !segs(callee, &mut v) {
        return None;
    }
    resolve_path(&v, renames)
}

// -------------------------------------------------------------------
// The walk
// -------------------------------------------------------------------

impl Resolver {
    fn scope(&self) -> ScopeId {
        *self.scopes.last().expect("a scope is open")
    }

    fn push_scope(&mut self, kind: ScopeKind) {
        let id = ScopeId(self.table.scope_kinds.len() as u32);
        self.table.scope_kinds.push(kind);
        self.scopes.push(id);
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn slot_for(&mut self, name: &str) -> SlotId {
        if let Some(s) = self.slots.get(name) {
            return *s;
        }
        let s = SlotId(self.next_slot);
        self.next_slot += 1;
        self.slots.insert(name.to_string(), s);
        s
    }

    /// Give this node an id, once. Only the two shapes that can
    /// produce a locus carry one.
    fn number(&mut self, e: &mut Expr) -> Option<ExprId> {
        let slot = match e {
            Expr::Struct { id, .. } | Expr::Call { id, .. } => id,
            _ => return None,
        };
        if !slot.is_none() {
            return Some(ExprId(slot.0));
        }
        let n = self.next_id;
        self.next_id += 1;
        *slot = NodeId(n);
        Some(ExprId(n))
    }

    fn record(
        &mut self,
        id: ExprId,
        owner: Owner,
        what: Produced,
        name: String,
        position: &'static str,
        span: Span,
    ) {
        let scope = self.scope();
        let decl = self.decl.clone();
        self.table.entries.insert(
            id,
            Entry { owner, scope, what, name, position, decl, span },
        );
    }

    fn locus_of_literal(&self, path: &QualifiedName) -> Option<String> {
        let segs = qname_segs(path);
        if let Some(n) = resolve_path(&segs, &self.renames) {
            if self.loci.contains(&n) {
                return Some(n);
            }
        }
        // A path whose rename is not in hand still names its locus by
        // its last segment in a single-seed program.
        let last = path.segments.last()?.name.clone();
        if self.loci.contains(&last) {
            return Some(last);
        }
        None
    }

    fn fresh_call_locus(&self, e: &Expr) -> Option<String> {
        let Expr::Call { callee, .. } = e else { return None };
        let n = callee_fn_name(callee, &self.renames)?;
        self.fresh.get(&n).cloned()
    }

    /// Is this expression one the table hands an owner to?
    fn is_locus_producing(&self, e: &Expr) -> bool {
        match e {
            Expr::Struct { path, .. } => {
                self.locus_of_literal(path).is_some()
            }
            Expr::Call { .. } => self.fresh_call_locus(e).is_some(),
            Expr::Or { inner, disposition, .. } => {
                if !self.is_locus_producing(inner) {
                    return false;
                }
                match disposition {
                    OrDisposition::Substitute(rhs) => {
                        self.is_locus_producing(rhs)
                    }
                    _ => true,
                }
            }
            Expr::If(_) | Expr::Match(_) | Expr::Block(_) => {
                let mut arms = Vec::new();
                return_arms(e, &mut arms);
                !arms.is_empty()
                    && arms.iter().all(|a| self.is_locus_producing(a))
            }
            Expr::Array(parts, _) | Expr::Tuple(parts, _) => {
                !parts.is_empty()
                    && parts.iter().all(|p| self.is_locus_producing(p))
            }
            _ => false,
        }
    }

    /// A NAME the field can hold without building anything — the
    /// `Borrowed` shape (`Mid { leaf: shared }`).
    fn is_handle_ref(e: &Expr) -> bool {
        matches!(
            e,
            Expr::Ident(_)
                | Expr::Path(_)
                | Expr::KwSelf(_)
                | Expr::Field { .. }
                | Expr::Index { .. }
        )
    }

    fn owner_from(&self, d: &Decision) -> Owner {
        match d {
            Decision::Binding(s) => Owner::Binding(*s),
            Decision::Field { owner, field } => Owner::Field {
                owner: *owner,
                field: field.clone(),
            },
            Decision::FrameTemp => Owner::FrameTemp(self.scope()),
            Decision::Caller => Owner::Caller,
            Decision::Placement(e) => Owner::Placement(e.clone()),
        }
    }

    // ---------------------------------------------------------------
    // assign: distribute a site's decision down to the leaves
    // ---------------------------------------------------------------

    fn assign(&mut self, e: &mut Expr, d: Decision, position: &'static str) {
        match e {
            Expr::Struct { path, inits, span, id: _ } => {
                let lname = self.locus_of_literal(path);
                let span = *span;
                match lname {
                    Some(lname) => {
                        let id = self.number(e).expect("a literal is numbered");
                        // An acceptor's own body: the child is
                        // appended to `__children[]` and reclaimed by
                        // the acceptor's cascade, whatever the
                        // position says (GH #253's spine).
                        let owner = if self.accepting.as_deref()
                            == Some(lname.as_str())
                        {
                            Owner::Field {
                                owner: ExprId::DECLARED,
                                field: "__children".to_string(),
                            }
                        } else {
                            self.owner_from(&d)
                        };
                        self.record(
                            id,
                            owner,
                            Produced::Literal,
                            lname.clone(),
                            position,
                            span,
                        );
                        let Expr::Struct { inits, .. } = e else {
                            unreachable!("matched a struct above")
                        };
                        let mut taken = std::mem::take(inits);
                        self.walk_literal_inits(id, &lname, &mut taken);
                        let Expr::Struct { inits, .. } = e else {
                            unreachable!("matched a struct above")
                        };
                        *inits = taken;
                    }
                    None => {
                        // A record / type literal: not a locus, but
                        // its initialisers are ordinary expressions.
                        for i in inits.iter_mut() {
                            self.assign(
                                &mut i.value,
                                Decision::FrameTemp,
                                "record field",
                            );
                        }
                    }
                }
            }
            Expr::Call { .. } => {
                let lname = self.fresh_call_locus(e);
                let span = e.span();
                if let Some(lname) = lname {
                    let id = self.number(e).expect("a call is numbered");
                    let owner = self.owner_from(&d);
                    self.record(
                        id,
                        owner,
                        Produced::FactoryCall,
                        lname,
                        position,
                        span,
                    );
                }
                // The callee's receiver chain and every argument are
                // expression-position values of this frame — GH #837
                // is exactly the rule that a call's decision is about
                // the call, never about what is written inside it.
                let Expr::Call { callee, args, .. } = e else {
                    unreachable!("matched a call above")
                };
                let mut taken_callee = std::mem::replace(
                    callee,
                    Box::new(Expr::KwSelf(Span::new(0, 0))),
                );
                let mut taken_args = std::mem::take(args);
                self.walk_callee(&mut taken_callee);
                for a in taken_args.iter_mut() {
                    self.assign(a, Decision::FrameTemp, "argument");
                }
                let Expr::Call { callee, args, .. } = e else {
                    unreachable!("matched a call above")
                };
                *callee = taken_callee;
                *args = taken_args;
            }
            Expr::Or { inner, disposition, span: _ } => {
                let through = d.through_delegate();
                self.assign(inner, through.clone(), position);
                match disposition {
                    OrDisposition::Substitute(rhs) => {
                        self.assign(rhs, through, "`or` substitute")
                    }
                    OrDisposition::Fail(payload, _) => self.assign(
                        payload,
                        Decision::FrameTemp,
                        "`or fail` payload",
                    ),
                    OrDisposition::Raise(_)
                    | OrDisposition::Discard(_)
                    | OrDisposition::Wait(_) => {}
                }
            }
            Expr::If(i) => self.assign_if(i, d.through_delegate(), position),
            Expr::Match(m) => {
                let through = d.through_delegate();
                self.assign(
                    &mut m.scrutinee,
                    Decision::FrameTemp,
                    "scrutinee",
                );
                for a in m.arms.iter_mut() {
                    if let Some(g) = &mut a.guard {
                        self.assign(g, Decision::FrameTemp, "arm guard");
                    }
                    match &mut a.body {
                        MatchArmBody::Expr(x) => {
                            self.assign(x, through.clone(), "`match` arm")
                        }
                        MatchArmBody::Block(b) => self.walk_block(
                            b,
                            Some((through.clone(), "`match` arm tail")),
                        ),
                    }
                }
            }
            Expr::Block(b) => {
                let through = d.through_delegate();
                self.walk_block(b, Some((through, "block tail")))
            }
            Expr::Array(parts, _) | Expr::Tuple(parts, _) => {
                let through = d.through_delegate();
                for p in parts.iter_mut() {
                    self.assign(p, through.clone(), "composite element");
                }
            }
            // Leaves and everything that is not locus-producing: the
            // decision stops here and whatever is written inside is an
            // ordinary expression-position temporary.
            Expr::Literal(_, _)
            | Expr::Ident(_)
            | Expr::Path(_)
            | Expr::KwSelf(_) => {}
            Expr::Binary { left, right, op: _, span: _ } => {
                self.assign(left, Decision::FrameTemp, "operand");
                self.assign(right, Decision::FrameTemp, "operand");
            }
            Expr::Unary { operand, .. } => {
                self.assign(operand, Decision::FrameTemp, "operand")
            }
            Expr::Field { receiver, .. } => {
                self.assign(receiver, Decision::FrameTemp, "field read")
            }
            Expr::Index { receiver, index, span: _ } => {
                self.assign(receiver, Decision::FrameTemp, "index receiver");
                self.assign(index, Decision::FrameTemp, "index");
            }
            Expr::Path2 { receiver, .. } => {
                self.assign(receiver, Decision::FrameTemp, "path receiver")
            }
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => {
                self.assign(inner, Decision::FrameTemp, "reduction")
            }
            Expr::Approx { left, right, tolerance, span: _ } => {
                self.assign(left, Decision::FrameTemp, "approx");
                self.assign(right, Decision::FrameTemp, "approx");
                self.assign(tolerance, Decision::FrameTemp, "approx");
            }
            Expr::Range { lo, hi, .. } => {
                self.assign(lo, Decision::FrameTemp, "range");
                self.assign(hi, Decision::FrameTemp, "range");
            }
            Expr::ArrayRepeat { val, .. } => {
                self.assign(val, Decision::FrameTemp, "array repeat")
            }
        }
    }

    fn assign_if(
        &mut self,
        i: &mut IfStmt,
        d: Decision,
        position: &'static str,
    ) {
        self.assign(&mut i.cond, Decision::FrameTemp, "condition");
        self.walk_block(&mut i.then_block, Some((d.clone(), position)));
        match i.else_block.as_deref_mut() {
            Some(ElseBranch::Else(b)) => {
                self.walk_block(b, Some((d, position)))
            }
            Some(ElseBranch::ElseIf(n)) => self.assign_if(n, d, position),
            None => {}
        }
    }

    /// A receiver written in front of a call — `Cfg { }.seed()` — is a
    /// value of this frame and nothing else. GH #896 is exactly the
    /// case where the flags hand it the field's decision.
    fn walk_callee(&mut self, callee: &mut Expr) {
        match callee {
            Expr::Ident(_) | Expr::Path(_) => {}
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
                self.assign(receiver, Decision::FrameTemp, "call receiver");
            }
            other => self.assign(other, Decision::FrameTemp, "callee"),
        }
    }

    /// The initialisers of a locus literal: a holder field's value
    /// transfers into the field, a holder field's NAME is borrowed,
    /// and everything else is an ordinary expression.
    fn walk_literal_inits(
        &mut self,
        lit_id: ExprId,
        locus: &str,
        inits: &mut [StructInit],
    ) {
        let saved = std::mem::replace(&mut self.owner_locus, locus.to_string());
        for init in inits.iter_mut() {
            let kind = self
                .locus_fields
                .get(locus)
                .and_then(|f| f.get(&init.name.name))
                .copied()
                .unwrap_or(FieldKind::Value);
            let placed = self
                .placed_fields
                .get(locus)
                .map(|s| s.contains(&init.name.name))
                .unwrap_or(false);
            let field = init.name.name.clone();
            self.walk_field_init(
                lit_id,
                &field,
                &mut init.value,
                kind,
                placed,
            );
        }
        self.owner_locus = saved;
    }

    fn walk_field_init(
        &mut self,
        owner: ExprId,
        field: &str,
        value: &mut Expr,
        kind: FieldKind,
        placed: bool,
    ) {
        if kind == FieldKind::Value {
            self.assign(value, Decision::FrameTemp, "field initialiser");
            return;
        }
        if self.is_locus_producing(value) {
            let d = if placed {
                Decision::Placement(field.to_string())
            } else {
                Decision::Field { owner, field: field.to_string() }
            };
            self.assign(value, d, "param-field initialiser");
        } else if Self::is_handle_ref(value) {
            // GH #730 / #731: a handle passed IN is never owned by the
            // receiver. `Expr::Ident` carries no id, so the row is
            // keyed by the field it initialises.
            let scope = self.scope();
            let decl = self.decl.clone();
            let entry = Entry {
                owner: Owner::Borrowed(ExprId::DECLARED),
                scope,
                what: Produced::Handle,
                name: field.to_string(),
                position: "param-field initialiser (a name)",
                decl,
                span: value.span(),
            };
            self.table
                .borrowed
                .insert((self.owner_locus.clone(), field.to_string()), entry);
            self.assign(value, Decision::FrameTemp, "handle");
        } else {
            self.assign(value, Decision::FrameTemp, "field initialiser");
        }
    }

    // ---------------------------------------------------------------
    // Statements
    // ---------------------------------------------------------------

    /// `tail` is the decision the block's trailing expression takes,
    /// when the block is in value position.
    fn walk_block(
        &mut self,
        b: &mut Block,
        tail: Option<(Decision, &'static str)>,
    ) {
        for s in b.stmts.iter_mut() {
            self.walk_stmt(s);
        }
        if let Some(t) = &mut b.tail {
            match tail {
                Some((d, pos)) => self.assign(t, d, pos),
                None => self.assign(t, Decision::FrameTemp, "block tail"),
            }
        }
    }

    fn walk_stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                let d = if self.returned.contains(&name.name) {
                    Decision::Caller
                } else {
                    Decision::Binding(self.slot_for(&name.name))
                };
                self.assign(value, d, "`let` RHS");
            }
            Stmt::LetTuple { names, value, .. } => {
                let slot = match names.first() {
                    Some(n) => self.slot_for(&n.name.clone()),
                    None => self.slot_for("__lettuple"),
                };
                self.assign(
                    value,
                    Decision::Binding(slot),
                    "`let` tuple RHS",
                );
            }
            Stmt::Assign { target, value, .. } => {
                let bare = target.tail.is_empty();
                let head = target.head.name.clone();
                for seg in target.tail.iter_mut() {
                    if let LValueSeg::Index(ix) = seg {
                        self.assign(
                            ix,
                            Decision::FrameTemp,
                            "assignment index",
                        );
                    }
                }
                if bare {
                    // The same carve-out `Stmt::Let` makes: a name
                    // this frame hands back with a bare `return x;`
                    // is the CALLER's, whichever statement last wrote
                    // it. Missed here, `fn f() -> Buf { let mut a =
                    // make(); a = Buf { }; return a; }` gave the
                    // literal to the frame's flush and the caller a
                    // reclaimed locus — found by
                    // `freefn_locus_rebind.rs` when GH #921 A3
                    // commit 6 made lowering read this decision.
                    let d = if self.returned.contains(&head) {
                        Decision::Caller
                    } else {
                        Decision::Binding(self.slot_for(&head))
                    };
                    self.assign(value, d, "assignment RHS");
                } else {
                    self.assign(
                        value,
                        Decision::FrameTemp,
                        "field-assignment RHS",
                    );
                }
            }
            Stmt::If(i) => self.walk_if_stmt(i),
            Stmt::Match(m) => self.walk_match_stmt(m),
            Stmt::For { iter, body, .. } => {
                self.assign(iter, Decision::FrameTemp, "iterator");
                self.push_scope(ScopeKind::LoopIteration);
                self.walk_block(body, None);
                self.pop_scope();
            }
            Stmt::While { cond, body, .. } => {
                self.assign(cond, Decision::FrameTemp, "condition");
                self.push_scope(ScopeKind::LoopIteration);
                self.walk_block(body, None);
                self.pop_scope();
            }
            Stmt::Return(Some(e), _) => {
                self.assign(e, Decision::Caller, "`return`")
            }
            Stmt::Return(None, _) => {}
            Stmt::Fail { value, .. } => {
                self.assign(value, Decision::FrameTemp, "`fail` payload")
            }
            Stmt::Recovery { args, modifier, .. } => {
                for a in args.iter_mut() {
                    self.assign(a, Decision::FrameTemp, "recovery argument");
                }
                match modifier {
                    Some(hale_syntax::ast::RecoveryModifier::For(e))
                    | Some(hale_syntax::ast::RecoveryModifier::Until(e)) => {
                        self.assign(
                            e,
                            Decision::FrameTemp,
                            "recovery modifier",
                        )
                    }
                    None => {}
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.assign(
                        p,
                        Decision::FrameTemp,
                        "violation payload",
                    );
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.assign(subject, Decision::FrameTemp, "send subject");
                self.assign(value, Decision::FrameTemp, "send payload");
                match or_disposition {
                    Some(OrDisposition::Substitute(rhs)) => self.assign(
                        rhs,
                        Decision::FrameTemp,
                        "send `or` substitute",
                    ),
                    Some(OrDisposition::Fail(p, _)) => self.assign(
                        p,
                        Decision::FrameTemp,
                        "send `or fail` payload",
                    ),
                    _ => {}
                }
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.assign(max, Decision::FrameTemp, "shm max");
                self.walk_block(body, None);
            }
            Stmt::Block(b) => self.walk_block(b, None),
            Stmt::Expr(e) => {
                // A bare locus literal statement is a fire-and-forget
                // temporary of this frame; so is everything else in
                // statement position.
                self.assign(e, Decision::FrameTemp, "bare statement")
            }
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }

    fn walk_if_stmt(&mut self, i: &mut IfStmt) {
        self.assign(&mut i.cond, Decision::FrameTemp, "condition");
        self.walk_block(&mut i.then_block, None);
        match i.else_block.as_deref_mut() {
            Some(ElseBranch::Else(b)) => self.walk_block(b, None),
            Some(ElseBranch::ElseIf(n)) => self.walk_if_stmt(n),
            None => {}
        }
    }

    fn walk_match_stmt(&mut self, m: &mut MatchStmt) {
        self.assign(&mut m.scrutinee, Decision::FrameTemp, "scrutinee");
        for a in m.arms.iter_mut() {
            if let Some(g) = &mut a.guard {
                self.assign(g, Decision::FrameTemp, "arm guard");
            }
            match &mut a.body {
                MatchArmBody::Expr(e) => {
                    self.assign(e, Decision::FrameTemp, "`match` arm")
                }
                MatchArmBody::Block(b) => self.walk_block(b, None),
            }
        }
    }

    // ---------------------------------------------------------------
    // Declarations
    // ---------------------------------------------------------------

    /// Open a frame scope with a fresh binding-slot namespace.
    fn open_frame(&mut self, decl: &str) -> SavedFrame {
        let decl = std::mem::replace(&mut self.decl, decl.to_string());
        let slots = std::mem::take(&mut self.slots);
        let returned = std::mem::take(&mut self.returned);
        self.push_scope(ScopeKind::Frame);
        SavedFrame { decl, slots, returned }
    }

    /// The same, for a body: the names it hands back with a bare
    /// `return x;` are collected first, so a `let` can see them.
    fn open_body_frame(&mut self, decl: &str, body: &Block) -> SavedFrame {
        let saved = self.open_frame(decl);
        collect_returned_names(body, &mut self.returned);
        saved
    }

    fn close_frame(&mut self, saved: SavedFrame) {
        self.pop_scope();
        self.decl = saved.decl;
        self.slots = saved.slots;
        self.returned = saved.returned;
    }

    fn walk_decls(&mut self, items: &mut [TopDecl], prefix: &str) {
        for item in items.iter_mut() {
            match item {
                TopDecl::Fn(f) => self.walk_fn(f, prefix, "fn"),
                TopDecl::Locus(l) => self.walk_locus(l, prefix),
                TopDecl::Module(m) => self.walk_module(m, prefix),
                TopDecl::Perspective(p) => self.walk_perspective(p, prefix),
                TopDecl::Const(c) => {
                    let saved = self.open_frame(&format!(
                        "{}const {}",
                        prefix, c.name.name
                    ));
                    self.assign(
                        &mut c.value,
                        Decision::FrameTemp,
                        "const initialiser",
                    );
                    self.close_frame(saved);
                }
                TopDecl::Type(t) => {
                    let name = t.name.name.clone();
                    if let hale_syntax::ast::TypeDeclBody::Struct(fields) =
                        &mut t.body
                    {
                        for f in fields.iter_mut() {
                            let fname = f.name.name.clone();
                            if let Some(d) = &mut f.default {
                                let saved = self.open_frame(&format!(
                                    "{}type {}.{}",
                                    prefix, name, fname
                                ));
                                self.assign(
                                    d,
                                    Decision::FrameTemp,
                                    "record field default",
                                );
                                self.close_frame(saved);
                            }
                        }
                    }
                }
                TopDecl::Interface(i) => {
                    let iname = i.name.name.clone();
                    for m in i.methods.iter_mut() {
                        let decl =
                            format!("{}{}.{}", prefix, iname, m.name.name);
                        self.walk_param_defaults(&mut m.params, &decl);
                    }
                }
                TopDecl::Topic(_)
                | TopDecl::RingLayout(_)
                | TopDecl::Target(_)
                | TopDecl::Group(_)
                | TopDecl::Claims(_)
                | TopDecl::Constitution(_) => {}
            }
        }
    }

    fn walk_module(&mut self, m: &mut ModuleDecl, prefix: &str) {
        let inner = format!("{}{}::", prefix, m.name.name);
        self.walk_decls(&mut m.items, &inner);
    }

    fn walk_perspective(
        &mut self,
        p: &mut hale_syntax::ast::PerspectiveDecl,
        prefix: &str,
    ) {
        use hale_syntax::ast::PerspectiveMember;
        let pname = format!("{}{}", prefix, p.name.name);
        for m in p.members.iter_mut() {
            match m {
                PerspectiveMember::Fn(f) => {
                    self.walk_fn(f, &format!("{}.", pname), "perspective fn")
                }
                PerspectiveMember::StableWhen(b) => {
                    let saved =
                        self.open_frame(&format!("{}.stable_when", pname));
                    self.walk_block(b, None);
                    self.close_frame(saved);
                }
                PerspectiveMember::Params(pb) => {
                    let owner = pname.clone();
                    self.walk_params_block(pb, &owner);
                }
                PerspectiveMember::SerializeAs(_)
                | PerspectiveMember::Bus(_) => {}
            }
        }
    }

    fn walk_fn(&mut self, f: &mut FnDecl, prefix: &str, what: &str) {
        let decl = format!("{} {}{}", what, prefix, f.name.name);
        let saved = self.open_body_frame(&decl, &f.body);
        for p in f.params.iter_mut() {
            if let Some(d) = &mut p.default {
                self.assign(d, Decision::FrameTemp, "param default");
            }
        }
        self.walk_block(
            &mut f.body,
            Some((Decision::Caller, "fn body tail")),
        );
        self.close_frame(saved);
    }

    fn walk_param_defaults(&mut self, params: &mut [Param], decl: &str) {
        let saved = self.open_frame(decl);
        for p in params.iter_mut() {
            if let Some(d) = &mut p.default {
                self.assign(d, Decision::FrameTemp, "param default");
            }
        }
        self.close_frame(saved);
    }

    fn walk_params_block(
        &mut self,
        pb: &mut hale_syntax::ast::ParamsBlock,
        owner: &str,
    ) {
        // `accept` retains children written in a METHOD body; a
        // params default runs with `params_init_self`, not
        // `current_self`, and takes the ordinary field path.
        let saved_accepting = self.accepting.take();
        let saved_owner =
            std::mem::replace(&mut self.owner_locus, owner.to_string());
        for pd in pb.params.iter_mut() {
            let field = pd.name.name.clone();
            let kind = self
                .locus_fields
                .get(owner)
                .and_then(|f| f.get(&field))
                .copied()
                .unwrap_or(FieldKind::Value);
            let placed = self
                .placed_fields
                .get(owner)
                .map(|s| s.contains(&field))
                .unwrap_or(false);
            let ParamInit::Value(v) = &mut pd.init else { continue };
            let decl = format!("{}.params.{}", owner, field);
            let saved = self.open_frame(&decl);
            self.walk_field_init(
                ExprId::DECLARED,
                &field,
                v,
                kind,
                placed,
            );
            self.close_frame(saved);
        }
        self.owner_locus = saved_owner;
        self.accepting = saved_accepting;
    }

    fn walk_locus(&mut self, l: &mut LocusDecl, prefix: &str) {
        use hale_syntax::ast::{
            BusMember, ClosureClause, EpochSpec, KeyFilter, LifecycleKind,
            ModeKind, TypeDeclBody,
        };
        let lname = l.name.name.clone();
        let qualified = format!("{}{}", prefix, lname);
        if let Some(form) = &mut l.form {
            let mut args = std::mem::take(&mut form.args);
            let saved = self.open_frame(&format!("{} @form", qualified));
            for a in args.iter_mut() {
                self.assign(
                    &mut a.value,
                    Decision::FrameTemp,
                    "@form argument",
                );
            }
            self.close_frame(saved);
            if let Some(form) = &mut l.form {
                form.args = args;
            }
        }
        let saved_accepting = std::mem::replace(
            &mut self.accepting,
            self.accepts.get(&lname).cloned(),
        );
        let mut members = std::mem::take(&mut l.members);
        for m in members.iter_mut() {
            match m {
                LocusMember::Params(pb) => {
                    self.walk_params_block(pb, &lname)
                }
                LocusMember::Fn(f) => {
                    self.walk_fn(f, &format!("{}.", qualified), "method")
                }
                LocusMember::Lifecycle(lc) => {
                    let kind = match lc.kind {
                        LifecycleKind::Birth => "birth",
                        LifecycleKind::Accept => "accept",
                        LifecycleKind::Release => "release",
                        LifecycleKind::Run => "run",
                        LifecycleKind::Drain => "drain",
                        LifecycleKind::Dissolve => "dissolve",
                    };
                    let saved = self.open_body_frame(
                        &format!("{}.{}", qualified, kind),
                        &lc.body,
                    );
                    for p in lc.params.iter_mut() {
                        if let Some(d) = &mut p.default {
                            self.assign(
                                d,
                                Decision::FrameTemp,
                                "param default",
                            );
                        }
                    }
                    self.walk_block(
                        &mut lc.body,
                        Some((Decision::Caller, "body tail")),
                    );
                    self.close_frame(saved);
                }
                LocusMember::Mode(md) => {
                    let kind = match md.kind {
                        ModeKind::Bulk => "bulk",
                        ModeKind::Harmonic => "harmonic",
                        ModeKind::Resolution => "resolution",
                    };
                    let saved = self.open_body_frame(
                        &format!("{}.{}", qualified, kind),
                        &md.body,
                    );
                    self.walk_block(
                        &mut md.body,
                        Some((Decision::Caller, "body tail")),
                    );
                    self.close_frame(saved);
                }
                LocusMember::Failure(fd) => {
                    let saved = self
                        .open_frame(&format!("{}.on_failure", qualified));
                    self.walk_block(&mut fd.body, None);
                    self.close_frame(saved);
                }
                LocusMember::Closure(cd) => {
                    let saved =
                        self.open_frame(&format!("{}.closure", qualified));
                    if let Some(a) = &mut cd.assertion {
                        self.assign(
                            &mut a.left,
                            Decision::FrameTemp,
                            "closure assertion",
                        );
                        self.assign(
                            &mut a.right,
                            Decision::FrameTemp,
                            "closure assertion",
                        );
                        self.assign(
                            &mut a.tolerance,
                            Decision::FrameTemp,
                            "closure tolerance",
                        );
                    }
                    for c in cd.clauses.iter_mut() {
                        if let ClosureClause::Epoch(EpochSpec::Duration(e)) =
                            c
                        {
                            self.assign(
                                e,
                                Decision::FrameTemp,
                                "epoch duration",
                            );
                        }
                    }
                    self.close_frame(saved);
                }
                LocusMember::BirthCheck(bc) => {
                    let saved = self
                        .open_frame(&format!("{}.birth_check", qualified));
                    self.assign(
                        &mut bc.cond,
                        Decision::FrameTemp,
                        "birth-check condition",
                    );
                    if let Some(p) = &mut bc.payload {
                        self.assign(
                            p,
                            Decision::FrameTemp,
                            "birth-check payload",
                        );
                    }
                    self.close_frame(saved);
                }
                LocusMember::Bus(bb) => {
                    for bm in bb.members.iter_mut() {
                        if let BusMember::Subscribe {
                            key_filter:
                                Some(KeyFilter::Specific { expr, .. }),
                            ..
                        } = bm
                        {
                            let saved = self
                                .open_frame(&format!("{}.bus", qualified));
                            self.assign(
                                expr,
                                Decision::FrameTemp,
                                "key filter",
                            );
                            self.close_frame(saved);
                        }
                    }
                }
                LocusMember::Bindings(bb) => {
                    self.walk_bindings(bb, &qualified)
                }
                LocusMember::Const(c) => {
                    let saved = self.open_frame(&format!(
                        "{}.const {}",
                        qualified, c.name.name
                    ));
                    self.assign(
                        &mut c.value,
                        Decision::FrameTemp,
                        "const initialiser",
                    );
                    self.close_frame(saved);
                }
                LocusMember::Type(t) => {
                    let tname = t.name.name.clone();
                    if let TypeDeclBody::Struct(fields) = &mut t.body {
                        for f in fields.iter_mut() {
                            let fname = f.name.name.clone();
                            if let Some(d) = &mut f.default {
                                let saved = self.open_frame(&format!(
                                    "{}.type {}.{}",
                                    qualified, tname, fname
                                ));
                                self.assign(
                                    d,
                                    Decision::FrameTemp,
                                    "record field default",
                                );
                                self.close_frame(saved);
                            }
                        }
                    }
                }
                LocusMember::Contract(_)
                | LocusMember::Capacity(_)
                | LocusMember::Placement(_)
                | LocusMember::Topology(_)
                | LocusMember::Claims(_) => {}
            }
        }
        l.members = members;
        self.accepting = saved_accepting;
    }

    /// `bindings { }` transports and codecs are program-lifetime
    /// instances the runtime dissolves at main exit — `Placement`, per
    /// Riley's answer to F.39's third open question.
    fn walk_bindings(
        &mut self,
        bb: &mut hale_syntax::ast::BindingsBlock,
        owner: &str,
    ) {
        use hale_syntax::ast::TransportSpec;
        for entry in bb.entries.iter_mut() {
            let topic = entry.topic.name.clone();
            if let TransportSpec::Adapter { locus, inits, .. } =
                &mut entry.transport
            {
                let lname = locus.name.clone();
                let mut taken = std::mem::take(inits);
                let saved = self.open_frame(&format!(
                    "{}.bindings {} adapter",
                    owner, topic
                ));
                self.walk_binding_inits(&lname, &mut taken, &topic);
                self.close_frame(saved);
                if let TransportSpec::Adapter { inits, .. } =
                    &mut entry.transport
                {
                    *inits = taken;
                }
            }
            if let Some(codec) = &mut entry.codec {
                let lname = codec.locus.name.clone();
                let mut taken = std::mem::take(&mut codec.inits);
                let saved = self.open_frame(&format!(
                    "{}.bindings {} codec",
                    owner, topic
                ));
                self.walk_binding_inits(&lname, &mut taken, &topic);
                self.close_frame(saved);
                if let Some(codec) = &mut entry.codec {
                    codec.inits = taken;
                }
            }
        }
    }

    fn walk_binding_inits(
        &mut self,
        locus: &str,
        inits: &mut [StructInit],
        topic: &str,
    ) {
        for init in inits.iter_mut() {
            let kind = self
                .locus_fields
                .get(locus)
                .and_then(|f| f.get(&init.name.name))
                .copied()
                .unwrap_or(FieldKind::Value);
            if kind == FieldKind::Holder
                && self.is_locus_producing(&init.value)
            {
                self.assign(
                    &mut init.value,
                    Decision::Placement(topic.to_string()),
                    "bindings transport field",
                );
            } else {
                self.assign(
                    &mut init.value,
                    Decision::FrameTemp,
                    "bindings transport field",
                );
            }
        }
    }
}

// ===================================================================
// Shadow reporting
// ===================================================================

/// How loudly the shadow speaks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShadowMode {
    /// The default: the table is built (so the pre-pass runs on every
    /// compile) but nothing is compared and nothing is printed. This
    /// PR changes no behaviour, and that includes stderr.
    Off,
    /// `LOTUS_OWNER_SHADOW=1` / `=log`: compare, and print one line
    /// per distinct disagreement so a corpus or matrix run finishes
    /// with every disagreement LISTED.
    Log,
    /// `LOTUS_OWNER_SHADOW=strict`: a disagreement — or an
    /// instantiation with no table entry — is a `CodegenError`.
    Strict,
}

pub fn shadow_mode() -> ShadowMode {
    match std::env::var("LOTUS_OWNER_SHADOW").as_deref() {
        Ok("strict") => ShadowMode::Strict,
        Ok("1") | Ok("log") | Ok("on") => ShadowMode::Log,
        _ => ShadowMode::Off,
    }
}

/// One line per distinct disagreement, deduplicated per process so a
/// corpus run produces a list rather than a transcript. Written to
/// `LOTUS_OWNER_SHADOW_LOG` when that names a file, so an in-process
/// matrix run can collect them without `--nocapture`.
pub fn report(line: &str) {
    use std::io::Write;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static SEEN: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(BTreeSet::new()));
    {
        let mut g = seen.lock().expect("owner-shadow log mutex");
        if !g.insert(line.to_string()) {
            return;
        }
    }
    let text = format!("[owner-shadow] {line}\n");
    match std::env::var("LOTUS_OWNER_SHADOW_LOG") {
        Ok(path) if !path.is_empty() => {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = f.write_all(text.as_bytes());
                return;
            }
            eprint!("{text}");
        }
        _ => eprint!("{text}"),
    }
}
