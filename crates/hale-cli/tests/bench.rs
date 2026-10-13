//! `hale bench` — the Layer-3 runner: *_bench.hl discovery,
//! bench_* zero-param fns, synthesized self-calibrating driver,
//! ns/op + allocs/op reporting, --json records.

use std::process::Command;

const SRC: &str = r#"fn bench_arith() {
    let mut acc = 0;
    let mut i = 0;
    while i < 100 { acc = acc + i; i = i + 1; }
    if acc != 4950 { std::process::exit(1); }
}

fn bench_concat() {
    let s = "a" + "b" + "c";
    if len(s) != 3 { std::process::exit(1); }
}

fn helper_not_a_bench(n: Int) -> Int { return n; }
"#;

fn write_fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hale_bench_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("demo_bench.hl");
    std::fs::write(&f, SRC).expect("write");
    f
}

#[test]
fn discovers_calibrates_and_reports() {
    let f = write_fixture();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("bench")
        .arg(f.parent().unwrap())
        .output()
        .expect("run bench");
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bench_arith"), "{}", stdout);
    assert!(stdout.contains("bench_concat"), "{}", stdout);
    assert!(stdout.contains("ns/op"), "{}", stdout);
    assert!(!stdout.contains("helper_not_a_bench"), "{}", stdout);

    // -run filter narrows to matching benches.
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["bench", "-run", "arith"])
        .arg(f.parent().unwrap())
        .output()
        .expect("run bench filtered");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bench_arith"), "{}", stdout);
    assert!(!stdout.contains("bench_concat"), "{}", stdout);

    // --json emits structured records with positive timings.
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["bench", "--json"])
        .arg(f.parent().unwrap())
        .output()
        .expect("run bench json");
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("json");
    let items = v.as_array().expect("array");
    assert_eq!(items.len(), 2, "{:?}", items);
    for it in items {
        assert!(it["iters"].as_i64().unwrap() > 0);
        assert!(it["ns_per_op"].as_i64().unwrap() >= 0);
    }

    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn bench_file_with_main_is_rejected() {
    let dir = std::env::temp_dir().join(format!(
        "hale_bench_main_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("bad_bench.hl");
    std::fs::write(&f, "fn bench_x() { }\nfn main() { }\n").expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("bench")
        .arg(&f)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("must not define"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A bench's fixture is made by its setup, outside the timing (Riley's
/// ruling on a downstream handoff: a `bench_*` fn was timed whole, so
/// building its fixture counted toward ns/op). `setup_lookup` sleeps
/// 150 ms, longer than a whole calibration batch: timed, it would end the
/// first batch with ns/op above 150 ms; untimed, the batches grow to
/// thousands of calls of a few ns each. A setup returning nothing runs
/// for its effect before each batch of a parameterless bench, untimed too.
const SETUP_SRC: &str = r#"type Fixture { name: String; n: Int; }

fn setup_lookup() -> Fixture {
    std::time::sleep(150ms);
    return Fixture { name: std::str::upper("abc"), n: 7 };
}

fn bench_lookup(f: Fixture) {
    if len(f.name) != 3 { std::process::exit(1); }
    if f.n != 7 { std::process::exit(1); }
}

fn setup_tick() {
    std::time::sleep(150ms);
}

fn bench_tick() {
    let s = "a" + "b";
    if len(s) != 2 { std::process::exit(1); }
}
"#;

fn bench_dir(tag: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hale_bench_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("fx_bench.hl"), src).expect("write");
    dir
}

#[test]
fn a_setup_makes_the_fixture_outside_the_timing() {
    let dir = bench_dir("setup", SETUP_SRC);
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(["bench", "--json"]).arg(&dir).output().expect("run bench");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let items = v.as_array().expect("array");
    let names: Vec<&str> = items.iter().map(|it| it["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["bench_lookup", "bench_tick"], "a setup is no bench: {items:?}");
    for it in items {
        let ns = it["ns_per_op"].as_i64().unwrap();
        assert!(ns < 10_000_000, "the 150 ms setup is not timed: {it}");
        assert!(it["iters"].as_i64().unwrap() >= 1000, "batches grew past the setup: {it}");
    }
}

/// The pairing is checked: a bench that takes a fixture with no setup to
/// make it, and a setup whose fixture no bench takes, are refused naming
/// the fn to declare.
#[test]
fn a_fixture_and_its_setup_must_pair() {
    let run = |src: &str| {
        let dir = bench_dir("pair", src);
        let out = Command::new(env!("CARGO_BIN_EXE_hale")).arg("bench").arg(&dir).output().expect("run bench");
        let _ = std::fs::remove_dir_all(&dir);
        (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let (code, err) = run("fn bench_x(n: Int) { }\n");
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("`bench_x` takes a parameter") && err.contains("fn setup_x() -> T"), "{err}");
    let (code, err) = run("fn setup_y() -> Int { return 1; }\nfn bench_y() { }\n");
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("`setup_y` returns a fixture, but `bench_y` takes none"), "{err}");
}
