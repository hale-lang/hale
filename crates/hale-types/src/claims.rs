//! GH #382 — claims: named, bundle-level sentences over the program
//! graph.
//!
//! Every structural proof the compiler performs is one judgment form:
//! over a graph derived from source, evaluate a property, and on
//! failure produce a witness. This module makes that layer a
//! user-facing surface: `group` declarations are the vocabulary,
//! `claims { }` on the main locus holds the sentences, and a
//! violation renders a minimal countermodel in author spelling.
//!
//! The verbs (#382 build order, phases 1–5):
//!   - `forbid reaches(A, B) [via {calls, bus}] [during P]
//!     [avoiding G]` — absence under the composed closure;
//!   - `only edges A -> B { publish T; … }` — isolation with an
//!     exhaustive grant enumeration (a grant is a reviewable line);
//!   - `bound C <= N on paths from G` — the `@budget` semiring (sum
//!     along paths, max at joins) over a user effect class;
//!   - `require subscribes/publishes(some G, topic T)` — existence;
//!   - `cover topic in seed(a): subscribed_by(some G)` — bounded
//!     universal over a seed's declared topics;
//!   - `count publishers/subscribers(topic T) ==/<=/>= N` — the
//!     cardinality family (single-writer topics).
//!
//! Soundness posture, inherited from #265/#353/#354:
//!   - **Unknown name = error, not empty set.** Groups, classes,
//!     topics, phases: a reference that resolves to nothing is the
//!     misspelt-effect-class bug wearing new clothing.
//!   - **Empty domain = vacuity error** unless explicitly opted out
//!     (`may_be_empty` on groups; a `cover` over a topic-less seed
//!     is always an error). A sentence trivially satisfied by an
//!     empty quantification domain is a fail-open in formal
//!     clothing.
//!   - **Unknown ⇒ violation.** An indirect call (fn-typed param)
//!     or a computed publish subject on a relevant path cannot be
//!     certified and is reported, exactly as `@no_syscall` treats
//!     the same shapes. Over-approximation only ever adds edges.
//!
//! Claims are ERRORS gating `hale check`, never advisories: an
//! advisory claim reads as law and doesn't bind — the #354 fail-open
//! shape. Weakening a claim is a source diff (delete the `forbid`,
//! widen the grant list), which is the review event this surface
//! exists to create.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hale_syntax::ast::*;
use hale_syntax::Diag;

use crate::alloc_summary::{AllocSummary, Callee, EffectSiteKind, FnKey};
use crate::bus_graph::BusGraph;
use crate::callgraph;
use crate::effects::{
    close, declared_of, defs_of, effect_names_of, ffi_names,
};
use crate::stdlib_surface::{self, EffectSet};

/// One evaluated claim, for the topology artifact (#382 phase 2).
#[derive(Debug, Clone)]
pub struct ClaimOutcome {
    pub name: String,
    /// The normalized sentence, rendered.
    pub form: String,
    /// `"holds"`, `"violated"`, or `"invalid"` (a vocabulary /
    /// reference error prevented evaluation).
    pub result: &'static str,
}

/// Diagnostics only — the `hale check` entry point.
pub fn claims_diags(
    programs: &[&Program],
    graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> Vec<Diag> {
    claims_report(programs, graph, import_renames).0
}

/// Diagnostics plus per-claim outcomes (the artifact's rows).
/// Demangles cross-seed symbols in the diagnostics so witnesses name
/// what the author wrote.
pub fn claims_report(
    programs: &[&Program],
    graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> (Vec<Diag>, Vec<ClaimOutcome>) {
    let (mut out, outcomes) =
        claims_report_inner(programs, graph, import_renames);
    crate::stdlib_bodies::demangle_imports(&mut out, import_renames);
    (out, outcomes)
}

/// A resolved group: the decls it names, projected to the fn grain.
///
/// Groups name DECLARED PROGRAM ELEMENTS (loci, free fns); each claim
/// projects them onto the sorts its relation needs. `reaches`
/// evaluates over the fn-grained path graph — a locus member projects
/// to all of its methods, lifecycle hooks, and modes (everything the
/// summary holds under its name), which only ever ADDS sources and
/// sinks: the conservative direction.
struct ResolvedGroup {
    /// Number of decls (not fns) the group resolved to — vacuity is
    /// judged at the decl grain. A group naming one fn-less locus is
    /// non-empty even though its fn projection is.
    decl_count: usize,
    may_be_empty: bool,
    /// Locus names in the group (for projection + display).
    loci: BTreeSet<String>,
    /// Free fns in the group.
    free_fns: BTreeSet<String>,
}

impl ResolvedGroup {
    /// Project to the fn grain against the bundle summary.
    fn fn_set(&self, summary: &AllocSummary) -> BTreeSet<FnKey> {
        let mut out: BTreeSet<FnKey> = BTreeSet::new();
        for k in summary.fns.keys() {
            match &k.locus {
                Some(l) if self.loci.contains(l) => {
                    out.insert(k.clone());
                }
                None if self.free_fns.contains(&k.fn_name) => {
                    out.insert(k.clone());
                }
                _ => {}
            }
        }
        // A free fn with no summary entry (empty body) still exists
        // as a source/sink decl.
        for f in &self.free_fns {
            out.insert(FnKey::free_fn(f.clone()));
        }
        out
    }

    /// Is this fn a member, at the decl grain?
    fn contains_fn(&self, k: &FnKey) -> bool {
        match &k.locus {
            Some(l) => self.loci.contains(l),
            None => self.free_fns.contains(&k.fn_name),
        }
    }

    fn is_empty(&self) -> bool {
        self.loci.is_empty() && self.free_fns.is_empty()
    }
}

/// Projection vacuity: vocabulary-nonempty (the group names decls)
/// is not the same as relation-projection-nonempty (this claim has
/// executable vertices). A group of fn-less loci — pure-data stores
/// — passes the decl-grain vacuity guard but projects to nothing
/// the fn-grained walk can see, so a `forbid`/`bound`/`only edges`
/// over it proves nothing while reading as law. Fail closed.
fn projection_vacuity(
    c: &ClaimDecl,
    which: &str,
    name: &Ident,
    g: &ResolvedGroup,
    summary: &AllocSummary,
    diags: &mut Vec<Diag>,
) -> bool {
    if g.decl_count == 0 || !g.fn_set(summary).is_empty() {
        return false;
    }
    diags.push(Diag::ty(
        name.span,
        format!(
            "claim `{}`: group `{}` projects to no executable {} \
             vertices — its declarations have no fns, so the claim \
             proves nothing about them. The fn-grained walk cannot \
             see pure-data access; name the loci that HOLD the \
             behavior, or drop the claim",
            c.name.name, name.name, which
        ),
    ));
    true
}

/// Everything the evaluators share.
struct Cx<'a> {
    groups: BTreeMap<String, ResolvedGroup>,
    topic_names: BTreeSet<String>,
    /// Import alias -> topic decls of that seed (mangled names).
    alias_topics: BTreeMap<String, Vec<String>>,
    summary: AllocSummary,
    graph: &'a BusGraph,
    ffi: BTreeSet<String>,
    defs: Vec<Option<Vec<EffectClass>>>,
    declared: BTreeSet<u16>,
    effect_names: Vec<String>,
    /// #392: the normalized model — decl provenance (witness spans,
    /// origin gating), the phase relation (`during`), the seed sort.
    model: crate::model::Model,
}

fn claims_report_inner(
    programs: &[&Program],
    graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> (Vec<Diag>, Vec<ClaimOutcome>) {
    // ---- collect group decls + claims blocks (modules included) ----
    //
    // #392 thread 2 — two tiers. Main-locus blocks are the WORLD
    // tier (bundle-wide law, as shipped). A TOP-LEVEL `claims { }`
    // block is the LIBRARY tier: a seed swears about itself and its
    // own boundary, the block travels with the import, and it
    // re-evaluates here — in the closing build's merged world —
    // every time. Library claims report with seed attribution
    // (`alias::name`), never as mangled symbols. A program that
    // declares `main locus` may not use the top-level form: world
    // law belongs in main (the closed-world gate), and the tier
    // split is what keeps a dependency from stating world-claims.
    let mut diags: Vec<Diag> = Vec::new();
    let mut group_decls: Vec<&GroupDecl> = Vec::new();
    let mut claims: Vec<ClaimDecl> = Vec::new();
    let mangled_to_alias: BTreeMap<&str, &str> = import_renames
        .iter()
        .filter_map(|(segs, mangled)| {
            segs.first().map(|a| (mangled.as_str(), a.as_str()))
        })
        .collect();
    fn walk_items<'a>(
        items: &'a [TopDecl],
        groups: &mut Vec<&'a GroupDecl>,
        claims: &mut Vec<ClaimDecl>,
        top_blocks: &mut Vec<&'a ClaimsBlock>,
        has_main: &mut bool,
    ) {
        for item in items {
            match item {
                TopDecl::Group(g) => groups.push(g),
                TopDecl::Locus(l) if l.is_main => {
                    *has_main = true;
                    for m in &l.members {
                        if let LocusMember::Claims(cb) = m {
                            claims.extend(cb.entries.iter().cloned());
                        }
                    }
                }
                TopDecl::Claims(cb) => top_blocks.push(cb),
                TopDecl::Module(m) => walk_items(
                    &m.items,
                    groups,
                    claims,
                    top_blocks,
                    has_main,
                ),
                _ => {}
            }
        }
    }
    let mut top_blocks: Vec<&ClaimsBlock> = Vec::new();
    let mut has_main = false;
    for p in programs {
        walk_items(
            &p.items,
            &mut group_decls,
            &mut claims,
            &mut top_blocks,
            &mut has_main,
        );
    }
    for cb in top_blocks {
        // The seed merge flattens imported items into the closing
        // program, so position cannot tell the tiers apart — the
        // mangle stage's `lib_tier` marker can: it only ever
        // touches imported seeds. An unmarked top-level block in a
        // bundle that closes (declares main) is the closing seed's
        // own — the world-claim surface the tier split forbids.
        if !cb.lib_tier && has_main {
            diags.push(Diag::ty(
                cb.span,
                "a top-level `claims { }` block is the LIBRARY \
                 tier — a seed swearing about itself and its own \
                 boundary. This seed declares `main locus`, the \
                 closed-world gate: state world law inside it"
                    .to_string(),
            ));
            continue;
        }
        // Attribution: the block's own group/topic refs were
        // canonicalized to mangled decl names, which map back to
        // the import alias — so a library claim reports as
        // `alias::name`, never as a mangled symbol.
        let alias = cb.entries.iter().find_map(|e| {
            let name = match &e.form {
                ClaimForm::ForbidReaches { src, .. } => match src {
                    ClaimSet::Group(g) => Some(&g.name),
                    _ => None,
                },
                ClaimForm::OnlyEdges { src, .. } => Some(&src.name),
                ClaimForm::Bound { from, .. } => Some(&from.name),
                ClaimForm::Require { group, .. }
                | ClaimForm::Cover { group, .. } => Some(&group.name),
                ClaimForm::Count { topic, .. } => {
                    topic.segments.first().map(|s| &s.name)
                }
            }?;
            mangled_to_alias.get(name.as_str()).copied()
        });
        for e in &cb.entries {
            let mut c = e.clone();
            if let Some(a) = alias {
                c.name.name = format!("{}::{}", a, c.name.name);
            }
            claims.push(c);
        }
    }
    if group_decls.is_empty() && claims.is_empty() {
        return (diags, Vec::new());
    }

    // ---- decl indexes for member / topic resolution ----
    let mut locus_names: BTreeSet<String> = BTreeSet::new();
    let mut free_fn_names: BTreeSet<String> = BTreeSet::new();
    let mut topic_names: BTreeSet<String> = BTreeSet::new();
    fn index_items(
        items: &[TopDecl],
        loci: &mut BTreeSet<String>,
        fns: &mut BTreeSet<String>,
        topics: &mut BTreeSet<String>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    loci.insert(l.name.name.clone());
                }
                TopDecl::Fn(f) => {
                    fns.insert(f.name.name.clone());
                }
                TopDecl::Topic(t) => {
                    topics.insert(t.name.name.clone());
                }
                TopDecl::Module(m) => {
                    index_items(&m.items, loci, fns, topics)
                }
                _ => {}
            }
        }
    }
    for p in programs {
        index_items(
            &p.items,
            &mut locus_names,
            &mut free_fn_names,
            &mut topic_names,
        );
    }
    let mut alias_topics: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (key, mangled) in import_renames {
        if key.len() == 2 && topic_names.contains(mangled) {
            alias_topics
                .entry(key[0].clone())
                .or_default()
                .push(mangled.clone());
        }
    }

    // ---- resolve groups ----
    let mut groups: BTreeMap<String, ResolvedGroup> = BTreeMap::new();
    for g in &group_decls {
        if groups.contains_key(&g.name.name) {
            diags.push(Diag::ty(
                g.name.span,
                format!(
                    "group `{}` is declared more than once",
                    g.name.name
                ),
            ));
            continue;
        }
        let mut rg = ResolvedGroup {
            decl_count: 0,
            may_be_empty: g.may_be_empty,
            loci: BTreeSet::new(),
            free_fns: BTreeSet::new(),
        };
        for m in &g.members {
            resolve_member(
                m,
                &locus_names,
                &free_fn_names,
                import_renames,
                &mut rg,
                &mut diags,
            );
        }
        // Vacuity: judged at the DECL grain. A `forbid` over an
        // empty domain holds trivially — fail-open — so an empty
        // group must say so explicitly.
        if rg.decl_count == 0 && !rg.may_be_empty {
            diags.push(Diag::ty(
                g.span,
                format!(
                    "group `{}` resolves to no declarations. A claim \
                     quantifying over an empty group holds vacuously — \
                     if the group may legitimately be empty, say \
                     `may_be_empty`",
                    g.name.name
                ),
            ));
        }
        groups.insert(g.name.name.clone(), rg);
    }

    if claims.is_empty() {
        return (diags, Vec::new());
    }

    // ---- claim names are the contract-of-record ----
    let mut seen_names: BTreeMap<&str, hale_syntax::Span> =
        BTreeMap::new();
    for c in &claims {
        if seen_names.insert(&c.name.name, c.name.span).is_some() {
            diags.push(Diag::ty(
                c.name.span,
                format!(
                    "claim `{}` is declared more than once — the name \
                     is the contract-of-record and must be unique",
                    c.name.name
                ),
            ));
        }
    }

    let cx = Cx {
        groups,
        topic_names,
        alias_topics,
        // Same substrate as every shipped certificate: the bundle
        // summary with stdlib bodies merged, so a chain through a
        // stdlib locus method resolves instead of stopping at the
        // boundary.
        summary: crate::stdlib_bodies::summarize_with_stdlib_and_renames(
            programs,
            import_renames,
        ),
        graph,
        ffi: ffi_names(programs),
        defs: defs_of(programs),
        declared: declared_of(programs),
        effect_names: effect_names_of(programs),
        model: crate::model::Model::derive(programs, import_renames),
    };

    // ---- validate, then evaluate ----
    let mut outcomes: Vec<ClaimOutcome> = Vec::new();
    for c in &claims {
        let valid = validate_claim(c, &cx, &mut diags);
        let result = if !valid {
            "invalid"
        } else {
            match &c.form {
                ClaimForm::ForbidReaches { .. } => {
                    evaluate_forbid_reaches(c, &cx, &mut diags)
                }
                ClaimForm::OnlyEdges { .. } => {
                    evaluate_only_edges(c, &cx, &mut diags)
                }
                ClaimForm::Bound { .. } => {
                    evaluate_bound(c, &cx, &mut diags)
                }
                ClaimForm::Require { .. } => {
                    evaluate_require(c, &cx, &mut diags)
                }
                ClaimForm::Cover { .. } => {
                    evaluate_cover(c, &cx, &mut diags)
                }
                ClaimForm::Count { .. } => {
                    evaluate_count(c, &cx, &mut diags)
                }
            }
        };
        outcomes.push(ClaimOutcome {
            name: c.name.name.clone(),
            form: render_form(&c.form),
            result,
        });
    }
    (diags, outcomes)
}

// ===================== validation =================================

/// Validate every name a claim references. Unknown ⇒ error, and the
/// claim is not evaluated (its outcome is `invalid`).
fn validate_claim(c: &ClaimDecl, cx: &Cx, diags: &mut Vec<Diag>) -> bool {
    let mut ok = true;
    let check_group = |name: &Ident, diags: &mut Vec<Diag>| -> bool {
        if cx.groups.contains_key(&name.name) {
            return true;
        }
        let mut near: Vec<&String> = cx
            .groups
            .keys()
            .filter(|g| close(g, &name.name))
            .collect();
        near.sort();
        let hint = match near.first() {
            Some(n) => format!(" Did you mean `{}`?", n),
            None => String::new(),
        };
        diags.push(Diag::ty(
            name.span,
            format!(
                "claim `{}` names group `{}`, which is never declared. \
                 Add `group {} = {{ … }};` at the top level.{}",
                c.name.name, name.name, name.name, hint
            ),
        ));
        false
    };
    let check_topic =
        |t: &TopicRef, cx: &Cx, diags: &mut Vec<Diag>| -> bool {
            if t.segments.len() == 1
                && cx.topic_names.contains(&t.segments[0].name)
            {
                return true;
            }
            if t.segments.len() > 1 {
                // Canonicalized at the mangle stage; still
                // multi-segment means no rename matched.
                diags.push(Diag::ty(
                    t.span,
                    format!(
                        "claim `{}`: topic reference `{}` does not \
                         resolve — no imported topic matches this \
                         path. Unknown names are errors, never empty \
                         sets",
                        c.name.name,
                        t.display()
                    ),
                ));
                return false;
            }
            let bad = &t.segments[0].name;
            let mut near: Vec<&String> = cx
                .topic_names
                .iter()
                .filter(|n| close(n, bad))
                .collect();
            near.sort();
            let hint = match near.first() {
                Some(n) => format!(" Did you mean `{}`?", n),
                None => String::new(),
            };
            diags.push(Diag::ty(
                t.span,
                format!(
                    "claim `{}` names topic `{}`, which is never \
                     declared.{}",
                    c.name.name, bad, hint
                ),
            ));
            false
        };
    let check_class = |class: &EffectClass,
                       name: &str,
                       span: hale_syntax::Span,
                       cx: &Cx,
                       diags: &mut Vec<Diag>|
     -> bool {
        let EffectClass::User(i) = class else { return true };
        if cx.declared.contains(i) {
            return true;
        }
        let mut near: Vec<&String> = cx
            .effect_names
            .iter()
            .enumerate()
            .filter(|(j, _)| cx.declared.contains(&(*j as u16)))
            .map(|(_, n)| n)
            .filter(|n| close(n, name))
            .collect();
        near.sort();
        let hint = match near.first() {
            Some(n) => format!(" Did you mean `{}`?", n),
            None => String::new(),
        };
        diags.push(Diag::ty(
            span,
            format!(
                "claim `{}` names effect class `{}`, which is never \
                 declared. Add `effect {};` at the top level.{}",
                c.name.name, name, name, hint
            ),
        ));
        false
    };
    match &c.form {
        ClaimForm::ForbidReaches {
            src,
            dst,
            avoiding,
            ..
        } => {
            match src {
                ClaimSet::Group(n) => ok &= check_group(n, diags),
                ClaimSet::Effects { span, .. } => {
                    diags.push(Diag::ty(
                        *span,
                        format!(
                            "claim `{}`: `effects(...)` is only valid \
                             in target position — sources must be \
                             declared groups",
                            c.name.name
                        ),
                    ));
                    ok = false;
                }
            }
            match dst {
                ClaimSet::Group(n) => ok &= check_group(n, diags),
                ClaimSet::Effects { class, name, span } => {
                    ok &= check_class(class, name, *span, cx, diags);
                }
            }
            if let Some(a) = avoiding {
                ok &= check_group(a, diags);
                // A mask overlapping an endpoint is a fail-open in
                // disguise: masking the target makes the claim hold
                // vacuously (no path can end at a masked vertex),
                // and masking a source silently drops roots. Both
                // read as law and prove less than they say.
                if cx.groups.contains_key(&a.name) {
                    let av = &cx.groups[&a.name];
                    for set in [src, dst] {
                        let ClaimSet::Group(n) = set else { continue };
                        let Some(gr) = cx.groups.get(&n.name) else {
                            continue;
                        };
                        let overlap = av
                            .loci
                            .intersection(&gr.loci)
                            .next()
                            .is_some()
                            || av
                                .free_fns
                                .intersection(&gr.free_fns)
                                .next()
                                .is_some();
                        if overlap {
                            diags.push(Diag::ty(
                                a.span,
                                format!(
                                    "claim `{}`: `avoiding {}` overlaps \
                                     `{}` — masking an endpoint makes \
                                     the claim weaker than it reads (a \
                                     masked target holds vacuously; a \
                                     masked source drops roots). Make \
                                     the gate disjoint from the \
                                     endpoints",
                                    c.name.name, a.name, n.name
                                ),
                            ));
                            ok = false;
                        }
                    }
                }
            }
        }
        ClaimForm::OnlyEdges { src, dst, grants } => {
            ok &= check_group(src, diags);
            ok &= check_group(dst, diags);
            for g in grants {
                ok &= check_topic(&g.topic, cx, diags);
            }
        }
        ClaimForm::Bound {
            class,
            class_name,
            class_span,
            from,
            ..
        } => {
            ok &= check_group(from, diags);
            if matches!(class, EffectClass::User(_)) {
                ok &= check_class(class, class_name, *class_span, cx, diags);
            } else {
                diags.push(Diag::ty(
                    *class_span,
                    format!(
                        "claim `{}`: `bound` takes a user-declared \
                         effect class — the counted built-ins keep \
                         their `@budget` spellings (`publish`, \
                         `block_points`, `alloc_per_call`)",
                        c.name.name
                    ),
                ));
                ok = false;
            }
        }
        ClaimForm::Require { group, topic, .. } => {
            ok &= check_group(group, diags);
            ok &= check_topic(topic, cx, diags);
        }
        ClaimForm::Cover { alias, group } => {
            ok &= check_group(group, diags);
            if !cx.alias_topics.contains_key(&alias.name) {
                diags.push(Diag::ty(
                    alias.span,
                    format!(
                        "claim `{}`: `seed({})` names no import alias \
                         with declared topics — the coverage domain \
                         would be empty, and a universal over an \
                         empty domain holds vacuously",
                        c.name.name, alias.name
                    ),
                ));
                ok = false;
            }
        }
        ClaimForm::Count { topic, .. } => {
            ok &= check_topic(topic, cx, diags);
        }
    }
    ok
}

/// The normalized sentence, for the artifact.
fn render_form(f: &ClaimForm) -> String {
    match f {
        ClaimForm::ForbidReaches {
            src,
            dst,
            via_calls,
            via_bus,
            during,
            avoiding,
        } => {
            let mut s = format!(
                "forbid reaches({}, {})",
                src.display(),
                dst.display()
            );
            match (via_calls, via_bus) {
                (true, true) => {}
                (true, false) => s.push_str(" via { calls }"),
                (false, true) => s.push_str(" via { bus }"),
                (false, false) => unreachable!("rejected at parse"),
            }
            if let Some(p) = during {
                s.push_str(&format!(" during {}", p.name));
            }
            if let Some(a) = avoiding {
                s.push_str(&format!(" avoiding {}", a.name));
            }
            s
        }
        ClaimForm::OnlyEdges { src, dst, grants } => {
            let gs: Vec<String> = grants
                .iter()
                .map(|g| {
                    format!(
                        "{} {}",
                        if g.publish { "publish" } else { "subscribe" },
                        g.topic.display()
                    )
                })
                .collect();
            format!(
                "only edges {} -> {} {{ {} }}",
                src.name,
                dst.name,
                gs.join("; ")
            )
        }
        ClaimForm::Bound {
            class_name,
            limit,
            from,
            ..
        } => format!(
            "bound {} <= {} on paths from {}",
            class_name, limit, from.name
        ),
        ClaimForm::Require {
            publishers,
            group,
            topic,
        } => format!(
            "require {}(some {}, topic {})",
            if *publishers { "publishes" } else { "subscribes" },
            group.name,
            topic.display()
        ),
        ClaimForm::Cover { alias, group } => format!(
            "cover topic in seed({}): subscribed_by(some {})",
            alias.name, group.name
        ),
        ClaimForm::Count {
            publishers,
            topic,
            cmp,
            n,
        } => format!(
            "count {}(topic {}) {} {}",
            if *publishers { "publishers" } else { "subscribers" },
            topic.display(),
            cmp.as_str(),
            n
        ),
    }
}

// ===================== group member resolution ====================

/// Resolve one group member into `rg`, or push a diagnostic.
fn resolve_member(
    m: &GroupMember,
    locus_names: &BTreeSet<String>,
    free_fn_names: &BTreeSet<String>,
    import_renames: &[(Vec<String>, String)],
    rg: &mut ResolvedGroup,
    diags: &mut Vec<Diag>,
) {
    if m.glob {
        // `alias::*` — enumeration over the imported seed's declared
        // decls, via the same rename table codegen resolves
        // `alias::Name` through. Trailing-only, single-level.
        if m.segments.len() != 1 {
            diags.push(Diag::ty(
                m.span,
                format!(
                    "group member `{}`: a glob expands a single import \
                     alias (`alias::*`); nested globs are not supported",
                    m.display()
                ),
            ));
            return;
        }
        let alias = &m.segments[0].name;
        let mut matched = false;
        for (key, mangled) in import_renames {
            if key.len() == 2 && &key[0] == alias {
                matched = true;
                // Only fn-bearing decls project into `reaches`;
                // types/topics/consts in the seed are simply not
                // path vertices.
                if locus_names.contains(mangled) {
                    rg.loci.insert(mangled.clone());
                    rg.decl_count += 1;
                } else if free_fn_names.contains(mangled) {
                    rg.free_fns.insert(mangled.clone());
                    rg.decl_count += 1;
                }
            }
        }
        if !matched {
            diags.push(Diag::ty(
                m.span,
                format!(
                    "group member `{}` names no import alias — the \
                     glob form expands `import \"…\" as {}`, which \
                     this bundle does not declare. Unknown names are \
                     errors, never empty sets",
                    m.display(),
                    alias
                ),
            ));
        }
        return;
    }
    if m.segments.len() > 1 {
        // Qualified members are canonicalized to the mangled single
        // segment at the mangle stage (the #334 path). Still
        // multi-segment here means no rename entry matched.
        diags.push(Diag::ty(
            m.span,
            format!(
                "group member `{}` does not resolve — no imported \
                 declaration matches this path. Unknown names are \
                 errors, never empty sets",
                m.display()
            ),
        ));
        return;
    }
    let name = &m.segments[0].name;
    let mut hit = false;
    if locus_names.contains(name) {
        rg.loci.insert(name.clone());
        rg.decl_count += 1;
        hit = true;
    }
    if free_fn_names.contains(name) {
        rg.free_fns.insert(name.clone());
        rg.decl_count += 1;
        hit = true;
    }
    if !hit {
        let mut near: Vec<&String> = locus_names
            .iter()
            .chain(free_fn_names.iter())
            .filter(|n| close(n, name))
            .collect();
        near.sort();
        near.dedup();
        let hint = match near.first() {
            Some(n) => format!(" Did you mean `{}`?", n),
            None => String::new(),
        };
        diags.push(Diag::ty(
            m.span,
            format!(
                "group member `{}` names no declared locus or fn. \
                 Unknown names are errors, never empty sets.{}",
                m.display(),
                hint
            ),
        ));
    }
}

// ===================== unresolved-callee backstop =================

/// The unresolved-callee backstop (#382 soundness audit).
///
/// After the receiver-typing root fix, the summarizer types struct-
/// literal receivers, chained fields, call results, and uniform
/// branch values — those all resolve to real edges now. What lands
/// here is the RESIDUE: a receiver that still cannot be typed at
/// this layer (an index result, a match value, a foreign
/// expression), recorded as `Unresolved` with `recv_ty: None` and
/// `receiver_present: true`. Such a call is a method of SOME bundle
/// locus reached through an opaque expression — and because it may
/// be a WRAPPER that reaches the target transitively, no name
/// comparison against the target set is sound. Any judgment that
/// traverses calls fails closed on the edge itself; the effect and
/// budget walkers apply the same rule, so fn-level certificates
/// and bundle-level claims agree. `recv_ty: Some` edges
/// (synthesized form/builtin methods like `counts.set`) are known
/// non-locus receivers and stay exempt.
///
/// #392: interface-dispatch calls never reach this predicate — a
/// dispatch WITH conformers arrives already fanned out to `Resolved`
/// alternatives, and one through an uninhabited interface is dead
/// code (no value of the interface can exist in this closed world),
/// which the walk skips like the summarizer's other non-edges.
fn unresolved_opaque_receiver(
    edge: &crate::alloc_summary::CallEdge,
) -> bool {
    edge.opaque_method_call()
}

// ===================== forbid reaches =============================

/// One step of a witness path. #392: each step carries the span of
/// the source decision that introduced the edge — the callsite, or
/// the publish site + subscription decl — so a violation can say
/// where to edit, not just which names are involved.
enum Step {
    Call {
        span: hale_syntax::Span,
        /// The interface, when this edge is one alternative of a
        /// dispatch rather than a direct call. Rendering it is the
        /// difference between a witness that reads as impossible and
        /// one that reads as conservative: the compiler fans an
        /// interface call out to EVERY conformer, so a path can end
        /// at `Sms::send` while the line in front of you constructs
        /// an `Email`. Shown as a direct call, that looks like a
        /// compiler bug; named as a dispatch, it is obviously sound.
        via_interface: Option<String>,
    },
    Bus {
        subject: String,
        publish_span: hale_syntax::Span,
        sub_span: hale_syntax::Span,
    },
}

/// Evaluate `forbid reaches(src, dst)` by BFS over the composed
/// graph, emitting ONE minimal countermodel per violated claim.
fn evaluate_forbid_reaches(
    c: &ClaimDecl,
    cx: &Cx,
    diags: &mut Vec<Diag>,
) -> &'static str {
    let ClaimForm::ForbidReaches {
        src,
        dst,
        via_calls,
        via_bus,
        during,
        avoiding,
    } = &c.form
    else {
        unreachable!("dispatched on form")
    };
    let ClaimSet::Group(src_name) = src else {
        unreachable!("rejected in validation")
    };
    let src_group = &cx.groups[&src_name.name];
    if projection_vacuity(
        c, "source", src_name, src_group, &cx.summary, diags,
    ) {
        return "invalid";
    }
    if let ClaimSet::Group(dst_name) = dst {
        let dst_group = &cx.groups[&dst_name.name];
        if projection_vacuity(
            c, "target", dst_name, dst_group, &cx.summary, diags,
        ) {
            return "invalid";
        }
    }
    let mut roots = src_group.fn_set(&cx.summary);
    // `during P` — restrict sources to the named phase of each
    // source locus, evaluated against the model's PHASE RELATION
    // (#392): lifecycle hooks and modes carry their runtime-driven
    // phase, ordinary methods their own name (the shipped
    // source-slice doctrine, now an explicit exported relation the
    // artifact carries — which is what makes a `during` row
    // independently re-derivable). Free fns have no phases and
    // drop out.
    if let Some(phase) = during {
        roots.retain(|k| {
            cx.model
                .phases
                .get(k)
                .is_some_and(|p| p.phase == phase.name)
        });
        if roots.is_empty() && !src_group.is_empty() {
            diags.push(Diag::ty(
                phase.span,
                format!(
                    "claim `{}`: phase `{}` names nothing in group \
                     `{}` — no member locus declares it. A claim over \
                     an empty phase holds vacuously",
                    c.name.name, phase.name, src_name.name
                ),
            ));
            return "invalid";
        }
    }
    // `avoiding G` — the vertex mask. Masked vertices are neither
    // tested nor traversed, so "no path avoiding the gate" is the
    // interposition proof.
    let mask_group = avoiding
        .as_ref()
        .map(|a| &cx.groups[&a.name]);

    // The dst membership test.
    enum DstTest<'a> {
        Group(&'a ResolvedGroup),
        Effects(EffectSet),
    }
    let dst_test = match dst {
        ClaimSet::Group(name) => DstTest::Group(&cx.groups[&name.name]),
        ClaimSet::Effects { class, .. } => {
            DstTest::Effects(crate::frontier::class_mask_with(
                *class, &cx.defs,
            ))
        }
    };
    // An empty dst domain forbids nothing; the vacuity guard already
    // fired at the group decl if that was unintentional.
    if let DstTest::Group(g) = &dst_test {
        if g.is_empty() {
            return "holds";
        }
    }

    let mut parent: BTreeMap<FnKey, (FnKey, Step)> = BTreeMap::new();
    let mut queue: VecDeque<FnKey> = VecDeque::new();
    let mut seen: BTreeSet<FnKey> = BTreeSet::new();
    for r in &roots {
        if mask_group.map_or(false, |m| m.contains_fn(r)) {
            continue;
        }
        if seen.insert(r.clone()) {
            queue.push_back(r.clone());
        }
    }
    let mut steps = 0u32;
    while let Some(k) = queue.pop_front() {
        steps += 1;
        if steps > callgraph::MAX_STEPS {
            diags.push(Diag::ty(
                c.name.span,
                format!(
                    "claim `{}`: reachability walk exceeded {} steps \
                     — cannot certify",
                    c.name.name,
                    callgraph::MAX_STEPS
                ),
            ));
            return "violated";
        }
        // Membership hit? Roots included: a decl in BOTH groups is a
        // zero-length path — a real boundary confusion `forbid`
        // should surface, not skip.
        let hit = match &dst_test {
            DstTest::Group(g) => g.contains_fn(&k),
            DstTest::Effects(mask) => {
                let direct = direct_effects(&cx.summary, &k, &cx.ffi);
                !direct.is_unclassified() && direct.0 & mask.0 != 0
            }
        };
        if hit {
            render_violation(
                c,
                src_name,
                &dst.display(),
                &k,
                &parent,
                cx,
                diags,
            );
            return "violated";
        }
        let Some(fs) = cx.summary.fns.get(&k) else { continue };
        if *via_calls {
            for edge in &fs.calls {
                match &edge.callee {
                    Callee::Resolved(next) => {
                        if mask_group
                            .map_or(false, |m| m.contains_fn(next))
                        {
                            continue;
                        }
                        if seen.insert(next.clone()) {
                            parent.insert(
                                next.clone(),
                                (
                                    k.clone(),
                                    Step::Call {
                                        span: edge.span,
                                        via_interface: edge
                                            .via_interface
                                            .clone(),
                                    },
                                ),
                            );
                            queue.push_back(next.clone());
                        }
                    }
                    Callee::Unresolved(name) => {
                        // #353: a call through a fn-typed param —
                        // the target is unknowable from here, so
                        // the claim cannot be certified. Unknown ⇒
                        // violation, same as every shipped
                        // certificate.
                        if edge.indirect
                            || fs.fn_params.iter().any(|p| p == name)
                        {
                            diags.push(Diag::ty(
                                c.name.span,
                                format!(
                                    "claim `{}` cannot be certified: \
                                     `{}` (reachable from `{}`) calls \
                                     through a function-typed \
                                     parameter, whose target is not \
                                     knowable statically. An \
                                     unresolvable edge fails closed",
                                    c.name.name,
                                    k.display(),
                                    src_name.name
                                ),
                            ));
                            return "violated";
                        }
                        // The backstop: an untyped-receiver call
                        // is a method of SOME locus, possibly a
                        // wrapper reaching the target. Fail closed.
                        if unresolved_opaque_receiver(edge) {
                            diags.push(Diag::ty(
                                c.name.span,
                                format!(
                                    "claim `{}` cannot be certified: \
                                     `{}` (reachable from `{}`) calls \
                                     `{}` on a receiver the compiler \
                                     cannot type, so the walk cannot \
                                     follow the edge. An unresolvable \
                                     edge fails closed — bind the \
                                     receiver to a typed field or \
                                     local so the call resolves",
                                    c.name.name,
                                    k.display(),
                                    src_name.name,
                                    name
                                ),
                            ));
                            return "violated";
                        }
                    }
                }
            }
        }
        if *via_bus {
            for site in &fs.effect_sites {
                let EffectSiteKind::Publish(subj) = &site.kind else {
                    continue;
                };
                let Some(subj) = subj else {
                    // Computed subject: could route anywhere the
                    // wire reaches. Fail closed.
                    diags.push(Diag::ty(
                        c.name.span,
                        format!(
                            "claim `{}` cannot be certified: `{}` \
                             (reachable from `{}`) publishes to a \
                             computed subject, which could route to \
                             any subscriber. An unresolvable edge \
                             fails closed",
                            c.name.name,
                            k.display(),
                            src_name.name
                        ),
                    ));
                    return "violated";
                };
                for (sub_locus, sub_handler, sub_span) in
                    subscribers_of(cx.graph, subj)
                {
                    let next =
                        FnKey::method(sub_locus, sub_handler);
                    if mask_group
                        .map_or(false, |m| m.contains_fn(&next))
                    {
                        continue;
                    }
                    if seen.insert(next.clone()) {
                        parent.insert(
                            next.clone(),
                            (
                                k.clone(),
                                Step::Bus {
                                    subject: subj.clone(),
                                    publish_span: site.span,
                                    sub_span,
                                },
                            ),
                        );
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }
    "holds"
}

// ===================== only edges =================================

/// Evaluate `only edges src -> dst { grants }`: every DIRECT edge
/// from src to dst must match a granted line. Reports EVERY
/// un-granted edge — the grant list is the review surface, so the
/// full diff matters.
fn evaluate_only_edges(
    c: &ClaimDecl,
    cx: &Cx,
    diags: &mut Vec<Diag>,
) -> &'static str {
    let ClaimForm::OnlyEdges { src, dst, grants } = &c.form else {
        unreachable!("dispatched on form")
    };
    let src_g = &cx.groups[&src.name];
    let dst_g = &cx.groups[&dst.name];
    if projection_vacuity(c, "source", src, src_g, &cx.summary, diags)
        || projection_vacuity(
            c, "target", dst, dst_g, &cx.summary, diags,
        )
    {
        return "invalid";
    }
    let granted: BTreeSet<&str> = grants
        .iter()
        .map(|g| g.topic.segments[0].name.as_str())
        .collect();
    let granted_disp = if granted.is_empty() {
        "none".to_string()
    } else {
        granted
            .iter()
            .map(|s| format!("`{}`", s))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut violated = false;
    let mut reported: BTreeSet<String> = BTreeSet::new();
    for k in src_g.fn_set(&cx.summary) {
        let Some(fs) = cx.summary.fns.get(&k) else { continue };
        for edge in &fs.calls {
            match &edge.callee {
                Callee::Resolved(next) => {
                    if dst_g.contains_fn(next) {
                        let key = format!(
                            "{}->{}",
                            k.display(),
                            next.display()
                        );
                        if reported.insert(key) {
                            diags.push(Diag::ty(
                                c.name.span,
                                format!(
                                    "claim `{}` violated: un-granted \
                                     edge `{}` -> `{}` — call edges \
                                     are not grantable; the boundary \
                                     between `{}` and `{}` must be a \
                                     bus edge named in the grant list",
                                    c.name.name,
                                    k.display(),
                                    next.display(),
                                    src.name,
                                    dst.name
                                ),
                            ));
                            violated = true;
                        }
                    }
                }
                Callee::Unresolved(name) => {
                    if edge.indirect
                        || fs.fn_params.iter().any(|p| p == name)
                    {
                        diags.push(Diag::ty(
                            c.name.span,
                            format!(
                                "claim `{}` cannot be certified: `{}` \
                                 calls through a function-typed \
                                 parameter, whose target is not \
                                 knowable statically. An unresolvable \
                                 edge fails closed",
                                c.name.name,
                                k.display()
                            ),
                        ));
                        return "violated";
                    }
                    if unresolved_opaque_receiver(edge) {
                        diags.push(Diag::ty(
                            c.name.span,
                            format!(
                                "claim `{}` cannot be certified: `{}` \
                                 calls `{}` on a receiver the \
                                 compiler cannot type, so the walk \
                                 cannot follow the edge. An \
                                 unresolvable edge fails closed — \
                                 bind the receiver to a typed field \
                                 or local so the call resolves",
                                c.name.name,
                                k.display(),
                                name
                            ),
                        ));
                        return "violated";
                    }
                }
            }
        }
        for site in &fs.effect_sites {
            let EffectSiteKind::Publish(subj) = &site.kind else {
                continue;
            };
            let Some(subj) = subj else {
                diags.push(Diag::ty(
                    c.name.span,
                    format!(
                        "claim `{}` cannot be certified: `{}` \
                         publishes to a computed subject, which could \
                         route to any subscriber. An unresolvable \
                         edge fails closed",
                        c.name.name,
                        k.display()
                    ),
                ));
                return "violated";
            };
            for (sub_locus, sub_handler, _sub_span) in
                subscribers_of(cx.graph, subj)
            {
                if !dst_g.loci.contains(&sub_locus) {
                    continue;
                }
                if granted.contains(subj.as_str()) {
                    continue;
                }
                let key = format!(
                    "{}-({})->{}::{}",
                    k.display(),
                    subj,
                    sub_locus,
                    sub_handler
                );
                if reported.insert(key) {
                    diags.push(Diag::ty(
                        c.name.span,
                        format!(
                            "claim `{}` violated: un-granted edge \
                             `{}` -(publishes \"{}\")-> `{}::{}`. \
                             Granted: {}. If this edge is intended, \
                             name it in the grant list — a grant is \
                             a reviewable line",
                            c.name.name,
                            k.display(),
                            subj,
                            sub_locus,
                            sub_handler,
                            granted_disp
                        ),
                    ));
                    violated = true;
                }
            }
        }
    }
    if violated {
        "violated"
    } else {
        "holds"
    }
}

// ===================== bound ======================================

/// Heaviest-path result: `None` = unbounded (cycle, loop-nested
/// carrier, indirect call, or computed subject on the path).
type Heaviest = Option<(u64, Vec<FnKey>)>;

/// Evaluate `bound C <= N on paths from G` — sum of carrier sites
/// along a path, max at joins, over the composed call ∘ bus graph.
fn evaluate_bound(
    c: &ClaimDecl,
    cx: &Cx,
    diags: &mut Vec<Diag>,
) -> &'static str {
    let ClaimForm::Bound {
        class,
        class_name,
        limit,
        from,
        ..
    } = &c.form
    else {
        unreachable!("dispatched on form")
    };
    let mask = crate::frontier::class_mask_with(*class, &cx.defs);
    let group = &cx.groups[&from.name];
    if projection_vacuity(c, "source", from, group, &cx.summary, diags)
    {
        return "invalid";
    }
    let mut worst: Heaviest = Some((0, Vec::new()));
    let mut worst_is_unbounded = false;
    for root in group.fn_set(&cx.summary) {
        let mut stack = Vec::new();
        let mut memo: BTreeMap<FnKey, (u64, Vec<FnKey>)> =
            BTreeMap::new();
        let mut steps = 0u32;
        match site_count(
            &root, cx, mask, &mut stack, &mut memo, &mut steps,
        ) {
            None => {
                worst_is_unbounded = true;
                break;
            }
            Some((w, p)) => {
                if worst.as_ref().map_or(true, |(bw, _)| w > *bw) {
                    worst = Some((w, p));
                }
            }
        }
    }
    if worst_is_unbounded {
        diags.push(Diag::ty(
            c.name.span,
            format!(
                "claim `{}` violated: paths from `{}` carry an \
                 unbounded number of `{}` sites (limit {}) — a \
                 recursion cycle, loop-nested carrier, indirect \
                 call, or computed publish subject makes the count \
                 unbounded",
                c.name.name, from.name, class_name, limit
            ),
        ));
        return "violated";
    }
    let (w, path) = worst.expect("finite worst");
    if w <= *limit {
        return "holds";
    }
    let chain = path
        .iter()
        .map(|k| format!("`{}`", k.display()))
        .collect::<Vec<_>>()
        .join(" -> ");
    diags.push(Diag::ty(
        c.name.span,
        format!(
            "claim `{}` violated: heaviest path from `{}` carries {} \
             `{}` sites, limit {} — path: {}",
            c.name.name, from.name, w, class_name, limit, chain
        ),
    ));
    "violated"
}

/// DFS: total carrier sites reachable from `k` per invocation — a
/// CALL-TREE SUM, exactly `@budget`'s per-call semantics (two calls
/// to a carrier are two sites; "max at joins" is the stack
/// dimension's rule, not the count's). Returns the total plus a
/// representative chain (each hop the largest contributor) for the
/// witness. `None` = unbounded. Memoizes only finite results
/// computed without a cycle hit (the `stack_depth` precedent: a
/// path-dependent verdict must not poison a diamond reached another
/// way).
fn site_count(
    k: &FnKey,
    cx: &Cx,
    mask: EffectSet,
    stack: &mut Vec<FnKey>,
    memo: &mut BTreeMap<FnKey, (u64, Vec<FnKey>)>,
    steps: &mut u32,
) -> Heaviest {
    if let Some(hit) = memo.get(k) {
        return Some(hit.clone());
    }
    if stack.contains(k) {
        return None; // cycle
    }
    *steps += 1;
    if *steps > callgraph::MAX_STEPS {
        return None;
    }
    let own: u64 = cx
        .summary
        .carries
        .get(k)
        .map_or(0, |c| if c.0 & mask.0 != 0 { 1 } else { 0 });
    let Some(fs) = cx.summary.fns.get(k) else {
        let r = (own, vec![k.clone()]);
        memo.insert(k.clone(), r.clone());
        return Some(r);
    };
    stack.push(k.clone());
    let mut total: u64 = 0;
    let mut best_child: (u64, Vec<FnKey>) = (0, Vec::new());
    // #392: fanned-out interface-dispatch alternatives share a group;
    // a dispatch invokes exactly ONE of them, so the group contributes
    // its MAX, not its sum — summing would count phantom calls that no
    // execution performs. (Any unbounded alternative still poisons the
    // whole count: dispatch may choose it.)
    let mut group_best: BTreeMap<u32, (u64, Vec<FnKey>)> =
        BTreeMap::new();
    let mut unbounded = false;
    for edge in &fs.calls {
        match &edge.callee {
            Callee::Resolved(next) => {
                match site_count(next, cx, mask, stack, memo, steps)
                {
                    None => {
                        unbounded = true;
                        break;
                    }
                    Some((w, p)) => {
                        // A carrier reached inside a loop repeats
                        // per iteration — unbounded, like every
                        // per-call contributor in `@budget`.
                        if edge.loop_depth > 0 && w > 0 {
                            unbounded = true;
                            break;
                        }
                        match edge.dispatch_group {
                            Some(g) => {
                                let e = group_best
                                    .entry(g)
                                    .or_insert((0, Vec::new()));
                                if w > e.0 {
                                    *e = (w, p);
                                }
                            }
                            None => {
                                total = total.saturating_add(w);
                                if w > best_child.0 {
                                    best_child = (w, p);
                                }
                            }
                        }
                    }
                }
            }
            Callee::Unresolved(name) => {
                if edge.indirect
                    || fs.fn_params.iter().any(|p| p == name)
                    // The backstop: an untyped-receiver call could
                    // be a wrapper reaching a carrier — an
                    // uncountable contribution. Fail closed
                    // (unbounded).
                    || unresolved_opaque_receiver(edge)
                {
                    unbounded = true;
                    break;
                }
            }
        }
    }
    if !unbounded {
        for (w, p) in group_best.into_values() {
            total = total.saturating_add(w);
            if w > best_child.0 {
                best_child = (w, p);
            }
        }
    }
    if !unbounded {
        for site in &fs.effect_sites {
            let EffectSiteKind::Publish(subj) = &site.kind else {
                continue;
            };
            let Some(subj) = subj else {
                unbounded = true;
                break;
            };
            for (sub_locus, sub_handler, _sub_span) in
                subscribers_of(cx.graph, subj)
            {
                let next = FnKey::method(sub_locus, sub_handler);
                match site_count(&next, cx, mask, stack, memo, steps)
                {
                    None => {
                        unbounded = true;
                        break;
                    }
                    Some((w, p)) => {
                        if site.loop_depth > 0 && w > 0 {
                            unbounded = true;
                            break;
                        }
                        total = total.saturating_add(w);
                        if w > best_child.0 {
                            best_child = (w, p);
                        }
                    }
                }
            }
            if unbounded {
                break;
            }
        }
    }
    stack.pop();
    if unbounded {
        return None;
    }
    let mut path = vec![k.clone()];
    path.extend(best_child.1);
    let r = (own + total, path);
    memo.insert(k.clone(), r.clone());
    Some(r)
}

// ===================== require / cover / count ====================

/// `require subscribes/publishes(some G, topic T)` — over the
/// DECLARED bus ends (the `bus { }` blocks), which is what "wired"
/// means.
fn evaluate_require(
    c: &ClaimDecl,
    cx: &Cx,
    diags: &mut Vec<Diag>,
) -> &'static str {
    let ClaimForm::Require {
        publishers,
        group,
        topic,
    } = &c.form
    else {
        unreachable!("dispatched on form")
    };
    let g = &cx.groups[&group.name];
    let subject = topic.segments[0].name.as_str();
    let hit = cx.graph.subjects.get(subject).map_or(false, |info| {
        if *publishers {
            info.publishers
                .iter()
                .any(|p| g.loci.contains(&p.locus))
        } else {
            info.subscribers
                .iter()
                .any(|s| g.loci.contains(&s.locus))
        }
    });
    if hit {
        return "holds";
    }
    diags.push(Diag::ty(
        c.name.span,
        format!(
            "claim `{}` violated: no member of `{}` {} `{}`",
            c.name.name,
            group.name,
            if *publishers { "publishes" } else { "subscribes" },
            topic.display()
        ),
    ));
    "violated"
}

/// `cover topic in seed(a): subscribed_by(some G)` — every topic the
/// seed declares has a subscriber in G. Reports EVERY uncovered
/// topic in one diagnostic.
fn evaluate_cover(
    c: &ClaimDecl,
    cx: &Cx,
    diags: &mut Vec<Diag>,
) -> &'static str {
    let ClaimForm::Cover { alias, group } = &c.form else {
        unreachable!("dispatched on form")
    };
    let g = &cx.groups[&group.name];
    let topics = &cx.alias_topics[&alias.name];
    let mut uncovered: Vec<&str> = Vec::new();
    for t in topics {
        let covered =
            cx.graph.subjects.get(t.as_str()).map_or(false, |info| {
                info.subscribers
                    .iter()
                    .any(|s| g.loci.contains(&s.locus))
            });
        if !covered {
            uncovered.push(t);
        }
    }
    if uncovered.is_empty() {
        return "holds";
    }
    let list = uncovered
        .iter()
        .map(|t| format!("`{}`", t))
        .collect::<Vec<_>>()
        .join(", ");
    diags.push(Diag::ty(
        c.name.span,
        format!(
            "claim `{}` violated: {} topic(s) declared in seed `{}` \
             have no subscriber in `{}`: {}",
            c.name.name,
            uncovered.len(),
            alias.name,
            group.name,
            list
        ),
    ));
    "violated"
}

/// `count publishers/subscribers(topic T) cmp N` — distinct loci on
/// the declared end.
fn evaluate_count(
    c: &ClaimDecl,
    cx: &Cx,
    diags: &mut Vec<Diag>,
) -> &'static str {
    let ClaimForm::Count {
        publishers,
        topic,
        cmp,
        n,
    } = &c.form
    else {
        unreachable!("dispatched on form")
    };
    let subject = topic.segments[0].name.as_str();
    let mut loci: BTreeSet<&str> = BTreeSet::new();
    if let Some(info) = cx.graph.subjects.get(subject) {
        if *publishers {
            for p in &info.publishers {
                loci.insert(&p.locus);
            }
        } else {
            for s in &info.subscribers {
                loci.insert(&s.locus);
            }
        }
    }
    let actual = loci.len() as u64;
    if cmp.holds(actual, *n) {
        return "holds";
    }
    let who = if loci.is_empty() {
        String::new()
    } else {
        format!(
            " ({})",
            loci.iter()
                .map(|l| format!("`{}`", l))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    diags.push(Diag::ty(
        c.name.span,
        format!(
            "claim `{}` violated: counted {} {}{} of `{}`, claim \
             requires {} {}",
            c.name.name,
            actual,
            if *publishers {
                "publisher(s)"
            } else {
                "subscriber(s)"
            },
            who,
            topic.display(),
            cmp.as_str(),
            n
        ),
    ));
    "violated"
}

// ===================== shared helpers =============================

/// Subscribers of a subject, including wildcard subscribers whose
/// pattern covers it (a `log.**` sink is an edge from every `log.x`
/// publish — more edges, the conservative direction).
fn subscribers_of(
    graph: &BusGraph,
    subject: &str,
) -> Vec<(String, String, hale_syntax::Span)> {
    let mut out = Vec::new();
    for (key, info) in &graph.subjects {
        let covers = key == subject
            || (key.contains("**")
                && crate::wildcard_match(key, subject));
        if !covers {
            continue;
        }
        for s in &info.subscribers {
            out.push((s.locus.clone(), s.handler.clone(), s.span));
        }
    }
    out
}

/// A fn's DIRECT effect contribution — its own body only, no
/// recursion (the BFS supplies transitivity). Mirrors the per-node
/// arm of `frontier::infer_effects`.
fn direct_effects(
    summary: &AllocSummary,
    key: &FnKey,
    ffi: &BTreeSet<String>,
) -> EffectSet {
    let mut acc = EffectSet::PURE;
    if let Some(c) = summary.carries.get(key) {
        acc = acc.union(*c);
    }
    let Some(fs) = summary.fns.get(key) else { return acc };
    if !fs.sites.is_empty() {
        acc = acc.union(EffectSet::ALLOC);
    }
    for site in &fs.effect_sites {
        acc = acc.union(match site.kind {
            EffectSiteKind::Publish(_) => EffectSet::PUBLISH,
            EffectSiteKind::Spawn(_) => EffectSet::ALLOC,
        });
    }
    for edge in &fs.calls {
        match &edge.callee {
            Callee::Resolved(k) => {
                // The callee's own contribution is tested when the
                // BFS visits it; a leaf's declared `is:` is on the
                // callee's `carries` entry.
                if k.locus.is_none() && ffi.contains(&k.fn_name) {
                    acc = acc.union(EffectSet::SYSCALL);
                }
            }
            Callee::Unresolved(name) => {
                let segs: Vec<&str> = name.split("::").collect();
                if let Some(e) = stdlib_surface::effects_for(&segs) {
                    if !e.is_unclassified() {
                        acc = acc.union(e);
                    }
                }
            }
        }
    }
    acc
}

/// Render the countermodel: the path from a source root to the hit,
/// with bus hops named. Matches the effect-witness house style —
/// the chain in author spelling, one line.
fn render_violation(
    c: &ClaimDecl,
    src_name: &Ident,
    dst_disp: &str,
    hit: &FnKey,
    parent: &BTreeMap<FnKey, (FnKey, Step)>,
    cx: &Cx,
    diags: &mut Vec<Diag>,
) {
    // Walk hit -> root collecting (node, incoming step), then
    // render forward.
    let mut rev: Vec<(FnKey, Option<&Step>)> = Vec::new();
    let mut cur = hit.clone();
    loop {
        match parent.get(&cur) {
            Some((prev, step)) => {
                rev.push((cur.clone(), Some(step)));
                cur = prev.clone();
            }
            None => {
                rev.push((cur.clone(), None));
                break;
            }
        }
    }
    rev.reverse();
    let mut path = String::new();
    for (node, incoming) in &rev {
        match incoming {
            None => path.push_str(&format!("`{}`", node.display())),
            Some(Step::Call { via_interface: Some(iface), .. }) => {
                // `-(dispatches Notifier.send)->` rather than `->`.
                path.push_str(&format!(
                    " -(dispatches {}.{})-> `{}`",
                    iface,
                    node.fn_name,
                    node.display()
                ));
            }
            Some(Step::Call { .. }) => {
                path.push_str(&format!(" -> `{}`", node.display()));
            }
            Some(Step::Bus { subject, .. }) => {
                path.push_str(&format!(
                    " -(publishes \"{}\")-> `{}`",
                    subject,
                    node.display()
                ));
            }
        }
    }
    diags.push(Diag::ty(
        c.name.span,
        format!(
            "claim `{}` violated: `{}` reaches `{}` — witness: {}",
            c.name.name, src_name.name, dst_disp, path,
        ),
    ));
    // #392 provenance: the witness names WHO; these point at WHERE
    // to edit — the source decision that introduced the crossing
    // edge, and the destination's declaration. Spans are emitted
    // only for bundle decls (`Model::is_bundle_fn`): stdlib bodies
    // parse in their own offset space, and a span from there
    // attributed to a bundle file would point at the wrong source.
    if let Some((entered, Some(step))) = rev.last().map(|(n, s)| (n, s))
    {
        // The fn whose body holds the crossing edge.
        let from = rev.len().checked_sub(2).map(|i| &rev[i].0);
        match step {
            Step::Call { span, via_interface } => {
                if from.is_some_and(|f| cx.model.is_bundle_fn(f)) {
                    // The dispatch case needs the extra sentence.
                    // Without it the reader looks at a line
                    // constructing one conformer, sees a witness
                    // naming another, and concludes the checker is
                    // wrong — when it is being conservative on
                    // purpose.
                    let msg = match via_interface {
                        Some(iface) => format!(
                            "claim `{}`: the boundary into `{}` is \
                             crossed by this dispatch through `{}`. \
                             A call on an interface reaches EVERY \
                             conforming locus, whatever this \
                             expression happens to construct — so the \
                             witness names one the claim forbids. \
                             Narrow the receiver's type, or exclude \
                             the conformer from the group",
                            c.name.name, dst_disp, iface
                        ),
                        None => format!(
                            "claim `{}`: the boundary into `{}` is \
                             crossed by this call",
                            c.name.name, dst_disp
                        ),
                    };
                    diags.push(Diag::ty(*span, msg));
                }
            }
            Step::Bus { publish_span, sub_span, .. } => {
                if from.is_some_and(|f| cx.model.is_bundle_fn(f)) {
                    diags.push(Diag::ty(
                        *publish_span,
                        format!(
                            "claim `{}`: the crossing publish \
                             happens here",
                            c.name.name
                        ),
                    ));
                }
                if cx.model.is_bundle_fn(entered) {
                    diags.push(Diag::ty(
                        *sub_span,
                        format!(
                            "claim `{}`: delivered at this \
                             subscription",
                            c.name.name
                        ),
                    ));
                }
            }
        }
    }
    let dst_decl =
        hit.locus.as_deref().unwrap_or(hit.fn_name.as_str());
    if let Some(span) = cx.model.decl_span(dst_decl) {
        diags.push(Diag::ty(
            span,
            format!(
                "claim `{}`: the forbidden destination `{}` is \
                 declared here",
                c.name.name, dst_decl
            ),
        ));
    }
}
