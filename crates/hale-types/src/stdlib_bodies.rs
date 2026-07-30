//! The Hale-source stdlib, parsed once, for *analysis*.
//!
//! `hale-stdlib` holds the `.hl` modules that implement part of the
//! standard library in Hale itself (`std::io::file::File`,
//! `std::cli::Resolver`, `std::log::Logger`, …). Codegen has always
//! appended them to the user program before lowering. The analyzer
//! never saw them, and that was a soundness hole: a call through a
//! stdlib handle (`f.read_all()`) had no body to walk, so it
//! contributed no effects and `@no_syscall` passed over real I/O.
//!
//! Feeding these bodies to the callgraph makes those effects
//! **inferred from the implementation** — the alternative was
//! hand-classifying 216 stdlib methods into the registry, which is
//! exactly the kind of transcription that drifts out of sync.
//!
//! Analysis-only: these items are never added to the *typecheck*
//! bundle, so this cannot introduce diagnostics against stdlib
//! source itself. Roots (annotated fns) are still collected from
//! user programs alone; the stdlib only ever appears as callee
//! bodies reached from a user root.

use std::sync::OnceLock;

use hale_syntax::ast::Program;

static PARSED: OnceLock<Option<Program>> = OnceLock::new();

/// The parsed Hale-source stdlib, or `None` if it fails to parse.
///
/// A parse failure here is a compiler bug, but it must not take the
/// user's build down: the analyzer degrades to the pre-existing
/// behaviour (stdlib bodies invisible) rather than refusing to
/// check the program. `stdlib_bodies_parse` in the test suite is
/// what turns that silent degradation into a red build.
pub fn program() -> Option<&'static Program> {
    PARSED
        .get_or_init(|| hale_syntax::parse_source(hale_stdlib::AP_SOURCE).ok())
        .as_ref()
}

/// Summarize the user programs **plus** the Hale-source stdlib, so
/// the callgraph can walk into stdlib locus methods. Every effect
/// query should build its summary through here; using
/// `summarize_programs` directly reintroduces the blind spot.
pub fn summarize_with_stdlib(
    programs: &[&Program],
) -> crate::alloc_summary::AllocSummary {
    summarize_with_stdlib_and_renames(programs, &[])
}

/// Same, additionally resolving cross-seed `alias::name` calls.
pub fn summarize_with_stdlib_and_renames(
    programs: &[&Program],
    import_renames: &[(Vec<String>, String)],
) -> crate::alloc_summary::AllocSummary {
    let mut all: Vec<&Program> = programs.to_vec();
    if let Some(std_prog) = program() {
        all.push(std_prog);
    }
    crate::alloc_summary::summarize_programs_with_renames(&all, import_renames)
}

/// `["std","io","file","File"]` → `"__StdIoFileFile"`, the mangled
/// name the bodies actually declare. Struct-literal paths in user
/// code are written in the public spelling, so resolving a
/// handle-method call needs this hop.
pub fn mangled_locus_name(segs: &[&str]) -> Option<&'static str> {
    hale_stdlib::PATH_RENAMES
        .iter()
        .find(|(path, _)| *path == segs)
        .map(|(_, name)| *name)
}

/// Rewrite merged cross-seed symbols back to the spelling the user
/// wrote (`__lib_foo_bar_Baz` -> `alias::Baz`).
///
/// A diagnostic naming a mangled symbol points at something that
/// appears nowhere in their source and cannot be searched for.
/// Longest-mangled-first so a symbol that is a prefix of another
/// cannot partially rewrite it.
pub fn demangle_imports(
    diags: &mut [hale_syntax::Diag],
    import_renames: &[(Vec<String>, String)],
) {
    if import_renames.is_empty() {
        return;
    }
    let mut table: Vec<(&str, String)> = import_renames
        .iter()
        .map(|(segs, mangled)| (mangled.as_str(), segs.join("::")))
        .collect();
    table.sort_by_key(|(m, _)| std::cmp::Reverse(m.len()));
    for d in diags.iter_mut() {
        for (mangled, public) in &table {
            if d.message.contains(mangled) {
                d.message = d.message.replace(mangled, public);
            }
        }
    }
}
