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

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

/// Identifies a fn for summary lookup. Free fns have `locus: None`; locus
/// methods + lifecycle hooks carry the enclosing locus's name. Lifecycle
/// hooks are keyed by their kind (`"run"`, `"birth"`, …) — these never
/// collide with method names since the kinds are reserved keywords.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FnKey {
    pub locus: Option<String>,
    pub fn_name: String,
}

impl FnKey {
    pub fn free_fn(name: impl Into<String>) -> Self {
        Self { locus: None, fn_name: name.into() }
    }
    pub fn method(locus: impl Into<String>, name: impl Into<String>) -> Self {
        Self { locus: Some(locus.into()), fn_name: name.into() }
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
}

impl AllocKind {
    fn label(&self) -> String {
        match self {
            AllocKind::StructLit(n) => format!("struct {}", n),
            AllocKind::ArrayLit => "array-literal".to_string(),
            AllocKind::ArrayRepeat => "array-repeat".to_string(),
            AllocKind::BytesLit => "bytes-literal".to_string(),
            AllocKind::CollectionInsert(form) => format!("{}-insert", form),
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
fn is_insert_method(method: &str) -> bool {
    matches!(method, "push" | "set" | "insert" | "add")
}

/// The unqualified name of a `Named` type expression (`SegVec` from
/// `path::to::SegVec`), or `None` for primitives / arrays / etc.
fn type_expr_name(te: &TypeExpr) -> Option<String> {
    match te {
        TypeExpr::Named { path, .. } => path.segments.last().map(|s| s.name.clone()),
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
    fn persists_across_calls(&self) -> bool {
        matches!(self, Escape::StoredToSelf | Escape::Returned)
    }
}

/// When an allocation's memory is actually reclaimed — the *empirically
/// validated* reclamation model (step 2), which differs from
/// `spec/memory.md`. Measured: a struct allocated inside a non-inlinable
/// free fn called 3M× in a loop accumulates to ~99 MB (vs ~5 MB for an
/// alloc-free loop) — i.e. **free-fn returns do NOT reclaim per call**
/// (the spec's §"Free fn functions" region-free-at-return is not what
/// runs). So a value allocation lives until its enclosing **locus**
/// dissolves; only bus sends get a per-dispatch arena.
///
/// The conservative consequence (and the whole point — "no false
/// bounded"): any value allocation inside a loop accumulates per
/// iteration, bounded only by the loop's trip count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReclaimScope {
    /// Freed wholesale only when the enclosing locus dissolves — so it
    /// accumulates across every loop iteration in between. Covers
    /// `Local`, `Returned`, and `StoredToSelf` value allocations.
    EnclosingLocus,
    /// Routed to the bus payload arena, reclaimed after the message is
    /// dispatched — a genuine per-iteration boundary. (Modeled from the
    /// spec + bus codegen; RSS-validation of this path is pending, noted
    /// in the step-2 validation test.)
    AfterBusDispatch,
}

impl ReclaimScope {
    fn of(escape: Escape) -> Self {
        match escape {
            Escape::Sent => ReclaimScope::AfterBusDispatch,
            _ => ReclaimScope::EnclosingLocus,
        }
    }
    /// Does this allocation persist across loop iterations (vs being
    /// reclaimed each iteration)?
    fn accumulates_in_loop(&self) -> bool {
        matches!(self, ReclaimScope::EnclosingLocus)
    }
    fn label(&self) -> &'static str {
        match self {
            ReclaimScope::EnclosingLocus => "reclaim@locus-dissolve",
            ReclaimScope::AfterBusDispatch => "reclaim@bus-dispatch",
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
    pub span: Span,
}

impl AllocSite {
    pub fn verdict(&self) -> SiteVerdict {
        if self.loop_depth == 0 {
            SiteVerdict::OncePerInvocation
        } else if !self.reclaim.accumulates_in_loop() {
            SiteVerdict::PerIterationReclaim
        } else if self.in_unbounded_loop {
            SiteVerdict::AccumulatesUnbounded
        } else {
            SiteVerdict::AccumulatesBoundedLoop
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
    pub span: Span,
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
    pub fns: BTreeMap<FnKey, FnSummary>,
    /// GH #18 item 1: loci carrying `@bounded` — their leak sites are
    /// reported even without the `--warn-unbounded-alloc` survey flag
    /// (the in-source opt-in).
    pub bounded_loci: BTreeSet<String>,
    /// Fns carrying `@unbounded` — an acknowledged-intentional
    /// accumulation. Their leak sites are dropped entirely (the
    /// greppable carve-out), even under the survey flag.
    pub unbounded_fns: BTreeSet<FnKey>,
    /// Per-locus storage shape (Phase D / D1) — capacity slots, `@form`,
    /// projection cap. Keyed by locus name.
    pub locus_shapes: BTreeMap<String, LocusShape>,
}

impl AllocSummary {
    /// Is this site's owning fn inside a `@bounded` locus? Drives
    /// "report by default" without the survey flag.
    pub fn owner_is_bounded_scope(&self, owner: &FnKey) -> bool {
        owner
            .locus
            .as_ref()
            .is_some_and(|l| self.bounded_loci.contains(l))
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
    /// caller is, or the call edge is inside an unbounded loop.
    pub fn unbounded_invoked(&self) -> BTreeSet<FnKey> {
        let mut set: BTreeSet<FnKey> = self
            .fns
            .values()
            .filter(|f| f.entry == Some(EntryKind::BusHandler))
            .map(|f| f.key.clone())
            .collect();
        loop {
            let mut changed = false;
            for f in self.fns.values() {
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
    /// method scratch, so they STOP the propagation — a value
    /// consumed there dies at method exit.
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
    /// per-activation loops to catch it.
    fn final_verdict(
        &self,
        owner: &FnKey,
        site: &AllocSite,
        unbounded: &BTreeSet<FnKey>,
        scratchless: &BTreeSet<FnKey>,
        callers: &BTreeMap<FnKey, BTreeSet<FnKey>>,
    ) -> SiteVerdict {
        let intra = site.verdict();
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
            _ if unbounded.contains(owner)
                && site.reclaim.accumulates_in_loop()
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
        let has_long_lived_entry = self.fns.values().any(|f| {
            matches!(
                f.entry,
                Some(EntryKind::Run) | Some(EntryKind::BusHandler)
            )
        });
        let has_main = self
            .fns
            .values()
            .any(|f| matches!(f.entry, Some(EntryKind::Main)));
        if has_main && !has_long_lived_entry {
            return Vec::new();
        }
        let unbounded = self.unbounded_invoked();
        let scratchless = self.scratchless_longlived();
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
                if self.final_verdict(&f.key, s, &unbounded, &scratchless, &callers) == SiteVerdict::AccumulatesUnbounded {
                    let reason = if matches!(s.verdict(), SiteVerdict::AccumulatesUnbounded) {
                        LeakReason::InUnboundedLoop
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

    /// Human-readable dump for `--dump-alloc-summary`.
    pub fn render(&self) -> String {
        let unbounded = self.unbounded_invoked();
        let scratchless = self.scratchless_longlived();
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
        let entries: Vec<&FnSummary> = self.fns.values().filter(|f| f.entry.is_some()).collect();
        out.push_str(&format!(
            "# {} fns, {} entry points, {} invoked-unboundedly\n\n",
            self.fns.len(),
            entries.len(),
            unbounded.len()
        ));
        // D1: per-locus storage shape — the capacity slots, `@form`, and
        // projection cap the bound solver (D2) will read.
        for shape in self.locus_shapes.values() {
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
        if !self.locus_shapes.values().all(|s| {
            s.capacity_slots.is_empty() && s.form.is_none() && s.recognition_cap.is_none()
        }) {
            out.push('\n');
        }
        for f in self.fns.values() {
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
                    &f.key, s, &unbounded, &scratchless, &callers,
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
pub fn summarize_programs(programs: &[&Program]) -> AllocSummary {
    // Phase 1 — collect every body with its key + entry classification.
    // For loci we first gather the set of bus-handler method names so a
    // method referenced by `subscribe ... -> handler` is tagged BusHandler.
    // The trailing `Vec<(String, String)>` seeds each body's var→type map
    // from its params (D2).
    type BodyEntry = (FnKey, Block, Option<EntryKind>, Option<String>, Vec<(String, String)>);
    let mut bodies: Vec<BodyEntry> = Vec::new();
    let mut known: BTreeSet<FnKey> = BTreeSet::new();
    // GH #18 item 1 — the `@bounded` / `@unbounded` opt-in/carve-out sets.
    let mut bounded_loci: BTreeSet<String> = BTreeSet::new();
    let mut unbounded_fns: BTreeSet<FnKey> = BTreeSet::new();
    // Phase D / D1 — the per-locus storage shape.
    let mut locus_shapes: BTreeMap<String, LocusShape> = BTreeMap::new();
    // Phase D / D2 — per-locus param field → declared type name.
    let mut locus_field_types: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    // 2026-07-01 — per-locus scalar-[T; N] param fields (inline layout).
    let mut locus_inline_arrays: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for program in programs {
        for item in &program.items {
            match item {
                TopDecl::Fn(decl) => {
                    let key = FnKey::free_fn(decl.name.name.clone());
                    let entry = if decl.name.name == "main" { Some(EntryKind::Main) } else { None };
                    if decl.unbounded {
                        unbounded_fns.insert(key.clone());
                    }
                    known.insert(key.clone());
                    bodies.push((key, decl.body.clone(), entry, None, param_var_types(&decl.params)));
                }
                TopDecl::Locus(l) => {
                    let locus = l.name.name.clone();
                    if l.bounded {
                        bounded_loci.insert(locus.clone());
                    }
                    locus_shapes.insert(locus.clone(), locus_shape_of(l));
                    locus_field_types.insert(locus.clone(), locus_param_field_types(l));
                    locus_inline_arrays
                        .insert(locus.clone(), locus_inline_array_fields(l));
                    let handlers = bus_handler_names(l);
                    for member in &l.members {
                        match member {
                            LocusMember::Fn(decl) => {
                                let key = FnKey::method(locus.clone(), decl.name.name.clone());
                                let entry = if handlers.contains(&decl.name.name) {
                                    Some(EntryKind::BusHandler)
                                } else {
                                    None
                                };
                                if decl.unbounded {
                                    unbounded_fns.insert(key.clone());
                                }
                                known.insert(key.clone());
                                bodies.push((
                                    key,
                                    decl.body.clone(),
                                    entry,
                                    Some(locus.clone()),
                                    param_var_types(&decl.params),
                                ));
                            }
                            LocusMember::Lifecycle(lc) => {
                                let (name, entry) = lifecycle_key(lc.kind);
                                let key = FnKey::method(locus.clone(), name);
                                if lc.unbounded {
                                    unbounded_fns.insert(key.clone());
                                }
                                known.insert(key.clone());
                                bodies.push((
                                    key,
                                    lc.body.clone(),
                                    Some(entry),
                                    Some(locus.clone()),
                                    param_var_types(&lc.params),
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

    // D2: type name → `@form(...)` name, for receiver-form lookup.
    let form_of: BTreeMap<String, String> = locus_shapes
        .iter()
        .filter_map(|(name, shape)| shape.form.as_ref().map(|f| (name.clone(), f.name.clone())))
        .collect();
    let empty_fields: BTreeMap<String, String> = BTreeMap::new();
    let empty_inline_arrays: BTreeSet<String> = BTreeSet::new();

    // Phase 2 — walk each body.
    let mut summary = AllocSummary::default();
    for (key, body, entry, enclosing_locus, param_types) in &bodies {
        let escaping = collect_escaping_names(body);
        let field_types = enclosing_locus
            .as_ref()
            .and_then(|l| locus_field_types.get(l))
            .unwrap_or(&empty_fields);
        let inline_array_fields = enclosing_locus
            .as_ref()
            .and_then(|l| locus_inline_arrays.get(l))
            .unwrap_or(&empty_inline_arrays);
        let mut w = Walker {
            sites: Vec::new(),
            calls: Vec::new(),
            loops: Vec::new(),
            escaping: &escaping,
            enclosing_locus: enclosing_locus.clone(),
            known: &known,
            loop_stack: Vec::new(),
            infinite_stack: Vec::new(),
            fn_body: body,
            store_target: None,
            var_types: param_types.iter().cloned().collect(),
            field_types,
            inline_array_fields,
            form_of: &form_of,
        };
        w.walk_block(body, 0, Escape::Local);
        summary.fns.insert(
            key.clone(),
            FnSummary {
                key: key.clone(),
                entry: *entry,
                sites: w.sites,
                calls: w.calls,
                loops: w.loops,
            },
        );
    }
    summary.bounded_loci = bounded_loci;
    summary.unbounded_fns = unbounded_fns;
    summary.locus_shapes = locus_shapes;
    summary
}

/// D2: a fn's params as (name, declared-type-name) pairs — the seed for
/// the var→type map (only `Named` types; primitives/arrays are skipped).
fn param_var_types(params: &[Param]) -> Vec<(String, String)> {
    params
        .iter()
        .filter_map(|p| type_expr_name(&p.ty).map(|t| (p.name.name.clone(), t)))
        .collect()
}

/// D2: a locus's `params { … }` fields as field → declared-type-name, so a
/// `self.<field>.push(x)` can resolve `<field>`'s form.
fn locus_param_field_types(l: &LocusDecl) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for member in &l.members {
        if let LocusMember::Params(pb) = member {
            for pd in &pb.params {
                if let Some(tn) = pd.ty.as_ref().and_then(type_expr_name) {
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

/// Bound-solver diagnostics: a warning per unbounded-accumulation site.
///
/// `include_all` selects the scope (GH #18 item 1, Phase B):
/// - `false` (default `hale check`): only sites inside a `@bounded` locus —
///   the in-source opt-in. A program with no `@bounded` locus is silent.
/// - `true` (`--warn-unbounded-alloc`): every site, the whole-program
///   survey.
///
/// Either way, `@unbounded`-fn sites are already dropped at `leak_sites()`.
pub fn unbounded_alloc_diags(programs: &[&Program], include_all: bool) -> Vec<Diag> {
    let summary = summarize_programs(programs);
    summary
        .leak_sites()
        .iter()
        .filter(|ls| include_all || summary.owner_is_bounded_scope(&ls.owner))
        .map(|ls| {
            let where_ = match ls.reason {
                LeakReason::InUnboundedLoop => "inside an unbounded loop",
                LeakReason::InvokedUnboundedly => {
                    "in a fn invoked unboundedly (a per-message bus handler, \
                     or reached through a call inside an unbounded loop)"
                }
            };
            Diag::warn(
                ls.span,
                format!(
                    "unbounded allocation: this {} {} accumulates in `{}`'s region \
                     until the locus dissolves — it is never reclaimed per iteration, \
                     so it grows without bound. Bound the loop; store it in a \
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

struct Walker<'a> {
    sites: Vec<AllocSite>,
    calls: Vec<CallEdge>,
    loops: Vec<LoopInfo>,
    escaping: &'a BTreeMap<String, Escape>,
    enclosing_locus: Option<String>,
    known: &'a BTreeSet<FnKey>,
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
    /// Phase D / D1: the `self.<field>` currently being assigned, set around
    /// the RHS walk of a `self.<field> = …` statement so an escaping
    /// allocation in that RHS records which field it lands in.
    store_target: Option<String>,
    /// Phase D / D2 (lite): local var / param name → declared type *name*,
    /// seeded from this fn's params and grown by typed `let`s. Lets a
    /// `v.push(x)` resolve `v`'s type to ask whether it is a growing form.
    var_types: BTreeMap<String, String>,
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
}

impl<'a> Walker<'a> {
    fn push_site(&mut self, kind: AllocKind, escape: Escape, depth: u32, span: Span) {
        // Only a StoredToSelf escape carries a target field — that's the
        // `self.<field> = <alloc>` whole-value replace the solver bounds.
        let target_field = if escape == Escape::StoredToSelf {
            self.store_target.clone()
        } else {
            None
        };
        self.sites.push(AllocSite {
            kind,
            escape,
            loop_depth: depth,
            in_unbounded_loop: self.loop_stack.iter().any(|bounded| !bounded),
            in_infinite_loop: self.infinite_stack.iter().any(|i| *i),
            reclaim: ReclaimScope::of(escape),
            target_field,
            span,
        });
    }

    /// D2: if `recv` is a value whose *declared* type is a growing
    /// `@form(vec | hashmap)` locus, the form name. Resolves a bare var via
    /// `var_types` and a `self.<field>` via `field_types`.
    fn growing_form_of_receiver(&self, recv: &Expr) -> Option<String> {
        let ty_name = match recv {
            Expr::Ident(v) => self.var_types.get(&v.name)?,
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. }
                if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
            {
                self.field_types.get(&name.name)?
            }
            _ => return None,
        };
        let form = self.form_of.get(ty_name)?;
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
            span,
        });
    }
}

impl<'a> Walker<'a> {
    fn walk_block(&mut self, b: &Block, depth: u32, tail_escape: Escape) {
        for s in &b.stmts {
            self.walk_stmt(s, depth);
        }
        if let Some(t) = &b.tail {
            self.walk_expr(t, depth, tail_escape);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt, depth: u32) {
        match stmt {
            Stmt::Let { name, ty, value, .. } => {
                // D2: a typed `let v: T = …` extends the var→type map so a
                // later `v.push(x)` can resolve `v`'s form.
                if let Some(tn) = ty.as_ref().and_then(type_expr_name) {
                    self.var_types.insert(name.name.clone(), tn);
                }
                let esc = self.escaping.get(&name.name).copied().unwrap_or(Escape::Local);
                self.walk_expr(value, depth, esc);
            }
            Stmt::LetTuple { value, .. } => self.walk_expr(value, depth, Escape::Local),
            Stmt::Assign { target, value, .. } => {
                let mut esc = if target.head.name == "self" {
                    Escape::StoredToSelf
                } else {
                    self.escaping.get(&target.head.name).copied().unwrap_or(Escape::Local)
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
            }
            Stmt::Return(Some(e), _) => self.walk_expr(e, depth, Escape::Returned),
            Stmt::Return(None, _) => {}
            Stmt::Fail { value, .. } => self.walk_expr(value, depth, Escape::Returned),
            Stmt::Send { subject, value, .. } => {
                self.walk_expr(subject, depth, Escape::Local);
                self.walk_expr(value, depth, Escape::Sent);
            }
            Stmt::For { iter, body, span, .. } => {
                self.walk_expr(iter, depth, Escape::Local);
                let kind = for_loop_kind(iter);
                let bounded = matches!(kind, LoopKind::ForRange { bounded: Some(_) });
                self.loops.push(LoopInfo { kind, depth, span: *span });
                self.loop_stack.push(bounded);
                self.infinite_stack.push(false);
                self.walk_block(body, depth + 1, Escape::Local);
                self.loop_stack.pop();
                self.infinite_stack.pop();
            }
            Stmt::While { cond, body, span } => {
                self.walk_expr(cond, depth, Escape::Local);
                // Loop-ranking: a `while v < N` counter whose `v` is
                // const-initialized and only ever incremented by positive
                // consts is const-bounded; any other `while` is unbounded
                // (its trip count is runtime, like a runtime `for`-iter).
                let bounded = while_counter_bounded(cond, self.fn_body);
                let kind = if bounded { LoopKind::WhileCounter } else { while_loop_kind(cond) };
                let infinite = matches!(
                    cond,
                    Expr::Literal(Literal::Bool(true), _)
                );
                self.loops.push(LoopInfo { kind, depth, span: *span });
                self.loop_stack.push(bounded);
                self.infinite_stack.push(infinite);
                self.walk_block(body, depth + 1, Escape::Local);
                self.loop_stack.pop();
                self.infinite_stack.pop();
            }
            Stmt::If(if_stmt) => self.walk_if(if_stmt, depth, Escape::Local),
            Stmt::Match(m) => self.walk_match(m, depth, Escape::Local),
            Stmt::Block(b) => self.walk_block(b, depth, Escape::Local),
            Stmt::ShmWrite { max, body, .. } => {
                self.walk_expr(max, depth, Escape::Local);
                // The body writes into the ring view, not the arena; treat
                // its allocations as local for now.
                self.walk_block(body, depth, Escape::Local);
            }
            Stmt::Expr(e) => self.walk_expr(e, depth, Escape::Local),
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
            if let Some(g) = &arm.guard {
                self.walk_expr(g, depth, Escape::Local);
            }
            match &arm.body {
                MatchArmBody::Block(b) => self.walk_block(b, depth, escape),
                MatchArmBody::Expr(e) => self.walk_expr(e, depth, escape),
            }
        }
    }

    fn walk_expr(&mut self, expr: &Expr, depth: u32, escape: Escape) {
        match expr {
            Expr::Struct { path, inits, span } => {
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
                if !inplace_no_heap {
                    self.push_site(AllocKind::StructLit(name), escape, depth, *span);
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
            // (type-free) pass doesn't have. Flagging every `i + 1` as a
            // leak precursor is the exact cry-wolf failure the scope warns
            // against, so String-concat detection is deferred to a
            // type-aware stage. Here `+` is just recursed, not a site.
            Expr::Binary { left, right, .. } => {
                self.walk_expr(left, depth, Escape::Local);
                self.walk_expr(right, depth, Escape::Local);
            }
            Expr::Unary { operand, .. } => self.walk_expr(operand, depth, Escape::Local),
            Expr::Call { callee, args, span } => {
                self.record_call(callee, *span, depth, escape);
                // D2: a `recv.<insert>(x)` where `recv`'s declared type is a
                // growing form is itself an accumulating allocation.
                if let Expr::Field { receiver, name, .. }
                | Expr::Path2 { receiver, name, .. } = callee.as_ref()
                {
                    if is_insert_method(&name.name) {
                        if let Some(form) = self.growing_form_of_receiver(receiver) {
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
                    _ => {}
                }
                for a in args {
                    self.walk_expr(a, depth, Escape::Local);
                }
            }
            Expr::Field { receiver, .. } | Expr::Index { receiver, .. } => {
                self.walk_expr(receiver, depth, Escape::Local);
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
                    OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
                }
            }
        }
    }

    fn record_call(&mut self, callee: &Expr, span: Span, depth: u32, escape: Escape) {
        let resolved = match callee {
            Expr::Ident(id) => {
                let key = FnKey::free_fn(id.name.clone());
                if self.known.contains(&key) {
                    Callee::Resolved(key)
                } else {
                    Callee::Unresolved(id.name.clone())
                }
            }
            Expr::Path(qp) => {
                let path = qp.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                Callee::Unresolved(path)
            }
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                if let (Expr::KwSelf(_), Some(locus)) = (receiver.as_ref(), &self.enclosing_locus) {
                    let key = FnKey::method(locus.clone(), name.name.clone());
                    if self.known.contains(&key) {
                        Callee::Resolved(key)
                    } else {
                        Callee::Unresolved(name.name.clone())
                    }
                } else {
                    Callee::Unresolved(name.name.clone())
                }
            }
            _ => Callee::Unresolved("<expr>".to_string()),
        };
        self.calls.push(CallEdge {
            callee: resolved,
            loop_depth: depth,
            in_unbounded_loop: self.loop_stack.iter().any(|bounded| !bounded),
            escape,
            receiver_slot: self_slot_receiver(callee),
            span,
        });
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
fn while_counter_bounded(cond: &Expr, fn_body: &Block) -> bool {
    let var = match cond {
        Expr::Binary { op: BinOp::Lt | BinOp::LtEq, left, right, .. } => {
            match (left.as_ref(), right.as_ref()) {
                (Expr::Ident(v), Expr::Literal(Literal::Int(_), _)) => v.name.as_str(),
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

/// Pre-pass: local names whose value flows to an escape position directly
/// as `Ident(name)` — covering the common `let x = <alloc>; … return x;`
/// indirection. Walks the whole body (nested blocks/ifs/loops/matches).
fn collect_escaping_names(body: &Block) -> BTreeMap<String, Escape> {
    let mut out = BTreeMap::new();
    collect_escaping_in_block(body, &mut out);
    out
}

fn collect_escaping_in_block(b: &Block, out: &mut BTreeMap<String, Escape>) {
    for s in &b.stmts {
        collect_escaping_in_stmt(s, out);
    }
    if let Some(t) = &b.tail {
        note_escape(t, Escape::Returned, out);
    }
}

fn collect_escaping_in_stmt(s: &Stmt, out: &mut BTreeMap<String, Escape>) {
    match s {
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

fn collect_escaping_in_if(if_stmt: &IfStmt, out: &mut BTreeMap<String, Escape>) {
    collect_escaping_in_block(&if_stmt.then_block, out);
    if let Some(else_br) = &if_stmt.else_block {
        match else_br.as_ref() {
            ElseBranch::Else(b) => collect_escaping_in_block(b, out),
            ElseBranch::ElseIf(inner) => collect_escaping_in_if(inner, out),
        }
    }
}

fn note_escape(e: &Expr, esc: Escape, out: &mut BTreeMap<String, Escape>) {
    if let Expr::Ident(id) = e {
        out.entry(id.name.clone()).or_insert(esc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    fn summarize(src: &str) -> AllocSummary {
        let program = parse_source(src).expect("parse");
        summarize_programs(&[&program])
    }

    fn fns(s: &AllocSummary, key: &FnKey) -> FnSummary {
        s.fns.get(key).cloned().unwrap_or_else(|| {
            panic!("no summary for {:?}; keys = {:?}", key, s.fns.keys().collect::<Vec<_>>())
        })
    }

    #[test]
    fn struct_returned_is_escaping() {
        let src = r#"
            type P { x: Int; }
            fn make(n: Int) -> P { return P { x: n }; }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, &FnKey::free_fn("make"));
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
        let f = fns(&s, &FnKey::free_fn("make"));
        assert_eq!(f.sites[0].escape, Escape::Returned, "let-bound + returned should escape");
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
        let f = fns(&s, &FnKey::free_fn("run_it"));
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
            let f = fns(&s, &FnKey::free_fn("run_it"));
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

        // (b) the same handler storing the struct INTO self → escapes the
        // scratch → accumulates across deliveries → flagged.
        let stored = r#"
            type T { n: Int; }
            type Tmp { a: Int; b: Int; }
            locus L {
                params { last: Tmp; }
                bus { subscribe "in" as on_in of type T; }
                fn on_in(m: T) { self.last = Tmp { a: m.n, b: m.n }; }
            }
            fn main() { }
        "#;
        assert_eq!(
            summarize(stored).leak_sites().len(),
            1,
            "a self-stored handler alloc persists across deliveries — must flag"
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
        let f = fns(&s, &FnKey::free_fn("loopy"));
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
        let f = fns(&s, &FnKey::method("C", "on_tick"));
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
        let f = fns(&s, &FnKey::method("C", "run"));
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
        let f = fns(&s, &FnKey::method("C", "use_it"));
        assert!(f.calls.iter().any(|c| c.callee == Callee::Resolved(FnKey::method("C", "helper"))));
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
        let f = fns(&s, &FnKey::method("C", "run"));
        let st = f.sites.iter().find(|s| matches!(s.kind, AllocKind::StructLit(_))).expect("struct");
        assert_eq!(st.reclaim, ReclaimScope::EnclosingLocus);
        assert!(st.in_unbounded_loop);
        assert_eq!(st.verdict(), SiteVerdict::AccumulatesUnbounded);
    }

    #[test]
    fn struct_in_bounded_for_is_bounded() {
        let src = r#"
            type Q { a: Int; }
            fn work() { for i in 0..100 { let q = Q { a: i }; } }
            fn main() { }
        "#;
        let s = summarize(src);
        let f = fns(&s, &FnKey::free_fn("work"));
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
        let f = fns(&s, &FnKey::free_fn("make"));
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
        let f = fns(&s, &FnKey::method("C", "run"));
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
        let f = fns(&s, &FnKey::method("C", "set"));
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
        assert!(s.unbounded_invoked().contains(&FnKey::free_fn("make")));
        let leaks = s.leak_sites();
        assert!(
            leaks.iter().any(|l| l.owner == FnKey::free_fn("make")
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
        let program = hale_syntax::parse_source(src).expect("parse");
        // Survey mode (`--warn-unbounded-alloc`) reports the leak even
        // though locus `C` carries no `@bounded` opt-in.
        let diags = unbounded_alloc_diags(&[&program], true);
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
        let program = hale_syntax::parse_source(src).expect("parse");
        let scoped = unbounded_alloc_diags(&[&program], false);
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
        let program = hale_syntax::parse_source(src).expect("parse");
        assert!(
            unbounded_alloc_diags(&[&program], true).is_empty(),
            "@unbounded suppresses the site under the survey flag"
        );
        assert!(
            unbounded_alloc_diags(&[&program], false).is_empty(),
            "@unbounded suppresses the site in @bounded scope too"
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
        let f = fns(&s, &FnKey::method("C", "run"));
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
        let f = fns(&s, &FnKey::method("C", "run"));
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
        let f = fns(&s, &FnKey::method("C", "run"));
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
        let f = fns(&s, &FnKey::method("W", "on_ev"));
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
        let f = fns(&s, &FnKey::free_fn("double_push"));
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
        let f = fns(&s, &FnKey::method("W", "on_ev"));
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
        let f = fns(&s, &FnKey::method("W", "run"));
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
        let f = fns(&s, &FnKey::method("W", "run"));
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
        let f = fns(&s, &FnKey::method("W", "run"));
        let site = insert_site(&f).expect("typed-let hashmap insert detected");
        assert_eq!(site.kind, AllocKind::CollectionInsert("hashmap".into()));
        assert_eq!(site.verdict(), SiteVerdict::AccumulatesUnbounded);
    }
}
