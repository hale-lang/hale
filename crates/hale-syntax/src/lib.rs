//! Hale: lexer, parser, AST.
//!
//! Public surface:
//! - [`lex`] — tokenize a source string.
//! - [`parse`] — parse a token stream into an AST.
//! - [`parse_source`] — convenience: lex + parse from a string.
//! - [`ast`] — AST node types.
//! - [`Span`] — source-position type.
//! - [`Diag`] — diagnostic type for errors.

pub mod ast;
pub mod desugar;
pub mod error;
pub mod keywords;
pub mod lexer;
pub mod parser;
pub mod span;

pub use crate::error::{Diag, DiagKind};
pub use crate::lexer::{lex, Token, TokenKind};
pub use crate::parser::parse;
pub use crate::span::{Pos, Span};

/// Lex + parse a source string into a [`ast::Program`].
pub fn parse_source(source: &str) -> Result<ast::Program, Vec<Diag>> {
    let tokens = lex(source)?;
    parse(tokens, source)
}

/// Lex + parse like [`parse_source`], but offset every span (and any
/// diagnostic span) by `base` bytes. A multi-file build parses each file
/// at a distinct `base` so the merged program's spans are globally
/// unique — a diagnostic span can then be demultiplexed back to its
/// originating file (its `base..base+len` range), giving the right
/// filename + line instead of rendering an imported span against the
/// entry file. `base` is a virtual coordinate; no combined source string
/// is built.
pub fn parse_source_at(source: &str, base: u32) -> Result<ast::Program, Vec<Diag>> {
    let mut tokens =
        lex(source).map_err(|ds| ds.into_iter().map(|d| d.shifted(base)).collect::<Vec<_>>())?;
    for t in &mut tokens {
        t.span = t.span.shifted(base);
    }
    parse(tokens, source).map_err(|ds| ds.into_iter().map(|d| d.shifted(base)).collect())
}
