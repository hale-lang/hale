//! pond P2 (FRICTION panic-exit-bypasses-atexit): a runtime panic must
//! run atexit-registered cleanup, so a full-screen TUI's termios/alt-
//! screen restore fires instead of stranding the terminal in raw mode.
//!
//! `lotus_view_stale_panic` (F.30b) used `_exit(1)`, which bypasses
//! atexit. This test installs an atexit hook through FFI glue (the way
//! pond/term does), triggers a stale-view panic, and asserts the hook
//! ran — i.e. the panic went through `exit()`, not `_exit()`.

use std::process::Command;

use hale_codegen::{build_executable_with_options, BuildOptions};

fn build_with_csrc(name: &str, hale_src: &str, csrc_body: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(hale_src).expect("parse");
    let mut tmpdir = std::env::temp_dir();
    tmpdir.push(format!("hale_test_panic_atexit_{}", name));
    let _ = std::fs::create_dir_all(&tmpdir);
    let csrc_path = tmpdir.join("glue.c");
    std::fs::write(&csrc_path, csrc_body).expect("write csrc");
    let bin = tmpdir.join("main");
    let options = BuildOptions {
        link_libs: Vec::new(),
        csrc_files: vec![csrc_path.clone()],
        ..Default::default()
    };
    build_executable_with_options(&program, &bin, &[], &options).expect("build");
    let _ = std::fs::remove_file(&csrc_path);
    bin
}

#[test]
fn view_stale_panic_runs_atexit_cleanup() {
    // The glue mirrors pond/term: an FFI fn that registers an atexit
    // restore. The restore writes a marker we can assert on.
    let csrc = r#"
        #include <stdlib.h>
        #include <unistd.h>
        static void p2_cleanup(void) {
            const char *m = "P2_CLEANUP_RAN\n";
            ssize_t _ = write(2, m, 15);
            (void)_;
        }
        void p2_install_cleanup(void) { atexit(p2_cleanup); }
    "#;
    let hale_src = r#"
        @ffi("c") fn p2_install_cleanup() -> ();
        fn main() {
            p2_install_cleanup();
            let b = std::bytes::BytesBuilder { initial_cap: 16 };
            b.append_str("x");
            let v = b.view();
            b.append_str("y");                  // mutate source after view()
            println(std::str::from_bytes(v));   // stale read → view_stale_panic
        }
    "#;
    let bin = build_with_csrc("view_stale", hale_src, csrc);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The panic itself fired (the violation diagnostic).
    assert!(
        stderr.contains("read after source BytesBuilder mutated"),
        "expected the stale-view violation; stderr: {:?}",
        stderr
    );
    // And the atexit cleanup ran — the whole point: the panic went
    // through exit(), not _exit(). Pre-fix this marker was absent.
    assert!(
        stderr.contains("P2_CLEANUP_RAN"),
        "atexit cleanup did NOT run — panic bypassed atexit (still _exit?); stderr: {:?}",
        stderr
    );
    // Fatal: non-zero exit.
    assert!(!out.status.success(), "stale-view panic should exit non-zero");
}
