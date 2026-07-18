//! `hale fmt` — the canonical formatter (spec/testing.md: Go-style,
//! zero config).
//!
//! Design: a TOKEN-STREAM formatter, not an AST pretty-printer. The
//! author's line-break structure is preserved (gofmt's philosophy —
//! no max-line-length enforcement); the formatter normalizes what
//! can be normalized without judgment calls:
//!
//!   * indentation — 4 spaces per bracket depth, computed by a
//!     bracket stack that records the indent of each opener's LINE
//!     (so `set(Rec {` opening two brackets on one line indents its
//!     contents once, and `});` returns to the opener's indent);
//!     bracket-less continuation lines (`&&` at line start, a
//!     trailing `+` on the previous line) get one extra level
//!   * inter-token spacing — a canonical pair table (space around
//!     binary operators, none inside `()`/`[]`, tight `.`/`::`/
//!     `..`, spaced `{ }` literal braces, unary `-`/`!` tight to
//!     their operand, generic `<...>` tight via a small classifier)
//!   * blank lines — collapsed to at most one, none at file start,
//!     exactly one trailing newline
//!   * comments — preserved verbatim in position: own-line comments
//!     indent with the code, trailing comments sit one space after
//!     the code
//!
//! Token text is always emitted as the RAW source slice (never
//! re-serialized from the token value), so string escapes, float
//! spellings, f-strings, and bytes literals round-trip exactly.
//!
//! Safety gate: `format_source` re-lexes its own output and requires
//! the semantic token stream (kind + raw text) to be IDENTICAL to
//! the input's. A formatter bug can therefore mangle whitespace at
//! worst — it cannot change what the compiler sees. Callers get
//! `FmtError::Changed` (a formatter bug, never written to disk) or
//! `FmtError::Parse` (the input didn't lex).

use crate::error::Diag;
use crate::lexer::{lex, Token, TokenKind};

/// Why formatting was refused.
#[derive(Debug)]
pub enum FmtError {
    /// The input didn't lex; diagnostics attached. Formatting a
    /// file that doesn't lex would destroy information.
    Parse(Vec<Diag>),
    /// The formatted output's token stream differed from the
    /// input's — a formatter bug. The output is attached for
    /// debugging but must not be written.
    Changed(String),
}

/// One element of the reconstructed full-fidelity stream: a
/// semantic token or a comment recovered from an inter-token gap.
enum Elem {
    /// Index into the token vec.
    Tok(usize),
    /// Comment text exactly as written (no trailing newline).
    Comment(String),
}

struct Item {
    elem: Elem,
    /// Newlines in the source between the previous item and this
    /// one. 0 = same line.
    newlines_before: usize,
}

/// Format Hale source into its canonical form.
pub fn format_source(source: &str) -> Result<String, FmtError> {
    let tokens = lex(source).map_err(FmtError::Parse)?;
    let items = build_stream(source, &tokens);
    let out = emit(source, &tokens, &items);

    // Safety gate: identical semantic token stream, or refuse.
    match lex(&out) {
        Ok(out_tokens) => {
            if !token_streams_equal(source, &tokens, &out, &out_tokens) {
                return Err(FmtError::Changed(out));
            }
        }
        Err(_) => return Err(FmtError::Changed(out)),
    }
    Ok(out)
}

/// Recover comments + line structure from the gaps between token
/// spans. The lexer guarantees gaps contain only whitespace and
/// comments (string contents are inside token spans), so a naive
/// scan of the gap text is exact.
fn build_stream(source: &str, tokens: &[Token]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut prev_end = 0usize;
    for (i, tok) in tokens.iter().enumerate() {
        if matches!(tok.kind, TokenKind::Eof) {
            // Trailing gap: comments after the last real token.
            push_gap_comments(source, prev_end, source.len(), &mut items);
            break;
        }
        let start = tok.span.start.as_usize();
        push_gap_comments(source, prev_end, start, &mut items);
        let nl = pending_newlines(source, prev_end, start, &items);
        items.push(Item {
            elem: Elem::Tok(i),
            newlines_before: nl,
        });
        prev_end = tok.span.end.as_usize();
    }
    items
}

/// Newlines between `from` and the next emitted element, counting
/// only the segment AFTER the last comment already emitted from
/// this gap (comment pushes advance a cursor via a sentinel — see
/// push_gap_comments, which records its own newline counts).
fn pending_newlines(
    source: &str,
    gap_start: usize,
    elem_start: usize,
    items: &[Item],
) -> usize {
    // If the previous item is a comment from this same gap, count
    // newlines from its end instead of the gap start.
    let mut from = gap_start;
    if let Some(Item {
        elem: Elem::Comment(_),
        ..
    }) = items.last()
    {
        // The comment's end offset was stashed by push_gap_comments
        // in LAST_COMMENT_END; using a thread-local keeps Item lean.
        LAST_COMMENT_END.with(|c| {
            let e = c.get();
            if e > from && e <= elem_start {
                from = e;
            }
        });
    }
    source[from..elem_start]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
}

thread_local! {
    static LAST_COMMENT_END: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Scan a gap for comments and push them as items with their own
/// newline counts.
fn push_gap_comments(
    source: &str,
    gap_start: usize,
    gap_end: usize,
    items: &mut Vec<Item>,
) {
    let bytes = source.as_bytes();
    let mut pos = gap_start;
    let mut seg_start = gap_start;
    while pos < gap_end {
        let b = bytes[pos];
        if b == b'/' && pos + 1 < gap_end && bytes[pos + 1] == b'/' {
            let mut end = pos;
            while end < gap_end && bytes[end] != b'\n' {
                end += 1;
            }
            let nl = source[seg_start..pos]
                .bytes()
                .filter(|&b| b == b'\n')
                .count();
            items.push(Item {
                elem: Elem::Comment(
                    source[pos..end].trim_end().to_string(),
                ),
                newlines_before: nl,
            });
            LAST_COMMENT_END.with(|c| c.set(end));
            pos = end;
            seg_start = end;
        } else if b == b'/' && pos + 1 < gap_end && bytes[pos + 1] == b'*' {
            let mut end = pos + 2;
            while end + 1 < gap_end
                && !(bytes[end] == b'*' && bytes[end + 1] == b'/')
            {
                end += 1;
            }
            end = (end + 2).min(gap_end);
            let nl = source[seg_start..pos]
                .bytes()
                .filter(|&b| b == b'\n')
                .count();
            items.push(Item {
                elem: Elem::Comment(source[pos..end].to_string()),
                newlines_before: nl,
            });
            LAST_COMMENT_END.with(|c| c.set(end));
            pos = end;
            seg_start = end;
        } else {
            pos += 1;
        }
    }
}

/// Word-shaped tokens: identifiers, keywords, literals, `self`,
/// `true`… Anything that must be space-separated from an adjacent
/// word.
fn is_wordlike(kind: &TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        kind,
        Ident(_)
            | IntLit(_)
            | FloatLit(_)
            | DecimalLit(_)
            | StringLit(_)
            | FStringLit(_)
            | BytesLit(_)
            | DurationLit(_)
            | TimeLit(_)
    ) || kind.keyword_lexeme().is_some()
}

fn is_binary_op(kind: &TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        kind,
        Plus | Minus
            | Star
            | Slash
            | Percent
            | Eq
            | EqEq
            | NotEq
            | Lt
            | Gt
            | LtEq
            | GtEq
            | AndAnd
            | OrOr
            | Amp
            | Pipe
            | Caret
            | Shl
            | Shr
            | LeftArrow
            | PlusEq
            | MinusEq
            | StarEq
            | SlashEq
            | PercentEq
            | AmpEq
            | PipeEq
            | CaretEq
            | TildeTilde
            | Arrow
            | FatArrow
    )
}

/// Can this token end an operand? (Determines whether a following
/// `-`/`+`/`!` is binary or unary.)
fn ends_operand(kind: &TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        kind,
        Ident(_)
            | IntLit(_)
            | FloatLit(_)
            | DecimalLit(_)
            | StringLit(_)
            | FStringLit(_)
            | BytesLit(_)
            | DurationLit(_)
            | TimeLit(_)
            | RParen
            | RBracket
            | RBrace
            | KwSelf
            | True
            | False
            | Nil
    )
}

/// Generic-angle classifier: token indices of `<` and `>` that
/// delimit a generic argument list (`Holder<Int>`), which format
/// TIGHT instead of as spaced comparisons. `<` qualifies iff it
/// directly follows an identifier and a matching `>` closes within
/// a short window with only type-shaped tokens between.
fn classify_generic_angles(tokens: &[Token]) -> Vec<bool> {
    use TokenKind::*;
    let mut generic = vec![false; tokens.len()];
    for i in 0..tokens.len() {
        if !matches!(tokens[i].kind, Lt) {
            continue;
        }
        let prev_is_ident =
            i > 0 && matches!(tokens[i - 1].kind, Ident(_));
        if !prev_is_ident {
            continue;
        }
        // Scan forward: only type-shaped tokens allowed, nested
        // angles tracked, bounded window.
        let mut depth = 1usize;
        let mut j = i + 1;
        let mut ok = false;
        let mut closers: Vec<usize> = Vec::new();
        while j < tokens.len() && j - i <= 32 {
            match &tokens[j].kind {
                Lt => depth += 1,
                Gt => {
                    depth -= 1;
                    closers.push(j);
                    if depth == 0 {
                        ok = true;
                        break;
                    }
                }
                Ident(_) | Comma | ColonColon | LBracket | RBracket
                | IntLit(_) | Semi => {}
                _ => break,
            }
            j += 1;
        }
        if ok {
            generic[i] = true;
            for c in closers {
                generic[c] = true;
            }
        }
    }
    generic
}

/// Canonical spacing between two adjacent tokens on one line.
struct SpaceCx<'a> {
    tokens: &'a [Token],
    generic_angle: &'a [bool],
}

impl<'a> SpaceCx<'a> {
    fn space_between(&self, pi: usize, ni: usize) -> bool {
        use TokenKind::*;
        let p = &self.tokens[pi].kind;
        let n = &self.tokens[ni].kind;

        // Generic angles are tight on both sides:
        // `Holder<Int>` / `Box<Int>>`.
        if matches!(n, Lt) && self.generic_angle[ni] {
            return false;
        }
        if matches!(p, Lt) && self.generic_angle[pi] {
            return false;
        }
        if matches!(n, Gt) && self.generic_angle[ni] {
            return false;
        }
        if matches!(p, Gt) && self.generic_angle[pi] {
            // `Box<Int> ` — space after a closing generic angle
            // unless the next token attaches (call paren, comma…).
            return !matches!(
                n,
                Comma | Semi | RParen | RBracket | LParen | Gt | LBracket
            );
        }

        // `locus X : serves Y` — the conformance-clause colon is
        // SPACED both sides by spec convention (spec/tokens.md's
        // own examples); every other colon is tight-left. Detected
        // by the contextual `serves` ident that must follow it.
        if matches!(n, Colon) {
            if let Some(next) = self.tokens.get(ni + 1) {
                if matches!(&next.kind, Ident(nm) if nm == "serves") {
                    return true;
                }
            }
            return false;
        }
        // Never a space BEFORE these.
        if matches!(n, Comma | Semi | Dot | ColonColon | DotDot
            | DotDotEq | RParen | RBracket | Question | QuestionQuestion)
        {
            return false;
        }
        // Never a space AFTER these.
        if matches!(p, Dot | ColonColon | DotDot | DotDotEq | At | Hash
            | Dollar | LParen | LBracket)
        {
            return false;
        }
        // `@decorator`: no space between @ and the name (covered
        // above); a space BEFORE `@` when preceded by a word.
        if matches!(n, At | Hash | Dollar) {
            return true;
        }

        // `n` is a unary operator (prev token can't end an
        // operand). Space BEFORE it depends on what `p` is: tight
        // after another unary (`!!x`), spaced after a BINARY
        // `-`/`+` (`a - -1`), spaced after commas/keywords/`=`
        // (`= -1`, `(a, -1)`). Its tightness to its own operand is
        // the NEXT pair's decision (the p-is-unary arm below).
        if matches!(n, Minus | Plus | Bang | Tilde) && !ends_operand(p) {
            if matches!(p, Bang | Tilde) {
                return false;
            }
            if matches!(p, Minus | Plus) {
                // p is itself unary iff ITS previous token can't
                // end an operand — then stack tight (`--x` never
                // legally lexes anyway); p binary → space.
                let p_binary = pi
                    .checked_sub(1)
                    .map(|i| ends_operand(&self.tokens[i].kind))
                    .unwrap_or(false);
                return p_binary;
            }
            return true;
        }
        // No space AFTER a unary operator.
        if matches!(p, Bang | Tilde) {
            return false;
        }
        if matches!(p, Minus | Plus) {
            // Unary if the token before p can't end an operand.
            let before = if pi == 0 {
                None
            } else {
                Some(&self.tokens[pi - 1].kind)
            };
            let unary = match before {
                Some(b) => !ends_operand(b),
                None => true,
            };
            if unary {
                return false;
            }
            return true;
        }

        // Binary operators: spaced both sides.
        if is_binary_op(p) || is_binary_op(n) {
            return true;
        }

        // `(` / `[` openers: tight after a callee/indexee (ident,
        // closer, self) and after the keyword-named declarations
        // that take parameter lists (`run()`, `birth()`, mode
        // methods); spaced after other keywords and operators
        // (`if (`, `return (`).
        if matches!(n, LParen | LBracket) {
            if matches!(
                p,
                Ident(_)
                    | KwSelf
                    | RParen
                    | RBracket
                    | Gt
                    | Run
                    | Birth
                    | Accept
                    | Drain
                    | Dissolve
                    | OnFailure
                    | Bulk
                    | Harmonic
                    | Resolution
                    | Perspective
            ) {
                return false;
            }
            return true;
        }

        // Braces: inline literal style — `Rec { key: 1 }`, `{ }`.
        if matches!(n, LBrace) {
            return true;
        }
        if matches!(p, LBrace) {
            return true;
        }
        if matches!(n, RBrace) {
            return true;
        }
        if matches!(p, RBrace) {
            // `};` and `})` and `},` handled by the no-space-before
            // rules above; `} else` gets a space.
            return true;
        }

        // After `,` `;` `:` — space.
        if matches!(p, Comma | Semi | Colon) {
            return true;
        }
        // `?`/`??` (reserved): space after.
        if matches!(p, Question | QuestionQuestion) {
            return true;
        }

        // Word next to word.
        if is_wordlike(p) || is_wordlike(n) {
            return true;
        }
        true
    }
}

/// Raw source slice for a token.
fn tok_text<'a>(source: &'a str, tok: &Token) -> &'a str {
    &source[tok.span.start.as_usize()..tok.span.end.as_usize()]
}

fn emit(source: &str, tokens: &[Token], items: &[Item]) -> String {
    use TokenKind::*;
    let generic_angle = classify_generic_angles(tokens);
    let cx = SpaceCx {
        tokens,
        generic_angle: &generic_angle,
    };

    let mut out = String::with_capacity(source.len() + 64);
    // Bracket stack: (opener kind, indent of the opener's line).
    let mut stack: Vec<(TokenKind, usize)> = Vec::new();
    let mut cur_line_indent = 0usize;
    let mut line_start = true;
    let mut first_line = true;
    // Index of the previous TOKEN emitted on the current line
    // (None right after a newline or when last emit was a comment).
    let mut prev_tok_on_line: Option<usize> = None;
    // Last token index emitted anywhere (for continuation logic).
    let mut last_tok: Option<usize> = None;

    for item in items {
        let is_comment = matches!(item.elem, Elem::Comment(_));
        let newlines = item.newlines_before;

        if newlines > 0 {
            if !first_line {
                out.push('\n');
                if newlines >= 2 {
                    out.push('\n');
                }
            }
            line_start = true;
            prev_tok_on_line = None;
        }

        match &item.elem {
            Elem::Tok(ti) => {
                let tok = &tokens[*ti];
                if line_start {
                    // Compute this line's indent from the bracket
                    // stack. A line whose first token closes a
                    // bracket gets the OPENER's line indent.
                    let indent = if matches!(tok.kind, RBrace | RParen | RBracket)
                    {
                        stack.last().map(|(_, li)| *li).unwrap_or(0)
                    } else {
                        let base = stack
                            .last()
                            .map(|(_, li)| *li + 1)
                            .unwrap_or(0);
                        // Bracket-less continuation: previous line
                        // ended mid-expression (trailing binary op)
                        // or this line starts with one (`&&`, `.`).
                        let continuation = match (last_tok, &tok.kind) {
                            (_, k) if is_binary_op(k) => true,
                            (_, Dot) => true,
                            (Some(lt), _) => {
                                let lk = &tokens[lt].kind;
                                is_binary_op(lk)
                                    && !matches!(lk, FatArrow)
                            }
                            _ => false,
                        };
                        base + usize::from(continuation)
                    };
                    cur_line_indent = indent;
                    for _ in 0..indent {
                        out.push_str("    ");
                    }
                    line_start = false;
                    first_line = false;
                } else if let Some(pi) = prev_tok_on_line {
                    if cx.space_between(pi, *ti) {
                        out.push(' ');
                    }
                } else {
                    // After an inline comment on the same line —
                    // can't happen for line comments (they eat to
                    // EOL); block comment then code: one space.
                    out.push(' ');
                }
                out.push_str(tok_text(source, tok));

                match tok.kind {
                    LBrace | LParen | LBracket => {
                        stack.push((tok.kind.clone(), cur_line_indent));
                    }
                    RBrace | RParen | RBracket => {
                        stack.pop();
                    }
                    _ => {}
                }
                prev_tok_on_line = Some(*ti);
                last_tok = Some(*ti);
            }
            Elem::Comment(text) => {
                if line_start {
                    // Own-line comment: indent like code. Closers on
                    // the NEXT line shouldn't affect it; use the
                    // normal non-closer rule.
                    let indent =
                        stack.last().map(|(_, li)| *li + 1).unwrap_or(0);
                    for _ in 0..indent {
                        out.push_str("    ");
                    }
                    line_start = false;
                    first_line = false;
                } else {
                    // Trailing comment: one space after the code.
                    out.push(' ');
                }
                // Multi-line block comments pass through verbatim;
                // their interior lines keep author formatting.
                out.push_str(text);
                if is_comment && text.starts_with("//") {
                    // A line comment always ends its line; the next
                    // item's newline count includes the newline that
                    // terminated it in the source, so nothing to do.
                }
                prev_tok_on_line = None;
            }
        }
    }

    // Exactly one trailing newline.
    while out.ends_with('\n') || out.ends_with(' ') {
        out.pop();
    }
    out.push('\n');
    out
}

/// Compare two semantic token streams: same kinds, same raw text.
fn token_streams_equal(
    src_a: &str,
    a: &[Token],
    src_b: &str,
    b: &[Token],
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (ta, tb) in a.iter().zip(b.iter()) {
        if std::mem::discriminant(&ta.kind) != std::mem::discriminant(&tb.kind)
        {
            return false;
        }
        if tok_text(src_a, ta) != tok_text(src_b, tb) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(s: &str) -> String {
        format_source(s).expect("formats")
    }

    #[test]
    fn normalizes_spacing_and_indent() {
        let src = "fn main(){\nlet x=1+2;\nprintln( \"v \" ,x );\n}\n";
        let out = fmt(src);
        assert_eq!(
            out,
            "fn main() {\n    let x = 1 + 2;\n    println(\"v \", x);\n}\n"
        );
    }

    #[test]
    fn preserves_comments() {
        let src = "// header\nfn main() {\n    let x = 1; // trailing\n    // own line\n    println(x);\n}\n";
        let out = fmt(src);
        assert_eq!(out, src);
    }

    #[test]
    fn idempotent_on_own_output() {
        let src = "fn main(){let x=-1;if x<0{println(\"neg\");}}";
        let once = fmt(src);
        let twice = fmt(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn unary_minus_stays_tight() {
        let out = fmt("fn f() -> Int {\n    return -1;\n}\n");
        assert!(out.contains("return -1;"), "{}", out);
        let out2 = fmt("fn g(a: Int) -> Int {\n    return a - -1;\n}\n");
        assert!(out2.contains("a - -1"), "{}", out2);
    }

    #[test]
    fn collapses_blank_lines() {
        let out = fmt("fn a() { }\n\n\n\nfn b() { }\n");
        assert_eq!(out, "fn a() { }\n\nfn b() { }\n");
    }

    #[test]
    fn refuses_unlexable() {
        assert!(matches!(
            format_source("fn main() { let s = \"unterminated; }"),
            Err(FmtError::Parse(_))
        ));
    }
}
