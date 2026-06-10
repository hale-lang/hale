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
}

impl AllocKind {
    fn label(&self) -> String {
        match self {
            AllocKind::StructLit(n) => format!("struct {}", n),
            AllocKind::ArrayLit => "array-literal".to_string(),
            AllocKind::ArrayRepeat => "array-repeat".to_string(),
            AllocKind::BytesLit => "bytes-literal".to_string(),
        }
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
    pub reclaim: ReclaimScope,
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
}

impl LoopKind {
    fn label(&self) -> String {
        match self {
            LoopKind::ForRange { bounded: Some(n) } => format!("for-range(bounded={})", n),
            LoopKind::ForRange { bounded: None } => "for-range(runtime)".to_string(),
            LoopKind::ForIter => "for-iter(runtime)".to_string(),
            LoopKind::While => "while".to_string(),
            LoopKind::WhileTrue => "while-true".to_string(),
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

/// The bundle-wide allocation summary + call graph.
#[derive(Debug, Clone, Default)]
pub struct AllocSummary {
    pub fns: BTreeMap<FnKey, FnSummary>,
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

    /// A site's final verdict, folding in call-graph multiplicity: an
    /// accumulating site in an unboundedly-invoked fn is unbounded even if
    /// it's only once-per-call in its own body. Bus-dispatch reclaim stays
    /// bounded regardless (the value is freed each dispatch).
    fn final_verdict(&self, owner: &FnKey, site: &AllocSite, unbounded: &BTreeSet<FnKey>) -> SiteVerdict {
        let intra = site.verdict();
        match intra {
            SiteVerdict::AccumulatesUnbounded | SiteVerdict::PerIterationReclaim => intra,
            _ if unbounded.contains(owner) && site.reclaim.accumulates_in_loop() => {
                SiteVerdict::AccumulatesUnbounded
            }
            _ => intra,
        }
    }

    /// Every site whose final verdict is unbounded accumulation — the
    /// step-3 result the diagnostic emits.
    pub fn leak_sites(&self) -> Vec<LeakSite> {
        let unbounded = self.unbounded_invoked();
        let mut out = Vec::new();
        for f in self.fns.values() {
            for s in &f.sites {
                if self.final_verdict(&f.key, s, &unbounded) == SiteVerdict::AccumulatesUnbounded {
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
        let mut out = String::new();
        out.push_str("# allocation summary (GH #18 item 1, steps 1-3)\n");
        let entries: Vec<&FnSummary> = self.fns.values().filter(|f| f.entry.is_some()).collect();
        out.push_str(&format!(
            "# {} fns, {} entry points, {} invoked-unboundedly\n\n",
            self.fns.len(),
            entries.len(),
            unbounded.len()
        ));
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
                let v = self.final_verdict(&f.key, s, &unbounded);
                let flag = if matches!(v, SiteVerdict::AccumulatesUnbounded) {
                    "  <-- LEAK"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "    alloc {:<16} {:<20} {:<22} {:<22} @{}..{}{}\n",
                    s.kind.label(),
                    s.escape.label(),
                    v.label(),
                    s.reclaim.label(),
                    s.span.start.0,
                    s.span.end.0,
                    flag
                ));
            }
            for c in &f.calls {
                let tgt = match &c.callee {
                    Callee::Resolved(k) => k.display(),
                    Callee::Unresolved(n) => format!("<unresolved: {}>", n),
                };
                out.push_str(&format!(
                    "    call  {} loop_depth={} result={}\n",
                    tgt,
                    c.loop_depth,
                    c.escape.label()
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
    let mut bodies: Vec<(FnKey, Block, Option<EntryKind>, Option<String>)> = Vec::new();
    let mut known: BTreeSet<FnKey> = BTreeSet::new();

    for program in programs {
        for item in &program.items {
            match item {
                TopDecl::Fn(decl) => {
                    let key = FnKey::free_fn(decl.name.name.clone());
                    let entry = if decl.name.name == "main" { Some(EntryKind::Main) } else { None };
                    known.insert(key.clone());
                    bodies.push((key, decl.body.clone(), entry, None));
                }
                TopDecl::Locus(l) => {
                    let locus = l.name.name.clone();
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
                                known.insert(key.clone());
                                bodies.push((key, decl.body.clone(), entry, Some(locus.clone())));
                            }
                            LocusMember::Lifecycle(lc) => {
                                let (name, entry) = lifecycle_key(lc.kind);
                                let key = FnKey::method(locus.clone(), name);
                                known.insert(key.clone());
                                bodies.push((key, lc.body.clone(), Some(entry), Some(locus.clone())));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Phase 2 — walk each body.
    let mut summary = AllocSummary::default();
    for (key, body, entry, enclosing_locus) in &bodies {
        let escaping = collect_escaping_names(body);
        let mut w = Walker {
            sites: Vec::new(),
            calls: Vec::new(),
            loops: Vec::new(),
            escaping: &escaping,
            enclosing_locus: enclosing_locus.clone(),
            known: &known,
            loop_stack: Vec::new(),
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
    summary
}

/// Bound-solver diagnostics: a warning per unbounded-accumulation site.
/// Opt-in (the corpus shows zero false positives, but allocation patterns
/// are common enough that this is gated until validated for default-on).
pub fn unbounded_alloc_diags(programs: &[&Program]) -> Vec<Diag> {
    let summary = summarize_programs(programs);
    summary
        .leak_sites()
        .iter()
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
                     so it grows without bound. Bound the loop, route the value over the \
                     bus (the payload arena reclaims per dispatch), or move the allocating \
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
}

impl<'a> Walker<'a> {
    fn push_site(&mut self, kind: AllocKind, escape: Escape, depth: u32, span: Span) {
        self.sites.push(AllocSite {
            kind,
            escape,
            loop_depth: depth,
            in_unbounded_loop: self.loop_stack.iter().any(|bounded| !bounded),
            reclaim: ReclaimScope::of(escape),
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
            Stmt::Let { name, value, .. } => {
                let esc = self.escaping.get(&name.name).copied().unwrap_or(Escape::Local);
                self.walk_expr(value, depth, esc);
            }
            Stmt::LetTuple { value, .. } => self.walk_expr(value, depth, Escape::Local),
            Stmt::Assign { target, value, .. } => {
                let esc = if target.head.name == "self" {
                    Escape::StoredToSelf
                } else {
                    self.escaping.get(&target.head.name).copied().unwrap_or(Escape::Local)
                };
                self.walk_expr(value, depth, esc);
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
                self.walk_block(body, depth + 1, Escape::Local);
                self.loop_stack.pop();
            }
            Stmt::While { cond, body, span } => {
                self.walk_expr(cond, depth, Escape::Local);
                // A `while` trip count is never a compile-time constant
                // here (the cond is runtime), so always unbounded.
                self.loops.push(LoopInfo { kind: while_loop_kind(cond), depth, span: *span });
                self.loop_stack.push(false);
                self.walk_block(body, depth + 1, Escape::Local);
                self.loop_stack.pop();
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
                self.push_site(AllocKind::StructLit(name), escape, depth, *span);
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
            span,
        });
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
        assert!(f.loops.iter().any(|l| matches!(l.kind, LoopKind::While)));
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
        let diags = unbounded_alloc_diags(&[&program]);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("unbounded allocation"));
    }
}
