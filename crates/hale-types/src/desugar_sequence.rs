//! The one desugar sequence before the check (F.40 phase 2.1b).
//!
//! [`desugar_before_check`] is what every entry point runs over the
//! program it loaded, after its own prefix passes and before it mints
//! the snapshot: the declaration-shaping rewrites that change how a
//! declaration is spelled without erasing a reference the checker's
//! laws read. The checker, the model and lowering then see one program
//! shape, so no consumer has to know the second spelling.
//!
//! In order:
//!
//! 1. JSON parsers: `json_gen` synthesizes `__json_parse_<T>` and
//!    rewrites `T::from_json` (per program).
//! 2. the api surface: `--api` injects the main locus's `api:` entry,
//!    and `generate_api` synthesizes the binding for any entry the
//!    source or the flag spelled, with the roles the caller binds
//!    (bundle-wide: the main locus in one file, subscribers in
//!    another).
//! 3. unit returns: `-> ()` is "no return type" on every fn-shaped
//!    declaration.
//! 4. construction aliases: a struct literal, variant path or
//!    constructor pattern spelled with a type alias names the
//!    declaration the alias chain ends at
//!    ([`crate::mangle::resolve_construction_aliases`]), over the
//!    whole bundle at once.
//! 5. the omitted `run`: a locus that declares no `run` gets an empty
//!    one (GH #735, spec `semantics.md` § `run()`: the two spellings
//!    are the same program), carrying its locus's span, so the checker
//!    and the model see the program lowering lowers.
//!
//! The first two generate declarations; everything after them sees the
//! generated ones too. The bundled stdlib goes through the passes that
//! shape a declaration (3 onward, [`bundled_stdlib`]) before the
//! resolved program appends it; it spells no `from_json` and no api.

use std::sync::OnceLock;

use hale_syntax::ast::{LocusMember, Program, TopDecl, TypeExpr};

/// What the sequence is run with: the inputs its passes read besides
/// the programs themselves.
pub struct Sequence<'a> {
    /// The build's cross-seed rename table (`alias::Name` → mangled).
    pub import_renames: &'a [(Vec<String>, String)],
    /// The build's `--api` path, if any: the entry the api pass injects.
    pub api: Option<&'a str>,
    /// The roles the build's environment binds, if any: what the api
    /// pass bakes into the binding.
    pub api_roles: Option<&'a str>,
}

/// Run the sequence over every program of a bundle, in place.
///
/// Idempotent: a program the sequence already shaped comes back
/// unchanged, so a caller that cannot tell whether its program went
/// through it (the test harness), or that ran a pass itself first (a
/// verb that reports a refused `--api` in its own words), may run it
/// again. The error is a refused `--api` injection: no main locus to
/// carry the entry.
pub fn desugar_before_check(
    programs: &mut [&mut Program],
    seq: &Sequence<'_>,
) -> Result<(), String> {
    for p in programs.iter_mut() {
        hale_syntax::json_gen::generate_json_parsers(p);
    }
    if let Some(path) = seq.api {
        // The entry goes on the main locus, wherever the bundle holds
        // it; a bundle with none is refused by the injection itself.
        let at = programs
            .iter()
            .position(|p| p.items.iter().any(|i| matches!(i, TopDecl::Locus(l) if l.is_main)))
            .unwrap_or(0);
        if let Some(p) = programs.get_mut(at) {
            hale_syntax::api_gen::inject_api_entry(p, path)?;
        }
    }
    hale_syntax::api_gen::generate_api(programs, seq.api_roles);
    // The bundled stdlib is what a bundle-wide pass reads besides the
    // bundle: the declarations an alias may end at. A stdlib that does
    // not parse is reported where it is appended (`resolve_program`);
    // here the passes run without it.
    let stdlib = bundled_stdlib().ok();
    shape(programs, seq, stdlib.as_slice());
    Ok(())
}

/// The passes that shape a declaration, in order. `context` is read and
/// never rewritten.
fn shape(programs: &mut [&mut Program], seq: &Sequence<'_>, context: &[&Program]) {
    for p in programs.iter_mut() {
        normalize_unit_return_annotations(&mut p.items);
    }
    crate::mangle::resolve_construction_aliases(programs, context, seq.import_renames);
    for p in programs.iter_mut() {
        hale_syntax::desugar::desugar_omitted_run(p);
    }
}

/// The bundled stdlib, parsed once and put through the passes that
/// shape a declaration: what the resolved program appends, and what the
/// checker's analyses read as the stdlib's bodies and declarations
/// ([`crate::stdlib_bodies::program`]), so both see one shape. Its own
/// declarations only: no rename table reaches it. The error is the
/// parse failure, rendered.
pub fn bundled_stdlib() -> Result<&'static Program, String> {
    static SHAPED: OnceLock<Result<Program, String>> = OnceLock::new();
    SHAPED
        .get_or_init(|| {
            let mut stdlib =
                hale_syntax::parse_source(hale_stdlib::AP_SOURCE).map_err(|diags| {
                    let summary = diags
                        .iter()
                        .map(|d| format!("{:?}", d))
                        .collect::<Vec<_>>()
                        .join("; ");
                    format!("stdlib parse: {}", summary)
                })?;
            let seq = Sequence { import_renames: &[], api: None, api_roles: None };
            shape(&mut [&mut stdlib], &seq, &[]);
            Ok(stdlib)
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// `-> ()` is spelled unit: rewrite an empty-tuple return annotation to
/// "no return type" on every fn-shaped declaration, so no signature
/// consumer (the checker's, lowering's) ever sees a 0-element tuple in
/// a return position. A `()` elsewhere (a fn TYPE's return, a type
/// argument) is still a type the checker resolves to `Unit`.
fn normalize_unit_return_annotations(items: &mut [TopDecl]) {
    fn norm(ret: &mut Option<TypeExpr>) {
        if matches!(ret, Some(TypeExpr::Tuple(parts, _)) if parts.is_empty()) {
            *ret = None;
        }
    }
    for item in items {
        match item {
            TopDecl::Fn(f) => norm(&mut f.ret),
            TopDecl::Interface(i) => {
                for m in &mut i.methods {
                    norm(&mut m.ret);
                }
            }
            TopDecl::Locus(l) => {
                for member in &mut l.members {
                    match member {
                        LocusMember::Fn(f) => norm(&mut f.ret),
                        LocusMember::Mode(md) => norm(&mut md.ret),
                        LocusMember::Lifecycle(lc) => norm(&mut lc.ret),
                        _ => {}
                    }
                }
            }
            TopDecl::Module(m) => normalize_unit_return_annotations(&mut m.items),
            _ => {}
        }
    }
}
