//! GH #254 — `std::compress` (gzip/zstd one-shot) + `std::tar`
//! (ustar) + the `std::io::fs::write_bytes` companion.
//!
//! Three oracles:
//!   1. In-Hale round-trips with magic-byte checks and
//!      corrupt-input kind assertions — the compiled program is
//!      its own test, exit code 0 = pass.
//!   2. System-tool cross-validation: the Hale-built `.tar.gz`
//!      on disk must be accepted by the host `tar -tzf` (both CI
//!      images ship tar), proving wire-format compatibility, not
//!      just self-consistency.
//!   3. zstd rides dlopen — on a machine without libzstd the
//!      program prints `zstd-unavailable` (kind "not_found") and
//!      the test treats it as a documented skip, not a failure.

use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_compress_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn gzip_zstd_roundtrip_and_corrupt_input() {
    let src = r#"
        fn on_err(e: IoError) -> Bytes {
            println("FAIL: ", e.kind, " via ", e.path);
            std::process::exit(1);
            return std::bytes::from_string("");
        }
        fn zstd_err(e: IoError) -> Bytes {
            if e.kind == "not_found" {
                println("zstd-unavailable");
                std::process::exit(0);
            }
            return on_err(e);
        }
        fn expect_invalid(e: IoError) -> Bytes {
            if e.kind != "invalid" {
                println("wrong kind: ", e.kind);
                std::process::exit(1);
            }
            return std::bytes::from_string("");
        }
        fn main() {
            let original = std::bytes::from_string("the quick brown fox jumps over the lazy dog, twice: the quick brown fox jumps over the lazy dog");
            let gz = std::compress::gzip(original) or on_err(err);
            if std::bytes::at(gz, 0) != 31 { std::process::exit(1); }
            if std::bytes::at(gz, 1) != 139 { std::process::exit(1); }
            let back = std::compress::gunzip(gz) or on_err(err);
            if len(back) != len(original) { std::process::exit(1); }
            let junk = std::bytes::from_string("definitely not a gzip stream");
            let bad = std::compress::gunzip(junk) or expect_invalid(err);
            let z = std::compress::zstd(original) or zstd_err(err);
            if std::bytes::at(z, 0) != 40 { std::process::exit(1); }
            let zback = std::compress::unzstd(z) or zstd_err(err);
            if len(zback) != len(original) { std::process::exit(1); }
            println("roundtrips-ok");
        }
    "#;
    let bin = build("roundtrip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status,
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("roundtrips-ok") || stdout.contains("zstd-unavailable"),
        "got: {}",
        stdout
    );
}

#[test]
fn tar_archive_readable_by_system_tar() {
    let tgz_path = std::env::temp_dir().join(format!(
        "hale_compress_systar_{}.tar.gz",
        std::process::id()
    ));
    let src = format!(
        r#"
        fn on_err(e: IoError) -> Bytes {{
            println("FAIL: ", e.kind, " via ", e.path);
            std::process::exit(1);
            return std::bytes::from_string("");
        }}
        fn on_err_unit(e: IoError) {{
            println("FAIL: ", e.kind, " via ", e.path);
            std::process::exit(1);
        }}
        fn on_err_int(e: IoError) -> Int {{
            println("FAIL: ", e.kind, " via ", e.path);
            std::process::exit(1);
            return -1;
        }}
        fn on_err_str(e: IoError) -> String {{
            println("FAIL: ", e.kind, " via ", e.path);
            std::process::exit(1);
            return "";
        }}
        fn main() {{
            let empty = std::bytes::from_string("");
            let a1 = std::tar::pack_dir(empty, "pkg") or on_err(err);
            let hello = std::bytes::from_string("hello from hale tar");
            let a2 = std::tar::pack(a1, "pkg/hello.txt", hello) or on_err(err);
            let archive = std::tar::finish(a2) or on_err(err);
            // Read-back assertions (self-consistency).
            let n = std::tar::entries(archive) or on_err_int(err);
            if n != 2 {{ std::process::exit(1); }}
            let name1 = std::tar::entry_name(archive, 1) or on_err_str(err);
            if name1 != "pkg/hello.txt" {{ std::process::exit(1); }}
            let d1 = std::tar::entry_data(archive, 1) or on_err(err);
            if len(d1) != len(hello) {{ std::process::exit(1); }}
            // Ship to disk as .tar.gz, binary-safe.
            let tgz = std::compress::gzip(archive) or on_err(err);
            std::io::fs::write_bytes("{path}", tgz) or on_err_unit(err);
            println("packed");
        }}
    "#,
        path = tgz_path.display()
    );
    let bin = build("systar", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("packed"),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status,
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    // Cross-validation: the host tar must list both entries.
    let tar_out = Command::new("tar")
        .arg("-tzf")
        .arg(&tgz_path)
        .output()
        .expect("system tar");
    let _ = std::fs::remove_file(&tgz_path);
    let listing = String::from_utf8_lossy(&tar_out.stdout);
    assert!(
        tar_out.status.success(),
        "system tar rejected the archive: {}",
        String::from_utf8_lossy(&tar_out.stderr)
    );
    assert!(
        listing.contains("pkg") && listing.contains("pkg/hello.txt"),
        "system tar listing missing entries: {}",
        listing
    );
}
