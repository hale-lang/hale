//! GH #921 — locus ownership is resolved before lowering.
//!
//! This module lives in the frontend (`hale-types`) because the
//! `ownership` family's home is the frontend: F.40 phase 1 moves the
//! pre-pass here with its algorithm unchanged, together with the
//! fresh-factory seed it is handed ([`compute_fresh_locus_factories`],
//! whose one producer this is). The frontend's resolved-program step
//! (`crate::resolved::resolve_program`) runs the pass over the merged
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
//! [`OwnerTable::binding_facts`], keyed by the `let`'s snapshot
//! identity (F.40 phase 1.2b), and the `let` lowering reads its own
//! row. A `let` in a body no walk reads has a row that says `false`
//! three times, and so does a `let` with no row at all. A generic fn's
//! monomorphs keep the template's identities, so a monomorph's `let`
//! reads the template's answer.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    AssignOp, Block, ElseBranch, Expr, FnDecl, Ident, IfStmt, LValueSeg,
    LocusDecl, LocusMember, MatchArmBody, MatchStmt, ModuleDecl, NodeId, OrDisposition, Param, ParamInit, Pattern, Program,
    QualifiedName, RecoveryModifier, Stmt, StructInit, TopDecl, TypeExpr,
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
    /// One row per `let`, keyed by the statement's snapshot index
    /// (see [`resolve_binding_facts`]).
    bindings: BTreeMap<u32, BindingFacts>,
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
    /// contain; a consumer reads that as three `false`s.
    pub fn binding_facts(&self, id: NodeId) -> Option<&BindingFacts> {
        if id.is_none() {
            return None;
        }
        self.bindings.get(&id.0)
    }

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
    returned: ReturnedBindings,
}

// ===================================================================
// Bindings, not names (GH #1140)
// ===================================================================

/// A binding's identity: its declaring identifier's span, in the
/// process-wide coordinates multi-file builds shift spans into. Two
/// `let`s that spell one name are two bindings — an inner shadow is an
/// ordinary fresh value its frame reclaims, whatever the fn returns.
pub(crate) type BindingKey = (u32, u32);

fn binding_key(i: &Ident) -> Option<BindingKey> {
    (i.span.end.0 > i.span.start.0).then_some((i.span.start.0, i.span.end.0))
}

/// What a name spelled at one point of a body resolves to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Resolved {
    /// A `let` (or tuple `let`) of the body, by its key.
    Let(BindingKey),
    /// A binding that is no keyed `let`: a `for` variable, a match
    /// pattern, a `shm_write` binding, a declaration with no span.
    Other,
    /// Nothing in the body's scopes: a param, a const, a global.
    Outside,
}

/// Which bindings a body hands back, each resolved to its declaration
/// (see [`body_bindings`]). Owned, so a walk that mutates the body can
/// hold it.
///
/// Every uncertainty answers "handed back", which suppresses a reclaim
/// (the old leak) and never adds one: a binding the walk could not key,
/// a use it could not resolve, a key two declarations share all answer
/// by name against every name a return spelled — the old rule, whole.
#[derive(Default, Debug, Clone)]
pub struct ReturnedBindings {
    /// The keyed `let`s handed back, directly or through the value of
    /// another handed-back binding.
    decls: BTreeSet<BindingKey>,
    /// Every name a return (or a counted block tail) spells as a value
    /// arm, however it resolved: the by-name answer, for what the walk
    /// cannot resolve.
    all_names: BTreeSet<String>,
    /// The names among those that resolve to no keyed `let` — a `for`
    /// variable, a pattern, a param. A keyed `let` spelled the same is
    /// counted too: such a binding can hold the `let`'s value (an
    /// element of an array the `let` went into), which no name says.
    nonlet_names: BTreeSet<String>,
    /// `let`s declared under a key of their own.
    keyed: BTreeSet<BindingKey>,
    /// What each use (every identifier the walk met) names, by the
    /// use's own span.
    uses: BTreeMap<BindingKey, Resolved>,
}

impl ReturnedBindings {
    fn key_is_returned(&self, k: BindingKey, name: &str) -> bool {
        self.decls.contains(&k) || self.nonlet_names.contains(name)
    }

    /// Whether the `let` declaring `name` is one the body hands back.
    pub fn let_is_returned(&self, name: &Ident) -> bool {
        match binding_key(name) {
            Some(k) if self.keyed.contains(&k) => {
                self.key_is_returned(k, &name.name)
            }
            _ => self.all_names.contains(&name.name),
        }
    }

    /// Whether a bare `=` writing `head` writes a binding the body
    /// hands back.
    pub fn assign_is_returned(&self, head: &Ident) -> bool {
        match binding_key(head).and_then(|u| self.uses.get(&u)) {
            Some(Resolved::Let(k)) if self.keyed.contains(k) => {
                self.key_is_returned(*k, &head.name)
            }
            _ => self.all_names.contains(&head.name),
        }
    }
}

/// A body's bindings as the fresh-factory walk reads them.
#[derive(Default)]
struct BodyBindings<'e> {
    returned: ReturnedBindings,
    /// Each keyed `let`'s right-hand side.
    lets: BTreeMap<BindingKey, &'e Expr>,
    /// Every value the body hands back: each `return <e>;`, wherever it
    /// stands, and each counted block tail.
    returns: Vec<&'e Expr>,
    /// Keyed `let`s some `=` writes, and each such write's value.
    assigned: BTreeSet<BindingKey>,
    writes: Vec<(BindingKey, &'e Expr)>,
    /// Names written by an `=` that resolves to no keyed `let`.
    assigned_names: BTreeSet<String>,
    /// How often each key was declared; a key declared twice keys
    /// nothing.
    declared: BTreeMap<BindingKey, u32>,
}

impl<'e> BodyBindings<'e> {
    /// The keyed `let` a name spelled at `use_site` resolves to.
    fn let_at(&self, use_site: &Ident) -> Option<BindingKey> {
        match binding_key(use_site).and_then(|u| self.returned.uses.get(&u)) {
            Some(Resolved::Let(k)) if self.returned.keyed.contains(k) => {
                Some(*k)
            }
            _ => None,
        }
    }

    /// Whether some bare `=` may write the binding `use_site` names.
    fn is_assigned(&self, use_site: &Ident, k: BindingKey) -> bool {
        self.assigned.contains(&k)
            || self.assigned_names.contains(&use_site.name)
    }

    /// Count `e`'s value arms as handed back: a name among them is the
    /// binding it resolves to.
    fn hand_back(&mut self, e: &'e Expr) {
        let mut arms = Vec::new();
        return_arms(e, &mut arms);
        for a in arms {
            if let Expr::Ident(i) = a {
                self.returned.all_names.insert(i.name.clone());
                match self.let_at(i) {
                    Some(k) => {
                        self.returned.decls.insert(k);
                    }
                    None => {
                        self.returned.nonlet_names.insert(i.name.clone());
                    }
                }
            }
        }
    }
}

/// The GH #1140 walk: every statement and expression of a body, blocks
/// as scopes, in order, so a name resolves to the declaration in scope
/// where it is spelled. The matches are exhaustive on purpose: a new
/// statement or expression form must say how it binds and what it
/// hands back before this compiles.
struct ScopeWalk<'e> {
    frames: Vec<Vec<(&'e str, Resolved)>>,
    out: BodyBindings<'e>,
}

/// Resolve a body's bindings.
fn body_bindings(b: &Block) -> BodyBindings<'_> {
    let mut w = ScopeWalk { frames: Vec::new(), out: BodyBindings::default() };
    w.block(b, true);
    let mut out = w.out;
    let twice: Vec<BindingKey> = out
        .declared
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(k, _)| *k)
        .collect();
    for k in twice {
        out.returned.keyed.remove(&k);
        out.lets.remove(&k);
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
            .lets
            .iter()
            .filter(|(k, _)| out.returned.decls.contains(k))
            .map(|(_, e)| *e)
            .chain(
                out.writes
                    .iter()
                    .filter(|(k, _)| out.returned.decls.contains(k))
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

/// The bindings a body hands back (see [`ReturnedBindings`]).
///
/// GH #383: distinct from `compute_fresh_locus_factories` and needed
/// separately: a fn that does NOT qualify as a clean factory can still
/// return a locus it bound from one. `nn::forward` is the case that
/// proved it — it binds several factory results, returns one, and
/// fails the freshness walk; the caller-scoped dissolve fired on the
/// binding it hands back and the caller read zeros. Conservative by
/// construction: a `true` merely suppresses a dissolve, which is the
/// old leak — never a double-free.
pub fn returned_bindings(b: &Block) -> ReturnedBindings {
    body_bindings(b).returned
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

/// Fill `table`'s binding rows for `program`: one row per `let`,
/// keyed by the statement's snapshot index.
///
/// Each body gets the walks [`binding_bodies`] gives it: `returned` is
/// [`ReturnedBindings::let_is_returned`] asked at the `let`'s own
/// identifier (the walk's span-keyed resolution, with its by-name
/// fallback for the uses it cannot resolve), `assign_moved` and
/// `stack_array` are the name's membership in the body's
/// [`assign_moved_names`] and [`stack_array_names`]. Every other `let`
/// of the program — in a body no walk reads — gets a row of three
/// `false`s. A `let` with a `NONE` id gets no row. A tuple `let` gets
/// none either: it binds several names under one id, and nothing asks.
///
/// Runs after [`resolve_owners`], over the program the snapshot minted.
/// A generic fn's monomorphs keep its sites' identities, so each reads
/// the template's rows.
pub fn resolve_binding_facts(program: &Program, table: &mut OwnerTable) {
    for (walks, params, body) in binding_bodies(program) {
        let returned = walks.returned.then(|| returned_bindings(body));
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
            if id.is_none() {
                continue;
            }
            table.bindings.insert(
                id.0,
                BindingFacts {
                    returned: returned
                        .as_ref()
                        .map(|rb| rb.let_is_returned(name))
                        .unwrap_or(false),
                    assign_moved: moved.contains(&name.name),
                    stack_array: stack.contains(&name.name),
                },
            );
        }
    }
    hale_syntax::sites::for_each_site(program, &mut |kind, _, id| {
        if kind == hale_syntax::sites::SiteKind::Let && !id.is_none() {
            table.bindings.entry(id.0).or_default();
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

impl<'e> ScopeWalk<'e> {
    fn lookup(&self, name: &str) -> Resolved {
        for frame in self.frames.iter().rev() {
            for (n, r) in frame.iter().rev() {
                if *n == name {
                    return *r;
                }
            }
        }
        Resolved::Outside
    }

    fn declare(&mut self, name: &'e Ident, r: Resolved) {
        if let Some(frame) = self.frames.last_mut() {
            frame.push((name.name.as_str(), r));
        }
    }

    fn declare_let(&mut self, name: &'e Ident, rhs: Option<&'e Expr>) {
        match binding_key(name) {
            Some(k) => {
                *self.out.declared.entry(k).or_insert(0) += 1;
                self.out.returned.keyed.insert(k);
                if let Some(e) = rhs {
                    self.out.lets.insert(k, e);
                }
                self.declare(name, Resolved::Let(k));
            }
            None => self.declare(name, Resolved::Other),
        }
    }

    fn note_use(&mut self, i: &Ident) -> Resolved {
        let r = self.lookup(&i.name);
        if let Some(u) = binding_key(i) {
            // two uses one span spells (a desugaring's copy) that resolve
            // apart resolve to neither: the answer falls back by name
            let slot = self.out.returned.uses.entry(u).or_insert(r);
            if *slot != r {
                *slot = Resolved::Other;
            }
        }
        r
    }

    /// A block as a scope. A statement-level block's tail counts as
    /// handed back (`counted`), the shape the name walk always read; an
    /// expression block's tail is its expression's value.
    fn block(&mut self, b: &'e Block, counted: bool) {
        self.frames.push(Vec::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = b.tail.as_deref() {
            self.expr(t);
            if counted {
                self.out.returns.push(t);
            }
        }
        self.frames.pop();
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
            self.frames.push(Vec::new());
            self.pattern(&a.pattern);
            if let Some(g) = &a.guard {
                self.expr(g);
            }
            match &a.body {
                MatchArmBody::Block(b) => self.block(b, counted),
                MatchArmBody::Expr(x) => self.expr(x),
            }
            self.frames.pop();
        }
    }

    fn pattern(&mut self, p: &'e Pattern) {
        match p {
            Pattern::Binding(i) => self.declare(i, Resolved::Other),
            Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
                for a in args {
                    self.pattern(a);
                }
            }
            Pattern::Literal(..) | Pattern::Wildcard(_) => {}
        }
    }

    fn or_disposition(&mut self, d: &'e OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                self.expr(e)
            }
            OrDisposition::Raise(_)
            | OrDisposition::Discard(_)
            | OrDisposition::Wait(_) => {}
        }
    }

    fn stmt(&mut self, s: &'e Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                self.expr(value);
                self.declare_let(name, Some(value));
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value);
                for n in names {
                    self.declare_let(n, None);
                }
            }
            Stmt::Assign { target, value, .. } => {
                for seg in &target.tail {
                    match seg {
                        LValueSeg::Index(ix) => self.expr(ix),
                        LValueSeg::Field(_) => {}
                    }
                }
                self.expr(value);
                match self.note_use(&target.head) {
                    Resolved::Let(k) => {
                        self.out.assigned.insert(k);
                        if target.tail.is_empty() {
                            self.out.writes.push((k, value));
                        }
                    }
                    _ => {
                        self.out
                            .assigned_names
                            .insert(target.head.name.clone());
                    }
                }
            }
            Stmt::Return(value, _) => {
                if let Some(e) = value {
                    self.expr(e);
                    self.out.returns.push(e);
                }
            }
            Stmt::If(i) => self.if_chain(i, true),
            Stmt::Match(m) => self.match_arms(m, true),
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter);
                self.frames.push(Vec::new());
                self.declare(name, Resolved::Other);
                self.block(body, true);
                self.frames.pop();
            }
            Stmt::ShmWrite { max, binding, body, .. } => {
                self.expr(max);
                self.frames.push(Vec::new());
                self.declare(binding, Resolved::Other);
                self.block(body, true);
                self.frames.pop();
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
                    Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) => {
                        self.expr(e)
                    }
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
            Expr::Ident(i) => {
                self.note_use(i);
            }
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
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
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
                self.expr(receiver)
            }
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
            bindings: BTreeMap::new(),
        },
        next_id: 0,
        next_slot: 0,
        scopes: Vec::new(),
        slots: BTreeMap::new(),
        decl: String::new(),
        owner_locus: String::new(),
        returned: ReturnedBindings::default(),
    };
    // The counter starts past the largest id already present: the
    // snapshot (F.40 1.1b) mints every site before lowering, and the
    // stdlib re-parsed in codegen arrives unnumbered, so numbering
    // from zero here would collide a stdlib literal with a minted user
    // site in the table. `numbered` stays "ids this run assigned".
    let mut start: u32 = 0;
    hale_syntax::sites::for_each_site(program, &mut |_, _, id| {
        if !id.is_none() {
            start = start.max(id.0 + 1);
        }
    });
    r.next_id = start;
    r.walk_decls(&mut program.items, "");
    r.table.numbered = r.next_id - start;
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

/// GH #383 — which free fns provably return a FRESH locus?
///
/// Since v0.14 a locus-typed field may only be assigned a locus
/// LITERAL (`check_locus_field_store`), so a locus a factory returns
/// has exactly one place it can come to rest: the binding that names
/// it. That is what makes caller-scoped teardown sound — the
/// ownership ambiguity which defeated the earlier attempts on this
/// issue is now a compile error rather than a runtime guess.
///
/// A fn qualifies when:
///   - its declared return type names a locus L (resolved through the
///     import-rename table, so `mat::Matrix` counts);
///   - every return is a direct `L { … }` literal, or one single
///     `let`-bound ident whose binding is itself fresh — an `L { … }`
///     literal or a call to an already-qualifying factory (hence the
///     fixpoint: helpers build on other factories);
///   - that binding never escapes into argument position, another
///     literal, or a reassignment (receiver-position use such as
///     `m.set(i, v)` is fine — using a locus is not transferring it);
///   - no statement form this walk does not explicitly recognize
///     appears.
///
/// The escape walk matches every expression form and answers by
/// identifier: a field, method or struct-init name that happens to
/// spell the binding is not a use of it (F.40 phase 1.2c; it used to
/// search the node's Debug rendering for the name).
///
/// Every "don't know" answers NOT fresh, preserving the old
/// program-lifetime behavior rather than risking a double dissolve.
///
/// The fresh-factory seed: every free fn proven to return a fresh
/// locus, keyed by its merged-program name, with the locus it returns
/// and the binding it hands back (if any). [`resolve_owners`] takes it
/// as `base` and [`OwnerTable::extended_fresh_factories`] adds the
/// carrier returns it misses. This is its one producer.
pub fn compute_fresh_locus_factories(
    program: &Program,
    import_renames: &[(Vec<String>, String)],
) -> BTreeMap<String, (String, Option<String>)> {

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
        Literal,
        CallTo(String),
        Other,
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
        lets: &mut Vec<(String, Freshness)>,
        l: &str,
        renames: &[(Vec<String>, String)],
    ) {
        for s in &b.stmts {
            match s {
                Stmt::Return(Some(e), _) => rets.push(e.clone()),
                Stmt::Let { name, value, .. } => {
                    let fr = match value {
                        Expr::Struct { path, .. }
                            if resolve(&qname(path), renames).as_deref()
                                == Some(l) =>
                        {
                            Freshness::Literal
                        }
                        Expr::Call { callee, .. } => {
                            match callee_name(callee, renames) {
                                Some(n) => Freshness::CallTo(n),
                                None => Freshness::Other,
                            }
                        }
                        _ => Freshness::Other,
                    };
                    lets.push((name.name.clone(), fr));
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

    let mut out: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    loop {
        let mut added = false;
        // GH #884: module nesting flattened — a factory fn one
        // brace deeper is lowered and called like any other, so it
        // has to enter the same fixpoint.
        for item in hale_syntax::ast::flat_decls(&program.items) {
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
            let mut fresh_name: Option<String> = None;
            let mut ok = true;
            for r in &rets {
                match r {
                    Expr::Struct { path, .. }
                        if resolve(&qname(path), import_renames).as_deref()
                            == Some(l.as_str()) => {}
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
                    Expr::Call { callee, .. }
                        if callee_name(callee, import_renames)
                            .and_then(|c| out.get(&c).cloned())
                            .map(|(cl, _)| cl == l)
                            .unwrap_or(false) => {}
                    Expr::Ident(i) => match &fresh_name {
                        None => fresh_name = Some(i.name.clone()),
                        Some(n) if *n == i.name => {}
                        Some(_) => { ok = false; break; }
                    },
                    _ => { ok = false; break; }
                }
            }
            if !ok {
                continue;
            }
            if let Some(x) = &fresh_name {
                let bindings: Vec<&(String, Freshness)> =
                    lets.iter().filter(|(n, _)| n == x).collect();
                if bindings.len() != 1 {
                    continue;
                }
                let fresh_binding = match &bindings[0].1 {
                    Freshness::Literal => true,
                    Freshness::CallTo(c) => {
                        out.get(c).map(|(cl, _)| *cl == l).unwrap_or(false)
                    }
                    Freshness::Other => false,
                };
                if !fresh_binding || !body_ok(&f.body, x) {
                    continue;
                }
            }
            out.insert(f.name.name.clone(), (l, fresh_name));
            added = true;
        }
        if !added {
            break;
        }
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

/// Seed from [`compute_fresh_locus_factories`]'s map and add the fns whose every return arm is
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
            // Every value the fn hands back, wherever the `return`
            // stands — inside an expression block too (GH #1140) — and
            // each binding resolved where the return spells it.
            let bindings = body_bindings(&f.body);
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
            // program). The name is resolved to the binding in scope
            // where the return spells it (GH #1140: an inner `let`
            // shadowing the name is another binding), and only when
            // that binding is never re-assigned; anything else leaves
            // the fn out of the set, which is the old leak and never a
            // double free.
            let all_fresh = arms
                .iter()
                .all(|a| arm_is_fresh(a, &l, &out, renames, &bindings, 0));
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
/// build one, or a name whose binding — the `let` in scope where it is
/// spelled, never re-assigned — holds either, including through a
/// carrier, which is the `let t = if c { make(1) } else { make2(1) };
/// return t;` spelling of the same program.
fn arm_is_fresh(
    a: &Expr,
    l: &str,
    known: &BTreeMap<String, String>,
    renames: &[(Vec<String>, String)],
    bindings: &BodyBindings<'_>,
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
            let Some(k) = bindings.let_at(i) else {
                return false;
            };
            if bindings.is_assigned(i, k) {
                return false;
            }
            let Some(rhs) = bindings.lets.get(&k) else {
                return false;
            };
            let mut inner = Vec::new();
            return_arms(rhs, &mut inner);
            !inner.is_empty()
                && inner.iter().all(|x| {
                    arm_is_fresh(x, l, known, renames, bindings, depth + 1)
                })
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
                let d = if self.returned.let_is_returned(name) {
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
                let head_is_returned =
                    bare && self.returned.assign_is_returned(&target.head);
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

    /// The same, for a body: the bindings it hands back with a bare
    /// `return x;` are resolved first, so a `let` can see them.
    fn open_body_frame(&mut self, decl: &str, body: &Block) -> SavedFrame {
        let saved = self.open_frame(decl);
        self.returned = returned_bindings(body);
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
                | TopDecl::Role(_)
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
