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

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn codec_locus_is_instantiated_at_main_prelude() {
    let src = r#"
        type Tick { sym: String = ""; price: Int = 0; }
        type EncErr { kind: String = ""; }
        type DecErr { kind: String = ""; }

        topic TickTopic { payload: Tick; subject: "ticks"; }

        locus TickJsonCodec {
            birth() {
                println("[codec] birth");
            }
            fn encode(v: Tick) -> Bytes fallible(EncErr) {
                return std::bytes::from_string(v.sym);
            }
            fn decode(b: Bytes) -> Tick fallible(DecErr) {
                return Tick { sym: "x", price: 0 };
            }
        }

        main locus App {
            bus { publish TickTopic; }
            bindings {
                TickTopic: unix("/tmp/codec_inst_smoke.sock", role: connect)
                           codec(TickJsonCodec { });
            }
            run() {
                println("[app] running");
            }
        }
        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_codec_instantiation");
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
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
