//! GH #921 — locus ownership is resolved before lowering.
//!
//! This module lives in the frontend (`hale-types`) because the
//! `ownership` family's home is the frontend: F.40 phase 1 moves the
//! pre-pass here with its algorithm unchanged, together with the
//! fresh-factory seed it is handed ([`fresh_factories`],
//! whose one producer this is). The frontend's resolved-program step
//! (`crate::resolved::resolve_rewritten`) runs the pass over the merged
//! program it hands codegen, and codegen reads the tables from that
//! envelope; it re-exports this module as `hale_codegen::ownership`.
//!
//! `spec/decisions.md` F.39 is the design. The short version: locus
//! ownership USED to be decided by seven one-shot flags on `Cx`
//! (`suppress_fresh_temp`, `defer_next_locus_dissolve`,
//! `instantiating_for_parent_field`,
//! `placement_for_next_locus_instantiation`, `or_field_owner_locus`,
//! the `returns_this_locus` / `current_user_fn_ret` spoof, and the
//! field-ownership predicates), each consumed by "the next literal or
//! call lowered". Every teardown leak the 2026-09 sweep fixed was a
//! flag taken by the wrong node. This module makes the same decisions
//! ONCE, from syntactic position, into a side table keyed by
//! expression identity, and lowering READS it — an instantiation with
//! no row is a `CodegenError`. A2 (PR #937) built the table and only
//! checked it against the flags; A3 switched the consumers over, one
//! flag per commit, and the flags are gone.
//!
//! ## The key
//!
//! [`ExprId`] is the [`NodeId`] the snapshot minted into the AST over
//! the merged, desugared program (`crate::snapshot::mint`, run by the
//! resolved-program step before this pass); the pass numbers nothing,
//! and refuses a literal or call the mint left `NONE`. It has to live in the
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
//! literal and a call. A node codegen synthesises after this pass has
//! to carry over the id of the source expression it stands for (the
//! generic-struct path rewrites do) or declare its owner at the site
//! (`Cx::declared_owner`); anything else keeps `NodeId::NONE` and is
//! refused with a span.
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
//! | an argument a callee hands back unchanged (`id(W { })`) | the call's owner |
//! | an argument a fresh factory keeps in the instance it returns (`mk(W { })`), that instance held by a field, a placement or a binding | [`Owner::Field`] of the call, its field |
//! | everything else — receiver, argument, operand, index, condition, bare statement | [`Owner::FrameTemp`] |
//!
//! A decision that passes THROUGH a delegating node — an `or`, a
//! carrier, a composite — is demoted from `Binding` to
//! `FrameTemp(innermost reclaim scope)`, because the binding names the
//! join and not the arm: the arm's value is materialised in a
//! temporary the frame reclaims — once per ITERATION inside a loop,
//! which is what GH #921 A4's fourth family was about.
//! `Caller`, `Field` and `Placement` are not demoted — the value
//! really does leave the frame, or the field's mask bit really does
//! claim whichever branch ran (GH #853).
//!
//! The innermost reclaim scope is the enclosing frame, or the
//! enclosing LOOP body when there is one: GH #824 gave a `let` in a
//! loop a per-iteration slot, and A4 found that a carrier or composite
//! RHS — which takes the GH #402 frame temporary instead — had none.
//! That difference is what [`ScopeKind`] carries, and A3 commit 1 gave
//! the GH #402 temporary the same per-iteration reclaim.
//!
//! ## The fresh-factory set
//!
//! The table derives factory calls from an EXTENDED set: the one
//! `fresh_factories` computes, plus every fn whose
//! return arms are fresh once `if` / `match` / block tails are
//! flattened, and every fn that hands back a BINDING of one.
//! `fresh_factories::collect` classifies the carrier
//! node and never its arms, which is why `return if c { make(1) }
//! else { make(2) }` was not a factory and its caller's binding did
//! not own the result — the 105-cell carrier-return family.
//!
//! GH #921 A3 commit 1 folds the extension back into the map lowering
//! reads ([`OwnerTable::extended_fresh_factories`], applied in
//! `lower_program`), so the two sides of every ownership decision are
//! computed once and cannot drift. It lives here rather than inside
//! `fresh_factories` because the flattening is the same
//! walk the table already does to decide each arm.
//!
//! The rows [`fresh_factories`] produces also carry what a call to
//! each factory constructs ([`FactoryRow::products`]), which is the
//! checker's self-containment rule's question (GH #870). The rule
//! reads the same rows over the bundle's programs rather than keeping
//! a mirror of the classification (F.40 phase 1.2c). The products do
//! not depend on the escape walk, which answers only the row's
//! [`FactoryRow::fresh`] half: a fn that passes its returned binding
//! to a call still constructs it.
//!
//! ## What lowering asks
//!
//! `lower_locus_instantiation` resolves its own [`Owner`] from this
//! table (or from the site's declaration) and derives every decision
//! it used to take from a flag: whether the enclosing frame reclaims
//! the instance at its flush, whether a param field owns it and its
//! mask bit claims it, whether a `placement { }` entry applies, and
//! whether the caller reclaims it. The GH #402 hook in `lower_expr`
//! and the GH #793 hook in `lower_or_expr` ask
//! [`OwnerTable::temp_verdict`] the same question for a factory
//! call's result.
//!
//! ## Binding facts
//!
//! A `let` asks three more questions of its own binding: does the
//! body hand it back, does a bare `=` move a value through it, and is
//! it a `[c; N]` that never escapes the frame. [`resolve_binding_facts`]
//! answers them once per binding site, into
//! [`OwnerTable::binding_facts`], one row per `let` carrying its
//! snapshot identity (F.40 phase 1.2b; a `SiteId` since the phase-1
//! review), and the `let` lowering reads its own row. A `let` in a body no walk reads has a row that says `false`
//! three times, and so does a `let` with no row at all. A generic fn's
//! monomorphs keep the template's identities, so a monomorph's `let`
//! reads the template's answer.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    AssignOp, Block, ElseBranch, Expr, FnDecl, Ident, IfStmt, LValueSeg,
    LocusDecl, LocusMember, MatchArmBody, MatchStmt, ModuleDecl, NodeId, OrDisposition, Param, ParamInit, Pattern, Program,
    QualifiedName, RecoveryModifier, Stmt, StructInit, TopDecl, TypeExpr,
};
use hale_graph::ids::SiteId;
use hale_syntax::Span;

// ===================================================================
// Ids
// ===================================================================

/// The identity of a locus-producing expression: the [`NodeId`] the
/// snapshot minted into the node. See the module docs for why it
/// cannot be an address or a span.
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
#[derive(Debug, Default, Clone)]
pub struct OwnerTable {
    entries: BTreeMap<ExprId, Entry>,
    /// `(owner locus, field)` -> the row for a field initialised from
    /// a NAME. Keyed separately because `Expr::Ident` carries no id.
    borrowed: BTreeMap<(String, String), Entry>,
    scope_kinds: Vec<ScopeKind>,
    /// The EXTENDED proven-fresh factory set: fn name -> locus.
    fresh: BTreeMap<String, String>,
    /// One row per `let`, with the statement's full snapshot identity
    /// (see [`resolve_binding_facts`]). Keyed by the identity's index:
    /// one counter numbers every seed of the resolved snapshot, so the
    /// index alone is unique, and it is what the `let` lowering holds
    /// (the `NodeId` on the statement), so its lookup is one search
    /// with no detour through the snapshot for the seed.
    bindings: BTreeMap<u32, (SiteId, BindingFacts)>,
    /// GH #1566 follow-up: a call of a fn that hands back one of its
    /// arguments unchanged (`id(W { })`), by the call: that argument's
    /// position. The argument is the call's value, and the site that
    /// owns the call's value owns it.
    passes: BTreeMap<ExprId, usize>,
    /// GH #1566 follow-up: a call of a fresh factory that keeps an
    /// argument it was handed in a field of the instance it builds and
    /// returns (`mk(W { })`, `fn mk(w: W) -> R { return R { w: w }; }`),
    /// by the call: those fields. The argument is that field's child,
    /// its owned bit set on the instance the call returns.
    kept: BTreeMap<ExprId, Vec<String>>,
    /// GH #1569: a call of a fn handing back a tuple or an array of loci
    /// (`ParamFlow::parts`), by the call: which parts the call's owner
    /// owns — the holder of the aggregate (a field: its elements'
    /// children; a binding or the frame: reclaimed with the frame). A part
    /// handed in as an argument the frame already owns is the frame's.
    aggregates: BTreeMap<ExprId, Vec<bool>>,
    /// GH #1569: the calls through a fn value the table decided (only a
    /// fresh factory may be one): when it hands back a tuple or an array,
    /// every locus part of it is the call's owner's.
    fn_value_calls: BTreeSet<ExprId>,
    /// GH #1569: an argument a literal part of that aggregate keeps (and
    /// no part is itself): by the call, (part, field) — the last part
    /// keeping it, torn down after every other part that keeps it reads
    /// it, owns it, its owned bit set at the call.
    aggregate_kept: BTreeMap<ExprId, Vec<(usize, String)>>,
}

/// What a `let` needs to know about its own binding before it lowers
/// the right-hand side. One row per binding site; see the module
/// docs' "Binding facts".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BindingFacts {
    /// The body hands the binding back (a `return` or a counted tail
    /// names it, directly or through another handed-back binding):
    /// the caller owns its value. See [`ReturnedBindings`].
    pub returned: bool,
    /// The binding's name is on either side of a bare-local `=` in the
    /// body: a value moves through it. See [`assign_moved_names`].
    pub assign_moved: bool,
    /// The binding is a literal `[c; N]` whose every use is an element
    /// access: its storage can live in the frame. See
    /// [`stack_array_names`].
    pub stack_array: bool,
}

impl OwnerTable {
    /// The facts for the `let` whose snapshot identity is `id`. `None`
    /// for a `NONE` id and for a site the resolved program did not
    /// contain. Lowering refuses a `let` with no row: its defaults are
    /// the answers that dissolve the value.
    pub fn binding_facts(&self, id: NodeId) -> Option<&BindingFacts> {
        if id.is_none() {
            return None;
        }
        self.bindings.get(&id.0).map(|(_, f)| f)
    }

    /// The snapshot identity of the `let` whose node carries `id`, if
    /// it has a row.
    pub fn binding_site(&self, id: NodeId) -> Option<SiteId> {
        if id.is_none() {
            return None;
        }
        self.bindings.get(&id.0).map(|(s, _)| *s)
    }

    /// Every binding row, by snapshot identity, in index order.
    pub fn binding_rows(&self) -> impl Iterator<Item = (SiteId, &BindingFacts)> {
        self.bindings.values().map(|(s, f)| (*s, f))
    }

    /// The id this pass gave the node, or `None` when the node carries
    /// no id at all (a shape that cannot produce a locus) or was built
    /// after the pass ran.
    pub fn id_of(&self, e: &Expr) -> Option<ExprId> {
        node_id(e).filter(|n| !n.is_none()).map(|n| ExprId(n.0))
    }

    /// GH #1566 follow-up: the fields of the instance a factory call
    /// returns that hold, as owned children, arguments the call was
    /// handed (`OwnerTable::kept`).
    /// GH #1569: the parts of the aggregate a call hands back that its
    /// owner owns (`OwnerTable::aggregates`), by position.
    pub fn aggregate_kept(&self, call: &Expr) -> &[(usize, String)] {
        self.id_of(call).and_then(|id| self.aggregate_kept.get(&id)).map_or(&[], |v| v.as_slice())
    }

    /// Whether the owner of the aggregate `call` hands back owns its part
    /// `k` (`OwnerTable::aggregates`; a call through a fn value owns every
    /// locus part); `None` for a call the table decided no parts of.
    pub fn aggregate_part_owned(&self, call: &Expr, k: usize) -> Option<bool> {
        let id = self.id_of(call)?;
        if let Some(m) = self.aggregates.get(&id) {
            return Some(m.get(k).copied().unwrap_or(false));
        }
        self.fn_value_calls.contains(&id).then_some(true)
    }

    /// Whether the table decided the parts of the aggregate `call` may
    /// hand back (a call through a fn value may return one).
    pub fn is_aggregate_call(&self, call: &Expr) -> bool {
        self.id_of(call).is_some_and(|id| self.aggregates.contains_key(&id) || self.fn_value_calls.contains(&id))
    }

    pub fn kept_fields(&self, call: &Expr) -> &[String] {
        self.id_of(call).and_then(|id| self.kept.get(&id)).map_or(&[], |v| v.as_slice())
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
    /// carrier-return arms `fresh_factories` misses are
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
            // a call that hands back an argument: that argument's row
            Expr::Call { args, .. } if self.id_of(e).and_then(|id| self.passes.get(&id)).is_some() => {
                let k = self.id_of(e).and_then(|id| self.passes.get(&id)).copied().expect("checked");
                args.get(k).and_then(|a| self.leaf_entry_of(a))
            }
            Expr::Struct { .. } | Expr::Call { .. } => self.entry_of(e),
            Expr::Or { inner, .. } => self.leaf_entry_of(inner),
            Expr::If(_) | Expr::Match(_) | Expr::Block(_) => {
                let mut arms = Vec::new();
                return_arms(e, &mut arms);
                arms.first().and_then(|a| self.leaf_entry_of(a))
            }
            // (a tuple's first part with a row: `(W { }, 1)`, GH #1569)
            Expr::Array(parts, _) | Expr::Tuple(parts, _) => {
                parts.iter().find_map(|p| self.leaf_entry_of(p))
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
    returned: ReturnedBindings,
}

// ===================================================================
// Bindings, not names (GH #1140)
// ===================================================================

/// Which bindings a body hands back, each the declaration the snapshot
/// resolved it to (`Snapshot::binding_of`, F.40 phase 2): see
/// [`returned_bindings`]. Owned, so a walk that mutates the body can
/// hold it.
///
/// Every uncertainty answers "handed back", which suppresses a reclaim
/// (the old leak) and never adds one: a `let` or a `=` the snapshot did
/// not mint answers by name against every name a return spelled.
#[derive(Default, Debug, Clone)]
pub struct ReturnedBindings {
    /// The `let`s handed back, directly or through the value of another
    /// handed-back binding.
    decls: BTreeSet<SiteId>,
    /// Every name a return (or a counted block tail) spells as a value
    /// arm: the by-name answer, for what the snapshot did not mint.
    all_names: BTreeSet<String>,
    /// The names among those that name no `let` of the body — a `for`
    /// variable, a pattern, a param. A `let` spelled the same is counted
    /// too: such a binding can hold the `let`'s value (an element of an
    /// array the `let` went into), which no resolution says.
    nonlet_names: BTreeSet<String>,
    /// The body's `let`s and tuple `let`s' names, by identity.
    lets: BTreeSet<SiteId>,
}

impl ReturnedBindings {
    fn decl_is_returned(&self, d: SiteId, name: &str) -> bool {
        self.decls.contains(&d) || self.nonlet_names.contains(name)
    }

    /// Whether the `let` whose statement is `site`, declaring `name`,
    /// is one the body hands back.
    pub fn let_is_returned(&self, site: Option<SiteId>, name: &str) -> bool {
        match site {
            Some(s) if self.lets.contains(&s) => self.decl_is_returned(s, name),
            _ => self.all_names.contains(name),
        }
    }

    /// Whether a bare `=` writing `head` writes a binding the body hands
    /// back: `decl` is the declaration the statement's head names.
    pub fn assign_is_returned(&self, decl: Option<SiteId>, head: &str) -> bool {
        match decl {
            Some(d) if self.lets.contains(&d) => self.decl_is_returned(d, head),
            _ => self.all_names.contains(head),
        }
    }
}

/// A body's bindings as the fresh-factory walk reads them.
struct BodyBindings<'e, 's> {
    ids: &'s crate::snapshot::Snapshot,
    returned: ReturnedBindings,
    /// Each `let`'s right-hand side.
    rhs: BTreeMap<SiteId, &'e Expr>,
    /// Every value the body hands back: each `return <e>;`, wherever it
    /// stands, and each counted block tail.
    returns: Vec<&'e Expr>,
    /// `let`s some `=` writes, and each such write's value.
    assigned: BTreeSet<SiteId>,
    writes: Vec<(SiteId, &'e Expr)>,
    /// Names written by an `=` that names no `let` of the body.
    assigned_names: BTreeSet<String>,
}

impl<'e> BodyBindings<'e, '_> {
    /// The `let` of the body a name spelled at `use_site` names.
    fn let_at(&self, use_site: &Ident) -> Option<SiteId> {
        self.ids
            .declaration_of(use_site.id)
            .filter(|d| self.returned.lets.contains(d))
    }

    /// Whether some bare `=` may write the binding `use_site` names.
    fn is_assigned(&self, use_site: &Ident, d: SiteId) -> bool {
        self.assigned.contains(&d) || self.assigned_names.contains(&use_site.name)
    }

    /// Count `e`'s value arms as handed back: a name among them is the
    /// binding it names.
    fn hand_back(&mut self, e: &'e Expr) {
        let mut arms = Vec::new();
        return_arms(e, &mut arms);
        for a in arms {
            if let Expr::Ident(i) = a {
                self.returned.all_names.insert(i.name.clone());
                match self.let_at(i) {
                    Some(d) => {
                        self.returned.decls.insert(d);
                    }
                    None => {
                        self.returned.nonlet_names.insert(i.name.clone());
                    }
                }
            }
        }
    }
}

/// The GH #1140 walk, reading the snapshot's resolution: every
/// statement and expression of a body, for its `let`s, its writes and
/// what it hands back. A statement-level block's tail counts as handed
/// back (`counted`); an expression block's is its expression's value.
/// The matches are exhaustive on purpose: a new statement or expression
/// form must say what it declares and hands back before this compiles.
struct BodyWalk<'e, 's> {
    out: BodyBindings<'e, 's>,
    /// Each bare `=`: the declaration its head names, the head, the
    /// value, and whether it writes the binding whole.
    assigns: Vec<(Option<SiteId>, &'e str, &'e Expr, bool)>,
}

/// Collect a body's bindings.
fn body_bindings<'e, 's>(b: &'e Block, ids: &'s crate::snapshot::Snapshot) -> BodyBindings<'e, 's> {
    let out = BodyBindings {
        ids,
        returned: ReturnedBindings::default(),
        rhs: BTreeMap::new(),
        returns: Vec::new(),
        assigned: BTreeSet::new(),
        writes: Vec::new(),
        assigned_names: BTreeSet::new(),
    };
    let mut w = BodyWalk { out, assigns: Vec::new() };
    w.block(b, true);
    let BodyWalk { mut out, assigns } = w;
    for (decl, head, value, whole) in assigns {
        match decl.filter(|d| out.returned.lets.contains(d)) {
            Some(d) => {
                out.assigned.insert(d);
                if whole {
                    out.writes.push((d, value));
                }
            }
            None => {
                out.assigned_names.insert(head.to_string());
            }
        }
    }
    // What the body hands back...
    let returns = out.returns.clone();
    for e in returns {
        out.hand_back(e);
    }
    // ...and what flows into it: a handed-back binding's `let` value
    // and every `=` into it hand back the bindings among their arms
    // (`let y = if c { let r = make(); r } else { … }; return y;`).
    loop {
        let before = out.returned.decls.len() + out.returned.nonlet_names.len();
        let flows: Vec<&Expr> = out
            .rhs
            .iter()
            .filter(|(d, _)| out.returned.decls.contains(d))
            .map(|(_, e)| *e)
            .chain(
                out.writes
                    .iter()
                    .filter(|(d, _)| out.returned.decls.contains(d))
                    .map(|(_, e)| *e),
            )
            .collect();
        for e in flows {
            out.hand_back(e);
        }
        if out.returned.decls.len() + out.returned.nonlet_names.len() == before {
            break;
        }
    }
    out
}

/// The bindings a body hands back (see [`ReturnedBindings`]), each the
/// declaration `ids` (the snapshot the body was minted in) resolves the
/// returned name to.
///
/// GH #383: distinct from `fresh_factories` and needed
/// separately: a fn that does NOT qualify as a clean factory can still
/// return a locus it bound from one. `nn::forward` is the case that
/// proved it — it binds several factory results, returns one, and
/// fails the freshness walk; the caller-scoped dissolve fired on the
/// binding it hands back and the caller read zeros. Conservative by
/// construction: a `true` merely suppresses a dissolve, which is the
/// old leak — never a double-free.
pub fn returned_bindings(b: &Block, ids: &crate::snapshot::Snapshot) -> ReturnedBindings {
    body_bindings(b, ids).returned
}

impl<'e> BodyWalk<'e, '_> {
    fn declare(&mut self, id: NodeId, rhs: Option<&'e Expr>) {
        if let Some(s) = self.out.ids.site_id(id) {
            self.out.returned.lets.insert(s);
            if let Some(e) = rhs {
                self.out.rhs.insert(s, e);
            }
        }
    }

    fn block(&mut self, b: &'e Block, counted: bool) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = b.tail.as_deref() {
            self.expr(t);
            if counted {
                self.out.returns.push(t);
            }
        }
    }

    fn if_chain(&mut self, i: &'e IfStmt, counted: bool) {
        self.expr(&i.cond);
        self.block(&i.then_block, counted);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b, counted),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n, counted),
            None => {}
        }
    }

    fn match_arms(&mut self, m: &'e MatchStmt, counted: bool) {
        self.expr(&m.scrutinee);
        for a in &m.arms {
            if let Some(g) = &a.guard {
                self.expr(g);
            }
            match &a.body {
                MatchArmBody::Block(b) => self.block(b, counted),
                MatchArmBody::Expr(x) => self.expr(x),
            }
        }
    }

    fn or_disposition(&mut self, d: &'e OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => self.expr(e),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }

    fn stmt(&mut self, s: &'e Stmt) {
        match s {
            Stmt::Let { value, id, .. } => {
                self.expr(value);
                self.declare(*id, Some(value));
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value);
                for n in names {
                    self.declare(n.id, None);
                }
            }
            Stmt::Assign { target, value, id, .. } => {
                for seg in &target.tail {
                    match seg {
                        LValueSeg::Index(ix) => self.expr(ix),
                        LValueSeg::Field(_) => {}
                    }
                }
                self.expr(value);
                let decl = self.out.ids.declaration_of(*id);
                self.assigns.push((decl, &target.head.name, value, target.tail.is_empty()));
            }
            Stmt::Return(value, _) => {
                if let Some(e) = value {
                    self.expr(e);
                    self.out.returns.push(e);
                }
            }
            Stmt::If(i) => self.if_chain(i, true),
            Stmt::Match(m) => self.match_arms(m, true),
            Stmt::For { iter, body, .. } => {
                self.expr(iter);
                self.block(body, true);
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max);
                self.block(body, true);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body, true);
            }
            Stmt::Block(body) => self.block(body, true),
            Stmt::Fail { value, .. } => self.expr(value),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.expr(a);
                }
                match modifier {
                    Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) => self.expr(e),
                    None => {}
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.expr(p);
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.or_disposition(d);
                }
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }

    fn expr(&mut self, e: &'e Expr) {
        match e {
            Expr::Ident(_) | Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Tuple(v, _) | Expr::Array(v, _) => {
                for x in v {
                    self.expr(x);
                }
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.expr(&i.value);
                }
            }
            Expr::Block(b) => self.block(b, false),
            Expr::If(i) => self.if_chain(i, false),
            Expr::Match(m) => self.match_arms(m, false),
            Expr::Sum(x, _) | Expr::Prod(x, _) => self.expr(x),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                self.or_disposition(disposition);
            }
        }
    }
}

// ===================================================================
// Binding facts (F.40 phase 1.2b)
// ===================================================================

/// Bindings that participate in a plain `=` between locals
/// (`a = nx;`, `a = make(...);`) — downstream handoff, free-fn
/// locus rebinding. An assignment MOVES a value between bindings
/// without the binding-scoped ownership rule seeing it: the
/// moved-from binding's scope-exit dissolve would fire on a value
/// the target (and possibly the caller) still holds, and the
/// target's own dissolve can fire on a value another binding
/// registered. Any name on either side of a bare-local `=` is
/// therefore disqualified from frame-scoped reclamation.
///
/// Conservative by construction, same stance as
/// [`ReturnedBindings`]: membership only suppresses a dissolve — the
/// old leak, never a double-free.
pub fn assign_moved_names(b: &Block) -> BTreeSet<String> {
    fn walk(b: &Block, out: &mut BTreeSet<String>) {
        for s in &b.stmts {
            match s {
                Stmt::Assign { target, op, value, .. } => {
                    if matches!(op, AssignOp::Eq) && target.tail.is_empty() {
                        out.insert(target.head.name.clone());
                        if let Expr::Ident(i) = value {
                            out.insert(i.name.clone());
                        }
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    walk(body, out)
                }
                Stmt::If(i) => {
                    walk(&i.then_block, out);
                    let mut cur = i.else_block.as_deref();
                    while let Some(eb) = cur {
                        match eb {
                            ElseBranch::Else(bb) => {
                                walk(bb, out);
                                cur = None;
                            }
                            ElseBranch::ElseIf(ei) => {
                                walk(&ei.then_block, out);
                                cur = ei.else_block.as_deref();
                            }
                        }
                    }
                }
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        if let MatchArmBody::Block(bb) = &arm.body {
                            walk(bb, out);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(b, &mut out);
    out
}

/// GH #767: the `let` bindings of one body whose initializer is a
/// literal `[c; N]` and whose every use in the body is an element
/// read (`t[i]`) or an element write (`t[i] = v`). Those are the
/// bindings whose storage can live in the fn's own frame instead of
/// an arena.
///
/// Why it matters: a free fn's temporaries are allocated in the
/// CALLER's arena and are not reclaimed until the caller returns, so a
/// fixed scratch table inside a helper is per-call churn for the whole
/// lifetime of the loop that calls it — 1.69 GB of RSS over 200k calls
/// in the measurement on #754.
///
/// Conservative by construction, and it has to be: a wrong answer here
/// is a dangling stack pointer, not a leak. The walker whitelists the
/// two element-access shapes and treats EVERY other occurrence of the
/// name — a bare mention, a call argument, a `return`, a field store, a
/// publish, a `for ... in t`, an alias `let u = t;` — as an escape. Any
/// `Expr`/`Stmt` variant added later must be handled explicitly: both
/// walkers match exhaustively, with no `_` arm.
///
/// A name bound more than once in one body, shadowed by a parameter, or
/// re-bound by a bare `t = ...` is dropped outright rather than
/// reasoned about.
pub fn stack_array_names(params: &[Param], body: &Block) -> BTreeSet<String> {
    /// Every occurrence of `name` in `e` is an element access.
    fn expr_uses_are_elementwise(e: &Expr, name: &str) -> bool {
        match e {
            // A bare mention hands the array's ADDRESS to whatever
            // context it sits in. Unclassifiable — treat as escape.
            Expr::Ident(i) => i.name != name,
            Expr::Index { receiver, index, .. } => {
                let recv_ok = match receiver.as_ref() {
                    // `t[i]` yields the ELEMENT, by value. The storage
                    // address stops here.
                    Expr::Ident(i) if i.name == name => true,
                    other => expr_uses_are_elementwise(other, name),
                };
                recv_ok && expr_uses_are_elementwise(index, name)
            }
            Expr::Literal(_, _) | Expr::Path(_) | Expr::KwSelf(_) => true,
            Expr::Binary { left, right, .. }
            | Expr::Range { lo: left, hi: right, .. } => {
                expr_uses_are_elementwise(left, name)
                    && expr_uses_are_elementwise(right, name)
            }
            Expr::Unary { operand, .. } => {
                expr_uses_are_elementwise(operand, name)
            }
            Expr::Call { callee, args, .. } => {
                expr_uses_are_elementwise(callee, name)
                    && args
                        .iter()
                        .all(|a| expr_uses_are_elementwise(a, name))
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
                expr_uses_are_elementwise(receiver, name)
            }
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => {
                xs.iter().all(|x| expr_uses_are_elementwise(x, name))
            }
            Expr::Struct { inits, .. } => inits
                .iter()
                .all(|si| expr_uses_are_elementwise(&si.value, name)),
            Expr::Block(b) => block_uses_are_elementwise(b, name),
            Expr::If(i) => if_uses_are_elementwise(i, name),
            Expr::Match(m) => match_uses_are_elementwise(m, name),
            Expr::Sum(x, _) | Expr::Prod(x, _) => {
                expr_uses_are_elementwise(x, name)
            }
            Expr::Approx { left, right, tolerance, .. } => {
                expr_uses_are_elementwise(left, name)
                    && expr_uses_are_elementwise(right, name)
                    && expr_uses_are_elementwise(tolerance, name)
            }
            Expr::ArrayRepeat { val, .. } => {
                expr_uses_are_elementwise(val, name)
            }
            Expr::Or { inner, .. } => expr_uses_are_elementwise(inner, name),
        }
    }

    fn if_uses_are_elementwise(i: &IfStmt, name: &str) -> bool {
        if !expr_uses_are_elementwise(&i.cond, name)
            || !block_uses_are_elementwise(&i.then_block, name)
        {
            return false;
        }
        match i.else_block.as_deref() {
            None => true,
            Some(ElseBranch::Else(b)) => block_uses_are_elementwise(b, name),
            Some(ElseBranch::ElseIf(inner)) => {
                if_uses_are_elementwise(inner, name)
            }
        }
    }

    fn match_uses_are_elementwise(m: &MatchStmt, name: &str) -> bool {
        if !expr_uses_are_elementwise(&m.scrutinee, name) {
            return false;
        }
        m.arms.iter().all(|arm| {
            let guard_ok = arm
                .guard
                .as_ref()
                .map(|g| expr_uses_are_elementwise(g, name))
                .unwrap_or(true);
            let body_ok = match &arm.body {
                MatchArmBody::Expr(e) => expr_uses_are_elementwise(e, name),
                MatchArmBody::Block(b) => block_uses_are_elementwise(b, name),
            };
            guard_ok && body_ok
        })
    }

    fn stmt_uses_are_elementwise(s: &Stmt, name: &str) -> bool {
        match s {
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
                expr_uses_are_elementwise(value, name)
            }
            Stmt::Assign { target, value, .. } => {
                let target_ok = if target.head.name == name {
                    // `t[i] = v` writes an element. Anything else with
                    // `t` at the head — `t = x` (a rebind), `t.f = x` —
                    // is not an element write.
                    match target.tail.as_slice() {
                        [LValueSeg::Index(ix)] => {
                            expr_uses_are_elementwise(ix, name)
                        }
                        _ => false,
                    }
                } else {
                    target.tail.iter().all(|seg| match seg {
                        LValueSeg::Index(ix) => {
                            expr_uses_are_elementwise(ix, name)
                        }
                        LValueSeg::Field(_) => true,
                    })
                };
                target_ok && expr_uses_are_elementwise(value, name)
            }
            Stmt::If(i) => if_uses_are_elementwise(i, name),
            Stmt::Match(m) => match_uses_are_elementwise(m, name),
            // `for x in t` reads elements, but the lowering walks the
            // storage — left out on purpose, the conservative side.
            Stmt::For { iter, body, .. } => {
                expr_uses_are_elementwise(iter, name)
                    && block_uses_are_elementwise(body, name)
            }
            Stmt::While { cond, body, .. } => {
                expr_uses_are_elementwise(cond, name)
                    && block_uses_are_elementwise(body, name)
            }
            Stmt::Return(v, _) => v
                .as_ref()
                .map(|e| expr_uses_are_elementwise(e, name))
                .unwrap_or(true),
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => true,
            Stmt::Fail { value, .. } => expr_uses_are_elementwise(value, name),
            Stmt::Block(b) => block_uses_are_elementwise(b, name),
            Stmt::Recovery { args, .. } => {
                args.iter().all(|a| expr_uses_are_elementwise(a, name))
            }
            Stmt::Violate { payload, .. } => payload
                .as_ref()
                .map(|e| expr_uses_are_elementwise(e, name))
                .unwrap_or(true),
            Stmt::Send { subject, value, .. } => {
                expr_uses_are_elementwise(subject, name)
                    && expr_uses_are_elementwise(value, name)
            }
            Stmt::ShmWrite { max, body, .. } => {
                expr_uses_are_elementwise(max, name)
                    && block_uses_are_elementwise(body, name)
            }
            Stmt::Expr(e) => expr_uses_are_elementwise(e, name),
        }
    }

    fn block_uses_are_elementwise(b: &Block, name: &str) -> bool {
        b.stmts.iter().all(|s| stmt_uses_are_elementwise(s, name))
            && b.tail
                .as_deref()
                .map(|t| expr_uses_are_elementwise(t, name))
                .unwrap_or(true)
    }

    /// Candidates (a `let` whose RHS is a literal `[c; N]`) and every
    /// other name the body binds. A name in both — a shadow, a second
    /// `let`, a loop variable — is dropped.
    fn collect_binders(
        b: &Block,
        candidates: &mut Vec<String>,
        other: &mut BTreeSet<String>,
    ) {
        fn visit_if(
            i: &IfStmt,
            candidates: &mut Vec<String>,
            other: &mut BTreeSet<String>,
        ) {
            collect_binders(&i.then_block, candidates, other);
            match i.else_block.as_deref() {
                None => {}
                Some(ElseBranch::Else(bb)) => {
                    collect_binders(bb, candidates, other)
                }
                Some(ElseBranch::ElseIf(inner)) => {
                    visit_if(inner, candidates, other)
                }
            }
        }
        for s in &b.stmts {
            match s {
                Stmt::Let { name, value, .. } => {
                    if matches!(value, Expr::ArrayRepeat { .. }) {
                        candidates.push(name.name.clone());
                    } else {
                        other.insert(name.name.clone());
                    }
                }
                Stmt::LetTuple { names, .. } => {
                    for n in names {
                        other.insert(n.name.clone());
                    }
                }
                Stmt::For { name, body, .. } => {
                    other.insert(name.name.clone());
                    collect_binders(body, candidates, other);
                }
                Stmt::While { body, .. }
                | Stmt::Block(body)
                | Stmt::ShmWrite { body, .. } => {
                    collect_binders(body, candidates, other)
                }
                Stmt::If(i) => visit_if(i, candidates, other),
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        if let MatchArmBody::Block(bb) = &arm.body {
                            collect_binders(bb, candidates, other);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut candidates: Vec<String> = Vec::new();
    let mut other: BTreeSet<String> = BTreeSet::new();
    collect_binders(body, &mut candidates, &mut other);
    for p in params {
        other.insert(p.name.name.clone());
    }
    let mut out = BTreeSet::new();
    for c in &candidates {
        if other.contains(c) {
            continue;
        }
        // Bound twice in one body — two `[c; N]` literals under one
        // name. Not worth reasoning about; drop it.
        if candidates.iter().filter(|x| *x == c).count() != 1 {
            continue;
        }
        if block_uses_are_elementwise(body, c) {
            out.insert(c.clone());
        }
    }
    out
}

/// Which of the three walks a body gets. The returned-bindings walk
/// reads free fns and modes; the `=` walk adds locus fns; the
/// stack-array walk adds the lifecycles too.
#[derive(Clone, Copy)]
struct Walks {
    returned: bool,
    assign_moved: bool,
    stack_array: bool,
}

/// Every body a binding-fact walk reads, in declaration order, with the
/// walks it gets and its params. GH #884: module nesting flattened — a
/// fn or member is lowered whatever its brace depth, so its facts are
/// computed at that depth too.
fn binding_bodies(program: &Program) -> Vec<(Walks, &[Param], &Block)> {
    const FREE_OR_MODE: Walks =
        Walks { returned: true, assign_moved: true, stack_array: true };
    const LOCUS_FN: Walks =
        Walks { returned: false, assign_moved: true, stack_array: true };
    const LIFECYCLE: Walks =
        Walks { returned: false, assign_moved: false, stack_array: true };
    let mut out: Vec<(Walks, &[Param], &Block)> = Vec::new();
    for item in hale_syntax::ast::flat_decls(&program.items) {
        match item {
            TopDecl::Fn(f) => out.push((FREE_OR_MODE, &f.params, &f.body)),
            TopDecl::Locus(l) => {
                for member in &l.members {
                    match member {
                        LocusMember::Fn(f) => {
                            out.push((LOCUS_FN, &f.params, &f.body))
                        }
                        // A `mode` is the third shape that legitimately
                        // returns a locus (alongside a free fn) — it IS
                        // the locus-valued projection surface. A mode
                        // returning a factory-built locus with no
                        // returned-bindings answer fired the GH #383
                        // dissolve on the binding the caller now owns,
                        // handing back a reclaimed locus.
                        LocusMember::Mode(md) => {
                            out.push((FREE_OR_MODE, &[], &md.body))
                        }
                        LocusMember::Lifecycle(lc) => {
                            out.push((LIFECYCLE, &lc.params, &lc.body))
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Fill `table`'s binding rows for `program`: one row per `let`, with
/// the statement's snapshot identity as `snapshot` minted it.
///
/// Each body gets the walks [`binding_bodies`] gives it: `returned` is
/// [`ReturnedBindings::let_is_returned`] asked at the `let`'s own
/// identifier (the walk's span-keyed resolution, with its by-name
/// fallback for the uses it cannot resolve), `assign_moved` and
/// `stack_array` are the name's membership in the body's
/// [`assign_moved_names`] and [`stack_array_names`]. Every other `let`
/// of the program — in a body no walk reads — gets a row of three
/// `false`s. A `let` the snapshot did not mint (a `NONE` id) gets no row. A tuple `let` gets
/// none either: it binds several names under one id, and nothing asks.
///
/// Runs after [`resolve_owners`], over the program the snapshot minted.
/// A generic fn's monomorphs keep its sites' identities, so each reads
/// the template's rows.
pub fn resolve_binding_facts(
    program: &Program,
    snapshot: &crate::snapshot::Snapshot,
    table: &mut OwnerTable,
) {
    for (walks, params, body) in binding_bodies(program) {
        let returned = walks.returned.then(|| returned_bindings(body, snapshot));
        let moved = if walks.assign_moved {
            assign_moved_names(body)
        } else {
            BTreeSet::new()
        };
        let stack = if walks.stack_array {
            stack_array_names(params, body)
        } else {
            BTreeSet::new()
        };
        let mut lets = Vec::new();
        body_lets(body, &mut lets);
        for (name, id) in lets {
            let Some(site) = snapshot.site_id(id) else { continue };
            table.bindings.insert(
                id.0,
                (
                    site,
                    BindingFacts {
                        returned: returned
                            .as_ref()
                            .map(|rb| rb.let_is_returned(Some(site), &name.name))
                            .unwrap_or(false),
                        assign_moved: moved.contains(&name.name),
                        stack_array: stack.contains(&name.name),
                    },
                ),
            );
        }
    }
    hale_syntax::sites::for_each_site(program, &mut |kind, _, id| {
        if kind == hale_syntax::sites::SiteKind::Let {
            if let Some(site) = snapshot.site_id(id) {
                table.bindings.entry(id.0).or_insert((site, BindingFacts::default()));
            }
        }
    });
}

/// Every `let` a body declares, wherever it stands in the body, with
/// its declaring identifier. The matches are exhaustive, like
/// [`ScopeWalk`]'s: a new statement or expression form must say where
/// its `let`s are before this compiles.
fn body_lets<'e>(b: &'e Block, out: &mut Vec<(&'e Ident, NodeId)>) {
    fn if_chain<'e>(i: &'e IfStmt, out: &mut Vec<(&'e Ident, NodeId)>) {
        expr(&i.cond, out);
        body_lets(&i.then_block, out);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => body_lets(b, out),
            Some(ElseBranch::ElseIf(n)) => if_chain(n, out),
            None => {}
        }
    }
    fn match_arms<'e>(m: &'e MatchStmt, out: &mut Vec<(&'e Ident, NodeId)>) {
        expr(&m.scrutinee, out);
        for a in &m.arms {
            if let Some(g) = &a.guard {
                expr(g, out);
            }
            match &a.body {
                MatchArmBody::Block(b) => body_lets(b, out),
                MatchArmBody::Expr(x) => expr(x, out),
            }
        }
    }
    fn disposition<'e>(
        d: &'e OrDisposition,
        out: &mut Vec<(&'e Ident, NodeId)>,
    ) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                expr(e, out)
            }
            OrDisposition::Raise(_)
            | OrDisposition::Discard(_)
            | OrDisposition::Wait(_) => {}
        }
    }
    fn stmt<'e>(s: &'e Stmt, out: &mut Vec<(&'e Ident, NodeId)>) {
        match s {
            Stmt::Let { name, value, id, .. } => {
                expr(value, out);
                out.push((name, *id));
            }
            Stmt::LetTuple { value, .. } => expr(value, out),
            Stmt::Assign { target, value, .. } => {
                for seg in &target.tail {
                    match seg {
                        LValueSeg::Index(ix) => expr(ix, out),
                        LValueSeg::Field(_) => {}
                    }
                }
                expr(value, out);
            }
            Stmt::Return(value, _) => {
                if let Some(e) = value {
                    expr(e, out);
                }
            }
            Stmt::If(i) => if_chain(i, out),
            Stmt::Match(m) => match_arms(m, out),
            Stmt::For { iter, body, .. } => {
                expr(iter, out);
                body_lets(body, out);
            }
            Stmt::ShmWrite { max, body, .. } => {
                expr(max, out);
                body_lets(body, out);
            }
            Stmt::While { cond, body, .. } => {
                expr(cond, out);
                body_lets(body, out);
            }
            Stmt::Block(body) => body_lets(body, out),
            Stmt::Fail { value, .. } => expr(value, out),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    expr(a, out);
                }
                match modifier {
                    Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) => {
                        expr(e, out)
                    }
                    None => {}
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    expr(p, out);
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                expr(subject, out);
                expr(value, out);
                if let Some(d) = or_disposition {
                    disposition(d, out);
                }
            }
            Stmt::Expr(e) => expr(e, out),
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }
    fn expr<'e>(e: &'e Expr, out: &mut Vec<(&'e Ident, NodeId)>) {
        match e {
            Expr::Ident(_) | Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
            Expr::Binary { left, right, .. } => {
                expr(left, out);
                expr(right, out);
            }
            Expr::Unary { operand, .. } => expr(operand, out),
            Expr::Call { callee, args, .. } => {
                expr(callee, out);
                for a in args {
                    expr(a, out);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
                expr(receiver, out)
            }
            Expr::Index { receiver, index, .. } => {
                expr(receiver, out);
                expr(index, out);
            }
            Expr::Tuple(v, _) | Expr::Array(v, _) => {
                for x in v {
                    expr(x, out);
                }
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    expr(&i.value, out);
                }
            }
            Expr::Block(b) => body_lets(b, out),
            Expr::If(i) => if_chain(i, out),
            Expr::Match(m) => match_arms(m, out),
            Expr::Sum(x, _) | Expr::Prod(x, _) => expr(x, out),
            Expr::Approx { left, right, tolerance, .. } => {
                expr(left, out);
                expr(right, out);
                expr(tolerance, out);
            }
            Expr::Range { lo, hi, .. } => {
                expr(lo, out);
                expr(hi, out);
            }
            Expr::ArrayRepeat { val, .. } => expr(val, out),
            Expr::Or { inner, disposition: d, .. } => {
                expr(inner, out);
                disposition(d, out);
            }
        }
    }
    for s in &b.stmts {
        stmt(s, out);
    }
    if let Some(t) = b.tail.as_deref() {
        expr(t, out);
    }
}

struct Resolver<'s> {
    /// The identities the program was minted with: which declaration
    /// each use names.
    ids: &'s crate::snapshot::Snapshot,
    /// Cross-seed import renames, for resolving a path-qualified
    /// literal or callee to the merged program's mangled name.
    renames: Vec<(Vec<String>, String)>,
    /// Declared locus names (including the stdlib's mangled ones).
    loci: BTreeSet<String>,
    /// locus -> field -> kind.
    locus_fields: BTreeMap<String, BTreeMap<String, FieldKind>>,
    /// locus -> the fields its `placement { }` names.
    placed_fields: BTreeMap<String, BTreeSet<String>>,
    /// locus -> the child loci its `accept(c: C)` takes: `C`, or each
    /// locus an interface `accept(c: I)` admits. A literal of one written
    /// inside one of its member bodies is retained on the acceptor's
    /// `__children[]` spine and reclaimed by its cascade, not by the
    /// frame that wrote it.
    accepts: BTreeMap<String, BTreeSet<String>>,
    /// The child types the locus whose member body is being walked
    /// accepts, if any.
    accepting: Option<BTreeSet<String>>,
    /// The EXTENDED proven-fresh factory set: fn name -> locus.
    fresh: BTreeMap<String, String>,
    /// What each free fn does with its locus arguments (`ParamFlow`).
    flows: BTreeMap<String, ParamFlow>,
    /// GH #1566 follow-up: the bindings of the frame being walked that
    /// hold a fn value returning a locus, by name, the locus: a call
    /// through one hands back a fresh instance (only a fresh factory may
    /// be taken as a value), owned as a factory call's.
    fn_vals: BTreeMap<String, String>,
    /// GH #1569: the bindings of the frame being walked declared as a fn
    /// value that returns no locus (`f: fn(Int) -> Int`): a call through
    /// one builds nothing the table decides.
    fn_nonlocus: BTreeSet<String>,
    /// GH #1569: the free fns declared to return a fn value, by name, the
    /// declared return type.
    fn_value_rets: BTreeMap<String, TypeExpr>,
    table: OwnerTable,
    next_slot: u32,
    /// The reclaim-scope stack; the last entry is the innermost.
    scopes: Vec<ScopeId>,
    /// Binding name -> slot, within the frame being walked.
    slots: BTreeMap<String, SlotId>,
    /// The bindings this frame hands back with a bare `return x;`,
    /// resolved by declaration (GH #1140). The binding does not own
    /// those — the caller does, the carve-out lowering makes as
    /// `is_my_returned_binding` over the same resolution.
    returned: ReturnedBindings,
    /// The declaration being walked, for reporting.
    decl: String,
    /// The locus whose literal / params block is being walked, for the
    /// borrowed-handle index.
    owner_locus: String,
}

/// Why [`resolve_owners`] refused a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnershipError {
    /// A `Struct` or `Call` node carries a `NONE` id: the snapshot never
    /// minted it, so no row can be keyed by it. Every node of the merged
    /// program is minted before the pass runs; one that is not was built
    /// by a pass that ran after the mint.
    Unminted { kind: hale_syntax::sites::SiteKind, span: Span },
}

impl std::fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OwnershipError::Unminted { kind, span } => write!(
                f,
                "ownership pre-pass: the {:?} at {}..{} has no snapshot identity \
                 (a node built after the mint)",
                kind, span.start.0, span.end.0
            ),
        }
    }
}

impl std::error::Error for OwnershipError {}

/// Build the owner table for `program`, keyed by the identity the
/// snapshot `ids` minted for every locus-producing expression node, and
/// reading from it which declaration each returned name is.
///
/// `fresh_factories` is codegen's own `fresh_locus_factories` map (fn
/// name -> (locus, returned binding)); the table seeds its EXTENDED
/// set from it and adds the carrier-return fns that map misses.
///
/// The pass numbers nothing: a locus-producing `Struct` or `Call` node
/// with a `NONE` id is [`OwnershipError::Unminted`], naming the node's
/// span.
///
/// `admitted` is the ownership graph's `accepts` relation over the
/// checked programs: an owner whose `accept` names an interface accepts
/// each locus the graph admitted for it (`check::interface_admits`),
/// which the program's own declarations cannot say. A concrete `accept`
/// is read from the program either way.
pub fn resolve_owners(
    program: &Program,
    ids: &crate::snapshot::Snapshot,
    fresh_factories: &BTreeMap<String, (String, Option<String>)>,
    import_renames: &[(Vec<String>, String)],
    admitted: &BTreeMap<String, BTreeSet<String>>,
) -> Result<OwnerTable, OwnershipError> {
    use hale_syntax::sites::SiteKind;
    let mut unminted = None;
    hale_syntax::sites::for_each_site(program, &mut |kind, span, id| {
        if matches!(kind, SiteKind::StructLiteral | SiteKind::Call) && id.is_none() {
            unminted.get_or_insert(OwnershipError::Unminted { kind, span });
        }
    });
    if let Some(e) = unminted {
        return Err(e);
    }
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
    let mut accepts = collect_accepts(program, import_renames);
    for (owner, children) in admitted {
        accepts.entry(owner.clone()).or_default().extend(children.iter().cloned());
    }

    let fresh = extend_fresh_factories(
        &[program],
        ids,
        fresh_factories,
        &loci,
        import_renames,
    );

    let flows = param_flows(&program.items);
    let mut r = Resolver {
        fn_vals: BTreeMap::new(),
        fn_nonlocus: BTreeSet::new(),
        fn_value_rets: fn_value_returns(&program.items),
        flows,
        ids,
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
            bindings: BTreeMap::new(),
            passes: BTreeMap::new(),
            kept: BTreeMap::new(),
            aggregates: BTreeMap::new(),
            fn_value_calls: BTreeSet::new(),
            aggregate_kept: BTreeMap::new(),
        },
        next_slot: 0,
        scopes: Vec::new(),
        slots: BTreeMap::new(),
        decl: String::new(),
        owner_locus: String::new(),
        returned: ReturnedBindings::default(),
    };
    r.walk_decls(&program.items, "");
    Ok(r.table)
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
                                // A bare type parameter may specialize to
                                // a handle. Only locus-producing values
                                // acquire a field owner in walk_field_init;
                                // scalar expressions produce no owned row.
                                let generic = matches!(&pd.ty,
                                    Some(TypeExpr::Named { path, generic_args, .. })
                                    if generic_args.is_empty() && path.segments.len() == 1
                                        && l.generics.iter().any(|g| g.name.name == path.segments[0].name));
                                let kind = if generic {
                                    FieldKind::Holder
                                } else {
                                    param_field_kind(pd.ty.as_ref(), &pd.init, loci, ifaces, renames)
                                };
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

/// locus -> the child locus type its `accept(c: C)` declares, named by
/// the one child resolver ([`crate::handler_routing::child_locus_name`])
/// the ownership graph's `accepts` relation and the handler rows use:
/// an alias is followed, generic arguments name the monomorph, and a
/// qualified path resolves through the renames. A param that names no
/// locus makes no entry.
fn collect_accepts(
    program: &Program,
    renames: &[(Vec<String>, String)],
) -> BTreeMap<String, BTreeSet<String>> {
    use crate::handler_routing::{child_locus_name, ChildRef, DeclaredNames};
    use hale_syntax::ast::LifecycleKind;
    let declared = DeclaredNames::of(&[program]);
    let mut out = BTreeMap::new();
    for item in hale_syntax::ast::flat_decls(&program.items) {
        let TopDecl::Locus(l) = item else { continue };
        for m in &l.members {
            let LocusMember::Lifecycle(lc) = m else { continue };
            if lc.kind != LifecycleKind::Accept {
                continue;
            }
            let Some(p) = lc.params.first() else { continue };
            if let ChildRef::Locus(name) = child_locus_name(&p.ty, &declared, renames) {
                out.entry(l.name.name.clone()).or_insert_with(BTreeSet::new).insert(name);
            }
        }
    }
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
    // GH #1566 follow-up: an array or tuple of a locus type holds its
    // elements as the owner's children, as a locus-typed field holds
    // its one (an element of an interface or a perspective type has no
    // recorded teardown of its own, and stays a value)
    fn of_loci(t: &TypeExpr, is_locus: &dyn Fn(&QualifiedName) -> bool) -> bool {
        match t {
            TypeExpr::Named { path, generic_args, .. } => generic_args.is_empty() && is_locus(path),
            TypeExpr::Array { elem, .. } => of_loci(elem, is_locus),
            // GH #1569: a tuple with a locus part holds it, beside parts
            // of other types (`(W, Int)`)
            TypeExpr::Tuple(ts, _) => ts.iter().any(|t| matches!(t, TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() && is_locus(path))),
            _ => false,
        }
    }
    let is_locus = |path: &QualifiedName| names(path).iter().any(|n| loci.contains(n));
    if let Some(t) = ty {
        return match t {
            TypeExpr::Array { .. } | TypeExpr::Tuple(..) if of_loci(t, &is_locus) => FieldKind::Holder,
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
        // an array or a tuple of locus literals: typed by them, as a
        // declared one (codegen's `default_literal_type`)
        ParamInit::Value(Expr::Array(parts, _))
            if !parts.is_empty()
                && parts.iter().all(|p| matches!(p, Expr::Struct { path, .. } if is_locus(path))) =>
        {
            FieldKind::Holder
        }
        ParamInit::Value(Expr::Tuple(parts, _)) if parts.iter().any(|p| matches!(p, Expr::Struct { path, .. } if is_locus(path))) => {
            FieldKind::Holder
        }
        _ => FieldKind::Value,
    }
}

/// A multi-segment path resolved to the single mangled name the
/// merged program declares, exactly as `fresh_factories`
/// and `Cx::mangled_for_path` resolve it: bundled `std::…` paths
/// through `hale_stdlib::PATH_RENAMES`, cross-seed imports through the
/// caller's rename list. Without this, every path-qualified stdlib
/// literal (`std::io::tcp::Stream { … }`) looked like a record
/// literal to the pre-pass and got no row.
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
/// GH #1566 follow-up: what a free fn does with its locus arguments,
/// read from its body's syntax. `returns`: every value it hands back is
/// the same parameter, unchanged (`fn id(w: W) -> W { return w; }`).
/// `holds`: it hands back a locus literal (or a `let` bound to one)
/// whose field is a parameter (`return R { w: w };`), by the parameter's
/// position, the field. A parameter named anywhere else in the body is
/// in neither: the fn may keep it somewhere the caller cannot see.
#[derive(Clone, Debug, Default)]
struct ParamFlow {
    returns: Option<usize>,
    holds: BTreeMap<usize, String>,
    /// It hands back one tuple or array literal (`return (H { w: w },
    /// w);`): what each part is — a literal it builds, a parameter named
    /// nowhere but inside that literal, or something else.
    parts: Option<Vec<PartFlow>>,
    /// The parameters a literal part keeps in a field (`(H { w: w }, ..)`):
    /// (part, parameter, field), the parameter named nowhere but inside
    /// the returned aggregate.
    part_holds: Vec<(usize, usize, String)>,
    /// GH #1569: it hands back, from every `return`, what one call hands
    /// back: a call through a parameter holding a fn value (every locus
    /// part of an aggregate one returns is fresh: only a fresh factory may
    /// be a fn value), or a call of the fn named (whose parts it passes on).
    returns_call_of: Option<CallOf>,
}

/// What a fn hands back by handing back a call's result
/// (`ParamFlow::returns_call_of`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum CallOf {
    FnParam,
    Fn(String),
}

/// A part of an aggregate a fn hands back (`ParamFlow::parts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartFlow {
    /// A literal the fn builds there.
    Fresh,
    /// The parameter at this position, named nowhere but inside the
    /// returned aggregate.
    Param(usize),
    /// GH #1569: a value that is no locus whatever its type (a literal,
    /// arithmetic, a range): the part the table decides nothing for.
    Value,
    Other,
}

/// GH #1569: an expression whose value is no locus whatever its type: a
/// literal, arithmetic, a reduction, a range.
fn is_plain_value(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Literal(..)
            | Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::Sum(..)
            | Expr::Prod(..)
            | Expr::Approx { .. }
            | Expr::Range { .. }
    )
}

/// GH #1569: whether `f`, declared to return a tuple or an array holding
/// loci, builds every locus part of every aggregate it returns: one
/// returned literal whose locus parts (`is_locus` on each part's declared
/// type) are each a literal it builds there. A fn value hands its caller
/// what it returns to own, so only such a fn may be taken as one.
pub fn aggregate_builds_its_loci<'f>(
    f: &'f FnDecl,
    is_locus: &dyn Fn(&TypeExpr) -> bool,
    fn_named: &dyn Fn(&str) -> Option<&'f FnDecl>,
) -> bool {
    builds_its_loci(f, is_locus, fn_named, &mut BTreeSet::new())
}

/// `aggregate_builds_its_loci`, through the named fns whose call `f`
/// hands back (each a fresh builder by the same summary).
fn builds_its_loci<'f>(
    f: &'f FnDecl,
    is_locus: &dyn Fn(&TypeExpr) -> bool,
    fn_named: &dyn Fn(&str) -> Option<&'f FnDecl>,
    seen: &mut BTreeSet<String>,
) -> bool {
    // a fn reached again on its own chain: assumed fresh (a greatest
    // fixpoint; a cycle with no way out returns nothing)
    if !seen.insert(f.name.name.clone()) {
        return true;
    }
    let flow = param_flow(f);
    // what a fn value it was handed builds, or what a named fn builds
    let through = |seen: &mut BTreeSet<String>| match &flow.returns_call_of {
        Some(CallOf::FnParam) => true,
        Some(CallOf::Fn(m)) => fn_named(m).is_some_and(|g| builds_its_loci(g, is_locus, fn_named, seen)),
        None => false,
    };
    match &f.ret {
        Some(TypeExpr::Tuple(ts, _)) => {
            let positions: Vec<bool> = ts.iter().map(is_locus).collect();
            if !positions.iter().any(|b| *b) {
                return true;
            }
            match &flow.parts {
                Some(ps) => ps.len() == positions.len() && ps.iter().zip(&positions).all(|(p, l)| !*l || *p == PartFlow::Fresh),
                None => through(seen),
            }
        }
        Some(TypeExpr::Array { elem, .. }) if is_locus(elem) => match &flow.parts {
            Some(ps) => ps.iter().all(|p| *p == PartFlow::Fresh),
            None => through(seen),
        },
        _ => true,
    }
}

/// The free fns declared to return a fn value (`fn pick() -> fn() -> W`),
/// by name, with that return type.
fn fn_value_returns(items: &[TopDecl]) -> BTreeMap<String, TypeExpr> {
    let mut out = BTreeMap::new();
    fn go(items: &[TopDecl], out: &mut BTreeMap<String, TypeExpr>) {
        for item in items {
            match item {
                TopDecl::Fn(f) => {
                    if let Some(t @ TypeExpr::Function { .. }) = &f.ret {
                        out.insert(f.name.name.clone(), t.clone());
                    }
                }
                TopDecl::Module(m) => go(&m.items, out),
                _ => {}
            }
        }
    }
    go(items, &mut out);
    out
}

fn param_flows(items: &[TopDecl]) -> BTreeMap<String, ParamFlow> {
    let mut out = BTreeMap::new();
    fn go(items: &[TopDecl], out: &mut BTreeMap<String, ParamFlow>) {
        for item in items {
            match item {
                TopDecl::Fn(f) => {
                    let flow = param_flow(f);
                    if flow.returns.is_some() || !flow.holds.is_empty() || flow.parts.is_some() || flow.returns_call_of.is_some() {
                        out.insert(f.name.name.clone(), flow);
                    }
                }
                TopDecl::Module(m) => go(&m.items, out),
                _ => {}
            }
        }
    }
    go(items, &mut out);
    out
}

fn param_flow(f: &FnDecl) -> ParamFlow {
    // the values the body hands back
    let mut rets: Vec<&Expr> = Vec::new();
    fn stmts<'e>(b: &'e Block, out: &mut Vec<&'e Expr>) {
        for s in &b.stmts {
            match s {
                Stmt::Return(Some(e), _) => return_arms(e, out),
                Stmt::If(i) => {
                    stmts(&i.then_block, out);
                    let mut e = i.else_block.as_deref();
                    while let Some(eb) = e {
                        match eb {
                            ElseBranch::Else(b) => {
                                stmts(b, out);
                                e = None;
                            }
                            ElseBranch::ElseIf(n) => {
                                stmts(&n.then_block, out);
                                e = n.else_block.as_deref();
                            }
                        }
                    }
                }
                Stmt::Block(b) => stmts(b, out),
                _ => {}
            }
        }
    }
    stmts(&f.body, &mut rets);
    if let Some(t) = &f.body.tail {
        return_arms(t, &mut rets);
    }
    if rets.is_empty() {
        return ParamFlow::default();
    }
    // how many times the body names `n` as a value or writes through it
    let named = |n: &str| block_name_uses(&f.body, n);
    let param = |n: &str| f.params.iter().position(|p| p.name.name == n);
    let mut flow = ParamFlow::default();
    // every value the same parameter, named nowhere else
    let idents: Vec<Option<usize>> = rets
        .iter()
        .map(|r| match r {
            Expr::Ident(i) => param(&i.name),
            _ => None,
        })
        .collect();
    if let Some(Some(k)) = idents.first() {
        if idents.iter().all(|x| *x == Some(*k)) && named(&f.params[*k].name.name) == rets.len() {
            flow.returns = Some(*k);
        }
    }
    // every return one call's result: a call through a fn-typed
    // parameter, or of one fn (GH #1569)
    let call_of = |r: &Expr| -> Option<CallOf> {
        let Expr::Call { callee, .. } = r else { return None };
        let Expr::Ident(i) = callee.as_ref() else { return None };
        match f.params.iter().find(|p| p.name.name == i.name) {
            // (one whose fn type may return a locus or an aggregate)
            Some(p) => matches!(
                &p.ty,
                TypeExpr::Function { ret: Some(r), .. }
                    if matches!(r.as_ref(), TypeExpr::Named { .. } | TypeExpr::Tuple(..) | TypeExpr::Array { .. })
            )
            .then_some(CallOf::FnParam),
            None => Some(CallOf::Fn(i.name.clone())),
        }
    };
    if let Some(first) = rets.first().and_then(|r| call_of(r)) {
        if rets.iter().all(|r| call_of(r).as_ref() == Some(&first)) {
            flow.returns_call_of = Some(first);
        }
    }
    // one returned tuple or array literal: each part
    if let [agg @ (Expr::Tuple(xs, _) | Expr::Array(xs, _))] = rets.as_slice() {
        let parts: Vec<PartFlow> = xs
            .iter()
            .map(|x| match x {
                Expr::Struct { .. } => PartFlow::Fresh,
                Expr::Ident(i) => match param(&i.name) {
                    Some(k) if expr_name_uses(agg, &i.name) == named(&i.name) => PartFlow::Param(k),
                    _ => PartFlow::Other,
                },
                x if is_plain_value(x) => PartFlow::Value,
                _ => PartFlow::Other,
            })
            .collect();
        if parts.iter().any(|p| matches!(p, PartFlow::Fresh | PartFlow::Param(_))) && parts.iter().all(|p| *p != PartFlow::Other) {
            flow.parts = Some(parts);
            for (k, x) in xs.iter().enumerate() {
                let Expr::Struct { inits, .. } = x else { continue };
                for init in inits {
                    if let Expr::Ident(i) = &init.value {
                        if let Some(p) = param(&i.name) {
                            if expr_name_uses(agg, &i.name) == named(&i.name) {
                                flow.part_holds.push((k, p, init.name.name.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    // one returned literal (or a `let` bound to one) holding parameters
    if rets.len() == 1 {
        let lit: Option<&Expr> = match rets[0] {
            Expr::Struct { .. } => Some(rets[0]),
            Expr::Ident(i) => f.body.stmts.iter().find_map(|s| match s {
                Stmt::Let { name, value: v @ Expr::Struct { .. }, .. } if name.name == i.name && named(&i.name) == 1 => Some(v),
                _ => None,
            }),
            _ => None,
        };
        // a parameter kept in a field of the literal, or of a literal it
        // holds (`K { a: H { w: w }, b: H { w: w } }`): the last field
        // keeping it, as a path from the returned instance (`b.w`); the
        // parameter named nowhere but inside the literal
        fn walk(e: &Expr, path: &mut Vec<String>, out: &mut Vec<(String, String)>) {
            let Expr::Struct { inits, .. } = e else { return };
            for init in inits {
                path.push(init.name.name.clone());
                match &init.value {
                    Expr::Ident(i) => out.push((i.name.clone(), path.join("."))),
                    v @ Expr::Struct { .. } => walk(v, path, out),
                    _ => {}
                }
                path.pop();
            }
        }
        if let Some(lit) = lit {
            let mut found = Vec::new();
            walk(lit, &mut Vec::new(), &mut found);
            for (name, path) in found {
                if let Some(k) = param(&name) {
                    if expr_name_uses(lit, &name) == named(&name) {
                        flow.holds.insert(k, path);
                    }
                }
            }
        }
    }
    flow
}

/// How many times `b` names `name` as a value, or at the head of a store
/// or a rebinding.
fn block_name_uses(b: &Block, name: &str) -> usize {
    b.stmts.iter().map(|s| stmt_name_uses(s, name)).sum::<usize>()
        + b.tail.as_deref().map_or(0, |t| expr_name_uses(t, name))
}

fn if_name_uses(i: &IfStmt, name: &str) -> usize {
    expr_name_uses(&i.cond, name)
        + block_name_uses(&i.then_block, name)
        + match i.else_block.as_deref() {
            None => 0,
            Some(ElseBranch::Else(b)) => block_name_uses(b, name),
            Some(ElseBranch::ElseIf(n)) => if_name_uses(n, name),
        }
}

fn match_name_uses(m: &MatchStmt, name: &str) -> usize {
    expr_name_uses(&m.scrutinee, name)
        + m.arms
            .iter()
            .map(|a| {
                a.guard.as_ref().map_or(0, |g| expr_name_uses(g, name))
                    + match &a.body {
                        MatchArmBody::Expr(e) => expr_name_uses(e, name),
                        MatchArmBody::Block(b) => block_name_uses(b, name),
                    }
            })
            .sum::<usize>()
}

fn stmt_name_uses(s: &Stmt, name: &str) -> usize {
    match s {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Fail { value, .. } | Stmt::Expr(value) => {
            expr_name_uses(value, name)
        }
        Stmt::Assign { target, value, .. } => {
            usize::from(target.head.name == name)
                + target
                    .tail
                    .iter()
                    .map(|seg| match seg {
                        LValueSeg::Index(ix) => expr_name_uses(ix, name),
                        LValueSeg::Field(_) => 0,
                    })
                    .sum::<usize>()
                + expr_name_uses(value, name)
        }
        Stmt::If(i) => if_name_uses(i, name),
        Stmt::Match(m) => match_name_uses(m, name),
        Stmt::For { iter, body, .. } => expr_name_uses(iter, name) + block_name_uses(body, name),
        Stmt::While { cond, body, .. } => expr_name_uses(cond, name) + block_name_uses(body, name),
        Stmt::Return(v, _) => v.as_ref().map_or(0, |e| expr_name_uses(e, name)),
        Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) | Stmt::Reperspective { .. } => 0,
        Stmt::Block(b) => block_name_uses(b, name),
        Stmt::Recovery { args, .. } => args.iter().map(|a| expr_name_uses(a, name)).sum(),
        Stmt::Violate { payload, .. } => payload.as_ref().map_or(0, |e| expr_name_uses(e, name)),
        Stmt::Send { subject, value, .. } => expr_name_uses(subject, name) + expr_name_uses(value, name),
        Stmt::ShmWrite { max, body, .. } => expr_name_uses(max, name) + block_name_uses(body, name),
    }
}

fn expr_name_uses(e: &Expr, name: &str) -> usize {
    match e {
        Expr::Ident(i) => usize::from(i.name == name),
        Expr::Literal(_, _) | Expr::Path(_) | Expr::KwSelf(_) => 0,
        Expr::Binary { left, right, .. } | Expr::Range { lo: left, hi: right, .. } => {
            expr_name_uses(left, name) + expr_name_uses(right, name)
        }
        Expr::Unary { operand, .. } => expr_name_uses(operand, name),
        Expr::Call { callee, args, .. } => {
            expr_name_uses(callee, name) + args.iter().map(|a| expr_name_uses(a, name)).sum::<usize>()
        }
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => expr_name_uses(receiver, name),
        Expr::Index { receiver, index, .. } => expr_name_uses(receiver, name) + expr_name_uses(index, name),
        Expr::Tuple(xs, _) | Expr::Array(xs, _) => xs.iter().map(|x| expr_name_uses(x, name)).sum(),
        Expr::Struct { inits, .. } => inits.iter().map(|i| expr_name_uses(&i.value, name)).sum(),
        Expr::Block(b) => block_name_uses(b, name),
        Expr::If(i) => if_name_uses(i, name),
        Expr::Match(m) => match_name_uses(m, name),
        Expr::Sum(x, _) | Expr::Prod(x, _) => expr_name_uses(x, name),
        Expr::Approx { left, right, tolerance, .. } => {
            expr_name_uses(left, name) + expr_name_uses(right, name) + expr_name_uses(tolerance, name)
        }
        Expr::ArrayRepeat { val, .. } => expr_name_uses(val, name),
        Expr::Or { inner, disposition, .. } => {
            expr_name_uses(inner, name)
                + match disposition {
                    OrDisposition::Substitute(x) | OrDisposition::Fail(x, _) => expr_name_uses(x, name),
                    OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => 0,
                }
        }
    }
}

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

/// One factory row: a free fn [`fresh_factories`] found to hand back a
/// locus it built. The row answers two questions and keeps them apart:
/// what a call constructs ([`FactoryRow::products`], which the
/// self-containment rule reads) and whether the caller may own what
/// comes back ([`FactoryRow::fresh`], which the ownership pre-pass
/// reads). The first does not depend on the second: a fn whose
/// returned binding escapes still builds what it builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactoryRow {
    /// The locus the fn returns, resolved through the rename table.
    pub locus: String,
    /// What a call constructs: every literal the fn hands back, as
    /// (locus, its supplied field names, sorted and deduplicated) —
    /// through a uniquely bound returned `let` too, followed one hop
    /// whether or not the binding escapes — and, for a return that is a
    /// call to a factory with a row, that factory's products. Sorted
    /// and deduplicated.
    pub products: Vec<(String, Vec<String>)>,
    /// The ownership-eligible answer: `Some` only when the returned
    /// binding, if any, passes the escape walk and every factory the fn
    /// hands back is itself fresh.
    pub fresh: Option<Fresh>,
}

/// The half of a [`FactoryRow`] the ownership pre-pass reads: the fn
/// hands back a locus nothing else holds, so its caller's binding may
/// own it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fresh {
    /// The binding the fn hands back, when a return names one; `None`
    /// when every return is a literal or a factory call.
    pub returned_binding: Option<String>,
}

/// The fresh-factory rows, keyed by fn name.
pub type FreshFactories = BTreeMap<String, FactoryRow>;

/// GH #383 — which free fns provably return a FRESH locus?
///
/// Since v0.14 a locus-typed field may only be assigned a locus
/// LITERAL (`check_locus_field_store`), so a locus a factory returns
/// has exactly one place it can come to rest: the binding that names
/// it. That is what makes caller-scoped teardown sound — the
/// ownership ambiguity which defeated the earlier attempts on this
/// issue is now a compile error rather than a runtime guess.
///
/// A fn has a row when:
///   - its declared return type names a locus L (resolved through the
///     import-rename table, so `mat::Matrix` counts);
///   - every return is a direct `L { … }` literal, a call to a factory
///     of L that already has a row, or one single `let`-bound ident
///     whose binding is either (hence the fixpoint: helpers build on
///     other factories).
///
/// That is all [`FactoryRow::products`] asks. The row is also
/// [`FactoryRow::fresh`] when, in addition:
///   - every factory it hands back, directly or through the binding, is
///     itself fresh;
///   - the returned binding never escapes into argument position,
///     another literal, or a reassignment (receiver-position use such
///     as `m.set(i, v)` is fine — using a locus is not transferring it);
///   - no statement form this walk does not explicitly recognize
///     appears.
///
/// A returned name is the declaration the snapshot `ids` resolve it to
/// (`Snapshot::binding_of`, F.40 phase 2): an inner `let` spelling it
/// is another binding (GH #1140), and a name that resolves to no `let`
/// the walk collected — a parameter, a pattern, a `let` inside a match
/// arm — gives the fn no row.
///
/// The escape walk matches every expression form and answers by
/// identifier: a field, method or struct-init name that happens to
/// spell the binding is not a use of it (F.40 phase 1.2c; it used to
/// search the node's Debug rendering for the name). It reads every
/// binding that spells the name as the returned one — a shadow's uses
/// count as escapes, the conservative side.
///
/// Every "don't know" answers NOT fresh, preserving the old
/// program-lifetime behavior rather than risking a double dissolve.
///
/// One row per fn found to return a locus it built, keyed by its name
/// in the program walked. It has two readers (F.40 phase 1.2c):
///
///   * the ownership pre-pass, over the resolved merged program: the
///     resolved-program step projects each `fresh` row's `(locus,
///     returned_binding)` into the seed [`resolve_owners`] takes as
///     `base`, and
///     [`OwnerTable::extended_fresh_factories`] adds the carrier
///     returns this walk misses;
///   * the checker's self-containment rule
///     (`check::check_self_containing_locus`, GH #870), over each
///     program of the bundle before desugar, which reads `products`: a
///     node of its containment graph is (locus, supplied fields), so a
///     call to a factory constructs what the factory hands back.
///
/// The two still walk different program shapes; phase 2's one
/// snapshot ends that.
/// `programs` are walked together: a seed's files hand factories to
/// one another, so the fixpoint runs over all of them at once (the
/// resolved program is one; the checker passes the bundle's files).
pub fn fresh_factories(
    programs: &[&Program],
    ids: &crate::snapshot::Snapshot,
    import_renames: &[(Vec<String>, String)],
) -> FreshFactories {

    fn resolve(
        v: &[String],
        renames: &[(Vec<String>, String)],
    ) -> Option<String> {
        if v.len() == 1 {
            return Some(v[0].clone());
        }
        let refs: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
        if let Some(m) = stdlib_mangled_for_path(&refs) {
            return Some(m.to_string());
        }
        renames
            .iter()
            .find(|(p, _)| p.len() == v.len() && p.iter().zip(v).all(|(a, b)| a == b))
            .map(|(_, m)| m.clone())
    }

    fn qname(q: &QualifiedName) -> Vec<String> {
        q.segments.iter().map(|s| s.name.clone()).collect()
    }

    fn ret_locus_name(
        f: &FnDecl,
        renames: &[(Vec<String>, String)],
    ) -> Option<String> {
        match f.ret.as_ref()? {
            TypeExpr::Named { path, .. } => resolve(&qname(path), renames),
            _ => None,
        }
    }

    #[derive(Clone, Debug)]
    enum Freshness {
        /// A literal of the locus, with the product it constructs.
        Literal((String, Vec<String>)),
        CallTo(String),
        Other,
    }

    /// The (locus, supplied field names) a literal constructs.
    fn product(l: &str, inits: &[StructInit]) -> (String, Vec<String>) {
        let mut supplied: Vec<String> =
            inits.iter().map(|i| i.name.name.clone()).collect();
        supplied.sort();
        supplied.dedup();
        (l.to_string(), supplied)
    }

    /// Accepts both spellings of a qualified callee: `a::b` parses to
    /// `Path2`, and some paths normalize to `Field` before this pass.
    /// Accepting only one silently classified every cross-seed
    /// factory call as opaque.
    fn callee_name(
        callee: &Expr,
        renames: &[(Vec<String>, String)],
    ) -> Option<String> {
        fn segs(e: &Expr, out: &mut Vec<String>) -> bool {
            match e {
                Expr::Ident(i) => {
                    out.push(i.name.clone());
                    true
                }
                // THREE spellings reach here for a qualified callee:
                // `Path` (the whole-name form the parser produces for
                // `mat::zeros(...)`), plus `Path2` / `Field` for the
                // receiver-chain forms. Missing `Path` silently
                // classified every cross-seed factory call as opaque,
                // which is why the neural helpers never qualified.
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
        resolve(&v, renames)
    }

    fn expr_ok(e: &Expr, x: &str) -> bool {
        match e {
            Expr::Ident(i) => i.name != x,
            Expr::Literal(..) => true,
            Expr::Field { receiver, .. } => recv_ok(receiver, x),
            Expr::Call { callee, args, .. } => {
                let c = match callee.as_ref() {
                    Expr::Field { receiver, .. } => recv_ok(receiver, x),
                    Expr::Ident(i) => i.name != x,
                    other => expr_ok(other, x),
                };
                c && args.iter().all(|a| expr_ok(a, x))
            }
            Expr::Binary { left, right, .. } => {
                expr_ok(left, x) && expr_ok(right, x)
            }
            Expr::Unary { operand, .. } => expr_ok(operand, x),
            Expr::Index { receiver, index, .. } => {
                recv_ok(receiver, x) && expr_ok(index, x)
            }
            Expr::Path2 { receiver, .. } => recv_ok(receiver, x),
            Expr::Or { inner, disposition, .. } => {
                let d = match disposition {
                    OrDisposition::Substitute(e) => expr_ok(e, x),
                    OrDisposition::Fail(e, _) => expr_ok(e, x),
                    _ => true,
                };
                expr_ok(inner, x) && d
            }
            Expr::Struct { inits, .. } => {
                inits.iter().all(|i| expr_ok(&i.value, x))
            }
            Expr::Array(parts, _) | Expr::Tuple(parts, _) => {
                parts.iter().all(|p| expr_ok(p, x))
            }
            Expr::Block(b) => block_ok(b, x),
            Expr::If(i) => if_ok(i, x),
            Expr::Match(m) => {
                expr_ok(&m.scrutinee, x)
                    && m.arms.iter().all(|a| {
                        pattern_ok(&a.pattern, x)
                            && a.guard.as_ref().map_or(true, |g| expr_ok(g, x))
                            && match &a.body {
                                MatchArmBody::Expr(e) => expr_ok(e, x),
                                MatchArmBody::Block(b) => block_ok(b, x),
                            }
                    })
            }
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => expr_ok(inner, x),
            Expr::Approx { left, right, tolerance, .. } => {
                expr_ok(left, x) && expr_ok(right, x) && expr_ok(tolerance, x)
            }
            Expr::Range { lo, hi, .. } => expr_ok(lo, x) && expr_ok(hi, x),
            Expr::ArrayRepeat { val, .. } => expr_ok(val, x),
            // A qualified path names a declaration, never a local
            // binding, and `self` is not one either.
            Expr::Path(_) | Expr::KwSelf(_) => true,
        }
    }

    /// A match arm that binds the name shadows it, which the walk
    /// treats as it treats a second `let` of it.
    fn pattern_ok(p: &Pattern, x: &str) -> bool {
        match p {
            Pattern::Binding(i) => i.name != x,
            Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
                args.iter().all(|a| pattern_ok(a, x))
            }
            Pattern::Literal(..) | Pattern::Wildcard(_) => true,
        }
    }

    fn recv_ok(e: &Expr, x: &str) -> bool {
        match e {
            Expr::Ident(_) => true,
            Expr::Field { receiver, .. } => recv_ok(receiver, x),
            Expr::Index { receiver, index, .. } => {
                recv_ok(receiver, x) && expr_ok(index, x)
            }
            Expr::Call { callee, args, .. } => {
                let c = match callee.as_ref() {
                    Expr::Field { receiver, .. } => recv_ok(receiver, x),
                    other => expr_ok(other, x),
                };
                c && args.iter().all(|a| expr_ok(a, x))
            }
            other => expr_ok(other, x),
        }
    }

    fn block_ok(b: &Block, x: &str) -> bool {
        b.stmts.iter().all(|s| stmt_ok(s, x))
            && b.tail.as_ref().map_or(true, |t| expr_ok(t, x))
    }

    fn stmt_ok(s: &Stmt, x: &str) -> bool {
        match s {
            Stmt::Let { name, value, .. } => {
                name.name != x && expr_ok(value, x)
            }
            Stmt::Assign { target, value, .. } => {
                target.head.name != x && expr_ok(value, x)
            }
            Stmt::Expr(e) => expr_ok(e, x),
            Stmt::While { cond, body, .. } => {
                expr_ok(cond, x) && block_ok(body, x)
            }
            Stmt::For { body, iter, .. } => {
                expr_ok(iter, x) && block_ok(body, x)
            }
            Stmt::If(i) => if_ok(i, x),
            Stmt::Return(Some(Expr::Ident(i)), _) if i.name == x => true,
            Stmt::Return(Some(e), _) => expr_ok(e, x),
            Stmt::Return(None, _) => true,
            Stmt::Break(_) | Stmt::Continue(_) => true,
            Stmt::Fail { value, .. } => expr_ok(value, x),
            _ => false,
        }
    }

    fn if_ok(i: &IfStmt, x: &str) -> bool {
        expr_ok(&i.cond, x)
            && block_ok(&i.then_block, x)
            && i.else_block.as_ref().map_or(true, |e| match e.as_ref() {
                ElseBranch::Else(b) => block_ok(b, x),
                ElseBranch::ElseIf(e) => if_ok(e, x),
            })
    }

    /// Skips the defining `let x = …;` (its own name check would trip).
    fn body_ok(b: &Block, x: &str) -> bool {
        for s in &b.stmts {
            match s {
                Stmt::Let { name, value, .. } if name.name == x => {
                    if !expr_ok(value, x) {
                        return false;
                    }
                }
                other => {
                    if !stmt_ok(other, x) {
                        return false;
                    }
                }
            }
        }
        b.tail.as_ref().map_or(true, |t| match t.as_ref() {
            Expr::Ident(i) if i.name == x => true,
            e => expr_ok(e, x),
        })
    }

    fn collect(
        b: &Block,
        rets: &mut Vec<Expr>,
        lets: &mut Vec<(String, NodeId, Freshness)>,
        l: &str,
        renames: &[(Vec<String>, String)],
    ) {
        for s in &b.stmts {
            match s {
                Stmt::Return(Some(e), _) => rets.push(e.clone()),
                Stmt::Let { name, value, id, .. } => {
                    let fr = match value {
                        Expr::Struct { path, inits, .. }
                            if resolve(&qname(path), renames).as_deref()
                                == Some(l) =>
                        {
                            Freshness::Literal(product(l, inits))
                        }
                        Expr::Call { callee, .. } => {
                            match callee_name(callee, renames) {
                                Some(n) => Freshness::CallTo(n),
                                None => Freshness::Other,
                            }
                        }
                        _ => Freshness::Other,
                    };
                    lets.push((name.name.clone(), *id, fr));
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    collect(body, rets, lets, l, renames)
                }
                Stmt::If(i) => {
                    collect(&i.then_block, rets, lets, l, renames);
                    let mut cur = i.else_block.as_deref();
                    while let Some(eb) = cur {
                        match eb {
                            ElseBranch::Else(bb) => {
                                collect(bb, rets, lets, l, renames);
                                cur = None;
                            }
                            ElseBranch::ElseIf(ei) => {
                                collect(&ei.then_block, rets, lets, l, renames);
                                cur = ei.else_block.as_deref();
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(t) = &b.tail {
            rets.push((**t).clone());
        }
    }

    let mut out: FreshFactories = BTreeMap::new();
    loop {
        let mut added = false;
        // GH #884: module nesting flattened — a factory fn one
        // brace deeper is lowered and called like any other, so it
        // has to enter the same fixpoint.
        for item in programs
            .iter()
            .flat_map(|p| hale_syntax::ast::flat_decls(&p.items))
        {
            let TopDecl::Fn(f) = item else { continue };
            if out.contains_key(&f.name.name) {
                continue;
            }
            let Some(l) = ret_locus_name(f, import_renames) else {
                continue;
            };
            let mut rets = Vec::new();
            let mut lets = Vec::new();
            collect(&f.body, &mut rets, &mut lets, &l, import_renames);
            if rets.is_empty() {
                continue;
            }
            // GH #1566 follow-up: a parameter of a fn type returning the
            // locus is a factory of it — a fn may be taken as a value
            // only if it is one (the check's rule) — so a call through
            // it hands back a fresh instance, constructing nothing this
            // walk can name
            let through_value = FactoryRow { locus: l.clone(), products: Vec::new(), fresh: Some(Fresh { returned_binding: None }) };
            let value_params: BTreeSet<String> = f
                .params
                .iter()
                .filter(|p| match &p.ty {
                    TypeExpr::Function { ret: Some(r), .. } => match r.as_ref() {
                        TypeExpr::Named { path, .. } => resolve(&qname(path), import_renames).as_deref() == Some(l.as_str()),
                        _ => false,
                    },
                    _ => false,
                })
                .map(|p| p.name.name.clone())
                .collect();
            // A factory of the same locus the fixpoint already
            // accepted, by the name a callee spells.
            let known = |callee: &Expr| {
                if let Expr::Ident(i) = callee {
                    if value_params.contains(&i.name) {
                        return Some(&through_value);
                    }
                }
                callee_name(callee, import_renames)
                    .and_then(|c| out.get(&c))
                    .filter(|row| row.locus == l)
            };
            let mut fresh_name: Option<String> = None;
            // The declaration every returned name names; `None` inside
            // when one names no declaration the snapshot minted.
            let mut fresh_decl: Option<Option<hale_graph::ids::SiteId>> = None;
            let mut products: Vec<(String, Vec<String>)> = Vec::new();
            // Whether every factory handed back is itself fresh. Each
            // row it reads was inserted, `fresh` and all, before this
            // one, so a row's answer is final at insertion.
            let mut callees_fresh = true;
            let mut ok = true;
            for r in &rets {
                match r {
                    Expr::Struct { path, inits, .. }
                        if resolve(&qname(path), import_renames).as_deref()
                            == Some(l.as_str()) =>
                    {
                        products.push(product(&l, inits));
                    }
                    // GH #402 shape 2: a return arm that is itself a
                    // call to an already-qualifying factory of the
                    // same locus. `matmul`'s guard arm — `if bad {
                    // return error_matrix(); }` — disqualified the
                    // whole fn under the original literal-or-ident
                    // rule, even though that arm hands back a value
                    // as fresh as the main one. Freshness is
                    // transitive here for the same reason it is for
                    // let-bindings, and the fixpoint already decides
                    // it.
                    Expr::Call { callee, .. } if known(callee).is_some() => {
                        if let Some(row) = known(callee) {
                            products.extend(row.products.iter().cloned());
                            callees_fresh &= row.fresh.is_some();
                        }
                    }
                    // Every returned name has to be the one binding, the
                    // declaration the snapshot resolves it to (GH #1140:
                    // an inner `let` spelling it is another binding).
                    Expr::Ident(i) => {
                        let d = ids.declaration_of(i.id);
                        match fresh_decl {
                            None => {
                                fresh_decl = Some(d);
                                fresh_name = Some(i.name.clone());
                            }
                            Some(prev) if prev.is_some() && prev == d => {}
                            Some(_) => { ok = false; break; }
                        }
                    }
                    _ => { ok = false; break; }
                }
            }
            if !ok {
                continue;
            }
            // The returned binding is followed one hop for what it
            // constructs whether or not it escapes; the escape walk
            // answers the fresh half alone.
            let mut escapes = false;
            if let Some(x) = &fresh_name {
                // The returned binding: the `let` it names (a name that
                // names a parameter, a pattern or nothing is no factory).
                let Some(Some(d)) = fresh_decl else { continue };
                let Some(binding) = lets.iter().find(|(_, id, _)| ids.site_id(*id) == Some(d))
                else {
                    continue;
                };
                let bound = match &binding.2 {
                    Freshness::Literal(p) => Some(vec![p.clone()]),
                    Freshness::CallTo(c) if value_params.contains(c) => Some(Vec::new()),
                    Freshness::CallTo(c) => {
                        out.get(c).filter(|row| row.locus == l).map(|row| {
                            callees_fresh &= row.fresh.is_some();
                            row.products.clone()
                        })
                    }
                    Freshness::Other => None,
                };
                let Some(bound) = bound else { continue };
                escapes = !body_ok(&f.body, x);
                products.extend(bound);
            }
            products.sort();
            products.dedup();
            let fresh = (callees_fresh && !escapes)
                .then(|| Fresh { returned_binding: fresh_name });
            out.insert(f.name.name.clone(), FactoryRow { locus: l, products, fresh });
            added = true;
        }
        if !added {
            break;
        }
    }
    // GH #1569: the fixpoint above admits a fn once every factory it
    // hands back is admitted, so fns handing back each other's calls
    // (`cyc1` builds at its base case, else returns `cyc2(..)`, which
    // returns `cyc1(..)`) never were. Their freshness is the GREATEST
    // fixpoint: every fn whose every return is a literal of its locus,
    // a call through a fn-typed parameter, or a call of a fresh factory
    // of it is assumed fresh, and any returning a call of something else
    // is removed until none is. Every path that returns builds a fresh
    // instance; a cycle with no way out returns nothing.
    type Candidate = (String, Vec<(String, Vec<String>)>, Vec<String>);
    let mut cand: BTreeMap<String, Candidate> = BTreeMap::new();
    for item in programs.iter().flat_map(|p| hale_syntax::ast::flat_decls(&p.items)) {
        let TopDecl::Fn(f) = item else { continue };
        if out.contains_key(&f.name.name) {
            continue;
        }
        let Some(l) = ret_locus_name(f, import_renames) else { continue };
        let mut rets = Vec::new();
        let mut lets = Vec::new();
        collect(&f.body, &mut rets, &mut lets, &l, import_renames);
        if rets.is_empty() {
            continue;
        }
        let value_param = |n: &str| {
            f.params.iter().any(|p| {
                p.name.name == n
                    && matches!(&p.ty, TypeExpr::Function { ret: Some(r), .. }
                        if matches!(r.as_ref(), TypeExpr::Named { path, .. } if resolve(&qname(path), import_renames).as_deref() == Some(l.as_str())))
            })
        };
        let (mut products, mut callees, mut ok) = (Vec::new(), Vec::new(), true);
        for r in &rets {
            match r {
                Expr::Struct { path, inits, .. } if resolve(&qname(path), import_renames).as_deref() == Some(l.as_str()) => {
                    products.push(product(&l, inits));
                }
                Expr::Call { callee, .. } if matches!(callee.as_ref(), Expr::Ident(i) if value_param(&i.name)) => {}
                Expr::Call { callee, .. } => match callee_name(callee, import_renames) {
                    Some(n) => callees.push(n),
                    None => ok = false,
                },
                _ => ok = false,
            }
        }
        if ok {
            cand.insert(f.name.name.clone(), (l, products, callees));
        }
    }
    loop {
        let gone: Vec<String> = cand
            .iter()
            .filter(|(_, (l, _, cs))| {
                !cs.iter().all(|c| {
                    out.get(c).is_some_and(|row| row.locus == *l && row.fresh.is_some())
                        || cand.get(c).is_some_and(|(cl, _, _)| cl == l)
                })
            })
            .map(|(n, _)| n.clone())
            .collect();
        if gone.is_empty() {
            break;
        }
        for n in gone {
            cand.remove(&n);
        }
    }
    // what each constructs, through the calls it hands back
    let mut products: BTreeMap<String, Vec<(String, Vec<String>)>> =
        cand.iter().map(|(n, (_, p, _))| (n.clone(), p.clone())).collect();
    loop {
        let mut grew = false;
        for (n, (_, _, cs)) in &cand {
            let mut acc = products[n].clone();
            for c in cs {
                match out.get(c) {
                    Some(row) => acc.extend(row.products.iter().cloned()),
                    None => acc.extend(products.get(c).cloned().unwrap_or_default()),
                }
            }
            acc.sort();
            acc.dedup();
            if acc.len() != products[n].len() {
                products.insert(n.clone(), acc);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    for (n, (l, _, _)) in cand {
        let products = products.remove(&n).unwrap_or_default();
        out.insert(n, FactoryRow { locus: l, products, fresh: Some(Fresh { returned_binding: None }) });
    }
    out
}

/// A bundled `std::…` path's mangled name, from
/// `hale_stdlib::PATH_RENAMES`; `None` for anything not under `std`.
pub fn stdlib_mangled_for_path(segs: &[&str]) -> Option<&'static str> {
    if !matches!(segs.first(), Some(&"std")) {
        return None;
    }
    let table: &[(&[&str], &str)] = hale_stdlib::PATH_RENAMES;
    table
        .iter()
        .find(|(p, _)| *p == segs)
        .map(|(_, name)| *name)
}

/// Seed from [`fresh_factories`]'s map and add the fns whose every
/// return arm is fresh once carriers are flattened. `ids` are the
/// identities `programs` were minted with; their fns are one fixpoint,
/// as [`fresh_factories`]'s are. Lowering reads the set through the
/// owner table ([`resolve_owners`]), the checker's self-containment law
/// through [`extended_factory_rows`].
fn extend_fresh_factories(
    programs: &[&Program],
    ids: &crate::snapshot::Snapshot,
    base: &BTreeMap<String, (String, Option<String>)>,
    loci: &BTreeSet<String>,
    renames: &[(Vec<String>, String)],
) -> BTreeMap<String, String> {
    // `fresh_factories` does not check that what a fn
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
    for program in programs {
        collect_fns(&program.items, &mut fns);
    }
    let pass = passthrough_fns(&fns, ids, loci, renames);
    loop {
        let mut added = false;
        for f in &fns {
            if out.contains_key(&f.name.name) {
                continue;
            }
            let Some(l) = declared_ret_locus(f, loci, renames) else {
                continue;
            };
            // Every value the fn hands back, wherever the `return`
            // stands — inside an expression block too (GH #1140) — and
            // each name the declaration the snapshot resolves it to.
            let bindings = body_bindings(&f.body, ids);
            if bindings.returns.is_empty() {
                continue;
            }
            let mut arms: Vec<&Expr> = Vec::new();
            for r in &bindings.returns {
                return_arms(r, &mut arms);
            }
            if arms.is_empty() {
                continue;
            }
            // The binding shape: `let t = <carrier>; return t;` is the
            // same program as `return <carrier>;` and has to be the
            // same answer (spec/semantics.md "Dissolve timing rules"
            // — the `let`-named and inline spellings are one
            // program). The name is the binding it names (GH #1140: an
            // inner `let` shadowing the name is another binding), and
            // only when that binding is never re-assigned; anything else
            // leaves the fn out of the set, which is the old leak and
            // never a double free.
            let all_fresh = arms
                .iter()
                .all(|a| arm_is_fresh(a, &l, &out, &pass, renames, &bindings, 0));
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

/// The fns that hand back their own locus parameters, each with the
/// indices of the parameters it can hand back: every value the fn
/// returns, through carriers, is one of its parameters — named
/// directly, or handed back by a call to another such fn — and no `=`
/// rebinds it. A call to one is the arguments it can hand back:
/// `pass(make(1))` is the locus `make(1)` built, `let t = make(1);
/// return pass(t);` hands `t` back, and `pick(a, b, c)` is `a` or `b`.
/// A fixpoint, since one may delegate to another. A parameter's name is
/// the declaration the snapshot resolves it to, so a `let` shadowing it
/// is another binding and leaves the fn out.
pub(crate) fn passthrough_fns(
    fns: &[&FnDecl],
    ids: &crate::snapshot::Snapshot,
    loci: &BTreeSet<String>,
    renames: &[(Vec<String>, String)],
) -> BTreeMap<String, Vec<usize>> {
    let mut out: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    loop {
        let mut added = false;
        for f in fns {
            if out.contains_key(&f.name.name) || declared_ret_locus(f, loci, renames).is_none() {
                continue;
            }
            let bindings = body_bindings(&f.body, ids);
            let mut arms: Vec<&Expr> = Vec::new();
            for r in &bindings.returns {
                return_arms(r, &mut arms);
            }
            let mut leaves: Vec<&Expr> = Vec::new();
            for a in &arms {
                look_through(a, &out, renames, &mut leaves);
            }
            let mut index: Vec<usize> = Vec::new();
            let all = !leaves.is_empty()
                && leaves.iter().all(|a| {
                    let Expr::Ident(i) = a else { return false };
                    let Some(d) = ids.declaration_of(i.id) else { return false };
                    let Some(k) = f.params.iter().position(|p| ids.site_id(p.name.id) == Some(d)) else {
                        return false;
                    };
                    if bindings.assigned_names.contains(&i.name) {
                        return false;
                    }
                    index.push(k);
                    true
                });
            if all {
                index.sort_unstable();
                index.dedup();
                out.insert(f.name.name.clone(), index);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    out
}

/// What a return arm can hand back, into `out`, once a call to a fn
/// that hands back its parameters ([`passthrough_fns`]) is read as the
/// arguments it can hand back. Anything else is itself.
fn look_through<'e>(
    a: &'e Expr,
    pass: &BTreeMap<String, Vec<usize>>,
    renames: &[(Vec<String>, String)],
    out: &mut Vec<&'e Expr>,
) {
    look_through_at(a, pass, renames, out, 0)
}

fn look_through_at<'e>(
    a: &'e Expr,
    pass: &BTreeMap<String, Vec<usize>>,
    renames: &[(Vec<String>, String)],
    out: &mut Vec<&'e Expr>,
    depth: u32,
) {
    if let Expr::Call { callee, args, .. } = a {
        if depth < 8 {
            if let Some(ks) = callee_fn_name(callee, renames).and_then(|n| pass.get(&n)) {
                if ks.iter().all(|k| *k < args.len()) {
                    for k in ks {
                        look_through_at(&args[*k], pass, renames, out, depth + 1);
                    }
                    return;
                }
            }
        }
    }
    out.push(a);
}

/// Is this return arm a value the fn freshly built?
///
/// A literal of the declared locus, a call to a fn already proven to
/// build one, or a name whose binding — the `let` it names, never
/// re-assigned — holds either, including through a carrier, which is
/// the `let t = if c { make(1) } else { make2(1) }; return t;` spelling
/// of the same program. A call to a fn that hands back its parameters
/// is the arguments it can hand back (`return pass(make(1))`, `return
/// pass(t)`, `return pick(a, b, c)`): the friction sweep of 2026-10-12
/// (downstream handoff) found such a fn left out, so its caller owned
/// nothing and the frame reclaimed the value on its way out.
fn arm_is_fresh(
    a: &Expr,
    l: &str,
    known: &BTreeMap<String, String>,
    pass: &BTreeMap<String, Vec<usize>>,
    renames: &[(Vec<String>, String)],
    bindings: &BodyBindings<'_, '_>,
    depth: u32,
) -> bool {
    if depth > 4 {
        return false;
    }
    let mut leaves = Vec::new();
    look_through(a, pass, renames, &mut leaves);
    if leaves.len() != 1 || !std::ptr::eq(leaves[0], a) {
        return leaves
            .iter()
            .all(|x| arm_is_fresh(x, l, known, pass, renames, bindings, depth + 1));
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
            let Some(d) = bindings.let_at(i) else {
                return false;
            };
            if bindings.is_assigned(i, d) {
                return false;
            }
            let Some(rhs) = bindings.rhs.get(&d) else {
                return false;
            };
            let mut inner = Vec::new();
            return_arms(rhs, &mut inner);
            !inner.is_empty()
                && inner.iter().all(|x| {
                    arm_is_fresh(x, l, known, pass, renames, bindings, depth + 1)
                })
        }
        _ => false,
    }
}

/// The fresh-factory rows of the extended set (F.40 phase 3, C5): every
/// row [`fresh_factories`] finds, and one for each fn the carrier fold
/// adds ([`extend_fresh_factories`]: every arm of every value it hands
/// back, through an `if`, a `match` or a block, is fresh), whose
/// products are its arms' together. What the checker's self-containment
/// law reads (GH #870), so a carrier-return factory in a param default
/// is an edge as it is a factory to lowering. `programs` are walked
/// together, as [`fresh_factories`] walks them.
pub fn extended_factory_rows(
    programs: &[&Program],
    ids: &crate::snapshot::Snapshot,
    renames: &[(Vec<String>, String)],
) -> FreshFactories {
    let mut rows = fresh_factories(programs, ids, renames);
    let base: BTreeMap<String, (String, Option<String>)> = rows
        .iter()
        .filter_map(|(f, row)| Some((f.clone(), (row.locus.clone(), row.fresh.as_ref()?.returned_binding.clone()))))
        .collect();
    let mut loci = BTreeSet::new();
    for program in programs {
        collect_loci(&program.items, &mut loci, &mut BTreeMap::new(), &mut BTreeMap::new());
    }
    let extended = extend_fresh_factories(programs, ids, &base, &loci, renames);
    let mut fns: Vec<&FnDecl> = Vec::new();
    for program in programs {
        collect_fns(&program.items, &mut fns);
    }
    let pass = passthrough_fns(&fns, ids, &loci, renames);
    // A fold-added fn's products, once every factory its arms call has
    // a row: a fixpoint, as the fold is.
    loop {
        let mut added = false;
        for f in &fns {
            let name = &f.name.name;
            let Some(l) = extended.get(name) else { continue };
            if rows.contains_key(name) {
                continue;
            }
            let bindings = body_bindings(&f.body, ids);
            let mut arms: Vec<&Expr> = Vec::new();
            for r in &bindings.returns {
                return_arms(r, &mut arms);
            }
            let mut products = Vec::new();
            if !arms.is_empty()
                && arms.iter().all(|a| arm_products(a, l, &rows, &pass, renames, &bindings, 0, &mut products))
            {
                products.sort();
                products.dedup();
                let row = FactoryRow { locus: l.clone(), products, fresh: Some(Fresh { returned_binding: None }) };
                rows.insert(name.clone(), row);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    rows
}

/// What a fresh return arm of a fn returning `l` constructs, into `out`:
/// a literal's (locus, supplied fields), a factory call's products, a
/// binding's value's arms. `false` when a callee has no row yet.
#[allow(clippy::too_many_arguments)]
fn arm_products(
    a: &Expr,
    l: &str,
    rows: &FreshFactories,
    pass: &BTreeMap<String, Vec<usize>>,
    renames: &[(Vec<String>, String)],
    bindings: &BodyBindings<'_, '_>,
    depth: u32,
    out: &mut Vec<(String, Vec<String>)>,
) -> bool {
    if depth > 4 {
        return false;
    }
    let mut leaves = Vec::new();
    look_through(a, pass, renames, &mut leaves);
    if leaves.len() != 1 || !std::ptr::eq(leaves[0], a) {
        return leaves
            .iter()
            .all(|x| arm_products(x, l, rows, pass, renames, bindings, depth + 1, out));
    }
    match a {
        Expr::Struct { inits, .. } => {
            let mut supplied: Vec<String> = inits.iter().map(|i| i.name.name.clone()).collect();
            supplied.sort();
            supplied.dedup();
            out.push((l.to_string(), supplied));
            true
        }
        Expr::Call { callee, .. } => match callee_fn_name(callee, renames).and_then(|n| rows.get(&n)) {
            Some(row) => {
                out.extend(row.products.iter().cloned());
                true
            }
            None => false,
        },
        Expr::Ident(i) => {
            let Some(rhs) = bindings.let_at(i).and_then(|d| bindings.rhs.get(&d)) else {
                return false;
            };
            let mut inner = Vec::new();
            return_arms(rhs, &mut inner);
            !inner.is_empty() && inner.iter().all(|x| arm_products(x, l, rows, pass, renames, bindings, depth + 1, out))
        }
        _ => false,
    }
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

impl Resolver<'_> {
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

    /// The id the snapshot minted for this literal or call. The entry
    /// check in [`resolve_owners`] refused a program with an unminted
    /// one, so every node the walk keys a row by has its id.
    fn minted_id(&self, e: &Expr) -> ExprId {
        match node_id(e) {
            Some(id) if !id.is_none() => ExprId(id.0),
            _ => unreachable!("resolve_owners refuses an unminted literal or call on entry"),
        }
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

    /// The arguments a literal part of the aggregate a call hands back
    /// keeps, by position: the last part keeping it, and the field.
    fn aggregate_holds(&self, e: &Expr) -> BTreeMap<usize, (usize, String)> {
        let Expr::Call { callee, .. } = e else { return BTreeMap::new() };
        let Some(n) = callee_fn_name(callee, &self.renames) else { return BTreeMap::new() };
        let mut out = BTreeMap::new();
        if let Some(f) = self.flows.get(&n) {
            for (k, p, field) in &f.part_holds {
                out.insert(*p, (*k, field.clone()));
            }
        }
        out
    }

    /// The parts of the aggregate a call hands back (`ParamFlow::parts`).
    fn aggregate_flow(&self, e: &Expr) -> Option<Vec<PartFlow>> {
        let Expr::Call { callee, .. } = e else { return None };
        let mut n = callee_fn_name(callee, &self.renames)?;
        // (through fns that hand back another fn's call, GH #1569)
        for _ in 0..16 {
            let f = self.flows.get(&n)?;
            if let Some(parts) = &f.parts {
                return Some(parts.clone());
            }
            match &f.returns_call_of {
                Some(CallOf::Fn(m)) => n = m.clone(),
                _ => return None,
            }
        }
        None
    }

    /// GH #1569: whether every locus part of the aggregate the call `e`
    /// may hand back is fresh: a call through a fn value, or of a fn
    /// handing back one (through fns that hand back another fn's call).
    fn aggregate_all_fresh(&self, e: &Expr) -> bool {
        if self.is_fn_value_call(e) {
            return true;
        }
        let Expr::Call { callee, .. } = e else { return false };
        let Some(mut n) = callee_fn_name(callee, &self.renames) else { return false };
        for _ in 0..16 {
            let Some(f) = self.flows.get(&n) else { return false };
            if f.parts.is_some() {
                return false;
            }
            match &f.returns_call_of {
                Some(CallOf::FnParam) => return true,
                Some(CallOf::Fn(m)) => n = m.clone(),
                None => return false,
            }
        }
        false
    }

    /// The position of the argument a call hands back unchanged
    /// (`ParamFlow::returns`).
    fn passed_arg(&self, e: &Expr) -> Option<usize> {
        let Expr::Call { callee, .. } = e else { return None };
        let n = callee_fn_name(callee, &self.renames)?;
        self.flows.get(&n).and_then(|f| f.returns)
    }

    /// The fields a fresh factory call keeps arguments in, by position
    /// (`ParamFlow::holds`).
    fn kept_args(&self, e: &Expr) -> BTreeMap<usize, String> {
        let Expr::Call { callee, .. } = e else { return BTreeMap::new() };
        let Some(n) = callee_fn_name(callee, &self.renames) else { return BTreeMap::new() };
        if !self.fresh.contains_key(&n) {
            return BTreeMap::new();
        }
        self.flows.get(&n).map(|f| f.holds.clone()).unwrap_or_default()
    }

    /// GH #1569: a call through a binding holding a fn value.
    fn is_fn_value_call(&self, e: &Expr) -> bool {
        let Expr::Call { callee, .. } = e else { return false };
        let Expr::Ident(i) = callee.as_ref() else { return false };
        self.fn_vals.contains_key(&i.name) || (self.ids.declaration_of(i.id).is_some() && !self.fn_nonlocus.contains(&i.name))
    }

    fn fresh_call_locus(&self, e: &Expr) -> Option<String> {
        let Expr::Call { callee, .. } = e else { return None };
        if let Expr::Ident(i) = callee.as_ref() {
            if let Some(l) = self.fn_vals.get(&i.name) {
                return Some(l.clone());
            }
            // GH #1569: a call through any other local binding — a loop
            // variable, a tuple pattern's name, a `let` of an `if` or a
            // method's call — is a call through a fn value, and only a
            // fresh factory may be one: when it returns a locus, the
            // instance is fresh, owned as a factory call's. The row
            // names the binding; one declared to return no locus has none.
            if self.ids.declaration_of(i.id).is_some() && !self.fn_nonlocus.contains(&i.name) {
                return Some(i.name.clone());
            }
        }
        let n = callee_fn_name(callee, &self.renames)?;
        if let Some(l) = self.fresh.get(&n) {
            return Some(l.clone());
        }
        // GH #1569: one handing back what a fn value it was handed builds
        self.aggregate_all_fresh(e).then(|| n.clone())
    }

    /// Is this expression one the table hands an owner to?
    fn is_locus_producing(&self, e: &Expr) -> bool {
        match e {
            Expr::Struct { path, .. } => {
                self.locus_of_literal(path).is_some()
            }
            Expr::Call { args, .. } => {
                self.fresh_call_locus(e).is_some()
                    || self.passed_arg(e).is_some_and(|k| args.get(k).is_some_and(|a| self.is_locus_producing(a)))
                    || self.aggregate_flow(e).is_some_and(|ps| ps.iter().any(|p| *p == PartFlow::Fresh))
            }
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
            Expr::Array(parts, _) => {
                !parts.is_empty()
                    && parts.iter().all(|p| self.is_locus_producing(p))
            }
            // GH #1569: a tuple's parts of other types are plain values
            // (`(W { }, 5)`)
            Expr::Tuple(parts, _) => {
                parts.iter().any(|p| self.is_locus_producing(p))
                    && parts.iter().all(|p| self.is_locus_producing(p) || is_plain_value(p))
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

    fn assign(&mut self, e: &Expr, d: Decision, position: &'static str) {
        match e {
            Expr::Struct { path, inits, span, id: _ } => {
                let lname = self.locus_of_literal(path);
                let span = *span;
                match lname {
                    Some(lname) => {
                        let id = self.minted_id(e);
                        // An acceptor's own body: the child is
                        // appended to `__children[]` and reclaimed by
                        // the acceptor's cascade, whatever the
                        // position says (GH #253's spine).
                        let owner = if self.accepting.as_ref().is_some_and(|a| a.contains(lname.as_str())) {
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
                        self.walk_literal_inits(id, &lname, inits);
                    }
                    None => {
                        // A record / type literal: not a locus, but
                        // its initialisers are ordinary expressions.
                        for i in inits.iter() {
                            self.assign(
                                &i.value,
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
                    let id = self.minted_id(e);
                    if self.aggregate_all_fresh(e) && !matches!(d, Decision::Caller) {
                        self.table.fn_value_calls.insert(id);
                    }
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
                self.walk_callee(callee);
                // GH #1566 follow-up: an argument the callee hands back
                // is the call's value, owned where the call's is; one a
                // fresh factory keeps in the instance it returns is that
                // instance's field's child — when the instance outlives
                // this frame's temporaries (a field, a placement, a
                // binding); a returned call keeps today's temporary.
                let passed = self.passed_arg(e);
                // GH #1569: a call handing back a tuple or an array of
                // loci: its owner owns the parts it builds, and, when the
                // aggregate outlives this frame's temporaries (a field, a
                // placement), the parts it was handed as literals too
                let aggregate = self.aggregate_flow(e);
                let holds_parts = matches!(d, Decision::Field { .. } | Decision::Placement(_));
                if let Some(parts) = &aggregate {
                    let call_id = self.minted_id(e);
                    let mask: Vec<bool> = parts
                        .iter()
                        .map(|p| match p {
                            PartFlow::Fresh => true,
                            PartFlow::Param(k) => holds_parts && args.get(*k).is_some_and(|a| self.is_locus_producing(a)),
                            PartFlow::Value | PartFlow::Other => false,
                        })
                        .collect();
                    if !matches!(d, Decision::Caller) && mask.iter().any(|b| *b) {
                        let owner = self.owner_from(&d);
                        self.record(call_id, owner, Produced::FactoryCall, "(aggregate)".to_string(), position, e.span());
                        self.table.aggregates.insert(call_id, mask);
                    }
                }
                let kept = match &d {
                    Decision::Field { .. } | Decision::Placement(_) | Decision::Binding(_) => self.kept_args(e),
                    Decision::FrameTemp | Decision::Caller => BTreeMap::new(),
                };
                let call_id = self.minted_id(e);
                for (k, a) in args.iter().enumerate() {
                    let part_of = aggregate.as_ref().is_some_and(|ps| ps.iter().any(|p| *p == PartFlow::Param(k)));
                    let held_by_part = if aggregate.is_some() { self.aggregate_holds(e).remove(&k) } else { None };
                    if part_of && holds_parts && self.is_locus_producing(a) {
                        self.assign(a, d.clone(), "argument handed back in an aggregate");
                    } else if let Some((part, field)) = held_by_part.filter(|_| holds_parts && self.is_locus_producing(a)) {
                        self.table.aggregate_kept.entry(call_id).or_default().push((part, field));
                        self.assign(a, d.clone(), "argument kept by a part of the aggregate");
                    } else if passed == Some(k) && self.is_locus_producing(a) {
                        self.table.passes.insert(call_id, k);
                        // (a binding names the call, as it names a
                        // carrier's join: the argument is then the
                        // frame's, as an arm is)
                        self.assign(a, d.through_delegate(), "argument handed back");
                    } else if let Some(field) = kept.get(&k).filter(|_| self.is_locus_producing(a)) {
                        self.table.kept.entry(call_id).or_default().push(field.clone());
                        self.assign(a, Decision::Field { owner: call_id, field: field.clone() }, "argument kept by the result");
                    } else {
                        self.assign(a, Decision::FrameTemp, "argument");
                    }
                }
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
                    &m.scrutinee,
                    Decision::FrameTemp,
                    "scrutinee",
                );
                for a in m.arms.iter() {
                    if let Some(g) = &a.guard {
                        self.assign(g, Decision::FrameTemp, "arm guard");
                    }
                    match &a.body {
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
                for p in parts.iter() {
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
        i: &IfStmt,
        d: Decision,
        position: &'static str,
    ) {
        self.assign(&i.cond, Decision::FrameTemp, "condition");
        self.walk_block(&i.then_block, Some((d.clone(), position)));
        match i.else_block.as_deref() {
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
    fn walk_callee(&mut self, callee: &Expr) {
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
        inits: &[StructInit],
    ) {
        let saved = std::mem::replace(&mut self.owner_locus, locus.to_string());
        for init in inits.iter() {
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
                &init.value,
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
        value: &Expr,
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
        b: &Block,
        tail: Option<(Decision, &'static str)>,
    ) {
        for s in b.stmts.iter() {
            self.walk_stmt(s);
        }
        if let Some(t) = &b.tail {
            match tail {
                Some((d, pos)) => self.assign(t, d, pos),
                None => self.assign(t, Decision::FrameTemp, "block tail"),
            }
        }
    }

    fn walk_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value, id, ty, .. } => {
                let d = if self.returned.let_is_returned(self.ids.site_id(*id), &name.name) {
                    Decision::Caller
                } else {
                    Decision::Binding(self.slot_for(&name.name))
                };
                self.assign(value, d, "`let` RHS");
                // a binding of a fn value returning a locus (`let f: fn(W)
                // -> R = mk;`, or of a fresh factory's name)
                let held = ty.as_ref().and_then(|t| self.fn_value_locus(t)).or_else(|| self.fn_value_held(value));
                match held {
                    Some(l) => {
                        self.fn_vals.insert(name.name.clone(), l);
                    }
                    None => {
                        self.fn_vals.remove(&name.name);
                    }
                }
                if ty.as_ref().is_some_and(|t| matches!(t, TypeExpr::Function { .. }) && self.fn_value_locus(t).is_none()) {
                    self.fn_nonlocus.insert(name.name.clone());
                } else {
                    self.fn_nonlocus.remove(&name.name);
                }
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
            Stmt::Assign { target, value, id, .. } => {
                let bare = target.tail.is_empty();
                let head = target.head.name.clone();
                let head_is_returned = bare
                    && self
                        .returned
                        .assign_is_returned(self.ids.declaration_of(*id), &target.head.name);
                for seg in target.tail.iter() {
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
                    let d = if head_is_returned {
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
                for a in args.iter() {
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

    fn walk_if_stmt(&mut self, i: &IfStmt) {
        self.assign(&i.cond, Decision::FrameTemp, "condition");
        self.walk_block(&i.then_block, None);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.walk_block(b, None),
            Some(ElseBranch::ElseIf(n)) => self.walk_if_stmt(n),
            None => {}
        }
    }

    fn walk_match_stmt(&mut self, m: &MatchStmt) {
        self.assign(&m.scrutinee, Decision::FrameTemp, "scrutinee");
        for a in m.arms.iter() {
            if let Some(g) = &a.guard {
                self.assign(g, Decision::FrameTemp, "arm guard");
            }
            match &a.body {
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

    /// The same, for a body: the bindings it hands back with a bare
    /// `return x;` are resolved first, so a `let` can see them.
    fn open_body_frame(&mut self, decl: &str, body: &Block) -> SavedFrame {
        let saved = self.open_frame(decl);
        self.returned = returned_bindings(body, self.ids);
        saved
    }

    fn close_frame(&mut self, saved: SavedFrame) {
        self.pop_scope();
        self.decl = saved.decl;
        self.slots = saved.slots;
        self.returned = saved.returned;
    }

    fn walk_decls(&mut self, items: &[TopDecl], prefix: &str) {
        for item in items.iter() {
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
                        &c.value,
                        Decision::FrameTemp,
                        "const initialiser",
                    );
                    self.close_frame(saved);
                }
                TopDecl::Type(t) => {
                    let name = t.name.name.clone();
                    if let hale_syntax::ast::TypeDeclBody::Struct(fields) =
                        &t.body
                    {
                        for f in fields.iter() {
                            let fname = f.name.name.clone();
                            if let Some(d) = &f.default {
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
                    for m in i.methods.iter() {
                        let decl =
                            format!("{}{}.{}", prefix, iname, m.name.name);
                        self.walk_param_defaults(&m.params, &decl);
                    }
                }
                TopDecl::Topic(_)
                | TopDecl::RingLayout(_)
                | TopDecl::Target(_)
                | TopDecl::Group(_)
                | TopDecl::Role(_)
                | TopDecl::Claims(_)
                | TopDecl::Constitution(_)
                | TopDecl::Unit(_)
                | TopDecl::Api(_) => {}
            }
        }
    }

    fn walk_module(&mut self, m: &ModuleDecl, prefix: &str) {
        let inner = format!("{}{}::", prefix, m.name.name);
        self.walk_decls(&m.items, &inner);
    }

    fn walk_perspective(
        &mut self,
        p: &hale_syntax::ast::PerspectiveDecl,
        prefix: &str,
    ) {
        use hale_syntax::ast::PerspectiveMember;
        let pname = format!("{}{}", prefix, p.name.name);
        for m in p.members.iter() {
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

    fn walk_fn(&mut self, f: &FnDecl, prefix: &str, what: &str) {
        let decl = format!("{} {}{}", what, prefix, f.name.name);
        let saved = self.open_body_frame(&decl, &f.body);
        let saved_vals = std::mem::take(&mut self.fn_vals);
        let saved_nonlocus = std::mem::take(&mut self.fn_nonlocus);
        for p in f.params.iter() {
            if let Some(l) = self.fn_value_locus(&p.ty) {
                self.fn_vals.insert(p.name.name.clone(), l);
            } else if matches!(p.ty, TypeExpr::Function { .. }) {
                self.fn_nonlocus.insert(p.name.name.clone());
            }
        }
        for p in f.params.iter() {
            if let Some(d) = &p.default {
                self.assign(d, Decision::FrameTemp, "param default");
            }
        }
        self.walk_block(
            &f.body,
            Some((Decision::Caller, "fn body tail")),
        );
        self.fn_vals = saved_vals;
        self.fn_nonlocus = saved_nonlocus;
        self.close_frame(saved);
    }

    /// The locus a fn value `e` returns when called, for a value spelled
    /// as a binding holding one, a fresh factory's name, or (GH #1569) a
    /// call of a fn declared to return a fn value returning a locus
    /// (`fn pick() -> fn() -> W`): whatever that call hands back was
    /// named as a value somewhere, which the check allows of a fresh
    /// factory only.
    fn fn_value_held(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Ident(i) if self.fn_vals.contains_key(&i.name) => self.fn_vals.get(&i.name).cloned(),
            Expr::Ident(i) => self.fresh.get(&i.name).cloned(),
            Expr::Call { callee, .. } => {
                let n = callee_fn_name(callee, &self.renames)?;
                self.fn_value_rets.get(&n).and_then(|t| self.fn_value_locus(t))
            }
            _ => None,
        }
    }

    /// The locus a fn type returns (`fn(W) -> Routes`), when it names one.
    fn fn_value_locus(&self, t: &TypeExpr) -> Option<String> {
        let TypeExpr::Function { ret: Some(r), .. } = t else { return None };
        let TypeExpr::Named { path, .. } = r.as_ref() else { return None };
        self.locus_of_literal(path)
    }

    fn walk_param_defaults(&mut self, params: &[Param], decl: &str) {
        let saved = self.open_frame(decl);
        for p in params.iter() {
            if let Some(d) = &p.default {
                self.assign(d, Decision::FrameTemp, "param default");
            }
        }
        self.close_frame(saved);
    }

    fn walk_params_block(
        &mut self,
        pb: &hale_syntax::ast::ParamsBlock,
        owner: &str,
    ) {
        // `accept` retains children written in a METHOD body; a
        // params default runs with `params_init_self`, not
        // `current_self`, and takes the ordinary field path.
        let saved_accepting = self.accepting.take();
        let saved_owner =
            std::mem::replace(&mut self.owner_locus, owner.to_string());
        for pd in pb.params.iter() {
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
            let ParamInit::Value(v) = &pd.init else { continue };
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

    fn walk_locus(&mut self, l: &LocusDecl, prefix: &str) {
        use hale_syntax::ast::{
            BusMember, ClosureClause, EpochSpec, KeyFilter, LifecycleKind,
            ModeKind, TypeDeclBody,
        };
        let lname = l.name.name.clone();
        let qualified = format!("{}{}", prefix, lname);
        if let Some(form) = &l.form {
            let saved = self.open_frame(&format!("{} @form", qualified));
            for a in form.args.iter() {
                self.assign(
                    &a.value,
                    Decision::FrameTemp,
                    "@form argument",
                );
            }
            self.close_frame(saved);
        }
        let saved_accepting = std::mem::replace(
            &mut self.accepting,
            self.accepts.get(&lname).cloned(),
        );
        for m in l.members.iter() {
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
                    for p in lc.params.iter() {
                        if let Some(d) = &p.default {
                            self.assign(
                                d,
                                Decision::FrameTemp,
                                "param default",
                            );
                        }
                    }
                    self.walk_block(
                        &lc.body,
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
                        &md.body,
                        Some((Decision::Caller, "body tail")),
                    );
                    self.close_frame(saved);
                }
                LocusMember::Failure(fd) => {
                    let saved = self
                        .open_frame(&format!("{}.on_failure", qualified));
                    self.walk_block(&fd.body, None);
                    self.close_frame(saved);
                }
                LocusMember::Closure(cd) => {
                    let saved =
                        self.open_frame(&format!("{}.closure", qualified));
                    if let Some(a) = &cd.assertion {
                        self.assign(
                            &a.left,
                            Decision::FrameTemp,
                            "closure assertion",
                        );
                        self.assign(
                            &a.right,
                            Decision::FrameTemp,
                            "closure assertion",
                        );
                        self.assign(
                            &a.tolerance,
                            Decision::FrameTemp,
                            "closure tolerance",
                        );
                    }
                    for c in cd.clauses.iter() {
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
                        &bc.cond,
                        Decision::FrameTemp,
                        "birth-check condition",
                    );
                    if let Some(p) = &bc.payload {
                        self.assign(
                            p,
                            Decision::FrameTemp,
                            "birth-check payload",
                        );
                    }
                    self.close_frame(saved);
                }
                LocusMember::Bus(bb) => {
                    for bm in bb.members.iter() {
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
                        &c.value,
                        Decision::FrameTemp,
                        "const initialiser",
                    );
                    self.close_frame(saved);
                }
                LocusMember::Type(t) => {
                    let tname = t.name.name.clone();
                    if let TypeDeclBody::Struct(fields) = &t.body {
                        for f in fields.iter() {
                            let fname = f.name.name.clone();
                            if let Some(d) = &f.default {
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
        self.accepting = saved_accepting;
    }

    /// `bindings { }` transports and codecs are program-lifetime
    /// instances the runtime dissolves at main exit — `Placement`, per
    /// Riley's answer to F.39's third open question.
    fn walk_bindings(
        &mut self,
        bb: &hale_syntax::ast::BindingsBlock,
        owner: &str,
    ) {
        use hale_syntax::ast::TransportSpec;
        for entry in bb.entries.iter() {
            let topic = entry.topic.name.clone();
            if let TransportSpec::Adapter { locus, inits, .. } =
                &entry.transport
            {
                let lname = locus.name.clone();
                let saved = self.open_frame(&format!(
                    "{}.bindings {} adapter",
                    owner, topic
                ));
                self.walk_binding_inits(&lname, inits, &topic);
                self.close_frame(saved);
            }
            if let Some(codec) = &entry.codec {
                let lname = codec.locus.name.clone();
                let saved = self.open_frame(&format!(
                    "{}.bindings {} codec",
                    owner, topic
                ));
                self.walk_binding_inits(&lname, &codec.inits, &topic);
                self.close_frame(saved);
            }
        }
    }

    fn walk_binding_inits(
        &mut self,
        locus: &str,
        inits: &[StructInit],
        topic: &str,
    ) {
        for init in inits.iter() {
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
                    &init.value,
                    Decision::Placement(topic.to_string()),
                    "bindings transport field",
                );
            } else {
                self.assign(
                    &init.value,
                    Decision::FrameTemp,
                    "bindings transport field",
                );
            }
        }
    }
}
