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
//! GH #1048 extends the table to a handle a **method keeps**: a
//! parameter of a locus-carrying type that the method's body stores into
//! `self` (a field, a container of `self`'s through a mutator, another
//! keeping method). `std::http::Router.add` keeps its handler, read from
//! the stdlib's own body. The argument at such a call is a borrow the
//! receiver holds, decided by the same table with the receiver as the
//! holder, so a handler literal built in a function that returns the
//! router, or hands it to someone else's, is refused (see `Kept`).
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
    borrow_lifetime_diags_with_renames(programs, &[])
}

/// The same, resolving an imported seed's `alias::Name` to the merged
/// program's mangled locus (GH #1048: a keeping method in a library).
pub fn borrow_lifetime_diags_with_renames(programs: &[&Program], renames: &[(Vec<String>, String)]) -> Vec<Diag> {
    let world = World::gather(programs, renames);
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
    /// GH #1048: the keeping facts, over the user's loci and the
    /// Hale-source stdlib's (by mangled name)
    kept: Kept,
}

impl World {
    fn gather(programs: &[&Program], renames: &[(Vec<String>, String)]) -> World {
        let mut w = World { loci: BTreeMap::new(), carriers: BTreeSet::new(), calls: Vec::new(), kept: Kept::default() };
        let mut with_stdlib: Vec<&Program> = programs.to_vec();
        if let Some(std_prog) = crate::stdlib_bodies::program() {
            with_stdlib.push(std_prog);
        }
        w.kept = Kept::gather(&with_stdlib, renames);
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
// GH #1048: what a method keeps
// ---------------------------------------------------------------------
//
// `r.add("GET", "/x", Echo { … })` hands the router a handle it stores
// (`self.entries.push(Entry { handler: h, … })`), so the argument is a
// borrow the ROUTER holds — the same borrow GH #730 decides for a
// literal's field, reached through a method. Which parameters a method
// keeps is read from its body, the stdlib's included: a parameter of a
// locus-carrying type stored into `self` — a field, a `@form`
// container of `self`'s through one of its mutators, or a method of
// `self`'s that keeps it in turn.

/// The mutators through which a `@form` container stores its argument.
const STORING_MUTATORS: &[&str] = &["push", "push_back", "push_front", "set", "insert", "put", "append"];

/// A binding's identity: its declaring identifier's span.
type DeclKey = (u32, u32);

fn decl_key(i: &hale_syntax::ast::Ident) -> Option<DeclKey> {
    (i.span.end.0 > i.span.start.0).then_some((i.span.start.0, i.span.end.0))
}

#[derive(Default)]
struct Kept {
    /// locus key -> method -> the parameter indices it keeps
    methods: BTreeMap<String, BTreeMap<String, BTreeSet<usize>>>,
    /// locus key -> param field -> the locus key its type names
    fields: BTreeMap<String, BTreeMap<String, String>>,
    /// every locus key: the declared names (the stdlib's and an
    /// imported seed's mangled ones included)
    loci: BTreeSet<String>,
    /// the `@form` loci: containers whose mutators store
    forms: BTreeSet<String>,
    /// every type that carries a locus handle: loci, interfaces,
    /// perspectives — only a parameter of one of these is a handle
    carriers: BTreeSet<String>,
    /// free fn -> the locus it returns, so `let r = fresh();` is typed
    fn_returns: BTreeMap<String, String>,
    /// cross-seed renames (`["tb", "Table"]` -> the mangled name)
    renames: Vec<(Vec<String>, String)>,
}

/// A locus key as the user spells it: a stdlib locus by its public
/// path, an imported one by `alias::Name`.
fn locus_display(key: &str, renames: &[(Vec<String>, String)]) -> String {
    if let Some((path, _)) = hale_stdlib::PATH_RENAMES.iter().find(|(_, m)| *m == key) {
        return path.join("::");
    }
    if let Some((path, _)) = renames.iter().find(|(_, m)| m == key) {
        return path.join("::");
    }
    key.to_string()
}

/// Whether `e` puts `name` itself somewhere as a value: the name, or a
/// record, tuple or array literal holding it.
fn holds_value(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Ident(i) => i.name == name,
        Expr::Struct { inits, .. } => inits.iter().any(|i| holds_value(&i.value, name)),
        Expr::Tuple(es, _) | Expr::Array(es, _) => es.iter().any(|x| holds_value(x, name)),
        _ => false,
    }
}

/// Whether `e` is `self` or a place under it (`self.f`, `self.f.g`).
fn rooted_at_self(e: &Expr) -> bool {
    match e {
        Expr::KwSelf(_) => true,
        Expr::Field { receiver, .. } => rooted_at_self(receiver),
        _ => false,
    }
}

/// A place's root and the fields under it: `a.b.c` -> (`a`, [b, c]).
fn place_chain(e: &Expr) -> Option<(&Expr, Vec<&str>)> {
    match e {
        Expr::Ident(_) | Expr::KwSelf(_) => Some((e, Vec::new())),
        Expr::Field { receiver, name, .. } => {
            let (root, mut chain) = place_chain(receiver)?;
            chain.push(name.name.as_str());
            Some((root, chain))
        }
        _ => None,
    }
}

impl Kept {
    /// A type path's key: a stdlib path as the mangled name the stdlib
    /// declares, an imported seed's as its rename, anything else by its
    /// last segment.
    fn key_of_segs(&self, segs: &[&str]) -> Option<String> {
        if segs.len() > 1 {
            if let Some(m) = crate::stdlib_bodies::mangled_locus_name(segs) {
                return Some(m.to_string());
            }
            if let Some((_, m)) = self.renames.iter().find(|(p, _)| p.len() == segs.len() && p.iter().zip(segs).all(|(a, b)| a == b)) {
                return Some(m.clone());
            }
        }
        segs.last().map(|s| s.to_string())
    }

    fn key_of(&self, t: &TypeExpr) -> Option<String> {
        let TypeExpr::Named { path, .. } = t else { return None };
        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
        self.key_of_segs(&segs)
    }

    /// The locus a literal builds, when it builds one.
    fn literal_locus(&self, e: &Expr) -> Option<String> {
        let Expr::Struct { path, .. } = e else { return None };
        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
        self.key_of_segs(&segs).filter(|k| self.loci.contains(k))
    }

    /// The locus a `let`'s value is, when it says: a locus literal, or a
    /// call to a free fn declared to return one.
    fn value_locus(&self, e: &Expr) -> Option<String> {
        if let Some(k) = self.literal_locus(e) {
            return Some(k);
        }
        if let Expr::Call { callee, .. } = e {
            let name = match callee.as_ref() {
                Expr::Ident(id) => Some(id.name.as_str()),
                Expr::Path(qn) if qn.segments.len() == 1 => Some(qn.segments[0].name.as_str()),
                _ => None,
            };
            return name.and_then(|n| self.fn_returns.get(n).cloned());
        }
        None
    }

    fn gather(programs: &[&Program], renames: &[(Vec<String>, String)]) -> Kept {
        let mut k = Kept { renames: renames.to_vec(), ..Kept::default() };
        // (locus, method, each param's name when its type carries a
        // handle, body)
        let mut bodies: Vec<(String, String, Vec<Option<String>>, &Block)> = Vec::new();
        fn declared(items: &[TopDecl], k: &mut Kept) {
            for item in items {
                match item {
                    TopDecl::Module(m) => declared(&m.items, k),
                    TopDecl::Locus(l) => {
                        k.carriers.insert(l.name.name.clone());
                        k.loci.insert(l.name.name.clone());
                        if l.form.is_some() {
                            k.forms.insert(l.name.name.clone());
                        }
                    }
                    TopDecl::Interface(i) => {
                        k.carriers.insert(i.name.name.clone());
                    }
                    TopDecl::Perspective(pd) => {
                        k.carriers.insert(pd.name.name.clone());
                    }
                    _ => {}
                }
            }
        }
        for p in programs {
            declared(&p.items, &mut k);
        }
        fn collect<'p>(items: &'p [TopDecl], k: &mut Kept, bodies: &mut Vec<(String, String, Vec<Option<String>>, &'p Block)>) {
            for item in items {
                match item {
                    TopDecl::Module(m) => collect(&m.items, k, bodies),
                    TopDecl::Fn(fd) => {
                        if let Some(r) = fd.ret.as_ref().and_then(|t| k.key_of(t)).filter(|t| k.loci.contains(t)) {
                            k.fn_returns.insert(fd.name.name.clone(), r);
                        }
                    }
                    TopDecl::Locus(l) => {
                        let name = l.name.name.clone();
                        let mut fields = BTreeMap::new();
                        for m in &l.members {
                            match m {
                                LocusMember::Params(pb) => {
                                    for pd in &pb.params {
                                        if let Some(key) = pd.ty.as_ref().and_then(|t| k.key_of(t)) {
                                            fields.insert(pd.name.name.clone(), key);
                                        }
                                    }
                                }
                                LocusMember::Fn(fd) => {
                                    let handles = fd
                                        .params
                                        .iter()
                                        .map(|p| k.key_of(&p.ty).filter(|t| k.carriers.contains(t)).map(|_| p.name.name.clone()))
                                        .collect();
                                    bodies.push((name.clone(), fd.name.name.clone(), handles, &fd.body))
                                }
                                _ => {}
                            }
                        }
                        k.fields.insert(name, fields);
                    }
                    _ => {}
                }
            }
        }
        for p in programs {
            collect(&p.items, &mut k, &mut bodies);
        }
        // to a fixpoint: a method that hands its parameter to one of
        // `self`'s keeping methods keeps it too
        loop {
            let mut grew = false;
            for (locus, method, params, body) in &bodies {
                for (i, p) in params.iter().enumerate() {
                    let Some(p) = p else { continue };
                    let already = k.methods.get(locus).and_then(|m| m.get(method)).map(|s| s.contains(&i)).unwrap_or(false);
                    if !already && k.body_keeps(locus, body, p) {
                        k.methods.entry(locus.clone()).or_default().entry(method.clone()).or_default().insert(i);
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        k
    }

    /// The parameter indices `(locus, method)` keeps.
    fn keeps(&self, locus: &str, method: &str) -> Option<&BTreeSet<usize>> {
        self.methods.get(locus).and_then(|m| m.get(method))
    }

    /// The locus a field chain names, from a root of type `root`.
    fn chain_type(&self, root: &str, chain: &[&str]) -> Option<String> {
        let mut t = root.to_string();
        for f in chain {
            t = self.fields.get(&t)?.get(*f)?.clone();
        }
        Some(t)
    }

    /// The locus key of a place under `self` in `locus` (`self.f`).
    fn place_type(&self, locus: &str, place: &Expr) -> Option<String> {
        let (root, chain) = place_chain(place)?;
        if !matches!(root, Expr::KwSelf(_)) {
            return None;
        }
        self.chain_type(locus, &chain)
    }

    /// Whether `b` stores the parameter `p` into `self`.
    fn body_keeps(&self, locus: &str, b: &Block, p: &str) -> bool {
        b.stmts.iter().any(|s| self.stmt_keeps(locus, s, p))
            || b.tail.as_deref().map(|t| self.expr_keeps(locus, t, p)).unwrap_or(false)
    }

    fn stmt_keeps(&self, locus: &str, s: &Stmt, p: &str) -> bool {
        match s {
            Stmt::Assign { target, value, .. } => {
                (target.head.name == "self" && !target.tail.is_empty() && holds_value(value, p))
                    || self.expr_keeps(locus, value, p)
            }
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Expr(value) | Stmt::Fail { value, .. } => {
                self.expr_keeps(locus, value, p)
            }
            Stmt::Return(Some(e), _) => self.expr_keeps(locus, e, p),
            Stmt::If(i) => self.if_keeps(locus, i, p),
            Stmt::Match(m) => m.arms.iter().any(|a| match &a.body {
                MatchArmBody::Expr(e) => self.expr_keeps(locus, e, p),
                MatchArmBody::Block(b) => self.body_keeps(locus, b, p),
            }),
            Stmt::For { body, .. } | Stmt::While { body, .. } | Stmt::Block(body) | Stmt::ShmWrite { body, .. } => {
                self.body_keeps(locus, body, p)
            }
            _ => false,
        }
    }

    fn if_keeps(&self, locus: &str, i: &hale_syntax::ast::IfStmt, p: &str) -> bool {
        self.body_keeps(locus, &i.then_block, p)
            || match i.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => self.body_keeps(locus, b, p),
                Some(ElseBranch::ElseIf(n)) => self.if_keeps(locus, n, p),
                None => false,
            }
    }

    fn expr_keeps(&self, locus: &str, e: &Expr, p: &str) -> bool {
        match e {
            Expr::Call { callee, args, .. } => {
                if let Expr::Field { receiver, name, .. } = callee.as_ref() {
                    if rooted_at_self(receiver) {
                        let recv_ty = self.place_type(locus, receiver);
                        // a mutator stores only into a `@form` container
                        // (or a place whose type this walk cannot say);
                        // any other method stores what its own body keeps
                        let mutator = STORING_MUTATORS.contains(&name.name.as_str())
                            && !matches!(receiver.as_ref(), Expr::KwSelf(_))
                            && recv_ty.as_ref().map(|t| self.forms.contains(t) || !self.loci.contains(t)).unwrap_or(true);
                        let keeping = recv_ty
                            .and_then(|t| self.keeps(&t, &name.name).cloned())
                            .unwrap_or_default();
                        for (i, a) in args.iter().enumerate() {
                            if holds_value(a, p) && (mutator || keeping.contains(&i)) {
                                return true;
                            }
                        }
                    }
                }
                args.iter().any(|a| self.expr_keeps(locus, a, p))
            }
            Expr::Block(b) => self.body_keeps(locus, b, p),
            Expr::If(i) => self.if_keeps(locus, i, p),
            Expr::Or { inner, .. } => self.expr_keeps(locus, inner, p),
            _ => false,
        }
    }
}

// ---- which declarations a body hands back -----------------------------
//
// By binding, not by name: `if c { let r = "none"; return r; }` does not
// make a later `let r = Router { }` the caller's. A `let x = y;` alias
// joins its source's fate, either way round.

struct ReturnScan {
    frames: Vec<Vec<(String, Option<DeclKey>)>>,
    returned: BTreeSet<DeclKey>,
    aliases: Vec<(DeclKey, DeclKey)>,
}

/// The declarations a body hands back, closed over `let` aliases.
fn returned_decls(b: &Block) -> BTreeSet<DeclKey> {
    let mut s = ReturnScan { frames: Vec::new(), returned: BTreeSet::new(), aliases: Vec::new() };
    s.block(b, true);
    let mut out = s.returned;
    loop {
        let before = out.len();
        for (a, b) in &s.aliases {
            if out.contains(a) || out.contains(b) {
                out.insert(*a);
                out.insert(*b);
            }
        }
        if out.len() == before {
            break;
        }
    }
    out
}

impl ReturnScan {
    fn lookup(&self, name: &str) -> Option<DeclKey> {
        for f in self.frames.iter().rev() {
            for (n, k) in f.iter().rev() {
                if n == name {
                    return *k;
                }
            }
        }
        None
    }
    /// A value handed back: a name, or the names a record, tuple or
    /// array literal carries out (`return Pair { r: r, … };`).
    fn hand_back(&mut self, e: &Expr) {
        match e {
            Expr::Ident(i) => {
                if let Some(k) = self.lookup(&i.name) {
                    self.returned.insert(k);
                }
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.hand_back(&i.value);
                }
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                for x in es {
                    self.hand_back(x);
                }
            }
            _ => {}
        }
    }
    fn block(&mut self, b: &Block, counted: bool) {
        self.frames.push(Vec::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = b.tail.as_deref() {
            self.expr(t);
            if counted {
                self.hand_back(t);
            }
        }
        self.frames.pop();
    }
    fn declare(&mut self, name: &hale_syntax::ast::Ident) {
        let k = decl_key(name);
        if let Some(f) = self.frames.last_mut() {
            f.push((name.name.clone(), k));
        }
    }
    fn if_chain(&mut self, i: &hale_syntax::ast::IfStmt, counted: bool) {
        self.expr(&i.cond);
        self.block(&i.then_block, counted);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b, counted),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n, counted),
            None => {}
        }
    }
    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                self.expr(value);
                if let (Expr::Ident(src), Some(k)) = (value, decl_key(name)) {
                    if let Some(from) = self.lookup(&src.name) {
                        self.aliases.push((k, from));
                    }
                }
                self.declare(name);
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value);
                for n in names {
                    self.declare(n);
                }
            }
            Stmt::Return(Some(e), _) => {
                self.expr(e);
                self.hand_back(e);
            }
            Stmt::If(i) => self.if_chain(i, true),
            Stmt::Match(m) => {
                self.expr(&m.scrutinee);
                for a in &m.arms {
                    self.frames.push(Vec::new());
                    match &a.body {
                        MatchArmBody::Block(b) => self.block(b, true),
                        MatchArmBody::Expr(e) => self.expr(e),
                    }
                    self.frames.pop();
                }
            }
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter);
                self.frames.push(vec![(name.name.clone(), None)]);
                self.block(body, true);
                self.frames.pop();
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body, true);
            }
            Stmt::Block(b) | Stmt::ShmWrite { body: b, .. } => self.block(b, true),
            Stmt::Assign { value, .. } | Stmt::Expr(value) | Stmt::Fail { value, .. } => self.expr(value),
            _ => {}
        }
    }
    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Block(b) => self.block(b, false),
            Expr::If(i) => self.if_chain(i, false),
            Expr::Match(m) => {
                self.expr(&m.scrutinee);
                for a in &m.arms {
                    match &a.body {
                        MatchArmBody::Block(b) => self.block(b, false),
                        MatchArmBody::Expr(x) => self.expr(x),
                    }
                }
            }
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                if let hale_syntax::ast::OrDisposition::Substitute(x) = disposition {
                    self.expr(x);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.expr(&i.value);
                }
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                for x in es {
                    self.expr(x);
                }
            }
            _ => {}
        }
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
    /// GH #1048: every binding in scope, innermost last, with what the
    /// keeping check needs of it
    binds: Vec<Bind>,
    /// the declarations this body hands back (by binding, over aliases)
    returned: BTreeSet<DeclKey>,
    /// how many loop bodies enclose the walk
    loop_depth: usize,
}

/// A binding as the GH #1048 check sees it.
#[derive(Clone, Debug)]
struct Bind {
    name: String,
    /// the locus it holds, when position says
    ty: Option<String>,
    /// its declaration, for a `let`
    key: Option<DeclKey>,
    param: bool,
    /// the loop bodies enclosing its declaration
    loop_depth: usize,
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
                        binds: Vec::new(),
                        returned: BTreeSet::new(),
                        loop_depth: 0,
                    });
                    self.enter(&fd.params, &fd.body);
                    self.block(&fd.body);
                    self.ctx = None;
                }
                TopDecl::Locus(l) => {
                    let facts = self.world.loci.get(&l.name.name).cloned().unwrap_or_default();
                    for m in &l.members {
                        let (name, decls, body) = match m {
                            LocusMember::Fn(fd) => (fd.name.name.clone(), &fd.params, &fd.body),
                            LocusMember::Lifecycle(lc) => (lifecycle_name(lc.kind).to_string(), &lc.params, &lc.body),
                            LocusMember::Mode(md) => ("<mode>".to_string(), &md.params, &md.body),
                            _ => continue,
                        };
                        self.ctx = Some(Ctx {
                            locus: Some(l.name.name.clone()),
                            is_handler: facts.handlers.contains(&name),
                            fn_name: name,
                            params: decls.iter().map(|p| p.name.name.clone()).collect(),
                            locals: Vec::new(),
                            depth: 0,
                            binds: Vec::new(),
                            returned: BTreeSet::new(),
                            loop_depth: 0,
                        });
                        self.enter(decls, body);
                        self.block(body);
                        self.ctx = None;
                    }
                }
                _ => {}
            }
        }
    }

    /// GH #1048: a body's params (and their loci) as bindings, and the
    /// declarations it hands back.
    fn enter(&mut self, decls: &[hale_syntax::ast::Param], body: &Block) {
        let kept = &self.world.kept;
        let Some(c) = self.ctx.as_mut() else { return };
        for p in decls {
            c.binds.push(Bind {
                name: p.name.name.clone(),
                ty: kept.key_of(&p.ty).filter(|k| kept.loci.contains(k)),
                key: None,
                param: true,
                loop_depth: 0,
            });
        }
        c.returned = returned_decls(body);
    }

    fn bind(&self, name: &str) -> Option<&Bind> {
        self.ctx.as_ref()?.binds.iter().rev().find(|b| b.name == name)
    }

    /// The locus a receiver is, when position says: a place under
    /// `self`, or under a binding whose locus is known.
    fn receiver_type(&self, recv: &Expr) -> Option<String> {
        let c = self.ctx.as_ref()?;
        let (root, chain) = place_chain(recv)?;
        let root_ty = match root {
            Expr::KwSelf(_) => c.locus.clone()?,
            Expr::Ident(id) => self.bind(&id.name)?.ty.clone()?,
            _ => return None,
        };
        self.world.kept.chain_type(&root_ty, &chain)
    }

    /// Who holds what a receiver keeps — decided by the receiver's root:
    /// `self` (and a child `self` accepted) for a place under it, the
    /// caller for a parameter or a binding the body hands back, else the
    /// frame, at the loop depth the binding was declared at. None for
    /// `self` itself: `self.observe(j)` is a store into `self` spelled as
    /// a call, left where GH #967 leaves the assignment it stands for.
    fn receiver_holder(&self, recv: &Expr) -> Option<(Holder, String, String, usize)> {
        let c = self.ctx.as_ref()?;
        let (root, chain) = place_chain(recv)?;
        let name = format!("`{}`", field_path(recv));
        match root {
            Expr::KwSelf(_) => {
                if chain.is_empty() {
                    return None;
                }
                Some((Holder::SelfOwned, name, "owned by `self`".to_string(), 0))
            }
            Expr::Ident(id) => {
                let b = self.bind(&id.name)?;
                if b.param {
                    // an accepted child is `self`'s
                    if c.fn_name == "accept" && c.params.first() == Some(&id.name) {
                        return Some((Holder::SelfOwned, name, "a child `self` accepted".to_string(), 0));
                    }
                    return Some((Holder::Caller, name, "a parameter, the caller's".to_string(), 0));
                }
                if b.key.map(|k| c.returned.contains(&k)).unwrap_or(false) {
                    return Some((Holder::Caller, name, format!("returned by `{}`", c.fn_name), 0));
                }
                Some((Holder::Frame(b.loop_depth), name, "a binding of this frame".to_string(), b.loop_depth))
            }
            _ => None,
        }
    }

    /// A call to a method that keeps its argument: that argument is a
    /// borrow the receiver holds (GH #1048), decided by the GH #730
    /// table with the receiver as the holder.
    fn kept_call(&mut self, callee: &Expr, args: &[Expr]) {
        let Expr::Field { receiver, name: method, .. } = callee else { return };
        let Some(recv_ty) = self.receiver_type(receiver) else { return };
        let Some(kept) = self.world.kept.keeps(&recv_ty, &method.name).cloned() else { return };
        let Some((holder, holder_name, lives, holder_loops)) = self.receiver_holder(receiver) else { return };
        let renames = &self.world.kept.renames;
        for i in kept {
            let Some(arg) = args.get(i) else { continue };
            let Some(ctx) = self.ctx.as_ref() else { return };
            let where_ = match &ctx.locus {
                Some(l) => format!("`{}.{}`", l, ctx.fn_name),
                None => format!("`{}`", ctx.fn_name),
            };
            // what the argument is — a locus literal built here, or a
            // handle by name — and the loop depth its storage belongs to
            let (src, arg_loops) = match arg {
                Expr::Struct { .. } => match self.world.kept.literal_locus(arg) {
                    Some(k) => (
                        Source::Owned { name: format!("{} {{ … }}", locus_display(&k, renames)), depth: ctx.depth },
                        ctx.loop_depth,
                    ),
                    None => continue,
                },
                other => {
                    let Some(s) = source_of(other, ctx) else { continue };
                    let loops = match other {
                        Expr::Ident(id) => self.bind(&id.name).map(|b| b.loop_depth).unwrap_or(0),
                        _ => 0,
                    };
                    (strip_alias(&s), loops)
                }
            };
            let why = match (&src, holder) {
                (Source::Owned { name, .. }, Holder::SelfOwned | Holder::Caller) => Some(format!(
                    "`{}` is built in {} and reclaimed when that frame ends, while {} ({}) lives on.",
                    name, where_, holder_name, lives
                )),
                // a loop body's locus is reclaimed when the next iteration
                // reuses its slot; any other block's lives to the frame's end
                (Source::Owned { name, .. }, Holder::Frame(_)) if arg_loops > holder_loops => Some(format!(
                    "`{}` is built inside a loop in {} and its storage is reused by the next iteration, while {} outlives the loop.",
                    name, where_, holder_name
                )),
                (Source::Payload(p), Holder::SelfOwned | Holder::Caller) => Some(format!(
                    "`{}` is the payload delivered to the handler {}, gone when it returns, while {} ({}) lives on (GH #712).",
                    p, where_, holder_name, lives
                )),
                (Source::SelfField(f), Holder::Caller) => Some(format!(
                    "`{}` is a field of `self`, and {} ({}) may be kept after `self` is gone.",
                    f, holder_name, lives
                )),
                (Source::Param(p), Holder::SelfOwned) => {
                    let callee_fn = (ctx.locus.clone(), ctx.fn_name.clone());
                    match ctx.params.iter().position(|x| x == p) {
                        Some(j) if ctx.locus.is_some() => self.witness(&callee_fn, j, 0, &mut Vec::new()).map(|(site, w)| {
                            format!(
                                "`{}` is a parameter of {}, and {} ({}) outlives the call. At the call in `{}`, {}",
                                p, where_, holder_name, lives, site.in_fn, w
                            )
                        }),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(why) = why {
                let call = format!("{}.{}", locus_display(&recv_ty, renames), method.name);
                self.diags.push(Diag::ty(
                    arg.span(),
                    format!(
                        "`{}` keeps this argument, so {} holds it as a borrow, and a borrow must \
                         outlive its holder: {} A handle a method keeps is borrowed, never owned \
                         by the holder (F.39): make it a field of the locus that owns {}, or \
                         build and use {} in the frame that builds the argument (GH #730, #1048).",
                        call, holder_name, why, holder_name, holder_name
                    ),
                ));
            }
        }
    }

    fn block(&mut self, b: &Block) {
        let mark = self.ctx.as_ref().map(|c| c.locals.len()).unwrap_or(0);
        let binds_mark = self.ctx.as_ref().map(|c| c.binds.len()).unwrap_or(0);
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
            c.binds.truncate(binds_mark);
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
                // the locus it holds: a literal's, a factory's declared
                // return, or the one an aliased binding holds
                let ty = match value {
                    Expr::Ident(id) => self.bind(&id.name).and_then(|b| b.ty.clone()),
                    other => self.world.kept.value_locus(other),
                };
                if let Some(c) = self.ctx.as_mut() {
                    c.locals.push((name.name.clone(), src));
                    let loop_depth = c.loop_depth;
                    c.binds.push(Bind { name: name.name.clone(), ty, key: decl_key(name), param: false, loop_depth });
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
                    c.loop_depth += 1;
                    let loop_depth = c.loop_depth;
                    c.binds.push(Bind { name: name.name.clone(), ty: None, key: None, param: false, loop_depth });
                }
                self.block(body);
                if let Some(c) = self.ctx.as_mut() {
                    c.locals.pop();
                    c.binds.pop();
                    c.loop_depth -= 1;
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, Position::Other);
                if let Some(c) = self.ctx.as_mut() {
                    c.loop_depth += 1;
                }
                self.block(body);
                if let Some(c) = self.ctx.as_mut() {
                    c.loop_depth -= 1;
                }
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
                self.kept_call(callee, args);
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
                binds: Vec::new(),
                returned: BTreeSet::new(),
                loop_depth: 0,
            };
            // a locus literal written as the argument is a temporary of
            // the calling frame, like a `let` of it
            let src = match self.world.kept.literal_locus(arg) {
                Some(k) => Source::Owned { name: format!("{} {{ … }}", locus_display(&k, &self.world.kept.renames)), depth: 0 },
                None => match source_of(arg, &ctx) {
                    Some(s) => s,
                    None => continue,
                },
            };
            match strip_alias(&src) {
                Source::Owned { name, .. } => {
                    return Some((
                        site.clone(),
                        format!(
                            "the argument `{}` is {} of that frame, reclaimed when it returns.",
                            name,
                            if name.ends_with("{ … }") { "a temporary" } else { "a `let`" }
                        ),
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
                        binds: Vec::new(),
                        returned: BTreeSet::new(),
                        loop_depth: 0,
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
                            binds: Vec::new(),
                            returned: BTreeSet::new(),
                            loop_depth: 0,
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
