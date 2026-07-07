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

use hale_codegen::build_executable_with_imports;
use hale_codegen::mangle;
use hale_syntax::ast::{Program, TopDecl};
use hale_syntax::parse_source;

fn fixtures_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p
}

/// Replicate the CLI's resolve-and-mangle pipeline for one import:
/// read every `.hl` in the lib dir, parse, build a unified rename
/// map across files, mangle each, and return (merged_lib_items,
/// per-build_renames). Mirrors the helper in `cross_seed_imports`.
fn resolve_and_mangle_lib(
    lib_dir: &PathBuf,
    alias: &str,
) -> (Vec<TopDecl>, Vec<(Vec<String>, String)>) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(lib_dir)
        .expect("read lib dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("hl"))
        .collect();
    files.sort();
    let mut parsed: Vec<(String, Program)> = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).expect("read lib file");
        let prog = parse_source(&src).expect("parse lib file");
        let stem = f
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("x")
            .to_string();
        parsed.push((stem, prog));
    }
    let stem_refs: Vec<(String, &Program)> =
        parsed.iter().map(|(s, p)| (s.clone(), p)).collect();
    let seed_renames = mangle::build_seed_renames(&stem_refs, alias);
    let mut renames: Vec<(Vec<String>, String)> = Vec::new();
    for (name, mangled) in &seed_renames {
        renames.push((vec![alias.to_string(), name.clone()], mangled.clone()));
    }
    let mut items: Vec<TopDecl> = Vec::new();
    for (_, mut prog) in parsed {
        mangle::mangle_with_renames(&mut prog, &seed_renames);
        items.extend(prog.items);
    }
    (items, renames)
}

#[test]
fn cross_seed_struct_literal_from_bus_deserialized_decimal() {
    let intent_dir = fixtures_dir().join("lib-ws1-intent");
    let grease_dir = fixtures_dir().join("lib-ws1-grease");
    let consumer_src_path = fixtures_dir()
        .join("import-ws1-bus-decimal-consumer")
        .join("main.hl");

    let consumer_src =
        std::fs::read_to_string(&consumer_src_path).expect("read consumer main.hl");
    let mut consumer_prog = parse_source(&consumer_src).expect("parse consumer");
    consumer_prog.imports.clear();

    // Bind both library seeds under their consumer aliases.
    let (intent_items, intent_renames) = resolve_and_mangle_lib(&intent_dir, "d");
    let (grease_items, grease_renames) = resolve_and_mangle_lib(&grease_dir, "gx");
    consumer_prog.items.extend(intent_items);
    consumer_prog.items.extend(grease_items);
    let mut renames = intent_renames;
    renames.extend(grease_renames);

    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_ws1_xseed_bus_decimal_{}", std::process::id()));
    build_executable_with_imports(&consumer_prog, &bin, &renames)
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
