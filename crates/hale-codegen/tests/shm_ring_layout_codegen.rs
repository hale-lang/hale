//! Proposal B (2026-06-06) — codegen wiring for a `layout:`-bound
//! shm_ring subscriber.
//!
//! Compiles a Hale program with a `ring_layout` declaration and a
//! subscriber whose binding carries `layout: ForeignRing`, dumps the
//! LLVM IR via `LOTUS_DUMP_IR`, and asserts codegen took the
//! foreign-layout path: it emits the descriptor global and a call
//! to `lotus_bus_register_subscriber_shm_ring_layout` (NOT the
//! native `lotus_bus_register_subscriber_shm_ring`).
//!
//! The runtime `byte_records` reader the descriptor drives is
//! validated separately by the C driver in `shm_ring_layout.rs`;
//! this test guards the codegen-side marshalling so the two halves
//! stay connected.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-pr0b-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ));
    p
}

#[test]
fn layout_binding_emits_layout_register_call() {
    let src = r#"
        ring_layout ForeignRing {
            magic 0x52494E47464D5431;
            version 1 at 8 : u32;
            buffer_size at 12 : u32;
            data_at 128;
            cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
            framing byte_records { len_prefix u32; align 8; }
            overflow lap_detect;
        }

        type Tick {
            px: Int;
            sz: Int;
        }
        topic Ticks { payload: Tick; }

        locus Sub {
            bus { subscribe Ticks as on_tick of type Tick; }
            fn on_tick(t: Tick) {
                println("tick px=", t.px, " sz=", t.sz);
            }
        }

        main locus App {
            bindings {
                Ticks: shm_ring("/foreign.ticks", on_overflow: drop, layout: ForeignRing) where zero_copy;
            }
        }

        fn main() {
            App { };
            Sub { };
        }
    "#;

    let bin = unique_path("layout", "bin");
    let ir = bin.with_extension("ll");
    let program = hale_syntax::parse_source(src).expect("parse");

    std::env::set_var("LOTUS_DUMP_IR", "1");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");

    let ir_text = std::fs::read_to_string(&ir).expect("read IR");

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);

    assert!(
        ir_text.contains("lotus_bus_register_subscriber_shm_ring_layout"),
        "layout-bound subscriber must register via the layout-aware \
         runtime path; IR did not reference \
         lotus_bus_register_subscriber_shm_ring_layout"
    );
    assert!(
        ir_text.contains("lotus.shm_ring.layout.desc"),
        "codegen must emit the descriptor global for the layout \
         binding; IR did not contain lotus.shm_ring.layout.desc"
    );
}

#[test]
fn layout_publisher_emits_producer_register_and_publish() {
    // Proposal B M3a: a bundle that *publishes* a layout-bound topic
    // creates the foreign ring in the prelude
    // (lotus_bus_register_shm_ring_layout) and frames each `<-`
    // through lotus_bus_publish_shm_ring_layout — NOT the native
    // register/publish.
    let src = r#"
        ring_layout ForeignRing {
            magic 0x52494E47464D5431;
            version 1 at 8 : u32;
            buffer_size at 12 : u32;
            data_at 128;
            cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
            framing byte_records { len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF; }
            overflow lap_detect;
        }

        type Tick { px: Int; sz: Int; }
        topic Ticks { payload: Tick; }

        locus Producer {
            bus { publish Ticks; }
            birth() { Ticks <- Tick { px: 1, sz: 7 }; }
        }

        main locus App {
            bindings {
                Ticks: shm_ring("/foreign.ticks", on_overflow: drop,
                                layout: ForeignRing, buffer_size: 4096) where zero_copy;
            }
        }

        fn main() { App { }; Producer { }; }
    "#;

    let bin = unique_path("producer", "bin");
    let ir = bin.with_extension("ll");
    let program = hale_syntax::parse_source(src).expect("parse");

    std::env::set_var("LOTUS_DUMP_IR", "1");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");

    let ir_text = std::fs::read_to_string(&ir).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);

    assert!(
        ir_text.contains("lotus_bus_register_shm_ring_layout"),
        "a publishing layout binding must create the ring via the \
         producer register; IR missing lotus_bus_register_shm_ring_layout"
    );
    assert!(
        ir_text.contains("lotus_bus_publish_shm_ring_layout"),
        "a `<-` on a layout topic must route through the layout publish \
         path; IR missing lotus_bus_publish_shm_ring_layout"
    );
    // The native register/publish must NOT appear for this binding.
    assert!(
        !ir_text.contains("call void @lotus_bus_register_shm_ring("),
        "a layout binding must not emit the native LRSRNG1 register"
    );
}
