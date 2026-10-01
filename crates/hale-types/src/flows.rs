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
//!
//! A clause whose type mentions its owner's type parameters
//! (`locus Manager<T> { release(c: T) { } }`) names no locus by itself:
//! which one it names is decided by each specialization lowering
//! creates. Such a clause carries its template ([`FlowClause::template`]:
//! the owner's identity, its parameters in order, the type as written),
//! and [`FlowRows::specialize`] answers for a specialization by applying
//! the consumer's own substitution to it and resolving the result the
//! way a concrete clause's type is resolved. The flow facts then cover
//! the concrete loci lowering creates, not only the ones written out
//! (outside review of #1295, finding 1).

use std::collections::BTreeMap;

use hale_syntax::ast::{LifecycleKind, LocusDecl, LocusMember, NodeId, Program, TopDecl, TypeExpr};
use hale_syntax::Span;

use crate::handler_routing::{child_locus_name, ChildRef, DeclaredNames};

/// One `release(c: T)` clause: the locus declaring it, the parameter's
/// name, where it is, and the locus its type denotes as lowering names
/// it (`None` when it denotes no declared locus, or when it is a
/// template's clause). Two instantiations of one generic share the
/// written key and resolve apart.
pub struct FlowClause {
    pub owner: String,
    pub param: String,
    pub span: Span,
    pub locus: Option<String>,
    /// Set when the clause's type mentions one of its owner's type
    /// parameters: the locus it names is the specialization's to decide.
    pub template: Option<TemplateClause>,
}

/// A generic owner's clause, before substitution.
pub struct TemplateClause {
    /// The owner's identity: a specialization is a clone of its
    /// template's declaration and keeps it.
    pub owner_id: NodeId,
    /// The owner's type parameters, in declaration order: a
    /// specialization's arguments are positional.
    pub generics: Vec<String>,
    /// The parameter's type as written (`T`, `Box<T>`).
    pub ty: TypeExpr,
}

/// A flow type and every clause that makes it one, in source order.
pub struct Flow {
    /// The child type as written (its path joined by `::`).
    pub child: String,
    pub clauses: Vec<FlowClause>,
}

/// The `flows` family's rows, with what a template clause is resolved
/// against once a specialization substitutes it: the declared loci and
/// aliases, and the bundle's import renames. Derefs to the rows.
pub struct FlowRows {
    flows: Vec<Flow>,
    declared: DeclaredNames,
    renames: Vec<(Vec<String>, String)>,
}

impl std::ops::Deref for FlowRows {
    type Target = [Flow];
    fn deref(&self) -> &[Flow] {
        &self.flows
    }
}

impl FlowRows {
    /// The loci a specialization of `template` makes flows: each of the
    /// template's clauses, its type passed through `substitute` (the
    /// consumer's substitution of the template's parameters by the
    /// specialization's arguments) and resolved the way a concrete
    /// clause's type is. The template is matched by its identity, or by
    /// its name and span when the program was never minted.
    pub fn specialize(&self, template: &LocusDecl, substitute: impl Fn(&TypeExpr) -> TypeExpr) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in self.flows.iter().flat_map(|f| &f.clauses) {
            let Some(t) = &c.template else { continue };
            let same = if t.owner_id.is_none() || template.id.is_none() {
                c.owner == template.name.name
                    && template.span.start <= c.span.start
                    && c.span.end <= template.span.end
            } else {
                t.owner_id.0 == template.id.0
            };
            if !same {
                continue;
            }
            if let Some(name) = self.locus(&substitute(&t.ty)) {
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }

    /// The locus `ty` denotes as lowering names it, if it denotes one.
    fn locus(&self, ty: &TypeExpr) -> Option<String> {
        match child_locus_name(ty, &self.declared, &self.renames) {
            ChildRef::Locus(name) => Some(name),
            ChildRef::External(_) => None,
        }
    }
}

/// Whether a row makes `locus` (a locus as lowering names it) a flow.
/// A template's clause names no locus here; a consumer that creates
/// specializations asks [`FlowRows::specialize`] for theirs.
pub fn is_flow(flows: &[Flow], locus: &str) -> bool {
    flows.iter().flat_map(|f| &f.clauses).any(|c| c.locus.as_deref() == Some(locus))
}

/// Whether `ty` mentions one of `generics`: the parameter itself, or a
/// generic argument that does (`Box<T>`).
fn mentions_param(ty: &TypeExpr, generics: &[String]) -> bool {
    let TypeExpr::Named { path, generic_args, .. } = ty else { return false };
    (path.segments.len() == 1 && generic_args.is_empty() && generics.contains(&path.segments[0].name))
        || generic_args.iter().any(|a| mentions_param(a, generics))
}

fn walk(items: &[TopDecl], rows: &FlowRows, out: &mut BTreeMap<String, Vec<FlowClause>>) {
    for item in items {
        match item {
            TopDecl::Locus(l) => {
                let generics: Vec<String> = l.generics.iter().map(|g| g.name.name.clone()).collect();
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
                    // A parameter shadows a declared name: `T` in
                    // `Manager<T>` is the argument, never a locus `T`.
                    let template = mentions_param(&p.ty, &generics).then(|| TemplateClause {
                        owner_id: l.id,
                        generics: generics.clone(),
                        ty: p.ty.clone(),
                    });
                    let locus = match template {
                        Some(_) => None,
                        None => rows.locus(&p.ty),
                    };
                    out.entry(child).or_default().push(FlowClause {
                        owner: l.name.name.clone(),
                        param: p.name.name.clone(),
                        span: lc.span,
                        locus,
                        template,
                    });
                }
            }
            TopDecl::Module(m) => walk(&m.items, rows, out),
            _ => {}
        }
    }
}

/// Every flow type in the bundle, by name as written, with the clauses
/// that make it one ordered by where they sit, each with the locus its
/// type denotes under the bundle's `import_renames` (or, for a generic
/// owner's clause, the template a specialization resolves).
pub fn survey(programs: &[&Program], import_renames: &[(Vec<String>, String)]) -> FlowRows {
    let mut rows = FlowRows {
        flows: Vec::new(),
        declared: DeclaredNames::of(programs),
        renames: import_renames.to_vec(),
    };
    let mut by_child: BTreeMap<String, Vec<FlowClause>> = BTreeMap::new();
    for p in programs {
        walk(&p.items, &rows, &mut by_child);
    }
    rows.flows = by_child
        .into_iter()
        .map(|(child, mut clauses)| {
            clauses.sort_by_key(|c| c.span.start.as_usize());
            Flow { child, clauses }
        })
        .collect();
    rows
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

    /// A clause over its owner's type parameter is the template's: it
    /// names no locus (not even a declared locus spelled like the
    /// parameter), and a specialization names one through the
    /// substitution it is handed, the parameter's position deciding
    /// which argument; a parameter nested in a generic child substitutes
    /// inside it.
    #[test]
    fn a_template_clause_names_the_specialization_s_argument() {
        let src = "locus Worker { }\nlocus T { }\ntype Job = Worker;\n\
                   locus Cell<U> { }\n\
                   locus Manager<K, T> { accept(c: T) { } release(c: T) { } }\n\
                   locus Boxer<T> { release(c: Cell<T>) { } }\n\
                   fn main() { }\n";
        let p = hale_syntax::parse_source(src).expect("parse");
        let rows = survey(&[&p], &[]);
        assert!(!is_flow(&rows, "T") && !is_flow(&rows, "Worker"), "a template clause names no locus");
        let decl = |name: &str| {
            p.items
                .iter()
                .find_map(|i| match i {
                    TopDecl::Locus(l) if l.name.name == name => Some(l.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let named = |s: &str| TypeExpr::Named {
            path: hale_syntax::ast::QualifiedName {
                segments: vec![hale_syntax::ast::Ident::new(s, Span::new(0, 0))],
                span: Span::new(0, 0),
            },
            generic_args: Vec::new(),
            span: Span::new(0, 0),
        };
        // the substitution a consumer applies: the template's parameters
        // by position
        fn subst(ty: &TypeExpr, by: &BTreeMap<String, TypeExpr>) -> TypeExpr {
            match ty {
                TypeExpr::Named { path, generic_args, .. }
                    if path.segments.len() == 1 && generic_args.is_empty() && by.contains_key(&path.segments[0].name) =>
                {
                    by[&path.segments[0].name].clone()
                }
                TypeExpr::Named { path, generic_args, span } => TypeExpr::Named {
                    path: path.clone(),
                    generic_args: generic_args.iter().map(|a| subst(a, by)).collect(),
                    span: *span,
                },
                other => other.clone(),
            }
        }
        let manager = decl("Manager");
        let by: BTreeMap<String, TypeExpr> =
            [("K".to_string(), named("Int")), ("T".to_string(), named("Job"))].into_iter().collect();
        assert_eq!(rows.specialize(&manager, |t| subst(t, &by)), vec!["Worker".to_string()]);
        let boxer = decl("Boxer");
        let by: BTreeMap<String, TypeExpr> = [("T".to_string(), named("Int"))].into_iter().collect();
        let cell_int = crate::mangle::mangle_generic_name("Cell", &[named("Int")]).unwrap();
        assert_eq!(rows.specialize(&boxer, |t| subst(t, &by)), vec![cell_int]);
        // a specialization of another template answers nothing
        assert!(rows.specialize(&decl("Cell"), |t| t.clone()).is_empty());
    }
}
