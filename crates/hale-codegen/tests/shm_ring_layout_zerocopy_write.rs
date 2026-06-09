//! A1 zero-copy writable view (2026-06-08): the `Topic.write(max) { w =>
//! ... }` construct lets a producer write record fields DIRECTLY into the
//! mapped ring slot (via `std::bytes::write_*`), reserving up to `max`
//! bytes and committing the byte count the body's tail yields — no
//! intermediate buffer + copy. Single binary (App creates the ring, Sub
//! attaches, Producer writes); the Sub decodes both differently-sized
//! records, proving reserve → write-in-place → commit round-trips.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_tag(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("zcw-{}-{}-{}", label, std::process::id(), nanos)
}

#[test]
fn hale_zero_copy_write_in_place_round_trips() {
    let shm_name = format!("/hale-{}", unique_tag("e2e"));
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
                Recs.write(24) {{ w =>
                    std::bytes::write_i64_le(w, 0, 1) or raise;
                    std::bytes::write_i64_le(w, 8, 100) or raise;
                    16
                }};
                Recs.write(24) {{ w =>
                    std::bytes::write_i64_le(w, 0, 2) or raise;
                    std::bytes::write_i64_le(w, 8, 200) or raise;
                    std::bytes::write_i64_le(w, 16, 201) or raise;
                    24
                }};
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
        "binary failed: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rec kind=1 a=100"), "missing 16-byte record:\n{}", stdout);
    assert!(stdout.contains("rec kind=2 a=200 b=201"), "missing 24-byte record:\n{}", stdout);
}
