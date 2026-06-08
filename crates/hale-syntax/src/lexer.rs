//! Lexer for Hale source.
//!
//! Produces a stream of [`Token`] values from a source string per
//! `spec/tokens.md`. Hand-written; zero external dependencies.

use crate::error::Diag;
use crate::span::Span;

/// A token produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Token { kind, span }
    }
}

/// v1.x-10: one segment of an f-string body. The lexer splits an
/// `f"..."` literal into a sequence of these so the parser doesn't
/// have to rescan for `{...}` boundaries (and `{{` / `}}` escapes
/// are already resolved into the Lit variant).
#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    /// Literal text fragment (escape-processed; braces unescaped).
    Lit(String),
    /// Raw text between `{` and `}`. Parsed as an Hale expression
    /// at parse time; an empty Interp is a lex error.
    Interp(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Identifiers and literals
    Ident(String),
    IntLit(i64),
    FloatLit(f64),
    /// Decimal literal carries the raw text (without the `d` suffix).
    DecimalLit(String),
    /// String literal payload (already escape-processed).
    StringLit(String),
    /// v1.x-10 f-string: pre-split sequence of literal text + raw
    /// interpolation-body parts. The parser sub-parses each Interp
    /// part as a small expression and desugars the whole token to
    /// `Lit + to_string(expr) + Lit + ...` joined by `+`.
    FStringLit(Vec<FStringPart>),
    /// Bytes literal payload.
    BytesLit(Vec<u8>),
    /// Duration literal in nanoseconds.
    DurationLit(i64),
    /// Time literal carries the raw ISO-8601 text (without backticks).
    TimeLit(String),

    // Keywords — declaration
    Locus,
    Perspective,
    Type,
    Const,
    Fn,
    Import,
    Export,
    Module,

    // Keywords — locus members
    Params,
    Contract,
    Bus,
    /// F.22 `capacity { ... }` block introducer. Inside the
    /// block, `pool` and `heap` (slot kinds) stay as plain
    /// idents and are recognized contextually by the parser —
    /// frees the math-shaped identifier pool outside this block
    /// (matches the `approx`/`within` precedent from F.10).
    Capacity,
    /// F.22 v1.x-4: slot-decl trailing clause introducer.
    /// `pool entries of T as_parent_for ChildL;` overrides
    /// accepted ChildL's same-named slot to share this allocator.
    AsParentFor,
    /// v1.x-FORM-4: slot-decl trailing clause introducer for
    /// `@form(hashmap)`. `pool entries of T indexed_by name;`
    /// says the cell type T has a field `name` that serves as
    /// the hashmap key. Only meaningful on a `@form(hashmap)`
    /// locus; ignored elsewhere (typecheck flags misuse).
    IndexedBy,

    // Keywords — lifecycle
    Birth,
    Accept,
    Run,
    Drain,
    Dissolve,
    OnFailure,

    // Keywords — mode. `mode` itself is contextual; see
    // `parse_locus_member`.
    Bulk,
    Harmonic,
    Resolution,

    // Keywords — projection class
    Projection,
    Rich,
    Chunked,
    Recognition,

    // Keywords — placement classes (F.31, 2026-05-23).
    //
    // Placement is bimodal: cooperative (shared pool thread) or
    // pinned (own OS thread). The keywords appear inside the
    // `placement { field: SPEC; }` block on `main locus`.
    //
    // `placement`, `cooperative`, `pinned`, `pool`, `core` are
    // all contextual idents — they lex as Ident and the parser
    // recognizes them positionally inside the placement block.
    // This frees the identifiers for use as fn / var / field
    // names outside placement contexts. Same F.10-style
    // narrowing the closure / mode keyword families use.
    //
    // Pre-F.31 surface had a per-locus `: schedule X` annotation
    // with TokenKind::Schedule + TokenKind::Cooperative +
    // TokenKind::Pinned. F.31 removed that surface; the tokens
    // are gone. Existing source using `: schedule ...` is now a
    // parse error (intentional — schedule moved to main).

    // Keywords — closure
    //
    // F.10-style contextual narrowing (2026-05-11): `approx` and
    // `within` are NOT reserved at the lexer level — they lex as
    // Ident. The parser recognizes them inside closure-block
    // bodies only (see parse_closure_assertion). This frees the
    // math-shaped identifier pool (`fn approx(...)`, `let within
    // = ...`) outside that context. `closure`, `epoch`,
    // `persists_through`, `resets_on`, and `resets_per_epoch`
    // stay reserved because they are unambiguously block-
    // introducers / clause-leaders.
    Closure,
    Epoch,
    PersistsThrough,
    ResetsOn,
    ResetsPerEpoch,

    // Keywords — recovery primitives
    Restart,
    RestartInPlace,
    Quarantine,
    Reorganize,
    Bubble,

    // Keywords — contract
    Expose,
    Consume,
    Inferred,

    // Keywords — bus
    Subscribe,
    Publish,
    On,
    Of,

    // Keywords — perspective
    StableWhen,
    SerializeAs,

    // Keywords — statement / expression
    Let,
    Mut,
    If,
    Else,
    Match,
    For,
    In,
    While,
    Return,
    Break,
    Continue,
    True,
    False,
    Nil,
    Tier,
    KwSelf,
    Interface,

    // (Primitive type names are not keywords. They are predefined
    // identifiers — `Int`, `Uint`, `Float`, `Decimal`, `String`,
    // `Bool`, `Time`, `Duration`, `Bytes` — recognized in type
    // position by the parser. Lowercase forms have no language
    // meaning and may be used as ordinary identifiers / namespaces
    // (e.g. `time::sleep`).)

    // Keywords — reserved (parse-error if used)
    Trait,
    Impl,
    Async,
    Await,
    Yield,
    Terminate,
    Release,
    Macro,
    Where,
    // v1.x-VIOLATE (F.27): `with` is no longer reserved at the
    // lexer level. The lexer emits it as an Ident; the parser
    // recognizes it contextually inside `violate_stmt`.

    // Operators / punctuation
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Eq,         // =
    EqEq,       // ==
    NotEq,      // !=
    Lt,         // <
    Gt,         // >
    LtEq,       // <=
    GtEq,       // >=
    AndAnd,     // &&
    OrOr,       // ||
    Bang,       // !
    Amp,        // &
    Pipe,       // |
    Caret,      // ^
    Tilde,      // ~
    Shl,        // <<
    Shr,        // >>
    LeftArrow,  // <-  (bus send: `"subject" <- msg`)
    PlusEq,     // +=
    MinusEq,    // -=
    StarEq,     // *=
    SlashEq,    // /=
    PercentEq,  // %=
    AmpEq,      // &=
    PipeEq,     // |=
    CaretEq,    // ^=
    TildeTilde, // ~~  (closure approx)
    Arrow,      // ->
    FatArrow,   // =>
    ColonColon, // ::
    DotDot,     // ..  (range; reserved)
    DotDotEq,   // ..= (range; reserved)
    Question,   // ?  (reserved)
    QuestionQuestion, // ?? (reserved)
    Colon,      // :
    Semi,       // ;
    Comma,      // ,
    Dot,        // .
    LBrace,     // {
    RBrace,     // }
    LParen,     // (
    RParen,     // )
    LBracket,   // [
    RBracket,   // ]
    At,         // @  (reserved)
    Hash,       // #  (reserved)
    Dollar,     // $  (reserved)

    /// End-of-file. Always the last token.
    Eof,
}

/// Lex a source string into tokens. Returns either a token stream
/// (with [`TokenKind::Eof`] appended) or one or more diagnostics.
pub fn lex(source: &str) -> Result<Vec<Token>, Vec<Diag>> {
    let mut lx = Lexer::new(source);
    let mut tokens = Vec::new();
    let mut diags = Vec::new();
    loop {
        match lx.next_token() {
            Ok(Some(tok)) => {
                let is_eof = matches!(tok.kind, TokenKind::Eof);
                tokens.push(tok);
                if is_eof {
                    break;
                }
            }
            Ok(None) => continue, // skipped (whitespace / comment)
            Err(d) => {
                diags.push(d);
                lx.skip_one(); // recover by advancing past the bad byte
            }
        }
    }
    if diags.is_empty() {
        Ok(tokens)
    } else {
        Err(diags)
    }
}

struct Lexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Lexer {
            source,
            bytes: source.as_bytes(),
            pos: 0,
        }
    }

    fn at_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u8> {
        self.bytes.get(self.pos + n).copied()
    }

    #[allow(dead_code)]
    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_one(&mut self) {
        if !self.at_eof() {
            self.pos += 1;
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(b) = self.peek() {
                        if b == b'\n' {
                            break;
                        }
                        // spec/tokens.md permits non-ASCII inside
                        // comments. A bare `pos += 1` lands in the
                        // middle of a multi-byte UTF-8 sequence on
                        // chars like em-dash (—) or box-draw (─),
                        // which then trips the downstream
                        // `&source[..]` slice on a char-boundary
                        // check (brained FRICTION F.8). Advance by
                        // UTF-8 char width when the leading byte
                        // is non-ASCII.
                        if b < 0x80 {
                            self.pos += 1;
                        } else {
                            let ch = self.source[self.pos..]
                                .chars()
                                .next()
                                .expect("non-ASCII leading byte implies a char");
                            self.pos += ch.len_utf8();
                        }
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    self.pos += 2;
                    while let Some(b) = self.peek() {
                        if b == b'*' && self.peek_at(1) == Some(b'/') {
                            self.pos += 2;
                            break;
                        }
                        if b < 0x80 {
                            self.pos += 1;
                        } else {
                            let ch = self.source[self.pos..]
                                .chars()
                                .next()
                                .expect("non-ASCII leading byte implies a char");
                            self.pos += ch.len_utf8();
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, Diag> {
        self.skip_ws_and_comments();
        if self.at_eof() {
            let span = Span::new(self.pos, self.pos);
            return Ok(Some(Token::new(TokenKind::Eof, span)));
        }

        let start = self.pos;
        let b = self.peek().unwrap();

        // v1.x-10 f-string literal: `f"..."`. Must precede the
        // generic ident path so the lone `f` doesn't lex as an
        // identifier when followed by an opening quote.
        if b == b'f' && self.source.as_bytes().get(self.pos + 1) == Some(&b'"') {
            self.pos += 1; // consume the leading `f`
            return self.lex_fstring(start).map(Some);
        }

        // B2 / G5: bytes literal `b"..."`. Same body as a string
        // literal, but escapes pass through as raw bytes (no UTF-
        // 8 promotion) so `\xNN` accepts the full 0x00..0xFF range.
        // Must precede the generic ident path so a lone `b`
        // followed by `"` doesn't lex as an identifier.
        if b == b'b' && self.source.as_bytes().get(self.pos + 1) == Some(&b'"') {
            self.pos += 1; // consume the leading `b`
            return self.lex_bytes(start).map(Some);
        }

        // Identifier or keyword
        if b.is_ascii_alphabetic() || b == b'_' {
            return Ok(Some(self.lex_ident_or_keyword(start)));
        }

        // Numeric literal (digit-led). Could be int, float, decimal, duration.
        if b.is_ascii_digit() {
            return self.lex_number(start).map(Some);
        }

        // String literal
        if b == b'"' {
            return self.lex_string(start).map(Some);
        }

        // Time literal
        if b == b'`' {
            return self.lex_time(start).map(Some);
        }

        // Operators / punctuation
        Ok(Some(self.lex_op_or_punct(start)?))
    }

    fn lex_ident_or_keyword(&mut self, start: usize) -> Token {
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = &self.source[start..self.pos];
        let span = Span::new(start, self.pos);

        let kind = match text {
            // Declaration
            "locus" => TokenKind::Locus,
            "perspective" => TokenKind::Perspective,
            "type" => TokenKind::Type,
            "const" => TokenKind::Const,
            "fn" => TokenKind::Fn,
            "import" => TokenKind::Import,
            "export" => TokenKind::Export,
            "module" => TokenKind::Module,
            // `topic` is contextually keyworded — recognized only
            // in top-level declaration position (see
            // `parse_top_decl`). Lexing it always as Ident frees
            // the identifier for use as a struct-field name etc.
            // Same pattern as `approx`/`within`/`pool`/`heap`.

            // Locus members
            "params" => TokenKind::Params,
            "contract" => TokenKind::Contract,
            "bus" => TokenKind::Bus,
            "capacity" => TokenKind::Capacity,
            "as_parent_for" => TokenKind::AsParentFor,
            "indexed_by" => TokenKind::IndexedBy,

            // Lifecycle
            "birth" => TokenKind::Birth,
            "accept" => TokenKind::Accept,
            "run" => TokenKind::Run,
            "drain" => TokenKind::Drain,
            "dissolve" => TokenKind::Dissolve,
            "on_failure" => TokenKind::OnFailure,

            // Mode. `mode` is contextually keyworded — recognized
            // only at locus-member position (see
            // `parse_locus_member`). Lexing it as Ident frees the
            // name for use as a param/field on user types
            // (raylib bindings: `cam.mode: Int`). Same pattern as
            // `bindings`/`birth_check`/`pool`/`heap`.
            "bulk" => TokenKind::Bulk,
            "harmonic" => TokenKind::Harmonic,
            "resolution" => TokenKind::Resolution,

            // Projection class
            "projection" => TokenKind::Projection,
            "rich" => TokenKind::Rich,
            "chunked" => TokenKind::Chunked,
            "recognition" => TokenKind::Recognition,

            // Placement keywords (F.31) are all contextual idents:
            // `placement`, `cooperative`, `pinned`, `pool`, `core`
            // lex as Ident and the parser recognizes them inside
            // the placement block on `main locus`. Frees them for
            // ordinary identifier use outside that context.

            // Closure. `approx` and `within` deliberately
            // omitted — they lex as Ident and are recognized
            // contextually inside closure blocks (F.10-style).
            "closure" => TokenKind::Closure,
            "epoch" => TokenKind::Epoch,
            "persists_through" => TokenKind::PersistsThrough,
            "resets_on" => TokenKind::ResetsOn,
            "resets_per_epoch" => TokenKind::ResetsPerEpoch,

            // Recovery
            "restart" => TokenKind::Restart,
            "restart_in_place" => TokenKind::RestartInPlace,
            "quarantine" => TokenKind::Quarantine,
            "reorganize" => TokenKind::Reorganize,
            "bubble" => TokenKind::Bubble,

            // Contract
            "expose" => TokenKind::Expose,
            "consume" => TokenKind::Consume,
            "inferred" => TokenKind::Inferred,

            // Bus
            "subscribe" => TokenKind::Subscribe,
            "publish" => TokenKind::Publish,
            "on" => TokenKind::On,
            "of" => TokenKind::Of,

            // Perspective
            "stable_when" => TokenKind::StableWhen,
            "serialize_as" => TokenKind::SerializeAs,

            // Statement / expression
            "let" => TokenKind::Let,
            "mut" => TokenKind::Mut,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "match" => TokenKind::Match,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "while" => TokenKind::While,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "nil" => TokenKind::Nil,
            "tier" => TokenKind::Tier,
            "self" => TokenKind::KwSelf,

            // (Primitive type names are NOT keywords; they fall
            // through to the `Ident` case below and are recognized
            // by the parser in type position.)

            // Reserved
            "trait" => TokenKind::Trait,
            "impl" => TokenKind::Impl,
            "interface" => TokenKind::Interface,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            "yield" => TokenKind::Yield,
            "terminate" => TokenKind::Terminate,
            "release" => TokenKind::Release,
            "macro" => TokenKind::Macro,
            "where" => TokenKind::Where,
            // `with` is NOT in this list — v1.x-VIOLATE (F.27)
            // makes it a contextual keyword inside the
            // `violate_stmt` production. It lexes as Ident so
            // `let with = ...` / `fn with(...)` stay admissible.

            // `violate` is a contextual keyword recognized only
            // as the leading token of a statement inside a locus
            // method body. Lexed as Ident; same F.10-style
            // narrowing as `fail`.
            //
            // `inline` is a contextual keyword recognized only as
            // an `epoch_spec` variant inside a closure body.
            // Lexed as Ident.
            //
            // `captures` is a contextual keyword recognized only
            // as a closure-clause leader (`captures: f1, f2 ...;`)
            // inside a closure body. Lexed as Ident.

            other => TokenKind::Ident(other.to_string()),
        };
        Token::new(kind, span)
    }

    fn lex_number(&mut self, start: usize) -> Result<Token, Diag> {
        // Detect prefixed bases: 0x, 0o, 0b
        if self.peek() == Some(b'0') {
            match self.peek_at(1) {
                Some(b'x') | Some(b'X') => {
                    self.pos += 2;
                    return self.lex_int_radix(start, 16);
                }
                Some(b'o') | Some(b'O') => {
                    self.pos += 2;
                    return self.lex_int_radix(start, 8);
                }
                Some(b'b') | Some(b'B') => {
                    self.pos += 2;
                    return self.lex_int_radix(start, 2);
                }
                _ => {}
            }
        }

        // Decimal digits (with optional underscores)
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || b == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }

        // Float? .digit but not .. (range)
        let mut is_float = false;
        if self.peek() == Some(b'.') && self.peek_at(1).map_or(false, |b| b.is_ascii_digit()) {
            is_float = true;
            self.pos += 1; // consume .
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() || b == b'_' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        // Exponent
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }

        let num_end = self.pos;
        let num_text: String = self.source[start..num_end]
            .chars()
            .filter(|&c| c != '_')
            .collect();

        // Suffix detection
        // `d` → decimal literal
        if self.peek() == Some(b'd') && !is_compound_alpha(self, 1) {
            self.pos += 1;
            let span = Span::new(start, self.pos);
            return Ok(Token::new(TokenKind::DecimalLit(num_text), span));
        }

        // Duration suffix: ns / us / ms / s / m / h / d (we already checked d
        // above; here d cannot follow because `d` alone would have been the
        // decimal suffix; but `1d` as duration is still valid — we resolve by
        // letting d-decimal win when followed by non-alpha, otherwise check
        // duration unit). For simplicity, only treat it as duration when the
        // base is an integer (no float).
        if !is_float && is_duration_unit_start(self) {
            return self.lex_duration_after(start, num_text.parse().unwrap_or(0));
        }

        let span = Span::new(start, self.pos);
        if is_float {
            let v: f64 = num_text.parse().map_err(|_| {
                Diag::lex(span, format!("invalid float literal: {}", num_text))
            })?;
            Ok(Token::new(TokenKind::FloatLit(v), span))
        } else {
            let v: i64 = num_text.parse().map_err(|_| {
                Diag::lex(span, format!("invalid integer literal: {}", num_text))
            })?;
            Ok(Token::new(TokenKind::IntLit(v), span))
        }
    }

    fn lex_int_radix(&mut self, start: usize, radix: u32) -> Result<Token, Diag> {
        let digits_start = self.pos;
        while let Some(b) = self.peek() {
            if b == b'_' {
                self.pos += 1;
            } else if b.is_ascii_alphanumeric() && (b as char).is_digit(radix) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text: String = self.source[digits_start..self.pos]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let span = Span::new(start, self.pos);
        if text.is_empty() {
            return Err(Diag::lex(span, "empty digits in numeric literal"));
        }
        // Radix (hex / binary / octal) literals accept the full u64
        // range and store the bit pattern as i64, so a mask or magic with
        // the top bit set (e.g. a `ring_layout magic 0xFFFF_FFFF_FFFF_FFFF`)
        // is expressible; a consumer recovers the intended value with
        // `x as u64`. Decimal literals keep the signed i64 range (see
        // lex_number). Only a value that overflows even u64 is an error.
        let v = match i64::from_str_radix(&text, radix) {
            Ok(v) => v,
            Err(_) => match u64::from_str_radix(&text, radix) {
                Ok(u) => u as i64,
                Err(_) => {
                    return Err(Diag::lex(
                        span,
                        format!(
                            "invalid integer literal: 0{:?}{}",
                            radix_prefix(radix),
                            text
                        ),
                    ));
                }
            },
        };
        Ok(Token::new(TokenKind::IntLit(v), span))
    }

    fn lex_duration_after(&mut self, start: usize, mut total_ns: i64) -> Result<Token, Diag> {
        // We just consumed the leading integer; loop reading <unit> and
        // optional <int><unit> compound suffixes.
        loop {
            let unit_start = self.pos;
            let unit_chars = take_alpha_run(self);
            let multiplier_ns = match unit_chars.as_str() {
                "ns" => 1i64,
                "us" => 1_000,
                "ms" => 1_000_000,
                "s" => 1_000_000_000,
                "m" => 60_000_000_000,
                "h" => 3_600_000_000_000,
                "d" => 86_400_000_000_000,
                _ => {
                    let span = Span::new(start, self.pos);
                    return Err(Diag::lex(
                        span,
                        format!("unknown duration unit: {}", unit_chars),
                    ));
                }
            };
            // total_ns is the most-recently consumed integer; multiply by unit
            total_ns = total_ns
                .checked_mul(multiplier_ns)
                .ok_or_else(|| {
                    Diag::lex(
                        Span::new(unit_start, self.pos),
                        "duration overflow",
                    )
                })?
                + 0; // (running accumulator — we'll add subsequent components)

            // Compound? Look for digit-led continuation.
            let save = total_ns;
            let cont_start = self.pos;
            let mut has_more_digits = false;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    has_more_digits = true;
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if has_more_digits {
                let next_int: i64 =
                    self.source[cont_start..self.pos].parse().map_err(|_| {
                        Diag::lex(
                            Span::new(cont_start, self.pos),
                            "invalid duration component",
                        )
                    })?;
                // Recurse for the new integer.
                let unit_start2 = self.pos;
                let unit2 = take_alpha_run(self);
                let mul2 = match unit2.as_str() {
                    "ns" => 1i64,
                    "us" => 1_000,
                    "ms" => 1_000_000,
                    "s" => 1_000_000_000,
                    "m" => 60_000_000_000,
                    "h" => 3_600_000_000_000,
                    "d" => 86_400_000_000_000,
                    _ => {
                        return Err(Diag::lex(
                            Span::new(unit_start2, self.pos),
                            format!("unknown duration unit: {}", unit2),
                        ));
                    }
                };
                total_ns = save + next_int.checked_mul(mul2).unwrap_or(0);
                continue;
            }
            break;
        }
        let span = Span::new(start, self.pos);
        Ok(Token::new(TokenKind::DurationLit(total_ns), span))
    }

    fn lex_string(&mut self, start: usize) -> Result<Token, Diag> {
        // Consume opening quote.
        self.pos += 1;
        let mut s = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(Diag::lex(
                        Span::new(start, self.pos),
                        "unterminated string literal",
                    ));
                }
                Some(b'"') => {
                    self.pos += 1;
                    let span = Span::new(start, self.pos);
                    return Ok(Token::new(TokenKind::StringLit(s), span));
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'n') => {
                            s.push('\n');
                            self.pos += 1;
                        }
                        Some(b't') => {
                            s.push('\t');
                            self.pos += 1;
                        }
                        Some(b'r') => {
                            s.push('\r');
                            self.pos += 1;
                        }
                        Some(b'\\') => {
                            s.push('\\');
                            self.pos += 1;
                        }
                        Some(b'"') => {
                            s.push('"');
                            self.pos += 1;
                        }
                        Some(b'0') => {
                            s.push('\0');
                            self.pos += 1;
                        }
                        Some(b'x') => {
                            // \xNN — two hex digits, emits the
                            // byte directly. Common across C / Rust /
                            // Go / JS; agents reach for it reflexively
                            // for non-printables (NUL, separators, etc.).
                            self.pos += 1;
                            let h1 = self.peek().and_then(hex_digit);
                            let h2 = self.peek_at(1).and_then(hex_digit);
                            match (h1, h2) {
                                (Some(a), Some(b)) => {
                                    let byte = (a << 4) | b;
                                    if byte >= 0x80 {
                                        // High-byte \x would UTF-8-
                                        // encode as 2 bytes (Rust
                                        // String invariant) and
                                        // surprise the caller. For
                                        // raw bytes use
                                        // std::bytes::from_string on
                                        // an ASCII literal, or build
                                        // a Bytes value through the
                                        // stdlib byte API.
                                        return Err(Diag::lex(
                                            Span::new(self.pos - 2, self.pos + 2),
                                            "\\x escape only accepts ASCII bytes \
                                             (\\x00..\\x7f); for high bytes use the \
                                             std::bytes::* API",
                                        ));
                                    }
                                    self.pos += 2;
                                    s.push(byte as char);
                                }
                                _ => {
                                    return Err(Diag::lex(
                                        Span::new(self.pos - 1, self.pos + 1),
                                        "\\x escape needs two hex digits (e.g. \\x01, \\x7f)",
                                    ));
                                }
                            }
                        }
                        Some(other) => {
                            return Err(Diag::lex(
                                Span::new(self.pos - 1, self.pos + 1),
                                format!("unknown string escape: \\{}", other as char),
                            ));
                        }
                        None => {
                            return Err(Diag::lex(
                                Span::new(self.pos - 1, self.pos),
                                "string ended after backslash",
                            ));
                        }
                    }
                }
                Some(b) => {
                    // UTF-8: treat next byte sequence as one char.
                    let ch_start = self.pos;
                    let ch = self.source[ch_start..].chars().next().unwrap();
                    self.pos += ch.len_utf8();
                    s.push(ch);
                    let _ = b;
                }
            }
        }
    }

    /// B2 / G5: lex a `b"..."` bytes literal. Same escapes as
    /// strings, but UTF-8 promotion is off — `\xNN` accepts the
    /// full 0x00..0xFF range, and non-ASCII source bytes in the
    /// body emit one entry per UTF-8 byte. Callers that need
    /// arbitrary bytes used to wire through
    /// `std::bytes::from_string("...")`; B2 / G5 removes that
    /// workaround.
    fn lex_bytes(&mut self, start: usize) -> Result<Token, Diag> {
        // Consume opening quote.
        self.pos += 1;
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match self.peek() {
                None => {
                    return Err(Diag::lex(
                        Span::new(start, self.pos),
                        "unterminated bytes literal",
                    ));
                }
                Some(b'"') => {
                    self.pos += 1;
                    let span = Span::new(start, self.pos);
                    return Ok(Token::new(TokenKind::BytesLit(bytes), span));
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'n') => {
                            bytes.push(b'\n');
                            self.pos += 1;
                        }
                        Some(b't') => {
                            bytes.push(b'\t');
                            self.pos += 1;
                        }
                        Some(b'r') => {
                            bytes.push(b'\r');
                            self.pos += 1;
                        }
                        Some(b'\\') => {
                            bytes.push(b'\\');
                            self.pos += 1;
                        }
                        Some(b'"') => {
                            bytes.push(b'"');
                            self.pos += 1;
                        }
                        Some(b'0') => {
                            bytes.push(0);
                            self.pos += 1;
                        }
                        Some(b'x') => {
                            self.pos += 1;
                            let h1 = self.peek().and_then(hex_digit);
                            let h2 = self.peek_at(1).and_then(hex_digit);
                            match (h1, h2) {
                                (Some(a), Some(b)) => {
                                    bytes.push((a << 4) | b);
                                    self.pos += 2;
                                }
                                _ => {
                                    return Err(Diag::lex(
                                        Span::new(self.pos - 1, self.pos + 1),
                                        "\\x escape needs two hex digits (e.g. \\x01, \\xff)",
                                    ));
                                }
                            }
                        }
                        Some(other) => {
                            return Err(Diag::lex(
                                Span::new(self.pos - 1, self.pos + 1),
                                format!("unknown bytes escape: \\{}", other as char),
                            ));
                        }
                        None => {
                            return Err(Diag::lex(
                                Span::new(self.pos - 1, self.pos),
                                "bytes literal ended after backslash",
                            ));
                        }
                    }
                }
                Some(b) => {
                    bytes.push(b);
                    self.pos += 1;
                }
            }
        }
    }

    /// v1.x-10 lex an `f"..."` f-string. Same escape table as
    /// lex_string (`\n`, `\t`, `\r`, `\\`, `\"`, `\0`) plus `\{`
    /// and `\}` for literal braces; bare `{` opens interpolation,
    /// `{{` / `}}` are literal braces. The body is returned
    /// pre-split into FStringParts so the parser doesn't rescan.
    fn lex_fstring(&mut self, start: usize) -> Result<Token, Diag> {
        // Consume opening quote.
        self.pos += 1;
        let mut parts: Vec<FStringPart> = Vec::new();
        let mut buf = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(Diag::lex(
                        Span::new(start, self.pos),
                        "unterminated f-string literal",
                    ));
                }
                Some(b'"') => {
                    self.pos += 1;
                    if !buf.is_empty() || parts.is_empty() {
                        parts.push(FStringPart::Lit(std::mem::take(&mut buf)));
                    }
                    let span = Span::new(start, self.pos);
                    return Ok(Token::new(TokenKind::FStringLit(parts), span));
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'n') => { buf.push('\n'); self.pos += 1; }
                        Some(b't') => { buf.push('\t'); self.pos += 1; }
                        Some(b'r') => { buf.push('\r'); self.pos += 1; }
                        Some(b'\\') => { buf.push('\\'); self.pos += 1; }
                        Some(b'"') => { buf.push('"'); self.pos += 1; }
                        Some(b'0') => { buf.push('\0'); self.pos += 1; }
                        Some(b'{') => { buf.push('{'); self.pos += 1; }
                        Some(b'}') => { buf.push('}'); self.pos += 1; }
                        Some(b'x') => {
                            self.pos += 1;
                            let h1 = self.peek().and_then(hex_digit);
                            let h2 = self.peek_at(1).and_then(hex_digit);
                            match (h1, h2) {
                                (Some(a), Some(b)) => {
                                    let byte = (a << 4) | b;
                                    if byte >= 0x80 {
                                        return Err(Diag::lex(
                                            Span::new(self.pos - 2, self.pos + 2),
                                            "\\x escape only accepts ASCII bytes \
                                             (\\x00..\\x7f); for high bytes use the \
                                             std::bytes::* API",
                                        ));
                                    }
                                    self.pos += 2;
                                    buf.push(byte as char);
                                }
                                _ => {
                                    return Err(Diag::lex(
                                        Span::new(self.pos - 1, self.pos + 1),
                                        "\\x escape needs two hex digits (e.g. \\x01, \\x7f)",
                                    ));
                                }
                            }
                        }
                        Some(other) => {
                            return Err(Diag::lex(
                                Span::new(self.pos - 1, self.pos + 1),
                                format!("unknown f-string escape: \\{}", other as char),
                            ));
                        }
                        None => {
                            return Err(Diag::lex(
                                Span::new(self.pos - 1, self.pos),
                                "f-string ended after backslash",
                            ));
                        }
                    }
                }
                Some(b'{') => {
                    if self.source.as_bytes().get(self.pos + 1) == Some(&b'{') {
                        buf.push('{');
                        self.pos += 2;
                    } else {
                        // Enter interpolation. Flush any pending literal.
                        parts.push(FStringPart::Lit(std::mem::take(&mut buf)));
                        let interp_open_pos = self.pos;
                        self.pos += 1;
                        let mut body = String::new();
                        // Quote state: every `\"` in the f-string source
                        // toggles in_str. While in_str, `{` / `}` don't
                        // affect depth — they're just chars inside an
                        // interpolated string literal. Limitation: a
                        // literal `"` cannot appear inside an Hale string
                        // inside an f-string interpolation (would require
                        // triple-escape `\\\"`); this hits the common case
                        // (call sites with string args) and leaves the
                        // rare nested-quote case as a v1 limitation.
                        let mut in_str = false;
                        let mut depth = 1usize;
                        while depth > 0 {
                            match self.peek() {
                                None => {
                                    return Err(Diag::lex(
                                        Span::new(interp_open_pos, self.pos),
                                        "unterminated interpolation in f-string",
                                    ));
                                }
                                Some(b'\\') => {
                                    self.pos += 1;
                                    match self.peek() {
                                        Some(b'"') => {
                                            body.push('"');
                                            self.pos += 1;
                                            in_str = !in_str;
                                        }
                                        Some(b'{') if !in_str => {
                                            body.push('{');
                                            self.pos += 1;
                                        }
                                        Some(b'}') if !in_str => {
                                            body.push('}');
                                            self.pos += 1;
                                        }
                                        Some(b'\\') => {
                                            // Preserve the backslash so the
                                            // inner sub-parser sees `\\`
                                            // as an escape inside a string.
                                            body.push('\\');
                                            body.push('\\');
                                            self.pos += 1;
                                        }
                                        Some(c) if in_str => {
                                            // Inside an interpolated string,
                                            // preserve `\X` raw so the inner
                                            // sub-parser's lex_string handles
                                            // it the same way a top-level
                                            // string literal would.
                                            body.push('\\');
                                            let ch = self.source[self.pos..]
                                                .chars().next().unwrap();
                                            body.push(ch);
                                            self.pos += ch.len_utf8();
                                            let _ = c;
                                        }
                                        Some(other) => {
                                            return Err(Diag::lex(
                                                Span::new(self.pos - 1, self.pos + 1),
                                                format!(
                                                    "unknown escape in f-string \
                                                     interpolation: \\{}",
                                                    other as char
                                                ),
                                            ));
                                        }
                                        None => {
                                            return Err(Diag::lex(
                                                Span::new(self.pos - 1, self.pos),
                                                "f-string ended after backslash",
                                            ));
                                        }
                                    }
                                }
                                Some(b'}') if !in_str => {
                                    depth -= 1;
                                    if depth == 0 { break; }
                                    body.push('}');
                                    self.pos += 1;
                                }
                                Some(b'{') if !in_str => {
                                    depth += 1;
                                    body.push('{');
                                    self.pos += 1;
                                }
                                Some(_) => {
                                    let ch = self.source[self.pos..]
                                        .chars().next().unwrap();
                                    body.push(ch);
                                    self.pos += ch.len_utf8();
                                }
                            }
                        }
                        let body = body.trim().to_string();
                        if body.is_empty() {
                            return Err(Diag::lex(
                                Span::new(interp_open_pos, self.pos + 1),
                                "empty interpolation `{}` in f-string",
                            ));
                        }
                        parts.push(FStringPart::Interp(body));
                        self.pos += 1; // consume `}`
                    }
                }
                Some(b'}') => {
                    if self.source.as_bytes().get(self.pos + 1) == Some(&b'}') {
                        buf.push('}');
                        self.pos += 2;
                    } else {
                        return Err(Diag::lex(
                            Span::new(self.pos, self.pos + 1),
                            "stray `}` in f-string — use `}}` for a literal brace",
                        ));
                    }
                }
                Some(_) => {
                    let ch = self.source[self.pos..].chars().next().unwrap();
                    self.pos += ch.len_utf8();
                    buf.push(ch);
                }
            }
        }
    }

    fn lex_time(&mut self, start: usize) -> Result<Token, Diag> {
        self.pos += 1; // consume opening backtick
        let body_start = self.pos;
        loop {
            match self.peek() {
                None => {
                    return Err(Diag::lex(
                        Span::new(start, self.pos),
                        "unterminated time literal",
                    ));
                }
                Some(b'`') => {
                    let body = self.source[body_start..self.pos].to_string();
                    self.pos += 1;
                    let span = Span::new(start, self.pos);
                    return Ok(Token::new(TokenKind::TimeLit(body), span));
                }
                Some(_) => {
                    let ch = self.source[self.pos..].chars().next().unwrap();
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn lex_op_or_punct(&mut self, start: usize) -> Result<Token, Diag> {
        macro_rules! emit {
            ($kind:expr, $len:expr) => {{
                self.pos += $len;
                let span = Span::new(start, self.pos);
                return Ok(Token::new($kind, span));
            }};
        }
        // 3-char operators. Match on the raw byte slice rather
        // than `&self.source[pos..pos+3]` — the latter panics when
        // pos+3 lands mid-UTF-8-codepoint, e.g. immediately before
        // a non-ASCII char in a String literal (brained F.9 — the
        // opening `(` of `println("─x")` reaches here with the
        // next bytes being `("` + the first byte of `─`).
        // All multi-char ops are pure ASCII, so a non-ASCII byte
        // in the window guarantees no match.
        if self.bytes.len() >= self.pos + 3 {
            let b3 = &self.bytes[self.pos..self.pos + 3];
            if matches!(b3, b"..=") {
                emit!(TokenKind::DotDotEq, 3);
            }
        }
        // 2-char operators — same byte-slice approach.
        if self.bytes.len() >= self.pos + 2 {
            let b2 = &self.bytes[self.pos..self.pos + 2];
            match b2 {
                b"==" => emit!(TokenKind::EqEq, 2),
                b"!=" => emit!(TokenKind::NotEq, 2),
                b"<=" => emit!(TokenKind::LtEq, 2),
                b">=" => emit!(TokenKind::GtEq, 2),
                b"&&" => emit!(TokenKind::AndAnd, 2),
                b"||" => emit!(TokenKind::OrOr, 2),
                b"<<" => emit!(TokenKind::Shl, 2),
                b">>" => emit!(TokenKind::Shr, 2),
                b"+=" => emit!(TokenKind::PlusEq, 2),
                b"-=" => emit!(TokenKind::MinusEq, 2),
                b"*=" => emit!(TokenKind::StarEq, 2),
                b"/=" => emit!(TokenKind::SlashEq, 2),
                b"%=" => emit!(TokenKind::PercentEq, 2),
                b"&=" => emit!(TokenKind::AmpEq, 2),
                b"|=" => emit!(TokenKind::PipeEq, 2),
                b"^=" => emit!(TokenKind::CaretEq, 2),
                b"~~" => emit!(TokenKind::TildeTilde, 2),
                b"->" => emit!(TokenKind::Arrow, 2),
                b"<-" => emit!(TokenKind::LeftArrow, 2),
                b"=>" => emit!(TokenKind::FatArrow, 2),
                b"::" => emit!(TokenKind::ColonColon, 2),
                b".." => emit!(TokenKind::DotDot, 2),
                b"??" => emit!(TokenKind::QuestionQuestion, 2),
                _ => {}
            }
        }
        // 1-char operators
        let b = self.peek().unwrap();
        match b {
            b'+' => emit!(TokenKind::Plus, 1),
            b'-' => emit!(TokenKind::Minus, 1),
            b'*' => emit!(TokenKind::Star, 1),
            b'/' => emit!(TokenKind::Slash, 1),
            b'%' => emit!(TokenKind::Percent, 1),
            b'=' => emit!(TokenKind::Eq, 1),
            b'<' => emit!(TokenKind::Lt, 1),
            b'>' => emit!(TokenKind::Gt, 1),
            b'!' => emit!(TokenKind::Bang, 1),
            b'&' => emit!(TokenKind::Amp, 1),
            b'|' => emit!(TokenKind::Pipe, 1),
            b'^' => emit!(TokenKind::Caret, 1),
            b'~' => emit!(TokenKind::Tilde, 1),
            b'?' => emit!(TokenKind::Question, 1),
            b':' => emit!(TokenKind::Colon, 1),
            b';' => emit!(TokenKind::Semi, 1),
            b',' => emit!(TokenKind::Comma, 1),
            b'.' => emit!(TokenKind::Dot, 1),
            b'{' => emit!(TokenKind::LBrace, 1),
            b'}' => emit!(TokenKind::RBrace, 1),
            b'(' => emit!(TokenKind::LParen, 1),
            b')' => emit!(TokenKind::RParen, 1),
            b'[' => emit!(TokenKind::LBracket, 1),
            b']' => emit!(TokenKind::RBracket, 1),
            b'@' => emit!(TokenKind::At, 1),
            b'#' => emit!(TokenKind::Hash, 1),
            b'$' => emit!(TokenKind::Dollar, 1),
            other => {
                let span = Span::new(self.pos, self.pos + 1);
                Err(Diag::lex(
                    span,
                    format!("unexpected byte: {:?}", other as char),
                ))
            }
        }
    }
}

/// Parse a single ASCII hex digit (0-9 / a-f / A-F) into its
/// 0..=15 value. Used by the `\xNN` string escape recognizer.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn radix_prefix(r: u32) -> &'static str {
    match r {
        16 => "x",
        8 => "o",
        2 => "b",
        _ => "",
    }
}

fn is_compound_alpha(lx: &Lexer, offset: usize) -> bool {
    matches!(
        lx.bytes.get(lx.pos + offset),
        Some(b) if b.is_ascii_alphanumeric() || *b == b'_'
    )
}

fn is_duration_unit_start(lx: &Lexer) -> bool {
    let next = match lx.peek() {
        Some(b) => b,
        None => return false,
    };
    if !matches!(next, b'n' | b'u' | b'm' | b's' | b'h' | b'd') {
        return false;
    }
    // Verify it's actually one of the legal units (ns, us, ms, s, m, h, d).
    // Look ahead a few chars; if the alpha-run matches a known unit, yes.
    let mut end = lx.pos;
    while end < lx.bytes.len() && lx.bytes[end].is_ascii_alphabetic() {
        end += 1;
    }
    let unit = &lx.source[lx.pos..end];
    matches!(unit, "ns" | "us" | "ms" | "s" | "m" | "h" | "d")
}

fn take_alpha_run(lx: &mut Lexer) -> String {
    let start = lx.pos;
    while let Some(b) = lx.peek() {
        if b.is_ascii_alphabetic() {
            lx.pos += 1;
        } else {
            break;
        }
    }
    lx.source[start..lx.pos].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<TokenKind> {
        lex(s)
            .expect("lex failed")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn lex_keywords() {
        let ks = kinds("locus fn main self");
        assert_eq!(
            ks,
            vec![
                TokenKind::Locus,
                TokenKind::Fn,
                TokenKind::Ident("main".into()),
                TokenKind::KwSelf,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_string() {
        let ks = kinds(r#""hello, world""#);
        assert_eq!(
            ks,
            vec![
                TokenKind::StringLit("hello, world".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_int() {
        let ks = kinds("42 0xff 0b1010 0o17");
        assert_eq!(
            ks,
            vec![
                TokenKind::IntLit(42),
                TokenKind::IntLit(255),
                TokenKind::IntLit(10),
                TokenKind::IntLit(15),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_decimal() {
        let ks = kinds("1.5d 0.05d");
        assert_eq!(
            ks,
            vec![
                TokenKind::DecimalLit("1.5".into()),
                TokenKind::DecimalLit("0.05".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_duration() {
        let ks = kinds("100ms 5s 1s");
        assert_eq!(
            ks,
            vec![
                TokenKind::DurationLit(100_000_000),
                TokenKind::DurationLit(5_000_000_000),
                TokenKind::DurationLit(1_000_000_000),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_operators() {
        let ks = kinds("== != ~~ -> +=");
        assert_eq!(
            ks,
            vec![
                TokenKind::EqEq,
                TokenKind::NotEq,
                TokenKind::TildeTilde,
                TokenKind::Arrow,
                TokenKind::PlusEq,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_comments() {
        let ks = kinds("locus // a line comment\nfn /* block */ main");
        assert_eq!(
            ks,
            vec![
                TokenKind::Locus,
                TokenKind::Fn,
                TokenKind::Ident("main".into()),
                TokenKind::Eof,
            ]
        );
    }
}
