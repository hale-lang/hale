//! Aperio type checker. Phase 1 milestone 2.
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

/// m94: subject wildcard matching used by the type checker
/// (publish-side authorization for computed subjects) and
/// mirrored at runtime by `aperio-runtime::bus::subject_match`
/// and the C runtime's `lotus_subject_match`. v0 supports a
/// trailing `**` that matches *zero or more* remaining
/// dot-separated segments — `log.app.**` matches the root
/// `log.app` AND any descendant. All three implementations
/// must agree.
pub fn wildcard_match(pattern: &str, subject: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("**") {
        if prefix.is_empty() {
            return true;
        }
        if !prefix.ends_with('.') {
            return false;
        }
        let root = &prefix[..prefix.len() - 1];
        if subject == root {
            return true;
        }
        subject.starts_with(prefix) && subject.len() > prefix.len()
    } else if pattern.contains("**") {
        false
    } else {
        pattern == subject
    }
}

use std::collections::BTreeMap;

use aperio_syntax::ast::Program;
use aperio_syntax::Diag;

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
/// spread across multiple `.ap` files, linked by `import`).
pub fn check_bundle(bundle: &Bundle<'_>) -> Vec<Diag> {
    let (top, mut diags) = resolve::build_top_scope(bundle);
    diags.extend(check::check_bundle(bundle, &top));
    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use aperio_syntax::parse_source;

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
    fn err_typo_in_self_field() {
        let src = r#"
            locus L {
                params { x: Int = 5; }
                closure typo_check {
                    self.greting ~~ self.x within 0;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("no field `greting`")),
            "expected typo detection; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_closure_pure_literal_assertion() {
        let src = r#"
            locus L {
                params { x: Int = 5; }
                closure dud {
                    5 ~~ 5 within 0;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("pure literals")),
            "expected pure-literal closure error; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_closure_one_side_literal() {
        // One literal side is fine — `self.x ~~ 0 within 5`
        // is a meaningful "x stays near zero" invariant.
        let src = r#"
            locus L {
                params { count: Int = 0; }
                closure stays_low {
                    self.count ~~ 0 within 100;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags; got: {:?}", diags);
    }

    #[test]
    fn err_match_not_exhaustive() {
        let src = r#"
            fn main() {
                let x = 7;
                match x {
                    1 -> println("one"),
                    2 -> println("two"),
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("not exhaustive")),
            "expected exhaustiveness error; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_generic_enum_match_with_monomorph_arms_no_wildcard() {
        // m68: matching a generic-enum-typed scrutinee with
        // arms that use the synthesized monomorph name
        // (Result_Int_String::Ok / ::Err) should be exhaustive
        // without a wildcard. The typechecker only sees the
        // template `Result` (with variants Ok, Err); the user's
        // arms use the mangled names codegen recognizes. The
        // exhaustiveness check accepts the mangle prefix as
        // covering the template's variants.
        let src = r#"
            type Result<T, E> = enum {
                Ok(T),
                Err(E),
            };

            fn main() {
                let r: Result<Int, String> = Result_Int_String::Ok(7);
                match r {
                    Result_Int_String::Ok(n)  -> println("ok: ", n),
                    Result_Int_String::Err(s) -> println("err: ", s),
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected no diags; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_match_with_wildcard() {
        let src = r#"
            fn main() {
                let x = 7;
                match x {
                    1 -> println("one"),
                    _ -> println("other"),
                }
            }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags; got: {:?}", diags);
    }

    #[test]
    fn ok_bool_match_covers_both_cases() {
        let src = r#"
            fn main() {
                let x = true;
                match x {
                    true -> println("yes"),
                    false -> println("no"),
                }
            }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags; got: {:?}", diags);
    }

    #[test]
    fn err_bool_match_only_true() {
        let src = r#"
            fn main() {
                let x = true;
                match x {
                    true -> println("yes"),
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("not exhaustive")),
            "expected exhaustiveness error; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_typo_on_struct_value() {
        let src = r#"
            type Point { x: Int; y: Int; }
            fn main() {
                let p = Point { x: 1, y: 2 };
                let _q = p.zee;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("no field `zee`")),
            "expected typo detection; got: {:?}",
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

    // m50: immutable-binding enforcement.
    #[test]
    fn err_assign_to_immutable_let() {
        let src = r#"
            fn main() {
                let x: Int = 0;
                x = 1;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("cannot assign to `x`")
                    && d.message.contains("immutable")
            }),
            "expected immutable-binding error on `x = 1;`, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_assign_to_let_mut() {
        let src = r#"
            fn main() {
                let mut n: Int = 0;
                n = 1;
                n = n + 2;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on let mut + reassignment; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_assign_to_fn_param() {
        let src = r#"
            fn bump(n: Int) {
                n = n + 1;
            }
            fn main() { bump(0); }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("cannot assign to `n`")
                    && d.message.contains("immutable")
            }),
            "expected immutable-binding error on fn-param reassignment, \
             got: {:?}",
            diags
        );
    }

    #[test]
    fn err_assign_to_for_loop_var() {
        let src = r#"
            fn main() {
                for i in 0..3 {
                    i = 99;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("cannot assign to `i`")
                    && d.message.contains("immutable")
            }),
            "expected immutable-binding error on for-loop-var \
             reassignment, got: {:?}",
            diags
        );
    }

    // Field/index reassignment THROUGH an immutable head still
    // allowed — `x.field = ...` mutates state, doesn't rebind x.
    #[test]
    fn ok_field_assign_through_immutable_self() {
        let src = r#"
            locus L {
                params { count: Int = 0; }
                run() {
                    self.count = 7;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on `self.field = ...` in lifecycle; \
             got: {:?}",
            diags
        );
    }

    // F.20 structural interfaces — typechecker recognizes the
    // declaration and enforces the structural-impl rule at every
    // call site where a fn declares an interface-typed param.

    #[test]
    fn ok_locus_satisfies_interface() {
        let src = r#"
            interface Sink {
                fn write(s: String);
                fn line(s: String);
            }
            locus StdoutSinkL {
                params { }
                fn write(s: String) { print(s); }
                fn line(s: String) { println(s); }
            }
            fn render(sink: Sink) { }
            fn main() {
                let s = StdoutSinkL { };
                render(s);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on satisfying locus; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_locus_missing_interface_method() {
        let src = r#"
            interface Sink {
                fn write(s: String);
                fn line(s: String);
            }
            locus BrokenL {
                params { }
                fn write(s: String) { print(s); }
            }
            fn render(sink: Sink) { }
            fn main() {
                let s = BrokenL { };
                render(s);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("does not satisfy interface")
                    && d.message.contains("missing method `line`")
            }),
            "expected missing-method diagnostic, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_string_plus_int_auto_coerces() {
        let src = r#"
            fn main() {
                let port = 8080;
                let msg = "port=" + port;
                println(msg);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on String + Int auto-coerce; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_int_plus_string_auto_coerces() {
        let src = r#"
            fn main() {
                let n = 42;
                let msg = n + " items";
                println(msg);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on Int + String (symmetric); got: {:?}",
            diags
        );
    }

    #[test]
    fn err_locus_interface_arity_mismatch() {
        let src = r#"
            interface Greet {
                fn hello(name: String);
            }
            locus BadArityL {
                params { }
                fn hello(name: String, extra: Int) { }
            }
            fn welcome(g: Greet) { }
            fn main() {
                let g = BadArityL { };
                welcome(g);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("arity does not match interface")
            }),
            "expected arity-mismatch diagnostic, got: {:?}",
            diags
        );
    }
}
