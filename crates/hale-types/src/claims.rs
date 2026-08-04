//! GH #382 phase 1 — claims: named, bundle-level sentences over the
//! program graph.
//!
//! Every structural proof the compiler performs is one judgment form:
//! over a graph derived from source, evaluate a property, and on
//! failure produce a witness. This module makes that layer a
//! user-facing surface: `group` declarations are the vocabulary,
//! `claims { }` on the main locus holds the sentences, and a
//! violation renders a minimal countermodel path in author spelling.
//!
//! Phase 1 ships one verb — `forbid reaches(A, B) [via {calls, bus}]`
//! — re-plumbing the machinery `causes:` / `depends:` already run on:
//! the call graph from `alloc_summary` (with stdlib bodies merged, so
//! chains through stdlib resolve) and the bus graph's declared
//! publish/subscribe edges.
//!
//! Soundness posture, inherited from #265/#353/#354:
//!   - **Unknown name = error, not empty set.** A group member that
//!     resolves to nothing is the misspelt-effect-class bug wearing
//!     group clothing.
//!   - **Empty group = vacuity error** unless `may_be_empty`. A
//!     `forbid` trivially satisfied by an empty quantification domain
//!     is a fail-open in formal clothing.
//!   - **Unknown ⇒ violation.** An indirect call (fn-typed param) or
//!     a computed publish subject on a path from a `forbid` source
//!     cannot be certified and is reported, exactly as `@no_syscall`
//!     treats the same shapes. Over-approximation only ever adds
//!     edges.
//!
//! Claims are ERRORS gating `hale check`, never advisories: an
//! advisory claim reads as law and doesn't bind — the #354 fail-open
//! shape. Weakening a claim is a source diff (delete the `forbid`),
//! which is the review event this surface exists to create.

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

/// Entry point: resolve groups, validate claims, evaluate, witness.
/// Demangles cross-seed symbols in the output so witnesses name what
/// the author wrote.
pub fn claims_diags(
    programs: &[&Program],
    graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> Vec<Diag> {
    let mut out = claims_diags_inner(programs, graph, import_renames);
    crate::stdlib_bodies::demangle_imports(&mut out, import_renames);
    out
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
}

fn claims_diags_inner(
    programs: &[&Program],
    graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> Vec<Diag> {
    // ---- collect group decls + claims blocks (modules included) ----
    let mut group_decls: Vec<&GroupDecl> = Vec::new();
    let mut claims: Vec<&ClaimDecl> = Vec::new();
    fn walk_items<'a>(
        items: &'a [TopDecl],
        groups: &mut Vec<&'a GroupDecl>,
        claims: &mut Vec<&'a ClaimDecl>,
    ) {
        for item in items {
            match item {
                TopDecl::Group(g) => groups.push(g),
                TopDecl::Locus(l) if l.is_main => {
                    for m in &l.members {
                        if let LocusMember::Claims(cb) = m {
                            claims.extend(cb.entries.iter());
                        }
                    }
                }
                TopDecl::Module(m) => {
                    walk_items(&m.items, groups, claims)
                }
                _ => {}
            }
        }
    }
    for p in programs {
        walk_items(&p.items, &mut group_decls, &mut claims);
    }
    if group_decls.is_empty() && claims.is_empty() {
        return Vec::new();
    }

    let mut diags: Vec<Diag> = Vec::new();

    // ---- decl indexes for member resolution ----
    let mut locus_names: BTreeSet<String> = BTreeSet::new();
    let mut free_fn_names: BTreeSet<String> = BTreeSet::new();
    fn index_items(
        items: &[TopDecl],
        loci: &mut BTreeSet<String>,
        fns: &mut BTreeSet<String>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    loci.insert(l.name.name.clone());
                }
                TopDecl::Fn(f) => {
                    fns.insert(f.name.name.clone());
                }
                TopDecl::Module(m) => index_items(&m.items, loci, fns),
                _ => {}
            }
        }
    }
    for p in programs {
        index_items(&p.items, &mut locus_names, &mut free_fn_names);
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
        return diags;
    }

    // ---- validate claim names + set references ----
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
    let declared = declared_of(programs);
    let effect_names = effect_names_of(programs);
    let defs = defs_of(programs);
    let mut set_ok = true;
    for c in &claims {
        let ClaimForm::ForbidReaches { src, dst, .. } = &c.form;
        for (which, set) in [("source", src), ("target", dst)] {
            match set {
                ClaimSet::Group(name) => {
                    if !groups.contains_key(&name.name) {
                        let mut near: Vec<&String> = groups
                            .keys()
                            .filter(|g| close(g, &name.name))
                            .collect();
                        near.sort();
                        let hint = match near.first() {
                            Some(n) => {
                                format!(" Did you mean `{}`?", n)
                            }
                            None => String::new(),
                        };
                        diags.push(Diag::ty(
                            name.span,
                            format!(
                                "claim `{}` names group `{}`, which is \
                                 never declared. Add `group {} = {{ … \
                                 }};` at the top level.{}",
                                c.name.name, name.name, name.name, hint
                            ),
                        ));
                        set_ok = false;
                    }
                }
                ClaimSet::Effects { class, name, span } => {
                    if which == "source" {
                        diags.push(Diag::ty(
                            *span,
                            format!(
                                "claim `{}`: `effects(...)` is only \
                                 valid in target position in phase 1 \
                                 — sources must be declared groups",
                                c.name.name
                            ),
                        ));
                        set_ok = false;
                    }
                    if let EffectClass::User(i) = class {
                        if !declared.contains(i) {
                            let mut near: Vec<&String> = effect_names
                                .iter()
                                .enumerate()
                                .filter(|(j, _)| {
                                    declared.contains(&(*j as u16))
                                })
                                .map(|(_, n)| n)
                                .filter(|n| close(n, name))
                                .collect();
                            near.sort();
                            let hint = match near.first() {
                                Some(n) => {
                                    format!(" Did you mean `{}`?", n)
                                }
                                None => String::new(),
                            };
                            diags.push(Diag::ty(
                                *span,
                                format!(
                                    "claim `{}` names effect class \
                                     `{}`, which is never declared. \
                                     Add `effect {};` at the top \
                                     level.{}",
                                    c.name.name, name, name, hint
                                ),
                            ));
                            set_ok = false;
                        }
                    }
                }
            }
        }
    }
    if !set_ok {
        return diags;
    }

    // ---- evaluate ----
    // Same substrate as every shipped certificate: the bundle summary
    // with stdlib bodies merged, so a chain through a stdlib locus
    // method resolves instead of stopping at the boundary.
    let summary = crate::stdlib_bodies::summarize_with_stdlib_and_renames(
        programs,
        import_renames,
    );
    let ffi = ffi_names(programs);
    for c in &claims {
        evaluate_forbid_reaches(
            c, &groups, &summary, graph, &ffi, &defs, &mut diags,
        );
    }
    diags
}

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

/// One step of a witness path.
enum Step {
    Call,
    Bus { subject: String },
}

/// Evaluate `forbid reaches(src, dst)` by BFS over the composed
/// graph, emitting ONE minimal countermodel per violated claim.
fn evaluate_forbid_reaches(
    c: &ClaimDecl,
    groups: &BTreeMap<String, ResolvedGroup>,
    summary: &AllocSummary,
    graph: &BusGraph,
    ffi: &BTreeSet<String>,
    defs: &[Option<Vec<EffectClass>>],
    diags: &mut Vec<Diag>,
) {
    let ClaimForm::ForbidReaches { src, dst, via_calls, via_bus } =
        &c.form;
    let ClaimSet::Group(src_name) = src else {
        return; // rejected in validation
    };
    let src_group = &groups[&src_name.name];
    let roots = src_group.fn_set(summary);

    // The dst membership test.
    enum DstTest<'a> {
        Group(&'a ResolvedGroup),
        Effects(EffectSet),
    }
    let dst_test = match dst {
        ClaimSet::Group(name) => DstTest::Group(&groups[&name.name]),
        ClaimSet::Effects { class, .. } => {
            DstTest::Effects(crate::frontier::class_mask_with(
                *class, defs,
            ))
        }
    };
    // An empty dst domain forbids nothing; the vacuity guard already
    // fired at the group decl if that was unintentional.
    if let DstTest::Group(g) = &dst_test {
        if g.loci.is_empty() && g.free_fns.is_empty() {
            return;
        }
    }

    let mut parent: BTreeMap<FnKey, (FnKey, Step)> = BTreeMap::new();
    let mut queue: VecDeque<FnKey> = VecDeque::new();
    let mut seen: BTreeSet<FnKey> = BTreeSet::new();
    for r in &roots {
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
            return;
        }
        // Membership hit? Roots included: a decl in BOTH groups is a
        // zero-length path — a real boundary confusion `forbid`
        // should surface, not skip.
        let hit = match &dst_test {
            DstTest::Group(g) => g.contains_fn(&k),
            DstTest::Effects(mask) => {
                let direct = direct_effects(summary, &k, ffi);
                !direct.is_unclassified() && direct.0 & mask.0 != 0
            }
        };
        if hit {
            diags.push(render_violation(
                c,
                src_name,
                &dst.display(),
                &k,
                &parent,
            ));
            return;
        }
        let Some(fs) = summary.fns.get(&k) else { continue };
        if *via_calls {
            for edge in &fs.calls {
                match &edge.callee {
                    Callee::Resolved(next) => {
                        if seen.insert(next.clone()) {
                            parent.insert(
                                next.clone(),
                                (k.clone(), Step::Call),
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
                            return;
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
                    return;
                };
                for (sub_locus, sub_handler) in
                    subscribers_of(graph, subj)
                {
                    let next =
                        FnKey::method(sub_locus, sub_handler);
                    if seen.insert(next.clone()) {
                        parent.insert(
                            next.clone(),
                            (
                                k.clone(),
                                Step::Bus {
                                    subject: subj.clone(),
                                },
                            ),
                        );
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }
}

/// Subscribers of a subject, including wildcard subscribers whose
/// pattern covers it (a `log.**` sink is an edge from every `log.x`
/// publish — more edges, the conservative direction).
fn subscribers_of(
    graph: &BusGraph,
    subject: &str,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, info) in &graph.subjects {
        let covers = key == subject
            || (key.contains("**")
                && crate::wildcard_match(key, subject));
        if !covers {
            continue;
        }
        for s in &info.subscribers {
            out.push((s.locus.clone(), s.handler.clone()));
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
) -> Diag {
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
            Some(Step::Call) => {
                path.push_str(&format!(" -> `{}`", node.display()));
            }
            Some(Step::Bus { subject }) => {
                path.push_str(&format!(
                    " -(publishes \"{}\")-> `{}`",
                    subject,
                    node.display()
                ));
            }
        }
    }
    Diag::ty(
        c.name.span,
        format!(
            "claim `{}` violated: `{}` reaches `{}` — witness: {}",
            c.name.name, src_name.name, dst_disp, path,
        ),
    )
}
