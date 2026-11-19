//! GH #1674: `///` doc comments (spec/tokens.md § Doc comments).
//!
//! The lexer drops every comment, a doc comment among them, so the token
//! stream (and everything that reads it: the parser's productions, `hale
//! fmt`'s gate) is the same with or without docs. A doc comment is found
//! here instead, in the gaps between the tokens a parse read, and
//! attached after the parse to the declaration it precedes: a type, a
//! record field, an enum variant, a fn (a handler among them) and its
//! parameters, a locus and its params, a constant, an interface and its
//! methods, a topic, a role, a surface and its rows. A doc comment
//! anywhere else documents nothing, and the parse records it as a
//! [`StrayDoc`] the checker warns at.
//!
//! A doc comment is the lines between two tokens that each begin, after
//! indentation, with `///` and not `////`. Blank lines and ordinary
//! comments may sit among them and after them; they are not part of the
//! doc. A `///` after code on its own line documents nothing.

use std::collections::BTreeMap;

use crate::ast::*;
use crate::lexer::{Token, TokenKind};
use crate::span::Span;

/// A doc comment found before the token at index `next`.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub text: String,
    pub span: Span,
    pub next: usize,
}

/// The doc comments of `source`, which `tokens` were lexed from (their
/// spans offset by the same base, which the `Eof` token's position tells),
/// and the doc comments that can document nothing. A token stream that
/// does not fit the source (no `Eof`, or one past the text) yields none.
pub fn scan(source: &str, tokens: &[Token]) -> (Vec<Found>, Vec<StrayDoc>) {
    let mut found = Vec::new();
    let mut stray = Vec::new();
    let Some(eof) = tokens.last().filter(|t| matches!(t.kind, TokenKind::Eof)) else {
        return (found, stray);
    };
    let Some(base) = (eof.span.start.0 as usize).checked_sub(source.len()) else {
        return (found, stray);
    };
    let local = |p: u32| (p as usize).checked_sub(base);
    let mut prev_end = 0usize;
    for (i, tok) in tokens.iter().enumerate() {
        let (Some(start), Some(end)) = (local(tok.span.start.0), local(tok.span.end.0)) else { return (found, stray) };
        let Some(gap) = source.get(prev_end..start) else { return (found, stray) };
        if gap.contains("///") {
            scan_gap(gap, prev_end, base, i, i > 0, matches!(tok.kind, TokenKind::Eof), &mut found, &mut stray);
        }
        prev_end = end.max(prev_end);
    }
    (found, stray)
}

/// One gap between tokens: `at` its offset in the source, `after_code`
/// whether a token precedes it.
#[allow(clippy::too_many_arguments)]
fn scan_gap(
    gap: &str,
    at: usize,
    base: usize,
    next: usize,
    after_code: bool,
    at_eof: bool,
    found: &mut Vec<Found>,
    stray: &mut Vec<StrayDoc>,
) {
    let bytes = gap.as_bytes();
    let span = |a: usize, b: usize| Span::new(base + at + a, base + at + b);
    // the doc lines read so far and where the first starts and the last ends
    let mut run: Vec<&str> = Vec::new();
    let mut run_span: Option<(usize, usize)> = None;
    let mut newline_seen = !after_code;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                newline_seen = true;
                i += 1;
            }
            // a block comment is no doc (a `///` inside it is its text)
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = gap[i + 2..].find("*/").map_or(bytes.len(), |k| i + 2 + k + 2);
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                let eol = gap[i..].find('\n').map_or(bytes.len(), |k| i + k);
                let line = &gap[i..eol];
                if line.starts_with("///") && !line.starts_with("////") {
                    if newline_seen {
                        let text = &line[3..];
                        let text = text.strip_prefix(' ').unwrap_or(text);
                        run.push(text.trim_end());
                        run_span = Some((run_span.map_or(i, |(a, _)| a), eol));
                    } else {
                        // after code on its line: documents nothing after it
                        stray.push(StrayDoc { span: span(i, eol), trailing: true });
                    }
                }
                i = eol;
            }
            // whitespace: the lexer leaves nothing else between two tokens
            _ => i += 1,
        }
    }
    if let Some((a, b)) = run_span {
        if at_eof {
            stray.push(StrayDoc { span: span(a, b), trailing: false });
        } else {
            let mut lines: &[&str] = &run;
            while let [first, rest @ ..] = lines {
                if !first.is_empty() {
                    break;
                }
                lines = rest;
            }
            while let [rest @ .., last] = lines {
                if !last.is_empty() {
                    break;
                }
                lines = rest;
            }
            found.push(Found { text: lines.join("\n"), span: span(a, b), next });
        }
    }
}

/// Attach the doc comments `found` (each before the token at its index in
/// `tokens`) to the declarations of `prog` they precede, and record every
/// one no declaration takes, with `stray`, as the program's stray docs.
///
/// A declaration takes the doc comments written in its head, before its
/// name: walking back from its name's token through its keyword and the
/// annotations and modifiers written ahead of it (`@rpc(requires:
/// [trader])`, `export`, `main`, an error class) to a `;`, `,`, `{` or `}`
/// outside a bracket, or an opening bracket. So a doc may stand above an
/// annotation or between it and the keyword.
pub fn attach(prog: &mut Program, tokens: &[Token], found: Vec<Found>, mut stray: Vec<StrayDoc>) {
    if found.is_empty() && stray.is_empty() {
        return;
    }
    let mut docs: BTreeMap<usize, Found> = found.into_iter().map(|f| (f.next, f)).collect();
    if !docs.is_empty() {
        let mut a = Attach { tokens, docs: &mut docs };
        for item in &mut prog.items {
            a.top(item);
        }
    }
    stray.extend(docs.into_values().map(|f| StrayDoc { span: f.span, trailing: false }));
    stray.sort_by_key(|d| d.span.start);
    prog.stray_docs = stray;
}

struct Attach<'t, 'd> {
    tokens: &'t [Token],
    docs: &'d mut BTreeMap<usize, Found>,
}

impl Attach<'_, '_> {
    /// The doc comments in the head of a declaration named `name`, taken
    /// and joined in source order.
    fn take(&mut self, name: &Ident) -> Doc {
        if self.docs.is_empty() {
            return Doc::NONE;
        }
        let Ok(mut k) = self.tokens.binary_search_by_key(&name.span.start, |t| t.span.start) else {
            return Doc::NONE;
        };
        let mut texts: Vec<String> = Vec::new();
        // brackets closed walking back: `)` and `]` open one, `(` and `[` close it
        let mut depth = 0usize;
        loop {
            if let Some(f) = self.docs.remove(&k) {
                texts.push(f.text);
            }
            if k == 0 {
                break;
            }
            k -= 1;
            match &self.tokens[k].kind {
                TokenKind::RParen | TokenKind::RBracket => depth += 1,
                TokenKind::LParen | TokenKind::LBracket if depth > 0 => depth -= 1,
                TokenKind::LParen | TokenKind::LBracket => break,
                TokenKind::Semi | TokenKind::Comma | TokenKind::LBrace | TokenKind::RBrace if depth == 0 => break,
                _ => {}
            }
        }
        if texts.is_empty() {
            return Doc::NONE;
        }
        texts.reverse();
        Doc(Some(texts.join("\n")))
    }

    fn top(&mut self, item: &mut TopDecl) {
        match item {
            TopDecl::Locus(l) => self.locus(l),
            TopDecl::Type(t) => self.ty(t),
            TopDecl::Const(c) => c.doc = self.take(&c.name),
            TopDecl::Fn(f) => self.func(f),
            TopDecl::Interface(i) => {
                i.doc = self.take(&i.name);
                for m in &mut i.methods {
                    m.doc = self.take(&m.name);
                    for p in &mut m.params {
                        p.doc = self.take(&p.name);
                    }
                }
            }
            TopDecl::Topic(t) => t.doc = self.take(&t.name),
            TopDecl::Role(r) => r.doc = self.take(&r.name),
            TopDecl::Api(a) => {
                a.doc = self.take(&a.name);
                for r in &mut a.rows {
                    r.doc = self.take(&r.method);
                }
            }
            TopDecl::RingLayout(_)
            | TopDecl::Target(_)
            | TopDecl::Group(_)
            | TopDecl::Claims(_)
            | TopDecl::Constitution(_)
            | TopDecl::Unit(_)
            | TopDecl::Fleet(_) => {}
        }
    }

    fn ty(&mut self, t: &mut TypeDecl) {
        t.doc = self.take(&t.name);
        match &mut t.body {
            TypeDeclBody::Struct(fields) => {
                for f in fields {
                    f.doc = self.take(&f.name);
                }
            }
            TypeDeclBody::Enum(variants) => {
                for v in variants {
                    v.doc = self.take(&v.name);
                }
            }
            TypeDeclBody::Alias(_) | TypeDeclBody::Scalar(_) => {}
        }
    }

    fn func(&mut self, f: &mut FnDecl) {
        f.doc = self.take(&f.name);
        for p in &mut f.params {
            p.doc = self.take(&p.name);
        }
    }

    fn locus(&mut self, l: &mut LocusDecl) {
        l.doc = self.take(&l.name);
        for m in &mut l.members {
            match m {
                LocusMember::Params(pb) => {
                    for p in &mut pb.params {
                        p.doc = self.take(&p.name);
                    }
                }
                LocusMember::Fn(f) => self.func(f),
                LocusMember::Const(c) => c.doc = self.take(&c.name),
                LocusMember::Type(t) => self.ty(t),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Program {
        crate::parse_source(src).expect("parses")
    }

    fn type_decl(p: &Program, name: &str) -> TypeDecl {
        p.items
            .iter()
            .find_map(|i| match i {
                TopDecl::Type(t) if t.name.name == name => Some(t.clone()),
                _ => None,
            })
            .expect("the type")
    }

    #[test]
    fn a_doc_attaches_to_the_type_its_fields_and_its_variants() {
        let p = parse(
            "/// An order to place.\n/// Second line.\ntype Order {\n    /// The symbol.\n    symbol: String;\n    qty: Int; /// trailing\n}\n\
             /// Why it failed.\ntype E = enum {\n    /// Not there.\n    @not_found Missing,\n    Closed,\n};\n",
        );
        let o = type_decl(&p, "Order");
        assert_eq!(o.doc.text(), Some("An order to place.\nSecond line."));
        assert_eq!(o.doc.summary(), Some("An order to place."));
        assert_eq!(o.doc.rest(), Some("Second line."));
        let TypeDeclBody::Struct(fs) = &o.body else { panic!() };
        assert_eq!(fs[0].doc.text(), Some("The symbol."));
        assert_eq!(fs[1].doc.text(), None);
        let e = type_decl(&p, "E");
        assert_eq!(e.doc.text(), Some("Why it failed."));
        let TypeDeclBody::Enum(vs) = &e.body else { panic!() };
        assert_eq!(vs[0].doc.text(), Some("Not there."));
        assert_eq!(vs[1].doc.text(), None);
        // the trailing one documents nothing
        assert_eq!(p.stray_docs.len(), 1);
        assert!(p.stray_docs[0].trailing);
    }

    #[test]
    fn a_doc_reaches_back_over_annotations_and_attaches_to_rows() {
        let p = parse(
            "role trader;\ntype R { x: Int; }\n/// The desk.\nlocus Desk {\n    params {\n        /// How many.\n        n: Int = 0;\n    }\n\
             /// Place one.\n    @rpc(requires: [trader])\n    fn place(\n        /// The request.\n        r: R) -> R { return r; }\n}\n\
             /// The public surface.\napi Public {\n    /// Cancel it.\n    rpc Desk::place;\n}\n",
        );
        assert!(p.stray_docs.is_empty(), "{:?}", p.stray_docs);
        let l = p
            .items
            .iter()
            .find_map(|i| match i {
                TopDecl::Locus(l) => Some(l),
                _ => None,
            })
            .unwrap();
        assert_eq!(l.doc.text(), Some("The desk."));
        let mut saw = 0;
        for m in &l.members {
            match m {
                LocusMember::Params(pb) => {
                    assert_eq!(pb.params[0].doc.text(), Some("How many."));
                    saw += 1;
                }
                LocusMember::Fn(f) => {
                    assert_eq!(f.doc.text(), Some("Place one."));
                    assert_eq!(f.params[0].doc.text(), Some("The request."));
                    saw += 1;
                }
                _ => {}
            }
        }
        assert_eq!(saw, 2);
        let a = p
            .items
            .iter()
            .find_map(|i| match i {
                TopDecl::Api(a) => Some(a),
                _ => None,
            })
            .unwrap();
        assert_eq!(a.doc.text(), Some("The public surface."));
        assert_eq!(a.rows[0].doc.text(), Some("Cancel it."));
    }

    #[test]
    fn a_doc_in_a_position_nothing_reads_is_stray() {
        let p = parse(
            "/// Before an import-free statement-level spot.\nlocus L {\n    run() {\n        /// inside a body\n        let x = 1;\n    }\n}\n/// at the end\n",
        );
        let l = p
            .items
            .iter()
            .find_map(|i| match i {
                TopDecl::Locus(l) => Some(l),
                _ => None,
            })
            .unwrap();
        assert!(l.doc.text().is_some());
        assert_eq!(p.stray_docs.len(), 2, "{:?}", p.stray_docs);
        assert!(p.stray_docs.iter().all(|d| !d.trailing));
    }

    #[test]
    fn ordinary_comments_are_not_doc_and_four_slashes_are_ordinary() {
        let p = parse(
            "/// The doc.\n// an implementation note\n/* a block */\ntype A { x: Int; }\n//// ruled\ntype B { x: Int; }\n\
             @form(vec)\n/// Between the annotation and the keyword.\nlocus L { }\n",
        );
        assert_eq!(type_decl(&p, "A").doc.text(), Some("The doc."));
        assert_eq!(type_decl(&p, "B").doc.text(), None);
        let l = p
            .items
            .iter()
            .find_map(|i| match i {
                TopDecl::Locus(l) => Some(l),
                _ => None,
            })
            .unwrap();
        assert_eq!(l.doc.text(), Some("Between the annotation and the keyword."));
        assert!(p.stray_docs.is_empty(), "{:?}", p.stray_docs);
    }

    #[test]
    fn hale_fmt_keeps_every_doc_comment() {
        let src = "/// The thing.\n///\n///   an indented line\ntype   T {\n/// a field\n      x:Int;\n  y: Int; /// trailing\n}\n";
        let out = crate::fmt::format_source(src).expect("formats");
        assert!(
            out.contains("/// The thing.\n///\n///   an indented line\ntype T {\n    /// a field\n    x: Int;\n    y: Int; /// trailing\n}"),
            "{out}"
        );
        assert_eq!(crate::fmt::format_source(&out).expect("formats"), out, "a fixed point");
        let (a, b) = (parse(src), parse(&out));
        let (ta, tb) = (type_decl(&a, "T"), type_decl(&b, "T"));
        assert_eq!(ta.doc, tb.doc);
        assert_eq!(ta.doc.text(), Some("The thing.\n\n  an indented line"));
        let field_docs = |t: &TypeDecl| match &t.body {
            TypeDeclBody::Struct(fs) => fs.iter().map(|f| f.doc.clone()).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        assert_eq!(field_docs(&ta), field_docs(&tb));
        assert_eq!(field_docs(&ta)[0].text(), Some("a field"));
        assert_eq!(a.stray_docs.len(), b.stray_docs.len());
    }

    #[test]
    fn docs_survive_a_parse_at_a_base() {
        let src = "/// The thing.\ntype T {\n    /// a field\n    x: Int;\n}\n";
        let p = crate::parse_source_at(src, 4096).expect("parses");
        let t = type_decl(&p, "T");
        assert_eq!(t.doc.text(), Some("The thing."));
        let TypeDeclBody::Struct(fs) = &t.body else { panic!() };
        assert_eq!(fs[0].doc.text(), Some("a field"));
    }
}
