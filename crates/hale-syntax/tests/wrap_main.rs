//! `--wrap-main` AST transform (`desugar::wrap_main_as_wasm_export`).
//!
//! Turns a bare-`main` program into a wasm browser-entry program on the
//! AST — replacing `fn main` with `@export locus __Main { birth() { … } }`
//! and injecting `target wasm` — instead of the brittle source-text rewrite
//! the playground used to do. Span-preserving, string/comment-safe (the
//! real lexer found the body), prefer-explicit.

use hale_syntax::ast::{LifecycleKind, LocusMember, TopDecl};
use hale_syntax::desugar::wrap_main_as_wasm_export;
use hale_syntax::parse_source;

#[test]
fn wraps_bare_main_into_export_locus_plus_target() {
    let src = r#"
        fn main() {
            let msg: String = "hi";
            println(msg);
        }
    "#;
    let mut program = parse_source(src).expect("parse");
    let wrapped = wrap_main_as_wasm_export(&mut program);
    assert!(wrapped, "a bare `fn main` should be wrapped");

    // No `fn main` survives; a `@export locus __Main` with a birth replaces it.
    assert!(
        !program.items.iter().any(|it| matches!(it, TopDecl::Fn(f) if f.name.name == "main")),
        "fn main should be replaced, not kept"
    );
    let locus = program
        .items
        .iter()
        .find_map(|it| match it {
            TopDecl::Locus(l) if l.name.name == "__Main" => Some(l),
            _ => None,
        })
        .expect("synthesized __Main locus");
    assert!(locus.export, "__Main must be @export");
    assert!(!locus.is_main, "__Main is not the `main locus`");
    let birth = locus
        .members
        .iter()
        .find_map(|m| match m {
            LocusMember::Lifecycle(l) if l.kind == LifecycleKind::Birth => Some(l),
            _ => None,
        })
        .expect("birth lifecycle");
    // main's body moved intact: the two statements are under birth.
    assert_eq!(birth.body.stmts.len(), 2, "main's body moved into birth");

    // A `target wasm` gate was injected.
    assert!(
        program.items.iter().any(|it| matches!(it, TopDecl::Target(t) if t.name.name == "wasm")),
        "target wasm gate should be injected"
    );
}

#[test]
fn preserves_main_body_statement_spans() {
    // The whole point: the moved body keeps its original spans, so a
    // diagnostic on the user's line stays on the user's line.
    let src = "fn main() {\n    println(oops);\n}\n";
    let mut program = parse_source(src).expect("parse");
    // Capture the body block's span before the transform.
    let before = match &program.items[0] {
        TopDecl::Fn(f) => f.body.span,
        _ => panic!("expected fn main"),
    };
    wrap_main_as_wasm_export(&mut program);
    let after = program
        .items
        .iter()
        .find_map(|it| match it {
            TopDecl::Locus(l) if l.name.name == "__Main" => l.members.iter().find_map(|m| {
                match m {
                    LocusMember::Lifecycle(li) if li.kind == LifecycleKind::Birth => {
                        Some(li.body.span)
                    }
                    _ => None,
                }
            }),
            _ => None,
        })
        .expect("birth body block");
    assert_eq!(before, after, "body block span must be unchanged (moved intact)");
}

#[test]
fn prefer_explicit_leaves_existing_export_untouched() {
    let src = r#"
        target wasm { }
        @export locus App { birth() { println("x"); } }
        fn main() { println("ignored"); }
    "#;
    let mut program = parse_source(src).expect("parse");
    let wrapped = wrap_main_as_wasm_export(&mut program);
    assert!(!wrapped, "existing @export entry ⇒ no wrap");
    // fn main is left exactly as-is.
    assert!(
        program.items.iter().any(|it| matches!(it, TopDecl::Fn(f) if f.name.name == "main")),
        "fn main should be left untouched when an @export entry exists"
    );
    assert!(
        !program.items.iter().any(|it| matches!(it, TopDecl::Locus(l) if l.name.name == "__Main")),
        "no __Main should be synthesized"
    );
}

#[test]
fn no_main_is_a_noop() {
    let src = "type T { x: Int; }\n";
    let mut program = parse_source(src).expect("parse");
    let before = program.items.len();
    let wrapped = wrap_main_as_wasm_export(&mut program);
    assert!(!wrapped);
    assert_eq!(program.items.len(), before, "no main ⇒ nothing added");
}

#[test]
fn does_not_double_inject_existing_target() {
    // A program with its own `target wasm` + a bare main: wrap the main but
    // don't add a second target.
    let src = "target wasm { }\nfn main() { println(1); }\n";
    let mut program = parse_source(src).expect("parse");
    wrap_main_as_wasm_export(&mut program);
    let targets = program
        .items
        .iter()
        .filter(|it| matches!(it, TopDecl::Target(_)))
        .count();
    assert_eq!(targets, 1, "must not double-inject target wasm");
}
