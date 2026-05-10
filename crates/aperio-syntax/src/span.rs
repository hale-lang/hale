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
