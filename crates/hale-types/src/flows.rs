//! GH #736: which `release` clause makes a locus type a flow.
//!
//! A child type is a *flow* — its `run()` completing reclaims it — when
//! any locus anywhere in the program declares `release(c: T)`, whether or
//! not that locus is ever instantiated; otherwise it is a *resident*,
//! reclaimed only when its owner dissolves (spec/semantics.md, "Flow and
//! resident children"). The rule is type-wide on purpose, and it can
//! surprise: removing the release hook from one owner leaves a child
//! reclaimed at `run()`'s end because another declaration, perhaps in an
//! imported seed, still names its type.
//!
//! `hale check --flows` answers "why is this a flow" by naming every
//! clause responsible. It reads the declarations codegen classifies by —
//! a release hook's first parameter — and keys the type as codegen does
//! after the bundle's import renaming: a one-segment path by its name (an
//! imported type is one mangled segment by then), a longer one (a stdlib
//! path, `std::io::tcp::Stream`) by the whole path, never by its last
//! segment, which would name a local `Stream` that is no flow.

use std::collections::BTreeMap;

use hale_syntax::ast::{LifecycleKind, LocusMember, Program, TopDecl, TypeExpr};
use hale_syntax::Span;

/// One `release(c: T)` clause: the locus declaring it, the parameter's
/// name, and where it is.
pub struct FlowClause {
    pub owner: String,
    pub param: String,
    pub span: Span,
}

/// A flow type and every clause that makes it one, in source order.
pub struct Flow {
    pub child: String,
    pub clauses: Vec<FlowClause>,
}

fn walk(items: &[TopDecl], out: &mut BTreeMap<String, Vec<FlowClause>>) {
    for item in items {
        match item {
            TopDecl::Locus(l) => {
                for member in &l.members {
                    let LocusMember::Lifecycle(lc) = member else { continue };
                    if lc.kind != LifecycleKind::Release {
                        continue;
                    }
                    let Some(p) = lc.params.first() else { continue };
                    let TypeExpr::Named { path, .. } = &p.ty else { continue };
                    if path.segments.is_empty() {
                        continue;
                    }
                    let child = path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                    out.entry(child).or_default().push(FlowClause {
                        owner: l.name.name.clone(),
                        param: p.name.name.clone(),
                        span: lc.span,
                    });
                }
            }
            TopDecl::Module(m) => walk(&m.items, out),
            _ => {}
        }
    }
}

/// Every flow type in the bundle, by name, with the clauses that make it
/// one ordered by where they sit.
pub fn survey(programs: &[&Program]) -> Vec<Flow> {
    let mut by_child: BTreeMap<String, Vec<FlowClause>> = BTreeMap::new();
    for p in programs {
        walk(&p.items, &mut by_child);
    }
    by_child
        .into_iter()
        .map(|(child, mut clauses)| {
            clauses.sort_by_key(|c| c.span.start.as_usize());
            Flow { child, clauses }
        })
        .collect()
}
