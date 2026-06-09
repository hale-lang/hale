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
//!   (each lowers to an arena alloc) and `+` sites (a *possible* String
//!   concat; type info in a later stage prunes the arithmetic ones), each
//!   tagged **local** (freed at scope exit) vs **escaping** (flows to a
//!   `return`, a `self.field` store, or a bus `<-`), with its enclosing
//!   **loop depth**;
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
use hale_syntax::Span;

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
    fn display(&self) -> String {
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
    PossibleConcat,
}

impl AllocKind {
    fn label(&self) -> String {
        match self {
            AllocKind::StructLit(n) => format!("struct {}", n),
            AllocKind::ArrayLit => "array-literal".to_string(),
            AllocKind::ArrayRepeat => "array-repeat".to_string(),
            AllocKind::BytesLit => "bytes-literal".to_string(),
            AllocKind::PossibleConcat => "possible-concat".to_string(),
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
    fn escapes(&self) -> bool {
        !matches!(self, Escape::Local)
    }
}

/// One allocation site in a body.
#[derive(Debug, Clone)]
pub struct AllocSite {
    pub kind: AllocKind,
    pub escape: Escape,
    pub loop_depth: u32,
    pub span: Span,
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

impl AllocSummary {
    /// Human-readable dump for `--dump-alloc-summary`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# allocation summary (GH #18 item 1, step 1 — no bound-proving)\n");
        let entries: Vec<&FnSummary> = self.fns.values().filter(|f| f.entry.is_some()).collect();
        out.push_str(&format!(
            "# {} fns, {} entry points\n\n",
            self.fns.len(),
            entries.len()
        ));
        for f in self.fns.values() {
            let entry = match f.entry {
                Some(e) => format!(
                    "   [entry: {} ({})]",
                    e.label(),
                    if e.one_shot() { "one-shot" } else { "per-message/unbounded" }
                ),
                None => String::new(),
            };
            out.push_str(&format!("fn {}{}\n", f.key.display(), entry));
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
                let flag = if s.escape.escapes() && s.loop_depth > 0 { "  <-- escaping-in-loop" } else { "" };
                out.push_str(&format!(
                    "    alloc {:<16} {:<20} loop_depth={} @{}..{}{}\n",
                    s.kind.label(),
                    s.escape.label(),
                    s.loop_depth,
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
                out.push_str(&format!("    call  {} loop_depth={}\n", tgt, c.loop_depth));
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
                self.loops.push(LoopInfo { kind: for_loop_kind(iter), depth, span: *span });
                self.walk_block(body, depth + 1, Escape::Local);
            }
            Stmt::While { cond, body, span } => {
                self.walk_expr(cond, depth, Escape::Local);
                self.loops.push(LoopInfo { kind: while_loop_kind(cond), depth, span: *span });
                self.walk_block(body, depth + 1, Escape::Local);
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
                let name = path.segments.last().map(|s| s.name.clone()).unwrap_or_default();
                self.sites.push(AllocSite {
                    kind: AllocKind::StructLit(name),
                    escape,
                    loop_depth: depth,
                    span: *span,
                });
                for si in inits {
                    self.walk_expr(&si.value, depth, escape);
                }
            }
            Expr::Array(xs, span) => {
                self.sites.push(AllocSite {
                    kind: AllocKind::ArrayLit,
                    escape,
                    loop_depth: depth,
                    span: *span,
                });
                for x in xs {
                    self.walk_expr(x, depth, escape);
                }
            }
            Expr::ArrayRepeat { val, span, .. } => {
                // `count` is a const `u64`, not an expr — nothing to walk.
                self.sites.push(AllocSite {
                    kind: AllocKind::ArrayRepeat,
                    escape,
                    loop_depth: depth,
                    span: *span,
                });
                self.walk_expr(val, depth, escape);
            }
            Expr::Literal(Literal::Bytes(_), span) => {
                self.sites.push(AllocSite {
                    kind: AllocKind::BytesLit,
                    escape,
                    loop_depth: depth,
                    span: *span,
                });
            }
            Expr::Literal(_, _) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
            Expr::Binary { op: BinOp::Add, left, right, span } => {
                // A `+` whose operands may be Strings → arena concat. With
                // no type info here this over-reports arithmetic `+`; a
                // later stage prunes via types.
                self.sites.push(AllocSite {
                    kind: AllocKind::PossibleConcat,
                    escape,
                    loop_depth: depth,
                    span: *span,
                });
                self.walk_expr(left, depth, escape);
                self.walk_expr(right, depth, escape);
            }
            Expr::Binary { left, right, .. } => {
                self.walk_expr(left, depth, Escape::Local);
                self.walk_expr(right, depth, Escape::Local);
            }
            Expr::Unary { operand, .. } => self.walk_expr(operand, depth, Escape::Local),
            Expr::Call { callee, args, span } => {
                self.record_call(callee, *span, depth);
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

    fn record_call(&mut self, callee: &Expr, span: Span, depth: u32) {
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
        self.calls.push(CallEdge { callee: resolved, loop_depth: depth, span });
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
}
