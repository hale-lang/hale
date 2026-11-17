//! GH #1995 E3: `if let` and `let … else` parse to the matches they mean,
//! so every later pass sees an ordinary `match` (its `form` says only
//! which spelling it came from); a chain with no `if let` parses as it
//! always did; `hale fmt` keeps the new forms as written.

use hale_syntax::ast::*;
use hale_syntax::fmt::format_source;
use hale_syntax::parse_source;

/// The body of `fn main` around `body`.
fn body_of(body: &str) -> Block {
    let src = format!("fn main() {{\n{body}\n}}\n");
    let p = parse_source(&src).unwrap_or_else(|d| panic!("parses: {d:?}\n{src}"));
    let TopDecl::Fn(f) = p.items.into_iter().next().expect("an item") else { panic!("a fn") };
    f.body
}

fn parse_error(body: &str) -> String {
    let src = format!("fn main() {{\n{body}\n}}\n");
    let d = parse_source(&src).expect_err("refused");
    d[0].message.clone()
}

fn arm_is_empty_block(arm: &MatchArm) -> bool {
    matches!(&arm.body, MatchArmBody::Block(b) if b.stmts.is_empty() && b.tail.is_none())
}

#[test]
fn an_if_let_statement_is_a_two_arm_match() {
    let b = body_of("    if let E::A(x) = e { f(x); }");
    let Stmt::Match(m) = &b.stmts[0] else { panic!("a match: {:?}", b.stmts[0]) };
    assert_eq!(m.form, MatchForm::IfLet);
    assert_eq!(m.arms.len(), 2);
    assert!(matches!(&m.arms[0].pattern, Pattern::Constructor { args, .. } if args.len() == 1));
    assert!(matches!(&m.arms[0].body, MatchArmBody::Block(b) if b.stmts.len() == 1));
    assert!(matches!(m.arms[1].pattern, Pattern::Wildcard(_)));
    assert!(arm_is_empty_block(&m.arms[1]), "no else: an empty block");
    assert!(m.arms.iter().all(|a| a.guard.is_none()));

    let b = body_of("    if let E::A(x) = e { f(x); } else { g(); }");
    let Stmt::Match(m) = &b.stmts[0] else { panic!("a match") };
    assert_eq!(m.form, MatchForm::IfLetElse);
    assert!(matches!(&m.arms[1].body, MatchArmBody::Block(b) if b.stmts.len() == 1));
}

#[test]
fn an_if_let_value_has_block_values_for_arms() {
    let b = body_of("    let v = if let E::A(x) = e { x } else { 0 };");
    let Stmt::Let { value: Expr::Match(m), .. } = &b.stmts[0] else { panic!("a match value") };
    assert_eq!(m.form, MatchForm::IfLetElse);
    assert!(m.arms.iter().all(|a| matches!(&a.body, MatchArmBody::Expr(Expr::Block(b)) if b.tail.is_some())));

    // a trailing value-producing `if let` is the block's tail, as an `if` is
    let b = body_of("    if let E::A(x) = e { x } else { 0 }");
    assert!(b.stmts.is_empty());
    assert!(matches!(b.tail.as_deref(), Some(Expr::Match(m)) if m.form == MatchForm::IfLetElse));
    // one with no `else` stays a statement
    let b = body_of("    if let E::A(x) = e { x }");
    assert!(b.tail.is_none() && matches!(&b.stmts[0], Stmt::Match(_)));
}

#[test]
fn an_or_pattern_heads_an_if_let() {
    let b = body_of("    if let E::A(x) | E::B(x) = e { f(x); }");
    let Stmt::Match(m) = &b.stmts[0] else { panic!("a match") };
    assert!(matches!(&m.arms[0].pattern, Pattern::Or(alts, _) if alts.len() == 2));
    // the scrutinee's `|` is the operator
    let b = body_of("    if let 3 = a | b { f(); }");
    let Stmt::Match(m) = &b.stmts[0] else { panic!("a match") };
    assert!(matches!(m.scrutinee, Expr::Binary { .. }));
}

#[test]
fn a_chain_without_if_let_parses_as_it_did() {
    let b = body_of("    if a { f(); } else if b { g(); } else { h(); }");
    let Stmt::If(i) = &b.stmts[0] else { panic!("an if") };
    let Some(ElseBranch::ElseIf(n)) = i.else_block.as_deref() else { panic!("else if") };
    assert!(matches!(n.else_block.as_deref(), Some(ElseBranch::Else(_))));
    let b = body_of("    if a { 1 } else if b { 2 } else { 3 }");
    assert!(matches!(b.tail.as_deref(), Some(Expr::If(_))));
}

#[test]
fn chains_mix_if_and_if_let_links() {
    // a plain `if`, then `else if let`: the `if let` is the `else` block's statement
    let b = body_of("    if a { f(); } else if let E::A(x) = e { g(x); } else { h(); }");
    let Stmt::If(i) = &b.stmts[0] else { panic!("an if") };
    let Some(ElseBranch::Else(blk)) = i.else_block.as_deref() else { panic!("an else block") };
    assert!(matches!(&blk.stmts[..], [Stmt::Match(m)] if m.form == MatchForm::IfLetElse));
    // an `if let`, then `else if`: the `if` is the `_` arm's statement
    let b = body_of("    if let E::A(x) = e { f(x); } else if a { g(); }");
    let Stmt::Match(m) = &b.stmts[0] else { panic!("a match") };
    assert!(matches!(&m.arms[1].body, MatchArmBody::Block(blk) if matches!(&blk.stmts[..], [Stmt::If(_)])));
    // the same chain as a value: each link is the `else` block's value
    let b = body_of("    let v = if a { 1 } else if let E::A(x) = e { x } else { 0 };");
    let Stmt::Let { value: Expr::If(i), .. } = &b.stmts[0] else { panic!("an if value") };
    let Some(ElseBranch::Else(blk)) = i.else_block.as_deref() else { panic!("an else block") };
    assert!(blk.stmts.is_empty() && matches!(blk.tail.as_deref(), Some(Expr::Match(_))));
}

#[test]
fn let_else_binds_the_patterns_names_after_it() {
    // one name: `let x = match e { P -> x, _ -> { … } };`
    let b = body_of("    let E::A(x) = e else { return; };");
    let Stmt::Let { name, value: Expr::Match(m), is_mut: false, .. } = &b.stmts[0] else { panic!("a let") };
    assert_eq!(name.name, "x");
    assert_eq!(m.form, MatchForm::LetElse);
    assert!(matches!(&m.arms[0].body, MatchArmBody::Expr(Expr::Ident(i)) if i.name == "x"));
    assert!(matches!(&m.arms[1].body, MatchArmBody::Block(b) if matches!(&b.stmts[..], [Stmt::Return(..)])));

    // several: a tuple of them
    let b = body_of("    let (a, E::B(c), 1) = t else { return; };");
    let Stmt::LetTuple { names, value: Expr::Match(m), .. } = &b.stmts[0] else { panic!("a tuple let") };
    assert_eq!(names.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["a", "c"]);
    assert!(matches!(&m.arms[0].body, MatchArmBody::Expr(Expr::Tuple(es, _)) if es.len() == 2));

    // none: a statement match
    let b = body_of("    let E::C = e else { return; };");
    assert!(matches!(&b.stmts[0], Stmt::Match(m) if m.form == MatchForm::LetElse && arm_is_empty_block(&m.arms[0])));

    // a name or a flat tuple of names takes an `else` too
    let b = body_of("    let x = e else { return; };");
    assert!(matches!(&b.stmts[0], Stmt::Let { value: Expr::Match(m), .. } if m.form == MatchForm::LetElse));
    let b = body_of("    let mut E::A(x) = e else { return; };");
    assert!(matches!(&b.stmts[0], Stmt::Let { is_mut: true, .. }));
}

#[test]
fn a_pattern_let_without_else_is_a_one_arm_match() {
    let b = body_of("    let E::A(x) = e;");
    let Stmt::Let { value: Expr::Match(m), .. } = &b.stmts[0] else { panic!("a let") };
    assert_eq!(m.form, MatchForm::Let);
    assert_eq!(m.arms.len(), 1, "no `_` arm: the check's exhaustiveness decides");

    // the two forms a `let` always took are unchanged
    let b = body_of("    let (a, b,) = t;\n    let x: Int = 1;\n    let (c, _) = t;");
    assert!(matches!(&b.stmts[0], Stmt::LetTuple { value: Expr::Ident(_), names, .. } if names.len() == 2));
    assert!(matches!(&b.stmts[1], Stmt::Let { value: Expr::Literal(..), ty: Some(_), .. }));
    assert!(matches!(&b.stmts[2], Stmt::LetTuple { value: Expr::Ident(_), .. }));
}

#[test]
fn a_pattern_let_takes_no_type() {
    assert!(parse_error("    let E::A(x): Int = e else { return; };").contains("a `let` with a pattern takes no type"));
}

#[test]
fn fmt_keeps_the_forms_as_written() {
    let src = "fn f(e: E) -> Int {\n    let E::A(x) = e else {\n        return 0;\n    };\n    if let E::B(y) | E::C(y) = e {\n        return y;\n    } else if let E::D = e {\n        return 1;\n    }\n    let v = if let E::A(z) = e { z } else { 0 };\n    return x + v;\n}\n";
    assert_eq!(format_source(src).expect("formats"), src);
    let squashed = "fn f(e: E) -> Int {\n    let E::A(x)=e else {return 0;};\n    if let E::B(y)|E::C(y)=e {return y;}\n    return x;\n}\n";
    let formatted = format_source(squashed).expect("formats");
    assert!(formatted.contains("let E::A(x) = e else { return 0; };"), "{formatted}");
    assert!(formatted.contains("if let E::B(y) | E::C(y) = e { return y; }"), "{formatted}");
    assert_eq!(format_source(&formatted).expect("formats again"), formatted, "a fixed point");
}
