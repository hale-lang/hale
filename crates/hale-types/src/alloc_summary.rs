//! GH #18 item 1 (memory-bound proofs) — staging step 1: the shared
//! per-method allocation summary + call graph scaffold.
//!
//! This is the reusable compile-time dataflow IR that the bound solver
//! (item 1's later stages), closure-lifting (item 3), and resource-budget
//! tracking (item 5) all consume. It does **no bound-proving**. It walks
//! every free fn, locus method, and lifecycle hook in a bundle and records,
//! per body:
//!
//! - **allocation sites** — `Struct` / `Array` / `[v; N]` / `Bytes` literals
//!   (each lowers to an arena alloc), tagged **local** vs **escaping**
//!   (flows to a `return`, a `self.field` store, or a bus `<-`), with its
//!   enclosing **loop depth** and (step 2) a **reclaim scope** + **bound
//!   verdict**. String `+` concat is also a real site, but telling it from
//!   arithmetic `+` needs type info this pass lacks, so it's deferred to a
//!   type-aware stage rather than flagged and crying wolf on every `i + 1`;
//! - **call edges** — resolved to a `FnKey` where possible (free fn,
//!   `self`-method) or left unresolved (foreign receiver / stdlib / builtin),
//!   each with its loop depth;
//! - **loops** — `for`-range (bounded if the range is a const literal),
//!   `for`-iter (runtime collection), `while`, `while true`;
//! - **entry classification** — `run()` / lifecycle hooks / `main` are
//!   one-shot; bus handlers are per-message (unbounded).
//!
//! The escape tagging is intentionally a first approximation: it catches
//! allocations *syntactically* in an escape position and the common
//! `let x = <alloc>; … return x;` indirection (via a name pre-pass).
//! Deeper aliasing (`let x = alloc; return Struct { f: x }`) and
//! type-driven concat/collection-grow refinement are the next stage's job.
//! Mirrors the structure of [`crate::purity`].

use std::collections::{BTreeMap, BTreeSet};

use hale_graph::ids::SiteId;
use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::placement::SiteUniverse;

/// A fn-like declaration's identity (F.40 phase 3, C3): the site the mint
/// gave the declaration (a fn, a method, a lifecycle hook, a mode, a
/// failure handler, a perspective's fn), as the universe that minted it
/// and its index there. One counter numbers every seed of a universe, so
/// the index names the site in it, and a reader holding a declaration's
/// `NodeId` names its row without the snapshot. The stdlib's analysis
/// copy is its own universe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeclId {
    pub universe: SiteUniverse,
    pub index: u32,
}

impl DeclId {
    /// The declaration `node` of `universe`; `None` for a node no mint
    /// numbered.
    pub fn of(universe: SiteUniverse, node: NodeId) -> Option<DeclId> {
        (!node.is_none()).then_some(DeclId { universe, index: node.0 })
    }

    /// A declaration of the checked programs.
    pub fn user(node: NodeId) -> Option<DeclId> {
        DeclId::of(SiteUniverse::User, node)
    }

    /// A declaration of the stdlib's analysis copy.
    pub fn stdlib(node: NodeId) -> Option<DeclId> {
        DeclId::of(SiteUniverse::StdlibAnalysis, node)
    }
}

/// Identifies a fn for summary lookup: the declaration's identity
/// ([`DeclId`]), with the (locus name, fn name) pair its display name.
/// Free fns have `locus: None`; locus methods + lifecycle hooks carry
/// the enclosing locus's name. Lifecycle hooks are keyed by their kind
/// (`"run"`, `"birth"`, …) — these never collide with method names
/// since the kinds are reserved keywords.
///
/// Two keys are one row only if they name one declaration: two
/// declarations sharing a name (a `fn f` in each of two modules) are two
/// rows. Ordering is the display name's first, so a reader iterating the
/// rows meets them in name order. `decl` is `None` only for a
/// declaration no snapshot minted (a bundle no entry point minted), whose
/// rows join by name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FnKey {
    pub locus: Option<String>,
    pub fn_name: String,
    pub decl: Option<DeclId>,
}

impl FnKey {
    /// The row of the free fn declared at `decl`.
    pub fn free_fn(decl: Option<DeclId>, name: impl Into<String>) -> Self {
        Self { locus: None, fn_name: name.into(), decl }
    }
    /// The row of the member of `locus` declared at `decl`.
    pub fn method(decl: Option<DeclId>, locus: impl Into<String>, name: impl Into<String>) -> Self {
        Self { locus: Some(locus.into()), fn_name: name.into(), decl }
    }
    pub fn display(&self) -> String {
        match &self.locus {
            Some(l) => format!("{}::{}", l, self.fn_name),
            None => self.fn_name.clone(),
        }
    }
}

/// What an allocation site allocates. `PossibleConcat` is a `+` whose
/// operands may be Strings (an arena `lotus_str_concat`) — but with no
/// type info here it also covers arithmetic `+`, which a later stage
/// prunes. The rest each unambiguously lower to an arena allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocKind {
    StructLit(String),
    ArrayLit,
    ArrayRepeat,
    BytesLit,
    /// Phase D / D2: an insert into a growing `@form(vec | hashmap)` slot
    /// (`v.push(x)` / `m.set(x)`). Detected when the receiver's *declared*
    /// type resolves to such a form locus. The collection's backing buffer
    /// grows with population and frees only at dissolve — so an insert in
    /// an unbounded context accumulates. The string is the form name.
    CollectionInsert(String),
    /// String concatenation (`a + b` where an operand is provably a
    /// String). Each `+` builds a fresh String in the arena.
    StringConcat,
}

impl AllocKind {
    fn label(&self) -> String {
        match self {
            AllocKind::StructLit(n) => format!("struct {}", n),
            AllocKind::ArrayLit => "array-literal".to_string(),
            AllocKind::ArrayRepeat => "array-repeat".to_string(),
            AllocKind::BytesLit => "bytes-literal".to_string(),
            AllocKind::CollectionInsert(form) => format!("{}-insert", form),
            AllocKind::StringConcat => "string-concat".to_string(),
        }
    }
}

/// Phase D / D2: the `@form(...)` names whose inserts grow without bound
/// (backing buffer grows with population, frees only at dissolve). A
/// `ring_buffer` / `lru_cache` is cap-bounded and excluded.
fn form_grows(form_name: &str) -> bool {
    matches!(form_name, "vec" | "hashmap")
}

/// Phase D / D2: the method names that *insert* into a collection (vs read
/// it). Gated by `form_grows` on the receiver's form, so a `get`/`len`/`pop`
/// never counts.
///
/// iris handoff P1 (2026-07-27): `set` no longer counts for VEC —
/// a vec `.set` replaces in place and RETIRES the old element onto
/// the reuse freelist (RSS-proven flat at 2M sets), so it is not an
/// accumulation channel anymore. `hashmap.set` keeps counting: it
/// can insert a NEW key (growth), and its replaced-value retirement
/// already has its own retired_store modeling (Gap D).
fn is_insert_method(method: &str) -> bool {
    matches!(method, "push" | "set" | "insert" | "add")
}

fn is_growing_insert(form_name: &str, method: &str) -> bool {
    if form_name == "vec" && method == "set" {
        return false;
    }
    form_grows(form_name) && is_insert_method(method)
}

/// The unqualified name of a `Named` type expression (`SegVec` from
/// `path::to::SegVec`), or `None` for primitives / arrays / etc.
fn type_expr_name(te: &TypeExpr) -> Option<String> {
    match te {
        TypeExpr::Named { path, .. } => {
            // #392: a PATH-written stdlib type (`std::http::Router`,
            // `std::http::RouteHandler`) resolves to the name its
            // decl actually carries — the same std-vs-user rule
            // struct-literal receivers already apply. Recording the
            // bare last segment ("Router") put these values in a
            // name space no summary map is keyed by, so a receiver
            // typed by one landed `Unresolved` and every judgment
            // dropped the edge — including the router chain's
            // interface dispatch.
            if path.segments.len() > 1 {
                let segs: Vec<&str> = path
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect();
                if let Some(m) =
                    crate::stdlib_bodies::mangled_locus_name(&segs)
                {
                    return Some(m.to_string());
                }
            }
            path.segments.last().map(|s| s.name.clone())
        }
        _ => None,
    }
}

/// Where an allocation's value flows. `Local` is freed at scope exit (the
/// arena/subregion reclaims it). The escaping variants persist past the
/// scope and so accumulate in the owner across invocations — the leak
/// channel the bound solver cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escape {
    Local,
    Returned,
    StoredToSelf,
    Sent,
}

impl Escape {
    fn label(&self) -> &'static str {
        match self {
            Escape::Local => "local",
            Escape::Returned => "escaping=return",
            Escape::StoredToSelf => "escaping=self-store",
            Escape::Sent => "escaping=bus-send",
        }
    }
    /// Does this value survive its method's return — i.e. persist across
    /// *invocations* of an unboundedly-invoked fn (a per-message handler)?
    ///
    /// A locus method / bus handler opens a **method-scratch subregion** at
    /// entry and destroys it at exit (per delivery) — transients allocate
    /// into the scratch and are freed per call, while escaping values are
    /// copied out to `self` / the caller first (see
    /// `open_method_scratch` / `emit_method_scratch_destroy` in codegen).
    /// So a `Local` is reclaimed per invocation and does NOT accumulate
    /// across deliveries; only `StoredToSelf` (persists in the locus) and
    /// `Returned` (escapes to the caller) do. `Sent` is reclaimed per
    /// dispatch (handled by its `AfterBusDispatch` reclaim scope).
    ///
    /// This is only about *cross-invocation* multiplicity. A `Local` in an
    /// unbounded loop *within a single call* still accumulates until that
    /// call returns — that case is caught by the in-loop verdict, not here.
    /// Nor is a `Local` reclaimed by every frame: a free fn that is not
    /// scratch-local opens no scratch, its body allocates into its
    /// caller's arena, and the boundary there is the caller's
    /// (`ReclaimScope`, E3b).
    fn persists_across_calls(&self) -> bool {
        matches!(self, Escape::StoredToSelf | Escape::Returned)
    }
}

/// When an allocation's memory is actually reclaimed — the *empirically
/// validated* reclamation model (step 2), which differs from
/// `spec/memory.md`. Measured: a struct allocated inside a non-inlinable
/// free fn called 3M× in a loop accumulates to ~99 MB (vs ~5 MB for an
/// alloc-free loop) — i.e. **a free fn's return does not reclaim per
/// call**: its body allocates into its caller's arena (codegen's
/// `current_arena_ptr`), so a value allocation lives until its enclosing
/// **locus** dissolves. Two boundaries come sooner: a bus send's
/// per-dispatch arena, and a scratch-local free fn's own subregion
/// (GH #1148), freed when the fn returns.
///
/// The boundary is judged relative to the loop analyzed
/// ([`ReclaimScope::accumulates_in_loop`]): a fn's return falls inside
/// each iteration of a loop that calls the fn, and outside every
/// iteration of a loop in the fn's own body, so a function return is not
/// an iteration's reclamation. The conservative consequence (and the
/// whole point — "no false bounded"): an allocation accumulates across
/// the iterations of every loop its boundary lies outside, bounded only
/// by that loop's trip count.
///
/// `Local` is not scratch: an allocation's [`Escape`] says whether its
/// value leaves the fn, and the fn's frame says whether anything frees
/// it when the fn returns. A `Local` in a free fn that is not
/// scratch-local lands in its caller's arena and lives as long as the
/// caller's frame does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReclaimScope {
    /// Freed wholesale only when the enclosing locus dissolves — so it
    /// accumulates across every loop iteration in between. Covers
    /// `Returned` and `StoredToSelf` value allocations, and a `Local`
    /// in any frame but a scratch-local free fn's (a method's or a
    /// handler's per-call scratch is the activation boundary
    /// `AllocSummary::final_verdict` applies to it).
    EnclosingLocus,
    /// Routed to the bus payload arena, reclaimed after the message is
    /// dispatched — a genuine per-iteration boundary. (Modeled from the
    /// spec + bus codegen; RSS-validation of this path is pending, noted
    /// in the step-2 validation test.)
    AfterBusDispatch,
    /// A `Local` allocation in a scratch-local free fn
    /// ([`FnSummary::frees_at_return`]): the fn's body allocates into
    /// its own subregion, destroyed at return after the epilogue
    /// deep-copies the return value into the caller's arena. A value
    /// that escapes the fn (its return value) is not reclaimed here,
    /// and a recursive fn keeps [`ReclaimScope::EnclosingLocus`]: its
    /// activations' subregions are alive at once, to an unbounded
    /// depth.
    FnReturn,
}

/// Where the loop a reclaim boundary is judged against sits, relative to
/// the allocation ([`ReclaimScope::accumulates_in_loop`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopAt {
    /// In the allocation's own body, enclosing it.
    OwnBody,
    /// In a caller, enclosing a call that reaches the allocation's fn.
    Caller,
}

impl ReclaimScope {
    fn of(escape: Escape) -> Self {
        match escape {
            Escape::Sent => ReclaimScope::AfterBusDispatch,
            _ => ReclaimScope::EnclosingLocus,
        }
    }
    /// Does this allocation persist across the iterations of a loop at
    /// `at` (vs being reclaimed within each iteration)? A fn's return
    /// ends each iteration of a caller's loop, never an iteration of the
    /// fn's own.
    pub fn accumulates_in_loop(&self, at: LoopAt) -> bool {
        match self {
            ReclaimScope::EnclosingLocus => true,
            ReclaimScope::AfterBusDispatch => false,
            ReclaimScope::FnReturn => at == LoopAt::OwnBody,
        }
    }
    fn label(&self) -> &'static str {
        match self {
            ReclaimScope::EnclosingLocus => "reclaim@locus-dissolve",
            ReclaimScope::AfterBusDispatch => "reclaim@bus-dispatch",
            ReclaimScope::FnReturn => "reclaim@fn-return",
        }
    }
}

/// The model's per-site bound verdict (step 2 output; step 3 turns the
/// unbounded case into a diagnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteVerdict {
    /// Not in any loop — at most one allocation per invocation. (Whether
    /// the invocation itself is unbounded is the call-graph multiplicity
    /// question, resolved in step 3.)
    OncePerInvocation,
    /// In a loop but reclaimed each iteration (bus dispatch) — bounded.
    PerIterationReclaim,
    /// Accumulates, but every enclosing loop has a const trip count —
    /// bounded by that constant.
    AccumulatesBoundedLoop,
    /// Accumulates inside an unbounded loop (`while true` / runtime
    /// `for`-iter) — the leak precursor.
    AccumulatesUnbounded,
}

impl SiteVerdict {
    fn label(&self) -> &'static str {
        match self {
            SiteVerdict::OncePerInvocation => "once-per-invocation",
            SiteVerdict::PerIterationReclaim => "per-iteration-reclaim",
            SiteVerdict::AccumulatesBoundedLoop => "accumulates×const",
            SiteVerdict::AccumulatesUnbounded => "ACCUMULATES-UNBOUNDED",
        }
    }
}

/// One allocation site in a body.
#[derive(Debug, Clone)]
pub struct AllocSite {
    pub kind: AllocKind,
    pub escape: Escape,
    pub loop_depth: u32,
    /// True if any enclosing loop has a non-const (runtime / `while true`)
    /// trip count.
    pub in_unbounded_loop: bool,
    /// M3 stage 5 gap B: true if any enclosing loop is literally
    /// `while true` — a frame stuck in one never reaches its
    /// method-exit scratch destroy.
    pub in_infinite_loop: bool,
    pub reclaim: ReclaimScope,
    /// Phase D / D1: when this allocation is stored straight into a
    /// `self.<field>` (a whole-value replace, `StoredToSelf`), the field
    /// name. `None` for non-self escapes and for indexed in-place writes.
    /// The solver (D2) uses it to ask whether `<field>` is a capacity slot.
    pub target_field: Option<String>,
    /// Gap D (2026-07-17): a whole-field `self.<f> = Struct { ... }`
    /// replace whose struct type has ONLY scalar / String fields. Since
    /// Gap A, such a store fully reclaims: the struct's bytes memcpy in
    /// place, and each replaced String clone RETIRES at the enclosing
    /// method's activation boundary and recycles on the next store —
    /// RSS-validated flat over 1M replaces (alloc_model_rss.rs::
    /// self_field_struct_replace_churn). Cross-invocation multiplicity
    /// therefore no longer accumulates for these sites. The within-call
    /// loop verdict is NOT flipped: retires only flush at activation
    /// exit, so a `while true` churn inside one call (a `run()` loop)
    /// still grows and stays flagged. Structs with Bytes / nested
    /// compound / array fields keep the conservative verdict (those
    /// leaves don't retire yet).
    pub retired_store: bool,
    /// The site is written inside a loop body. `loop_depth` is how
    /// deep the allocation repeats, which a `return` / `fail` payload
    /// resets (it allocates once per call); this is where it sits.
    pub in_loop: bool,
    /// A struct literal written as a whole statement (`Child { … };`).
    pub bare_stmt: bool,
    /// A struct literal written as the whole right side of a
    /// `self.<field> = …` replace: the assignment statement's span.
    pub self_replace: Option<Span>,
    pub span: Span,
}

impl AllocSite {
    pub fn verdict(&self) -> SiteVerdict {
        if self.loop_depth == 0 {
            SiteVerdict::OncePerInvocation
        } else if !self.reclaim.accumulates_in_loop(LoopAt::OwnBody) {
            SiteVerdict::PerIterationReclaim
        } else if self.in_unbounded_loop {
            SiteVerdict::AccumulatesUnbounded
        } else {
            SiteVerdict::AccumulatesBoundedLoop
        }
    }
}

/// How a call's callee is written. The edge's [`Callee`] is what the
/// call resolves to; this is the spelling, which a resolved edge no
/// longer shows (a cross-seed `alias::name` resolves to its mangled
/// symbol, a typed method call to its locus's key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallSpelling {
    /// `name(…)`.
    Ident(String),
    /// `a::b::name(…)`: the path, joined.
    Path(String),
    /// `receiver.name(…)`.
    Method(String),
    /// `receiver::name(…)`.
    PathMethod(String),
    /// Any other callee expression.
    Expr,
}

impl CallSpelling {
    pub fn of(callee: &Expr) -> CallSpelling {
        match callee {
            Expr::Ident(id) => CallSpelling::Ident(id.name.clone()),
            Expr::Path(qn) => CallSpelling::Path(
                qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
            ),
            Expr::Field { name, .. } => CallSpelling::Method(name.name.clone()),
            Expr::Path2 { name, .. } => CallSpelling::PathMethod(name.name.clone()),
            _ => CallSpelling::Expr,
        }
    }

    /// The allocating receives: a `recv` that returns a freshly
    /// allocated result buffer (in the caller's scratch), where
    /// `recv_into` with a reused buffer is the zero-alloc alternative.
    /// Their spelling as written: the full path for the path-call form
    /// (`std::io::udp::recv`); the method name for the method-call form
    /// (`stream.recv_bytes(n)`), whose stdlib handle receiver may not
    /// type, and which is kept to the two names specific enough to the
    /// stdlib that a user method is unlikely to share them (a plain
    /// `recv` counts in the path-call form only). The one list: the
    /// hot-path lint and `@budget` both read it off the edge.
    pub fn allocating_recv(&self) -> Option<String> {
        match self {
            CallSpelling::Path(p) => matches!(
                p.as_str(),
                "std::io::tcp::recv"
                    | "std::io::tcp::recv_bytes"
                    | "std::io::udp::recv"
                    | "std::io::udp::recv_with_source"
                    | "std::io::tls::recv_bytes"
            )
            .then(|| p.clone()),
            CallSpelling::Method(m) => {
                matches!(m.as_str(), "recv_bytes" | "recv_with_source").then(|| m.clone())
            }
            _ => None,
        }
    }
}

/// A resolved or unresolved call target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callee {
    /// A bundle-local free fn or `self`-method we resolved to a key.
    Resolved(FnKey),
    /// A foreign-receiver method, stdlib path, or builtin we can't
    /// resolve at this (type-free) layer.
    Unresolved(String),
}

/// One call edge out of a body.
#[derive(Debug, Clone)]
pub struct CallEdge {
    pub callee: Callee,
    /// #341: the receiver's declared type name for a method call.
    ///
    /// A SYNTHESIZED form method (`counts.set(x)`) has no summary
    /// entry, so it resolves to `Unresolved("set")` — a bare name with
    /// the receiver discarded. That lost the one fact an effect
    /// predicate needs about it: whether the receiver's form carries a
    /// `sync` discipline, and therefore a lock.
    ///
    /// The name was already computed one line earlier to attempt
    /// resolution; this keeps it instead of throwing it away.
    pub recv_ty: Option<String>,
    /// #382 soundness audit: this call was written WITH a method
    /// receiver (`expr.m(...)`) that is not `self` and not an import
    /// alias — so if the receiver could not be typed, the callee is a
    /// method of SOME bundle locus reached through an expression the
    /// summarizer cannot see through (a struct literal, a chained
    /// field, a call result, a branch value). Recorded so a soundness
    /// judgment can fail closed on `receiver_present &&
    /// recv_ty.is_none() && unresolved` instead of silently dropping
    /// the edge — the wrapper variant of the same hole that made
    /// #353 record `indirect`.
    pub receiver_present: bool,
    /// #353: this call goes through a FUNCTION-TYPED PARAMETER of the
    /// enclosing fn — an indirect call whose target is not knowable
    /// from this fn alone.
    ///
    /// Recorded here for the same reason `recv_ty` is: the enclosing
    /// fn's parameter list is in hand at construction and nowhere
    /// else, and without it the edge is indistinguishable from a call
    /// to an unknown free fn. That indistinguishability is what let an
    /// indirect call void every certificate.
    ///
    /// E5 (F.40 phase 3): a call through a local the body's bindings do
    /// not follow to a fn (`unresolved_local`) and a computed callee
    /// (`pick()(x)`) are indirect too: a function value whose target
    /// this body does not name is the same unknown as a parameter's, and
    /// every certificate and budget reader takes it as one (a classified
    /// correction: such a call was a call to nothing). The capability
    /// admission words the shapes apart (`through_param`).
    pub indirect: bool,
    /// The indirect call is through a function-typed parameter of the
    /// enclosing fn (#353's shape), not a local or a computed callee.
    pub through_param: bool,
    /// The call is through a local the body's bindings follow to a fn
    /// (`let f = pid; f()`): the local's name, and the edge's callee is
    /// what a direct call of the bound name or path reaches. A local bound
    /// to anything else leaves the edge as it was written (`Unresolved`
    /// with the local's name).
    pub via_local: Option<String>,
    /// The call is through a local the body's bindings do not follow to
    /// a fn (`let f = self.g; f()`, a parameter, a reassigned local): an
    /// `Unresolved` call named for the local, and `indirect`. The
    /// capability admission reads this as a hole in the program's own
    /// code.
    pub unresolved_local: bool,
    /// E5: that local is bound to a field read (`let f = e.handler_fn`):
    /// the field's name. Every declaration of a field of that name types
    /// the call when each declares a function type.
    pub bound_field: Option<String>,
    pub loop_depth: u32,
    /// True if the call is inside an unbounded loop — then the callee is
    /// invoked unboundedly many times regardless of its own multiplicity.
    pub in_unbounded_loop: bool,
    /// Where the call's *result* flows — the call-result analog of an
    /// allocation site's escape. A resource-acquiring call (an fd opener)
    /// whose result escapes in an unbounded context holds the resource
    /// resident → a leak; a `Local` result is bound-and-dissolved per
    /// iteration → bounded. (Closes the gap noted in
    /// notes/resource-budgets.md; also lets #1 see a factory call whose
    /// result escapes when the callee body is external/unresolved.)
    pub escape: Escape,
    /// Phase D / D1: when the call is a method on a `self.<slot>` receiver
    /// (`self.entries.acquire()`, `self.items.alloc()`), the slot name.
    /// `None` for free fns, `self`-methods, and form methods (`self.push`,
    /// where the slot is implicit from `@form`). The solver (D2) pairs this
    /// with the method name + `LocusShape` to classify a slot insert.
    pub receiver_slot: Option<String>,
    /// #392: this call dispatches through an INTERFACE-typed receiver
    /// (`route.handler.handle(ctx)`). Set on two edge shapes, both
    /// produced by the fan-out pass at the end of summary construction:
    ///
    ///  * a `Resolved` alternative — the closed world makes the
    ///    implementor set enumerable, so the one written call becomes
    ///    one edge per conforming locus (over-approximation: only adds
    ///    edges). The interface name is kept for diagnostics and the
    ///    topology artifact.
    ///  * a still-`Unresolved` edge whose interface has NO conforming
    ///    locus in the bundle. That is not an unknown: an interface
    ///    value only ever comes from coercing a conforming locus, so
    ///    an uninhabited interface has no values and the call site is
    ///    DEAD in this build. Walkers contribute nothing for it; the
    ///    topology artifact records it (inside the hashed model half)
    ///    so an outside evaluator applies the same rule and a
    ///    conformer appearing later changes `shape_hash`. (The
    ///    router's `m.before(cur)` over an empty middleware list is
    ///    the everyday instance — failing closed there would refuse
    ///    every certificate through the stdlib router.)
    pub via_interface: Option<String>,
    /// #392: groups the fanned-out `Resolved` alternatives of ONE
    /// dispatch site (unique across the summary). A dispatch invokes
    /// exactly one alternative at runtime, so counting judgments
    /// (`bound`, `@budget`, quantitative dims) take the MAX over a
    /// group where a real call sequence would SUM; reachability and
    /// effect-union judgments walk every alternative as usual.
    pub dispatch_group: Option<u32>,
    /// E5: this edge is one alternative of an indirect call resolved to
    /// the program's function values (`resolve_function_values`): the
    /// callee as written (`f`, `__route_fn`, `<expr>`). Its alternatives
    /// share a `dispatch_group`, as an interface dispatch's do.
    pub via_value: Option<String>,
    /// How many arguments the call passes.
    pub arity: usize,
    /// How the callee is written.
    pub spelling: CallSpelling,
    /// The call is written inside a loop body (`loop_depth` is reset
    /// in a `return` / `fail` payload, as an [`AllocSite`]'s is).
    pub in_loop: bool,
    /// The call is the whole value of a `let` (or a tuple `let`): the
    /// statement's span.
    pub let_span: Option<Span>,
    /// The call is an allocating receive
    /// ([`CallSpelling::allocating_recv`]): as written.
    pub allocating_recv: Option<String>,
    pub span: Span,
    /// The callee expression's own span (`std::process::pid` of
    /// `std::process::pid()`): where a diagnostic about the callee, not
    /// the call, is located (the capability admission's refusals).
    pub callee_span: Span,
}

impl CallEdge {
    /// The shared fail-closed test for an `Unresolved` method call —
    /// the #382 backstop, in one place for all five walkers. True when
    /// the receiver was written but could not be typed (an index
    /// result, a match value, a foreign expression: the callee is a
    /// method of SOME bundle locus reached through an expression the
    /// walk cannot see through, possibly a wrapper). Every judgment
    /// that traverses calls refuses such an edge — unknown ⇒
    /// violation — so fn-level certificates and bundle-level claims
    /// agree.
    ///
    /// Deliberately NOT included: an unresolved dispatch through an
    /// uninhabited interface (`via_interface` on an `Unresolved`
    /// edge). That edge is dead, not unknown — see the field doc.
    pub fn opaque_method_call(&self) -> bool {
        self.receiver_present && self.recv_ty.is_none()
    }

    /// What an indirect call goes through, as a diagnostic says it.
    pub fn indirect_through(&self) -> &'static str {
        if self.through_param {
            "a function-typed parameter"
        } else {
            "a function value"
        }
    }
}

/// A loop in a body. `bounded` carries a const trip count when the loop is
/// a `for` over a literal-int range; `None` means the trip count is
/// runtime input (the bound solver's hard case).
#[derive(Debug, Clone)]
pub enum LoopKind {
    ForRange { bounded: Option<i64> },
    ForIter,
    While,
    WhileTrue,
    /// `while v < N { … v += c … }` — a const-bounded counter: `v` is
    /// const-initialized and only ever incremented by positive consts
    /// toward a const ceiling `N`, so the trip count is bounded by a
    /// compile-time constant (proven by loop-ranking).
    WhileCounter,
}

impl LoopKind {
    fn label(&self) -> String {
        match self {
            LoopKind::ForRange { bounded: Some(n) } => format!("for-range(bounded={})", n),
            LoopKind::ForRange { bounded: None } => "for-range(runtime)".to_string(),
            LoopKind::ForIter => "for-iter(runtime)".to_string(),
            LoopKind::While => "while".to_string(),
            LoopKind::WhileTrue => "while-true".to_string(),
            LoopKind::WhileCounter => "while-counter(bounded)".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoopInfo {
    pub kind: LoopKind,
    pub depth: u32,
    pub span: Span,
}

/// How a body is reached. One-shot entries run once per locus instance;
/// `BusHandler` fires per message (unbounded). Non-entry fns carry `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Main,
    Run,
    BusHandler,
    Birth,
    Accept,
    Release,
    Drain,
    Dissolve,
}

impl EntryKind {
    fn one_shot(&self) -> bool {
        !matches!(self, EntryKind::BusHandler)
    }
    fn label(&self) -> &'static str {
        match self {
            EntryKind::Main => "main",
            EntryKind::Run => "run",
            EntryKind::BusHandler => "bus-handler",
            EntryKind::Birth => "birth",
            EntryKind::Accept => "accept",
            EntryKind::Release => "release",
            EntryKind::Drain => "drain",
            EntryKind::Dissolve => "dissolve",
        }
    }
}

/// The per-fn summary.
#[derive(Debug, Clone)]
pub struct FnSummary {
    pub key: FnKey,
    pub entry: Option<EntryKind>,
    pub sites: Vec<AllocSite>,
    pub calls: Vec<CallEdge>,
    pub loops: Vec<LoopInfo>,
    /// GH #265: non-call effect sites in this fn's own body — effects
    /// carried by SYNTAX rather than by a stdlib call, so a leaf
    /// predicate over call edges alone can never see them. Today:
    /// `Topic <- value` publishes and locus instantiations.
    pub effect_sites: Vec<EffectSite>,
    /// #353: names of this fn's FUNCTION-TYPED parameters.
    ///
    /// A call through one of these lands in the graph as
    /// `Callee::Unresolved(param_name)` — indistinguishable from an
    /// unknown free fn, which contributed nothing. That is how an
    /// indirect call voided every certificate: `@no_syscall` on a fn
    /// whose body is `return f(v);` passed while the program performed
    /// the syscall. Knowing which unresolved names are parameters is
    /// what lets the walk treat them as indirect rather than absent.
    pub fn_params: Vec<String>,
    /// The fn carries `@hot` (a lifecycle hook or a mode cannot).
    pub hot: bool,
    /// The row is a `mode` body.
    pub mode: bool,
    /// The declaration's position in the order the summary walks the
    /// programs' declarations: program by program, a module's in place,
    /// a locus's members in order.
    pub decl_index: usize,
    /// The struct literals stored whole into a `self` field with every
    /// init scalar or static (`self.f = P { x: 1 }`): codegen copies
    /// them over the existing value, so they allocate nothing and are
    /// not in `sites`. Kept for a reader about the literal as written.
    pub in_place_sites: Vec<AllocSite>,
    /// What a free fn's body allocates into, as lowering routes it
    /// ([`Frame`]); `None` for a method, a lifecycle hook, a mode and
    /// `main` (lowered as the program's entry), and for a row the summary
    /// did not classify.
    pub frame: Option<Frame>,
}

/// What a free fn's body allocates into (E3b): the scratch-local
/// classification lowering reads (`crate::alloc_routing`'s
/// `scratch_local` row, the same producer over the declarations the
/// summary holds). An allocation's [`Escape`] says whether its value
/// leaves the fn; the frame says whether anything frees it when the fn
/// returns: `Local` is not scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// Scratch-local: the body allocates into the fn's own subregion,
    /// freed at its return after the return value is copied out.
    /// `recursive` when the fn is in a cycle of resolved calls (it can
    /// call itself).
    ScratchLocal { recursive: bool },
    /// Any other free fn: the body allocates into its caller's arena,
    /// and nothing is freed at its return.
    CallersArena,
}

impl FnSummary {
    /// The fn frees the allocations its body keeps at its return: a
    /// scratch-local free fn that is not recursive. Its `Local` sites
    /// reclaim at [`ReclaimScope::FnReturn`], and it is a frame with a
    /// boundary of its own, like a method's per-call scratch.
    pub fn frees_at_return(&self) -> bool {
        self.frame == Some(Frame::ScratchLocal { recursive: false })
    }
}

/// GH #265: an effect a fn performs directly in its own body,
/// syntactically. The classified stdlib frontier covers effects
/// reached by CALLING something; these are the ones Hale expresses
/// as language constructs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectSite {
    pub kind: EffectSiteKind,
    pub loop_depth: u32,
    pub span: Span,
}

/// A publish site's subject, WITH its syntactic form. The resolver
/// distinguishes a string-literal wire address (`"subject" <- v`)
/// from a declared-topic reference (`Topic <- v`) — a literal keeps
/// its literal address even when its text collides with a topic
/// NAME, so consumers must never decide "declared topic?" from the
/// string alone (GH #476 Change 2, round 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishSubject {
    /// The subject as written: the literal text, the topic name, or
    /// the author-spelled qualified path.
    pub text: String,
    /// `true` iff the source expression was a string literal — a
    /// wire address, never a topic reference.
    pub literal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectSiteKind {
    /// `Topic <- value` / `"subject" <- value`. Carries the subject
    /// as written when it is a compile-time-constant string or a
    /// declared topic name (the closed topic set makes publish-set
    /// assertions exact); None for a computed subject.
    Publish(Option<PublishSubject>),
    /// A locus instantiation (`Child { ... }`) — an arena creation
    /// plus, depending on placement, a thread spawn or a pool post.
    Spawn(String),
}

/// A distilled view of a locus's storage shape (GH #18 item 1, Phase D —
/// D1 infra). The bound solver reads this to decide which storage slot an
/// escaping allocation lands in and how that slot bounds it:
///
/// - **slot 0** — the locus's own bump Arena (no entry here; the default).
/// - **slots 1..N** — `capacity { pool/heap … }` slots, optionally fronted
///   by a `@form`. `pool` recycles (bounded-by-balanced-release); `heap`
///   grows; `@form(ring_buffer)` is cap-bounded; `@form(vec)` /
///   `@form(hashmap)` grow.
///
/// D1 only *captures* this — no verdict reads it yet (that's D2).
#[derive(Debug, Clone, Default)]
pub struct LocusShape {
    pub name: String,
    pub capacity_slots: Vec<SlotShape>,
    pub form: Option<FormShape>,
    /// `: projection recognition(cap = N, …)` — a hard static cap on
    /// *child-entity* count (fed by `accept`). Recorded for completeness;
    /// the value-allocation proof reads the capacity-data slots, not this
    /// (entity-count bounding is a separate analysis).
    pub recognition_cap: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SlotShape {
    pub name: String,
    pub kind: CapacitySlotKind,
}

#[derive(Debug, Clone)]
pub struct FormShape {
    pub name: String,
    /// A `cap = N` form-arg, if a literal int. NOTE: per `spec/forms.md`
    /// this is an *initial-size hint* for vec/hashmap (not a bound); it is
    /// a real cap for ring_buffer. The solver (D2) interprets it per form.
    pub cap: Option<i64>,
}

/// The bundle-wide allocation summary + call graph.
#[derive(Debug, Clone, Default)]
pub struct AllocSummary {
    /// iris handoff P2.2: loci only ever instantiated eagerly.
    pub eager_only_loci: BTreeSet<String>,
    /// One row per fn-like declaration, keyed by its identity.
    pub fns: BTreeMap<FnKey, FnSummary>,
    /// The row a call that spells `(locus, name)` reaches: the walk's
    /// resolution, by which an edge names its declaration (the last
    /// declaration of the name, the copy's after the program's). A reader
    /// that holds only the names a call spells (a bus subscriber's
    /// handler) resolves them here ([`AllocSummary::resolve`]).
    pub by_name: BTreeMap<(Option<String>, String), FnKey>,
    /// GH #18 item 1: loci carrying `@bounded` — their leak sites are
    /// reported even without the `--warn-unbounded-alloc` survey flag
    /// (the in-source opt-in).
    pub bounded_loci: BTreeSet<String>,
    /// #340/#341: loci that hold a form carrying a `sync` discipline.
    ///
    /// The LOCK LIVES ON THE FORM, so a call into a locus that holds
    /// one can take that lock. This is INFERRED from structure — no
    /// annotation — because whether the lock exists is a property of
    /// the form's own declaration, not of anyone's intent or of how a
    /// consumer wires up placement.
    pub sync_holding_loci: BTreeSet<String>,
    /// #341: form loci whose `sync` discipline synchronizes — the forms
    /// themselves, not the loci that hold them. A direct call into one
    /// takes its lock. The summary holds the stdlib analysis copy's, from
    /// that universe's form rows (`stdlib_bodies::forms`); the effects
    /// engine adds the program's own, from its snapshot's rows
    /// ([`AllocSummary::add_sync_forms`]). No reader takes a `sync =`
    /// argument for it.
    pub sync_forms: BTreeSet<String>,
    /// #345: classes a fn/locus DECLARES it carries, via
    /// `@effects(is: {…})`. The classification half of a user effect:
    /// propagation is the same engine, this is how a leaf says what it
    /// is.
    pub carries: BTreeMap<FnKey, crate::stdlib_surface::EffectSet>,
    /// Fns carrying `@unbounded` — an acknowledged-intentional
    /// accumulation. Their leak sites are dropped entirely (the
    /// greppable carve-out), even under the survey flag.
    pub unbounded_fns: BTreeSet<FnKey>,
    /// Per-locus storage shape (Phase D / D1) — capacity slots, `@form`,
    /// projection cap. Keyed by locus name.
    pub locus_shapes: BTreeMap<String, LocusShape>,
    /// The fns the program reaches, when the summary holds the stdlib's
    /// analysis copy beside the program: the program's own fns, what
    /// their calls reach, and the hooks and bus handlers of every locus
    /// they start (an instantiation, or a param field of a started
    /// locus). A copy's fn the program never reaches is not run, so it
    /// invokes nothing ([`AllocSummary::unbounded_invoked`]). `None` when
    /// every fn is the program's.
    pub reached: Option<BTreeSet<FnKey>>,
    /// The fns of the stdlib's analysis copy summarized beside the
    /// program; empty when every fn is the program's.
    pub analysis_copy: BTreeSet<FnKey>,
    /// The loci of the stdlib's analysis copy, likewise.
    pub analysis_copy_loci: BTreeSet<String>,
    /// The interfaces of the stdlib's analysis copy, likewise.
    pub analysis_copy_interfaces: BTreeSet<String>,
    /// The unresolved function values (a builtin or a stdlib path read
    /// as a value) only the stdlib's analysis copy takes: an alternative
    /// of a function-value dispatch the program alone does not have.
    pub analysis_copy_values: BTreeSet<String>,
    /// The program's bodies that are no fn's row, each summarized as a
    /// member of its declaration ([`DeclarationBody`]). No judgment reads
    /// them; the declaration dependents relation reads their call edges.
    pub declaration_bodies: Vec<DeclarationBody>,
}

/// A body of the program the reveal rule reads (`secret_reveal`) that is
/// no fn's row: an `on_failure` handler, a params block's initializers, a
/// constant's value, a `birth_check`, a perspective's members, a
/// synthesized hook, each wherever it is declared (a `module { }`'s
/// included), and every subexpression a walk does not descend into (a
/// callee that is no name). Each is walked as a row is, and kept
/// beside the rows rather than among them, so the judgments over the rows
/// see what they saw; the declaration dependents relation
/// (`hale_frontend::dependents`) reads its call edges as the declaration's
/// own (F.40 phase 3, X2), so a helper the body calls has the declaration
/// for a reader.
#[derive(Debug, Clone)]
pub struct DeclarationBody {
    /// The declaration the body is a member of, by its site: the locus
    /// for its `on_failure` or params, the perspective for its fn, the
    /// declaration of the body a subexpression sits in.
    pub declaration: SiteId,
    /// What the body is (`on_failure`, `params`, `const`, `birth_check`,
    /// `stable_when`, `fn` (a perspective's), `lifecycle` (a synthesized
    /// hook), `subexpression`).
    pub position: &'static str,
    pub summary: FnSummary,
}

impl AllocSummary {
    /// The row a call spelling `name` (a member of `locus`, or a free fn
    /// for `None`) reaches, as the walk resolves one.
    pub fn resolve(&self, locus: Option<&str>, name: &str) -> Option<&FnKey> {
        self.by_name.get(&(locus.map(str::to_string), name.to_string()))
    }

    /// Is this site's owning fn inside a `@bounded` locus? Drives
    /// "report by default" without the survey flag.
    pub fn owner_is_bounded_scope(&self, owner: &FnKey) -> bool {
        owner
            .locus
            .as_ref()
            .is_some_and(|l| self.bounded_loci.contains(l))
    }

    /// Whether a struct literal's path as written (an
    /// [`AllocKind::StructLit`]'s name) names a locus of the summary:
    /// by its name, or a stdlib locus by its full path.
    pub fn names_locus(&self, written: &str) -> bool {
        let segs: Vec<&str> = written.split("::").collect();
        let name = match segs.as_slice() {
            [one] => Some(*one),
            _ => crate::stdlib_bodies::mangled_locus_name(&segs),
        };
        name.is_some_and(|n| self.locus_shapes.contains_key(n))
    }

    /// Whether the fn `key` is the program's own, not the stdlib's
    /// analysis copy's.
    pub fn is_own(&self, key: &FnKey) -> bool {
        !self.analysis_copy.contains(key)
    }

    /// Whether the locus `name` is the program's own.
    pub fn is_own_locus(&self, name: &str) -> bool {
        !self.analysis_copy_loci.contains(name)
    }

    /// Whether an alternative of a function-value dispatch
    /// (`CallEdge::via_value`) is one only the stdlib's analysis copy
    /// supplies (its fn, or a leaf only it reads as a value): the
    /// program alone has no such alternative, and [`Self::own_rows`]
    /// leaves it out.
    pub fn copy_alternative(&self, e: &CallEdge) -> bool {
        e.via_value.is_some()
            && match &e.callee {
                Callee::Resolved(k) => self.analysis_copy.contains(k),
                Callee::Unresolved(n) => self.analysis_copy_values.contains(n),
            }
    }

    /// The function-value dispatches of `fs` that keep an alternative
    /// of the program's own ([`Self::copy_alternative`]): by group.
    pub fn value_groups_kept(&self, fs: &FnSummary) -> BTreeSet<u32> {
        fs.calls
            .iter()
            .filter(|c| c.via_value.is_some() && !self.copy_alternative(c))
            .filter_map(|c| c.dispatch_group)
            .collect()
    }

    /// The program's own rows, as the user-program readers (the model,
    /// the `@budget` engines, the artifact's rows) project them: the
    /// program's fns and loci only, and a call into the stdlib's
    /// analysis copy is the unresolved call it is when the program is
    /// summarized alone — the method's bare name, the receiver kept. A
    /// dispatch through an interface keeps its alternatives among the
    /// program's own loci, renumbered in key order; through the copy's
    /// interface it is no dispatch at all, and through the program's
    /// own with no conformer of its own left it is the dead site. The
    /// model is a user-program model and its `shape_hash` is the build
    /// and replay identity, so it reads these rows and the program
    /// alone decides it.
    pub fn own_rows(&self) -> AllocSummary {
        let copy = |k: &FnKey| self.analysis_copy.contains(k);
        // A group whose every alternative is the copy's, or whose
        // interface is the copy's, collapses to its written call.
        let mut collapsed: BTreeSet<u32> = BTreeSet::new();
        // A function-value dispatch (E5) keeps its alternatives among the
        // program's own fns and the unresolved leaves; with none left it
        // is the indirect call as written, as the program alone resolves
        // it to nothing of its own.
        let mut value_own: BTreeSet<u32> = BTreeSet::new();
        for f in self.fns.values().filter(|f| self.is_own(&f.key)) {
            value_own.extend(self.value_groups_kept(f));
        }
        for f in self.fns.values().filter(|f| self.is_own(&f.key)) {
            let mut own_alt: BTreeMap<u32, bool> = BTreeMap::new();
            for c in f.calls.iter().filter(|c| c.via_value.is_none()) {
                if let (Some(g), Callee::Resolved(k)) = (c.dispatch_group, &c.callee) {
                    let through_copy = c.via_interface.as_ref().is_some_and(|i| self.analysis_copy_interfaces.contains(i));
                    *own_alt.entry(g).or_default() |= !copy(k) && !through_copy;
                }
            }
            collapsed.extend(own_alt.into_iter().filter(|(_, own)| !own).map(|(g, _)| g));
        }
        let mut renumber: BTreeMap<u32, u32> = BTreeMap::new();
        let mut value_order: Vec<u32> = Vec::new();
        let mut emitted: BTreeSet<u32> = BTreeSet::new();
        let mut fns: BTreeMap<FnKey, FnSummary> = BTreeMap::new();
        for f in self.fns.values().filter(|f| self.is_own(&f.key)) {
            let mut calls: Vec<CallEdge> = Vec::with_capacity(f.calls.len());
            for c in &f.calls {
                let mut e = c.clone();
                if let (Some(written), Some(g)) = (&c.via_value, c.dispatch_group) {
                    if value_own.contains(&g) {
                        if self.copy_alternative(c) {
                            continue;
                        }
                        // Numbered below, after every interface group.
                        if !value_order.contains(&g) {
                            value_order.push(g);
                        }
                    } else {
                        if !emitted.insert(g) {
                            continue;
                        }
                        e.callee = Callee::Unresolved(written.clone());
                        e.indirect = true;
                        e.via_value = None;
                        e.via_local = None;
                        e.dispatch_group = None;
                    }
                    calls.push(e);
                    continue;
                }
                match (c.dispatch_group, &c.callee) {
                    (Some(g), Callee::Resolved(k)) if collapsed.contains(&g) => {
                        if !emitted.insert(g) {
                            continue;
                        }
                        let through_copy = c.via_interface.as_ref().is_some_and(|i| self.analysis_copy_interfaces.contains(i));
                        e.callee = Callee::Unresolved(k.fn_name.clone());
                        e.dispatch_group = None;
                        if through_copy {
                            e.via_interface = None;
                        }
                    }
                    (Some(_), Callee::Resolved(k)) if copy(k) => continue,
                    (Some(g), Callee::Resolved(_)) => {
                        let next = renumber.len() as u32;
                        e.dispatch_group = Some(*renumber.entry(g).or_insert(next));
                    }
                    (None, Callee::Resolved(k)) if copy(k) => {
                        e.callee = Callee::Unresolved(k.fn_name.clone());
                    }
                    (_, Callee::Unresolved(_)) => {
                        if c.via_interface.as_ref().is_some_and(|i| self.analysis_copy_interfaces.contains(i)) {
                            e.via_interface = None;
                        }
                    }
                    _ => {}
                }
                calls.push(e);
            }
            fns.insert(f.key.clone(), FnSummary { calls, ..f.clone() });
        }
        // The program alone numbers its function-value dispatches after
        // all of its interface dispatches, each in the fns' order.
        let first = renumber.len() as u32;
        for f in fns.values_mut() {
            for e in f.calls.iter_mut().filter(|e| e.via_value.is_some()) {
                if let Some(g) = e.dispatch_group {
                    let at = value_order.iter().position(|v| *v == g).expect("a kept value group is ordered");
                    e.dispatch_group = Some(first + at as u32);
                }
            }
        }
        let own_locus = |l: &String| self.is_own_locus(l);
        let own_key = |k: &FnKey| self.is_own(k);
        AllocSummary {
            eager_only_loci: self.eager_only_loci.iter().filter(|l| own_locus(l)).cloned().collect(),
            fns,
            by_name: self.by_name.iter().filter(|(_, k)| own_key(k)).map(|(n, k)| (n.clone(), k.clone())).collect(),
            bounded_loci: self.bounded_loci.iter().filter(|l| own_locus(l)).cloned().collect(),
            sync_holding_loci: self.sync_holding_loci.iter().filter(|l| own_locus(l)).cloned().collect(),
            sync_forms: self.sync_forms.iter().filter(|l| own_locus(l)).cloned().collect(),
            carries: self.carries.iter().filter(|(k, _)| own_key(k)).map(|(k, v)| (k.clone(), *v)).collect(),
            unbounded_fns: self.unbounded_fns.iter().filter(|k| own_key(k)).cloned().collect(),
            locus_shapes: self.locus_shapes.iter().filter(|(l, _)| own_locus(l)).map(|(l, s)| (l.clone(), s.clone())).collect(),
            reached: None,
            analysis_copy: BTreeSet::new(),
            analysis_copy_loci: BTreeSet::new(),
            analysis_copy_interfaces: BTreeSet::new(),
            declaration_bodies: self.declaration_bodies.clone(),
            analysis_copy_values: BTreeSet::new(),
        }
    }
}

/// Why a site's final verdict is unbounded — the diagnostic anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakReason {
    /// The allocation is directly inside an unbounded loop in its own body.
    InUnboundedLoop,
    /// The allocation's fn is *invoked* unboundedly (a bus handler, or
    /// reached through a call inside an unbounded loop), so even a
    /// once-per-call alloc accumulates.
    InvokedUnboundedly,
    /// A `Local` allocation of a fn with no scratch of its own, run once
    /// per iteration of an unbounded loop in a long-lived frame: it lands
    /// in that caller's arena, whose boundary is outside the loop (E3b).
    InCallersArena,
}

/// A confirmed unbounded-accumulation site (step 3 output → diagnostic).
#[derive(Debug, Clone)]
pub struct LeakSite {
    pub owner: FnKey,
    pub kind: AllocKind,
    pub escape: Escape,
    pub reason: LeakReason,
    pub span: Span,
}

impl AllocSummary {
    /// Fns reached under an unbounded-multiplicity context — the call-graph
    /// half of the bound solver. Seeded with bus handlers (per-message),
    /// then a fixed point: a resolved callee is invoked unboundedly if its
    /// caller is, or the call edge is inside an unbounded loop. Only the
    /// fns the program reaches ([`AllocSummary::reached`]) seed it or
    /// call: a stdlib loop the program never starts invokes nothing.
    pub fn unbounded_invoked(&self) -> BTreeSet<FnKey> {
        let runs = |f: &&FnSummary| self.reached.as_ref().is_none_or(|r| r.contains(&f.key));
        let mut set: BTreeSet<FnKey> = self
            .fns
            .values()
            .filter(runs)
            .filter(|f| f.entry == Some(EntryKind::BusHandler))
            .map(|f| f.key.clone())
            .collect();
        loop {
            let mut changed = false;
            for f in self.fns.values().filter(runs) {
                let caller_unbounded = set.contains(&f.key);
                for c in &f.calls {
                    if let Callee::Resolved(callee) = &c.callee {
                        let edge_unbounded = caller_unbounded || c.in_unbounded_loop;
                        if edge_unbounded
                            && self.fns.contains_key(callee)
                            && set.insert(callee.clone())
                        {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        set
    }

    /// M3 stage 5 gap A/B (2026-07-02, audit notes/unbounded-alloc-
    /// audit-2026-07-02.md): the set of frames whose allocations live
    /// in a LONG-LIVED arena with no per-call scratch — `main` and
    /// `run` bodies, plus (fixpoint) free fns reached from one: the
    /// empirical reclaim model says free-fn returns do NOT reclaim,
    /// so a free fn called from `run` allocates straight into run's
    /// lifetime arena. Member fns and bus handlers open a per-call
    /// method scratch, and a scratch-local free fn its own subregion
    /// ([`FnSummary::frees_at_return`]), so they STOP the propagation
    /// — a value consumed there dies at its exit.
    fn scratchless_longlived(&self) -> BTreeSet<FnKey> {
        let mut set: BTreeSet<FnKey> = self
            .fns
            .values()
            .filter(|f| {
                matches!(f.entry, Some(EntryKind::Main) | Some(EntryKind::Run))
            })
            .map(|f| f.key.clone())
            .collect();
        loop {
            let mut changed = false;
            for f in self.fns.values() {
                if !set.contains(&f.key) {
                    continue;
                }
                for c in &f.calls {
                    if let Callee::Resolved(callee) = &c.callee {
                        if callee.locus.is_none()
                            && self.fns.get(callee).is_some_and(|g| !g.frees_at_return())
                            && set.insert(callee.clone())
                        {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        set
    }

    /// The free fns of `scratchless` whose frame is their caller's arena
    /// ([`Frame::CallersArena`]) that run once per iteration of an
    /// unbounded loop in a long-lived frame: called inside such a loop
    /// of a `scratchless` frame, or (fixpoint) by one of them. Their
    /// allocations land in that frame's arena, whose boundary is outside
    /// the loop, so even a `Local` one accumulates across its
    /// iterations: `Local` is not scratch. A fn the summary did not
    /// classify is in none.
    fn repeated_in_longlived(&self, scratchless: &BTreeSet<FnKey>) -> BTreeSet<FnKey> {
        let lands_in_caller = |k: &FnKey| {
            scratchless.contains(k) && self.fns.get(k).is_some_and(|f| f.frame == Some(Frame::CallersArena))
        };
        let runs = |f: &&FnSummary| self.reached.as_ref().is_none_or(|r| r.contains(&f.key));
        let mut set: BTreeSet<FnKey> = BTreeSet::new();
        loop {
            let mut changed = false;
            for f in self.fns.values().filter(runs).filter(|f| scratchless.contains(&f.key)) {
                let repeated = set.contains(&f.key);
                for c in &f.calls {
                    if let Callee::Resolved(callee) = &c.callee {
                        if (repeated || c.in_unbounded_loop)
                            && lands_in_caller(callee)
                            && set.insert(callee.clone())
                        {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        set
    }

    /// A site's final verdict, folding in call-graph multiplicity: an
    /// accumulating site in an unboundedly-invoked fn is unbounded even if
    /// it's only once-per-call in its own body. Bus-dispatch reclaim stays
    /// bounded regardless (the value is freed each dispatch).
    ///
    /// M3 stage 5 gap A+B refinements (2026-07-02, audit-driven —
    /// 74% FP rate before, dominated by two over-approximations):
    ///
    /// GAP A — a `Returned` value only accumulates when a DIRECT
    /// caller consumes it in a scratch-less long-lived frame
    /// (`main`/`run`/free-fn-chain therefrom). Consumed inside a
    /// member fn or bus handler, it dies with that frame's per-call
    /// scratch. KNOWN HOLE (accepted, documented): a member fn that
    /// FORWARDS the value (returns it onward to run) deep-copies it
    /// into the caller arena and does accumulate — value-flow
    /// through frames isn't tracked, so that shape is missed.
    ///
    /// GAP B — a `Local` in an unbounded loop inside a fn WITH
    /// per-call scratch is bounded by the activation (reclaimed at
    /// method exit); only scratch-less frames turn in-loop locals
    /// into true accumulation. KNOWN HOLE: a `while true` loop that
    /// never exits inside a handler defeats the "dies at exit"
    /// argument; rare, and the old behavior flagged 155 bounded
    /// per-activation loops to catch it. A scratch-local free fn's
    /// subregion is such a scratch (F.40 phase 3, E3b).
    ///
    /// E3b — the boundary is judged relative to the loop: a `Local`
    /// in a frame with no boundary of its own (`repeated`, see
    /// [`AllocSummary::repeated_in_longlived`]) accumulates across a
    /// long-lived caller's unbounded loop, since its caller's arena
    /// holds it; one reclaimed at its fn's return does not.
    fn final_verdict(
        &self,
        owner: &FnKey,
        site: &AllocSite,
        unbounded: &BTreeSet<FnKey>,
        scratchless: &BTreeSet<FnKey>,
        repeated: &BTreeSet<FnKey>,
        callers: &BTreeMap<FnKey, BTreeSet<FnKey>>,
    ) -> SiteVerdict {
        let intra = site.verdict();
        // iris handoff P2.2: EAGER-ONLY loci — every instantiation is
        // a bare statement that dissolves (arena destroyed) at the
        // statement itself.
        //  (a) An instantiation SITE of such a locus in a parent
        //      loop reclaims per iteration — the fresh instance
        //      cannot outlive the statement.
        //  (b) A site INSIDE such a locus's own frames whose value
        //      stays in the instance (Local / StoredToSelf) is
        //      bounded by that statement-scoped lifetime. Returned /
        //      Sent escapes keep their normal analysis — they leave
        //      the instance.
        // `while true` inside the eager locus still disqualifies
        // (the statement never completes).
        // (a) applies regardless of the PARENT loop's kind — even
        // under `while true`, the eager child drains/dissolves and
        // its arena is destroyed at the statement, every iteration.
        if let AllocKind::StructLit(n) = &site.kind {
            if self.eager_only_loci.contains(n) {
                return SiteVerdict::PerIterationReclaim;
            }
        }
        if !site.in_infinite_loop {
            if let Some(l) = &owner.locus {
                if self.eager_only_loci.contains(l)
                    && matches!(
                        site.escape,
                        Escape::Local | Escape::StoredToSelf
                    )
                {
                    return SiteVerdict::PerIterationReclaim;
                }
            }
        }
        let owner_scratchless = scratchless.contains(owner);
        match intra {
            SiteVerdict::AccumulatesUnbounded => {
                // GAP B: in-loop Local in a scratch-ful frame dies
                // at method exit — bounded per activation. EXCEPT
                // inside a literal `while true`, where the method
                // never exits and the scratch never destroys.
                if matches!(site.escape, Escape::Local)
                    && !owner_scratchless
                    && !site.in_infinite_loop
                {
                    SiteVerdict::PerIterationReclaim
                } else {
                    intra
                }
            }
            SiteVerdict::PerIterationReclaim => intra,
            // `Local` is not scratch: in a frame with no boundary of its
            // own, run once per iteration of a long-lived frame's
            // unbounded loop, the allocation lands in that frame's arena,
            // whose boundary is outside the loop.
            // A locus instantiation is not a value in the caller's
            // arena: the instance has an arena of its own, which the
            // fn's dissolve frame destroys at its return.
            _ if site.escape == Escape::Local
                && repeated.contains(owner)
                && site.reclaim.accumulates_in_loop(LoopAt::Caller)
                && !matches!(&site.kind, AllocKind::StructLit(n) if self.names_locus(n)) =>
            {
                SiteVerdict::AccumulatesUnbounded
            }
            _ if unbounded.contains(owner)
                && site.reclaim.accumulates_in_loop(LoopAt::Caller)
                && site.escape.persists_across_calls() =>
            {
                match site.escape {
                    // GAP A: Returned — accumulate only when some
                    // direct caller consumes in a scratch-less
                    // long-lived frame.
                    Escape::Returned => {
                        let consumed_dangerously = callers
                            .get(owner)
                            .map(|cs| {
                                cs.iter().any(|c| scratchless.contains(c))
                            })
                            .unwrap_or(false)
                            || owner_scratchless;
                        if consumed_dangerously {
                            SiteVerdict::AccumulatesUnbounded
                        } else {
                            SiteVerdict::PerIterationReclaim
                        }
                    }
                    // Gap D (2026-07-17): a retired whole-field struct
                    // replace reclaims at each invocation's activation
                    // boundary (Gap A anchor retirement, RSS-proven
                    // flat) — unbounded INVOCATION no longer means
                    // unbounded accumulation. Needs the activation
                    // boundary: a scratchless owner never flushes its
                    // pending retires, so it keeps the conservative
                    // verdict.
                    Escape::StoredToSelf
                        if site.retired_store && !owner_scratchless =>
                    {
                        SiteVerdict::PerIterationReclaim
                    }
                    _ => SiteVerdict::AccumulatesUnbounded,
                }
            }
            _ => intra,
        }
    }

    /// Every site whose final verdict is unbounded accumulation — the
    /// step-3 result the diagnostic emits.
    pub fn leak_sites(&self) -> Vec<LeakSite> {
        // M3 stage 5 gap E (2026-07-02): a bundle with NO long-lived
        // entry point — no `run` loop, no bus handler — is a
        // run-to-exit program (a script, a smoke binary, a lib
        // checked standalone). Per the tool's own philosophy ("a
        // memory-bound proof only means something for long-lived
        // processes"), nothing in it can leak in the sense this
        // analysis measures. A LIB's latent leaks still surface at
        // every consumer's whole-program check, where the
        // consumer's run/handlers are present — that's the right
        // place: the same lib fn may be one-shot in a script and
        // hot in a daemon.
        // Refinement: only suppress ACTUAL run-to-exit programs (a
        // `main` present, nothing long-lived). A LIB checked
        // standalone has no main at all — its warnings stay, because
        // per-dir consumer checks don't re-bundle vendored libs and
        // would otherwise never surface the lib's real leaks
        // (pond/websocket's per-message stores were the case in
        // point).
        // The program's own entries, never the stdlib's analysis copy's:
        // the copy always carries `run` hooks, so counting them would
        // switch the rule off for every program.
        let own = || self.fns.values().filter(|f| self.is_own(&f.key));
        let has_long_lived_entry = own().any(|f| {
            matches!(
                f.entry,
                Some(EntryKind::Run) | Some(EntryKind::BusHandler)
            )
        });
        let has_main = own().any(|f| matches!(f.entry, Some(EntryKind::Main)));
        if has_main && !has_long_lived_entry {
            return Vec::new();
        }
        let unbounded = self.unbounded_invoked();
        let scratchless = self.scratchless_longlived();
        let repeated = self.repeated_in_longlived(&scratchless);
        let mut callers: BTreeMap<FnKey, BTreeSet<FnKey>> = BTreeMap::new();
        for f in self.fns.values() {
            for c in &f.calls {
                if let Callee::Resolved(callee) = &c.callee {
                    callers
                        .entry(callee.clone())
                        .or_default()
                        .insert(f.key.clone());
                }
            }
        }
        let mut out = Vec::new();
        for f in self.fns.values() {
            // `@unbounded` is the acknowledged carve-out — its sites never
            // surface, with or without the survey flag, in or out of a
            // `@bounded` locus.
            if self.unbounded_fns.contains(&f.key) {
                continue;
            }
            for s in &f.sites {
                if self.final_verdict(&f.key, s, &unbounded, &scratchless, &repeated, &callers) == SiteVerdict::AccumulatesUnbounded {
                    let reason = if matches!(s.verdict(), SiteVerdict::AccumulatesUnbounded) {
                        LeakReason::InUnboundedLoop
                    } else if s.escape == Escape::Local {
                        LeakReason::InCallersArena
                    } else {
                        LeakReason::InvokedUnboundedly
                    };
                    out.push(LeakSite {
                        owner: f.key.clone(),
                        kind: s.kind.clone(),
                        escape: s.escape,
                        reason,
                        span: s.span,
                    });
                }
            }
        }
        out
    }

    /// Human-readable dump for `--dump-alloc-summary`: the program's own
    /// fns and loci, judged over the whole summary (the stdlib's analysis
    /// copy beside them included).
    pub fn render(&self) -> String {
        let unbounded = self.unbounded_invoked();
        let scratchless = self.scratchless_longlived();
        let repeated = self.repeated_in_longlived(&scratchless);
        let mut callers: BTreeMap<FnKey, BTreeSet<FnKey>> = BTreeMap::new();
        for f in self.fns.values() {
            for c in &f.calls {
                if let Callee::Resolved(callee) = &c.callee {
                    callers
                        .entry(callee.clone())
                        .or_default()
                        .insert(f.key.clone());
                }
            }
        }
        let mut out = String::new();
        out.push_str("# allocation summary (GH #18 item 1, steps 1-3 + D1 slot shape)\n");
        let own_fns: Vec<&FnSummary> = self.fns.values().filter(|f| self.is_own(&f.key)).collect();
        out.push_str(&format!(
            "# {} fns, {} entry points, {} invoked-unboundedly\n\n",
            own_fns.len(),
            own_fns.iter().filter(|f| f.entry.is_some()).count(),
            unbounded.iter().filter(|k| self.is_own(k)).count()
        ));
        let own_shapes: Vec<&LocusShape> =
            self.locus_shapes.values().filter(|s| self.is_own_locus(&s.name)).collect();
        // D1: per-locus storage shape — the capacity slots, `@form`, and
        // projection cap the bound solver (D2) will read.
        for shape in own_shapes.iter().copied() {
            if shape.capacity_slots.is_empty()
                && shape.form.is_none()
                && shape.recognition_cap.is_none()
            {
                continue;
            }
            out.push_str(&format!("locus {} [shape]\n", shape.name));
            for slot in &shape.capacity_slots {
                let kind = match slot.kind {
                    CapacitySlotKind::Pool => "pool",
                    CapacitySlotKind::Heap => "heap",
                };
                out.push_str(&format!("    slot  {} ({})\n", slot.name, kind));
            }
            if let Some(form) = &shape.form {
                let cap = form.cap.map(|c| format!(", cap={}", c)).unwrap_or_default();
                out.push_str(&format!("    form  @form({}{})\n", form.name, cap));
            }
            if let Some(cap) = shape.recognition_cap {
                out.push_str(&format!("    proj  recognition(cap={})\n", cap));
            }
        }
        if !own_shapes.iter().all(|s| {
            s.capacity_slots.is_empty() && s.form.is_none() && s.recognition_cap.is_none()
        }) {
            out.push('\n');
        }
        for f in own_fns {
            let mut tags = Vec::new();
            if let Some(e) = f.entry {
                tags.push(format!(
                    "entry: {} ({})",
                    e.label(),
                    if e.one_shot() { "one-shot" } else { "per-message" }
                ));
            }
            if unbounded.contains(&f.key) {
                tags.push("invoked-unboundedly".to_string());
            }
            // The frame, beside each site's escape: a scratch-local fn
            // frees its `Local` allocations at return, unless recursive.
            if let Some(Frame::ScratchLocal { recursive }) = f.frame {
                tags.push(if recursive { "scratch-local, recursive" } else { "scratch-local" }.to_string());
            }
            let tag = if tags.is_empty() { String::new() } else { format!("   [{}]", tags.join(", ")) };
            out.push_str(&format!("fn {}{}\n", f.key.display(), tag));
            if f.sites.is_empty() && f.calls.is_empty() && f.loops.is_empty() {
                out.push_str("    (no allocations, calls, or loops)\n");
            }
            for l in &f.loops {
                out.push_str(&format!(
                    "    loop  {} depth={} @{}..{}\n",
                    l.kind.label(),
                    l.depth,
                    l.span.start.0,
                    l.span.end.0
                ));
            }
            for s in &f.sites {
                let v = self.final_verdict(
                    &f.key, s, &unbounded, &scratchless, &repeated, &callers,
                );
                let flag = if matches!(v, SiteVerdict::AccumulatesUnbounded) {
                    "  <-- LEAK"
                } else {
                    ""
                };
                let field = s
                    .target_field
                    .as_ref()
                    .map(|f| format!(" ->self.{}", f))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "    alloc {:<16} {:<20} {:<22} {:<22} @{}..{}{}{}\n",
                    s.kind.label(),
                    s.escape.label(),
                    v.label(),
                    s.reclaim.label(),
                    s.span.start.0,
                    s.span.end.0,
                    field,
                    flag
                ));
            }
            for c in &f.calls {
                let tgt = match &c.callee {
                    Callee::Resolved(k) => k.display(),
                    Callee::Unresolved(n) => format!("<unresolved: {}>", n),
                };
                let slot = c
                    .receiver_slot
                    .as_ref()
                    .map(|s| format!(" recv=self.{}", s))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "    call  {} loop_depth={} result={}{}\n",
                    tgt,
                    c.loop_depth,
                    c.escape.label(),
                    slot
                ));
            }
        }
        out
    }
}

/// Entry point. Walks every free fn, locus method, and lifecycle hook in
/// the bundle and returns the per-fn allocation summary + call graph.
/// #345: the classes an `@effects(is: {…})` clause declares.
fn carried_by(
    effects: &[hale_syntax::ast::EffectAssert],
    classes: &crate::effect_classes::EffectClassTable,
) -> crate::stdlib_surface::EffectSet {
    let mut acc = crate::stdlib_surface::EffectSet::PURE;
    for a in effects {
        if let hale_syntax::ast::EffectAssert::Carries(cs) = a {
            for c in cs {
                acc = acc.union(classes.mask(*c));
            }
        }
    }
    acc
}

/// The `alloc_summary` family's producer: the summary of the bundle's
/// checked programs with the stdlib's analysis copy beside them, each
/// with its own identities, and cross-seed `alias::name` calls resolved
/// through the bundle's import renames. A snapshot runs it once
/// (`hale_frontend::snapshot::Snapshot::demand_alloc_summary`); a
/// bundle no snapshot holds runs it once for itself.
pub fn derive_alloc_summary(bundle: &crate::symbol::Bundle<'_>) -> AllocSummary {
    let mut identified: Vec<(&Program, &crate::snapshot::Snapshot)> =
        bundle.programs.values().map(|p| (*p, &bundle.snapshot)).collect();
    // The stdlib's analysis copy, with its own identities: a call through
    // a stdlib handle has a body to walk (`stdlib_bodies`).
    if let (Some(program), Some(ids)) =
        (crate::stdlib_bodies::program(), crate::stdlib_bodies::identities())
    {
        identified.push((program, ids));
    }
    summarize_identified(&identified, &bundle.import_renames)
}

/// Every top-level locus of `programs` with a param whose type is one of
/// `sync_forms`: it holds that form, so a call into it can take the
/// form's lock.
fn collect_sync_holding_loci(
    programs: &[&Program],
    sync_forms: &BTreeSet<String>,
    out: &mut BTreeSet<String>,
) {
    for program in programs {
        for item in flat_decls(&program.items) {
            let TopDecl::Locus(l) = item else { continue };
            for m in &l.members {
                let LocusMember::Params(pb) = m else { continue };
                for prm in &pb.params {
                    let Some(TypeExpr::Named { path, .. }) = &prm.ty else {
                        continue;
                    };
                    if path
                        .segments
                        .last()
                        .is_some_and(|s| sync_forms.contains(&s.name))
                    {
                        out.insert(l.name.name.clone());
                    }
                }
            }
        }
    }
}

impl AllocSummary {
    /// The forms of `programs` their form rows say carry a `sync`
    /// discipline (F.40 phase 3, C1: [`crate::form_rows::FormRows::synchronizes`],
    /// inference's pick included), added to the stdlib analysis copy's the
    /// summary holds, and the loci holding them. The effects certificate
    /// engine reads both: the summary holds none of the program's own.
    pub fn add_sync_forms(&mut self, programs: &[&Program], forms: &crate::form_rows::FormRows) {
        for program in programs {
            for item in flat_decls(&program.items) {
                if let TopDecl::Locus(l) = item {
                    if forms.synchronizes(l) {
                        self.sync_forms.insert(l.name.name.clone());
                    }
                }
            }
        }
        collect_sync_holding_loci(programs, &self.sync_forms, &mut self.sync_holding_loci);
    }

    /// [`AllocSummary::add_sync_forms`] over a shared summary (the
    /// `alloc_summary` family's): the summary itself when the rows add
    /// no form it does not already hold, else a copy with them added.
    pub fn with_sync_forms(
        &self,
        programs: &[&Program],
        forms: &crate::form_rows::FormRows,
    ) -> std::borrow::Cow<'_, AllocSummary> {
        let adds = programs.iter().flat_map(|p| flat_decls(&p.items)).any(|item| {
            matches!(item, TopDecl::Locus(l)
                if forms.synchronizes(l) && !self.sync_forms.contains(&l.name.name))
        });
        if !adds {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut summary = self.clone();
        summary.add_sync_forms(programs, forms);
        std::borrow::Cow::Owned(summary)
    }
}

/// The summary of programs minted by different snapshots — a bundle's
/// programs beside the stdlib's (`stdlib_bodies`), each with its own
/// identities — with the bundle's cross-seed import renames.
///
/// A call written `alias::name` reaches the callgraph as a qualified
/// path, while the imported decl was merged under a MANGLED symbol.
/// Without the table the two never meet, so every cross-seed call was
/// an unresolved edge — which is why effect assertions, budgets and
/// taint all stopped dead at a seed boundary while still reporting
/// success. Codegen has always had this table; the analysis phases
/// did not.
pub fn summarize_identified(
    identified: &[(&Program, &crate::snapshot::Snapshot)],
    import_renames: &[(Vec<String>, String)],
) -> AllocSummary {
    let programs: Vec<&Program> = identified.iter().map(|(p, _)| *p).collect();
    let programs = programs.as_slice();
    // Each seed's names resolve in its own scope. The programs minted
    // with one set of identities are one scope (a bundle's programs; the
    // stdlib's analysis copy beside them is another): a body's bare
    // free-fn name names a fn of its own scope, so a stdlib body's
    // builtin `count(...)` is the builtin, never a user fn that happens
    // to be called `count`. The import renames are the bundle's names;
    // the stdlib's copy imports nothing.
    let mut scopes: Vec<&crate::snapshot::Snapshot> = Vec::new();
    for (_, ids) in identified {
        if !scopes.iter().any(|s| std::ptr::eq(*s, *ids)) {
            scopes.push(ids);
        }
    }
    let scope_index = |ids: &crate::snapshot::Snapshot| {
        scopes.iter().position(|s| std::ptr::eq(*s, ids)).expect("every program's identities are a scope")
    };
    let mut scope_fns: Vec<BTreeSet<String>> = vec![BTreeSet::new(); scopes.len()];
    {
        fn collect_fns(items: &[TopDecl], out: &mut BTreeSet<String>) {
            for item in items {
                match item {
                    TopDecl::Fn(f) => {
                        out.insert(f.name.name.clone());
                    }
                    TopDecl::Module(m) => collect_fns(&m.items, out),
                    _ => {}
                }
            }
        }
        for (program, ids) in identified {
            collect_fns(&program.items, &mut scope_fns[scope_index(ids)]);
        }
    }
    let is_stdlib_copy = |ids: &crate::snapshot::Snapshot| {
        crate::stdlib_bodies::identities().is_some_and(|s| std::ptr::eq(s, ids))
    };
    // What the checked programs declare. A program that is stdlib source
    // itself (`hale check` over a stdlib file) declares what the analysis
    // copy beside it declares; the program's declaration is the row, and
    // the copy's of the same name stays out.
    //
    // Every declaration pass below walks `module { … }` nesting
    // (`flat_decls`): a module is a namespace, not an analysis boundary
    // (GH #764), and the resolver keys a nested declaration by its bare
    // name, so the summary keys it the same way.
    let mut declared_by_program: BTreeSet<String> = BTreeSet::new();
    for (program, ids) in identified {
        if is_stdlib_copy(ids) {
            continue;
        }
        for item in flat_decls(&program.items) {
            match item {
                TopDecl::Fn(f) => declared_by_program.insert(f.name.name.clone()),
                TopDecl::Locus(l) => declared_by_program.insert(l.name.name.clone()),
                TopDecl::Interface(i) => declared_by_program.insert(i.name.name.clone()),
                _ => false,
            };
        }
    }
    let shadowed = |ids: &crate::snapshot::Snapshot, item: &TopDecl| {
        is_stdlib_copy(ids)
            && match item {
                TopDecl::Fn(f) => declared_by_program.contains(&f.name.name),
                TopDecl::Locus(l) => declared_by_program.contains(&l.name.name),
                TopDecl::Interface(i) => declared_by_program.contains(&i.name.name),
                _ => false,
            }
    };
    // #345: what each `@effects(is: {…})` declares, through the bundle's
    // one class table.
    let classes = crate::effect_classes::EffectClassTable::of(programs);
    // Phase 1 — collect every body with its key + entry classification.
    // For loci we first gather the set of bus-handler method names so a
    // method referenced by `subscribe ... -> handler` is tagged BusHandler.
    // The trailing `Vec<(String, String)>` seeds each body's var→type map
    // from its params (D2); then the identities of the program the body
    // is in, whether it is an `@hot` fn and a mode, and the declaration
    // it is a member of, and every param's name (the outermost of its
    // bindings). A body's place in the list is its `decl_index`.
    type BodyEntry<'i> = (FnKey, Block, Option<EntryKind>, Option<String>, Vec<(String, String)>, Vec<String>, Vec<(String, String)>, &'i crate::snapshot::Snapshot, (bool, bool), NodeId, Vec<String>);
    let mut bodies: Vec<BodyEntry> = Vec::new();
    // What a call spelling (locus, name) resolves to: the row of the last
    // declaration of the name.
    let mut known: Known = BTreeMap::new();
    let universe_of = |ids: &crate::snapshot::Snapshot| {
        if is_stdlib_copy(ids) {
            SiteUniverse::StdlibAnalysis
        } else {
            SiteUniverse::User
        }
    };
    // GH #18 item 1 — the `@bounded` / `@unbounded` opt-in/carve-out sets.
    // GH #265: every declared locus type name (spawn-site detection).
    let mut locus_type_names: BTreeSet<String> = BTreeSet::new();
    // GH #533: every declared interface name, so an interface-typed
    // slot keeps its DECLARED type and dispatch fans to every
    // conformer instead of binding to the default literal.
    let mut interface_names: BTreeSet<String> = BTreeSet::new();
    for p in programs {
        for item in flat_decls(&p.items) {
            match item {
                TopDecl::Locus(l) => {
                    locus_type_names.insert(l.name.name.clone());
                }
                TopDecl::Interface(i) => {
                    interface_names.insert(i.name.name.clone());
                }
                _ => {}
            }
        }
    }
    let mut bounded_loci: BTreeSet<String> = BTreeSet::new();
    let mut sync_holding_loci: BTreeSet<String> = BTreeSet::new();
    let mut sync_forms: BTreeSet<String> = BTreeSet::new();
    let mut carries: BTreeMap<FnKey, crate::stdlib_surface::EffectSet> = BTreeMap::new();
    let mut unbounded_fns: BTreeSet<FnKey> = BTreeSet::new();
    let mut analysis_copy_loci: BTreeSet<String> = BTreeSet::new();
    // Phase D / D1 — the per-locus storage shape.
    let mut locus_shapes: BTreeMap<String, LocusShape> = BTreeMap::new();
    // Phase D / D2 — per-locus param field → declared type name.
    let mut locus_field_types: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    // Gap D — struct types whose fields are ALL scalar / String: a
    // whole-field replace of one fully reclaims via anchor retirement
    // (Gap A). Conservative: any Bytes / nested compound / array /
    // view field disqualifies (those leaves don't retire yet).
    let mut retirable_structs: BTreeSet<String> = BTreeSet::new();
    // 2026-07-01 — per-locus scalar-[T; N] param fields (inline layout).
    let mut locus_inline_arrays: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // iris handoff P2.1 (2026-07-27): top-level `const` ints, so a
    // `while i < NET_SLOTS * WINDOW` ceiling const-folds to a real
    // bound instead of ranking as a runtime `while`.
    let mut const_ints: BTreeMap<String, i64> = BTreeMap::new();
    for program in programs {
        for item in flat_decls(&program.items) {
            if let TopDecl::Const(c) = item {
                if let Some(v) =
                    const_int_eval(&c.value, &const_ints)
                {
                    const_ints.insert(c.name.name.clone(), v);
                }
            }
        }
    }

    // iris handoff P2.2 (2026-07-27): loci whose every instantiation
    // is EAGER — a bare statement `L { ... };` that drains/dissolves
    // (arena destroyed) at the statement itself. Their per-instance
    // region accumulation is bounded by one parent statement, and an
    // instantiation site in a parent loop reclaims per iteration.
    // Conservative subtraction: any subscription (defers to scope
    // exit), any use as a param-field type (parent-owned lifetime),
    // any let-binding of the literal (defers to method exit), any
    // appearance as an accept() param type, or `main` status
    // disqualifies.
    let mut eager_only_loci: BTreeSet<String> = BTreeSet::new();
    {
        let mut all: BTreeSet<String> = BTreeSet::new();
        let mut deferred: BTreeSet<String> = BTreeSet::new();
        fn has_while_true(b: &Block) -> bool {
            b.stmts.iter().any(|st| match st {
                Stmt::While { cond, body, .. } => {
                    matches!(cond, Expr::Literal(Literal::Bool(true), _))
                        || has_while_true(body)
                }
                Stmt::For { body, .. } => has_while_true(body),
                Stmt::If(i) => {
                    fn hif(i: &IfStmt) -> bool {
                        if has_while_true(&i.then_block) {
                            return true;
                        }
                        match i.else_block.as_deref() {
                            Some(ElseBranch::Else(b)) => has_while_true(b),
                            Some(ElseBranch::ElseIf(i2)) => hif(i2),
                            None => false,
                        }
                    }
                    hif(i)
                }
                Stmt::Block(b2) => has_while_true(b2),
                _ => false,
            })
        }
        fn scan_lets_if(i: &IfStmt, out: &mut BTreeSet<String>) {
            scan_lets(&i.then_block, out);
            if let Some(e) = &i.else_block {
                match e.as_ref() {
                    ElseBranch::Else(b) => scan_lets(b, out),
                    ElseBranch::ElseIf(i2) => scan_lets_if(i2, out),
                }
            }
        }
        fn scan_lets(b: &Block, out: &mut BTreeSet<String>) {
            for st in &b.stmts {
                match st {
                    Stmt::Let { value, .. } => {
                        if let Expr::Struct { path, .. } = value {
                            if path.segments.len() == 1 {
                                out.insert(
                                    path.segments[0].name.clone(),
                                );
                            }
                        }
                    }
                    Stmt::While { body, .. } => scan_lets(body, out),
                    Stmt::For { body, .. } => scan_lets(body, out),
                    Stmt::If(i) => scan_lets_if(i, out),
                    Stmt::Block(b2) => scan_lets(b2, out),
                    _ => {}
                }
            }
        }
        for program in programs {
            for item in flat_decls(&program.items) {
                let TopDecl::Locus(l) = item else { continue };
                all.insert(l.name.name.clone());
                if l.is_main {
                    deferred.insert(l.name.name.clone());
                }
                for member in &l.members {
                    match member {
                        LocusMember::Bus(_) => {
                            let has_sub =
                                l.members.iter().any(|m| match m {
                                    LocusMember::Bus(bb) => {
                                        bb.members.iter().any(|bm| {
                                            matches!(
                                                bm,
                                                BusMember::Subscribe {
                                                    ..
                                                }
                                            )
                                        })
                                    }
                                    _ => false,
                                });
                            if has_sub {
                                deferred.insert(l.name.name.clone());
                            }
                        }
                        LocusMember::Params(pb) => {
                            for p in &pb.params {
                                if let Some(ty) = &p.ty {
                                    if let Some(n) =
                                        single_named_type_name(ty)
                                    {
                                        deferred.insert(n);
                                    }
                                }
                            }
                        }
                        LocusMember::Lifecycle(lc) => {
                            for p in &lc.params {
                                if let Some(n) =
                                    single_named_type_name(&p.ty)
                                {
                                    deferred.insert(n);
                                }
                            }
                            scan_lets(&lc.body, &mut deferred);
                            // A `while true` anywhere in the locus's
                            // own bodies means an eager statement may
                            // never complete — no statement-scoped
                            // lifetime to lean on.
                            if has_while_true(&lc.body) {
                                deferred.insert(l.name.name.clone());
                            }
                        }
                        LocusMember::Fn(f) => {
                            scan_lets(&f.body, &mut deferred);
                            if has_while_true(&f.body) {
                                deferred.insert(l.name.name.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for program in programs {
            for item in flat_decls(&program.items) {
                if let TopDecl::Fn(f) = item {
                    scan_lets(&f.body, &mut deferred);
                }
            }
        }
        for l in all {
            if !deferred.contains(&l) {
                eager_only_loci.insert(l);
            }
        }
    }

    for (program, ids) in identified {
        let ids: &crate::snapshot::Snapshot = ids;
        let universe = universe_of(ids);
        // The stdlib's analysis copy's form rows: its own universe's
        // (`stdlib_bodies::forms`). A program's own forms are its
        // snapshot's rows, which the effects engine adds
        // (`add_sync_forms`), so nothing here reads a `sync =` argument.
        let copy_forms = is_stdlib_copy(ids).then(crate::stdlib_bodies::forms).flatten();
        for item in flat_decls(&program.items).filter(|item| !shadowed(ids, item)) {
            match item {
                TopDecl::Fn(decl) => {
                    let key = FnKey::free_fn(DeclId::of(universe, decl.id), decl.name.name.clone());
                    {
                        let c = carried_by(&decl.effects, &classes);
                        if c.0 != 0 {
                            carries.insert(key.clone(), c);
                        }
                    }
                    let entry = if decl.name.name == "main" { Some(EntryKind::Main) } else { None };
                    if decl.unbounded {
                        unbounded_fns.insert(key.clone());
                    }
                    known.insert((None, key.fn_name.clone()), key.clone());
                    bodies.push((key, decl.body.clone(), entry, None, param_var_types(&decl.params), fn_typed_params(&decl.params), param_var_elem_types(&decl.params), ids, (decl.hot, false), decl.id, param_names(&decl.params)));
                }
                TopDecl::Type(td) => {
                    if let TypeDeclBody::Struct(fields) = &td.body {
                        let all_retirable = fields.iter().all(|f| {
                            matches!(
                                &f.ty,
                                TypeExpr::Primitive(
                                    PrimType::Int
                                        | PrimType::Uint
                                        | PrimType::Float
                                        | PrimType::Decimal
                                        | PrimType::String
                                        | PrimType::Bool
                                        | PrimType::Time
                                        | PrimType::Duration,
                                    _
                                )
                            )
                        });
                        if all_retirable {
                            retirable_structs.insert(td.name.name.clone());
                        }
                    }
                }
                TopDecl::Locus(l) => {
                    let locus = l.name.name.clone();
                    if is_stdlib_copy(ids) {
                        analysis_copy_loci.insert(locus.clone());
                    }
                    if l.bounded {
                        bounded_loci.insert(locus.clone());
                    }
                    if copy_forms.as_ref().is_some_and(|forms| forms.synchronizes(l)) {
                        sync_forms.insert(locus.clone());
                    }
                    locus_shapes.insert(locus.clone(), locus_shape_of(l));
                    locus_field_types.insert(
                        locus.clone(),
                        locus_param_field_types(
                            l,
                            &locus_type_names,
                            &interface_names,
                        ),
                    );
                    locus_inline_arrays
                        .insert(locus.clone(), locus_inline_array_fields(l));
                    let handlers = bus_handler_names(l);
                    for member in &l.members {
                        match member {
                            // A `mode` body was never collected, so
                            // its callees were invisible to the
                            // callgraph and `@no_syscall` certified a
                            // path straight through one. Modes are
                            // called like methods (`self.bulk()`), so
                            // they key the same way.
                            LocusMember::Mode(md) => {
                                let name = match md.kind {
                                    ModeKind::Bulk => "bulk",
                                    ModeKind::Harmonic => "harmonic",
                                    ModeKind::Resolution => "resolution",
                                };
                                let key = FnKey::method(
                                    DeclId::of(universe, md.id),
                                    locus.clone(),
                                    name.to_string(),
                                );
                                known.insert((key.locus.clone(), key.fn_name.clone()), key.clone());
                                bodies.push((
                                    key,
                                    md.body.clone(),
                                    None,
                                    Some(locus.clone()),
                                    param_var_types(&md.params),
                                    fn_typed_params(&md.params),
                                    param_var_elem_types(&md.params),
                                    ids,
                                    (false, true),
                                    l.id,
                                    param_names(&md.params),
                                ));
                            }
                            LocusMember::Fn(decl) => {
                                let key =
                                    FnKey::method(DeclId::of(universe, decl.id), locus.clone(), decl.name.name.clone());
                                let c = carried_by(&decl.effects, &classes);
                                if c.0 != 0 {
                                    carries.insert(key.clone(), c);
                                }
                                let entry = if handlers.contains(&decl.name.name) {
                                    Some(EntryKind::BusHandler)
                                } else {
                                    None
                                };
                                if decl.unbounded {
                                    unbounded_fns.insert(key.clone());
                                }
                                known.insert((key.locus.clone(), key.fn_name.clone()), key.clone());
                                bodies.push((
                                    key,
                                    decl.body.clone(),
                                    entry,
                                    Some(locus.clone()),
                                    param_var_types(&decl.params),
                                    fn_typed_params(&decl.params),
                                    param_var_elem_types(&decl.params),
                                    ids,
                                    (decl.hot, false),
                                    l.id,
                                    param_names(&decl.params),
                                ));
                            }
                            // The empty `run` a locus that declares none
                            // is given has no body to summarize, and the
                            // summary's keys are the model's functions:
                            // it lists the hooks the author wrote.
                            LocusMember::Lifecycle(lc) if !lc.synthesized => {
                                let (name, entry) = lifecycle_key(lc.kind);
                                let key = FnKey::method(DeclId::of(universe, lc.id), locus.clone(), name);
                                if lc.unbounded {
                                    unbounded_fns.insert(key.clone());
                                }
                                known.insert((key.locus.clone(), key.fn_name.clone()), key.clone());
                                bodies.push((
                                    key,
                                    lc.body.clone(),
                                    Some(entry),
                                    Some(locus.clone()),
                                    param_var_types(&lc.params),
                                    fn_typed_params(&lc.params),
                                    param_var_elem_types(&lc.params),
                                    ids,
                                    (false, false),
                                    l.id,
                                    param_names(&lc.params),
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // The program's bodies that are no row ([`DeclarationBody`]), with
    // what a row's walk is given: the stdlib's analysis copy has none the
    // relation reads.
    let mut member_bodies: Vec<(SiteId, MemberBody, &crate::snapshot::Snapshot)> = Vec::new();
    for (program, ids) in identified {
        if is_stdlib_copy(ids) {
            continue;
        }
        let mut found = Vec::new();
        for item in flat_decls(&program.items) {
            member_bodies_of(item, &mut found);
        }
        for (decl, body) in found {
            if let Some(site) = ids.site_id(decl) {
                member_bodies.push((site, body, ids));
            }
        }
    }

    // D2: type name → `@form(...)` name, for receiver-form lookup.
    let form_of: BTreeMap<String, String> = locus_shapes
        .iter()
        .filter_map(|(name, shape)| shape.form.as_ref().map(|f| (name.clone(), f.name.clone())))
        .collect();
    let empty_fields: BTreeMap<String, String> = BTreeMap::new();
    let empty_inline_arrays: BTreeSet<String> = BTreeSet::new();

    // `alias::name` -> mangled symbol, for cross-seed call resolution.
    let rename_map: BTreeMap<String, String> = import_renames
        .iter()
        .map(|(segs, mangled)| (segs.join("::"), mangled.clone()))
        .collect();
    let no_renames: BTreeMap<String, String> = BTreeMap::new();

    // #382 receiver-typing: struct TYPE field -> type-name map (a
    // chained receiver may pass through a plain struct: `route.
    // handler` where `route` is a struct value), and per-locus
    // ELEMENT types for iterables (array-typed params fields +
    // capacity slots), so a `for child in self.children` binder
    // gets a type instead of landing as an untypeable receiver.
    let mut struct_field_types: BTreeMap<String, BTreeMap<String, String>> =
        BTreeMap::new();
    let mut locus_elem_types: BTreeMap<String, BTreeMap<String, String>> =
        BTreeMap::new();
    {
        fn elem_type_name(te: &TypeExpr) -> Option<String> {
            match te {
                TypeExpr::Array { elem, .. } => type_expr_name(elem),
                _ => None,
            }
        }
        fn collect_maps(
            items: &[TopDecl],
            structs: &mut BTreeMap<String, BTreeMap<String, String>>,
            elems: &mut BTreeMap<String, BTreeMap<String, String>>,
        ) {
            for item in items {
                match item {
                    TopDecl::Type(td) => {
                        if let TypeDeclBody::Struct(fields) = &td.body {
                            let mut m = BTreeMap::new();
                            for f in fields {
                                if let Some(tn) = type_expr_name(&f.ty)
                                {
                                    m.insert(f.name.name.clone(), tn);
                                }
                            }
                            structs.insert(td.name.name.clone(), m);
                        }
                    }
                    TopDecl::Locus(l) => {
                        let mut m = BTreeMap::new();
                        for member in &l.members {
                            match member {
                                LocusMember::Params(pb) => {
                                    for pd in &pb.params {
                                        if let Some(tn) = pd
                                            .ty
                                            .as_ref()
                                            .and_then(elem_type_name)
                                        {
                                            m.insert(
                                                pd.name.name.clone(),
                                                tn,
                                            );
                                        }
                                    }
                                }
                                LocusMember::Capacity(cb) => {
                                    for slot in &cb.slots {
                                        if let Some(tn) =
                                            type_expr_name(
                                                &slot.elem_ty,
                                            )
                                        {
                                            m.insert(
                                                slot.name.name.clone(),
                                                tn,
                                            );
                                        }
                                    }
                                }
                                // The implicit accepted-children
                                // collection: `for child in
                                // self.children` iterates what
                                // `accept(le: T)` registered, so
                                // the element type is the accept
                                // param's. A declared field named
                                // `children` wins (entry API is
                                // first-insert via or_insert).
                                LocusMember::Lifecycle(lc)
                                    if matches!(
                                        lc.kind,
                                        LifecycleKind::Accept
                                    ) =>
                                {
                                    if let Some(tn) = lc
                                        .params
                                        .first()
                                        .and_then(|p| {
                                            type_expr_name(&p.ty)
                                        })
                                    {
                                        m.entry(
                                            "children".to_string(),
                                        )
                                        .or_insert(tn);
                                    }
                                }
                                _ => {}
                            }
                        }
                        if !m.is_empty() {
                            elems.insert(l.name.name.clone(), m);
                        }
                    }
                    TopDecl::Module(md) => {
                        collect_maps(&md.items, structs, elems)
                    }
                    _ => {}
                }
            }
        }
        for program in programs {
            collect_maps(
                &program.items,
                &mut struct_field_types,
                &mut locus_elem_types,
            );
        }
    }

    // #382 receiver-typing: free-fn name -> declared LOCUS return
    // type. `let b = make_b(); b.work(n)` was an unresolved edge —
    // the call-result receiver had no type — which made every
    // effect certificate and claim blind to the call (the audit's
    // false-certificate shape 6). Methods never appear here: the
    // no-locus-return rule forbids locus returns from methods.
    let mut fn_ret_types: BTreeMap<String, String> = BTreeMap::new();
    {
        fn collect_ret(
            items: &[TopDecl],
            locus_type_names: &BTreeSet<String>,
            out: &mut BTreeMap<String, String>,
        ) {
            for item in items {
                match item {
                    TopDecl::Fn(f) => {
                        if let Some(tn) =
                            f.ret.as_ref().and_then(type_expr_name)
                        {
                            if locus_type_names.contains(&tn) {
                                out.insert(f.name.name.clone(), tn);
                            }
                        }
                    }
                    TopDecl::Module(m) => collect_ret(
                        &m.items,
                        locus_type_names,
                        out,
                    ),
                    _ => {}
                }
            }
        }
        for program in programs {
            collect_ret(
                &program.items,
                &locus_type_names,
                &mut fn_ret_types,
            );
        }
    }

    // Phase 2 — walk each body.
    let mut summary = AllocSummary::default();
    summary.eager_only_loci = eager_only_loci;
    summary.analysis_copy_loci = analysis_copy_loci;
    // What each body starts, and whose it is, for `reached`.
    let mut starts_of: BTreeMap<FnKey, BTreeSet<String>> = BTreeMap::new();
    let mut own: BTreeSet<FnKey> = BTreeSet::new();
    // A body's walk; `(hot, mode, decl_index)` are a row's columns, and a
    // declaration body, which is no row, has none of them.
    let walk = |key: &FnKey,
                body: &Block,
                entry: Option<EntryKind>,
                enclosing_locus: &Option<String>,
                param_types: &[(String, String)],
                fn_params: &[String],
                param_elems: &[(String, String)],
                params: &[String],
                ids: &crate::snapshot::Snapshot,
                (hot, mode, decl_index): (bool, bool, usize)|
     -> (FnSummary, BTreeSet<String>, Vec<Expr>) {
        let escaping = Escaping { ids, map: collect_escaping_decls(body, ids) };
        let field_types = enclosing_locus
            .as_ref()
            .and_then(|l| locus_field_types.get(l))
            .unwrap_or(&empty_fields);
        let inline_array_fields = enclosing_locus
            .as_ref()
            .and_then(|l| locus_inline_arrays.get(l))
            .unwrap_or(&empty_inline_arrays);
        let mut w = Walker {
            fn_params: fn_params.to_vec(),
            locals: vec![params.iter().map(|p| (p.clone(), Local::Unresolved)).collect()],
            sites: Vec::new(),
            effect_sites: Vec::new(),
            locus_types: &locus_type_names,
            calls: Vec::new(),
            skipped: Vec::new(),
            loops: Vec::new(),
            escaping: &escaping,
            enclosing_locus: enclosing_locus.clone(),
            known: &known,
            starts: BTreeSet::new(),
            scope_fns: &scope_fns[scope_index(ids)],
            rename_map: if is_stdlib_copy(ids) { &no_renames } else { &rename_map },
            loop_stack: Vec::new(),
            infinite_stack: Vec::new(),
            fn_body: body,
            const_ints: &const_ints,
            store_target: None,
            var_types: param_types.iter().cloned().collect(),
            var_elem_types: param_elems.iter().cloned().collect(),
            field_types,
            all_field_types: &locus_field_types,
            all_struct_fields: &struct_field_types,
            all_elem_types: &locus_elem_types,
            fn_ret_types: &fn_ret_types,
            inline_array_fields,
            form_of: &form_of,
            retirable_structs: &retirable_structs,
            loops_as_written: 0,
            bare_stmt: None,
            self_replace: None,
            let_call: None,
            in_place_sites: Vec::new(),
        };
        w.walk_block(body, 0, Escape::Local);
        let starts = std::mem::take(&mut w.starts);
        (
            FnSummary {
                key: key.clone(),
                entry,
                sites: w.sites,
                calls: w.calls,
                loops: w.loops,
                effect_sites: w.effect_sites,
                fn_params: fn_params.to_vec(),
                hot,
                mode,
                decl_index,
                in_place_sites: w.in_place_sites,
                frame: None,
            },
            starts,
            w.skipped,
        )
    };
    // The declaration bodies still to walk: the program's bodies that are
    // no row, then every subexpression a walk skipped, under the key and
    // the parameters of the body it sits in.
    type Pending<'i> =
        (SiteId, &'static str, FnKey, Block, Vec<(String, String)>, Vec<String>, Vec<(String, String)>, &'i crate::snapshot::Snapshot, Vec<String>);
    let mut pending: Vec<Pending> = Vec::new();
    let skipped_block = |es: Vec<Expr>| Block {
        span: es.iter().map(|e| e.span()).reduce(|a, b| a.merge(b)).unwrap_or(Span::new(0, 0)),
        stmts: es.into_iter().map(Stmt::Expr).collect(),
        tail: None,
    };
    for (decl_index, (key, body, entry, enclosing_locus, param_types, fn_params, param_elems, ids, (hot, mode), decl, params)) in
        bodies.iter().enumerate()
    {
        let (row, starts, skipped) =
            walk(key, body, *entry, enclosing_locus, param_types, fn_params, param_elems, params, ids, (*hot, *mode, decl_index));
        starts_of.insert(key.clone(), starts);
        if is_stdlib_copy(ids) {
            summary.analysis_copy.insert(key.clone());
        } else {
            own.insert(key.clone());
            if let Some(site) = ids.site_id(*decl).filter(|_| !skipped.is_empty()) {
                pending.push((
                    site,
                    "subexpression",
                    key.clone(),
                    skipped_block(skipped),
                    param_types.clone(),
                    fn_params.clone(),
                    param_elems.clone(),
                    ids,
                    params.clone(),
                ));
            }
        }
        summary.fns.insert(key.clone(), row);
    }
    for (site, m, ids) in member_bodies {
        pending.push((
            site,
            m.position,
            m.key,
            m.body,
            param_var_types(m.params),
            fn_typed_params(m.params),
            param_var_elem_types(m.params),
            ids,
            param_names(m.params),
        ));
    }
    let mut next = 0;
    while next < pending.len() {
        let (site, position, key, body, param_types, fn_params, param_elems, ids, params) = pending[next].clone();
        next += 1;
        let (body_summary, _, skipped) =
            walk(&key, &body, None, &key.locus, &param_types, &fn_params, &param_elems, &params, ids, (false, false, usize::MAX));
        summary.declaration_bodies.push(DeclarationBody { declaration: site, position, summary: body_summary });
        if !skipped.is_empty() {
            pending.push((site, "subexpression", key, skipped_block(skipped), param_types, fn_params, param_elems, ids, params));
        }
    }
    summary.bounded_loci = bounded_loci;
    // Second pass: a locus holding a sync-bearing form can take that
    // form's lock, so calls into it are potentially blocking.
    collect_sync_holding_loci(programs, &sync_forms, &mut sync_holding_loci);
    summary.sync_holding_loci = sync_holding_loci;
    summary.sync_forms = sync_forms;
    summary.carries = carries;
    summary.unbounded_fns = unbounded_fns;
    summary.locus_shapes = locus_shapes;

    // #392 — interface dispatch. A method call on an interface-typed
    // value (`route.handler.handle(ctx)`) types its receiver to the
    // INTERFACE name, which owns no bodies, so the edge lands
    // `Unresolved` — and before this pass it was silently dropped by
    // every judgment, the one remaining receiver shape where a real
    // call contributed nothing. The closed world makes the implementor
    // set enumerable: fan the one written call out to an edge per
    // conforming locus (over-approximation — only adds edges), tagged
    // with a dispatch group so counting judgments take the max over
    // the alternatives. An interface nothing conforms to has no
    // values in this build (a value only arises by coercing a
    // conformer), so its call sites are DEAD: the edge stays
    // unresolved but tagged, walkers skip it, the artifact records it.
    //
    // Conformance here is name + arity over the ASTs — a superset of
    // the checker's full structural check (which also compares types).
    // A false extra conformer only adds edges; the direction is safe.
    let mut ifaces: BTreeMap<String, Vec<(String, usize)>> =
        BTreeMap::new();
    let mut locus_methods: BTreeMap<String, BTreeMap<String, usize>> =
        BTreeMap::new();
    for (program, ids) in identified {
        for item in flat_decls(&program.items).filter(|item| !shadowed(ids, item)) {
            match item {
                TopDecl::Interface(i) => {
                    if is_stdlib_copy(ids) {
                        summary.analysis_copy_interfaces.insert(i.name.name.clone());
                    }
                    ifaces.insert(
                        i.name.name.clone(),
                        i.methods
                            .iter()
                            .map(|m| (m.name.name.clone(), m.params.len()))
                            .collect(),
                    );
                }
                TopDecl::Locus(l) => {
                    let e = locus_methods
                        .entry(l.name.name.clone())
                        .or_default();
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            e.insert(
                                fd.name.name.clone(),
                                fd.params.len(),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut conformers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (iname, methods) in &ifaces {
        let mut who: Vec<&str> = Vec::new();
        for (lname, lmethods) in &locus_methods {
            if methods.iter().all(|(m, arity)| {
                lmethods.get(m).is_some_and(|a| a == arity)
            }) {
                who.push(lname);
            }
        }
        conformers.insert(iname, who);
    }
    let mut next_group: u32 = 0;
    // The rows first, so their groups are numbered as before the
    // declaration bodies were summarized beside them.
    let rows = summary.fns.values_mut().chain(summary.declaration_bodies.iter_mut().map(|b| &mut b.summary));
    for fs in rows {
        let mut rewritten: Vec<CallEdge> =
            Vec::with_capacity(fs.calls.len());
        for edge in fs.calls.drain(..) {
            let dispatch = match (&edge.callee, edge.recv_ty.as_deref())
            {
                (Callee::Unresolved(m), Some(t))
                    if edge.receiver_present
                        && ifaces.contains_key(t) =>
                {
                    Some((m.clone(), t.to_string()))
                }
                _ => None,
            };
            let Some((method, iface)) = dispatch else {
                rewritten.push(edge);
                continue;
            };
            let targets: Vec<FnKey> = conformers[iface.as_str()]
                .iter()
                .filter_map(|l| known.get(&(Some(l.to_string()), method.clone())).cloned())
                .collect();
            if targets.is_empty() {
                let mut e = edge;
                e.via_interface = Some(iface);
                rewritten.push(e);
                continue;
            }
            let gid = next_group;
            next_group += 1;
            for t in targets {
                let mut e = edge.clone();
                e.callee = Callee::Resolved(t);
                e.via_interface = Some(iface.clone());
                e.dispatch_group = Some(gid);
                rewritten.push(e);
            }
        }
        fs.calls = rewritten;
    }
    let value_scopes: Vec<ValueScope<'_>> = identified
        .iter()
        .map(|(program, ids)| ValueScope {
            items: program.items.iter().filter(|item| !shadowed(ids, item)).collect(),
            scope_fns: &scope_fns[scope_index(ids)],
            renames: if is_stdlib_copy(ids) { &no_renames } else { &rename_map },
            copy: is_stdlib_copy(ids),
        })
        .collect();
    resolve_function_values(&mut summary, &value_scopes, &known, next_group);
    summary.by_name = known.clone();
    // The reclaim boundary of a scratch-local fn (GH #1208). The
    // classification is lowering's (`alloc_routing`), run over the
    // declarations lowering runs it over (`merged`: the program, its
    // imported seeds under their mangled names, and the stdlib) with the
    // same renames: the stdlib's declarations are beside the program's
    // whether or not the summary holds the analysis copy's rows, so a
    // program summarized alone has its own rows' frames. A recursive fn
    // keeps the locus boundary: the hole's conservative answer.
    {
        let imports: BTreeMap<Vec<String>, String> = import_renames.iter().cloned().collect();
        let copy_beside = identified.iter().any(|(_, ids)| is_stdlib_copy(ids));
        let stdlib_items = crate::stdlib_bodies::program()
            .filter(|_| !copy_beside)
            .into_iter()
            .flat_map(|p| flat_decls(&p.items))
            .filter(|item| match item {
                TopDecl::Fn(f) => !declared_by_program.contains(&f.name.name),
                _ => false,
            });
        let scratch_local = crate::alloc_routing::scratch_local_free_fns(
            identified
                .iter()
                .flat_map(|(program, ids)| flat_decls(&program.items).filter(move |item| !shadowed(ids, item)))
                .chain(stdlib_items),
            &imports,
        );
        let recursive = recursive_fns(&summary.fns);
        for fs in summary.fns.values_mut() {
            if fs.key.locus.is_some() || fs.entry.is_some() {
                continue;
            }
            fs.frame = Some(if scratch_local.contains(&fs.key.fn_name) {
                Frame::ScratchLocal { recursive: recursive.contains(&fs.key) }
            } else {
                Frame::CallersArena
            });
            if fs.frees_at_return() {
                for s in fs.sites.iter_mut().filter(|s| s.escape == Escape::Local) {
                    s.reclaim = ReclaimScope::FnReturn;
                }
            }
        }
    }
    // What the program reaches, when the stdlib's analysis copy is
    // beside it: its own fns, what their calls reach (the interface
    // fan-out included), and what they start. Starting a locus runs its
    // hooks and bus handlers, and starts the loci its param fields hold
    // (by declared type, or a default's literal). Every locus of the
    // program's own is started.
    if identified.iter().any(|(_, ids)| is_stdlib_copy(ids)) {
        let mut param_starts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut started: BTreeSet<String> = BTreeSet::new();
        let mut pending: Vec<String> = Vec::new();
        for (program, ids) in identified {
            for item in flat_decls(&program.items).filter(|item| !shadowed(ids, item)) {
                let TopDecl::Locus(l) = item else { continue };
                let held = param_starts.entry(l.name.name.clone()).or_default();
                for m in &l.members {
                    let LocusMember::Params(pb) = m else { continue };
                    for p in &pb.params {
                        if let Some(t) = p.ty.as_ref().and_then(type_expr_name) {
                            if locus_type_names.contains(&t) {
                                held.insert(t);
                            }
                        }
                        if let ParamInit::Value(e) = &p.init {
                            literal_loci(e, &locus_type_names, held);
                        }
                    }
                }
                if !is_stdlib_copy(ids) {
                    pending.push(l.name.name.clone());
                }
            }
        }
        let mut reached: BTreeSet<FnKey> = BTreeSet::new();
        let mut work: Vec<FnKey> = own.into_iter().collect();
        loop {
            while let Some(l) = pending.pop() {
                if !started.insert(l.clone()) {
                    continue;
                }
                work.extend(
                    summary
                        .fns
                        .values()
                        .filter(|f| f.key.locus.as_ref() == Some(&l) && f.entry.is_some())
                        .map(|f| f.key.clone()),
                );
                pending.extend(param_starts.get(&l).into_iter().flatten().cloned());
            }
            let Some(k) = work.pop() else { break };
            if !reached.insert(k.clone()) {
                continue;
            }
            let Some(f) = summary.fns.get(&k) else { continue };
            for c in &f.calls {
                if let Callee::Resolved(callee) = &c.callee {
                    work.push(callee.clone());
                }
            }
            pending.extend(starts_of.get(&k).into_iter().flatten().cloned());
        }
        summary.reached = Some(reached);
    }
    summary
}

/// The fns in a cycle of resolved calls: a strongly connected component
/// of more than one fn, or a fn that calls itself (Tarjan's, iterative,
/// in key order).
fn recursive_fns(fns: &BTreeMap<FnKey, FnSummary>) -> BTreeSet<FnKey> {
    let keys: Vec<&FnKey> = fns.keys().collect();
    let index_of: BTreeMap<&FnKey, usize> = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let succ: Vec<Vec<usize>> = keys
        .iter()
        .map(|k| {
            fns[*k]
                .calls
                .iter()
                .filter_map(|c| match &c.callee {
                    Callee::Resolved(t) => index_of.get(t).copied(),
                    Callee::Unresolved(_) => None,
                })
                .collect()
        })
        .collect();
    let n = keys.len();
    let (mut index, mut low) = (vec![usize::MAX; n], vec![0usize; n]);
    let mut on_stack = vec![false; n];
    let (mut stack, mut next) = (Vec::new(), 0usize);
    let mut out = BTreeSet::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        // (node, the next successor to visit)
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut i)) = work.last_mut() {
            if let Some(&w) = succ[v].get(*i) {
                *i += 1;
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                loop {
                    let w = stack.pop().expect("the component's root is on the stack");
                    on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                if component.len() > 1 || succ[v].contains(&v) {
                    out.extend(component.into_iter().map(|w| keys[w].clone()));
                }
            }
        }
    }
    out
}

/// D2: a fn's params as (name, declared-type-name) pairs — the seed for
/// the var→type map (only `Named` types; primitives/arrays are skipped).
/// #353: names of the FUNCTION-TYPED parameters. `param_var_types`
/// drops these — `type_expr_name` has no name for a function type — so
/// a call through one is indistinguishable from a call to an unknown
/// free fn, and contributed nothing to any effect set.
fn fn_typed_params(params: &[Param]) -> Vec<String> {
    params
        .iter()
        .filter(|p| matches!(p.ty, TypeExpr::Function { .. }))
        .map(|p| p.name.name.clone())
        .collect()
}

fn param_names(params: &[Param]) -> Vec<String> {
    params.iter().map(|p| p.name.name.clone()).collect()
}

/// #382 receiver-typing: array-typed params' ELEMENT types, so a
/// free fn iterating an array parameter (`__http_run_chain(entries,
/// …) { for e in entries { … } }`) types the binder.
fn param_var_elem_types(params: &[Param]) -> Vec<(String, String)> {
    params
        .iter()
        .filter_map(|p| match &p.ty {
            TypeExpr::Array { elem, .. } => type_expr_name(elem)
                .map(|t| (p.name.name.clone(), t)),
            _ => None,
        })
        .collect()
}

fn param_var_types(params: &[Param]) -> Vec<(String, String)> {
    params
        .iter()
        .filter_map(|p| type_expr_name(&p.ty).map(|t| (p.name.name.clone(), t)))
        .collect()
}

/// A body that is no row, as [`member_bodies_of`] finds it: the key it is
/// walked under (its enclosing locus or perspective, for `self`), what it
/// is, the block (an expression position is a block of its expressions),
/// and its parameters.
struct MemberBody<'p> {
    key: FnKey,
    position: &'static str,
    body: Block,
    params: &'p [Param],
}

/// The bodies of `item` the reveal rule walks that are no row (a
/// [`DeclarationBody`] each), with the declaration each is a member of.
/// The rows are a fn's body and a locus's methods, modes and authored
/// hooks, a `module { }`'s declarations' included; the caller hands in
/// those too (`flat_decls`). `item` is the checked programs'; each key
/// names the member's own declaration (a handler's, a perspective fn's, a
/// synthesized hook's, a constant's), or the locus's or perspective's
/// for its params, `birth_check` and `stable_when`.
fn member_bodies_of<'p>(item: &'p TopDecl, out: &mut Vec<(NodeId, MemberBody<'p>)>) {
    let initializers = |pb: &'p ParamsBlock| -> Vec<&'p Expr> {
        pb.params
            .iter()
            .filter_map(|pd| match &pd.init {
                ParamInit::Value(e) => Some(e),
                ParamInit::Inferred => None,
            })
            .collect()
    };
    let exprs = |es: Vec<&Expr>, span: Span| Block {
        stmts: es.into_iter().map(|e| Stmt::Expr(e.clone())).collect(),
        tail: None,
        span,
    };
    match item {
        TopDecl::Const(c) => out.push((
            c.id,
            MemberBody {
                key: FnKey::free_fn(DeclId::user(c.id), c.name.name.clone()),
                position: "const",
                body: exprs(vec![&c.value], c.span),
                params: &[],
            },
        )),
        TopDecl::Locus(l) => {
            let locus = &l.name.name;
            let mut push = |member: NodeId, name: &str, position: &'static str, body: Block, params: &'p [Param]| {
                let key = FnKey::method(DeclId::user(member), locus.clone(), name);
                out.push((l.id, MemberBody { key, position, body, params }))
            };
            for m in &l.members {
                match m {
                    LocusMember::Lifecycle(lc) if lc.synthesized => {
                        push(lc.id, &lifecycle_key(lc.kind).0, "lifecycle", lc.body.clone(), &lc.params)
                    }
                    LocusMember::Failure(fd) => push(fd.id, "on_failure", "on_failure", fd.body.clone(), &fd.params),
                    LocusMember::Params(pb) => push(l.id, "params", "params", exprs(initializers(pb), pb.span), &[]),
                    LocusMember::Const(c) => push(c.id, &c.name.name, "const", exprs(vec![&c.value], c.span), &[]),
                    LocusMember::BirthCheck(bc) => {
                        let es = std::iter::once(&bc.cond).chain(&bc.payload).collect();
                        push(l.id, "birth_check", "birth_check", exprs(es, bc.span), &[])
                    }
                    _ => {}
                }
            }
        }
        TopDecl::Perspective(p) => {
            let perspective = &p.name.name;
            let mut push = |member: NodeId, name: &str, position: &'static str, body: Block, params: &'p [Param]| {
                let key = FnKey::method(DeclId::user(member), perspective.clone(), name);
                out.push((p.id, MemberBody { key, position, body, params }))
            };
            for m in &p.members {
                match m {
                    PerspectiveMember::Fn(f) => push(f.id, &f.name.name, "fn", f.body.clone(), &f.params),
                    PerspectiveMember::Params(pb) => {
                        push(p.id, "params", "params", exprs(initializers(pb), pb.span), &[])
                    }
                    PerspectiveMember::StableWhen(b) => push(p.id, "stable_when", "stable_when", b.clone(), &[]),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// D2: a locus's `params { … }` fields as field → declared-type-name, so a
/// `self.<field>.push(x)` can resolve `<field>`'s form.
fn locus_param_field_types(
    l: &LocusDecl,
    locus_types: &BTreeSet<String>,
    interface_types: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for member in &l.members {
        if let LocusMember::Params(pb) = member {
            for pd in &pb.params {
                let declared = pd.ty.as_ref().and_then(type_expr_name);
                // F.20 interface-typed slot: the DECLARED type is an
                // interface, which has no body, so `self.sink.emit()`
                // resolved to nothing and every effect behind the slot
                // was invisible. F.20's fix resolved through the
                // DEFAULT literal — and GH #533 (DNA F.11) showed that
                // is fail-open: `Holder { dep: Real { } }` overrides a
                // `dep: Gate = Noop { }` default, `Real::apply` runs,
                // and `forbid reaches(.., effects(apply_it))` passed
                // because the walker only ever saw `Noop`. (The
                // mirror — default `Real`, override `Noop` — was a
                // false positive.)
                //
                // Now: a declared INTERFACE keeps its own name, so the
                // call is `Unresolved` with an interface receiver and
                // the dispatch rewrite fans it to every conformer in
                // the closed world — the same rule a one-hop
                // `self.dep.apply()` and an interface-typed fn param
                // already follow. Every impl the program could store
                // into the slot is a conformer, so no override can
                // hide; "a hole beats a false proof of absence"
                // (spec/model.md). The default-literal fallback stays
                // for a declared name that is neither a locus nor an
                // interface (a `perspective(P)` slot, whose program-
                // global 1-1 designation IS the default until
                // reperspective — tracked separately).
                let concrete_default = match &pd.init {
                    ParamInit::Value(Expr::Struct { path, .. }) => path
                        .segments
                        .last()
                        .map(|s| s.name.clone())
                        .filter(|n| locus_types.contains(n)),
                    _ => None,
                };
                let resolved = match (&declared, &concrete_default) {
                    (Some(d), _) if interface_types.contains(d) => {
                        Some(d.clone())
                    }
                    (Some(d), Some(c)) if !locus_types.contains(d) => {
                        Some(c.clone())
                    }
                    (Some(d), _) => Some(d.clone()),
                    (None, Some(c)) => Some(c.clone()),
                    (None, None) => None,
                };
                if let Some(tn) = resolved {
                    m.insert(pd.name.name.clone(), tn);
                }
            }
        }
    }
    m
}

/// 2026-07-01 inline fixed arrays: the locus's param fields whose declared
/// type is a scalar-element `[T; N]`. Codegen lays these out INLINE in the
/// locus struct (see hale-codegen `array_inline_spec`), so a whole-value
/// replace `self.f = [ … ]` is an in-place element memcpy — the RHS
/// literal is scratch-reclaimed at method exit and nothing persists. The
/// walker downgrades such a store's escape to `Local` so store-latest
/// verdicts don't false-positive on the now-bounded shape.
fn locus_inline_array_fields(l: &LocusDecl) -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    for member in &l.members {
        if let LocusMember::Params(pb) = member {
            for pd in &pb.params {
                if let Some(TypeExpr::Array { elem, .. }) = pd.ty.as_ref() {
                    if matches!(
                        elem.as_ref(),
                        TypeExpr::Primitive(
                            PrimType::Int
                                | PrimType::Float
                                | PrimType::Bool
                                | PrimType::Decimal
                                | PrimType::Duration,
                            _,
                        )
                    ) {
                        s.insert(pd.name.name.clone());
                    }
                }
            }
        }
    }
    s
}

/// Phase D / D1: distill a `LocusDecl` into the storage shape the bound
/// solver reads — capacity slots, `@form`, and the recognition projection
/// cap. Pure AST read; no type inference.
fn locus_shape_of(l: &LocusDecl) -> LocusShape {
    let mut capacity_slots = Vec::new();
    for m in &l.members {
        if let LocusMember::Capacity(cb) = m {
            for slot in &cb.slots {
                capacity_slots.push(SlotShape {
                    name: slot.name.name.clone(),
                    kind: slot.kind,
                });
            }
        }
    }
    let form = l.form.as_ref().map(|f| FormShape {
        name: f.name.name.clone(),
        cap: f.args.iter().find_map(|a| {
            if a.name.name == "cap" {
                if let Expr::Literal(Literal::Int(n), _) = &a.value {
                    return Some(*n);
                }
            }
            None
        }),
    });
    let recognition_cap = l.annotations.iter().find_map(|a| match a {
        LocusAnnotation::Projection(ProjectionClass::Recognition(Some(p))) => Some(p.cap),
        _ => None,
    });
    LocusShape {
        name: l.name.name.clone(),
        capacity_slots,
        form,
        recognition_cap,
    }
}

/// The unbounded-allocation advisory's sites: every leak site of the
/// program's own fns ([`AllocSummary::leak_sites`], [`AllocSummary::is_own`])
/// with an author position ([`AuthorPositions`]). The one entry the
/// check's warnings, the editor's diagnostics and the editor's
/// `hale/allocSummary` read.
pub fn advisory_leak_sites(
    summary: &AllocSummary,
    programs: &[&Program],
    ids: &crate::snapshot::Snapshot,
    sources: &[crate::symbol::SourceFile],
) -> Vec<LeakSite> {
    let positions = AuthorPositions::of(programs, ids, sources);
    summary
        .leak_sites()
        .into_iter()
        .filter(|ls| summary.is_own(&ls.owner) && positions.has(ls))
        .collect()
}

/// Bound-solver diagnostics: a warning per unbounded-accumulation site.
///
/// `include_all` selects the scope (GH #18 item 1, Phase B):
/// - `false` (default `hale check`): only sites inside a `@bounded` locus —
///   the in-source opt-in. A program with no `@bounded` locus is silent.
/// - `true` (`--warn-unbounded-alloc`): every site, the whole-program
///   survey.
///
/// Either way, `@unbounded`-fn sites are already dropped at `leak_sites()`,
/// and a site of the stdlib's analysis copy or with no author position is
/// dropped here ([`advisory_leak_sites`]). `summary` is the bundle's
/// ([`derive_alloc_summary`], a snapshot's `demand_alloc_summary`);
/// `programs`, `ids` and `sources` are the bundle's programs, identities
/// and file table.
pub fn unbounded_alloc_diags(
    summary: &AllocSummary,
    programs: &[&Program],
    ids: &crate::snapshot::Snapshot,
    sources: &[crate::symbol::SourceFile],
    include_all: bool,
) -> Vec<Diag> {
    advisory_leak_sites(summary, programs, ids, sources)
        .iter()
        .filter(|ls| include_all || summary.owner_is_bounded_scope(&ls.owner))
        .map(|ls| {
            let where_ = match ls.reason {
                LeakReason::InUnboundedLoop => "inside an unbounded loop",
                LeakReason::InvokedUnboundedly => {
                    "in a fn invoked unboundedly (a per-message bus handler, \
                     or reached through a call inside an unbounded loop)"
                }
                LeakReason::InCallersArena => {
                    let (what, owner) = (ls.kind.label(), ls.owner.display());
                    return Diag::warn(
                        ls.span,
                        format!(
                            "unbounded allocation: this {what} in `{owner}` lands in its \
                             caller's arena — `{owner}` has no scratch of its own, so its \
                             return does not reclaim it — and `{owner}` runs once per \
                             iteration of an unbounded loop in a long-lived frame (`main`, \
                             `run`, or a free fn they call), so it accumulates until the locus \
                             dissolves (or acknowledge an intentionally-/domain-bounded shape \
                             with `@unbounded` on the enclosing fn). Bound the loop; make \
                             `{owner}` scratch-local (String and scalar params and return, no \
                             struct literal, no method call: its own subregion is freed at \
                             return); or move the work into a method, whose per-call scratch \
                             reclaims it."
                        ),
                    );
                }
            };
            Diag::warn(
                ls.span,
                format!(
                    "unbounded allocation: this {} {} accumulates in `{}`'s region \
                     until the locus dissolves — it is never reclaimed per iteration, \
                     so it grows without bound (or acknowledge an \
                     intentionally-/domain-bounded shape with `@unbounded` \
                     on the enclosing fn or lifecycle hook). Bound the loop; \
                     store it in a \
                     capacity-bounded form (`@form(ring_buffer)` / `@form(lru_cache)` / \
                     a `capacity` slot) instead of a replaced field; mutate fixed state \
                     in place rather than rebuilding it; route the value over the bus \
                     (the payload arena reclaims per dispatch); or move the allocating \
                     work into a per-iteration child locus.",
                    ls.kind.label(),
                    where_,
                    ls.owner.display()
                ),
            )
        })
        .collect()
}

/// Which leak sites have an author position: a place in the author's
/// source where the finding can be shown and acted on. A site has none
/// when its span lies in source a desugar generated — at or beyond
/// `API_SYNTH_BASE`, where the api binding parses what it generates, or
/// in a declaration the snapshot's origin rows mark synthesized whose
/// offset no source file owns. `json_gen` parses its parsers and the api
/// codecs at offset 0, where their offsets coincide with the first
/// file's without being its text, so a declaration of those origins
/// owns no file offset at all. Every other site has one, whoever wrote
/// the code that calls it. The check's warnings and the editor's
/// `hale/allocSummary` both decide with [`AuthorPositions::has`], and
/// drop nothing else (F.40 phase 2 review F1).
pub struct AuthorPositions {
    /// The fns and hooks whose declaration, or whose locus, carries an
    /// origin row.
    synthesized: BTreeMap<FnKey, crate::snapshot::Origin>,
    /// Each source file's window of the bundle-global offsets.
    files: Vec<(u32, u32)>,
}

impl AuthorPositions {
    pub fn of(
        programs: &[&Program],
        ids: &crate::snapshot::Snapshot,
        sources: &[crate::symbol::SourceFile],
    ) -> Self {
        let origin = |id: hale_syntax::ast::NodeId| ids.site_id(id).and_then(|s| ids.origin(s));
        let mut synthesized = BTreeMap::new();
        for p in programs {
            for item in flat_decls(&p.items) {
                match item {
                    TopDecl::Fn(f) => {
                        if let Some(o) = origin(f.id) {
                            synthesized.insert(FnKey::free_fn(DeclId::user(f.id), f.name.name.clone()), o);
                        }
                    }
                    TopDecl::Locus(l) => {
                        let of_locus = origin(l.id);
                        for m in &l.members {
                            let (name, id) = match m {
                                LocusMember::Fn(f) => (f.name.name.clone(), f.id),
                                LocusMember::Lifecycle(lc) => (lifecycle_key(lc.kind).0, lc.id),
                                _ => continue,
                            };
                            if let Some(o) = origin(id).or(of_locus) {
                                synthesized.insert(FnKey::method(DeclId::user(id), l.name.name.clone(), name), o);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let files = sources.iter().map(|f| (f.base, f.len)).collect();
        Self { synthesized, files }
    }

    /// Whether `site` lands at an author position.
    pub fn has(&self, site: &LeakSite) -> bool {
        use crate::snapshot::Origin;
        let at = site.span.start.0;
        if at >= hale_syntax::api_gen::API_SYNTH_BASE {
            return false;
        }
        match self.synthesized.get(&site.owner) {
            None => true,
            Some(Origin::JsonParsers | Origin::ApiSurface) => false,
            Some(_) => self.files.iter().any(|&(base, len)| hale_syntax::file_owns_offset(base, len, at)),
        }
    }
}

fn lifecycle_key(kind: LifecycleKind) -> (String, EntryKind) {
    match kind {
        LifecycleKind::Birth => ("birth".into(), EntryKind::Birth),
        LifecycleKind::Accept => ("accept".into(), EntryKind::Accept),
        LifecycleKind::Release => ("release".into(), EntryKind::Release),
        LifecycleKind::Run => ("run".into(), EntryKind::Run),
        LifecycleKind::Drain => ("drain".into(), EntryKind::Drain),
        LifecycleKind::Dissolve => ("dissolve".into(), EntryKind::Dissolve),
    }
}

/// The locus a struct literal's path instantiates, by its declared name:
/// a stdlib locus written by its public path (`std::http::Server`) is
/// the mangled name its declaration carries. `None` for a plain struct.
fn struct_locus(path: &hale_syntax::ast::QualifiedName,locus_types: &BTreeSet<String>) -> Option<String> {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    let name = match segs.as_slice() {
        [one] => one.to_string(),
        _ => crate::stdlib_bodies::mangled_locus_name(&segs)?.to_string(),
    };
    locus_types.contains(&name).then_some(name)
}

/// The loci a struct-literal expression instantiates, nested literals
/// included (a param field's default).
fn literal_loci(e: &Expr, locus_types: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    if let Expr::Struct { path, inits, .. } = e {
        if let Some(l) = struct_locus(path, locus_types) {
            out.insert(l);
        }
        for si in inits {
            literal_loci(&si.value, locus_types, out);
        }
    }
}

/// The set of method names a locus subscribes as bus handlers.
fn bus_handler_names(l: &LocusDecl) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for member in &l.members {
        if let LocusMember::Bus(bus) = member {
            for bm in &bus.members {
                if let BusMember::Subscribe { handler, .. } = bm {
                    out.insert(handler.name.clone());
                }
            }
        }
    }
    out
}

/// M3 stage 5 gap C: is this field-init expression scalar-valued or
/// a static literal — i.e., does storing it into an existing struct
/// slot allocate NOTHING in the locus arena? Conservative: anything
/// unrecognized returns false (the site stays flagged).
fn init_is_scalar_or_static(e: &Expr) -> bool {
    match e {
        // Static literals: strings/bytes live in .rodata (the
        // anchor's static-skip); scalar literals are by-value.
        Expr::Literal(_, _) => true,
        Expr::Unary { operand, .. } => init_is_scalar_or_static(operand),
        Expr::Binary { left, right, op, .. } => {
            // Arithmetic/comparison on scalars is by-value. `+` over
            // strings CONCATENATES (fresh heap) — only accept when
            // both sides are non-string-literal scalars; a string
            // literal on either side of `+` marks concat.
            let string_side = matches!(
                left.as_ref(),
                Expr::Literal(Literal::String(_), _)
            ) || matches!(
                right.as_ref(),
                Expr::Literal(Literal::String(_), _)
            );
            !(matches!(op, hale_syntax::ast::BinOp::Add) && string_side)
                && init_is_scalar_or_static(left)
                && init_is_scalar_or_static(right)
        }
        // self.field re-reads are the RMW pattern — the anchor's
        // same-arena gate makes re-storing them identity. A bare
        // LOCAL Ident stays conservative: it may hold a freshly
        // parsed String, and storing that into self is exactly the
        // TP-3 anchor-clone leak.
        Expr::Field { receiver, .. } => {
            matches!(receiver.as_ref(), Expr::KwSelf(_))
        }
        _ => false,
    }
}

/// Whether a value of one declared type can be the other's, as far as
/// the type expressions say without resolving them: `false` only where
/// the checker's exact equality of function types cannot hold (two
/// different primitives; a primitive, function, tuple or array against
/// one of the others; either against a nominal type). A named type
/// that is a type alias, a generic parameter, or not a declaration the
/// bundle names as itself (an import alias's spelling) may be anything,
/// so it is compatible with everything; so is a projection or a
/// perspective type. Two named types are always compatible: one type
/// has several spellings across seeds.
struct FnTypes<'a> {
    aliases: &'a BTreeSet<String>,
    nominal: &'a BTreeSet<String>,
}

impl FnTypes<'_> {
    fn compatible(&self, a: &TypeExpr, b: &TypeExpr, generic: &dyn Fn(&str) -> bool) -> bool {
        // A named type the bundle declares as itself: a struct, enum,
        // locus, interface or perspective, or a stdlib path.
        let nominal = |t: &TypeExpr| match t {
            TypeExpr::Named { path, .. } => {
                let joined = path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                let last = path.segments.last().map(|s| s.name.as_str()).unwrap_or_default();
                joined.starts_with("std::")
                    || (path.segments.len() == 1
                        && self.nominal.contains(last)
                        && !self.aliases.contains(last)
                        && !generic(last))
            }
            _ => false,
        };
        let open = |t: &TypeExpr| match t {
            TypeExpr::Named { .. } => !nominal(t),
            TypeExpr::Projection { .. } | TypeExpr::Perspective { .. } => true,
            _ => false,
        };
        if open(a) || open(b) {
            return true;
        }
        match (a, b) {
            // One type, two spellings (`std::http::Context` and the
            // stdlib's own declared name) is compatible; two stdlib paths,
            // or two names the bundle declares, are one type only when
            // they are one spelling.
            (TypeExpr::Named { path: pa, .. }, TypeExpr::Named { path: pb, .. }) => {
                let std = |p: &QualifiedName| p.segments.first().is_some_and(|s| s.name == "std");
                std(pa) != std(pb) || pa.segments.iter().map(|s| &s.name).eq(pb.segments.iter().map(|s| &s.name))
            }
            (TypeExpr::Primitive(p, _), TypeExpr::Primitive(q, _)) => p == q,
            (
                TypeExpr::Function { params: pa, ret: ra, .. },
                TypeExpr::Function { params: pb, ret: rb, .. },
            ) => {
                pa.len() == pb.len()
                    && pa.iter().zip(pb).all(|(x, y)| self.compatible(x, y, generic))
                    && match (ra, rb) {
                        (Some(x), Some(y)) => self.compatible(x, y, generic),
                        _ => true,
                    }
            }
            (TypeExpr::Tuple(xa, _), TypeExpr::Tuple(xb, _)) => {
                xa.len() == xb.len() && xa.iter().zip(xb).all(|(x, y)| self.compatible(x, y, generic))
            }
            (
                TypeExpr::Array { elem: ea, .. } | TypeExpr::Bounded { elem: ea, .. },
                TypeExpr::Array { elem: eb, .. } | TypeExpr::Bounded { elem: eb, .. },
            ) => self.compatible(ea, eb, generic),
            _ => false,
        }
    }
}

/// One program's declarations, as the function-value pass reads them:
/// its items (the analysis copy's without those a checked program
/// declares again), and the scope its names resolve in.
struct ValueScope<'a> {
    items: Vec<&'a TopDecl>,
    scope_fns: &'a BTreeSet<String>,
    renames: &'a BTreeMap<String, String>,
    copy: bool,
}

/// E5 (F.40 phase 3): an indirect call resolves to the program's
/// function values.
///
/// An indirect call (through a function-typed parameter, a local the
/// walk does not follow to a fn, or a computed callee) reaches a function
/// value, and a function value only arises from a function's name read
/// as a value ([`crate::fn_values`]). The closed world makes that set
/// enumerable: every name read as a value anywhere in the bundle (the
/// stdlib's analysis copy included), resolved as a `let` binding of it
/// resolves — a fn of the reading program's scope, an imported fn
/// through the import renames, a module-nested fn by its bare name, a
/// builtin or a registered stdlib path as the unresolved leaf a direct
/// call of it is. The one written call becomes one edge per value whose
/// fn takes as many parameters as the call passes (a leaf of unknown
/// arity matches every call), tagged with a dispatch group so counting
/// judgments take the max over the alternatives, as for an interface
/// dispatch (#392). Arity is a superset of the checker's type match; a
/// false extra target only adds edges.
///
/// The set is not named, and the call stays the indirect row every
/// reader fails closed on, when no value matches (a value of the type
/// cannot exist, but a dead call is not worth a reader of its own), or
/// when a member is read as a value under a name that is some locus's
/// method and no declaration's field (a method value: the checker types
/// it, codegen does not lower it, and this pass does not follow it).
fn resolve_function_values<'a>(
    summary: &mut AllocSummary,
    scopes: &[ValueScope<'a>],
    known: &Known,
    mut next_group: u32,
) {
    use crate::fn_values::ValueName;
    fn each_decl<'a>(item: &'a TopDecl, f: &mut dyn FnMut(&'a TopDecl)) {
        f(item);
        if let TopDecl::Module(m) = item {
            for i in &m.items {
                each_decl(i, f);
            }
        }
    }
    // Every free fn's and locus method's declaration, by its row's key (a
    // value's signature, and an indirect call's enclosing declaration,
    // whose function-typed parameter types it).
    let mut decls: BTreeMap<FnKey, &'a FnDecl> = BTreeMap::new();
    let mut methods: BTreeSet<String> = BTreeSet::new();
    // Every declared field, by name: its declared type (`None` where it
    // has none, or holds no value a call could be through).
    let mut field_tys: BTreeMap<String, Vec<Option<&'a TypeExpr>>> = BTreeMap::new();
    let mut aliases: BTreeSet<String> = BTreeSet::new();
    let mut nominal: BTreeSet<String> = BTreeSet::new();
    for s in scopes {
        let universe = if s.copy { SiteUniverse::StdlibAnalysis } else { SiteUniverse::User };
        for item in &s.items {
            fn type_decl<'a>(
                t: &'a TypeDecl,
                aliases: &mut BTreeSet<String>,
                nominal: &mut BTreeSet<String>,
                field_tys: &mut BTreeMap<String, Vec<Option<&'a TypeExpr>>>,
            ) {
                match &t.body {
                    TypeDeclBody::Alias(_) => {
                        aliases.insert(t.name.name.clone());
                    }
                    TypeDeclBody::Struct(fs) => {
                        nominal.insert(t.name.name.clone());
                        for f in fs {
                            field_tys.entry(f.name.name.clone()).or_default().push(Some(&f.ty));
                        }
                    }
                    TypeDeclBody::Enum(_) => {
                        nominal.insert(t.name.name.clone());
                    }
                }
            }
            each_decl(item, &mut |d| {
                match d {
                    TopDecl::Fn(f) => {
                        decls.insert(FnKey::free_fn(DeclId::of(universe, f.id), f.name.name.clone()), f);
                    }
                    TopDecl::Type(t) => type_decl(t, &mut aliases, &mut nominal, &mut field_tys),
                    TopDecl::Interface(i) => {
                        nominal.insert(i.name.name.clone());
                    }
                    TopDecl::Locus(l) => {
                        nominal.insert(l.name.name.clone());
                        for m in &l.members {
                            match m {
                                LocusMember::Fn(f) => {
                                    methods.insert(f.name.name.clone());
                                    let key = FnKey::method(DeclId::of(universe, f.id), l.name.name.clone(), f.name.name.clone());
                                    decls.insert(key, f);
                                }
                                LocusMember::Type(t) => type_decl(t, &mut aliases, &mut nominal, &mut field_tys),
                                LocusMember::Params(pb) => {
                                    for p in &pb.params {
                                        field_tys.entry(p.name.name.clone()).or_default().push(p.ty.as_ref());
                                    }
                                }
                                LocusMember::Capacity(cb) => {
                                    for slot in &cb.slots {
                                        field_tys.entry(slot.name.name.clone()).or_default().push(None);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    TopDecl::Perspective(p) => {
                        nominal.insert(p.name.name.clone());
                        for m in &p.members {
                            match m {
                                PerspectiveMember::Fn(f) => {
                                    methods.insert(f.name.name.clone());
                                }
                                PerspectiveMember::Params(pb) => {
                                    for p in &pb.params {
                                        field_tys.entry(p.name.name.clone()).or_default().push(p.ty.as_ref());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            });
        }
    }
    // Each value: its callee, its declaration when it is a fn of the
    // bundle (a builtin or a stdlib path has none, and matches every
    // call), and whether a checked program (not only the analysis copy)
    // takes it.
    let mut taken: BTreeMap<String, (Callee, Option<&'a FnDecl>, bool)> = BTreeMap::new();
    let mut method_value = false;
    for s in scopes {
        let mut names = Vec::new();
        for item in &s.items {
            crate::fn_values::value_names(std::slice::from_ref(*item), &mut names);
        }
        let mut take = |callee: Callee, decl: Option<&'a FnDecl>| {
            let key = match &callee {
                Callee::Resolved(k) => k.display(),
                Callee::Unresolved(n) => n.clone(),
            };
            taken.entry(key).or_insert((callee, decl, false)).2 |= !s.copy;
        };
        let own_fn = |name: &str| {
            let key = known.get(&(None, name.to_string())).filter(|_| s.scope_fns.contains(name))?;
            Some((key.clone(), decls.get(key).copied()))
        };
        for name in names {
            match name {
                ValueName::Ident(n) => {
                    if let Some((key, decl)) = own_fn(&n) {
                        take(Callee::Resolved(key), decl);
                    } else if crate::check::BARE_BUILTIN_CALLEES.contains(&n.as_str()) {
                        take(Callee::Unresolved(n), None);
                    }
                }
                ValueName::Path(p) => {
                    if let Some(mangled) = s.renames.get(&p) {
                        if let Some((key, decl)) = own_fn(mangled) {
                            take(Callee::Resolved(key), decl);
                        }
                    } else if p.starts_with("std::") {
                        let segs: Vec<&str> = p.split("::").collect();
                        if crate::stdlib_surface::effects_for(&segs).is_some() {
                            take(Callee::Unresolved(p), None);
                        }
                    } else if let Some(last) = p.rsplit("::").next() {
                        if let Some((key, decl)) = own_fn(last) {
                            take(Callee::Resolved(key), decl);
                        }
                    }
                }
                ValueName::Member(n) => {
                    if methods.contains(&n) && !field_tys.contains_key(&n) {
                        method_value = true;
                    }
                }
            }
        }
    }
    summary.analysis_copy_values =
        taken.iter().filter(|(_, (c, _, own))| !own && matches!(c, Callee::Unresolved(_))).map(|(k, _)| k.clone()).collect();
    if method_value {
        return;
    }
    let types = FnTypes { aliases: &aliases, nominal: &nominal };
    // Declaration bodies carry the same call facts for editor dependents.
    // Keep their groups after the fn rows, as for interface dispatch.
    let rows = summary.fns.values_mut().chain(summary.declaration_bodies.iter_mut().map(|b| &mut b.summary));
    for fs in rows {
        if !fs.calls.iter().any(|e| e.indirect) {
            continue;
        }
        let enclosing = decls.get(&fs.key).copied();
        let mut rewritten: Vec<CallEdge> = Vec::with_capacity(fs.calls.len());
        for edge in fs.calls.drain(..) {
            let written = match &edge.callee {
                Callee::Unresolved(n) if edge.indirect => n.clone(),
                _ => {
                    rewritten.push(edge);
                    continue;
                }
            };
            // The callee's declared types, when the program states them:
            // a function-typed parameter's, or every declared type of the
            // field a local was bound from (each a function type).
            let declared: Option<Vec<&TypeExpr>> = if edge.through_param {
                enclosing
                    .and_then(|d| d.params.iter().find(|p| p.name.name == written))
                    .map(|p| vec![&p.ty])
            } else {
                edge.bound_field.as_ref().and_then(|f| field_tys.get(f)).and_then(|tys| tys.iter().copied().collect())
            };
            let declared: Option<Vec<(&Vec<TypeExpr>, Option<&TypeExpr>)>> = declared.and_then(|tys| {
                tys.into_iter()
                    .map(|t| match t {
                        TypeExpr::Function { params, ret, .. } => Some((params, ret.as_deref())),
                        _ => None,
                    })
                    .collect()
            });
            let enclosing_generics: Vec<&str> =
                enclosing.map(|d| d.generics.iter().map(|g| g.name.name.as_str()).collect()).unwrap_or_default();
            let targets: Vec<&Callee> = taken
                .values()
                .filter(|(_, decl, _)| match decl {
                    None => true,
                    Some(t) if t.params.len() != edge.arity => false,
                    Some(t) => declared.as_ref().map_or(true, |sigs| {
                        let generic =
                            |n: &str| enclosing_generics.contains(&n) || t.generics.iter().any(|g| g.name.name == n);
                        sigs.iter().any(|(params, ret)| {
                            params.len() == t.params.len()
                                && params.iter().zip(&t.params).all(|(a, b)| types.compatible(a, &b.ty, &generic))
                                && match (ret, &t.ret) {
                                    (Some(a), Some(b)) if t.fallible.is_none() => types.compatible(a, b, &generic),
                                    _ => true,
                                }
                        })
                    }),
                })
                .map(|(c, _, _)| c)
                .collect();
            if targets.is_empty() {
                rewritten.push(edge);
                continue;
            }
            let gid = next_group;
            next_group += 1;
            for t in targets {
                let mut e = edge.clone();
                e.callee = t.clone();
                e.indirect = false;
                match e.spelling {
                    CallSpelling::Ident(_) => e.via_local = Some(written.clone()),
                    // A computed callee was written with no receiver to
                    // type; resolved, it is not an opaque method call.
                    _ => e.receiver_present = false,
                }
                e.via_value = Some(written.clone());
                e.dispatch_group = Some(gid);
                rewritten.push(e);
            }
        }
        fs.calls = rewritten;
    }
}

/// What a call through a local binding reaches.
#[derive(Debug, Clone)]
enum Local {
    /// `let f = <fn name or path>`: what a direct call of the name or path
    /// would reach.
    Fn(Callee),
    /// A parameter, or a local bound to anything else (a call's result, a
    /// field, a parameter), by a tuple `let`, a loop or a pattern, or
    /// reassigned:
    /// the walk does not follow values or flow, and a call through it is
    /// the edge it always was (indirect through a function-typed
    /// parameter, #353).
    Unresolved,
    /// `let f = <expr>.name`: not followed either, but the field's name
    /// is kept, whose declared types type a call through `f` (E5,
    /// `CallEdge::bound_field`).
    Field(String),
}

/// What a call spelling `(locus, name)` resolves to: the row of the last
/// declaration of that name ([`AllocSummary::by_name`]).
type Known = BTreeMap<(Option<String>, String), FnKey>;

struct Walker<'a> {
    sites: Vec<AllocSite>,
    /// GH #265: syntactic effect sites (publish / spawn).
    effect_sites: Vec<EffectSite>,
    /// GH #265: declared locus type names — a struct literal of one
    /// of these is a locus INSTANTIATION (arena create + possibly a
    /// thread spawn / pool post), not a plain data allocation.
    locus_types: &'a BTreeSet<String>,
    /// #353: function-typed parameter names of the fn being walked, so
    /// a call through one can be marked indirect on its edge.
    fn_params: Vec<String>,
    /// The body's local bindings, innermost scope last, the params the
    /// outermost: what a call through each would reach.
    locals: Vec<BTreeMap<String, Local>>,
    calls: Vec<CallEdge>,
    /// The subexpressions the walk does not descend into (a callee that
    /// is no name), each summarized
    /// beside the body as a [`DeclarationBody`] of its declaration.
    skipped: Vec<Expr>,
    loops: Vec<LoopInfo>,
    escaping: &'a Escaping<'a>,
    enclosing_locus: Option<String>,
    known: &'a Known,
    /// The loci this body instantiates, by their declared names.
    starts: BTreeSet<String>,
    /// The free fns of the body's own scope (its seed's programs,
    /// modules included): the only fns a bare name resolves to.
    scope_fns: &'a BTreeSet<String>,
    /// `alias::name` -> mangled symbol (cross-seed imports), empty for
    /// the stdlib's analysis copy.
    rename_map: &'a BTreeMap<String, String>,
    /// One entry per enclosing loop: `true` if that loop has a const trip
    /// count. A value alloc is in an unbounded loop iff any entry is false.
    loop_stack: Vec<bool>,
    /// M3 stage 5 gap B: parallel stack, `true` when the loop is
    /// literally `while true` (never exits) — the one shape where a
    /// scratch-ful frame's "reclaimed at method exit" argument
    /// fails, because the method never exits.
    infinite_stack: Vec<bool>,
    /// The whole fn body — loop-ranking scans it to decide whether a
    /// `while v < N` counter is const-bounded (const init + only positive
    /// const increments anywhere in the fn).
    fn_body: &'a Block,
    /// iris handoff P2.1: top-level const ints for ceiling folding.
    const_ints: &'a BTreeMap<String, i64>,
    /// Phase D / D1: the `self.<field>` currently being assigned, set around
    /// the RHS walk of a `self.<field> = …` statement so an escaping
    /// allocation in that RHS records which field it lands in.
    store_target: Option<String>,
    /// Phase D / D2 (lite): local var / param name → declared type *name*,
    /// seeded from this fn's params and grown by typed `let`s. Lets a
    /// `v.push(x)` resolve `v`'s type to ask whether it is a growing form.
    var_types: BTreeMap<String, String>,
    /// #382 receiver-typing: local/param name -> ELEMENT type for
    /// array-typed bindings, so `for e in entries` types the binder
    /// when `entries` is an array param or an array-literal let.
    var_elem_types: BTreeMap<String, String>,
    /// #382 receiver-typing: EVERY locus's field->type map, for
    /// chained receivers (`self.mid.inner.work()` types `mid` via
    /// the enclosing locus, then `inner` via Mid's map).
    all_field_types: &'a BTreeMap<String, BTreeMap<String, String>>,
    /// #382 receiver-typing: struct TYPE field->type maps, the
    /// fallback for chained receivers passing through plain
    /// structs (`route.handler.handle()`).
    all_struct_fields: &'a BTreeMap<String, BTreeMap<String, String>>,
    /// #382 receiver-typing: per-locus ELEMENT types (array params
    /// fields + capacity slots), for `for`-binder typing.
    all_elem_types: &'a BTreeMap<String, BTreeMap<String, String>>,
    /// #382 receiver-typing: free-fn -> declared locus return type,
    /// for call-result receivers (`make_b().work()`).
    fn_ret_types: &'a BTreeMap<String, String>,
    /// The enclosing locus's param fields → declared type name, so a
    /// `self.<field>.push(x)` resolves `<field>`'s type the same way.
    field_types: &'a BTreeMap<String, String>,
    /// 2026-07-01: the enclosing locus's scalar-[T; N] param fields —
    /// codegen lays these out inline, so a whole-value replace is an
    /// in-place memcpy whose RHS is scratch-reclaimed (walked Local).
    inline_array_fields: &'a BTreeSet<String>,
    /// Every locus's `@form(...)` name, keyed by locus type name — the
    /// lookup `var_types`/`field_types` feed into `form_grows`.
    form_of: &'a BTreeMap<String, String>,
    /// Gap D: struct type names whose fields are ALL scalar / String —
    /// a whole-field replace of such a struct fully reclaims via anchor
    /// retirement (see `AllocSite::retired_store`).
    retirable_structs: &'a BTreeSet<String>,
    /// The loops the current point is written inside, which a `return`
    /// / `fail` payload does not reset (`AllocSite::in_loop`).
    loops_as_written: u32,
    /// The span of the struct literal the current statement is,
    /// whole (`AllocSite::bare_stmt`).
    bare_stmt: Option<Span>,
    /// The struct literal on the right of the current `self.<field> =`
    /// replace and the statement's span (`AllocSite::self_replace`).
    self_replace: Option<(Span, Span)>,
    /// The call that is the value of the current `let` and the
    /// statement's span (`CallEdge::let_span`).
    let_call: Option<(Span, Span)>,
    /// `FnSummary::in_place_sites`.
    in_place_sites: Vec<AllocSite>,
}

impl<'a> Walker<'a> {
    /// GH #265: record a syntactic effect site (publish / spawn) in
    /// the fn currently being summarized.
    fn push_effect_site(
        &mut self,
        kind: EffectSiteKind,
        depth: u32,
        span: Span,
    ) {
        self.effect_sites.push(EffectSite {
            kind,
            loop_depth: depth,
            span,
        });
    }

    fn push_site(&mut self, kind: AllocKind, escape: Escape, depth: u32, span: Span) {
        let site = self.site(kind, escape, depth, span);
        self.sites.push(site);
    }

    fn site(&self, kind: AllocKind, escape: Escape, depth: u32, span: Span) -> AllocSite {
        // Only a StoredToSelf escape carries a target field — that's the
        // `self.<field> = <alloc>` whole-value replace the solver bounds.
        let target_field = if escape == Escape::StoredToSelf {
            self.store_target.clone()
        } else {
            None
        };
        // Gap D: whole-field struct replace of an all-scalar/String
        // struct — reclaimed via anchor retirement (Gap A).
        let retired_store = target_field.is_some()
            && matches!(&kind, AllocKind::StructLit(n)
                if self.retirable_structs.contains(n));
        AllocSite {
            kind,
            escape,
            loop_depth: depth,
            in_unbounded_loop: self.loop_stack.iter().any(|bounded| !bounded),
            in_infinite_loop: self.infinite_stack.iter().any(|i| *i),
            reclaim: ReclaimScope::of(escape),
            target_field,
            retired_store,
            in_loop: self.loops_as_written > 0,
            bare_stmt: false,
            self_replace: None,
            span,
        }
    }

    /// `fail` / `return` terminate the enclosing invocation, so their
    /// payload allocates at most once per call no matter how many loops
    /// enclose the statement — walk it with the loop context cleared
    /// (depth 0, no unbounded/infinite flags). The payload still escapes
    /// to the caller (`Escape::Returned`): a fail payload is allocated
    /// into the per-call subregion and stored to the caller's err slot,
    /// so the cross-fn unbounded-invocation arm (GAP A in
    /// `final_verdict`) must keep seeing it. `violate` also diverges but
    /// its payload is walked `Local` (framework-consumed) — see the
    /// `Stmt::Violate` arm.
    fn walk_diverging_payload(&mut self, e: &Expr) {
        let saved_loops = std::mem::take(&mut self.loop_stack);
        let saved_infinite = std::mem::take(&mut self.infinite_stack);
        self.walk_expr(e, 0, Escape::Returned);
        self.loop_stack = saved_loops;
        self.infinite_stack = saved_infinite;
    }

    /// Is this expression provably a String?
    ///
    /// Deliberately narrow: a string literal, an f-string, or a name
    /// whose declared type is `String`. Anything it cannot prove it
    /// leaves alone, so `i + 1` is never mistaken for concatenation.
    fn is_string_expr(&self, e: &Expr) -> bool {
        match e {
            Expr::Literal(Literal::String(_), _) => true,
            Expr::Ident(v) => {
                self.var_types.get(&v.name).map(|t| t == "String").unwrap_or(false)
            }
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. }
                if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
            {
                self.field_types.get(&name.name).map(|t| t == "String").unwrap_or(false)
            }
            // `(a + b) + c` — a concat is itself a String.
            Expr::Binary { left, right, op: BinOp::Add, .. } => {
                self.is_string_expr(left) || self.is_string_expr(right)
            }
            _ => false,
        }
    }

    /// The declared/inferable LOCUS type of a receiver expression
    /// (#382 receiver-typing root fix). Beyond the original two
    /// shapes (bare var via `var_types`, `self.<field>` via
    /// `field_types`), this types the four audit shapes that were
    /// silently-dropped unresolved edges: a struct-literal receiver
    /// (`B { }.work()`), a chained field (`self.mid.inner.work()`),
    /// a call result (`make_b().work()`), and a uniform if/else
    /// value. Anything genuinely untypeable at this layer (an index
    /// result, a match value, a foreign expression) returns `None`
    /// and downstream soundness judgments fail closed on the edge.
    /// Used to spot growing-`@form` receivers (D2) and to resolve
    /// handle-method call edges (`record_call`).
    fn receiver_type_name(&self, recv: &Expr) -> Option<String> {
        match recv {
            Expr::Ident(v) => self.var_types.get(&v.name).cloned(),
            Expr::KwSelf(_) => self.enclosing_locus.clone(),
            Expr::Field { receiver, name, .. }
            | Expr::Path2 { receiver, name, .. } => {
                if matches!(receiver.as_ref(), Expr::KwSelf(_)) {
                    self.field_types.get(&name.name).cloned()
                } else {
                    // Chained: type the receiver, then look the
                    // field up on THAT type's map — locus params
                    // first, plain-struct fields as the fallback.
                    let rt = self.receiver_type_name(receiver)?;
                    self.all_field_types
                        .get(&rt)
                        .and_then(|m| m.get(&name.name))
                        .or_else(|| {
                            self.all_struct_fields
                                .get(&rt)
                                .and_then(|m| m.get(&name.name))
                        })
                        .cloned()
                }
            }
            Expr::Struct { path, .. } => {
                // Same std-vs-user resolution as the `let`-literal
                // inference below.
                let segs: Vec<&str> = path
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect();
                crate::stdlib_bodies::mangled_locus_name(&segs)
                    .map(|m| m.to_string())
                    .or_else(|| {
                        path.segments.last().map(|s| s.name.clone())
                    })
            }
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Ident(f) if self.scope_fns.contains(&f.name) => {
                    self.fn_ret_types.get(&f.name).cloned()
                }
                // `entries.get(j)` on a collection locus with ONE
                // element slot returns that element type — the
                // form-getter shape stdlib chains iterate with.
                Expr::Field { receiver, name, .. }
                | Expr::Path2 { receiver, name, .. }
                    if name.name == "get" =>
                {
                    let owner = self.receiver_type_name(receiver)?;
                    let elems = self.all_elem_types.get(&owner)?;
                    if elems.len() == 1 {
                        elems.values().next().cloned()
                    } else {
                        None
                    }
                }
                Expr::Path(qp) => {
                    let path = qp
                        .segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join("::");
                    let name = self
                        .rename_map
                        .get(&path)
                        .cloned()
                        .unwrap_or(path);
                    if !self.scope_fns.contains(&name) {
                        return None;
                    }
                    self.fn_ret_types.get(&name).cloned()
                }
                _ => None,
            },
            Expr::If(ifs) => self.if_locus_type(ifs),
            // `mws.get(i) or raise` — the disposition unwraps the
            // fallible; the VALUE'S type is the inner call's.
            Expr::Or { inner, .. } => self.receiver_type_name(inner),
            _ => None,
        }
    }

    /// The ELEMENT type of an iterable expression: `self.field` /
    /// chained fields via the per-locus element maps (array params
    /// fields + capacity slots), or a uniform array literal.
    fn iter_elem_type(&self, iter: &Expr) -> Option<String> {
        match iter {
            Expr::Field { receiver, name, .. }
            | Expr::Path2 { receiver, name, .. } => {
                let owner = self.receiver_type_name(receiver)?;
                self.all_elem_types
                    .get(&owner)?
                    .get(&name.name)
                    .cloned()
            }
            Expr::Ident(v) => {
                self.var_elem_types.get(&v.name).cloned()
            }
            Expr::Array(elems, _) => {
                let mut ty: Option<String> = None;
                for e in elems {
                    let t = self.receiver_type_name(e)?;
                    match &ty {
                        None => ty = Some(t),
                        Some(prev) if *prev == t => {}
                        Some(_) => return None,
                    }
                }
                ty
            }
            _ => None,
        }
    }

    /// A uniform if/else VALUE's locus type: every branch's tail
    /// must type to the same locus, else `None`.
    fn if_locus_type(&self, ifs: &IfStmt) -> Option<String> {
        let then_ty = ifs
            .then_block
            .tail
            .as_ref()
            .and_then(|t| self.receiver_type_name(t))?;
        let else_ty = match ifs.else_block.as_deref()? {
            ElseBranch::Else(b) => b
                .tail
                .as_ref()
                .and_then(|t| self.receiver_type_name(t))?,
            ElseBranch::ElseIf(inner) => self.if_locus_type(inner)?,
        };
        if then_ty == else_ty {
            Some(then_ty)
        } else {
            None
        }
    }

    /// D2: if `recv` is a value whose *declared* type is a growing
    /// `@form(vec | hashmap)` locus, the form name.
    fn growing_form_of_receiver(&self, recv: &Expr) -> Option<String> {
        let ty_name = self.receiver_type_name(recv)?;
        let form = self.form_of.get(&ty_name)?;
        if form_grows(form) {
            Some(form.clone())
        } else {
            None
        }
    }

    /// D2: record a growing-collection insert as an accumulating site. It
    /// persists into the collection (reclaim at the owner's dissolve), so
    /// in an unbounded context it accumulates — same verdict path as a
    /// `StoredToSelf` value alloc.
    fn push_collection_insert(&mut self, form: String, depth: u32, span: Span) {
        self.sites.push(AllocSite {
            kind: AllocKind::CollectionInsert(form),
            escape: Escape::StoredToSelf,
            loop_depth: depth,
            in_unbounded_loop: self.loop_stack.iter().any(|bounded| !bounded),
            in_infinite_loop: self.infinite_stack.iter().any(|i| *i),
            reclaim: ReclaimScope::EnclosingLocus,
            target_field: None,
            retired_store: false,
            in_loop: self.loops_as_written > 0,
            bare_stmt: false,
            self_replace: None,
            span,
        });
    }
}

impl<'a> Walker<'a> {
    fn walk_block(&mut self, b: &Block, depth: u32, tail_escape: Escape) {
        self.locals.push(BTreeMap::new());
        for s in &b.stmts {
            self.walk_stmt(s, depth);
        }
        if let Some(t) = &b.tail {
            self.walk_expr(t, depth, tail_escape);
        }
        self.locals.pop();
    }

    fn bind(&mut self, name: &str, local: Local) {
        self.locals.last_mut().expect("a body has a scope").insert(name.to_string(), local);
    }

    fn local(&self, name: &str) -> Option<&Local> {
        self.locals.iter().rev().find_map(|scope| scope.get(name))
    }

    /// The binding `name` names now holds a value the walk does not
    /// follow.
    fn unresolve(&mut self, name: &str) {
        if let Some(scope) = self.locals.iter_mut().rev().find(|s| s.contains_key(name)) {
            scope.insert(name.to_string(), Local::Unresolved);
        }
    }

    /// Before a loop is walked, each binding it reassigns is unresolved
    /// for the whole loop and after it ([`loop_reassigned`]).
    fn enter_loop(&mut self, cond: Option<&Expr>, body: &Block) {
        for name in loop_reassigned(cond, body) {
            self.unresolve(&name);
        }
    }

    /// The row a bare free-fn name of the body's own scope resolves to.
    fn own_fn(&self, name: &str) -> Option<FnKey> {
        self.known.get(&(None, name.to_string())).filter(|_| self.scope_fns.contains(name)).cloned()
    }

    /// What a direct call of a `Path` callee resolves to.
    fn path_callee(&self, qp: &QualifiedName) -> Callee {
        let path = qp.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
        // A cross-seed `alias::name` names a decl merged under
        // a mangled symbol. Resolving it here is what lets the
        // callgraph walk INTO an imported seed instead of
        // stopping at the boundary and reporting nothing.
        match self.rename_map.get(&path) {
            Some(mangled) => match self.own_fn(mangled) {
                Some(key) => Callee::Resolved(key),
                None => Callee::Unresolved(path),
            },
            None => Callee::Unresolved(path),
        }
    }

    /// What a `let` binds its name to: the fn a fn name or path names (a
    /// local bound to one passes it on), or nothing the walk can follow.
    fn bound_value(&self, value: &Expr) -> Local {
        match value {
            Expr::Ident(id) => match self.local(&id.name) {
                Some(Local::Fn(c)) => Local::Fn(c.clone()),
                Some(Local::Unresolved) => Local::Unresolved,
                Some(Local::Field(f)) => Local::Field(f.clone()),
                None => {
                    if let Some(key) = self.own_fn(&id.name) {
                        Local::Fn(Callee::Resolved(key))
                    } else if crate::check::BARE_BUILTIN_CALLEES.contains(&id.name.as_str()) {
                        Local::Fn(Callee::Unresolved(id.name.clone()))
                    } else {
                        Local::Unresolved
                    }
                }
            },
            Expr::Path(qp) => Local::Fn(self.path_callee(qp)),
            Expr::Field { name, .. } => Local::Field(name.name.clone()),
            _ => Local::Unresolved,
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt, depth: u32) {
        match stmt {
            Stmt::Let { name, ty, value, id, span, .. } => {
                // D2: a typed `let v: T = …` extends the var→type map so a
                // later `v.push(x)` can resolve `v`'s form.
                if let Some(et) = ty.as_ref().and_then(|t| match t {
                    TypeExpr::Array { elem, .. } => type_expr_name(elem),
                    _ => None,
                }) {
                    self.var_elem_types.insert(name.name.clone(), et);
                } else if let Expr::Array(elems, _) = value {
                    // `let xs = [B { }, make_b()];` — a uniform
                    // locus array literal types the elements.
                    let mut et: Option<String> = None;
                    let mut uniform = true;
                    for e in elems {
                        match (self.receiver_type_name(e), &et) {
                            (Some(t), None) => et = Some(t),
                            (Some(t), Some(prev)) if t == *prev => {}
                            _ => {
                                uniform = false;
                                break;
                            }
                        }
                    }
                    if uniform {
                        if let Some(t) = et {
                            self.var_elem_types
                                .insert(name.name.clone(), t);
                        }
                    }
                }
                if let Some(tn) = ty.as_ref().and_then(type_expr_name) {
                    self.var_types.insert(name.name.clone(), tn);
                } else if let Some(ty) = self.receiver_type_name(value) {
                    // GH #265 / #382: `let r = Reader { … }`, `let b =
                    // make_b();`, and the uniform if/else value all
                    // bind a locus the walker can type. Without this,
                    // `b.method()` stays an unresolved edge and every
                    // effect assertion and claim is blind to it.
                    self.var_types.insert(name.name.clone(), ty);
                }
                let esc = self.escaping.of_let(*id);
                self.walk_let_value(value, *span, depth, esc);
                let bound = self.bound_value(value);
                self.bind(&name.name, bound);
            }
            Stmt::LetTuple { names, value, span, .. } => {
                self.walk_let_value(value, *span, depth, Escape::Local);
                for n in names {
                    self.bind(&n.name, Local::Unresolved);
                }
            }
            Stmt::Assign { target, value, id, span, .. } => {
                if target.head.name == "self"
                    && matches!(target.tail.as_slice(), [LValueSeg::Field(_)])
                    && matches!(value, Expr::Struct { .. })
                {
                    self.self_replace = Some((value.span(), *span));
                }
                let mut esc = if target.head.name == "self" {
                    Escape::StoredToSelf
                } else {
                    self.escaping.of_assign(*id)
                };
                // D1: record the `self.<field>` being assigned for the RHS
                // walk — but only for a whole-field replace (`self.f = …`),
                // not an indexed in-place write (`self.f[i] = …`, which has
                // a trailing `Index` segment and allocates nothing new).
                let prev = self.store_target.take();
                if esc == Escape::StoredToSelf {
                    let target_field = self_replace_field(target);
                    // 2026-07-01 inline fixed arrays: a whole-value
                    // replace of a scalar-[T; N] field is an in-place
                    // element memcpy (codegen `array_inline_spec`) —
                    // nothing persists in the locus arena; the RHS
                    // literal is scratch-reclaimed at method exit.
                    // Walk the RHS as Local so store-latest verdicts
                    // don't flag the now-bounded shape.
                    if let Some(f) = &target_field {
                        if self.inline_array_fields.contains(f) {
                            esc = Escape::Local;
                        }
                    }
                    if esc == Escape::StoredToSelf {
                        self.store_target = target_field;
                    }
                }
                self.walk_expr(value, depth, esc);
                self.store_target = prev;
                self.self_replace = None;
                // A reassigned local holds whichever value the run took
                // last: the walk does not follow flow.
                if target.tail.is_empty() {
                    self.unresolve(&target.head.name);
                }
            }
            Stmt::Return(Some(e), _) => self.walk_diverging_payload(e),
            Stmt::Return(None, _) => {}
            // Perspectives Phase 2b: `reperspective` instantiates a
            // fresh impl (bounded, one per swap) — no expression to
            // walk for the alloc summary.
            Stmt::Reperspective { .. } => {}
            Stmt::Fail { value, .. } => self.walk_diverging_payload(value),
            Stmt::Send { subject, value, span, .. } => {
                // GH #265: a publish is an effect the language
                // expresses syntactically — record it so publish-set
                // assertions (and @no_publish) can see it. The
                // subject is exact when it is a literal / declared
                // topic name, which is the common case and what makes
                // the closed-topic-set claim hold.
                let subj = match subject {
                    Expr::Literal(Literal::String(s), _) => {
                        Some(PublishSubject {
                            text: s.clone(),
                            literal: true,
                        })
                    }
                    // A declared topic name (`Sig <- v`) parses as a
                    // bare identifier.
                    Expr::Ident(id) => Some(PublishSubject {
                        text: id.name.clone(),
                        literal: false,
                    }),
                    // A topic imported from a shared catalog
                    // (`t::SharedTopic <- v`) is a qualified
                    // path. Treating it as "computed" made every
                    // publish of a SHARED topic unprovable — which is
                    // every topic that crosses a binary, and so the
                    // only ones a publish-set contract is really for.
                    Expr::Path(qp) => Some(PublishSubject {
                        text: qp
                            .segments
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join("::"),
                        literal: false,
                    }),
                    _ => None,
                };
                self.push_effect_site(
                    EffectSiteKind::Publish(subj),
                    depth,
                    *span,
                );
                self.walk_expr(subject, depth, Escape::Local);
                self.walk_expr(value, depth, Escape::Sent);
            }
            Stmt::For { name, iter, body, span, .. } => {
                self.walk_expr(iter, depth, Escape::Local);
                // #382 receiver-typing: type the loop BINDER from the
                // iterable's element type, so `for child in
                // self.children { child.m() }` is a resolved edge
                // rather than an untypeable receiver.
                let elem = self.iter_elem_type(iter);
                let shadowed = match &elem {
                    Some(t) => self
                        .var_types
                        .insert(name.name.clone(), t.clone()),
                    None => self.var_types.remove(&name.name),
                };
                let kind = for_loop_kind(iter);
                let bounded = matches!(kind, LoopKind::ForRange { bounded: Some(_) });
                self.loops.push(LoopInfo { kind, depth, span: *span });
                self.loop_stack.push(bounded);
                self.infinite_stack.push(false);
                self.loops_as_written += 1;
                self.enter_loop(None, body);
                self.locals.push(BTreeMap::from([(name.name.clone(), Local::Unresolved)]));
                self.walk_block(body, depth + 1, Escape::Local);
                self.locals.pop();
                self.loops_as_written -= 1;
                self.loop_stack.pop();
                self.infinite_stack.pop();
                match shadowed {
                    Some(prev) => {
                        self.var_types.insert(name.name.clone(), prev);
                    }
                    None => {
                        self.var_types.remove(&name.name);
                    }
                }
            }
            Stmt::While { cond, body, span } => {
                self.enter_loop(Some(cond), body);
                self.walk_expr(cond, depth, Escape::Local);
                // Loop-ranking: a `while v < N` counter whose `v` is
                // const-initialized and only ever incremented by positive
                // consts is const-bounded; any other `while` is unbounded
                // (its trip count is runtime, like a runtime `for`-iter).
                let bounded =
                    while_counter_bounded(cond, self.fn_body, self.const_ints);
                let kind = if bounded { LoopKind::WhileCounter } else { while_loop_kind(cond) };
                let infinite = matches!(
                    cond,
                    Expr::Literal(Literal::Bool(true), _)
                );
                self.loops.push(LoopInfo { kind, depth, span: *span });
                self.loop_stack.push(bounded);
                self.infinite_stack.push(infinite);
                self.loops_as_written += 1;
                self.walk_block(body, depth + 1, Escape::Local);
                self.loops_as_written -= 1;
                self.loop_stack.pop();
                self.infinite_stack.pop();
            }
            Stmt::If(if_stmt) => self.walk_if(if_stmt, depth, Escape::Local),
            Stmt::Match(m) => self.walk_match(m, depth, Escape::Local),
            Stmt::Block(b) => self.walk_block(b, depth, Escape::Local),
            Stmt::ShmWrite { max, binding, body, .. } => {
                self.walk_expr(max, depth, Escape::Local);
                // The body writes into the ring view, not the arena; treat
                // its allocations as local for now.
                self.locals.push(BTreeMap::from([(binding.name.clone(), Local::Unresolved)]));
                self.walk_block(body, depth, Escape::Local);
                self.locals.pop();
            }
            Stmt::Expr(e) => {
                if let Expr::Struct { span, .. } = e {
                    self.bare_stmt = Some(*span);
                }
                self.walk_expr(e, depth, Escape::Local);
                self.bare_stmt = None;
            }
            Stmt::Recovery { args, .. } => {
                for a in args {
                    self.walk_expr(a, depth, Escape::Local);
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.walk_expr(p, depth, Escape::Local);
                }
            }
            Stmt::Yield(_) | Stmt::Terminate(_) | Stmt::Break(_) | Stmt::Continue(_) => {}
        }
    }

    /// A `let`'s value: a call that is the whole value carries the
    /// statement's span (`CallEdge::let_span`).
    fn walk_let_value(&mut self, value: &Expr, stmt: Span, depth: u32, escape: Escape) {
        if let Expr::Call { span, .. } = value {
            self.let_call = Some((*span, stmt));
        }
        self.walk_expr(value, depth, escape);
        self.let_call = None;
    }

    fn walk_if(&mut self, if_stmt: &IfStmt, depth: u32, escape: Escape) {
        self.walk_expr(&if_stmt.cond, depth, Escape::Local);
        self.walk_block(&if_stmt.then_block, depth, escape);
        if let Some(else_br) = &if_stmt.else_block {
            match else_br.as_ref() {
                ElseBranch::Else(b) => self.walk_block(b, depth, escape),
                ElseBranch::ElseIf(inner) => self.walk_if(inner, depth, escape),
            }
        }
    }

    fn walk_match(&mut self, m: &MatchStmt, depth: u32, escape: Escape) {
        self.walk_expr(&m.scrutinee, depth, Escape::Local);
        for arm in &m.arms {
            let mut bound = BTreeMap::new();
            pattern_bindings(&arm.pattern, &mut bound);
            self.locals.push(bound);
            if let Some(g) = &arm.guard {
                self.walk_expr(g, depth, Escape::Local);
            }
            match &arm.body {
                MatchArmBody::Block(b) => self.walk_block(b, depth, escape),
                MatchArmBody::Expr(e) => self.walk_expr(e, depth, escape),
            }
            self.locals.pop();
        }
    }

    fn walk_expr(&mut self, expr: &Expr, depth: u32, escape: Escape) {
        match expr {
            Expr::Struct { path, inits, span, .. } => {
                // The *qualified* path (joined) — a local struct is
                // single-segment ("Quote"), a stdlib one carries its full
                // path ("std::io::tcp::Listener") so consumers can match it
                // without colliding with a same-named user type.
                let name = path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                // M3 stage 5 gap C (2026-07-02, audit): a
                // whole-value `self.<field> = X { ... }` replace
                // where every init is a scalar expression or a
                // STATIC literal does not grow the arena — codegen's
                // emit_self_field_inplace_assign memcpys over the
                // existing struct, and anchor_struct_fields_in_place
                // skips static-literal subfields (they live in
                // .rodata, not any arena). A single fresh heap
                // subfield (a parsed String, a concat) re-enables
                // the site — that's the TP-3 anchor-clone class.
                let inplace_no_heap = matches!(escape, Escape::StoredToSelf)
                    && inits.iter().all(|si| init_is_scalar_or_static(&si.value));
                if let Some(l) = struct_locus(path, self.locus_types) {
                    self.starts.insert(l);
                }
                if self.locus_types.contains(&name) {
                    // GH #265: locus instantiation — an effect in its
                    // own right (arena create, possibly a thread or
                    // pool post), recorded whether or not the alloc
                    // site below is elided.
                    self.push_effect_site(
                        EffectSiteKind::Spawn(name.clone()),
                        depth,
                        *span,
                    );
                }
                let mut site = self.site(AllocKind::StructLit(name), escape, depth, *span);
                site.bare_stmt = self.bare_stmt.take() == Some(*span);
                site.self_replace = self.self_replace.take().filter(|(v, _)| v == span).map(|(_, s)| s);
                if inplace_no_heap {
                    self.in_place_sites.push(site);
                } else {
                    self.sites.push(site);
                }
                for si in inits {
                    self.walk_expr(&si.value, depth, escape);
                }
            }
            Expr::Array(xs, span) => {
                self.push_site(AllocKind::ArrayLit, escape, depth, *span);
                for x in xs {
                    self.walk_expr(x, depth, escape);
                }
            }
            Expr::ArrayRepeat { val, span, .. } => {
                // `count` is a const `u64`, not an expr — nothing to walk.
                self.push_site(AllocKind::ArrayRepeat, escape, depth, *span);
                self.walk_expr(val, depth, escape);
            }
            Expr::Literal(Literal::Bytes(_), span) => {
                self.push_site(AllocKind::BytesLit, escape, depth, *span);
            }
            Expr::Literal(_, _) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
            // NOTE: a String `+` is an arena concat and a real allocation
            // site, but telling it from arithmetic `+` needs type info this
            // Flagging every `i + 1` would be the cry-wolf failure this
            // pass must avoid, so concatenation is recorded ONLY where an
            // operand is provably a String — a string literal, or a name
            // whose DECLARED type is String. Int arithmetic is untouched.
            //
            // It has to be recorded somewhere: `"x" + a + "y"` performs 34
            // heap allocations, and before this a fn doing exactly that
            // passed `@budget(alloc_per_call = 0)`. A zero-allocation
            // certificate that ignores the most common way Hale code
            // allocates is worse than no certificate — it reads as proof.
            Expr::Binary { left, right, op, .. } => {
                if matches!(op, BinOp::Add)
                    && (self.is_string_expr(left) || self.is_string_expr(right))
                {
                    let span = left.span().merge(right.span());
                    self.sites.push(AllocSite {
                        kind: AllocKind::StringConcat,
                        escape,
                        loop_depth: depth,
                        in_unbounded_loop: self
                            .loop_stack
                            .iter()
                            .any(|bounded| !bounded),
                        in_infinite_loop: self
                            .infinite_stack
                            .iter()
                            .any(|i| *i),
                        reclaim: ReclaimScope::EnclosingLocus,
                        target_field: None,
                        retired_store: false,
                        in_loop: self.loops_as_written > 0,
                        bare_stmt: false,
                        self_replace: None,
                        span,
                    });
                }
                self.walk_expr(left, depth, Escape::Local);
                self.walk_expr(right, depth, Escape::Local);
            }
            Expr::Unary { operand, .. } => self.walk_expr(operand, depth, Escape::Local),
            Expr::Call { callee, args, span, .. } => {
                self.record_call(callee, args.len(), *span, depth, escape);
                // D2: a `recv.<insert>(x)` where `recv`'s declared type is a
                // growing form is itself an accumulating allocation.
                if let Expr::Field { receiver, name, .. }
                | Expr::Path2 { receiver, name, .. } = callee.as_ref()
                {
                    if is_insert_method(&name.name) {
                        if let Some(form) = self
                            .growing_form_of_receiver(receiver)
                            .filter(|f| is_growing_insert(f, &name.name))
                        {
                            self.push_collection_insert(form, depth, *span);
                        }
                    }
                }
                // The callee receiver may itself allocate; its result
                // doesn't escape via this site.
                match callee.as_ref() {
                    Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
                        self.walk_expr(receiver, depth, Escape::Local);
                    }
                    Expr::Ident(_) | Expr::Path(_) => {}
                    other => self.skipped.push(other.clone()),
                }
                for a in args {
                    self.walk_expr(a, depth, Escape::Local);
                }
            }
            Expr::Field { receiver, .. } => self.walk_expr(receiver, depth, Escape::Local),
            // The subscript is evaluated as any operand is: what it
            // allocates or calls is the row's, `@hot`'s hard rejection
            // included (E3a part C, review finding).
            Expr::Index { receiver, index, .. } => {
                self.walk_expr(receiver, depth, Escape::Local);
                self.walk_expr(index, depth, Escape::Local);
            }
            Expr::Path2 { receiver, .. } => self.walk_expr(receiver, depth, Escape::Local),
            Expr::Tuple(xs, _) => {
                for x in xs {
                    self.walk_expr(x, depth, escape);
                }
            }
            Expr::Block(b) => self.walk_block(b, depth, escape),
            Expr::If(if_stmt) => self.walk_if(if_stmt, depth, escape),
            Expr::Match(m) => self.walk_match(m, depth, escape),
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.walk_expr(inner, depth, escape),
            Expr::Approx { left, right, tolerance, .. } => {
                self.walk_expr(left, depth, Escape::Local);
                self.walk_expr(right, depth, Escape::Local);
                self.walk_expr(tolerance, depth, Escape::Local);
            }
            Expr::Range { lo, hi, .. } => {
                self.walk_expr(lo, depth, Escape::Local);
                self.walk_expr(hi, depth, Escape::Local);
            }
            Expr::Or { inner, disposition, .. } => {
                self.walk_expr(inner, depth, escape);
                match disposition {
                    OrDisposition::Substitute(e) => self.walk_expr(e, depth, escape),
                    OrDisposition::Fail(e, _) => self.walk_expr(e, depth, escape),
                    OrDisposition::Raise(_)
                    | OrDisposition::Discard(_)
                    | OrDisposition::Wait(_) => {}
                }
            }
        }
    }

    fn record_call(&mut self, callee: &Expr, arity: usize, span: Span, depth: u32, escape: Escape) {
        let mut recv_ty: Option<String> = None;
        let mut receiver_present = false;
        let mut via_local: Option<String> = None;
        let mut unresolved_local = false;
        let mut bound_field: Option<String> = None;
        let mut computed = false;
        let resolved = match callee {
            Expr::Ident(id) => {
                if let Some(key) = self.own_fn(&id.name) {
                    Callee::Resolved(key)
                } else if crate::check::BARE_BUILTIN_CALLEES.contains(&id.name.as_str()) {
                    Callee::Unresolved(id.name.clone())
                } else {
                    // A bare callee names a builtin or a fn before a
                    // local, as codegen lowers it. A call through a
                    // local the bindings follow to a fn reaches that fn;
                    // through any other, the edge is what it always was.
                    match self.local(&id.name) {
                        Some(Local::Fn(c)) => {
                            via_local = Some(id.name.clone());
                            c.clone()
                        }
                        Some(Local::Unresolved) => {
                            unresolved_local = true;
                            Callee::Unresolved(id.name.clone())
                        }
                        Some(Local::Field(f)) => {
                            unresolved_local = true;
                            bound_field = Some(f.clone());
                            Callee::Unresolved(id.name.clone())
                        }
                        None => Callee::Unresolved(id.name.clone()),
                    }
                }
            }
            Expr::Path(qp) => self.path_callee(qp),
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                // #382: a receiver that is a bare ident naming an
                // IMPORT ALIAS is a qualified free-fn call
                // (`pay::charge(n)`), not a method receiver. Any
                // other non-`self` receiver expression is a real
                // method receiver.
                let is_alias = match receiver.as_ref() {
                    Expr::Ident(r) => {
                        let prefix = format!("{}::", r.name);
                        self.rename_map
                            .keys()
                            .any(|k| k.starts_with(&prefix))
                    }
                    _ => false,
                };
                receiver_present =
                    !matches!(receiver.as_ref(), Expr::KwSelf(_))
                        && !is_alias;
                // `self.m()` — the enclosing locus is the receiver type.
                let owner = if matches!(receiver.as_ref(), Expr::KwSelf(_)) {
                    self.enclosing_locus.clone()
                } else {
                    // GH #265 soundness: a call through a HANDLE
                    // (`r.slurp()`, `f.read_all()`) is a real callgraph
                    // edge, but resolving it needs the receiver's type.
                    // Dropping to `Unresolved(<bare name>)` lost that
                    // edge entirely, so every effect assertion was blind
                    // to method calls on loci — which is the idiomatic
                    // way to do I/O in Hale, making `@no_syscall`
                    // decorative outside free-fn code.
                    self.receiver_type_name(receiver)
                };
                recv_ty = owner.clone();
                match owner {
                    Some(ty) => match self.known.get(&(Some(ty), name.name.clone())) {
                        Some(key) => Callee::Resolved(key.clone()),
                        None => Callee::Unresolved(name.name.clone()),
                    },
                    None => Callee::Unresolved(name.name.clone()),
                }
            }
            _ => {
                // A call through an arbitrary expression — nothing
                // the walk can see through. Conservative: treat as
                // receiver-present so soundness judgments fail
                // closed.
                receiver_present = true;
                computed = true;
                Callee::Unresolved("<expr>".to_string())
            }
        };
        // A call through a function-typed parameter, a local the bindings
        // do not follow to a fn, or a computed callee: a function value
        // whose target this body does not name.
        let through_param = match &resolved {
            Callee::Unresolved(n) if via_local.is_none() => self.fn_params.iter().any(|p| p == n),
            _ => false,
        };
        let indirect = through_param || unresolved_local || computed;
        let spelling = CallSpelling::of(callee);
        self.calls.push(CallEdge {
            recv_ty,
            receiver_present,
            callee: resolved,
            indirect,
            through_param,
            via_local,
            unresolved_local,
            bound_field,
            loop_depth: depth,
            in_unbounded_loop: self.loop_stack.iter().any(|bounded| !bounded),
            escape,
            receiver_slot: self_slot_receiver(callee),
            via_interface: None,
            dispatch_group: None,
            via_value: None,
            arity,
            allocating_recv: spelling.allocating_recv(),
            spelling,
            in_loop: self.loops_as_written > 0,
            let_span: self.let_call.take().filter(|(c, _)| *c == span).map(|(_, s)| s),
            span,
            callee_span: callee.span(),
        });
    }
}

/// The names a match pattern binds, none of them a fn the walk follows.
fn pattern_bindings(p: &Pattern, out: &mut BTreeMap<String, Local>) {
    match p {
        Pattern::Binding(id) => {
            out.insert(id.name.clone(), Local::Unresolved);
        }
        Pattern::Constructor { args: ps, .. } | Pattern::Tuple(ps, _) => {
            for p in ps {
                pattern_bindings(p, out);
            }
        }
        Pattern::Literal(..) | Pattern::Wildcard(_) => {}
    }
}

/// The bindings a loop reassigns (P3 2 of 3, a review fix): every name a
/// plain assignment in its condition or body rebinds, at any depth, but
/// not one a `let`, loop binder or pattern inside the loop shadows where
/// the assignment is written. A walk that follows a local function value
/// visits a loop once, so before it walks the loop it takes each of these
/// for unresolved, for the whole loop and after it: a call ahead of the
/// assignment runs, on a later iteration, the value the assignment
/// stored. Every walker that tracks bindings uses this one rule.
pub(crate) fn loop_reassigned(cond: Option<&Expr>, body: &Block) -> BTreeSet<String> {
    let mut a = Reassigned { scopes: Vec::new(), out: BTreeSet::new() };
    if let Some(c) = cond {
        a.expr(c);
    }
    a.block(body, Vec::new());
    a.out
}

/// [`loop_reassigned`]'s walk: the names bound inside the loop, innermost
/// scope last.
struct Reassigned {
    scopes: Vec<BTreeSet<String>>,
    out: BTreeSet<String>,
}

impl Reassigned {
    fn bind(&mut self, name: &str) {
        self.scopes.last_mut().expect("a scope").insert(name.to_string());
    }

    fn block(&mut self, b: &Block, binders: Vec<String>) {
        self.scopes.push(binders.into_iter().collect());
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                self.expr(value);
                self.bind(&name.name);
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value);
                for n in names {
                    self.bind(&n.name);
                }
            }
            Stmt::Assign { target, value, .. } => {
                for seg in &target.tail {
                    if let LValueSeg::Index(e) = seg {
                        self.expr(e);
                    }
                }
                self.expr(value);
                let name = &target.head.name;
                if target.tail.is_empty() && !self.scopes.iter().any(|s| s.contains(name)) {
                    self.out.insert(name.clone());
                }
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(m) => self.match_stmt(m),
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter);
                self.block(body, vec![name.name.clone()]);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body, Vec::new());
            }
            Stmt::ShmWrite { max, binding, body, .. } => {
                self.expr(max);
                self.block(body, vec![binding.name.clone()]);
            }
            Stmt::Block(b) => self.block(b, Vec::new()),
            Stmt::Return(Some(e), _) | Stmt::Fail { value: e, .. } | Stmt::Expr(e) => self.expr(e),
            Stmt::Violate { payload: Some(e), .. } => self.expr(e),
            Stmt::Recovery { args, .. } => {
                for a in args {
                    self.expr(a);
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(OrDisposition::Substitute(x) | OrDisposition::Fail(x, _)) = or_disposition {
                    self.expr(x);
                }
            }
            Stmt::Return(None, _)
            | Stmt::Violate { payload: None, .. }
            | Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }

    fn if_stmt(&mut self, i: &IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block, Vec::new());
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b, Vec::new()),
            Some(ElseBranch::ElseIf(e)) => self.if_stmt(e),
            None => {}
        }
    }

    fn match_stmt(&mut self, m: &MatchStmt) {
        self.expr(&m.scrutinee);
        for arm in &m.arms {
            let mut bound = BTreeMap::new();
            pattern_bindings(&arm.pattern, &mut bound);
            self.scopes.push(bound.into_keys().collect());
            if let Some(g) = &arm.guard {
                self.expr(g);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b, Vec::new()),
            }
            self.scopes.pop();
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Block(b) => self.block(b, Vec::new()),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo: left, hi: right, .. } | Expr::Index { receiver: left, index: right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand: x, .. }
            | Expr::Field { receiver: x, .. }
            | Expr::Path2 { receiver: x, .. }
            | Expr::Sum(x, _)
            | Expr::Prod(x, _)
            | Expr::ArrayRepeat { val: x, .. } => self.expr(x),
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => {
                for x in xs {
                    self.expr(x);
                }
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.expr(&i.value);
                }
            }
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                if let OrDisposition::Substitute(x) | OrDisposition::Fail(x, _) = disposition {
                    self.expr(x);
                }
            }
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }
}

/// D1: the `self.<field>` field name of a whole-value replace
/// (`self.f = …`), or `None` for an indexed in-place write (`self.f[i] = …`)
/// or a nested store (`self.f.g = …`). A clean top-level whole-field
/// replace is `tail == [Field(name)]`.
fn self_replace_field(target: &LValue) -> Option<String> {
    if target.head.name != "self" {
        return None;
    }
    match target.tail.as_slice() {
        [LValueSeg::Field(f)] => Some(f.name.clone()),
        _ => None,
    }
}

/// D1: for a call whose callee is `self.<slot>.<method>(…)`, the slot name.
/// `None` for free fns, `self`-methods, and form methods (`self.push`).
fn self_slot_receiver(callee: &Expr) -> Option<String> {
    let (Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. }) = callee else {
        return None;
    };
    match receiver.as_ref() {
        Expr::Field { receiver: inner, name, .. }
        | Expr::Path2 { receiver: inner, name, .. }
            if matches!(inner.as_ref(), Expr::KwSelf(_)) =>
        {
            Some(name.name.clone())
        }
        _ => None,
    }
}

fn for_loop_kind(iter: &Expr) -> LoopKind {
    if let Expr::Range { lo, hi, inclusive, .. } = iter {
        if let (Expr::Literal(Literal::Int(a), _), Expr::Literal(Literal::Int(b), _)) =
            (lo.as_ref(), hi.as_ref())
        {
            let span = if *inclusive { b - a + 1 } else { b - a };
            return LoopKind::ForRange { bounded: Some(span.max(0)) };
        }
        return LoopKind::ForRange { bounded: None };
    }
    LoopKind::ForIter
}

fn while_loop_kind(cond: &Expr) -> LoopKind {
    if matches!(cond, Expr::Literal(Literal::Bool(true), _)) {
        LoopKind::WhileTrue
    } else {
        LoopKind::While
    }
}

/// Loop-ranking: is `while <cond> { … }` a const-bounded counter? True iff
/// the cond is `v < N` / `v <= N` (`N` a const Int literal, `v` a local
/// Ident) and, across the whole fn body, `v` is const-initialized exactly
/// once and its only mutations are positive const increments (`v += k` /
/// `v = v + k`, `k > 0`). Then `v` rises monotonically from a const toward
/// a const ceiling → the trip count is bounded by a compile-time constant.
/// Conservative: any non-increment mutation, shadowing, non-const init, or
/// a `self.field` counter → false (never a false "bounded").
fn while_counter_bounded(
    cond: &Expr,
    fn_body: &Block,
    const_ints: &BTreeMap<String, i64>,
) -> bool {
    // iris handoff P2.1: the ceiling may be any CONST-FOLDABLE int
    // expression — a literal, a top-level `const` ident, or +/-/*
    // arithmetic over those (`NET_SLOTS * WINDOW`). Runtime values
    // still rank unbounded.
    let var = match cond {
        Expr::Binary { op: BinOp::Lt | BinOp::LtEq, left, right, .. } => {
            match left.as_ref() {
                Expr::Ident(v)
                    if const_int_eval(right, const_ints).is_some() =>
                {
                    v.name.as_str()
                }
                _ => return false,
            }
        }
        _ => return false,
    };
    // M3 stage 5 note (2026-07-02): a runtime-invariant ceiling
    // extension (len()/param ceilings ranked bounded) was tried and
    // REVERTED — the RSS-validated model_unbounded_verdict test is
    // the authority: a param-ceiling loop in a scratchless frame
    // accumulates linearly IN THE INPUT (3M iters ≈ 190 MB), which
    // is exactly what "unbounded" means here. Scratch-ful frames
    // already get their per-activation reclaim in final_verdict
    // (gap B); the one-shot-main shape is gap E, a lifetime
    // question, not a loop-bound one.
    let mut s = CounterScan::default();
    scan_counter_block(fn_body, var, &mut s);
    s.const_inits == 1
        && s.nonconst_inits == 0
        && s.rebindings == 0
        && s.bad_assigns == 0
        && s.pos_increments >= 1
}

/// iris handoff P2.2: single-segment named type → its name.
fn single_named_type_name(ty: &TypeExpr) -> Option<String> {
    match ty {
        TypeExpr::Named { path, generic_args, .. }
            if path.segments.len() == 1 && generic_args.is_empty() =>
        {
            Some(path.segments[0].name.clone())
        }
        _ => None,
    }
}

/// iris handoff P2.1: fold an int expression over literals,
/// top-level `const` idents, and +/-/* arithmetic. None = not a
/// compile-time constant.
fn const_int_eval(
    e: &Expr,
    const_ints: &BTreeMap<String, i64>,
) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Int(v), _) => Some(*v),
        Expr::Ident(id) => const_ints.get(&id.name).copied(),
        Expr::Binary { op, left, right, .. } => {
            let l = const_int_eval(left, const_ints)?;
            let r = const_int_eval(right, const_ints)?;
            match op {
                BinOp::Add => l.checked_add(r),
                BinOp::Sub => l.checked_sub(r),
                BinOp::Mul => l.checked_mul(r),
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Default)]
struct CounterScan {
    const_inits: usize,    // `let mut v = <const int>`
    nonconst_inits: usize, // `let mut v = <non-const>`
    rebindings: usize,     // a `for v` / `let (…, v, …)` shadow
    pos_increments: usize, // `v += k` / `v = v + k`, k > 0
    bad_assigns: usize,    // any other assignment to v
}

fn scan_counter_block(b: &Block, v: &str, s: &mut CounterScan) {
    for stmt in &b.stmts {
        scan_counter_stmt(stmt, v, s);
    }
}

fn scan_counter_stmt(stmt: &Stmt, v: &str, s: &mut CounterScan) {
    match stmt {
        Stmt::Let { name, value, .. } => {
            if name.name == v {
                if matches!(value, Expr::Literal(Literal::Int(_), _)) {
                    s.const_inits += 1;
                } else {
                    s.nonconst_inits += 1;
                }
            }
        }
        Stmt::LetTuple { names, .. } => {
            if names.iter().any(|n| n.name == v) {
                s.rebindings += 1;
            }
        }
        Stmt::Assign { target, op, value, .. } => {
            if target.head.name == v && target.tail.is_empty() {
                if is_pos_increment(v, op, value) {
                    s.pos_increments += 1;
                } else {
                    s.bad_assigns += 1;
                }
            }
        }
        Stmt::For { name, body, .. } => {
            if name.name == v {
                s.rebindings += 1;
            }
            scan_counter_block(body, v, s);
        }
        Stmt::While { body, .. } => scan_counter_block(body, v, s),
        Stmt::If(if_stmt) => scan_counter_if(if_stmt, v, s),
        Stmt::Match(m) => {
            for arm in &m.arms {
                if let MatchArmBody::Block(bl) = &arm.body {
                    scan_counter_block(bl, v, s);
                }
            }
        }
        Stmt::Block(bl) => scan_counter_block(bl, v, s),
        Stmt::ShmWrite { body, .. } => scan_counter_block(body, v, s),
        _ => {}
    }
}

fn scan_counter_if(if_stmt: &IfStmt, v: &str, s: &mut CounterScan) {
    scan_counter_block(&if_stmt.then_block, v, s);
    if let Some(eb) = &if_stmt.else_block {
        match eb.as_ref() {
            ElseBranch::Else(bl) => scan_counter_block(bl, v, s),
            ElseBranch::ElseIf(inner) => scan_counter_if(inner, v, s),
        }
    }
}

fn is_pos_increment(v: &str, op: &AssignOp, value: &Expr) -> bool {
    match op {
        AssignOp::PlusEq => matches!(value, Expr::Literal(Literal::Int(k), _) if *k > 0),
        AssignOp::Eq => {
            if let Expr::Binary { op: BinOp::Add, left, right, .. } = value {
                let is_v = |e: &Expr| matches!(e, Expr::Ident(i) if i.name == v);
                let pos = |e: &Expr| matches!(e, Expr::Literal(Literal::Int(k), _) if *k > 0);
                (is_v(left) && pos(right)) || (pos(left) && is_v(right))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Which bindings of a body escape, and how: what a `let`'s value and an
/// `=`'s value are walked as. Keyed by the declarations the escaping
/// uses name (`Snapshot::binding_of`).
struct Escaping<'a> {
    ids: &'a crate::snapshot::Snapshot,
    map: BTreeMap<SiteId, Escape>,
}

impl Escaping<'_> {
    /// How the value of the `let` whose statement is `let_id` escapes.
    fn of_let(&self, let_id: hale_syntax::ast::NodeId) -> Escape {
        self.ids
            .site_id(let_id)
            .and_then(|s| self.map.get(&s).copied())
            .unwrap_or(Escape::Local)
    }

    /// How the value of the `=` whose statement is `assign_id` escapes:
    /// as the binding its head names does.
    fn of_assign(&self, assign_id: hale_syntax::ast::NodeId) -> Escape {
        self.ids
            .declaration_of(assign_id)
            .and_then(|d| self.map.get(&d).copied())
            .unwrap_or(Escape::Local)
    }
}

/// Pre-pass: the bindings whose value flows to an escape position
/// directly as a use of the name — the common `let x = <alloc>; …
/// return x;` indirection — each the declaration the use names
/// (`Snapshot::binding_of`), so an inner shadow of a returned name is
/// its own binding. Closed over `let x = y;` aliases, as borrow
/// lifetime's `returned_decls` is: when x escapes, the declaration y
/// names escapes the same way, so `let p = fresh(); let p = p; return
/// p;` tags the allocating `let`. Walks the body at statement level: a
/// nested block, `if`, loop and a block-bodied match arm are entered; an
/// expression-bodied match arm and an expression block are not.
fn collect_escaping_decls(
    body: &Block,
    ids: &crate::snapshot::Snapshot,
) -> BTreeMap<SiteId, Escape> {
    let mut out: BTreeMap<SiteId, Escape> = BTreeMap::new();
    // `let x = y;`: x's statement and the declaration y names.
    let mut aliases: Vec<(SiteId, SiteId)> = Vec::new();
    collect_escaping_in_block(body, &mut |flow| match flow {
        Flow::Escapes(i, esc) => {
            if let Some(d) = ids.declaration_of(i.id) {
                note_obligation(&mut out, d, esc);
            }
        }
        Flow::Alias { let_id, from } => {
            if let (Some(at), Some(d)) = (ids.site_id(let_id), ids.declaration_of(from.id)) {
                aliases.push((at, d));
            }
        }
    });
    // Close over the aliases, one direction: when `x` is returned or
    // stored, the declaration it aliases carries the same obligation,
    // because the value that leaves is the source's allocation. A SEND
    // is not propagated: `Out <- x` publishes a payload copy and
    // reclaims that copy per dispatch, while the source's own storage
    // stays where it was allocated (outside review of #1291, finding
    // 1: propagating `Sent` marked the source as reclaimed per dispatch
    // and silenced its retained allocation).
    loop {
        let mut changed = false;
        for (alias, source) in &aliases {
            match out.get(alias).copied() {
                Some(esc @ (Escape::Returned | Escape::StoredToSelf)) => {
                    let before = out.get(source).copied();
                    note_obligation(&mut out, *source, esc);
                    changed |= out.get(source).copied() != before;
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    out
}

/// Record an obligation on a declaration. A declaration keeps its
/// first obligation, except that a longer-lived one replaces a send:
/// a value both sent and returned (or stored) is retained until the
/// return, and the per-dispatch reclaim of the published copy must not
/// erase that.
fn note_obligation(out: &mut BTreeMap<SiteId, Escape>, d: SiteId, esc: Escape) {
    match (out.get(&d).copied(), esc) {
        (None, e) => {
            out.insert(d, e);
        }
        (Some(Escape::Sent), e @ (Escape::Returned | Escape::StoredToSelf)) => {
            out.insert(d, e);
        }
        _ => {}
    }
}

/// What the pre-pass walk reports, in body order: a use whose value
/// escapes, and how; or a `let` whose value is a use of another name.
enum Flow<'e> {
    Escapes(&'e hale_syntax::ast::Ident, Escape),
    Alias { let_id: hale_syntax::ast::NodeId, from: &'e hale_syntax::ast::Ident },
}

type EscapeSink<'s> = dyn FnMut(Flow<'_>) + 's;

fn collect_escaping_in_block(b: &Block, out: &mut EscapeSink<'_>) {
    for s in &b.stmts {
        collect_escaping_in_stmt(s, out);
    }
    if let Some(t) = &b.tail {
        note_escape(t, Escape::Returned, out);
    }
}

fn collect_escaping_in_stmt(s: &Stmt, out: &mut EscapeSink<'_>) {
    match s {
        Stmt::Let { value: Expr::Ident(from), id, .. } => out(Flow::Alias { let_id: *id, from }),
        Stmt::Return(Some(e), _) => note_escape(e, Escape::Returned, out),
        Stmt::Fail { value, .. } => note_escape(value, Escape::Returned, out),
        Stmt::Send { value, .. } => note_escape(value, Escape::Sent, out),
        Stmt::Assign { target, value, .. } if target.head.name == "self" => {
            note_escape(value, Escape::StoredToSelf, out)
        }
        Stmt::If(if_stmt) => collect_escaping_in_if(if_stmt, out),
        Stmt::Match(m) => {
            for arm in &m.arms {
                match &arm.body {
                    MatchArmBody::Block(b) => collect_escaping_in_block(b, out),
                    MatchArmBody::Expr(_) => {}
                }
            }
        }
        Stmt::For { body, .. } | Stmt::While { body, .. } => collect_escaping_in_block(body, out),
        Stmt::Block(b) => collect_escaping_in_block(b, out),
        Stmt::ShmWrite { body, .. } => collect_escaping_in_block(body, out),
        _ => {}
    }
}

fn collect_escaping_in_if(if_stmt: &IfStmt, out: &mut EscapeSink<'_>) {
    collect_escaping_in_block(&if_stmt.then_block, out);
    if let Some(else_br) = &if_stmt.else_block {
        match else_br.as_ref() {
            ElseBranch::Else(b) => collect_escaping_in_block(b, out),
            ElseBranch::ElseIf(inner) => collect_escaping_in_if(inner, out),
        }
    }
}

fn note_escape(e: &Expr, esc: Escape, out: &mut EscapeSink<'_>) {
    if let Expr::Ident(id) = e {
        out(Flow::Escapes(id, esc));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    /// A parsed program and the identities minted over it.
    fn minted(src: &str) -> (Program, crate::snapshot::Snapshot) {
        let mut program = parse_source(src).expect("parse");
        let ids = crate::snapshot::mint([("app.hl", &mut program)], &[]);
        (program, ids)
    }

    fn summarize(src: &str) -> AllocSummary {
        let (program, ids) = minted(src);
        summarize_identified(&[(&program, &ids)], &[])
    }

    /// The advisory over `programs` alone, every one minted with `ids`:
    /// no stdlib copy beside them, no renames.
    fn plain_advisory(
        programs: &[&Program],
        ids: &crate::snapshot::Snapshot,
        sources: &[crate::symbol::SourceFile],
        include_all: bool,
    ) -> Vec<Diag> {
        let identified: Vec<_> = programs.iter().map(|p| (*p, ids)).collect();
        let summary = summarize_identified(&identified, &[]);
        unbounded_alloc_diags(&summary, programs, ids, sources, include_all)
    }

    /// The row a call spelling `name` (of `locus`) resolves to.
    fn named(s: &AllocSummary, locus: Option<&str>, name: &str) -> FnKey {
        s.resolve(locus, name).cloned().unwrap_or_else(|| {
            panic!("no row for {:?}::{}; keys = {:?}", locus, name, s.fns.keys().collect::<Vec<_>>())
        })
    }

    fn fns(s: &AllocSummary, locus: Option<&str>, name: &str) -> FnSummary {
        s.fns[&named(s, locus, name)].clone()
    }

    #[test]
    fn struct_returned_is_escaping() {
        let src = r#"
            type P { x: Int; }
            fn make(n: Int) -> P { return P { x: n }; }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "make");
        assert_eq!(f.sites.len(), 1);
        assert!(matches!(f.sites[0].kind, AllocKind::StructLit(_)));
        assert_eq!(f.sites[0].escape, Escape::Returned);
        assert_eq!(f.sites[0].loop_depth, 0);
    }

    #[test]
    fn struct_via_let_then_return_is_escaping() {
        let src = r#"
            type P { x: Int; }
            fn make(n: Int) -> P { let p = P { x: n }; return p; }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "make");
        assert_eq!(f.sites[0].escape, Escape::Returned, "let-bound + returned should escape");
    }

    /// GH #1140, the escape tag's half (F.40 phase 2, use-site
    /// identity): an inner `let` spelling the returned name is its own
    /// binding, so its value is local; only the binding the return names
    /// escapes. Tagged by name, both were `escaping=return`.
    #[test]
    fn an_inner_shadow_of_the_returned_name_is_local() {
        let src = r#"
            type P { x: Int; }
            fn fresh_p(n: Int) -> P { return P { x: n }; }
            fn make(t: Bool) -> P {
                let p = P { x: 1 };
                if t { let p = fresh_p(2); }
                while t { let p = P { x: 3 }; }
                return p;
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "make");
        let escapes: Vec<Escape> = f.sites.iter().map(|s| s.escape).collect();
        assert_eq!(escapes, vec![Escape::Returned, Escape::Local], "the outer literal, the loop's: {:?}", f.sites);
        let call = f
            .calls
            .iter()
            .find(|c| c.callee == Callee::Resolved(named(&s, None, "fresh_p")))
            .expect("the call in the `if`");
        assert_eq!(call.escape, Escape::Local, "the inner shadow's value");
    }

    /// F.40 phase 2 review F3: escape follows `let x = y;` aliases back
    /// to the allocating declaration, whether the alias rebinds the same
    /// name or spells another; a pattern binding of the returned name in
    /// an arm stays its own.
    #[test]
    fn the_escape_tag_follows_a_rebind_to_the_allocating_let() {
        let src = r#"
            type P { x: Int; }
            fn fresh_p(n: Int) -> P { return P { x: n }; }
            fn rebind() -> P { let p = fresh_p(1); let p = p; return p; }
            fn alias() -> P { let p = fresh_p(2); let q = p; let r = q; return r; }
            fn kept(t: Bool) -> P {
                let p = fresh_p(3);
                if t { let q = p; println(q.x); }
                return P { x: 4 };
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let call_escape = |f: &str| {
            fns(&s, None, f)
                .calls
                .iter()
                .find(|c| c.callee == Callee::Resolved(named(&s, None, "fresh_p")))
                .expect("the fresh_p call")
                .escape
        };
        assert_eq!(call_escape("rebind"), Escape::Returned, "`let p = p` hands the first binding's value back");
        assert_eq!(call_escape("alias"), Escape::Returned, "a chain of aliases closes");
        assert_eq!(call_escape("kept"), Escape::Local, "an alias that does not escape escapes nothing");
    }

    /// Outside review of #1291, finding 1: sending an alias publishes a
    /// payload copy and must not mark the source allocation as reclaimed
    /// per dispatch; a value both sent and returned is retained until the
    /// return, whichever is written first.
    #[test]
    fn a_sent_alias_leaves_the_source_allocation_where_it_is() {
        let src = r#"
            type P { x: Int; }
            topic Out { payload: P; subject: "out"; }
            fn fresh_p(n: Int) -> P { return P { x: n }; }
            fn sent_alias() { let p = fresh_p(1); let q = p; Out <- q; }
            fn sent_then_returned() -> P { let p = fresh_p(2); let q = p; Out <- q; return p; }
            fn sent_direct_then_aliased_return() -> P { let p = fresh_p(3); Out <- p; let q = p; return q; }
            fn main() { }
        "#;
        let s = summarize(src);
        let call_escape = |f: &str| {
            fns(&s, None, f)
                .calls
                .iter()
                .find(|c| c.callee == Callee::Resolved(named(&s, None, "fresh_p")))
                .expect("the fresh_p call")
                .escape
        };
        assert_eq!(call_escape("sent_alias"), Escape::Local, "the published copy is the alias's; the source stays local");
        assert_eq!(call_escape("sent_then_returned"), Escape::Returned, "a send cannot erase the return");
        assert_eq!(call_escape("sent_direct_then_aliased_return"), Escape::Returned, "a direct send first, then a returned alias: the return wins");
    }

    #[test]
    fn local_struct_in_loop_is_local_with_depth() {
        let src = r#"
            type P { x: Int; }
            fn run_it() {
                let mut i = 0;
                while i < 10 {
                    let p = P { x: i };
                    i = i + 1;
                }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "run_it");
        let st = f.sites.iter().find(|s| matches!(s.kind, AllocKind::StructLit(_))).expect("struct site");
        assert_eq!(st.escape, Escape::Local);
        assert_eq!(st.loop_depth, 1);
        // `while i < 10 { … i = i + 1 }` is a const-bounded counter
        // (loop-ranking), so the loop ranks WhileCounter and the in-loop
        // struct accumulates by a constant, not unboundedly.
        assert!(f.loops.iter().any(|l| matches!(l.kind, LoopKind::WhileCounter)));
        assert_eq!(st.verdict(), SiteVerdict::AccumulatesBoundedLoop);
    }

    #[test]
    fn while_counter_ranking_is_sound() {
        let verdict = |src: &str| {
            let s = summarize(src);
            let f = fns(&s, None, "run_it");
            f.sites
                .iter()
                .find(|s| matches!(s.kind, AllocKind::StructLit(_)))
                .expect("struct site")
                .verdict()
        };
        // (a) const init + only positive const increment → bounded.
        assert_eq!(
            verdict(
                r#"type Q { a: Int; }
                   fn run_it() { let mut i = 0; while i < 100 { let q = Q { a: i }; let _ = q; i = i + 1; } }
                   fn main() { }"#
            ),
            SiteVerdict::AccumulatesBoundedLoop,
            "const counter must rank bounded"
        );
        // (b) a reset in the body breaks monotonicity → unbounded.
        assert_eq!(
            verdict(
                r#"type Q { a: Int; }
                   fn run_it() { let mut i = 0; while i < 100 { let q = Q { a: i }; let _ = q; if i == 50 { i = 0; } i = i + 1; } }
                   fn main() { }"#
            ),
            SiteVerdict::AccumulatesUnbounded,
            "a counter reset must stay unbounded (no false bounded)"
        );
        // (c) a runtime (non-const) increment → unbounded.
        assert_eq!(
            verdict(
                r#"type Q { a: Int; }
                   fn run_it(step: Int) { let mut i = 0; while i < 100 { let q = Q { a: i }; let _ = q; i = i + step; } }
                   fn main() { }"#
            ),
            SiteVerdict::AccumulatesUnbounded,
            "a runtime step must stay unbounded"
        );
        // (d) a runtime (non-const) initial value → unbounded.
        assert_eq!(
            verdict(
                r#"type Q { a: Int; }
                   fn run_it(start: Int) { let mut i = start; while i < 100 { let q = Q { a: i }; let _ = q; i = i + 1; } }
                   fn main() { }"#
            ),
            SiteVerdict::AccumulatesUnbounded,
            "a runtime init must stay unbounded"
        );
    }

    #[test]
    fn r1_escape_awareness_on_the_cross_invocation_path() {
        // (a) a per-message handler builds a transient it does NOT store →
        // reclaimed at the per-delivery method-scratch destroy → not a leak.
        let transient = r#"
            type T { n: Int; }
            type Tmp { a: Int; b: Int; }
            locus L {
                params { last: Int = 0; }
                bus { subscribe "in" as on_in of type T; }
                fn on_in(m: T) { let tmp = Tmp { a: m.n, b: m.n }; self.last = tmp.a; }
            }
            fn main() { }
        "#;
        assert!(
            summarize(transient).leak_sites().is_empty(),
            "a non-escaping handler local is reclaimed per delivery — must not flag"
        );

        // (b) the same handler storing the struct INTO self. Gap D
        // (2026-07-17) split this by field types: an ALL-scalar/String
        // struct replace fully reclaims (in-place memcpy + Gap A anchor
        // retirement at the activation boundary, RSS-proven flat), so
        // it is no longer flagged...
        let stored_retirable = r#"
            type T { n: Int; }
            type Tmp { a: Int; b: Int; }
            locus L {
                params { last: Tmp; }
                bus { subscribe "in" as on_in of type T; }
                fn on_in(m: T) { self.last = Tmp { a: m.n, b: m.n }; }
            }
            fn main() { }
        "#;
        assert!(
            summarize(stored_retirable).leak_sites().is_empty(),
            "an all-scalar struct replace reclaims per activation (Gap A \
             retirement) — must NOT flag"
        );
        // ...while a struct carrying a non-retirable leaf (Bytes /
        // nested compound) keeps the conservative escape verdict —
        // the escape-awareness this test exists to pin.
        let stored_leaky = r#"
            type T { n: Int; }
            type Tmp { a: Int; b: Bytes; }
            locus L {
                params { last: Tmp; }
                bus { subscribe "in" as on_in of type T; }
                fn on_in(m: T) { self.last = Tmp { a: m.n, b: std::bytes::from_string("x") }; }
            }
            fn main() { }
        "#;
        assert_eq!(
            summarize(stored_leaky).leak_sites().len(),
            1,
            "a self-stored alloc with a non-retirable leaf persists \
             across deliveries — must flag"
        );

        // (c) a non-escaping local in an UNBOUNDED LOOP inside the handler
        // accumulates within the (never-returning) call → still flagged.
        // R1 only changes the cross-invocation path, not the in-loop one.
        let in_loop = r#"
            type T { n: Int; }
            type Tmp { a: Int; }
            locus L {
                bus { subscribe "in" as on_in of type T; }
                fn on_in(m: T) { while true { let tmp = Tmp { a: m.n }; let _ = tmp.a; } }
            }
            fn main() { }
        "#;
        assert_eq!(
            summarize(in_loop).leak_sites().len(),
            1,
            "a non-escaping local in an unbounded loop still accumulates within the call"
        );
    }

    #[test]
    fn bounded_for_range_is_detected() {
        let src = r#"
            fn loopy() { for i in 0..8 { let _ = i; } }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "loopy");
        assert!(matches!(f.loops[0].kind, LoopKind::ForRange { bounded: Some(8) }));
    }

    #[test]
    fn bus_handler_is_classified_unbounded() {
        let src = r#"
            type Tick { n: Int; }
            locus C {
                bus { subscribe "t" as on_tick of type Tick; }
                fn on_tick(t: Tick) { let _ = t.n; }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "on_tick");
        assert_eq!(f.entry, Some(EntryKind::BusHandler));
        assert!(!f.entry.unwrap().one_shot());
    }

    #[test]
    fn run_lifecycle_is_collected_and_one_shot() {
        let src = r#"
            locus C {
                run { let mut i = 0; while true { i = i + 1; } }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "run");
        assert_eq!(f.entry, Some(EntryKind::Run));
        assert!(f.loops.iter().any(|l| matches!(l.kind, LoopKind::WhileTrue)));
    }

    #[test]
    fn self_method_call_resolves() {
        let src = r#"
            locus C {
                fn helper() -> Int { return 1; }
                fn use_it() -> Int { return self.helper(); }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "use_it");
        assert!(f.calls.iter().any(|c| c.callee == Callee::Resolved(named(&s, Some("C"), "helper"))));
    }

    #[test]
    fn struct_in_while_true_is_leak_precursor() {
        // The empirically-validated model: a value alloc in an unbounded
        // loop accumulates (reclaim@locus-dissolve), so the verdict is
        // ACCUMULATES-UNBOUNDED — even though it never escapes.
        let src = r#"
            type Q { a: Int; }
            locus C {
                run { let mut i = 0; while true { let q = Q { a: i }; i = i + 1; } }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "run");
        let st = f.sites.iter().find(|s| matches!(s.kind, AllocKind::StructLit(_))).expect("struct");
        assert_eq!(st.reclaim, ReclaimScope::EnclosingLocus);
        assert!(st.in_unbounded_loop);
        assert_eq!(st.verdict(), SiteVerdict::AccumulatesUnbounded);
    }

    #[test]
    fn fail_payload_in_unbounded_loop_is_once_per_invocation() {
        // `fail` diverges: however many iterations the loop runs, the
        // payload allocates at most once per invocation. The 40-advisory
        // false-positive class from strict parsers (`fail E { … }` inside
        // `while`) must not flag.
        let src = r#"
            type E { code: Int; }
            fn pump(n: Int) -> Int fallible(E) {
                while true {
                    if n == 1 { fail E { code: n }; }
                }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "pump");
        let st = f
            .sites
            .iter()
            .find(|s| matches!(s.kind, AllocKind::StructLit(_)))
            .expect("fail payload site");
        assert_eq!(st.escape, Escape::Returned, "payload still escapes to the caller");
        assert!(!st.in_unbounded_loop, "divergence clears the loop context");
        assert_eq!(st.loop_depth, 0);
        assert_eq!(st.verdict(), SiteVerdict::OncePerInvocation);
    }

    #[test]
    fn return_payload_in_unbounded_loop_is_once_per_invocation() {
        // Same divergence class as `fail`: a `return` in a loop executes
        // at most once per invocation.
        let src = r#"
            type Q { a: Int; }
            fn find(n: Int) -> Q {
                let mut i = 0;
                while i < n {
                    if i == 7 { return Q { a: i }; }
                    i = i + 1;
                }
                return Q { a: 0 };
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "find");
        for st in f.sites.iter().filter(|s| matches!(s.kind, AllocKind::StructLit(_))) {
            assert_eq!(st.verdict(), SiteVerdict::OncePerInvocation);
        }
    }

    #[test]
    fn divergence_exemption_is_site_scoped() {
        // A genuinely-accumulating alloc in the same loop as a `fail`
        // must still flag — only the fail/return payload itself is
        // at-most-once.
        let src = r#"
            type Q { a: Int; }
            type E { code: Int; }
            fn pump(n: Int) -> Int fallible(E) {
                while true {
                    let q = Q { a: n };
                    let _ = q.a;
                    if n == 1 { fail E { code: n }; }
                }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "pump");
        let q = f
            .sites
            .iter()
            .find(|s| matches!(&s.kind, AllocKind::StructLit(name) if name == "Q"))
            .expect("Q site");
        assert_eq!(q.verdict(), SiteVerdict::AccumulatesUnbounded);
        let e = f
            .sites
            .iter()
            .find(|s| matches!(&s.kind, AllocKind::StructLit(name) if name == "E"))
            .expect("E site");
        assert_eq!(e.verdict(), SiteVerdict::OncePerInvocation);
    }

    #[test]
    fn struct_in_bounded_for_is_bounded() {
        let src = r#"
            type Q { a: Int; }
            fn work() { for i in 0..100 { let q = Q { a: i }; } }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "work");
        let st = f.sites.iter().find(|s| matches!(s.kind, AllocKind::StructLit(_))).expect("struct");
        assert!(!st.in_unbounded_loop);
        assert_eq!(st.verdict(), SiteVerdict::AccumulatesBoundedLoop);
    }

    #[test]
    fn struct_outside_loop_is_once_per_invocation() {
        let src = r#"
            type Q { a: Int; }
            fn make() -> Q { return Q { a: 1 }; }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "make");
        assert_eq!(f.sites[0].verdict(), SiteVerdict::OncePerInvocation);
    }

    #[test]
    fn bus_send_in_loop_reclaims_per_iteration() {
        // A sent value routes to the payload arena (per-dispatch reclaim),
        // so even in an unbounded loop its verdict is per-iteration-bounded.
        let src = r#"
            type Q { a: Int; }
            locus C {
                bus { publish "t" of type Q; }
                run {
                    let mut i = 0;
                    while true { let q = Q { a: i }; "t" <- q; i = i + 1; }
                }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "run");
        // The `q` bound by `let` is tagged Sent (it flows to the send), so
        // its alloc reclaims at dispatch.
        let sent = f.sites.iter().find(|s| s.reclaim == ReclaimScope::AfterBusDispatch);
        assert!(sent.is_some(), "expected a bus-dispatch-reclaimed site");
        assert_eq!(sent.unwrap().verdict(), SiteVerdict::PerIterationReclaim);
    }

    #[test]
    fn struct_stored_to_self_escapes() {
        let src = r#"
            type P { x: Int; }
            locus C {
                params { p: P; }
                fn set(n: Int) { self.p = P { x: n }; }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "set");
        assert_eq!(f.sites[0].escape, Escape::StoredToSelf);
    }

    // ---- step 3: call-graph propagation + the bound solver ----

    #[test]
    fn alloc_in_fn_called_in_unbounded_loop_is_flagged() {
        // The JSON leak class: an allocating helper called in a hot loop.
        // The helper's own body has no loop, but it's invoked unboundedly.
        let src = r#"
            type Q { a: Int; }
            fn make(n: Int) -> Q { return Q { a: n }; }
            locus C { run { let mut i = 0; while true { let q = make(i); i = i + 1; } } }
            fn main() { }
        "#;
        let s = summarize(src);
        assert!(s.unbounded_invoked().contains(&named(&s, None, "make")));
        let leaks = s.leak_sites();
        assert!(
            leaks.iter().any(|l| l.owner == named(&s, None, "make")
                && l.reason == LeakReason::InvokedUnboundedly),
            "make's alloc should be flagged via call-graph propagation; got {:?}",
            leaks
        );
    }

    #[test]
    fn alloc_in_fn_called_once_is_not_flagged() {
        let src = r#"
            type Q { a: Int; }
            fn make(n: Int) -> Q { return Q { a: n }; }
            fn main() { let q = make(1); }
        "#;
        let s = summarize(src);
        assert!(
            s.leak_sites().is_empty(),
            "a once-called allocation must not be flagged: {:?}",
            s.leak_sites()
        );
    }

    #[test]
    fn sent_alloc_in_handler_is_not_flagged() {
        // A per-message handler that only sends its allocation: bounded,
        // because the payload arena reclaims per dispatch.
        let src = r#"
            type Q { a: Int; }
            locus C {
                bus { subscribe "in" as on_in of type Q; publish "out" of type Q; }
                fn on_in(m: Q) { let q = Q { a: m.a }; "out" <- q; }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        assert!(
            s.leak_sites().is_empty(),
            "a sent allocation in a handler reclaims per dispatch: {:?}",
            s.leak_sites()
        );
    }

    #[test]
    fn diags_are_located_and_nonempty_for_a_leak() {
        let src = r#"
            type Q { a: Int; }
            locus C { run { while true { let q = Q { a: 1 }; } } }
            fn main() { }
        "#;
        let (program, ids) = minted(src);
        // Survey mode (`--warn-unbounded-alloc`) reports the leak even
        // though locus `C` carries no `@bounded` opt-in.
        let diags = plain_advisory(&[&program], &ids, &[], true);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("unbounded allocation"));
    }

    #[test]
    fn bounded_locus_reports_without_the_survey_flag() {
        // The same leak, but the locus opts in with `@bounded`. Now the
        // default (non-survey) scope reports it.
        let src = r#"
            type Q { a: Int; }
            @bounded locus C { run { while true { let q = Q { a: 1 }; } } }
            fn main() { }
        "#;
        let (program, ids) = minted(src);
        let scoped = plain_advisory(&[&program], &ids, &[], false);
        assert_eq!(scoped.len(), 1, "@bounded locus reports by default");
        assert!(scoped[0].message.contains("unbounded allocation"));
    }

    #[test]
    fn unbounded_fn_carves_out_even_in_a_bounded_locus() {
        // `@bounded` on the locus opts in; `@unbounded` on the method opts
        // that one method back out — silent in survey mode AND scoped mode.
        let src = r#"
            type Q { a: Int; }
            @bounded locus C {
                @unbounded run { while true { let q = Q { a: 1 }; } }
            }
            fn main() { }
        "#;
        let (program, ids) = minted(src);
        assert!(
            plain_advisory(&[&program], &ids, &[], true).is_empty(),
            "@unbounded suppresses the site under the survey flag"
        );
        assert!(
            plain_advisory(&[&program], &ids, &[], false).is_empty(),
            "@unbounded suppresses the site in @bounded scope too"
        );
    }

    /// F.40 phase 2 review F1: a finding is dropped per site, when the
    /// site has no author position, and for no other reason. A generated
    /// parser's offsets coincide with the author's file (json_gen parses
    /// at 0) yet are not its text; source the api binding generates sits
    /// at `API_SYNTH_BASE`; the author's own fn beside them still reports.
    #[test]
    fn a_site_with_no_author_position_is_dropped_and_nothing_else() {
        let src = r#"
            type Q { a: Int; }
            fn __json_parse_Q() { while true { let q = Q { a: 1 }; } }
            fn author_fn() { while true { let q = Q { a: 2 }; } }
            locus C { run { __json_parse_Q(); author_fn(); } }
            fn main() { }
        "#;
        let (program, ids) = minted(src);
        let file = crate::symbol::SourceFile {
            id: 0,
            path: "app.hl".into(),
            digest: String::new(),
            base: 0,
            len: src.len() as u32,
        };
        let all = summarize(src).leak_sites();
        assert_eq!(all.len(), 2, "both loops leak before the rule: {all:?}");
        let diags = plain_advisory(&[&program], &ids, &[file], true);
        assert_eq!(diags.len(), 1, "only the author's site reports: {diags:?}");
        assert!(diags[0].message.contains("`author_fn`"), "{}", diags[0].message);

        // The same locus parsed at 0 and at the api binding's base, beside
        // the author's program, in a harness bundle (no origin rows): the
        // span alone decides.
        let synth = "locus S { run { while true { let q = Q { a: 3 }; } } }\n";
        let harness = crate::snapshot::Snapshot::default();
        let reported = |base: u32| {
            let generated = hale_syntax::parse_source_at(synth, base).expect("parse");
            plain_advisory(&[&program, &generated], &harness, &[], true).len()
        };
        assert_eq!(reported(1 << 20), 3, "at an author offset, S's site reports");
        assert_eq!(
            reported(hale_syntax::api_gen::API_SYNTH_BASE),
            2,
            "at API_SYNTH_BASE, S's site has no position"
        );
    }

    // === Gap D (2026-07-17): anchor-retirement verdict flip ===========

    fn leak_msgs(src: &str) -> Vec<String> {
        let (program, ids) = minted(src);
        plain_advisory(&[&program], &ids, &[], true)
            .into_iter()
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn retired_struct_replace_in_method_not_flagged() {
        // Gap A retires the replaced String clones of an all-scalar/
        // String struct at the method activation boundary — a method
        // invoked unboundedly from a run-loop no longer accumulates
        // (RSS-proven flat). The verdict model must not flag it.
        let src = r#"
            type Cell { s: String = ""; t: String = ""; n: Int = 0; }
            locus Rec {
                params { st: Cell = Cell { }; }
                fn record(i: Int) {
                    self.st = Cell { s: "v" + i, t: "" + i, n: i };
                }
                run() {
                    while true { self.record(1); }
                }
            }
            fn main() { Rec { }; }
        "#;
        let msgs = leak_msgs(src);
        assert!(
            !msgs.iter().any(|m| m.contains("struct Cell")),
            "retired struct replace must not be flagged, got: {:?}",
            msgs
        );
    }

    #[test]
    fn const_expr_ceiling_ranks_bounded() {
        // iris handoff P2.1: `while i < NET_SLOTS * WINDOW` with both
        // top-level consts is a const-bounded counter, not a runtime
        // while.
        let src = r#"
            const NET_SLOTS: Int = 16;
            const WINDOW: Int = 4;
            locus K {
                params { seen: Int = 0; }
                bus { subscribe "k" as on_k of type Int; }
                birth() {
                    let mut i = 0;
                    while i < NET_SLOTS * WINDOW {
                        let s = "x" + i;
                        self.seen = self.seen + len(s);
                        i = i + 1;
                    }
                }
                fn on_k(n: Int) { self.seen = self.seen + n; }
            }
            fn main() { }
        "#;
        let msgs = leak_msgs(src);
        assert!(
            msgs.is_empty(),
            "const-expr ceiling must rank bounded, got: {:?}",
            msgs
        );
    }

    #[test]
    fn eager_child_in_run_loop_not_flagged() {
        // iris handoff P2.2: a bare-statement child instantiated per
        // iteration dissolves at the statement — neither the
        // instantiation site nor the child's own self-stores
        // accumulate, even under the parent's `while true`.
        let src = r#"
            locus Cycle {
                params { n: Int = 0; work: String = ""; }
                birth() {
                    let mut i = 0;
                    while i < 8 {
                        self.work = self.work + "x";
                        i = i + 1;
                    }
                }
            }
            locus Driver {
                params { r: Int = 0; }
                bus { subscribe "tick" as on_t of type Int; }
                run() {
                    while true {
                        Cycle { n: self.r };
                        self.r = self.r + 1;
                    }
                }
                fn on_t(n: Int) { self.r = self.r + n; }
            }
            fn main() { }
        "#;
        let msgs = leak_msgs(src);
        assert!(
            !msgs.iter().any(|m| m.contains("Cycle")),
            "eager per-iteration child must not flag, got: {:?}",
            msgs
        );
    }

    #[test]
    fn let_bound_child_in_run_loop_still_flagged() {
        // The counterpart: a LET-BOUND instantiation defers to method
        // exit — under `while true` that is a real accumulation and
        // must keep flagging.
        let src = r#"
            locus Cycle {
                params { n: Int = 0; }
            }
            locus Driver {
                params { r: Int = 0; }
                bus { subscribe "tick" as on_t of type Int; }
                run() {
                    while true {
                        let c = Cycle { n: self.r };
                        self.r = self.r + c.n;
                    }
                }
                fn on_t(n: Int) { self.r = self.r + n; }
            }
            fn main() { }
        "#;
        let msgs = leak_msgs(src);
        assert!(
            msgs.iter().any(|m| m.contains("Cycle")),
            "let-bound child in while-true must stay flagged, got: {:?}",
            msgs
        );
    }

    #[test]
    fn bytes_field_struct_replace_still_flagged() {
        // Bytes fields don't retire (v1) — the conservative verdict
        // stays for structs carrying one.
        let src = r#"
            type Cell { s: String = ""; b: Bytes; n: Int = 0; }
            locus Rec {
                params { st: Cell; }
                fn record(i: Int) {
                    self.st = Cell { s: "v" + i, b: std::bytes::from_string("x"), n: i };
                }
                run() {
                    while true { self.record(1); }
                }
            }
            fn main() { }
        "#;
        let msgs = leak_msgs(src);
        assert!(
            msgs.iter().any(|m| m.contains("struct Cell")),
            "Bytes-field struct replace must stay flagged, got: {:?}",
            msgs
        );
    }

    #[test]
    fn retired_struct_replace_in_run_loop_direct_still_flagged() {
        // Directly inside run()'s `while true` there is no activation
        // boundary — pending retires never flush, so the store still
        // accumulates and must stay flagged.
        let src = r#"
            type Cell { s: String = ""; n: Int = 0; }
            locus Rec {
                params { st: Cell = Cell { }; }
                run() {
                    let mut i = 0;
                    while true {
                        self.st = Cell { s: "v" + i, n: i };
                        i = i + 1;
                    }
                }
            }
            fn main() { Rec { }; }
        "#;
        let msgs = leak_msgs(src);
        assert!(
            msgs.iter().any(|m| m.contains("struct Cell")),
            "run-loop direct struct replace must stay flagged, got: {:?}",
            msgs
        );
    }

    // === Phase D / D1: storage-shape + slot/field identity capture =====

    #[test]
    fn d1_captures_capacity_slots_and_form() {
        let src = r#"
            type Entry { k: Int; }
            @form(hashmap, cap = 1024)
            locus Reg { capacity { pool entries of Entry indexed_by k; } }
            locus Plain { capacity { heap log of Entry; } }
            fn main() { }
        "#;
        let s = summarize(src);
        let reg = s.locus_shapes.get("Reg").expect("Reg shape");
        assert_eq!(reg.capacity_slots.len(), 1);
        assert_eq!(reg.capacity_slots[0].name, "entries");
        assert!(matches!(reg.capacity_slots[0].kind, CapacitySlotKind::Pool));
        let form = reg.form.as_ref().expect("@form captured");
        assert_eq!(form.name, "hashmap");
        assert_eq!(form.cap, Some(1024), "cap form-arg captured as a literal");

        let plain = s.locus_shapes.get("Plain").expect("Plain shape");
        assert!(matches!(plain.capacity_slots[0].kind, CapacitySlotKind::Heap));
        assert!(plain.form.is_none());
    }

    #[test]
    fn d1_captures_recognition_cap() {
        let src = r#"
            type Leaf { v: Int; }
            locus Coord : projection recognition(cap = 64, fixed_cell) {
                contract { consume value: Int; }
                accept(c: Leaf) { }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let coord = s.locus_shapes.get("Coord").expect("Coord shape");
        assert_eq!(coord.recognition_cap, Some(64));
    }

    #[test]
    fn d1_store_latest_records_target_field() {
        // M3 stage 5 gap C: the ALL-SCALAR variant of this store is
        // now carved out (in-place memcpy, no arena growth), so the
        // struct here carries a fresh heap subfield to stay a site.
        let src = r#"
            type Q { a: Int; s: String; }
            locus C {
                params { latest: Q = Q { a: 0, s: "" }; }
                run { while true { self.latest = Q { a: 1, s: to_string(1) }; } }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "run");
        let st = f.sites.iter().find(|s| matches!(s.kind, AllocKind::StructLit(_))).expect("struct");
        assert_eq!(st.escape, Escape::StoredToSelf);
        assert_eq!(
            st.target_field.as_deref(),
            Some("latest"),
            "whole-value `self.latest = …` replace records the field"
        );
        // The verdict is unchanged by D1 — this is still a slot-0 leak.
        assert_eq!(st.verdict(), SiteVerdict::AccumulatesUnbounded);
    }

    #[test]
    fn d1_indexed_inplace_write_has_no_target_field() {
        // An indexed in-place write allocates nothing; a *local* struct in
        // the same body must not pick up a target field.
        let src = r#"
            type Q { a: Int; }
            locus C {
                params { recent: [Int; 4] = [0,0,0,0]; }
                run {
                    while true {
                        self.recent[0] = 1;
                        let q = Q { a: 2 };
                    }
                }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "run");
        let st = f.sites.iter().find(|s| matches!(s.kind, AllocKind::StructLit(_))).expect("struct");
        assert_eq!(st.escape, Escape::Local);
        assert_eq!(st.target_field, None, "a local alloc has no self-field target");
    }

    #[test]
    fn d1_slot_insert_records_receiver_slot() {
        let src = r#"
            type Q { a: Int; }
            locus C {
                capacity { heap log of Q; }
                run { let c = self.log.alloc(); }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("C"), "run");
        let call = f.calls.iter().find(|c| c.receiver_slot.is_some()).expect("slot call");
        assert_eq!(call.receiver_slot.as_deref(), Some("log"));
    }

    // === Phase D / D2: growing-collection (@form vec/hashmap) inserts =====

    fn insert_site(f: &FnSummary) -> Option<&AllocSite> {
        f.sites.iter().find(|s| matches!(s.kind, AllocKind::CollectionInsert(_)))
    }

    #[test]
    fn d2_vec_field_push_in_handler_is_unbounded() {
        let src = r#"
            @form(vec) locus IntVec { capacity { heap items of Int; } }
            locus W {
                params { buf: IntVec = IntVec { }; }
                bus { subscribe "ev" as on_ev of type Int; }
                fn on_ev(x: Int) { self.buf.push(x); }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("W"), "on_ev");
        let site = insert_site(&f).expect("vec-insert site");
        assert_eq!(site.kind, AllocKind::CollectionInsert("vec".into()));
        // A per-message handler is an unbounded-invocation context.
        let leaks = s.leak_sites();
        assert!(
            leaks.iter().any(|l| matches!(l.kind, AllocKind::CollectionInsert(_))),
            "vec push in a per-message handler should be flagged"
        );
    }

    #[test]
    fn d2_vec_param_push_called_once_is_not_flagged() {
        let src = r#"
            @form(vec) locus IntVec { capacity { heap items of Int; } }
            fn double_push(v: IntVec, n: Int) { v.push(n); v.push(n); }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, None, "double_push");
        // The insert is recorded (typed param resolved)…
        assert!(insert_site(&f).is_some(), "param-typed vec push is detected");
        // …but called once, not in a loop → not a leak.
        assert_eq!(insert_site(&f).unwrap().verdict(), SiteVerdict::OncePerInvocation);
        assert!(s.leak_sites().is_empty(), "a call-once push is bounded");
    }

    #[test]
    fn d2_ring_buffer_push_is_not_flagged() {
        // ring_buffer is cap-bounded — its push is not a growing insert.
        let src = r#"
            @form(ring_buffer, cap = 16) locus Ring { capacity { pool slots of Int; } }
            locus W {
                params { r: Ring = Ring { }; }
                bus { subscribe "ev" as on_ev of type Int; }
                fn on_ev(x: Int) { self.r.push(x); }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("W"), "on_ev");
        assert!(insert_site(&f).is_none(), "ring_buffer push must not flag");
        assert!(s.leak_sites().is_empty());
    }

    #[test]
    fn d2_bounded_loop_push_is_not_a_leak() {
        let src = r#"
            @form(vec) locus IntVec { capacity { heap items of Int; } }
            locus W {
                params { buf: IntVec = IntVec { }; }
                run { let mut i = 0; while i < 4 { self.buf.push(i); i = i + 1; } }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("W"), "run");
        let site = insert_site(&f).expect("insert detected");
        assert_eq!(
            site.verdict(),
            SiteVerdict::AccumulatesBoundedLoop,
            "a const-bounded loop bounds the inserts"
        );
        assert!(s.leak_sites().is_empty());
    }

    #[test]
    fn d2_user_method_named_push_on_non_form_locus_is_not_flagged() {
        // `Segment` is a plain locus with a user `push` method — not a form.
        // (The `54-geom-leading-edge` corpus shape.) Must not flag.
        let src = r#"
            locus Segment {
                fn push(t: Int) { }
            }
            locus W {
                params { seg: Segment = Segment { }; }
                run { while true { self.seg.push(1); } }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("W"), "run");
        assert!(insert_site(&f).is_none(), "user push on a non-form locus is not an insert");
        assert!(s.leak_sites().is_empty());
    }

    #[test]
    fn d2_typed_let_receiver_resolves() {
        let src = r#"
            @form(hashmap) locus Map { capacity { pool entries of Int; } }
            locus W {
                run {
                    let m: Map = Map { };
                    while true { m.set(1); }
                }
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, Some("W"), "run");
        let site = insert_site(&f).expect("typed-let hashmap insert detected");
        assert_eq!(site.kind, AllocKind::CollectionInsert("hashmap".into()));
        assert_eq!(site.verdict(), SiteVerdict::AccumulatesUnbounded);
    }

    /// The calls a fn's body writes, as (callee, local, indirect), in
    /// the order written.
    ///
    /// The walk's own reading: an indirect call the summary went on to
    /// resolve to the program's function values (`via_value`) is the one
    /// indirect call it was written as ([`value_calls`] reads the
    /// alternatives).
    fn local_calls(src: &str, f: &str) -> Vec<(String, Option<String>, bool)> {
        let s = summarize(src);
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        fns(&s, None, f)
            .calls
            .iter()
            .filter(|c| c.via_value.is_none() || c.dispatch_group.is_some_and(|g| seen.insert(g)))
            .map(|c| match &c.via_value {
                Some(written) => (format!("?{written}"), None, true),
                None => {
                    let callee = match &c.callee {
                        Callee::Resolved(k) => k.display(),
                        Callee::Unresolved(n) => format!("?{n}"),
                    };
                    (callee, c.via_local.clone(), c.indirect)
                }
            })
            .collect()
    }

    /// The calls a fn's body writes, an indirect call resolved to the
    /// program's function values as its alternatives: (callee, the
    /// callee as written when it is one).
    fn value_calls(src: &str, f: &str) -> Vec<(String, Option<String>)> {
        let s = summarize(src);
        fns(&s, None, f)
            .calls
            .iter()
            .map(|c| {
                let callee = match &c.callee {
                    Callee::Resolved(k) => k.display(),
                    Callee::Unresolved(n) => format!("?{n}"),
                };
                (callee, c.via_value.clone())
            })
            .collect()
    }

    /// E5: an indirect call resolves to the program's function values of
    /// its arity: a fn read as a value anywhere (an argument, a params
    /// default, a struct literal), but not a fn only called, nor a name
    /// a local shadows. A call with no such value stays indirect.
    #[test]
    fn an_indirect_call_resolves_to_the_function_values_of_its_arity() {
        let src = r#"
            fn one() -> Int { return 1; }
            fn two() -> Int { return 2; }
            fn called() -> Int { return 3; }
            fn inc(n: Int) -> Int { return n + 1; }
            fn shadowed() -> Int { return 4; }
            type Slot { f: fn() -> Int; }
            locus K { params { s: Slot = Slot { f: two }; } }
            fn g(c: Bool) -> Int {
                let f = if c { one } else { called() };
                let shadowed = 0;
                let k = shadowed;
                let h = inc;
                return f() + called() + h(k);
            }
            fn binary(p: fn(Int, Int) -> Int) -> Int { return p(1, 2); }
            fn main() { }
        "#;
        let v = |c: &str, w: Option<&str>| (c.to_string(), w.map(str::to_string));
        assert_eq!(
            value_calls(src, "g"),
            vec![v("called", None), v("one", Some("f")), v("two", Some("f")), v("called", None), v("inc", None)],
        );
        assert_eq!(value_calls(src, "binary"), vec![v("?p", None)], "no value takes two parameters");
    }

    /// A call through a function-typed parameter keeps the values whose
    /// declared signature can be the parameter's: a different primitive,
    /// or a declared nominal type against a primitive, is not; a type
    /// alias or a generic is anything.
    #[test]
    fn a_parameters_declared_type_narrows_its_values() {
        let src = r#"
            type Id = Int;
            type Tag { n: Int; }
            fn by_int(n: Int) -> Int { return n; }
            fn by_string(s: String) -> Int { return 0; }
            fn by_tag(t: Tag) -> Int { return t.n; }
            fn by_id(i: Id) -> Int { return i; }
            fn to_string_(n: Int) -> String { return ""; }
            fn apply(f: fn(Int) -> Int, v: Int) -> Int { return f(v); }
            fn main() {
                let fs = [by_int, by_int];
                let a = by_string;
                let b = by_tag;
                let c = by_id;
                let d = to_string_;
            }
        "#;
        let v = |c: &str| (c.to_string(), Some("f".to_string()));
        assert_eq!(value_calls(src, "apply"), vec![v("by_id"), v("by_int")]);
    }

    fn through(callee: &str, local: &str) -> (String, Option<String>, bool) {
        (callee.to_string(), Some(local.to_string()), false)
    }

    fn written(callee: &str) -> (String, Option<String>, bool) {
        (callee.to_string(), None, false)
    }

    /// A call through a function value the walk does not name: indirect
    /// (F.40 E5, a classified correction: through a local, it was the
    /// edge as written and a call to nothing).
    fn indirect(callee: &str) -> (String, Option<String>, bool) {
        (callee.to_string(), None, true)
    }

    /// A call through a local bound to a fn name or path reaches what a
    /// direct call of the name or path reaches, the local on the edge
    /// (P3 2 of 3, a classified correction: the edge was `Unresolved`
    /// with the local's name, a call to nothing).
    #[test]
    fn a_call_through_a_let_bound_fn_reaches_the_fn() {
        let src = r#"
            fn target() -> Int { return 1; }
            fn g() -> Int {
                let f = target;
                let p = std::process::pid;
                let n = len;
                p();
                n("x");
                return f();
            }
            fn main() { }
        "#;
        assert_eq!(
            local_calls(src, "g"),
            vec![through("?std::process::pid", "p"), through("?len", "n"), through("target", "f")],
        );
    }

    /// The bindings are scoped: a local bound in a block reaches its fn
    /// inside it, through a local bound to it, and is gone after it.
    #[test]
    fn a_let_bound_fn_is_followed_through_nested_scopes() {
        let src = r#"
            fn target() -> Int { return 1; }
            fn g(c: Bool) -> Int {
                let f = target;
                if c {
                    let h = f;
                    h();
                    while c {
                        let k = target;
                        k();
                    }
                    k();
                }
                return f();
            }
            fn main() { }
        "#;
        assert_eq!(
            local_calls(src, "g"),
            vec![through("target", "h"), through("target", "k"), written("?k"), through("target", "f")],
        );
    }

    /// An inner binding shadows an outer one for its scope only; a
    /// parameter shadows a fn of the same name; a bare callee names a fn
    /// before a local, as codegen lowers it.
    #[test]
    fn a_shadowing_binding_is_the_one_called() {
        let src = r#"
            fn target() -> Int { return 1; }
            fn make() -> Int { return 2; }
            fn other() -> Int { return 3; }
            fn g(c: Bool, other: Int) -> Int {
                let f = target;
                if c {
                    let f = make();
                    f();
                }
                let o = other;
                o();
                let make = target;
                make();
                return f();
            }
            fn main() { }
        "#;
        assert_eq!(
            local_calls(src, "g"),
            vec![written("make"), indirect("?f"), indirect("?o"), written("make"), through("target", "f")],
        );
    }

    /// A reassigned local holds whichever value the run took last, and a
    /// tuple binding is not followed: either call is `Unresolved` with
    /// the local's name, and indirect.
    #[test]
    fn a_reassigned_or_tuple_bound_local_is_not_followed() {
        let src = r#"
            fn target() -> Int { return 1; }
            fn other() -> Int { return 2; }
            fn g() -> Int {
                let f = target;
                f = other;
                f();
                let (t, u) = (target, other);
                t();
                return u();
            }
            fn main() { }
        "#;
        assert_eq!(local_calls(src, "g"), vec![indirect("?f"), indirect("?t"), indirect("?u")]);
    }

    /// A call through a function-typed parameter stays indirect (#353);
    /// a local bound to one is not followed, and its call is indirect
    /// too.
    #[test]
    fn a_function_typed_parameter_stays_indirect() {
        let src = r#"
            fn g(cb: fn() -> Int) -> Int {
                let h = cb;
                h();
                return cb();
            }
            fn main() { }
        "#;
        assert_eq!(local_calls(src, "g"), vec![indirect("?h"), indirect("?cb")]);
        let s = summarize(src);
        let param: Vec<bool> = fns(&s, None, "g").calls.iter().map(|c| c.through_param).collect();
        assert_eq!(param, [false, true], "only the parameter's own call is through the parameter");
    }

    /// A call through a local the bindings do not follow is marked for
    /// the capability admission alone; a call by name, through a followed
    /// local, or of a name no binding holds is not.
    #[test]
    fn a_call_through_an_unfollowed_local_is_marked() {
        let src = r#"
            fn target() -> Int { return 1; }
            fn g(c: Bool, cb: fn() -> Int) -> Int {
                let f = if c { target } else { target };
                f();
                let h = target;
                h();
                let k = cb;
                k();
                cb();
                len("x");
                unknown();
                return target();
            }
            fn main() { }
        "#;
        let s = summarize(src);
        let marked: Vec<(String, bool)> = fns(&s, None, "g")
            .calls
            .iter()
            .map(|c| match &c.spelling {
                CallSpelling::Ident(n) => (n.clone(), c.unresolved_local),
                other => panic!("a bare callee: {other:?}"),
            })
            .collect();
        assert_eq!(
            marked,
            [("f", true), ("h", false), ("k", true), ("cb", true), ("len", false), ("unknown", false), ("target", false)]
                .map(|(n, m)| (n.to_string(), m))
        );
    }

    /// The walk visits a loop once, so a binding the loop reassigns is
    /// not followed anywhere in it — its condition included — nor after
    /// it (P3 2 of 3, a review fix): a call ahead of the assignment runs,
    /// on a later iteration, the value the assignment stored. A binding
    /// the loop only reads is followed, and an assignment to a `let` the
    /// loop shadows leaves the outer binding alone.
    #[test]
    fn a_binding_a_loop_reassigns_is_not_followed() {
        let src = r#"
            fn width() -> Int { return 1; }
            fn other() -> Int { return 2; }
            fn g(c: Bool) -> Int {
                let mut f = width;
                let mut i = 0;
                while i < 2 {
                    f();
                    f = other;
                    i = i + 1;
                }
                f();
                let mut q = width;
                while q() < 2 {
                    q = other;
                }
                let mut h = width;
                for x in [1, 2] {
                    h();
                    if c {
                        h = other;
                    }
                }
                let k = width;
                let s = width;
                while c {
                    k();
                    let mut s = other;
                    s();
                    s = width;
                }
                s();
                return k();
            }
            fn main() { }
        "#;
        assert_eq!(
            local_calls(src, "g"),
            vec![
                indirect("?f"),
                indirect("?f"),
                indirect("?q"),
                indirect("?h"),
                through("width", "k"),
                through("other", "s"),
                through("width", "s"),
                through("width", "k"),
            ],
        );
        // Each unfollowed call is two alternatives (`width`, `other`), each
        // still marked as written through an unfollowed local.
        let s = summarize(src);
        let marked: Vec<bool> = fns(&s, None, "g").calls.iter().map(|c| c.unresolved_local).collect();
        assert_eq!(marked, [true, true, true, true, true, true, true, true, false, false, false, false]);
    }
}
