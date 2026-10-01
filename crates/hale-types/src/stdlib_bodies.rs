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

use crate::snapshot::Snapshot;

/// The stdlib the analyses read, and its own identities: a copy of
/// [`crate::desugar_sequence::bundled_stdlib`] minted under
/// [`crate::snapshot::STDLIB_SEED`], once per process. A copy, because
/// the resolved program appends the bundled one to a user program and
/// mints the two together, which an id of the stdlib's own counter
/// would collide with; the analyses read the stdlib beside a bundle and
/// never merge it, so each side reads its own identities.
fn analysis() -> Option<&'static (Program, Snapshot)> {
    static MINTED: OnceLock<Option<(Program, Snapshot)>> = OnceLock::new();
    MINTED
        .get_or_init(|| {
            let mut stdlib = crate::desugar_sequence::bundled_stdlib().ok()?.clone();
            let ids = crate::snapshot::mint([(crate::snapshot::STDLIB_SEED, &mut stdlib)], &[]);
            Some((stdlib, ids))
        })
        .as_ref()
}

/// The parsed Hale-source stdlib, after the desugar sequence user
/// programs go through before their check — the program the resolved
/// program appends ([`crate::desugar_sequence::bundled_stdlib`]),
/// minted ([`identities`]) — or `None` if it fails to parse.
///
/// A parse failure here is a compiler bug, but it must not take the
/// user's build down: the analyzer degrades to the pre-existing
/// behaviour (stdlib bodies invisible) rather than refusing to
/// check the program. `stdlib_bodies_parse` in the test suite is
/// what turns that silent degradation into a red build.
pub fn program() -> Option<&'static Program> {
    analysis().map(|(p, _)| p)
}

/// The identities of [`program`]: which declaration each of its uses
/// names, for a walk over a stdlib body.
pub fn identities() -> Option<&'static Snapshot> {
    analysis().map(|(_, ids)| ids)
}

/// Summarize the user programs **plus** the Hale-source stdlib, so
/// the callgraph can walk into stdlib locus methods. Every effect
/// query should build its summary through here; using
/// `summarize_programs` directly reintroduces the blind spot. `ids`
/// are the identities the user programs were minted with.
pub fn summarize_with_stdlib(
    programs: &[&Program],
    ids: &Snapshot,
) -> crate::alloc_summary::AllocSummary {
    summarize_with_stdlib_and_renames(programs, ids, &[])
}

/// Same, additionally resolving cross-seed `alias::name` calls.
pub fn summarize_with_stdlib_and_renames(
    programs: &[&Program],
    ids: &Snapshot,
    import_renames: &[(Vec<String>, String)],
) -> crate::alloc_summary::AllocSummary {
    let mut all: Vec<(&Program, &Snapshot)> = programs.iter().map(|p| (*p, ids)).collect();
    if let Some((std_prog, std_ids)) = analysis() {
        all.push((std_prog, std_ids));
    }
    crate::alloc_summary::summarize_identified(&all, import_renames)
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
    // GH #436 follow-up: the stdlib table applies unconditionally.
    // This used to return early when a program had no imports, so a
    // diagnostic naming a Hale-source stdlib locus rendered its
    // MANGLED name — `__StdSecretSigner::sign` — a symbol that
    // appears nowhere in the author's program. Claims witnesses hit
    // this the moment `secret_use` became findable.
    // Only genuinely MANGLED spellings are rewritten. Some stdlib
    // renames alias a BARE name (`ParseError`, `IoError` — the
    // injected error types); substring-replacing those corrupts any
    // message that legitimately contains the word ("MyParseError" →
    // "Mystd::str::ParseError"). A `__` prefix is what makes a name
    // unspeakable in user source, and unspeakable names are the
    // entire reason this pass exists.
    let table = demangle_table(import_renames);
    for d in diags.iter_mut() {
        for (mangled, public) in &table {
            if d.message.contains(mangled.as_str()) {
                d.message = d.message.replace(mangled.as_str(), public);
            }
        }
    }
}

/// One path-rename key as the author wrote it: the head unscoped
/// (GH #746), the rest verbatim.
fn display_path(segs: &[String]) -> String {
    let mut out = String::new();
    for (i, seg) in segs.iter().enumerate() {
        if i > 0 {
            out.push_str("::");
        }
        if i == 0 {
            out.push_str(hale_syntax::ast::unscoped_alias(seg));
        } else {
            out.push_str(seg);
        }
    }
    out
}

/// The rename table `demangle_imports` applies (imports ∪ stdlib
/// PATH_RENAMES, `__`-prefixed only, longest-mangled-first) —
/// shared with the model builder's stdlib-absorption displays
/// (GH #476 Change 5a) so witness spellings cannot drift.
pub(crate) fn demangle_table(
    import_renames: &[(Vec<String>, String)],
) -> Vec<(String, String)> {
    let mut table: Vec<(String, String)> = import_renames
        .iter()
        // GH #746: a key's head may be a SCOPED alias (`u$0`); a
        // diagnostic shows the alias the author wrote.
        .map(|(segs, mangled)| (mangled.clone(), display_path(segs)))
        .chain(hale_stdlib::PATH_RENAMES.iter().map(
            |(segs, mangled)| (mangled.to_string(), segs.join("::")),
        ))
        .filter(|(mangled, _)| mangled.starts_with("__"))
        .collect();
    table.sort_by_key(|(m, _)| std::cmp::Reverse(m.len()));
    table
}

/// Demangle ONE string through the same table. Builds the table for
/// that one string: a caller with many strings builds it once with
/// `demangle_table` and goes through `demangle_with` (GH #1159 — the
/// model builder did this per absorbed edge, and the table was 92%
/// of `hale check` on a generated organization).
pub fn demangle_str(
    s: &str,
    import_renames: &[(Vec<String>, String)],
) -> String {
    demangle_with(s, &demangle_table(import_renames))
}

/// Demangle ONE string through a table `demangle_table` built.
pub(crate) fn demangle_with(s: &str, table: &[(String, String)]) -> String {
    let mut out = s.to_string();
    for (mangled, public) in table {
        if out.contains(mangled.as_str()) {
            out = out.replace(mangled.as_str(), public);
        }
    }
    out
}

/// Where a STDLIB-origin span sits, as `<file>:<line>:<col>` in the
/// embedded stdlib source (GH #856).
///
/// A span raised from a stdlib body is an offset into
/// `hale_stdlib::AP_SOURCE`, which parses at base 0 in a space of
/// its own. It is not a bundle offset, so no seed file's window may
/// be asked about it — but it IS a real position in a real file, and
/// the renderers name it instead of inventing a seed location: the
/// language server materializes those files to a read-only cache for
/// exactly this reason, so `io_tcp.hl:118` is something a reader can
/// open.
///
/// `None` when the offset falls outside the concatenation (a
/// synthetic span, or the separator byte between two files).
pub fn stdlib_span_location(span: hale_syntax::Span) -> Option<String> {
    let off = span.start.as_usize();
    let (name, content, local) = hale_stdlib::ap_file_at(off)?;
    let (line, col) =
        hale_syntax::Span::new(local, local).line_col(content);
    Some(format!("{}:{}:{}", name, line, col))
}

/// The note a renderer prints in place of a position for a
/// stdlib-origin span: the sentence every channel says, plus the
/// stdlib file and line when the offset resolves to one.
pub fn stdlib_span_note(span: hale_syntax::Span) -> String {
    match stdlib_span_location(span) {
        Some(at) => {
            format!("{}, {}", hale_syntax::Diag::STDLIB_NOTE, at)
        }
        None => hale_syntax::Diag::STDLIB_NOTE.to_string(),
    }
}
