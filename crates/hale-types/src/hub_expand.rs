//! GH #1417 (R5): a topic bound to a hub is a stream.
//!
//! `Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;`
//! names a stream row of the hub instance `self.hub` (a param of the main
//! locus, built by a `ws::Hub { … }` literal). The check reads the rows as
//! written; this pass, which runs in the desugar sequence beside
//! `rpc_expand` and before the check, is what makes a program that holds
//! one buildable. For each hub it adds, as `rpc_expand` does for a socket
//! transport:
//!
//! - the hub's own literal rewritten to the standard library's
//!   (`std::api::ws::Hub`), carrying the hub's number (`tid`), the rows
//!   adapter below, and the optional extensions of its sources (the
//!   bearer's `expiry`, the role source's `grants`);
//! - a listener param (`__StdApiWsListener`) after every authored param of
//!   the main locus (its `bind:` may read one), placed on the shared
//!   `async_io` pool `__api_ws`, and before the exposure of a serve over the
//!   hub (`rpc_expand` pushes it later), as a socket transport's;
//! - for each stream, an adapter binding in the main locus's `bindings`
//!   block (`Topic: __StdApiHubStream { hub: N, stream: I } codec(…)`), so
//!   the publish fanout reaches the hub as it reaches any adapter, and the
//!   publish contract and the no-subscriber lint read the topic as bound
//!   outward; the codec is the JSON codec of the topic's payload;
//! - a top-level locus `__HubRows_<n>` implementing `std::api::HubRows`:
//!   the stream rows as if-chains.
//!
//! The hub bindings themselves stay on the block (`BindingsBlock::hubs`):
//! they are the rows `hale check --api` describes.
//!
//! The pass is idempotent (a hub whose literal is already the standard
//! library's is left alone) and conservative: a hub it cannot resolve is
//! left as written, for [`crate::surfaces::unserved_sites`] to report.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::api_names::API_SYNTH_BASE;
use hale_syntax::ast::{
    flat_decls, Expr, Ident, Literal, LocusDecl, LocusMember, ParamDecl, ParamInit, Program, QualifiedName,
    StructInit, TopDecl, TypeExpr,
};
use hale_syntax::{parse_source_at, Span};

use crate::topic_identity::Shapes;

/// What the pass made of one hub.
#[derive(Debug, Clone)]
pub struct HubExpansion {
    /// The hub's number: unique in the program, from 1.
    pub id: i64,
    /// `as:`.
    pub name: String,
    /// The main locus whose block binds the streams.
    pub locus: String,
    /// The param that is the hub.
    pub param: String,
}

/// One stream row, as the pass reads it.
#[derive(Debug, Clone)]
struct StreamPlan {
    /// The topic, as the binding writes it.
    topic: Ident,
    requires: Vec<String>,
    bound: u64,
    on_full: String,
    payload: TypeExpr,
    span: Span,
}

struct HubPlan {
    /// `ws` or `udp`.
    kind: &'static str,
    program: usize,
    locus: String,
    param: String,
    id: i64,
    name: String,
    bind: Expr,
    principals: Option<Expr>,
    roles: Option<Expr>,
    expiring: bool,
    revised: bool,
    streams: Vec<StreamPlan>,
    pieces: crate::surface_doc::HubPieces,
}

fn lit_int(n: i64, span: Span) -> Expr {
    Expr::Literal(Literal::Int(n), span)
}

fn fresh(e: &Expr) -> Expr {
    let mut e = e.clone();
    hale_syntax::sites::clear_ids_in_expr(&mut e);
    e
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `ws::Hub` or `std::api::ws::Hub`, as a literal's or a type's path.
pub(crate) fn is_ws_hub(path: &QualifiedName) -> bool {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    matches!(segs.as_slice(), ["ws", "Hub"] | ["std", "api", "ws", "Hub"])
}

/// `udp::Hub` or `std::api::udp::Hub`.
pub(crate) fn is_udp_hub(path: &QualifiedName) -> bool {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    matches!(segs.as_slice(), ["udp", "Hub"] | ["std", "api", "udp", "Hub"])
}

/// A hub's literal or type path: `ws::Hub` or `udp::Hub`.
pub(crate) fn is_hub(path: &QualifiedName) -> bool {
    is_ws_hub(path) || is_udp_hub(path)
}

/// Whether a hub param's literal is the one this pass wrote.
pub(crate) fn is_built(init: &ParamInit) -> bool {
    matches!(init, ParamInit::Value(Expr::Struct { path, inits, .. })
        if is_hub(path) && path.segments.len() == 4 && inits.iter().any(|i| i.name.name == "tid"))
}

/// The hub's literal in `l`, by param name.
fn hub_literal<'a>(l: &'a LocusDecl, field: &str) -> Option<&'a Expr> {
    l.members.iter().find_map(|m| match m {
        LocusMember::Params(pb) => pb.params.iter().find_map(|p| match &p.init {
            ParamInit::Value(e @ Expr::Struct { path, .. }) if p.name.name == field && is_hub(path) => Some(e),
            _ => None,
        }),
        _ => None,
    })
}

/// Whether any locus of the programs binds a topic to a hub.
pub fn binds_a_hub(programs: &[&mut Program]) -> bool {
    programs.iter().any(|p| {
        flat_decls(&p.items).any(|d| {
            matches!(d, TopDecl::Locus(l) if l.members.iter().any(|m|
                matches!(m, LocusMember::Bindings(bb) if !bb.hubs.is_empty())))
        })
    })
}

fn find_locus_mut<'a>(items: &'a mut [TopDecl], name: &str) -> Option<&'a mut LocusDecl> {
    for item in items {
        match item {
            TopDecl::Locus(l) if l.name.name == name => return Some(l),
            TopDecl::Module(m) => {
                if let Some(l) = find_locus_mut(&mut m.items, name) {
                    return Some(l);
                }
            }
            _ => {}
        }
    }
    None
}

/// Run the pass over a bundle's programs. The standard library's hub source
/// and topics are appended first (once), by the caller.
pub fn expand(programs: &mut [&mut Program]) -> Vec<HubExpansion> {
    // ---- the immutable phase: hubs, their streams, the codecs' names ----
    let mut plans: Vec<HubPlan> = Vec::new();
    let confer;
    let convs;
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut builtins: BTreeMap<String, Vec<(String, hale_syntax::ast::PrimType)>> = BTreeMap::new();
    {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        let slices: Vec<&[TopDecl]> = ro.iter().map(|p| p.items.as_slice()).collect();
        let shapes = Shapes::of_all(&slices);
        confer = crate::rpc_expand::conferring(&ro);
        convs = hale_syntax::json_gen::scalar_convs(&ro);
        let mut loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
        let mut topics: BTreeMap<&str, &TypeExpr> = BTreeMap::new();
        let mut subjects: BTreeMap<&str, Option<&String>> = BTreeMap::new();
        let schemas = crate::surfaces::Schemas::of(&ro);
        let mut publishes: BTreeSet<String> = BTreeSet::new();
        for p in &ro {
            for d in flat_decls(&p.items) {
                match d {
                    TopDecl::Locus(l) => {
                        loci.insert(l.name.name.as_str(), l);
                        for m in &l.members {
                            if let LocusMember::Bus(b) = m {
                                for bm in &b.members {
                                    if let hale_syntax::ast::BusMember::Publish {
                                        subject: hale_syntax::ast::BusSubject::Topic(t),
                                        ..
                                    } = bm
                                    {
                                        publishes.insert(t.name.clone());
                                    }
                                }
                            }
                        }
                    }
                    TopDecl::Topic(t) => {
                        topics.insert(t.name.name.as_str(), &t.payload);
                        subjects.insert(t.name.name.as_str(), t.subject.as_ref());
                    }
                    _ => {}
                }
            }
        }
        let mut seen_names: BTreeSet<String> = BTreeSet::new();
        for (pi, p) in ro.iter().enumerate() {
            for d in flat_decls(&p.items) {
                let TopDecl::Locus(l) = d else { continue };
                if !l.is_main {
                    continue;
                }
                // the hubs this locus binds, in the order first bound
                let mut order: Vec<&str> = Vec::new();
                for m in &l.members {
                    if let LocusMember::Bindings(bb) = m {
                        for hb in &bb.hubs {
                            if !order.contains(&hb.instance.name.as_str()) {
                                order.push(hb.instance.name.as_str());
                            }
                        }
                    }
                }
                for field in order {
                    // a hub is a param built by a `ws::Hub` literal, once
                    let Some(lit) = hub_literal(l, field) else { continue };
                    if l.members.iter().any(|m| {
                        matches!(m, LocusMember::Params(pb) if pb.params.iter().any(|p| p.name.name == field && is_built(&p.init)))
                    }) {
                        continue;
                    }
                    let Expr::Struct { path: lit_path, inits, .. } = lit else { continue };
                    let kind = if is_udp_hub(lit_path) { "udp" } else { "ws" };
                    let init = |n: &str| inits.iter().find(|i| i.name.name == n).map(|i| &i.value);
                    let Some(bind) = init("bind") else { continue };
                    let Some(Expr::Literal(Literal::String(name), _)) = init("as").or_else(|| init("name")) else {
                        continue;
                    };
                    // the hub is named once among the exposures
                    if !seen_names.insert(name.clone()) {
                        continue;
                    }
                    let params = crate::surfaces::param_types(l);
                    let has_fn = |src: Option<&Expr>, want: &str| -> bool {
                        let Some(e) = src else { return false };
                        let ty = match e {
                            Expr::Struct { path, .. } => {
                                path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")
                            }
                            e => match crate::surfaces::self_field(e).and_then(|f| params.get(f).cloned()) {
                                Some(t) => t,
                                None => return false,
                            },
                        };
                        loci.get(ty.as_str()).is_some_and(|d| {
                            d.members.iter().any(|m| matches!(m, LocusMember::Fn(f) if f.name.name == want))
                        })
                    };
                    let principals = init("principals").map(fresh);
                    let roles = init("roles").map(fresh);
                    let mut streams: Vec<StreamPlan> = Vec::new();
                    let mut ok = true;
                    for m in &l.members {
                        let LocusMember::Bindings(bb) = m else { continue };
                        for hb in bb.hubs.iter().filter(|h| h.instance.name == field) {
                            let Some(payload) = topics.get(hb.topic.name.as_str()) else {
                                ok = false;
                                continue;
                            };
                            let on_full = hb.on_full.as_ref().map(|i| i.name.clone()).unwrap_or_default();
                            let Some((bound, _)) = hb.bound else {
                                ok = false;
                                continue;
                            };
                            if !matches!(on_full.as_str(), "drop_old" | "drop_new") || !publishes.contains(&hb.topic.name) {
                                ok = false;
                                continue;
                            }
                            streams.push(StreamPlan {
                                topic: hb.topic.clone(),
                                requires: hb.requires.iter().map(|i| i.name.clone()).collect(),
                                bound,
                                on_full,
                                payload: (*payload).clone(),
                                span: hb.span,
                            });
                        }
                    }
                    // a hub of more than sixteen streams, or a stream the laws
                    // refuse, is left as written
                    if !ok || streams.is_empty() || streams.len() > 16 {
                        continue;
                    }
                    for s in &streams {
                        crate::rpc_expand::struct_closure(&shapes, &s.payload, &mut names, &mut builtins);
                    }
                    // the hub as the description reads it (`hale check --api`)
                    let row_ty = |te: &TypeExpr| {
                        let shape = shapes.shape(te, crate::topic_identity::ShapeForm::Contract).unwrap_or_default();
                        crate::surfaces::RowTy {
                            te: te.clone(),
                            display: crate::surfaces::type_text(te),
                            hash: crate::topic_identity::shape_hash(&shape),
                            shape,
                        }
                    };
                    let source = |e: Option<&Expr>| {
                        let f = crate::surfaces::self_field(e?)?;
                        Some(crate::surfaces::Source { source: format!("self.{f}"), ty: params.get(f).cloned() })
                    };
                    let doc_hub = crate::surfaces::Hub {
                        instance: format!("self.{field}"),
                        transport: crate::surfaces::Transport {
                            kind: kind.to_string(),
                            address: match bind {
                                Expr::Literal(Literal::String(a), _) => Some(a.clone()),
                                _ => None,
                            },
                            codec: "json".to_string(),
                            principals: source(principals.as_ref()),
                            roles: source(roles.as_ref()),
                            name: Some(name.clone()),
                        },
                        streams: streams
                            .iter()
                            .map(|st| crate::surfaces::Stream {
                                topic: st.topic.name.clone(),
                                subject: subjects
                                    .get(st.topic.name.as_str())
                                    .and_then(|s| s.cloned())
                                    .unwrap_or_else(|| st.topic.name.clone()),
                                direction: "out",
                                payload: Some(row_ty(&st.payload)),
                                codec: "json".to_string(),
                                bound: Some(st.bound),
                                on_full: Some(st.on_full.clone()),
                                replay: false,
                                requires: st.requires.iter().map(|r| (r.clone(), st.span)).collect(),
                                span: st.span,
                            })
                            .collect(),
                        span: l.name.span,
                    };
                    let pieces = crate::surface_doc::hub_pieces(&doc_hub, &schemas);
                    plans.push(HubPlan {
                        kind,
                        program: pi,
                        locus: l.name.name.clone(),
                        param: field.to_string(),
                        id: plans.len() as i64 + 1,
                        name: name.clone(),
                        bind: fresh(bind),
                        expiring: has_fn(principals.as_ref(), "expiry"),
                        revised: has_fn(roles.as_ref(), "grants"),
                        principals,
                        roles,
                        streams,
                        pieces,
                    });
                }
            }
        }
    }
    if plans.is_empty() {
        return Vec::new();
    }

    // ---- the codecs of what the streams carry ----
    {
        let names: Vec<String> = names.iter().cloned().collect();
        let first = plans[0].program;
        let builtins: Vec<(String, Vec<(String, hale_syntax::ast::PrimType)>)> =
            builtins.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
        hale_syntax::json_gen::generate_rpc_codecs(programs, first, &names, &builtins);
    }

    // ---- the generated items ----
    let ro_snapshot: Vec<Program> = programs.iter().map(|p| (**p).clone()).collect();
    let ro_refs: Vec<&Program> = ro_snapshot.iter().collect();
    let slices: Vec<&[TopDecl]> = ro_refs.iter().map(|p| p.items.as_slice()).collect();
    let shapes = Shapes::of_all(&slices);
    let codec = crate::rpc_expand::Codec::new(&shapes, &convs);
    let mut generated = String::new();
    for plan in &plans {
        generated.push_str(&rows_src(plan, &confer));
        for (i, s) in plan.streams.iter().enumerate() {
            generated.push_str(&codec_src(plan.id, i, s, &codec));
        }
    }

    // ---- mutate: the main loci ----
    let mut out: Vec<HubExpansion> = Vec::new();
    for plan in &plans {
        let program = &mut *programs[plan.program];
        let Some(l) = find_locus_mut(&mut program.items, &plan.locus) else { continue };
        let span = l.name.span;
        // the hub's literal: the standard library's, numbered, with its rows
        // and the extensions of its sources
        for m in l.members.iter_mut() {
            let LocusMember::Params(pb) = m else { continue };
            for p in pb.params.iter_mut().filter(|p| p.name.name == plan.param) {
                let std_path = ["std", "api", plan.kind, "Hub"];
                if let Some(ty) = &mut p.ty {
                    if let TypeExpr::Named { path, .. } = ty {
                        if is_hub(path) {
                            path.segments = std_path.iter().map(|n| Ident::new(*n, span)).collect();
                        }
                    }
                }
                let ParamInit::Value(Expr::Struct { path, inits, .. }) = &mut p.init else { continue };
                if !is_hub(path) {
                    continue;
                }
                path.segments = std_path.iter().map(|n| Ident::new(*n, span)).collect();
                // the exposure's name is `as:`; a locus has no param of that name
                for i in inits.iter_mut().filter(|i| i.name.name == "as") {
                    i.name = Ident::new("name", i.name.span);
                }
                let mut push = |name: &str, value: Expr| {
                    inits.push(StructInit { name: Ident::new(name, span), value, span });
                };
                push("tid", lit_int(plan.id, span));
                push("kind", Expr::Literal(Literal::String(plan.kind.to_string()), span));
                if let Some(rows) = parse_expr(&format!("__HubRows_{} {{ }}", plan.id)) {
                    push("rows", rows);
                }
                if let Some(pr) = &plan.principals {
                    if plan.expiring {
                        push("expiring", pr.clone());
                        push("has_expiry", Expr::Literal(Literal::Bool(true), span));
                    }
                }
                if let Some(r) = &plan.roles {
                    if plan.revised {
                        push("revised", r.clone());
                        push("has_revised", Expr::Literal(Literal::Bool(true), span));
                    }
                }
            }
        }
        // the listener: a param born after the authored ones (the address may
        // read one), on a pool of its own
        if let Some((lp, entry)) = parse_listener(plan.id, plan.kind, fresh(&plan.bind), span) {
            if let Some(LocusMember::Params(pb)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Params(_))) {
                pb.params.push(lp);
            }
            if let Some(LocusMember::Placement(pl)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Placement(_))) {
                pl.entries.push(entry);
            } else {
                l.members.push(LocusMember::Placement(hale_syntax::ast::PlacementBlock { entries: vec![entry], span }));
            }
        }
        // the adapter bindings, one per stream
        if let Some(LocusMember::Bindings(bb)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Bindings(_))) {
            for (i, s) in plan.streams.iter().enumerate() {
                if let Some(mut entry) = parse_binding(plan.id, i, &s.topic, s.span) {
                    entry.topic = s.topic.clone();
                    bb.entries.push(entry);
                }
            }
        }
        out.push(HubExpansion { id: plan.id, name: plan.name.clone(), locus: plan.locus.clone(), param: plan.param.clone() });
    }

    // ---- the generated top-level items, beside the first hub's program ----
    let first = plans[0].program;
    match parse_source_at(&generated, API_SYNTH_BASE + 0x0900_0000) {
        Ok(g) => programs[first].items.extend(g.items),
        Err(ds) => eprintln!(
            "hub_expand: generated source did not parse: {}\n{}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; "),
            generated
        ),
    }
    out
}

fn parse_expr(src: &str) -> Option<Expr> {
    let text = format!("main locus __Tmp {{ params {{ __x: Int = {src}; }} }}\n");
    let prog = parse_source_at(&text, API_SYNTH_BASE + 0x0800_0000).ok()?;
    for item in prog.items {
        if let TopDecl::Locus(l) = item {
            for m in l.members {
                if let LocusMember::Params(pb) = m {
                    if let Some(ParamDecl { init: ParamInit::Value(e), .. }) = pb.params.into_iter().next() {
                        return Some(e);
                    }
                }
            }
        }
    }
    None
}

/// The listener of hub `id` and its placement.
fn parse_listener(id: i64, kind: &str, bind: Expr, span: Span) -> Option<(ParamDecl, hale_syntax::ast::PlacementEntry)> {
    let field = format!("__rpc_w_{id}");
    let ty = if kind == "udp" { "__StdApiUdpEndpoint" } else { "__StdApiWsListener" };
    let src = format!(
        "main locus __Tmp {{ params {{ {field}: {ty} = {ty} {{ tid: {id} }}; }} placement {{ {field}: cooperative(pool = __api_ws) where async_io; }} }}\n"
    );
    let prog = parse_source_at(&src, API_SYNTH_BASE + 0x0A00_0000 + id as u32 * 0x100).ok()?;
    let mut param = None;
    let mut entry = None;
    for item in prog.items {
        if let TopDecl::Locus(l) = item {
            for m in l.members {
                match m {
                    LocusMember::Params(pb) => param = pb.params.into_iter().next(),
                    LocusMember::Placement(pl) => entry = pl.entries.into_iter().next(),
                    _ => {}
                }
            }
        }
    }
    let mut param = param?;
    if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut param.init {
        inits.push(StructInit { name: Ident::new("bind", span), value: bind, span });
    }
    Some((param, entry?))
}

/// The adapter binding of stream `i` of hub `id`.
fn parse_binding(id: i64, i: usize, topic: &Ident, span: Span) -> Option<hale_syntax::ast::BindingEntry> {
    let src = format!(
        "main locus __Tmp {{ bindings {{ Topic: WsStream {{ hub: {id}, stream: {i} }} codec(__ApiHubCodec_{id}_{i} {{ }}); }} }}\n"
    );
    let _ = (topic, span);
    let prog = match parse_source_at(&src, API_SYNTH_BASE + 0x0B00_0000 + id as u32 * 0x1000 + i as u32 * 0x100) {
        Ok(p) => p,
        Err(ds) => {
            eprintln!("hub_expand: binding did not parse: {}\n{}", ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; "), src);
            return None;
        }
    };
    for item in prog.items {
        if let TopDecl::Locus(l) = item {
            for m in l.members {
                if let LocusMember::Bindings(bb) = m {
                    // the parser takes a capitalized adapter name; the stdlib's
                    // is spelled with its prefix
                    let mut entry = bb.entries.into_iter().next()?;
                    if let hale_syntax::ast::TransportSpec::Adapter { locus, .. } = &mut entry.transport {
                        locus.name = "__StdApiHubStream".to_string();
                    }
                    return Some(entry);
                }
            }
        }
    }
    None
}

/// The rows adapter of hub `plan.id`.
fn rows_src(plan: &HubPlan, confer: &BTreeMap<String, Vec<String>>) -> String {
    let mut s = String::new();
    s.push_str(&format!("locus __HubRows_{} {{\n", plan.id));
    s.push_str(&format!("    fn count() -> Int {{ return {}; }}\n", plan.streams.len()));
    s.push_str("    fn topic(i: Int) -> String {\n");
    for (i, st) in plan.streams.iter().enumerate() {
        s.push_str(&format!("        if i == {i} {{ return {}; }}\n", q(&st.topic.name)));
    }
    s.push_str("        return \"\";\n    }\n    fn requires(i: Int) -> String {\n");
    for (i, st) in plan.streams.iter().enumerate() {
        if !st.requires.is_empty() {
            s.push_str(&format!("        if i == {i} {{ return {}; }}\n", q(&st.requires.join(","))));
        }
    }
    s.push_str("        return \"\";\n    }\n    fn bound(i: Int) -> Int {\n");
    for (i, st) in plan.streams.iter().enumerate() {
        s.push_str(&format!("        if i == {i} {{ return {}; }}\n", st.bound));
    }
    s.push_str("        return 0;\n    }\n    fn on_full(i: Int) -> String {\n");
    for (i, st) in plan.streams.iter().enumerate() {
        s.push_str(&format!("        if i == {i} {{ return {}; }}\n", q(&st.on_full)));
    }
    s.push_str("        return \"\";\n    }\n    fn find(topic: String) -> Int {\n");
    for (i, st) in plan.streams.iter().enumerate() {
        s.push_str(&format!("        if topic == {} {{ return {i}; }}\n", q(&st.topic.name)));
    }
    s.push_str("        return -1;\n    }\n    fn grants(role: String) -> String {\n");
    for (role, who) in confer {
        if who.len() > 1 {
            s.push_str(&format!("        if role == {} {{ return {}; }}\n", q(role), q(&who.join(","))));
        }
    }
    s.push_str("        return role;\n    }\n");
    // the description's pieces (`surface_doc::hub_pieces`)
    let pc = &plan.pieces;
    s.push_str(&format!("    fn head() -> String {{ return {}; }}\n", q(&pc.head)));
    s.push_str("    fn stream_doc(i: Int) -> String {\n");
    for (i, d) in pc.streams.iter().enumerate() {
        s.push_str(&format!("        if i == {i} {{ return {}; }}\n", q(d)));
    }
    s.push_str("        return \"\";\n    }\n");
    s.push_str(&format!("    fn types() -> Int {{ return {}; }}\n", pc.types.len()));
    s.push_str("    fn type_doc(j: Int) -> String {\n");
    for (j, (_, d, _)) in pc.types.iter().enumerate() {
        s.push_str(&format!("        if j == {j} {{ return {}; }}\n", q(d)));
    }
    s.push_str("        return \"\";\n    }\n    fn type_mask(j: Int) -> Int {\n");
    for (j, (_, _, m)) in pc.types.iter().enumerate() {
        s.push_str(&format!("        if j == {j} {{ return {m}; }}\n"));
    }
    s.push_str("        return 0;\n    }\n");
    s.push_str(&format!("    fn outcomes() -> String {{ return {}; }}\n", q(&pc.outcomes)));
    s.push_str(&format!("    fn notes() -> String {{ return {}; }}\n", q(&pc.notes)));
    s.push_str(&format!("    fn required() -> String {{ return {}; }}\n", q(&pc.required.join(","))));
    s.push_str("}\n");
    s
}

/// The JSON codec of stream `i` of hub `id`: its payload as the exposure's
/// codec writes it (spec/api.md § Codecs).
fn codec_src(id: i64, i: usize, st: &StreamPlan, codec: &crate::rpc_expand::Codec<'_>) -> String {
    let ty = crate::surfaces::type_text(&st.payload);
    let bail = "fail __StdApiHubCodecError { kind: \"decode\" };";
    format!(
        "locus __ApiHubCodec_{id}_{i} {{\n    fn encode(v: {ty}) -> Bytes fallible(__StdApiHubCodecError) {{\n        return std::bytes::from_string({enc});\n    }}\n    fn decode(b: Bytes) -> {ty} fallible(__StdApiHubCodecError) {{\n        {dec}        return __v;\n    }}\n}}\n",
        enc = codec.encode(&st.payload, "v"),
        dec = codec.decode(&st.payload, "std::str::from_bytes(b)", "__v", bail),
    )
}
