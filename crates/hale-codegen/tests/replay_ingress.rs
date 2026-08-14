//! GH #296 phase 5b — hermetic ingress: recorded wire input is
//! INJECTED under replay, and is a feedable tape under feed mode.
//!
//! What this holds the runtime to:
//!
//!   1. **The recording captures the wire form.** A listen-binding
//!      subscriber records each received message's verbatim wire
//!      bytes (ingress-flagged) — not just struct metadata.
//!   2. **Strict replay is hermetic and equivalent.** Replaying the
//!      subscriber WITHOUT the publisher (and without the socket):
//!      bound transports never open, the injector re-dispatches the
//!      tape with each delivery's RECORDED identity, and the run
//!      produces byte-identical stdout with ZERO divergences — the
//!      per-consumer order enforcement matched every injected
//!      delivery to its recorded consume.
//!   3. **Feed mode re-executes changed code on the same inputs.**
//!      A DIFFERENT program (same subject, different handler) fed
//!      the same tape processes the recorded payload values — the
//!      backtesting contract: same inputs, changed code, live
//!      everything else, and an exit report that says what was fed.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::{build_executable_with_options, BuildOptions};

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(name.to_string(), &program);
    let bundle = hale_types::Bundle::new(programs);
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let bin = harness::unique_bin(&format!("hale_test_ingress_{}", name));
    let options = BuildOptions {
        model_hash: Some(model_hash),
        ..BuildOptions::default()
    };
    build_executable_with_options(&program, &bin, &[], &options)
        .expect("build");
    bin
}

fn unique_socket_path() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}/hale-296-ingress-{}-{}.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    )
}

fn rec_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale_ingress_{}_{}.halerec",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&p);
    p
}

fn sub_src(sock: &str) -> String {
    format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Sub {{
            params {{ seen: Int = 0; total: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                self.total = self.total + t.n;
                println("got=", t.n);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.sub.seen < 3 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 120 {{
                        std::process::exit(3);
                    }}
                }}
                println("total=", self.sub.total);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    )
}

fn pub_src(sock: &str) -> String {
    format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                // Settle before the first send: the listener binds at
                // realize, BEFORE the subscriber's bus registration —
                // a send in that window is silently dropped (the
                // reader's no-deserializer path), which on a loaded
                // CI runner starved the recorded run. Not a
                // recording property; a boot-order one.
                std::time::sleep(400ms);
                Evt <- T {{ n: 7 }};
                std::time::sleep(50ms);
                Evt <- T {{ n: 11 }};
                std::time::sleep(50ms);
                Evt <- T {{ n: 23 }};
                std::time::sleep(200ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    )
}

/// Record a listener run fed by a real connecting publisher, and
/// return (recording path, the listener's recorded stdout).
fn record_session(
    tag: &str,
    sub_bin: &std::path::Path,
    pub_bin: &std::path::Path,
) -> (std::path::PathBuf, String) {
    let rec = rec_path(tag);
    let sub = Command::new(sub_bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn recorded subscriber");
    // The publisher's connect-with-retry rides out the listener's
    // boot; run it to completion.
    let p = Command::new(pub_bin).output().expect("run publisher");
    assert!(
        p.status.success(),
        "publisher failed: {}",
        String::from_utf8_lossy(&p.stderr)
    );
    let out = sub.wait_with_output().expect("subscriber exit");
    assert!(
        out.status.success(),
        "recorded subscriber failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("got=7")
            && stdout.contains("got=11")
            && stdout.contains("got=23")
            && stdout.contains("total=41"),
        "recorded run did not see the wire traffic:\n{}",
        stdout
    );
    (rec, stdout)
}

#[test]
fn strict_replay_injects_the_recorded_ingress_hermetically() {
    let sock = unique_socket_path();
    let sub_bin = build("sub", &sub_src(&sock));
    let pub_bin = build("pub", &pub_src(&sock));
    let (rec, recorded_stdout) = record_session("strict", &sub_bin, &pub_bin);

    // Replay with NO publisher and NO socket: the bound transport
    // must never open, and the tape must carry the run.
    let _ = std::fs::remove_file(&sock);
    let replayed = Command::new(&sub_bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay subscriber");
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(
        replayed.status.success(),
        "hermetic replay failed; stderr:\n{}",
        err
    );
    // Byte-identical observable behavior, from the tape alone.
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        recorded_stdout,
        "replay stdout differs from the recorded run's"
    );
    // The wire was hermetic and the tape was injected...
    assert!(
        err.contains("hermetic wire") && err.contains("injected"),
        "expected the hermetic-wire report; stderr:\n{}",
        err
    );
    // ...with recorded identities: every injected delivery matched
    // its recorded consume — zero divergences, not merely "ran".
    assert!(
        err.contains("0 divergences"),
        "expected a divergence-free replay; stderr:\n{}",
        err
    );
    // And the listener socket was never created.
    assert!(
        !std::path::Path::new(&sock).exists(),
        "replay must not open the bound transport"
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
}

#[test]
fn feed_mode_runs_changed_code_on_the_recorded_tape() {
    let sock = unique_socket_path();
    let sub_bin = build("feedsub", &sub_src(&sock));
    let pub_bin = build("feedpub", &pub_src(&sock));
    let (rec, _) = record_session("feed", &sub_bin, &pub_bin);

    // A DIFFERENT program: same subject, different handler logic
    // (scales each payload by 10). Model hash differs — feed mode
    // admits it by design.
    let changed = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Scaler {{
            params {{ seen: Int = 0; total: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                self.total = self.total + t.n * 10;
                println("scaled=", t.n * 10);
            }}
        }}
        main locus App {{
            params {{ s: Scaler = Scaler {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.s.seen < 3 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 120 {{
                        std::process::exit(3);
                    }}
                }}
                println("scaled_total=", self.s.total);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let changed_bin = build("feedchanged", &changed);

    let _ = std::fs::remove_file(&sock);
    let fed = Command::new(&changed_bin)
        .env("LOTUS_REPLAY_FEED", &rec)
        .output()
        .expect("feed changed program");
    let out = String::from_utf8_lossy(&fed.stdout);
    let err = String::from_utf8_lossy(&fed.stderr);
    assert!(
        fed.status.success(),
        "feed run failed; stderr:\n{}",
        err
    );
    // The changed code processed the RECORDED inputs.
    assert!(
        out.contains("scaled=70")
            && out.contains("scaled=110")
            && out.contains("scaled=230")
            && out.contains("scaled_total=410"),
        "changed program did not process the tape:\n{}",
        out
    );
    // And the exit report says what was fed.
    assert!(
        err.contains("hale feed:") && err.contains("injected"),
        "expected the feed report; stderr:\n{}",
        err
    );
    assert!(
        !std::path::Path::new(&sock).exists(),
        "feed must not open the bound transport"
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&changed_bin);
}

// ---- review-round canaries -----------------------------------------

/// FNV-1a/32 as the recorder spells it — the collision premise below
/// is asserted, not assumed.
fn fnv32(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// A message the original application REJECTED (deserialize failure)
/// must never enter the injectable tape: identity is allocated per
/// accepted message, so record and replay derive the same sequence.
#[cfg(target_os = "linux")]
#[test]
fn rejected_wire_never_enters_the_tape() {
    let sock = unique_socket_path();
    // A String payload: its wire form is length-framed, so a runt
    // frame genuinely FAILS deserialization (an Int payload's codec
    // is lenient enough to accept almost anything).
    let sub = format!(
        r#"
        type Msg {{ text: String = ""; }}
        topic Evt {{ payload: Msg; subject: "evt"; }}
        locus Sub {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe Evt as on_m; }}
            fn on_m(m: Msg) {{
                self.seen = self.seen + 1;
                println("got=", m.text);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.sub.seen < 3 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 120 {{ std::process::exit(3); }}
                }}
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let pubs = format!(
        r#"
        type Msg {{ text: String = ""; }}
        topic Evt {{ payload: Msg; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                std::time::sleep(400ms);
                Evt <- Msg {{ text: "alpha" }};
                std::time::sleep(50ms);
                Evt <- Msg {{ text: "beta" }};
                std::time::sleep(50ms);
                Evt <- Msg {{ text: "gamma" }};
                std::time::sleep(200ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let sub_bin = build("rejsub", &sub);
    let pub_bin = build("rejpub", &pubs);
    let rec = rec_path("rej");

    let sub = Command::new(&sub_bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn recorded subscriber");

    // First peer: a raw SEQPACKET connection delivering one garbage
    // frame the deserializer must reject. Retry the connect until
    // the listener is bound.
    unsafe {
        let mut fd = -1;
        for _ in 0..200 {
            fd = libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0);
            assert!(fd >= 0);
            let mut addr: libc::sockaddr_un = std::mem::zeroed();
            addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
            let bytes = sock.as_bytes();
            for (i, b) in bytes.iter().enumerate() {
                addr.sun_path[i] = *b as libc::c_char;
            }
            let len = std::mem::size_of::<libc::sa_family_t>()
                + bytes.len()
                + 1;
            if libc::connect(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                len as libc::socklen_t,
            ) == 0
            {
                break;
            }
            libc::close(fd);
            fd = -1;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(fd >= 0, "could not connect the garbage peer");
        // A 3-byte runt frame: no codec for T can accept it. (A
        // plausible-length frame of 0xFF bytes deserializes fine —
        // the codec reads an Int from it — which is the point of
        // the accepted/rejected distinction being the CODEC'''s.)
        let junk = [0xFFu8; 3];
        assert!(
            libc::send(fd, junk.as_ptr() as *const _, junk.len(), 0)
                > 0
        );
        std::thread::sleep(std::time::Duration::from_millis(150));
        libc::close(fd);
    }

    // Second peer: the honest publisher (3 valid messages).
    let p = Command::new(&pub_bin).output().expect("run publisher");
    assert!(p.status.success());
    let out = sub.wait_with_output().expect("subscriber exit");
    assert!(
        out.status.success(),
        "recorded subscriber failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let recorded_stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    // Replay hermetically: the tape must carry EXACTLY the three
    // accepted messages — the rejected frame is not input.
    let _ = std::fs::remove_file(&sock);
    let replayed = Command::new(&sub_bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay");
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(replayed.status.success(), "stderr:\n{}", err);
    assert!(
        err.contains("3 of 3 recorded ingress payload(s) injected"),
        "tape must hold only the accepted messages; stderr:\n{}",
        err
    );
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        recorded_stdout
    );
    assert!(err.contains("0 divergences"), "stderr:\n{}", err);

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
}

/// Tape routing is by FULL subject string: two subjects whose
/// FNV-1a/32 hashes collide must each receive their own traffic.
#[test]
fn colliding_subject_hashes_route_by_full_name() {
    // The premise, asserted: these collide under the recorder's hash.
    assert_eq!(fnv32("i4lt8ja7"), fnv32("w6s0e3sz"));

    let sock_a = unique_socket_path();
    let sock_b = format!("{}.b", sock_a);
    let ready = format!("{}.ready", sock_a);
    let sub = format!(
        r#"
        type TA {{ n: Int = 0; }}
        type TB {{ n: Int = 0; }}
        topic A {{ payload: TA; subject: "i4lt8ja7"; }}
        topic B {{ payload: TB; subject: "w6s0e3sz"; }}
        locus SubA {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe A as on_a; }}
            fn on_a(t: TA) {{
                self.seen = self.seen + 1;
                println("a=", t.n);
            }}
        }}
        locus SubB {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe B as on_b; }}
            fn on_b(t: TB) {{
                self.seen = self.seen + 1;
                println("b=", t.n);
            }}
        }}
        main locus App {{
            params {{ sa: SubA = SubA {{ }}; sb: SubB = SubB {{ }}; }}
            bindings {{
                A: unix("{}", role: listen);
                B: unix("{}", role: listen);
            }}
            run() {{
                // run() begins only after every boot registration:
                // the ready-file is the true "subscribers exist"
                // signal (the sockets exist earlier, at realize —
                // a message in that window drops silently).
                std::io::fs::write_file("{}", "ready") or discard;
                // At least one message per subject proves routing;
                // demanding every message makes the test hostage to
                // residual live-ingest loss under load — replay
                // equivalence holds for whatever the tape carries.
                let mut waited = 0;
                while self.sa.seen < 1 || self.sb.seen < 1 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 120 {{ std::process::exit(3); }}
                }}
                std::time::sleep(400ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock_a, sock_b, ready
    );
    let pubs = format!(
        r#"
        type TA {{ n: Int = 0; }}
        type TB {{ n: Int = 0; }}
        topic A {{ payload: TA; subject: "i4lt8ja7"; }}
        topic B {{ payload: TB; subject: "w6s0e3sz"; }}
        main locus App {{
            bus {{ publish A; publish B; }}
            bindings {{
                A: unix("{}", role: connect);
                B: unix("{}", role: connect);
            }}
            run() {{
                A <- TA {{ n: 101 }};
                B <- TB {{ n: 202 }};
                std::time::sleep(50ms);
                A <- TA {{ n: 103 }};
                B <- TB {{ n: 204 }};
                std::time::sleep(900ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock_a, sock_b
    );
    let sub_bin = build("collsub", &sub);
    let pub_bin = build("collpub", &pubs);
    let rec = rec_path("coll");

    let subp = Command::new(&sub_bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(30);
    while !std::path::Path::new(&ready).exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "subscriber never reached run()"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let p = Command::new(&pub_bin).output().expect("pubs");
    assert!(p.status.success());
    let out = subp.wait_with_output().expect("sub exit");
    assert!(
        out.status.success(),
        "recorded collision run failed:\nstdout:{}\nstderr:{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let recorded_stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // The routing property: A-values (10x) only ever print under
    // `a=`, B-values (20x) only under `b=` — colliding hashes must
    // not cross the streams, live or replayed.
    assert!(
        recorded_stdout.contains("a=10")
            && recorded_stdout.contains("b=20")
            && !recorded_stdout.contains("a=20")
            && !recorded_stdout.contains("b=10"),
        "recorded routing crossed the streams:\n{}",
        recorded_stdout
    );

    let _ = std::fs::remove_file(&sock_a);
    let _ = std::fs::remove_file(&sock_b);
    let replayed = Command::new(&sub_bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay");
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(replayed.status.success(), "stderr:\n{}", err);
    // Byte-identical routing: colliding hashes cannot cross the
    // streams when identity is the full subject.
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        recorded_stdout,
        "replay stderr:\n{}",
        err
    );
    assert!(err.contains("0 divergences"), "stderr:\n{}", err);

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&ready);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
}

/// Strict pacing at a bounded queue: the original ingress was slow
/// enough that a capacity-1 subscriber shed nothing; an unpaced
/// replay would burst-shed at enqueue, which no dequeue-side order
/// gate can undo. Paced injection must deliver every message.
#[test]
fn bounded_capacity_one_replays_without_shedding() {
    let sock = unique_socket_path();
    let sub = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Sub {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe Evt as on_e bounded(1, drop_old); }}
            fn on_e(t: T) {{
                self.seen = self.seen + 1;
                println("got=", t.n);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.sub.seen < 6 {{
                    std::time::sleep(40ms);
                    waited = waited + 1;
                    if waited > 400 {{ std::process::exit(3); }}
                }}
                println("all=", self.sub.seen);
                std::time::sleep(100ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let pubs = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                std::time::sleep(400ms);
                let mut i = 0;
                while i < 6 {{
                    Evt <- T {{ n: i }};
                    std::time::sleep(150ms);
                    i = i + 1;
                }}
                std::time::sleep(200ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let sub_bin = build("capsub", &sub);
    let pub_bin = build("cappub", &pubs);
    let rec = rec_path("cap");
    let subp = Command::new(&sub_bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let p = Command::new(&pub_bin).output().expect("pub");
    assert!(p.status.success());
    let out = subp.wait_with_output().expect("sub exit");
    assert!(
        out.status.success(),
        "recorded run failed (original must not shed): {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let recorded_stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        recorded_stdout.contains("all=6"),
        "original must shed nothing:\n{}",
        recorded_stdout
    );

    let _ = std::fs::remove_file(&sock);
    let replayed = Command::new(&sub_bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay");
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(replayed.status.success(), "stderr:\n{}", err);
    // Zero shedding: all 8 delivered, byte-identical, no divergence.
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        recorded_stdout,
        "paced injection must not shed at the bounded queue"
    );
    assert!(err.contains("0 divergences"), "stderr:\n{}", err);

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
}

/// Feed targets that DROPPED the listener binding still get the
/// tape: tape presence, not the survival of the old listener
/// declaration, decides that injection starts.
#[test]
fn feed_reaches_a_target_without_the_listener_binding() {
    let sock = unique_socket_path();
    let sub_bin = build("nobindsub", &sub_src(&sock));
    let pub_bin = build("nobindpub", &pub_src(&sock));
    let (rec, _) = record_session("nobind", &sub_bin, &pub_bin);

    // Same subscribed subject; NO bindings block at all.
    let changed = r#"
        type T { n: Int = 0; }
        topic Evt { payload: T; subject: "evt"; }
        locus Sum {
            params { seen: Int = 0; total: Int = 0; }
            bus { subscribe Evt as on_e; }
            fn on_e(t: T) {
                self.seen = self.seen + 1;
                self.total = self.total + t.n;
            }
        }
        main locus App {
            params { s: Sum = Sum { }; }
            run() {
                let mut waited = 0;
                while self.s.seen < 3 {
                    std::time::sleep(50ms);
                    waited = waited + 1;
                    if waited > 100 { std::process::exit(3); }
                }
                println("sum=", self.s.total);
            }
        }
        fn main() { App { }; }
    "#;
    let changed_bin = build("nobindchanged", changed);
    let fed = Command::new(&changed_bin)
        .env("LOTUS_REPLAY_FEED", &rec)
        .output()
        .expect("feed");
    let outp = String::from_utf8_lossy(&fed.stdout);
    let err = String::from_utf8_lossy(&fed.stderr);
    assert!(fed.status.success(), "stderr:\n{}", err);
    assert!(
        outp.contains("sum=41"),
        "the binding-less target must still receive the tape:\n{}\n{}",
        outp,
        err
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&changed_bin);
}

/// Same subject, changed payload shape: wire identity refuses the
/// injection (named), and an unfed tape fails feed by default —
/// `--allow-unmatched-feed` is the explicit acceptance.
#[test]
fn incompatible_payload_shape_fails_feed_closed() {
    let sock = unique_socket_path();
    let sub_bin = build("shapesub", &sub_src(&sock));
    let pub_bin = build("shapepub", &pub_src(&sock));
    let (rec, _) = record_session("shape", &sub_bin, &pub_bin);

    // Subject "evt" survives; the payload gains a field — a
    // different canonical shape, so the recorded bytes must not be
    // fed to the new deserializer as a plausible wrong value.
    let changed = r#"
        type T { n: Int = 0; scale: Int = 1; }
        topic Evt { payload: T; subject: "evt"; }
        locus Sub {
            params { seen: Int = 0; }
            bus { subscribe Evt as on_e; }
            fn on_e(t: T) { self.seen = self.seen + 1; }
        }
        main locus App {
            params { s: Sub = Sub { }; }
            run() { std::time::sleep(300ms); }
        }
        fn main() { App { }; }
    "#;
    let changed_bin = build("shapechanged", changed);

    let fed = Command::new(&changed_bin)
        .env("LOTUS_REPLAY_FEED", &rec)
        .output()
        .expect("feed");
    let err = String::from_utf8_lossy(&fed.stderr);
    assert!(
        !fed.status.success(),
        "an entirely-unfed tape must fail feed by default; stderr:\n{}",
        err
    );
    assert!(
        err.contains("incompatible") && err.contains("shape"),
        "the refusal must name the incompatibility; stderr:\n{}",
        err
    );

    let allowed = Command::new(&changed_bin)
        .env("LOTUS_REPLAY_FEED", &rec)
        .env("LOTUS_REPLAY_FEED_ALLOW_UNMATCHED", "1")
        .output()
        .expect("feed allowed");
    assert!(
        allowed.status.success(),
        "--allow-unmatched-feed must accept the partial feed: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&changed_bin);
}

/// A binding backend with no replay class fails CLOSED at the
/// runtime seam too (the CLI refuses earlier with a nicer message):
/// a replayed process must never open, create, or mutate live
/// shared memory.
#[test]
fn shm_ring_refuses_replay_and_touches_no_shared_memory() {
    // Any valid recording will do — admission is the CLI's job; the
    // runtime guard fires at shm open regardless of the tape.
    let sock = unique_socket_path();
    let sub_bin = build("shmrecsub", &sub_src(&sock));
    let pub_bin = build("shmrecpub", &pub_src(&sock));
    let (rec, _) = record_session("shmrec", &sub_bin, &pub_bin);

    let shm_name = format!(
        "/hale-296-shmguard-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    let prog = format!(
        r#"
        type W {{ n: Int = 0; }}
        topic World {{ payload: W; subject: "world"; }}
        locus Pub {{
            bus {{ publish World; }}
            run() {{ World <- W {{ n: 1 }}; }}
        }}
        main locus App {{
            params {{ p: Pub = Pub {{ }}; }}
            bindings {{
                World: shm_ring("{}", slot_count: 64,
                                on_overflow: drop);
            }}
            run() {{ std::time::sleep(50ms); }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        shm_name
    );
    let shm_bin = build("shmguard", &prog);

    let replayed = Command::new(&shm_bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay shm program");
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(
        !replayed.status.success(),
        "shm_ring must refuse replay; stderr:\n{}",
        err
    );
    assert!(
        err.contains("shm_ring") && err.contains("no replay class"),
        "the refusal must name the backend; stderr:\n{}",
        err
    );
    let shm_path = format!("/dev/shm{}", shm_name);
    assert!(
        !std::path::Path::new(&shm_path).exists(),
        "replay must not create the shared-memory object"
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&shm_bin);
}

/// Feed's verdict must not depend on teardown having run (review
/// round 2, finding 4): a target that exits during birth — before
/// the boot-phase injector even starts — must still fail with the
/// whole tape reported unprocessed, because the report derives the
/// unclassified remainder itself.
#[test]
fn feed_early_exit_cannot_fail_open() {
    let sock = unique_socket_path();
    let sub_bin = build("earlysub", &sub_src(&sock));
    let pub_bin = build("earlypub", &pub_src(&sock));
    let (rec, _) = record_session("early", &sub_bin, &pub_bin);

    let bail = r#"
        type T { n: Int = 0; }
        topic Evt { payload: T; subject: "evt"; }
        locus Sub {
            params { seen: Int = 0; }
            bus { subscribe Evt as on_e; }
            fn on_e(t: T) { self.seen = self.seen + 1; }
        }
        main locus App {
            params { s: Sub = Sub { }; }
            birth() { std::process::exit(0); }
            run() { std::time::sleep(100ms); }
        }
        fn main() { App { }; }
    "#;
    let bail_bin = build("earlybail", bail);

    let fed = Command::new(&bail_bin)
        .env("LOTUS_REPLAY_FEED", &rec)
        .output()
        .expect("feed early-exit target");
    let err = String::from_utf8_lossy(&fed.stderr);
    assert!(
        !fed.status.success(),
        "an exit(0) before injection must not fail open; stderr:\n{}",
        err
    );
    assert!(
        err.contains("0 of 3") && err.contains("unprocessed"),
        "the whole tape must be reported unprocessed; stderr:\n{}",
        err
    );

    let allowed = Command::new(&bail_bin)
        .env("LOTUS_REPLAY_FEED", &rec)
        .env("LOTUS_REPLAY_FEED_ALLOW_UNMATCHED", "1")
        .output()
        .expect("feed allowed");
    assert!(
        allowed.status.success(),
        "--allow-unmatched-feed accepts the (empty) partial feed: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&bail_bin);
}
