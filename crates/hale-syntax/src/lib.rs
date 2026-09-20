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
pub mod chains;
pub mod desugar;
pub mod error;
pub mod fmt;
pub mod fstring;
pub mod json_gen;
pub mod keywords;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod time_literal;

pub use crate::error::{Diag, DiagKind};
pub use crate::lexer::{lex, Token, TokenKind};
pub use crate::parser::parse;
pub use crate::span::{Pos, Span};

/// Lex + parse a source string into a [`ast::Program`].
pub fn parse_source(source: &str) -> Result<ast::Program, Vec<Diag>> {
    let tokens = lex(source)?;
    parse(tokens, source)
}

/// Lex + parse like [`parse_source`], but offset every span by `base`
/// bytes — including the spans a diagnostic carries, which follow the
/// token they came from. A multi-file build parses each file
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
        // An f-string's interpolation bodies carry their own byte
        // offsets (the parser sub-parses each one and shifts the
        // sub-parse's spans by them). They are offsets into THIS
        // file's text, so they move with the token — otherwise every
        // span inside `f"{x}"` in a non-first file lands `base` bytes
        // early, i.e. in some earlier file.
        if let TokenKind::FStringLit(parts) = &mut t.kind {
            for p in parts {
                if let lexer::FStringPart::Interp { start, end, .. } = p {
                    *start += base as usize;
                    *end += base as usize;
                }
            }
        }
    }
    // GH #725: parse diagnostics are NOT shifted here. Their spans
    // come from the tokens above, which are already in the shifted
    // coordinate space; shifting again put every parse error in a
    // non-first file `base` bytes past where it belongs — usually off
    // the end of the file, so `render_located` printed a line past EOF
    // and no source snippet. Lex diagnostics above ARE shifted: the
    // lexer never saw `base`.
    parse(tokens, source)
}
