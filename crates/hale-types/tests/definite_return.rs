//! GH #1940: definite return. A body with a declared return type that can
//! reach its end without a value is refused by `hale check`, at the
//! declaration's name, with the path that falls through as a secondary
//! location; a trailing `match`, `if … else` or block is the body's value.
//! Before this, `hale check` said `ok` and `hale build` refused the body
//! with no file or line.

#[path = "support/entries.rs"]
mod entries;

use hale_syntax::Diag;

fn check(src: &str) -> Vec<Diag> {
    entries::check_files(&[("main.hl", src)])
}

/// The definite-return refusals, as (the text the primary span covers,
/// the text the related span starts with, the message).
fn refusals(src: &str) -> Vec<(String, String, String)> {
    check(src)
        .into_iter()
        .filter(|d| d.is_error() && d.message.contains("can reach its end without a value"))
        .map(|d| {
            let at = |s: hale_syntax::Span| {
                let base = src_base(src, &d);
                let (a, b) = (s.start.as_usize() - base, s.end.as_usize() - base);
                src.get(a..b).unwrap_or("?").to_string()
            };
            let related = d.related.first().map(|r| at(r.span)).unwrap_or_default();
            (at(d.span), related, d.message.clone())
        })
        .collect()
}

/// The offset the seed's one file was parsed at: the primary span names
/// the declaration, whose name the source holds once.
fn src_base(src: &str, d: &Diag) -> usize {
    let name = d.message.split('`').nth(1).unwrap_or("");
    let name = name.rsplit('.').next().unwrap_or(name);
    let at = src.find(&format!("fn {name}")).map(|i| i + 3).unwrap_or(0);
    d.span.start.as_usize() - at
}

fn ok(src: &str) {
    let errors: Vec<String> = check(src).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "expected a clean check, got {errors:#?}");
}

#[test]
fn an_if_without_else_is_refused_at_the_fn_naming_the_if() {
    let src = r#"
fn sign(n: Int) -> Int {
    if n > 0 { return 1; }
}

fn main() {
    print(sign(3));
}
"#;
    let found = refusals(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    let (primary, related, msg) = &found[0];
    assert_eq!(primary, "sign", "located at the fn's name");
    assert!(related.starts_with("if n > 0"), "the path is the `if`: {related:?}");
    assert!(msg.contains("fn `sign` returns `Int`"), "{msg}");
    assert!(msg.contains("has no `else`"), "{msg}");
}

#[test]
fn a_match_arm_that_falls_through_is_named() {
    let src = r#"
fn pick(n: Int) -> String {
    match n {
        1 -> "one",
        _ -> { print(n); },
    }
}

fn main() {
    print(pick(1));
}
"#;
    let found = refusals(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    let (primary, related, msg) = &found[0];
    assert_eq!(primary, "pick");
    assert!(related.starts_with("print(n)"), "the arm's last statement: {related:?}");
    assert!(msg.contains("the body ends after this statement"), "{msg}");
}

#[test]
fn a_loop_with_a_break_falls_through_at_the_break() {
    let src = r#"
fn find(n: Int) -> Int {
    let mut i = 0;
    while true {
        if i == n { break; }
        i = i + 1;
    }
}

fn main() {
    print(find(3));
}
"#;
    let found = refusals(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    let (primary, related, msg) = &found[0];
    assert_eq!(primary, "find");
    assert!(related.starts_with("break;"), "the `break` that leaves the loop: {related:?}");
    assert!(msg.contains("`break` leaves the `while true` loop"), "{msg}");
}

#[test]
fn a_conditional_loop_falls_through_at_the_loop() {
    let src = r#"
fn count(n: Int) -> Int {
    let mut i = 0;
    while i < n {
        return i;
    }
}

fn main() {
    print(count(3));
}
"#;
    let found = refusals(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].1.starts_with("while i < n"), "{:?}", found[0]);
}

#[test]
fn two_methods_are_refused_by_their_names() {
    let src = r#"
locus Gauge {
    params { level: Int; }

    fn reading() -> Int {
        if self.level > 0 { return self.level; }
    }

    fn bulk() -> String {
        print("bulk");
    }
}

fn main() {
    let g = Gauge { level: 1 };
    print(g.reading());
}
"#;
    let msgs: Vec<String> = check(src)
        .into_iter()
        .filter(|d| d.message.contains("can reach its end without a value"))
        .map(|d| d.message)
        .collect();
    assert!(msgs.iter().any(|m| m.contains("method `Gauge.reading` returns `Int`")), "{msgs:#?}");
    assert!(msgs.iter().any(|m| m.contains("method `Gauge.bulk` returns `String`")), "{msgs:#?}");
}

#[test]
fn while_true_without_break_never_falls_through() {
    ok(r#"
fn first_over(limit: Int) -> Int {
    let mut i = 0;
    while true {
        for j in 0..3 {
            if j == 1 { break; }
        }
        if i > limit { return i; }
        i = i + 1;
    }
}

fn main() {
    print(first_over(2));
}
"#);
}

#[test]
fn tail_match_if_and_block_are_the_value() {
    ok(r#"
type P { x: Int; }

fn word(n: Int) -> String {
    match n { 1 -> "one", _ -> "other" }
}

fn early(n: Int) -> Int {
    if n > 0 { return 1; } else { 2 }
}

fn rec(n: Int) -> P {
    let d = n * 2;
    { P { x: d } }
}

fn arms(n: Int) -> Int {
    match n {
        0 -> { return 0; },
        1 -> { let y = n + 1; y },
        _ -> n,
    }
}

fn main() {
    print(word(1));
    print(early(1));
    print(rec(1).x);
    print(arms(2));
}
"#);
}

#[test]
fn a_tail_arm_is_typed_against_the_return_type() {
    let src = r#"
fn word(n: Int) -> String {
    match n { 1 -> "one", _ -> 2 }
}

fn main() {
    print(word(1));
}
"#;
    let msgs: Vec<String> = check(src).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(
        msgs.iter().any(|m| m.contains("this is the body's value") && m.contains("expected `String`, got `Int`")),
        "{msgs:#?}"
    );
}

#[test]
fn a_body_with_no_return_type_is_not_judged() {
    ok(r#"
fn log(n: Int) {
    if n > 0 { print(n); }
}

fn main() {
    log(1);
}
"#);
}
