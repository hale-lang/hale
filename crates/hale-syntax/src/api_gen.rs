//! GH #1106: the API binding, as pre-check synthesis.
//!
//! `bindings { api: unix("/run/app.sock", bound: 64, on_full: refuse); }`
//! on the main locus makes the program drivable from outside with no
//! other change to its source: every topic some locus subscribes is a
//! command, every topic some locus publishes is a stream, and every
//! `expose` of the main locus or of one of its default children is a
//! read. This pass turns that one entry into ordinary Hale: envelope
//! types and topics, a synthesized subscription per subscriber that
//! calls the author's handler and publishes its return as the reply,
//! a read subject per exposed member answered on the locus's own
//! pool, and two loci on their own `async_io` pool that own the
//! socket. Everything it emits is typechecked like the author's code,
//! and a program without the entry gets nothing.
//!
//! The wire is one JSON object per line over a Unix domain stream
//! socket: `{"call": "Topic", "payload": {...}}`, `{"read": "name"}`
//! or `{"watch": "Topic"}`, each with an optional `"id"` the client
//! chooses. Every answer carries the binding's `request_id`, echoes
//! `id`, and is `{"ok": true, ...}` or `{"ok": false, "refusal":
//! {"kind": ..., "reason": ...}}`. The contract is in
//! `spec/semantics.md` § "The api binding".
//!
//! Layout of this file: [`api_surface`] classifies the program (the
//! checker calls it too, so the warnings it raises and the code this
//! pass emits describe one surface), [`generate_api`] emits.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{
    ApiBinding, ApiTransport, ApiUnauthorizedPolicy, BusMember, BusSubject, ContractDirection,
    ContractKind, ContractName, Expr, Ident, Literal, LocusDecl, LocusMember, ParamInit,
    PlacementBlock, Program, ShedPolicy, StructField, TopDecl, TopicDecl, TypeDeclBody, TypeExpr,
};
use crate::span::Span;
use crate::json_gen;
use crate::parse_source;

/// The dev defaults `hale run --api` fills in, and the wording a
/// missing knob's diagnostic quotes.
pub const DEV_BOUND: i64 = 64;

/// The parse space of the synthesized source (GH #1109 review): every
/// span of a generated item starts here, past any file of a bundle, so
/// a diagnostic about the binding is rendered as such rather than at a
/// position in whatever file the bundle lists first.
pub const API_SYNTH_BASE: u32 = 0x7000_0000;

/// A subscriber of a command topic.
#[derive(Debug, Clone)]
pub struct ApiSubscriber {
    pub locus: String,
    pub handler: String,
    pub handler_span: Span,
    /// The handler's declared return type, when it has one: the reply.
    pub ret: Option<TypeExpr>,
    /// The handler takes `Drain<T>`: a batch consumer.
    pub is_drain: bool,
    /// The handler declares a second `std::api::Context` parameter.
    pub takes_context: bool,
    /// GH #1109: the handler's `@gated(role:)`.
    pub role: Option<String>,
    /// The subscription's `where key ==` filter, cloned onto the
    /// synthesized subscription so a keyed command routes as its
    /// topic does.
    pub member: BusMember,
}

#[derive(Debug, Clone)]
pub struct ApiCommand {
    /// The topic as the author spells it (`Verdict`, `lib::Verdict`);
    /// the name a caller writes in `"call"`.
    pub name: String,
    /// The topic's name in the program (mangled for an imported one):
    /// what synthesized source refers to.
    pub internal: String,
    /// The topic's wire subject, as the description reports it.
    pub subject: String,
    /// The payload type's name in the program.
    pub payload: String,
    /// `keyed_by` field and its type, when the topic is keyed.
    pub key: Option<(String, TypeExpr)>,
    pub subscribers: Vec<ApiSubscriber>,
    /// Index into `subscribers` of the one that answers, if any.
    pub replier: Option<usize>,
    /// GH #1109: the role a caller must hold, from the subscribers'
    /// `@gated(role:)` (the checker makes them agree).
    pub role: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiRead {
    /// `ledger` on the main locus, `billing.ledger` on a default child.
    pub name: String,
    pub locus: String,
    pub member: String,
    /// The member is a fn (called with no arguments) rather than a field.
    pub is_fn: bool,
    pub ty: TypeExpr,
    /// GH #1109: the `expose` member's `@gated(role:)`.
    pub role: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiStream {
    pub name: String,
    pub internal: String,
    pub subject: String,
    pub payload: String,
    /// GH #1109: the `publish` member's `@gated(role:)`, checked once
    /// when a watcher attaches.
    pub role: Option<String>,
}

/// GH #1109: a declared role. `owner` is always present — the full
/// description is a read gated on it — declared by the program only
/// to give it `includes`.
#[derive(Debug, Clone)]
pub struct ApiRole {
    pub name: String,
    pub includes: Vec<String>,
}

/// One field of a struct the description carries a schema for.
#[derive(Debug, Clone)]
pub struct ApiField {
    pub name: String,
    /// The JSON key: the `json:` tag when one is set, else the name.
    pub key: String,
    /// `Int`, `Float`, `Bool`, `String`, or the name of a nested struct.
    pub kind: String,
    pub nested: bool,
    /// No literal default, so a decode without it is `missing_field`.
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct ApiSchema {
    pub name: String,
    pub fields: Vec<ApiField>,
}

/// A topic or member the api leaves out, with the reason the checker
/// reports at the `api:` entry.
#[derive(Debug, Clone)]
pub struct ApiExcluded {
    pub what: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ApiSurface {
    pub main_locus: String,
    pub binding: ApiBinding,
    pub commands: Vec<ApiCommand>,
    pub reads: Vec<ApiRead>,
    pub streams: Vec<ApiStream>,
    pub excluded: Vec<ApiExcluded>,
    /// Two subscribers of one topic both declare a return type: the
    /// reply would be ambiguous. (topic, first handler span, second).
    pub ambiguous_replies: Vec<(String, Span, Span)>,
    /// Every struct type a JSON codec is generated for.
    pub json_types: Vec<String>,
    /// The field schema of every type in `json_types`, sorted by name.
    pub schemas: Vec<ApiSchema>,
    /// GH #1109: the declared roles plus `owner`, sorted by name.
    pub roles: Vec<ApiRole>,
    /// Program name -> author spelling, for imported types.
    pub type_display: BTreeMap<String, String>,
}

// ---- walking -------------------------------------------------------

fn walk_items<'a>(items: &'a [TopDecl], f: &mut dyn FnMut(&'a TopDecl)) {
    for item in items {
        f(item);
        if let TopDecl::Module(m) = item {
            walk_items(&m.items, f);
        }
    }
}

fn walk_items_mut(items: &mut [TopDecl], f: &mut dyn FnMut(&mut TopDecl)) {
    for item in items {
        f(item);
        if let TopDecl::Module(m) = item {
            walk_items_mut(&mut m.items, f);
        }
    }
}

fn subject_name(s: &BusSubject) -> Option<String> {
    match s {
        BusSubject::Topic(i) => Some(i.name.clone()),
        BusSubject::QualifiedTopic(qn) => Some(
            qn.segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join("::"),
        ),
        BusSubject::Literal { .. } => None,
    }
}

fn named_type(te: &TypeExpr) -> Option<String> {
    match te {
        TypeExpr::Named { path, generic_args, .. }
            if generic_args.is_empty() && !path.segments.is_empty() =>
        {
            Some(
                path.segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join("::"),
            )
        }
        _ => None,
    }
}

/// `std::api::Context`, in either of its spellings.
pub fn is_context_type(te: &TypeExpr) -> bool {
    match te {
        TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            segs == ["std", "api", "Context"] || segs == ["__StdApiContext"]
        }
        _ => false,
    }
}

/// `alias::Topic` → `alias__Topic`: a name usable inside an identifier.
pub fn mangle(name: &str) -> String {
    name.replace("::", "__")
}

/// The type of a field, looked up across the bundle's struct decls.
fn field_type<'a>(
    types: &'a BTreeMap<String, &'a [StructField]>,
    ty: &str,
    field: &str,
) -> Option<&'a TypeExpr> {
    types
        .get(ty)
        .and_then(|fs| fs.iter().find(|f| f.name.name == field))
        .map(|f| &f.ty)
}

/// Why a type has no JSON form, or `None` when it has one. Walks the
/// nested structs it reaches and records every name it saw in
/// `seen`, so the caller knows which codecs to generate.
fn json_reason(
    types: &BTreeMap<String, &[StructField]>,
    te: &TypeExpr,
    seen: &mut BTreeSet<String>,
    depth: usize,
) -> Option<String> {
    if depth > 16 {
        return Some("nests deeper than 16 levels".to_string());
    }
    if json_gen::scalar_name(te).is_some() {
        return None;
    }
    let Some(name) = named_type(te) else {
        return Some(match te {
            TypeExpr::Primitive(p, _) => {
                format!("has type `{:?}`, which has no JSON form yet", p)
            }
            TypeExpr::Array { .. } => "is an array, which has no JSON form yet".to_string(),
            _ => "is not a scalar or a struct, so it has no JSON form".to_string(),
        });
    };
    let Some(fields) = types.get(&name) else {
        return Some(format!(
            "has type `{}`, which is not a struct the api can encode",
            name
        ));
    };
    if !seen.insert(name.clone()) {
        return None;
    }
    for f in fields.iter() {
        if let Some(r) = json_reason(types, &f.ty, seen, depth + 1) {
            return Some(format!(
                "field `{}` of `{}` {}",
                f.name.name, name, r
            ));
        }
    }
    None
}

/// Find the one main locus carrying an `api:` entry and classify the
/// program around it. `None` when no program has one.
pub fn api_surface(programs: &[&Program]) -> Option<ApiSurface> {
    let mut main: Option<(&LocusDecl, ApiBinding)> = None;
    let mut topics: BTreeMap<String, &TopicDecl> = BTreeMap::new();
    let mut loci: BTreeMap<String, &LocusDecl> = BTreeMap::new();
    let mut types: BTreeMap<String, &[StructField]> = BTreeMap::new();
    let mut role_decls: Vec<ApiRole> = Vec::new();
    let mut type_display: BTreeMap<String, String> = BTreeMap::new();
    for p in programs {
        walk_items(&p.items, &mut |item| match item {
            TopDecl::Locus(l) => {
                loci.insert(l.name.name.clone(), l);
                // An imported seed's main locus is not the entrypoint
                // (its bindings are inert): a composed head that imports
                // one carrying an api entry gets no binding from it.
                if l.is_main && !l.imported && main.is_none() {
                    for m in &l.members {
                        if let LocusMember::Bindings(bb) = m {
                            if let Some(api) = &bb.api {
                                main = Some((l, api.clone()));
                            }
                        }
                    }
                }
            }
            TopDecl::Topic(t) => {
                topics.insert(t.name.name.clone(), t);
            }
            TopDecl::Role(r) => {
                role_decls.push(ApiRole {
                    name: r.name.name.clone(),
                    includes: r.includes.iter().map(|i| i.name.clone()).collect(),
                });
            }
            TopDecl::Type(t) => {
                if let TypeDeclBody::Struct(fs) = &t.body {
                    types.insert(t.name.name.clone(), fs.as_slice());
                }
                if let Some(d) = &t.display {
                    type_display.insert(t.name.name.clone(), d.clone());
                }
            }
            _ => {}
        });
    }
    let (main_locus, binding) = main?;
    let mut excluded = Vec::new();
    let mut json_types: BTreeSet<String> = BTreeSet::new();
    let mut ambiguous = Vec::new();

    // Commands and streams: what the loci subscribe and publish, by
    // topic name. A literal subject has no topic and stays out.
    let mut subs: BTreeMap<String, Vec<ApiSubscriber>> = BTreeMap::new();
    // A stream's gate is its publishers' `@gated`; the checker makes
    // every publisher of one topic agree, so the first is the one.
    let mut pubs: BTreeMap<String, Option<String>> = BTreeMap::new();
    for l in loci.values() {
        // A library's loci are not the application's API (GH #1104
        // piece 5): the head that imports its core must not serve the
        // core's internal bus.
        if l.name.name.starts_with("__Api") || l.imported {
            continue;
        }
        let fns: BTreeMap<&str, &crate::ast::FnDecl> = l
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Fn(f) => Some((f.name.name.as_str(), f)),
                _ => None,
            })
            .collect();
        for m in &l.members {
            let LocusMember::Bus(bb) = m else { continue };
            for member in &bb.members {
                match member {
                    BusMember::Subscribe { subject, handler, .. } => {
                        let Some(name) = subject_name(subject) else { continue };
                        let hname = handler.name.as_str();
                        let Some(f) = fns.get(hname) else { continue };
                        let is_drain = f.params.first().is_some_and(|p| {
                            matches!(&p.ty, TypeExpr::Named { path, .. }
                                if path.segments.len() == 1 && path.segments[0].name == "Drain")
                        });
                        let takes_context = f.params.len() == 2 && is_context_type(&f.params[1].ty);
                        subs.entry(name).or_default().push(ApiSubscriber {
                            locus: l.name.name.clone(),
                            handler: hname.to_string(),
                            handler_span: f.name.span,
                            ret: f.ret.clone(),
                            is_drain,
                            takes_context,
                            role: f.gated.as_ref().map(|g| g.name.clone()),
                            member: member.clone(),
                        });
                    }
                    BusMember::Publish { subject, gated, .. } => {
                        if let Some(name) = subject_name(subject) {
                            let e = pubs.entry(name).or_default();
                            if e.is_none() {
                                *e = gated.as_ref().map(|g| g.name.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    let mut commands = Vec::new();
    for (name, subscribers) in subs {
        if name.starts_with("__Api") {
            continue;
        }
        let Some(t) = topics.get(&name) else { continue };
        let Some(payload) = named_type(&t.payload) else {
            excluded.push(ApiExcluded {
                what: format!("topic `{}`", name),
                reason: "its payload is not a struct".to_string(),
            });
            continue;
        };
        let mut seen = BTreeSet::new();
        if let Some(r) = json_reason(&types, &t.payload, &mut seen, 0) {
            excluded.push(ApiExcluded {
                what: format!("topic `{}`", name),
                reason: format!("its payload {}", r),
            });
            continue;
        }
        // A batch handler takes `Drain<T>`; the cooperative queue has no
        // batch delivery, so the topic is out until it does (GH #1106
        // defers bulk requests).
        if subscribers.iter().any(|s| s.is_drain) {
            excluded.push(ApiExcluded {
                what: format!("topic `{}`", name),
                reason: "a `Drain<T>` handler subscribes it, and bulk requests are not \
                         delivered through the api binding yet"
                    .to_string(),
            });
            continue;
        }
        let key = match &t.keyed_by {
            Some(k) => match field_type(&types, &payload, &k.name) {
                Some(kt) => Some((k.name.clone(), kt.clone())),
                None => {
                    excluded.push(ApiExcluded {
                        what: format!("topic `{}`", name),
                        reason: format!(
                            "it is keyed by `{}`, which the api cannot find on `{}`",
                            k.name, payload
                        ),
                    });
                    continue;
                }
            },
            None => None,
        };
        let mut replier = None;
        let mut ok = true;
        for (i, s) in subscribers.iter().enumerate() {
            let Some(ret) = &s.ret else { continue };
            let mut seen_r = BTreeSet::new();
            if let Some(r) = json_reason(&types, ret, &mut seen_r, 0) {
                excluded.push(ApiExcluded {
                    what: format!("topic `{}`", name),
                    reason: format!(
                        "the return type of `{}.{}` {}",
                        s.locus, s.handler, r
                    ),
                });
                ok = false;
                break;
            }
            match replier {
                None => {
                    replier = Some(i);
                    json_types.extend(seen_r);
                }
                Some(first) => {
                    ambiguous.push((
                        name.clone(),
                        subscribers[first].handler_span,
                        s.handler_span,
                    ));
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        json_types.extend(seen);
        let subject = t.subject.clone().unwrap_or_else(|| name.clone());
        let role = subscribers.iter().find_map(|s| s.role.clone());
        commands.push(ApiCommand {
            name: t.display.clone().unwrap_or_else(|| name.clone()),
            internal: name,
            subject,
            payload,
            key,
            subscribers,
            replier,
            role,
        });
    }

    let mut streams = Vec::new();
    for (name, pub_role) in pubs {
        if name.starts_with("__Api") {
            continue;
        }
        // A stream follows the same gate as the topic's subscribers
        // unless its publish member states its own (review ruling).
        let role = pub_role.or_else(|| commands.iter().find(|c| c.internal == name).and_then(|c| c.role.clone()));
        let Some(t) = topics.get(&name) else { continue };
        let Some(payload) = named_type(&t.payload) else {
            excluded.push(ApiExcluded {
                what: format!("stream `{}`", name),
                reason: "its payload is not a struct".to_string(),
            });
            continue;
        };
        let mut seen = BTreeSet::new();
        if let Some(r) = json_reason(&types, &t.payload, &mut seen, 0) {
            excluded.push(ApiExcluded {
                what: format!("stream `{}`", name),
                reason: format!("its payload {}", r),
            });
            continue;
        }
        json_types.extend(seen);
        let subject = t.subject.clone().unwrap_or_else(|| name.clone());
        streams.push(ApiStream {
            name: t.display.clone().unwrap_or_else(|| name.clone()),
            internal: name,
            subject,
            payload,
            role,
        });
    }

    // Reads: the main locus's exposes, and those of a default child
    // whose type appears once among main's params.
    let mut reads = Vec::new();
    let mut read_targets: Vec<(String, &LocusDecl)> = vec![(String::new(), main_locus)];
    let mut param_types: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in &main_locus.members {
        if let LocusMember::Params(pb) = m {
            for p in &pb.params {
                if let Some(tn) = p.ty.as_ref().and_then(named_type) {
                    param_types.entry(tn).or_default().push(p.name.name.clone());
                }
            }
        }
    }
    for (tn, params) in &param_types {
        let Some(l) = loci.get(tn) else { continue };
        if l.imported {
            continue;
        }
        if params.len() != 1 {
            if l.members.iter().any(|m| matches!(m, LocusMember::Contract(_))) {
                excluded.push(ApiExcluded {
                    what: format!("reads of `{}`", tn),
                    reason: format!(
                        "main holds {} params of that type, so a read name would be \
                         ambiguous",
                        params.len()
                    ),
                });
            }
            continue;
        }
        read_targets.push((params[0].clone(), l));
    }
    for (prefix, l) in read_targets {
        let fields: BTreeMap<&str, &TypeExpr> = l
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Params(pb) => Some(pb),
                _ => None,
            })
            .flat_map(|pb| pb.params.iter())
            .filter_map(|p| p.ty.as_ref().map(|t| (p.name.name.as_str(), t)))
            .collect();
        let fns: BTreeMap<&str, &crate::ast::FnDecl> = l
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Fn(f) => Some((f.name.name.as_str(), f)),
                _ => None,
            })
            .collect();
        for m in &l.members {
            let LocusMember::Contract(cb) = m else { continue };
            let ContractKind::Members(members) = &cb.kind else { continue };
            for cm in members {
                if cm.direction != ContractDirection::Expose {
                    continue;
                }
                let ContractName::Named(id) = &cm.name else { continue };
                let name = if prefix.is_empty() {
                    id.name.clone()
                } else {
                    format!("{}.{}", prefix, id.name)
                };
                let (ty, is_fn) = if let Some(t) = fields.get(id.name.as_str()) {
                    ((*t).clone(), false)
                } else if let Some(f) = fns.get(id.name.as_str()) {
                    match (&f.ret, f.params.is_empty(), f.fallible.is_none()) {
                        (Some(r), true, true) => (r.clone(), true),
                        _ => {
                            excluded.push(ApiExcluded {
                                what: format!("read `{}`", name),
                                reason: "an exposed fn is readable only when it takes no \
                                         argument, returns a value and is not fallible"
                                    .to_string(),
                            });
                            continue;
                        }
                    }
                } else if let Some(t) = &cm.ty {
                    (t.clone(), false)
                } else {
                    continue;
                };
                let mut seen = BTreeSet::new();
                if let Some(r) = json_reason(&types, &ty, &mut seen, 0) {
                    excluded.push(ApiExcluded {
                        what: format!("read `{}`", name),
                        reason: format!("its type {}", r),
                    });
                    continue;
                }
                json_types.extend(seen);
                reads.push(ApiRead {
                    name,
                    locus: l.name.name.clone(),
                    member: id.name.clone(),
                    is_fn,
                    ty,
                    role: cm.gated.as_ref().map(|g| g.name.clone()),
                });
            }
        }
    }

    let json_types: Vec<String> = json_types.into_iter().collect();
    let schemas = json_types
        .iter()
        .filter_map(|tn| {
            let fields = types.get(tn)?;
            Some(ApiSchema {
                name: tn.clone(),
                fields: fields
                    .iter()
                    .map(|f| {
                        let (kind, nested) = match json_gen::scalar_name(&f.ty) {
                            Some(k) => (k.to_string(), false),
                            None => (named_type(&f.ty).unwrap_or_default(), true),
                        };
                        ApiField {
                            name: f.name.name.clone(),
                            key: f
                                .tag
                                .as_deref()
                                .and_then(|t| crate::desugar::tag_value(t, "json"))
                                .unwrap_or_else(|| f.name.name.clone()),
                            kind,
                            nested,
                            required: nested || f.default.is_none(),
                        }
                    })
                    .collect(),
            })
        })
        .collect();
    // The roles: declared ones plus `owner`, which needs no
    // declaration (a program declares it only to give it `includes`).
    let mut roles = role_decls;
    if !roles.iter().any(|r| r.name == "owner") {
        roles.push(ApiRole {
            name: "owner".to_string(),
            includes: Vec::new(),
        });
    }
    roles.sort_by(|a, b| a.name.cmp(&b.name));
    roles.dedup_by(|a, b| a.name == b.name);
    Some(ApiSurface {
        main_locus: main_locus.name.name.clone(),
        binding,
        commands,
        reads,
        streams,
        excluded,
        ambiguous_replies: ambiguous,
        json_types,
        schemas,
        type_display,
        roles,
    })
}

/// GH #1109: the roles whose holding grants `role`: itself first,
/// then every role that `includes` it, transitively, in name order.
/// `includes` is grant-only and union-only, so this is the whole
/// answer; a cycle (the checker refuses one) cannot loop it.
pub fn grants(surface: &ApiSurface, role: &str) -> Vec<String> {
    let mut out = vec![role.to_string()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        let mut more: Vec<String> = surface
            .roles
            .iter()
            .filter(|r| r.includes.iter().any(|inc| *inc == cur))
            .map(|r| r.name.clone())
            .filter(|n| !out.contains(n))
            .collect();
        more.sort();
        more.dedup();
        out.extend(more);
        i += 1;
    }
    out
}

/// GH #1109: the roles the program declares (`role x;`), bundle-wide,
/// in name order — what `hale check --matrix` asks each environment
/// to map. `owner` is included when the program has an api binding or
/// declares a role, because that is when something is gated on it.
pub fn declared_roles(programs: &[&Program]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut has_api = false;
    for p in programs {
        walk_items(&p.items, &mut |i| match i {
            TopDecl::Role(r) => out.push(r.name.name.clone()),
            TopDecl::Locus(l) if l.is_main && !l.imported => {
                if l.members.iter().any(|m| matches!(m, LocusMember::Bindings(bb) if bb.api.is_some())) {
                    has_api = true;
                }
            }
            _ => {}
        });
    }
    if (has_api || !out.is_empty()) && !out.iter().any(|r| r == "owner") {
        out.push("owner".to_string());
    }
    out.sort();
    out.dedup();
    out
}

// ---- the description ----------------------------------------------------

/// The description a binding serves and `hale describe` prints: the
/// program's commands, reads and streams with their schemas, as one
/// JSON object with every key in a fixed order and every list sorted,
/// so the same program describes itself in the same bytes. It is
/// rendered from the surface the binding was built from, so it can
/// never name a subject the binding would refuse. The two notes are
/// part of the document on purpose: a client that presents a gate
/// as a proof or a read as a live view is misreading it.
pub const API_DESCRIPTION_VERSION: i64 = 1;
pub const GATE_NOTE: &str = "a role gate is a boundary check at the binding, never a proof over the program's internal call paths";
pub const READ_NOTE: &str = "a read is a snapshot taken on the locus's own pool, carried with an as_of digest; a live view is a stream";

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_kind(kind: &str) -> &'static str {
    match kind {
        "Int" => "integer",
        "Float" => "number",
        "Bool" => "boolean",
        _ => "string",
    }
}

/// A type's author spelling for the description.
fn shown(surface: &ApiSurface, internal: &str) -> String {
    surface
        .type_display
        .get(internal)
        .cloned()
        .unwrap_or_else(|| internal.to_string())
}

/// GH #1109: the description in pieces, so the binding can serve the
/// slice a caller may use. `head` runs up to and including
/// `"commands":[`; each item is its compact JSON with the role that
/// shows it (`None` = always); a schema is shown when any item that
/// references its type is, so `roles` empty means always and
/// otherwise names the roles any of which shows it. Joining every
/// piece gives [`describe`] byte for byte.
pub struct DescriptionParts {
    pub head: String,
    pub commands: Vec<(String, Option<String>)>,
    pub reads: Vec<(String, Option<String>)>,
    pub streams: Vec<(String, Option<String>)>,
    pub schemas: Vec<(String, Vec<String>)>,
}

/// The struct types a value of `start` carries, transitively, per
/// the surface's schemas (a scalar or unknown name has none).
fn type_closure(surface: &ApiSurface, start: &str, out: &mut BTreeSet<String>) {
    if !out.insert(start.to_string()) {
        return;
    }
    if let Some(sc) = surface.schemas.iter().find(|sc| sc.name == start) {
        for f in &sc.fields {
            if f.nested {
                type_closure(surface, &f.kind, out);
            }
        }
    }
}

fn role_json(role: &Option<String>) -> String {
    match role {
        Some(r) => json_str(r),
        None => "null".to_string(),
    }
}

pub fn describe_parts(surface: &ApiSurface) -> DescriptionParts {
    // The socket path is deployment (I2), not form: a description
    // names what the program is, never where one copy of it listens.
    let head = format!(
        "{{\"hale_api\":{},\"app\":{},\"notes\":{{\"gates\":{},\"reads\":{}}},\"commands\":[",
        API_DESCRIPTION_VERSION,
        json_str(&surface.main_locus),
        json_str(GATE_NOTE),
        json_str(READ_NOTE)
    );
    // Which items show which schema: a type referenced by an ungated
    // item is always shown; otherwise by the roles of the items that
    // reference it.
    let mut schema_roles: BTreeMap<String, Option<BTreeSet<String>>> = BTreeMap::new();
    let mut note = |ty: &str, role: &Option<String>| {
        let mut closure = BTreeSet::new();
        type_closure(surface, ty, &mut closure);
        for t in closure {
            let e = schema_roles.entry(t).or_insert_with(|| Some(BTreeSet::new()));
            match (role, e.as_mut()) {
                (None, _) => *e = None,
                (Some(r), Some(set)) => {
                    set.insert(r.clone());
                }
                (Some(_), None) => {}
            }
        }
    };
    let mut cmds: Vec<&ApiCommand> = surface.commands.iter().collect();
    cmds.sort_by(|a, c| a.name.cmp(&c.name));
    let mut commands = Vec::new();
    for c in &cmds {
        let reply_ty = c.replier.and_then(|i| c.subscribers[i].ret.as_ref()).map(json_type_name);
        note(&c.payload, &c.role);
        if let Some(r) = &reply_ty {
            note(r, &c.role);
        }
        let reply = match &reply_ty {
            Some(r) => json_str(&shown(surface, r)),
            None => "null".to_string(),
        };
        commands.push((
            format!(
                "{{\"name\":{},\"subject\":{},\"payload\":{},\"reply\":{},\"keyed_by\":{},\"role\":{}}}",
                json_str(&c.name),
                json_str(&c.subject),
                json_str(&shown(surface, &c.payload)),
                reply,
                match &c.key {
                    Some((f, _)) => json_str(f),
                    None => "null".to_string(),
                },
                role_json(&c.role)
            ),
            c.role.clone(),
        ));
    }
    let mut rds: Vec<&ApiRead> = surface.reads.iter().collect();
    rds.sort_by(|a, c| a.name.cmp(&c.name));
    let mut reads = Vec::new();
    for r in &rds {
        let ty = json_type_name(&r.ty);
        note(&ty, &r.role);
        reads.push((
            format!(
                "{{\"name\":{},\"type\":{},\"snapshot\":true,\"role\":{}}}",
                json_str(&r.name),
                json_str(&shown(surface, &ty)),
                role_json(&r.role)
            ),
            r.role.clone(),
        ));
    }
    let mut sts: Vec<&ApiStream> = surface.streams.iter().collect();
    sts.sort_by(|a, c| a.name.cmp(&c.name));
    let mut streams = Vec::new();
    for st in &sts {
        note(&st.payload, &st.role);
        streams.push((
            format!(
                "{{\"name\":{},\"subject\":{},\"payload\":{},\"role\":{}}}",
                json_str(&st.name),
                json_str(&st.subject),
                json_str(&shown(surface, &st.payload)),
                role_json(&st.role)
            ),
            st.role.clone(),
        ));
    }
    let mut schemas = Vec::new();
    for sc in &surface.schemas {
        let mut b = String::new();
        b.push_str(&format!("{}:{{\"type\":\"object\",\"properties\":{{", json_str(&shown(surface, &sc.name))));
        for (j, f) in sc.fields.iter().enumerate() {
            if j > 0 {
                b.push(',');
            }
            if f.nested {
                b.push_str(&format!("{}:{{\"$ref\":\"#/schemas/{}\"}}", json_str(&f.key), shown(surface, &f.kind)));
            } else {
                b.push_str(&format!("{}:{{\"type\":\"{}\"}}", json_str(&f.key), json_kind(&f.kind)));
            }
        }
        b.push_str("},\"required\":[");
        let req: Vec<String> = sc.fields.iter().filter(|f| f.required).map(|f| json_str(&f.key)).collect();
        b.push_str(&req.join(","));
        b.push_str("]}");
        let roles: Vec<String> = match schema_roles.get(&sc.name) {
            Some(Some(set)) => set.iter().cloned().collect(),
            _ => Vec::new(),
        };
        schemas.push((b, roles));
    }
    DescriptionParts {
        head,
        commands,
        reads,
        streams,
        schemas,
    }
}

/// The description, as compact JSON: every piece, which is what an
/// `owner` is served for `{"describe": "full"}` and what `hale check
/// --dump-api` emits.
pub fn describe(surface: &ApiSurface) -> String {
    let p = describe_parts(surface);
    let mut b = p.head;
    b.push_str(&p.commands.iter().map(|(j, _)| j.as_str()).collect::<Vec<_>>().join(","));
    b.push_str("],\"reads\":[");
    b.push_str(&p.reads.iter().map(|(j, _)| j.as_str()).collect::<Vec<_>>().join(","));
    b.push_str("],\"streams\":[");
    b.push_str(&p.streams.iter().map(|(j, _)| j.as_str()).collect::<Vec<_>>().join(","));
    b.push_str("],\"schemas\":{");
    b.push_str(&p.schemas.iter().map(|(j, _)| j.as_str()).collect::<Vec<_>>().join(","));
    b.push_str("}}");
    b
}

/// `hale run --api <path>`: put `api: unix(path, bound: 64, on_full:
/// refuse)` on the main locus's bindings before the checker runs. A
/// main locus that already carries an `api:` entry keeps it (the
/// source wins over the flag); a program with no main locus cannot
/// hold a binding and is refused with the rule.
pub fn inject_api_entry(program: &mut Program, path: &str) -> Result<(), String> {
    let path = path.strip_prefix("unix:").unwrap_or(path).to_string();
    let mut injected = false;
    let mut main_seen = false;
    walk_items_mut(&mut program.items, &mut |item| {
        let TopDecl::Locus(l) = item else { return };
        if !l.is_main || injected {
            return;
        }
        main_seen = true;
        let span = l.name.span;
        let entry = ApiBinding {
            transport: ApiTransport::Unix { path: Expr::Literal(Literal::String(path.clone()), span), span },
            roles: None,
            bound: Some((DEV_BOUND, span)),
            on_full: Some((crate::ast::ApiFullPolicy::Refuse, span)),
            watch_bound: None,
            on_watch_full: None,
            on_unauthorized: None,
            span,
        };
        if let Some(LocusMember::Bindings(bb)) =
            l.members.iter_mut().find(|m| matches!(m, LocusMember::Bindings(_)))
        {
            if bb.api.is_none() {
                bb.api = Some(entry);
            }
        } else {
            l.members.push(LocusMember::Bindings(crate::ast::BindingsBlock {
                entries: Vec::new(),
                api: Some(entry),
                span,
            }));
        }
        injected = true;
    });
    if !main_seen {
        return Err(
            "--api needs a `main locus` to bind: the api entry lives in its \
             `bindings { }` block, and this program has only a bare `fn main`"
                .to_string(),
        );
    }
    Ok(())
}

// ---- emission -------------------------------------------------------

fn json_type_name(te: &TypeExpr) -> String {
    match json_gen::scalar_name(te) {
        Some(s) => s.to_string(),
        None => named_type(te).unwrap_or_default(),
    }
}

/// The Hale expression that JSON-encodes `expr` of type `te`.
fn encode_expr(te: &TypeExpr, expr: &str) -> String {
    match json_gen::scalar_name(te) {
        Some("String") => format!("__api_json_str({})", expr),
        Some(_) => format!("to_string({})", expr),
        None => format!(
            "__api_encode_{}({})",
            named_type(te).unwrap_or_default(),
            expr
        ),
    }
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The one-time helpers, types and topics every api program carries.
fn common_src(watch_bound: i64) -> String {
    let mut b = String::new();
    b.push_str(
        r#"
type __ApiRead { peer: Int; request_id: Int; client_id: String; caller: std::api::Principal; role: String; }
type __ApiReply { peer: Int; request_id: Int; client_id: String; ok: Bool; counted: Bool; body: String; as_of: String; caller: std::api::Principal; role: String; }
topic __ApiReplyT { payload: __ApiReply; subject: "__api.reply"; keyed_by peer; }
type __ApiFrame { subject: String; body: String; }
topic __ApiFrameT { payload: __ApiFrame; subject: "__api.frame"; }
type __ApiIngress { peer: Int; client_id: String; verb: String; subject: String; body: String; caller: std::api::Principal; }
topic __ApiIngressT { payload: __ApiIngress; subject: "__api.ingress"; }

fn __api_hex(b: Bytes) -> String {
    let digits = "0123456789abcdef";
    let mut out = "";
    let n = len(b);
    let mut i = 0;
    while i < n {
        let v = std::bytes::at(b, i) or 0;
        out = out + digits[(v / 16)..(v / 16 + 1)] + digits[(v % 16)..(v % 16 + 1)];
        i = i + 1;
    }
    return out;
}
fn __api_digest(s: String) -> String {
    return "sha256:" + __api_hex(std::crypto::sha256(std::bytes::from_string(s)));
}
fn __api_json_str(s: String) -> String {
    return "\"" + std::json::escape_string(s) + "\"";
}
fn __api_refusal(kind: String, reason: String) -> String {
    return "\"refusal\":{\"kind\":" + __api_json_str(kind) + ",\"reason\":" + __api_json_str(reason) + "}";
}
fn __api_reply_line(r: __ApiReply) -> String {
    let mut line = "{\"request_id\":" + to_string(r.request_id);
    if len(r.client_id) > 0 { line = line + ",\"id\":" + r.client_id; }
    if r.ok { line = line + ",\"ok\":true,"; } else { line = line + ",\"ok\":false,"; }
    line = line + r.body;
    if len(r.as_of) > 0 { line = line + ",\"as_of\":" + __api_json_str(r.as_of); }
    line = line + ",\"caller\":{\"mode\":" + __api_json_str(r.caller.mode) + ",\"name\":" + __api_json_str(r.caller.name)
        + ",\"uid\":" + to_string(r.caller.uid) + ",\"gid\":" + to_string(r.caller.gid) + ",\"pid\":" + to_string(r.caller.pid) + "}";
    if len(r.role) > 0 { line = line + ",\"role\":" + __api_json_str(r.role); }
    return line + "}";
}
fn __api_refusal_role(role: String) -> String {
    return "\"refusal\":{\"kind\":\"unauthorized\",\"reason\":" + __api_json_str("needs role " + role) + ",\"role\":" + __api_json_str(role) + "}";
}
fn __api_join(acc: String, item: String) -> String {
    if len(acc) == 0 { return item; }
    return acc + "," + item;
}
fn __api_context(caller: std::api::Principal, role: String, request_id: Int) -> std::api::Context {
    return std::api::Context { caller: caller, role: role, request_id: request_id, via: "api" };
}
"#,
    );
    b.push_str(&format!(
        "@form(ring_buffer, cap = {})\nlocus __ApiFrameQ {{\n    capacity {{ pool q of __ApiFrame; }}\n}}\n",
        watch_bound
    ));
    b
}

fn peer_src(surface: &ApiSurface, drop_old: bool) -> String {
    let mut b = String::new();
    b.push_str("locus __ApiPeer {\n    params {\n        peer: Int = 0;\n        fd: Int = -1;\n        buf: String = \"\";\n        dropped: Int = 0;\n        flushing: Bool = false;\n        frames: __ApiFrameQ = __ApiFrameQ { };\n        stream: std::io::tcp::Stream = std::io::tcp::Stream { conn_fd: -1, owns_fd: false };\n        caller: std::api::Principal = std::api::Principal { };\n");
    for s in &surface.streams {
        b.push_str(&format!("        watch_{}: Bool = false;\n", mangle(&s.name)));
    }
    b.push_str("    }\n    bus {\n        subscribe __ApiReplyT as on_reply where key == self.peer;\n        subscribe __ApiFrameT as on_frame;\n        publish __ApiIngressT;\n    }\n");
    b.push_str(
        r#"    birth() {
        self.stream = std::io::tcp::Stream { conn_fd: self.fd, owns_fd: true };
        // The peer's credentials, as its kernel vouches for them, once
        // per connection. -1 means the kernel would not say: the peer
        // is then unauthenticated, not anyone.
        let uid = std::io::unix::peer_uid(self.fd);
        let n = std::io::unix::peer_groups_count(self.fd);
        let mut groups = "";
        let mut k = 0;
        while k < n {
            let g = std::io::unix::peer_group_at(self.fd, k);
            if g >= 0 {
                if len(groups) > 0 { groups = groups + ","; }
                groups = groups + to_string(g);
            }
            k = k + 1;
        }
        self.caller = std::api::Principal { mode: "unix", name: "uid:" + to_string(uid), uid: uid, gid: std::io::unix::peer_gid(self.fd), pid: std::io::unix::peer_pid(self.fd), groups: groups };
    }
    @unbounded
    run() {
        let mut open = true;
        while open && !self.draining {
            let chunk = self.stream.recv(65536) or "";
            if len(chunk) == 0 {
                open = false;
            } else {
                self.buf = self.buf + chunk;
                self.drain_lines();
            }
        }
    }
    @unbounded
    fn drain_lines() {
        let mut nl = std::str::index_of(self.buf, "\n");
        while nl >= 0 {
            let line = self.buf[0..nl];
            self.buf = self.buf[(nl + 1)..len(self.buf)];
            self.handle_line(line);
            nl = std::str::index_of(self.buf, "\n");
        }
    }
    fn refuse_here(client_id: String, kind: String, reason: String) {
        self.write_line(__api_reply_line(__ApiReply { peer: self.peer, request_id: 0, client_id: client_id, ok: false, counted: false, body: __api_refusal(kind, reason), as_of: "", caller: self.caller, role: "" }));
    }
    fn handle_line(line: String) {
        let t = std::str::trim(line);
        if len(t) == 0 { return; }
        if !std::json::valid_object(t) {
            self.refuse_here("", "malformed", "a request is one JSON object per line");
            return;
        }
        let client_id = std::json::find_field_raw(t, "id");
        // An unauthenticated peer is refused everything, gated or not:
        // the binding vouches for who is calling, and -1 is nobody.
        if self.caller.uid < 0 {
            self.refuse_here(client_id, "unauthenticated", "the kernel would not say who the peer is");
            return;
        }
        let call = std::json::string_field(t, "call");
        if call.kind == "string" {
            let body = std::json::find_field_raw(t, "payload");
            if len(body) == 0 {
                self.refuse_here(client_id, "malformed", "a call carries a \"payload\" object");
                return;
            }
            __ApiIngressT <- __ApiIngress { peer: self.peer, client_id: client_id, verb: "call", subject: call.text, body: body, caller: self.caller };
            return;
        }
        let rd = std::json::string_field(t, "read");
        if rd.kind == "string" {
            __ApiIngressT <- __ApiIngress { peer: self.peer, client_id: client_id, verb: "read", subject: rd.text, body: "", caller: self.caller };
            return;
        }
        let w = std::json::string_field(t, "watch");
        if w.kind == "string" {
            __ApiIngressT <- __ApiIngress { peer: self.peer, client_id: client_id, verb: "watch", subject: w.text, body: "", caller: self.caller };
            return;
        }
        let d = std::json::find_field_raw(t, "describe");
        if d == "true" {
            __ApiIngressT <- __ApiIngress { peer: self.peer, client_id: client_id, verb: "describe", subject: "", body: "", caller: self.caller };
            return;
        }
        if d == "\"full\"" {
            __ApiIngressT <- __ApiIngress { peer: self.peer, client_id: client_id, verb: "describe", subject: "full", body: "", caller: self.caller };
            return;
        }
        self.refuse_here(client_id, "malformed", "a request is a \"call\", a \"read\", a \"watch\" or a \"describe\"");
    }
    fn on_reply(r: __ApiReply) {
"#,
    );
    for s in &surface.streams {
        b.push_str(&format!(
            "        if r.ok && r.body == \"\\\"attached\\\":\\\"{}\\\"\" {{ self.watch_{} = true; }}\n",
            s.name,
            mangle(&s.name)
        ));
    }
    b.push_str("        self.write_line(__api_reply_line(r));\n    }\n    fn wants(subject: String) -> Bool {\n");
    for s in &surface.streams {
        b.push_str(&format!(
            "        if subject == \"\\\"{}\\\"\" {{ return self.watch_{}; }}\n",
            s.name,
            mangle(&s.name)
        ));
    }
    b.push_str("        return false;\n    }\n");
    b.push_str("    fn on_frame(f: __ApiFrame) {\n        if !self.wants(f.subject) { return; }\n");
    if drop_old {
        b.push_str("        if self.frames.is_full() {\n            let _old = self.frames.pop() or __ApiFrame { subject: \"\", body: \"\" };\n            self.dropped = self.dropped + 1;\n        }\n        let _ok = self.frames.push(f);\n");
    } else {
        b.push_str("        if self.frames.is_full() {\n            self.dropped = self.dropped + 1;\n        } else {\n            let _ok = self.frames.push(f);\n        }\n");
    }
    b.push_str(
        r#"        self.flush();
    }
    fn flush() {
        if self.flushing { return; }
        self.flushing = true;
        let mut more = true;
        while more {
            let fr = self.frames.pop() or __ApiFrame { subject: "", body: "" };
            if len(fr.subject) == 0 {
                more = false;
            } else {
                if self.dropped > 0 {
                    self.write_line("{\"stream\":" + fr.subject + ",\"dropped\":" + to_string(self.dropped) + "}");
                    self.dropped = 0;
                }
                self.write_line("{\"stream\":" + fr.subject + ",\"value\":" + fr.body + "}");
            }
        }
        self.flushing = false;
    }
    fn write_line(line: String) {
        self.stream.send(line + "\n") or discard;
    }
}
"#,
    );
    b
}

/// The Hale statements that gate one operation on `role` (none when
/// ungated): the authorizing role lands in `cur_role`, or the caller
/// is refused and the arm returns. A non-holder is told `unknown`, as
/// for a name that does not exist: an item outside a caller's slice is
/// not disclosed to it (the role is named only on `full`, whose
/// existence every caller knows).
fn gate_src(role: &Option<String>) -> String {
    match role {
        Some(r) => format!(
            "let g = self.__api_gate_{r}(i.caller); if len(g) == 0 {{ self.refuse_gate(i.peer, rid, i.client_id, i.subject); return; }} self.cur_role = g;\n",
            r = r
        ),
        None => String::new(),
    }
}

fn binding_src(surface: &ApiSurface, bound: i64, table: Option<&str>) -> String {
    let mut b = String::new();
    b.push_str("locus __ApiBinding {\n    params {\n");
    // The path comes in from the main locus (the entry's expression).
    b.push_str("        path: String = \"\";\n");
    b.push_str(&format!("        bound: Int = {};\n", bound));
    // GH #1109: the membership source, typed by the interface so the
    // program's own (`roles: <expr>` on the entry, handed in by the main
    // locus) and the stdlib's static table are one param. The table has
    // the environment's roles baked in (empty when no `--env` named
    // one: every gate then refuses until `LOTUS_API_ROLES` says
    // otherwise, never the other way round) and the roles the program
    // declares, so a table naming another is refused at birth.
    let known: Vec<&str> = surface.roles.iter().map(|r| r.name.as_str()).collect();
    b.push_str(&format!(
        "        roles: std::api::RoleSource = std::api::StaticRoles {{ table: {}, known: {} }};\n",
        q(table.unwrap_or("")),
        q(&known.join(" "))
    ));
    let drop = matches!(surface.binding.on_unauthorized, Some((ApiUnauthorizedPolicy::Drop, _)));
    b.push_str(&format!("        unauthorized_drop: Bool = {};\n", drop));
    b.push_str(
        "        listen_fd: Int = -1;\n        bind_failed: String = \"\";\n        next_peer: Int = 1;\n        next_request: Int = 1;\n        in_flight: Int = 0;\n        cur_peer: Int = 0;\n        cur_request: Int = 0;\n        cur_client: String = \"\";\n        cur_caller: std::api::Principal = std::api::Principal { };\n        cur_role: String = \"\";\n        decode_failed: Bool = false;\n    }\n    bus {\n        subscribe __ApiIngressT as on_ingress;\n        subscribe __ApiReplyT as on_reply_seen;\n        publish __ApiReplyT;\n        publish __ApiFrameT;\n",
    );
    for s in &surface.streams {
        b.push_str(&format!(
            "        subscribe {} as __api_stream_{};\n",
            s.internal,
            mangle(&s.name)
        ));
    }
    for c in &surface.commands {
        b.push_str(&format!("        publish __ApiCallT_{};\n", mangle(&c.name)));
    }
    for r in &surface.reads {
        b.push_str(&format!("        publish __ApiReadT_{};\n", mangle(&r.name).replace('.', "_")));
    }
    b.push_str(
        r#"    }
    birth() {
        if std::env::var_exists("LOTUS_API") { self.path = std::env::var("LOTUS_API"); }
        self.listen_fd = std::io::unix::listen_socket(self.path) or -1;
        // A socket that cannot be bound (the path is held by a live
        // process, or cannot be made) does not take the program with it:
        // the rest of the program serves, and the binding says why it does
        // not (review B3).
        if self.listen_fd < 0 {
            self.bind_failed = "the api binding could not listen on " + self.path;
            eprintln("api: " + self.bind_failed + "; the program runs without its socket");
        }
    }
    @unbounded
    run() {
        if self.listen_fd < 0 { return; }
        while !self.draining {
            let conn = std::io::tcp::__accept_one(self.listen_fd);
            if conn < 0 { break; }
            let n = self.next_peer;
            self.next_peer = n + 1;
            __ApiPeer { peer: n, fd: conn };
        }
    }
    accept(c: __ApiPeer) { }
    release(c: __ApiPeer) { }
    dissolve() {
        if self.listen_fd < 0 { return; }
        std::io::tcp::__shutdown_listen_socket(self.listen_fd);
        std::io::tcp::__close_fd(self.listen_fd);
        std::io::fs::unlink(self.path) or discard;
    }
    fn reply(peer: Int, rid: Int, cid: String, ok: Bool, body: String) {
        __ApiReplyT <- __ApiReply { peer: peer, request_id: rid, client_id: cid, ok: ok, counted: false, body: body, as_of: "", caller: self.cur_caller, role: self.cur_role };
    }
    fn unauthorized(peer: Int, rid: Int, cid: String, role: String) {
        if self.unauthorized_drop { return; }
        self.reply(peer, rid, cid, false, __api_refusal_role(role));
    }
    fn refuse_gate(peer: Int, rid: Int, cid: String, subject: String) {
        if self.unauthorized_drop { return; }
        self.reply(peer, rid, cid, false, __api_refusal("unknown", subject));
    }
    fn on_reply_seen(r: __ApiReply) {
        if r.counted { self.in_flight = self.in_flight - 1; }
    }
    fn decode_refused(e: JsonError) {
        self.decode_failed = true;
        self.reply(self.cur_peer, self.cur_request, self.cur_client, false, __api_refusal("malformed", e.kind + ": " + e.field));
    }
    fn on_ingress(i: __ApiIngress) {
        let rid = self.next_request;
        self.next_request = rid + 1;
        self.cur_caller = i.caller;
        self.cur_role = "";
        if i.verb == "describe" {
            if i.subject == "full" {
                let g = self.__api_gate_owner(i.caller);
                if len(g) == 0 { self.unauthorized(i.peer, rid, i.client_id, "owner"); return; }
                self.cur_role = g;
                self.reply(i.peer, rid, i.client_id, true, "\"value\":" + self.describe_for(i.caller, true));
                return;
            }
            self.reply(i.peer, rid, i.client_id, true, "\"value\":" + self.describe_for(i.caller, false));
            return;
        }
        if i.verb == "watch" {
"#,
    );
    for s in &surface.streams {
        b.push_str(&format!(
            "            if i.subject == {} {{\n                {gate}self.reply(i.peer, rid, i.client_id, true, \"\\\"attached\\\":\\\"{}\\\"\"); return; }}\n",
            q(&s.name),
            s.name,
            gate = gate_src(&s.role)
        ));
    }
    for c in &surface.commands {
        if surface.streams.iter().any(|s| s.name == c.name) {
            continue;
        }
        b.push_str(&format!(
            "            if i.subject == {} {{ self.reply(i.peer, rid, i.client_id, false, __api_refusal(\"not_a_stream\", i.subject)); return; }}\n",
            q(&c.name)
        ));
    }
    b.push_str("            self.reply(i.peer, rid, i.client_id, false, __api_refusal(\"unknown\", i.subject));\n            return;\n        }\n");
    b.push_str("        if self.in_flight >= self.bound {\n            self.reply(i.peer, rid, i.client_id, false, __api_refusal(\"over_bound\", to_string(self.bound)));\n            return;\n        }\n");
    b.push_str("        self.cur_peer = i.peer;\n        self.cur_request = rid;\n        self.cur_client = i.client_id;\n        self.decode_failed = false;\n        if i.verb == \"call\" {\n");
    for c in &surface.commands {
        let m = mangle(&c.name);
        b.push_str(&format!(
            "            if i.subject == {} {{\n                {gate}self.__api_try_{}(i.peer, rid, i.client_id, i.body) or self.decode_refused(err);\n",
            q(&c.name),
            m,
            gate = gate_src(&c.role)
        ));
        if c.replier.is_some() {
            b.push_str("                if !self.decode_failed { self.in_flight = self.in_flight + 1; }\n");
        } else {
            b.push_str("                if !self.decode_failed { self.reply(i.peer, rid, i.client_id, true, \"\\\"accepted\\\":true\"); }\n");
        }
        b.push_str("                return;\n            }\n");
    }
    for s in &surface.streams {
        if surface.commands.iter().any(|c| c.name == s.name) {
            continue;
        }
        b.push_str(&format!(
            "            if i.subject == {} {{ self.reply(i.peer, rid, i.client_id, false, __api_refusal(\"not_a_command\", i.subject)); return; }}\n",
            q(&s.name)
        ));
    }
    b.push_str("            self.reply(i.peer, rid, i.client_id, false, __api_refusal(\"unknown\", i.subject));\n            return;\n        }\n");
    b.push_str("        if i.verb == \"read\" {\n");
    for r in &surface.reads {
        let m = mangle(&r.name).replace('.', "_");
        b.push_str(&format!(
            "            if i.subject == {} {{\n                {gate}__ApiReadT_{} <- __ApiRead {{ peer: i.peer, request_id: rid, client_id: i.client_id, caller: i.caller, role: self.cur_role }};\n                self.in_flight = self.in_flight + 1;\n                return;\n            }}\n",
            q(&r.name),
            m,
            gate = gate_src(&r.role)
        ));
    }
    b.push_str("            self.reply(i.peer, rid, i.client_id, false, __api_refusal(\"unknown\", i.subject));\n            return;\n        }\n    }\n");
    // Per-command decode-and-publish, and per-stream forwarders.
    for c in &surface.commands {
        let m = mangle(&c.name);
        b.push_str(&format!(
            "    fn __api_try_{m}(peer: Int, rid: Int, cid: String, body: String) fallible(JsonError) {{\n        let p = __api_decode_{p}(body) or raise;\n        __ApiCallT_{m} <- __ApiCall_{m} {{ peer: peer, request_id: rid, client_id: cid, caller: self.cur_caller, role: self.cur_role, {key}payload: p }};\n    }}\n",
            m = m,
            p = c.payload,
            key = match &c.key {
                Some((f, _)) => format!("key: p.{}, ", f),
                None => String::new(),
            }
        ));
    }
    for s in &surface.streams {
        b.push_str(&format!(
            "    fn __api_stream_{m}(p: {ty}) {{\n        __ApiFrameT <- __ApiFrame {{ subject: \"\\\"{n}\\\"\", body: __api_encode_{ty}(p) }};\n    }}\n",
            m = mangle(&s.name),
            ty = s.payload,
            n = s.name
        ));
    }
    // GH #1109: one gate per role. Holding the role, or any role that
    // `includes` it, authorizes; the answer is the role that did, so
    // the receipt and the handler's context name it.
    for r in &surface.roles {
        b.push_str(&format!("    fn __api_gate_{}(p: std::api::Principal) -> String {{\n", r.name));
        for g in grants(surface, &r.name) {
            b.push_str(&format!("        if self.roles.holds(p, {q}) {{ return {q}; }}\n", q = q(&g)));
        }
        b.push_str("        return \"\";\n    }\n");
    }
    // The description a caller is served: the pieces its roles show.
    // `full` (an owner's read) shows every piece, which is the
    // document `hale check --dump-api` emits, byte for byte.
    let parts = describe_parts(surface);
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (_, r) in parts.commands.iter().chain(parts.reads.iter()).chain(parts.streams.iter()) {
        if let Some(r) = r {
            used.insert(r.clone());
        }
    }
    for (_, rs) in &parts.schemas {
        used.extend(rs.iter().cloned());
    }
    b.push_str("    fn describe_for(p: std::api::Principal, full: Bool) -> String {\n");
    for r in &used {
        b.push_str(&format!(
            "        let h_{r}: Bool = full || len(self.__api_gate_{r}(p)) > 0;\n",
            r = r
        ));
    }
    let section = |b: &mut String, var: &str, items: &[(String, Vec<String>)]| {
        b.push_str(&format!("        let mut {} = \"\";\n", var));
        for (json, roles) in items {
            if roles.is_empty() {
                b.push_str(&format!("        {v} = __api_join({v}, {j});\n", v = var, j = q(json)));
            } else {
                let cond: Vec<String> = roles.iter().map(|r| format!("h_{}", r)).collect();
                b.push_str(&format!(
                    "        if {c} {{ {v} = __api_join({v}, {j}); }}\n",
                    c = cond.join(" || "),
                    v = var,
                    j = q(json)
                ));
            }
        }
    };
    let one = |v: &[(String, Option<String>)]| -> Vec<(String, Vec<String>)> {
        v.iter().map(|(j, r)| (j.clone(), r.iter().cloned().collect())).collect()
    };
    section(&mut b, "cmds", &one(&parts.commands));
    section(&mut b, "reads", &one(&parts.reads));
    section(&mut b, "streams", &one(&parts.streams));
    section(&mut b, "schemas", &parts.schemas);
    b.push_str(&format!(
        "        return {head} + cmds + \"],\\\"reads\\\":[\" + reads + \"],\\\"streams\\\":[\" + streams + \"],\\\"schemas\\\":{{\" + schemas + \"}}}}\";\n    }}\n",
        head = q(&parts.head)
    ));
    b.push_str("}\n");
    b
}

/// Envelope types and topics per command, and read topics.
fn envelopes_src(surface: &ApiSurface) -> String {
    let mut b = String::new();
    for c in &surface.commands {
        let m = mangle(&c.name);
        let key_field = match &c.key {
            Some((_, kt)) => format!(" key: {};", json_type_name(kt)),
            None => String::new(),
        };
        b.push_str(&format!(
            "type __ApiCall_{m} {{ peer: Int; request_id: Int; client_id: String; caller: std::api::Principal; role: String;{key} payload: {p}; }}\ntopic __ApiCallT_{m} {{ payload: __ApiCall_{m}; subject: \"__api.call.{m}\";{keyed} }}\n",
            m = m,
            key = key_field,
            p = c.payload,
            keyed = if c.key.is_some() { " keyed_by key;" } else { "" }
        ));
    }
    for r in &surface.reads {
        let m = mangle(&r.name).replace('.', "_");
        b.push_str(&format!(
            "topic __ApiReadT_{m} {{ payload: __ApiRead; subject: \"__api.read.{m}\"; }}\n",
            m = m
        ));
    }
    b
}

/// The members a subscriber locus gains for one command: the synthesized
/// subscription (cloned from the author's, so a key filter rides
/// along) and the thunk method.
fn subscriber_members(c: &ApiCommand, s: &ApiSubscriber, replies: bool) -> Result<(BusMember, Vec<LocusMember>), String> {
    let m = mangle(&c.name);
    let thunk = format!("__api_{}_{}", s.handler, m);
    let args = if s.takes_context {
        "r.payload, __api_context(r.caller, r.role, r.request_id)".to_string()
    } else {
        "r.payload".to_string()
    };
    let body = if replies {
        let ret = s.ret.as_ref().expect("replier has a return type");
        format!(
            "let v = self.{h}({args});\n        __ApiReplyT <- __ApiReply {{ peer: r.peer, request_id: r.request_id, client_id: r.client_id, ok: true, counted: true, body: \"\\\"value\\\":\" + {enc}, as_of: \"\", caller: r.caller, role: r.role }};",
            h = s.handler,
            args = args,
            enc = encode_expr(ret, "v")
        )
    } else {
        format!("self.{}({});", s.handler, args)
    };
    let src = format!(
        "locus __ApiTmp {{\n    fn {thunk}(r: __ApiCall_{m}) {{\n        {body}\n    }}\n}}\n",
        thunk = thunk,
        m = m,
        body = body
    );
    let members = parse_locus_members(&src)?;
    let mut member = s.member.clone();
    if let BusMember::Subscribe { subject, handler, ty, bound, .. } = &mut member {
        *subject = BusSubject::Topic(Ident {
            name: format!("__ApiCallT_{}", m),
            span: handler.span,
        });
        *handler = Ident {
            name: thunk,
            span: handler.span,
        };
        *ty = None;
        *bound = None;
    }
    Ok((member, members))
}

fn read_members(r: &ApiRead) -> Result<(BusMember, Vec<LocusMember>), String> {
    let m = mangle(&r.name).replace('.', "_");
    let access = if r.is_fn {
        format!("self.{}()", r.member)
    } else {
        format!("self.{}", r.member)
    };
    let src = format!(
        "locus __ApiTmp {{\n    bus {{ subscribe __ApiReadT_{m} as __api_read_{m}; }}\n    fn __api_read_{m}(r: __ApiRead) {{\n        let j = {enc};\n        __ApiReplyT <- __ApiReply {{ peer: r.peer, request_id: r.request_id, client_id: r.client_id, ok: true, counted: true, body: \"\\\"value\\\":\" + j, as_of: __api_digest(j), caller: r.caller, role: r.role }};\n    }}\n}}\n",
        m = m,
        enc = encode_expr(&r.ty, &access)
    );
    let mut members = parse_locus_members(&src)?;
    let bus_idx = members
        .iter()
        .position(|m| matches!(m, LocusMember::Bus(_)))
        .expect("bus block");
    let LocusMember::Bus(bb) = members.remove(bus_idx) else { unreachable!() };
    let sub = bb.members.into_iter().next().expect("one subscribe");
    Ok((sub, members))
}

fn parse_locus_members(src: &str) -> Result<Vec<LocusMember>, String> {
    let prog = crate::parse_source_at(src, API_SYNTH_BASE).map_err(|ds| {
        format!(
            "api_gen: generated source did not parse: {}\n{}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; "),
            src
        )
    })?;
    for item in prog.items {
        if let TopDecl::Locus(l) = item {
            return Ok(l.members);
        }
    }
    Err("api_gen: no locus in generated fragment".to_string())
}

fn publish_reply_member(span: Span) -> BusMember {
    BusMember::Publish {
        subject: BusSubject::Topic(Ident {
            name: "__ApiReplyT".to_string(),
            span,
        }),
        ty: None,
        alias: None,
        gated: None,
        span,
    }
}

/// Add bus members and methods to the named locus, wherever it lives.
fn extend_locus(
    programs: &mut [&mut Program],
    locus: &str,
    subs: Vec<BusMember>,
    methods: Vec<LocusMember>,
) {
    let mut done = false;
    for p in programs.iter_mut() {
        walk_items_mut(&mut p.items, &mut |item| {
            if done {
                return;
            }
            let TopDecl::Locus(l) = item else { return };
            if l.name.name != locus {
                return;
            }
            done = true;
            let span = l.name.span;
            let mut subs = subs.clone();
            let has_reply_publish = subs.iter().any(|m| matches!(m, BusMember::Publish { subject, .. } if subject.canonical() == "__ApiReplyT"));
            if !has_reply_publish && methods.iter().any(|_| true) && !l.members.iter().any(|m| {
                matches!(m, LocusMember::Bus(bb) if bb.members.iter().any(|x| matches!(x, BusMember::Publish { subject, .. } if subject.canonical() == "__ApiReplyT")))
            }) {
                subs.push(publish_reply_member(span));
            }
            if let Some(LocusMember::Bus(bb)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Bus(_))) {
                bb.members.extend(subs);
            } else {
                l.members.push(LocusMember::Bus(crate::ast::BusBlock { members: subs, span }));
            }
            l.members.extend(methods.clone());
        });
        if done {
            break;
        }
    }
}

/// Synthesize the api binding across a bundle. `programs` are the
/// seed's programs (one when merged); the generated top-level items
/// land beside the main locus. Idempotent: a bundle that already has
/// `__ApiBinding` is left alone. Returns the surface it emitted, or
/// `None` when no main locus carries an `api:` entry.
pub fn generate_api(programs: &mut [&mut Program], roles_table: Option<&str>) -> Option<ApiSurface> {
    let already = programs.iter().any(|p| {
        let mut found = false;
        walk_items(&p.items, &mut |i| {
            if matches!(i, TopDecl::Locus(l) if l.name.name == "__ApiBinding") {
                found = true;
            }
        });
        found
    });
    if already {
        return None;
    }
    let surface = {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        api_surface(&ro)?
    };
    // Which program holds the main locus.
    let main_idx = programs.iter().position(|p| {
        let mut found = false;
        walk_items(&p.items, &mut |i| {
            if matches!(i, TopDecl::Locus(l) if l.name.name == surface.main_locus) {
                found = true;
            }
        });
        found
    })?;

    let (path, bound, watch_bound, drop_old) = {
        let b = &surface.binding;
        let ApiTransport::Unix { path, .. } = &b.transport;
        let bound = b.bound.map(|(n, _)| n).unwrap_or(DEV_BOUND);
        let watch_bound = b.watch_bound.map(|(n, _)| n).unwrap_or(bound);
        let drop_old = !matches!(b.on_watch_full, Some((ShedPolicy::DropNew, _)));
        (path.clone(), bound, watch_bound, drop_old)
    };
    let path_span = surface.binding.transport_span();

    // JSON codecs for every type that reaches the binding, strict.
    json_gen::generate_api_codecs(programs, main_idx, &surface.json_types);

    // Top-level items beside the main locus.
    let mut src = common_src(watch_bound);
    src.push_str(&envelopes_src(&surface));
    src.push_str(&peer_src(&surface, drop_old));
    src.push_str(&binding_src(&surface, bound, roles_table));
    match crate::parse_source_at(&src, API_SYNTH_BASE) {
        Ok(generated) => programs[main_idx].items.extend(generated.items),
        Err(ds) => {
            // A generator bug, never user error; leave the program as
            // it was so the checker reports the entry as unlowered.
            eprintln!(
                "api_gen: generated source did not parse: {}",
                ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
            );
            return None;
        }
    }

    // Subscribers and readers gain their synthesized members.
    let mut per_locus: BTreeMap<String, (Vec<BusMember>, Vec<LocusMember>)> = BTreeMap::new();
    for c in &surface.commands {
        for (i, s) in c.subscribers.iter().enumerate() {
            let replies = c.replier == Some(i);
            match subscriber_members(c, s, replies) {
                Ok((sub, methods)) => {
                    let e = per_locus.entry(s.locus.clone()).or_default();
                    e.0.push(sub);
                    e.1.extend(methods);
                }
                Err(msg) => {
                    eprintln!("{}", msg);
                    return None;
                }
            }
        }
    }
    for r in &surface.reads {
        match read_members(r) {
            Ok((sub, methods)) => {
                let e = per_locus.entry(r.locus.clone()).or_default();
                e.0.push(sub);
                e.1.extend(methods);
            }
            Err(msg) => {
                eprintln!("{}", msg);
                return None;
            }
        }
    }
    for (locus, (subs, methods)) in per_locus {
        extend_locus(programs, &locus, subs, methods);
    }

    // The main locus holds the binding as its last param, on a pool
    // of its own.
    let param_src = format!(
        "main locus __ApiTmp {{\n    params {{ __api: __ApiBinding = __ApiBinding {{ bound: {} }}; }}\n    placement {{ __api: cooperative(pool = __api_io) where async_io; }}\n}}\n",
        bound
    );
    let members = match parse_locus_members(&param_src) {
        Ok(m) => m,
        Err(msg) => {
            eprintln!("{}", msg);
            return None;
        }
    };
    let mut new_param = None;
    let mut new_placement = None;
    for m in members {
        match m {
            LocusMember::Params(pb) => new_param = pb.params.into_iter().next(),
            LocusMember::Placement(pl) => new_placement = pl.entries.into_iter().next(),
            _ => {}
        }
    }
    let (mut new_param, new_placement) = (new_param?, new_placement?);
    // The socket path is the entry's expression, evaluated on the main
    // locus like every param default there (a literal, or `self.socket`
    // the program computed) — review B3.
    if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut new_param.init {
        inits.push(crate::ast::StructInit {
            name: Ident {
                name: "path".to_string(),
                span: path_span,
            },
            value: path,
            span: path_span,
        });
    }
    // GH #1109: the program's own membership source rides into the
    // binding as the expression the entry wrote, evaluated on the main
    // locus like every param default there (`self.roles` names a main
    // param; a literal builds the source) — review F6.
    if let Some(r) = &surface.binding.roles {
        if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut new_param.init {
            inits.push(crate::ast::StructInit {
                name: Ident {
                    name: "roles".to_string(),
                    span: r.span,
                },
                value: r.expr.clone(),
                span: r.span,
            });
        }
    }
    let main_name = surface.main_locus.clone();
    walk_items_mut(&mut programs[main_idx].items, &mut |item| {
        let TopDecl::Locus(l) = item else { return };
        if l.name.name != main_name {
            return;
        }
        let span = l.name.span;
        if let Some(LocusMember::Params(pb)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Params(_))) {
            pb.params.push(new_param.clone());
        } else {
            l.members.push(LocusMember::Params(crate::ast::ParamsBlock {
                params: vec![new_param.clone()],
                span,
            }));
        }
        if let Some(LocusMember::Placement(pl)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Placement(_))) {
            pl.entries.push(new_placement.clone());
        } else {
            l.members.push(LocusMember::Placement(PlacementBlock {
                entries: vec![new_placement.clone()],
                span,
            }));
        }
    });
    let _ = ParamInit::Inferred;
    Some(surface)
}
