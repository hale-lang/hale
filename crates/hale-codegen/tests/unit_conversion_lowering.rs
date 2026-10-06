//! GH #1076, step U2: a conversion is lowered from its row.
//!
//! The checker records every conversion between an identity or a range
//! and its family in the typed bodies' `conversions` column, and
//! `lower_conversion` reads the row and decides nothing. These pin the
//! emitted shape per policy, as the IR text of one function before the
//! optimizer runs: a total conversion and a widening emit nothing (no
//! call, no compare); a narrowing emits its two comparisons and, from the
//! row's policy, two selects (`clamp`), a remainder corrected for a
//! negative one (`wrap`), or the checked value the `or`'s join takes
//! (a substitute's phi, a handler's call, a raise's store into the
//! enclosing error). A view without the row lowers the cast as the
//! ordinary call it then is.

use hale_codegen::{build_resolved, CodegenError};
use hale_frontend::snapshot::{Config, Snapshot, Target};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const DECLS: &str = "type OrderId = distinct Int;\ntype Session = distinct Int { range: 0..64; }\n\
                     type Byte = Int { range: 0..256; }\nfn to_zero(e: RangeError) -> Session { return 0; }\n";

/// The IR of `conv`, a fn whose body is `body`, before optimization.
fn conv_ir(signature: &str, body: &str) -> String {
    let src = format!("{DECLS}fn conv{signature} {{\n{body}}}\nfn main() {{ println(1); }}\n");
    let bin = harness::unique_bin("unit_conversion_lowering");
    let ir = harness::build_source_ir_text(&src, &bin).unwrap_or_else(|e| panic!("lowers: {e:?}\n{src}"));
    let _ = std::fs::remove_file(&bin);
    let start = ir
        .lines()
        .position(|l| l.starts_with("define") && l.contains("conv("))
        .unwrap_or_else(|| panic!("no `conv` in the module:\n{}", defines(&ir)));
    ir.lines().skip(start).take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
}

fn defines(ir: &str) -> String {
    ir.lines().filter(|l| l.starts_with("define")).collect::<Vec<_>>().join("\n")
}

/// How many instructions of `fun` carry `op`.
fn count(fun: &str, op: &str) -> usize {
    fun.lines().filter(|l| l.contains(op)).count()
}

/// The calls of `fun` beside the frame's own (its arena's, `@lotus_…`).
fn program_calls(fun: &str) -> Vec<&str> {
    fun.lines().filter(|l| l.contains(" call ") && !l.contains("@lotus_")).collect()
}

#[test]
fn a_total_conversion_and_a_widening_emit_nothing() {
    let fun = conv_ir("(n: Int) -> Int", "    let o = OrderId(n);\n    let b: Byte = 7;\n    return Int(o) + Int(b);\n");
    assert_eq!(count(&fun, "icmp"), 0, "{fun}");
    assert_eq!(program_calls(&fun), Vec::<&str>::new(), "{fun}");
    assert_eq!(count(&fun, "narrow."), 0, "{fun}");
    // The value is stored and read as the `Int` it is.
    assert!(fun.contains("store i64 %n1, ptr %o"), "{fun}");
}

#[test]
fn a_substituted_narrowing_is_two_compares_and_the_join() {
    let fun = conv_ir("(n: Int) -> Session", "    let s = Session(n) or 5;\n    return s;\n");
    // `Session`'s range is `0..64`: below 0, above 63.
    assert!(fun.contains("%narrow.below = icmp slt i64 %n1, 0"), "{fun}");
    assert!(fun.contains("%narrow.above = icmp sgt i64 %n1, 63"), "{fun}");
    assert!(fun.contains("%narrow.outside = or i1 %narrow.below, %narrow.above"), "{fun}");
    assert!(fun.contains("br i1 %narrow.outside, label %narrow.err, label %narrow.ok"), "{fun}");
    assert!(fun.contains("RangeError.alloc"), "the error path builds the RangeError: {fun}");
    assert!(fun.contains("%or.result = phi i64"), "the substitute joins: {fun}");
    assert_eq!(count(&fun, "icmp"), 2, "{fun}");
}

#[test]
fn a_clamp_is_two_selects() {
    let fun = conv_ir("(n: Int) -> Int", "    let b = Byte(n) or clamp;\n    return b;\n");
    assert!(fun.contains("%narrow.below = icmp slt i64"), "{fun}");
    assert!(fun.contains("%narrow.above = icmp sgt i64"), "{fun}");
    assert!(fun.contains("%narrow.clamp.lo = select i1 %narrow.below, i64 0,"), "{fun}");
    assert!(fun.contains("%narrow.clamp = select i1 %narrow.above, i64 255,"), "{fun}");
    assert_eq!(count(&fun, " select "), 2, "{fun}");
    assert_eq!(count(&fun, "narrow.err"), 0, "a clamp cannot fail: {fun}");
    assert_eq!(count(&fun, "br i1"), 0, "a clamp does not branch: {fun}");
}

#[test]
fn a_wrap_is_a_remainder_corrected_for_negatives() {
    let fun = conv_ir("(n: Int) -> Int", "    let b = Byte(n) or wrap;\n    return b;\n");
    assert!(fun.contains("%narrow.wrap.off = sub i64"), "{fun}");
    assert!(fun.contains("%narrow.wrap.rem = srem i64 %narrow.wrap.off, 256"), "{fun}");
    assert!(fun.contains("%narrow.wrap.neg = icmp slt i64 %narrow.wrap.rem, 0"), "{fun}");
    assert!(fun.contains("%narrow.wrap.lift = add i64 %narrow.wrap.rem, 256"), "{fun}");
    assert!(fun.contains("%narrow.wrap.mod = select i1 %narrow.wrap.neg"), "{fun}");
    assert_eq!(count(&fun, "narrow.err"), 0, "a wrap cannot fail: {fun}");
}

#[test]
fn a_handler_is_called_on_the_error_path() {
    let fun = conv_ir("(n: Int) -> Session", "    let s = Session(n) or to_zero(err);\n    return s;\n");
    assert!(fun.contains("br i1 %narrow.outside, label %narrow.err, label %narrow.ok"), "{fun}");
    let calls = program_calls(&fun);
    assert!(calls.len() == 1 && calls[0].contains("@to_zero("), "one call, the handler's: {fun}");
    assert!(fun.contains("%or.result = phi i64 [ %or.ok.val, %or.ok ], [ %to_zero.call, %or.err ]"), "{fun}");
}

#[test]
fn a_raise_takes_the_enclosing_error_path() {
    let fun = conv_ir("(n: Int) -> Session fallible(RangeError)", "    let s = Session(n) or raise;\n    return s;\n");
    assert!(fun.contains("br i1 %narrow.outside, label %narrow.err, label %narrow.ok"), "{fun}");
    assert!(fun.contains("or.raise.payload.load"), "the RangeError is the fn's error: {fun}");
    assert_eq!(count(&fun, "lotus_root_panic"), 0, "{fun}");
}

/// A call is a conversion when its row says so, and no row is the total
/// answer: the call is no conversion, whatever its callee is named. A
/// view without the typed bodies' `conversions` column lowers each cast
/// as the ordinary call it then is, refused as any call to a name no fn
/// declares (a narrowing under its `or`, a total conversion alone),
/// never as a missing row and never by the name's being a type.
#[test]
fn a_cast_without_its_row_is_an_ordinary_call() {
    let narrowing = format!("{DECLS}fn main() {{\n    let n = 70;\n    let s = Session(n) or 0;\n    println(Int(s));\n}}\n");
    let total = format!("{DECLS}fn main() {{\n    let n = 70;\n    let o = OrderId(n);\n    println(Int(o));\n}}\n");
    for (tag, src, refusal) in [
        ("narrowing", &narrowing, "`or` over call to unknown fn `Session`"),
        ("total", &total, "call to `OrderId`: no free fn / generic fn / fn-pointer binding with that name is in scope"),
    ] {
        let program = hale_syntax::parse_source(src).expect("parses");
        let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
            panic!("the program does not load");
        };
        let whole = snap.demand_lowering().unwrap_or_else(|b| panic!("lowering blocked: {:?}", b.refused)).clone();
        let lower = |what: &str, view: &hale_types::resolved::LoweringView| {
            let bin = harness::unique_bin(&format!("unit_conversion_row_{tag}_{what}"));
            let built = build_resolved(view, &bin, &build_opts::options());
            let _ = std::fs::remove_file(&bin);
            built
        };
        lower("control", &whole).unwrap_or_else(|e| panic!("the control lowers: {e}"));
        let mut cut = whole;
        cut.typed = hale_types::typed_bodies::TypedBodies::default();
        match lower("cut", &cut) {
            Err(CodegenError::Unsupported(msg)) => assert_eq!(msg, refusal, "{tag}"),
            other => panic!("{tag}: expected the strict callee's refusal, got {other:?}"),
        }
    }
}
