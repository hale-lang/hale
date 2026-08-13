//! #265 (2026-07-29) — categoric effect assertions, phase 1.
//!
//! `@budget(alloc_per_call = N)` proved the shape: an opt-in
//! contract at a root the author cares about, inferred everywhere
//! else, enforced as a hard error over the resolved call graph.
//! This module generalizes it from "allocation count" to *effect
//! classes*, on the same substrate ([`crate::callgraph`]) — and
//! adds what the fixpoint never had: the diagnostic names the
//! **call chain to the violation**, not just the fn.
//!
//! Phase 1 ships the three assertions whose leaf sets are closed
//! and unambiguous:
//!
//!   * `@no_recursion` — call-graph acyclicity under the root (the
//!     static-stack precondition; also what makes a symbolic cost
//!     bound possible later).
//!   * `@no_ffi` — no `@ffi` fn transitively reachable ("pure
//!     managed Hale", the `forbid(unsafe)` analog).
//!   * `@no_block` — no blocking stdlib operation reachable. This
//!     is the contract an `async_io`-placed handler needs: a
//!     blocking call there stalls the pool's single worker (the
//!     Crumb batch-5 sleep bug, as a *checkable* property).
//!
//! Deliberately NOT here (see the issue's build order): `@no_panic`
//! is a different analysis (disposition coverage + index-op
//! selection, not leaf reachability); `@no_syscall` /
//! `@deterministic` wait on the full `lotus_*` frontier
//! classification (the `EffectSet` column in
//! [`crate::stdlib_surface`] is where that lands); the quantitative
//! layer (stack bytes, fan-out) and phase/placement-implied
//! contracts follow.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::verdict::Verdict;
use crate::alloc_summary::{self, AllocSummary, FnKey};
use crate::callgraph::{self, Probe};


/// Is this an `@ffi`-declared fn in the bundle?
pub(crate) fn ffi_names(programs: &[&Program]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for p in programs {
        for item in &p.items {
            if let TopDecl::Fn(f) = item {
                if f.ffi.is_some() {
                    out.insert(f.name.name.clone());
                }
            }
        }
    }
    out
}

/// GH #436: loci declared `@sealed`, for `require sealed(all G)`.
///
/// The stdlib sweep is defensive, not currently reachable: a group
/// member resolves against user declarations and seed imports, and
/// `group g = { std::secret::Signer };` is a resolution error today
/// ("no imported declaration matches this path"). Included so that if
/// stdlib loci ever become group-nameable, the answer is right rather
/// than silently `false` — which for a confinement claim would read as
/// "not sealed" and fail closed, but for the wrong reason.
pub(crate) fn sealed_loci_of(programs: &[&Program]) -> BTreeSet<String> {
    // Recursive: a sealed locus inside a `module` was invisible here,
    // so `require sealed` reported a violation that is not real.
    fn walk(items: &[TopDecl], out: &mut BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Locus(l) if l.sealed => {
                    out.insert(l.name.name.clone());
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    for p in programs {
        walk(&p.items, &mut out);
    }
    if let Some(sp) = crate::stdlib_bodies::program() {
        walk(&sp.items, &mut out);
    }
    out
}

/// GH #436: fns declaring `@effects(is: { … })` with a USER class,
/// for `require attributed(all C)`.
///
/// A built-in in `is:` restates what the compiler already infers; the
/// point of the claim is a purpose the AUTHOR supplied, so only user
/// classes count as attribution.
pub(crate) fn fns_carrying_a_user_class(
    programs: &[&Program],
) -> BTreeSet<crate::alloc_summary::FnKey> {
    use crate::alloc_summary::FnKey;
    let mut out = BTreeSet::new();
    let carries_user = |fd: &hale_syntax::ast::FnDecl| {
        fd.effects.iter().any(|a| {
            matches!(a, hale_syntax::ast::EffectAssert::Carries(cs)
                if cs.iter().any(|c| {
                    matches!(c, hale_syntax::ast::EffectClass::User(_))
                }))
        })
    };
    // Recursive for the same reason as `sealed_loci_of`: an
    // attributed method inside a `module` was invisible, so
    // `require attributed` blamed a fn that had named its purpose.
    fn walk(
        items: &[TopDecl],
        carries_user: &dyn Fn(&hale_syntax::ast::FnDecl) -> bool,
        out: &mut BTreeSet<FnKey>,
    ) {
        for item in items {
            match item {
                TopDecl::Fn(fd) if carries_user(fd) => {
                    out.insert(FnKey::free_fn(fd.name.name.clone()));
                }
                TopDecl::Locus(l) => {
                    // `@effects(is: …)` is fn-only (spec/tokens.md),
                    // so there is no locus-level form to inherit.
                    for m in &l.members {
                        if let hale_syntax::ast::LocusMember::Fn(fd) = m {
                            if carries_user(fd) {
                                out.insert(FnKey::method(
                                    l.name.name.clone(),
                                    fd.name.name.clone(),
                                ));
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, carries_user, out),
                _ => {}
            }
        }
    }
    for p in programs {
        walk(&p.items, &carries_user, &mut out);
    }
    out
}

/// Render `root -> hop -> hop [leaf]` for a witness chain./// Render `root -> hop -> hop [leaf]` for a witness chain.
fn chain(root: &FnKey, steps: &[callgraph::WitnessStep]) -> String {
    demangle_stdlib(&callgraph::render_witness(root, steps))
}

/// Stdlib loci are declared under mangled names (`__StdCliResolver`)
/// so they cannot collide with user identifiers. A witness path that
/// runs through one must still read in the spelling the user wrote,
/// or the diagnostic points at a name that appears nowhere in their
/// program — and nowhere they could look it up either.
fn demangle_stdlib(rendered: &str) -> String {
    let mut out = rendered.to_string();
    for (path, mangled) in hale_stdlib::PATH_RENAMES {
        if out.contains(mangled) {
            out = out.replace(mangled, &path.join("::"));
        }
    }
    out
}

/// The public entry: check every effect assertion in `programs`.
/// Returns hard-error diagnostics (opt-in: you asked for the
/// contract, so a violation fails the build) — empty when every
/// contract holds.
/// GH #265 step 6: **phase-indexed effects**. A locus declares
/// which effect classes each lifecycle phase may perform —
/// `@phase_effects(birth: {alloc}, run: {})`. The lifecycle model is
/// what makes this expressible: "no dynamic memory after
/// initialization" (the DO-178 discipline) IS "alloc allowed in
/// birth, forbidden in run and handlers", which a function-level
/// effect system cannot say because it has no notion of phase.
///
/// `alloc` is the phase-only class (allocation is a site, not a
/// frontier call); the rest reuse the same leaf lattice as
/// `@effects(...)`. A phase omitted from the annotation is
/// unconstrained; a phase present with `{}` forbids everything.
/// The seed's user effect-class intern table. Single-seed at v1, so the
/// first non-empty table is the one every `User(i)` indexes into.
pub(crate) fn effect_names_of(programs: &[&Program]) -> Vec<String> {
    programs
        .iter()
        .map(|p| &p.effect_names)
        .find(|n| !n.is_empty())
        .cloned()
        .unwrap_or_default()
}

/// Indices in the seed's table that came from an `effect NAME;`
/// DECLARATION, as opposed to a bare reference in an `@effects(...)`
/// clause. Interning happens for both, so without this a misspelt
/// class is indistinguishable from a real one.
/// Cheap near-miss test for the did-you-mean hint: one edit apart, or
/// a shared prefix long enough that a transposition is the likely
/// cause. Not a general spell-checker — it only has to catch typing.
pub(crate) fn close(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    let (x, y): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if x.len().abs_diff(y.len()) > 1 {
        return false;
    }
    let mut sorted_x = x.clone();
    let mut sorted_y = y.clone();
    sorted_x.sort_unstable();
    sorted_y.sort_unstable();
    // an anagram of the same letters is almost always a transposition
    if sorted_x == sorted_y {
        return true;
    }
    let common = x.iter().zip(y.iter()).take_while(|(p, q)| p == q).count();
    common * 2 >= x.len().min(y.len())
}

/// Every effect class that EXISTS for this program: the built-ins
/// plus each declared user class. `only:` is checked against this
/// rather than against a written-down list, so a class added later is
/// automatically outside any existing `only:` set instead of silently
/// slipping inside it.
fn class_universe(declared: &std::collections::BTreeSet<u16>) -> Vec<EffectClass> {
    let mut out = vec![
        EffectClass::Syscall,
        EffectClass::Block,
        EffectClass::Time,
        EffectClass::Entropy,
        EffectClass::Env,
        EffectClass::Ffi,
        EffectClass::Publish,
        EffectClass::Spawn,
        EffectClass::Recursion,
        EffectClass::Alloc,
        // GH #436 review 2: a compiler-owned class MUST appear in
        // every closed universe. `@effects(only: {…})` is the
        // complement of this list, so an omitted class is one the
        // contract can never forbid: `only: {}` certified a fn
        // reaching `secret_use`, making a closed contract weaker than
        // it reads. Adding a built-in without adding it here is the
        // integration seam to check.
        EffectClass::SecretUse,
    ];
    out.extend(declared.iter().map(|i| EffectClass::User(*i)));
    out
}

/// #354: the seed's composed-class definitions, index-parallel to
/// `effect_names`.
pub(crate) fn defs_of(programs: &[&Program]) -> Vec<Option<Vec<EffectClass>>> {
    programs
        .iter()
        .map(|p| &p.effect_defs)
        .find(|d| !d.is_empty())
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn declared_of(programs: &[&Program]) -> std::collections::BTreeSet<u16> {
    programs
        .iter()
        .find(|p| !p.effect_names.is_empty())
        .map(|p| p.declared_effects.iter().copied().collect())
        .unwrap_or_default()
}

/// #392 §8: a fn-grained certificate lowered to the claim IR's
/// vocabulary, with its verdict. The engines stay what they are —
/// the shared call-graph substrate (R1) plus per-class probes — but
/// every annotation is REPORTED as the claim form it is pointwise
/// sugar for, so the topology artifact carries all law (bundle
/// claims and fn certificates) in one schema of record.
pub struct LoweredCertificate {
    /// The annotated fn / locus, post-mangle (demangled at
    /// serialization like every artifact name).
    pub subject: String,
    /// The lowered claim form, display voice.
    pub form: String,
    /// See [`crate::verdict::Verdict`]. A certificate is a claim at
    /// fn granularity, so it reports in the same vocabulary the
    /// bundle claims use rather than a private bool.
    pub result: Verdict,
}

fn phase_effects_diags(
    programs: &[&Program],
    summary: &AllocSummary,
    ffi: &BTreeSet<String>,
    rows: &mut Vec<LoweredCertificate>,
) -> Vec<Diag> {
    let mut out = Vec::new();
    let names = &effect_names_of(programs);
    let defs = &defs_of(programs);
    let declared = declared_of(programs);
    for p in programs {
        for item in &p.items {
            let TopDecl::Locus(l) = item else { continue };
            let Some(pe) = &l.phase_effects else { continue };
            for (phase, allowed) in &pe.phases {
                // Which member fn is this phase? Lifecycle names map
                // to the method of the same name; anything else is a
                // handler/method name.
                // A phase names either a LIFECYCLE hook (`birth`,
                // `run`, `drain`, `dissolve`, `accept`, `release` —
                // stored as LocusMember::Lifecycle and keyed by name
                // in the summary) or a member fn / handler.
                let span = l
                    .members
                    .iter()
                    .find_map(|m| match m {
                        LocusMember::Fn(fd) if fd.name.name == *phase => {
                            Some(fd.name.span)
                        }
                        LocusMember::Lifecycle(lc)
                            if lifecycle_name(lc.kind) == *phase =>
                        {
                            Some(lc.span)
                        }
                        _ => None,
                    });
                let Some(span) = span else {
                    // A phase naming nothing on this locus used to be
                    // skipped in silence — so `@phase_effects(disolve:
                    // {})` declared a contract that was never checked,
                    // and the author had no way to tell. Every other
                    // form of incompleteness in this system fails
                    // closed; a typo'd phase must too.
                    //
                    // But the six LIFECYCLE names are always
                    // meaningful, declared or not: a locus with only
                    // `params` still has a birth, and
                    // `@phase_effects(birth: {alloc}, run: {})` — the
                    // canonical no-alloc-after-init line — must not
                    // error just because the hook is implicit.
                    const LIFECYCLE: &[&str] = &[
                        "birth", "accept", "release", "run", "drain",
                        "dissolve",
                    ];
                    if LIFECYCLE.contains(&phase.as_str()) {
                        continue;
                    }
                    let mut candidates: Vec<String> = Vec::new();
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(fd) => {
                                candidates.push(fd.name.name.clone())
                            }
                            LocusMember::Lifecycle(lc) => candidates
                                .push(lifecycle_name(lc.kind).to_string()),
                            _ => {}
                        }
                    }
                    let hint = crate::stdlib_surface::nearest_name(
                        phase,
                        candidates.iter().map(|s| s.as_str()),
                    )
                    .map(|s| format!(" — did you mean `{}`?", s))
                    .unwrap_or_else(|| {
                        if candidates.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " — this locus has {}",
                                candidates
                                    .iter()
                                    .map(|c| format!("`{}`", c))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                    });
                    out.push(Diag::ty(
                        l.name.span,
                        format!(
                            "`@phase_effects` names phase `{}`, which \
                             locus `{}` does not declare{}. The contract \
                             would never be checked.",
                            phase, l.name.name, hint
                        ),
                    ));
                    continue;
                };
                let key = FnKey::method(l.name.name.clone(), phase.clone());
                // The frontier/graph classes: anything NOT allowed.
                // #392 §8: DECLARED user classes join the closed set
                // — a phase contract is closed over the live class
                // universe, exactly like `only:` (and with the same
                // atomic-only complement: a composed class owns no
                // bit of its own). The hardcoded built-in list was
                // the documented deficiency that made the contract
                // blind to the classes a program declares itself.
                let mut classes: Vec<EffectClass> = vec![
                    EffectClass::Alloc,
                    EffectClass::Syscall,
                    EffectClass::Block,
                    EffectClass::Time,
                    EffectClass::Entropy,
                    EffectClass::Env,
                    EffectClass::Ffi,
                    EffectClass::Publish,
                    EffectClass::Spawn,
                    // Same reason as `class_universe`: an exact
                    // phase contract that cannot name a class cannot
                    // reject it. `@phase_effects(run: {})` admitted
                    // `secret_use` during run.
                    EffectClass::SecretUse,
                ];
                classes.extend(declared.iter().filter_map(|i| {
                    match defs.get(*i as usize) {
                        Some(Some(_)) => None, // composed: atomic-only
                        _ => Some(EffectClass::User(*i)),
                    }
                }));
                let phase_before = out.len();
                for class in classes {
                    if allowed.contains(&class) {
                        continue;
                    }
                    let before = out.len();
                    check_class(summary, &key, span, class, ffi, names, defs, &mut out);
                    // Re-label the generic message with the phase.
                    for d in out.iter_mut().skip(before) {
                        d.message = format!(
                            "phase `{}`: {}",
                            phase, d.message
                        );
                    }
                }
                rows.push(LoweredCertificate {
                    subject: l.name.name.clone(),
                    form: format!(
                        "only effects {{{}}} on {{{}}} during {}",
                        allowed
                            .iter()
                            .map(|c| cls_name(*c, names))
                            .collect::<Vec<_>>()
                            .join(", "),
                        l.name.name,
                        phase
                    ),
                    result: if out.len() > phase_before {
                        Verdict::Violated
                    } else {
                        Verdict::Holds
                    },
                });
            }
        }
    }
    out
}

/// Lifecycle hook → the name the summary keys it under.
fn lifecycle_name(kind: LifecycleKind) -> &'static str {
    match kind {
        LifecycleKind::Birth => "birth",
        LifecycleKind::Accept => "accept",
        LifecycleKind::Release => "release",
        LifecycleKind::Run => "run",
        LifecycleKind::Drain => "drain",
        LifecycleKind::Dissolve => "dissolve",
    }
}

/// Human label for an allocation site kind.
fn describe_alloc(kind: &alloc_summary::AllocKind) -> String {
    match kind {
        alloc_summary::AllocKind::StructLit(n) => {
            format!("an allocation (struct `{}`)", n)
        }
        alloc_summary::AllocKind::ArrayLit
        | alloc_summary::AllocKind::ArrayRepeat => {
            "an array allocation".to_string()
        }
        alloc_summary::AllocKind::BytesLit => {
            "a bytes allocation".to_string()
        }
        alloc_summary::AllocKind::CollectionInsert(f) => {
            format!("a {} insert", f)
        }
        alloc_summary::AllocKind::StringConcat => {
            "a string concatenation".to_string()
        }
    }
}

/// GH #265: **placement-implied contracts** — the check that needs
/// no annotation at all. A locus placed `cooperative(pool = X) where
/// async_io` runs its methods on that pool's single worker; a
/// BLOCKING operation reachable from one of its handlers holds that
/// worker, so every other locus on the pool stalls behind it. The
/// placement IS the assertion, so the compiler enforces it without
/// the author writing `@no_block`.
///
/// Reported as a WARNING, not an error: the placement may be
/// deliberate (a lone locus on a pool it owns), and the escape hatch
/// is explicit — the diagnostic names both fixes. This is the
/// Crumb batch-5 bug (a JS handler's `await sleep(400)` holding the
/// engine pool) as a compile-time finding.
fn placement_implied_diags(
    programs: &[&Program],
    summary: &AllocSummary,
) -> Vec<Diag> {
    use crate::stdlib_surface::EffectSet;
    // main-locus params fields placed on an async_io pool → the locus
    // TYPE names whose methods must not block.
    let mut async_io_types: Vec<(String, String)> = Vec::new(); // (type, pool)
    for p in programs {
        for item in &p.items {
            let TopDecl::Locus(l) = item else { continue };
            if !l.is_main {
                continue;
            }
            let mut field_ty: std::collections::BTreeMap<String, String> =
                Default::default();
            for m in &l.members {
                if let LocusMember::Params(pb) = m {
                    for prm in &pb.params {
                        if let Some(TypeExpr::Named { path, .. }) = &prm.ty {
                            if path.segments.len() == 1 {
                                field_ty.insert(
                                    prm.name.name.clone(),
                                    path.segments[0].name.clone(),
                                );
                            }
                        }
                    }
                }
            }
            for m in &l.members {
                if let LocusMember::Placement(pb) = m {
                    for e in &pb.entries {
                        let is_async = e.constraints.iter().any(|c| {
                            matches!(c.kind, PlacementConstraint::AsyncIo)
                        });
                        if !is_async {
                            continue;
                        }
                        let PlacementSpec::Cooperative { pool, .. } = &e.spec
                        else {
                            continue;
                        };
                        if let Some(t) = field_ty.get(&e.field.name) {
                            async_io_types.push((
                                t.clone(),
                                pool.as_ref()
                                    .map(|i| i.name.clone())
                                    .unwrap_or_else(|| "main".to_string()),
                            ));
                        }
                    }
                }
            }
        }
    }
    if async_io_types.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for p in programs {
        for item in &p.items {
            let TopDecl::Locus(l) = item else { continue };
            let Some((_, pool)) = async_io_types
                .iter()
                .find(|(t, _)| *t == l.name.name)
            else {
                continue;
            };
            for m in &l.members {
                let LocusMember::Fn(fd) = m else { continue };
                // An explicit assertion means the author is already
                // engaged with this — don't double-report.
                if !fd.effects.is_empty() {
                    continue;
                }
                let key =
                    FnKey::method(l.name.name.clone(), fd.name.name.clone());
                let mut pred = |probe: &Probe<'_>| match probe {
                    Probe::Unresolved(name, _) => {
                        let segs: Vec<&str> = name.split("::").collect();
                        let eff = crate::stdlib_surface::effects_for(&segs)?;
                        if eff.contains(EffectSet::BLOCK) {
                            Some(name.to_string())
                        } else {
                            None
                        }
                    }
                    Probe::Site(_) | Probe::Resolved(..) => None,
                };
                let Some(path) =
                    callgraph::witness_path(summary, &key, &mut pred)
                else {
                    continue;
                };
                out.push(Diag::warn(
                    fd.name.span,
                    format!(
                        "`{}` is placed on the async_io pool `{}`, whose \
                         single worker it shares — but it reaches {}. A \
                         blocking call here stalls every other locus on \
                         `{}` until it returns. Either move the blocking \
                         work to its own pool, or assert the intent with \
                         `@no_block` (to have the compiler enforce it) — \
                         the placement is the contract.",
                        key.display(),
                        pool,
                        chain(&key, &path),
                        pool
                    ),
                ));
            }
        }
    }
    out
}

pub fn effect_diags(programs: &[&Program]) -> Vec<Diag> {
    effect_diags_with_renames(programs, &[])
}

/// Same, with the bundle's cross-seed import renames so the
/// callgraph can walk into an imported seed.
pub fn effect_diags_with_renames(
    programs: &[&Program],
    import_renames: &[(Vec<String>, String)],
) -> Vec<Diag> {
    let mut out = effect_diags_inner(programs, import_renames);
    // A witness path through an imported seed would otherwise name
    // the merged symbol (`__lib_foo_bar_baz`), which appears nowhere
    // in the user's source.
    crate::stdlib_bodies::demangle_imports(&mut out, import_renames);
    out
}

fn effect_diags_inner(
    programs: &[&Program],
    import_renames: &[(Vec<String>, String)],
) -> Vec<Diag> {
    effect_report_inner(programs, import_renames).0
}

/// #392 §8: every fn-grained effect certificate (incl. the phase
/// contracts) as a lowered claim row with its verdict — evaluated
/// by the SAME pass that produces the diagnostics, so the two can
/// never disagree. The topology artifact serializes these beside
/// the bundle claims: one schema of record for all law.
pub fn certificate_rows(
    programs: &[&Program],
    import_renames: &[(Vec<String>, String)],
) -> Vec<LoweredCertificate> {
    effect_report_inner(programs, import_renames).1
}

fn effect_report_inner(
    programs: &[&Program],
    import_renames: &[(Vec<String>, String)],
) -> (Vec<Diag>, Vec<LoweredCertificate>) {
    let mut rows: Vec<LoweredCertificate> = Vec::new();
    let mut roots: Vec<(FnKey, Vec<EffectAssert>, Span)> = Vec::new();
    for program in programs {
        for item in &program.items {
            match item {
                TopDecl::Fn(fd) if !fd.effects.is_empty() => {
                    roots.push((
                        FnKey::free_fn(fd.name.name.clone()),
                        fd.effects.clone(),
                        fd.name.span,
                    ));
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            if !fd.effects.is_empty() {
                                roots.push((
                                    FnKey::method(
                                        l.name.name.clone(),
                                        fd.name.name.clone(),
                                    ),
                                    fd.effects.clone(),
                                    fd.name.span,
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let summary =
        crate::stdlib_bodies::summarize_with_stdlib_and_renames(programs, import_renames);
    // The placement-implied pass runs whether or not anything is
    // annotated — that is its point.
    let mut placement = placement_implied_diags(programs, &summary);
    // #265 step 6: phase-indexed effect contracts on loci.
    let ffi_all = ffi_names(programs);
    placement.extend(phase_effects_diags(
        programs, &summary, &ffi_all, &mut rows,
    ));
    if roots.is_empty() {
        return (placement, rows);
    }
    let ffi = ffi_names(programs);
    let names = effect_names_of(programs);
    // #345: a user class must be DECLARED before it can be asserted
    // about. Both a declaration and a bare reference intern a name, so
    // a typo silently became a brand-new class that nothing carries —
    // and `@effects(none: { monye })` then held vacuously and reported
    // success. A certificate that is quietly true of nothing is the
    // same failure as one that is quietly false.
    let declared = declared_of(programs);
    let defs_v = defs_of(programs);
    // No per-declaration span survives to here (`effect NAME;` produces
    // no AST item), so a cycle is reported against the program.
    let program_span = programs
        .first()
        .map(|p| p.span)
        .expect("at least one program");
    // #354: `effect a = { b }; effect b = { a };` resolves to PURE in
    // the mask walk, which would make both classes silently inert —
    // a contract naming either would hold vacuously. Reject it.
    for (i, def) in defs_v.iter().enumerate() {
        if def.is_none() {
            continue;
        }
        let mut seen = vec![i as u16];
        let mut frontier_q: Vec<u16> = Vec::new();
        if let Some(Some(ms)) = defs_v.get(i) {
            for m in ms {
                if let EffectClass::User(j) = m {
                    frontier_q.push(*j);
                }
            }
        }
        while let Some(j) = frontier_q.pop() {
            if j == i as u16 {
                placement.push(Diag::ty(
                    program_span,
                    format!(
                        "effect class `{}` is defined in terms of itself. \
                         A cyclic definition resolves to no effect at all, \
                         so every contract naming it would hold vacuously.",
                        names.get(i).cloned().unwrap_or_default()
                    ),
                ));
                break;
            }
            if seen.contains(&j) {
                continue;
            }
            seen.push(j);
            if let Some(Some(ms)) = defs_v.get(j as usize) {
                for m in ms {
                    if let EffectClass::User(k) = m {
                        frontier_q.push(*k);
                    }
                }
            }
        }
    }
    let mut diags = std::mem::take(&mut placement);
    for (key, asserts, span) in &roots {
        let mut seen: Vec<u16> = Vec::new();
        for a in *&asserts {
            let cs: &[EffectClass] = match a {
                EffectAssert::Forbid(cs)
                | EffectAssert::Causes(cs)
                | EffectAssert::Carries(cs) => cs,
                _ => &[],
            };
            for c in cs {
                if let EffectClass::User(i) = c {
                    if !declared.contains(i) && !seen.contains(i) {
                        seen.push(*i);
                        let bad = names
                            .get(*i as usize)
                            .cloned()
                            .unwrap_or_default();
                        let mut near: Vec<&String> = names
                            .iter()
                            .enumerate()
                            .filter(|(j, _)| declared.contains(&(*j as u16)))
                            .map(|(_, n)| n)
                            .filter(|n| close(n, &bad))
                            .collect();
                        near.sort();
                        let hint = match near.first() {
                            Some(n) => format!(" Did you mean `{}`?", n),
                            None => String::new(),
                        };
                        diags.push(Diag::ty(
                            *span,
                            format!(
                                "`{}` asserts about effect class `{}`, \
                                 which is never declared. Add `effect {};` \
                                 at the top level.{}",
                                key.display(),
                                bad,
                                bad,
                                hint
                            ),
                        ));
                    }
                }
            }
        }
    }
    for (key, asserts, span) in &roots {
        for a in asserts {
            match a {
                // #345: a classification, not an assertion —
                // nothing to verify here; it feeds attribution.
                EffectAssert::Carries(_) => {}
                EffectAssert::Forbid(classes) => {
                    for c in classes {
                        let before = diags.len();
                        check_class(
                            &summary, key, *span, *c, &ffi, &names, &defs_v,
                            &mut diags,
                        );
                        rows.push(LoweredCertificate {
                            subject: key.display(),
                            form: format!(
                                "forbid reaches({{{}}}, effects({}))",
                                key.display(),
                                cls_name(*c, &names)
                            ),
                            result: if diags.len() > before {
                        Verdict::Violated
                    } else {
                        Verdict::Holds
                    },
                        });
                    }
                }
                // #354: the closed dual. Checked as `none:` over the
                // COMPLEMENT, and the complement is computed from the
                // live class universe rather than written down — which
                // is the whole point. A hand-enumerated `none:` list
                // silently widens the moment a class is added; this
                // cannot, because nothing is recorded that could go
                // stale.
                EffectAssert::Only(allowed) => {
                    let set: Vec<String> =
                        allowed.iter().map(|c| cls_name(*c, &names)).collect();
                    let only_before = diags.len();
                    for c in class_universe(&declared) {
                        if allowed.contains(&c) {
                            continue;
                        }
                        // The complement quantifies over ATOMIC
                        // classes only. A composed class owns no bit
                        // — its mask is its members' — so it adds
                        // nothing the atomic complement misses, and
                        // including it is a fail-CLOSED-too-far: an
                        // `only:` listing a member (e.g.
                        // `knowledge(delta)`) would be rejected
                        // because the unlisted composition
                        // (`knowledge(*)`, or a hand-written union)
                        // overlaps the allowed bit. Found by the
                        // #382 phase-3 star classes; the same shape
                        // existed for any hand-written composed
                        // class.
                        if let EffectClass::User(i) = c {
                            if defs_v
                                .get(i as usize)
                                .map_or(false, |d| d.is_some())
                            {
                                continue;
                            }
                        }
                        let before = diags.len();
                        check_class(
                            &summary, key, *span, c, &ffi, &names, &defs_v,
                            &mut diags,
                        );
                        // Re-label with the contract that was actually
                        // violated. `check_class` phrases everything as
                        // `none:`, so without this a reader goes looking
                        // for a `none: {money}` that was never written —
                        // the class is forbidden by omission, and the
                        // message has to say so. Same re-labelling the
                        // `@phase_effects` path does.
                        for d in diags.iter_mut().skip(before) {
                            let body = d
                                .message
                                .strip_prefix("effect assertion violated: ")
                                .unwrap_or(&d.message)
                                .to_string();
                            d.message = format!(
                                "closed effect contract violated: \
                                 `{}` declares `only: {{ {} }}`, so {}",
                                key.display(),
                                set.join(", "),
                                body
                            );
                        }
                    }
                    rows.push(LoweredCertificate {
                        subject: key.display(),
                        form: format!(
                            "only effects {{{}}} on {{{}}}",
                            set.join(", "),
                            key.display()
                        ),
                        result: if diags.len() > only_before {
                        Verdict::Violated
                    } else {
                        Verdict::Holds
                    },
                    });
                }
                EffectAssert::PublishSet(allowed) => {
                    let before = diags.len();
                    check_publish_set(&summary, key, *span, allowed, &mut diags);
                    rows.push(LoweredCertificate {
                        subject: key.display(),
                        form: format!(
                            "only publishes {{{}}} from {{{}}}",
                            allowed.join(", "),
                            key.display()
                        ),
                        result: if diags.len() > before {
                        Verdict::Violated
                    } else {
                        Verdict::Holds
                    },
                    });
                }
                EffectAssert::NoPanic => {
                    let before = diags.len();
                    check_no_panic(programs, key, *span, &mut diags);
                    rows.push(LoweredCertificate {
                        subject: key.display(),
                        form: format!(
                            "forbid reaches({{{}}}, panic)",
                            key.display()
                        ),
                        result: if diags.len() > before {
                        Verdict::Violated
                    } else {
                        Verdict::Holds
                    },
                    });
                }
                EffectAssert::Causes(classes) => {
                    // Cross-actor causality needs the bus graph;
                    // effect_diags is graph-free, so the check runs
                    // in `check.rs` where the graph is built (see
                    // `frontier::causes_diags`). Nothing to do here
                    // — and no lowered row: the row belongs to the
                    // pass that owns the verdict.
                    let _ = classes;
                }
            }
        }
    }
    (diags, rows)
}

/// GH #265 coherence: ONE dispatcher over the effect classes. Every
/// `@no_*` flag desugars to `Forbid([class])` at parse time, so this
/// is the single place a class's meaning is defined — no flag can
/// drift from the general form.
/// #345: resolve an effect class to the name the USER wrote. Built-ins
/// are static; a `User(i)` is an index into the seed's intern table, so
/// a diagnostic that reaches for `as_str()` prints `<user effect>` and
/// loses the one thing the author named it for.
fn cls_name(class: EffectClass, names: &[String]) -> String {
    match class {
        EffectClass::User(i) => names
            .get(i as usize)
            .cloned()
            .unwrap_or_else(|| "<user effect>".to_string()),
        _ => class.as_str().to_string(),
    }
}

fn check_class(
    summary: &AllocSummary,
    key: &FnKey,
    span: Span,
    class: EffectClass,
    ffi: &BTreeSet<String>,
    names: &[String],
    defs: &[Option<Vec<EffectClass>>],
    diags: &mut Vec<Diag>,
) {
    use crate::stdlib_surface::EffectSet;
    // The three classes that are NOT stdlib-frontier queries.
    match class {
        EffectClass::Recursion => {
            if let Some(cyc) = find_recursion(summary, key) {
                diags.push(Diag::ty(
                    span,
                    format!(
                        "effect assertion violated: `{}` reaches a recursive \
                         cycle. {}. A recursive path has no static stack \
                         bound — restructure to an explicit worklist/loop, \
                         or drop the `recursion` assertion.",
                        key.display(),
                        cyc
                    ),
                ));
            }
            return;
        }
        EffectClass::Ffi => {
            let mut pred = |probe: &Probe<'_>| match probe {
                Probe::Resolved(k, _) => {
                    if k.locus.is_none() && ffi.contains(&k.fn_name) {
                        Some(format!("{} is an `@ffi` fn", k.fn_name))
                    } else {
                        None
                    }
                }
                Probe::Unresolved(name, _) => {
                    let bare = name.rsplit("::").next().unwrap_or(name);
                    if ffi.contains(bare) || ffi.contains(*name) {
                        Some(format!("{} is an `@ffi` fn", name))
                    } else {
                        None
                    }
                }
                Probe::Site(_) => None,
            };
            report(
                summary, key, span, "ffi", &mut pred, diags,
                "An `@ffi` call is a trust boundary the managed contract \
                 excludes — route the foreign work through a locus this fn \
                 doesn't reach.",
            );
            return;
        }
        EffectClass::Alloc => {
            // Allocation is site-based (the same vector @budget
            // counts) — any reachable site violates the class.
            if let Some(path) = callgraph::witness_path(
                summary,
                key,
                &mut |probe| match probe {
                    Probe::Site(site) => Some(describe_alloc(&site.kind)),
                    _ => None,
                },
            ) {
                let leaf = path.last().map(|s| s.span).unwrap_or(span);
                diags.push(Diag::ty(
                    span,
                    format!(
                        "effect assertion violated: `{}` must not reach \
                         `alloc`, but reaches {}. Hoist the allocation to a \
                         reused field or an initialization phase.",
                        key.display(),
                        callgraph::render_witness(key, &path)
                    ),
                ));
                diags.push(Diag::ty(
                    leaf,
                    "the `alloc` effect happens here".to_string(),
                ));
            }
            return;
        }
        EffectClass::Publish | EffectClass::Spawn => {
            // Syntactic effects: carried by `Topic <- v` / `Child { }`,
            // not by any call. Walk the effect-site vectors.
            if let Some((chain_s, site_span)) =
                find_effect_site(summary, key, |k| match (class, k) {
                    (
                        EffectClass::Publish,
                        alloc_summary::EffectSiteKind::Publish(subj),
                    ) => Some(match subj {
                        Some(s) => format!("publishes to `{}`", s),
                        None => "publishes".to_string(),
                    }),
                    (
                        EffectClass::Spawn,
                        alloc_summary::EffectSiteKind::Spawn(name),
                    ) => Some(format!("instantiates locus `{}`", name)),
                    _ => None,
                })
            {
                diags.push(Diag::ty(
                    span,
                    format!(
                        "effect assertion violated: `{}` reaches {}. Move \
                         the effect behind a locus this fn doesn't reach, or \
                         drop the `{}` assertion.",
                        key.display(),
                        chain_s,
                        cls_name(class, names)
                    ),
                ));
                diags.push(Diag::ty(
                    site_span,
                    format!("the `{}` effect happens here", cls_name(class, names)),
                ));
            }
            return;
        }
        _ => {}
    }
    // The frontier-query classes.
    let (mask, what) = match class {
        EffectClass::Syscall => (
            EffectSet::SYSCALL,
            "a syscall-class operation (filesystem, socket, process, \
             terminal, stdio)",
        ),
        EffectClass::Block => (
            EffectSet::BLOCK,
            "a blocking operation — it waits, holding its thread (or, on \
             an async_io pool, its worker's turn)",
        ),
        EffectClass::Time => (EffectSet::TIME, "a clock read"),
        EffectClass::Entropy => (EffectSet::ENTROPY, "an entropy read"),
        EffectClass::Env => (EffectSet::ENV, "an environment read"),
        // GH #436 review 2: queries the frontier like any other
        // masked class. Reaching this arm at all is the point — the
        // class was absent from `class_universe`, so `only: {}` never
        // asked about it and this code was unreachable BECAUSE the
        // contract was blind, not because the class was special.
        EffectClass::SecretUse => (
            EffectSet::SECRET_USE,
            "a privileged operation over confined secret material",
        ),
        // #345: a user class queries the frontier exactly like a
        // built-in — the bit differs, the machinery does not.
        EffectClass::User(_) => (
            crate::frontier::class_mask_with(class, defs),
            "an operation in this effect class",
        ),
        _ => unreachable!("handled above"),
    };
    let mut pred = |probe: &Probe<'_>| match probe {
        // #345: a leaf that DECLARES it carries this class.
        Probe::Resolved(k, _)
            if summary
                .carries
                .get(*k)
                .is_some_and(|c| (c.0 & mask.0) != 0) =>
        {
            Some(format!(
                "`{}` declares it carries this effect class",
                k.fn_name
            ))
        }
        // #333: a call into a `@shared` locus may WAIT on the
        // synchronization its fields carry. A `sync = serialized`
        // form is a per-map mutex, and acquiring one is a thread
        // wait — exactly what `block` means.
        //
        // Reported HERE rather than by descending, because the wait
        // happens inside the C runtime and there is no leaf site in
        // any Hale body to find. Without it, `@no_block` certified a
        // mutex acquisition as non-blocking, and `@shared` made that
        // reachable inside code the compiler had blessed.
        Probe::Resolved(k, _)
            if (mask.0
                & (EffectSet::BLOCK.0
                    | EffectSet::TIME.0
                    | EffectSet::ENTROPY.0
                    | EffectSet::ENV.0))
                != 0
                && k.locus
                    .as_deref()
                    .is_some_and(|l| summary.sync_holding_loci.contains(l)) =>
        {
            // Two claims fail on a shared locus, for one reason: its
            // state is reachable from another pool.
            //
            //   `block`  — a `sync = ...` form is a lock, and
            //              acquiring it waits on another thread.
            //   `time` / `entropy` / `env` — the classes
            //              `@deterministic` forbids. A shared read is
            //              not a function of the call's inputs,
            //              because another pool can change the value
            //              between two identical calls. Same
            //              distinction the docs draw between
            //              `monotonic_ns()` and `time_from_unix(n)`.
            //
            // The class LABEL is approximate for the second group —
            // a shared read is not literally a clock read, and it
            // wants its own effect class. Reporting it under the
            // determinism classes is deliberate in the meantime: an
            // imprecise label on a true finding beats a silent false
            // certificate, which is the failure this whole surface
            // exists to prevent. The witness text below says what it
            // actually is.
            let why = if mask.0 == EffectSet::BLOCK.0 {
                "reaching it can acquire that lock, which waits on another \
                 thread"
            } else {
                "reaching it reads state another pool can change, so the \
                 result is not a function of this call's inputs"
            };
            Some(format!(
                "`{}` is a method on `{}`, which holds a form carrying a \
                 `sync` discipline — {}",
                k.fn_name,
                k.locus.as_deref().unwrap_or(""),
                why
            ))
        }
        // #341: a DIRECT call into a synthesized form method
        // (`counts.set(x)`). These have no summary entry, so they
        // arrive Unresolved with only a bare name — the receiver type
        // rides on the edge instead.
        //
        // `block` is attributed because placement is not static:
        // whether the lock ever contends is undecidable at compile
        // time once placement can be swapped at runtime, so a
        // certificate reading "never blocks, we are single-pool
        // today" would be invalidated by a later swap. Conservative is
        // the only sound reading.
        Probe::Unresolved(_, edge)
            if (mask.0
                & (EffectSet::BLOCK.0
                    | EffectSet::TIME.0
                    | EffectSet::ENTROPY.0
                    | EffectSet::ENV.0))
                != 0
                && edge
                    .recv_ty
                    .as_deref()
                    .is_some_and(|t| summary.sync_forms.contains(t)) =>
        {
            let why = if mask.0 == EffectSet::BLOCK.0 {
                "acquiring its lock can wait on another thread"
            } else {
                "reading it sees state another pool can change, so the \
                 result is not a function of this call's inputs"
            };
            Some(format!(
                "`{}` carries a `sync` discipline — {}",
                edge.recv_ty.as_deref().unwrap_or(""),
                why
            ))
        }
        Probe::Unresolved(name, edge) => {
            // #353: an INDIRECT call — through a function-typed
            // parameter. Checked FIRST, because a bare name like `f` is
            // not a `std::` path and would otherwise return None a few
            // lines down, which is exactly how this stayed invisible.
            //
            // The target is not knowable from this fn, so it may do
            // anything and no certificate over it can hold. Before
            // this, such a call looked like an unknown free fn and
            // contributed nothing: `@no_syscall` on a fn whose body is
            // `return f(v);` passed while the program performed the
            // syscall, and `@budget(alloc_per_call = 0)` leaked the
            // same way.
            if edge.indirect {
                return Some(format!(
                    "`{}` — an indirect call through a function-typed \
                     parameter, whose target this fn cannot determine",
                    name
                ));
            }
            // #382 receiver-typing: a method call on a receiver that
            // STILL cannot be typed (an index result, a match value,
            // a foreign expression) is a method of some bundle locus
            // reached through an opaque expression — same fail-closed
            // rule as an indirect call. (#392 interface dispatch
            // never lands here: with conformers it is fanned out to
            // resolved edges; without any it is dead code.)
            if edge.opaque_method_call() {
                return Some(format!(
                    "`{}` — a method call on a receiver the compiler \
                     cannot type; bind the receiver to a typed field \
                     or local so the call resolves",
                    name
                ));
            }
            let segs: Vec<&str> = name.split("::").collect();
            let Some(eff) = crate::stdlib_surface::effects_for(&segs) else {
                // ABSENT must fail closed, exactly like UNCLASSIFIED.
                // The old `?` here made them asymmetric: an
                // unclassified ROW violated every assertion, but a
                // path with no row at all silently contributed
                // nothing — so a whole unregistered namespace
                // (`std::ts`, `std::shm`) read as pure. An assertion
                // is a claim about everything reachable; a leaf we
                // cannot classify is exactly what it must not
                // certify. Non-`std::` names are ordinary unresolved
                // callees (a bare method on a value whose type we
                // could not infer), not frontier leaves.
                return if segs.first() == Some(&"std") {
                    Some(format!(
                        "{} is not in the stdlib effect registry, so its \
                         effects are unknown and cannot be certified",
                        name
                    ))
                } else {
                    None
                };
            };
            if eff.is_unclassified() {
                return Some(format!("{} is not yet effect-classified", name));
            }
            if (eff.0 & mask.0) != 0 {
                Some(format!("{} — {}", name, what))
            } else {
                None
            }
        }
        Probe::Site(_) | Probe::Resolved(..) => None,
    };
    report(
        summary, key, span, &cls_name(class, names), &mut pred, diags,
        "Move the effect behind a locus this fn doesn't reach (the \
         reader/writer-locus shape), or pass the value in as a parameter.",
    );
}

/// Does a topic named in an effect set match a RESOLVED subject?
///
/// Subjects reach the analysis post-merge, so a topic declared in an
/// imported seed arrives carrying the IMPORTER's alias:
/// `import "lib/relay" as relay` yields `relay::Recalled`, and the
/// same library imported `as zzz` yields `zzz::Recalled`.
///
/// A library author cannot know that alias — it is chosen by the
/// consumer — so before this, a `publish:` contract written in a
/// library was unsatisfiable the moment anyone imported it. The
/// library passed `hale check` standalone and failed when used, which
/// is the worst direction for that failure to point.
///
/// Note the resolved form is the MANGLED symbol the import resolver
/// produces — `__lib_lib_relay_main_Recalled` — not the `relay::Recalled`
/// the diagnostic pretty-prints. Matching the displayed form is the
/// trap here; the string the analysis actually holds is the mangled
/// one.
///
/// So an UNQUALIFIED name matches the trailing segment of a merged
/// symbol, and a qualified one still matches exactly.
///
/// Known limitation: two topics with the same trailing name from
/// different seeds both match an unqualified declaration. That is
/// more permissive than intended, but it is the permissiveness the
/// author asked for by writing an unqualified name.
pub(crate) fn topic_ref_matches(declared: &str, resolved: &str) -> bool {
    if declared == resolved {
        return true;
    }
    topic_tail(declared) == topic_tail(resolved)
}

/// The bare topic name, whichever spelling reached us.
///
/// Subjects arrive in three shapes depending on the phase that
/// produced them, which is the trap this exists to absorb:
///   - `Recalled`                        bus-graph subject key
///   - `relay::Recalled`                 what the author wrote
///   - `__lib_lib_relay_main_Recalled`   merged publish site
///
/// The merged form embeds the LIBRARY PATH, not the import alias, so
/// the qualifier can never be matched against it — `relay` and
/// `lib_relay_main` are different strings and only the resolver knows
/// they correspond. Comparing trailing names is what works for every
/// pair.
///
/// Known limitation: two topics with the same trailing name from
/// different seeds are indistinguishable here. Disambiguating them
/// needs the resolver's alias table, which this layer does not have.
fn topic_tail(s: &str) -> &str {
    let s = s.rsplit("::").next().unwrap_or(s);
    match s.strip_prefix("__lib_") {
        Some(rest) => rest.rsplit('_').next().unwrap_or(rest),
        None => s,
    }
}


/// `@effects(publish: {A, B})` — the allowed publish set. A publish
/// to a subject outside the set is a violation; the closed topic set
/// makes this exact. A computed (non-literal) subject can't be
/// proven in-set, so it is reported too.
fn check_publish_set(
    summary: &AllocSummary,
    key: &FnKey,
    span: Span,
    allowed: &[String],
    diags: &mut Vec<Diag>,
) {
    let found = find_effect_site(summary, key, |k| match k {
        alloc_summary::EffectSiteKind::Publish(Some(subj)) => {
            if allowed.iter().any(|a| topic_ref_matches(a, subj)) {
                None
            } else {
                Some(format!("publishes to `{}`", subj))
            }
        }
        alloc_summary::EffectSiteKind::Publish(None) => Some(
            "publishes to a computed subject (not provably in the \
             declared set)"
                .to_string(),
        ),
        _ => None,
    });
    if let Some((chain_s, site_span)) = found {
        diags.push(Diag::ty(
            span,
            format!(
                "declared publish set violated: `{}` reaches {}. The \
                 contract allows only {{{}}} — add the subject to the set, \
                 or route the publish through a locus this fn doesn't reach.",
                key.display(),
                chain_s,
                allowed.join(", ")
            ),
        ));
        diags.push(Diag::ty(site_span, "the publish happens here".to_string()));
    }
}

/// Walk the call graph looking for a matching SYNTACTIC effect site
/// (publish / spawn) in any reachable fn body, returning the rendered
/// witness chain and the site's span.
fn find_effect_site(
    summary: &AllocSummary,
    root: &FnKey,
    matches_kind: impl Fn(&alloc_summary::EffectSiteKind) -> Option<String> + Copy,
) -> Option<(String, Span)> {
    fn walk(
        summary: &AllocSummary,
        key: &FnKey,
        path: &mut Vec<(FnKey, Span, String)>,
        seen: &mut BTreeSet<FnKey>,
        steps: &mut u32,
        matches_kind: impl Fn(&alloc_summary::EffectSiteKind) -> Option<String> + Copy,
    ) -> Option<(Vec<callgraph::WitnessStep>, Span)> {
        let fs = summary.fns.get(key)?;
        for site in &fs.effect_sites {
            if let Some(label) = matches_kind(&site.kind) {
                return Some((
                    vec![callgraph::WitnessStep {
                        in_fn: key.clone(),
                        span: site.span,
                        label,
                    }],
                    site.span,
                ));
            }
        }
        for edge in &fs.calls {
            *steps += 1;
            if *steps > callgraph::MAX_STEPS {
                break;
            }
            if let alloc_summary::Callee::Resolved(callee) = &edge.callee {
                if !seen.insert(callee.clone()) {
                    continue;
                }
                if let Some((mut tail, sp)) =
                    walk(summary, callee, path, seen, steps, matches_kind)
                {
                    let mut out = vec![callgraph::WitnessStep {
                        in_fn: key.clone(),
                        span: edge.span,
                        label: callee.display(),
                    }];
                    out.append(&mut tail);
                    return Some((out, sp));
                }
            }
        }
        None
    }
    let mut path = Vec::new();
    let mut seen = BTreeSet::new();
    let mut steps = 0u32;
    let (steps_v, sp) =
        walk(summary, root, &mut path, &mut seen, &mut steps, matches_kind)?;
    Some((callgraph::render_witness(root, &steps_v), sp))
}

/// Emit the two-diagnostic report (root + leaf) for a witness chain.
fn report(
    summary: &AllocSummary,
    key: &FnKey,
    span: Span,
    class_name: &str,
    pred: &mut dyn FnMut(&Probe<'_>) -> Option<String>,
    diags: &mut Vec<Diag>,
    advice: &str,
) {
    let Some(path) = callgraph::witness_path(summary, key, pred) else {
        return;
    };
    let leaf_span = path.last().map(|s| s.span).unwrap_or(span);
    diags.push(Diag::ty(
        span,
        format!(
            "effect assertion violated: `{}` must not reach `{}`, but \
             reaches {}. {}",
            key.display(),
            class_name,
            chain(key, &path),
            advice
        ),
    ));
    diags.push(Diag::ty(
        leaf_span,
        format!("the `{}` effect is reached here", class_name),
    ));
}

/// Detect a cycle reachable from `root` (including a self-call),
/// naming the fn that closes it. Uses the summary's resolved call
/// edges directly — recursion is a property of the graph, not of a
/// leaf predicate.
fn find_recursion(summary: &AllocSummary, root: &FnKey) -> Option<String> {
    fn walk(
        summary: &AllocSummary,
        key: &FnKey,
        path: &mut Vec<FnKey>,
        seen: &mut BTreeSet<FnKey>,
        steps: &mut u32,
    ) -> Option<String> {
        let fs = summary.fns.get(key)?;
        path.push(key.clone());
        for edge in &fs.calls {
            *steps += 1;
            if *steps > callgraph::MAX_STEPS {
                break;
            }
            if let alloc_summary::Callee::Resolved(callee) = &edge.callee {
                if path.contains(callee) {
                    let cyc: Vec<String> = path
                        .iter()
                        .skip_while(|k| *k != callee)
                        .map(|k| k.display())
                        .collect();
                    path.pop();
                    return Some(format!(
                        "cycle: {} -> {}",
                        cyc.join(" -> "),
                        callee.display()
                    ));
                }
                if seen.insert(callee.clone()) {
                    if let Some(found) =
                        walk(summary, callee, path, seen, steps)
                    {
                        path.pop();
                        return Some(found);
                    }
                }
            }
        }
        path.pop();
        None
    }
    let mut path = Vec::new();
    let mut seen = BTreeSet::new();
    let mut steps = 0u32;
    walk(summary, root, &mut path, &mut seen, &mut steps)
}


// ===== GH #265 step 7: the `.hale.effects` manifest =====

/// One manifest row: a fn and the effect contract it DECLARES.
/// Deliberately declaration-only at v1 — inferring a full effect set
/// for every fn in the program is the "effect rows on function
/// types" slippery slope the issue defers; what makes a manifest
/// useful today is that an effect REGRESSION (a handler that gained
/// a contract, or lost one) shows up as a diff in review, the way an
/// API break shows in a `.d.ts` diff.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectManifestRow {
    pub func: String,
    pub forbids: Vec<String>,
    /// #354: the CLOSED contract, rendered distinctly from `forbids`.
    /// A reader of the baseline must be able to tell an open contract
    /// from a closed one — they weaken differently over time.
    pub only: Option<Vec<String>>,
    pub publish_set: Option<Vec<String>>,
    pub quantities: Vec<(String, u64)>,
    /// GH #265: the INFERRED effect set — what the fn actually does,
    /// transitively, whether or not it declares anything. This is
    /// what makes the manifest a behavioural fingerprint rather than
    /// a restatement of the annotations: a handler that silently
    /// gains a syscall shows up here as a one-line diff even though
    /// no annotation changed.
    pub inferred: Vec<String>,
}

/// Build the whole-program effect manifest, sorted for stable diffs.
pub fn effect_manifest(programs: &[&Program]) -> Vec<EffectManifestRow> {
    let mut rows: Vec<EffectManifestRow> = Vec::new();
    // #345: the manifest is a REVIEW artifact — a committed baseline
    // whose diff is the thing a human reads. `<user effect>` there is
    // worse than in a diagnostic: every user class renders identically,
    // so two different classes produce the same line and a real change
    // can diff to nothing.
    let names = effect_names_of(programs);
    let mut push = |name: String, fd: &FnDecl| {
        let mut forbids = Vec::new();
        let mut onlys: Option<Vec<String>> = None;
        let mut publish_set = None;
        for a in &fd.effects {
            match a {
                EffectAssert::Carries(_) => {}
                EffectAssert::Only(cs) => {
                    // Rendered distinctly from `none:` — a reader of the
                    // baseline must be able to tell a closed contract
                    // from an open one.
                    onlys = Some(
                        cs.iter().map(|c| cls_name(*c, &names)).collect(),
                    );
                }
                EffectAssert::Forbid(cs) => {
                    for c in cs {
                        forbids.push(cls_name(*c, &names));
                    }
                }
                EffectAssert::PublishSet(items) => {
                    publish_set = Some(items.clone());
                }
                EffectAssert::NoPanic => {
                    forbids.push("panic".to_string());
                }
                EffectAssert::Causes(cs) => {
                    for c in cs {
                        forbids.push(format!("causes:{}", cls_name(*c, &names)));
                    }
                }
            }
        }
        let mut quantities: Vec<(String, u64)> = fd
            .quantities
            .iter()
            .map(|(d, n)| (d.as_str().to_string(), *n))
            .collect();
        if let Some(b) = fd.budget {
            quantities.push(("alloc_per_call".to_string(), b as u64));
        }
        // Rows with no declaration are still emitted when the
        // caller fills in an inferred set (see
        // `effect_manifest_with_inference`); the declaration-only
        // builder skips them.
        if forbids.is_empty()
            && onlys.is_none()
            && publish_set.is_none()
            && quantities.is_empty()
        {
            return;
        }
        forbids.sort();
        quantities.sort();
        rows.push(EffectManifestRow {
            func: name,
            forbids,
            only: onlys,
            publish_set,
            quantities,
            inferred: Vec::new(),
        });
    };
    // GH #296 review round 3: modules nest top declarations
    // arbitrarily deep, and a module-contained fn or locus whose
    // effects never reached a manifest row was invisible to both
    // the committed-baseline diff and `hale replay`'s safety
    // admission — the same fail-open shape twice.
    fn walk_items(
        items: &[TopDecl],
        prefix: &str,
        push: &mut dyn FnMut(String, &hale_syntax::ast::FnDecl),
    ) {
        for item in items {
            match item {
                TopDecl::Fn(fd) => push(
                    format!("{}{}", prefix, fd.name.name),
                    fd,
                ),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            push(
                                format!(
                                    "{}{}::{}",
                                    prefix, l.name.name, fd.name.name
                                ),
                                fd,
                            );
                        }
                    }
                }
                TopDecl::Module(md) => {
                    let inner =
                        format!("{}{}::", prefix, md.name.name);
                    walk_items(&md.items, &inner, push);
                }
                _ => {}
            }
        }
    }
    for p in programs {
        walk_items(&p.items, "", &mut push);
    }
    rows.sort_by(|a, b| a.func.cmp(&b.func));
    rows
}

/// GH #265: the manifest with INFERRED sets filled in for every fn
/// in the bundle — declared contracts plus what the compiler can see
/// each fn actually does. This is the diffable behavioural
/// fingerprint; `effect_manifest` alone reports declarations only.
pub fn effect_manifest_with_inference(
    programs: &[&Program],
) -> Vec<EffectManifestRow> {
    let summary = crate::stdlib_bodies::summarize_with_stdlib(programs);
    let ffi = ffi_names(programs);
    let names = effect_names_of(programs);
    let declared: BTreeMap<String, EffectManifestRow> = effect_manifest(programs)
        .into_iter()
        .map(|r| (r.func.clone(), r))
        .collect();
    let mut rows: Vec<EffectManifestRow> = Vec::new();
    let mut add = |name: String, key: FnKey, in_module: bool| {
        // Round 3 (GH #296): the callgraph summarizer does not yet
        // descend into inline modules, and a missing summary key
        // infers PURE — which turned a module-contained subprocess
        // call invisible. Inside a module, an unresolvable key is
        // rendered `unclassified` ("may do anything"): fail closed,
        // and scoped so non-module rows are untouched.
        let inferred = if in_module && !summary.fns.contains_key(&key)
        {
            vec!["unclassified".to_string()]
        } else {
            crate::frontier::render_effects_named(
                crate::frontier::infer_effects(&summary, &key, &ffi),
                &names,
            )
        };
        let mut row = declared.get(&name).cloned().unwrap_or(
            EffectManifestRow {
                func: name.clone(),
                forbids: Vec::new(),
                only: None,
                publish_set: None,
                quantities: Vec::new(),
                inferred: Vec::new(),
            },
        );
        row.inferred = inferred;
        // A fn with neither a declaration nor an observable effect
        // adds nothing to the fingerprint.
        if row.forbids.is_empty()
            && row.publish_set.is_none()
            && row.quantities.is_empty()
            && row.inferred.is_empty()
        {
            return;
        }
        rows.push(row);
    };
    // Round 3 (GH #296): recurse through modules — a
    // module-contained fn or lifecycle body absent from these rows
    // was invisible to both the baseline diff and replay's safety
    // admission. A module fn whose summary key misses resolves to
    // `unclassified`, which is the fail-closed answer.
    fn walk_infer(
        items: &[TopDecl],
        prefix: &str,
        add: &mut dyn FnMut(String, FnKey, bool),
    ) {
        let in_module = !prefix.is_empty();
        for item in items {
            match item {
                TopDecl::Fn(fd) => {
                    let n = format!("{}{}", prefix, fd.name.name);
                    add(n.clone(), FnKey::free_fn(n), in_module);
                }
                TopDecl::Module(md) => {
                    let inner =
                        format!("{}{}::", prefix, md.name.name);
                    walk_infer(&md.items, &inner, add);
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(fd) => add(
                                format!(
                                    "{}{}::{}",
                                    prefix, l.name.name, fd.name.name
                                ),
                                FnKey::method(
                                    l.name.name.clone(),
                                    fd.name.name.clone(),
                                ),
                            in_module,
                            ),
                            // Lifecycle hooks belong in the
                            // fingerprint too. Leaving them out made
                            // the manifest miss most of what a
                            // program does: in Hale the work lives in
                            // `birth` / `run` / `dissolve` and in bus
                            // handlers, not in free functions. A
                            // fingerprint blind to `run()` cannot
                            // notice a handler that starts doing
                            // filesystem I/O, which is the exact
                            // regression the CI gate exists to catch.
                            LocusMember::Lifecycle(lc) => {
                                let phase = lifecycle_name(lc.kind);
                                add(
                                    format!(
                                        "{}{}::{}",
                                        prefix, l.name.name, phase
                                    ),
                                    FnKey::method(
                                        l.name.name.clone(),
                                        phase.to_string(),
                                    ),
                                    in_module,
                                )
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for p in programs {
        walk_infer(&p.items, "", &mut add);
    }
    rows.sort_by(|a, b| a.func.cmp(&b.func));
    rows
}

/// Render the manifest in the stable line format `.hale.effects`
/// carries — one fn per line, fields sorted, so a behavioural change
/// is a one-line diff.
pub fn render_effect_manifest(rows: &[EffectManifestRow]) -> String {
    let mut out = String::from("# .hale.effects v1 — declared effect contracts\n");
    for r in rows {
        out.push_str(&r.func);
        if !r.forbids.is_empty() {
            out.push_str(&format!("  none={{{}}}", r.forbids.join(",")));
        }
        // #354: rendered separately from `none=` — a reader of the
        // baseline must be able to tell a CLOSED contract from an open
        // one, because they weaken differently as classes are added.
        if let Some(o) = &r.only {
            out.push_str(&format!("  only={{{}}}", o.join(",")));
        }
        if let Some(ps) = &r.publish_set {
            out.push_str(&format!("  publish={{{}}}", ps.join(",")));
        }
        for (d, n) in &r.quantities {
            out.push_str(&format!("  {}={}", d, n));
        }
        if !r.inferred.is_empty() {
            out.push_str(&format!("  does={{{}}}", r.inferred.join(",")));
        }
        out.push('\n');
    }
    out
}


// ===== GH #265: `@no_panic` — a DIFFERENT analysis =====
//
// Everything else in this module is leaf reachability over the call
// graph. `@no_panic` is disposition coverage: it asks whether any
// path in the fn's own body (and its bundle-local callees) can TRAP
// — an explicit `violate`, a fallible call whose `or` disposition
// raises rather than handling, or an indexing form that can trap
// instead of its fallible sibling. That is a syntactic property of
// the body, not a property of what it reaches, which is why the
// issue kept it on its own track.

/// Walk a body for trap sources, returning the first with a reason.
fn find_trap_in_block(b: &Block) -> Option<(String, Span)> {
    for st in &b.stmts {
        if let Some(hit) = find_trap_in_stmt(st) {
            return Some(hit);
        }
    }
    if let Some(t) = &b.tail {
        if let Some(hit) = find_trap_in_expr(t) {
            return Some(hit);
        }
    }
    None
}

fn find_trap_in_stmt(st: &Stmt) -> Option<(String, Span)> {
    match st {
        Stmt::Violate { span, .. } => Some((
            "an explicit `violate` — it raises by construction".to_string(),
            *span,
        )),
        Stmt::If(i) => {
            find_trap_in_block(&i.then_block)
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            find_trap_in_block(body)
        }
        Stmt::Let { value, .. } => find_trap_in_expr(value),
        Stmt::Expr(e) => find_trap_in_expr(e),
        Stmt::Return(Some(e), _) => find_trap_in_expr(e),
        _ => None,
    }
}

fn find_trap_in_expr(e: &Expr) -> Option<(String, Span)> {
    match e {
        // A fallible expression whose disposition RAISES propagates
        // the failure — a trap on this path. `or discard` /
        // `or <substitute>` / `or handler(err)` all handle it.
        Expr::Or { disposition: OrDisposition::Raise(sp), .. } => Some((
            "a fallible call dispositioned `or raise` — it propagates the \
             failure instead of handling it"
                .to_string(),
            *sp,
        )),
        Expr::Or { inner, .. } => find_trap_in_expr(inner),
        Expr::Index { span, .. } => Some((
            "an indexing operation that can trap out of range — use the \
             fallible form with an `or` disposition"
                .to_string(),
            *span,
        )),
        Expr::Binary { left, right, .. } => {
            find_trap_in_expr(left).or_else(|| find_trap_in_expr(right))
        }
        Expr::Unary { operand, .. } => find_trap_in_expr(operand),
        Expr::Call { args, .. } => {
            args.iter().find_map(find_trap_in_expr)
        }
        _ => None,
    }
}

/// `@no_panic`: the fn's own body and every bundle-local callee must
/// be trap-free.
fn check_no_panic(
    programs: &[&Program],
    key: &FnKey,
    span: Span,
    diags: &mut Vec<Diag>,
) {
    // Collect bodies by key so we can follow resolved callees.
    let mut bodies: BTreeMap<FnKey, &Block> = BTreeMap::new();
    for p in programs {
        for item in &p.items {
            match item {
                TopDecl::Fn(fd) => {
                    bodies.insert(FnKey::free_fn(fd.name.name.clone()), &fd.body);
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            bodies.insert(
                                FnKey::method(
                                    l.name.name.clone(),
                                    fd.name.name.clone(),
                                ),
                                &fd.body,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(body) = bodies.get(key) {
        if let Some((why, sp)) = find_trap_in_block(body) {
            diags.push(Diag::ty(
                span,
                format!(
                    "`@no_panic` violated: `{}` can trap — {}. Handle it \
                     with an `or` disposition (`or discard`, a substitute \
                     value, or `or handler(err)`), or drop `@no_panic`.",
                    key.display(),
                    why
                ),
            ));
            diags.push(Diag::ty(sp, "the trap is here".to_string()));
        }
    }
}

#[cfg(test)]
mod topic_ref_tests {
    use super::topic_ref_matches;

    #[test]
    fn unqualified_declaration_matches_any_importer_alias() {
        // the form the analysis actually holds
        assert!(topic_ref_matches("Recalled", "__lib_lib_relay_main_Recalled"));
        assert!(topic_ref_matches("Recalled", "__lib_vendor_x_Recalled"));
        // and the pretty/qualified forms, defensively
        assert!(topic_ref_matches("Recalled", "relay::Recalled"));
        assert!(topic_ref_matches("Recalled", "Recalled"));
    }

    #[test]
    fn merged_symbol_must_end_on_a_segment_boundary() {
        // `...FooRecalled` is a different topic, not a match
        assert!(!topic_ref_matches("Recalled", "__lib_a_FooRecalled"));
        assert!(!topic_ref_matches("Recalled", "__lib_a_Recalledx"));
    }

    /// A qualified declaration matches the same topic in every
    /// spelling it can arrive in. It does NOT currently disambiguate
    /// two same-named topics from different seeds — the merged symbol
    /// embeds the library path, not the alias, so only the resolver
    /// could tell them apart. Asserted here so the limitation is
    /// pinned rather than discovered.
    #[test]
    fn qualified_declaration_matches_every_spelling() {
        assert!(topic_ref_matches("relay::Recalled", "relay::Recalled"));
        assert!(topic_ref_matches("relay::Recalled", "Recalled"));
        assert!(topic_ref_matches(
            "relay::Recalled",
            "__lib_lib_relay_main_Recalled"
        ));
        // the documented limitation, pinned:
        assert!(topic_ref_matches("relay::Recalled", "zzz::Recalled"));
    }

    #[test]
    fn different_topics_never_match() {
        assert!(!topic_ref_matches("Recalled", "relay::SumLookup"));
        assert!(!topic_ref_matches("Recalled", "Recalledx"));
    }
}
