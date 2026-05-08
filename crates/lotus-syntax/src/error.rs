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

    pub fn render(&self, source: &str) -> String {
        let (line, col) = self.span.line_col(source);
        let kind = match self.kind {
            DiagKind::Lex => "lex error",
            DiagKind::Parse => "parse error",
            DiagKind::Type => "type error",
        };
        format!("{}:{}: {}: {}", line, col, kind, self.message)
    }
}
