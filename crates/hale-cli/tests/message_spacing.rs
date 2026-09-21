//! GH #906: no user-facing message carries a run of spaces
//! mid-sentence.
//!
//! A multi-line string literal that is reflowed onto one line without
//! collapsing the continuation indent keeps that indent INSIDE the
//! string, so the message a user reads comes out as
//! ``primitive `Bytes` as a generic argument                (v0
//! supports Int / …)``. A dozen of these had accumulated across the
//! diagnostic surface — the compiler's most-read text — and nothing
//! could see them, because the source looks fine and only the
//! rendered message is wrong.
//!
//! The rule this pins is the defect's own shape: a run of **three or
//! more spaces between two non-space characters on one line of a
//! string literal**. Leading and trailing runs are indentation, which
//! every renderer in the tree builds deliberately (`"    "`,
//! `"        "` — 400+ of them), and a run adjacent to a newline
//! inside the literal is the same thing one level in.
//!
//! Deliberate MID-line alignment does exist: the `usage()` verb
//! tables, `hale dna`'s `kept    <path>` outcome column, the
//! `[import]` trace. Those are allow-listed by (file, enclosing fn)
//! with a reason each — never by loosening the rule, because the rule
//! is what a future reflow has to trip over. An allow-listed region
//! that no longer has an aligned literal fails too, so the list
//! cannot rot into a blanket exemption.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The regions whose literals align columns on purpose.
/// `(crate-relative path, enclosing fn, why)`.
const ALIGNED: &[(&str, &str, &str)] = &[
    (
        "crates/hale-cli/src/dna.rs",
        "init",
        "the `kept    <path>` / `cut     <path>` outcome column, and \
         the `hale check --matrix <dir>   # …` next-step lines",
    ),
    (
        "crates/hale-cli/src/dna.rs",
        "owners_text",
        "the generated `dna/org/owners` file's own comment header",
    ),
    (
        "crates/hale-cli/src/dna.rs",
        "report",
        "the `found   <path>` outcome column",
    ),
    (
        "crates/hale-cli/src/dna.rs",
        "upgrade",
        "the `note    <path>` outcome column",
    ),
    (
        "crates/hale-cli/src/dna.rs",
        "usage",
        "the `hale dna <verb>` table: verb column, then description",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "check_usage",
        "the `hale check` flag table: flag column, then description",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "fleet_usage",
        "the `hale fleet` verb table",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "main",
        "`hale init`'s closing next-step table (`hale run <dir>      # \
         compile + run`) and its `kept    <path>` outcome column",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "resolve_imports",
        "the `[import]` trace's aligned phase column \
         (HALE_IMPORT_DEBUG=1)",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "run_bench",
        "the bench result table (`{:<40} {:>12} iters …`)",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "run_fleet",
        "the `hale fleet` sub-usage lines, each a command column then \
         its description",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "run_test",
        "the `ok   <path>` result column, aligned with `FAIL <path>`",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "subcommand_help",
        "`hale <command> --help`: each entry is a usage line whose \
         description starts at a fixed column",
    ),
    (
        "crates/hale-cli/src/main.rs",
        "usage",
        "the top-level command table",
    ),
    (
        "crates/hale-cli/src/topology_graph.rs",
        "usage_text",
        "the `hale topology graph` flag table",
    ),
    (
        "crates/hale-codegen/src/target.rs",
        "describe_from",
        "the target description's aligned `arch: … os: …` columns",
    ),
    (
        "crates/hale-types/src/resource_budget.rs",
        "render",
        "the budget report's label column (`bus subjects:   <n>`)",
    ),
];

/// The smallest run this refuses. Two spaces is a sentence break some
/// people still type; three is a column.
const RUN: usize = 3;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // repo root
    p
}

/// Every `.rs` file under `crates/*/src`, repo-relative path and text.
fn crate_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let mut out = Vec::new();
    let Ok(crates) = std::fs::read_dir(root.join("crates")) else {
        return out;
    };
    let mut roots: Vec<PathBuf> = crates
        .flatten()
        .map(|e| e.path().join("src"))
        .filter(|p| p.is_dir())
        .collect();
    roots.sort();
    for r in roots {
        walk(&r, &root, &mut out);
    }
    out.sort();
    out
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk(&p, root, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            if let Ok(t) = std::fs::read_to_string(&p) {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .to_string();
                out.push((rel, t));
            }
        }
    }
}

/// One offending literal.
#[derive(Debug)]
struct Hit {
    file: String,
    line: usize,
    func: String,
    text: String,
}

/// The name of the `fn` each line is inside, by "the last `fn NAME`
/// seen at or above it". Crude on purpose: it has to agree with what
/// a reader looking at the file would say, not with the AST.
fn enclosing_fns(text: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(text.lines().count() + 1);
    let mut cur = String::from("<top>");
    out.push(cur.clone()); // index 0 unused; lines are 1-based
    for l in text.lines() {
        let t = l.trim_start();
        let t = t.strip_prefix("pub ").unwrap_or(t);
        let t = match t.find(") ") {
            // `pub(crate) fn …`
            Some(i) if t.starts_with("pub(") => &t[i + 2..],
            _ => t,
        };
        let t = t.strip_prefix("const ").unwrap_or(t);
        let t = t.strip_prefix("async ").unwrap_or(t);
        let t = t.strip_prefix("unsafe ").unwrap_or(t);
        if let Some(rest) = t.strip_prefix("fn ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                cur = name;
            }
        }
        out.push(cur.clone());
    }
    out
}

/// Walk `text` and yield `(line, value)` for every ordinary (non-raw)
/// string literal, with escapes resolved — so a `\` line continuation,
/// which eats the newline AND the indent after it, contributes
/// nothing, exactly as it does at run time. Raw strings are skipped:
/// they hold embedded `.hl` fixtures and generated source, where
/// column alignment is the payload rather than a message.
fn string_literals(text: &str) -> Vec<(usize, String)> {
    let b: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && b.get(i + 1) == Some(&'*') {
            let mut depth = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == '\n' {
                    line += 1;
                }
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    continue;
                }
                i += 1;
            }
            continue;
        }
        // A char literal (`'x'`, `'\n'`) — a lifetime has no closing
        // quote and falls through harmlessly.
        if c == '\'' {
            if b.get(i + 1) == Some(&'\\') && b.get(i + 3) == Some(&'\'') {
                i += 4;
                continue;
            }
            if b.get(i + 2) == Some(&'\'') {
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }
        // A raw string: r"…", r#"…"#, br#"…"#.
        if c == 'r' || (c == 'b' && b.get(i + 1) == Some(&'r')) {
            let mut j = if c == 'b' { i + 2 } else { i + 1 };
            let hashes = {
                let start = j;
                while b.get(j) == Some(&'#') {
                    j += 1;
                }
                j - start
            };
            if b.get(j) == Some(&'"') {
                // Not a raw string if `r` is part of an identifier.
                let prev_ident = i > 0
                    && (b[i - 1].is_alphanumeric() || b[i - 1] == '_');
                if !prev_ident {
                    j += 1;
                    let mut close = String::from("\"");
                    for _ in 0..hashes {
                        close.push('#');
                    }
                    let tail: String = b[j..].iter().collect();
                    // `find` answers in BYTES and these files hold
                    // multi-byte text (every em dash in a message), so
                    // the offset has to come back as a CHAR count or
                    // every line number after the first raw string
                    // drifts.
                    let end = tail
                        .find(&close)
                        .map(|k| j + tail[..k].chars().count())
                        .unwrap_or(b.len());
                    for k in j..end.min(b.len()) {
                        if b[k] == '\n' {
                            line += 1;
                        }
                    }
                    i = (end + close.len()).min(b.len());
                    continue;
                }
            }
        }
        if c == '"' {
            let start_line = line;
            let mut j = i + 1;
            let mut val = String::new();
            while j < b.len() {
                match b[j] {
                    '\\' => {
                        match b.get(j + 1) {
                            // Line continuation: the newline and the
                            // indentation that follows it are not in
                            // the string.
                            Some('\n') => {
                                line += 1;
                                j += 2;
                                while matches!(b.get(j), Some(' ') | Some('\t')) {
                                    j += 1;
                                }
                            }
                            Some('n') => {
                                val.push('\n');
                                j += 2;
                            }
                            Some(other) => {
                                val.push(*other);
                                j += 2;
                            }
                            None => j += 1,
                        }
                    }
                    '"' => break,
                    ch => {
                        if ch == '\n' {
                            line += 1;
                        }
                        val.push(ch);
                        j += 1;
                    }
                }
            }
            out.push((start_line, val));
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// A run of `RUN`+ spaces with a non-space on BOTH sides, within one
/// line of the literal.
fn has_mid_line_run(s: &str) -> bool {
    for l in s.split('\n') {
        let ch: Vec<char> = l.chars().collect();
        let mut i = 0;
        while i < ch.len() {
            if ch[i] != ' ' {
                i += 1;
                continue;
            }
            let start = i;
            while i < ch.len() && ch[i] == ' ' {
                i += 1;
            }
            if i - start >= RUN && start > 0 && i < ch.len() {
                return true;
            }
        }
    }
    false
}

fn scan() -> Vec<Hit> {
    let mut hits = Vec::new();
    for (file, text) in crate_sources() {
        let fns = enclosing_fns(&text);
        for (line, val) in string_literals(&text) {
            if !has_mid_line_run(&val) {
                continue;
            }
            hits.push(Hit {
                file: file.clone(),
                line,
                func: fns.get(line).cloned().unwrap_or_default(),
                text: val.chars().take(90).collect(),
            });
        }
    }
    hits
}

#[test]
fn no_message_carries_a_run_of_spaces() {
    let allowed: BTreeSet<(&str, &str)> =
        ALIGNED.iter().map(|(f, n, _)| (*f, *n)).collect();
    let hits = scan();
    let mut offenders: Vec<&Hit> = Vec::new();
    for h in &hits {
        if !allowed.contains(&(h.file.as_str(), h.func.as_str())) {
            offenders.push(h);
        }
    }
    assert!(
        offenders.is_empty(),
        "these string literals carry {}+ spaces mid-sentence ({} \
         found) — a multi-line literal reflowed onto one line without \
         collapsing its continuation indent:\n{}\n\n\
         Collapse the run to one space (a `\\` line continuation at \
         the end of a source line eats the newline and the indent \
         after it, so it is the way to keep the source wrapped). If \
         the literal aligns a COLUMN on purpose, add its (file, fn) \
         to `ALIGNED` in this file with the reason.",
        RUN,
        offenders.len(),
        offenders
            .iter()
            .map(|h| format!("  {}:{} (fn {}): {:?}", h.file, h.line, h.func, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// An allow-list entry that no longer has an aligned literal is a
/// blanket exemption waiting to hide the next reflow.
#[test]
fn every_allow_list_entry_is_still_earning_it() {
    let hits = scan();
    let live: BTreeSet<(&str, &str)> = hits
        .iter()
        .map(|h| (h.file.as_str(), h.func.as_str()))
        .collect();
    let stale: Vec<String> = ALIGNED
        .iter()
        .filter(|(f, n, _)| !live.contains(&(*f, *n)))
        .map(|(f, n, why)| format!("  {} :: {} — {}", f, n, why))
        .collect();
    assert!(
        stale.is_empty(),
        "these `ALIGNED` entries no longer match any literal ({} \
         found):\n{}\n\nDelete them: the rule is stricter without \
         them, and a name that matches nothing is an exemption \
         nobody is checking.",
        stale.len(),
        stale.join("\n")
    );
}

/// The scan has to be able to SEE a defect, and has to be looking at
/// the whole tree. A parser that silently matches nothing would make
/// the test above pass over any amount of drift.
#[test]
fn the_scan_is_not_vacuous() {
    let srcs = crate_sources();
    assert!(
        srcs.len() > 100,
        "expected to scan every crate's src/, saw {} files",
        srcs.len()
    );
    let literals: usize =
        srcs.iter().map(|(_, t)| string_literals(t).len()).sum();
    assert!(
        literals > 10_000,
        "only {} string literals parsed out of {} files — the \
         literal scanner is not seeing the tree it thinks it is",
        literals,
        srcs.len()
    );
    // The aligned tables are what the allow-list is for; if the scan
    // stops finding them, it has stopped finding anything.
    assert!(
        !scan().is_empty(),
        "the scan found not one mid-line run, not even the \
         allow-listed usage tables"
    );

    // The defect, synthesized: each of these must be caught, and the
    // well-formed spellings beside them must not be.
    for bad in [
        "as a generic argument                (v0 supports Int)",
        "a: 1   b: 2",
        "one\ntwo    three",
    ] {
        assert!(has_mid_line_run(bad), "missed the run in {:?}", bad);
    }
    for good in [
        "as a generic argument (v0 supports Int)",
        "    indented, which is not a message defect",
        "trailing run is not one either    ",
        "one\n        two",
        "two  spaces is a sentence break",
    ] {
        assert!(!has_mid_line_run(good), "false positive on {:?}", good);
    }

    // And the literal parser itself: a `\` continuation contributes
    // neither the newline nor the indent that follows it.
    let src = "fn f() { let s = \"a \\\n        b\"; }";
    let lits = string_literals(src);
    assert_eq!(lits.len(), 1, "{:?}", lits);
    assert_eq!(lits[0].1, "a b", "the continuation's indent is not in \
         the string: {:?}", lits);
}
