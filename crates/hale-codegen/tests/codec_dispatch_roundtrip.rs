//! F.36 Slice 3b: end-to-end round-trip through the synthesized
//! encode + decode thunks. Verifies both halves fire by using an
//! XOR codec — the wire bytes are NOT the raw value, so an m70
//! fallthrough would also work (identity wire format) and mask
//! the bug. With XOR, the round-trip recovers the original value
//! ONLY if both encode and decode are invoked.
//!
//! No remote transport is needed: `lotus_bus_dispatch` always
//! routes through the wire path when `serialize_fn` is set
//! (which Slice 3b makes the encode thunk), and intra-process
//! subscribers receive via `lotus_bus_dispatch_wire` which calls
//! each subscriber's `deserialize_fn` (which Slice 3b makes the
//! decode thunk). The unix-binding role is `connect` only — its
//! transport is exercised by the wire path, not by an actual
//! socket peer.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

/// Compile transport_driver.c + lotus_arena.c into a peer binary
/// (same recipe as tests/transport.rs). GH #227 made an
/// unrealizable binding a birth failure, so the connect-role
/// binding below needs a real listener peer — the pre-#227
/// version of this test silently relied on the broker tolerating
/// a dead transport.
fn build_peer_driver(tag: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_codec_e2e_peer_{}", tag));
    let status = Command::new("clang")
        .arg(manifest.join("tests").join("transport_driver.c"))
        .arg(manifest.join("runtime").join("lotus_arena.c"))
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building peer driver");
    bin
}

#[test]
fn xor_codec_round_trip_through_in_process_wire_path() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let sock = format!(
        "{}/codec-e2e-{}-{}.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    );
    let src = format!(
        r#"
        type Msg {{ tag: Int = 0; }}
        type EncErr {{ kind: String = ""; }}
        type DecErr {{ kind: String = ""; }}

        topic MsgTopic {{ payload: Msg; subject: "codec.xor.msgs"; }}

        locus XorCodec {{
            fn encode(v: Msg) -> Bytes fallible(EncErr) {{
                let scrambled = v.tag ^ 170;
                return std::bytes::from_int(scrambled);
            }}
            fn decode(b: Bytes) -> Msg fallible(DecErr) {{
                let raw = std::bytes::at(b, 0)
                    or fail DecErr {{ kind: "oob" }};
                let unxor = raw ^ 170;
                return Msg {{ tag: unxor }};
            }}
        }}

        main locus App {{
            bus {{
                publish   MsgTopic;
                subscribe MsgTopic as on_msg;
            }}
            bindings {{
                MsgTopic: unix("{}", role: connect)
                          codec(XorCodec {{ }});
            }}
            fn on_msg(m: Msg) {{
                println("[sub] tag=", m.tag);
            }}
            run() {{
                MsgTopic <- Msg {{ tag: 42 }};
                std::time::sleep(150ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_codec_dispatch_roundtrip");
    build_executable(&program, &bin).expect("build");
    // Listener peer first so the app's connect-with-retry lands.
    // The peer just absorbs the (scrambled) wire bytes; the
    // round-trip under test is the in-process wire path.
    let driver = build_peer_driver("xor");
    let listener = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn listener peer");
    let output = Command::new(&bin).output().expect("run");
    let _ = listener.wait_with_output();
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The original tag is 42. encode XORs with 170 → 0xAA wire byte
    // (1 byte). decode XORs back → 42. Both encode and decode MUST
    // have fired for the subscriber to receive tag=42 — an m70
    // fallthrough would deliver tag=42 too (identity wire format),
    // BUT the encode thunk's XOR would scramble the wire bytes and
    // the m70 deserializer would reconstruct a Msg with the
    // scrambled-then-truncated tag instead. Net: tag=42 in the
    // subscriber proves both thunks ran.
    assert!(
        stdout.contains("[sub] tag=42"),
        "codec round-trip didn't recover original tag. Stdout: {:?}",
        stdout
    );
}
