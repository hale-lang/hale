//! The keyword list is frozen (STABILITY.md § Keywords, GH #1950).
//!
//! `spec/keywords.txt` holds every hard and every contextual keyword.
//! `keywords_are_frozen` fails on any difference between it and
//! `hale_syntax::keywords`, and `lexer_reserves_exactly_the_hard_keywords`
//! fails when the lexer reserves a word the list does not name (the
//! other direction is `keyword_sync.rs`'s), so a keyword cannot be added,
//! dropped or moved between the two kinds by editing one file.
//!
//! Changing the list is a language change and goes through an RFC: an
//! issue titled `RFC: …`, decided and recorded in `spec/decisions.md`.
//! The PR that implements the decision edits `spec/keywords.txt` by hand
//! and names the decision in its CHANGELOG fragment's `### Breaking`
//! section (a new hard keyword breaks every program using the word as a
//! name; a new contextual one may, in its position).

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use hale_syntax::keywords::{CONTEXTUAL_KEYWORDS, HARD_KEYWORDS};

const RFC: &str = "The keyword list is frozen: adding, removing or reclassifying a keyword \
     takes an RFC (an issue titled `RFC: ...`, its decision recorded in spec/decisions.md; \
     see STABILITY.md § Keywords). With the decision in hand, update spec/keywords.txt \
     to the expected text above in the same PR and list the change under `### Breaking` \
     in its CHANGELOG fragment.";

const LITERALS: &[&str] = &["true", "false", "nil"];

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/keywords.txt")
}

fn sorted(words: &[&'static str]) -> BTreeSet<&'static str> {
    words.iter().copied().collect()
}

fn render() -> String {
    let mut out = String::from(
        "# Hale's keywords, frozen (STABILITY.md § Keywords).\n\
         # Held equal to crates/hale-syntax/src/keywords.rs by\n\
         # crates/hale-syntax/tests/keyword_freeze.rs. A change is an RFC.\n\
         #\n\
         # hard: lexed as their own token, never a name (spec/tokens.md\n\
         # § Reserved words). contextual: lexed as a name, a keyword only in\n\
         # its position (F.43).\n",
    );
    out.push_str("\n[hard]\n");
    for w in sorted(HARD_KEYWORDS) {
        out.push_str(w);
        out.push('\n');
    }
    out.push_str("\n[contextual]\n");
    for w in sorted(CONTEXTUAL_KEYWORDS) {
        out.push_str(w);
        out.push('\n');
    }
    out
}

fn words_of(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn keywords_are_frozen() {
    let want = render();
    let have = fs::read_to_string(golden_path()).unwrap_or_default();
    if have == want {
        return;
    }
    let (w, h) = (words_of(&want), words_of(&have));
    let added: Vec<&String> = w.difference(&h).collect();
    let removed: Vec<&String> = h.difference(&w).collect();
    panic!(
        "spec/keywords.txt differs from crates/hale-syntax/src/keywords.rs.\n\
         lines in keywords.rs but not the golden: {added:?}\n\
         lines in the golden but not keywords.rs: {removed:?}\n\
         expected spec/keywords.txt:\n{want}\n{RFC}"
    );
}

#[test]
fn lexer_reserves_exactly_the_hard_keywords() {
    // The lexer's keyword match: `"word" => TokenKind::X,` arms.
    let src = include_str!("../src/lexer.rs");
    let mut reserved = BTreeSet::new();
    for line in src.lines() {
        let l = line.trim_start();
        let Some(rest) = l.strip_prefix('"') else { continue };
        let Some((word, arm)) = rest.split_once('"') else { continue };
        if arm.trim_start().starts_with("=> TokenKind::")
            && !word.is_empty()
            && word.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        {
            reserved.insert(word);
        }
    }
    for lit in LITERALS {
        assert!(reserved.remove(lit), "the lexer no longer reserves the literal `{lit}`");
    }
    let listed = sorted(HARD_KEYWORDS);
    let unlisted: Vec<&&str> = reserved.difference(&listed).collect();
    assert!(
        unlisted.is_empty(),
        "the lexer reserves {unlisted:?}, which HARD_KEYWORDS (and spec/keywords.txt) do not list.\n{RFC}"
    );
    let unreserved: Vec<&&str> = listed.difference(&reserved).collect();
    assert!(
        unreserved.is_empty(),
        "HARD_KEYWORDS lists {unreserved:?}, which the lexer's keyword match does not reserve.\n{RFC}"
    );
}
