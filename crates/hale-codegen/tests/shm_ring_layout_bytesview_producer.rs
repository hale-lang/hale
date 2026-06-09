//! BytesView producer over a foreign `ring_layout` (2026-06-08) — the
//! producer mirror of the BytesView consumer (#72).
//!
//! A Hale program publishes variable-length / heterogeneous records to a
//! `byte_records` ring by sending a `BytesView` value: `Recs <- b.view()`
//! frames `[len_prefix len][bytes]` where `len` is the value's actual
//! byte length (not a fixed struct size). Single binary (App creates the
//! ring, Sub attaches, Producer publishes) to avoid the cross-process
//! attach-ordering race; the Sub decodes each record with
//! `std::bytes::read_*`, proving the producer's variable-length framing
//! round-trips.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_tag(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("bvp-{}-{}-{}", label, std::process::id(), nanos)
}

#[test]
fn hale_bytesview_producer_frames_variable_length_records() {
    let shm_name = format!("/hale-{}", unique_tag("e2e"));

    // Producer sends two records of DIFFERENT sizes (a 16-byte kind-1 and
    // a 24-byte kind-2), tagged by an i64 discriminator at offset 0. The
    // BytesView consumer decodes both — only possible if the producer
    // framed each record at its own length.
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

        locus Producer {{
            bus {{ publish Recs; }}
            birth() {{
                let r1 = std::bytes::BytesBuilder {{ initial_cap: 64 }};
                r1.append_i64_le(1);     // kind
                r1.append_i64_le(100);   // a            -> 16-byte record
                Recs <- r1.view();
                let r2 = std::bytes::BytesBuilder {{ initial_cap: 64 }};
                r2.append_i64_le(2);     // kind
                r2.append_i64_le(200);   // a
                r2.append_i64_le(201);   // b            -> 24-byte record
                Recs <- r2.view();
            }}
        }}

        main locus App {{
            bindings {{
                Recs: shm_ring("{shm_name}", on_overflow: drop,
                               layout: ForeignRing, buffer_size: 4096);
            }}
        }}

        fn main() {{
            App {{ }};
            Sub {{ }};
            // Let the reader's first poll initialize its cursor to 0 before
            // the in-process producer publishes (no historical replay).
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
    assert!(
        stdout.contains("rec kind=1 a=100"),
        "missing the 16-byte record. stdout:\n{}",
        stdout
    );
    assert!(
        stdout.contains("rec kind=2 a=200 b=201"),
        "missing the 24-byte record. stdout:\n{}",
        stdout
    );
}
