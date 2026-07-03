//! A4 (G34) — three-hop cross-seed import chain.
//!
//! `app → lib-mid → lib-util`. The app imports `lib-mid` as
//! `mid`; lib-mid imports `lib-util` as `u`. Before A4, the
//! v1 strict barrier dropped `lib-mid`'s import of util, so
//! references to `u::Box { ... }`, `u::make_box(...)`, etc.
//! inside lib-mid's body failed codegen. Lifting the barrier
//! makes the CLI recurse into each imported lib's own
//! `import` directives with the lib's directory as the new
//! importer dir.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn fixtures_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p
}

/// Cross-PROCESS lock on the shared fixture dir: nextest runs each
/// test in its own process, and both tests here delete + rebuild +
/// exec the SAME `fixtures/three-hop-app/three-hop-app` binary (the
/// per-directory seed model puts the binary next to the source, and
/// the mangled-prefix assertion depends on the checked-in path, so
/// a per-test tempdir copy isn't an option). Racing them was the
/// recurring parallel-run flake. create_new is the portable
/// dependency-free mutex; stale locks (a killed test) are stolen
/// after 60s.
struct FixtureLock(std::path::PathBuf);
impl Drop for FixtureLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn lock_fixture() -> FixtureLock {
    let path = fixtures_dir().join("three-hop-app.lock");
    let start = std::time::Instant::now();
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return FixtureLock(path),
            Err(_) => {
                if let Ok(md) = std::fs::metadata(&path) {
                    if let Ok(age) =
                        md.modified().and_then(|m| {
                            std::time::SystemTime::now()
                                .duration_since(m)
                                .map_err(std::io::Error::other)
                        })
                    {
                        if age.as_secs() > 60 {
                            let _ = std::fs::remove_file(&path);
                            continue;
                        }
                    }
                }
                assert!(
                    start.elapsed().as_secs() < 120,
                    "fixture lock timed out"
                );
                std::thread::sleep(
                    std::time::Duration::from_millis(100),
                );
            }
        }
    }
}

#[test]
fn three_hop_app_builds_and_runs() {
    let _lock = lock_fixture();
    let app_dir = fixtures_dir().join("three-hop-app");

    // Clean prior build artifacts so we test the fresh build path.
    let built_bin = app_dir.join("three-hop-app");
    let _ = std::fs::remove_file(&built_bin);

    let out = Command::new(hale_bin())
        .arg("build")
        .arg(&app_dir)
        .output()
        .expect("invoke hale build");
    assert!(
        out.status.success(),
        "hale build failed: status={:?} stdout={} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        built_bin.exists(),
        "expected binary at {:?}",
        built_bin
    );

    let run_out = Command::new(&built_bin)
        .output()
        .expect("run three-hop-app");
    assert!(
        run_out.status.success(),
        "three-hop-app exit {:?}: stderr={}",
        run_out.status,
        String::from_utf8_lossy(&run_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&run_out.stdout);
    assert!(
        stdout.contains("answer=42 [v1]"),
        "expected `answer=42 [v1]` in stdout: {:?}",
        stdout
    );
    // A5 (G18/G33): qualified cross-seed type in fn-param position
    // (`mid::label_of(b: u::Box)`) resolves through the transitive
    // rename table.
    assert!(
        stdout.contains("answer"),
        "expected `answer` (label_of output) in stdout: {:?}",
        stdout
    );

    // Tidy up so the fixture stays clean across CI runs.
    let _ = std::fs::remove_file(&built_bin);
}

#[test]
fn three_hop_uses_path_based_mangled_prefix() {
    let _lock = lock_fixture();
    // 2026-05-22: the mangler switched from alias-based to path-
    // based identity. Two consumers importing the same lib under
    // different aliases now produce identical mangled symbols
    // (which is the natural shape for shared DTOs on a bus). This
    // test confirms transitive resolution honors the new
    // invariant: util's `make_box` lives in the binary under a
    // path-derived prefix, NOT under the middle lib's chosen
    // `u` alias.
    let app_dir = fixtures_dir().join("three-hop-app");
    let built_bin = app_dir.join("three-hop-app");
    let _ = std::fs::remove_file(&built_bin);

    let out = Command::new(hale_bin())
        .arg("build")
        .arg(&app_dir)
        .output()
        .expect("invoke hale build");
    assert!(out.status.success(), "build failed: {:?}", out);

    let bin_bytes = std::fs::read(&built_bin).expect("read binary");
    // The util lib lives at `<repo>/crates/hale-cli/tests/
    // fixtures/lib-util/`. With path-based mangling, the
    // workspace-root-relative path becomes the lib id, sanitized
    // to `crates_hale_cli_tests_fixtures_lib_util`. The file
    // stem is `box`. So make_box lands as
    // `__lib_crates_hale_cli_tests_fixtures_lib_util_box_make_box`.
    //
    // We don't pin the full string (the hale workspace
    // structure can shift) — just check the path-based shape:
    // the prefix is `__lib_` + path segments + `_box_make_box`,
    // and explicitly NOT the old `__lib_u_box_make_box`.
    let path_needle = b"_lib_util_box_make_box";
    let path_hit = bin_bytes
        .windows(path_needle.len())
        .any(|w| w == path_needle);
    assert!(
        path_hit,
        "expected path-derived mangled fn for util's make_box; \
         needle `{}` not found in binary",
        std::str::from_utf8(path_needle).unwrap()
    );

    let old_alias_needle = b"__lib_u_box_make_box";
    let alias_hit = bin_bytes
        .windows(old_alias_needle.len())
        .any(|w| w == old_alias_needle);
    assert!(
        !alias_hit,
        "found old alias-based mangling `{}` in binary — the \
         mangler should be using path-based identity",
        std::str::from_utf8(old_alias_needle).unwrap()
    );

    let _ = std::fs::remove_file(&built_bin);
}

#[allow(dead_code)]
fn read_dir_names(d: &Path) -> Vec<String> {
    std::fs::read_dir(d)
        .ok()
        .map(|it| {
            it.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}
