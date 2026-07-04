//! Symbol resolution: build the bundle-wide top-level scope.
//!
//! Phase 1: walk every top-level decl in every program and
//! produce a `TopScope` keyed by name. Each entry carries the
//! pre-resolved [`crate::symbol::TopSymbol`] form (params with
//! types resolved, bus subjects/payloads resolved, etc.).
//!
//! Resolution of a `TypeExpr → Ty` happens here too — but only
//! against primitive types and the bundle's own top-level
//! names. Type names not visible at this stage become
//! `Ty::Unknown`; full call-site / external-path resolution is
//! milestone 3.

use std::collections::BTreeMap;

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::symbol::*;
use crate::ty::Ty;

#[derive(Debug, Default)]
pub struct TopScope {
    pub symbols: BTreeMap<String, TopSymbol>,
}

impl TopScope {
    pub fn lookup(&self, name: &str) -> Option<&TopSymbol> {
        self.symbols.get(name)
    }
}

pub fn build_top_scope(bundle: &Bundle<'_>) -> (TopScope, Vec<Diag>) {
    let mut scope = TopScope::default();
    let mut diags = Vec::new();

    // First pass: register every top-level *type-like* name
    // (locus, type, perspective) so type expressions in a
    // second pass can resolve cross-file references.
    let mut known_names: BTreeMap<String, Span> = BTreeMap::new();
    for program in bundle.programs.values() {
        collect_type_names(&program.items, &mut known_names, &mut diags);
    }

    // v1.x-FORM-1 PR3b: pre-register form-specific stdlib type
    // names (e.g. `IndexError` for `@form(vec)`) so that
    // user-written `fn handle(err: IndexError)` resolves the
    // type during pass 2. Idempotent: if the user has already
    // declared `IndexError`, the user's declaration wins (the
    // duplicate-name path in `insert_name` won't fire because
    // we go through the known_names map directly here).
    if bundle_uses_form_machinery(bundle) {
        let zero = Span::new(0, 0);
        known_names
            .entry("IndexError".to_string())
            .or_insert(zero);
        known_names
            .entry("KeyError".to_string())
            .or_insert(zero);
        known_names
            .entry("EmptyError".to_string())
            .or_insert(zero);
    }
    // Phase 3 routing-keys v0.2 (2026-05-26): pre-register
    // BusUnmatchedKey when any topic declares `on_unmatched:
    // fail`. Same idempotent pattern as the form-stdlib names.
    if bundle_uses_fail_topics(bundle) {
        known_names
            .entry("BusUnmatchedKey".to_string())
            .or_insert(Span::new(0, 0));
    }

    // Pre-pass: build a name → ResolvedTopic table for every
    // declared topic, including parent chain + wire subject.
    // Loci that reference topics in their bus blocks resolve
    // through this table during the main register pass below, so
    // iteration order between locus and topic decls doesn't
    // matter. Diagnostics for unknown parents / cycles / dup
    // subjects also originate here.
    let mut topics_resolved: BTreeMap<String, ResolvedTopic> = BTreeMap::new();
    for program in bundle.programs.values() {
        collect_topic_decls(&program.items, &known_names, &mut topics_resolved, &mut diags);
    }
    finalize_topic_chain(&mut topics_resolved, &mut diags);

    // Second pass: resolve and emit full TopSymbol entries.
    for program in bundle.programs.values() {
        register_top_decls(
            &program.items,
            &known_names,
            &topics_resolved,
            &mut scope,
            &mut diags,
        );
    }

    // v1.x-FORM-1 PR3b: inject the form-specific stdlib type
    // structs (IndexError fields) into the scope so call sites
    // that use them resolve. Idempotent — user-declared
    // IndexError wins.
    if bundle_uses_form_machinery(bundle) {
        inject_form_stdlib_types(&mut scope);
    }
    // Phase 3 routing-keys v0.2 (2026-05-26): inject
    // BusUnmatchedKey for `on_unmatched: fail` topics whose
    // publishes use `or handler(err)` / `or fail <payload>`
    // dispositions. Gated to keep the name out of scope for
    // programs that don't use fail-policy topics. Idempotent:
    // user-declared BusUnmatchedKey wins.
    if bundle_uses_fail_topics(bundle) {
        inject_bus_unmatched_key_type(&mut scope);
    }
    // FUv0.8.2 #1 (2026-05-25): if the user has declared a
    // type whose name shadows a stdlib error type but with
    // a different shape AND the program actually uses a
    // stdlib path-call that needs that type, fire a type
    // error at the user's decl span pointing at the rename /
    // qualify workarounds — earlier than the codegen-time
    // `emit_*_error_alloc` site, with a real user-side span.
    //
    // Per-type usage gating avoids false-positives for users
    // who happen to name a type `ParseError` in a program
    // that never calls `std::str::parse_*`.
    let stdlib_usage = scan_stdlib_error_usage(bundle);
    check_stdlib_error_shadowing(&scope, &stdlib_usage, &mut diags);

    (scope, diags)
}

/// True when at least one locus in the bundle carries a
/// `@form(...)` annotation. Used to gate stdlib-type injection
/// so projects that don't use the form machinery don't get
/// `IndexError` / similar names spuriously in scope.
fn bundle_uses_form_machinery(bundle: &Bundle<'_>) -> bool {
    fn te_has_bounded(t: &TypeExpr) -> bool {
        match t {
            TypeExpr::Bounded { .. } => true,
            TypeExpr::Array { elem, .. } => te_has_bounded(elem),
            TypeExpr::Projection { inner, .. } => te_has_bounded(inner),
            TypeExpr::Tuple(parts, _) => {
                parts.iter().any(te_has_bounded)
            }
            _ => false,
        }
    }
    fn scan_items(items: &[TopDecl]) -> bool {
        items.iter().any(|item| match item {
            TopDecl::Locus(l) => {
                l.form.is_some()
                    || l.members.iter().any(|m| match m {
                        LocusMember::Params(p) => p
                            .params
                            .iter()
                            .any(|param| {
                                param
                                    .ty
                                    .as_ref()
                                    .is_some_and(te_has_bounded)
                            }),
                        _ => false,
                    })
            }
            // bounded[T; N] fields (2026-07-02) use the same
            // injected error types (CapacityError / IndexError).
            TopDecl::Type(td) => match &td.body {
                TypeDeclBody::Struct(fields) => {
                    fields.iter().any(|f| te_has_bounded(&f.ty))
                }
                _ => false,
            },
            TopDecl::Module(m) => scan_items(&m.items),
            _ => false,
        })
    }
    bundle.programs.values().any(|p| scan_items(&p.items))
}

/// True when at least one topic in the bundle declares
/// `on_unmatched: fail`. Used to gate BusUnmatchedKey injection
/// — keeps the stdlib name out of scope for programs that
/// don't use fail-policy topics.
fn bundle_uses_fail_topics(bundle: &Bundle<'_>) -> bool {
    fn scan_items(items: &[TopDecl]) -> bool {
        items.iter().any(|item| match item {
            TopDecl::Topic(t) => matches!(
                t.on_unmatched,
                Some(hale_syntax::ast::UnmatchedPolicy::Fail)
            ),
            TopDecl::Module(m) => scan_items(&m.items),
            _ => false,
        })
    }
    bundle.programs.values().any(|p| scan_items(&p.items))
}

/// Phase 3 routing-keys v0.2 (2026-05-26): synthesize the
/// `BusUnmatchedKey` stdlib type so `on_unmatched: fail` topic
/// publishes can carry err payloads through
/// `or handler(err)` / `or fail <payload>` dispositions.
/// Mirrors `inject_form_stdlib_types`'s idempotent pattern.
pub(crate) fn inject_bus_unmatched_key_type(scope: &mut TopScope) {
    let zero = Span::new(0, 0);
    if scope.symbols.contains_key("BusUnmatchedKey") {
        return;
    }
    scope.symbols.insert(
        "BusUnmatchedKey".to_string(),
        TopSymbol::Type(TypeInfo {
            name: "BusUnmatchedKey".to_string(),
            kind: TypeKind::Struct(vec![
                FieldInfo {
                    name: "subject".to_string(),
                    ty: Ty::Prim(PrimType::String),
                    has_default: false,
                    tag: None,
                    span: zero,
                },
                FieldInfo {
                    name: "key_lo".to_string(),
                    ty: Ty::Prim(PrimType::Int),
                    has_default: false,
                    tag: None,
                    span: zero,
                },
                FieldInfo {
                    name: "key_hi".to_string(),
                    ty: Ty::Prim(PrimType::Int),
                    has_default: false,
                    tag: None,
                    span: zero,
                },
            ]),
            span: zero,
        }),
    );
}

fn collect_type_names(
    items: &[TopDecl],
    known: &mut BTreeMap<String, Span>,
    diags: &mut Vec<Diag>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) => insert_name(known, &l.name, diags),
            TopDecl::Type(t) => insert_name(known, &t.name, diags),
            TopDecl::Perspective(p) => insert_name(known, &p.name, diags),
            TopDecl::Interface(i) => insert_name(known, &i.name, diags),
            TopDecl::Topic(t) => insert_name(known, &t.name, diags),
            TopDecl::RingLayout(r) => insert_name(known, &r.name, diags),
            TopDecl::Module(m) => collect_type_names(&m.items, known, diags),
            _ => {}
        }
    }
}

fn insert_name(
    known: &mut BTreeMap<String, Span>,
    ident: &Ident,
    diags: &mut Vec<Diag>,
) {
    if let Some(prev) = known.get(&ident.name) {
        diags.push(Diag::ty(
            ident.span,
            format!(
                "duplicate top-level name `{}` (previous declaration at {:?})",
                ident.name, prev
            ),
        ));
        return;
    }
    known.insert(ident.name.clone(), ident.span);
}

/// Pre-resolved topic data. Built before the main register pass
/// so locus `bus { subscribe T as h; }` and `bindings { T: ... }`
/// can resolve regardless of source order. `wire_subject` is
/// finalized post-collect by `finalize_topic_chain` (which walks
/// the parent chain and concatenates segments).
#[derive(Debug, Clone)]
pub(crate) struct ResolvedTopic {
    pub name: String,
    pub payload: Ty,
    pub parent: Option<String>,
    /// Own subject segment — explicit `subject: "..."` else a
    /// lowercased default of the topic name.
    pub subject: String,
    /// Materialized dot-path; `String::new()` until
    /// `finalize_topic_chain` runs (or if the topic is part of an
    /// unresolved cycle).
    pub wire_subject: String,
    pub span: Span,
}

/// Walk every `topic Foo : Parent { payload: T; subject: "..."; }`
/// decl in `items` and record the resolved payload + parent +
/// subject. Diagnostics emitted for missing-payload / dup-subject
/// among siblings are deferred to the typecheck pass; here we
/// only record what's syntactically present.
fn collect_topic_decls(
    items: &[TopDecl],
    known: &BTreeMap<String, Span>,
    topics: &mut BTreeMap<String, ResolvedTopic>,
    _diags: &mut Vec<Diag>,
) {
    for item in items {
        match item {
            TopDecl::Topic(t) => {
                let payload = resolve_type_expr(&t.payload, known);
                let subject = t
                    .subject
                    .clone()
                    .unwrap_or_else(|| default_subject_segment(&t.name.name));
                topics.insert(
                    t.name.name.clone(),
                    ResolvedTopic {
                        name: t.name.name.clone(),
                        payload,
                        parent: t.parent.as_ref().map(|p| p.name.clone()),
                        subject,
                        wire_subject: String::new(),
                        span: t.span,
                    },
                );
            }
            TopDecl::Module(m) => {
                collect_topic_decls(&m.items, known, topics, _diags);
            }
            _ => {}
        }
    }
}

/// Default wire subject segment when the user didn't write
/// `subject: "..."`. Verbatim topic name — preserves Phase 1
/// behavior where `topic Ticks` desugars to literal subject
/// "Ticks". Style guides can choose to be explicit
/// (`subject: "ticks"`).
fn default_subject_segment(name: &str) -> String {
    name.to_string()
}

/// Walk parent chains, detect cycles + missing parents, and
/// materialize each topic's `wire_subject` (dot-joined ancestor
/// subjects). Topics that hit a missing-parent or cycle keep
/// `wire_subject = ""` and trigger diagnostics; downstream code
/// treats an empty wire subject as "skip codegen-side wiring".
fn finalize_topic_chain(
    topics: &mut BTreeMap<String, ResolvedTopic>,
    diags: &mut Vec<Diag>,
) {
    // Snapshot keys so we can mutably index `topics` while looping.
    let names: Vec<String> = topics.keys().cloned().collect();
    let mut wire: BTreeMap<String, String> = BTreeMap::new();
    for name in &names {
        if wire.contains_key(name) {
            continue;
        }
        let mut chain: Vec<String> = Vec::new();
        let mut cur = name.clone();
        let mut bad = false;
        loop {
            if chain.contains(&cur) {
                let span = topics.get(&cur).map(|t| t.span).unwrap_or(Span::new(0, 0));
                diags.push(Diag::ty(
                    span,
                    format!("topic `{}` parent chain forms a cycle", cur),
                ));
                bad = true;
                break;
            }
            chain.push(cur.clone());
            let parent = match topics.get(&cur).and_then(|t| t.parent.clone()) {
                Some(p) => p,
                None => break,
            };
            if !topics.contains_key(&parent) {
                let span = topics.get(&cur).map(|t| t.span).unwrap_or(Span::new(0, 0));
                diags.push(Diag::ty(
                    span,
                    format!(
                        "topic `{}` declares unknown parent topic `{}`",
                        cur, parent
                    ),
                ));
                bad = true;
                break;
            }
            cur = parent;
        }
        if bad {
            for n in chain {
                wire.entry(n).or_insert_with(String::new);
            }
            continue;
        }
        // chain is leaf-to-root; reverse to root-to-leaf and join
        // each topic's own `subject` segment.
        chain.reverse();
        let segments: Vec<String> = chain
            .iter()
            .map(|n| topics[n].subject.clone())
            .collect();
        // Now record wire_subject for every prefix so siblings
        // sharing ancestors don't recompute.
        let mut acc: Vec<String> = Vec::new();
        for (i, seg) in segments.iter().enumerate() {
            acc.push(seg.clone());
            wire.entry(chain[i].clone())
                .or_insert_with(|| acc.join("."));
        }
    }
    for (n, w) in wire {
        if let Some(t) = topics.get_mut(&n) {
            t.wire_subject = w;
        }
    }

    // Duplicate-wire-subject check: two distinct topic names with
    // the same materialized subject would route ambiguously on a
    // path-shaped transport. Skip empty wire subjects (those are
    // already errored out above).
    let mut by_wire: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (n, t) in topics.iter() {
        if t.wire_subject.is_empty() {
            continue;
        }
        by_wire
            .entry(t.wire_subject.clone())
            .or_default()
            .push(n.clone());
    }
    for (w, owners) in by_wire {
        if owners.len() > 1 {
            for n in &owners {
                let span = topics[n].span;
                diags.push(Diag::ty(
                    span,
                    format!(
                        "topic `{}` shares wire subject `{}` with: {}",
                        n,
                        w,
                        owners.iter().filter(|x| *x != n).cloned().collect::<Vec<_>>().join(", ")
                    ),
                ));
            }
        }
    }
}

fn register_top_decls(
    items: &[TopDecl],
    known: &BTreeMap<String, Span>,
    topics: &BTreeMap<String, ResolvedTopic>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) => register_locus(l, known, topics, scope, diags),
            TopDecl::Type(t) => register_type(t, known, scope, diags),
            TopDecl::Perspective(p) => register_perspective(p, known, scope, diags),
            TopDecl::Const(c) => register_const(c, known, scope, diags),
            TopDecl::Fn(f) => register_fn(f, known, scope, diags),
            TopDecl::Module(m) => {
                register_top_decls(&m.items, known, topics, scope, diags);
            }
            TopDecl::Interface(i) => register_interface(i, known, scope, diags),
            TopDecl::Topic(t) => register_topic(t, topics, scope, diags),
            TopDecl::RingLayout(r) => register_ring_layout(r, scope, diags),
            TopDecl::Target(_) => {
                // FUv0.8.2 #7 (2026-05-25): target capability
                // declarations don't introduce a scope entry —
                // they're metadata for codegen / the v0.3
                // capability gate, not first-class symbols
                // users reference by name. Parser registered
                // them on `program.items`; downstream passes
                // walk that list when they need the target
                // info.
            }
        }
    }
}

/// Resolve a `BusSubject` to the (canonical_subject_string,
/// payload_ty) pair downstream code consumes. Literal subjects
/// take payload from the explicit `of type T` clause; topic
/// references look payload up in the topic-payload table built
/// during the pre-pass. Diagnostics fire for:
///   - topic-ref with no matching `topic` decl
///   - topic-ref with a stray `of type T` clause (forbidden;
///     the topic carries payload type)
///   - literal subject with no `of type T` clause (still required;
///     legacy form)
fn resolve_bus_subject(
    subject: &BusSubject,
    ty: Option<&TypeExpr>,
    known: &BTreeMap<String, Span>,
    topics: &BTreeMap<String, ResolvedTopic>,
    diags: &mut Vec<Diag>,
    ctx: &'static str,
) -> (String, Ty) {
    match subject {
        BusSubject::Literal { subject: s, .. } => {
            let payload = match ty {
                Some(te) => resolve_type_expr(te, known),
                None => Ty::Unknown,
            };
            (s.clone(), payload)
        }
        BusSubject::Topic(ident) => {
            if let Some(te) = ty {
                diags.push(Diag::ty(
                    te.span(),
                    format!(
                        "{} `{}` is a topic reference; `of type T` is forbidden \
                         (the topic carries the payload type)",
                        ctx, ident.name
                    ),
                ));
            }
            match topics.get(&ident.name) {
                Some(t) => (ident.name.clone(), t.payload.clone()),
                None => {
                    diags.push(Diag::ty(
                        ident.span,
                        format!(
                            "{} references unknown topic `{}` (no `topic {}` \
                             declaration in scope)",
                            ctx, ident.name, ident.name
                        ),
                    ));
                    (ident.name.clone(), Ty::Unknown)
                }
            }
        }
        BusSubject::QualifiedTopic(qn) => {
            // A7 (G16): cross-seed `subscribe alias::Foo as h;`.
            // The typechecker can't reach the imported topic decl
            // without seeing the merged + mangled program (cross-
            // seed visibility lives at the codegen pre-pass that
            // build_executable_with_imports runs after mangle).
            // For typecheck-only consumers, accept the path and
            // treat the payload as Unknown — the codegen-side
            // resolution will catch unresolved paths.
            if let Some(te) = ty {
                diags.push(Diag::ty(
                    te.span(),
                    format!(
                        "{} `{}` is a topic reference; `of type T` is forbidden \
                         (the topic carries the payload type)",
                        ctx,
                        qn.segments
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join("::")
                    ),
                ));
            }
            let leaf = qn
                .segments
                .last()
                .map(|s| s.name.clone())
                .unwrap_or_default();
            (leaf, Ty::Unknown)
        }
    }
}

fn register_topic(
    decl: &TopicDecl,
    topics: &BTreeMap<String, ResolvedTopic>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    // Pre-pass collected payload + parent + subject + wire_subject;
    // just lift it into a TopSymbol. Parent/cycle/dup-subject diags
    // already fired during finalize_topic_chain.
    let r = match topics.get(&decl.name.name) {
        Some(r) => r.clone(),
        None => return,
    };
    let info = crate::symbol::TopicInfo {
        name: r.name,
        payload: r.payload,
        parent: r.parent,
        subject: r.subject,
        wire_subject: r.wire_subject,
        keyed_by: decl.keyed_by.as_ref().map(|i| i.name.clone()),
        on_unmatched: decl.on_unmatched,
        span: decl.span,
    };
    register_symbol(
        scope,
        &decl.name.name,
        TopSymbol::Topic(info),
        decl.span,
        diags,
    );
}

/// shm-ring-interop Proposal B: register a `ring_layout Name` as a
/// resolvable symbol. The layout contract (offsets, reprs, framing) is
/// validated separately in `check`; here we just bind the name so a
/// `shm_ring(..., layout: Name)` reference can find it.
fn register_ring_layout(
    decl: &hale_syntax::ast::RingLayoutDecl,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let info = crate::symbol::RingLayoutInfo {
        name: decl.name.name.clone(),
        decl: decl.clone(),
        span: decl.span,
    };
    register_symbol(
        scope,
        &decl.name.name,
        TopSymbol::RingLayout(info),
        decl.span,
        diags,
    );
}

fn register_interface(
    decl: &hale_syntax::ast::InterfaceDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let methods = decl
        .methods
        .iter()
        .map(|m| crate::symbol::InterfaceMethodInfo {
            name: m.name.name.clone(),
            params: m
                .params
                .iter()
                .map(|p| (p.name.name.clone(), resolve_type_expr(&p.ty, known)))
                .collect(),
            ret: match &m.ret {
                Some(te) => resolve_type_expr(te, known),
                None => Ty::Unit,
            },
            span: m.span,
        })
        .collect();
    let info = crate::symbol::InterfaceInfo {
        name: decl.name.name.clone(),
        methods,
        span: decl.span,
    };
    register_symbol(
        scope,
        &decl.name.name,
        TopSymbol::Interface(info),
        decl.span,
        diags,
    );
}

fn register_locus(
    decl: &LocusDecl,
    known: &BTreeMap<String, Span>,
    topics: &BTreeMap<String, ResolvedTopic>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let mut params: Vec<ParamInfo> = Vec::new();
    let mut bus_publishes: Vec<BusPublishInfo> = Vec::new();
    let mut bus_subscribes: Vec<BusSubscribeInfo> = Vec::new();
    let mut accept_param: Option<(String, Ty)> = None;
    let mut mode_returns: BTreeMap<ModeKind, Ty> = BTreeMap::new();
    let mut annotations = Annotations::default();
    let mut contract_expose: Vec<ContractEntry> = Vec::new();
    let mut contract_consume: Vec<ContractEntry> = Vec::new();
    let mut methods: Vec<MethodInfo> = Vec::new();

    for ann in &decl.annotations {
        match ann {
            LocusAnnotation::Tier(n) => annotations.tier = Some(*n),
            LocusAnnotation::Projection(p) => annotations.projection = Some(*p),
        }
    }
    // F.31 (2026-05-23): per-locus schedule annotation gone.
    // Placement is read from main's `placement { }` block at
    // codegen time and applied per-instance. `annotations.schedule`
    // stays in the symbol shape for Phase 1 transitional purposes
    // and defaults to None — Phase 3 codegen reads placement
    // instead.
    let _ = &annotations;

    // v1.x-3: surface the "v1.x pending" rejection for the two
    // sub-modes we parse + typecheck but haven't shipped a runtime
    // for yet. Mirrors v1.x-4's surface-then-runtime split — the
    // user gets a clear "spelled correctly but not yet implemented"
    // diagnostic at typecheck rather than discovering it during
    // codegen. v1 ships fixed_cell + shared_slab.
    if let Some(ProjectionClass::Recognition(Some(params))) = annotations.projection {
        match params.sub_mode {
            RecognitionSubMode::Spillover => {
                diags.push(Diag::ty(
                    decl.name.span,
                    "recognition sub-mode `spillover` is parsed but not yet \
                     shipped — v1.x pending. Use `fixed_cell` for \
                     hard-bounded cells or `shared_slab` for a \
                     wholesale-freed bump arena."
                        .to_string(),
                ));
            }
            RecognitionSubMode::SummaryOnly => {
                diags.push(Diag::ty(
                    decl.name.span,
                    "recognition sub-mode `summary_only` is parsed but not \
                     yet shipped — v1.x pending. The \"no child arena \
                     allocation\" type-system rule isn't in tree yet. Use \
                     `fixed_cell` or `shared_slab` for v1."
                        .to_string(),
                ));
            }
            RecognitionSubMode::FixedCell | RecognitionSubMode::SharedSlab => {}
        }
    }

    for member in &decl.members {
        match member {
            LocusMember::Params(pb) => {
                for p in &pb.params {
                    let ty = match &p.ty {
                        Some(te) => resolve_type_expr(te, known),
                        None => match &p.init {
                            ParamInit::Value(e) => infer_literal_ty(e),
                            ParamInit::Inferred => Ty::Unknown,
                        },
                    };
                    let has_default = matches!(p.init, ParamInit::Value(_));
                    params.push(ParamInfo {
                        name: p.name.name.clone(),
                        ty,
                        has_default,
                        span: p.span,
                    });
                }
            }
            LocusMember::Bus(bb) => {
                for bm in &bb.members {
                    match bm {
                        BusMember::Subscribe { subject, handler, ty, span, .. } => {
                            let (subject_str, payload) = resolve_bus_subject(
                                subject, ty.as_ref(), known, topics, diags, "subscribe",
                            );
                            bus_subscribes.push(BusSubscribeInfo {
                                subject: subject_str,
                                handler: handler.name.clone(),
                                payload,
                                span: *span,
                            });
                        }
                        BusMember::Publish { subject, ty, span, .. } => {
                            let (subject_str, payload) = resolve_bus_subject(
                                subject, ty.as_ref(), known, topics, diags, "publish",
                            );
                            bus_publishes.push(BusPublishInfo {
                                subject: subject_str,
                                payload,
                                span: *span,
                            });
                        }
                    }
                }
            }
            LocusMember::Lifecycle(lc) if matches!(lc.kind, LifecycleKind::Accept) => {
                if let Some(p) = lc.params.first() {
                    let ty = resolve_type_expr(&p.ty, known);
                    accept_param = Some((p.name.name.clone(), ty));
                }
            }
            LocusMember::Mode(md) => {
                let ret = match &md.ret {
                    Some(te) => resolve_type_expr(te, known),
                    None => Ty::Unit,
                };
                mode_returns.insert(md.kind, ret.clone());
                let mname = match md.kind {
                    ModeKind::Bulk => "bulk",
                    ModeKind::Harmonic => "harmonic",
                    ModeKind::Resolution => "resolution",
                };
                methods.push(MethodInfo {
                    name: mname.to_string(),
                    params: md
                        .params
                        .iter()
                        .map(|p| resolve_type_expr(&p.ty, known))
                        .collect(),
                    ret,
                    fallible: None,
                });
            }
            LocusMember::Fn(f) => {
                let ret = match &f.ret {
                    Some(te) => resolve_type_expr(te, known),
                    None => Ty::Unit,
                };
                let fallible = f
                    .fallible
                    .as_ref()
                    .map(|te| resolve_type_expr(te, known));
                methods.push(MethodInfo {
                    name: f.name.name.clone(),
                    params: f
                        .params
                        .iter()
                        .map(|p| resolve_type_expr(&p.ty, known))
                        .collect(),
                    ret,
                    fallible,
                });
            }
            LocusMember::Contract(cb) => {
                if let ContractKind::Members(members) = &cb.kind {
                    for m in members {
                        let ContractName::Named(name) = &m.name else {
                            continue;
                        };
                        let Some(te) = &m.ty else {
                            continue;
                        };
                        let entry = ContractEntry {
                            name: name.name.clone(),
                            ty: resolve_type_expr(te, known),
                            span: m.span,
                        };
                        match m.direction {
                            ContractDirection::Expose => contract_expose.push(entry),
                            ContractDirection::Consume => contract_consume.push(entry),
                        }
                    }
                }
            }
            _ => {} // closure / failure / fn / const / type members
                    // are not yet hoisted into the locus's external surface in
                    // milestone 2.
        }
    }

    // F.22: collect capacity slot names so the typechecker can
    // recognize `self.<slot>` as a slot reference rather than a
    // missing field. Kinds + element types stay codegen-side;
    // typecheck only needs the names for the
    // self-field-vs-slot distinction.
    let mut capacity_slot_names: Vec<String> = Vec::new();
    for member in &decl.members {
        if let LocusMember::Capacity(cb) = member {
            for slot in &cb.slots {
                capacity_slot_names.push(slot.name.name.clone());
            }
        }
    }

    // v1.x-VIOLATE (F.27): collect closure declarations so the
    // typechecker can resolve `violate NAME;` against the
    // enclosing locus and enforce the `epoch inline` gate.
    let mut closures: Vec<ClosureSymInfo> = Vec::new();
    for member in &decl.members {
        if let LocusMember::Closure(cd) = member {
            let is_inline = cd.clauses.iter().any(|c| {
                matches!(c, ClosureClause::Epoch(EpochSpec::Inline))
            });
            let captures = cd
                .clauses
                .iter()
                .flat_map(|c| match c {
                    ClosureClause::Captures(names) => {
                        names.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                })
                .collect();
            closures.push(ClosureSymInfo {
                name: cd.name.name.clone(),
                is_inline,
                captures,
                span: cd.span,
            });
        }
    }

    // v1.x-FORM-1 PR3b: synthesize standard methods for
    // form-annotated loci. Method synthesis is form-specific:
    // @form(vec) emits push/get/pop/len/is_empty over the cell
    // type T derived from the heap slot. Shape-verification
    // (PR3a) has already filtered out invalid form/capacity
    // combinations and emitted diagnostics — here we soldier
    // on best-effort even when the shape is invalid (returning
    // Ty::Unknown for T) so downstream typechecks don't
    // cascade additional errors past the form-shape diag.
    if let Some(form) = &decl.form {
        match form.name.name.as_str() {
            "vec" => {
                let cell_ty = form_vec_cell_ty(decl, known);
                synthesize_form_vec_methods(&mut methods, &cell_ty);
            }
            "hashmap" => {
                let (value_ty, key_ty) = form_hashmap_value_and_key_ty(decl, known, scope);
                synthesize_form_hashmap_methods(&mut methods, &value_ty, &key_ty);
            }
            "ring_buffer" => {
                let cell_ty = form_ring_buffer_cell_ty(decl, known);
                synthesize_form_ring_buffer_methods(&mut methods, &cell_ty);
            }
            "lru_cache" => {
                // Same keyed shape as @form(hashmap): value struct S +
                // key K from the `indexed_by` field. lru_cache pairs
                // that key surface with a fixed `cap = N` (the only
                // form needing BOTH), enforced in check.rs.
                let (value_ty, key_ty) = form_hashmap_value_and_key_ty(decl, known, scope);
                synthesize_form_lru_cache_methods(&mut methods, &value_ty, &key_ty);
            }
            _ => {}
        }
    }

    let info = LocusInfo {
        name: decl.name.name.clone(),
        params,
        bus_publishes,
        bus_subscribes,
        accept_param,
        mode_returns,
        annotations,
        contract_expose,
        contract_consume,
        methods,
        capacity_slot_names,
        closures,
        span: decl.span,
    };

    register_symbol(scope, &decl.name.name, TopSymbol::Locus(info), decl.span, diags);
}

fn register_type(
    decl: &TypeDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let kind = match &decl.body {
        TypeDeclBody::Alias(te) => TypeKind::Alias(resolve_type_expr(te, known)),
        TypeDeclBody::Struct(fields) => {
            let infos: Vec<FieldInfo> = fields
                .iter()
                .map(|f| FieldInfo {
                    name: f.name.name.clone(),
                    ty: resolve_type_expr(&f.ty, known),
                    has_default: f.default.is_some(),
                    tag: f.tag.clone(),
                    span: f.span,
                })
                .collect();
            TypeKind::Struct(infos)
        }
        TypeDeclBody::Enum(variants) => {
            let infos: Vec<VariantInfo> = variants
                .iter()
                .map(|v| VariantInfo {
                    name: v.name.name.clone(),
                    fields: v.fields.iter().map(|t| resolve_type_expr(t, known)).collect(),
                    span: v.span,
                })
                .collect();
            TypeKind::Enum(infos)
        }
    };
    let info = TypeInfo {
        name: decl.name.name.clone(),
        kind,
        span: decl.span,
    };
    register_symbol(scope, &decl.name.name, TopSymbol::Type(info), decl.span, diags);
}

fn register_perspective(
    decl: &PerspectiveDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let mut params = Vec::new();
    let mut serialize_as = None;
    let mut methods: Vec<MethodInfo> = Vec::new();
    for member in &decl.members {
        match member {
            PerspectiveMember::Params(pb) => {
                for p in &pb.params {
                    let ty = match &p.ty {
                        Some(te) => resolve_type_expr(te, known),
                        None => match &p.init {
                            ParamInit::Value(e) => infer_literal_ty(e),
                            ParamInit::Inferred => Ty::Unknown,
                        },
                    };
                    params.push(ParamInfo {
                        name: p.name.name.clone(),
                        ty,
                        has_default: matches!(p.init, ParamInit::Value(_)),
                        span: p.span,
                    });
                }
            }
            PerspectiveMember::SerializeAs(te) => {
                serialize_as = Some(resolve_type_expr(te, known));
            }
            PerspectiveMember::Fn(f) => {
                let ret = match &f.ret {
                    Some(te) => resolve_type_expr(te, known),
                    None => Ty::Unit,
                };
                let fallible = f
                    .fallible
                    .as_ref()
                    .map(|te| resolve_type_expr(te, known));
                methods.push(MethodInfo {
                    name: f.name.name.clone(),
                    params: f
                        .params
                        .iter()
                        .map(|p| resolve_type_expr(&p.ty, known))
                        .collect(),
                    ret,
                    fallible,
                });
            }
            PerspectiveMember::StableWhen(_) => {
                // stable_when is a built-in method on every
                // perspective: `p.is_stable() -> Bool`.
                methods.push(MethodInfo {
                    name: "is_stable".to_string(),
                    params: Vec::new(),
                    ret: Ty::Prim(PrimType::Bool),
                    fallible: None,
                });
            }
        }
    }
    let info = PerspectiveInfo {
        name: decl.name.name.clone(),
        params,
        serialize_as,
        methods,
        span: decl.span,
    };
    register_symbol(
        scope,
        &decl.name.name,
        TopSymbol::Perspective(info),
        decl.span,
        diags,
    );
}

fn register_const(
    decl: &ConstDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let ty = resolve_type_expr(&decl.ty, known);
    let info = ConstInfo {
        name: decl.name.name.clone(),
        ty,
        span: decl.span,
    };
    register_symbol(scope, &decl.name.name, TopSymbol::Const(info), decl.span, diags);
}

fn register_fn(
    decl: &FnDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let params = decl
        .params
        .iter()
        .map(|p| (p.name.name.clone(), resolve_type_expr(&p.ty, known)))
        .collect();
    let ret = match &decl.ret {
        Some(te) => resolve_type_expr(te, known),
        None => Ty::Unit,
    };
    let fallible = decl
        .fallible
        .as_ref()
        .map(|te| resolve_type_expr(te, known));
    let sig = FnSig {
        name: decl.name.name.clone(),
        params,
        ret,
        fallible,
        span: decl.span,
    };
    register_symbol(scope, &decl.name.name, TopSymbol::Fn(sig), decl.span, diags);
}

fn register_symbol(
    scope: &mut TopScope,
    name: &str,
    sym: TopSymbol,
    span: Span,
    diags: &mut Vec<Diag>,
) {
    if scope.symbols.contains_key(name) {
        // Duplicate already reported by collect_type_names for
        // type-like names; fns/consts get caught here.
        if let TopSymbol::Fn(_) | TopSymbol::Const(_) = &sym {
            diags.push(Diag::ty(
                span,
                format!("duplicate top-level name `{}`", name),
            ));
        }
        return;
    }
    scope.symbols.insert(name.to_string(), sym);
}

/// Resolve a syntactic [`TypeExpr`] to a [`Ty`], using the
/// bundle-wide set of known type-like names. Names not in
/// `known` and not primitive resolve to [`Ty::Unknown`].
/// M3 stage 3 tranche 2: mirror of codegen's m61
/// `type_expr_mangle_token` — one token per generic arg.
fn type_expr_mangle_token(
    t: &TypeExpr,
    known: &BTreeMap<String, Span>,
) -> Option<String> {
    match t {
        TypeExpr::Primitive(p, _) => match p {
            PrimType::Int => Some("Int".into()),
            PrimType::Float => Some("Float".into()),
            PrimType::Bool => Some("Bool".into()),
            PrimType::String => Some("String".into()),
            PrimType::Duration => Some("Duration".into()),
            PrimType::Decimal => Some("Decimal".into()),
            PrimType::Time => Some("Time".into()),
            _ => None,
        },
        TypeExpr::Named { path, generic_args, .. }
            if path.segments.len() == 1 =>
        {
            let base = &path.segments[0].name;
            if generic_args.is_empty() {
                if known.contains_key(base) {
                    Some(base.clone())
                } else {
                    None
                }
            } else {
                let mut toks: Vec<String> = Vec::new();
                for a in generic_args {
                    toks.push(type_expr_mangle_token(a, known)?);
                }
                Some(format!("{}_{}", base, toks.join("_")))
            }
        }
        _ => None,
    }
}

pub fn resolve_type_expr(te: &TypeExpr, known: &BTreeMap<String, Span>) -> Ty {
    match te {
        TypeExpr::Primitive(p, _) => Ty::Prim(*p),
        TypeExpr::Named { path, generic_args, .. } => {
            if path.segments.len() == 1 {
                let name = &path.segments[0].name;
                // M3 stage 3 tranche 2 (2026-07-02): a generic
                // instantiation type-expr (`Box<Int>`) resolves to
                // its MANGLED monomorph name (`Box_Int`) — the same
                // name codegen synthesizes and the same Ty a
                // `Box_Int { ... }` literal produces, so the two
                // unify. Unknown when any arg token can't be named
                // (permissive, as before).
                if !generic_args.is_empty() {
                    let mut toks: Vec<String> = Vec::new();
                    for a in generic_args {
                        match type_expr_mangle_token(a, known) {
                            Some(t) => toks.push(t),
                            None => return Ty::Unknown,
                        }
                    }
                    return Ty::Named(format!(
                        "{}_{}",
                        name,
                        toks.join("_")
                    ));
                }
                if known.contains_key(name) {
                    Ty::Named(name.clone())
                } else {
                    Ty::Unknown
                }
            } else {
                // qualified path -> external (stdlib, module)
                Ty::Unknown
            }
        }
        TypeExpr::Projection { class, inner, .. } => {
            Ty::Projection(*class, Box::new(resolve_type_expr(inner, known)))
        }
        TypeExpr::Bounded { elem, cap, .. } => {
            Ty::Bounded(Box::new(resolve_type_expr(elem, known)), *cap)
        }
        TypeExpr::Array { elem, size, .. } => {
            let n = match size {
                Some(Expr::Literal(Literal::Int(n), _)) if *n >= 0 => Some(*n as u64),
                _ => None,
            };
            Ty::Array(Box::new(resolve_type_expr(elem, known)), n)
        }
        TypeExpr::Tuple(parts, _) => {
            // G2 follow-up: `()` parses as TypeExpr::Tuple([], _).
            // Normalize the empty case to Ty::Unit so downstream
            // checks (e.g. `or discard` requires success type to
            // be Unit) treat `-> () fallible(E)` and bare
            // `fallible(E)` interchangeably, matching the codegen
            // ABI which already does this normalization at
            // signature-lowering time (codegen.rs ~9948).
            if parts.is_empty() {
                Ty::Unit
            } else {
                Ty::Tuple(parts.iter().map(|t| resolve_type_expr(t, known)).collect())
            }
        }
        TypeExpr::Function { params, ret, .. } => {
            let p = params.iter().map(|t| resolve_type_expr(t, known)).collect();
            let r = match ret {
                Some(te) => resolve_type_expr(te, known),
                None => Ty::Unit,
            };
            Ty::Function {
                params: p,
                ret: Box::new(r),
            }
        }
    }
}

/// v1.x-FORM-1 PR3b: derive the cell type T for a `@form(vec)`
/// locus from its (single) heap slot. Shape verification
/// happens in `check.rs`; this function is best-effort and
/// returns `Ty::Unknown` when the shape is malformed so
/// downstream typechecks don't cascade.
fn form_vec_cell_ty(decl: &LocusDecl, known: &BTreeMap<String, Span>) -> Ty {
    for member in &decl.members {
        if let LocusMember::Capacity(cb) = member {
            if let Some(slot) = cb.slots.first() {
                return resolve_type_expr(&slot.elem_ty, known);
            }
        }
    }
    Ty::Unknown
}

/// v1.x-FORM-4 PR3: extract the value type S (= cell type) and
/// key type K (= type of the indexed-by field on the cell
/// struct) from a `@form(hashmap)` locus. Returns
/// `(Ty::Unknown, Ty::Unknown)` when the shape is invalid —
/// shape verification (PR2) has already reported diagnostics in
/// that case, so we don't double-report; method synthesis just
/// falls back to Unknown so downstream typechecks don't cascade.
fn form_hashmap_value_and_key_ty(
    decl: &LocusDecl,
    known: &BTreeMap<String, Span>,
    scope: &TopScope,
) -> (Ty, Ty) {
    let unknown = (Ty::Unknown, Ty::Unknown);
    let cb = match decl.members.iter().find_map(|m| match m {
        LocusMember::Capacity(cb) => Some(cb),
        _ => None,
    }) {
        Some(cb) => cb,
        None => return unknown,
    };
    let slot = match cb.slots.first() {
        Some(s) => s,
        None => return unknown,
    };
    let value_ty = resolve_type_expr(&slot.elem_ty, known);
    let field_ident = match &slot.indexed_by {
        Some(i) => i,
        None => return (value_ty, Ty::Unknown),
    };
    let cell_name = match &value_ty {
        Ty::Named(n) => n.clone(),
        _ => return (value_ty, Ty::Unknown),
    };
    let key_ty = match scope.symbols.get(&cell_name) {
        Some(TopSymbol::Type(info)) => match &info.kind {
            TypeKind::Struct(fields) => {
                match fields.iter().find(|f| f.name == field_ident.name) {
                    Some(f) => f.ty.clone(),
                    None => Ty::Unknown,
                }
            }
            _ => Ty::Unknown,
        },
        _ => Ty::Unknown,
    };
    (value_ty, key_ty)
}

/// v1.x-FORM-1 PR3b: synthesize the standard `@form(vec)`
/// method set over cell type T. Method signatures match
/// `spec/forms.md`:
///   `push(x: T) -> ()`                          (infallible)
///   `get(i: Int) -> T fallible(IndexError)`
///   `pop() -> T fallible(IndexError)`
///   `len() -> Int`                              (infallible)
///   `is_empty() -> Bool`                        (infallible)
///
/// `IndexError` is a synthesized stdlib type; the resolver
/// injects it into the top scope when the first form-locus
/// is registered (see `inject_form_stdlib_types`).
fn synthesize_form_vec_methods(methods: &mut Vec<MethodInfo>, cell_ty: &Ty) {
    let index_err = Ty::Named("IndexError".to_string());
    methods.push(MethodInfo {
        name: "push".to_string(),
        params: vec![cell_ty.clone()],
        ret: Ty::Unit,
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "get".to_string(),
        params: vec![Ty::Prim(PrimType::Int)],
        ret: cell_ty.clone(),
        fallible: Some(index_err.clone()),
    });
    methods.push(MethodInfo {
        name: "set".to_string(),
        params: vec![Ty::Prim(PrimType::Int), cell_ty.clone()],
        ret: Ty::Unit,
        fallible: Some(index_err.clone()),
    });
    methods.push(MethodInfo {
        name: "pop".to_string(),
        params: Vec::new(),
        ret: cell_ty.clone(),
        fallible: Some(index_err),
    });
    methods.push(MethodInfo {
        name: "len".to_string(),
        params: Vec::new(),
        ret: Ty::Prim(PrimType::Int),
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "is_empty".to_string(),
        params: Vec::new(),
        ret: Ty::Prim(PrimType::Bool),
        fallible: None,
    });
    // Sort family. `sort()` is primitive-only at v1 (codegen
    // rejects struct cells with a clear diagnostic pointing at
    // `sort_by`). `sort_by` / `sort_desc_by` take a user
    // comparator `fn(T, T) -> Bool` meaning "a comes before b";
    // sort_desc_by flips it. All in-place, no return value.
    let cmp_ty = Ty::Function {
        params: vec![cell_ty.clone(), cell_ty.clone()],
        ret: Box::new(Ty::Prim(PrimType::Bool)),
    };
    methods.push(MethodInfo {
        name: "sort".to_string(),
        params: Vec::new(),
        ret: Ty::Unit,
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "sort_by".to_string(),
        params: vec![cmp_ty.clone()],
        ret: Ty::Unit,
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "sort_desc_by".to_string(),
        params: vec![cmp_ty],
        ret: Ty::Unit,
        fallible: None,
    });
}

/// v1.x-FORM-4 PR3: synthesize the standard `@form(hashmap)`
/// method set over value type S (= cell struct) and key type K
/// (= type of the indexed-by field). Method signatures match
/// `spec/forms.md`:
///   `get(key: K) -> S fallible(KeyError)`
///   `set(value: S) -> ()`                       (infallible; insert/replace)
///   `has(key: K) -> Bool`                       (infallible)
///   `remove(key: K) -> () fallible(KeyError)`
///   `len() -> Int`                              (infallible)
///   `is_empty() -> Bool`                        (infallible)
///
/// `KeyError` is a synthesized stdlib type injected by
/// `inject_form_stdlib_types` alongside `IndexError`.
///
/// The key-by-field intrusive shape (the cell carries its own
/// key as one of its fields) means `set(value: S)` takes the
/// whole struct rather than a `(K, V)` pair — the substrate
/// extracts the key from the value at insertion time.
fn synthesize_form_hashmap_methods(
    methods: &mut Vec<MethodInfo>,
    value_ty: &Ty,
    key_ty: &Ty,
) {
    let key_err = Ty::Named("KeyError".to_string());
    methods.push(MethodInfo {
        name: "get".to_string(),
        params: vec![key_ty.clone()],
        ret: value_ty.clone(),
        fallible: Some(key_err.clone()),
    });
    methods.push(MethodInfo {
        name: "set".to_string(),
        params: vec![value_ty.clone()],
        ret: Ty::Unit,
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "has".to_string(),
        params: vec![key_ty.clone()],
        ret: Ty::Prim(PrimType::Bool),
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "remove".to_string(),
        params: vec![key_ty.clone()],
        ret: Ty::Unit,
        fallible: Some(key_err),
    });
    methods.push(MethodInfo {
        name: "len".to_string(),
        params: Vec::new(),
        ret: Ty::Prim(PrimType::Int),
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "is_empty".to_string(),
        params: Vec::new(),
        ret: Ty::Prim(PrimType::Bool),
        fallible: None,
    });
    // 2026-05-16 iteration + counter sugar. key_at / entry_at
    // unlock the "iterate the hashmap" pattern without a
    // parallel keys vec; bump collapses the has/get/set
    // increment-or-init dance into one method call. See
    // experiments/token-efficiency/handoffs/library-dev-2026-05-16.md
    // for the wordfreq-corpus reinvention frequency that
    // motivated each.
    let idx_err = Ty::Named("IndexError".to_string());
    methods.push(MethodInfo {
        name: "key_at".to_string(),
        params: vec![Ty::Prim(PrimType::Int)],
        ret: key_ty.clone(),
        fallible: Some(idx_err.clone()),
    });
    methods.push(MethodInfo {
        name: "entry_at".to_string(),
        params: vec![Ty::Prim(PrimType::Int)],
        ret: value_ty.clone(),
        fallible: Some(idx_err),
    });
    methods.push(MethodInfo {
        name: "bump".to_string(),
        params: vec![key_ty.clone()],
        ret: Ty::Unit,
        fallible: None,
    });
}

/// v1.x-FORM-5: extract cell type T from a `@form(ring_buffer)`
/// locus. Same shape as `form_vec_cell_ty` — the cell type is
/// the (single) pool slot's `elem_ty`. Returns `Ty::Unknown` if
/// the shape is invalid (shape diags already emitted).
fn form_ring_buffer_cell_ty(
    decl: &LocusDecl,
    known: &BTreeMap<String, Span>,
) -> Ty {
    for member in &decl.members {
        if let LocusMember::Capacity(cb) = member {
            if let Some(slot) = cb.slots.first() {
                return resolve_type_expr(&slot.elem_ty, known);
            }
        }
    }
    Ty::Unknown
}

/// v1.x-FORM-5: synthesize the standard `@form(ring_buffer)`
/// method set over cell type T. Per `spec/forms.md`:
///   `push(x: T) -> Bool`                         (infallible; false = full)
///   `pop() -> T fallible(EmptyError)`
///   `len() -> Int`                               (infallible)
///   `is_full() -> Bool`                          (infallible)
///
/// `push` returns a Bool indicating success — the buffer is
/// fixed-capacity and may refuse a push when full. Callers
/// decide drop vs backpressure based on the result.
/// `pop` is the fallible counterpart: `EmptyError` payload
/// when the buffer is empty.
fn synthesize_form_ring_buffer_methods(
    methods: &mut Vec<MethodInfo>,
    cell_ty: &Ty,
) {
    let empty_err = Ty::Named("EmptyError".to_string());
    methods.push(MethodInfo {
        name: "push".to_string(),
        params: vec![cell_ty.clone()],
        ret: Ty::Prim(PrimType::Bool),
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "pop".to_string(),
        params: Vec::new(),
        ret: cell_ty.clone(),
        fallible: Some(empty_err),
    });
    methods.push(MethodInfo {
        name: "len".to_string(),
        params: Vec::new(),
        ret: Ty::Prim(PrimType::Int),
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "is_full".to_string(),
        params: Vec::new(),
        ret: Ty::Prim(PrimType::Bool),
        fallible: None,
    });
}

/// v1.x-FORM-6: synthesize the `@form(lru_cache)` method set over
/// value type S (= cell struct) and key type K (= type of the
/// `indexed_by` field), matching `spec/forms.md`:
///   `put(x: S) -> ()`                 (infallible; silent LRU evict)
///   `get(k: K) -> S fallible(KeyError)` (lookup + recency touch)
///   `contains(k: K) -> Bool`          (membership, NO recency touch)
///   `len() -> Int`                    (infallible; current count ≤ cap)
///
/// lru_cache is the "cap-bounded, never-flagged" keyed form: like
/// `@form(hashmap)` it is intrusively keyed (the cell carries its
/// own key), so `put` takes the whole struct and the substrate
/// extracts the key at insertion time. Unlike hashmap it never
/// grows — inserting over `cap` silently evicts the least-recently-
/// used entry. `KeyError` (shared with hashmap) is the `get` miss
/// payload; `put` is infallible (eviction is silent, matching the
/// "never flagged / bounded" contract).
fn synthesize_form_lru_cache_methods(
    methods: &mut Vec<MethodInfo>,
    value_ty: &Ty,
    key_ty: &Ty,
) {
    let key_err = Ty::Named("KeyError".to_string());
    methods.push(MethodInfo {
        name: "put".to_string(),
        params: vec![value_ty.clone()],
        ret: Ty::Unit,
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "get".to_string(),
        params: vec![key_ty.clone()],
        ret: value_ty.clone(),
        fallible: Some(key_err),
    });
    methods.push(MethodInfo {
        name: "contains".to_string(),
        params: vec![key_ty.clone()],
        ret: Ty::Prim(PrimType::Bool),
        fallible: None,
    });
    methods.push(MethodInfo {
        name: "len".to_string(),
        params: Vec::new(),
        ret: Ty::Prim(PrimType::Int),
        fallible: None,
    });
}

/// v1.x-FORM-1 PR3b: inject form-specific stdlib types into the
/// top scope so synthesized method signatures' payload types
/// resolve. v1 injects `IndexError` (used by `@form(vec)`)
/// and `KeyError` (used by `@form(hashmap)`); future forms
/// will inject their own payload types here.
///
/// Idempotent per name: if a name already exists in the scope
/// (declared by user code or a stdlib `.hl` file), that
/// injection is a no-op. Keeps the form machinery non-breaking
/// for projects that already shipped their own error shapes.
pub(crate) fn inject_form_stdlib_types(scope: &mut TopScope) {
    let zero = Span::new(0, 0);
    // bounded[T; N] (2026-07-02): push-at-cap payload.
    if !scope.symbols.contains_key("CapacityError") {
        scope.symbols.insert(
            "CapacityError".to_string(),
            TopSymbol::Type(TypeInfo {
                name: "CapacityError".to_string(),
                kind: TypeKind::Struct(vec![
                    FieldInfo {
                        name: "cap".to_string(),
                        ty: Ty::Prim(PrimType::Int),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                    FieldInfo {
                        name: "count".to_string(),
                        ty: Ty::Prim(PrimType::Int),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                ]),
                span: zero,
            }),
        );
    }
    if !scope.symbols.contains_key("IndexError") {
        scope.symbols.insert(
            "IndexError".to_string(),
            TopSymbol::Type(TypeInfo {
                name: "IndexError".to_string(),
                kind: TypeKind::Struct(vec![
                    FieldInfo {
                        name: "kind".to_string(),
                        ty: Ty::Prim(PrimType::String),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                    FieldInfo {
                        name: "index".to_string(),
                        ty: Ty::Prim(PrimType::Int),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                    FieldInfo {
                        name: "len".to_string(),
                        ty: Ty::Prim(PrimType::Int),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                ]),
                span: zero,
            }),
        );
    }
    // v1.x-FORM-4: KeyError for @form(hashmap) get/remove
    // fallible methods. Minimal shape — just a kind tag at v1.
    // The key itself isn't carried because the key type K
    // varies per hashmap; carrying it would require a generic
    // KeyError<K> which v1 doesn't have. Users wanting key
    // context construct it via `or <fallback>` substitution:
    //   let v = reg.get("foo") or Default { ... };
    if !scope.symbols.contains_key("KeyError") {
        scope.symbols.insert(
            "KeyError".to_string(),
            TopSymbol::Type(TypeInfo {
                name: "KeyError".to_string(),
                kind: TypeKind::Struct(vec![FieldInfo {
                    name: "kind".to_string(),
                    ty: Ty::Prim(PrimType::String),
                    has_default: false,
                    tag: None,
                    span: zero,
                }]),
                span: zero,
            }),
        );
    }
    // v1.x-FORM-5: EmptyError for @form(ring_buffer)'s pop()
    // fallible. Same minimal-shape rationale as KeyError —
    // a single `kind` tag is enough at v1; richer context can
    // be constructed at the `or` substitute site.
    if !scope.symbols.contains_key("EmptyError") {
        scope.symbols.insert(
            "EmptyError".to_string(),
            TopSymbol::Type(TypeInfo {
                name: "EmptyError".to_string(),
                kind: TypeKind::Struct(vec![FieldInfo {
                    name: "kind".to_string(),
                    ty: Ty::Prim(PrimType::String),
                    has_default: false,
                    tag: None,
                    span: zero,
                }]),
                span: zero,
            }),
        );
    }
    // IoError for the `std::io::fs::*` and `std::io::tcp::*`
    // path-calls that return `fallible(IoError)`. One Error type
    // for I/O surfaces uniform pattern-matching in the agent —
    // the same `or fallback(err)` clause shape works for both
    // file and network operations. Fields:
    //   - kind: a string tag — "not_found", "permission_denied",
    //     "is_dir", "io", "would_block", "connection_refused",
    //     "timeout", "host_unreachable" (extensible).
    //   - errno: raw platform errno for callers that want it.
    //   - path: file path / connection target / "stdin" / "" —
    //     diagnostic context naming what failed.
    if !scope.symbols.contains_key("IoError") {
        scope.symbols.insert(
            "IoError".to_string(),
            TopSymbol::Type(TypeInfo {
                name: "IoError".to_string(),
                kind: TypeKind::Struct(vec![
                    FieldInfo {
                        name: "kind".to_string(),
                        ty: Ty::Prim(PrimType::String),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                    FieldInfo {
                        name: "errno".to_string(),
                        ty: Ty::Prim(PrimType::Int),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                    FieldInfo {
                        name: "path".to_string(),
                        ty: Ty::Prim(PrimType::String),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                ]),
                span: zero,
            }),
        );
    }
    // 2026-05-17 — ParseError for `std::str::parse_int` /
    // `parse_float` after their flip to fallible. Carries:
    //   - kind: "parse_int" / "parse_float" — surfaces which
    //     parser rejected the input.
    //   - input: the original String that failed to parse —
    //     for diagnostic messages.
    if !scope.symbols.contains_key("ParseError") {
        scope.symbols.insert(
            "ParseError".to_string(),
            TopSymbol::Type(TypeInfo {
                name: "ParseError".to_string(),
                kind: TypeKind::Struct(vec![
                    FieldInfo {
                        name: "kind".to_string(),
                        ty: Ty::Prim(PrimType::String),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                    FieldInfo {
                        name: "input".to_string(),
                        ty: Ty::Prim(PrimType::String),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                ]),
                span: zero,
            }),
        );
    }
    // 2026-06-04 — CryptoError for the `std::crypto::*` path-calls
    // that return `fallible(CryptoError)` in `or` context (currently
    // `ecdsa_p256_sign`). Carries:
    //   - kind: op tag — "ecdsa_p256_sign" — which crypto op failed.
    //   - detail: human-readable failure context (e.g. "signing
    //     failed (bad key or non-P-256 curve)").
    if !scope.symbols.contains_key("CryptoError") {
        scope.symbols.insert(
            "CryptoError".to_string(),
            TopSymbol::Type(TypeInfo {
                name: "CryptoError".to_string(),
                kind: TypeKind::Struct(vec![
                    FieldInfo {
                        name: "kind".to_string(),
                        ty: Ty::Prim(PrimType::String),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                    FieldInfo {
                        name: "detail".to_string(),
                        ty: Ty::Prim(PrimType::String),
                        has_default: false,
                        tag: None,
                        span: zero,
                    },
                ]),
                span: zero,
            }),
        );
    }
}

/// FUv0.8.2 #1 (2026-05-25): when a user has declared a `type
/// E { ... }` whose name shadows one of the stdlib's expected
/// error types, the prior architecture silently let the user's
/// version win at the resolver layer; codegen later failed when
/// it tried to allocate the stdlib's shape and found a field
/// missing. The failure point was 100s of LOC away from the
/// user's declaration, with a span-less diagnostic.
///
/// This pass fires AFTER `inject_form_stdlib_types` so the
/// `scope.symbols[name]` for each candidate type holds whichever
/// version "won" the contains_key race. If the winner is a
/// user decl (real span) and its shape doesn't match the
/// stdlib's expected shape (named field by name + primitive
/// type), emit a type error at the user's decl span pointing
/// at the rename / qualify workarounds.
///
/// Each stdlib error type the stdlib internally constructs by
/// bare name lands in this table. If the language ever moves
/// to mangled-name lookups internally (so user collisions are
/// truly impossible), this pass becomes dead code and can be
/// retired.
/// Set of stdlib error type names that the program actually
/// reaches for — populated from a walk of all path-call sites.
/// If `ParseError` isn't in this set, the user can shadow it
/// freely; no diagnostic fires.
#[derive(Debug, Default)]
struct StdlibErrorUsage {
    parse_error: bool,
    io_error: bool,
    index_error: bool,
    key_error: bool,
    empty_error: bool,
    crypto_error: bool,
}

/// Scan every top-level item, every fn / locus-method body,
/// and every Expr for stdlib path-calls whose codegen needs
/// one of the stdlib error types. Detection is by qualified
/// path prefix (`std::str::parse_*`, `std::io::fs::*`,
/// `std::io::tcp::*`) plus any `@form(vec)` / `@form(hashmap)`
/// / `@form(ring_buffer)` decl that synthesizes a fallible
/// method using the respective error type.
fn scan_stdlib_error_usage(bundle: &Bundle<'_>) -> StdlibErrorUsage {
    let mut out = StdlibErrorUsage::default();
    for program in bundle.programs.values() {
        scan_items_for_stdlib_usage(&program.items, &mut out);
    }
    out
}

fn scan_items_for_stdlib_usage(items: &[TopDecl], out: &mut StdlibErrorUsage) {
    for item in items {
        match item {
            TopDecl::Locus(l) => {
                if let Some(form) = &l.form {
                    match form.name.name.as_str() {
                        "vec" => out.index_error = true,
                        "hashmap" => out.key_error = true,
                        "ring_buffer" => out.empty_error = true,
                        _ => {}
                    }
                }
                for member in &l.members {
                    match member {
                        LocusMember::Lifecycle(lc) => {
                            scan_block_for_stdlib_usage(&lc.body, out)
                        }
                        LocusMember::Fn(f) => {
                            scan_block_for_stdlib_usage(&f.body, out)
                        }
                        LocusMember::Mode(md) => {
                            scan_block_for_stdlib_usage(&md.body, out)
                        }
                        LocusMember::Failure(fd) => {
                            scan_block_for_stdlib_usage(&fd.body, out)
                        }
                        LocusMember::Closure(c) => {
                            if let Some(a) = &c.assertion {
                                scan_expr_for_stdlib_usage(&a.left, out);
                                scan_expr_for_stdlib_usage(&a.right, out);
                                scan_expr_for_stdlib_usage(
                                    &a.tolerance,
                                    out,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            TopDecl::Fn(f) => scan_block_for_stdlib_usage(&f.body, out),
            TopDecl::Module(m) => {
                scan_items_for_stdlib_usage(&m.items, out)
            }
            _ => {}
        }
    }
}

fn scan_block_for_stdlib_usage(b: &Block, out: &mut StdlibErrorUsage) {
    for s in &b.stmts {
        scan_stmt_for_stdlib_usage(s, out);
    }
}

fn scan_stmt_for_stdlib_usage(s: &Stmt, out: &mut StdlibErrorUsage) {
    match s {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            scan_expr_for_stdlib_usage(value, out)
        }
        Stmt::Assign { value, .. } => scan_expr_for_stdlib_usage(value, out),
        Stmt::If(s) => {
            scan_expr_for_stdlib_usage(&s.cond, out);
            scan_block_for_stdlib_usage(&s.then_block, out);
            match s.else_block.as_deref() {
                None => {}
                Some(ElseBranch::Else(b)) => {
                    scan_block_for_stdlib_usage(b, out)
                }
                Some(ElseBranch::ElseIf(inner)) => {
                    scan_stmt_for_stdlib_usage(
                        &Stmt::If(inner.clone()),
                        out,
                    )
                }
            }
        }
        Stmt::Match(m) => {
            scan_expr_for_stdlib_usage(&m.scrutinee, out);
            for arm in &m.arms {
                if let Some(g) = &arm.guard {
                    scan_expr_for_stdlib_usage(g, out);
                }
                match &arm.body {
                    MatchArmBody::Expr(e) => {
                        scan_expr_for_stdlib_usage(e, out)
                    }
                    MatchArmBody::Block(b) => {
                        scan_block_for_stdlib_usage(b, out)
                    }
                }
            }
        }
        Stmt::For { iter, body, .. } => {
            scan_expr_for_stdlib_usage(iter, out);
            scan_block_for_stdlib_usage(body, out);
        }
        Stmt::While { cond, body, .. } => {
            scan_expr_for_stdlib_usage(cond, out);
            scan_block_for_stdlib_usage(body, out);
        }
        Stmt::Return(Some(e), _) => scan_expr_for_stdlib_usage(e, out),
        Stmt::Fail { value, .. } => scan_expr_for_stdlib_usage(value, out),
        Stmt::Block(b) => scan_block_for_stdlib_usage(b, out),
        Stmt::ShmWrite { max, body, .. } => {
            scan_expr_for_stdlib_usage(max, out);
            scan_block_for_stdlib_usage(body, out);
        }
        Stmt::Send { subject, value, .. } => {
            scan_expr_for_stdlib_usage(subject, out);
            scan_expr_for_stdlib_usage(value, out);
        }
        Stmt::Expr(e) => scan_expr_for_stdlib_usage(e, out),
        Stmt::Recovery { args, .. } => {
            for a in args {
                scan_expr_for_stdlib_usage(a, out);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(p) = payload {
                scan_expr_for_stdlib_usage(p, out);
            }
        }
        Stmt::Return(None, _)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Yield(_) | Stmt::Terminate(_) => {}
    }
}

fn scan_expr_for_stdlib_usage(e: &Expr, out: &mut StdlibErrorUsage) {
    match e {
        Expr::Call { callee, args, .. } => {
            check_callee_path_for_stdlib_error(callee, out);
            scan_expr_for_stdlib_usage(callee, out);
            for a in args {
                scan_expr_for_stdlib_usage(a, out);
            }
        }
        Expr::Path(qn) => mark_stdlib_error_from_path(qn, out),
        Expr::Binary { left, right, .. } => {
            scan_expr_for_stdlib_usage(left, out);
            scan_expr_for_stdlib_usage(right, out);
        }
        Expr::Unary { operand, .. } => {
            scan_expr_for_stdlib_usage(operand, out)
        }
        Expr::Field { receiver, .. } => {
            scan_expr_for_stdlib_usage(receiver, out)
        }
        Expr::Index { receiver, index, .. } => {
            scan_expr_for_stdlib_usage(receiver, out);
            scan_expr_for_stdlib_usage(index, out);
        }
        Expr::Path2 { receiver, .. } => {
            scan_expr_for_stdlib_usage(receiver, out)
        }
        Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
            for p in parts {
                scan_expr_for_stdlib_usage(p, out);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                scan_expr_for_stdlib_usage(&i.value, out);
            }
        }
        Expr::Block(b) => scan_block_for_stdlib_usage(b, out),
        Expr::If(s) => {
            scan_expr_for_stdlib_usage(&s.cond, out);
            scan_block_for_stdlib_usage(&s.then_block, out);
            if let Some(ElseBranch::Else(b)) = s.else_block.as_deref() {
                scan_block_for_stdlib_usage(b, out);
            }
        }
        Expr::Match(m) => {
            scan_expr_for_stdlib_usage(&m.scrutinee, out);
            for arm in &m.arms {
                if let Some(g) = &arm.guard {
                    scan_expr_for_stdlib_usage(g, out);
                }
                match &arm.body {
                    MatchArmBody::Expr(e) => {
                        scan_expr_for_stdlib_usage(e, out)
                    }
                    MatchArmBody::Block(b) => {
                        scan_block_for_stdlib_usage(b, out)
                    }
                }
            }
        }
        Expr::Or { inner, disposition, .. } => {
            scan_expr_for_stdlib_usage(inner, out);
            match disposition {
                OrDisposition::Substitute(rhs) => {
                    scan_expr_for_stdlib_usage(rhs, out)
                }
                OrDisposition::Fail(payload, _) => {
                    scan_expr_for_stdlib_usage(payload, out)
                }
                OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
            }
        }
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => {
            scan_expr_for_stdlib_usage(inner, out)
        }
        Expr::Approx { left, right, tolerance, .. } => {
            scan_expr_for_stdlib_usage(left, out);
            scan_expr_for_stdlib_usage(right, out);
            scan_expr_for_stdlib_usage(tolerance, out);
        }
        Expr::Range { lo, hi, .. } => {
            scan_expr_for_stdlib_usage(lo, out);
            scan_expr_for_stdlib_usage(hi, out);
        }
        Expr::ArrayRepeat { val, .. } => {
            scan_expr_for_stdlib_usage(val, out)
        }
        _ => {}
    }
}

fn check_callee_path_for_stdlib_error(
    callee: &Expr,
    out: &mut StdlibErrorUsage,
) {
    let Expr::Path(qn) = callee else { return };
    mark_stdlib_error_from_path(qn, out);
}

fn mark_stdlib_error_from_path(
    qn: &QualifiedName,
    out: &mut StdlibErrorUsage,
) {
    let segs: Vec<&str> =
        qn.segments.iter().map(|s| s.name.as_str()).collect();
    if segs.len() >= 3 && segs[0] == "std" {
        match segs[1] {
            "str" => {
                // parse_int, parse_float, parse_decimal, plus the
                // 2026-05-26 range-bounded variants
                // range_parse_int / range_parse_decimal.
                if segs[2].starts_with("parse_")
                    || segs[2].starts_with("range_parse_")
                {
                    out.parse_error = true;
                }
            }
            "io" => {
                // std::io::fs::* and std::io::tcp::* both
                // return fallible(IoError). std::io::stdin
                // also has io fallibles.
                if segs.len() >= 3 {
                    out.io_error = true;
                }
            }
            "crypto" => {
                // std::crypto::ecdsa_p256_sign returns
                // fallible(CryptoError) in `or` context.
                if segs[2] == "ecdsa_p256_sign" {
                    out.crypto_error = true;
                }
            }
            "bytes" => {
                // std::bytes::at + the binary-pack readers
                // (read_u32_le, ...) + the A1 writers (write_u32_le, ...)
                // return fallible(IndexError).
                if segs[2] == "at"
                    || segs[2].starts_with("read_")
                    || segs[2].starts_with("write_")
                {
                    out.index_error = true;
                }
            }
            _ => {}
        }
    }
}

fn check_stdlib_error_shadowing(
    scope: &TopScope,
    usage: &StdlibErrorUsage,
    diags: &mut Vec<Diag>,
) {
    // (stdlib type name, expected fields, qualified-path
    // suggestion to surface in the diagnostic, "is this type
    // actually used in this program" gate).
    let expected: &[(&str, &[(&str, PrimType)], &str, bool)] = &[
        (
            "ParseError",
            &[
                ("kind", PrimType::String),
                ("input", PrimType::String),
            ],
            "std::str::ParseError",
            usage.parse_error,
        ),
        (
            "IoError",
            &[
                ("kind", PrimType::String),
                ("errno", PrimType::Int),
                ("path", PrimType::String),
            ],
            "std::io::IoError",
            usage.io_error,
        ),
        (
            "IndexError",
            &[
                ("kind", PrimType::String),
                ("index", PrimType::Int),
                ("len", PrimType::Int),
            ],
            "std::index::IndexError",
            usage.index_error,
        ),
        (
            "KeyError",
            &[("kind", PrimType::String)],
            "std::form::hashmap::KeyError",
            usage.key_error,
        ),
        (
            "EmptyError",
            &[("kind", PrimType::String)],
            "std::form::ring_buffer::EmptyError",
            usage.empty_error,
        ),
        (
            "CryptoError",
            &[
                ("kind", PrimType::String),
                ("detail", PrimType::String),
            ],
            "std::crypto::CryptoError",
            usage.crypto_error,
        ),
    ];
    for (name, expected_fields, qualified, in_use) in expected {
        if !in_use {
            continue;
        }
        let Some(TopSymbol::Type(ti)) = scope.symbols.get(*name)
        else {
            continue;
        };
        // Zero-span entries are the stdlib's own injections —
        // those by definition match the stdlib's expected
        // shape. Skip.
        if ti.span.start.0 == 0 && ti.span.end.0 == 0 {
            continue;
        }
        let TypeKind::Struct(actual) = &ti.kind else {
            diags.push(Diag::ty(
                ti.span,
                format!(
                    "user-declared `type {}` shadows the stdlib's \
                     `{}` (used by stdlib path-calls that return \
                     `fallible({})`) but is not a struct type. \
                     Either rename your type (e.g. `My{}`) or use \
                     `{}` qualified where you need the stdlib's \
                     shape.",
                    name, name, name, name, qualified,
                ),
            ));
            continue;
        };
        for (field_name, expected_ty) in *expected_fields {
            let matches = actual.iter().any(|f| {
                f.name == *field_name
                    && matches!(&f.ty, Ty::Prim(p) if p == expected_ty)
            });
            if matches {
                continue;
            }
            diags.push(Diag::ty(
                ti.span,
                format!(
                    "user-declared `type {}` shadows the stdlib's \
                     `{}` but is missing the expected `{}: {}` \
                     field (used by stdlib path-calls that \
                     allocate `{}` on failure). Either match the \
                     stdlib shape, rename your type (e.g. `My{}`), \
                     or use `{}` qualified where you need the \
                     stdlib's shape.",
                    name,
                    name,
                    field_name,
                    prim_display(*expected_ty),
                    name,
                    name,
                    qualified,
                ),
            ));
            break;
        }
    }
}

fn prim_display(p: PrimType) -> &'static str {
    match p {
        PrimType::Int => "Int",
        PrimType::Uint => "Uint",
        PrimType::Float => "Float",
        PrimType::Decimal => "Decimal",
        PrimType::Bool => "Bool",
        PrimType::String => "String",
        PrimType::Time => "Time",
        PrimType::Duration => "Duration",
        PrimType::Bytes => "Bytes",
        PrimType::BytesView => "BytesView",
        PrimType::BytesMut => "BytesMut",
        PrimType::StringView => "StringView",
    }
}

/// Best-effort literal-typing for params declared with a value
/// but no explicit `: ty`. Just enough to give `B: Int = 100`
/// the right type when `: Int` is omitted; falls through to
/// Unknown otherwise.
fn infer_literal_ty(e: &Expr) -> Ty {
    match e {
        Expr::Literal(Literal::Int(_), _) => Ty::Prim(PrimType::Int),
        Expr::Literal(Literal::Float(_), _) => Ty::Prim(PrimType::Float),
        Expr::Literal(Literal::Decimal(_), _) => Ty::Prim(PrimType::Decimal),
        Expr::Literal(Literal::String(_), _) => Ty::Prim(PrimType::String),
        Expr::Literal(Literal::Bool(_), _) => Ty::Prim(PrimType::Bool),
        Expr::Literal(Literal::Duration(_), _) => Ty::Prim(PrimType::Duration),
        Expr::Literal(Literal::Time(_), _) => Ty::Prim(PrimType::Time),
        Expr::Literal(Literal::Bytes(_), _) => Ty::Prim(PrimType::Bytes),
        _ => Ty::Unknown,
    }
}
