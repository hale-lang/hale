//! What a cross-pool spawn marshals (GH #1942).
//!
//! A cross-pool spawn marshals the child's params through the bus wire
//! serializer and births the child on the owner's thread from the
//! payload. The spawn carries a scalar of its set (`Int`, `Float`,
//! `Bool`, `Decimal`, `Duration`, `Time`), a `String`, a `Bytes`, a
//! struct, and a fixed array of those; inside a struct, the serializer
//! carries what a bus payload's field may be (a scalar, `U256` and
//! `I256` included, a payload-free enum's tag, a nested struct, a
//! `bounded` buffer of inline elements, a fixed array of scalars or
//! structs). Any other param checked clean and the build refused the
//! spawn. This module is the rule the check refuses one by
//! (`lowering_laws::cross_pool_child_shape`), read off the declarations
//! as lowering reads them; a bus payload's own fields are judged by the
//! checker's `struct_field_not_carried`.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{PrimType, TopDecl, TypeDecl, TypeDeclBody, TypeExpr};

use crate::Bundle;

/// The declarations the rules read: the bundle's types and the names of
/// its loci, interfaces and perspectives (a handle, which no wire
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
                    TopDecl::Perspective(p) => {
                        handles.insert(p.name.name.as_str());
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
            TypeExpr::Perspective { .. } => refuse("a locus handle"),
            TypeExpr::Projection { .. } => refuse("a projection"),
            TypeExpr::Bounded { elem, .. } => match elem.as_ref() {
                TypeExpr::Primitive(PrimType::String | PrimType::Bytes, _) => refuse("a `bounded` buffer of pointers"),
                e if self.named(e).is_some_and(|d| matches!(d.body, TypeDeclBody::Struct(_))) => {
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
            TypeExpr::Perspective { .. } => refuse("a locus handle"),
            TypeExpr::Projection { .. } | TypeExpr::Bounded { .. } => refuse("a type the spawn does not marshal"),
        }
    }
}
