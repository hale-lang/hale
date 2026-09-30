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
//! its identifier must be used in a code line, not only a comment.
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

fn mentioned_in_code(text: &str, name: &str) -> bool {
    text.lines().any(|line| {
        let t = line.trim_start();
        if t.starts_with("//") {
            return false;
        }
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
    })
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
