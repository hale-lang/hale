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
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

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
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_{}", unique_tag("producer")));
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
            time::sleep(1200ms);
        }}
    "#,
        shm_name = shm_name,
    );

    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut consumer_bin = std::env::temp_dir();
    consumer_bin.push(format!("lotus_{}.bin", unique_tag("consumer")));
    build_executable(&program, &consumer_bin).expect("build consumer");

    let producer_bin = build_producer();

    // Producer creates the ring + writes the header, then waits ~250ms
    // before publishing — spawn it first so the ring exists, then the
    // consumer attaches inside that window.
    let mut producer = Command::new(&producer_bin)
        .arg("produce_external")
        .arg(&shm_name)
        .spawn()
        .expect("spawn producer");

    std::thread::sleep(std::time::Duration::from_millis(80));

    let consumer_out = Command::new(&consumer_bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run consumer");

    let producer_status = producer.wait().expect("wait producer");
    let _ = std::fs::remove_file(&producer_bin);
    let _ = std::fs::remove_file(&consumer_bin);

    assert!(
        producer_status.success(),
        "producer failed: {:?}",
        producer_status
    );
    assert!(
        consumer_out.status.success(),
        "consumer failed: status={:?}\nstderr: {}",
        consumer_out.status,
        String::from_utf8_lossy(&consumer_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&consumer_out.stdout);

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
