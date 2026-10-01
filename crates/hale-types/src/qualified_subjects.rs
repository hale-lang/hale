//! Qualified bus subjects, resolved once (F.40 phase 3, C2).
//!
//! A cross-seed topic is written `alias::Topic` in a `subscribe` or
//! `publish`, as the left side of a send (`alias::Topic <- v`), and as
//! a `bindings` entry. [`resolve_qualified_bus_subjects`] rewrites each
//! to the single-segment topic the imported declaration ends up at (its
//! mangled name), through the build's rename table and the stdlib's
//! path renames: the desugar sequence runs it before the check, so the
//! checker, the model and lowering all read the one resolution, and no
//! consumer holds a second lookup of its own.

use hale_syntax::ast::{
    BindingEntry, Block, BusMember, BusSubject, ElseBranch, Expr, Ident, IfStmt, LocusMember,
    MatchArmBody, Program, Stmt, TopDecl,
};

use crate::resolved::lookup_qualified_path as lookup;

/// A7 (G16): walk the program and resolve every
/// `BusSubject::QualifiedTopic(alias::Foo)` ref to the mangled
/// single-segment ident the imported topic decl ends up at.
/// Leaves the variant in place if the path doesn't resolve so a
/// downstream "unknown topic" diagnostic can cite the source path.
pub(crate) fn resolve_qualified_bus_subjects(
    program: &mut Program,
    import_renames: &[(Vec<String>, String)],
) {
    fn rewrite(subject: &mut BusSubject, import_renames: &[(Vec<String>, String)]) {
        if let BusSubject::QualifiedTopic(qn) = subject {
            let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
            if let Some(mangled) = lookup(&segs, import_renames) {
                let span = qn.span;
                *subject = BusSubject::Topic(Ident::new(mangled, span));
            }
        }
    }
    // GH #527 B6: `bindings { alias::Topic: unix(...); }` — the
    // entry keeps the joined path as its ident; resolve it here for
    // the build path exactly as the qualified bus subjects are.
    fn rewrite_binding(entry: &mut BindingEntry, import_renames: &[(Vec<String>, String)]) {
        if !entry.topic.name.contains("::") {
            return;
        }
        let segs: Vec<&str> = entry.topic.name.split("::").collect();
        if let Some(mangled) = lookup(&segs, import_renames) {
            entry.topic.name = mangled;
        }
    }
    fn rewrite_send_subject(e: &mut Expr, import_renames: &[(Vec<String>, String)]) {
        // `source::Heartbeat <- payload;` — Expr::Path multi-segment
        // resolves to a single-segment Ident with the mangled topic
        // name so the desugar's Stmt::Send rewriter (which only
        // looks at Expr::Ident) handles it uniformly with intra-
        // seed sends.
        if let Expr::Path(qn) = e {
            if qn.segments.len() > 1 {
                let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                if let Some(mangled) = lookup(&segs, import_renames) {
                    let span = qn.span;
                    *e = Expr::Ident(Ident::new(mangled, span));
                }
            }
        }
    }
    fn walk_if(i: &mut IfStmt, import_renames: &[(Vec<String>, String)]) {
        walk_block(&mut i.then_block, import_renames);
        if let Some(eb) = &mut i.else_block {
            match eb.as_mut() {
                ElseBranch::Else(b) => walk_block(b, import_renames),
                ElseBranch::ElseIf(nested) => walk_if(nested, import_renames),
            }
        }
    }
    fn walk_block(b: &mut Block, import_renames: &[(Vec<String>, String)]) {
        for s in &mut b.stmts {
            walk_stmt(s, import_renames);
        }
        // Tail expr can't be a Send (Send is statement-only).
        let _ = &b.tail;
    }
    fn walk_stmt(s: &mut Stmt, import_renames: &[(Vec<String>, String)]) {
        match s {
            Stmt::Send { subject, .. } => {
                rewrite_send_subject(subject, import_renames);
            }
            Stmt::If(i) => walk_if(i, import_renames),
            Stmt::Match(m) => {
                for arm in &mut m.arms {
                    if let MatchArmBody::Block(b) = &mut arm.body {
                        walk_block(b, import_renames);
                    }
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                walk_block(body, import_renames);
            }
            Stmt::Block(b) => walk_block(b, import_renames),
            _ => {}
        }
    }
    // GH #884: module nesting flattened — a qualified bus subject
    // written one brace deeper names the same topic.
    hale_syntax::ast::for_each_decl_mut(&mut program.items, &mut |item| {
        if let TopDecl::Locus(l) = item {
            for m in &mut l.members {
                match m {
                    LocusMember::Bus(b) => {
                        for bm in &mut b.members {
                            match bm {
                                BusMember::Subscribe { subject, .. } => {
                                    rewrite(subject, import_renames);
                                }
                                BusMember::Publish { subject, .. } => {
                                    rewrite(subject, import_renames);
                                }
                            }
                        }
                    }
                    LocusMember::Lifecycle(lc) => {
                        walk_block(&mut lc.body, import_renames);
                    }
                    LocusMember::Mode(md) => {
                        walk_block(&mut md.body, import_renames);
                    }
                    LocusMember::Fn(fd) => {
                        walk_block(&mut fd.body, import_renames);
                    }
                    LocusMember::Bindings(bb) => {
                        for entry in &mut bb.entries {
                            rewrite_binding(entry, import_renames);
                        }
                    }
                    _ => {}
                }
            }
        } else if let TopDecl::Fn(fd) = item {
            walk_block(&mut fd.body, import_renames);
        }
    });
}
