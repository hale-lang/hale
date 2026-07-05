//! Canonical Hale keyword lists — the single source of truth for tooling
//! that needs to know "what's a keyword": syntax highlighters (the docs
//! site `docs/hale-highlight.js`, the README SVG generator
//! `tools/hale_svg.py`) and the `pond/heron` tree-sitter grammar.
//!
//! Hale has two kinds of keyword, stored differently in the compiler:
//!
//!   - **hard** — reserved in the lexer ([`crate::lexer`]); each lexes to
//!     its own `TokenKind`, never `Ident`.
//!   - **contextual** — lexed as an identifier and recognized by the
//!     parser only in position, which frees the word for ordinary
//!     identifier use elsewhere (e.g. `topic`, `shm_ring`, `mode`).
//!
//! Highlighters want the union ([`all`]). Literals (`true`/`false`/`nil`)
//! and primitive type names are coloured separately and aren't listed.
//!
//! `tests/keyword_sync.rs` keeps this honest: it verifies every
//! [`HARD_KEYWORDS`] entry really lexes to a non-`Ident` token and every
//! [`CONTEXTUAL_KEYWORDS`] entry lexes to `Ident`, and it regenerates the
//! highlighter keyword blocks from [`all`] (run with `UPDATE_KEYWORDS=1`
//! to bless). Add a keyword here and the test points at what's stale.

/// Reserved keywords — each lexes to its own `TokenKind` (never `Ident`).
/// Mirrors the match in `lexer.rs` (minus the `true`/`false`/`nil`
/// literals, which highlighters colour as literals, not keywords).
pub const HARD_KEYWORDS: &[&str] = &[
    "locus", "perspective", "type", "const", "fn", "import", "export", "module",
    "params", "contract", "bus", "capacity", "as_parent_for", "indexed_by",
    "birth", "accept", "run", "drain", "dissolve", "on_failure",
    "bulk", "harmonic", "resolution",
    "projection", "rich", "chunked", "recognition",
    "closure", "epoch", "persists_through", "resets_on", "resets_per_epoch",
    "restart", "restart_in_place", "quarantine", "reorganize", "bubble",
    "expose", "consume", "inferred",
    "subscribe", "publish", "on", "of",
    "stable_when", "serialize_as",
    "let", "mut", "if", "else", "match", "for", "in", "while",
    "return", "break", "continue", "tier", "self",
    "trait", "impl", "interface", "async", "await", "yield",
    "terminate", "release", "macro", "where",
];

/// Contextual keywords — lexed as `Ident`, coloured as keywords (matching
/// `pond/heron`). Recognized by the parser in position; the parser still
/// matches the strings inline, so this list is for tooling, not parsing.
pub const CONTEXTUAL_KEYWORDS: &[&str] = &[
    "topic", "ring_layout", "as", "main",
    "mode", "bindings", "birth_check", "placement",
    "cooperative", "pinned", "pool", "core", "cores", "heap",
    "topology", "node", "l3", "reserve",
    "schedule", "fixed_cell", "shared_slab", "spillover", "summary_only", "cap",
    "payload", "subject",
    "captures", "inline", "tick", "duration", "explicit", "approx", "within",
    "with", "violate", "fail", "until",
    "sum", "prod", "or", "fallible",
    "unix", "shm_ring", "role", "listen", "connect", "slot_count",
    "on_overflow", "block", "drop",
    "intra_process", "intra_machine", "cross_machine", "zero_copy",
];

/// The union of [`HARD_KEYWORDS`] and [`CONTEXTUAL_KEYWORDS`], sorted and
/// deduped — exactly what a syntax highlighter colours as a keyword.
pub fn all() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = HARD_KEYWORDS
        .iter()
        .chain(CONTEXTUAL_KEYWORDS.iter())
        .copied()
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}
