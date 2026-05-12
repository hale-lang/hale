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
    fn ok_approx_within_as_idents_outside_closure() {
        // F.10-style contextual narrowing (2026-05-11): `approx`
        // and `within` are not reserved at the lexer level, so
        // they can appear as free-fn / let-binding identifiers
        // outside closure bodies. Resolves
        // notes/aperio-friction.md 2026-05-10
        // closure-keyword-shadows-helper-ident.
        let src = r#"
            fn approx(actual: Float, expected: Float, eps: Float) -> Bool {
                let diff = actual - expected;
                let within = -eps;
                return diff > within;
            }
            fn main() {
                let ok = approx(3.14, 3.14159, 0.01);
                println("ok=", ok);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected `approx` / `within` to parse as idents; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_approx_keyword_inside_closure_still_works() {
        // The contextual narrowing must still admit the
        // long-form `approx` spelling inside closure assertions
        // (alongside the `~~` operator). `approx` is the infix
        // operator-keyword: `LEFT approx RIGHT within TOL`.
        let src = r#"
            locus L {
                params { x: Int = 0; }
                closure stays_low {
                    self.x approx 0 within 100;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected long-form `approx` inside closure to parse; got: {:?}",
            diags
        );
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

    // === v1.x-FORM-1 PR2 fallible typecheck =============

    #[test]
    fn err_fallible_call_not_addressed_in_let() {
        let src = r#"
            type E { msg: String; }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn main() {
                let v = parse("42");
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("error not addressed")),
            "expected error-not-addressed diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_fallible_call_not_addressed_in_expr_stmt() {
        let src = r#"
            type E { }
            fn doit() -> Int fallible(E) { return 0; }
            fn main() {
                doit();
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("error not addressed")),
            "expected error-not-addressed diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_fallible_addressed_via_or_raise() {
        let src = r#"
            type E { }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn main() {
                let v = parse("42") or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on `or raise`, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_fallible_addressed_via_or_substitute() {
        let src = r#"
            type E { }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn main() {
                let v = parse("42") or 99;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on `or 99`, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_fallible_substitute_type_mismatch() {
        let src = r#"
            type E { }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn main() {
                let v = parse("42") or "not an int";
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("does not match success type")),
            "expected substitute-type-mismatch diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_err_binding_in_or_substitute_rhs() {
        let src = r#"
            type E { code: Int; }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn handle(e: E) -> Int { return e.code; }
            fn main() {
                let v = parse("42") or handle(err);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on `or handle(err)`, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_or_on_non_fallible_expression() {
        let src = r#"
            fn main() {
                let v = 1 + 1 or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("expects a fallible-typed")),
            "expected non-fallible-or diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_fail_with_matching_payload_type() {
        let src = r#"
            type E { code: Int; }
            fn parse(s: String) -> Int fallible(E) {
                fail E { code: 1 };
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on matching-payload fail, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_fail_payload_type_mismatch() {
        let src = r#"
            type E { code: Int; }
            type Other { msg: String; }
            fn parse(s: String) -> Int fallible(E) {
                fail Other { msg: "wrong type" };
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("fail: expected payload")),
            "expected fail-payload-type-mismatch diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_right_associative_chain_typechecks() {
        let src = r#"
            type E { }
            fn a() -> Int fallible(E) { return 0; }
            fn b() -> Int fallible(E) { return 0; }
            fn main() {
                let v = a() or b() or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on chain, got: {:?}",
            diags
        );
    }

    // === v1.x-FORM-1 PR3 form-shape verification ========

    #[test]
    fn ok_form_vec_with_correct_shape() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() { ItemListL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on @form(vec) with heap slot, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_with_pool_slot() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { pool items of Int; }
            }
            fn main() { ItemListL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("@form(vec) requires a `heap` slot")),
            "expected pool-rejected diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_with_no_capacity() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                params { x: Int = 0; }
            }
            fn main() { ItemListL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("found no `capacity")),
            "expected missing-capacity diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_with_multiple_slots() {
        let src = r#"
            @form(vec)
            locus L {
                capacity {
                    heap a of Int;
                    heap b of Int;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("exactly one `heap`")),
            "expected multiple-slots diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_with_args() {
        let src = r#"
            @form(vec, cap = 64)
            locus L {
                capacity { heap items of Int; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("@form(vec) takes no arguments")),
            "expected vec-no-args diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_not_yet_implemented() {
        let src = r#"
            @form(hashmap)
            locus L {
                capacity { pool entries of Int; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("not yet implemented")),
            "expected hashmap-deferred diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_unknown_name() {
        let src = r#"
            @form(banana)
            locus L { }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("unknown form")),
            "expected unknown-form diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_locus_without_form_unaffected() {
        // Regression guard: locus declarations without @form
        // are completely unaffected by the form-shape checks.
        let src = r#"
            locus L {
                capacity { pool entries of Int; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "non-form locus regressed, got: {:?}",
            diags
        );
    }

    // === v1.x-FORM-1 PR3b form-method-synthesis ===========

    #[test]
    fn ok_form_vec_push_resolves() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() {
                let l = ItemListL { };
                l.push(42);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "synthesized push should resolve, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_get_fallible_addressed() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() {
                let l = ItemListL { };
                let v = l.get(0) or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "get + or raise should typecheck, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_get_not_addressed() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() {
                let l = ItemListL { };
                let v = l.get(0);
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("error not addressed")),
            "expected error-not-addressed on bare get(), got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_pop_substitute_with_typed_err_handler() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn fallback(e: IndexError) -> Int { return -1; }
            fn main() {
                let l = ItemListL { };
                let v = l.pop() or fallback(err);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "pop + or handler(err) should typecheck (err typed as IndexError), \
             got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_len_and_is_empty_synthesized() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() {
                let l = ItemListL { };
                let n = l.len();
                let e = l.is_empty();
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "synthesized len/is_empty should resolve, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_with_struct_cell_type() {
        // Cell type can be a user-defined struct; synthesized
        // methods carry that T through.
        let src = r#"
            type Pair { x: Int; y: Int; }
            @form(vec)
            locus PairsL {
                capacity { heap items of Pair; }
            }
            fn main() {
                let l = PairsL { };
                l.push(Pair { x: 1, y: 2 });
                let p = l.get(0) or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "@form(vec) over a struct cell should typecheck, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_index_error_type_in_scope_when_form_used() {
        // The synthesized IndexError type is callable as an
        // ordinary type in user code when any form is used.
        let src = r#"
            @form(vec)
            locus L {
                capacity { heap items of Int; }
            }
            fn inspect(e: IndexError) -> Int { return e.index; }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "IndexError should be in scope when form is used, got: {:?}",
            diags
        );
    }
}
