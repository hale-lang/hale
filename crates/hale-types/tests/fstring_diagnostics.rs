//! GH #469 — what the checker says about f-strings, and WHERE.
//!
//! The output of interpolation is asserted in Hale, next to the code
//! (`tests/hale/formatting_test.hl`). What belongs here is the part a
//! running program cannot observe: the diagnostics, and above all
//! their spans.
//!
//! The spans are the point. Before this, every error inside an
//! interpolation reported at `1:1` — the sub-parse ran on a private
//! string whose offsets started at zero, so the caret landed on
//! whatever declaration happened to be at the top of the file. The
//! author was told their type declaration was wrong.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<hale_syntax::error::Diag> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
}

fn errors(src: &str) -> Vec<String> {
    diags(src)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

fn warnings(src: &str) -> Vec<String> {
    diags(src)
        .into_iter()
        .filter(|d| !d.is_error())
        .map(|d| d.message)
        .collect()
}

/// The span, as `(line, col)` of the caret.
fn error_positions(src: &str) -> Vec<(usize, usize)> {
    diags(src)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.span.line_col(src))
        .collect()
}

// ===================== A4: spans ================================

#[test]
fn an_interpolation_error_points_at_the_interpolation() {
    let src = r#"
type Blob { data: Bytes; }
fn main() {
    let b = Blob { data: b"x" };
    println(f"blob = {b}");
}
"#;
    let pos = error_positions(src);
    assert_eq!(pos.len(), 1, "one error, not a duplicate: {:?}", pos);
    // Line 5 is the println; the caret must be on `b`, which is the
    // 23rd column. The old behaviour was (2, 1) — the type decl.
    assert_eq!(pos[0].0, 5, "the error is on the println line");
    let (_, col) = pos[0];
    let line = src.lines().nth(4).unwrap();
    assert_eq!(
        &line[col - 1..col],
        "b",
        "caret should sit on the interpolated expression, got column \
         {} of {:?}",
        col,
        line
    );
}

#[test]
fn the_caret_finds_the_right_one_of_several_interpolations() {
    // The failing interpolation is the THIRD; a fix that merely
    // pointed at "the f-string" would pass a single-interpolation
    // test and still be useless here.
    let src = r#"
type Blob { data: Bytes; }
fn main() {
    let n = 1;
    let m = 2;
    let b = Blob { data: b"x" };
    println(f"{n} {m} {b} {n}");
}
"#;
    let pos = error_positions(src);
    assert_eq!(pos.len(), 1, "{:?}", pos);
    let (line_no, col) = pos[0];
    assert_eq!(line_no, 7);
    let line = src.lines().nth(6).unwrap();
    assert_eq!(&line[col - 1..col], "b", "line was {:?}", line);
}

#[test]
fn a_parse_error_inside_an_interpolation_lands_inside_it() {
    let src = "fn main() { println(f\"x = {1 +}\"); }";
    let program = parse_source(src);
    let e = program.expect_err("`1 +` is not an expression");
    let d = &e[0];
    let (_, col) = d.span.line_col(src);
    assert!(
        col > 25,
        "a parse error inside an interpolation must not report at \
         the start of the file; got column {}",
        col
    );
    assert!(
        d.message.contains("f-string interpolation"),
        "and it should say where it is: {}",
        d.message
    );
}

#[test]
fn a_bad_format_spec_is_a_parse_error_naming_the_grammar() {
    let src = "fn main() { let n = 1; println(f\"{n:zz}\"); }";
    let e = parse_source(src).expect_err("`zz` is not a format kind");
    assert!(
        e[0].message.contains("unknown format kind")
            && e[0].message.contains("[[fill]align][width]"),
        "the message should teach the grammar: {}",
        e[0].message
    );
}

#[test]
fn one_mistake_is_reported_once() {
    // `println(to_string(x))` visits the argument twice, so the same
    // diagnostic used to be emitted twice. Invisible while both
    // copies reported at 1:1; a duplicate pair of real carets is not.
    let src = r#"
type Blob { data: Bytes; }
fn main() {
    let b = Blob { data: b"x" };
    println(to_string(b));
}
"#;
    assert_eq!(errors(src).len(), 1, "{:?}", errors(src));
}

// ===================== A2: what renders =========================

#[test]
fn a_struct_of_printable_fields_renders() {
    let src = r#"
type Point { x: Int; y: Int; }
fn main() {
    let p = Point { x: 1, y: 2 };
    println(f"{p}");
}
"#;
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn rendering_recurses_and_a_bad_field_anywhere_stops_it() {
    let src = r#"
type Inner { blob: Bytes; }
type Outer { inner: Inner; }
fn main() {
    let o = Outer { inner: Inner { blob: b"x" } };
    println(f"{o}");
}
"#;
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("Outer")),
        "an unprintable field two levels down makes the whole record \
         unprintable: {:?}",
        es
    );
}

#[test]
fn a_locus_is_never_printable() {
    // The load-bearing exclusion (GH #436): if a locus rendered,
    // debug-printing one would hand out the `params` of a `@sealed`
    // locus, which is exactly what the seal exists to prevent.
    let src = r#"
locus Svc { params { n: Int = 1; } }
main locus App { params { s: Svc = Svc { }; } }
fn main() { let a = App { }; println(f"{a.s}"); }
"#;
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("Svc")),
        "rendering a locus must stay refused: {:?}",
        es
    );
}

#[test]
fn a_sealed_locus_has_no_printable_path_to_its_params() {
    let src = r#"
@sealed locus Signer { params { key: String = "k"; } }
main locus App { params { s: Signer = Signer { }; } }
fn main() { let a = App { }; println(f"{a.s}"); }
"#;
    assert!(
        !errors(src).is_empty(),
        "a sealed locus must not become renderable through the \
         composite path"
    );
}

#[test]
fn arrays_and_tuples_of_scalars_render() {
    let src = r#"
type Corners { at: [Int; 4]; }
fn main() {
    let c = Corners { at: [1, 2, 3, 4] };
    println(f"{c}");
    println(f"{(1, true, 2.5)}");
}
"#;
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

// ===================== A3: spec vs value ========================

#[test]
fn hex_applies_to_integers_only() {
    let src = r#"fn main() { let s = "x"; println(f"{s:x}"); }"#;
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("hexadecimal")),
        "{:?}",
        es
    );
    let ok = r#"fn main() { let n = 1; println(f"{n:x}"); }"#;
    assert!(errors(ok).is_empty(), "{:?}", errors(ok));
}

#[test]
fn precision_applies_to_fractional_types_only() {
    // Int is excluded deliberately: it is a number, but it has no
    // fractional part, and accepting `.2` there would silently do
    // nothing.
    for bad in [
        r#"fn main() { let s = "x"; println(f"{s:.2}"); }"#,
        r#"fn main() { let n = 1; println(f"{n:.2}"); }"#,
    ] {
        let es = errors(bad);
        assert!(
            es.iter().any(|m| m.contains("precision")),
            "{bad} → {:?}",
            es
        );
    }
    for ok in [
        r#"fn main() { let f = 1.5; println(f"{f:.2}"); }"#,
        r#"fn main() { let d = 1.5d; println(f"{d:.2}"); }"#,
    ] {
        assert!(errors(ok).is_empty(), "{ok} → {:?}", errors(ok));
    }
}

#[test]
fn a_width_applies_to_anything_printable() {
    let src = r#"
type Point { x: Int; y: Int; }
fn main() {
    let p = Point { x: 1, y: 2 };
    println(f"{p:>30}");
}
"#;
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

// ===================== the bonus lint ===========================

#[test]
fn a_plain_string_naming_a_local_in_braces_is_flagged() {
    let src = r#"fn main() { let x = 3; println("x={x}"); }"#;
    let ws = warnings(src);
    assert!(
        ws.iter().any(|m| m.contains("plain string")
            && m.contains("f-string")),
        "{:?}",
        ws
    );
}

#[test]
fn the_lint_stays_quiet_when_the_braces_name_nothing() {
    // Each of these is a program that means what it says. A lint
    // that fires on them is worse than no lint: it trains the reader
    // to ignore the category.
    for quiet in [
        r#"fn main() { println("{}"); }"#,
        r#"fn main() { println("{\"a\": 1}"); }"#,
        r#"fn main() { println("use {name} as the placeholder"); }"#,
        r#"fn main() { let x = 3; println("literal {{x}} brace"); }"#,
        r#"fn main() { let x = 3; println(f"x={x}"); }"#,
        r#"fn main() { let x = 3; println("x=", x); }"#,
    ] {
        let ws = warnings(quiet);
        assert!(
            !ws.iter().any(|m| m.contains("plain string")),
            "{quiet} should not warn: {:?}",
            ws
        );
    }
}

#[test]
fn the_lint_is_a_warning_not_an_error() {
    // Printing braces around a word that happens to be a local is
    // unusual, not illegal.
    let src = r#"fn main() { let x = 3; println("x={x}"); }"#;
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn println_accepts_exactly_what_interpolation_does() {
    // `println` builds a printf format string on a path of its own,
    // so it kept a SECOND copy of the printable rule. Two divergences
    // came out of that during #469 — a struct, and then a `bounded`
    // whose refusal sat FIRST in the match and shadowed the composite
    // arm entirely. Both typechecked and then failed to build.
    //
    // The corpus agreement gate catches this class only for shapes
    // some embedded program happens to contain, and none printed a
    // bounded. So state the invariant directly: if the checker
    // accepts it for interpolation, `println` takes it too.
    let src = r#"
type P { x: Int; y: String; }
type W { id: String; s: bounded[Int; 4]; }
fn main() {
    let p = P { x: 1, y: "a" };
    println(p);
    println(f"{p}");
    println((1, 2), [3, 4], p);
    let mut w = W { id: "w" };
    push(w.s, 7) or raise;
    println(w.s);
    println(w);
}
"#;
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn an_unprintable_component_is_named_not_just_implied() {
    // `type Message { id: String; tags: bounded[String; 32] }` reads
    // as though it qualifies — both field types appear in any list of
    // printable things, and the rule that excludes it (sequences
    // render only with scalar elements) is a level down. Enumerating
    // the printable set and leaving the author to diff it against
    // their declaration is exactly the wrong help here.
    let src = r#"
type Message { id: String; tags: bounded[String; 4]; }
fn main() {
    let m = Message { id: "m" };
    println(f"{m}");
}
"#;
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("`tags`")
            && m.contains("scalar elements")),
        "the diagnostic must name the offending field: {:?}",
        es
    );
}

#[test]
fn a_nested_unprintable_field_is_named_by_path() {
    let src = r#"
type Inner { blob: Bytes; }
type Outer { name: String; inner: Inner; }
fn main() {
    let o = Outer { name: "x", inner: Inner { blob: b"y" } };
    println(f"{o}");
}
"#;
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("`inner`") && m.contains("`blob`")),
        "the path to the offending field should be walkable: {:?}",
        es
    );
}
