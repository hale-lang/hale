//! Format specifications for f-string interpolations (GH #469 A3).
//!
//! `f"{x:>8.2}"` — the part after the `:` says how to render the
//! value, not what to render. The grammar is a deliberate subset of
//! the one most readers already know from Rust and Python:
//!
//! ```text
//! spec := [ [fill] align ] [ width ] [ "." precision ] [ kind ]
//! align := "<" | "^" | ">"
//! kind  := "x" | "X"
//! ```
//!
//! Everything is optional, so `{x:8}` and `{x:.3}` and `{x:x}` all
//! parse. What is deliberately absent: sign control, `#` alternate
//! forms, `0` zero-padding as a distinct flag (write `:0>8`), thousands
//! separators, and `$`-style dynamic widths. Each is a real feature and
//! each can be added later without moving what is here; shipping the
//! smallest set that covers log and table output keeps the failure
//! modes enumerable.
//!
//! This module is shared by the checker and by codegen ON PURPOSE.
//! A spec that one accepts and the other rejects is a check/build
//! divergence — the worst diagnostic shape the toolchain has, and the
//! one `corpus_check_build_agreement` exists to gate. One parser means
//! there is no second opinion to disagree with.

/// Where the padding goes when a rendering is narrower than `width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    /// The wire encoding handed to the runtime padder.
    pub fn code(self) -> i32 {
        match self {
            Align::Left => 1,
            Align::Center => 3,
            Align::Right => 2,
        }
    }
}

/// How to render the value before padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecKind {
    /// Whatever `to_string` would produce.
    Display,
    /// Lower-case hexadecimal. Integers only.
    HexLower,
    /// Upper-case hexadecimal. Integers only.
    HexUpper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatSpec {
    pub fill: char,
    /// `None` means "decide from the value's type": numbers pad on
    /// the left (so columns of figures line up on the ones digit)
    /// and everything else pads on the right. Making this depend on
    /// the type rather than picking one global default is the whole
    /// reason it is an `Option` this far down the pipeline.
    pub align: Option<Align>,
    pub width: Option<u32>,
    pub precision: Option<u32>,
    pub kind: SpecKind,
}

impl Default for FormatSpec {
    fn default() -> Self {
        FormatSpec {
            fill: ' ',
            align: None,
            width: None,
            precision: None,
            kind: SpecKind::Display,
        }
    }
}

impl FormatSpec {
    /// Parse the text after the `:`. `Err` carries a message meant
    /// to be shown to the author verbatim.
    pub fn parse(s: &str) -> Result<FormatSpec, String> {
        let mut spec = FormatSpec::default();
        let cs: Vec<char> = s.chars().collect();
        let mut i = 0usize;

        // [[fill] align] — a fill character is only recognised when
        // an alignment follows it, which is what lets `{x:8}` mean
        // width 8 rather than fill '8'.
        let is_align = |c: char| matches!(c, '<' | '^' | '>');
        if cs.len() >= 2 && is_align(cs[1]) {
            spec.fill = cs[0];
            spec.align = Some(align_of(cs[1]));
            i = 2;
        } else if !cs.is_empty() && is_align(cs[0]) {
            spec.align = Some(align_of(cs[0]));
            i = 1;
        }

        // [width]
        let start = i;
        while i < cs.len() && cs[i].is_ascii_digit() {
            i += 1;
        }
        if i > start {
            let text: String = cs[start..i].iter().collect();
            spec.width = Some(text.parse::<u32>().map_err(|_| {
                format!("format width `{}` is too large", text)
            })?);
        }

        // ["." precision]
        if i < cs.len() && cs[i] == '.' {
            i += 1;
            let start = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            if i == start {
                return Err(
                    "format precision `.` must be followed by digits \
                     (e.g. `.2`)"
                        .to_string(),
                );
            }
            let text: String = cs[start..i].iter().collect();
            let p = text.parse::<u32>().map_err(|_| {
                format!("format precision `{}` is too large", text)
            })?;
            if p > 17 {
                // Past 17 significant digits an f64 is inventing
                // decimals; saying so beats printing noise.
                return Err(format!(
                    "format precision {} exceeds the 17 digits an \
                     f64 can represent",
                    p
                ));
            }
            spec.precision = Some(p);
        }

        // [kind]
        if i < cs.len() {
            spec.kind = match cs[i] {
                'x' => SpecKind::HexLower,
                'X' => SpecKind::HexUpper,
                c => {
                    return Err(format!(
                        "unknown format kind `{}` — supported: `x` and \
                         `X` (hexadecimal). A spec is \
                         `[[fill]align][width][.precision][kind]`, e.g. \
                         `{{n:>8}}`, `{{ratio:.2}}`, `{{addr:x}}`",
                        c
                    ));
                }
            };
            i += 1;
        }

        if i != cs.len() {
            let rest: String = cs[i..].iter().collect();
            return Err(format!(
                "trailing `{}` in format spec — a spec is \
                 `[[fill]align][width][.precision][kind]`",
                rest
            ));
        }

        if spec.precision.is_some() && spec.kind != SpecKind::Display {
            return Err(
                "a precision and a hexadecimal kind cannot be combined \
                 — hex has no fractional part"
                    .to_string(),
            );
        }
        Ok(spec)
    }

    /// True when nothing about the rendering changes, so the caller
    /// can skip the format call entirely.
    pub fn is_identity(&self) -> bool {
        self.width.is_none()
            && self.precision.is_none()
            && self.kind == SpecKind::Display
    }
}

fn align_of(c: char) -> Align {
    match c {
        '<' => Align::Left,
        '^' => Align::Center,
        _ => Align::Right,
    }
}

/// Split an interpolation body into `(expression, spec)`.
///
/// The split is on the LAST top-level `:` that is not part of a `::`
/// path separator, so `f"{std::time::now():>12}"` splits once, in
/// the right place, and `f"{std::time::now()}"` does not split at
/// all. "Top level" excludes anything inside brackets or a string
/// literal, so a `:` in `f"{m[\"a:b\"]}"` is left alone.
///
/// Returns `None` for the spec when the body carries no format spec.
pub fn split_spec(body: &str) -> (&str, Option<&str>) {
    let b = body.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut cut: Option<usize> = None;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => {
                i += 2;
                continue;
            }
            b'"' => in_str = !in_str,
            b'(' | b'[' | b'{' if !in_str => depth += 1,
            b')' | b']' | b'}' if !in_str => depth -= 1,
            b':' if !in_str && depth == 0 => {
                // `::` is one token, never a spec separator; skip
                // both bytes so the second colon cannot be mistaken
                // for the start of a spec.
                if b.get(i + 1) == Some(&b':') {
                    i += 2;
                    continue;
                }
                cut = Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    match cut {
        Some(c) => (body[..c].trim_end(), Some(body[c + 1..].trim())),
        None => (body, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_separator_is_not_a_spec_separator() {
        assert_eq!(split_spec("std::time::now()"), ("std::time::now()", None));
        assert_eq!(
            split_spec("std::time::now():>12"),
            ("std::time::now()", Some(">12"))
        );
    }

    #[test]
    fn a_colon_inside_brackets_or_a_string_is_left_alone() {
        assert_eq!(split_spec("m[\"a:b\"]"), ("m[\"a:b\"]", None));
        assert_eq!(split_spec("f(a, b)"), ("f(a, b)", None));
    }

    #[test]
    fn the_grammar_parses_the_documented_shapes() {
        assert_eq!(FormatSpec::parse("").unwrap(), FormatSpec::default());
        assert_eq!(FormatSpec::parse("8").unwrap().width, Some(8));
        assert_eq!(FormatSpec::parse(".2").unwrap().precision, Some(2));
        assert_eq!(FormatSpec::parse("x").unwrap().kind, SpecKind::HexLower);
        let s = FormatSpec::parse("0>8").unwrap();
        assert_eq!((s.fill, s.align, s.width), ('0', Some(Align::Right), Some(8)));
        let s = FormatSpec::parse("^10.3").unwrap();
        assert_eq!(
            (s.align, s.width, s.precision),
            (Some(Align::Center), Some(10), Some(3))
        );
    }

    #[test]
    fn a_bare_width_is_a_width_not_a_fill() {
        // The regression this ordering exists to prevent: reading
        // `8` as the fill character and then finding no width.
        let s = FormatSpec::parse("8").unwrap();
        assert_eq!(s.fill, ' ');
        assert_eq!(s.width, Some(8));
    }

    #[test]
    fn nonsense_is_rejected_with_something_actionable() {
        for bad in ["zz", ".", "8.", "2.2x", "8q"] {
            let e = FormatSpec::parse(bad).unwrap_err();
            assert!(!e.is_empty(), "{bad} must explain itself");
        }
    }
}
