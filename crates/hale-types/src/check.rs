//! Type checking — milestone 2 cut.
//!
//! Walks every program in the bundle and verifies a tractable
//! subset of the type rules:
//!
//! - Literal expressions get their natural primitive type.
//! - Binary / unary operator operand-type compatibility.
//! - `let x: T = e;` — e's inferred type assignable to T.
//! - Struct-literal field names + types match the type
//!   declaration.
//! - Bus send (`"subject" <- v`): subject is declared in the
//!   enclosing locus's bus block, payload type matches.
//! - `~~` closure assertion: left and right have compatible
//!   types; tolerance is numeric-ish (we don't enforce strictly
//!   in milestone 2 — just that something is there).
//! - `self.field`: resolves against enclosing locus's params.
//!
//! Names referenced via paths the bundle can't see resolve to
//! `Ty::Unknown`, which is bidirectionally compatible. GH #470
//! tightened the STDLIB half of that tolerance: the Hale-source
//! stdlib surface is registered into the top scope (signatures
//! only — its bodies are never checked here), so `std::…` type
//! exprs, literals, fields, methods, and interface coercions
//! check for real, and a `std::` literal that resolves nowhere is
//! an error. Rust-implemented builtins keep the tolerance (their
//! path-call NAMES are validated by `stdlib_surface`).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::resolve::{resolve_type_expr, KnownNames, TopScope};
use crate::symbol::*;
use crate::ty::{is_flat_shapeable, is_key_eligible, Ty};

fn method_to_fn_ty(m: &MethodInfo) -> Ty {
    Ty::Function {
        params: m.params.clone(),
        ret: Box::new(m.ret.clone()),
    }
}

/// GH #734 — the member names the compiler injects on EVERY locus,
/// with the phrase that explains each one in a diagnostic.
///
/// `self.children` (the accept'd-child collection), `self.k_max`
/// (F.1 displacement bound) and `self.draining` (F.27 drain flag)
/// are resolved before any declared member of the same name, in
/// both `field_ty` here and the field lowering in codegen. A locus
/// that declares one of these spellings therefore has a member it
/// can never read back: the read takes the synthetic member's type
/// and lowering instead, and the program fails somewhere else — as
/// an unrelated type mismatch at the USE site (`expected Int, got
/// [?]`), or, when the declared type happens to match, as a codegen
/// error about params the locus never declared. Reserving the names
/// at the declaration keeps the failure at the one line that can fix
/// it, and keeps a collision from reaching codegen at all.
const SYNTHETIC_LOCUS_MEMBERS: [(&str, &str); 3] = [
    (
        "children",
        "the accept'd-child collection every locus carries \
         (`for c in self.children`, `self.children.count`)",
    ),
    (
        "k_max",
        "the F.1 displacement bound every locus carries \
         (`self.k_max`)",
    ),
    (
        "draining",
        "the F.27 drain flag every locus carries (`self.draining`)",
    ),
];

/// The explanatory phrase for `name` iff it is a synthetic locus
/// member, else `None`.
fn synthetic_locus_member(name: &str) -> Option<&'static str> {
    SYNTHETIC_LOCUS_MEMBERS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, what)| *what)
}

/// Push the GH #734 reserved-member diagnostic at `id`'s span when
/// `id` spells a synthetic locus member. `kind` names what was
/// declared ("params field", "method", "capacity slot").
fn reserved_member_diag(diags: &mut Vec<Diag>, kind: &str, id: &Ident) {
    let Some(what) = synthetic_locus_member(&id.name) else {
        return;
    };
    diags.push(Diag::ty(
        id.span,
        format!(
            "{kind} `{n}`: `{n}` is a reserved locus member — it names \
             {what}, and the compiler resolves that before anything a \
             locus declares, so `self.{n}` never reaches this {kind}. \
             Rename it (`own_{n}`, say).",
            kind = kind,
            n = id.name,
            what = what,
        ),
    ));
}

/// Phase 3 migration: two loci have the same *footprint* iff their
/// params (the user-visible state fields) match by name and type in
/// declaration order. A state-preserving `reperspective` keeps the
/// current impl's data and swaps only the vtable, so the new impl's
/// field accesses land on the retained state — sound exactly when
/// the footprints are identical.
fn footprints_match(a: &[ParamInfo], b: &[ParamInfo]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.name == y.name && x.ty == y.ty)
}

/// GH #831 — the declaration a CONSTRUCTION path names.
///
/// `type Row2 = Row;` makes `Row2` a second spelling of `Row`, not a
/// nominal type of its own (GH #759, spec `types.md` § "Type
/// aliases"), so a struct / locus literal (`Row2 { id: 1 }`) and an
/// enum-variant path (`Row2::V`) spelled with the alias name build
/// the target's declaration. `build_top_scope` has already expanded
/// every chain, so this is one hop.
///
/// `None` — and the caller keeps the name as written, with whatever
/// diagnostic it already produced — when the name is not an alias,
/// or when its target is not a NAME: nothing is constructible from
/// `type Thing = Int;` or `type TwoRows = [Row; 2];` with `{ }` or
/// `::`. Codegen's `resolve_construction_aliases` draws the same
/// line over the same declarations, which is what keeps `check` and
/// `build` from disagreeing about a literal.
fn construction_target(top: &TopScope, name: &str) -> Option<String> {
    match top.lookup(name) {
        Some(TopSymbol::Type(TypeInfo {
            kind: TypeKind::Alias(Ty::Named(target)),
            ..
        })) => Some(target.clone()),
        _ => None,
    }
}

/// True if the match arms cover every possible scrutinee
/// value. v0 rules:
///   - Any arm without a guard whose pattern is wildcard `_`
///     or a bare binding makes the match exhaustive.
///   - For Bool scrutinee: literal `true` AND literal `false`
///     arms (both unguarded) is also exhaustive.
///   - For an enum-typed scrutinee (m47): every declared variant
///     must be covered by an unguarded `EnumName::Variant`
///     constructor pattern.
///   - For everything else: a wildcard / binding is required.
fn match_is_exhaustive(scrut_ty: &Ty, arms: &[MatchArm], top: &TopScope) -> bool {
    let unguarded = |a: &&MatchArm| a.guard.is_none();
    let has_catchall = arms.iter().filter(unguarded).any(|a| {
        matches!(a.pattern, Pattern::Wildcard(_) | Pattern::Binding(_))
    });
    if has_catchall {
        return true;
    }
    if matches!(scrut_ty, Ty::Prim(PrimType::Bool)) {
        let mut has_true = false;
        let mut has_false = false;
        for arm in arms.iter().filter(unguarded) {
            if let Pattern::Literal(Literal::Bool(b), _) = &arm.pattern {
                if *b {
                    has_true = true;
                } else {
                    has_false = true;
                }
            }
        }
        return has_true && has_false;
    }
    if let Ty::Named(name) = scrut_ty {
        if let Some(TopSymbol::Type(TypeInfo {
            kind: TypeKind::Enum(variants),
            ..
        })) = top.symbols.get(name)
        {
            let mut covered: std::collections::BTreeSet<&str> =
                std::collections::BTreeSet::new();
            // m68: also accept arms whose enum_seg is a
            // synthesized monomorph of `name` — e.g. arms
            // written as `Result_Int_String::Ok` count as
            // covering `Ok` for a scrutinee typed as the
            // generic `Result` template. Codegen monomorphizes
            // generic enums into mangled-name decls
            // (`Result_Int_String`) but the typechecker only
            // sees the original template, so the user's match
            // arms (which use the mangled names that codegen
            // recognizes) would otherwise false-positive as
            // non-exhaustive. The mangle convention is
            // `<template>_<arg>_<arg>...` so the prefix check
            // is unambiguous.
            let mangle_prefix = format!("{}_", name);
            // GH #534: an importer's arm is `alias::Enum::Variant` —
            // three segments whose middle is the enum's authored name,
            // while the scrutinee is typed by the seed-mangled name
            // `__lib_<id>_<stem>_Enum`. The checker holds no alias
            // table (the same limitation the `__lib_` tail matching
            // elsewhere documents), so match on the mangled tail.
            for arm in arms.iter().filter(unguarded) {
                if let Pattern::Constructor { path, .. } = &arm.pattern {
                    let segs: Vec<&str> =
                        path.segments.iter().map(|s| s.name.as_str()).collect();
                    let (enum_seg, variant_seg) = match segs.as_slice() {
                        [e, v] => (*e, *v),
                        [_, e, v]
                            if name.starts_with("__lib_")
                                && name.ends_with(&format!("_{}", e)) =>
                        {
                            (name.as_str(), *v)
                        }
                        _ => continue,
                    };
                    {
                        // GH #831: the arm may spell the enum with
                        // an alias of it (`type C2 = Color; C2::Red
                        // -> ...`). An alias is a second spelling,
                        // so the arm covers the same variant —
                        // without this, a match whose arms all use
                        // the alias read as covering nothing, and
                        // one with a `_` arm checked clean and then
                        // failed to BUILD ("constructor pattern:
                        // unknown enum").
                        let resolved = construction_target(top, enum_seg);
                        let enum_seg: &str =
                            resolved.as_deref().unwrap_or(enum_seg);
                        let matches_template_or_monomorph =
                            enum_seg == *name
                                || enum_seg.starts_with(&mangle_prefix);
                        if matches_template_or_monomorph {
                            // m47-payloads: a Constructor arm
                            // covers its variant whether the
                            // sub-patterns are wildcards / bindings
                            // (catch-all over the payload) or
                            // empty (no-payload variant). Literal
                            // sub-patterns are narrower and
                            // wouldn't cover all values of the
                            // variant; we still treat them as
                            // covering for v0.1 — same permissive
                            // policy the Bool literal arms get.
                            covered.insert(variant_seg);
                        }
                    }
                }
            }
            return variants.iter().all(|v| covered.contains(v.name.as_str()));
        }
        // m68: a named type the typechecker doesn't know about
        // at all (commonly: a fully-mangled monomorph that
        // somehow flows in — codegen synthesizes those, the
        // typechecker doesn't see them) should be permissive
        // for exhaustiveness, same as Ty::Unknown. Narrowed
        // to "name not in top.symbols" so known structs / loci
        // / perspectives still require a wildcard / binding arm.
        if !top.symbols.contains_key(name) {
            return true;
        }
    }
    // Be permissive on Unknown — we genuinely can't say.
    matches!(scrut_ty, Ty::Unknown)
}

/// True if `e` is composed entirely of literals — no
/// identifiers, no `self`, no calls, no field access. Used by
/// closure-cycle-existence: a closure assertion with pure-
/// literal sides has nothing to audit.
fn is_pure_literal(e: &Expr) -> bool {
    match e {
        Expr::Literal(_, _) => true,
        Expr::Unary { operand, .. } => is_pure_literal(operand),
        Expr::Binary { left, right, .. } => {
            is_pure_literal(left) && is_pure_literal(right)
        }
        Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
            parts.iter().all(is_pure_literal)
        }
        _ => false,
    }
}

/// M3 stage 3: collect generic fn templates (recursing modules).
fn collect_generic_fns<'a>(
    items: &'a [TopDecl],
    out: &mut BTreeMap<String, &'a FnDecl>,
) {
    for item in items {
        match item {
            TopDecl::Fn(f) if !f.generics.is_empty() => {
                out.insert(f.name.name.clone(), f);
            }
            TopDecl::Module(m) => collect_generic_fns(&m.items, out),
            _ => {}
        }
    }
}

/// M3 stage 3 tranche 2: collect generic type templates.
fn collect_generic_types<'a>(
    items: &'a [TopDecl],
    out: &mut BTreeMap<String, &'a TypeDecl>,
) {
    for item in items {
        match item {
            TopDecl::Type(t) if !t.generics.is_empty() => {
                out.insert(t.name.name.clone(), t);
            }
            TopDecl::Module(m) => collect_generic_types(&m.items, out),
            _ => {}
        }
    }
}

/// GH #911 B5: collect generic LOCUS templates. The locus twin of
/// `collect_generic_types` — `locus Cache<K, V> { ... }` is
/// monomorphized exactly as `type Box<T> { ... }` is (one mangled
/// name per (template, type args) tuple, synthesized by codegen from
/// discovery), so the checker has to know both or it refuses
/// instantiations the build lowers.
fn collect_generic_loci<'a>(
    items: &'a [TopDecl],
    out: &mut BTreeMap<String, &'a LocusDecl>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) if !l.generics.is_empty() => {
                out.insert(l.name.name.clone(), l);
            }
            TopDecl::Module(m) => collect_generic_loci(&m.items, out),
            _ => {}
        }
    }
}

/// GH #911 B5: the declaration a mangled monomorph name (`Box_Int`,
/// `Cache_Int_String`) comes from.
///
/// Generic types and generic loci monomorphize the same way, so
/// resolving a mangled name is one function over both. What differs
/// is which SITES codegen can rewrite a bare template name at —
/// which is why the callers say whether a locus counts rather than
/// the lookup deciding for them. The site-by-site table is in
/// `crates/hale-codegen/tests/generic_monomorph_agreement.rs`.
#[derive(Clone, Copy)]
enum GenericTemplate<'a> {
    Type(&'a TypeDecl),
    Locus(&'a LocusDecl),
}

impl<'a> GenericTemplate<'a> {
    fn name(self) -> &'a str {
        match self {
            GenericTemplate::Type(t) => t.name.name.as_str(),
            GenericTemplate::Locus(l) => l.name.name.as_str(),
        }
    }

    fn generics(self) -> &'a [GenericParam] {
        match self {
            GenericTemplate::Type(t) => &t.generics,
            GenericTemplate::Locus(l) => &l.generics,
        }
    }

    /// The declaration keyword, for a diagnostic.
    fn kind(self) -> &'static str {
        match self {
            GenericTemplate::Type(_) => "type",
            GenericTemplate::Locus(_) => "locus",
        }
    }

    fn is_type(self) -> bool {
        matches!(self, GenericTemplate::Type(_))
    }
}

/// M3 stage 3 tranche 2: resolve one mangle token (`Int`, `Float`,
/// a user type name, or a nested mangled monomorph) to a Ty. The
/// codegen mangle joins tokens with `_`, so this stays permissive
/// (Unknown) for anything it can't confidently name.
fn mangle_token_to_ty(
    tok: &str,
    known: &KnownNames,
) -> Ty {
    match tok {
        "Int" => Ty::Prim(PrimType::Int),
        "Float" => Ty::Prim(PrimType::Float),
        "Bool" => Ty::Prim(PrimType::Bool),
        "String" => Ty::Prim(PrimType::String),
        "Duration" => Ty::Prim(PrimType::Duration),
        "Decimal" => Ty::Prim(PrimType::Decimal),
        "Time" => Ty::Prim(PrimType::Time),
        other => {
            if known.contains_key(other) {
                Ty::Named(other.to_string())
            } else {
                Ty::Unknown
            }
        }
    }
}

/// M3 stage 3: Ty-level mirror of codegen's m62
/// `unify_generic_param_bindings`. Binds generic names appearing in
/// `param_te` against the actual arg type. Top-level generic names
/// bind directly; Array/Bounded recurse on the element. Generic
/// names nested under generic-ARG'd Named types (Box<T> in param
/// position) stay unbound here — permissive, codegen's own unifier
/// still runs. Returns Err((name, existing, new)) on a conflict.
fn unify_generic_ty(
    param_te: &TypeExpr,
    arg: &Ty,
    generics: &std::collections::BTreeSet<String>,
    bindings: &mut BTreeMap<String, Ty>,
) -> Result<(), (String, Ty, Ty)> {
    match param_te {
        TypeExpr::Named { path, generic_args, .. }
            if generic_args.is_empty() && path.segments.len() == 1 =>
        {
            let name = &path.segments[0].name;
            if !generics.contains(name) {
                return Ok(());
            }
            if matches!(arg, Ty::Unknown) {
                return Ok(());
            }
            match bindings.get(name) {
                Some(existing) if existing != arg => Err((
                    name.clone(),
                    existing.clone(),
                    arg.clone(),
                )),
                Some(_) => Ok(()),
                None => {
                    bindings.insert(name.clone(), arg.clone());
                    Ok(())
                }
            }
        }
        TypeExpr::Array { elem, .. } => match arg {
            Ty::Array(a_elem, _) => {
                unify_generic_ty(elem, a_elem, generics, bindings)
            }
            _ => Ok(()),
        },
        TypeExpr::Bounded { elem, .. } => match arg {
            Ty::Bounded(a_elem, _) => {
                unify_generic_ty(elem, a_elem, generics, bindings)
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

/// M3 stage 3: resolve a template TypeExpr with generic bindings
/// applied — generic names map through `bindings`; everything else
/// through the ordinary resolver. Unbound generics (or shapes the
/// resolver can't see) fall to Unknown, keeping the checks
/// permissive exactly where inference was.
fn substitute_generic_ty(
    te: &TypeExpr,
    bindings: &BTreeMap<String, Ty>,
    known: &KnownNames,
) -> Ty {
    match te {
        TypeExpr::Named { path, generic_args, .. }
            if generic_args.is_empty() && path.segments.len() == 1 =>
        {
            if let Some(t) = bindings.get(&path.segments[0].name) {
                return t.clone();
            }
            resolve_type_expr(te, known)
        }
        TypeExpr::Array { elem, size, .. } => {
            let n = match size {
                Some(Expr::Literal(Literal::Int(n), _)) if *n >= 0 => {
                    Some(*n as u64)
                }
                _ => None,
            };
            Ty::Array(
                Box::new(substitute_generic_ty(elem, bindings, known)),
                n,
            )
        }
        TypeExpr::Bounded { elem, cap, .. } => Ty::Bounded(
            Box::new(substitute_generic_ty(elem, bindings, known)),
            *cap,
        ),
        _ => resolve_type_expr(te, known),
    }
}

pub fn check_bundle(
    bundle: &Bundle<'_>,
    top: &TopScope,
    allow_unowned_subscriber: bool,
) -> Vec<Diag> {
    check_bundle_scoped(bundle, top, allow_unowned_subscriber, false, false)
}

/// Two whole-program strictnesses, both off for a partial program.
///
/// `strict_callees`: refuse a call to a bare name nothing binds
/// (dna/FRICTION.md F.18) — the rule `hale build` holds. On only when
/// the caller checked a WHOLE seed, so a single file of a multi-file
/// seed, a styleguide snippet or a harness's partial program keeps the
/// permissive `Unknown` it always had for a sibling's fn.
///
/// `strict_idents` (GH #721): the same rule for a bare identifier in
/// VALUE position. Separate from the callee flag because the two have
/// different safe surfaces — the build path holds the identifier rule
/// (what it bundles is exactly what it compiles) without holding the
/// callee rule, which codegen already enforces for itself. (Until
/// GH #779 the callee rule also over-fired on bare names codegen
/// answers itself but `BARE_BUILTIN_CALLEES` did not list; that gap
/// is closed and tested.)
pub fn check_bundle_scoped(
    bundle: &Bundle<'_>,
    top: &TopScope,
    allow_unowned_subscriber: bool,
    strict_callees: bool,
    strict_idents: bool,
) -> Vec<Diag> {
    let mut diags = Vec::new();
    let known = collect_known_names(top, &bundle.import_renames);
    // WASM plan: the bundle targets wasm if any program declares
    // `target wasm` / `target browser_js`. Drives stdlib gating below.
    let wasm_target = bundle.programs.values().any(|p| {
        p.items.iter().any(|it| {
            matches!(it, TopDecl::Target(t)
                if matches!(t.name.name.as_str(), "wasm" | "browser_js"))
        })
    });
    // GH #255: bundle-wide set of transport-bound topic names,
    // for the `or wait` legality check at publish sites.
    //
    // GH #825: this one fails the other way. Every other walk in the
    // issue loses a finding when it stops at the top level; this one
    // INVENTS one — a binding it cannot see reads as "this topic has
    // no transport", so a legal `or wait` is REFUSED. A correct
    // program with its `bindings { }` block one brace deeper did not
    // compile.
    let mut bound_topics: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                for member in &l.members {
                    if let LocusMember::Bindings(bb) = member {
                        for e in &bb.entries {
                            bound_topics.insert(e.topic.name.clone());
                        }
                    }
                }
            }
        });
    }
    // GH #724: aliases of imports this bundle never resolved. Empty on
    // every CLI path (the merge strips `imports`); populated only for a
    // consumer of a library whose seed is not in the bundle.
    let mut unresolved_import_aliases: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    for program in bundle.programs.values() {
        for imp in &program.imports {
            if let Some(alias) = &imp.alias {
                unresolved_import_aliases.insert(alias.clone());
            }
        }
    }
    for program in bundle.programs.values() {
        let mut generic_fns: BTreeMap<String, &FnDecl> = BTreeMap::new();
        collect_generic_fns(&program.items, &mut generic_fns);
        let mut generic_types: BTreeMap<String, &TypeDecl> =
            BTreeMap::new();
        collect_generic_types(&program.items, &mut generic_types);
        let mut generic_loci: BTreeMap<String, &LocusDecl> = BTreeMap::new();
        collect_generic_loci(&program.items, &mut generic_loci);
        let mut cx = Checker {
            top,
            known: &known,
            diags: &mut diags,
            locals: ScopeStack::new(),
            current_locus: None,
            in_lifecycle: false,
            in_closure: false,
            in_on_failure: false,
            fallible_ctx: None,
            return_ctx: None,
            wasm_target,
            target_has_async_io: bundle.target_has_async_io,
            target_label: bundle.target_label,
            strict_callees,
            strict_idents,
            or_value_discarded: false,
            generic_params: Vec::new(),
            generic_fns,
            generic_types,
            generic_loci,
            bound_topics: &bound_topics,
            import_renames: &bundle.import_renames,
            unresolved_import_aliases: &unresolved_import_aliases,
        };
        for item in &program.items {
            cx.check_top_decl(item);
        }
    }
    // Bundle-level rules around topic bindings:
    //   - at most one `main` locus per bundle
    //   - bindings entries reference declared topics
    //   - duplicate bindings for the same topic are forbidden
    check_main_and_bindings(bundle, top, &mut diags);
    // GH #911 (B6): and the entry point is top-level only, which the
    // build path has always assumed and check did not say.
    check_entry_point_placement(bundle, &mut diags);
    // Phase 3 routing-keys (2026-05-25): bundle-level checks
    //   - `on_unmatched: fallback` topics must have at least one
    //     `where key == _` subscriber program-wide.
    //   - `where key == _` is only legal on fallback topics.
    check_phase3_fallback_subscribers(bundle, &mut diags);
    // GH #255 phase 2: bounded-topic pairing + subscriber-bound
    // placement rules.
    check_bounded_bus(bundle, &mut diags);
    // F.31 Phase 5: single-threaded-method invariant. Walks
    // method bodies looking for cross-pool `self.X.foo()` calls
    // where X's locus type is placed on a different pool than
    // self's. Cross-pool coordination must go through the bus,
    // not a direct method call. See spec/types.md
    // § "Single-threaded-method invariant (F.31)".
    check_placement_single_thread(bundle, top, &mut diags);
    // GH #826: a `pinned` placement entry gives its field an OS
    // thread whose join record is one alloca per instantiation SITE,
    // so instantiating the placing locus inside a loop orphans every
    // thread but the last and leaks its arena. Placement describes a
    // static topology; the loop is rejected.
    check_pinned_locus_in_loop(bundle, top, &mut diags);
    // GH #890: a placement entry is carried by the locus LITERAL
    // lowered for its field and by nothing else, so a field built any
    // other way (a factory call the commonest) leaves the entry
    // untaken and it is silently dropped. Every entry must be
    // consumed by exactly one instantiation.
    check_placement_entry_consumed(bundle, &mut diags);
    // Pool affinity (2026-08-12): `cooperative(pool = X, cores/…)`
    // entries naming one pool must agree, and affinity on the main
    // pool has no thread to bind.
    check_pool_affinity(bundle, &mut diags);
    // #334 / #333: F.31 above reasons per field DECLARATION, so it
    // cannot see one instance aliased into two towers.
    check_instance_aliasing(bundle, &mut diags);
    // F.31-followup (2026-05-28): the nested-long-running-child
    // antipattern. A non-main locus whose `run()` body has work
    // to do, holding a params field of a locus type whose own
    // `run()` doesn't return (or is on the known-long-running
    // stdlib list), gets a hard error pointing at the canonical
    // sibling-in-main + placement fix. See `spec/runtime.md §
    // Long-running cooperative children`.
    check_nested_long_running_child(bundle, &mut diags);
    // GH #813: a locus reachable from its own param defaults. The
    // by-value containment graph must be acyclic — a locus that
    // contains itself can never be built, and the compiler used to
    // discover that by overflowing its own stack in
    // `lower_locus_instantiation`. See `check_self_containing_locus`.
    check_self_containing_locus(bundle, &mut diags);
    check_cooperative_pool_blocking(bundle, &mut diags);
    // Perf lint (2026-07-16): hot-path allocation anti-patterns — a
    // locus instantiated per loop iteration, an allocating recv in a
    // loop. Warnings that steer toward the allocation-free shape
    // (hoisted field / `recv_into`).
    check_hot_path_alloc(bundle, top, &mut diags);
    // GH #723: the fn-level contract decorators stack, so a stack can
    // be incoherent — the same decorator twice, or `@unbounded` against
    // a contract that forbids allocation.
    check_decorator_stacks(bundle, &mut diags);
    // Gap D (2026-07-17): accept-without-release on a daemon-shaped
    // locus — resident children accumulate until OOM.
    check_accept_release(bundle, &mut diags);
    // Lever 2 (2026-07-16): `@budget(alloc_per_call = N)` — an opt-in
    // hot-path allocation contract. A hard error when an annotated fn
    // allocates more than its declared per-call ceiling (0 = zero-alloc
    // certificate). Reuses the `alloc_summary` call graph.
    {
        // Everything in this block is LAW — fn-grained certificates
        // (`@budget`, `@effects`, `@phase_effects`, the frontier
        // contracts) and bundle claims. #392 §8 already reports the
        // first as the claim form it is pointwise sugar for, and the
        // artifact records both in one vocabulary.
        //
        // They are marked `DiagKind::Claim` as a batch below, which
        // matters for one thing: a law failure means the program
        // typechecks and breaks a rule, so its model is sound and the
        // artifact must still be emitted to record the verdict. A
        // TYPE error means the model describes no program and the
        // artifact must not exist. Rendering is identical either way.
        let law_start = diags.len();
        let programs_vec: Vec<&Program> = bundle.programs.values().copied().collect();
        // GH #476 Change 5h: `@budget` is judged over the model
        // through the evidence sidecar — the counting engines
        // measure, `judge_certificates` decides. See
        // `check_bundle_opts`.
        // #265: categoric effect assertions (@no_recursion /
        // @no_ffi / @no_block) — same opt-in-contract discipline as
        // @budget, over the shared callgraph witness engine.
        diags.extend(crate::effects::effect_diags_with_renames(
            &programs_vec,
            &bundle.import_renames,
        ));
        {
            let graph = crate::bus_graph::build_bus_graph(bundle, top);
            // GH #265 frontier: cross-actor causality (needs the
            // bus graph), supervision coverage, and secret taint.
            // GH #476 Change 5f/5g: `causes:` and its backward dual
            // `depends:` (RFC #330) are judged over the model with
            // the other migrated families — see `check_bundle_opts`.
            // GH #382 phase 1: bundle-level claims — group
            // resolution (unknown name = error, vacuity) and
            // `forbid reaches` evaluation with countermodel
            // witnesses. Errors, gating check from day one: an
            // advisory claim reads as law and doesn't bind.
            // GH #476 Change 9: ONE authority per question. Law
            // SELECTION (which laws exist: constitutions, group
            // resolution, the tier rule) stays with the claim
            // surface; the VERDICTS come from the judgment engines
            // over the canonical model — the same judgment the
            // artifact projects, instead of a second evaluator
            // that re-derived the same four families from source.
            // `tests/claim_diags_differential.rs` held the two
            // byte-equal over the corpus through the cutover.
            diags.extend(crate::claims::selection_diags(
                &programs_vec,
                &graph,
                &bundle.import_renames,
            ));
            // The VERDICTS are appended by `check_bundle_opts`,
            // after this whole pass establishes that the program
            // denotes a valid model — see the note there. Selection
            // stays here: it reads the claim surface directly and is
            // meaningful even for a program that does not typecheck.
            diags.extend(crate::frontier::supervised_diags(&programs_vec));
            diags.extend(crate::frontier::secret_taint_diags(&programs_vec));
        }
        for d in &mut diags[law_start..] {
            if d.kind == hale_syntax::error::DiagKind::Type {
                d.kind = hale_syntax::error::DiagKind::Claim;
            }
        }
    }
    // 2026-05-29: a bus-subscribing locus instantiated non-owned
    // inside another locus's method/handler body dissolves at that
    // method's scope exit, so its subscription can never fire.
    // Hard error unless `--allow-unowned-subscriber` is set.
    check_unowned_subscriber_locus(bundle, allow_unowned_subscriber, &mut diags);
    // GH #18 #4: bus-graph property checks over the typed topic
    // topology. v1 (PR A): orphan topics — declared/used subjects
    // wired to only one end. Gated on a closed-world program (a
    // `main` locus present), so library seeds whose consumers are
    // external aren't falsely flagged.
    check_bus_graph(bundle, top, &mut diags);
    // GH #18 #4 (PR B): bus-graph cycles. A cross-locus publish→
    // subscribe→publish loop spins the cooperative queue (warning);
    // an intra-locus loop is devirtualized synchronous self-dispatch
    // that recurses without bound (error).
    check_bus_cycles(bundle, &mut diags);
    // GH #18 #4: backpressure. An unbounded publish loop with no
    // yield/throttle floods the bus — the producer has no
    // backpressure. Structural heuristic (warning).
    check_bus_backpressure(bundle, &mut diags);
    // GH #18 #4: subject type-mismatch. Two literal-subject sites
    // addressing the same wire subject must agree on the payload
    // type — otherwise a subscriber decodes the publisher's bytes as
    // the wrong type. Declared `topic`s are already unified by their
    // declaration; this closes the literal-subject gap.
    check_bus_subject_types(bundle, &mut diags);
    // GH #876: the declared payload must be a type the bus can
    // CARRY. The rule above relates two sites to each other; this one
    // relates one site to the runtime, which takes a user type, a
    // has-payload enum or `BytesView` and nothing else.
    check_bus_payload_carriable(bundle, top, &known, &mut diags);
    diags
}

/// True if the locus declares at least one `bus { subscribe ... }`.
fn locus_has_bus_subscribe(l: &LocusDecl) -> bool {
    l.members.iter().any(|m| match m {
        LocusMember::Bus(b) => b
            .members
            .iter()
            .any(|bm| matches!(bm, BusMember::Subscribe { .. })),
        _ => false,
    })
}

/// True if `parent` declares `accept(c: <child_name>)` — i.e. it
/// owns instantiations of that locus type as children (accept
/// fires by type, regardless of let-vs-statement binding).
fn locus_accepts(parent: &LocusDecl, child_name: &str) -> bool {
    parent.members.iter().any(|m| match m {
        LocusMember::Lifecycle(lc)
            if lc.kind == LifecycleKind::Accept =>
        {
            lc.params.first().is_some_and(|p| {
                matches!(&p.ty, TypeExpr::Named { path, .. }
                    if path.segments.last()
                        .is_some_and(|s| s.name == child_name))
            })
        }
        _ => false,
    })
}

/// Collect single-segment locus-instantiation sites
/// (`L { ... }`) reachable in a method body, as
/// (locus_name, span). Filtered to actual loci by the caller.
fn collect_locus_instantiations(
    block: &Block,
    out: &mut Vec<(String, Span)>,
) {
    for stmt in &block.stmts {
        collect_in_stmt(stmt, out);
    }
}

fn collect_in_stmt(stmt: &Stmt, out: &mut Vec<(String, Span)>) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            collect_in_expr(value, out)
        }
        Stmt::Assign { value, .. } => collect_in_expr(value, out),
        Stmt::Send { subject, value, .. } => {
            collect_in_expr(subject, out);
            collect_in_expr(value, out);
        }
        Stmt::If(i) => collect_in_if(i, out),
        Stmt::Match(m) => collect_in_match(m, out),
        Stmt::For { iter, body, .. } => {
            collect_in_expr(iter, out);
            collect_locus_instantiations(body, out);
        }
        Stmt::While { cond, body, .. } => {
            collect_in_expr(cond, out);
            collect_locus_instantiations(body, out);
        }
        Stmt::Return(Some(e), _) => collect_in_expr(e, out),
        Stmt::Fail { value, .. } => collect_in_expr(value, out),
        Stmt::Violate { payload: Some(e), .. } => collect_in_expr(e, out),
        Stmt::Recovery { args, .. } => {
            for a in args {
                collect_in_expr(a, out);
            }
        }
        Stmt::Block(b) => collect_locus_instantiations(b, out),
        Stmt::Expr(e) => collect_in_expr(e, out),
        _ => {}
    }
}

fn collect_in_if(stmt: &IfStmt, out: &mut Vec<(String, Span)>) {
    collect_in_expr(&stmt.cond, out);
    collect_locus_instantiations(&stmt.then_block, out);
    if let Some(eb) = &stmt.else_block {
        match eb.as_ref() {
            ElseBranch::Else(b) => collect_locus_instantiations(b, out),
            ElseBranch::ElseIf(nested) => collect_in_if(nested, out),
        }
    }
}

fn collect_in_match(stmt: &MatchStmt, out: &mut Vec<(String, Span)>) {
    collect_in_expr(&stmt.scrutinee, out);
    for arm in &stmt.arms {
        if let Some(g) = &arm.guard {
            collect_in_expr(g, out);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => collect_in_expr(e, out),
            MatchArmBody::Block(b) => collect_locus_instantiations(b, out),
        }
    }
}

fn collect_in_expr(expr: &Expr, out: &mut Vec<(String, Span)>) {
    match expr {
        Expr::Struct { path, inits, span, .. } => {
            if path.segments.len() == 1 {
                out.push((path.segments[0].name.clone(), *span));
            }
            for init in inits {
                collect_in_expr(&init.value, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_in_expr(left, out);
            collect_in_expr(right, out);
        }
        Expr::Unary { operand, .. } => collect_in_expr(operand, out),
        Expr::Call { callee, args, .. } => {
            collect_in_expr(callee, out);
            for a in args {
                collect_in_expr(a, out);
            }
        }
        Expr::Field { receiver, .. }
        | Expr::Path2 { receiver, .. } => collect_in_expr(receiver, out),
        Expr::Index { receiver, index, .. } => {
            collect_in_expr(receiver, out);
            collect_in_expr(index, out);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for e in es {
                collect_in_expr(e, out);
            }
        }
        Expr::Block(b) => collect_locus_instantiations(b, out),
        Expr::If(i) => collect_in_if(i, out),
        Expr::Match(m) => collect_in_match(m, out),
        Expr::Sum(e, _) | Expr::Prod(e, _) => collect_in_expr(e, out),
        Expr::Approx { left, right, tolerance, .. } => {
            collect_in_expr(left, out);
            collect_in_expr(right, out);
            collect_in_expr(tolerance, out);
        }
        Expr::Range { lo, hi, .. } => {
            collect_in_expr(lo, out);
            collect_in_expr(hi, out);
        }
        Expr::ArrayRepeat { val, .. } => collect_in_expr(val, out),
        Expr::Or { inner, disposition, .. } => {
            collect_in_expr(inner, out);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    collect_in_expr(e, out)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn check_unowned_subscriber_locus(
    bundle: &Bundle<'_>,
    allow: bool,
    diags: &mut Vec<Diag>,
) {
    if allow {
        return;
    }
    // GH #825: a module is a namespace, not an analysis boundary —
    // both the index and the walk flatten it.
    let mut local_loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                local_loci.insert(l.name.name.as_str(), l);
            }
        });
    }

    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(p) = item else {
                return;
            };
            // Collect this locus's bus-handler fn names. The
            // antipattern is narrow on purpose: a subscriber
            // spawned in a BUS HANDLER body is unambiguously
            // broken — a handler returns after each message, so
            // the spawned subscriber dissolves before it could
            // receive the next one. A subscriber spawned in
            // `run()` / `birth()` / a plain method is NOT flagged:
            // it lives for that scope and can legitimately receive
            // messages published during it (the canonical
            // `run()` spawns N watchers then publishes` pattern).
            let mut handler_names: std::collections::BTreeSet<&str> =
                std::collections::BTreeSet::new();
            for member in &p.members {
                if let LocusMember::Bus(b) = member {
                    for bm in &b.members {
                        if let BusMember::Subscribe { handler, .. } = bm {
                            handler_names.insert(handler.name.as_str());
                        }
                    }
                }
            }
            if handler_names.is_empty() {
                return;
            }
            for member in &p.members {
                let LocusMember::Fn(fd) = member else {
                    continue;
                };
                if !handler_names.contains(fd.name.name.as_str()) {
                    continue;
                }
                let mut hits: Vec<(String, Span)> = Vec::new();
                collect_locus_instantiations(&fd.body, &mut hits);
                for (name, span) in hits {
                    let Some(child) = local_loci.get(name.as_str()) else {
                        continue;
                    };
                    if !locus_has_bus_subscribe(child) {
                        continue;
                    }
                    if locus_accepts(p, &name) {
                        continue; // owned as a child — fine
                    }
                    diags.push(Diag::ty(
                        span,
                        format!(
                            "locus `{}` declares `bus subscribe` but is \
                             instantiated unowned inside `{}`'s bus handler \
                             `{}`. A bus handler returns after each message, \
                             so the locals it binds dissolve immediately — \
                             `{}`'s subscription would never fire for a later \
                             message.\n\n\
                             Own it so it shares the parent's lifetime \
                             instead of the handler's:\n\
                             - `accept(c: {})` on `{}` (child membership; the \
                             canonical N-dynamic-children shape), or\n\
                             - a capacity pool / params field of `{}`.\n\n\
                             If you manage its lifetime another way, pass \
                             `--allow-unowned-subscriber` to downgrade this \
                             to allowed.",
                            name,
                            p.name.name,
                            fd.name.name,
                            name,
                            name,
                            p.name.name,
                            p.name.name,
                        ),
                    ));
                }
            }
        });
    }
}

/// Known stdlib loci whose `run()` body is structurally non-
/// terminating (accept loops, daemon loops). Used by the
/// nested-long-running-child check; the typechecker can't see
/// stdlib bodies, so the list is maintained explicitly.
const KNOWN_LONG_RUNNING_STDLIB_LOCI: &[&[&str]] = &[
    &["std", "http", "Server"],
];

fn is_known_long_running_stdlib(path_segments: &[&str]) -> bool {
    KNOWN_LONG_RUNNING_STDLIB_LOCI
        .iter()
        .any(|known| *known == path_segments)
}

fn locus_has_nontrivial_run(l: &LocusDecl) -> bool {
    l.members.iter().any(|m| match m {
        LocusMember::Lifecycle(LifecycleDecl {
            kind: LifecycleKind::Run,
            body,
            ..
        }) => !body.stmts.is_empty(),
        _ => false,
    })
}

// === statically non-returning run() (pool starvation) =========
//
// A cooperative pool runs each posted `run()` cell to completion, so
// two loci on one pool whose `run()` bodies never return means the
// second never starts — silently (bus handlers still fire at
// sleep/yield drains, which makes the hang look like a healthy idle).
// The predicate below is deliberately conservative (same style as
// `while_counter_bounded` in alloc_summary.rs): it only claims
// "statically never returns" for shapes it can prove, so the
// starvation warning never false-fires on a loop that can exit.

/// Does this block contain a statement that can exit the enclosing
/// `run()` loop — `break`, `return`, `terminate`, `fail`, or
/// `violate`? Walked recursively through nested statement bodies but
/// NOT into expressions (a `return` inside a failure-closure exits
/// the closure, not `run()`). A `break` in a *nested* loop only exits
/// that loop, but counting it as an exit here is the conservative
/// direction (a missed warning, never a false one).
fn block_has_loop_exit(block: &Block) -> bool {
    block.stmts.iter().any(stmt_has_loop_exit)
}

fn stmt_has_loop_exit(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break(_)
        | Stmt::Return(..)
        | Stmt::Terminate(_)
        | Stmt::Fail { .. }
        | Stmt::Violate { .. } => true,
        Stmt::If(if_stmt) => if_has_loop_exit(if_stmt),
        Stmt::Match(m) => m.arms.iter().any(|arm| match &arm.body {
            MatchArmBody::Block(b) => block_has_loop_exit(b),
            MatchArmBody::Expr(_) => false,
        }),
        Stmt::For { body, .. } | Stmt::While { body, .. } => block_has_loop_exit(body),
        Stmt::Block(b) => block_has_loop_exit(b),
        Stmt::ShmWrite { body, .. } => block_has_loop_exit(body),
        _ => false,
    }
}

fn if_has_loop_exit(if_stmt: &IfStmt) -> bool {
    if block_has_loop_exit(&if_stmt.then_block) {
        return true;
    }
    match if_stmt.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => block_has_loop_exit(b),
        Some(ElseBranch::ElseIf(inner)) => if_has_loop_exit(inner),
        None => false,
    }
}

/// `Some(field_name)` iff the expression is a bare `self.<field>` read.
fn self_bool_field(e: &Expr) -> Option<&str> {
    match e {
        Expr::Field { receiver, name, .. }
            if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
        {
            Some(name.name.as_str())
        }
        _ => None,
    }
}

/// Is `self.<field> = ...` (or a compound assign to it) present in any
/// member body of the locus? Walked through nested statement bodies.
fn locus_assigns_self_field(decl: &LocusDecl, field: &str) -> bool {
    fn block_assigns(block: &Block, field: &str) -> bool {
        block.stmts.iter().any(|s| stmt_assigns(s, field))
    }
    fn if_assigns(if_stmt: &IfStmt, field: &str) -> bool {
        block_assigns(&if_stmt.then_block, field)
            || match if_stmt.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => block_assigns(b, field),
                Some(ElseBranch::ElseIf(inner)) => if_assigns(inner, field),
                None => false,
            }
    }
    fn stmt_assigns(stmt: &Stmt, field: &str) -> bool {
        match stmt {
            Stmt::Assign { target, .. } => {
                target.head.name == "self"
                    && matches!(
                        target.tail.first(),
                        Some(LValueSeg::Field(f)) if f.name == field
                    )
            }
            Stmt::If(if_stmt) => if_assigns(if_stmt, field),
            Stmt::Match(m) => m.arms.iter().any(|arm| match &arm.body {
                MatchArmBody::Block(b) => block_assigns(b, field),
                MatchArmBody::Expr(_) => false,
            }),
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                block_assigns(body, field)
            }
            Stmt::Block(b) => block_assigns(b, field),
            Stmt::ShmWrite { body, .. } => block_assigns(body, field),
            _ => false,
        }
    }
    decl.members.iter().any(|m| match m {
        LocusMember::Fn(f) => block_assigns(&f.body, field),
        LocusMember::Lifecycle(l) => block_assigns(&l.body, field),
        LocusMember::Mode(md) => block_assigns(&md.body, field),
        LocusMember::Failure(fd) => block_assigns(&fd.body, field),
        _ => false,
    })
}

/// The literal Bool default of a params field, if it has one.
fn param_bool_default(decl: &LocusDecl, field: &str) -> Option<bool> {
    decl.members.iter().find_map(|m| {
        let LocusMember::Params(pb) = m else { return None };
        pb.params.iter().find_map(|p| {
            if p.name.name != field {
                return None;
            }
            match &p.init {
                ParamInit::Value(Expr::Literal(Literal::Bool(b), _)) => Some(*b),
                _ => None,
            }
        })
    })
}

/// `Some(span of the terminal while)` iff this `run()` body statically
/// never returns: its last statement is a `while` whose body contains
/// no exit statement and whose condition provably never flips false —
///   - `while true`,
///   - `while !self.draining` (the synthetic drain flag flips only at
///     shutdown, so for the pool's purposes the loop runs forever),
///   - `while !self.f` / `while self.f` where `f` is a Bool params
///     field that no member body ever assigns and whose declared
///     default keeps the loop live (`false` / `true` respectively).
fn run_statically_nonreturning(run_body: &Block, decl: &LocusDecl) -> Option<Span> {
    let Some(Stmt::While { cond, body, span }) = run_body.stmts.last() else {
        return None;
    };
    if block_has_loop_exit(body) {
        return None;
    }
    let never_flips = match cond {
        Expr::Literal(Literal::Bool(true), _) => true,
        Expr::Unary { op: UnaryOp::Not, operand, .. } => {
            match self_bool_field(operand) {
                Some("draining") => true,
                Some(f) => {
                    !locus_assigns_self_field(decl, f)
                        && param_bool_default(decl, f) == Some(false)
                }
                None => false,
            }
        }
        _ => match self_bool_field(cond) {
            Some(f) if f != "draining" => {
                !locus_assigns_self_field(decl, f)
                    && param_bool_default(decl, f) == Some(true)
            }
            _ => false,
        },
    };
    if never_flips {
        Some(*span)
    } else {
        None
    }
}

/// A stdlib path call that blocks the calling OS thread until the
/// I/O completes. A cooperative (non-`async_io`) locus that runs one
/// in its `run()` loop holds the pool's thread for the call's whole
/// duration — stalling every other locus scheduled on that pool and
/// the pool's bus drain. (`async_io` parks instead of blocking;
/// `pinned` owns its own thread.) The warning path follows the call
/// graph interprocedurally — a `run()` that blocks through a helper
/// fn or a `self.method` is flagged (see `find_blocking_deep_in_block`
/// and the `blocking_*_fns` fixpoints). Still best-effort: blocking
/// via a method on a stdlib *handle* (`stream.recv(...)`) or across a
/// cross-locus `self.field.method()` hop isn't traced — this is a
/// warning, so the residual incompleteness is acceptable.
///
/// GH #830: the leaf set is the effects registry's `block`
/// classification (minus the leaves that yield a cooperative worker
/// while they wait), not a second hand list. The hand list this
/// replaces named 11 paths and so had no opinion at all about
/// `io::stdin::*`, `io::file::read_line`, `udp::recv*`,
/// `std::http::*`, `tcp::connect`/`accept_one`,
/// `tls::connect`/`upgrade` or `process::read_std*` — a `run()` that
/// blocked through one of those warned only if it *also* touched one
/// of the 11. See `stdlib_surface::holds_cooperative_worker`.
fn blocking_path_match(segs: &[&str]) -> Option<String> {
    crate::stdlib_surface::holds_cooperative_worker(segs)
        .then(|| segs.join("::"))
}

fn find_blocking_in_block(block: &Block) -> Option<(String, Span)> {
    block.stmts.iter().find_map(find_blocking_in_stmt)
}

fn find_blocking_in_stmt(stmt: &Stmt) -> Option<(String, Span)> {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            find_blocking_in_expr(value)
        }
        Stmt::Assign { value, .. } => find_blocking_in_expr(value),
        Stmt::Send { subject, value, .. } => {
            find_blocking_in_expr(subject).or_else(|| find_blocking_in_expr(value))
        }
        Stmt::Return(Some(e), _) => find_blocking_in_expr(e),
        Stmt::Fail { value, .. } => find_blocking_in_expr(value),
        Stmt::Violate { payload: Some(e), .. } => find_blocking_in_expr(e),
        Stmt::Recovery { args, .. } => args.iter().find_map(find_blocking_in_expr),
        Stmt::Expr(e) => find_blocking_in_expr(e),
        Stmt::If(i) => find_blocking_in_if(i),
        Stmt::Match(m) => find_blocking_in_match(m),
        Stmt::For { iter, body, .. } => {
            find_blocking_in_expr(iter).or_else(|| find_blocking_in_block(body))
        }
        Stmt::While { cond, body, .. } => {
            find_blocking_in_expr(cond).or_else(|| find_blocking_in_block(body))
        }
        Stmt::Block(b) => find_blocking_in_block(b),
        _ => None,
    }
}

fn find_blocking_in_if(i: &IfStmt) -> Option<(String, Span)> {
    find_blocking_in_expr(&i.cond)
        .or_else(|| find_blocking_in_block(&i.then_block))
        .or_else(|| match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => find_blocking_in_block(b),
            Some(ElseBranch::ElseIf(n)) => find_blocking_in_if(n),
            None => None,
        })
}

fn find_blocking_in_match(m: &MatchStmt) -> Option<(String, Span)> {
    find_blocking_in_expr(&m.scrutinee).or_else(|| {
        m.arms.iter().find_map(|arm| {
            arm.guard
                .as_ref()
                .and_then(find_blocking_in_expr)
                .or_else(|| match &arm.body {
                    MatchArmBody::Expr(e) => find_blocking_in_expr(e),
                    MatchArmBody::Block(b) => find_blocking_in_block(b),
                })
        })
    })
}

fn find_blocking_in_expr(expr: &Expr) -> Option<(String, Span)> {
    match expr {
        Expr::Call { callee, args, span, .. } => {
            if let Expr::Path(qn) = callee.as_ref() {
                let segs: Vec<&str> =
                    qn.segments.iter().map(|s| s.name.as_str()).collect();
                if let Some(name) = blocking_path_match(&segs) {
                    return Some((name, *span));
                }
            }
            find_blocking_in_expr(callee)
                .or_else(|| args.iter().find_map(find_blocking_in_expr))
        }
        Expr::Binary { left, right, .. } => {
            find_blocking_in_expr(left).or_else(|| find_blocking_in_expr(right))
        }
        Expr::Unary { operand, .. } => find_blocking_in_expr(operand),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            find_blocking_in_expr(receiver)
        }
        Expr::Index { receiver, index, .. } => {
            find_blocking_in_expr(receiver).or_else(|| find_blocking_in_expr(index))
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            es.iter().find_map(find_blocking_in_expr)
        }
        Expr::Struct { inits, .. } => {
            inits.iter().find_map(|i| find_blocking_in_expr(&i.value))
        }
        Expr::Block(b) => find_blocking_in_block(b),
        Expr::If(i) => find_blocking_in_if(i),
        Expr::Match(m) => find_blocking_in_match(m),
        Expr::Sum(e, _) | Expr::Prod(e, _) => find_blocking_in_expr(e),
        Expr::Or { inner, disposition, .. } => find_blocking_in_expr(inner)
            .or_else(|| match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    find_blocking_in_expr(e)
                }
                _ => None,
            }),
        _ => None,
    }
}

// === Interprocedural blocking detection (warning path only) =========
//
// The direct-call walk above only sees blocking ops written literally
// in `run()`. A `run()` that calls a helper fn — `self.drain()` or a
// free `pump(conn)` — that itself blocks holds the pool's thread just
// as surely, but escaped the syntactic walk. These helpers build a
// small call graph and propagate "blocks" from leaf stdlib ops up
// through callees, so the pool-stall **warning** also fires on
// blocking reached one or more fn-hops deep. (The dead-receiver ERROR
// deliberately stays direct-call-only — it over-fired once before, so
// we don't widen its call-graph surface; see
// `check_cooperative_pool_blocking`.)

/// Names a call expression's callee resolves to, split into free-fn
/// names (bare ident / single-segment path) and `self.method` names.
/// Used to build the call graph; over-collection is harmless (the
/// fixpoint only follows edges into fns it actually knows).
#[derive(Default)]
struct CalleeSet {
    free: BTreeSet<String>,
    self_methods: BTreeSet<String>,
}

fn collect_callees_in_block(b: &Block, out: &mut CalleeSet) {
    for s in &b.stmts {
        collect_callees_in_stmt(s, out);
    }
}

fn collect_callees_in_stmt(stmt: &Stmt, out: &mut CalleeSet) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            collect_callees_in_expr(value, out)
        }
        Stmt::Assign { value, .. } => collect_callees_in_expr(value, out),
        Stmt::Send { subject, value, .. } => {
            collect_callees_in_expr(subject, out);
            collect_callees_in_expr(value, out);
        }
        Stmt::Return(Some(e), _) => collect_callees_in_expr(e, out),
        Stmt::Fail { value, .. } => collect_callees_in_expr(value, out),
        Stmt::Violate { payload: Some(e), .. } => collect_callees_in_expr(e, out),
        Stmt::Recovery { args, .. } => {
            args.iter().for_each(|e| collect_callees_in_expr(e, out))
        }
        Stmt::Expr(e) => collect_callees_in_expr(e, out),
        Stmt::If(i) => collect_callees_in_if(i, out),
        Stmt::Match(m) => collect_callees_in_match(m, out),
        Stmt::For { iter, body, .. } => {
            collect_callees_in_expr(iter, out);
            collect_callees_in_block(body, out);
        }
        Stmt::While { cond, body, .. } => {
            collect_callees_in_expr(cond, out);
            collect_callees_in_block(body, out);
        }
        Stmt::Block(b) => collect_callees_in_block(b, out),
        _ => {}
    }
}

fn collect_callees_in_if(i: &IfStmt, out: &mut CalleeSet) {
    collect_callees_in_expr(&i.cond, out);
    collect_callees_in_block(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => collect_callees_in_block(b, out),
        Some(ElseBranch::ElseIf(n)) => collect_callees_in_if(n, out),
        None => {}
    }
}

fn collect_callees_in_match(m: &MatchStmt, out: &mut CalleeSet) {
    collect_callees_in_expr(&m.scrutinee, out);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            collect_callees_in_expr(g, out);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => collect_callees_in_expr(e, out),
            MatchArmBody::Block(b) => collect_callees_in_block(b, out),
        }
    }
}

/// Record the callee a `Call` resolves to (if a free fn or
/// `self.method`), then recurse into sub-expressions.
fn collect_callees_in_expr(expr: &Expr, out: &mut CalleeSet) {
    match expr {
        Expr::Call { callee, args, .. } => {
            match callee.as_ref() {
                Expr::Ident(id) => {
                    out.free.insert(id.name.clone());
                }
                Expr::Path(qn) if qn.segments.len() == 1 => {
                    out.free.insert(qn.segments[0].name.clone());
                }
                Expr::Field { receiver, name, .. }
                    if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
                {
                    out.self_methods.insert(name.name.clone());
                }
                _ => {}
            }
            collect_callees_in_expr(callee, out);
            args.iter().for_each(|a| collect_callees_in_expr(a, out));
        }
        Expr::Binary { left, right, .. } => {
            collect_callees_in_expr(left, out);
            collect_callees_in_expr(right, out);
        }
        Expr::Unary { operand, .. } => collect_callees_in_expr(operand, out),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            collect_callees_in_expr(receiver, out)
        }
        Expr::Index { receiver, index, .. } => {
            collect_callees_in_expr(receiver, out);
            collect_callees_in_expr(index, out);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            es.iter().for_each(|e| collect_callees_in_expr(e, out))
        }
        Expr::Struct { inits, .. } => {
            inits.iter().for_each(|i| collect_callees_in_expr(&i.value, out))
        }
        Expr::Block(b) => collect_callees_in_block(b, out),
        Expr::If(i) => collect_callees_in_if(i, out),
        Expr::Match(m) => collect_callees_in_match(m, out),
        Expr::Sum(e, _) | Expr::Prod(e, _) => collect_callees_in_expr(e, out),
        Expr::Or { inner, disposition, .. } => {
            collect_callees_in_expr(inner, out);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    collect_callees_in_expr(e, out)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// The set of free-fn names that block — directly (a leaf stdlib op in
/// the body) or transitively (they call a blocking free fn).
/// Fixpoint over the free-fn call graph; free fns can't reference
/// `self`, so they only depend on other free fns.
fn blocking_free_fns(free_fns: &BTreeMap<String, &Block>) -> BTreeSet<String> {
    let mut blocking: BTreeSet<String> = free_fns
        .iter()
        .filter(|(_, body)| find_blocking_in_block(body).is_some())
        .map(|(n, _)| n.clone())
        .collect();
    let mut callees: BTreeMap<&str, CalleeSet> = BTreeMap::new();
    for (n, body) in free_fns {
        let mut cs = CalleeSet::default();
        collect_callees_in_block(body, &mut cs);
        callees.insert(n.as_str(), cs);
    }
    loop {
        let mut changed = false;
        for (n, cs) in &callees {
            if blocking.contains(*n) {
                continue;
            }
            if cs.free.iter().any(|c| blocking.contains(c)) {
                blocking.insert((*n).to_string());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    blocking
}

/// The set of a locus's own method names that block — directly, via a
/// blocking free fn, or via another blocking method on the same locus.
/// Seeded by `blocking_free`; fixpoint over the intra-locus method
/// call graph.
fn blocking_self_methods(
    methods: &BTreeMap<String, &Block>,
    blocking_free: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut callees: BTreeMap<&str, CalleeSet> = BTreeMap::new();
    let mut blocking: BTreeSet<String> = BTreeSet::new();
    for (n, body) in methods {
        let mut cs = CalleeSet::default();
        collect_callees_in_block(body, &mut cs);
        if find_blocking_in_block(body).is_some()
            || cs.free.iter().any(|c| blocking_free.contains(c))
        {
            blocking.insert(n.clone());
        }
        callees.insert(n.as_str(), cs);
    }
    loop {
        let mut changed = false;
        for (n, cs) in &callees {
            if blocking.contains(*n) {
                continue;
            }
            if cs.self_methods.iter().any(|c| blocking.contains(c)) {
                blocking.insert((*n).to_string());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    blocking
}

/// Interprocedural form of `find_blocking_in_block`: reports the first
/// blocking reach in `run()` — a direct leaf stdlib op, OR a call to a
/// blocking free fn / `self.method`. The returned span is the
/// run()-level call site; the name describes what blocks. Used for the
/// pool-stall warning only.
fn find_blocking_deep_in_block(
    b: &Block,
    blocking_free: &BTreeSet<String>,
    blocking_self: &BTreeSet<String>,
) -> Option<(String, Span)> {
    b.stmts
        .iter()
        .find_map(|s| find_blocking_deep_in_stmt(s, blocking_free, blocking_self))
}

fn find_blocking_deep_in_stmt(
    stmt: &Stmt,
    bf: &BTreeSet<String>,
    bs: &BTreeSet<String>,
) -> Option<(String, Span)> {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            find_blocking_deep_in_expr(value, bf, bs)
        }
        Stmt::Assign { value, .. } => find_blocking_deep_in_expr(value, bf, bs),
        Stmt::Send { subject, value, .. } => {
            find_blocking_deep_in_expr(subject, bf, bs)
                .or_else(|| find_blocking_deep_in_expr(value, bf, bs))
        }
        Stmt::Return(Some(e), _) => find_blocking_deep_in_expr(e, bf, bs),
        Stmt::Fail { value, .. } => find_blocking_deep_in_expr(value, bf, bs),
        Stmt::Violate { payload: Some(e), .. } => {
            find_blocking_deep_in_expr(e, bf, bs)
        }
        Stmt::Recovery { args, .. } => {
            args.iter().find_map(|e| find_blocking_deep_in_expr(e, bf, bs))
        }
        Stmt::Expr(e) => find_blocking_deep_in_expr(e, bf, bs),
        Stmt::If(i) => find_blocking_deep_in_if(i, bf, bs),
        Stmt::Match(m) => find_blocking_deep_in_match(m, bf, bs),
        Stmt::For { iter, body, .. } => find_blocking_deep_in_expr(iter, bf, bs)
            .or_else(|| find_blocking_deep_in_block(body, bf, bs)),
        Stmt::While { cond, body, .. } => find_blocking_deep_in_expr(cond, bf, bs)
            .or_else(|| find_blocking_deep_in_block(body, bf, bs)),
        Stmt::Block(b) => find_blocking_deep_in_block(b, bf, bs),
        _ => None,
    }
}

fn find_blocking_deep_in_if(
    i: &IfStmt,
    bf: &BTreeSet<String>,
    bs: &BTreeSet<String>,
) -> Option<(String, Span)> {
    find_blocking_deep_in_expr(&i.cond, bf, bs)
        .or_else(|| find_blocking_deep_in_block(&i.then_block, bf, bs))
        .or_else(|| match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => find_blocking_deep_in_block(b, bf, bs),
            Some(ElseBranch::ElseIf(n)) => find_blocking_deep_in_if(n, bf, bs),
            None => None,
        })
}

fn find_blocking_deep_in_match(
    m: &MatchStmt,
    bf: &BTreeSet<String>,
    bs: &BTreeSet<String>,
) -> Option<(String, Span)> {
    find_blocking_deep_in_expr(&m.scrutinee, bf, bs).or_else(|| {
        m.arms.iter().find_map(|arm| {
            arm.guard
                .as_ref()
                .and_then(|g| find_blocking_deep_in_expr(g, bf, bs))
                .or_else(|| match &arm.body {
                    MatchArmBody::Expr(e) => find_blocking_deep_in_expr(e, bf, bs),
                    MatchArmBody::Block(b) => find_blocking_deep_in_block(b, bf, bs),
                })
        })
    })
}

fn find_blocking_deep_in_expr(
    expr: &Expr,
    bf: &BTreeSet<String>,
    bs: &BTreeSet<String>,
) -> Option<(String, Span)> {
    match expr {
        Expr::Call { callee, args, span, .. } => {
            match callee.as_ref() {
                Expr::Path(qn) => {
                    let segs: Vec<&str> =
                        qn.segments.iter().map(|s| s.name.as_str()).collect();
                    if let Some(name) = blocking_path_match(&segs) {
                        return Some((name, *span));
                    }
                    if segs.len() == 1 && bf.contains(segs[0]) {
                        return Some((
                            format!("{}() (which makes a blocking call)", segs[0]),
                            *span,
                        ));
                    }
                }
                Expr::Ident(id) if bf.contains(&id.name) => {
                    return Some((
                        format!("{}() (which makes a blocking call)", id.name),
                        *span,
                    ));
                }
                Expr::Field { receiver, name, .. }
                    if matches!(receiver.as_ref(), Expr::KwSelf(_))
                        && bs.contains(&name.name) =>
                {
                    return Some((
                        format!("self.{}() (which makes a blocking call)", name.name),
                        *span,
                    ));
                }
                _ => {}
            }
            find_blocking_deep_in_expr(callee, bf, bs)
                .or_else(|| args.iter().find_map(|a| find_blocking_deep_in_expr(a, bf, bs)))
        }
        Expr::Binary { left, right, .. } => find_blocking_deep_in_expr(left, bf, bs)
            .or_else(|| find_blocking_deep_in_expr(right, bf, bs)),
        Expr::Unary { operand, .. } => find_blocking_deep_in_expr(operand, bf, bs),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            find_blocking_deep_in_expr(receiver, bf, bs)
        }
        Expr::Index { receiver, index, .. } => {
            find_blocking_deep_in_expr(receiver, bf, bs)
                .or_else(|| find_blocking_deep_in_expr(index, bf, bs))
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            es.iter().find_map(|e| find_blocking_deep_in_expr(e, bf, bs))
        }
        Expr::Struct { inits, .. } => {
            inits.iter().find_map(|i| find_blocking_deep_in_expr(&i.value, bf, bs))
        }
        Expr::Block(b) => find_blocking_deep_in_block(b, bf, bs),
        Expr::If(i) => find_blocking_deep_in_if(i, bf, bs),
        Expr::Match(m) => find_blocking_deep_in_match(m, bf, bs),
        Expr::Sum(e, _) | Expr::Prod(e, _) => find_blocking_deep_in_expr(e, bf, bs),
        Expr::Or { inner, disposition, .. } => {
            find_blocking_deep_in_expr(inner, bf, bs).or_else(|| match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    find_blocking_deep_in_expr(e, bf, bs)
                }
                _ => None,
            })
        }
        _ => None,
    }
}

/// Warn when a locus placed `cooperative(pool = X)` without
/// `where async_io` calls a known-blocking stdlib op in its `run()`.
/// Such a call holds the pool's OS thread, starving co-scheduled loci
/// (this silently bricked a downstream team's metrics server when a
/// blocking gateway was moved onto a shared pool). A warning, not an
/// error — a single-purpose blocking server with nothing co-scheduled
/// is legitimate; the smell is real but situational.
/// A comparable key for a bus subject (topic name / literal /
/// qualified path) — used to tell whether a subscription is to a
/// topic the locus also publishes.
fn bus_subject_key(s: &BusSubject) -> String {
    match s {
        BusSubject::Literal { subject, .. } => subject.clone(),
        BusSubject::Topic(id) => id.name.clone(),
        BusSubject::QualifiedTopic(qn) => qn
            .segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("::"),
    }
}

/// Handler names for the locus's `subscribe` entries on topics it
/// does NOT itself publish — i.e. genuine cross-context receives. A
/// self-publish→self-subscribe is devirtualized to a direct
/// `self.handler(...)` call (same instance, same thread), not a bus
/// receive, so it's excluded.
fn external_subscription_handlers(decl: &LocusDecl) -> Vec<String> {
    let mut published: BTreeSet<String> = BTreeSet::new();
    let mut handlers: Vec<(String, String)> = Vec::new(); // (subject_key, handler)
    for m in &decl.members {
        let LocusMember::Bus(b) = m else { continue };
        for bm in &b.members {
            match bm {
                BusMember::Publish { subject, .. } => {
                    published.insert(bus_subject_key(subject));
                }
                BusMember::Subscribe { subject, handler, .. } => {
                    handlers.push((bus_subject_key(subject), handler.name.clone()));
                }
            }
        }
    }
    handlers
        .into_iter()
        .filter(|(subj, _)| !published.contains(subj))
        .map(|(_, h)| h)
        .collect()
}

// ===================================================================
// Perf lint (downstream handoff 2026-07-16): hot-path allocation
// anti-patterns. Steer the naive shape toward the allocation-free one
// so the fast path is the path of least resistance rather than expert
// folklore. Two loop-scoped patterns, both warnings:
//   1. a locus (its own arena / heap buffer) instantiated per loop
//      iteration — hoist to a reused field;
//   2. an allocating `recv` in a loop — use `recv_into` with a reused
//      buffer.
// Both accumulate in the method scratch until the enclosing method
// returns, and a `run()` read loop never returns, so under load they
// dominate the p50 tax and (per the recv_bytes-in-a-loop footgun) can
// grow unboundedly. Loop-scoped keeps the signal clean (per-iteration
// is the unambiguous case); a handler-scratch instantiation reclaims
// per invocation and isn't flagged.
// ===================================================================

struct HotPathCx<'a> {
    top: &'a TopScope,
    diags: &'a mut Vec<Diag>,
    loop_depth: u32,
    /// Gap D (2026-07-17): walking a BUS HANDLER body. A handler runs
    /// per message, so per-call allocation findings fire at any
    /// nesting depth, not just inside loops (the ~4.5 KB/frame
    /// locus-in-handler class).
    in_handler: bool,
    /// Gap D: inside an `@hot fn`. Findings become hard errors and
    /// the stricter perf hints (`snapshot()`/`finish()` in a loop,
    /// whole-struct self-field replace) activate.
    hot: bool,
    /// GH #526 (2026-09-05): inside an `@unbounded` fn or lifecycle
    /// hook. Every advisory this lint emits ends with "or acknowledge
    /// an intentional shape with `@unbounded` on the enclosing
    /// fn/hook" — and the walker never read the flag, so the
    /// acknowledgement the message promised did nothing and `hale
    /// verify` stayed red on a param-bounded fan-out loop. The flag
    /// silences the ADVISORY only; `@hot` still hard-errors (a hot
    /// fn that allocates unboundedly is a contradiction, not an
    /// acknowledgement).
    unbounded: bool,
}

impl HotPathCx<'_> {
    /// Advisory warn by default; hard error inside `@hot`; silent
    /// inside `@unbounded` (unless also `@hot`).
    fn emit(&mut self, span: Span, msg: String) {
        if self.hot {
            self.diags.push(Diag::ty(
                span,
                format!("@hot: {}", msg),
            ));
        } else if !self.unbounded {
            self.diags.push(Diag::warn(span, msg));
        }
    }
}

/// A locus literal worth hoisting out of a loop: a user locus (carries
/// its own arena) or a heap-bearing stdlib builder. Plain struct/type
/// literals are values and don't allocate, so they're not flagged.
fn hot_locus_name(path: &QualifiedName, top: &TopScope) -> Option<String> {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    if segs == ["std", "bytes", "BytesBuilder"] {
        return Some("std::bytes::BytesBuilder".to_string());
    }
    if segs.len() == 1 && matches!(top.lookup(segs[0]), Some(TopSymbol::Locus(_))) {
        return Some(segs[0].to_string());
    }
    None
}

/// GH #402: does `callee` name a free fn whose return type is a locus?
/// Returns `(display spelling, locus name)`.
///
/// Factories are how loci get built once construction is factored out
/// — m90 forbids a method returning a locus, so a free fn is the only
/// shape available — which is precisely why the literal-only lint had
/// a hole here.
fn hot_factory_locus(
    callee: &Expr,
    top: &TopScope,
) -> Option<(String, String)> {
    let (disp, sym) = match callee {
        Expr::Ident(id) => (id.name.clone(), top.lookup(&id.name)?),
        Expr::Path(qn) => {
            let segs: Vec<&str> =
                qn.segments.iter().map(|s| s.name.as_str()).collect();
            let disp = segs.join("::");
            // An imported seed's fn is merged under a MANGLED name
            // (`__lib_<id>_<stem>_<fn>`) while the call site keeps
            // its author spelling (`lb::make`), so neither the
            // joined form nor the bare tail resolves. Fall back to
            // scanning for the mangled spelling.
            //
            // Two seeds can export the same tail name, and this
            // layer has no alias table to tell them apart (the same
            // limitation `topic_tail` documents). A lint must not
            // invent a finding from an ambiguity, so a tail that
            // matches candidates DISAGREEING about whether they
            // return a locus is dropped.
            let tail = *segs.last()?;
            let sym = top.lookup(&disp).or_else(|| top.lookup(tail));
            match sym {
                Some(sym) => (disp, sym),
                None => {
                    let mut hit: Option<&TopSymbol> = None;
                    let (mut factories, mut others) = (0usize, 0usize);
                    for (k, v) in &top.symbols {
                        let matches_tail = k
                            .strip_prefix("__lib_")
                            .is_some_and(|r| {
                                r.rsplit('_').next() == Some(tail)
                            });
                        if !matches_tail {
                            continue;
                        }
                        let is_factory = matches!(v, TopSymbol::Fn(sig)
                            if matches!(&sig.ret, Ty::Named(n)
                                if matches!(
                                    top.lookup(n),
                                    Some(TopSymbol::Locus(_))
                                )));
                        if is_factory {
                            hit = Some(v);
                            factories += 1;
                        } else {
                            others += 1;
                        }
                    }
                    // Ambiguous either way: two seeds exporting a
                    // locus factory under this tail, or one that does
                    // and one that doesn't. Counting BOTH kinds
                    // rather than short-circuiting keeps the verdict
                    // independent of `symbols` iteration order — a
                    // non-factory seen BEFORE the factory has to
                    // poison the tail exactly as one seen after it
                    // does.
                    if factories != 1 || others > 0 {
                        return None;
                    }
                    (disp, hit?)
                }
            }
        }
        _ => return None,
    };
    let TopSymbol::Fn(sig) = sym else { return None };
    let Ty::Named(ret) = &sig.ret else { return None };
    match top.lookup(ret) {
        Some(TopSymbol::Locus(_)) => Some((disp, ret.clone())),
        _ => None,
    }
}

/// An allocating recv path-call (the result Bytes/String lands in the
/// caller's scratch). `recv_into` is the zero-alloc alternative.
fn allocating_recv_name(callee: &Expr) -> Option<String> {
    match callee {
        // Path-call form: `std::io::udp::recv(fd, n)`.
        Expr::Path(qn) => {
            let segs: Vec<&str> =
                qn.segments.iter().map(|s| s.name.as_str()).collect();
            match segs.as_slice() {
                ["std", "io", "tcp", "recv"]
                | ["std", "io", "tcp", "recv_bytes"]
                | ["std", "io", "udp", "recv"]
                | ["std", "io", "udp", "recv_with_source"]
                | ["std", "io", "tls", "recv_bytes"] => Some(segs.join("::")),
                _ => None,
            }
        }
        // Method-call form: `stream.recv_bytes(n)`. The Stream receiver
        // types as Unknown in the checker (stdlib handle locus), so key
        // off the method name. `recv_bytes` / `recv_with_source` are
        // stdlib-specific enough that false positives are rare; plain
        // `recv` (a common user-method name) is only flagged in the
        // path-call form above.
        Expr::Field { name, .. } => match name.name.as_str() {
            "recv_bytes" | "recv_with_source" => Some(name.name.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn hot_walk_block(b: &Block, cx: &mut HotPathCx) {
    for s in &b.stmts {
        hot_walk_stmt(s, cx);
    }
    if let Some(t) = &b.tail {
        hot_walk_expr(t, cx);
    }
}

fn hot_walk_if(i: &IfStmt, cx: &mut HotPathCx) {
    hot_walk_expr(&i.cond, cx);
    hot_walk_block(&i.then_block, cx);
    if let Some(eb) = &i.else_block {
        match eb.as_ref() {
            ElseBranch::Else(b) => hot_walk_block(b, cx),
            ElseBranch::ElseIf(i2) => hot_walk_if(i2, cx),
        }
    }
}

fn hot_walk_match(m: &MatchStmt, cx: &mut HotPathCx) {
    hot_walk_expr(&m.scrutinee, cx);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            hot_walk_expr(g, cx);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => hot_walk_expr(e, cx),
            MatchArmBody::Block(b) => hot_walk_block(b, cx),
        }
    }
}

fn hot_walk_stmt(s: &Stmt, cx: &mut HotPathCx) {
    match s {
        Stmt::While { cond, body, .. } => {
            hot_walk_expr(cond, cx);
            cx.loop_depth += 1;
            hot_walk_block(body, cx);
            cx.loop_depth -= 1;
        }
        Stmt::For { iter, body, .. } => {
            hot_walk_expr(iter, cx);
            cx.loop_depth += 1;
            hot_walk_block(body, cx);
            cx.loop_depth -= 1;
        }
        Stmt::Let { value, span, .. } | Stmt::LetTuple { value, span, .. } => {
            hot_walk_expr(value, cx);
            // GH #402: a locus does not have to be spelled as a
            // LITERAL to be allocated here. `let m = mat::zeros(r, c)`
            // in a loop body allocates a fresh Matrix — its own arena
            // — every iteration and, being let-bound, is reclaimed
            // only when the enclosing fn returns, exactly like the
            // literal the advisory above already covers. The lint saw
            // only `Expr::Struct`, so a codebase that factors its
            // construction behind factory functions (the idiomatic
            // shape, and the one m90 pushes you toward since a method
            // cannot return a locus) got no warning at all while
            // leaking linearly.
            //
            // Only the LET form warns. An unbound temporary in
            // statement position is registered and reclaimed at the
            // statement since #403, so it is the recommended fix
            // rather than a finding.
            if cx.loop_depth > 0 || cx.in_handler {
                if let Expr::Call { callee, .. } = value {
                    if let Some((fn_disp, locus)) =
                        hot_factory_locus(callee, cx.top)
                    {
                        let where_ = if cx.loop_depth > 0 {
                            "every iteration"
                        } else {
                            "every message"
                        };
                        cx.emit(
                            *span,
                            format!(
                                "hot-path allocation: `{}` returns the locus `{}`, so a \
                                 fresh instance (its own arena / heap buffer) is \
                                 allocated {} and, being let-bound, is only reclaimed \
                                 when the enclosing fn returns. Hoist the result to a \
                                 reused field and refill it, drop the binding if the \
                                 value is only passed on (an unbound factory result is \
                                 reclaimed at the statement), or acknowledge an \
                                 intentional shape with `@unbounded` on the enclosing \
                                 fn/hook.",
                                fn_disp, locus, where_
                            ),
                        );
                    }
                }
            }
        }
        Stmt::Assign { target, value, span, .. } => {
            hot_walk_expr(value, cx);
            // Gap D (@hot only): whole-struct replace of a direct
            // self-field on a certified-hot path. Since Gap A the
            // replaced String clones RETIRE (this is no longer a
            // leak in methods/handlers) — but each store still pays
            // an anchor-clone + retire per heap field, where
            // in-place scalar mutation is allocation-free. Perf
            // hint, so @hot-gated.
            if cx.hot
                && (cx.loop_depth > 0 || cx.in_handler)
                && target.head.name == "self"
                && target.tail.len() == 1
                && matches!(target.tail[0], LValueSeg::Field(_))
                && matches!(value, Expr::Struct { .. })
            {
                cx.emit(
                    *span,
                    "hot-path store: whole-struct replace of a \
                     self-field — every store re-anchors the struct's \
                     heap fields (clone + retire per String field). \
                     On a certified-hot path prefer mutating the \
                     changed scalar fields in place \
                     (`self.field.x = v`), or keep the replace if \
                     most fields genuinely change."
                        .to_string(),
                );
            }
        }
        Stmt::If(i) => hot_walk_if(i, cx),
        Stmt::Match(m) => hot_walk_match(m, cx),
        Stmt::Return(Some(e), _) => hot_walk_expr(e, cx),
        Stmt::Fail { value, .. } => hot_walk_expr(value, cx),
        Stmt::Expr(e) => {
            // iris handoff P2.2: a BARE-STATEMENT instantiation of
            // a subscription-less locus dissolves EAGERLY at the
            // statement — its arena is destroyed every iteration,
            // which is the documented per-iteration-child idiom
            // (and the very fix this advisory's "hoist it"
            // guidance competes with). Only let-bound and
            // subscription-bearing instantiations defer to
            // method exit, so only those keep the warning. The
            // field inits still walk (a nested builder inside
            // the child literal is still a finding).
            if let Expr::Struct { path, inits, .. } = e {
                let eager = hot_locus_name(path, cx.top)
                    .map(|n| match cx.top.lookup(&n) {
                        Some(TopSymbol::Locus(li)) => {
                            li.bus_subscribes.is_empty()
                        }
                        _ => false,
                    })
                    .unwrap_or(false);
                if eager {
                    for init in inits {
                        hot_walk_expr(&init.value, cx);
                    }
                    return;
                }
            }
            hot_walk_expr(e, cx);
        }
        _ => {}
    }
}

fn hot_walk_expr(e: &Expr, cx: &mut HotPathCx) {
    match e {
        Expr::Struct { path, inits, span, .. } => {
            for init in inits {
                hot_walk_expr(&init.value, cx);
            }
            // Gap D: a bus handler runs per message — a locus/builder
            // instantiated ANYWHERE in it is the ~4.5 KB/frame class
            // (a fresh arena per frame, reclaimed only at handler
            // return... and its chunk only at locus dissolve), so the
            // handler context fires at depth 0 too.
            if cx.loop_depth > 0 || cx.in_handler {
                if let Some(name) = hot_locus_name(path, cx.top) {
                    // GH #815 retired half of what this advisory used
                    // to say: a locus created in a LOOP is now
                    // reclaimed when the next iteration reaches the
                    // same instantiation, so residency no longer grows
                    // without bound and a `run()` read loop that never
                    // returns is no longer the worst case. The
                    // allocation itself is still per-iteration, which
                    // is what the advisory is for, so say THAT — the
                    // arena create/destroy pair on the hot path —
                    // rather than a reclaim rule that no longer holds.
                    // The handler-at-depth-0 half is unchanged: one
                    // instantiation per message, reclaimed at the
                    // handler's return.
                    let message = if cx.loop_depth > 0 {
                        format!(
                            "hot-path allocation: locus `{}` is instantiated \
                             inside a loop — a fresh instance (its own arena \
                             / heap buffer) is allocated every iteration, and \
                             reclaimed only when the next iteration replaces \
                             it, so an arena create/destroy pair and the \
                             instance's whole lifecycle are on the hot path. \
                             Hoist it to a reused field, `clear()` and refill \
                             one builder, or acknowledge an intentional shape \
                             with `@unbounded` on the enclosing fn/hook.",
                            name
                        )
                    } else {
                        format!(
                            "hot-path allocation: locus `{}` is instantiated \
                             inside a bus handler — a fresh instance (its own \
                             arena / heap buffer) is allocated every message \
                             and, being let-bound or subscription-bearing, is \
                             only reclaimed when the enclosing method \
                             returns. Hoist it to a reused field, `clear()` \
                             and refill one builder, use a bare-statement \
                             per-message child (eagerly dissolved), or \
                             acknowledge an intentional shape with \
                             `@unbounded` on the enclosing fn/hook.",
                            name
                        )
                    };
                    cx.emit(*span, message);
                }
            }
        }
        Expr::Call { callee, args, span, .. } => {
            hot_walk_expr(callee, cx);
            for a in args {
                hot_walk_expr(a, cx);
            }
            // Loop-only (NOT handler-at-depth-0): a single recv per
            // handler call reclaims at the handler's scratch destroy;
            // only the in-loop shape accumulates within one
            // activation.
            if cx.loop_depth > 0 {
                if let Some(disp) = allocating_recv_name(callee) {
                    cx.emit(
                        *span,
                        format!(
                            "hot-path allocation: `{}` in a loop allocates a \
                             fresh result buffer each iteration (it accumulates \
                             in the method scratch until the method returns). \
                             Use `recv_into(fd, buf, max)` with a reused \
                             `std::bytes::BytesBuilder` for a zero-alloc hot \
                             path.",
                            disp
                        ),
                    );
                }
            }
            // Gap D (@hot only): `snapshot()` / `finish()` in a loop
            // or handler materializes a fresh String/Bytes copy of
            // the builder's contents per call — `.view()` /
            // `.text_view()` reads the same bytes zero-copy.
            if cx.hot && (cx.loop_depth > 0 || cx.in_handler) {
                if let Expr::Field { name, .. } = callee.as_ref() {
                    if name.name == "snapshot" || name.name == "finish" {
                        cx.emit(
                            *span,
                            format!(
                                "hot-path allocation: `.{}()` copies the \
                                 builder's full contents into a fresh \
                                 buffer each call. On a certified-hot path \
                                 prefer `.view()` / `.text_view()` \
                                 (zero-copy; valid until the next \
                                 overwrite).",
                                name.name
                            ),
                        );
                    }
                }
            }
        }
        Expr::Binary { left, right, .. } => {
            hot_walk_expr(left, cx);
            hot_walk_expr(right, cx);
        }
        Expr::Unary { operand, .. } => hot_walk_expr(operand, cx),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            hot_walk_expr(receiver, cx)
        }
        Expr::Index { receiver, index, .. } => {
            hot_walk_expr(receiver, cx);
            hot_walk_expr(index, cx);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for e in es {
                hot_walk_expr(e, cx);
            }
        }
        Expr::Sum(e, _) | Expr::Prod(e, _) => hot_walk_expr(e, cx),
        Expr::Approx { left, right, tolerance, .. } => {
            hot_walk_expr(left, cx);
            hot_walk_expr(right, cx);
            hot_walk_expr(tolerance, cx);
        }
        Expr::Range { lo, hi, .. } => {
            hot_walk_expr(lo, cx);
            hot_walk_expr(hi, cx);
        }
        Expr::ArrayRepeat { val, .. } => hot_walk_expr(val, cx),
        Expr::Block(b) => hot_walk_block(b, cx),
        Expr::If(i) => hot_walk_if(i, cx),
        Expr::Match(m) => hot_walk_match(m, cx),
        Expr::Or { inner, disposition, .. } => {
            hot_walk_expr(inner, cx);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    hot_walk_expr(e, cx)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Every declaration in `items`, with `module { … }` nesting
/// flattened: the module itself is yielded, then each of its items,
/// recursively.
///
/// GH #764: a module is a NAMESPACE, not an analysis boundary. The
/// resolver's `register_top_decls` recurses through modules and keys
/// `TopScope` by the BARE name, so a fn or locus inside one is an
/// ordinary member of the bundle everywhere except in a check that
/// walks `program.items` and stops. A declaration-shaped check that
/// does that silently sees half the program.
///
/// The `'a` on the yielded reference is load-bearing (GH #825): most
/// of the bundle-level checks build a name → `&LocusDecl` index in
/// one pass and consult it in the next, so the borrow handed to the
/// visitor has to outlive the walk. Without the named lifetime the
/// closure is higher-ranked over it and nothing it sees can be
/// stored.
fn walk_decls<'a>(items: &'a [TopDecl], f: &mut impl FnMut(&'a TopDecl)) {
    // GH #884: the walk itself moved to `hale_syntax::ast` when
    // codegen needed the same one. Same order, same yield of the
    // module node before its contents.
    for item in hale_syntax::ast::flat_decls(items) {
        f(item);
    }
}

fn check_hot_path_alloc(bundle: &Bundle<'_>, top: &TopScope, diags: &mut Vec<Diag>) {
    fn check_decl(item: &TopDecl, top: &TopScope, diags: &mut Vec<Diag>) {
        match item {
            TopDecl::Locus(l) => {
                // Gap D: fn members bound as bus handlers get the
                // per-message context (findings fire at depth 0).
                let handler_names: BTreeSet<&str> = l
                    .members
                    .iter()
                    .filter_map(|m| match m {
                        LocusMember::Bus(bb) => Some(bb.members.iter()),
                        _ => None,
                    })
                    .flatten()
                    .filter_map(|bm| match bm {
                        BusMember::Subscribe { handler, .. } => {
                            Some(handler.name.as_str())
                        }
                        _ => None,
                    })
                    .collect();
                for m in &l.members {
                    let (body, in_handler, hot, unbounded) = match m {
                        LocusMember::Fn(fd) => (
                            Some(&fd.body),
                            handler_names
                                .contains(fd.name.name.as_str()),
                            fd.hot,
                            fd.unbounded,
                        ),
                        LocusMember::Lifecycle(ld) => {
                            (Some(&ld.body), false, false, ld.unbounded)
                        }
                        _ => (None, false, false, false),
                    };
                    if let Some(b) = body {
                        let mut cx = HotPathCx {
                            top,
                            diags: &mut *diags,
                            loop_depth: 0,
                            in_handler,
                            hot,
                            unbounded,
                        };
                        hot_walk_block(b, &mut cx);
                    }
                }
            }
            TopDecl::Fn(fd) => {
                let mut cx = HotPathCx {
                    top,
                    diags: &mut *diags,
                    loop_depth: 0,
                    in_handler: false,
                    hot: fd.hot,
                    unbounded: fd.unbounded,
                };
                hot_walk_block(&fd.body, &mut cx);
            }
            _ => {}
        }
    }
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| check_decl(item, top, diags));
    }
}

// === GH #723: decorator stacks =====================================

/// GH #723: the coherence of a fn's DECORATOR STACK.
///
/// `@unbounded`, `@hot`, `@budget(...)` and the effect assertions state
/// orthogonal things, so the parser accepts them in any order and in
/// any combination (before #723 a second decorator was a parse error,
/// which is why a `@unbounded` method needed a free `@no_syscall`
/// wrapper to carry both contracts). What the parser can no longer say
/// is whether a stack MEANS anything, and two shapes do not:
///
/// - the same decorator twice — the second is either redundant or
///   silently overrides the first (`@budget`), and either way the
///   author wrote something they did not mean;
/// - `@unbounded` against a contract that forbids allocation. Every
///   conflicting pair is this one pair in different spellings: the
///   `@hot` certification (which turns the allocation advisory
///   `@unbounded` silences into a hard error), an assertion that
///   forbids the `alloc` class, and the `@budget(alloc_per_call = 0)`
///   zero-allocation certificate. A decorator that contradicts its
///   neighbour cannot be enforced; both together is a statement about
///   the program that is not true of any program.
///
/// A ceiling ABOVE zero is not a conflict: `@budget(alloc_per_call =
/// 2) @unbounded` says "at most two allocations per call, and their
/// aggregate growth is deliberate" — a cache insert per call is
/// exactly that shape.
fn check_decorator_stacks(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    fn check_fn(fd: &FnDecl, diags: &mut Vec<Diag>) {
        // Duplicates. The general `@effects(...)` is exempt: it is a
        // set of clauses (`none:` / `publish:` / `is:` / …) and two of
        // them compose, where a bare flag can only repeat itself.
        let mut seen: Vec<&FnDecorator> = Vec::new();
        for d in &fd.decorators {
            if d.name == "effects" {
                continue;
            }
            if let Some(prev) = seen.iter().find(|p| p.name == d.name) {
                diags.push(
                    Diag::ty(
                        d.span,
                        format!(
                            "duplicate `@{}` on `{}` — it is already \
                             declared on this fn. State it once.",
                            d.name, fd.name.name
                        ),
                    )
                    .with_related(prev.span, "first written here"),
                );
                continue;
            }
            seen.push(d);
        }
        let Some(unbounded) = fd.decorators.iter().find(|d| d.name == "unbounded")
        else {
            return;
        };
        let spelled = |name: &str| -> Span {
            fd.decorators
                .iter()
                .find(|d| d.name == name)
                .map(|d| d.span)
                .unwrap_or(unbounded.span)
        };
        if let Some(hot) = fd.decorators.iter().find(|d| d.name == "hot") {
            diags.push(
                Diag::ty(
                    unbounded.span,
                    format!(
                        "`@unbounded` and `@hot` contradict on `{}`: \
                         `@unbounded` acknowledges an allocation this fn \
                         makes without a static bound, and `@hot` makes \
                         exactly that allocation a hard error. Keep the one \
                         that states the intent.",
                        fd.name.name
                    ),
                )
                .with_related(hot.span, "the hot-path certification"),
            );
        }
        // An assertion forbids `alloc` either openly (`none: {alloc}`)
        // or by closing a set that leaves it out (`only: {…}` — the
        // fn's inferred effects must be a SUBSET, so an unlisted class
        // is forbidden).
        let forbids_alloc = fd.effects.iter().any(|a| match a {
            EffectAssert::Forbid(cs) => cs.contains(&EffectClass::Alloc),
            EffectAssert::Only(cs) => !cs.contains(&EffectClass::Alloc),
            _ => false,
        });
        if forbids_alloc {
            diags.push(
                Diag::ty(
                    unbounded.span,
                    format!(
                        "`@unbounded` and an effect assertion that forbids \
                         `alloc` contradict on `{}`: one acknowledges an \
                         allocation without a static bound, the other says \
                         this fn performs none. Drop `@unbounded`, or let \
                         the assertion admit `alloc`.",
                        fd.name.name
                    ),
                )
                .with_related(spelled("effects"), "the assertion that forbids `alloc`"),
            );
        }
        if fd.budget == Some(0) {
            diags.push(
                Diag::ty(
                    unbounded.span,
                    format!(
                        "`@unbounded` and `@budget(alloc_per_call = 0)` \
                         contradict on `{}`: the budget is the zero-alloc \
                         certificate and `@unbounded` acknowledges an \
                         allocation without a static bound. A ceiling above \
                         zero does stack with `@unbounded` — bounded per \
                         call, deliberately unbounded in aggregate.",
                        fd.name.name
                    ),
                )
                .with_related(spelled("budget"), "the zero-alloc certificate"),
            );
        }
    }
    // A module nests top declarations arbitrarily deep; a decorator
    // inside one is still a decorator — `walk_decls` flattens them.
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| match item {
            TopDecl::Fn(fd) => check_fn(fd, diags),
            TopDecl::Locus(l) => {
                for m in &l.members {
                    if let LocusMember::Fn(fd) = m {
                        check_fn(fd, diags);
                    }
                }
            }
            _ => {}
        });
    }
}

/// Gap D (2026-07-17): `accept` without `release` on a daemon-shaped
/// locus. Declaring `release(c: C)` marks `C` a FLOW child (reclaimed
/// when its `run()` completes); without it every accept'd child is
/// RESIDENT — it lives until the accepting locus dissolves. On a
/// run-to-exit program that's fine (and the corpus's accept examples
/// are exactly that shape), but on a locus whose `run()` never
/// returns (`while true { accept-and-spawn }` — the daemon idiom)
/// resident children are O(accepted) growth until OOM. Daemon signal
/// (deliberately narrow, corpus-clean): the accepting locus's own
/// `run()` contains a literal `while true` loop.
fn check_accept_release(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    fn block_has_while_true(b: &Block) -> bool {
        b.stmts.iter().any(|s| match s {
            Stmt::While { cond, body, .. } => {
                matches!(cond, Expr::Literal(Literal::Bool(true), _))
                    || block_has_while_true(body)
            }
            Stmt::For { body, .. } => block_has_while_true(body),
            Stmt::If(i) => {
                block_has_while_true(&i.then_block)
                    || i.else_block.as_deref().is_some_and(|eb| match eb {
                        ElseBranch::Else(blk) => block_has_while_true(blk),
                        ElseBranch::ElseIf(i2) => {
                            block_has_while_true(&i2.then_block)
                        }
                    })
            }
            _ => false,
        })
    }
    // GH #825: a daemon-shaped locus inside a `module { … }` leaks
    // accepted children exactly as one at the top level does.
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(l) = item else { return };
            let mut accepts: Vec<(&LifecycleDecl, String)> = Vec::new();
            let mut releases: BTreeSet<String> = BTreeSet::new();
            let mut run_daemon = false;
            for m in &l.members {
                let LocusMember::Lifecycle(ld) = m else { continue };
                match ld.kind {
                    LifecycleKind::Accept => {
                        if let Some(p) = ld.params.first() {
                            if let TypeExpr::Named { path: qn, .. } = &p.ty {
                                if let Some(seg) = qn.segments.last() {
                                    accepts
                                        .push((ld, seg.name.clone()));
                                }
                            }
                        }
                    }
                    LifecycleKind::Release => {
                        if let Some(p) = ld.params.first() {
                            if let TypeExpr::Named { path: qn, .. } = &p.ty {
                                if let Some(seg) = qn.segments.last() {
                                    releases.insert(seg.name.clone());
                                }
                            }
                        }
                    }
                    LifecycleKind::Run => {
                        if block_has_while_true(&ld.body) {
                            run_daemon = true;
                        }
                    }
                    _ => {}
                }
            }
            if !run_daemon {
                return;
            }
            for (ld, child_ty) in accepts {
                if releases.contains(&child_ty) {
                    continue;
                }
                diags.push(Diag::warn(
                    ld.span,
                    format!(
                        "locus `{}` accepts `{}` children but declares no \
                         `release({}: {})`, and its `run()` loops forever — \
                         every accepted child is RESIDENT (lives until this \
                         locus dissolves), so long-running accept traffic \
                         grows memory without bound. Declare `release(c: \
                         {})` to reclaim each child when its `run()` \
                         completes (or `terminate` it from a handler body).",
                        l.name.name,
                        child_ty,
                        child_ty.to_lowercase().chars().next().unwrap_or('c'),
                        child_ty,
                        child_ty
                    ),
                ));
            }
        });
    }
}

/// Flag blocking calls on cooperative pools. Two outcomes:
///   * a non-main cooperative SUBSCRIBER whose `run()` blocks is a
///     **dead receiver** (error) — the blocking call starves the
///     dispatch that would deliver to its handlers;
///   * any other cooperative locus whose `run()` blocks gets a
///     **warning** — it stalls co-scheduled loci on the pool.
/// An event-driven subscriber (no blocking call — handlers + a sleep
/// loop, or `where async_io`) is flagged by neither: it receives fine.
fn check_cooperative_pool_blocking(
    bundle: &Bundle<'_>,
    diags: &mut Vec<Diag>,
) {
    // GH #825: all three passes flatten `module { … }`. The index of
    // free fns is what the interprocedural blocking call graph is
    // built from, so a module-nested helper that blocks has to be in
    // it or a top-level `run()` calling it looks clean.
    let mut local_loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    let mut free_fns: BTreeMap<String, &Block> = BTreeMap::new();
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| match item {
            TopDecl::Locus(l) => {
                local_loci.insert(l.name.name.as_str(), l);
            }
            TopDecl::Fn(f) => {
                free_fns.insert(f.name.name.clone(), &f.body);
            }
            _ => {}
        });
    }
    // Interprocedural call graph for the warning path: free fns that
    // block (directly or via another blocking free fn).
    let blocking_free = blocking_free_fns(&free_fns);

    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(main) = item else { return };
            if !main.is_main {
                return;
            }
            // The placement block is optional: phase 1 (blocking-call
            // diagnostics) needs entries, but phase 2 (run() starvation)
            // also covers fields with no entry (they default to pool
            // `main`).
            let pb = main.members.iter().find_map(|m| match m {
                LocusMember::Placement(pb) => Some(pb),
                _ => None,
            });
            // field name -> single-segment locus type name.
            let mut field_locus: BTreeMap<&str, &str> = BTreeMap::new();
            for m in &main.members {
                let LocusMember::Params(params) = m else { continue };
                for pd in &params.params {
                    if let Some(TypeExpr::Named { path, .. }) = &pd.ty {
                        if path.segments.len() == 1 {
                            field_locus.insert(
                                pd.name.name.as_str(),
                                path.segments[0].name.as_str(),
                            );
                        }
                    }
                }
            }

            // Pools where the dead-receiver ERROR fired — the pool-
            // starvation warning (phase 2) is suppressed there; the
            // error already says the pool thread is monopolized.
            let mut errored_pools: BTreeSet<String> = BTreeSet::new();

            for entry in pb.map(|pb| pb.entries.as_slice()).unwrap_or(&[]) {
                let PlacementSpec::Cooperative { pool, .. } = &entry.spec else {
                    continue;
                };
                // `where async_io` parks blocking I/O — not a stall.
                if entry
                    .constraints
                    .iter()
                    .any(|c| matches!(c.kind, PlacementConstraint::AsyncIo))
                {
                    continue;
                }
                let Some(locus_name) = field_locus.get(entry.field.name.as_str())
                else {
                    continue;
                };
                let Some(decl) = local_loci.get(locus_name) else {
                    continue;
                };
                let Some(run_body) = decl.members.iter().find_map(|m| match m {
                    LocusMember::Lifecycle(LifecycleDecl {
                        kind: LifecycleKind::Run,
                        body,
                        ..
                    }) => Some(body),
                    _ => None,
                }) else {
                    continue;
                };
                // The locus's own methods (named fns + lifecycle
                // bodies) form the intra-locus call graph for the
                // interprocedural warning.
                let mut methods: BTreeMap<String, &Block> = BTreeMap::new();
                for m in &decl.members {
                    match m {
                        LocusMember::Fn(f) => {
                            methods.insert(f.name.name.clone(), &f.body);
                        }
                        LocusMember::Lifecycle(LifecycleDecl {
                            kind, body, ..
                        }) => {
                            methods.insert(format!("{:?}", kind), body);
                        }
                        _ => {}
                    }
                }
                let blocking_self =
                    blocking_self_methods(&methods, &blocking_free);

                // WARNING trigger: blocking reachable from run() either
                // directly or through a helper fn / self-method.
                let Some((deep_call, deep_span)) = find_blocking_deep_in_block(
                    run_body,
                    &blocking_free,
                    &blocking_self,
                ) else {
                    // Event-driven (nothing blocking reachable): the
                    // pool thread stays free, the bus dispatch runs,
                    // cells arrive. Nothing to flag — even a non-main
                    // cooperative subscriber receives fine this way.
                    continue;
                };
                // DEAD-RECEIVER trigger stays direct-call-only — its
                // call-graph surface is deliberately NOT widened (it
                // over-fired once; see below).
                let direct = find_blocking_in_block(run_body);
                let pool_name =
                    pool.as_ref().map(|i| i.name.as_str()).unwrap_or("main");
                // Handlers for topics this locus does NOT itself publish
                // (a self-publish→subscribe is a devirtualized direct
                // call, not a bus receive).
                let dead = external_subscription_handlers(decl);
                if pool_name != "main" && !dead.is_empty() && direct.is_some() {
                    let (call, span) = direct.expect("is_some checked");
                    errored_pools.insert(pool_name.to_string());
                    // DEAD RECEIVER (error). A non-main cooperative
                    // subscriber whose run() blocks: cross-process
                    // dispatch reaches a cooperative locus only when its
                    // pool thread is free to run the dispatch, and a
                    // blocking call monopolizes it, so these handlers
                    // never fire. (Corrected 2026-06-03 from the
                    // placement-only rule, which over-fired on
                    // event-driven subscribers — `Reader`/`Dispatcher`
                    // received fine for 16h+ in production.)
                    diags.push(Diag::ty(
                        span,
                        format!(
                            "locus `{}` (field `{}`) subscribes to bus topics \
                             ({}) but its `run()` makes the blocking call `{}` \
                             while placed `cooperative(pool = {})`. The \
                             blocking call monopolizes the pool's thread, so \
                             the dispatch that would deliver those cells never \
                             runs — the handlers can't fire. (An event-driven \
                             subscriber that yields — handlers plus a \
                             `time::sleep` loop, or `where async_io` — receives \
                             fine; the problem is the blocking call, not the \
                             placement.) Use `pinned` (its own thread + a \
                             mailbox drained at sleep/yield), or keep `run()` \
                             non-blocking.",
                            locus_name,
                            entry.field.name,
                            dead.join(", "),
                            call,
                            pool_name,
                        ),
                    ));
                } else {
                    // Blocking on a cooperative pool stalls co-scheduled
                    // loci (and the pool's bus drain), even when this
                    // locus isn't itself a subscriber. Interprocedural:
                    // `deep_call` may name a helper fn / self-method that
                    // blocks transitively, not just a literal stdlib op.
                    diags.push(Diag::warn(
                        deep_span,
                        format!(
                            "locus `{}` (field `{}`) is placed `cooperative(pool \
                             = {})` and reaches the blocking call `{}` in its \
                             `run()`. A blocking call holds the pool's OS thread \
                             for its whole duration, stalling every other locus \
                             scheduled on `{}` (and the pool's bus drain). Use \
                             `pinned` (its own thread — the prescribed shape for \
                             blocking I/O), or `cooperative(pool = {}) where \
                             async_io` (which parks on I/O readiness instead of \
                             blocking the thread).",
                            locus_name,
                            entry.field.name,
                            pool_name,
                            deep_call,
                            pool_name,
                            pool_name,
                        ),
                    ));
                }
            }

            // === phase 2: pool starvation ======================
            //
            // Two (or more) statically non-returning `run()` bodies on
            // one cooperative pool: the pool runs each `run()` cell to
            // completion in birth order, so the first-born runs forever
            // and the later `run()` bodies never start. Distinct from
            // the blocking-call diagnostics above — the archetypal
            // shape (a `sleep` metronome) contains no blocking call,
            // and the starved locus needn't be a subscriber. A locus
            // can legitimately draw both warnings (blocking AND
            // starving a sibling); they name different defects.
            let mut by_pool: BTreeMap<String, Vec<(String, Span)>> =
                BTreeMap::new();
            for m in &main.members {
                let LocusMember::Params(params) = m else { continue };
                for pd in &params.params {
                    let field = pd.name.name.as_str();
                    let entry = pb.and_then(|pb| {
                        pb.entries.iter().find(|e| e.field.name == field)
                    });
                    let pool_name: String = match entry.map(|e| (&e.spec, e)) {
                        Some((PlacementSpec::Pinned { .. }, _)) => continue,
                        Some((PlacementSpec::Cooperative { pool, .. }, e)) => {
                            // `where async_io` run-cells are parkable
                            // coros — co-scheduled runs interleave at
                            // park points, so no starvation claim.
                            if e.constraints.iter().any(|c| {
                                matches!(c.kind, PlacementConstraint::AsyncIo)
                            }) {
                                continue;
                            }
                            pool.as_ref()
                                .map(|i| i.name.clone())
                                .unwrap_or_else(|| "main".to_string())
                        }
                        // No placement entry: fields default to pool
                        // `main` (mirrors compute_pool_of_locus_type).
                        None => "main".to_string(),
                    };
                    // Stdlib loci with known non-terminating run()
                    // (typecheck can't see their bodies).
                    if let Some(TypeExpr::Named { path, .. }) = &pd.ty {
                        let segs: Vec<&str> = path
                            .segments
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect();
                        if is_known_long_running_stdlib(&segs) {
                            by_pool.entry(pool_name).or_default().push((
                                format!(
                                    "locus `{}` (field `{}`)",
                                    segs.join("::"),
                                    field
                                ),
                                pd.span,
                            ));
                            continue;
                        }
                    }
                    let Some(locus_name) = field_locus.get(field) else {
                        continue;
                    };
                    let Some(decl) = local_loci.get(locus_name) else {
                        continue;
                    };
                    let Some(run_body) = decl.members.iter().find_map(|m| {
                        match m {
                            LocusMember::Lifecycle(LifecycleDecl {
                                kind: LifecycleKind::Run,
                                body,
                                ..
                            }) => Some(body),
                            _ => None,
                        }
                    }) else {
                        continue;
                    };
                    let Some(span) = run_statically_nonreturning(run_body, decl)
                    else {
                        continue;
                    };
                    by_pool.entry(pool_name).or_default().push((
                        format!("locus `{}` (field `{}`)", locus_name, field),
                        span,
                    ));
                }
            }
            // The main locus itself runs on pool `main`; its run()
            // begins only after params-init completes, so it is
            // birth-ordered last.
            let mut main_starved_on_main = false;
            if let Some(main_run) = main.members.iter().find_map(|m| match m {
                LocusMember::Lifecycle(LifecycleDecl {
                    kind: LifecycleKind::Run,
                    body,
                    ..
                }) => Some(body),
                _ => None,
            }) {
                if let Some(span) = run_statically_nonreturning(main_run, main) {
                    by_pool.entry("main".to_string()).or_default().push((
                        format!("the main locus `{}`", main.name.name),
                        span,
                    ));
                    main_starved_on_main = true;
                }
            }
            for (pool_name, members) in &by_pool {
                if members.len() < 2 || errored_pools.contains(pool_name) {
                    continue;
                }
                let names = members
                    .iter()
                    .map(|(d, _)| d.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let (first_name, first_span) = &members[0];
                let main_note = if pool_name == "main" && main_starved_on_main {
                    format!(
                        " For pool `main` the last of these is the main \
                         locus's own `run()`, which begins only after \
                         params-init — boot appears to complete, then the \
                         process sits idle."
                    )
                } else {
                    String::new()
                };
                diags.push(Diag::warn(
                    *first_span,
                    format!(
                        "cooperative pool `{}` is shared by {}, whose `run()` \
                         bodies all statically never return (terminal `while` \
                         loop with no `break`/`return`/`terminate`). A \
                         cooperative pool runs each `run()` to completion in \
                         birth order: {}'s `run()` starts first and never \
                         finishes, so the later `run()` bodies never start. \
                         Bus handlers still fire at sleep/yield drains, which \
                         makes the starvation look like a healthy idle.{} Use \
                         `pinned` (its own thread + a mailbox drained at \
                         sleep/yield) for all but one of them, or keep the \
                         extra `run()` bodies non-blocking and event-driven \
                         (handlers plus a bounded loop).",
                        pool_name, names, first_name, main_note,
                    ),
                ));
            }

            // === birth-order trap (2026-08-03, downstream handoff) ===
            //
            // Strictly worse than the pool-starvation warning above,
            // and not implied by it. That one needs TWO non-returning
            // `run()` bodies and claims only that the later `run()`
            // never starts. This one needs ONE: a locus field whose
            // `run()` runs INLINE on the main thread (default
            // placement, or an explicit `cooperative(pool = main)`)
            // and never returns means every param declared after it
            // is never even BORN — its `birth()` never runs, so the
            // subscriptions it registers, the sockets it binds and
            // the children it accepts silently never exist.
            //
            // Measured 2026-08-03 (all four placements):
            //   default                    -> blocks later births
            //   cooperative(pool = main)    -> blocks later births
            //   cooperative(pool = io)      -> posted to a worker, no
            //   pinned                      -> own thread, no
            // The LATER field's own placement is irrelevant — the
            // instantiation itself runs inline on main, so even a
            // `pinned` sibling declared after the blocker is stuck.
            // Only the blocker's placement matters.
            //
            // This is what a downstream handoff reported as "a bus
            // handler's write to a `self` param isn't observed by
            // `run()`", which we in turn mis-filed as a cooperative-
            // child handler-cadence question. It is neither: the
            // drain is fine and the cadence is fine; the publisher
            // simply had not been born yet.
            {
                // Params in declaration order, tagged with whether
                // each one blocks the births that follow it.
                let mut ordered: Vec<(&str, Span, bool, String)> = Vec::new();
                for m in &main.members {
                    let LocusMember::Params(params) = m else { continue };
                    for pd in &params.params {
                        let field = pd.name.name.as_str();
                        let entry = pb.and_then(|pb| {
                            pb.entries.iter().find(|e| e.field.name == field)
                        });
                        // Only an inline-on-main run() blocks. A
                        // pinned or off-main-pool field runs
                        // elsewhere; `where async_io` parks.
                        let inline_on_main = match entry.map(|e| (&e.spec, e)) {
                            Some((PlacementSpec::Pinned { .. }, _)) => false,
                            Some((PlacementSpec::Cooperative { pool, .. }, e)) => {
                                !e.constraints.iter().any(|c| {
                                    matches!(c.kind, PlacementConstraint::AsyncIo)
                                }) && pool
                                    .as_ref()
                                    .map(|i| i.name == "main")
                                    .unwrap_or(true)
                            }
                            None => true,
                        };
                        let segs: Vec<&str> = match &pd.ty {
                            Some(TypeExpr::Named { path, .. }) => path
                                .segments
                                .iter()
                                .map(|s| s.name.as_str())
                                .collect(),
                            _ => Vec::new(),
                        };
                        let display = if segs.is_empty() {
                            field.to_string()
                        } else {
                            format!("{}: {}", field, segs.join("::"))
                        };
                        // Non-returning: a proven-terminal loop in a
                        // local locus, or a stdlib locus on the
                        // known-long-running allowlist (whose body
                        // typecheck cannot see).
                        let nonreturning = if is_known_long_running_stdlib(&segs) {
                            true
                        } else {
                            field_locus
                                .get(field)
                                .and_then(|n| local_loci.get(n))
                                .and_then(|decl| {
                                    decl.members
                                        .iter()
                                        .find_map(|m| match m {
                                            LocusMember::Lifecycle(
                                                LifecycleDecl {
                                                    kind: LifecycleKind::Run,
                                                    body,
                                                    ..
                                                },
                                            ) => Some((body, *decl)),
                                            _ => None,
                                        })
                                        .and_then(|(body, decl)| {
                                            run_statically_nonreturning(
                                                body, decl,
                                            )
                                        })
                                })
                                .is_some()
                        };
                        ordered.push((
                            field,
                            pd.span,
                            inline_on_main && nonreturning,
                            display,
                        ));
                    }
                }
                // Report the FIRST blocker only: every later one is a
                // consequence of it, not an independent defect.
                if let Some(i) = ordered.iter().position(|(_, _, b, _)| *b) {
                    let starved: Vec<&str> = ordered[i + 1..]
                        .iter()
                        .map(|(_, _, _, d)| d.as_str())
                        .collect();
                    if !starved.is_empty() {
                        let (field, span, _, _) = &ordered[i];
                        diags.push(Diag::warn(
                            *span,
                            format!(
                                "params field `{}` runs inline on the main \
                                 thread and its `run()` statically never \
                                 returns (terminal `while` loop with no \
                                 `break`/`return`/`terminate`), so the \
                                 params declared after it are never BORN: \
                                 {}. Their `birth()` bodies never run, so \
                                 any subscription they register, socket \
                                 they bind or child they accept silently \
                                 never exists — the process looks like it \
                                 booted and then idles. Either declare \
                                 them BEFORE `{}`, or move `{}` off the \
                                 main thread with `placement {{ {}: \
                                 pinned; }}` (own thread) or \
                                 `cooperative(pool = io)` (posted to a \
                                 worker); both let the remaining params \
                                 finish being born.",
                                field,
                                starved.join(", "),
                                field,
                                field,
                                field,
                            ),
                        ));
                    }
                }
            }
        });
    }
}

fn check_nested_long_running_child(
    bundle: &Bundle<'_>,
    diags: &mut Vec<Diag>,
) {
    // Build a name → LocusDecl index across the bundle so we can
    // resolve params-field locus types to their target body.
    // GH #825: both passes flatten `module { … }`. The index is half
    // the rule — a TOP-LEVEL parent holding a module-nested child
    // resolves the child's type through it, and an index that stops
    // at the top level answers "not long-running" for every one.
    let mut local_loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                local_loci.insert(l.name.name.as_str(), l);
            }
        });
    }

    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(parent) = item else {
                return;
            };
            if parent.is_main {
                return;
            }
            if !locus_has_nontrivial_run(parent) {
                return;
            }
            // Walk params fields. Each ParamDecl whose declared
            // type is a locus reference goes through the locus-
            // type-with-run check.
            for member in &parent.members {
                let LocusMember::Params(pb) = member else {
                    continue;
                };
                for pd in &pb.params {
                    let Some(ty) = &pd.ty else {
                        continue;
                    };
                    let TypeExpr::Named { path, .. } = ty else {
                        continue;
                    };
                    let segs: Vec<&str> = path
                        .segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect();
                    // Single-segment names: look up locally.
                    // Multi-segment: check against the known-
                    // long-running stdlib allowlist.
                    let target_is_long_running = if segs.len() == 1 {
                        local_loci
                            .get(segs[0])
                            .filter(|l| !l.is_main)
                            .map(|l| locus_has_nontrivial_run(l))
                            .unwrap_or(false)
                    } else {
                        is_known_long_running_stdlib(&segs)
                    };
                    if !target_is_long_running {
                        continue;
                    }
                    let target_display = segs.join("::");
                    diags.push(Diag::ty(
                        pd.span,
                        format!(
                            "locus `{}` declares params field `{}: {}` \
                             with a non-trivial `run()` body of its own. \
                             Nested cooperative children share the parent's \
                             OS thread; the child's `run()` runs to \
                             completion before the parent's `run()` begins, \
                             so a long-running child (`{}`'s accept loop \
                             never returns) starves the parent.\n\n\
                             Canonical fix: hoist both loci to siblings of \
                             a `main locus` and use a `placement {{ }}` \
                             block to put them on different pools.\n\n\
                             ```\n\
                             main locus App {{\n\
                                 params {{\n\
                                     parent: {} = {} {{ ... }};\n\
                                     {}: {} = {} {{ ... }};\n\
                                 }}\n\
                                 placement {{\n\
                                     {}: cooperative(pool = io);\n\
                                 }}\n\
                             }}\n\
                             ```\n\n\
                             See spec/runtime.md § Long-running cooperative \
                             children: placement closes Item D.",
                            parent.name.name,
                            pd.name.name,
                            target_display,
                            target_display,
                            parent.name.name,
                            parent.name.name,
                            pd.name.name,
                            target_display,
                            target_display,
                            pd.name.name,
                        ),
                    ));
                }
            }
        });
    }
}

/// One node of the param-default containment graph (GH #813): a
/// locus name plus the field names a literal SUPPLIES. The defaults a
/// literal expands are exactly the ones it does not supply, so the
/// pair — not the locus alone — is what the construction re-enters.
type ContainmentState = (String, Vec<String>);

/// One edge out of a param default (GH #870): the state the default
/// constructs, and the factory fn it went through — `None` when the
/// default spells the literal itself. The graph is the same either
/// way; the name is what the diagnostic shows the author, who is
/// looking at a call, not at a literal.
type ContainmentEdge = (ContainmentState, Option<String>);

/// GH #813: a locus whose construction requires constructing one of
/// its own kind.
///
/// `locus Node { params { next: Node = Node { n: 1 }; } }` is not a
/// linked list — it is a locus that cannot exist. The `Node` the
/// default builds leaves ITS `next` to the same default, which builds
/// another, and the nesting has no floor. It cannot be broken from a
/// call site either: writing `Node { next: … }` needs a `Node` to
/// hand over, and building one asks the same question again. So the
/// declaration is the error, independently of whether anything
/// instantiates it.
///
/// Before this the program passed `hale check` and
/// `lower_locus_instantiation` recursed through the default until the
/// compiler's own stack ran out ("thread 'main' has overflowed its
/// stack"). Codegen now keeps the same guard for itself — it must,
/// since `build_executable` never runs this checker — but a stack
/// trace is not a diagnostic, and the author's mistake is at a param.
///
/// The graph is over BY-VALUE containment: an edge `L → M` where a
/// param default of `L` *constructs* an `M` — an `M { … }` locus
/// literal anywhere in the default's expression, or (GH #870) a call
/// to a fn that freshly constructs one.
///
/// The second half is the residue #813 left behind. `next: Node =
/// make()` with `fn make() -> Node { return Node { }; }` compiled —
/// lowering a call emits a call rather than inlining the callee, so
/// nothing recursed at COMPILE time and there was no crash to
/// prevent — and then overflowed the program's own stack at RUN time,
/// because every `Node` `make` builds leaves ITS `next` to the same
/// default, which calls `make` again. The ring is the same ring; only
/// the spelling of one edge changed.
///
/// Telling that apart from an accessor handing back a `Node` somebody
/// else already owns is a whole-program question, and
/// `fresh_locus_factory_products` is the answer: codegen's
/// `compute_fresh_locus_factories` classification, mirrored over the
/// bundle rather than imported (hale-codegen depends on hale-types,
/// so the dependency cannot run the other way; only the pure "what
/// does this fn hand back" part is repeated, and it answers with each
/// literal's supplied fields, which the ownership map has no use
/// for). A call it cannot see as fresh — an accessor, a method, a
/// `std::` or cross-seed path — takes no edge and stays accepted,
/// exactly as before; a program like that recurses at RUN time only
/// if the callee really does build one, which is what `@no_recursion`
/// is the contract for.
///
/// A node is (locus, supplied field names) rather than the locus
/// alone: `A { n: 1, m: 2 }` written inside `A`'s own default for `m`
/// expands no default and terminates, and keying on the type would
/// report it as a cycle.
///
/// Nothing is reported for a locus this bundle cannot see. A single
/// file of a multi-file seed holds no `TopDecl::Locus` for its
/// sibling's types, so a cycle that crosses files simply has no edge
/// here and stays permissive until the whole seed is checked
/// together — the gating every other cross-file rule uses, arrived at
/// by having nothing to say rather than by a flag. The literal walk
/// is likewise deliberately partial (it does not descend into a
/// block, `if` or `match` body): an unvisited form costs a report,
/// never a false one, and codegen's guard is underneath it.
fn check_self_containing_locus(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    fn collect<'a>(
        items: &'a [TopDecl],
        out: &mut BTreeMap<&'a str, &'a LocusDecl>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    out.entry(l.name.name.as_str()).or_insert(l);
                }
                TopDecl::Module(m) => collect(&m.items, out),
                _ => {}
            }
        }
    }
    let mut loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    for program in bundle.programs.values() {
        collect(&program.items, &mut loci);
    }
    if loci.is_empty() {
        return;
    }
    // GH #870: which fns hand back a locus they freshly built, and
    // what each call constructs. Computed once for the bundle.
    let factories = fresh_locus_factory_products(bundle, &loci);
    // Classic gray/black DFS. `finished` is the black set: every
    // cycle reachable from a state was found while that state was
    // being explored, so re-entering it later has nothing to add —
    // which is also what keeps one cycle from being reported once per
    // locus on it.
    let mut finished: BTreeSet<ContainmentState> = BTreeSet::new();
    let mut reported: BTreeSet<(u32, String)> = BTreeSet::new();
    for name in loci.keys().copied() {
        let mut path: Vec<ContainmentState> = Vec::new();
        walk_param_default_containment(
            name,
            &[],
            &loci,
            &factories,
            &mut path,
            &mut finished,
            &mut reported,
            diags,
        );
    }
}

/// One DFS step of the GH #813 containment walk. `supplied` is the
/// set of field names the literal that got us here wrote out; every
/// OTHER param of `locus` expands its default, and each locus literal
/// inside that default — plus (GH #870) each fresh-factory call — is
/// an edge.
fn walk_param_default_containment(
    locus: &str,
    supplied: &[String],
    loci: &BTreeMap<&str, &LocusDecl>,
    factories: &BTreeMap<String, Vec<ContainmentState>>,
    path: &mut Vec<ContainmentState>,
    finished: &mut BTreeSet<ContainmentState>,
    reported: &mut BTreeSet<(u32, String)>,
    diags: &mut Vec<Diag>,
) {
    let state: ContainmentState = (locus.to_string(), supplied.to_vec());
    if finished.contains(&state) {
        return;
    }
    let Some(decl) = loci.get(locus) else {
        // A sibling file's locus, or a stdlib one reached by a
        // multi-segment path: no body here, no edge, no report.
        return;
    };
    path.push(state.clone());
    for member in &decl.members {
        let LocusMember::Params(pb) = member else {
            continue;
        };
        for pd in &pb.params {
            if supplied.iter().any(|s| s == &pd.name.name) {
                continue;
            }
            let ParamInit::Value(e) = &pd.init else {
                continue;
            };
            let mut built: Vec<ContainmentEdge> = Vec::new();
            collect_constructed_loci(e, loci, factories, &mut built);
            for (child, via) in built {
                let Some(at) = path.iter().position(|s| *s == child) else {
                    walk_param_default_containment(
                        &child.0, &child.1, loci, factories, path,
                        finished, reported, diags,
                    );
                    continue;
                };
                // The cycle, as a ring of type names, rotated so the
                // locus the author is reading about comes first.
                let mut ring: Vec<&str> =
                    path[at..].iter().map(|s| s.0.as_str()).collect();
                if let Some(k) = ring.iter().position(|n| *n == locus) {
                    ring.rotate_left(k);
                }
                let chain = if ring.len() > 1 {
                    format!(" (`{}` → `{}`)", ring.join("` → `"), ring[0])
                } else {
                    String::new()
                };
                // The two spellings of one edge: a literal in the
                // default, or a call to a fn that builds one (GH
                // #870). Same rule, same ring — what differs is what
                // the author is looking at on that line.
                let (how, why) = match &via {
                    None => (
                        format!("defaults to a `{}`", child.0),
                        "every one the default builds needs another, \
                         and no locus literal can end the chain"
                            .to_string(),
                    ),
                    Some(f) => (
                        format!(
                            "defaults to `{}()`, which builds a fresh \
                             `{}`",
                            f, child.0,
                        ),
                        "every one the factory builds asks the same \
                         default again, so the program compiles and \
                         then recurses until its stack overflows"
                            .to_string(),
                    ),
                };
                let message = format!(
                    "param `{}` of `{}` {}; a locus cannot contain \
                     itself by value{} — {}. Drop the default and \
                     take the child from the caller (`{}: {};`), or \
                     hold a value rather than a locus.",
                    pd.name.name,
                    locus,
                    how,
                    chain,
                    why,
                    pd.name.name,
                    child.0,
                );
                if reported.insert((pd.span.start.0, message.clone())) {
                    diags.push(Diag::ty(pd.span, message));
                }
            }
        }
    }
    path.pop();
    finished.insert(state);
}

/// Every locus `M` constructed while `e` is evaluated, as (locus
/// name, the field names it supplies) plus the factory fn the default
/// reached it through, if any.
///
/// Two spellings construct one:
///
///   * a locus literal `M { … }`. Nested literals count too — a
///     literal inside a literal's field is constructed just as surely
///     as the outer one. Only single-segment paths that name a locus
///     in this bundle are edges; a `type` literal, a stdlib path and
///     a sibling file's name are all skipped;
///   * GH #870: a call to a fn `fresh_locus_factory_products`
///     classified as freshly building one. The states it contributes
///     are the literals that fn hands back, so `fn make() -> Node {
///     return Node { n: 5 }; }` contributes `(Node, [n])` — the same
///     node the literal `Node { n: 5 }` would.
fn collect_constructed_loci(
    e: &Expr,
    loci: &BTreeMap<&str, &LocusDecl>,
    factories: &BTreeMap<String, Vec<ContainmentState>>,
    out: &mut Vec<ContainmentEdge>,
) {
    match e {
        Expr::Struct { path, inits, .. } => {
            if path.segments.len() == 1 {
                let name = path.segments[0].name.as_str();
                if loci.contains_key(name) {
                    let mut supplied: Vec<String> = inits
                        .iter()
                        .map(|i| i.name.name.clone())
                        .collect();
                    supplied.sort();
                    supplied.dedup();
                    out.push(((name.to_string(), supplied), None));
                }
            }
            for i in inits {
                collect_constructed_loci(&i.value, loci, factories, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_constructed_loci(left, loci, factories, out);
            collect_constructed_loci(right, loci, factories, out);
        }
        Expr::Unary { operand, .. } => {
            collect_constructed_loci(operand, loci, factories, out)
        }
        Expr::Call { callee, args, .. } => {
            if let Some(f) = plain_callee_name(callee) {
                if let Some(states) = factories.get(f) {
                    for s in states {
                        out.push((s.clone(), Some(f.to_string())));
                    }
                }
            }
            collect_constructed_loci(callee, loci, factories, out);
            for a in args {
                collect_constructed_loci(a, loci, factories, out);
            }
        }
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            collect_constructed_loci(receiver, loci, factories, out)
        }
        Expr::Index { receiver, index, .. } => {
            collect_constructed_loci(receiver, loci, factories, out);
            collect_constructed_loci(index, loci, factories, out);
        }
        Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
            for p in parts {
                collect_constructed_loci(p, loci, factories, out);
            }
        }
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => {
            collect_constructed_loci(inner, loci, factories, out)
        }
        Expr::ArrayRepeat { val, .. } => {
            collect_constructed_loci(val, loci, factories, out)
        }
        Expr::Range { lo, hi, .. } => {
            collect_constructed_loci(lo, loci, factories, out);
            collect_constructed_loci(hi, loci, factories, out);
        }
        Expr::Approx { left, right, tolerance, .. } => {
            collect_constructed_loci(left, loci, factories, out);
            collect_constructed_loci(right, loci, factories, out);
            collect_constructed_loci(tolerance, loci, factories, out);
        }
        Expr::Or { inner, disposition, .. } => {
            collect_constructed_loci(inner, loci, factories, out);
            match disposition {
                OrDisposition::Substitute(s) => {
                    collect_constructed_loci(s, loci, factories, out)
                }
                OrDisposition::Fail(p, _) => {
                    collect_constructed_loci(p, loci, factories, out)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// The single-segment name a callee spells, or `None` for a method,
/// a path, or anything computed. A qualified callee resolves through
/// codegen's import-rename table, which the checker has no
/// equivalent of, so it is not followed here.
fn plain_callee_name(callee: &Expr) -> Option<&str> {
    match callee {
        Expr::Ident(i) => Some(i.name.as_str()),
        Expr::Path(q) if q.segments.len() == 1 => {
            Some(q.segments[0].name.as_str())
        }
        _ => None,
    }
}

/// GH #870: every fn in the bundle that hands back a locus it
/// freshly BUILT, with the [`ContainmentState`]s a call to it
/// constructs.
///
/// The classification is codegen's `compute_fresh_locus_factories`:
/// a fn qualifies when every arm it returns is a literal of its
/// declared locus, a call to another qualifying fn of that locus, or
/// one local binding that was itself bound to either — a fixpoint,
/// since the second and third forms are answers about other fns.
/// Two deliberate differences from the codegen map:
///
///   * it answers with each literal's SUPPLIED FIELD NAMES, not just
///     the locus. `Node { n: 5 }` expands every default but `n`, and
///     the containment graph's nodes are (locus, supplied) pairs for
///     exactly that reason. Ownership has no use for the names, so
///     the codegen map does not carry them;
///   * it drops the escape analysis codegen runs on a returned
///     binding (`body_ok`). That asks who OWNS the value; the
///     question here is only whether one was built, which the `let`
///     already answered.
///
/// Everything it cannot follow makes a fn opaque rather than fresh,
/// which costs a report and never invents one: a multi-segment
/// return type or callee (a `std::` or cross-seed path), a carrier
/// arm (`return if c { … } else { … }`), a returned binding written
/// twice, and any statement form that could hide a `return` this walk
/// does not model.
fn fresh_locus_factory_products(
    bundle: &Bundle<'_>,
    loci: &BTreeMap<&str, &LocusDecl>,
) -> BTreeMap<String, Vec<ContainmentState>> {
    /// `M { … }` spelled as a single-segment path naming `locus`, as
    /// the state it constructs.
    fn literal_state(e: &Expr, locus: &str) -> Option<ContainmentState> {
        let Expr::Struct { path, inits, .. } = e else { return None };
        if path.segments.len() != 1 || path.segments[0].name != locus {
            return None;
        }
        let mut supplied: Vec<String> =
            inits.iter().map(|i| i.name.name.clone()).collect();
        supplied.sort();
        supplied.dedup();
        Some((locus.to_string(), supplied))
    }

    /// What one returned expression hands back, or `None` if this
    /// walk cannot see it as a fresh `locus`. `lets` is empty on the
    /// recursive step: a binding is followed one level, since
    /// chasing a chain of them needs flow sensitivity this walk does
    /// not have.
    fn arm_states(
        e: &Expr,
        locus: &str,
        lets: &[(&str, &Expr)],
        known: &BTreeMap<String, Vec<ContainmentState>>,
    ) -> Option<Vec<ContainmentState>> {
        if let Some(s) = literal_state(e, locus) {
            return Some(vec![s]);
        }
        if let Expr::Call { callee, .. } = e {
            let states = known.get(plain_callee_name(callee)?)?;
            if states.iter().all(|(l, _)| l == locus) {
                return Some(states.clone());
            }
            return None;
        }
        if let Expr::Ident(i) = e {
            let bound: Vec<&Expr> = lets
                .iter()
                .filter(|(n, _)| *n == i.name)
                .map(|(_, v)| *v)
                .collect();
            if bound.len() != 1 {
                return None;
            }
            return arm_states(bound[0], locus, &[], known);
        }
        None
    }

    /// Every value a fn body can hand back, and every `let` in it.
    /// `false` means the walk met a statement form that could carry a
    /// `return` it does not model — an unseen one would make an
    /// accessor look like a factory, so the fn is opaque instead.
    ///
    /// A nested block's tail counts as a value the body produces: a
    /// locus literal evaluated anywhere in the callee is constructed
    /// as surely as one it returns.
    fn collect_returns<'a>(
        b: &'a Block,
        rets: &mut Vec<&'a Expr>,
        lets: &mut Vec<(&'a str, &'a Expr)>,
    ) -> bool {
        for s in &b.stmts {
            match s {
                Stmt::Return(Some(e), _) => rets.push(e),
                Stmt::Let { name, value, .. } => {
                    lets.push((name.name.as_str(), value))
                }
                Stmt::If(i) => {
                    if !if_returns(i, rets, lets) {
                        return false;
                    }
                }
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        match &arm.body {
                            MatchArmBody::Block(bb) => {
                                if !collect_returns(bb, rets, lets) {
                                    return false;
                                }
                            }
                            // An arm evaluated for its effect: a
                            // match STATEMENT hands nothing back.
                            MatchArmBody::Expr(_) => {}
                        }
                    }
                }
                Stmt::For { body, .. }
                | Stmt::While { body, .. }
                | Stmt::Block(body) => {
                    if !collect_returns(body, rets, lets) {
                        return false;
                    }
                }
                Stmt::Return(None, _)
                | Stmt::LetTuple { .. }
                | Stmt::Assign { .. }
                | Stmt::Expr(_)
                | Stmt::Break(_)
                | Stmt::Continue(_)
                | Stmt::Fail { .. }
                | Stmt::Yield(_)
                | Stmt::Terminate(_)
                | Stmt::Reperspective { .. }
                | Stmt::Recovery { .. }
                | Stmt::Violate { .. }
                | Stmt::Send { .. } => {}
                _ => return false,
            }
        }
        if let Some(t) = &b.tail {
            rets.push(t);
        }
        true
    }

    fn if_returns<'a>(
        i: &'a IfStmt,
        rets: &mut Vec<&'a Expr>,
        lets: &mut Vec<(&'a str, &'a Expr)>,
    ) -> bool {
        if !collect_returns(&i.then_block, rets, lets) {
            return false;
        }
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => collect_returns(b, rets, lets),
            Some(ElseBranch::ElseIf(nested)) => {
                if_returns(nested, rets, lets)
            }
            None => true,
        }
    }

    // A name declared twice keeps the first declaration, as the
    // locus map above does: a bundle with two is ill-formed for
    // another reason, and this pass is not the place to say so.
    let mut fns: BTreeMap<&str, &FnDecl> = BTreeMap::new();
    for program in bundle.programs.values() {
        for item in flat_decls(&program.items) {
            if let TopDecl::Fn(f) = item {
                fns.entry(f.name.name.as_str()).or_insert(f);
            }
        }
    }
    let mut out: BTreeMap<String, Vec<ContainmentState>> = BTreeMap::new();
    loop {
        let mut added = false;
        for (name, f) in &fns {
            if out.contains_key(*name) {
                continue;
            }
            let locus = match f.ret.as_ref() {
                Some(TypeExpr::Named { path, .. })
                    if path.segments.len() == 1 =>
                {
                    path.segments[0].name.as_str()
                }
                _ => continue,
            };
            if !loci.contains_key(locus) {
                continue;
            }
            let mut rets: Vec<&Expr> = Vec::new();
            let mut lets: Vec<(&str, &Expr)> = Vec::new();
            if !collect_returns(&f.body, &mut rets, &mut lets) {
                continue;
            }
            let mut states: Vec<ContainmentState> = Vec::new();
            let mut fresh = !rets.is_empty();
            for r in &rets {
                match arm_states(r, locus, &lets, &out) {
                    Some(s) => states.extend(s),
                    None => {
                        fresh = false;
                        break;
                    }
                }
            }
            if !fresh || states.is_empty() {
                continue;
            }
            states.sort();
            states.dedup();
            out.insert((*name).to_string(), states);
            added = true;
        }
        if !added {
            break;
        }
    }
    out
}

/// F.31 Phase 5: pool identity. Each main-locus params field
/// gets one of these via the placement block (or default to
/// `Cooperative("main")`). Nested loci inherit the parent
/// tower's pool.
///
/// `Cooperative` pools are name-scoped — two loci on
/// `cooperative(pool = "io")` share an OS thread. `Pinned`
/// pools are uniquely identified by the owning field path
/// (each pinned locus spawns its own pthread, so two pinned
/// siblings — even of the same locus type — live on different
/// threads).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PoolId {
    Cooperative(String),
    /// Field-path string of the pinned locus's instantiation
    /// site, e.g. `"heartbeat"` for `main.heartbeat: pinned`.
    /// Uniqueness across pinned instances is the load-bearing
    /// property; the string is for diagnostics.
    Pinned(String),
}

impl PoolId {
    pub fn display(&self) -> String {
        match self {
            PoolId::Cooperative(name) => {
                format!("cooperative(pool = {})", name)
            }
            PoolId::Pinned(path) => format!("pinned (at `{}`)", path),
        }
    }
}

/// F.31 Phase 5 entry. Builds the per-locus-type pool map from
/// main's placement block, then walks every locus method body in
/// the bundle and flags direct `recv.foo(args)` calls whose
/// receiver resolves to a field of a locus type with a different
/// pool than the enclosing method's locus.
/// FUv0.8.2 #4 (2026-05-25): F.31 pool propagation extracted
/// as a pub helper so callers outside this module (the
/// `apply_sync_inference` finalization pass that runs before
/// codegen) can re-derive the map without re-running typecheck.
///
/// Seeds from the main locus's `placement { }` block, then
/// propagates the pool to each nested locus-typed param field.
/// First-wins on conflict — a single locus type appearing in
/// two towers with different pools is rare in v1; we pick the
/// first.
///
/// Returns an empty map for programs without a main locus
/// (free-fn-main scripts), so callers can skip the rest of
/// the analysis cheaply.
pub fn compute_pool_of_locus_type(
    bundle: &Bundle<'_>,
    top: &TopScope,
) -> BTreeMap<String, PoolId> {
    // GH #825: `main locus` inside a `module { … }` is still the
    // program's main locus — the resolver keys `TopScope` by the bare
    // name and codegen finds it the same way. A lookup that stops at
    // the top level returns an EMPTY map for such a program, and
    // every caller reads an empty map as "no placement to reason
    // about" and returns early: the whole F.31 layer switched off.
    let mut main_locus: Option<&LocusDecl> = None;
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                if l.is_main {
                    main_locus = Some(l);
                }
            }
        });
    }
    let Some(main) = main_locus else {
        return BTreeMap::new();
    };

    let placement_block = main.members.iter().find_map(|m| match m {
        LocusMember::Placement(pb) => Some(pb),
        _ => None,
    });
    let placement_map: BTreeMap<String, PoolId> = placement_block
        .map(|pb| {
            pb.entries
                .iter()
                .map(|e| {
                    (
                        e.field.name.clone(),
                        placement_spec_to_pool(&e.spec, &e.field.name),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let mut pool_of_locus_type: BTreeMap<String, PoolId> = BTreeMap::new();
    pool_of_locus_type.insert(
        main.name.name.clone(),
        PoolId::Cooperative("main".to_string()),
    );
    let main_params = main.members.iter().find_map(|m| match m {
        LocusMember::Params(pb) => Some(pb),
        _ => None,
    });
    if let Some(params) = main_params {
        for p in &params.params {
            let pool = placement_map
                .get(&p.name.name)
                .cloned()
                .unwrap_or_else(|| {
                    PoolId::Cooperative("main".to_string())
                });
            if let Some(ty) = &p.ty {
                if let Some(locus_name) = type_expr_locus_name(ty, top) {
                    pool_of_locus_type
                        .entry(locus_name.clone())
                        .or_insert_with(|| pool.clone());
                    walk_nested_loci(
                        &locus_name,
                        &pool,
                        top,
                        &mut pool_of_locus_type,
                    );
                }
            }
        }
    }
    pool_of_locus_type
}

/// Pool affinity (2026-08-12): validity of `cooperative(pool = X,
/// core/cores/node/l3 = ...)` placement entries.
///
/// * Affinity with no pool (or pool `main`) is rejected — the main
///   pool is the program's main thread; binding it belongs to the
///   operator (taskset), not a placement entry.
/// * Two entries naming ONE pool with two different affinities is a
///   contradiction: the pool has one worker thread. An entry that
///   names the pool without an affinity is compatible with any.
fn check_pool_affinity(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    // GH #825: `main locus` inside a `module { … }` declares the same
    // placement block, and an affinity with no named pool is just as
    // meaningless there.
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(l) = item else { return };
            if !l.is_main {
                return;
            }
            let mut declared: BTreeMap<String, (PinAffinity, Span)> =
                BTreeMap::new();
            for m in &l.members {
                let LocusMember::Placement(pb) = m else { continue };
                for entry in &pb.entries {
                    let PlacementSpec::Cooperative { pool, affinity } =
                        &entry.spec
                    else {
                        continue;
                    };
                    if matches!(affinity, PinAffinity::Any) {
                        continue;
                    }
                    let pool_name = match pool {
                        Some(p) if p.name != "main" => p.name.clone(),
                        _ => {
                            diags.push(Diag::ty(
                                entry.span,
                                "cooperative affinity needs a named pool \
                                 (`cooperative(pool = X, cores = ...)`) — \
                                 the main pool is the program's main \
                                 thread, whose affinity belongs to the \
                                 operator, not a placement entry",
                            ));
                            continue;
                        }
                    };
                    match declared.get(&pool_name) {
                        None => {
                            declared.insert(
                                pool_name,
                                (affinity.clone(), entry.span),
                            );
                        }
                        Some((prev, _)) if prev == affinity => {}
                        Some((_, prev_span)) => {
                            diags.push(
                                Diag::ty(
                                    entry.span,
                                    format!(
                                        "pool `{}` is given two different \
                                         affinities — a pool has ONE worker \
                                         thread; declare the affinity on \
                                         one entry (the others inherit it)",
                                        pool_name
                                    ),
                                )
                                .with_related(
                                    *prev_span,
                                    "first affinity declared here",
                                ),
                            );
                        }
                    }
                }
            }
        });
    }
}

fn check_placement_single_thread(
    bundle: &Bundle<'_>,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    let pool_of_locus_type = compute_pool_of_locus_type(bundle, top);
    if pool_of_locus_type.is_empty() {
        return;
    }
    // The main locus is needed downstream for the cross-pool
    // walk's `enclosing_locus`; re-locate it (cheap).
    let mut main_locus: Option<&LocusDecl> = None;
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                if l.is_main {
                    main_locus = Some(l);
                }
            }
        });
    }
    let _main = main_locus;

    // 4. Walk every locus method body in the bundle and emit
    //    diagnostics for direct cross-pool calls. The check
    //    only flags `recv.foo(args)` shapes where `recv` is a
    //    field-access expression whose declared type names a
    //    locus with a known pool. Local-variable receivers,
    //    deeply-chained receivers, and stdlib/free-fn calls all
    //    fall back to OK (they need richer flow analysis we
    //    defer to v1.x).
    // F.32-0 (2026-05-24): collect locus types whose state is
    // held in `@form(...)` cells AND that carry an explicit
    // `sync = X` kwarg with X != `none`. The cross-pool
    // diagnostic is skipped only for these.
    //
    // History: 3ec6391 (2026-05-24, first cut) admitted any
    // `@form(...)` locus into this set on the assumption that
    // the form ABI serialized cell access. Bench-prep for
    // F.32-1 found the runtime (`lotus_arena.c:1869+`) has no
    // synchronization on `lotus_hashmap_set` / `_grow` — two
    // writers double-free during concurrent grow. F.32-0
    // scopes the exemption to explicitly-opted-in loci; the
    // sync disciplines themselves (α/β/γ) land in F.32-1.
    //
    // `form_bearing_loci` is a wider set (every @form locus,
    // sync or no sync) used only to tailor the diagnostic's
    // upgrade hint — receivers in this set get a "declare
    // `sync = ...` to opt in" suggestion.
    let mut cross_pool_safe_loci: BTreeSet<String> = BTreeSet::new();
    let mut form_bearing_loci: BTreeSet<String> = BTreeSet::new();
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                if let Some(form) = &l.form {
                    form_bearing_loci.insert(l.name.name.clone());
                    if form_has_explicit_sync_discipline(form) {
                        cross_pool_safe_loci.insert(l.name.name.clone());
                    }
                }
            }
        });
    }

    // F.32-1∞ (2026-05-25): pre-compute sync inference for
    // every `@form(hashmap)` locus without explicit `sync = `.
    // The F.32-0 diagnostic below consults this map to name
    // the specific discipline the rule would pick, instead of
    // suggesting a generic "choose one of serialized/striped".
    let inferred_sync = crate::sync_inference::infer_sync_for_bundle(
        bundle,
        top,
        &pool_of_locus_type,
    );

    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                let caller_pool = pool_of_locus_type.get(&l.name.name);
                for member in &l.members {
                    if let Some(body) = locus_member_body(member) {
                        let mut cx = PoolCheckCx {
                            enclosing_locus: l,
                            caller_pool,
                            cross_pool_safe_loci: &cross_pool_safe_loci,
                            form_bearing_loci: &form_bearing_loci,
                            inferred_sync: &inferred_sync,
                            top,
                            diags,
                        };
                        walk_block_pool(body, &mut cx);
                    }
                }
            }
        });
    }
}

/// GH #826: a locus whose `placement { }` block pins a field cannot
/// be instantiated inside a loop.
///
/// `pinned` is the placement class that gives a field its OWN OS
/// thread, spawned in the enclosing locus's params-init and joined
/// at the instantiating scope's exit. Both halves of that bookkeeping
/// — the deferred-dissolve slot and the `pthread_t` it joins — are
/// ONE alloca per instantiation SITE, so a site reached a second time
/// overwrites the record of the first: at scope exit only the LAST
/// instance is joined and arena-destroyed, and every earlier pinned
/// thread is orphaned with its arena still live (GH #815's per-
/// iteration slot reclaim deliberately stepped over the pinned entry,
/// because reclaiming it means joining the previous thread).
///
/// `placement { }` is `main locus`-only (rule 1), so the reachable
/// shape is the main locus itself instantiated inside a loop —
/// the deployment root booted once per iteration. That is a category
/// error against the model placement describes: entries name static
/// resources (a core, a NUMA node, `replicas = K`), one thread per
/// entry for the program's life. Rejecting it is rule 17 rather than
/// a per-iteration join because a per-iteration OS thread is never
/// the intent, and because today's behaviour turns on an invisible
/// internal path — a main locus with a bus subscription takes the
/// deferred teardown and leaks, one without takes the eager path and
/// happens to be clean. A rule that fires on only one of those is
/// worse than one that rejects the shape.
///
/// Scope: the check is positional — it flags the locus LITERAL where
/// it stands, in any fn, locus method, or lifecycle hook, at any loop
/// nesting depth. A factory called in a loop (`fn boot() { App { }; }`)
/// is not flagged and does not leak: the literal is not in a loop, so
/// the pinned entry is flushed at the factory's own fn exit and every
/// call joins its own thread. Hoisting the literal out of the loop —
/// or behind a fn the loop calls — is the fix in both directions.
fn check_pinned_locus_in_loop(
    bundle: &Bundle<'_>,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    // Loci that pin at least one field. Mirrors codegen's
    // `collect_main_placement`: an imported seed's main locus is
    // renamed `__lib_*` and is NOT the deployment root, so its
    // placement entries never reach the plan and never spawn a
    // thread — flagging it would be a false positive.
    let mut pinned_by: BTreeMap<String, (String, Span)> = BTreeMap::new();
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(l) = item else { return };
            if !l.is_main || l.name.name.starts_with("__lib_") {
                return;
            }
            for m in &l.members {
                let LocusMember::Placement(pb) = m else { continue };
                for entry in &pb.entries {
                    if matches!(entry.spec, PlacementSpec::Pinned { .. }) {
                        pinned_by
                            .entry(l.name.name.clone())
                            .or_insert_with(|| {
                                (entry.field.name.clone(), entry.span)
                            });
                    }
                }
            }
        });
    }
    if pinned_by.is_empty() {
        return;
    }
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let mut cx = PinnedLoopCx {
                top,
                pinned_by: &pinned_by,
                diags: &mut *diags,
                loop_depth: 0,
            };
            match item {
                TopDecl::Fn(fd) => pinned_walk_block(&fd.body, &mut cx),
                TopDecl::Locus(l) => {
                    for member in &l.members {
                        if let Some(body) = locus_member_body(member) {
                            pinned_walk_block(body, &mut cx);
                        }
                    }
                }
                _ => {}
            }
        });
    }
}

struct PinnedLoopCx<'a> {
    top: &'a TopScope,
    /// locus name → (the first field it pins, that entry's span).
    pinned_by: &'a BTreeMap<String, (String, Span)>,
    diags: &'a mut Vec<Diag>,
    loop_depth: u32,
}

fn pinned_walk_block(b: &Block, cx: &mut PinnedLoopCx) {
    for s in &b.stmts {
        pinned_walk_stmt(s, cx);
    }
    if let Some(t) = &b.tail {
        pinned_walk_expr(t, cx);
    }
}

fn pinned_walk_if(i: &IfStmt, cx: &mut PinnedLoopCx) {
    pinned_walk_expr(&i.cond, cx);
    pinned_walk_block(&i.then_block, cx);
    if let Some(eb) = &i.else_block {
        match eb.as_ref() {
            ElseBranch::Else(b) => pinned_walk_block(b, cx),
            ElseBranch::ElseIf(i2) => pinned_walk_if(i2, cx),
        }
    }
}

fn pinned_walk_match(m: &MatchStmt, cx: &mut PinnedLoopCx) {
    pinned_walk_expr(&m.scrutinee, cx);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            pinned_walk_expr(g, cx);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => pinned_walk_expr(e, cx),
            MatchArmBody::Block(b) => pinned_walk_block(b, cx),
        }
    }
}

fn pinned_walk_stmt(s: &Stmt, cx: &mut PinnedLoopCx) {
    match s {
        Stmt::While { cond, body, .. } => {
            pinned_walk_expr(cond, cx);
            cx.loop_depth += 1;
            pinned_walk_block(body, cx);
            cx.loop_depth -= 1;
        }
        Stmt::For { iter, body, .. } => {
            pinned_walk_expr(iter, cx);
            cx.loop_depth += 1;
            pinned_walk_block(body, cx);
            cx.loop_depth -= 1;
        }
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            pinned_walk_expr(value, cx)
        }
        Stmt::Assign { value, .. } => pinned_walk_expr(value, cx),
        Stmt::If(i) => pinned_walk_if(i, cx),
        Stmt::Match(m) => pinned_walk_match(m, cx),
        Stmt::Return(Some(e), _) => pinned_walk_expr(e, cx),
        Stmt::Fail { value, .. } => pinned_walk_expr(value, cx),
        Stmt::Expr(e) => pinned_walk_expr(e, cx),
        _ => {}
    }
}

fn pinned_walk_expr(e: &Expr, cx: &mut PinnedLoopCx) {
    match e {
        Expr::Struct { path, inits, span, .. } => {
            for init in inits {
                pinned_walk_expr(&init.value, cx);
            }
            if cx.loop_depth == 0 {
                return;
            }
            // A `placement { }` block lives on the bundle's own main
            // locus, which is never reached through an import alias
            // (an imported main is renamed `__lib_*` and filtered
            // above), so a single-segment name is the whole surface.
            let segs: Vec<&str> =
                path.segments.iter().map(|s| s.name.as_str()).collect();
            if segs.len() != 1 {
                return;
            }
            if !matches!(cx.top.lookup(segs[0]), Some(TopSymbol::Locus(_))) {
                return;
            }
            let Some((field, entry_span)) = cx.pinned_by.get(segs[0]) else {
                return;
            };
            cx.diags.push(
                Diag::ty(
                    *span,
                    format!(
                        "locus `{}` is instantiated inside a loop, but its \
                         `placement {{ }}` block pins field `{}` to its own \
                         OS thread. Every iteration spawns a fresh pinned \
                         thread while only the last one is joined, so the \
                         earlier threads are orphaned and their arenas leak. \
                         Placement names static resources (a core, a NUMA \
                         node, `replicas = K`) — one thread per entry for \
                         the program's life — so instantiate `{}` once, \
                         outside the loop. (A loop that calls a fn holding \
                         the literal is fine: each call joins its own \
                         thread.)",
                        segs[0], field, segs[0]
                    ),
                )
                .with_related(
                    *entry_span,
                    format!("field `{}` is placed `pinned` here", field),
                ),
            );
        }
        Expr::Call { callee, args, .. } => {
            pinned_walk_expr(callee, cx);
            for a in args {
                pinned_walk_expr(a, cx);
            }
        }
        Expr::Binary { left, right, .. } => {
            pinned_walk_expr(left, cx);
            pinned_walk_expr(right, cx);
        }
        Expr::Unary { operand, .. } => pinned_walk_expr(operand, cx),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            pinned_walk_expr(receiver, cx)
        }
        Expr::Index { receiver, index, .. } => {
            pinned_walk_expr(receiver, cx);
            pinned_walk_expr(index, cx);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for e in es {
                pinned_walk_expr(e, cx);
            }
        }
        Expr::Sum(e, _) | Expr::Prod(e, _) => pinned_walk_expr(e, cx),
        Expr::Approx { left, right, tolerance, .. } => {
            pinned_walk_expr(left, cx);
            pinned_walk_expr(right, cx);
            pinned_walk_expr(tolerance, cx);
        }
        Expr::Range { lo, hi, .. } => {
            pinned_walk_expr(lo, cx);
            pinned_walk_expr(hi, cx);
        }
        Expr::ArrayRepeat { val, .. } => pinned_walk_expr(val, cx),
        Expr::Block(b) => pinned_walk_block(b, cx),
        Expr::If(i) => pinned_walk_if(i, cx),
        Expr::Match(m) => pinned_walk_match(m, cx),
        Expr::Or { inner, disposition, .. } => {
            pinned_walk_expr(inner, cx);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    pinned_walk_expr(e, cx)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// GH #890: a `placement { }` entry no instantiation consumes.
///
/// A placement entry is carried to codegen as an override on the
/// NEXT locus instantiation lowered for that field
/// (`placement_for_next_locus_instantiation`, plus the parallel pool
/// and NUMA-node overrides). A locus LITERAL takes it; nothing else
/// does. So a field whose value arrives any other way — a factory
/// call, a fallible call, a conditional, a reference to an instance
/// somebody else built — leaves the override untaken, and the next
/// field's turn through the params-init loop resets it. No thread is
/// spawned, no pool is joined, and nothing is said: the entry the
/// author wrote is silently dropped.
///
/// Applying the entry after the fact is not available. The pinned
/// path is not "mark this instance pinned" but "spawn a pthread that
/// runs the whole lifecycle — birth, run, the mailbox loop, drain,
/// dissolve — on it", and a factory's literal has already run birth
/// and run() (and registered its subscriptions against the global
/// queue) before the value returns. There is nothing left to place.
/// So the entry is refused at its source instead, pointing at the
/// literal form that does carry it.
///
/// The rule is the backstop the issue asks for rather than a
/// factory-shaped special case: EVERY entry must be consumed by
/// exactly one instantiation. The value the entry places is the
/// instantiation-site init when the literal supplies one and the
/// params default otherwise, so both spellings are checked — and a
/// default every site overrides is dead text, not a dropped
/// placement.
///
/// Scope mirrors `collect_main_placement` (and rule 17's check): an
/// imported seed's main locus is renamed `__lib_*` and is not the
/// deployment root, so its entries never reach the plan and flagging
/// them would be a false positive.
fn check_placement_entry_consumed(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    let mut main: Option<&LocusDecl> = None;
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                if l.is_main && !l.name.name.starts_with("__lib_") {
                    main = Some(l);
                }
            }
        });
    }
    let Some(main) = main else { return };
    let Some(pb) = main.members.iter().find_map(|m| match m {
        LocusMember::Placement(pb) => Some(pb),
        _ => None,
    }) else {
        return;
    };
    let Some(params) = main.members.iter().find_map(|m| match m {
        LocusMember::Params(p) => Some(p),
        _ => None,
    }) else {
        return;
    };

    // Every instantiation of the main locus in the bundle, as the
    // field inits it writes. `fn main() { App { }; }` is the usual
    // one and supplies nothing, but a params field declared without a
    // default is supplied here, and that init is what carries the
    // placement.
    let mut cx = PlacementSiteCx {
        main: main.name.name.as_str(),
        sites: Vec::new(),
    };
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| match item {
            TopDecl::Fn(fd) => placement_site_walk_block(&fd.body, &mut cx),
            TopDecl::Locus(l) => {
                for member in &l.members {
                    if let Some(body) = locus_member_body(member) {
                        placement_site_walk_block(body, &mut cx);
                    }
                }
            }
            _ => {}
        });
    }
    let sites = cx.sites;

    for entry in &pb.entries {
        let field = entry.field.name.as_str();
        // Unknown / non-locus fields are `check_placement_block`'s to
        // report; saying it twice helps nobody.
        let Some(param) = params.params.iter().find(|p| p.name.name == field)
        else {
            continue;
        };
        // The literal form to suggest: the declared type as written
        // (a stdlib locus is a qualified path, and its literal is
        // spelled the same way), or `T` when the type is inferred
        // from the default and there is nothing to quote.
        let ty_name = match &param.ty {
            Some(TypeExpr::Named { path, .. })
                if !path.segments.is_empty() =>
            {
                path.segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join("::")
            }
            _ => "T".to_string(),
        };

        // The site inits for this field, and whether any site leaves
        // the field to its default.
        let mut overrides: Vec<&Expr> = Vec::new();
        let mut any_site_takes_default = false;
        for inits in &sites {
            match inits.iter().find(|i| i.name.name == field) {
                Some(init) => overrides.push(&init.value),
                None => any_site_takes_default = true,
            }
        }
        for init in overrides {
            if matches!(init, Expr::Struct { .. }) {
                continue;
            }
            diags.push(placement_unconsumed_diag(
                field,
                &ty_name,
                init,
                entry.span,
                true,
            ));
        }
        // The default is live when some site omits the field — and
        // when the bundle instantiates the main locus nowhere at all
        // (a library seed checked on its own: the default is the only
        // initialiser there is).
        if !(any_site_takes_default || sites.is_empty()) {
            continue;
        }
        let ParamInit::Value(default) = &param.init else {
            // No default and no site init: the missing-required-param
            // rule owns that program, not this one.
            continue;
        };
        if matches!(default, Expr::Struct { .. }) {
            continue;
        }
        diags.push(placement_unconsumed_diag(
            field,
            &ty_name,
            default,
            entry.span,
            false,
        ));
    }
}

/// The GH #890 diagnostic, for an initialiser written at an
/// instantiation site (`at_site`) or in the params default.
fn placement_unconsumed_diag(
    field: &str,
    ty_name: &str,
    init: &Expr,
    entry_span: Span,
    at_site: bool,
) -> Diag {
    let shape = match init {
        Expr::Call { .. } => "a call",
        Expr::Or { .. } => "a fallible call",
        Expr::If(_) | Expr::Match(_) => "a conditional",
        Expr::Ident(_) | Expr::Path(_) | Expr::Field { .. }
        | Expr::Path2 { .. } | Expr::Index { .. } | Expr::KwSelf(_) => {
            "a reference to an instance built elsewhere"
        }
        _ => "an expression that is not a locus literal",
    };
    Diag::ty(
        init.span(),
        format!(
            "placement entry `{}` names a field no locus literal \
             initialises: {} is {}. A placement is carried by the locus \
             LITERAL lowered for the field — a factory's literal is \
             lowered inside the factory, out of this entry's reach — so \
             the entry would be silently dropped and `{}` would run \
             wherever an unplaced field runs. Write the literal {} \
             (`{}`), and move the factory's other work into the locus's \
             own params or `birth()`.",
            field,
            if at_site {
                format!("the value supplied for `{}` here", field)
            } else {
                format!("`{}`'s default", field)
            },
            shape,
            field,
            if at_site { "at this site" } else { "in the field" },
            if at_site {
                format!("{}: {} {{ }}", field, ty_name)
            } else {
                format!("{}: {} = {} {{ }};", field, ty_name, ty_name)
            },
        ),
    )
    .with_related(entry_span, format!("`{}` is placed here", field))
}

/// Collects the field inits of every literal of the main locus.
/// Same AST coverage `pinned_walk_*` has, without rule 17's loop
/// bookkeeping — a placement site is positional in neither sense.
struct PlacementSiteCx<'a> {
    /// The bundle's main locus name. `placement { }` is main-only
    /// (rule 1) and an imported main is renamed `__lib_*`, so a
    /// single-segment name is the whole surface — the same reasoning
    /// rule 17's walk uses.
    main: &'a str,
    sites: Vec<&'a [StructInit]>,
}

fn placement_site_walk_block<'a>(
    b: &'a Block,
    cx: &mut PlacementSiteCx<'a>,
) {
    for s in &b.stmts {
        placement_site_walk_stmt(s, cx);
    }
    if let Some(t) = &b.tail {
        placement_site_walk_expr(t, cx);
    }
}

fn placement_site_walk_if<'a>(i: &'a IfStmt, cx: &mut PlacementSiteCx<'a>) {
    placement_site_walk_expr(&i.cond, cx);
    placement_site_walk_block(&i.then_block, cx);
    if let Some(eb) = &i.else_block {
        match eb.as_ref() {
            ElseBranch::Else(b) => placement_site_walk_block(b, cx),
            ElseBranch::ElseIf(i2) => placement_site_walk_if(i2, cx),
        }
    }
}

fn placement_site_walk_match<'a>(
    m: &'a MatchStmt,
    cx: &mut PlacementSiteCx<'a>,
) {
    placement_site_walk_expr(&m.scrutinee, cx);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            placement_site_walk_expr(g, cx);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => placement_site_walk_expr(e, cx),
            MatchArmBody::Block(b) => placement_site_walk_block(b, cx),
        }
    }
}

fn placement_site_walk_stmt<'a>(s: &'a Stmt, cx: &mut PlacementSiteCx<'a>) {
    match s {
        Stmt::While { cond, body, .. } => {
            placement_site_walk_expr(cond, cx);
            placement_site_walk_block(body, cx);
        }
        Stmt::For { iter, body, .. } => {
            placement_site_walk_expr(iter, cx);
            placement_site_walk_block(body, cx);
        }
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            placement_site_walk_expr(value, cx)
        }
        Stmt::Assign { value, .. } => placement_site_walk_expr(value, cx),
        Stmt::If(i) => placement_site_walk_if(i, cx),
        Stmt::Match(m) => placement_site_walk_match(m, cx),
        Stmt::Return(Some(e), _) => placement_site_walk_expr(e, cx),
        Stmt::Fail { value, .. } => placement_site_walk_expr(value, cx),
        Stmt::Expr(e) => placement_site_walk_expr(e, cx),
        _ => {}
    }
}

fn placement_site_walk_expr<'a>(e: &'a Expr, cx: &mut PlacementSiteCx<'a>) {
    match e {
        Expr::Struct { path, inits, .. } => {
            let segs: Vec<&str> =
                path.segments.iter().map(|s| s.name.as_str()).collect();
            if segs.len() == 1 && segs[0] == cx.main {
                cx.sites.push(inits.as_slice());
            }
            for init in inits {
                placement_site_walk_expr(&init.value, cx);
            }
        }
        Expr::Call { callee, args, .. } => {
            placement_site_walk_expr(callee, cx);
            for a in args {
                placement_site_walk_expr(a, cx);
            }
        }
        Expr::Binary { left, right, .. } => {
            placement_site_walk_expr(left, cx);
            placement_site_walk_expr(right, cx);
        }
        Expr::Unary { operand, .. } => placement_site_walk_expr(operand, cx),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            placement_site_walk_expr(receiver, cx)
        }
        Expr::Index { receiver, index, .. } => {
            placement_site_walk_expr(receiver, cx);
            placement_site_walk_expr(index, cx);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for e in es {
                placement_site_walk_expr(e, cx);
            }
        }
        Expr::Sum(e, _) | Expr::Prod(e, _) => placement_site_walk_expr(e, cx),
        Expr::Approx { left, right, tolerance, .. } => {
            placement_site_walk_expr(left, cx);
            placement_site_walk_expr(right, cx);
            placement_site_walk_expr(tolerance, cx);
        }
        Expr::Range { lo, hi, .. } => {
            placement_site_walk_expr(lo, cx);
            placement_site_walk_expr(hi, cx);
        }
        Expr::ArrayRepeat { val, .. } => placement_site_walk_expr(val, cx),
        Expr::Block(b) => placement_site_walk_block(b, cx),
        Expr::If(i) => placement_site_walk_if(i, cx),
        Expr::Match(m) => placement_site_walk_match(m, cx),
        Expr::Or { inner, disposition, .. } => {
            placement_site_walk_expr(inner, cx);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    placement_site_walk_expr(e, cx)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn placement_spec_to_pool(
    spec: &hale_syntax::ast::PlacementSpec,
    field_name: &str,
) -> PoolId {
    use hale_syntax::ast::PlacementSpec;
    match spec {
        PlacementSpec::Cooperative { pool, .. } => {
            let name = pool
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "main".to_string());
            PoolId::Cooperative(name)
        }
        PlacementSpec::Pinned { .. } => {
            PoolId::Pinned(field_name.to_string())
        }
    }
}

/// Resolve a type expression to a locus name, if the type
/// resolves to a `TopSymbol::Locus`. Returns `None` for
/// non-locus types or unresolved names.
fn type_expr_locus_name(ty: &TypeExpr, top: &TopScope) -> Option<String> {
    let TypeExpr::Named { path, .. } = ty else {
        return None;
    };
    if path.segments.len() != 1 {
        return None;
    }
    let name = &path.segments[0].name;
    match top.lookup(name) {
        Some(TopSymbol::Locus(_)) => Some(name.clone()),
        _ => None,
    }
}

/// Walk a locus type's params block transitively, propagating
/// the tower's pool to each nested locus-typed field. First-wins
/// on conflict.
fn walk_nested_loci(
    locus_name: &str,
    pool: &PoolId,
    top: &TopScope,
    map: &mut BTreeMap<String, PoolId>,
) {
    // We need the original LocusDecl to walk its params. The
    // bundle isn't threaded through here; instead use the
    // resolved LocusInfo's params from `top`. LocusInfo carries
    // Param `Ty` already resolved, so we walk those.
    let info = match top.lookup(locus_name) {
        Some(TopSymbol::Locus(l)) => l,
        _ => return,
    };
    for p in &info.params {
        let nested = match &p.ty {
            Ty::Named(n) => match top.lookup(n) {
                Some(TopSymbol::Locus(_)) => Some(n.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(child) = nested {
            // First-wins: if already assigned, skip to avoid
            // cycles + multi-tower conflicts.
            if !map.contains_key(&child) {
                map.insert(child.clone(), pool.clone());
                walk_nested_loci(&child, pool, top, map);
            }
        }
    }
}

/// Return the body block of a locus member that carries one
/// (lifecycle, on_failure, fn, mode). Anything else (params,
/// bus, closure decl, etc.) returns None.
fn locus_member_body(member: &LocusMember) -> Option<&Block> {
    match member {
        LocusMember::Lifecycle(lc) => Some(&lc.body),
        LocusMember::Failure(fd) => Some(&fd.body),
        LocusMember::Fn(fd) => Some(&fd.body),
        LocusMember::Mode(md) => Some(&md.body),
        _ => None,
    }
}

/// F.32-0 (2026-05-24): true when a form annotation carries an
/// explicit `sync = X` kwarg where X names a recognized sync
/// discipline (`serialized`, `striped`, or `lockfree`). The
/// cross-pool exemption applies only to such loci — the
/// substrate's runtime gives no thread-safety to plain
/// `@form(...)` cells (the 3ec6391 commit's "form ABI
/// serializes" claim was aspirational; see
/// `notes/f32-cache-aware-delivery-plan.md` § F.32-0).
///
/// Unknown / malformed `sync = X` values return false here
/// (so the cross-pool diagnostic still fires). F.32-1α/β2
/// validates the recognized values; `lockfree` (γ) is in the
/// accept set syntactically but the per-locus check rejects
/// it as deferred. This helper only gates the cross-pool
/// exemption — codegen does its own mapping to SyncMode.
fn form_has_explicit_sync_discipline(form: &FormAnnotation) -> bool {
    form.args.iter().any(|arg| {
        if arg.name.name != "sync" {
            return false;
        }
        match &arg.value {
            Expr::Ident(i) => matches!(
                i.name.as_str(),
                "serialized" | "striped" | "lockfree"
            ),
            _ => false,
        }
    })
}

/// Visitor context for the cross-pool call walk. Carried by
/// reference so the recursive Stmt/Expr traversal doesn't pay
/// a closure-capture allocation per node.
struct PoolCheckCx<'a> {
    enclosing_locus: &'a LocusDecl,
    caller_pool: Option<&'a PoolId>,
    /// F.32-0 (2026-05-24): locus type names that opt in to
    /// cross-pool access by declaring `@form(<name>, sync = X)`
    /// where X is a recognized discipline (`serialized` /
    /// `striped` / `lockfree`; F.32-1α/β/γ). Cross-pool method
    /// calls into receivers landing in this set skip the
    /// diagnostic — the chosen sync discipline carries the
    /// substrate's safety contract.
    ///
    /// Plain `@form(hashmap)` / `@form(vec)` / `@form(ring_buffer)`
    /// (no sync kwarg) does NOT land in this set: the runtime
    /// has no synchronization on those paths and concurrent
    /// writers corrupt the structure (`lotus_arena.c:1869+` —
    /// `lotus_hashmap_set` / `_grow` are non-atomic single-
    /// threaded code).
    cross_pool_safe_loci: &'a BTreeSet<String>,
    /// Wider companion to `cross_pool_safe_loci`: every locus
    /// type carrying any `@form(...)` annotation (with or
    /// without a sync kwarg). Used only to specialize the
    /// cross-pool diagnostic — receivers in this set get a
    /// "declare `sync = ...` to opt in" upgrade hint.
    form_bearing_loci: &'a BTreeSet<String>,
    /// F.32-1∞ (2026-05-25): sync-inference results keyed by
    /// locus type name. Present only for `@form(hashmap)`
    /// loci without explicit `sync = `. The cross-pool
    /// diagnostic reads this to name the specific discipline
    /// the rule picks (so the upgrade hint is actionable, not
    /// generic).
    inferred_sync: &'a BTreeMap<
        String,
        crate::sync_inference::InferredSync,
    >,
    top: &'a TopScope,
    diags: &'a mut Vec<Diag>,
}

fn walk_block_pool(block: &Block, cx: &mut PoolCheckCx) {
    for stmt in &block.stmts {
        walk_stmt_pool(stmt, cx);
    }
    if let Some(tail) = &block.tail {
        walk_expr_pool(tail, cx);
    }
}

fn walk_stmt_pool(stmt: &Stmt, cx: &mut PoolCheckCx) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            walk_expr_pool(value, cx);
        }
        Stmt::Assign { value, .. } => walk_expr_pool(value, cx),
        Stmt::If(i) => walk_if_pool(i, cx),
        Stmt::Match(m) => walk_match_pool(m, cx),
        Stmt::For { iter, body, .. } => {
            walk_expr_pool(iter, cx);
            walk_block_pool(body, cx);
        }
        Stmt::While { cond, body, .. } => {
            walk_expr_pool(cond, cx);
            walk_block_pool(body, cx);
        }
        Stmt::Return(opt, _) => {
            if let Some(e) = opt {
                walk_expr_pool(e, cx);
            }
        }
        Stmt::Fail { value, .. } => walk_expr_pool(value, cx),
        Stmt::Block(b) => walk_block_pool(b, cx),
        Stmt::ShmWrite { max, body, .. } => {
            walk_expr_pool(max, cx);
            walk_block_pool(body, cx);
        }
        Stmt::Recovery { args, .. } => {
            for a in args {
                walk_expr_pool(a, cx);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(p) = payload {
                walk_expr_pool(p, cx);
            }
        }
        Stmt::Send { subject, value, .. } => {
            walk_expr_pool(subject, cx);
            walk_expr_pool(value, cx);
        }
        Stmt::Expr(e) => walk_expr_pool(e, cx),
        Stmt::Yield(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Terminate(_)
        | Stmt::Reperspective { .. } => {}
    }
}

fn walk_if_pool(stmt: &IfStmt, cx: &mut PoolCheckCx) {
    walk_expr_pool(&stmt.cond, cx);
    walk_block_pool(&stmt.then_block, cx);
    if let Some(eb) = &stmt.else_block {
        match eb.as_ref() {
            ElseBranch::Else(b) => walk_block_pool(b, cx),
            ElseBranch::ElseIf(nested) => walk_if_pool(nested, cx),
        }
    }
}

fn walk_match_pool(stmt: &MatchStmt, cx: &mut PoolCheckCx) {
    walk_expr_pool(&stmt.scrutinee, cx);
    for arm in &stmt.arms {
        if let Some(g) = &arm.guard {
            walk_expr_pool(g, cx);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => walk_expr_pool(e, cx),
            MatchArmBody::Block(b) => walk_block_pool(b, cx),
        }
    }
}

fn walk_expr_pool(expr: &Expr, cx: &mut PoolCheckCx) {
    if let Expr::Call { callee, args, span, .. } = expr {
        // F.31 Phase 5: flag `self.X.foo(args)` where the
        // field X's locus type is on a different pool than
        // the enclosing locus. Only the `Field` callee
        // shape is checked; `Path2`-style stdlib/free-fn
        // calls are pool-neutral (per spec).
        if let Expr::Field { receiver, name: method, .. } = callee.as_ref() {
            if let Some(field_locus) = receiver_field_locus_type(
                receiver,
                cx.enclosing_locus,
                cx.top,
            ) {
                // F.31 (downstream handoff 2026-07-15): the receiver's
                // pool is the pool of THIS field INSTANCE, inferred at
                // the call site — not a type-global property. A locus
                // type used as a plain field in two loci on two pools
                // yields two independent instances, one co-located with
                // each owner; `pool_of_locus_type` collapses the type to
                // a single (first-seen) pool and would false-flag every
                // other owner's own `self.<field>` call (two separate
                // `@form` maps each touched by a single pool need no sync
                // — flagging them was never sound). Compute it
                // owner-relative: the enclosing locus's OWN placement of
                // the field if it names one (e.g. `db: pinned` on the
                // main locus — a genuine off-owner cross-pool access),
                // else the field co-locates with its owner (the caller's
                // pool → same pool → not flagged).
                let field_name = match receiver.as_ref() {
                    Expr::Field { name, .. } => name.name.clone(),
                    _ => String::new(),
                };
                let instance_pool: Option<PoolId> =
                    cx.caller_pool.map(|caller| {
                        enclosing_field_placement(
                            cx.enclosing_locus,
                            &field_name,
                        )
                        .unwrap_or_else(|| caller.clone())
                    });
                if let (Some(callee_pool), Some(caller_pool_val)) = (
                    instance_pool.as_ref(),
                    cx.caller_pool,
                ) {
                    // F.32-0: receivers with an explicit
                    // sync discipline (`@form(..., sync = X)`,
                    // X != none) opt in to cross-pool calls;
                    // their chosen discipline carries the
                    // safety contract. Plain `@form(...)` is
                    // single-pool by default — the diagnostic
                    // fires with an upgrade hint.
                    if cx.cross_pool_safe_loci.contains(&field_locus) {
                        // skip the diagnostic
                    } else if callee_pool != caller_pool_val {
                        // F.32-1∞: prefer the inference-specific
                        // hint when it yields a non-None
                        // discipline (names the picked sync + the
                        // observed writer/reader pools). Fall
                        // back to the generic F.32-0 upgrade
                        // hint when the inference returns None
                        // (single-pool, or the offending call
                        // shape isn't one of the recognized
                        // `@form(hashmap)` methods so the walker
                        // observed no signal) or when the
                        // receiver isn't a hashmap (e.g. plain
                        // `@form(vec)`).
                        let inferred_hint = cx
                            .inferred_sync
                            .get(&field_locus)
                            .and_then(|inf| {
                                crate::sync_inference::render_inference_hint(
                                    &field_locus, inf,
                                )
                            });
                        let upgrade_hint = match inferred_hint {
                            Some(h) => h,
                            None => {
                                if cx.form_bearing_loci.contains(&field_locus) {
                                    format!(
                                        "\n  hint: receiver `{}` is `@form(...)`. \
                                         Cross-pool access requires an explicit sync \
                                         discipline:\n    \
                                         `@form(hashmap, sync = serialized)` — per-map \
                                         mutex (simplest, lowest throughput)\n    \
                                         `@form(hashmap, sync = striped)` — parallel \
                                         writers, cache-padded cells (F.32-1β)\n  \
                                         See `notes/f32-cache-aware-delivery-plan.md` \
                                         § F.32-0 / F.32-1.",
                                        field_locus,
                                    )
                                } else {
                                    String::new()
                                }
                            }
                        };
                        cx.diags.push(Diag::ty(
                            *span,
                            format!(
                                "cross-pool method call: `{}.{}` invokes a method \
                                 on locus `{}` placed `{}`, but the enclosing \
                                 locus `{}` is placed `{}`. Cross-pool \
                                 coordination must go through the bus, not a \
                                 direct call. See spec/types.md \
                                 § \"Single-threaded-method invariant (F.31)\".{}",
                                receiver_display(receiver),
                                method.name,
                                field_locus,
                                callee_pool.display(),
                                cx.enclosing_locus.name.name,
                                caller_pool_val.display(),
                                upgrade_hint,
                            ),
                        ));
                    }
                }
            }
        }
        walk_expr_pool(callee, cx);
        for a in args {
            walk_expr_pool(a, cx);
        }
        return;
    }
    match expr {
        Expr::Binary { left, right, .. } => {
            walk_expr_pool(left, cx);
            walk_expr_pool(right, cx);
        }
        Expr::Unary { operand, .. } => walk_expr_pool(operand, cx),
        Expr::Field { receiver, .. } => walk_expr_pool(receiver, cx),
        Expr::Index { receiver, index, .. } => {
            walk_expr_pool(receiver, cx);
            walk_expr_pool(index, cx);
        }
        Expr::Path2 { receiver, .. } => walk_expr_pool(receiver, cx),
        Expr::Tuple(items, _) | Expr::Array(items, _) => {
            for e in items {
                walk_expr_pool(e, cx);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                walk_expr_pool(&i.value, cx);
            }
        }
        Expr::Block(b) => walk_block_pool(b, cx),
        Expr::If(stmt) => walk_if_pool(stmt, cx),
        Expr::Match(stmt) => walk_match_pool(stmt, cx),
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => walk_expr_pool(inner, cx),
        Expr::Approx { left, right, tolerance, .. } => {
            walk_expr_pool(left, cx);
            walk_expr_pool(right, cx);
            walk_expr_pool(tolerance, cx);
        }
        Expr::Range { lo, hi, .. } => {
            walk_expr_pool(lo, cx);
            walk_expr_pool(hi, cx);
        }
        Expr::ArrayRepeat { val, .. } => walk_expr_pool(val, cx),
        Expr::Or { inner, disposition, .. } => {
            walk_expr_pool(inner, cx);
            match disposition {
                OrDisposition::Substitute(e) => walk_expr_pool(e, cx),
                OrDisposition::Fail(e, _) => walk_expr_pool(e, cx),
                OrDisposition::Raise(_)
                | OrDisposition::Discard(_)
                | OrDisposition::Wait(_) => {}
            }
        }
        Expr::Literal(_, _)
        | Expr::Ident(_)
        | Expr::Path(_)
        | Expr::KwSelf(_) => {}
        // Already handled above
        Expr::Call { .. } => unreachable!(),
    }
}

/// If `receiver` is `self.X` where X is a field of
/// `enclosing_locus` whose declared type names a locus, return
/// that locus's name. Otherwise None.
fn receiver_field_locus_type(
    receiver: &Expr,
    enclosing_locus: &LocusDecl,
    top: &TopScope,
) -> Option<String> {
    let Expr::Field { receiver: inner, name, .. } = receiver else {
        return None;
    };
    if !matches!(inner.as_ref(), Expr::KwSelf(_)) {
        return None;
    }
    // Find the field on enclosing_locus's params block.
    let params = enclosing_locus
        .members
        .iter()
        .find_map(|m| match m {
            LocusMember::Params(pb) => Some(pb),
            _ => None,
        })?;
    let param = params.params.iter().find(|p| p.name.name == name.name)?;
    let ty = param.ty.as_ref()?;
    type_expr_locus_name(ty, top)
}

/// The pool a field is EXPLICITLY placed on by its owning locus's
/// own `placement { }` block, if any. `None` means the field carries
/// no explicit placement, so its instance co-locates with its owner's
/// pool. (F.31: a field instance's pool is owner-relative, not a
/// type-global property — the same locus type used as a field in two
/// loci on two pools yields two instances, one per owner.)
fn enclosing_field_placement(
    enclosing_locus: &LocusDecl,
    field_name: &str,
) -> Option<PoolId> {
    let pb = enclosing_locus.members.iter().find_map(|m| match m {
        LocusMember::Placement(pb) => Some(pb),
        _ => None,
    })?;
    let entry = pb.entries.iter().find(|e| e.field.name == field_name)?;
    Some(placement_spec_to_pool(&entry.spec, &entry.field.name))
}

fn receiver_display(e: &Expr) -> String {
    match e {
        Expr::KwSelf(_) => "self".to_string(),
        Expr::Field { receiver, name, .. } => {
            format!("{}.{}", receiver_display(receiver), name.name)
        }
        Expr::Ident(i) => i.name.clone(),
        _ => "<expr>".to_string(),
    }
}

/// Bundle-wide validation for the v1.x topic-bindings feature.
/// Runs after per-locus checks because it cuts across loci. The
/// rules:
///   - At most one `main` locus per bundle. (Zero is fine — the
///     classic `fn main()` shape is still legal.)
///   - Each `bindings { Topic: <transport>; }` entry must name a
///     declared `topic`.
///   - A topic may appear at most once across all bindings.
///   - For `unix(...)` bindings without an explicit `role:` kwarg,
///     the role must be inferable from the bus block's
///     publish/subscribe declarations on this topic. Pub-only →
///     connect, sub-only → listen, both → compile error
///     ("specify `role:`").
/// A method signature as a diagnostic shows it (GH #732):
/// `fn put(String) -> Int fallible(E)`.
fn sig_text<'a>(
    name: &str,
    params: impl Iterator<Item = &'a Ty>,
    ret: &Ty,
    fallible: Option<&Ty>,
) -> String {
    let params: Vec<String> = params.map(|t| t.display()).collect();
    let mut s = format!("fn {}({})", name, params.join(", "));
    if *ret != Ty::Unit {
        s.push_str(&format!(" -> {}", ret.display()));
    }
    if let Some(e) = fallible {
        s.push_str(&format!(" fallible({})", e.display()));
    }
    s
}

/// Wave B: verify an adapter-binding locus satisfies the bus's
/// `__StdBusAdapter` contract (currently a single `send(subject:
/// String, bytes: Bytes)` method). Stand-alone shape — same logic
/// as `Checker::check_structural_impl` but callable from
/// `check_main_and_bindings` which doesn't construct a `Checker`.
fn check_satisfies_bus_adapter(
    top: &TopScope,
    locus_name: &str,
) -> Result<(), String> {
    const IFACE: &str = "__StdBusAdapter";
    let iface = match top.lookup(IFACE) {
        Some(TopSymbol::Interface(i)) => i,
        _ => {
            // The stdlib seed defines this interface; absence means
            // the seed wasn't loaded. Treat as OK rather than
            // failing user code with a stdlib-shape diagnostic.
            return Ok(());
        }
    };
    let locus = match top.lookup(locus_name) {
        Some(TopSymbol::Locus(l)) => l,
        _ => return Err(format!("`{}` is not a locus", locus_name)),
    };
    for im in &iface.methods {
        let lm = match locus.methods.iter().find(|lm| lm.name == im.name) {
            Some(m) => m,
            None => {
                return Err(format!(
                    "locus `{}` does not satisfy `{}`: missing method `{}`",
                    locus_name, IFACE, im.name
                ));
            }
        };
        if lm.params.len() != im.params.len() {
            return Err(format!(
                "locus `{}` method `{}` arity does not match `{}`: \
                 expected {} arg(s), locus has {}",
                locus_name,
                im.name,
                IFACE,
                im.params.len(),
                lm.params.len()
            ));
        }
        for (i, (lp, ip)) in lm.params.iter().zip(im.params.iter()).enumerate() {
            let want = &ip.1;
            if !want.assignable_from(lp) {
                return Err(format!(
                    "locus `{}` method `{}` arg #{} type mismatch: \
                     `{}` requires `{}`, locus has `{}`",
                    locus_name,
                    im.name,
                    i,
                    IFACE,
                    want.display(),
                    lp.display()
                ));
            }
        }
        if !im.ret.assignable_from(&lm.ret) {
            return Err(format!(
                "locus `{}` method `{}` return type mismatch: \
                 `{}` requires `{}`, locus returns `{}`",
                locus_name,
                im.name,
                IFACE,
                im.ret.display(),
                lm.ret.display()
            ));
        }
    }
    Ok(())
}

/// Form K4a (2026-05-20): validate the operational constraints
/// declared via the `where ...` clause on a binding entry.
///
/// Three classes of check:
///   1. **Intra-constraint consistency** — at most one scope
///      keyword per binding; `zero_copy` + `cross_machine` is a
///      contradiction.
///   2. **Transport-constraint compatibility** — does the
///      named transport satisfy each declared constraint? `unix`
///      is intra-machine, NOT zero-copy; `Adapter` is trusted
///      for scope (user-supplied transport), NOT zero-copy.
///   3. **Payload-shape compatibility** — `zero_copy` requires
///      the topic's payload to satisfy `is_flat_shapeable`.
///
/// Diagnostics are pushed to `diags`; the function returns
/// nothing (zero-or-more errors per binding).
/// F.36 Slice 2 (2026-05-28): codec(L) binding-clause typecheck.
/// When a binding entry carries a `codec(L { ... })` clause,
/// verify that:
///   1. L is a declared locus.
///   2. L has `fn encode(v: T) -> Bytes fallible(...)` where
///      T = the topic's payload type.
///   3. L has `fn decode(b: Bytes) -> T fallible(...)` where
///      T = the topic's payload type.
///   4. Both encode and decode are pure per F.36 Slice 1's
///      purity inference — codecs may be dispatched from the
///      bus reader thread / publisher's pool / consumer pools
///      concurrently, and have no coordination in scope to
///      serialize mutations to `self`.
fn check_binding_codec(
    entry: &BindingEntry,
    top: &TopScope,
    purity_map: &crate::purity::PurityMap,
    diags: &mut Vec<Diag>,
) {
    let codec = match &entry.codec {
        Some(c) => c,
        None => return,
    };
    // (1) Resolve the topic to its payload Ty.
    let topic_payload: Ty = match top.lookup(&entry.topic.name) {
        Some(TopSymbol::Topic(t)) => t.payload.clone(),
        _ => {
            // Already diagnosed by the parent "topic existence"
            // check; we can't verify the codec without a
            // payload type, so bail out silently.
            return;
        }
    };
    // (2) Resolve the codec locus.
    let locus_info = match top.lookup(&codec.locus.name) {
        Some(TopSymbol::Locus(l)) => l.clone(),
        Some(_) => {
            diags.push(Diag::ty(
                codec.locus.span,
                format!(
                    "codec binding for topic `{}`: `{}` is not a locus \
                     — `codec(L {{ ... }})` must name a locus that \
                     provides `encode` and `decode` methods",
                    entry.topic.name, codec.locus.name
                ),
            ));
            return;
        }
        None => {
            diags.push(Diag::ty(
                codec.locus.span,
                format!(
                    "codec binding for topic `{}`: unknown locus `{}`",
                    entry.topic.name, codec.locus.name
                ),
            ));
            return;
        }
    };
    // (3) Verify encode + decode methods exist with the right
    // signatures.
    let encode = locus_info.methods.iter().find(|m| m.name == "encode");
    let decode = locus_info.methods.iter().find(|m| m.name == "decode");
    let bytes_ty = Ty::Prim(hale_syntax::ast::PrimType::Bytes);

    match encode {
        None => {
            diags.push(Diag::ty(
                codec.locus.span,
                format!(
                    "codec `{}` for topic `{}` is missing required method \
                     `encode(v: {}) -> Bytes fallible(...)`",
                    codec.locus.name,
                    entry.topic.name,
                    topic_payload.display(),
                ),
            ));
        }
        Some(m) => {
            if m.params.len() != 1
                || !m.params[0].assignable_from(&topic_payload)
            {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `encode` must take one \
                         param of the topic's payload type `{}`; got params \
                         `{:?}`",
                        codec.locus.name,
                        entry.topic.name,
                        topic_payload.display(),
                        m.params.iter().map(|t| t.display()).collect::<Vec<_>>(),
                    ),
                ));
            }
            if !m.ret.assignable_from(&bytes_ty) {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `encode` must return \
                         `Bytes`; got `{}`",
                        codec.locus.name,
                        entry.topic.name,
                        m.ret.display(),
                    ),
                ));
            }
            if m.fallible.is_none() {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `encode` must be \
                         declared `fallible(E)` (encoding can fail; the \
                         binding-site dispatch needs a typed error \
                         channel)",
                        codec.locus.name, entry.topic.name,
                    ),
                ));
            }
        }
    }

    match decode {
        None => {
            diags.push(Diag::ty(
                codec.locus.span,
                format!(
                    "codec `{}` for topic `{}` is missing required method \
                     `decode(b: Bytes) -> {} fallible(...)`",
                    codec.locus.name,
                    entry.topic.name,
                    topic_payload.display(),
                ),
            ));
        }
        Some(m) => {
            if m.params.len() != 1
                || !m.params[0].assignable_from(&bytes_ty)
            {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `decode` must take one \
                         param of type `Bytes`; got params `{:?}`",
                        codec.locus.name,
                        entry.topic.name,
                        m.params.iter().map(|t| t.display()).collect::<Vec<_>>(),
                    ),
                ));
            }
            if !m.ret.assignable_from(&topic_payload) {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `decode` must return \
                         the topic's payload type `{}`; got `{}`",
                        codec.locus.name,
                        entry.topic.name,
                        topic_payload.display(),
                        m.ret.display(),
                    ),
                ));
            }
            if m.fallible.is_none() {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `decode` must be \
                         declared `fallible(E)` (decoding can fail; the \
                         binding-site dispatch needs a typed error \
                         channel)",
                        codec.locus.name, entry.topic.name,
                    ),
                ));
            }
        }
    }
    // (4) Purity assertion. Codec methods may be invoked from
    // arbitrary threads (bus reader thread, publisher pool,
    // consumer pools) concurrently with no coordination in scope
    // to serialize mutations to self. They MUST be pure.
    for method_name in &["encode", "decode"] {
        let key = crate::purity::PurityKey::method(
            codec.locus.name.clone(),
            (*method_name).to_string(),
        );
        match purity_map.get(&key) {
            Some(crate::purity::Purity::Pure) => {}
            Some(crate::purity::Purity::Impure(reason)) => {
                let (line, hint) = render_impurity(reason);
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}.{}` is not safe to dispatch from \
                         arbitrary threads\n\n\
                         note: codec methods must be stateless — they may \
                         be invoked from the bus reader thread, the \
                         publisher's pool, and consumer pools concurrently. \
                         No coordination is in scope to serialize mutations \
                         to `self`.\n\n\
                         note: {}\n\n\
                         help: {}",
                        codec.locus.name, method_name, line, hint,
                    ),
                ));
            }
            None => {
                // Method should have been in the map if the
                // locus was indexed; absence means the locus
                // doesn't actually have the named method (the
                // signature-mismatch branch above will have
                // already diagnosed). Quiet here to avoid
                // duplicate diagnostics.
            }
        }
    }
}

/// Render an [`Impurity`] as `(note_line, fix_hint)` strings for
/// embedding in a codec binding-site diagnostic.
fn render_impurity(
    reason: &crate::purity::Impurity,
) -> (String, &'static str) {
    use crate::purity::Impurity::*;
    match reason {
        SelfFieldWrite { field_chain, .. } => (
            format!("writes to `{}` (mutates the codec instance)", field_chain),
            "codecs are pure transformations on input data. \
             Move per-call counters out of the codec — push them \
             through the bus as observability events, or measure at \
             the adapter layer where state has lifecycle.",
        ),
        BusSend { subject_repr, .. } => (
            format!(
                "publishes to a bus topic ({}) — a side effect outside \
                 the codec's input/output channel",
                subject_repr
            ),
            "codecs translate between values and bytes; they don't \
             route messages. If you need to fire downstream events, \
             do it from the locus that owns the relationship, not \
             from the codec.",
        ),
        Violate { closure_name, .. } => (
            format!(
                "violates closure `{}` — escalates a structural failure \
                 through the parent",
                closure_name
            ),
            "codecs report failures via their `fallible(E)` return \
             channel, not via closure violations. Replace `violate` \
             with `fail SomeError {{ ... }}` in the codec body.",
        ),
        ImpureStdlibCall { fn_name, .. } => (
            format!(
                "calls `{}`, which has side effects (printing, \
                 file/process I/O, sleeping, or recovery)",
                fn_name
            ),
            "codecs must be deterministic, side-effect-free \
             transformations. Remove the offending call from the \
             codec body.",
        ),
        ImpureCalleeCall { callee_name, .. } => (
            format!(
                "calls `{}`, which is itself not pure (transitively)",
                callee_name
            ),
            "either make the called fn pure (no self-writes, no I/O, \
             no impure callees), or inline the small pure pieces \
             directly into the codec body.",
        ),
    }
}

fn check_binding_constraints(
    entry: &BindingEntry,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    if entry.constraints.is_empty() {
        return;
    }

    // (1) intra-constraint consistency.
    let scope_constraints: Vec<&SpannedBindingConstraint> = entry
        .constraints
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                BindingConstraint::IntraProcess
                    | BindingConstraint::IntraMachine
                    | BindingConstraint::CrossMachine
            )
        })
        .collect();
    if scope_constraints.len() > 1 {
        // Diagnostic cites the second one; the first is the
        // surviving "declared" scope. Pick whichever the user
        // sees first in source order; the parser preserves
        // declaration order.
        diags.push(Diag::ty(
            scope_constraints[1].span,
            format!(
                "binding for topic `{}` has multiple scope constraints \
                 (`{}` and `{}`); pick one",
                entry.topic.name,
                scope_constraints[0].kind.name(),
                scope_constraints[1].kind.name(),
            ),
        ));
    }

    let has_zero_copy = entry
        .constraints
        .iter()
        .any(|c| matches!(c.kind, BindingConstraint::ZeroCopy));
    let has_cross_machine = entry
        .constraints
        .iter()
        .any(|c| matches!(c.kind, BindingConstraint::CrossMachine));
    if has_zero_copy && has_cross_machine {
        // Find the zero_copy span for the diagnostic location.
        let span = entry
            .constraints
            .iter()
            .find(|c| matches!(c.kind, BindingConstraint::ZeroCopy))
            .map(|c| c.span)
            .unwrap_or(entry.span);
        diags.push(Diag::ty(
            span,
            format!(
                "binding for topic `{}`: `zero_copy` and `cross_machine` \
                 contradict — network transports require serialization",
                entry.topic.name
            ),
        ));
    }

    // (2) transport-constraint compatibility.
    for c in &entry.constraints {
        if let Some(msg) = transport_satisfies(&entry.transport, c.kind) {
            diags.push(Diag::ty(
                c.span,
                format!("binding for topic `{}`: {}", entry.topic.name, msg),
            ));
        }
    }

    // (3) payload-shape compatibility — `zero_copy` requires
    //     `is_flat_shapeable`. Look the topic's payload up
    //     through the resolved top scope; skip silently if the
    //     topic isn't registered (a separate diagnostic upstream
    //     will catch the missing topic).
    if has_zero_copy {
        if let Some(TopSymbol::Topic(topic)) =
            top.lookup(&entry.topic.name)
        {
            if !is_flat_shapeable(&topic.payload, top) {
                let span = entry
                    .constraints
                    .iter()
                    .find(|c| matches!(c.kind, BindingConstraint::ZeroCopy))
                    .map(|c| c.span)
                    .unwrap_or(entry.span);
                diags.push(Diag::ty(
                    span,
                    format!(
                        "binding for topic `{}` requires `zero_copy` but \
                         payload type `{}` is not flat-shapeable — it \
                         contains a String, Bytes, or fixed-size array \
                         field, whose storage is out-of-line (a pointer), \
                         so a raw memcpy would share a pointer that dangles \
                         across the zero-copy boundary. Use only fixed-size \
                         scalar fields in a `zero_copy` payload",
                        entry.topic.name,
                        topic.payload.display()
                    ),
                ));
            }
        }
    }
}

/// Returns `Some(reason)` if `transport` cannot satisfy
/// `constraint`. Returns `None` when the transport satisfies it
/// (or when the satisfaction can't be determined and trust
/// defaults to "OK" — adapter loci for scope constraints).
fn transport_satisfies(
    transport: &TransportSpec,
    constraint: BindingConstraint,
) -> Option<String> {
    use BindingConstraint::*;
    match (transport, constraint) {
        // unix: intra-machine substrate, kernel-memcpy at the
        // socket boundary.
        (TransportSpec::Unix { .. }, IntraProcess) => Some(
            "`unix` transport crosses OS process boundaries; cannot \
             satisfy `intra_process`"
                .into(),
        ),
        (TransportSpec::Unix { .. }, IntraMachine) => None,
        (TransportSpec::Unix { .. }, CrossMachine) => Some(
            "`unix` transport is host-local (AF_UNIX); cannot satisfy \
             `cross_machine`"
                .into(),
        ),
        (TransportSpec::Unix { .. }, ZeroCopy) => Some(
            "`unix` transport memcpys at the kernel boundary; cannot \
             satisfy `zero_copy`"
                .into(),
        ),

        // Adapter: user-supplied. Trust for scope constraints
        // (the adapter body knows where it routes). Reject
        // zero_copy — the Adapter contract (`fn send(subject: \
        // String, bytes: Bytes)`) requires serialization.
        (TransportSpec::Adapter { .. }, ZeroCopy) => Some(
            "`Adapter` transports cannot satisfy `zero_copy` — the \
             Adapter contract (`fn send(subject, bytes)`) requires \
             serialization to Bytes"
                .into(),
        ),
        (TransportSpec::Adapter { .. }, _) => None,

        // shm_ring: POSIX SHM ring substrate. Cross-process by
        // design (different procs mmap the same fd); host-local
        // (POSIX SHM doesn't traverse the network); satisfies
        // zero_copy intrinsically.
        (TransportSpec::ShmRing { .. }, IntraProcess) => Some(
            "`shm_ring` is cross-process by design (POSIX SHM); \
             cannot satisfy `intra_process`"
                .into(),
        ),
        (TransportSpec::ShmRing { .. }, IntraMachine) => None,
        (TransportSpec::ShmRing { .. }, CrossMachine) => Some(
            "`shm_ring` is host-local (POSIX SHM); cannot satisfy \
             `cross_machine`"
                .into(),
        ),
        (TransportSpec::ShmRing { .. }, ZeroCopy) => None,
    }
}

/// Phase 3 routing-keys (2026-05-25): cross-program checks for
/// the `fallback` policy.
///
/// * Every topic declared `on_unmatched: fallback` must have at
///   least one `where key == _` subscriber in the program.
///   Otherwise unmatched-key publishes would have nowhere to go
///   and the fallback policy is silently degraded to swallow.
/// * `where key == _` is only legal on topics that explicitly
///   declared `on_unmatched: fallback`. Using it on swallow or
///   unkeyed topics catches programmer typos.
///
/// Subscribers reference topics by either name (`subscribe K as
/// h`) or literal subject (`subscribe "k" as h of type T`). We
/// validate both forms; for literal subjects we look up the topic
/// by its wire subject string.
/// GH #255 phase 2 bundle checks:
/// * a topic `bounded(N)` requires `on_full: fail` (v1's only
///   topic-level policy) and vice versa;
/// * a subscribe-site `bounded(N, policy)` is only legal on a
///   MAIN-queue subscriber: pool queues and pinned mailboxes are
///   already bounded MPSC rings with producer-blocking
///   backpressure (GH #125), so shed bounds there would
///   misdescribe the actual contract.
fn check_bounded_bus(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    // GH #825: a `topic` and a subscriber inside a `module { … }` are
    // ordinary bundle members — `collect_subscriber_placements`
    // already reads them, so only these two walks were short.
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Topic(t) = item {
                match (t.bounded, t.on_full_fail) {
                    (Some((_, bspan)), None) => diags.push(Diag::ty(
                        bspan,
                        "topic `bounded(N)` requires `on_full: fail;` \
                         — a capacity with no declared policy is \
                         meaningless (consumer-side shedding is \
                         declared on the subscribe instead: \
                         `bounded(N, drop_old)`)",
                    )),
                    (None, Some(fspan)) => diags.push(Diag::ty(
                        fspan,
                        "topic `on_full: fail;` requires `bounded(N);` \
                         — a refusal policy with no capacity never \
                         fires",
                    )),
                    _ => {}
                }
            }
        });
    }
    let placements = crate::bus_graph::collect_subscriber_placements(bundle);
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(l) = item else { return };
            for member in &l.members {
                let LocusMember::Bus(bb) = member else { continue };
                for bm in &bb.members {
                    let BusMember::Subscribe {
                        bound: Some(b), ..
                    } = bm
                    else {
                        continue;
                    };
                    let placed = placements
                        .get(&l.name.name)
                        .cloned()
                        .unwrap_or(crate::bus_graph::Placement::SameThread);
                    if placed != crate::bus_graph::Placement::SameThread {
                        diags.push(Diag::ty(
                            b.span,
                            format!(
                                "subscriber `bounded(N, ...)` is only \
                                 supported on main-queue subscribers at \
                                 v1, and `{}` is placed off-main — its \
                                 pool/mailbox ring is already bounded \
                                 with producer-blocking backpressure \
                                 (GH #125)",
                                l.name.name
                            ),
                        ));
                    }
                }
            }
        });
    }
}

fn check_phase3_fallback_subscribers(
    bundle: &Bundle<'_>,
    diags: &mut Vec<Diag>,
) {
    // Collect all topics with their on_unmatched policy + wire
    // subject. Indexed by both topic name and wire subject so
    // subscribe-by-string can resolve.
    let mut by_name: BTreeMap<String, (Option<UnmatchedPolicy>, Span)> =
        BTreeMap::new();
    let mut by_wire: BTreeMap<String, (Option<UnmatchedPolicy>, Span)> =
        BTreeMap::new();
    // Filter validity (2026-08-12): (is_keyed, key_is_string) per
    // topic, for the `where key ==` rules below. The String
    // question resolves the payload struct's keyed_by field type
    // directly against the bundle's type decls.
    let mut key_shape_by_name: BTreeMap<String, (bool, bool)> =
        BTreeMap::new();
    let mut key_shape_by_wire: BTreeMap<String, (bool, bool)> =
        BTreeMap::new();
    let field_is_string = |payload: &TypeExpr, field: &str| -> bool {
        let TypeExpr::Named { path, .. } = payload else {
            return false;
        };
        if path.segments.len() != 1 {
            return false;
        }
        let tname = &path.segments[0].name;
        // GH #825: the payload struct can be declared in a module
        // too, and a payload this lookup cannot find reads as "not a
        // String key", which decides the `where key == replica` rule.
        // First declaration wins, exactly as the nested `for` it
        // replaces did — a second one of the same name is a
        // duplicate the resolver reports.
        let mut answer: Option<bool> = None;
        for program in bundle.programs.values() {
            walk_decls(&program.items, &mut |item| {
                if answer.is_some() {
                    return;
                }
                if let TopDecl::Type(td) = item {
                    if &td.name.name == tname {
                        if let TypeDeclBody::Struct(fields) = &td.body {
                            answer = Some(fields.iter().any(|f| {
                                f.name.name == field
                                    && matches!(
                                        &f.ty,
                                        TypeExpr::Primitive(
                                            PrimType::String,
                                            _,
                                        )
                                    )
                            }));
                        }
                    }
                }
            });
        }
        answer.unwrap_or(false)
    };
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Topic(t) = item {
                by_name.insert(
                    t.name.name.clone(),
                    (t.on_unmatched, t.span),
                );
                let key_shape = match &t.keyed_by {
                    Some(f) => {
                        (true, field_is_string(&t.payload, &f.name))
                    }
                    None => (false, false),
                };
                key_shape_by_name
                    .insert(t.name.name.clone(), key_shape);
                let wire = t
                    .subject
                    .clone()
                    .unwrap_or_else(|| t.name.name.clone());
                key_shape_by_wire.insert(wire.clone(), key_shape);
                by_wire.insert(wire, (t.on_unmatched, t.span));
            }
        });
    }

    // Walk every subscriber. For each `where key == _` filter,
    // resolve the topic and validate it's a fallback topic.
    // Track which fallback topics have at least one `_` sub.
    let mut fallback_has_catchall: BTreeMap<String, bool> = BTreeMap::new();
    for (name, (policy, _)) in &by_name {
        if matches!(policy, Some(UnmatchedPolicy::Fallback)) {
            fallback_has_catchall.insert(name.clone(), false);
        }
    }
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(l) = item else { return };
            for m in &l.members {
                let LocusMember::Bus(bb) = m else { continue };
                for bm in &bb.members {
                    let BusMember::Subscribe { subject, key_filter, .. } = bm
                    else {
                        continue;
                    };
                    let Some(kf) = key_filter else { continue };
                    // 2026-08-12: every `where key ==` filter needs a
                    // KEYED topic — an unkeyed publish never runs the
                    // key match, so a filtered subscriber would
                    // silently receive nothing, forever (previously
                    // accepted without a word). Cross-seed / string
                    // subjects that resolve to no declared topic stay
                    // permissive, matching the rest of this pass.
                    {
                        let shape = match subject {
                            BusSubject::Topic(i) => {
                                key_shape_by_name.get(&i.name).copied()
                            }
                            BusSubject::Literal { subject: sname, .. } => {
                                key_shape_by_wire.get(sname).copied()
                            }
                            BusSubject::QualifiedTopic(_) => None,
                        };
                        if let Some((is_keyed, key_is_string)) = shape {
                            if !is_keyed {
                                diags.push(Diag::ty(
                                    kf.span(),
                                    format!(
                                        "`where key == …` requires a keyed topic, and `{}` \
                                         declares no `keyed_by` — an unkeyed publish never \
                                         runs the key match, so this subscriber would \
                                         silently receive nothing",
                                        subject.canonical()
                                    ),
                                ));
                                continue;
                            }
                            if key_is_string
                                && matches!(kf, KeyFilter::Replica { .. })
                            {
                                diags.push(Diag::ty(
                                    kf.span(),
                                    format!(
                                        "`where key == replica` needs an Int-family key; \
                                         topic `{}` is keyed by a String field — shard on \
                                         an Int field to fan out across replicas",
                                        subject.canonical()
                                    ),
                                ));
                                continue;
                            }
                        }
                    }
                    let is_catchall = matches!(kf, KeyFilter::Unmatched { .. });
                    if !is_catchall {
                        continue;
                    }
                    let (topic_key, policy) = match subject {
                        BusSubject::Topic(i) => (
                            i.name.clone(),
                            by_name.get(&i.name).map(|x| x.0).flatten(),
                        ),
                        BusSubject::Literal { subject: s, .. } => (
                            s.clone(),
                            by_wire.get(s).map(|x| x.0).flatten(),
                        ),
                        BusSubject::QualifiedTopic(qn) => {
                            let last = qn
                                .segments
                                .last()
                                .map(|s| s.name.clone())
                                .unwrap_or_default();
                            (
                                last.clone(),
                                by_name.get(&last).map(|x| x.0).flatten(),
                            )
                        }
                    };
                    if !matches!(policy, Some(UnmatchedPolicy::Fallback)) {
                        diags.push(Diag::ty(
                            kf.span(),
                            format!(
                                "`where key == _` is only legal on \
                                 topics declared `on_unmatched: \
                                 fallback`; topic `{}` declares {}",
                                topic_key,
                                match policy {
                                    Some(UnmatchedPolicy::Swallow) =>
                                        "`on_unmatched: swallow`",
                                    Some(UnmatchedPolicy::Fail) =>
                                        "`on_unmatched: fail`",
                                    Some(UnmatchedPolicy::Fallback) =>
                                        unreachable!(),
                                    None => "no `on_unmatched` (default: \
                                             swallow)",
                                },
                            ),
                        ));
                    } else {
                        fallback_has_catchall.insert(topic_key, true);
                    }
                }
            }
        });
    }
    for (name, has) in &fallback_has_catchall {
        if *has {
            continue;
        }
        let span = by_name.get(name).map(|(_, s)| *s).unwrap_or(Span::new(0, 0));
        diags.push(Diag::ty(
            span,
            format!(
                "topic `{}` declares `on_unmatched: fallback` but \
                 no subscriber declares `where key == _`; \
                 unmatched-key publishes would have nowhere to go",
                name
            ),
        ));
    }
}

/// GH #911 (B6): the entry point is TOP-LEVEL only.
///
/// A module is a namespace and not an analysis boundary, so nearly
/// every declaration means the same thing one brace deeper (GH #825,
/// GH #884). The entry point is the exception `spec/semantics.md`
/// § "Declarations inside `module { }`" names: codegen looks for
/// `fn main` in `program.items` and nowhere else — a module-nested
/// one is not the program's entry point, and promoting it there
/// would make codegen the only layer that thinks so.
///
/// What it was NOT is a reason for `check` to stay quiet. A seed whose
/// only `fn main` sits inside a module checked clean and then failed
/// to build with codegen's spanless `program has no `fn main()``, so
/// the two layers disagreed about a program (the ruling on GH #911,
/// 2026-09-20). They agree here instead, with the position of the
/// declaration that has to move.
fn check_entry_point_placement(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    fn walk(items: &[TopDecl], module: Option<&str>, diags: &mut Vec<Diag>) {
        for item in items {
            match item {
                TopDecl::Fn(f) if f.name.name == "main" => {
                    let Some(m) = module else { continue };
                    diags.push(Diag::ty(
                        f.name.span,
                        format!(
                            "the entry point must be top-level: a `fn \
                             main` inside `module {}` does not start the \
                             program — move it out of the module, or \
                             rename it if it is an ordinary function",
                            m
                        ),
                    ));
                }
                TopDecl::Module(m) => walk(&m.items, Some(&m.name.name), diags),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, None, diags);
    }
}

fn check_main_and_bindings(
    bundle: &Bundle<'_>,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    let mut mains: Vec<(String, Span)> = Vec::new();
    let mut bound: BTreeMap<String, Span> = BTreeMap::new();

    // For role inference: gather, per wire-subject, whether ANY
    // locus in the bundle publishes / subscribes to it. Bindings
    // reference topic-name, so map name → (publishes, subscribes).
    let (topic_publishes, topic_subscribes) = collect_topic_pub_sub(bundle);

    // F.36 Slice 2 (2026-05-28): compute the bundle-wide purity
    // map so binding-site codec checks can assert the codec's
    // encode/decode methods are pure. Done once here; threaded
    // into `check_binding_codec`. v0.1 always computes; future
    // polish could gate on "any binding has codec" to skip the
    // walk for the common case.
    let programs_vec: Vec<&Program> = bundle.programs.values().copied().collect();
    let purity_map = crate::purity::infer_purity_for_bundle(&programs_vec, top);

    // GH #825: a `main locus` and a `bindings { }` block inside a
    // `module { … }` are ordinary bundle members. The at-most-one-main
    // rule in particular is a whole-bundle count, and a count that
    // skips half the declarations is not a count.
    // (`collect_topic_pub_sub`, which feeds role inference, already
    // recursed — it grew its own `TopDecl::Module` arm.)
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                // An imported seed's main locus is renamed `__lib_*` and
                // is not this program's entry (its bindings are inert),
                // so it does not count: a composed head imports the
                // standalone head, main locus and all (GH #1104 piece 5).
                if l.is_main && !l.imported {
                    mains.push((l.name.name.clone(), l.span));
                }
                for member in &l.members {
                    if let LocusMember::Bindings(bb) = member {
                        for entry in &bb.entries {
                            // Topic existence
                            match top.lookup(&entry.topic.name) {
                                Some(TopSymbol::Topic(_)) => {}
                                _ => {
                                    diags.push(Diag::ty(
                                        entry.topic.span,
                                        format!(
                                            "binding references unknown topic `{}`",
                                            entry.topic.name
                                        ),
                                    ));
                                }
                            }
                            // Duplicate topic across all bindings
                            if let Some(prev) = bound.get(&entry.topic.name) {
                                diags.push(
                                    Diag::ty(
                                        entry.topic.span,
                                        format!(
                                            "topic `{}` already bound",
                                            entry.topic.name
                                        ),
                                    )
                                    .with_related(*prev, "previous binding"),
                                );
                            } else {
                                bound.insert(entry.topic.name.clone(), entry.topic.span);
                            }

                            // Role inference validation. Substrate
                            // Unix bindings need a role (inferred or
                            // explicit); Adapter bindings carry
                            // direction inside the adapter locus's
                            // own params and are opaque here.
                            if let TransportSpec::Unix { role, .. } =
                                &entry.transport
                            {
                                if role.is_none() {
                                    let pubs = topic_publishes
                                        .contains(&entry.topic.name);
                                    let subs = topic_subscribes
                                        .contains(&entry.topic.name);
                                    if pubs && subs {
                                        diags.push(Diag::ty(
                                            entry.topic.span,
                                            format!(
                                                "binding for topic `{}` is ambiguous: \
                                                 some locus publishes it AND some locus \
                                                 subscribes to it; specify `role:` \
                                                 (e.g. `unix(\"/path\", role: listen)`)",
                                                entry.topic.name
                                            ),
                                        ));
                                    } else if !pubs && !subs {
                                        diags.push(Diag::ty(
                                            entry.topic.span,
                                            format!(
                                                "binding for topic `{}` has no publisher \
                                                 or subscriber in the bundle; nothing to \
                                                 route. Add a `bus {{ publish | subscribe }}` \
                                                 or remove the binding",
                                                entry.topic.name
                                            ),
                                        ));
                                    }
                                    // Otherwise (exactly one of pubs/subs):
                                    // role is inferable; desugar fills it in.
                                }
                            }

                            // Wave B: adapter binding checks. Verify
                            // the named symbol is a locus and that it
                            // structurally satisfies `__StdBusAdapter`
                            // (i.e. exposes `fn send(subject: String,
                            // bytes: Bytes)`). Field-init shape is
                            // codegen's job once the locus is
                            // resolved.
                            if let TransportSpec::Adapter { locus, .. } =
                                &entry.transport
                            {
                                match top.lookup(&locus.name) {
                                    Some(TopSymbol::Locus(_)) => {
                                        if let Err(msg) = check_satisfies_bus_adapter(
                                            top, &locus.name,
                                        ) {
                                            diags.push(Diag::ty(
                                                locus.span,
                                                format!(
                                                    "adapter binding for topic `{}`: {}",
                                                    entry.topic.name, msg
                                                ),
                                            ));
                                        }
                                    }
                                    Some(_) => {
                                        diags.push(Diag::ty(
                                            locus.span,
                                            format!(
                                                "adapter binding for topic `{}`: \
                                                 `{}` is not a locus — adapter \
                                                 transport spec must name a locus \
                                                 that satisfies `__StdBusAdapter`",
                                                entry.topic.name, locus.name
                                            ),
                                        ));
                                    }
                                    None => {
                                        diags.push(Diag::ty(
                                            locus.span,
                                            format!(
                                                "adapter binding for topic `{}`: \
                                                 unknown locus `{}`",
                                                entry.topic.name, locus.name
                                            ),
                                        ));
                                    }
                                }
                            }

                            // shm-ring-interop Proposal B: a
                            // `shm_ring(..., layout: Name)` binding's
                            // layout reference must resolve to a
                            // declared `ring_layout`. Absent layout =
                            // the native shape (back-compat).
                            if let TransportSpec::ShmRing {
                                layout: Some(lid), buffer_size, ..
                            } = &entry.transport
                            {
                                match top.lookup(&lid.name) {
                                    Some(TopSymbol::RingLayout(rl)) => {
                                        // The producer's compile-time
                                        // `buffer_size:` must be a multiple
                                        // of the layout's record `align`,
                                        // else a record header near the wrap
                                        // lands in (cap-len_prefix, cap) →
                                        // OOB. (The consumer enforces the
                                        // same at attach for the foreign
                                        // header's capacity.)
                                        if let Some(bs) = buffer_size {
                                            use hale_syntax::ast::RingAttrValue;
                                            let align = rl
                                                .decl
                                                .framing
                                                .as_ref()
                                                .and_then(|f| f.attrs.iter().find_map(|a| {
                                                    match (a.key.name.as_str(), &a.value) {
                                                        ("align", RingAttrValue::Int(n)) => Some(*n),
                                                        _ => None,
                                                    }
                                                }))
                                                .unwrap_or(1);
                                            if align > 1 && (*bs as i64) % align != 0 {
                                                diags.push(Diag::ty(
                                                    entry.span,
                                                    format!(
                                                        "shm_ring binding for topic `{}`: \
                                                         `buffer_size: {}` must be a \
                                                         multiple of the `{}` layout's \
                                                         record `align` ({}) — otherwise a \
                                                         record header can straddle the \
                                                         wrap boundary",
                                                        entry.topic.name, bs, lid.name, align
                                                    ),
                                                ));
                                            }
                                        }
                                        // Conformance (2026-06-06): a
                                        // layout-bound topic is read by
                                        // direct pointer-cast and written
                                        // by memcpy of the payload struct
                                        // (the bindgen-style contract — the
                                        // foreign format is fixed, so the
                                        // Hale struct must *be* the record
                                        // bytes). That's only sound if the
                                        // payload is flat-shapeable, and it
                                        // holds whether or not the binding
                                        // also asserts `where zero_copy`.
                                        if let Some(TopSymbol::Topic(topic)) =
                                            top.lookup(&entry.topic.name)
                                        {
                                            // A `BytesView` payload selects the raw-frame
                                            // mode: the consumer hands the handler a
                                            // bounded view over each record (decoded with
                                            // `std::bytes::read_*` + a discriminator), for
                                            // heterogeneous / variable-length rings. Any
                                            // other payload takes the typed-flat path,
                                            // which is read by direct cast and so must be
                                            // flat-shapeable.
                                            let is_raw_view = matches!(
                                                &topic.payload,
                                                Ty::Prim(hale_syntax::ast::PrimType::BytesView)
                                            );
                                            if !is_raw_view
                                                && !is_flat_shapeable(&topic.payload, top)
                                            {
                                                diags.push(Diag::ty(
                                                    entry.span,
                                                    format!(
                                                        "shm_ring binding for topic \
                                                         `{}` with `layout: {}` requires \
                                                         a flat-shapeable payload (read by \
                                                         direct cast) or a `BytesView` \
                                                         payload (raw-frame mode), but \
                                                         `{}` is neither — it contains \
                                                         String, Bytes, or other \
                                                         variable-size fields",
                                                        entry.topic.name,
                                                        lid.name,
                                                        topic.payload.display()
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                    Some(_) => diags.push(Diag::ty(
                                        lid.span,
                                        format!(
                                            "shm_ring binding for topic `{}`: \
                                             `layout: {}` is not a `ring_layout` \
                                             declaration",
                                            entry.topic.name, lid.name
                                        ),
                                    )),
                                    None => diags.push(Diag::ty(
                                        lid.span,
                                        format!(
                                            "shm_ring binding for topic `{}`: \
                                             unknown ring_layout `{}`",
                                            entry.topic.name, lid.name
                                        ),
                                    )),
                                }
                            }

                            // Form K4a (2026-05-20): operational-
                            // constraint validity. The `where ...`
                            // clause asserts properties of the
                            // route; the typechecker validates
                            // intra-constraint consistency,
                            // transport compatibility, and
                            // payload-shape compatibility.
                            check_binding_constraints(
                                entry, top, diags,
                            );

                            // F.36 Slice 2 (2026-05-28): pluggable
                            // codec validity. When the binding
                            // entry carries a `codec(L { ... })`
                            // clause, verify L has the encode /
                            // decode methods with the right
                            // signatures (against the topic's
                            // payload type) AND that both methods
                            // are pure per Slice 1's inference.
                            check_binding_codec(entry, top, &purity_map, diags);

                            // Form K6b (2026-05-20): shm_ring
                            // Hale-side subscribers are wired
                            // (reader thread + handler dispatch
                            // in lotus_bus_register_subscriber_shm_ring).
                            // No typecheck rejection needed; the
                            // codegen handles both publish-only
                            // and subscribe-bearing programs.
                            let _ = &topic_subscribes;
                        }
                    }
                }
            }
        });
    }
    check_api_binding(&programs_vec, diags);
    check_api_roles(&programs_vec, diags);
    if mains.len() > 1 {
        for (name, span) in &mains {
            diags.push(Diag::ty(
                *span,
                format!(
                    "more than one `main` locus declared (`{}` is one of {})",
                    name,
                    mains.len()
                ),
            ));
        }
    }
}

/// GH #1106: the `api:` entry. The knobs the entry must carry, the
/// one-replier rule, and what the api leaves out. The surface is the
/// one `api_gen` emitted from, so a warning here names exactly what
/// the binding will not serve.
fn check_api_binding(programs: &[&Program], diags: &mut Vec<Diag>) {
    let Some(surface) = hale_syntax::api_gen::api_surface(programs) else {
        return;
    };
    let b = &surface.binding;
    if b.bound.is_none() || b.on_full.is_none() {
        diags.push(Diag::ty(
            b.span,
            format!(
                "api binding: `bound:` and `on_full: refuse` are required — the \
                 request side of the binding needs a bound and a policy (the dev \
                 default `hale run --api` uses is `bound: {}, on_full: refuse`)",
                hale_syntax::api_gen::DEV_BOUND
            ),
        ));
    }
    if b.watch_bound.is_some() != b.on_watch_full.is_some() {
        diags.push(Diag::ty(
            b.watch_bound
                .map(|(_, s)| s)
                .or(b.on_watch_full.map(|(_, s)| s))
                .unwrap_or(b.span),
            "api binding: `watch_bound:` and `on_watch_full:` go together — a \
             watcher's queue needs a bound and a policy (`drop_old` or \
             `drop_new`); omit both to reuse `bound` with `drop_old`"
                .to_string(),
        ));
    }
    for (topic, first, second) in &surface.ambiguous_replies {
        diags.push(
            Diag::ty(
                *second,
                format!(
                    "api binding: topic `{}` has two subscribers that declare a \
                     return type, so the reply through the binding would be \
                     ambiguous; at most one subscriber of a topic answers",
                    topic
                ),
            )
            .with_related(*first, "the first replying handler"),
        );
    }
    for ex in &surface.excluded {
        diags.push(Diag::warn(
            b.span,
            format!(
                "api binding: {} is not served through the binding — {}",
                ex.what, ex.reason
            ),
        ));
    }
}

/// GH #1109: the role vocabulary and the `@gated(role:)` sites.
///
/// Roles are declared vocabulary like `group` and `effect`: a name
/// nothing declares is an error, bundle-wide, with `owner` the one
/// role that needs no declaration (a program declares it only to
/// give it `includes`). `includes` is grant-only and union-only, so a
/// cycle says nothing and is refused. A gate goes on a subscribed
/// handler, an `expose` member or a `publish`, and every subscriber
/// (or publisher) of one topic states the same gate, because the
/// binding refuses the message, not the handler. A gated handler's
/// topic cannot also be bound to a transport in `bindings { }`: that
/// transport has no gate, so the annotation would promise a check
/// that does not run.
fn check_api_roles(programs: &[&Program], diags: &mut Vec<Diag>) {
    use hale_syntax::ast::{ApiRoles, BusSubject, ContractDirection, ContractKind, Expr, Ident, PrimType};
    use std::collections::{BTreeMap, BTreeSet};

    let mut decls: BTreeMap<String, (Vec<Ident>, Span)> = BTreeMap::new();
    for p in programs {
        walk_decls(&p.items, &mut |item| {
            if let TopDecl::Role(r) = item {
                if let Some((_, first)) = decls.get(&r.name.name) {
                    diags.push(
                        Diag::ty(
                            r.name.span,
                            format!(
                                "role `{}` is declared twice; a role is one name the \
                                 deployment maps, so declare it once and `includes` it \
                                 where a wider role should hold it",
                                r.name.name
                            ),
                        )
                        .with_related(*first, "the first declaration"),
                    );
                } else {
                    decls.insert(r.name.name.clone(), (r.includes.clone(), r.name.span));
                }
            }
        });
    }
    let declared = |n: &str| n == "owner" || decls.contains_key(n);
    let undeclared = |n: &Ident, at: &str| {
        Diag::ty(
            n.span,
            format!(
                "{} names role `{}`, which nothing declares — roles are declared \
                 vocabulary: `role {};` at top level (only `owner` needs no declaration)",
                at, n.name, n.name
            ),
        )
    };
    for (name, (incs, span)) in &decls {
        for inc in incs {
            if !declared(&inc.name) {
                diags.push(undeclared(inc, &format!("`role {} includes …`", name)));
            }
        }
        // A cycle: `name` reachable from its own includes.
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = incs.iter().map(|i| i.name.clone()).collect();
        while let Some(cur) = stack.pop() {
            if cur == *name {
                diags.push(Diag::ty(
                    *span,
                    format!(
                        "role `{}` includes itself through its `includes` chain; \
                         composition is grant-only and union-only, so a cycle says nothing",
                        name
                    ),
                ));
                break;
            }
            if !seen.insert(cur.clone()) {
                continue;
            }
            if let Some((more, _)) = decls.get(&cur) {
                stack.extend(more.iter().map(|i| i.name.clone()));
            }
        }
    }

    // Topics bound to a transport in `bindings { }` have no gate; the
    // api entry's own source, and the main locus's params (a source
    // may be `self.<param>`).
    let mut bound: BTreeSet<String> = BTreeSet::new();
    let mut api_roles: Option<ApiRoles> = None;
    let mut api_span: Option<Span> = None;
    let mut main_params: Vec<(String, TypeExpr)> = Vec::new();
    for p in programs {
        walk_decls(&p.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                for m in &l.members {
                    if let LocusMember::Bindings(bb) = m {
                        for e in &bb.entries {
                            bound.insert(e.topic.name.clone());
                        }
                        if let Some(api) = &bb.api {
                            api_span = Some(api.span);
                            if let Some(r) = &api.roles {
                                api_roles = Some(r.clone());
                            }
                            for pm in &l.members {
                                if let LocusMember::Params(pb) = pm {
                                    for prm in &pb.params {
                                        if let Some(t) = &prm.ty {
                                            main_params.push((prm.name.name.clone(), t.clone()));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // Review F3: a gate on a free fn is an error — nothing there is
    // reached from the binding — and its role is checked all the same.
    for p in programs {
        walk_decls(&p.items, &mut |item| {
            if let TopDecl::Fn(f) = item {
                if let Some(g) = &f.gated {
                    diags.push(Diag::ty(
                        g.span,
                        format!(
                            "`@gated(role: {})` on the free fn `{}`: a gate goes on a subscribed \
                             handler, an `expose` member or a `publish` — it is checked at the api \
                             binding, and a free fn is never reached from there",
                            g.name, f.name.name
                        ),
                    ));
                    if !declared(&g.name) {
                        diags.push(undeclared(g, &format!("`@gated` on `{}`", f.name.name)));
                    }
                }
            }
        });
    }

    // The sites, and the gate each topic's subscribers and publishers
    // state: (site, role, span).
    let mut sub_gates: BTreeMap<String, Vec<(String, Option<String>, Span)>> = BTreeMap::new();
    let mut pub_gates: BTreeMap<String, Vec<(String, Option<String>, Span)>> = BTreeMap::new();
    let mut loci: BTreeMap<String, &hale_syntax::ast::LocusDecl> = BTreeMap::new();
    for p in programs {
        walk_decls(&p.items, &mut |item| {
            let TopDecl::Locus(l) = item else { return };
            if l.name.name.starts_with("__Api") {
                return;
            }
            loci.insert(l.name.name.clone(), l);
            // An imported locus is not reached from the binding, so its
            // gates (or their absence) say nothing about the entrypoint's.
            if l.imported {
                return;
            }
            // Every subscription, by handler: one handler may subscribe
            // several topics (review F5), and each is a site.
            let mut subscribed: Vec<(&str, Option<String>)> = Vec::new();
            for m in &l.members {
                let LocusMember::Bus(bb) = m else { continue };
                for bm in &bb.members {
                    match bm {
                        BusMember::Subscribe { subject, handler, .. } => {
                            let topic = match subject {
                                BusSubject::Topic(id) => Some(id.name.clone()),
                                _ => None,
                            };
                            subscribed.push((handler.name.as_str(), topic));
                        }
                        BusMember::Publish { subject, gated, span, .. } => {
                            if let Some(g) = gated {
                                if !declared(&g.name) {
                                    diags.push(undeclared(g, &format!("`@gated` on `{}`'s publish", l.name.name)));
                                }
                            }
                            if let BusSubject::Topic(id) = subject {
                                pub_gates.entry(id.name.clone()).or_default().push((
                                    format!("{} publishes it", l.name.name),
                                    gated.as_ref().map(|g| g.name.clone()),
                                    *span,
                                ));
                            }
                        }
                    }
                }
            }
            for m in &l.members {
                match m {
                    LocusMember::Fn(f) => {
                        let Some(g) = &f.gated else { continue };
                        if !declared(&g.name) {
                            diags.push(undeclared(g, &format!("`@gated` on `{}.{}`", l.name.name, f.name.name)));
                        }
                        let mine: Vec<&Option<String>> = subscribed
                            .iter()
                            .filter(|(h, _)| *h == f.name.name.as_str())
                            .map(|(_, t)| t)
                            .collect();
                        if mine.is_empty() {
                            diags.push(Diag::ty(
                                g.span,
                                format!(
                                    "`@gated(role: {})` on `{}.{}`, which no `subscribe` line of \
                                     `{}` names: a gate goes on a subscribed handler, an `expose` \
                                     member or a `publish` — it is checked at the binding, and a \
                                     plain method is never reached from there",
                                    g.name, l.name.name, f.name.name, l.name.name
                                ),
                            ));
                        }
                        for topic in mine.into_iter().flatten() {
                            if bound.contains(topic) {
                                diags.push(Diag::ty(
                                    g.span,
                                    format!(
                                        "`@gated(role: {})` on `{}.{}`, but its topic `{}` is \
                                         bound to a transport in `bindings {{ }}` that has no \
                                         gate: a message from another process would reach the \
                                         handler unchecked. Reach the topic through the api \
                                         binding, or drop the annotation",
                                        g.name, l.name.name, f.name.name, topic
                                    ),
                                ));
                            }
                            sub_gates.entry(topic.clone()).or_default().push((
                                format!("{}.{}", l.name.name, f.name.name),
                                Some(g.name.clone()),
                                g.span,
                            ));
                        }
                    }
                    LocusMember::Contract(cb) => {
                        let ContractKind::Members(members) = &cb.kind else { continue };
                        for cm in members {
                            let Some(g) = &cm.gated else { continue };
                            if cm.direction != ContractDirection::Expose {
                                continue;
                            }
                            if !declared(&g.name) {
                                diags.push(undeclared(g, &format!("`@gated` on an `expose` of `{}`", l.name.name)));
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Ungated subscribers count too: every subscriber of a
            // topic must agree.
            for (h, topic) in &subscribed {
                let Some(t) = topic else { continue };
                let gated = l.members.iter().any(|m| matches!(m, LocusMember::Fn(f) if f.name.name == *h && f.gated.is_some()));
                if !gated {
                    let span = l
                        .members
                        .iter()
                        .find_map(|m| match m {
                            LocusMember::Fn(f) if f.name.name == *h => Some(f.name.span),
                            _ => None,
                        })
                        .unwrap_or(l.name.span);
                    sub_gates.entry(t.clone()).or_default().push((format!("{}.{}", l.name.name, h), None, span));
                }
            }
        });
    }
    for (kind, gates) in [("subscribes", &sub_gates), ("publishes", &pub_gates)] {
        for (topic, sites) in gates {
            let distinct: BTreeSet<Option<String>> = sites.iter().map(|(_, r, _)| r.clone()).collect();
            if distinct.len() < 2 {
                continue;
            }
            let listed: Vec<String> = sites
                .iter()
                .map(|(site, r, _)| match r {
                    Some(r) => format!("{} gated `{}`", site, r),
                    None => format!("{} ungated", site),
                })
                .collect();
            let (_, _, span) = sites.iter().find(|(_, r, _)| r.is_some()).unwrap_or(&sites[0]);
            diags.push(Diag::ty(
                *span,
                format!(
                    "topic `{}`: {} — every locus that {} one topic states the same gate, \
                     because the binding refuses the message, not the handler",
                    topic,
                    listed.join(", "),
                    kind
                ),
            ));
        }
    }

    // A program-named source is a locus satisfying std::api::RoleSource
    // (review F6): a locus literal or `self.<param>` of the main locus
    // is checked structurally here, with the fn's span; any other
    // expression is typed against the interface at the generated init.
    let Some(src) = api_roles else { return };
    let named = |path: &hale_syntax::ast::QualifiedName| -> String {
        path.segments.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("::")
    };
    let locus_name: Option<String> = match &src.expr {
        Expr::Struct { path, .. } => Some(named(path)),
        Expr::Field { receiver, name, .. } if matches!(**receiver, Expr::KwSelf(_)) => main_params
            .iter()
            .find(|(n, _)| *n == name.name)
            .and_then(|(_, t)| match t {
                TypeExpr::Named { path, .. } => Some(named(path)),
                _ => None,
            }),
        _ => None,
    };
    let Some(locus_name) = locus_name else { return };
    if locus_name.starts_with("__Std") || locus_name.starts_with("std::") {
        return;
    }
    // A qualified path (`lib::TableRoles`) is renamed to the imported
    // locus's mangled name only on the build path; here the generated
    // init is typed against the interface, which is check enough.
    if locus_name.contains("::") && !loci.contains_key(&locus_name) {
        return;
    }
    let entry = api_span.unwrap_or(src.span);
    let Some(l) = loci.get(&locus_name) else {
        diags.push(Diag::ty(
            src.span,
            format!(
                "api binding: `roles:` names `{}`, which is no locus of this bundle; a role \
                 source is a locus with `fn holds(p: std::api::Principal, r: String) -> Bool` \
                 (std::api::RoleSource)",
                locus_name
            ),
        ));
        return;
    };
    let holds = l.members.iter().find_map(|m| match m {
        LocusMember::Fn(f) if f.name.name == "holds" => Some(f),
        _ => None,
    });
    let Some(f) = holds else {
        diags.push(Diag::ty(
            src.span,
            format!(
                "api binding: `roles:` names `{}`, which has no `fn holds`: a role source \
                 answers `fn holds(p: std::api::Principal, r: String) -> Bool` \
                 (std::api::RoleSource) — whether the principal holds the role directly; \
                 the binding walks `includes` itself",
                locus_name
            ),
        ));
        return;
    };
    let is_principal = |t: &TypeExpr| match t {
        TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            segs == ["std", "api", "Principal"] || segs == ["__StdApiPrincipal"]
        }
        _ => false,
    };
    let mut why: Vec<String> = Vec::new();
    if f.params.len() != 2 {
        why.push(format!("takes {} parameter(s), not 2", f.params.len()));
    } else {
        if !is_principal(&f.params[0].ty) {
            why.push(format!("its first parameter is not a `std::api::Principal`"));
        }
        if !matches!(&f.params[1].ty, TypeExpr::Primitive(PrimType::String, _)) {
            why.push(format!("its second parameter is not a `String`"));
        }
    }
    if !matches!(&f.ret, Some(TypeExpr::Primitive(PrimType::Bool, _))) {
        why.push("it does not return `Bool`".to_string());
    }
    if f.fallible.is_some() {
        why.push("it is fallible".to_string());
    }
    if !why.is_empty() {
        diags.push(
            Diag::ty(
                f.name.span,
                format!(
                    "`{}.holds` does not satisfy std::api::RoleSource: {} — a role source \
                     answers `fn holds(p: std::api::Principal, r: String) -> Bool`, not \
                     fallible, whether the principal holds the role directly (the binding \
                     walks `includes` itself)",
                    locus_name,
                    why.join("; ")
                ),
            )
            .with_related(entry, "the api entry that names it"),
        );
    }
}

/// Walk the bundle and collect, per topic name (the binding-side
/// identifier), the set of topics that have at least one publisher
/// and the set that have at least one subscriber across all loci.
/// Used by role-inference validation in `check_main_and_bindings`.
fn collect_topic_pub_sub(
    bundle: &Bundle<'_>,
) -> (
    std::collections::BTreeSet<String>,
    std::collections::BTreeSet<String>,
) {
    let mut pubs = std::collections::BTreeSet::new();
    let mut subs = std::collections::BTreeSet::new();
    fn walk(
        items: &[TopDecl],
        pubs: &mut std::collections::BTreeSet<String>,
        subs: &mut std::collections::BTreeSet<String>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for member in &l.members {
                        if let LocusMember::Bus(bb) = member {
                            for bm in &bb.members {
                                match bm {
                                    BusMember::Publish { subject, .. } => {
                                        if let BusSubject::Topic(id) = subject {
                                            pubs.insert(id.name.clone());
                                        }
                                    }
                                    BusMember::Subscribe { subject, .. } => {
                                        if let BusSubject::Topic(id) = subject {
                                            subs.insert(id.name.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, pubs, subs),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &mut pubs, &mut subs);
    }
    (pubs, subs)
}

// === Bus-graph property checks (GH #18 #4) =========================
//
// The bus topology is a typed directed graph already in the AST.
// PR A walks it for ORPHANs — a subject wired to only one end. Gated
// on a closed-world program (a `main` locus present): a library seed
// whose publishers/subscribers live in downstream consumers must not
// be flagged, since the other half is out of this bundle.
//
// Subjects are keyed by `BusSubject::canonical()` (literal string /
// topic name / qualified last segment), which is exactly the key a
// declared topic's name matches. False-positive guards: transport
// bindings (external peer), trailing-`**` wildcard coverage, and
// cross-seed (`alias::Foo`) references (the other seed owns the other
// half). A declared topic is matched by both its name and its
// `wire_subject` (a literal site may address it by the wire form).

fn check_bus_graph(
    bundle: &Bundle<'_>,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    // Closed-world gate: only a complete program (has `main`) has
    // both ends of every channel in-bundle.
    let has_main = bundle.programs.values().any(|p| {
        p.items
            .iter()
            .any(|i| matches!(i, TopDecl::Locus(l) if l.is_main))
    });
    if !has_main {
        return;
    }

    // The publisher/subscriber/bound/cross-seed walk is shared with
    // `bus_graph::build_bus_graph` (the static-devirt analysis) — one
    // walk, two consumers. The orphan diagnostics below use only the
    // ends + bound + cross_seed; the per-site detail is ignored here.
    let crate::bus_graph::BusWalk {
        publishers,
        subscribers,
        bound,
        cross_seed,
        ..
    } = crate::bus_graph::collect_bus_walk(bundle);

    // A subject has a publisher if some locus publishes it (exactly
    // or via wildcard), it is bound to a transport (external peer),
    // or it's referenced cross-seed. Same for subscriber.
    // GH #1106: an api binding makes every subscribed topic a command a
    // caller may publish and every published topic a stream a caller
    // may subscribe, so neither half of this lint applies under one.
    let api_bound = bundle.programs.values().any(|p| {
        let mut found = false;
        walk_decls(&p.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                if l.is_main
                    && l.members.iter().any(|m| {
                        matches!(m, LocusMember::Bindings(bb) if bb.api.is_some())
                    })
                {
                    found = true;
                }
            }
        });
        found
    });
    let has_pub = |aliases: &[&str]| {
        api_bound
            || aliases.iter().any(|a| {
                publishers.covers(a) || bound.contains(*a) || cross_seed.contains(*a)
            })
    };
    let has_sub = |aliases: &[&str]| {
        api_bound
            || aliases.iter().any(|a| {
                subscribers.covers(a) || bound.contains(*a) || cross_seed.contains(*a)
            })
    };

    // 1) Declared topics — matched by name and wire_subject.
    let mut declared_keys: BTreeSet<String> = BTreeSet::new();
    for (name, sym) in &top.symbols {
        let TopSymbol::Topic(info) = sym else { continue };
        // Topics that failed parent resolution carry an empty wire
        // subject and already have a diagnostic — skip.
        if info.wire_subject.is_empty() {
            continue;
        }
        declared_keys.insert(name.clone());
        declared_keys.insert(info.wire_subject.clone());
        let aliases: Vec<&str> = if info.wire_subject == *name {
            vec![name.as_str()]
        } else {
            vec![name.as_str(), info.wire_subject.as_str()]
        };
        let p = has_pub(&aliases);
        let s = has_sub(&aliases);
        if p && !s {
            let span = publishers
                .concrete
                .get(name)
                .or_else(|| publishers.concrete.get(&info.wire_subject))
                .copied()
                .unwrap_or(info.span);
            diags.push(Diag::warn(
                span,
                format!(
                    "bus topic `{}` is published but has no subscriber — \
                     the cells go nowhere. Add a `subscribe` for it, bind it \
                     to a transport, or drop the publish.",
                    name
                ),
            ));
        } else if s && !p {
            let span = subscribers
                .concrete
                .get(name)
                .or_else(|| subscribers.concrete.get(&info.wire_subject))
                .copied()
                .unwrap_or(info.span);
            diags.push(Diag::warn(
                span,
                format!(
                    "bus topic `{}` is subscribed but never published — its \
                     handler can't fire. Add a `publish` for it, bind it to a \
                     transport, or drop the subscription.",
                    name
                ),
            ));
        } else if !p && !s {
            diags.push(Diag::warn(
                info.span,
                format!(
                    "bus topic `{}` is declared but neither published nor \
                     subscribed — it's dead wiring.",
                    name
                ),
            ));
        }
    }

    // 2) Literal subjects (not a declared topic's name or wire form).
    let mut literal_keys: BTreeSet<String> = BTreeSet::new();
    for k in publishers.concrete.keys().chain(subscribers.concrete.keys()) {
        if !declared_keys.contains(k) {
            literal_keys.insert(k.clone());
        }
    }
    for k in literal_keys {
        let aliases = [k.as_str()];
        let p = has_pub(&aliases);
        let s = has_sub(&aliases);
        if p && !s {
            let span = publishers.concrete.get(&k).copied().unwrap();
            diags.push(Diag::warn(
                span,
                format!(
                    "bus subject `\"{}\"` is published but has no subscriber — \
                     the cells go nowhere. Add a `subscribe`, bind it to a \
                     transport, or drop the publish.",
                    k
                ),
            ));
        } else if s && !p {
            let span = subscribers.concrete.get(&k).copied().unwrap();
            diags.push(Diag::warn(
                span,
                format!(
                    "bus subject `\"{}\"` is subscribed but never published — \
                     its handler can't fire. Add a `publish`, bind it to a \
                     transport, or drop the subscription.",
                    k
                ),
            ));
        }
    }

    check_wildcard_publish_payloads(top, diags);
}

/// A wildcard publish declaration authorizes its locus to publish any
/// subject under the pattern, with the declared payload type. So every
/// subscription whose subject falls under that pattern must expect
/// that same payload — otherwise a computed publish inside the
/// pattern lands on a handler that reinterprets the bytes as its own
/// type.
///
/// The runtime guard on computed subjects (`lower_send`) confines a
/// publish to its declared patterns, which closes the case of a
/// foreign subject. This closes the complement: a subject INSIDE the
/// pattern whose subscriber disagrees about the payload. Both are the
/// same defect — a cell delivered to a handler typed for something
/// else — and neither is caught by the per-subject payload agreement
/// check, which only compares publishers and subscribers that name
/// the same concrete subject.
///
/// Checked statically because the closed world makes it decidable:
/// the subscription subjects are all declared. A cross-process
/// subscriber is the fleet tier's job (artifact composition already
/// compares payload shapes per subject).
fn check_wildcard_publish_payloads(
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    // Wildcard publish declarations: (pattern, payload, locus, span).
    let mut patterns: Vec<(&str, &Ty, &str, Span)> = Vec::new();
    for (lname, sym) in &top.symbols {
        let TopSymbol::Locus(l) = sym else { continue };
        for p in &l.bus_publishes {
            if p.subject.contains("**") {
                patterns.push((
                    p.subject.as_str(),
                    &p.payload,
                    lname.as_str(),
                    p.span,
                ));
            }
        }
    }
    if patterns.is_empty() {
        return;
    }
    // Every subscription's concrete wire subject, with the payload its
    // handler expects. A topic-form subscribe resolves through the
    // topic's wire subject; a literal-form one is already concrete.
    let mut subs: Vec<(String, &Ty, &str, Span)> = Vec::new();
    for (lname, sym) in &top.symbols {
        let TopSymbol::Locus(l) = sym else { continue };
        for s in &l.bus_subscribes {
            if s.subject.contains("**") {
                // Wildcard-vs-wildcard overlap is not a delivery
                // fact on its own; skip.
                continue;
            }
            let wire = match top.symbols.get(&s.subject) {
                Some(TopSymbol::Topic(t))
                    if !t.wire_subject.is_empty() =>
                {
                    t.wire_subject.clone()
                }
                _ => s.subject.clone(),
            };
            subs.push((wire, &s.payload, lname.as_str(), s.span));
        }
    }
    for (pat, ppay, plocus, pspan) in &patterns {
        for (wire, spay, slocus, sspan) in &subs {
            if !crate::wildcard_match(pat, wire) {
                continue;
            }
            if ppay.assignable_from(spay) || spay.assignable_from(ppay)
            {
                continue;
            }
            // Report on the subscription — that is the end whose
            // handler would receive the wrong shape — and name the
            // declaration that can reach it.
            //
            // A WARNING, not an error, because the hazard is
            // conditional: the pattern's owner may never publish a
            // subject that reaches here (a `std::log::Logger`
            // declares all of `log.**` but publishes only under its
            // own path). Making it fatal would refuse
            // programs in which the publish cannot happen. The
            // publish site enforces it for real — a computed send
            // whose payload disagrees with a matching subscriber is
            // refused there, where the subject is finally known.
            diags.push(Diag::warn(
                *sspan,
                format!(
                    "bus subject `\"{}\"` is subscribed by locus `{}` \
                     expecting payload `{}`, but locus `{}` declares \
                     `publish \"{}\"`, whose payload is `{}`. If that \
                     locus publishes a computed subject reaching \
                     `\"{}\"`, the send is refused at runtime rather \
                     than delivered as `{}`. Give the pattern a \
                     payload this subscriber accepts, narrow the \
                     pattern, or move the subscription out from under \
                     it.",
                    wire,
                    slocus,
                    spay.display(),
                    plocus,
                    pat,
                    ppay.display(),
                    wire,
                    spay.display()
                ),
            ));
            let _ = pspan;
        }
    }
}

// === Bus-graph cycles (GH #18 #4, PR B) ============================
//
// Edges of the bus graph: when locus `L` subscribes subject `S` with
// handler `H`, and `H`'s body sends to subject `D`, that's an edge
// `S →(L) D` — a cell on `S` can cause a cell on `D`. A cycle in this
// graph is a publish→subscribe→publish loop.
//
// The dispatch model splits the two outcomes:
//   - A **cross-locus** cycle (edges from ≥2 loci) hops between loci
//     via the cooperative *queue* (drained at yield) — it spins the
//     queue / livelocks → WARNING.
//   - An **intra-locus** cycle (all edges in one locus) is
//     intra-locus self-dispatch, which is **devirtualized to a direct
//     synchronous call** (spec/semantics.md), so it recurses on one
//     thread without bound → stack overflow → ERROR.
// The error stays on the provably-synchronous intra-locus case only,
// matching the error-precision discipline used elsewhere.

/// The subject a `Topic <- v` send addresses, as a canonical key
/// (matching `BusSubject::canonical`): a string literal, a bare topic
/// name, or a qualified path's last segment. None for computed
/// subjects (not statically traceable).
fn send_subject_key(e: &Expr) -> Option<String> {
    match e {
        Expr::Literal(Literal::String(s), _) => Some(s.clone()),
        Expr::Ident(id) => Some(id.name.clone()),
        Expr::Path(qn) => qn.segments.last().map(|s| s.name.clone()),
        _ => None,
    }
}

/// Collect the subjects a handler/`run()` body sends to (the targets
/// of `Topic <- value`). When `descend_cond` is false, sends nested
/// inside `if`/`match`/`for`/`while` are skipped — leaving only the
/// **unconditional** sends that fire on every execution. The
/// intra-locus error uses the unconditional set (a guarded
/// self-republish is a terminating state machine, not unbounded
/// recursion); the cross-locus warning uses all sends.
fn collect_sends_in_block(
    b: &Block,
    descend_cond: bool,
    out: &mut Vec<(String, Span)>,
) {
    for s in &b.stmts {
        collect_sends_in_stmt(s, descend_cond, out);
    }
}

fn collect_sends_in_stmt(
    stmt: &Stmt,
    descend_cond: bool,
    out: &mut Vec<(String, Span)>,
) {
    match stmt {
        Stmt::Send { subject, span, .. } => {
            if let Some(k) = send_subject_key(subject) {
                out.push((k, *span));
            }
        }
        Stmt::If(i) if descend_cond => collect_sends_in_if(i, out),
        Stmt::Match(m) if descend_cond => {
            for arm in &m.arms {
                if let MatchArmBody::Block(b) = &arm.body {
                    collect_sends_in_block(b, descend_cond, out);
                }
            }
        }
        Stmt::For { body, .. } | Stmt::While { body, .. }
            if descend_cond =>
        {
            collect_sends_in_block(body, descend_cond, out)
        }
        // A plain `{ ... }` block always executes — its sends stay
        // unconditional regardless of `descend_cond`.
        Stmt::Block(b) => collect_sends_in_block(b, descend_cond, out),
        _ => {}
    }
}

fn collect_sends_in_if(i: &IfStmt, out: &mut Vec<(String, Span)>) {
    collect_sends_in_block(&i.then_block, true, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => collect_sends_in_block(b, true, out),
        Some(ElseBranch::ElseIf(n)) => collect_sends_in_if(n, out),
        None => {}
    }
}

/// One directed edge `from → to`, tagged with the producing locus and
/// the send-site span for diagnostics.
#[derive(Clone)]
struct BusEdge {
    to: String,
    locus: String,
    span: Span,
}

type BusAdj = BTreeMap<String, Vec<BusEdge>>;

/// DFS for a cycle; returns the node sequence `[a, …, a]` of the first
/// cycle found, or None. Colors: 0 white, 1 gray (on stack), 2 black.
fn dfs_bus_cycle(
    node: &str,
    adj: &BusAdj,
    color: &mut BTreeMap<String, u8>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    color.insert(node.to_string(), 1);
    path.push(node.to_string());
    if let Some(edges) = adj.get(node) {
        for e in edges {
            match color.get(&e.to).copied().unwrap_or(0) {
                1 => {
                    let start =
                        path.iter().position(|n| n == &e.to).unwrap_or(0);
                    let mut cyc = path[start..].to_vec();
                    cyc.push(e.to.clone());
                    return Some(cyc);
                }
                0 => {
                    if let Some(c) = dfs_bus_cycle(&e.to, adj, color, path) {
                        return Some(c);
                    }
                }
                _ => {}
            }
        }
    }
    path.pop();
    color.insert(node.to_string(), 2);
    None
}

/// The set of loci whose edges realize `cyc`, plus a representative
/// send span (the first edge's).
fn cycle_loci(cyc: &[String], adj: &BusAdj) -> (BTreeSet<String>, Span) {
    let mut loci = BTreeSet::new();
    let mut span = Span::new(0, 0);
    let mut first = true;
    for w in cyc.windows(2) {
        if let Some(edges) = adj.get(&w[0]) {
            if let Some(e) = edges.iter().find(|e| e.to == w[1]) {
                loci.insert(e.locus.clone());
                if first {
                    span = e.span;
                    first = false;
                }
            }
        }
    }
    (loci, span)
}

fn check_bus_cycles(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    // Build the edge set. For each locus, map its subscribe handlers
    // by name, then for each subscribed subject add edges to whatever
    // the handler body sends.
    let mut global: BusAdj = BTreeMap::new();
    // Per-locus adjacency (only that locus's edges) for the intra
    // (synchronous) cycle check.
    let mut per_locus: BTreeMap<String, BusAdj> = BTreeMap::new();

    fn walk_loci<'a>(
        items: &'a [TopDecl],
        global: &mut BusAdj,
        per_locus: &mut BTreeMap<String, BusAdj>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    // handler name -> body
                    let mut handler_bodies: BTreeMap<&str, &Block> =
                        BTreeMap::new();
                    let mut subs: Vec<(String, String)> = Vec::new(); // (subject, handler)
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(f) => {
                                handler_bodies.insert(f.name.name.as_str(), &f.body);
                            }
                            LocusMember::Bus(bb) => {
                                for bm in &bb.members {
                                    if let BusMember::Subscribe {
                                        subject, handler, ..
                                    } = bm
                                    {
                                        subs.push((
                                            subject.canonical().to_string(),
                                            handler.name.clone(),
                                        ));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    let lname = l.name.name.clone();
                    for (subject, handler) in subs {
                        let Some(body) = handler_bodies.get(handler.as_str())
                        else {
                            continue;
                        };
                        // Cross-locus warning: every send (incl. guarded).
                        let mut sends_all = Vec::new();
                        collect_sends_in_block(body, true, &mut sends_all);
                        for (to, span) in sends_all {
                            global.entry(subject.clone()).or_default().push(
                                BusEdge { to, locus: lname.clone(), span },
                            );
                        }
                        // Intra-locus error: only unconditional sends —
                        // a guarded self-republish terminates.
                        let mut sends_uncond = Vec::new();
                        collect_sends_in_block(body, false, &mut sends_uncond);
                        for (to, span) in sends_uncond {
                            per_locus
                                .entry(lname.clone())
                                .or_default()
                                .entry(subject.clone())
                                .or_default()
                                .push(BusEdge { to, locus: lname.clone(), span });
                        }
                    }
                }
                TopDecl::Module(md) => walk_loci(&md.items, global, per_locus),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk_loci(&program.items, &mut global, &mut per_locus);
    }

    // 1) Intra-locus cycles → error (one per locus). Sound because
    //    intra-locus self-dispatch is devirtualized synchronous.
    let mut intra_loci: BTreeSet<String> = BTreeSet::new();
    for (lname, adj) in &per_locus {
        let roots: Vec<String> = adj.keys().cloned().collect();
        for root in roots {
            let mut color = BTreeMap::new();
            let mut path = Vec::new();
            if let Some(cyc) = dfs_bus_cycle(&root, adj, &mut color, &mut path) {
                let (_, span) = cycle_loci(&cyc, adj);
                diags.push(Diag::ty(
                    span,
                    format!(
                        "locus `{}` has a re-entrant synchronous bus cycle \
                         `{}`: each publish onto a topic the locus also \
                         subscribes is a direct in-thread call (intra-locus \
                         self-dispatch), so this recurses without bound and \
                         overflows the stack. Break the cycle, or route one \
                         hop through a different pool (an async enqueue).",
                        lname,
                        cyc.join(" → "),
                    ),
                ));
                intra_loci.insert(lname.clone());
                break;
            }
        }
    }

    // 2) Cross-locus cycles → warning. Exclude edges from loci that
    //    already have an intra-locus error so those don't shadow a
    //    genuine cross-locus loop.
    let mut cross_adj: BusAdj = BTreeMap::new();
    for (from, edges) in &global {
        for e in edges {
            if !intra_loci.contains(&e.locus) {
                cross_adj
                    .entry(from.clone())
                    .or_default()
                    .push(e.clone());
            }
        }
    }
    let mut reported: BTreeSet<String> = BTreeSet::new();
    let roots: Vec<String> = cross_adj.keys().cloned().collect();
    for root in roots {
        let mut color = BTreeMap::new();
        let mut path = Vec::new();
        if let Some(cyc) = dfs_bus_cycle(&root, &cross_adj, &mut color, &mut path)
        {
            let (loci, span) = cycle_loci(&cyc, &cross_adj);
            if loci.len() < 2 {
                continue;
            }
            let mut nodes: Vec<String> = cyc.clone();
            nodes.sort();
            nodes.dedup();
            let key = nodes.join("|");
            if !reported.insert(key) {
                continue;
            }
            diags.push(Diag::warn(
                span,
                format!(
                    "bus cycle `{}` across loci ({}): a cell can re-trigger \
                     its own publish, spinning the cooperative queue. Break \
                     the loop or add a terminating condition.",
                    cyc.join(" → "),
                    loci.into_iter().collect::<Vec<_>>().join(", "),
                ),
            ));
        }
    }
}

// === Bus backpressure (GH #18 #4) ==================================
//
// A producer with no flow control floods the bus without bound. A
// full "consumer can't sustain the rate" analysis is undecidable, so
// this is a deliberately narrow structural heuristic for the clearest
// case: an UNBOUNDED loop (`while true`) that publishes on some
// iteration but contains no flow-control or exit point — no
// cooperative `yield` (which lets a co-scheduled consumer drain), no
// `time::sleep`/`tick` throttle, no input-pacing blocking `recv`, and
// no `break`/`return` that could exit. Such a loop posts cells faster
// than anything can drain them — the queue and the payload arena grow
// without bound. A warning (the heuristic can't prove the OOM, only
// flag the missing backpressure). Bounded loops (`for`, `while
// cond`) are never flagged — only literal `while true`.

/// Flow-control / exit primitives whose presence anywhere in an
/// unbounded loop body rules out the flood: the loop either paces
/// #353: does this block ALWAYS leave without producing a value?
///
/// A fallback that diverges has no type, so `or { break; }` must not
/// be asked to match the success type. Conservative: only a block
/// whose LAST statement unconditionally transfers control counts, so
/// a conditional `break` still requires a substitute.
fn block_always_diverges(b: &Block) -> bool {
    if b.tail.is_some() {
        return false;
    }
    matches!(
        b.stmts.last(),
        Some(
            Stmt::Break(_)
                | Stmt::Continue(_)
                | Stmt::Return(..)
                | Stmt::Fail { .. }
                | Stmt::Terminate(_)
        )
    )
}

/// itself or can leave.
fn block_has_flow_control(b: &Block) -> bool {
    b.stmts.iter().any(stmt_has_flow_control)
}

fn stmt_has_flow_control(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Yield(_) | Stmt::Break(_) | Stmt::Return(..) => true,
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            expr_has_flow_control_call(value)
        }
        Stmt::Assign { value, .. } => expr_has_flow_control_call(value),
        Stmt::Expr(e) => expr_has_flow_control_call(e),
        Stmt::Send { subject, value, .. } => {
            expr_has_flow_control_call(subject)
                || expr_has_flow_control_call(value)
        }
        Stmt::If(i) => if_has_flow_control(i),
        Stmt::Match(m) => m.arms.iter().any(|arm| match &arm.body {
            MatchArmBody::Block(b) => block_has_flow_control(b),
            MatchArmBody::Expr(e) => expr_has_flow_control_call(e),
        }),
        Stmt::For { body, .. } | Stmt::While { body, .. } => {
            block_has_flow_control(body)
        }
        Stmt::Block(b) => block_has_flow_control(b),
        _ => false,
    }
}

fn if_has_flow_control(i: &IfStmt) -> bool {
    block_has_flow_control(&i.then_block)
        || match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => block_has_flow_control(b),
            Some(ElseBranch::ElseIf(n)) => if_has_flow_control(n),
            None => false,
        }
}

/// A call to a throttle (`time::sleep`/`tick`) or an input-pacing
/// blocking op (`recv`/`accept`/`wait`) — both bound the publish rate.
fn expr_has_flow_control_call(expr: &Expr) -> bool {
    match expr {
        Expr::Call { callee, args, .. } => {
            if let Expr::Path(qn) = callee.as_ref() {
                let segs: Vec<&str> =
                    qn.segments.iter().map(|s| s.name.as_str()).collect();
                if blocking_path_match(&segs).is_some()
                    || segs == ["std", "time", "sleep"]
                    || segs == ["std", "time", "tick"]
                {
                    return true;
                }
            }
            expr_has_flow_control_call(callee)
                || args.iter().any(expr_has_flow_control_call)
        }
        Expr::Binary { left, right, .. } => {
            expr_has_flow_control_call(left)
                || expr_has_flow_control_call(right)
        }
        Expr::Unary { operand, .. } => expr_has_flow_control_call(operand),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            expr_has_flow_control_call(receiver)
        }
        Expr::Index { receiver, index, .. } => {
            expr_has_flow_control_call(receiver)
                || expr_has_flow_control_call(index)
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            es.iter().any(expr_has_flow_control_call)
        }
        Expr::Struct { inits, .. } => {
            inits.iter().any(|i| expr_has_flow_control_call(&i.value))
        }
        Expr::Block(b) => block_has_flow_control(b),
        Expr::If(i) => {
            block_has_flow_control(&i.then_block)
                || matches!(i.else_block.as_deref(), Some(ElseBranch::Else(b)) if block_has_flow_control(b))
        }
        Expr::Match(m) => m.arms.iter().any(|arm| match &arm.body {
            MatchArmBody::Block(b) => block_has_flow_control(b),
            MatchArmBody::Expr(e) => expr_has_flow_control_call(e),
        }),
        Expr::Sum(e, _) | Expr::Prod(e, _) => expr_has_flow_control_call(e),
        Expr::Or { inner, .. } => expr_has_flow_control_call(inner),
        _ => false,
    }
}

/// Whether a block subtree contains a bus `Topic <- value` send.
fn block_has_send(b: &Block) -> bool {
    b.stmts.iter().any(stmt_has_send)
}

fn stmt_has_send(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Send { .. } => true,
        Stmt::If(i) => if_has_send(i),
        Stmt::Match(m) => m.arms.iter().any(|arm| {
            matches!(&arm.body, MatchArmBody::Block(b) if block_has_send(b))
        }),
        Stmt::For { body, .. } | Stmt::While { body, .. } => block_has_send(body),
        Stmt::Block(b) => block_has_send(b),
        _ => false,
    }
}

fn if_has_send(i: &IfStmt) -> bool {
    block_has_send(&i.then_block)
        || match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => block_has_send(b),
            Some(ElseBranch::ElseIf(n)) => if_has_send(n),
            None => false,
        }
}

fn is_literal_true(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::Bool(true), _))
}

/// Walk a method/lifecycle body for unbounded publish-flood loops.
fn scan_flood_in_block(b: &Block, locus: &str, diags: &mut Vec<Diag>) {
    for s in &b.stmts {
        scan_flood_in_stmt(s, locus, diags);
    }
}

fn scan_flood_in_stmt(stmt: &Stmt, locus: &str, diags: &mut Vec<Diag>) {
    match stmt {
        Stmt::While { cond, body, span } if is_literal_true(cond) => {
            if block_has_send(body) && !block_has_flow_control(body) {
                diags.push(Diag::warn(
                    *span,
                    format!(
                        "locus `{}` publishes to the bus inside an unbounded \
                         `while true` loop with no flow control — no `yield`, \
                         `time::sleep`/`tick`, input-pacing `recv`, or \
                         `break`/`return`. The producer has no backpressure, \
                         so cells pile up in the queue (and the payload arena) \
                         without bound. Pace the loop (a `time::sleep`/`tick`), \
                         drive it from an input (a blocking `recv` so the \
                         publish rate follows the input rate), or `yield` to \
                         let a co-scheduled subscriber drain.",
                        locus
                    ),
                ));
                // Reported the outermost flood; don't descend further.
            } else {
                scan_flood_in_block(body, locus, diags);
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            scan_flood_in_block(body, locus, diags)
        }
        Stmt::If(i) => scan_flood_in_if(i, locus, diags),
        Stmt::Match(m) => {
            for arm in &m.arms {
                if let MatchArmBody::Block(b) = &arm.body {
                    scan_flood_in_block(b, locus, diags);
                }
            }
        }
        Stmt::Block(b) => scan_flood_in_block(b, locus, diags),
        _ => {}
    }
}

fn scan_flood_in_if(i: &IfStmt, locus: &str, diags: &mut Vec<Diag>) {
    scan_flood_in_block(&i.then_block, locus, diags);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => scan_flood_in_block(b, locus, diags),
        Some(ElseBranch::ElseIf(n)) => scan_flood_in_if(n, locus, diags),
        None => {}
    }
}

// === Bus subject type-mismatch (GH #18 #4) ========================
//
// A *declared* `topic` fixes its payload type once, so every `publish
// Foo` / `subscribe Foo` site is unified by the declaration — no
// mismatch possible (and `of type` is forbidden on topic refs). The
// hole is *literal* subjects (`publish "wire.sig" of type Tick`):
// nothing ties the `of type` annotations at two sites on the same
// wire string together, so a publisher's `Tick` and a subscriber's
// `Pulse` both compile — and at runtime the subscriber decodes the
// publisher's bytes as the wrong type. That is a hard correctness
// bug, so this is an **error**.
//
// Grouping is by EXACT subject string, which deliberately sidesteps
// wildcards (`log.**` is a different string than `log.app`, so the
// two are never cross-compared — wildcard-subscriber type
// compatibility is a separate, fuzzier question).

/// A canonical, comparable rendering of a `TypeExpr` — equal strings
/// mean the same type at this layer. Also used in the diagnostic.
fn type_expr_key(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Primitive(p, _) => format!("{:?}", p),
        TypeExpr::Perspective { name, .. } => {
            format!("perspective({})", name.name)
        }
        TypeExpr::Bounded { elem, cap, .. } => {
            format!("bounded[{}; {}]", type_expr_key(elem), cap)
        }
        TypeExpr::Named { path, generic_args, .. } => {
            let base = path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join("::");
            if generic_args.is_empty() {
                base
            } else {
                let args = generic_args
                    .iter()
                    .map(type_expr_key)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}<{}>", base, args)
            }
        }
        TypeExpr::Projection { class, inner, .. } => {
            format!("{:?}({})", class, type_expr_key(inner))
        }
        TypeExpr::Array { elem, .. } => format!("[{}]", type_expr_key(elem)),
        TypeExpr::Tuple(tys, _) => format!(
            "({})",
            tys.iter().map(type_expr_key).collect::<Vec<_>>().join(", ")
        ),
        TypeExpr::Function { params, ret, .. } => format!(
            "fn({}){}",
            params.iter().map(type_expr_key).collect::<Vec<_>>().join(", "),
            ret.as_ref()
                .map(|r| format!(" -> {}", type_expr_key(r)))
                .unwrap_or_default()
        ),
    }
}

fn check_bus_subject_types(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    // subject string -> the distinct payload types seen, each with a
    // representative site span. Insertion order preserved so the
    // first-declared type is the "expected" one in the message.
    let mut subjects: BTreeMap<String, Vec<(String, Span)>> = BTreeMap::new();

    fn record(
        subject: &BusSubject,
        ty: &Option<TypeExpr>,
        span: Span,
        subjects: &mut BTreeMap<String, Vec<(String, Span)>>,
    ) {
        // Only literal subjects carry an independent `of type`; topic
        // refs are unified by their declaration, qualified refs live
        // in another seed.
        let BusSubject::Literal { subject: subj, .. } = subject else {
            return;
        };
        let Some(ty) = ty else { return };
        let key = type_expr_key(ty);
        let entry = subjects.entry(subj.clone()).or_default();
        if !entry.iter().any(|(k, _)| k == &key) {
            entry.push((key, span));
        }
    }

    fn walk(
        items: &[TopDecl],
        subjects: &mut BTreeMap<String, Vec<(String, Span)>>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Bus(bb) = m {
                            for bm in &bb.members {
                                match bm {
                                    BusMember::Publish { subject, ty, span, .. } => {
                                        record(subject, ty, *span, subjects)
                                    }
                                    BusMember::Subscribe { subject, ty, span, .. } => {
                                        record(subject, ty, *span, subjects)
                                    }
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(md) => walk(&md.items, subjects),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &mut subjects);
    }

    for (subj, types) in &subjects {
        if types.len() < 2 {
            continue;
        }
        let (expected, _) = &types[0];
        // Report each divergent type once, at its site.
        for (got, span) in types.iter().skip(1) {
            diags.push(Diag::ty(
                *span,
                format!(
                    "bus subject `\"{}\"` is used with conflicting payload \
                     types: `{}` here vs `{}` at another site. Every \
                     publish/subscribe on the same subject must carry the \
                     same payload type — a mismatch decodes the wire bytes as \
                     the wrong type at runtime. Declare a `topic` to fix the \
                     type in one place, or align the `of type` annotations.",
                    subj, got, expected,
                ),
            ));
        }
    }
}

/// What the bus can do with a type written in an `of type` clause.
enum Carriage {
    /// A field layout a cell can serialize, or the raw-frame opt-out.
    Carried,
    /// Resolved, and the runtime has no way to put it on the wire.
    /// Carries the repair that fits the shape.
    NotCarried(&'static str),
    /// Not resolvable from this bundle, so not this check's to judge.
    Unresolved,
}

/// GH #876: can a cell carry a payload of this type?
///
/// The accepted set is the runtime's, and it is small because a
/// delivery is a serialized STRUCT: the payload needs a field layout
/// — a user `type`, or the storage struct of an enum that has at
/// least one variant with fields — or it opts out of typing
/// altogether with `BytesView`, the raw-frame path a foreign ring
/// writes. A primitive has neither: there is no `__serialize_Int`,
/// and codegen says so (`bus/dispatch.rs`, `locus/decl.rs`).
///
/// [`Carriage::Unresolved`] is the permissive half, and it is
/// deliberately where RESOLUTION stops rather than where the rule
/// stops — this check must not turn "I cannot see that name" into "no
/// such payload":
///
///   - a qualified path (`shared::Metric`) whose seed is not in this
///     bundle, which is what a single-file check of a multi-file seed
///     holds — `resolve_type_expr` types it `Unknown` by design;
///   - a generic instantiation (`Box<Int>`), whose mangled monomorph
///     is synthesized during lowering and is no bundle symbol yet;
///   - a bare name nothing in the bundle declares, which is a missing
///     declaration rather than an uncarriable payload, and is named
///     as such by the rule for unknown type names.
fn payload_carriage(
    te: &TypeExpr,
    top: &TopScope,
    known: &KnownNames,
) -> Carriage {
    if let TypeExpr::Named { generic_args, .. } = te {
        if !generic_args.is_empty() {
            return Carriage::Unresolved;
        }
    }
    match resolve_type_expr(te, known) {
        // The raw-frame path: no struct type, bounded view per record.
        Ty::Prim(PrimType::BytesView) => Carriage::Carried,
        Ty::Named(n) => match top.symbols.get(&n) {
            Some(TopSymbol::Type(ti)) => match &ti.kind {
                TypeKind::Struct(_) => Carriage::Carried,
                // A no-payload enum has no storage struct to
                // serialize — codegen refuses it by name.
                TypeKind::Enum(variants) => {
                    if variants.iter().any(|v| !v.fields.is_empty()) {
                        Carriage::Carried
                    } else {
                        Carriage::NotCarried(
                            "give one variant a payload, or wrap the enum \
                             in a user `type`",
                        )
                    }
                }
                // Aliases are transparent through
                // `resolve_type_expr`, so this arm is unreachable in
                // practice; judging it would be judging a name.
                TypeKind::Alias(_) => Carriage::Unresolved,
            },
            // A locus, interface, perspective or topic name in
            // payload position is a different mistake, reported
            // elsewhere (the handler-signature rule names the
            // topic-as-type one).
            _ => Carriage::Unresolved,
        },
        Ty::Unknown => Carriage::Unresolved,
        // Every remaining shape — the other primitives, arrays,
        // tuples, `bounded[T; N]`, projections, fn types — reaches
        // codegen as something other than a TypeRef and is refused
        // there.
        _ => Carriage::NotCarried("wrap it in a user `type`"),
    }
}

/// GH #876: a bus subject's declared payload must be a type the bus
/// can carry.
///
/// `bus { publish "org.metrics" of type Int; }` typechecked clean and
/// could not be lowered. The publish side died with `bus send payload
/// must be a user-type or has-payload enum value; got Int` and the
/// subscribe side with `missing or unsupported payload type (m60
/// requires a TypeRef, has-payload Enum, or BytesView)` — both from
/// codegen, with no span, on a declaration that is among the first
/// things an author writes. That is the check/build divergence class
/// `corpus_check_build_agreement` gates, and three corpus programs
/// carried its ratchet rows.
///
/// PR #458's rule compares a handler's parameter against the declared
/// payload; it never asks whether the declared payload is a payload at
/// all. This asks exactly that, once per `of type` clause, at the
/// clause's own span.
///
/// Scope: the `of type` clause on a `publish` / `subscribe`. A
/// `topic T { payload: Int; }` is under the same contract and is
/// still refused during lowering, because the checker never runs
/// `desugar_topics` (codegen does) — so a topic-ref subscribe
/// carries no `ty` at this layer and there is nothing here to judge.
/// Closing that half means reading `TopicDecl.payload`, which moves
/// its own set of ratchet rows; it is deliberately not this change.
fn check_bus_payload_carriable(
    bundle: &Bundle<'_>,
    top: &TopScope,
    known: &KnownNames,
    diags: &mut Vec<Diag>,
) {
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(l) = item else { return };
            for m in &l.members {
                let LocusMember::Bus(bb) = m else { continue };
                for bm in &bb.members {
                    let (verb, subject, ty) = match bm {
                        BusMember::Subscribe { subject, ty, .. } => {
                            ("subscribe", subject, ty)
                        }
                        BusMember::Publish { subject, ty, .. } => {
                            ("publish", subject, ty)
                        }
                    };
                    let Some(ty) = ty else { continue };
                    // A topic ref takes its payload from the topic
                    // declaration, and an `of type` clause on one is
                    // already an error ("forbidden on topic refs").
                    // Judging the stray clause too would report one
                    // mistake twice.
                    if !matches!(subject, BusSubject::Literal { .. }) {
                        continue;
                    }
                    let Carriage::NotCarried(repair) =
                        payload_carriage(ty, top, known)
                    else {
                        continue;
                    };
                    diags.push(Diag::ty(
                        ty.span(),
                        format!(
                            "{} `{}`: a bus subject's payload must be a type \
                             the bus can carry — a user `type`, an enum with \
                             a payload variant, or `BytesView` \
                             (`std::bytes::BytesView`, the raw-frame path). \
                             `{}` is not carried on the bus: a delivery is a \
                             serialized struct, so the payload needs a field \
                             layout. To send one, {} and declare the subject \
                             `of type` that.",
                            verb,
                            subject.canonical(),
                            resolve_type_expr(ty, known).display(),
                            repair,
                        ),
                    ));
                }
            }
        });
    }
}

fn check_bus_backpressure(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    fn walk(items: &[TopDecl], diags: &mut Vec<Diag>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let lname = l.name.name.as_str();
                    for m in &l.members {
                        match m {
                            LocusMember::Lifecycle(LifecycleDecl {
                                body, ..
                            }) => scan_flood_in_block(body, lname, diags),
                            LocusMember::Fn(f) => {
                                scan_flood_in_block(&f.body, lname, diags)
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(md) => walk(&md.items, diags),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, diags);
    }
}

fn collect_known_names(
    top: &TopScope,
    import_renames: &[(Vec<String>, String)],
) -> KnownNames {
    let mut m = KnownNames::default();
    // GH #833: the checker resolves type expressions against THIS
    // table, rebuilt from the top scope — so without the bundle's
    // rename rows a qualified cross-seed annotation would come back
    // `Unknown` here even though `build_top_scope` had just typed the
    // same annotation in a signature.
    m.set_imports(import_renames);
    for (name, sym) in &top.symbols {
        if matches!(
            sym,
            TopSymbol::Locus(_) | TopSymbol::Type(_) | TopSymbol::Perspective(_)
        ) {
            m.insert(name.clone(), sym.span());
        }
        // GH #759: carry the alias targets across, already
        // expanded by `build_top_scope` — the checker resolves
        // type expressions against THIS table, so without them a
        // `type Thing = Int;` use would come back `Ty::Named`
        // again and stop unifying with `Int`.
        if let TopSymbol::Type(info) = sym {
            if let TypeKind::Alias(t) = &info.kind {
                m.set_alias(name.clone(), t.clone());
            }
        }
    }
    m
}

/// Stage-1 FFI (2026-05-22): predicate returning the rejection
/// reason if `ty` is not portable across the C-ABI boundary.
/// Returns `None` when the type is permitted in `@ffi` parameter
/// and return positions. See `spec/ffi.md` for the contract.
///
/// Stage 1 allows: scalar primitives (Int / Float / Bool /
/// Duration / Time), reference primitives with stable C
/// representation (String → `const char *`, Bytes → Hale
/// `[int64 len][payload]` ptr, BytesView / StringView → 16-byte
/// struct by value), and named user-type structs (layout-
/// compatible C struct by value — the library author is
/// responsible for keeping the Hale side and C side in sync;
/// future spec iteration may add a layout-assertion mechanism).
///
/// Stage 1 rejects: `Decimal` (i128 ABI is platform-variable),
/// `Uint` (Hale-internal type, no portable C mapping at v0),
/// projections / arrays / tuples / fallibles / functions / unit-
/// in-param-position. Unit (`Ty::Unit`) is allowed only as a
/// return type — the parser models `fn ...;` (no `-> T`) as
/// `ret: None`, which downstream represents as Unit; the caller
/// of this predicate already handles that path.
fn ffi_type_unportable(ty: &Ty) -> Option<&'static str> {
    match ty {
        Ty::Bounded(_, _) => Some(
            "bounded[T; N] has no portable C mapping — pass the \
             element pointer + count separately",
        ),
        Ty::Prim(p) => match p {
            PrimType::Int
            | PrimType::Float
            | PrimType::Bool
            | PrimType::String
            | PrimType::Bytes
            | PrimType::BytesView
            | PrimType::StringView
            | PrimType::BytesMut
            | PrimType::Time
            | PrimType::Duration => None,
            PrimType::Decimal => Some(
                "Decimal (i128) has platform-variable ABI; marshal as \
                 Int/Float at the Hale side instead",
            ),
            PrimType::Uint => Some(
                "Uint is Hale-internal; declare as Int in the @ffi \
                 signature",
            ),
        },
        // Unit allowed in return position; check_fn handles `ret:
        // None`. A `Ty::Unit` reaching this predicate from a param
        // came from an empty `()` type expr, which is invalid.
        Ty::Unit => Some(
            "() (unit) is not a meaningful FFI parameter type",
        ),
        // Named user-type structs are permitted at Stage 1. The
        // library author is responsible for keeping the Hale
        // struct's field order + types layout-compatible with the
        // C struct on the other side. Future spec iteration may
        // add a `@ffi_layout("c")` attribute for compile-time
        // layout assertions.
        Ty::Named(_) => None,
        Ty::Projection(_, _) => Some(
            "projection-typed values (Rich / Chunked / Recognition) \
             carry per-locus metadata and don't cross the C-ABI \
             boundary",
        ),
        Ty::Array(_, _) => Some(
            "fixed-size arrays don't cross the C-ABI boundary at \
             Stage 1; pass Bytes / a wrapper struct instead",
        ),
        Ty::Tuple(_) => Some(
            "tuples have no portable C struct layout; declare a named \
             type instead",
        ),
        Ty::Function { .. } => Some(
            "function-pointer types are not yet FFI-portable; declare \
             the wrapper at the C side and pass a struct/handle",
        ),
        Ty::Fallible { .. } => Some(
            "fallible(E) is an Hale internal channel; C functions \
             must return an error sentinel and the Hale wrapper \
             above translates",
        ),
        // Unknown comes from unresolved type names. Be permissive
        // — the named-type resolution may not have completed yet,
        // or the type may live behind an import this check can't
        // see. Codegen will catch genuinely-broken signatures at
        // LLVM-declaration emit time.
        Ty::Unknown => None,
    }
}

/// WASM plan — stdlib portability table. Returns a one-line reason +
/// browser alternative for a `std::` path unavailable under `target
/// wasm` (no syscalls in the browser sandbox), or `None` for the
/// portable surface (str/bytes/json/text/math/crypto/rand/decimal/bus/
/// diag/time/env/iter/...). Keyed on the leading namespace so every
/// call under it is covered.
fn wasm_unavailable_stdlib(segs: &[&str]) -> Option<&'static str> {
    match segs {
        ["std", "io", "tcp", ..] => Some(
            "raw TCP sockets don't exist in the browser; use a WebSocket \
             bus adapter (`ws://`) for networking",
        ),
        ["std", "io", "udp", ..] => {
            Some("raw UDP sockets don't exist in the browser")
        }
        ["std", "io", "tls", ..] => Some(
            "raw TLS isn't available; the browser performs TLS transparently \
             for `wss://` / `https://`",
        ),
        ["std", "io", "fs", ..] | ["std", "io", "file", ..] => Some(
            "filesystem access isn't available in the browser sandbox; use \
             `fetch` (via an `@ffi(\"js\")` host import) or a bus message",
        ),
        ["std", "io", "stdin", ..] | ["std", "io", "stdout", ..] => Some(
            "raw terminal I/O isn't available; use `println(...)` (the loader \
             routes it to the host console)",
        ),
        ["std", "term", ..] => {
            Some("terminal control (`std::term`) isn't available in the browser")
        }
        ["std", "process", ..] => Some(
            "OS process control (`std::process`) isn't available in the browser",
        ),
        ["std", "http", ..] => Some(
            "the `std::http` server is built on raw TCP and isn't available \
             in the browser",
        ),
        _ => None,
    }
}

struct Checker<'a> {
    top: &'a TopScope,
    known: &'a KnownNames,
    diags: &'a mut Vec<Diag>,
    locals: ScopeStack,
    current_locus: Option<&'a LocusInfo>,
    in_lifecycle: bool,
    in_closure: bool,
    /// v1.x-VIOLATE (F.27): true while typechecking an
    /// `on_failure` body. Gates the rejection of `violate`
    /// inside `on_failure` (use `bubble(err)` instead).
    in_on_failure: bool,
    /// v1.x-FORM-1: when inside a `fallible(E)` fn body, holds
    /// `(success_ret, payload_E)`. Used to validate `return`
    /// against the success type, `fail <expr>;` against the
    /// payload type, and to gate the `err` implicit binding on
    /// `or`-substitute RHS scopes.
    fallible_ctx: Option<(Ty, Ty)>,
    /// #335: the enclosing fn's declared return type, for ANY fn.
    /// `fallible_ctx` only covered fallible bodies, so a plain
    /// `fn f() -> Int { return "s"; }` had no return check at all and
    /// surfaced at codegen as "unsupported in codegen v0".
    return_ctx: Option<Ty>,
    /// WASM plan: true when the bundle declares `target wasm` /
    /// `target browser_js`. Gates the POSIX-only stdlib (no syscalls in
    /// the browser sandbox) at typecheck — see `wasm_unavailable_stdlib`.
    wasm_target: bool,
    /// The build target has the `async_io` pool backend — the bundle's
    /// [`Bundle::target_has_async_io`], never the host's (GH #970).
    target_has_async_io: bool,
    target_label: &'static str,
    strict_callees: bool, // F.18: on for a whole seed (`hale check <dir>`), off for a partial program
    /// GH #721: on for a whole program — every import resolved, so a
    /// bare identifier nothing binds is a typo rather than a name a
    /// sibling file supplies. `hale check <dir>` and the build path
    /// both set it; one file checked alone does not.
    strict_idents: bool,
    /// M3 stage 2 (2026-07-02): true while checking an `or`
    /// expression whose value is discarded (statement position) —
    /// the Substitute arm skips the fallback-vs-success type match.
    /// Set by Stmt::Expr, consumed and cleared by the Or arm so
    /// nested `or`s in subexpressions don't inherit it.
    or_value_discarded: bool,
    /// M3 stage 3 (2026-07-02): generic fn templates declared in
    /// the program being checked (name → decl). Call sites mirror
    /// codegen's m62 inference at the Ty level — every generic
    /// param must be pinned by an arg, bindings must not conflict,
    /// args must match the substituted params — and the call types
    /// as the SUBSTITUTED return instead of Unknown.
    generic_fns: BTreeMap<String, &'a FnDecl>,
    /// GH #877: the generic parameters of the declaration being
    /// checked — a fn's `<T>`, a generic `type`'s. They name no
    /// top-level declaration and resolve to `Ty::Unknown` by design,
    /// so the unknown-bare-type-name rule has to know them to avoid
    /// reporting `T` as a typo.
    generic_params: Vec<String>,
    /// M3 stage 3 tranche 2: generic TYPE templates (name → decl).
    /// Mangled monomorph literals (`Box_Int { ... }`) resolve
    /// against these — previously "unknown type" at typecheck,
    /// which made generic types unusable through the CLI (only
    /// codegen unit tests, which skip the checker, exercised
    /// them). Fields validate against the SUBSTITUTED types.
    generic_types: BTreeMap<String, &'a TypeDecl>,
    /// GH #911 B5: generic LOCUS templates (name → decl). The same
    /// table for `locus Cache<K, V> { ... }`, and for the same
    /// reason: without it the checker refused every instantiation of
    /// a generic locus while `hale build` lowered it, so the shape
    /// was reachable only from codegen tests (which skip the
    /// checker). A locus's `params` are its fields — the monomorph's
    /// are the template's with the arguments substituted.
    generic_loci: BTreeMap<String, &'a LocusDecl>,
    /// GH #255 phase 1: topic names with a declared transport
    /// binding (any `bindings { }` entry, bundle-wide). Gates
    /// `or wait` on publishes — the loss window it waits out
    /// only exists for bound topics.
    bound_topics: &'a std::collections::BTreeSet<String>,
    /// Cross-seed import renames: `["alias", "name"] -> mangled`.
    /// Lets a qualified locus/struct literal (`mat::Grid { }`) resolve
    /// to the merged symbol codegen will lower, instead of typing as
    /// `Ty::Unknown` — which hid every field/method access behind it
    /// from the checker (a method call that codegen can't lower then
    /// passed `check` and died at build). Empty for a single-seed
    /// bundle, so this only ever activates on imported literals.
    import_renames: &'a [(Vec<String>, String)],
    /// GH #724: import aliases whose seed this bundle does NOT hold.
    /// Every CLI path merges the imported seeds and hands the checker
    /// one program with no `import` directives left, so this set is
    /// empty there. `hale lsp` bundles a directory's own files with
    /// their `import` lines intact and no rename table at all — which
    /// is why a qualified type or call types as opaque in the editor
    /// rather than as an error. A qualified perspective path behind
    /// such an alias gets the same tolerance, so the editor does not
    /// squiggle a program `hale check` accepts.
    unresolved_import_aliases: &'a std::collections::BTreeSet<String>,
}

#[derive(Default)]
struct ScopeStack {
    frames: Vec<BTreeMap<String, LocalSym>>,
}

#[derive(Debug, Clone)]
struct LocalSym {
    ty: Ty,
    /// m50: tracks whether the binding was declared with `mut`.
    /// `let x = ...` is immutable; `let mut x = ...` permits
    /// reassignment. Per spec/types.md "Mutability" + design-
    /// rationale §E. Locus state on `self` is mutable
    /// independently (locus fields aren't bindings — they're
    /// state — and lifecycle methods update them through
    /// `self.field = ...` regardless of any binding's is_mut).
    /// Fn params, loop variables, and pattern bindings default
    /// to false: the surface spec says params are immutable,
    /// loop vars rebind fresh each iteration, and pattern arm
    /// bindings exist only for the duration of the arm body.
    is_mut: bool,
}

impl ScopeStack {
    fn new() -> Self {
        Self {
            frames: vec![BTreeMap::new()],
        }
    }
    fn push(&mut self) {
        self.frames.push(BTreeMap::new());
    }
    fn pop(&mut self) {
        self.frames.pop();
    }
    fn insert(&mut self, name: &str, sym: LocalSym) {
        self.frames
            .last_mut()
            .expect("at least one scope")
            .insert(name.to_string(), sym);
    }
    fn lookup(&self, name: &str) -> Option<&LocalSym> {
        for frame in self.frames.iter().rev() {
            if let Some(s) = frame.get(name) {
                return Some(s);
            }
        }
        None
    }
}

/// GH #436 follow-up: which half of confinement a site exercises.
///
/// Reads resolve through the expression field-access arm and writes
/// through LValue traversal — two paths, and the original check only
/// hooked the first. Naming the distinction keeps the diagnostic
/// honest ("writes one from outside", not "reads") and makes the
/// second path impossible to forget again.
#[derive(Clone, Copy)]
enum SealedAccess {
    Read,
    Write,
}

impl<'a> Checker<'a> {
    fn check_top_decl(&mut self, decl: &'a TopDecl) {
        match decl {
            TopDecl::Locus(l) => self.check_locus(l),
            TopDecl::Fn(f) => self.check_fn(f, None),
            TopDecl::Const(c) => {
                // GH #877: the ascription is an annotation like any
                // other.
                self.check_type_annotation(&c.ty);
                let want = resolve_type_expr(&c.ty, self.known);
                let got = self.check_expr(&c.value);
                if !want.assignable_from(&got) {
                    self.diags.push(Diag::ty(
                        c.value.span(),
                        format!(
                            "const `{}`: expected `{}`, got `{}`",
                            c.name.name,
                            want.display(),
                            got.display()
                        ),
                    ));
                }
            }
            TopDecl::Module(m) => {
                for item in &m.items {
                    self.check_top_decl(item);
                }
            }
            TopDecl::Type(t) => {
                // Structure already validated by resolver; field
                // types are checked when something instantiates
                // them via struct literal.
                //
                // GH #877: except that a field whose type names
                // nothing is never checked by a literal — the field
                // types as `Unknown`, which accepts every
                // initializer, and the program dies at lowering with
                // `unknown type name in signature`. The declaration
                // is where the name is written, so it is where the
                // rule fires. The type's own generic parameters are
                // in scope for its fields.
                let prev_generics = std::mem::replace(
                    &mut self.generic_params,
                    t.generics
                        .iter()
                        .map(|g| g.name.name.clone())
                        .collect(),
                );
                match &t.body {
                    TypeDeclBody::Struct(fields) => {
                        for f in fields {
                            self.check_type_annotation(&f.ty);
                        }
                    }
                    TypeDeclBody::Enum(variants) => {
                        for v in variants {
                            for te in &v.fields {
                                self.check_type_annotation(te);
                            }
                        }
                    }
                    TypeDeclBody::Alias(te) => {
                        self.check_type_annotation(te);
                    }
                }
                self.generic_params = prev_generics;
            }
            TopDecl::Perspective(_) => {
                // Structure already validated by resolver; the
                // contract's method signatures are checked against
                // the serving locus at `serves` conformance.
            }
            TopDecl::Interface(_) => {
                // Interface declarations are pure type-level —
                // method signatures only, no bodies. The resolver
                // collected them; the structural impl-check fires
                // at the use site (call expression where the
                // expected type is an interface).
            }
            TopDecl::Group(_) => {
                // GH #382: claim vocabulary. Membership resolution
                // (unknown name = error, vacuity) is a bundle-level
                // pass — the law judgment — because members
                // may name imported decls only the merged bundle
                // can see. Nothing per-decl to check here.
            }
            TopDecl::Role(_) => {
                // GH #1109: authorization vocabulary. Declared-ness,
                // `includes` and the annotation sites are checked
                // bundle-wide in `check_api_roles`.
            }
            TopDecl::Topic(t) => {
                // Topic declarations carry `payload: T; subject:
                // "...";` and now (Phase 3, 2026-05-25) optional
                // `keyed_by FIELD;` + `on_unmatched: V;`. The
                // resolver validated the payload type expression
                // already; per-use-site handler/send checks happen
                // at bus-block and send sites that reference the
                // topic. Below: Phase-3 specific static checks.

                // (5) / (6) on_unmatched policy validation:
                //   - swallow / None: nothing to check here.
                //   - fail: Send sites for this topic must carry
                //     an `or raise` / `or discard` disposition;
                //     validated at the Send site in check_send.
                //   - fallback: a program-wide `where key == _`
                //     subscriber must exist; validated in
                //     check_phase3_fallback_subscribers (bundle
                //     pass).
                let _ = t.on_unmatched;

                // (1) keyed_by field must exist on the payload
                // type and resolve to an int-shaped scalar
                // (Int, Decimal, Time, Duration, Bool, or
                // no-payload enum). For payloads that don't
                // resolve to a user-declared struct (Ty::Unknown
                // / external type / primitive), skip the check —
                // the resolver will already have flagged the
                // payload as unresolvable.
                if let Some(field_ident) = &t.keyed_by {
                    let payload_ty_name = match &t.payload {
                        TypeExpr::Named { path, .. }
                            if path.segments.len() == 1 =>
                        {
                            Some(path.segments[0].name.clone())
                        }
                        _ => None,
                    };
                    let mut found_field_ty: Option<Ty> = None;
                    if let Some(name) = &payload_ty_name {
                        if let Some(TopSymbol::Type(info)) =
                            self.top.lookup(name)
                        {
                            if let TypeKind::Struct(fields) = &info.kind {
                                if let Some(f) = fields
                                    .iter()
                                    .find(|f| f.name == field_ident.name)
                                {
                                    found_field_ty = Some(f.ty.clone());
                                }
                            }
                        }
                    }
                    if payload_ty_name.is_some() && found_field_ty.is_none() {
                        self.diags.push(Diag::ty(
                            field_ident.span,
                            format!(
                                "topic `{}`'s `keyed_by` references \
                                 field `{}`, which does not exist on \
                                 payload type `{}`",
                                t.name.name,
                                field_ident.name,
                                payload_ty_name.as_deref().unwrap_or("?"),
                            ),
                        ));
                    }
                    if let Some(fty) = found_field_ty {
                        if !is_key_eligible(&fty, &self.top) {
                            self.diags.push(Diag::ty(
                                field_ident.span,
                                format!(
                                    "topic `{}`'s `keyed_by` field \
                                     `{}` has type `{}`; routing-key \
                                     fields must be Int, Decimal, \
                                     Time, Duration, Bool, String, \
                                     or a no-payload enum",
                                    t.name.name,
                                    field_ident.name,
                                    fty.display(),
                                ),
                            ));
                        }
                    }
                }

                // (also covers the case where) `on_unmatched` was
                // specified on a topic that doesn't declare
                // `keyed_by`: it has no meaning and is rejected
                // (catches typos / leftover from earlier drafts).
                if t.on_unmatched.is_some() && t.keyed_by.is_none() {
                    self.diags.push(Diag::ty(
                        t.span,
                        format!(
                            "topic `{}` sets `on_unmatched` but has \
                             no `keyed_by` — `on_unmatched` is only \
                             meaningful on keyed topics",
                            t.name.name
                        ),
                    ));
                }
            }
            TopDecl::Target(_) => {
                // FUv0.8.2 #7 (2026-05-25): target capability
                // block. v0.2 lands the parser + AST surface;
                // the capability-enforcement pass (rejecting
                // programs that reach beyond the declared
                // capability set) is v0.3. Today's check is
                // structural only — the resolver registered
                // the target name; no use-site checks here yet.
            }
            TopDecl::RingLayout(r) => self.check_ring_layout(r),
            TopDecl::Claims(_) | TopDecl::Constitution(_) => {
                // #392 thread 2 / GH #409: a library-tier claims
                // block, or a named constitution. Placement rules and
                // evaluation live in the bundle-level claims pass —
                // this checker is per-decl, and both are law over the
                // assembled whole.
            }
        }
    }

    /// shm-ring-interop Proposal B: validate a `ring_layout`'s
    /// contract — known width reprs, a recognized framing kind (with
    /// `len_prefix` for byte_records), at least one cursor with an
    /// `at` offset and a known repr/ordering/unit, and non-negative
    /// offsets. A wrong-but-well-formed layout still produces wrong
    /// *values* not OOB (the safety argument), but these catch the
    /// obvious declaration mistakes at build time.
    fn check_ring_layout(&mut self, r: &'a hale_syntax::ast::RingLayoutDecl) {
        use hale_syntax::ast::RingAttrValue;
        const WIDTHS: &[&str] = &[
            "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64",
        ];
        const ORDERINGS: &[&str] =
            &["relaxed", "acquire", "release", "acq_rel", "seq_cst"];

        if let Some(off) = r.data_at {
            if off < 0 {
                self.diags.push(Diag::ty(
                    r.span,
                    format!("ring_layout `{}`: data_at must be >= 0", r.name.name),
                ));
            }
        }
        for f in &r.scalars {
            if f.at < 0 {
                self.diags.push(Diag::ty(
                    f.span,
                    format!("ring_layout field `{}`: offset must be >= 0", f.name.name),
                ));
            }
            if !WIDTHS.contains(&f.repr.name.as_str()) {
                self.diags.push(Diag::ty(
                    f.repr.span,
                    format!(
                        "ring_layout field `{}`: unknown repr `{}` (expected one \
                         of u8/u16/u32/u64, i8/i16/i32/i64, f32/f64)",
                        f.name.name, f.repr.name
                    ),
                ));
            }
        }

        // At least one cursor, each with an `at` offset and known attrs.
        if r.cursors.is_empty() {
            self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: needs at least one `cursor {{ ... }}` \
                     (the published position a consumer reads)",
                    r.name.name
                ),
            ));
        }
        for c in &r.cursors {
            let mut has_at = false;
            for a in &c.attrs {
                match a.key.name.as_str() {
                    "at" => {
                        has_at = true;
                        if let RingAttrValue::Int(n) = a.value {
                            if n < 0 {
                                self.diags.push(Diag::ty(
                                    a.span,
                                    "cursor `at` offset must be >= 0".to_string(),
                                ));
                            }
                        } else {
                            self.diags.push(Diag::ty(
                                a.span,
                                "cursor `at` must be an integer offset".to_string(),
                            ));
                        }
                    }
                    "load" | "store" => {
                        if let RingAttrValue::Ident(id) = &a.value {
                            if !ORDERINGS.contains(&id.name.as_str()) {
                                self.diags.push(Diag::ty(
                                    a.span,
                                    format!(
                                        "cursor `{}`: unknown memory ordering `{}` \
                                         (relaxed/acquire/release/acq_rel/seq_cst)",
                                        a.key.name, id.name
                                    ),
                                ));
                            }
                        }
                    }
                    "unit" => {
                        if let RingAttrValue::Ident(id) = &a.value {
                            if id.name != "bytes" && id.name != "slots" {
                                self.diags.push(Diag::ty(
                                    a.span,
                                    format!(
                                        "cursor `unit`: expected `bytes` or `slots`, \
                                         got `{}`",
                                        id.name
                                    ),
                                ));
                            }
                        }
                    }
                    // `kind`, `repr` accepted as free idents (the
                    // descriptor build in PR3 maps the known ones).
                    _ => {}
                }
            }
            if !has_at {
                self.diags.push(Diag::ty(
                    c.span,
                    format!(
                        "ring_layout `{}`: cursor needs an `at OFFSET;`",
                        r.name.name
                    ),
                ));
            }
        }

        // Framing: required, recognized kind; byte_records needs a
        // len_prefix width.
        match &r.framing {
            None => self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: needs a `framing byte_records {{ ... }}` \
                     (or `framing slots {{ ... }}`)",
                    r.name.name
                ),
            )),
            Some(fr) => {
                if fr.kind.name != "byte_records" && fr.kind.name != "slots" {
                    self.diags.push(Diag::ty(
                        fr.kind.span,
                        format!(
                            "ring_layout `{}`: unknown framing `{}` (expected \
                             `byte_records` or `slots`)",
                            r.name.name, fr.kind.name
                        ),
                    ));
                }
                // Slots framing: a fixed-stride slot ring (the native
                // LotusRing shape). The consumer reads the geometry from
                // the foreign header, so it needs `slot_size` + `slot_count`
                // scalars (no `len_prefix` — records aren't length-framed).
                if fr.kind.name == "slots" {
                    for field in ["slot_size", "slot_count"] {
                        if !r.scalars.iter().any(|s| s.name.name == field) {
                            self.diags.push(Diag::ty(
                                fr.span,
                                format!(
                                    "framing slots: needs a `{field}` scalar — the \
                                     consumer reads the slot geometry from the \
                                     foreign header (e.g. `{field} at <off> : u64;`)"
                                ),
                            ));
                        }
                    }
                }
                if fr.kind.name == "byte_records" {
                    let len_prefix = fr.attrs.iter().find(|a| a.key.name == "len_prefix");
                    match len_prefix {
                        None => self.diags.push(Diag::ty(
                            fr.span,
                            "framing byte_records: needs `len_prefix u32;` (the \
                             record length-prefix width)".to_string(),
                        )),
                        Some(a) => {
                            if let RingAttrValue::Ident(id) = &a.value {
                                if !WIDTHS.contains(&id.name.as_str()) {
                                    self.diags.push(Diag::ty(
                                        a.span,
                                        format!(
                                            "framing byte_records: `len_prefix` repr \
                                             `{}` is not a known width",
                                            id.name
                                        ),
                                    ));
                                }
                            } else {
                                self.diags.push(Diag::ty(
                                    a.span,
                                    "framing byte_records: `len_prefix` must be a \
                                     width ident (e.g. u32)".to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        // shm-ring-interop conformance (2026-06-06): cross-field
        // geometric consistency. the foreign format is fixed and
        // unchangeable, so a `ring_layout` that mis-transcribes it is
        // purely *our* bug — and several of these fields silently
        // corrupt PR3's already-shipped reader if they're wrong (a
        // cursor past `data_at`, an overlapping field, a non-power-of-
        // two `align` the reader masks with, a `pad_sentinel` too wide
        // for the `len_prefix` to hold). Catch them at compile time.
        fn repr_bytes(name: &str) -> Option<i64> {
            Some(match name {
                "u8" | "i8" => 1,
                "u16" | "i16" => 2,
                "u32" | "i32" | "f32" => 4,
                "u64" | "i64" | "f64" | "atomic_u64" => 8,
                _ => return None,
            })
        }

        // Occupied header intervals [start, end) with a label + span.
        let mut intervals: Vec<(i64, i64, String, Span)> = Vec::new();
        for f in &r.scalars {
            if f.at >= 0 {
                if let Some(w) = repr_bytes(&f.repr.name) {
                    intervals.push((
                        f.at,
                        f.at + w,
                        format!("field `{}`", f.name.name),
                        f.span,
                    ));
                }
            }
        }
        for c in &r.cursors {
            let at = c.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                ("at", RingAttrValue::Int(n)) if *n >= 0 => Some(*n),
                _ => None,
            });
            if let Some(at) = at {
                let label = c
                    .name
                    .as_ref()
                    .map(|n| format!("cursor `{}`", n.name))
                    .unwrap_or_else(|| "cursor".to_string());
                // A cursor is an atomic u64 — 8 bytes.
                intervals.push((at, at + 8, label, c.span));
            }
        }

        // (1) every header field + cursor must lie before `data_at`.
        if let Some(data_at) = r.data_at {
            for (start, end, label, span) in &intervals {
                if *end > data_at {
                    self.diags.push(Diag::ty(
                        *span,
                        format!(
                            "ring_layout `{}`: {} occupies bytes [{}, {}) which \
                             overruns `data_at {}` — header fields and the cursor \
                             must lie before the data region",
                            r.name.name, label, start, end, data_at
                        ),
                    ));
                }
            }
        }

        // (2) no two header fields / cursor may overlap.
        for i in 0..intervals.len() {
            for j in (i + 1)..intervals.len() {
                let (a0, a1) = (intervals[i].0, intervals[i].1);
                let (b0, b1) = (intervals[j].0, intervals[j].1);
                if a0 < b1 && b0 < a1 {
                    self.diags.push(Diag::ty(
                        intervals[i].3,
                        format!(
                            "ring_layout `{}`: {} (bytes [{}, {})) overlaps {} \
                             (bytes [{}, {}))",
                            r.name.name, intervals[i].2, a0, a1, intervals[j].2, b0, b1
                        ),
                    ));
                }
            }
        }

        // (3) byte_records framing: `align` must be a power of two
        //     (the reader does `& ~(align-1)`); `pad_sentinel` must
        //     fit in the `len_prefix` width or wrap-detection reads a
        //     truncated sentinel and never fires.
        if let Some(fr) = &r.framing {
            if fr.kind.name == "byte_records" {
                let align_attr =
                    fr.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                        ("align", RingAttrValue::Int(n)) => Some((*n, a.span)),
                        _ => None,
                    });
                // Absent `align` defaults to 1 (byte-packed) at runtime.
                let align = align_attr.map(|(n, _)| n).unwrap_or(1);
                let align_span = align_attr.map(|(_, s)| s).unwrap_or(fr.span);
                if align < 1 || (align & (align - 1)) != 0 {
                    self.diags.push(Diag::ty(
                        align_span,
                        format!(
                            "ring_layout `{}`: framing `align {}` must be a power \
                             of two — it's the record-stride alignment the reader \
                             masks with",
                            r.name.name, align
                        ),
                    ));
                }
                let len_width =
                    fr.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                        ("len_prefix", RingAttrValue::Ident(id)) => repr_bytes(&id.name),
                        _ => None,
                    });
                // The length prefix must fit within one alignment unit, or
                // a record near the wrap boundary can land its header in
                // (cap - len_prefix_width, cap) → an OOB read/write. With
                // `len_prefix_width <= align` and `cap % align == 0`, every
                // record header is fully inside the data region.
                if let Some(w) = len_width {
                    if align >= 1 && w > align {
                        self.diags.push(Diag::ty(
                            align_span,
                            format!(
                                "ring_layout `{}`: framing `len_prefix` width ({} \
                                 bytes) exceeds `align` ({}) — the record header \
                                 could straddle the wrap boundary; set `align` to at \
                                 least the len-prefix width",
                                r.name.name, w, align
                            ),
                        ));
                    }
                }
                let pad =
                    fr.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                        ("pad_sentinel", RingAttrValue::Int(n)) => Some((*n, a.span)),
                        _ => None,
                    });
                if let (Some(w), Some((pad, span))) = (len_width, pad) {
                    let max: u64 = if w >= 8 { u64::MAX } else { (1u64 << (w * 8)) - 1 };
                    if pad < 0 || (pad as u64) > max {
                        self.diags.push(Diag::ty(
                            span,
                            format!(
                                "ring_layout `{}`: `pad_sentinel {:#x}` does not fit \
                                 in the `len_prefix` width ({} byte{}) — wrap \
                                 detection would read a truncated value and never \
                                 trigger",
                                r.name.name,
                                pad,
                                w,
                                if w == 1 { "" } else { "s" }
                            ),
                        ));
                    }
                }
            }
        }

        // --- Frontend hardening (2026-06-08) ---
        let is_byte_records =
            r.framing.as_ref().map(|f| f.kind.name == "byte_records").unwrap_or(false);
        let is_slots =
            r.framing.as_ref().map(|f| f.kind.name == "slots").unwrap_or(false);

        // Unaligned atomic cursor: an `atomic_u64` cursor whose `at` is
        // not 8-aligned makes the runtime's atomic load undefined (torn /
        // misaligned). This is the one genuinely UB-adjacent frontend gap.
        for c in &r.cursors {
            let repr = c.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                ("repr", RingAttrValue::Ident(id)) => Some(id.name.as_str()),
                _ => None,
            });
            let at = c.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                ("at", RingAttrValue::Int(n)) => Some(*n),
                _ => None,
            });
            if repr == Some("atomic_u64") {
                if let Some(at) = at {
                    if at % 8 != 0 {
                        self.diags.push(Diag::ty(
                            c.span,
                            format!(
                                "ring_layout `{}`: an `atomic_u64` cursor's `at` \
                                 offset ({}) must be 8-byte aligned — an unaligned \
                                 atomic load is undefined",
                                r.name.name, at
                            ),
                        ));
                    }
                }
            }
            // Cursor `unit` must agree with the framing kind: byte cursors
            // for `byte_records`, slot cursors for `slots`.
            let unit = c.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                ("unit", RingAttrValue::Ident(id)) => Some(id.name.as_str()),
                _ => None,
            });
            if let (Some(unit), Some(fr)) = (unit, &r.framing) {
                let ok = (unit == "bytes" && fr.kind.name == "byte_records")
                    || (unit == "slots" && fr.kind.name == "slots");
                if !ok && (unit == "bytes" || unit == "slots") {
                    self.diags.push(Diag::ty(
                        c.span,
                        format!(
                            "ring_layout `{}`: cursor `unit {}` doesn't match \
                             `framing {}` (expected `bytes` with `byte_records`, \
                             `slots` with `slots`)",
                            r.name.name, unit, fr.kind.name
                        ),
                    ));
                }
            }
            // Typo-safety: warn on unrecognized cursor attr keys.
            for a in &c.attrs {
                if !matches!(a.key.name.as_str(),
                    "at" | "repr" | "load" | "store" | "unit" | "kind") {
                    self.diags.push(Diag::warn(
                        a.span,
                        format!(
                            "ring_layout `{}`: unknown cursor attribute `{}` \
                             (recognized: at, repr, load, store, unit, kind)",
                            r.name.name, a.key.name
                        ),
                    ));
                }
            }
        }

        // Presence: the runtime needs a source for each of these. Today a
        // layout can omit them and still typecheck, leaving the consumer
        // with nothing to read.
        if r.magic.is_none() {
            self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: needs a `magic` value — without it the \
                     consumer cannot validate it attached the right segment",
                    r.name.name
                ),
            ));
        }
        if (is_byte_records || is_slots) && r.data_at.is_none() {
            self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: framing needs `data_at` (the first \
                     record/slot offset)",
                    r.name.name
                ),
            ));
        }
        // byte_records reads the data-region capacity from a `buffer_size`
        // scalar; slots derives it from slot_size × slot_count instead.
        if is_byte_records && !r.scalars.iter().any(|s| s.name.name == "buffer_size") {
            self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: needs a `buffer_size` scalar — the consumer \
                     reads the ring's data-region capacity from it",
                    r.name.name
                ),
            ));
        }

        // Overflow policy must be one of the known kinds.
        if let Some(ov) = &r.overflow {
            const POLICIES: &[&str] = &["lap_detect", "block", "drop", "fail"];
            if !POLICIES.contains(&ov.name.as_str()) {
                self.diags.push(Diag::ty(
                    ov.span,
                    format!(
                        "ring_layout `{}`: unknown `overflow {}` (expected one of \
                         lap_detect, block, drop, fail)",
                        r.name.name, ov.name
                    ),
                ));
            }
        }

        // Typo-safety: warn on unrecognized framing attr keys.
        if let Some(fr) = &r.framing {
            for a in &fr.attrs {
                if !matches!(a.key.name.as_str(),
                    "len_prefix" | "align" | "pad_sentinel" | "slot_size"
                    | "record_header_bytes" | "pad_field_offset"
                    | "pad_field_width" | "pad_field_value" | "recheck"
                    | "seq_offset" | "seq_width"
                    | "kernel_ns_offset" | "kernel_ns_width"
                    | "user_ns_offset" | "user_ns_width") {
                    self.diags.push(Diag::warn(
                        a.span,
                        format!(
                            "ring_layout `{}`: unknown framing attribute `{}` \
                             (recognized: len_prefix, align, pad_sentinel, \
                             slot_size, record_header_bytes, pad_field_offset, \
                             pad_field_width, pad_field_value, recheck)",
                            r.name.name, a.key.name
                        ),
                    ));
                }
            }
        }

        // #5 (fast-protocol-I/O): validate the record_header / pad_field /
        // recheck attrs (byte_records). record_header_bytes is the fixed
        // per-record header before the payload; it must be a multiple of
        // `align` (so the reader's align_up(hdr + len, align) stride equals
        // header + align(len)) and at least len_prefix wide (the len field
        // lives at record offset 0). A pad_field (a header discriminant
        // marking a tail pad, e.g. the reference crate kind==1) must fit inside the
        // header. `recheck` only knows `post_copy`.
        if let Some(fr) = &r.framing {
            let attr_int = |key: &str| -> Option<i64> {
                fr.attrs.iter().find_map(|a| match (&a.key.name[..], &a.value) {
                    (k, RingAttrValue::Int(n)) if k == key => Some(*n),
                    _ => None,
                })
            };
            let align = attr_int("align").unwrap_or(1).max(1) as u64;
            let len_prefix_w = fr
                .attrs
                .iter()
                .find(|a| a.key.name == "len_prefix")
                .and_then(|a| match &a.value {
                    RingAttrValue::Ident(id) => Some(match id.name.as_str() {
                        "u8" | "i8" => 1u64,
                        "u16" | "i16" => 2,
                        "u32" | "i32" | "f32" => 4,
                        "u64" | "i64" | "f64" => 8,
                        _ => 0,
                    }),
                    RingAttrValue::Int(n) => Some(*n as u64),
                })
                .unwrap_or(0);
            let rh = attr_int("record_header_bytes");
            if let Some(rhv) = rh {
                let span = fr.span;
                if rhv <= 0 {
                    self.diags.push(Diag::ty(span, format!(
                        "ring_layout `{}`: record_header_bytes must be positive",
                        r.name.name)));
                } else {
                    let rhu = rhv as u64;
                    if align > 1 && rhu % align != 0 {
                        self.diags.push(Diag::ty(span, format!(
                            "ring_layout `{}`: record_header_bytes ({}) must be a \
                             multiple of align ({}) — the record stride is \
                             header + align(len)", r.name.name, rhu, align)));
                    }
                    if len_prefix_w != 0 && rhu < len_prefix_w {
                        self.diags.push(Diag::ty(span, format!(
                            "ring_layout `{}`: record_header_bytes ({}) is smaller \
                             than len_prefix ({}) — the length field at record \
                             offset 0 would not fit", r.name.name, rhu, len_prefix_w)));
                    }
                }
            }
            // pad_field: offset/width/value must be coherent and fit the header.
            let pad_off = attr_int("pad_field_offset");
            let pad_w = attr_int("pad_field_width");
            let pad_v = attr_int("pad_field_value");
            if pad_off.is_some() || pad_w.is_some() || pad_v.is_some() {
                let span = fr.span;
                match (pad_off, pad_w, pad_v) {
                    (Some(off), Some(w), Some(_)) => {
                        if !matches!(w, 1 | 2 | 4 | 8) {
                            self.diags.push(Diag::ty(span, format!(
                                "ring_layout `{}`: pad_field_width must be 1/2/4/8, got {}",
                                r.name.name, w)));
                        }
                        if off < 0 {
                            self.diags.push(Diag::ty(span, format!(
                                "ring_layout `{}`: pad_field_offset must be non-negative",
                                r.name.name)));
                        } else if let Some(rhv) = rh {
                            if rhv > 0 && (off + w) as i64 > rhv {
                                self.diags.push(Diag::ty(span, format!(
                                    "ring_layout `{}`: pad_field [{}, {}) falls outside \
                                     the {}-byte record header",
                                    r.name.name, off, off + w, rhv)));
                            }
                        }
                    }
                    _ => self.diags.push(Diag::ty(span, format!(
                        "ring_layout `{}`: a pad_field needs all of pad_field_offset, \
                         pad_field_width, pad_field_value", r.name.name))),
                }
            }
            // recheck: only post_copy is known.
            for a in &fr.attrs {
                if a.key.name == "recheck" {
                    let ok = matches!(&a.value, RingAttrValue::Ident(id) if id.name == "post_copy");
                    if !ok {
                        self.diags.push(Diag::ty(a.span, format!(
                            "ring_layout `{}`: unknown recheck mode (only `post_copy`)",
                            r.name.name)));
                    }
                }
            }
            // #5 follow-on: in-band header field offsets/widths
            // (seq/kernel_ns/user_ns) must fit inside the record header.
            for (off_key, w_key) in [
                ("seq_offset", "seq_width"),
                ("kernel_ns_offset", "kernel_ns_width"),
                ("user_ns_offset", "user_ns_width"),
            ] {
                let off = attr_int(off_key);
                let w = attr_int(w_key);
                if off.is_none() && w.is_none() {
                    continue;
                }
                let span = fr.span;
                match (off, w) {
                    (Some(o), Some(width)) => {
                        if !matches!(width, 1 | 2 | 4 | 8) {
                            self.diags.push(Diag::ty(span, format!(
                                "ring_layout `{}`: {} must be 1/2/4/8, got {}",
                                r.name.name, w_key, width)));
                        }
                        if o < 0 {
                            self.diags.push(Diag::ty(span, format!(
                                "ring_layout `{}`: {} must be non-negative",
                                r.name.name, off_key)));
                        } else if let Some(rhv) = rh {
                            if rhv > 0 && (o + width) as i64 > rhv {
                                self.diags.push(Diag::ty(span, format!(
                                    "ring_layout `{}`: header field [{}, {}) falls \
                                     outside the {}-byte record header",
                                    r.name.name, o, o + width, rhv)));
                            }
                        }
                    }
                    _ => self.diags.push(Diag::ty(span, format!(
                        "ring_layout `{}`: header field `{}` needs both {} and {}",
                        r.name.name, off_key.trim_end_matches("_offset"),
                        off_key, w_key))),
                }
            }
            // A declared in-band header field requires a record_header_bytes
            // (otherwise the reader's header is only the length prefix and
            // the field reads past the record — silent corruption), and no
            // two header fields (or a field and the length prefix at offset
            // 0) may overlap.
            let mut intervals: Vec<(&str, i64, i64)> = Vec::new();
            if len_prefix_w > 0 {
                intervals.push(("len_prefix", 0, len_prefix_w as i64));
            }
            let mut any_header_field = false;
            for (name, off_key, w_key) in [
                ("pad_field", "pad_field_offset", "pad_field_width"),
                ("seq", "seq_offset", "seq_width"),
                ("kernel_ns", "kernel_ns_offset", "kernel_ns_width"),
                ("user_ns", "user_ns_offset", "user_ns_width"),
            ] {
                if let (Some(o), Some(w)) = (attr_int(off_key), attr_int(w_key)) {
                    any_header_field = true;
                    if o >= 0 && matches!(w, 1 | 2 | 4 | 8) {
                        intervals.push((name, o, o + w));
                    }
                }
            }
            if any_header_field && rh.map_or(true, |v| v <= 0) {
                self.diags.push(Diag::ty(fr.span, format!(
                    "ring_layout `{}`: record_header_bytes must be declared \
                     (and positive) when in-band header fields (pad_field / \
                     seq / kernel_ns / user_ns) are used — without it the \
                     reader's header is only the length prefix and the field \
                     would read past the record",
                    r.name.name)));
            }
            for i in 0..intervals.len() {
                for j in (i + 1)..intervals.len() {
                    let (an, a0, a1) = intervals[i];
                    let (bn, b0, b1) = intervals[j];
                    if a0 < b1 && b0 < a1 {
                        self.diags.push(Diag::ty(fr.span, format!(
                            "ring_layout `{}`: header fields `{}` [{}, {}) and \
                             `{}` [{}, {}) overlap",
                            r.name.name, an, a0, a1, bn, b0, b1)));
                    }
                }
            }
        }
    }

    /// GH #734: a declared member may not take the spelling of a
    /// synthetic one (`children` / `k_max` / `draining`). Reported
    /// at the declaration, where the rename happens — reads of such
    /// a member resolve to the synthetic member, so the collision
    /// otherwise surfaces as an unrelated error at a use site, or
    /// (when the declared type matches the synthetic one) not until
    /// codegen.
    fn check_reserved_member_names(&mut self, decl: &LocusDecl) {
        for member in &decl.members {
            match member {
                LocusMember::Params(pb) => {
                    for p in &pb.params {
                        reserved_member_diag(
                            &mut self.diags,
                            "params field",
                            &p.name,
                        );
                    }
                }
                LocusMember::Fn(f) => {
                    reserved_member_diag(&mut self.diags, "method", &f.name);
                }
                LocusMember::Capacity(cb) => {
                    for slot in &cb.slots {
                        reserved_member_diag(
                            &mut self.diags,
                            "capacity slot",
                            &slot.name,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// GH #756: `type` is a TOP-LEVEL declaration. The parser accepts
    /// one inside a locus body (`LocusMember::Type`) and the checker
    /// ignored it, so a locus-level `type` passed `hale check` — and
    /// then codegen, which has no lowering for the member, refused
    /// the whole program with `locus L member kind not yet lowered to
    /// codegen`: a program the gate accepted could not be built, and
    /// the message named no line. Nothing could USE the declaration
    /// either: the resolver never registers a member type, so the
    /// name it introduces is invisible everywhere, including inside
    /// the locus that declares it.
    ///
    /// Rejecting it at the declaration is the fix rather than
    /// lowering it, for the reasons that decided the sibling member
    /// `const` (GH #747): a namespaced type has no settled spelling
    /// (`Holder::Pair` from outside, bare `Pair` inside), it would
    /// need new resolution in the checker AND in codegen, and it
    /// lands on the generic-monomorph and cross-seed rename paths
    /// that already walk a member type's field types. A top-level
    /// `type` is in scope everywhere in the seed and is what the
    /// author wanted.
    fn check_no_member_types(&mut self, decl: &LocusDecl) {
        for member in &decl.members {
            let LocusMember::Type(t) = member else {
                continue;
            };
            // At the `type` keyword: `TypeDecl::span` runs from the
            // keyword to the declaration's end in all three forms
            // (struct, alias, enum), so the declaration's first
            // token is the one line that has to change.
            let kw = Span::new(
                t.span.start.as_usize(),
                t.span.start.as_usize() + "type".len(),
            );
            self.diags.push(Diag::ty(
                kw,
                format!(
                    "`type {n}` is declared inside locus `{l}`: `type` is a \
                     top-level declaration, not a locus member. Move it \
                     above the locus — a top-level `type` is in scope \
                     everywhere in the seed, including inside every locus.",
                    n = t.name.name,
                    l = decl.name.name,
                ),
            ));
        }
    }

    /// GH #747: `const` is a TOP-LEVEL declaration. The parser
    /// accepts one inside a locus body (`LocusMember::Const`) and
    /// the checker used to typecheck its value, so a locus-level
    /// const passed `hale check` — and then codegen, which has no
    /// lowering for the member, refused the whole program with
    /// `locus L member kind not yet lowered to codegen`: a program
    /// the gate accepted could not be built, and the message named
    /// no line.
    ///
    /// Rejecting it at the declaration is the fix rather than
    /// lowering it: a locus-level const has no settled scope or
    /// spelling (`L::name` from outside, bare `name` inside, and
    /// `self.name` — which it is NOT, a const being per-type and not
    /// per-instance state), and lowering would have to answer all
    /// three in the checker AND in codegen, plus the generic-locus
    /// monomorph path that already walks a member const's type. The
    /// two things the author wanted both exist: a top-level `const`
    /// is in scope inside every locus of the seed, and a `params`
    /// field with a default gives each instance its own copy.
    fn check_no_member_consts(&mut self, decl: &LocusDecl) {
        for member in &decl.members {
            let LocusMember::Const(c) = member else {
                continue;
            };
            // At the `const` keyword: `ConstDecl::span` runs from
            // the keyword to the `;`, so the declaration's first
            // token is the one line that has to change.
            let kw = Span::new(
                c.span.start.as_usize(),
                c.span.start.as_usize() + "const".len(),
            );
            self.diags.push(Diag::ty(
                kw,
                format!(
                    "`const {n}` is declared inside locus `{l}`: `const` is \
                     a top-level declaration, not a locus member. Move it \
                     above the locus — a top-level `const` is in scope \
                     inside every locus of the seed — or, if each instance \
                     should carry its own, make it a params field with a \
                     default (`params {{ {n}: ... = ...; }}`).",
                    n = c.name.name,
                    l = decl.name.name,
                ),
            ));
        }
    }

    fn check_locus(&mut self, decl: &'a LocusDecl) {
        // GH #734 — reserved member names. Runs before the symbol
        // lookup below so it fires for every parsed locus.
        self.check_reserved_member_names(decl);

        // GH #756 — a `type` is not a locus member. Also before the
        // lookup, for the same reason.
        self.check_no_member_types(decl);
        // GH #747 — a `const` is not a locus member. Also before the
        // lookup, for the same reason.
        self.check_no_member_consts(decl);

        let info = match self.top.lookup(&decl.name.name) {
            Some(TopSymbol::Locus(info)) => info,
            _ => return,
        };
        let prev = self.current_locus.replace(info);

        // v1.x-FORM-1: verify the form annotation's shape
        // contract against the declared capacity. PR3 handles
        // shape verification; method synthesis lands in PR3b
        // (so call sites like `l.push(42)` still won't resolve
        // yet — that's expected for this PR).
        if let Some(form) = &decl.form {
            self.check_form_shape(decl, form);
        }

        // Phase 2a: `locus L : serves P` conformance — L must
        // provide every contract method P declares (matching
        // arity + param + return types), the perspective analog
        // of interface structural satisfaction.
        if !decl.serves.is_empty() {
            self.check_serves_conformance(decl);
        }

        // #18.6 — Hale enforces CQRS at the locus boundary:
        // methods on loci may not return locus values. Reject
        // such declarations with a span-targeted diagnostic
        // naming the canonical alternatives (accept-as-child +
        // contract reads, bus topics, delegation). See
        // spec/semantics.md § Locus method dispatch.
        self.check_no_locus_return(decl);

        // Validate that bus-subscribe handlers are declared on
        // the locus body (as fn members).
        let fn_members: BTreeMap<String, &FnDecl> = decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Fn(f) => Some((f.name.name.clone(), f)),
                _ => None,
            })
            .collect();
        for sub in &info.bus_subscribes {
            if !fn_members.contains_key(&sub.handler) {
                self.diags.push(Diag::ty(
                    sub.span,
                    format!(
                        "bus subscribe `{}` references handler `{}` which is \
                         not declared on locus `{}`",
                        sub.subject, sub.handler, info.name
                    ),
                ));
                continue;
            }
            // Open-question #24 MVP (2026-05-25): fallible-handler
            // check. Bus dispatch has no caller frame to address
            // a value return — a fallible handler would have
            // nowhere to send `out_err` to. So a fn that's
            // fallible-by-decl can't also be subscribed; the
            // subscription site is rejected, not the fn (one
            // fn may be referenced by zero or more handlers,
            // but each subscription points at one fn).
            if let Some(handler_fn) = fn_members.get(&sub.handler) {
                if let Some(payload_te) = &handler_fn.fallible {
                    self.diags.push(Diag::ty(
                        sub.span,
                        format!(
                            "bus subscribe `{}` references fn `{}` which \
                             declares `fallible({})` — bus-subscribed \
                             handlers can't be fallible because bus \
                             dispatch has no caller frame to address the \
                             error channel. Drop `fallible(E)` from the \
                             handler and route value-error structurally \
                             via an inline closure (a closure assertion \
                             firing into `on_failure`), or do the work in \
                             a separate fallible fn the handler calls and \
                             address the error inside the handler body \
                             with `or <disposition>`.",
                            sub.subject,
                            sub.handler,
                            crate::resolve::resolve_type_expr(
                                payload_te,
                                self.known,
                            )
                            .display(),
                        ),
                    ));
                }
                // Downstream handoff (2026-08-11), SOUNDNESS: the
                // handler's parameter type was never compared
                // against the subject's payload — any type was
                // accepted, and the published value reinterpreted
                // field-by-field at the handler (a String field
                // read through an Int parameter surfaced its heap
                // pointer, from safe code, with both gates green).
                // The string-subject `of type` conflict check
                // already models this hazard cross-site; this is
                // the same comparison at the handler boundary,
                // covering BOTH subject forms. `Ty::Unknown` on
                // either side stays permissive — cross-seed topics,
                // stdlib paths, and `Drain<T>` batch handlers all
                // resolve Unknown here by design.
                // GH #1108: a second `std::api::Context` parameter is the
                // handler asking for the caller. Dispatch hands it the
                // local context; the api binding hands it the caller it
                // established. A topic bound to a transport reaches the
                // handler from another process as `local`, which must
                // never read as trust, so that combination is refused.
                let ctx_param = match handler_fn.params.as_slice() {
                    [_, c] if hale_syntax::api_gen::is_context_type(&c.ty) => Some(c),
                    _ => None,
                };
                if let Some(c) = ctx_param {
                    if self.bound_topics.contains(&sub.subject) {
                        self.diags.push(Diag::ty(
                            c.span,
                            format!(
                                "bus subscribe `{}` handler `{}` takes a \
                                 `std::api::Context`, but the topic is bound to a \
                                 transport in `bindings {{ }}`: a message from another \
                                 process would reach it as the local principal, and \
                                 `local` never stands for trust. Drop the context \
                                 parameter, or reach the topic through the api \
                                 binding instead",
                                sub.subject, sub.handler
                            ),
                        ));
                    }
                    if let TypeExpr::Named { path, .. } = &handler_fn.params[0].ty {
                        if path.segments.len() == 1 && path.segments[0].name == "Drain" {
                            self.diags.push(Diag::ty(
                                c.span,
                                format!(
                                    "bus subscribe `{}` handler `{}`: a `Drain<T>` batch \
                                     handler cannot take a `std::api::Context` yet — a \
                                     batch over a ring has no single caller until bulk \
                                     requests land",
                                    sub.subject, sub.handler
                                ),
                            ));
                        }
                    }
                }
                if !matches!(sub.payload, Ty::Unknown) {
                    let shape: &[hale_syntax::ast::Param] = match handler_fn.params.as_slice() {
                        [p, _] if ctx_param.is_some() => std::slice::from_ref(p),
                        other => other,
                    };
                    match shape {
                        [p] => {
                            // A `Drain<T>` batch handler receives the
                            // same payload per element — compare T.
                            let (payload_te, is_drain) = match &p.ty {
                                TypeExpr::Named {
                                    path, generic_args, ..
                                } if path.segments.len() == 1
                                    && path.segments[0].name == "Drain"
                                    && generic_args.len() == 1 =>
                                {
                                    (&generic_args[0], true)
                                }
                                other => (other, false),
                            };
                            let got = crate::resolve::resolve_type_expr(
                                payload_te, self.known,
                            );
                            let got_display = if is_drain {
                                format!("Drain<{}>", got.display())
                            } else {
                                got.display()
                            };
                            if matches!(got, Ty::Unknown) {
                                // The mistake that led here: the
                                // TOPIC name used as the parameter
                                // type (`subscribe Hello as on_h` +
                                // `fn on_h(msg: Hello)`). Un-checked
                                // it survived to codegen as an
                                // `unknown type name` — mangled and
                                // ungreppable across a seed boundary.
                                if let TypeExpr::Named {
                                    path,
                                    generic_args,
                                    ..
                                } = payload_te
                                {
                                    if generic_args.is_empty()
                                        && path.segments.len() == 1
                                        && matches!(
                                            self.top.lookup(
                                                &path.segments[0].name
                                            ),
                                            Some(TopSymbol::Topic(_))
                                        )
                                    {
                                        self.diags.push(Diag::ty(
                                            p.ty.span(),
                                            format!(
                                                "bus subscribe `{}` handler \
                                                 `{}`: `{}` is the topic, \
                                                 not a type — the handler \
                                                 receives the topic's \
                                                 payload; declare the \
                                                 parameter as `{}`",
                                                sub.subject,
                                                sub.handler,
                                                path.segments[0].name,
                                                sub.payload.display(),
                                            ),
                                        ));
                                    }
                                }
                            } else if got != sub.payload {
                                self.diags.push(Diag::ty(
                                    p.ty.span(),
                                    format!(
                                        "bus subscribe `{}` handler `{}` \
                                         takes `{}`, but the subject \
                                         carries payload `{}` — the \
                                         mismatch would reinterpret the \
                                         payload's bytes as the wrong \
                                         type at runtime",
                                        sub.subject,
                                        sub.handler,
                                        got_display,
                                        sub.payload.display(),
                                    ),
                                ));
                            }
                        }
                        params => {
                            self.diags.push(Diag::ty(
                                handler_fn.name.span,
                                format!(
                                    "bus subscribe `{}` handler `{}` must \
                                     take the payload `{}` (or `Drain<T>`), \
                                     optionally followed by `ctx: \
                                     std::api::Context`, but takes {} \
                                     parameters",
                                    sub.subject,
                                    sub.handler,
                                    sub.payload.display(),
                                    params.len(),
                                ),
                            ));
                        }
                    }
                }
            }
        }

        // F.8: contract compatibility. If this locus consumes
        // fields from coordinatees, the accept-child type
        // must expose each consumed field at a compatible
        // type. The check fires once per parent locus; the
        // child's expose-set must be a superset (by name) of
        // the parent's consume-set, with assignable types.
        if !info.contract_consume.is_empty() {
            self.check_contract_compatibility(info);
        }
        // M3 stage 4 (2026-07-02): expose-side validity. Codegen
        // treats `contract` members as pure declaration, so this is
        // the ONLY place a lying expose can be caught — an entry
        // must bind against something real on this locus (a params
        // field, a mode, or a fn member) at a matching type, or a
        // consuming parent type-checks against fiction.
        if !info.contract_expose.is_empty() {
            self.check_contract_expose_validity(info);
        }

        // F.31 (2026-05-23): validate the `placement { }` block
        // when present. The parser already enforced "main-only"
        // and required-Ident keys; here we check that each entry
        // references an actual main-locus `params` field whose
        // type is a locus. Pinned-restrictions (no accept(),
        // no closures) are checked at codegen time when
        // placement → runtime wiring fires.
        let placement_blocks: Vec<_> = decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Placement(pb) => Some(pb),
                _ => None,
            })
            .collect();
        if placement_blocks.len() > 1 {
            self.diags.push(Diag::ty(
                placement_blocks[1].span,
                format!(
                    "locus `{}` declares multiple `placement {{ }}` blocks; \
                     at most one is permitted",
                    info.name
                ),
            ));
        }
        // Topology Phase 1b: the (declare-only) `topology { }`
        // block that `pinned(node =)` / `pinned(l3 =)` resolve
        // against. Validate its internal consistency, then thread
        // it into placement validation for reference checking.
        let topology_blocks: Vec<_> = decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Topology(tb) => Some(tb),
                _ => None,
            })
            .collect();
        if topology_blocks.len() > 1 {
            self.diags.push(Diag::ty(
                topology_blocks[1].span,
                format!(
                    "locus `{}` declares multiple `topology {{ }}` blocks; \
                     at most one is permitted",
                    info.name
                ),
            ));
        }
        let topology = topology_blocks.first().copied();
        if let Some(tb) = topology {
            self.check_topology_block(info, tb);
        }
        if let Some(pb) = placement_blocks.first() {
            self.check_placement_block(info, pb, topology);
        }

        // 2026-06-01: `release(c: T)` is the death-side bookend of
        // `accept(c: T)` — it fires when an accept'd child of type
        // T completes, and declaring it marks T a "flow". Without a
        // matching `accept(c: T)` the locus never owns a T child, so
        // the release can never fire: it's a dead declaration and
        // almost always a mistake (wrong child type, or the author
        // forgot the `accept`). Reject it with a focused diagnostic.
        for member in &decl.members {
            if let LocusMember::Lifecycle(lc) = member {
                if lc.kind == LifecycleKind::Release {
                    let child_name = lc.params.first().and_then(|p| {
                        match &p.ty {
                            TypeExpr::Named { path, .. } => path
                                .segments
                                .last()
                                .map(|s| s.name.clone()),
                            _ => None,
                        }
                    });
                    match child_name {
                        Some(name) if locus_accepts(decl, &name) => {}
                        Some(name) => self.diags.push(Diag::ty(
                            lc.span,
                            format!(
                                "locus `{}` declares `release(c: {})` but has \
                                 no matching `accept(c: {})` — release is the \
                                 death-side bookend of accept and can only \
                                 fire for an accept'd child type",
                                info.name, name, name
                            ),
                        )),
                        None => {}
                    }
                }
            }
        }

        // GH #877: `locus Cache<K, V>`'s parameters are in scope for
        // every annotation its members write — a `params` field, a
        // method signature, a capacity slot. They name no
        // declaration by design (codegen monomorphizes at the use
        // site), so the unknown-bare-type-name rule has to hold them
        // while the members are walked.
        let prev_generics = std::mem::replace(
            &mut self.generic_params,
            decl.generics.iter().map(|g| g.name.name.clone()).collect(),
        );
        for member in &decl.members {
            self.check_locus_member(member);
        }
        self.generic_params = prev_generics;

        self.current_locus = prev;
    }

    /// F.31: validate a `placement { field: spec; }` block on
    /// `main locus`. Each entry must:
    ///   1. Reference a declared `params` field on this locus
    ///      (the parser only enforces "main-only" and Ident
    ///      keying).
    ///   2. The referenced field must be a locus type —
    ///      placement applies only to locus instances, not
    ///      primitives or structs.
    ///   3. No duplicate field keys.
    ///
    /// Pinned-class restrictions (no `accept()`, no closures
    /// on a locus placed `pinned`) move to placement-time
    /// enforcement in Phase 3 codegen; the spec lock is here
    /// but the typecheck implementation is deferred until
    /// codegen reads placement.
    fn check_placement_block(
        &mut self,
        info: &crate::symbol::LocusInfo,
        pb: &hale_syntax::ast::PlacementBlock,
        topology: Option<&hale_syntax::ast::TopologyBlock>,
    ) {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for entry in &pb.entries {
            // (3) duplicate check
            if !seen.insert(entry.field.name.clone()) {
                self.diags.push(Diag::ty(
                    entry.span,
                    format!(
                        "placement entry: duplicate field `{}` (each \
                         field may have at most one placement spec)",
                        entry.field.name
                    ),
                ));
                continue;
            }
            // (1) field exists in this locus's params
            let param = info
                .params
                .iter()
                .find(|p| p.name == entry.field.name);
            let param = match param {
                Some(p) => p,
                None => {
                    self.diags.push(Diag::ty(
                        entry.field.span,
                        format!(
                            "placement entry: field `{}` is not declared in \
                             locus `{}`'s params block",
                            entry.field.name, info.name
                        ),
                    ));
                    continue;
                }
            };
            // (2) field's type must be a locus type. `Ty::Named(L)`
            // where L resolves to a `TopSymbol::Locus`. Unknown
            // is permissive (cross-seed or stdlib loci resolve to
            // Unknown — match the existing assignable_from rule).
            match &param.ty {
                Ty::Named(name) => {
                    let is_locus = matches!(
                        self.top.lookup(name),
                        Some(TopSymbol::Locus(_))
                    );
                    let is_unknown_external = !self.top.symbols.contains_key(name);
                    if !is_locus && !is_unknown_external {
                        self.diags.push(Diag::ty(
                            entry.field.span,
                            format!(
                                "placement entry: field `{}` has type `{}` \
                                 which is not a locus type; placement applies \
                                 only to locus instances",
                                entry.field.name,
                                param.ty.display()
                            ),
                        ));
                    }
                }
                Ty::Unknown => {
                    // Cross-seed / stdlib locus — be permissive,
                    // matching assignable_from's Unknown rule.
                }
                other => {
                    self.diags.push(Diag::ty(
                        entry.field.span,
                        format!(
                            "placement entry: field `{}` has type `{}` \
                             which is not a locus type; placement applies \
                             only to locus instances",
                            entry.field.name,
                            other.display()
                        ),
                    ));
                }
            }
            // Topology Phase 1a: `pinned(cores = ...)` static
            // validity. The spec is closed-world (literal bounds),
            // so an empty range or a duplicated set element is a
            // definite authoring error, catchable here. Whether
            // the cores exist on the deploy box stays best-effort
            // at runtime (same contract as `pinned(core = N)` on
            // a smaller CI machine).
            if let PlacementSpec::Pinned {
                affinity: PinAffinity::Cores(spec),
                ..
            } = &entry.spec
            {
                match spec {
                    CoreSpec::Range { lo, hi, inclusive } => {
                        let empty =
                            if *inclusive { hi < lo } else { hi <= lo };
                        if empty {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: `cores = {}..{}{}` \
                                     selects no cores — the range is empty. \
                                     (`A..B` excludes B; use `A..=B` to \
                                     include it.)",
                                    entry.field.name,
                                    lo,
                                    if *inclusive { "=" } else { "" },
                                    hi,
                                ),
                            ));
                        }
                    }
                    CoreSpec::Set(v) => {
                        let mut sorted = v.clone();
                        sorted.sort_unstable();
                        sorted.dedup();
                        if sorted.len() != v.len() {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: duplicate core \
                                     index in `cores = {{...}}` set",
                                    entry.field.name
                                ),
                            ));
                        }
                    }
                    CoreSpec::Single(_) => {}
                }
            }

            // Topology Phase 1b: `pinned(node = N)` / `pinned(l3 =
            // name)` must reference a domain declared in the
            // `topology { }` block. Using either with no topology
            // block, or naming an undeclared node/domain, is a
            // definite authoring error (closed-world resolution).
            if let PlacementSpec::Pinned { affinity, replicas } =
                &entry.spec
            {
                // Topology Phase 1c: `replicas = K` must be >= 1.
                // `Some(0)` / negative fans out nothing (or is a
                // typo); `None` / `Some(1)` is a single instance.
                if let Some(k) = replicas {
                    if *k < 1 {
                        self.diags.push(Diag::ty(
                            entry.span,
                            format!(
                                "placement entry `{}`: `replicas = {}` must \
                                 be at least 1 (it fans the locus into K \
                                 single-threaded instances)",
                                entry.field.name, k
                            ),
                        ));
                    }
                }
                match affinity {
                    PinAffinity::Node(n) => match topology {
                        None => self.diags.push(Diag::ty(
                            entry.span,
                            format!(
                                "placement entry `{}`: `pinned(node = {})` \
                                 needs a `topology {{ }}` block to resolve \
                                 the node to its cores — none is declared \
                                 on `{}`",
                                entry.field.name, n, info.name
                            ),
                        )),
                        Some(tb) if tb.node_cores(*n).is_none() => {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: `pinned(node = \
                                     {})` references NUMA node `{}`, which \
                                     the `topology {{ }}` block does not \
                                     declare",
                                    entry.field.name, n, n
                                ),
                            ));
                        }
                        Some(tb)
                            if tb
                                .node_cores(*n)
                                .map(|c| c.is_empty())
                                .unwrap_or(false) =>
                        {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: `pinned(node = \
                                     {})` resolves to no cores — node `{}` \
                                     declares no `l3` domains",
                                    entry.field.name, n, n
                                ),
                            ));
                        }
                        _ => {}
                    },
                    PinAffinity::L3(name) => match topology {
                        None => self.diags.push(Diag::ty(
                            entry.span,
                            format!(
                                "placement entry `{}`: `pinned(l3 = {})` \
                                 needs a `topology {{ }}` block to resolve \
                                 the cache domain to its cores — none is \
                                 declared on `{}`",
                                entry.field.name, name.name, info.name
                            ),
                        )),
                        Some(tb) if tb.l3_cores(&name.name).is_none() => {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: `pinned(l3 = {})` \
                                     references cache domain `{}`, which the \
                                     `topology {{ }}` block does not declare",
                                    entry.field.name, name.name, name.name
                                ),
                            ));
                        }
                        _ => {}
                    },
                    PinAffinity::Any | PinAffinity::Cores(_) => {}
                }
            }

            // (The dead-bus-receiver check moved to
            // `check_cooperative_pool_blocking` and was corrected: a
            // non-main cooperative subscriber is dead only when its
            // `run()` ALSO makes a blocking call that starves the pool
            // thread — placement alone over-fires on event-driven
            // subscribers, which receive fine. See that fn.)

            // F.35: per-entry constraint validity.
            for c in &entry.constraints {
                match c.kind {
                    PlacementConstraint::AsyncIo => {
                        // The async_io pool backend is epoll (Linux)
                        // or kqueue (macOS, GH #970) over ucontext
                        // coroutines. Where the TARGET's runtime has
                        // none — musl, whose libc declares ucontext and
                        // implements nothing — reject `where async_io`
                        // at compile time with actionable guidance,
                        // mirroring the wasm-target stdlib gating: the
                        // C runtime's async_io functions are inert stubs
                        // there (LOTUS_HAVE_ASYNC_IO == 0), so this is
                        // the clean-failure path (vs a link error). The
                        // target, not the host: this asked
                        // `cfg!(target_os = "macos")` until GH #970.
                        if !self.target_has_async_io {
                            self.diags.push(Diag::ty(
                                c.span,
                                format!(
                                    "placement entry `{}`: `async_io` pools \
                                     aren't supported on {} yet — use a \
                                     cooperative pool (drop `where async_io`), \
                                     or build for glibc Linux or macOS. (The \
                                     backend is epoll or kqueue over ucontext \
                                     coroutines; this target's libc has no \
                                     ucontext.)",
                                    entry.field.name, self.target_label
                                ),
                            ));
                        }
                        match &entry.spec {
                            PlacementSpec::Pinned { .. } => {
                                self.diags.push(Diag::ty(
                                    c.span,
                                    format!(
                                        "placement entry `{}`: `where async_io` \
                                         is not valid on a pinned placement. \
                                         Pinned loci own their own OS thread \
                                         and have no shared drain loop to \
                                         park on. Use `cooperative(pool = X) \
                                         where async_io` instead.",
                                        entry.field.name
                                    ),
                                ));
                            }
                            PlacementSpec::Cooperative { pool, .. } => {
                                let pool_name = pool
                                    .as_ref()
                                    .map(|i| i.name.as_str())
                                    .unwrap_or("main");
                                if pool_name == "main" {
                                    self.diags.push(Diag::ty(
                                        c.span,
                                        format!(
                                            "placement entry `{}`: `where \
                                             async_io` is not valid on pool \
                                             `main`. The main pool runs \
                                             inline on the binary's primary \
                                             thread, with no dedicated \
                                             worker thread to integrate \
                                             epoll into. Move the field to \
                                             a named cooperative pool (e.g. \
                                             `cooperative(pool = io) where \
                                             async_io`).",
                                            entry.field.name
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        // F.35 cross-entry rule: every entry on the same named
        // cooperative pool must agree on whether the pool runs in
        // async_io mode. Mixing an async_io entry with a non-
        // async_io entry on the same pool is rejected because the
        // pool's worker drain loop is one-or-the-other.
        let mut pool_async_io: BTreeMap<String, bool> = BTreeMap::new();
        let mut pool_first_span: BTreeMap<String, hale_syntax::Span> =
            BTreeMap::new();
        for entry in &pb.entries {
            let pool_name = match &entry.spec {
                PlacementSpec::Cooperative { pool: Some(name), .. } => {
                    name.name.clone()
                }
                _ => continue,
            };
            if pool_name == "main" {
                continue;
            }
            let has_async_io = entry.constraints.iter().any(|c| {
                matches!(c.kind, PlacementConstraint::AsyncIo)
            });
            match pool_async_io.get(&pool_name).copied() {
                None => {
                    pool_async_io.insert(pool_name.clone(), has_async_io);
                    pool_first_span.insert(pool_name, entry.span);
                }
                Some(prev) if prev == has_async_io => {}
                Some(_) => {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "placement entry `{}`: pool `{}` has mixed I/O \
                             modes across placement entries. Every entry on \
                             a pool must either declare `where async_io` or \
                             none must; the pool's worker drain loop is \
                             one-or-the-other. (The pool first appeared at \
                             the entry whose I/O mode is the other; pick \
                             one and apply consistently.)\n\nNote: \
                             `where async_io` governs non-blocking I/O \
                             readiness — it makes blocking `recv`/`accept`/\
                             `send` park-and-resume instead of holding the \
                             thread. It does NOT affect bus delivery or \
                             handler dispatch; if a `subscribe` handler \
                             isn't firing, `async_io` is not the fix (check \
                             the locus's placement instead).",
                            entry.field.name, pool_name
                        ),
                    ));
                }
            }
        }
    }

    /// Topology Phase 1b: validate the `topology { }` block's
    /// internal consistency (declare-only, so everything is
    /// checkable statically):
    ///   - node ids are unique;
    ///   - L3-domain names are globally unique (they're referenced
    ///     by `pinned(l3 = name)` without node qualification);
    ///   - each `cores` spec is well-formed (non-empty range, no
    ///     duplicate set element — same rules as `pinned(cores)`);
    ///   - no core belongs to two L3 domains (ambiguous affinity);
    ///   - no domain core overlaps a `reserve`d core (reserved
    ///     cores are held back for the OS / main).
    /// Whether the cores exist on the deploy box stays best-effort
    /// at runtime — the machine-match is not enforced here.
    fn check_topology_block(
        &mut self,
        info: &crate::symbol::LocusInfo,
        tb: &hale_syntax::ast::TopologyBlock,
    ) {
        let _ = info;
        for spec in &tb.reserved {
            self.check_core_spec_wellformed(spec, tb.span, "reserve cores");
        }
        let reserved: BTreeSet<i64> =
            tb.reserved_cores().into_iter().collect();

        let mut node_ids: BTreeSet<i64> = BTreeSet::new();
        let mut l3_names: BTreeSet<String> = BTreeSet::new();
        let mut core_owner: BTreeMap<i64, String> = BTreeMap::new();

        for node in &tb.nodes {
            if !node_ids.insert(node.id) {
                self.diags.push(Diag::ty(
                    node.id_span,
                    format!(
                        "topology: duplicate NUMA node id `{}` — each \
                         `node N` must have a distinct id",
                        node.id
                    ),
                ));
            }
            for d in &node.domains {
                if !l3_names.insert(d.name.name.clone()) {
                    self.diags.push(Diag::ty(
                        d.name.span,
                        format!(
                            "topology: duplicate L3 domain name `{}` — \
                             domain names are referenced by `pinned(l3 = \
                             {})` and must be globally unique across nodes",
                            d.name.name, d.name.name
                        ),
                    ));
                }
                self.check_core_spec_wellformed(
                    &d.cores,
                    d.span,
                    &format!("l3 {}", d.name.name),
                );
                for c in d.cores.expand() {
                    if let Some(prev) = core_owner.get(&c) {
                        if prev != &d.name.name {
                            self.diags.push(Diag::ty(
                                d.span,
                                format!(
                                    "topology: core {} is claimed by both \
                                     L3 domains `{}` and `{}` — a core \
                                     belongs to at most one cache domain",
                                    c, prev, d.name.name
                                ),
                            ));
                        }
                    } else {
                        core_owner.insert(c, d.name.name.clone());
                    }
                    if reserved.contains(&c) {
                        self.diags.push(Diag::ty(
                            d.span,
                            format!(
                                "topology: core {} is both `reserve`d and \
                                 assigned to L3 domain `{}` — reserved cores \
                                 are held back for the OS / main and can't be \
                                 placed on",
                                c, d.name.name
                            ),
                        ));
                    }
                }
            }
        }
    }

    /// Shared well-formedness check for a `CoreSpec` literal
    /// (used by both `topology { }` domain/reserve specs and the
    /// `pinned(cores)` placement path): a range must select at
    /// least one core, a set must have no duplicate element.
    fn check_core_spec_wellformed(
        &mut self,
        spec: &CoreSpec,
        span: hale_syntax::Span,
        ctx: &str,
    ) {
        match spec {
            CoreSpec::Range { lo, hi, inclusive } => {
                let empty = if *inclusive { hi < lo } else { hi <= lo };
                if empty {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "topology `{}`: `{}..{}{}` selects no cores — \
                             the range is empty (`A..B` excludes B; use \
                             `A..=B` to include it)",
                            ctx,
                            lo,
                            if *inclusive { "=" } else { "" },
                            hi,
                        ),
                    ));
                }
            }
            CoreSpec::Set(v) => {
                let mut sorted = v.clone();
                sorted.sort_unstable();
                sorted.dedup();
                if sorted.len() != v.len() {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "topology `{}`: duplicate core index in the \
                             `{{...}}` set",
                            ctx
                        ),
                    ));
                }
            }
            CoreSpec::Single(_) => {}
        }
    }

    /// #18.6 — Hale enforces CQRS at the locus boundary:
    /// methods on loci may not return locus values. The pattern
    /// `fn factory(...) -> SomeLocus` is rejected at the
    /// declaration site.
    ///
    /// The lotus model treats loci as managed entities — they
    /// live as accepted children of a parent, expose data
    /// through `contract`, communicate cross-tower through the
    /// bus. Returning an entity from a method puts the entity
    /// into a stranger position at every call site (LoD), mixes
    /// command/query semantics (CQRS), and depends on a
    /// concretion rather than an abstraction (Dependency
    /// Inversion). Mechanically, every call leaks via the m90
    /// payload-arena routing.
    ///
    /// The five lenses (SOLID, LoD, CQRS, mechanical sympathy,
    /// the lotus model itself) converge on the same rule: a
    /// method must return data, not an entity. The compiler
    /// enforces it.
    ///
    /// Three canonical remedies in the diagnostic:
    ///   1. Parent-child: `accept(c: T)` + contract reads
    ///   2. Bus topic: publish events; receiver subscribes
    ///   3. Delegation: expose the operation directly on the
    ///      owning locus
    ///
    /// Free fns can still return loci (entity creation —
    /// `std::io::file::open(path) -> File fallible(IoError)`).
    /// Lifecycle methods / modes / failure handlers don't have
    /// return types in the value-bearing sense, so they're
    /// unaffected.
    ///
    /// Spec home: `spec/semantics.md § Locus method dispatch`.
    fn check_no_locus_return(&mut self, decl: &'a LocusDecl) {
        for member in &decl.members {
            let LocusMember::Fn(f) = member else { continue };
            // Walk the declared return type (if any) and the
            // fallible-payload type (if any). Both can carry a
            // locus.
            if let Some(ret) = &f.ret {
                self.report_locus_return(decl, f, ret, "return type");
            }
            if let Some(payload) = &f.fallible {
                // `fallible(L)` carries the error payload type.
                // A locus payload is the same antipattern as a
                // locus return — the caller would have to call
                // methods on the recovered locus, violating the
                // friendship boundary the same way.
                self.report_locus_return(
                    decl, f, payload, "fallible payload type",
                );
            }
        }
    }

    fn report_locus_return(
        &mut self,
        decl: &'a LocusDecl,
        f: &'a FnDecl,
        ty_expr: &'a TypeExpr,
        slot_label: &str,
    ) {
        let resolved = resolve_type_expr(ty_expr, self.known);
        let Ty::Named(name) = &resolved else { return };
        if !matches!(self.top.lookup(name), Some(TopSymbol::Locus(_))) {
            return;
        }
        self.diags.push(Diag::ty(
            ty_expr.span(),
            format!(
                "method `{locus}.{method}` declares {slot} `{ret}` — \
                 methods on loci may not return locus values.\n\n\
                 The lotus model treats loci as managed entities (parent-\
                 child accept, contract exposure, bus topics). Returning \
                 an entity from a method puts it in a stranger position \
                 at every call site (violating LoD), mixes \
                 command/query semantics (CQRS), and depends on a \
                 concretion rather than an abstraction. Mechanically, \
                 every call leaks via the m90 payload-arena routing.\n\n\
                 Rewrite as one of:\n\
                 1. Parent-child: declare `accept(c: {ret})` on the \
                    parent and read via `contract {{ expose ... }}`.\n\
                 2. Bus topic: publish events; receiver subscribes.\n\
                 3. Delegation: expose the operation directly on `{locus}`.\n\n\
                 Free fns can still return loci (entity creation \
                 patterns like `std::io::file::open`). See \
                 spec/semantics.md § Locus method dispatch.",
                locus = decl.name.name,
                method = f.name.name,
                slot = slot_label,
                ret = name,
            ),
        ));
    }

    /// v1.x-FORM-1: verify a `@form(<name>)` annotation's
    /// shape contract against the locus's actual capacity
    /// declaration. v1 ships shape checks for `@form(vec)`
    /// (FORM-2), `@form(hashmap)` (FORM-4), and
    /// `@form(ring_buffer)` (FORM-5).
    fn check_form_shape(&mut self, decl: &'a LocusDecl, form: &'a FormAnnotation) {
        match form.name.name.as_str() {
            "vec" => self.check_form_vec_shape(decl, form),
            "hashmap" => self.check_form_hashmap_shape(decl, form),
            // #353: identical capacity shape to hashmap — the value
            // slot is what makes membership storable. Only the method
            // surface differs.
            "set" => self.check_form_hashmap_shape(decl, form),
            "ring_buffer" => self.check_form_ring_buffer_shape(decl, form),
            "lru_cache" => self.check_form_lru_cache_shape(decl, form),
            other => {
                self.diags.push(Diag::ty(
                    form.name.span,
                    format!(
                        "unknown form `{}`; v1 recognizes: vec, hashmap, \
                         ring_buffer, lru_cache",
                        other
                    ),
                ));
            }
        }
    }

    /// v1.x-FORM-5: `@form(ring_buffer, cap = N)` requires
    /// exactly one capacity slot of kind `pool`, holding any
    /// cell type T. The `cap` annotation arg is required and
    /// must be a positive integer literal — the backing buffer
    /// is pre-allocated at locus birth and never grows.
    fn check_form_ring_buffer_shape(
        &mut self,
        decl: &'a LocusDecl,
        form: &'a FormAnnotation,
    ) {
        // Validate args: exactly one, named `cap`, positive int literal.
        let mut cap_arg: Option<&FormArg> = None;
        for arg in &form.args {
            if arg.name.name == "cap" {
                if cap_arg.is_some() {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        "@form(ring_buffer): duplicate `cap` arg".to_string(),
                    ));
                } else {
                    cap_arg = Some(arg);
                }
            } else {
                self.diags.push(Diag::ty(
                    arg.name.span,
                    format!(
                        "@form(ring_buffer): unknown arg `{}`; v1 accepts \
                         `cap = N` only",
                        arg.name.name
                    ),
                ));
            }
        }
        match cap_arg {
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(ring_buffer) requires a `cap = N` arg (fixed \
                     capacity; the buffer is pre-allocated at locus birth)"
                        .to_string(),
                ));
            }
            Some(arg) => match &arg.value {
                Expr::Literal(Literal::Int(n), _) if *n > 0 => {
                    // OK.
                }
                _ => {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        "@form(ring_buffer) `cap` must be a positive \
                         integer literal (v1 doesn't const-evaluate \
                         expressions for form args)"
                            .to_string(),
                    ));
                }
            },
        }

        let capacity = decl.members.iter().find_map(|m| match m {
            LocusMember::Capacity(cb) => Some(cb),
            _ => None,
        });
        let cb = match capacity {
            Some(cb) => cb,
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(ring_buffer) requires exactly one `pool` capacity \
                     slot; found no `capacity { ... }` block on this locus"
                        .to_string(),
                ));
                return;
            }
        };
        match cb.slots.len() {
            0 => {
                self.diags.push(Diag::ty(
                    cb.span,
                    "@form(ring_buffer) requires exactly one `pool` capacity \
                     slot; found an empty capacity block"
                        .to_string(),
                ));
            }
            1 => {
                let slot = &cb.slots[0];
                match slot.kind {
                    CapacitySlotKind::Pool => {
                        // OK.
                    }
                    CapacitySlotKind::Heap => {
                        self.diags.push(Diag::ty(
                            slot.span,
                            format!(
                                "@form(ring_buffer) requires a `pool` slot; \
                                 got `heap {} of ...`. Ring buffer recycles \
                                 fixed-capacity cells (pool discipline); \
                                 heap is the growable shape covered by \
                                 @form(vec).",
                                slot.name.name
                            ),
                        ));
                    }
                }
                if slot.as_parent_for.is_some() {
                    self.diags.push(Diag::ty(
                        slot.span,
                        "@form(ring_buffer) slot cannot also be an \
                         `as_parent_for` override; form-lowered slots own \
                         their own allocator"
                            .to_string(),
                    ));
                }
                if slot.indexed_by.is_some() {
                    self.diags.push(Diag::ty(
                        slot.span,
                        "@form(ring_buffer) slot does not take an `indexed_by` \
                         clause (that clause belongs to @form(hashmap))"
                            .to_string(),
                    ));
                }
            }
            n => {
                self.diags.push(Diag::ty(
                    cb.span,
                    format!(
                        "@form(ring_buffer) requires exactly one `pool` \
                         capacity slot; found {}",
                        n
                    ),
                ));
            }
        }
    }

    /// v1.x-FORM-6: `@form(lru_cache, cap = N)` is the one form
    /// that needs BOTH a key and a cap. It borrows the
    /// `@form(hashmap)` key surface — exactly one `pool` capacity
    /// slot with an `indexed_by <fieldname>` clause over a
    /// user-declared struct cell — AND the `@form(ring_buffer)`
    /// required `cap = N` positive-int-literal arg (the cache is
    /// pre-allocated at locus birth and evicts LRU on over-cap
    /// insert; it never grows).
    fn check_form_lru_cache_shape(
        &mut self,
        decl: &'a LocusDecl,
        form: &'a FormAnnotation,
    ) {
        // Args: exactly one `cap = N`, positive int literal.
        let mut cap_arg: Option<&FormArg> = None;
        for arg in &form.args {
            if arg.name.name == "cap" {
                if cap_arg.is_some() {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        "@form(lru_cache): duplicate `cap` arg".to_string(),
                    ));
                } else {
                    cap_arg = Some(arg);
                }
            } else {
                self.diags.push(Diag::ty(
                    arg.name.span,
                    format!(
                        "@form(lru_cache): unknown arg `{}`; v1 accepts \
                         `cap = N` only",
                        arg.name.name
                    ),
                ));
            }
        }
        match cap_arg {
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(lru_cache) requires a `cap = N` arg (fixed \
                     capacity; the cache is pre-allocated at locus birth \
                     and evicts the least-recently-used entry on over-cap \
                     insert — it never grows)"
                        .to_string(),
                ));
            }
            Some(arg) => match &arg.value {
                Expr::Literal(Literal::Int(n), _) if *n > 0 => { /* OK */ }
                _ => {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        "@form(lru_cache) `cap` must be a positive integer \
                         literal (v1 doesn't const-evaluate expressions for \
                         form args)"
                            .to_string(),
                    ));
                }
            },
        }

        // Capacity block: exactly one `pool` slot with `indexed_by`,
        // cell = user struct, indexed-by field exists on it.
        let capacity = decl.members.iter().find_map(|m| match m {
            LocusMember::Capacity(cb) => Some(cb),
            _ => None,
        });
        let cb = match capacity {
            Some(cb) => cb,
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(lru_cache) requires exactly one `pool` capacity \
                     slot with `indexed_by <fieldname>`; found no \
                     `capacity { ... }` block on this locus"
                        .to_string(),
                ));
                return;
            }
        };
        if cb.slots.is_empty() {
            self.diags.push(Diag::ty(
                cb.span,
                "@form(lru_cache) requires exactly one `pool` capacity slot \
                 with `indexed_by <fieldname>`; found an empty capacity block"
                    .to_string(),
            ));
            return;
        }
        if cb.slots.len() > 1 {
            self.diags.push(Diag::ty(
                cb.span,
                format!(
                    "@form(lru_cache) requires exactly one capacity slot; \
                     found {} slots. lru_cache is a single keyed store.",
                    cb.slots.len()
                ),
            ));
            return;
        }
        let slot = &cb.slots[0];
        match slot.kind {
            CapacitySlotKind::Pool => {}
            CapacitySlotKind::Heap => {
                self.diags.push(Diag::ty(
                    slot.span,
                    format!(
                        "@form(lru_cache) requires a `pool` slot; got `heap \
                         {} of ...`. The cache recycles a fixed cell \
                         population as entries are inserted and evicted — \
                         that's the `pool` discipline.",
                        slot.name.name
                    ),
                ));
            }
        }
        let field_ident = match &slot.indexed_by {
            Some(i) => i,
            None => {
                self.diags.push(Diag::ty(
                    slot.span,
                    format!(
                        "@form(lru_cache) slot `{}` must declare `indexed_by \
                         <fieldname>` naming the field of the cell type that \
                         serves as the cache key",
                        slot.name.name
                    ),
                ));
                return;
            }
        };
        let cell_name = match &slot.elem_ty {
            TypeExpr::Named { path, .. } if path.segments.len() == 1 => {
                path.segments[0].name.clone()
            }
            _ => {
                self.diags.push(Diag::ty(
                    slot.elem_ty.span(),
                    "@form(lru_cache) cell type must be a user-declared \
                     struct (so the `indexed_by` field can resolve to a \
                     typed key); got a primitive, qualified path, or \
                     composite type"
                        .to_string(),
                ));
                return;
            }
        };
        let cell_name = self.through_type_alias(cell_name);
        match self.top.lookup(&cell_name) {
            Some(TopSymbol::Type(info)) => match &info.kind {
                TypeKind::Struct(fields) => {
                    if !fields.iter().any(|f| f.name == field_ident.name) {
                        self.diags.push(Diag::ty(
                            field_ident.span,
                            format!(
                                "@form(lru_cache) cell type `{}` has no field \
                                 `{}` — the `indexed_by` field must exist on \
                                 the cell struct",
                                cell_name, field_ident.name
                            ),
                        ));
                        return;
                    }
                }
                TypeKind::Enum(_) => {
                    self.diags.push(Diag::ty(
                        slot.elem_ty.span(),
                        format!(
                            "@form(lru_cache) cell type `{}` is an enum; cell \
                             must be a struct so `indexed_by` can resolve to a \
                             typed key field",
                            cell_name
                        ),
                    ));
                    return;
                }
                TypeKind::Alias(_) => {
                    self.diags.push(Diag::ty(
                        slot.elem_ty.span(),
                        format!(
                            "@form(lru_cache) cell type `{}` is a type alias; \
                             cell must be a struct so `indexed_by` can resolve",
                            cell_name
                        ),
                    ));
                    return;
                }
            },
            Some(TopSymbol::Locus(_)) => {
                self.diags.push(Diag::ty(
                    slot.elem_ty.span(),
                    format!(
                        "@form(lru_cache) cell type `{}` is a locus. Cells \
                         are data; loci are managed entities. Store data in \
                         the cache and route entity membership through \
                         `accept(c: ...)` instead.",
                        cell_name
                    ),
                ));
                return;
            }
            _ => {
                // Cell type unresolved — separate error already
                // raised by the type resolver. Skip so we don't
                // double-report.
                return;
            }
        }
        if slot.as_parent_for.is_some() {
            self.diags.push(Diag::ty(
                slot.span,
                "@form(lru_cache) slot cannot also be an `as_parent_for` \
                 override; form-lowered slots own their own allocator"
                    .to_string(),
            ));
        }
    }

    /// v1.x-FORM-4: `@form(hashmap)` requires exactly one
    /// capacity slot, of kind `pool`, with an `indexed_by
    /// <fieldname>` clause. The slot's cell type must be a
    /// user-declared struct; the indexed-by field must exist
    /// on that struct. The field's type becomes the hashmap
    /// key type K; the cell type becomes the value type S.
    fn check_form_hashmap_shape(&mut self, decl: &'a LocusDecl, form: &'a FormAnnotation) {
        // F.32-1α/β2/γ (2026-05-24 → 2026-05-25): @form(hashmap)
        // accepts optional kwargs:
        //   sync = X  (X ∈ {none, serialized, striped, lockfree})
        //   cap  = N  (positive int literal; REQUIRED when
        //              sync = lockfree, rejected otherwise)
        //
        // Plain `@form(hashmap)` keeps the single-pool default
        // (no sync overhead; cross-pool calls typecheck-rejected
        // per F.32-0).
        let mut sync_value: Option<&str> = None;
        let mut cap_arg: Option<&FormArg> = None;
        for arg in &form.args {
            match arg.name.name.as_str() {
                "sync" => {
                    let val = match &arg.value {
                        Expr::Ident(i) => i.name.as_str(),
                        _ => {
                            self.diags.push(Diag::ty(
                                arg.span,
                                "@form(hashmap, sync = X): X must be a \
                                 bare identifier (one of `serialized`, \
                                 `striped`, `lockfree`)".to_string(),
                            ));
                            continue;
                        }
                    };
                    match val {
                        "serialized" => { sync_value = Some("serialized"); }
                        "striped"    => { sync_value = Some("striped"); }
                        "lockfree"   => { sync_value = Some("lockfree"); }
                        "none"       => { /* same as omitting */ }
                        other => {
                            self.diags.push(Diag::ty(
                                arg.span,
                                format!(
                                    "@form(hashmap, sync = {}): unknown sync \
                                     discipline; v1 accepts `serialized` \
                                     (F.32-1α), `striped` (F.32-1β2), and \
                                     `lockfree` (F.32-1γ-v1).",
                                    other
                                ),
                            ));
                        }
                    }
                }
                "cap" => {
                    cap_arg = Some(arg);
                }
                other => {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        format!(
                            "@form(hashmap): unknown arg `{}`; v1 accepts \
                             `sync = X` and (when sync = lockfree) `cap = N`",
                            other
                        ),
                    ));
                }
            }
        }
        // F.32-1γ-v1/v2: lockfree accepts `cap = N` as an
        // initial-size hint. Pre-v2 (no grow path) the cap was
        // required because the table couldn't grow; v2 ships
        // grow (2026-05-26) so cap is now optional — omitting it
        // starts the table at LOTUS_HASHMAP_INITIAL_CAP and
        // grows on demand. Other sync modes still reject cap
        // (they have their own initial size + grow policy).
        match (sync_value, cap_arg) {
            (Some("lockfree"), Some(arg)) => {
                match &arg.value {
                    Expr::Literal(Literal::Int(n), _) if *n > 0 => {
                        /* OK */
                    }
                    _ => {
                        self.diags.push(Diag::ty(
                            arg.span,
                            "@form(hashmap, sync = lockfree) `cap` must be a \
                             positive integer literal (v1 doesn't const-evaluate \
                             expressions for form args)".to_string(),
                        ));
                    }
                }
            }
            (_, Some(arg)) => {
                self.diags.push(Diag::ty(
                    arg.name.span,
                    "@form(hashmap): `cap = N` is only valid with \
                     `sync = lockfree`. Other sync modes (none, serialized, \
                     striped) grow dynamically; their initial cap is \
                     LOTUS_HASHMAP_INITIAL_CAP (8) and managed by the \
                     runtime."
                        .to_string(),
                ));
            }
            _ => { /* nothing else to check */ }
        }
        let capacity = decl.members.iter().find_map(|m| match m {
            LocusMember::Capacity(cb) => Some(cb),
            _ => None,
        });
        let cb = match capacity {
            Some(cb) => cb,
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(hashmap) requires exactly one `pool` capacity slot \
                     with `indexed_by <fieldname>`; found no `capacity { ... }` \
                     block on this locus"
                        .to_string(),
                ));
                return;
            }
        };
        if cb.slots.is_empty() {
            self.diags.push(Diag::ty(
                cb.span,
                "@form(hashmap) requires exactly one `pool` capacity slot \
                 with `indexed_by <fieldname>`; found an empty capacity block"
                    .to_string(),
            ));
            return;
        }
        if cb.slots.len() > 1 {
            self.diags.push(Diag::ty(
                cb.span,
                format!(
                    "@form(hashmap) requires exactly one capacity slot; \
                     found {} slots. Hashmap is a single keyed store.",
                    cb.slots.len()
                ),
            ));
            return;
        }
        let slot = &cb.slots[0];
        // Slot kind must be Pool (cells recycle as entries come
        // and go); Heap doesn't model the "bounded recyclable
        // population" the hashmap needs.
        match slot.kind {
            CapacitySlotKind::Pool => {}
            CapacitySlotKind::Heap => {
                self.diags.push(Diag::ty(
                    slot.span,
                    format!(
                        "@form(hashmap) requires a `pool` slot; got `heap {} \
                         of ...`. Hashmap recycles cells as entries are \
                         inserted and removed — that's the `pool` discipline. \
                         `heap` is the unordered growable shape (use @form(vec)).",
                        slot.name.name
                    ),
                ));
            }
        }
        // Slot must declare `indexed_by <fieldname>`.
        let field_ident = match &slot.indexed_by {
            Some(i) => i,
            None => {
                self.diags.push(Diag::ty(
                    slot.span,
                    format!(
                        "@form(hashmap) slot `{}` must declare `indexed_by \
                         <fieldname>` naming the field of the cell type that \
                         serves as the hashmap key",
                        slot.name.name
                    ),
                ));
                return;
            }
        };
        // The cell type must be a user-declared struct so we can
        // verify the indexed-by field exists. Primitives, enums,
        // and locus refs are rejected.
        let cell_name = match &slot.elem_ty {
            TypeExpr::Named { path, .. } if path.segments.len() == 1 => {
                path.segments[0].name.clone()
            }
            _ => {
                self.diags.push(Diag::ty(
                    slot.elem_ty.span(),
                    "@form(hashmap) cell type must be a user-declared struct \
                     (so the `indexed_by` field can resolve to a typed key); \
                     got a primitive, qualified path, or composite type"
                        .to_string(),
                ));
                return;
            }
        };
        let cell_name = self.through_type_alias(cell_name);
        let field_ty = match self.top.lookup(&cell_name) {
            Some(TopSymbol::Type(info)) => match &info.kind {
                TypeKind::Struct(fields) => {
                    match fields.iter().find(|f| f.name == field_ident.name) {
                        Some(f) => f.ty.clone(),
                        None => {
                            self.diags.push(Diag::ty(
                                field_ident.span,
                                format!(
                                    "@form(hashmap) cell type `{}` has no field \
                                     `{}` — the `indexed_by` field must exist on \
                                     the cell struct",
                                    cell_name, field_ident.name
                                ),
                            ));
                            return;
                        }
                    }
                }
                TypeKind::Enum(_) => {
                    self.diags.push(Diag::ty(
                        slot.elem_ty.span(),
                        format!(
                            "@form(hashmap) cell type `{}` is an enum; cell \
                             must be a struct so `indexed_by` can resolve to a \
                             typed key field",
                            cell_name
                        ),
                    ));
                    return;
                }
                TypeKind::Alias(_) => {
                    self.diags.push(Diag::ty(
                        slot.elem_ty.span(),
                        format!(
                            "@form(hashmap) cell type `{}` is a type alias; \
                             cell must be a struct so `indexed_by` can resolve",
                            cell_name
                        ),
                    ));
                    return;
                }
            },
            Some(TopSymbol::Locus(_)) => {
                self.diags.push(Diag::ty(
                    slot.elem_ty.span(),
                    format!(
                        "@form(hashmap) cell type `{}` is a locus. Cells \
                         are data; loci are managed entities. Storing an \
                         entity in a hashmap means the synthesized `.get \
                         (key)` returns a stranger to the caller, which \
                         violates the rule in `spec/semantics.md § Locus \
                         method dispatch` (same shape as a method returning \
                         a locus).\n\n\
                         Canonical alternatives for keyed-children patterns:\n\
                         1. Parent-child: declare `accept(c: {})` on the \
                            parent. Pair with a `@form(hashmap)` of cell \
                            type `type Index {{ key: String; child_idx: \
                            Int; }}` if name-based lookup is needed.\n\
                         2. Bus topic: publish commands keyed by name; \
                            subscriber dispatches into the right child.\n\
                         3. Delegation: collapse the per-child operation \
                            onto the parent (`parent.inc_named(name)`).\n\n\
                         See spec/forms.md § @form(hashmap) cell type and \
                         spec/semantics.md § Locus method dispatch.",
                        cell_name, cell_name
                    ),
                ));
                return;
            }
            _ => {
                // Cell type unresolved — separate error already
                // raised by the type resolver. Skip further checks
                // so we don't double-report.
                return;
            }
        };
        // as_parent_for and form-lowered slots don't compose:
        // the form owns the slot's allocator.
        if slot.as_parent_for.is_some() {
            self.diags.push(Diag::ty(
                slot.span,
                "@form(hashmap) slot cannot also be an `as_parent_for` \
                 override; form-lowered slots own their own allocator"
                    .to_string(),
            ));
        }
        // PR3 reads the key type `field_ty` to synthesize methods;
        // for now we just verified it resolves.
        let _ = field_ty;
    }

    /// v1.x-FORM-1: `@form(vec)` requires exactly one capacity
    /// slot, of kind `heap`, holding any cell type T. The slot
    /// name is user-chosen and not part of the contract.
    fn check_form_vec_shape(&mut self, decl: &'a LocusDecl, form: &'a FormAnnotation) {
        if !form.args.is_empty() {
            self.diags.push(Diag::ty(
                form.span,
                format!(
                    "@form(vec) takes no arguments; got {} (vec has no \
                     tuning knobs in v1 — drop the arg list)",
                    form.args.len()
                ),
            ));
        }
        let capacity = decl.members.iter().find_map(|m| match m {
            LocusMember::Capacity(cb) => Some(cb),
            _ => None,
        });
        let cb = match capacity {
            Some(cb) => cb,
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(vec) requires exactly one `heap` capacity slot; \
                     found no `capacity { ... }` block on this locus"
                        .to_string(),
                ));
                return;
            }
        };
        match cb.slots.len() {
            0 => {
                self.diags.push(Diag::ty(
                    cb.span,
                    "@form(vec) requires exactly one `heap` capacity slot; \
                     found an empty capacity block"
                        .to_string(),
                ));
                return;
            }
            1 => {
                let slot = &cb.slots[0];
                match slot.kind {
                    CapacitySlotKind::Heap => {
                        // OK: the contract is satisfied.
                        // Cell type T is whatever's declared;
                        // PR3b synthesizes methods over it.
                    }
                    CapacitySlotKind::Pool => {
                        self.diags.push(Diag::ty(
                            slot.span,
                            format!(
                                "@form(vec) requires a `heap` slot; got `pool {} \
                                 of ...`. Vec is the contiguous-growable shape; \
                                 `pool` is the unordered free-list shape — they're \
                                 different storage disciplines.",
                                slot.name.name
                            ),
                        ));
                    }
                }
                if slot.as_parent_for.is_some() {
                    self.diags.push(Diag::ty(
                        slot.span,
                        "@form(vec) slot cannot also be an `as_parent_for` \
                         override; form-lowered slots own their own allocator"
                            .to_string(),
                    ));
                }
            }
            n => {
                self.diags.push(Diag::ty(
                    cb.span,
                    format!(
                        "@form(vec) requires exactly one `heap` capacity slot; \
                         found {} slots. Vec is a single contiguous buffer.",
                        n
                    ),
                ));
            }
        }
    }

    /// M3 stage 4 (2026-07-02): every `expose` entry must bind
    /// against something real on the declaring locus — a params
    /// field, a mode (bulk/harmonic/resolution, per the
    /// mode-pull rule in semantics.md), or a `fn` member — with a
    /// type matching what's declared there (v0 equality via
    /// assignable_from, which stays permissive on Unknown).
    /// Without this, `expose value: String;` over an Int field
    /// compiles, and a parent consuming `value: String` checks
    /// clean against fiction.
    fn check_contract_expose_validity(&mut self, locus: &LocusInfo) {
        // GH #436: `@sealed` and `expose` are contradictory claims
        // about the same boundary, and sealing wins — so an `expose`
        // here reads as a permission it cannot grant. The contract
        // consistency check passes (a matching `consume` binds fine),
        // and then every use of the exposed field is rejected.
        //
        // `expose` cannot become the sealed allowlist without a
        // redesign: it is the coordinator/coordinatee surface, so
        // honouring it would grant reads to an `accept`ing parent and
        // still deny them to a parent holding the same child as a
        // param — one field, public to one kind of holder and not the
        // other. Rejecting the pair is the honest reading until
        // `expose` means "public param" independent of `accept`.
        if locus.sealed {
            if let Some(entry) = locus.contract_expose.first() {
                self.diags.push(Diag::ty(
                    entry.span,
                    format!(
                        "locus `{}` is `@sealed`, so `expose {}` cannot \
                         grant anything: sealing denies every read from \
                         outside the locus, including one a coordinator \
                         `consume`s. Drop the `@sealed`, or drop the \
                         contract and let callers use its methods.",
                        locus.name, entry.name
                    ),
                ));
                return;
            }
        }
        for entry in &locus.contract_expose {
            // 1. params field
            if let Some(p) =
                locus.params.iter().find(|p| p.name == entry.name)
            {
                if !entry.ty.assignable_from(&p.ty) {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "contract: locus `{}` exposes `{}: {}`, but the \
                             field is declared `{}`",
                            locus.name,
                            entry.name,
                            entry.ty.display(),
                            p.ty.display()
                        ),
                    ));
                }
                continue;
            }
            // 2. mode (exposed-mode pull: `expose bulk: T;`)
            let mode = match entry.name.as_str() {
                "bulk" => Some(ModeKind::Bulk),
                "harmonic" => Some(ModeKind::Harmonic),
                "resolution" => Some(ModeKind::Resolution),
                _ => None,
            };
            if let Some(mk) = mode {
                match locus.mode_returns.get(&mk) {
                    Some(ret) => {
                        if !entry.ty.assignable_from(ret) {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "contract: locus `{}` exposes `{}: {}`, \
                                     but the mode returns `{}`",
                                    locus.name,
                                    entry.name,
                                    entry.ty.display(),
                                    ret.display()
                                ),
                            ));
                        }
                    }
                    None => {
                        self.diags.push(Diag::ty(
                            entry.span,
                            format!(
                                "contract: locus `{}` exposes mode `{}` but \
                                 does not declare it",
                                locus.name, entry.name
                            ),
                        ));
                    }
                }
                continue;
            }
            // 3. fn member (vertical method surface)
            if let Some(m) =
                locus.methods.iter().find(|m| m.name == entry.name)
            {
                if !entry.ty.assignable_from(&m.ret) {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "contract: locus `{}` exposes `{}: {}`, but the \
                             method returns `{}`",
                            locus.name,
                            entry.name,
                            entry.ty.display(),
                            m.ret.display()
                        ),
                    ));
                }
                continue;
            }
            self.diags.push(Diag::ty(
                entry.span,
                format!(
                    "contract: locus `{}` exposes `{}` but has no field, \
                     mode, or method with that name",
                    locus.name, entry.name
                ),
            ));
        }
    }

    fn check_contract_compatibility(&mut self, parent: &LocusInfo) {
        let child_name = match &parent.accept_param {
            Some((_, Ty::Named(n))) => n.clone(),
            Some((_, _)) => return, // non-named child type → can't statically resolve
            None => {
                // Parent declares consume but doesn't accept any
                // child. Static error per F.8 — the consume
                // surface has nothing to bind against.
                for entry in &parent.contract_consume {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "locus `{}`: contract consumes `{}` but declares no \
                             `accept(_: ChildType)` to bind against",
                            parent.name, entry.name
                        ),
                    ));
                }
                return;
            }
        };
        let child = match self.top.lookup(&child_name) {
            Some(TopSymbol::Locus(c)) => c,
            _ => return, // unresolved child type — separate error already raised
        };
        for need in &parent.contract_consume {
            match child
                .contract_expose
                .iter()
                .find(|e| e.name == need.name)
            {
                Some(have) => {
                    if !need.ty.assignable_from(&have.ty) {
                        self.diags.push(Diag::ty(
                            need.span,
                            format!(
                                "contract: locus `{}` consumes `{}: {}`, but child \
                                 locus `{}` exposes it as `{}`",
                                parent.name,
                                need.name,
                                need.ty.display(),
                                child.name,
                                have.ty.display()
                            ),
                        ));
                    }
                }
                None => {
                    self.diags.push(Diag::ty(
                        need.span,
                        format!(
                            "contract: locus `{}` consumes `{}` but child locus \
                             `{}` does not expose it",
                            parent.name, need.name, child.name
                        ),
                    ));
                }
            }
        }
    }

    fn check_locus_member(&mut self, member: &'a LocusMember) {
        match member {
            LocusMember::Params(pb) => {
                // Param defaults ARE typechecked. The Milestone-2
                // cut here claimed they were "checked implicitly
                // when the param is referenced" — they were not, so
                // `params { n: Int = "nope"; }` and a default naming
                // something that does not exist both passed `hale
                // check` and failed in codegen with a message the
                // checker was better placed to give.
                //
                // Scope is the same one codegen resolves against:
                // top-level consts and `self.<param>`. A bare
                // sibling name is NOT in scope — with a `const n`
                // and a param `n`, a default written `n` takes the
                // const — so an unresolved bare name is a genuine
                // unknown identifier, and `check_expr` reports it.
                // GH #877: every param's declared type, whether or
                // not it carries a default — an undeclared name here
                // typed the field `Unknown`, which accepts every
                // store and every read, and the locus died at
                // lowering with no span.
                for p in &pb.params {
                    if let Some(te) = &p.ty {
                        self.check_type_annotation(te);
                    }
                }
                for p in &pb.params {
                    let ParamInit::Value(init) = &p.init else {
                        continue;
                    };
                    // A bare name resolves to top-level const
                    // scope only. Unresolved means unknown — and
                    // the most likely intent is a sibling param, so
                    // say the spelling that works.
                    if let Expr::Ident(id) = init {
                        if self.top.lookup(&id.name).is_none() {
                            let sibling = pb
                                .params
                                .iter()
                                .any(|q| q.name.name == id.name);
                            let hint = if sibling {
                                format!(
                                    " — `{}` is another param of this locus; a default reaches one through `self.{}`",
                                    id.name, id.name
                                )
                            } else {
                                String::new()
                            };
                            self.diags.push(Diag::ty(
                                id.span,
                                format!(
                                    "param `{}`: unknown identifier `{}` in its default{}",
                                    p.name.name, id.name, hint
                                ),
                            ));
                            continue;
                        }
                    }
                    let got = self.check_expr(init);
                    let Some(te) = &p.ty else { continue };
                    let want = resolve_type_expr(te, self.known);
                    // `Unknown` on either side is an unresolved
                    // type, already diagnosed (or deliberately
                    // opaque, as multi-segment stdlib handles are).
                    // Re-reporting it here would be noise on top of
                    // a real error.
                    if matches!(want, Ty::Unknown)
                        || matches!(got, Ty::Unknown)
                    {
                        continue;
                    }
                    // Int literals widen into Float params, the same
                    // rule field initialization uses.
                    // Exactly the coercions CODEGEN accepts on a
                    // param default (`locus/decl.rs`, the
                    // `literal_to_view` rule): a String or Bytes
                    // literal into a view param.
                    //
                    // NOT Int -> Float. Field initialization widens
                    // there, and it would be reasonable here, but
                    // codegen refuses it — so accepting it in the
                    // checker would trade one divergence for
                    // another. Widening param defaults is a
                    // deliberate change to make on BOTH sides, not a
                    // leniency to add on one.
                    let widen_ok = matches!(
                        (&want, &got),
                        (
                            Ty::Prim(PrimType::StringView),
                            Ty::Prim(PrimType::String)
                        ) | (
                            Ty::Prim(PrimType::BytesView),
                            Ty::Prim(PrimType::Bytes)
                        )
                    );
                    // A param typed as a PERSPECTIVE (or an
                    // interface) is legitimately initialized with a
                    // locus that serves/conforms to it — the
                    // perspective fixtures do exactly this. Plain
                    // assignability does not know about conformance,
                    // so ask separately before reporting.
                    let conforms = match (&want, &got) {
                        (Ty::Named(w), Ty::Named(g)) => {
                            let serves_or_iface =
                                match self.top.symbols.get(g) {
                                    Some(TopSymbol::Locus(li)) => li
                                        .serves
                                        .iter()
                                        .any(|sv| sv == w)
                                        || matches!(
                                            self.top.symbols.get(w),
                                            Some(TopSymbol::Interface(_))
                                        ),
                                    _ => false,
                                };
                            // `b: Box<Int> = Box { value: 0 }` — the
                            // annotation resolves to the mangled
                            // monomorph `Box_Int` while the literal
                            // types as the template `Box`. The
                            // monomorph's own field validation
                            // already checked the arguments; this is
                            // the same value under two spellings.
                            //
                            // GH #911 B5: generic TYPES only.
                            // `c: Cache<Int, String> = Cache { }` on
                            // a generic LOCUS is refused at build
                            // ("generic instantiation
                            // `Cache_Int_String` not synthesized —
                            // discovery missed the use site"), so
                            // accepting it here would trade one
                            // divergence for another.
                            let monomorph_of_template = self
                                .resolve_generic_monomorph(w)
                                .is_some_and(|(t, _)| {
                                    t.is_type() && t.name() == g.as_str()
                                });
                            serves_or_iface || monomorph_of_template
                        }
                        _ => false,
                    };
                    if !widen_ok && !conforms && !want.assignable_from(&got) {
                        self.diags.push(Diag::ty(
                            init.span(),
                            format!(
                                "param `{}`: declared `{}`, default is `{}`",
                                p.name.name,
                                want.display(),
                                got.display()
                            ),
                        ));
                    }
                }
            }
            LocusMember::Bus(bb) => {
                // A subscribe on a LITERAL or wildcard subject must
                // say `of type T`: there is no declaration to take
                // the payload from, and codegen needs one to pick a
                // deserializer. Without this the omission passed
                // `hale check` and failed the build with no span —
                // the divergence class `corpus_check_build_agreement`
                // gates. A declared-topic subscribe is exempt: the
                // topic carries the payload.
                for bm in &bb.members {
                    let BusMember::Subscribe { subject, ty, span, .. } = bm
                    else {
                        continue;
                    };
                    if ty.is_some() {
                        continue;
                    }
                    if !matches!(subject, BusSubject::Literal { .. }) {
                        continue;
                    }
                    self.diags.push(Diag::ty(
                        *span,
                        format!(
                            "subscribe `{}`: a literal subject carries no payload declaration, so the subscription must name one — `subscribe \"{}\" as <handler> of type <T>;`",
                            subject.canonical(),
                            subject.canonical()
                        ),
                    ));
                }
            }
            LocusMember::Contract(_) => {
                // Already lowered by the resolver.
            }
            LocusMember::Bindings(_) => {
                // Bindings are checked by a separate top-level pass
                // (validate_bindings); nothing to do here.
            }
            LocusMember::Placement(_) => {
                // F.31: placement entries are validated by a
                // dedicated top-level pass (Phase 2 — pending).
                // The parser already enforces "main-only" so the
                // block's syntactic shape is OK here.
            }
            LocusMember::Topology(_) => {
                // Topology Phase 1b: the `topology { }` block is
                // validated by the dedicated placement pass
                // (check_placement_block), alongside the
                // `pinned(node =)` / `pinned(l3 =)` references
                // that resolve against it. Nothing member-local.
            }
            LocusMember::Claims(_) => {
                // GH #382: claims are evaluated by the dedicated
                // bundle-level law judgment — they
                // quantify over the merged bundle, not this locus.
                // The parser already enforces "main-only".
            }
            LocusMember::Lifecycle(lc) => {
                self.in_lifecycle = true;
                self.locals.push();
                for p in &lc.params {
                    // GH #877: `accept(c: Chld)` names the child
                    // locus; an undeclared name is the same typo in
                    // the same position.
                    self.check_type_annotation(&p.ty);
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&lc.body);
                self.locals.pop();
                self.in_lifecycle = false;
            }
            LocusMember::Mode(md) => {
                self.in_lifecycle = true;
                self.locals.push();
                for p in &md.params {
                    self.check_type_annotation(&p.ty);
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&md.body);
                self.locals.pop();
                self.in_lifecycle = false;
            }
            LocusMember::Failure(fd) => {
                // The handler's SIGNATURE, checked here rather than
                // only in codegen. `on_failure` is the supervision
                // surface a reader meets early, and getting its shape
                // wrong used to pass `hale check` and fail the build
                // with no source location — the divergence class
                // `corpus_check_build_agreement` gates.
                //
                // Same two rules codegen enforces (locus/decl.rs):
                // exactly (child, err), and the error is
                // `ClosureViolation`.
                let locus_name = self
                    .current_locus
                    .map(|l| l.name.clone())
                    .unwrap_or_default();
                if fd.params.len() != 2 {
                    self.diags.push(Diag::ty(
                        fd.span,
                        format!(
                            "locus `{}`: `on_failure` takes exactly two params — the failing child and the error — got {}",
                            locus_name,
                            fd.params.len()
                        ),
                    ));
                } else {
                    let err_ty =
                        resolve_type_expr(&fd.params[1].ty, self.known);
                    let is_violation = matches!(
                        &err_ty,
                        Ty::Named(n) if n == "ClosureViolation"
                    );
                    if !is_violation && !matches!(err_ty, Ty::Unknown) {
                        self.diags.push(Diag::ty(
                            fd.params[1].name.span,
                            format!(
                                "locus `{}`: `on_failure`'s second param is the error and must be `ClosureViolation`, got `{}`",
                                locus_name,
                                err_ty.display()
                            ),
                        ));
                    }
                }
                self.in_lifecycle = true;
                self.in_on_failure = true;
                self.locals.push();
                for p in &fd.params {
                    self.check_type_annotation(&p.ty);
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&fd.body);
                self.locals.pop();
                self.in_on_failure = false;
                self.in_lifecycle = false;
            }
            LocusMember::Closure(cd) => {
                self.in_closure = true;
                self.in_lifecycle = true;
                // v1.x-VIOLATE (F.27): structural rules on the
                // closure declaration itself.
                let is_inline = cd.clauses.iter().any(|c| {
                    matches!(c, ClosureClause::Epoch(EpochSpec::Inline))
                });
                let captures: Vec<&Ident> = cd
                    .clauses
                    .iter()
                    .flat_map(|c| match c {
                        ClosureClause::Captures(names) => names.iter().collect(),
                        _ => Vec::new(),
                    })
                    .collect();
                // 1. Assertion-presence must match epoch shape.
                //    - `epoch inline`: assertion MUST be absent
                //      (inline fires only via `violate`; an
                //      assertion that never fires is dead).
                //    - Any other epoch: assertion MUST be present.
                if is_inline && cd.assertion.is_some() {
                    self.diags.push(Diag::ty(
                        cd.span,
                        format!(
                            "closure `{}`: `epoch inline` closures must \
                             omit the assertion (inline closures fire \
                             only via `violate`; the assertion has no \
                             evaluation site)",
                            cd.name.name,
                        ),
                    ));
                }
                if !is_inline && cd.assertion.is_none() {
                    self.diags.push(Diag::ty(
                        cd.span,
                        format!(
                            "closure `{}`: missing assertion. Assertion-\
                             less closures require an `epoch inline` \
                             clause (per F.27); otherwise declare the \
                             `LEFT ~~ RIGHT within TOL;` band",
                            cd.name.name,
                        ),
                    ));
                }
                // 2. `captures:` is only meaningful on inline
                //    closures.
                if !captures.is_empty() && !is_inline {
                    self.diags.push(Diag::ty(
                        cd.span,
                        format!(
                            "closure `{}`: `captures:` is meaningful only \
                             on `epoch inline` closures (the snapshot \
                             happens at `violate` fire time, which \
                             auto-epoch closures don't reach)",
                            cd.name.name,
                        ),
                    ));
                }
                // 3. Each captured field name must exist on the
                //    locus param/state surface.
                if let Some(locus) = self.current_locus {
                    for f in &captures {
                        if !locus.params.iter().any(|p| p.name == f.name) {
                            self.diags.push(Diag::ty(
                                f.span,
                                format!(
                                    "closure `{}`: `captures:` references \
                                     field `{}`, which is not declared on \
                                     locus `{}`",
                                    cd.name.name, f.name, locus.name,
                                ),
                            ));
                        }
                    }
                }
                // F.34 (v1.x-WINDOWED): `resets_per_epoch(...)` is
                // only meaningful on `epoch duration(N)` closures
                // (the field-zeroing hook fires at duration-boundary,
                // and the other epochs either don't recur — birth /
                // dissolve / inline — or recur too fast to be a
                // useful rate-budget window — tick).
                let is_duration = cd.clauses.iter().any(|c| {
                    matches!(
                        c,
                        ClosureClause::Epoch(EpochSpec::Duration(_))
                    )
                });
                let resets_pe_fields: Vec<&Ident> = cd
                    .clauses
                    .iter()
                    .flat_map(|c| match c {
                        ClosureClause::ResetsPerEpoch(names) => {
                            names.iter().collect()
                        }
                        _ => Vec::new(),
                    })
                    .collect();
                if !resets_pe_fields.is_empty() && !is_duration {
                    self.diags.push(Diag::ty(
                        cd.span,
                        format!(
                            "closure `{}`: `resets_per_epoch(...)` is \
                             meaningful only on `epoch duration(N)` \
                             closures. Other epochs either don't recur \
                             (birth / dissolve / inline) or recur too \
                             fast to be a useful rate-budget window \
                             (tick).",
                            cd.name.name,
                        ),
                    ));
                }
                if let Some(locus) = self.current_locus {
                    for f in &resets_pe_fields {
                        let Some(p) = locus
                            .params
                            .iter()
                            .find(|p| p.name == f.name)
                        else {
                            self.diags.push(Diag::ty(
                                f.span,
                                format!(
                                    "closure `{}`: `resets_per_epoch(...)` \
                                     references field `{}`, which is not \
                                     declared on locus `{}`",
                                    cd.name.name, f.name, locus.name,
                                ),
                            ));
                            continue;
                        };
                        let is_numeric = matches!(
                            &p.ty,
                            Ty::Prim(PrimType::Int)
                                | Ty::Prim(PrimType::Uint)
                                | Ty::Prim(PrimType::Float)
                                | Ty::Prim(PrimType::Decimal)
                        );
                        if !is_numeric {
                            self.diags.push(Diag::ty(
                                f.span,
                                format!(
                                    "closure `{}`: `resets_per_epoch(...)` \
                                     field `{}` has non-numeric type `{}`. \
                                     The reset hook zeros the field, which \
                                     only makes sense for Int / Uint / \
                                     Float / Decimal counters.",
                                    cd.name.name,
                                    f.name,
                                    p.ty.display(),
                                ),
                            ));
                        }
                    }
                }
                // Original assertion checks for assertion-bearing
                // closures.
                if let Some(assertion) = &cd.assertion {
                    let lt = self.check_expr(&assertion.left);
                    let rt = self.check_expr(&assertion.right);
                    if !lt.assignable_from(&rt) && !rt.assignable_from(&lt) {
                        self.diags.push(Diag::ty(
                            assertion.span,
                            format!(
                                "closure `{}`: assertion sides have incompatible types \
                                 `{}` and `{}`",
                                cd.name.name,
                                lt.display(),
                                rt.display()
                            ),
                        ));
                    }
                    if is_pure_literal(&assertion.left)
                        && is_pure_literal(&assertion.right)
                    {
                        self.diags.push(Diag::ty(
                            assertion.span,
                            format!(
                                "closure `{}`: both assertion sides are pure literals; \
                                 a closure must observe at least one runtime-varying \
                                 value (e.g. `self.x`) to audit anything",
                                cd.name.name
                            ),
                        ));
                    }
                    let _ = self.check_expr(&assertion.tolerance);
                }
                self.in_lifecycle = false;
                self.in_closure = false;
            }
            LocusMember::Fn(f) => {
                // Open-question #24 MVP (2026-05-25): user-
                // declared locus member fns may now carry
                // `fallible(E)` (the value-level error channel).
                // Substrate-facing surfaces still can't —
                // lifecycle methods (Lifecycle decls), mode
                // methods (Mode decls), and bus-subscribed
                // handlers stay non-fallible because the
                // substrate orchestrates them and has no caller
                // frame to address a value return. Lifecycle /
                // Mode are physically incapable (their AST
                // structs don't carry a `fallible` field); the
                // bus-subscribed check lives at the subscribe-
                // site loop above (search for "fallible-handler
                // check").
                //
                // Closure assertions can't *call* fallible
                // member fns inside the assertion expression —
                // `or <disposition>` is statement-position and
                // doesn't compose inside expression-shaped
                // assertion bodies; factor the value-error path
                // into a separate fn and have the closure assert
                // over pre-computed locus state instead. Not
                // checked here for v0.1 — the assertion grammar
                // already rejects most call shapes.
                self.in_lifecycle = true;
                self.check_fn(f, self.current_locus);
                self.in_lifecycle = false;
            }
            LocusMember::Const(_) => {
                // GH #747: refused at its declaration by
                // `check_no_member_consts` before any member is
                // walked. Typechecking the value here too would
                // print a second message ("const `x`: expected
                // Int, got String") about a declaration that has
                // to move either way — one mistake, one diagnostic.
            }
            LocusMember::Type(_) => {
                // GH #756: refused at its declaration by
                // `check_no_member_types` before any member is
                // walked. Its fields were never checked here either
                // — the resolver registers no member type, so there
                // is nothing to check them against.
            }
            LocusMember::Capacity(cb) => {
                // F.22 restriction 1: cell type must be a value-shape,
                // not a LocusRef. Loci have lifecycle; recycling
                // (Pool.release) or individual free (Heap.free) would
                // orphan the locus. The spec routes locus-membership
                // through `accept(c: SomeL)`; slots are for types.
                let mut seen: BTreeMap<String, Span> = BTreeMap::new();
                for slot in &cb.slots {
                    if let Some(prev) = seen.insert(
                        slot.name.name.clone(),
                        slot.name.span,
                    ) {
                        self.diags.push(
                            Diag::ty(
                                slot.name.span,
                                format!(
                                    "duplicate capacity slot name `{}`",
                                    slot.name.name
                                ),
                            )
                            .with_related(prev, "first declared here"),
                        );
                    }
                    // GH #877: the cell type is an annotation too.
                    self.check_type_annotation(&slot.elem_ty);
                    let elem_ty = resolve_type_expr(&slot.elem_ty, self.known);
                    let kind_word = match slot.kind {
                        CapacitySlotKind::Pool => "pool",
                        CapacitySlotKind::Heap => "heap",
                    };
                    if let Ty::Named(n) = &elem_ty {
                        if matches!(
                            self.top.symbols.get(n),
                            Some(TopSymbol::Locus(_))
                        ) {
                            self.diags.push(Diag::ty(
                                slot.span,
                                format!(
                                    "capacity slot `{} {} of {}`: cell \
                                     type cannot be a locus. Cells are \
                                     data; loci are managed entities. \
                                     Locus recycling/free would orphan \
                                     the locus's lifecycle. Route locus \
                                     membership through `accept(c: {})` \
                                     instead, and pair with a parallel \
                                     index slot (e.g. `@form(hashmap)` \
                                     keyed by name) if name-based lookup \
                                     is needed. See spec/semantics.md § \
                                     Locus method dispatch and spec/forms.md \
                                     § Cell type restrictions.",
                                    kind_word, slot.name.name, n, n
                                ),
                            ));
                        }
                    }
                    // F.22 v1.x-4: `as_parent_for ChildL` clause —
                    // validate that ChildL exists, is a locus, and
                    // has a slot with matching name/kind/elem_ty.
                    // The mechanic (handing the parent's allocator
                    // to the child at accept-time) is the v1.x-4b
                    // runtime followup; this pass just gates the
                    // surface so a malformed override fails at
                    // typecheck.
                    if let Some(child_ident) = &slot.as_parent_for {
                        let child_name = &child_ident.name;
                        match self.top.symbols.get(child_name) {
                            Some(TopSymbol::Locus(child_info)) => {
                                if !child_info
                                    .capacity_slot_names
                                    .iter()
                                    .any(|n| n == &slot.name.name)
                                {
                                    self.diags.push(Diag::ty(
                                        child_ident.span,
                                        format!(
                                            "capacity slot `{} {}` declared \
                                             `as_parent_for {}`, but `{}` \
                                             has no slot named `{}` — \
                                             override needs a matching \
                                             slot on the child",
                                            kind_word,
                                            slot.name.name,
                                            child_name,
                                            child_name,
                                            slot.name.name
                                        ),
                                    ));
                                }
                                // TODO v1.x-4b: also verify
                                // kind + elem_ty match — needs
                                // capacity-slot kind/ty info in
                                // the symbol-level LocusInfo.
                            }
                            Some(_) => {
                                self.diags.push(Diag::ty(
                                    child_ident.span,
                                    format!(
                                        "`as_parent_for {}`: `{}` is not \
                                         a locus",
                                        child_name, child_name
                                    ),
                                ));
                            }
                            None => {
                                self.diags.push(Diag::ty(
                                    child_ident.span,
                                    format!(
                                        "`as_parent_for {}`: locus `{}` \
                                         not declared",
                                        child_name, child_name
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
            LocusMember::BirthCheck(bc) => {
                // F.27 v2: validate that the cond is a Bool expr,
                // and that the referenced closure exists on the
                // enclosing locus and is epoch-inline (same rules
                // as a regular `violate NAME;`). Payload type
                // matching against captures is deferred to a
                // follow-up phase (same as the regular violate
                // checker — see Stmt::Violate handler).
                self.locals.push();
                let cond_ty = self.check_expr(&bc.cond);
                if cond_ty != Ty::Prim(PrimType::Bool) {
                    self.diags.push(Diag::ty(
                        bc.span,
                        format!(
                            "birth_check cond must be Bool, got {}",
                            cond_ty.display()
                        ),
                    ));
                }
                if let Some(payload) = &bc.payload {
                    let _ = self.check_expr(payload);
                }
                self.locals.pop();
                match self.current_locus {
                    None => {
                        self.diags.push(Diag::ty(
                            bc.span,
                            "birth_check used outside a locus context"
                                .to_string(),
                        ));
                    }
                    Some(locus) => {
                        match locus
                            .closures
                            .iter()
                            .find(|c| c.name == bc.closure_name.name)
                        {
                            None => {
                                self.diags.push(Diag::ty(
                                    bc.closure_name.span,
                                    format!(
                                        "birth_check: locus `{}` has no \
                                         closure named `{}`",
                                        locus.name, bc.closure_name.name
                                    ),
                                ));
                            }
                            Some(c) if !c.is_inline => {
                                self.diags.push(Diag::ty(
                                    bc.closure_name.span,
                                    format!(
                                        "birth_check `{}`: closure `{}` on \
                                         locus `{}` is not declared \
                                         `epoch inline`. Only epoch-inline \
                                         closures can be fired via \
                                         birth_check (same rule as `violate`)",
                                        bc.closure_name.name,
                                        bc.closure_name.name,
                                        locus.name
                                    ),
                                ));
                            }
                            Some(_) => {
                                // Closure exists and is epoch-inline.
                            }
                        }
                    }
                }
            }
        }
    }

    fn check_fn(&mut self, decl: &'a FnDecl, locus: Option<&'a LocusInfo>) {
        let prev_locus = self.current_locus;
        if locus.is_some() {
            self.current_locus = locus;
        }
        // GH #877: the signature's own type names, before anything
        // resolves them to `Ty::Unknown`. The generic parameters are
        // in scope for the whole declaration — the signature AND the
        // `let x: T` annotations in the body — so they are pushed
        // here and restored on both exits. A method of a generic
        // locus ADDS to the locus's parameters rather than replacing
        // them: `locus Cache<K, V> { fn map<T>(k: K) -> T }` has
        // three in scope.
        let prev_generics = self.generic_params.clone();
        self.generic_params
            .extend(decl.generics.iter().map(|g| g.name.name.clone()));
        for p in &decl.params {
            self.check_type_annotation(&p.ty);
        }
        if let Some(ret) = &decl.ret {
            self.check_type_annotation(ret);
        }
        if let Some(payload) = &decl.fallible {
            self.check_type_annotation(payload);
        }
        // Stage-1 FFI (2026-05-22): @ffi fn declarations validate
        // their parameter and return types against the FFI-portable
        // type set, then skip body verification (the body is a
        // synthesized empty Block, per parse_fn_decl_with_ffi).
        // Locus context is forbidden — @ffi only valid on top-level
        // free fns at Stage 1; the parser dispatch in
        // parse_top_decl enforces this, but defend in depth here.
        // Crumb batch-2 item 1 (C→Hale re-entry): an `@export fn` is
        // a C-ABI symbol on native (and a wasm export) — its params
        // and return must sit in the same FFI-portable set the
        // `@ffi` import direction validates, and it can't be
        // fallible (no C error channel) or take defaults (fixed
        // C arity).
        if decl.export && locus.is_none() {
            for p in &decl.params {
                let ty = resolve_type_expr(&p.ty, self.known);
                if let Some(reason) = ffi_type_unportable(&ty) {
                    self.diags.push(Diag::ty(
                        p.ty.span(),
                        format!(
                            "`@export` fn `{}` parameter `{}` has type {} \
                             — {} (the export is a C-ABI symbol; only the \
                             FFI-portable set crosses)",
                            decl.name.name,
                            p.name.name,
                            ty.display(),
                            reason,
                        ),
                    ));
                }
                if p.default.is_some() {
                    self.diags.push(Diag::ty(
                        p.ty.span(),
                        format!(
                            "`@export` fn `{}`: defaulted params aren't \
                             exportable — a C caller has fixed arity",
                            decl.name.name
                        ),
                    ));
                }
            }
            if let Some(ret_te) = &decl.ret {
                let ret_ty = resolve_type_expr(ret_te, self.known);
                if let Some(reason) = ffi_type_unportable(&ret_ty) {
                    self.diags.push(Diag::ty(
                        ret_te.span(),
                        format!(
                            "`@export` fn `{}` return type {} — {}",
                            decl.name.name,
                            ret_ty.display(),
                            reason,
                        ),
                    ));
                }
            }
            if decl.fallible.is_some() {
                self.diags.push(Diag::ty(
                    decl.span,
                    format!(
                        "`@export` fn `{}`: fallible fns aren't \
                         exportable (no C error channel) — return a \
                         sentinel or split the error out",
                        decl.name.name
                    ),
                ));
            }
        }
        if let Some(ffi) = &decl.ffi {
            if locus.is_some() {
                self.diags.push(Diag::ty(
                    ffi.span,
                    "`@ffi` is only valid on top-level free fns at Stage 1, \
                     not on locus methods",
                ));
            }
            for p in &decl.params {
                let ty = resolve_type_expr(&p.ty, self.known);
                if let Some(reason) = ffi_type_unportable(&ty) {
                    self.diags.push(Diag::ty(
                        p.ty.span(),
                        format!(
                            "`@ffi` fn `{}` parameter `{}` has type {} — {}",
                            decl.name.name,
                            p.name.name,
                            ty.display(),
                            reason,
                        ),
                    ));
                }
            }
            if let Some(ret_te) = &decl.ret {
                let ret_ty = resolve_type_expr(ret_te, self.known);
                if let Some(reason) = ffi_type_unportable(&ret_ty) {
                    self.diags.push(Diag::ty(
                        ret_te.span(),
                        format!(
                            "`@ffi` fn `{}` return type {} — {}",
                            decl.name.name,
                            ret_ty.display(),
                            reason,
                        ),
                    ));
                }
            }
            self.current_locus = prev_locus;
            self.generic_params = prev_generics;
            return;
        }
        // v1.x-FORM-1: push fallible_ctx if this fn is fallible.
        let prev_fallible = self.fallible_ctx.take();
        // #335: and the declared return type for every fn.
        let prev_return = self.return_ctx.take();
        self.return_ctx = decl
            .ret
            .as_ref()
            .map(|te| resolve_type_expr(te, self.known));
        if let Some(payload_te) = &decl.fallible {
            let success_ret = match &decl.ret {
                Some(te) => resolve_type_expr(te, self.known),
                None => Ty::Unit,
            };
            let payload = resolve_type_expr(payload_te, self.known);
            self.fallible_ctx = Some((success_ret, payload));
        }
        self.locals.push();
        for p in &decl.params {
            let ty = resolve_type_expr(&p.ty, self.known);
            self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
        }
        self.check_block(&decl.body);
        self.locals.pop();
        self.fallible_ctx = prev_fallible;
        self.return_ctx = prev_return;
        self.current_locus = prev_locus;
        self.generic_params = prev_generics;
    }

    fn check_block(&mut self, block: &Block) {
        self.locals.push();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            let _ = self.check_expr(tail);
        }
        self.locals.pop();
    }

    /// Block-as-expression typecheck: walks stmts then returns the
    /// trailing expression's type. Returns `Ty::Unit` if the block
    /// has no trailing expression (caller decides whether that's an
    /// error — for if-expression arms it is).
    fn check_block_as_expr(&mut self, block: &Block) -> Ty {
        self.locals.push();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        let ty = match &block.tail {
            Some(tail) => self.check_expr(tail),
            None => Ty::Unit,
        };
        self.locals.pop();
        ty
    }

    /// GH #383: a locus-typed field may only be assigned a locus
    /// LITERAL, never a locus value produced elsewhere.
    ///
    /// `self.conn = Connection { url: next };` is a documented
    /// lifecycle event: break-before-make, the new instance built
    /// directly into this locus's arena, owned by the field. But
    /// `self.held = make_row(...)` stores a locus somebody else
    /// constructed, and the language has no way to say who owns it
    /// afterwards — the field, or the frame that produced it.
    ///
    /// Today that question is dodged by never freeing: a locus
    /// returned from a free fn is routed to a program-lifetime arena
    /// (m90), so the field's pointer stays valid because nothing
    /// reclaims it. **The leak is the safety mechanism** — the same
    /// shape as the synced-map clone-on-read (#373) and the
    /// zero-reads (#381), and the reason every attempt to give those
    /// loci a real lifetime produced either a use-after-free or
    /// silently wrong values (see #383 for the four measured
    /// attempts).
    ///
    /// Forbidding the store is what closes the question: with no way
    /// for a factory result to escape into a field, its owner is the
    /// binding that named it, and ordinary scope-exit teardown is
    /// correct. This is the same principle the language already
    /// applies to locus RETURNS from methods (CQRS / no-locus-return
    /// — `fn get() -> SomeLocus` is rejected); a locus is structure,
    /// not a value to be handed around.
    ///
    /// Scope: only a whole-field store of a locus-typed field.
    /// Non-locus fields, index/tail stores, and locus literals are
    /// untouched.
    fn check_locus_field_store(
        &mut self,
        target: &LValue,
        value: &Expr,
        span: Span,
    ) {
        /// GH #716: a stdlib handle locus whose whole purpose is to
        /// outlive the frame that produced it ships its own ownership
        /// transfer, and the general remedies above do not reach it —
        /// the pid + fds come out of a syscall inside a factory, so
        /// there is no literal to write and `accept()` cannot adopt
        /// what a free fn built. Name the module's transfer instead
        /// of leaving the author to hand-copy the fields, which is
        /// the double-close this rule exists to prevent.
        fn handle_handoff_hint(tname: &str, field: &str) -> String {
            let child = hale_stdlib::PATH_RENAMES
                .iter()
                .find(|(p, _)| *p == ["std", "process", "Child"])
                .map(|(_, m)| *m);
            if Some(tname) != child {
                return String::new();
            }
            format!(
                "\n\nA spawned `std::process::Child` is the exception \
                 that has a transfer: `std::process::adopt` moves the \
                 handle's pid and pipe fds into the Child this field \
                 already owns, releases whatever it held, and disarms \
                 the source so only one handle ever closes them:\n\
                 \n    let spawned = std::process::spawn(argv) or \
                 std::process::Child {{ }};\
                 \n    std::process::adopt(self.{}, spawned);\n\n\
                 A failed spawn hands back an empty Child, so adopting \
                 it leaves the field empty rather than half-built. Do \
                 not copy pid/fds across by hand — two armed handles \
                 double-close.",
                field
            )
        }
        // whole-field store only (`self.x = …` / `x.y = …`, one
        // segment), and the field must be locus-typed
        if target.tail.len() != 1 {
            return;
        }
        let want = self.lvalue_ty(target);
        let Ty::Named(tname) = &want else { return };
        // An `interface`- or `perspective(P)`-typed field returns here:
        // only an interface VALUE (a param, a field — a fat pointer
        // somebody else owns) can reach such a store, and codegen
        // treats the store as a borrow (GH #967): the field's own
        // child is reclaimed, the holder never reclaims the handle.
        // The locus-typed case below is the ambiguous one.
        if !matches!(self.top.lookup(tname), Some(TopSymbol::Locus(_))) {
            return;
        }
        // A literal is construction-in-place: allowed, and the
        // documented reassignment lifecycle event.
        if matches!(value, Expr::Struct { .. }) {
            return;
        }
        // Anything else naming a locus value is the ambiguous case.
        // index stores (`x[i] = …`) are not whole-field stores
        let Some(LValueSeg::Field(fid)) = target.tail.first() else {
            return;
        };
        let field = fid.name.clone();
        self.diags.push(Diag::ty(
            span,
            format!(
                "cannot assign an existing locus value into locus-typed \
                 field `{}`: ownership would be ambiguous — the field \
                 and the frame that produced the value would both \
                 claim it.\n\n\
                 Assign a locus LITERAL instead, which builds the new \
                 instance directly into this locus's arena and is the \
                 documented reassignment lifecycle event:\n\
                 \n    self.{} = {} {{ ... }};\n\n\
                 If the value must come from a factory, route the \
                 membership through `accept(c: {})` instead of a \
                 field, or have the factory hand back the data and \
                 build the locus here. Same principle as the \
                 no-locus-return rule on methods: a locus is \
                 structure, not a value to hand around.{}",
                field,
                field,
                tname,
                tname,
                handle_handoff_hint(tname, &field)
            ),
        ));
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { is_mut, name, ty, value, .. } => {
                let got = self.check_expr_addressed(value);
                let bound = match ty {
                    Some(te) => {
                        // GH #877: the one annotation that lives in a
                        // body.
                        self.check_type_annotation(te);
                        let want = resolve_type_expr(te, self.known);
                        // GH #911 B5: `let h: Holder<Int> = Holder { };`
                        // — the annotation resolves to the mangled
                        // monomorph `Holder_Int` while the literal
                        // types as the template `Holder`. Codegen
                        // rewrites the bare template name against
                        // exactly this ascription and builds the
                        // monomorph, for a generic locus as well as a
                        // generic type; the checker refused both, so
                        // neither shape was reachable outside the
                        // codegen tests (which skip the checker).
                        let monomorph = self
                            .two_spellings_of_one_monomorph(
                                &want, &got, true,
                            );
                        if !monomorph && !want.assignable_from(&got) {
                            self.diags.push(Diag::ty(
                                value.span(),
                                format!(
                                    "let `{}`: expected `{}`, got `{}`",
                                    name.name,
                                    want.display(),
                                    got.display()
                                ),
                            ));
                        }
                        want
                    }
                    None => {
                        // GH #911 B5: no annotation is no arguments.
                        if matches!(value, Expr::Struct { .. }) {
                            self.refuse_generic_literal_without_arguments(
                                &got,
                                value.span(),
                            );
                        }
                        got
                    }
                };
                self.locals.insert(
                    &name.name,
                    LocalSym { ty: bound, is_mut: *is_mut },
                );
            }
            Stmt::LetTuple { is_mut, names, ty, value, .. } => {
                let got = self.check_expr_addressed(value);
                // GH #877: `let (a, b): (Int, Strng) = ...` — the
                // annotation is a tuple type expression, walked the
                // same way.
                if let Some(te) = ty {
                    self.check_type_annotation(te);
                }
                let elem_tys: Vec<Ty> = match (&got, ty) {
                    (Ty::Tuple(parts), _) if parts.len() == names.len() => {
                        parts.clone()
                    }
                    (Ty::Tuple(parts), _) => {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "let-tuple: expected {} elements, got `{}`",
                                names.len(),
                                got.display()
                            ),
                        ));
                        // Best-effort: pad / truncate so subsequent
                        // typechecking can still proceed.
                        let mut v = parts.clone();
                        v.resize(names.len(), Ty::Unknown);
                        v
                    }
                    (other, _) => {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "let-tuple: rhs is `{}`, not a tuple",
                                other.display()
                            ),
                        ));
                        vec![Ty::Unknown; names.len()]
                    }
                };
                for (n, t) in names.iter().zip(elem_tys.iter()) {
                    self.locals.insert(
                        &n.name,
                        LocalSym { ty: t.clone(), is_mut: *is_mut },
                    );
                }
            }
            Stmt::Assign { target, value, span, .. } => {
                let got = self.check_expr_addressed(value);
                let want = self.lvalue_ty(target);
                // bounded[T; N] fields cannot be whole-assigned
                // (even from another bounded of the same shape —
                // no copy semantics exist; the mutation surface is
                // push/clear).
                if matches!(want, Ty::Bounded(_, _)) {
                    self.diags.push(Diag::ty(
                        *span,
                        "bounded[T; N] fields cannot be assigned as a \
                         whole — mutate through push(...) / clear(...)"
                            .to_string(),
                    ));
                }
                if !want.assignable_from(&got) {
                    self.diags.push(Diag::ty(
                        value.span(),
                        format!(
                            "assignment: target type `{}` not assignable from `{}`",
                            want.display(),
                            got.display()
                        ),
                    ));
                }
                self.check_locus_field_store(target, value, *span);
                // m50: bare-head reassignment to a non-mut local is
                // a compile-time error per spec/types.md "Mutability"
                // + design-rationale §E. Field/index segments
                // (`x.field = ...`, `x[i] = ...`) don't rebind the
                // local — they mutate state through it — so they
                // stay allowed even when the head binding is
                // immutable. `self.field = ...` is also allowed
                // because `self` is locus state, not a binding.
                if target.tail.is_empty() && target.head.name != "self" {
                    if let Some(sym) = self.locals.lookup(&target.head.name) {
                        if !sym.is_mut {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "cannot assign to `{}`: binding is \
                                     immutable. Declare with `let mut {}` \
                                     to permit reassignment.",
                                    target.head.name, target.head.name
                                ),
                            ));
                        }
                    }
                }
            }
            Stmt::Send { subject, value, span, or_disposition } => {
                self.check_send(subject, value, or_disposition.as_ref(), *span);
            }
            Stmt::If(if_stmt) => self.check_if(if_stmt),
            Stmt::Match(m) => self.check_match(m),
            Stmt::For { name, iter, body, .. } => {
                // 2026-07-02 @form iteration surface: `for e in
                // m.entries` (hashmap) / `for x in v.items` (vec).
                // `entries`/`items` are pseudo-fields the generic
                // field check would reject — when the receiver is a
                // locus, check only the receiver and bind the loop
                // var Unknown (codegen resolves the cell type and
                // rejects non-form receivers with a focused error).
                let mut handled = false;
                if let Expr::Field { receiver, name: fname, .. } = iter {
                    if fname.name == "entries" || fname.name == "items" {
                        let recv_ty = self.check_expr(receiver);
                        if let Ty::Named(ln) = &recv_ty {
                            if matches!(
                                self.top.lookup(ln),
                                Some(TopSymbol::Locus(_))
                            ) {
                                handled = true;
                            }
                        }
                    }
                }
                if !handled {
                    let _ = self.check_expr(iter);
                }
                self.locals.push();
                self.locals.insert(&name.name, LocalSym { ty: Ty::Unknown, is_mut: false });
                self.check_block(body);
                self.locals.pop();
            }
            Stmt::While { cond, body, .. } => {
                let ct = self.check_expr(cond);
                if !ct.assignable_from(&Ty::Prim(PrimType::Bool)) {
                    self.diags.push(Diag::ty(
                        cond.span(),
                        format!(
                            "while condition must be Bool; got `{}`",
                            ct.display()
                        ),
                    ));
                }
                self.check_block(body);
            }
            Stmt::Return(expr, _) => {
                if let Some(e) = expr {
                    let got = self.check_expr_addressed(e);
                    // v1.x-FORM-1: returning from a fallible fn
                    // means returning the success value; payload
                    // type is checked at `fail` sites instead.
                    // Check that the returned type matches the fn's
                    // declared success return type when in a
                    // fallible body.
                    if let Some((expected_ret, _)) = &self.fallible_ctx {
                        // GH #911 B5: the return slot is the other
                        // site codegen rewrites a bare generic
                        // template name at — `fn make() -> Box<Int> {
                        // return Box { value: 4 }; }` builds and runs.
                        let monomorph = self
                            .two_spellings_of_one_monomorph(
                                expected_ret,
                                &got,
                                true,
                            );
                        if !monomorph && !expected_ret.assignable_from(&got) {
                            self.diags.push(Diag::ty(
                                e.span(),
                                format!(
                                    "return: expected `{}`, got `{}`",
                                    expected_ret.display(),
                                    got.display()
                                ),
                            ));
                        }
                    } else if let Some(expected_ret) = &self.return_ctx {
                        // #335: the non-fallible case, which had no
                        // check. Same Unknown-permissive rule, and the
                        // same Int -> Float widening a call site
                        // allows.
                        let widening = matches!(
                            (expected_ret, &got),
                            (
                                Ty::Prim(PrimType::Float),
                                Ty::Prim(PrimType::Int)
                            )
                        );
                        // GH #911 B5: the same return-slot rewrite as
                        // the fallible branch above.
                        let monomorph = self
                            .two_spellings_of_one_monomorph(
                                expected_ret,
                                &got,
                                true,
                            );
                        if !widening
                            && !monomorph
                            && !expected_ret.assignable_from(&got)
                        {
                            self.diags.push(Diag::ty(
                                e.span(),
                                format!(
                                    "return: expected `{}`, got `{}`",
                                    expected_ret.display(),
                                    got.display()
                                ),
                            ));
                        }
                    }
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) => {}
            Stmt::Terminate(span) => {
                // `terminate;` ends the *current locus's* own
                // lifecycle (the locus analogue of `return`), so it
                // only has meaning inside a locus method body —
                // there must be a `self` whose lifecycle to end. In
                // a free function there is no locus to terminate;
                // previously this fell through to a codegen
                // "no self" error with no source location. Gate it
                // here with a focused diagnostic.
                if self.current_locus.is_none() {
                    self.diags.push(Diag::ty(
                        *span,
                        "`terminate` is only valid inside a locus method \
                         — it ends the enclosing locus's own lifecycle, so \
                         there is nothing to terminate in a free function"
                            .to_string(),
                    ));
                }
            }
            Stmt::Reperspective { field, impl_name, span } => {
                self.check_reperspective(field, impl_name, *span);
            }
            Stmt::Fail { value, span } => {
                // v1.x-FORM-1: `fail <expr>;` must appear inside
                // a fallible fn body, and its payload type must
                // match the fn's declared fallible(T) payload.
                // The parser already gates statement-position
                // recognition on the in-fallible-body flag, but
                // we re-check at typecheck for completeness and
                // to produce a clear diagnostic if a Fail node
                // is constructed by other means (interpreter
                // synth, future macro, etc.).
                let payload_ty = self.check_expr_addressed(value);
                match &self.fallible_ctx {
                    None => self.diags.push(Diag::ty(
                        *span,
                        "fail: `fail <expr>;` is only valid inside a \
                         fallible fn body (declared with `fallible(T)`)"
                            .to_string(),
                    )),
                    Some((_, expected_payload)) => {
                        if !expected_payload.assignable_from(&payload_ty) {
                            self.diags.push(Diag::ty(
                                value.span(),
                                format!(
                                    "fail: expected payload type `{}`, got `{}`",
                                    expected_payload.display(),
                                    payload_ty.display()
                                ),
                            ));
                        }
                    }
                }
            }
            Stmt::Block(b) => self.check_block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    let _ = self.check_expr(a);
                }
                if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                    let _ = self.check_expr(e);
                }
            }
            // v1.x-VIOLATE (F.27): rejection-context enforcement
            // + closure-name resolution against the enclosing
            // locus + epoch-inline gate. The parser already
            // accepts `violate NAME [with EXPR];` only at
            // statement positions where it disambiguates from a
            // function call; here we enforce the structural
            // rules from F.27.
            Stmt::Violate { name, payload, span } => {
                if let Some(p) = payload {
                    let _ = self.check_expr(p);
                }
                match self.current_locus {
                    None => {
                        self.diags.push(Diag::ty(
                            *span,
                            format!(
                                "`violate {}`: free fns can't use `violate` \
                                 (no `self` to anchor the closure name). \
                                 Use `fail <payload>;` if this fn is \
                                 declared `fallible(E)`, or move the call \
                                 into a locus method body",
                                name.name,
                            ),
                        ));
                    }
                    Some(locus) if self.in_on_failure => {
                        self.diags.push(Diag::ty(
                            *span,
                            format!(
                                "`violate {}`: not allowed inside an \
                                 `on_failure` body (use `bubble(err)` to \
                                 propagate the child's failure instead — \
                                 `on_failure` is the parent-side handler, \
                                 not a place to fire `{}`'s own closures)",
                                name.name, locus.name,
                            ),
                        ));
                    }
                    Some(locus) => {
                        match locus.closures.iter().find(|c| c.name == name.name) {
                            None => {
                                self.diags.push(Diag::ty(
                                    name.span,
                                    format!(
                                        "`violate {}`: locus `{}` has no \
                                         closure named `{}`",
                                        name.name, locus.name, name.name,
                                    ),
                                ));
                            }
                            Some(c) if !c.is_inline => {
                                self.diags.push(Diag::ty(
                                    name.span,
                                    format!(
                                        "`violate {}`: closure `{}` on locus \
                                         `{}` is not declared `epoch inline`. \
                                         Only assertion-less, inline-epoch \
                                         closures can be fired via `violate`; \
                                         add `epoch inline;` to its clause \
                                         list (or use `bubble(err)` from an \
                                         `on_failure` body instead)",
                                        name.name, name.name, locus.name,
                                    ),
                                ));
                            }
                            Some(_) => {
                                // Closure exists and is epoch-inline.
                                // Payload-type validation against the
                                // closure's captures + `with` shape
                                // lands in phase 4 alongside
                                // ClosureViolation synthesis.
                            }
                        }
                    }
                }
            }
            Stmt::Expr(e) => {
                // M3 stage 2 (2026-07-02): a statement-position
                // `call() or fallback` discards its value, so the
                // fallback's type needn't match the success type —
                // `write_file(p, s) or handler(err);` with a
                // Bool-returning handler is fine (and common in
                // production code). The flag is consumed (and
                // cleared) by the Or/Substitute arm.
                if matches!(e, Expr::Or { .. }) {
                    self.or_value_discarded = true;
                }
                let got = self.check_expr_addressed(e);
                self.or_value_discarded = false;
                // GH #911 B5: `Cache { cap: 2 };` in statement
                // position — the other site with no declared type to
                // take the arguments from. (`App { };`, the ordinary
                // main-locus instantiation, names no generic
                // template and is untouched.)
                if matches!(e, Expr::Struct { .. }) {
                    self.refuse_generic_literal_without_arguments(
                        &got,
                        e.span(),
                    );
                }
            }
            Stmt::ShmWrite { topic, max, binding, body, span } => {
                // The receiver must be a declared topic (its layout-bound
                // producer binding is validated at the binding site).
                if !matches!(
                    self.top.lookup(&topic.name),
                    Some(TopSymbol::Topic(_))
                ) {
                    self.diags.push(Diag::ty(
                        topic.span,
                        format!("`{}.write`: `{}` is not a declared topic",
                                topic.name, topic.name),
                    ));
                }
                let max_ty = self.check_expr_addressed(max);
                if !matches!(max_ty, Ty::Prim(PrimType::Int) | Ty::Unknown) {
                    self.diags.push(Diag::ty(
                        max.span(),
                        format!("`{}.write(max)`: max must be Int, got {}",
                                topic.name, max_ty.display()),
                    ));
                }
                // Bind the writable view over the body, then require the
                // body to end with the Int byte count to commit.
                self.locals.push();
                self.locals.insert(
                    &binding.name,
                    LocalSym { ty: Ty::Unknown, is_mut: false },
                );
                for s in &body.stmts {
                    self.check_stmt(s);
                }
                match &body.tail {
                    Some(t) => {
                        let tt = self.check_expr_addressed(t);
                        if !matches!(tt, Ty::Prim(PrimType::Int) | Ty::Unknown) {
                            self.diags.push(Diag::ty(
                                t.span(),
                                format!(
                                    "`{}.write {{ ... }}` must end with the Int \
                                     byte count to commit, got {}",
                                    topic.name, tt.display()
                                ),
                            ));
                        }
                    }
                    None => self.diags.push(Diag::ty(
                        *span,
                        format!(
                            "`{}.write {{ ... }}` must end with the Int byte \
                             count to commit (the record length)",
                            topic.name
                        ),
                    )),
                }
                self.locals.pop();
            }
        }
    }

    fn check_if(&mut self, stmt: &IfStmt) {
        let ct = self.check_expr(&stmt.cond);
        if !ct.assignable_from(&Ty::Prim(PrimType::Bool)) {
            self.diags.push(Diag::ty(
                stmt.cond.span(),
                format!("if condition must be Bool; got `{}`", ct.display()),
            ));
        }
        self.check_block(&stmt.then_block);
        if let Some(else_branch) = &stmt.else_block {
            match else_branch.as_ref() {
                ElseBranch::Else(b) => self.check_block(b),
                ElseBranch::ElseIf(s) => self.check_if(s),
            }
        }
    }

    /// If-as-expression: cond checked as Bool; then/else arms checked
    /// as block-expressions; the result type is the unified arm type.
    /// Returns `Ty::Unknown` if arms disagree (with a diagnostic).
    fn check_if_as_expr(&mut self, stmt: &IfStmt) -> Ty {
        let ct = self.check_expr(&stmt.cond);
        if !ct.assignable_from(&Ty::Prim(PrimType::Bool)) {
            self.diags.push(Diag::ty(
                stmt.cond.span(),
                format!("if condition must be Bool; got `{}`", ct.display()),
            ));
        }
        let then_ty = self.check_block_as_expr(&stmt.then_block);
        let else_ty = match &stmt.else_block {
            Some(b) => match b.as_ref() {
                ElseBranch::Else(blk) => self.check_block_as_expr(blk),
                ElseBranch::ElseIf(nested) => self.check_if_as_expr(nested),
            },
            None => Ty::Unit,
        };
        if then_ty.display() != else_ty.display()
            && !then_ty.assignable_from(&else_ty)
            && !else_ty.assignable_from(&then_ty)
        {
            self.diags.push(Diag::ty(
                stmt.span,
                format!(
                    "if-expression arms have mismatched types: \
                     then=`{}`, else=`{}`",
                    then_ty.display(),
                    else_ty.display()
                ),
            ));
            return Ty::Unknown;
        }
        then_ty
    }

    fn check_match(&mut self, stmt: &MatchStmt) {
        let _ = self.check_match_core(stmt, false);
    }

    /// Enter a match arm's pattern binders into the current scope.
    ///
    /// Typed `Unknown` deliberately: the binders' types are already
    /// resolved by codegen (scrutinee for a bare binder, element type
    /// for a tuple sub-pattern, payload field for a constructor arg),
    /// and inventing a narrower type here would turn a scope fix into
    /// a new class of type error on programs that compile today. What
    /// this establishes is that the NAME exists.
    fn bind_pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Binding(id) => self.locals.insert(
                &id.name,
                LocalSym { ty: Ty::Unknown, is_mut: false },
            ),
            Pattern::Constructor { args, .. } => {
                for a in args {
                    self.bind_pattern(a);
                }
            }
            Pattern::Tuple(parts, _) => {
                for p in parts {
                    self.bind_pattern(p);
                }
            }
            Pattern::Literal(_, _) | Pattern::Wildcard(_) => {}
        }
    }

    /// Shared match checking (Gap C, 2026-07-17). In statement
    /// position (`as_expr = false`) arm-body types are discarded —
    /// heterogeneous arms are legal, exactly the pre-Gap-C
    /// behavior. In expression position the match's type is the
    /// join of its arm-body types: every value arm must agree
    /// (`Unknown` is lenient in either direction), and a mismatch
    /// diags here with the match's span rather than surfacing as a
    /// codegen error. Block arm bodies type as `Unknown` at v0.1
    /// (no block-tail typing helper yet); codegen's phi-build
    /// still validates them.
    fn check_match_core(&mut self, stmt: &MatchStmt, as_expr: bool) -> Ty {
        let scrut_ty = self.check_expr(&stmt.scrutinee);
        let mut joined: Option<Ty> = None;
        for arm in &stmt.arms {
            // GH #721: an arm's pattern BINDS — `v`, `(a, b)`,
            // `Event::Tick(n)` — and the binders are in scope for the
            // guard and the body (codegen's `bindings` vector does
            // exactly this). The checker never entered them, which
            // was invisible while an unresolved name typed as
            // `Unknown` and became a false "unknown identifier" the
            // moment that stopped being free.
            self.locals.push();
            self.bind_pattern(&arm.pattern);
            if let Some(g) = &arm.guard {
                let _ = self.check_expr(g);
            }
            let arm_ty = match &arm.body {
                MatchArmBody::Expr(e) => self.check_expr(e),
                MatchArmBody::Block(b) => {
                    self.check_block(b);
                    Ty::Unknown
                }
            };
            self.locals.pop();
            if as_expr {
                match &joined {
                    None => joined = Some(arm_ty),
                    Some(prev) => {
                        if *prev == Ty::Unknown {
                            joined = Some(arm_ty);
                        } else if arm_ty != Ty::Unknown && arm_ty != *prev {
                            self.diags.push(Diag::ty(
                                stmt.span,
                                format!(
                                    "match expression arms have \
                                     mismatched types: `{}` vs `{}`",
                                    prev.display(),
                                    arm_ty.display()
                                ),
                            ));
                        }
                    }
                }
            }
        }
        if !match_is_exhaustive(&scrut_ty, &stmt.arms, self.top) {
            self.diags.push(Diag::ty(
                stmt.span,
                format!(
                    "match is not exhaustive; add a `_` arm or cover all \
                     cases of `{}`",
                    scrut_ty.display()
                ),
            ));
        }
        if as_expr {
            joined.unwrap_or(Ty::Unknown)
        } else {
            Ty::Unit
        }
    }

    fn check_send(
        &mut self,
        subject: &Expr,
        value: &Expr,
        or_disposition: Option<&OrDisposition>,
        span: Span,
    ) {
        let payload_ty = self.check_expr(value);
        // Phase 3 routing keys (2026-05-25): the `or DISPOSITION`
        // clause on Send is legal only when the target topic
        // declares `on_unmatched: fail`. Conversely, fail topics
        // REQUIRE the clause — a fail-policy publish without an
        // or-disposition leaves the no-match err unhandled. We
        // resolve the topic by name/literal-subject and validate
        // both directions.
        let target_topic: Option<(String, Option<UnmatchedPolicy>, bool)> =
            match subject {
                Expr::Literal(Literal::String(s), _) => self
                    .top
                    .symbols
                    .values()
                    .find_map(|sym| match sym {
                        TopSymbol::Topic(ti)
                            if ti.subject == *s
                                || ti.wire_subject == *s
                                || ti.name == *s =>
                        {
                            Some((
                                ti.name.clone(),
                                ti.on_unmatched,
                                ti.on_full_fail,
                            ))
                        }
                        _ => None,
                    }),
                Expr::Ident(id) => match self.top.lookup(&id.name) {
                    Some(TopSymbol::Topic(ti)) => Some((
                        ti.name.clone(),
                        ti.on_unmatched,
                        ti.on_full_fail,
                    )),
                    _ => None,
                },
                _ => None,
            };
        let target_policy = target_topic.as_ref().map(|(_, p, _)| *p);
        let target_full_fail =
            target_topic.as_ref().map_or(false, |(_, _, f)| *f);
        // GH #255 phase 2: an `on_full: fail` topic's publishes are
        // refusal-fallible — every send site carries a disposition.
        // v1 wires raise / discard / wait; the err-payload
        // dispositions (`or handler(err)` / `or fail <p>`) land in
        // the follow-up slice and are rejected with a pointer.
        if target_full_fail {
            match or_disposition {
                None => self.diags.push(Diag::ty(
                    span,
                    "publish to a topic with `on_full: fail` must \
                     carry an `or` disposition — e.g. \
                     `Subject <- value or raise` (or `or discard` / \
                     `or wait`)",
                )),
                Some(OrDisposition::Substitute(_))
                | Some(OrDisposition::Fail(_, _)) => {
                    self.diags.push(Diag::ty(
                        span,
                        "`or handler(err)` / `or fail <payload>` on an \
                         `on_full: fail` topic aren't wired yet — use \
                         `or raise`, `or discard`, or `or wait` (the \
                         err-payload dispositions land in the next \
                         slice)",
                    ));
                }
                _ => {}
            }
        }
        // GH #255 phase 1: `or wait` is its own disposition kind —
        // a delivery-mode modifier on a NON-fallible publish, only
        // meaningful when the topic has a declared transport
        // binding (the loss window is what it waits out). Checked
        // before the fail-policy matrix below because it is not a
        // member of the fallible-disposition family.
        if let Some(OrDisposition::Wait(wsp)) = or_disposition {
            if matches!(
                target_policy.flatten(),
                Some(UnmatchedPolicy::Fail)
            ) {
                self.diags.push(Diag::ty(
                    *wsp,
                    "`or wait` is not an unmatched-key disposition — an \
                     unmatched routing key is not a transient \
                     condition to outwait. Use `or raise` / \
                     `or discard` / `or handler(err)` / \
                     `or fail <payload>`",
                ));
            } else {
                let is_bound = target_topic
                    .as_ref()
                    .map_or(false, |(name, _, full_fail)| {
                        *full_fail || self.bound_topics.contains(name)
                    });
                if !is_bound {
                    self.diags.push(Diag::ty(
                        *wsp,
                        "`or wait` requires the topic to have a declared \
                         transport binding (a loss window to wait \
                         out) or `on_full: fail` capacity (queue \
                         space to wait for)",
                    ));
                }
            }
        }
        match (target_policy.flatten(), or_disposition) {
            // GH #255: wait was fully validated above — pass it
            // through so the fail-policy matrix doesn't
            // mis-diagnose it as an illegal disposition.
            (_, Some(OrDisposition::Wait(_))) => {}
            (Some(UnmatchedPolicy::Fail), None) => {
                self.diags.push(Diag::ty(
                    span,
                    "publish to topic with `on_unmatched: fail` must \
                     carry an `or` disposition — e.g. \
                     `Subject <- value or raise`",
                ));
            }
            (Some(UnmatchedPolicy::Fail), Some(disp)) => {
                // v0.2 (2026-05-26): all four dispositions wired.
                //   - Raise / Discard: as v0.1 — no err-payload
                //     needed, codegen panics or no-ops.
                //   - Substitute: `err: BusUnmatchedKey` in scope
                //     on the RHS; expression evaluated for side
                //     effects (Send is a statement, no value
                //     binding to type-match).
                //   - Fail: `err: BusUnmatchedKey` in scope on
                //     the payload expr; payload type must match
                //     the enclosing fallible fn's declared err.
                match disp {
                    OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
                    // Unreachable: the Wait arm of the outer
                    // matrix consumed it (GH #255).
                    OrDisposition::Wait(_) => {}
                    OrDisposition::Substitute(rhs) => {
                        let err_ty =
                            Ty::Named("BusUnmatchedKey".to_string());
                        self.locals.push();
                        self.locals.insert(
                            "err",
                            LocalSym {
                                ty: err_ty,
                                is_mut: false,
                            },
                        );
                        let _ = self.check_expr(rhs);
                        self.locals.pop();
                    }
                    OrDisposition::Fail(payload_expr, sp) => {
                        let err_ty =
                            Ty::Named("BusUnmatchedKey".to_string());
                        self.locals.push();
                        self.locals.insert(
                            "err",
                            LocalSym {
                                ty: err_ty,
                                is_mut: false,
                            },
                        );
                        let new_payload_ty =
                            self.check_expr_addressed(payload_expr);
                        self.locals.pop();
                        match &self.fallible_ctx {
                            None => self.diags.push(Diag::ty(
                                *sp,
                                "`or fail X`: only valid inside a \
                                 fallible fn body (declared with \
                                 `fallible(T)`). Use `or raise` to \
                                 propagate the no-match as a panic, \
                                 or `or <expr>` to side-effect a \
                                 handler.",
                            )),
                            Some((_, expected_payload)) => {
                                if !expected_payload
                                    .assignable_from(&new_payload_ty)
                                {
                                    self.diags.push(Diag::ty(
                                        payload_expr.span(),
                                        format!(
                                            "`or fail`: expected \
                                             payload type `{}`, got \
                                             `{}`",
                                            expected_payload.display(),
                                            new_payload_ty.display()
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            (_, Some(_)) => {
                self.diags.push(Diag::ty(
                    span,
                    "`or` disposition on a bus send is only legal \
                     when the target topic declares \
                     `on_unmatched: fail`",
                ));
            }
            (_, None) => {
                // Default path — unkeyed / swallow / fallback.
            }
        }
        // Subject extraction. Two static forms produce a fixed
        // wire-format subject string: a literal `"S" <- expr` and
        // a topic-ref `Foo <- expr` where Foo names a `topic`
        // decl. Anything else is a computed subject and goes
        // through the wildcard-publish path further below.
        let subject_str = match subject {
            Expr::Literal(Literal::String(s), _) => Some(s.clone()),
            Expr::Ident(id) => match self.top.lookup(&id.name) {
                Some(TopSymbol::Topic(_)) => Some(id.name.clone()),
                _ => None,
            },
            // A7 (G16): cross-seed `alias::Topic <- payload;`. The
            // typechecker can't resolve cross-seed names directly
            // (mangling happens at the codegen-side pre-pass), so
            // we use the leaf segment as the subject — mirroring
            // resolve_bus_subject's handling of QualifiedTopic in
            // subscribe/publish declarations, which also stores the
            // leaf name. The locus's bus_publishes entry for this
            // topic has payload=Unknown, so the assignability check
            // below is permissive; the codegen-side mangle resolves
            // the full path and binds the wire subject.
            Expr::Path(qn) if qn.segments.len() > 1 => {
                qn.segments.last().map(|s| s.name.clone())
            }
            _ => None,
        };
        let locus = match self.current_locus {
            Some(l) => l,
            None => {
                self.diags.push(Diag::ty(
                    span,
                    "bus send (`<-`) only valid inside a locus body".to_string(),
                ));
                return;
            }
        };
        // m94: a non-literal subject is allowed when the locus
        // declares a wildcard `publish` whose payload matches.
        // The wildcard declaration acts as the authorization +
        // type-binding for any concrete subject computed at
        // runtime that matches the pattern. Static subject-pattern
        // verification is impossible by definition; we trust the
        // declaration and let runtime dispatch route to whichever
        // subscribers (exact or wildcard) match.
        let subject_str = match subject_str {
            Some(s) => s,
            None => {
                let wildcard_match = locus.bus_publishes.iter().find(|p| {
                    p.subject.contains("**")
                        && p.payload.assignable_from(&payload_ty)
                });
                if wildcard_match.is_none() {
                    let any_wildcard = locus
                        .bus_publishes
                        .iter()
                        .any(|p| p.subject.contains("**"));
                    if any_wildcard {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "bus send (computed subject): payload `{}` does \
                                 not match any wildcard publish declaration in \
                                 locus `{}`",
                                payload_ty.display(),
                                locus.name
                            ),
                        ));
                    } else {
                        self.diags.push(Diag::ty(
                            subject.span(),
                            format!(
                                "bus send with computed subject requires a \
                                 wildcard `publish` declaration (e.g. \
                                 `publish \"log.**\" of type T`) in locus `{}`",
                                locus.name
                            ),
                        ));
                    }
                }
                return;
            }
        };
        let pub_decl = locus
            .bus_publishes
            .iter()
            .find(|p| p.subject == subject_str);
        match pub_decl {
            Some(decl) => {
                if !decl.payload.assignable_from(&payload_ty) {
                    self.diags.push(Diag::ty(
                        value.span(),
                        format!(
                            "bus send `{}`: payload `{}` not assignable to declared `{}`",
                            subject_str,
                            payload_ty.display(),
                            decl.payload.display()
                        ),
                    ));
                }
            }
            None => {
                // m94: an exact-literal subject is also valid when
                // it matches a wildcard publish declaration of the
                // right type. This lets a locus declare
                // `publish "log.**" of type LogEvent` once and
                // then send on `"log.app"` etc. literally.
                let wildcard_match = locus.bus_publishes.iter().find(|p| {
                    p.subject.contains("**")
                        && super::wildcard_match(&p.subject, &subject_str)
                        && p.payload.assignable_from(&payload_ty)
                });
                if wildcard_match.is_none() {
                    self.diags.push(Diag::ty(
                        subject.span(),
                        format!(
                            "bus send subject `{}` is not declared in locus `{}`'s bus block",
                            subject_str, locus.name
                        ),
                    ));
                }
            }
        }
    }

    fn lvalue_ty(&mut self, lv: &LValue) -> Ty {
        let mut ty = if lv.head.name == "self" {
            self.self_ty()
        } else if let Some(s) = self.locals.lookup(&lv.head.name) {
            s.ty.clone()
        } else {
            Ty::Unknown
        };
        for seg in &lv.tail {
            match seg {
                LValueSeg::Field(f) => {
                    // GH #436 follow-up: an assignment TARGET resolves
                    // here, not through the expression field-access
                    // arm, so sealing was enforced on reads and not on
                    // writes. Confinement that stops a read and permits
                    // a write is not confinement — for `std::secret` it
                    // let outside code CHOOSE the signing key, which is
                    // worse than reading it.
                    self.check_sealed_access(
                        &ty,
                        f,
                        f.span,
                        SealedAccess::Write,
                    );
                    ty = self.field_ty(&ty, &f.name).unwrap_or(Ty::Unknown);
                }
                LValueSeg::Index(idx) => {
                    let _ = self.check_expr(idx);
                    ty = match ty {
                        Ty::Array(elem, _) => *elem,
                        _ => Ty::Unknown,
                    };
                }
            }
        }
        ty
    }

    fn self_ty(&self) -> Ty {
        match self.current_locus {
            Some(l) => Ty::Named(l.name.clone()),
            None => Ty::Unknown,
        }
    }

    /// Look up a named field on a type. Resolves struct fields,
    /// locus params (when accessing a locus handle's exposed
    /// state — milestone 2 just exposes all params), and
    /// perspective params.
    /// Verify that a locus structurally implements an interface:
    /// for every method the interface declares, the locus has a
    /// method with the same name, same arity, compatible param
    /// types, and a compatible return type. Returns Err with a
    /// human-readable message on the first mismatch.
    ///
    /// Both arguments are top-symbol names. Caller has already
    /// verified that `iface_name` resolves to a TopSymbol::Interface.
    /// `locus_name` may be any TopSymbol — non-locus returns Err.
    /// GH #724: does this name reference a seed the bundle does not
    /// hold? True only for a `::`-joined path whose head is an import
    /// alias still unresolved in this bundle AND which the rename
    /// table cannot map. A resolvable path never reaches here as a
    /// path — the import-rename pass collapsed it to the imported
    /// declaration's mangled name before typecheck — so this is the
    /// "we genuinely cannot see the declaration" case, kept tolerant
    /// exactly as an imported struct literal's fields are when its
    /// declaration is invisible.
    fn unresolved_alias_path(&self, name: &str) -> bool {
        let Some((head, _)) = name.split_once("::") else {
            return false;
        };
        if !self.unresolved_import_aliases.contains(head) {
            return false;
        }
        let key: Vec<String> =
            name.split("::").map(|s| s.to_string()).collect();
        !self.import_renames.iter().any(|(p, _)| *p == key)
    }

    /// Phase 2a: verify a `locus L : serves P` provides every
    /// method of perspective contract `P`. Emits a diagnostic per
    /// missing / mismatched method (arity, param types, return
    /// type) — the perspective analog of `check_structural_impl`
    /// for interfaces, reading the contract's method signatures.
    fn check_serves_conformance(&mut self, decl: &LocusDecl) {
        let Some(TopSymbol::Locus(locus)) =
            self.top.lookup(&decl.name.name)
        else {
            return;
        };
        for persp_name in &decl.serves {
            // GH #724: `serves lib::Routing` where this bundle holds
            // no `lib`. The contract is behind an alias we cannot
            // see, so there is nothing to conform to — the same
            // tolerance every other qualified reference already has
            // in that situation. A path whose head is NOT an
            // unresolved alias (`lib::Nope` in a bundle that did
            // resolve `lib`, a bare typo) still reports below.
            if self.unresolved_alias_path(&persp_name.name) {
                continue;
            }
            let persp = match self.top.lookup(&persp_name.name) {
                Some(TopSymbol::Perspective(p)) => p,
                Some(_) => {
                    self.diags.push(Diag::ty(
                        persp_name.span,
                        format!(
                            "locus `{}` serves `{}`, but `{}` is not a \
                             perspective contract",
                            decl.name.name, persp_name.name, persp_name.name
                        ),
                    ));
                    continue;
                }
                None => {
                    self.diags.push(Diag::ty(
                        persp_name.span,
                        format!(
                            "locus `{}` serves unknown perspective `{}`",
                            decl.name.name, persp_name.name
                        ),
                    ));
                    continue;
                }
            };
            for pm in &persp.methods {
                // `is_stable` is synthesized on every perspective
                // (from `stable_when`) — not a contract method the
                // impl must provide.
                if pm.name == "is_stable" {
                    continue;
                }
                let lm = locus.methods.iter().find(|lm| lm.name == pm.name);
                let lm = match lm {
                    Some(m) => m,
                    None => {
                        self.diags.push(Diag::ty(
                            persp_name.span,
                            format!(
                                "locus `{}` serves `{}` but is missing \
                                 contract method `{}`",
                                decl.name.name, persp_name.name, pm.name
                            ),
                        ));
                        continue;
                    }
                };
                if lm.params.len() != pm.params.len() {
                    self.diags.push(Diag::ty(
                        persp_name.span,
                        format!(
                            "locus `{}` method `{}` has {} arg(s) but \
                             perspective `{}` requires {}",
                            decl.name.name,
                            pm.name,
                            lm.params.len(),
                            persp_name.name,
                            pm.params.len()
                        ),
                    ));
                    continue;
                }
                for (i, (lp, pp)) in
                    lm.params.iter().zip(pm.params.iter()).enumerate()
                {
                    if !pp.assignable_from(lp) {
                        self.diags.push(Diag::ty(
                            persp_name.span,
                            format!(
                                "locus `{}` method `{}` arg #{}: perspective \
                                 `{}` requires `{}`, locus has `{}`",
                                decl.name.name,
                                pm.name,
                                i,
                                persp_name.name,
                                pp.display(),
                                lp.display()
                            ),
                        ));
                    }
                }
                if !pm.ret.assignable_from(&lm.ret) {
                    self.diags.push(Diag::ty(
                        persp_name.span,
                        format!(
                            "locus `{}` method `{}` returns `{}` but \
                             perspective `{}` requires `{}`",
                            decl.name.name,
                            pm.name,
                            lm.ret.display(),
                            persp_name.name,
                            pm.ret.display()
                        ),
                    ));
                }
            }
            // Phase 2c: bus-surface conformance — the impl must
            // subscribe / publish every subject the contract declares.
            for want in &persp.bus_subscribes {
                if !locus.bus_subscribes.iter().any(|s| &s.subject == want) {
                    self.diags.push(Diag::ty(
                        persp_name.span,
                        format!(
                            "locus `{}` serves `{}` but does not subscribe \
                             the contract subject `{}`",
                            decl.name.name, persp_name.name, want
                        ),
                    ));
                }
            }
            for want in &persp.bus_publishes {
                if !locus.bus_publishes.iter().any(|s| &s.subject == want) {
                    self.diags.push(Diag::ty(
                        persp_name.span,
                        format!(
                            "locus `{}` serves `{}` but does not publish \
                             the contract subject `{}`",
                            decl.name.name, persp_name.name, want
                        ),
                    ));
                }
            }
        }
    }

    /// Phase 2b: typecheck `reperspective self.<field> as <Impl>;`.
    ///   - must be inside a locus method (there is a `self`);
    ///   - `field` must be a `perspective(P)`-typed param of the
    ///     current locus (the owner holds the slot);
    ///   - `Impl` must be a locus that `serves P`.
    fn check_reperspective(
        &mut self,
        field: &Ident,
        impl_name: &Ident,
        span: hale_syntax::Span,
    ) {
        let Some(locus) = self.current_locus else {
            self.diags.push(Diag::ty(
                span,
                "`reperspective` is only valid inside a locus method — it \
                 re-points a perspective slot the enclosing locus owns"
                    .to_string(),
            ));
            return;
        };
        // GH #724: `reperspective self.f as lib::Impl` where this
        // bundle holds no `lib`. Both ends — the impl and the field's
        // contract — sit behind an alias we cannot see, so nothing
        // here is decidable. Every CLI path merges the imported seed
        // before checking, so this only ever skips in a tool holding
        // one seed (the LSP).
        if self.unresolved_alias_path(&impl_name.name) {
            return;
        }
        // Resolve the field's declared perspective contract.
        let field_ty =
            locus.params.iter().find(|p| p.name == field.name).map(|p| &p.ty);
        let persp = match field_ty {
            Some(Ty::Named(n))
                if matches!(
                    self.top.lookup(n),
                    Some(TopSymbol::Perspective(_))
                ) =>
            {
                n.clone()
            }
            Some(_) => {
                self.diags.push(Diag::ty(
                    span,
                    format!(
                        "`reperspective self.{}`: field `{}` is not a \
                         `perspective(...)` handle",
                        field.name, field.name
                    ),
                ));
                return;
            }
            None => {
                self.diags.push(Diag::ty(
                    span,
                    format!(
                        "`reperspective self.{}`: locus `{}` has no field \
                         `{}`",
                        field.name, locus.name, field.name
                    ),
                ));
                return;
            }
        };
        // Phase 2c gate: swapping a perspective that has a BUS surface
        // needs the async mailbox-swap runtime (re-point the current
        // impl's subscriptions), which is not built yet. Reject with a
        // clear message rather than silently leaving the old impl's
        // subscriptions live.
        // Phase 2c-runtime: a bus-backed perspective now swaps its
        // subscriptions too (the codegen tombstones the current
        // impl's registrations on the shared slot data and
        // re-registers the new impl's handlers). Perspective impls
        // are designated via a field default, never a `placement`
        // entry, so they are always cooperative — the re-registration
        // routes through the global queue, no mailbox hand-off. No
        // gate needed.
        // The new impl must be a locus that serves this perspective.
        match self.top.lookup(&impl_name.name) {
            Some(TopSymbol::Locus(impl_info)) => {
                if !impl_info.serves.iter().any(|s| s == &persp) {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "`reperspective self.{} as {}`: locus `{}` does \
                             not `serve {}` — only an impl of the same \
                             perspective can be swapped in",
                            field.name, impl_name.name, impl_name.name, persp
                        ),
                    ));
                    return;
                }
                // Phase 3 migration: the swap preserves state by
                // keeping the slot's data pointer and re-pointing only
                // the vtable (the note's layout-identity "zero
                // migration" — code and state are already separate).
                // That is layout-safe only if every impl of the
                // perspective shares the same footprint, so the new
                // impl's methods interpret the retained state
                // correctly. A footprint change is the `migrate` case
                // — deferred; reject it with an actionable message
                // rather than silently reinterpreting bytes.
                for (other_name, sym) in &self.top.symbols {
                    if other_name == &impl_name.name {
                        continue;
                    }
                    if let TopSymbol::Locus(other) = sym {
                        if other.serves.iter().any(|s| s == &persp)
                            && !footprints_match(&impl_info.params, &other.params)
                        {
                            self.diags.push(Diag::ty(
                                span,
                                format!(
                                    "`reperspective self.{} as {}`: impl `{}` has \
                                     a different footprint than `{}` (also serving \
                                     `{}`). A state-preserving swap requires all \
                                     impls of a perspective to share the same \
                                     params; a footprint change needs `migrate`, \
                                     which isn't supported yet.",
                                    field.name, impl_name.name, impl_name.name,
                                    other_name, persp
                                ),
                            ));
                            return;
                        }
                    }
                }
            }
            _ => {
                self.diags.push(Diag::ty(
                    span,
                    format!(
                        "`reperspective self.{} as {}`: `{}` is not a locus",
                        field.name, impl_name.name, impl_name.name
                    ),
                ));
            }
        }
    }

    fn check_structural_impl(
        &self,
        locus_name: &str,
        iface_name: &str,
    ) -> Result<(), String> {
        let iface = match self.top.lookup(iface_name) {
            Some(TopSymbol::Interface(i)) => i,
            _ => return Ok(()),
        };
        let locus = match self.top.lookup(locus_name) {
            Some(TopSymbol::Locus(l)) => l,
            _ => {
                return Err(format!(
                    "type `{}` cannot satisfy interface `{}` — only loci satisfy interfaces",
                    locus_name, iface_name
                ));
            }
        };
        for im in &iface.methods {
            let lm = locus.methods.iter().find(|lm| lm.name == im.name);
            let lm = match lm {
                Some(m) => m,
                None => {
                    return Err(format!(
                        "locus `{}` does not satisfy interface `{}`: missing method `{}`",
                        locus_name, iface_name, im.name
                    ));
                }
            };
            if lm.params.len() != im.params.len() {
                return Err(format!(
                    "locus `{}` method `{}` arity does not match interface `{}`: expected {} arg(s), locus has {}",
                    locus_name,
                    im.name,
                    iface_name,
                    im.params.len(),
                    lm.params.len()
                ));
            }
            for (i, (lp, ip)) in
                lm.params.iter().zip(im.params.iter()).enumerate()
            {
                let want = &ip.1;
                if !want.assignable_from(lp) {
                    return Err(format!(
                        "locus `{}` method `{}` arg #{} type mismatch: interface `{}` requires `{}`, locus has `{}`",
                        locus_name,
                        im.name,
                        i,
                        iface_name,
                        want.display(),
                        lp.display()
                    ));
                }
            }
            if !im.ret.assignable_from(&lm.ret) {
                return Err(format!(
                    "locus `{}` method `{}` return type mismatch: interface `{}` requires `{}`, locus returns `{}`",
                    locus_name,
                    im.name,
                    iface_name,
                    im.ret.display(),
                    lm.ret.display()
                ));
            }
            // GH #732: an infallible method satisfies a fallible
            // interface method; a fallible one never satisfies an
            // infallible one, and the error types must be the same
            // type (no subtyping of error payloads).
            let clash = match (&im.fallible, &lm.fallible) {
                (None, Some(_)) => Some("is fallible where the interface's is not"),
                (Some(ie), Some(le)) if ie != le => {
                    Some("declares a different error type")
                }
                _ => None,
            };
            if let Some(why) = clash {
                let iface_sig = sig_text(
                    &im.name,
                    im.params.iter().map(|(_, t)| t),
                    &im.ret,
                    im.fallible.as_ref(),
                );
                let locus_sig =
                    sig_text(&lm.name, lm.params.iter(), &lm.ret, lm.fallible.as_ref());
                return Err(format!(
                    "locus `{}` method `{}` {}: interface `{}` declares `{}`, locus declares `{}`",
                    locus_name, im.name, why, iface_name, iface_sig, locus_sig
                ));
            }
        }
        Ok(())
    }

    /// GH #436: `@sealed` — a sealed locus's `params` are reachable
    /// only from inside its own methods.
    ///
    /// The rule is about the *reader*, not the receiver syntax: what
    /// matters is whether the enclosing locus IS the sealed one. A
    /// parent holding `s: Signer` reads `self.s.key` with receiver
    /// type `Signer` while `current_locus` is `Gateway`, and that is
    /// the read this forbids. `self.key` inside `Signer` has the same
    /// receiver type with `current_locus == Signer`, and is fine.
    ///
    /// Only `params` are sealed. Capacity slots and methods are
    /// untouched — sealing confines state, it does not make a locus
    /// uncallable, which is the entire point.
    fn check_sealed_access(
        &mut self,
        rt: &Ty,
        name: &Ident,
        span: Span,
        access: SealedAccess,
    ) {
        let Ty::Named(locus_name) = rt else { return };
        let Some(TopSymbol::Locus(li)) = self.top.symbols.get(locus_name)
        else {
            return;
        };
        if !li.sealed {
            return;
        }
        // Inside the sealed locus itself: every read is legal.
        if self.current_locus.map_or(false, |cur| cur.name == li.name) {
            return;
        }
        // Only `params` are confined; a slot or method name reaching
        // here is not a state read.
        if !li.params.iter().any(|p| p.name == name.name) {
            return;
        }
        // Render the spelling the author wrote. A stdlib locus is
        // declared under a mangled name (`__StdSecretSigner`) that
        // appears nowhere in their program; they wrote
        // `std::secret::Signer`.
        let shown = hale_stdlib::PATH_RENAMES
            .iter()
            .find(|(_, m)| *m == li.name)
            .map(|(p, _)| p.join("::"))
            .unwrap_or_else(|| li.name.clone());
        let callable: Vec<&str> =
            li.methods.iter().map(|m| m.name.as_str()).collect();
        let hint = if callable.is_empty() {
            format!(
                "`{shown}` declares no methods, so its state is \
                 reachable only from inside it"
            )
        } else {
            format!("call one of its methods instead ({})", callable.join(", "))
        };
        let (verb, gerund) = match access {
            SealedAccess::Read => ("readable", "reads"),
            SealedAccess::Write => ("writable", "writes"),
        };
        self.diags.push(Diag::ty(
            span,
            format!(
                "`{shown}` is `@sealed`: its `params` are {verb} only \
                 from inside its own methods, and `{shown}.{}` {gerund} \
                 one from outside — {hint}",
                name.name
            ),
        ));
    }

    /// GH #759: a type name written in a position the checker reads
    /// SYNTACTICALLY (the `@form(hashmap)` / `@form(lru_cache)` cell
    /// slot, which needs the declaring struct to resolve
    /// `indexed_by`) may be a transparent alias. Answer with the
    /// name the alias expands to; anything that isn't an alias of a
    /// named type comes back unchanged, so the existing "not a
    /// struct" diagnostics still fire on their own terms.
    fn through_type_alias(&self, name: String) -> String {
        match self.known.alias_target(&name) {
            Some(Ty::Named(target)) => target.clone(),
            _ => name,
        }
    }

    fn field_ty(&self, ty: &Ty, name: &str) -> Option<Ty> {
        match ty {
            // Numeric tuple field access: `t.0`, `t.1`. Parser
            // stores the digit string as the field name, so we
            // recognize it as a usize index here.
            Ty::Tuple(parts) => {
                if let Ok(i) = name.parse::<usize>() {
                    if i < parts.len() {
                        return Some(parts[i].clone());
                    }
                }
                None
            }
            Ty::Named(n) => {
                // M3 stage 3 tranche 2: field reads on mangled
                // generic monomorph values (`b.value` where
                // b: Box_Int) resolve through the template with
                // the type args substituted.
                if self.top.lookup(n).is_none() {
                    let (template, bindings) =
                        self.resolve_generic_monomorph(n)?;
                    match template {
                        GenericTemplate::Type(td) => {
                            if let TypeDeclBody::Struct(tfields) = &td.body {
                                return tfields
                                    .iter()
                                    .find(|f| f.name.name == name)
                                    .map(|f| {
                                        substitute_generic_ty(
                                            &f.ty,
                                            &bindings,
                                            self.known,
                                        )
                                    });
                            }
                            return None;
                        }
                        // GH #911 B5: `c.cap` where
                        // `c: Cache<Int, String>`. A locus's params
                        // are its fields, and the monomorph's are
                        // the template's with the arguments
                        // substituted — the struct rule above, for
                        // the locus half. Codegen reads the field
                        // (the synthesized locus goes through the
                        // ordinary locus passes); the checker
                        // answered "no field `cap` on
                        // `Cache_Int_String`" and refused a program
                        // that builds and runs.
                        GenericTemplate::Locus(ld) => {
                            for m in &ld.members {
                                let LocusMember::Params(pb) = m else {
                                    continue;
                                };
                                for p in &pb.params {
                                    if p.name.name != name {
                                        continue;
                                    }
                                    // A param with no declared type
                                    // takes it from its default, and
                                    // that inference does not run
                                    // here — stay permissive rather
                                    // than invent one.
                                    return Some(match &p.ty {
                                        Some(te) => substitute_generic_ty(
                                            te,
                                            &bindings,
                                            self.known,
                                        ),
                                        None => Ty::Unknown,
                                    });
                                }
                            }
                            return None;
                        }
                    }
                }
                match self.top.lookup(n)? {
                TopSymbol::Type(info) => match &info.kind {
                    TypeKind::Struct(fields) => fields
                        .iter()
                        .find(|f| f.name == name)
                        .map(|f| f.ty.clone()),
                    TypeKind::Alias(t) => self.field_ty(t, name),
                    TypeKind::Enum(_) => None,
                },
                TopSymbol::Locus(info) => {
                    if name == "children" {
                        return Some(match &info.accept_param {
                            Some((_, t)) => Ty::Array(Box::new(t.clone()), None),
                            None => Ty::Array(Box::new(Ty::Unknown), None),
                        });
                    }
                    if name == "k_max" {
                        // F.1: k_max = B / [(1-phi)c + phi*sigma].
                        // Fractional in general; Float regardless of
                        // whether B/c/sigma are Int (the divisor is
                        // a phi-weighted blend).
                        return Some(Ty::Prim(PrimType::Float));
                    }
                    // v1.x-VIOLATE (F.27): synthetic Bool flag
                    // readable from any locus method body. True
                    // while the locus is winding down after
                    // `violate`; canonical use is to gate
                    // downstream sends after escalation. Backed
                    // by `__drain_requested` at codegen.
                    if name == "draining" {
                        return Some(Ty::Prim(PrimType::Bool));
                    }
                    if let Some(p) = info.params.iter().find(|p| p.name == name) {
                        return Some(p.ty.clone());
                    }
                    info.methods
                        .iter()
                        .find(|m| m.name == name)
                        .map(method_to_fn_ty)
                }
                TopSymbol::Perspective(info) => {
                    if let Some(p) = info.params.iter().find(|p| p.name == name) {
                        return Some(p.ty.clone());
                    }
                    info.methods
                        .iter()
                        .find(|m| m.name == name)
                        .map(method_to_fn_ty)
                }
                // 2026-05-16 — method lookup on an interface-typed
                // receiver. Resolves `obj.handle(req)` when `obj`
                // has interface type, so call-site typecheck sees
                // the method's signature instead of "no field".
                // Codegen already routes the call through the fat
                // pointer's vtable (lower_method_call's
                // CodegenTy::Interface arm).
                TopSymbol::Interface(info) => {
                    info.methods.iter().find(|m| m.name == name).map(|m| {
                        Ty::Function {
                            params: m.params.iter().map(|(_, t)| t.clone()).collect(),
                            ret: Box::new(m.ret.clone()),
                        }
                    })
                }
                _ => None,
            }
            }
            // `get(i) -> T fallible(IndexError)` on the two
            // type-level collections.
            //
            // Element chains desugar — before typecheck, so with no
            // type to dispatch on — into a loop that fetches through
            // the source's `get`. A `@form(vec)` is a locus and has
            // that method; a fixed array and a `bounded[T; N]` are
            // types, whose operations are grammar intrinsics
            // (`at(f, i)`, `count(f)`), so the desugared loop hit
            // "no field `get`" and chains simply could not anchor on
            // them (downstream handoff: ~11 would-be sites in one
            // fleet, ~44 hand-rolled walks across it).
            //
            // This is the one method-position operation on these
            // types, and it exists so the chain source protocol is
            // uniform across locus-form and type-level collections.
            // Same signature and semantics as `at`, which stays the
            // idiomatic spelling for a direct index.
            Ty::Array(elem, _) | Ty::Bounded(elem, _)
                if name == "get" =>
            {
                Some(Ty::Function {
                    params: vec![Ty::Prim(PrimType::Int)],
                    ret: Box::new(Ty::Fallible {
                        success: elem.clone(),
                        payload: Box::new(Ty::Named(
                            "IndexError".into(),
                        )),
                    }),
                })
            }
            Ty::Unknown => Some(Ty::Unknown),
            _ => None,
        }
    }

    /// GH #892: does the program's OWN `fn NAME` answer this call,
    /// ahead of the `bounded[T; N]` intrinsic of the same name?
    ///
    /// `count` / `clear` / `truncate` / `push` / `at` / `set` are
    /// intrinsics only where the first argument IS a bounded
    /// receiver, which is why they are absent from the parser's
    /// `BUILTIN_CALL_FORMS` and a free `fn` of any of those names is
    /// legal (`dna/tests/books_slice_test.hl` declares `fn count(app,
    /// kind, entity, needle)` and calls it). On a bounded argument,
    /// though, the intrinsic took the call from the declaration in
    /// BOTH layers and said nothing: `fn count(xs: bounded[Int; 8])
    /// -> Int` beside `count(w.samples)` typed as the intrinsic here
    /// and ran the intrinsic there, and only because both answer
    /// `Int` did the program check at all.
    ///
    /// The rule is lexical scope: **a declaration whose first
    /// parameter is the receiver's own `bounded[T; N]` answers the
    /// call**, at any arity that declaration accepts. Dispatch is
    /// type-directed, so the shadow is decidable at the call site —
    /// which is what makes this resolvable in the program's favour
    /// where GH #880's flat-namespace names were not.
    ///
    /// Element type and capacity are part of the match, because the
    /// Hale-source standard library is merged into this same global
    /// fn namespace and calls the intrinsics on buffers of its own
    /// (`bounded[String; 8]`, `bounded[Float; 32]`,
    /// `bounded[Int; 33]`). A name-only shadow would retarget the
    /// LIBRARY's calls at the user's declaration — the same capture
    /// that made `print` a claimed name. Matching the receiver type
    /// holds the shadow to the calls the author's own type reaches.
    ///
    /// Codegen holds the identical rule
    /// (`user_fn_shadows_bounded_intrinsic` in
    /// `crates/hale-codegen/src/form/bounded.rs`).
    fn user_fn_shadows_bounded_intrinsic(
        &self,
        name: &str,
        argc: usize,
        recv_elem: &Ty,
        recv_cap: u64,
    ) -> bool {
        let Some(TopSymbol::Fn(sig)) = self.top.lookup(name) else {
            return false;
        };
        let Some((_, Ty::Bounded(param_elem, param_cap))) =
            sig.params.first()
        else {
            return false;
        };
        param_elem.as_ref() == recv_elem
            && *param_cap == recv_cap
            && argc >= sig.required_params()
            && argc <= sig.params.len()
    }

    /// A bare identifier in expression position.
    ///
    /// GH #721: an identifier that binds NOTHING used to type as
    /// `Ty::Unknown` in silence, so `let total = 1; println("" +
    /// totl);` passed `hale check` and `hale verify` and then died in
    /// `hale build` as `unknown identifier` with no source location —
    /// the one place a located message costs nothing to produce. The
    /// checker holds codegen's rule now, whenever the program it was
    /// handed is WHOLE (`strict_idents`): every import is resolved, so
    /// a name nothing binds is a typo, not a sibling file's const. One
    /// file of a multi-file seed, a snippet or a harness's partial
    /// program keeps the permissive `Unknown` — the corpus has seeds
    /// whose every file reads a const another file declares.
    ///
    /// `report_unknown` is false at a CALL's callee, where F.18's own
    /// diagnostic — which names fn-pointer bindings and generic fns,
    /// the things a callee may also be — already covers the same
    /// mistake; reporting both would print two messages for one typo.
    fn check_ident_expr(&mut self, id: &Ident, report_unknown: bool) -> Ty {
        if let Some(s) = self.locals.lookup(&id.name) {
            return s.ty.clone();
        }
        let Some(sym) = self.top.lookup(&id.name) else {
            if report_unknown && self.strict_idents {
                let hint = self
                    .closest_name_in_scope(&id.name)
                    .map(|h| format!(" — did you mean `{}`?", h))
                    .unwrap_or_default();
                self.diags.push(Diag::ty(
                    id.span,
                    format!(
                        "unknown identifier `{}`: no binding, param, \
                         const or declaration with that name is in \
                         scope{}",
                        id.name, hint
                    ),
                ));
            }
            return Ty::Unknown;
        };
        match sym {
            TopSymbol::Const(c) => c.ty.clone(),
            TopSymbol::Fn(sig) => Ty::Function {
                params: sig.params.iter().map(|(_, t)| t.clone()).collect(),
                ret: Box::new(sig.ret.clone()),
            },
            // Locus / Type / Perspective / Interface
            // names used in expression position resolve
            // to the type (struct-literal, call site,
            // or interface-typed binding).
            TopSymbol::Locus(_)
            | TopSymbol::Type(_)
            | TopSymbol::Perspective(_)
            | TopSymbol::Interface(_) => Ty::Named(id.name.clone()),
            // Topics aren't values — they only address
            // a bus channel. They appear legally only on
            // the left of `<-` (handled in check_send,
            // before check_expr ever sees the subject).
            // Anywhere else is an error.
            TopSymbol::Topic(_) => {
                self.diags.push(Diag::ty(
                    id.span,
                    format!(
                        "topic `{}` is not a value; use `{} <- expr` \
                         to publish on it",
                        id.name, id.name
                    ),
                ));
                Ty::Unknown
            }
            TopSymbol::RingLayout(_) => {
                self.diags.push(Diag::ty(
                    id.span,
                    format!(
                        "ring_layout `{}` is not a value; reference it \
                         in a `shm_ring(..., layout: {})` binding",
                        id.name, id.name
                    ),
                ));
                Ty::Unknown
            }
        }
    }

    /// Nearest spelling to `name` among the things a bare identifier
    /// could have meant here: the locals in scope first (the typo is
    /// nearly always a local), then the program's top-level names.
    fn closest_name_in_scope(&self, name: &str) -> Option<String> {
        let mut locals: Vec<&str> = Vec::new();
        for frame in self.locals.frames.iter() {
            locals.extend(frame.keys().map(|k| k.as_str()));
        }
        if let Some(hit) = closest_bare_name(name, &locals) {
            return Some(hit.to_string());
        }
        let tops: Vec<&str> =
            self.top.symbols.keys().map(|k| k.as_str()).collect();
        closest_bare_name(name, &tops).map(|h| h.to_string())
    }

    /// GH #877: a BARE type name in an annotation that names no
    /// declaration.
    ///
    /// `resolve_type_expr` maps an unresolvable single-segment name
    /// to `Ty::Unknown`, which is permissive everywhere — so `fn
    /// helper() -> int` (the lowercase spelling of `Int`) passed
    /// `hale check` with `ok: 1 file(s) typechecked` and then died in
    /// codegen as `unknown type name 'int' in signature`: late, from
    /// another layer, and with no source location. Same frontier as
    /// GH #803 / #833, opposite answer, because the two names are not
    /// the same case.
    ///
    /// A QUALIFIED name keeps its tolerance and is not touched here.
    /// `lib::Thing` resolves only when the bundle carries the build's
    /// import renames, so a tool holding one seed WITHOUT its imports
    /// (the LSP's per-directory bundle) must not squiggle it. A bare
    /// name has no such escape: the only thing that can declare it is
    /// a declaration in the bundle.
    ///
    /// Gated on `strict_idents` for the reason the bare-IDENTIFIER
    /// rule is: one file of a multi-file seed, checked alone, reads
    /// declarations its siblings make, and a type is no different
    /// from a `const` there. `hale check <dir>` and every build path
    /// hold the whole program and hold the rule.
    fn check_type_annotation(&mut self, te: &TypeExpr) {
        // GH #911 B3 (#907): the generic-argument vocabulary is a
        // property of the type expression, not of how much of the
        // program this bundle holds, so it is decided before the
        // strict-identifier gate below — see the function's own doc.
        self.check_generic_arg_vocabulary(te);
        // GH #911 B5: the argument COUNT is not a strictness — it is
        // decided by the declaration, which a single file of a
        // multi-file seed reads as well as the whole bundle does.
        self.check_generic_arity(te);
        if !self.strict_idents {
            return;
        }
        match te {
            TypeExpr::Named { path, generic_args, span } => {
                // A generic argument is an annotation in its own
                // right: `[Box<Strng>; 2]` is the same typo.
                for arg in generic_args {
                    self.check_type_annotation(arg);
                }
                if path.segments.len() != 1 {
                    // GH #803: the QUALIFIED twin of this rule, whose
                    // answer is the same in every position a path can
                    // stand in and so is written once, below.
                    self.check_qualified_path(path);
                    return;
                }
                let name = &path.segments[0].name;
                if self.type_name_is_declared(name) {
                    return;
                }
                let hint = self
                    .closest_type_name(name)
                    .map(|h| format!(" — did you mean `{}`?", h))
                    .unwrap_or_default();
                self.diags.push(Diag::ty(
                    *span,
                    format!(
                        "unknown type `{}`: no type, enum, locus, \
                         interface or alias with that name is \
                         declared{}",
                        name, hint
                    ),
                ));
            }
            TypeExpr::Projection { inner, .. } => {
                self.check_type_annotation(inner);
            }
            TypeExpr::Array { elem, .. } | TypeExpr::Bounded { elem, .. } => {
                self.check_type_annotation(elem);
            }
            TypeExpr::Tuple(parts, _) => {
                for p in parts {
                    self.check_type_annotation(p);
                }
            }
            TypeExpr::Function { params, ret, .. } => {
                for p in params {
                    self.check_type_annotation(p);
                }
                if let Some(r) = ret {
                    self.check_type_annotation(r);
                }
            }
            // A primitive is resolved by the parser. `perspective(P)`
            // names a contract, not a type expression's bare name —
            // its own resolution rules are #724's, unchanged.
            TypeExpr::Primitive(_, _) | TypeExpr::Perspective { .. } => {}
        }
    }

    /// GH #911 B3 (#907): a generic argument must be something
    /// codegen can name a monomorph for.
    ///
    /// `type Holder { b: Box<Bytes>; }` passed `hale check` and died
    /// at build, because minting `Box_Bytes` is codegen's business
    /// and the checker had no opinion about which arguments it can
    /// mint. Four of the five missing primitives got tokens
    /// ([`crate::ty::GENERIC_ARG_PRIMS`]); `Uint` has no storage
    /// representation to name at all, so it is refused HERE, at the
    /// instantiation's span, with the supported set named.
    ///
    /// Deliberately NOT gated on `strict_idents`, unlike its caller:
    /// this asks what a `PrimType` is, which the parser has already
    /// decided and no absent sibling seed can change. A tool holding
    /// one file of a multi-file seed is as entitled to the answer as
    /// the build is.
    ///
    /// Deliberately narrow, too — only a PRIMITIVE argument. Codegen
    /// also refuses an array / tuple / `bounded` / fn-type argument
    /// and a qualified path, but a qualified path is single-segment by
    /// the time the mangler sees it (the import renames collapse it),
    /// so a checker that refused what the mangler refuses would
    /// refuse programs `hale build` accepts — the GH #779 direction,
    /// which is the worse one. Those forms stay codegen's to report.
    fn check_generic_arg_vocabulary(&mut self, te: &TypeExpr) {
        match te {
            TypeExpr::Named { generic_args, .. } => {
                for arg in generic_args {
                    if let TypeExpr::Primitive(p, span) = arg {
                        if crate::ty::generic_arg_mangle_token(*p).is_none() {
                            self.diags.push(Diag::ty(
                                *span,
                                crate::ty::generic_arg_refusal(*p),
                            ));
                        }
                    }
                    // A nested instantiation carries its own
                    // arguments: `Box<Box<Uint>>` is the same refusal.
                    self.check_generic_arg_vocabulary(arg);
                }
            }
            TypeExpr::Projection { inner, .. } => {
                self.check_generic_arg_vocabulary(inner);
            }
            TypeExpr::Array { elem, .. } | TypeExpr::Bounded { elem, .. } => {
                self.check_generic_arg_vocabulary(elem);
            }
            TypeExpr::Tuple(parts, _) => {
                for p in parts {
                    self.check_generic_arg_vocabulary(p);
                }
            }
            TypeExpr::Function { params, ret, .. } => {
                for p in params {
                    self.check_generic_arg_vocabulary(p);
                }
                if let Some(r) = ret {
                    self.check_generic_arg_vocabulary(r);
                }
            }
            // A primitive that is not itself a generic argument is
            // whatever its own position allows; `perspective(P)` names
            // a contract and takes no type arguments.
            TypeExpr::Primitive(_, _) | TypeExpr::Perspective { .. } => {}
        }
    }

    /// GH #877: does this bare name end at something a type
    /// annotation may spell?
    ///
    /// The answer must be at least as permissive as codegen's, or the
    /// rule refuses programs `hale build` accepts. The table it
    /// resolves against (`known`) is rebuilt from the top scope, so
    /// it carries user loci / types / enums / perspectives, every
    /// alias target, and the whole Hale-source stdlib surface
    /// (GH #470) — but not interfaces (registered as their own
    /// symbol), not the compiler-synthesized types, and not the
    /// generic parameters in scope. Each of those is asked for
    /// separately below.
    fn type_name_is_declared(&self, name: &str) -> bool {
        if self.known.contains_key(name)
            || self.known.alias_target(name).is_some()
            || SYNTHESIZED_TYPE_NAMES.contains(&name)
            || self.generic_params.iter().any(|g| g == name)
            || self.generic_types.contains_key(name)
        {
            return true;
        }
        if matches!(
            self.top.lookup(name),
            Some(
                TopSymbol::Locus(_)
                    | TopSymbol::Type(_)
                    | TopSymbol::Perspective(_)
                    | TopSymbol::Interface(_)
                    | TopSymbol::Topic(_)
                    | TopSymbol::RingLayout(_)
            )
        ) {
            return true;
        }
        // `Box_Int` written out: the mangled monomorph name a
        // generic instantiation resolves to, which codegen
        // synthesizes from the template.
        //
        // GH #911 B5: generic TYPES only. A generic LOCUS's monomorph
        // name is not a spelling codegen accepts anywhere (see
        // `check_struct_literal`), so admitting it as an annotation
        // would admit a program the build refuses.
        self.resolve_generic_monomorph(name)
            .is_some_and(|(t, _)| t.is_type())
    }

    /// Nearest spelling to `name` among the things a type annotation
    /// could have meant: the primitives first — `int` for `Int` is
    /// the headline case — then the program's declared type names
    /// and the generic parameters in scope. Mangled stdlib symbols
    /// (`__StdHttpRouter`) are excluded: they are not spellable in
    /// source, so suggesting one would be advice that cannot be
    /// taken.
    fn closest_type_name(&self, name: &str) -> Option<String> {
        let mut cands: Vec<&str> =
            hale_syntax::parser::PRIMITIVE_TYPE_NAMES.to_vec();
        if let Some(hit) = closest_bare_name(name, &cands) {
            return Some(hit.to_string());
        }
        cands.clear();
        cands.extend(
            self.known
                .keys()
                .map(|k| k.as_str())
                .filter(|k| !k.starts_with("__")),
        );
        cands.extend(self.generic_params.iter().map(|g| g.as_str()));
        cands.extend(SYNTHESIZED_TYPE_NAMES.iter().copied());
        cands.extend(
            self.top
                .symbols
                .iter()
                .filter(|(_, s)| {
                    matches!(
                        s,
                        TopSymbol::Locus(_)
                            | TopSymbol::Type(_)
                            | TopSymbol::Perspective(_)
                            | TopSymbol::Interface(_)
                    )
                })
                .map(|(k, _)| k.as_str())
                .filter(|k| !k.starts_with("__")),
        );
        closest_bare_name(name, &cands).map(|h| h.to_string())
    }

    /// GH #803: the located refusal of a qualified path that resolves
    /// to nothing, at the position the author wrote it.
    ///
    /// The resolver is where the tolerance lives (`resolve_type_expr`
    /// maps an unknown path to `Ty::Unknown`), but it has no
    /// diagnostic sink and some forty call sites — PR #851's note —
    /// so the rule is emitted from the positions instead: every
    /// annotation (`check_type_annotation`), a call / const / enum
    /// variant (`Expr::Path`), and a struct or locus literal
    /// (`check_struct_literal`). A `bindings { }` topic needs no site
    /// of its own: `check_main_and_bindings` already refuses a topic
    /// nothing declares, qualified or not.
    /// GH #1028: the imported free fn a qualified path names, through
    /// the same table codegen resolves it with (`import_renames`:
    /// `["lib", "add3"]` -> the mangled symbol the library's seed
    /// declared). With the path as the author wrote it, unscoped
    /// (GH #746's `u$0` head reads `u`), for diagnostics.
    fn imported_fn(&self, path: &QualifiedName) -> Option<(String, FnSig)> {
        if path.segments.len() < 2 {
            return None;
        }
        let key: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
        let (_, mangled) = self
            .import_renames
            .iter()
            .find(|(k, _)| k.iter().map(|s| s.as_str()).eq(key.iter().copied()))?;
        match self.top.lookup(mangled) {
            Some(TopSymbol::Fn(sig)) => {
                let written: Vec<&str> = std::iter::once(unscoped_alias(key[0]))
                    .chain(key[1..].iter().copied())
                    .collect();
                Some((written.join("::"), sig.clone()))
            }
            _ => None,
        }
    }

    fn check_qualified_path(&mut self, path: &QualifiedName) {
        if let Some(msg) = self.unresolved_qualified(path) {
            self.diags.push(Diag::ty(path.span, msg));
        }
    }

    /// GH #803 (GH #911 B2): does this qualified path name something
    /// no seed in the program declares? If so, what to say about it.
    ///
    /// The bare-TYPE rule above (#877) and the bare-IDENTIFIER rule
    /// (#721) end at the same reasoning, and this is its last
    /// spelling: in a program every import of which is resolved, a
    /// name nothing declares is a typo, and a typo deserves a span.
    /// `zz::f()` in a seed that imports nothing as `zz` passed `hale
    /// check` and `hale verify` and then died in codegen as `path
    /// call zz::f in expression position` — no location, a different
    /// layer, and after the gate had said yes.
    ///
    /// Two answers, because the author made one of two mistakes:
    ///
    /// - the HEAD names nothing at all — not `std::`, not an import
    ///   this build resolved, not a declaration of this program.
    ///   Nothing can ever answer it.
    /// - the head IS answered by an import and the name behind it is
    ///   not one that library declares (`b::Greeting` where `b`
    ///   provides `Greeting` under some other spelling, or not at
    ///   all). PR #819's `import_library_key.rs` had to run every
    ///   such case through `build` precisely because `check` could
    ///   not see it.
    ///
    /// Permissive, each case for a reason:
    ///
    /// - `std::`, the bundled namespace no seed imports — the stdlib
    ///   tables answer a `std::` path and report their own typos.
    /// - an import this bundle never RESOLVED (GH #724: a consumer
    ///   holding one seed without its libraries — the LSP's
    ///   per-directory bundle). The declaration genuinely is not
    ///   here, which is why every other qualified reference is
    ///   tolerant in that situation too.
    /// - a head that names a declaration: `Color::Red`, a wire
    ///   struct's `Frame::seq`, a `type C2 = Color;` alias, a generic
    ///   parameter. None of those is an import at all.
    /// - one file of a multi-file seed (`strict_idents` off), whose
    ///   `import` line may live in a sibling — #721's boundary, and
    ///   the reason the flag rather than the CLI decides.
    ///
    /// Scoped per seed by GH #762 / #746 rather than here: a head
    /// another seed declares and this one does not is that rule's,
    /// reported by the CLI before the table can answer it.
    fn unresolved_qualified(&self, path: &QualifiedName) -> Option<String> {
        if !self.strict_idents || path.segments.len() < 2 {
            return None;
        }
        let head = path.segments[0].name.as_str();
        let name = path.segments[1].name.as_str();
        if head == "std"
            || self.unresolved_import_aliases.contains(head)
            || self.type_name_is_declared(head)
            || self.top.lookup(head).is_some()
        {
            return None;
        }
        // The rule must refuse nothing the build accepts, and codegen
        // still lowers two unprefixed paths itself.
        if UNPREFIXED_STDLIB_PATHS
            .iter()
            .any(|(ns, f)| *ns == head && *f == name)
        {
            return None;
        }
        // The names this build's imports registered under this head.
        // A row for the name being written means the path resolves
        // exactly as codegen resolves it, and there is nothing to say.
        let mut provided: Vec<&str> = Vec::new();
        for (key, _) in self.import_renames.iter() {
            if key.first().map(|s| s.as_str()) != Some(head) {
                continue;
            }
            match key.get(1) {
                Some(n) if n == name => return None,
                Some(n) => provided.push(n.as_str()),
                None => {}
            }
        }
        // GH #746 gives a CONTESTED alias a head of its own (`u` ->
        // `u$0`) in the table and in its seed's own references; the
        // author wrote `u`, so that is what the message says.
        let alias = unscoped_alias(head);
        if !provided.is_empty() {
            provided.sort_unstable();
            provided.dedup();
            // Internal `__` names are reachable but are not the
            // surface to advertise.
            provided.retain(|n| !n.starts_with("__"));
            let base = format!(
                "`{}::{}` is not declared by the library imported as `{}`",
                alias, name, alias
            );
            return Some(match nearest_qualified_segment(name, &provided) {
                Some(hit) => {
                    format!("{} — did you mean `{}::{}`?", base, alias, hit)
                }
                // No near spelling: what the library DOES provide is
                // the next most useful thing to say, which is the
                // shape codegen's twin message has for the same table
                // (`unknown_qualified_name` in codegen.rs).
                None if !provided.is_empty() => {
                    let extra = provided.len().saturating_sub(8);
                    let mut shown = provided[..provided.len().min(8)]
                        .join(", ");
                    if extra > 0 {
                        shown.push_str(&format!(", … ({} more)", extra));
                    }
                    format!("{}; `{}` provides: {}", base, alias, shown)
                }
                None => base,
            });
        }
        let written: Vec<&str> = std::iter::once(alias)
            .chain(path.segments[1..].iter().map(|s| s.name.as_str()))
            .collect();
        let written = written.join("::");
        // The stdlib lives under `std::`, and dropping the prefix is
        // the canonical typo (`env::args_count`). When the prefix
        // would resolve the path — in the surface table or in the
        // Hale-source stdlib — say that instead, which is the answer
        // codegen gives for the same mistake.
        let prefixed: Vec<&str> = std::iter::once("std")
            .chain(path.segments.iter().map(|s| s.name.as_str()))
            .collect();
        if crate::stdlib_surface::signature_for(&prefixed).is_some()
            || crate::stdlib_bodies::mangled_locus_name(&prefixed).is_some()
        {
            return Some(format!(
                "`{}` is unresolved — did you mean `std::{}`? The \
                 stdlib lives under the `std::` prefix",
                written, written
            ));
        }
        let hint = self
            .closest_path_head(head)
            .map(|h| format!(" — did you mean `{}`?", h))
            .unwrap_or_default();
        Some(format!(
            "`{}`: `{}` is not an import or a type of this seed{}",
            written, alias, hint
        ))
    }

    /// Nearest spelling to a qualified path's HEAD among the things a
    /// head can be: the aliases this build's imports registered
    /// first — a mistyped alias is the likely mistake — then the
    /// program's own type-like names. Primitives are not candidates
    /// (`Int::x` is not a path anyone means to write, and `zz` is
    /// three edits from `Int`), and mangled symbols are unspellable
    /// in source, so suggesting one would be advice that cannot be
    /// taken.
    fn closest_path_head(&self, head: &str) -> Option<String> {
        let mut aliases: Vec<&str> = self
            .import_renames
            .iter()
            .filter_map(|(key, _)| key.first())
            .map(|h| unscoped_alias(h.as_str()))
            .collect();
        aliases.sort_unstable();
        aliases.dedup();
        if let Some(hit) = nearest_qualified_segment(head, &aliases) {
            return Some(hit);
        }
        let mut types: Vec<&str> = self
            .known
            .keys()
            .map(|k| k.as_str())
            .filter(|k| !k.starts_with("__"))
            .collect();
        types.extend(
            self.top
                .symbols
                .iter()
                .filter(|(_, s)| {
                    matches!(
                        s,
                        TopSymbol::Locus(_)
                            | TopSymbol::Type(_)
                            | TopSymbol::Perspective(_)
                            | TopSymbol::Interface(_)
                    )
                })
                .map(|(k, _)| k.as_str())
                .filter(|k| !k.starts_with("__")),
        );
        types.sort_unstable();
        types.dedup();
        nearest_qualified_segment(head, &types)
    }

    fn check_expr(&mut self, expr: &Expr) -> Ty {
        match expr {
            Expr::Literal(lit, span) => {
                // GH #607: a Time literal is an instant, parsed here so
                // a malformed one is the author's error, not the
                // program's at runtime.
                if let Literal::Time(s) = lit {
                    if hale_syntax::time_literal::parse_iso8601_utc_ns(s).is_none() {
                        self.diags.push(Diag::ty(
                            *span,
                            format!(
                                "time literal `{s}` is not an ISO-8601 UTC instant \
                                 (`YYYY-MM-DDTHH:MM:SS[.fraction]Z`; an offset such as \
                                 `+01:00` is rejected, UTC only)"
                            ),
                        ));
                    }
                }
                lit_ty(lit)
            }
            Expr::Ident(id) => self.check_ident_expr(id, true),
            Expr::Path(qn) => {
                // m47-followup: 2-segment path may be an enum
                // variant construction (`EnumName::VariantName`).
                // Resolve to the enum type so let-bindings,
                // tuple/array literals, and struct fields can
                // unify against the declared shape rather than
                // falling through to Unknown (which made `let x:
                // Color = Color::Red;` fail with `expected Color,
                // got ?`).
                if qn.segments.len() == 2 {
                    // GH #831: the head may be an alias of the enum
                    // (`type C2 = Color; C2::Red`) — a second
                    // spelling, so it constructs the same variant.
                    let spelled = &qn.segments[0].name;
                    let resolved = construction_target(self.top, spelled);
                    let enum_name = resolved.as_ref().unwrap_or(spelled);
                    let variant_name = &qn.segments[1].name;
                    if let Some(TopSymbol::Type(TypeInfo {
                        kind: TypeKind::Enum(variants),
                        ..
                    })) = self.top.symbols.get(enum_name)
                    {
                        if variants.iter().any(|v| v.name == *variant_name) {
                            return Ty::Named(enum_name.clone());
                        }
                    }
                }
                // GH #1028: an imported seed's free fn, `alias::f`. It
                // is typed like a bare fn, so a call through the path
                // gets the arity bounds and the argument types a
                // same-seed call gets (the call arm reads this type),
                // and the value it returns has a type downstream.
                if let Some((_, sig)) = self.imported_fn(qn) {
                    return Ty::Function {
                        params: sig.params.iter().map(|(_, t)| t.clone()).collect(),
                        ret: Box::new(sig.ret.clone()),
                    };
                }
                // GH #803: nothing above answered the path, and in a
                // whole program nothing else will. This is the CALL
                // position too — a `Expr::Call` with a path callee
                // checks the callee through here — and the const and
                // enum-variant positions.
                self.check_qualified_path(qn);
                Ty::Unknown
            }
            Expr::Path2 { .. } => Ty::Unknown,
            Expr::KwSelf(span) => {
                if self.current_locus.is_none() {
                    self.diags.push(Diag::ty(
                        *span,
                        "`self` used outside a locus body".to_string(),
                    ));
                }
                self.self_ty()
            }
            Expr::Binary { op, left, right, span } => {
                let lt = self.check_expr(left);
                let rt = self.check_expr(right);
                self.binop_ty(*op, &lt, &rt, *span)
            }
            Expr::Unary { op, operand, .. } => {
                let t = self.check_expr(operand);
                match op {
                    UnaryOp::Neg | UnaryOp::BitNot => t,
                    UnaryOp::Not => Ty::Prim(PrimType::Bool),
                }
            }
            Expr::Call { callee, args, .. } => {
                // WASM plan — stdlib target-gating. Under `target wasm`
                // the browser sandbox has no syscalls, so reject a
                // POSIX-only `std::` call at compile time with guidance
                // (rather than letting it become an inert host import).
                // Typecheck runs on USER code before the stdlib is merged
                // in codegen, so this never flags the stdlib's own
                // internals — only the program's calls.
                if self.wasm_target {
                    if let Expr::Path(qn) = callee.as_ref() {
                        let segs: Vec<&str> =
                            qn.segments.iter().map(|s| s.name.as_str()).collect();
                        if let Some(why) = wasm_unavailable_stdlib(&segs) {
                            self.diags.push(Diag::ty(
                                qn.span,
                                format!(
                                    "`std::{}` is unavailable under `target wasm`: {}",
                                    segs[1..].join("::"),
                                    why
                                ),
                            ));
                        }
                    }
                }
                // Typecheck M3 stage 1 (2026-07-02): stdlib fn-name
                // validation. Within a TABLED namespace an unknown
                // name is an error with a did-you-mean; untabled
                // namespaces keep the permissive Unknown behavior,
                // so table incompleteness degrades to the status
                // quo, never to a false error.
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.first().map(|s| s.name.as_str())
                        == Some("std")
                    {
                        let segs: Vec<&str> = qn
                            .segments
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect();
                        // GH #470: a std:: path that renames to a
                        // REGISTERED Hale-source free fn checks
                        // against its real signature — full
                        // fidelity: arity, param types, nominal
                        // return, fallibility. The surface table
                        // below stays authoritative only for the
                        // Rust-implemented builtins it was built
                        // for. (Before this, `std::io::file::open`
                        // checked against the table's stale fd-era
                        // row — `-> Int` — so the DOCUMENTED File
                        // handle pattern could not typecheck.)
                        if let Some(mangled) =
                            crate::stdlib_bodies::mangled_locus_name(
                                &segs,
                            )
                        {
                            if let Some(TopSymbol::Fn(f)) =
                                self.top.lookup(mangled).cloned()
                            {
                                let min = f
                                    .min_params
                                    .unwrap_or(f.params.len());
                                if args.len() < min
                                    || args.len() > f.params.len()
                                {
                                    self.diags.push(Diag::ty(
                                        qn.span,
                                        format!(
                                            "`{}` takes {} argument{}, \
                                             got {}",
                                            segs.join("::"),
                                            f.params.len(),
                                            if f.params.len() == 1 {
                                                ""
                                            } else {
                                                "s"
                                            },
                                            args.len()
                                        ),
                                    ));
                                }
                                for (i, a) in args.iter().enumerate() {
                                    let got =
                                        self.check_expr_addressed(a);
                                    if let Some((_, want)) =
                                        f.params.get(i)
                                    {
                                        if !want.assignable_from(&got) {
                                            self.diags.push(Diag::ty(
                                                a.span(),
                                                format!(
                                                    "`{}` argument {}: \
                                                     expected `{}`, \
                                                     got `{}`",
                                                    segs.join("::"),
                                                    i + 1,
                                                    want.display(),
                                                    got.display()
                                                ),
                                            ));
                                        }
                                    }
                                }
                                return match &f.fallible {
                                    Some(payload) => Ty::Fallible {
                                        success: Box::new(
                                            f.ret.clone(),
                                        ),
                                        payload: Box::new(
                                            payload.clone(),
                                        ),
                                    },
                                    None => f.ret.clone(),
                                };
                            }
                        }
                        if let Some(msg) =
                            crate::stdlib_surface::unknown_fn_error(&segs)
                        {
                            self.diags.push(Diag::ty(qn.span, msg));
                        }
                        // M3 stage 2 (2026-07-02): signature
                        // enforcement — arity, arg types, and the
                        // REAL return type (killing the Unknown
                        // passthrough for tabled fns). Fallible rows
                        // return Ty::Fallible, so `or 0` on an
                        // Int-success call checks the substitute
                        // against Int and codegen's must-address
                        // rule gets a typecheck twin.
                        if let Some(sig) =
                            crate::stdlib_surface::signature_for(&segs)
                        {
                            // (bounded-intrinsic block is below —
                            // stdlib paths never collide with it.)
                            if args.len() != sig.params.len() {
                                self.diags.push(Diag::ty(
                                    qn.span,
                                    format!(
                                        "`{}` takes {} argument{}, got {}",
                                        sig.display_path(),
                                        sig.params.len(),
                                        if sig.params.len() == 1 {
                                            ""
                                        } else {
                                            "s"
                                        },
                                        args.len()
                                    ),
                                ));
                            }
                            for (i, a) in args.iter().enumerate() {
                                let got = self.check_expr_addressed(a);
                                if let Some(want) = sig.params.get(i) {
                                    if !want.accepts(&got) {
                                        self.diags.push(Diag::ty(
                                            a.span(),
                                            format!(
                                                "`{}` argument {}: expected \
                                                 `{}`, got `{}`",
                                                sig.display_path(),
                                                i + 1,
                                                want.to_ty().display(),
                                                got.display()
                                            ),
                                        ));
                                    }
                                }
                            }
                            return sig.ret_ty();
                        }
                    }
                }
                // GH #241 (audit items 1+3): bare-builtin arg
                // validation. println/print/to_string accept only
                // printable types (primitives + enums — structs
                // and loci have no rendering and previously died
                // as spanless codegen errors); abs/min/max accept
                // numerics (Int/Float/Duration/Decimal). Guarded
                // on the name NOT resolving to a user fn, so user
                // shadowing keeps its own signature.
                if let Expr::Ident(id) = callee.as_ref() {
                    let shadowed = self.top.lookup(&id.name).is_some()
                        || self.locals.lookup(&id.name).is_some();
                    if !shadowed {
                        match id.name.as_str() {
                            "println" | "print" | "to_string" => {
                                for a in args {
                                    self.warn_if_meant_an_fstring(a);
                                    let at = self.check_expr_addressed(a);
                                    if !self.ty_is_printable(&at) {
                                        self.diags.push(Diag::ty(
                                            a.span(),
                                            format!(
                                                "`{}` cannot render a value \
                                                 of type `{}` — {}",
                                                id.name,
                                                at.display(),
                                                self.why_unprintable(&at, 0)
                                                    .unwrap_or_else(|| {
                                                        "it is not printable"
                                                            .to_string()
                                                    })
                                            ),
                                        ));
                                    }
                                }
                            }
                            // GH #469 A3: `f"{x:spec}"` desugars to
                            // `__fmt(x, "spec")`. The spec grammar
                            // was already validated by the parser;
                            // what is checked here is the pairing of
                            // spec and VALUE, which the parser could
                            // not know.
                            hale_syntax::parser::FMT_BUILTIN => {
                                self.check_fmt_builtin(args, callee);
                                return Ty::Prim(PrimType::String);
                            }
                            "abs" | "min" | "max" => {
                                for a in args {
                                    let at = self.check_expr_addressed(a);
                                    let numeric = matches!(
                                        &at,
                                        Ty::Prim(
                                            PrimType::Int
                                                | PrimType::Float
                                                | PrimType::Duration
                                                | PrimType::Decimal
                                        ) | Ty::Unknown
                                    );
                                    if !numeric {
                                        self.diags.push(Diag::ty(
                                            a.span(),
                                            format!(
                                                "`{}` takes numeric operands (Int / Float / \
                                                 Duration / Decimal), got `{}`",
                                                id.name,
                                                at.display()
                                            ),
                                        ));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // bounded[T; N] intrinsics (2026-07-02):
                // push/at/count/clear over a bounded-typed first
                // arg. Probed speculatively — when arg0 isn't
                // bounded, its diags are rolled back and the call
                // falls through to the normal paths.
                //
                // GH #892: it also falls through when the program
                // DECLARES the name over this receiver's own bounded
                // type — lexical scope wins, and codegen's arms hold
                // the same rule, so both layers resolve the call to
                // the same fn.
                if let Expr::Ident(id) = callee.as_ref() {
                    if matches!(
                        id.name.as_str(),
                        "push" | "at" | "set" | "count" | "clear"
                            | "truncate"
                    ) && !args.is_empty()
                    {
                        let mark = self.diags.len();
                        let recv_ty = self.check_expr(&args[0]);
                        let shadowed = match &recv_ty {
                            Ty::Bounded(elem, cap) => self
                                .user_fn_shadows_bounded_intrinsic(
                                    id.name.as_str(),
                                    args.len(),
                                    elem.as_ref(),
                                    *cap,
                                ),
                            _ => false,
                        };
                        if let (false, Ty::Bounded(elem, _cap)) =
                            (shadowed, recv_ty)
                        {
                            let want_args = match id.name.as_str() {
                                "push" | "at" | "truncate" => 2,
                                "set" => 3,
                                _ => 1,
                            };
                            if args.len() != want_args {
                                self.diags.push(Diag::ty(
                                    callee.span(),
                                    format!(
                                        "`{}(bounded, ...)` takes {} \
                                         argument{}, got {}",
                                        id.name,
                                        want_args,
                                        if want_args == 1 { "" } else { "s" },
                                        args.len()
                                    ),
                                ));
                            }
                            if id.name == "set" {
                                if let Some(i) = args.get(1) {
                                    let it =
                                        self.check_expr_addressed(i);
                                    if !Ty::Prim(PrimType::Int)
                                        .assignable_from(&it)
                                    {
                                        self.diags.push(Diag::ty(
                                            i.span(),
                                            format!(
                                                "set: index must be \
                                                 Int, got `{}`",
                                                it.display()
                                            ),
                                        ));
                                    }
                                }
                                if let Some(x) = args.get(2) {
                                    let xt =
                                        self.check_expr_addressed(x);
                                    let widen_ok = matches!(
                                        (elem.as_ref(), &xt),
                                        (
                                            Ty::Prim(PrimType::Float),
                                            Ty::Prim(PrimType::Int)
                                        )
                                    );
                                    if !widen_ok
                                        && !elem.assignable_from(&xt)
                                    {
                                        self.diags.push(Diag::ty(
                                            x.span(),
                                            format!(
                                                "set: element type `{}` \
                                                 does not match bounded \
                                                 element `{}`",
                                                xt.display(),
                                                elem.display()
                                            ),
                                        ));
                                    }
                                }
                                return Ty::Fallible {
                                    success: Box::new(Ty::Unit),
                                    payload: Box::new(Ty::Named(
                                        "IndexError".into(),
                                    )),
                                };
                            }
                            if id.name == "truncate" {
                                if let Some(n) = args.get(1) {
                                    let nt =
                                        self.check_expr_addressed(n);
                                    if !Ty::Prim(PrimType::Int)
                                        .assignable_from(&nt)
                                    {
                                        self.diags.push(Diag::ty(
                                            n.span(),
                                            format!(
                                                "truncate: n must be \
                                                 Int, got `{}`",
                                                nt.display()
                                            ),
                                        ));
                                    }
                                }
                                return Ty::Prim(PrimType::Int);
                            }
                            match id.name.as_str() {
                                "push" => {
                                    if let Some(x) = args.get(1) {
                                        let xt =
                                            self.check_expr_addressed(x);
                                        let widen_ok = matches!(
                                            (elem.as_ref(), &xt),
                                            (
                                                Ty::Prim(PrimType::Float),
                                                Ty::Prim(PrimType::Int)
                                            )
                                        );
                                        if !widen_ok
                                            && !elem.assignable_from(&xt)
                                        {
                                            self.diags.push(Diag::ty(
                                                x.span(),
                                                format!(
                                                    "push: element type \
                                                     `{}` does not match \
                                                     bounded element `{}`",
                                                    xt.display(),
                                                    elem.display()
                                                ),
                                            ));
                                        }
                                    }
                                    return Ty::Fallible {
                                        success: Box::new(Ty::Unit),
                                        payload: Box::new(Ty::Named(
                                            "CapacityError".into(),
                                        )),
                                    };
                                }
                                "at" => {
                                    if let Some(i) = args.get(1) {
                                        let it =
                                            self.check_expr_addressed(i);
                                        if !Ty::Prim(PrimType::Int)
                                            .assignable_from(&it)
                                        {
                                            self.diags.push(Diag::ty(
                                                i.span(),
                                                format!(
                                                    "at: index must be \
                                                     Int, got `{}`",
                                                    it.display()
                                                ),
                                            ));
                                        }
                                    }
                                    return Ty::Fallible {
                                        success: elem,
                                        payload: Box::new(Ty::Named(
                                            "IndexError".into(),
                                        )),
                                    };
                                }
                                "count" => {
                                    return Ty::Prim(PrimType::Int);
                                }
                                _ => return Ty::Unit,
                            }
                        }
                        // Not bounded, or the program's own fn owns
                        // the name for this receiver type (GH #892):
                        // roll back the speculative diags and let the
                        // ordinary call paths resolve it.
                        self.diags.truncate(mark);
                    }
                }
                // M3 stage 3 (2026-07-02): generic fn call
                // validation — the Ty-level mirror of codegen's m62
                // inference, with spans. Checks: arity, every
                // generic param pinned, no conflicting bindings,
                // args vs substituted params; the call types as the
                // substituted return (fallible payloads
                // substituted too).
                if let Expr::Ident(id) = callee.as_ref() {
                    if let Some(template) =
                        self.generic_fns.get(id.name.as_str()).copied()
                    {
                        if args.len() != template.params.len() {
                            self.diags.push(Diag::ty(
                                callee.span(),
                                format!(
                                    "generic fn `{}` takes {} \
                                     argument{}, got {}",
                                    id.name,
                                    template.params.len(),
                                    if template.params.len() == 1 {
                                        ""
                                    } else {
                                        "s"
                                    },
                                    args.len()
                                ),
                            ));
                        }
                        let arg_tys: Vec<Ty> = args
                            .iter()
                            .map(|a| self.check_expr_addressed(a))
                            .collect();
                        let generic_names: std::collections::BTreeSet<
                            String,
                        > = template
                            .generics
                            .iter()
                            .map(|g| g.name.name.clone())
                            .collect();
                        let mut bindings: BTreeMap<String, Ty> =
                            BTreeMap::new();
                        for (p, at) in
                            template.params.iter().zip(arg_tys.iter())
                        {
                            if let Err((gname, was, now)) =
                                unify_generic_ty(
                                    &p.ty,
                                    at,
                                    &generic_names,
                                    &mut bindings,
                                )
                            {
                                self.diags.push(Diag::ty(
                                    callee.span(),
                                    format!(
                                        "generic fn `{}`: parameter \
                                         `{}` bound to both `{}` and \
                                         `{}` by this call's arguments",
                                        id.name,
                                        gname,
                                        was.display(),
                                        now.display()
                                    ),
                                ));
                            }
                        }
                        // Every generic must be pinned — UNLESS an
                        // arg typed Unknown could have pinned it
                        // (stay permissive where inference was).
                        let any_unknown_arg = arg_tys
                            .iter()
                            .any(|t| matches!(t, Ty::Unknown));
                        if !any_unknown_arg {
                            for g in &template.generics {
                                if !bindings.contains_key(&g.name.name)
                                {
                                    self.diags.push(Diag::ty(
                                        callee.span(),
                                        format!(
                                            "generic fn `{}`: cannot \
                                             infer `{}` from this call \
                                             — every generic param \
                                             must appear in an \
                                             argument position that \
                                             pins it",
                                            id.name, g.name.name
                                        ),
                                    ));
                                }
                            }
                        }
                        // Args vs substituted params.
                        for ((p, at), a) in template
                            .params
                            .iter()
                            .zip(arg_tys.iter())
                            .zip(args.iter())
                        {
                            let want = substitute_generic_ty(
                                &p.ty,
                                &bindings,
                                self.known,
                            );
                            if !want.assignable_from(at) {
                                self.diags.push(Diag::ty(
                                    a.span(),
                                    format!(
                                        "generic fn `{}` argument \
                                         `{}`: expected `{}`, got `{}`",
                                        id.name,
                                        p.name.name,
                                        want.display(),
                                        at.display()
                                    ),
                                ));
                            }
                        }
                        let ret = match &template.ret {
                            Some(te) => substitute_generic_ty(
                                te,
                                &bindings,
                                self.known,
                            ),
                            None => Ty::Unit,
                        };
                        if let Some(fe) = &template.fallible {
                            let payload = substitute_generic_ty(
                                fe,
                                &bindings,
                                self.known,
                            );
                            return Ty::Fallible {
                                success: Box::new(ret),
                                payload: Box::new(payload),
                            };
                        }
                        return ret;
                    }
                }
                // m47-payloads: enum-variant construction with
                // args. `EnumName::Variant(..)` resolves to the
                // enum's named type. We still walk the args to
                // surface their own type errors, but don't unify
                // them against declared field types yet — codegen
                // performs that strict check, and the typechecker
                // is permissive on Unknowns elsewhere.
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.len() == 2 {
                        // GH #831: through an alias of the enum too,
                        // exactly as the payload-less form above.
                        let spelled = &qn.segments[0].name;
                        let resolved = construction_target(self.top, spelled);
                        let enum_name = resolved.as_ref().unwrap_or(spelled);
                        let variant_name = &qn.segments[1].name;
                        if let Some(TopSymbol::Type(TypeInfo {
                            kind: TypeKind::Enum(variants),
                            ..
                        })) = self.top.symbols.get(enum_name)
                        {
                            if variants.iter().any(|v| v.name == *variant_name) {
                                let enum_name = enum_name.clone();
                                for a in args {
                                    let _ = self.check_expr(a);
                                }
                                return Ty::Named(enum_name);
                            }
                        }
                    }
                }
                // Proposal A′: validate repr-tagged field accessors. When
                // `T::member(...)` names a wire-layout struct (a type with
                // a `repr:`-tagged field), `member` must be one of its
                // fields (read: `T::field`) or `set_<field>` (write). This
                // catches a mistyped field at typecheck — otherwise the
                // accessor desugars to an unknown `std::bytes::*` call and
                // only fails at codegen. Valid accessors stay permissively
                // typed (the desugar lowers them to the pack primitives).
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.len() == 2 {
                        let tname = &qn.segments[0].name;
                        let member = &qn.segments[1].name;
                        if let Some(TopSymbol::Type(TypeInfo {
                            kind: TypeKind::Struct(fields),
                            ..
                        })) = self.top.symbols.get(tname)
                        {
                            let is_wire = fields.iter().any(|f| {
                                f.tag
                                    .as_deref()
                                    .and_then(|t| {
                                        hale_syntax::desugar::tag_value(t, "repr")
                                    })
                                    .is_some()
                            });
                            if is_wire {
                                let field = member
                                    .strip_prefix("set_")
                                    .unwrap_or(member);
                                let known =
                                    fields.iter().any(|f| f.name == *field);
                                if !known {
                                    let names: Vec<&str> = fields
                                        .iter()
                                        .map(|f| f.name.as_str())
                                        .collect();
                                    self.diags.push(Diag::ty(
                                        qn.span,
                                        format!(
                                            "`{}` has no wire field `{}`. \
                                             Accessors are `{}::<field>` \
                                             (read) and `{}::set_<field>` \
                                             (write); fields: {}",
                                            tname,
                                            field,
                                            tname,
                                            tname,
                                            names.join(", ")
                                        ),
                                    ));
                                }
                                for a in args {
                                    let _ = self.check_expr(a);
                                }
                                return Ty::Unknown;
                            }
                        }
                    }
                }
                // GH #721: a bare callee gets F.18's diagnostic below,
                // never the generic unknown-identifier one — two
                // messages for one typo, and the callee's is the more
                // informative of the two.
                let callee_ty = match callee.as_ref() {
                    Expr::Ident(id) => self.check_ident_expr(id, false),
                    other => self.check_expr(other),
                };
                // GH #583 (dna/FRICTION.md F.18): a bare callee that names
                // nothing — not a local (a fn-pointer binding), not a
                // top-level fn, not a generic fn, not a builtin — was
                // typed Unknown and sailed through `check`, to die in
                // `hale build` as "no free fn / generic fn / fn-pointer
                // binding with that name is in scope". An organization
                // whose gate is `check` applied a candidate that named a
                // router function nobody had written, and found out at
                // expression. The checker holds codegen's rule now, for a
                // WHOLE seed (`strict_callees`); a partial program — one
                // file of a seed, a snippet — legitimately calls what a
                // sibling defines and keeps the permissive Unknown. The
                // builtin list is the set of bare names codegen answers
                // itself.
                if let Expr::Ident(id) = callee.as_ref() {
                    let name = id.name.as_str();
                    let known = self.locals.lookup(name).is_some()
                        || self.top.lookup(name).is_some()
                        || self.generic_fns.contains_key(name)
                        || BARE_BUILTIN_CALLEES.contains(&name);
                    if self.strict_callees && !known && matches!(callee_ty, Ty::Unknown) {
                        let candidates: Vec<&str> = self
                            .top
                            .symbols
                            .iter()
                            .filter(|(_, sym)| matches!(sym, TopSymbol::Fn(_)))
                            .map(|(n, _)| n.as_str())
                            .collect();
                        let hint = closest_bare_name(name, &candidates)
                            .map(|h| format!(" — did you mean `{}`?", h))
                            .unwrap_or_default();
                        self.diags.push(Diag::ty(
                            id.span,
                            format!(
                                "call to `{}`: no free fn, generic fn or fn-pointer \
                                 binding with that name is in scope{}",
                                name, hint
                            ),
                        ));
                    }
                }
                let arg_tys: Vec<Ty> = args.iter().map(|a| self.check_expr(a)).collect();
                // F.20: when a fn param is an interface type, the
                // arg's locus type must structurally satisfy the
                // interface (have the required methods with
                // compatible signatures). Permissive on Unknown,
                // permissive on shape mismatch — the existing
                // checker doesn't enforce arg-vs-param positional
                // typing in general; this fires *only* when the
                // param is an interface, so we don't widen the
                // call-site checking surface beyond that.
                // GH #229: arity UPPER bound at check phase. More
                // args than params is always an error (defaults
                // can only excuse omissions, never extras), and
                // pre-#229 it sailed through check to die as a
                // spanless codegen internal error. The LOWER bound
                // needs default-param counts plumbed into
                // FnSig/MethodInfo — follow-through on the issue.
                if let Ty::Function { params, .. } = &callee_ty {
                    if arg_tys.len() > params.len() {
                        let display = match callee.as_ref() {
                            Expr::Field { name, .. } => {
                                format!("method `{}`", name.name)
                            }
                            Expr::Ident(id) => {
                                format!("fn `{}`", id.name)
                            }
                            Expr::Path(qn) => match self.imported_fn(qn) {
                                Some((written, _)) => format!("fn `{}`", written),
                                None => "this callee".to_string(),
                            },
                            _ => "this callee".to_string(),
                        };
                        self.diags.push(Diag::ty(
                            callee.span(),
                            format!(
                                "{} takes at most {} argument{}, got {}",
                                display,
                                params.len(),
                                if params.len() == 1 { "" } else { "s" },
                                arg_tys.len()
                            ),
                        ));
                    }
                    // GH #229: arity LOWER bound. Ty::Function
                    // erases default-param info, so resolve the
                    // callee's symbol directly; the receiver
                    // re-check is speculative (diags rolled
                    // back — the real check already ran above).
                    let min_required: Option<(usize, String)> =
                        match callee.as_ref() {
                            Expr::Ident(id) => {
                                match self.top.lookup(&id.name) {
                                    Some(TopSymbol::Fn(sig)) => Some((
                                        sig.required_params(),
                                        format!("fn `{}`", id.name),
                                    )),
                                    _ => None,
                                }
                            }
                            // GH #1028: `lib::f(..)`, the imported seed's fn.
                            Expr::Path(qn) => self.imported_fn(qn).map(
                                |(written, sig)| {
                                    (sig.required_params(), format!("fn `{}`", written))
                                },
                            ),
                            Expr::Field { receiver, name, .. } => {
                                let mark = self.diags.len();
                                let rt = self.check_expr(receiver);
                                self.diags.truncate(mark);
                                match rt {
                                    Ty::Named(tn) => {
                                        match self.top.symbols.get(&tn) {
                                            Some(TopSymbol::Locus(li)) => li
                                                .methods
                                                .iter()
                                                .find(|m| m.name == name.name)
                                                .map(|m| (
                                                    m.required_params(),
                                                    format!(
                                                        "method `{}`",
                                                        name.name
                                                    ),
                                                )),
                                            Some(TopSymbol::Perspective(
                                                pi,
                                            )) => pi
                                                .methods
                                                .iter()
                                                .find(|m| m.name == name.name)
                                                .map(|m| (
                                                    m.required_params(),
                                                    format!(
                                                        "method `{}`",
                                                        name.name
                                                    ),
                                                )),
                                            _ => None,
                                        }
                                    }
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                    if let Some((min, display)) = min_required {
                        if arg_tys.len() < min {
                            self.diags.push(Diag::ty(
                                callee.span(),
                                format!(
                                    "{} takes at least {} argument{}, got {}",
                                    display,
                                    min,
                                    if min == 1 { "" } else { "s" },
                                    arg_tys.len()
                                ),
                            ));
                        }
                    }
                    for (i, (param_ty, arg_ty)) in
                        params.iter().zip(arg_tys.iter()).enumerate()
                    {
                        // #335: compare the declared parameter type
                        // against the argument. This loop already had
                        // the callee's params and the arg types in
                        // hand — it checked interface conformance
                        // below and arity above, but never the types
                        // themselves, so `take("s")` on
                        // `fn take(n: Int)` reached codegen and
                        // surfaced as "unsupported in codegen v0".
                        //
                        // `assignable_from` is Unknown-permissive on
                        // both sides, so anything the checker could
                        // not resolve stays silent rather than
                        // becoming a false positive.
                        //
                        // Int -> Float widening is legal at a CALL
                        // (codegen emits the conversion) though not at
                        // an assignment, so it is allowed explicitly
                        // here rather than by relaxing
                        // `assignable_from`.
                        let widening = matches!(
                            (param_ty, arg_ty),
                            (
                                Ty::Prim(PrimType::Float),
                                Ty::Prim(PrimType::Int)
                            )
                        );
                        // F.30 / F.30b: a view coerces to its owned
                        // form at a read-position fn-arg site, and
                        // codegen emits the epoch-checked unpack. This
                        // is legal and common in real code — the
                        // in-tree corpus simply has no call site that
                        // does it, so a first cut of this check
                        // flagged seven files in a downstream
                        // application that build fine.
                        let view_coerces = matches!(
                            (param_ty, arg_ty),
                            (
                                Ty::Prim(PrimType::String),
                                Ty::Prim(PrimType::StringView)
                            ) | (
                                Ty::Prim(PrimType::Bytes),
                                Ty::Prim(PrimType::BytesView)
                            )
                        );
                        // An interface-typed parameter legitimately
                        // accepts any locus that satisfies it, and
                        // `assignable_from` is nominal so it would
                        // reject that. The structural check below owns
                        // this case; skip it here rather than
                        // duplicating (or contradicting) it.
                        let param_is_iface = matches!(
                            param_ty,
                            Ty::Named(n) if matches!(
                                self.top.lookup(n),
                                Some(TopSymbol::Interface(_))
                            )
                        );
                        if !widening
                            && !view_coerces
                            && !param_is_iface
                            && !param_ty.assignable_from(arg_ty)
                        {
                            self.diags.push(Diag::ty(
                                callee.span(),
                                format!(
                                    "argument {} type mismatch: expected \
                                     `{}`, got `{}`",
                                    i,
                                    param_ty.display(),
                                    arg_ty.display()
                                ),
                            ));
                        }
                        if let (Ty::Named(iface_name), Ty::Named(arg_name)) =
                            (param_ty, arg_ty)
                        {
                            // Look up param-named symbol; only
                            // check if it actually resolves to an
                            // interface (not a locus / type /
                            // perspective).
                            let is_iface = matches!(
                                self.top.lookup(iface_name),
                                Some(TopSymbol::Interface(_))
                            );
                            // G20 follow-up: skip the structural
                            // check when the arg is itself the same
                            // interface — interface → same-interface
                            // is identity, no fat-pointer rebuild.
                            // (Different-interface → interface
                            // subtyping is a separate design call.)
                            if is_iface && arg_name != iface_name {
                                if let Err(msg) =
                                    self.check_structural_impl(arg_name, iface_name)
                                {
                                    let span = args
                                        .get(i)
                                        .map(|e| e.span())
                                        .unwrap_or_else(|| Span::new(0, 0));
                                    self.diags.push(Diag::ty(span, msg));
                                }
                            }
                        }
                    }
                }
                let base_ret = match callee_ty {
                    Ty::Function { ret, .. } => *ret,
                    _ => Ty::Unknown,
                };
                // v1.x-FORM-1: if the callee resolves to a
                // fallible fn, wrap the result type so the
                // caller is forced to address the error.
                if let Some(payload) = self.callee_fallible_payload(callee) {
                    Ty::Fallible {
                        success: Box::new(base_ret),
                        payload: Box::new(payload),
                    }
                } else {
                    base_ret
                }
            }
            Expr::Field { receiver, name, span } => {
                // F.11 entity-collection sugar: `self.children.count`
                // (Int) and `self.children.is_empty` (Bool) read the
                // accept'd-child tracker's live count. `self.children`
                // alone is only a `for` iterand (typed `[Child]`); these
                // two accessors are the summary surface F.11 commits to.
                if let Expr::Field {
                    receiver: inner,
                    name: inner_name,
                    ..
                } = receiver.as_ref()
                {
                    if matches!(inner.as_ref(), Expr::KwSelf(_))
                        && inner_name.name == "children"
                        && (name.name == "count" || name.name == "is_empty")
                    {
                        let accepts = self
                            .current_locus
                            .map_or(false, |li| li.accept_param.is_some());
                        if !accepts {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "`self.children.{}` requires the enclosing \
                                     locus to `accept` a child type",
                                    name.name
                                ),
                            ));
                        }
                        return if name.name == "count" {
                            Ty::Prim(PrimType::Int)
                        } else {
                            Ty::Prim(PrimType::Bool)
                        };
                    }
                }
                let rt = self.check_expr(receiver);
                self.check_sealed_access(
                    &rt,
                    name,
                    *span,
                    SealedAccess::Read,
                );
                match self.field_ty(&rt, &name.name) {
                    Some(t) => t,
                    None => {
                        // F.22: `self.<slot>` references a capacity
                        // slot, not a field. Slots don't have a
                        // value-level type the typechecker reasons
                        // about (the cell value only appears when
                        // they're used as a method-call receiver),
                        // so return Unknown rather than diagnosing
                        // a missing field. Codegen catches misuse
                        // (slot in non-method-call position).
                        if matches!(receiver.as_ref(), Expr::KwSelf(_)) {
                            if let Ty::Named(locus_name) = &rt {
                                if let Some(TopSymbol::Locus(li)) =
                                    self.top.symbols.get(locus_name)
                                {
                                    if li
                                        .capacity_slot_names
                                        .iter()
                                        .any(|n| n == &name.name)
                                    {
                                        return Ty::Unknown;
                                    }
                                }
                            }
                        }
                        // Permissive on Unknown — stdlib paths
                        // and externally-typed values pass
                        // through. Strict when the receiver
                        // is a known type and the field
                        // doesn't exist on it: catches typos
                        // statically. GH #241: with a nearest-
                        // name suggestion drawn from the
                        // receiver's fields + methods.
                        // GH #470: stdlib handle-method sugar.
                        // `f.write_line(x)` on a handle typed
                        // `std::io::file::File` is codegen-dispatched
                        // to the handle namespace's free fn
                        // (`__std_io_file_write_line(f, x)`). Mirror
                        // that here: reverse-map the receiver's
                        // mangled name to its public path, swap the
                        // type leaf for the member name, resolve
                        // through the same renames, and type the
                        // member as a bound fn (receiver = arg 0,
                        // checked for compatibility). Without this,
                        // making handles nominal would have turned
                        // the documented handle patterns into false
                        // "no field" errors.
                        if let Ty::Named(tn) = &rt {
                            if let Some(pub_path) = hale_stdlib::PATH_RENAMES
                                .iter()
                                .find(|(_, m)| *m == tn.as_str())
                                .map(|(p, _)| *p)
                            {
                                let mut segs: Vec<&str> = pub_path.to_vec();
                                if let Some(last) = segs.last_mut() {
                                    *last = name.name.as_str();
                                }
                                if let Some(m2) =
                                    crate::stdlib_bodies::mangled_locus_name(
                                        &segs,
                                    )
                                {
                                    if let Some(TopSymbol::Fn(f)) =
                                        self.top.lookup(m2).cloned()
                                    {
                                        let recv_ok = f
                                            .params
                                            .first()
                                            .map(|(_, t)| {
                                                t.assignable_from(&rt)
                                            })
                                            .unwrap_or(false);
                                        if recv_ok {
                                            let tail: Vec<Ty> = f
                                                .params
                                                .iter()
                                                .skip(1)
                                                .map(|(_, t)| t.clone())
                                                .collect();
                                            let ret = match &f.fallible {
                                                Some(p) => Ty::Fallible {
                                                    success: Box::new(
                                                        f.ret.clone(),
                                                    ),
                                                    payload: Box::new(
                                                        p.clone(),
                                                    ),
                                                },
                                                None => f.ret.clone(),
                                            };
                                            return Ty::Function {
                                                params: tail,
                                                ret: Box::new(ret),
                                            };
                                        }
                                    }
                                }
                            }
                        }
                        if !matches!(rt, Ty::Unknown) {
                            let mut candidates: Vec<String> = Vec::new();
                            if let Ty::Named(tn) = &rt {
                                match self.top.symbols.get(tn) {
                                    Some(TopSymbol::Locus(li)) => {
                                        candidates.extend(
                                            li.params.iter().map(|p| p.name.clone()),
                                        );
                                        candidates.extend(
                                            li.methods.iter().map(|m| m.name.clone()),
                                        );
                                    }
                                    Some(TopSymbol::Perspective(pi)) => {
                                        candidates.extend(
                                            pi.params.iter().map(|p| p.name.clone()),
                                        );
                                        candidates.extend(
                                            pi.methods.iter().map(|m| m.name.clone()),
                                        );
                                    }
                                    Some(TopSymbol::Type(ti)) => {
                                        if let TypeKind::Struct(fields) = &ti.kind {
                                            candidates.extend(
                                                fields.iter().map(|f| f.name.clone()),
                                            );
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // (An element-chain source that cannot
                            // answer `get` used to be hinted here.
                            // Both type-level collections answer it
                            // now, so the hint was unreachable —
                            // dead advice about a limitation that no
                            // longer exists is worse than none.)
                            let mut hint = crate::stdlib_surface::nearest_name(
                                &name.name,
                                candidates.iter().map(|s| s.as_str()),
                            )
                            .map(|s| format!(" — did you mean `{}`?", s))
                            .unwrap_or_default();
                            // GH #722: `s.len()` / `s.length` on a
                            // String or Bytes is the member spelling
                            // of a builtin; no candidate name can
                            // bridge it, so point at the builtin.
                            if hint.is_empty() {
                                if let Some(advice) =
                                    crate::stdlib_surface::builtin_member_advice(
                                        &rt, &name.name,
                                    )
                                {
                                    hint = format!(" — {}", advice);
                                }
                            }
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "no field `{}` on `{}`{}",
                                    name.name,
                                    rt.display(),
                                    hint
                                ),
                            ));
                        }
                        Ty::Unknown
                    }
                }
            }
            Expr::Index { receiver, index, .. } => {
                let rt = self.check_expr(receiver);
                let _ = self.check_expr(index);
                match rt {
                    Ty::Array(elem, _) => *elem,
                    _ => Ty::Unknown,
                }
            }
            Expr::Tuple(parts, _) => {
                Ty::Tuple(parts.iter().map(|e| self.check_expr_local(e)).collect())
            }
            Expr::Array(parts, _) => {
                let elem = if let Some(first) = parts.first() {
                    self.check_expr_local(first)
                } else {
                    Ty::Unknown
                };
                for e in parts.iter().skip(1) {
                    let _ = self.check_expr(e);
                }
                Ty::Array(Box::new(elem), Some(parts.len() as u64))
            }
            Expr::ArrayRepeat { val, count, .. } => {
                // `[val; N]` — same array shape, single element
                // type repeated N times. Count is parser-validated
                // as a non-negative Int literal.
                let elem = self.check_expr_local(val);
                Ty::Array(Box::new(elem), Some(*count))
            }
            Expr::Struct { path, inits, span, .. } => self.check_struct_literal(path, inits, *span),
            Expr::Block(b) => self.check_block_as_expr(b),
            Expr::If(s) => self.check_if_as_expr(s),
            // Gap C (2026-07-17): match in expression position types
            // as the join of its arm-body types.
            Expr::Match(m) => self.check_match_core(m, true),
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.check_expr(inner),
            Expr::Approx { left, right, tolerance, span } => {
                if !self.in_closure {
                    self.diags.push(Diag::ty(
                        *span,
                        "approximate-equality (`~~`) only valid inside a closure block"
                            .to_string(),
                    ));
                }
                let _ = self.check_expr(left);
                let _ = self.check_expr(right);
                let _ = self.check_expr(tolerance);
                Ty::Prim(PrimType::Bool)
            }
            Expr::Range { lo, hi, .. } => {
                // v0 ranges are integer iterators only. Both sides
                // must be Int. The expression itself doesn't have a
                // first-class type beyond "iterator over Int" — the
                // for-stmt handler is the only consumer that
                // recognizes it. Returning Unknown lets callers in
                // non-iterator positions still typecheck without
                // the result being used as a value.
                let _ = self.check_expr(lo);
                let _ = self.check_expr(hi);
                Ty::Unknown
            }
            Expr::Or { inner, disposition, span } => {
                let value_discarded = self.or_value_discarded;
                self.or_value_discarded = false;
                let inner_ty = self.check_expr(inner);
                // M3 stage 2 (2026-07-02): stdlib fallible
                // path-calls are dual-mode at codegen (bare = the
                // legacy direct form, `or` = fallible ABI), so the
                // Call arm types them Unknown; the precise
                // success/payload for the `or` form comes from the
                // signature table here.
                let stdlib_or = match inner.as_ref() {
                    Expr::Call { callee, .. } => match callee.as_ref() {
                        Expr::Path(qn) => {
                            let segs: Vec<&str> = qn
                                .segments
                                .iter()
                                .map(|s| s.name.as_str())
                                .collect();
                            // GH #470: a path renaming to a
                            // REGISTERED Hale-source fn is an
                            // ordinary fn call — the Call arm typed
                            // it with its real (possibly Fallible)
                            // signature, and the generic unwrap
                            // below handles it. The surface table's
                            // dual-mode or_types are only for the
                            // Rust builtins (its `open` row was the
                            // stale fd era and OVERRODE the real
                            // File return here).
                            let renamed_registered =
                                crate::stdlib_bodies::mangled_locus_name(
                                    &segs,
                                )
                                .map(|m| {
                                    matches!(
                                        self.top.lookup(m),
                                        Some(TopSymbol::Fn(_))
                                    )
                                })
                                .unwrap_or(false);
                            if renamed_registered {
                                None
                            } else {
                                crate::stdlib_surface::signature_for(
                                    &segs,
                                )
                                .and_then(|sig| sig.or_types())
                            }
                        }
                        _ => None,
                    },
                    _ => None,
                };
                // Unwrap the fallible to get success + payload
                // types. If the inner isn't actually fallible,
                // the `or` clause is a no-op at best and likely
                // a user mistake.
                let (success, payload) = match (stdlib_or, inner_ty) {
                    (Some((s, p)), _) => (s, p),
                    (None, Ty::Fallible { success, payload }) => {
                        (*success, *payload)
                    }
                    (None, Ty::Unknown) => (Ty::Unknown, Ty::Unknown),
                    (None, other) => {
                        // GH #535: name the callee when there is one —
                        // "`std::json::find_int_field` is not fallible"
                        // beats "got `Int`".
                        let callee = match inner.as_ref() {
                            Expr::Call { callee, .. } => match callee.as_ref() {
                                Expr::Path(qn) => Some(
                                    qn.segments
                                        .iter()
                                        .map(|s| s.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join("::"),
                                ),
                                Expr::Ident(id) => Some(id.name.clone()),
                                _ => None,
                            },
                            _ => None,
                        };
                        let msg = match callee {
                            Some(c) => format!(
                                "`{}` is not fallible (it returns `{}`); \
                                 drop the `or` clause",
                                c,
                                other.display()
                            ),
                            None => format!(
                                "`or` disposition expects a fallible-typed \
                                 expression on the left; got `{}` (not fallible). \
                                 Drop the `or` clause if the call can't fail.",
                                other.display()
                            ),
                        };
                        self.diags.push(Diag::ty(inner.span(), msg));
                        return other;
                    }
                };
                match disposition {
                    OrDisposition::Raise(_) => {
                        // `or raise` diverges via closure
                        // violation; expression's value type is
                        // the success type.
                        success
                    }
                    OrDisposition::Wait(span) => {
                        // GH #255: `or wait` is a publish
                        // delivery-mode modifier, not an error
                        // disposition — a fallible call has an
                        // error to handle, not a transient
                        // refusal to outwait.
                        self.diags.push(Diag::ty(
                            *span,
                            "`or wait` is only legal on a bus send to a \
                             transport-bound topic (it parks the \
                             publisher through the binding's loss \
                             window); use `or raise` / `or <default>` / \
                             `or handler(err)` to dispose of a fallible \
                             call's error",
                        ));
                        success
                    }
                    OrDisposition::Discard(span) => {
                        // `or discard` — swallow error, produce
                        // Unit. Requires the underlying call's
                        // success type to be Unit (since discard
                        // doesn't carry a value).
                        if !matches!(success, Ty::Unit | Ty::Unknown) {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "`or discard` requires the underlying \
                                     call's success type to be Unit (so the \
                                     discard branch produces no value to \
                                     bind); got `{}`. Use `or <default>` or \
                                     `or raise` for value-bearing fallibles.",
                                    success.display()
                                ),
                            ));
                        }
                        let _ = payload;
                        Ty::Unit
                    }
                    OrDisposition::Fail(payload_expr, span) => {
                        // B3 / G6: `or fail X` diverges via the
                        // enclosing fallible fn's error path. The
                        // payload's type must match the enclosing
                        // fn's declared error type — not the
                        // inner call's payload. Same divergence
                        // rule as `or raise`: expression type
                        // collapses to the inner success type.
                        //
                        // GH #721: `err` — the INNER call's error, the
                        // same binding the substitute RHS gets — is in
                        // scope on the payload, which is what makes
                        // `or fail DstError { kind: err.kind }` an
                        // inline translation rather than a helper call
                        // (`tests/hale/or_fail_err_binding_test.hl`
                        // runs it). The checker had not entered it.
                        self.locals.push();
                        self.locals.insert(
                            "err",
                            LocalSym {
                                ty: payload.clone(),
                                is_mut: false,
                            },
                        );
                        let new_payload_ty = self.check_expr_addressed(payload_expr);
                        self.locals.pop();
                        match &self.fallible_ctx {
                            None => self.diags.push(Diag::ty(
                                *span,
                                "`or fail X`: only valid inside a fallible \
                                 fn body (declared with `fallible(T)`). \
                                 Use `or raise` to propagate the inner \
                                 payload, or `or <fallback>` to substitute \
                                 a value".to_string(),
                            )),
                            Some((_, expected_payload)) => {
                                if !expected_payload.assignable_from(&new_payload_ty) {
                                    self.diags.push(Diag::ty(
                                        payload_expr.span(),
                                        format!(
                                            "`or fail`: expected payload \
                                             type `{}`, got `{}`",
                                            expected_payload.display(),
                                            new_payload_ty.display()
                                        ),
                                    ));
                                }
                            }
                        }
                        success
                    }
                    OrDisposition::Substitute(rhs) => {
                        // The implicit `err` binding is in scope
                        // on the RHS, typed as the payload type.
                        self.locals.push();
                        self.locals.insert(
                            "err",
                            LocalSym {
                                ty: payload.clone(),
                                is_mut: false,
                            },
                        );
                        let rhs_ty = self.check_expr(rhs);
                        self.locals.pop();
                        // 2026-05-18 — locus → interface coercion at
                        // `or <substitute>` site. Mirrors the
                        // call-site and struct-literal coercion: when
                        // the fallible's success type is an interface
                        // and the substitute expression is a concrete
                        // locus that structurally satisfies it, accept
                        // the substitute. Without this, the substitute
                        // disposition was the only `or` arm that
                        // refused locus→interface flow.
                        let interface_satisfied = if let (
                            Ty::Named(iface_name),
                            Ty::Named(rhs_name),
                        ) = (&success, &rhs_ty)
                        {
                            if matches!(
                                self.top.lookup(iface_name),
                                Some(TopSymbol::Interface(_))
                            ) {
                                // GH #730: the same interface is identity.
                                if rhs_name == iface_name {
                                    true
                                } else {
                                match self.check_structural_impl(rhs_name, iface_name) {
                                    Ok(()) => true,
                                    Err(msg) => {
                                        self.diags.push(Diag::ty(rhs.span(), msg));
                                        true
                                    }
                                }
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        // 2026-07-02 fallible handlers: the substitute
                        // may itself be a fallible call —
                        // `db() or self.convert(err)` where `convert`
                        // is `fallible(E2)`. Semantics are implicit
                        // `or raise` on the handler: its success value
                        // substitutes; its failure propagates through
                        // the ENCLOSING fn's error path, so E2 must be
                        // assignable to the enclosing fallible
                        // payload. (Sugar for the already-legal
                        // `db() or (self.convert(err) or raise)` —
                        // this closes the pond stash-bridge idiom that
                        // made jobs::Queue non-reentrant.)
                        if let Ty::Fallible {
                            success: h_success,
                            payload: h_payload,
                        } = &rhs_ty
                        {
                            if !value_discarded
                                && !success.assignable_from(h_success)
                            {
                                self.diags.push(Diag::ty(
                                    *span,
                                    format!(
                                        "`or <handler>`: handler's success \
                                         type `{}` does not match the \
                                         call's success type `{}`",
                                        h_success.display(),
                                        success.display()
                                    ),
                                ));
                            }
                            match &self.fallible_ctx {
                                None => self.diags.push(Diag::ty(
                                    *span,
                                    format!(
                                        "`or <handler>`: handler is \
                                         `fallible({})` but the enclosing \
                                         fn is not fallible — the \
                                         handler's failure has nowhere to \
                                         go. Declare the enclosing fn \
                                         `fallible({})`, or handle inside \
                                         the handler and drop its \
                                         `fallible`",
                                        h_payload.display(),
                                        h_payload.display()
                                    ),
                                )),
                                Some((_, expected_payload)) => {
                                    if !expected_payload
                                        .assignable_from(h_payload)
                                    {
                                        self.diags.push(Diag::ty(
                                            *span,
                                            format!(
                                                "`or <handler>`: handler \
                                                 fails with `{}` but the \
                                                 enclosing fn is \
                                                 `fallible({})` — the \
                                                 propagated payload must \
                                                 match",
                                                h_payload.display(),
                                                expected_payload.display()
                                            ),
                                        ));
                                    }
                                }
                            }
                            return success;
                        }
                        // Docs/spec pass find (2026-07-02): a
                        // STDLIB fallible path-call used directly
                        // as the handler compiles but silently
                        // yields the un-addressed sret ("" / 0) on
                        // the handler's OWN failure instead of
                        // propagating — the codegen handler
                        // classifier doesn't cover stdlib paths.
                        // Reject with the working spelling until
                        // it does.
                        if let Expr::Call { callee, .. } = rhs.as_ref() {
                            if let Expr::Path(qn) = callee.as_ref() {
                                let segs: Vec<&str> = qn
                                    .segments
                                    .iter()
                                    .map(|s| s.name.as_str())
                                    .collect();
                                let is_fallible_stdlib =
                                    crate::stdlib_surface::signature_for(
                                        &segs,
                                    )
                                    .map(|sig| sig.fallible.is_some())
                                    .unwrap_or(false);
                                if is_fallible_stdlib {
                                    self.diags.push(Diag::ty(
                                        *span,
                                        format!(
                                            "`or {}(...)`: a fallible \
                                             stdlib call can't be the \
                                             handler directly yet — \
                                             write the nested form `or \
                                             ({}(...) or raise)` so its \
                                             own failure has a path",
                                            segs.join("::"),
                                            segs.join("::")
                                        ),
                                    ));
                                }
                            }
                        }
                        // The substitute RHS must produce a
                        // value of the success type (or be a
                        // nested `or` that ultimately produces
                        // one). Permissive on Unknown so we
                        // don't false-positive when the
                        // typechecker can't see through a
                        // stdlib path.
                        // #353: a DIVERGING fallback yields no value,
                        // so there is no type to match. `or { break; }`
                        // / `continue` / `return` / `fail` never reach
                        // the binding, and demanding a typed substitute
                        // from them forces callers to invent a default
                        // that is never used — and for a generic
                        // element type there is no default to invent.
                        let rhs_diverges = matches!(
                            rhs.as_ref(),
                            Expr::Block(b) if block_always_diverges(b)
                        );
                        if !interface_satisfied
                            && !value_discarded
                            && !rhs_diverges
                            && !success.assignable_from(&rhs_ty)
                        {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "`or <substitute>`: fallback type `{}` \
                                     does not match success type `{}`",
                                    rhs_ty.display(),
                                    success.display()
                                ),
                            ));
                        }
                        success
                    }
                }
            }
        }
    }

    /// Same as check_expr but used when we need a type without
    /// risking borrow conflicts with the recursion. (In practice
    /// it's identical; named to mark intent at the call sites.)
    fn check_expr_local(&mut self, expr: &Expr) -> Ty {
        self.check_expr(expr)
    }

    /// v1.x-FORM-1: check an expression that's expected to
    /// produce a regular (non-fallible) value. If the expression
    /// is fallible-typed at its outermost level, emit an
    /// `error not addressed` diagnostic and return the
    /// (would-be) success type so downstream typechecks can
    /// continue without cascading errors.
    fn check_expr_addressed(&mut self, expr: &Expr) -> Ty {
        let ty = self.check_expr(expr);
        match ty {
            Ty::Fallible { success, .. } => {
                self.diags.push(Diag::ty(
                    expr.span(),
                    "error not addressed: this expression's fallible result \
                     must be handled with an `or` clause (`or raise`, \
                     `or <fallback>`, `or handler(err)`) or a `match`"
                        .to_string(),
                ));
                *success
            }
            other => other,
        }
    }

    /// v1.x-FORM-1: if `callee` is a name reference resolving to
    /// a known fallible fn (or method on a locus / perspective),
    /// return the fn's payload type. Returns None for non-fn
    /// callees or non-fallible callees — caller uses the result
    /// to decide whether to wrap the call's return in
    /// `Ty::Fallible`.
    fn callee_fallible_payload(&mut self, callee: &Expr) -> Option<Ty> {
        match callee {
            Expr::Ident(id) => match self.top.lookup(&id.name)? {
                TopSymbol::Fn(sig) => sig.fallible.clone(),
                _ => None,
            },
            Expr::Path(qn) if qn.segments.len() == 1 => {
                match self.top.lookup(&qn.segments[0].name)? {
                    TopSymbol::Fn(sig) => sig.fallible.clone(),
                    _ => None,
                }
            }
            // GH #1028: an imported seed's fn, `alias::f(..)` — typed
            // like a bare one, fallibility included.
            Expr::Path(qn) => self.imported_fn(qn).and_then(|(_, sig)| sig.fallible),
            // v1.x-FORM-1 PR3b: method calls like `l.get(i)`. The
            // callee is a Field expression whose receiver resolves
            // to a locus/perspective; we look up the method by
            // name on that type and inspect its fallibility.
            Expr::Field { receiver, name, .. } => {
                let rt = self.check_expr_local(receiver);
                let type_name = match rt {
                    Ty::Named(n) => n,
                    _ => return None,
                };
                match self.top.lookup(&type_name)? {
                    TopSymbol::Locus(info) => info
                        .methods
                        .iter()
                        .find(|m| m.name == name.name)
                        .and_then(|m| m.fallible.clone()),
                    TopSymbol::Perspective(info) => info
                        .methods
                        .iter()
                        .find(|m| m.name == name.name)
                        .and_then(|m| m.fallible.clone()),
                    // GH #732: a call through an interface carries the
                    // method's declared error channel.
                    TopSymbol::Interface(info) => info
                        .methods
                        .iter()
                        .find(|m| m.name == name.name)
                        .and_then(|m| m.fallible.clone()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether a value of type `t` can be auto-coerced to String
    /// inside a `String + <t>` expression. Mirrors the codegen
    /// `value_to_string_supports` set: every primitive that
    /// `to_string(...)` accepts, plus enums (which render as their
    /// variant name).
    fn ty_is_printable(&self, t: &Ty) -> bool {
        self.ty_is_printable_at(t, 0)
    }

    /// GH #469 bonus lint: `println("x={x}")` prints the braces.
    ///
    /// A plain string literal is not interpolated — only `f"…"` is —
    /// so writing the `{x}` form without the `f` silently produces
    /// `x={x}` instead of `x=3`. Silently is the problem: the
    /// program compiles, runs, and prints something that looks like
    /// a template engine failed. Once f-strings exist this becomes
    /// the single most likely thing for a reader coming from Python,
    /// JavaScript, Rust or C# to get wrong.
    ///
    /// The lint fires only when the braces name something that is
    /// ACTUALLY IN SCOPE. That is what separates it from a
    /// heuristic: `println("{}")`, `println("{\"a\": 1}")` and
    /// `println("use {name} for the placeholder")` in prose about a
    /// template stay quiet, because there is no binding called
    /// `name`. Warning, never an error — a program that means to
    /// print braces around a word that happens to be a local is
    /// unusual but legal.
    fn warn_if_meant_an_fstring(&mut self, a: &Expr) {
        let Expr::Literal(Literal::String(s), span) = a else {
            return;
        };
        let b = s.as_bytes();
        let mut i = 0usize;
        let mut found: Vec<String> = Vec::new();
        while i < b.len() {
            if b[i] != b'{' {
                i += 1;
                continue;
            }
            // `{{` is how a literal brace is written in an f-string;
            // seeing it here signals the author knows the syntax and
            // is deliberately not using it.
            if b.get(i + 1) == Some(&b'{') {
                return;
            }
            let Some(close) = b[i + 1..].iter().position(|c| *c == b'}')
            else {
                break;
            };
            let inner = &s[i + 1..i + 1 + close];
            i += close + 2;
            // Only the head of a dotted path can be looked up;
            // `{p.x}` is just as much a mistake as `{p}`.
            let head = inner.split('.').next().unwrap_or("");
            let is_ident = !head.is_empty()
                && head
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_alphabetic() || c == '_')
                    .unwrap_or(false)
                && head
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !is_ident {
                continue;
            }
            if self.locals.lookup(head).is_some() {
                found.push(inner.to_string());
            }
        }
        if found.is_empty() {
            return;
        }
        self.diags.push(Diag::warn(
            *span,
            format!(
                "lint: this is a plain string, so `{{{}}}` prints \
                 literally — {} names a binding that is in scope. Did \
                 you mean an f-string (`f\"…\"`)? Write `{{{{` for a \
                 literal brace.",
                found[0],
                found[0]
            ),
        ));
    }

    /// GH #469 A3: check `__fmt(value, "spec")`, the desugaring of
    /// `f"{value:spec}"`.
    ///
    /// The parser already rejected specs that are not *grammatical*.
    /// The pairings it could not judge are the ones where the spec
    /// only makes sense for certain values — hexadecimal of a
    /// String, a decimal precision on an Int — and getting those
    /// wrong should read as a type error on the interpolation, not
    /// as a surprise at the far end of the pipeline.
    fn check_fmt_builtin(&mut self, args: &[Expr], callee: &Expr) {
        use hale_syntax::fstring::{FormatSpec, SpecKind};
        if args.len() != 2 {
            self.diags.push(Diag::ty(
                callee.span(),
                format!(
                    "internal: format desugaring expects 2 arguments, \
                     got {}",
                    args.len()
                ),
            ));
            return;
        }
        let vt = self.check_expr_addressed(&args[0]);
        if !self.ty_is_printable(&vt) {
            self.diags.push(Diag::ty(
                args[0].span(),
                format!(
                    "cannot render a value of type `{}` — {}",
                    vt.display(),
                    self.why_unprintable(&vt, 0).unwrap_or_else(|| {
                        "it is not printable".to_string()
                    })
                ),
            ));
            return;
        }
        let Expr::Literal(Literal::String(spec_text), _) = &args[1] else {
            // Only the parser constructs this call, and it always
            // passes a literal.
            return;
        };
        let Ok(spec) = FormatSpec::parse(spec_text) else {
            return;
        };
        // Precision applies to the FRACTIONAL types only. Int and
        // Duration are excluded even though they are numbers:
        // `{n:.2}` on an integer has no meaning, and accepting it
        // silently — codegen has no fractional part to round —
        // would print `42` and leave the author believing the spec
        // did something.
        let fractional = matches!(
            vt,
            Ty::Prim(PrimType::Float | PrimType::Decimal) | Ty::Unknown
        );
        if spec.kind != SpecKind::Display
            && !matches!(vt, Ty::Prim(PrimType::Int) | Ty::Unknown)
        {
            self.diags.push(Diag::ty(
                args[0].span(),
                format!(
                    "hexadecimal formatting applies to `Int`, not \
                     `{}` — drop the `x`, or convert first",
                    vt.display()
                ),
            ));
        }
        if spec.precision.is_some() && !fractional {
            self.diags.push(Diag::ty(
                args[0].span(),
                format!(
                    "a decimal precision applies to `Float` or \
                     `Decimal`, not `{}` — there is no fractional \
                     part to round. For a column WIDTH, use a width \
                     (`{{x:8}}`)",
                    vt.display()
                ),
            ));
        }
    }

    /// GH #469 A2: composite debug rendering.
    ///
    /// `println` / `to_string` / f-string interpolation used to
    /// accept only scalars, String and enums, so `f"{point}"` — the
    /// single most natural thing to write while debugging — was a
    /// type error advising you to "render struct/locus fields
    /// individually". Structs, tuples, fixed arrays and `bounded`
    /// now render recursively.
    ///
    /// Three deliberate exclusions:
    ///
    ///   * **loci, perspectives and interfaces stay unprintable.**
    ///     A locus is flow, not shape; rendering one would also
    ///     hand out the `params` of a `@sealed` locus, which GH #436
    ///     exists to confine. This is the reason the seal is not
    ///     re-litigated below: there is no printable path to it.
    ///   * **`Bytes` stays unprintable** — it is a binary blob, and
    ///     the useful rendering (hex? length? UTF-8 attempt?) is a
    ///     choice the author should make explicitly.
    ///   * **unsized arrays** (`[T]` with no length) have no length
    ///     to walk at the render site.
    ///
    /// `depth` guards mutually-recursive type declarations. Codegen
    /// rejects those at layout time (an inline cycle has no finite
    /// size), but the checker runs first and must not hang on a
    /// program it is about to reject for another reason.
    fn ty_is_printable_at(&self, t: &Ty, depth: u32) -> bool {
        if depth > 16 {
            return false;
        }
        match t {
            Ty::Tuple(ts) => {
                ts.iter().all(|x| self.ty_is_printable_at(x, depth + 1))
            }
            // Element types are restricted to the scalars, matching
            // `value_to_string_supports` in codegen: those are the
            // shapes whose runtime value is reliably a pointer to
            // inline storage the renderer can walk. Widening this
            // without widening codegen is a check/build divergence,
            // which `corpus_check_build_agreement` fails on.
            Ty::Array(elem, Some(_)) | Ty::Bounded(elem, _) => matches!(
                elem.as_ref(),
                Ty::Prim(
                    PrimType::Int
                        | PrimType::Float
                        | PrimType::Bool
                        | PrimType::Decimal
                        | PrimType::Duration
                )
            ),
            Ty::Named(n) => match self.top.symbols.get(n) {
                Some(TopSymbol::Type(ti)) => match &ti.kind {
                    TypeKind::Enum(_) => true,
                    TypeKind::Struct(fs) => fs.iter().all(|f| {
                        self.ty_is_printable_at(&f.ty, depth + 1)
                    }),
                    TypeKind::Alias(inner) => {
                        self.ty_is_printable_at(inner, depth + 1)
                    }
                },
                _ => self.ty_is_printable_scalar(t),
            },
            _ => self.ty_is_printable_scalar(t),
        }
    }

    /// Name the *reason* a type is not printable, as a trailing
    /// clause for the diagnostic.
    ///
    /// Without this the message enumerates the printable set and
    /// leaves the author to diff it against their declaration —
    /// which is hard precisely in the cases that matter. `type
    /// Message { id: String; tags: bounded[String; 32] }` reads as
    /// though it qualifies: both field types appear in the list of
    /// printable things, and the rule that actually excludes it (a
    /// sequence renders only with scalar elements) is two levels
    /// down. So report the offending component by name.
    ///
    /// Returns `None` when the type IS printable, so a caller can
    /// use it as the whole explanation.
    fn why_unprintable(&self, t: &Ty, depth: u32) -> Option<String> {
        if self.ty_is_printable_at(t, depth) {
            return None;
        }
        match t {
            Ty::Named(n) => match self.top.symbols.get(n) {
                Some(TopSymbol::Type(ti)) => match &ti.kind {
                    TypeKind::Struct(fs) => fs.iter().find_map(|f| {
                        self.why_unprintable(&f.ty, depth + 1).map(|why| {
                            format!("`{}`: {}", f.name, why)
                        })
                    }),
                    _ => Some(format!("`{}` does not render", n)),
                },
                Some(TopSymbol::Locus(_)) => Some(format!(
                    "`{}` is a locus — a locus is flow, not shape, and \
                     rendering one would expose the `params` a \
                     `@sealed` locus confines. Render a field",
                    n
                )),
                Some(TopSymbol::Perspective(_) | TopSymbol::Interface(_)) => {
                    Some(format!("`{}` has no text form", n))
                }
                _ => Some(format!("`{}` does not render", n)),
            },
            Ty::Tuple(ts) => ts.iter().enumerate().find_map(|(i, x)| {
                self.why_unprintable(x, depth + 1)
                    .map(|why| format!("component {}: {}", i, why))
            }),
            Ty::Array(elem, Some(_)) | Ty::Bounded(elem, _) => Some(format!(
                "`{}` renders only with scalar elements (Int, Float, \
                 Bool, Decimal, Duration), and its element type is \
                 `{}`",
                t.display(),
                elem.display()
            )),
            Ty::Array(_, None) => Some(
                "an unsized array has no length to walk at the render \
                 site"
                    .to_string(),
            ),
            Ty::Prim(PrimType::Bytes) => Some(
                "`Bytes` is binary — choose a rendering (hex, length, \
                 or a text decode)"
                    .to_string(),
            ),
            other => Some(format!("`{}` does not render", other.display())),
        }
    }

    fn ty_is_printable_scalar(&self, t: &Ty) -> bool {
        match t {
            Ty::Prim(p) => matches!(
                p,
                PrimType::String
                    | PrimType::Int
                    | PrimType::Bool
                    | PrimType::Float
                    | PrimType::Decimal
                    | PrimType::Duration
                    | PrimType::Time
                    | PrimType::StringView
            ),
            // GH #241 (audit item 1): enums render via to_string
            // (variant name); structs / loci / perspectives do
            // NOT — pre-#241 this deferred to a spanless codegen
            // error ("`to_string` not supported for type ...").
            // Resolve the name: enum printable, everything else
            // known is not. Unresolved names stay permissive
            // (imports / synthesized types).
            Ty::Named(n) => match self.top.symbols.get(n) {
                Some(TopSymbol::Type(ti)) => {
                    matches!(ti.kind, TypeKind::Enum(_))
                }
                Some(
                    TopSymbol::Locus(_)
                    | TopSymbol::Perspective(_)
                    | TopSymbol::Interface(_),
                ) => false,
                _ => true,
            },
            Ty::Unknown => true,
            _ => false,
        }
    }

    fn binop_ty(&mut self, op: BinOp, lt: &Ty, rt: &Ty, span: Span) -> Ty {
        use BinOp::*;
        // Ergonomics arc: `String + <printable>` and the symmetric
        // form auto-coerce in codegen via value_to_string. The
        // typechecker mirrors that by short-circuiting on the
        // mixed-String add as a permitted shape that yields String.
        if matches!(op, Add) {
            let l_str = matches!(lt, Ty::Prim(PrimType::String));
            let r_str = matches!(rt, Ty::Prim(PrimType::String));
            if (l_str && self.ty_is_printable(rt))
                || (r_str && self.ty_is_printable(lt))
            {
                return Ty::Prim(PrimType::String);
            }
        }
        // B13 / G30: F.23 Int → Float widening in binary-op
        // position. If exactly one side is Int and the other is
        // Float, the result is Float. Decimal stays strict —
        // F.23 explicitly does NOT widen Int/Float into Decimal
        // (Decimal precision must not silently promote out from
        // monetary scale-9). Mirrors the codegen-side coercion.
        let is_int_float_mix = matches!(
            (lt, rt),
            (Ty::Prim(PrimType::Int), Ty::Prim(PrimType::Float))
                | (Ty::Prim(PrimType::Float), Ty::Prim(PrimType::Int))
        );
        // Crumb batch-3 item 5: Duration scalar arithmetic. A
        // runtime-computed delay (`ms * 1ms` where ms is an Int
        // from an FFI boundary) had no direct expression —
        // Duration literals compose only with Durations. `Int *
        // Duration` (either order) scales the interval; `Duration
        // / Int` divides it. Duration is i64 nanoseconds
        // internally, so both are plain integer ops in codegen.
        let is_dur_scalar_mul = matches!(op, Mul)
            && matches!(
                (lt, rt),
                (Ty::Prim(PrimType::Int), Ty::Prim(PrimType::Duration))
                    | (Ty::Prim(PrimType::Duration), Ty::Prim(PrimType::Int))
            );
        let is_dur_scalar_div = matches!(op, Div)
            && matches!(
                (lt, rt),
                (Ty::Prim(PrimType::Duration), Ty::Prim(PrimType::Int))
            );
        // GH #607: Time is an instant, i64 nanoseconds since the
        // epoch. `Time ± Duration` (and `Duration + Time`) is a Time;
        // `Time - Time` is a Duration; anything else with a Time in
        // it has no meaning and is refused here.
        let is_time = |t: &Ty| matches!(t, Ty::Prim(PrimType::Time));
        let is_dur = |t: &Ty| matches!(t, Ty::Prim(PrimType::Duration));
        let is_time_shift = (matches!(op, Add | Sub) && is_time(lt) && is_dur(rt))
            || (matches!(op, Add) && is_dur(lt) && is_time(rt));
        let is_time_diff = matches!(op, Sub) && is_time(lt) && is_time(rt);
        match op {
            Add | Sub | Mul | Div | Mod | BitAnd | BitOr | BitXor | Shl | Shr => {
                if is_int_float_mix {
                    return Ty::Prim(PrimType::Float);
                }
                if is_dur_scalar_mul || is_dur_scalar_div {
                    return Ty::Prim(PrimType::Duration);
                }
                if is_time_shift {
                    return Ty::Prim(PrimType::Time);
                }
                if is_time_diff {
                    return Ty::Prim(PrimType::Duration);
                }
                if is_time(lt) || is_time(rt) {
                    self.diags.push(Diag::ty(
                        span,
                        "`Time` arithmetic: an instant shifts by a `Duration` \
                         (`t + 5s`, `t - 1h`) and two instants differ by one \
                         (`t2 - t1`); nothing else has a meaning"
                            .to_string(),
                    ));
                    return Ty::Prim(PrimType::Time);
                }
                // Duration × Duration (and ÷ / %) has no unit-
                // sane meaning (ns² / a dimensionless ratio) —
                // reject with a pointer at the scalar forms
                // instead of the codegen catch-all it used to
                // die on.
                if matches!(op, Mul | Div | Mod)
                    && matches!(lt, Ty::Prim(PrimType::Duration))
                    && matches!(rt, Ty::Prim(PrimType::Duration))
                {
                    self.diags.push(Diag::ty(
                        span,
                        "`Duration` cannot be multiplied or divided by \
                         another `Duration` — scale with an Int instead \
                         (`n * 1ms`, `d / 2`)"
                            .to_string(),
                    ));
                    return Ty::Prim(PrimType::Duration);
                }
                if !lt.assignable_from(rt) && !rt.assignable_from(lt) {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "binary op: incompatible operand types `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                if matches!(lt, Ty::Unknown) {
                    rt.clone()
                } else {
                    lt.clone()
                }
            }
            Eq | NotEq | Lt | Gt | LtEq | GtEq => {
                if is_int_float_mix {
                    return Ty::Prim(PrimType::Bool);
                }
                if !lt.assignable_from(rt) && !rt.assignable_from(lt) {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "comparison: incompatible operand types `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                Ty::Prim(PrimType::Bool)
            }
            And | Or => Ty::Prim(PrimType::Bool),
        }
    }

    /// M3 stage 3 tranche 2: match a mangled monomorph name
    /// (`Box_Int`, `Pair_Int_String`) against a generic
    /// template, producing the generic→Ty bindings. The mangle
    /// joins single tokens with `_`; template base names
    /// containing `_` are handled by prefix match. None when no
    /// template matches or the token count disagrees.
    ///
    /// GH #911 B5: generic LOCI are searched too (`Cache_Int_String`
    /// against `locus Cache<K, V>`). The answer says which kind of
    /// declaration it found, because a caller's site decides whether
    /// a locus may appear there — see
    /// [`Self::two_spellings_of_one_monomorph`].
    fn resolve_generic_monomorph(
        &self,
        name: &str,
    ) -> Option<(GenericTemplate<'a>, BTreeMap<String, Ty>)> {
        let templates = self
            .generic_types
            .iter()
            .map(|(base, t)| (base, GenericTemplate::Type(*t)))
            .chain(
                self.generic_loci
                    .iter()
                    .map(|(base, l)| (base, GenericTemplate::Locus(*l))),
            );
        for (base, template) in templates {
            let prefix = format!("{}_", base);
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            let toks: Vec<&str> = rest.split('_').collect();
            if toks.len() != template.generics().len() {
                continue;
            }
            let mut bindings: BTreeMap<String, Ty> = BTreeMap::new();
            for (g, tok) in template.generics().iter().zip(toks.iter()) {
                bindings.insert(
                    g.name.name.clone(),
                    mangle_token_to_ty(tok, self.known),
                );
            }
            return Some((template, bindings));
        }
        None
    }

    /// GH #911 B5: are `want` and `got` the two spellings of ONE
    /// monomorph — an annotation that resolved to the mangled name
    /// (`Box_Int`, `Cache_Int_String`) and a literal that typed as
    /// the generic TEMPLATE it instantiates (`Box`, `Cache`)?
    ///
    /// A literal spelled with the template name carries no type
    /// arguments of its own; codegen takes them from the declared
    /// type at the site and rewrites the path to the monomorph
    /// (`resolve_generic_struct_path`). The checker has to allow the
    /// same thing at the same sites or it refuses programs the build
    /// lowers.
    ///
    /// `allow_loci` is a per-SITE answer, not a property of the
    /// question. Codegen does the rewrite for a generic locus as
    /// well as a generic type at a `let` ascription and a return
    /// slot; at a locus param DEFAULT and at a locus literal's field
    /// init only a generic TYPE resolves (a generic locus there dies
    /// with "not synthesized — discovery missed the use site"), so
    /// those two sites pass `false` and keep check agreeing with the
    /// build. `crates/hale-codegen/tests/generic_monomorph_agreement.rs`
    /// holds the site-by-site evidence.
    fn two_spellings_of_one_monomorph(
        &self,
        want: &Ty,
        got: &Ty,
        allow_loci: bool,
    ) -> bool {
        let (Ty::Named(w), Ty::Named(g)) = (want, got) else {
            return false;
        };
        self.resolve_generic_monomorph(w).is_some_and(|(t, _)| {
            (allow_loci || t.is_type()) && t.name() == g.as_str()
        })
    }

    /// GH #911 B5: the generic template a literal's own path names,
    /// if any. `Box { value: 1 }` types as `Ty::Named("Box")` — the
    /// template, not a monomorph — and that is only lowerable where
    /// a declared type supplies the arguments.
    fn generic_template_named(&self, ty: &Ty) -> Option<GenericTemplate<'a>> {
        let Ty::Named(n) = ty else { return None };
        if let Some(t) = self.generic_types.get(n.as_str()) {
            return Some(GenericTemplate::Type(*t));
        }
        self.generic_loci
            .get(n.as_str())
            .map(|l| GenericTemplate::Locus(*l))
    }

    /// GH #911 B5: refuse a generic literal at a site that declares
    /// no type for it.
    ///
    /// `let b = Box { value: 1 };` and `Cache { cap: 2 };` checked
    /// clean and then died at build with an unlocated
    /// `expression form Discriminant(12)` / `struct literal
    /// "Box": no locus or type by that name` — codegen has no
    /// inference from the literal's fields back to `T`, so there is
    /// nothing to infer the arguments from. The rule is therefore
    /// "say them", and the error carries the line that has to change.
    fn refuse_generic_literal_without_arguments(
        &mut self,
        ty: &Ty,
        span: Span,
    ) {
        let Some(template) = self.generic_template_named(ty) else {
            return;
        };
        let name = template.name();
        let params: Vec<&str> = template
            .generics()
            .iter()
            .map(|g| g.name.name.as_str())
            .collect();
        self.diags.push(Diag::ty(
            span,
            format!(
                "`{}` is a generic {}: a literal spelled with the \
                 template name takes its type arguments from the \
                 declared type at the site, and this site declares \
                 none — write them (`let x: {}<{}> = {} {{ ... }};`)",
                name,
                template.kind(),
                name,
                params.join(", "),
                name
            ),
        ));
    }

    /// GH #911 B5: a generic instantiation's argument COUNT must
    /// match the template's parameter count.
    ///
    /// `Box<Int, String>` used to resolve, silently, to the
    /// monomorph name `Box_Int_String` — a name nothing declares —
    /// and every use of the binding then failed for an unrelated
    /// reason ("no field `value` on `Box_Int_String`") while
    /// `hale build` refused the arity directly. Reported at the
    /// annotation, and not gated on `strict_idents`: arity is
    /// decided by the declaration, which one file of a multi-file
    /// seed can see as well as the whole bundle can.
    fn check_generic_arity(&mut self, te: &TypeExpr) {
        match te {
            TypeExpr::Named { path, generic_args, span } => {
                for arg in generic_args {
                    self.check_generic_arity(arg);
                }
                if path.segments.len() != 1 || generic_args.is_empty() {
                    return;
                }
                let name = &path.segments[0].name;
                let Some(template) =
                    self.generic_template_named(&Ty::Named(name.clone()))
                else {
                    return;
                };
                let want = template.generics().len();
                if generic_args.len() == want {
                    return;
                }
                self.diags.push(Diag::ty(
                    *span,
                    format!(
                        "generic {} `{}` takes {} type argument{}, not {}",
                        template.kind(),
                        name,
                        want,
                        if want == 1 { "" } else { "s" },
                        generic_args.len()
                    ),
                ));
            }
            TypeExpr::Projection { inner, .. } => {
                self.check_generic_arity(inner);
            }
            TypeExpr::Array { elem, .. } | TypeExpr::Bounded { elem, .. } => {
                self.check_generic_arity(elem);
            }
            TypeExpr::Tuple(parts, _) => {
                for p in parts {
                    self.check_generic_arity(p);
                }
            }
            TypeExpr::Function { params, ret, .. } => {
                for p in params {
                    self.check_generic_arity(p);
                }
                if let Some(r) = ret {
                    self.check_generic_arity(r);
                }
            }
            TypeExpr::Primitive(_, _) | TypeExpr::Perspective { .. } => {}
        }
    }

    fn check_struct_literal(
        &mut self,
        path: &QualifiedName,
        inits: &[StructInit],
        span: Span,
    ) -> Ty {
        let mut qualified_resolved: Option<String> = None;
        if path.segments.len() != 1 {
            // Resolve an imported qualified literal (`mat::Grid { }`)
            // to the merged symbol codegen lowers it to, so field and
            // method access on the RESULT is checked against the real
            // definition instead of being waved through as Unknown —
            // the gap that let `mat::Grid { }.make(x)` (no such method;
            // `make` is a free fn) pass `check` and die in codegen.
            //
            // GH #707: the INITS are validated against that resolved
            // declaration too, exactly as a local literal's are.
            // Resolving only the result type left the field names
            // unchecked, so `alias::Resume { key: "main" }` — a typo
            // for `scope` — silently constructed the default while
            // `check` reported `ok`; the same literal on a local type
            // was rejected. Empty renames (every single-seed bundle)
            // skip straight past this.
            let key: Vec<String> =
                path.segments.iter().map(|s| s.name.clone()).collect();
            let imported: Option<String> = self
                .import_renames
                .iter()
                .find(|(p, _)| *p == key)
                .map(|(_, mangled)| mangled.clone());
            if let Some(mangled) = imported {
                if matches!(
                    self.top.lookup(&mangled),
                    Some(
                        TopSymbol::Locus(_)
                            | TopSymbol::Type(_)
                            | TopSymbol::Perspective(_)
                    )
                ) {
                    // Falls through to the full literal validation
                    // below under the merged name. Diagnostics name
                    // `__lib_*` symbols, which the CLI demangles back
                    // to the `alias::Name` the author wrote.
                    qualified_resolved = Some(mangled);
                } else {
                    // Renamed to something that isn't a declaration we
                    // can see: keep the historical tolerance rather
                    // than invent errors against a definition we don't
                    // have.
                    for init in inits {
                        let _ = self.check_expr(&init.value);
                    }
                    return Ty::Unknown;
                }
            }
            if qualified_resolved.is_none() {
                // GH #470: a STDLIB qualified literal resolves through
                // PATH_RENAMES to the mangled symbol the Hale-source
                // stdlib declares (now registered in the top scope),
                // and then falls through to FULL literal validation
                // below — fields checked, interface coercions
                // enforced. The old `Ty::Unknown` tolerance here was
                // fail-open all the way to runtime memory corruption
                // (a wrong-arity middleware coerced to
                // `std::http::Middleware` unchecked).
                let segs: Vec<&str> =
                    path.segments.iter().map(|s| s.name.as_str()).collect();
                if let Some(m) =
                    crate::stdlib_bodies::mangled_locus_name(&segs)
                {
                    if self.top.lookup(m).is_some() {
                        qualified_resolved = Some(m.to_string());
                    } else {
                        // Renamed but not Hale-source-declared (a
                        // Rust-implemented handle, e.g.
                        // std::io::tcp::Listener): keep the historical
                        // tolerance — a Named with no symbol behind it
                        // would trade fail-open for false errors.
                        for init in inits {
                            let _ = self.check_expr(&init.value);
                        }
                        return Ty::Unknown;
                    }
                } else if segs.first() == Some(&"std") {
                    // GH #470: a std:: literal that matches nothing in
                    // the rename table is a typo, not an Unknown —
                    // `std::log::TotallyFakeSink {}` used to typecheck.
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "unknown stdlib type `{}` in struct/locus \
                             literal",
                            key.join("::")
                        ),
                    ));
                    for init in inits {
                        let _ = self.check_expr(&init.value);
                    }
                    return Ty::Unknown;
                } else {
                    // GH #803: a non-`std::` qualified literal the
                    // rename table cannot answer. The tolerance below
                    // stays — the inits are still checked, and the
                    // literal is still `Unknown` — but in a whole
                    // program the path itself is now reported, rather
                    // than left for codegen's `unknown qualified name
                    // `zz::T``.
                    self.check_qualified_path(path);
                    for init in inits {
                        let _ = self.check_expr(&init.value);
                    }
                    return Ty::Unknown;
                }
            }
        }
        let spelled: &String =
            qualified_resolved.as_ref().unwrap_or(&path.segments[0].name);
        // GH #831: `Row2 { id: 1 }` where `type Row2 = Row;`. The
        // alias is transparent in every type position already; a
        // literal spelled with it builds the declaration the chain
        // ends at. Everything below — the monomorph path, the
        // struct / locus / perspective dispatch, the field
        // validation — then runs against that declaration exactly as
        // if the author had written its name.
        let resolved_alias = construction_target(self.top, spelled);
        let name: &String = resolved_alias.as_ref().unwrap_or(spelled);
        // M3 stage 3 tranche 2 (2026-07-02): mangled generic
        // monomorph literal (`Box_Int { ... }`). Resolve the
        // `Base_Tok[_Tok...]` shape against a generic type
        // template, substitute the type args into the field
        // types, and validate the inits like a concrete struct.
        // (Previously "unknown type" — generic types were
        // unusable through the CLI.)
        if self.top.lookup(name).is_none() {
            if let Some((template, bindings)) =
                self.resolve_generic_monomorph(name)
            {
                match template {
                    GenericTemplate::Type(td) => {
                        if let TypeDeclBody::Struct(tfields) = &td.body {
                            let fields: Vec<(String, Ty, bool)> = tfields
                                .iter()
                                .map(|f| {
                                    (
                                        f.name.name.clone(),
                                        substitute_generic_ty(
                                            &f.ty,
                                            &bindings,
                                            self.known,
                                        ),
                                        f.default.is_some(),
                                    )
                                })
                                .collect();
                            return self.check_literal_fields(
                                name, &fields, "type", true, inits, span,
                            );
                        }
                    }
                    // GH #911 B5: the mangled name of a generic
                    // LOCUS monomorph, written out. `hale build`
                    // refuses it — the ownership pre-pass never
                    // numbers a node spelled this way (F.39), with
                    // or without the monomorph having been
                    // discovered — so the checker refuses it too,
                    // and says which spelling does work instead of
                    // "unknown type".
                    GenericTemplate::Locus(ld) => {
                        let args: Vec<String> = ld
                            .generics
                            .iter()
                            .map(|g| {
                                bindings
                                    .get(&g.name.name)
                                    .map(|t| t.display())
                                    .unwrap_or_else(|| "?".to_string())
                            })
                            .collect();
                        let args = args.join(", ");
                        let base = ld.name.name.clone();
                        self.diags.push(Diag::ty(
                            span,
                            format!(
                                "`{}` is the compiler's name for the \
                                 generic locus `{}<{}>`, not a spelling \
                                 you can instantiate — build it through \
                                 the template name with the type \
                                 arguments on the binding \
                                 (`let x: {}<{}> = {} {{ ... }};`)",
                                name, base, args, base, args, base
                            ),
                        ));
                        for init in inits {
                            let _ = self.check_expr(&init.value);
                        }
                        return Ty::Unknown;
                    }
                }
            }
        }
        let sym = match self.top.lookup(name) {
            Some(s) => s,
            None => {
                self.diags.push(Diag::ty(
                    span,
                    format!("unknown type `{}` in struct/locus literal", name),
                ));
                for init in inits {
                    let _ = self.check_expr(&init.value);
                }
                return Ty::Unknown;
            }
        };

        // 2026-05-16 — loci + perspectives now also enforce
        // "missing field" when the param has no default
        // (`has_default: false`). Previously omitted for loci
        // because every param historically carried a default; the
        // required-param form (`name: T;`) introduced 2026-05-16
        // makes the check meaningful — otherwise `Server { port:
        // 8080 }` (missing required `handler`) would silently fall
        // through to codegen.
        let (fields, kind_label, requires_all): (Vec<(String, Ty, bool)>, &str, bool) = match sym {
            TopSymbol::Type(info) => match &info.kind {
                TypeKind::Struct(fields) => (
                    fields
                        .iter()
                        .map(|f| (f.name.clone(), f.ty.clone(), f.has_default))
                        .collect(),
                    "type",
                    true,
                ),
                _ => {
                    self.diags.push(Diag::ty(
                        span,
                        format!("`{}` is not a struct type", name),
                    ));
                    return Ty::Unknown;
                }
            },
            TopSymbol::Locus(info) => (
                info.params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone(), p.has_default))
                    .collect(),
                "locus",
                true,
            ),
            TopSymbol::Perspective(info) => (
                info.params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone(), p.has_default))
                    .collect(),
                "perspective",
                true,
            ),
            _ => {
                self.diags.push(Diag::ty(
                    span,
                    format!("`{}` cannot be instantiated with `{{...}}`", name),
                ));
                return Ty::Unknown;
            }
        };

        self.check_literal_fields(
            name, &fields, kind_label, requires_all, inits, span,
        )
    }

    /// Shared literal field validation — concrete structs/loci/
    /// perspectives and (M3 stage 3 tranche 2) substituted generic
    /// monomorphs both land here.
    fn check_literal_fields(
        &mut self,
        name: &str,
        fields: &[(String, Ty, bool)],
        kind_label: &str,
        requires_all: bool,
        inits: &[StructInit],
        span: Span,
    ) -> Ty {
        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        for init in inits {
            let got = self.check_expr(&init.value);
            match fields.iter().find(|(n, _, _)| n == &init.name.name) {
                Some((_, want, _)) => {
                    // 2026-05-16 — locus → interface coercion at
                    // struct/locus literal init. Mirrors the fn-arg
                    // call-site coercion above so a stateful locus
                    // can flow into an interface-typed field (e.g.
                    // `Server { handler: MyHandler { } }` where
                    // `handler: HttpHandler`).
                    let interface_satisfied = if let (Ty::Named(iface_name), Ty::Named(arg_name)) =
                        (want, &got)
                    {
                        if matches!(
                            self.top.lookup(iface_name),
                            Some(TopSymbol::Interface(_))
                        ) {
                            // GH #730: an interface VALUE into a field of
                            // the same interface is identity — a handle
                            // somebody else owns, stored as a borrow (F.39;
                            // the assignment form since GH #967). The
                            // structural check is for a concrete locus.
                            if arg_name == iface_name {
                                true
                            } else {
                            match self.check_structural_impl(arg_name, iface_name) {
                                Ok(()) => true,
                                Err(msg) => {
                                    self.diags.push(Diag::ty(
                                        init.value.span(),
                                        msg,
                                    ));
                                    true
                                }
                            }
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    // GH #525 item 2 (2026-09-04): perspective
                    // designation at a CONSTRUCTION site —
                    // `App { gw: Gateway { router: RouterV2 { } } }`
                    // where `router: perspective(Router)`. The
                    // param-default path already asks `serves`
                    // (the `conforms` computation in the params
                    // check); this path only knew about
                    // interfaces, so `perspective(P)` — a plain
                    // `Ty::Named(P)` whose symbol is a Perspective,
                    // not an Interface — fell through to
                    // `assignable_from` and `P != Impl`. Codegen's
                    // designation branch already fires for an
                    // override value, so the checker was the only
                    // thing refusing it. Note the slot is
                    // program-global (1-1): an override designates
                    // the whole program's slot, exactly as a
                    // default does.
                    //
                    // LOCUS literals only. This fn is shared with
                    // data-type literals, and codegen's
                    // `populate_user_type_fields` has no designation
                    // path (`type Holder { r: perspective(P); }` then
                    // `Holder { r: Impl { } }` fails at build). A
                    // data-type field keeps the plain mismatch below
                    // so check and build agree (PR #531 review).
                    let perspective_designated = if let (
                        "locus",
                        Ty::Named(pname),
                        Ty::Named(arg_name),
                    ) = (kind_label, want, &got)
                    {
                        if matches!(
                            self.top.lookup(pname),
                            Some(TopSymbol::Perspective(_))
                        ) {
                            match self.top.symbols.get(arg_name) {
                                Some(TopSymbol::Locus(li)) => {
                                    if !li.serves.iter().any(|sv| sv == pname) {
                                        self.diags.push(Diag::ty(
                                            init.value.span(),
                                            format!(
                                                "{} `{}`: field `{}` is \
                                                 `perspective({})`, but `{}` \
                                                 does not serve it — declare \
                                                 `locus {} : serves {}`",
                                                kind_label,
                                                name,
                                                init.name.name,
                                                pname,
                                                arg_name,
                                                arg_name,
                                                pname
                                            ),
                                        ));
                                    }
                                    // Reported (or fine) — either way
                                    // the generic mismatch below would
                                    // only repeat it.
                                    true
                                }
                                _ => false,
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    // GH #911 B5: `Outer { inner: Box { value: 9 } }`
                    // where `inner: Box<Int>` — the field's declared
                    // type resolved to the mangled monomorph, the
                    // literal typed as the template.
                    // `populate_user_type_fields` rewrites the bare
                    // name against the declared field type, so a
                    // TYPE literal's field init builds and runs.
                    //
                    // TYPE literals only, and generic types only.
                    // Codegen's locus-literal path has no such
                    // rewrite (it rewrites a param DEFAULT, not a
                    // field init at the literal), and
                    // `L { b: Box { value: 8 } }` dies at build — so
                    // a locus literal keeps the plain mismatch below,
                    // the same call PR #531's review made for
                    // perspective designation.
                    let monomorph_field = kind_label == "type"
                        && self.two_spellings_of_one_monomorph(
                            want, &got, false,
                        );
                    if !interface_satisfied
                        && !perspective_designated
                        && !monomorph_field
                        && !want.assignable_from(&got)
                    {
                        self.diags.push(Diag::ty(
                            init.value.span(),
                            format!(
                                "{} `{}`: field `{}` expects `{}`, got `{}`",
                                kind_label,
                                name,
                                init.name.name,
                                want.display(),
                                got.display()
                            ),
                        ));
                    }
                }
                None => {
                    self.diags.push(Diag::ty(
                        init.span,
                        format!(
                            "{} `{}` has no field `{}`",
                            kind_label, name, init.name.name
                        ),
                    ));
                }
            }
            seen.insert(init.name.name.clone(), ());
        }
        if requires_all {
            for (fname, fty, has_default) in fields {
                // bounded[T; N] fields auto-init empty and cannot
                // be spelled in a literal.
                if matches!(fty, Ty::Bounded(_, _)) {
                    if seen.contains_key(fname) {
                        self.diags.push(Diag::ty(
                            span,
                            format!(
                                "{} `{}` field `{}`: bounded[T; N] \
                                 fields cannot be initialized in a \
                                 literal — they start empty; use \
                                 push(...)",
                                kind_label, name, fname
                            ),
                        ));
                    }
                    continue;
                }
                if !seen.contains_key(fname) && !has_default {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "{} `{}`: missing field `{}`",
                            kind_label, name, fname
                        ),
                    ));
                }
            }
        }

        Ty::Named(name.to_string())
    }
}

fn lit_ty(lit: &Literal) -> Ty {
    match lit {
        Literal::Int(_) => Ty::Prim(PrimType::Int),
        Literal::Float(_) => Ty::Prim(PrimType::Float),
        Literal::Decimal(_) => Ty::Prim(PrimType::Decimal),
        Literal::String(_) => Ty::Prim(PrimType::String),
        Literal::Bool(_) => Ty::Prim(PrimType::Bool),
        Literal::Nil => Ty::Unknown,
        Literal::Duration(_) => Ty::Prim(PrimType::Duration),
        Literal::Time(_) => Ty::Prim(PrimType::Time),
        Literal::Bytes(_) => Ty::Prim(PrimType::Bytes),
    }
}

/// #334 / #333: one locus instance may not be shared by two
/// main-locus fields placed on different pools.
///
/// F.31 keeps a locus's methods on its own pool's thread, but it
/// reasons per FIELD DECLARATION: for `self.s.bump()` inside
/// `WorkerA`, `s` has no placement entry of its own so it inherits
/// WorkerA's pool, and WorkerB reasons identically about its own `s`.
/// Neither is wrong about its own field — they are wrong about each
/// other, and nothing related the two declarations back to the single
/// object they both name.
///
/// The result was a silent data race: two pinned workers mutating one
/// locus lost ~30% of their writes with `hale check` reporting `ok`.
///
/// Scope, deliberately: the STATIC params-init tower of the main
/// locus. Instances created dynamically (in a loop, via `accept()`)
/// have no static identity to reason about, and they inherit their
/// creator's pool, so they are not the shape this protects.
fn check_instance_aliasing(
    bundle: &Bundle,
    diags: &mut Vec<Diag>,
) {
    // GH #825: the main locus, and the aliased locus type whose
    // state this rule asks about, are both found by name — a module
    // changes neither.
    let mut main: Option<&LocusDecl> = None;
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            if let TopDecl::Locus(l) = item {
                if l.is_main {
                    main = Some(l);
                }
            }
        });
    }
    let Some(main) = main else { return };

    let placement: BTreeMap<String, PoolId> = main
        .members
        .iter()
        .find_map(|m| match m {
            LocusMember::Placement(pb) => Some(pb),
            _ => None,
        })
        .map(|pb| {
            pb.entries
                .iter()
                .map(|e| {
                    (
                        e.field.name.clone(),
                        placement_spec_to_pool(&e.spec, &e.field.name),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let Some(params) = main.members.iter().find_map(|m| match m {
        LocusMember::Params(pb) => Some(pb),
        _ => None,
    }) else {
        return;
    };

    // shared field -> [(holder field, pool, span)]
    let mut holders: BTreeMap<String, Vec<(String, PoolId, Span)>> =
        BTreeMap::new();
    for p in &params.params {
        let ParamInit::Value(init) = &p.init else { continue };
        let pool = placement
            .get(&p.name.name)
            .cloned()
            .unwrap_or(PoolId::Cooperative("main".to_string()));
        let mut refs = Vec::new();
        collect_self_field_refs(init, &mut refs);
        for r in refs {
            holders.entry(r).or_default().push((
                p.name.name.clone(),
                pool.clone(),
                p.span,
            ));
        }
    }

    for (shared, hs) in &holders {
        if hs.len() < 2 {
            continue;
        }
        let first_pool = &hs[0].1;
        // The type of the aliased field, so we can ask whether it
        // actually holds unsynchronized state.
        let aliased_ty = params
            .params
            .iter()
            .find(|p| &p.name.name == shared)
            .and_then(|p| p.ty.as_ref())
            .and_then(|te| match te {
                TypeExpr::Named { path, .. } => {
                    path.segments.last().map(|s| s.name.clone())
                }
                _ => None,
            });
        let Some(why) = aliased_ty
            .as_deref()
            .and_then(|ty| locus_has_unsynchronized_state(bundle, ty))
        else {
            // Every mutable field is behind a `sync` discipline, so
            // the form orders the accesses and the alias is safe.
            // This is the case that used to need `@shared` to
            // silence — the check was too blunt, not the program
            // wrong.
            continue;
        };
        if let Some(other) =
            hs.iter().find(|(_, pool, _)| pool != first_pool)
        {
            // A WARNING, not an error, and deliberately so. The
            // sanctioned way to share state across pools is a
            // `@form(..., sync = ...)` locus, and a plain locus whose
            // mutable state sits entirely behind such fields is a
            // legitimate design — two applications in a downstream
            // fleet do exactly that, with the reasoning written in a
            // comment above their placement block.
            //
            // Distinguishing "all mutable state is synchronized" from
            // "this races" needs the declared shared-locus surface
            // discussed on #333; until that exists, reporting the
            // aliasing without failing the build is the honest
            // position. The race it points at is real: two pinned
            // workers mutating one plain locus lose ~30% of their
            // writes.
            diags.push(Diag::warn(
                other.2,
                format!(
                    "locus `self.{}` is shared by `{}` and `{}`, which are \
                     placed on different pools, and it holds \
                     unsynchronized mutable state: {}. Two threads can \
                     reach that state with nothing ordering them. Put the \
                     state behind a form carrying a `sync` discipline, \
                     give each holder its own instance, or coordinate \
                     through the bus.",
                    shared, hs[0].0, other.0, why
                ),
            ));
        }
    }
}

/// `self.<field>` references appearing as values inside a locus
/// initializer, which is how one instance reaches two towers.
fn collect_self_field_refs(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Field { receiver, name, .. }
        | Expr::Path2 { receiver, name, .. } => {
            if matches!(receiver.as_ref(), Expr::KwSelf(_)) {
                out.push(name.name.clone());
            } else {
                collect_self_field_refs(receiver, out);
            }
        }
        Expr::Struct { inits, .. } => {
            for f in inits {
                collect_self_field_refs(&f.value, out);
            }
        }
        Expr::Call { args, .. } => {
            for a in args {
                collect_self_field_refs(a, out);
            }
        }
        _ => {}
    }
}

fn collect_self_assign_spans(b: &Block, f: &mut impl FnMut(Span)) {
    for s in &b.stmts {
        collect_self_assign_in_stmt(s, f);
    }
}

fn collect_self_assign_in_stmt(s: &Stmt, f: &mut impl FnMut(Span)) {
    match s {
        Stmt::Assign { target, span, .. } => {
            // `self.n = ...` is head `self` with a field tail. A bare
            // `n = ...` (head only) is a local, not shared state.
            if target.head.name == "self" && !target.tail.is_empty() {
                f(*span);
            }
        }
        Stmt::If(i) => {
            collect_self_assign_spans(&i.then_block, f);
            if let Some(e) = &i.else_block {
                match e.as_ref() {
                    ElseBranch::Else(b) => collect_self_assign_spans(b, f),
                    ElseBranch::ElseIf(inner) => {
                        collect_self_assign_in_stmt(
                            &Stmt::If(inner.clone()),
                            f,
                        );
                    }
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            collect_self_assign_spans(body, f)
        }
        Stmt::Block(b) => collect_self_assign_spans(b, f),
        _ => {}
    }
}

/// #341 follow-up: does aliasing this locus across pools actually
/// race?
///
/// The hazard was never "is it shared" — a locus whose mutable state
/// lives entirely behind `sync`-bearing forms is safe to reach from
/// several pools, because the form's discipline orders the accesses.
/// The hazard is **unsynchronized** mutable state reachable from two
/// threads, and that is directly checkable:
///
///   - a method assigning `self.<field> = ...` mutates a plain field
///     with nothing ordering it
///   - a field whose type is a form WITHOUT a `sync` kwarg is mutable
///     through its synthesized methods with nothing ordering it
///
/// Either makes the alias a race. Neither makes it safe by
/// declaration, which is why this needs no annotation.
fn locus_has_unsynchronized_state(
    bundle: &Bundle,
    locus_ty: &str,
) -> Option<String> {
    // GH #825: `forms` decides whether each field of the aliased
    // locus is behind a `sync` discipline. A `@form` locus this walk
    // cannot see is simply absent from the map, and an absent entry
    // reads as "not an unsynchronized form" — so a module-nested
    // form silenced the finding for a top-level alias too.
    let mut decl: Option<&LocusDecl> = None;
    let mut forms: BTreeMap<String, bool> = BTreeMap::new();
    for program in bundle.programs.values() {
        walk_decls(&program.items, &mut |item| {
            let TopDecl::Locus(l) = item else { return };
            if l.name.name == locus_ty {
                decl = Some(l);
            }
            if let Some(f) = &l.form {
                let synced =
                    f.args.iter().any(|a| a.name.name == "sync");
                forms.insert(l.name.name.clone(), synced);
            }
        });
    }
    let l = decl?;

    for m in &l.members {
        match m {
            LocusMember::Fn(fd) => {
                let mut hit: Option<String> = None;
                collect_self_assign_spans(&fd.body, &mut |_| {
                    hit.get_or_insert_with(|| {
                        format!("`{}` assigns its own fields", fd.name.name)
                    });
                });
                if let Some(h) = hit {
                    return Some(h);
                }
            }
            LocusMember::Params(pb) => {
                for prm in &pb.params {
                    let Some(TypeExpr::Named { path, .. }) = &prm.ty else {
                        continue;
                    };
                    let Some(seg) = path.segments.last() else { continue };
                    if forms.get(&seg.name) == Some(&false) {
                        return Some(format!(
                            "field `{}` is a `@form({})` with no `sync` \
                             discipline",
                            prm.name.name, seg.name
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// GH #877: the type names the COMPILER declares, which therefore
/// name something even when no declaration in the bundle does.
///
/// Codegen synthesizes each of these unconditionally (`codegen.rs`'s
/// builtin-type declarations; `CapacityError` in `form/bounded.rs`),
/// so a signature naming one lowers. The resolver injects most of
/// them into the top scope as well — `IoError`, `ParseError`,
/// `CryptoError`, `IndexError`, `KeyError`, `EmptyError`,
/// `CapacityError` are there unconditionally since 2026-07-29, and
/// `BusUnmatchedKey` only when a topic declares `on_unmatched: fail`
/// — but `ClosureViolation`, the `on_failure` error payload, is
/// injected nowhere and has always resolved to `Ty::Unknown`.
///
/// Listing all of them keeps the unknown-bare-type-name rule at
/// least as permissive as codegen: an entry that the top scope
/// already carries is simply redundant, while a missing one would
/// refuse a program `hale build` accepts.
const SYNTHESIZED_TYPE_NAMES: &[&str] = &[
    "BusUnmatchedKey",
    "CapacityError",
    "ClosureViolation",
    "CryptoError",
    "EmptyError",
    "IndexError",
    "IoError",
    "KeyError",
    "ParseError",
];

/// The bare names codegen answers itself when they resolve to no user
/// fn. A call to any other unbound bare name is refused by `hale
/// build`, so the checker refuses it first (dna/FRICTION.md F.18).
///
/// **The table is a contract in both directions, and both are
/// tested rather than trusted.**
///
/// A name codegen answers that this table LACKS makes the admission
/// gate refuse a program `hale run` executes — GH #779, where
/// `starts_with` / `contains` / `eprint` / `check_closures` / `mean`
/// were all missing and four corpus fixtures were red under `hale
/// check <dir>` while building and running fine. Enforced by
/// `corpus_check_build_agreement`'s
/// `strict_check_refuses_nothing_the_build_accepts`, which runs the
/// strict-callee rule over the whole corpus and builds anything the
/// rule refuses.
///
/// A name this table LISTS that codegen cannot lower is the mirror
/// defect: `hale check` accepts a call `hale build` refuses, late,
/// from another layer and without a span — GH #800, where `hex`,
/// `panic`, `exit` and five of the six primitive-type spellings had
/// no arm anywhere in codegen. Enforced by the same file's
/// `every_bare_builtin_callee_lowers`, which compiles a program
/// calling every name below.
///
/// Grouped by the codegen dispatch site that answers each name.
pub const BARE_BUILTIN_CALLEES: &[&str] = &[
    // lower_expr's `Expr::Call` arms (hale-codegen `codegen.rs`).
    // `Int` and `Float` are the two numeric casts of
    // spec/types.md § "Explicit numeric conversions"; the other
    // primitive type names are types, not conversions, and a call
    // to one is an ordinary unbound callee.
    "len", "to_string", "Int", "Float", "abs", "min", "max",
    // lower_str_predicate_builtin.
    "starts_with", "contains",
    // Statement position: lower_print_call's four printers, and the
    // explicit-epoch closure surface.
    "println", "print", "eprintln", "eprint", "check_closures",
    // Accumulator vocabulary inside a closure assertion
    // (`collect_sum_calls`). `count()` and `mean(x)` arrive here as
    // calls. `sum(x)` and `prod(x)` do NOT: the parser gives them
    // dedicated AST nodes (`Expr::Sum` / `Expr::Prod`), so they are
    // never a bare callee and this rule never consults the table
    // for them. #779 listed them for the reader; that made the
    // table claim codegen answers `prod`, which it does not — there
    // is no `Expr::Prod` arm in `lower_expr` at all.
    "count", "mean",
    // bounded[T; N] intrinsics — `clear`/`truncate` direct,
    // `push`/`at`/`set` through the fallible (`or`) path.
    "clear", "truncate", "push", "at", "set",
    // lower_fmt_builtin: the parser's desugaring of `f"{x:spec}"`.
    hale_syntax::parser::FMT_BUILTIN,
];

/// GH #803: the qualified paths codegen answers WITHOUT the `std::`
/// prefix — the qualified twin of [`BARE_BUILTIN_CALLEES`], and the
/// same obligation: the unresolvable-path rule must refuse nothing
/// the build accepts.
///
/// These two are a legacy spelling from before the stdlib moved under
/// `std::`, still lowered by `lower_path_call` (statement position)
/// and `lower_path_call_expr` (`monotonic`) in `codegen.rs` and still
/// written by the embedded test corpus. Every OTHER unprefixed
/// stdlib-looking path is refused by codegen, which is why the rule
/// reports it — with codegen's own "did you mean `std::…`" when the
/// prefix would resolve it.
pub const UNPREFIXED_STDLIB_PATHS: &[(&str, &str)] =
    &[("time", "sleep"), ("time", "monotonic")];

/// Nearest name by edit distance, for the "did you mean" hint; `None`
/// when nothing is within a short distance.
fn closest_bare_name<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    fn dist(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut cur = vec![i; b.len() + 1];
            for j in 1..=b.len() {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            }
            prev = cur;
        }
        prev[b.len()]
    }
    candidates
        .iter()
        .map(|c| (dist(name, c), *c))
        .filter(|(d, _)| *d <= 3)
        .min_by_key(|(d, c)| (*d, c.to_string()))
        .map(|(_, c)| c)
}

/// GH #803: the name to suggest for one SEGMENT of a qualified path —
/// a substring match first (`recv` for `recv_into`, which a plain
/// edit distance misses), then a spelling within one or two edits.
///
/// Codegen's `unknown_qualified_name` (codegen.rs) chooses by these
/// same two rules over the same rename table, so the check-time
/// message and the build-time one suggest the same name for the same
/// mistake. The looser `closest_bare_name` above is right for a BARE
/// name, where the candidate set is the program's own vocabulary; a
/// path segment is matched against a library's exports, where a
/// three-edit "match" is noise (`nope` is three edits from `Mood`).
fn nearest_qualified_segment(name: &str, candidates: &[&str]) -> Option<String> {
    let lc = name.to_lowercase();
    let substring_hit = candidates.iter().find(|c| {
        let c_lc = c.to_lowercase();
        (lc.len() >= 3 && c_lc.contains(&lc))
            || (c_lc.len() >= 3 && lc.contains(&c_lc))
    });
    if let Some(hit) = substring_hit {
        return Some((*hit).to_string());
    }
    crate::stdlib_surface::nearest_name(name, candidates.iter().copied())
}
