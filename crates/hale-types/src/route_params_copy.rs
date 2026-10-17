//! A copy of a context's route params read for a capture the copy
//! cannot hold.
//!
//! `std::http::is_route(c, method, pattern)` writes the pattern's
//! `:name` captures into `c.params` when it matches. A `let p =
//! c.params;` taken before the match is a copy (a record is a value),
//! made while the captures were empty, so `std::http::path_param(p,
//! "id")` reads `""` in every branch, silently: the handler runs with an
//! empty id. `query_param` on the same copy is right (`params.qs` is
//! filled by `build_context`), which is how the copy gets written.
//!
//! The rule, per body (a fn, a method, a hook, a mode, a failure
//! handler): a `path_param` whose first argument is an immutable local
//! bound as `<ctx>.params`, where a call `is_route(<ctx>, …)` on the same
//! place stands between that `let` and the read in the source, is
//! warned at the read, with the spelling that works. A local bound after
//! the match, or one a later assignment refreshes (a `let mut`), is not
//! the shape and is left alone.
//!
//! Syntactic, over the program as written, and run once per check: it
//! costs a program nothing at run time.

use hale_syntax::ast::{flat_decls, Block, Expr, LocusMember, PerspectiveMember, Stmt, TopDecl};
use hale_syntax::error::Diag;
use hale_syntax::Span;

use crate::listener_window::{visit_block, Visited};
use crate::symbol::Bundle;

/// Every read of a stale route-params copy in `bundle`'s bodies.
pub(crate) fn stale_route_params_diags(bundle: &Bundle<'_>) -> Vec<Diag> {
    let mut out = Vec::new();
    for p in bundle.programs.values() {
        for d in flat_decls(&p.items) {
            match d {
                TopDecl::Fn(f) => body(&f.body, &mut out),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(f) => body(&f.body, &mut out),
                            LocusMember::Lifecycle(lc) => body(&lc.body, &mut out),
                            LocusMember::Mode(md) => body(&md.body, &mut out),
                            LocusMember::Failure(fd) => body(&fd.body, &mut out),
                            _ => {}
                        }
                    }
                }
                TopDecl::Perspective(pd) => {
                    for m in &pd.members {
                        if let PerspectiveMember::Fn(f) = m {
                            body(&f.body, &mut out);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// A place spelled as a local or `self` and its fields (`c`,
/// `self.ctx`), as text; `None` for anything else.
fn place(e: &Expr) -> Option<String> {
    match e {
        Expr::Ident(id) => Some(id.name.clone()),
        Expr::KwSelf(_) => Some("self".to_string()),
        Expr::Field { receiver, name, .. } => Some(format!("{}.{}", place(receiver)?, name.name)),
        _ => None,
    }
}

/// Is `callee` the stdlib's `std::http::<name>`, as written or as the
/// stdlib's own name for it (`__http_<name>`).
fn is_http_fn(callee: &Expr, name: &str) -> bool {
    match callee {
        Expr::Path(q) => {
            let n = q.segments.len();
            n >= 2 && q.segments[n - 1].name == name && q.segments[n - 2].name == "http"
        }
        Expr::Ident(id) => id.name.strip_prefix("__http_") == Some(name),
        _ => false,
    }
}

fn body(b: &Block, out: &mut Vec<Diag>) {
    // (local, end of its `let`, the context place it copied params of)
    let mut copies: Vec<(String, usize, String)> = Vec::new();
    // (start of the call, the context place it matched)
    let mut matches: Vec<(usize, String)> = Vec::new();
    // (local read, the argument's span)
    let mut reads: Vec<(String, Span)> = Vec::new();
    visit_block(b, &mut |v| match v {
        Visited::Stmt(Stmt::Let { is_mut: false, name, value: Expr::Field { receiver, name: field, .. }, span, .. })
            if field.name == "params" =>
        {
            if let Some(ctx) = place(receiver) {
                copies.push((name.name.clone(), span.end.as_usize(), ctx));
            }
        }
        Visited::Expr(Expr::Call { callee, args, span, .. }) => {
            if is_http_fn(callee, "is_route") {
                if let Some(ctx) = args.first().and_then(place) {
                    matches.push((span.start.as_usize(), ctx));
                }
            } else if is_http_fn(callee, "path_param") {
                if let Some(Expr::Ident(id)) = args.first() {
                    reads.push((id.name.clone(), id.span));
                }
            }
        }
        _ => {}
    });
    for (local, at) in reads {
        let read = at.start.as_usize();
        // the binding the read sees: the nearest `let` of the name before it
        let Some((_, bound, ctx)) =
            copies.iter().filter(|(n, end, _)| *n == local && *end <= read).max_by_key(|(_, end, _)| *end)
        else {
            continue;
        };
        if matches.iter().any(|(m, c)| c == ctx && *bound <= *m && *m < read) {
            out.push(Diag::warn(
                at,
                format!(
                    "`{local}` is a copy of `{ctx}.params` taken before `std::http::is_route({ctx}, …)` \
                     matched, and a match writes its `:name` captures into `{ctx}.params`, not into a \
                     copy: `path_param({local}, …)` is \"\" here. Read the capture from the context after \
                     the match: `std::http::path_param({ctx}.params, …)`"
                ),
            ));
        }
    }
}
