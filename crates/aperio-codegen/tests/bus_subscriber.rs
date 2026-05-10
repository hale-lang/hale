//! m59: subscriber-side reader thread integration test.
//!
//! Builds two lotus binaries — a subscriber and a publisher —
//! and exercises the full cross-process bus path end-to-end:
//!
//!   subscriber (LISTEN role)               publisher (CONNECT role)
//!     |                                       |
//!     | spawn reader thread (blocks accept)   | <- "evt" | Ping{n}
//!     |   <----------- AF_UNIX SEQPACKET -----|
//!     | recv → lotus_bus_local_dispatch       |
//!     | enqueue cell on cooperative queue     |
//!     | main: time::sleep(500ms); yield;      |
//!     | drain → Sub.on_evt(Ping{n})           |
//!     |   prints "subscriber got n=..."       |
//!
//! The subscriber's stdout is asserted to contain the printed
//! Ping value, proving the reader thread → local dispatch path
//! delivers cross-process publishes into a real lotus handler.
//!
//! At v0.1 the wire format is raw struct bytes: same arch +
//! same compiler version means identical layout on both sides.
//! The serializer milestone (m60+) replaces this with a
//! field-by-field little-endian encoding to defend against
//! padding drift across binary versions and to enable
//! heterogeneous-host targets.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-m59-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ));
    p
}

fn build_binary(src: &str, tag: &str) -> PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag, "bin");
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn two_lotus_binaries_round_trip_a_publish() {
    // Sentinel chosen so each byte is unique + ASCII-printable —
    // shows up as "HGFEDCBA" in stdout if a layout regression
    // ever scrambles the bytes.
    let sentinel: i64 = 0x4142_4344_4546_4748;

    let subscriber_src = r#"
        type Ping {
            n: Int;
        }

        locus Sub {
            bus {
                subscribe "evt" as on_evt of type Ping;
            }
            fn on_evt(p: Ping) {
                println("subscriber got n=", p.n);
            }
        }

        fn main() {
            Sub { };
            // Wait for the cross-process publish to arrive. The
            // reader thread (spawned by lotus_bus_load_config when
            // the LISTEN-role config entry registers) enqueues the
            // cell on the cooperative queue while we're blocked in
            // sleep. After sleep, `yield` drains the queue ->
            // fires Sub.on_evt synchronously on the main thread.
            time::sleep(500ms);
            yield;
        }
    "#;

    let publisher_src = format!(
        r#"
        type Ping {{
            n: Int;
        }}

        // Dummy local subscriber so the BusState gate is satisfied
        // at compile time (codegen errors on `<-` without any
        // `bus subscribe` declared somewhere in the program).
        locus Sub {{
            bus {{
                subscribe "evt" as on_evt of type Ping;
            }}
            fn on_evt(p: Ping) {{
                println("publisher local got n=", p.n);
            }}
        }}

        locus Pub {{
            bus {{
                publish "evt" of type Ping;
            }}
            birth() {{
                "evt" <- Ping {{ n: {} }};
            }}
        }}

        fn main() {{
            Sub {{ }};
            Pub {{ }};
        }}
    "#,
        sentinel,
    );

    let sub_bin = build_binary(subscriber_src, "sub");
    let pub_bin = build_binary(&publisher_src, "pub");

    let sock = unique_path("sock", "sock");
    let sub_cfg = unique_path("subcfg", "conf");
    let pub_cfg = unique_path("pubcfg", "conf");
    std::fs::write(
        &sub_cfg,
        format!("evt = unix://{} : listen\n", sock.display()),
    )
    .expect("write sub cfg");
    std::fs::write(
        &pub_cfg,
        format!("evt = unix://{} : connect\n", sock.display()),
    )
    .expect("write pub cfg");

    // Spawn the subscriber first so its reader thread gets to
    // bind/listen before the publisher's connect-with-retry
    // starts. The connect side retries on ENOENT for ~1s, but
    // listener-first is the natural order and keeps the test
    // deterministic.
    let subscriber = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");

    // Brief delay so the subscriber's reader thread has a chance
    // to call lotus_transport_create(LISTEN) -> bind/listen
    // before the publisher tries to connect. Not strictly
    // required (connect retries) but reduces stderr noise from
    // ENOENT-and-backoff messages.
    std::thread::sleep(Duration::from_millis(50));

    let pub_out = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run publisher");

    let sub_out = subscriber.wait_with_output().expect("subscriber wait");

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&sub_cfg);
    let _ = std::fs::remove_file(&pub_cfg);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);

    assert!(
        pub_out.status.success(),
        "publisher exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        pub_out.status,
        String::from_utf8_lossy(&pub_out.stdout),
        String::from_utf8_lossy(&pub_out.stderr),
    );
    assert!(
        sub_out.status.success(),
        "subscriber exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        sub_out.status,
        String::from_utf8_lossy(&sub_out.stdout),
        String::from_utf8_lossy(&sub_out.stderr),
    );

    let sub_stdout = String::from_utf8_lossy(&sub_out.stdout);
    let expected_line = format!("subscriber got n={}", sentinel);
    assert!(
        sub_stdout.contains(&expected_line),
        "subscriber stdout should contain '{}'; got: {:?}\n\
         publisher stderr: {}",
        expected_line,
        sub_stdout,
        String::from_utf8_lossy(&pub_out.stderr),
    );
}

#[test]
fn publisher_fans_out_to_two_connect_peers() {
    // m69: a publisher with TWO `subject = url : connect` lines
    // (same subject, distinct unix sockets) should fan out a
    // single publish to both peers. The existing
    // lotus_bus_remote_fanout iterates the remote-entries table
    // and dispatches to every CONNECT-role entry whose subject
    // matches; this test locks in that multi-peer behavior end-
    // to-end (was always-true substrate, never asserted before
    // m69).
    let sentinel: i64 = 0x1122_3344_5566_7788;

    let subscriber_src = r#"
        type Ping {
            n: Int;
        }

        locus Sub {
            bus {
                subscribe "evt" as on_evt of type Ping;
            }
            fn on_evt(p: Ping) {
                println("subscriber got n=", p.n);
            }
        }

        fn main() {
            Sub { };
            time::sleep(500ms);
            yield;
        }
    "#;

    let publisher_src = format!(
        r#"
        type Ping {{
            n: Int;
        }}

        locus Sub {{
            bus {{
                subscribe "evt" as on_evt of type Ping;
            }}
            fn on_evt(p: Ping) {{
                println("publisher local got n=", p.n);
            }}
        }}

        locus Pub {{
            bus {{
                publish "evt" of type Ping;
            }}
            birth() {{
                "evt" <- Ping {{ n: {} }};
            }}
        }}

        fn main() {{
            Sub {{ }};
            Pub {{ }};
        }}
    "#,
        sentinel,
    );

    let sub_bin = build_binary(subscriber_src, "fanout-sub");
    let pub_bin = build_binary(&publisher_src, "fanout-pub");

    let sock_a = unique_path("fanout-a", "sock");
    let sock_b = unique_path("fanout-b", "sock");
    let sub_a_cfg = unique_path("fanout-suba", "conf");
    let sub_b_cfg = unique_path("fanout-subb", "conf");
    let pub_cfg = unique_path("fanout-pub", "conf");
    std::fs::write(
        &sub_a_cfg,
        format!("evt = unix://{} : listen\n", sock_a.display()),
    )
    .expect("write sub A cfg");
    std::fs::write(
        &sub_b_cfg,
        format!("evt = unix://{} : listen\n", sock_b.display()),
    )
    .expect("write sub B cfg");
    std::fs::write(
        &pub_cfg,
        format!(
            "evt = unix://{} : connect\nevt = unix://{} : connect\n",
            sock_a.display(),
            sock_b.display(),
        ),
    )
    .expect("write pub cfg");

    let sub_a = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_a_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber A");
    let sub_b = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_b_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber B");

    std::thread::sleep(Duration::from_millis(50));

    let pub_out = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run publisher");

    let a_out = sub_a.wait_with_output().expect("subscriber A wait");
    let b_out = sub_b.wait_with_output().expect("subscriber B wait");

    let _ = std::fs::remove_file(&sock_a);
    let _ = std::fs::remove_file(&sock_b);
    let _ = std::fs::remove_file(&sub_a_cfg);
    let _ = std::fs::remove_file(&sub_b_cfg);
    let _ = std::fs::remove_file(&pub_cfg);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);

    assert!(
        pub_out.status.success(),
        "publisher exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        pub_out.status,
        String::from_utf8_lossy(&pub_out.stdout),
        String::from_utf8_lossy(&pub_out.stderr),
    );
    assert!(
        a_out.status.success() && b_out.status.success(),
        "subscriber non-zero: A={:?} B={:?}\nA stderr: {}\nB stderr: {}",
        a_out.status,
        b_out.status,
        String::from_utf8_lossy(&a_out.stderr),
        String::from_utf8_lossy(&b_out.stderr),
    );

    let expected = format!("subscriber got n={}", sentinel);
    let a_stdout = String::from_utf8_lossy(&a_out.stdout);
    let b_stdout = String::from_utf8_lossy(&b_out.stdout);
    assert!(
        a_stdout.contains(&expected),
        "subscriber A missing expected line '{}'; got: {:?}",
        expected,
        a_stdout,
    );
    assert!(
        b_stdout.contains(&expected),
        "subscriber B missing expected line '{}'; got: {:?}",
        expected,
        b_stdout,
    );
}

#[test]
fn cross_process_string_field_round_trips() {
    // m70: payload type with a String field round-trips
    // cross-process. Pre-m70 the m60 memcpy serializer copied
    // the String pointer verbatim, which segfaulted (or
    // garbage-printed) on the subscriber side because the
    // pointer was meaningless in the receiver's address space.
    // The m70 wire format length-prefixes Strings on the wire
    // and allocates fresh storage from the subscriber's lazy
    // global payload arena on deserialize, so the pointer in
    // the deserialized struct points to valid local memory.
    let subscriber_src = r#"
        type Msg {
            tag: String;
            n: Int;
        }

        locus Sub {
            bus {
                subscribe "evt" as on_evt of type Msg;
            }
            fn on_evt(m: Msg) {
                println("subscriber got tag=", m.tag, " n=", m.n);
            }
        }

        fn main() {
            Sub { };
            time::sleep(500ms);
            yield;
        }
    "#;

    let publisher_src = r#"
        type Msg {
            tag: String;
            n: Int;
        }

        // Local subscriber so the BusState gate is satisfied.
        locus Sub {
            bus {
                subscribe "evt" as on_evt of type Msg;
            }
            fn on_evt(m: Msg) {
                println("publisher local got tag=", m.tag, " n=", m.n);
            }
        }

        locus Pub {
            bus {
                publish "evt" of type Msg;
            }
            birth() {
                "evt" <- Msg { tag: "hello-world", n: 42 };
            }
        }

        fn main() {
            Sub { };
            Pub { };
        }
    "#;

    let sub_bin = build_binary(subscriber_src, "string-sub");
    let pub_bin = build_binary(publisher_src, "string-pub");

    let sock = unique_path("string-sock", "sock");
    let sub_cfg = unique_path("string-subcfg", "conf");
    let pub_cfg = unique_path("string-pubcfg", "conf");
    std::fs::write(
        &sub_cfg,
        format!("evt = unix://{} : listen\n", sock.display()),
    )
    .expect("write sub cfg");
    std::fs::write(
        &pub_cfg,
        format!("evt = unix://{} : connect\n", sock.display()),
    )
    .expect("write pub cfg");

    let subscriber = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");

    std::thread::sleep(Duration::from_millis(50));

    let pub_out = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run publisher");

    let sub_out = subscriber.wait_with_output().expect("subscriber wait");

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&sub_cfg);
    let _ = std::fs::remove_file(&pub_cfg);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);

    assert!(
        pub_out.status.success(),
        "publisher exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        pub_out.status,
        String::from_utf8_lossy(&pub_out.stdout),
        String::from_utf8_lossy(&pub_out.stderr),
    );
    assert!(
        sub_out.status.success(),
        "subscriber exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        sub_out.status,
        String::from_utf8_lossy(&sub_out.stdout),
        String::from_utf8_lossy(&sub_out.stderr),
    );

    let sub_stdout = String::from_utf8_lossy(&sub_out.stdout);
    assert!(
        sub_stdout.contains("subscriber got tag=hello-world n=42"),
        "subscriber missing expected line; got: {:?}\npublisher stderr: {}",
        sub_stdout,
        String::from_utf8_lossy(&pub_out.stderr),
    );
}
