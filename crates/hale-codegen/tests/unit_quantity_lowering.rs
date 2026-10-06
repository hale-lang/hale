//! GH #1076, step U3: a change of denomination is lowered from its row.
//!
//! The checker records every place a quantity's or a point's value
//! changes denomination in the typed bodies' `conversions` column, with
//! the exact factor, a point's shift and the policy; `lower_scale` reads
//! the row and decides nothing. These pin the emitted shape, as the IR
//! text of one function before the optimizer runs: a widening is one
//! multiplication; a narrowing one division, rounded by its policy (each
//! of the five one shape) or checked exact for its `or`'s join (an
//! `InexactError` on the error path); `.split(u)` a floored division and
//! its remainder; a point across origins a shift; a literal its
//! converted count, with no arithmetic at all. A factor no `Int` holds is
//! a located error, never a wrap.

use hale_codegen::CodegenError;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const DECLS: &str = "unit nsec;\nunit usec = 1_000 nsec;\nunit msec = 1_000 usec;\nunit sec = 1_000 msec;\n\
                     unit mK;\nunit K = 1_000 mK;\n\
                     type Span = quantity Int in nsec;\ntype Seconds = Span in sec;\n\
                     type TempDelta = quantity Int in mK;\ntype Kelvin = point TempDelta;\n\
                     type Celsius = point TempDelta { origin: 273_150 mK; }\n\
                     fn to_zero(e: InexactError) -> Seconds { return 0sec; }\n";

/// The IR of `conv`, a fn whose body is `body`, before optimization.
fn conv_ir(signature: &str, body: &str) -> String {
    conv_ir_after("", signature, body)
}

/// The same, with `decls` declared beside the common ones.
fn conv_ir_after(decls: &str, signature: &str, body: &str) -> String {
    let src = format!("{DECLS}{decls}fn conv{signature} {{\n{body}}}\nfn main() {{ println(1); }}\n");
    let bin = harness::unique_bin("unit_quantity_lowering");
    let ir = harness::build_source_ir_text(&src, &bin).unwrap_or_else(|e| panic!("lowers: {e:?}\n{src}"));
    let _ = std::fs::remove_file(&bin);
    let start = ir
        .lines()
        .position(|l| l.starts_with("define") && l.contains("conv("))
        .unwrap_or_else(|| panic!("no `conv` in the module:\n{src}"));
    ir.lines().skip(start).take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
}

/// How many instructions of `fun` carry `op`.
fn count(fun: &str, op: &str) -> usize {
    fun.lines().filter(|l| l.contains(op)).count()
}

#[test]
fn a_widening_is_one_multiplication() {
    let fun = conv_ir("(s: Seconds) -> Span", "    return s;\n");
    assert!(fun.contains("%unit.scale = mul i64 %s1, 1000000000"), "{fun}");
    assert_eq!(count(&fun, "sdiv"), 0, "{fun}");
    assert_eq!(count(&fun, "icmp"), 0, "{fun}");
}

#[test]
fn a_literal_is_its_converted_count() {
    let fun = conv_ir("() -> Span", "    let d: Span = 3sec;\n    return d;\n");
    assert!(fun.contains("i64 3000000000"), "the count, converted at compile time: {fun}");
    assert_eq!(count(&fun, "unit."), 0, "no arithmetic: {fun}");
}

/// Each rounding is the truncating quotient and remainder, then one
/// correction: none (`trunc`), down on a negative remainder (`floor`), up
/// on a positive one (`ceil`), away from zero at half or more (`half_up`),
/// and past half or at half to an even quotient (`half_even`).
#[test]
fn each_rounding_is_one_shape() {
    let shape = |policy: &str| conv_ir("(d: Span) -> Seconds", &format!("    return Seconds(d) or {policy};\n"));
    for policy in ["trunc", "floor", "ceil", "half_up", "half_even"] {
        let fun = shape(policy);
        assert!(fun.contains("%unit.quo = sdiv i64 %d1, 1000000000"), "{policy}: {fun}");
        assert!(fun.contains("%unit.rem = srem i64 %d1, 1000000000"), "{policy}: {fun}");
        assert_eq!(count(&fun, "br i1"), 0, "{policy} is total, it does not branch: {fun}");
    }
    let trunc = shape("trunc");
    assert_eq!(count(&trunc, "icmp"), 0, "{trunc}");
    let floor = shape("floor");
    assert!(floor.contains("%unit.floor.below = icmp slt i64 %unit.rem, 0"), "{floor}");
    assert!(floor.contains("%unit.floor.adj = zext i1 %unit.floor.below to i64"), "{floor}");
    assert!(floor.contains("%unit.floor = sub i64 %unit.quo, %unit.floor.adj"), "{floor}");
    let ceil = shape("ceil");
    assert!(ceil.contains("%unit.ceil.above = icmp sgt i64 %unit.rem, 0"), "{ceil}");
    assert!(ceil.contains("%unit.ceil = add i64 %unit.quo, %unit.ceil.adj"), "{ceil}");
    let half_up = shape("half_up");
    assert!(half_up.contains("%unit.half.away = icmp sge i64 %unit.half.mag, %unit.half.other"), "{half_up}");
    assert!(half_up.contains("%unit.half = add i64 %unit.quo, %unit.half.adj"), "{half_up}");
    let half_even = shape("half_even");
    assert!(half_even.contains("%unit.half.past = icmp sgt i64 %unit.half.mag, %unit.half.other"), "{half_even}");
    assert!(half_even.contains("%unit.half.tie = icmp eq i64 %unit.half.mag, %unit.half.other"), "{half_even}");
    assert!(half_even.contains("%unit.half.odd = icmp ne i64 %unit.half.low, 0"), "{half_even}");
}

/// `or <value>`, `or handler(err)`: exact, or the error path with the
/// `InexactError`, joined by the `or`.
#[test]
fn a_checked_division_is_exact_or_the_error_path() {
    let fun = conv_ir("(d: Span) -> Seconds", "    return Seconds(d) or 0sec;\n");
    assert!(fun.contains("%unit.inexact = icmp ne i64 %unit.rem, 0"), "{fun}");
    assert!(fun.contains("br i1 %unit.inexact, label %unit.err, label %unit.ok"), "{fun}");
    assert!(fun.contains("InexactError.alloc"), "the error path builds the InexactError: {fun}");
    assert!(fun.contains("%or.result = phi i64"), "the substitute joins: {fun}");
    let handled = conv_ir("(d: Span) -> Seconds", "    return Seconds(d) or to_zero(err);\n");
    assert!(handled.contains("@to_zero("), "{handled}");
}

#[test]
fn a_split_is_a_floored_division_and_its_remainder() {
    let fun = conv_ir("(d: Span) -> Int", "    let (s, rest) = d.split(sec);\n    return s;\n");
    assert!(fun.contains("%unit.split.quo = sdiv i64"), "{fun}");
    assert!(fun.contains("%unit.split.whole = sub i64 %unit.split.quo, %unit.split.adj"), "{fun}");
    assert!(fun.contains("%unit.split.rest = add i64 %unit.split.rem, %unit.split.lift"), "{fun}");
}

/// U3 polish (A): a literal in a default is converted per evaluation.
/// `L`'s default `Seconds(2000msec)` is evaluated twice in `conv`: at
/// `l`, where it is the cast and the literal its count in `sec`, 2; at
/// `m`, where a local `Seconds` holding `fake` shadows the type, so the
/// literal is `fake`'s `Span` argument, its count in `nsec`.
#[test]
fn a_literal_in_a_default_is_each_evaluations_count() {
    let fun = conv_ir_after(
        "type L { b: Seconds = Seconds(2000msec); }\nfn fake(d: Span) -> Seconds { return 7sec; }\n",
        "() -> Int",
        "    let l = L {};\n    {\n        let Seconds = fake;\n        let m = L {};\n        println(m.b / 1sec);\n    }\n    return l.b / 1sec;\n",
    );
    assert!(fun.contains("store i64 2, ptr %L.b.ptr,"), "the cast's evaluation stores the count in `sec`: {fun}");
    let calls: Vec<&str> = fun.lines().filter(|l| l.contains("%fnptr.call = call i64 %Seconds")).collect();
    assert_eq!(calls.len(), 1, "one call, the local's: {fun}");
    assert!(calls[0].ends_with(", i64 2000000000)"), "the local is handed the count in `nsec`: {fun}");
}

/// The text of the function `@name` in `ir`, from its `define` to its
/// closing brace.
fn function_ir<'a>(ir: &'a str, name: &str) -> Option<String> {
    let head = format!("@{name}(");
    let start = ir.lines().position(|l| l.starts_with("define") && l.contains(&head))?;
    Some(ir.lines().skip(start).take_while(|l| *l != "}").collect::<Vec<_>>().join("\n"))
}

const SECONDS: &str = "unit msec;\nunit sec = 1_000 msec;\ntype Span = quantity Int in msec;\ntype Seconds = Span in sec;\n";

/// U3 polish (B): lowering reads a row from the body it is emitting. The
/// stdlib parses at base 0, so a stdlib expression can stand at the very
/// span of a user expression the checker converted; a read through the
/// one index over every body found the user's row there. Here `conv`'s
/// returned name, which widens `sec` into `msec`, is placed at the span
/// of a name a stdlib fn the module defines reads as a value (today
/// `__replace_all`'s `needle`, a `String`, which that read refused to
/// build): `conv` multiplies, and the stdlib fn carries no conversion at
/// all.
#[test]
fn a_users_row_never_reaches_a_stdlib_expression_at_its_span() {
    let bin = harness::unique_bin("unit_quantity_stdlib_span");
    let ir = harness::build_source_ir_text(&format!("{SECONDS}fn main() {{ println(1sec); }}\n"), &bin)
        .unwrap_or_else(|e| panic!("lowers: {e:?}"));
    let _ = std::fs::remove_file(&bin);
    let lead = |name: &str| format!("{SECONDS}fn conv({name}: Seconds) -> Span {{\n    return ");
    // The first plain name a stdlib fn the module defines reads, far
    // enough in for `conv`'s `return` to reach it.
    let stdlib = hale_types::stdlib_bodies::program().expect("the stdlib parses");
    let mut in_fn: Option<String> = None;
    let mut found: Option<(String, String, usize)> = None;
    hale_syntax::sites::for_each_named_site(stdlib, &mut |kind, span, name, _| {
        if found.is_some() {
            return;
        }
        match (kind, name) {
            (hale_syntax::sites::SiteKind::Fn, Some(f)) => {
                in_fn = function_ir(&ir, f).is_some().then(|| f.to_string());
            }
            (hale_syntax::sites::SiteKind::Use, Some(n)) => {
                // A name read as a value: not a callee, a receiver or a
                // field's.
                let source = hale_stdlib::AP_SOURCE.as_bytes();
                let read = !matches!(source.get(span.end.as_usize()), Some(b'(' | b'.' | b':'))
                    && source.get(span.start.as_usize().wrapping_sub(1)) != Some(&b'.');
                let plain = read
                    && n.len() <= 8
                    && n.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                    && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    && !["conv", "main", "sec", "msec"].contains(&n);
                if let Some(f) = in_fn.as_ref().filter(|_| plain && span.start.as_usize() >= lead(n).len()) {
                    found = Some((f.clone(), n.to_string(), span.start.as_usize()));
                }
            }
            _ => {}
        }
    });
    let (stdlib_fn, name, at) = found.expect("a name a defined stdlib fn reads");
    let pad = " ".repeat(at - lead(&name).len());
    let src = format!("{}{pad}{name};\n}}\nfn main() {{ println(conv(2sec)); }}\n", lead(&name));
    assert_eq!(&src[at..at + name.len()], name, "`conv`'s returned name stands at the stdlib's span");
    let bin = harness::unique_bin("unit_quantity_stdlib_span");
    let ir = harness::build_source_ir_text(&src, &bin).unwrap_or_else(|e| panic!("lowers: {e:?}"));
    let _ = std::fs::remove_file(&bin);
    let conv = function_ir(&ir, "conv").expect("`conv` is defined");
    assert!(conv.contains("%unit.scale = mul i64") && conv.contains(", 1000"), "the user's row: {conv}");
    let theirs = function_ir(&ir, &stdlib_fn).expect("the stdlib fn is defined");
    assert_eq!(theirs.matches("unit.").count(), 0, "`{stdlib_fn}` reads `{name}` at {at}, and no row of `conv`'s: {theirs}");
}

/// U3 polish (B): a user fn whose name starts with `__` and contains a
/// stdlib declaration's name converts what it converts. The rule this
/// replaces skipped every row inside a function so named, taking it for
/// the stdlib's.
#[test]
fn a_user_fn_named_like_the_stdlib_converts() {
    let stdlib = hale_types::stdlib_bodies::program().expect("the stdlib parses");
    let taken = hale_syntax::ast::flat_decls(&stdlib.items)
        .find_map(|item| match item {
            hale_syntax::ast::TopDecl::Fn(f) if f.name.name.starts_with("__") => Some(f.name.name.clone()),
            _ => None,
        })
        .expect("a stdlib fn");
    let name = format!("{taken}_seconds");
    let src = format!("{SECONDS}fn {name}(s: Seconds) -> Span {{\n    return s;\n}}\nfn main() {{ println({name}(2sec)); }}\n");
    let bin = harness::unique_bin("unit_quantity_stdlib_name");
    let ir = harness::build_source_ir_text(&src, &bin).unwrap_or_else(|e| panic!("lowers: {e:?}\n{src}"));
    let _ = std::fs::remove_file(&bin);
    let fun = function_ir(&ir, &name).unwrap_or_else(|| panic!("`{name}` is defined"));
    assert!(fun.contains("%unit.scale = mul i64 %s1, 1000"), "{fun}");
}

#[test]
fn a_point_across_origins_is_a_shift() {
    let fun = conv_ir("(c: Celsius) -> Kelvin", "    return Kelvin(c);\n");
    assert!(fun.contains("%unit.shift = add i64 %c1, 273150"), "{fun}");
    assert_eq!(count(&fun, "mul"), 0, "one denomination, no factor: {fun}");
}

/// A factor no `Int` holds (10^24 here) is refused where the conversion
/// is, never wrapped.
#[test]
fn a_factor_no_int_holds_is_a_located_error() {
    let src = "unit a;\nunit b = 1_000_000_000_000 a;\nunit c = 1_000_000_000_000 b;\ntype A = quantity Int in a;\n\
               fn conv(n: Int) -> A { return n * 1c; }\nfn main() { println(conv(1)); }\n";
    let bin = harness::unique_bin("unit_quantity_overflow");
    let err = build_opts::build_source(src, &bin, &build_opts::options()).expect_err("no `Int` holds the factor");
    let _ = std::fs::remove_file(&bin);
    match err {
        CodegenError::UnsupportedAt(msg, span) => {
            assert_eq!(span.slice(src), "n * 1c");
            assert_eq!(msg, "the conversion into `A` is by the factor 1000000000000000000000000, which no `Int` holds");
        }
        other => panic!("expected a located refusal, got {other}"),
    }
}
