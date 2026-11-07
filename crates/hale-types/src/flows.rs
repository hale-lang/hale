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
//!
//! Beside the clauses, every locus declaration has a run row
//! ([`RunRow`]): whether its `run()` is *long-running* (a body of its
//! own) and whether it *never returns*. The checker asks both, of
//! different rules, so they are two columns, never one predicate
//! (F.40 phase 3, E2). A third column says whether the `run()` *watches
//! the drain*: it reads `draining`, itself or through a fn or method of
//! the program it calls (GH #1858). Such a locus takes the drain into
//! its own hands; a read anywhere else (a handler, a hook, a method
//! only another locus calls) does not.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    Block, ElseBranch, Expr, IfStmt, LValueSeg, LifecycleKind, Literal, LocusDecl, LocusMember,
    MatchArmBody, NodeId, ParamInit, Program, Stmt, TopDecl, TypeExpr, UnaryOp,
};
use hale_syntax::Span;

use crate::handler_routing::{child_locus_name, ChildRef, DeclaredNames};

/// One `release(c: T)` clause: the locus declaring it, the parameter's
/// name, where it is, and the locus its type denotes as lowering names
/// it (`None` when it denotes no declared locus, or when it is a
/// template's clause). Two instantiations of one generic share the
/// written key and resolve apart.
#[derive(Clone)]
pub struct FlowClause {
    pub owner: String,
    pub param: String,
    pub span: Span,
    pub locus: Option<String>,
    /// Set when the clause's type mentions one of its owner's type
    /// parameters: the locus it names is the specialization's to decide.
    pub template: Option<TemplateClause>,
    /// The type as written when it names no locus and no type parameter:
    /// an interface's name, or a name that resolves to nothing.
    pub interface: Option<String>,
    /// For a clause whose type names an interface (`release(c: I)`), the
    /// loci it admits: each locus the program declares that satisfies `I`
    /// ([`FlowRows::admit_interfaces`]), every one a flow as if the owner
    /// had declared `release(c: L)` for it. Empty for any other clause.
    pub admits: Vec<String>,
}

/// A generic owner's clause, before substitution.
#[derive(Clone)]
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
#[derive(Clone)]
pub struct Flow {
    /// The child type as written (its path joined by `::`).
    pub child: String,
    pub clauses: Vec<FlowClause>,
}

/// A locus's `run()`, as the two questions the checker asks of it
/// (F.40 phase 3, E2). They are different questions, asked by
/// different rules, so each is a column of its own
/// (spec/runtime.md § Typecheck enforcement):
///
/// - **long-running**: the `run()` body has a statement of its own. A
///   nested cooperative child runs its `run()` to completion before its
///   parent's begins, so any body delays the parent whether or not it
///   ever returns: the nested-long-running-child rule asks this.
/// - **never returns**: the `run()` body provably never returns (its
///   last statement is a `while` with no exit whose condition never
///   flips false). A cooperative pool runs each `run()` cell to
///   completion, so only such a body starves the cells after it: the
///   starvation and birth-order laws ask this.
///
/// A body that never returns is long-running; the converse does not
/// hold (`run() { std::time::sleep(1min); }` is long-running and returns).
#[derive(Clone)]
pub struct RunRow {
    /// The locus as declared, and where: a declaration is found by its
    /// name and span ([`FlowRows::run_of`]).
    pub locus: String,
    pub span: Span,
    pub long_running: bool,
    /// The terminal `while`, when the body never returns.
    pub never_returns: Option<Span>,
    /// GH #1858: the `run()` reads `draining` (on any receiver), in its
    /// own body or in a fn or method of the program it calls,
    /// transitively ([`run_reads_draining`]). Such a locus stops on the
    /// drain by its own code: lowering places no drain stops in its
    /// loops, a wait it begins during the drain is cut only until it has
    /// read the flag, and a parent does not wait for its `run()` before a
    /// tree's drain. A read in a handler, a hook or another method does
    /// not count. `false` with no `run()`.
    pub watches_drain: bool,
}

/// The `flows` family's rows, with what a template clause is resolved
/// against once a specialization substitutes it: the declared loci and
/// aliases, and the bundle's import renames. Derefs to the rows.
#[derive(Clone)]
pub struct FlowRows {
    flows: Vec<Flow>,
    /// One run row per locus declaration, a module's included, in
    /// declaration order.
    runs: Vec<RunRow>,
    declared: DeclaredNames,
    renames: Vec<(Vec<String>, String)>,
    /// The non-generic loci the surveyed programs declare, in declaration
    /// order: what an interface clause can admit.
    concrete: Vec<String>,
}

impl std::ops::Deref for FlowRows {
    type Target = [Flow];
    fn deref(&self) -> &[Flow] {
        &self.flows
    }
}

impl FlowRows {
    /// The run row of a declaration the rows were surveyed over.
    pub fn run_of(&self, decl: &LocusDecl) -> Option<&RunRow> {
        self.runs.iter().find(|r| r.locus == decl.name.name && r.span == decl.span)
    }

    /// Whether `decl`'s `run()` watches the drain (the [`RunRow`]'s
    /// column). A specialization lowering created (a generic's, under a
    /// name of its own) is the template's declaration substituted, so it
    /// is found by the template's span. A declaration with no row (one
    /// the rows were not surveyed over) answers `true`: lowering then
    /// leaves its loops to it, as before GH #1858, rather than stop a
    /// loop whose author may have written its own stop.
    pub fn run_watches_drain(&self, decl: &LocusDecl) -> bool {
        self.run_of(decl)
            .or_else(|| self.runs.iter().find(|r| r.span == decl.span))
            .map_or(true, |r| r.watches_drain)
    }

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

    /// These rows and the flows the merged stdlib's own `release`
    /// clauses make, surveyed over the merged program's stdlib tail:
    /// what lowering reads. A stdlib locus that owns per-connection
    /// children (`std::http::Server`'s, on an `async_io` pool) needs its
    /// children reclaimed when their `run()` completes, as a user owner's
    /// are; the checked programs' survey never sees the stdlib, so
    /// without this a stdlib `release` clause made nothing a flow and its
    /// children lived until the owner dissolved. The run rows stay the
    /// checked programs': only the flow rows are extended.
    pub fn with_stdlib(mut self, stdlib_items: &[TopDecl]) -> FlowRows {
        let mut by_child: BTreeMap<String, Vec<FlowClause>> = BTreeMap::new();
        let mut runs: Vec<RunRow> = Vec::new();
        let mut concrete: Vec<String> = Vec::new();
        walk(stdlib_items, &self, &DrainReads::empty(), &mut by_child, &mut runs, &mut concrete);
        for (child, mut clauses) in by_child {
            clauses.sort_by_key(|c| c.span.start.as_usize());
            self.flows.push(Flow { child, clauses });
        }
        self
    }

    /// These rows with every interface clause's admitted loci filled in
    /// (`check::interface_admits` over the surveyed programs' non-generic
    /// loci): `release(c: I)` makes each locus satisfying `I` a flow, as
    /// the ownership graph makes its owner's `accept(c: I)` accept each.
    /// A clause whose type names no interface admits nothing. The
    /// snapshot's rows are made with its scope; without one an interface
    /// clause stays empty.
    pub fn admit_interfaces(mut self, top: &crate::resolve::TopScope) -> FlowRows {
        let concrete = std::mem::take(&mut self.concrete);
        for c in self.flows.iter_mut().flat_map(|f| f.clauses.iter_mut()) {
            if c.locus.is_some() || c.template.is_some() {
                continue;
            }
            let Some(iface) = &c.interface else { continue };
            c.admits = crate::check::interface_admits(top, iface, concrete.iter().map(String::as_str))
                .into_iter()
                .collect();
        }
        self.concrete = concrete;
        self
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
    flows
        .iter()
        .flat_map(|f| &f.clauses)
        .any(|c| c.locus.as_deref() == Some(locus) || c.admits.iter().any(|a| a == locus))
}

/// Whether `ty` mentions one of `generics`: the parameter itself, or a
/// generic argument that does (`Box<T>`).
fn mentions_param(ty: &TypeExpr, generics: &[String]) -> bool {
    let TypeExpr::Named { path, generic_args, .. } = ty else { return false };
    (path.segments.len() == 1 && generic_args.is_empty() && generics.contains(&path.segments[0].name))
        || generic_args.iter().any(|a| mentions_param(a, generics))
}

fn walk<'a>(
    items: &'a [TopDecl],
    rows: &FlowRows,
    reads: &DrainReads<'a>,
    out: &mut BTreeMap<String, Vec<FlowClause>>,
    runs: &mut Vec<RunRow>,
    concrete: &mut Vec<String>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) => {
                if l.generics.is_empty() {
                    concrete.push(l.name.name.clone());
                }
                let run = l.members.iter().find_map(|m| match m {
                    LocusMember::Lifecycle(lc) if lc.kind == LifecycleKind::Run => Some(&lc.body),
                    _ => None,
                });
                runs.push(RunRow {
                    locus: l.name.name.clone(),
                    span: l.span,
                    long_running: run.is_some_and(|b| !b.stmts.is_empty()),
                    never_returns: run.and_then(|b| run_statically_nonreturning(b, l)),
                    watches_drain: run.is_some_and(|b| run_reads_draining(b, l, reads)),
                });
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
                    let interface = (locus.is_none() && template.is_none()).then(|| child.clone());
                    out.entry(child).or_default().push(FlowClause {
                        owner: l.name.name.clone(),
                        param: p.name.name.clone(),
                        span: lc.span,
                        locus,
                        template,
                        interface,
                        admits: Vec::new(),
                    });
                }
            }
            TopDecl::Module(m) => walk(&m.items, rows, reads, out, runs, concrete),
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
        runs: Vec::new(),
        declared: DeclaredNames::of(programs),
        renames: import_renames.to_vec(),
        concrete: Vec::new(),
    };
    let mut by_child: BTreeMap<String, Vec<FlowClause>> = BTreeMap::new();
    let mut runs: Vec<RunRow> = Vec::new();
    let mut concrete: Vec<String> = Vec::new();
    let reads = DrainReads::of(programs);
    for p in programs {
        walk(&p.items, &rows, &reads, &mut by_child, &mut runs, &mut concrete);
    }
    rows.runs = runs;
    rows.concrete = concrete;
    rows.flows = by_child
        .into_iter()
        .map(|(child, mut clauses)| {
            clauses.sort_by_key(|c| c.span.start.as_usize());
            Flow { child, clauses }
        })
        .collect();
    rows
}

// === the run()'s drain reads: the watches-drain column (GH #1858) =====

/// The program's free fns by name, a module's included: what a bare call
/// in a `run()` reaches.
pub type FreeFns<'a> = BTreeMap<&'a str, &'a hale_syntax::ast::FnDecl>;

/// The free fns of `programs`, a module's included; the first declaration
/// of a name wins.
pub fn free_fns_of<'a>(programs: &[&'a Program]) -> FreeFns<'a> {
    let mut free_fns = FreeFns::new();
    for program in programs {
        for item in hale_syntax::ast::flat_decls(&program.items) {
            if let TopDecl::Fn(f) = item {
                free_fns.entry(f.name.name.as_str()).or_insert(f);
            }
        }
    }
    free_fns
}

/// What a receiver expression names, as far as the declarations tell: a
/// locus by its declared name, something that is no locus (a primitive,
/// a declared record or enum, a fn with no value), or unresolved. The
/// walk is conservative in the safe direction on `Unknown`: a
/// `.draining` read on it counts, and a method call on it is followed.
/// (The safe direction: a `run()` wrongly taken to watch the drain is
/// not waited for, the behaviour before #1869; one wrongly taken not
/// to is awaited by a `main` with no `run()` and never ends.)
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Receiver {
    Locus(String),
    NotLocus,
    Unknown,
}

/// The declarations the watches-drain walk resolves receivers against:
/// every locus name, each locus's params fields and what each holds by
/// its declared type, each locus's methods, and the free fns by name.
pub struct DrainReads<'a> {
    loci: BTreeSet<&'a str>,
    /// `type X = Y;` declarations: a field or parameter typed `X` is
    /// classified by `Y`, transitively.
    aliases: BTreeMap<&'a str, &'a TypeExpr>,
    /// The other declared types (records, enums): no locus.
    records: BTreeSet<&'a str>,
    fields: BTreeMap<&'a str, BTreeMap<&'a str, Receiver>>,
    methods: BTreeMap<&'a str, BTreeMap<&'a str, &'a hale_syntax::ast::FnDecl>>,
    free_fns: FreeFns<'a>,
}

impl<'a> DrainReads<'a> {
    /// No declarations: every receiver unresolved.
    pub fn empty() -> Self {
        DrainReads {
            loci: BTreeSet::new(),
            aliases: BTreeMap::new(),
            records: BTreeSet::new(),
            fields: BTreeMap::new(),
            methods: BTreeMap::new(),
            free_fns: FreeFns::new(),
        }
    }

    /// The rows of `programs`' declarations, a module's included.
    pub fn of(programs: &[&'a Program]) -> Self {
        let mut rows = DrainReads::empty();
        rows.free_fns = free_fns_of(programs);
        for program in programs {
            for item in hale_syntax::ast::flat_decls(&program.items) {
                match item {
                    TopDecl::Locus(l) => {
                        rows.loci.insert(l.name.name.as_str());
                        let methods = rows.methods.entry(l.name.name.as_str()).or_default();
                        for m in &l.members {
                            if let LocusMember::Fn(f) = m {
                                methods.entry(f.name.name.as_str()).or_insert(f);
                            }
                        }
                    }
                    TopDecl::Type(t) => match &t.body {
                        hale_syntax::ast::TypeDeclBody::Alias(target) => {
                            rows.aliases.entry(t.name.name.as_str()).or_insert(target);
                        }
                        _ => {
                            rows.records.insert(t.name.name.as_str());
                        }
                    },
                    _ => {}
                }
            }
        }
        for program in programs {
            for item in hale_syntax::ast::flat_decls(&program.items) {
                let TopDecl::Locus(l) = item else { continue };
                let mut fields = BTreeMap::new();
                for m in &l.members {
                    let LocusMember::Params(params) = m else { continue };
                    for p in &params.params {
                        let kind = match (&p.ty, &p.init) {
                            (Some(ty), _) => rows.kind_of_type(ty),
                            (None, hale_syntax::ast::ParamInit::Value(e)) => rows.kind_of_expr(e, &Env::default()),
                            (None, hale_syntax::ast::ParamInit::Inferred) => Receiver::Unknown,
                        };
                        fields.insert(p.name.name.as_str(), kind);
                    }
                }
                rows.fields.insert(l.name.name.as_str(), fields);
            }
        }
        rows
    }

    /// The locus a type names: a declared locus by its bare name (an
    /// imported one by its last segment, when that is a declared name), a
    /// type alias by its target, transitively (`type WorkerRef = Worker;`
    /// hides the locus from nothing), else no locus (a primitive, a
    /// declared record or enum) or, for a name the rows cannot place,
    /// unresolved — never no locus, since an unresolved name may be this
    /// locus and the wrong answer there is a hang. A collection or
    /// projection of a locus holds that locus.
    fn kind_of_type(&self, ty: &TypeExpr) -> Receiver {
        self.kind_of_type_within(ty, 0)
    }

    fn kind_of_type_within(&self, ty: &TypeExpr, depth: usize) -> Receiver {
        match ty {
            TypeExpr::Named { path, .. } => self.kind_of_name_within(path, depth),
            TypeExpr::Primitive(..) => Receiver::NotLocus,
            TypeExpr::Projection { inner, .. } | TypeExpr::Array { elem: inner, .. } | TypeExpr::Bounded { elem: inner, .. } => {
                self.kind_of_type_within(inner, depth)
            }
            _ => Receiver::Unknown,
        }
    }

    fn kind_of_name(&self, path: &hale_syntax::ast::QualifiedName) -> Receiver {
        self.kind_of_name_within(path, 0)
    }

    fn kind_of_name_within(&self, path: &hale_syntax::ast::QualifiedName, depth: usize) -> Receiver {
        let Some(last) = path.segments.last() else { return Receiver::Unknown };
        let name = last.name.as_str();
        if self.loci.contains(name) {
            Receiver::Locus(last.name.clone())
        } else if let Some(target) = self.aliases.get(name) {
            // An alias chain is finite; a cycle (refused by the check) is
            // cut as unresolved.
            if depth > 32 {
                Receiver::Unknown
            } else {
                self.kind_of_type_within(target, depth + 1)
            }
        } else if path.segments.len() == 1 && self.records.contains(name) {
            Receiver::NotLocus
        } else {
            Receiver::Unknown
        }
    }

    /// The loci that MAY hold `this`, by declared types: a locus whose
    /// params declare a field whose type is `this` (or a collection or
    /// projection of it), or a field whose type is such a holder,
    /// transitively, or a field the rows cannot type (taken as one).
    fn holders_of(&self, this: &str) -> BTreeSet<String> {
        let mut holders: BTreeSet<String> = BTreeSet::new();
        loop {
            let before = holders.len();
            for (locus, fields) in &self.fields {
                if holders.contains(*locus) {
                    continue;
                }
                let holds = fields.values().any(|k| match k {
                    Receiver::Locus(l) => l == this || holders.contains(l),
                    Receiver::Unknown => true,
                    Receiver::NotLocus => false,
                });
                if holds {
                    holders.insert(locus.to_string());
                }
            }
            if holders.len() == before {
                return holders;
            }
        }
    }

    /// What `e` names as a receiver, under the body's bindings `env`.
    fn kind_of_expr(&self, e: &Expr, env: &Env) -> Receiver {
        match e {
            Expr::KwSelf(_) => env.this.clone().map_or(Receiver::Unknown, Receiver::Locus),
            Expr::Ident(id) => env.names.get(id.name.as_str()).cloned().unwrap_or(Receiver::Unknown),
            Expr::Struct { path, .. } => self.kind_of_name(path),
            Expr::Literal(..) => Receiver::NotLocus,
            Expr::Field { receiver, name, .. } => match self.kind_of_expr(receiver, env) {
                Receiver::Locus(l) => self
                    .fields
                    .get(l.as_str())
                    .and_then(|f| f.get(name.name.as_str()))
                    .cloned()
                    .unwrap_or(Receiver::Unknown),
                _ => Receiver::Unknown,
            },
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Ident(id) => match self.free_fns.get(id.name.as_str()) {
                    Some(f) => f.ret.as_ref().map_or(Receiver::NotLocus, |t| self.kind_of_type(t)),
                    None => Receiver::Unknown,
                },
                // A method's result: unresolved, so a read on it counts.
                _ => Receiver::Unknown,
            },
            _ => Receiver::Unknown,
        }
    }
}

/// A body's bindings: what `self` is (the locus whose body it is;
/// nothing in a free fn), and each name's receiver.
#[derive(Default, Clone, Debug)]
pub struct Env {
    this: Option<String>,
    names: BTreeMap<String, Receiver>,
}

/// One body's callees the walk follows, with the bindings each is
/// entered under: the locus whose method it is (`None` for a free fn),
/// its name, its declaration, and the receivers its parameters are bound
/// to (by the parameter's declared type where that names a locus or
/// rules one out, else by what the argument names). Followed: this
/// locus's own method called on `self`, an alias of it or an unresolved
/// receiver; another locus's method when the call hands it this locus
/// (or something unresolved) or when that locus's declared fields may
/// hold this one (`holders`); a bare call, as a free fn and as a method
/// of the body's locus. The flag `read` comes back set when the body
/// reads this locus's `draining` itself — on `self`, an alias, a
/// parameter, a declared field path of this locus's type, or a receiver
/// the rows cannot resolve — or hands this locus to a method the rows
/// cannot find (a builtin's, a stdlib locus's), which is taken as a
/// read. A `let` binds its name for the rest of its block only
/// (`Visited::Enter` / `Leave`), so an inner shadow never changes the
/// receiver of an expression outside it.
pub fn followed_callees<'a>(
    body: &'a Block,
    this: &str,
    holders: &BTreeSet<String>,
    env: Env,
    rows: &DrainReads<'a>,
    read: &mut bool,
) -> Vec<(Option<String>, &'a str, &'a hale_syntax::ast::FnDecl, Env)> {
    use crate::listener_window::{visit_block, Visited};
    let this_r = Receiver::Locus(this.to_string());
    let reaches_this = |r: &Receiver| matches!(r, Receiver::Unknown) || *r == this_r;
    // (the locus whose method, or None for a free fn; name; args; env at the call)
    let mut callees: Vec<(Option<String>, &'a str, &'a [Expr], Env)> = Vec::new();
    let mut scopes: Vec<Env> = vec![env];
    visit_block(body, &mut |v| {
        let env = scopes.last().expect("a block's bindings");
        match v {
            Visited::Enter(_) => {
                let inner = env.clone();
                scopes.push(inner);
            }
            Visited::Leave(_) => {
                scopes.pop();
            }
            Visited::Stmt(Stmt::Let { name, ty, value, .. }) => {
                let kind = match ty.as_ref().map(|t| rows.kind_of_type(t)) {
                    Some(known) if known != Receiver::Unknown => known,
                    _ => rows.kind_of_expr(value, env),
                };
                scopes.last_mut().expect("a block's bindings").names.insert(name.name.clone(), kind);
            }
            Visited::Expr(Expr::Field { receiver, name, .. }) if name.name == "draining" => {
                if reaches_this(&rows.kind_of_expr(receiver, env)) {
                    *read = true;
                }
            }
            Visited::Expr(Expr::Call { callee, args, .. }) => match callee.as_ref() {
                Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                    let name = name.name.as_str();
                    let hands_this = args.iter().any(|a| rows.kind_of_expr(a, env) == this_r);
                    let may_hand_this = hands_this || args.iter().any(|a| rows.kind_of_expr(a, env) == Receiver::Unknown);
                    match rows.kind_of_expr(receiver, env) {
                        // This locus's own method, whatever it is handed.
                        Receiver::Locus(l) if l == this => {
                            callees.push((Some(l), name, args.as_slice(), env.clone()));
                        }
                        // Another locus's method, when the call hands it
                        // this locus (or something unresolved), or when
                        // its declared fields may hold this locus: the
                        // read may be on that argument or through
                        // `self.<field>`. One the rows cannot find (a
                        // stdlib locus's, a builtin's) that is handed this
                        // locus is taken as a read.
                        Receiver::Locus(l) => {
                            if may_hand_this || holders.contains(&l) {
                                if rows.methods.get(l.as_str()).and_then(|m| m.get(name)).is_some() {
                                    callees.push((Some(l), name, args.as_slice(), env.clone()));
                                } else if hands_this {
                                    *read = true;
                                }
                            }
                        }
                        Receiver::NotLocus => {}
                        // An unresolved receiver: this locus's method of
                        // the name, every holder's, and, when the call
                        // hands this locus (or something unresolved),
                        // every locus's; none found and this locus handed
                        // over is a read.
                        Receiver::Unknown => {
                            let mut any = false;
                            if rows.methods.get(this).and_then(|m| m.get(name)).is_some() {
                                callees.push((Some(this.to_string()), name, args.as_slice(), env.clone()));
                                any = true;
                            }
                            for (l, methods) in &rows.methods {
                                if *l != this && methods.contains_key(name) && (may_hand_this || holders.contains(*l)) {
                                    callees.push((Some(l.to_string()), name, args.as_slice(), env.clone()));
                                    any = true;
                                }
                            }
                            if !any && hands_this {
                                *read = true;
                            }
                        }
                    }
                }
                Expr::Ident(id) => {
                    callees.push((None, id.name.as_str(), args.as_slice(), env.clone()));
                    if let Some(l) = &env.this {
                        callees.push((Some(l.clone()), id.name.as_str(), args.as_slice(), env.clone()));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    });
    let mut out = Vec::new();
    for (of, name, args, env) in callees {
        let callee = match &of {
            Some(l) => rows.methods.get(l.as_str()).and_then(|m| m.get(name)).copied(),
            None => rows.free_fns.get(name).copied(),
        };
        let Some(f) = callee else { continue };
        let mut names = BTreeMap::new();
        for (i, p) in f.params.iter().enumerate() {
            let kind = match rows.kind_of_type(&p.ty) {
                Receiver::Unknown => args.get(i).map_or(Receiver::Unknown, |a| rows.kind_of_expr(a, &env)),
                known => known,
            };
            names.insert(p.name.name.clone(), kind);
        }
        out.push((of.clone(), name, f, Env { this: of, names }));
    }
    out
}

/// Whether `body` reads `draining` (on any receiver) itself.
pub fn block_reads_draining(body: &Block) -> bool {
    use crate::listener_window::{visit_block, Visited};
    let mut found = false;
    visit_block(body, &mut |v| {
        if let Visited::Expr(Expr::Field { name, .. }) = v {
            if name.name == "draining" {
                found = true;
            }
        }
    });
    found
}

/// Whether a `run()` body of `decl`'s reads `decl`'s `draining`: the
/// watches-drain column of the run row. In its own body, or in a method
/// (of `decl`'s, or of another locus's when a call hands it `decl` or
/// when that locus's params declare a field that may hold `decl`) or a
/// free fn it calls, transitively ([`followed_callees`]), through any
/// receiver that resolves to `decl` by declared type — `self` in its own
/// bodies, a `let` alias of it in its block, a parameter bound to it, a
/// declared field path whose type is `decl` (by declared types, not by
/// construction) — or that the rows cannot resolve, which is taken as a
/// read. A read of another locus's `draining` (a holder's own
/// `self.draining`), and a read in a handler, a hook or a method the
/// `run()` never reaches, is not the `run()`'s. Callees are entered once
/// per name and binding of their parameters.
pub fn run_reads_draining<'a>(run: &'a Block, decl: &'a LocusDecl, rows: &DrainReads<'a>) -> bool {
    let this = decl.name.name.as_str();
    let holders = rows.holders_of(this);
    let mut seen: BTreeSet<(Option<String>, &'a str, Vec<Receiver>)> = BTreeSet::new();
    let mut pending: Vec<(&'a Block, Env)> =
        vec![(run, Env { this: Some(this.to_string()), names: BTreeMap::new() })];
    while let Some((body, env)) = pending.pop() {
        let mut read = false;
        let callees = followed_callees(body, this, &holders, env, rows, &mut read);
        if read {
            return true;
        }
        for (of, name, f, env) in callees {
            let key = (of, name, env.names.values().cloned().collect());
            if seen.insert(key) {
                pending.push((&f.body, env));
            }
        }
    }
    false
}

// === statically non-returning run(): the never-returns column =====
//
// Moved here from the checker (F.40 phase 3, E2) so the run row holds
// it beside the long-running column; the starvation and birth-order
// laws read the column. A cooperative pool runs each posted `run()`
// cell to completion, so
// two loci on one pool whose `run()` bodies never return means the
// second never starts — silently (bus handlers still fire at
// sleep/yield drains, which makes the hang look like a healthy idle).
// The predicate below is deliberately conservative (same style as
// `while_counter_bounded` in alloc_summary.rs): it only claims
// "statically never returns" for shapes it can prove, so the
// starvation warning never false-fires on a loop that can exit.

/// Does this block contain a statement that can exit the enclosing
/// `run()` loop — `break`, `return`, `terminate`, `fail`, or
/// `violate`? Walked recursively through nested statement bodies but
/// NOT into expressions (a `return` inside a failure-closure exits
/// the closure, not `run()`). A `break` in a *nested* loop only exits
/// that loop, but counting it as an exit here is the conservative
/// direction (a missed warning, never a false one).
fn block_has_loop_exit(block: &Block) -> bool {
    block.stmts.iter().any(stmt_has_loop_exit)
}

fn stmt_has_loop_exit(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break(_)
        | Stmt::Return(..)
        | Stmt::Terminate(_)
        | Stmt::Fail { .. }
        | Stmt::Violate { .. } => true,
        Stmt::If(if_stmt) => if_has_loop_exit(if_stmt),
        Stmt::Match(m) => m.arms.iter().any(|arm| match &arm.body {
            MatchArmBody::Block(b) => block_has_loop_exit(b),
            MatchArmBody::Expr(_) => false,
        }),
        Stmt::For { body, .. } | Stmt::While { body, .. } => block_has_loop_exit(body),
        Stmt::Block(b) => block_has_loop_exit(b),
        Stmt::ShmWrite { body, .. } => block_has_loop_exit(body),
        _ => false,
    }
}

fn if_has_loop_exit(if_stmt: &IfStmt) -> bool {
    if block_has_loop_exit(&if_stmt.then_block) {
        return true;
    }
    match if_stmt.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => block_has_loop_exit(b),
        Some(ElseBranch::ElseIf(inner)) => if_has_loop_exit(inner),
        None => false,
    }
}

/// `Some(field_name)` iff the expression is a bare `self.<field>` read.
fn self_bool_field(e: &Expr) -> Option<&str> {
    match e {
        Expr::Field { receiver, name, .. }
            if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
        {
            Some(name.name.as_str())
        }
        _ => None,
    }
}

/// Is `self.<field> = ...` (or a compound assign to it) present in any
/// member body of the locus? Walked through nested statement bodies.
fn locus_assigns_self_field(decl: &LocusDecl, field: &str) -> bool {
    fn block_assigns(block: &Block, field: &str) -> bool {
        block.stmts.iter().any(|s| stmt_assigns(s, field))
    }
    fn if_assigns(if_stmt: &IfStmt, field: &str) -> bool {
        block_assigns(&if_stmt.then_block, field)
            || match if_stmt.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => block_assigns(b, field),
                Some(ElseBranch::ElseIf(inner)) => if_assigns(inner, field),
                None => false,
            }
    }
    fn stmt_assigns(stmt: &Stmt, field: &str) -> bool {
        match stmt {
            Stmt::Assign { target, .. } => {
                target.head.name == "self"
                    && matches!(
                        target.tail.first(),
                        Some(LValueSeg::Field(f)) if f.name == field
                    )
            }
            Stmt::If(if_stmt) => if_assigns(if_stmt, field),
            Stmt::Match(m) => m.arms.iter().any(|arm| match &arm.body {
                MatchArmBody::Block(b) => block_assigns(b, field),
                MatchArmBody::Expr(_) => false,
            }),
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                block_assigns(body, field)
            }
            Stmt::Block(b) => block_assigns(b, field),
            Stmt::ShmWrite { body, .. } => block_assigns(body, field),
            _ => false,
        }
    }
    decl.members.iter().any(|m| match m {
        LocusMember::Fn(f) => block_assigns(&f.body, field),
        LocusMember::Lifecycle(l) => block_assigns(&l.body, field),
        LocusMember::Mode(md) => block_assigns(&md.body, field),
        LocusMember::Failure(fd) => block_assigns(&fd.body, field),
        _ => false,
    })
}

/// The literal Bool default of a params field, if it has one.
fn param_bool_default(decl: &LocusDecl, field: &str) -> Option<bool> {
    decl.members.iter().find_map(|m| {
        let LocusMember::Params(pb) = m else { return None };
        pb.params.iter().find_map(|p| {
            if p.name.name != field {
                return None;
            }
            match &p.init {
                ParamInit::Value(Expr::Literal(Literal::Bool(b), _)) => Some(*b),
                _ => None,
            }
        })
    })
}

/// `Some(span of the terminal while)` iff this `run()` body statically
/// never returns: its last statement is a `while` whose body contains
/// no exit statement and whose condition provably never flips false —
///   - `while true`,
///   - `while !self.draining` (the synthetic drain flag flips only at
///     shutdown, so for the pool's purposes the loop runs forever),
///   - `while !self.f` / `while self.f` where `f` is a Bool params
///     field that no member body ever assigns and whose declared
///     default keeps the loop live (`false` / `true` respectively).
fn run_statically_nonreturning(run_body: &Block, decl: &LocusDecl) -> Option<Span> {
    let Some(Stmt::While { cond, body, span }) = run_body.stmts.last() else {
        return None;
    };
    if block_has_loop_exit(body) {
        return None;
    }
    let never_flips = match cond {
        Expr::Literal(Literal::Bool(true), _) => true,
        Expr::Unary { op: UnaryOp::Not, operand, .. } => {
            match self_bool_field(operand) {
                Some("draining") => true,
                Some(f) => {
                    !locus_assigns_self_field(decl, f)
                        && param_bool_default(decl, f) == Some(false)
                }
                None => false,
            }
        }
        _ => match self_bool_field(cond) {
            Some(f) if f != "draining" => {
                !locus_assigns_self_field(decl, f)
                    && param_bool_default(decl, f) == Some(true)
            }
            _ => false,
        },
    };
    if never_flips {
        Some(*span)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Long-running" and "never returns" are two columns: a `run()`
    /// that sleeps once is long-running and returns; a terminal
    /// `while true` is both; an empty `run()` and no `run()` are neither.
    /// A module's loci have rows too.
    #[test]
    fn long_running_and_never_returns_are_two_columns() {
        let src = "locus Sleeper { run() { std::time::sleep(1min); } }\n\
                   locus Daemon { run() { while true { std::time::sleep(1s); } } }\n\
                   locus Idle { run() { } }\n\
                   locus Plain { params { n: Int = 0; } }\n\
                   module inner { locus Nested { run() { while true { } } } }\n\
                   fn main() { }\n";
        let p = hale_syntax::parse_source(src).expect("parse");
        let rows = survey(&[&p], &[]);
        let col = |name: &str| {
            let r = rows.runs.iter().find(|r| r.locus == name).expect("a run row");
            (r.long_running, r.never_returns.is_some())
        };
        assert_eq!(col("Sleeper"), (true, false));
        assert_eq!(col("Daemon"), (true, true));
        assert_eq!(col("Idle"), (false, false));
        assert_eq!(col("Plain"), (false, false));
        assert_eq!(col("Nested"), (true, true));
    }

    /// GH #1858: the watches-drain column is the `run()`'s own reads of
    /// `draining`, through the methods and free fns it calls; a read in a
    /// hook, a handler or a method the run does not call does not count,
    /// nor does one only another locus's method makes.
    #[test]
    fn watches_drain_is_the_run_s_reads_and_what_it_calls() {
        let src = "locus Direct { run() { while !self.draining { } } }\n\
                   locus ViaMethod { fn stop() -> Bool { return self.draining; } run() { while !self.stop() { } } }\n\
                   locus ViaFree { run() { while !gate(self) { } } }\n\
                   fn gate(l: ViaFree) -> Bool { return l.draining; }\n\
                   locus Hook { run() { while true { } } drain() { if self.draining { } } }\n\
                   locus Handler { fn on(x: Int) { if self.draining { } } run() { } }\n\
                   locus Uncalled { fn stop() -> Bool { return self.draining; } run() { } }\n\
                   locus NoRun { fn stop() -> Bool { return self.draining; } }\n\
                   fn main() { }\n";
        let p = hale_syntax::parse_source(src).expect("parse");
        let rows = survey(&[&p], &[]);
        let watches = |name: &str| rows.runs.iter().find(|r| r.locus == name).expect("a run row").watches_drain;
        assert!(watches("Direct"));
        assert!(watches("ViaMethod"));
        assert!(watches("ViaFree"));
        assert!(!watches("Hook"));
        assert!(!watches("Handler"));
        assert!(!watches("Uncalled"));
        assert!(!watches("NoRun"));
    }

    /// The read is reached through any receiver that is the locus by
    /// declared type, not only a literal `self` (the #1869 review's
    /// follow-ups, 2026-11-07, each a teardown hang when missed): a `let`
    /// alias, an alias handed to a free fn, another locus's method handed
    /// this locus as an argument, a holder's declared field (`Watch {
    /// target: self }` read as `self.target.draining`), a two-level
    /// holder chain, and an alias an inner block shadows (the bindings
    /// are scoped, so the loop's `w` is still the locus). The other side:
    /// a holder's method reading only its own `self.draining`, with or
    /// without the locus as an argument, is another locus's read; a
    /// `run()` reading a child's `draining` reads another locus's. A
    /// holder's field typed by an alias (a chain of two) is the locus it
    /// aliases; an alias of another locus is that other locus.
    #[test]
    fn watches_drain_is_reached_through_aliases_arguments_and_declared_fields() {
        let src = "locus Alias { fn stop() -> Bool { return self.draining; } run() { let w = self; while !w.stop() { } } }
                   fn gate(w: FreeAlias) -> Bool { return w.draining; }
                   locus FreeAlias { run() { let w = self; while !gate(w) { } } }
                   locus Watch { fn stopped(w: ByArg) -> Bool { return w.draining; } fn own(w: ByArg) -> Bool { return self.draining; } }
                   locus ByArg { params { watcher: Watch = Watch { }; } run() { while !self.watcher.stopped(self) { } } }
                   locus ByArgOwn { params { watcher: Watch = Watch { }; } run() { while !self.watcher.own(self) { } } }
                   locus Keeper { params { target: ByField; } fn stopped() -> Bool { return self.target.draining; } fn own() -> Bool { return self.draining; } }
                   locus ByField { run() { let k = Keeper { target: self }; while !k.stopped() { } } }
                   locus ByFieldOwn { params { k: Keeper; } run() { while !self.k.own() { } } }
                   locus Inner { params { target: ByChain; } }
                   locus Outer { params { inner: Inner; } fn stopped() -> Bool { return self.inner.target.draining; } }
                   locus ByChain { run() { let o = Outer { inner: Inner { target: self } }; while !o.stopped() { } } }
                   locus Other { params { n: Int = 0; } }
                   locus Shadow { run() { let w = self; if true { let w = Other { }; println(w.n); } while !w.draining { } } }
                   locus Child { params { n: Int = 0; } }
                   locus ReadsChild { params { c: Child = Child { }; } run() { while !self.c.draining { } } }
                   type AliasedRef = ByAlias;
                   type AliasedRefRef = AliasedRef;
                   locus AliasKeeper { params { target: AliasedRefRef; } fn stopped() -> Bool { return self.target.draining; } }
                   locus ByAlias { run() { let k = AliasKeeper { target: self }; while !k.stopped() { } } }
                   type OtherRef = Other;
                   locus OtherKeeper { params { target: OtherRef; } fn stopped() -> Bool { return self.target.draining; } }
                   locus ByOtherAlias { params { k: OtherKeeper; } run() { while !self.k.stopped() { } } }
                   fn main() { }
";
        let p = hale_syntax::parse_source(src).expect("parse");
        let rows = survey(&[&p], &[]);
        let watches = |name: &str| rows.runs.iter().find(|r| r.locus == name).expect("a run row").watches_drain;
        assert!(watches("Alias"));
        assert!(watches("FreeAlias"));
        assert!(watches("ByArg"));
        assert!(!watches("ByArgOwn"));
        assert!(watches("ByField"));
        assert!(!watches("ByFieldOwn"));
        assert!(watches("ByChain"));
        assert!(watches("Shadow"));
        assert!(!watches("ReadsChild"));
        assert!(watches("ByAlias"), "a type alias hides the locus from nothing");
        assert!(!watches("ByOtherAlias"), "an alias of another locus is that locus");
    }

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
