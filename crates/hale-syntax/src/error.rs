//! Diagnostic types.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub kind: DiagKind,
    pub span: Span,
    pub message: String,
    /// Secondary locations (downstream handoff, 2026-08-11): a
    /// diagnostic whose story spans two places — "duplicate name"
    /// pointing at the PREVIOUS declaration — carries the other
    /// span as data instead of `{:?}`-formatting it into the
    /// message. The renderers (which have the sources and the
    /// file-base table) turn each entry into `path:line:col`; the
    /// LSP maps them to `DiagnosticRelatedInformation`, which
    /// clients render as a clickable second location.
    pub related: Vec<(Span, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiagKind {
    /// Lexer errors.
    Lex,
    /// Parser errors.
    Parse,
    /// Type-checker errors.
    Type,
    /// Claim violations and claim-vocabulary errors. Errors like any
    /// other, and rendered identically to `Type` on purpose — the
    /// message already begins "claim `x` violated", so a distinct
    /// prefix would only stutter.
    ///
    /// The kind exists so a consumer can tell "this program does not
    /// typecheck" from "this program typechecks and breaks a law".
    /// The topology artifact needs exactly that distinction: a
    /// violated claim is a truthful report about a sound model, while
    /// a type error means the model was derived from a program the
    /// compiler could not understand and must not be published at all.
    Claim,
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
            related: Vec::new(),
        }
    }

    pub fn parse(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Parse,
            span,
            message: msg.into(),
            related: Vec::new(),
        }
    }

    pub fn codegen(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Codegen,
            span,
            message: msg.into(),
            related: Vec::new(),
        }
    }

    pub fn ty(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Type,
            span,
            message: msg.into(),
            related: Vec::new(),
        }
    }

    /// A non-fatal advisory (see `DiagKind::Warn`). Surfaced to the
    /// user but does NOT fail the build.
    pub fn warn(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Warn,
            span,
            message: msg.into(),
            related: Vec::new(),
        }
    }

    /// Attach a secondary location (see the `related` field doc).
    pub fn with_related(
        mut self,
        span: Span,
        label: impl Into<String>,
    ) -> Self {
        self.related.push((span, label.into()));
        self
    }

    /// Offset this diagnostic's spans by `delta` bytes (see
    /// `Span::shifted` / `parse_source_at`). Related spans move
    /// with the primary — a diagnostic shifts between coordinate
    /// spaces whole. (Cross-FILE related spans should be resolved
    /// from the un-shifted diagnostic instead; see the LSP.)
    pub fn shifted(mut self, delta: u32) -> Self {
        self.span = self.span.shifted(delta);
        for (rspan, _) in &mut self.related {
            *rspan = rspan.shifted(delta);
        }
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
            // deliberately the same label — see the variant's doc
            DiagKind::Claim => "type error",
            DiagKind::Codegen => "codegen error",
            DiagKind::Warn => "warning",
        }
    }

    pub fn render(&self, source: &str) -> String {
        let (line, col) = self.span.line_col(source);
        let mut out = format!(
            "{}:{}: {}: {}{}",
            line,
            col,
            self.kind_str(),
            self.message,
            Self::context_snippet(self.span, source, line, col)
        );
        // Related notes, resolved against the same single source.
        // Multi-file callers use the CLI's file-base-aware helper,
        // which renders these with a path instead.
        for (rspan, label) in &self.related {
            let (rl, rc) = rspan.line_col(source);
            out.push_str(&format!("\n    note: {} at {}:{}", label, rl, rc));
        }
        out
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
