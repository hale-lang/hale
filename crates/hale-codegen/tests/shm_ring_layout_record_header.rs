//! `record_header` framing + `post_copy` recheck (#5 of the
//! fast-protocol-I/O substrate plan).
//!
//! A foreign ring whose records have a fixed 32-byte header
//! (len@0:u32, kind@4:u8 — the reference crate's shape) before the payload: stride is
//! `record_header_bytes + align(len)`, the payload starts past the header,
//! and a `kind == 1` header field marks a tail pad (not a len sentinel).
//! Without `record_header_bytes` the reader would desync after one record.
//!
//! End-to-end: the C producer (`shm_ring_layout_driver.c
//! produce_record_header`) writes 40 fixed-header records through a small
//! ring (forcing repeated kind==1 tail pads at the wrap); a Hale BytesView
//! subscriber bound with `record_header_bytes`/`pad_field`/`recheck
//! post_copy` reads each i64 payload in order.

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
    format!("rh-{}-{}-{}", label, std::process::id(), nanos)
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
fn hale_subscriber_reads_record_header_ring_with_post_copy() {
    let shm_name = format!("/hale-{}", unique_tag("e2e"));

    let src = format!(
        r#"
        ring_layout FixedHdrRing {{
            magic 0x52494E47464D5431;
            version 1 at 8 : u32;
            buffer_size at 12 : u32;
            data_at 128;
            cursor published {{ at 64; repr atomic_u64; load acquire; unit bytes; }}
            framing byte_records {{
                len_prefix u32;
                align 8;
                record_header_bytes 32;
                pad_field_offset 4;
                pad_field_width 1;
                pad_field_value 1;
                recheck post_copy;
            }}
            overflow lap_detect;
        }}

        topic Recs {{ payload: BytesView; }}

        locus Sub {{
            bus {{ subscribe Recs as on_rec; }}
            fn on_rec(v: BytesView) {{
                let val = std::bytes::read_i64_le(v, 0) or -1;
                println("rec val=", to_string(val));
            }}
        }}

        main locus App {{
            bindings {{
                Recs: shm_ring("{shm_name}", on_overflow: drop, layout: FixedHdrRing);
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
    let run = shm_lockstep::run(&producer_bin, "produce_record_header", &shm_name, &consumer_bin, 40);
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

    // val = (i+1)*7 for i in 0..40 → 7, 14, ..., 280. If the stride or
    // payload offset were wrong the reader would desync after the first
    // record; if pad-skip were wrong it would stall at the first wrap.
    for i in 0..40 {
        let want = format!("rec val={}", (i + 1) * 7);
        assert!(stdout.contains(&want), "missing `{}`. stdout:\n{}", want, stdout);
    }
}
