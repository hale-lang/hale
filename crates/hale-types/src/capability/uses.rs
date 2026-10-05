//! The use producer and the admission law (F.40 phase 3, P3 2 of 3;
//! `notes/f40-capability-matrix.md` §1.4): every way a program asks its
//! target for a capability, as a use row with the chain that witnesses
//! it, and the refusal of every use whose cell is `Reject` on the
//! effective target.
//!
//! **Resolved identities.** The operational uses are read off the
//! resolved call graph, the allocation summary the snapshot already
//! holds (the program's bodies, the imported seeds' bodies under their
//! merged names, and the stdlib's analysis copy): a call resolved to a
//! fn, a method resolved through its receiver's declared type, a
//! cross-seed call resolved through the import renames, a construction
//! of a locus. A codegen-native stdlib primitive has no body; its call
//! keeps its absolute `std::` path, which names a namespace of the
//! stdlib's own table ([`std_namespace`]), the one identity a primitive
//! has.
//!
//! **Requirement summaries.** Each fn's requirements are computed once,
//! as a fixpoint over the graph: the namespaces of the primitives it
//! calls, the requirements of the fns it calls, and the lifecycle of
//! every locus it constructs (its `birth`, `run`, handlers and
//! `dissolve`), each with the chain from the fn down to the primitive.
//! What a locus beyond the horizon runs that the summary keys no body
//! for — each params initializer and its `on_failure` handler — is a
//! node of the graph too, walked by the graph's own walk, so its
//! construction carries those requirements as well.
//!
//! **The horizon.** The program's own sources are the horizon: a use is
//! located at the first site in them. A wrapper the program writes is
//! judged at the use in its own body, once, and its callers carry
//! nothing; a fn beyond the horizon (an imported seed's, the stdlib's)
//! is judged at the call that crosses into it, with its summary, and a
//! locus beyond it at its construction. The horizon relocates a refusal;
//! it never erases a requirement.
//!
//! **Holes.** A call the graph cannot resolve — a method on a receiver
//! whose type the walk cannot name, a call through a function-typed
//! parameter, a computed callee, or (in the program's own code and in a
//! member body beyond the horizon) a call through a local function value
//! whose binding the walk cannot follow to a fn — leaves the use's
//! requirements unknown. On a target whose
//! column rejects anything in the stdlib family (wasm32) that is a
//! refusal; elsewhere it is a recorded hole, counted, never silent. A
//! call through a local the summary follows to a fn (`let f = pid;
//! f()`) is a call of that fn, the local a link of its witness.
//!
//! **Type-only mentions** are not uses: a parameter, field or return
//! type naming `std::io::tcp::Stream` asks nothing of the target.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    flat_decls, Block, ElseBranch, Expr, IfStmt, LValueSeg, LifecycleKind, LocusDecl, LocusMember, MatchArmBody,
    MatchStmt, OrDisposition, ParamInit, Pattern, PlacementConstraint, PlacementSpec, Program, QualifiedName, Stmt,
    TopDecl, TransportSpec, TypeExpr,
};
use hale_syntax::{Diag, Span};

use super::{
    Abi, BehaviourVerdict, Capability, CapabilityMatrix, Inversion, KnownOpen, OpenCell, Origin,
    TargetClass, TargetRow, Transport, KNOWN_OPEN,
};
use crate::alloc_summary::{loop_reassigned, AllocKind, AllocSummary, CallEdge, CallSpelling, Callee, DeclId, FnKey};
use crate::placement::SiteUniverse;

/// What a use asks for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Need {
    Capability(Capability),
    /// Requirements the graph cannot establish, and why.
    Hole(&'static str),
}

/// How a program reaches the capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UseKind {
    /// A call whose written callee is a stdlib path.
    Call,
    /// A method call on a handle, resolved through the receiver's type.
    Receiver,
    /// A call to a fn beyond the horizon: an imported seed's, the
    /// stdlib's.
    Crossing,
    /// A construction of a locus beyond the horizon: its lifecycle runs.
    Construction,
    /// A `placement { }` entry.
    Placement,
    /// A `bindings { }` entry.
    Binding,
    /// An `@export fn` or `@export locus`.
    Export,
    /// A program with `@export`s and no `fn main`.
    ExportOnly,
    /// An `@export locus` that writes `run()`.
    ExportedRun,
    /// An `@ffi("…")` declaration.
    ForeignDecl,
}

/// One use row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityUse {
    pub need: Need,
    pub kind: UseKind,
    /// The first site in the program's own sources.
    pub span: Span,
    /// The witness chain, from the written use down to the primitive
    /// (`std::io::tcp::Listener` → `birth()` → `std::io::tcp::__listen_socket`),
    /// as the graph names each link (merged and stdlib names are
    /// demangled when a diagnostic renders them).
    pub chain: Vec<String>,
    /// The use's own holes in the cell's wording (`field`, `locus`).
    pub holes: Vec<(&'static str, String)>,
}

/// The use rows of a program.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityUses {
    pub uses: Vec<CapabilityUse>,
}

impl CapabilityUses {
    /// The uses whose requirements the graph could not establish.
    pub fn holes(&self) -> impl Iterator<Item = &CapabilityUse> {
        self.uses.iter().filter(|u| matches!(u.need, Need::Hole(_)))
    }
}

/// The namespace an absolute stdlib path lies in: the longest
/// namespace of the stdlib's table (the matrix's `StdNamespace` keys,
/// which the laws hold to the stdlib's own namespaces) that holds the
/// path's last segment. `None` for a path that is not the stdlib's.
pub fn std_namespace(m: &CapabilityMatrix, path: &str) -> Option<&'static str> {
    let segs: Vec<&str> = path.split("::").collect();
    if segs.len() < 3 || segs[0] != "std" {
        return None;
    }
    let inner = &segs[1..segs.len() - 1];
    m.behaviours
        .iter()
        .filter_map(|r| match r.capability {
            Capability::StdNamespace(ns) => Some(ns),
            _ => None,
        })
        .filter(|ns| {
            let n: Vec<&str> = ns.split("::").collect();
            inner.len() >= n.len() && inner[..n.len()] == n[..]
        })
        .max_by_key(|ns| ns.split("::").count())
}

/// One fn's requirements: per capability, the chain from the fn's body
/// down to the primitive; and the first hole the walk met, with its
/// chain.
#[derive(Debug, Clone, Default)]
struct Req {
    caps: BTreeMap<Capability, Vec<String>>,
    hole: Option<(Vec<String>, &'static str)>,
}

/// The reason a call edge is a hole, if it is one.
fn hole_of(e: &CallEdge) -> Option<&'static str> {
    if e.via_interface.is_some() {
        // A dispatch through an interface no locus conforms to: dead,
        // not unknown (`CallEdge::via_interface`).
        return None;
    }
    // An indirect call the summary resolved to the program's function
    // values is its alternatives (`CallEdge::via_value`), never a hole.
    if e.indirect && e.through_param {
        return Some("it is called through a function-typed parameter, whose target is not known here");
    }
    // A computed callee (`pick()(x)`), worded as the member walk words it.
    if e.indirect && e.spelling == CallSpelling::Expr {
        return Some(UNRESOLVED_VALUE);
    }
    if e.opaque_method_call() {
        return Some("its receiver's type is not known here");
    }
    // A local still unresolved after function-value resolution is a
    // hole in every body, the program's own and an imported one alike.
    if e.indirect && e.unresolved_local {
        return Some(UNRESOLVED_VALUE);
    }
    None
}

struct Graph<'a> {
    summary: &'a AllocSummary,
    m: &'a CapabilityMatrix,
    /// `alias::Name` → the merged name, the bundle's import renames.
    renames: BTreeMap<String, String>,
    /// The stdlib's locus names, by their public path's namespace.
    std_loci: BTreeMap<String, &'static str>,
    /// Merged and stdlib names to the spelling the author writes.
    demangle: crate::stdlib_bodies::Demangler,
    /// Per locus beyond the horizon, what its existence runs that the
    /// summary keys no body for — each params initializer and its
    /// `on_failure` handler — as a node of the graph: its key, its link
    /// and what the graph's own walk met in it.
    members: BTreeMap<String, Vec<(FnKey, String, Vec<Met>)>>,
}

impl<'a> Graph<'a> {
    fn new(
        summary: &'a AllocSummary,
        m: &'a CapabilityMatrix,
        import_renames: &[(Vec<String>, String)],
        programs: &[&Program],
    ) -> Self {
        let renames = import_renames.iter().map(|(k, v)| (k.join("::"), v.clone())).collect();
        let std_loci = hale_stdlib::PATH_RENAMES
            .iter()
            .filter_map(|(path, mangled)| std_namespace(m, &path.join("::")).map(|ns| (mangled.to_string(), ns)))
            .collect();
        let demangle = crate::stdlib_bodies::Demangler::new(import_renames);
        let mut g = Graph { summary, m, renames, std_loci, demangle, members: BTreeMap::new() };
        // The imported seeds' loci are in the bundle under their merged
        // names; the stdlib's are its analysis copy's.
        let stdlib = crate::stdlib_bodies::program().filter(|_| !summary.analysis_copy_loci.is_empty());
        let mut members = BTreeMap::new();
        let universes = programs
            .iter()
            .map(|p| (*p, SiteUniverse::User))
            .chain(stdlib.map(|p| (p, SiteUniverse::StdlibAnalysis)));
        for (p, universe) in universes {
            for item in flat_decls(&p.items) {
                let TopDecl::Locus(l) = item else { continue };
                if g.own_locus(&l.name.name) || members.contains_key(&l.name.name) {
                    continue;
                }
                members.insert(l.name.name.clone(), g.member_nodes(l, universe));
            }
        }
        g.members = members;
        g
    }

    /// The nodes of a locus beyond the horizon that the summary keys no
    /// body for. The walk resolves a method through the declared types
    /// it can name (`self`, a params field, the handler's params, a
    /// literal), and a call through a local to the fn the local is bound
    /// to (`let f = pid; f()` calls `pid`); a receiver it cannot type, or
    /// a function value it cannot resolve (a field, a call's result, a
    /// reassigned local), leaves a hole, so a requirement it cannot
    /// establish stays a refusal on a target that rejects one.
    fn member_nodes(&self, l: &LocusDecl, universe: SiteUniverse) -> Vec<(FnKey, String, Vec<Met>)> {
        let locus = l.name.name.clone();
        let mut fields = BTreeMap::new();
        for member in &l.members {
            if let LocusMember::Params(pb) = member {
                for prm in &pb.params {
                    let ty = match (&prm.ty, &prm.init) {
                        (Some(t), _) => type_name(t),
                        (None, ParamInit::Value(Expr::Struct { path, .. })) => Some(qualified(path)),
                        (None, _) => None,
                    };
                    fields.insert(prm.name.name.clone(), ty);
                }
            }
        }
        let mut out = Vec::new();
        for member in &l.members {
            match member {
                LocusMember::Params(pb) => {
                    for prm in &pb.params {
                        if let ParamInit::Value(e) = &prm.init {
                            let mut w = Walker::beyond(self, &locus, &fields, BTreeMap::new());
                            w.expr(e);
                            let link = format!("params {{ {} }}", prm.name.name);
                            let key = FnKey::method(DeclId::of(universe, prm.id), locus.clone(), link.clone());
                            out.push((key, link, w.met));
                        }
                    }
                }
                LocusMember::Failure(f) => {
                    let params = f.params.iter().map(|p| (p.name.name.clone(), type_name(&p.ty))).collect();
                    let mut w = Walker::beyond(self, &locus, &fields, params);
                    w.block(&f.body);
                    let link = "on_failure()".to_string();
                    out.push((FnKey::method(DeclId::of(universe, f.id), locus.clone(), link.clone()), link, w.met));
                }
                _ => {}
            }
        }
        out
    }

    /// A name as the author spells it.
    fn public(&self, name: &str) -> String {
        self.demangle.demangle(name)
    }

    /// Whether a fn is the program's own: neither the stdlib's analysis
    /// copy nor an imported seed's merged decl.
    fn own(&self, k: &FnKey) -> bool {
        !self.summary.analysis_copy.contains(k) && !k.locus.as_deref().is_some_and(merged) && !merged(&k.fn_name)
    }

    fn own_locus(&self, name: &str) -> bool {
        !self.summary.analysis_copy_loci.contains(name) && !merged(name)
    }

    /// The locus a struct literal's joined path names: a stdlib locus by
    /// its declared (mangled) name, an imported one by its merged name.
    fn locus_of(&self, written: &str) -> String {
        let segs: Vec<&str> = written.split("::").collect();
        if let Some(m) = crate::stdlib_bodies::mangled_locus_name(&segs) {
            return m.to_string();
        }
        self.renames.get(written).cloned().unwrap_or_else(|| written.to_string())
    }

    /// The fns that run because a locus exists: its lifecycle hooks and
    /// handlers (every entry the summary keys on it) and, beyond the
    /// horizon, its params initializers and `on_failure` handler (its
    /// [`Graph::members`]). A construction relocates their requirements
    /// to the literal; it never erases one.
    fn implied(&self, locus: &str) -> Vec<(FnKey, String)> {
        let hooks = self
            .summary
            .fns
            .iter()
            .filter(|(k, f)| k.locus.as_deref() == Some(locus) && f.entry.is_some())
            .map(|(k, _)| (k.clone(), format!("{}()", k.fn_name)));
        let members = self.members.get(locus).into_iter().flatten().map(|(k, link, _)| (k.clone(), link.clone()));
        hooks.chain(members).collect()
    }

    /// What one call edge asks for directly, and the fn it reaches.
    fn edge(&self, e: &CallEdge) -> Edge {
        match &e.callee {
            Callee::Resolved(k) => Edge::Calls(k.clone()),
            Callee::Unresolved(name) => {
                if let Some(ns) = std_namespace(self.m, name) {
                    return Edge::Needs(Capability::StdNamespace(ns), name.clone());
                }
                // A method the summary did not find on a stdlib handle (a
                // handle method dispatched to the namespace's free fn):
                // the handle's namespace.
                if let Some(ns) = e.recv_ty.as_deref().and_then(|t| self.std_loci.get(t)) {
                    let t = e.recv_ty.clone().unwrap_or_default();
                    return Edge::Needs(Capability::StdNamespace(ns), format!("{t}::{name}"));
                }
                match hole_of(e) {
                    // A function value's link is its call, as the member
                    // walk writes it (`f()`).
                    Some(why) if why == UNRESOLVED_VALUE => Edge::Hole(format!("{name}()"), why),
                    Some(why) => Edge::Hole(name.clone(), why),
                    None => Edge::Nothing,
                }
            }
        }
    }

    /// Every fn's requirements, and every member node's, as a fixpoint
    /// over the graph.
    fn requirements(&self) -> BTreeMap<FnKey, Req> {
        let mut req: BTreeMap<FnKey, Req> = BTreeMap::new();
        // Each node's out-edges to other nodes, with the links they add.
        let mut out: BTreeMap<FnKey, Vec<(FnKey, Vec<String>)>> = BTreeMap::new();
        for (key, fs) in &self.summary.fns {
            let r = req.entry(key.clone()).or_default();
            let edges = out.entry(key.clone()).or_default();
            for e in &fs.calls {
                match self.edge(e) {
                    Edge::Needs(cap, link) => {
                        r.caps.entry(cap).or_insert_with(|| through(e.via_local.as_deref(), link));
                    }
                    Edge::Hole(name, why) => {
                        if r.hole.is_none() {
                            r.hole = Some((vec![name], why));
                        }
                    }
                    Edge::Calls(k) => {
                        if self.summary.fns.contains_key(&k) {
                            let links = through(e.via_local.as_deref(), k.display());
                            edges.push((k, links));
                        }
                    }
                    Edge::Nothing => {}
                }
            }
            for site in &fs.sites {
                if let AllocKind::StructLit(written) = &site.kind {
                    let locus = self.locus_of(written);
                    for (hook, link) in self.implied(&locus) {
                        edges.push((hook, vec![locus.clone(), link]));
                    }
                }
            }
        }
        for (key, _, met) in self.members.values().flatten() {
            let r = req.entry(key.clone()).or_default();
            let edges = out.entry(key.clone()).or_default();
            for m in met {
                match m {
                    Met::Needs(cap, links, _) => {
                        r.caps.entry(*cap).or_insert_with(|| links.clone());
                    }
                    Met::Hole(name, why, _) => {
                        if r.hole.is_none() {
                            r.hole = Some((vec![name.clone()], why));
                        }
                    }
                    Met::Calls(k, links, _) => {
                        if self.summary.fns.contains_key(k) {
                            edges.push((k.clone(), links.clone()));
                        }
                    }
                    Met::Constructs(written, _) => {
                        let locus = self.locus_of(written);
                        for (hook, link) in self.implied(&locus) {
                            edges.push((hook, vec![locus.clone(), link]));
                        }
                    }
                }
            }
        }
        loop {
            let mut changed = false;
            for (key, edges) in &out {
                for (callee, links) in edges {
                    let Some(from) = req.get(callee).cloned() else { continue };
                    let r = req.get_mut(key).expect("every node has a row");
                    for (cap, chain) in from.caps {
                        if !r.caps.contains_key(&cap) {
                            r.caps.insert(cap, links.iter().cloned().chain(chain).collect());
                            changed = true;
                        }
                    }
                    if r.hole.is_none() {
                        if let Some((chain, why)) = from.hole {
                            r.hole = Some((links.iter().cloned().chain(chain).collect(), why));
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                return req;
            }
        }
    }
}

/// What one call edge asks for.
enum Edge {
    /// A primitive's capability, with the primitive's link.
    Needs(Capability, String),
    /// A resolved fn.
    Calls(FnKey),
    /// A callee the graph cannot resolve, and why.
    Hole(String, &'static str),
    /// A builtin, or a method the receiver's type resolves to nothing
    /// the target is asked for.
    Nothing,
}

/// A call's links: the local a call through one is written with, then
/// what it reaches.
fn through(local: Option<&str>, link: String) -> Vec<String> {
    local.map(str::to_string).into_iter().chain([link]).collect()
}

/// Whether a name is an imported seed's merged decl.
fn merged(name: &str) -> bool {
    name.split("::").any(|seg| seg.starts_with("__lib_"))
}

/// The use rows of the programs a bundle holds, over the bundle's
/// allocation summary (`target_capability`'s producer for uses).
pub fn derive_capability_uses(bundle: &crate::Bundle<'_>, summary: &AllocSummary) -> CapabilityUses {
    // The bundle's authoritative summary already includes module-nested
    // bodies and resolved function-value alternatives. Read those rows.
    let m = super::derive_capability_matrix();
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let g = Graph::new(summary, &m, &bundle.import_renames, &programs);
    let req = g.requirements();
    let mut uses = Vec::new();

    // ---- operational uses, in the program's own bodies.
    for (key, fs) in &summary.fns {
        if !g.own(key) {
            continue;
        }
        for e in &fs.calls {
            match g.edge(e) {
                Edge::Needs(cap, link) => uses.push(CapabilityUse {
                    need: Need::Capability(cap),
                    kind: if e.receiver_present { UseKind::Receiver } else { UseKind::Call },
                    span: e.callee_span,
                    chain: through(e.via_local.as_deref(), link),
                    holes: Vec::new(),
                }),
                Edge::Hole(name, why) => uses.push(CapabilityUse {
                    need: Need::Hole(why),
                    kind: if e.receiver_present { UseKind::Receiver } else { UseKind::Call },
                    span: e.callee_span,
                    chain: vec![name],
                    holes: Vec::new(),
                }),
                Edge::Calls(k) if !g.own(&k) => {
                    // A stdlib path that names a Hale-source stdlib fn
                    // (`std::io::file::open`): the written path is the
                    // use, refused as the stdlib gate always worded it;
                    // what else the fn reaches is carried as a crossing.
                    // Through a local, the local is what is written: the
                    // path is a link of the crossing's witness.
                    let written = g.public(&k.display());
                    let direct = (!e.receiver_present && e.via_local.is_none())
                        .then(|| std_namespace(&m, &written))
                        .flatten();
                    if let Some(ns) = direct {
                        uses.push(CapabilityUse {
                            need: Need::Capability(Capability::StdNamespace(ns)),
                            kind: UseKind::Call,
                            span: e.callee_span,
                            chain: vec![written],
                            holes: Vec::new(),
                        });
                    }
                    let kind = if e.receiver_present { UseKind::Receiver } else { UseKind::Crossing };
                    let skip = direct.map(Capability::StdNamespace);
                    crossing(&mut uses, &req, &k, kind, e.callee_span, through(e.via_local.as_deref(), k.display()), skip);
                }
                Edge::Calls(_) | Edge::Nothing => {}
            }
        }
        for site in &fs.sites {
            if let AllocKind::StructLit(written) = &site.kind {
                construction(&mut uses, &g, &req, written, site.span);
            }
        }
    }

    // ---- constructions and calls in what the summary keys no body
    // for: the program's own params initializers and `on_failure`
    // handlers. (A locus beyond the horizon carries its own to every
    // construction of it: `Graph::members`.)
    for p in &programs {
        for item in flat_decls(&p.items) {
            let TopDecl::Locus(l) = item else { continue };
            if !g.own_locus(&l.name.name) {
                continue;
            }
            let mut w = Walker::own(&g);
            for member in &l.members {
                match member {
                    LocusMember::Params(pb) => {
                        for prm in &pb.params {
                            if let ParamInit::Value(e) = &prm.init {
                                w.expr(e);
                            }
                        }
                    }
                    LocusMember::Failure(f) => w.block(&f.body),
                    _ => {}
                }
            }
            for met in w.met {
                match met {
                    Met::Needs(cap, chain, span) => uses.push(CapabilityUse {
                        need: Need::Capability(cap),
                        kind: UseKind::Call,
                        span,
                        chain,
                        holes: Vec::new(),
                    }),
                    Met::Hole(name, why, span) => uses.push(CapabilityUse {
                        need: Need::Hole(why),
                        kind: UseKind::Call,
                        span,
                        chain: vec![name],
                        holes: Vec::new(),
                    }),
                    Met::Calls(k, links, span) if !g.own(&k) => {
                        crossing(&mut uses, &req, &k, UseKind::Crossing, span, links, None)
                    }
                    Met::Constructs(written, span) => construction(&mut uses, &g, &req, &written, span),
                    Met::Calls(..) => {}
                }
            }
        }
    }

    // ---- declaration-level uses.
    let mut exports: Vec<Span> = Vec::new();
    // The entry point is a top-level `fn main` (GH #911), as codegen
    // finds it; an `@export` at any depth is lowered.
    let has_main = programs
        .iter()
        .any(|p| p.items.iter().any(|i| matches!(i, TopDecl::Fn(f) if f.name.name == "main")));
    for p in &programs {
        for item in flat_decls(&p.items) {
            match item {
                TopDecl::Fn(f) => {
                    if f.export {
                        exports.push(f.name.span);
                        uses.push(decl(Capability::ExportSurface, UseKind::Export, f.name.span, &f.name.name, vec![]));
                    }
                    if let Some(a) = &f.ffi {
                        if let Some(abi) = Abi::of(&a.abi) {
                            uses.push(decl(
                                Capability::ForeignAbi(abi),
                                UseKind::ForeignDecl,
                                a.span,
                                &format!("@ffi(\"{}\") fn {}", a.abi, f.name.name),
                                vec![("fn", f.name.name.clone())],
                            ));
                        }
                    }
                }
                TopDecl::Locus(l) => {
                    if l.export {
                        exports.push(l.name.span);
                        uses.push(decl(Capability::ExportSurface, UseKind::Export, l.name.span, &l.name.name, vec![]));
                        for member in &l.members {
                            let written_run = match member {
                                LocusMember::Lifecycle(h) if h.kind == LifecycleKind::Run && !h.synthesized => Some(h.span),
                                LocusMember::Fn(f) if f.name.name == "run" => Some(f.name.span),
                                _ => None,
                            };
                            if let Some(span) = written_run {
                                uses.push(decl(
                                    Capability::EntryInversion(Inversion::ExportedLocusRun),
                                    UseKind::ExportedRun,
                                    span,
                                    "run()",
                                    vec![("locus", l.name.name.clone())],
                                ));
                            }
                        }
                    }
                    for member in &l.members {
                        match member {
                            LocusMember::Placement(b) => {
                                for e in &b.entries {
                                    let field = e.field.name.clone();
                                    match &e.spec {
                                        PlacementSpec::Pinned { .. } => uses.push(decl(
                                            Capability::PinnedThreads,
                                            UseKind::Placement,
                                            e.span,
                                            &format!("placement `{field}`: pinned"),
                                            vec![("field", field.clone())],
                                        )),
                                        PlacementSpec::Cooperative { pool: Some(pool), .. } if pool.name != "main" => {
                                            uses.push(decl(
                                                Capability::PoolThreads,
                                                UseKind::Placement,
                                                e.span,
                                                &format!("placement `{field}`: cooperative(pool = {})", pool.name),
                                                vec![("field", field.clone())],
                                            ))
                                        }
                                        _ => {}
                                    }
                                    for c in &e.constraints {
                                        if c.kind == PlacementConstraint::AsyncIo {
                                            uses.push(decl(
                                                Capability::AsyncIoPool,
                                                UseKind::Placement,
                                                c.span,
                                                &format!("placement `{field}`: where async_io"),
                                                vec![("field", field.clone())],
                                            ));
                                        }
                                    }
                                }
                            }
                            LocusMember::Bindings(b) => {
                                for e in &b.entries {
                                    let (t, span) = match &e.transport {
                                        TransportSpec::Unix { span, .. } => (Transport::Unix, *span),
                                        TransportSpec::ShmRing { span, .. } => (Transport::ShmRing, *span),
                                        TransportSpec::Adapter { span, .. } => (Transport::Adapter, *span),
                                    };
                                    uses.push(decl(
                                        Capability::RemoteTransport(t),
                                        UseKind::Binding,
                                        span,
                                        &format!("bindings `{}`", e.topic.name),
                                        vec![("topic", e.topic.name.clone())],
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if !has_main {
        if let Some(first) = exports.iter().min_by_key(|s| s.start.0) {
            uses.push(decl(
                Capability::EntryInversion(Inversion::ExportOnly),
                UseKind::ExportOnly,
                *first,
                "@export with no `fn main`",
                vec![],
            ));
        }
    }

    // One row per (site, need): a call the parser shares between two
    // parents (an f-string's interpolation) is walked twice.
    let mut seen = BTreeSet::new();
    uses.retain(|u| seen.insert((u.span.start.0, u.span.end.0, u.need.clone())));
    uses.sort_by_key(|u| (u.span.start.0, u.span.end.0));
    CapabilityUses { uses }
}

fn decl(cap: Capability, kind: UseKind, span: Span, what: &str, holes: Vec<(&'static str, String)>) -> CapabilityUse {
    CapabilityUse { need: Need::Capability(cap), kind, span, chain: vec![what.to_string()], holes }
}

/// The uses a call into `k`, beyond the horizon, carries: its summary.
fn crossing(
    uses: &mut Vec<CapabilityUse>,
    req: &BTreeMap<FnKey, Req>,
    k: &FnKey,
    kind: UseKind,
    span: Span,
    links: Vec<String>,
    skip: Option<Capability>,
) {
    let Some(r) = req.get(k) else { return };
    for (cap, chain) in &r.caps {
        if Some(*cap) == skip {
            continue;
        }
        uses.push(CapabilityUse {
            need: Need::Capability(*cap),
            kind,
            span,
            chain: links.iter().cloned().chain(chain.iter().cloned()).collect(),
            holes: Vec::new(),
        });
    }
    if let Some((chain, why)) = &r.hole {
        uses.push(CapabilityUse {
            need: Need::Hole(why),
            kind,
            span,
            chain: links.iter().cloned().chain(chain.iter().cloned()).collect(),
            holes: Vec::new(),
        });
    }
}

/// The uses a construction of a locus beyond the horizon carries: the
/// summaries of what its existence runs.
fn construction(uses: &mut Vec<CapabilityUse>, g: &Graph<'_>, req: &BTreeMap<FnKey, Req>, written: &str, span: Span) {
    let locus = g.locus_of(written);
    if g.own_locus(&locus) {
        return;
    }
    for (hook, link) in g.implied(&locus) {
        crossing(uses, req, &hook, UseKind::Construction, span, vec![locus.clone(), link], None);
    }
}

/// What the graph's own walk meets in a body the summary keys none for
/// (a params initializer, an `on_failure` handler).
#[derive(Debug, Clone)]
enum Met {
    /// A call spelled with a stdlib path: the primitive's namespace,
    /// with the links to it (the local a call through one is written
    /// with, then the path).
    Needs(Capability, Vec<String>, Span),
    /// A call to a fn the summary keys, with the links to it.
    Calls(FnKey, Vec<String>, Span),
    /// A literal, as its path is written.
    Constructs(String, Span),
    /// A call whose requirements the walk cannot establish: a method
    /// whose receiver's type it cannot name (beyond the horizon only: it
    /// is the construction's to locate), or a function value it cannot
    /// resolve.
    Hole(String, &'static str, Span),
}

/// What a local function value beyond the horizon resolves to: what a
/// direct call of the bound callee would meet.
#[derive(Debug, Clone)]
enum FnValue {
    /// A fn the summary keys.
    Fn(FnKey),
    /// A stdlib primitive: its namespace, with the path.
    Primitive(Capability, String),
    /// A builtin, or a merged fn the summary keys no body for: a direct
    /// call of it meets nothing either.
    Nothing,
}

/// Why a call through a local function value is a hole.
const UNRESOLVED_VALUE: &str = "the callee is a function value the summary cannot resolve";

/// A declared type's name as written (`std::http::Server`, a merged
/// name), or `None` for a builtin type, which has no methods a target is
/// asked for.
fn type_name(t: &TypeExpr) -> Option<String> {
    match t {
        TypeExpr::Named { path, .. } => Some(qualified(path)),
        _ => None,
    }
}

fn qualified(qn: &QualifiedName) -> String {
    qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")
}

/// The graph's own walk of a body the summary keys none for.
struct Walker<'w, 'a> {
    g: &'w Graph<'a>,
    /// Beyond the horizon, the declared types a method's receiver
    /// resolves through: the locus (`self`), its params fields and the
    /// handler's params. `None` in the program's own sources, whose
    /// methods are judged in their own bodies.
    receivers: Option<Receivers<'w>>,
    /// The body's local bindings, innermost scope last, each with the
    /// function value it holds: `None` for one the walk cannot resolve
    /// (a field, a call's result, a loop or pattern binding, a handler
    /// param, a reassigned binding).
    locals: Vec<BTreeMap<String, Option<FnValue>>>,
    met: Vec<Met>,
}

struct Receivers<'w> {
    locus: &'w str,
    fields: &'w BTreeMap<String, Option<String>>,
    params: BTreeMap<String, Option<String>>,
}

impl<'w, 'a> Walker<'w, 'a> {
    fn own(g: &'w Graph<'a>) -> Self {
        Walker { g, receivers: None, locals: Vec::new(), met: Vec::new() }
    }

    fn beyond(
        g: &'w Graph<'a>,
        locus: &'w str,
        fields: &'w BTreeMap<String, Option<String>>,
        params: BTreeMap<String, Option<String>>,
    ) -> Self {
        let locals = vec![params.keys().map(|p| (p.clone(), None)).collect()];
        Walker { g, receivers: Some(Receivers { locus, fields, params }), locals, met: Vec::new() }
    }

    fn bind(&mut self, name: &str, value: Option<FnValue>) {
        if self.locals.is_empty() {
            self.locals.push(BTreeMap::new());
        }
        self.locals.last_mut().expect("a scope").insert(name.to_string(), value);
    }

    fn local(&self, name: &str) -> Option<&Option<FnValue>> {
        self.locals.iter().rev().find_map(|scope| scope.get(name))
    }

    /// The binding `name` names now holds a value the walk cannot
    /// resolve.
    fn unresolve(&mut self, name: &str) {
        if let Some(scope) = self.locals.iter_mut().rev().find(|s| s.contains_key(name)) {
            scope.insert(name.to_string(), None);
        }
    }

    /// Before a loop is walked, each binding it reassigns is unresolved
    /// for the whole loop and after it, as in the summary's walk
    /// ([`loop_reassigned`]).
    fn enter_loop(&mut self, cond: Option<&Expr>, body: &Block) {
        for name in loop_reassigned(cond, body) {
            self.unresolve(&name);
        }
    }

    /// Whether a bare callee names a builtin or a fn a direct call
    /// reaches: a merged name beyond the horizon, a fn of the program's
    /// own in its sources.
    fn names_fn(&self, name: &str) -> bool {
        let keyed = || self.g.summary.resolve(None, name).is_some();
        crate::check::BARE_BUILTIN_CALLEES.contains(&name)
            || (name.starts_with("__") || self.receivers.is_none()) && keyed()
    }

    /// The function value an expression evaluates to, or `None` when
    /// the walk cannot resolve it: a local's binding, a fn the merged
    /// name keys (in the program's own sources, a fn of its own), a
    /// builtin, a stdlib path, an import alias's fn.
    fn fn_value(&self, e: &Expr) -> Option<FnValue> {
        match e {
            Expr::Ident(id) => match self.local(&id.name) {
                Some(bound) => bound.clone(),
                None if id.name.starts_with("__") => Some(match self.g.summary.resolve(None, &id.name) {
                    Some(k) => FnValue::Fn(k.clone()),
                    None => FnValue::Nothing,
                }),
                None if self.receivers.is_none() && self.g.summary.resolve(None, &id.name).is_some() => {
                    self.g.summary.resolve(None, &id.name).map(|k| FnValue::Fn(k.clone()))
                }
                None => crate::check::BARE_BUILTIN_CALLEES.contains(&id.name.as_str()).then_some(FnValue::Nothing),
            },
            Expr::Path(qn) => {
                let path = qualified(qn);
                if let Some(ns) = std_namespace(self.g.m, &path) {
                    Some(FnValue::Primitive(Capability::StdNamespace(ns), path))
                } else {
                    // A merged name the summary keys no row for names
                    // nothing the graph reaches.
                    self.g.renames.get(&path).map(|mangled| {
                        self.g.summary.resolve(None, mangled).map_or(FnValue::Nothing, |k| FnValue::Fn(k.clone()))
                    })
                }
            }
            _ => None,
        }
    }

    /// A call through a function value: a use of the target it resolves
    /// to, through the local it is written with; a hole when it resolves
    /// to none.
    fn call_value(&mut self, value: Option<FnValue>, callee: String, span: Span) {
        match value {
            Some(FnValue::Fn(k)) => {
                let links = through(Some(&callee), k.display());
                self.met.push(Met::Calls(k, links, span))
            }
            Some(FnValue::Primitive(cap, path)) => self.met.push(Met::Needs(cap, through(Some(&callee), path), span)),
            Some(FnValue::Nothing) => {}
            None => self.met.push(Met::Hole(format!("{callee}()"), UNRESOLVED_VALUE, span)),
        }
    }

    /// A method call beyond the horizon: the locus's method its
    /// receiver's declared type names, a stdlib handle's namespace, or a
    /// hole when the walk cannot name the type.
    fn method(&mut self, receiver: &Expr, name: &str, span: Span) {
        let Some(r) = &self.receivers else { return };
        let ty = match receiver {
            Expr::KwSelf(_) => Some(Some(r.locus.to_string())),
            Expr::Ident(id) => r.params.get(&id.name).cloned(),
            Expr::Field { receiver, name: field, .. } if matches!(**receiver, Expr::KwSelf(_)) => {
                r.fields.get(&field.name).cloned()
            }
            Expr::Struct { path, .. } => Some(Some(qualified(path))),
            _ => None,
        };
        match ty {
            None => self.met.push(Met::Hole(name.to_string(), "its receiver's type is not known here", span)),
            Some(None) => {}
            Some(Some(ty)) => {
                let ty = self.g.locus_of(&ty);
                if let Some(k) = self.g.summary.resolve(Some(&ty), name).cloned() {
                    let links = vec![k.display()];
                    self.met.push(Met::Calls(k, links, span));
                } else if let Some(ns) = self.g.std_loci.get(&ty) {
                    self.met.push(Met::Needs(Capability::StdNamespace(ns), vec![format!("{ty}::{name}")], span));
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Struct { path, inits, span, .. } => {
                self.met.push(Met::Constructs(qualified(path), *span));
                for i in inits {
                    self.expr(&i.value);
                }
            }
            Expr::Call { callee, args, span, .. } => {
                match callee.as_ref() {
                    Expr::Path(qn) => {
                        let path = qualified(qn);
                        if let Some(ns) = std_namespace(self.g.m, &path) {
                            self.met.push(Met::Needs(Capability::StdNamespace(ns), vec![path], qn.span));
                        } else if let Some(k) =
                            self.g.renames.get(&path).and_then(|mangled| self.g.summary.resolve(None, mangled)).cloned()
                        {
                            let links = vec![k.display()];
                            self.met.push(Met::Calls(k, links, qn.span));
                        }
                    }
                    // A call through a local is a use of the fn value it
                    // holds, or a hole: never nothing. A bare callee names
                    // a builtin or a fn before a local, as codegen lowers
                    // it.
                    Expr::Ident(id) if self.local(&id.name).is_some() && !self.names_fn(&id.name) => {
                        let value = self.local(&id.name).cloned().flatten();
                        self.call_value(value, id.name.clone(), id.span);
                    }
                    // A bare call beyond the horizon names its seed's own
                    // fn by the merged (unspeakable) name; a builtin's is
                    // speakable.
                    Expr::Ident(id) if self.receivers.is_some() && id.name.starts_with("__") => {
                        if let Some(k) = self.g.summary.resolve(None, &id.name).cloned() {
                            let links = vec![k.display()];
                            self.met.push(Met::Calls(k, links, id.span));
                        }
                    }
                    Expr::Field { receiver, name, span } | Expr::Path2 { receiver, name, span } => {
                        self.method(receiver, &name.name, *span);
                        // A method's receiver is evaluated: `xs[i].m()`
                        // runs `i`.
                        self.expr(receiver);
                    }
                    // A callee computed by an expression: the walk cannot
                    // resolve what it calls.
                    other @ (Expr::Ident(_) | Expr::KwSelf(_)) => self.expr(other),
                    other => {
                        self.call_value(None, "<expr>".to_string(), *span);
                        self.expr(other)
                    }
                }
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => {
                for x in xs {
                    self.expr(x);
                }
            }
            Expr::ArrayRepeat { val, .. } | Expr::Sum(val, _) | Expr::Prod(val, _) => self.expr(val),
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                if let OrDisposition::Substitute(x) | OrDisposition::Fail(x, _) = disposition {
                    self.expr(x);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }

    fn block(&mut self, b: &Block) {
        self.locals.push(BTreeMap::new());
        for s in &b.stmts {
            match s {
                Stmt::Let { name, value, .. } => {
                    self.expr(value);
                    let bound = self.fn_value(value);
                    self.bind(&name.name, bound);
                }
                Stmt::LetTuple { names, value, .. } => {
                    self.expr(value);
                    for n in names {
                        self.bind(&n.name, None);
                    }
                }
                Stmt::Fail { value, .. } => self.expr(value),
                Stmt::Assign { value, target, .. } => {
                    for seg in &target.tail {
                        if let LValueSeg::Index(e) = seg {
                            self.expr(e);
                        }
                    }
                    self.expr(value);
                    // A reassigned binding holds whichever value the
                    // run took last: the walk does not follow flow.
                    if target.tail.is_empty() {
                        self.unresolve(&target.head.name);
                    }
                }
                Stmt::If(i) => self.if_stmt(i),
                Stmt::Match(m) => self.match_stmt(m),
                Stmt::For { name, iter, body, .. } => {
                    self.expr(iter);
                    self.enter_loop(None, body);
                    self.locals.push(BTreeMap::from([(name.name.clone(), None)]));
                    self.block(body);
                    self.locals.pop();
                }
                Stmt::While { cond, body, .. } => {
                    self.enter_loop(Some(cond), body);
                    self.expr(cond);
                    self.block(body);
                }
                Stmt::Return(e, _) => {
                    if let Some(e) = e {
                        self.expr(e);
                    }
                }
                Stmt::Violate { payload, .. } => {
                    if let Some(e) = payload {
                        self.expr(e);
                    }
                }
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
                Stmt::ShmWrite { max, body, .. } => {
                    self.expr(max);
                    self.block(body);
                }
                Stmt::Block(b) => self.block(b),
                Stmt::Expr(e) => self.expr(e),
                Stmt::Break(_)
                | Stmt::Continue(_)
                | Stmt::Yield(_)
                | Stmt::Terminate(_)
                | Stmt::Reperspective { .. } => {}
            }
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
        self.locals.pop();
    }

    fn if_stmt(&mut self, i: &IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(e)) => self.if_stmt(e),
            None => {}
        }
    }

    fn match_stmt(&mut self, m: &MatchStmt) {
        self.expr(&m.scrutinee);
        for arm in &m.arms {
            let mut bound = BTreeMap::new();
            pattern_bindings(&arm.pattern, &mut bound);
            self.locals.push(bound);
            if let Some(guard) = &arm.guard {
                self.expr(guard);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b),
            }
            self.locals.pop();
        }
    }
}

/// The names a match pattern binds, none of them a value the walk
/// resolves.
fn pattern_bindings(p: &Pattern, out: &mut BTreeMap<String, Option<FnValue>>) {
    match p {
        Pattern::Binding(id) => {
            out.insert(id.name.clone(), None);
        }
        Pattern::Constructor { args: ps, .. } | Pattern::Tuple(ps, _) => {
            for p in ps {
                pattern_bindings(p, out);
            }
        }
        Pattern::Literal(..) | Pattern::Wildcard(_) => {}
    }
}

/// How a refusal names what put the program under its target.
fn selector(row: &TargetRow) -> String {
    match row.class {
        Some(TargetClass::Wasm32) => row.wasm32_selector().to_string(),
        _ => format!("`{}`", row.effective.triple),
    }
}

/// Whether a `Reject` cell is still today's late refusal (a known-open
/// cell the admission does not locate yet).
fn late(class: TargetClass, cap: Capability) -> bool {
    KNOWN_OPEN.iter().any(|k: &KnownOpen| k.class == class && k.cell == OpenCell::LateRefusal(cap))
}

/// The admission law: every use's cell for the effective target must be
/// `Lower`; each one that is not is a located diagnostic naming the
/// capability, the target and the witness. A hole is refused on a
/// target whose column rejects anything in the stdlib family.
pub fn admission_diags(
    uses: &CapabilityUses,
    row: &TargetRow,
    import_renames: &[(Vec<String>, String)],
) -> Vec<Diag> {
    let Some(class) = row.class else { return Vec::new() };
    // A target the row itself refuses (an explicit `--target` a written
    // declaration contradicts) is not one the program is judged against:
    // the conflict is the refusal.
    if !row.refusals.is_empty() {
        return Vec::new();
    }
    let m = super::derive_capability_matrix();
    let table = crate::stdlib_bodies::Demangler::new(import_renames);
    let name = |s: &str| table.demangle(s);
    let witness = |chain: &[String]| chain.iter().map(|l| format!("`{}`", name(l))).collect::<Vec<_>>().join(" → ");
    let rejects_std = m.behaviours.iter().any(|r| {
        matches!(r.capability, Capability::StdNamespace(_)) && !r.cells.get(class).is_lower()
    });
    let sel = selector(row);
    let mut out = Vec::new();
    let refused: BTreeSet<(u32, u32)> = uses
        .uses
        .iter()
        .filter(|u| match &u.need {
            Need::Capability(cap) => m.behaviour(class, *cap).is_some_and(|c| {
                !c.is_lower() && c.origin == Origin::Source && !late(class, *cap)
            }),
            Need::Hole(_) => false,
        })
        .map(|u| (u.span.start.0, u.span.end.0))
        .collect();
    for u in &uses.uses {
        match &u.need {
            Need::Capability(cap) => {
                let Some(cell) = m.behaviour(class, *cap) else { continue };
                let BehaviourVerdict::Reject(refusal) = &cell.verdict else { continue };
                if cell.origin != Origin::Source || late(class, *cap) {
                    continue;
                }
                let mut holes: Vec<(&str, String)> = u.holes.iter().map(|(k, v)| (*k, v.clone())).collect();
                holes.push(("selector", sel.clone()));
                let msg = match cap {
                    // The written callee is the stdlib path itself: the
                    // refusal names it, as the stdlib gate always has.
                    // (A local bound to the path is written instead: its
                    // chain is the witness.)
                    Capability::StdNamespace(_) if u.kind == UseKind::Call && u.chain.len() == 1 => {
                        let path = name(&u.chain[0]);
                        holes.push(("path", path.trim_start_matches("std::").to_string()));
                        refusal.render(&cell.witness, &holes_ref(&holes))
                    }
                    Capability::StdNamespace(ns) => {
                        holes.push(("path", ns.to_string()));
                        format!("{} — witness: {}", refusal.render(&cell.witness, &holes_ref(&holes)), witness(&u.chain))
                    }
                    _ => refusal.render(&cell.witness, &holes_ref(&holes)),
                };
                out.push(Diag::ty(u.span, msg));
            }
            Need::Hole(why) => {
                // A site already refused for a capability needs no second
                // refusal for what else it might reach.
                if !rejects_std || refused.contains(&(u.span.start.0, u.span.end.0)) {
                    continue;
                }
                let mut msg = format!("cannot establish what `{}` requires on {}: {why}", name(&u.chain[0]), class.name());
                if u.chain.len() > 1 {
                    msg.push_str(&format!(" — witness: {}", witness(&u.chain)));
                }
                out.push(Diag::ty(u.span, msg));
            }
        }
    }
    out
}

fn holes_ref<'h>(holes: &'h [(&'h str, String)]) -> Vec<(&'h str, &'h str)> {
    holes.iter().map(|(k, v)| (*k, v.as_str())).collect()
}
