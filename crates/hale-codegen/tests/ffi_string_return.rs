//! Does `@ffi("c")` support a `String` *return* (C → Hale)?
//!
//! Hale `String` is a NUL-terminated `char*` with no length header
//! (lotus_arena.c: "Strings in the codegen are NUL-terminated byte
//! arrays"), and `@ffi` marshals `String` as `const char *`. So a C
//! function returning a NUL-terminated `const char *` should hand
//! back a directly-usable Hale `String`. This is the one `@ffi`
//! capability pond/sqlite needs that wasn't yet covered by a test
//! (its `column_text(...) -> String`); if it works, a SQLite
//! binding is a pure-library `@ffi` job, no stdlib primitive.
//!
//! Exercises: print the returned string, concatenate it (which
//! `strlen`s it via `lotus_str_concat`, proving NUL-termination +
//! usability), and round-trip a String arg back out.

use std::process::Command;

use hale_codegen::{build_executable_with_options, BuildOptions};

fn build_with_csrc(name: &str, hale_src: &str, csrc_body: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(hale_src).expect("parse");
    let mut tmpdir = std::env::temp_dir();
    tmpdir.push(format!("hale_test_ffi_strret_{}", name));
    let _ = std::fs::create_dir_all(&tmpdir);
    let csrc_path = tmpdir.join("glue.c");
    std::fs::write(&csrc_path, csrc_body).expect("write csrc");
    let bin = tmpdir.join("main");
    let options = BuildOptions {
        link_libs: Vec::new(),
        csrc_files: vec![csrc_path.clone()],
    };
    build_executable_with_options(&program, &bin, &[], &options).expect("build");
    let _ = std::fs::remove_file(&csrc_path);
    bin
}

#[test]
fn ffi_string_return_is_usable_hale_string() {
    let hale_src = r#"
        @ffi("c") fn ffi_greeting() -> String;
        @ffi("c") fn ffi_echo(s: String) -> String;
        fn main() {
            let g = ffi_greeting();
            println("g=", g);
            // Concatenation strlen's the C-returned string — proves
            // it is a usable NUL-terminated Hale String, not just a
            // printable opaque pointer.
            let combined = g + " (wrapped)";
            println("combined=", combined);
            // String arg → C → String return round-trip.
            let e = ffi_echo("ping");
            println("echo=", e);
        }
    "#;
    let csrc = r#"
        #include <stdio.h>
        #include <string.h>
        #include <stdlib.h>
        // A static NUL-terminated string: valid for the program's
        // lifetime, so the Hale side can hold/clone it freely.
        const char *ffi_greeting(void) { return "hello from C"; }
        // Echo: return a heap copy prefixed with "c:". (A real glue
        // for sqlite's column_text would similarly hand back a
        // pointer valid until the Hale wrapper clones it.)
        const char *ffi_echo(const char *s) {
            static char buf[256];
            snprintf(buf, sizeof(buf), "c:%s", s ? s : "");
            return buf;
        }
    "#;
    let bin = build_with_csrc("strret", hale_src, csrc);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit (@ffi String return regressed): {:?}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("g=hello from C"), "got: {:?}", stdout);
    assert!(
        stdout.contains("combined=hello from C (wrapped)"),
        "concat over a C-returned String failed: {:?}",
        stdout
    );
    assert!(stdout.contains("echo=c:ping"), "got: {:?}", stdout);
}
