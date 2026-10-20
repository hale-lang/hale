//! A reader for the subset of YAML an OpenAPI document is written in, giving
//! the `serde_json::Value` the JSON reader gives for the same document
//! (spec/api.md § Imported surfaces).
//!
//! Read: block mappings and sequences by indentation (`- key: v` items
//! included), plain, single-quoted and double-quoted scalars, `|` and `>`
//! block scalars with their indentation and chomping indicators, flow
//! `[a, b]` and `{k: v}` collections, comments, one document (with its
//! `---` and `...` markers). Scalars resolve by the YAML 1.2 core schema:
//! `~` and `null` are null, `true` and `false` are booleans, `on` and `yes`
//! are strings, a mapping key is always the text it is written as.
//!
//! Refused, naming the line: anchors, aliases and merge keys (`&`, `*`,
//! `<<`), tags (`!`), complex keys (`?`), directives, more than one
//! document, tab indentation, duplicate keys.

use serde_json::{Map, Number, Value};

type Res<T> = Result<T, String>;

struct Line {
    no: usize,
    indent: usize,
    text: String,
    raw: String,
}

struct P {
    lines: Vec<Line>,
    pos: usize,
}

fn err<T>(no: usize, msg: &str) -> Res<T> {
    Err(format!("line {no}: {msg}"))
}

/// The document `text` as JSON.
pub fn parse(text: &str) -> Res<Value> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = text
        .lines()
        .enumerate()
        .map(|(i, raw)| {
            let raw = raw.trim_end_matches('\r');
            let indent = raw.len() - raw.trim_start_matches(' ').len();
            Line {
                no: i + 1,
                indent,
                text: raw[indent..].to_string(),
                raw: raw.to_string(),
            }
        })
        .collect();
    let mut p = P { lines, pos: 0 };
    p.document()
}

fn is_marker(l: &Line, m: &str) -> bool {
    l.indent == 0 && (l.text == m || l.text.starts_with(&format!("{m} ")))
}

fn is_seq_start(t: &str) -> bool {
    t == "-" || t.starts_with("- ")
}

/// `text` without its trailing comment (a `#` at the start or after a
/// space, outside quotes).
fn strip_comment(text: &str) -> &str {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let token_start = i == 0 || matches!(b[i - 1], b' ' | b'[' | b'{' | b',' | b':');
        match b[i] {
            b'#' if i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t' => return text[..i].trim_end(),
            b'"' if token_start => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'\'' if token_start => {
                i += 1;
                while i < b.len() {
                    if b[i] == b'\'' {
                        if b.get(i + 1) == Some(&b'\'') {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    text.trim_end()
}

/// A `key: rest` line as (the key as written, the rest), when it is one.
fn key_split(text: &str) -> Option<(&str, &str)> {
    let b = text.as_bytes();
    match b.first()? {
        b'[' | b'{' | b'|' | b'>' | b'#' => return None,
        q @ (b'"' | b'\'') => {
            let mut i = 1;
            loop {
                if i >= b.len() {
                    return None;
                }
                if *q == b'"' && b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == *q {
                    if *q == b'\'' && b.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            let end = i + 1;
            let mut j = end;
            while b.get(j) == Some(&b' ') {
                j += 1;
            }
            if b.get(j) == Some(&b':') && (j + 1 == b.len() || b[j + 1] == b' ') {
                return Some((&text[..end], text[j + 1..].trim_start()));
            }
            return None;
        }
        _ => {}
    }
    for i in 0..b.len() {
        if b[i] == b':' && (i + 1 == b.len() || b[i + 1] == b' ') {
            return Some((text[..i].trim_end(), text[i + 1..].trim_start()));
        }
        if b[i] == b'#' && i > 0 && b[i - 1] == b' ' {
            return None;
        }
    }
    None
}

/// The text of a quoted scalar opening at `chars[0]`, and the index after
/// its closing quote; `None` when the text ends first.
fn scan_quoted(chars: &[char], no: usize) -> Res<Option<(String, usize)>> {
    let q = chars[0];
    let mut out = String::new();
    let mut i = 1;
    while i < chars.len() {
        let c = chars[i];
        if c == q {
            if q == '\'' && chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            return Ok(Some((out, i + 1)));
        }
        if c == '\n' {
            // a line break folds: one space, or one newline per blank line
            while out.ends_with(' ') || out.ends_with('\t') {
                out.pop();
            }
            let mut n = 1;
            i += 1;
            loop {
                while matches!(chars.get(i), Some(' ' | '\t')) {
                    i += 1;
                }
                if chars.get(i) == Some(&'\n') {
                    n += 1;
                    i += 1;
                } else {
                    break;
                }
            }
            if n == 1 {
                out.push(' ');
            } else {
                out.extend(std::iter::repeat('\n').take(n - 1));
            }
            continue;
        }
        if q == '"' && c == '\\' {
            i += 1;
            let Some(&e) = chars.get(i) else {
                return Ok(None);
            };
            let hex = |n: usize, i: &mut usize| -> Res<char> {
                let s: String = chars
                    .get(*i + 1..*i + 1 + n)
                    .unwrap_or(&[])
                    .iter()
                    .collect();
                let v = u32::from_str_radix(&s, 16).ok().filter(|_| s.len() == n);
                *i += n;
                match v.and_then(char::from_u32) {
                    Some(c) => Ok(c),
                    None => err(no, "a bad \\x, \\u or \\U escape in a double-quoted string"),
                }
            };
            match e {
                '0' => out.push('\0'),
                'a' => out.push('\x07'),
                'b' => out.push('\x08'),
                't' | '\t' => out.push('\t'),
                'n' => out.push('\n'),
                'v' => out.push('\x0b'),
                'f' => out.push('\x0c'),
                'r' => out.push('\r'),
                'e' => out.push('\x1b'),
                ' ' => out.push(' '),
                '"' => out.push('"'),
                '/' => out.push('/'),
                '\\' => out.push('\\'),
                'N' => out.push('\u{85}'),
                '_' => out.push('\u{a0}'),
                'L' => out.push('\u{2028}'),
                'P' => out.push('\u{2029}'),
                'x' => out.push(hex(2, &mut i)?),
                'u' => out.push(hex(4, &mut i)?),
                'U' => out.push(hex(8, &mut i)?),
                '\n' => {
                    // an escaped line break joins the lines
                    i += 1;
                    while matches!(chars.get(i), Some(' ' | '\t')) {
                        i += 1;
                    }
                    continue;
                }
                _ => {
                    return err(
                        no,
                        &format!("`\\{e}` is no escape in a double-quoted string"),
                    )
                }
            }
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    Ok(None)
}

fn is_float(s: &str) -> bool {
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mant, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (int, frac) = match mant.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (mant, None),
    };
    let digits = |t: &str| t.bytes().all(|c| c.is_ascii_digit());
    if !digits(int)
        || !frac.map_or(true, digits)
        || (int.is_empty() && frac.map_or(true, str::is_empty))
    {
        return false;
    }
    match exp {
        None => true,
        Some(e) => {
            let e = e.strip_prefix(['-', '+']).unwrap_or(e);
            !e.is_empty() && digits(e)
        }
    }
}

/// A plain scalar read by the core schema.
fn resolve(s: &str) -> Value {
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
    if !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit()) {
        if let Ok(n) = s.parse::<i64>() {
            return Value::from(n);
        }
        if let Ok(n) = s.parse::<u64>() {
            return Value::from(n);
        }
    } else if let Some(h) = s.strip_prefix("0x").filter(|h| !h.is_empty()) {
        if let Ok(n) = i64::from_str_radix(h, 16) {
            return Value::from(n);
        }
    } else if let Some(o) = s.strip_prefix("0o").filter(|o| !o.is_empty()) {
        if let Ok(n) = i64::from_str_radix(o, 8) {
            return Value::from(n);
        }
    }
    if is_float(s) {
        if let Some(n) = s.parse::<f64>().ok().and_then(Number::from_f64) {
            return Value::Number(n);
        }
    }
    Value::String(s.to_string())
}

fn refuse_indicator(c: char, no: usize) -> Res<()> {
    match c {
        '&' => err(
            no,
            "anchors (`&`) are not read; write the value out where it is used",
        ),
        '*' => err(
            no,
            "aliases (`*`) are not read; write the value out where it is used",
        ),
        '!' => err(no, "tags (`!`) are not read; write the value plainly"),
        '@' | '`' => err(no, &format!("a scalar cannot start with `{c}`; quote it")),
        _ => Ok(()),
    }
}

impl P {
    fn skip_blank(&mut self) -> Res<()> {
        while let Some(l) = self.lines.get(self.pos) {
            if l.text.trim().is_empty() || l.text.starts_with('#') {
                self.pos += 1;
            } else {
                if l.text.starts_with('\t') {
                    return err(l.no, "a tab cannot indent a line; use spaces");
                }
                break;
            }
        }
        Ok(())
    }

    /// The current line's indent, text and number, after blanks.
    fn cur(&mut self) -> Res<Option<(usize, String, usize)>> {
        self.skip_blank()?;
        Ok(self
            .lines
            .get(self.pos)
            .map(|l| (l.indent, l.text.clone(), l.no)))
    }

    fn document(&mut self) -> Res<Value> {
        let Some((_, text, no)) = self.cur()? else {
            return Ok(Value::Null);
        };
        if text.starts_with('%') && self.lines[self.pos].indent == 0 {
            return err(no, "directives (`%`) are not read");
        }
        if is_marker(&self.lines[self.pos], "---") {
            let rest = strip_comment(text[3..].trim_start()).to_string();
            if rest.is_empty() {
                self.pos += 1;
            } else {
                let l = &mut self.lines[self.pos];
                l.indent = 4;
                l.text = rest;
            }
        }
        let root = match self.cur()? {
            None => Value::Null,
            Some((_, _, _)) if is_marker(&self.lines[self.pos], "...") => Value::Null,
            Some((_, _, _)) if is_marker(&self.lines[self.pos], "---") => {
                return err(
                    self.lines[self.pos].no,
                    "more than one document; the import reads one",
                )
            }
            Some((ind, _, _)) => self.block(ind, -1)?,
        };
        if let Some((_, _, no)) = self.cur()? {
            let l = &self.lines[self.pos];
            if is_marker(l, "...") {
                self.pos += 1;
                if let Some((_, _, no)) = self.cur()? {
                    return err(no, "content after `...`; the import reads one document");
                }
            } else if is_marker(l, "---") {
                return err(no, "more than one document; the import reads one");
            } else {
                return err(
                    no,
                    "unexpected content (is the line indented to match its siblings?)",
                );
            }
        }
        Ok(root)
    }

    /// The node starting at the current line, whose lines sit at `ind`;
    /// `parent` is the indent a scalar's continuation must exceed.
    fn block(&mut self, ind: usize, parent: i64) -> Res<Value> {
        let (_, text, no) = self.cur()?.expect("a line");
        if is_seq_start(&text) {
            self.seq(ind)
        } else if key_split(&text).is_some() || text == "?" || text.starts_with("? ") {
            self.map(ind)
        } else {
            self.value(&text, parent, no)
        }
    }

    fn seq(&mut self, ind: usize) -> Res<Value> {
        let mut out = Vec::new();
        loop {
            let Some((i, text, no)) = self.cur()? else {
                break;
            };
            if i < ind
                || (i == 0
                    && (is_marker(&self.lines[self.pos], "---")
                        || is_marker(&self.lines[self.pos], "...")))
            {
                break;
            }
            if i > ind {
                return err(no, "this line is indented deeper than its siblings");
            }
            if !is_seq_start(&text) {
                break;
            }
            let after = &text[1..];
            let rest = after.trim_start_matches(' ');
            let col = ind + 1 + (after.len() - rest.len());
            if rest.is_empty() || rest.starts_with('#') {
                self.pos += 1;
                out.push(match self.cur()? {
                    Some((n, _, _)) if n > ind => self.block(n, ind as i64)?,
                    _ => Value::Null,
                });
            } else {
                let l = &mut self.lines[self.pos];
                l.indent = col;
                l.text = rest.to_string();
                out.push(self.block(col, ind as i64)?);
            }
        }
        Ok(Value::Array(out))
    }

    fn map(&mut self, ind: usize) -> Res<Value> {
        let mut out = Map::new();
        loop {
            let Some((i, text, no)) = self.cur()? else {
                break;
            };
            if i < ind
                || (i == 0
                    && (is_marker(&self.lines[self.pos], "---")
                        || is_marker(&self.lines[self.pos], "...")))
            {
                break;
            }
            if i > ind {
                return err(no, "this line is indented deeper than its siblings");
            }
            if text == "?" || text.starts_with("? ") {
                return err(no, "complex keys (`?`) are not read");
            }
            let Some((kraw, rest)) = key_split(&text) else {
                return err(no, "expected `key: value` here");
            };
            let key = self.key(kraw, no)?;
            if out.contains_key(&key) {
                return err(no, &format!("the key `{key}` appears twice in one mapping"));
            }
            let rest = rest.to_string();
            let v = if strip_comment(&rest).is_empty() {
                self.pos += 1;
                match self.cur()? {
                    Some((n, _, _)) if n > ind => self.block(n, ind as i64)?,
                    Some((n, t, _)) if n == ind && is_seq_start(&t) => self.seq(ind)?,
                    _ => Value::Null,
                }
            } else {
                self.value(&rest, ind as i64, no)?
            };
            out.insert(key, v);
        }
        Ok(Value::Object(out))
    }

    fn key(&self, kraw: &str, no: usize) -> Res<String> {
        if kraw.starts_with(['"', '\'']) {
            let chars: Vec<char> = kraw.chars().collect();
            return match scan_quoted(&chars, no)? {
                Some((s, _)) => Ok(s),
                None => err(no, "an unterminated quoted key"),
            };
        }
        if let Some(c) = kraw.chars().next() {
            refuse_indicator(c, no)?;
            if kraw.starts_with('[') || kraw.starts_with('{') {
                return err(no, "complex keys are not read");
            }
        } else {
            return err(no, "an empty key; write it quoted");
        }
        if kraw == "<<" {
            return err(
                no,
                "merge keys (`<<`) are not read; write the merged entries out",
            );
        }
        Ok(kraw.to_string())
    }

    /// The value written from `text` on the current line, onward; consumes
    /// the lines it spans.
    fn value(&mut self, text: &str, parent: i64, no: usize) -> Res<Value> {
        let first = text.chars().next().expect("a value");
        match first {
            '|' | '>' => return self.block_scalar(text, parent, no),
            '[' | '{' => return self.flow_value(text, no),
            '"' | '\'' => return self.quoted(text, no),
            _ => refuse_indicator(first, no)?,
        }
        if is_seq_start(text) {
            return err(no, "a block sequence entry is not allowed here");
        }
        if key_split(text).is_some() {
            return err(
                no,
                "a mapping value is not allowed here; start the mapping on its own line",
            );
        }
        self.pos += 1;
        let mut out = strip_comment(text).to_string();
        let mut done = out.len() != text.trim_end().len();
        let mut multi = false;
        while !done {
            let mut j = self.pos;
            while j < self.lines.len() && self.lines[j].text.trim().is_empty() {
                j += 1;
            }
            let Some(l) = self.lines.get(j) else { break };
            if (l.indent as i64) <= parent
                || l.text.starts_with('#')
                || is_marker(l, "---")
                || is_marker(l, "...")
            {
                break;
            }
            if key_split(&l.text).is_some() {
                return err(l.no, "a mapping value is not allowed here; is the line indented to match its siblings?");
            }
            let t = strip_comment(&l.text);
            done = t.len() != l.text.trim_end().len();
            if j > self.pos {
                out.extend(std::iter::repeat('\n').take(j - self.pos));
            } else {
                out.push(' ');
            }
            out.push_str(t);
            multi = true;
            self.pos = j + 1;
        }
        Ok(if multi {
            Value::String(out)
        } else {
            resolve(&out)
        })
    }

    fn quoted(&mut self, text: &str, no: usize) -> Res<Value> {
        self.pos += 1;
        let mut buf: Vec<char> = text.chars().collect();
        loop {
            if let Some((s, end)) = scan_quoted(&buf, no)? {
                let rest: String = buf[end..].iter().collect();
                let rest = rest.trim();
                if !rest.is_empty() && !rest.starts_with('#') {
                    return err(no, "text after a quoted scalar");
                }
                return Ok(Value::String(s));
            }
            let Some(l) = self.lines.get(self.pos) else {
                return err(no, "a quoted scalar is never closed");
            };
            buf.push('\n');
            buf.extend(l.raw.chars());
            self.pos += 1;
        }
    }

    fn block_scalar(&mut self, header: &str, parent: i64, no: usize) -> Res<Value> {
        let h = strip_comment(header);
        let (mut chomp, mut explicit) = (' ', None);
        for c in h[1..].chars() {
            match c {
                '-' | '+' if chomp == ' ' => chomp = c,
                '1'..='9' if explicit.is_none() => explicit = Some(c as usize - '0' as usize),
                _ => return err(no, "a block scalar header is `|` or `>` with an optional chomping `-` or `+` and indentation digit"),
            }
        }
        let literal = h.starts_with('|');
        self.pos += 1;
        let start = self.pos;
        let mut j = start;
        let content_indent = match explicit {
            Some(d) => (parent.max(0) as usize) + d,
            None => {
                let mut k = start;
                while k < self.lines.len() && self.lines[k].raw.trim().is_empty() {
                    k += 1;
                }
                match self.lines.get(k) {
                    Some(l) if (l.indent as i64) > parent => l.indent,
                    _ => usize::MAX,
                }
            }
        };
        let mut body: Vec<String> = Vec::new();
        while let Some(l) = self.lines.get(j) {
            if l.raw.trim().is_empty() {
                body.push(
                    l.raw
                        .get(content_indent.min(l.raw.len())..)
                        .unwrap_or("")
                        .to_string(),
                );
            } else if content_indent != usize::MAX && l.indent >= content_indent {
                body.push(l.raw[content_indent..].to_string());
            } else {
                break;
            }
            j += 1;
        }
        self.pos = j;
        let mut trailing = 0;
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
            trailing += 1;
        }
        let mut out = String::new();
        if literal {
            out = body.join("\n");
        } else {
            let mut blanks = 0;
            let mut prev: Option<&String> = None;
            for l in &body {
                if l.trim().is_empty() {
                    blanks += 1;
                    continue;
                }
                let more = |s: &str| s.starts_with([' ', '\t']);
                match prev {
                    None => out.extend(std::iter::repeat('\n').take(blanks)),
                    Some(p) => {
                        let extra = usize::from(more(p) || more(l));
                        if blanks + extra == 0 {
                            out.push(' ');
                        } else {
                            out.extend(std::iter::repeat('\n').take(blanks + extra));
                        }
                    }
                }
                out.push_str(l);
                blanks = 0;
                prev = Some(l);
            }
        }
        if !body.is_empty() {
            match chomp {
                '-' => {}
                '+' => out.extend(std::iter::repeat('\n').take(1 + trailing)),
                _ => out.push('\n'),
            }
        } else if chomp == '+' {
            out.extend(std::iter::repeat('\n').take(trailing));
        }
        Ok(Value::String(out))
    }

    /// A flow collection opening `text`, which may run onto later lines.
    fn flow_value(&mut self, text: &str, no: usize) -> Res<Value> {
        self.pos += 1;
        let mut buf = strip_comment(text).to_string();
        while depth(&buf) > 0 {
            let Some(l) = self.lines.get(self.pos) else {
                return err(no, "a flow collection is never closed");
            };
            buf.push(' ');
            buf.push_str(strip_comment(l.text.trim()));
            self.pos += 1;
        }
        let mut f = Flow {
            c: buf.chars().collect(),
            i: 0,
            no,
        };
        let v = f.value()?;
        f.ws();
        if f.i < f.c.len() {
            return err(no, "text after a flow collection");
        }
        Ok(v)
    }
}

/// Open brackets in `s`, outside quotes.
fn depth(s: &str) -> i32 {
    let (mut d, mut q) = (0, None);
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match q {
            Some('"') if c == '\\' => i += 1,
            Some(k) if c == k => q = None,
            Some(_) => {}
            None => match c {
                '[' | '{' => d += 1,
                ']' | '}' => d -= 1,
                '"' | '\'' if i == 0 || matches!(b[i - 1], ' ' | '[' | '{' | ',' | ':') => {
                    q = Some(c)
                }
                _ => {}
            },
        }
        i += 1;
    }
    d
}

struct Flow {
    c: Vec<char>,
    i: usize,
    no: usize,
}

impl Flow {
    fn ws(&mut self) {
        while matches!(self.c.get(self.i), Some(' ' | '\t')) {
            self.i += 1;
        }
    }

    fn quoted(&mut self) -> Res<String> {
        match scan_quoted(&self.c[self.i..], self.no)? {
            Some((s, n)) => {
                self.i += n;
                Ok(s)
            }
            None => err(self.no, "a quoted scalar is never closed"),
        }
    }

    /// A plain scalar's text, up to a flow indicator.
    fn plain(&mut self) -> Res<String> {
        let start = self.i;
        while let Some(&ch) = self.c.get(self.i) {
            let next = self.c.get(self.i + 1).copied();
            if matches!(ch, ',' | ']' | '}')
                || (ch == ':' && matches!(next, None | Some(' ' | ',' | ']' | '}')))
                || (ch == '#' && self.i > start && self.c[self.i - 1] == ' ')
            {
                break;
            }
            self.i += 1;
        }
        let s: String = self.c[start..self.i].iter().collect();
        let s = s.trim().to_string();
        if let Some(ch) = s.chars().next() {
            refuse_indicator(ch, self.no)?;
        }
        Ok(s)
    }

    fn value(&mut self) -> Res<Value> {
        self.ws();
        match self.c.get(self.i) {
            Some('[') => {
                self.i += 1;
                let mut out = Vec::new();
                loop {
                    self.ws();
                    match self.c.get(self.i) {
                        Some(']') => {
                            self.i += 1;
                            return Ok(Value::Array(out));
                        }
                        None => return err(self.no, "a flow sequence is never closed"),
                        _ => {}
                    }
                    let v = self.item()?;
                    // `[k: v]` is a one-pair mapping
                    self.ws();
                    out.push(v);
                    match self.c.get(self.i) {
                        Some(',') => self.i += 1,
                        Some(']') => {}
                        _ => return err(self.no, "expected `,` or `]` in a flow sequence"),
                    }
                }
            }
            Some('{') => {
                self.i += 1;
                let mut out = Map::new();
                loop {
                    self.ws();
                    match self.c.get(self.i) {
                        Some('}') => {
                            self.i += 1;
                            return Ok(Value::Object(out));
                        }
                        None => return err(self.no, "a flow mapping is never closed"),
                        Some('[' | '{' | '?') => return err(self.no, "complex keys are not read"),
                        _ => {}
                    }
                    let key = if matches!(self.c.get(self.i), Some('"' | '\'')) {
                        self.quoted()?
                    } else {
                        let k = self.plain()?;
                        if k == "<<" {
                            return err(
                                self.no,
                                "merge keys (`<<`) are not read; write the merged entries out",
                            );
                        }
                        k
                    };
                    self.ws();
                    let v = if self.c.get(self.i) == Some(&':') {
                        self.i += 1;
                        self.ws();
                        if matches!(self.c.get(self.i), Some(',' | '}')) {
                            Value::Null
                        } else {
                            self.value()?
                        }
                    } else {
                        Value::Null
                    };
                    if out.insert(key.clone(), v).is_some() {
                        return err(
                            self.no,
                            &format!("the key `{key}` appears twice in one mapping"),
                        );
                    }
                    self.ws();
                    match self.c.get(self.i) {
                        Some(',') => self.i += 1,
                        Some('}') => {}
                        _ => return err(self.no, "expected `,` or `}` in a flow mapping"),
                    }
                }
            }
            Some('"' | '\'') => Ok(Value::String(self.quoted()?)),
            Some(_) => {
                let s = self.plain()?;
                Ok(resolve(&s))
            }
            None => err(self.no, "a flow collection ends too soon"),
        }
    }

    /// A sequence item: a value, or a `key: value` pair.
    fn item(&mut self) -> Res<Value> {
        let quoted = matches!(self.c.get(self.i), Some('"' | '\''));
        let at = self.i;
        let v = self.value()?;
        self.ws();
        if self.c.get(self.i) == Some(&':') && !matches!(v, Value::Array(_) | Value::Object(_)) {
            let key = if quoted {
                match v {
                    Value::String(s) => s,
                    _ => unreachable!(),
                }
            } else {
                self.c[at..self.i]
                    .iter()
                    .collect::<String>()
                    .trim()
                    .to_string()
            };
            self.i += 1;
            self.ws();
            let val = if matches!(self.c.get(self.i), Some(',' | ']')) {
                Value::Null
            } else {
                self.value()?
            };
            let mut m = Map::new();
            m.insert(key, val);
            return Ok(Value::Object(m));
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn y(s: &str) -> Value {
        parse(s).unwrap_or_else(|e| panic!("{e}\n{s}"))
    }

    fn refused(s: &str, line: usize, word: &str) {
        let e = parse(s).expect_err(s);
        assert!(e.starts_with(&format!("line {line}: ")), "{e} for\n{s}");
        assert!(e.contains(word), "{e} lacks `{word}` for\n{s}");
    }

    #[test]
    fn block_mappings_and_sequences() {
        let v = y("a: 1\nb:\n  c: x\n  d:\n    - 1\n    - two\ne:\n- p\n- q\n");
        assert_eq!(
            v,
            json!({"a": 1, "b": {"c": "x", "d": [1, "two"]}, "e": ["p", "q"]})
        );
    }

    #[test]
    fn sequence_items_that_are_mappings_and_sequences() {
        let v = y(
            "- name: a\n  in: query\n  schema:\n    type: integer\n- - 1\n  - 2\n-\n  k: v\n- \n",
        );
        assert_eq!(
            v,
            json!([{"name": "a", "in": "query", "schema": {"type": "integer"}}, [1, 2], {"k": "v"}, null])
        );
    }

    #[test]
    fn plain_scalars_read_by_the_core_schema() {
        let v = y("a: ~\nb: null\nc: true\nd: False\ne: 12\nf: -3\ng: 1.5\nh: 1e3\ni: 0x1F\nj: 0o17\nk: on\nl: yes\nm: no\nn: 1.2.3\no: .5\np: http://x/y\nq:\n");
        assert_eq!(
            v,
            json!({"a": null, "b": null, "c": true, "d": false, "e": 12, "f": -3, "g": 1.5, "h": 1000.0,
                   "i": 31, "j": 15, "k": "on", "l": "yes", "m": "no", "n": "1.2.3", "o": 0.5,
                   "p": "http://x/y", "q": null})
        );
    }

    #[test]
    fn keys_are_the_text_they_are_written_as() {
        let v = y(
            "responses:\n  200:\n    description: ok\n  'default': x\n  \"a b\": y\n  true: z\n",
        );
        assert_eq!(
            v,
            json!({"responses": {"200": {"description": "ok"}, "default": "x", "a b": "y", "true": "z"}})
        );
    }

    #[test]
    fn quoted_scalars_and_their_escapes() {
        let v = y("a: 'it''s'\nb: \"tab\\there \\u00e9 \\x41 \\\"q\\\" \\\\\"\nc: \"123\"\nd: ''\ne: 'x: y # z'\n");
        assert_eq!(
            v,
            json!({"a": "it's", "b": "tab\there \u{e9} A \"q\" \\", "c": "123", "d": "", "e": "x: y # z"})
        );
        let v = y("a: \"one\n  two\n\n  three\"\nb: 'x\n  y'\n");
        assert_eq!(v, json!({"a": "one two\nthree", "b": "x y"}));
    }

    #[test]
    fn comments() {
        let v =
            y("# top\na: 1 # one\nb: 'x # y' # z\n\n  # indented\nc: # none\n  - d # d\n#end\n");
        assert_eq!(v, json!({"a": 1, "b": "x # y", "c": ["d"]}));
        assert_eq!(y("a: b#c\n"), json!({"a": "b#c"}));
    }

    #[test]
    fn multi_line_plain_scalars() {
        let v = y("a: one\n  two\n\n  three\nb: x\n");
        assert_eq!(v, json!({"a": "one two\nthree", "b": "x"}));
    }

    #[test]
    fn literal_block_scalars_and_chomping() {
        let v = y("a: |\n  l1\n    l2\n\n  l3\n\nb: |-\n  x\nc: |+\n  y\n\nd: |\n  z\n");
        assert_eq!(
            v,
            json!({"a": "l1\n  l2\n\nl3\n", "b": "x", "c": "y\n\n", "d": "z\n"})
        );
        assert_eq!(y("a: |2\n    x\n"), json!({"a": "  x\n"}));
        assert_eq!(y("- |\n  in seq\n- next\n"), json!(["in seq\n", "next"]));
        assert_eq!(y("a: |\nb: 1\n"), json!({"a": "", "b": 1}));
    }

    #[test]
    fn folded_block_scalars() {
        let v =
            y("a: >\n  one\n  two\n\n  three\n    more\n  four\nb: >-\n  x\n  y\nc: >+\n  z\n\n");
        assert_eq!(
            v,
            json!({"a": "one two\nthree\n  more\nfour\n", "b": "x y", "c": "z\n\n"})
        );
    }

    #[test]
    fn flow_collections() {
        let v = y("a: [1, 'b', \"c\", [x, y], {k: v}]\nb: {p: 1, q: [2, 3], r: , 's t': ~}\nc: []\nd: {}\ne: [a: 1]\n");
        assert_eq!(
            v,
            json!({"a": [1, "b", "c", ["x", "y"], {"k": "v"}], "b": {"p": 1, "q": [2, 3], "r": null, "s t": null},
                   "c": [], "d": {}, "e": [{"a": 1}]})
        );
        let v = y("a: [1,\n  2, # two\n  3]\nb: {\n  k: v,\n}\n");
        assert_eq!(v, json!({"a": [1, 2, 3], "b": {"k": "v"}}));
        assert_eq!(y("[a, {b: c}]\n"), json!(["a", {"b": "c"}]));
    }

    #[test]
    fn document_markers() {
        assert_eq!(y("---\na: 1\n...\n"), json!({"a": 1}));
        assert_eq!(y("--- # c\na: 1\n"), json!({"a": 1}));
        assert_eq!(y("--- text\n"), json!("text"));
        assert_eq!(y(""), Value::Null);
        assert_eq!(y("# only\n"), Value::Null);
        assert_eq!(y("a: 1\r\nb: 2\r\n"), json!({"a": 1, "b": 2}));
    }

    #[test]
    fn a_yaml_document_reads_as_its_json() {
        let json_doc = r#"{"a": [1, 2.5, "x", null, true], "b": {"c": "d e"}}"#;
        assert_eq!(
            y(json_doc),
            serde_json::from_str::<Value>(json_doc).unwrap()
        );
    }

    #[test]
    fn what_the_subset_does_not_read_is_refused_with_its_line() {
        refused("a: 1\nb: &x 2\n", 2, "anchors");
        refused("a: 1\nb: *x\n", 2, "aliases");
        refused("a: 1\n<<: {b: 2}\n", 2, "merge keys");
        refused("a: 1\nb: {<<: x}\n", 2, "merge keys");
        refused("a: 1\nb: !!str 2\n", 2, "tags");
        refused("a: 1\n? [x]\n: y\n", 2, "complex keys");
        refused("a: 1\n---\nb: 2\n", 2, "more than one document");
        refused("---\na: 1\n---\nb: 2\n", 3, "more than one document");
        refused("a: 1\n...\nb: 2\n", 3, "content after");
        refused("a: 1\na: 2\n", 2, "twice");
        refused("a:\n\tb: 1\n", 2, "tab");
        refused("%YAML 1.2\n---\na: 1\n", 1, "directives");
        refused("a: 'open\n", 1, "never closed");
        refused("a: [1, 2\n", 1, "never closed");
        refused("a: \"\\q\"\n", 1, "escape");
        refused("a: b: c\n", 1, "mapping value");
        refused("a: 1\n  b: 2\n", 2, "mapping value");
        refused("a:\n  b: 1\n c: 2\n", 3, "deeper");
        refused("a: @x\n", 1, "quote it");
    }
}
