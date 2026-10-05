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
//!    4b. qualified bus subjects: `subscribe alias::Topic`,
//!    `publish alias::Topic`, `alias::Topic <- v` and a `bindings`
//!    entry name the single-segment topic the imported declaration
//!    ends up at ([`crate::qualified_subjects`]), so the checker and
//!    lowering read one resolution of the path.
//! 5. the omitted `run`: a locus that declares no `run` gets an empty
//!    one (GH #735, spec `semantics.md` § `run()`: the two spellings
//!    are the same program), carrying its locus's span, so the checker
//!    and the model see the program lowering lowers.
//! 6. repr accessors: `L2::price(v)` / `L2::set_price(w, x)` on a
//!    `repr:`-tagged wire type become the `std::bytes::read_*` /
//!    `write_*` calls they mean, with the layout's offsets (bundle-wide:
//!    the type in one file, the accessor in another). An accessor the
//!    pass refuses (no such field, the wrong arity) stays as written,
//!    for the checker to report.
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
/// again. The value is the api surface the sequence generated a binding
/// for, if it generated one (a second run finds the binding and
/// generates none). The error is a refused `--api` injection: no main
/// locus to carry the entry.
pub fn desugar_before_check(
    programs: &mut [&mut Program],
    seq: &Sequence<'_>,
) -> Result<Option<hale_syntax::api_gen::ApiSurface>, String> {
    for p in programs.iter_mut() {
        hale_syntax::json_gen::generate_json_parsers(p);
    }
    // The api binding joins the root lowering deploys as a param on a
    // pool of its own, so it is that root's: the entry row's lowering
    // root, read over the programs as they stand (F.40 phase 3), never
    // an imported library's `main locus`, and the entry once lowering
    // deploys the entry (L4). `--api` puts its entry there, and a seed
    // with no root is refused, saying why.
    let row = {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        crate::entry::entry_row_in(&ro)
    };
    let root = row.lowering_root.as_ref().and_then(|m| m.index_in());
    if let Some(path) = seq.api {
        let (at, item) = root.ok_or_else(|| api_refusal(row.no_entry()))?;
        let l = hale_syntax::ast::locus_at_mut(&mut programs[at].items, item)
            .expect("the entry row names a locus of these programs");
        hale_syntax::api_gen::inject_api_entry(l, path);
    }
    let surface = hale_syntax::api_gen::generate_api(programs, root, seq.api_roles);
    // The bundled stdlib is what a bundle-wide pass reads besides the
    // bundle: the declarations an alias may end at. A stdlib that does
    // not parse is reported where it is appended (`resolve_program`);
    // here the passes run without it.
    let stdlib = bundled_stdlib().ok();
    shape(programs, seq, stdlib.as_slice());
    Ok(surface)
}

/// Why `--api` has nowhere to put its entry: lowering deploys no `main
/// locus` of the seed's own (`why` is the row's account of the entry).
fn api_refusal(why: Option<crate::entry::NoEntry>) -> String {
    match why {
        Some(crate::entry::NoEntry::NoMain) => "--api needs a `main locus` to bind: the api entry lives in its \
                                               `bindings { }` block, and this program has only a bare `fn main`"
            .to_string(),
        _ => "--api needs the seed's own `main locus` to bind: the api entry lives in its \
              `bindings { }` block, and the only `main locus` here is an imported library's, \
              whose bindings are inert"
            .to_string(),
    }
}

/// The passes that shape a declaration, in order. `context` is read and
/// never rewritten.
fn shape(programs: &mut [&mut Program], seq: &Sequence<'_>, context: &[&Program]) {
    for p in programs.iter_mut() {
        normalize_unit_return_annotations(&mut p.items);
    }
    crate::mangle::resolve_construction_aliases(programs, context, seq.import_renames);
    for p in programs.iter_mut() {
        crate::qualified_subjects::resolve_qualified_bus_subjects(p, seq.import_renames);
    }
    for p in programs.iter_mut() {
        hale_syntax::desugar::desugar_omitted_run(p);
    }
    hale_syntax::desugar::desugar_repr_accessors(programs);
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
