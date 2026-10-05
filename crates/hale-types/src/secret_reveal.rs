//! A revealed secret is consumed in its statement (spec/semantics.md
//! § "@sealed").
//!
//! `std::secret::Credential` is `@sealed`, but `reveal()` and
//! `reveal_text()` hand the material out in the open, as a `String` or
//! `Bytes`. This pass holds every reveal to the one use it exists for:
//!
//! - it is called in a **locus method**, never a free fn;
//! - its value reaches a **consumer in the same statement**: the payload
//!   or header argument of a wire write (`std::io::tcp::send_fd`,
//!   `std::http::post`, a `ClientRequest { … }` literal's `headers` and
//!   `body`, `Stream.send`, …), a **comparison of the whole value**
//!   (`==`, `!=`, a `match` whose patterns bind nothing,
//!   `Credential.matches`), or a parameter declared `@secret`;
//! - on the way to a wire write it may pass only through composition
//!   that cannot keep it: `+`, an `if`/`match` arm, a record, tuple or
//!   array literal handed on, a call to a stdlib fn named in
//!   [`VALUE_FNS`], a pure builtin, or a free fn this pass proves
//!   transparent — its result carries the secret on. A comparison takes
//!   the value whole: taken apart or combined first, it answers a
//!   question about part of it, and is refused;
//! - it is never bound to a `let` or a pattern, stored in a field, a
//!   param default or a locus literal, returned, raised, published,
//!   printed, iterated, used to decide a branch, or handed to
//!   `std::process`.
//!
//! A `@secret` parameter is a declared consumer, so the same rule holds
//! inside its fn: the parameter may reach only a wire write, a
//! comparison, or another `@secret` parameter, through the same
//! composition. A one-way derivation (a hash, an HMAC, PBKDF2) is NOT a
//! consumer: a key derived from a password is password-equivalent — it
//! authenticates exactly as the password does — so a derivation carries
//! the secret on like any other composition.
//!
//! Anything the walk does not reach fails closed: a reveal in a place no
//! walk covers is refused, a receiver whose type the pass cannot see is
//! taken to be a `Credential`, and a call it cannot follow keeps what it
//! is given.
//!
//! Four sites wait on one change, `pq` taking a `Credential` (the
//! Postgres DSN and its SCRAM exchange): they are allowed by qualified
//! name AND by the body pinned here, each with a warning naming that
//! deferral. Any other site, or one of the four with its body changed, is
//! refused.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    BinOp, Block, ElseBranch, Expr, FnDecl, IfStmt, LocusMember, MatchArmBody, MatchStmt, OrDisposition, Param,
    ParamInit, Pattern, PerspectiveMember, Program, Stmt, TopDecl, TypeDeclBody, TypeExpr,
};
use hale_syntax::error::Diag;
use hale_syntax::Span;

/// The sites allowed until `pq` takes a `std::secret::Credential`:
/// `(file stem, locus or "" for a free fn, fn, qualified name, the
/// fingerprint of the fn's body, a companion free fn the same module
/// declares)`. The stem and the companion identify the module: a
/// program's own fn that happens to share a pinned name, in a file
/// that does not also declare the companion, is no pin. The fingerprint is [`fingerprint`]
/// of the declaration as the checker sees it; an edit to one of these
/// fns refuses it until the pin is updated here, so the exemption covers
/// the reviewed code and nothing added to it.
const PQ_DEFERRED: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("memory_schema", "", "role_password", "dna::role_password", "9c4445e325202333", "role_vault_name"),
    (
        "infra",
        "ReferenceInfrastructure",
        "knowledge_database",
        "dna::ReferenceInfrastructure.knowledge_database",
        "0aa957e955be225f",
        "transport_kind",
    ),
    ("scram", "", "salted_password", "pq::salted_password", "01506142ce37f56c", "compute_client_final"),
    ("scram", "", "compute_client_final", "pq::compute_client_final", "23267b5a65b98ff0", "salted_password"),
];

const DEFERRED_LINE: &str = "Deferred: `pq` takes a `std::secret::Credential`";

/// Stdlib free fns that put arguments on the wire, and which of their
/// arguments are the payload. The others (a descriptor, a host, a URL)
/// are not the wire's: an error repeats them back.
const WIRE_FNS: &[(&[&str], &[usize])] = &[
    (&["std", "io", "tcp", "send_fd"], &[1]),
    (&["std", "io", "tcp", "__send"], &[1]),
    (&["std", "io", "tcp", "__send_bytes"], &[1]),
    (&["std", "io", "tls", "send_bytes"], &[1]),
    (&["std", "io", "udp", "send"], &[3]),
    // (url, body, content type): the body and a header
    (&["std", "http", "post"], &[1, 2]),
];

/// Stdlib locus methods that put arguments on the wire: `(mangled locus,
/// method, payload arguments)`.
const WIRE_METHODS: &[(&str, &str, &[usize])] = &[
    ("__StdIoTcpStream", "send", &[0]),
    ("__StdIoTcpStream", "send_bytes", &[0]),
    ("__StdHttpClient", "post", &[1, 2]),
];

/// Calls that take one `ClientRequest`: only its `headers` and `body`
/// are the wire's, so the request must be a literal the pass can read.
const REQUEST_FNS: &[&[&str]] = &[&["std", "http", "request"]];
const REQUEST_METHODS: &[(&str, &str)] = &[("__StdHttpClient", "request")];
const REQUEST_WIRE_FIELDS: &[&str] = &["headers", "body"];

/// Methods that compare their argument with a sealed value.
const COMPARE_METHODS: &[(&str, &str)] = &[(CREDENTIAL, "matches")];

/// Stdlib fns that compute a value from their arguments and keep none of
/// them: a secret passes through one and its result carries it on. A
/// positive list, because a `PURE` effect class says nothing of an
/// out-parameter (`std::str::builder_append` is pure and keeps what it is
/// handed).
const VALUE_FNS: &[&[&str]] = &[
    &["std", "bytes", "from_string"],
    &["std", "bytes", "to_string"],
    &["std", "json", "escape_string"],
    &["std", "str", "trim"],
    &["std", "str", "index_of"],
    &["std", "str", "from_bytes"],
    &["std", "str", "builder_new"],
    &["std", "str", "builder_finish"],
    &["std", "bytes", "at"],
    &["std", "bytes", "slice"],
    &["std", "str", "lower"],
    &["std", "str", "upper"],
    &["std", "encoding", "base64", "encode"],
    &["std", "encoding", "base64", "encode_url"],
    &["std", "encoding", "hex", "encode"],
    &["std", "crypto", "sha256"],
    &["std", "crypto", "hmac_sha256"],
];

/// Stdlib fns that write into one argument, `(fn, that argument)`: a
/// transparent fn may call one only on a builder it made itself.
const LOCAL_OUT_FNS: &[(&[&str], usize)] = &[(&["std", "str", "builder_append"], 0)];

/// Builtins that compute a value and keep nothing.
const VALUE_BUILTINS: &[&str] = &["len", "to_string"];

const CREDENTIAL: &str = "__StdSecretCredential";

const TAKEN_APART: &str = "is compared after it is taken apart or combined, which answers a question about \
                           part of it: a comparison takes the whole revealed value (`Credential.matches` \
                           compares in constant time)";

/// What the value of the expression being walked turns into.
#[derive(Clone, Debug)]
enum Flow {
    /// It reaches a consumer: a wire write, a `@secret` parameter.
    Consumed,
    /// It is compared whole: consumed if it arrives here as it is, a
    /// leak if it was taken apart or combined on the way.
    Compared,
    /// It ends somewhere a secret may not go, said as a clause
    /// ("is bound to `token`").
    Leaks(String),
}

impl Flow {
    fn leaks(clause: impl Into<String>) -> Flow {
        Flow::Leaks(clause.into())
    }

    /// The flow of an operand of composition whose result flows as
    /// `self`: a comparison no longer sees the whole value.
    fn composed(&self) -> Flow {
        match self {
            Flow::Compared => Flow::leaks(TAKEN_APART),
            f => f.clone(),
        }
    }
}

struct FnInfo {
    /// each param: its name, and whether it is `@secret`
    params: Vec<(String, bool)>,
    /// where it is declared: its program's key and its name's offset,
    /// so its file is known from the source map
    site: (String, u32),
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
    /// type alias -> its target's key
    aliases: BTreeMap<String, String>,
    /// free fn -> the locus it returns
    fn_returns: BTreeMap<String, String>,
    /// free fns proven transparent
    transparent: BTreeSet<String>,
    renames: Vec<(Vec<String>, String)>,
    /// the bundle's source map: (path, base, len), so a declaration's
    /// file is known from its span
    sources: Vec<(String, u32, u32)>,
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
        let mut key = segs.last().map(|s| s.to_string());
        if segs.len() > 1 {
            if let Some(m) = crate::stdlib_bodies::mangled_locus_name(segs) {
                key = Some(m.to_string());
            } else if let Some((_, m)) = self
                .renames
                .iter()
                .find(|(p, _)| p.len() == segs.len() && p.iter().zip(segs).all(|(a, b)| a == b))
            {
                key = Some(m.clone());
            }
        }
        // an alias is its target (`type Cred = std::secret::Credential;`)
        let mut hops = 0;
        while let Some(t) = key.as_ref().and_then(|k| self.aliases.get(k)) {
            key = Some(t.clone());
            hops += 1;
            if hops > 16 {
                break;
            }
        }
        key
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

    /// The source unit a bundle-global offset falls in, when the bundle
    /// has a source map.
    fn file_of(&self, at: u32) -> Option<&str> {
        self.sources
            .iter()
            .find(|(_, base, len)| at >= *base && at < base.saturating_add(*len))
            .map(|(path, _, _)| path.as_str())
    }

    fn gather(programs: &[(&str, &Program)], renames: &[(Vec<String>, String)]) -> World {
        let mut w = World { renames: renames.to_vec(), ..World::default() };
        fn declared(items: &[TopDecl], w: &mut World, aliases: &mut Vec<(String, TypeExpr)>) {
            for item in items {
                match item {
                    TopDecl::Module(m) => declared(&m.items, w, aliases),
                    TopDecl::Locus(l) => {
                        w.loci.insert(l.name.name.clone());
                        for m in &l.members {
                            if let LocusMember::Type(td) = m {
                                if let TypeDeclBody::Alias(t) = &td.body {
                                    aliases.push((td.name.name.clone(), t.clone()));
                                }
                            }
                        }
                    }
                    TopDecl::Type(td) => {
                        if let TypeDeclBody::Alias(t) = &td.body {
                            aliases.push((td.name.name.clone(), t.clone()));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut aliases = Vec::new();
        for (_, p) in programs {
            declared(&p.items, &mut w, &mut aliases);
        }
        // the stdlib's loci are keys too (`std::http::Client` ->
        // `__StdHttpClient`); its records (`ClientRequest`) are not
        if let Some(std_prog) = crate::stdlib_bodies::program() {
            declared(&std_prog.items, &mut w, &mut Vec::new());
        }
        for (name, t) in &aliases {
            if let Some(k) = w.key_of(t) {
                if k != *name {
                    w.aliases.insert(name.clone(), k);
                }
            }
        }
        fn info(key: &str, fd: &FnDecl) -> FnInfo {
            FnInfo {
                params: fd.params.iter().map(|p| (p.name.name.clone(), p.secret)).collect(),
                site: (key.to_string(), fd.name.span.start.0),
            }
        }
        fn collect(key: &str, items: &[TopDecl], w: &mut World) {
            for item in items {
                match item {
                    TopDecl::Module(m) => collect(key, &m.items, w),
                    TopDecl::Fn(fd) => {
                        w.fns.insert(fd.name.name.clone(), info(key, fd));
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
                                    methods.insert(fd.name.name.clone(), info(key, fd));
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
        for (key, p) in programs {
            collect(key, &p.items, &mut w);
        }
        // transparency, to a fixpoint: a free fn whose every call is to
        // a value fn, a value builtin or another transparent fn, that
        // calls no method, builds no locus, sends nothing and assigns
        // only its own locals
        let mut bodies: Vec<(String, &[Param], &Block)> = Vec::new();
        fn free_bodies<'p>(items: &'p [TopDecl], out: &mut Vec<(String, &'p [Param], &'p Block)>) {
            for item in items {
                match item {
                    TopDecl::Module(m) => free_bodies(&m.items, out),
                    TopDecl::Fn(fd) => out.push((fd.name.name.clone(), &fd.params, &fd.body)),
                    _ => {}
                }
            }
        }
        for (_, p) in programs {
            free_bodies(&p.items, &mut bodies);
        }
        loop {
            let mut grew = false;
            for (name, params, body) in &bodies {
                if w.transparent.contains(name) {
                    continue;
                }
                let mut locals: Vec<(String, bool)> = params.iter().map(|p| (p.name.name.clone(), true)).collect();
                if w.block_transparent(body, &mut locals) {
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

    /// Whether a call to `callee` keeps nothing of its arguments. A name
    /// bound in the fn (a param, a `let`) is a value the pass cannot see
    /// into, whatever it names.
    fn call_transparent(&self, callee: &Expr, args: &[Expr], locals: &[(String, bool)]) -> bool {
        match callee {
            Expr::Path(qn) if qn.segments.first().map(|s| s.name == "std").unwrap_or(false) => {
                let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                if let Some((_, out)) = LOCAL_OUT_FNS.iter().find(|(f, _)| *f == segs.as_slice()) {
                    // writes into its `out` argument: fine when that is a
                    // `let` of this fn, never a param the caller holds
                    return matches!(args.get(*out), Some(Expr::Ident(id))
                        if locals.iter().rev().find(|(n, _)| *n == id.name).map(|(_, param)| !param).unwrap_or(false));
                }
                is_value_fn(&segs)
            }
            Expr::Ident(id) if locals.iter().any(|(n, _)| *n == id.name) => false,
            Expr::Ident(id) if !self.fns.contains_key(&id.name) => VALUE_BUILTINS.contains(&id.name.as_str()),
            other => self.fn_name(other).map(|n| self.transparent.contains(&n)).unwrap_or(false),
        }
    }

    fn block_transparent(&self, b: &Block, locals: &mut Vec<(String, bool)>) -> bool {
        let mark = locals.len();
        let ok = b.stmts.iter().all(|s| self.stmt_transparent(s, locals))
            && b.tail.as_deref().map(|t| self.expr_transparent(t, locals)).unwrap_or(true);
        locals.truncate(mark);
        ok
    }

    fn stmt_transparent(&self, s: &Stmt, locals: &mut Vec<(String, bool)>) -> bool {
        match s {
            Stmt::Let { name, value, .. } => {
                let ok = self.expr_transparent(value, locals);
                locals.push((name.name.clone(), false));
                ok
            }
            Stmt::LetTuple { names, value, .. } => {
                let ok = self.expr_transparent(value, locals);
                locals.extend(names.iter().map(|n| (n.name.clone(), false)));
                ok
            }
            Stmt::Expr(value) => self.expr_transparent(value, locals),
            // only a local of its own: a field, an index or a param's
            // insides are somewhere the value outlives the call
            Stmt::Assign { target, value, .. } => {
                target.tail.is_empty() && target.head.name != "self" && self.expr_transparent(value, locals)
            }
            Stmt::Return(e, _) => e.as_ref().map(|e| self.expr_transparent(e, locals)).unwrap_or(true),
            Stmt::If(i) => self.if_transparent(i, locals),
            Stmt::Match(m) => self.match_transparent(m, locals),
            Stmt::For { name, iter, body, .. } => {
                if !self.expr_transparent(iter, locals) {
                    return false;
                }
                locals.push((name.name.clone(), false));
                let ok = self.block_transparent(body, locals);
                locals.pop();
                ok
            }
            Stmt::While { cond, body, .. } => self.expr_transparent(cond, locals) && self.block_transparent(body, locals),
            Stmt::Block(b) => self.block_transparent(b, locals),
            Stmt::Break(_) | Stmt::Continue(_) => true,
            _ => false,
        }
    }

    fn if_transparent(&self, i: &IfStmt, locals: &mut Vec<(String, bool)>) -> bool {
        self.expr_transparent(&i.cond, locals)
            && self.block_transparent(&i.then_block, locals)
            && match i.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => self.block_transparent(b, locals),
                Some(ElseBranch::ElseIf(n)) => self.if_transparent(n, locals),
                None => true,
            }
    }

    fn match_transparent(&self, m: &MatchStmt, locals: &mut Vec<(String, bool)>) -> bool {
        self.expr_transparent(&m.scrutinee, locals)
            && m.arms.iter().all(|a| {
                let mark = locals.len();
                let mut names = Vec::new();
                pattern_bindings(&a.pattern, &mut names);
                locals.extend(names.into_iter().map(|n| (n, false)));
                let ok = a.guard.as_ref().map(|g| self.expr_transparent(g, locals)).unwrap_or(true)
                    && match &a.body {
                        MatchArmBody::Expr(e) => self.expr_transparent(e, locals),
                        MatchArmBody::Block(b) => self.block_transparent(b, locals),
                    };
                locals.truncate(mark);
                ok
            })
    }

    fn expr_transparent(&self, e: &Expr, locals: &mut Vec<(String, bool)>) -> bool {
        match e {
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) => true,
            Expr::KwSelf(_) => false,
            Expr::Binary { left, right, .. } => self.expr_transparent(left, locals) && self.expr_transparent(right, locals),
            Expr::Unary { operand, .. } => self.expr_transparent(operand, locals),
            Expr::Call { callee, args, .. } => {
                !matches!(callee.as_ref(), Expr::Field { .. } | Expr::Path2 { .. })
                    && self.call_transparent(callee, args, locals)
                    && args.iter().all(|a| self.expr_transparent(a, locals))
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr_transparent(receiver, locals),
            Expr::Index { receiver, index, .. } => {
                self.expr_transparent(receiver, locals) && self.expr_transparent(index, locals)
            }
            Expr::Tuple(v, _) | Expr::Array(v, _) => v.iter().all(|x| self.expr_transparent(x, locals)),
            Expr::Struct { path, inits, .. } => {
                let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                let is_locus = self.key_of_segs(&segs).map(|k| self.loci.contains(&k)).unwrap_or(false);
                !is_locus && inits.iter().all(|i| self.expr_transparent(&i.value, locals))
            }
            Expr::Block(b) => self.block_transparent(b, locals),
            Expr::If(i) => self.if_transparent(i, locals),
            Expr::Match(m) => self.match_transparent(m, locals),
            Expr::Sum(x, _) | Expr::Prod(x, _) => self.expr_transparent(x, locals),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr_transparent(left, locals)
                    && self.expr_transparent(right, locals)
                    && self.expr_transparent(tolerance, locals)
            }
            Expr::Range { lo, hi, .. } => self.expr_transparent(lo, locals) && self.expr_transparent(hi, locals),
            Expr::ArrayRepeat { val, .. } => self.expr_transparent(val, locals),
            Expr::Or { inner, disposition, .. } => {
                self.expr_transparent(inner, locals)
                    && match disposition {
                        OrDisposition::Substitute(x) | OrDisposition::Fail(x, _) => self.expr_transparent(x, locals),
                        _ => true,
                    }
            }
        }
    }
}

fn is_value_fn(segs: &[&str]) -> bool {
    VALUE_FNS.iter().any(|v| *v == segs)
}

/// The names a pattern binds, pushed onto `out`.
fn pattern_bindings(p: &Pattern, out: &mut Vec<String>) {
    match p {
        Pattern::Binding(id) => out.push(id.name.clone()),
        Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
            for a in args {
                pattern_bindings(a, out);
            }
        }
        Pattern::Literal(..) | Pattern::Wildcard(_) => {}
    }
}

/// The reveal rule over `programs` (the user's seeds, keyed by source
/// path), resolving imported seeds through `renames`.
pub fn secret_reveal_diags(
    programs: &BTreeMap<String, &Program>,
    renames: &[(Vec<String>, String)],
    sources: &[crate::symbol::SourceFile],
) -> Vec<Diag> {
    secret_reveal_by_item(programs, renames, sources, &|_, _| None).into_values().flatten().flatten().collect()
}

/// The reveal rule per top-level declaration: each program's items' own
/// diagnostics, in order, the world gathered over every program. The walk
/// of one item reads the world and that item alone, so `reused` may stand
/// in for an item whose result the caller already holds (F.40 phase 3,
/// X2, `check::check_bundle_by_declaration`), keyed `(program key, item
/// index)`.
pub fn secret_reveal_by_item(
    programs: &BTreeMap<String, &Program>,
    renames: &[(Vec<String>, String)],
    sources: &[crate::symbol::SourceFile],
    reused: &dyn Fn(&str, usize) -> Option<Vec<Diag>>,
) -> BTreeMap<String, Vec<Vec<Diag>>> {
    let list: Vec<(&str, &Program)> = programs.iter().map(|(k, p)| (k.as_str(), *p)).collect();
    let mut world = World::gather(&list, renames);
    world.sources = sources.iter().map(|u| (u.path.clone(), u.base, u.len)).collect();
    programs
        .iter()
        .map(|(key, p)| {
            let per = p
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    reused(key, i).unwrap_or_else(|| {
                        let mut diags = Vec::new();
                        walk_items(&world, key, std::slice::from_ref(item), &mut diags);
                        diags
                    })
                })
                .collect();
            (key.clone(), per)
        })
        .collect()
}

/// `Pos(…)`-free, rename-free text of a declaration: the same body reads
/// the same checked as its own seed, imported under any alias, or from
/// any directory.
fn fingerprint(fd: &FnDecl, renames: &[(Vec<String>, String)]) -> String {
    // positions (`Pos(12)`) and the snapshot's numbering (`NodeId(7)`)
    // depend on where the text sits, not what it says
    // The stripping applies outside string literals only: an authored
    // literal that spells `Pos(12)` or `id: NodeId(123), ` is body text
    // and counts in full (outside review, finding 2).
    let text = outside_strings(&format!("{:?}", fd), |seg| {
        let mut text = seg.to_string();
        for open in ["Pos(", "NodeId("] {
            let mut out = String::with_capacity(text.len());
            let mut rest = text.as_str();
            while let Some(at) = rest.find(open) {
                let digits = &rest[at + open.len()..];
                let n = digits.bytes().take_while(u8::is_ascii_digit).count();
                out.push_str(&rest[..at + open.len()]);
                rest = &digits[n..];
            }
            out.push_str(rest);
            text = out;
        }
        // F.40 1.1b: every declaration, member and statement carries a
        // snapshot identity field, `id: NodeId(..)`, which says where the
        // site is, not what the body says. The field text goes too, so a
        // change to identity never reads as a change to a secret's body.
        for field in ["id: NodeId(), ", ", id: NodeId()"] {
            text = text.replace(field, "");
        }
        text
    });
    let mut text = text;
    // an imported seed's names arrive mangled (`__lib_<id>__<stem>__<name>`)
    for (path, mangled) in renames {
        if let Some(last) = path.last() {
            text = text.replace(&format!("\"{}\"", mangled), &format!("\"{}\"", last));
        }
    }
    format!("{:016x}", hale_graph::identity::fnv64(text.as_bytes()))
}

/// `f` applied to every segment of `text` outside a Rust-escaped string
/// literal (`"…"`, with `\\"` and `\\\\` escapes); the literals' bytes are
/// kept exactly.
fn outside_strings(text: &str, f: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut seg = String::new();
    let mut in_str = false;
    let mut esc = false;
    for c in text.chars() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            out.push_str(&f(&seg));
            seg.clear();
            out.push(c);
            in_str = true;
        } else {
            seg.push(c);
        }
    }
    out.push_str(&f(&seg));
    out
}

/// The file a declaration at `at` sits in, in the program keyed `key`.
/// The source map says, when the bundle has one; every verb that checks
/// hands its bundle the map it minted with, and a caller that wants a
/// pin to match supplies one. Without it the program's key is the file
/// only when the key names one (a `.hl` path): a directory target's key
/// is the directory, whose name says nothing of which file declared
/// what, so there the declaration has no file and no pin matches it
/// (outside review of #1277, finding 1).
fn own_file<'w>(world: &'w World, key: &'w str, at: u32) -> Option<&'w str> {
    if !world.sources.is_empty() {
        return world.file_of(at);
    }
    (std::path::Path::new(key).extension().and_then(|e| e.to_str()) == Some("hl")).then_some(key)
}

/// The deferral a declaration is allowed by: its name (as written, or
/// mangled from the file stem) and its pinned body.
fn deferral(world: &World, key: &str, locus: Option<&str>, fd: &FnDecl) -> Deferral {
    let fn_name = fd.name.name.as_str();
    // The declaration is identified by its module, not its spelling: the
    // source unit its span falls in must have the pin's stem, and the
    // module must declare the pin's companion fn. A program's own fn that
    // happens to be called `role_password` is no pin (outside review,
    // finding 1). An imported seed's declarations arrive mangled
    // (`__lib_<id>__<stem>__<name>`), the companion included. The
    // directory is not part of the identity: the DNA seeds are checked
    // from the tree, from a scratch copy and from the embedded host
    // cache, and only the file names travel with them.
    let file = own_file(world, key, fd.name.span.start.0);
    let file_stem = file
        .and_then(|f| std::path::Path::new(f).file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let Some((_, _, _, q, pin, _)) = PQ_DEFERRED.iter().find(|(stem, l, f, _, _, companion)| {
        // the name that pins the declaration: a method's locus (a
        // method's own name is never mangled), a free fn's name
        let (have, want) = match locus {
            Some(have) if !l.is_empty() && fn_name == *f => (have, *l),
            None if l.is_empty() => (fn_name, *f),
            _ => return false,
        };
        // The companion is declared in the candidate's OWN module, never
        // merely somewhere in the world: two unrelated libraries, each
        // holding one pinned name, do not vouch for each other (outside
        // review of #1277, finding 2). Unmangled, that is the same file;
        // mangled, the same library and stem.
        if file_stem == *stem && have == want {
            return companion.is_empty()
                || world.fns.get(*companion).is_some_and(|c| {
                    let there = own_file(world, &c.site.0, c.site.1);
                    there.is_some() && there == file
                });
        }
        // Mangled, `have` names the library it was declared in; with the
        // stem, that is its one module.
        match crate::mangle::mangled_library(have, stem, want) {
            Some(lib_id) => {
                // The name says a library and a file stem, but not
                // provenance: the candidate's own file must have the pin's
                // stem, and the companion must be declared in that same
                // file. Without provenance, no pin (outside review of
                // #1279).
                let Some(here) = file else { return false };
                if file_stem != *stem {
                    return false;
                }
                companion.is_empty()
                    || world.fns.get(&crate::mangle::mangled(&lib_id, stem, companion)).is_some_and(|c| {
                        own_file(world, &c.site.0, c.site.1) == Some(here)
                    })
            }
            None => false,
        }
    }) else {
        return Deferral::None;
    };
    let have = fingerprint(fd, &world.renames);
    if have == *pin {
        Deferral::Allowed(q)
    } else {
        Deferral::Changed { qualified: q, have, at: fd.name.span }
    }
}

enum Deferral {
    None,
    Allowed(&'static str),
    /// The pinned body changed; `at` is the declaration's name, where a
    /// stale pin is reported even when no reveal fires in the body.
    Changed { qualified: &'static str, have: String, at: Span },
}

fn walk_fn(world: &World, key: &str, locus: Option<&str>, fd: &FnDecl, diags: &mut Vec<Diag>) {
    let deferred = deferral(world, key, locus, fd);
    let mut b = Body::new(world, locus, Context::of(locus, &fd.name.name), &fd.params, deferred, diags);
    b.block(&fd.body, &Flow::leaks("is returned"));
    b.finish();
}

fn walk_items(world: &World, key: &str, items: &[TopDecl], diags: &mut Vec<Diag>) {
    for item in items {
        match item {
            TopDecl::Module(m) => walk_items(world, key, &m.items, diags),
            TopDecl::Fn(fd) => walk_fn(world, key, None, fd, diags),
            TopDecl::Const(c) => {
                let mut b = Body::new(world, None, Context::Const(c.name.name.clone()), &[], Deferral::None, diags);
                b.expr(&c.value, &Flow::leaks("is a constant"));
                b.finish();
            }
            TopDecl::Locus(l) => {
                let locus = Some(l.name.name.as_str());
                for m in &l.members {
                    walk_member(world, key, locus, &l.name.name, m, diags);
                }
            }
            TopDecl::Perspective(p) => {
                let locus = Some(p.name.name.as_str());
                for m in &p.members {
                    match m {
                        PerspectiveMember::Fn(fd) => walk_fn(world, key, locus, fd, diags),
                        PerspectiveMember::Params(pb) => {
                            let mut b =
                                Body::new(world, locus, Context::of(locus, "params"), &[], Deferral::None, diags);
                            for pd in &pb.params {
                                if let ParamInit::Value(e) = &pd.init {
                                    b.expr(e, &Flow::leaks("is a param default, which the perspective keeps"));
                                }
                            }
                            b.finish();
                        }
                        PerspectiveMember::StableWhen(blk) => {
                            let mut b = Body::new(
                                world,
                                locus,
                                Context::of(locus, "stable_when"),
                                &[],
                                Deferral::None,
                                diags,
                            );
                            b.block(blk, &Flow::leaks("decides whether the perspective is stable"));
                            b.finish();
                        }
                        other => backstop(format!("{:?}", other), p.span, diags),
                    }
                }
            }
            other => backstop(format!("{:?}", other), other.span(), diags),
        }
    }
}

fn walk_member(world: &World, key: &str, locus: Option<&str>, locus_name: &str, m: &LocusMember, diags: &mut Vec<Diag>) {
    let body = |name: &str, params: &[Param], blk: &Block, diags: &mut Vec<Diag>| {
        let mut b = Body::new(world, locus, Context::of(locus, name), params, Deferral::None, diags);
        b.block(blk, &Flow::leaks("is returned"));
        b.finish();
    };
    match m {
        LocusMember::Fn(fd) => walk_fn(world, key, locus, fd, diags),
        LocusMember::Lifecycle(lc) => body(&format!("{:?}", lc.kind).to_lowercase(), &lc.params, &lc.body, diags),
        LocusMember::Mode(md) => body("<mode>", &md.params, &md.body, diags),
        LocusMember::Failure(fd) => body("on_failure", &fd.params, &fd.body, diags),
        LocusMember::Params(pb) => {
            let mut b = Body::new(world, locus, Context::of(locus, "params"), &[], Deferral::None, diags);
            for pd in &pb.params {
                if let ParamInit::Value(e) = &pd.init {
                    b.expr(e, &Flow::leaks(format!("is the default of `{}`, which the locus keeps", pd.name.name)));
                }
            }
            b.finish();
        }
        LocusMember::Const(c) => {
            let mut b = Body::new(world, locus, Context::of(locus, &c.name.name), &[], Deferral::None, diags);
            b.expr(&c.value, &Flow::leaks("is a constant"));
            b.finish();
        }
        LocusMember::BirthCheck(bc) => {
            let mut b = Body::new(world, locus, Context::of(locus, "birth_check"), &[], Deferral::None, diags);
            b.expr(&bc.cond, &Flow::leaks("decides a `birth_check`"));
            if let Some(p) = &bc.payload {
                b.expr(p, &Flow::leaks("is a violation payload"));
            }
            b.finish();
        }
        other => {
            let _ = locus_name;
            backstop(format!("{:?}", other), member_span(other), diags)
        }
    }
}

fn member_span(m: &LocusMember) -> Span {
    // the members the walk does not cover carry their own span
    let raw = format!("{:?}", m);
    let Some(at) = raw.rfind("span: Span { start: Pos(") else { return Span::new(0, 0) };
    let tail = &raw[at + "span: Span { start: Pos(".len()..];
    let start: usize = tail.split(')').next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let end: usize = tail
        .split("end: Pos(")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(start);
    Span::new(start, end)
}

/// Fails closed: a reveal the walk does not reach (a contract, a
/// closure, a topic, a claims block …) is refused where it is.
fn backstop(raw: String, span: Span, diags: &mut Vec<Diag>) {
    if raw.contains("name: \"reveal\"") || raw.contains("name: \"reveal_text\"") {
        diags.push(Diag::ty(
            span,
            "a `reveal()` / `reveal_text()` here is outside every fn body the reveal rule walks, so nothing \
             shows it is consumed in its statement; reveal a `Credential` only in a locus method \
             (spec/semantics.md § \"@sealed\")."
                .to_string(),
        ));
    }
}

/// Where a body sits, for the message.
enum Context {
    Method(String, String),
    Free(String),
    Const(String),
}

impl Context {
    fn of(locus: Option<&str>, name: &str) -> Context {
        match locus {
            Some(l) => Context::Method(l.to_string(), name.to_string()),
            None => Context::Free(name.to_string()),
        }
    }
}

/// One fn or method body.
struct Body<'a> {
    world: &'a World,
    locus: Option<&'a str>,
    ctx: Context,
    /// its `@secret` params: sources in this body, while no inner
    /// binding shadows them
    secret_params: BTreeSet<String>,
    /// how many of `types`' first entries are the params
    n_params: usize,
    /// name -> the locus a binding holds, innermost last
    types: Vec<(String, Option<String>)>,
    deferred: Deferral,
    found: Vec<(Span, String)>,
    diags: &'a mut Vec<Diag>,
}

impl<'a> Body<'a> {
    fn new(
        world: &'a World,
        locus: Option<&'a str>,
        ctx: Context,
        params: &[Param],
        deferred: Deferral,
        diags: &'a mut Vec<Diag>,
    ) -> Body<'a> {
        let types = params.iter().map(|p| (p.name.name.clone(), world.key_of(&p.ty))).collect();
        Body {
            world,
            locus,
            ctx,
            secret_params: params.iter().filter(|p| p.secret).map(|p| p.name.name.clone()).collect(),
            n_params: params.len(),
            types,
            deferred,
            found: Vec::new(),
            diags,
        }
    }

    fn where_(&self) -> String {
        match &self.ctx {
            Context::Method(l, f) => format!("`{}.{}`", self.world.display(l), f),
            Context::Free(f) => format!("the free fn `{}`", self.world.display(f)),
            Context::Const(c) => format!("the constant `{}`", self.world.display(c)),
        }
    }

    fn finish(self) {
        let Body { found, deferred, diags, .. } = self;
        // A pin that no longer matches its body is refused whether or
        // not the body reveals anything the rule catches: the pin is
        // the record of a review, and a stale one would let the first
        // reveal added later pass under the old review's name.
        if found.is_empty() {
            if let Deferral::Changed { qualified, have, at } = &deferred {
                diags.push(Diag::ty(
                    *at,
                    format!(
                        "`{}` is allowed by name only with the body that was reviewed, and this one has \
                         changed (fingerprint {}); until `pq` takes a `std::secret::Credential` ({}), a \
                         change to it is reviewed and its pin in crates/hale-types/src/secret_reveal.rs \
                         updated.",
                        qualified, have, DEFERRED_LINE
                    ),
                ));
            }
            return;
        }
        for (span, msg) in found {
            match &deferred {
                Deferral::Allowed(q) => diags.push(Diag::warn(
                    span,
                    format!("{} It is allowed here by name, in `{}`, until `pq` takes a `std::secret::Credential` ({}).", msg, q, DEFERRED_LINE),
                )),
                Deferral::Changed { qualified, have, .. } => diags.push(Diag::ty(
                    span,
                    format!(
                        "{} `{}` is allowed by name only with the body that was reviewed, and this one has \
                         changed (fingerprint {}); until `pq` takes a `std::secret::Credential` ({}), a \
                         change to it is reviewed and its pin in crates/hale-types/src/secret_reveal.rs \
                         updated.",
                        msg, qualified, have, DEFERRED_LINE
                    ),
                )),
                Deferral::None => diags.push(Diag::ty(span, msg)),
            }
        }
    }

    fn is_local(&self, name: &str) -> bool {
        self.types.iter().any(|(n, _)| n == name)
    }

    fn type_of_name(&self, name: &str) -> Option<String> {
        self.types.iter().rev().find(|(n, _)| n == name).and_then(|(_, t)| t.clone())
    }

    /// The type key of a receiver, when position says.
    fn receiver_type(&self, recv: &Expr) -> Option<String> {
        match recv {
            Expr::KwSelf(_) => self.locus.map(str::to_string),
            Expr::Ident(id) => self.type_of_name(&id.name),
            Expr::Struct { path, .. } => {
                let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                self.world.key_of_segs(&segs)
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

    /// Whether `.reveal()` / `.reveal_text()` on `recv` reveals a
    /// credential: unless the receiver is a locus of the program's own
    /// that declares the method, it does — an alias, a value the pass
    /// cannot type, a `Credential` all count.
    fn reveals(&self, recv: &Expr, method: &str) -> bool {
        match self.receiver_type(recv) {
            Some(k) if k != CREDENTIAL => {
                !self.world.methods.get(&k).map(|ms| ms.contains_key(method)).unwrap_or(false)
            }
            _ => true,
        }
    }

    /// A secret that ends at `flow`: refused unless consumed.
    fn source(&mut self, span: Span, what: &str, flow: &Flow) {
        if let Flow::Leaks(clause) = flow {
            self.found.push((
                span,
                format!(
                    "{} must be consumed in the statement that {}, by a wire write, a comparison of the \
                     whole value or a `@secret` parameter: here it {}. A revealed secret never outlives \
                     the statement that sends it (spec/semantics.md § \"@sealed\").",
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
                self.expr(value, &Flow::leaks(format!("is bound to `{}`", name.name)));
                let t = match value {
                    Expr::Ident(id) => self.type_of_name(&id.name),
                    other => self.value_type(other),
                };
                self.types.push((name.name.clone(), t));
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value, &Flow::leaks("is bound by a tuple `let`"));
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
                        self.expr(ix, &Flow::leaks("is used as an index"));
                    }
                }
                self.expr(value, &Flow::leaks(place));
            }
            Stmt::Return(Some(e), _) => self.expr(e, &Flow::leaks("is returned")),
            Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) => {}
            Stmt::Fail { value, .. } => self.expr(value, &Flow::leaks("is raised as a failure")),
            Stmt::If(i) => self.if_chain(i, &Flow::leaks("is not consumed")),
            Stmt::Match(m) => self.match_arms(m, &Flow::leaks("is not consumed")),
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter, &Flow::leaks("is iterated"));
                self.types.push((name.name.clone(), None));
                self.block(body, &Flow::leaks("is not consumed"));
                self.types.pop();
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, &Flow::leaks("decides a branch"));
                self.block(body, &Flow::leaks("is not consumed"));
            }
            Stmt::Block(b) => self.block(b, &Flow::leaks("is not consumed")),
            Stmt::Send { subject, value, .. } => {
                self.expr(subject, &Flow::leaks("is a bus subject"));
                self.expr(value, &Flow::leaks("is published on the bus"));
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max, &Flow::leaks("is a shared-memory size"));
                self.block(body, &Flow::leaks("is not consumed"));
            }
            Stmt::Recovery { args, .. } => {
                for a in args {
                    self.expr(a, &Flow::leaks("is a recovery argument"));
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.expr(p, &Flow::leaks("is a violation payload"));
                }
            }
            Stmt::Reperspective { .. } => {}
            Stmt::Expr(e) => self.expr(e, &Flow::leaks("is not consumed")),
        }
    }

    /// A condition's value decides a branch: a secret reaches one only
    /// through a comparison of the whole value.
    fn if_chain(&mut self, i: &IfStmt, flow: &Flow) {
        self.expr(&i.cond, &Flow::leaks("decides a branch"));
        self.block(&i.then_block, flow);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b, flow),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n, flow),
            None => {}
        }
    }

    fn match_arms(&mut self, m: &MatchStmt, flow: &Flow) {
        // matching the whole value against patterns that bind nothing
        // compares it; a pattern that binds keeps it under a new name
        let mut bound = Vec::new();
        for a in &m.arms {
            pattern_bindings(&a.pattern, &mut bound);
        }
        let scrutinee = match bound.first() {
            None => Flow::Compared,
            Some(n) => Flow::leaks(format!("is bound to `{}` by a `match` pattern", n)),
        };
        self.expr(&m.scrutinee, &scrutinee);
        for a in &m.arms {
            let mark = self.types.len();
            let mut names = Vec::new();
            pattern_bindings(&a.pattern, &mut names);
            // a pattern's binding is a value the pass does not type
            self.types.extend(names.into_iter().map(|n| (n, None)));
            if let Some(g) = &a.guard {
                self.expr(g, &Flow::leaks("decides a branch"));
            }
            match &a.body {
                MatchArmBody::Expr(e) => self.expr(e, flow),
                MatchArmBody::Block(b) => self.block(b, flow),
            }
            self.types.truncate(mark);
        }
    }

    fn not_the_wire(&self, spelled: &str, i: usize) -> Flow {
        Flow::leaks(format!(
            "is argument {} of `{}`, which is not its payload: an error can repeat it back",
            i + 1,
            spelled
        ))
    }

    /// The flow of argument `i` of a call.
    fn arg_flow(&self, callee: &Expr, i: usize, result_flow: &Flow) -> Flow {
        match callee {
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                let method = name.name.as_str();
                let Some(t) = self.receiver_type(receiver) else {
                    return Flow::leaks(format!("reaches `.{}(…)` on a value whose type the check cannot see", method));
                };
                if let Some((_, _, payload)) = WIRE_METHODS.iter().find(|(l, m, _)| *l == t && *m == method) {
                    return if payload.contains(&i) {
                        Flow::Consumed
                    } else {
                        self.not_the_wire(&format!("{}.{}", self.world.display(&t), method), i)
                    };
                }
                if COMPARE_METHODS.iter().any(|(l, m)| *l == t && *m == method) {
                    return Flow::Compared;
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
                    _ => Flow::leaks(format!(
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
                    return Flow::leaks(format!("reaches `{}`", spelled));
                }
                if let Some((_, payload)) = WIRE_FNS.iter().find(|(w, _)| *w == segs.as_slice()) {
                    return if payload.contains(&i) { Flow::Consumed } else { self.not_the_wire(&spelled, i) };
                }
                if is_value_fn(&segs) {
                    // composition: the result carries it on
                    return result_flow.composed();
                }
                Flow::leaks(format!(
                    "reaches `{}`, which is not a wire write, a comparison or a `@secret` parameter",
                    spelled
                ))
            }
            Expr::Ident(id) if self.is_local(&id.name) => {
                Flow::leaks(format!("reaches `{}`, a value called as a fn, which the check cannot follow", id.name))
            }
            other => {
                let Some(n) = self.world.fn_name(other) else {
                    return Flow::leaks("reaches a call the check cannot follow");
                };
                if let Some(f) = self.world.fns.get(&n) {
                    if f.params.get(i).map(|(_, s)| *s).unwrap_or(false) {
                        return Flow::Consumed;
                    }
                    if self.world.transparent.contains(&n) {
                        return result_flow.composed();
                    }
                    return Flow::leaks(format!(
                        "reaches `{}`, which is not a wire write, a comparison or a `@secret` parameter",
                        self.world.display(&n)
                    ));
                }
                if VALUE_BUILTINS.contains(&n.as_str()) {
                    return result_flow.composed();
                }
                match crate::stdlib_surface::builtin_effects(&n) {
                    Some(_) => Flow::leaks(format!("is printed (`{}`)", n)),
                    None => Flow::leaks(format!("reaches `{}`, which the check cannot follow", n)),
                }
            }
        }
    }

    /// A call that takes one `ClientRequest`: the literal's `headers`
    /// and `body` are the wire's, its `url` and `method` are not (an
    /// `HttpError` repeats the host back).
    fn request_arg(&mut self, spelled: &str, arg: &Expr) {
        match arg {
            Expr::Struct { path, inits, .. } if path.segments.last().map(|s| s.name == "ClientRequest").unwrap_or(false) => {
                for init in inits {
                    let f = if REQUEST_WIRE_FIELDS.contains(&init.name.name.as_str()) {
                        Flow::Consumed
                    } else {
                        Flow::leaks(format!(
                            "is the request's `{}`, which is not the wire's: only its `headers` and `body` \
                             are, and an error can repeat the rest back",
                            init.name.name
                        ))
                    };
                    self.expr(&init.value, &f);
                }
            }
            other => self.expr(
                other,
                &Flow::leaks(format!(
                    "reaches `{}` in a request built elsewhere: only a `ClientRequest {{ … }}` literal's \
                     `headers` and `body` are the wire's",
                    spelled
                )),
            ),
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
                let f = if matches!(op, BinOp::Eq | BinOp::NotEq) { Flow::Compared } else { flow.composed() };
                self.expr(left, &f);
                self.expr(right, &f);
            }
            Expr::Unary { operand, .. } => self.expr(operand, &flow.composed()),
            Expr::Call { callee, args, span, .. } => {
                // a reveal is a source
                if let Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } = callee.as_ref() {
                    if name.name == "reveal" || name.name == "reveal_text" {
                        // inside `Credential` itself, `self.reveal()` is the
                        // sealed side calling itself (`reveal_text`), not a
                        // reveal to the application
                        let sealed_side = matches!(receiver.as_ref(), Expr::KwSelf(_)) && self.locus == Some(CREDENTIAL);
                        if !sealed_side && self.reveals(receiver, &name.name) {
                            let what = format!("`Credential.{}()`", name.name);
                            if self.locus.is_none() {
                                self.found.push((
                                    *span,
                                    format!(
                                        "{} may be called only in a locus method, and {} is not one: a revealed \
                                         secret is consumed in the statement that reveals it, by a wire write, \
                                         a comparison of the whole value or a `@secret` parameter \
                                         (spec/semantics.md § \"@sealed\").",
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
                    self.expr(receiver, &Flow::leaks(format!("has `.{}(…)` called on it", name.name)));
                }
                // a call taking a `ClientRequest`
                let request = match callee.as_ref() {
                    Expr::Path(qn) => {
                        let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                        REQUEST_FNS.iter().any(|r| *r == segs.as_slice()).then(|| segs.join("::"))
                    }
                    Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                        self.receiver_type(receiver).and_then(|t| {
                            REQUEST_METHODS
                                .iter()
                                .any(|(l, m)| *l == t && *m == name.name)
                                .then(|| format!("{}.{}", self.world.display(&t), name.name))
                        })
                    }
                    _ => None,
                };
                if let (Some(spelled), [arg]) = (request, args.as_slice()) {
                    self.request_arg(&spelled, arg);
                    return;
                }
                for (i, a) in args.iter().enumerate() {
                    let f = self.arg_flow(callee, i, flow);
                    self.expr(a, &f);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver, &flow.composed()),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver, &flow.composed());
                self.expr(index, &Flow::leaks("is used as an index"));
            }
            Expr::Tuple(v, _) | Expr::Array(v, _) => {
                for x in v {
                    self.expr(x, &flow.composed());
                }
            }
            Expr::Struct { path, inits, .. } => {
                let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                let locus = self.world.key_of_segs(&segs).filter(|k| self.world.loci.contains(k));
                let f = match locus {
                    Some(l) => Flow::leaks(format!("is stored in a `{} {{ … }}`", self.world.display(&l))),
                    None => flow.composed(),
                };
                for i in inits {
                    self.expr(&i.value, &f);
                }
            }
            Expr::Block(b) => self.block(b, flow),
            Expr::If(i) => self.if_chain(i, flow),
            Expr::Match(m) => self.match_arms(m, flow),
            Expr::Sum(x, _) | Expr::Prod(x, _) | Expr::ArrayRepeat { val: x, .. } => self.expr(x, &flow.composed()),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left, &flow.composed());
                self.expr(right, &flow.composed());
                self.expr(tolerance, &flow.composed());
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo, &flow.composed());
                self.expr(hi, &flow.composed());
            }
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner, flow);
                match disposition {
                    OrDisposition::Substitute(x) => self.expr(x, flow),
                    OrDisposition::Fail(x, _) => self.expr(x, &Flow::leaks("is raised as a failure")),
                    OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
                }
            }
        }
    }
}
