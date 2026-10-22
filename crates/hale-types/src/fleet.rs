//! Phase 4 (C2): the fleet locus, checked.
//!
//! `fleet locus NAME { params { … } placement { … } bindings { … }
//! claims { … } }` declares a deployment of other seeds' `main` loci.
//! It is a locus KIND, not a typed param: a fleet is checked as one
//! graph (the routes between its instances, the claims across them) and
//! never runs itself, where a param is a thing a running program holds.
//!
//! This module reads the fleet out of a checked bundle: the laws a fleet
//! seed obeys ([`fleet_laws`]), and the fleet as resolved source
//! ([`fleet_source`]), which the `deployment` family's producer
//! (`hale-cli`'s `deployment::derive_deployment`) joins with each
//! instance's artifact to derive the deployment's rows. The bundle's
//! own analyses never see a fleet: it holds no code, and the seeds it
//! names are composed as artifacts, never merged as source
//! (spec/verification.md § Fleet composition).

use hale_syntax::ast::{
    flat_decls, ClaimForm, ClaimSet, CountCmp, FleetDecl, FleetPlacementSpec, LocusDecl, ParamInit,
    Program, TopDecl, TopicDecl, TransportSpec, TypeExpr,
};
use hale_syntax::{Diag, Span};

use crate::resolve::TopScope;
use crate::symbol::{Bundle, TopSymbol};

/// A fleet as resolved source: its instances with the main each runs
/// and its placement default, its routes' topics with their wire
/// subjects, and its claims over instance groups.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetSource {
    pub name: String,
    pub instances: Vec<FleetInstance>,
    pub bindings: Vec<FleetBinding>,
    pub claims: Vec<FleetClaim>,
}

/// One param of the fleet: an instance, or `replicas` of them, of
/// another seed's `main locus`.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetInstance {
    /// The param's name: the instance's id, or the stem of its
    /// replicas' ids, and the label its claims group it by.
    pub param: String,
    /// The import alias the param's type is spelled through (`oms` in
    /// `oms::Oms`): which seed the instance is built from.
    pub alias: String,
    /// The main locus as its importer spells it (`oms::Oms`).
    pub main: String,
    pub host: Option<String>,
    pub replicas: u64,
}

/// One binding: the topic the fleet wires its instances by, its wire
/// subject, and the route's transport.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetBinding {
    /// The topic as written (`t::OrderRequest`).
    pub topic: String,
    pub subject: String,
    /// `unix`, `shm_ring`, or the adapter locus as written.
    pub transport: String,
}

/// A claim across the fleet's instances, each group an instance param.
#[derive(Debug, Clone, PartialEq)]
pub enum FleetClaim {
    ForbidReaches {
        name: String,
        from: String,
        to: String,
        avoiding: Option<String>,
    },
    OnlyEdges {
        name: String,
        from: String,
        to: String,
        grant_subjects: Vec<String>,
    },
    Require {
        name: String,
        publishes: bool,
        group: String,
        subject: String,
    },
    Count {
        name: String,
        publishers: bool,
        subject: String,
        cmp: CountCmp,
        n: u64,
    },
}

/// Every fleet the bundle declares, with the program that holds it.
fn fleets<'a>(bundle: &'a Bundle<'_>) -> Vec<&'a FleetDecl> {
    let mut out = Vec::new();
    for p in bundle.programs.values() {
        for d in flat_decls(&p.items) {
            if let TopDecl::Fleet(f) = d {
                out.push(f);
            }
        }
    }
    out
}

fn locus_named<'a>(bundle: &'a Bundle<'_>, name: &str) -> Option<&'a LocusDecl> {
    bundle
        .programs
        .values()
        .flat_map(|p: &&Program| flat_decls(&p.items))
        .find_map(|d| match d {
            TopDecl::Locus(l) if l.name.name == name => Some(l),
            _ => None,
        })
}

fn topic_named<'a>(bundle: &'a Bundle<'_>, name: &str) -> Option<&'a TopicDecl> {
    bundle
        .programs
        .values()
        .flat_map(|p: &&Program| flat_decls(&p.items))
        .find_map(|d| match d {
            TopDecl::Topic(t) if t.name.name == name => Some(t),
            _ => None,
        })
}

/// The laws a fleet seed obeys. Empty for a bundle with no fleet.
pub fn fleet_laws(
    bundle: &Bundle<'_>,
    top: &TopScope,
    entry: &crate::entry::EntryRow,
) -> Vec<Diag> {
    fleet_source(bundle, top, entry).1
}

/// The bundle's fleet as resolved source, with the diagnostics of every
/// law it breaks. `None` when the bundle declares no fleet, or when a
/// law is broken (the diagnostics say which).
pub fn fleet_source(
    bundle: &Bundle<'_>,
    top: &TopScope,
    entry: &crate::entry::EntryRow,
) -> (Option<FleetSource>, Vec<Diag>) {
    let all = fleets(bundle);
    let Some(fl) = all.first().copied() else {
        return (None, Vec::new());
    };
    let mut diags = Vec::new();
    for extra in &all[1..] {
        diags.push(Diag::ty(
            extra.name.span,
            format!(
                "a seed declares one fleet: `fleet locus {}` is a second beside `{}`; a deployment \
                 that differs is a seed of its own",
                extra.name.name, fl.name.name
            ),
        ));
    }
    // A fleet never runs: its seed starts nothing.
    for m in entry.own() {
        diags.push(Diag::ty(
            m.span,
            format!(
                "`main locus {}` beside `fleet locus {}`: a fleet seed never runs, its instances \
                 do, each from its own seed",
                m.name, fl.name.name
            ),
        ));
    }
    for p in bundle.programs.values() {
        for d in flat_decls(&p.items) {
            if let TopDecl::Fn(f) = d {
                if f.name.name == "main" {
                    diags.push(Diag::ty(
                        f.name.span,
                        format!(
                            "`fn main` beside `fleet locus {}`: a fleet seed never runs, its \
                             instances do, each from its own seed",
                            fl.name.name
                        ),
                    ));
                }
            }
        }
    }

    // The instances: each param an imported seed's `main locus`.
    let mut instances: Vec<FleetInstance> = Vec::new();
    for p in &fl.params.params {
        if instances.iter().any(|i| i.param == p.name.name) {
            diags.push(Diag::ty(
                p.name.span,
                format!("fleet param `{}` is declared twice", p.name.name),
            ));
            continue;
        }
        if let ParamInit::Value(e) = &p.init {
            diags.push(Diag::ty(
                e.span(),
                format!(
                    "fleet param `{}` takes no value: an instance is built from its own seed, as \
                     `hale build` builds it",
                    p.name.name
                ),
            ));
            continue;
        }
        let written = fl.written.iter().find(|(n, _, _)| *n == p.name.name);
        let resolved = match &p.ty {
            Some(TypeExpr::Named { path, .. }) if path.segments.len() == 1 => {
                locus_named(bundle, &path.segments[0].name)
            }
            _ => None,
        };
        let spelled = match written {
            Some((_, a, m)) => format!("{a}::{m}"),
            None => match &p.ty {
                Some(TypeExpr::Named { path, .. }) => path
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join("::"),
                Some(t) => t.form_name().to_string(),
                None => "no type".to_string(),
            },
        };
        let span = p.ty.as_ref().map(|t| t.span()).unwrap_or(p.name.span);
        match (written, resolved) {
            (Some((_, alias, _)), Some(l)) if l.is_main && l.imported => {
                instances.push(FleetInstance {
                    param: p.name.name.clone(),
                    alias: alias.clone(),
                    main: l.display.clone().unwrap_or_else(|| spelled.clone()),
                    host: None,
                    replicas: 1,
                });
            }
            (_, Some(l)) if l.imported => diags.push(Diag::ty(
                span,
                format!(
                    "fleet param `{}` names `{}`, which is not its seed's `main locus`: a fleet's \
                     instances are other seeds' mains",
                    p.name.name, spelled
                ),
            )),
            (_, Some(_)) => diags.push(Diag::ty(
                span,
                format!(
                    "fleet param `{}` names `{}`, a locus of this seed: a fleet's instances are \
                     other seeds' mains, named through the seed's import (`alias::Main`)",
                    p.name.name, spelled
                ),
            )),
            (_, None) => diags.push(Diag::ty(
                span,
                format!(
                    "fleet param `{}` names `{}`, which no import declares: a fleet's instances \
                     are other seeds' mains, named through the seed's import (`alias::Main`)",
                    p.name.name, spelled
                ),
            )),
        }
    }
    if fl.params.params.is_empty() {
        diags.push(Diag::ty(
            fl.name.span,
            format!(
                "fleet locus `{}` has no instances: its `params` block is empty",
                fl.name.name
            ),
        ));
    }

    // Placement: one default per instance.
    let mut placed: Vec<&str> = Vec::new();
    for e in &fl.placement {
        let name = e.instance.name.as_str();
        if placed.contains(&name) {
            diags.push(Diag::ty(
                e.span,
                format!("instance `{name}` is placed twice"),
            ));
            continue;
        }
        placed.push(name);
        let Some(inst) = instances.iter_mut().find(|i| i.param == name) else {
            if !fl.params.params.iter().any(|p| p.name.name == name) {
                diags.push(Diag::ty(
                    e.instance.span,
                    format!(
                        "placement names `{name}`, which is not a param of fleet locus `{}`",
                        fl.name.name
                    ),
                ));
            }
            continue;
        };
        match &e.spec {
            FleetPlacementSpec::Host(h) if h.trim().is_empty() => diags.push(Diag::ty(
                e.span,
                format!("instance `{name}`: `host(\"\")` names no host"),
            )),
            FleetPlacementSpec::Host(h) => inst.host = Some(h.clone()),
            FleetPlacementSpec::Replicas(0) => diags.push(Diag::ty(
                e.span,
                format!("instance `{name}`: `replicas(0)` deploys nothing; drop the param instead"),
            )),
            FleetPlacementSpec::Replicas(n) => inst.replicas = *n,
        }
    }

    // Bindings: each a topic with a declared subject, and the route's
    // transport.
    let subject_of = |name: &str| -> Option<(String, Option<String>)> {
        match top.lookup(name) {
            Some(TopSymbol::Topic(t)) => {
                let declared = topic_named(bundle, name).and_then(|d| d.subject.clone());
                Some((t.wire_subject.clone(), declared))
            }
            _ => None,
        }
    };
    let display_of = |name: &str| {
        topic_named(bundle, name)
            .and_then(|d| d.display.clone())
            .unwrap_or(name.to_string())
    };
    let mut bindings: Vec<FleetBinding> = Vec::new();
    if let Some(bb) = &fl.bindings {
        for h in &bb.hubs {
            diags.push(Diag::ty(
                h.span,
                "a fleet binding wires instances by a topic; a hub is a program's own stream"
                    .to_string(),
            ));
        }
        for b in &bb.entries {
            let topic = display_of(&b.topic.name);
            if !b.constraints.is_empty() || b.codec.is_some() {
                diags.push(Diag::ty(
                    b.span,
                    format!(
                        "fleet binding `{topic}` names its topic and its route's transport; \
                         constraints and codecs are the instances' own bindings"
                    ),
                ));
            }
            let Some((wire, declared)) = subject_of(&b.topic.name) else {
                diags.push(Diag::ty(
                    b.topic.span,
                    format!("fleet binding names unknown topic `{topic}`"),
                ));
                continue;
            };
            if declared.is_none() {
                diags.push(Diag::ty(
                    b.topic.span,
                    format!(
                        "fleet binding `{topic}`: the topic declares no `subject:`, so its wire name \
                         is local to each program and cannot join two instances"
                    ),
                ));
                continue;
            }
            if bindings.iter().any(|x| x.subject == wire) {
                diags.push(Diag::ty(
                    b.span,
                    format!(
                        "subject `{wire}` is bound twice in fleet locus `{}`",
                        fl.name.name
                    ),
                ));
                continue;
            }
            let transport = match &b.transport {
                TransportSpec::Unix { .. } => "unix".to_string(),
                TransportSpec::ShmRing { .. } => "shm_ring".to_string(),
                TransportSpec::Adapter { locus, .. } => {
                    format!("adapter:{}", display_of(&locus.name))
                }
            };
            bindings.push(FleetBinding {
                topic,
                subject: wire,
                transport,
            });
        }
    }

    // Claims: the fleet tier's forms, each group an instance.
    let mut claims: Vec<FleetClaim> = Vec::new();
    if let Some(cb) = &fl.claims {
        for a in &cb.adopts {
            diags.push(Diag::ty(
                a.span,
                "a constitution is adopted by a program's main; a fleet states its own claims"
                    .to_string(),
            ));
        }
        let is_instance = |g: &str| fl.params.params.iter().any(|p| p.name.name == g);
        for c in &cb.entries {
            let name = c.name.name.clone();
            let mut groups: Vec<&hale_syntax::ast::Ident> = c.form.group_operands();
            groups.retain(|g| !is_instance(&g.name));
            for g in groups {
                diags.push(Diag::ty(
                    g.span,
                    format!(
                        "fleet claim `{name}` names `{}`, which is not an instance of fleet locus `{}`: \
                         a fleet claim's groups are its params",
                        g.name, fl.name.name
                    ),
                ));
            }
            let topic_subject = |segs: &[hale_syntax::ast::Ident],
                                 span: Span,
                                 diags: &mut Vec<Diag>| {
                let joined = segs
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join("::");
                match subject_of(&joined) {
                    Some((w, Some(_))) => Some(w),
                    _ => {
                        diags.push(Diag::ty(
                            span,
                            format!("fleet claim `{name}` names `{joined}`, which is no topic with a declared `subject:`"),
                        ));
                        None
                    }
                }
            };
            match &c.form {
                ClaimForm::ForbidReaches { src: ClaimSet::Group(from), dst: ClaimSet::Group(to), via_calls: true, via_bus: true, during: None, avoiding } => {
                    claims.push(FleetClaim::ForbidReaches {
                        name,
                        from: from.name.clone(),
                        to: to.name.clone(),
                        avoiding: avoiding.as_ref().map(|a| a.name.clone()),
                    })
                }
                ClaimForm::OnlyEdges { src, dst, grants } => {
                    let mut subjects = Vec::new();
                    for g in grants {
                        if let Some(s) = topic_subject(&g.topic.segments, g.topic.span, &mut diags) {
                            subjects.push(s);
                        }
                    }
                    claims.push(FleetClaim::OnlyEdges { name, from: src.name.clone(), to: dst.name.clone(), grant_subjects: subjects })
                }
                ClaimForm::Require { publishers, group, topic } => {
                    if let Some(subject) = topic_subject(&topic.segments, topic.span, &mut diags) {
                        claims.push(FleetClaim::Require { name, publishes: *publishers, group: group.name.clone(), subject })
                    }
                }
                ClaimForm::Count { publishers, topic, cmp, n } => {
                    if let Some(subject) = topic_subject(&topic.segments, topic.span, &mut diags) {
                        claims.push(FleetClaim::Count { name, publishers: *publishers, subject, cmp: *cmp, n: *n })
                    }
                }
                _ => diags.push(Diag::ty(
                    c.span,
                    format!(
                        "fleet claim `{name}`: across a fleet a claim is `forbid reaches(A, B) [avoiding G]` \
                         over instances, `only edges`, `require publishes|subscribes` or `count`; \
                         the other forms quantify inside one program and belong in its main"
                    ),
                )),
            }
        }
    }

    if !diags.is_empty() {
        return (None, diags);
    }
    (
        Some(FleetSource {
            name: fl.name.name.clone(),
            instances,
            bindings,
            claims,
        }),
        diags,
    )
}
