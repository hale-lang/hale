//! GH #730, second half — a borrow outlives its holder.
//!
//! A handle stored into a locus-carrying param field — a `LocusRef`, an
//! `interface`, a `perspective(P)` — is a **borrow** (F.39: a name in a
//! field initialiser is `Owner::Borrowed`; #967 and #730's first half
//! made the assignment and the same-interface literal forms the same).
//! The holder never reclaims it, so the borrowed instance has to
//! outlive the holder. Ownership is structural and reclamation is a
//! tree cascade, so "outlives" is decidable from position, without
//! annotations:
//!
//! | handle comes from | holder owned by the frame | by `self` (a field, an accepted child) | by the caller (returned) |
//! |---|---|---|---|
//! | a field of `self` | sound | sound | refused |
//! | a `let` of this frame | sound while the binding is in scope | refused | refused |
//! | a bus handler's payload | sound | refused (GH #712) | refused |
//! | a parameter | sound | every caller is asked (below) | left alone |
//!
//! A parameter's provenance is the caller's. For a holder `self` owns,
//! every call site of the method is found and its argument classified
//! by the same table; the first caller that hands a `let` of its own
//! frame or a handler payload is the witness, and a parameter at the
//! caller recurses, to a bounded depth. Nothing else is refused: a
//! chain the walk cannot follow is left alone, never guessed at.
//!
//! What this pass does not do: thread domains (a borrow across pools;
//! the per-instance pool inference is codegen's) and the data case of
//! GH #712 (a container a handler built, handed to a resident's
//! `@form` field), which needs the field kinds this walk does not
//! model. Both are named in the issue.
//!
//! Beside it, the GH #737 notice: a keyed subscription reads its key
//! when it is registered, at construction and before `birth()`, so a
//! key field assigned in `birth()` did not reach the subscription.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    Block, BusMember, ElseBranch, Expr, KeyFilter, LifecycleKind,
    LocusMember, MatchArmBody, Program, Stmt, TopDecl,
    TypeExpr,
};
use hale_syntax::error::Diag;
use hale_syntax::Span;

/// The lifetime rule and the key notice over `programs`.
pub fn borrow_lifetime_diags(programs: &[&Program]) -> Vec<Diag> {
    let world = World::gather(programs);
    let mut diags = Vec::new();
    for p in programs {
        let mut w = Walk { world: &world, diags: &mut diags, ctx: None, calls: &world.calls };
        w.items(&p.items);
    }
    diags.extend(keyed_in_birth(programs));
    diags
}

// ---------------------------------------------------------------------
// what the program declares
// ---------------------------------------------------------------------

#[derive(Default, Clone)]
struct LocusFacts {
    /// param name -> the declared type's name (last path segment)
    params: BTreeMap<String, String>,
    /// the child types this locus accepts
    accepts: BTreeSet<String>,
    /// the fns a `subscribe … as h` names
    handlers: BTreeSet<String>,
    /// param fields mentioned as `self.<f>` anywhere but `birth()`: a
    /// borrowed handle read only in `birth()` lives no longer than the
    /// instantiation it is born in, which runs inside the frame that
    /// owns the handle — a birth-scoped borrow, sound by construction
    used_outside_birth: BTreeSet<String>,
}

#[derive(Clone)]
struct CallSite {
    /// the fn called: `(locus, name)` for a method on `self` or on a
    /// field of `self` whose type is known, `(None, name)` for a free fn
    callee: (Option<String>, String),
    args: Vec<Expr>,
    /// the context the call is made from
    in_locus: Option<String>,
    in_fn: String,
    in_handler: bool,
    params: Vec<String>,
    /// the caller's locals in scope at the call, name -> (depth, class)
    locals: BTreeMap<String, Source>,
}

struct World {
    loci: BTreeMap<String, LocusFacts>,
    carriers: BTreeSet<String>, // locus, interface and perspective names
    calls: Vec<CallSite>,
}

impl World {
    fn gather(programs: &[&Program]) -> World {
        let mut w = World { loci: BTreeMap::new(), carriers: BTreeSet::new(), calls: Vec::new() };
        for p in programs {
            w.gather_items(&p.items);
        }
        // the call sites, once the declarations are known
        let mut sites = Vec::new();
        for p in programs {
            let mut c = CallCollector { world: &w, out: &mut sites, ctx: None };
            c.items(&p.items);
        }
        w.calls = sites;
        w
    }
    fn gather_items(&mut self, items: &[TopDecl]) {
        for item in items {
            match item {
                TopDecl::Module(m) => self.gather_items(&m.items),
                TopDecl::Interface(i) => {
                    self.carriers.insert(i.name.name.clone());
                }
                TopDecl::Perspective(pd) => {
                    self.carriers.insert(pd.name.name.clone());
                }
                TopDecl::Locus(l) => {
                    self.carriers.insert(l.name.name.clone());
                    let mut f = LocusFacts::default();
                    for m in &l.members {
                        match m {
                            LocusMember::Params(pb) => {
                                for pd in &pb.params {
                                    if let Some(t) = &pd.ty {
                                        if let Some(n) = type_name(t) {
                                            f.params.insert(pd.name.name.clone(), n);
                                        }
                                    }
                                }
                            }
                            LocusMember::Lifecycle(lc) if lc.kind == LifecycleKind::Accept => {
                                if let Some(p) = lc.params.first() {
                                    if let Some(n) = type_name(&p.ty) {
                                        f.accepts.insert(n);
                                    }
                                }
                            }
                            LocusMember::Bus(b) => {
                                for bm in &b.members {
                                    if let BusMember::Subscribe { handler, .. } = bm {
                                        f.handlers.insert(handler.name.clone());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    for m in &l.members {
                        let body = match m {
                            LocusMember::Fn(fd) => &fd.body,
                            LocusMember::Lifecycle(lc) if lc.kind != LifecycleKind::Birth => &lc.body,
                            LocusMember::Mode(md) => &md.body,
                            _ => continue,
                        };
                        self_fields_in_block(body, &mut f.used_outside_birth);
                    }
                    self.loci.insert(l.name.name.clone(), f);
                }
                _ => {}
            }
        }
    }
    fn carries_locus(&self, ty_name: &str) -> bool {
        self.carriers.contains(ty_name)
    }
}

fn type_name(t: &TypeExpr) -> Option<String> {
    match t {
        TypeExpr::Named { path, .. } => path.segments.last().map(|s| s.name.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// where a handle comes from, and who owns a holder
// ---------------------------------------------------------------------

/// Where a handle came from, as far as position says.
#[derive(Clone, Debug, PartialEq)]
enum Source {
    /// `self.f` (or deeper under `self`): lives as long as `self`
    SelfField(String),
    /// a `let` of this frame that OWNS a locus (a literal, a call):
    /// the block depth it was declared at
    Owned { name: String, depth: usize },
    /// a `let` that names something else (an alias): its source
    Alias(Box<Source>),
    /// a parameter of this fn
    Param(String),
    /// the payload parameter of a bus handler: the dispatch's
    Payload(String),
    /// nothing this walk can place
    Unknown,
}

/// Who reclaims the locus literal being built.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Holder {
    /// a frame binding or temporary, at this block depth
    Frame(usize),
    /// a field of `self`, or a child `self` accepts
    SelfOwned,
    /// handed back to the caller
    Caller,
}

/// The context of one fn or lifecycle body.
struct Ctx {
    locus: Option<String>,
    fn_name: String,
    params: Vec<String>,
    is_handler: bool,
    /// name -> source, innermost binding wins; with the depth it was
    /// declared at
    locals: Vec<(String, Source)>,
    depth: usize,
}

impl Ctx {
    fn resolve(&self, name: &str) -> Source {
        for (n, s) in self.locals.iter().rev() {
            if n == name {
                return s.clone();
            }
        }
        if self.params.iter().any(|p| p == name) {
            if self.is_handler && self.params.first().map(|p| p == name).unwrap_or(false) {
                return Source::Payload(name.to_string());
            }
            return Source::Param(name.to_string());
        }
        Source::Unknown
    }
}

/// The source of a handle expression in `ctx`, or None when the
/// expression is not a handle (a literal, a call, an operator).
fn source_of(e: &Expr, ctx: &Ctx) -> Option<Source> {
    match e {
        Expr::Ident(id) => Some(ctx.resolve(&id.name)),
        Expr::Field { receiver, .. } => {
            // self.f, self.a.b, x.f: the root decides
            let mut r = receiver.as_ref();
            loop {
                match r {
                    Expr::KwSelf(_) => {
                        return Some(Source::SelfField(field_path(e)));
                    }
                    Expr::Field { receiver: inner, .. } => r = inner.as_ref(),
                    Expr::Ident(id) => {
                        return Some(match ctx.resolve(&id.name) {
                            Source::Unknown => Source::Unknown,
                            other => other,
                        });
                    }
                    _ => return Some(Source::Unknown),
                }
            }
        }
        _ => None,
    }
}

fn field_path(e: &Expr) -> String {
    match e {
        Expr::Field { receiver, name, .. } => {
            let head = match receiver.as_ref() {
                Expr::KwSelf(_) => "self".to_string(),
                other => field_path(other),
            };
            format!("{}.{}", head, name.name)
        }
        Expr::Ident(id) => id.name.clone(),
        _ => "…".to_string(),
    }
}

fn strip_alias(s: &Source) -> Source {
    match s {
        Source::Alias(inner) => strip_alias(inner),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------
// the walk
// ---------------------------------------------------------------------

struct Walk<'a> {
    world: &'a World,
    diags: &'a mut Vec<Diag>,
    ctx: Option<Ctx>,
    calls: &'a [CallSite],
}

/// How a locus literal sits in its statement.
#[derive(Clone, Copy)]
enum Position {
    Let,
    SelfFieldAssign,
    BareStmt,
    Return,
    Other,
}

impl<'a> Walk<'a> {
    fn items(&mut self, items: &[TopDecl]) {
        for item in items {
            match item {
                TopDecl::Module(m) => self.items(&m.items),
                TopDecl::Fn(fd) => {
                    self.ctx = Some(Ctx {
                        locus: None,
                        fn_name: fd.name.name.clone(),
                        params: fd.params.iter().map(|p| p.name.name.clone()).collect(),
                        is_handler: false,
                        locals: Vec::new(),
                        depth: 0,
                    });
                    self.block(&fd.body);
                    self.ctx = None;
                }
                TopDecl::Locus(l) => {
                    let facts = self.world.loci.get(&l.name.name).cloned().unwrap_or_default();
                    for m in &l.members {
                        let (name, params, body) = match m {
                            LocusMember::Fn(fd) => (
                                fd.name.name.clone(),
                                fd.params.iter().map(|p| p.name.name.clone()).collect::<Vec<_>>(),
                                &fd.body,
                            ),
                            LocusMember::Lifecycle(lc) => (
                                lifecycle_name(lc.kind).to_string(),
                                lc.params.iter().map(|p| p.name.name.clone()).collect(),
                                &lc.body,
                            ),
                            LocusMember::Mode(md) => (
                                "<mode>".to_string(),
                                md.params.iter().map(|p| p.name.name.clone()).collect(),
                                &md.body,
                            ),
                            _ => continue,
                        };
                        self.ctx = Some(Ctx {
                            locus: Some(l.name.name.clone()),
                            is_handler: facts.handlers.contains(&name),
                            fn_name: name,
                            params,
                            locals: Vec::new(),
                            depth: 0,
                        });
                        self.block(body);
                        self.ctx = None;
                    }
                }
                _ => {}
            }
        }
    }

    fn block(&mut self, b: &Block) {
        let mark = self.ctx.as_ref().map(|c| c.locals.len()).unwrap_or(0);
        if let Some(c) = self.ctx.as_mut() {
            c.depth += 1;
        }
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t, Position::Other);
        }
        if let Some(c) = self.ctx.as_mut() {
            c.depth -= 1;
            c.locals.truncate(mark);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                self.expr(value, Position::Let);
                let src = match value {
                    Expr::Struct { .. } | Expr::Call { .. } | Expr::Or { .. } => {
                        let depth = self.ctx.as_ref().map(|c| c.depth).unwrap_or(0);
                        Source::Owned { name: name.name.clone(), depth }
                    }
                    other => match self.ctx.as_ref().and_then(|c| source_of(other, c)) {
                        Some(s) => Source::Alias(Box::new(s)),
                        None => Source::Unknown,
                    },
                };
                if let Some(c) = self.ctx.as_mut() {
                    c.locals.push((name.name.clone(), src));
                }
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value, Position::Other);
                if let Some(c) = self.ctx.as_mut() {
                    for n in names {
                        c.locals.push((n.name.clone(), Source::Unknown));
                    }
                }
            }
            Stmt::Assign { target, value, .. } => {
                let pos = if target.head.name == "self" && target.tail.len() == 1 {
                    Position::SelfFieldAssign
                } else {
                    Position::Other
                };
                self.expr(value, pos);
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(m) => self.match_stmt(m),
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter, Position::Other);
                if let Some(c) = self.ctx.as_mut() {
                    c.locals.push((name.name.clone(), Source::Unknown));
                }
                self.block(body);
                if let Some(c) = self.ctx.as_mut() {
                    c.locals.pop();
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, Position::Other);
                self.block(body);
            }
            Stmt::Return(Some(e), _) => self.expr(e, Position::Return),
            Stmt::Fail { value, .. } => self.expr(value, Position::Other),
            Stmt::Block(b) => self.block(b),
            Stmt::Send { subject, value, .. } => {
                self.expr(subject, Position::Other);
                self.expr(value, Position::Other);
            }
            Stmt::ShmWrite { body, .. } => self.block(body),
            Stmt::Expr(e) => self.expr(e, Position::BareStmt),
            _ => {}
        }
    }

    fn if_stmt(&mut self, i: &hale_syntax::ast::IfStmt) {
        self.expr(&i.cond, Position::Other);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(n)) => self.if_stmt(n),
            None => {}
        }
    }

    fn match_stmt(&mut self, m: &hale_syntax::ast::MatchStmt) {
        self.expr(&m.scrutinee, Position::Other);
        for arm in &m.arms {
            match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e, Position::Other),
                MatchArmBody::Block(b) => self.block(b),
            }
        }
    }

    fn expr(&mut self, e: &Expr, pos: Position) {
        match e {
            Expr::Struct { path, inits, span, .. } => {
                let name = path.segments.last().map(|s| s.name.clone()).unwrap_or_default();
                if let Some(facts) = self.world.loci.get(&name).cloned() {
                    let holder = self.holder_of(&name, pos);
                    for init in inits {
                        let Some(field_ty) = facts.params.get(&init.name.name) else { continue };
                        if !self.world.carries_locus(field_ty) {
                            continue;
                        }
                        let Some(ctx) = self.ctx.as_ref() else { continue };
                        let Some(src) = source_of(&init.value, ctx) else { continue };
                        self.decide(&name, &init.name.name, &strip_alias(&src), holder, init.value.span(), *span);
                    }
                }
                for init in inits {
                    // a nested literal is owned by the outer one
                    self.expr(&init.value, pos);
                }
            }
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner, pos);
                if let hale_syntax::ast::OrDisposition::Substitute(sub) = disposition {
                    self.expr(sub, pos);
                }
            }
            Expr::Call { callee, args, .. } => {
                self.expr(callee, Position::Other);
                for a in args {
                    self.expr(a, Position::Other);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left, Position::Other);
                self.expr(right, Position::Other);
            }
            Expr::Unary { operand, .. } => self.expr(operand, Position::Other),
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
                self.expr(receiver, Position::Other)
            }
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver, Position::Other);
                self.expr(index, Position::Other);
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                for x in es {
                    self.expr(x, Position::Other);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Sum(x, _) | Expr::Prod(x, _) => self.expr(x, Position::Other),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left, Position::Other);
                self.expr(right, Position::Other);
                self.expr(tolerance, Position::Other);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo, Position::Other);
                self.expr(hi, Position::Other);
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val, Position::Other),
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }

    fn holder_of(&self, literal_locus: &str, pos: Position) -> Holder {
        let ctx = self.ctx.as_ref();
        let depth = ctx.map(|c| c.depth).unwrap_or(0);
        match pos {
            Position::Let | Position::Other => Holder::Frame(depth),
            Position::SelfFieldAssign => Holder::SelfOwned,
            Position::Return => Holder::Caller,
            Position::BareStmt => {
                let accepted = ctx
                    .and_then(|c| c.locus.as_ref())
                    .and_then(|l| self.world.loci.get(l))
                    .map(|f| f.accepts.contains(literal_locus))
                    .unwrap_or(false);
                if accepted { Holder::SelfOwned } else { Holder::Frame(depth) }
            }
        }
    }

    fn decide(&mut self, locus: &str, field: &str, src: &Source, holder: Holder, at: Span, literal: Span) {
        let ctx = self.ctx.as_ref().expect("in a body");
        let where_ = match &ctx.locus {
            Some(l) => format!("`{}.{}`", l, ctx.fn_name),
            None => format!("`{}`", ctx.fn_name),
        };
        let refuse = |diags: &mut Vec<Diag>, what: String| {
            diags.push(Diag::ty(
                at,
                format!(
                    "`{}.{}` would hold a borrow that does not outlive it: {} \
                     A locus stored by name is borrowed, never owned by the \
                     holder (F.39), so its owner must reclaim it after the \
                     holder is gone. Build the value where the holder lives, \
                     hand a field of `self` instead, or hold it by a \
                     shorter-lived binding (GH #730).",
                    locus, field, what
                ),
            ));
        };
        // a borrow the holder reads only in `birth()` is birth-scoped:
        // the instantiation runs inside the frame or dispatch that owns
        // the handle, so nothing outlives anything
        if holder == Holder::SelfOwned {
            let birth_only = self
                .world
                .loci
                .get(locus)
                .map(|f| !f.used_outside_birth.contains(field))
                .unwrap_or(false);
            if birth_only {
                return;
            }
        }
        match (src, holder) {
            (Source::SelfField(f), Holder::Caller) => refuse(
                self.diags,
                format!(
                    "`{}` is a field of `self`, and the literal built in {} is returned \
                     to the caller, who may keep it after `self` is gone.",
                    f, where_
                ),
            ),
            (Source::Owned { name, .. }, Holder::SelfOwned) => refuse(
                self.diags,
                format!(
                    "`{}` is a `let` of {} — reclaimed when its scope ends — and the \
                     literal is `self`'s (a field, or an accepted child), which lives on.",
                    name, where_
                ),
            ),
            (Source::Owned { name, .. }, Holder::Caller) => refuse(
                self.diags,
                format!(
                    "`{}` is a `let` of {}, reclaimed when {} returns, and the literal \
                     is what {} returns.",
                    name, where_, where_, where_
                ),
            ),
            (Source::Owned { name, depth }, Holder::Frame(d)) if *depth > d => refuse(
                self.diags,
                format!(
                    "`{}` is bound in a block inside {} and is reclaimed when that block \
                     ends, before the binding that holds the literal.",
                    name, where_
                ),
            ),
            (Source::Payload(p), Holder::SelfOwned) => refuse(
                self.diags,
                format!(
                    "`{}` is the payload delivered to the handler {} and lives for that \
                     dispatch only; the literal is a resident of `self` that outlives it \
                     (GH #712).",
                    p, where_
                ),
            ),
            (Source::Payload(p), Holder::Caller) => refuse(
                self.diags,
                format!(
                    "`{}` is the payload delivered to the handler {}, gone when the \
                     handler returns, and the literal is returned.",
                    p, where_
                ),
            ),
            (Source::Param(p), Holder::SelfOwned) => {
                // every caller is asked: the first that hands a
                // frame-owned or dispatch-owned value is the witness
                let callee = (ctx.locus.clone(), ctx.fn_name.clone());
                let index = ctx.params.iter().position(|x| x == p);
                if let (Some(i), true) = (index, ctx.locus.is_some()) {
                    if let Some((site, why)) = self.witness(&callee, i, 0, &mut Vec::new()) {
                        self.diags.push(Diag::ty(
                            at,
                            format!(
                                "`{}.{}` would hold a borrow that does not outlive it: `{}` \
                                 is a parameter of {}, and the literal is `self`'s (a field, \
                                 or an accepted child). At the call in `{}`, {} \
                                 A locus stored by name is borrowed, never owned by the \
                                 holder (F.39); hand it a field of `self`, or a value that \
                                 lives as long as the holder (GH #730).",
                                locus, field, p, where_, site.in_fn, why
                            ),
                        ));
                    }
                }
                let _ = literal;
            }
            _ => {}
        }
    }

    /// The first call site of `callee` whose argument `i` is a value
    /// that dies with its frame or its dispatch — or, through a
    /// parameter, reaches one — with the reason.
    fn witness(
        &self,
        callee: &(Option<String>, String),
        i: usize,
        depth: usize,
        seen: &mut Vec<(Option<String>, String)>,
    ) -> Option<(CallSite, String)> {
        if depth > 4 || seen.contains(callee) {
            return None;
        }
        seen.push(callee.clone());
        for site in self.calls.iter().filter(|s| &s.callee == callee) {
            let Some(arg) = site.args.get(i) else { continue };
            let ctx = Ctx {
                locus: site.in_locus.clone(),
                fn_name: site.in_fn.clone(),
                params: site.params.clone(),
                is_handler: site.in_handler,
                locals: site.locals.iter().map(|(n, s)| (n.clone(), s.clone())).collect(),
                depth: 0,
            };
            let Some(src) = source_of(arg, &ctx) else { continue };
            match strip_alias(&src) {
                Source::Owned { name, .. } => {
                    return Some((
                        site.clone(),
                        format!("the argument `{}` is a `let` of that frame, reclaimed when it returns.", name),
                    ));
                }
                Source::Payload(p) => {
                    return Some((
                        site.clone(),
                        format!("the argument `{}` is the payload of that handler, gone when it returns.", p),
                    ));
                }
                Source::Param(p) => {
                    let inner = (site.in_locus.clone(), site.in_fn.clone());
                    if let Some(j) = site.params.iter().position(|x| x == &p) {
                        if let Some((deeper, why)) = self.witness(&inner, j, depth + 1, seen) {
                            return Some((
                                deeper,
                                format!("through `{}`'s parameter `{}`, {}", site.in_fn, p, why),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }
}

fn lifecycle_name(k: LifecycleKind) -> &'static str {
    match k {
        LifecycleKind::Birth => "birth",
        LifecycleKind::Accept => "accept",
        LifecycleKind::Release => "release",
        LifecycleKind::Run => "run",
        LifecycleKind::Drain => "drain",
        LifecycleKind::Dissolve => "dissolve",
    }
}

// ---------------------------------------------------------------------
// call sites, with the caller's context frozen at the call
// ---------------------------------------------------------------------

struct CallCollector<'a> {
    world: &'a World,
    out: &'a mut Vec<CallSite>,
    ctx: Option<Ctx>,
}

impl<'a> CallCollector<'a> {
    fn items(&mut self, items: &[TopDecl]) {
        for item in items {
            match item {
                TopDecl::Module(m) => self.items(&m.items),
                TopDecl::Fn(fd) => {
                    self.ctx = Some(Ctx {
                        locus: None,
                        fn_name: fd.name.name.clone(),
                        params: fd.params.iter().map(|p| p.name.name.clone()).collect(),
                        is_handler: false,
                        locals: Vec::new(),
                        depth: 0,
                    });
                    self.block(&fd.body);
                    self.ctx = None;
                }
                TopDecl::Locus(l) => {
                    let facts = self.world.loci.get(&l.name.name).cloned().unwrap_or_default();
                    for m in &l.members {
                        let (name, params, body) = match m {
                            LocusMember::Fn(fd) => (
                                fd.name.name.clone(),
                                fd.params.iter().map(|p| p.name.name.clone()).collect::<Vec<_>>(),
                                &fd.body,
                            ),
                            LocusMember::Lifecycle(lc) => (
                                lifecycle_name(lc.kind).to_string(),
                                lc.params.iter().map(|p| p.name.name.clone()).collect(),
                                &lc.body,
                            ),
                            LocusMember::Mode(md) => (
                                "<mode>".to_string(),
                                md.params.iter().map(|p| p.name.name.clone()).collect(),
                                &md.body,
                            ),
                            _ => continue,
                        };
                        self.ctx = Some(Ctx {
                            locus: Some(l.name.name.clone()),
                            is_handler: facts.handlers.contains(&name),
                            fn_name: name,
                            params,
                            locals: Vec::new(),
                            depth: 0,
                        });
                        self.block(body);
                        self.ctx = None;
                    }
                }
                _ => {}
            }
        }
    }
    fn block(&mut self, b: &Block) {
        let mark = self.ctx.as_ref().map(|c| c.locals.len()).unwrap_or(0);
        if let Some(c) = self.ctx.as_mut() {
            c.depth += 1;
        }
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
        if let Some(c) = self.ctx.as_mut() {
            c.depth -= 1;
            c.locals.truncate(mark);
        }
    }
    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                self.expr(value);
                let src = match value {
                    Expr::Struct { .. } | Expr::Call { .. } | Expr::Or { .. } => {
                        let depth = self.ctx.as_ref().map(|c| c.depth).unwrap_or(0);
                        Source::Owned { name: name.name.clone(), depth }
                    }
                    other => match self.ctx.as_ref().and_then(|c| source_of(other, c)) {
                        Some(s) => Source::Alias(Box::new(s)),
                        None => Source::Unknown,
                    },
                };
                if let Some(c) = self.ctx.as_mut() {
                    c.locals.push((name.name.clone(), src));
                }
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value);
                if let Some(c) = self.ctx.as_mut() {
                    for n in names {
                        c.locals.push((n.name.clone(), Source::Unknown));
                    }
                }
            }
            Stmt::Assign { value, .. } => self.expr(value),
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(m) => {
                self.expr(&m.scrutinee);
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e),
                        MatchArmBody::Block(b) => self.block(b),
                    }
                }
            }
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter);
                if let Some(c) = self.ctx.as_mut() {
                    c.locals.push((name.name.clone(), Source::Unknown));
                }
                self.block(body);
                if let Some(c) = self.ctx.as_mut() {
                    c.locals.pop();
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::Return(Some(e), _) | Stmt::Fail { value: e, .. } | Stmt::Expr(e) => self.expr(e),
            Stmt::Block(b) => self.block(b),
            Stmt::Send { subject, value, .. } => {
                self.expr(subject);
                self.expr(value);
            }
            Stmt::ShmWrite { body, .. } => self.block(body),
            _ => {}
        }
    }
    fn if_stmt(&mut self, i: &hale_syntax::ast::IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(n)) => self.if_stmt(n),
            None => {}
        }
    }
    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Call { callee, args, .. } => {
                if let Some(ctx) = self.ctx.as_ref() {
                    let target: Option<(Option<String>, String)> = match callee.as_ref() {
                        Expr::Ident(id) => Some((None, id.name.clone())),
                        Expr::Path(qn) if qn.segments.len() == 1 => {
                            Some((None, qn.segments[0].name.clone()))
                        }
                        Expr::Field { receiver, name, .. } => match receiver.as_ref() {
                            // self.m(…)
                            Expr::KwSelf(_) => ctx.locus.clone().map(|l| (Some(l), name.name.clone())),
                            // self.f.m(…): the field's declared type
                            Expr::Field { receiver: r2, name: f, .. } if matches!(r2.as_ref(), Expr::KwSelf(_)) => ctx
                                .locus
                                .as_ref()
                                .and_then(|l| self.world.loci.get(l))
                                .and_then(|facts| facts.params.get(&f.name).cloned())
                                .map(|t| (Some(t), name.name.clone())),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(t) = target {
                        self.out.push(CallSite {
                            callee: t,
                            args: args.clone(),
                            in_locus: ctx.locus.clone(),
                            in_fn: ctx.fn_name.clone(),
                            in_handler: ctx.is_handler,
                            params: ctx.params.clone(),
                            locals: ctx.locals.iter().map(|(n, s)| (n.clone(), s.clone())).collect(),
                        });
                    }
                }
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                if let hale_syntax::ast::OrDisposition::Substitute(sub) = disposition {
                    self.expr(sub);
                }
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.expr(&i.value);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                for x in es {
                    self.expr(x);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => {
                self.expr(&m.scrutinee);
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e),
                        MatchArmBody::Block(b) => self.block(b),
                    }
                }
            }
            Expr::Sum(x, _) | Expr::Prod(x, _) | Expr::ArrayRepeat { val: x, .. } => self.expr(x),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }
}

// ---------------------------------------------------------------------
// GH #737: the key is read when the subscription is registered
// ---------------------------------------------------------------------

fn keyed_in_birth(programs: &[&Program]) -> Vec<Diag> {
    let mut diags = Vec::new();
    fn go(items: &[TopDecl], diags: &mut Vec<Diag>) {
        for item in items {
            match item {
                TopDecl::Module(m) => go(&m.items, diags),
                TopDecl::Locus(l) => {
                    // the fields a `where key == self.f` reads
                    let mut keyed: BTreeMap<String, String> = BTreeMap::new();
                    for m in &l.members {
                        if let LocusMember::Bus(b) = m {
                            for bm in &b.members {
                                if let BusMember::Subscribe { subject, key_filter: Some(KeyFilter::Specific { expr, .. }), .. } = bm {
                                    if let Expr::Field { receiver, name, .. } = expr {
                                        if matches!(receiver.as_ref(), Expr::KwSelf(_)) {
                                            let topic = match subject {
                                                hale_syntax::ast::BusSubject::Topic(t) => t.name.clone(),
                                                hale_syntax::ast::BusSubject::Literal { subject, .. } => subject.clone(),
                                                hale_syntax::ast::BusSubject::QualifiedTopic(qn) => qn
                                                    .segments
                                                    .iter()
                                                    .map(|s| s.name.clone())
                                                    .collect::<Vec<_>>()
                                                    .join("::"),
                                            };
                                            keyed.insert(name.name.clone(), topic);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if keyed.is_empty() {
                        continue;
                    }
                    for m in &l.members {
                        if let LocusMember::Lifecycle(lc) = m {
                            if lc.kind == LifecycleKind::Birth {
                                for s in &lc.body.stmts {
                                    if let Stmt::Assign { target, span, .. } = s {
                                        if target.head.name == "self" && target.tail.len() == 1 {
                                            if let hale_syntax::ast::LValueSeg::Field(f) = &target.tail[0] {
                                                if let Some(topic) = keyed.get(&f.name) {
                                                    diags.push(Diag::warn(
                                                        *span,
                                                        format!(
                                                            "`{}` keys its subscription to `{}` by `self.{}`, and the key was read \
                                                             when the subscription was registered — at construction, before \
                                                             `birth()` ran. This assignment does not retarget it: the \
                                                             subscription stays on the field's value at the literal. Pass \
                                                             `{}` as a param at the literal (`{} {{ {}: … }}`) (GH #737).",
                                                            l.name.name, topic, f.name, f.name, l.name.name, f.name
                                                        ),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for p in programs {
        go(&p.items, &mut diags);
    }
    diags
}

/// Every `self.<f>` a block mentions, read or written.
fn self_fields_in_block(b: &Block, out: &mut BTreeSet<String>) {
    fn expr(e: &Expr, out: &mut BTreeSet<String>) {
        match e {
            Expr::Field { receiver, name, .. } => {
                if matches!(receiver.as_ref(), Expr::KwSelf(_)) {
                    out.insert(name.name.clone());
                }
                expr(receiver, out);
            }
            Expr::Path2 { receiver, .. } | Expr::Unary { operand: receiver, .. } => expr(receiver, out),
            Expr::Call { callee, args, .. } => {
                expr(callee, out);
                for a in args {
                    expr(a, out);
                }
            }
            Expr::Binary { left, right, .. } => {
                expr(left, out);
                expr(right, out);
            }
            Expr::Index { receiver, index, .. } => {
                expr(receiver, out);
                expr(index, out);
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                for x in es {
                    expr(x, out);
                }
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    expr(&i.value, out);
                }
            }
            Expr::Block(b) => self_fields_in_block(b, out),
            Expr::If(i) => if_stmt(i, out),
            Expr::Match(m) => {
                expr(&m.scrutinee, out);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        expr(g, out);
                    }
                    match &arm.body {
                        MatchArmBody::Expr(e) => expr(e, out),
                        MatchArmBody::Block(b) => self_fields_in_block(b, out),
                    }
                }
            }
            Expr::Sum(x, _) | Expr::Prod(x, _) | Expr::ArrayRepeat { val: x, .. } => expr(x, out),
            Expr::Approx { left, right, tolerance, .. } => {
                expr(left, out);
                expr(right, out);
                expr(tolerance, out);
            }
            Expr::Range { lo, hi, .. } => {
                expr(lo, out);
                expr(hi, out);
            }
            Expr::Or { inner, disposition, .. } => {
                expr(inner, out);
                if let hale_syntax::ast::OrDisposition::Substitute(sub) = disposition {
                    expr(sub, out);
                }
            }
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }
    fn if_stmt(i: &hale_syntax::ast::IfStmt, out: &mut BTreeSet<String>) {
        expr(&i.cond, out);
        self_fields_in_block(&i.then_block, out);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self_fields_in_block(b, out),
            Some(ElseBranch::ElseIf(n)) => if_stmt(n, out),
            None => {}
        }
    }
    for s in &b.stmts {
        match s {
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Expr(value) => expr(value, out),
            Stmt::Assign { target, value, .. } => {
                if target.head.name == "self" {
                    if let Some(hale_syntax::ast::LValueSeg::Field(f)) = target.tail.first() {
                        out.insert(f.name.clone());
                    }
                }
                for seg in &target.tail {
                    if let hale_syntax::ast::LValueSeg::Index(ix) = seg {
                        expr(ix, out);
                    }
                }
                expr(value, out);
            }
            Stmt::If(i) => if_stmt(i, out),
            Stmt::Match(m) => {
                expr(&m.scrutinee, out);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        expr(g, out);
                    }
                    match &arm.body {
                        MatchArmBody::Expr(e) => expr(e, out),
                        MatchArmBody::Block(b) => self_fields_in_block(b, out),
                    }
                }
            }
            Stmt::For { iter, body, .. } => {
                expr(iter, out);
                self_fields_in_block(body, out);
            }
            Stmt::While { cond, body, .. } => {
                expr(cond, out);
                self_fields_in_block(body, out);
            }
            Stmt::Return(Some(e), _) | Stmt::Fail { value: e, .. } => expr(e, out),
            Stmt::Block(b) | Stmt::ShmWrite { body: b, .. } => self_fields_in_block(b, out),
            Stmt::Send { subject, value, .. } => {
                expr(subject, out);
                expr(value, out);
            }
            Stmt::Recovery { args, .. } => {
                for a in args {
                    expr(a, out);
                }
            }
            Stmt::Violate { payload: Some(e), .. } => expr(e, out),
            _ => {}
        }
    }
    if let Some(t) = &b.tail {
        expr(t, out);
    }
}
