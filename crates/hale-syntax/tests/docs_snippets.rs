//! GH #228: every ```hale snippet in the mdBook must parse.
//! Docs the compiler rejects are self-defeating — models and
//! humans both learn the wrong language. Deliberately partial
//! snippets (a bare block, a method body, an elided sketch) opt
//! out with the info-string `hale,fragment` — mdBook keys
//! highlighting off the first token, so rendering is unchanged
//! (the rust,ignore convention).
//!
//! This is the parse gate; running snippets where feasible is
//! the corpus's job (fixtures/examples). CI runs this test in
//! the normal suite, so docs drift fails the build.

use std::path::{Path, PathBuf};

fn docs_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("src")
}

fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read docs dir") {
        let p = entry.expect("dir entry").path();
        if p.is_dir() {
            collect_md(&p, out);
        } else if p.extension().map_or(false, |e| e == "md") {
            out.push(p);
        }
    }
}

struct Snippet {
    file: PathBuf,
    line: usize,
    body: String,
}

fn extract_hale_snippets(file: &Path) -> Vec<Snippet> {
    let text = std::fs::read_to_string(file).expect("read md");
    let mut snippets = Vec::new();
    let mut in_block = false;
    let mut skip = false;
    let mut start_line = 0usize;
    let mut body = String::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if !in_block {
            if let Some(info) = trimmed.strip_prefix("```") {
                let info = info.trim();
                if info == "hale" || info.starts_with("hale,") {
                    in_block = true;
                    skip = info.contains("fragment");
                    start_line = i + 1;
                    body.clear();
                }
            }
        } else if trimmed.starts_with("```") {
            in_block = false;
            if !skip {
                snippets.push(Snippet {
                    file: file.to_path_buf(),
                    line: start_line,
                    body: body.clone(),
                });
            }
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    snippets
}

#[test]
fn every_docs_hale_snippet_parses() {
    let mut files = Vec::new();
    collect_md(&docs_src(), &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no markdown found under docs/src — path wiring broke"
    );
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for f in &files {
        for s in extract_hale_snippets(f) {
            checked += 1;
            if let Err(e) = hale_syntax::parse_source(&s.body) {
                failures.push(format!(
                    "{}:{}: {:?}",
                    s.file.display(),
                    s.line,
                    e
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} docs ```hale snippet(s) fail to parse (mark \
         deliberately-partial blocks ```hale,fragment):\n{}",
        failures.len(),
        checked,
        failures.join("\n")
    );
    // The gate only means something if it sees real coverage.
    assert!(
        checked >= 20,
        "only {} hale snippets found — extraction likely broke",
        checked
    );
}
