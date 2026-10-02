//! Hale: lexer, parser, AST.
//!
//! Public surface:
//! - [`lex`] — tokenize a source string.
//! - [`parse`] — parse a token stream into an AST.
//! - [`parse_source`] — convenience: lex + parse from a string.
//! - [`ast`] — AST node types.
//! - [`Span`] — source-position type.
//! - [`Diag`] — diagnostic type for errors.

pub mod api_gen;
pub mod ast;
pub mod chains;
pub mod desugar;
pub mod error;
pub mod fmt;
pub mod fstring;
pub mod json_gen;
pub mod keywords;
pub mod lexer;
pub mod names;
pub mod parser;
pub mod shift;
pub mod sites;
pub mod span;
pub mod time_literal;

pub use crate::error::{Diag, DiagKind, Related, SpanOrigin};
pub use crate::lexer::{lex, Token, TokenKind};
pub use crate::parser::parse;
pub use crate::span::{file_owns_offset, Pos, Span};

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
    parse_source_at_in(source, base, &mut ast::EffectClasses::default())
}

/// [`parse_source_at`], interning user effect classes into `classes`:
/// the load's one table, which every seed of the load is parsed
/// through, so a class has one index in every seed.
pub fn parse_source_at_in(
    source: &str,
    base: u32,
    classes: &mut ast::EffectClasses,
) -> Result<ast::Program, Vec<Diag>> {
    let tokens = lex_at(source, base)?;
    // GH #765: the PARSER's diagnostics must NOT be shifted again — it
    // takes the tokens `lex_at` shifted, whose spans already carry
    // `base`, and cites them. Shifting a second time put a parse error
    // at `2 * base + local`, outside its own file's `base..base+len`
    // range: the span demultiplexed to no file at all (an imported
    // seed's parse error rendered with no filename, positioned against
    // whichever source happened to be first) and a consumer that
    // un-shifts once — `parse_files`, `hale lsp` — landed `base` bytes
    // past the real position, so a parse error in the second or later
    // file of a seed pointed at the wrong line.
    // (GH #725 found the same double shift independently, from the
    // reserved-word cascade: the same one-line fix.)
    parser::parse_in(tokens, source, classes)
}

/// The tokens [`parse_source_at`] parses: `source` lexed, every span
/// (a diagnostic's too) offset by `base`. A caller that parses one text
/// twice (`parser::parse_in`) lexes it once.
pub fn lex_at(source: &str, base: u32) -> Result<Vec<Token>, Vec<Diag>> {
    // The LEXER works on the unshifted source, so its diagnostics have
    // to be shifted here.
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
    Ok(tokens)
}
