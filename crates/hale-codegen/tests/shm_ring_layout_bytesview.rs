//! Raw-frame (BytesView) foreign-ring consumer (2026-06-08).
//!
//! A `shm_ring(layout:)` topic with a `BytesView` payload reads a
//! heterogeneous / variable-length ring: the runtime hands the handler
//! a bounded BytesView over each record (no fixed value_size, no resync
//! on a differently-sized valid record), and the handler decodes with
//! `std::bytes::read_*` + a discriminator.
//!
//! End-to-end: a C producer (`shm_ring_layout_driver.c produce_external`)
//! writes records of two different sizes, tagged by an i64 `kind` at
//! payload offset 0; a single Hale `BytesView` subscriber decodes both.
//! The producer creates the ring + waits for the consumer to attach
//! before publishing (the reader starts at the live cursor — no replay).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/shm_lockstep.rs"]
mod shm_lockstep;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn unique_tag(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("bv-{}-{}-{}", label, std::process::id(), nanos)
}

fn build_producer() -> PathBuf {
    let mut driver_c = manifest_dir();
    driver_c.push("tests");
    driver_c.push("shm_ring_layout_driver.c");
    let mut ring_c = manifest_dir();
    ring_c.push("runtime");
    ring_c.push("lotus_shm_ring.c");
    let bin = harness::unique_bin(&format!("lotus_{}", unique_tag("producer")));
    let status = Command::new("clang")
        .arg(&driver_c)
        .arg(&ring_c)
        .arg("-O2")
        .arg("-lrt")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang");
    assert!(status.success(), "clang failed building producer driver");
    bin
}

#[test]
fn hale_bytesview_subscriber_decodes_heterogeneous_ring() {
    let shm_name = format!("/hale-{}", unique_tag("e2e"));

    // Hale consumer: one BytesView subscriber decoding both record
    // shapes via std::bytes::read_* + a `kind` discriminator.
    let src = format!(
        r#"
        ring_layout ForeignRing {{
            magic 0x52494E47464D5431;
            version 1 at 8 : u32;
            buffer_size at 12 : u32;
            data_at 128;
            cursor published {{ at 64; repr atomic_u64; load acquire; unit bytes; }}
            framing byte_records {{ len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF; }}
            overflow lap_detect;
        }}

        topic Recs {{ payload: BytesView; }}

        locus Sub {{
            bus {{ subscribe Recs as on_rec; }}
            fn on_rec(v: BytesView) {{
                let kind = std::bytes::read_i64_le(v, 0) or -1;
                let a = std::bytes::read_i64_le(v, 8) or -1;
                if kind == 2 {{
                    let b = std::bytes::read_i64_le(v, 16) or -1;
                    println("rec kind=2 a=", to_string(a), " b=", to_string(b));
                }} else {{
                    println("rec kind=1 a=", to_string(a));
                }}
            }}
        }}

        main locus App {{
            bindings {{
                Recs: shm_ring("{shm_name}", on_overflow: drop, layout: ForeignRing);
            }}
        }}

        fn main() {{
            App {{ }};
            Sub {{ }};
            // subscribed: the reader starts at the ring's cursor now, and
            // the producer sends its first record only once it has read this
            println("ready");
            let _end = std::io::stdin::read_line();
        }}
    "#,
        shm_name = shm_name,
    );

    let consumer_bin = harness::unique_bin(&format!("lotus_{}.bin", unique_tag("consumer")));
    build_opts::build_source(&src, &consumer_bin, &build_opts::options()).expect("build consumer");

    let producer_bin = build_producer();

    // in lockstep with the producer: no clock (support/shm_lockstep.rs)
    let run = shm_lockstep::run(&producer_bin, "produce_external", &shm_name, &consumer_bin, 6);
    let _ = std::fs::remove_file(&producer_bin);
    let _ = std::fs::remove_file(&consumer_bin);

    assert!(run.producer.success(), "producer failed: {:?}", run.producer);
    assert!(
        run.consumer.success(),
        "consumer failed: status={:?}\nstderr: {}",
        run.consumer,
        run.stderr
    );
    let stdout = &run.stdout;

    // i: 0..6, kind = (i%2)+1, a = 100+i, b = 200+i (kind 2 only).
    // kind 1 at i = 0,2,4 → a = 100,102,104
    // kind 2 at i = 1,3,5 → a = 101,103,105 ; b = 201,203,205
    for a in [100, 102, 104] {
        let want = format!("rec kind=1 a={}", a);
        assert!(stdout.contains(&want), "missing `{}`. stdout:\n{}", want, stdout);
    }
    for (a, b) in [(101, 201), (103, 203), (105, 205)] {
        let want = format!("rec kind=2 a={} b={}", a, b);
        assert!(stdout.contains(&want), "missing `{}`. stdout:\n{}", want, stdout);
    }
}
