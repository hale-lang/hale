//! WS1#2 carrier — cross-seed struct literal whose Decimal (i128)
//! fields come from a bus-deserialized struct.
//!
//! A downstream app's P2.1 reported a flaky segfault ("heap corruption
//! signature") constructing `gx::GreaseOrderRequest { px: oi.px,
//! qty: oi.qty }` from a bus-received `d::OrderIntent`. The two
//! contributing axes are (a) the value crossed a bus-delivery
//! boundary copy and (b) the destination is a qualified-seed
//! struct literal. Decimal is an i128 inline value, so the latent
//! hazard is alignment: an i128 store (`movaps`) traps on an
//! 8-byte-aligned destination (the 2026-05-20 arena bug, fixed in
//! `lotus_arena_off_for`). This deterministic carrier exercises
//! the bus + cross-seed allocation path so any regression of that
//! alignment guarantee — on whatever allocation the literal lands
//! in — surfaces as a crash or a garbled read-back rather than a
//! flaky field report downstream.
//!
//! Two library seeds are bound: `d` (the topic + payload type) and
//! `gx` (the downstream struct). The consumer subscribes, and in
//! the handler builds the `gx` literal from the delivered Decimal
//! fields and prints them back.

use std::path::PathBuf;
use std::process::Command;


#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn fixtures_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p
}

#[test]
fn cross_seed_struct_literal_from_bus_deserialized_decimal() {
    let consumer_src_path = fixtures_dir()
        .join("import-ws1-bus-decimal-consumer")
        .join("main.hl");

    // The consumer's two `import` lines bind both library seeds under
    // their aliases; the loader resolves them.
    let bin = harness::unique_bin(&format!("hale_ws1_xseed_bus_decimal_{}", std::process::id()));
    build_opts::build_seed_dir(&consumer_src_path, &bin, &build_opts::options())
        .expect("build consumer + two libs");

    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit (WS1#2 segfault carrier regressed): {:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Each Decimal must survive the bus boundary + cross-seed
    // literal intact. Exact i128 round-trip, not approximate.
    assert!(
        stdout.contains("req id=1 px=12345.67 qty=0.5 tag=grease"),
        "intent 1 Decimal fields corrupted across bus+cross-seed: {:?}",
        stdout
    );
    assert!(
        stdout.contains("req id=2 px=99999.99 qty=250.125 tag=grease"),
        "intent 2 Decimal fields corrupted across bus+cross-seed: {:?}",
        stdout
    );
    assert!(
        stdout.contains("req id=3 px=0.000001 qty=1000000 tag=grease"),
        "intent 3 Decimal fields corrupted across bus+cross-seed: {:?}",
        stdout
    );
    // Arithmetic over the persisted (locus-arena) Decimals:
    // 12345.67 + 99999.99 + 0.000001 = 112345.660001. A partial
    // i128 corruption that survived the per-field eyeball would
    // still skew this sum.
    assert!(
        stdout.contains("acc=112345.660001"),
        "persisted Decimal accumulation wrong (corruption across the \
         locus-arena store): {:?}",
        stdout
    );
}
