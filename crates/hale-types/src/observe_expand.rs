//! GH #1592: the backends a program is observed through are params of its
//! main locus, recognized by their type, as a hub is (`hub_expand`).
//!
//! ```text
//! params {
//!     metrics: std::observe::Scrape = std::observe::Scrape { port: 9100 };
//!     events: std::observe::Otlp = std::observe::Otlp { };
//! }
//! placement { metrics: pinned; events: pinned; }
//! ```
//!
//! A backend is an ordinary locus: the author names it, places it, and it
//! is built, drained and torn down as any param of the main locus. What the
//! compiler adds is what a hub's param gets:
//!
//! - its number: the literal, at the standard library's path, carries
//!   `oid: N` (unique in the program, from 1), which is how the checker
//!   knows the param's default is the backend ([`is_built`]): a
//!   construction that supplies the param is refused, and a main locus
//!   holds at most one backend of a kind;
//! - the backends' source (`observe.hl`, `hale_stdlib::OBSERVE_SOURCE`),
//!   appended once to a program that holds one
//!   ([`crate::rpc_expand::inject_observe`]);
//! - the topology artifact's `observe` section, read off these params.
//!
//! Codegen reads the same params to decide whether the program observes:
//! only then are the registry and series calls lowered and the runtime's
//! observe support compiled in.
//!
//! The pass is idempotent: a literal already numbered is left alone.

use hale_syntax::ast::{
    flat_decls, Expr, Ident, Literal, LocusDecl, LocusMember, ParamInit, Program, QualifiedName, StructInit,
    TopDecl, TypeExpr,
};

/// The backend kind a literal's or a type's path names: `metrics` for
/// `std::observe::Scrape`, `events` for `std::observe::Otlp` (or their
/// declarations' names).
pub fn backend_kind(path: &QualifiedName) -> Option<&'static str> {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    match segs.as_slice() {
        ["std", "observe", "Scrape"] | ["__StdObserveScrape"] => Some("metrics"),
        ["std", "observe", "Otlp"] | ["__StdObserveOtlp"] => Some("events"),
        _ => None,
    }
}

/// A backend kind's type, as the author spells it.
pub fn backend_type(kind: &str) -> &'static str {
    if kind == "events" {
        "std::observe::Otlp"
    } else {
        "std::observe::Scrape"
    }
}

/// Whether a param's default is a backend this pass numbered.
pub fn is_built(init: &ParamInit) -> bool {
    matches!(init, ParamInit::Value(Expr::Struct { path, inits, .. })
        if backend_kind(path).is_some() && inits.iter().any(|i| i.name.name == "oid"))
}

/// One backend param of a main locus: the param's name, the kind, and its
/// default (the backend's literal), when it has one.
pub struct BackendParam<'a> {
    pub param: &'a str,
    pub kind: &'static str,
    pub literal: Option<&'a Expr>,
    pub span: hale_syntax::Span,
}

/// The backend params of `l`, in written order: a param whose literal
/// default, or whose declared type, is a backend.
pub fn backends(l: &LocusDecl) -> Vec<BackendParam<'_>> {
    let mut out = Vec::new();
    for m in &l.members {
        let LocusMember::Params(pb) = m else { continue };
        for p in &pb.params {
            let literal = match &p.init {
                ParamInit::Value(e @ Expr::Struct { path, .. }) if backend_kind(path).is_some() => Some(e),
                _ => None,
            };
            let kind = match (literal, &p.ty) {
                (Some(Expr::Struct { path, .. }), _) => backend_kind(path),
                (None, Some(TypeExpr::Named { path, .. })) => backend_kind(path),
                _ => None,
            };
            if let Some(kind) = kind {
                out.push(BackendParam { param: p.name.name.as_str(), kind, literal, span: p.name.span });
            }
        }
    }
    out
}

/// Whether any main locus of `items` holds a backend of `kind`.
pub fn holds_backend(items: &[TopDecl], kind: &str) -> bool {
    flat_decls(items)
        .any(|d| matches!(d, TopDecl::Locus(l) if l.is_main && backends(l).iter().any(|b| b.kind == kind)))
}

/// Number every main locus's backend params and append the backends'
/// source to a program that holds one.
pub fn expand(programs: &mut [&mut Program]) {
    let holds = programs.iter().any(|p| {
        flat_decls(&p.items).any(|d| matches!(d, TopDecl::Locus(l) if l.is_main && !backends(l).is_empty()))
    });
    if !holds {
        return;
    }
    crate::rpc_expand::inject_observe(programs);
    let mut next = 1i64;
    for p in programs.iter_mut() {
        number_items(&mut p.items, &mut next);
    }
}

fn number_items(items: &mut [TopDecl], next: &mut i64) {
    for item in items {
        match item {
            TopDecl::Module(m) => number_items(&mut m.items, next),
            TopDecl::Locus(l) if l.is_main => {
                for m in l.members.iter_mut() {
                    let LocusMember::Params(pb) = m else { continue };
                    for p in pb.params.iter_mut() {
                        if is_built(&p.init) {
                            *next += 1;
                            continue;
                        }
                        let ParamInit::Value(Expr::Struct { path, inits, span, .. }) = &mut p.init else { continue };
                        let Some(kind) = backend_kind(path) else { continue };
                        // the standard library's path, as a hub's literal is
                        // rewritten to `std::api::ws::Hub`
                        if path.segments.len() == 1 {
                            *path = QualifiedName {
                                segments: backend_type(kind)
                                    .split("::")
                                    .map(|s| Ident::new(s.to_string(), *span))
                                    .collect(),
                                span: *span,
                            };
                        }
                        inits.push(StructInit {
                            name: Ident::new("oid".to_string(), *span),
                            value: Expr::Literal(Literal::Int(*next), *span),
                            span: *span,
                        });
                        *next += 1;
                    }
                }
            }
            _ => {}
        }
    }
}
