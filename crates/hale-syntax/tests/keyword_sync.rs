//! Keeps the canonical keyword list (`hale_syntax::keywords`) honest and
//! the downstream syntax highlighters generated from it.
//!
//!   1. `keyword_lists_match_the_lexer` — every `HARD_KEYWORDS` entry
//!      really lexes to its own token (never `Ident`), and every
//!      `CONTEXTUAL_KEYWORDS` entry lexes to `Ident` (so it's free as an
//!      identifier and recognized by the parser in position). This ties
//!      the list to the real lexer; misclassify one and this fails.
//!
//!   2. `highlighter_keyword_blocks_are_in_sync` — the `keyword` block in
//!      `docs/hale-highlight.js` and the `KEYWORDS` set in
//!      `tools/hale_svg.py` are generated from `keywords::all()`. Run with
//!      `UPDATE_KEYWORDS=1` to regenerate (bless); otherwise it asserts
//!      they're current, so a new keyword can't silently drift the docs
//!      site or the README SVGs out of date.

use std::fs;
use std::path::{Path, PathBuf};

use hale_syntax::keywords;
use hale_syntax::lexer::{lex, TokenKind};

#[test]
fn keyword_lists_match_the_lexer() {
    for kw in keywords::HARD_KEYWORDS {
        let toks = lex(kw).unwrap_or_else(|e| panic!("lex `{kw}`: {e:?}"));
        assert!(
            !matches!(toks[0].kind, TokenKind::Ident(_)),
            "HARD keyword `{kw}` lexes to an identifier — it isn't reserved \
             in the lexer. Move it to CONTEXTUAL_KEYWORDS (or add it to the \
             lexer)."
        );
    }
    for kw in keywords::CONTEXTUAL_KEYWORDS {
        let toks = lex(kw).unwrap_or_else(|e| panic!("lex `{kw}`: {e:?}"));
        assert!(
            matches!(toks[0].kind, TokenKind::Ident(_)),
            "CONTEXTUAL keyword `{kw}` lexes to {:?}, not Ident — it's \
             reserved in the lexer, so move it to HARD_KEYWORDS.",
            toks[0].kind
        );
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/hale-syntax -> repo root")
        .to_path_buf()
}

/// The text strictly between the begin-marker line and the end-marker line.
fn region_body<'a>(content: &'a str, begin: &str, end: &str) -> &'a str {
    let bp = content
        .find(begin)
        .unwrap_or_else(|| panic!("begin marker not found: {begin}"));
    let after = bp + content[bp..].find('\n').expect("newline after begin") + 1;
    let ep = after
        + content[after..]
            .find(end)
            .unwrap_or_else(|| panic!("end marker not found: {end}"));
    &content[after..ep]
}

#[test]
fn highlighter_keyword_blocks_are_in_sync() {
    let kw = keywords::all().join(" ");
    let root = repo_root();
    let bless = std::env::var_os("UPDATE_KEYWORDS").is_some();

    let targets = [
        (
            root.join("docs/hale-highlight.js"),
            "      // BEGIN GENERATED KEYWORDS",
            "      // END GENERATED KEYWORDS",
            format!("      keyword:\n        \"{kw}\",\n"),
        ),
        (
            root.join("tools/hale_svg.py"),
            "# BEGIN GENERATED KEYWORDS",
            "# END GENERATED KEYWORDS",
            format!("KEYWORDS = set(\n    \"{kw}\".split()\n)\n"),
        ),
    ];

    for (path, begin, end, want) in targets {
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let have = region_body(&content, begin, end);
        if have == want {
            continue;
        }
        assert!(
            bless,
            "{} keyword block is stale.\n--- generated ---\n{want}\n--- in file ---\n{have}\n\
             Run `UPDATE_KEYWORDS=1 cargo test -p hale-syntax --test keyword_sync` to regenerate.",
            path.display()
        );
        let updated = content.replacen(have, &want, 1);
        fs::write(&path, updated).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }
}
