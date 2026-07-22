//! GH #236 item 2: per-binding telemetry counters, bumped at the
//! transport choke points, surfaced by LOTUS_BUS_COUNTERS_DUMP=1
//! as one stderr line per binding at teardown. No in-process
//! consumer yet — the iris observer attaches later; the dump is
//! the operator/test surface.
//!
//! Scenario reuses the re-arm shape: one subscriber lifetime
//! serves two sequential publisher runs. Expectations:
//!   subscriber: delivered=2, rearms=2 (one per peer EOF)
//!   publisher (each run): sent=1, no failures.

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_counters_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn counters_dump_reflects_rearm_scenario() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let sock = format!(
        "{}/hale-236-ctr-{}-{}.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    );
    let sub_src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Sub {{
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{ }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                std::time::sleep(4000ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let pub_src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                Evt <- T {{ n: 7 }};
                std::time::sleep(200ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let sub_bin = build("sub", &sub_src);
    let pub_bin = build("pub", &pub_src);

    let mut sub = Command::new(&sub_bin)
        .env("LOTUS_BUS_COUNTERS_DUMP", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");
    let p1 = Command::new(&pub_bin)
        .env("LOTUS_BUS_COUNTERS_DUMP", "1")
        .output()
        .expect("run publisher 1");
    assert!(p1.status.success());
    let p2 = Command::new(&pub_bin)
        .env("LOTUS_BUS_COUNTERS_DUMP", "1")
        .output()
        .expect("run publisher 2");
    assert!(p2.status.success());

    let sub_out = sub.wait_with_output().expect("subscriber output");
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sock);

    let sub_err = String::from_utf8_lossy(&sub_out.stderr);
    let pub_err = String::from_utf8_lossy(&p1.stderr);
    let sub_line = sub_err
        .lines()
        .find(|l| l.contains("[bus counters]") && l.contains("subject=evt"))
        .unwrap_or_else(|| {
            panic!("no counters dump line from subscriber.\nstderr: {:?}", sub_err)
        });
    assert!(
        sub_line.contains("delivered=2") && sub_line.contains("rearms=2"),
        "subscriber counters wrong.\nline: {:?}",
        sub_line
    );
    let pub_line = pub_err
        .lines()
        .find(|l| l.contains("[bus counters]") && l.contains("subject=evt"))
        .unwrap_or_else(|| {
            panic!("no counters dump line from publisher.\nstderr: {:?}", pub_err)
        });
    assert!(
        pub_line.contains("sent=1") && pub_line.contains("send_failures=0"),
        "publisher counters wrong.\nline: {:?}",
        pub_line
    );
}
