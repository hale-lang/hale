//! Source positions and spans.

/// A byte offset into a source file (0-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos(pub u32);

impl Pos {
    pub fn new(byte: usize) -> Self {
        Pos(byte as u32)
    }
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
    /// Offset this position by `delta` bytes. Used to relocate a single
    /// file's spans into a process-wide coordinate space so multi-file
    /// builds can demultiplex a span back to its originating file (see
    /// `parse_source_at`).
    pub fn shifted(self, delta: u32) -> Self {
        Pos(self.0.wrapping_add(delta))
    }
}

/// A half-open byte range [start, end) into the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span {
            start: Pos::new(start),
            end: Pos::new(end),
        }
    }

    /// Offset both ends by `delta` bytes (see `Pos::shifted`).
    pub fn shifted(self, delta: u32) -> Span {
        Span {
            start: self.start.shifted(delta),
            end: self.end.shifted(delta),
        }
    }

    /// Construct a span enclosing both inputs.
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start.as_usize()..self.end.as_usize()]
    }

    /// Compute (line, column) of the start byte. 1-indexed; expensive
    /// (linear scan); for diagnostic rendering only.
    pub fn line_col(&self, source: &str) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        for (i, ch) in source.char_indices() {
            if i >= self.start.as_usize() {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }
}

/// Does the file parsed at `base`, `len` bytes long, own merged
/// offset `off`? The one window test every multi-file span
/// demultiplexer uses — `hale check`'s text and JSON renderers,
/// and the language server's diagnostic, related-location and
/// definition mappings.
///
/// It is INCLUSIVE of `base + len`, the one-past-the-last-byte
/// position an end-of-file span carries (`Span::new(pos, pos)` at
/// the end of the source, which is what the `Eof` token holds).
/// A diagnostic that cites EOF — `expected }, got Eof`, the
/// missing closing brace, the commonest syntactic mistake there is
/// — sits exactly there, and a half-open window put it in NO file
/// at all: `hale check` rendered it with no filename and, under
/// `--json`, as `"file":"","line":0,"col":0` (GH #777); the
/// language server dropped it, so the editor showed nothing for a
/// program the CLI rejects (GH #805).
///
/// Files are parsed at bases spaced `len + 1` apart
/// (`parse_source_at` callers: the CLI's `parse_files` /
/// `resolve_imports`, the server's seed analysis), so that byte
/// belongs to no other file and the windows stay disjoint: file
/// `i` owns `[base, base + len]` and file `i + 1` starts at
/// `base + len + 1`.
pub fn file_owns_offset(base: u32, len: u32, off: u32) -> bool {
    off >= base && off <= base.saturating_add(len)
}

#[cfg(test)]
mod tests {
    use super::file_owns_offset;

    /// The one-past-the-last-byte position belongs to the file it
    /// is one past the end OF — the whole point of the rule.
    #[test]
    fn a_file_owns_its_end_of_file_position() {
        assert!(file_owns_offset(0, 10, 10));
        assert!(file_owns_offset(11, 20, 31));
    }

    /// …and not one byte more: the next file's base is `base + len
    /// + 1`, so the inclusive end must stop short of it.
    #[test]
    fn a_file_owns_nothing_past_its_end() {
        assert!(!file_owns_offset(0, 10, 11));
        assert!(!file_owns_offset(11, 20, 32));
        assert!(!file_owns_offset(11, 20, 10));
    }

    /// Over a table of files laid out the way the parsers lay them
    /// out, every offset in the merged space belongs to EXACTLY
    /// one file. Half-open windows left one hole per file; a
    /// window inclusive at both ends would overlap instead.
    #[test]
    fn every_merged_offset_belongs_to_exactly_one_file() {
        let lens = [7u32, 0, 13, 1];
        let mut bases = Vec::new();
        let mut base = 0u32;
        for len in lens {
            bases.push((base, len));
            base += len + 1;
        }
        let last = bases.last().map(|(b, l)| b + l).expect("nonempty");
        for off in 0..=last {
            let owners = bases
                .iter()
                .filter(|(b, l)| file_owns_offset(*b, *l, off))
                .count();
            assert_eq!(
                owners, 1,
                "offset {off} is owned by {owners} files, not 1: {bases:?}"
            );
        }
    }

    /// A zero-length file is a real case (an editor's freshly
    /// created `.hl`), and it owns its own single position rather
    /// than none.
    #[test]
    fn an_empty_file_owns_its_only_position() {
        assert!(file_owns_offset(0, 0, 0));
        assert!(!file_owns_offset(0, 0, 1));
    }
}
