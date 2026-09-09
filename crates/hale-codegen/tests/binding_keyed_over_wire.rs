//! DNA F.12 (GH #529 prep): a keyed subscription hears a wire delivery.
//!
//! Remote fanout is unkeyed — the wire carries no key material — so a
//! `subscribe T as h where key == self.k` subscription behind a listen
//! binding received nothing: the receive side dispatched unkeyed and the
//! keyed dispatch skips filtered subscribers by design. Codegen now
//! synthesizes a per-keyed-topic extractor (the publish site's exact
//! computation over the deserialized payload) and the runtime derives
//! the key on every inbound path. Both key kinds: Int and String.

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/transport.rs"]
mod transport_support;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_keyedwire_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn keyed_subscribers_receive_only_their_key_over_a_unix_binding() {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let sock = format!("{}/hale-f12-{}-{}.sock", std::env::temp_dir().display(), std::process::id(), nanos);
    let sub_src = format!(
        r#"
        type Tick {{ lane: Int = 0; n: Int = 0; }}
        type Note {{ who: String = ""; n: Int = 0; }}
        topic Ticks {{ payload: Tick; subject: "f12.ticks"; keyed_by lane; }}
        topic Notes {{ payload: Note; subject: "f12.notes"; keyed_by who; }}
        locus Lane {{
            params {{ lane: Int = 0; seen: Int = 0; }}
            bus {{ subscribe Ticks as on_tick where key == self.lane; }}
            fn on_tick(t: Tick) {{
                self.seen = self.seen + 1;
                println("lane", self.lane, " got lane=", t.lane, " n=", t.n);
            }}
        }}
        locus Inbox {{
            params {{ who: String = ""; seen: Int = 0; }}
            bus {{ subscribe Notes as on_note where key == self.who; }}
            fn on_note(m: Note) {{
                self.seen = self.seen + 1;
                println("inbox ", self.who, " got who=", m.who, " n=", m.n);
            }}
        }}
        main locus App {{
            params {{
                a: Lane = Lane {{ lane: 1 }};
                b: Lane = Lane {{ lane: 2 }};
                riley: Inbox = Inbox {{ who: "riley" }};
                sam: Inbox = Inbox {{ who: "sam" }};
            }}
            bindings {{
                Ticks: unix("{sock}.ticks", role: listen);
                Notes: unix("{sock}.notes", role: listen);
            }}
            run() {{
                let mut waited = 0;
                while self.a.seen + self.b.seen < 3 || self.riley.seen + self.sam.seen < 2 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 150 {{ std::process::exit(3); }}
                }}
                std::time::sleep(300ms);
                println("a=", self.a.seen, " b=", self.b.seen, " riley=", self.riley.seen, " sam=", self.sam.seen);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock = sock
    );
    let pub_src = format!(
        r#"
        type Tick {{ lane: Int = 0; n: Int = 0; }}
        type Note {{ who: String = ""; n: Int = 0; }}
        topic Ticks {{ payload: Tick; subject: "f12.ticks"; keyed_by lane; }}
        topic Notes {{ payload: Note; subject: "f12.notes"; keyed_by who; }}
        main locus App {{
            bus {{ publish Ticks; publish Notes; }}
            bindings {{
                Ticks: unix("{sock}.ticks", role: connect);
                Notes: unix("{sock}.notes", role: connect);
            }}
            run() {{
                Ticks <- Tick {{ lane: 1, n: 10 }};
                Ticks <- Tick {{ lane: 2, n: 20 }};
                Ticks <- Tick {{ lane: 1, n: 11 }};
                Ticks <- Tick {{ lane: 9, n: 99 }};
                Notes <- Note {{ who: "riley", n: 1 }};
                Notes <- Note {{ who: "sam", n: 2 }};
                Notes <- Note {{ who: "nobody", n: 3 }};
                std::time::sleep(300ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock = sock
    );
    let sub_bin = build("sub", &sub_src);
    let pub_bin = build("pub", &pub_src);
    let sub = Command::new(&sub_bin).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn subscriber");
    assert!(transport_support::wait_for_listener(&format!("{sock}.ticks"), std::time::Duration::from_secs(30)));
    assert!(transport_support::wait_for_listener(&format!("{sock}.notes"), std::time::Duration::from_secs(30)));
    let p = Command::new(&pub_bin).output().expect("run publisher");
    assert!(p.status.success(), "publisher: {}", String::from_utf8_lossy(&p.stderr));
    let out = sub.wait_with_output().expect("subscriber output");
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(format!("{sock}.ticks"));
    let _ = std::fs::remove_file(format!("{sock}.notes"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "subscriber exit {:?}\nstdout: {stdout}\nstderr: {stderr}", out.status);
    // Int keys: lane 1 hears its two, lane 2 its one, lane 9 nobody.
    assert!(stdout.contains("a=2 b=1"), "int-keyed delivery over the wire:\n{stdout}\n{stderr}");
    assert!(!stdout.contains("n=99"), "an unmatched key reaches no filtered subscriber:\n{stdout}");
    assert!(stdout.contains("lane1 got lane=1 n=10") && stdout.contains("lane1 got lane=1 n=11") && stdout.contains("lane2 got lane=2 n=20"), "{stdout}");
    // String keys: hash-gated, then compared by value on the receiver.
    assert!(stdout.contains("riley=1 sam=1"), "string-keyed delivery over the wire:\n{stdout}\n{stderr}");
    assert!(!stdout.contains("who=nobody"), "{stdout}");
}
