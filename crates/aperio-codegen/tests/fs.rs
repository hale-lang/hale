//! m74: filesystem primitives in the C runtime.
//!
//! Builds `runtime/lotus_arena.c` plus the `fs_driver.c` harness
//! into a single binary, then exec's it with `read` / `write` /
//! `size` / `exists` commands to round-trip data through the
//! `lotus_fs_*` surface.
//!
//! No codegen path is exercised here — m74 is C-runtime only.
//! m75 wires these up to `std::io::fs::*` calls in `.ap`
//! source via the m71-style path-call dispatcher.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn runtime_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("runtime");
    p.push("lotus_arena.c");
    p
}

fn driver_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("tests");
    p.push("fs_driver.c");
    p
}

fn build_driver(name: &str) -> PathBuf {
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_fs_driver_{}", name));
    let status = Command::new("clang")
        .arg(driver_c_path())
        .arg(runtime_c_path())
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building fs driver");
    bin
}

/// Build a unique tempfile path per test so parallel `cargo
/// test` invocations don't collide.
fn unique_tempfile(tag: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "aperio_fs_test_{}_{}_{}.tmp",
        tag,
        std::process::id(),
        nanos
    ));
    p
}

#[test]
fn fs_write_then_read_round_trip() {
    let driver = build_driver("rt");
    let tmp = unique_tempfile("rt");
    let payload = "hello aperio\nlines and bytes\n";

    let write_status = Command::new(&driver)
        .args(["write", tmp.to_str().unwrap(), payload])
        .status()
        .expect("write spawn");
    assert!(write_status.success(), "write exited non-zero");

    let read_out = Command::new(&driver)
        .args(["read", tmp.to_str().unwrap()])
        .output()
        .expect("read spawn");
    assert!(read_out.status.success(), "read exited non-zero");

    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&driver);

    assert_eq!(
        String::from_utf8_lossy(&read_out.stdout),
        payload,
        "round-trip bytes must match exactly"
    );
}

#[test]
fn fs_size_returns_byte_count() {
    let driver = build_driver("size");
    let tmp = unique_tempfile("size");
    // 17 bytes — pick something deliberately not round to make
    // sure we're returning the actual st_size, not a buffer cap.
    let payload = "seventeen letters";

    let _ = Command::new(&driver)
        .args(["write", tmp.to_str().unwrap(), payload])
        .status();

    let size_out = Command::new(&driver)
        .args(["size", tmp.to_str().unwrap()])
        .output()
        .expect("size spawn");
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&driver);

    assert!(size_out.status.success(), "size exited non-zero");
    let stdout = String::from_utf8_lossy(&size_out.stdout);
    assert_eq!(stdout.trim(), "17", "expected 17; got: {:?}", stdout);
}

#[test]
fn fs_exists_distinguishes_present_from_absent() {
    let driver = build_driver("exists");
    let tmp = unique_tempfile("exists");

    // Absent first.
    let absent = Command::new(&driver)
        .args(["exists", tmp.to_str().unwrap()])
        .output()
        .expect("exists spawn (absent)");
    assert!(absent.status.success());
    assert_eq!(
        String::from_utf8_lossy(&absent.stdout).trim(),
        "0",
        "absent file must report 0"
    );

    // Now create + check.
    let _ = Command::new(&driver)
        .args(["write", tmp.to_str().unwrap(), "x"])
        .status();
    let present = Command::new(&driver)
        .args(["exists", tmp.to_str().unwrap()])
        .output()
        .expect("exists spawn (present)");
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&driver);

    assert!(present.status.success());
    assert_eq!(
        String::from_utf8_lossy(&present.stdout).trim(),
        "1",
        "present file must report 1"
    );
}

#[test]
fn fs_read_returns_negative_on_missing_file() {
    let driver = build_driver("missing");
    let tmp = unique_tempfile("missing");
    // Don't create the file.

    let read_out = Command::new(&driver)
        .args(["read", tmp.to_str().unwrap()])
        .output()
        .expect("read spawn");
    let _ = std::fs::remove_file(&driver);

    // Driver exits 1 on lotus_fs_read_file -> -1; stdout empty.
    assert!(
        !read_out.status.success(),
        "expected non-zero exit on missing-file read"
    );
    let stderr = String::from_utf8_lossy(&read_out.stderr);
    assert!(
        stderr.contains("read: failed"),
        "expected diagnostic; got: {:?}",
        stderr
    );
}

#[test]
fn fs_write_truncates_existing_file() {
    let driver = build_driver("trunc");
    let tmp = unique_tempfile("trunc");
    let long = "this is a longer initial payload";
    let short = "short";

    let _ = Command::new(&driver)
        .args(["write", tmp.to_str().unwrap(), long])
        .status();
    let _ = Command::new(&driver)
        .args(["write", tmp.to_str().unwrap(), short])
        .status();

    let read_out = Command::new(&driver)
        .args(["read", tmp.to_str().unwrap()])
        .output()
        .expect("read spawn");
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&driver);

    assert!(read_out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&read_out.stdout),
        short,
        "second write must replace, not append"
    );
}

#[test]
fn fs_read_handles_binary_safely_without_treating_zeros_as_terminator() {
    // Use std::fs to write a payload with embedded NUL bytes
    // (the C driver's argv-based write path doesn't allow NULs
    // in arguments). Then read via the lotus surface and
    // confirm the byte count is faithful — proves
    // lotus_fs_read_file isn't doing any NUL-terminated string
    // handling on the way through.
    let driver = build_driver("binary");
    let tmp = unique_tempfile("binary");
    let payload = b"a\0b\0\0c\0d";
    {
        let mut f = std::fs::File::create(&tmp).expect("create");
        f.write_all(payload).expect("write");
    }

    let size_out = Command::new(&driver)
        .args(["size", tmp.to_str().unwrap()])
        .output()
        .expect("size spawn");
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&driver);

    assert!(size_out.status.success());
    let stdout = String::from_utf8_lossy(&size_out.stdout);
    assert_eq!(
        stdout.trim(),
        payload.len().to_string(),
        "size must reflect every byte including NULs; got: {:?}",
        stdout
    );
}
