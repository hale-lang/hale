//! GH #863: a free `fn` may not be named after a built-in call form.
//!
//! `sum` / `prod` are claimed by the parser at expression head and
//! `min` / `max` by codegen's math-builtin arm, both ahead of any
//! user declaration. So `fn sum(a: Int) -> Int { … }` used to pass
//! `hale check` and fail `hale build` with an unlocated `unsupported
//! in codegen v0` — and a two-arg `fn min(a: Int, b: Int)` was worse
//! still: it BUILT, and silently ran the builtin instead of the body.
//!
//! The rule now: the four names are refused in free-`fn` declaration
//! position, at the name, once. A locus METHOD keeps them — it is
//! reached through a receiver, which no builtin claims.
//!
//! The rest of the chains vocabulary is recognized only after a `.`,
//! so those names stay free; `no_other_chains_word_is_reserved` pins
//! that list so a future tranche cannot quietly widen it.
//!
//! Spans are asserted as `line:col` through the renderer, which is
//! what the author actually sees.

use hale_syntax::parse_source;

/// Every diagnostic a source produces, as `line:col message`.
fn diags(src: &str) -> Vec<String> {
    match parse_source(src) {
        Ok(_) => Vec::new(),
        Err(ds) => ds
            .iter()
            .map(|d| {
                let (line, col) = d.span.line_col(src);
                format!("{}:{} {}", line, col, d.message)
            })
            .collect(),
    }
}

/// The one diagnostic, or a panic naming everything that came out.
fn only_diag(src: &str) -> String {
    let ds = diags(src);
    assert_eq!(
        ds.len(),
        1,
        "expected exactly one diagnostic, got {}:\n{}",
        ds.len(),
        ds.join("\n")
    );
    ds.into_iter().next().unwrap()
}

/// The four names the compiler claims at a bare call site.
const CLAIMED: [&str; 4] = ["sum", "prod", "min", "max"];

fn expected(word: &str) -> String {
    format!(
        "1:4 `{w}` is a built-in call form and cannot name a fn; \
         rename it (every `{w}(...)` call site lowers to the builtin, \
         so the declaration could never be reached — spec/tokens.md \
         § Built-in identifiers lists them). A locus METHOD may still \
         be named `{w}`: it is reached through a receiver, which no \
         builtin claims.",
        w = word
    )
}

// === declaration position is refused =========================

#[test]
fn free_fn_named_after_a_claimed_form() {
    // One arg: the shape from the issue. `hale check` said ok and
    // `hale build` then said "unsupported in codegen v0" with no
    // span at all.
    for word in CLAIMED {
        let src = format!(
            "fn {w}(a: Int) -> Int {{\n    return a + 1;\n}}\n\n\
             fn main() {{\n    println(\"{{}}\", {w}(1));\n}}\n",
            w = word
        );
        assert_eq!(only_diag(&src), expected(word), "word: {word}");
    }
}

#[test]
fn two_arg_declaration_reports_only_the_declaration() {
    // `sum(a, b)` at the call site used to be a SECOND diagnostic
    // (`expected ), got Comma`) about the same rename. With the
    // declaration reported, the call parses as the ordinary call it
    // was written as.
    for word in CLAIMED {
        let src = format!(
            "fn {w}(a: Int, b: Int) -> Int {{\n    return a + b;\n}}\n\n\
             fn main() {{\n    println(\"{{}}\", {w}(1, 2));\n}}\n",
            w = word
        );
        assert_eq!(only_diag(&src), expected(word), "word: {word}");
    }
}

#[test]
fn generic_and_decorated_declarations_are_refused_too() {
    // The decorated path (`@hot fn …`) and the generic path reach a
    // different `parse_fn_decl_with_ffi` call site each.
    let src = "@hot\nfn sum(a: Int) -> Int {\n    return a;\n}\nfn main() { }\n";
    let d = only_diag(src);
    assert!(d.starts_with("2:4 `sum` is a built-in call form"), "got: {d}");

    let src = "fn max<T>(a: T) -> T {\n    return a;\n}\nfn main() { }\n";
    let d = only_diag(src);
    assert!(d.starts_with("1:4 `max` is a built-in call form"), "got: {d}");
}

#[test]
fn a_module_fn_is_a_free_fn() {
    // `module { }` items resolve into the same global fn namespace
    // (resolve.rs flattens them), so the same call site claims them.
    let src = "module m {\n    fn sum(a: Int) -> Int { return a; }\n}\nfn main() { }\n";
    let d = only_diag(src);
    assert!(d.starts_with("2:8 `sum` is a built-in call form"), "got: {d}");
}

// === what stays legal ========================================

#[test]
fn a_locus_method_may_keep_the_name() {
    // `tests/hale/intra_locus_publish_reclaim_test.hl` declares
    // exactly this and calls it as `self.sub.sum()`.
    let src = "\
locus Bucket {
    params { total: Int = 7; }
    fn sum() -> Int { return self.total; }
    fn min(a: Int, b: Int) -> Int { return a; }
}
fn main() {
    let b = Bucket { };
    println(\"{}\", b.sum());
}
";
    assert!(diags(src).is_empty(), "got: {:?}", diags(src));
}

#[test]
fn an_interface_method_may_keep_the_name() {
    let src = "\
interface Totals {
    fn sum() -> Int;
}
fn main() { }
";
    assert!(diags(src).is_empty(), "got: {:?}", diags(src));
}

#[test]
fn the_builtin_call_forms_still_parse() {
    // Nothing declares the name, so `sum(...)` / `prod(...)` keep
    // their dedicated AST nodes and `min` / `max` keep their calls.
    let src = "\
locus Closed {
    params { n: Int = 0; }
    closure within_band {
        sum(self.n) ~~ 0 within 100;
        epoch tick;
    }
    fn go() -> Int { return min(1, 2) + max(3, 4); }
}
fn main() { }
";
    assert!(diags(src).is_empty(), "got: {:?}", diags(src));

    // And the AST node the production exists for is still built.
    let prog = parse_source(src).expect("parses");
    assert!(
        format!("{:?}", prog).contains("Sum("),
        "`sum(self.n)` must still lower to Expr::Sum"
    );
}

#[test]
fn chain_terminals_still_parse() {
    let src = "\
type Row {
    v: Int = 0;
}

@form(vec)
locus Nums {
    capacity {
        heap rows of Row;
    }
}
fn main() {
    let xs = Nums { };
    println(\"{}\", xs.filter(it > 2).sum());
    println(\"{}\", xs.filter(it > 2).min());
    println(\"{}\", xs.map(it * 2).count());
}
";
    assert!(diags(src).is_empty(), "got: {:?}", diags(src));
}

#[test]
fn no_other_chains_word_is_reserved() {
    // The true list, pinned: everything else in the chains
    // vocabulary is recognized only after a `.`, so a free fn may
    // take the name. `crates/hale-codegen/tests/fixtures/
    // lib-enum-persp/lib.hl` really does declare `fn first()`.
    for word in [
        "map",
        "filter",
        "count",
        "into",
        "any",
        "all",
        "first",
        "find",
        "each",
        "take",
        "skip",
        "enumerate",
        "sort_into",
        "reverse_into",
        "group_count_into",
    ] {
        let src = format!(
            "fn {w}(a: Int) -> Int {{\n    return a + 1;\n}}\n\n\
             fn main() {{\n    println(\"{{}}\", {w}(1));\n}}\n",
            w = word
        );
        assert!(
            diags(&src).is_empty(),
            "`{word}` must stay usable as a fn name, got: {:?}",
            diags(&src)
        );
    }
}

// === the bare two-arg call, with nothing declared ============

#[test]
fn a_multi_arg_aggregate_says_what_claimed_the_name() {
    // Without a declaration in the file the parser still owns
    // `sum(`; the message used to be `expected ), got Comma`.
    let src = "fn main() {\n    println(\"{}\", sum(1, 2));\n}\n";
    let d = only_diag(src);
    assert!(
        d.starts_with("2:24 `sum(...)` is a built-in aggregate over ONE argument"),
        "got: {d}"
    );
    let src = "fn main() {\n    println(\"{}\", prod(1, 2));\n}\n";
    let d = only_diag(src);
    assert!(
        d.starts_with("2:25 `prod(...)` is a built-in aggregate over ONE argument"),
        "got: {d}"
    );
}
