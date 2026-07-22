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
    /// GH #241: codegen-raised errors that carry a source span
    /// (CodegenError::UnsupportedAt) — rendered with the same
    /// location + caret treatment as check diagnostics.
    Codegen,
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

    pub fn codegen(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Codegen,
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

    /// Offset this diagnostic's span by `delta` bytes (see
    /// `Span::shifted` / `parse_source_at`).
    pub fn shifted(mut self, delta: u32) -> Self {
        self.span = self.span.shifted(delta);
        self
    }

    /// True for diagnostics that should fail a build. Warnings are
    /// printed but non-fatal; everything else is an error.
    pub fn is_error(&self) -> bool {
        !matches!(self.kind, DiagKind::Warn)
    }

    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            DiagKind::Lex => "lex error",
            DiagKind::Parse => "parse error",
            DiagKind::Type => "type error",
            DiagKind::Codegen => "codegen error",
            DiagKind::Warn => "warning",
        }
    }

    pub fn render(&self, source: &str) -> String {
        let (line, col) = self.span.line_col(source);
        format!(
            "{}:{}: {}: {}{}",
            line,
            col,
            self.kind_str(),
            self.message,
            Self::context_snippet(self.span, source, line, col)
        )
    }

    /// Render as `path:line:col: kind: message`, un-shifting the span by
    /// the file's virtual `base` (from `parse_source_at`) so the line/col
    /// are relative to the file's own source — for multi-file builds.
    pub fn render_located(&self, path: &str, source: &str, base: u32) -> String {
        let span = self.span.shifted(base.wrapping_neg());
        let (line, col) = span.line_col(source);
        format!(
            "{}:{}:{}: {}: {}{}",
            path,
            line,
            col,
            self.kind_str(),
            self.message,
            Self::context_snippet(span, source, line, col)
        )
    }

    /// GH #241: two extra lines under every rendered diagnostic —
    /// the offending source line and a caret underline at the
    /// span. Tab alignment: the padding reuses the line's own
    /// prefix characters (tabs stay tabs) so the caret lands
    /// where the terminal renders the column. Empty when the
    /// span's line can't be recovered (synthetic spans).
    fn context_snippet(
        span: Span,
        source: &str,
        line: usize,
        col: usize,
    ) -> String {
        let Some(src_line) = source.lines().nth(line.saturating_sub(1))
        else {
            return String::new();
        };
        if src_line.trim().is_empty() {
            return String::new();
        }
        let caret_at = col.saturating_sub(1);
        let span_len = span.end.as_usize().saturating_sub(span.start.as_usize());
        let rest = src_line.chars().count().saturating_sub(caret_at);
        let width = span_len.clamp(1, rest.max(1));
        let pad: String = src_line
            .chars()
            .take(caret_at)
            .map(|c| if c == '\t' { '\t' } else { ' ' })
            .collect();
        format!(
            "\n    {}\n    {}{}",
            src_line,
            pad,
            "^".repeat(width)
        )
    }
}
