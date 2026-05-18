//! std::io::file::File — held-open file I/O via let-bound locus.
//!
//! Coverage:
//!   - open + write_line + dissolve at fn exit closes the fd.
//!   - open ("r") + at_eof + read_line loop walks the file body.
//!   - open ("a") append mode preserves prior content.
//!   - bad-mode kwarg surfaces a "kind=invalid" IoError.
//!   - missing-file open surfaces a "kind=not_found" IoError.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn unique_path(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "aperio_file_locus_{}_{}_{}",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(name: &str, source: &str) -> (String, String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_file_locus_bin_{}_{}_{}",
        name,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status,
    )
}

#[test]
fn write_then_read_round_trip() {
    let scratch = unique_path("rt");
    let scratch_str = scratch.display().to_string();
    let src = format!(
        r#"
        fn write_log(path: String) -> () fallible(IoError) {{
            let f = std::io::file::open(path, "w") or raise;
            std::io::file::write_line(f, "alpha") or raise;
            std::io::file::write_line(f, "beta") or raise;
            std::io::file::write_line(f, "gamma") or raise;
            // f dissolves here, closing the fd before the
            // reader-side fn opens the same path.
        }}

        fn read_log(path: String) -> () fallible(IoError) {{
            let f = std::io::file::open(path, "r") or raise;
            while !std::io::file::at_eof(f) {{
                let line = std::io::file::read_line(f);
                print("LINE:");
                print(line);
            }}
        }}

        fn main() {{
            write_log("{path}") or raise;
            read_log("{path}") or raise;
        }}
    "#,
        path = scratch_str,
    );
    let (stdout, stderr, status) = build_and_run("rt", &src);
    let _ = std::fs::remove_file(&scratch);
    assert!(status.success(), "exit: {:?}\nstderr: {}", status, stderr);
    // Each line includes its trailing \n, so the LINE:... markers
    // join up as "LINE:alpha\nLINE:beta\nLINE:gamma\n".
    assert!(
        stdout.contains("LINE:alpha\n"),
        "missing alpha line; stdout: {:?}",
        stdout,
    );
    assert!(
        stdout.contains("LINE:beta\n"),
        "missing beta line; stdout: {:?}",
        stdout,
    );
    assert!(
        stdout.contains("LINE:gamma\n"),
        "missing gamma line; stdout: {:?}",
        stdout,
    );
}

#[test]
fn append_mode_preserves_prior_content() {
    let scratch = unique_path("ap");
    let scratch_str = scratch.display().to_string();
    let src = format!(
        r#"
        fn main() {{
            // Write initial content with "w" (truncate).
            let f1 = std::io::file::open("{path}", "w") or raise;
            std::io::file::write_line(f1, "first") or raise;
            // f1 dissolves at scope exit; we let it run by
            // ending the outer fn. To force dissolve before
            // re-open, wrap in a helper.
            __sync_close_first("{path}") or raise;
        }}

        fn __sync_close_first(path: String) -> () fallible(IoError) {{
            // Re-open with append, then dissolve.
            let f2 = std::io::file::open(path, "a") or raise;
            std::io::file::write_line(f2, "second") or raise;
            std::io::file::write_line(f2, "third") or raise;
            // f2 dissolves on return.
            __sync_read_back(path) or raise;
        }}

        fn __sync_read_back(path: String) -> () fallible(IoError) {{
            let r = std::io::file::open(path, "r") or raise;
            while !std::io::file::at_eof(r) {{
                let line = std::io::file::read_line(r);
                print("L:");
                print(line);
            }}
        }}
    "#,
        path = scratch_str,
    );
    let (stdout, stderr, status) = build_and_run("ap", &src);
    let _ = std::fs::remove_file(&scratch);
    assert!(status.success(), "exit: {:?}\nstderr: {}", status, stderr);
    assert!(
        stdout.contains("L:first\n"),
        "missing first line (append should not truncate); stdout: {:?}",
        stdout,
    );
    assert!(
        stdout.contains("L:second\n"),
        "missing second line; stdout: {:?}",
        stdout,
    );
    assert!(
        stdout.contains("L:third\n"),
        "missing third line; stdout: {:?}",
        stdout,
    );
}

#[test]
fn open_missing_file_in_read_mode_surfaces_not_found_error() {
    let scratch = unique_path("missing");
    let scratch_str = scratch.display().to_string();
    // try_open returns an Int (1 on success, error otherwise);
    // the `or` substitute prints the kind and returns 0.
    let src = format!(
        r#"
        fn try_open(path: String) -> Int fallible(IoError) {{
            let f = std::io::file::open(path, "r") or raise;
            return 1;
        }}

        fn main() {{
            let r = try_open("{path}") or {{
                println("kind=", err.kind);
                0
            }};
            println("r=", r);
        }}
    "#,
        path = scratch_str,
    );
    let (stdout, stderr, status) = build_and_run("missing", &src);
    assert!(status.success(), "exit: {:?}\nstderr: {}", status, stderr);
    assert!(
        stdout.contains("kind=not_found"),
        "expected kind=not_found for missing file open; got: {:?}",
        stdout,
    );
    assert!(
        stdout.contains("r=0"),
        "expected r=0 substitute; got: {:?}",
        stdout,
    );
}

#[test]
fn open_with_invalid_mode_surfaces_invalid_kind_error() {
    let src = r#"
        fn try_open() -> Int fallible(IoError) {
            let f = std::io::file::open("/tmp/whatever", "q") or raise;
            return 1;
        }

        fn main() {
            let r = try_open() or {
                println("kind=", err.kind);
                0
            };
            println("r=", r);
        }
    "#;
    let (stdout, stderr, status) = build_and_run("badmode", src);
    assert!(status.success(), "exit: {:?}\nstderr: {}", status, stderr);
    assert!(
        stdout.contains("kind=invalid"),
        "expected kind=invalid for bad mode; got: {:?}",
        stdout,
    );
}
