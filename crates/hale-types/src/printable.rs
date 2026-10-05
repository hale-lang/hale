//! What can be printed (F.40 phase 4, S7): one predicate, two readers.
//!
//! `println`, `to_string`, f-string interpolation and the `String + x`
//! coercion render a value through one renderer, and which types it
//! renders is this module's answer. The checker refuses a value of a type
//! it says does not print; lowering reads it before rendering. Each side
//! maps its own type into a [`PrintShape`] (the checker its `Ty`, lowering
//! its `CodegenTy`) and asks [`prints`], so there is no second copy of
//! the rule to keep in step.
//!
//! The shape is one level deep and its children are the reader's own
//! types, expanded only when the rule looks at them: a record's fields
//! are mapped when the record is asked about, so a mutually recursive
//! declaration (which lowering refuses at layout, but the checker sees
//! first) costs [`MAX_DEPTH`] levels, not a tree.

use hale_syntax::ast::PrimType;

/// Nesting deeper than this does not print: the guard for mutually
/// recursive declarations.
pub const MAX_DEPTH: u32 = 16;

/// The primitives that render: text and the scalars. `Bytes` and its
/// views are binary, and the useful rendering (hex, a length, a text
/// decode) is the author's choice; `Uint` has no representation.
pub const PRINTABLE_PRIMS: &[PrimType] = &[
    PrimType::String,
    PrimType::Int,
    PrimType::Bool,
    PrimType::Float,
    PrimType::Decimal,
    PrimType::Duration,
    PrimType::Time,
    PrimType::StringView,
];

/// The element types a fixed array or a `bounded` renders with: the
/// scalars, whose storage is inline where the renderer walks it.
pub const SEQUENCE_ELEMENT_PRIMS: &[PrimType] =
    &[PrimType::Int, PrimType::Float, PrimType::Bool, PrimType::Decimal, PrimType::Duration];

/// A type as the printable rule sees it, one level deep; `T` is the
/// reader's own type, for the children the rule may look at.
#[derive(Clone, Debug, PartialEq)]
pub enum PrintShape<T> {
    /// A primitive: prints when it is in [`PRINTABLE_PRIMS`].
    Prim(PrimType),
    /// An enum: prints as its variant (and its payload).
    Enum,
    /// A record (`type T { .. }`): prints as `T { field: v, .. }` when
    /// every field does.
    Record(Vec<T>),
    /// Another name for a type (an alias): prints when that type does.
    Alias(T),
    /// A tuple: prints when every component does.
    Tuple(Vec<T>),
    /// A fixed array or a `bounded`: prints when its element is in
    /// [`SEQUENCE_ELEMENT_PRIMS`].
    Sequence(T),
    /// A type the checker cannot see: `Unknown`, or a name it does not
    /// resolve. It prints: the checker is permissive about what it
    /// cannot see.
    Unseen,
    /// No text form: a locus (flow, not shape: rendering one would hand
    /// out the `params` a `@sealed` locus confines), a perspective, an
    /// interface, a function, an unsized array (no length to walk), a
    /// fallible, a cell, a batch.
    NoTextForm,
}

/// Whether a value of type `t` prints; `shape_of` maps the reader's type
/// into its shape.
pub fn prints<T>(t: &T, shape_of: &impl Fn(&T) -> PrintShape<T>) -> bool {
    prints_at(t, 0, shape_of)
}

/// [`prints`] for a type found `depth` levels inside the one asked
/// about.
pub fn prints_at<T>(t: &T, depth: u32, shape_of: &impl Fn(&T) -> PrintShape<T>) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    match shape_of(t) {
        PrintShape::Prim(p) => PRINTABLE_PRIMS.contains(&p),
        PrintShape::Enum | PrintShape::Unseen => true,
        PrintShape::Record(fields) => fields.iter().all(|f| prints_at(f, depth + 1, shape_of)),
        PrintShape::Alias(inner) => prints_at(&inner, depth + 1, shape_of),
        PrintShape::Tuple(parts) => parts.iter().all(|p| prints_at(p, depth + 1, shape_of)),
        PrintShape::Sequence(elem) => {
            matches!(shape_of(&elem), PrintShape::Prim(p) if SEQUENCE_ELEMENT_PRIMS.contains(&p))
        }
        PrintShape::NoTextForm => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A toy reader: a type is a shape whose children index a table.
    fn shape(table: &[PrintShape<usize>]) -> impl Fn(&usize) -> PrintShape<usize> + '_ {
        move |i| table[*i].clone()
    }

    #[test]
    fn the_rule_over_each_shape() {
        use PrintShape::*;
        let t = [
            Prim(PrimType::Int),        // 0
            Prim(PrimType::Bytes),      // 1
            Prim(PrimType::StringView), // 2
            Record(vec![0, 2]),         // 3
            Record(vec![0, 1]),         // 4
            Sequence(0),                // 5
            Sequence(2),                // 6
            Tuple(vec![0, 3]),          // 7
            Tuple(vec![4]),             // 8
            Record(vec![9]),            // 9: refers to itself
            Alias(1),                   // 10
            Enum,                       // 11
            Unseen,                     // 12
            NoTextForm,                 // 13
            Sequence(12),               // 14
        ];
        let f = shape(&t);
        let answers: Vec<bool> = (0..t.len()).map(|i| prints(&i, &f)).collect();
        assert_eq!(
            answers,
            [true, false, true, true, false, true, false, true, false, false, false, true, true, false, false]
        );
    }
}
