//! A revealed secret is consumed in its statement (spec/semantics.md
//! § "@sealed").
//!
//! `std::secret::Credential` is `@sealed`, but `reveal()` and
//! `reveal_text()` hand the material out in the open, as a `String` or
//! `Bytes`. This pass holds every reveal to the one use it exists for:
//!
//! - it is called in a **locus method**, never a free fn;
//! - its value reaches a **consumer in the same statement**: a wire
//!   write (`std::io::tcp::send_fd`, `std::io::tls::send_bytes`,
//!   `std::http::post`/`request`, `Stream.send`, `Client.post`, …), a
//!   comparison (`==`, `!=`, `Credential.matches`), or a parameter
//!   declared `@secret`;
//! - on the way it may pass only through composition that cannot keep
//!   it: `+`, an `if`/`match` arm, a record, tuple or array literal
//!   handed on, and a call to a stdlib fn the registry classes `PURE` or
//!   to a free fn this pass proves transparent (no method call, no locus
//!   built, no bus send, only such calls itself) — its result carries
//!   the secret on;
//! - it is never bound to a `let`, stored in a field or a locus
//!   literal, returned, raised, published, printed, iterated or handed
//!   to `std::process`.
//!
//! A `@secret` parameter is a declared consumer, so the same rule holds
//! inside its fn: the parameter may reach only a wire write, a
//! comparison, or another `@secret` parameter, through the same
//! composition. A one-way derivation (a hash, an HMAC, PBKDF2) is NOT a
//! consumer: a key derived from a password is password-equivalent — it
//! authenticates exactly as the password does — so a derivation carries
//! the secret on like any other composition.
//!
//! Four sites wait on one change, `pq` taking a `Credential` (the
//! Postgres DSN and its SCRAM exchange): they are allowed by qualified
//! name, each with a warning naming that deferral. A fifth is refused.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    BinOp, Block, ElseBranch, Expr, IfStmt, LocusMember, MatchArmBody, MatchStmt, OrDisposition, Param,
    Program, Stmt, TopDecl, TypeExpr,
};
use hale_syntax::error::Diag;
use hale_syntax::Span;

use crate::stdlib_surface::{effects_for, EffectSet};

/// The sites allowed by name until `pq` takes a `std::secret::Credential`:
/// `(seed directory, file stem, locus or "" for a free fn, fn, qualified
/// name)`.
const PQ_DEFERRED: &[(&str, &str, &str, &str, &str)] = &[
    ("dna/core", "memory_schema", "", "role_password", "dna::role_password"),
    ("dna/host", "infra", "ReferenceInfrastructure", "knowledge_database", "dna::ReferenceInfrastructure.knowledge_database"),
    ("dna/core/pond/pq", "scram", "", "salted_password", "pq::salted_password"),
    ("dna/core/pond/pq", "scram", "", "compute_client_final", "pq::compute_client_final"),
];

const DEFERRED_LINE: &str = "Deferred: `pq` takes a `std::secret::Credential`";

/// Stdlib free fns that put their arguments on the wire.
const WIRE_FNS: &[&[&str]] = &[
    &["std", "io", "tcp", "send_fd"],
    &["std", "io", "tcp", "__send"],
    &["std", "io", "tcp", "__send_bytes"],
    &["std", "io", "tls", "send_bytes"],
    &["std", "io", "udp", "send"],
    &["std", "http", "post"],
    &["std", "http", "request"],
];

/// Stdlib locus methods that put their arguments on the wire, or compare
/// them: `(mangled locus, method)`.
const WIRE_METHODS: &[(&str, &str)] = &[
    ("__StdIoTcpStream", "send"),
    ("__StdIoTcpStream", "send_bytes"),
    ("__StdHttpClient", "post"),
    ("__StdHttpClient", "request"),
    ("__StdSecretCredential", "matches"),
];

const CREDENTIAL: &str = "__StdSecretCredential";

/// What the value of the expression being walked turns into.
#[derive(Clone, Debug)]
enum Flow {
    /// It reaches a consumer: a wire write, a comparison, a `@secret`
    /// parameter.
    Consumed,
    /// It ends somewhere a secret may not go, said as a clause
    /// ("is bound to `token`").
    Leaks(String),
}

struct FnInfo {
    /// each param: its name, and whether it is `@secret`
    params: Vec<(String, bool)>,
}

#[derive(Default)]
struct World {
    /// free fn (declared name) -> params
    fns: BTreeMap<String, FnInfo>,
    /// locus -> method -> params
    methods: BTreeMap<String, BTreeMap<String, FnInfo>>,
    /// locus -> param field -> its type's key
    fields: BTreeMap<String, BTreeMap<String, String>>,
    loci: BTreeSet<String>,
    /// free fn -> the locus it returns
    fn_returns: BTreeMap<String, String>,
    /// free fns proven transparent
    transparent: BTreeSet<String>,
    renames: Vec<(Vec<String>, String)>,
}

impl World {
    /// A key as the user spells it: a stdlib locus by its public path, an
    /// imported seed's by `alias::Name`.
    fn display(&self, key: &str) -> String {
        if let Some((path, _)) = hale_stdlib::PATH_RENAMES.iter().find(|(_, m)| *m == key) {
            return path.join("::");
        }
        if let Some((path, _)) = self.renames.iter().find(|(_, m)| m == key) {
            return path.join("::");
        }
        key.to_string()
    }

    fn key_of_segs(&self, segs: &[&str]) -> Option<String> {
        if segs.len() > 1 {
            if let Some(m) = crate::stdlib_bodies::mangled_locus_name(segs) {
                return Some(m.to_string());
            }
            if let Some((_, m)) = self
                .renames
                .iter()
                .find(|(p, _)| p.len() == segs.len() && p.iter().zip(segs).all(|(a, b)| a == b))
            {
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

    /// A called free fn's declared name: `f(…)`, `alias::f(…)`.
    fn fn_name(&self, callee: &Expr) -> Option<String> {
        match callee {
            Expr::Ident(id) => Some(id.name.clone()),
            Expr::Path(qn) => {
                let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                if segs.first() == Some(&"std") {
                    return None;
                }
                if segs.len() == 1 {
                    return Some(segs[0].to_string());
                }
                self.renames
                    .iter()
                    .find(|(p, _)| p.len() == segs.len() && p.iter().zip(&segs).all(|(a, b)| a == b))
                    .map(|(_, m)| m.clone())
            }
            _ => None,
        }
    }

    fn gather(programs: &[&Program], renames: &[(Vec<String>, String)]) -> World {
        let mut w = World { renames: renames.to_vec(), ..World::default() };
        fn declared(items: &[TopDecl], w: &mut World) {
            for item in items {
                match item {
                    TopDecl::Module(m) => declared(&m.items, w),
                    TopDecl::Locus(l) => {
                        w.loci.insert(l.name.name.clone());
                    }
                    _ => {}
                }
            }
        }
        for p in programs {
            declared(&p.items, &mut w);
        }
        // the stdlib's loci are keys too (`std::http::Client` ->
        // `__StdHttpClient`); its records (`ClientRequest`) are not
        if let Some(std_prog) = crate::stdlib_bodies::program() {
            declared(&std_prog.items, &mut w);
        }
        fn info(params: &[Param]) -> FnInfo {
            FnInfo { params: params.iter().map(|p| (p.name.name.clone(), p.secret)).collect() }
        }
        fn collect(items: &[TopDecl], w: &mut World) {
            for item in items {
                match item {
                    TopDecl::Module(m) => collect(&m.items, w),
                    TopDecl::Fn(fd) => {
                        w.fns.insert(fd.name.name.clone(), info(&fd.params));
                        if let Some(r) = fd.ret.as_ref().and_then(|t| w.key_of(t)).filter(|t| w.loci.contains(t)) {
                            w.fn_returns.insert(fd.name.name.clone(), r);
                        }
                    }
                    TopDecl::Locus(l) => {
                        let mut fields = BTreeMap::new();
                        let mut methods = BTreeMap::new();
                        for m in &l.members {
                            match m {
                                LocusMember::Params(pb) => {
                                    for pd in &pb.params {
                                        if let Some(k) = pd.ty.as_ref().and_then(|t| w.key_of(t)) {
                                            fields.insert(pd.name.name.clone(), k);
                                        }
                                    }
                                }
                                LocusMember::Fn(fd) => {
                                    methods.insert(fd.name.name.clone(), info(&fd.params));
                                }
                                _ => {}
                            }
                        }
                        w.fields.insert(l.name.name.clone(), fields);
                        w.methods.insert(l.name.name.clone(), methods);
                    }
                    _ => {}
                }
            }
        }
        for p in programs {
            collect(&p.items, &mut w);
        }
        // transparency, to a fixpoint: a free fn whose every call is to
        // a pure stdlib fn or another transparent fn, that calls no
        // method, builds no locus and sends nothing
        let mut bodies: Vec<(String, &Block)> = Vec::new();
        fn free_bodies<'p>(items: &'p [TopDecl], out: &mut Vec<(String, &'p Block)>) {
            for item in items {
                match item {
                    TopDecl::Module(m) => free_bodies(&m.items, out),
                    TopDecl::Fn(fd) => out.push((fd.name.name.clone(), &fd.body)),
                    _ => {}
                }
            }
        }
        for p in programs {
            free_bodies(&p.items, &mut bodies);
        }
        loop {
            let mut grew = false;
            for (name, body) in &bodies {
                if !w.transparent.contains(name) && w.block_transparent(body) {
                    w.transparent.insert(name.clone());
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        w
    }

    fn call_transparent(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Path(qn) if qn.segments.first().map(|s| s.name == "std").unwrap_or(false) => {
                let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                segs.get(1) != Some(&"process") && effects_for(&segs) == Some(EffectSet::PURE)
            }
            Expr::Ident(id) if !self.fns.contains_key(&id.name) => {
                // a builtin: pure unless the registry classes it (println)
                crate::stdlib_surface::builtin_effects(&id.name).is_none()
            }
            other => self.fn_name(other).map(|n| self.transparent.contains(&n)).unwrap_or(false),
        }
    }

    fn block_transparent(&self, b: &Block) -> bool {
        b.stmts.iter().all(|s| self.stmt_transparent(s)) && b.tail.as_deref().map(|t| self.expr_transparent(t)).unwrap_or(true)
    }

    fn stmt_transparent(&self, s: &Stmt) -> bool {
        match s {
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Expr(value) => self.expr_transparent(value),
            Stmt::Assign { target, value, .. } => target.head.name != "self" && self.expr_transparent(value),
            Stmt::Return(e, _) => e.as_ref().map(|e| self.expr_transparent(e)).unwrap_or(true),
            Stmt::If(i) => self.if_transparent(i),
            Stmt::Match(m) => self.match_transparent(m),
            Stmt::For { iter, body, .. } => self.expr_transparent(iter) && self.block_transparent(body),
            Stmt::While { cond, body, .. } => self.expr_transparent(cond) && self.block_transparent(body),
            Stmt::Block(b) => self.block_transparent(b),
            Stmt::Break(_) | Stmt::Continue(_) => true,
            _ => false,
        }
    }

    fn if_transparent(&self, i: &IfStmt) -> bool {
        self.expr_transparent(&i.cond)
            && self.block_transparent(&i.then_block)
            && match i.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => self.block_transparent(b),
                Some(ElseBranch::ElseIf(n)) => self.if_transparent(n),
                None => true,
            }
    }

    fn match_transparent(&self, m: &MatchStmt) -> bool {
        self.expr_transparent(&m.scrutinee)
            && m.arms.iter().all(|a| {
                a.guard.as_ref().map(|g| self.expr_transparent(g)).unwrap_or(true)
                    && match &a.body {
                        MatchArmBody::Expr(e) => self.expr_transparent(e),
                        MatchArmBody::Block(b) => self.block_transparent(b),
                    }
            })
    }

    fn expr_transparent(&self, e: &Expr) -> bool {
        match e {
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) => true,
            Expr::KwSelf(_) => false,
            Expr::Binary { left, right, .. } => self.expr_transparent(left) && self.expr_transparent(right),
            Expr::Unary { operand, .. } => self.expr_transparent(operand),
            Expr::Call { callee, args, .. } => {
                !matches!(callee.as_ref(), Expr::Field { .. } | Expr::Path2 { .. })
                    && self.call_transparent(callee)
                    && args.iter().all(|a| self.expr_transparent(a))
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr_transparent(receiver),
            Expr::Index { receiver, index, .. } => self.expr_transparent(receiver) && self.expr_transparent(index),
            Expr::Tuple(v, _) | Expr::Array(v, _) => v.iter().all(|x| self.expr_transparent(x)),
            Expr::Struct { path, inits, .. } => {
                let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                let is_locus = self.key_of_segs(&segs).map(|k| self.loci.contains(&k)).unwrap_or(false);
                !is_locus && inits.iter().all(|i| self.expr_transparent(&i.value))
            }
            Expr::Block(b) => self.block_transparent(b),
            Expr::If(i) => self.if_transparent(i),
            Expr::Match(m) => self.match_transparent(m),
            Expr::Sum(x, _) | Expr::Prod(x, _) => self.expr_transparent(x),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr_transparent(left) && self.expr_transparent(right) && self.expr_transparent(tolerance)
            }
            Expr::Range { lo, hi, .. } => self.expr_transparent(lo) && self.expr_transparent(hi),
            Expr::ArrayRepeat { val, .. } => self.expr_transparent(val),
            Expr::Or { inner, disposition, .. } => {
                self.expr_transparent(inner)
                    && match disposition {
                        OrDisposition::Substitute(x) | OrDisposition::Fail(x, _) => self.expr_transparent(x),
                        _ => true,
                    }
            }
        }
    }
}

/// The reveal rule over `programs` (the user's seeds, keyed by source
/// path), resolving imported seeds through `renames`.
pub fn secret_reveal_diags(programs: &BTreeMap<String, &Program>, renames: &[(Vec<String>, String)]) -> Vec<Diag> {
    let list: Vec<&Program> = programs.values().copied().collect();
    let world = World::gather(&list, renames);
    let mut diags = Vec::new();
    for (path, p) in programs {
        walk_items(&world, path, &p.items, &mut diags);
    }
    diags
}

fn walk_items(world: &World, path: &str, items: &[TopDecl], diags: &mut Vec<Diag>) {
    for item in items {
        match item {
            TopDecl::Module(m) => walk_items(world, path, &m.items, diags),
            TopDecl::Fn(fd) => {
                let mut b = Body::new(world, path, None, &fd.name.name, &fd.params, diags);
                b.block(&fd.body, &Flow::Leaks("is returned".to_string()));
                b.finish();
            }
            TopDecl::Locus(l) => {
                for m in &l.members {
                    let (name, params, body): (String, &[Param], &Block) = match m {
                        LocusMember::Fn(fd) => (fd.name.name.clone(), &fd.params, &fd.body),
                        LocusMember::Lifecycle(lc) => (format!("{:?}", lc.kind).to_lowercase(), &lc.params, &lc.body),
                        LocusMember::Mode(md) => ("<mode>".to_string(), &md.params, &md.body),
                        _ => continue,
                    };
                    let mut b = Body::new(world, path, Some(&l.name.name), &name, params, diags);
                    b.block(body, &Flow::Leaks("is returned".to_string()));
                    b.finish();
                }
            }
            _ => {}
        }
    }
}

/// One fn or method body.
struct Body<'a> {
    world: &'a World,
    locus: Option<&'a str>,
    fn_name: String,
    /// its `@secret` params: sources in this body, while no inner
    /// binding shadows them
    secret_params: BTreeSet<String>,
    /// how many of `types`' first entries are the params
    n_params: usize,
    /// name -> the locus a binding holds, innermost last
    types: Vec<(String, Option<String>)>,
    /// the qualified name this body is allowed by, until pq takes a
    /// Credential
    deferred: Option<&'static str>,
    found: Vec<(Span, String)>,
    diags: &'a mut Vec<Diag>,
}

impl<'a> Body<'a> {
    fn new(
        world: &'a World,
        path: &str,
        locus: Option<&'a str>,
        fn_name: &str,
        params: &[Param],
        diags: &'a mut Vec<Diag>,
    ) -> Body<'a> {
        let deferred = PQ_DEFERRED
            .iter()
            .find(|(seed, stem, l, f, _)| {
                // Checked as its own seed, the seed's files are one program
                // keyed by the seed directory (or by the one file checked),
                // and the names are as written. Imported, the seed is merged
                // into the importer's program and its top-level names arrive
                // mangled with the file's stem
                // (`__lib_dna_core_memory_schema_role_password`).
                let own = path.ends_with(seed) || path.ends_with(&format!("{}/{}.hl", seed, stem));
                let named = |have: &str, want: &str| {
                    have == want && own || have.starts_with("__lib_") && have.ends_with(&format!("_{}_{}", stem, want))
                };
                match locus {
                    // a method's name is never mangled: its locus pins it
                    Some(have) => !l.is_empty() && named(have, l) && fn_name == *f,
                    None => l.is_empty() && named(fn_name, f),
                }
            })
            .map(|(_, _, _, _, q)| *q);
        let types = params
            .iter()
            .map(|p| (p.name.name.clone(), world.key_of(&p.ty).filter(|k| world.loci.contains(k))))
            .collect();
        Body {
            world,
            locus,
            fn_name: fn_name.to_string(),
            secret_params: params.iter().filter(|p| p.secret).map(|p| p.name.name.clone()).collect(),
            n_params: params.len(),
            types,
            deferred,
            found: Vec::new(),
            diags,
        }
    }

    fn where_(&self) -> String {
        match self.locus {
            Some(l) => format!("`{}.{}`", l, self.fn_name),
            None => format!("`{}`", self.fn_name),
        }
    }

    fn finish(self) {
        let Body { found, deferred, diags, .. } = self;
        for (span, msg) in found {
            match deferred {
                Some(q) => diags.push(Diag::warn(
                    span,
                    format!("{} It is allowed here by name, in `{}`, until `pq` takes a `std::secret::Credential` ({}).", msg, q, DEFERRED_LINE),
                )),
                None => diags.push(Diag::ty(span, msg)),
            }
        }
    }

    fn type_of_name(&self, name: &str) -> Option<String> {
        self.types.iter().rev().find(|(n, _)| n == name).and_then(|(_, t)| t.clone())
    }

    /// The locus a receiver is, when position says.
    fn receiver_type(&self, recv: &Expr) -> Option<String> {
        match recv {
            Expr::KwSelf(_) => self.locus.map(str::to_string),
            Expr::Ident(id) => self.type_of_name(&id.name),
            Expr::Struct { path, .. } => {
                let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                self.world.key_of_segs(&segs).filter(|k| self.world.loci.contains(k))
            }
            Expr::Field { receiver, name, .. } => {
                let owner = self.receiver_type(receiver)?;
                self.world.fields.get(&owner)?.get(&name.name).cloned()
            }
            _ => None,
        }
    }

    fn value_type(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Struct { .. } | Expr::Ident(_) | Expr::Field { .. } => self.receiver_type(e),
            Expr::Call { callee, .. } => self.world.fn_name(callee).and_then(|n| self.world.fn_returns.get(&n).cloned()),
            _ => None,
        }
    }

    /// A secret that ends at `flow`: refused unless consumed.
    fn source(&mut self, span: Span, what: &str, flow: &Flow) {
        if let Flow::Leaks(clause) = flow {
            self.found.push((
                span,
                format!(
                    "{} must be consumed in the statement that {}, by a wire write, a comparison or a \
                     `@secret` parameter: here it {}. A revealed secret never outlives the statement \
                     that sends it (spec/semantics.md § \"@sealed\").",
                    what,
                    if what.starts_with("the `@secret`") { "names it" } else { "reveals it" },
                    clause
                ),
            ));
        }
    }

    fn block(&mut self, b: &Block, tail_flow: &Flow) {
        let mark = self.types.len();
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = b.tail.as_deref() {
            self.expr(t, tail_flow);
        }
        self.types.truncate(mark);
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                self.expr(value, &Flow::Leaks(format!("is bound to `{}`", name.name)));
                let t = match value {
                    Expr::Ident(id) => self.type_of_name(&id.name),
                    other => self.value_type(other),
                };
                self.types.push((name.name.clone(), t));
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value, &Flow::Leaks("is bound by a tuple `let`".to_string()));
                for n in names {
                    self.types.push((n.name.clone(), None));
                }
            }
            Stmt::Assign { target, value, .. } => {
                let place = if target.tail.is_empty() {
                    format!("is assigned to `{}`", target.head.name)
                } else if target.head.name == "self" {
                    "is stored in a field of `self`".to_string()
                } else {
                    format!("is stored under `{}`", target.head.name)
                };
                for seg in &target.tail {
                    if let hale_syntax::ast::LValueSeg::Index(ix) = seg {
                        self.expr(ix, &Flow::Leaks("is used as an index".to_string()));
                    }
                }
                self.expr(value, &Flow::Leaks(place));
            }
            Stmt::Return(Some(e), _) => self.expr(e, &Flow::Leaks("is returned".to_string())),
            Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) => {}
            Stmt::Fail { value, .. } => self.expr(value, &Flow::Leaks("is raised as a failure".to_string())),
            Stmt::If(i) => self.if_chain(i, &Flow::Leaks("is not consumed".to_string())),
            Stmt::Match(m) => self.match_arms(m, &Flow::Leaks("is not consumed".to_string())),
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter, &Flow::Leaks("is iterated".to_string()));
                self.types.push((name.name.clone(), None));
                self.block(body, &Flow::Leaks("is not consumed".to_string()));
                self.types.pop();
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, &Flow::Consumed);
                self.block(body, &Flow::Leaks("is not consumed".to_string()));
            }
            Stmt::Block(b) => self.block(b, &Flow::Leaks("is not consumed".to_string())),
            Stmt::Send { subject, value, .. } => {
                self.expr(subject, &Flow::Leaks("is a bus subject".to_string()));
                self.expr(value, &Flow::Leaks("is published on the bus".to_string()));
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max, &Flow::Leaks("is a shared-memory size".to_string()));
                self.block(body, &Flow::Leaks("is not consumed".to_string()));
            }
            Stmt::Recovery { args, .. } => {
                for a in args {
                    self.expr(a, &Flow::Leaks("is a recovery argument".to_string()));
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.expr(p, &Flow::Leaks("is a violation payload".to_string()));
                }
            }
            Stmt::Reperspective { .. } => {}
            Stmt::Expr(e) => self.expr(e, &Flow::Leaks("is not consumed".to_string())),
        }
    }

    fn if_chain(&mut self, i: &IfStmt, flow: &Flow) {
        self.expr(&i.cond, &Flow::Consumed);
        self.block(&i.then_block, flow);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b, flow),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n, flow),
            None => {}
        }
    }

    fn match_arms(&mut self, m: &MatchStmt, flow: &Flow) {
        // matching on a secret compares it
        self.expr(&m.scrutinee, &Flow::Consumed);
        for a in &m.arms {
            if let Some(g) = &a.guard {
                self.expr(g, &Flow::Consumed);
            }
            match &a.body {
                MatchArmBody::Expr(e) => self.expr(e, flow),
                MatchArmBody::Block(b) => self.block(b, flow),
            }
        }
    }

    /// The flow of argument `i` of a call.
    fn arg_flow(&self, callee: &Expr, i: usize, result_flow: &Flow) -> Flow {
        match callee {
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                let method = name.name.as_str();
                let Some(t) = self.receiver_type(receiver) else {
                    return Flow::Leaks(format!(
                        "reaches `.{}(…)` on a value whose type the check cannot see",
                        method
                    ));
                };
                if WIRE_METHODS.iter().any(|(l, m)| *l == t && *m == method) {
                    return Flow::Consumed;
                }
                let secret = self
                    .world
                    .methods
                    .get(&t)
                    .and_then(|ms| ms.get(method))
                    .and_then(|f| f.params.get(i))
                    .map(|(_, s)| *s);
                match secret {
                    Some(true) => Flow::Consumed,
                    _ => Flow::Leaks(format!(
                        "reaches `{}.{}`, which is not a wire write, a comparison or a `@secret` parameter",
                        self.world.display(&t),
                        method
                    )),
                }
            }
            Expr::Path(qn) if qn.segments.first().map(|s| s.name == "std").unwrap_or(false) => {
                let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                let spelled = segs.join("::");
                if segs.get(1) == Some(&"process") {
                    return Flow::Leaks(format!("reaches `{}`", spelled));
                }
                if WIRE_FNS.iter().any(|w| *w == segs.as_slice()) {
                    return Flow::Consumed;
                }
                if effects_for(&segs) == Some(EffectSet::PURE) {
                    // composition: the result carries it on
                    return result_flow.clone();
                }
                Flow::Leaks(format!(
                    "reaches `{}`, which is not a wire write, a comparison or a `@secret` parameter",
                    spelled
                ))
            }
            other => {
                let Some(n) = self.world.fn_name(other) else {
                    return Flow::Leaks("reaches a call the check cannot follow".to_string());
                };
                if let Some(f) = self.world.fns.get(&n) {
                    if f.params.get(i).map(|(_, s)| *s).unwrap_or(false) {
                        return Flow::Consumed;
                    }
                    if self.world.transparent.contains(&n) {
                        return result_flow.clone();
                    }
                    return Flow::Leaks(format!(
                        "reaches `{}`, which is not a wire write, a comparison or a `@secret` parameter",
                        self.world.display(&n)
                    ));
                }
                // a builtin
                match crate::stdlib_surface::builtin_effects(&n) {
                    Some(_) => Flow::Leaks(format!("is printed (`{}`)", n)),
                    None => result_flow.clone(),
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr, flow: &Flow) {
        match e {
            Expr::Ident(id) => {
                let is_param = self
                    .types
                    .iter()
                    .rposition(|(n, _)| *n == id.name)
                    .map(|i| i < self.n_params)
                    .unwrap_or(false);
                if self.secret_params.contains(&id.name) && is_param {
                    let what = format!("the `@secret` parameter `{}` of {}", id.name, self.where_());
                    self.source(id.span, &what, flow);
                }
            }
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
            Expr::Binary { op, left, right, .. } => {
                let f = if matches!(op, BinOp::Eq | BinOp::NotEq) { Flow::Consumed } else { flow.clone() };
                self.expr(left, &f);
                self.expr(right, &f);
            }
            Expr::Unary { operand, .. } => self.expr(operand, flow),
            Expr::Call { callee, args, span, .. } => {
                // a reveal is a source
                if let Expr::Field { receiver, name, .. } = callee.as_ref() {
                    if name.name == "reveal" || name.name == "reveal_text" {
                        let t = self.receiver_type(receiver);
                        let is_credential = t.as_deref().map(|t| t == CREDENTIAL).unwrap_or(true);
                        // inside `Credential` itself, `self.reveal()` is the
                        // sealed side calling itself (`reveal_text`), not a
                        // reveal to the application
                        let sealed_side = matches!(receiver.as_ref(), Expr::KwSelf(_)) && self.locus == Some(CREDENTIAL);
                        if is_credential && !sealed_side {
                            let what = format!("`Credential.{}()`", name.name);
                            if self.locus.is_none() {
                                self.found.push((
                                    *span,
                                    format!(
                                        "{} may be called only in a locus method, and {} is a free fn: a revealed \
                                         secret is consumed in the statement that reveals it, by a wire write, a \
                                         comparison or a `@secret` parameter (spec/semantics.md § \"@sealed\").",
                                        what,
                                        self.where_()
                                    ),
                                ));
                            } else {
                                self.source(*span, &what, flow);
                            }
                        }
                    }
                }
                // the receiver: a value a method is called on
                if let Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } = callee.as_ref() {
                    self.expr(receiver, &Flow::Leaks(format!("has `.{}(…)` called on it", name.name)));
                }
                for (i, a) in args.iter().enumerate() {
                    let f = self.arg_flow(callee, i, flow);
                    self.expr(a, &f);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver, flow),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver, flow);
                self.expr(index, &Flow::Leaks("is used as an index".to_string()));
            }
            Expr::Tuple(v, _) | Expr::Array(v, _) => {
                for x in v {
                    self.expr(x, flow);
                }
            }
            Expr::Struct { path, inits, .. } => {
                let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                let locus = self.world.key_of_segs(&segs).filter(|k| self.world.loci.contains(k));
                let f = match locus {
                    Some(l) => Flow::Leaks(format!("is stored in a `{} {{ … }}`", self.world.display(&l))),
                    None => flow.clone(),
                };
                for i in inits {
                    self.expr(&i.value, &f);
                }
            }
            Expr::Block(b) => self.block(b, flow),
            Expr::If(i) => self.if_chain(i, flow),
            Expr::Match(m) => self.match_arms(m, flow),
            Expr::Sum(x, _) | Expr::Prod(x, _) | Expr::ArrayRepeat { val: x, .. } => self.expr(x, flow),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left, flow);
                self.expr(right, flow);
                self.expr(tolerance, flow);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo, flow);
                self.expr(hi, flow);
            }
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner, flow);
                match disposition {
                    OrDisposition::Substitute(x) => self.expr(x, flow),
                    OrDisposition::Fail(x, _) => self.expr(x, &Flow::Leaks("is raised as a failure".to_string())),
                    OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
                }
            }
        }
    }
}
