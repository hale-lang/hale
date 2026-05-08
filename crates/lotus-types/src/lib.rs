//! Lotus type checker. Phase 1 milestone 2.
//!
//! Public surface:
//! - [`check_program`] — check a single program in isolation.
//! - [`check_bundle`] — check a multi-file bundle (e.g., a
//!   project that imports across files).
//! - [`Bundle`] — the compilation-unit shape the bundle checker
//!   takes.
//! - [`ty::Ty`] — resolved-type representation.
//!
//! Milestone-2 cut: literal typing, binary/unary op type
//! compatibility, struct-literal field typing, bus-send
//! subject + payload type matching, closure-assertion type
//! compatibility, `self.field` resolution against enclosing
//! locus's params. Externally-imported names (stdlib paths
//! like `time::sleep`, `println` builtins) resolve to
//! `Ty::Unknown` and pass through.
//!
//! Deferred to milestone 3: contract compatibility (F.8),
//! generic instantiation, k_max compile-time computation,
//! closure cycle existence, full call-site signature checking
//! against built-ins.

pub mod check;
pub mod resolve;
pub mod symbol;
pub mod ty;

use std::collections::BTreeMap;

use lotus_syntax::ast::Program;
use lotus_syntax::Diag;

pub use crate::symbol::Bundle;
pub use crate::ty::Ty;

/// Check a single program. Returns all diagnostics from
/// resolution + type checking.
pub fn check_program(program: &Program) -> Vec<Diag> {
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    programs.insert(String::new(), program);
    check_bundle(&Bundle { programs })
}

/// Check a bundle of programs (one logical compilation unit
/// spread across multiple `.lt` files, linked by `import`).
pub fn check_bundle(bundle: &Bundle<'_>) -> Vec<Diag> {
    let (top, mut diags) = resolve::build_top_scope(bundle);
    diags.extend(check::check_bundle(bundle, &top));
    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use lotus_syntax::parse_source;

    fn check(src: &str) -> Vec<Diag> {
        let p = parse_source(src).expect("parses");
        check_program(&p)
    }

    #[test]
    fn ok_simple_locus() {
        let src = r#"
            locus L {
                params { x: Int = 5; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags, got: {:?}", diags);
    }

    #[test]
    fn err_struct_field_type_mismatch() {
        let src = r#"
            type Point { x: Int; y: Int; }
            fn main() {
                let p = Point { x: "hi", y: 2 };
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("field `x`")),
            "expected field-type error, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_send_subject_not_declared() {
        let src = r#"
            type Msg { text: String; }
            locus L {
                bus { publish "ok" of type Msg; }
                run() { "wrong" <- Msg { text: "x" }; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("not declared in locus")),
            "expected undeclared-subject error, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_send_payload_type_mismatch() {
        let src = r#"
            type Msg { text: String; }
            type Other { v: Int; }
            locus L {
                bus { publish "s" of type Msg; }
                run() { "s" <- Other { v: 1 }; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("not assignable")),
            "expected payload-type error, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_contract_consume_missing_on_child() {
        let src = r#"
            locus ChildL {
                params { v: Int = 0; }
                contract { expose v: Int; }
            }
            locus ParentL {
                contract { consume value: Int; }
                accept(c: ChildL) { }
            }
            fn main() { ParentL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("does not expose")),
            "expected contract-missing error; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_contract_type_mismatch() {
        let src = r#"
            locus ChildL {
                params { value: String = "hi"; }
                contract { expose value: String; }
            }
            locus ParentL {
                contract { consume value: Int; }
                accept(c: ChildL) { }
            }
            fn main() { ParentL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("exposes it as")),
            "expected type-mismatch error; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_consume_without_accept() {
        let src = r#"
            locus ParentL {
                contract { consume thing: Int; }
            }
            fn main() { ParentL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("declares no `accept")),
            "expected accept-missing error; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_contract_compatible() {
        let src = r#"
            locus ChildL {
                params { value: Int = 0; }
                contract { expose value: Int; }
            }
            locus ParentL {
                contract { consume value: Int; }
                accept(c: ChildL) { }
            }
            fn main() { ParentL { }; }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected clean check; got: {:?}", diags);
    }

    #[test]
    fn err_let_type_mismatch() {
        let src = r#"
            fn main() {
                let x: Int = "hello";
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("let `x`")),
            "expected let-type error, got: {:?}",
            diags
        );
    }
}
