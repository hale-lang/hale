//! Proposal A′ (2026-06-09): repr-tagged field accessors. A struct whose
//! fields carry `repr:"<wire-type>"` tags is a binary layout; `L2::price(v)`
//! reads the field at its computed offset from a BytesView, and
//! `L2::set_price(w, x)` writes it into a Topic.write block's BytesMut.
//! Desugars to `std::bytes::read_*`/`write_*`, so it composes with the
//! foreign-ring consumer and the zero-copy producer. End-to-end: a producer
//! writes records field-by-field via accessors, a subscriber reads them
//! back via accessors.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn tag(label: &str) -> String {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("rfa-{}-{}-{}", label, std::process::id(), n)
}

#[test]
fn repr_accessors_round_trip_through_a_foreign_ring() {
    let shm = format!("/hale-{}", tag("e2e"));
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

        // A 12-byte wire record: u8 kind @0, u32_le price @4 (padded),
        // u32_le qty @8. Offsets pinned to exercise the `at=` override.
        type L2 {{
            kind:  Int `repr:"u8,at=0"`;
            price: Int `repr:"u32_le,at=4"`;
            qty:   Int `repr:"u32_le,at=8"`;
        }}

        topic Recs {{ payload: BytesView; }}

        locus Sub {{
            bus {{ subscribe Recs as on_rec; }}
            fn on_rec(v: BytesView) {{
                let k = L2::kind(v)  or -1;
                let p = L2::price(v) or -1;
                let q = L2::qty(v)   or -1;
                println("rec kind=", to_string(k), " price=", to_string(p), " qty=", to_string(q));
            }}
        }}

        locus Producer {{
            bus {{ publish Recs; }}
            birth() {{
                Recs.write(12) {{ w =>
                    L2::set_kind(w, 1)     or raise;
                    L2::set_price(w, 250)  or raise;
                    L2::set_qty(w, 9)      or raise;
                    12
                }};
                Recs.write(12) {{ w =>
                    L2::set_kind(w, 2)     or raise;
                    L2::set_price(w, 999)  or raise;
                    L2::set_qty(w, 3)      or raise;
                    12
                }};
            }}
        }}

        main locus App {{
            bindings {{
                Recs: shm_ring("{shm}", on_overflow: drop, layout: ForeignRing, buffer_size: 4096);
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
        shm = shm,
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_{}.bin", tag("bin")));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "failed: {:?}\nstderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rec kind=1 price=250 qty=9"), "missing record 1:\n{}", stdout);
    assert!(stdout.contains("rec kind=2 price=999 qty=3"), "missing record 2:\n{}", stdout);
}
