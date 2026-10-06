//! The unit dialect's declaration layer (GH #1076, U1).
//!
//! A program declares units (`unit us = 1_000 ns;`) and the scalar types
//! counted in them (`type Money = quantity Int in cent;`).
//! [`derive_unit_rows`] makes one row per declaration (the
//! `unit_declarations` family), resolves every unit a declaration names,
//! and closes the program's catalogue from the equations
//! ([`UnitGraph::close`]). [`unit_laws`] judges the rows, each law a
//! registered rule of `spec/verification.md`'s structural table, and is
//! the one entry the check runs.
//!
//! Values of the new types are not typed yet: [`value_not_yet`] is the
//! one error a use of one gets, wherever a value would live.

use std::collections::BTreeMap;

use hale_graph::ids::{SeedId, SiteId};
use hale_syntax::ast::{
    flat_decls, Expr, Literal, PrimType, ScalarClause, ScalarDecl, ScalarKind, TopDecl, TypeDecl, TypeDeclBody,
    TypeExpr, UnaryOp, UnitDecl,
};
use hale_syntax::lexer::DURATION_SUFFIXES;
use hale_syntax::{Diag, Span};
use num_bigint::BigInt;
use num_rational::BigRational;

use crate::law::{Law, RuleId, Violation};
use crate::placement::SiteRef;
use crate::stdlib_surface::nearest_name;
use crate::unit_graph::{CatalogueError, Denom, Equation, Ratio, UnitGraph};
use crate::Bundle;

/// Law 1: a unit declared twice.
const DECLARED_ONCE: RuleId = RuleId::registered("verification/structural", "unit-declared-once");
/// Law 2: an equation, a denomination or an origin naming no declared unit.
const DECLARED_UNIT: RuleId = RuleId::registered("verification/structural", "declared-unit");
/// Law 3: a cycle of equations whose product is not one.
const CYCLES: RuleId = RuleId::registered("verification/structural", "unit-cycles-multiply-to-one");
/// Law 4: two quantities in one component.
const ONE_QUANTITY: RuleId = RuleId::registered("verification/structural", "one-quantity-per-component");
/// Law 5: a quantity that counts no `Int` or names no denomination.
const COUNTS_AN_INT: RuleId = RuleId::registered("verification/structural", "quantity-counts-an-int");
/// Law 6: a point over no quantity, an origin off a point or out of its
/// component.
const POINT: RuleId = RuleId::registered("verification/structural", "point-over-a-quantity");
/// Law 7: a `round:` or a `range:` that cannot mean anything.
const CLAUSES: RuleId = RuleId::registered("verification/structural", "round-and-range-clauses");
/// Law 8: an identity that is not `distinct Int`, a range over no `Int`.
const BASES: RuleId = RuleId::registered("verification/structural", "identity-and-range-bases");
/// Law 9: a refinement of a struct, an enum, or nothing.
const REFINEMENT: RuleId = RuleId::registered("verification/structural", "refinement-of-a-scalar");
/// Law 10: a unit named like a duration literal's suffix.
const DURATION_NAME: RuleId = RuleId::registered("verification/structural", "unit-named-like-a-duration-suffix");

/// The number one, as a node of the program's catalogue: the target of
/// an equation written against a number (`unit pct = 1/100;`). It is an
/// identity the snapshot's mint never issues (seeds are numbered from
/// zero, one per seed), so no declaration has it, and the catalogue's
/// API, which knows only `SiteId`s, needs no notion of it.
const PURE_NUMBER: SiteId = SiteId::new(SeedId(u32::MAX), u32::MAX);

/// Every unit-dialect declaration of the programs, resolved, and the
/// catalogue closed from them.
#[derive(Debug, Clone, Default)]
pub struct UnitRows {
    /// One per `unit` declaration, in program order.
    pub units: Vec<UnitRow>,
    /// One per equation (`= N TARGET`, `= N/D`), in program order.
    pub equations: Vec<EquationRow>,
    /// One per scalar `type` declaration (a quantity, a point, an
    /// identity, a range or a refinement of one), in program order.
    pub scalars: Vec<ScalarRow>,
    /// Every place a declaration names a unit: an equation's target, a
    /// denomination, an origin; with the unit it names when one is
    /// declared.
    pub refs: Vec<UnitRef>,
    /// The closed catalogue; `None` when a catalogue law fails (a unit
    /// declared twice, an equation naming no declared unit, a cycle that
    /// does not multiply to one).
    pub catalogue: Option<UnitGraph>,
    /// What the closure refused: the inconsistent cycles, with their
    /// witnesses.
    pub cycles: Vec<CatalogueError>,
}

/// A `unit` declaration.
#[derive(Debug, Clone)]
pub struct UnitRow {
    pub site: SiteRef,
    pub name: String,
    /// The declaration, and its name.
    pub span: Span,
    pub name_span: Span,
    /// Its component: the lowest unit row connected to it by the
    /// equations whose target is declared.
    pub component: usize,
    /// Whether its component holds the number one (law 11): every unit
    /// written against a number is in this one component.
    pub dimensionless: bool,
}

/// What an equation's target is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    Unit(usize),
    /// The number one: `unit pct = 1/100;`.
    Pure,
}

/// `unit NAME = N/D TARGET;`: one `unit` is `factor` `target`.
#[derive(Debug, Clone)]
pub struct EquationRow {
    pub site: SiteRef,
    pub unit: usize,
    /// `None` when the target names no declared unit (law 2).
    pub target: Option<Node>,
    pub factor: Ratio,
    /// The equation as written.
    pub span: Span,
}

/// Where a unit is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefAt {
    /// An equation's target.
    Equation(usize),
    /// A scalar's `in` denomination.
    Denomination(usize),
    /// A scalar's `origin:`.
    Origin(usize),
}

/// A unit named by a declaration.
#[derive(Debug, Clone)]
pub struct UnitRef {
    pub name: String,
    pub span: Span,
    /// The unit row it names, the first of that name.
    pub unit: Option<usize>,
    pub at: RefAt,
}

/// What a scalar declaration is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarKindRow {
    /// `quantity Int in cent`, and a refinement of a quantity.
    Quantity,
    /// `point Q`, and a refinement of a point.
    Point,
    /// `distinct Int`.
    Identity,
    /// `Int { range: … }`, and a range over an identity or a range.
    Range,
}

impl ScalarKindRow {
    fn article(self) -> &'static str {
        match self {
            ScalarKindRow::Quantity => "a quantity",
            ScalarKindRow::Point => "a point",
            ScalarKindRow::Identity => "an identity",
            ScalarKindRow::Range => "a range",
        }
    }
}

/// A `round:` policy: how a narrowing into the type discards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundPolicy {
    Floor,
    Ceil,
    Trunc,
    HalfEven,
    HalfUp,
}

impl RoundPolicy {
    pub const ALL: [RoundPolicy; 5] =
        [RoundPolicy::Floor, RoundPolicy::Ceil, RoundPolicy::Trunc, RoundPolicy::HalfEven, RoundPolicy::HalfUp];

    pub fn name(self) -> &'static str {
        match self {
            RoundPolicy::Floor => "floor",
            RoundPolicy::Ceil => "ceil",
            RoundPolicy::Trunc => "trunc",
            RoundPolicy::HalfEven => "half_even",
            RoundPolicy::HalfUp => "half_up",
        }
    }

    fn of(name: &str) -> Option<RoundPolicy> {
        RoundPolicy::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// A scalar `type` declaration, resolved.
#[derive(Debug, Clone)]
pub struct ScalarRow {
    pub site: SiteRef,
    /// The declared name (an imported one mangled), and the name as its
    /// author spells it.
    pub name: String,
    pub display: String,
    pub span: Span,
    pub name_span: Span,
    pub kind: ScalarKindRow,
    /// The scalar it is written over (`point Duration`'s `Duration`,
    /// `Ledger { … }`'s `Ledger`), when its base is one.
    pub parent: Option<usize>,
    /// A quantity's or a point's component: its denomination's unit's.
    pub component: Option<usize>,
    /// A unit and an exact positive multiple of it: as written, or the
    /// parent's.
    pub denomination: Option<Denom>,
    /// The component's one quantity (law 4): declared with `quantity`
    /// and no policy.
    pub principal: bool,
    /// A point's quantity; a refinement's or a range's parent; a boundary
    /// denomination's principal quantity.
    pub of: Option<usize>,
    /// A point's origin, as a count of its own denomination.
    pub origin: Option<BigRational>,
    pub policy: Option<RoundPolicy>,
    /// Half-open, as written (an inclusive bound is stored plus one).
    pub range: Option<(i128, i128)>,
    /// The declaration as written: what the laws judge and point at.
    pub written: Written,
}

/// A scalar declaration as written.
#[derive(Debug, Clone)]
pub struct Written {
    pub kind: Option<ScalarKind>,
    pub base: Base,
    pub base_span: Span,
    pub denomination: Option<WrittenDenom>,
    /// The policy's name and its span.
    pub round: Option<(String, Span)>,
    pub range: Option<WrittenRange>,
    pub origin: Option<WrittenOrigin>,
}

/// `in 100 ms`: the multiple, the unit's ref, the denomination's span.
#[derive(Debug, Clone, Copy)]
pub struct WrittenDenom {
    pub multiple: u64,
    pub unit: usize,
    pub span: Span,
}

/// `range: LO..HI`: the bounds, or the span of a bound that is no
/// integer literal; and the clause's span.
#[derive(Debug, Clone, Copy)]
pub struct WrittenRange {
    pub bounds: Result<(i128, i128), Span>,
    pub span: Span,
}

/// `origin: N UNIT`: the count, the unit's ref, the clause's span.
#[derive(Debug, Clone, Copy)]
pub struct WrittenOrigin {
    pub value: i64,
    pub unit: usize,
    pub span: Span,
}

/// What a scalar declaration is written over.
#[derive(Debug, Clone, PartialEq)]
pub enum Base {
    Int,
    /// Another primitive (`Float`, `String`).
    Primitive(PrimType),
    /// A scalar row.
    Scalar(usize),
    /// A declared type that is no scalar: its name, and what it is (`a
    /// struct`, `an alias of an enum`, `a locus`).
    Other { name: String, what: String },
    /// A name nothing declares.
    Unknown(String),
    /// A refinement that comes back to itself: the names on the way.
    Cycle(Vec<String>),
}

impl Base {
    /// The base as a message names it.
    fn spelled(&self, rows: &UnitRows) -> String {
        match self {
            Base::Int => "`Int`".to_string(),
            Base::Primitive(p) => format!("`{}`", crate::ty::prim_name(*p)),
            Base::Scalar(i) => format!("`{}`", rows.scalars[*i].display),
            Base::Other { name, .. } | Base::Unknown(name) => format!("`{name}`"),
            Base::Cycle(names) => format!("`{}`", names[0]),
        }
    }

    /// What the base is, for a message: `a point`, `a struct`.
    fn what(&self, rows: &UnitRows) -> String {
        match self {
            Base::Int | Base::Primitive(_) => "a primitive".to_string(),
            Base::Scalar(i) => rows.scalars[*i].kind.article().to_string(),
            Base::Other { what, .. } => what.clone(),
            Base::Unknown(_) | Base::Cycle(_) => "not a declared type".to_string(),
        }
    }

    /// Whether another law already said what is wrong with the base.
    fn unresolved(&self) -> bool {
        matches!(self, Base::Unknown(_) | Base::Cycle(_))
    }
}

/// The declarations a base may name, by name: the bundle's types (the
/// scalars among them by row), and its other declarations a type
/// position could name.
struct Names<'b> {
    types: BTreeMap<&'b str, &'b TypeDecl>,
    scalars: BTreeMap<&'b str, usize>,
    others: BTreeMap<&'b str, &'static str>,
}

/// The `unit_declarations` family's producer: every `unit` and scalar
/// `type` declaration of the bundle's programs (module-nested ones
/// included), in program order, each by its declaration's site, every
/// unit a declaration names resolved by name, and the catalogue closed
/// from the equations whose units resolve. The stdlib declares no unit
/// and no scalar yet.
pub fn derive_unit_rows(bundle: &Bundle<'_>) -> UnitRows {
    let mut rows = UnitRows::default();
    let mut unit_decls: Vec<&UnitDecl> = Vec::new();
    let mut scalar_decls: Vec<(&TypeDecl, &ScalarDecl, SiteId)> = Vec::new();
    let mut names = Names { types: BTreeMap::new(), scalars: BTreeMap::new(), others: BTreeMap::new() };
    for program in bundle.programs.values() {
        for item in flat_decls(&program.items) {
            match item {
                TopDecl::Unit(u) => unit_decls.push(u),
                TopDecl::Type(t) => {
                    names.types.entry(t.name.name.as_str()).or_insert(t);
                    if let (TypeDeclBody::Scalar(s), Some(site)) = (&t.body, bundle.snapshot.site_id(t.id)) {
                        names.scalars.entry(t.name.name.as_str()).or_insert(scalar_decls.len());
                        scalar_decls.push((t, s, site));
                    }
                }
                TopDecl::Locus(l) => {
                    names.others.entry(l.name.name.as_str()).or_insert("a locus");
                }
                TopDecl::Interface(i) => {
                    names.others.entry(i.name.name.as_str()).or_insert("an interface");
                }
                TopDecl::Perspective(p) => {
                    names.others.entry(p.name.name.as_str()).or_insert("a perspective");
                }
                _ => {}
            }
        }
    }

    // The units, then their equations: an equation may name a unit
    // declared after it.
    let mut by_name: BTreeMap<&str, usize> = BTreeMap::new();
    let mut declared: Vec<&UnitDecl> = Vec::new();
    for u in unit_decls {
        let Some(site) = bundle.snapshot.site_id(u.id) else { continue };
        let i = rows.units.len();
        by_name.entry(u.name.name.as_str()).or_insert(i);
        rows.units.push(UnitRow {
            site: SiteRef::user(site),
            name: u.name.name.clone(),
            span: u.span,
            name_span: u.name.span,
            component: i,
            dimensionless: false,
        });
        declared.push(u);
    }
    for (i, u) in declared.iter().enumerate() {
        let Some(eq) = &u.equation else { continue };
        let Some(site) = bundle.snapshot.site_id(eq.id) else { continue };
        // The parser refuses a zero in either place.
        let Some(factor) = Ratio::new(BigInt::from(eq.num), BigInt::from(eq.den)) else { continue };
        let e = rows.equations.len();
        let target = match &eq.target {
            None => Some(Node::Pure),
            Some(t) => {
                let unit = by_name.get(t.name.as_str()).copied();
                rows.refs.push(UnitRef { name: t.name.clone(), span: t.span, unit, at: RefAt::Equation(e) });
                unit.map(Node::Unit)
            }
        };
        rows.equations.push(EquationRow { site: SiteRef::user(site), unit: i, target, factor, span: eq.span });
    }
    components(&mut rows);
    close(&mut rows, &by_name);

    // The scalars, as written, then resolved.
    for (k, (t, s, site)) in scalar_decls.iter().enumerate() {
        let mut unit_ref = |name: &hale_syntax::ast::Ident, at: RefAt| {
            let unit = by_name.get(name.name.as_str()).copied();
            rows.refs.push(UnitRef { name: name.name.clone(), span: name.span, unit, at });
            rows.refs.len() - 1
        };
        let denomination = s.denom.as_ref().map(|d| WrittenDenom {
            multiple: d.multiple,
            unit: unit_ref(&d.unit, RefAt::Denomination(k)),
            span: d.span,
        });
        let mut round = None;
        let mut range = None;
        let mut origin = None;
        for c in &s.clauses {
            match c {
                ScalarClause::Round { policy, .. } => round = Some((policy.name.clone(), policy.span)),
                ScalarClause::Range { lo, hi, inclusive, span } => {
                    let bounds = match (bound(lo), bound(hi)) {
                        (Ok(lo), Ok(hi)) => Ok((lo, if *inclusive { hi + 1 } else { hi })),
                        (Err(at), _) | (_, Err(at)) => Err(at),
                    };
                    range = Some(WrittenRange { bounds, span: *span });
                }
                ScalarClause::Origin { value, unit, span } => {
                    origin = Some(WrittenOrigin { value: *value, unit: unit_ref(unit, RefAt::Origin(k)), span: *span })
                }
            }
        }
        rows.scalars.push(ScalarRow {
            site: SiteRef::user(*site),
            name: t.name.name.clone(),
            display: t.display.clone().unwrap_or_else(|| t.name.name.clone()),
            span: t.span,
            name_span: t.name.span,
            kind: ScalarKindRow::Range,
            parent: None,
            component: None,
            denomination: None,
            principal: false,
            of: None,
            origin: None,
            policy: round.as_ref().and_then(|(p, _)| RoundPolicy::of(p)),
            range: range.and_then(|r| r.bounds.ok()),
            written: Written {
                kind: s.kind,
                base: base(&s.base, &names, 0),
                base_span: s.base.span(),
                denomination,
                round,
                range,
                origin,
            },
        });
    }
    refinement_cycles(&mut rows);
    for i in 0..rows.scalars.len() {
        rows.scalars[i].kind = kind(&rows, i);
        if let Base::Scalar(p) = rows.scalars[i].written.base {
            rows.scalars[i].parent = Some(p);
        }
    }
    let mut state = vec![Settle::Open; rows.scalars.len()];
    for i in 0..rows.scalars.len() {
        settle(&mut rows, &mut state, i);
    }
    principals(&mut rows);
    rows
}

/// A range bound: an integer literal, with a leading minus allowed; the
/// bound's span when it is anything else.
fn bound(e: &Expr) -> Result<i128, Span> {
    match e {
        Expr::Literal(Literal::Int(n), _) => Ok(i128::from(*n)),
        Expr::Unary { op: UnaryOp::Neg, operand, span } => match operand.as_ref() {
            Expr::Literal(Literal::Int(n), _) => Ok(-i128::from(*n)),
            _ => Err(*span),
        },
        other => Err(other.span()),
    }
}

/// What a scalar declaration's base names. `depth` bounds an alias chain
/// (a cycle of aliases is the resolver's error).
fn base(te: &TypeExpr, names: &Names<'_>, depth: usize) -> Base {
    match te {
        TypeExpr::Primitive(PrimType::Int, _) => Base::Int,
        TypeExpr::Primitive(p, _) => Base::Primitive(*p),
        TypeExpr::Named { path, generic_args, .. } if path.segments.len() == 1 && generic_args.is_empty() => {
            let name = path.segments[0].name.as_str();
            if let Some(i) = names.scalars.get(name) {
                return Base::Scalar(*i);
            }
            let other = |what: &str| Base::Other { name: name.to_string(), what: what.to_string() };
            match names.types.get(name).map(|t| &t.body) {
                Some(TypeDeclBody::Struct(_)) => other("a struct"),
                Some(TypeDeclBody::Enum(_)) => other("an enum"),
                Some(TypeDeclBody::Alias(target)) if depth < 16 => match base(target, names, depth + 1) {
                    Base::Other { what, .. } => other(&format!("an alias of {what}")),
                    Base::Unknown(_) | Base::Cycle(_) => other("an alias of an undeclared type"),
                    resolved => resolved,
                },
                Some(_) => other("an alias"),
                None => match names.others.get(name) {
                    Some(what) => other(what),
                    None => Base::Unknown(name.to_string()),
                },
            }
        }
        other => Base::Other { name: crate::check::type_expr_text(other), what: other.form_name().to_string() },
    }
}

/// A refinement chain (declarations with no kind word, each over the
/// next) that comes back to itself: each declaration on the cycle gets
/// the cycle as its base.
fn refinement_cycles(rows: &mut UnitRows) {
    for start in 0..rows.scalars.len() {
        let mut path = vec![start];
        let mut at = start;
        loop {
            let row = &rows.scalars[at];
            let Base::Scalar(next) = row.written.base else { break };
            if row.written.kind.is_some() {
                break;
            }
            if let Some(from) = path.iter().position(|p| *p == next) {
                if from == 0 {
                    let mut names: Vec<String> = path.iter().map(|p| rows.scalars[*p].display.clone()).collect();
                    names.push(rows.scalars[start].display.clone());
                    rows.scalars[start].written.base = Base::Cycle(names);
                }
                break;
            }
            path.push(next);
            at = next;
        }
    }
}

/// A scalar's kind: its word's, else its base's (a refinement of a
/// quantity is a quantity, of a point a point; of an identity, a range
/// or `Int`, a range; `Int in D` is a quantity missing its word).
fn kind(rows: &UnitRows, i: usize) -> ScalarKindRow {
    let mut at = i;
    for _ in 0..=rows.scalars.len() {
        let w = &rows.scalars[at].written;
        match w.kind {
            Some(ScalarKind::Quantity) => return ScalarKindRow::Quantity,
            Some(ScalarKind::Point) => return ScalarKindRow::Point,
            Some(ScalarKind::Distinct) => return if at == i { ScalarKindRow::Identity } else { ScalarKindRow::Range },
            None => {}
        }
        match w.base {
            Base::Scalar(p) => at = p,
            Base::Int if w.denomination.is_some() => return ScalarKindRow::Quantity,
            _ => return ScalarKindRow::Range,
        }
    }
    ScalarKindRow::Range
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Settle {
    Open,
    Visiting,
    Done,
}

/// A quantity's or a point's denomination, component and quantity, its
/// parent's first; a range's parent; a point's origin as a count.
fn settle(rows: &mut UnitRows, state: &mut [Settle], i: usize) {
    if state[i] != Settle::Open {
        return;
    }
    state[i] = Settle::Visiting;
    let row = &rows.scalars[i];
    let parent = row.parent.filter(|p| matches!(rows.scalars[*p].kind, ScalarKindRow::Quantity | ScalarKindRow::Point));
    if let Some(p) = parent {
        settle(rows, state, p);
    }
    let row = &rows.scalars[i];
    let written = row.written.denomination.and_then(|d| {
        let unit = rows.refs[d.unit].unit?;
        let multiple = Ratio::new(BigInt::from(d.multiple), BigInt::from(1))?;
        Some((unit, Denom { unit: rows.units[unit].site.id, multiple }))
    });
    let (component, denomination, of, origin) = match row.kind {
        ScalarKindRow::Quantity | ScalarKindRow::Point => {
            let inherited = parent.map(|p| &rows.scalars[p]);
            let component =
                inherited.and_then(|p| p.component).or_else(|| written.as_ref().map(|(u, _)| rows.units[*u].component));
            let denomination =
                written.as_ref().map(|(_, d)| d.clone()).or_else(|| inherited.and_then(|p| p.denomination.clone()));
            let of = if row.kind == ScalarKindRow::Point { parent } else { None };
            let origin = row.written.origin.and_then(|o| {
                let unit = rows.refs[o.unit].unit?;
                let d = denomination.as_ref()?;
                let one = Denom { unit: rows.units[unit].site.id, multiple: Ratio::one() };
                let f = rows.catalogue.as_ref()?.factor(&one, d)?;
                Some(
                    BigRational::from_integer(BigInt::from(o.value))
                        * BigRational::new(f.numerator().clone(), f.denominator().clone()),
                )
            });
            (component, denomination, of, origin)
        }
        ScalarKindRow::Range => (None, None, row.parent, None),
        ScalarKindRow::Identity => (None, None, None, None),
    };
    let row = &mut rows.scalars[i];
    (row.component, row.denomination, row.of, row.origin) = (component, denomination, of, origin);
    state[i] = Settle::Done;
}

/// Each component's quantity: the first declared with `quantity` and no
/// policy (law 4 refuses a second); every other quantity over the
/// component records it as its `of`.
fn principals(rows: &mut UnitRows) {
    let mut principal: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, s) in rows.scalars.iter_mut().enumerate() {
        if let (Some(c), true) = (s.component, is_principal_candidate(s)) {
            if !principal.contains_key(&c) {
                principal.insert(c, i);
                s.principal = true;
            }
        }
    }
    for s in rows.scalars.iter_mut().filter(|s| s.kind == ScalarKindRow::Quantity && !s.principal) {
        s.of = s.component.and_then(|c| principal.get(&c).copied());
    }
}

/// Declared with `quantity` and no policy: a component's quantity, or a
/// second one law 4 refuses.
fn is_principal_candidate(s: &ScalarRow) -> bool {
    s.written.kind == Some(ScalarKind::Quantity) && s.written.round.is_none()
}

/// The units' components: the equations whose target is declared join
/// their two units, and the number one joins every unit written against
/// a number.
fn components(rows: &mut UnitRows) {
    let n = rows.units.len();
    let mut parent: Vec<usize> = (0..=n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for e in &rows.equations {
        let to = match e.target {
            Some(Node::Unit(j)) => j,
            Some(Node::Pure) => n,
            None => continue,
        };
        let (a, b) = (find(&mut parent, e.unit), find(&mut parent, to));
        parent[a] = b;
    }
    let mut lowest: BTreeMap<usize, usize> = BTreeMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        lowest.entry(root).or_insert(i);
    }
    let pure = find(&mut parent, n);
    for i in 0..n {
        let root = find(&mut parent, i);
        rows.units[i].component = lowest[&root];
        rows.units[i].dimensionless = root == pure;
    }
}

/// Close the catalogue over the units and the equations whose target is
/// declared. It is kept only when no catalogue law fails.
fn close(rows: &mut UnitRows, by_name: &BTreeMap<&str, usize>) {
    let key = |node: Node| match node {
        Node::Unit(j) => rows.units[j].site.id,
        Node::Pure => PURE_NUMBER,
    };
    let units = rows.units.iter().map(|u| u.site.id).chain([PURE_NUMBER]);
    let equations: Vec<Equation> = rows
        .equations
        .iter()
        .filter_map(|e| {
            Some(Equation {
                site: e.site.id,
                from: rows.units[e.unit].site.id,
                to: key(e.target?),
                factor: e.factor.clone(),
            })
        })
        .collect();
    let complete = by_name.len() == rows.units.len() && rows.equations.iter().all(|e| e.target.is_some());
    match UnitGraph::close(units, equations) {
        Ok(graph) => rows.catalogue = complete.then_some(graph),
        Err(errors) => rows.cycles = errors,
    }
}

/// Every unit-dialect law over `rows`, as diagnostics, law by law.
pub fn unit_laws(rows: &UnitRows) -> Vec<Diag> {
    let laws: [Law<UnitRows>; 10] = [
        Law { rule: DECLARED_ONCE, eval: declared_twice },
        Law { rule: DECLARED_UNIT, eval: undeclared_units },
        Law { rule: CYCLES, eval: inconsistent_cycles },
        Law { rule: ONE_QUANTITY, eval: second_quantity },
        Law { rule: COUNTS_AN_INT, eval: quantity_shape },
        Law { rule: POINT, eval: point_shape },
        Law { rule: CLAUSES, eval: clauses },
        Law { rule: BASES, eval: identity_and_range_bases },
        Law { rule: REFINEMENT, eval: refinement_bases },
        Law { rule: DURATION_NAME, eval: duration_suffix_names },
    ];
    laws.iter().flat_map(|law| law.diags(rows)).collect()
}

/// The one error a value of the unit dialect gets until values are typed
/// (the next step deletes this function and its callers in the check): a
/// scalar type's name where a value would live, or a quantity literal.
/// `what` names it (``type `Money` ``, ``quantity literal `3bp` ``).
pub(crate) fn value_not_yet(span: Span, what: &str) -> Diag {
    Diag::ty(
        span,
        format!(
            "{what}: values of the unit dialect's types are not typed yet (GH #1076): declarations are \
             checked, and values arrive with the next step; until then, count in `Int`"
        ),
    )
}

/// Law 1: a unit is declared once. At the second declaration's name; the
/// witness is the first.
fn declared_twice(rows: &UnitRows, out: &mut Vec<Violation>) {
    for (i, u) in rows.units.iter().enumerate() {
        let Some(first) = rows.units[..i].iter().find(|f| f.name == u.name) else { continue };
        out.push(
            Violation::error(
                DECLARED_ONCE,
                u.name_span,
                format!(
                    "unit `{}` is declared twice: a unit is one node of the catalogue, so it is declared once, \
                     with at most one equation; remove this declaration or give the unit another name",
                    u.name
                ),
            )
            .step(first.name_span, format!("`{}` is first declared here", u.name)),
        );
    }
}

/// Who names a unit, for a message.
fn named_by(rows: &UnitRows, at: RefAt) -> (String, &'static str) {
    match at {
        RefAt::Equation(e) => (format!("unit `{}`", rows.units[rows.equations[e].unit].name), "its equation"),
        RefAt::Denomination(s) => (format!("type `{}`", rows.scalars[s].display), "its denomination"),
        RefAt::Origin(s) => (format!("type `{}`", rows.scalars[s].display), "its origin"),
    }
}

/// Law 2: an equation, a denomination or an origin names a declared
/// unit. At the name, suggesting the nearest declared one.
fn undeclared_units(rows: &UnitRows, out: &mut Vec<Violation>) {
    for r in rows.refs.iter().filter(|r| r.unit.is_none()) {
        let (who, role) = named_by(rows, r.at);
        let message = if DURATION_SUFFIXES.contains(&r.name.as_str()) {
            format!(
                "{who}: {role} names `{}`, a built-in duration suffix and no unit: no `unit` may take that name \
                 until `Time` and `Duration` are declarations (GH #1076); declare a unit of another name (`{}`)",
                r.name,
                own_name(&r.name)
            )
        } else {
            let suggestion = nearest_name(&r.name, rows.units.iter().map(|u| u.name.as_str()))
                .map(|n| format!("; did you mean `{n}`?"))
                .unwrap_or_default();
            format!(
                "{who}: {role} names `{}`, which no `unit` declares: declare it (`unit {};`){suggestion}",
                r.name, r.name
            )
        };
        out.push(Violation::error(DECLARED_UNIT, r.span, message));
    }
}

/// `one `pct``, or `the number one`.
fn one(rows: &UnitRows, node: Node) -> String {
    match node {
        Node::Unit(j) => format!("one `{}`", rows.units[j].name),
        Node::Pure => "the number one".to_string(),
    }
}

/// `30 `ms``, or `1/100` (of the number one).
fn amount(rows: &UnitRows, factor: &Ratio, node: Node) -> String {
    match node {
        Node::Unit(j) => format!("{factor} `{}`", rows.units[j].name),
        Node::Pure => format!("{factor}"),
    }
}

/// An equation as its declaration states it: `unit us = 1000 ns`.
fn equation_text(rows: &UnitRows, e: &EquationRow) -> String {
    let target = match e.target {
        Some(Node::Unit(j)) => format!(" {}", rows.units[j].name),
        _ => String::new(),
    };
    format!("unit {} = {}{target}", rows.units[e.unit].name, e.factor)
}

/// Law 3: the catalogue closes, every cycle multiplying to one. At the
/// equation the closure found inconsistent, stating what it claims; the
/// witness is the other path, each step at its declaration.
fn inconsistent_cycles(rows: &UnitRows, out: &mut Vec<Violation>) {
    let by_site: BTreeMap<SiteId, usize> = rows.equations.iter().enumerate().map(|(i, e)| (e.site.id, i)).collect();
    for err in &rows.cycles {
        let CatalogueError::InconsistentCycle { equation, claimed, implied, cycle } = err else { continue };
        let Some(blamed) = by_site.get(equation).map(|i| &rows.equations[*i]) else { continue };
        let Some(target) = blamed.target else { continue };
        let from = Node::Unit(blamed.unit);
        let mut v = Violation::error(
            CYCLES,
            blamed.span,
            format!(
                "unit `{}`: `{}` makes {} {}, and the other equations make it {}: every cycle of equations \
                 multiplies to one, so one equation on this cycle is wrong; correct it (to `{}` if it is this one)",
                rows.units[blamed.unit].name,
                equation_text(rows, blamed),
                one(rows, from),
                amount(rows, claimed, target),
                amount(rows, implied, target),
                replacement(rows, blamed, implied),
            ),
        );
        // The other path: every step of the cycle but the last, which is
        // the blamed equation walked back.
        for step in &cycle[..cycle.len().saturating_sub(1)] {
            let Some(e) = by_site.get(&step.equation).map(|i| &rows.equations[*i]) else { continue };
            let Some(t) = e.target else { continue };
            let (a, b, f) = if step.reversed {
                (t, Node::Unit(e.unit), e.factor.reciprocal())
            } else {
                (Node::Unit(e.unit), t, e.factor.clone())
            };
            let decl = rows.units[e.unit].span;
            v = v.step(decl, format!("`{}`: {} is {}", equation_text(rows, e), one(rows, a), amount(rows, &f, b)));
        }
        out.push(v);
    }
}

/// The blamed equation, corrected to what the other path implies.
fn replacement(rows: &UnitRows, e: &EquationRow, implied: &Ratio) -> String {
    let target = match e.target {
        Some(Node::Unit(j)) => format!(" {}", rows.units[j].name),
        _ => String::new(),
    };
    format!("unit {} = {implied}{target};", rows.units[e.unit].name)
}

/// A denomination as written: `cent`, `100 ms`.
fn denom_text(rows: &UnitRows, d: &WrittenDenom) -> String {
    let unit = &rows.refs[d.unit].name;
    if d.multiple == 1 {
        unit.clone()
    } else {
        format!("{} {unit}", d.multiple)
    }
}

/// Law 4: one quantity per component. A second quantity declared with no
/// policy is refused at its name, the component's quantity its witness;
/// and a refinement of a quantity is denominated in its parent's
/// component.
fn second_quantity(rows: &UnitRows, out: &mut Vec<Violation>) {
    for s in &rows.scalars {
        if s.kind != ScalarKindRow::Quantity {
            continue;
        }
        if is_principal_candidate(s) && !s.principal {
            let Some(c) = s.component else { continue };
            let Some(p) = rows.scalars.iter().find(|p| p.principal && p.component == Some(c)) else { continue };
            let Some(d) = s.written.denomination else { continue };
            let denom = denom_text(rows, &d);
            out.push(
                Violation::error(
                    ONE_QUANTITY,
                    s.name_span,
                    format!(
                        "type `{}`: the units of `{}` already have their quantity, `{}`: a component of the \
                         catalogue has one quantity, and every other type over it is a denomination of that one; \
                         write `type {} = {} in {denom};`, or give it a `round:` policy",
                        s.display, rows.refs[d.unit].name, p.display, s.display, p.display
                    ),
                )
                .step(p.name_span, format!("`{}` is that component's quantity, declared with no policy", p.display)),
            );
        }
        // A refinement written over a quantity, in a unit of another
        // component.
        let (Some(parent), None) = (s.parent, s.written.kind) else { continue };
        let Some(d) = s.written.denomination else { continue };
        let Some(unit) = rows.refs[d.unit].unit else { continue };
        let p = &rows.scalars[parent];
        if p.kind != ScalarKindRow::Quantity || p.component.is_none() || p.component == Some(rows.units[unit].component)
        {
            continue;
        }
        out.push(Violation::error(
            ONE_QUANTITY,
            rows.refs[d.unit].span,
            format!(
                "type `{}`: `{}` is not a unit of `{}`'s component, so `{}` cannot be denominated in it: a \
                 refinement of a quantity is a denomination of that quantity",
                s.display,
                rows.units[unit].name,
                p.display,
                s.display
            ),
        ));
    }
}

/// Law 5: a quantity counts an `Int` and names its denomination.
fn quantity_shape(rows: &UnitRows, out: &mut Vec<Violation>) {
    for s in &rows.scalars {
        let w = &s.written;
        let in_text = w.denomination.map(|d| denom_text(rows, &d)).unwrap_or_else(|| "<unit>".to_string());
        match w.kind {
            Some(ScalarKind::Quantity) => {
                if w.base != Base::Int && !w.base.unresolved() {
                    out.push(Violation::error(
                        COUNTS_AN_INT,
                        w.base_span,
                        format!(
                            "type `{}`: a quantity counts an `Int`, and {} is not one: write `quantity Int in \
                             {in_text}`",
                            s.display,
                            w.base.spelled(rows)
                        ),
                    ));
                }
                if w.denomination.is_none() {
                    out.push(Violation::error(
                        COUNTS_AN_INT,
                        s.name_span,
                        format!(
                            "type `{}`: a quantity names its denomination, the unit it counts: write \
                             `quantity Int in <unit>` with a declared unit",
                            s.display
                        ),
                    ));
                }
            }
            None if w.base == Base::Int && w.denomination.is_some() => out.push(Violation::error(
                COUNTS_AN_INT,
                w.base_span,
                format!(
                    "type `{}`: `Int in {in_text}` is a quantity without its word: write `quantity Int in {in_text}`",
                    s.display
                ),
            )),
            _ => {}
        }
    }
}

/// Law 6: a point is over a quantity (or refines a point), an `origin:`
/// is a point's, and a point's denomination and origin are units of its
/// quantity's component.
fn point_shape(rows: &UnitRows, out: &mut Vec<Violation>) {
    for s in &rows.scalars {
        let w = &s.written;
        if w.kind == Some(ScalarKind::Point) && !w.base.unresolved() {
            let over_a_quantity = matches!(w.base, Base::Scalar(p) if rows.scalars[p].kind == ScalarKindRow::Quantity);
            if !over_a_quantity {
                let hint = match w.base {
                    Base::Scalar(p) if rows.scalars[p].kind == ScalarKindRow::Point => format!(
                        "; to refine the point, drop the word: `type {} = {} in <unit> {{ … }}`",
                        s.display, rows.scalars[p].display
                    ),
                    _ => String::new(),
                };
                out.push(Violation::error(
                    POINT,
                    w.base_span,
                    format!(
                        "type `{}`: a point is over a quantity, and {} is {}{hint}",
                        s.display,
                        w.base.spelled(rows),
                        w.base.what(rows)
                    ),
                ));
            }
        }
        if let Some(o) = w.origin {
            if s.kind != ScalarKindRow::Point {
                out.push(Violation::error(
                    POINT,
                    o.span,
                    format!(
                        "type `{}`: `origin:` places a point's zero, and `{}` is {}, not a point: remove it",
                        s.display,
                        s.display,
                        s.kind.article()
                    ),
                ));
            }
        }
        if s.kind != ScalarKindRow::Point {
            continue;
        }
        let Some(component) = s.component else { continue };
        let of_name = s.of.map(|q| rows.scalars[q].display.as_str()).unwrap_or(s.display.as_str());
        if let Some(d) = w.denomination {
            if let Some(unit) = rows.refs[d.unit].unit {
                if rows.units[unit].component != component {
                    out.push(Violation::error(
                        POINT,
                        rows.refs[d.unit].span,
                        format!(
                            "type `{}`: `{}` is not a unit of `{of_name}`'s component, so the point cannot be \
                             denominated in it: a point is denominated in a unit of its quantity",
                            s.display, rows.units[unit].name
                        ),
                    ));
                }
            }
        }
        if let Some(o) = w.origin {
            if let Some(unit) = rows.refs[o.unit].unit {
                if rows.units[unit].component != component {
                    out.push(Violation::error(
                        POINT,
                        rows.refs[o.unit].span,
                        format!(
                            "type `{}`: the origin is in `{}`, which is not a unit of `{of_name}`'s component: an \
                             origin is a count of the point's own quantity",
                            s.display, rows.units[unit].name
                        ),
                    ));
                }
            }
        }
    }
}

/// The policies, as a message lists them.
fn policies() -> String {
    let names: Vec<String> = RoundPolicy::ALL.iter().map(|p| format!("`{}`", p.name())).collect();
    format!("{} or {}", names[..names.len() - 1].join(", "), names[names.len() - 1])
}

/// The nearest ancestor of `i` that states a range, and its range.
fn parent_range(rows: &UnitRows, i: usize) -> Option<(usize, (i128, i128))> {
    let mut at = rows.scalars[i].parent;
    for _ in 0..rows.scalars.len() {
        let p = at?;
        if let Some(r) = rows.scalars[p].range {
            return Some((p, r));
        }
        at = rows.scalars[p].parent;
    }
    None
}

/// Law 7: `round:` names one of the five policies, on a quantity or a
/// point; a `range:`'s bounds are integer literals spanning at least one
/// value, inside its parent's range.
fn clauses(rows: &UnitRows, out: &mut Vec<Violation>) {
    for (i, s) in rows.scalars.iter().enumerate() {
        let w = &s.written;
        if let Some((name, span)) = &w.round {
            if RoundPolicy::of(name).is_none() {
                let suggestion = nearest_name(name, RoundPolicy::ALL.iter().map(|p| p.name()))
                    .map(|n| format!("; did you mean `{n}`?"))
                    .unwrap_or_default();
                out.push(Violation::error(
                    CLAUSES,
                    *span,
                    format!(
                        "type `{}`: `{name}` is not a rounding policy: `round:` names {}{suggestion}",
                        s.display,
                        policies()
                    ),
                ));
            } else if !matches!(s.kind, ScalarKindRow::Quantity | ScalarKindRow::Point) {
                out.push(Violation::error(
                    CLAUSES,
                    *span,
                    format!(
                        "type `{}`: `round:` is how a narrowing into a quantity or a point discards, and `{}` is {}: \
                         remove it",
                        s.display,
                        s.display,
                        s.kind.article()
                    ),
                ));
            }
        }
        let Some(r) = w.range else { continue };
        let (lo, hi) = match r.bounds {
            Err(at) => {
                out.push(Violation::error(
                    CLAUSES,
                    at,
                    format!(
                        "type `{}`: a bound of `range:` is an integer literal (`0`, `-5`, `1_000`), and this one is not",
                        s.display
                    ),
                ));
                continue;
            }
            Ok(b) => b,
        };
        if lo >= hi {
            out.push(Violation::error(
                CLAUSES,
                r.span,
                format!(
                    "type `{}`: `range: {lo}..{hi}` holds no value: its lower bound is below its upper (and `..=` \
                     includes the upper)",
                    s.display
                ),
            ));
            continue;
        }
        let Some((p, (plo, phi))) = parent_range(rows, i) else { continue };
        if plo <= lo && hi <= phi {
            continue;
        }
        let mut v = Violation::error(
            CLAUSES,
            r.span,
            format!(
                "type `{}`: `range: {lo}..{hi}` is not inside `{}`'s `{plo}..{phi}`: a refinement narrows its \
                 parent's range, never widens it",
                s.display, rows.scalars[p].display
            ),
        );
        if let Some(pr) = rows.scalars[p].written.range {
            v = v.step(pr.span, format!("`{}`'s range is `{plo}..{phi}`", rows.scalars[p].display));
        }
        out.push(v);
    }
}

/// Law 8: an identity is `distinct Int`, with no denomination; a range
/// type refines `Int`, an identity or another range, states its range
/// over `Int`, and has no denomination.
fn identity_and_range_bases(rows: &UnitRows, out: &mut Vec<Violation>) {
    for s in &rows.scalars {
        let w = &s.written;
        match (w.kind, s.kind) {
            (Some(ScalarKind::Distinct), _) => {
                if w.base != Base::Int && !w.base.unresolved() {
                    out.push(Violation::error(
                        BASES,
                        w.base_span,
                        format!(
                            "type `{}`: an identity is `distinct Int`, and {} is not `Int`: write `distinct Int`",
                            s.display,
                            w.base.spelled(rows)
                        ),
                    ));
                }
                if let Some(d) = w.denomination {
                    out.push(Violation::error(
                        BASES,
                        d.span,
                        format!(
                            "type `{}`: an identity counts nothing, so it has no denomination: remove `in {}`",
                            s.display,
                            denom_text(rows, &d)
                        ),
                    ));
                }
            }
            (None, ScalarKindRow::Range) => match &w.base {
                Base::Primitive(_) => out.push(Violation::error(
                    BASES,
                    w.base_span,
                    format!(
                        "type `{}`: a range type refines `Int`, an identity or another range, and {} is none of them",
                        s.display,
                        w.base.spelled(rows)
                    ),
                )),
                Base::Int | Base::Scalar(_) => {
                    // `Int in D` is a quantity (law 5), so a range here is
                    // over an identity or a range.
                    if let Some(d) = w.denomination {
                        out.push(Violation::error(
                            BASES,
                            d.span,
                            format!(
                                "type `{}`: a range counts nothing, so it has no denomination: remove `in {}`",
                                s.display,
                                denom_text(rows, &d)
                            ),
                        ));
                    }
                    if w.base == Base::Int && w.range.is_none() {
                        out.push(Violation::error(
                            BASES,
                            s.name_span,
                            format!(
                                "type `{}`: a refinement of `Int` is a range, and states it: write `{{ range: \
                                 LO..HI; }}`, or `type {} = Int;` for another name of `Int`",
                                s.display, s.display
                            ),
                        ));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// Law 9: a declaration's base is declared, and a refinement (no kind
/// word) refines a quantity, a point, an identity, a range or `Int`;
/// never a struct, an enum, an alias of one, or itself.
fn refinement_bases(rows: &UnitRows, out: &mut Vec<Violation>) {
    for s in &rows.scalars {
        let w = &s.written;
        match &w.base {
            Base::Unknown(name) => {
                let mut candidates: Vec<&str> = rows.scalars.iter().map(|r| r.display.as_str()).collect();
                candidates.push("Int");
                let suggestion =
                    nearest_name(name, candidates).map(|n| format!("; did you mean `{n}`?")).unwrap_or_default();
                out.push(Violation::error(
                    REFINEMENT,
                    w.base_span,
                    format!("type `{}`: `{name}` is not a declared type{suggestion}", s.display),
                ));
            }
            Base::Cycle(names) => out.push(Violation::error(
                REFINEMENT,
                w.base_span,
                format!(
                    "type `{}` refines itself ({}): a chain of refinements ends at a quantity, a point, an identity \
                     or `Int`",
                    s.display,
                    names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(" refines ")
                ),
            )),
            Base::Other { name, what } if w.kind.is_none() => out.push(Violation::error(
                REFINEMENT,
                w.base_span,
                format!(
                    "type `{}`: a refinement refines a quantity, a point, an identity, a range or `Int` (with a \
                     `range:`), and `{name}` is {what}",
                    s.display
                ),
            )),
            _ => {}
        }
    }
}

/// Law 10: no unit takes a built-in duration suffix's name, which the
/// lexer reads as a duration literal (`5ms`) whatever is declared.
fn duration_suffix_names(rows: &UnitRows, out: &mut Vec<Violation>) {
    for u in rows.units.iter().filter(|u| DURATION_SUFFIXES.contains(&u.name.as_str())) {
        out.push(Violation::error(
            DURATION_NAME,
            u.name_span,
            format!(
                "unit `{}`: `{}` is a built-in duration suffix, so `5{}` is a `Duration` literal and never this \
                 unit: no `unit` may take one of {} until `Time` and `Duration` are declarations (GH #1076); name \
                 it otherwise (`{}`)",
                u.name,
                u.name,
                u.name,
                DURATION_SUFFIXES.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", "),
                own_name(&u.name)
            ),
        ));
    }
}

/// A name a program may give the unit a duration suffix spells.
fn own_name(suffix: &str) -> &'static str {
    match suffix {
        "ns" => "nsec",
        "us" => "usec",
        "ms" => "msec",
        "s" => "sec",
        "m" => "min",
        "h" => "hr",
        _ => "day",
    }
}
