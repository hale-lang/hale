//! Parent admission and release work, derived from shared source facts.
//! Construction context is neither parameter `self` nor failure supervision.
use std::collections::{BTreeMap, BTreeSet};

use hale_graph::ids::SiteId;
use hale_syntax::{ast::LifecycleKind, Span};

use crate::{
    alloc_summary::{AllocSummary, DeclId, EffectSite, EffectSiteKind, FnKey},
    lifecycle_hooks::LifecycleOwner,
    ownership_graph::{DefaultEvaluations, ExpandedLiteral, OtherPosition, OwnershipGraph},
    placement::SiteUniverse,
};

pub use hale_model::ProtocolStage;

pub fn stage_of(kind: LifecycleKind) -> Option<ProtocolStage> {
    match kind {
        LifecycleKind::Accept => Some(ProtocolStage::Accept),
        LifecycleKind::Release => Some(ProtocolStage::Release),
        _ => None,
    }
}

/// One real declaration; the child symbol is resolved by the declaration
/// inventory's alias/import resolver. None is not a wildcard admission.
#[derive(Clone, Debug)]
pub struct ProtocolHook {
    pub key: FnKey,
    pub stage: ProtocolStage,
    pub parameter: Option<DeclId>,
    pub child: Option<String>,
    pub interface: bool,
    pub empty: bool,
    pub synthesized: bool,
    pub span: Span,
}
pub type ProtocolHooks = BTreeMap<LifecycleOwner, Vec<ProtocolHook>>;

/// The root's source namespace. These are source-summary ordinals; a portable
/// model must project them through its own authored-site correspondence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtocolRoot {
    Construction(u32),
    Call(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolGap {
    InvocationContext,
    OwnerContext,
    SourceIdentity,
    DefaultContext,
    ForeignContext,
}
impl ProtocolGap {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvocationContext => "admission, release opportunity, scheduling, frame and retained lifetime remain unresolved",
            Self::OwnerContext => "the accepting owner or generic/interface child match is unresolved",
            Self::SourceIdentity => "the original constructor or triggering root is unavailable",
            Self::DefaultContext => "the inline default has no exact caller construction context",
            Self::ForeignContext => "parent protocol work lies beyond the source construction-context horizon",
        }
    }
}

/// A may-enter edge under the parent's admission/release opportunity. Defaults
/// attach here to their caller root, never to a shared default-body node.
#[derive(Clone, Debug)]
pub struct ProtocolEvaluation {
    /// Resolved constructed symbol, distinct from its generic template identity.
    pub child: String,
    pub target: FnKey,
    pub stage: ProtocolStage,
    pub owner: LifecycleOwner,
    pub root: ProtocolRoot,
    pub root_source: SiteId,
    pub constructor_function: FnKey,
    pub constructor: u32,
    pub constructor_source: SiteId,
    pub constructor_span: Span,
    pub span: Span,
    pub loop_depth: u32,
    pub in_unbounded_loop: bool,
}

#[derive(Clone)]
struct Root {
    function: FnKey,
    kind: ProtocolRoot,
    span: Span,
    loop_depth: u32,
    unbounded: bool,
}

/// Enrich one source summary once, after verified omission/default contexts
/// exist. No AST survey, plan, emitted artifact or model participates.
pub fn derive(
    base: &AllocSummary,
    graph: &OwnershipGraph,
    defaults: &DefaultEvaluations,
) -> AllocSummary {
    let mut out = base.clone();
    // Unminted/partial inputs cannot turn absent source joins into certainty.
    out.parent_execution_supplied = graph.declarations.iter().all(|d| d.id.is_some())
        && base.fns.values().all(|f| {
            f.key.decl.is_some()
                && f.constructions.iter().all(|c| c.site.is_some())
                && f.initializations.iter().all(|e| e.invocation.is_some())
        });
    if !base
        .protocol_hooks
        .values()
        .flatten()
        .any(|h| !h.synthesized)
    {
        return out;
    }
    let mut roots: BTreeMap<SiteId, Vec<Root>> = BTreeMap::new();
    let mut constructors = BTreeMap::new();
    for (key, body) in &base.fns {
        if key.decl.is_some_and(|d| d.universe != SiteUniverse::User) {
            continue;
        }
        for (ordinal, c) in body.constructions.iter().enumerate() {
            let Some(site) = c.site else { continue };
            constructors.insert(site, (key, ordinal as u32, c));
            if !base.initializers.contains_key(key) {
                roots.entry(site).or_default().push(Root {
                    function: key.clone(),
                    kind: ProtocolRoot::Construction(ordinal as u32),
                    span: c.span,
                    loop_depth: c.loop_depth,
                    unbounded: c.in_unbounded_loop,
                });
            }
        }
        if !base.initializers.contains_key(key) {
            for (ordinal, call) in body.calls.iter().enumerate() {
                if let Some(site) = call.authored {
                    roots.entry(site).or_default().push(Root {
                        function: key.clone(),
                        kind: ProtocolRoot::Call(ordinal as u32),
                        span: call.span,
                        loop_depth: call.loop_depth,
                        unbounded: call.in_unbounded_loop,
                    });
                }
            }
        }
    }
    let mut contexts = Vec::new();
    for (i, s) in graph
        .sites
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.params_default && s.arg_default.is_none())
    {
        contexts.push((
            ExpandedLiteral::Owned(i),
            Some(s.enclosing_decl),
            s.literal,
            false,
        ));
    }
    for (i, s) in graph
        .free_fn_sites
        .iter()
        .enumerate()
        .filter(|(_, s)| s.arg_default.is_none())
    {
        contexts.push((ExpandedLiteral::Free(i), None, s.literal, false));
    }
    for (i, s) in graph.binding_sites.iter().enumerate() {
        contexts.push((ExpandedLiteral::Binding(i), None, s.literal, false));
    }
    for (i, s) in graph.other_sites.iter().enumerate() {
        if s.position == OtherPosition::Closure {
            contexts.push((
                ExpandedLiteral::Other(i),
                s.enclosing_decl,
                s.site.literal,
                false,
            ));
        }
    }
    contexts.extend(
        defaults
            .rows
            .iter()
            .map(|e| (e.literal, e.context, e.root_site, e.per_use.is_some())),
    );
    // Record fields are per-use positions in the ownership walk. The
    // initializer relation already names each actual omitted-field use;
    // query those existing edges instead of treating all candidate callers
    // of a field declaration as actual roots or surveying its syntax again.
    let literals: BTreeMap<_, _> = graph
        .sites
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.literal.map(|id| (id, ExpandedLiteral::Owned(i))))
        .chain(
            graph
                .free_fn_sites
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.literal.map(|id| (id, ExpandedLiteral::Free(i)))),
        )
        .chain(
            graph
                .other_sites
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.site.literal.map(|id| (id, ExpandedLiteral::Other(i)))),
        )
        .chain(
            graph
                .binding_sites
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.literal.map(|id| (id, ExpandedLiteral::Binding(i)))),
        )
        .collect();
    for (source, root_rows) in &roots {
        for root in root_rows {
            let body = &base.fns[&root.function];
            let context = body
                .closure_owner
                .filter(|d| d.universe == SiteUniverse::User)
                .and_then(|d| {
                    graph
                        .declarations
                        .iter()
                        .position(|row| row.id.is_some_and(|id| id.index == d.index))
                });
            let Some(context) = context else { continue };
            let posted =
                |from: &crate::alloc_summary::FnSummary,
                 edge: &crate::alloc_summary::InitializationEdge| {
                    edge.initializer.kind == hale_model::InitializerKind::Parameter
                        && from
                            .constructions
                            .get(edge.ordinal as usize)
                            .and_then(|c| c.lifecycle_owner.as_ref())
                            .is_some_and(|child| {
                                defaults.posted.contains_key(&(
                                    graph.declarations[context].name.clone(),
                                    child.name.clone(),
                                ))
                            })
                };
            let mut pending: Vec<_> = body
                .initializations
                .iter()
                .filter(|edge| edge.invocation == Some(*source))
                .filter(|edge| match root.kind {
                    ProtocolRoot::Construction(site) => {
                        edge.ordinal == site
                            && edge.initializer.kind != hale_model::InitializerKind::Argument
                    }
                    ProtocolRoot::Call(site) => {
                        edge.ordinal == site
                            && edge.initializer.kind == hale_model::InitializerKind::Argument
                    }
                })
                .filter(|edge| !posted(body, edge))
                .map(|edge| {
                    (
                        &edge.target,
                        edge.initializer.kind == hale_model::InitializerKind::Field,
                    )
                })
                .collect();
            let mut entered = BTreeSet::new();
            while let Some((target, field_path)) = pending.pop() {
                if !entered.insert((target, field_path)) {
                    continue;
                }
                let Some(default) = base.fns.get(target) else {
                    continue;
                };
                if field_path {
                    for child in &default.constructions {
                        if let Some(literal) = child.site.and_then(|id| literals.get(&id)) {
                            contexts.push((*literal, Some(context), Some(*source), false));
                        }
                    }
                }
                pending.extend(
                    default
                        .initializations
                        .iter()
                        .filter(|edge| !posted(default, edge))
                        .map(|edge| {
                            (
                                &edge.target,
                                field_path
                                    || edge.initializer.kind == hale_model::InitializerKind::Field,
                            )
                        }),
                );
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut owner_cache = BTreeMap::new();
    let mut ancestry_cache = BTreeMap::<usize, BTreeSet<String>>::new();
    for (literal, context, root_source, per_use) in contexts {
        // A no-locus caller does not gain a parent through an ordinary call.
        let Some(context) = context else { continue };
        let s = graph.literal(literal);
        let Some(root_source) = root_source else {
            continue;
        };
        let Some(root_rows) = roots.get(&root_source) else {
            continue;
        };
        for root in root_rows {
            if !seen.insert((root.function.clone(), root.kind, s.literal, context)) {
                continue;
            }
            let child = s.child_key.as_deref();
            let (owners, uncertain) = match child {
                Some(child) => owner_cache
                    .entry((context, child.to_string()))
                    .or_insert_with(|| graph.protocol_owners(context, child))
                    .clone(),
                None => (Vec::new(), true),
            };
            let ctor = s.literal.and_then(|id| constructors.get(&id).copied());
            let mut gap = per_use.then_some(ProtocolGap::DefaultContext);
            if uncertain {
                gap = Some(ProtocolGap::OwnerContext);
            }
            if ctor.is_none() {
                gap = Some(ProtocolGap::SourceIdentity);
            }
            // Foreign children may not participate in this graph's accepts
            // table. Only matching hooks along this context's source ancestry
            // can create this boundary; an unrelated acceptor is irrelevant.
            let ancestry = ancestry_cache.entry(context).or_insert_with(|| {
                let mut names = BTreeSet::new();
                let mut pending = vec![graph.declarations[context].name.clone()];
                while let Some(name) = pending.pop() {
                    if !names.insert(name.clone()) {
                        continue;
                    }
                    pending.extend(
                        graph
                            .instantiated_by
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .cloned(),
                    );
                }
                names
            });
            if s.child_decl.is_none()
                && base
                    .protocol_hooks
                    .iter()
                    .filter(|(owner, _)| {
                        owner.universe == SiteUniverse::User && ancestry.contains(&owner.name)
                    })
                    .flat_map(|(_, hooks)| hooks)
                    .any(|h| {
                        !h.empty && !h.synthesized && (h.interface || h.child.as_deref() == child)
                    })
            {
                gap = Some(ProtocolGap::ForeignContext);
            }
            let body = out
                .fns
                .get_mut(&root.function)
                .expect("root belongs to source summary");
            if let Some(reason) = gap {
                body.effect_sites.push(EffectSite {
                    kind: EffectSiteKind::ProtocolInvocation {
                        target: None,
                        stage: None,
                        reason,
                    },
                    loop_depth: root.loop_depth,
                    span: root.span,
                });
            }
            let Some((constructor_function, constructor, c)) = ctor else {
                continue;
            };
            let Some(constructor_source) = c.site else {
                continue;
            };
            for owner_index in owners {
                let decl = &graph.declarations[owner_index];
                let Some(owner_id) = decl.id else { continue };
                let owner = LifecycleOwner {
                    universe: SiteUniverse::User,
                    declaration: Some(owner_id.index),
                    name: decl.name.clone(),
                };
                let Some(hooks) = base.protocol_hooks.get(&owner) else {
                    continue;
                };
                for hook in hooks.iter().filter(|h| !h.synthesized) {
                    // Both bookends must match the admitted child. A release
                    // declaration elsewhere never makes this parent's release.
                    let admitted = hook.child.as_deref() == child
                        || hook.interface
                            && graph
                                .accepts
                                .get(&decl.name)
                                .is_some_and(|a| child.is_some_and(|c| a.contains(c)));
                    if !admitted {
                        if hook.empty { continue; }
                        body.effect_sites.push(EffectSite {
                            kind: EffectSiteKind::ProtocolInvocation {
                                target: None,
                                stage: Some(hook.stage),
                                reason: ProtocolGap::OwnerContext,
                            },
                            loop_depth: root.loop_depth,
                            span: root.span,
                        });
                        continue;
                    }
                    if hook.stage == ProtocolStage::Accept {
                        body.purity_sites.push(crate::purity::PuritySite {
                            action: crate::purity::StateAction::ParentAdmission { owner: owner.name.clone(), child: child.unwrap().to_string() },
                            span: root.span,
                        });
                    }
                    if hook.empty { continue; }
                    body.effect_sites.push(EffectSite {
                        kind: EffectSiteKind::ProtocolInvocation {
                            target: Some(hook.key.clone()),
                            stage: Some(hook.stage),
                            reason: ProtocolGap::InvocationContext,
                        },
                        loop_depth: root.loop_depth.max(c.loop_depth),
                        span: root.span,
                    });
                    body.protocol_evaluations.push(ProtocolEvaluation {
                        child: child.expect("an admitted child has a resolved symbol").to_string(),
                        target: hook.key.clone(),
                        stage: hook.stage,
                        owner: owner.clone(),
                        root: root.kind,
                        root_source,
                        constructor_function: constructor_function.clone(),
                        constructor,
                        constructor_source,
                        constructor_span: c.span,
                        span: root.span,
                        loop_depth: root.loop_depth.max(c.loop_depth),
                        in_unbounded_loop: root.unbounded || c.in_unbounded_loop,
                    });
                }
            }
        }
    }
    out
}

/// Source identity projection used by native hook selection. No lowering-time
/// body survey or declaration-name fallback supplies an absent row.
#[derive(Clone, Debug)]
pub struct LoweredProtocolHook {
    pub declaration: hale_syntax::ast::NodeId,
    pub stage: ProtocolStage,
    pub empty: bool,
    pub synthesized: bool,
}
pub type LoweredProtocolHooks = BTreeMap<u32, Vec<LoweredProtocolHook>>;
pub fn lower(
    hooks: &ProtocolHooks,
    stdlib: impl Fn(hale_syntax::ast::NodeId) -> Option<hale_syntax::ast::NodeId>,
) -> Result<LoweredProtocolHooks, &'static str> {
    use hale_syntax::ast::NodeId;
    let node = |d: DeclId| match d.universe {
        SiteUniverse::User => Some(NodeId(d.index)),
        SiteUniverse::StdlibAnalysis => stdlib(NodeId(d.index)),
    };
    let mut out = BTreeMap::new();
    for (owner, hooks) in hooks {
        let declaration = owner
            .declaration
            .ok_or("a parent protocol owner has no source identity")?;
        let Some(owner) = node(DeclId {
            universe: owner.universe,
            index: declaration,
        }) else {
            continue;
        };
        let mut rows = Vec::new();
        for hook in hooks {
            let declaration = node(
                hook.key
                    .decl
                    .ok_or("a parent protocol hook has no source identity")?,
            )
            .ok_or("a merged parent lost its protocol declaration correspondence")?;
            rows.push(LoweredProtocolHook {
                declaration,
                stage: hook.stage,
                empty: hook.empty,
                synthesized: hook.synthesized,
            });
        }
        if out.insert(owner.0, rows).is_some() {
            return Err("parent protocol owner correspondence is ambiguous");
        }
    }
    Ok(out)
}
