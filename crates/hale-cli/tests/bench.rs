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
