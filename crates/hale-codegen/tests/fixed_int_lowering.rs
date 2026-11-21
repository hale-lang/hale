//! GH #2132 (F.64): the fixed-width integers `I32`, `U32`, `I16`, `U16`,
//! `I8` and `U8` are their declared width, and their operators are
//! checked at it. These pin the emitted shape as the IR text before the
//! optimizer runs: a record's `bounded[U8; 1024]` field is 1024 bytes of
//! `i8` (against 1024 `i64`s for an `Int`'s), a locus field and a local
//! are the narrow integer, `+` is the overflow intrinsic of the width (a
//! signed one's `s`, an unsigned one's `u`), an unsigned comparison and
//! division are unsigned, a signed `>>` is arithmetic, `wrapping_*` is
//! the plain instruction a vectorizer widens, a widening is a `sext` /
//! `zext`, and an `@ffi("c")` 8- or 16-bit parameter carries the
//! extension a C prototype's does.

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The module's IR, before optimization.
fn module_ir(src: &str) -> String {
    let bin = harness::unique_bin("fixed_int_lowering");
    let ir = harness::build_source_ir_text(src, &bin).unwrap_or_else(|e| panic!("lowers: {e:?}\n{src}"));
    let _ = std::fs::remove_file(&bin);
    ir
}

/// The IR of the fn `name` in `ir`.
fn fn_ir(ir: &str, name: &str) -> String {
    let start = ir
        .lines()
        .position(|l| l.starts_with("define") && l.contains(&format!("@{name}(")))
        .unwrap_or_else(|| panic!("no `{name}` in the module:\n{ir}"));
    ir.lines().skip(start).take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
}

/// The body of the named struct type `name` (`{ … }`).
fn struct_ty<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("%type.{name} = type ");
    ir.lines()
        .find_map(|l| l.strip_prefix(&head))
        .unwrap_or_else(|| panic!("no struct type `{name}` in the module:\n{ir}"))
}

#[test]
fn a_bounded_u8_field_is_its_bytes() {
    let ir = module_ir(
        "type Frame {\n    tag: U8;\n    buf: bounded[U8; 1024];\n}\n\
         type Wide {\n    tag: Int;\n    buf: bounded[Int; 1024];\n}\n\
         type Mixed {\n    a: U8;\n    b: I16;\n    c: U32;\n    d: I8;\n}\n\
         fn main() {\n    let f = Frame { tag: 1 };\n    push(f.buf, 7) or discard;\n    \
         let w = Wide { tag: 1 };\n    push(w.buf, 7) or discard;\n    let m = Mixed { a: 1, b: 2, c: 3, d: 4 };\n    \
         println(count(f.buf), count(w.buf), m.c);\n}\n",
    );
    // the buffer is its count and 1024 one-byte elements: 1024 bytes of
    // elements where an `Int`'s are 8192
    assert_eq!(struct_ty(&ir, "Frame"), "{ i8, { i64, [1024 x i8] } }");
    assert_eq!(struct_ty(&ir, "Wide"), "{ i64, { i64, [1024 x i64] } }");
    // each field its own width
    assert_eq!(struct_ty(&ir, "Mixed"), "{ i8, i16, i32, i8 }");
}

#[test]
fn the_operators_are_checked_at_the_width() {
    let ir = module_ir(
        "fn add8(a: U8, b: U8) -> U8 {\n    return a + b;\n}\n\
         fn sub16(a: I16, b: I16) -> I16 {\n    return a - b;\n}\n\
         fn lt32(a: U32, b: U32) -> Bool {\n    return a < b;\n}\n\
         fn div32(a: U32, b: U32) -> U32 {\n    return a / b;\n}\n\
         fn shr8(a: I8) -> I8 {\n    return a >> 2;\n}\n\
         fn wrap32(a: U32, b: U32) -> U32 {\n    return U32::wrapping_add(a, U32::wrapping_mul(b, 3));\n}\n\
         fn widen(a: U8, b: I16) -> Int {\n    let x: Int = a;\n    let y: Int = b;\n    return x + y;\n}\n\
         fn main() {\n    println(add8(1, 2), sub16(3, 4), lt32(5, 6), div32(7, 8), shr8(-9), wrap32(1, 2), widen(3, -4));\n}\n",
    );
    let add = fn_ir(&ir, "add8");
    assert!(add.contains("@llvm.uadd.with.overflow.i8(i8"), "{add}");
    assert!(add.contains("@lotus_fixed_violation"), "{add}");
    let sub = fn_ir(&ir, "sub16");
    assert!(sub.contains("@llvm.ssub.with.overflow.i16(i16"), "{sub}");
    let lt = fn_ir(&ir, "lt32");
    assert!(lt.contains("icmp ult i32"), "{lt}");
    let div = fn_ir(&ir, "div32");
    assert!(div.contains("udiv i32"), "{div}");
    let shr = fn_ir(&ir, "shr8");
    assert!(shr.contains("ashr i8"), "{shr}");
    // a kernel's wrapping arithmetic is the plain instruction at the width
    let wrap = fn_ir(&ir, "wrap32");
    assert!(wrap.contains("add i32") && wrap.contains("mul i32"), "{wrap}");
    assert!(!wrap.contains("with.overflow"), "{wrap}");
    let widen = fn_ir(&ir, "widen");
    assert!(widen.contains("zext i8") && widen.contains("sext i16"), "{widen}");
}

#[test]
fn a_c_parameter_carries_its_extension() {
    let ir = module_ir(
        "@ffi(\"c\") fn takes(a: U8, b: I16, c: U32, d: I8) -> U16;\n\
         fn main() {\n    println(takes(1, 2, 3, 4));\n}\n",
    );
    let decl = ir.lines().find(|l| l.starts_with("declare") && l.contains("@takes(")).expect("declared");
    assert!(decl.starts_with("declare zeroext i16 @takes(i8 zeroext, i16 signext, i32, i8 signext)"), "{decl}");
}
