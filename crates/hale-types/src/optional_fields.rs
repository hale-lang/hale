//! The value an absent optional field holds (GH #1951 item 3, F.52).
//!
//! `x?: T` declares `x` beside its presence flag `has_x`
//! (`hale_syntax::ast::FieldOptional`, synthesized by the parser). A record
//! literal leaves `x` out to build the field absent, so `x` needs a value
//! for that literal to fill in: its declared default (`x?: T = d`), and,
//! where it declares none, its type's zero, the value the JSON codecs give
//! an absent optional field (spec/api.md § Codecs): `0`, `0.0`, `false`,
//! `""`, a `Decimal`'s `0`, a 256-bit integer's `0`, the epoch for a
//! `Time`, `0ns` for a `Duration`, `null` for a `std::json::Value`, an
//! identity's or a quantity's `0`, a range's `0` (its least value where it
//! leaves `0` out), a fixed array of its element's zero, an enum's first
//! variant with its values' zeros, and a record built of its fields'
//! defaults and zeros. [`fill_optional_zeros`] writes that zero
//! as the field's default, before the JSON parsers are generated, so every
//! later reader (the checker's literal rule, the codecs, lowering) sees an
//! ordinary defaulted field. A `bounded` field needs none: it starts empty.
//!
//! A type with no zero here (a point, whose origin is no count; a
//! `Bytes`; a tuple; a generic parameter; a record that reaches itself)
//! keeps no default, and the
//! checker refuses the field at its name, asking for one
//! ([`missing_zero_diag`]).

use std::collections::BTreeMap;

use hale_syntax::ast::{
    Expr, FieldOptional, PrimType, Program, Stmt, StructField, TopDecl, TypeDeclBody, TypeExpr,
};
use hale_syntax::json_gen::Conv;
use hale_syntax::Diag;

use crate::topic_identity::{Shapes, TypeClass};

/// Give each declared optional field of `programs` that has no default
/// its type's zero as one. `context` (the bundled stdlib) is read for the
/// declarations a field's type may end at and never rewritten.
/// Idempotent: a field with a default is left as it is.
pub fn fill_optional_zeros(programs: &mut [&mut Program], context: &[&Program]) {
    let any = programs.iter().any(|p| has_declared_optional(&p.items));
    if !any {
        return;
    }
    // (program, type name, field name) -> the zero's source text
    let mut zeros: BTreeMap<(usize, String, String), String> = BTreeMap::new();
    {
        let mut slices: Vec<&[TopDecl]> = context.iter().map(|p| p.items.as_slice()).collect();
        slices.extend(programs.iter().map(|p| p.items.as_slice()));
        let shapes = Shapes::of_all(&slices);
        let all: Vec<&Program> = context.iter().copied().chain(programs.iter().map(|p| &**p)).collect();
        let convs = hale_syntax::json_gen::scalar_convs(&all);
        let z = Zeros { shapes: &shapes, convs: &convs };
        for (i, p) in programs.iter().enumerate() {
            each_struct(&p.items, &mut |tname, fields| {
                for f in fields {
                    if !matches!(f.optional, FieldOptional::Declared { .. }) || f.default.is_some() {
                        continue;
                    }
                    if let Some(src) = z.zero(&f.ty, 0) {
                        zeros.insert((i, tname.to_string(), f.name.name.clone()), src);
                    }
                }
            });
        }
    }
    for ((i, tname, fname), src) in zeros {
        let p = &mut *programs[i];
        each_struct_mut(&mut p.items, &mut |name, fields| {
            if name != tname {
                return;
            }
            if let Some(f) = fields.iter_mut().find(|f| f.name.name == fname) {
                if f.default.is_none() {
                    f.default = parse_zero(&src, f.name.span.start.0);
                }
            }
        });
    }
}

/// The checker's refusal of a declared optional field that still has no
/// value for an absent literal to hold: no default, and no zero
/// [`fill_optional_zeros`] could write.
pub fn missing_zero_diag(type_name: &str, f: &StructField, shown: &str) -> Option<Diag> {
    if !matches!(f.optional, FieldOptional::Declared { .. }) || f.default.is_some() {
        return None;
    }
    if matches!(f.ty, TypeExpr::Bounded { .. }) {
        return None;
    }
    Some(Diag::ty(
        f.name.span,
        format!(
            "optional field `{}` of type `{}`: `{}` has no zero for an absent `{}` to hold, so give \
             it a default: `{}?: {} = <value>;` (a record literal that leaves `{}` out builds it \
             absent)",
            f.name.name, type_name, shown, f.name.name, f.name.name, shown, f.name.name
        ),
    ))
}

fn has_declared_optional(items: &[TopDecl]) -> bool {
    let mut any = false;
    each_struct(items, &mut |_, fields| {
        any |= fields.iter().any(|f| matches!(f.optional, FieldOptional::Declared { .. }));
    });
    any
}

fn each_struct<'a>(items: &'a [TopDecl], f: &mut dyn FnMut(&'a str, &'a [StructField])) {
    for item in items {
        match item {
            TopDecl::Type(t) => {
                if let TypeDeclBody::Struct(fields) = &t.body {
                    f(&t.name.name, fields);
                }
            }
            _ => {}
        }
    }
}

fn each_struct_mut(items: &mut [TopDecl], f: &mut dyn FnMut(&str, &mut Vec<StructField>)) {
    for item in items {
        match item {
            TopDecl::Type(t) => {
                if let TypeDeclBody::Struct(fields) = &mut t.body {
                    f(&t.name.name, fields);
                }
            }
            _ => {}
        }
    }
}

/// Parse the zero's source as an expression whose spans start at `at`
/// (the field's name), so a diagnostic about it lands on the field.
fn parse_zero(src: &str, at: u32) -> Option<Expr> {
    const HEAD: &str = "fn __optional_zero() { let __z = ";
    let text = format!("{HEAD}{src}; }}");
    let base = at.saturating_sub(HEAD.len() as u32);
    let p = hale_syntax::parse_source_at(&text, base).ok()?;
    let TopDecl::Fn(f) = p.items.into_iter().next()? else { return None };
    match f.body.stmts.into_iter().next()? {
        Stmt::Let { value, .. } => Some(value),
        _ => None,
    }
}

struct Zeros<'s, 'a> {
    shapes: &'s Shapes<'a>,
    convs: &'s BTreeMap<String, Conv>,
}

impl Zeros<'_, '_> {
    /// The zero of `te` as source text, if it has one.
    fn zero(&self, te: &TypeExpr, depth: usize) -> Option<String> {
        if depth > 16 {
            return None;
        }
        let d = self.shapes.dealias(te);
        if hale_syntax::json_gen::is_json_value(&d) {
            return Some("std::json::Value::null()".to_string());
        }
        if let TypeExpr::Array { elem, size: Some(size), .. } = &d {
            let hale_syntax::ast::Expr::Literal(hale_syntax::ast::Literal::Int(n), _) = size else { return None };
            return Some(format!("[{}; {n}]", self.zero(elem, depth + 1)?));
        }
        match self.shapes.classify(&d) {
            TypeClass::Prim(p) => prim_zero(p),
            TypeClass::Named { name, .. } => match self.convs.get(name)? {
                Conv::Identity(n, _) => Some(format!("{n}(0)")),
                Conv::Quantity(m, u, _) => Some(format!("0 * {m}{u}")),
                // a range's zero is `0` where it holds it, its least value
                // where it does not; a literal is a range's value in a default
                Conv::Range(_, z, _) => Some(z.to_string()),
            },
            // a record's zero is built of its fields' defaults and zeros
            TypeClass::Struct { name, fields } => {
                let mut inits = Vec::new();
                for f in fields {
                    // a nested optional field is left out, so it is absent:
                    // this pass gives it its own default (or the checker
                    // refuses it at its declaration)
                    if f.default.is_some()
                        || matches!(f.ty, TypeExpr::Bounded { .. })
                        || matches!(f.optional, FieldOptional::Declared { .. })
                    {
                        continue;
                    }
                    inits.push(format!("{}: {}", f.name.name, self.zero(&f.ty, depth + 1)?));
                }
                Some(if inits.is_empty() { format!("{name} {{ }}") } else { format!("{name} {{ {} }}", inits.join(", ")) })
            }
            // an enum's zero is its first variant, with its values' zeros
            TypeClass::Enum { name, variants } => {
                let v = variants.first()?;
                if v.fields.is_empty() {
                    return Some(format!("{name}::{}", v.name.name));
                }
                let zs: Option<Vec<String>> = v.fields.iter().map(|t| self.zero(t, depth + 1)).collect();
                Some(format!("{name}::{}({})", v.name.name, zs?.join(", ")))
            }
            TypeClass::Builtin(_) | TypeClass::Other => None,
        }
    }
}

fn prim_zero(p: PrimType) -> Option<String> {
    Some(
        match p {
            PrimType::Int => "0",
            PrimType::U64 => "std::u64::from_int(0)",
            PrimType::Float => "0.0",
            PrimType::Bool => "false",
            PrimType::String => "\"\"",
            PrimType::Decimal(_) => "0d",
            PrimType::U256 => "std::u256::from_int(0)",
            PrimType::I256 => "std::i256::from_int(0)",
            PrimType::Time => "`1970-01-01T00:00:00Z`",
            PrimType::Duration => "0ns",
            _ => return None,
        }
        .to_string(),
    )
}
