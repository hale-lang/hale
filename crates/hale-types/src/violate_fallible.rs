//! A value-returning fn that may `violate` is `fallible(ClosureViolation)`
//! (decision F.42): its callers say what happens when the locus fails,
//! and the call takes its failure path instead of handing them a value
//! nothing computed.
//!
//! "May violate" is read off the closed call graph, the allocation
//! summary's ([`crate::alloc_summary`]: the checked programs and the
//! stdlib's analysis copy, cross-seed calls resolved): a `violate`
//! statement in the body, or a resolved call to a VALUE-RETURNING fn that
//! may violate and is not `fallible`. A violation in a method that returns
//! nothing is not the caller's failure (the method exits as a return does
//! and the caller goes on to compute its own value), so a call to one
//! carries nothing; a `fallible` callee is answered at its call by the
//! bare-call law (GH #738). Once the law holds no callee of the first kind
//! is left, so the inference is the direct statement in practice. An edge
//! the summary cannot resolve contributes nothing.
//!
//! Three judgments over the program's own fns and locus methods:
//!
//! - a value-returning one that may violate and is not `fallible` is
//!   refused (the law);
//! - one that may violate and is `fallible(E)` with `E` other than
//!   `ClosureViolation` is refused: a fn has one error type, and the
//!   violation record is the one a violator carries;
//! - a Unit one that may violate and is not `fallible` is warned about
//!   (the lint, which a later release makes the law).
//!
//! Lifecycle bodies, bus handlers and `fn main` are exempt: the runtime
//! is their caller. The stdlib's analysis copy is a callee only; its own
//! fns are held to the law by `tests/violate_fallible.rs`.
//!
//! A locus method that serves a perspective fn (`serves`, matched by name
//! as conformance matches it) is exempt from the law and the lint, and
//! carries nothing to its callers, until a perspective call can carry the
//! failure: conformance holds the method to the perspective fn's
//! fallibility, and the build refuses a `fallible` perspective call, so no
//! spelling of it both checks and runs. It is warned about instead, and
//! keeps today's lowering.

use std::collections::BTreeMap;

use hale_syntax::ast::{flat_decls, LocusMember, PerspectiveMember, Program, TopDecl, TypeExpr};
use hale_syntax::{Diag, Span};

use crate::alloc_summary::{AllocSummary, Callee, DeclId, EntryKind, FnKey};
use crate::symbol::Bundle;

/// Why a fn may violate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MayViolate {
    /// A `violate` statement in its own body: the closure it names, where.
    Direct { closure: String, span: Span },
    /// A call, at `call`, to `callee`, which may violate and is not
    /// `fallible`.
    Through { callee: FnKey, call: Span },
}

/// What a declaration says about its failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declared {
    /// No `fallible` clause.
    Infallible,
    /// `fallible(ClosureViolation)`.
    Violation,
    /// `fallible(E)` with another `E`, as written.
    Other(String),
}

/// One fn or locus method declaration the judgments read.
#[derive(Debug, Clone)]
pub struct FnDeclRow {
    /// Its display name: `name` for a free fn, `Locus.name` for a method
    /// (a stdlib locus in its public spelling).
    pub name: String,
    pub name_span: Span,
    /// It returns a value (a declared return type other than `()`).
    pub returns_value: bool,
    pub declared: Declared,
    /// It is the stdlib analysis copy's.
    pub stdlib: bool,
    /// The perspective fn it serves, as `Perspective.fn`: a method of a
    /// locus that `serves` a perspective declaring a fn of its name.
    pub serves: Option<String>,
}

/// The rows of every fn and locus method of `programs` (the checked ones)
/// and of the stdlib's analysis copy, keyed as the summary keys them.
pub fn fn_decl_rows(programs: &[&Program]) -> BTreeMap<FnKey, FnDeclRow> {
    // Each perspective's fns, by its name (a `serves` names it as
    // written, qualified or not, and is matched by its last segment).
    let mut perspectives: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for p in programs {
        for item in flat_decls(&p.items) {
            if let TopDecl::Perspective(p) = item {
                let fns = p.members.iter().filter_map(|m| match m {
                    PerspectiveMember::Fn(f) => Some(f.name.name.as_str()),
                    _ => None,
                });
                perspectives.entry(p.name.name.as_str()).or_default().extend(fns);
            }
        }
    }
    let mut out = BTreeMap::new();
    for p in programs {
        collect(p, false, &perspectives, &mut out);
    }
    if let Some(p) = crate::stdlib_bodies::program() {
        collect(p, true, &BTreeMap::new(), &mut out);
    }
    out
}

fn collect(
    program: &Program,
    stdlib: bool,
    perspectives: &BTreeMap<&str, Vec<&str>>,
    out: &mut BTreeMap<FnKey, FnDeclRow>,
) {
    let decl = |node| if stdlib { DeclId::stdlib(node) } else { DeclId::user(node) };
    let row = |name: String, f: &hale_syntax::ast::FnDecl| FnDeclRow {
        name,
        name_span: f.name.span,
        returns_value: match &f.ret {
            None => false,
            Some(TypeExpr::Tuple(parts, _)) => !parts.is_empty(),
            Some(_) => true,
        },
        declared: match &f.fallible {
            None => Declared::Infallible,
            Some(TypeExpr::Named { path, .. })
                if path.segments.last().is_some_and(|s| s.name == "ClosureViolation") =>
            {
                Declared::Violation
            }
            Some(t) => Declared::Other(type_text(t)),
        },
        stdlib,
        serves: None,
    };
    for item in flat_decls(&program.items) {
        match item {
            TopDecl::Fn(f) => {
                out.insert(FnKey::free_fn(decl(f.id), f.name.name.clone()), row(f.name.name.clone(), f));
            }
            TopDecl::Locus(l) => {
                let shown = public_locus_name(&l.name.name);
                for m in &l.members {
                    if let LocusMember::Fn(f) = m {
                        let serves = l.serves.iter().find_map(|p| {
                            let last = p.name.rsplit("::").next().unwrap_or(&p.name);
                            let fns = perspectives.get(last)?;
                            fns.contains(&f.name.name.as_str()).then(|| format!("{}.{}", p.name, f.name.name))
                        });
                        out.insert(
                            FnKey::method(decl(f.id), l.name.name.clone(), f.name.name.clone()),
                            FnDeclRow { serves, ..row(format!("{shown}.{}", f.name.name), f) },
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// `__StdBytesBytesBuilder` → `std::bytes::BytesBuilder`; any other name
/// as it is.
fn public_locus_name(name: &str) -> String {
    hale_stdlib::PATH_RENAMES
        .iter()
        .find(|(_, mangled)| *mangled == name)
        .map(|(path, _)| path.join("::"))
        .unwrap_or_else(|| name.to_string())
}

fn type_text(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named { path, .. } => {
            path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")
        }
        _ => "…".to_string(),
    }
}

/// Every fn of `summary` that may violate, with why: the fixpoint of a
/// `violate` in the body and a resolved call to a value-returning fn that
/// may violate and is not `fallible`. A fn's reason is the first its body gives, in body
/// order, at the shortest distance from a `violate`.
pub fn may_violate(summary: &AllocSummary, decls: &BTreeMap<FnKey, FnDeclRow>) -> BTreeMap<FnKey, MayViolate> {
    let mut found: BTreeMap<FnKey, MayViolate> = BTreeMap::new();
    for (key, fs) in &summary.fns {
        if let Some((closure, span)) = fs.violates.first() {
            found.insert(key.clone(), MayViolate::Direct { closure: closure.clone(), span: *span });
        }
    }
    // A callee whose violation is its caller's: it returns a value and
    // has no failure path to report it on. A perspective-served method
    // keeps today's behaviour, so it carries nothing.
    let carries = |k: &FnKey| {
        decls.get(k).is_some_and(|d| d.returns_value && d.declared == Declared::Infallible && d.serves.is_none())
    };
    loop {
        let mut next = Vec::new();
        for (key, fs) in &summary.fns {
            if found.contains_key(key) {
                continue;
            }
            let through = fs.calls.iter().find_map(|edge| match &edge.callee {
                Callee::Resolved(c) if found.contains_key(c) && carries(c) => {
                    Some(MayViolate::Through { callee: c.clone(), call: edge.span })
                }
                _ => None,
            });
            if let Some(why) = through {
                next.push((key.clone(), why));
            }
        }
        if next.is_empty() {
            return found;
        }
        found.extend(next);
    }
}

/// The runtime is the caller of `main`, of a bus handler and of a
/// lifecycle body (which is no fn declaration, so never judged).
fn exempt(summary: &AllocSummary, key: &FnKey) -> bool {
    (key.locus.is_none() && key.fn_name == "main")
        || summary
            .fns
            .get(key)
            .is_some_and(|fs| matches!(fs.entry, Some(EntryKind::Main | EntryKind::BusHandler)))
}

/// One fn's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Value-returning, may violate, not `fallible`: refused.
    Law,
    /// May violate, `fallible(E)` with another `E`: refused.
    WrongError(String),
    /// Unit, may violate, not `fallible`: warned.
    Lint,
    /// Serves a perspective fn, may violate, not `fallible`: exempt from
    /// the law and the lint until a perspective call can carry the
    /// failure, and warned about.
    Serves(String),
}

/// The verdict on each of the program's own fns that may violate and is
/// not answered by its declaration, with why it may violate.
pub fn verdicts(
    summary: &AllocSummary,
    decls: &BTreeMap<FnKey, FnDeclRow>,
    may: &BTreeMap<FnKey, MayViolate>,
) -> Vec<(FnKey, Verdict)> {
    let mut out = Vec::new();
    for key in may.keys() {
        let Some(d) = decls.get(key) else { continue };
        if d.stdlib || exempt(summary, key) {
            continue;
        }
        let verdict = match &d.declared {
            Declared::Violation => continue,
            Declared::Other(e) => Verdict::WrongError(e.clone()),
            Declared::Infallible if d.serves.is_some() => Verdict::Serves(d.serves.clone().unwrap_or_default()),
            Declared::Infallible if d.returns_value => Verdict::Law,
            Declared::Infallible => Verdict::Lint,
        };
        out.push((key.clone(), verdict));
    }
    out
}

/// The law, the wrong-error refusal, the lint and the perspective-served
/// exemption over `bundle`'s own fns, as diagnostics: errors for the first
/// two, warnings for the others.
pub fn violate_fallible_laws(bundle: &Bundle<'_>, summary: &AllocSummary) -> Vec<Diag> {
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let decls = fn_decl_rows(&programs);
    let may = may_violate(summary, &decls);
    let mut diags = Vec::new();
    for (key, verdict) in verdicts(summary, &decls, &may) {
        let d = &decls[&key];
        let how = how(&key, &may, &decls);
        let message = match &verdict {
            Verdict::Law => format!(
                "`{}` returns a value and may violate ({how}): declare it `fallible(ClosureViolation)`; \
                 its callers then say what happens when the locus fails",
                d.name
            ),
            Verdict::WrongError(e) => format!(
                "`{}` may violate ({how}) and is declared `fallible({e})`: a fn that may violate is \
                 `fallible(ClosureViolation)`, and a fn has one error type",
                d.name
            ),
            Verdict::Lint => format!(
                "`{}` may violate ({how}): declare it `fallible(ClosureViolation)`; its callers then \
                 say what happens when the locus fails. This becomes a law in a later release",
                d.name
            ),
            Verdict::Serves(p) => format!(
                "`{}` serves `{p}` and may violate ({how}); a perspective call cannot carry the failure yet, \
                 so the method keeps today's behaviour{}",
                d.name,
                if d.returns_value { " (the caller of a violated perspective call must not read the value)" } else { "" }
            ),
        };
        let mut diag = match verdict {
            Verdict::Lint | Verdict::Serves(_) => Diag::warn(d.name_span, message),
            _ => Diag::ty(d.name_span, message),
        };
        for (span, label, stdlib) in path_notes(&key, &may, &decls) {
            diag = if stdlib { diag.with_stdlib_related(span, label) } else { diag.with_related(span, label) };
        }
        diags.push(diag);
    }
    diags
}

/// The parenthesis of the message: the closure a direct `violate` names,
/// or the callee the path goes through.
fn how(key: &FnKey, may: &BTreeMap<FnKey, MayViolate>, decls: &BTreeMap<FnKey, FnDeclRow>) -> String {
    match &may[key] {
        MayViolate::Direct { closure, .. } => format!("`violate {closure}`"),
        MayViolate::Through { callee, .. } => {
            let shown = decls.get(callee).map(|d| d.name.clone()).unwrap_or_else(|| callee.display());
            format!("through `{shown}`")
        }
    }
}

/// The path's locations, as related notes: the call the fn makes, and the
/// `violate` the path ends at, each with whether it is in the stdlib.
fn path_notes(
    key: &FnKey,
    may: &BTreeMap<FnKey, MayViolate>,
    decls: &BTreeMap<FnKey, FnDeclRow>,
) -> Vec<(Span, String, bool)> {
    let in_stdlib = |k: &FnKey| decls.get(k).is_some_and(|d| d.stdlib);
    let mut notes = Vec::new();
    let mut at = key.clone();
    let mut seen = vec![key.clone()];
    loop {
        match &may[&at] {
            MayViolate::Direct { closure, span } => {
                notes.push((*span, format!("`violate {closure}` here"), in_stdlib(&at)));
                return notes;
            }
            MayViolate::Through { callee, call } => {
                if notes.is_empty() {
                    let shown = decls.get(callee).map(|d| d.name.clone()).unwrap_or_else(|| callee.display());
                    notes.push((*call, format!("the call to `{shown}`"), in_stdlib(&at)));
                }
                if seen.contains(callee) {
                    return notes;
                }
                seen.push(callee.clone());
                at = callee.clone();
            }
        }
    }
}
