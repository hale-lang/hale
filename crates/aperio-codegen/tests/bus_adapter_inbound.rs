//! m105 (Wave B inbound) — `std::bus::__local_dispatch` primitive
//! that lets an adapter locus deliver wire-bytes payloads into
//! local subscribers.
//!
//! Outbound (`adapter.send(subject, bytes)`) shipped in Wave B
//! proper; m105 closes the loop so adapters can implement both
//! halves. The primitive looks up the registered deserialize fn
//! by subject, reconstructs the struct-layout bytes, and fans into
//! the local handler set via lotus_bus_local_dispatch (same shape
//! the unix reader thread uses).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_adapter_inbound_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn loopback_adapter_relays_payload_to_local_subscriber() {
    // A "loopback" adapter immediately calls __local_dispatch with
    // the bytes it received from the outbound fanout. The local
    // subscriber sees the payload twice: once from the original
    // local-publish path, and once relayed through the adapter.
    // The relay arm is the m105 surface under test.
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        locus Loopback {
            fn send(subject: String, bytes: Bytes) {
                println("adapter saw subject=" + subject);
                std::bus::__local_dispatch(subject, bytes);
            }
        }

        locus Receiver {
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: Tick) {
                println("rcv n=" + t.n);
            }
        }

        locus Producer {
            bus { publish Beat; }
            birth() {
                Beat <- Tick { n: 7 };
                Beat <- Tick { n: 42 };
            }
        }

        main locus App {
            bindings {
                Beat: Loopback { };
            }
        }

        fn main() {
            App { };
            Receiver { };
            Producer { };
        }
    "#;
    let (stdout, status) = build_and_run("loopback", src);
    assert!(status.success(), "non-zero: {:?}", status);
    // The adapter must have seen both sends.
    let adapter_calls = stdout
        .lines()
        .filter(|l| l.contains("adapter saw subject=beat"))
        .count();
    assert_eq!(
        adapter_calls, 2,
        "adapter should see 2 outbound payloads; got stdout: {:?}",
        stdout
    );
    // The receiver sees each payload TWICE: once via the
    // local-publish path, once relayed through the adapter via
    // __local_dispatch. The relayed copies prove m105 works
    // end-to-end (deserialize lookup + dispatch).
    let n7 = stdout.lines().filter(|l| l.contains("rcv n=7")).count();
    let n42 = stdout.lines().filter(|l| l.contains("rcv n=42")).count();
    assert_eq!(
        n7, 2,
        "n=7 should arrive twice (local + adapter-relay); got: {:?}",
        stdout
    );
    assert_eq!(
        n42, 2,
        "n=42 should arrive twice (local + adapter-relay); got: {:?}",
        stdout
    );
}

#[test]
fn payload_preserved_through_serialize_dispatch_roundtrip() {
    // Stronger assertion: the wire-bytes round-trip preserves the
    // payload's Int field value. (If deserialize were wrong, the
    // relayed payload would arrive with a garbage n value while the
    // local-publish path delivered the correct value — they'd
    // disagree.) Use a distinctive value the loopback can echo.
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        locus Loopback {
            fn send(subject: String, bytes: Bytes) {
                std::bus::__local_dispatch(subject, bytes);
            }
        }

        locus Receiver {
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: Tick) {
                println("n=" + t.n);
            }
        }

        locus Producer {
            bus { publish Beat; }
            birth() {
                Beat <- Tick { n: 12345 };
            }
        }

        main locus App {
            bindings {
                Beat: Loopback { };
            }
        }

        fn main() {
            App { };
            Receiver { };
            Producer { };
        }
    "#;
    let (stdout, status) = build_and_run("roundtrip", src);
    assert!(status.success(), "non-zero: {:?}", status);
    let n12345 = stdout
        .lines()
        .filter(|l| l.contains("n=12345"))
        .count();
    assert_eq!(
        n12345, 2,
        "expected 2 copies of n=12345 (local + adapter-relay); got: {:?}",
        stdout
    );
    // No corrupted values.
    for l in stdout.lines() {
        if l.starts_with("n=") {
            assert_eq!(
                l, "n=12345",
                "unexpected/corrupted payload line: {:?}",
                stdout
            );
        }
    }
}
