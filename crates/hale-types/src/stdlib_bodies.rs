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

use std::collections::HashMap;
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
///
/// Builds the [`Demangler`] for this one call; a caller that demangles
/// again for the same table keeps one.
pub fn demangle_imports(
    diags: &mut [hale_syntax::Diag],
    import_renames: &[(Vec<String>, String)],
) {
    Demangler::new(import_renames).demangle_diags(diags);
}

/// One rename table's demangling, indexed (F.40 phase 3, X3): the table
/// [`demangle_imports`] applies, and an index from each mangled name to
/// its rows.
///
/// The table is applied as a scan applies it, row by row, longest
/// mangled name first, each row rewriting every occurrence of its
/// mangled name in the text as it stands after the rows before it. The
/// scan tested every row against every string (dna/host's 224 messages
/// against the 58,005 rows its table held before X3 1 of 3: 1.1 to
/// 3.5 s per editor publication). Every
/// mangled name starts `__`, so the rows a string contains are found
/// from the string instead: at each `__` in it, each length a mangled
/// name has, looked up in the index. The next row applied is the first
/// row, after the one applied last, whose name the text contains; the
/// rows between them are rows the scan would test against that same
/// text and skip. So a string comes out exactly as the scan leaves it,
/// a rewrite that makes a new occurrence (or a later row of the same
/// mangled name) included, at the price of its own length.
///
/// Shared with the model builder's stdlib-absorption displays (GH #476
/// Change 5a), so witness spellings cannot drift.
pub struct Demangler {
    /// `(mangled, public)`, `__`-prefixed only, longest mangled first,
    /// ties in table order: the order the rows apply in.
    table: Vec<(String, String)>,
    /// Each mangled name's rows, as positions in `table`, ascending.
    rows: HashMap<String, Vec<usize>>,
    /// The lengths the mangled names have, each once.
    lengths: Vec<usize>,
}

impl Demangler {
    /// The demangling of `import_renames` and the stdlib's
    /// `PATH_RENAMES`.
    pub fn new(import_renames: &[(Vec<String>, String)]) -> Demangler {
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
        Demangler::from_rows(
            import_renames
                .iter()
                // GH #746: a key's head may be a SCOPED alias (`u$0`); a
                // diagnostic shows the alias the author wrote.
                .map(|(segs, mangled)| (mangled.clone(), display_path(segs)))
                .chain(hale_stdlib::PATH_RENAMES.iter().map(
                    |(segs, mangled)| (mangled.to_string(), segs.join("::")),
                ))
                .filter(|(mangled, _)| mangled.starts_with("__"))
                .collect(),
        )
    }

    /// The demangling of `(mangled, public)` rows, each mangled name
    /// `__`-prefixed, applied longest first.
    fn from_rows(mut table: Vec<(String, String)>) -> Demangler {
        table.sort_by_key(|(m, _)| std::cmp::Reverse(m.len()));
        let mut rows: HashMap<String, Vec<usize>> = HashMap::new();
        for (at, (mangled, _)) in table.iter().enumerate() {
            rows.entry(mangled.clone()).or_default().push(at);
        }
        let mut lengths: Vec<usize> = table.iter().map(|(m, _)| m.len()).collect();
        lengths.dedup();
        Demangler { table, rows, lengths }
    }

    /// `s` in the author's spelling.
    pub fn demangle(&self, s: &str) -> String {
        let mut out = s.to_string();
        let mut from = 0;
        while let Some(at) = self.next_row(&out, from) {
            let (mangled, public) = &self.table[at];
            out = out.replace(mangled.as_str(), public);
            from = at + 1;
        }
        out
    }

    /// Every message of `diags` in the author's spelling.
    pub fn demangle_diags(&self, diags: &mut [hale_syntax::Diag]) {
        for d in diags.iter_mut() {
            if d.message.contains("__") {
                d.message = self.demangle(&d.message);
            }
        }
    }

    /// The first row at or after `from` whose mangled name `s` contains.
    fn next_row(&self, s: &str, from: usize) -> Option<usize> {
        let bytes = s.as_bytes();
        let mut next: Option<usize> = None;
        for start in 0..bytes.len().saturating_sub(1) {
            if bytes[start] != b'_' || bytes[start + 1] != b'_' {
                continue;
            }
            for &len in &self.lengths {
                let Some(name) = s.get(start..start + len) else { continue };
                let Some(rows) = self.rows.get(name) else { continue };
                let after = rows.partition_point(|&r| r < from);
                if let Some(&row) = rows.get(after) {
                    next = Some(next.map_or(row, |n| n.min(row)));
                }
            }
        }
        next
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

/// Demangle ONE string through the same table. Builds the
/// [`Demangler`] for that one string: a caller with many strings builds
/// it once (GH #1159 — the model builder built the table per absorbed
/// edge, and the table was 92% of `hale check` on a generated
/// organization).
pub fn demangle_str(
    s: &str,
    import_renames: &[(Vec<String>, String)],
) -> String {
    Demangler::new(import_renames).demangle(s)
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

#[cfg(test)]
mod tests {
    use super::Demangler;

    /// The scan the index replaced: every row, in order, tested against
    /// the text as the rows before it left it.
    fn scan(d: &Demangler, s: &str) -> String {
        let mut out = s.to_string();
        for (mangled, public) in &d.table {
            if out.contains(mangled.as_str()) {
                out = out.replace(mangled.as_str(), public);
            }
        }
        out
    }

    fn rows(rows: &[(&str, &str)]) -> Demangler {
        Demangler::from_rows(rows.iter().map(|(m, p)| (m.to_string(), p.to_string())).collect())
    }

    #[test]
    fn the_index_spells_what_the_scan_spells() {
        let cases: &[(&[(&str, &str)], &str, &str)] = &[
            // a name that is a prefix of another: the longer first
            (&[("__a_b", "x::b"), ("__a_bc", "x::bc")], "use __a_bc and __a_b", "use x::bc and x::b"),
            // one mangled name under two aliases: the first row's spelling
            (&[("__l_T", "a::T"), ("__l_T", "b::T")], "`__l_T`", "`a::T`"),
            // a rewrite that makes another row's name: applied, as the scan applies it
            (&[("__long_name", "__x"), ("__x", "done")], "see __long_name", "see done"),
            // two names of one length overlapping: the earlier row wins
            (&[("__b_cd", "B"), ("__a__b", "A")], "__a__b_cd", "__aB"),
            // Unicode before and inside a name keeps byte boundaries intact.
            (&[("__café", "café::T"), ("__😀", "emoji::T")], "é `__café` 😀 __😀", "é `café::T` 😀 emoji::T"),
            // A replacement can complete an adjacent later name.
            (&[("__long", "_"), ("__x", "done")], "__long_x", "done"),
            // a name the text does not hold, and a name recurring
            (&[("__q", "Q")], "nothing mangled", "nothing mangled"),
            (&[("__q", "Q")], "__q__q___q", "QQ_Q"),
        ];
        for (table, input, spelled) in cases {
            let d = rows(table);
            assert_eq!(scan(&d, input), *spelled, "the scan, over {input:?}");
            assert_eq!(d.demangle(input), *spelled, "the index, over {input:?}");
        }
    }

    /// Random tables and texts over a small alphabet, so names overlap,
    /// nest and recur: the index spells every text as the scan does.
    #[test]
    fn the_index_equals_the_scan_over_random_tables() {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        const ALPHABET: &[u8] = b"__ab:";
        fn word(next: &mut dyn FnMut(u64) -> u64, max: u64) -> String {
            (0..next(max) + 1).map(|_| ALPHABET[next(ALPHABET.len() as u64) as usize] as char).collect()
        }
        for _ in 0..3000 {
            let table: Vec<(String, String)> = (0..next(6) + 1)
                .map(|_| (format!("__{}", word(&mut next, 4)), word(&mut next, 5)))
                .collect();
            let d = Demangler::from_rows(table);
            for _ in 0..10 {
                let text = word(&mut next, 24);
                assert_eq!(d.demangle(&text), scan(&d, &text), "{:?} over {text:?}", d.table);
            }
        }
    }
}
