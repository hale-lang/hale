//! GH #476 Change 2 — derive the canonical `ApplicationModel` from a
//! checked bundle.
//!
//! One entry point, [`derive_application_model_over`], assembling the
//! model from the SAME trusted analyses the topology artifact
//! consumes today: `AllocSummary` (calls, sites, unresolved
//! residue), `BusGraph` (endpoints with spans), `model::Model`
//! (decl provenance, phases, seeds), effects/frontier
//! (classifications), plus direct AST reads for the facts no
//! summary carries yet (topic key/bound policy, subscription
//! filters and bounds, group selectors, the declaration universe,
//! `@sealed`). Extraction recipes deliberately mirror
//! `topology::dump_topology`'s — the Change-3 projection must
//! reproduce that artifact from this value, and the differential
//! tests in `tests/model_builder.rs` hold the two extractions to
//! agreement until Change 3 folds the artifact's own gathering
//! into this builder (strangler order: introduce → adapt →
//! migrate → THEN deduplicate).
//!
//! ## Demand
//!
//! Nothing calls this on the ordinary check path. The model is
//! built only when a consumer asks (`hale model dump` today; the
//! claims evaluator from Change 5a; the artifact encoder from
//! Change 3). The frontend's snapshot counts its derivations per
//! snapshot (`Snapshot::builds`, read by `tests/demand_gate.rs`), and
//! `HALE_MODEL_TRACE=1` prints one stderr line per derivation —
//! the cross-process hook the no-claims check test uses to prove the
//! builder never ran ("cached" must not become "always built").
//!
//! ## What Change 2 deliberately leaves empty
//!
//! Locus instances, ownership, placement, thread domains, and
//! bindings stay empty tables with their capabilities `false`:
//! no current fragment exports them (the artifact carries none),
//! the ownership facts live procedurally in codegen, and inventing
//! loss semantics for adapter bindings would be guessing. They are
//! Change 8's completion work — the schema seats exist so nothing
//! must be retrofitted.

use std::collections::{BTreeMap, BTreeSet};

use hale_model::{
    ApplicationModel, Call, Capabilities, DeadInterfaceCall, DeclKind,
    Declaration, DeclaredIn, DispatchKind, Entities, EntityRef, Function,
    FunctionId, FunctionKind, Group, GroupId, GroupMember, GroupSelector,
    Hole, HoleKind, InterfaceDecl, InterfaceDeclId, KeyDomain,
    KeyOnUnmatched, KeyPredicate, LabelRow, LocusDecl, LocusDeclId,
    MemberOf, ModelHeader, PayloadContract, PayloadContractId, Phase,
    PhaseId, PhaseOf, Provenance, ProvenanceId, ProvenanceTable, Publish,
    PublishDisposition, Relations, Seed, SeedId, SelectorForm, Subject,
    SubjectId, Subscribe, Supervises, SupervisedRef, SupervisionPolicy,
    Topic, TopicBound, TopicId, TopicKey, TopicOnFull, TypeDecl as MTypeDecl,
    TypeDeclId, MODEL_SEMANTICS_V1,
};
use hale_syntax::ast::{
    Block, BusMember, BusSubject, ElseBranch, Expr, GroupDecl,
    KeyFilter, Literal, LocusDecl as AstLocusDecl, LocusMember,
    Program,
    ShedPolicy as AstShedPolicy, Stmt, TopDecl, TopicDecl, TypeExpr,
    UnmatchedPolicy,
};

use crate::alloc_summary::{self, Callee, EffectSiteKind, FnKey};
use crate::handler_routing::ChildRef;
use crate::symbol::Bundle;

/// The dispatch group a call row shares its site with. The model has no
/// function-value dispatch kind (its one shared-site dispatch is an
/// interface's, and its laws hold a shared site to one interface and
/// method), so each alternative of a function-value dispatch (E5,
/// `CallEdge::via_value`) is a direct call at a site of its own: the
/// calls stay exact, and a counting claim over them sums where the
/// `@budget` engines take the max.
fn model_group(edge: &alloc_summary::CallEdge) -> Option<u32> {
    if edge.via_value.is_some() {
        None
    } else {
        edge.dispatch_group
    }
}

/// A named TypeExpr's raw path (joined `::`), `"?"` otherwise.
fn te_name_of(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named { path, .. } => path
            .segments
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join("::"),
        _ => "?".to_string(),
    }
}

/// Derive the canonical application model. Pure with respect to
/// the bundle; every returned value passes
/// `ApplicationModel::validate()` (asserted in tests over the
/// corpus, and cheap enough to keep as a debug assertion here).
/// A source-independent structural descriptor for a payload type
/// expression that is not a bare named struct — the identity of an
/// `opaque:` payload contract. Raw canonical names only (path
/// segments as merged, no demangling), so the same type spelled
/// through different importer aliases descriptorizes identically,
/// and distinct primitive / array / tuple / fn forms stay distinct
/// (round 9 — these previously collapsed to `opaque:?`).
fn type_descriptor(ty: &TypeExpr) -> String {
    fn prim(p: &hale_syntax::ast::PrimType) -> &'static str {
        use hale_syntax::ast::PrimType as P;
        match p {
            P::Int => "Int",
            P::Uint => "Uint",
            P::Float => "Float",
            P::Bool => "Bool",
            P::Decimal => "Decimal",
            P::Time => "Time",
            P::Duration => "Duration",
            P::String => "String",
            P::StringView => "StringView",
            P::Bytes => "Bytes",
            P::BytesView => "BytesView",
            P::BytesMut => "BytesMut",
        }
    }
    match ty {
        TypeExpr::Primitive(p, _) => prim(p).to_string(),
        TypeExpr::Named { path, generic_args, .. } => {
            let base = path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join("::");
            if generic_args.is_empty() {
                base
            } else {
                format!(
                    "{}<{}>",
                    base,
                    generic_args
                        .iter()
                        .map(type_descriptor)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
        TypeExpr::Projection { class, inner, .. } => format!(
            "projection:{:?}({})",
            class,
            type_descriptor(inner)
        ),
        TypeExpr::Array { elem, size, .. } => {
            // A literal length is part of the identity; a computed
            // one degrades to `_` (still elem-distinct).
            let n = match size {
                Some(hale_syntax::ast::Expr::Literal(
                    hale_syntax::ast::Literal::Int(v),
                    _,
                )) => format!("; {}", v),
                Some(_) => "; _".to_string(),
                None => String::new(),
            };
            format!("[{}{}]", type_descriptor(elem), n)
        }
        TypeExpr::Bounded { elem, cap, .. } => {
            format!("bounded[{}; {}]", type_descriptor(elem), cap)
        }
        TypeExpr::Tuple(ts, _) => format!(
            "({})",
            ts.iter()
                .map(type_descriptor)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::Function { params, ret, .. } => format!(
            "fn({}){}",
            params
                .iter()
                .map(type_descriptor)
                .collect::<Vec<_>>()
                .join(","),
            match ret {
                Some(r) => format!(" -> {}", type_descriptor(r)),
                None => String::new(),
            }
        ),
        TypeExpr::Perspective { name, .. } => {
            format!("perspective({})", name.name)
        }
    }
}

/// The model of a bundle no snapshot holds, built where the families it
/// reads are built for it ([`crate::derive_application_model`]).
pub use crate::derive_application_model;

/// What the model reads from the families it does not own, each built
/// once over the CHECKED programs (F.40 phase 2.3): the top scope with
/// its topic rows, the bus graph, the ownership graph, the handler
/// rows, the effect rows, the form rows and the placement table. The
/// frontend's snapshot demands each as a family of its own
/// (`Snapshot::demand_scope`, `demand_bus_graph`,
/// `demand_ownership_graph`, `demand_handlers`, `demand_effects`,
/// `demand_forms`, `demand_placement`) and hands them here; the model
/// builds none of them.
pub struct ModelInputs<'a> {
    pub top: &'a crate::resolve::TopScope,
    pub bus_graph: &'a crate::bus_graph::BusGraph,
    pub ownership: &'a crate::ownership_graph::OwnershipGraph,
    pub handlers: &'a crate::handler_routing::HandlerRouting,
    /// The effect rows (F.40 phase 3, E1): each fn's effect set and its
    /// lower bound, and the stdlib-merged summary their walk read, which
    /// the model's attribution and absorbed-path walks read too.
    pub effects: &'a crate::effect_rows::EffectRows,
    /// The form rows (F.40 phase 3, C1): which forms carry a `sync`
    /// discipline, inference's included, for each locus's `sync_form`.
    pub forms: &'a crate::form_rows::FormRows,
    /// The binding rows (F.40 phase 3, P2): main's entries with the
    /// role the ends decide, for the binding thread domains.
    pub bindings: &'a crate::binding_rows::BindingRows,
    /// The placement table (F.40 phase 3, P1): the arrangement's
    /// instances, ownership edges and domains are its rows, projected.
    pub placement: &'a crate::placement::PlacementTable,
}

/// The application model of `bundle`, over the families `inputs` holds.
pub fn derive_application_model_over(
    bundle: &Bundle<'_>,
    inputs: &ModelInputs<'_>,
) -> ApplicationModel {
    if std::env::var("HALE_MODEL_TRACE").as_deref() == Ok("1") {
        eprintln!("[hale-model] deriving ApplicationModel");
    }

    let programs: Vec<&Program> =
        bundle.programs.values().copied().collect();
    // GH #1159: the rename table once per derivation, not per string.
    let rename_table = crate::stdlib_bodies::demangle_table(&bundle.import_renames);
    let graph = inputs.bus_graph;
    // The summary the effect rows' walk read: the checked programs with
    // the stdlib's analysis copy beside them, cross-seed calls resolved.
    let merged: &alloc_summary::AllocSummary = &inputs.effects.summary;
    // The model is a user-program model: its rows are the summary's own,
    // and a call into the stdlib's analysis copy is the unresolved call
    // (the same site, row and hole) the program alone decides, so the
    // copy never moves `shape_hash`, the build and replay identity.
    let summary = merged.own_rows();
    let vmodel =
        crate::model::Model::derive(&programs, &bundle.import_renames);
    let effect_classes = &inputs.effects.classes;
    let effect_names = effect_classes.names();
    let ffi = &inputs.effects.ffi;

    // ---- author-spelling map (same recipe as dump_topology) ----
    let demangle: BTreeMap<&str, String> = bundle
        .import_renames
        .iter()
        .map(|(segs, mangled)| (mangled.as_str(), segs.join("::")))
        .collect();
    let name = |n: &str| -> String {
        demangle.get(n).cloned().unwrap_or_else(|| n.to_string())
    };
    // The inverse: author-spelled path ("alias::helper") → raw
    // post-merge symbol. An unresolved call can carry the AUTHOR
    // spelling while the model's tables are keyed raw (round 10).
    let remangle: BTreeMap<String, String> = demangle
        .iter()
        .map(|(m, d)| (d.clone(), (*m).to_string()))
        .collect();
    // CANONICAL identity is the RAW post-merge symbol (stable across
    // importers — `p::Store` vs `db::Store` must be one identity);
    // author spelling lives in per-row `display` fields and nothing
    // else (review round 7). fn_name is raw; fn_display demangles
    // both halves.
    let fn_name = |k: &FnKey| -> String {
        match &k.locus {
            Some(l) => format!("{}::{}", l, k.fn_name),
            None => k.fn_name.clone(),
        }
    };
    let fn_display = |k: &FnKey| -> String {
        match &k.locus {
            Some(l) => format!("{}::{}", name(l), k.fn_name),
            None => name(&k.fn_name),
        }
    };

    // ---- AST walks: the declaration universe ----
    struct Decls<'a> {
        loci: Vec<&'a AstLocusDecl>,
        topics: Vec<&'a TopicDecl>,
        groups: Vec<&'a GroupDecl>,
        types: Vec<(&'a str, hale_syntax::Span)>,
        interfaces: Vec<(&'a str, hale_syntax::Span)>,
        others: Vec<(DeclKind, &'a str, hale_syntax::Span)>,
        free_fns: BTreeSet<&'a str>,
    }
    fn walk_decls<'a>(items: &'a [TopDecl], out: &mut Decls<'a>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => out.loci.push(l),
                TopDecl::Topic(t) => out.topics.push(t),
                TopDecl::Group(g) => out.groups.push(g),
                TopDecl::Role(_) => {}
                TopDecl::Type(t) => {
                    out.types.push((t.name.name.as_str(), t.name.span))
                }
                TopDecl::Interface(i) => out
                    .interfaces
                    .push((i.name.name.as_str(), i.name.span)),
                TopDecl::Fn(f) => {
                    out.free_fns.insert(f.name.name.as_str());
                }
                TopDecl::Perspective(p) => out.others.push((
                    DeclKind::Perspective,
                    p.name.name.as_str(),
                    p.name.span,
                )),
                TopDecl::Const(c) => out.others.push((
                    DeclKind::Const,
                    c.name.name.as_str(),
                    c.name.span,
                )),
                TopDecl::RingLayout(r) => out.others.push((
                    DeclKind::RingLayout,
                    r.name.name.as_str(),
                    r.name.span,
                )),
                TopDecl::Target(t) => out.others.push((
                    DeclKind::Target,
                    t.name.name.as_str(),
                    t.name.span,
                )),
                TopDecl::Module(m) => walk_decls(&m.items, out),
                _ => {}
            }
        }
    }
    let mut ast = Decls {
        loci: Vec::new(),
        topics: Vec::new(),
        groups: Vec::new(),
        types: Vec::new(),
        interfaces: Vec::new(),
        others: Vec::new(),
        free_fns: BTreeSet::new(),
    };
    for p in &programs {
        walk_decls(&p.items, &mut ast);
    }
    let raw_loci: BTreeSet<&str> =
        ast.loci.iter().map(|l| l.name.name.as_str()).collect();
    // Raw type-name -> decl, for keyed_by field-type resolution.
    let mut type_decl_by_raw: BTreeMap<&str, &hale_syntax::ast::TypeDecl> =
        BTreeMap::new();
    {
        fn walk_types<'a>(
            items: &'a [TopDecl],
            out: &mut BTreeMap<&'a str, &'a hale_syntax::ast::TypeDecl>,
        ) {
            for item in items {
                match item {
                    TopDecl::Type(t) => {
                        out.insert(t.name.name.as_str(), t);
                    }
                    TopDecl::Module(m) => walk_types(&m.items, out),
                    _ => {}
                }
            }
        }
        for pr in &programs {
            walk_types(&pr.items, &mut type_decl_by_raw);
        }
    }
    // The canonical KEY-TYPE name for `keyed_by FIELD` on payload
    // type `raw_ty` — AnyOfType names the key's TYPE (Int, Bool,
    // Time, Duration, Decimal, String, or a no-payload enum's
    // name), never the field. "?" when unresolvable (a hole-free
    // fallback would be a lie; the differential pins the resolved
    // cases and "?" only appears on programs the checker refuses).
    let key_type_of = |raw_ty: &str, field: &str| -> String {
        let Some(td) = type_decl_by_raw.get(raw_ty) else {
            return "?".to_string();
        };
        let hale_syntax::ast::TypeDeclBody::Struct(fields) = &td.body
        else {
            return "?".to_string();
        };
        let Some(f) = fields.iter().find(|f| f.name.name == field)
        else {
            return "?".to_string();
        };
        match &f.ty {
            TypeExpr::Primitive(prim, _) => {
                use hale_syntax::ast::PrimType;
                match prim {
                    PrimType::Int | PrimType::Uint => "Int",
                    PrimType::Bool => "Bool",
                    PrimType::Time => "Time",
                    PrimType::Duration => "Duration",
                    PrimType::Decimal => "Decimal",
                    PrimType::String | PrimType::StringView => "String",
                    _ => "?",
                }
                .to_string()
            }
            TypeExpr::Named { path, .. }
                if path.segments.len() == 1 =>
            {
                // A no-payload enum key routes by tag; its canonical
                // key-type name is the enum's RAW post-merge name —
                // importer-independent, like every identity
                // (round 9; display never enters AnyOfType).
                path.segments[0].name.clone()
            }
            _ => "?".to_string(),
        }
    };
    let user_key = |k: &FnKey| -> bool {
        match &k.locus {
            Some(l) => raw_loci.contains(l.as_str()),
            None => ast.free_fns.contains(k.fn_name.as_str()),
        }
    };

    // ---- provenance interning ----
    let mut prov = ProvenanceTable::default();
    for sf in &bundle.sources {
        prov.sources.push(hale_model::provenance::SourceUnit {
            path: sf.path.clone(),
            digest: sf.digest.clone(),
        });
    }
    let mut prov_map: BTreeMap<(i64, u32, u32), ProvenanceId> =
        BTreeMap::new();
    let mut synth_map: BTreeMap<String, ProvenanceId> = BTreeMap::new();
    // Bundle-global offset -> (source id, local offset); same
    // resolution rule as the artifact's provenance section.
    let sources = bundle.sources.clone();
    let loc = move |pos: u32| -> (i64, u32) {
        match sources
            .iter()
            .filter(|f| hale_syntax::file_owns_offset(f.base, f.len, pos))
            .max_by_key(|f| f.base)
        {
            Some(f) => (f.id as i64, pos - f.base),
            None => (-1, pos),
        }
    };
    let mut intern_span = |records: &mut Vec<Provenance>,
                           span: hale_syntax::Span|
     -> ProvenanceId {
        let s = span.start.as_usize() as u32;
        let e = span.end.as_usize() as u32;
        let (src, ls) = loc(s);
        let (_, le) = loc(e);
        let key = (src, ls, le.max(ls));
        if let Some(id) = prov_map.get(&key) {
            return *id;
        }
        let id = ProvenanceId(records.len() as u32);
        records.push(if src >= 0 {
            Provenance::Source {
                source: hale_model::SourceId(src as u32),
                span: (ls, le.max(ls)),
            }
        } else {
            // Round 2: an unplaceable span keeps its OFFSETS —
            // `ForeignSpan` is verbatim offset-space; collapsing
            // to Synthetic dropped the one thing the provenance
            // section preserves (legacy renders source -1 with the
            // global span).
            Provenance::ForeignSpan {
                span: (s, e.max(s)),
            }
        });
        prov_map.insert(key, id);
        id
    };
    let mut intern_synth = |records: &mut Vec<Provenance>,
                            origin: &str|
     -> ProvenanceId {
        if let Some(id) = synth_map.get(origin) {
            return *id;
        }
        let id = ProvenanceId(records.len() as u32);
        records.push(Provenance::Synthetic {
            origin: origin.to_string(),
        });
        synth_map.insert(origin.to_string(), id);
        id
    };

    // ---- entity tables (canonical order via BTree keys, RAW) ----
    // Loci.
    struct LocusRow {
        sealed: bool,
        span: hale_syntax::Span,
        sync_form: bool,
        /// (param name, declared type's last segment)
        params: Vec<(String, String)>,
    }
    let mut locus_rows: BTreeMap<String, LocusRow> = BTreeMap::new();
    for l in &ast.loci {
        // #340: the `sync` discipline is the form's own, its row's
        // (written or inferred). There is no annotation for it, and
        // there should not be — it is a property of how the form is
        // shared, not a claim about it.
        let sync_form = inputs.forms.synchronizes(l);
        let mut params = Vec::new();
        for m in &l.members {
            let LocusMember::Params(pb) = m else { continue };
            for prm in &pb.params {
                // GH #527 B4: EVERY declared param, not only the
                // locus-typed ones — `hale model diff` compares the
                // params facet of a locus contract, and a widened
                // `limit: Int` is a contract change. A named type
                // keeps its last segment (what `decl` resolves
                // against); any other form renders through the
                // payload descriptor so distinct forms stay
                // distinct.
                let Some(ty) = &prm.ty else { continue };
                let rendered = match ty {
                    TypeExpr::Named { path, generic_args, .. }
                        if generic_args.is_empty() =>
                    {
                        match path.segments.last() {
                            Some(seg) => seg.name.clone(),
                            None => continue,
                        }
                    }
                    other => type_descriptor(other),
                };
                params.push((prm.name.name.clone(), rendered));
            }
        }
        locus_rows.insert(
            l.name.name.clone(),
            LocusRow {
                sealed: l.sealed,
                span: l.name.span,
                sync_form,
                params,
            },
        );
    }
    let locus_id: BTreeMap<&String, LocusDeclId> = locus_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (k, LocusDeclId(i as u32)))
        .collect();

    // Functions: the DECLARATION universe (free fns, methods,
    // lifecycle hooks, modes), unioned with the summary's user keys.
    // The summary is a behavior analysis, not a declaration
    // inventory — an EMPTY free fn has no summary entry but must
    // still exist as an entity (group member, seed member, zero-
    // length reachability endpoint); the claims layer carries the
    // same correction. The union keeps the artifact's fn sort a
    // subset of the model's.
    struct FnInfo {
        kind: FunctionKind,
        locus: Option<String>,
        display: String,
        span: Option<hale_syntax::Span>,
        /// The behavior analysis did not walk this body (an
        /// on_failure handler) — emits an UnanalyzedBody hole.
        unanalyzed: bool,
    }
    fn hook_name(k: &hale_syntax::ast::LifecycleKind) -> &'static str {
        use hale_syntax::ast::LifecycleKind as LK;
        match k {
            LK::Birth => "birth",
            LK::Accept => "accept",
            LK::Release => "release",
            LK::Run => "run",
            LK::Drain => "drain",
            LK::Dissolve => "dissolve",
        }
    }
    fn mode_name(k: &hale_syntax::ast::ModeKind) -> &'static str {
        use hale_syntax::ast::ModeKind as MK;
        match k {
            MK::Bulk => "bulk",
            MK::Harmonic => "harmonic",
            MK::Resolution => "resolution",
        }
    }
    let mut fn_rows: BTreeMap<String, FnInfo> = BTreeMap::new();
    // The failure handlers' rows, keyed by the handler's site (the
    // routing row's identity, `HandlerRow::is_row_of`), each with its
    // name: two handlers are two rows whatever their signatures spell.
    // A handler nothing minted keys by its name, in `fn_rows`.
    let mut handler_fn_rows: BTreeMap<u32, (String, FnInfo)> = BTreeMap::new();
    {
        // A declaration inside a module is analyzed like one at the top
        // level: the behavior summary collects module-nested bodies
        // (F.40 phase 3, E3a part C), so they no longer hole out as
        // unanalyzed (review round 7's Change 2 shape).
        let mut frees = Vec::new();
        let mut mod_loci = Vec::new();
        for pr in &programs {
            for item in hale_syntax::ast::flat_decls(&pr.items) {
                match item {
                    TopDecl::Fn(f) => frees.push((f.name.name.as_str(), f.name.span)),
                    TopDecl::Locus(l) => mod_loci.push(l),
                    _ => {}
                }
            }
        }
        for (n, sp) in frees {
            fn_rows.insert(
                n.to_string(),
                FnInfo {
                    kind: FunctionKind::Free,
                    locus: None,
                    display: name(n),
                    span: Some(sp),
                    unanalyzed: false,
                },
            );
        }
        for l in &mod_loci {
            let ld = l.name.name.clone();
            let ld_display = name(&ld);
            for m in &l.members {
                let (fname, kind, sp) = match m {
                    LocusMember::Fn(f) => (
                        f.name.name.clone(),
                        FunctionKind::Method,
                        f.name.span,
                    ),
                    // The model lists the hooks the author wrote, not
                    // the omitted `run` (`LifecycleDecl::synthesized`).
                    LocusMember::Lifecycle(lc) if !lc.synthesized => (
                        hook_name(&lc.kind).to_string(),
                        FunctionKind::Hook,
                        lc.span,
                    ),
                    LocusMember::Mode(md) => (
                        mode_name(&md.kind).to_string(),
                        FunctionKind::Mode,
                        md.span,
                    ),
                    LocusMember::Failure(fd) => {
                        // on_failure handlers ARE executable hooks;
                        // the summary never walks them, so they
                        // enter the universe with an UnanalyzedBody
                        // hole. The row is the handler's, by its
                        // site; its name spells the (child, err)
                        // signature.
                        let sig = fd
                            .params
                            .iter()
                            .map(|pa| te_name_of(&pa.ty))
                            .collect::<Vec<_>>()
                            .join(",");
                        let handler_name = format!("{}::on_failure({})", ld, sig);
                        let row = FnInfo {
                            kind: FunctionKind::FailureHandler,
                            locus: Some(ld.clone()),
                            display: format!(
                                "{}::on_failure({})",
                                ld_display,
                                fd.params
                                    .iter()
                                    .map(|pa| name(&te_name_of(&pa.ty)))
                                    .collect::<Vec<_>>()
                                    .join(",")
                            ),
                            span: Some(fd.span),
                            unanalyzed: true,
                        };
                        if fd.id.is_none() {
                            fn_rows.insert(handler_name, row);
                        } else {
                            handler_fn_rows.insert(fd.id.0, (handler_name, row));
                        }
                        continue;
                    }
                    _ => continue,
                };
                fn_rows.insert(
                    format!("{}::{}", ld, fname),
                    FnInfo {
                        kind,
                        locus: Some(ld.clone()),
                        display: format!("{}::{}", ld_display, fname),
                        span: Some(sp),
                        unanalyzed: false,
                    },
                );
            }
        }
        // Union: summary keys the enumeration missed (analysis-
        // synthesized shapes) join with kind inferred from phases.
        for k in summary.fns.keys() {
            if !user_key(k) {
                continue;
            }
            fn_rows.entry(fn_name(k)).or_insert_with(|| FnInfo {
                kind: match vmodel.phases.get(k) {
                    Some(p) if p.hook => FunctionKind::Hook,
                    Some(_) => FunctionKind::Method,
                    None => FunctionKind::Free,
                },
                locus: k.locus.clone(),
                display: fn_display(k),
                span: None,
                unanalyzed: false,
            });
        }
    }
    // Round 11: the summarized set — behavior-summary keys, which
    // IS the legacy fn sort's universe.
    let summarized_names: BTreeSet<String> = summary
        .fns
        .keys()
        .filter(|k| user_key(k))
        .map(fn_name)
        .collect();
    // The function universe in id order: every row ranked by its name
    // (the handler rows' names among the others'; a name two handler
    // rows share ranks them by site). `fn_id` answers a name for the
    // rows keyed by one, never for a handler's.
    let mut fn_universe: Vec<(&String, &FnInfo, Option<u32>)> = fn_rows
        .iter()
        .map(|(n, info)| (n, info, None))
        .chain(handler_fn_rows.iter().map(|(site, (n, info))| (n, info, Some(*site))))
        .collect();
    fn_universe.sort_by(|a, b| (a.0, a.2).cmp(&(b.0, b.2)));
    let fn_universe: Vec<(FunctionId, &String, &FnInfo)> = fn_universe
        .into_iter()
        .enumerate()
        .map(|(i, (n, info, _))| (FunctionId(i as u32), n, info))
        .collect();
    let fn_id: BTreeMap<&String, FunctionId> = fn_universe
        .iter()
        .filter(|(_, n, _)| fn_rows.contains_key(*n))
        .map(|(id, n, _)| (*n, *id))
        .collect();

    // Phases (distinct names) + phase_of.
    let mut phase_names: BTreeSet<String> = BTreeSet::new();
    let mut phase_of_pairs: BTreeMap<String, (String, bool)> =
        BTreeMap::new();
    for (k, p) in &vmodel.phases {
        if user_key(k) {
            phase_names.insert(p.phase.clone());
            phase_of_pairs
                .insert(fn_name(k), (p.phase.clone(), p.hook));
        }
    }
    let phase_id: BTreeMap<&String, PhaseId> = phase_names
        .iter()
        .enumerate()
        .map(|(i, k)| (k, PhaseId(i as u32)))
        .collect();

    // Topics + subjects + payloads. THREE identities per topic, kept
    // apart (the review-1 law, re-learned here in review 6): the RAW
    // post-merge declaration name (what wire_subjects and the graph
    // key by), the DISPLAY spelling (what the model's Topic.name and
    // the artifact's topic sort carry), and the WIRE subject (the
    // byte-exact runtime/recording join key — deliberately RAW in
    // the artifact, never author-spelled).
    struct TInfo<'t> {
        decl: &'t TopicDecl,
        wire: String,
    }
    impl<'t> TInfo<'t> {
        fn raw_payload_ty(&self) -> String {
            match &self.decl.payload {
                TypeExpr::Named { path, .. }
                    if path.segments.len() == 1 =>
                {
                    path.segments[0].name.clone()
                }
                _ => "?".to_string(),
            }
        }
    }
    let all_items: Vec<TopDecl> = programs
        .iter()
        .flat_map(|p| p.items.iter().cloned())
        .collect();
    // Keyed by RAW name — canonical identity — with the author
    // spelling carried alongside.
    let mut topic_decl_by_name: BTreeMap<String, TInfo> = BTreeMap::new();
    for t in &ast.topics {
        let raw = t.name.name.clone();
        // The scope's topic rows are keyed by the RAW name; a
        // subject-less topic's default wire subject is likewise the raw
        // name (parent joins included) — exactly the artifact's rule.
        let wire = inputs
            .top
            .topics
            .named(&raw)
            .map(|row| row.wire.clone())
            .unwrap_or_else(|| raw.clone());
        topic_decl_by_name
            .insert(raw, TInfo { decl: t, wire });
    }

    let mut subject_set: BTreeSet<String> = BTreeSet::new();
    for info in topic_decl_by_name.values() {
        subject_set.insert(info.wire.clone());
    }
    for (subject, _) in &graph.subjects {
        if !topic_decl_by_name.contains_key(subject.as_str()) {
            subject_set.insert(subject.clone());
        }
    }

    // Payload contracts: SHAPE-ONLY identity — the schema separates
    // address identity (Subject) from payload identity, so the
    // fused subject+shape hash the runtime keeps is a Change-3
    // projection (`hash(wire ++ ':' ++ shape)`), never the payload
    // schema. Structural equality across subjects shares one
    // contract; a field change on a literal endpoint's type changes
    // its contract; renaming a type without structural change does
    // not.
    let fnv = |s: &str| -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    };
    // Structural shape of a payload TYPE (by raw post-merge name),
    // through the SAME renderer topics use. A non-bare-struct
    // payload has no canonical shape; its contract falls back to an
    // opaque per-type identity over the RAW name — the raw symbol
    // is path-derived and importer-independent, so `p::Status` and
    // `db::Status` share one contract (round 9; display spelling
    // never enters an identity).
    let shape_of_type = |raw_ty: &str| -> (String, u64, bool) {
        let shape = crate::topic_identity::canonical_type_shape(
            &all_items, raw_ty,
        );
        if shape.is_empty() {
            let opaque = format!("opaque:{}", raw_ty);
            let h = fnv(&opaque);
            (opaque, h, true)
        } else {
            let h = fnv(&shape);
            (shape, h, false)
        }
    };
    // The ONE payload-contract rule for a type EXPRESSION — used by
    // declared topics and explicit `of type T` endpoint clauses
    // alike (round 10; endpoint clauses previously collapsed every
    // non-named form to `opaque:?` through a name-only path):
    //   bare named struct → canonical structural shape;
    //   every other form  → opaque over the structural descriptor.
    let contract_of_te = |te: &TypeExpr| -> (String, u64, bool) {
        match te {
            TypeExpr::Named { path, generic_args, .. }
                if path.segments.len() == 1
                    && generic_args.is_empty() =>
            {
                shape_of_type(&path.segments[0].name)
            }
            other => shape_of_type(&type_descriptor(other)),
        }
    };
    let mut payload_rows: BTreeMap<(String, u64, bool), ()> =
        BTreeMap::new();
    let mut topic_payload: BTreeMap<String, (String, u64, bool)> =
        BTreeMap::new();
    for (tname, info) in &topic_decl_by_name {
        let key = contract_of_te(&info.decl.payload);
        payload_rows.insert(key.clone(), ());
        topic_payload.insert(tname.clone(), key);
    }
    let mut endpoint_payload: BTreeMap<String, (String, u64, bool)> =
        BTreeMap::new();
    for (subject, info) in &graph.subjects {
        let display = subject.clone();
        if topic_decl_by_name.contains_key(&display) {
            continue;
        }
        let ty = info
            .publishers
            .first()
            .map(|p| p.payload.clone())
            .or_else(|| {
                info.subscribers.first().map(|s| s.payload.clone())
            })
            .unwrap_or_else(|| "?".to_string());
        let key = shape_of_type(&ty);
        payload_rows.insert(key.clone(), ());
        endpoint_payload.insert(display, key);
    }
    // Totality: a publish effect-site or bus declaration can name a
    // subject the BusGraph never recorded (an unresolved cross-seed
    // reference, a standalone parse of one seed of a multi-seed
    // app). The builder must be TOTAL over parseable programs —
    // such endpoints get a subject row and an unresolved "?"
    // payload contract rather than a lookup panic. (An unresolvable
    // program fails typecheck; the model of a parseable bundle
    // still exists, holes and all.)
    // The unresolved contract is OPAQUE by definition — nothing
    // structural is known about it.
    let unresolved_payload = ("?".to_string(), fnv("?"), true);
    {
        // `known` overrides the "?" fallback with the endpoint's
        // real structural contract (an explicit `of type T`) — one
        // closure owns ALL mutation so the borrows stay linear.
        // `literal` is the SYNTACTIC form (round 10): a string-
        // literal subject is a wire address and NEVER resolves
        // through the topic table, even when its text collides with
        // a topic NAME — only a topic REFERENCE takes the early
        // return.
        let mut need = |display: String,
                        literal: bool,
                        known: Option<(String, u64, bool)>| {
            if !literal && topic_decl_by_name.contains_key(&display) {
                return;
            }
            subject_set.insert(display.clone());
            match known {
                Some(key) => {
                    payload_rows.insert(key.clone(), ());
                    endpoint_payload.insert(display, key);
                }
                None => {
                    if !endpoint_payload.contains_key(&display) {
                        payload_rows
                            .insert(unresolved_payload.clone(), ());
                        endpoint_payload.insert(
                            display,
                            unresolved_payload.clone(),
                        );
                    }
                }
            }
        };
        for (k, fs) in &summary.fns {
            if !user_key(k) {
                continue;
            }
            for site in &fs.effect_sites {
                if let EffectSiteKind::Publish(Some(subj)) = &site.kind
                {
                    need(subj.text.clone(), subj.literal, None);
                }
            }
        }
        for l in &ast.loci {
            for m in &l.members {
                let LocusMember::Bus(bus) = m else { continue };
                for bm in &bus.members {
                    match bm {
                        BusMember::Subscribe {
                            subject, ty, ..
                        }
                        | BusMember::Publish { subject, ty, .. } => {
                            // An explicit `of type T` names the
                            // endpoint's real structural contract —
                            // registered so every consumer (graph,
                            // sends, declared ends) shares one key,
                            // through the ONE type-expression rule.
                            need(
                                subject.canonical().to_string(),
                                matches!(
                                    subject,
                                    BusSubject::Literal { .. }
                                ),
                                ty.as_ref().map(&contract_of_te),
                            );
                        }
                    }
                }
            }
        }
    }
    let subject_id: BTreeMap<&String, SubjectId> = subject_set
        .iter()
        .enumerate()
        .map(|(i, k)| (k, SubjectId(i as u32)))
        .collect();
    let payload_id: BTreeMap<&(String, u64, bool), PayloadContractId> =
        payload_rows
            .keys()
            .enumerate()
            .map(|(i, k)| (k, PayloadContractId(i as u32)))
            .collect();

    let topic_names: Vec<String> =
        topic_decl_by_name.keys().cloned().collect();
    let wire_of = |display: &str| -> String {
        topic_decl_by_name[display].wire.clone()
    };
    let topic_id: BTreeMap<&String, TopicId> = topic_names
        .iter()
        .enumerate()
        .map(|(i, k)| (k, TopicId(i as u32)))
        .collect();

    // Seeds: the rename-table aliases, PLUS any alias a group glob
    // names that resolution never populated (a standalone parse of
    // one seed) — the authored alias exists even when its members
    // are unknown, and a dangling-id fail-open here was exactly the
    // corpus property's first catch.
    let mut seed_name_set: BTreeSet<String> =
        vmodel.seeds.keys().cloned().collect();
    for g in &ast.groups {
        for m in &g.members {
            if m.glob {
                if let Some(a) = m.segments.first() {
                    seed_name_set.insert(a.name.clone());
                }
            }
        }
    }
    let seed_names: Vec<String> = seed_name_set.into_iter().collect();
    let seed_id: BTreeMap<&String, SeedId> = seed_names
        .iter()
        .enumerate()
        .map(|(i, k)| (k, SeedId(i as u32)))
        .collect();

    // Groups, keyed raw.
    let mut group_rows: BTreeMap<String, &GroupDecl> = BTreeMap::new();
    for g in &ast.groups {
        group_rows.insert(g.name.name.clone(), g);
    }
    let group_id: BTreeMap<&String, GroupId> = group_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (k, GroupId(i as u32)))
        .collect();

    // Types / interfaces / other declarations (display-spelled).
    let mut type_rows: BTreeMap<String, hale_syntax::Span> =
        BTreeMap::new();
    for (n, sp) in &ast.types {
        type_rows.insert(n.to_string(), *sp);
    }
    let type_id: BTreeMap<&String, TypeDeclId> = type_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (k, TypeDeclId(i as u32)))
        .collect();
    let mut iface_rows: BTreeMap<String, hale_syntax::Span> =
        BTreeMap::new();
    for (n, sp) in &ast.interfaces {
        iface_rows.insert(n.to_string(), *sp);
    }
    let iface_id: BTreeMap<&String, InterfaceDeclId> = iface_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (k, InterfaceDeclId(i as u32)))
        .collect();
    let mut other_rows: BTreeMap<(String, DeclKind), hale_syntax::Span> =
        BTreeMap::new();
    for (kind, n, sp) in &ast.others {
        other_rows.insert((n.to_string(), *kind), *sp);
    }
    let other_id: BTreeMap<&(String, DeclKind), hale_model::DeclarationId> =
        other_rows
            .keys()
            .enumerate()
            .map(|(i, k)| (k, hale_model::DeclarationId(i as u32)))
            .collect();

    // A universal name -> EntityRef classifier for seed membership
    // and group members, in the artifact's own precedence.
    let entity_of = |display: &str| -> Option<EntityRef> {
        if let Some(id) = locus_id.get(&display.to_string()) {
            return Some(EntityRef::LocusDecl(*id));
        }
        if let Some(id) = fn_id.get(&display.to_string()) {
            return Some(EntityRef::Function(*id));
        }
        if let Some(id) = topic_id.get(&display.to_string()) {
            return Some(EntityRef::Topic(*id));
        }
        if let Some(id) = group_id.get(&display.to_string()) {
            return Some(EntityRef::Group(*id));
        }
        if let Some(id) = type_id.get(&display.to_string()) {
            return Some(EntityRef::Type(*id));
        }
        if let Some(id) = iface_id.get(&display.to_string()) {
            return Some(EntityRef::Interface(*id));
        }
        for kind in [
            DeclKind::Perspective,
            DeclKind::Const,
            DeclKind::RingLayout,
            DeclKind::Target,
        ] {
            if let Some(id) =
                other_id.get(&(display.to_string(), kind))
            {
                return Some(EntityRef::Declaration(*id));
            }
        }
        None
    };

    // ---- relations ----
    let mut records: Vec<Provenance> = Vec::new();

    // calls / dead dispatches / holes, at SITE grain.
    let mut calls: BTreeMap<
        (FunctionId, FunctionId, DispatchKind, u32),
        (bool, bool, ProvenanceId),
    > = BTreeMap::new();
    let mut dead: BTreeMap<(FunctionId, u32), (String, String, ProvenanceId)> =
        BTreeMap::new();
    let mut holes: BTreeMap<
        (EntityRef, HoleKind, String),
        (hale_model::RelationSet, Option<u32>, ProvenanceId),
    > = BTreeMap::new();
    for (k, fs) in &summary.fns {
        if !user_key(k) {
            continue;
        }
        let from = fn_id[&fn_name(k)];
        // Authored-site ordinals: every conformer alternative of ONE
        // interface dispatch shares one dispatch_group and therefore
        // ONE site ordinal (one source expression = one site; a new
        // conformer must not renumber later calls). Unresolved and
        // dead edges consume ordinals too — they are authored sites.
        let mut next_ordinal: u32 = 0;
        let mut group_site: BTreeMap<u32, u32> = BTreeMap::new();
        let mut site_of = |group: Option<u32>| -> u32 {
            match group {
                Some(g) => *group_site.entry(g).or_insert_with(|| {
                    let o = next_ordinal;
                    next_ordinal += 1;
                    o
                }),
                None => {
                    let o = next_ordinal;
                    next_ordinal += 1;
                    o
                }
            }
        };
        for edge in &fs.calls {
            match &edge.callee {
                Callee::Resolved(next) => {
                    let site = site_of(model_group(edge));
                    if !user_key(next) {
                        continue;
                    }
                    let to = fn_id[&fn_name(next)];
                    let dispatch = match &edge.via_interface {
                        Some(i) => DispatchKind::Interface {
                            interface: i.clone(),
                        },
                        None => DispatchKind::Direct,
                    };
                    let pid = intern_span(&mut records, edge.span);
                    calls.insert(
                        (from, to, dispatch, site),
                        (
                            edge.loop_depth > 0,
                            edge.in_unbounded_loop,
                            pid,
                        ),
                    );
                }
                Callee::Unresolved(n) => {
                    let site = site_of(model_group(edge));
                    let anchor = EntityRef::Function(from);
                    let pid = intern_span(&mut records, edge.span);
                    if edge.indirect
                        || fs.fn_params.iter().any(|p| p == n)
                    {
                        holes
                            .entry((
                                anchor,
                                HoleKind::IndirectCall,
                                format!("call through `{}`", n),
                            ))
                            .or_insert((
                                hale_model::RelationSet::CALLS
                                    .union(hale_model::RelationSet::EFFECTS)
                                    // Change 5h: a call whose target
                                    // is chosen by the caller hides
                                    // its COSTS too — this is exactly
                                    // where the budget engines
                                    // saturate rather than count
                                    // zero (#353).
                                    .union(hale_model::RelationSet::COSTS),
                                Some(site),
                                pid,
                            ));
                    } else if let Some(iface) = &edge.via_interface {
                        dead.insert(
                            (from, site),
                            (iface.clone(), n.clone(), pid),
                        );
                    } else if edge.receiver_present
                        && edge.recv_ty.is_none()
                    {
                        holes
                            .entry((
                                anchor,
                                HoleKind::UntypedReceiver {
                                    callee: n.clone(),
                                },
                                format!(
                                    "method call `{}` on untyped \
                                     receiver",
                                    n
                                ),
                            ))
                            .or_insert((
                                hale_model::RelationSet::CALLS
                                    .union(hale_model::RelationSet::EFFECTS)
                                    // #382's untypeable receiver is
                                    // the same rule as the indirect
                                    // call above, in the method
                                    // shape.
                                    .union(hale_model::RelationSet::COSTS),
                                Some(site),
                                pid,
                            ));
                    } else if edge.receiver_present {
                        // A TYPED method miss (`w.tick()` with
                        // `recv_ty` known): resolve by call shape —
                        // raw `RecvTy::method` in the declaration
                        // universe, where a method the summary left
                        // unresolved still lands. A stdlib/external method
                        // that resolves nowhere must NOT wire to a
                        // same-named free fn (round 10); the effect
                        // frontier and the stdlib contraction own
                        // those.
                        if let Some(t) = &edge.recv_ty {
                            let mkey = format!("{}::{}", t, n);
                            if let Some(to) = fn_id.get(&mkey) {
                                calls.insert(
                                    (
                                        from,
                                        *to,
                                        DispatchKind::Direct,
                                        site,
                                    ),
                                    (
                                        edge.loop_depth > 0,
                                        edge.in_unbounded_loop,
                                        pid,
                                    ),
                                );
                            }
                        }
                    } else if let Some(to) =
                        fn_id.get(n).or_else(|| {
                            // An imported qualified miss keeps its
                            // AUTHOR spelling (`alias::helper`);
                            // the universe is keyed raw.
                            remangle
                                .get(n)
                                .and_then(|raw| fn_id.get(raw))
                        })
                    {
                        // The summary resolves callees against its
                        // own analyzed-body set only, so a direct
                        // call to a declared fn it holds no row for
                        // arrives Unresolved (a module-scoped fn's
                        // did until the summary collected
                        // module-nested bodies, F.40 phase 3) — but
                        // the DECLARATION universe knows the target.
                        // The edge is authored fact and must exist
                        // ("a concrete path beats a hole" is
                        // impossible if the path is dropped); a
                        // callee's UnanalyzedBody hole bounds any
                        // reasoning past it (round 9).
                        calls.insert(
                            (
                                from,
                                *to,
                                DispatchKind::Direct,
                                site,
                            ),
                            (
                                edge.loop_depth > 0,
                                edge.in_unbounded_loop,
                                pid,
                            ),
                        );
                    }
                }
            }
        }
    }
    // Through-stdlib contraction — with a TWO-component lattice.
    // "Inside a loop" and "unbounded" are separate facts (a path
    // through a statically bounded loop is looped but NOT
    // unbounded), and a node is revisited whenever EITHER component
    // strengthens, so results cannot depend on traversal order.
    #[derive(Clone, Copy, PartialEq, Eq, Default)]
    struct PathFlags {
        in_loop: bool,
        unbounded: bool,
    }
    impl PathFlags {
        fn join(self, o: PathFlags) -> PathFlags {
            PathFlags {
                in_loop: self.in_loop || o.in_loop,
                unbounded: self.unbounded || o.unbounded,
            }
        }
        fn strengthens(self, prev: PathFlags) -> bool {
            (self.in_loop && !prev.in_loop)
                || (self.unbounded && !prev.unbounded)
        }
    }
    let mut via_stdlib: BTreeMap<(FunctionId, FunctionId), PathFlags> =
        BTreeMap::new();
    for (k, fs) in &merged.fns {
        if !user_key(k) {
            continue;
        }
        let from = fn_id[&fn_name(k)];
        let mut stack: Vec<(FnKey, PathFlags)> = Vec::new();
        let mut seen: BTreeMap<FnKey, PathFlags> = BTreeMap::new();
        let edge_flags = |edge: &crate::alloc_summary::CallEdge| {
            PathFlags {
                in_loop: edge.loop_depth > 0,
                unbounded: edge.in_unbounded_loop,
            }
        };
        for edge in &fs.calls {
            if let Callee::Resolved(next) = &edge.callee {
                if !user_key(next) {
                    let f = edge_flags(edge);
                    let prev =
                        seen.get(next).copied().unwrap_or_default();
                    if !seen.contains_key(next) || f.strengthens(prev)
                    {
                        seen.insert(next.clone(), f.join(prev));
                        stack.push((next.clone(), f));
                    }
                }
            }
        }
        let mut steps = 0u32;
        while let Some((n, lp)) = stack.pop() {
            steps += 1;
            if steps > crate::callgraph::MAX_STEPS {
                break;
            }
            let Some(nfs) = merged.fns.get(&n) else { continue };
            for edge in &nfs.calls {
                let Callee::Resolved(next) = &edge.callee else {
                    continue;
                };
                let f2 = lp.join(edge_flags(edge));
                if user_key(next) {
                    let to = fn_id[&fn_name(next)];
                    let e = via_stdlib
                        .entry((from, to))
                        .or_insert_with(PathFlags::default);
                    *e = e.join(f2);
                } else {
                    let prev =
                        seen.get(next).copied().unwrap_or_default();
                    if !seen.contains_key(next)
                        || f2.strengthens(prev)
                    {
                        seen.insert(next.clone(), f2.join(prev));
                        stack.push((next.clone(), f2));
                    }
                }
            }
        }
    }
    // Contracted edges get site ordinals AFTER the direct sites of
    // their caller (deterministic: sorted by callee id), with
    // synthetic provenance — no single authored location exists.
    {
        let mut next_site: BTreeMap<FunctionId, u32> = BTreeMap::new();
        for ((from, ..), _) in &calls {
            let n = next_site.entry(*from).or_insert(0);
            *n = (*n).max(
                calls
                    .keys()
                    .filter(|(f, ..)| f == from)
                    .map(|(.., s)| s + 1)
                    .max()
                    .unwrap_or(0),
            );
        }
        let contracted: Vec<((FunctionId, FunctionId), PathFlags)> =
            via_stdlib.into_iter().collect();
        for ((from, to), flags) in contracted {
            let pid = intern_synth(
                &mut records,
                "through-stdlib contraction",
            );
            let site = {
                let n = next_site.entry(from).or_insert(0);
                let s = *n;
                *n += 1;
                s
            };
            calls.insert(
                (from, to, DispatchKind::ViaStdlib, site),
                (flags.in_loop, flags.unbounded, pid),
            );
        }
    }

    // Publish dispositions: a span-indexed map of every authored
    // send statement's `or` disposition, joined to effect sites by
    // containment (sends do not nest). Walks EVERY statement
    // container so a send inside a match arm or nested block cannot
    // silently read as Default.
    let mut send_dispositions: Vec<(u32, u32, PublishDisposition)> =
        Vec::new();
    {
        use hale_syntax::ast::OrDisposition;
        fn disp(d: &Option<OrDisposition>) -> PublishDisposition {
            match d {
                None => PublishDisposition::Default,
                Some(OrDisposition::Raise(_)) => {
                    PublishDisposition::Raise
                }
                Some(OrDisposition::Discard(_)) => {
                    PublishDisposition::Discard
                }
                Some(OrDisposition::Wait(_)) => PublishDisposition::Wait,
                // `or fail <p>` / `or <handler-ish expr>` both route
                // the refusal into user code — the Handler class.
                Some(OrDisposition::Fail(..))
                | Some(OrDisposition::Substitute(_)) => {
                    PublishDisposition::Handler
                }
            }
        }
        fn walk_block(
            b: &Block,
            out: &mut Vec<(u32, u32, PublishDisposition)>,
        ) {
            for st in &b.stmts {
                match st {
                    Stmt::Send {
                        or_disposition, span, ..
                    } => out.push((
                        span.start.as_usize() as u32,
                        span.end.as_usize() as u32,
                        disp(or_disposition),
                    )),
                    Stmt::If(i) => {
                        walk_block(&i.then_block, out);
                        let mut cur = i.else_block.as_deref();
                        while let Some(eb) = cur {
                            match eb {
                                ElseBranch::Else(bb) => {
                                    walk_block(bb, out);
                                    cur = None;
                                }
                                ElseBranch::ElseIf(ei) => {
                                    walk_block(&ei.then_block, out);
                                    cur = ei.else_block.as_deref();
                                }
                            }
                        }
                    }
                    Stmt::While { body, .. }
                    | Stmt::For { body, .. } => walk_block(body, out),
                    Stmt::Block(bb) => walk_block(bb, out),
                    Stmt::Match(m) => {
                        for arm in &m.arms {
                            if let hale_syntax::ast::MatchArmBody::Block(
                                bb,
                            ) = &arm.body
                            {
                                walk_block(bb, out);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        fn walk_members(
            items: &[TopDecl],
            out: &mut Vec<(u32, u32, PublishDisposition)>,
        ) {
            for item in items {
                match item {
                    TopDecl::Fn(f) => walk_block(&f.body, out),
                    TopDecl::Locus(l) => {
                        for m in &l.members {
                            match m {
                                LocusMember::Fn(f) => {
                                    walk_block(&f.body, out)
                                }
                                LocusMember::Lifecycle(lc) => {
                                    walk_block(&lc.body, out)
                                }
                                LocusMember::Mode(md) => {
                                    walk_block(&md.body, out)
                                }
                                LocusMember::Failure(fd) => {
                                    walk_block(&fd.body, out)
                                }
                                _ => {}
                            }
                        }
                    }
                    TopDecl::Module(m) => walk_members(&m.items, out),
                    _ => {}
                }
            }
        }
        for pr in &programs {
            walk_members(&pr.items, &mut send_dispositions);
        }
        send_dispositions.sort_by_key(|(a, b, _)| (*a, *b));
    }
    let disposition_at = |pos: u32| -> PublishDisposition {
        send_dispositions
            .iter()
            .find(|(s, e2, _)| *s <= pos && pos <= *e2)
            .map(|(_, _, d)| *d)
            .unwrap_or(PublishDisposition::Default)
    };

    // publishes: effect sites at SITE grain, + computed-subject holes.
    let mut publishes: BTreeMap<
        (FunctionId, SubjectId, u32),
        (
            Option<TopicId>,
            PayloadContractId,
            Option<KeyDomain>,
            PublishDisposition,
            bool,
            ProvenanceId,
        ),
    > = BTreeMap::new();
    for (k, fs) in &summary.fns {
        if !user_key(k) {
            continue;
        }
        let from = fn_id[&fn_name(k)];
        // EVERY publish effect site consumes one source-order
        // ordinal — known-subject rows and computed-subject holes
        // share the space, so a consumer interleaving them by site
        // sees authored order (review round 2: a computed publish
        // used to leave `site` unchanged, giving the NEXT known
        // publish the same ordinal and reordering the pair).
        let mut site: u32 = 0;
        for s in &fs.effect_sites {
            let authored_site = site;
            if matches!(s.kind, EffectSiteKind::Publish(_)) {
                site += 1;
            }
            match &s.kind {
                EffectSiteKind::Publish(Some(subj)) => {
                    let display = subj.text.clone();
                    let pid = intern_span(&mut records, s.span);
                    // The SYNTACTIC form decides: a string-literal
                    // send is a wire address even when its text
                    // collides with a topic NAME (round 10).
                    let declared_info = if subj.literal {
                        None
                    } else {
                        topic_decl_by_name.get(&display)
                    };
                    let (declared, subject_str, payload, keyed) =
                        match declared_info {
                            Some(t) => {
                                let shape_hash =
                                    topic_payload[&display].clone();
                                (
                                    Some(topic_id[&display]),
                                    wire_of(&display),
                                    payload_id[&shape_hash],
                                    t.decl.keyed_by.as_ref().map(|f| {
                                        KeyDomain::AnyOfType(
                                            key_type_of(
                                                &t.raw_payload_ty(),
                                                &f.name,
                                            ),
                                        )
                                    }),
                                )
                            }
                            None => (
                                None,
                                display.clone(),
                                payload_id[&endpoint_payload
                                    [&display]
                                    .clone()],
                                None,
                            ),
                        };
                    publishes.insert(
                        (from, subject_id[&subject_str], authored_site),
                        (
                            declared,
                            payload,
                            keyed,
                            disposition_at(
                                s.span.start.as_usize() as u32
                            ),
                            s.loop_depth > 0,
                            pid,
                        ),
                    );
                }
                EffectSiteKind::Publish(None) => {
                    let pid = intern_span(&mut records, s.span);
                    holes
                        .entry((
                            EntityRef::Function(from),
                            HoleKind::ComputedSubject,
                            "publish with computed subject"
                                .to_string(),
                        ))
                        .or_insert((
                            hale_model::RelationSet::PUBLISHES,
                            Some(authored_site),
                            pid,
                        ));
                }
                _ => {}
            }
        }
    }

    // subscribes: AST bus blocks (filters + bounds), joined with the
    // graph's per-site spans.
    let mut subscribes: BTreeMap<
        (SubjectId, FunctionId, u32),
        (
            Option<TopicId>,
            PayloadContractId,
            KeyPredicate,
            hale_model::Capacity,
            hale_model::ShedPolicy,
            ProvenanceId,
        ),
    > = BTreeMap::new();
    for l in &ast.loci {
        let locus_raw = l.name.name.clone();
        let mut site: u32 = 0;
        for m in &l.members {
            let LocusMember::Bus(bus) = m else { continue };
            for bm in &bus.members {
                let BusMember::Subscribe {
                    subject,
                    handler,
                    key_filter,
                    bound,
                    span,
                    ty,
                    ..
                } = bm
                else {
                    continue;
                };
                let display = subject.canonical().to_string();
                let handler_full =
                    format!("{}::{}", locus_raw, handler.name);
                let Some(hid) = fn_id.get(&handler_full) else {
                    continue;
                };
                // The BusSubject VARIANT decides declaredness — a
                // literal `subscribe "Orders"` keeps its literal
                // wire address and its own `of type` contract even
                // when the text collides with a topic NAME
                // (round 10; the resolver's distinction must
                // survive extraction).
                let declared_info = match subject {
                    BusSubject::Literal { .. } => None,
                    BusSubject::Topic(_)
                    | BusSubject::QualifiedTopic(_) => {
                        topic_decl_by_name.get(&display)
                    }
                };
                let (declared, subject_str, payload) =
                    match declared_info {
                        Some(_) => (
                            Some(topic_id[&display]),
                            wire_of(&display),
                            payload_id
                                [&topic_payload[&display].clone()],
                        ),
                        None => (
                            None,
                            display.clone(),
                            match ty {
                                Some(t) => {
                                    payload_id[&contract_of_te(t)]
                                }
                                None => payload_id
                                    [&endpoint_payload[&display]],
                            },
                        ),
                    };
                let predicate = match key_filter {
                    None => KeyPredicate::Any,
                    Some(KeyFilter::Replica { .. }) => {
                        KeyPredicate::EqReplica
                    }
                    Some(KeyFilter::Unmatched { .. }) => {
                        KeyPredicate::Fallback
                    }
                    Some(KeyFilter::Specific { expr, .. }) => {
                        match expr {
                            Expr::Literal(Literal::Int(v), _) => {
                                KeyPredicate::EqLiteral(
                                    hale_model::KeyValue::Int(*v),
                                )
                            }
                            Expr::Literal(Literal::String(sv), _) => {
                                KeyPredicate::EqLiteral(
                                    hale_model::KeyValue::Str(
                                        sv.clone(),
                                    ),
                                )
                            }
                            Expr::Literal(Literal::Bool(b), _) => {
                                KeyPredicate::EqLiteral(
                                    hale_model::KeyValue::Bool(*b),
                                )
                            }
                            _ => KeyPredicate::Unknown,
                        }
                    }
                };
                let pid = intern_span(&mut records, *span);
                if matches!(predicate, KeyPredicate::Unknown) {
                    holes
                        .entry((
                            EntityRef::Function(*hid),
                            HoleKind::UnknownKeyDomain,
                            "instantiation-time key filter"
                                .to_string(),
                        ))
                        .or_insert((
                            hale_model::RelationSet::KEY_FILTERS,
                            None,
                            pid,
                        ));
                }
                let (cap, shed) = match bound {
                    None => (
                        hale_model::Capacity::Unbounded,
                        hale_model::ShedPolicy::None,
                    ),
                    Some(b) => (
                        hale_model::Capacity::Bounded(b.cap as u64),
                        match b.policy {
                            AstShedPolicy::DropOld => {
                                hale_model::ShedPolicy::DropOld
                            }
                            AstShedPolicy::DropNew => {
                                hale_model::ShedPolicy::DropNew
                            }
                        },
                    ),
                };
                subscribes.insert(
                    (subject_id[&subject_str], *hid, site),
                    (declared, payload, predicate, cap, shed, pid),
                );
                site += 1;
            }
        }
    }

    // supervision (per-handler).
    // Keyed WITH the authored ordinal: duplicate-signature handlers
    // are check-clean and the legacy artifact serializes each
    // declaration -- a (parent, child, err)-only key silently
    // dropped the earlier one (review round 14).
    let mut sup: BTreeMap<
        (LocusDeclId, SupervisedRef, String, u32),
        (Vec<String>, Option<i64>, ProvenanceId),
    > = BTreeMap::new();
    //
    // F.40 phase 1.4: projected from the handler rows. The child is the
    // row's: a locus the model has a declaration for (a monomorph is
    // its template, the declaration written), else the written name.
    // The authored ordinal stays the handler's position in the bundle
    // walk (the rows come in that order), not the row's per-parent
    // ordinal, so the canonical key keeps its values.
    //
    // The rows are the bundle's, demanded once per snapshot: a child
    // declared in a sibling file is a locus here as it is to lowering.
    //
    // Parent and child join the locus table by declaration identity:
    // the row's parent site, and the site of the declaration its child
    // resolves to (a monomorph's template's). A child declared outside
    // the snapshot's programs (a stdlib locus, which the model has no
    // declaration for) is external, by its written name. A bundle no
    // entry point minted has no sites, and joins by name.
    let mut locus_by_site: BTreeMap<u32, LocusDeclId> = BTreeMap::new();
    for l in &ast.loci {
        if !l.id.is_none() {
            locus_by_site.insert(l.id.0, locus_id[&l.name.name]);
        }
    }
    for (authored, row) in inputs.handlers.rows().iter().enumerate() {
        let parent = match row.parent_id {
            Some(site) => locus_by_site[&site.index],
            None => locus_id[&row.parent],
        };
        let declared = match (&row.child, row.child_decl) {
            (ChildRef::External(_), _) => None,
            (ChildRef::Locus(_), Some(d)) => match d.universe {
                crate::placement::SiteUniverse::User => locus_by_site.get(&d.id.index),
                crate::placement::SiteUniverse::StdlibAnalysis => None,
            },
            (ChildRef::Locus(_), None) if row.id.is_some() => None,
            (ChildRef::Locus(n), None) => {
                locus_id.get(n).or_else(|| locus_id.get(&row.written))
            }
        };
        let child = match declared {
            Some(id) => SupervisedRef::Locus(*id),
            None => SupervisedRef::External(row.written.clone()),
        };
        let ops: Vec<String> = row
            .ops
            .iter()
            .map(|op| crate::handler_routing::op_name(*op).to_string())
            .collect();
        let pid = intern_span(&mut records, row.span);
        sup.insert(
            (parent, child, row.error_type.clone(), authored as u32),
            (ops, row.retry_bound, pid),
        );
    }

    // groups: authored selectors + resolved membership.
    let mut group_members: BTreeSet<(GroupId, EntityRef)> =
        BTreeSet::new();
    let mut group_selectors: Vec<GroupSelector> = Vec::new();
    let mut gm_prov: BTreeMap<(GroupId, EntityRef), ProvenanceId> =
        BTreeMap::new();
    for (gname, g) in &group_rows {
        let gid = group_id[gname];
        for (ordinal, m) in g.members.iter().enumerate() {
            let pid = intern_span(&mut records, m.span);
            if m.glob {
                let alias = m
                    .segments
                    .first()
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                let sid = seed_id[&alias];
                group_selectors.push(GroupSelector {
                    group: gid,
                    ordinal: ordinal as u32,
                    selector: SelectorForm::SeedGlob {
                        seed: sid,
                        display: m.display(),
                    },
                    provenance: pid,
                });
                // Enumerate: the alias's loci and free fns.
                if let Some(members) = vmodel.seeds.get(&alias) {
                    for mangled in members {
                        let display = mangled.clone();
                        if let Some(id) =
                            locus_id.get(&display)
                        {
                            group_members.insert((
                                gid,
                                EntityRef::LocusDecl(*id),
                            ));
                            gm_prov
                                .entry((
                                    gid,
                                    EntityRef::LocusDecl(*id),
                                ))
                                .or_insert(pid);
                        }
                        if let Some(id) = fn_id.get(&display) {
                            group_members.insert((
                                gid,
                                EntityRef::Function(*id),
                            ));
                            gm_prov
                                .entry((
                                    gid,
                                    EntityRef::Function(*id),
                                ))
                                .or_insert(pid);
                        }
                    }
                }
            } else {
                // Lookups are by RAW name (qualified members were
                // collapsed to mangled segments by the import
                // pass); the selector row's display re-spells them.
                let display = m.display();
                // Both-join rule: a name shared by a locus and a
                // fn contributes both members; the selector row
                // references the locus when both exist.
                let mut named_ref: Option<EntityRef> = None;
                if let Some(id) = locus_id.get(&display) {
                    let r = EntityRef::LocusDecl(*id);
                    group_members.insert((gid, r));
                    gm_prov.entry((gid, r)).or_insert(pid);
                    named_ref.get_or_insert(r);
                }
                if let Some(id) = fn_id.get(&display) {
                    let r = EntityRef::Function(*id);
                    group_members.insert((gid, r));
                    gm_prov.entry((gid, r)).or_insert(pid);
                    named_ref.get_or_insert(r);
                }
                if let Some(r) = named_ref {
                    group_selectors.push(GroupSelector {
                        group: gid,
                        ordinal: ordinal as u32,
                        selector: SelectorForm::Named {
                            member: r,
                            // Author spelling: qualified members have
                            // been collapsed to mangled single
                            // segments by the import pass, so the
                            // raw display() may read __lib_… — the
                            // STORED spelling runs through the
                            // demangle map exactly as the artifact's
                            // does (lookups above stayed raw).
                            display: name(&m.display()),
                        },
                        provenance: pid,
                    });
                }
            }
        }
    }
    group_selectors.sort_by_key(|s| (s.group, s.ordinal));

    // declared_in: alias-seed membership over the full universe.
    let mut declared_in: BTreeSet<(EntityRef, SeedId)> = BTreeSet::new();
    let mut di_prov: BTreeMap<(EntityRef, SeedId), ProvenanceId> =
        BTreeMap::new();
    for (alias, members) in &vmodel.seeds {
        let sid = seed_id[alias];
        for mangled in members {
            if let Some(r) = entity_of(mangled) {
                declared_in.insert((r, sid));
                let pid = intern_synth(
                    &mut records,
                    &format!("seed `{}` import table", alias),
                );
                di_prov.entry((r, sid)).or_insert(pid);
            }
        }
    }

    // ---- assemble, in canonical order ----
    let mut e = Entities::default();
    // Effect classes (GH #476 Change 4): the USER class vocabulary,
    // with declaration status and NORMALIZED atomic composition —
    // the table's declared set and composition, made model rows so a
    // ClaimIr consumer can tell a declared class from an interned typo
    // and expand `effect io = {…}` without the AST.
    {
        let mut rows: Vec<hale_model::EffectClassDecl> = Vec::new();
        for (i, n) in effect_classes.names().iter().enumerate() {
            let definition = if effect_classes.is_composed(i as u16) {
                // A cycle resolves to no effect and must stay
                // distinguishable from an atomic class (review round
                // 16; the checker rejects it at the declaration).
                let (atoms, cyclic) = effect_classes.atoms(i as u16);
                if cyclic {
                    hale_model::EffectClassDefinition::InvalidCycle
                } else {
                    hale_model::EffectClassDefinition::Composed {
                        atoms: atoms.into_iter().collect(),
                    }
                }
            } else {
                hale_model::EffectClassDefinition::Atomic
            };
            let pid = intern_synth(
                &mut records,
                "effect class declaration",
            );
            rows.push(hale_model::EffectClassDecl {
                name: n.clone(),
                declaration_index: i as u32,
                declared: effect_classes.declared().contains(&(i as u16)),
                definition,
                provenance: pid,
            });
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        e.effect_classes = rows;
    }
    // Round 10: a locus is UNANALYZABLE only when it carries
    // executable members the engines never walked, and the flag is
    // recomputable at admission from the member coverage. Every
    // member body is walked (a module-nested locus's included since
    // the behavior summary collects module-nested bodies, E3a part
    // C; failure handlers do not count), so every locus is
    // analyzable.
    for (n, row) in &locus_rows {
        let pid = intern_span(&mut records, row.span);
        e.loci.push(LocusDecl {
            name: n.clone(),
            display: name(n),
            sealed: row.sealed,
            analyzable: true,
            sync_form: row.sync_form,
            params: row
                .params
                .iter()
                .map(|(pn, ty)| hale_model::LocusParam {
                    name: pn.clone(),
                    type_name: ty.clone(),
                    decl: locus_id.get(ty).copied(),
                })
                .collect(),
            provenance: pid,
        });
    }
    // Effects per fn (the derived classes the artifact exports), read
    // from the effect rows, and the DIRECT sets the reachability
    // judgment's `effects(C)` destination test reads (GH #476 Change
    // 5a) — each fn's `direct` column of the same rows, over the
    // stdlib-merged summary the rows' walk read.
    let mut derived_effects: BTreeMap<String, Vec<String>> =
        BTreeMap::new();
    let mut direct_effects: BTreeMap<String, Vec<String>> =
        BTreeMap::new();
    let mut effect_lower_bounds: BTreeMap<String, Vec<String>> =
        BTreeMap::new();
    let mut effects_unknown: BTreeSet<String> = BTreeSet::new();
    for k in merged.fns.keys() {
        if !user_key(k) {
            continue;
        }
        let row = &inputs.effects.rows[k];
        let classes =
            crate::frontier::render_effects_named(row.effects, effect_names);
        if !classes.is_empty() {
            derived_effects.insert(fn_name(k), classes);
        }
        // …and the LOWER BOUND, kept apart from the rendering.
        // `UNCLASSIFIED` is saturation, not a bit, so the known
        // classes cannot be masked back out of `row.effects` — the
        // row's walk flags an unnameable edge instead of swallowing
        // the set (GH #476 Change 5f review).
        let (known, unknown) = (row.known, row.unknown);
        let known_classes =
            crate::frontier::render_effects_named(known, effect_names);
        if !known_classes.is_empty() {
            effect_lower_bounds.insert(fn_name(k), known_classes);
        }
        if unknown {
            effects_unknown.insert(fn_name(k));
        }
    }
    let authored_user_class: BTreeSet<String> =
        crate::effects::fns_carrying_a_user_class(&programs)
            .into_iter()
            .map(|k| fn_name(&k))
            .collect();
    let mut attribution: BTreeMap<String, Vec<String>> =
        BTreeMap::new();
    let mut opaque_calls: BTreeSet<String> = BTreeSet::new();
    for (k, fs) in &merged.fns {
        if !user_key(k) || !vmodel.is_bundle_fn(k) {
            continue;
        }
        let mut classes: Vec<String> = Vec::new();
        for name in [
            "syscall",
            "block",
            "publish",
            "time",
            "entropy",
            "env",
            "alloc",
            "secret_use",
        ] {
            let mask = crate::claims::attributed_mask(name)
                .expect("builtin mask");
            if crate::claims::performs_directly_for(
                &merged, &vmodel, &ffi, k, fs, mask,
            ) {
                classes.push(name.to_string());
            }
        }
        classes.sort();
        if !classes.is_empty() {
            attribution.insert(fn_name(k), classes);
        }
        if crate::claims::has_opaque_unresolved(fs) {
            opaque_calls.insert(fn_name(k));
        }
    }
    for k in summary.fns.keys() {
        if !user_key(k) {
            continue;
        }
        let d = inputs.effects.direct(k);
        if !d.is_unclassified() && d != crate::stdlib_surface::EffectSet::PURE {
            let mut classes = crate::frontier::render_effects_named(
                d,
                &effect_names,
            );
            classes.sort();
            classes.dedup();
            direct_effects.insert(fn_name(k), classes);
        }
    }
    for &(fid, n, info) in &fn_universe {
        debug_assert_eq!(e.functions.len(), fid.index());
        let pid = match info.span {
            Some(sp) => intern_span(&mut records, sp),
            None => intern_synth(&mut records, "fn (summary key)"),
        };
        e.functions.push(Function {
            name: n.clone(),
            display: info.display.clone(),
            kind: info.kind,
            effects: derived_effects.get(n).cloned().unwrap_or_default(),
            effect_lower_bound: effect_lower_bounds
                .get(n)
                .cloned()
                .unwrap_or_default(),
            effects_unknown: effects_unknown.contains(n),
            direct_effects: direct_effects
                .get(n)
                .cloned()
                .unwrap_or_default(),
            attribution: attribution
                .get(n)
                .cloned()
                .unwrap_or_default(),
            opaque_call: opaque_calls.contains(n),
            carries_user_class: authored_user_class.contains(n),
            analyzed: !info.unanalyzed,
            summarized: summarized_names.contains(n),
            owner: info
                .locus
                .as_ref()
                .and_then(|ld| locus_id.get(ld).copied()),
            provenance: pid,
        });
    }
    for s in &subject_set {
        let pid = intern_synth(&mut records, "wire subject");
        e.subjects.push(Subject {
            pattern: s.clone(),
            exact: !s.contains('*'),
            provenance: pid,
        });
    }
    for ((shape, hash, opaque), ()) in &payload_rows {
        let pid = intern_synth(&mut records, "payload contract");
        e.payloads.push(PayloadContract {
            shape: shape.clone(),
            opaque: *opaque,
            hash: *hash,
            provenance: pid,
        });
    }
    for tname in &topic_names {
        let info = &topic_decl_by_name[tname];
        let t = info.decl;
        let pid = intern_span(&mut records, t.name.span);
        e.topics.push(Topic {
            name: tname.clone(),
            display: name(tname),
            subject: subject_id[&info.wire],
            payload: payload_id[&topic_payload[tname].clone()],
            key: t.keyed_by.as_ref().map(|f| TopicKey {
                field: f.name.clone(),
                on_unmatched: match t.on_unmatched {
                    None | Some(UnmatchedPolicy::Swallow) => {
                        KeyOnUnmatched::Swallow
                    }
                    Some(UnmatchedPolicy::Fail) => KeyOnUnmatched::Fail,
                    Some(UnmatchedPolicy::Fallback) => {
                        KeyOnUnmatched::Fallback
                    }
                },
            }),
            bound: t.bounded.map(|(n, _)| TopicBound {
                capacity: n.max(1) as u64,
                on_full: TopicOnFull::Fail,
            }),
            provenance: pid,
        });
    }
    for p in &phase_names {
        let pid = intern_synth(&mut records, "phase");
        e.phases.push(Phase {
            name: p.clone(),
            provenance: pid,
        });
    }
    for s in &seed_names {
        let pid =
            intern_synth(&mut records, &format!("seed `{}`", s));
        e.seeds.push(Seed {
            name: s.clone(),
            provenance: pid,
        });
    }
    for (n, g) in &group_rows {
        let pid = intern_span(&mut records, g.name.span);
        e.groups.push(Group {
            name: n.clone(),
            display: name(n),
            may_be_empty: g.may_be_empty,
            provenance: pid,
        });
    }
    for (n, sp) in &type_rows {
        let pid = intern_span(&mut records, *sp);
        e.types.push(MTypeDecl {
            name: n.clone(),
            display: name(n),
            provenance: pid,
        });
    }
    for (n, sp) in &iface_rows {
        let pid = intern_span(&mut records, *sp);
        e.interfaces.push(InterfaceDecl {
            name: n.clone(),
            display: name(n),
            provenance: pid,
        });
    }
    for ((n, kind), sp) in &other_rows {
        let pid = intern_span(&mut records, *sp);
        e.declarations.push(Declaration {
            kind: *kind,
            name: n.clone(),
            display: name(n),
            provenance: pid,
        });
    }

    let mut r = Relations::default();
    // member_of + phase_of from the fn rows.
    for &(fid, n, info) in &fn_universe {
        if let Some(ld) = &info.locus {
            if let Some(lid) = locus_id.get(ld) {
                let pid =
                    intern_synth(&mut records, "locus membership");
                r.member_of.push(MemberOf {
                    function: fid,
                    locus: *lid,
                    provenance: pid,
                });
            }
        }
        if let Some((phase, _)) = phase_of_pairs.get(n) {
            let pid = intern_synth(&mut records, "phase relation");
            r.phase_of.push(PhaseOf {
                function: fid,
                phase: phase_id[phase],
                provenance: pid,
            });
        }
    }
    for ((from, to, dispatch, site), (in_loop, unbounded, pid)) in &calls
    {
        r.calls.push(Call {
            from: *from,
            to: *to,
            dispatch: dispatch.clone(),
            site: *site,
            in_loop: *in_loop,
            unbounded: *unbounded,
            provenance: *pid,
        });
    }
    for ((from, site), (iface, method, pid)) in &dead {
        r.dead_interface_calls.push(DeadInterfaceCall {
            from: *from,
            site: *site,
            interface: iface.clone(),
            method: method.clone(),
            provenance: *pid,
        });
    }
    for (
        (f, s, site),
        (declared, payload, keyed, dispo, in_loop, pid),
    ) in &publishes
    {
        r.publishes.push(Publish {
            function: *f,
            subject: *s,
            declared_topic: *declared,
            payload: *payload,
            site: *site,
            in_loop: *in_loop,
            key_domain: keyed.clone(),
            disposition: *dispo,
            provenance: *pid,
        });
    }
    for ((s, h, site), (declared, payload, pred, cap, shed, pid)) in
        &subscribes
    {
        r.subscribes.push(Subscribe {
            subject: *s,
            declared_topic: *declared,
            payload: *payload,
            handler: *h,
            site: *site,
            key_predicate: pred.clone(),
            capacity: *cap,
            shed: *shed,
            provenance: *pid,
        });
    }
    for ((parent, child, err, authored), (ops, retry, pid)) in &sup {
        r.supervises.push(Supervises {
            parent: *parent,
            child: child.clone(),
            error_type: err.clone(),
            policy: SupervisionPolicy {
                ops: ops.clone(),
                retry_bound: *retry,
            },
            authored_ordinal: *authored,
            provenance: *pid,
        });
    }
    for (gid, member) in &group_members {
        r.group_members.push(GroupMember {
            group: *gid,
            member: *member,
            provenance: gm_prov[&(*gid, *member)],
        });
    }
    r.group_selectors = group_selectors;
    for (entity, sid) in &declared_in {
        r.declared_in.push(DeclaredIn {
            entity: *entity,
            seed: *sid,
            provenance: di_prov[&(*entity, *sid)],
        });
    }

    // labels: declared effect carriers. Entities in canonical
    // order, but WITHIN one entity the class order is semantic —
    // `render_effects_named` order (fixed built-ins, then user
    // classes in declaration order), which the artifact hashes.
    // Flattening through a sorted set here once lexicalized
    // `["zebra","alpha"]` into `["alpha","zebra"]` and silently
    // changed the projected identity (review round 11).
    let mut labels: Vec<LabelRow> = Vec::new();
    {
        let mut per_fn: BTreeMap<hale_model::FunctionId, Vec<String>> =
            BTreeMap::new();
        for (k, set) in &summary.carries {
            if !user_key(k) {
                continue;
            }
            let classes = crate::frontier::render_effects_named(
                *set,
                &effect_names,
            );
            if !classes.is_empty() {
                per_fn.insert(fn_id[&fn_name(k)], classes);
            }
        }
        for (f, classes) in per_fn {
            for label in classes {
                let pid = intern_synth(
                    &mut records,
                    "declared effect carrier",
                );
                labels.push(LabelRow {
                    at: EntityRef::Function(f),
                    label,
                    provenance: pid,
                });
            }
        }
    }

    // Unanalyzed bodies (on_failure): declared
    // executable entities whose calls/publishes/effects the summary
    // never walked — typed holes keep the capabilities honest.
    for &(fid, _, info) in &fn_universe {
        if info.unanalyzed {
            let pid = match info.span {
                Some(sp) => intern_span(&mut records, sp),
                None => intern_synth(&mut records, "unanalyzed body"),
            };
            holes
                .entry((
                    EntityRef::Function(fid),
                    HoleKind::UnanalyzedBody,
                    "body not walked by the behavior analysis"
                        .to_string(),
                ))
                .or_insert((
                    hale_model::RelationSet::CALLS
                        .union(hale_model::RelationSet::PUBLISHES)
                        .union(hale_model::RelationSet::EFFECTS),
                    None,
                    pid,
                ));
        }
    }

    // Declared publisher ends: `bus { publish T; }` — the endpoint
    // grain, independent of whether any send exists.
    {
        // Round 13: the canonical identity INCLUDES the typed
        // declaredness — `publish "wire.orders" of type Msg;` and
        // `publish Orders;` resolve to one SubjectId but are
        // distinct semantic facts (BusSubject::canonical and the
        // endpoint judgment both branch on declared_topic), so
        // both survive regardless of declaration order.
        let mut ends: BTreeMap<
            (
                hale_model::LocusDeclId,
                SubjectId,
                Option<TopicId>,
            ),
            (PayloadContractId, ProvenanceId),
        > = BTreeMap::new();
        for l in &ast.loci {
            let Some(lid) = locus_id.get(&l.name.name) else {
                continue;
            };
            for m in &l.members {
                let LocusMember::Bus(bus) = m else { continue };
                for bm in &bus.members {
                    let BusMember::Publish { subject, ty, span, .. } =
                        bm
                    else {
                        continue;
                    };
                    let raw = subject.canonical().to_string();
                    // Variant-decided, like every endpoint: a
                    // literal `publish "addr" of type T` declares a
                    // publisher end on a WIRE ADDRESS with its own
                    // contract, never on a name-colliding topic
                    // (round 10).
                    let declared_info = match subject {
                        BusSubject::Literal { .. } => None,
                        BusSubject::Topic(_)
                        | BusSubject::QualifiedTopic(_) => {
                            topic_decl_by_name.get(&raw)
                        }
                    };
                    let (declared, subj_str, payload) =
                        match declared_info {
                            Some(info) => (
                                Some(topic_id[&raw]),
                                info.wire.clone(),
                                payload_id
                                    [&topic_payload[&raw].clone()],
                            ),
                            None => {
                                let key = ty
                                    .as_ref()
                                    .map(&contract_of_te)
                                    .or_else(|| {
                                        endpoint_payload
                                            .get(&raw)
                                            .cloned()
                                    })
                                    .unwrap_or_else(|| {
                                        unresolved_payload.clone()
                                    });
                                (None, raw.clone(), payload_id[&key])
                            }
                        };
                    let pid = intern_span(&mut records, *span);
                    ends.entry((
                        *lid,
                        subject_id[&subj_str],
                        declared,
                    ))
                    .or_insert((payload, pid));
                }
            }
        }
        for ((lid, sid, declared), (payload, pid)) in ends {
            r.declares_publish.push(
                hale_model::DeclaresPublish {
                    locus: lid,
                    subject: sid,
                    declared_topic: declared,
                    payload,
                    provenance: pid,
                },
            );
        }
    }

    // GH #476 Change 8: the BusGraph's per-subject dispatch gates,
    // bridged verbatim — DispatchPlan::derive combines them with
    // the arrangement into the typed lowering plan.
    //
    // Keyed at WIRE grain, not at `BusSubject::canonical()` grain.
    // The graph this builder runs over sees the AUTHORED program, so
    // a topic-addressed site keys by the topic's declaration name
    // (`Evt`); the resolved program desugars topics to their wire
    // subject before building lowering's graph, so the very same
    // dispatch keys by `evt` there — and the wire string is the identity the runtime, the
    // artifact (Change 7's route grain), and the static bucket all
    // use. Mapping here is what makes the two plans comparable at
    // all.
    //
    // A program that addresses one topic BOTH ways (`Evt <- x` in
    // one locus, `"evt" <- x` in another) has two authored subjects
    // collapsing onto one wire subject — codegen computes ONE
    // eligibility over the union of those sites, so the merge is
    // conjunctive (a subject is static/direct here only if every
    // authored view of it was) with the site sets unioned. That is
    // the conservative side: the plan can under-promote relative to
    // codegen, never over-promote.
    //
    // Known wrong (the F.40 phase 1.5 shadow's one divergence): a
    // LITERAL subject spelled like a topic's name (`"Evt" <- x` beside
    // `topic Evt { subject: "evt"; }`) shares the topic's key in the
    // authored graph, so it is merged onto the topic's wire here. The
    // model's plan then has no row for the literal subject and the
    // topic's row carries the literal's subscribers; lowering's graph,
    // over wire literals, keeps the two apart.
    //
    // The wire is the scope's topic row's (F.40 phase 2.3): the one
    // table the checker reads a subject through.
    let mut gate_by_wire: BTreeMap<String, hale_model::DispatchGate> =
        BTreeMap::new();
    for (subject, info) in &graph.subjects {
        let wire = inputs
            .top
            .topics
            .named(subject)
            .map(|row| row.wire.clone())
            .unwrap_or_else(|| subject.clone());
        let publisher_loci: Vec<String> =
            info.publishers.iter().map(|p| p.locus.clone()).collect();
        let subscribers: Vec<(String, String)> = info
            .subscribers
            .iter()
            .map(|s2| (s2.locus.clone(), s2.handler.clone()))
            .collect();
        let reason =
            info.ineligible_reason.as_ref().map(|r| r.tag().to_string());
        match gate_by_wire.get_mut(&wire) {
            Some(g) => {
                g.static_eligible &= info.eligible;
                g.direct_eligible &= info.direct_call_eligible;
                g.payload_flat &= info.payload_flat;
                if g.ineligible_reason.is_none() {
                    g.ineligible_reason = reason;
                }
                g.publisher_loci.extend(publisher_loci);
                g.subscribers.extend(subscribers);
            }
            None => {
                gate_by_wire.insert(
                    wire.clone(),
                    hale_model::DispatchGate {
                        subject: wire,
                        static_eligible: info.eligible,
                        direct_eligible: info.direct_call_eligible,
                        payload_flat: info.payload_flat,
                        ineligible_reason: reason,
                        publisher_loci,
                        subscribers,
                    },
                );
            }
        }
    }
    let dispatch_gates: Vec<hale_model::DispatchGate> = gate_by_wire
        .into_values()
        .map(|mut g| {
            g.publisher_loci.sort();
            g.publisher_loci.dedup();
            g.subscribers.sort();
            g.subscribers.dedup();
            g
        })
        .collect();

    // The Change-3 bridge: the legacy artifact's fn sort, recorded
    // so TopologyShapeV1 projects from the model alone.
    // GH #476 Change 5a: the stdlib-absorption sidecar — what the
    // evaluator's merged-summary walk sees inside stdlib bodies
    // reachable from each user call site. Site ordinals replicate
    // the call-loop's allocation (dispatch_group shared), so the
    // judgment can interleave absorbed edges at the evaluator's
    // position. Per-entry BFS over the merged summary; holes and
    // re-emergences recorded in walk order.
    // Locus -> its declared WILDCARD publish patterns. A computed
    // publish is admitted only under such a declaration and enforced
    // against it at the publish site, so these bound which subjects
    // an unresolved publish inside the locus can address. Without the
    // bound, one stdlib I/O call (std::io::tcp logs to a
    // runtime-chosen subject) withdrew publish-completeness from
    // every topic in the program.
    // Spans the STDLIB's loci as well as the user's: the absorption
    // walk names its nodes with the stdlib's own (already mangled)
    // locus names — `__StdIoTcpStream` — and those declarations are
    // exactly the ones that bound the holes this is here to scope.
    // `ast.loci` covers only the user seeds, so a map built from it
    // alone comes back empty and every hole stays unconstrained.
    let wildcard_publish_patterns: BTreeMap<String, Vec<String>> = {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut take = |l: &hale_syntax::ast::LocusDecl| {
            for mem in &l.members {
                // `continue`, not `return`: this is a closure, and a
                // locus almost always declares `params` before its
                // `bus` block — returning on the first non-bus member
                // skipped every declaration.
                let LocusMember::Bus(bus) = mem else { continue };
                for bm in &bus.members {
                    let BusMember::Publish { subject, .. } = bm
                    else {
                        continue;
                    };
                    let raw = subject.canonical().to_string();
                    if raw.contains("**") {
                        m.entry(l.name.name.clone())
                            .or_default()
                            .push(raw);
                    }
                }
            }
        };
        for l in &ast.loci {
            take(l);
        }
        if let Some(sp) = crate::stdlib_bodies::program() {
            fn walk(
                items: &[TopDecl],
                f: &mut impl FnMut(&hale_syntax::ast::LocusDecl),
            ) {
                for item in items {
                    match item {
                        TopDecl::Locus(l) => f(l),
                        TopDecl::Module(md) => walk(&md.items, f),
                        _ => {}
                    }
                }
            }
            walk(&sp.items, &mut take);
        }
        m
    };
    let mut stdlib_absorption: Vec<hale_model::StdlibAbsorption> =
        Vec::new();
    // MERGED summary: the evaluator's Cx.summary — stdlib conformer
    // alternatives and resolved stdlib methods only exist there.
    for (k, fs) in &merged.fns {
        if !user_key(k) {
            continue;
        }
        let from = fn_id[&fn_name(k)];
        let mut next_ordinal: u32 = 0;
        let mut group_site: BTreeMap<u32, u32> = BTreeMap::new();
        let mut site_of = |group: Option<u32>| -> u32 {
            match group {
                Some(g) => *group_site.entry(g).or_insert_with(|| {
                    let o = next_ordinal;
                    next_ordinal += 1;
                    o
                }),
                None => {
                    let o = next_ordinal;
                    next_ordinal += 1;
                    o
                }
            }
        };
        // A function-value dispatch numbers its sites as the program's
        // own rows hold it (`AllocSummary::own_rows`): an alternative
        // only the analysis copy supplies is not one of them, and a
        // dispatch with none of the program's own is its one indirect
        // call as written.
        let kept = merged.value_groups_kept(fs);
        let mut restored: BTreeSet<u32> = BTreeSet::new();
        for edge in &fs.calls {
            if let (Some(_), Some(g)) = (&edge.via_value, edge.dispatch_group) {
                if !kept.contains(&g) {
                    if restored.insert(g) {
                        site_of(None);
                    }
                    continue;
                }
                if merged.copy_alternative(edge) {
                    continue;
                }
            }
            let site = site_of(model_group(edge));
            let Callee::Resolved(next) = &edge.callee else {
                continue;
            };
            if user_key(next) {
                continue;
            }
            // Entry into a stdlib body (direct call, or a stdlib
            // conformer alternative of a dispatch): capture the
            // interior GRAPH the evaluator would traverse — vertices
            // in discovery order, each body's events in body order —
            // so the judgment replays BFS layering (and therefore
            // hole-vs-hit timing) exactly.
            // The interface is rendered as the interface rows of the same
            // dispatch render it (`calls.dispatch_site` holds them to
            // one identity): a program's OWN interface keeps its
            // declared spelling; a stdlib interface demangles to its
            // public path. The stdlib's own seeds are harvested into
            // the corpus, and there `__StdHttpHandler` is the program's
            // declaration, not a mangled stdlib name (GH #730, first
            // half, made those seeds check clean).
            let entry_dispatch =
                edge.via_interface.as_ref().map(|i| {
                    let shown = if ast.interfaces.iter().any(|(n, _)| *n == i.as_str()) {
                        i.clone()
                    } else {
                        crate::stdlib_bodies::demangle_with(i, &rename_table)
                    };
                    (shown, next.fn_name.clone())
                });
            let entry_in_loop = edge.loop_depth > 0;
            let entry_group = model_group(edge);
            let entry_provenance =
                intern_span(&mut records, edge.span);
            let disp = |kk: &FnKey| -> String {
                crate::stdlib_bodies::demangle_with(&kk.display(), &rename_table)
            };
            let mut nodes: Vec<hale_model::AbsorbedNode> = Vec::new();
            let mut index: BTreeMap<FnKey, u32> = BTreeMap::new();
            let mut order: Vec<FnKey> = Vec::new();
            index.insert(next.clone(), 0);
            order.push(next.clone());
            let mut cursor = 0usize;
            let mut truncated = false;
            while cursor < order.len() {
                if order.len() as u32 > crate::callgraph::MAX_STEPS {
                    // Explicit residue: a judgment must not treat a
                    // truncated interior as fully explored.
                    truncated = true;
                    break;
                }
                let n = order[cursor].clone();
                cursor += 1;
                let mut events: Vec<hale_model::AbsorbedEvent> =
                    Vec::new();
                let node_direct = {
                    let d = inputs.effects.direct(&n);
                    if d.is_unclassified() {
                        Vec::new()
                    } else {
                        let mut c =
                            crate::frontier::render_effects_named(
                                d,
                                &effect_names,
                            );
                        c.sort();
                        c.dedup();
                        c
                    }
                };
                let carries = merged
                    .carries
                    .get(&n)
                    .map(|set| {
                        let mut c =
                            crate::frontier::render_effects_named(
                                *set,
                                &effect_names,
                            );
                        c.sort();
                        c.dedup();
                        c
                    })
                    .unwrap_or_default();
                if let Some(nfs) = merged.fns.get(&n) {
                    for e2 in &nfs.calls {
                        match &e2.callee {
                            Callee::Resolved(nn) => {
                                let dsp = e2
                                    .via_interface
                                    .as_ref()
                                    .map(|i| {
                                        (
                                            crate::stdlib_bodies::demangle_with(i, &rename_table),
                                            nn.fn_name.clone(),
                                        )
                                    });
                                let target = if user_key(nn) {
                                    hale_model::AbsorbedTarget::User(
                                        fn_id[&fn_name(nn)],
                                    )
                                } else {
                                    let idx = *index
                                        .entry(nn.clone())
                                        .or_insert_with(|| {
                                            order.push(nn.clone());
                                            (order.len() - 1) as u32
                                        });
                                    hale_model::AbsorbedTarget::Interior(idx)
                                };
                                events.push(
                                    hale_model::AbsorbedEvent::Call {
                                        target,
                                        dispatch: dsp,
                                        in_loop: e2.loop_depth > 0,
                                        group: model_group(e2),
                                    },
                                );
                            }
                            Callee::Unresolved(un) => {
                                if e2.indirect
                                    || nfs
                                        .fn_params
                                        .iter()
                                        .any(|pp| pp == un)
                                {
                                    events.push(hale_model::AbsorbedEvent::CallHole(
                                        hale_model::AbsorbedHoleKind::IndirectCall,
                                    ));
                                } else if e2.opaque_method_call() {
                                    events.push(hale_model::AbsorbedEvent::CallHole(
                                        hale_model::AbsorbedHoleKind::OpaqueCall {
                                            callee: un.clone(),
                                        },
                                    ));
                                }
                            }
                        }
                    }
                    for site2 in &nfs.effect_sites {
                        match &site2.kind {
                            alloc_summary::EffectSiteKind::Publish(
                                None,
                            ) => {
                                // Bounded by the declaring locus's
                                // wildcard patterns: the language
                                // requires one for a computed
                                // subject and the publish site
                                // enforces it, so a hole under
                                // `io.tcp.**` cannot explain a
                                // publisher of an application topic.
                                // No locus (a free fn) or no
                                // declaration recovered ⇒ empty ⇒
                                // unconstrained, as before.
                                let patterns = n
                                    .locus
                                    .as_ref()
                                    .and_then(|l| {
                                        wildcard_publish_patterns
                                            .get(l)
                                    })
                                    .cloned()
                                    .unwrap_or_default();
                                events.push(
                                    hale_model::AbsorbedEvent::PublishHole {
                                        patterns,
                                    },
                                );
                            }
                            alloc_summary::EffectSiteKind::Publish(
                                Some(subj),
                            ) => {
                                // The SYNTACTIC form decides, same
                                // rule as user publish rows: a
                                // string literal is a wire address
                                // even when its text collides with
                                // a topic name (round 6).
                                let declared_topic = if subj.literal
                                {
                                    None
                                } else {
                                    topic_id
                                        .get(&subj.text)
                                        .copied()
                                };
                                events.push(
                                    hale_model::AbsorbedEvent::Publish {
                                        subject: subj.text.clone(),
                                        declared_topic,
                                        in_loop: site2.loop_depth > 0,
                                    },
                                );
                            }
                            _ => {}
                        }
                    }
                }
                nodes.push(hale_model::AbsorbedNode {
                    display: disp(&n),
                    direct_effects: node_direct,
                    carries,
                    events,
                });
            }
            if truncated {
                // Truncation lands at the actual UNEXPANDED
                // frontier (review round 3): every discovered-but-
                // unwalked node materializes with a lone Truncated
                // event, so (a) interior targets referencing it stay
                // in range, (b) the known prefix keeps its complete
                // event lists, and (c) a walk surfaces saturation
                // exactly where knowledge ends — not on entry.
                for k in order.iter().skip(nodes.len()) {
                    nodes.push(hale_model::AbsorbedNode {
                        display: disp(k),
                        carries: Vec::new(),
                        direct_effects: Vec::new(),
                        events: vec![
                            hale_model::AbsorbedEvent::Truncated,
                        ],
                    });
                }
            }
            // Keep every entry that can matter to ANY judgment:
            // events (walks), carriers (counts), or direct effects
            // (effect sinks) — an effect-bearing leaf with no
            // outgoing events is still a countable destination.
            // Keep every entry that can matter: events (walks) or
            // direct effects (an effect-bearing leaf with no
            // outgoing events is still a destination).
            if nodes.iter().any(|nd| {
                !nd.events.is_empty()
                    || !nd.direct_effects.is_empty()
            }) {
                stdlib_absorption.push(
                    hale_model::StdlibAbsorption {
                        from,
                        site,
                        entry_dispatch,
                        entry_in_loop,
                        entry_group,
                        entry_provenance,
                        nodes,
                    },
                );
            }
        }
    }
    stdlib_absorption.sort_by_key(|a| (a.from, a.site));

    // holes, canonically ordered.
    // ==== GH #476 Change 8: the ARRANGEMENT — instances,
    // ownership, placement, thread domains, bindings. These are
    // the tables Change 2 deliberately left empty ("no current
    // fragment exports them"); their consumers land here
    // (#464's DispatchPlan, iris instance IDs). None of this
    // participates in the artifact's model half, so shape
    // identity is untouched.
    {
        use hale_model::{
            Binding, BindingId, BindingRole, LocusInstance,
            LocusInstanceId, Owns, PlacedIn, Realizes,
            ThreadDomain, ThreadDomainId, TopicBinding,
            TransportKind,
        };
        use hale_syntax::ast::{
            LocusMember, TopDecl,
        };
        // Locus decls by RAW name, with their members, across the
        // whole bundle (modules included — a module locus can be
        // arranged like any other).
        let mut decls_by_name: BTreeMap<
            &str,
            &hale_syntax::ast::LocusDecl,
        > = BTreeMap::new();
        fn walk_loci<'a>(
            items: &'a [TopDecl],
            out: &mut BTreeMap<
                &'a str,
                &'a hale_syntax::ast::LocusDecl,
            >,
        ) {
            for item in items {
                match item {
                    TopDecl::Locus(l) => {
                        out.entry(l.name.name.as_str()).or_insert(l);
                    }
                    TopDecl::Module(m) => {
                        walk_loci(&m.items, out)
                    }
                    _ => {}
                }
            }
        }
        for pr in &programs {
            walk_loci(&pr.items, &mut decls_by_name);
        }
        // The arrangement is the placement table's rows, projected
        // (F.40 phase 3, P1; `notes/f40-placement-correspondence.md`
        // § 2.4): the instances of the root lowering deploys (one
        // template per literal of it, or the entry's implicit
        // construction of a root no literal builds), each where it
        // runs. A held instance is arranged under its holder, as the
        // source's actual rows the table projects there; the source
        // template's own rows (`PlacementTable::handed_off`) answer
        // where it was built, and are not arranged.
        //
        // A path is the fields from the root, the replica index after
        // the replicated field (`App.f[2].k`). It has no construction
        // component, so one path stands for the rows of every template
        // and alternative that reach it: it is arranged when they agree
        // on what they realize and where they run, and otherwise it is
        // left out with everything under it, each declaration realized
        // there a hole (contract 3).
        //
        // The projection is user-only (U-4): a row realizing a stdlib
        // declaration (a field typed `std::io::tcp::Listener`, a stdlib
        // locus nested under a user one) is left out with its subtree,
        // since its declaration is no entity of this model and adding
        // one would be shape (contract 1). The coverage is partial by
        // design: the table, not the arrangement, answers every
        // placement question. An adapter of the root's `bindings { }`
        // is no instance of the arrangement, and a literal `fn main`
        // builds besides the root stays outside it, a birth the holes
        // below account for.
        use crate::placement::{DomainKind, InstanceKey, InstanceRow, Origin, SiteRef};
        struct Arranged {
            path: String,
            decl: LocusDeclId,
            replica: Option<u32>,
            domain: String,
            parent: Option<String>,
            span: hale_syntax::Span,
        }
        let table = inputs.placement;
        // The user's locus declarations by their minted site: what a
        // row's `realizes` names. A stdlib site is never here.
        let mut decl_at: BTreeMap<SiteRef, &hale_syntax::ast::LocusDecl> = BTreeMap::new();
        for pr in &programs {
            for item in hale_syntax::ast::flat_decls(&pr.items) {
                if let TopDecl::Locus(l) = item {
                    if let Some(id) = bundle.snapshot.site_id(l.id) {
                        decl_at.insert(SiteRef::user(id), l);
                    }
                }
            }
        }
        let root_decl = table.root.as_ref().and_then(|r| decl_at.get(&r.realizes.site).copied());
        let root_name: Option<&str> = root_decl.map(|l| l.name.name.as_str());
        let root_template = |o: Origin| match o {
            Origin::Entry(_) => true,
            Origin::Construction(c) => {
                table.root.as_ref().is_some_and(|r| r.constructions.iter().any(|x| x.literal == c))
            }
            Origin::Binding(_) => false,
        };
        let path_of = |k: &InstanceKey| -> String {
            let mut p = root_name.unwrap_or_default().to_string();
            for (i, step) in k.path.iter().enumerate() {
                p.push('.');
                p.push_str(&step.field);
                if i == 0 {
                    if let Some(r) = k.replica {
                        p.push_str(&format!("[{r}]"));
                    }
                }
            }
            p
        };
        let domain_name = |d: crate::placement::DomainId| -> String {
            match &table.domain(d).kind {
                DomainKind::Main => "main".to_string(),
                DomainKind::Pool { name, .. } => format!("pool:{name}"),
                DomainKind::Pinned { anchor, .. } => format!("pinned:{}", path_of(anchor)),
            }
        };
        // The model's declaration a row realizes: a user locus.
        let model_decl = |r: &InstanceRow| -> Option<(LocusDeclId, &hale_syntax::ast::LocusDecl)> {
            let l = *decl_at.get(&r.realizes.as_ref()?.site)?;
            Some((*locus_id.get(&l.name.name)?, l))
        };
        let prefix = |k: &InstanceKey, i: usize| InstanceKey {
            origin: k.origin,
            path: k.path[..i].to_vec(),
            replica: if i == 0 { None } else { k.replica },
        };
        let handed_off = table.handed_off();
        // Keep coverage before projecting user declarations. A template
        // or alternative whose subtree is unenumerable must not borrow
        // a known descendant from another template or alternative.
        let mut coverage: BTreeMap<String, BTreeSet<&InstanceKey>> = BTreeMap::new();
        let mut parents: BTreeMap<String, BTreeSet<&InstanceKey>> = BTreeMap::new();
        // Per path, every user row that reaches it.
        let mut at_path: BTreeMap<String, Vec<Arranged>> = BTreeMap::new();
        for (k, r) in &table.instances {
            if !root_template(k.origin) || handed_off.contains(k) {
                continue;
            }
            let path = path_of(k);
            coverage.entry(path.clone()).or_default().insert(k);
            // The row and every row above it realize a user locus.
            if !(0..k.path.len()).all(|i| table.instances.get(&prefix(k, i)).and_then(model_decl).is_some()) {
                continue;
            }
            let Some((decl, l)) = model_decl(r) else { continue };
            // The field's name in its owner's params, as written; the
            // root's own name for the root.
            let span = match (k.path.last(), r.owner.as_ref().and_then(|o| table.instances.get(o)).and_then(model_decl)) {
                (Some(step), Some((_, owner))) => owner
                    .members
                    .iter()
                    .filter_map(|m| match m {
                        LocusMember::Params(pb) => pb.params.iter().find(|p| p.name.name == step.field),
                        _ => None,
                    })
                    .map(|p| p.name.span)
                    .next()
                    .unwrap_or(l.name.span),
                _ => l.name.span,
            };
            if let Some(owner) = &r.owner {
                parents.entry(path.clone()).or_default().insert(owner);
            }
            at_path.entry(path.clone()).or_default().push(Arranged {
                path,
                decl,
                // The instance's OWN replica index — what codegen bakes
                // into replica `i` and what a keyed subscriber on this
                // field registers under: the replica row's, never an
                // ancestor's copied down (`validate` requires `None` on
                // every path whose last component is not a replica).
                replica: if k.path.len() == 1 { k.replica } else { None },
                domain: domain_name(r.domain),
                parent: r.owner.as_ref().map(path_of),
                span,
            });
        }
        let disagree: Vec<String> = at_path
            .iter()
            .filter(|(path, rows)| {
                rows.iter().any(|a| a.decl != rows[0].decl || a.domain != rows[0].domain)
                    || rows.len() != coverage[*path].len()
                    // Full instance keys retain construction and
                    // alternative identity, which the model path omits.
                    || rows[0].parent.as_ref().is_some_and(|parent| {
                        parents.get(*path) != coverage.get(parent)
                    })
            })
            .map(|(p, _)| p.clone())
            .collect();
        let mut arranged: Vec<Arranged> = Vec::new();
        for (path, rows) in at_path {
            let under = disagree
                .iter()
                .find(|d| path == **d || path.starts_with(&format!("{d}.")) || path.starts_with(&format!("{d}[")));
            match under {
                None => arranged.extend(rows.into_iter().take(1)),
                Some(d) => {
                    for a in rows {
                        let pid = intern_span(&mut records, a.span);
                        holes
                            .entry((
                                EntityRef::LocusDecl(a.decl),
                                HoleKind::RuntimeInheritedPlacement,
                                format!(
                                    "the root's construction templates disagree at `{d}`: the arrangement names \
                                     no instance there"
                                ),
                            ))
                            .or_insert((hale_model::RelationSet::OWNS.union(hale_model::RelationSet::PLACED), None, pid));
                    }
                }
            }
        }
        // Thread domains, canonical order: every domain any
        // instance landed in, plus a reader domain per binding
        // entry (#468: a binding's reader thread is a real
        // domain).
        let mut domain_names: BTreeSet<String> = arranged
            .iter()
            .map(|a| a.domain.clone())
            .collect();
        // Bindings (main's `bindings { }` block).
        struct BindingEntry {
            topic: String,
            span: hale_syntax::Span,
            /// `None` for adapter/shm transports — a declared
            /// external boundary, holed out rather than guessed.
            unix_role: Option<hale_syntax::ast::TransportRole>,
            adapter: bool,
        }
        let mut binding_entries: Vec<BindingEntry> = Vec::new();
        if let Some(main_name) = root_name {
            // The binding rows of main's `bindings { }` block, in the
            // order the entries are declared. The role is the row's: the
            // one rule the checker and lowering read (this bundle is not
            // desugared, so the AST field is still `None` on every
            // inferred binding).
            for row in inputs.bindings.rows.iter().filter(|r| r.locus == main_name) {
                binding_entries.push(BindingEntry {
                    topic: row.topic.clone(),
                    span: row.span,
                    unix_role: row.role,
                    adapter: row.transport != crate::capability::Transport::Unix,
                });
                domain_names.insert(format!("binding:{}", row.topic));
            }
        }
        let domain_id: BTreeMap<&String, ThreadDomainId> =
            domain_names
                .iter()
                .enumerate()
                .map(|(i, n)| (n, ThreadDomainId(i as u32)))
                .collect();
        for n in &domain_names {
            let pid =
                intern_synth(&mut records, "thread domain");
            e.thread_domains.push(ThreadDomain {
                name: n.clone(),
                provenance: pid,
            });
        }
        // The CPU sets (U-5): each arranged domain's resolved affinity,
        // read from the table's domain, never from the written entries,
        // so two entries naming one pool give one row. A CPU set is a
        // column of a thread domain and never a domain of its own: a
        // pool's is the set its one worker may run on, a pinned
        // replica's its own thread's. Where the table's domains behind
        // one name (one per construction template) disagree, or name an
        // empty set, the domain has no row.
        let mut cpu_sets: BTreeMap<String, BTreeSet<Vec<u32>>> = BTreeMap::new();
        let mut clause: BTreeMap<String, hale_syntax::Span> = BTreeMap::new();
        for d in &table.domains {
            let cores = match &d.kind {
                DomainKind::Pool { affinity: Some(c), .. } | DomainKind::Pinned { affinity: Some(c), .. } => c,
                _ => continue,
            };
            let name = domain_name(d.id);
            if !domain_id.contains_key(&name) {
                continue;
            }
            let set: BTreeSet<u32> = cores.0.iter().filter_map(|c| u32::try_from(*c).ok()).collect();
            cpu_sets.entry(name.clone()).or_default().insert(set.into_iter().collect());
            // The affinity clause: the entry that decided a row in the
            // domain.
            let entry = table.instances.values().find_map(|r| match &r.decided_by {
                crate::placement::Decision::Entry { entry, .. } if r.domain == d.id => Some(*entry),
                _ => None,
            });
            if let Some(span) = entry.and_then(|s| bundle.snapshot.site(s.id)).map(|s| s.span) {
                clause.entry(name).or_insert(span);
            }
        }
        for (name, sets) in &cpu_sets {
            let Some(cores) = sets.iter().next().filter(|c| sets.len() == 1 && !c.is_empty()) else { continue };
            let pid = match clause.get(name) {
                Some(span) => intern_span(&mut records, *span),
                None => intern_synth(&mut records, "affinity"),
            };
            r.affined_to.push(hale_model::AffinedTo {
                domain: domain_id[name],
                cores: hale_model::CoreSet(cores.clone()),
                provenance: pid,
            });
        }
        // Instances in canonical (path-sorted) order.
        arranged.sort_by(|a, b| a.path.cmp(&b.path));
        let inst_id: BTreeMap<&String, LocusInstanceId> =
            arranged
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    (&a.path, LocusInstanceId(i as u32))
                })
                .collect();
        for a in &arranged {
            let pid = intern_span(&mut records, a.span);
            e.locus_instances.push(LocusInstance {
                path: a.path.clone(),
                decl: a.decl,
                replica: a.replica,
                provenance: pid,
            });
        }
        for a in &arranged {
            let pid =
                intern_synth(&mut records, "arrangement");
            r.realizes.push(Realizes {
                instance: inst_id[&a.path],
                decl: a.decl,
                provenance: pid,
            });
            r.placed_in.push(PlacedIn {
                instance: inst_id[&a.path],
                domain: domain_id[&a.domain],
                provenance: pid,
            });
        }
        let mut own_edges: Vec<(
            LocusInstanceId,
            LocusInstanceId,
        )> = arranged
            .iter()
            .filter_map(|a| {
                a.parent.as_ref().map(|p2| {
                    (inst_id[p2], inst_id[&a.path])
                })
            })
            .collect();
        own_edges.sort();
        for (parent, child) in own_edges {
            let pid =
                intern_synth(&mut records, "arrangement");
            r.owns.push(Owns {
                parent,
                child,
                provenance: pid,
            });
        }
        // Binding entities + binds rows. Adapter transports are
        // declared external boundaries with opaque internals —
        // they hole out instead of guessing loss semantics.
        //
        // Decoded FIRST, sorted canonically SECOND, ids assigned
        // THIRD. `validate` requires the entity table sorted by
        // (subject, transport, role); assigning ids in source order
        // would hand back an invalid model for a perfectly valid
        // main locus whose two binding entries are authored in
        // reverse subject order.
        struct DecodedBinding {
            topic: hale_model::TopicId,
            subject: hale_model::SubjectId,
            transport: TransportKind,
            role: BindingRole,
            loss: hale_model::keys::BindingLossBehavior,
            span: hale_syntax::Span,
        }
        let mut decoded: Vec<DecodedBinding> = Vec::new();
        for entry in &binding_entries {
            let Some(tid) = topic_id.get(&entry.topic) else {
                continue;
            };
            let pid = intern_span(&mut records, entry.span);
            // An adapter transport, or a unix binding whose role
            // is not inferable (typecheck has diagnosed it; codegen
            // refuses to lower it), is a boundary the model does
            // not describe — a hole, never a guessed row.
            let role = match (entry.adapter, entry.unix_role) {
                (false, Some(hale_syntax::ast::TransportRole::Listen)) => {
                    Some(BindingRole::Listen)
                }
                (false, Some(hale_syntax::ast::TransportRole::Connect)) => {
                    Some(BindingRole::Connect)
                }
                _ => None,
            };
            let Some(role) = role else {
                holes
                    .entry((
                        EntityRef::Topic(*tid),
                        HoleKind::ExternalOpaque,
                        if entry.adapter {
                            "adapter transport: external boundary \
                             with opaque internals"
                                .to_string()
                        } else {
                            "binding role is not inferable: the \
                             route this topic takes is undetermined"
                                .to_string()
                        },
                    ))
                    .or_insert((
                        hale_model::RelationSet::BINDS.union(
                            hale_model::RelationSet::DELIVERY,
                        ),
                        None,
                        pid,
                    ));
                continue;
            };
            // Loss behavior follows the ROLE, because that is what
            // the runtime does: the connect side is the publish
            // side, where a send failure marks the entry lost and
            // `or wait` can park through the reconnect window
            // (WaitCapable); the listen side re-arms on peer EOF
            // (peer EOF is not connection loss) and a link it
            // cannot serve is structural (Fail).
            let loss = match role {
                BindingRole::Connect => {
                    hale_model::keys::BindingLossBehavior::WaitCapable
                }
                BindingRole::Listen => {
                    hale_model::keys::BindingLossBehavior::Fail
                }
            };
            decoded.push(DecodedBinding {
                topic: *tid,
                subject: e.topics[tid.index()].subject,
                transport: TransportKind::Unix,
                role,
                loss,
                span: entry.span,
            });
        }
        decoded.sort_by_key(|d| {
            (d.subject, d.transport.clone(), d.role, d.topic)
        });
        decoded.dedup_by_key(|d| {
            (d.subject, d.transport.clone(), d.role)
        });
        let mut bind_rows: Vec<(
            hale_model::TopicId,
            BindingId,
        )> = Vec::new();
        for (i, d) in decoded.iter().enumerate() {
            let pid = intern_span(&mut records, d.span);
            let bid = BindingId(i as u32);
            e.bindings.push(Binding {
                subject: d.subject,
                transport: d.transport.clone(),
                role: d.role,
                loss: d.loss,
                provenance: pid,
            });
            bind_rows.push((d.topic, bid));
        }
        bind_rows.sort();
        for (t, b) in bind_rows {
            let pid = intern_synth(&mut records, "binding");
            r.binds.push(TopicBinding {
                topic: t,
                binding: b,
                provenance: pid,
            });
        }
        // Dynamic births: a method-body instantiation site's
        // instance is not in the arrangement — its ownership and
        // placement are runtime facts. Typed holes keep the
        // capability account honest (RuntimeInheritedPlacement is
        // exactly this shape).
        let og = inputs.ownership;
        let free_fn_births =
            crate::ownership_graph::free_fn_birth_sites(bundle);
        // Params-default births ARE the arrangement — only sites
        // outside every params block of their enclosing locus are
        // dynamic.
        let params_spans: BTreeMap<
            &str,
            Vec<hale_syntax::Span>,
        > = decls_by_name
            .iter()
            .map(|(n, d)| {
                (
                    *n,
                    d.members
                        .iter()
                        .filter_map(|m| match m {
                            LocusMember::Params(pb) => {
                                Some(pb.span)
                            }
                            _ => None,
                        })
                        .collect(),
                )
            })
            .collect();
        // Method-body births (the ownership graph's sites) PLUS
        // free-function births — `fn main() { EchoL { }; }` is the
        // most common arrangement-free program shape in the corpus,
        // and it must not read as "no instances, exact placement".
        let dyn_sites: Vec<(&str, &str, hale_syntax::Span)> = og
            .sites
            .iter()
            .map(|s| {
                (
                    s.child_ty.as_str(),
                    s.enclosing_locus.as_str(),
                    s.span,
                )
            })
            .chain(
                free_fn_births
                    .iter()
                    .map(|(ty, sp)| (ty.as_str(), "", *sp)),
            )
            .collect();
        for (child_ty, enclosing, span) in dyn_sites {
            // `fn main() { App { }; }` — the birth of the
            // arrangement ROOT — is not a dynamic birth: it is how
            // the arrangement is entered, and the root instance is
            // already modeled (path `App`, domain `main`). Every
            // OTHER free-standing birth is outside the arrangement.
            if root_name == Some(child_ty) {
                continue;
            }
            let in_arrangement = params_spans
                .get(enclosing)
                .is_some_and(|spans| {
                    spans.iter().any(|ps| {
                        span.start >= ps.start && span.end <= ps.end
                    })
                });
            if in_arrangement {
                continue;
            }
            let pid = intern_span(&mut records, span);
            // Anchored at the BORN locus, not the birthplace: the
            // fact hidden is "instances of this locus exist that
            // the arrangement does not name", which is true of the
            // child whether it was born in a method or a free fn.
            let Some(lid) = locus_id.get(&child_ty.to_string()) else {
                continue;
            };
            let at = EntityRef::LocusDecl(*lid);
            holes
                .entry((
                    at,
                    HoleKind::RuntimeInheritedPlacement,
                    "instance born outside the arrangement: \
                     owner and placement resolve at runtime"
                        .to_string(),
                ))
                .or_insert((
                    hale_model::RelationSet::OWNS.union(
                        hale_model::RelationSet::PLACED,
                    ),
                    None,
                    pid,
                ));
        }
    }

    let mut hole_rows: Vec<Hole> = holes
        .into_iter()
        .map(|((at, kind, reason), (hides, site, pid))| Hole {
            at,
            kind,
            hides,
            reason,
            authored_site: site,
            provenance: pid,
        })
        .collect();
    hole_rows.sort_by(|a, b| {
        (a.at, a.kind.clone(), &a.reason)
            .cmp(&(b.at, b.kind.clone(), &b.reason))
    });

    // capabilities: computed FROM the holes so the two accounts
    // cannot disagree by construction. The Change-2 scope leaves
    // ownership/placement/routes/cardinality/delivery unclaimed.
    // Absorption residue participates (round 8): a CallHole /
    // PublishHole / Truncated inside a stdlib interior is
    // unresolved knowledge exactly like a top-level hole row.
    let absorption_hides = {
        let mut m = hale_model::RelationSet(0);
        for a in &stdlib_absorption {
            for n in &a.nodes {
                for ev in &n.events {
                    match ev {
                        hale_model::AbsorbedEvent::CallHole(_) => {
                            m = m
                                .union(hale_model::RelationSet::CALLS)
                                .union(
                                    hale_model::RelationSet::EFFECTS,
                                )
                                // Change 5h round 2: an unfollowable
                                // interior edge may reach an
                                // allocation or a blocking call, so
                                // the COST account beyond it is not
                                // complete either.
                                .union(hale_model::RelationSet::COSTS);
                        }
                        hale_model::AbsorbedEvent::PublishHole { .. } => {
                            m = m.union(
                                hale_model::RelationSet::PUBLISHES,
                            );
                        }
                        hale_model::AbsorbedEvent::Truncated => {
                            m = m
                                .union(hale_model::RelationSet::CALLS)
                                .union(
                                    hale_model::RelationSet::PUBLISHES,
                                )
                                .union(
                                    hale_model::RelationSet::EFFECTS,
                                )
                                // An unexplored interior may contain
                                // anything, costs included.
                                .union(hale_model::RelationSet::COSTS);
                        }
                        _ => {}
                    }
                }
            }
        }
        m
    };
    let hides_any = |fam: hale_model::RelationSet| {
        hole_rows.iter().any(|h| h.hides.intersects(fam))
            || absorption_hides.intersects(fam)
    };
    let capabilities = Capabilities {
        exact_calls: !hides_any(hale_model::RelationSet::CALLS),
        exact_publishes: !hides_any(
            hale_model::RelationSet::PUBLISHES,
        ),
        exact_subscribes: !hides_any(
            hale_model::RelationSet::SUBSCRIBES,
        ),
        exact_key_filters: !hides_any(
            hale_model::RelationSet::KEY_FILTERS,
        ),
        exact_effects: !hides_any(hale_model::RelationSet::EFFECTS),
        // Endpoint multiplicity comes from a complete closed-world
        // AST enumeration — counts are exact unless a hole says the
        // endpoint sets are incomplete (round 1: leaving this
        // unclaimed made endpoint adequacy permanently `degraded`,
        // including for fully known publisher/subscriber counts).
        exact_cardinality: !hides_any(
            hale_model::RelationSet::CARDINALITY,
        ),
        // Change 8: the arrangement tables are populated, so the
        // ownership/placement/binding accounts are positive unless
        // a hole (a dynamic birth, an adapter boundary) withdraws
        // them.
        exact_ownership: !hides_any(hale_model::RelationSet::OWNS),
        exact_placement: !hides_any(
            hale_model::RelationSet::PLACED,
        ),
        exact_routes: !hides_any(hale_model::RelationSet::BINDS),
        exact_costs: !hides_any(hale_model::RelationSet::COSTS),
        ..Capabilities::default()
    };

    // ---- per-call COST sites (GH #476 Change 5h) ----
    //
    // Site-grained like publishes, and for the same reason: a
    // per-call budget is a statement about ONE invocation, so
    // whether a site sits inside a loop is the difference between a
    // finite count and an unbounded one. `publish` and `fanout` are
    // deliberately absent — `relations.publishes` plus the delivery
    // join already answer those, and a second copy would be a
    // second authority.
    {
        let frames = crate::quantitative::frame_map(&programs);
        for (k, fs) in &summary.fns {
            if !user_key(k) {
                continue;
            }
            let Some(f) = fn_id.get(&fn_name(k)) else { continue };
            for site in &fs.sites {
                let pid = intern_span(&mut records, site.span);
                r.costs.push(hale_model::CostSite {
                    function: *f,
                    dimension: hale_model::CostDimension::Alloc,
                    amount: 1,
                    in_loop: site.loop_depth > 0,
                    provenance: pid,
                });
            }
            // Blocking points, from the SAME stdlib classification
            // the quantitative engine reads (round 2). `Block` was
            // declared in the cost vocabulary but nothing emitted
            // it, so an ordinary analyzed function calling a known
            // blocking operation had no block row while
            // `exact_costs` claimed its cost account was complete.
            for edge in &fs.calls {
                let crate::alloc_summary::Callee::Unresolved(name) =
                    &edge.callee
                else {
                    continue;
                };
                let segs: Vec<&str> = name.split("::").collect();
                let Some(eff) =
                    crate::stdlib_surface::effects_for(&segs)
                else {
                    continue;
                };
                if !eff
                    .contains(crate::stdlib_surface::EffectSet::BLOCK)
                {
                    continue;
                }
                let pid = intern_span(&mut records, edge.span);
                r.costs.push(hale_model::CostSite {
                    function: *f,
                    dimension: hale_model::CostDimension::Block,
                    amount: 1,
                    in_loop: edge.loop_depth > 0,
                    provenance: pid,
                });
            }
            if let Some(bytes) = frames.get(k) {
                let fn_prov = e.functions[f.index()].provenance;
                r.costs.push(hale_model::CostSite {
                    function: *f,
                    dimension: hale_model::CostDimension::FrameBytes,
                    amount: *bytes,
                    // A frame is charged once per call, never per
                    // loop iteration — the same frame is reused.
                    in_loop: false,
                    provenance: fn_prov,
                });
            }
        }
        r.costs.sort_by(|a, b| {
            (a.function.0, a.dimension, a.provenance.0)
                .cmp(&(b.function.0, b.dimension, b.provenance.0))
        });
    }


    // entrypoint: the main locus, else "main".
    let entrypoint = ast
        .loci
        .iter()
        .find(|l| l.is_main)
        .map(|l| l.name.name.clone())
        .unwrap_or_else(|| "main".to_string());

    prov.records = records;
    let model = ApplicationModel {
        analyses: hale_model::Analyses {
            dispatch_gates,
            stdlib_absorption,
        },
        header: ModelHeader {
            semantics: MODEL_SEMANTICS_V1,
            entrypoint,
        },
        entities: e,
        relations: r,
        labels,
        weights: Vec::new(),
        holes: hole_rows,
        capabilities,
        provenance: prov,
    };
    debug_assert_eq!(model.validate(), Ok(()));
    model
}

/// Deterministic internal rendering of a model — the `hale model
/// dump` surface. EXPLICITLY not a stable format: it exists for
/// inspection, corpus snapshots, and the Change-2 differential
/// tests; the lossless external encoding is Change 3's projection.
/// Canonical table order in, deterministic text out.
pub fn render_internal(m: &ApplicationModel) -> String {
    let mut s = String::new();
    s.push_str("# hale ApplicationModel (internal dump — not a stable format)\n");
    s.push_str(&format!(
        "semantics {}\nentrypoint {}\n",
        m.header.semantics, m.header.entrypoint
    ));
    let e = &m.entities;
    let fn_name =
        |id: FunctionId| e.functions[id.index()].display.clone();
    let locus_name = |id: LocusDeclId| e.loci[id.index()].display.clone();
    let subject_pat = |id: SubjectId| e.subjects[id.index()].pattern.clone();
    let topic_name = |id: TopicId| e.topics[id.index()].display.clone();

    s.push_str(&format!("loci ({}):\n", e.loci.len()));
    for l in &e.loci {
        s.push_str(&format!(
            "  {}{}\n",
            l.display,
            if l.sealed { " @sealed" } else { "" }
        ));
    }
    s.push_str(&format!("functions ({}):\n", e.functions.len()));
    for f in &e.functions {
        s.push_str(&format!(
            "  {} [{}]{}\n",
            f.display,
            match f.kind {
                FunctionKind::Hook => "hook",
                FunctionKind::Method => "method",
                FunctionKind::Free => "free",
                FunctionKind::Mode => "mode",
                FunctionKind::FailureHandler => "failure",
            },
            if f.effects.is_empty() {
                String::new()
            } else {
                format!(" {{{}}}", f.effects.join(","))
            }
        ));
    }
    s.push_str(&format!("topics ({}):\n", e.topics.len()));
    for t in &e.topics {
        let mut line = format!(
            "  {} -> {} payload#{:016x}",
            t.display,
            subject_pat(t.subject),
            e.payloads[t.payload.index()].hash
        );
        if let Some(k) = &t.key {
            line.push_str(&format!(
                " keyed_by {} on_unmatched {}",
                k.field,
                match k.on_unmatched {
                    KeyOnUnmatched::Swallow => "swallow",
                    KeyOnUnmatched::Fail => "fail",
                    KeyOnUnmatched::Fallback => "fallback",
                }
            ));
        }
        if let Some(b) = &t.bound {
            line.push_str(&format!(" bounded({})", b.capacity));
        }
        s.push_str(&line);
        s.push('\n');
    }
    s.push_str(&format!("subjects ({}):\n", e.subjects.len()));
    for x in &e.subjects {
        s.push_str(&format!(
            "  {}{}\n",
            x.pattern,
            if x.exact { "" } else { " (pattern)" }
        ));
    }
    s.push_str(&format!(
        "groups ({}): {}\n",
        e.groups.len(),
        e.groups
            .iter()
            .map(|g| {
                format!(
                    "{}{}",
                    g.display,
                    if g.may_be_empty { " may_be_empty" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s.push_str(&format!(
        "declaration universe: types [{}], interfaces [{}], other [{}]\n",
        e.types
            .iter()
            .map(|t| t.display.clone())
            .collect::<Vec<_>>()
            .join(", "),
        e.interfaces
            .iter()
            .map(|t| t.display.clone())
            .collect::<Vec<_>>()
            .join(", "),
        e.declarations
            .iter()
            .map(|d| format!("{:?}:{}", d.kind, d.display))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    let r = &m.relations;
    s.push_str(&format!("calls ({} sites):\n", r.calls.len()));
    for c in &r.calls {
        s.push_str(&format!(
            "  {} -> {} [{}]{}{}{}\n",
            fn_name(c.from),
            fn_name(c.to),
            match &c.dispatch {
                DispatchKind::Direct => "direct".to_string(),
                DispatchKind::Interface { interface } =>
                    format!("iface {}", interface),
                DispatchKind::ViaStdlib => "via-stdlib".to_string(),
            },
            format!(" site {}", c.site),
            if c.in_loop { " loop" } else { "" },
            if c.unbounded { " unbounded" } else { "" },
        ));
    }
    s.push_str(&format!(
        "dead_interface_calls ({}):\n",
        r.dead_interface_calls.len()
    ));
    for d in &r.dead_interface_calls {
        s.push_str(&format!(
            "  {} -[dead]-> {}.{}\n",
            fn_name(d.from),
            d.interface,
            d.method
        ));
    }
    s.push_str(&format!("publishes ({} sites):\n", r.publishes.len()));
    for p in &r.publishes {
        s.push_str(&format!(
            "  {} -> {}{} site {}{}\n",
            fn_name(p.function),
            subject_pat(p.subject),
            p.declared_topic
                .map(|t| format!(" (topic {})", topic_name(t)))
                .unwrap_or_default(),
            p.site,
            match &p.key_domain {
                None => String::new(),
                Some(KeyDomain::AnyOfType(t)) =>
                    format!(" key any-of {}", t),
                Some(other) => format!(" key {:?}", other),
            }
        ));
    }
    s.push_str(&format!("subscribes ({}):\n", r.subscribes.len()));
    for x in &r.subscribes {
        s.push_str(&format!(
            "  {}{} -> {} site {} where {:?} cap {:?} shed {:?}\n",
            subject_pat(x.subject),
            x.declared_topic
                .map(|t| format!(" (topic {})", topic_name(t)))
                .unwrap_or_default(),
            fn_name(x.handler),
            x.site,
            x.key_predicate,
            x.capacity,
            x.shed
        ));
    }
    s.push_str(&format!("supervises ({}):\n", r.supervises.len()));
    for x in &r.supervises {
        s.push_str(&format!(
            "  {} -> {} on {} ops [{}]{}\n",
            locus_name(x.parent),
            match &x.child {
                SupervisedRef::Locus(id) => locus_name(*id),
                SupervisedRef::External(n) =>
                    format!("(external) {}", n),
            },
            x.error_type,
            x.policy.ops.join(","),
            x.policy
                .retry_bound
                .map(|n| format!(" retry {}", n))
                .unwrap_or_default()
        ));
    }
    s.push_str(&format!(
        "group_selectors ({}):\n",
        r.group_selectors.len()
    ));
    for gs in &r.group_selectors {
        s.push_str(&format!(
            "  {}[{}] = {}\n",
            e.groups[gs.group.index()].display,
            gs.ordinal,
            match &gs.selector {
                SelectorForm::Named { display, .. } => display.clone(),
                SelectorForm::SeedGlob { display, .. } =>
                    format!("{} (glob)", display),
            }
        ));
    }
    s.push_str(&format!("holes ({}):\n", m.holes.len()));
    for h in &m.holes {
        s.push_str(&format!(
            "  {:?} at {:?}: {} (hides {:#x})\n",
            h.kind, h.at, h.reason, h.hides.0
        ));
    }
    s.push_str(&format!(
        "capabilities: calls={} pub={} sub={} keys={} effects={}\n",
        m.capabilities.exact_calls,
        m.capabilities.exact_publishes,
        m.capabilities.exact_subscribes,
        m.capabilities.exact_key_filters,
        m.capabilities.exact_effects
    ));
    s.push_str(&format!(
        " ownership={} placement={} routes={}\n",
        m.capabilities.exact_ownership,
        m.capabilities.exact_placement,
        m.capabilities.exact_routes
    ));
    // U-5 (F.40 phase 3, P1): the CPU set each thread domain may run on.
    // Printed only where a domain has one, so a program with no affinity
    // dumps as it did.
    if !m.relations.affined_to.is_empty() {
        s.push_str(&format!("affined_to ({}):\n", m.relations.affined_to.len()));
        for a in &m.relations.affined_to {
            let cores: Vec<String> = a.cores.0.iter().map(|c| c.to_string()).collect();
            s.push_str(&format!(
                "  {} cpus [{}]\n",
                m.entities.thread_domains[a.domain.index()].name,
                cores.join(",")
            ));
        }
    }
    // GH #476 Change 8: the derived lowering plan. Not model rows —
    // a CONCLUSION, printed here because this dump is the survey
    // surface #464's stage 0 asks its question of ("how much queued
    // bus traffic is same-thread-domain?").
    let plan = hale_model::dispatch_plan::DispatchPlan::derive(m);
    let (same, total) = plan.same_domain_queued();
    s.push_str(&format!(
        "dispatch_plan ({} subjects, {} same-domain queued):\n",
        total, same
    ));
    for sp in &plan.subjects {
        s.push_str(&format!(
            "  {} {}{} pub[{}] sub[{}]{}\n",
            sp.subject,
            sp.flavor.as_str(),
            sp.ineligible_reason
                .as_deref()
                .map(|r| format!(" ({})", r))
                .unwrap_or_default(),
            sp.publisher_domains.join(","),
            sp.subscriber_domains.join(","),
            if sp.same_domain { " same-domain" } else { "" }
        ));
    }
    s
}
