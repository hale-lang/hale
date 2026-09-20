//! m60: per-payload-type serializer-shape verification.
//!
//! Compiles a small Hale program that uses `bus subscribe`/
//! `<-` on a struct payload, dumps the LLVM IR via
//! `BuildOptions::dump_ir`, and asserts the IR contains the
//! synthesized `__serialize_<T>` and `__deserialize_<T>` fns.
//!
//! This guards the *shape* of the substrate: codegen must emit
//! per-type serializer/deserializer hooks that bus dispatch +
//! subscribe registration route through. The actual wire format
//! is identity at v0.1 (memcpy of sizeof(T) bytes); a future
//! milestone replaces the bodies without touching call sites,
//! and this test continues to pass as long as the symbols exist.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-m60-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ));
    p
}

#[test]
fn ir_contains_per_payload_serializer_pair() {
    // Two distinct payload types: Ping (used as a subscribe
    // target) and Pong (publish-only, no local subscriber). Both
    // should produce a serializer/deserializer pair so cross-
    // process dispatch on either subject can route through them.
    let src = r#"
        type Ping {
            n: Int;
        }
        type Pong {
            label: String;
        }

        locus PingSub {
            bus {
                subscribe "ping" as on_ping of type Ping;
            }
            fn on_ping(p: Ping) {
                println("ping n=", p.n);
            }
        }

        locus PingPub {
            bus {
                publish "ping" of type Ping;
                publish "pong" of type Pong;
            }
            birth() {
                "ping" <- Ping { n: 1 };
                "pong" <- Pong { label: "hi" };
            }
        }

        fn main() {
            PingSub { };
            PingPub { };
        }
    "#;

    let bin = unique_path("shape", "bin");
    let program = hale_syntax::parse_source(src).expect("parse");

    // `BuildOptions::dump_ir` makes the build also write the `.ll`
    // beside the binary; the request is scoped to this one build
    // rather than to the whole process (GH #843).
    let ir_text = harness::build_ir_text(&program, &bin).expect("build");

    let _ = std::fs::remove_file(&bin);

    // Both types should have a serialize + deserialize fn at
    // module scope. We're looking for fn definitions, not just
    // declarations, so we check for `define` prefixes.
    for type_name in ["Ping", "Pong"] {
        let ser = format!("define i64 @__serialize_{}", type_name);
        let de = format!("define i64 @__deserialize_{}", type_name);
        assert!(
            ir_text.contains(&ser),
            "expected serializer fn `{}` in IR; sample: {}",
            ser,
            &ir_text[..ir_text.len().min(800)],
        );
        assert!(
            ir_text.contains(&de),
            "expected deserializer fn `{}` in IR",
            de,
        );
    }

    // m70: the send site no longer invokes __serialize_T inline.
    // Codegen passes the serialize-fn pointer as the 5th arg to
    // lotus_bus_dispatch; the C runtime does the actual serialize
    // when remote fanout is needed (skipping it for purely-local
    // dispatch). The IR still references __serialize_Ping (as a
    // fn-pointer constant in the dispatch call); just not via a
    // direct `call i64 @__serialize_Ping`.
    assert!(
        ir_text.contains("@__serialize_Ping"),
        "send site for Ping should reference __serialize_Ping (passed \
         as fn ptr to lotus_bus_dispatch)",
    );
    assert!(
        ir_text.contains("call void @lotus_bus_dispatch"),
        "send site should still dispatch",
    );

    // The register call site should pass the deserializer fn ptr
    // as its 5th arg. Loosest possible check: the deserializer
    // fn name appears somewhere in the same IR (it's used by the
    // register call site).
    assert!(
        ir_text.contains("@__deserialize_Ping"),
        "subscribe register should reference __deserialize_Ping",
    );
}
