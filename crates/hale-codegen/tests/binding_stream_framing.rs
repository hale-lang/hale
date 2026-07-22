//! GH #231/#236: framed SOCK_STREAM unix transport. Darwin has
//! no AF_UNIX SOCK_SEQPACKET, so message boundaries come from a
//! [u64 len][u64 seq] header instead of the kernel. Linux runs
//! the same code path under LOTUS_UNIX_STREAM=1 (set for every
//! process on the socket — the wire formats don't mix), which is
//! what this test forces. The seq stamp is #236 item 1: loss on
//! a networked edge becomes computable (seq_gaps counter).
//!
//! Scenario: the re-arm shape (one subscriber lifetime, two
//! sequential publisher runs) — exercising framed listener
//! create/accept, framed send/recv, per-connection seq-space
//! reset at re-arm, and the counters dump.

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_stream_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn framed_stream_delivers_and_rearms_without_seq_gaps() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let sock = format!(
        "{}/hale-231-stream-{}-{}.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    );
    let sub_src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Sub {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                println("got=", self.seen);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                // Wait-until-delivered (cap ~12s), then settle so
                // the final peer EOF re-arms before teardown — a
                // fixed sleep flaked on loaded CI (window expired
                // mid-exchange; teardown ate a queued message).
                let mut waited = 0;
                while self.sub.seen < 4 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 120 {{
                        std::process::exit(3);
                    }}
                }}
                std::time::sleep(500ms);
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
                Evt <- T {{ n: 8 }};
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
        .env("LOTUS_UNIX_STREAM", "1")
        .env("LOTUS_BUS_COUNTERS_DUMP", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");
    for i in 0..2 {
        let p = Command::new(&pub_bin)
            .env("LOTUS_UNIX_STREAM", "1")
            .output()
            .expect("run publisher");
        assert!(
            p.status.success(),
            "publisher run {} failed: {:?}",
            i,
            String::from_utf8_lossy(&p.stderr)
        );
    }
    let out = sub.wait_with_output().expect("subscriber output");
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Two publishes per run × two runs, framed boundaries intact.
    assert!(
        stdout.contains("got=4"),
        "expected 4 framed deliveries across two peers.\nstdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    let line = stderr
        .lines()
        .find(|l| l.contains("[bus counters]") && l.contains("subject=evt"))
        .unwrap_or_else(|| {
            panic!("no counters line.\nstderr: {:?}", stderr)
        });
    assert!(
        line.contains("delivered=4")
            && line.contains("rearms=2")
            && line.contains("seq_gaps=0"),
        "framed counters wrong (seq space must reset per peer).\nline: {:?}",
        line
    );
}
