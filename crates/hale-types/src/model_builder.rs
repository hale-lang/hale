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
    Block, BusMember, ElseBranch, Expr, GroupDecl, KeyFilter, Literal,
    LocusDecl as AstLocusDecl, LocusMember, Program, RecoveryModifier,
    RecoveryOp, ShedPolicy as AstShedPolicy, Stmt, TopDecl, TopicDecl,
    TypeExpr, UnmatchedPolicy,
};

use crate::alloc_summary::{self, Callee, EffectSiteKind, FnKey};
use crate::symbol::Bundle;

static BUILDS: AtomicU64 = AtomicU64::new(0);

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
    let fn_name = |k: &FnKey| -> String {
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

    // ---- entity tables (canonical order via BTree keys) ----
    // Loci.
    let mut locus_rows: BTreeMap<String, (bool, hale_syntax::Span)> =
        BTreeMap::new();
    for l in &ast.loci {
        locus_rows
            .insert(name(&l.name.name), (l.sealed, l.name.span));
    }
    let locus_id: BTreeMap<&String, LocusDeclId> = locus_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (k, LocusDeclId(i as u32)))
        .collect();

    // Functions: the summary's user keys (the fn sort).
    let mut fn_rows: BTreeMap<String, FnKey> = BTreeMap::new();
    for k in summary.fns.keys() {
        if user_key(k) {
            fn_rows.insert(fn_name(k), k.clone());
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

    // Topics + subjects + payloads.
    // Subject strings: a declared topic's WIRE subject; plus every
    // literal endpoint subject from the bus graph.
    let mut topic_decl_by_name: BTreeMap<String, &TopicDecl> =
        BTreeMap::new();
    for t in &ast.topics {
        topic_decl_by_name.insert(name(&t.name.name), t);
    }
    let wire_subjects = crate::topic_identity::topic_wire_subjects(
        &programs
            .iter()
            .flat_map(|p| p.items.iter().cloned())
            .collect::<Vec<_>>(),
    );
    let wire_of = |topic_display: &str| -> String {
        wire_subjects
            .get(topic_display)
            .cloned()
            .unwrap_or_else(|| topic_display.to_string())
    };

    let mut subject_set: BTreeSet<String> = BTreeSet::new();
    for tname in topic_decl_by_name.keys() {
        subject_set.insert(wire_of(tname));
    }
    for (subject, _) in &graph.subjects {
        let display = name(subject);
        if !topic_decl_by_name.contains_key(&display) {
            subject_set.insert(display);
        }
    }

    // Payload contracts: canonical shapes for declared topics, and
    // name-only contracts for literal-endpoint payload types.
    let all_items: Vec<TopDecl> = programs
        .iter()
        .flat_map(|p| p.items.iter().cloned())
        .collect();
    let mut payload_rows: BTreeMap<(String, u64), ()> = BTreeMap::new();
    let mut topic_payload: BTreeMap<String, (String, u64)> =
        BTreeMap::new();
    for (tname, t) in &topic_decl_by_name {
        let shape =
            crate::topic_identity::canonical_topic_shape(&all_items, t);
        let subject = wire_of(tname);
        let hash = crate::topic_identity::topic_shape_hash(
            &subject, &shape,
        );
        payload_rows.insert((shape.clone(), hash), ());
        topic_payload.insert(tname.clone(), (shape, hash));
    }
    // Literal endpoints: the payload TYPE NAME is the contract
    // identity we have (BusGraph resolves it); shape falls back to
    // the type name with a name-derived hash — honest and stable,
    // upgraded when Change 3 unifies shape rendering.
    let fnv = |s: &str| -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    };
    let mut endpoint_payload: BTreeMap<String, (String, u64)> =
        BTreeMap::new();
    for (subject, info) in &graph.subjects {
        let display = name(subject);
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
        let key = (ty.clone(), fnv(&ty));
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
        let mut need = |display: String| {
            if !topic_decl_by_name.contains_key(&display)
                && !endpoint_payload.contains_key(&display)
            {
                subject_set.insert(display.clone());
                payload_rows.insert(unresolved_payload.clone(), ());
                endpoint_payload
                    .insert(display, unresolved_payload.clone());
            }
        };
        for (k, fs) in &summary.fns {
            if !user_key(k) {
                continue;
            }
            for site in &fs.effect_sites {
                if let EffectSiteKind::Publish(Some(subj)) = &site.kind
                {
                    need(name(subj));
                }
            }
        }
        for l in &ast.loci {
            for m in &l.members {
                let LocusMember::Bus(bus) = m else { continue };
                for bm in &bus.members {
                    match bm {
                        BusMember::Subscribe { subject, .. }
                        | BusMember::Publish { subject, .. } => {
                            need(name(subject.canonical()));
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

    // Groups.
    let mut group_rows: BTreeMap<String, &GroupDecl> = BTreeMap::new();
    for g in &ast.groups {
        group_rows.insert(name(&g.name.name), g);
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
        type_rows.insert(name(n), *sp);
    }
    let type_id: BTreeMap<&String, TypeDeclId> = type_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (k, TypeDeclId(i as u32)))
        .collect();
    let mut iface_rows: BTreeMap<String, hale_syntax::Span> =
        BTreeMap::new();
    for (n, sp) in &ast.interfaces {
        iface_rows.insert(name(n), *sp);
    }
    let iface_id: BTreeMap<&String, InterfaceDeclId> = iface_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (k, InterfaceDeclId(i as u32)))
        .collect();
    let mut other_rows: BTreeMap<(String, DeclKind), hale_syntax::Span> =
        BTreeMap::new();
    for (kind, n, sp) in &ast.others {
        other_rows.insert((name(n), *kind), *sp);
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
        let mut site: u32 = 0;
        let mut dead_site: u32 = 0;
        for edge in &fs.calls {
            match &edge.callee {
                Callee::Resolved(next) => {
                    if !user_key(next) {
                        continue;
                    }
                    let to = fn_id[&fn_name(next)];
                    let dispatch = match &edge.via_interface {
                        Some(i) => DispatchKind::Interface {
                            interface: name(i),
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
                    site += 1;
                }
                Callee::Unresolved(n) => {
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
                            (from, dead_site),
                            (name(iface), n.clone(), pid),
                        );
                        dead_site += 1;
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
                    }
                }
            }
        }
    }
    // Through-stdlib contraction (same walk as the artifact's).
    let mut via_stdlib: BTreeMap<(FunctionId, FunctionId), bool> =
        BTreeMap::new();
    for (k, fs) in &merged.fns {
        if !user_key(k) {
            continue;
        }
        let from = fn_id[&fn_name(k)];
        let mut stack: Vec<(FnKey, bool)> = Vec::new();
        let mut seen: BTreeSet<FnKey> = BTreeSet::new();
        for edge in &fs.calls {
            if let Callee::Resolved(next) = &edge.callee {
                if !user_key(next) && seen.insert(next.clone()) {
                    stack.push((
                        next.clone(),
                        edge.loop_depth > 0 || edge.in_unbounded_loop,
                    ));
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
                let l2 =
                    lp || edge.loop_depth > 0 || edge.in_unbounded_loop;
                if user_key(next) {
                    let to = fn_id[&fn_name(next)];
                    let e =
                        via_stdlib.entry((from, to)).or_insert(false);
                    *e |= l2;
                } else if seen.insert(next.clone()) {
                    stack.push((next.clone(), l2));
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
        let contracted: Vec<((FunctionId, FunctionId), bool)> =
            via_stdlib.into_iter().collect();
        for ((from, to), looped) in contracted {
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
                (looped, looped, pid),
            );
        }
    }

    // publishes: effect sites at SITE grain, + computed-subject holes.
    let mut publishes: BTreeMap<
        (FunctionId, SubjectId, u32),
        (Option<TopicId>, PayloadContractId, Option<KeyDomain>, ProvenanceId),
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
                    let display = name(subj);
                    let pid = intern_span(&mut records, s.span);
                    let (declared, subject_str, payload, keyed) =
                        match topic_decl_by_name.get(&display) {
                            Some(t) => {
                                let shape_hash =
                                    topic_payload[&display].clone();
                                (
                                    Some(topic_id[&display]),
                                    wire_of(&display),
                                    payload_id[&shape_hash],
                                    t.keyed_by.as_ref().map(|f| {
                                        KeyDomain::AnyOfType(
                                            f.name.clone(),
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
                        (declared, payload, keyed, pid),
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
        let locus_display = name(&l.name.name);
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
                    ..
                } = bm
                else {
                    continue;
                };
                let display = name(subject.canonical());
                let handler_full =
                    format!("{}::{}", locus_display, handler.name);
                let Some(hid) = fn_id.get(&handler_full) else {
                    continue;
                };
                let (declared, subject_str, payload) =
                    match topic_decl_by_name.get(&display) {
                        Some(_) => (
                            Some(topic_id[&display]),
                            wire_of(&display),
                            payload_id
                                [&topic_payload[&display].clone()],
                        ),
                        None => (
                            None,
                            display.clone(),
                            payload_id[&endpoint_payload[&display]],
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
                    let parent = locus_id[&name(&l.name.name)];
                    let child_name = fd
                        .params
                        .first()
                        .map(|p| name(&te_name(&p.ty)))
                        .unwrap_or_else(|| "?".to_string());
                    let child = match locus_id.get(&child_name) {
                        Some(id) => SupervisedRef::Locus(*id),
                        None => SupervisedRef::External(child_name),
                    };
                    let err = fd
                        .params
                        .get(1)
                        .map(|p| name(&te_name(&p.ty)))
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
                        let display = name(mangled);
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
                let display = name(&m.display());
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
                            display: m.display(),
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
            let display = name(mangled);
            if let Some(r) = entity_of(&display) {
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
            display: n.clone(),
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
    for (n, key) in &fn_rows {
        let kind = match vmodel.phases.get(key) {
            Some(p) if p.hook => FunctionKind::Hook,
            Some(_) => FunctionKind::Method,
            None => FunctionKind::Free,
        };
        let pid = intern_synth(&mut records, "fn (summary key)");
        e.functions.push(Function {
            name: n.clone(),
            display: n.clone(),
            kind,
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
        let t = topic_decl_by_name[tname];
        let pid = intern_span(&mut records, t.name.span);
        e.topics.push(Topic {
            name: tname.clone(),
            subject: subject_id[&wire_of(tname)],
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
            may_be_empty: g.may_be_empty,
            provenance: pid,
        });
    }
    for (n, sp) in &type_rows {
        let pid = intern_span(&mut records, *sp);
        e.types.push(MTypeDecl {
            name: n.clone(),
            display: n.clone(),
            provenance: pid,
        });
    }
    for (n, sp) in &iface_rows {
        let pid = intern_span(&mut records, *sp);
        e.interfaces.push(InterfaceDecl {
            name: n.clone(),
            display: n.clone(),
            provenance: pid,
        });
    }
    for ((n, kind), sp) in &other_rows {
        let pid = intern_span(&mut records, *sp);
        e.declarations.push(Declaration {
            kind: *kind,
            name: n.clone(),
            display: n.clone(),
            provenance: pid,
        });
    }

    let mut r = Relations::default();
    // member_of + phase_of from the fn rows.
    for (n, key) in &fn_rows {
        if let Some(l) = &key.locus {
            let ld = name(l);
            if let Some(lid) = locus_id.get(&ld) {
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
    for ((f, s, site), (declared, payload, keyed, pid)) in &publishes {
        r.publishes.push(Publish {
            function: *f,
            subject: *s,
            declared_topic: *declared,
            payload: *payload,
            site: *site,
            key_domain: keyed.clone(),
            disposition: PublishDisposition::Default,
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
    let fn_name = |id: FunctionId| e.functions[id.index()].name.clone();
    let locus_name = |id: LocusDeclId| e.loci[id.index()].name.clone();
    let subject_pat = |id: SubjectId| e.subjects[id.index()].pattern.clone();
    let topic_name = |id: TopicId| e.topics[id.index()].name.clone();

    s.push_str(&format!("loci ({}):\n", e.loci.len()));
    for l in &e.loci {
        s.push_str(&format!(
            "  {}{}\n",
            l.name,
            if l.sealed { " @sealed" } else { "" }
        ));
    }
    s.push_str(&format!("functions ({}):\n", e.functions.len()));
    for f in &e.functions {
        s.push_str(&format!(
            "  {} [{}]{}\n",
            f.name,
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
            t.name,
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
                    g.name,
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
            .map(|t| t.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        e.interfaces
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        e.declarations
            .iter()
            .map(|d| format!("{:?}:{}", d.kind, d.name))
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
            e.groups[gs.group.index()].name,
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
