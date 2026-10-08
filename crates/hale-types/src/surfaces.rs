//! GH #1417 (R1): the `surface` family — the program's API as rows
//! (spec/api.md § Surfaces and their rows).
//!
//! An `api` block's `rpc` lines and the `@rpc` handlers are one row
//! family: the surface, the member as a caller names it, the handler's
//! request, response and error types with their contract shape hashes
//! (`spec/model.md` § The shape of a type), the pools its locus runs on,
//! and the roles a caller must hold. Each surface's contract digest is
//! folded over its rows (`hale_model::surface::surface_digest`). Nothing
//! else about a locus is read: not its subscriptions, not its `expose`
//! members, not its position in the tree.
//!
//! The serve sites (`api::serve(…)`) and the topic bindings to a hub are
//! NAMED here, as far as a description needs them (the surface, the
//! transport's kind, listener, codec and sources, `as:`, the receivers,
//! `bound:` and `on_full:`; a hub's `as:` and its stream rows), and
//! neither checked nor lowered: their laws and their runtime are R2's
//! and R5's, and a build refuses them ([`unserved_sites`]).
//!
//! One producer ([`surface_rows`]), demanded once per snapshot
//! (`Snapshot::demand_surface_rows`): the check's surface laws read it
//! (`CheckInputs::surfaces`), the model projects its surfaces and rows,
//! and `hale check --api` renders the inventory and the descriptions
//! from it.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    flat_decls, Block, ElseBranch, Expr, FnDecl, LifecycleKind, LocusDecl, LocusMember, MatchArmBody, ModeKind,
    ParamInit, Program, ServeSite, Stmt, TopDecl, TypeExpr,
};
use hale_syntax::{Diag, Span};

use crate::alloc_summary::{DeclId, FnKey};
use crate::placement::{DomainKind, PlacementTable};
use crate::symbol::Bundle;
use crate::topic_identity::{ShapeForm, Shapes, TopicRows};

/// The `surface` family of one bundle.
#[derive(Debug, Clone, Default)]
pub struct SurfaceRows {
    /// Every surface, by name.
    pub surfaces: Vec<Surface>,
    /// Every row, by surface and member as bytes; two rows of one member
    /// in one surface (law 3) are both kept, in declaration order.
    pub rows: Vec<Row>,
    /// The serve sites, in declaration order.
    pub serves: Vec<Serve>,
    /// The hubs the topic bindings name, in declaration order.
    pub hubs: Vec<Hub>,
    /// The entry's name: the inventory's `app`.
    pub app: Option<String>,
}

/// A surface: an `api` block (two blocks of one name are one surface),
/// or the seed's default surface, which its `@rpc` handlers feed.
#[derive(Debug, Clone)]
pub struct Surface {
    pub name: String,
    /// The first `api` block of the name, or the first `@rpc` of a
    /// default surface.
    pub span: Span,
    /// The contract digest over the surface's rows.
    pub digest: u64,
}

/// One row: an `rpc` line, or an `@rpc` handler.
#[derive(Debug, Clone)]
pub struct Row {
    pub surface: String,
    /// The handler as a caller names it: `Locus::fn`, the locus as the
    /// row's author wrote it (`lib::Orders::cancel`).
    pub member: String,
    /// The locus declaration the row names, by its post-merge name.
    pub locus: String,
    pub method: String,
    /// The roles a caller must hold, as written.
    pub requires: Vec<(String, Span)>,
    /// The `rpc` line, or the `@rpc` attribute.
    pub span: Span,
    /// From `@rpc` rather than an `api` block.
    pub from_attr: bool,
    /// The row's position among the bundle's rows as written: a
    /// surface's rows in the order its author wrote them.
    pub written_at: usize,
    pub handler: Handled,
}

/// What a row's `Locus::fn` resolves to.
#[derive(Debug, Clone)]
pub enum Handled {
    /// A member fn of a declared locus.
    Fn(Handler),
    /// No locus of the name is declared; the declared loci, for a
    /// did-you-mean.
    NoLocus { loci: Vec<String> },
    /// The locus declares no fn of the name; its fns, for a
    /// did-you-mean.
    NoFn { fns: Vec<String> },
    /// The name is the locus's lifecycle method, mode or `on_failure`.
    NotAHandler { kind: &'static str },
    /// `@rpc` on a free fn.
    FreeFn,
}

/// A handler's signature, as the rows read it.
#[derive(Debug, Clone)]
pub struct Handler {
    /// The summary's key for the fn: the may-violate judgment's join.
    pub key: FnKey,
    pub name_span: Span,
    /// The value parameters, a trailing `ctx: std::api::Context`
    /// excluded: each its name, type and span.
    pub params: Vec<(String, TypeExpr, Span)>,
    /// The first value parameter's type.
    pub request: Option<RowTy>,
    /// The return type; `None` for `()`.
    pub response: Option<RowTy>,
    /// `E` of `fallible(E)`.
    pub error: Option<RowTy>,
    /// The error type is `ClosureViolation`.
    pub server_error: bool,
    /// Every pool an instance of the locus runs on, sorted.
    pub pools: Vec<String>,
}

/// A type a row names.
#[derive(Debug, Clone)]
pub struct RowTy {
    pub te: TypeExpr,
    /// As the signature writes it.
    pub display: String,
    /// Its contract shape and that shape's hash.
    pub shape: String,
    pub hash: u64,
}

/// A transport instance's source, as the program names it: the param
/// (`self.bearer`) and its locus type; `kernel` for the Unix
/// transport's peer credentials, which has no type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub source: String,
    pub ty: Option<String>,
}

/// What a transport instance's literal says: its kind (`http`, `unix`,
/// `ws`, … the path's namespace), its address (`bind:` or `path:`), its
/// codec (`codec:`, `json` when it names none), its bearer source
/// (`principals:`) and its role source (`roles:`).
#[derive(Debug, Clone)]
pub struct Transport {
    pub kind: String,
    pub address: Option<String>,
    pub codec: String,
    pub principals: Option<Source>,
    pub roles: Option<Source>,
    /// `as:` on the literal: a hub's exposure name.
    pub name: Option<String>,
}

/// A serve site, named: `api::serve(SURFACE, TRANSPORT, as: NAME,
/// receivers: { TYPE: INSTANCE, … }, bound: N, on_full: POLICY)`.
#[derive(Debug, Clone)]
pub struct Serve {
    /// The surface as written.
    pub surface: Option<String>,
    pub transport: Option<Transport>,
    /// `as:`.
    pub name: Option<String>,
    /// The receivers: the bound ones as written, then each locus type
    /// the surface's rows name that the serving locus holds exactly one
    /// of, inferred.
    pub receivers: Vec<Receiver>,
    pub bound: Option<i64>,
    pub on_full: Option<String>,
    /// The locus whose body holds the site.
    pub serving_locus: String,
    pub span: Span,
}

/// One receiver of a serve site.
#[derive(Debug, Clone)]
pub struct Receiver {
    /// The locus type, as written.
    pub ty: String,
    /// The instance, as written (`self.orders`).
    pub instance: String,
    /// Bound in `receivers:` rather than inferred.
    pub explicit: bool,
    /// The pool the instance runs on, when the placement table places it.
    pub pool: Option<String>,
}

/// A hub: a param of a transport type the program binds topics to.
#[derive(Debug, Clone)]
pub struct Hub {
    /// `self.<param>`.
    pub instance: String,
    pub transport: Transport,
    pub streams: Vec<Stream>,
    pub span: Span,
}

/// One stream row of a hub: a topic binding to it.
#[derive(Debug, Clone)]
pub struct Stream {
    pub topic: String,
    /// The topic's wire subject.
    pub subject: String,
    /// `out` for a topic the program publishes, `in` otherwise.
    pub direction: &'static str,
    pub payload: Option<RowTy>,
    pub codec: String,
    pub bound: Option<u64>,
    pub on_full: Option<String>,
    /// A hub binding provides no replay in v1.
    pub replay: bool,
    pub requires: Vec<(String, Span)>,
    pub span: Span,
}

impl SurfaceRows {
    /// The rows of `surface`, by member.
    pub fn rows_of<'s>(&'s self, surface: &'s str) -> impl Iterator<Item = &'s Row> + 's {
        self.rows.iter().filter(move |r| r.surface == surface)
    }

    pub fn surface(&self, name: &str) -> Option<&Surface> {
        self.surfaces.iter().find(|s| s.name == name)
    }

    /// Whether the bundle declares any surface, serve site or hub.
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty() && self.serves.is_empty() && self.hubs.is_empty()
    }
}

impl Hub {
    /// The hub's stream digest over its stream rows.
    pub fn digest(&self) -> u64 {
        let reqs: Vec<Vec<String>> =
            self.streams.iter().map(|s| s.requires.iter().map(|(r, _)| r.clone()).collect()).collect();
        let lines: Vec<hale_model::surface::StreamLine<'_>> = self
            .streams
            .iter()
            .zip(&reqs)
            .map(|(s, r)| hale_model::surface::StreamLine {
                topic: &s.topic,
                payload: s.payload.as_ref().map_or(0, |p| p.hash),
                direction: s.direction,
                codec: &s.codec,
                bound: s.bound.unwrap_or(0),
                on_full: s.on_full.as_deref().unwrap_or("-"),
                replay: s.replay,
                requires: r,
            })
            .collect();
        hale_model::surface::stream_digest(&lines)
    }
}

/// A type as its declaration writes it.
pub fn type_text(te: &TypeExpr) -> String {
    match te {
        TypeExpr::Primitive(p, _) => crate::ty::prim_name(*p).to_string(),
        TypeExpr::Named { path, generic_args, .. } => {
            let base = path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
            if generic_args.is_empty() {
                base
            } else {
                format!("{base}<{}>", generic_args.iter().map(type_text).collect::<Vec<_>>().join(", "))
            }
        }
        TypeExpr::Array { elem, size: None, .. } => format!("[{}]", type_text(elem)),
        TypeExpr::Array { elem, size: Some(_), .. } => format!("[{}; …]", type_text(elem)),
        TypeExpr::Bounded { elem, cap, .. } => format!("bounded[{}; {cap}]", type_text(elem)),
        TypeExpr::Tuple(parts, _) => format!("({})", parts.iter().map(type_text).collect::<Vec<_>>().join(", ")),
        TypeExpr::Function { .. } => "fn".to_string(),
        TypeExpr::Projection { .. } => "projection".to_string(),
        TypeExpr::Perspective { name, .. } => format!("perspective({})", name.name),
    }
}

/// A pool as a row and a receiver name it: `main`, a pool's name,
/// `pinned:<path>`.
fn pool_label(table: &PlacementTable, id: crate::placement::DomainId) -> String {
    match &table.domain(id).kind {
        DomainKind::Main => "main".to_string(),
        DomainKind::Pool { name, .. } => name.clone(),
        DomainKind::Pinned { anchor, .. } => format!("pinned:{}", table.path_of(anchor)),
    }
}

/// The seed a bundle is: its program's file stem, or its directory's
/// name; the default surface of its `@rpc` handlers is named after it.
fn seed_name(bundle: &Bundle<'_>) -> String {
    let keys: Vec<&String> = bundle.programs.keys().collect();
    let stem = |k: &str| {
        std::path::Path::new(k).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| k.to_string())
    };
    match keys.as_slice() {
        [one] => stem(one),
        [first, ..] => std::path::Path::new(first.as_str())
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| stem(first)),
        [] => String::new(),
    }
}

/// The value parameters of a handler: a trailing `ctx:
/// std::api::Context` is not part of the request.
fn value_params(f: &FnDecl) -> Vec<(String, TypeExpr, Span)> {
    let mut ps: Vec<(String, TypeExpr, Span)> =
        f.params.iter().map(|p| (p.name.name.clone(), p.ty.clone(), p.span)).collect();
    if ps.last().is_some_and(|(_, t, _)| hale_syntax::api_gen::is_context_type(t)) {
        ps.pop();
    }
    ps
}

fn lifecycle_name(k: &LifecycleKind) -> &'static str {
    match k {
        LifecycleKind::Birth => "birth",
        LifecycleKind::Accept => "accept",
        LifecycleKind::Release => "release",
        LifecycleKind::Run => "run",
        LifecycleKind::Drain => "drain",
        LifecycleKind::Dissolve => "dissolve",
        #[allow(unreachable_patterns)]
        _ => "lifecycle",
    }
}

fn mode_name(k: &ModeKind) -> &'static str {
    match k {
        ModeKind::Bulk => "bulk",
        ModeKind::Harmonic => "harmonic",
        ModeKind::Resolution => "resolution",
    }
}

/// `self.<field>` as written, and the field.
pub(crate) fn self_field(e: &Expr) -> Option<&str> {
    match e {
        Expr::Field { receiver, name, .. } if matches!(receiver.as_ref(), Expr::KwSelf(_)) => Some(name.name.as_str()),
        _ => None,
    }
}

/// A locus's params, by name, with each one's type as written: the
/// declared type, or the path of the literal that initializes it.
pub(crate) fn param_types(l: &LocusDecl) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for m in &l.members {
        if let LocusMember::Params(pb) = m {
            for p in &pb.params {
                let ty = match (&p.ty, &p.init) {
                    (Some(t), _) => Some(type_text(t)),
                    (None, ParamInit::Value(Expr::Struct { path, .. })) => {
                        Some(path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"))
                    }
                    _ => None,
                };
                if let Some(t) = ty {
                    out.insert(p.name.name.clone(), t);
                }
            }
        }
    }
    out
}

/// What a transport instance's literal says, its sources typed by the
/// params of the locus that holds them.
fn transport_of(e: &Expr, params: &BTreeMap<String, String>) -> Option<Transport> {
    let Expr::Struct { path, inits, .. } = e else { return None };
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    let kind = match segs.as_slice() {
        [.., ns, _] => ns.to_string(),
        [one] => one.to_lowercase(),
        [] => return None,
    };
    let field = |n: &str| inits.iter().find(|i| i.name.name == n).map(|i| &i.value);
    let text = |e: Option<&Expr>| match e {
        Some(Expr::Literal(hale_syntax::ast::Literal::String(s), _)) => Some(s.clone()),
        _ => None,
    };
    let source = |e: Option<&Expr>| {
        let e = e?;
        let f = self_field(e)?;
        Some(Source { source: format!("self.{f}"), ty: params.get(f).cloned() })
    };
    let codec = match field("codec") {
        Some(Expr::Ident(i)) => i.name.clone(),
        Some(Expr::Path(qn)) => qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
        _ => "json".to_string(),
    };
    Some(Transport {
        kind,
        address: text(field("bind")).or_else(|| text(field("path"))),
        codec,
        principals: source(field("principals")),
        roles: source(field("roles")),
        name: text(field("as")),
    })
}

/// Every serve site in a block, in order.
pub(crate) fn serve_sites_in<'a>(b: &'a Block, out: &mut Vec<ServeSite<'a>>) {
    for s in &b.stmts {
        match s {
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Assign { value, .. } => expr_sites(value, out),
            Stmt::Expr(e) => expr_sites(e, out),
            Stmt::If(i) => if_sites(i, out),
            Stmt::Match(m) => {
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Expr(e) => expr_sites(e, out),
                        MatchArmBody::Block(b) => serve_sites_in(b, out),
                    }
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => serve_sites_in(body, out),
            Stmt::Block(b) => serve_sites_in(b, out),
            _ => {}
        }
    }
    if let Some(t) = &b.tail {
        expr_sites(t, out);
    }
}

fn if_sites<'a>(i: &'a hale_syntax::ast::IfStmt, out: &mut Vec<ServeSite<'a>>) {
    serve_sites_in(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => serve_sites_in(b, out),
        Some(ElseBranch::ElseIf(e)) => if_sites(e, out),
        None => {}
    }
}

fn expr_sites<'a>(e: &'a Expr, out: &mut Vec<ServeSite<'a>>) {
    if let Some(s) = ServeSite::of(e) {
        out.push(s);
    }
}

/// The serve sites a locus's bodies hold.
pub(crate) fn locus_serve_sites(l: &LocusDecl) -> Vec<ServeSite<'_>> {
    let mut out = Vec::new();
    for m in &l.members {
        match m {
            LocusMember::Lifecycle(lc) => serve_sites_in(&lc.body, &mut out),
            LocusMember::Fn(f) => serve_sites_in(&f.body, &mut out),
            LocusMember::Mode(md) => serve_sites_in(&md.body, &mut out),
            _ => {}
        }
    }
    out
}

/// The `surface` family of `bundle` (spec/api.md § Surfaces and their
/// rows): every row an `api` block or an `@rpc` handler declares,
/// resolved against the loci it names; each surface's digest; the serve
/// sites and hubs, named. `placement` answers the pools; `topics` the
/// wire subjects of the hubs' streams.
pub fn surface_rows(
    bundle: &Bundle<'_>,
    entry: &crate::entry::EntryRow,
    placement: &PlacementTable,
    topics: &TopicRows,
) -> SurfaceRows {
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let slices: Vec<&[TopDecl]> = programs.iter().map(|p| p.items.as_slice()).collect();
    let shapes = Shapes::of_all(&slices);
    let row_ty = |te: &TypeExpr| {
        let shape = shapes.shape(te, ShapeForm::Contract).unwrap_or_default();
        RowTy { te: te.clone(), display: type_text(te), hash: crate::topic_identity::shape_hash(&shape), shape }
    };

    let mut loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    for p in &programs {
        for d in flat_decls(&p.items) {
            if let TopDecl::Locus(l) = d {
                loci.insert(l.name.name.as_str(), l);
            }
        }
    }
    let handed_off = placement.handed_off();
    let pools_of = |locus: &str| -> Vec<String> {
        let set: BTreeSet<String> = placement
            .instances
            .iter()
            .filter(|(k, r)| !handed_off.contains(k) && r.realizes.as_ref().is_some_and(|d| d.lowered == locus))
            .map(|(_, r)| pool_label(placement, r.domain))
            .collect();
        set.into_iter().collect()
    };
    let resolve = |locus: &str, method: &str| -> Handled {
        let Some(l) = loci.get(locus) else {
            return Handled::NoLocus { loci: loci.keys().map(|k| k.to_string()).collect() };
        };
        for m in &l.members {
            match m {
                LocusMember::Fn(f) if f.name.name == method => {
                    let params = value_params(f);
                    let response = match &f.ret {
                        None => None,
                        Some(TypeExpr::Tuple(parts, _)) if parts.is_empty() => None,
                        Some(t) => Some(row_ty(t)),
                    };
                    let server_error = matches!(&f.fallible, Some(TypeExpr::Named { path, .. })
                        if path.segments.last().is_some_and(|s| s.name == "ClosureViolation"));
                    return Handled::Fn(Handler {
                        key: FnKey::method(DeclId::user(f.id), l.name.name.clone(), f.name.name.clone()),
                        name_span: f.name.span,
                        request: params.first().map(|(_, t, _)| row_ty(t)),
                        params,
                        response,
                        error: f.fallible.as_ref().map(row_ty),
                        server_error,
                        pools: pools_of(locus),
                    });
                }
                LocusMember::Lifecycle(lc) if lifecycle_name(&lc.kind) == method => {
                    return Handled::NotAHandler { kind: "lifecycle method" };
                }
                LocusMember::Mode(md) if mode_name(&md.kind) == method => {
                    return Handled::NotAHandler { kind: "mode" };
                }
                LocusMember::Failure(_) if method == "on_failure" => {
                    return Handled::NotAHandler { kind: "failure handler" };
                }
                _ => {}
            }
        }
        let fns = l
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Fn(f) => Some(f.name.name.clone()),
                _ => None,
            })
            .collect();
        Handled::NoFn { fns }
    };

    // ---- the rows, as written ----
    let default_surface = seed_name(bundle);
    let mut rows: Vec<Row> = Vec::new();
    let mut surface_spans: BTreeMap<String, Span> = BTreeMap::new();
    for p in &programs {
        for d in flat_decls(&p.items) {
            match d {
                TopDecl::Api(a) => {
                    surface_spans.entry(a.name.name.clone()).or_insert(a.span);
                    for r in &a.rows {
                        rows.push(Row {
                            surface: a.name.name.clone(),
                            member: format!("{}::{}", r.written, r.method.name),
                            locus: r.locus.name.clone(),
                            method: r.method.name.clone(),
                            requires: r.requires.iter().map(|i| (i.name.clone(), i.span)).collect(),
                            span: r.span,
                            from_attr: false,
                            written_at: rows.len(),
                            handler: resolve(&r.locus.name, &r.method.name),
                        });
                    }
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        let LocusMember::Fn(f) = m else { continue };
                        let Some(attr) = &f.rpc else { continue };
                        // A locus another seed declares feeds that seed's
                        // default surface, named by its import alias.
                        let (surface, written) = match bundle.import_renames.iter().find(|(_, m)| *m == l.name.name) {
                            Some((path, _)) => (path[0].clone(), path.join("::")),
                            None => (default_surface.clone(), l.name.name.clone()),
                        };
                        surface_spans.entry(surface.clone()).or_insert(attr.span);
                        rows.push(Row {
                            surface,
                            member: format!("{written}::{}", f.name.name),
                            locus: l.name.name.clone(),
                            method: f.name.name.clone(),
                            requires: attr.requires.iter().map(|i| (i.name.clone(), i.span)).collect(),
                            span: attr.span,
                            from_attr: true,
                            written_at: rows.len(),
                            handler: resolve(&l.name.name, &f.name.name),
                        });
                    }
                }
                TopDecl::Fn(f) => {
                    if let Some(attr) = &f.rpc {
                        rows.push(Row {
                            surface: default_surface.clone(),
                            member: f.name.name.clone(),
                            locus: String::new(),
                            method: f.name.name.clone(),
                            requires: attr.requires.iter().map(|i| (i.name.clone(), i.span)).collect(),
                            span: attr.span,
                            from_attr: true,
                            written_at: rows.len(),
                            handler: Handled::FreeFn,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    // Canonical order: by surface, then member as bytes; a member's two
    // rows in one surface in the order written.
    rows.sort_by(|a, b| {
        (a.surface.as_bytes(), a.member.as_bytes(), a.written_at).cmp(&(b.surface.as_bytes(), b.member.as_bytes(), b.written_at))
    });

    // ---- the surfaces and their digests ----
    let mut surfaces: Vec<Surface> = surface_spans
        .iter()
        .map(|(name, span)| {
            let reqs: Vec<(String, Vec<String>)> = rows
                .iter()
                .filter(|r| &r.surface == name)
                .map(|r| (r.member.clone(), r.requires.iter().map(|(n, _)| n.clone()).collect()))
                .collect();
            let lines: Vec<hale_model::surface::DigestLine<'_>> = rows
                .iter()
                .filter(|r| &r.surface == name)
                .zip(&reqs)
                .map(|(r, (_, req))| {
                    let h = match &r.handler {
                        Handled::Fn(h) => Some(h),
                        _ => None,
                    };
                    hale_model::surface::DigestLine {
                        member: &r.member,
                        request: h.and_then(|h| h.request.as_ref()).map(|t| t.hash),
                        response: h.and_then(|h| h.response.as_ref()).map(|t| t.hash),
                        error: h.and_then(|h| h.error.as_ref()).map(|t| t.hash),
                        requires: req,
                    }
                })
                .collect();
            Surface { name: name.clone(), span: *span, digest: hale_model::surface::surface_digest(&lines) }
        })
        .collect();
    surfaces.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));

    // ---- serve sites and hubs, named ----
    let mut serves = Vec::new();
    let mut hubs: Vec<Hub> = Vec::new();
    let publishes: BTreeSet<String> = programs
        .iter()
        .flat_map(|p| flat_decls(&p.items))
        .filter_map(|d| match d {
            TopDecl::Locus(l) => Some(l),
            _ => None,
        })
        .flat_map(|l| l.members.iter())
        .filter_map(|m| match m {
            LocusMember::Bus(b) => Some(b),
            _ => None,
        })
        .flat_map(|b| b.members.iter())
        .filter_map(|m| match m {
            hale_syntax::ast::BusMember::Publish { subject: hale_syntax::ast::BusSubject::Topic(t), .. } => {
                Some(t.name.clone())
            }
            _ => None,
        })
        .collect();
    for p in &programs {
        for d in flat_decls(&p.items) {
            let TopDecl::Locus(l) = d else { continue };
            let params = param_types(l);
            for site in locus_serve_sites(l) {
                let surface = match site.surface {
                    Expr::Ident(i) => Some(i.name.clone()),
                    Expr::Path(qn) => Some(qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")),
                    _ => None,
                };
                let instance_pool = |field: &str| -> Option<String> {
                    placement
                        .instances
                        .iter()
                        .find(|(k, r)| {
                            k.replica.is_none()
                                && k.path.len() == 1
                                && k.path[0].field == field
                                && r.owner.as_ref().is_some_and(|o| o.path.is_empty())
                        })
                        .map(|(_, r)| pool_label(placement, r.domain))
                };
                let mut receivers: Vec<Receiver> = site
                    .receivers()
                    .iter()
                    .map(|i| {
                        let field = self_field(&i.value);
                        Receiver {
                            ty: i.name.name.clone(),
                            instance: field.map_or_else(|| "?".to_string(), |f| format!("self.{f}")),
                            explicit: true,
                            pool: field.and_then(instance_pool),
                        }
                    })
                    .collect();
                // The types the surface's rows name, in the order its
                // rows were written, each the serving locus holds exactly
                // one of, inferred.
                let mut named: Vec<(usize, &str)> = rows
                    .iter()
                    .filter(|r| Some(&r.surface) == surface.as_ref() && !r.locus.is_empty())
                    .map(|r| (r.written_at, r.locus.as_str()))
                    .collect();
                named.sort();
                let mut seen: BTreeSet<String> = receivers.iter().map(|r| r.ty.clone()).collect();
                for (_, ty) in named {
                    if !seen.insert(ty.to_string()) {
                        continue;
                    }
                    let holders: Vec<&String> = params.iter().filter(|(_, t)| t.as_str() == ty).map(|(n, _)| n).collect();
                    if let [one] = holders.as_slice() {
                        receivers.push(Receiver {
                            ty: ty.to_string(),
                            instance: format!("self.{one}"),
                            explicit: false,
                            pool: instance_pool(one),
                        });
                    }
                }
                let int = |e: Option<&Expr>| match e {
                    Some(Expr::Literal(hale_syntax::ast::Literal::Int(n), _)) => Some(*n),
                    _ => None,
                };
                serves.push(Serve {
                    surface,
                    transport: site.transport.and_then(|t| transport_of(t, &params)),
                    name: match site.option("as") {
                        Some(Expr::Literal(hale_syntax::ast::Literal::String(s), _)) => Some(s.clone()),
                        _ => None,
                    },
                    receivers,
                    bound: int(site.option("bound")),
                    on_full: match site.option("on_full") {
                        Some(Expr::Ident(i)) => Some(i.name.clone()),
                        _ => None,
                    },
                    serving_locus: l.name.name.clone(),
                    span: site.span,
                });
            }
            // The hubs this locus binds topics to: each a param whose
            // literal is a transport instance.
            for m in &l.members {
                let LocusMember::Bindings(bb) = m else { continue };
                for hb in &bb.hubs {
                    let instance = format!("self.{}", hb.instance.name);
                    let topic = topics.named(&hb.topic.name);
                    let stream = Stream {
                        topic: hb.topic.name.clone(),
                        subject: topic.map_or_else(|| hb.topic.name.clone(), |t| t.wire.clone()),
                        direction: if publishes.contains(&hb.topic.name) { "out" } else { "in" },
                        payload: topic.map(|t| row_ty(&t.payload)),
                        codec: String::new(),
                        bound: hb.bound.map(|(n, _)| n),
                        on_full: hb.on_full.as_ref().map(|i| i.name.clone()),
                        replay: false,
                        requires: hb.requires.iter().map(|i| (i.name.clone(), i.span)).collect(),
                        span: hb.span,
                    };
                    match hubs.iter_mut().find(|h| h.instance == instance) {
                        Some(h) => h.streams.push(Stream { codec: h.transport.codec.clone(), ..stream }),
                        None => {
                            let literal = l.members.iter().find_map(|m| match m {
                                LocusMember::Params(pb) => pb.params.iter().find(|p| p.name.name == hb.instance.name),
                                _ => None,
                            });
                            let transport = match literal.map(|p| &p.init) {
                                Some(ParamInit::Value(e)) => transport_of(e, &params),
                                _ => None,
                            }
                            .unwrap_or(Transport {
                                kind: String::new(),
                                address: None,
                                codec: "json".to_string(),
                                principals: None,
                                roles: None,
                                name: None,
                            });
                            let codec = transport.codec.clone();
                            hubs.push(Hub { instance, transport, streams: vec![Stream { codec, ..stream }], span: hb.span });
                        }
                    }
                }
            }
        }
    }
    let app = entry.entry().map(|m| m.name.clone());
    SurfaceRows { surfaces, rows, serves, hubs, app }
}

/// What a build refuses: a topic bound to a hub is named and described
/// (spec/api.md § Streams) and served by nothing until R5, so a build that
/// lowered the program would drop it silently. (A serve site is served
/// from R2a: `rpc_expand` builds its exposure.) A surface nobody serves
/// builds: it is a table nobody reads.
pub fn unserved_sites(programs: &[&Program]) -> Vec<Diag> {
    let mut diags = Vec::new();
    for p in programs {
        for d in flat_decls(&p.items) {
            let TopDecl::Locus(l) = d else { continue };
            // A serve site whose exposure the expansion could not build:
            // over a transport this compiler does not ship yet.
            for site in locus_serve_sites(l) {
                let Some(Expr::Literal(hale_syntax::ast::Literal::String(name), _)) = site.option("as") else { continue };
                let built = l.members.iter().any(|m| {
                    matches!(m, LocusMember::Params(pb)
                        if pb.params.iter().any(|p| p.name.name == crate::rpc_expand::exposure_param(name)))
                });
                if built {
                    continue;
                }
                let transport = match site.transport {
                    Some(Expr::Struct { path, .. }) => {
                        path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")
                    }
                    _ => String::new(),
                };
                if transport.is_empty() {
                    continue;
                }
                diags.push(Diag::ty(
                    site.span,
                    format!(
                        "`api::serve` over `{transport}`: this compiler serves a surface over \
                         `std::api::test::Rpc`, the in-process transport, or a transport the program declares; the \
                         socket transports follow (spec/api.md § The `Rpc` interface)"
                    ),
                ));
            }
            for m in &l.members {
                let LocusMember::Bindings(bb) = m else { continue };
                for hb in &bb.hubs {
                    diags.push(Diag::ty(
                        hb.span,
                        format!(
                            "`{}` is bound to the hub `self.{}`, which is described (`hale check --api`) but not yet \
                             served: this compiler does not lower a hub binding, so the program cannot be built \
                             (spec/api.md § Streams)",
                            hb.topic.name, hb.instance.name
                        ),
                    ));
                }
            }
        }
    }
    diags
}

// ---- the admission law (spec/api.md § Surfaces and their rows) ----

/// A count as a word, for a message.
fn count_word(n: usize) -> String {
    const WORDS: [&str; 10] = ["no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
    WORDS.get(n).map_or_else(|| n.to_string(), |w| w.to_string())
}

fn did_you_mean<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> String {
    crate::stdlib_surface::nearest_name(name, candidates).map_or_else(String::new, |n| format!("; did you mean `{n}`?"))
}

/// Why a type a row names has no JSON form (spec/api.md § Codecs: `Int`,
/// `Float`, `Bool`, `String` and nested structs of the same; an
/// identity, a range, a quantity and a point as their integer): the
/// field path from the row's type to what the codec does not carry,
/// and that type as written. `None` when every field has a form.
fn json_refusal(
    shapes: &Shapes<'_>,
    te: &TypeExpr,
    path: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) -> Option<(Vec<String>, String)> {
    use crate::topic_identity::TypeClass;
    use hale_syntax::ast::PrimType;
    let carried = |p: PrimType| matches!(p, PrimType::Int | PrimType::Float | PrimType::Bool | PrimType::String);
    match shapes.classify(te) {
        TypeClass::Prim(p) if carried(p) => None,
        // An identity, a range and a quantity are their `Int`; a point is
        // not a count (its origin is not on the wire), so R2a's codec does
        // not carry one.
        TypeClass::Named { base, unit, .. }
            if carried(base) && !unit.as_deref().is_some_and(|u| u.contains(" point")) =>
        {
            None
        }
        TypeClass::Builtin(_) => None,
        TypeClass::Struct { name, fields } => {
            if !seen.insert(name.to_string()) {
                return None;
            }
            for f in fields {
                path.push(f.name.name.clone());
                if let Some(r) = json_refusal(shapes, &f.ty, path, seen) {
                    return Some(r);
                }
                path.pop();
            }
            None
        }
        _ => Some((path.clone(), type_text(te))),
    }
}

/// Law 6, the statement of a row whose error type is
/// `ClosureViolation`: what `hale check --api` prints beside the member,
/// for each such row, by surface and member.
pub fn server_error_notes(rows: &SurfaceRows) -> Vec<(String, String, String)> {
    rows.rows
        .iter()
        .filter(|r| matches!(&r.handler, Handled::Fn(h) if h.server_error))
        .map(|r| {
            (
                r.surface.clone(),
                r.member.clone(),
                format!(
                    "rpc `{}`: its error type is `ClosureViolation`, so a failure is the server error; a \
                     description carries no error schema for it",
                    r.member
                ),
            )
        })
        .collect()
}

/// The admission law over the rows (spec/api.md § Surfaces and their
/// rows), each in its wording, in the order the rows were written:
///
/// 1. a row names a handler: a member fn of a declared locus, never a
///    lifecycle method, a mode or `on_failure`, and `@rpc` sits on a
///    locus fn;
/// 2. a handler takes one request;
/// 3. a member is one row of its surface;
/// 4. a required role is declared;
/// 5. every shape has a codec form: the request, response and error
///    types are carried by the JSON codec, the one codec a serve site
///    has in R1 (a `ClosureViolation` error carries no schema, so no
///    form is asked of it);
/// 7. a handler that may violate is `fallible(ClosureViolation)`
///    whatever it returns: F.42 refuses a value-returning one that is
///    not, and law 7 adds the one returning nothing, whose violation a
///    remote caller could not otherwise see. The may-violate judgment is
///    F.42's (`violate_fallible::may_violate`, over the summary).
///
/// Law 6 is a statement of the row, not a refusal
/// ([`server_error_notes`]). The serve sites' laws are R2's: a serve
/// site is named, not checked.
pub fn surface_laws(
    bundle: &Bundle<'_>,
    rows: &SurfaceRows,
    roles: &crate::roles::RoleRows,
    summary: &crate::alloc_summary::AllocSummary,
) -> Vec<Diag> {
    if rows.rows.is_empty() {
        return Vec::new();
    }
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let slices: Vec<&[TopDecl]> = programs.iter().map(|p| p.items.as_slice()).collect();
    let shapes = Shapes::of_all(&slices);
    let decls = crate::violate_fallible::fn_decl_rows(&programs);
    let may = crate::violate_fallible::may_violate(summary, &decls);
    let declared: BTreeSet<&str> = roles.roles.iter().map(|r| r.name.name.as_str()).collect();

    let mut written: Vec<&Row> = rows.rows.iter().collect();
    written.sort_by_key(|r| r.written_at);
    let mut first_of: BTreeMap<(&str, &str), Span> = BTreeMap::new();
    let mut diags = Vec::new();
    for r in written {
        let m = &r.member;
        // Law 3: a member is one row of its surface.
        if let Some(first) = first_of.get(&(r.surface.as_str(), m.as_str())) {
            diags.push(
                Diag::ty(
                    r.span,
                    format!(
                        "rpc `{m}` is in `{}` twice: a member is one row; keep one, with the roles it requires",
                        r.surface
                    ),
                )
                .with_related(*first, "its first row"),
            );
        } else {
            first_of.insert((r.surface.as_str(), m.as_str()), r.span);
        }
        // Law 4: a required role is declared.
        for (role, span) in &r.requires {
            if !declared.contains(role.as_str()) {
                diags.push(Diag::ty(
                    *span,
                    format!(
                        "rpc `{m}` requires `{role}`, which no `role` declares{}",
                        did_you_mean(role, declared.iter().copied())
                    ),
                ));
            }
        }
        // Law 1: a row names a handler.
        let locus_written = m.rsplit_once("::").map_or(m.as_str(), |(l, _)| l);
        let h = match &r.handler {
            Handled::Fn(h) => h,
            Handled::NoLocus { loci } => {
                diags.push(Diag::ty(
                    r.span,
                    format!(
                        "rpc `{m}`: no locus `{locus_written}` is declared{}",
                        did_you_mean(&r.locus, loci.iter().map(String::as_str))
                    ),
                ));
                continue;
            }
            Handled::NoFn { fns } => {
                diags.push(Diag::ty(
                    r.span,
                    format!(
                        "rpc `{m}`: `{locus_written}` declares no fn `{}`{}",
                        r.method,
                        did_you_mean(&r.method, fns.iter().map(String::as_str))
                    ),
                ));
                continue;
            }
            Handled::NotAHandler { kind } => {
                diags.push(Diag::ty(
                    r.span,
                    format!(
                        "rpc `{m}`: `{}` is a {kind} of `{locus_written}`, and a handler is a member fn",
                        r.method
                    ),
                ));
                continue;
            }
            Handled::FreeFn => {
                diags.push(Diag::ty(
                    r.span,
                    format!("`@rpc` on `{m}`: a handler is a member fn of a locus, and `{m}` is a free fn"),
                ));
                continue;
            }
        };
        // Law 2: a handler takes one request.
        if h.params.len() > 1 {
            diags.push(
                Diag::ty(
                    r.span,
                    format!(
                        "rpc `{m}`: a handler takes its request as one parameter, and `{}` takes {}: declare a \
                         type for the request",
                        r.method,
                        count_word(h.params.len())
                    ),
                )
                .with_related(h.name_span, "the handler"),
            );
        }
        // Law 5: every shape has a codec form.
        let slots = [("request", h.request.as_ref()), ("response", h.response.as_ref())]
            .into_iter()
            .chain((!h.server_error).then_some(("error", h.error.as_ref())));
        for (slot, ty) in slots {
            let Some(ty) = ty else { continue };
            if let Some((path, at)) = json_refusal(&shapes, &ty.te, &mut Vec::new(), &mut BTreeSet::new()) {
                let message = if path.is_empty() {
                    format!("rpc `{m}`: its {slot} is `{at}`, which the JSON codec does not carry")
                } else {
                    format!(
                        "rpc `{m}`: its {slot} `{}` has a field `{}: {at}`, which the JSON codec does not carry",
                        ty.display,
                        path.join(".")
                    )
                };
                diags.push(Diag::ty(r.span, message).with_related(h.name_span, "the handler"));
            }
        }
        // Law 7: a handler that may violate is `fallible(ClosureViolation)`,
        // whatever it returns.
        if h.response.is_none() && h.error.is_none() && may.contains_key(&h.key) {
            let how = crate::violate_fallible::how(&h.key, &may, &decls);
            diags.push(
                Diag::ty(
                    r.span,
                    format!(
                        "rpc `{m}` may violate ({how}) and returns nothing: an rpc handler that may violate is \
                         `fallible(ClosureViolation)`, so its caller receives the server error instead of a result"
                    ),
                )
                .with_related(h.name_span, "the handler"),
            );
        }
    }
    diags
}

// ---- the laws of a serve site (spec/api.md § Serving) ----

/// The name each `let h = api::serve(…)` binds, by the site's start.
fn let_handles(b: &Block, out: &mut BTreeMap<usize, String>) {
    for s in &b.stmts {
        match s {
            Stmt::Let { name, value, .. } => {
                if let Some(site) = ServeSite::of(value) {
                    out.insert(site.span.start.0 as usize, name.name.clone());
                }
            }
            Stmt::If(i) => {
                let_handles(&i.then_block, out);
                let mut e = i.else_block.as_deref();
                while let Some(branch) = e {
                    match branch {
                        ElseBranch::Else(b) => {
                            let_handles(b, out);
                            e = None;
                        }
                        ElseBranch::ElseIf(i) => {
                            let_handles(&i.then_block, out);
                            e = i.else_block.as_deref();
                        }
                    }
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => let_handles(body, out),
            Stmt::Block(b) => let_handles(b, out),
            _ => {}
        }
    }
}

/// The laws of a serve site (spec/api.md § Serving), each in its wording:
///
/// 1. an exposure is named once;
/// 2. every receiver type is bound: to the one instance the serving locus
///    holds, or by `receivers:`;
/// 3. a receiver outlives its exposure: a param of the serving locus,
///    built by a literal, never a `let`-bound child;
/// 4. a bound type is one the rows name;
/// 5. a serve site states its queue: `bound:` and `on_full: refuse`.
///
/// And what a site needs to be served at all: a surface that is declared,
/// a name (`as:`), a transport instance, a place in a locus's body.
pub fn serve_laws(bundle: &Bundle<'_>, rows: &SurfaceRows) -> Vec<Diag> {
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let mut diags = Vec::new();
    let mut first_named: BTreeMap<String, Span> = BTreeMap::new();
    let surface_names: Vec<&str> = rows.surfaces.iter().map(|s| s.name.as_str()).collect();
    for p in &programs {
        for d in flat_decls(&p.items) {
            match d {
                TopDecl::Fn(f) => {
                    let mut out = Vec::new();
                    serve_sites_in(&f.body, &mut out);
                    for s in out {
                        diags.push(Diag::ty(
                            s.span,
                            format!(
                                "`api::serve` in `{}`: a serve site belongs to a locus's body, since its exposure is \
                                 a param of the serving locus; serve from the locus that holds the receivers",
                                f.name.name
                            ),
                        ));
                    }
                }
                TopDecl::Locus(l) => serve_laws_of(l, rows, &surface_names, &mut first_named, &mut diags),
                _ => {}
            }
        }
    }
    diags
}

fn serve_laws_of(
    l: &LocusDecl,
    rows: &SurfaceRows,
    surface_names: &[&str],
    first_named: &mut BTreeMap<String, Span>,
    diags: &mut Vec<Diag>,
) {
    let sites = locus_serve_sites(l);
    if sites.is_empty() {
        return;
    }
    let mut handles: BTreeMap<usize, String> = BTreeMap::new();
    for m in &l.members {
        match m {
            LocusMember::Lifecycle(lc) => let_handles(&lc.body, &mut handles),
            LocusMember::Fn(f) => let_handles(&f.body, &mut handles),
            LocusMember::Mode(md) => let_handles(&md.body, &mut handles),
            _ => {}
        }
    }
    let params = param_types(l);
    let literal_params: BTreeSet<&str> = l
        .members
        .iter()
        .filter_map(|m| match m {
            LocusMember::Params(pb) => Some(pb.params.iter()),
            _ => None,
        })
        .flatten()
        .filter(|p| matches!(&p.init, ParamInit::Value(Expr::Struct { .. })))
        .map(|p| p.name.name.as_str())
        .collect();
    for site in sites {
        let surface = match site.surface {
            Expr::Ident(i) => Some(i.name.clone()),
            Expr::Path(qn) => Some(qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")),
            _ => None,
        };
        let Some(surface) = surface else { continue };
        let name = match site.option("as") {
            Some(Expr::Literal(hale_syntax::ast::Literal::String(s), _)) => Some(s.clone()),
            _ => None,
        };
        if !surface_names.contains(&surface.as_str()) {
            diags.push(Diag::ty(
                site.span,
                format!(
                    "serve of `{surface}`: no surface `{surface}` is declared{}",
                    did_you_mean(&surface, surface_names.iter().copied())
                ),
            ));
            continue;
        }
        if site.transport.is_none() {
            diags.push(Diag::ty(
                site.span,
                format!("serve of `{surface}`: a serve site names the transport instance that carries it"),
            ));
        }
        let Some(name) = name else {
            diags.push(Diag::ty(
                site.span,
                format!("serve of `{surface}`: a serve site names its exposure with `as:`"),
            ));
            continue;
        };
        // Law 1: an exposure is named once.
        if let Some(first) = first_named.get(&name) {
            diags.push(
                Diag::ty(
                    site.span,
                    format!("exposure `{name}` is served twice: `as:` names one exposure; name this one apart"),
                )
                .with_related(*first, "its first serve"),
            );
        } else {
            first_named.insert(name.clone(), site.span);
        }
        // Law 5: a serve site states its queue.
        let bounded = matches!(site.option("bound"), Some(Expr::Literal(hale_syntax::ast::Literal::Int(n), _)) if *n > 0);
        let refuses = matches!(site.option("on_full"), Some(Expr::Ident(i)) if i.name == "refuse");
        if !bounded || !refuses {
            diags.push(Diag::ty(
                site.span,
                format!(
                    "serve of `{surface}` as `{name}`: a serve site states `bound:`, the requests it holds accepted \
                     and not yet answered, and `on_full: refuse`, the one policy for a request"
                ),
            ));
        }
        // The types the surface's rows name, in the order written.
        let mut named: Vec<(usize, &str)> = rows
            .rows
            .iter()
            .filter(|r| r.surface == surface && !r.locus.is_empty())
            .map(|r| (r.written_at, r.locus.as_str()))
            .collect();
        named.sort();
        let mut row_types: Vec<&str> = Vec::new();
        for (_, t) in named {
            if !row_types.contains(&t) {
                row_types.push(t);
            }
        }
        // Law 4: a bound type is one the rows name.
        let is_row_type = |written: &str| {
            row_types.iter().any(|t| *t == written || written.rsplit("::").next().is_some_and(|tail| tail == *t))
        };
        for i in site.receivers() {
            if !is_row_type(&i.name.name) {
                diags.push(Diag::ty(
                    i.span,
                    format!(
                        "serve of `{surface}`: `receivers:` binds `{}`, which no row of `{surface}` names",
                        i.name.name
                    ),
                ));
            }
        }
        // Laws 2 and 3, per receiver type the rows name.
        for ty in &row_types {
            let ty: &str = ty;
            let bound_here = site.receivers().iter().find(|i| {
                i.name.name == ty || i.name.name.rsplit("::").next().is_some_and(|tail| tail == ty)
            });
            let instance: Option<(&Expr, Span)> = bound_here.map(|i| (&i.value, i.span));
            let Some((expr, at)) = instance else {
                let holders: Vec<&String> = params.iter().filter(|(_, t)| t.as_str() == ty).map(|(n, _)| n).collect();
                match holders.as_slice() {
                    [one] => {
                        if !literal_params.contains(one.as_str()) {
                            diags.push(Diag::ty(
                                site.span,
                                format!(
                                    "serve of `{surface}`: `{one}` is not built by a literal in a param of `{}`: the \
                                     serve numbers the instance in the literal that builds it",
                                    l.name.name
                                ),
                            ));
                        }
                    }
                    [] => diags.push(Diag::ty(
                        site.span,
                        format!(
                            "serve of `{surface}` as `{name}`: `{ty}` is held by no param of `{}`, and the serve binds \
                             none: bind the instance that answers (`receivers: {{ {ty}: self.<param> }}`)",
                            l.name.name
                        ),
                    )),
                    many => {
                        let spelled: Vec<String> = many.iter().map(|h| format!("`self.{h}`")).collect();
                        let list = match spelled.as_slice() {
                            [a, b] => format!("{a} and {b}"),
                            [init @ .., last] => format!("{} and {last}", init.join(", ")),
                            [] => String::new(),
                        };
                        diags.push(Diag::ty(
                            site.span,
                            format!(
                                "serve of `{surface}` as `{name}`: `{ty}` is held {}, as {list}, and the serve binds \
                                 {}: name the one that answers (`receivers: {{ {ty}: self.{} }}`)",
                                if many.len() == 2 { "twice".to_string() } else { format!("{} times", count_word(many.len())) },
                                if many.len() == 2 { "neither".to_string() } else { "none of them".to_string() },
                                many[0]
                            ),
                        ));
                    }
                }
                continue;
            };
            // Law 3: a receiver outlives its exposure.
            match self_field(expr) {
                Some(field) if params.contains_key(field) => {
                    if !literal_params.contains(field) {
                        diags.push(Diag::ty(
                            at,
                            format!(
                                "serve of `{surface}`: `{field}` is not built by a literal in a param of `{}`: the \
                                 serve numbers the instance in the literal that builds it",
                                l.name.name
                            ),
                        ));
                    }
                }
                Some(field) => diags.push(Diag::ty(
                    at,
                    format!(
                        "serve of `{surface}`: `self.{field}` is no param of `{}`; hold the receiver as a param",
                        l.name.name
                    ),
                )),
                None => {
                    let local = match expr {
                        Expr::Ident(i) => Some(i.name.clone()),
                        _ => None,
                    };
                    let stop = handles
                        .get(&(site.span.start.0 as usize))
                        .map_or_else(|| "its `stop()`".to_string(), |h| format!("`{h}.stop()`"));
                    match local {
                        Some(local) => diags.push(Diag::ty(
                            at,
                            format!(
                                "serve of `{surface}`: `{local}` is `let`-bound and dissolves at the end of this block, \
                                 before {stop}; hold it as a param"
                            ),
                        )),
                        None => diags.push(Diag::ty(
                            at,
                            format!(
                                "serve of `{surface}`: the receiver of `{ty}` is no param of `{}`; hold it as a param \
                                 and bind it as `self.<param>`",
                                l.name.name
                            ),
                        )),
                    }
                }
            }
        }
    }
}

// ---- the schemas a description carries (spec/api.md § The description) ----

/// A field's, a member's or a stream's JSON Schema in a description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldSchema {
    /// `{"type": …}`, with a named scalar's `x-hale-type` and a
    /// quantity's or a point's `x-hale-unit` (its `q(…)` tag).
    Scalar { json: &'static str, hale_type: Option<String>, unit: Option<String> },
    /// `{"$ref": "#/schemas/<name>"}`: a struct, its schema in the
    /// document's `schemas`.
    Ref(String),
    /// A type the codec does not carry (law 5 refuses its row).
    Unformed,
}

/// A struct's schema: its properties in declaration order, each its
/// JSON key (a `json:"key"` tag renames it) and schema, and the keys a
/// value must carry (every field without a literal default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSchema {
    pub properties: Vec<(String, FieldSchema)>,
    pub required: Vec<String>,
}

/// The schemas of the types a bundle declares, read as the JSON codec
/// reads them (law 5's classification).
pub struct Schemas<'a> {
    shapes: Shapes<'a>,
}

impl<'a> Schemas<'a> {
    pub fn of(programs: &[&'a Program]) -> Schemas<'a> {
        let slices: Vec<&'a [TopDecl]> = programs.iter().map(|p| p.items.as_slice()).collect();
        Schemas { shapes: Shapes::of_all(&slices) }
    }

    /// The schema `te` is in a description, every struct schema it
    /// reaches added to `out` by name.
    pub fn type_ref(&self, te: &TypeExpr, out: &mut BTreeMap<String, TypeSchema>) -> FieldSchema {
        use crate::topic_identity::TypeClass;
        use hale_syntax::ast::PrimType;
        let json = |p: PrimType| match p {
            PrimType::Int => Some("integer"),
            PrimType::Float => Some("number"),
            PrimType::Bool => Some("boolean"),
            PrimType::String => Some("string"),
            _ => None,
        };
        match self.shapes.classify(te) {
            TypeClass::Prim(p) => {
                json(p).map_or(FieldSchema::Unformed, |j| FieldSchema::Scalar { json: j, hale_type: None, unit: None })
            }
            TypeClass::Named { name, base, unit } => json(base).map_or(FieldSchema::Unformed, |j| FieldSchema::Scalar {
                json: j,
                hale_type: Some(name.to_string()),
                unit,
            }),
            TypeClass::Struct { name, fields } => {
                if !out.contains_key(name) {
                    // Reserve the name first, so a type that reaches
                    // itself stops here.
                    out.insert(name.to_string(), TypeSchema { properties: Vec::new(), required: Vec::new() });
                    let mut properties = Vec::new();
                    let mut required = Vec::new();
                    for f in fields {
                        let key = f
                            .tag
                            .as_deref()
                            .and_then(|t| hale_syntax::desugar::tag_value(t, "json"))
                            .unwrap_or_else(|| f.name.name.clone());
                        properties.push((key.clone(), self.type_ref(&f.ty, out)));
                        if f.default.is_none() {
                            required.push(key);
                        }
                    }
                    out.insert(name.to_string(), TypeSchema { properties, required });
                }
                FieldSchema::Ref(name.to_string())
            }
            TypeClass::Builtin(name) => {
                // A builtin record is its fields, as the contract shape
                // renders them; a user declaration of the name wins
                // (`classify` sees it as a struct first).
                if !out.contains_key(name) {
                    let mut properties = Vec::new();
                    let mut required = Vec::new();
                    for (f, ty) in self.shapes.builtin_fields(te) {
                        properties.push((f.clone(), self.type_ref(&ty, out)));
                        required.push(f);
                    }
                    out.insert(name.to_string(), TypeSchema { properties, required });
                }
                FieldSchema::Ref(name.to_string())
            }
            TypeClass::Enum | TypeClass::Other => FieldSchema::Unformed,
        }
    }
}
