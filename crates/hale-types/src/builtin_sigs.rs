//! The bare builtins' signatures (F.40 phase 3, E4): `len`,
//! `to_string`, the `Int` / `Float` casts, `abs` / `min` / `max` and
//! `starts_with` / `contains`, the value builtins lowering answers by
//! name in `lower_expr`.
//!
//! One table, two readers. The checker types a call by it, so
//! `first(len(s))` binds `T` to `Int` instead of leaving the generic
//! call a hole; lowering reads each builtin's arity and result from the
//! same row. The table is lowering's inference written down, not a new
//! judgment: [`BuiltinSig::result`] answers a type only for the operand
//! types lowering lowers, and `None` everywhere lowering refuses, so the
//! checker keeps the call `Unknown` there and its diagnostics (or their
//! absence) stay what they were.
//!
//! The accumulator vocabulary (`count()` / `mean(x)` inside a closure
//! assertion) is not here: its types are the accumulator column's.

use crate::ty::Ty;
use hale_syntax::ast::PrimType;

/// The operand types a builtin lowers, by class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operands {
    /// `len`: a String or Bytes, a view of either, or a fixed-size
    /// array.
    Lengthed,
    /// `to_string`: a printable value (the checker's printable set,
    /// in lockstep with lowering's `value_to_string_supports`).
    Printable,
    /// `Int(x)` / `Float(x)`: an Int or a Float.
    IntOrFloat,
    /// `abs` / `min` / `max`: Int, Float, Duration or Decimal, every
    /// operand the same type.
    Numeric,
    /// `starts_with` / `contains`: Strings.
    Strings,
}

/// What a builtin call types as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Returns {
    /// Always this primitive.
    Prim(PrimType),
    /// The operands' own type (`abs` / `min` / `max`).
    Operand,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuiltinSig {
    pub name: &'static str,
    pub arity: usize,
    pub operands: Operands,
    pub returns: Returns,
}

pub const BARE_BUILTIN_SIGS: &[BuiltinSig] = &[
    BuiltinSig { name: "len", arity: 1, operands: Operands::Lengthed, returns: Returns::Prim(PrimType::Int) },
    BuiltinSig {
        name: "to_string",
        arity: 1,
        operands: Operands::Printable,
        returns: Returns::Prim(PrimType::String),
    },
    BuiltinSig { name: "Int", arity: 1, operands: Operands::IntOrFloat, returns: Returns::Prim(PrimType::Int) },
    BuiltinSig {
        name: "Float",
        arity: 1,
        operands: Operands::IntOrFloat,
        returns: Returns::Prim(PrimType::Float),
    },
    BuiltinSig { name: "abs", arity: 1, operands: Operands::Numeric, returns: Returns::Operand },
    BuiltinSig { name: "min", arity: 2, operands: Operands::Numeric, returns: Returns::Operand },
    BuiltinSig { name: "max", arity: 2, operands: Operands::Numeric, returns: Returns::Operand },
    BuiltinSig {
        name: "starts_with",
        arity: 2,
        operands: Operands::Strings,
        returns: Returns::Prim(PrimType::Bool),
    },
    BuiltinSig {
        name: "contains",
        arity: 2,
        operands: Operands::Strings,
        returns: Returns::Prim(PrimType::Bool),
    },
];

/// The row for a bare builtin callee, if `name` is one.
pub fn bare_builtin_sig(name: &str) -> Option<&'static BuiltinSig> {
    BARE_BUILTIN_SIGS.iter().find(|s| s.name == name)
}

impl BuiltinSig {
    /// The call's type over its operands' types: the type lowering
    /// gives the call when it lowers it, `None` where it refuses (a
    /// wrong arity, an operand outside the class, `min` / `max` over
    /// two different types). An `Unknown` operand is admitted where
    /// the result does not depend on it; where it does (`abs` /
    /// `min` / `max`) the call stays `None`.
    ///
    /// `printable` is the checker's printable rule, which needs the
    /// program's declarations.
    pub fn result(&self, args: &[Ty], printable: impl Fn(&Ty) -> bool) -> Option<Ty> {
        if args.len() != self.arity {
            return None;
        }
        let admits = |t: &Ty| match (self.operands, t) {
            (_, Ty::Unknown) => true,
            (Operands::Lengthed, Ty::Prim(p)) => matches!(
                p,
                PrimType::String | PrimType::StringView | PrimType::Bytes | PrimType::BytesView
            ),
            (Operands::Lengthed, Ty::Array(_, Some(_))) => true,
            (Operands::Printable, t) => printable(t),
            (Operands::IntOrFloat, Ty::Prim(p)) => matches!(p, PrimType::Int | PrimType::Float),
            (Operands::Numeric, Ty::Prim(p)) => {
                matches!(p, PrimType::Int | PrimType::Float | PrimType::Duration | PrimType::Decimal)
            }
            (Operands::Strings, Ty::Prim(PrimType::String)) => true,
            _ => false,
        };
        if !args.iter().all(admits) {
            return None;
        }
        match self.returns {
            Returns::Prim(p) => Some(Ty::Prim(p)),
            Returns::Operand => {
                let first = &args[0];
                (!matches!(first, Ty::Unknown) && args.iter().all(|t| t == first)).then(|| first.clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(p: PrimType) -> Ty {
        Ty::Prim(p)
    }

    #[test]
    fn the_rows_answer_lowerings_types() {
        let sig = |n| bare_builtin_sig(n).unwrap();
        let any = |_: &Ty| true;
        assert_eq!(sig("len").result(&[p(PrimType::String)], any), Some(p(PrimType::Int)));
        assert_eq!(sig("len").result(&[Ty::Array(Box::new(p(PrimType::Int)), Some(3))], any), Some(p(PrimType::Int)));
        assert_eq!(sig("len").result(&[Ty::Array(Box::new(p(PrimType::Int)), None)], any), None);
        assert_eq!(sig("to_string").result(&[p(PrimType::Int)], any), Some(p(PrimType::String)));
        assert_eq!(sig("to_string").result(&[p(PrimType::Int)], |_| false), None);
        assert_eq!(sig("Int").result(&[p(PrimType::Float)], any), Some(p(PrimType::Int)));
        assert_eq!(sig("Float").result(&[p(PrimType::Bool)], any), None);
        assert_eq!(sig("abs").result(&[p(PrimType::Duration)], any), Some(p(PrimType::Duration)));
        assert_eq!(sig("abs").result(&[Ty::Unknown], any), None);
        assert_eq!(sig("min").result(&[p(PrimType::Float), p(PrimType::Float)], any), Some(p(PrimType::Float)));
        assert_eq!(sig("max").result(&[p(PrimType::Int), p(PrimType::Float)], any), None);
        assert_eq!(sig("max").result(&[p(PrimType::Int)], any), None);
        assert_eq!(
            sig("starts_with").result(&[p(PrimType::String), Ty::Unknown], any),
            Some(p(PrimType::Bool))
        );
        assert_eq!(sig("contains").result(&[p(PrimType::String), p(PrimType::Int)], any), None);
        assert!(bare_builtin_sig("println").is_none());
    }
}
