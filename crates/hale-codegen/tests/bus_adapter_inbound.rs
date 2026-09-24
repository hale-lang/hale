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

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_adapter_inbound_{}_{}",
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

/// GH #1040: the JSON codec both codec tests below bind. Wire bytes
/// are plain JSON, so a relayed or injected payload proves the codec
/// (not the m70 wire format) ran on each side.
const JSON_CODEC: &str = r#"
    type Msg { tag: Int = 0; who: String = ""; }
    type EncErr { kind: String = ""; }
    type DecErr { kind: String = ""; }

    locus JsonCodec {
        fn encode(v: Msg) -> Bytes fallible(EncErr) {
            return std::bytes::from_string("{\"tag\":" + to_string(v.tag) + ",\"who\":\"" + v.who + "\"}");
        }
        fn decode(b: Bytes) -> Msg fallible(DecErr) {
            let t = std::str::from_bytes(b);
            return Msg { tag: std::json::find_int_field(t, "tag"), who: std::json::find_string_field(t, "who") };
        }
    }
"#;

#[test]
fn codec_decodes_local_dispatch_from_a_pinned_thread() {
    // GH #1040: `__local_dispatch` from a pinned thread — the only
    // thread an adapter's receive loop has — segfaulted in the
    // codec's `decode`: the adapter binding never built the codec,
    // so the decode thunk ran with a null `self`. The subscriber
    // must hear the JSON the pinned child injected.
    let src = format!(
        "{JSON_CODEC}{}",
        r#"
        topic InTopic { payload: Msg; subject: "codec.json.in"; }

        locus Sink { fn send(subject: String, bytes: Bytes) { } }

        locus Rev {
            params { got: Int = 0; who: String = ""; }
            bus { subscribe InTopic as on_in; }
            fn on_in(m: Msg) {
                self.got = self.got + m.tag;
                self.who = m.who;
            }
        }

        locus Pump {
            run() {
                std::bus::__local_dispatch("codec.json.in", std::bytes::from_string("{\"tag\":7,\"who\":\"purpose\"}"));
            }
        }

        main locus App {
            params { r: Rev = Rev { }; p: Pump = Pump { }; }
            placement { p: pinned; }
            bindings { InTopic: Sink { } codec(JsonCodec { }); }
            run() {
                let i = 0;
                while self.r.got == 0 && i < 500 {
                    std::time::sleep(10ms);
                    i = i + 1;
                }
                println("got=" + to_string(self.r.got) + " who=" + self.r.who);
            }
        }

        fn main() { App { }; }
    "#
    );
    let (stdout, status) = build_and_run("codec_pinned_decode", &src);
    assert!(status.success(), "non-zero: {:?}; stdout: {:?}", status, stdout);
    assert!(
        stdout.lines().any(|l| l == "got=7 who=purpose"),
        "the pinned dispatch must decode through the codec; stdout: {:?}",
        stdout
    );
}

#[test]
fn codec_round_trips_through_a_loopback_adapter() {
    // GH #1040, both halves at once: publish → codec encode → the
    // adapter's `send` → `__local_dispatch` → codec decode → the
    // subscriber. The relayed copy carries the fields the codec's
    // JSON carried; the local-publish copy arrives as well.
    let src = format!(
        "{JSON_CODEC}{}",
        r#"
        topic Evt { payload: Msg; subject: "codec.json.evt"; }

        locus Loopback {
            fn send(subject: String, bytes: Bytes) {
                println("wire " + std::str::from_bytes(bytes));
                std::bus::__local_dispatch(subject, bytes);
            }
        }

        locus Receiver {
            bus { subscribe Evt as on_evt; }
            fn on_evt(m: Msg) { println("rcv tag=" + to_string(m.tag) + " who=" + m.who); }
        }

        locus Producer {
            bus { publish Evt; }
            birth() { Evt <- Msg { tag: 12345, who: "ana" }; }
        }

        main locus App {
            bindings { Evt: Loopback { } codec(JsonCodec { }); }
        }

        fn main() {
            App { };
            Receiver { };
            Producer { };
        }
    "#
    );
    let (stdout, status) = build_and_run("codec_roundtrip", &src);
    assert!(status.success(), "non-zero: {:?}; stdout: {:?}", status, stdout);
    assert!(
        stdout.lines().any(|l| l == r#"wire {"tag":12345,"who":"ana"}"#),
        "the adapter must carry the codec's JSON; stdout: {:?}",
        stdout
    );
    let rcv = stdout
        .lines()
        .filter(|l| *l == "rcv tag=12345 who=ana")
        .count();
    assert_eq!(
        rcv, 2,
        "local publish + codec-decoded relay; stdout: {:?}",
        stdout
    );
}
