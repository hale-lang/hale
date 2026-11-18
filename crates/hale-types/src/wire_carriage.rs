//! What the bus wire carries (GH #1951 item 4), and what a cross-pool
//! spawn marshals (GH #1942).
//!
//! [`WIRE_TABLE`] is the bus wire format's carriage table (m70; internal,
//! spec/decisions.md F.36): every shape a payload's value may take, and
//! the bytes it travels as. The checker's refusal of a payload names what
//! the wire carries from it ([`carried_list`]), and the serializer
//! (`hale-codegen`'s `bus/wire.rs`) dispatches a value on the shape
//! [`WireShape`] it classifies it as, so the list a refusal prints and
//! the encodings the serializer emits are the one table. [`ty_uncarried`]
//! is the checker's walk of a payload type against it.
//!
//! A cross-pool spawn marshals the child's params through the bus wire
//! serializer and births the child on the owner's thread from the
//! payload. The spawn carries a scalar of its set (`Int`, `Float`,
//! `Bool`, `Decimal`, `Duration`, `Time`), a `String`, a `Bytes`, a
//! struct, and a fixed array of those; inside a struct, the spawn
//! carries the wire's fixed-walk shapes (a scalar, `U256` and `I256`
//! included, a payload-free enum's tag, a nested struct, a `bounded`
//! buffer of inline elements, a fixed array of scalars or structs). The
//! counted shapes (a `bounded` of strings or records, an enum's cell, a
//! tuple) are walked by a loop or a switch, and a create cell's dispatch
//! is one straight-line block, so the spawn does not marshal them. Any
//! other param checked clean and the build refused the spawn. This
//! module is the rule the check refuses one by
//! (`lowering_laws::cross_pool_child_shape`), read off the declarations
//! as lowering reads them.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{PrimType, TopDecl, TypeDecl, TypeDeclBody, TypeExpr};

use crate::Bundle;
use crate::resolve::TopScope;
use crate::symbol::{TopSymbol, TypeInfo, TypeKind};
use crate::ty::Ty;

/// A shape a bus payload's value takes on the wire (m70). The checker
/// classifies a `Ty`, the serializer a lowered type, into one of these;
/// [`WIRE_TABLE`] says what each carries and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireShape {
    /// A fixed-width scalar: its bytes.
    Scalar,
    /// A payload-free enum: its tag.
    PayloadFreeEnum,
    String,
    Bytes,
    /// A record: its fields in declaration order.
    Struct,
    /// A `bounded[T; N]` of scalars or payload-free enums: its inline
    /// storage.
    BoundedInline,
    /// A `bounded[T; N]` of strings, bytes, records or enums with
    /// payloads: a count, then the live elements.
    BoundedCounted,
    /// A fixed array `[T; N]`: its `N` elements.
    Array,
    /// An enum with payloads: its tag, then the variant's values.
    PayloadEnum,
    /// A tuple: as a record of its elements.
    Tuple,
    /// `std::json::Value`: its text.
    JsonValue,
}

/// One row of the carriage table.
#[derive(Debug, Clone, Copy)]
pub struct WireRow {
    pub shape: WireShape,
    /// What the row carries, as a refusal lists it.
    pub carried: &'static str,
    /// The bytes a value of the shape travels as.
    pub encoding: &'static str,
}

/// The bus wire format's carriage table (m70, internal). A payload is a
/// record or an enum with payloads, and every value inside it is one of
/// these shapes, at any depth. The encodings are little-endian; a
/// payload's frame is its value's encoding with nothing between fields.
pub const WIRE_TABLE: &[WireRow] = &[
    WireRow {
        shape: WireShape::Scalar,
        carried: "scalars (`Int`, `Float`, `Bool`, `Decimal`, `U256`, `I256`, `Duration`, `Time`, a unit type)",
        encoding: "its bytes: 8 (`Int`, `Float`, `Bool`, `Duration`, `Time`, a unit type), 16 (`Decimal`) or 32 (`U256`, `I256`)",
    },
    WireRow { shape: WireShape::PayloadFreeEnum, carried: "payload-free enums", encoding: "its tag, a `u32` (the variant's declaration index)" },
    WireRow { shape: WireShape::String, carried: "`String`", encoding: "an `i64` length, then its bytes" },
    WireRow { shape: WireShape::Bytes, carried: "`Bytes`", encoding: "an `i64` length, then its bytes" },
    WireRow {
        shape: WireShape::BoundedInline,
        carried: "`bounded` lists",
        encoding: "of scalars or payload-free enums: its inline storage, the `i64` count and all `N` slots",
    },
    WireRow {
        shape: WireShape::BoundedCounted,
        carried: "`bounded` lists",
        encoding: "of strings, bytes, records or enums with payloads: an `i64` count, then that many elements",
    },
    WireRow { shape: WireShape::Array, carried: "fixed arrays", encoding: "its `N` elements" },
    WireRow {
        shape: WireShape::PayloadEnum,
        carried: "enums with payloads",
        encoding: "its tag, a `u32` (the variant's declaration index), then the variant's values in order",
    },
    WireRow { shape: WireShape::Tuple, carried: "tuples", encoding: "as a record of its elements" },
    WireRow { shape: WireShape::JsonValue, carried: "`std::json::Value`", encoding: "its text, as a `String`" },
    WireRow { shape: WireShape::Struct, carried: "records of these", encoding: "its fields, in declaration order" },
];

/// The row of `shape`.
pub fn wire_row(shape: WireShape) -> &'static WireRow {
    WIRE_TABLE.iter().find(|r| r.shape == shape).expect("every wire shape has a row")
}

/// What the wire carries, as a refusal lists it: each row's `carried`
/// once, in the table's order (`scalars (…), payload-free enums, …, and
/// records of these`).
pub fn carried_list() -> String {
    let mut seen: Vec<&str> = Vec::new();
    for r in WIRE_TABLE {
        if !seen.contains(&r.carried) {
            seen.push(r.carried);
        }
    }
    match seen.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{}, and {}", rest.join(", "), last),
        Some((last, _)) => last.to_string(),
        None => String::new(),
    }
}

/// What a value is that the wire has no form for: a refusal's words.
pub mod refusal {
    pub const FN_VALUE: &str = "a fn value";
    pub const HANDLE: &str = "a locus handle";
    pub const PROJECTION: &str = "a projection";
    pub const VIEW: &str = "a view (`BytesView`, `StringView`, `BytesMut`)";
    pub const NO_FORM: &str = "a primitive the wire has no form for";
    pub const UNSIZED_ARRAY: &str = "an array of no fixed length";
    /// a type that reaches itself: its frame has no bound
    pub const RECURSIVE: &str = "a recursive type";
}

/// The `Ty` of a struct field (or element, value) as the checker sees it,
/// classified: `Ok(Some(shape))`, `Ok(None)` when this bundle cannot say
/// (an unresolved or generic name, which the rules for those name), or
/// `Err(what)`.
fn ty_shape(ty: &Ty, top: &TopScope) -> Result<Option<WireShape>, &'static str> {
    use hale_syntax::ast::PrimType as P;
    Ok(Some(match ty {
        Ty::Prim(p) => match p {
            P::Int | P::Float | P::Bool | P::Decimal(_) | P::U256 | P::I256 | P::Duration | P::Time => WireShape::Scalar,
            P::String => WireShape::String,
            P::Bytes => WireShape::Bytes,
            P::BytesView | P::StringView | P::BytesMut => return Err(refusal::VIEW),
            P::Uint => return Err(refusal::NO_FORM),
        },
        Ty::Named(n) if n == hale_syntax::json_gen::JSON_VALUE => WireShape::JsonValue,
        Ty::Named(n) => match top.lookup(n) {
            Some(TopSymbol::Type(TypeInfo { kind, .. })) => match kind {
                TypeKind::Struct(_) => WireShape::Struct,
                TypeKind::Enum(vs) if vs.iter().all(|v| v.fields.is_empty()) => WireShape::PayloadFreeEnum,
                TypeKind::Enum(_) => WireShape::PayloadEnum,
                TypeKind::Scalar(_) => WireShape::Scalar,
                TypeKind::Alias(inner) => return ty_shape(inner, top),
            },
            Some(TopSymbol::Locus(_) | TopSymbol::Interface(_)) => return Err(refusal::HANDLE),
            _ => return Ok(None),
        },
        Ty::Bounded(e, _) => match ty_shape(e, top)? {
            Some(WireShape::Scalar | WireShape::PayloadFreeEnum) => WireShape::BoundedInline,
            Some(_) => WireShape::BoundedCounted,
            None => return Ok(None),
        },
        Ty::Array(_, Some(_)) => WireShape::Array,
        Ty::Array(_, None) => return Err(refusal::UNSIZED_ARRAY),
        Ty::Tuple(_) => WireShape::Tuple,
        Ty::Function { .. } => return Err(refusal::FN_VALUE),
        Ty::Projection(..) => return Err(refusal::PROJECTION),
        Ty::Unit | Ty::Fallible { .. } | Ty::Unknown => return Ok(None),
    }))
}

/// The first part of a payload of type `ty` the wire does not carry,
/// through records, list and array elements, tuple elements and enum
/// variants: where it is (`xs`, `inner.xs`, `xs[]`, `pair.1`,
/// `Variant.0`; empty for `ty` itself) and what it is. `None` when
/// every part is carried or not judged here.
pub fn ty_uncarried(ty: &Ty, top: &TopScope) -> Option<Uncarried> {
    let mut path = Vec::new();
    walk_ty(ty, top, &mut path, &mut BTreeSet::new())
}

fn walk_ty(ty: &Ty, top: &TopScope, path: &mut Vec<String>, open: &mut BTreeSet<String>) -> Option<Uncarried> {
    if let Ty::Named(n) = ty {
        if let Some(TopSymbol::Type(TypeInfo { kind: TypeKind::Alias(inner), .. })) = top.lookup(n) {
            return walk_ty(inner, top, path, open);
        }
    }
    let here = |path: &Vec<String>, what: &str| {
        let mut p = String::new();
        for seg in path {
            if !p.is_empty() && !seg.starts_with('[') {
                p.push('.');
            }
            p.push_str(seg);
        }
        Some(Uncarried { path: p, ty: ty.display(), what: what.to_string() })
    };
    let shape = match ty_shape(ty, top) {
        Ok(Some(s)) => s,
        Ok(None) => return None,
        Err(what) => return here(path, what),
    };
    let within = |t: &Ty, seg: String, path: &mut Vec<String>, open: &mut BTreeSet<String>| {
        path.push(seg);
        let found = walk_ty(t, top, path, open);
        path.pop();
        found
    };
    match (shape, ty) {
        (WireShape::Struct | WireShape::PayloadEnum, Ty::Named(n)) => {
            // A type reached from inside itself: the frame of one has no
            // bound (and a serializer that walks it inline, no end).
            if !open.insert(n.clone()) {
                return here(path, refusal::RECURSIVE);
            }
            let found = match top.lookup(n) {
                Some(TopSymbol::Type(TypeInfo { kind: TypeKind::Struct(fields), .. })) => {
                    fields.iter().find_map(|f| within(&f.ty, f.name.clone(), path, open))
                }
                Some(TopSymbol::Type(TypeInfo { kind: TypeKind::Enum(vs), .. })) => vs.iter().find_map(|v| {
                    v.fields.iter().enumerate().find_map(|(i, t)| within(t, format!("{}.{i}", v.name), path, open))
                }),
                _ => None,
            };
            open.remove(n);
            found
        }
        (WireShape::BoundedCounted | WireShape::Array, Ty::Bounded(e, _) | Ty::Array(e, _)) => {
            within(e, "[]".to_string(), path, open)
        }
        (WireShape::Tuple, Ty::Tuple(parts)) => parts.iter().enumerate().find_map(|(i, t)| within(t, i.to_string(), path, open)),
        _ => None,
    }
}


/// The declarations the rules read: the bundle's types and the names of
/// its loci and interfaces (a handle, which no wire
/// carries).
pub struct WireDecls<'a> {
    types: BTreeMap<&'a str, &'a TypeDecl>,
    handles: BTreeSet<&'a str>,
}

/// A field the wire format cannot carry: where it is (`xs`, or
/// `inner.xs` through a nested struct) and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uncarried {
    pub path: String,
    pub ty: String,
    pub what: String,
}

impl<'a> WireDecls<'a> {
    pub fn of(bundle: &'a Bundle<'a>) -> Self {
        fn collect<'a>(items: &'a [TopDecl], types: &mut BTreeMap<&'a str, &'a TypeDecl>, handles: &mut BTreeSet<&'a str>) {
            for item in items {
                match item {
                    TopDecl::Type(t) => {
                        types.entry(t.name.name.as_str()).or_insert(t);
                    }
                    TopDecl::Locus(l) => {
                        handles.insert(l.name.name.as_str());
                    }
                    TopDecl::Interface(i) => {
                        handles.insert(i.name.name.as_str());
                    }
                    TopDecl::Module(m) => collect(&m.items, types, handles),
                    _ => {}
                }
            }
        }
        let mut types = BTreeMap::new();
        let mut handles = BTreeSet::new();
        for program in bundle.programs.values() {
            collect(&program.items, &mut types, &mut handles);
        }
        WireDecls { types, handles }
    }

    /// The declaration a single-segment name resolves to, through aliases.
    fn named(&self, te: &'a TypeExpr) -> Option<&'a TypeDecl> {
        let mut te = te;
        for _ in 0..16 {
            let TypeExpr::Named { path, generic_args, .. } = te else { return None };
            if path.segments.len() != 1 || !generic_args.is_empty() {
                return None;
            }
            let decl = self.types.get(path.segments[0].name.as_str()).copied()?;
            match &decl.body {
                TypeDeclBody::Alias(inner) => te = inner,
                _ => return Some(decl),
            }
        }
        None
    }

    fn is_handle(&self, te: &TypeExpr) -> bool {
        matches!(te, TypeExpr::Named { path, .. } if path.segments.len() == 1 && self.handles.contains(path.segments[0].name.as_str()))
    }

    fn walk_struct(&self, decl: &'a TypeDecl, prefix: &str, seen: &mut BTreeSet<&'a str>) -> Option<Uncarried> {
        let TypeDeclBody::Struct(fields) = &decl.body else { return None };
        if !decl.generics.is_empty() || !seen.insert(decl.name.name.as_str()) {
            return None;
        }
        for f in fields {
            let path = format!("{prefix}{}", f.name.name);
            if let Some(found) = self.field(&f.ty, &path, seen) {
                return Some(found);
            }
        }
        None
    }

    fn field(&self, te: &'a TypeExpr, path: &str, seen: &mut BTreeSet<&'a str>) -> Option<Uncarried> {
        let refuse = |what: &str| Some(Uncarried { path: path.to_string(), ty: crate::surfaces::type_text(te), what: what.to_string() });
        match te {
            TypeExpr::Primitive(p, _) => match p {
                PrimType::BytesMut | PrimType::Uint => refuse("a primitive the wire has no form for"),
                _ => None,
            },
            TypeExpr::Named { .. } => {
                if self.is_handle(te) {
                    return refuse("a locus handle");
                }
                let decl = self.named(te)?;
                match &decl.body {
                    TypeDeclBody::Struct(_) => self.walk_struct(decl, &format!("{path}."), seen),
                    TypeDeclBody::Enum(vs) if vs.iter().any(|v| !v.fields.is_empty()) => refuse("an enum with payloads"),
                    _ => None,
                }
            }
            TypeExpr::Tuple(..) => refuse("a tuple"),
            TypeExpr::Function { .. } => refuse("a fn value"),
            TypeExpr::Projection { .. } => refuse("a projection"),
            TypeExpr::Bounded { elem, .. } => match elem.as_ref() {
                TypeExpr::Primitive(PrimType::String | PrimType::Bytes, _) => refuse("a `bounded` buffer of pointers"),
                // a record, or an enum with payloads: a pointer each,
                // which the wire carries counted, in a loop
                e if self.named(e).is_some_and(|d| match &d.body {
                    TypeDeclBody::Struct(_) => true,
                    TypeDeclBody::Enum(vs) => vs.iter().any(|v| !v.fields.is_empty()),
                    _ => false,
                }) =>
                {
                    refuse("a `bounded` buffer of pointers")
                }
                _ => None,
            },
            TypeExpr::Array { elem, .. } => match elem.as_ref() {
                TypeExpr::Primitive(
                    PrimType::Int
                    | PrimType::Float
                    | PrimType::Bool
                    | PrimType::Time
                    | PrimType::Duration
                    | PrimType::Decimal(_)
                    | PrimType::U256
                    | PrimType::I256,
                    _,
                ) => None,
                e => match self.named(e).map(|d| (d, &d.body)) {
                    Some((d, TypeDeclBody::Struct(_))) => self.walk_struct(d, &format!("{path}[]."), seen),
                    Some((_, TypeDeclBody::Scalar(_))) => None,
                    _ => refuse("an array of an element the wire has no fixed form for"),
                },
            },
        }
    }

    /// The first param of a cross-pool child, its type written `te`, that
    /// the spawn cannot marshal: a scalar of the cross-pool set (`Int`,
    /// `Float`, `Bool`, `Decimal`, `Duration`, `Time`), a `String`, a
    /// `Bytes`, an identity, a range or a quantity (its `Int`), a struct
    /// the wire carries, or a fixed array of those.
    pub fn crosspool_uncarried(&self, te: &'a TypeExpr, path: &str) -> Option<Uncarried> {
        let refuse = |what: &str| Some(Uncarried { path: path.to_string(), ty: crate::surfaces::type_text(te), what: what.to_string() });
        match te {
            TypeExpr::Primitive(p, _) => match p {
                PrimType::Int
                | PrimType::Float
                | PrimType::Bool
                | PrimType::String
                | PrimType::Duration
                | PrimType::Decimal(_)
                | PrimType::Time
                | PrimType::Bytes => None,
                _ => refuse("a primitive the spawn does not marshal"),
            },
            TypeExpr::Named { .. } => {
                if self.is_handle(te) {
                    return refuse("a locus handle");
                }
                let Some(decl) = self.named(te) else { return None };
                match &decl.body {
                    TypeDeclBody::Struct(_) => {
                        let mut seen = BTreeSet::new();
                        self.walk_struct(decl, &format!("{path}."), &mut seen)
                    }
                    TypeDeclBody::Enum(_) => refuse("an enum"),
                    _ => None,
                }
            }
            TypeExpr::Array { elem, .. } => match self.crosspool_uncarried(elem, path) {
                Some(_) => refuse("an array of an element the spawn does not marshal"),
                None => {
                    let mut seen = BTreeSet::new();
                    self.field(te, path, &mut seen)
                }
            },
            TypeExpr::Tuple(..) => refuse("a tuple"),
            TypeExpr::Function { .. } => refuse("a fn value"),
            TypeExpr::Projection { .. } | TypeExpr::Bounded { .. } => refuse("a type the spawn does not marshal"),
        }
    }
}
