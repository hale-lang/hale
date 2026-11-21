//! GH #2068 (F.59): a receiver call on a text value, written as the path
//! call it means.
//!
//! For `x` of type `String` or `StringView`, `x.f(args)` is
//! `std::str::f(x, args)`; for a `Bytes`, a `BytesView` or a `BytesMut`,
//! `std::bytes::f(x, args)`; when that fn is public and takes the
//! receiver's type first. Only the check knows a receiver's type, so the
//! check decides: it types each such call as the path call it means and
//! records it by the call's node id (`TypingRecord::receiver_calls`).
//! The snapshot then rewrites each recorded call into that path call
//! ([`rewrite`]) and types the program again, so every pass after the
//! check (the effects, allocation and ownership analyses, the summaries,
//! lowering) reads the one call it would read had the program spelled the
//! path: a receiver call is classified exactly as the path call is,
//! because it is the path call.
//!
//! The rewritten call keeps its node id and its span, and its callee
//! spells the path at the method name's span, so a diagnostic about it
//! points where the name was written. Its receiver becomes its first
//! argument.

use std::collections::BTreeMap;

use hale_syntax::ast::*;

/// Whether `programs` hold a call the check may resolve as a receiver
/// call: a method call (`x.f(…)`, `x` not `self`) whose name is a public
/// fn of `std::str` or `std::bytes`. A program with none is never typed
/// to decide.
pub fn any_candidate(programs: &mut [&mut Program]) -> bool {
    let mut found = false;
    for p in programs.iter_mut() {
        hale_syntax::json_gen::rewrite_items(&mut p.items, &mut |e: &mut Expr| {
            if let Expr::Call { callee, .. } = e {
                if let Expr::Field { receiver, name, .. } = callee.as_ref() {
                    if !matches!(receiver.as_ref(), Expr::KwSelf(_))
                        && crate::stdlib_surface::is_receiver_fn_name(&name.name)
                    {
                        found = true;
                    }
                }
            }
        });
        if found {
            return true;
        }
    }
    false
}

/// Whether `programs` hold a receiver call that may be a text source
/// (`x.words()`, [`hale_syntax::str_sources::receiver_source_shape`]): a
/// `for` or a chain over one is left as written until the snapshot
/// settles the receiver calls, which then desugars it.
pub fn any_deferred_source(programs: &mut [&mut Program]) -> bool {
    let mut found = false;
    for p in programs.iter_mut() {
        hale_syntax::json_gen::rewrite_items(&mut p.items, &mut |e: &mut Expr| {
            if hale_syntax::str_sources::receiver_source_shape(e) {
                found = true;
            }
        });
        if found {
            return true;
        }
    }
    false
}

/// Rewrite each call `calls` records (by node id) into the path call it
/// means: `x.f(args)` becomes `std::str::f(x, args)`. Returns how many
/// were rewritten.
pub fn rewrite(programs: &mut [&mut Program], calls: &BTreeMap<u32, Vec<String>>) -> usize {
    let mut n = 0;
    for p in programs.iter_mut() {
        hale_syntax::json_gen::rewrite_items(&mut p.items, &mut |e: &mut Expr| {
            let Expr::Call { callee, args, id, .. } = e else { return };
            if id.is_none() {
                return;
            }
            let Some(path) = calls.get(&id.0) else { return };
            let Expr::Field { receiver, name, .. } = callee.as_mut() else { return };
            let at = name.span;
            let receiver = std::mem::replace(receiver.as_mut(), Expr::Literal(Literal::Nil, at));
            **callee = Expr::Path(QualifiedName {
                segments: path.iter().map(|s| Ident::new(s.clone(), at)).collect(),
                span: at,
            });
            args.insert(0, receiver);
            n += 1;
        });
    }
    n
}
