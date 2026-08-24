//! LAW SELECTION — which claims exist, over which entities.
//!
//! This file used to be the claim EVALUATOR too: ~1900 lines that
//! walked the program and returned a verdict per clause, in
//! parallel with the judgment engines answering the same questions
//! over the canonical model. GH #476 removed that duplication one
//! family at a time (Changes 5a–5h), and Change 10 deleted what was
//! left. `hale check` and the artifact read one judgment now.
//!
//! What stays is everything that runs BEFORE a verdict exists:
//!
//!   * **enumeration** — which clauses the bundle declares, across
//!     the world tier (a main locus's `claims { }`) and the library
//!     tier (a seed swearing about its own boundary);
//!   * **adoption** — constitutions, their closures, and the
//!     normalized digest that gives each an identity independent of
//!     its name;
//!   * **group resolution** — who a selector names, and whether the
//!     result is judgable at all (an unknown member and an
//!     undeclared-empty group are both refusals, and a law over a
//!     refused domain has no witness);
//!   * the small **vocabulary** helpers the model builder calls, so
//!     the model's effect columns are computed here once rather
//!     than approximated there.
//!
//! Selection is deliberately not a judgment. It reads clause text,
//! adoption, and membership — never the bus graph, and never an
//! effect walk. The public entry points still accept a `&BusGraph`
//! because their callers hold one and a future selection rule might
//! need topology; nothing in here consults it today.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;
use hale_syntax::Diag;

use crate::alloc_summary::{AllocSummary, Callee, EffectSiteKind, FnKey};
use crate::bus_graph::BusGraph;
use hale_model::GroupSelection;
use crate::effects::close;
use crate::stdlib_surface::{self, EffectSet};



/// THE law-selection result: the clauses selected, and every
/// diagnostic selection produced.
///
/// GH #476 Change 9 review: `selection_diags` and `lower_claims`
/// were BOTH doing selection, and doing different amounts of it.
/// The lowering called `enumerate_clauses` alone, so it saw
/// constitution problems but not group resolution — an unknown
/// group member failed `hale check` while the artifact recorded no
/// issue for it and could serialize the dependent law as `holds`.
/// The checker and the document then gave opposite machine-readable
/// answers about the same program, which is worse than the two
/// implementations this change set out to delete.
///
/// One result, two consumers. The claim rows come from `universe`;
/// the issues are `diags`, which cover clause enumeration AND group
/// resolution AND vacuity.
pub(crate) struct Selection<'a> {
    pub universe: ClauseUniverse<'a>,
    pub diags: Vec<Diag>,
    /// Per-group outcome, by RAW name — carried, never re-derived.
    pub groups: BTreeMap<String, GroupSelection>,
}

pub(crate) fn select<'a>(
    programs: &[&'a Program],
    // Unused since Change 10 — see `claims_report_inner`.
    _graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> Selection<'a> {
    let universe = enumerate_clauses(programs, import_renames);
    let (mut diags, _, groups) =
        claims_report_inner(programs, import_renames);
    crate::stdlib_bodies::demangle_imports(&mut diags, import_renames);
    for d in &mut diags {
        if d.kind == hale_syntax::error::DiagKind::Type {
            d.kind = hale_syntax::error::DiagKind::Claim;
        }
    }
    Selection { universe, diags, groups }
}

/// The identities of the constitutions actually adopted — GH #409's
/// normalized-closure digests.
///
/// GH #476 Change 9: both consumers (the artifact's constitution
/// section, `hale fleet`'s matrix) wanted ONLY the identities and
/// discarded the diagnostics and outcomes that came with them,
/// which meant every artifact dump ran the whole legacy evaluation
/// for a value it threw away. Adoption is settled during law
/// selection, so this stops there.
pub fn constitution_identities(
    programs: &[&Program],
    _graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> Adoption {
    let (_d, adoption, _groups) =
        claims_report_inner(programs, import_renames);
    adoption_identities(programs, adoption)
}

/// GH #476 Change 9 — LAW SELECTION only: which laws exist at all.
///
/// Clause enumeration (constitutions: unknown, cyclic, illegally
/// adopted, colliding), group resolution (a member naming nothing,
/// a group resolving to nothing without `may_be_empty`), and the
/// library/world tier rule. These are questions about the claim
/// SURFACE, and this module is their one authority.
///
/// What it deliberately does NOT do is judge. Verdicts — and the
/// validation diagnostics that precede them — come from the
/// judgment engines over the canonical model
/// (`judgment::claim_law_diags`). Before Change 9 both halves lived
/// here AND in the engines, and `hale check` read this copy while
/// the artifact read the other.
pub fn selection_diags(
    programs: &[&Program],
    graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> Vec<Diag> {
    select(programs, graph, import_renames).diags
}

/// Diagnostics plus per-claim outcomes (the artifact's rows).
/// Demangles cross-seed symbols in the diagnostics so witnesses name
/// what the author wrote.
/// GH #409 (review finding 3): a constitution's identity is its
/// NORMALIZED CLOSURE, not its display name.
///
/// Constitution names are flat and unmangled so diagnostics can cite
/// them as written. That is right for display and useless for
/// identity: two seeds can each declare `Core` with different
/// clauses, and an environment binding a bare name would accept
/// either. The matrix would then have proved "each entrypoint had
/// SOME constitution called Core" — not "every entrypoint was
/// evaluated against one shared claimset".
///
/// The digest covers the constitution's own clauses (name + rendered
/// form, sorted) and, recursively, its bases' digests. Two
/// declarations agree iff their whole closure agrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstitutionIdentity {
    pub name: String,
    pub digest: String,
}


/// Resolve an adoption's names to identities (name + normalized
/// closure digest). Shared by the full report and by
/// `constitution_identities`.
fn adoption_identities(
    programs: &[&Program],
    adoption: AdoptionInfo,
) -> Adoption {
    let mut consts: Vec<&ConstitutionDecl> = Vec::new();
    fn walk<'a>(items: &'a [TopDecl], out: &mut Vec<&'a ConstitutionDecl>) {
        for i in items {
            match i {
                TopDecl::Constitution(c) => out.push(c),
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    for p in programs {
        walk(&p.items, &mut consts);
    }
    let by_name: BTreeMap<&str, &ConstitutionDecl> =
        consts.iter().map(|c| (c.name.name.as_str(), *c)).collect();
    let id_of = |n: &String| ConstitutionIdentity {
        name: n.clone(),
        digest: constitution_digest(n, &by_name, &mut Vec::new()),
    };
    Adoption {
        roots: adoption.roots.iter().map(&id_of).collect(),
        closure: adoption.closure.iter().map(&id_of).collect(),
    }
}

/// The identities an evaluation adopted: the roots it named directly,
/// and the whole closure those roots reach.
///
/// A consumer needs both. `roots` is what the manifest asked for, so
/// it is what a matrix must compare across entrypoints; `closure` is
/// the law that actually applied.
#[derive(Debug, Default, Clone)]
pub struct Adoption {
    pub roots: Vec<ConstitutionIdentity>,
    pub closure: Vec<ConstitutionIdentity>,
}

/// FNV-1a/64 over the normalized closure. Same family as the
/// artifact's other identities, and dependency-free.
fn constitution_digest(
    name: &str,
    by_name: &BTreeMap<&str, &ConstitutionDecl>,
    stack: &mut Vec<String>,
) -> String {
    if stack.iter().any(|s| s == name) {
        return "cycle".to_string();
    }
    let Some(cd) = by_name.get(name) else {
        return "unresolved".to_string();
    };
    stack.push(name.to_string());
    // Dedup the bases. Expansion already visits each constitution
    // once, so `extends Core, Core` and `extends Core` have identical
    // evaluated clauses — hashing the base twice gave them different
    // digests, which would report a false mismatch between two
    // semantically identical closures. Fail-closed, but it means the
    // value called a NORMALIZED closure was not normalized.
    let mut bases: Vec<&str> =
        cd.extends.iter().map(|b| b.name.as_str()).collect();
    bases.sort_unstable();
    bases.dedup();
    let mut parts: Vec<String> = bases
        .iter()
        .map(|b| constitution_digest(b, by_name, stack))
        .collect();
    parts.sort();
    let mut own: Vec<String> = cd
        .entries
        .iter()
        .map(|e| format!("{}={}", e.name.name, render_form(&e.form)))
        .collect();
    own.sort();
    parts.extend(own);
    stack.pop();
    let joined = parts.join(";");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in joined.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", h)
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


}



/// What a deployment environment contributed to this evaluation: its
/// label, and the constitutions it required.
#[derive(Debug, Default, Clone)]
pub struct EnvBinding {
    pub name: Option<String>,
    /// Constitutions injected by the manifest rather than written in
    /// source. Kept so a diagnostic about one can say WHERE it was
    /// required from — without this, "unknown constitution `Prod`"
    /// points at a `main locus` containing no `adopt` line at all,
    /// and the author has no way to know the manifest asked for it.
    pub injected: Vec<String>,
}

thread_local! {
    /// A thread-local because the artifact is serialized, and claims
    /// are evaluated, far from the CLI that knows the label —
    /// threading an `EnvBinding` through every intervening signature
    /// would buy nothing.
    static ENV_BINDING: std::cell::RefCell<EnvBinding> =
        const { std::cell::RefCell::new(EnvBinding {
            name: None,
            injected: Vec::new(),
        }) };
}

/// Record the environment this evaluation is for.
pub fn set_env_binding(b: EnvBinding) {
    ENV_BINDING.with(|e| *e.borrow_mut() = b);
}

pub fn current_environment() -> Option<String> {
    ENV_BINDING.with(|e| e.borrow().name.clone())
}

fn injected_from_manifest(name: &str) -> Option<String> {
    ENV_BINDING.with(|e| {
        let b = e.borrow();
        if b.injected.iter().any(|i| i == name) {
            Some(match &b.name {
                Some(env) => format!(
                    ". `[environments.{}]` in hale.toml requires it — \
                     this entrypoint cannot see a declaration, so \
                     either import the seed that declares it or fix \
                     the manifest",
                    env
                ),
                None => ". It was required by hale.toml".to_string(),
            })
        } else {
            None
        }
    })
}

/// GH #409: what an evaluation adopted.
///
/// `roots` are the constitutions named directly (by source `adopt`
/// lines and by `--env` injection); `closure` is every constitution
/// reached from them. Both come from the adoption traversal, never
/// from inspecting which claims happened to be emitted.
#[derive(Debug, Default, Clone)]
pub struct AdoptionInfo {
    pub roots: Vec<String>,
    pub closure: Vec<String>,
}

/// GH #409: expand a main's `adopt` lines into its claim set,
/// returning claim-name → originating constitution.
///
/// Composition is UNION. A derived constitution may add clauses and
/// may not replace one, which is enforced by the collision rule
/// below: the same claim name arriving from two different origins is
/// an error. That is what makes weakening *unexpressible* rather
/// than merely discouraged — a stricter bound is a second named
/// claim that coexists with the inherited one, and both are checked.
fn expand_adoptions(
    consts: &[&ConstitutionDecl],
    adopts: &[Ident],
    lib_adopts: &[Ident],
    claims: &mut Vec<ClaimDecl>,
    diags: &mut Vec<Diag>,
    info: &mut AdoptionInfo,
) -> BTreeMap<String, String> {
    let mut origins: BTreeMap<String, String> = BTreeMap::new();

    // A library seed may DECLARE a constitution — that is how one is
    // shared — but adopting is the closing world's act, because
    // adoption is what fixes which world the clauses are evaluated
    // against.
    for a in lib_adopts {
        diags.push(Diag::ty(
            a.span,
            format!(
                "`adopt {}` is only legal in a `main locus`'s \
                 `claims` block. A constitution is evaluated against \
                 a CLOSED world, and a library seed does not close \
                 one — declare the constitution here and adopt it in \
                 each entrypoint",
                a.name
            ),
        ));
    }

    let mut by_name: BTreeMap<&str, &ConstitutionDecl> = BTreeMap::new();
    for c in consts {
        if let Some(prev) = by_name.insert(c.name.name.as_str(), c) {
            diags.push(Diag::ty(
                c.name.span,
                format!(
                    "constitution `{}` is declared twice — the second \
                     would silently shadow the first, and a claimset \
                     cited by name must resolve to one thing",
                    prev.name.name
                ),
            ));
        }
    }
    if adopts.is_empty() {
        return origins;
    }

    // Expansion, depth-first, deduped by CONSTITUTION. A diamond
    // (two bases sharing one) contributes the shared base's clauses
    // exactly once — dedup by origin, not by claim name, so a real
    // collision between two distinct origins still surfaces below.
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut collected: Vec<(String, ClaimDecl)> = Vec::new();
    fn visit(
        name: &Ident,
        by_name: &BTreeMap<&str, &ConstitutionDecl>,
        done: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
        out: &mut Vec<(String, ClaimDecl)>,
        diags: &mut Vec<Diag>,
    ) {
        if stack.contains(&name.name) {
            diags.push(Diag::ty(
                name.span,
                format!(
                    "constitution `{}` extends itself, directly or \
                     through {}",
                    name.name,
                    stack.join(" -> ")
                ),
            ));
            return;
        }
        if !done.insert(name.name.clone()) {
            return; // already contributed (diamond)
        }
        let Some(cd) = by_name.get(name.name.as_str()) else {
            let mut msg = format!(
                "unknown constitution `{}` — nothing declares it",
                name.name
            );
            // Manifest provenance: an injected adoption has no source
            // line, so the span points at the main locus and the
            // author sees no `adopt` to explain the error.
            if let Some(extra) = injected_from_manifest(&name.name) {
                msg.push_str(&extra);
            }
            if let Some(near) = by_name.keys().find(|k| {
                k.len().abs_diff(name.name.len()) <= 2
                    && k.chars().next() == name.name.chars().next()
            }) {
                msg.push_str(&format!(". Did you mean `{}`?", near));
            }
            diags.push(Diag::ty(name.span, msg));
            return;
        };
        stack.push(name.name.clone());
        for base in &cd.extends {
            visit(base, by_name, done, stack, out, diags);
        }
        stack.pop();
        for e in &cd.entries {
            out.push((cd.name.name.clone(), e.clone()));
        }
    }
    let mut stack = Vec::new();
    for a in adopts {
        if !info.roots.contains(&a.name) {
            info.roots.push(a.name.clone());
        }
        visit(
            a,
            &by_name,
            &mut done,
            &mut stack,
            &mut collected,
            diags,
        );
    }
    // The EFFECTIVE closure comes from the traversal, which visits
    // every constitution reached — directly adopted roots, empty
    // composition constitutions, intermediates, bases, and
    // diamond-deduplicated ancestors alike.
    //
    // Deriving it instead from the `source` of emitted claim rows
    // silently dropped any constitution that contributes no clause of
    // its own. `constitution Dev extends Left { }` was then absent
    // from the artifact and from the matrix's identity comparison —
    // so two entrypoints resolving the SAME `Dev` to different bases
    // shared no comparison key and passed. Pure composition is not an
    // edge case; #415's own corpus fixture uses it.
    info.closure = done.iter().cloned().collect();

    // Collisions. Claim names are the contract of record — cited in
    // reviews, in diagnostics, in the artifact — and are deliberately
    // never mangled, so they are one flat namespace.
    let local: BTreeSet<String> =
        claims.iter().map(|c| c.name.name.clone()).collect();
    for (origin, decl) in &collected {
        if local.contains(&decl.name.name) {
            diags.push(Diag::ty(
                decl.name.span,
                format!(
                    "claim `{}` is declared in this main AND adopted \
                     from constitution `{}`. A local clause cannot \
                     replace an adopted one — that is how a law gets \
                     quietly weakened. Rename one; a stricter \
                     variant is a separate named claim",
                    decl.name.name, origin
                ),
            ));
            continue;
        }
        match origins.get(&decl.name.name) {
            Some(prev) if prev != origin => {
                diags.push(Diag::ty(
                    decl.name.span,
                    format!(
                        "claim `{}` is declared by two constitutions, \
                         `{}` and `{}`. Composition is union, so a \
                         name must mean one thing across the whole \
                         adopted set",
                        decl.name.name, prev, origin
                    ),
                ));
            }
            // Same origin, same name. Diamond duplication is already
            // handled by the constitution-level `done` set, so a
            // constitution's entries are collected exactly once —
            // which means reaching here can only be TWO clauses
            // declared with one name inside a single constitution.
            // Skipping it silently dropped the second clause and let
            // the build pass while a law the author wrote went
            // unchecked.
            Some(_) => {
                diags.push(Diag::ty(
                    decl.name.span,
                    format!(
                        "constitution `{}` declares claim `{}` twice. \
                         The second would silently replace nothing and \
                         go unchecked — claim names are the contract \
                         of record, so one name means one clause",
                        origin, decl.name.name
                    ),
                ));
            }
            None => {
                origins.insert(
                    decl.name.name.clone(),
                    origin.clone(),
                );
                claims.push(decl.clone());
            }
        }
    }
    origins
}

/// The clause universe — the ONE enumeration of every claims-block
/// law the evaluator sees: this main's world tier, adopted
/// constitution clauses (#409), and library-tier top-level blocks
/// (#392 thread 2), plus the group declarations and adoption info
/// the evaluation needs. Extracted from `claims_report_inner` so
/// the `ClaimIr` lowering (GH #476 Change 4) walks EXACTLY the
/// clauses the evaluator walks — two enumerations would drift.
pub(crate) struct ClauseUniverse<'a> {
    pub claims: Vec<ClaimDecl>,
    /// claim name → originating constitution (adopted clauses).
    pub origins: BTreeMap<String, String>,
    /// claim name → attribution alias (library-tier clauses).
    pub library: BTreeMap<String, Option<String>>,
    pub group_decls: Vec<&'a GroupDecl>,
    pub adoption: AdoptionInfo,
    pub diags: Vec<Diag>,
}

/// The clauses law selection SELECTED, in authored order, each
/// with the constitution it was adopted from (`None` for a
/// main-block or library-tier clause).
///
/// Change 10: the lowering's parity obligation used to be stated
/// against the evaluator's outcome list. That was always a proxy —
/// the evaluator walked exactly the selected clauses — and the
/// evaluator is gone. This is the thing it was standing in for.
pub fn selected_clauses(
    programs: &[&Program],
    _graph: &BusGraph,
    import_renames: &[(Vec<String>, String)],
) -> Vec<(String, Option<String>)> {
    let u = enumerate_clauses(programs, import_renames);
    u.claims
        .iter()
        .map(|c| {
            (
                c.name.name.clone(),
                u.origins.get(&c.name.name).cloned(),
            )
        })
        .collect()
}

pub(crate) fn enumerate_clauses<'a>(
    programs: &[&'a Program],
    import_renames: &[(Vec<String>, String)],
) -> ClauseUniverse<'a> {
    let mut library: BTreeMap<String, Option<String>> = BTreeMap::new();
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
    #[allow(clippy::too_many_arguments)]
    fn walk_items<'a>(
        items: &'a [TopDecl],
        groups: &mut Vec<&'a GroupDecl>,
        claims: &mut Vec<ClaimDecl>,
        top_blocks: &mut Vec<&'a ClaimsBlock>,
        has_main: &mut bool,
        consts: &mut Vec<&'a ConstitutionDecl>,
        adopts: &mut Vec<Ident>,
        lib_adopts: &mut Vec<Ident>,
    ) {
        for item in items {
            match item {
                TopDecl::Group(g) => groups.push(g),
                TopDecl::Locus(l) if l.is_main => {
                    *has_main = true;
                    for m in &l.members {
                        if let LocusMember::Claims(cb) = m {
                            claims.extend(cb.entries.iter().cloned());
                            adopts.extend(cb.adopts.iter().cloned());
                        }
                    }
                }
                TopDecl::Claims(cb) => {
                    top_blocks.push(cb);
                    lib_adopts.extend(cb.adopts.iter().cloned());
                }
                TopDecl::Constitution(cd) => consts.push(cd),
                TopDecl::Module(m) => walk_items(
                    &m.items,
                    groups,
                    claims,
                    top_blocks,
                    has_main,
                    consts,
                    adopts,
                    lib_adopts,
                ),
                _ => {}
            }
        }
    }
    let mut top_blocks: Vec<&ClaimsBlock> = Vec::new();
    let mut has_main = false;
    let mut consts: Vec<&ConstitutionDecl> = Vec::new();
    let mut adopts: Vec<Ident> = Vec::new();
    let mut lib_adopts: Vec<Ident> = Vec::new();
    for p in programs {
        walk_items(
            &p.items,
            &mut group_decls,
            &mut claims,
            &mut top_blocks,
            &mut has_main,
            &mut consts,
            &mut adopts,
            &mut lib_adopts,
        );
    }
    // GH #409: expand adopted constitutions into this main's claim
    // set. Authoring is shared, evaluation is not — every clause is
    // checked HERE, against this entrypoint's closed world.
    let mut adoption = AdoptionInfo::default();
    let origins = expand_adoptions(
        &consts,
        &adopts,
        &lib_adopts,
        &mut claims,
        &mut diags,
        &mut adoption,
    );
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
                | ClaimForm::RequireSealed { group }
                | ClaimForm::Cover { group, .. } => Some(&group.name),
                ClaimForm::Count { topic, .. } => {
                    topic.segments.first().map(|s| &s.name)
                }
                // Names no group: a universal over the whole world.
                ClaimForm::RequireAttributed { .. } => None,
            }?;
            mangled_to_alias.get(name.as_str()).copied()
        });
        for e in &cb.entries {
            let mut c = e.clone();
            if let Some(a) = alias {
                c.name.name = format!("{}::{}", a, c.name.name);
            }
            library.insert(
                c.name.name.clone(),
                alias.map(|a| a.to_string()),
            );
            claims.push(c);
        }
    }
    ClauseUniverse {
        claims,
        origins,
        library,
        group_decls,
        adoption,
        diags,
    }
}

/// The normalized sentence, for the artifact.
/// Kept with SELECTION, not the evaluator: a constitution's
/// identity digest is the normalized text of its clause forms, so
/// rendering a form is part of deciding WHICH laws are adopted.
fn render_form(f: &ClaimForm) -> String {
    match f {
        ClaimForm::RequireSealed { group } => {
            format!("require sealed(all {})", group.name)
        }
        ClaimForm::RequireAttributed { class_name } => {
            format!("require attributed(all {})", class_name.name)
        }
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

/// Resolve one group member into `rg`, or push a diagnostic.
/// Kept with SELECTION: resolving a group's members is how the
/// selection decides which laws exist over which entities.
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

/// Change 10: selection does not consult the BUS GRAPH — that was
/// the evaluator's input, for counting publishers and walking
/// edges. Deciding WHICH laws exist is a question about clause
/// text, adoption, and group membership. The public entry points
/// still take a graph: their callers hold one anyway, and the
/// parameter is where a future selection rule that needs topology
/// would arrive.
fn claims_report_inner(
    programs: &[&Program],
    import_renames: &[(Vec<String>, String)],
) -> (Vec<Diag>, AdoptionInfo, BTreeMap<String, GroupSelection>) {
    let ClauseUniverse {
        claims,
        origins: _,
        library: _,
        group_decls,
        adoption,
        diags,
    } = enumerate_clauses(programs, import_renames);
    let mut diags = diags;
    if group_decls.is_empty() && claims.is_empty() {
        return (diags, adoption, BTreeMap::new());
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
    //
    // Each declaration's OUTCOME is recorded as it is decided (GH
    // #476 Change 9, review round 2). Downstream must not
    // re-derive "did selection accept this group?" from the model's
    // member count: an unresolved selector leaves no member behind,
    // so a misspelled name is indistinguishable from an
    // intentionally empty group, a partly-resolved group looks
    // whole, and a duplicated name looks fine while the model keeps
    // the LAST declaration and selection keeps the first.
    let mut groups: BTreeMap<String, ResolvedGroup> = BTreeMap::new();
    let mut group_selection: BTreeMap<String, GroupSelection> =
        BTreeMap::new();
    for g in &group_decls {
        if groups.contains_key(&g.name.name) {
            diags.push(Diag::ty(
                g.name.span,
                format!(
                    "group `{}` is declared more than once",
                    g.name.name
                ),
            ));
            // The NAME is refused, whichever declaration a later
            // stage happens to keep.
            group_selection
                .insert(g.name.name.clone(), GroupSelection::Refused);
            continue;
        }
        let mut rg = ResolvedGroup {
            decl_count: 0,
            may_be_empty: g.may_be_empty,
            loci: BTreeSet::new(),
            free_fns: BTreeSet::new(),
        };
        let mut selector_failed = false;
        for m in &g.members {
            let before = diags.len();
            resolve_member(
                m,
                &locus_names,
                &free_fn_names,
                import_renames,
                &mut rg,
                &mut diags,
            );
            // `resolve_member` reports an unresolvable selector and
            // contributes nothing — its failure is otherwise
            // invisible by construction.
            selector_failed |= diags.len() > before;
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
        let status = if selector_failed {
            // `may_be_empty` authorizes an intentionally empty
            // group; it does not turn a misspelled member into
            // intent.
            GroupSelection::SelectorFailed
        } else if rg.decl_count == 0 {
            if rg.may_be_empty {
                GroupSelection::IntentionallyEmpty
            } else {
                GroupSelection::Refused
            }
        } else {
            GroupSelection::Resolved
        };
        group_selection.insert(g.name.name.clone(), status);
        groups.insert(g.name.name.clone(), rg);
    }

    (diags, adoption, group_selection)
}


// ===================== only edges =================================


// ===================== bound ======================================





// ===================== require / cover / count ====================



/// The classes `require attributed` can actually check: those carried
/// by a registry row or a syntactic site, so a DIRECT site exists to
/// attribute. `ffi` / `spawn` / `recursion` are structural and have
/// none; a user class would be trivially true.
///
/// Validation and evaluation share this so a form the evaluator would
/// answer with unconditional success can never be accepted.
pub(crate) fn attributed_mask(
    name: &str,
) -> Option<crate::stdlib_surface::EffectSet> {
    use crate::stdlib_surface::EffectSet;
    Some(match name {
        "syscall" => EffectSet::SYSCALL,
        "block" => EffectSet::BLOCK,
        "publish" => EffectSet::PUBLISH,
        "time" => EffectSet::TIME,
        "entropy" => EffectSet::ENTROPY,
        "env" => EffectSet::ENV,
        "alloc" => EffectSet::ALLOC,
        "secret_use" => EffectSet::SECRET_USE,
        _ => return None,
    })
}


/// The `require attributed` DIRECT-site predicate, extracted so the
/// model builder computes `Function.attribution` with the SAME rule
/// (GH #476 Change 5c) — one definition, never approximated.
pub(crate) fn performs_directly_for(
    summary: &AllocSummary,
    model: &crate::model::Model,
    ffi: &BTreeSet<String>,
    key: &FnKey,
    fs: &crate::alloc_summary::FnSummary,
    mask: crate::stdlib_surface::EffectSet,
) -> bool {
    use crate::stdlib_surface::EffectSet;
    let direct_ffi = mask == EffectSet::SYSCALL
        && key.locus.is_none()
        && ffi.contains(&key.fn_name);
    let directly_classified = summary
        .carries
        .get(key)
        .map_or(false, |eff| eff.contains(mask));
    direct_ffi
        || directly_classified
        || fs.calls.iter().any(|e| match &e.callee {
            Callee::Unresolved(name) => {
                let segs: Vec<&str> = name.split("::").collect();
                crate::stdlib_surface::effects_for(&segs)
                    .map_or(false, |eff| eff.contains(mask))
            }
            Callee::Resolved(callee) => {
                if model.is_bundle_fn(callee) {
                    return false;
                }
                crate::frontier::infer_effects(summary, callee, ffi)
                    .contains(mask)
            }
        })
        || (mask == EffectSet::ALLOC && !fs.sites.is_empty())
        || (mask == EffectSet::PUBLISH
            && fs.effect_sites.iter().any(|s| {
                matches!(
                    s.kind,
                    crate::alloc_summary::EffectSiteKind::Publish(_)
                )
            }))
}

/// The `require attributed` opaque-call predicate (an unresolved
/// callee that is not a frontier path), shared with the model
/// builder for `Function.opaque_call`.
pub(crate) fn has_opaque_unresolved(
    fs: &crate::alloc_summary::FnSummary,
) -> bool {
    fs.calls.iter().any(|e| match &e.callee {
        Callee::Unresolved(n) => {
            let segs: Vec<&str> = n.split("::").collect();
            crate::stdlib_surface::effects_for(&segs).is_none()
        }
        Callee::Resolved(_) => false,
    })
}





// ===================== shared helpers =============================


/// A fn's DIRECT effect contribution — its own body only, no
/// recursion (the BFS supplies transitivity). Mirrors the per-node
/// arm of `frontier::infer_effects`.
pub(crate) fn direct_effects(
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

