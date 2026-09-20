//! Typecheck M3 stage 1 (2026-07-02): stdlib fn-name validation.
//! Within a tabled namespace an unknown name is an error with a
//! did-you-mean; untabled namespaces stay permissive; locus paths
//! and valid names never flag.
//!
//! GH #722 (2026-09-19): plus the conventional-spelling table — a
//! `std::` path that names an operation living as a bare builtin is
//! answered with that builtin's call shape instead of an edit-distance
//! guess.

use hale_syntax::parse_source;
use hale_types::check_program;
use hale_types::stdlib_surface::BUILTIN_SPELLINGS;

fn msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn error_msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

#[test]
fn typo_in_tabled_namespace_is_caught_with_suggestion() {
    let m = msgs(
        r#"
        fn main() {
            let n = std::str::parse_itn("42") or 0;
            println(n);
        }
    "#,
    );
    assert!(
        m.iter().any(|s| s.contains("unknown stdlib function")
            && s.contains("std::str::parse_itn")
            && s.contains("did you mean `std::str::parse_int`")),
        "got: {:?}",
        m
    );
}

#[test]
fn valid_names_do_not_flag() {
    let m = msgs(
        r#"
        fn main() {
            let n = std::str::parse_int("42") or 0;
            let f = std::math::sqrt(4.0);
            let t = std::time::monotonic_ns();
            let b = std::bytes::from_string("x");
            let r = std::io::fs::read_file("/dev/null") or "";
            println(n, f, t, b, r);
        }
    "#,
    );
    assert!(
        !m.iter().any(|s| s.contains("unknown stdlib function")),
        "got: {:?}",
        m
    );
}

#[test]
fn untabled_namespace_stays_permissive() {
    // std::io::sockopt dispatches non-literal names (constant table)
    // — deliberately untabled, so no name errors even for nonsense.
    let m = msgs(
        r#"
        fn main() {
            let v = std::io::sockopt::TOTALLY_MADE_UP();
            println(v);
        }
    "#,
    );
    assert!(
        !m.iter().any(|s| s.contains("unknown stdlib function")),
        "got: {:?}",
        m
    );
}

#[test]
fn locus_paths_never_flag() {
    let m = msgs(
        r#"
        fn main() {
            let b = std::bytes::BytesBuilder { };
            let _ = b;
        }
    "#,
    );
    assert!(
        !m.iter().any(|s| s.contains("unknown stdlib function")),
        "got: {:?}",
        m
    );
}

// ---------------------------------------------------------------
// GH #722: conventional spellings point at the builtin.
// ---------------------------------------------------------------

/// The builtin call shapes the table is allowed to advise, each
/// paired with the concrete call that
/// `every_advised_builtin_shape_typechecks` compiles below — so a row
/// can never advertise an arity the compiler would refuse.
const VERIFIED_SHAPES: &[(&str, &str)] = &[
    ("len(s)", "len(s)"),
    ("len(b)", "len(b)"),
    ("len(a)", "len(a)"),
    ("abs(x)", "abs(0 - 3)"),
    ("min(a, b)", "min(1, 2)"),
    ("max(a, b)", "max(1, 2)"),
    ("to_string(x)", "to_string(42)"),
    ("print(x)", "print(\"x\")"),
    ("println(x)", "println(\"y\")"),
];

/// One reproducer per table row: the path, the argument list to call
/// it with, and the builtin shape the diagnostic must name.
/// `the_table_and_the_reproducers_agree` keeps this in lockstep with
/// `BUILTIN_SPELLINGS`.
const CASES: &[(&[&str], &str, &str)] = &[
    (&["std", "bytes", "len"], "by", "len(b)"),
    (&["std", "cmp", "max"], "1, 2", "max(a, b)"),
    (&["std", "cmp", "min"], "1, 2", "min(a, b)"),
    (&["std", "fmt", "print"], "\"hi\"", "print(x)"),
    (&["std", "fmt", "println"], "\"hi\"", "println(x)"),
    (&["std", "io", "print"], "\"hi\"", "print(x)"),
    (&["std", "io", "println"], "\"hi\"", "println(x)"),
    (&["std", "io", "stdout", "print"], "\"hi\"", "print(x)"),
    (&["std", "io", "stdout", "println"], "\"hi\"", "println(x)"),
    (&["std", "math", "abs"], "0 - 3", "abs(x)"),
    (&["std", "math", "max"], "1, 2", "max(a, b)"),
    (&["std", "math", "min"], "1, 2", "min(a, b)"),
    (&["std", "str", "from_int"], "42", "to_string(x)"),
    (&["std", "str", "len"], "s", "len(s)"),
    (&["std", "str", "length"], "s", "len(s)"),
    (&["std", "str", "to_string"], "42", "to_string(x)"),
    (&["std", "string", "len"], "s", "len(s)"),
    (&["std", "string", "length"], "s", "len(s)"),
    (&["std", "vec", "len"], "arr", "len(a)"),
];

/// The shape an advice string quotes, i.e. the text inside its final
/// pair of backticks.
fn advised_shape(advice: &str) -> &str {
    advice.rsplit('`').nth(1).expect("advice quotes a call shape")
}

#[test]
fn every_conventional_spelling_points_at_its_builtin() {
    for (path, args, shape) in CASES {
        let p = path.join("::");
        let src = format!(
            "fn main() {{
                 let s = \"abc\";
                 let arr: [Int; 3] = [1, 2, 3];
                 let by = std::bytes::from_string(\"abc\");
                 let v = {}({});
                 println(v);
             }}",
            p, args
        );
        let m = msgs(&src);
        let named: Vec<&String> =
            m.iter().filter(|s| s.contains(&format!("`{}`", p))).collect();
        assert!(
            named.iter().any(|s| s.contains(&format!(
                "`{}` is not a stdlib function; ",
                p
            )) && s.contains(&format!("the builtin `{}`", shape))),
            "`{}` should point at `{}`; got: {:?}",
            p,
            shape,
            m
        );
        // The generic hint is what used to mislead here
        // (`std::string::len` drew "did you mean `std::ring`?",
        // `std::math::min` drew "did you mean `std::math::sin`?").
        assert!(
            !named.iter().any(|s| s.contains("did you mean")),
            "`{}` should not also guess by edit distance; got: {:?}",
            p,
            m
        );
    }
}

#[test]
fn the_table_and_the_reproducers_agree() {
    let rows: std::collections::BTreeSet<String> =
        BUILTIN_SPELLINGS.iter().map(|(p, _)| p.join("::")).collect();
    let cases: std::collections::BTreeSet<String> =
        CASES.iter().map(|(p, _, _)| p.join("::")).collect();
    assert_eq!(
        rows, cases,
        "every BUILTIN_SPELLINGS row needs a reproducer above (and \
         vice versa)"
    );
}

#[test]
fn no_row_shadows_a_real_stdlib_function() {
    // The table is consulted before the surface lookup, so a row
    // naming a fn the stdlib actually grows later would hide it.
    // Nothing in the table may resolve to a real name.
    for (path, _) in BUILTIN_SPELLINGS {
        if let Some((surface, fn_idx)) = hale_types::stdlib_surface::lookup(path)
        {
            let name = path[fn_idx];
            assert!(
                !surface.fns.iter().any(|e| e.name == name),
                "`{}` is a real stdlib function and must not be a \
                 conventional-spelling row",
                path.join("::")
            );
        }
    }
}

#[test]
fn every_row_advises_a_verified_shape() {
    for (path, advice) in BUILTIN_SPELLINGS {
        let shape = advised_shape(advice);
        assert!(
            VERIFIED_SHAPES.iter().any(|(s, _)| *s == shape),
            "row `{}` advises `{}`, which no test typechecks",
            path.join("::"),
            shape
        );
    }
}

#[test]
fn every_advised_builtin_shape_typechecks() {
    // The arity claim, proven: each VERIFIED_SHAPES entry appears
    // here as a real call. A wrong arity in the table is a wrong
    // arity here, and the checker refuses it.
    let src = r#"
        fn main() {
            let s = "abc";
            let b = std::bytes::from_string("abc");
            let a: [Int; 3] = [1, 2, 3];
            println(len(s));
            println(len(b));
            println(len(a));
            println(abs(0 - 3));
            println(min(1, 2));
            println(max(1, 2));
            println(to_string(42));
            print("x");
            println("y");
        }
    "#;
    let m = error_msgs(src);
    assert!(m.is_empty(), "the advised shapes must compile; got: {:?}", m);
    for (shape, call) in VERIFIED_SHAPES {
        assert!(
            src.contains(call),
            "shape `{}` claims to be exercised by `{}`, which is not \
             in the program above",
            shape,
            call
        );
    }
}

#[test]
fn the_member_spelling_points_at_the_builtin() {
    // `s.len()` / `s.length` is the same namespace-to-builtin move
    // written as a member access; no candidate field name can bridge
    // it either.
    let m = msgs(
        r#"
        fn main() {
            let s = "abc";
            let n = s.len();
            let l = s.length;
            println(n, l);
        }
    "#,
    );
    for field in ["len", "length"] {
        assert!(
            m.iter().any(|s| s.contains(&format!(
                "no field `{}` on `String`",
                field
            )) && s.contains("the builtin `len(s)`")),
            "`.{}` on a String should point at `len(s)`; got: {:?}",
            field,
            m
        );
    }
}

#[test]
fn an_unrelated_unknown_function_gets_no_builtin_suggestion() {
    // The control: names with no builtin equivalent keep the plain
    // unknown-fn diagnostic. `std::json::parse` is the interesting
    // one — a conventional spelling whose operation genuinely has no
    // builtin, so it must NOT be pointed anywhere.
    for call in ["std::str::frobnicate(\"x\")", "std::json::parse(\"{}\")"] {
        let src = format!("fn main() {{ let v = {}; println(v); }}", call);
        let m = msgs(&src);
        assert!(
            m.iter().any(|s| s.contains("unknown stdlib function")),
            "{} should keep the plain diagnostic; got: {:?}",
            call,
            m
        );
        assert!(
            !m.iter().any(|s| s.contains("is not a stdlib function")
                || s.contains("the builtin")),
            "{} must not gain a builtin suggestion; got: {:?}",
            call,
            m
        );
    }
}
