//! Multi-file diagnostic locations: `parse_source_at` shifts a file's
//! spans into a process-wide coordinate space so a merged build can
//! demultiplex a diagnostic back to its originating file, and
//! `Diag::render_located` un-shifts it to the file's own line/col.
//! Regression for the "317:1, no filename" mislocation that mis-reported
//! errors from imported files against the entry file.

use hale_syntax::{parse_source, parse_source_at, Diag};

#[test]
fn parse_source_at_shifts_spans_by_base() {
    let src = "fn main() {\n    foo();\n}\n";
    let base = 5000u32;
    let p0 = parse_source(src).expect("parse");
    let pb = parse_source_at(src, base).expect("parse at base");
    assert_eq!(p0.items.len(), pb.items.len());
    let s0 = p0.items[0].span();
    let sb = pb.items[0].span();
    // Identical structure, every span offset by exactly `base`.
    assert_eq!(sb.start.as_usize(), s0.start.as_usize() + base as usize);
    assert_eq!(sb.end.as_usize(), s0.end.as_usize() + base as usize);
}

#[test]
fn render_located_unshifts_to_file_line_col() {
    let src = "fn main() {\n    foo();\n}\n";
    let base = 5000u32;
    let pb = parse_source_at(src, base).expect("parse at base");
    let span = pb.items[0].span(); // shifted span, as it'd appear post-merge
    let d = Diag::ty(span, "boom");
    // Demux: rendering against the file's own source + base recovers the
    // real location (line 1 — `fn main` is the first line).
    let out = d.render_located("lib/foo.hl", src, base);
    // GH #241: rendered diagnostics carry a source-context
    // snippet (offending line + caret underline).
    assert!(
        out.starts_with("lib/foo.hl:1:1: type error: boom\n"),
        "got: {:?}",
        out
    );
    assert!(
        out.contains("\n    fn main() {\n    ^"),
        "expected context snippet with caret; got: {:?}",
        out
    );
}

/// GH #765: a PARSE ERROR's span is shifted by `base` exactly once.
///
/// The error arm used to shift twice — the parser is handed tokens
/// whose spans already carry `base` and cites them, and the result was
/// then `.shifted(base)` again. A parse diagnostic therefore landed at
/// `2 * base + local`, outside its own file's `base..base + len`
/// window, and every consumer that demultiplexes by that window lost
/// it: an imported seed's parse error rendered with no filename at all,
/// positioned against whichever source happened to be first. A
/// consumer that un-shifts once — `parse_files`, `hale lsp` — landed
/// `base` bytes late, which is why a parse error in the second or later
/// file of a seed pointed at the wrong line.
#[test]
fn a_parse_error_span_is_shifted_by_base_exactly_once() {
    // Missing `;` before the closing brace.
    let src = "fn double(x: Int) -> Int { return x * 2 }\n";
    let base = 5000u32;
    let d0 = parse_source(src).expect_err("must not parse");
    let db = parse_source_at(src, base).expect_err("must not parse at base");
    assert_eq!(d0.len(), db.len(), "same diagnostics either way");
    for (a, b) in d0.iter().zip(db.iter()) {
        assert_eq!(a.message, b.message);
        assert_eq!(
            b.span.start.as_usize(),
            a.span.start.as_usize() + base as usize,
            "parse-error span must carry `base` once, not twice"
        );
        // Inside this file's window, so a merged build can demultiplex
        // it back to the file it came from.
        let off = b.span.start.as_usize();
        assert!(
            off >= base as usize && off < base as usize + src.len(),
            "offset {} must fall in {}..{}",
            off,
            base,
            base as usize + src.len()
        );
    }
    // And the location a renderer recovers is the file's own.
    let rendered = db[0].render_located("lib/main.hl", src, base);
    assert!(
        rendered.starts_with("lib/main.hl:1:41: parse error:"),
        "got: {:?}",
        rendered
    );
}

/// The LEXER's diagnostics still need the shift — it reads the
/// unshifted source, so its spans are file-local until this function
/// moves them. Same window invariant.
#[test]
fn a_lex_error_span_is_shifted_by_base_exactly_once() {
    // An unterminated string literal is a LEX error, not a parse error.
    let src = "fn main() { println(\"oops); }\n";
    let base = 5000u32;
    let d0 = parse_source(src).expect_err("must not lex");
    let db = parse_source_at(src, base).expect_err("must not lex at base");
    assert_eq!(d0.len(), db.len());
    for (a, b) in d0.iter().zip(db.iter()) {
        assert_eq!(
            b.span.start.as_usize(),
            a.span.start.as_usize() + base as usize,
            "lex-error span must carry `base` once"
        );
        let off = b.span.start.as_usize();
        assert!(
            off >= base as usize && off < base as usize + src.len(),
            "offset {} must fall in {}..{}",
            off,
            base,
            base as usize + src.len()
        );
    }
}
