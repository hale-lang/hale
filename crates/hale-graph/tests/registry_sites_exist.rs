//! Every site the registry names exists in the tree.
//!
//! A registry that cites a function which was renamed or deleted is
//! worse than none: it tells the next contributor to read code that
//! is not there. So every `Site { path, symbol }` in every family,
//! legacy producer, owned helper, consumer, seam and rule evaluator is
//! checked against the workspace (the frozen Debug renderings are
//! matched by the guard's own scan).
//!
//! A definition site (a producer, an owned helper, a rule evaluator)
//! whose symbol is an identifier must be **defined** in its file: `fn NAME(`, `struct NAME`, `enum NAME`, `const NAME`,
//! `static NAME`, `type NAME` or `trait NAME`. A comment that still
//! mentions a deleted function does not keep its entry alive. A
//! symbol that is not an identifier is a text fragment (a region
//! inside a large function) and must appear verbatim. A legacy site
//! is often a call that re-runs a producer or a row constructor, so
//! its identifier must be used in code, not only in a comment: a `//`
//! comment and a `/* … */` block (nested, across lines) are not code,
//! and a comment marker inside a string (raw strings included) opens no
//! comment, so a comment that still names a removed call does not keep
//! its entry alive either.
//! Consumer and seam sites reference a symbol rather than define it,
//! so they are held to the verbatim rule. A family's focused tests are
//! paths, and each must be a file.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !s.chars().next().unwrap().is_ascii_digit()
}

fn defines(text: &str, name: &str) -> bool {
    let kinds = [
        "fn", "struct", "enum", "const", "static", "type", "trait", "mod",
    ];
    text.lines().any(|line| {
        let t = line.trim_start();
        kinds.iter().any(|k| {
            let Some(rest) = t
                .strip_prefix("pub ")
                .and_then(|r| r.strip_prefix(k))
                .or_else(|| {
                    t.strip_prefix("pub(crate) ")
                        .and_then(|r| r.strip_prefix(k))
                })
                .or_else(|| {
                    t.strip_prefix("pub(super) ")
                        .and_then(|r| r.strip_prefix(k))
                })
                .or_else(|| t.strip_prefix(k))
            else {
                return false;
            };
            let rest = rest.trim_start();
            rest.starts_with(name)
                && rest[name.len()..]
                    .chars()
                    .next()
                    .map(|c| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(true)
        })
    })
}

enum Rule {
    /// The symbol is defined here (a producer, an owned helper, a
    /// rule evaluator).
    Definition,
    /// The symbol is used here in code, not only in a comment (a
    /// legacy site is often a call that re-runs a producer, or a
    /// row constructor).
    CodeMention,
    /// The text appears verbatim (consumers, seams, Debug scans).
    Verbatim,
}

/// The file's code, scanned once: every comment (a `//` line, a
/// `/* … */` block nested to any depth and across lines) blanked to
/// spaces, newlines kept. String literals (ordinary, byte, and raw —
/// `r"…"`, `r#"…"#` with any number of `#`, `br…` too) are read whole,
/// so a comment marker inside one opens nothing; their contents stay,
/// because codegen names a runtime or mangled symbol in a string
/// (`"lotus_replay_start_ingress"`, `format!("__reclaim_{}", …)`) and
/// that is a use in code. A character literal is stepped over whole, so
/// the quote in `'"'` opens nothing; a `'` that closes no character is
/// a lifetime.
fn code_text(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for x in &mut out[from.min(b.len())..to.min(b.len())] {
            if *x != b'\n' {
                *x = b' ';
            }
        }
    };
    let ident = |i: usize| i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = b[i..].iter().position(|&x| x == b'\n').map_or(b.len(), |k| i + k);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut depth = 0usize;
                let mut j = i;
                while j < b.len() {
                    if b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
                        depth += 1;
                        j += 2;
                    } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                        depth -= 1;
                        j += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        j += 1;
                    }
                }
                blank(&mut out, i, j);
                i = j;
            }
            // a raw string: `r` (or `br`) starting a token, then `#`s and a quote
            b'r' if !ident(i) || (b[i - 1] == b'b' && !ident(i - 1)) => {
                let hashes = b[i + 1..].iter().take_while(|&&x| x == b'#').count();
                if b.get(i + 1 + hashes) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let body = i + 2 + hashes;
                let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
                let end = b[body..].windows(close.len()).position(|w| w == close.as_slice()).map_or(b.len(), |k| body + k);
                i = end + close.len();
            }
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                i = j + 1;
            }
            b'\'' => {
                if b.get(i + 1) == Some(&b'\\') {
                    // an escaped character literal, `'\''` among them
                    i += b.get(i + 3..).and_then(|r| r.iter().position(|&x| x == b'\'')).map_or(1, |k| 4 + k);
                } else {
                    let l = text[i + 1..].chars().next().map_or(0, char::len_utf8);
                    i += if l > 0 && b.get(i + 1 + l) == Some(&b'\'') { 2 + l } else { 1 };
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("blanking replaces whole characters with spaces")
}

fn mentioned_in_code(text: &str, name: &str) -> bool {
    let t = code_text(text);
    let t = t.as_str();
    let mut from = 0;
    while let Some(i) = t[from..].find(name) {
        let start = from + i;
        let end = start + name.len();
        let before_ok = start == 0
            || !t[..start]
                .chars()
                .last()
                .map(|c| c.is_ascii_alphanumeric() || c == '_')
                .unwrap_or(false);
        let after_ok = t[end..]
            .chars()
            .next()
            .map(|c| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

fn check_site(
    root: &Path,
    site: &hale_graph::Site,
    rule: Rule,
    where_: &str,
    missing: &mut Vec<String>,
) {
    let path = root.join(site.path);
    let Ok(text) = std::fs::read_to_string(&path) else {
        missing.push(format!("{where_}: {} does not exist", site.path));
        return;
    };
    let ok = match rule {
        Rule::Definition if is_identifier(site.symbol) => defines(&text, site.symbol),
        Rule::CodeMention if is_identifier(site.symbol) => mentioned_in_code(&text, site.symbol),
        _ => text.contains(site.symbol),
    };
    if !ok {
        missing.push(format!(
            "{where_}: `{}` is not {} in {} (renamed or deleted; fix the registry)",
            site.symbol,
            match rule {
                Rule::Definition if is_identifier(site.symbol) => "defined",
                Rule::CodeMention if is_identifier(site.symbol) => "used in code",
                _ => "present",
            },
            site.path
        ));
    }
}

#[test]
fn every_registered_site_exists() {
    let root = workspace_root();
    let mut missing = Vec::new();
    let mut seen = 0usize;
    for f in hale_graph::families() {
        if let Some(p) = &f.producer {
            check_site(
                &root,
                p,
                Rule::Definition,
                &format!("family `{}` producer", f.name),
                &mut missing,
            );
            seen += 1;
        }
        for l in f.legacy {
            check_site(
                &root,
                &l.site,
                Rule::CodeMention,
                &format!("family `{}` legacy", f.name),
                &mut missing,
            );
            seen += 1;
        }
        for o in f.owned {
            check_site(
                &root,
                o,
                Rule::Definition,
                &format!("family `{}` owned", f.name),
                &mut missing,
            );
            seen += 1;
        }
        for c in f.consumers {
            if let Some(s) = &c.site {
                check_site(
                    &root,
                    s,
                    Rule::Verbatim,
                    &format!("family `{}` consumer {}", f.name, c.who),
                    &mut missing,
                );
                seen += 1;
            }
        }
        // A focused test is a path, optionally followed by a note
        // (`path (the test's name)`): the path is what must exist.
        for t in f.tests {
            let path = t.split(' ').next().unwrap_or(t);
            if !root.join(path).is_file() {
                missing.push(format!(
                    "family `{}` test: {path} does not exist",
                    f.name
                ));
            }
            seen += 1;
        }
        for s in f.seams {
            for (allowed, _) in s.allowed {
                let site = hale_graph::Site {
                    path: allowed,
                    symbol: s.symbol,
                };
                check_site(
                    &root,
                    &site,
                    Rule::Verbatim,
                    &format!("family `{}` seam `{}`", f.name, s.symbol),
                    &mut missing,
                );
                seen += 1;
            }
        }
    }
    for r in hale_graph::rules() {
        if let Some(e) = &r.evaluator {
            check_site(
                &root,
                e,
                Rule::Definition,
                &format!("rule `{}` evaluator", r.id),
                &mut missing,
            );
            seen += 1;
        }
    }
    assert!(seen > 150, "the site scan is vacuous ({seen} sites)");
    assert!(
        missing.is_empty(),
        "{} registered site(s) do not exist:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn a_comment_is_not_a_code_mention() {
    let trailing = "fn run() {\n    let g = graph(); // build_bus_graph used to be here\n}\n";
    assert!(
        !mentioned_in_code(trailing, "build_bus_graph"),
        "a file whose only mention is a trailing comment does not use the symbol in code"
    );
    assert!(!mentioned_in_code(
        "    // build_bus_graph(p)\n",
        "build_bus_graph"
    ));
    assert!(!mentioned_in_code(
        "let g = /* build_bus_graph */ graph();\n",
        "build_bus_graph"
    ));
    assert!(!mentioned_in_code(
        "let g = graph(); /* build_bus_graph(\n",
        "build_bus_graph"
    ));
    // code after a closed block comment, and code before a trailing one
    assert!(mentioned_in_code(
        "let g = /* old */ build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "let g = build_bus_graph(p); // the one call\n",
        "build_bus_graph"
    ));
    // a comment marker inside a string literal is text, not a comment
    assert!(mentioned_in_code(
        "let u = \"http://x\"; build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "let u = \"a \\\" // b\"; build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "if c == '\"' { build_bus_graph(p); }\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "if c == '\\'' { build_bus_graph(p); }\n",
        "build_bus_graph"
    ));
    // a lifetime is not a character literal: the quote after it opens
    // a string as usual
    assert!(mentioned_in_code(
        "fn f<'a>(s: &'a str) { let u = \"//\"; build_bus_graph(p); }\n",
        "build_bus_graph"
    ));
}

/// Block comments nest and span lines, and a raw string's contents are
/// text whatever they spell (outside review of #1278, finding 2).
#[test]
fn a_nested_or_multi_line_comment_and_a_raw_string_are_read_whole() {
    // the mention sits inside the outer comment, past the inner `*/`
    assert!(!mentioned_in_code(
        "/* outer /* inner */ build_bus_graph(p) */ let x = 0;\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "/* outer /* inner */ still */ build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    // a block comment across lines
    assert!(!mentioned_in_code(
        "let x = 0; /*\n    build_bus_graph(p);\n*/\nlet y = 1;\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "/*\n  old\n*/\nbuild_bus_graph(p);\n",
        "build_bus_graph"
    ));
    // a comment marker inside a raw string opens nothing
    assert!(mentioned_in_code(
        "let s = r#\"\"//\"#; build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "let s = r##\"a \"# /* b\"##; build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "let s = r\"/*\"; build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    // byte strings, plain and raw
    assert!(mentioned_in_code(
        "let s = b\"/*\"; build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    assert!(mentioned_in_code(
        "let s = br#\"//\"#; build_bus_graph(p);\n",
        "build_bus_graph"
    ));
    // a raw string across lines holds a comment marker, and code follows
    assert!(mentioned_in_code(
        "let s = r#\"\n /* \"#;\nbuild_bus_graph(p);\n",
        "build_bus_graph"
    ));
    // a symbol spelled in a string is a use: codegen names the runtime
    // that way
    assert!(mentioned_in_code(
        "b.declare(\"build_bus_graph\");\n",
        "build_bus_graph"
    ));
    // an identifier ending in `r` before a string is not a raw string
    assert!(mentioned_in_code(
        "let v = for_r\"#\"; build_bus_graph(p);\n",
        "build_bus_graph"
    ));
}
