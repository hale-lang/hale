//! GH #1174 round 2, item 1: a codec-bound subject's encode thunk
//! used to return -1 (the generic serialize-failure code) when the
//! encoded `Bytes` crossed the caller's cap — and `lotus_bus_dispatch`
//! treats a <= 0 serialize result as "drop the publish", LOCAL
//! delivery included. spec/semantics.md and the book both say local
//! delivery is bounded only by the payload arena; a codec subject
//! silently violated that past 64 KiB.
//!
//! The fix makes the thunk match `lotus_serialize_fn`'s contract:
//! write nothing past cap, answer the size the payload needs anyway.
//! `lotus_bus_wire_encode` already re-invokes with a bigger cap when
//! the first call's answer is over what it passed in, so the codec
//! path now gets the same malloc-and-retry the plain field serializer
//! gets. No remote transport is exercised — the binding's role is
//! `connect` only (a listener peer just absorbs the wire bytes); the
//! round-trip under test is the in-process LOCAL delivery.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_peer_driver(tag: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = harness::unique_bin(&format!("hale_codec_oversize_peer_{}", tag));
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
fn codec_encode_over_cap_still_delivers_locally_whole() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let sock = format!(
        "{}/codec-oversize-{}-{}.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    );
    let src = format!(
        r#"
        type Msg {{ text: String = ""; }}
        type EncErr {{ kind: String = ""; }}
        type DecErr {{ kind: String = ""; }}

        topic MsgTopic {{ payload: Msg; subject: "codec.oversize.msgs"; }}

        locus PassthroughCodec {{
            fn encode(v: Msg) -> Bytes fallible(EncErr) {{
                return std::bytes::from_string(v.text);
            }}
            fn decode(b: Bytes) -> Msg fallible(DecErr) {{
                return Msg {{ text: std::str::from_bytes(b) }};
            }}
        }}

        main locus App {{
            bus {{
                publish   MsgTopic;
                subscribe MsgTopic as on_msg;
            }}
            bindings {{
                MsgTopic: unix("{}", role: connect)
                          codec(PassthroughCodec {{ }});
            }}
            fn on_msg(m: Msg) {{
                println("[sub] len=", len(m.text));
            }}
            run() {{
                let mut t = "0123456789abcdef";
                let mut i = 0;
                while i < 16 {{
                    t = t + t;
                    i = i + 1;
                }}
                MsgTopic <- Msg {{ text: t }};
                std::time::sleep(150ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin("hale_test_codec_encode_oversize");
    build_executable(&program, &bin).expect("build");
    let driver = build_peer_driver("oversize");
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
    // "0123456789abcdef" doubled 16 times is 2^16 * 16 = 1048576 bytes.
    assert!(
        stdout.contains("[sub] len=1048576"),
        "the 1 MiB codec payload must reach the local subscriber whole \
         (before the fix, the encode thunk returning -1 past cap made \
         lotus_bus_dispatch drop the publish outright). Stdout: {:?}",
        stdout
    );
}
