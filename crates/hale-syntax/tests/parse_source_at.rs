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

/// GH #725: a parse diagnostic's span is shifted ONCE. It is built
/// from a token whose span this function already shifted, so the
/// blanket `.shifted(base)` that used to wrap the `Err` arm moved it a
/// second time — every parse error in a file with a non-zero base
/// rendered `base` bytes late, usually past the end of the file, which
/// costs the caret too (the renderer cannot find the line).
#[test]
fn a_parse_diagnostic_is_shifted_exactly_once() {
    let src = "fn main() {\n    let where = 1;\n}\n";
    let base = 5000u32;
    let d0 = parse_source(src).unwrap_err();
    let db = parse_source_at(src, base).unwrap_err();
    assert_eq!(d0.len(), 1, "one diagnostic: {d0:?}");
    assert_eq!(db.len(), d0.len());
    assert_eq!(
        db[0].span.start.as_usize(),
        d0[0].span.start.as_usize() + base as usize,
        "shifted by base, not by twice base"
    );
    // Which is what makes the round trip land on the real line.
    let out = db[0].render_located("lib/foo.hl", src, base);
    assert!(out.starts_with("lib/foo.hl:2:9:"), "got: {out}");
    assert!(out.contains("^^^^^"), "the caret survives: {out}");
}

/// GH #725: an f-string's interpolation offsets are byte offsets into
/// the file, and the parser sub-parses each interpolation and shifts
/// the sub-parse's spans by them — so they have to move with their
/// token. They did not, which left every span inside `f"{x}"` in a
/// non-first file `base` bytes early, i.e. pointing into whichever
/// file happened to occupy that range.
#[test]
fn no_span_lands_before_the_file_that_owns_it() {
    let src = "\
fn main() {
    let total = 1;
    println(f\"total={total}\");
}
";
    let base = 5000u32;
    let pb = parse_source_at(src, base).expect("parse at base");
    // Every Pos in the tree — including the ones the f-string's
    // sub-parse produced — must be inside this file's window.
    let dump = format!("{:?}", pb);
    let mut seen = 0usize;
    let mut rest = dump.as_str();
    while let Some(i) = rest.find("Pos(") {
        rest = &rest[i + 4..];
        let end = rest.find(')').expect("closing paren");
        let n: usize = rest[..end].parse().expect("a byte offset");
        assert!(
            n >= base as usize,
            "a span at {} is before this file's base {} — it points \
             into an earlier file",
            n,
            base
        );
        seen += 1;
    }
    assert!(seen > 10, "the dump should carry many spans, saw {}", seen);
}
