//! Diagnostic types.

use crate::span::Span;

/// Which coordinate space a span's byte offsets live in.
///
/// Every renderer demultiplexes a span back to a file by testing its
/// offset against the seed's file windows ([`crate::file_owns_offset`]).
/// That only works for spans the seed's own parse produced. The
/// embedded Hale-source stdlib parses at base 0 in a space of its
/// own, so a stdlib offset is numerically indistinguishable from a
/// seed offset: past the seed's end it belongs to no window (placed
/// nowhere, which is at least not wrong), and inside one it is
/// published at a nonsense position in a file that has nothing to do
/// with it (GH #856).
///
/// The origin cannot be recovered from the number, so it travels with
/// the diagnostic. It is set at the emit site, where the owning fn —
/// and therefore the space its span belongs to — is a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpanOrigin {
    /// The bundle's own files, at the bases `parse_source_at` was
    /// given. The only spans a file window may be asked about.
    #[default]
    Seed,
    /// The embedded stdlib's parse space (`hale_stdlib::AP_SOURCE`).
    /// Never a seed location: a renderer names the stdlib instead.
    Stdlib,
}

/// A secondary location (downstream handoff, 2026-08-11): a
/// diagnostic whose story spans two places — "duplicate name"
/// pointing at the PREVIOUS declaration — carries the other span as
/// data instead of `{:?}`-formatting it into the message. The
/// renderers (which have the sources and the file-base table) turn
/// each entry into `path:line:col`; the LSP maps it to
/// `DiagnosticRelatedInformation`, which clients render as a
/// clickable second location.
#[derive(Debug, Clone, PartialEq)]
pub struct Related {
    pub span: Span,
    pub label: String,
    /// The space `span` is measured in — see [`SpanOrigin`]. A
    /// related location may come from the stdlib while the primary
    /// is in the user's source, so the origin is per-entry.
    pub origin: SpanOrigin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub kind: DiagKind,
    pub span: Span,
    pub message: String,
    /// The space [`Diag::span`] is measured in — see [`SpanOrigin`].
    pub origin: SpanOrigin,
    /// Secondary locations; see [`Related`].
    pub related: Vec<Related>,
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
            origin: SpanOrigin::Seed,
            related: Vec::new(),
        }
    }

    pub fn parse(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Parse,
            span,
            message: msg.into(),
            origin: SpanOrigin::Seed,
            related: Vec::new(),
        }
    }

    pub fn codegen(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Codegen,
            span,
            message: msg.into(),
            origin: SpanOrigin::Seed,
            related: Vec::new(),
        }
    }

    pub fn ty(span: Span, msg: impl Into<String>) -> Self {
        Diag {
            kind: DiagKind::Type,
            span,
            message: msg.into(),
            origin: SpanOrigin::Seed,
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
            origin: SpanOrigin::Seed,
            related: Vec::new(),
        }
    }

    /// Mark this diagnostic's primary span as living in the embedded
    /// stdlib's parse space (GH #856). Called at the emit site, the
    /// one place that knows which fn body a walk was standing in.
    pub fn in_stdlib(mut self) -> Self {
        self.origin = SpanOrigin::Stdlib;
        self
    }

    /// Attach a secondary location in the seed's own coordinate space
    /// (see [`Related`]).
    pub fn with_related(
        self,
        span: Span,
        label: impl Into<String>,
    ) -> Self {
        self.with_related_from(span, label, SpanOrigin::Seed)
    }

    /// Attach a secondary location that lives in the embedded
    /// stdlib's parse space (GH #856).
    pub fn with_stdlib_related(
        self,
        span: Span,
        label: impl Into<String>,
    ) -> Self {
        self.with_related_from(span, label, SpanOrigin::Stdlib)
    }

    fn with_related_from(
        mut self,
        span: Span,
        label: impl Into<String>,
        origin: SpanOrigin,
    ) -> Self {
        self.related.push(Related {
            span,
            label: label.into(),
            origin,
        });
        self
    }

    /// Offset this diagnostic's spans by `delta` bytes (see
    /// `Span::shifted` / `parse_source_at`). Related spans move
    /// with the primary — a diagnostic shifts between coordinate
    /// spaces whole. (Cross-FILE related spans should be resolved
    /// from the un-shifted diagnostic instead; see the LSP.)
    ///
    /// A STDLIB-origin span does not move: `delta` relocates between
    /// two bundle bases, and the stdlib's space is neither of them
    /// (GH #856). Shifting it would turn a wrong offset into a
    /// differently wrong one, and lose the only fact that lets a
    /// renderer refuse to place it.
    pub fn shifted(mut self, delta: u32) -> Self {
        if self.origin == SpanOrigin::Seed {
            self.span = self.span.shifted(delta);
        }
        for r in &mut self.related {
            if r.origin == SpanOrigin::Seed {
                r.span = r.span.shifted(delta);
            }
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

    /// The tail every renderer appends to a location it refuses to
    /// invent: the note that says the position is in the standard
    /// library, not in the reader's source. The CLI and the language
    /// server name the stdlib FILE as well (they can map the offset
    /// through `hale_stdlib::ap_file_at`); this crate has no stdlib
    /// to consult, so it says the one thing it knows.
    pub const STDLIB_NOTE: &'static str = "in the standard library";

    /// The whole of a stdlib-origin diagnostic's first line: the
    /// kind, the message, and the note in place of a position.
    fn stdlib_header(&self) -> String {
        format!(
            "{}: {} ({})",
            self.kind_str(),
            self.message,
            Self::STDLIB_NOTE
        )
    }

    pub fn render(&self, source: &str) -> String {
        // A stdlib-origin span has no position in THIS source. Its
        // offsets measure another file entirely, so rendering them
        // against the reader's text prints a line that is not the
        // one the diagnostic is about (GH #856).
        let mut out = if self.origin == SpanOrigin::Stdlib {
            self.stdlib_header()
        } else {
            let (line, col) = self.span.line_col(source);
            format!(
                "{}:{}: {}: {}{}",
                line,
                col,
                self.kind_str(),
                self.message,
                Self::context_snippet(self.span, source, line, col)
            )
        };
        // Related notes, resolved against the same single source.
        // Multi-file callers use the CLI's file-base-aware helper,
        // which renders these with a path instead.
        for r in &self.related {
            if r.origin == SpanOrigin::Stdlib {
                out.push_str(&format!(
                    "\n    note: {} ({})",
                    r.label,
                    Self::STDLIB_NOTE
                ));
                continue;
            }
            let (rl, rc) = r.span.line_col(source);
            out.push_str(&format!("\n    note: {} at {}:{}", r.label, rl, rc));
        }
        out
    }

    /// Render as `path:line:col: kind: message`, un-shifting the span by
    /// the file's virtual `base` (from `parse_source_at`) so the line/col
    /// are relative to the file's own source — for multi-file builds.
    ///
    /// A stdlib-origin span is not in `path` and not measured against
    /// `base`, so it renders position-free rather than at a line of a
    /// file it was never in (GH #856). The rule lives here as well as
    /// in the callers so no future caller can reintroduce it.
    pub fn render_located(&self, path: &str, source: &str, base: u32) -> String {
        if self.origin == SpanOrigin::Stdlib {
            return self.stdlib_header();
        }

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

#[cfg(test)]
mod span_origin {
    use super::*;

    const SRC: &str = "fn main() {\n    println(\"x\");\n}\n";

    /// GH #856: `shifted` relocates a diagnostic between two BUNDLE
    /// bases. A stdlib-origin span is in neither space, so it must
    /// not move — the language server shifts every diagnostic it
    /// places by the owning file's base, and a shifted stdlib offset
    /// is a wrong number that has lost the fact that it is wrong.
    #[test]
    fn a_stdlib_span_does_not_move_when_the_diagnostic_is_shifted() {
        let d =
            Diag::ty(Span::new(13_039, 13_046), "the effect happens here")
                .in_stdlib()
                .with_stdlib_related(Span::new(900, 910), "reached here");
        let moved = d.clone().shifted(64);
        assert_eq!(moved.span, d.span, "the primary stayed put");
        assert_eq!(
            moved.related[0].span, d.related[0].span,
            "and so did the related location"
        );
    }

    /// A seed-origin diagnostic still shifts, related spans and all
    /// — the behaviour every multi-file build depends on.
    #[test]
    fn a_seed_span_still_shifts_with_its_related_locations() {
        let d = Diag::ty(Span::new(10, 14), "boom")
            .with_related(Span::new(2, 4), "declared here")
            .shifted(100);
        assert_eq!(d.span, Span::new(110, 114));
        assert_eq!(d.related[0].span, Span::new(102, 104));
    }

    /// A stdlib-origin span has no position in the reader's source,
    /// so neither renderer invents one: the note says where it is
    /// instead of pointing at a line the reader never wrote.
    #[test]
    fn a_stdlib_origin_diagnostic_renders_without_a_position() {
        let d =
            Diag::ty(Span::new(13_039, 13_046), "the effect happens here")
                .in_stdlib();
        let text = d.render(SRC);
        assert!(
            text.starts_with("type error: the effect happens here ("),
            "no line:col prefix: {text}"
        );
        assert!(text.contains(Diag::STDLIB_NOTE), "{text}");
        assert_eq!(
            d.render_located("/seed/main.hl", SRC, 0),
            text,
            "the located renderer refuses the same span, whatever \
             file it is handed"
        );
    }

    /// The same for a secondary location: a stdlib-origin related
    /// entry is a note, not a `file:line:col` in the seed.
    #[test]
    fn a_stdlib_origin_related_location_renders_as_a_note() {
        let d = Diag::ty(Span::new(5, 9), "duplicate")
            .with_stdlib_related(Span::new(13_039, 13_046), "reached here");
        let text = d.render(SRC);
        assert!(text.starts_with("1:6: type error: duplicate"), "{text}");
        assert!(
            text.contains(&format!(
                "note: reached here ({})",
                Diag::STDLIB_NOTE
            )),
            "{text}"
        );
        assert_eq!(
            d.related[0].origin,
            SpanOrigin::Stdlib,
            "and the entry says so structurally"
        );
    }
}
