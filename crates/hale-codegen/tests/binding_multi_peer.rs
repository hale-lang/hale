//! DNA F.13 (GH #529 prep): a listen binding serves many peers at once.
//!
//! The serve loop used to accept ONE peer and read it to EOF before
//! accepting the next; a second connector sat in the backlog with its
//! messages unread for as long as the first stayed connected. Now the
//! listener polls every accepted peer: two publishers connected at the
//! same time — one long-lived, one that comes and goes — both deliver,
//! and each keeps its own framed seq space (no gaps).

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/transport.rs"]
mod transport_support;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_multipeer_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn two_connected_publishers_both_deliver_to_one_listener() {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let sock = format!("{}/hale-f13-{}-{}.sock", std::env::temp_dir().display(), std::process::id(), nanos);
    let sub_src = format!(
        r#"
        type T {{ from: Int = 0; n: Int = 0; }}
        topic Evt {{ payload: T; subject: "f13.evt"; }}
        locus Sub {{
            params {{ seen: Int = 0; a: Int = 0; b: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                if t.from == 1 {{ self.a = self.a + 1; }}
                if t.from == 2 {{ self.b = self.b + 1; }}
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{sock}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.sub.seen < 8 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 200 {{
                        println("timeout seen=", self.sub.seen, " a=", self.sub.a, " b=", self.sub.b);
                        std::process::exit(3);
                    }}
                }}
                std::time::sleep(300ms);
                println("seen=", self.sub.seen, " a=", self.sub.a, " b=", self.sub.b);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock = sock
    );
    // Publisher 1 stays connected for ~3s, sending slowly; publisher 2
    // connects while 1 is still attached, sends its four, and leaves.
    let pub_src = |from: i32, pause: &str, count: i32| {
        format!(
            r#"
        type T {{ from: Int = 0; n: Int = 0; }}
        topic Evt {{ payload: T; subject: "f13.evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{sock}", role: connect); }}
            run() {{
                let mut i = 0;
                while i < {count} {{
                    Evt <- T {{ from: {from}, n: i }};
                    std::time::sleep({pause});
                    i = i + 1;
                }}
                std::time::sleep(300ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
            sock = sock,
            from = from,
            pause = pause,
            count = count
        )
    };
    let sub_bin = build("sub", &sub_src);
    let slow_bin = build("slow", &pub_src(1, "600ms", 4));
    let fast_bin = build("fast", &pub_src(2, "20ms", 4));
    let sub = Command::new(&sub_bin)
        .env("LOTUS_UNIX_STREAM", "1")
        .env("LOTUS_BUS_COUNTERS_DUMP", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");
    assert!(transport_support::wait_for_listener(&sock, std::time::Duration::from_secs(30)));
    let slow = Command::new(&slow_bin).env("LOTUS_UNIX_STREAM", "1").stdout(Stdio::null()).spawn().expect("slow publisher");
    std::thread::sleep(std::time::Duration::from_millis(400));
    // the fast publisher connects WHILE the slow one holds a connection
    let fast = Command::new(&fast_bin).env("LOTUS_UNIX_STREAM", "1").output().expect("fast publisher");
    assert!(fast.status.success(), "fast publisher: {}", String::from_utf8_lossy(&fast.stderr));
    let slow = slow.wait_with_output().expect("slow publisher");
    assert!(slow.status.success(), "slow publisher: {}", String::from_utf8_lossy(&slow.stderr));
    let out = sub.wait_with_output().expect("subscriber output");
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&slow_bin);
    let _ = std::fs::remove_file(&fast_bin);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "subscriber exit {:?}\nstdout: {stdout}\nstderr: {stderr}", out.status);
    assert!(stdout.contains("seen=8 a=4 b=4"), "both peers delivered while connected together:\n{stdout}\n{stderr}");
    let line = stderr
        .lines()
        .find(|l| l.contains("[bus counters]") && l.contains("subject=f13.evt"))
        .unwrap_or_else(|| panic!("no counters line.\nstderr: {stderr}"));
    // The fast peer's EOF is a re-arm; the slow peer may still be
    // connected when the listener exits (timing), so one or two.
    assert!(line.contains("delivered=8") && line.contains("seq_gaps=0") && (line.contains("rearms=1") || line.contains("rearms=2")), "per-peer seq spaces, peer EOFs:\n{line}");
}
