//! Resolved types — what the type checker reasons about.
//!
//! `Ty` is the post-resolution form. The parser produces
//! [`aperio_syntax::ast::TypeExpr`] (a syntactic shape with
//! qualified-name lookups and generic-arg expressions); the
//! resolver turns that into `Ty` by looking each name up in the
//! enclosing scope.
//!
//! Milestone-2 cut: parametric / generic types are not fully
//! resolved (we record the args but don't substitute). Stdlib
//! types referenced via paths we can't load (`time::Time` from
//! the `std/time` import) resolve to `Ty::Unknown` rather than
//! erroring — milestone-3 will tighten this.

use aperio_syntax::ast::{PrimType, ProjectionClass};

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Prim(PrimType),
    Named(String),
    Projection(ProjectionClass, Box<Ty>),
    Array(Box<Ty>, Option<u64>),
    Tuple(Vec<Ty>),
    Function {
        params: Vec<Ty>,
        ret: Box<Ty>,
    },
    /// Unit / no value. The implicit return type of statements
    /// and functions without `-> ...`.
    Unit,
    /// v1.x-FORM-1: the result type of a call to a `fallible(E)`
    /// function. Models "either a success value of type T, or
    /// an error has occurred carrying a payload of type E."
    ///
    /// A `Ty::Fallible` is NOT assignable to its success type —
    /// the caller MUST address the error first, via an
    /// `or`-disposition or a `match`. The typechecker rejects
    /// bare consumption of fallible values with
    /// `error: error not addressed`. `Expr::Or` unwraps a
    /// fallible into its success type.
    Fallible {
        success: Box<Ty>,
        payload: Box<Ty>,
    },
    /// External or not-yet-resolved. Compatible with anything
    /// in milestone 2 — the checker is permissive about names
    /// it can't see (e.g., stdlib paths).
    Unknown,
}

impl Ty {
    pub fn display(&self) -> String {
        match self {
            Ty::Prim(p) => prim_name(*p).to_string(),
            Ty::Named(n) => n.clone(),
            Ty::Projection(c, inner) => {
                let cn = match c {
                    ProjectionClass::Rich => "Rich",
                    ProjectionClass::Chunked => "Chunked",
                    ProjectionClass::Recognition(_) => "Recognition",
                };
                format!("{}<{}>", cn, inner.display())
            }
            Ty::Array(elem, size) => match size {
                Some(n) => format!("[{}; {}]", elem.display(), n),
                None => format!("[{}]", elem.display()),
            },
            Ty::Tuple(parts) => {
                let body: Vec<String> = parts.iter().map(|t| t.display()).collect();
                format!("({})", body.join(", "))
            }
            Ty::Function { params, ret } => {
                let body: Vec<String> = params.iter().map(|t| t.display()).collect();
                format!("fn({}) -> {}", body.join(", "), ret.display())
            }
            Ty::Unit => "()".to_string(),
            Ty::Fallible { success, payload } => {
                format!("{} fallible({})", success.display(), payload.display())
            }
            Ty::Unknown => "?".to_string(),
        }
    }

    /// Is `self` assignable from `other`? Milestone-2 rule:
    /// `Unknown` is bidirectionally compatible (recursively
    /// through composite types); otherwise structural equality.
    ///
    /// The recursive Unknown-permissiveness lets an interface-
    /// typed slot accept a satisfying locus inside composite
    /// shapes — `let arr: [Greeter; 2] = [Hi {}, Hey {}];`
    /// works because Greeter resolves to Unknown
    /// (`collect_known_names` omits Interface decls today), and
    /// the per-element Unknown ≈ Hi check now passes. Codegen
    /// emits the per-element fat-pointer coercion at the
    /// destination's known type. G20 (2026-05-23).
    pub fn assignable_from(&self, other: &Ty) -> bool {
        if matches!(self, Ty::Unknown) || matches!(other, Ty::Unknown) {
            return true;
        }
        match (self, other) {
            (Ty::Array(a_elem, a_n), Ty::Array(b_elem, b_n)) if a_n == b_n => {
                a_elem.assignable_from(b_elem)
            }
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                a.iter().zip(b.iter()).all(|(x, y)| x.assignable_from(y))
            }
            (Ty::Fallible { success: a_s, payload: a_p },
             Ty::Fallible { success: b_s, payload: b_p }) => {
                a_s.assignable_from(b_s) && a_p.assignable_from(b_p)
            }
            (Ty::Projection(a_c, a_inner), Ty::Projection(b_c, b_inner)) if a_c == b_c => {
                a_inner.assignable_from(b_inner)
            }
            _ => self == other,
        }
    }
}

pub fn prim_name(p: PrimType) -> &'static str {
    match p {
        PrimType::Int => "Int",
        PrimType::Uint => "Uint",
        PrimType::Float => "Float",
        PrimType::Decimal => "Decimal",
        PrimType::String => "String",
        PrimType::Bool => "Bool",
        PrimType::Time => "Time",
        PrimType::Duration => "Duration",
        PrimType::Bytes => "Bytes",
        PrimType::BytesView => "BytesView",
        PrimType::StringView => "StringView",
    }
}

/// Form K (2026-05-20): does `ty` have a fixed, statically-known
/// memory layout suitable for zero-copy bus payload routing?
///
/// True iff every leaf is a fixed-size primitive (Int, Uint,
/// Float, Decimal, Bool, Time, Duration), Unit, a fixed-size
/// array of flat-shapeable, a tuple of flat-shapeables, or a
/// named struct whose every field is flat-shapeable.
///
/// False for String, Bytes, BytesView, StringView (heap-shaped
/// / fat-pointer; their backing storage isn't part of the
/// value's own layout), `Array(_, None)` (unbounded),
/// `Fallible`/`Function`/`Projection` (not valid bus payloads
/// anyway), and `Unknown` (conservative — the predicate cannot
/// assert flatness for a type it can't see).
///
/// Used by Form K's route-selection matrix and the `zero_copy`
/// binding constraint (rejects bindings whose topic payloads
/// don't satisfy this predicate). Pure: no side effects on
/// `scope`.
pub fn is_flat_shapeable(ty: &Ty, scope: &crate::resolve::TopScope) -> bool {
    let mut seen: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    is_flat_shapeable_inner(ty, scope, &mut seen)
}

fn is_flat_shapeable_inner(
    ty: &Ty,
    scope: &crate::resolve::TopScope,
    seen: &mut std::collections::BTreeSet<String>,
) -> bool {
    match ty {
        Ty::Prim(p) => match p {
            PrimType::Int
            | PrimType::Uint
            | PrimType::Float
            | PrimType::Decimal
            | PrimType::Bool
            | PrimType::Time
            | PrimType::Duration => true,
            PrimType::String
            | PrimType::Bytes
            | PrimType::BytesView
            | PrimType::StringView => false,
        },
        Ty::Unit => true,
        Ty::Array(elem, Some(_)) => is_flat_shapeable_inner(elem, scope, seen),
        Ty::Array(_, None) => false,
        Ty::Tuple(parts) => parts
            .iter()
            .all(|p| is_flat_shapeable_inner(p, scope, seen)),
        Ty::Named(name) => {
            // Cycle guard: a struct cannot transitively contain
            // itself by value (codegen would reject earlier), so
            // a re-entry on the same name is either a bug or an
            // alias chain. Treat as "not flat" conservatively
            // rather than recursing into a loop.
            if !seen.insert(name.clone()) {
                return false;
            }
            let res = match scope.lookup(name) {
                Some(crate::symbol::TopSymbol::Type(info)) => match &info.kind
                {
                    crate::symbol::TypeKind::Struct(fields) => fields
                        .iter()
                        .all(|f| is_flat_shapeable_inner(&f.ty, scope, seen)),
                    crate::symbol::TypeKind::Alias(inner) => {
                        is_flat_shapeable_inner(inner, scope, seen)
                    }
                    // Enums are not currently shipped as bus
                    // payloads on the flat path; treat as not
                    // flat until/unless we add a discriminator
                    // layout story.
                    crate::symbol::TypeKind::Enum(_) => false,
                },
                // Unknown name / non-Type symbol → cannot assert
                // flatness.
                _ => false,
            };
            seen.remove(name);
            res
        }
        // Loci-as-types, fallible, functions, unknown: not
        // legal bus payload shapes (or unknown — conservative).
        Ty::Projection(_, _)
        | Ty::Function { .. }
        | Ty::Fallible { .. }
        | Ty::Unknown => false,
    }
}
