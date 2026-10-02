//! The typed-body table (F.40 phase 3, E4): what the checker typed,
//! carried to lowering so lowering reads it instead of typing again.
//!
//! The checker records its own answers as it walks
//! ([`TypingRecord`], filled by `check::check_bundle_typing`); the
//! snapshot's `typed_bodies` family packages them into one
//! [`TypedBodies`] per snapshot (`Snapshot::demand_typed_bodies`) and
//! adds the conformance column. Every value is the checker's own `Ty`;
//! nothing here types an expression.
//!
//! The table is keyed by declaration identity, the site the snapshot
//! minted: a body by its declaration's id (a fn, a lifecycle hook, a
//! mode, an `on_failure`, a closure; the enclosing locus or top-level
//! declaration for a value outside any body), a call by its `Call`
//! site. A monomorph is keyed by its template's site and its type
//! arguments, never by a name string; its mangled name is a value of
//! the row, the symbol lowering emits.
//!
//! Five columns:
//!
//! 1. `accumulators`, per closure: each `sum(x)`, `count()` and
//!    `mean(x)` of its assertion, in [`accumulator_sites`]' order,
//!    with the element type the checker gave `x`.
//! 2. `generic_calls`, per call site of a generic fn: the inferred type
//!    arguments and the unified parameter types.
//! 3. `monomorphs`, one table per snapshot: template site x type
//!    arguments -> the specialization, for every generic fn a call
//!    instantiates and every generic type or locus a type expression
//!    writes.
//! 4. `conformance`, per (concrete locus, interface) pair of declared
//!    declarations: whether the locus satisfies the interface, with the
//!    witness when it does not.
//! 5. `fallible_calls`, per call site whose callee is fallible, stdlib
//!    callees included: the callee, its error type, and what addresses
//!    the call where it stands. The `bare_fallible` law reads it.
//!
//! A site the checker could not type is a [`Hole`] with its reason, and
//! a reader refuses it at its span rather than guessing.

use std::collections::BTreeMap;

use hale_syntax::ast::{
    Block, BusMember, ClosureAssertion, ContractKind, ElseBranch, Expr, IfStmt, LValueSeg, LocusMember,
    MatchArmBody, MatchStmt, NodeId, OrDisposition, ParamInit, PerspectiveMember, QualifiedName,
    RecoveryModifier, Stmt, TopDecl, TypeDeclBody, TypeExpr,
};
use hale_syntax::Span;

use crate::ty::Ty;

/// A site the checker could not type, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Hole {
    pub span: Span,
    pub reason: String,
}

/// The checker's answer at a site, or the hole where it has none.
#[derive(Debug, Clone, PartialEq)]
pub enum Typed<T> {
    Known(T),
    Hole(Hole),
}

impl<T> Typed<T> {
    pub fn known(&self) -> Result<&T, &Hole> {
        match self {
            Typed::Known(t) => Ok(t),
            Typed::Hole(h) => Err(h),
        }
    }
}

/// The accumulator vocabulary of a closure assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccumulatorKind {
    /// `sum(x)`: one slot of `x`'s type.
    Sum,
    /// `count()`: one Int slot.
    Count,
    /// `mean(x)`: a running sum of `x`'s type and an Int count.
    Mean,
}

/// One accumulator of an assertion, as [`accumulator_sites`] finds it.
#[derive(Debug, Clone, Copy)]
pub struct AccumulatorSite<'e> {
    pub kind: AccumulatorKind,
    /// The accumulated expression (`x` of `sum(x)`); `None` for `count()`.
    pub inner: Option<&'e Expr>,
    /// The accumulator's own span.
    pub span: Span,
}

/// Every accumulator of `assertion`, in slot order: its left side, its
/// right side, its tolerance, each walked depth-first (a call's callee
/// before its arguments, a binary's left before its right). The order
/// is the accumulator's identity within its closure.
pub fn accumulator_sites(assertion: &ClosureAssertion) -> Vec<AccumulatorSite<'_>> {
    fn walk<'e>(expr: &'e Expr, out: &mut Vec<AccumulatorSite<'e>>) {
        match expr {
            Expr::Sum(inner, span) => {
                out.push(AccumulatorSite { kind: AccumulatorKind::Sum, inner: Some(inner), span: *span });
            }
            Expr::Call { callee, args, span, .. } => {
                if let Expr::Ident(id) = callee.as_ref() {
                    if id.name == "count" && args.is_empty() {
                        out.push(AccumulatorSite { kind: AccumulatorKind::Count, inner: None, span: *span });
                        return;
                    }
                    if id.name == "mean" && args.len() == 1 {
                        out.push(AccumulatorSite {
                            kind: AccumulatorKind::Mean,
                            inner: Some(&args[0]),
                            span: *span,
                        });
                        return;
                    }
                }
                walk(callee, out);
                for a in args {
                    walk(a, out);
                }
            }
            Expr::Binary { left, right, .. } => {
                walk(left, out);
                walk(right, out);
            }
            Expr::Unary { operand, .. } => walk(operand, out),
            Expr::Field { receiver, .. } => walk(receiver, out),
            Expr::Index { receiver, index, .. } => {
                walk(receiver, out);
                walk(index, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&assertion.left, &mut out);
    walk(&assertion.right, &mut out);
    walk(&assertion.tolerance, &mut out);
    out
}

/// One accumulator's row: its kind, its span, and the element type the
/// checker gave the accumulated expression (`None` for `count()`,
/// which accumulates no expression).
#[derive(Debug, Clone, PartialEq)]
pub struct AccumulatorRow {
    pub kind: AccumulatorKind,
    pub span: Span,
    pub elem: Option<Typed<Ty>>,
}

/// A call of a generic fn: the template it instantiates, the type
/// arguments the checker inferred (in the template's parameter order)
/// and the parameter types with those arguments substituted.
#[derive(Debug, Clone, PartialEq)]
pub struct GenericCall {
    pub template: NodeId,
    pub type_args: Vec<Ty>,
    pub params: Vec<Ty>,
}

/// How the checker knows a call's callee is fallible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalleeKind {
    /// The call types as `Fallible` and its callee is a fn or locus
    /// method the program declares `fallible(E)`, spelled the way
    /// lowering resolves one: a free fn by its name (not a generic
    /// one), an imported fn or a bundled stdlib fn by its path, a
    /// locus's member fn on `self`, a local or a field of `self`.
    Declared,
    /// Any other call that types as `Fallible`: a generic fn, a
    /// perspective's or an interface's method, a method on a receiver
    /// of another shape, a stdlib handle's fallible method, a
    /// container's or an array's `get`, a bounded intrinsic.
    Typed,
    /// A stdlib entry point the signature table marks fallible. The
    /// checker types the bare call as `Unknown` (its legacy form), so
    /// the row is the table's mark, read at the call.
    Stdlib,
}

/// What addresses a fallible call where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handling {
    /// The call is the operand of an `or` (`f() or raise`, `f() or
    /// 0`, `f() or handler(err)`, ...).
    Or,
    /// The call is the handler of an `or` (`g() or f(err)`): its own
    /// failure takes the enclosing fn's error path, an implicit `or
    /// raise`. The span is the `or`'s.
    Handler(Span),
    /// Nothing: an argument, an operand, a `match` scrutinee, a `let`
    /// initializer, a statement, a returned value.
    Bare,
}

/// A call whose callee is fallible.
#[derive(Debug, Clone, PartialEq)]
pub struct FallibleCall {
    pub span: Span,
    pub kind: CalleeKind,
    /// The callee as the program spells it (`f`, `self.read`,
    /// `std::str::parse_int`).
    pub callee: String,
    /// The error type the callee declares.
    pub payload: Ty,
    pub handled: Handling,
}

/// The rows of one body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypedBody {
    pub accumulators: Vec<AccumulatorRow>,
    /// A generic locus's closure: its accumulators for each of the
    /// template's monomorphs, by the monomorph's type arguments, each
    /// `self.X` typed with the field's declared type substituted.
    pub specialized_accumulators: Vec<(Vec<Ty>, Vec<AccumulatorRow>)>,
    /// By call site.
    pub generic_calls: BTreeMap<u32, Typed<GenericCall>>,
    /// A generic fn's body: its generic calls for each of the fn's
    /// monomorphs, by the monomorph's type arguments, typed with the
    /// template's parameters bound to them.
    pub specialized_generic_calls: Vec<(Vec<Ty>, BTreeMap<u32, Typed<GenericCall>>)>,
    /// By call site.
    pub fallible_calls: BTreeMap<u32, FallibleCall>,
}

/// What a monomorph's template is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Fn,
    Type,
    Locus,
}

/// One specialization: its template's site and its type arguments (the
/// key), and the name lowering gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct Monomorph {
    pub template: NodeId,
    pub kind: TemplateKind,
    pub args: Vec<Ty>,
    /// The mangled name (`Box_Int`): the symbol the specialization is
    /// declared under, and the name a `Ty::Named` carries for it.
    pub name: String,
}

/// The specialization table of one snapshot.
#[derive(Debug, Clone, Default)]
pub struct Monomorphs {
    rows: Vec<Monomorph>,
    by_template: BTreeMap<u32, Vec<usize>>,
    by_name: BTreeMap<String, usize>,
}

impl Monomorphs {
    pub fn rows(&self) -> &[Monomorph] {
        &self.rows
    }

    /// The specialization of `template` at `args`.
    pub fn of(&self, template: NodeId, args: &[Ty]) -> Option<&Monomorph> {
        self.by_template
            .get(&template.0)?
            .iter()
            .map(|&i| &self.rows[i])
            .find(|m| m.args == args)
    }

    /// The specialization a `Ty::Named` names: the name is the row's
    /// value, so this is the row the type was resolved to.
    pub fn named(&self, name: &str) -> Option<&Monomorph> {
        self.by_name.get(name).map(|&i| &self.rows[i])
    }

    /// Record a specialization; a key already held keeps its row.
    pub fn insert(&mut self, row: Monomorph) {
        if self.of(row.template, &row.args).is_some() {
            return;
        }
        let i = self.rows.len();
        self.by_template.entry(row.template.0).or_default().push(i);
        self.by_name.entry(row.name.clone()).or_insert(i);
        self.rows.push(row);
    }
}

/// The mangle token a type argument contributes to a monomorph name,
/// in `crate::mangle::mangle_generic_name`'s vocabulary: a primitive
/// its name, a named type (a monomorph included) its name. `None` for
/// a form no monomorph can be named for.
pub fn mangle_token(t: &Ty) -> Option<String> {
    match t {
        Ty::Prim(p) => crate::ty::generic_arg_mangle_token(*p).map(str::to_string),
        Ty::Named(n) => Some(n.clone()),
        _ => None,
    }
}

/// Why a locus does not satisfy an interface: the first requirement it
/// leaves unmet, in the interface's method order (the checker's one
/// conformance function, `check::conformance_witness`).
#[derive(Debug, Clone, PartialEq)]
pub enum Unsatisfied {
    /// The type is no locus (a monomorph of a generic locus included:
    /// the checker's scope declares no locus by its name).
    NotALocus,
    Missing { method: String },
    Arity { method: String, want: usize, got: usize },
    Param { method: String, index: usize, want: Ty, got: Ty },
    Ret { method: String, want: Ty, got: Ty },
    /// GH #732: the method's error channel is not the interface's.
    ErrorChannel { method: String, why: &'static str, iface_sig: String, locus_sig: String },
}

impl Unsatisfied {
    /// The witness as an interface conformance's diagnostic.
    pub fn interface_message(&self, locus: &str, iface: &str) -> String {
        match self {
            Unsatisfied::NotALocus => {
                format!("type `{locus}` cannot satisfy interface `{iface}` — only loci satisfy interfaces")
            }
            Unsatisfied::Missing { method } => {
                format!("locus `{locus}` does not satisfy interface `{iface}`: missing method `{method}`")
            }
            Unsatisfied::Arity { method, want, got } => format!(
                "locus `{locus}` method `{method}` arity does not match interface `{iface}`: expected {want} arg(s), locus has {got}"
            ),
            Unsatisfied::Param { method, index, want, got } => format!(
                "locus `{locus}` method `{method}` arg #{index} type mismatch: interface `{iface}` requires `{}`, locus has `{}`",
                want.display(),
                got.display()
            ),
            Unsatisfied::Ret { method, want, got } => format!(
                "locus `{locus}` method `{method}` return type mismatch: interface `{iface}` requires `{}`, locus returns `{}`",
                want.display(),
                got.display()
            ),
            Unsatisfied::ErrorChannel { method, why, iface_sig, locus_sig } => format!(
                "locus `{locus}` method `{method}` {why}: interface `{iface}` declares `{iface_sig}`, locus declares `{locus_sig}`"
            ),
        }
    }

    /// The witness as the bus adapter contract's diagnostic (`iface` is
    /// `__StdBusAdapter`).
    pub fn adapter_message(&self, locus: &str, iface: &str) -> String {
        match self {
            Unsatisfied::NotALocus => format!("`{locus}` is not a locus"),
            Unsatisfied::Missing { method } => {
                format!("locus `{locus}` does not satisfy `{iface}`: missing method `{method}`")
            }
            Unsatisfied::Arity { method, want, got } => format!(
                "locus `{locus}` method `{method}` arity does not match `{iface}`: expected {want} arg(s), locus has {got}"
            ),
            Unsatisfied::Param { method, index, want, got } => format!(
                "locus `{locus}` method `{method}` arg #{index} type mismatch: `{iface}` requires `{}`, locus has `{}`",
                want.display(),
                got.display()
            ),
            Unsatisfied::Ret { method, want, got } => format!(
                "locus `{locus}` method `{method}` return type mismatch: `{iface}` requires `{}`, locus returns `{}`",
                want.display(),
                got.display()
            ),
            // The adapter contract does not judge the error channel.
            Unsatisfied::ErrorChannel { .. } => self.interface_message(locus, iface),
        }
    }
}

/// Whether a concrete locus satisfies an interface, with the witness
/// when it does not.
#[derive(Debug, Clone, PartialEq)]
pub struct Conformance {
    pub concrete: NodeId,
    pub interface: NodeId,
    pub verdict: Result<(), Unsatisfied>,
}

/// What the checker records as it walks: the body rows and the
/// monomorph table. [`typed_bodies`] packages it.
#[derive(Debug, Clone, Default)]
pub struct TypingRecord {
    pub bodies: BTreeMap<u32, TypedBody>,
    /// Which body each recorded call site belongs to.
    pub sites: BTreeMap<u32, u32>,
    pub monomorphs: Monomorphs,
}

impl TypingRecord {
    pub fn body(&mut self, decl: NodeId) -> &mut TypedBody {
        self.bodies.entry(decl.0).or_default()
    }

    pub fn generic_call(&mut self, body: NodeId, call: NodeId, row: Typed<GenericCall>) {
        if call.is_none() {
            return;
        }
        self.sites.insert(call.0, body.0);
        self.body(body).generic_calls.insert(call.0, row);
    }

    pub fn specialized_generic_call(&mut self, body: NodeId, args: Vec<Ty>, call: NodeId, row: Typed<GenericCall>) {
        if call.is_none() {
            return;
        }
        let rows = &mut self.body(body).specialized_generic_calls;
        let at = match rows.iter().position(|(a, _)| *a == args) {
            Some(i) => i,
            None => {
                rows.push((args, BTreeMap::new()));
                rows.len() - 1
            }
        };
        rows[at].1.insert(call.0, row);
    }

    /// Record a fallible call; a call already recorded keeps its row
    /// (the call arm, which knows the callee, records before the walk's
    /// general recording).
    pub fn fallible_call(&mut self, body: NodeId, call: NodeId, row: FallibleCall) {
        if call.is_none() {
            return;
        }
        self.sites.insert(call.0, body.0);
        self.body(body).fallible_calls.entry(call.0).or_insert(row);
    }
}

/// The table of one snapshot: the `typed_bodies` family's.
#[derive(Debug, Clone, Default)]
pub struct TypedBodies {
    bodies: BTreeMap<u32, TypedBody>,
    sites: BTreeMap<u32, u32>,
    monomorphs: Monomorphs,
    conformance: BTreeMap<(u32, u32), Conformance>,
    monomorph_conformance: Vec<(Monomorph, u32, Result<(), Unsatisfied>)>,
}

impl TypedBodies {
    /// The rows of the body declared at `decl`.
    pub fn body(&self, decl: NodeId) -> Option<&TypedBody> {
        self.bodies.get(&decl.0)
    }

    pub fn bodies(&self) -> impl Iterator<Item = (NodeId, &TypedBody)> {
        self.bodies.iter().map(|(id, b)| (NodeId(*id), b))
    }

    /// The accumulators of the closure declared at `closure`, in slot
    /// order; empty for a closure the checker did not walk.
    pub fn accumulators(&self, closure: NodeId) -> &[AccumulatorRow] {
        self.body(closure).map_or(&[], |b| b.accumulators.as_slice())
    }

    /// The accumulators of the closure declared at `closure` in the
    /// specialization `mono` of its generic locus: the template's rows
    /// with each `self.X` typed for the monomorph's arguments. Empty for
    /// a monomorph the checker specialized no rows for.
    pub fn specialized_accumulators(&self, closure: NodeId, mono: &Monomorph) -> &[AccumulatorRow] {
        self.body(closure)
            .and_then(|b| b.specialized_accumulators.iter().find(|(args, _)| *args == mono.args))
            .map_or(&[], |(_, rows)| rows.as_slice())
    }

    /// The row of the generic call at `call`.
    pub fn generic_call(&self, call: NodeId) -> Option<&Typed<GenericCall>> {
        let body = self.sites.get(&call.0)?;
        self.bodies.get(body)?.generic_calls.get(&call.0)
    }

    /// The row of the generic call at `call` inside the generic fn
    /// declared at `template`, in its monomorph at `args`.
    pub fn specialized_generic_call(&self, template: NodeId, args: &[Ty], call: NodeId) -> Option<&Typed<GenericCall>> {
        self.body(template)?
            .specialized_generic_calls
            .iter()
            .find(|(a, _)| a == args)?
            .1
            .get(&call.0)
    }

    /// The row of the fallible call at `call`; `None` for a call whose
    /// callee is not fallible.
    pub fn fallible_call(&self, call: NodeId) -> Option<&FallibleCall> {
        let body = self.sites.get(&call.0)?;
        self.bodies.get(body)?.fallible_calls.get(&call.0)
    }

    /// Every fallible call's row, body by body.
    pub fn fallible_calls(&self) -> impl Iterator<Item = &FallibleCall> {
        self.bodies.values().flat_map(|b| b.fallible_calls.values())
    }

    pub fn monomorphs(&self) -> &Monomorphs {
        &self.monomorphs
    }

    /// Whether the locus declared at `concrete` satisfies the interface
    /// declared at `interface`; `None` for a pair the table holds no row
    /// for (a declaration the checked program does not declare).
    pub fn conformance(&self, concrete: NodeId, interface: NodeId) -> Option<&Conformance> {
        self.conformance.get(&(concrete.0, interface.0))
    }

    pub fn conformance_rows(&self) -> impl Iterator<Item = &Conformance> {
        self.conformance.values()
    }

    /// Whether the specialization `mono` of a generic locus satisfies
    /// the interface declared at `interface`: the checker's verdict for
    /// the monomorph's name, which its scope declares no locus by.
    pub fn monomorph_conformance(&self, mono: &Monomorph, interface: NodeId) -> Option<&Result<(), Unsatisfied>> {
        self.monomorph_conformance
            .iter()
            .find(|(m, i, _)| m.template.0 == mono.template.0 && m.args == mono.args && *i == interface.0)
            .map(|(_, _, verdict)| verdict)
    }

    /// These rows and the conformance of every pair of `items`' loci and
    /// interfaces they do not hold, judged by the same function over
    /// `top`: the lowering view's, whose merged program declares the
    /// bundled stdlib, which the check does not walk. Its declarations
    /// are found by the identities the view's mint gave them.
    pub fn extended(&self, items: &[TopDecl], top: &crate::resolve::TopScope) -> TypedBodies {
        let mut decls = Declared::default();
        decls.add(items);
        self.extended_by(&decls, top)
    }

    fn extended_by(&self, decls: &Declared<'_>, top: &crate::resolve::TopScope) -> TypedBodies {
        let mut out = self.clone();
        for &(i, iname) in &decls.interfaces {
            for &(l, ln) in &decls.loci {
                out.conformance.entry((l.0, i.0)).or_insert_with(|| Conformance {
                    concrete: l,
                    interface: i,
                    verdict: crate::check::conformance_witness(top, ln, iname, true),
                });
            }
        }
        for &(i, iname) in &decls.interfaces {
            for m in self.monomorphs.rows().iter().filter(|m| m.kind == TemplateKind::Locus) {
                if out.monomorph_conformance(m, i).is_none() {
                    let verdict = crate::check::conformance_witness(top, &m.name, iname, true);
                    out.monomorph_conformance.push((m.clone(), i.0, verdict));
                }
            }
        }
        out
    }
}

/// The concrete loci and the interfaces declarations declare, by
/// identity.
#[derive(Default)]
struct Declared<'a> {
    loci: Vec<(NodeId, &'a str)>,
    interfaces: Vec<(NodeId, &'a str)>,
}

impl<'a> Declared<'a> {
    fn add(&mut self, items: &'a [TopDecl]) {
        for item in hale_syntax::ast::flat_decls(items) {
            match item {
                TopDecl::Locus(l) if l.generics.is_empty() && !l.id.is_none() => {
                    self.loci.push((l.id, l.name.name.as_str()))
                }
                TopDecl::Interface(i) if !i.id.is_none() => self.interfaces.push((i.id, i.name.name.as_str())),
                _ => {}
            }
        }
    }
}

/// The producer: the checker's record, with the conformance column
/// judged by the checker's conformance function over `top`, for every
/// pair of a concrete locus and an interface the bundle declares, and
/// for every specialization of a generic locus the monomorph table
/// holds against every interface.
pub fn typed_bodies(bundle: &crate::Bundle<'_>, top: &crate::resolve::TopScope, record: &TypingRecord) -> TypedBodies {
    let table = TypedBodies {
        bodies: record.bodies.clone(),
        sites: record.sites.clone(),
        monomorphs: record.monomorphs.clone(),
        conformance: BTreeMap::new(),
        monomorph_conformance: Vec::new(),
    };
    let mut decls = Declared::default();
    for p in bundle.programs.values() {
        decls.add(&p.items);
    }
    table.extended_by(&decls, top)
}


/// A place a program spells a type.
#[derive(Debug, Clone, Copy)]
pub enum TypeSpelling<'a> {
    /// A type expression, outermost only (a reader recurses into its
    /// generic arguments itself).
    Annotation(&'a TypeExpr),
    /// A struct or locus literal's path.
    Literal(&'a QualifiedName),
}

type Visit<'v, 'a> = dyn FnMut(TypeSpelling<'a>) + 'v;

/// Every place `items` spells a type: every declaration's signature,
/// field, parameter, payload, slot and alias, every `let` ascription,
/// and every struct or locus literal's path, at any depth of every body
/// and default.
pub fn for_each_type_spelling<'a>(items: &'a [TopDecl], f: &mut Visit<'_, 'a>) {
    for item in hale_syntax::ast::flat_decls(items) {
        match item {
            TopDecl::Type(t) => type_body(&t.body, f),
            TopDecl::Fn(fd) => fn_decl(fd, f),
            TopDecl::Locus(l) => {
                for m in &l.members {
                    locus_member(m, f);
                }
            }
            TopDecl::Const(c) => {
                ann(&c.ty, f);
                expr(&c.value, f);
            }
            TopDecl::Topic(t) => ann(&t.payload, f),
            TopDecl::Interface(i) => {
                for m in &i.methods {
                    for p in &m.params {
                        ann(&p.ty, f);
                    }
                    if let Some(r) = &m.ret {
                        ann(r, f);
                    }
                    if let Some(e) = &m.fallible {
                        ann(e, f);
                    }
                }
            }
            TopDecl::Perspective(p) => {
                for m in &p.members {
                    match m {
                        PerspectiveMember::Params(pb) => params(pb, f),
                        PerspectiveMember::StableWhen(b) => block(b, f),
                        PerspectiveMember::SerializeAs(t) => ann(t, f),
                        PerspectiveMember::Fn(fd) => fn_decl(fd, f),
                        PerspectiveMember::Bus(bb) => bus(bb, f),
                    }
                }
            }
            _ => {}
        }
    }
}

fn ann<'a>(t: &'a TypeExpr, f: &mut Visit<'_, 'a>) {
    f(TypeSpelling::Annotation(t));
}

fn type_body<'a>(body: &'a TypeDeclBody, f: &mut Visit<'_, 'a>) {
    match body {
        TypeDeclBody::Struct(fields) => {
            for fl in fields {
                ann(&fl.ty, f);
                if let Some(d) = &fl.default {
                    expr(d, f);
                }
            }
        }
        TypeDeclBody::Enum(variants) => {
            for v in variants {
                for t in &v.fields {
                    ann(t, f);
                }
            }
        }
        TypeDeclBody::Alias(t) => ann(t, f),
    }
}

fn params<'a>(pb: &'a hale_syntax::ast::ParamsBlock, f: &mut Visit<'_, 'a>) {
    for pd in &pb.params {
        if let Some(t) = &pd.ty {
            ann(t, f);
        }
        if let ParamInit::Value(e) = &pd.init {
            expr(e, f);
        }
    }
}

fn fn_params<'a>(ps: &'a [hale_syntax::ast::Param], f: &mut Visit<'_, 'a>) {
    for p in ps {
        ann(&p.ty, f);
        if let Some(d) = &p.default {
            expr(d, f);
        }
    }
}

fn fn_decl<'a>(fd: &'a hale_syntax::ast::FnDecl, f: &mut Visit<'_, 'a>) {
    for g in &fd.generics {
        if let Some(b) = &g.bound {
            ann(b, f);
        }
    }
    fn_params(&fd.params, f);
    if let Some(r) = &fd.ret {
        ann(r, f);
    }
    if let Some(e) = &fd.fallible {
        ann(e, f);
    }
    block(&fd.body, f);
}

fn bus<'a>(bb: &'a hale_syntax::ast::BusBlock, f: &mut Visit<'_, 'a>) {
    for bm in &bb.members {
        match bm {
            BusMember::Subscribe { ty: Some(t), .. } | BusMember::Publish { ty: Some(t), .. } => ann(t, f),
            _ => {}
        }
    }
}

fn locus_member<'a>(m: &'a LocusMember, f: &mut Visit<'_, 'a>) {
    match m {
        LocusMember::Params(pb) => params(pb, f),
        LocusMember::Bus(bb) => bus(bb, f),
        LocusMember::Lifecycle(lc) => {
            fn_params(&lc.params, f);
            if let Some(r) = &lc.ret {
                ann(r, f);
            }
            block(&lc.body, f);
        }
        LocusMember::Mode(md) => {
            fn_params(&md.params, f);
            if let Some(r) = &md.ret {
                ann(r, f);
            }
            block(&md.body, f);
        }
        LocusMember::Failure(fd) => {
            fn_params(&fd.params, f);
            block(&fd.body, f);
        }
        LocusMember::Closure(cd) => {
            if let Some(a) = &cd.assertion {
                expr(&a.left, f);
                expr(&a.right, f);
                expr(&a.tolerance, f);
            }
        }
        LocusMember::Fn(fd) => fn_decl(fd, f),
        LocusMember::Const(c) => {
            ann(&c.ty, f);
            expr(&c.value, f);
        }
        LocusMember::Type(t) => type_body(&t.body, f),
        LocusMember::Capacity(cb) => {
            for s in &cb.slots {
                ann(&s.elem_ty, f);
            }
        }
        LocusMember::Contract(cb) => {
            if let ContractKind::Members(ms) = &cb.kind {
                for cm in ms {
                    if let Some(t) = &cm.ty {
                        ann(t, f);
                    }
                }
            }
        }
        _ => {}
    }
}

fn block<'a>(b: &'a Block, f: &mut Visit<'_, 'a>) {
    for s in &b.stmts {
        stmt(s, f);
    }
    if let Some(t) = &b.tail {
        expr(t, f);
    }
}

fn if_stmt<'a>(i: &'a IfStmt, f: &mut Visit<'_, 'a>) {
    expr(&i.cond, f);
    block(&i.then_block, f);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => block(b, f),
        Some(ElseBranch::ElseIf(n)) => if_stmt(n, f),
        None => {}
    }
}

fn match_stmt<'a>(m: &'a MatchStmt, f: &mut Visit<'_, 'a>) {
    expr(&m.scrutinee, f);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            expr(g, f);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => expr(e, f),
            MatchArmBody::Block(b) => block(b, f),
        }
    }
}

fn stmt<'a>(s: &'a Stmt, f: &mut Visit<'_, 'a>) {
    match s {
        Stmt::Let { ty, value, .. } | Stmt::LetTuple { ty, value, .. } => {
            if let Some(t) = ty {
                ann(t, f);
            }
            expr(value, f);
        }
        Stmt::Assign { target, value, .. } => {
            for seg in &target.tail {
                if let LValueSeg::Index(ix) = seg {
                    expr(ix, f);
                }
            }
            expr(value, f);
        }
        Stmt::If(i) => if_stmt(i, f),
        Stmt::Match(m) => match_stmt(m, f),
        Stmt::For { iter, body, .. } => {
            expr(iter, f);
            block(body, f);
        }
        Stmt::While { cond, body, .. } => {
            expr(cond, f);
            block(body, f);
        }
        Stmt::Return(Some(e), _) | Stmt::Fail { value: e, .. } | Stmt::Expr(e) => expr(e, f),
        Stmt::Block(b) => block(b, f),
        Stmt::Recovery { args, modifier, .. } => {
            for a in args {
                expr(a, f);
            }
            if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                expr(e, f);
            }
        }
        Stmt::Violate { payload: Some(e), .. } => expr(e, f),
        Stmt::Send { subject, value, or_disposition, .. } => {
            expr(subject, f);
            expr(value, f);
            if let Some(OrDisposition::Substitute(sub)) = or_disposition {
                expr(sub, f);
            }
        }
        Stmt::ShmWrite { max, body, .. } => {
            expr(max, f);
            block(body, f);
        }
        _ => {}
    }
}

fn expr<'a>(e: &'a Expr, f: &mut Visit<'_, 'a>) {
    match e {
        Expr::Struct { path, inits, .. } => {
            f(TypeSpelling::Literal(path));
            for i in inits {
                expr(&i.value, f);
            }
        }
        Expr::Block(b) => block(b, f),
        Expr::If(i) => if_stmt(i, f),
        Expr::Match(m) => match_stmt(m, f),
        Expr::Call { callee, args, .. } => {
            expr(callee, f);
            for a in args {
                expr(a, f);
            }
        }
        Expr::Binary { left, right, .. } => {
            expr(left, f);
            expr(right, f);
        }
        Expr::Unary { operand, .. } => expr(operand, f),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => expr(receiver, f),
        Expr::Index { receiver, index, .. } => {
            expr(receiver, f);
            expr(index, f);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for x in es {
                expr(x, f);
            }
        }
        Expr::Sum(x, _) | Expr::Prod(x, _) => expr(x, f),
        Expr::Approx { left, right, tolerance, .. } => {
            expr(left, f);
            expr(right, f);
            expr(tolerance, f);
        }
        Expr::Range { lo, hi, .. } => {
            expr(lo, f);
            expr(hi, f);
        }
        Expr::ArrayRepeat { val, .. } => expr(val, f),
        Expr::Or { inner, disposition, .. } => {
            expr(inner, f);
            if let OrDisposition::Substitute(sub) = disposition {
                expr(sub, f);
            }
        }
        Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
    }
}
