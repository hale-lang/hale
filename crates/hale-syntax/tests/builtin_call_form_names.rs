//! GH #863 / GH #880: a free `fn` may not be named after a built-in
//! call form.
//!
//! `sum` / `prod` are claimed by the parser at expression head and
//! `min` / `max` by codegen's math-builtin arm, both ahead of any
//! user declaration. So `fn sum(a: Int) -> Int { … }` used to pass
//! `hale check` and fail `hale build` with an unlocated `unsupported
//! in codegen v0` — and a two-arg `fn min(a: Int, b: Int)` was worse
//! still: it BUILT, and silently ran the builtin instead of the body.
//!
//! #863 claimed those four. #880 measured the rest of the bare
//! builtins the same way and found the same two failure modes:
//! `abs`, `to_string`, `Int` and `Float` BUILT and ran the builtin;
//! `len`, `starts_with`, `contains`, `print`, `eprintln` and
//! `__fmt` checked clean and were refused unlocated at build. So
//! the rule now covers every name a call site claims.
//!
//! The rule is on the free DECLARATION, at the name, reported once.
//! A locus METHOD keeps the names — it is reached through a
//! receiver, which no builtin claims, and the stdlib's own
//! `mirror_ring.hl` / `bytes_builder.hl` declare `fn len()`.
//!
//! Names no call site claims stay free, and
//! `no_other_chains_word_is_reserved` /
//! `unclaimed_bare_builtin_names_stay_free` pin those lists so a
//! future tranche cannot quietly widen the rule.
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

/// Every name the compiler claims at a bare call site, with the
/// clause the diagnostic splices in — deliberately a second copy of
/// the parser's `BUILTIN_CALL_FORMS`, so widening the rule means
/// stating the reason twice and a reviewer sees both.
///
/// `None` is the common "every `NAME(...)` call site lowers to the
/// builtin".
const CLAIMED: &[(&str, Option<&str>)] = &[
    // GH #863: the chains / aggregate four.
    ("sum", None),
    ("prod", None),
    ("min", None),
    ("max", None),
    // GH #880: the rest of the bare builtins.
    ("abs", None),
    ("to_string", None),
    (
        "Int",
        Some("`Int(x)` is the built-in Float → Int narrowing cast"),
    ),
    (
        "Float",
        Some("`Float(x)` is the built-in Int → Float widening cast"),
    ),
    (
        "len",
        Some(
            "`len(...)` is polymorphic over String, Bytes and \
             `bounded`, and is answered at the call site ahead of \
             every user fn",
        ),
    ),
    ("starts_with", None),
    ("contains", None),
    ("print", Some(PRINTER_CLAIM)),
    ("println", Some(PRINTER_CLAIM)),
    ("eprint", Some(PRINTER_CLAIM)),
    ("eprintln", Some(PRINTER_CLAIM)),
    (
        "check_closures",
        Some(
            "`check_closures()` is the explicit-epoch closure \
             surface and is answered at statement position ahead of \
             every user fn",
        ),
    ),
    (
        "__fmt",
        Some("`__fmt(...)` is what an f-string interpolation desugars into"),
    ),
];

const PRINTER_CLAIM: &str =
    "the printers are variadic over every printable type, and the \
     Hale-source standard library is merged into this same global fn \
     namespace — the declaration would capture the LIBRARY's own \
     `print(...)` calls";

fn expected_at(line: usize, word: &str, why: Option<&str>) -> String {
    let why = match why {
        Some(w) => w.to_string(),
        None => format!("every `{}(...)` call site lowers to the builtin", word),
    };
    format!(
        "{line}:4 `{w}` is a built-in call form and cannot name a fn; \
         rename it ({why}, so the declaration could never be reached \
         — spec/tokens.md § Built-in identifiers lists them). A locus \
         METHOD may still be named `{w}`: it is reached through a \
         receiver, which no builtin claims.",
        line = line,
        w = word,
        why = why,
    )
}

fn expected(word: &str, why: Option<&str>) -> String {
    expected_at(1, word, why)
}

// === declaration position is refused =========================

#[test]
fn free_fn_named_after_a_claimed_form() {
    // One arg: the shape from the issue. `hale check` said ok and
    // `hale build` then said "unsupported in codegen v0" with no
    // span at all — or, for `abs` / `to_string` / `Int` / `Float`,
    // said nothing and ran the builtin (GH #880).
    for (word, why) in CLAIMED {
        let src = format!(
            "fn {w}(a: Int) -> Int {{\n    return a + 1;\n}}\n\n\
             fn main() {{\n    println(\"{{}}\", {w}(1));\n}}\n",
            w = word
        );
        assert_eq!(only_diag(&src), expected(word, *why), "word: {word}");
    }
}

#[test]
fn two_arg_declaration_reports_only_the_declaration() {
    // `sum(a, b)` at the call site used to be a SECOND diagnostic
    // (`expected ), got Comma`) about the same rename. With the
    // declaration reported, the call parses as the ordinary call it
    // was written as.
    for (word, why) in CLAIMED {
        let src = format!(
            "fn {w}(a: Int, b: Int) -> Int {{\n    return a + b;\n}}\n\n\
             fn main() {{\n    println(\"{{}}\", {w}(1, 2));\n}}\n",
            w = word
        );
        assert_eq!(only_diag(&src), expected(word, *why), "word: {word}");
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
fn every_claimed_name_is_still_legal_as_a_method() {
    // GH #880 widened the list; the receiver carve-out has to widen
    // with it. `crates/hale-stdlib/hl/mirror_ring.hl` and
    // `bytes_builder.hl` declare `fn len()`, and
    // `dna/core/workspace.hl` declares `fn contains(...)` in an
    // interface AND in the locus that serves it — claiming those at
    // the receiver would break the standard library.
    for (word, _) in CLAIMED {
        let src = format!(
            "locus Holder {{\n    params {{ n: Int = 0; }}\n    \
             fn {w}(a: Int) -> Int {{ return self.n + a; }}\n}}\n\
             fn main() {{\n    let h = Holder {{ }};\n    \
             println(\"{{}}\", h.{w}(1));\n}}\n",
            w = word
        );
        assert!(
            diags(&src).is_empty(),
            "`{word}` must stay legal as a method, got: {:?}",
            diags(&src)
        );
    }
}

#[test]
fn an_interface_method_may_keep_a_claimed_name() {
    // `dna/core/workspace.hl` really declares `fn contains(path:
    // String, commit: String) -> Bool;` in an interface.
    let src = "\
interface Repo {
    fn contains(path: String, commit: String) -> Bool;
    fn len() -> Int;
}
fn main() { }
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

#[test]
fn unclaimed_bare_builtin_names_stay_free() {
    // GH #880 probed every name in spec/tokens.md's built-in
    // identifier table plus every entry of
    // `hale_types::check::BARE_BUILTIN_CALLEES`, at one and two
    // arguments, through check + build + a RUN of the binary. These
    // are the ones no call site claims: each one builds and runs its
    // own body, so the rule must NOT take them.
    //
    // `count` is the load-bearing case — `dna/tests/books_slice_
    // test.hl` declares a free `fn count(app, kind, entity, needle)`.
    // The `bounded[T; N]` intrinsics and the accumulator vocabulary
    // are claimed only when the argument IS a bounded receiver, or
    // inside a closure assertion.
    for word in [
        // spec/tokens.md's table, minus the claimed names: the
        // framework spellings and the two conventional ones.
        "B",
        "c",
        "sigma",
        "phi",
        "k_max",
        "span_max",
        "length",
        "empty",
        // bounded[T; N] intrinsics + accumulator vocabulary.
        "count",
        "mean",
        "clear",
        "truncate",
        "push",
        "at",
        "set",
    ] {
        for params in ["a: Int", "a: Int, b: Int"] {
            let src = format!(
                "fn {w}({p}) -> Int {{\n    return 8801;\n}}\n\n\
                 fn main() {{\n    println(\"{{}}\", {w}(1));\n}}\n",
                w = word,
                p = params,
            );
            assert!(
                diags(&src).is_empty(),
                "`{word}` must stay usable as a fn name ({params}), \
                 got: {:?}",
                diags(&src)
            );
        }
    }
}

#[test]
fn a_module_fn_is_refused_for_every_claimed_name() {
    // `module { }` items flatten into the same global fn namespace,
    // so the call site claims them exactly as it claims a top-level
    // declaration. Pinned per name because the module path reaches
    // `parse_fn_decl_with_ffi` through its own call site.
    for (word, why) in CLAIMED {
        let src = format!(
            "module m {{\n    fn {w}(a: Int) -> Int {{ return a; }}\n}}\n\
             fn main() {{ }}\n",
            w = word
        );
        let d = only_diag(&src);
        let want = expected_at(2, word, *why);
        // The module body is indented four spaces, so the name
        // starts at column 8 rather than 4.
        let want = want.replacen("2:4", "2:8", 1);
        assert_eq!(d, want, "word: {word}");
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
