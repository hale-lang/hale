//! GH #1530: the routes of `http::Rest` (spec/api.md § The REST transport).
//!
//! A surface row may say where it is served over `http::Rest`, `rpc
//! Orders::cancel at "DELETE /orders/{order}" status: 200;`. This module
//! reads that text, judges it (the form, the parameters against the
//! request, the routes of one surface against each other), and writes the
//! table the transport matches a request against.
//!
//! The table is the compiled matcher's data: one line per row, `METHOD|
//! /pattern/:param|Member|status|field:kind,…`, the most specific pattern
//! of a method first, so the transport's first match is the most specific
//! one. Two routes of a method are ambiguous when some path matches both
//! and neither is the more specific; they are refused at the check, so a
//! served table has one answer for every path.

use std::cmp::Ordering;

use hale_syntax::ast::{PrimType, RpcRoute, TypeExpr};

use crate::topic_identity::{Shapes, TypeClass};

/// The methods a route may name.
pub const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// The most path parameters a route holds (the matcher's capture bound).
pub const MAX_PARAMS: usize = 8;

/// One segment of a route's path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Lit(String),
    Param(String),
}

/// A route, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub method: String,
    pub segs: Vec<Seg>,
    pub status: u16,
}

impl Parsed {
    /// The pattern the matcher reads: `/orders/:order`.
    pub fn pattern(&self) -> String {
        if self.segs.is_empty() {
            return "/".to_string();
        }
        self.segs
            .iter()
            .map(|s| match s {
                Seg::Lit(l) => format!("/{l}"),
                Seg::Param(p) => format!("/:{p}"),
            })
            .collect()
    }

    /// The route as a reader sees it, parameters unnamed: `GET /orders/{}`.
    pub fn shape(&self) -> String {
        let path: String = if self.segs.is_empty() {
            "/".to_string()
        } else {
            self.segs
                .iter()
                .map(|s| match s {
                    Seg::Lit(l) => format!("/{l}"),
                    Seg::Param(_) => "/{}".to_string(),
                })
                .collect()
        };
        format!("{} {path}", self.method)
    }

    pub fn params(&self) -> impl Iterator<Item = &str> {
        self.segs.iter().filter_map(|s| match s {
            Seg::Param(p) => Some(p.as_str()),
            Seg::Lit(_) => None,
        })
    }
}

/// Read `METHOD /path/{param}` and the row's status.
pub fn parse(r: &RpcRoute) -> Result<Parsed, String> {
    let (method, path) = r.text.split_once(' ').ok_or_else(|| {
        format!(
            "`{}` is no route: a route is `METHOD /path`, as `GET /orders/{{order}}`",
            r.text
        )
    })?;
    if !METHODS.contains(&method) {
        return Err(format!(
            "`{method}` is no method a route takes: one of {}",
            METHODS.join(", ")
        ));
    }
    if !path.starts_with('/') {
        return Err(format!("the path `{path}` does not start with `/`"));
    }
    if path.contains(['?', '#', '|', ' ', '\t', '\n']) {
        return Err(format!("the path `{path}` holds a `?`, `#`, `|` or a space: the query is bound from the request's fields, not written in the route"));
    }
    let mut segs = Vec::new();
    let body = &path[1..];
    if !body.is_empty() {
        let parts: Vec<&str> = body.split('/').collect();
        for (k, part) in parts.iter().enumerate() {
            if part.is_empty() {
                if k + 1 == parts.len() {
                    return Err(format!(
                        "the path `{path}` ends in `/`: a route has no trailing slash"
                    ));
                }
                return Err(format!("the path `{path}` holds an empty segment"));
            }
            if let Some(inner) = part.strip_prefix('{') {
                let name = inner.strip_suffix('}').ok_or_else(|| {
                    format!("the segment `{part}` of `{path}` opens a `{{` it does not close: a parameter is a whole segment, `{{name}}`")
                })?;
                if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err(format!("the parameter `{part}` of `{path}` is named by a field: `{{name}}` with letters, digits and `_`"));
                }
                if segs.iter().any(|s| matches!(s, Seg::Param(p) if p == name)) {
                    return Err(format!("the parameter `{{{name}}}` is in `{path}` twice"));
                }
                segs.push(Seg::Param(name.to_string()));
            } else {
                if part.contains(['{', '}']) || part.starts_with(':') {
                    return Err(format!("the segment `{part}` of `{path}` is neither a literal nor a whole `{{name}}` parameter"));
                }
                segs.push(Seg::Lit(part.to_string()));
            }
        }
    }
    if path == "/.description" || path.starts_with("/.description/") {
        return Err(
            "`/.description` is the description's, which every `http` transport serves".to_string(),
        );
    }
    let parsed = Parsed {
        method: method.to_string(),
        segs,
        status: 200,
    };
    if parsed.params().count() > MAX_PARAMS {
        return Err(format!("a route holds at most {MAX_PARAMS} parameters"));
    }
    let status = match r.status {
        None => 200,
        Some(n) if (200..=299).contains(&n) => n as u16,
        Some(n) => {
            return Err(format!(
                "`status: {n}` is no success status: a route's is 2xx"
            ))
        }
    };
    Ok(Parsed { status, ..parsed })
}

/// How two routes meet: [`Meet::Apart`] when no path matches both (another
/// method, another length, another literal), [`Meet::Same`] when they match
/// the same paths, [`Meet::AFirst`] or [`Meet::BFirst`] when one is the more
/// specific (a literal where the other has a parameter, and nowhere less),
/// and [`Meet::Ambiguous`] when they overlap and neither is.
pub fn meet(a: &Parsed, b: &Parsed) -> Meet {
    if a.method != b.method || a.segs.len() != b.segs.len() {
        return Meet::Apart;
    }
    let mut a_literal = false;
    let mut b_literal = false;
    for (x, y) in a.segs.iter().zip(&b.segs) {
        match (x, y) {
            (Seg::Lit(p), Seg::Lit(q)) if p != q => return Meet::Apart,
            (Seg::Lit(_), Seg::Param(_)) => a_literal = true,
            (Seg::Param(_), Seg::Lit(_)) => b_literal = true,
            _ => {}
        }
    }
    match (a_literal, b_literal) {
        (false, false) => Meet::Same,
        (true, false) => Meet::AFirst,
        (false, true) => Meet::BFirst,
        (true, true) => Meet::Ambiguous,
    }
}

/// See [`meet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meet {
    Apart,
    Same,
    AFirst,
    BFirst,
    Ambiguous,
}

/// The order the table is written in: by method, then segment by segment a
/// literal before a parameter, then as bytes.
pub fn order(a: &Parsed, b: &Parsed) -> Ordering {
    let rank = |s: &Seg| match s {
        Seg::Lit(_) => 0,
        Seg::Param(_) => 1,
    };
    for (x, y) in a.segs.iter().zip(&b.segs) {
        let o = rank(x).cmp(&rank(y));
        if o != Ordering::Equal {
            return o;
        }
    }
    a.segs
        .len()
        .cmp(&b.segs.len())
        .then_with(|| a.pattern().cmp(&b.pattern()))
}

/// What a request field is on the wire as a route binds it: `i` an
/// integer, `f` a number, `b` a bool, `s` a string (text, a time, an enum's
/// name); `None` for a field the path and the query cannot carry.
pub fn field_kind(shapes: &Shapes<'_>, te: &TypeExpr) -> Option<char> {
    let prim = |p: PrimType| match p {
        PrimType::Int | PrimType::Duration => Some('i'),
        PrimType::Float => Some('f'),
        PrimType::Bool => Some('b'),
        PrimType::String | PrimType::Time => Some('s'),
        _ => None,
    };
    let d = shapes.dealias(te);
    if let TypeExpr::Primitive(p, _) = &d {
        return prim(*p);
    }
    match shapes.classify(&d) {
        TypeClass::Prim(p) => prim(p),
        TypeClass::Named { base, unit, .. }
            if !unit.as_deref().is_some_and(|u| u.contains(" point")) =>
        {
            prim(base)
        }
        TypeClass::Enum { variants, .. } if variants.iter().all(|v| v.fields.is_empty()) => {
            Some('s')
        }
        _ => None,
    }
}

/// The scalar fields of a request record, by name and kind; none for a
/// request that is no record.
pub fn fields(shapes: &Shapes<'_>, request: Option<&TypeExpr>) -> Vec<(String, char)> {
    let Some(te) = request else { return Vec::new() };
    let TypeClass::Struct { fields, .. } = shapes.classify(&shapes.dealias(te)) else {
        return Vec::new();
    };
    fields
        .iter()
        .filter_map(|f| field_kind(shapes, &f.ty).map(|k| (f.name.name.clone(), k)))
        .collect()
}

/// The scalar fields of a request, `name:kind` comma separated.
pub fn fields_text(shapes: &Shapes<'_>, request: Option<&TypeExpr>) -> String {
    fields(shapes, request)
        .into_iter()
        .map(|(n, k)| format!("{n}:{k}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One row of the table the transport is written with.
pub struct Entry<'a> {
    pub member: &'a str,
    pub route: Option<&'a RpcRoute>,
    pub request: Option<&'a TypeExpr>,
}

/// The table: a row with a route at it, a row without one at `POST
/// /call/<member>`, where `http::Rpc` serves it. A route the check refuses
/// is left out.
pub fn table(shapes: &Shapes<'_>, entries: &[Entry<'_>]) -> String {
    let mut lines: Vec<(Parsed, String)> = Vec::new();
    for e in entries {
        let (parsed, fields) = match e.route {
            Some(r) => match parse(r) {
                Ok(p) => (p, fields_text(shapes, e.request)),
                Err(_) => continue,
            },
            None => (
                Parsed {
                    method: "POST".to_string(),
                    segs: vec![Seg::Lit("call".to_string()), Seg::Lit(e.member.to_string())],
                    status: 200,
                },
                String::new(),
            ),
        };
        let line = format!(
            "{}|{}|{}|{}|{}",
            parsed.method,
            parsed.pattern(),
            e.member,
            parsed.status,
            fields
        );
        lines.push((parsed, line));
    }
    lines.sort_by(|a, b| order(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)));
    lines
        .into_iter()
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::Span;

    fn r(text: &str, status: Option<i64>) -> RpcRoute {
        RpcRoute {
            text: text.to_string(),
            status,
            span: Span::new(0, 0),
        }
    }

    #[test]
    fn a_route_is_read_and_its_faults_named() {
        let p = parse(&r("GET /v1/models/{model}", None)).unwrap();
        assert_eq!(p.pattern(), "/v1/models/:model");
        assert_eq!(p.shape(), "GET /v1/models/{}");
        assert_eq!(p.status, 200);
        assert_eq!(parse(&r("POST /x", Some(201))).unwrap().status, 201);
        for bad in [
            "FETCH /x",
            "GET x",
            "GET /a//b",
            "GET /a/",
            "GET /a/{b",
            "GET /a/{b}/{b}",
            "GET /a?b=1",
            "GET /.description",
            "GET /:a",
        ] {
            assert!(parse(&r(bad, None)).is_err(), "{bad}");
        }
        assert!(parse(&r("GET /x", Some(404))).is_err());
    }

    #[test]
    fn routes_meet_by_specificity_or_not_at_all() {
        let p = |t: &str| parse(&r(t, None)).unwrap();
        assert_eq!(meet(&p("GET /u/summary"), &p("GET /u/{id}")), Meet::AFirst);
        assert_eq!(meet(&p("GET /u/{id}"), &p("GET /u/summary")), Meet::BFirst);
        assert_eq!(meet(&p("GET /u/{a}"), &p("GET /u/{b}")), Meet::Same);
        assert_eq!(
            meet(&p("GET /u/{a}/c"), &p("GET /u/b/{d}")),
            Meet::Ambiguous
        );
        assert_eq!(meet(&p("GET /u/{a}"), &p("POST /u/{a}")), Meet::Apart);
        assert_eq!(meet(&p("GET /u/{a}"), &p("GET /u/{a}/x")), Meet::Apart);
    }
}
