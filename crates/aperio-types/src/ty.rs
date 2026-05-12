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
    /// `Unknown` is bidirectionally compatible; otherwise
    /// structural equality.
    pub fn assignable_from(&self, other: &Ty) -> bool {
        if matches!(self, Ty::Unknown) || matches!(other, Ty::Unknown) {
            return true;
        }
        self == other
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
    }
}
