//! GH #476 Change 2 — derive the canonical `ApplicationModel` from a
//! checked bundle.
//!
//! One entry point, [`derive_application_model`], assembling the
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
//! Change 3). [`builds`] counts invocations in-process, and
//! `HALE_MODEL_TRACE=1` prints one stderr line per derivation —
//! the cross-process hook the no-claims LSP/check test uses to
//! prove the builder never ran ("cached" must not become "always
//! built").
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
use std::sync::atomic::{AtomicU64, Ordering};

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
    Program, RecoveryModifier, RecoveryOp,
    ShedPolicy as AstShedPolicy, Stmt, TopDecl, TopicDecl, TypeExpr,
    UnmatchedPolicy,
};

use crate::alloc_summary::{self, Callee, EffectSiteKind, FnKey};
use crate::symbol::Bundle;

static BUILDS: AtomicU64 = AtomicU64::new(0);

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

/// How many times the builder has run in this process — the
/// demand-gating instrumentation. The no-claims check path must
/// leave this at zero.
pub fn builds() -> u64 {
    BUILDS.load(Ordering::Relaxed)
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

pub fn derive_application_model(bundle: &Bundle<'_>) -> ApplicationModel {
    BUILDS.fetch_add(1, Ordering::Relaxed);
    if std::env::var("HALE_MODEL_TRACE").as_deref() == Ok("1") {
        eprintln!("[hale-model] deriving ApplicationModel");
    }

    let programs: Vec<&Program> =
        bundle.programs.values().copied().collect();
    let (top, _diags) = crate::resolve::build_top_scope(bundle);
    let graph = crate::bus_graph::build_bus_graph(bundle, &top);
    let summary = alloc_summary::summarize_programs_with_renames(
        &programs,
        &bundle.import_renames,
    );
    let merged = crate::stdlib_bodies::summarize_with_stdlib_and_renames(
        &programs,
        &bundle.import_renames,
    );
    let vmodel =
        crate::model::Model::derive(&programs, &bundle.import_renames);
    let effect_names = crate::effects::effect_names_of(&programs);
    let ffi = crate::effects::ffi_names(&programs);

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
            digest: u64::from_str_radix(&sf.digest, 16).unwrap_or(0),
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
            .filter(|f| {
                pos >= f.base && pos < f.base.saturating_add(f.len + 1)
            })
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
            Provenance::Synthetic {
                origin: "unplaceable span".to_string(),
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
    let mut locus_rows: BTreeMap<String, (bool, hale_syntax::Span)> =
        BTreeMap::new();
    for l in &ast.loci {
        locus_rows
            .insert(l.name.name.clone(), (l.sealed, l.name.span));
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
        /// The behavior analysis did not walk this body (module
        /// scope / on_failure at Change 2) — emits an
        /// UnanalyzedBody hole.
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
    {
        // depth > 0 = inside a module: the behavior summary's body
        // walk does not recurse into modules, so those bodies are
        // UNANALYZED at Change 2 and must hole out (review round 7).
        fn walk_free<'a>(
            items: &'a [TopDecl],
            depth: u32,
            out: &mut Vec<(&'a str, hale_syntax::Span, bool)>,
        ) {
            for item in items {
                match item {
                    TopDecl::Fn(f) => out.push((
                        f.name.name.as_str(),
                        f.name.span,
                        depth > 0,
                    )),
                    TopDecl::Module(m) => {
                        walk_free(&m.items, depth + 1, out)
                    }
                    _ => {}
                }
            }
        }
        fn walk_loci_mod<'a>(
            items: &'a [TopDecl],
            depth: u32,
            out: &mut Vec<(&'a AstLocusDecl, bool)>,
        ) {
            for item in items {
                match item {
                    TopDecl::Locus(l) => out.push((l, depth > 0)),
                    TopDecl::Module(m) => {
                        walk_loci_mod(&m.items, depth + 1, out)
                    }
                    _ => {}
                }
            }
        }
        let mut frees = Vec::new();
        let mut mod_loci = Vec::new();
        for pr in &programs {
            walk_free(&pr.items, 0, &mut frees);
            walk_loci_mod(&pr.items, 0, &mut mod_loci);
        }
        for (n, sp, in_module) in frees {
            fn_rows.insert(
                n.to_string(),
                FnInfo {
                    kind: FunctionKind::Free,
                    locus: None,
                    display: name(n),
                    span: Some(sp),
                    unanalyzed: in_module,
                },
            );
        }
        for (l, in_module) in &mod_loci {
            let ld = l.name.name.clone();
            let ld_display = name(&ld);
            for m in &l.members {
                let (fname, kind, sp) = match m {
                    LocusMember::Fn(f) => (
                        f.name.name.clone(),
                        FunctionKind::Method,
                        f.name.span,
                    ),
                    LocusMember::Lifecycle(lc) => (
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
                        // hole. Uniqueness: one handler per
                        // (child, err) signature.
                        let sig = fd
                            .params
                            .iter()
                            .map(|pa| te_name_of(&pa.ty))
                            .collect::<Vec<_>>()
                            .join(",");
                        fn_rows.insert(
                            format!("{}::on_failure({})", ld, sig),
                            FnInfo {
                                kind: FunctionKind::Hook,
                                locus: Some(ld.clone()),
                                display: format!(
                                    "{}::on_failure({})",
                                    ld_display,
                                    fd.params
                                        .iter()
                                        .map(|pa| name(&te_name_of(
                                            &pa.ty
                                        )))
                                        .collect::<Vec<_>>()
                                        .join(",")
                                ),
                                span: Some(fd.span),
                                unanalyzed: true,
                            },
                        );
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
                        unanalyzed: *in_module,
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
    let fn_id: BTreeMap<&String, FunctionId> = fn_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (k, FunctionId(i as u32)))
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
        raw: String,
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
    let wire_subjects =
        crate::topic_identity::topic_wire_subjects(&all_items);
    // Keyed by RAW name — canonical identity — with the author
    // spelling carried alongside.
    let mut topic_decl_by_name: BTreeMap<String, TInfo> = BTreeMap::new();
    for t in &ast.topics {
        let raw = t.name.name.clone();
        // The wire map is keyed by the RAW name; a subject-less
        // topic's default wire subject is likewise the raw name
        // (parent joins included) — exactly the artifact's rule.
        let wire = wire_subjects
            .get(&raw)
            .cloned()
            .unwrap_or_else(|| raw.clone());
        topic_decl_by_name
            .insert(raw.clone(), TInfo { raw, decl: t, wire });
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
    let shape_of_type = |raw_ty: &str| -> (String, u64) {
        let shape = crate::topic_identity::canonical_type_shape(
            &all_items, raw_ty,
        );
        if shape.is_empty() {
            let opaque = format!("opaque:{}", raw_ty);
            let h = fnv(&opaque);
            (opaque, h)
        } else {
            let h = fnv(&shape);
            (shape, h)
        }
    };
    // The ONE payload-contract rule for a type EXPRESSION — used by
    // declared topics and explicit `of type T` endpoint clauses
    // alike (round 10; endpoint clauses previously collapsed every
    // non-named form to `opaque:?` through a name-only path):
    //   bare named struct → canonical structural shape;
    //   every other form  → opaque over the structural descriptor.
    let contract_of_te = |te: &TypeExpr| -> (String, u64) {
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
    let mut payload_rows: BTreeMap<(String, u64), ()> = BTreeMap::new();
    let mut topic_payload: BTreeMap<String, (String, u64)> =
        BTreeMap::new();
    for (tname, info) in &topic_decl_by_name {
        let key = contract_of_te(&info.decl.payload);
        payload_rows.insert(key.clone(), ());
        topic_payload.insert(tname.clone(), key);
    }
    let mut endpoint_payload: BTreeMap<String, (String, u64)> =
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
    let unresolved_payload = ("?".to_string(), fnv("?"));
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
                        known: Option<(String, u64)>| {
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
    let payload_id: BTreeMap<&(String, u64), PayloadContractId> =
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
        (hale_model::RelationSet, ProvenanceId),
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
                    let site = site_of(edge.dispatch_group);
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
                    let site = site_of(edge.dispatch_group);
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
                                hale_model::RelationSet::CALLS.union(
                                    hale_model::RelationSet::EFFECTS,
                                ),
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
                                HoleKind::UntypedReceiver,
                                format!(
                                    "method call `{}` on untyped \
                                     receiver",
                                    n
                                ),
                            ))
                            .or_insert((
                                hale_model::RelationSet::CALLS.union(
                                    hale_model::RelationSet::EFFECTS,
                                ),
                                pid,
                            ));
                    } else if edge.receiver_present {
                        // A TYPED method miss (`w.tick()` with
                        // `recv_ty` known): resolve by call shape —
                        // raw `RecvTy::method` in the declaration
                        // universe, where a module-scoped locus's
                        // method lands. A stdlib/external method
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
                        // call to a module-scoped fn arrives
                        // Unresolved — but the DECLARATION universe
                        // knows the target. The edge is authored
                        // fact and must exist ("a concrete path
                        // beats a hole" is impossible if the path is
                        // dropped); the callee's UnanalyzedBody hole
                        // bounds any reasoning past it (round 9).
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
            ProvenanceId,
        ),
    > = BTreeMap::new();
    for (k, fs) in &summary.fns {
        if !user_key(k) {
            continue;
        }
        let from = fn_id[&fn_name(k)];
        let mut site: u32 = 0;
        for s in &fs.effect_sites {
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
                        (from, subject_id[&subject_str], site),
                        (
                            declared,
                            payload,
                            keyed,
                            disposition_at(
                                s.span.start.as_usize() as u32
                            ),
                            pid,
                        ),
                    );
                    site += 1;
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

    // supervision (same walk as the artifact's, per-handler).
    let mut sup: BTreeMap<
        (LocusDeclId, SupervisedRef, String),
        (Vec<String>, Option<u32>, ProvenanceId),
    > = BTreeMap::new();
    {
        fn te_name(t: &TypeExpr) -> String {
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
        fn walk_ops(
            b: &Block,
            ops: &mut Vec<String>,
            retry: &mut Option<u32>,
        ) {
            for st in &b.stmts {
                match st {
                    Stmt::Recovery { op, modifier, .. } => {
                        let n = match op {
                            RecoveryOp::Restart => "restart",
                            RecoveryOp::RestartInPlace => {
                                "restart_in_place"
                            }
                            RecoveryOp::Quarantine => "quarantine",
                            RecoveryOp::Reorganize => "reorganize",
                            RecoveryOp::Bubble => "bubble",
                        };
                        if !ops.iter().any(|o| o == n) {
                            ops.push(n.to_string());
                        }
                        if let Some(RecoveryModifier::For(
                            Expr::Literal(Literal::Int(kk), _),
                        )) = modifier
                        {
                            *retry = Some(*kk as u32);
                        }
                    }
                    Stmt::If(i) => {
                        walk_ops(&i.then_block, ops, retry);
                        let mut cur = i.else_block.as_deref();
                        while let Some(eb) = cur {
                            match eb {
                                ElseBranch::Else(bb) => {
                                    walk_ops(bb, ops, retry);
                                    cur = None;
                                }
                                ElseBranch::ElseIf(ei) => {
                                    walk_ops(
                                        &ei.then_block,
                                        ops,
                                        retry,
                                    );
                                    cur = ei.else_block.as_deref();
                                }
                            }
                        }
                    }
                    Stmt::While { body, .. }
                    | Stmt::For { body, .. } => {
                        walk_ops(body, ops, retry)
                    }
                    Stmt::Block(bb) => walk_ops(bb, ops, retry),
                    _ => {}
                }
            }
        }
        for l in &ast.loci {
            for member in &l.members {
                if let LocusMember::Failure(fd) = member {
                    let mut ops = Vec::new();
                    let mut retry = None;
                    walk_ops(&fd.body, &mut ops, &mut retry);
                    let parent = locus_id[&l.name.name];
                    let child_name = fd
                        .params
                        .first()
                        .map(|p| te_name(&p.ty))
                        .unwrap_or_else(|| "?".to_string());
                    let child = match locus_id.get(&child_name) {
                        Some(id) => SupervisedRef::Locus(*id),
                        None => SupervisedRef::External(child_name),
                    };
                    let err = fd
                        .params
                        .get(1)
                        .map(|p| te_name(&p.ty))
                        .unwrap_or_else(|| "?".to_string());
                    let pid = intern_span(&mut records, fd.span);
                    sup.insert(
                        (parent, child, err),
                        (ops, retry, pid),
                    );
                }
            }
        }
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
    for (n, (sealed, sp)) in &locus_rows {
        let pid = intern_span(&mut records, *sp);
        e.loci.push(LocusDecl {
            name: n.clone(),
            display: name(n),
            sealed: *sealed,
            provenance: pid,
        });
    }
    // Effects per fn (the derived classes the artifact exports).
    let mut derived_effects: BTreeMap<String, Vec<String>> =
        BTreeMap::new();
    for k in merged.fns.keys() {
        if !user_key(k) {
            continue;
        }
        let eff = crate::frontier::infer_effects(&merged, k, &ffi);
        let classes =
            crate::frontier::render_effects_named(eff, &effect_names);
        if !classes.is_empty() {
            derived_effects.insert(fn_name(k), classes);
        }
    }
    for (n, info) in &fn_rows {
        let pid = match info.span {
            Some(sp) => intern_span(&mut records, sp),
            None => intern_synth(&mut records, "fn (summary key)"),
        };
        e.functions.push(Function {
            name: n.clone(),
            display: info.display.clone(),
            kind: info.kind,
            effects: derived_effects.get(n).cloned().unwrap_or_default(),
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
    for ((shape, hash), ()) in &payload_rows {
        let pid = intern_synth(&mut records, "payload contract");
        e.payloads.push(PayloadContract {
            shape: shape.clone(),
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
    for (n, info) in &fn_rows {
        if let Some(ld) = &info.locus {
            if let Some(lid) = locus_id.get(ld) {
                let pid =
                    intern_synth(&mut records, "locus membership");
                r.member_of.push(MemberOf {
                    function: fn_id[n],
                    locus: *lid,
                    provenance: pid,
                });
            }
        }
        if let Some((phase, _)) = phase_of_pairs.get(n) {
            let pid = intern_synth(&mut records, "phase relation");
            r.phase_of.push(PhaseOf {
                function: fn_id[n],
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
    for ((f, s, site), (declared, payload, keyed, dispo, pid)) in
        &publishes
    {
        r.publishes.push(Publish {
            function: *f,
            subject: *s,
            declared_topic: *declared,
            payload: *payload,
            site: *site,
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
    for ((parent, child, err), (ops, retry, pid)) in &sup {
        r.supervises.push(Supervises {
            parent: *parent,
            child: child.clone(),
            error_type: err.clone(),
            policy: SupervisionPolicy {
                ops: ops.clone(),
                retry_bound: *retry,
            },
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

    // labels: declared effect carriers.
    let mut labels: Vec<LabelRow> = Vec::new();
    {
        let mut rows: BTreeSet<(EntityRef, String)> = BTreeSet::new();
        for (k, set) in &summary.carries {
            if !user_key(k) {
                continue;
            }
            let classes = crate::frontier::render_effects_named(
                *set,
                &effect_names,
            );
            for c in classes {
                rows.insert((
                    EntityRef::Function(fn_id[&fn_name(k)]),
                    c,
                ));
            }
        }
        for (at, label) in rows {
            let pid =
                intern_synth(&mut records, "declared effect carrier");
            labels.push(LabelRow {
                at,
                label,
                provenance: pid,
            });
        }
    }

    // Unanalyzed bodies (module scope, on_failure): declared
    // executable entities whose calls/publishes/effects the summary
    // never walked — typed holes keep the capabilities honest.
    for (n, info) in &fn_rows {
        if info.unanalyzed {
            let pid = match info.span {
                Some(sp) => intern_span(&mut records, sp),
                None => intern_synth(&mut records, "unanalyzed body"),
            };
            holes
                .entry((
                    EntityRef::Function(fn_id[n]),
                    HoleKind::UnanalyzedBody,
                    "body not walked by the behavior analysis"
                        .to_string(),
                ))
                .or_insert((
                    hale_model::RelationSet::CALLS
                        .union(hale_model::RelationSet::PUBLISHES)
                        .union(hale_model::RelationSet::EFFECTS),
                    pid,
                ));
        }
    }

    // Declared publisher ends: `bus { publish T; }` — the endpoint
    // grain, independent of whether any send exists.
    {
        let mut ends: BTreeMap<
            (hale_model::LocusDeclId, SubjectId),
            (Option<TopicId>, PayloadContractId, ProvenanceId),
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
                    ends.entry((*lid, subject_id[&subj_str]))
                        .or_insert((declared, payload, pid));
                }
            }
        }
        for ((lid, sid), (declared, payload, pid)) in ends {
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

    // The Change-3 bridge: the legacy artifact's fn sort, recorded
    // so TopologyShapeV1 projects from the model alone.
    let mut legacy_fns: Vec<hale_model::FunctionId> = summary
        .fns
        .keys()
        .filter(|k| user_key(k))
        .map(|k| fn_id[&fn_name(k)])
        .collect();
    legacy_fns.sort();
    legacy_fns.dedup();
    // …and the legacy contracted rows, from the SHARED legacy walk
    // (one Boolean, no revisit) — NOT from the model's lattice rows,
    // whose loop bits can legitimately be stronger. This is what
    // lets Change 3 reproduce serialized `calls_via_stdlib` (and
    // therefore the TopologyShapeV1 hash) from the model alone
    // (round 9).
    let legacy_via: Vec<(
        hale_model::FunctionId,
        hale_model::FunctionId,
        bool,
    )> = {
        let mut rows: BTreeMap<
            (hale_model::FunctionId, hale_model::FunctionId),
            bool,
        > = BTreeMap::new();
        for ((k, next), looped) in
            crate::callgraph::legacy_via_stdlib_contraction(
                &merged, &user_key,
            )
        {
            let e = rows
                .entry((fn_id[&fn_name(&k)], fn_id[&fn_name(&next)]))
                .or_insert(false);
            *e |= looped;
        }
        rows.into_iter().map(|((f, t), l)| (f, t, l)).collect()
    };

    // holes, canonically ordered.
    let mut hole_rows: Vec<Hole> = holes
        .into_iter()
        .map(|((at, kind, reason), (hides, pid))| Hole {
            at,
            kind,
            hides,
            reason,
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
    let hides_any = |fam: hale_model::RelationSet| {
        hole_rows.iter().any(|h| h.hides.intersects(fam))
    };
    let capabilities = Capabilities {
        exact_calls: !hides_any(hale_model::RelationSet::CALLS),
        exact_bus_endpoints: !hides_any(
            hale_model::RelationSet::PUBLISHES
                .union(hale_model::RelationSet::SUBSCRIBES),
        ),
        exact_key_filters: !hides_any(
            hale_model::RelationSet::KEY_FILTERS,
        ),
        exact_effects: !hides_any(hale_model::RelationSet::EFFECTS),
        ..Capabilities::default()
    };

    // entrypoint: the main locus, else "main".
    let entrypoint = ast
        .loci
        .iter()
        .find(|l| l.is_main)
        .map(|l| l.name.name.clone())
        .unwrap_or_else(|| "main".to_string());

    prov.records = records;
    let model = ApplicationModel {
        legacy: hale_model::LegacyProjection {
            topology_v1_fns: legacy_fns,
            topology_v1_calls_via_stdlib: legacy_via,
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
        "capabilities: calls={} bus={} keys={} effects={}\n",
        m.capabilities.exact_calls,
        m.capabilities.exact_bus_endpoints,
        m.capabilities.exact_key_filters,
        m.capabilities.exact_effects
    ));
    s
}
