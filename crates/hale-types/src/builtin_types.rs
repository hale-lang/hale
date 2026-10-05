//! The builtin error types (F.40 phase 4, S8): one declaration, three
//! readers.
//!
//! The compiler declares a handful of record types no program spells:
//! the error payloads of the fallible stdlib calls, of the synthesized
//! `@form` methods and of a `bounded` push, the `on_unmatched: fail`
//! publish's, and an `on_failure` handler's `ClosureViolation`. The
//! checker injects each into the top scope (`resolve::inject_builtin_types`),
//! the unknown-type-name rule accepts each name, and lowering builds each
//! as a struct (`declare_builtin_types`), all from [`BUILTIN_TYPES`], so
//! there is no second copy of a shape to keep in step.
//!
//! A declaration of the same name in the program wins on both sides: the
//! checker injects only a name its scope does not hold, and lowering
//! declares the builtins before the program's types, which replace them.

use hale_syntax::ast::PrimType;

/// When the checker puts a builtin type into the top scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Injected {
    /// In every program.
    Always,
    /// Only in a bundle with a topic that declares `on_unmatched: fail`,
    /// whose publishes carry the type through an `or`.
    WhenAFailTopic,
}

/// One builtin type: its name, its fields in declaration order (each a
/// name and a primitive), and when the checker injects it.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinType {
    pub name: &'static str,
    pub fields: &'static [(&'static str, PrimType)],
    pub injected: Injected,
}

use PrimType::{Int, String as Str};

/// Every builtin type, by name.
pub const BUILTIN_TYPES: &[BuiltinType] = &[
    // Routing keys v0.2: the payload of an `on_unmatched: fail` publish
    // no subscriber's key matched. `key_lo` / `key_hi` are the low and
    // high 64 bits of the key (`key_hi` is 0 for an Int key).
    BuiltinType {
        name: "BusUnmatchedKey",
        fields: &[("subject", Str), ("key_lo", Int), ("key_hi", Int)],
        injected: Injected::WhenAFailTopic,
    },
    // `bounded[T; N]`: a push at capacity.
    BuiltinType { name: "CapacityError", fields: &[("cap", Int), ("count", Int)], injected: Injected::Always },
    // An `on_failure` handler's error. `diff` is `left - right` for an
    // Int or Duration closure and 0 for a Float or Decimal one; the
    // closure's `left` / `right` / `tolerance` are not carried, since
    // their type is the closure's.
    BuiltinType {
        name: "ClosureViolation",
        fields: &[("locus", Str), ("closure", Str), ("diff", Int)],
        injected: Injected::Always,
    },
    // The fallible `std::crypto::*` calls: `kind` names the operation,
    // `detail` the failure.
    BuiltinType { name: "CryptoError", fields: &[("kind", Str), ("detail", Str)], injected: Injected::Always },
    // `@form(ring_buffer)`'s `pop`.
    BuiltinType { name: "EmptyError", fields: &[("kind", Str)], injected: Injected::Always },
    // `@form(vec)`'s `get` / `pop`, `std::bytes::at` and the binary
    // readers and writers.
    BuiltinType {
        name: "IndexError",
        fields: &[("kind", Str), ("index", Int), ("len", Int)],
        injected: Injected::Always,
    },
    // The fallible `std::io::*` calls: `kind` is the errno-derived tag
    // (`not_found`, `permission_denied`, `timeout`, ...), `errno` the raw
    // platform errno, `path` the file or connection target.
    BuiltinType {
        name: "IoError",
        fields: &[("kind", Str), ("errno", Int), ("path", Str)],
        injected: Injected::Always,
    },
    // `@form(hashmap)`'s `get` / `remove`. The key is not carried: its
    // type varies per map.
    BuiltinType { name: "KeyError", fields: &[("kind", Str)], injected: Injected::Always },
    // `std::str::parse_int` / `parse_float` / `parse_decimal`: `kind`
    // names the parser, `input` the text it refused.
    BuiltinType { name: "ParseError", fields: &[("kind", Str), ("input", Str)], injected: Injected::Always },
];

/// The builtin type named `name`.
pub fn builtin_type(name: &str) -> Option<&'static BuiltinType> {
    BUILTIN_TYPES.iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_sorted() {
        let names: Vec<&str> = BUILTIN_TYPES.iter().map(|t| t.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names, sorted);
    }

    #[test]
    fn every_field_is_a_string_or_an_int() {
        // Lowering lays a field out as a pointer or an i64.
        for t in BUILTIN_TYPES {
            for (f, p) in t.fields {
                assert!(matches!(p, Int | Str), "{}.{f}", t.name);
            }
        }
    }
}
