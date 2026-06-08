//! Proposal B M3a (2026-06-06) — end-to-end Hale producer over a
//! foreign `ring_layout`.
//!
//! A single Hale binary both produces and consumes its own foreign
//! ring, which avoids the cross-process attach-ordering race (a
//! broadcast consumer reads only records published after it
//! attaches, and cannot create the ring):
//!
//!   1. `App {}` birth → the prelude CREATES the ring (the bundle
//!      publishes `Ticks`, so it's the producer).
//!   2. `Sub {}` birth → attaches the ring read-only and spawns the
//!      byte_records reader thread.
//!   3. `Producer {}` birth → publishes N ticks via
//!      `lotus_bus_publish_shm_ring_layout`.
//!
//! The subscriber prints each received tick; we assert all N arrive
//! in order, proving the producer's framing is read back correctly
//! by the consumer (and, by construction, by any magus2-shaped
//! reader).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_tag(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("m3a-{}-{}-{}", label, std::process::id(), nanos)
}

#[test]
fn hale_producer_roundtrips_through_foreign_layout() {
    let shm_name = format!("/hale-{}", unique_tag("e2e"));
    let n_msgs: i64 = 6;

    let src = format!(
        r#"
        ring_layout MagusRing {{
            magic 0x4D475348514D4B54;
            version 1 at 8 : u32;
            buffer_size at 12 : u32;
            data_at 128;
            cursor published {{ at 64; repr atomic_u64; load acquire; unit bytes; }}
            framing byte_records {{ len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF; }}
            overflow lap_detect;
        }}

        type Tick {{ px: Int; sz: Int; }}
        topic Ticks {{ payload: Tick; }}

        locus Sub {{
            bus {{ subscribe Ticks as on_tick; }}
            fn on_tick(t: Tick) {{
                println("tick px=", t.px, " sz=", t.sz);
            }}
        }}

        locus Producer {{
            bus {{ publish Ticks; }}
            birth() {{
                Ticks <- Tick {{ px: 1, sz: 7 }};
                Ticks <- Tick {{ px: 2, sz: 14 }};
                Ticks <- Tick {{ px: 3, sz: 21 }};
                Ticks <- Tick {{ px: 4, sz: 28 }};
                Ticks <- Tick {{ px: 5, sz: 35 }};
                Ticks <- Tick {{ px: 6, sz: 42 }};
            }}
        }}

        main locus App {{
            bindings {{
                Ticks: shm_ring("{shm_name}", on_overflow: drop,
                                layout: MagusRing, buffer_size: 4096) where zero_copy;
            }}
        }}

        fn main() {{
            App {{ }};
            Sub {{ }};
            // Let the reader thread do its first poll (initializing its
            // byte cursor to the ring's current 0) BEFORE the producer
            // publishes — the byte_records reader starts at the live
            // cursor (no historical replay), so an in-process producer
            // that ran first would advance the cursor past records the
            // just-attached reader never saw. A real external producer
            // (magus2) is long-running, so this is a test-only barrier.
            time::sleep(50ms);
            Producer {{ }};
            time::sleep(500ms);
        }}
    "#,
        shm_name = shm_name,
    );

    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_{}.bin", unique_tag("bin")));
    build_executable(&program, &bin).expect("build");

    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "producer/consumer binary failed: status={:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for i in 1..=n_msgs {
        let want = format!("tick px={} sz={}", i, i * 7);
        assert!(
            stdout.contains(&want),
            "subscriber missing `{}`. Full stdout:\n{}",
            want,
            stdout
        );
    }

    // The producer owns + unlinks the ring at exit.
    let stripped = shm_name.trim_start_matches('/');
    assert!(
        !PathBuf::from(format!("/dev/shm/{}", stripped)).exists(),
        "atexit cleanup failed: `{}` persists in /dev/shm/",
        shm_name
    );
}
