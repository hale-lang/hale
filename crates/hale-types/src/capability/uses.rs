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
//!
//! **The horizon.** The program's own sources are the horizon: a use is
//! located at the first site in them. A wrapper the program writes is
//! judged at the use in its own body, once, and its callers carry
//! nothing; a fn beyond the horizon (an imported seed's, the stdlib's)
//! is judged at the call that crosses into it, with its summary.
//!
//! **Holes.** A call the graph cannot resolve — a method on a receiver
//! whose type the walk cannot name, a call through a function-typed
//! parameter — leaves the use's requirements unknown. On a target whose
//! column rejects anything in the stdlib family (wasm32) that is a
//! refusal; elsewhere it is a recorded hole, counted, never silent.
//!
//! **Type-only mentions** are not uses: a parameter, field or return
//! type naming `std::io::tcp::Stream` asks nothing of the target.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    flat_decls, Block, ElseBranch, Expr, IfStmt, LValueSeg, LifecycleKind, LocusMember, MatchArmBody,
    MatchStmt, OrDisposition, ParamInit, PlacementConstraint, PlacementSpec, Program, Stmt, TopDecl,
    TransportSpec,
};
use hale_syntax::{Diag, Span};

use super::{
    Abi, BehaviourVerdict, Capability, CapabilityMatrix, Inversion, KnownOpen, OpenCell, Origin,
    TargetClass, TargetRow, Transport, KNOWN_OPEN,
};
use crate::alloc_summary::{AllocKind, AllocSummary, CallEdge, Callee, FnKey};

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
    if e.indirect {
        return Some("it is called through a function-typed parameter, whose target is not known here");
    }
    if e.opaque_method_call() {
        return Some("its receiver's type is not known here");
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
    demangle: Vec<(String, String)>,
}

impl<'a> Graph<'a> {
    fn new(summary: &'a AllocSummary, m: &'a CapabilityMatrix, import_renames: &[(Vec<String>, String)]) -> Self {
        let renames = import_renames.iter().map(|(k, v)| (k.join("::"), v.clone())).collect();
        let std_loci = hale_stdlib::PATH_RENAMES
            .iter()
            .filter_map(|(path, mangled)| std_namespace(m, &path.join("::")).map(|ns| (mangled.to_string(), ns)))
            .collect();
        let demangle = crate::stdlib_bodies::demangle_table(import_renames);
        Graph { summary, m, renames, std_loci, demangle }
    }

    /// A name as the author spells it.
    fn public(&self, name: &str) -> String {
        crate::stdlib_bodies::demangle_with(name, &self.demangle)
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
    /// handlers (every entry the summary keys on it).
    fn implied(&self, locus: &str) -> Vec<(&'a FnKey, String)> {
        self.summary
            .fns
            .iter()
            .filter(|(k, f)| k.locus.as_deref() == Some(locus) && f.entry.is_some())
            .map(|(k, _)| (k, format!("{}()", k.fn_name)))
            .collect()
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
                    Some(why) => Edge::Hole(name.clone(), why),
                    None => Edge::Nothing,
                }
            }
        }
    }

    /// Every fn's requirements, as a fixpoint over the graph.
    fn requirements(&self) -> BTreeMap<&'a FnKey, Req> {
        let mut req: BTreeMap<&'a FnKey, Req> = BTreeMap::new();
        // Each fn's out-edges to other fns, with the links they add.
        let mut out: BTreeMap<&'a FnKey, Vec<(&'a FnKey, Vec<String>)>> = BTreeMap::new();
        for (key, fs) in &self.summary.fns {
            let r = req.entry(key).or_default();
            let edges = out.entry(key).or_default();
            for e in &fs.calls {
                match self.edge(e) {
                    Edge::Needs(cap, link) => {
                        r.caps.entry(cap).or_insert_with(|| vec![link]);
                    }
                    Edge::Hole(name, why) => {
                        if r.hole.is_none() {
                            r.hole = Some((vec![name], why));
                        }
                    }
                    Edge::Calls(k) => {
                        if let Some((callee, _)) = self.summary.fns.get_key_value(&k) {
                            edges.push((callee, vec![callee.display()]));
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
        loop {
            let mut changed = false;
            for (key, edges) in &out {
                for (callee, links) in edges {
                    let Some(from) = req.get(callee).cloned() else { continue };
                    let r = req.get_mut(key).expect("every fn has a row");
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

/// Whether a name is an imported seed's merged decl.
fn merged(name: &str) -> bool {
    name.split("::").any(|seg| seg.starts_with("__lib_"))
}

/// The use rows of the programs a bundle holds, over the bundle's
/// allocation summary (`target_capability`'s producer for uses).
pub fn derive_capability_uses(bundle: &crate::Bundle<'_>, summary: &AllocSummary) -> CapabilityUses {
    // The summary keys top-level declarations; a declaration inside a
    // `module { }` is lowered at any depth (GH #884), so a program that
    // nests one is walked flattened, with the stdlib's copy beside it.
    let flattened;
    let summary = if bundle.programs.values().any(|p| p.items.iter().any(|i| matches!(i, TopDecl::Module(_)))) {
        let flat: Vec<Program> = bundle
            .programs
            .values()
            .map(|p| {
                let mut q = (*p).clone();
                q.items = flat_decls(&p.items).filter(|i| !matches!(i, TopDecl::Module(_))).cloned().collect();
                q
            })
            .collect();
        let mut identified: Vec<(&Program, &crate::snapshot::Snapshot)> =
            flat.iter().map(|p| (p, &bundle.snapshot)).collect();
        if let (Some(program), Some(ids)) = (crate::stdlib_bodies::program(), crate::stdlib_bodies::identities()) {
            identified.push((program, ids));
        }
        flattened = crate::alloc_summary::summarize_identified(&identified, &bundle.import_renames);
        &flattened
    } else {
        summary
    };
    let m = super::derive_capability_matrix();
    let g = Graph::new(summary, &m, &bundle.import_renames);
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
                    chain: vec![link],
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
                    let written = g.public(&k.display());
                    let direct = (!e.receiver_present).then(|| std_namespace(&m, &written)).flatten();
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
                    crossing(&mut uses, &req, &k, kind, e.callee_span, vec![k.display()], skip);
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
    // handlers.
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    for p in &programs {
        for item in flat_decls(&p.items) {
            let TopDecl::Locus(l) = item else { continue };
            if !g.own_locus(&l.name.name) {
                continue;
            }
            for member in &l.members {
                match member {
                    LocusMember::Params(pb) => {
                        for prm in &pb.params {
                            if let ParamInit::Value(e) = &prm.init {
                                initializer(&mut uses, &g, &req, e);
                            }
                        }
                    }
                    LocusMember::Failure(f) => block(&mut uses, &g, &req, &f.body),
                    _ => {}
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
    req: &BTreeMap<&FnKey, Req>,
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
fn construction(uses: &mut Vec<CapabilityUse>, g: &Graph<'_>, req: &BTreeMap<&FnKey, Req>, written: &str, span: Span) {
    let locus = g.locus_of(written);
    if g.own_locus(&locus) {
        return;
    }
    for (hook, link) in g.implied(&locus) {
        crossing(uses, req, hook, UseKind::Construction, span, vec![locus.clone(), link], None);
    }
}

/// The calls and constructions in a params initializer.
fn initializer(uses: &mut Vec<CapabilityUse>, g: &Graph<'_>, req: &BTreeMap<&FnKey, Req>, e: &Expr) {
    match e {
        Expr::Struct { path, inits, span, .. } => {
            let written = path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
            construction(uses, g, req, &written, *span);
            for i in inits {
                initializer(uses, g, req, &i.value);
            }
        }
        Expr::Call { callee, args, .. } => {
            match callee.as_ref() {
                Expr::Path(qn) => {
                    let path = qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                    if let Some(ns) = std_namespace(g.m, &path) {
                        uses.push(CapabilityUse {
                            need: Need::Capability(Capability::StdNamespace(ns)),
                            kind: UseKind::Call,
                            span: qn.span,
                            chain: vec![path],
                            holes: Vec::new(),
                        });
                    } else if let Some(mangled) = g.renames.get(&path) {
                        let k = FnKey::free_fn(mangled.clone());
                        if !g.own(&k) {
                            crossing(uses, req, &k, UseKind::Crossing, qn.span, vec![k.display()], None);
                        }
                    }
                }
                // A method's receiver is evaluated: `xs[i].m()` runs `i`.
                other => initializer(uses, g, req, other),
            }
            for a in args {
                initializer(uses, g, req, a);
            }
        }
        Expr::Binary { left, right, .. } => {
            initializer(uses, g, req, left);
            initializer(uses, g, req, right);
        }
        Expr::Approx { left, right, tolerance, .. } => {
            initializer(uses, g, req, left);
            initializer(uses, g, req, right);
            initializer(uses, g, req, tolerance);
        }
        Expr::Range { lo, hi, .. } => {
            initializer(uses, g, req, lo);
            initializer(uses, g, req, hi);
        }
        Expr::Unary { operand, .. } => initializer(uses, g, req, operand),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => initializer(uses, g, req, receiver),
        Expr::Index { receiver, index, .. } => {
            initializer(uses, g, req, receiver);
            initializer(uses, g, req, index);
        }
        Expr::Tuple(xs, _) | Expr::Array(xs, _) => {
            for x in xs {
                initializer(uses, g, req, x);
            }
        }
        Expr::ArrayRepeat { val, .. } | Expr::Sum(val, _) | Expr::Prod(val, _) => initializer(uses, g, req, val),
        Expr::Or { inner, disposition, .. } => {
            initializer(uses, g, req, inner);
            if let OrDisposition::Substitute(x) | OrDisposition::Fail(x, _) = disposition {
                initializer(uses, g, req, x);
            }
        }
        Expr::Block(b) => block(uses, g, req, b),
        Expr::If(i) => if_stmt(uses, g, req, i),
        Expr::Match(m) => match_stmt(uses, g, req, m),
        Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
    }
}

/// The calls and constructions in a body the summary does not key (an
/// `on_failure` handler's).
fn block(uses: &mut Vec<CapabilityUse>, g: &Graph<'_>, req: &BTreeMap<&FnKey, Req>, b: &Block) {
    for s in &b.stmts {
        match s {
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Fail { value, .. } => {
                initializer(uses, g, req, value)
            }
            Stmt::Assign { value, target, .. } => {
                for seg in &target.tail {
                    if let LValueSeg::Index(e) = seg {
                        initializer(uses, g, req, e);
                    }
                }
                initializer(uses, g, req, value);
            }
            Stmt::If(i) => if_stmt(uses, g, req, i),
            Stmt::Match(m) => match_stmt(uses, g, req, m),
            Stmt::For { iter, body, .. } => {
                initializer(uses, g, req, iter);
                block(uses, g, req, body);
            }
            Stmt::While { cond, body, .. } => {
                initializer(uses, g, req, cond);
                block(uses, g, req, body);
            }
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    initializer(uses, g, req, e);
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(e) = payload {
                    initializer(uses, g, req, e);
                }
            }
            Stmt::Recovery { args, .. } => {
                for a in args {
                    initializer(uses, g, req, a);
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                initializer(uses, g, req, subject);
                initializer(uses, g, req, value);
                if let Some(OrDisposition::Substitute(x) | OrDisposition::Fail(x, _)) = or_disposition {
                    initializer(uses, g, req, x);
                }
            }
            Stmt::ShmWrite { max, body, .. } => {
                initializer(uses, g, req, max);
                block(uses, g, req, body);
            }
            Stmt::Block(b) => block(uses, g, req, b),
            Stmt::Expr(e) => initializer(uses, g, req, e),
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }
    if let Some(t) = &b.tail {
        initializer(uses, g, req, t);
    }
}

fn if_stmt(uses: &mut Vec<CapabilityUse>, g: &Graph<'_>, req: &BTreeMap<&FnKey, Req>, i: &IfStmt) {
    initializer(uses, g, req, &i.cond);
    block(uses, g, req, &i.then_block);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => block(uses, g, req, b),
        Some(ElseBranch::ElseIf(e)) => if_stmt(uses, g, req, e),
        None => {}
    }
}

fn match_stmt(uses: &mut Vec<CapabilityUse>, g: &Graph<'_>, req: &BTreeMap<&FnKey, Req>, m: &MatchStmt) {
    initializer(uses, g, req, &m.scrutinee);
    for arm in &m.arms {
        if let Some(guard) = &arm.guard {
            initializer(uses, g, req, guard);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => initializer(uses, g, req, e),
            MatchArmBody::Block(b) => block(uses, g, req, b),
        }
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
    let table = crate::stdlib_bodies::demangle_table(import_renames);
    let name = |s: &str| crate::stdlib_bodies::demangle_with(s, &table);
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
                    Capability::StdNamespace(_) if u.kind == UseKind::Call => {
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
