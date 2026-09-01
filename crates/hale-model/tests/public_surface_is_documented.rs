//! Every public field of the model carries a doc comment.
//!
//! This crate IS the documentation of the model for anyone writing a
//! judgment, a consumer, or a downstream tool: `spec/model.md` states
//! the laws and the grain, and the rustdoc says what each row holds.
//! The epic that built this crate scoped its documentation as
//! "design note ships as crate-level rustdoc", and the fields drifted
//! to 35% covered without anything noticing — a field added in a
//! review round is exactly the field nobody comes back to describe.
//!
//! So it is checked. A public field with no `///` fails here, naming
//! itself, which is cheaper than discovering years later that the
//! reference has holes in it.
//!
//! Scoped to fields deliberately. Types are covered by
//! `spec/model.md`'s inventory, and a rule that demanded a comment on
//! every enum variant would be answered with noise rather than
//! meaning.

use std::path::PathBuf;

fn src_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("src");
    p
}

/// Is this line a `pub <name>:` field declaration?
///
/// Struct fields only — `pub fn`, `pub const`, `pub mod` and `pub
/// use` all fail the trailing-colon test, and tuple-struct members
/// (`pub Vec<u32>`) have no name to document.
fn field_name(line: &str) -> Option<&str> {
    let t = line.trim();
    let rest = t.strip_prefix("pub ")?;
    let (name, _) = rest.split_once(':')?;
    let name = name.trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    Some(name)
}

#[test]
fn every_public_model_field_has_a_doc_comment() {
    let mut undocumented: Vec<String> = Vec::new();
    let mut total = 0usize;

    let mut files: Vec<PathBuf> = std::fs::read_dir(src_dir())
        .expect("src/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no sources found — bad test wiring");

    for path in files {
        let text = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = text.lines().collect();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        for (i, line) in lines.iter().enumerate() {
            let Some(field) = field_name(line) else { continue };
            total += 1;
            // Walk back over attributes and blank lines to the
            // nearest meaningful line; a doc comment there documents
            // this field.
            let mut j = i as isize - 1;
            while j >= 0 {
                let prev = lines[j as usize].trim();
                if prev.starts_with("#[") || prev.is_empty() {
                    j -= 1;
                    continue;
                }
                break;
            }
            let documented = j >= 0
                && lines[j as usize].trim().starts_with("///");
            if !documented {
                undocumented.push(format!("{}:{} {}", name, i + 1, field));
            }
        }
    }

    // The scan itself can rot: a refactor that renames the fields out
    // from under `field_name` would leave this test passing over
    // nothing at all, which is worse than failing.
    assert!(
        total > 200,
        "only found {} public fields — the scan is broken, not the docs",
        total
    );
    assert!(
        undocumented.is_empty(),
        "{} public field(s) have no doc comment. The model's rustdoc \
         IS its field reference; a row nobody describes is a row a \
         consumer has to guess at:\n  {}",
        undocumented.len(),
        undocumented.join("\n  ")
    );
}
