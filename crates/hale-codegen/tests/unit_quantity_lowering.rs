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
