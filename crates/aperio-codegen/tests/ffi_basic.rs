//! Stage-1 FFI vertical-slice regression test (2026-05-22).
//!
//! Declares a single `@ffi("c") fn` in an Aperio program, ships a
//! one-line C glue file with the matching symbol, builds the
//! program with `BuildOptions::csrc_files` pointing at the glue,
//! runs the binary, asserts the C function's return value reached
//! the Aperio side. Validates the full Stage-1 pipeline end-to-end:
//! parser accepts the `@ffi` annotation, typecheck validates the
//! Int/Int signature, codegen emits an LLVM `declare`, the CLI's
//! `--csrc` path (here the equivalent `BuildOptions.csrc_files`)
//! compiles the glue alongside the runtime, and the linker
//! resolves the extern symbol.
//!
//! See `notes/ffi-design.md` Stage 1 + `spec/ffi.md`.

use std::process::Command;

use aperio_codegen::{build_executable_with_options, BuildOptions};

fn build_with_csrc(
    name: &str,
    aperio_src: &str,
    csrc_body: &str,
) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(aperio_src).expect("parse");
    let mut tmpdir = std::env::temp_dir();
    tmpdir.push(format!("aperio_test_ffi_basic_{}", name));
    let _ = std::fs::create_dir_all(&tmpdir);

    let csrc_path = tmpdir.join("glue.c");
    std::fs::write(&csrc_path, csrc_body).expect("write csrc");

    let bin = tmpdir.join("main");
    let options = BuildOptions {
        link_libs: Vec::new(),
        csrc_files: vec![csrc_path.clone()],
    };
    build_executable_with_options(&program, &bin, &[], &options)
        .expect("build");
    let _ = std::fs::remove_file(&csrc_path);
    bin
}

#[test]
fn ffi_int_arg_int_return_round_trips_through_c() {
    let aperio_src = r#"
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
    let bin = build_with_csrc("int_double", aperio_src, csrc);
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

#[test]
fn ffi_void_return_with_no_args_invokes_c_side_effect() {
    // Confirms the void-return path and the no-args path. The C
    // side increments a counter; the Aperio side reads it back via
    // a separate Int-returning @ffi fn.
    let aperio_src = r#"
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
    let bin = build_with_csrc("void_bump", aperio_src, csrc);
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
fn ffi_string_arg_passes_nul_terminated_to_c() {
    // String → const char *. The C side calls strlen on it to
    // confirm the pointer is a valid NUL-terminated string.
    let aperio_src = r#"
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
    let bin = build_with_csrc("strlen", aperio_src, csrc);
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
