//! In-band record-header field delivery (#5 follow-on, fast-protocol-I/O).
//!
//! The `record_header` framing can declare where the per-record header
//! scalars live (`seq_offset/width`, `kernel_ns_offset/width`,
//! `user_ns_offset/width`); the byte_records reader decodes them per record
//! into thread-locals the subscribe handler reads via
//! `std::shm::last_record_{seq, kernel_ns, user_ns}()` — the errno-style
//! idiom of `recv_stamped`'s `last_recv_*_ns`. The payload is still the
//! BytesView; this surfaces the in-band sequence number + kernel/user
//! timestamps a market-data consumer wants.
//!
//! End-to-end: the C producer (`produce_record_header`) writes ws-fast-shaped
//! records with seq@8 / kernel_ns@16 / user_ns@24, and a Hale BytesView
//! subscriber reads both the payload and the three header fields per record.

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
    format!("rhf-{}-{}-{}", label, std::process::id(), nanos)
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
fn hale_subscriber_reads_in_band_header_fields() {
    let shm_name = format!("/hale-{}", unique_tag("e2e"));

    let src = format!(
        r#"
        ring_layout WsFastish {{
            magic 0x52494E47464D5431;
            version 1 at 8 : u32;
            buffer_size at 12 : u32;
            data_at 128;
            cursor published {{ at 64; repr atomic_u64; load acquire; unit bytes; }}
            framing byte_records {{
                len_prefix u32;
                align 8;
                record_header_bytes 32;
                pad_field_offset 4; pad_field_width 1; pad_field_value 1;
                seq_offset 8; seq_width 8;
                kernel_ns_offset 16; kernel_ns_width 8;
                user_ns_offset 24; user_ns_width 8;
            }}
            overflow lap_detect;
        }}

        topic Recs {{ payload: BytesView; }}

        locus Sub {{
            bus {{ subscribe Recs as on_rec; }}
            fn on_rec(v: BytesView) {{
                let val = std::bytes::read_i64_le(v, 0) or -1;
                let seq = std::shm::last_record_seq();
                let kns = std::shm::last_record_kernel_ns();
                let uns = std::shm::last_record_user_ns();
                println("rec val=", to_string(val), " seq=", to_string(seq),
                        " kns=", to_string(kns), " uns=", to_string(uns));
            }}
        }}

        main locus App {{
            bindings {{
                Recs: shm_ring("{shm_name}", on_overflow: drop, layout: WsFastish);
            }}
        }}

        fn main() {{
            App {{ }};
            Sub {{ }};
            time::sleep(1500ms);
        }}
    "#,
        shm_name = shm_name,
    );

    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut consumer_bin = std::env::temp_dir();
    consumer_bin.push(format!("lotus_{}.bin", unique_tag("consumer")));
    build_executable(&program, &consumer_bin).expect("build consumer");

    let producer_bin = build_producer();
    let mut producer = Command::new(&producer_bin)
        .arg("produce_record_header")
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

    assert!(producer_status.success(), "producer failed: {:?}", producer_status);
    assert!(
        consumer_out.status.success(),
        "consumer failed: status={:?}\nstderr: {}",
        consumer_out.status,
        String::from_utf8_lossy(&consumer_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&consumer_out.stdout);

    // Record i (1-based): val=(i)*7, seq=i, kns=i*1000, uns=i*1000+7.
    for i in 1..=40i64 {
        let want = format!(
            "rec val={} seq={} kns={} uns={}",
            i * 7, i, i * 1000, i * 1000 + 7
        );
        assert!(stdout.contains(&want), "missing `{}`. stdout:\n{}", want, stdout);
    }
}
