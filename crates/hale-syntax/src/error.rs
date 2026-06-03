//! Diagnostic types.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub kind: DiagKind,
    pub span: Span,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiagKind {
    /// Lexer errors.
    Lex,
    /// Parser errors.
    Parse,
    /// Type-checker errors.
    Type,
    /// Non-fatal advisories — the program still compiles. The first
    /// is the blocking-syscall-on-a-cooperative-pool smell: legal,
    /// but it stalls co-scheduled loci, so it's surfaced rather than
    /// rejected (cf. the hard `Type` errors for genuinely-broken
    /// shapes). Build gates fail only on `is_error()` diagnostics.
    Warn,
}

impl Diag {
    pub fn lex(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Lex,
            span,
            message: msg.into(),
        }
    }

    pub fn parse(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Parse,
            span,
            message: msg.into(),
        }
    }

    pub fn ty(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Type,
            span,
            message: msg.into(),
        }
    }

    /// A non-fatal advisory (see `DiagKind::Warn`). Surfaced to the
    /// user but does NOT fail the build.
    pub fn warn(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Warn,
            span,
            message: msg.into(),
        }
    }

    /// True for diagnostics that should fail a build. Warnings are
    /// printed but non-fatal; everything else is an error.
    pub fn is_error(&self) -> bool {
        !matches!(self.kind, DiagKind::Warn)
    }

    pub fn render(&self, source: &str) -> String {
        let (line, col) = self.span.line_col(source);
        let kind = match self.kind {
            DiagKind::Lex => "lex error",
            DiagKind::Parse => "parse error",
            DiagKind::Type => "type error",
            DiagKind::Warn => "warning",
        };
        format!("{}:{}: {}: {}", line, col, kind, self.message)
    }
}
