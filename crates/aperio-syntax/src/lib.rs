//! Aperio: lexer, parser, AST.
//!
//! Public surface:
//! - [`lex`] — tokenize a source string.
//! - [`parse`] — parse a token stream into an AST.
//! - [`parse_source`] — convenience: lex + parse from a string.
//! - [`ast`] — AST node types.
//! - [`Span`] — source-position type.
//! - [`Diag`] — diagnostic type for errors.

pub mod ast;
pub mod error;
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
