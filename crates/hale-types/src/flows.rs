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
//! a release hook's first parameter — and keys the type as written: a
//! one-segment path by its name (an imported type is one mangled segment
//! by then), a longer one (a stdlib path, `std::io::tcp::Stream`) by the
//! whole path, never by its last segment, which would name a local
//! `Stream` that is no flow.
//!
//! Each row also carries the locus the type denotes as lowering names it
//! ([`Flow::locus`], resolved once by `handler_routing::child_locus_name`:
//! through aliases, generic instantiations and qualified paths), and
//! lowering reads flow-ness from there: a locus is a flow when a row
//! names it.

use std::collections::BTreeMap;

use hale_syntax::ast::{LifecycleKind, LocusMember, Program, TopDecl, TypeExpr};
use hale_syntax::Span;

use crate::handler_routing::{child_locus_name, ChildRef, DeclaredNames};

/// One `release(c: T)` clause: the locus declaring it, the parameter's
/// name, where it is, and the locus its type denotes as lowering names
/// it (`None` when it denotes no declared locus). Two instantiations of
/// one generic share the written key and resolve apart.
pub struct FlowClause {
    pub owner: String,
    pub param: String,
    pub span: Span,
    pub locus: Option<String>,
}

/// A flow type and every clause that makes it one, in source order.
pub struct Flow {
    /// The child type as written (its path joined by `::`).
    pub child: String,
    pub clauses: Vec<FlowClause>,
}

/// Whether a row makes `locus` (a locus as lowering names it) a flow.
pub fn is_flow(flows: &[Flow], locus: &str) -> bool {
    flows.iter().flat_map(|f| &f.clauses).any(|c| c.locus.as_deref() == Some(locus))
}

struct Resolver<'a> {
    declared: DeclaredNames,
    renames: &'a [(Vec<String>, String)],
}

impl Resolver<'_> {
    fn locus(&self, ty: &TypeExpr) -> Option<String> {
        match child_locus_name(ty, &self.declared, self.renames) {
            ChildRef::Locus(name) => Some(name),
            ChildRef::External(_) => None,
        }
    }
}

fn walk(items: &[TopDecl], r: &Resolver<'_>, out: &mut BTreeMap<String, Vec<FlowClause>>) {
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
                        locus: r.locus(&p.ty),
                    });
                }
            }
            TopDecl::Module(m) => walk(&m.items, r, out),
            _ => {}
        }
    }
}

/// Every flow type in the bundle, by name as written, with the clauses
/// that make it one ordered by where they sit, each with the locus its
/// type denotes under the bundle's `import_renames`.
pub fn survey(programs: &[&Program], import_renames: &[(Vec<String>, String)]) -> Vec<Flow> {
    let r = Resolver { declared: DeclaredNames::of(programs), renames: import_renames };
    let mut by_child: BTreeMap<String, Vec<FlowClause>> = BTreeMap::new();
    for p in programs {
        walk(&p.items, &r, &mut by_child);
    }
    by_child
        .into_iter()
        .map(|(child, mut clauses)| {
            clauses.sort_by_key(|c| c.span.start.as_usize());
            Flow { child, clauses }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clause's child is the locus lowering names: an alias is
    /// followed to its target, so two spellings of one child are two
    /// written keys over one flow; a type that is no locus is no flow.
    #[test]
    fn a_clause_names_the_locus_lowering_names() {
        let src = "locus Worker { params { id: Int = 0; } }\n\
                   type Job = Worker;\n\
                   locus A { accept(c: Worker) { } release(c: Worker) { } }\n\
                   locus B { accept(c: Job) { } release(c: Job) { } }\n\
                   locus C { release(c: Nothing) { } }\n\
                   fn main() { }\n";
        let p = hale_syntax::parse_source(src).expect("parse");
        let flows = survey(&[&p], &[]);
        let keys: Vec<(&str, Vec<Option<&str>>)> = flows
            .iter()
            .map(|f| (f.child.as_str(), f.clauses.iter().map(|c| c.locus.as_deref()).collect()))
            .collect();
        assert_eq!(
            keys,
            vec![("Job", vec![Some("Worker")]), ("Nothing", vec![None]), ("Worker", vec![Some("Worker")])]
        );
        assert!(is_flow(&flows, "Worker"));
        assert!(!is_flow(&flows, "Job") && !is_flow(&flows, "Nothing") && !is_flow(&flows, "A"));
    }
}
