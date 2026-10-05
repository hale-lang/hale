//! The role rows (F.40 phase 4, A4): what the program says about who
//! may reach its served surface — every `role` declaration, every
//! `@gated(role:)` site and the api entry's role source — over the
//! programs after the desugar sequence, with or without an `api:`
//! entry. Rows of the `api_surface` family: the snapshot demands them
//! once (`Snapshot::demand_role_rows`) and hands them to the check
//! (`CheckInputs::roles`), and the roles an environment maps are their
//! projection ([`RoleRows::declared_roles`]).
//!
//! What another family owns is not copied here: which handler
//! subscribes to which topic and which sites publish it are the bus
//! graph's, the bound topics the binding rows', a topic's wire key the
//! topic rows'.

use std::collections::BTreeMap;

use hale_syntax::ast::{
    BusMember, ContractDirection, ContractKind, ContractName, Expr, Ident, LocusMember, NodeId, PerspectiveMember,
    QualifiedName, TopDecl, TypeExpr,
};
use hale_syntax::Span;

use crate::entry::EntryRow;
use crate::Bundle;

/// One `role` declaration, as written. Two declarations of one name are
/// two rows: the vocabulary's own rule reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleDeclRow {
    /// The role's name, with its span.
    pub name: Ident,
    /// The roles it `includes`, each with its span, in the order written.
    pub includes: Vec<Ident>,
    /// The declaration's span.
    pub span: Span,
    /// Its site: the bundle program that holds it, and its item index at
    /// each depth (a module's contents under the module's index).
    pub program: String,
    pub path: Vec<usize>,
}

/// Where a `@gated(role:)` sits: every place the syntax takes one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateKind {
    /// A top-level `fn`.
    FreeFn,
    /// A locus's `fn`.
    Method,
    /// A perspective's `fn`.
    PerspectiveMethod,
    /// A `contract` member, with its direction (the parser takes a gate
    /// on an `expose` only).
    Contract(ContractDirection),
    /// A `publish` member of a `bus { }` block, by its identity: the bus
    /// graph's `PublishRow::id`.
    Publish(NodeId),
}

/// The locus declaration a gate sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateLocus {
    pub name: String,
    /// Its ordinal among the bundle's locus declarations in walk order:
    /// the index of its row in the bus graph's `decls`.
    pub decl: usize,
    /// An imported seed's locus.
    pub imported: bool,
}

/// One `@gated(role:)` site.
#[derive(Debug, Clone, PartialEq)]
pub struct GateRow {
    pub kind: GateKind,
    /// The declaration it sits on: the free fn, or the locus or
    /// perspective that holds the member.
    pub decl: String,
    /// The locus, for a gate on a locus's member.
    pub locus: Option<GateLocus>,
    /// The member's name: the fn's, the contract member's (empty for an
    /// inferred one), or the subject a `publish` names
    /// (`BusSubject::canonical`).
    pub member: String,
    /// The role it names, with its span.
    pub role: Ident,
    /// The gated declaration's span: the fn's, the contract member's or
    /// the `publish` member's.
    pub span: Span,
}

/// A `fn holds` as written.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldsFn {
    pub name: Ident,
    /// Its parameters' types as written.
    pub params: Vec<TypeExpr>,
    pub ret: Option<TypeExpr>,
    pub fallible: bool,
}

/// The locus a role source names, as the bundle declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceLocus {
    /// Its first `fn holds`, if it declares one.
    pub holds: Option<HoldsFn>,
}

/// The api entry's `roles:` clause: the membership source a gate asks.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleSource {
    /// The `api:` entry it is named in: the bundle's last.
    pub entry: Span,
    /// The clause's span.
    pub span: Span,
    /// The locus it names, as written: a locus literal's path, or the
    /// declared type of the `self.<param>` it reads (a param of a locus
    /// that carries an `api:` entry). `None` for any other expression,
    /// which is typed against `std::api::RoleSource` at the generated
    /// init.
    pub names: Option<String>,
    /// The bundle's locus of that name — the last declared, a generated
    /// `__Api` locus aside — when there is one.
    pub locus: Option<SourceLocus>,
}

/// The role rows of a bundle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoleRows {
    /// Every `role` declaration, in walk order.
    pub roles: Vec<RoleDeclRow>,
    /// Every `@gated(role:)` site, in walk order: a declaration's members
    /// in the order written, a `bus { }` block's `publish` members among
    /// them.
    pub gates: Vec<GateRow>,
    /// The last `roles:` clause of an `api:` entry, when one names a
    /// source.
    pub source: Option<RoleSource>,
    /// The entry row's root carries an `api:` entry: the binding is
    /// generated, and `owner` gates its description.
    pub served: bool,
}

impl RoleRows {
    /// The roles an environment maps: the declared names, sorted and
    /// de-duplicated, with `owner` when the program is served or
    /// declares any role, because that is when something is gated on it.
    pub fn declared_roles(&self) -> Vec<String> {
        self.vocabulary().into_iter().map(|(name, _)| name).collect()
    }

    /// The vocabulary the binding is generated with: each declared role
    /// once, by name, with the `includes` of its first declaration, and
    /// `owner` as [`RoleRows::declared_roles`] adds it, with no
    /// `includes` unless a declaration gives it some.
    pub fn vocabulary(&self) -> Vec<(String, Vec<String>)> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in &self.roles {
            out.entry(r.name.name.clone())
                .or_insert_with(|| r.includes.iter().map(|i| i.name.clone()).collect());
        }
        if self.served || !out.is_empty() {
            out.entry("owner".to_string()).or_default();
        }
        out.into_iter().collect()
    }
}

/// The role rows' producer: one walk over the bundle's declarations, in
/// the bundle's program order. `entry` is the bundle's entry row, whose
/// root is the `main locus` the api binding is generated into.
pub fn role_rows(bundle: &Bundle<'_>, entry: &EntryRow) -> RoleRows {
    struct Walk<'a> {
        rows: RoleRows,
        program: String,
        path: Vec<usize>,
        /// Locus declarations seen so far: the next one's ordinal.
        loci: usize,
        /// The source's inputs: the last `api:` entry's span, the last
        /// `roles:` clause, the params of every locus that carries an
        /// entry, and the loci a source may name.
        api_entry: Option<Span>,
        clause: Option<&'a hale_syntax::ast::ApiRoles>,
        params: Vec<(&'a str, &'a TypeExpr)>,
        by_name: BTreeMap<&'a str, &'a hale_syntax::ast::LocusDecl>,
    }

    fn gate(kind: GateKind, decl: &str, locus: Option<GateLocus>, member: String, role: &Ident, span: Span) -> GateRow {
        GateRow { kind, decl: decl.to_string(), locus, member, role: role.clone(), span }
    }

    fn walk<'a>(items: &'a [TopDecl], w: &mut Walk<'a>) {
        for (i, item) in items.iter().enumerate() {
            w.path.push(i);
            match item {
                TopDecl::Role(r) => w.rows.roles.push(RoleDeclRow {
                    name: r.name.clone(),
                    includes: r.includes.clone(),
                    span: r.span,
                    program: w.program.clone(),
                    path: w.path.clone(),
                }),
                TopDecl::Fn(f) => {
                    if let Some(g) = &f.gated {
                        let row = gate(GateKind::FreeFn, &f.name.name, None, f.name.name.clone(), g, f.span);
                        w.rows.gates.push(row);
                    }
                }
                TopDecl::Perspective(p) => {
                    for m in &p.members {
                        if let PerspectiveMember::Fn(f) = m {
                            if let Some(g) = &f.gated {
                                let kind = GateKind::PerspectiveMethod;
                                w.rows.gates.push(gate(kind, &p.name.name, None, f.name.name.clone(), g, f.span));
                            }
                        }
                    }
                }
                TopDecl::Locus(l) => {
                    let at = GateLocus { name: l.name.name.clone(), decl: w.loci, imported: l.imported };
                    w.loci += 1;
                    if !l.name.name.starts_with("__Api") {
                        w.by_name.insert(&l.name.name, l);
                    }
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(f) => {
                                if let Some(g) = &f.gated {
                                    let (kind, member) = (GateKind::Method, f.name.name.clone());
                                    w.rows.gates.push(gate(kind, &l.name.name, Some(at.clone()), member, g, f.span));
                                }
                            }
                            LocusMember::Contract(cb) => {
                                let ContractKind::Members(members) = &cb.kind else { continue };
                                for cm in members {
                                    let Some(g) = &cm.gated else { continue };
                                    let member = match &cm.name {
                                        ContractName::Named(n) => n.name.clone(),
                                        ContractName::Inferred => String::new(),
                                    };
                                    let kind = GateKind::Contract(cm.direction);
                                    w.rows.gates.push(gate(kind, &l.name.name, Some(at.clone()), member, g, cm.span));
                                }
                            }
                            LocusMember::Bus(bb) => {
                                for bm in &bb.members {
                                    let BusMember::Publish { subject, gated: Some(g), span, id, .. } = bm else {
                                        continue;
                                    };
                                    let (kind, member) = (GateKind::Publish(*id), subject.canonical().to_string());
                                    w.rows.gates.push(gate(kind, &l.name.name, Some(at.clone()), member, g, *span));
                                }
                            }
                            LocusMember::Bindings(bb) => {
                                let Some(api) = &bb.api else { continue };
                                w.api_entry = Some(api.span);
                                if let Some(r) = &api.roles {
                                    w.clause = Some(r);
                                }
                                for pm in &l.members {
                                    if let LocusMember::Params(pb) = pm {
                                        for prm in &pb.params {
                                            if let Some(t) = &prm.ty {
                                                w.params.push((&prm.name.name, t));
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, w),
                _ => {}
            }
            w.path.pop();
        }
    }

    let mut w = Walk {
        rows: RoleRows::default(),
        program: String::new(),
        path: Vec::new(),
        loci: 0,
        api_entry: None,
        clause: None,
        params: Vec::new(),
        by_name: BTreeMap::new(),
    };
    for (name, p) in &bundle.programs {
        w.program = name.clone();
        walk(&p.items, &mut w);
    }

    if let Some(clause) = w.clause {
        let named = |path: &QualifiedName| -> String {
            path.segments.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("::")
        };
        let names = match &clause.expr {
            Expr::Struct { path, .. } => Some(named(path)),
            Expr::Field { receiver, name, .. } if matches!(**receiver, Expr::KwSelf(_)) => {
                w.params.iter().find(|(n, _)| *n == name.name).and_then(|(_, t)| match t {
                    TypeExpr::Named { path, .. } => Some(named(path)),
                    _ => None,
                })
            }
            _ => None,
        };
        let locus = names.as_deref().and_then(|n| w.by_name.get(n)).map(|l| SourceLocus {
            holds: l.members.iter().find_map(|m| match m {
                LocusMember::Fn(f) if f.name.name == "holds" => Some(HoldsFn {
                    name: f.name.clone(),
                    params: f.params.iter().map(|p| p.ty.clone()).collect(),
                    ret: f.ret.clone(),
                    fallible: f.fallible.is_some(),
                }),
                _ => None,
            }),
        });
        w.rows.source = Some(RoleSource { entry: w.api_entry.unwrap_or(clause.span), span: clause.span, names, locus });
    }
    w.rows.served = entry.root().and_then(|m| m.decl(bundle)).is_some_and(|l| {
        l.members.iter().any(|m| matches!(m, LocusMember::Bindings(bb) if bb.api.is_some()))
    });
    w.rows
}
