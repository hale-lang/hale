//! One source of Hale programs for every corpus-wide property test.
//!
//! ## The dark matter
//!
//! Corpus properties — `fmt` idempotence, effect-classification
//! totality, the parse sweep, the ASan oracle — all walked
//! `tests/fixtures/examples` and the stdlib: **7,032 lines**. But the
//! test suite also carries **1,391 Hale programs embedded in Rust
//! string literals, 21,621 lines** — 3.1× more Hale than the on-disk
//! corpus, and none of it visible to any of those properties.
//!
//! That is where the interesting programs live. Fixtures are written
//! to be *examples*; the embedded programs are written to hit feature
//! intersections, edge cases, and regressions. Excluding them from
//! every property meant the properties held over the tidy code and
//! were untested against the gnarly code.
//!
//! Pointing one new property (frontier completeness) at this corpus
//! immediately surfaced a real soundness hole — `std::cli`,
//! `std::log` and `std::source` were reaching effectful calls with no
//! registry row.
//!
//! ## Why scraping is legitimate here
//!
//! `hale-syntax/tests/docs_snippets.rs` already extracts Hale out of
//! markdown and parses it, so pulling Hale out of a non-`.hl` host
//! file is an established pattern in this repo. Extraction is also
//! verified rather than assumed: [`embedded`] skips templates that
//! need interpolation, and `corpus_extraction.rs` pins the parse rate
//! at 100% of what it yields.
//!
//! Programs are NOT moved to disk. Keeping them next to their
//! assertions is the right call — this crate just makes them
//! *reachable*.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A Hale program with enough provenance to name it in a failure.
#[derive(Clone, Debug)]
pub struct Program {
    /// Where it came from: a path for on-disk fixtures, or
    /// `file.rs#N` for the Nth literal scraped from a test file.
    pub origin: String,
    pub source: String,
}

/// Walk up from this crate to the workspace root.
pub fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // root
    p
}

fn collect_hl(dir: &Path, out: &mut Vec<Program>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_hl(&p, out);
        } else if p.extension().is_some_and(|e| e == "hl") {
            if let Ok(source) = std::fs::read_to_string(&p) {
                let origin = p
                    .strip_prefix(repo_root())
                    .unwrap_or(&p)
                    .display()
                    .to_string();
                out.push(Program { origin, source });
            }
        }
    }
}

/// Every `.hl` file on disk: the example corpus, the CLI fixtures,
/// and the Hale-source stdlib.
pub fn fixtures() -> Vec<Program> {
    let root = repo_root();
    let mut out = Vec::new();
    for rel in [
        "crates/hale-codegen/tests/fixtures/examples",
        "crates/hale-cli/tests/fixtures",
        // The Hale-source stdlib moved out of hale-codegen when it
        // was hoisted upstream for the effect analysis; probe both so
        // this crate works either side of that change.
        "crates/hale-stdlib/hl",
        "crates/hale-codegen/runtime/stdlib",
    ] {
        let dir = root.join(rel);
        if dir.is_dir() {
            collect_hl(&dir, &mut out);
        }
    }
    out
}

/// Hale programs embedded in Rust test sources as `r#"…"#` literals.
///
/// Two filters keep this honest:
///
///   * only literals that *look like a program* (a top-level `fn
///     main`, `locus`, or `main locus`) — the suite is full of
///     expected-output strings and fragments;
///   * `format!` templates whose placeholders are still unsubstituted
///     are skipped, since their text is not valid Hale. Doubled
///     braces (`{{`) ARE unescaped — that is just `format!` quoting a
///     literal brace, and skipping those would drop 139 real
///     programs.
pub fn embedded() -> Vec<Program> {
    let mut out = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rs(&repo_root().join("crates"), &mut files);
    files.sort();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let name = f
            .strip_prefix(repo_root())
            .unwrap_or(&f)
            .display()
            .to_string();
        for (i, (raw, is_format)) in
            raw_string_literals(&text).into_iter().enumerate()
        {
            let Some(src) = program_like(&raw, is_format) else { continue };
            out.push(Program { origin: format!("{}#{}", name, i), source: src });
        }
    }
    out
}

/// Fixtures + embedded, deduplicated by source text.
///
/// This includes NEGATIVE fixtures — programs written to assert a
/// specific diagnostic (`where bogus_constraint`, `"\xff"`, a
/// `bindings` block in a non-main locus). They are a real and
/// intentional part of the suite, so a property that needs valid
/// input should use [`parseable`] rather than assume every program
/// here compiles.
pub fn all() -> Vec<Program> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for p in fixtures().into_iter().chain(embedded()) {
        if seen.insert(p.source.clone()) {
            out.push(p);
        }
    }
    out
}

/// [`all`] minus the programs that don't parse.
///
/// The caller supplies the parse so this crate stays dependency-free
/// (it sits upstream of `hale-syntax` in the dev-dependency graph and
/// is used by it).
pub fn parseable<F>(parses: F) -> Vec<Program>
where
    F: Fn(&str) -> bool,
{
    all().into_iter().filter(|p| parses(&p.source)).collect()
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            // `target/` is build output and `.claude/` holds agent
            // worktrees — both would double-count the whole tree.
            let skip = p
                .file_name()
                .map(|n| n == "target" || n == ".claude")
                .unwrap_or(false);
            if !skip {
                collect_rs(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "rs")
            && p.components().any(|c| c.as_os_str() == "tests")
        {
            out.push(p);
        }
    }
}

/// Each literal, plus whether it is an argument to `format!`.
///
/// That flag matters: `{{` means two different things depending on
/// the host. In a `format!` template it is an escaped brace to be
/// unescaped. In a plain literal it is *Hale* source — an f-string
/// writing a literal brace, `f"json={{\"k\": 1}}"`. Unescaping the
/// second corrupts the program. Detecting the host precisely beats
/// guessing from the braces, which is what the first cut did.
fn raw_string_literals(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = text[i..].find("r#\"") {
        let lit_start = i + rel;
        let start = lit_start + 3;
        let Some(end_rel) = text[start..].find("\"#") else { break };
        // Look back over whitespace for a `format!(` opening.
        let head = text[..lit_start].trim_end();
        let is_format = head.ends_with("format!(");
        out.push((text[start..start + end_rel].to_string(), is_format));
        i = start + end_rel + 2;
    }
    out
}

/// `Some(source)` if this literal is a Hale program we can hand to a
/// parser as-is.
fn program_like(raw: &str, is_format_template: bool) -> Option<String> {
    let looks_like = raw.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("fn main") || t.starts_with("locus ") || t.starts_with("main locus")
    });
    if !looks_like {
        return None;
    }
    if is_format_template && (raw.contains("{{") || raw.contains("}}")) {
        // Distinguish an escaped brace from a live placeholder:
        // blank the escapes, and if a `{…}` still remains the text is
        // a template awaiting values, not a program.
        let probe = raw.replace("{{", "\u{0}").replace("}}", "\u{1}");
        let mut chars = probe.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '{' {
                for d in chars.by_ref() {
                    if d == '}' {
                        return None;
                    }
                    if d == '\n' {
                        break;
                    }
                }
            }
        }
        return Some(probe.replace('\u{0}', "{").replace('\u{1}', "}"));
    }
    Some(raw.to_string())
}
