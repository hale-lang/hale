//! WS1#4 carrier — whole-reassignment of a nested locus param of a
//! cross-seed (imported) type.
//!
//! Fathom mdgw-evm reported that `self.conn = ws::WsClient { url:
//! …, … }` (reconnecting by swapping the whole nested-locus param)
//! left the new instance half-initialized: `conn.url` logged
//! `(null)` and the first `read_msg()` crashed. In-place field
//! mutation (`self.conn.url = …`) worked. The single-seed form of
//! this passes at HEAD (see `notes/ws0-friction-verification`), so
//! this carrier exercises the untested axis: the reassigned type is
//! imported from another seed, and its handle-like fields are
//! established in `birth()`.
//!
//! Expectation: the reassigned instance is fully live — params land
//! (`url` = the new value, not null), `birth()` re-runs (the
//! handle `fd` = 7, `ready` = 1), and `read_msg()` returns without
//! crashing. A half-init regression shows up as a null/zero field
//! or a crash.

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
fn cross_seed_nested_locus_param_whole_reassignment_is_fully_initialized() {
    let conn_dir = fixtures_dir().join("lib-ws1-conn");
    let consumer_src_path = fixtures_dir()
        .join("import-ws1-conn-reassign-consumer")
        .join("main.hl");

    let consumer_src =
        std::fs::read_to_string(&consumer_src_path).expect("read consumer main.hl");
    let mut consumer_prog = parse_source(&consumer_src).expect("parse consumer");
    consumer_prog.imports.clear();

    let (conn_items, renames) = resolve_and_mangle_lib(&conn_dir, "wsx");
    consumer_prog.items.extend(conn_items);

    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_ws1_xseed_reassign_{}", std::process::id()));
    build_executable_with_imports(&consumer_prog, &bin, &renames)
        .expect("build consumer + lib");

    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit (WS1#4 half-init carrier regressed — likely a \
         crash in read_msg() on a half-initialized reassigned conn): \
         {:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Initial instance (default param + birth()).
    assert!(
        stdout.contains("init   url=wss://first fd=7 ready=1 read=8"),
        "initial cross-seed instance not fully initialized: {:?}",
        stdout
    );
    // After whole-reassignment: new url landed, birth() re-ran
    // (fd=7, ready=1), read_msg() ran on a fresh seq (fd + seq=1 = 8).
    assert!(
        stdout.contains("reconn url=wss://second fd=7 ready=1 read=8"),
        "reassigned cross-seed nested param is half-initialized \
         (fathom mdgw-evm shape): {:?}",
        stdout
    );
}
