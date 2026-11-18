//! RFC #2035 (F.54): declared display templates.
//!
//! `@display("${dollars}.{cents:0>2} {currency}") type Money { … }` and
//! `@display("other: {0}") Other(String)` on an enum's variant declare how
//! a value renders as text for people. Every rendering path reads it:
//! `println`, `print`, `to_string`, `String + x` and an f-string's
//! interpolation. `{x:?}` bypasses it, at every depth.
//!
//! A template is literal text and holes. A hole names a field of the
//! record, or a payload position of the variant (`{0}`), with an optional
//! f-string format spec. It holds no expression, call or conditional, so
//! rendering through one cannot fail or run anything. `{{` and `}}` are a
//! literal brace.
//!
//! The desugar writes one function per template ([`generate_displays_in`]):
//! `__display_tpl_<T>(__v: T) -> String` for a record, and
//! `__display_tpl_<E>__<V>(__v: E) -> String` for an enum's variant. Each
//! hole is `to_string(field)` or `__fmt(field, "spec")`, the calls an
//! f-string makes, placed at the hole's span. So the checker holds each
//! hole to the f-string rules (a printable value, a spec that fits it) at
//! the template, and lowering renders a value of the type by calling the
//! function. `T::display(v)` is written as `__display_<T>`, whose body is
//! `to_string(v)`, and a use of it is rewritten to call it. Both are
//! ordinary functions: what they allocate lands in the caller's frame, as
//! the f-string they stand for does, and the analyses read them as they
//! read any function.

use std::collections::{BTreeMap, HashSet};

use crate::ast::{
    flat_decls, DisplayTemplate, EnumVariant, Expr, Ident, Program, QualifiedName, TemplatePart, TopDecl,
    TypeDeclBody,
};
use crate::error::Diag;
use crate::json_gen::{hale_str, next_synth_base, rewrite_items};
use crate::span::Span;

/// The function a record's template renders through.
pub fn record_fn(t: &str) -> String {
    format!("__display_tpl_{t}")
}

/// The function an enum variant's template renders through.
pub fn variant_fn(e: &str, v: &str) -> String {
    format!("__display_tpl_{e}__{v}")
}

/// The function `T::display` names.
pub fn display_fn(t: &str) -> String {
    format!("__display_{t}")
}

/// A function the desugar wrote, as a diagnostic names it: `T::display`,
/// and a template's `@display(T)` or `@display(E::V)`; `None` for any
/// other name.
pub fn generated_display(f: &str) -> Option<String> {
    if let Some(rest) = f.strip_prefix("__display_tpl_") {
        return Some(match rest.split_once("__") {
            Some((e, v)) => format!("@display({e}::{v})"),
            None => format!("@display({rest})"),
        });
    }
    f.strip_prefix("__display_").filter(|t| !t.is_empty()).map(|t| format!("{t}::display"))
}

/// Parse a template's text. `lit` is the string literal's span (quotes
/// included); a hole's span is its place in it, which is exact when the
/// text before it holds no escape.
pub fn parse_template(text: &str, ann: Span, lit: Span) -> Result<DisplayTemplate, Diag> {
    let base = lit.start.0 as usize + 1;
    let end = lit.end.0 as usize;
    let at = |from: usize, to: usize| -> Span {
        let s = (base + from).min(end);
        let e = (base + to).min(end).max(s);
        Span::new(s, e)
    };
    let mut parts = Vec::new();
    let mut lit_buf = String::new();
    let b = text.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'{' if b.get(i + 1) == Some(&b'{') => {
                lit_buf.push('{');
                i += 2;
            }
            b'}' if b.get(i + 1) == Some(&b'}') => {
                lit_buf.push('}');
                i += 2;
            }
            b'}' => {
                return Err(Diag::parse(
                    at(i, i + 1),
                    "a `}` in a `@display` template closes no hole; write `}}` for a literal brace",
                ));
            }
            b'{' => {
                let Some(close) = text[i + 1..].find('}').map(|c| i + 1 + c) else {
                    return Err(Diag::parse(
                        at(i, text.len()),
                        "a `{` in a `@display` template opens a hole that never closes; write `{{` for a literal brace",
                    ));
                };
                let body = &text[i + 1..close];
                let span = at(i, close + 1);
                let (key, spec) = match body.split_once(':') {
                    Some((k, s)) => (k.trim(), Some(s.trim())),
                    None => (body.trim(), None),
                };
                let ok_key = !key.is_empty()
                    && (key.bytes().all(|c| c.is_ascii_digit())
                        || (key.bytes().next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                            && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')));
                if !ok_key {
                    return Err(Diag::parse(
                        span,
                        format!(
                            "`{{{body}}}`: a `@display` hole names a field (`{{dollars}}`) or a variant's payload \
                             position (`{{0}}`), with an optional format spec (`{{cents:0>2}}`); a template holds no \
                             expression"
                        ),
                    ));
                }
                if let Some(sp) = spec {
                    crate::fstring::FormatSpec::parse(sp)
                        .map_err(|m| Diag::parse(span, format!("`@display` hole `{{{body}}}`: {m}")))?;
                }
                if !lit_buf.is_empty() {
                    parts.push(TemplatePart::Lit(std::mem::take(&mut lit_buf)));
                }
                parts.push(TemplatePart::Hole { key: key.to_string(), spec: spec.map(str::to_string), span });
                i = close + 1;
            }
            _ => {
                let c = text[i..].chars().next().expect("a char at a byte boundary");
                lit_buf.push(c);
                i += c.len_utf8();
            }
        }
    }
    if !lit_buf.is_empty() {
        parts.push(TemplatePart::Lit(lit_buf));
    }
    Ok(DisplayTemplate { text: text.to_string(), span: ann.merge(lit), parts })
}

/// The holes of `t` against a record's fields: the first that names no
/// field, as the refusal to report.
pub fn record_hole_refusal(t: &DisplayTemplate, type_name: &str, fields: &[&str]) -> Option<Diag> {
    t.parts.iter().find_map(|p| match p {
        TemplatePart::Hole { key, span, .. } if !fields.contains(&key.as_str()) => Some(Diag::parse(
            *span,
            if key.bytes().all(|c| c.is_ascii_digit()) {
                format!(
                    "`{{{key}}}` names a payload position, and `{type_name}` is a record: a record's template names \
                     its fields ({})",
                    fields.iter().map(|f| format!("`{{{f}}}`")).collect::<Vec<_>>().join(", ")
                )
            } else {
                format!(
                    "`{type_name}` has no field `{key}`: a `@display` hole names a field of the record ({})",
                    fields.iter().map(|f| format!("`{{{f}}}`")).collect::<Vec<_>>().join(", ")
                )
            },
        )),
        _ => None,
    })
}

/// The holes of `t` against a variant's payload: the first that names no
/// position of it, as the refusal to report.
pub fn variant_hole_refusal(t: &DisplayTemplate, variant: &str, arity: usize) -> Option<Diag> {
    t.parts.iter().find_map(|p| match p {
        TemplatePart::Hole { key, span, .. } => {
            let ok = key.bytes().all(|c| c.is_ascii_digit()) && key.parse::<usize>().is_ok_and(|n| n < arity);
            (!ok).then(|| {
                Diag::parse(
                    *span,
                    if arity == 0 {
                        format!("`{{{key}}}`: the variant `{variant}` carries no payload, so its template has no hole")
                    } else {
                        format!(
                            "`{{{key}}}`: a variant's template names its payload by position, and `{variant}` carries \
                             {arity} ({})",
                            (0..arity).map(|n| format!("`{{{n}}}`")).collect::<Vec<_>>().join(", ")
                        )
                    },
                )
            })
        }
        _ => None,
    })
}

/// The expression a template renders: its parts joined with `+`, each hole
/// `to_string(<place>)` or `__fmt(<place>, "spec")`, where `place` gives a
/// hole's value as source.
fn render_source(t: &DisplayTemplate, place: &dyn Fn(&str) -> String) -> String {
    // A template that opens with a hole starts from `""`, so the result is
    // a String of its own even when the hole is one (`{v}` of a String
    // field); one that opens with text concatenates as the f-string does.
    let mut pieces: Vec<String> = match t.parts.first() {
        Some(TemplatePart::Lit(_)) => Vec::new(),
        _ => vec!["\"\"".to_string()],
    };
    for p in &t.parts {
        match p {
            TemplatePart::Lit(s) => pieces.push(format!("\"{}\"", hale_str(s))),
            TemplatePart::Hole { key, spec: None, .. } => pieces.push(format!("to_string({})", place(key))),
            TemplatePart::Hole { key, spec: Some(sp), .. } => {
                pieces.push(format!("{}({}, \"{}\")", crate::parser::FMT_BUILTIN, place(key), hale_str(sp)))
            }
        }
    }
    pieces.join(" + ")
}

/// The holes' spans in the order [`render_source`] writes their calls.
fn hole_spans(t: &DisplayTemplate) -> Vec<Span> {
    t.parts
        .iter()
        .filter_map(|p| match p {
            TemplatePart::Hole { span, .. } => Some(*span),
            _ => None,
        })
        .collect()
}

/// Place each hole's call, and everything in it, at the hole: the calls of
/// a generated body in source order, `to_string` or `__fmt` taking the next
/// span. A diagnostic about a hole then points into the template.
fn place_holes(items: &mut [TopDecl], template: Span, spans: &[Span]) {
    // Every expression first at the template, so a diagnostic about what
    // the rendering allocates points at the annotation, not into the
    // generated text.
    fn returns(b: &mut crate::ast::Block, template: Span) {
        for st in &mut b.stmts {
            match st {
                crate::ast::Stmt::Return(Some(e), sp) => {
                    crate::shift::set_expr_spans(e, template);
                    *sp = template;
                }
                crate::ast::Stmt::Match(m) => {
                    for arm in &mut m.arms {
                        if let crate::ast::MatchArmBody::Block(b) = &mut arm.body {
                            returns(b, template);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for d in items.iter_mut() {
        if let TopDecl::Fn(f) = d {
            returns(&mut f.body, template);
        }
    }
    let mut next = 0usize;
    rewrite_items(items, &mut |e: &mut Expr| {
        if let Expr::Call { callee, .. } = e {
            if let Expr::Ident(id) = callee.as_ref() {
                if (id.name == "to_string" || id.name == crate::parser::FMT_BUILTIN) && next < spans.len() {
                    let s = spans[next];
                    next += 1;
                    crate::shift::set_expr_spans(e, s);
                }
            }
        }
    });
}

struct Templated {
    program: usize,
    /// The record's template, or each variant's that has one.
    record: Option<DisplayTemplate>,
    variants: Vec<EnumVariant>,
}

/// Write each template's function into the program that declares its type,
/// and `T::display` where a program names it (`T::display(v)`, or the
/// function value `T::display`, through an import as well), rewriting each
/// use to name the function. Idempotent: a function a program already has
/// is not written again.
pub fn generate_displays_in(programs: &mut [&mut Program], renames: &[(Vec<String>, String)]) {
    let mut types: BTreeMap<String, Templated> = BTreeMap::new();
    for (i, p) in programs.iter().enumerate() {
        for d in flat_decls(&p.items) {
            let TopDecl::Type(td) = d else { continue };
            match &td.body {
                TypeDeclBody::Struct(_) if td.template.is_some() => {
                    types.entry(td.name.name.clone()).or_insert(Templated {
                        program: i,
                        record: td.template.clone(),
                        variants: Vec::new(),
                    });
                }
                TypeDeclBody::Enum(vs) if vs.iter().any(|v| v.template.is_some()) => {
                    types.entry(td.name.name.clone()).or_insert(Templated {
                        program: i,
                        record: None,
                        variants: vs.clone(),
                    });
                }
                _ => {}
            }
        }
    }
    if types.is_empty() {
        return;
    }
    // `T::display(v)` and `T::display` as a value.
    let mut wanted: HashSet<String> = HashSet::new();
    let target = |qn: &QualifiedName| -> Option<String> {
        let n = qn.segments.len();
        if n < 2 || qn.segments[n - 1].name != "display" {
            return None;
        }
        let name = if n == 2 {
            qn.segments[0].name.clone()
        } else {
            let head: Vec<&str> = qn.segments[..n - 1].iter().map(|s| s.name.as_str()).collect();
            renames.iter().find(|(k, _)| k.iter().map(String::as_str).eq(head.iter().copied()))?.1.clone()
        };
        let t = types.get(&name)?;
        // an enum that declares a variant `display` keeps meaning it
        (!t.variants.iter().any(|v| v.name.name == "display")).then_some(name)
    };
    for p in programs.iter_mut() {
        rewrite_items(&mut p.items, &mut |e: &mut Expr| match e {
            Expr::Call { callee, args, span, .. } if args.len() == 1 => {
                let Expr::Path(qn) = callee.as_ref() else { return };
                let Some(name) = target(qn) else { return };
                *callee = Box::new(Expr::Ident(Ident::new(display_fn(&name), *span)));
                wanted.insert(name);
            }
            Expr::Path(qn) => {
                let Some(name) = target(qn) else { return };
                *e = Expr::Ident(Ident::new(display_fn(&name), qn.span));
                wanted.insert(name);
            }
            _ => {}
        });
    }
    let mut existing: HashSet<String> = HashSet::new();
    for p in programs.iter() {
        for d in flat_decls(&p.items) {
            if let TopDecl::Fn(f) = d {
                existing.insert(f.name.name.clone());
            }
        }
    }
    // One source per (program, function), so each function's holes can be
    // placed after it parses.
    let mut generated: Vec<(usize, String, String, Span, Vec<Span>)> = Vec::new();
    for (name, t) in &types {
        if let Some(tpl) = &t.record {
            let f = record_fn(name);
            if existing.insert(f.clone()) {
                let body = render_source(tpl, &|k| format!("__v.{k}"));
                generated.push((t.program, f.clone(), format!("fn {f}(__v: {name}) -> String {{\n    return {body};\n}}\n"), tpl.span, hole_spans(tpl)));
            }
        }
        for v in &t.variants {
            let Some(tpl) = &v.template else { continue };
            let f = variant_fn(name, &v.name.name);
            if !existing.insert(f.clone()) {
                continue;
            }
            let binds: Vec<String> = (0..v.fields.len()).map(|i| format!("__p{i}")).collect();
            let pattern = if binds.is_empty() {
                format!("{name}::{}", v.name.name)
            } else {
                format!("{name}::{}({})", v.name.name, binds.join(", "))
            };
            let body = render_source(tpl, &|k| format!("__p{k}"));
            let src = format!(
                "fn {f}(__v: {name}) -> String {{\n    match __v {{\n        {pattern} -> {{ return {body}; }},\n        _ -> {{ return \"\"; }},\n    }}\n}}\n"
            );
            generated.push((t.program, f, src, tpl.span, hole_spans(tpl)));
        }
        if wanted.contains(name) {
            let f = display_fn(name);
            if existing.insert(f.clone()) {
                generated.push((t.program, f.clone(), format!("fn {f}(__v: {name}) -> String {{\n    return to_string(__v);\n}}\n"), Span::new(0, 0), Vec::new()));
            }
        }
    }
    for (i, f, src, tpl_span, spans) in generated {
        let base = next_synth_base(programs.iter().map(|p| &**p));
        match crate::parse_source_at(&src, base) {
            Ok(mut parsed) => {
                let _ = &f;
                if !spans.is_empty() || tpl_span != Span::new(0, 0) {
                    place_holes(&mut parsed.items, tpl_span, &spans);
                }
                programs[i].items.extend(parsed.items);
            }
            // The generator writes well-formed source; a parse failure is its
            // bug. A rendering then names nothing, which lowering reports.
            Err(ds) => eprintln!(
                "display: generated source did not parse: {}",
                ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(text: &str) -> Vec<TemplatePart> {
        parse_template(text, Span::new(0, 0), Span::new(0, text.len() + 2)).expect("parses").parts
    }

    #[test]
    fn literal_text_holes_and_braces() {
        let ps = parts("${dollars}.{cents:0>2} {{x}}");
        assert_eq!(ps.len(), 5);
        assert_eq!(ps[0], TemplatePart::Lit("$".into()));
        assert!(matches!(&ps[1], TemplatePart::Hole { key, spec: None, .. } if key == "dollars"));
        assert_eq!(ps[2], TemplatePart::Lit(".".into()));
        assert!(matches!(&ps[3], TemplatePart::Hole { key, spec: Some(s), .. } if key == "cents" && s == "0>2"));
        assert_eq!(ps[4], TemplatePart::Lit(" {x}".into()));
    }

    #[test]
    fn a_hole_holds_no_expression() {
        for bad in ["{a + b}", "{f(x)}", "{}", "{a.b}", "{", "}", "{x:q}"] {
            assert!(parse_template(bad, Span::new(0, 0), Span::new(0, 20)).is_err(), "{bad}");
        }
    }

    #[test]
    fn generated_names_display_as_written() {
        assert_eq!(generated_display("__display_Money").as_deref(), Some("Money::display"));
        assert_eq!(generated_display("__display_tpl_Money").as_deref(), Some("@display(Money)"));
        assert_eq!(generated_display("__display_tpl_Side__Buy").as_deref(), Some("@display(Side::Buy)"));
        assert_eq!(generated_display("__enum_name_Side"), None);
    }
}
