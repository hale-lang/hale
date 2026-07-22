//! F.36 Slice 3a: codec locus instantiation + `lotus_bus_register_codec`
//! emission at the main prelude.
//!
//! At this slice the codec is constructed and its encode/decode
//! method ptrs are registered on the bus runtime's remote entry,
//! but the publish-side and receive-side dispatch paths don't yet
//! consult those ptrs — they fall through to m70. Slice 3b will
//! wire the actual dispatch (with synthesized thunks bridging the
//! user-method ABI to the runtime's expected `void* (*)(void*,
//! void*)` shape).
//!
//! Test verifies the construction is observable: a `birth()` body
//! on the codec locus runs at startup, printing a tag we can check.
//! Pre-Slice-3a, the codec body would be unused (no instantiation
//! site); a missing tag in stdout would catch the regression.

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
    bin.push(format!("hale_codec_inst_peer_{}", tag));
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
fn codec_locus_is_instantiated_at_main_prelude() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let sock = format!(
        "{}/codec-inst-{}-{}.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    );
    let src = format!(
        r#"
        type Tick {{ sym: String = ""; price: Int = 0; }}
        type EncErr {{ kind: String = ""; }}
        type DecErr {{ kind: String = ""; }}

        topic TickTopic {{ payload: Tick; subject: "ticks"; }}

        locus TickJsonCodec {{
            birth() {{
                println("[codec] birth");
            }}
            fn encode(v: Tick) -> Bytes fallible(EncErr) {{
                return std::bytes::from_string(v.sym);
            }}
            fn decode(b: Bytes) -> Tick fallible(DecErr) {{
                return Tick {{ sym: "x", price: 0 }};
            }}
        }}

        main locus App {{
            bus {{ publish TickTopic; }}
            bindings {{
                TickTopic: unix("{}", role: connect)
                           codec(TickJsonCodec {{ }});
            }}
            run() {{
                println("[app] running");
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_codec_instantiation");
    build_executable(&program, &bin).expect("build");
    // Listener peer first so the app's connect-with-retry lands.
    let driver = build_peer_driver("smoke");
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
    // The codec's birth ran iff the codec was instantiated at the
    // bindings prelude. (m90-routed alloc lives in the payload
    // arena; birth fires synchronously on the main thread before
    // App's run().)
    assert!(
        stdout.contains("[codec] birth"),
        "codec birth() didn't run — instantiation/register path \
         likely broke. Stdout: {:?}",
        stdout
    );
    assert!(
        stdout.contains("[app] running"),
        "App.run() didn't fire. Stdout: {:?}",
        stdout
    );
}
