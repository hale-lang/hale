//! The `handler_routing` family (F.40 phase 1.4): which `on_failure`
//! handler a failing child reaches.
//!
//! A failing child is routed to its parent's `on_failure` by the
//! child's locus type alone, and the FIRST handler the parent declares
//! for that type is the one that runs (the checker refuses a second:
//! it could never run). [`handler_rows`] makes one row per handler,
//! keyed by (parent locus, ordinal), with the child type resolved
//! once, by [`child_locus_name`]; lowering, the checker and the model
//! read the same row instead of naming the child three ways (codegen's
//! lowered locus name, the checker's `Ty::display()`, the model's
//! joined path).
//!
//! A row also carries the handler's recovery ops, from one walk over
//! the whole body ([`recovery_ops`]): which restart ops it can invoke
//! and the bound each `restart(c) for N` states ([`StatedBound`]: a
//! literal, or the site of the expression that computes it), of which
//! the model's `retry_bound` is the last literal. Lowering reads the
//! same entries by the statement's span
//! ([`HandlerRouting::retry_bound_at`]), so the model's bound and the
//! one lowered are one fact. Restart-in-place
//! attribution is a question over those ops. Beside the rows, the
//! routing states per locus declaration whether a failure can originate
//! there ([`HandlerRouting::can_fail`]), which is where lowering emits
//! restart points.
//!
//! The row carries the handler's snapshot identity (a `SiteId`, looked
//! up in the snapshot the caller hands in) as a column, not as its key:
//! every entry point and `resolve_program` mint it, but a test that
//! builds a bundle without minting has none there. A reader holding a
//! declaration joins it to its row by that identity
//! ([`HandlerRow::is_row_of`]); the span is the fallback for the
//! unminted bundle alone, since two declarations may share one.
//!
//! The parent's identity is a column too (`parent_id`), and the rows
//! are indexed by it: a reader holding a locus declaration asks for its
//! rows by the declaration's id ([`HandlerRouting::handlers_of_decl`]).
//! A monomorph keeps its template's id. At synthesis, `specialize`
//! substitutes its child types through the same resolver and preserves
//! the handler sites. `handlers_of_instance` selects those concrete rows
//! by template identity and specialization name.

use std::collections::{BTreeMap, BTreeSet};

pub use hale_graph::ids::SiteId;
use hale_syntax::ast::{
    Block, ElseBranch, Expr, FailureDecl, IfStmt, Literal, LocusDecl, LocusMember, LValueSeg,
    MatchArmBody, NodeId, OrDisposition, PerspectiveMember, Program, RecoveryModifier,
    RecoveryOp, Stmt, TopDecl, TypeDeclBody, TypeExpr,
};
use hale_syntax::Span;

use crate::placement::SiteRef;
use crate::snapshot::Snapshot;

/// The child type a handler names, resolved.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChildRef {
    /// A locus this program declares, by its resolved name: the name
    /// lowering gives it (an alias followed, a generic instantiation
    /// mangled, a qualified path renamed).
    Locus(String),
    /// A type the program does not declare as a locus: the name as
    /// written, its path joined by `::`.
    External(String),
}

impl ChildRef {
    /// The name the row is routed by: the resolved locus name, or the
    /// written one.
    pub fn name(&self) -> &str {
        match self {
            ChildRef::Locus(n) | ChildRef::External(n) => n,
        }
    }
}

/// One `on_failure` handler.
#[derive(Debug, Clone)]
pub struct HandlerRow {
    /// The locus that declares the handler.
    pub parent: String,
    /// The parent declaration's snapshot identity (`None` when
    /// unminted).
    pub parent_id: Option<SiteId>,
    pub child: ChildRef,
    /// Source type retained for the producer's specialization query.
    child_ty: TypeExpr,
    /// The declaration of the locus `child` resolves to (a monomorph's
    /// template's), qualified by the store that minted it: a program's
    /// in the snapshot the rows were built against, a stdlib locus's in
    /// the analysis copy. `None` for an external child, and for an
    /// unminted one.
    pub child_decl: Option<crate::placement::SiteRef>,
    /// The child type as written, its path joined by `::`: what a
    /// reader who has no row for the resolved locus names it by.
    pub written: String,
    /// The error param's type, as written (its path joined by `::`).
    pub error_type: String,
    /// The handler's position among its parent's two-param handlers.
    pub ordinal: u32,
    /// The declaration's snapshot identity (`None` when unminted).
    pub id: Option<SiteId>,
    pub span: Span,
    /// The recovery ops the body can invoke, deduplicated, in source
    /// order.
    pub ops: Vec<RecoveryOp>,
    /// Every `for` bound the body writes, in source order.
    pub bounds: Vec<StatedBound>,
    /// `restart(c) for N`'s literal `N`, the last one written: the
    /// model's summary of [`HandlerRow::bounds`], derived from them.
    pub retry_bound: Option<i64>,
}

/// The bound a `for` modifier states on a recovery statement
/// (`restart(c) for N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryBound {
    /// An integer literal: the bound is known before the program runs.
    Const(i64),
    /// Any other expression: the bound is its value where the
    /// statement runs. The site is the expression's span; lowering
    /// lowers the expression written there, once, as the statement
    /// executes.
    Expr(Span),
}

/// One recovery statement's `for` bound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatedBound {
    /// The recovery statement's span: the key lowering reads it by.
    pub statement: Span,
    pub op: RecoveryOp,
    pub bound: RetryBound,
}

impl HandlerRow {
    /// Whether `decl` is the declaration this row was made from.
    ///
    /// Joined by identity: the row's site against the declaration's
    /// minted id, so two handlers that share a span (a synthetic AST
    /// with shared provenance, a desugar that stamps one span on
    /// several declarations) are still two rows, and a same-named locus
    /// in another file is not this one. A monomorph's members are its
    /// template's, ids included, so it joins its template's rows.
    ///
    /// The span is the fallback when either side is unminted: a bundle
    /// no entry point minted (the checker's own tests build one) has
    /// `NodeId::NONE` on every declaration and no site on any row.
    pub fn is_row_of(&self, decl: &FailureDecl) -> bool {
        match self.id {
            Some(site) if !decl.id.is_none() => site.index == decl.id.0,
            _ => self.span == decl.span,
        }
    }
}

/// Every handler of a program, with the routing index.
#[derive(Debug, Clone, Default)]
pub struct HandlerRouting {
    rows: Vec<HandlerRow>,
    /// parent → its rows' indices, in ordinal order.
    by_parent: BTreeMap<String, Vec<usize>>,
    /// The parent declaration's site index → its rows' indices, in
    /// ordinal order (minted rows only).
    by_parent_decl: BTreeMap<u32, Vec<usize>>,
    /// (parent, child name) → the first row's index: the handler that
    /// runs.
    first: BTreeMap<(String, String), usize>,
    /// Concrete rows, separate from the declaration-level snapshot rows.
    specialized: BTreeMap<(u32, String), Vec<HandlerRow>>,
    declared: DeclaredNames,
    renames: Vec<(Vec<String>, String)>,
    declaration_sites: BTreeMap<String, SiteRef>,
    /// The restart rows' failure column: the loci, by declared name, a
    /// failure can originate in ([`HandlerRouting::can_fail`]).
    failing: BTreeSet<String>,
    /// Every recovery statement's `for` bound, by the statement's span
    /// ([`HandlerRouting::retry_bound_at`]): the handlers' rows' and
    /// those written in any other body.
    bounds: BTreeMap<(u32, u32), StatedBound>,
}

impl HandlerRouting {
    pub fn rows(&self) -> &[HandlerRow] {
        &self.rows
    }

    /// The handler a failing child of type `child` reaches in `parent`:
    /// the first one `parent` declares for that type.
    pub fn route(&self, parent: &str, child: &str) -> Option<&HandlerRow> {
        self.first
            .get(&(parent.to_string(), child.to_string()))
            .map(|&i| &self.rows[i])
    }

    /// `parent`'s handlers, in ordinal order.
    pub fn handlers_of<'a>(
        &'a self,
        parent: &str,
    ) -> impl Iterator<Item = &'a HandlerRow> + 'a {
        self.by_parent
            .get(parent)
            .into_iter()
            .flatten()
            .map(|&i| &self.rows[i])
    }

    /// The rows of the locus declared at `decl`, in ordinal order: by
    /// the declaration's identity, so a monomorph (which keeps its
    /// template's id) reads its template's rows. Empty for an unminted
    /// declaration.
    pub fn handlers_of_decl<'a>(
        &'a self,
        decl: NodeId,
    ) -> impl Iterator<Item = &'a HandlerRow> + 'a {
        (!decl.is_none())
            .then(|| self.by_parent_decl.get(&decl.0))
            .flatten()
            .into_iter()
            .flatten()
            .map(|&i| &self.rows[i])
    }

    /// The handler a failing child of type `child` reaches in the locus
    /// declared at `decl`: the first one it declares for that type.
    pub fn route_decl(&self, decl: NodeId, child: &str) -> Option<&HandlerRow> {
        self.handlers_of_decl(decl).find(|r| r.child.name() == child)
    }

    /// Resolve a monomorph's child types using the same substitution
    /// as locus synthesis. Handler and parent sites remain the template's;
    /// the specialization name distinguishes instances sharing those sites.
    pub fn specialize(
        &mut self,
        template: &LocusDecl,
        name: &str,
        substitute: impl Fn(&TypeExpr) -> TypeExpr,
    ) {
        let rows = self.rows.iter().filter(|row| match row.parent_id {
            Some(site) if !template.id.is_none() => site.index == template.id.0,
            _ => row.parent == template.name.name
                && template.span.start <= row.span.start
                && row.span.end <= template.span.end,
        }).map(|row| {
            let mut row = row.clone();
            row.parent = name.to_string();
            row.child_ty = substitute(&row.child_ty);
            let resolved = resolve_locus_type(&row.child_ty, &self.declared, &self.renames);
            row.child_decl = resolved.as_ref()
                .and_then(|r| self.declaration_sites.get(&r.declaration).copied());
            row.child = resolved.map(|r| ChildRef::Locus(r.name))
                .unwrap_or_else(|| ChildRef::External(written_name(&row.child_ty)));
            row
        }).collect();
        self.specialized.insert((template.id.0, name.to_string()), rows);
    }

    /// A concrete locus's rows: a specialization's when registered,
    /// otherwise the declaration's. An empty specialization is authoritative.
    pub fn handlers_of_instance<'a>(
        &'a self,
        decl: NodeId,
        name: &str,
    ) -> impl Iterator<Item = &'a HandlerRow> + 'a {
        let specialized = self.specialized.get(&(decl.0, name.to_string()));
        specialized.into_iter().flatten().chain(
            self.handlers_of_decl(decl).filter(move |_| specialized.is_none())
        )
    }

    /// The first concrete handler for `child` in this specialization.
    pub fn route_instance(&self, decl: NodeId, name: &str, child: &str) -> Option<&HandlerRow> {
        self.handlers_of_instance(decl, name).find(|r| r.child.name() == child)
    }

    /// Whether a failure can originate in the locus declared as `locus`,
    /// so that it pays for restart points (`__restart_<L>`,
    /// `__resume_<L>`): it declares a closure, of any epoch (`inline`
    /// ones included, and every `violate` names one), or a
    /// `birth_check`. A column of the declaration, read by its declared
    /// name: a monomorph's name is no declaration's, so a generic
    /// template's specializations are not in it and get no restart
    /// points, as lowering's walk over the declarations gave them none
    /// (known open: `Cell<T>` declaring a closure fails, and its
    /// monomorphs cannot be restarted).
    pub fn can_fail(&self, locus: &str) -> bool {
        self.failing.contains(locus)
    }

    /// The `for` bound the recovery statement at `statement` states, if
    /// it writes one: lowering's bound, from the same entries the rows'
    /// `retry_bound` is derived from. A monomorph's body keeps its
    /// template's spans, so it reads its template's entries.
    pub fn retry_bound_at(&self, statement: Span) -> Option<RetryBound> {
        self.bounds.get(&(statement.start.0, statement.end.0)).map(|b| b.bound)
    }

    /// Whether some handler, in any parent, restarts a child of locus
    /// type `child` in place: such a child keeps a copy of the params
    /// it was built with.
    pub fn restarts_in_place(&self, child: &str) -> bool {
        self.rows.iter().chain(self.specialized.values().flatten()).any(|r| {
            matches!(&r.child, ChildRef::Locus(n) if n == child)
                && r.ops.contains(&RecoveryOp::RestartInPlace)
        })
    }

    fn push(&mut self, row: HandlerRow) {
        let i = self.rows.len();
        self.by_parent.entry(row.parent.clone()).or_default().push(i);
        if let Some(p) = row.parent_id {
            self.by_parent_decl.entry(p.index).or_default().push(i);
        }
        self.first
            .entry((row.parent.clone(), row.child.name().to_string()))
            .or_insert(i);
        self.rows.push(row);
    }
}

/// What [`child_locus_name`] resolves against: the loci a program
/// declares and its (non-generic) type aliases.
#[derive(Debug, Clone, Default)]
pub struct DeclaredNames {
    pub loci: BTreeSet<String>,
    pub aliases: BTreeMap<String, TypeExpr>,
    /// Each declared locus's declaration: the first a program
    /// declares, else the bundled stdlib analysis copy's.
    pub decls: BTreeMap<String, DeclAt>,
}

/// Where a locus a child type resolves to is declared, by the id the
/// store that minted it gave the declaration: the programs handed in
/// (the snapshot's), or the bundled stdlib's analysis copy, which is
/// minted alone and numbers its own sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclAt {
    Program(NodeId),
    Stdlib(NodeId),
}

impl DeclaredNames {
    /// The loci and aliases of `programs`, module-nested ones included,
    /// and the bundled stdlib's loci: every snapshot carries the
    /// stdlib (the checker registers its surface, lowering merges its
    /// declarations), so `std::bytes::BytesBuilder` names the same
    /// locus whether or not the program handed here is the merged one.
    pub fn of(programs: &[&Program]) -> DeclaredNames {
        let mut out = DeclaredNames::default();
        if let Some(std) = crate::stdlib_bodies::program() {
            for item in hale_syntax::ast::flat_decls(&std.items) {
                if let TopDecl::Locus(l) = item {
                    out.loci.insert(l.name.name.clone());
                    out.decls.entry(l.name.name.clone()).or_insert(DeclAt::Stdlib(l.id));
                }
            }
        }
        let mut own: BTreeSet<String> = BTreeSet::new();
        for p in programs {
            for item in hale_syntax::ast::flat_decls(&p.items) {
                match item {
                    TopDecl::Locus(l) => {
                        out.loci.insert(l.name.name.clone());
                        if own.insert(l.name.name.clone()) {
                            out.decls.insert(l.name.name.clone(), DeclAt::Program(l.id));
                        }
                    }
                    TopDecl::Type(t) if t.generics.is_empty() => {
                        if let TypeDeclBody::Alias(te) = &t.body {
                            out.aliases.insert(t.name.name.clone(), te.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }
}

/// The one resolver of a child type (an `on_failure`'s first param, an
/// `accept`'s param): the name lowering gives the locus it denotes.
///
/// - a single-segment name is itself; with generic arguments it is the
///   monomorph's name (`mangle_generic_name`, as codegen mangles a
///   generic locus), a locus when the template is one;
/// - a qualified path resolves through the stdlib's path renames, then
///   the build's cross-seed `import_renames` (the lookup the resolved
///   program's qualified bus subjects use);
/// - a type alias (`type A = B`) is followed to its target, as codegen
///   follows it; a cyclic chain stops where it repeats.
///
/// The result is `Locus` when `declared` has a locus of that name (a
/// monomorph when the template is one), else `External` with the name
/// as written. This is where the three former namings could disagree:
/// codegen resolved aliases, generics and qualified paths; the checker
/// resolved aliases, generics, stdlib paths and imports; the model
/// joined the written path and resolved nothing, so an alias, a
/// generic instantiation or a qualified path named a different child
/// there than the one lowering routed.
pub fn child_locus_name(
    te: &TypeExpr,
    declared: &DeclaredNames,
    import_renames: &[(Vec<String>, String)],
) -> ChildRef {
    child_locus(te, declared, import_renames).0
}

/// [`child_locus_name`]'s resolution, with the declaration the locus
/// is: its own, or a monomorph's template's (a monomorph keeps its
/// template's identity). `None` for an external child.
pub fn child_locus(
    te: &TypeExpr,
    declared: &DeclaredNames,
    import_renames: &[(Vec<String>, String)],
) -> (ChildRef, Option<DeclAt>) {
    match resolve_locus_type(te, declared, import_renames) {
        Some(r) => (ChildRef::Locus(r.name), r.at),
        None => (ChildRef::External(written_name(te)), None),
    }
}

/// One resolution, retaining both the lowered name and its declaring
/// template. Ownership uses the latter even for an unminted bundle;
/// neither consumer reconstructs a template from a mangled suffix.
pub(crate) struct ResolvedLocusType {
    pub name: String,
    pub declaration: String,
    pub at: Option<DeclAt>,
}

pub(crate) fn resolve_locus_type(
    te: &TypeExpr,
    declared: &DeclaredNames,
    import_renames: &[(Vec<String>, String)],
) -> Option<ResolvedLocusType> {
    let (name, declaration) = resolve(te, declared, import_renames, &mut Vec::new())?;
    let at = declared.decls.get(&declaration).copied();
    Some(ResolvedLocusType { name, declaration, at })
}

/// The locus name `te` denotes if it denotes one, with the name of the
/// declaration that locus is (a monomorph's template): `None` for a
/// type no declared locus answers to.
fn resolve(
    te: &TypeExpr,
    declared: &DeclaredNames,
    renames: &[(Vec<String>, String)],
    seen: &mut Vec<String>,
) -> Option<(String, String)> {
    let TypeExpr::Named { path, generic_args, .. } = te else {
        return None;
    };
    let name = if path.segments.len() == 1 {
        path.segments[0].name.clone()
    } else {
        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
        crate::resolved::lookup_qualified_path(&segs, renames)?
    };
    if !generic_args.is_empty() {
        if !declared.loci.contains(&name) {
            return None;
        }
        let mangled = crate::mangle::mangle_generic_name(&name, generic_args).ok()?;
        return Some((mangled, name));
    }
    match declared.aliases.get(&name) {
        Some(target) if !seen.contains(&name) => {
            seen.push(name);
            resolve(target, declared, renames, seen)
        }
        Some(_) => None,
        None => declared.loci.contains(&name).then(|| (name.clone(), name)),
    }
}

/// A type as written, for a row that names no locus: a named type's
/// path joined by `::`, a primitive by its name.
fn written_name(te: &TypeExpr) -> String {
    match te {
        TypeExpr::Named { path, .. } => path
            .segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("::"),
        TypeExpr::Primitive(p, _) => crate::ty::Ty::Prim(*p).display(),
        _ => "?".to_string(),
    }
}

/// Every `on_failure` handler of `programs` that takes the two params
/// (child, error) the signature rule requires. A handler with any other
/// arity makes no row: the checker refuses it, and it is not the one
/// that runs.
///
/// `programs` is the bundle, not one file of it: a child type resolves
/// against every locus the bundle declares, so a child declared in a
/// sibling file (the LSP's bundle holds one program per file) is that
/// locus, not an external type. Rows come in program order, then
/// declaration order. `snapshot` is the one `programs` were minted
/// into; each row's `id` is the handler's site in it.
pub fn handler_rows(
    programs: &[&Program],
    import_renames: &[(Vec<String>, String)],
    snapshot: &Snapshot,
) -> HandlerRouting {
    let declared = DeclaredNames::of(programs);
    let declaration_sites = declared.decls.iter().filter_map(|(name, at)| {
        declaration_site(*at, snapshot).map(|site| (name.clone(), site))
    }).collect();
    let mut routing = HandlerRouting {
        declared: declared.clone(),
        renames: import_renames.to_vec(),
        declaration_sites,
        ..HandlerRouting::default()
    };
    let items = programs.iter().flat_map(|p| hale_syntax::ast::flat_decls(&p.items));
    for item in items {
        let TopDecl::Locus(l) = item else { continue };
        let fails = l
            .members
            .iter()
            .any(|m| matches!(m, LocusMember::Closure(_) | LocusMember::BirthCheck(_)));
        if fails {
            routing.failing.insert(l.name.name.clone());
        }
        let mut ordinal: u32 = 0;
        for member in &l.members {
            let LocusMember::Failure(fd) = member else { continue };
            if fd.params.len() != 2 {
                continue;
            }
            let (ops, bounds) = recovery_ops(&fd.body);
            let (child, at) = child_locus(&fd.params[0].ty, &declared, import_renames);
            let child_decl = at.and_then(|at| declaration_site(at, snapshot));
            routing.push(HandlerRow {
                parent: l.name.name.clone(),
                parent_id: snapshot.site_id(l.id),
                child,
                child_ty: fd.params[0].ty.clone(),
                child_decl,
                written: written_name(&fd.params[0].ty),
                error_type: written_name(&fd.params[1].ty),
                ordinal,
                id: snapshot.site_id(fd.id),
                span: fd.span,
                ops,
                retry_bound: last_literal(&bounds),
                bounds,
            });
            ordinal += 1;
        }
    }
    // Lowering reads every recovery statement's bound by its span: the
    // handlers' (the rows' own, by the same walk), and one written in
    // any other body (a method that restarts a child it holds).
    let mut w = OpWalk::default();
    for item in programs.iter().flat_map(|p| hale_syntax::ast::flat_decls(&p.items)) {
        w.top_decl(item);
    }
    for b in w.bounds {
        routing.bounds.insert((b.statement.start.0, b.statement.end.0), b);
    }
    routing
}

/// The last literal bound in `bounds`: the rows' `retry_bound`.
fn last_literal(bounds: &[StatedBound]) -> Option<i64> {
    bounds.iter().rev().find_map(|b| match b.bound {
        RetryBound::Const(n) => Some(n),
        RetryBound::Expr(_) => None,
    })
}

fn declaration_site(at: DeclAt, snapshot: &Snapshot) -> Option<SiteRef> {
    match at {
        DeclAt::Program(n) => snapshot.site_id(n).map(SiteRef::user),
        DeclAt::Stdlib(n) => crate::stdlib_bodies::identities()
            .and_then(|ids| ids.site_id(n))
            .map(SiteRef::stdlib),
    }
}

/// The recovery ops a handler body can invoke, deduplicated in source
/// order, and every `for` bound it writes (`restart(c) for N`), in
/// source order. The walk reaches every statement, a block inside an
/// expression (`if` / `match` used as a value) included: a recovery op
/// missed here is a restart that re-evaluates nothing and restores
/// nothing.
pub fn recovery_ops(body: &Block) -> (Vec<RecoveryOp>, Vec<StatedBound>) {
    let mut w = OpWalk::default();
    w.block(body);
    (w.ops, w.bounds)
}

/// The name a recovery op is written with.
pub fn op_name(op: RecoveryOp) -> &'static str {
    match op {
        RecoveryOp::Restart => "restart",
        RecoveryOp::RestartInPlace => "restart_in_place",
        RecoveryOp::Quarantine => "quarantine",
        RecoveryOp::Reorganize => "reorganize",
        RecoveryOp::Bubble => "bubble",
    }
}

#[derive(Default)]
struct OpWalk {
    ops: Vec<RecoveryOp>,
    bounds: Vec<StatedBound>,
}

impl OpWalk {
    /// Every body and expression of a declaration a recovery statement
    /// can be lowered from.
    fn top_decl(&mut self, d: &TopDecl) {
        match d {
            TopDecl::Locus(l) => {
                for m in &l.members {
                    self.locus_member(m);
                }
            }
            TopDecl::Perspective(p) => {
                for m in &p.members {
                    match m {
                        PerspectiveMember::Params(pb) => self.params_block(pb),
                        PerspectiveMember::StableWhen(b) => self.block(b),
                        PerspectiveMember::Fn(f) => self.fn_decl(f),
                        PerspectiveMember::SerializeAs(_) | PerspectiveMember::Bus(_) => {}
                    }
                }
            }
            TopDecl::Fn(f) => self.fn_decl(f),
            TopDecl::Const(c) => self.expr(&c.value),
            _ => {}
        }
    }

    fn locus_member(&mut self, m: &LocusMember) {
        match m {
            LocusMember::Params(pb) => self.params_block(pb),
            LocusMember::Lifecycle(ld) => {
                self.params(&ld.params);
                self.block(&ld.body);
            }
            LocusMember::Mode(md) => {
                self.params(&md.params);
                self.block(&md.body);
            }
            LocusMember::Failure(fd) => {
                self.params(&fd.params);
                self.block(&fd.body);
            }
            LocusMember::Fn(f) => self.fn_decl(f),
            LocusMember::Const(c) => self.expr(&c.value),
            LocusMember::Closure(cd) => {
                if let Some(a) = &cd.assertion {
                    self.expr(&a.left);
                    self.expr(&a.right);
                    self.expr(&a.tolerance);
                }
            }
            LocusMember::BirthCheck(bc) => {
                self.expr(&bc.cond);
                if let Some(p) = &bc.payload {
                    self.expr(p);
                }
            }
            _ => {}
        }
    }

    fn fn_decl(&mut self, f: &hale_syntax::ast::FnDecl) {
        self.params(&f.params);
        self.block(&f.body);
    }

    fn params(&mut self, ps: &[hale_syntax::ast::Param]) {
        for p in ps.iter().filter_map(|p| p.default.as_ref()) {
            self.expr(p);
        }
    }

    fn params_block(&mut self, pb: &hale_syntax::ast::ParamsBlock) {
        for p in &pb.params {
            if let hale_syntax::ast::ParamInit::Value(e) = &p.init {
                self.expr(e);
            }
        }
    }

    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
    }

    fn if_stmt(&mut self, i: &IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(ei)) => self.if_stmt(ei),
            None => {}
        }
    }

    fn match_stmt(&mut self, m: &hale_syntax::ast::MatchStmt) {
        self.expr(&m.scrutinee);
        for arm in &m.arms {
            if let Some(g) = &arm.guard {
                self.expr(g);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b),
            }
        }
    }

    fn or_disposition(&mut self, d: &OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => self.expr(e),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Recovery { op, args, modifier, span } => {
                if !self.ops.contains(op) {
                    self.ops.push(*op);
                }
                for a in args {
                    self.expr(a);
                }
                let stated = |bound| StatedBound { statement: *span, op: *op, bound };
                match modifier {
                    Some(RecoveryModifier::For(Expr::Literal(Literal::Int(n), _))) => {
                        self.bounds.push(stated(RetryBound::Const(*n)));
                    }
                    Some(RecoveryModifier::For(e)) => {
                        self.bounds.push(stated(RetryBound::Expr(e.span())));
                        self.expr(e)
                    }
                    Some(RecoveryModifier::Until(e)) => self.expr(e),
                    None => {}
                }
            }
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => self.expr(value),
            Stmt::Assign { target, value, .. } => {
                for seg in &target.tail {
                    if let LValueSeg::Index(e) = seg {
                        self.expr(e);
                    }
                }
                self.expr(value);
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(m) => self.match_stmt(m),
            Stmt::For { iter, body, .. } => {
                self.expr(iter);
                self.block(body);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            Stmt::Fail { value, .. } => self.expr(value),
            Stmt::Block(b) => self.block(b),
            Stmt::Violate { payload, .. } => {
                if let Some(e) = payload {
                    self.expr(e);
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.or_disposition(d);
                }
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max);
                self.block(body);
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Tuple(items, _) | Expr::Array(items, _) => {
                for it in items {
                    self.expr(it);
                }
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.expr(&i.value);
                }
            }
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.expr(inner),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                self.or_disposition(disposition);
            }
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }
}
