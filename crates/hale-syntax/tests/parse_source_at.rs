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
    assert_eq!(out, "lib/foo.hl:1:1: type error: boom");
}
