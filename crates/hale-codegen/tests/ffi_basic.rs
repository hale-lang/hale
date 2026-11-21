//! Stage-1 FFI vertical-slice regression test (2026-05-22).
//!
//! Declares a single `@ffi("c") fn` in an Hale program, ships a
//! one-line C glue file with the matching symbol, builds the
//! program with `BuildOptions::csrc_files` pointing at the glue,
//! runs the binary, asserts the C function's return value reached
//! the Hale side. Validates the full Stage-1 pipeline end-to-end:
//! parser accepts the `@ffi` annotation, typecheck validates the
//! Int/Int signature, codegen emits an LLVM `declare`, the CLI's
//! `--csrc` path (here the equivalent `BuildOptions.csrc_files`)
//! compiles the glue alongside the runtime, and the linker
//! resolves the extern symbol.
//!
//! See `notes/ffi-design.md` Stage 1 + `spec/ffi.md`.

use std::process::Command;

use hale_codegen::BuildOptions;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_with_csrc(
    name: &str,
    hale_src: &str,
    csrc_body: &str,
) -> std::path::PathBuf {
    let tmpdir = harness::unique_bin(&format!("hale_test_ffi_basic_{}", name));
    let _ = std::fs::create_dir_all(&tmpdir);

    let csrc_path = tmpdir.join("glue.c");
    std::fs::write(&csrc_path, csrc_body).expect("write csrc");

    let bin = tmpdir.join("main");
    let options = BuildOptions {
        link_libs: Vec::new(),
        csrc_files: vec![csrc_path.clone()],
        ..build_opts::options()
    };
    build_opts::build_source(hale_src, &bin, &options)
        .expect("build");
    let _ = std::fs::remove_file(&csrc_path);
    bin
}

#[test]
fn ffi_int_arg_int_return_round_trips_through_c() {
    let hale_src = r#"
        @ffi("c") fn ffi_test_double(x: Int) -> Int;
        fn main() {
            let n = ffi_test_double(21);
            println("result=", n);
        }
    "#;
    let csrc = r#"
        #include <stdint.h>
        int64_t ffi_test_double(int64_t x) { return x * 2; }
    "#;
    let bin = build_with_csrc("int_double", hale_src, csrc);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("result=42"),
        "expected `result=42` in stdout; got: {:?}",
        stdout
    );
}

/// GH #2132: every fixed-width integer crosses as the C integer of its
/// width, each end of its range, in both directions: the glue answers
/// the argument it got and the argument one step toward zero, so a value
/// the caller did not extend (or extended the wrong way) shows as a wrong
/// number.
#[test]
fn ffi_fixed_width_integers_cross_as_their_c_types() {
    let hale_src = r#"
        @ffi("c") fn ffi_u8(x: U8) -> U8;
        @ffi("c") fn ffi_i8(x: I8) -> I8;
        @ffi("c") fn ffi_u16(x: U16) -> U16;
        @ffi("c") fn ffi_i16(x: I16) -> I16;
        @ffi("c") fn ffi_u32(x: U32) -> U32;
        @ffi("c") fn ffi_i32(x: I32) -> I32;
        @ffi("c") fn ffi_wide(a: U8, b: I8, c: U16, d: I16) -> Int;
        fn main() {
            println("u8=", ffi_u8(255), " ", ffi_u8(0));
            println("i8=", ffi_i8(-128), " ", ffi_i8(127));
            println("u16=", ffi_u16(65535), " i16=", ffi_i16(-32768));
            println("u32=", ffi_u32(4294967295), " i32=", ffi_i32(-2147483648));
            println("wide=", ffi_wide(255, -1, 65535, -1));
        }
    "#;
    let csrc = r#"
        #include <stdint.h>
        uint8_t ffi_u8(uint8_t x) { return x == 0 ? 0 : x - 1; }
        int8_t ffi_i8(int8_t x) { return x < 0 ? x + 1 : x - 1; }
        uint16_t ffi_u16(uint16_t x) { return x - 1; }
        int16_t ffi_i16(int16_t x) { return x + 1; }
        uint32_t ffi_u32(uint32_t x) { return x - 1; }
        int32_t ffi_i32(int32_t x) { return x + 1; }
        int64_t ffi_wide(uint8_t a, int8_t b, uint16_t c, int16_t d) {
            return (int64_t)a + (int64_t)b + (int64_t)c + (int64_t)d;
        }
    "#;
    let bin = build_with_csrc("fixed_width", hale_src, csrc);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    for want in ["u8=254 0", "i8=-127 126", "u16=65534 i16=-32767", "u32=4294967294 i32=-2147483647", "wide=65788"] {
        assert!(stdout.contains(want), "expected `{want}` in stdout; got: {stdout:?}");
    }
}

#[test]
fn ffi_void_return_with_no_args_invokes_c_side_effect() {
    // Confirms the void-return path and the no-args path. The C
    // side increments a counter; the Hale side reads it back via
    // a separate Int-returning @ffi fn.
    let hale_src = r#"
        @ffi("c") fn ffi_test_bump() -> ();
        @ffi("c") fn ffi_test_count() -> Int;
        fn main() {
            ffi_test_bump();
            ffi_test_bump();
            ffi_test_bump();
            let n = ffi_test_count();
            println("count=", n);
        }
    "#;
    let csrc = r#"
        #include <stdint.h>
        static int64_t counter = 0;
        void ffi_test_bump(void) { counter += 1; }
        int64_t ffi_test_count(void) { return counter; }
    "#;
    let bin = build_with_csrc("void_bump", hale_src, csrc);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("count=3"),
        "expected `count=3` in stdout; got: {:?}",
        stdout
    );
}

#[test]
fn ffi_user_type_struct_passes_by_pointer_and_returns_via_sret() {
    // User-type struct marshalling: Hale passes the struct as a
    // pointer (C glue dereferences); the C glue fills a caller-
    // allocated return slot for struct returns. See spec/ffi.md.
    let hale_src = r#"
        type Vec3i { x: Int = 0; y: Int = 0; z: Int = 0; }
        @ffi("c") fn vec3i_sum(v: Vec3i) -> Int;
        @ffi("c") fn vec3i_scale(v: Vec3i, k: Int) -> Vec3i;
        fn main() {
            let v = Vec3i { x: 1, y: 2, z: 3 };
            let s = vec3i_sum(v);
            println("sum=", s);
            let scaled = vec3i_scale(v, 10);
            println("scaled=(", scaled.x, ",", scaled.y, ",", scaled.z, ")");
        }
    "#;
    let csrc = r#"
        #include <stdint.h>
        typedef struct { int64_t x; int64_t y; int64_t z; } Vec3i;
        int64_t vec3i_sum(const Vec3i *v) {
            return v->x + v->y + v->z;
        }
        void vec3i_scale(Vec3i *out, const Vec3i *v, int64_t k) {
            out->x = v->x * k;
            out->y = v->y * k;
            out->z = v->z * k;
        }
    "#;
    let bin = build_with_csrc("struct_roundtrip", hale_src, csrc);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("sum=6"),
        "expected `sum=6` in stdout; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("scaled=(10,20,30)"),
        "expected `scaled=(10,20,30)` in stdout; got: {:?}",
        stdout
    );
}

#[test]
fn ffi_string_arg_passes_nul_terminated_to_c() {
    // String → const char *. The C side calls strlen on it to
    // confirm the pointer is a valid NUL-terminated string.
    let hale_src = r#"
        @ffi("c") fn ffi_test_strlen(s: String) -> Int;
        fn main() {
            let n = ffi_test_strlen("hello");
            println("len=", n);
        }
    "#;
    let csrc = r#"
        #include <stdint.h>
        #include <string.h>
        int64_t ffi_test_strlen(const char *s) {
            return (int64_t)strlen(s);
        }
    "#;
    let bin = build_with_csrc("strlen", hale_src, csrc);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("len=5"),
        "expected `len=5` in stdout; got: {:?}",
        stdout
    );
}
