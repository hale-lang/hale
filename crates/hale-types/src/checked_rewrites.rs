//! Rewrites the check decides, written into the program before any pass
//! after the check reads it.
//!
//! Some passes run before the check (the desugar sequence) and decide by
//! spelling what only the check's types can tell. Where the check finds
//! such a decision left open, it records what the program means
//! (`TypingRecord`'s columns), and the snapshot writes it here, then types
//! the program again, so every analysis and lowering read the program as
//! it means.
//!
//! GH #2069 (F.58): a call whose bare callee names a fn value a `.Variant`
//! arm binds, where the shadowing desugar (`crate::shadowed_callees`) did
//! not see the variant's enum (`match h.op { .Use(f) -> f(2), … }` with
//! another enum declaring `Use`), is the call through the binding,
//! `{ f }(2)`, as `Op::Use(f) -> f(2)` is written by the desugar.

use std::collections::BTreeSet;

use hale_syntax::ast::*;

/// Whether `programs` hold a `.Variant` pattern that binds a name: the
/// programs the check may have a binding call to record in.
pub fn any_dot_binding(programs: &[&Program]) -> bool {
    use crate::listener_window::{for_each_expression_root, visit_block, visit_expr, Root, Visited};
    fn binds(p: &Pattern) -> bool {
        match p {
            Pattern::DotVariant { args, .. } => !args.is_empty(),
            Pattern::Or(alts, _) => alts.iter().any(binds),
            _ => false,
        }
    }
    let mut found = false;
    let mut visit = |v: Visited<'_>| {
        let m = match v {
            Visited::Stmt(Stmt::Match(m)) => m,
            Visited::Expr(Expr::Match(m)) => m.as_ref(),
            _ => return,
        };
        found |= m.arms.iter().any(|a| binds(&a.pattern));
    };
    for p in programs {
        for_each_expression_root(&p.items, &mut |root| match root {
            Root::Block(b) => visit_block(b, &mut visit),
            Root::Expr(e) => visit_expr(e, &mut visit),
        });
    }
    found
}

/// Write each call `calls` records (by node id) as the call through its
/// callee's binding, `{ f }(args)`, the shadowing desugar's spelling.
/// Returns how many were written.
pub fn rewrite_binding_calls(programs: &mut [&mut Program], calls: &BTreeSet<u32>) -> usize {
    let mut n = 0;
    for p in programs.iter_mut() {
        hale_syntax::json_gen::rewrite_items(&mut p.items, &mut |e: &mut Expr| {
            let Expr::Call { callee, id, .. } = e else { return };
            if id.is_none() || !calls.contains(&id.0) {
                return;
            }
            let Expr::Ident(name) = callee.as_ref() else { return };
            let span = name.span;
            let read = std::mem::replace(callee.as_mut(), Expr::Literal(Literal::Nil, span));
            **callee = Expr::Block(Block { stmts: Vec::new(), tail: Some(Box::new(read)), span });
            n += 1;
        });
    }
    n
}
