//! m58: deployment-config subject binding integration test.
//!
//! Builds a tiny lotus publisher that publishes a struct payload
//! on subject "evt", runs it under `LOTUS_BUS_CONFIG=<file>` where
//! the file routes "evt" to a unix socket in connect role, and
//! asserts the m57 transport_driver running in listen role
//! receives the sentinel bytes intact.
//!
//! This is the publisher-side proof of cross-process bus dispatch:
//! the configured transport opens at boot, `<- "evt" | Ping{n: ...}`
//! fans out to local subscribers AND through the transport, and
//! the receiver picks up exactly one SOCK_SEQPACKET message. The
//! receiver-side reader-thread (for a lotus binary in listen role)
//! is m59+; here the listener role is satisfied by the m57 driver
//! so we can verify the publisher pipeline end-to-end without
//! yet wiring receive-side dispatch.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn runtime_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("runtime");
    p.push("lotus_arena.c");
    p
}

fn driver_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("tests");
    p.push("transport_driver.c");
    p
}

/// Compile transport_driver.c + lotus_arena.c into a one-off
/// listener binary in $TMPDIR. Same recipe as tests/transport.rs.
fn build_listener_driver(tag: &str) -> PathBuf {
    let bin = harness::unique_bin(&format!("lotus_m58_listener_{}", tag));
    let status = Command::new("clang")
        .arg(driver_c_path())
        .arg(runtime_c_path())
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building listener driver");
    bin
}

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-m58-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ));
    p
}

#[test]
fn deployment_config_routes_publisher_to_remote_listener() {
    // Sentinel chosen so each byte is unique + ASCII-printable —
    // shows up as "HGFEDCBA" when stdout is dumped on test
    // failure. Little-endian i64.
    let sentinel: i64 = 0x4142_4344_4546_4748;

    let src = format!(
        r#"
        type Ping {{
            n: Int;
        }}

        locus Sub {{
            bus {{
                subscribe "evt" as on_evt of type Ping;
            }}
            fn on_evt(p: Ping) {{
                println("local sub got n=", p.n);
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
            // Sub registers first so its local subscription is in
            // place when Pub.birth() publishes; the cross-process
            // transport receives the same publish via m58 fanout.
            Sub {{ }};
            Pub {{ }};
        }}
    "#,
        sentinel,
    );

    let driver = build_listener_driver("rt");
    let sock = unique_path("rt", "sock");
    let cfg = unique_path("rt", "conf");
    let cfg_body = format!(
        "# m58 integration test\n\
         evt = unix://{} : connect\n",
        sock.display(),
    );
    std::fs::write(&cfg, &cfg_body).expect("write config");

    // Spawn the listener first. m57's transport_create blocks on
    // accept(); the publisher's lotus_bus_load_config blocks on
    // connect-with-retry. Either order works (connect retries on
    // ENOENT for ~1s) but listener-first is the natural flow.
    let listener = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let bin = harness::unique_bin("m58_publisher");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build publisher");

    let pub_out = Command::new(&bin)
        .env("LOTUS_BUS_CONFIG", &cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run publisher");

    let listen_out = listener.wait_with_output().expect("listener wait");

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&cfg);
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);

    assert!(
        pub_out.status.success(),
        "publisher exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        pub_out.status,
        String::from_utf8_lossy(&pub_out.stdout),
        String::from_utf8_lossy(&pub_out.stderr),
    );
    assert!(
        listen_out.status.success(),
        "listener exited non-zero: {:?}\nstderr: {}",
        listen_out.status,
        String::from_utf8_lossy(&listen_out.stderr),
    );
    assert!(
        listen_out.stdout.len() >= 8,
        "listener received {} bytes (need at least 8 for an i64): \
         {:?}\npublisher stderr: {}",
        listen_out.stdout.len(),
        listen_out.stdout,
        String::from_utf8_lossy(&pub_out.stderr),
    );
    let mut sentinel_bytes = [0u8; 8];
    sentinel_bytes.copy_from_slice(&listen_out.stdout[..8]);
    let received = i64::from_le_bytes(sentinel_bytes);
    assert_eq!(
        received, sentinel,
        "listener should receive the sentinel int published by the \
         lotus binary; got bytes {:?}",
        listen_out.stdout,
    );
    // Local subscriber's println should also have fired in the
    // publisher process — confirms m58 fanout routes to BOTH
    // local and remote in a single dispatch.
    assert!(
        String::from_utf8_lossy(&pub_out.stdout)
            .contains(&format!("local sub got n={}", sentinel)),
        "publisher's local subscriber should also have received the \
         publish; stdout was: {:?}",
        String::from_utf8_lossy(&pub_out.stdout),
    );
}

#[test]
fn no_config_set_behaves_as_pre_m58() {
    // A binary with bus subscribe/publish but no LOTUS_BUS_CONFIG
    // set should behave identically to a pre-m58 single-process
    // bus program: local dispatch fires, no transport opens, no
    // hang. This is the cheap regression check that the codegen
    // prelude addition didn't break the no-config path.
    let src = r#"
        type Ping {
            n: Int;
        }

        locus Sub {
            bus {
                subscribe "evt" as on_evt of type Ping;
            }
            fn on_evt(p: Ping) {
                println("got n=", p.n);
            }
        }

        locus Pub {
            bus {
                publish "evt" of type Ping;
            }
            birth() {
                "evt" <- Ping { n: 7 };
            }
        }

        fn main() {
            Sub { };
            Pub { };
        }
    "#;
    let bin = harness::unique_bin("m58_publisher");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");

    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "no-config binary exited non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("got n=7"),
        "local dispatch should still work without LOTUS_BUS_CONFIG; \
         stdout was: {:?}",
        stdout,
    );
}

/// A `LOTUS_BUS_CONFIG` binding is served by no locus, so nothing marks
/// it lost when its peer goes away (review of #1869, finding 7): #1869
/// made a peer hang-up a DEBUG record on every send path, which left a
/// dead `unix://` peer of a configured binding visible only in a counter
/// unless `HALE_LOG=debug`. The first lost send on such a binding is one
/// WARN record, written regardless of `HALE_LOG`; the later ones are
/// DEBUG, written only under `HALE_LOG=debug`; any other errno stays
/// ERROR. A binding served by a locus keeps the DEBUG record and the
/// lost marking (`http_peer_hangup.rs` for the TCP side).
///
/// The listener (`transport_driver.c`, `listen`) takes one message and
/// exits, so the publisher's second and third sends go to a peer that
/// has hung up.
#[test]
fn a_dead_config_peer_is_warned_once_then_debug() {
    let src = r#"
        type Ping {
            n: Int;
        }

        locus Pub {
            bus {
                publish "evt" of type Ping;
            }
            birth() {
                "evt" <- Ping { n: 1 };
                std::time::sleep(300ms);
                "evt" <- Ping { n: 2 };
                "evt" <- Ping { n: 3 };
                println("published");
            }
        }

        fn main() {
            Pub { };
        }
    "#;
    let driver = build_listener_driver("dead");
    let bin = harness::unique_bin("m58_dead_peer");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build publisher");

    // one run: the listener, the publisher under the config (and `env`),
    // and what the publisher wrote on stderr
    let run = |env: &[(&str, &str)], tag: &str| -> String {
        let sock = unique_path(tag, "sock");
        let cfg = unique_path(tag, "conf");
        std::fs::write(&cfg, format!("evt = unix://{} : connect\n", sock.display())).expect("write config");
        let listener = Command::new(&driver)
            .arg("listen")
            .arg(&sock)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn listener");
        let mut cmd = Command::new(&bin);
        cmd.env("LOTUS_BUS_CONFIG", &cfg).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run publisher");
        let listen_out = listener.wait_with_output().expect("listener wait");
        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_file(&cfg);
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains("published"),
            "the publisher did not run to its end: {:?}\nstdout: {}\nstderr: {stderr}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
        );
        assert!(listen_out.status.success(), "the listener took no message:\n{}", String::from_utf8_lossy(&listen_out.stderr));
        stderr
    };

    // the default level: one WARN for the first lost send, nothing for
    // the second, and no ERROR
    let err = run(&[], "warn");
    let sends: Vec<&str> = err.lines().filter(|l| l.contains("send failed")).collect();
    assert_eq!(sends.len(), 1, "one WARN record for the first lost send and nothing for the second:\n{err}");
    assert!(sends[0].contains("level=warn") && sends[0].contains("logger=lotus.bus"), "{}", sends[0]);
    let epipe = format!("errno={} ", libc::EPIPE);
    let reset = format!("errno={} ", libc::ECONNRESET);
    assert!(sends[0].contains(&epipe) || sends[0].contains(&reset), "EPIPE or ECONNRESET: {}", sends[0]);
    assert!(!err.contains("level=error"), "an ERROR record for a peer that hung up:\n{err}");

    // under HALE_LOG=debug the second lost send is a DEBUG record
    let err = run(&[("HALE_LOG", "debug")], "debug");
    let sends: Vec<&str> = err.lines().filter(|l| l.contains("send failed")).collect();
    assert_eq!(sends.len(), 2, "the WARN and then a DEBUG record:\n{err}");
    assert!(sends[0].contains("level=warn"), "{}", sends[0]);
    assert!(sends[1].contains("level=debug"), "{}", sends[1]);

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);
}
