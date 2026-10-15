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
//! The columns:
//!
//! 1. `accumulators`, per closure: each `sum(x)`, `count()` and
//!    `mean(x)` of its assertion, in [`accumulator_sites`]' order,
//!    with the element type the checker gave `x`.
//! 2. `generic_calls`, per call site of a generic fn: the inferred type
//!    arguments and the unified parameter types. An omitted function or
//!    method default also records its invocation path and the caller's
//!    specialization, keeping the default expression's source site.
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
//! 6. `omitted_args`, per call site that leaves arguments to their
//!    defaults: the declaration it calls (a fn or a locus method of the
//!    program, resolved as the checker resolves the call, a method by
//!    its receiver's type) and the first parameter it leaves. Lowering
//!    expands those defaults at the call, in the caller's context; the
//!    cross-pool value law reads where (C3 rest, the review of #1351).
//!    A call in a default is resolved in the scope of each evaluation of
//!    that default (a receiver that is the caller's local can be of one
//!    type at one literal and another at the next), so every call the
//!    checker types also has a row by evaluation path ([`CallRow`]: the
//!    callee and what it leaves, a call that leaves nothing included, so
//!    an empty omission is an answer and not a missing one), which the
//!    cross-pool value law and the listener window read
//!    ([`OmittedView::at`]); the union by call is what a reader with no
//!    path, or on a path the checker never typed the call on, asks.
//! 7. `param_accesses`, per body: each read or write of a locus's
//!    `params` field through a receiver the checker typed as that locus
//!    (`self.k`, `self.child.k`, `x.k = v`), with the locus it is read
//!    from inside and the receiver's declaration, recorded before any
//!    rule judges it, on the evaluation path it was typed on (a
//!    parameter's default has a row per call that leaves it). The sealed
//!    rule and the `--sealable` survey read it (F.40 phase 4, W4).
//! 8. `conversions`, per body (GH #1076, U2): every conversion between
//!    an identity or a range and its family, by site (a cast by its call,
//!    an implicit widening by the value's span), with its kind (total,
//!    widening, narrowing), the range a narrowing checks against and what
//!    discharges it. Lowering reads a cast's row (`lower_conversion`) and
//!    decides nothing; the `bare_fallible` law refuses a narrowing nothing
//!    discharges. U3 adds a quantity's and a point's: every place a value
//!    of one denomination meets another (an argument, a binding, a return,
//!    an operand, `.in(u)`, `.split(u)`, a cast, a division by a literal)
//!    with its exact factor ([`Scale`]) and its policy, each quantity
//!    literal with its count converted at compile time, and each printed
//!    quantity with its unit.
//! 9. `recoveries`, per body: each recovery statement written outside
//!    every `on_failure` body (`restart(self.w)` in a method), with its
//!    operation, whether it states a `for` bound, and its receiver as
//!    the checker typed the first argument: the locus that type names, a
//!    monomorph's template. The unreached-event law reads it (F.40 phase
//!    4's leftovers).
//! 10. `ident_reads`, per identifier expression the checker resolved as a
//!    value and per evaluation of it: whether it read a top-level const,
//!    a top-level fn (a callee, or a fn used as a value), or a local or a
//!    parameter in scope there (or another declaration). A default is evaluated, and typed, in the scope of
//!    each literal or call that leaves it (`spec/types.md` § Locus
//!    params), so one site has an answer per evaluation path (the
//!    literals and calls, outermost first, as a conversion row's
//!    `ConversionSite::path`); a site in a body or a declaration's own
//!    default has the empty path. The listener window follows a const
//!    read into the const's value, evaluated where it is read, on the
//!    path it walks, and reads no other name as one (GH #1565). Beside
//!    it, `literal_paths`: every evaluation path each struct or locus
//!    literal was typed on, so a reader starts a construction's walk on
//!    the checker's own path for it (a literal in a default left at two
//!    literals is two constructions) instead of rebuilding it.
//! 11. `views` ([`ViewTypes`], GH #1554), per top-level declaration and
//!    by the node typed rather than by body: each expression's type, each
//!    binding's (a `let`'s, a tuple `let`'s names, a `for` element's, a
//!    parameter's), each assignment target's head and segments, and each
//!    call's callee as the checker resolved it ([`CalleeRef`]). The
//!    owned-view pass (`crate::view_bindings`) reads it instead of
//!    resolving a spelling itself. A reused declaration's table comes
//!    back with its other per-declaration results (`check::DeclChecked`),
//!    re-keyed onto the new parse ([`ViewTypes::rekeyed`]).
//!
//! A site the checker could not type is a [`Hole`] with its reason, and
//! a reader refuses it at its span rather than guessing.

use std::collections::BTreeMap;

use hale_syntax::ast::{
    Block, BusMember, ClosureAssertion, ContractKind, ElseBranch, Expr, Ident, IfStmt, LValueSeg, LocusMember,
    MatchArmBody, MatchStmt, NodeId, OrDisposition, ParamInit, PerspectiveMember, QualifiedName,
    RecoveryModifier, ScalarClause, Stmt, TopDecl, TypeDeclBody, TypeExpr,
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

/// What a conversion between a unit-dialect type and its family is (GH
/// #1076, U2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionKind {
    /// Nothing can be lost and nothing is checked: an `Int` into an
    /// identity with no range (`OrderId(n)`), an identity's `Int`
    /// (`Int(id)`).
    Total,
    /// Into an ancestor: a range into its parent, an `Int`-rooted range
    /// into `Int`, written (`Int(b)`) or implicit (an argument, a `let`).
    Widening,
    /// Into a range the value may be outside of (`Session(n)`): fallible,
    /// discharged where it stands.
    Narrowing,
}

/// What discharges a narrowing where it stands: the form after its `or`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discharge {
    /// `or <value>`: the value instead of one outside the range.
    Substitute,
    /// `or clamp`: the nearest bound.
    Clamp,
    /// `or wrap`: the value modulo the range's width, from its low bound.
    Wrap,
    /// `or handler(err)`: a call given the `RangeError`.
    Handler,
    /// `or raise`: the `RangeError` takes the enclosing error path.
    Raise,
    /// `or fail <payload>`: the enclosing fn fails with a payload of its
    /// own.
    Fail,
    /// `or discard`, which the check refuses on a value.
    Discard,
    /// GH #1076 (U3): a rounding of a ratio narrowing (`or floor`, …),
    /// total: written after the `or` at the site, or the target type's
    /// `round:` policy (`from_type`), which needs no `or`.
    Round { policy: crate::units::RoundPolicy, from_type: bool },
}

impl Discharge {
    /// The form as the program writes it.
    pub fn spelled(self) -> &'static str {
        match self {
            Discharge::Substitute => "or <value>",
            Discharge::Clamp => "or clamp",
            Discharge::Wrap => "or wrap",
            Discharge::Handler => "or handler(err)",
            Discharge::Raise => "or raise",
            Discharge::Fail => "or fail",
            Discharge::Discard => "or discard",
            Discharge::Round { policy, .. } => match policy {
                crate::units::RoundPolicy::Floor => "or floor",
                crate::units::RoundPolicy::Ceil => "or ceil",
                crate::units::RoundPolicy::Trunc => "or trunc",
                crate::units::RoundPolicy::HalfEven => "or half_even",
                crate::units::RoundPolicy::HalfUp => "or half_up",
            },
        }
    }
}

/// Where a conversion is: what it is at ([`SiteKind`]) and the evaluation
/// path it was typed on. `path` is empty outside a default. In a default
/// it is every evaluation from the outermost inward: the struct literal
/// that leaves a field or the call that leaves a parameter, then the
/// literal or call in that default that leaves the next default, and so
/// on. A default is an expression of each scope that leaves it, so its
/// names mean what that scope says (a local can shadow a type there and
/// not elsewhere) and what its values flow into is that scope's: two
/// evaluations of one default, each in its own scope, are two keys for
/// every conversion in it, a cast's, a literal's, a value's. Lowering
/// reads the row by the path it is lowering.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConversionSite {
    pub kind: SiteKind,
    pub path: Vec<u32>,
}

/// What a conversion is at: a cast `T(x)` (and a quantity's `.in(u)` and
/// `.split(u)`) by its call site; an implicit conversion by the span of
/// the value that converts (a quantity literal's own row is there too);
/// a quantity divided by a literal by the division's span (U3); a printed
/// quantity by the span of the value printed (U3); an arithmetic
/// operator over a quantity or a point by its span (U4): the algebra's
/// result type, which lowering reads for the representation the result
/// is (`Int * Duration` is a `Duration`, `Time - Time` a `Duration`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SiteKind {
    Cast(u32),
    Value { start: u32, end: u32 },
    Divide { start: u32, end: u32 },
    Printed { start: u32, end: u32 },
    Operator { start: u32, end: u32 },
}

impl SiteKind {
    pub fn operator(span: Span) -> SiteKind {
        SiteKind::Operator { start: span.start.as_usize() as u32, end: span.end.as_usize() as u32 }
    }

    pub fn value(span: Span) -> SiteKind {
        SiteKind::Value { start: span.start.as_usize() as u32, end: span.end.as_usize() as u32 }
    }

    pub fn divide(span: Span) -> SiteKind {
        SiteKind::Divide { start: span.start.as_usize() as u32, end: span.end.as_usize() as u32 }
    }

    pub fn printed(span: Span) -> SiteKind {
        SiteKind::Printed { start: span.start.as_usize() as u32, end: span.end.as_usize() as u32 }
    }
}

impl ConversionSite {
    /// The site of `kind` on the evaluation path `path` (empty outside a
    /// default).
    pub fn new(kind: SiteKind, path: Vec<u32>) -> ConversionSite {
        ConversionSite { kind, path }
    }
}

/// GH #1076 (U3): a change of denomination, exact: a count of the
/// source becomes `(count × factor.numerator + offset) /
/// factor.denominator` of the target. The denominator is one for a
/// widening; above one, the row's policy says what becomes of the
/// remainder. `offset` is a point's: the two origins' difference, in
/// the numerator's terms (zero for a quantity and for two points of one
/// origin).
#[derive(Debug, Clone, PartialEq)]
pub struct Scale {
    pub factor: crate::unit_graph::Ratio,
    pub offset: i64,
}

/// One conversion the checker classified: the `conversions` column.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversionRow {
    /// The cast's whole call, or the value that widens.
    pub span: Span,
    pub from: Ty,
    pub to: Ty,
    pub kind: ConversionKind,
    /// A narrowing's target range, half-open: what lowering compares
    /// against.
    pub range: Option<(i128, i128)>,
    /// What discharges a narrowing; `None` for a narrowing nothing
    /// discharges (the `bare_fallible` law refuses it) and for every
    /// total or widening conversion, which has nothing to discharge.
    pub policy: Option<Discharge>,
    /// The target as the program spells it (`Session`, `Int`).
    pub target: String,
    /// GH #1076 (U3): a quantity's or a point's change of denomination;
    /// `None` for an identity's or a range's conversion, which changes
    /// no count.
    pub scale: Option<Scale>,
    /// A quantity literal's count in the denomination it flows into,
    /// converted (and, under the target's `round:`, rounded) by the
    /// checker: lowering emits it as a constant.
    pub count: Option<i64>,
    /// `.split(u)`: the count of `u` in the widened denomination, the
    /// divisor of the whole part.
    pub split: Option<i64>,
    /// A printed quantity's or point's unit, written after its count
    /// (`ms`, ` Money in 1/10000 cent`).
    pub printed: Option<String>,
}

impl ConversionRow {
    /// A row of `kind` from `from` to `to`, at `span`, with nothing else
    /// said.
    pub fn new(span: Span, from: Ty, to: Ty, kind: ConversionKind, target: String) -> ConversionRow {
        ConversionRow {
            span,
            from,
            to,
            kind,
            range: None,
            policy: None,
            target,
            scale: None,
            count: None,
            split: None,
            printed: None,
        }
    }
}

/// A call that leaves trailing arguments to their defaults: the
/// declaration of the fn or locus method it calls (a [`hale_syntax::ast::FnDecl`]'s
/// id, in the checked program), and the index of the first parameter it
/// leaves. Lowering expands each of those parameters' defaults at the
/// call, under the caller's locus (`lower_default_in_caller`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OmittedArgs {
    pub callee: u32,
    pub from: usize,
}

/// The `omitted_args` column: by call site, what each call leaves to
/// its defaults. A call the checker walks more than once (a default
/// expanded at two callers, a generic body specialized) keeps every
/// answer.
pub type OmittedArgsByCall = BTreeMap<u32, std::collections::BTreeSet<OmittedArgs>>;

/// An `ident_reads` answer: a local or a parameter in scope.
pub const READ_BINDING: u8 = 1;
/// An `ident_reads` answer: a top-level const.
pub const READ_CONST: u8 = 2;
/// An `ident_reads` answer: a top-level fn (a callee, or a fn used as a
/// value).
pub const READ_FN: u8 = 4;

/// What the checker resolved one call to on one evaluation: the fn or
/// method of the program it calls (`None`: none the program declares, a
/// builtin's, the standard library's or a function value's), and the
/// first parameter it leaves to its default (`None`: it leaves none).
/// A call that leaves nothing has its row too: an empty omission is an
/// answer, not a missing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CallRow {
    pub callee: Option<u32>,
    pub from: Option<usize>,
}

/// The calls the checker typed, by (call site, evaluation path): each
/// call on the one evaluation the path names (the checker's
/// `default_evaluation`, outermost first; empty in a body or a
/// declaration's own default). Not the type declaration's walk of its
/// fields' calls, which is no evaluation.
pub type CallsByPath = BTreeMap<(u32, Vec<u32>), std::collections::BTreeSet<CallRow>>;

static NO_CALLS: OmittedArgsByCall = BTreeMap::new();
static NO_PATHS: CallsByPath = BTreeMap::new();

/// The `omitted_args` column as a reader asks it: by call, the union of
/// every evaluation's answer, or on one evaluation path.
#[derive(Debug, Clone, Copy)]
pub struct OmittedView<'a> {
    pub by_call: &'a OmittedArgsByCall,
    pub by_path: &'a CallsByPath,
}

impl Default for OmittedView<'_> {
    fn default() -> Self {
        OmittedView { by_call: &NO_CALLS, by_path: &NO_PATHS }
    }
}

impl<'a> OmittedView<'a> {
    /// What `call` leaves on the evaluation `path`, nothing when it was
    /// typed there and leaves nothing; with no path (a reader that cannot
    /// say which evaluation it is on), or one the checker never typed the
    /// call on, every evaluation's answer.
    pub fn at(&self, call: u32, path: Option<&[u32]>) -> Vec<OmittedArgs> {
        if let Some(rows) = path.and_then(|p| self.by_path.get(&(call, p.to_vec()))) {
            return rows.iter().filter_map(|r| Some(OmittedArgs { callee: r.callee?, from: r.from? })).collect();
        }
        self.by_call.get(&call).into_iter().flatten().copied().collect()
    }

    /// What the checker resolved `call` to on the evaluation `path`, a row
    /// per answer; `None` when it never typed the call there.
    pub fn calls_at(&self, call: u32, path: &[u32]) -> Option<&'a std::collections::BTreeSet<CallRow>> {
        self.by_path.get(&(call, path.to_vec()))
    }
}

/// Generic calls in defaults evaluated by one caller. Source sites
/// stay unchanged; the invocation path distinguishes repeated expansion
/// of a default, and the caller's arguments distinguish its monomorphs.
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultCalls {
    pub invocations: Vec<u32>,
    pub specialization: Option<Vec<Ty>>,
    pub calls: BTreeMap<u32, Typed<GenericCall>>,
}

/// Whether an access reads a param or assigns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// A field read (`self.child.k`).
    Read,
    /// An assignment target's segment (`self.child.k = v`).
    Write,
}

/// A locus declaration by its site: the store that minted it and its id
/// there. A receiver may be a `std::` locus, whose declaration the
/// bundled stdlib's own mint numbers, so the id alone names nothing.
/// Equal by the id's value: `NodeId`'s own `PartialEq` ignores it.
#[derive(Debug, Clone, Copy, Eq, PartialOrd, Ord, Hash)]
pub struct LocusRef {
    pub universe: crate::placement::SiteUniverse,
    pub decl: NodeId,
}

impl PartialEq for LocusRef {
    fn eq(&self, other: &Self) -> bool {
        self.universe == other.universe && self.decl.0 == other.decl.0
    }
}

/// One access to a `params` field of a locus, through a receiver the
/// checker typed as that locus: the `param_accesses` column.
///
/// Recorded wherever the checker types a field read or an assignment
/// target's field segment whose receiver is a locus declaring that
/// param, sealed or not, and whoever reads it: an access through `self`
/// inside the locus's own members is a row whose reader is the
/// receiver. Only `params` are rows: a capacity slot or a method named
/// on a locus is no state access. A receiver typed as a generic locus's
/// monomorph (`Box_Int`), which the scope declares no locus by, reaches
/// its template's params: the row names the template.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamAccess {
    /// The locus whose member the access is written in (its method, hook,
    /// mode, closure or params default), by declaration; `None` outside
    /// every locus (a free fn).
    pub reader: Option<LocusRef>,
    /// The locus the receiver is typed as, by declaration.
    pub receiver: LocusRef,
    /// The receiver locus's name in the scope (`TopScope::symbols`): the
    /// declared name, mangled for a `std::` locus. Not the spelling a
    /// diagnostic shows.
    pub locus: String,
    pub param: String,
    pub kind: AccessKind,
    /// The read's whole `receiver.param` expression; a write's param
    /// segment.
    pub span: Span,
    /// The evaluation path the access was typed on, as a conversion's
    /// (`ConversionSite::path`): empty outside a default; inside a
    /// parameter's default, typed at each call that leaves it, every
    /// evaluation from the outermost inward. One access in a default is
    /// a row per evaluation, each with that scope's reader.
    pub evaluation: Vec<u32>,
    /// The specialization the access was typed in, by its monomorph's
    /// type arguments: empty on the ordinary walk; a generic fn's or
    /// locus's body walked per monomorph, its parameters bound, has a
    /// row per monomorph for an access through a value of a parameter
    /// type (`self.inner.key` where `inner: T`).
    pub specialization: Vec<Ty>,
}

impl ParamAccess {
    /// Whether the access is written inside the receiver's own members.
    pub fn from_inside(&self) -> bool {
        self.reader == Some(self.receiver)
    }
}

/// One recovery statement written outside every `on_failure` body (a
/// method or a lifecycle body that restarts a child it holds), as the
/// checker typed it: the `recoveries` column. A handler's own statements
/// are its handler row's `ops`, on the handler's child.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryRow {
    /// The locus whose member writes the statement, by declaration (a
    /// generic locus's template); `None` in a free fn.
    pub parent: Option<LocusRef>,
    /// That locus's declared name, as a witness says it.
    pub parent_name: Option<String>,
    /// The statement's span.
    pub statement: Span,
    pub op: hale_syntax::ast::RecoveryOp,
    /// Whether it states a `for` bound: a spent one quarantines the child.
    pub bounded: bool,
    /// The type the checker gave the receiver, the statement's first
    /// argument (`Unknown` for one it could not type).
    pub receiver: Ty,
    /// The locus the receiver's type names, by declaration: a monomorph's
    /// template's. `None` for any other type, an untyped receiver's
    /// included, whose child no row names.
    pub child: Option<LocusRef>,
    /// The specialization the statement was typed in, by its monomorph's
    /// type arguments: empty on the ordinary walk. A generic body walked
    /// per monomorph has a row per monomorph, so a receiver of a
    /// parameter type is named in each.
    pub specialization: Vec<Ty>,
}

/// The rows of one body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypedBody {
    /// The generic locus declaring this member or params default.
    /// Free functions have no enclosing locus, even when a default is
    /// evaluated while a locus method is being lowered.
    pub enclosing_locus: Option<NodeId>,
    pub accumulators: Vec<AccumulatorRow>,
    /// A generic locus's closure: its accumulators for each of the
    /// template's monomorphs, by the monomorph's type arguments, each
    /// `self.X` typed with the field's declared type substituted.
    pub specialized_accumulators: Vec<(Vec<Ty>, Vec<AccumulatorRow>)>,
    /// By call site.
    pub generic_calls: BTreeMap<u32, Typed<GenericCall>>,
    /// A generic fn or locus member: its generic calls for each of the
    /// enclosing template's monomorphs, by their type arguments, typed with the
    /// template's parameters bound to them.
    pub specialized_generic_calls: Vec<(Vec<Ty>, BTreeMap<u32, Typed<GenericCall>>)>,
    pub default_calls: Vec<DefaultCalls>,
    /// By call site.
    pub fallible_calls: BTreeMap<u32, FallibleCall>,
    /// In walk order. A generic fn's or locus's body has the template
    /// walk's rows, then each walk per monomorph's, by its
    /// specialization (`ParamAccess::specialization`): what an access
    /// through a parameter type reaches is known only there.
    pub param_accesses: Vec<ParamAccess>,
    /// GH #1076 (U2): by site, every conversion between an identity or a
    /// range and its family (a cast, an implicit widening).
    pub conversions: BTreeMap<ConversionSite, ConversionRow>,
    /// In walk order: the template walk's, then each walk per monomorph's.
    pub recoveries: Vec<RecoveryRow>,
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

    /// Drop the rows from `len` on, as if never inserted: each was pushed
    /// last on its template's list, and named by its name unless an
    /// earlier row already was.
    fn truncate(&mut self, len: usize) {
        while self.rows.len() > len {
            let row = self.rows.pop().expect("a row past len");
            let i = self.rows.len();
            if let Some(list) = self.by_template.get_mut(&row.template.0) {
                list.pop();
                if list.is_empty() {
                    self.by_template.remove(&row.template.0);
                }
            }
            if self.by_name.get(&row.name) == Some(&i) {
                self.by_name.remove(&row.name);
            }
        }
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

/// What a call names, as the checker resolved it: the `views` column's
/// callee row ([`ViewTypes::callee`], GH #1554).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CalleeRef {
    /// A fn the program (or the stdlib's Hale source) declares, by its
    /// declared name: a bare call, a generic fn, an imported `lib::f`, a
    /// `std::` path to a Hale-source stdlib fn.
    Fn(String),
    /// A stdlib handle's member called as a method (`f.write_line(x)`):
    /// the stdlib fn it is sugar for, the receiver its first argument.
    HandleFn(String),
    /// A method of the locus `owner` declares (its type, as resolved).
    Method { owner: String, name: String },
    /// A method through an interface or a perspective: whichever
    /// declaration's of the name serves it.
    Dispatch(String),
    /// A form's synthesized method on the `@form` locus it names.
    FormOp(String),
    /// An enum variant's constructor.
    Ctor,
    /// A builtin, a stdlib leaf with no Hale body, a conversion to a
    /// scalar type, a builtin member of a primitive value: by its path.
    Std(Vec<String>),
    /// A bare `bounded[T; N]` intrinsic over its first argument.
    Bounded,
    /// A fn value: a local or parameter bound to one, a field of fn type.
    Value,
    /// A callee the checker did not resolve.
    Unknown,
}

/// The `views` column (GH #1554): what the checker typed for the
/// owned-view pass, by the node it typed — an expression, a binding's
/// name, an assignment target's head or segment — one table per top-level
/// declaration. A node is named by its identity in the parsed program the
/// walk and the pass both read (generated code shares spans, so a span
/// names no node), and a reused declaration's table is re-keyed onto the
/// new parse of it ([`ViewTypes::rekeyed`]). A node typed twice with two
/// answers (one expression walked in two contexts) answers `Unknown`,
/// which the pass reads as costing a copy; a node with no row is
/// `Unknown` too.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewTypes {
    exprs: std::collections::HashMap<usize, Ty>,
    bindings: std::collections::HashMap<usize, Ty>,
    places: std::collections::HashMap<usize, Ty>,
    callees: std::collections::HashMap<usize, CalleeRef>,
    /// The declaration reads a method as a value (`let f = k.get;`): a
    /// call through a fn value may hide the receiver it was read from.
    method_values: bool,
}

/// A monomorph of a generic type or locus as its template is with its
/// parameters bound to the monomorph's arguments (GH #1554).
#[derive(Debug, Clone, PartialEq)]
pub enum MonoShape {
    /// A record: its fields' types.
    Struct(Vec<Ty>),
    /// An enum: each variant's payload types.
    Enum(Vec<Vec<Ty>>),
    /// A locus: whether an `@form`, whether one whose reads leave the
    /// cell's nested blocks shared, and its params' types.
    Locus { form: bool, view_form: bool, params: Vec<Ty> },
}

/// Every type and locus monomorph's shape, by its name.
pub type MonoShapes = BTreeMap<String, MonoShape>;

/// Every declaration's `views` table, by program key, in item order (a
/// walk's record holds the bundle's).
pub type ProgramViews = BTreeMap<String, Vec<std::sync::Arc<ViewTypes>>>;

static UNKNOWN_TY: Ty = Ty::Unknown;

/// A node's identity: its address in the parsed program.
fn node<T>(n: &T) -> usize {
    n as *const T as usize
}

fn note<V: PartialEq>(map: &mut std::collections::HashMap<usize, V>, at: usize, v: V, unknown: impl FnOnce() -> V) {
    match map.get_mut(&at) {
        Some(held) if *held != v => *held = unknown(),
        Some(_) => {}
        None => {
            map.insert(at, v);
        }
    }
}

impl ViewTypes {
    /// The type the checker gave the expression `e`.
    pub fn expr(&self, e: &Expr) -> &Ty {
        self.exprs.get(&node(e)).unwrap_or(&UNKNOWN_TY)
    }

    /// Whether the checker typed the expression `e`.
    pub fn has_expr(&self, e: &Expr) -> bool {
        self.exprs.contains_key(&node(e))
    }

    /// The type of the binding named by `name`: a `let`'s, a tuple
    /// `let`'s, a `for` element's, a parameter's. A binding declared as an
    /// interface is that interface (`Ty::Named`), which the checker's own
    /// slot types as `Unknown`.
    pub fn binding(&self, name: &Ident) -> &Ty {
        self.bindings.get(&node(name)).unwrap_or(&UNKNOWN_TY)
    }

    /// The type of an assignment target's head (by its name) or of its
    /// path through a segment (a field by its name, an element by its
    /// index expression).
    pub fn place<T>(&self, at: &T) -> &Ty {
        self.places.get(&node(at)).unwrap_or(&UNKNOWN_TY)
    }

    /// The callee of the call `call`.
    pub fn callee(&self, call: &Expr) -> Option<&CalleeRef> {
        self.callees.get(&node(call))
    }

    pub fn note_expr(&mut self, e: &Expr, ty: Ty) {
        note(&mut self.exprs, node(e), ty, || Ty::Unknown);
    }

    pub fn note_binding(&mut self, name: &Ident, ty: Ty) {
        note(&mut self.bindings, node(name), ty, || Ty::Unknown);
    }

    pub fn note_place<T>(&mut self, at: &T, ty: Ty) {
        note(&mut self.places, node(at), ty, || Ty::Unknown);
    }

    /// Whether the declaration reads a method as a value.
    pub fn method_values(&self) -> bool {
        self.method_values
    }

    pub fn note_method_value(&mut self) {
        self.method_values = true;
    }

    pub fn note_callee(&mut self, call: &Expr, callee: CalleeRef) {
        note(&mut self.callees, node(call), callee, || CalleeRef::Unknown);
    }

    /// The table of a declaration typed as `old`, re-keyed onto `new`, a
    /// parse of the same declaration (equal but for its positions): the
    /// nodes paired in the one order [`view_nodes`] visits both in. A
    /// row of a node that order does not visit is dropped (it reads as
    /// `Unknown`, which costs a copy).
    pub fn rekeyed(&self, old: &TopDecl, new: &TopDecl) -> ViewTypes {
        let (mut from, mut to) = (Vec::new(), Vec::new());
        view_nodes(old, &mut from);
        view_nodes(new, &mut to);
        if from.len() != to.len() {
            return ViewTypes::default();
        }
        let map: std::collections::HashMap<usize, usize> = from.into_iter().zip(to).collect();
        fn move_rows<V: Clone>(
            rows: &std::collections::HashMap<usize, V>,
            map: &std::collections::HashMap<usize, usize>,
        ) -> std::collections::HashMap<usize, V> {
            rows.iter().filter_map(|(k, v)| Some((*map.get(k)?, v.clone()))).collect()
        }
        ViewTypes {
            exprs: move_rows(&self.exprs, &map),
            bindings: move_rows(&self.bindings, &map),
            places: move_rows(&self.places, &map),
            callees: move_rows(&self.callees, &map),
            method_values: self.method_values,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.exprs.is_empty() && self.bindings.is_empty() && self.places.is_empty() && self.callees.is_empty()
    }
}

/// Every node of `item` a `views` row may name, in one fixed order: each
/// body's parameters' names and every expression, binding name and
/// assignment target of the body (a fn's, a locus member's, a
/// perspective's, a module's items' in turn). Two parses of one
/// declaration visit their nodes pairwise.
pub fn view_nodes(item: &TopDecl, out: &mut Vec<usize>) {
    fn params(ps: &[hale_syntax::ast::Param], out: &mut Vec<usize>) {
        for p in ps {
            out.push(node(&p.name));
        }
    }
    fn block(b: &Block, out: &mut Vec<usize>) {
        for st in &b.stmts {
            stmt(st, out);
        }
        if let Some(t) = &b.tail {
            expr(t, out);
        }
    }
    fn if_stmt(i: &IfStmt, out: &mut Vec<usize>) {
        expr(&i.cond, out);
        block(&i.then_block, out);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => block(b, out),
            Some(ElseBranch::ElseIf(e)) => if_stmt(e, out),
            None => {}
        }
    }
    fn pattern(p: &hale_syntax::ast::Pattern, out: &mut Vec<usize>) {
        use hale_syntax::ast::Pattern;
        match p {
            Pattern::Binding(id) => out.push(node(id)),
            Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => args.iter().for_each(|a| pattern(a, out)),
            Pattern::Literal(..) | Pattern::Wildcard(_) => {}
        }
    }
    fn match_stmt(m: &MatchStmt, out: &mut Vec<usize>) {
        expr(&m.scrutinee, out);
        for arm in &m.arms {
            pattern(&arm.pattern, out);
            if let Some(g) = &arm.guard {
                expr(g, out);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => expr(e, out),
                MatchArmBody::Block(b) => block(b, out),
            }
        }
    }
    fn stmt(st: &Stmt, out: &mut Vec<usize>) {
        match st {
            Stmt::Let { name, value, .. } => {
                expr(value, out);
                out.push(node(name));
            }
            Stmt::LetTuple { names, value, .. } => {
                expr(value, out);
                names.iter().for_each(|n| out.push(node(n)));
            }
            Stmt::Assign { target, value, .. } => {
                out.push(node(&target.head));
                for seg in &target.tail {
                    match seg {
                        LValueSeg::Field(f) => out.push(node(f)),
                        LValueSeg::Index(e) => expr(e, out),
                    }
                }
                expr(value, out);
            }
            Stmt::If(i) => if_stmt(i, out),
            Stmt::Match(m) => match_stmt(m, out),
            Stmt::For { name, iter, body, .. } => {
                expr(iter, out);
                out.push(node(name));
                block(body, out);
            }
            Stmt::While { cond, body, .. } => {
                expr(cond, out);
                block(body, out);
            }
            Stmt::Return(e, _) => e.iter().for_each(|e| expr(e, out)),
            Stmt::Fail { value, .. } => expr(value, out),
            Stmt::Recovery { args, .. } => args.iter().for_each(|e| expr(e, out)),
            Stmt::Violate { payload, .. } => payload.iter().for_each(|e| expr(e, out)),
            Stmt::Send { subject, value, or_disposition, .. } => {
                expr(subject, out);
                expr(value, out);
                if let Some(d) = or_disposition {
                    disposition(d, out);
                }
            }
            Stmt::ShmWrite { max, body, .. } => {
                expr(max, out);
                block(body, out);
            }
            Stmt::Block(b) => block(b, out),
            Stmt::Expr(e) => expr(e, out),
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) | Stmt::Reperspective { .. } => {}
        }
    }
    fn disposition(d: &OrDisposition, out: &mut Vec<usize>) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => expr(e, out),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }
    fn expr(e: &Expr, out: &mut Vec<usize>) {
        out.push(node(e));
        match e {
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
            Expr::Binary { left, right, .. } => {
                expr(left, out);
                expr(right, out);
            }
            Expr::Unary { operand, .. } => expr(operand, out),
            Expr::Call { callee, args, .. } => {
                expr(callee, out);
                args.iter().for_each(|a| expr(a, out));
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => expr(receiver, out),
            Expr::Index { receiver, index, .. } => {
                expr(receiver, out);
                expr(index, out);
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => es.iter().for_each(|x| expr(x, out)),
            Expr::Struct { inits, .. } => inits.iter().for_each(|i| expr(&i.value, out)),
            Expr::Block(b) => block(b, out),
            Expr::If(i) => if_stmt(i, out),
            Expr::Match(m) => match_stmt(m, out),
            Expr::Sum(x, _) | Expr::Prod(x, _) => expr(x, out),
            Expr::Approx { left, right, tolerance, .. } => {
                expr(left, out);
                expr(right, out);
                expr(tolerance, out);
            }
            Expr::Range { lo, hi, .. } => {
                expr(lo, out);
                expr(hi, out);
            }
            Expr::ArrayRepeat { val, .. } => expr(val, out),
            Expr::Or { inner, disposition: d, .. } => {
                expr(inner, out);
                disposition(d, out);
            }
        }
    }
    match item {
        TopDecl::Fn(f) => {
            params(&f.params, out);
            block(&f.body, out);
        }
        TopDecl::Locus(l) => {
            for m in &l.members {
                match m {
                    LocusMember::Fn(f) => {
                        params(&f.params, out);
                        block(&f.body, out);
                    }
                    LocusMember::Lifecycle(d) => {
                        params(&d.params, out);
                        block(&d.body, out);
                    }
                    LocusMember::Mode(d) => {
                        params(&d.params, out);
                        block(&d.body, out);
                    }
                    LocusMember::Failure(d) => {
                        params(&d.params, out);
                        block(&d.body, out);
                    }
                    _ => {}
                }
            }
        }
        TopDecl::Perspective(d) => {
            for m in &d.members {
                if let PerspectiveMember::Fn(f) = m {
                    params(&f.params, out);
                    block(&f.body, out);
                }
            }
        }
        TopDecl::Module(m) => m.items.iter().for_each(|i| view_nodes(i, out)),
        _ => {}
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
    /// The `omitted_args` column.
    pub omitted_args: OmittedArgsByCall,
    /// The `omitted_args` column by evaluation path. Each path is its own
    /// row, so a discarded walk keeps what it adds (a field default's
    /// calls are typed only in such a walk).
    pub calls_by_path: CallsByPath,
    /// Which body each conversion belongs to.
    pub conversion_sites: BTreeMap<ConversionSite, u32>,
    /// The `ident_reads` column, by (site, evaluation path). A discarded
    /// walk keeps what it adds: a field default is typed only in such a
    /// walk, and each path is its own row.
    pub ident_reads: BTreeMap<(u32, Vec<u32>), u8>,
    /// Every evaluation path each struct or locus literal was typed on;
    /// kept across discarded walks, as `ident_reads` is.
    pub literal_paths: BTreeMap<u32, std::collections::BTreeSet<Vec<u32>>>,
    /// The `views` column: every declaration's table, the reused ones'
    /// included ([`ViewTypes`]), by program key and item.
    pub views: ProgramViews,
    /// Each monomorph of a generic type or locus, by its name, as its
    /// template is with its parameters bound ([`MonoShape`]): what the
    /// owned-view pass classifies `Box_Int` through.
    pub mono_shapes: std::sync::Arc<MonoShapes>,
    /// What a discarded walk changed, while one is open
    /// ([`Self::open_walk`]).
    walk: Option<WalkUndo>,
}

/// The record as an open walk found it, kept for what the walk writes
/// only: each entry's value before the walk's first write to it (`None`
/// where it had none), and the monomorph table's length. Putting these
/// back leaves the record as a copy taken when the walk opened would,
/// at the price of what the walk touched instead of the whole record.
#[derive(Debug, Clone, Default)]
struct WalkUndo {
    bodies: BTreeMap<u32, Option<TypedBody>>,
    sites: BTreeMap<u32, Option<u32>>,
    omitted_args: BTreeMap<u32, Option<std::collections::BTreeSet<OmittedArgs>>>,
    conversion_sites: BTreeMap<ConversionSite, Option<u32>>,
    monomorphs: usize,
}

impl TypingRecord {
    /// Open a walk whose rows are discarded (a struct field's default,
    /// typed at a literal that leaves it): every write from here on is
    /// undone by [`Self::close_walk`]. Walks do not nest.
    pub fn open_walk(&mut self) {
        debug_assert!(self.walk.is_none(), "a discarded walk opened inside another");
        self.walk = Some(WalkUndo { monomorphs: self.monomorphs.rows.len(), ..WalkUndo::default() });
    }

    /// Close the open walk: the record put back as it was when the walk
    /// opened, and the conversions the walk recorded at a site the record
    /// did not hold then returned (by site), each with its row as the
    /// walk left it.
    pub fn close_walk(&mut self) -> Vec<(ConversionSite, ConversionRow)> {
        let Some(undo) = self.walk.take() else { return Vec::new() };
        let converted: Vec<_> = undo
            .conversion_sites
            .iter()
            .filter(|(_, before)| before.is_none())
            .filter_map(|(site, _)| Some((site.clone(), self.conversion_at(site)?.clone())))
            .collect();
        fn put_back<K: Ord, V>(map: &mut BTreeMap<K, V>, undo: BTreeMap<K, Option<V>>) {
            for (k, before) in undo {
                match before {
                    Some(v) => {
                        map.insert(k, v);
                    }
                    None => {
                        map.remove(&k);
                    }
                }
            }
        }
        put_back(&mut self.bodies, undo.bodies);
        put_back(&mut self.sites, undo.sites);
        put_back(&mut self.omitted_args, undo.omitted_args);
        put_back(&mut self.conversion_sites, undo.conversion_sites);
        self.monomorphs.truncate(undo.monomorphs);
        converted
    }

    fn touch_body(&mut self, decl: u32) {
        if let Some(undo) = &mut self.walk {
            undo.bodies.entry(decl).or_insert_with(|| self.bodies.get(&decl).cloned());
        }
    }

    fn touch_site(&mut self, call: u32) {
        if let Some(undo) = &mut self.walk {
            undo.sites.entry(call).or_insert_with(|| self.sites.get(&call).copied());
        }
    }

    fn touch_conversion_site(&mut self, site: &ConversionSite) {
        if let Some(undo) = &mut self.walk {
            if !undo.conversion_sites.contains_key(site) {
                undo.conversion_sites.insert(site.clone(), self.conversion_sites.get(site).copied());
            }
        }
    }

    /// Record a conversion; a site already recorded keeps its row (a
    /// generic body is walked again per specialization).
    pub fn conversion(&mut self, body: NodeId, site: ConversionSite, row: ConversionRow) {
        let unminted = match &site.kind {
            SiteKind::Cast(id) => NodeId(*id).is_none(),
            SiteKind::Value { .. } | SiteKind::Divide { .. } | SiteKind::Printed { .. } | SiteKind::Operator { .. } => {
                false
            }
        };
        if unminted || site.path.iter().any(|at| NodeId(*at).is_none()) {
            return;
        }
        self.touch_conversion_site(&site);
        self.conversion_sites.insert(site.clone(), body.0);
        self.body(body).conversions.entry(site).or_insert(row);
    }

    /// Record the conversion at `site` in place of what it held: a
    /// quantity literal's row once the checker knows where the literal
    /// flows (U3).
    pub fn reconversion(&mut self, body: NodeId, site: ConversionSite, row: ConversionRow) {
        self.touch_conversion_site(&site);
        self.conversion_sites.insert(site.clone(), body.0);
        self.body(body).conversions.insert(site, row);
    }

    /// What `site` holds, with the body it is held under: what
    /// [`Self::restore_conversion`] puts back.
    pub fn saved_conversion(&self, site: &ConversionSite) -> Option<(u32, ConversionRow)> {
        let body = *self.conversion_sites.get(site)?;
        Some((body, self.bodies.get(&body)?.conversions.get(site)?.clone()))
    }

    /// Put `site` back as [`Self::saved_conversion`] read it: an array
    /// literal's elements, converted into the elements' meet, convert into
    /// the element type of the place the array flows into instead (U3).
    pub fn restore_conversion(&mut self, site: &ConversionSite, saved: Option<(u32, ConversionRow)>) {
        self.touch_conversion_site(site);
        if let Some(body) = self.conversion_sites.remove(site) {
            self.touch_body(body);
            if let Some(b) = self.bodies.get_mut(&body) {
                b.conversions.remove(site);
            }
        }
        if let Some((body, row)) = saved {
            self.reconversion(NodeId(body), site.clone(), row);
        }
    }

    /// Record what discharges the narrowing at `site` (its `or`).
    pub fn discharge(&mut self, site: &ConversionSite, policy: Discharge) {
        let Some(body) = self.conversion_sites.get(site).copied() else { return };
        if let Some(row) = self.body(NodeId(body)).conversions.get_mut(site) {
            row.policy = Some(policy);
        }
    }

    /// The row recorded at `site`.
    pub fn conversion_at(&self, site: &ConversionSite) -> Option<&ConversionRow> {
        let body = self.conversion_sites.get(site)?;
        self.bodies.get(body)?.conversions.get(site)
    }

    /// Record what the identifier expression `site` read on the
    /// evaluation `path`: a top-level const (`constant`), or a binding in
    /// scope or another declaration.
    pub fn ident_read(&mut self, site: NodeId, path: &[u32], read: u8) {
        if site.is_none() {
            return;
        }
        *self.ident_reads.entry((site.0, path.to_vec())).or_insert(0) |= read;
    }

    /// Record that the struct or locus literal `literal` was typed on the
    /// evaluation `path`.
    pub fn literal_typed(&mut self, literal: NodeId, path: &[u32]) {
        if literal.is_none() {
            return;
        }
        self.literal_paths.entry(literal.0).or_default().insert(path.to_vec());
    }

    /// Record what `call` was resolved to on the evaluation `path`:
    /// `row` ([`CallRow`]), a call that leaves nothing included.
    pub fn call_typed(&mut self, call: NodeId, path: &[u32], row: CallRow) {
        if call.is_none() {
            return;
        }
        self.calls_by_path.entry((call.0, path.to_vec())).or_default().insert(row);
    }

    /// Record that `call` leaves `callee`'s parameters from `from` on to
    /// their defaults.
    pub fn omitted_args(&mut self, call: NodeId, callee: NodeId, from: usize) {
        if call.is_none() || callee.is_none() {
            return;
        }
        if let Some(undo) = &mut self.walk {
            undo.omitted_args.entry(call.0).or_insert_with(|| self.omitted_args.get(&call.0).cloned());
        }
        self.omitted_args.entry(call.0).or_default().insert(OmittedArgs { callee: callee.0, from });
    }

    pub fn body(&mut self, decl: NodeId) -> &mut TypedBody {
        self.touch_body(decl.0);
        self.bodies.entry(decl.0).or_default()
    }

    /// Record a default's call under the caller, without changing the
    /// default expression's lexical source identity.
    pub fn default_generic_call(
        &mut self, body: NodeId, invocations: &[u32],
        specialization: Option<Vec<Ty>>, call: NodeId, row: Typed<GenericCall>,
    ) {
        if call.is_none() || invocations.is_empty() {
            return;
        }
        self.touch_site(invocations[0]);
        self.sites.insert(invocations[0], body.0);
        let rows = &mut self.body(body).default_calls;
        let at = match rows.iter().position(|r| r.invocations == invocations && r.specialization == specialization) {
            Some(i) => i,
            None => {
                rows.push(DefaultCalls { invocations: invocations.to_vec(), specialization, calls: BTreeMap::new() });
                rows.len() - 1
            }
        };
        rows[at].calls.insert(call.0, row);
    }

    pub fn generic_call(&mut self, body: NodeId, call: NodeId, row: Typed<GenericCall>) {
        if call.is_none() {
            return;
        }
        self.touch_site(call.0);
        self.sites.insert(call.0, body.0);
        self.body(body).generic_calls.insert(call.0, row);
    }

    pub fn specialized_generic_call(&mut self, body: NodeId, args: Vec<Ty>, call: NodeId, row: Typed<GenericCall>) {
        if call.is_none() {
            return;
        }
        self.touch_site(call.0);
        self.sites.insert(call.0, body.0);
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
        self.touch_site(call.0);
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
    omitted_args: OmittedArgsByCall,
    calls_by_path: CallsByPath,
    conversion_sites: BTreeMap<ConversionSite, u32>,
    ident_reads: BTreeMap<(u32, Vec<u32>), u8>,
    literal_paths: BTreeMap<u32, std::collections::BTreeSet<Vec<u32>>>,
    views: ProgramViews,
    /// Whether a row changes a denomination, prints a unit (U3) or types
    /// an operator's result (U4): a program with none has no value row
    /// for lowering to look up.
    scaled: bool,
}

impl TypedBodies {
    /// Whether lowering has a value's row to look up at all (U3): a
    /// quantity's or a point's conversion, or a printed unit.
    pub fn has_quantity_rows(&self) -> bool {
        self.scaled
    }

    /// The `conversions` column's row at `site`: lowering reads a cast's
    /// by its call (`SiteKind::Cast`), a value's by its span, each in a
    /// default by the evaluation path it lowers too; `None` for a call
    /// that is no conversion there.
    pub fn conversion(&self, site: &ConversionSite) -> Option<&ConversionRow> {
        let body = self.conversion_sites.get(site)?;
        self.bodies.get(body)?.conversions.get(site)
    }

    /// The row at `site` in the body declared at `body`, and in no other:
    /// lowering reads a row from the body it is emitting. A site is
    /// unique within a body but not across bodies (the stdlib's spans
    /// start at 0, like the first user file's), so [`Self::conversion`]'s
    /// one index over every body can answer with another body's row.
    pub fn conversion_in(&self, body: NodeId, site: &ConversionSite) -> Option<&ConversionRow> {
        self.bodies.get(&body.0)?.conversions.get(site)
    }

    /// Whether the checker recorded any conversion in `body`: a body it
    /// typed a quantity literal in has its row. A body with none is one
    /// the checker does not type (the stdlib's own bodies, U4), whose
    /// time literals lowering reads from the stdlib's catalogue.
    pub fn has_conversions_in(&self, body: NodeId) -> bool {
        self.bodies.get(&body.0).is_some_and(|b| !b.conversions.is_empty())
    }

    /// Every conversion's row, body by body.
    pub fn conversions(&self) -> impl Iterator<Item = &ConversionRow> {
        self.bodies.values().flat_map(|b| b.conversions.values())
    }

    /// Every conversion's row with its site, body by body.
    pub fn conversion_sites(&self) -> impl Iterator<Item = (ConversionSite, &ConversionRow)> {
        self.bodies.values().flat_map(|b| b.conversions.iter().map(|(s, r)| (s.clone(), r)))
    }

    /// The `omitted_args` column: what each call leaves to its defaults,
    /// by call and by evaluation path.
    pub fn omitted_args(&self) -> OmittedView<'_> {
        OmittedView { by_call: &self.omitted_args, by_path: &self.calls_by_path }
    }

    /// The `ident_reads` column at the identifier expression `site` on the
    /// evaluation `path`: whether the checker resolved it to a top-level
    /// const there. `None` for a site it never typed as a value on that
    /// path.
    pub fn reads_const(&self, site: NodeId, path: &[u32]) -> Option<bool> {
        self.reads(site, path).map(|r| r & READ_CONST != 0)
    }

    /// The `ident_reads` column at the identifier expression `site` on the
    /// evaluation `path`: what it read there, as [`READ_BINDING`],
    /// [`READ_CONST`] and [`READ_FN`] bits (a site typed more than once
    /// there has each answer's bit); `None` for a site the checker never
    /// typed as a value on that path.
    pub fn reads(&self, site: NodeId, path: &[u32]) -> Option<u8> {
        self.ident_reads.get(&(site.0, path.to_vec())).copied()
    }

    /// Every evaluation path the checker typed the literal `literal` on:
    /// each a construction of it.
    pub fn literal_paths(&self, literal: NodeId) -> impl Iterator<Item = &Vec<u32>> {
        self.literal_paths.get(&literal.0).into_iter().flatten()
    }

    /// The `views` column ([`ViewTypes`]): each program's declarations'
    /// tables, in item order.
    pub fn views(&self) -> &ProgramViews {
        &self.views
    }

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

    /// The declaration owning a generic call's source site. A locus's
    /// different monomorphs keep this same body identity.
    pub fn generic_call_body(&self, call: NodeId) -> Option<NodeId> {
        self.sites.get(&call.0).copied().map(NodeId)
    }

    /// The generic locus declaring this call's source body, if any.
    pub fn generic_call_locus(&self, call: NodeId) -> Option<NodeId> {
        self.body(self.generic_call_body(call)?)?.enclosing_locus
    }

    /// A default's generic call evaluated along this invocation path,
    /// in the first invocation's caller and its concrete specialization.
    pub fn default_generic_call(
        &self, invocations: &[u32], specialization: Option<&[Ty]>, call: NodeId,
    ) -> Option<&Typed<GenericCall>> {
        let body = self.generic_call_body(NodeId(*invocations.first()?))?;
        self.body(body)?.default_calls.iter()
            .find(|r| r.invocations == invocations && r.specialization.as_deref() == specialization)?
            .calls.get(&call.0)
    }

    /// A generic call's concrete row, in its owning body, for the
    /// enclosing fn or locus's type arguments.
    pub fn specialized_generic_call_at(&self, args: &[Ty], call: NodeId) -> Option<&Typed<GenericCall>> {
        self.specialized_generic_call(self.generic_call_body(call)?, args, call)
    }

    /// The row of the generic call at `call` inside the body declared
    /// at `template`, for its enclosing template's monomorph at `args`.
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

    /// Every recovery statement's row outside the handlers, body by body
    /// (by the body's declaration), each body's in walk order.
    pub fn recoveries(&self) -> impl Iterator<Item = (NodeId, &RecoveryRow)> {
        self.bodies.iter().flat_map(|(id, b)| b.recoveries.iter().map(move |r| (NodeId(*id), r)))
    }

    /// Every param access's row, body by body (by the body's
    /// declaration), each body's in walk order.
    pub fn param_accesses(&self) -> impl Iterator<Item = (NodeId, &ParamAccess)> {
        self.bodies.iter().flat_map(|(id, b)| b.param_accesses.iter().map(move |a| (NodeId(*id), a)))
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
        omitted_args: record.omitted_args.clone(),
        calls_by_path: record.calls_by_path.clone(),
        conversion_sites: record.conversion_sites.clone(),
        ident_reads: record.ident_reads.clone(),
        literal_paths: record.literal_paths.clone(),
        views: record.views.clone(),
        scaled: record.bodies.values().any(|b| {
            b.conversions.iter().any(|(site, r)| {
                r.scale.is_some() || r.printed.is_some() || matches!(site.kind, SiteKind::Operator { .. })
            })
        }),
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

struct Visit<'v, 'a> {
    f: &'v mut dyn FnMut(TypeSpelling<'a>, &[String]),
    parameters: Vec<String>,
}

impl<'v, 'a> Visit<'v, 'a> {
    fn emit(&mut self, spelling: TypeSpelling<'a>) {
        (self.f)(spelling, &self.parameters);
    }

    fn with_parameters(&mut self, parameters: &[hale_syntax::ast::GenericParam], walk: impl FnOnce(&mut Self)) {
        let previous = self.parameters.len();
        self.parameters.extend(parameters.iter().map(|p| p.name.name.clone()));
        walk(self);
        self.parameters.truncate(previous);
    }
}

/// Every place `items` spells a type: every declaration's signature,
/// field, parameter, payload, slot and alias, every `let` ascription,
/// and every struct or locus literal's path, at any depth of every body
/// and default. Each visit carries the lexical type parameters in scope.
pub fn for_each_type_spelling<'a>(items: &'a [TopDecl], f: &mut dyn FnMut(TypeSpelling<'a>, &[String])) {
    let mut visitor = Visit { f, parameters: Vec::new() };
    let f = &mut visitor;
    for item in hale_syntax::ast::flat_decls(items) {
        match item {
            TopDecl::Type(t) => f.with_parameters(&t.generics, |f| type_body(&t.body, f)),
            TopDecl::Fn(fd) => fn_decl(fd, f),
            TopDecl::Locus(l) => f.with_parameters(&l.generics, |f| {
                for m in &l.members {
                    locus_member(m, f);
                }
            }),
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
    f.emit(TypeSpelling::Annotation(t));
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
        TypeDeclBody::Scalar(s) => {
            ann(&s.base, f);
            for c in &s.clauses {
                if let ScalarClause::Range { lo, hi, .. } = c {
                    expr(lo, f);
                    expr(hi, f);
                }
            }
        }
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
    f.with_parameters(&fd.generics, |f| {
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
    });
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
        LocusMember::Type(t) => f.with_parameters(&t.generics, |f| type_body(&t.body, f)),
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
            f.emit(TypeSpelling::Literal(path));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// GH #1076 (U3 polish B): a span is a site within one body only. Two
    /// bodies record a row at one equal `Value` span (a user fn's
    /// expression and a stdlib body's, whose spans start at 0 like the
    /// first user file's): each body's read finds its own row and not the
    /// other's, a body that recorded none finds none, and the one index
    /// over every body answers with whichever body recorded last.
    #[test]
    fn a_body_reads_its_own_row_at_a_span_another_body_shares() {
        let span = Span::new(10, 14);
        let row = |target: &str| {
            ConversionRow::new(span, Ty::Named("A".into()), Ty::Named(target.into()), ConversionKind::Widening, target.into())
        };
        let (user, stdlib, neither) = (NodeId(3), NodeId(900), NodeId(7));
        let site = ConversionSite::new(SiteKind::value(span), Vec::new());
        let mut record = TypingRecord::default();
        record.conversion(user, site.clone(), row("User"));
        record.conversion(stdlib, site.clone(), row("Stdlib"));
        let table = TypedBodies {
            bodies: record.bodies.clone(),
            conversion_sites: record.conversion_sites.clone(),
            ..TypedBodies::default()
        };
        let target = |r: Option<&ConversionRow>| r.map(|r| r.target.clone());
        assert_eq!(target(table.conversion_in(user, &site)), Some("User".to_string()));
        assert_eq!(target(table.conversion_in(stdlib, &site)), Some("Stdlib".to_string()));
        assert_eq!(target(table.conversion_in(neither, &site)), None);
        assert_eq!(target(table.conversion(&site)), Some("Stdlib".to_string()), "the one index: the last body's");
    }

    /// A discarded walk (a struct field's default, typed at a literal)
    /// leaves the record as a copy taken when it opened would: every
    /// column it wrote is put back, entries it made are gone, entries it
    /// overwrote or removed hold what they held, the monomorphs it added
    /// are dropped. What it returns is the conversions it recorded at a
    /// site the record did not hold, with the rows it left there.
    #[test]
    fn a_closed_walk_leaves_the_record_as_it_opened() {
        use hale_syntax::ast::PrimType;
        let row = |at: usize, target: &str| {
            ConversionRow::new(Span::new(at, at + 2), Ty::Named("A".into()), Ty::Named(target.into()), ConversionKind::Widening, target.into())
        };
        let site = |at: usize| ConversionSite::new(SiteKind::value(Span::new(at, at + 2)), Vec::new());
        let call = |template: u32| Typed::Known(GenericCall { template: NodeId(template), type_args: Vec::new(), params: Vec::new() });
        let mono = |template: u32, arg: PrimType, name: &str| Monomorph {
            template: NodeId(template),
            kind: TemplateKind::Fn,
            args: vec![Ty::Prim(arg)],
            name: name.into(),
        };
        let (caller, other, fresh) = (NodeId(3), NodeId(5), NodeId(8));
        let mut record = TypingRecord::default();
        record.conversion(caller, site(10), row(10, "Kept"));
        record.conversion(other, site(20), row(20, "Removed"));
        record.conversion(caller, site(30), row(30, "Overwritten"));
        record.generic_call(other, NodeId(40), call(1));
        record.omitted_args(NodeId(50), NodeId(2), 1);
        record.call_typed(NodeId(50), &[], CallRow { callee: Some(2), from: Some(1) });
        record.monomorphs.insert(mono(60, PrimType::Int, "f_Int"));
        let before = record.clone();

        record.open_walk();
        record.conversion(caller, site(10), row(10, "Ignored"));
        record.conversion(caller, site(11), row(11, "New"));
        record.restore_conversion(&site(20), None);
        record.reconversion(caller, site(30), row(30, "Rewritten"));
        record.reconversion(caller, site(31), row(31, "Withdrawn"));
        record.restore_conversion(&site(31), None);
        record.generic_call(caller, NodeId(40), call(9));
        record.generic_call(fresh, NodeId(41), call(9));
        record.omitted_args(NodeId(50), NodeId(2), 0);
        record.omitted_args(NodeId(51), NodeId(2), 0);
        record.call_typed(NodeId(50), &[7], CallRow { callee: Some(2), from: Some(0) });
        record.call_typed(NodeId(51), &[7], CallRow { callee: Some(2), from: None });
        record.body(caller).enclosing_locus = Some(NodeId(70));
        record.monomorphs.insert(mono(60, PrimType::Float, "f_Float"));
        record.monomorphs.insert(mono(61, PrimType::Int, "f_Int"));
        let kept = record.close_walk();

        assert_eq!(record.bodies, before.bodies);
        assert_eq!(record.sites, before.sites);
        assert_eq!(record.omitted_args, before.omitted_args);
        // each evaluation path is its own row, kept
        assert_eq!(record.calls_by_path.len(), 3);
        assert_eq!(record.conversion_sites, before.conversion_sites);
        assert_eq!(record.monomorphs.rows, before.monomorphs.rows);
        assert_eq!(record.monomorphs.by_template, before.monomorphs.by_template);
        assert_eq!(record.monomorphs.by_name, before.monomorphs.by_name);
        assert!(record.walk.is_none());
        let kept: Vec<(ConversionSite, String)> = kept.into_iter().map(|(s, r)| (s, r.target)).collect();
        assert_eq!(kept, vec![(site(11), "New".to_string())], "only the site the record did not hold, still held");
        assert!(record.monomorphs.of(NodeId(60), &[Ty::Prim(PrimType::Float)]).is_none());
        assert_eq!(record.monomorphs.named("f_Int").map(|m| m.template), Some(NodeId(60)));
    }
}
