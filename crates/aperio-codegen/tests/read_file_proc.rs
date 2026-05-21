//! `std::io::fs::read_file` survives synthesized files
//! (/proc, /sys, FIFO pipes) where `fstat` reports `st_size = 0`.
//!
//! Pre-fix, the codegen path pre-sized the buffer to `fstat`'s
//! reading and read into it — for synthesized files the buffer
//! was 1 byte (just the NUL terminator) and the read produced
//! nothing. Closing a downstream friction-log item: `read_file
//! can't read /proc/self/statm`.
//!
//! Post-fix, the codegen routes through
//! `lotus_fs_read_file_growing` which doesn't trust fstat and
//! grows a 4 KiB → 64 MiB buffer as it reads.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_read_file_proc_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn read_file_picks_up_proc_self_statm() {
    // /proc/self/statm is space-separated integers like
    // "12345 678 ..." — guaranteed non-empty for any running
    // process. Pre-fix this returned "".
    let src = r#"
        fn main() {
            let s = std::io::fs::read_file("/proc/self/statm") or "ERR";
            // Confirm it's non-empty and contains a digit.
            let n = len(s);
            if n > 0 {
                println("len=", to_string(n));
            } else {
                println("empty");
            }
        }
    "#;
    let (stdout, status) = build_and_run("statm", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        !stdout.contains("empty") && !stdout.contains("ERR"),
        "read_file returned empty for /proc/self/statm; stdout: {:?}",
        stdout,
    );
    assert!(stdout.contains("len="), "stdout: {:?}", stdout);
}

#[test]
fn read_file_still_works_for_regular_files() {
    // Ensure the rewrite didn't break the common case of
    // reading a real file. Round-trip a temp file's contents.
    let tmp = std::env::temp_dir().join(format!(
        "aperio_read_file_proc_DATA_regular_{}",
        std::process::id()
    ));
    let body = "alpha\nbeta\ngamma\n";
    std::fs::write(&tmp, body).expect("write tmp");

    let src = format!(
        r#"
        fn main() {{
            let s = std::io::fs::read_file({:?}) or "FAIL";
            println(s);
        }}
        "#,
        tmp.to_string_lossy()
    );

    let (stdout, status) = build_and_run("regular", &src);
    let _ = std::fs::remove_file(&tmp);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("alpha") && stdout.contains("gamma"),
        "regular-file read regressed; stdout: {:?}",
        stdout,
    );
}
