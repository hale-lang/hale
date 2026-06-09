//! Dogfood, end-to-end (2026-06-08): a Hale program reads the native
//! LotusRing through the `ring_layout` abstraction.
//!
//! The native ring (`LRSRNG1`, a fixed-stride slot ring) is the shape the
//! runtime hardcodes for `shm_ring` bindings. This proves the same ring
//! is expressible as a `ring_layout LotusRing { ... framing slots ... }`
//! and readable by a `layout: LotusRing` consumer — the abstraction
//! covers our own format, not just foreign `byte_records` rings.
//!
//! A C native producer (`lotus_shm_ring_open` + claim/commit, the real
//! production write path) creates + writes the ring; a Hale subscriber
//! bound `layout: LotusRing` decodes the slots. The producer controls
//! timing (creates, waits for attach, publishes) so the test is
//! deterministic.

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
    format!("dog-{}-{}-{}", label, std::process::id(), nanos)
}

fn build_native_producer() -> PathBuf {
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
    assert!(status.success(), "clang failed building native producer driver");
    bin
}

#[test]
fn hale_layout_lotus_ring_reads_native_producer() {
    let shm_name = format!("/hale-{}", unique_tag("e2e"));

    // A `ring_layout LotusRing` describing the native LRSRNG1 header:
    // magic@0, slot_size@8, slot_count@16, seqno@24, slots@128. The
    // consumer binds `layout: LotusRing` and reads the typed payload.
    let src = format!(
        r#"
        type Tick {{ seq_id: Int; value: Int; }}

        ring_layout LotusRing {{
            magic 0x4C5253524E4731;
            slot_size  at 8  : u64;
            slot_count at 16 : u64;
            data_at 128;
            cursor published {{ at 24; repr atomic_u64; load acquire; unit slots; }}
            framing slots {{ }}
            overflow lap_detect;
        }}

        topic Ticks {{ payload: Tick; }}

        locus Sub {{
            bus {{ subscribe Ticks as on_tick; }}
            fn on_tick(t: Tick) {{
                println("tick seq=", to_string(t.seq_id), " val=", to_string(t.value));
            }}
        }}

        main locus App {{
            bindings {{
                Ticks: shm_ring("{shm_name}", on_overflow: drop, layout: LotusRing)
                    where zero_copy;
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

    let producer_bin = build_native_producer();

    // Producer creates the native ring + waits ~250ms before publishing;
    // spawn it first so the ring exists, then the consumer attaches.
    let mut producer = Command::new(&producer_bin)
        .arg("produce_native_external")
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

    // seq 1..6, value = seq * 7.
    for seq in 1..=6 {
        let want = format!("tick seq={} val={}", seq, seq * 7);
        assert!(stdout.contains(&want), "missing `{}`. stdout:\n{}", want, stdout);
    }
}
