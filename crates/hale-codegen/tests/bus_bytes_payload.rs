//! A8 (G15) — Bytes in bus payload structs.
//!
//! The v1 codegen wire-format walker accepts a Bytes field inside
//! a bus payload struct (and inside any nested struct under it).
//! The wire shape is the same as Bytes' in-memory layout —
//! `[i64 len][N bytes]` — so embedded NULs survive round-trip
//! (where String would truncate at the first NUL).

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_bus_bytes_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn bus_payload_top_level_struct_with_bytes_field_round_trips() {
    // The classic shape: a payload struct with a String label and
    // a Bytes blob. Publisher creates the Bytes inline; subscriber
    // reads len(blob).
    let src = r#"
        type Frame {
            label: String;
            blob: Bytes;
        }
        topic Frames { payload: Frame; }
        locus Sink {
            params { received: Int = 0; }
            bus { subscribe Frames as on_frame; }
            fn on_frame(f: Frame) {
                self.received = self.received + 1;
                println("label=" + f.label + " size=" + len(f.blob));
            }
        }
        locus Source {
            bus { publish Frames; }
            run() {
                Frames <- Frame { label: "alpha", blob: std::bytes::from_string("hello") };
                Frames <- Frame { label: "beta",  blob: std::bytes::from_string("hi there") };
            }
        }
        fn main() {
            let s = Sink { };
            Source { };
        }
    "#;
    let (stdout, status) = build_and_run("top_level", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("label=alpha size=5"),
        "missing frame 1: {:?}",
        stdout
    );
    assert!(
        stdout.contains("label=beta size=8"),
        "missing frame 2: {:?}",
        stdout
    );
}

#[test]
fn bus_payload_nested_struct_with_bytes_field_round_trips() {
    // Nested case — Bytes lives inside an inner struct that's a
    // field of the top-level payload. Exercises the recursive
    // emit_per_field_deserialize_size path.
    let src = r#"
        type Body {
            blob: Bytes;
        }
        type Envelope {
            tag: String;
            body: Body;
        }
        topic Envelopes { payload: Envelope; }
        locus Reader {
            bus { subscribe Envelopes as on_env; }
            fn on_env(e: Envelope) {
                println("tag=" + e.tag + " size=" + len(e.body.blob));
            }
        }
        locus Writer {
            bus { publish Envelopes; }
            run() {
                Envelopes <- Envelope {
                    tag: "boxed",
                    body: Body { blob: std::bytes::from_string("payload") }
                };
            }
        }
        fn main() {
            let r = Reader { };
            Writer { };
        }
    "#;
    let (stdout, status) = build_and_run("nested", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("tag=boxed size=7"),
        "missing nested frame: {:?}",
        stdout
    );
}
