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
//! Width ([`format_source_wrapped`], `hale fmt --wrap`): one more rule
//! on the same stream. A source line that renders wider than
//! [`WRAP_WIDTH`] is broken by adding line breaks (and, for a call or a
//! record literal, a trailing comma) inside it, by its shape, outermost
//! first (`Emitter::break_segment`); the indentation rules above then
//! place the new lines, so wrapped code indents like written code. A
//! line that fits is never touched and lines are never joined, which is
//! what makes wrapping a fixed point: every line it writes fits, or has
//! nothing left to break.
//!
//! Token text is always emitted as the RAW source slice (never
//! re-serialized from the token value), so string escapes, float
//! spellings, f-strings, and bytes literals round-trip exactly. The
//! one exception is a block string (`"""` … `"""`, spec/tokens.md §
//! Block strings): its lines move with the code, the closing fence one
//! level deeper than the line the literal opens on and every line of
//! the text the same distance from it as before, which is exactly what
//! keeps its value.
//!
//! Safety gate: `format_source` re-lexes its own output and requires
//! the semantic token stream (kind + raw text) to be IDENTICAL to
//! the input's — for a block string, kind + value, since its raw text
//! is re-indented. A formatter bug can therefore mangle whitespace at
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
#[derive(Clone)]
enum Elem {
    /// Index into the token vec.
    Tok(usize),
    /// Comment text exactly as written (no trailing newline).
    Comment(String),
    /// A trailing comma the wrapper adds to a call or record literal
    /// it breaks one element per line. The gate knows each by its
    /// offset in the output.
    Comma,
}

#[derive(Clone)]
struct Item {
    elem: Elem,
    /// Newlines in the source between the previous item and this
    /// one. 0 = same line.
    newlines_before: usize,
    /// The wrapper found no break for the segment this item starts.
    nobreak: bool,
}

/// The width a wrapped line is held to (spec/testing.md § `hale fmt`).
pub const WRAP_WIDTH: usize = 100;

/// Format Hale source into its canonical form, keeping the author's
/// line breaks.
pub fn format_source(source: &str) -> Result<String, FmtError> {
    format_at(source, None)
}

/// Format Hale source into its canonical form and break every line
/// wider than [`WRAP_WIDTH`] columns (spec/testing.md § `hale fmt`).
pub fn format_source_wrapped(source: &str) -> Result<String, FmtError> {
    format_at(source, Some(WRAP_WIDTH))
}

/// [`format_source_wrapped`] at another width: the tests' lever for
/// reaching every wrapping shape the corpus holds.
#[doc(hidden)]
pub fn format_source_at_width(source: &str, width: usize) -> Result<String, FmtError> {
    format_at(source, Some(width))
}

fn format_at(source: &str, width: Option<usize>) -> Result<String, FmtError> {
    wrap_report(source, width).map(|(out, _)| out)
}

/// The formatted text, and whether the trailing commas had to be
/// dropped because a construct that looked like a call was not one:
/// the corpus test holds that to never.
#[doc(hidden)]
pub fn wrap_report(source: &str, width: Option<usize>) -> Result<(String, bool), FmtError> {
    let mut fell_back = false;
    let tokens = lex(source).map_err(FmtError::Parse)?;
    let items = build_stream(source, &tokens);
    let (mut out, mut commas) = emit(source, &tokens, items.clone(), width, true);
    // A trailing comma goes only where the grammar takes one. A
    // construct the token stream cannot tell from a call must not turn
    // a program that parsed into one that does not: wrap it again
    // without them.
    if !commas.is_empty()
        && crate::parse_source(&out).is_err()
        && crate::parse_source(source).is_ok()
    {
        (out, commas) = emit(source, &tokens, items, width, false);
        fell_back = true;
    }

    // Safety gate: identical semantic token stream, or refuse. The
    // trailing commas the wrapper added are the one difference allowed,
    // each at the offset it recorded.
    match lex(&out) {
        Ok(out_tokens) => {
            let kept: Vec<Token> = out_tokens
                .into_iter()
                .filter(|t| {
                    !(matches!(t.kind, TokenKind::Comma)
                        && commas.binary_search(&t.span.start.as_usize()).is_ok())
                })
                .collect();
            if !token_streams_equal(source, &tokens, &out, &kept) {
                return Err(FmtError::Changed(out));
            }
        }
        Err(_) => return Err(FmtError::Changed(out)),
    }
    Ok((out, fell_back))
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
            nobreak: false,
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
                nobreak: false,
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
                nobreak: false,
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
            | QuantityLit(..)
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
            | QuantityLit(..)
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
                // `Box<Decimal<18>>`: the lexer reads the two closers
                // as one `>>`, which closes this level (and the one
                // around it, whose own scan finds the same token).
                // Only where a type ends: `a < b >> c` is a comparison
                // of a shift.
                Shr if tokens.get(j + 1).map_or(true, |t| {
                    matches!(&t.kind, Semi | Comma | Eq | RParen | RBracket | LBrace | RBrace | Gt | Shr)
                        || matches!(&t.kind, Ident(w) if w == "fallible")
                }) =>
                {
                    closers.push(j);
                    if depth <= 2 {
                        ok = true;
                        break;
                    }
                    depth -= 2;
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
    /// Does the token at `i` end an operand? `ends_operand` answers by
    /// kind; a keyword is the one kind whose answer depends on its
    /// neighbour. After `.` a reserved word is a FIELD name —
    /// `err.closure`, `x.epoch`, `self.tier` — and ends the operand like
    /// any identifier, so a `+` / `-` after it is binary (GH #1064: it
    /// read as unary and `err.closure + " "` became `err.closure +" "`).
    /// The keywords come from the lexer's own table (`keyword_lexeme`,
    /// kept in step with `keywords::HARD_KEYWORDS`), so a keyword added
    /// later is covered without a list here.
    fn ends_operand_at(&self, i: usize) -> bool {
        let kind = &self.tokens[i].kind;
        if ends_operand(kind) {
            return true;
        }
        kind.keyword_lexeme().is_some()
            && i > 0
            && matches!(self.tokens[i - 1].kind, TokenKind::Dot)
    }

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
        if matches!(n, Gt | Shr) && self.generic_angle[ni] {
            return false;
        }
        if matches!(p, Gt | Shr) && self.generic_angle[pi] {
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
        if matches!(n, Minus | Plus | Bang | Tilde) && !self.ends_operand_at(pi) {
            if matches!(p, Bang | Tilde) {
                return false;
            }
            if matches!(p, Minus | Plus) {
                // p is itself unary iff ITS previous token can't
                // end an operand — then stack tight (`--x` never
                // legally lexes anyway); p binary → space.
                let p_binary = pi
                    .checked_sub(1)
                    .map(|i| self.ends_operand_at(i))
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
            let unary = match pi.checked_sub(1) {
                Some(b) => !self.ends_operand_at(b),
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
        // closer, self) — the contextual keywords among them, so the
        // hooks (`run()`, `release(c: T)`, `on_failure(c: C)`) and the
        // recovery statements (`restart(c) for 3;`) read like calls —
        // and after `perspective(P)`; spaced after the hard keywords
        // and operators (`if (`, `return (`).
        if matches!(n, LParen | LBracket) {
            if matches!(
                p,
                Ident(_) | KwSelf | RParen | RBracket | Gt | Perspective
            ) {
                return false;
            }
            // GH #1076: `d.in(s)` is a method call; `in` after `.` is
            // a member name.
            if matches!(p, In) && pi > 0 && matches!(self.tokens[pi - 1].kind, Dot) {
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

/// Render the stream. With a `width`, each source line that renders
/// wider is broken first ([`Emitter::wrap_line`]). Returns the text and
/// the offsets of the trailing commas the wrapper added, ascending.
fn emit(
    source: &str,
    tokens: &[Token],
    items: Vec<Item>,
    width: Option<usize>,
    commas: bool,
) -> (String, Vec<usize>) {
    let generic_angle = classify_generic_angles(tokens);
    let em = Emitter {
        source,
        tokens,
        cx: SpaceCx {
            tokens,
            generic_angle: &generic_angle,
        },
        commas,
        declaration_zone: declaration_zones(tokens),
    };
    let mut st = State {
        stack: Vec::new(),
        last_tok: None,
        first_line: true,
    };
    let mut out = Out::new(false);
    let mut i = 0;
    while i < items.len() {
        // One source line: the items up to the next newline.
        let mut j = i + 1;
        while j < items.len() && items[j].newlines_before == 0 {
            j += 1;
        }
        let mut line: Vec<Item> = items[i..j].to_vec();
        if let Some(w) = width {
            em.wrap_line(&st, &mut line, w);
        }
        em.render(&mut st, &line, &mut out);
        i = j;
    }

    // Exactly one trailing newline.
    let mut s = out.s;
    while s.ends_with('\n') || s.ends_with(' ') {
        s.pop();
    }
    s.push('\n');
    (s, out.commas)
}

/// What rendering carries from one line to the next.
#[derive(Clone)]
struct State {
    /// Bracket stack: (opener kind, indent of the opener's line).
    stack: Vec<(TokenKind, usize)>,
    /// Last token index emitted anywhere (for continuation logic).
    last_tok: Option<usize>,
    first_line: bool,
}

/// Where a token landed: its first and last physical line, and its
/// start and end column on them.
#[derive(Clone, Copy)]
struct TokCols {
    line: usize,
    start: usize,
    end_line: usize,
    end: usize,
}

/// The rendered text, and, for the wrapper, what it measures: each
/// physical line's code width (columns up to the end of its last code
/// token, so a trailing comment does not count, and 0 for a line inside
/// a multi-line token, whose text is not the code's to break), where
/// each wrapped segment starts, and where each token landed.
struct Out {
    s: String,
    col: usize,
    code: usize,
    widths: Vec<usize>,
    track: bool,
    seg_line: Vec<usize>,
    cols: Vec<Option<TokCols>>,
    commas: Vec<usize>,
}

impl Out {
    fn new(track: bool) -> Self {
        Out {
            s: String::new(),
            col: 0,
            code: 0,
            widths: Vec::new(),
            track,
            seg_line: Vec::new(),
            cols: Vec::new(),
            commas: Vec::new(),
        }
    }

    fn newline(&mut self) {
        self.widths.push(self.code);
        self.s.push('\n');
        self.col = 0;
        self.code = 0;
    }

    fn ws(&mut self, t: &str) {
        self.s.push_str(t);
        self.col += t.len();
    }

    /// Text that may span lines (a block string, a string with a raw
    /// line break, a block comment).
    fn text(&mut self, t: &str, is_code: bool) {
        let mut parts = t.split('\n');
        self.col += parts.next().unwrap_or("").chars().count();
        if is_code {
            self.code = self.col;
        }
        let rest: Vec<&str> = parts.collect();
        if let Some((last, interior)) = rest.split_last() {
            self.widths.push(self.code);
            self.widths.extend(interior.iter().map(|_| 0));
            self.col = last.chars().count();
            self.code = if is_code { self.col } else { 0 };
        }
        self.s.push_str(t);
    }

    fn finish(&mut self) {
        self.widths.push(self.code);
    }

    /// The widest physical line of segment `k`.
    fn seg_width(&self, k: usize) -> usize {
        let lo = self.seg_line[k];
        let hi = self.seg_line.get(k + 1).copied().unwrap_or(self.widths.len());
        self.widths[lo..hi.max(lo + 1).min(self.widths.len())]
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
    }
}

struct Emitter<'a> {
    source: &'a str,
    tokens: &'a [Token],
    cx: SpaceCx<'a>,
    /// Add a trailing comma to a call or record literal broken one
    /// element per line.
    commas: bool,
    /// Tokens inside a block whose parenthesised forms are
    /// declarations ([`declaration_zones`]).
    declaration_zone: Vec<bool>,
}

/// Mark the tokens inside each block whose parenthesised forms are
/// declarations, not calls: `claims { … }` and `constitution N { … }`
/// (claim forms, `forbid reaches(a, b)`), `closure N { … }`
/// (`persists_through(quarantine)`), `bindings { … }` (transports,
/// `unix("p")`), `bus { … }` (`subscribe E as h bounded(4, drop_old)`),
/// `enum { … }` (variant payloads, `Failed(String, Class)`) and an
/// annotation's arguments (`@effects(is: { knowledge(d) })`).
fn declaration_zones(tokens: &[Token]) -> Vec<bool> {
    use TokenKind::*;
    let mut zone = vec![false; tokens.len()];
    let mut i = 0;
    while i < tokens.len() {
        let lb = match &tokens[i].kind {
            // an annotation's arguments: `@effects(is: { knowledge(d) })`
            At if matches!(tokens.get(i + 1).map(|t| &t.kind), Some(Ident(_))) => i + 2,
            Ident(w) if matches!(w.as_str(), "claims" | "bindings" | "bus" | "enum") => i + 1,
            // `constitution N extends A, B {`, `closure within_band {`
            Ident(w) if w == "constitution" || w == "closure" => (i + 1..tokens.len())
                .find(|&j| !matches!(tokens[j].kind, Ident(_) | Comma))
                .unwrap_or(tokens.len()),
            _ => {
                i += 1;
                continue;
            }
        };
        let opener = if matches!(tokens[i].kind, At) { LParen } else { LBrace };
        if tokens.get(lb).map(|t| &t.kind) != Some(&opener) {
            i += 1;
            continue;
        }
        let mut depth = 0usize;
        let mut j = lb;
        while j < tokens.len() {
            match tokens[j].kind {
                LBrace | LParen | LBracket => depth += 1,
                RBrace | RParen | RBracket => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            zone[j] = true;
            j += 1;
        }
        i = j + 1;
    }
    zone
}

/// The segments of a line being wrapped: item ranges, each starting at
/// the line's first item or at a break.
fn segments(line: &[Item]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut a = 0;
    for k in 1..=line.len() {
        if k == line.len() || line[k].newlines_before > 0 {
            out.push((a, k));
            a = k;
        }
    }
    out
}

impl<'a> Emitter<'a> {
    fn render(&self, st: &mut State, items: &[Item], out: &mut Out) {
        use TokenKind::*;
        let mut cur_line_indent = 0usize;
        let mut line_start = true;
        // Index of the previous TOKEN emitted on the current line
        // (None right after a newline or when last emit was a comment).
        let mut prev_tok_on_line: Option<usize> = None;
        if out.track {
            out.cols = vec![None; items.len()];
        }

        for (idx, item) in items.iter().enumerate() {
            let newlines = item.newlines_before;
            if newlines > 0 {
                if !st.first_line {
                    out.newline();
                    if newlines >= 2 {
                        out.newline();
                    }
                }
                line_start = true;
                prev_tok_on_line = None;
            }
            if out.track && (idx == 0 || newlines > 0) {
                out.seg_line.push(out.widths.len());
            }

            match &item.elem {
                Elem::Tok(ti) => {
                    let tok = &self.tokens[*ti];
                    if line_start {
                        // Compute this line's indent from the bracket
                        // stack. A line whose first token closes a
                        // bracket gets the OPENER's line indent.
                        let indent = if matches!(tok.kind, RBrace | RParen | RBracket) {
                            st.stack.last().map(|(_, li)| *li).unwrap_or(0)
                        } else {
                            let base = st.stack.last().map(|(_, li)| *li + 1).unwrap_or(0);
                            base + usize::from(self.continues(st.last_tok, &tok.kind))
                        };
                        cur_line_indent = indent;
                        out.ws(&"    ".repeat(indent));
                        line_start = false;
                        st.first_line = false;
                    } else if let Some(pi) = prev_tok_on_line {
                        if self.cx.space_between(pi, *ti) {
                            out.ws(" ");
                        }
                    } else {
                        // After an inline comment on the same line —
                        // can't happen for line comments (they eat to
                        // EOL); block comment then code: one space.
                        out.ws(" ");
                    }
                    let (line, start) = (out.widths.len(), out.col);
                    let text = tok_text(self.source, tok);
                    if is_block_string(text) {
                        out.text(&reindent_block_string(text, cur_line_indent + 1), true);
                    } else {
                        out.text(text, true);
                    }
                    if out.track {
                        out.cols[idx] = Some(TokCols {
                            line,
                            start,
                            end_line: out.widths.len(),
                            end: out.col,
                        });
                    }

                    match tok.kind {
                        LBrace | LParen | LBracket => {
                            st.stack.push((tok.kind.clone(), cur_line_indent));
                        }
                        RBrace | RParen | RBracket => {
                            st.stack.pop();
                        }
                        _ => {}
                    }
                    prev_tok_on_line = Some(*ti);
                    st.last_tok = Some(*ti);
                }
                Elem::Comma => {
                    out.commas.push(out.s.len());
                    out.text(",", true);
                    prev_tok_on_line = None;
                }
                Elem::Comment(text) => {
                    if line_start {
                        // Own-line comment: indent like code. Closers on
                        // the NEXT line shouldn't affect it; use the
                        // normal non-closer rule.
                        let indent = st.stack.last().map(|(_, li)| *li + 1).unwrap_or(0);
                        out.ws(&"    ".repeat(indent));
                        line_start = false;
                        st.first_line = false;
                    } else {
                        // Trailing comment: one space after the code.
                        out.ws(" ");
                    }
                    // Multi-line block comments pass through verbatim;
                    // their interior lines keep author formatting.
                    out.text(text, false);
                    prev_tok_on_line = None;
                }
            }
        }
    }

    /// Bracket-less continuation: the previous line ended mid-expression
    /// (trailing binary op) or this line starts with one (`&&`, `.`).
    fn continues(&self, last_tok: Option<usize>, first: &TokenKind) -> bool {
        match (last_tok, first) {
            (_, k) if is_binary_op(k) => true,
            (_, TokenKind::Dot) => true,
            (Some(lt), _) => {
                let lk = &self.tokens[lt].kind;
                is_binary_op(lk) && !matches!(lk, TokenKind::FatArrow)
            }
            _ => false,
        }
    }

    /// Break one source line until every segment of it fits in `width`
    /// or has no break left. Parents break before children: each round
    /// takes the first segment that does not fit, so a call's arguments
    /// are only looked at once the call is broken.
    fn wrap_line(&self, st: &State, line: &mut Vec<Item>, width: usize) {
        loop {
            let mut probe = st.clone();
            let mut m = Out::new(true);
            self.render(&mut probe, line, &mut m);
            m.finish();
            let target = segments(line)
                .into_iter()
                .enumerate()
                .find(|&(k, (a, _))| !line[a].nobreak && m.seg_width(k) > width);
            let Some((_, (a, b))) = target else { return };
            if !self.break_segment(line, a, b, &m, width) {
                line[a].nobreak = true;
            }
        }
    }

    /// Is the `{` at token `ti` a record literal's? The parser's own
    /// test (`looks_like_struct_lit`): a name before it, and `}` or
    /// `name:` after it; plus no `;` directly inside, which a type's
    /// field list (`type T { a: Int; }`) has and a literal never does.
    fn is_record_brace(&self, ti: usize) -> bool {
        use TokenKind::*;
        let t = self.tokens;
        let named = ti > 0
            && match &t[ti - 1].kind {
                Ident(_) => true,
                Gt | Shr => self.cx.generic_angle[ti - 1],
                _ => false,
            };
        if !named {
            return false;
        }
        match t.get(ti + 1).map(|t| &t.kind) {
            Some(RBrace) => true,
            Some(k) if matches!(k, Ident(_)) || k.keyword_lexeme().is_some() => {
                matches!(t.get(ti + 2).map(|t| &t.kind), Some(Colon)) && !self.semi_inside(ti)
            }
            _ => false,
        }
    }

    /// A `;` directly inside the brace group opened at token `ti`.
    fn semi_inside(&self, ti: usize) -> bool {
        use TokenKind::*;
        let mut depth = 0usize;
        for t in &self.tokens[ti + 1..] {
            match t.kind {
                LBrace | LParen | LBracket => depth += 1,
                RBrace | RParen | RBracket => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                Semi if depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    /// Does the grammar take a trailing comma in the group opened at
    /// token `ti`? A call's arguments (`f(a, b,)`, after a callee) and a
    /// record literal's fields do (spec/grammar.ebnf `argument_list`,
    /// `struct_expr`). What only looks like a call does not: a
    /// declaration's parameters (`fn f(`, a hook's `on_failure(c: C, e:
    /// E)`, any list of `name: T`), a transport's keyword arguments
    /// (`unix("p", role: listen)`), a header's (`recognition(cap = 4)`),
    /// an annotation's (`@form(`), a pattern (`Result(r) =>` / `->`),
    /// `fallible(E)`, the one-expression `sum(` / `prod(`, and the forms
    /// of a declaration block ([`declaration_zones`]).
    fn takes_trailing_comma(&self, ti: usize) -> bool {
        use TokenKind::*;
        let t = self.tokens;
        if self.declaration_zone[ti] {
            return false;
        }
        match t[ti].kind {
            LBrace => return self.is_record_brace(ti),
            LParen if ti > 0 => {}
            _ => return false,
        }
        let callee = match &t[ti - 1].kind {
            // `sum(` / `prod(` take one expression, not an argument list;
            // a name after a word is a declaration's (`effect k(`,
            // `epoch duration(`), but for `x or fallback(…)`.
            Ident(w) if matches!(w.as_str(), "fallible" | "sum" | "prod") => false,
            Ident(_) => {
                ti < 2
                    || match &t[ti - 2].kind {
                        Fn | At => false,
                        Ident(w) => w == "or",
                        _ => true,
                    }
            }
            RParen | RBracket | KwSelf => true,
            Gt => self.cx.generic_angle[ti - 1],
            _ => false,
        };
        if !callee {
            return false;
        }
        // Walk the group: an element that opens with `name:` is a
        // parameter or a keyword argument; what follows the closer
        // tells a pattern from a call.
        let mut depth = 0usize;
        let mut element_start = true;
        for j in ti + 1..t.len() {
            let k = &t[j].kind;
            if depth == 0 && element_start {
                let named = matches!(k, Ident(_)) || k.keyword_lexeme().is_some();
                if named && matches!(t.get(j + 1).map(|t| &t.kind), Some(Colon | Eq)) {
                    return false;
                }
            }
            element_start = false;
            match k {
                LParen | LBracket | LBrace => depth += 1,
                RParen | RBracket | RBrace if depth > 0 => depth -= 1,
                RParen => {
                    return !matches!(t.get(j + 1).map(|t| &t.kind), Some(Arrow | FatArrow));
                }
                Comma if depth == 0 => element_start = true,
                _ => {}
            }
        }
        false
    }

    /// Choose and place the breaks for one segment of a line too wide
    /// for `width`, outermost shape first (spec/testing.md § `hale fmt`):
    ///
    /// 1. statements on one line: one per line;
    /// 2. a block written on one line: its statements one per line;
    /// 3. a list the author already broke: one element per line;
    /// 4. an `||` chain, else an `&&` chain: one term per line, the
    ///    operator leading (`||` binds loosest, so its terms go first);
    ///    else a `+`/`-` chain: broken at the operators, as many terms
    ///    to a line as fit; a block's `{` after the chain goes on a line
    ///    of its own;
    /// 5. the widest call, record literal, array or parenthesis: one
    ///    element per line, a call's and a literal's with a trailing
    ///    comma;
    /// 6. a bracket left open at the end of the line: one element per
    ///    line after it.
    ///
    /// Returns false when the segment has none of these.
    fn break_segment(&self, line: &mut Vec<Item>, a: usize, b: usize, m: &Out, width: usize) -> bool {
        use TokenKind::*;
        // The segment's tokens: (item index, token index).
        let toks: Vec<(usize, usize)> = (a..b)
            .filter_map(|k| match line[k].elem {
                Elem::Tok(t) => Some((k, t)),
                _ => None,
            })
            .collect();
        let n = toks.len();
        if n < 2 {
            return false;
        }
        let kind = |p: usize| &self.tokens[toks[p].1].kind;

        // Bracket structure local to the segment. `depth[p]` counts the
        // groups opened in the segment around token p (an opener is
        // outside its own group, a closer too); `angle[p]` the generic
        // angles, whose commas separate nothing here.
        let mut depth = vec![0usize; n];
        let mut angle = vec![0usize; n];
        let mut close_of: Vec<Option<usize>> = vec![None; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut angles = 0usize;
        for p in 0..n {
            let ti = toks[p].1;
            match kind(p) {
                LParen | LBracket | LBrace => {
                    depth[p] = stack.len();
                    stack.push(p);
                }
                RParen | RBracket | RBrace => {
                    if let Some(o) = stack.pop() {
                        close_of[o] = Some(p);
                    }
                    depth[p] = stack.len();
                }
                _ => depth[p] = stack.len(),
            }
            match kind(p) {
                Lt if self.cx.generic_angle[ti] => angles += 1,
                Gt if self.cx.generic_angle[ti] => angles = angles.saturating_sub(1),
                Shr if self.cx.generic_angle[ti] => angles = angles.saturating_sub(2),
                _ => {}
            }
            angle[p] = angles;
        }
        let unclosed = stack.first().copied();
        let top = |p: usize| depth[p] == 0 && angle[p] == 0;
        // Item indices to break before, and items to put a comma after.
        let mut brk: Vec<usize> = Vec::new();
        let mut commas: Vec<usize> = Vec::new();
        let after = |p: usize| toks[p].0 + 1;
        let is_block = |p: usize| matches!(kind(p), LBrace) && !self.is_record_brace(toks[p].1);
        // A block's `{` that ends the segment, opened after a condition.
        let trailing_block = unclosed.filter(|&u| u == n - 1 && is_block(u));

        // 1. Statements.
        for p in 0..n - 1 {
            if top(p) && matches!(kind(p), Semi) {
                brk.push(after(p));
            }
        }
        if apply_breaks(line, a, b, &brk, &commas) {
            return true;
        }

        // 2. Blocks written on one line.
        for p in 0..n {
            let Some(c) = close_of[p] else { continue };
            if !(top(p) && is_block(p)) {
                continue;
            }
            if c > p + 1 {
                brk.push(after(p));
                for q in p + 1..c - 1 {
                    if depth[q] == 1 && angle[q] == angle[p] && matches!(kind(q), Semi | Comma) {
                        brk.push(after(q));
                    }
                }
                brk.push(toks[c].0);
            }
            // `} fn next() {`: what follows a block on its line starts a
            // line of its own, but for `} else`.
            if c + 1 < n {
                let next = kind(c + 1);
                if (matches!(next, Ident(_)) || next.keyword_lexeme().is_some()) && !matches!(next, Else) {
                    brk.push(after(c));
                }
            }
        }
        if apply_breaks(line, a, b, &brk, &commas) {
            return true;
        }

        // 3. A list already broken by its author.
        for p in 0..n - 1 {
            if top(p) && matches!(kind(p), Comma) {
                brk.push(after(p));
            }
        }
        if apply_breaks(line, a, b, &brk, &commas) {
            return true;
        }

        // 4. Chains.
        let ops = |want: fn(&TokenKind) -> bool| -> Vec<usize> {
            (1..n).filter(|&p| top(p) && want(kind(p))).collect()
        };
        let mut chain = ops(|k| matches!(k, OrOr));
        if chain.is_empty() {
            chain = ops(|k| matches!(k, AndAnd));
        }
        if !chain.is_empty() {
            brk.extend(chain.iter().map(|&p| toks[p].0));
            brk.extend(trailing_block.map(|u| toks[u].0));
            if apply_breaks(line, a, b, &brk, &commas) {
                return true;
            }
        }
        let adds: Vec<usize> = (1..n)
            .filter(|&p| {
                top(p) && matches!(kind(p), Plus | Minus) && self.cx.ends_operand_at(toks[p - 1].1)
            })
            .collect();
        if !adds.is_empty() {
            self.greedy_breaks(&toks, &adds, m, width, &mut brk);
            brk.extend(trailing_block.map(|u| toks[u].0));
            if apply_breaks(line, a, b, &brk, &commas) {
                return true;
            }
        }

        // 5. The widest closed group.
        let mut best: Option<(usize, usize)> = None;
        for p in 0..n {
            let Some(c) = close_of[p] else { continue };
            if !top(p) || c <= p + 1 || is_block(p) {
                continue;
            }
            // `a[i]` is an index, not a list.
            if matches!(kind(p), LBracket) && p > 0 && self.cx.ends_operand_at(toks[p - 1].1) {
                continue;
            }
            let w = match (m.cols[toks[p].0], m.cols[toks[c].0]) {
                (Some(o), Some(e)) if o.line == e.end_line => e.end - o.start,
                _ => usize::MAX,
            };
            if best.is_none_or(|(bw, _)| w >= bw) {
                best = Some((w, p));
            }
        }
        if let Some((_, p)) = best {
            let c = close_of[p].expect("a closed group");
            brk.push(after(p));
            for q in p + 1..c - 1 {
                if depth[q] == 1 && angle[q] == angle[p] && matches!(kind(q), Comma) {
                    brk.push(after(q));
                }
            }
            brk.push(toks[c].0);
            if self.commas
                && !matches!(kind(c - 1), Comma)
                && self.takes_trailing_comma(toks[p].1)
            {
                commas.push(toks[c - 1].0);
            }
            if apply_breaks(line, a, b, &brk, &commas) {
                return true;
            }
            commas.clear();
        }

        // 6. A bracket left open at the end of the line.
        if let Some(u) = unclosed {
            if u < n - 1 {
                brk.push(after(u));
                for q in u + 1..n - 1 {
                    if depth[q] == 1 && angle[q] == angle[u] && matches!(kind(q), Comma | Semi) {
                        brk.push(after(q));
                    }
                }
                if apply_breaks(line, a, b, &brk, &commas) {
                    return true;
                }
            }
        }
        false
    }

    /// A `+`/`-` chain's breaks: as many terms on a line as fit in
    /// `width`, a break before the operator of the first that does not,
    /// the next line one level in. Widths come from the segment as
    /// rendered on one line; the wrapper measures again after, so an
    /// estimate that is off only moves where the next round breaks.
    fn greedy_breaks(&self, toks: &[(usize, usize)], ops: &[usize], m: &Out, width: usize, brk: &mut Vec<usize>) {
        let cols = |p: usize| m.cols[toks[p].0];
        let n = toks.len();
        let Some(first) = cols(0) else { return };
        let cont = first.start
            + if is_binary_op(&self.tokens[toks[0].1].kind)
                || self.continues(toks[0].1.checked_sub(1), &self.tokens[toks[0].1].kind)
            {
                0
            } else {
                4
            };
        let piece = |from: usize, to: usize| -> usize {
            match (cols(from), cols(to)) {
                (Some(s), Some(e)) if s.line == e.end_line => e.end - s.start,
                _ => usize::MAX / 4,
            }
        };
        let mut cur = piece(0, ops[0] - 1) + first.start;
        let before = brk.len();
        for (k, &op) in ops.iter().enumerate() {
            let end = ops.get(k + 1).map_or(n - 1, |&next| next - 1);
            let w = piece(op, end);
            if cur + 1 + w > width {
                brk.push(toks[op].0);
                cur = cont + w;
            } else {
                cur += 1 + w;
            }
        }
        if brk.len() == before {
            // Every term fits by the estimate and the line still does
            // not: the last operator is the break that shortens it.
            brk.push(toks[*ops.last().expect("a chain")].0);
        }
    }
}

/// Put breaks before the items in `brk` and commas after those in
/// `commas`, within the segment `a..b`. True when a break was new.
fn apply_breaks(line: &mut Vec<Item>, a: usize, b: usize, brk: &[usize], commas: &[usize]) -> bool {
    let mut any = false;
    for &k in brk {
        if k > a && k < b && line[k].newlines_before == 0 {
            line[k].newlines_before = 1;
            any = true;
        }
    }
    if !any {
        return false;
    }
    let mut commas = commas.to_vec();
    commas.sort_unstable();
    commas.dedup();
    for &k in commas.iter().rev() {
        line.insert(
            k + 1,
            Item {
                elem: Elem::Comma,
                newlines_before: 0,
                nobreak: false,
            },
        );
    }
    true
}

/// A block string's raw text: it opens with a fence of three or more
/// quotes (a plain string's opening quote is never followed by two
/// more, `""` being the empty string).
fn is_block_string(raw: &str) -> bool {
    raw.starts_with("\"\"\"")
}

/// A block string re-indented so its closing fence stands at `level`
/// (spec/tokens.md § Block strings). Every line of the text keeps its
/// distance from the fence, a line of whitespace alone becomes empty, a
/// `\r\n` becomes `\n` and whitespace after the opening fence goes:
/// none of which the value sees.
fn reindent_block_string(raw: &str, level: usize) -> String {
    let fence = raw.bytes().take_while(|&b| b == b'"').count();
    let after_open = raw.find('\n').map_or(raw.len(), |i| i + 1);
    let rest = &raw[after_open..];
    let (body, close) = match rest.rfind('\n') {
        Some(i) => (Some(&rest[..i]), &rest[i + 1..]),
        None => (None, rest),
    };
    let old_indent = &close[..close.len() - fence];
    let new_indent = "    ".repeat(level);
    let mut out = String::with_capacity(raw.len() + 16);
    out.push_str(&raw[..fence]);
    out.push('\n');
    if let Some(body) = body {
        for line in body.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if !line.bytes().all(|c| c == b' ' || c == b'\t') {
                out.push_str(&new_indent);
                out.push_str(&line[old_indent.len()..]);
            }
            out.push('\n');
        }
    }
    out.push_str(&new_indent);
    out.push_str(&raw[..fence]);
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
        let (ra, rb) = (tok_text(src_a, ta), tok_text(src_b, tb));
        if is_block_string(ra) || is_block_string(rb) {
            // Re-indented: the same form and the same value.
            if !(is_block_string(ra) && is_block_string(rb) && ta.kind == tb.kind) {
                return false;
            }
        } else if ra != rb {
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
    fn binary_op_after_a_keyword_named_field_stays_spaced() {
        // GH #1064: after `.` a reserved word is a field name, so the
        // operator after it is binary, not unary.
        let out = fmt("fn f(err: ClosureViolation) -> String {\n    return err.closure + \" \";\n}\n");
        assert!(out.contains("err.closure + \" \""), "{}", out);
        let out = fmt("fn g(x: T) -> Int {\n    return x.epoch - 1 + x.tier + x.run;\n}\n");
        assert!(out.contains("x.epoch - 1 + x.tier + x.run"), "{}", out);
        // a keyword NOT after `.` still starts an operand: `return -1`
        let out = fmt("fn h() -> Int {\n    return -1;\n}\n");
        assert!(out.contains("return -1;"), "{}", out);
    }

    /// `Box<Decimal<18>>`: the lexer reads the two closers as one `>>`,
    /// which still closes two generic levels tight; a shift stays one.
    #[test]
    fn nested_generic_closers_stay_tight() {
        let out = fmt("type P {\n    b: Box<Decimal<18>>;\n    c: Box<Box<Int>>;\n}\nfn f(a: Int, b: Int) -> Bool {\n    return a < b >> 1;\n}\n");
        assert_eq!(
            out,
            "type P {\n    b: Box<Decimal<18>>;\n    c: Box<Box<Int>>;\n}\nfn f(a: Int, b: Int) -> Bool {\n    return a < b >> 1;\n}\n"
        );
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

    /// A block string's lines move with the code: the closing fence one
    /// level deeper than the line the literal opens on, every line of
    /// the text at the same distance from it, whitespace-only lines
    /// emptied. The value is what the gate holds, and it holds.
    #[test]
    fn block_string_moves_with_the_code() {
        let src = "fn main() {\nlet p = \"\"\"   \n  fn main() {\n      println(\"a\\n\");\n  \n  }\n\n  \"\"\";\n        println(p);\n}\n";
        let want = "fn main() {\n    let p = \"\"\"\n        fn main() {\n            println(\"a\\n\");\n\n        }\n\n        \"\"\";\n    println(p);\n}\n";
        let out = fmt(src);
        assert_eq!(out, want);
        assert_eq!(fmt(&out), out, "a fixed point");
        let value = |s: &str| {
            lex(s)
                .unwrap()
                .into_iter()
                .find_map(|t| match t.kind {
                    TokenKind::StringLit(v) if v.contains("main") => Some(v),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(value(src), value(&out));
        assert_eq!(value(&out), "fn main() {\n    println(\"a\\n\");\n\n}\n");
    }

    /// The same at a deeper opening line and as a call's argument, with
    /// a longer fence; tokens after the closing fence stay on its line.
    #[test]
    fn block_string_as_an_argument() {
        let src = "fn main() {\n    if true {\n        check(\"\"\"\"\n  x \"\"\"\n  \"\"\"\", 1);\n    }\n}\n";
        let want = "fn main() {\n    if true {\n        check(\"\"\"\"\n            x \"\"\"\n            \"\"\"\", 1);\n    }\n}\n";
        assert_eq!(fmt(src), want);
        assert_eq!(fmt(want), want);
    }

    /// The gate compares a block string by value: a different value is
    /// refused even when the raw text is close.
    #[test]
    fn gate_compares_block_strings_by_value() {
        let a = "let s = \"\"\"\n    x\n    \"\"\";\n";
        let b = "let s = \"\"\"\n    y\n    \"\"\";\n";
        let ta = lex(a).unwrap();
        let tb = lex(b).unwrap();
        assert!(!token_streams_equal(a, &ta, b, &tb));
        let c = "let s = \"\"\"\n        x\n        \"\"\";\n";
        let tc = lex(c).unwrap();
        assert!(token_streams_equal(a, &ta, c, &tc));
    }

    /// `hale fmt --wrap`, held to its own fixed point.
    fn wrap(s: &str) -> String {
        let once = format_source_wrapped(s).expect("wraps");
        assert_eq!(format_source_wrapped(&once).expect("wraps again"), once, "not a fixed point");
        once
    }

    /// A line that fits is untouched, however it could be broken; one
    /// column more and it is broken.
    #[test]
    fn wrap_leaves_a_line_that_fits() {
        let fits = format!("fn f() {{\n    g(aaaa, bbbb, \"{}\");\n}}\n", "x".repeat(100 - 22));
        assert_eq!(fits.lines().nth(1).unwrap().len(), 100);
        assert_eq!(wrap(&fits), fits);
        let wide = fits.replace("aaaa", "aaaaa");
        assert_eq!(wrap(&wide).lines().count(), 7, "{}", wrap(&wide));
    }

    /// An `&&` chain: one term per line, the operator leading, one level
    /// in; the block's `{` on a line of its own, so the body sits one
    /// level under the `if` and the `}` under the `if`.
    #[test]
    fn wrap_an_and_chain() {
        let src = "fn f(a: Int) -> Bool {\n    if alpha_condition(a) && beta_condition(a) && gamma_condition(a, a) && delta_condition(a, a, a) {\n        return true;\n    }\n    return false;\n}\n";
        let want = "fn f(a: Int) -> Bool {\n    if alpha_condition(a)\n        && beta_condition(a)\n        && gamma_condition(a, a)\n        && delta_condition(a, a, a)\n    {\n        return true;\n    }\n    return false;\n}\n";
        assert_eq!(wrap(src), want);
    }

    /// `||` binds loosest, so its terms are the chain's; an `&&` inside
    /// a term stays on the term's line while it fits.
    #[test]
    fn wrap_an_or_chain() {
        let src = "fn f(a: Int, b: Int) -> Bool {\n    let ok = first_long_predicate(a) || second_long_predicate(b) && third_long_predicate(a, b) || fourth(a);\n    return ok;\n}\n";
        let want = "fn f(a: Int, b: Int) -> Bool {\n    let ok = first_long_predicate(a)\n        || second_long_predicate(b) && third_long_predicate(a, b)\n        || fourth(a);\n    return ok;\n}\n";
        assert_eq!(wrap(src), want);
    }

    /// A `+` chain breaks at the operator, as many terms to a line as
    /// fit.
    #[test]
    fn wrap_a_plus_chain() {
        let src = "fn f(a: Int, b: Int) -> String {\n    let message = \"the first part of the message \" + to_string(a) + \" and the second part \" + to_string(b) + \" end\";\n    return message;\n}\n";
        let want = "fn f(a: Int, b: Int) -> String {\n    let message = \"the first part of the message \" + to_string(a) + \" and the second part \"\n        + to_string(b) + \" end\";\n    return message;\n}\n";
        assert_eq!(wrap(src), want);
    }

    /// A call: one argument per line, a trailing comma added (and not
    /// doubled where the author wrote one); an argument that fits stays
    /// whole, one that does not breaks in turn.
    #[test]
    fn wrap_a_call() {
        let src = "fn f() {\n    let r = build_the_record(alpha_argument_value, beta_argument_value, gamma_argument_value, delta);\n    let t = outer_call(inner_call(first_argument_value, second_argument_value), third_argument_value_xy,);\n    let u = outer_call(inner_call(first_argument_value_that_is_long, second_argument_value_that_is_longer, third_one), x);\n}\n";
        let want = "fn f() {\n    let r = build_the_record(\n        alpha_argument_value,\n        beta_argument_value,\n        gamma_argument_value,\n        delta,\n    );\n    let t = outer_call(\n        inner_call(first_argument_value, second_argument_value),\n        third_argument_value_xy,\n    );\n    let u = outer_call(\n        inner_call(\n            first_argument_value_that_is_long,\n            second_argument_value_that_is_longer,\n            third_one,\n        ),\n        x,\n    );\n}\n";
        assert_eq!(wrap(src), want);
    }

    /// A record literal: one field per line with a trailing comma; a
    /// nested literal that fits stays on its field's line.
    #[test]
    fn wrap_a_record_literal() {
        let src = "fn f(a: Int, b: Int) {\n    let rec = Record { first_field: compute(a), second_field: compute(b), nested: Inner { x: 1, y: 2 } };\n}\n";
        let want = "fn f(a: Int, b: Int) {\n    let rec = Record {\n        first_field: compute(a),\n        second_field: compute(b),\n        nested: Inner { x: 1, y: 2 },\n    };\n}\n";
        assert_eq!(wrap(src), want);
    }

    /// Comments inside a wrapped construct stay with what they were
    /// written beside: an inline block comment with the argument after
    /// it, a trailing line comment at the end of the statement, and an
    /// own-line comment in a list the author already broke.
    #[test]
    fn wrap_keeps_comments_in_place() {
        let src = "fn f() {\n    call_with_comments(first_argument_value, /* the second */ second_argument_value, third_value_here); // trailing note\n    let t = Thing {\n        // the first field\n        first: 1, second_field_value_here: computed_value(alpha, beta, gamma), third: three_value_xyz,\n    };\n}\n";
        let want = "fn f() {\n    call_with_comments(\n        first_argument_value,\n        /* the second */ second_argument_value,\n        third_value_here,\n    ); // trailing note\n    let t = Thing {\n        // the first field\n        first: 1,\n        second_field_value_here: computed_value(alpha, beta, gamma),\n        third: three_value_xyz,\n    };\n}\n";
        assert_eq!(wrap(src), want);
    }

    /// A line with nothing to break at stays as it is, past the width:
    /// a string literal is never split.
    #[test]
    fn wrap_keeps_a_line_it_cannot_break() {
        let src = "fn f() {\n    let s = \"a string literal so long that no break can bring this line under the width the formatter holds\";\n}\n";
        assert_eq!(wrap(src), src);
    }

    /// A block written on one line: its statements one per line; a
    /// declaration's parameters one per line, with no trailing comma
    /// (the grammar takes none there).
    #[test]
    fn wrap_a_block_and_a_signature() {
        let src = "fn a_function_with_a_long_signature(first_parameter: Int, second_parameter: String, third: Bool) -> Int { return first_parameter; }\nfn g(x: Int) -> Int { let first_value = compute_something(x); let second = other(first_value); return second; }\n";
        let want = "fn a_function_with_a_long_signature(\n    first_parameter: Int,\n    second_parameter: String,\n    third: Bool\n) -> Int {\n    return first_parameter;\n}\nfn g(x: Int) -> Int {\n    let first_value = compute_something(x);\n    let second = other(first_value);\n    return second;\n}\n";
        assert_eq!(wrap(src), want);
    }

    /// A block string as an argument of a broken call: its own line,
    /// re-indented under it; its text never counts toward the width.
    #[test]
    fn wrap_a_block_string_argument() {
        let src = format!(
            "fn f() {{\n    check(\"\"\"\n        {}\n        \"\"\", \"the label that makes it too wide, and then some more words to be sure of it, many more\");\n}}\n",
            "t".repeat(120)
        );
        let want = format!(
            "fn f() {{\n    check(\n        \"\"\"\n            {}\n            \"\"\",\n        \"the label that makes it too wide, and then some more words to be sure of it, many more\",\n    );\n}}\n",
            "t".repeat(120)
        );
        assert_eq!(wrap(&src), want);
    }
}
