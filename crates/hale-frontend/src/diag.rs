use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::imports::ImportDiag;

pub fn render_located(
    d: &hale_syntax::Diag,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> String {
    // A stdlib-origin primary has no seed file to be rendered in —
    // the witness leaf of a violated effect assertion is the usual
    // one. It reads as a note beside the finding it belongs to,
    // which the emitter pushed immediately before it.
    if d.origin == hale_syntax::SpanOrigin::Stdlib {
        return format!(
            "    note: {} ({})",
            d.message,
            stdlib_note(d.span)
        );
    }
    let off = d.span.start.as_usize() as u32;
    for (base, path, len) in file_bases {
        if hale_syntax::file_owns_offset(*base, *len, off) {
            if let Some(src) = sources.get(path) {
                let mut out =
                    d.render_located(&diag_file_name(path), src, *base);
                // Secondary locations, each resolved through the
                // file table — a related span may live in a
                // DIFFERENT file than the primary, or (GH #856) in
                // no file of the seed at all.
                for r in &d.related {
                    if r.origin == hale_syntax::SpanOrigin::Stdlib {
                        out.push_str(&format!(
                            "\n    note: {} ({})",
                            r.label,
                            stdlib_note(r.span)
                        ));
                        continue;
                    }
                    if let Some((rf, rl, rc)) =
                        locate_span(r.span, file_bases, sources)
                    {
                        out.push_str(&format!(
                            "\n    note: {} at {}:{}:{}",
                            r.label, rf, rl, rc
                        ));
                    }
                }
                return out;
            }
        }
    }
    // A span in the api binding's own parse space (GH #1109): the
    // source was synthesized from the `api:` entry, so no file of the
    // bundle owns it; say that, rather than a position in whichever
    // file happens to be listed first.
    if off >= hale_syntax::api_gen::API_SYNTH_BASE {
        return format!(
            "{}: {} (in the api binding synthesized from the `api:` entry; `hale check --dump-api` shows what it serves)",
            d.kind_str(),
            d.message
        );
    }
    let any = sources.values().next().map(|s| s.as_str()).unwrap_or("");
    d.render(any)
}

/// One NDJSON diagnostic line for `hale check --json`.
pub fn render_diag_json(
    d: &hale_syntax::Diag,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> String {
    fn esc(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 8);
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32))
                }
                c => out.push(c),
            }
        }
        out
    }
    let off = d.span.start.as_usize() as u32;
    let mut file = String::new();
    let mut line = 0usize;
    let mut col = 0usize;
    // GH #856: a stdlib-origin span is not an offset into any file
    // of this seed, so the windows are not consulted for it at all.
    // The record keeps the positionless shape (`""`, 0, 0) an
    // unplaceable finding has always had, and the stdlib file and
    // line ride in the message as a note.
    let in_stdlib = d.origin == hale_syntax::SpanOrigin::Stdlib;
    if !in_stdlib {
        for (base, path, len) in file_bases {
            if hale_syntax::file_owns_offset(*base, *len, off) {
                if let Some(src) = sources.get(path) {
                    let (l, c) = d
                        .span
                        .shifted(base.wrapping_neg())
                        .line_col(src);
                    // GH #822: the `file` field is the join key
                    // downstream tooling matches on, so it is the
                    // same string the text renderer prints.
                    file = diag_file_name(path);
                    line = l;
                    col = c;
                }
                break;
            }
        }
    }
    let message = if in_stdlib {
        format!("{} ({})", d.message, stdlib_note(d.span))
    } else {
        d.message.clone()
    };
    let severity = if d.is_error() { "error" } else { "warning" };
    // Secondary locations ride along as a `related` array (absent
    // when empty, so existing consumers see an unchanged shape).
    let related = if d.related.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = d
            .related
            .iter()
            .filter_map(|r| {
                // A stdlib-origin secondary has no seed position to
                // put in `file`/`line`/`col`, and a consumer joining
                // on those fields must not be handed a seed file
                // that has nothing to do with it (GH #856): the
                // entry carries the note and no location.
                if r.origin == hale_syntax::SpanOrigin::Stdlib {
                    return Some(format!(
                        "{{\"file\":\"\",\"line\":0,\"col\":0,\"note\":\"{}\"}}",
                        esc(&format!(
                            "{} ({})",
                            r.label,
                            stdlib_note(r.span)
                        ))
                    ));
                }
                let (rf, rl, rc) =
                    locate_span(r.span, file_bases, sources)?;
                Some(format!(
                    "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"note\":\"{}\"}}",
                    esc(&rf),
                    rl,
                    rc,
                    esc(&r.label)
                ))
            })
            .collect();
        if entries.is_empty() {
            String::new()
        } else {
            format!(",\"related\":[{}]", entries.join(","))
        }
    };
    render_json_record(
        &file,
        line,
        col,
        severity,
        d.kind_str(),
        &message,
        &related,
    )
}

/// The one NDJSON record writer for `hale check --json` and
/// `hale verify --json`.
///
/// Both producers format here: a `Diag` through [`render_diag_json`],
/// which resolves its position against the file windows first, and an
/// unreadable input through [`IoDiag::record`] (GH #806), which has
/// no position and passes `0, 0`. One writer is the point — the
/// record shape is a consumed contract (`spec/projects.md`, the
/// `hale check --json` row), and a second `format!` of it somewhere
/// else is how the two drift.
///
/// `related` is already-formatted JSON, `,"related":[…]` or empty.
pub fn render_json_record(
    file: &str,
    line: usize,
    col: usize,
    severity: &str,
    kind: &str,
    message: &str,
    related: &str,
) -> String {
    format!(
        "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"severity\":\"{}\",\"kind\":\"{}\",\"message\":\"{}\"{}}}",
        json_escape(file),
        line,
        col,
        severity,
        json_escape(kind),
        json_escape(message),
        related
    )
}

/// GH #736: `hale check --flows` — every flow type, and the `release`
/// clauses that make it one, in the spelling the author wrote and at the
/// file and line each sits.
pub fn render_flows(
    flows: &[hale_types::flows::Flow],
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
    import_renames: &[(Vec<String>, String)],
) -> String {
    let demangler = hale_types::stdlib_bodies::Demangler::new(import_renames);
    let spell = |s: &str| demangler.demangle(s);
    if flows.is_empty() {
        return "flows: none — every accept'd child is a resident: it ends by its own `terminate;` or in its owner's dissolve cascade\n".to_string();
    }
    let mut out = format!(
        "flows: {} locus type(s) reclaimed when their run() completes — a `release(c: T)` anywhere in the program, imported seeds included, makes every T a flow, whether or not its declaring locus is instantiated:\n",
        flows.len()
    );
    // sorted as printed, not by the mangled key
    let mut shown: Vec<&hale_types::flows::Flow> = flows.iter().collect();
    shown.sort_by_key(|f| spell(&f.child));
    for f in shown {
        out.push_str(&format!("\n  {} — a flow, by:\n", spell(&f.child)));
        for c in &f.clauses {
            let at = locate_span(c.span, file_bases, sources)
                .map(|(path, l, col)| format!("{path}:{l}:{col}"))
                .unwrap_or_else(|| "(no seed location)".to_string());
            out.push_str(&format!(
                "    release({}: {}) in {}  {}\n",
                c.param,
                spell(&f.child),
                spell(&c.owner),
                at
            ));
        }
    }
    out.push_str("\nA resident's run() returning means \"ready\"; it lives on, ends by its own `terminate;` or with its owner's dissolve cascade. Dropping one owner's hook leaves T a flow while any clause above remains.\n");
    out
}

/// Render a post-merge diagnostic, demultiplexing its (globally-unique,
/// `parse_source_at`-shifted) span back to the file it came from via
/// `file_bases`, so the output reads `path:line:col` against that file's
/// own source instead of an arbitrary file. Falls back to the entry
/// source if the span isn't in any known file range.
/// Resolve a merged-bundle span to `(path, line, col)` via the
/// file-base table. Shared by the text and JSON renderers for both
/// primary and related spans.
///
/// The path comes out through [`diag_file_name`] (GH #822), so a
/// related location — the clickable second location of a two-place
/// diagnostic, in `--json` and in the `note:` line alike — is spelled
/// the way the primary is.
pub fn locate_span(
    span: hale_syntax::Span,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> Option<(String, usize, usize)> {
    let off = span.start.as_usize() as u32;
    for (base, path, len) in file_bases {
        if hale_syntax::file_owns_offset(*base, *len, off) {
            let src = sources.get(path)?;
            let (l, c) = span.shifted(base.wrapping_neg()).line_col(src);
            return Some((diag_file_name(path), l, c));
        }
    }
    None
}

/// GH #822: the one spelling a diagnostic names its file by —
/// absolute, symlinks resolved, no `.` or `..` component.
///
/// The same file used to have two names depending on which channel
/// reported it. `check` resolves an imported file's diagnostic
/// through `file_bases`, whose entries `resolve_imports` records
/// CANONICALLY (the `visited` set is keyed that way, so a cycle is
/// broken however many aliases reach the lib), and printed
/// `/abs/lib/second.hl`. `build`, `run` and `test` render the same
/// diagnostic from [`ImportDiag`], which carries the path as the
/// resolver REACHED it, and printed `/abs/app/../lib/second.hl`.
/// The target's OWN files are a third spelling again: `parse_files`
/// records them exactly as the command line spelled them, so `hale
/// check ../lib/second.hl` named a file relative to a directory the
/// reader has to already know. Every one of them names the right
/// file and every one is clickable — but none of them is a JOIN KEY,
/// which is what `check --json`'s `file` field is used as: a gate
/// diffing `--json` against a build failure, or an editor matching a
/// build error to an open buffer, compared two strings for one file.
///
/// The rule lives at the rendering boundary rather than at each site
/// that records a path, because that is the one thing every channel
/// does and nothing else does. `file_bases` entries are also the KEY
/// `sources` and the program map are looked up by, and the entry
/// path is the base the replay digest's logical source ids are taken
/// relative to; rewriting those to fix a display string would move
/// four things to fix one. Every renderer below turns a `&Path` into
/// the string the user and the tool read through here, so a new one
/// cannot quietly reintroduce a second spelling.
pub fn diag_file_name(path: &Path) -> String {
    if let Ok(canon) = path.canonicalize() {
        return canon.display().to_string();
    }
    // A file that is not on disk cannot be asked of the filesystem —
    // a target that does not exist, or one deleted between the read
    // and the report. Canonicalize the deepest ancestor that DOES
    // exist and re-join what is left: the `..` in
    // `app/../lib/gone.hl` is then resolved by the parent's
    // canonicalization, which is the only correct way to resolve one
    // (removing `..` lexically walks out of the wrong directory the
    // moment a symlink is involved).
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur: &Path = &absolute;
    while let (Some(name), Some(parent)) = (cur.file_name(), cur.parent()) {
        tail.push(name.to_os_string());
        if let Ok(canon) = parent.canonicalize() {
            let mut out = canon;
            for seg in tail.iter().rev() {
                out.push(seg);
            }
            return out.display().to_string();
        }
        cur = parent;
    }
    absolute.display().to_string()
}

/// Render `to` for someone reading a diagnostic about a file in
/// `from_dir`: relative when the two share a root, else as it is.
pub fn display_relative(from_dir: &Path, to: &Path) -> String {
    let from = from_dir
        .canonicalize()
        .unwrap_or_else(|_| from_dir.to_path_buf());
    let to = to.canonicalize().unwrap_or_else(|_| to.to_path_buf());
    let fc: Vec<_> = from.components().collect();
    let tc: Vec<_> = to.components().collect();
    let common = fc
        .iter()
        .zip(tc.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return to.display().to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..fc.len() {
        parts.push("..".to_string());
    }
    for c in &tc[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        return ".".to_string();
    }
    parts.join("/")
}

/// GH #856: the note a stdlib-origin span renders as, in place of a
/// `file:line:col` it has no right to. The embedded stdlib parses at
/// base 0 in its own space, so its offsets collide with the seed's:
/// past the seed's end the window test placed such a span nowhere,
/// and INSIDE a seed file's window it named that file at a position
/// belonging to neither. A stdlib span is never a seed location, and
/// the renderers say where it really is instead.
pub fn stdlib_note(span: hale_syntax::Span) -> String {
    hale_types::stdlib_bodies::stdlib_span_note(span)
}

/// Escape a string for embedding in a JSON string literal.
/// Shared by `hale test --json` (mirrors the private `esc` inside
/// `render_diag_json`).
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// GH #806: an input `check` and `verify` needed and could not READ.
///
/// Not a [`hale_syntax::Diag`]: there is no text to position it in —
/// the file never opened, or the target is not there at all. That is
/// why it travelled as a bare `eprintln!` plus a non-zero exit for as
/// long as it did, and why it was the last thing on the check path
/// still doing so: under `--json` the stream was EMPTY and the exit
/// was 1, the exact shape GH #777 retired for diagnostics, so a gate
/// reading the stream could not tell a missing target or an
/// unreadable file from a crash. It is a record like everything else
/// now, at `"line":0,"col":0` — no position, because there is no text
/// to have a position in — and the text channel prints exactly what
/// it always printed.
pub struct IoDiag {
    /// The path the failure is about: the target itself when the
    /// target cannot be read, otherwise the file that would not open.
    pub path: PathBuf,
    /// What the TEXT channel prints — byte for byte the line the
    /// `eprintln!` at the failing site printed before this existed.
    pub text: String,
    /// The record's `message`: the OS error, which is the part a
    /// consumer can act on.
    pub message: String,
}

impl IoDiag {
    /// A file that would not open. `text` is the failing site's own
    /// sentence — it names the importing seed, or the file, as it
    /// always did — and the record carries the OS error alone.
    pub fn read(path: &Path, err: &std::io::Error, text: String) -> Self {
        Self {
            path: path.to_path_buf(),
            text,
            message: err.to_string(),
        }
    }

    /// A target (or an import target) whose `.hl` files could not be
    /// collected. Those messages are prose — `not a file or
    /// directory: …` — and for the commonest case of all, a path that
    /// is simply not there, the OS has the better answer: ask it, and
    /// the record says `No such file or directory (os error 2)`. A
    /// target that DOES stat (an empty directory, a directory whose
    /// `read_dir` failed with an error already in the sentence) keeps
    /// the sentence.
    pub fn target(path: &Path, text: String) -> Self {
        let message = match fs::metadata(path) {
            Err(e) => e.to_string(),
            Ok(_) => text.clone(),
        };
        Self {
            path: path.to_path_buf(),
            text,
            message,
        }
    }

    /// The NDJSON record, through the one writer every `--json`
    /// record is formatted by.
    ///
    /// GH #822: `file` is spelled by the rule every other record's
    /// `file` is — [`diag_file_name`] — so a consumer joining on it
    /// does not have to know that this row came from a file that
    /// never opened. A path that is not on disk (the commonest case
    /// here: a target that is not there) still comes out absolute
    /// and `..`-free. The `text` channel is untouched: it is the
    /// sentence the failing site composed, byte for byte what it
    /// printed before it travelled.
    pub fn record(&self) -> String {
        render_json_record(
            &diag_file_name(&self.path),
            0,
            0,
            "error",
            "io error",
            &self.message,
            "",
        )
    }
}

/// Render every import diagnostic in `errors` to stderr, one per
/// line-group, and hand back the failing exit code.
pub fn report_import_diags(errors: &[ImportDiag]) -> ExitCode {
    for e in errors {
        eprintln!("{}", e.render());
    }
    ExitCode::from(1)
}
