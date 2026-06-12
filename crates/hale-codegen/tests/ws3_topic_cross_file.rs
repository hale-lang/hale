//! WS3.3 — a bus topic declared in a different file than its
//! publisher (and a cross-seed subscriber).
//!
//! pond reported (FRICTION, corrected 2026-06-08) that `publish T`
//! + `T <- v` only resolved a `topic T` declared in the *same*
//! `.hl` file as the publishing locus, forcing topics + publishers
//! to be collapsed into one file in every library. This builds a
//! two-file library seed — `topics.hl` declares `Heartbeat`,
//! `emitter.hl` publishes it by bare name and sends on it — imported
//! by a consumer that subscribes via the qualified `hb::Heartbeat`.
//! If the publish/send sites resolve the topic across the file
//! boundary, the consumer receives both messages.

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
fn topic_decl_and_publisher_in_separate_lib_files() {
    let lib_dir = fixtures_dir().join("lib-ws33-topic-split");
    let consumer_src_path = fixtures_dir()
        .join("import-ws33-topic-split-consumer")
        .join("main.hl");

    let consumer_src =
        std::fs::read_to_string(&consumer_src_path).expect("read consumer main.hl");
    let mut consumer_prog = parse_source(&consumer_src).expect("parse consumer");
    consumer_prog.imports.clear();

    let (lib_items, renames) = resolve_and_mangle_lib(&lib_dir, "hb");
    consumer_prog.items.extend(lib_items);

    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_ws33_topic_split_{}", std::process::id()));
    build_executable_with_imports(&consumer_prog, &bin, &renames)
        .expect("build consumer + split-topic lib");

    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit (cross-file topic resolution regressed): {:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("got 7"), "missing first publish: {:?}", stdout);
    assert!(stdout.contains("got 11"), "missing second publish: {:?}", stdout);
    assert!(stdout.contains("done"), "missing done sentinel: {:?}", stdout);
}
