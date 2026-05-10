//! m102.5: tower-join end-to-end test.
//!
//! Builds `apps/tower-join/main.ap`, runs it against both
//! checked-in fixtures (operational-graph and import-graph),
//! and asserts on the cross-tower-agreement rule:
//!
//!   - ≥ 2 tower roles populated   → "locus"
//!   - exactly 1 role              → "type_or_fn"
//!   - 0 roles                     → "structural"
//!
//! Per `notes/aperio-types-vs-loci.md`, the join is a
//! never-invent-loci pipeline: cross-tower coincidence
//! decides locus identity, not heuristic guessing.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn build_join() -> PathBuf {
    let src_path = workspace_root()
        .join("apps")
        .join("tower-join")
        .join("main.ap");
    let src = std::fs::read_to_string(&src_path).expect("read main.ap");
    let program = aperio_syntax::parse_source(&src).expect("parse main.ap");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_tower_join_{}_{}",
        std::process::id(),
        nanos
    ));
    build_executable(&program, &bin).expect("build join");
    bin
}

fn run_against(fixture_subdir: &str) -> String {
    let bin = build_join();
    let fixture = workspace_root().join("apps").join(fixture_subdir).join("fixture");
    let out = Command::new(&bin)
        .arg(fixture)
        .output()
        .expect("run join");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "join exited non-zero: {:?}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

// ---- operational-graph fixture: every file has ≥ 2 tower
// roles → all four classify as "locus" ----

#[test]
fn operational_fixture_emits_all_files_as_loci() {
    let json = run_against("operational-graph");
    assert!(
        json.contains("\"loci\": 4"),
        "expected all 4 files to classify as loci; output:\n{}",
        json
    );
    // No structural / type_or_fn classifications expected.
    assert!(
        json.contains("\"type_or_fn\": 0"),
        "unexpected type_or_fn entries; output:\n{}",
        json
    );
    assert!(
        json.contains("\"structural\": 0"),
        "unexpected structural entries; output:\n{}",
        json
    );
}

// Slice from `pos` up to the next "  }," or "  }\n" line, which
// is the end of one file entry in the JSON output. Fallback to
// EOF if not found.
fn entry_scope(json: &str, pos: usize) -> &str {
    // The closing brace of a file entry is "    }" at indent 4
    // (followed by ",\n" or "\n" with the closing array). Look
    // for the first "    }" boundary after `pos`.
    let from = pos;
    if let Some(end) = json[from..].find("\n    }") {
        &json[from..from + end + 6]
    } else {
        &json[from..]
    }
}

#[test]
fn operational_main_file_proposes_main_l_locus_name() {
    let json = run_against("operational-graph");
    // main.go has: harmonic(log,net/http) + operational(main+init+spawn).
    // 2 roles → locus → name MainL.
    let main_pos = json
        .find("\"name\": \"main.go\"")
        .expect("main.go entry");
    let scope = entry_scope(&json, main_pos);
    assert!(
        scope.contains("\"verdict\": \"locus\""),
        "main.go should be locus; scope: {}",
        scope
    );
    assert!(
        scope.contains("\"proposed_locus_name\": \"MainL\""),
        "main.go should propose MainL; scope: {}",
        scope
    );
    assert!(
        scope.contains("\"main\": true"),
        "operational role should mark main=true; scope: {}",
        scope
    );
    assert!(
        scope.contains("\"init\": true"),
        "operational role should mark init=true; scope: {}",
        scope
    );
}

#[test]
fn operational_store_file_emits_domain_role_with_motion_forms() {
    let json = run_against("operational-graph");
    // store.go has: harmonic(sync) + domain(3 types). The
    // domain role section embeds motion-forms inline.
    let pos = json.find("\"name\": \"store.go\"").expect("store.go entry");
    let scope = entry_scope(&json, pos);
    assert!(
        scope.contains("\"verdict\": \"locus\""),
        "store.go should be locus; scope: {}",
        scope
    );
    assert!(
        scope.contains("\"proposed_locus_name\": \"StoreL\""),
        "store.go should propose StoreL; scope: {}",
        scope
    );
    assert!(
        scope.contains("RequestCache"),
        "expected RequestCache type in domain role; scope: {}",
        scope
    );
    assert!(
        scope.contains("\"motion\": \"<unknown:Request>-remembering\""),
        "expected motion form embedded inline; scope: {}",
        scope
    );
}

// ---- import-graph fixture: util.go has 0 roles → "structural";
// greet.go + server.go have only imports → "type_or_fn";
// main.go has imports + main fn → "locus" ----

#[test]
fn import_graph_fixture_discriminates_verdicts() {
    let json = run_against("import-graph");
    // Summary counts.
    assert!(json.contains("\"loci\": 1"), "expected 1 locus; output:\n{}", json);
    assert!(json.contains("\"type_or_fn\": 2"), "expected 2 type_or_fn; output:\n{}", json);
    assert!(json.contains("\"structural\": 1"), "expected 1 structural; output:\n{}", json);
}

#[test]
fn import_graph_util_classifies_as_structural() {
    // util.go has no imports + no operational signal + no types
    // (just const + free fn). agreement=0 → structural.
    let json = run_against("import-graph");
    let pos = json.find("\"name\": \"util.go\"").expect("util.go entry");
    let scope = entry_scope(&json, pos);
    assert!(
        scope.contains("\"verdict\": \"structural\""),
        "util.go should be structural; scope: {}",
        scope
    );
    assert!(
        scope.contains("\"agreement\": 0"),
        "util.go agreement should be 0; scope: {}",
        scope
    );
}

#[test]
fn import_graph_imports_only_files_classify_as_type_or_fn() {
    // greet.go and server.go have imports but no operational
    // signal, no types. agreement=1 → type_or_fn.
    let json = run_against("import-graph");
    for f in ["greet.go", "server.go"] {
        let pos = json
            .find(&format!("\"name\": \"{}\"", f))
            .unwrap_or_else(|| panic!("missing entry for {}; output:\n{}", f, json));
        let scope = entry_scope(&json, pos);
        assert!(
            scope.contains("\"verdict\": \"type_or_fn\""),
            "{} should be type_or_fn; scope: {}",
            f,
            scope
        );
        assert!(
            scope.contains("\"agreement\": 1"),
            "{} agreement should be 1; scope: {}",
            f,
            scope
        );
    }
}

#[test]
fn import_graph_main_classifies_as_locus_via_op_plus_harmonic() {
    // main.go has imports(fmt) + main fn. agreement=2 → locus.
    let json = run_against("import-graph");
    let pos = json.find("\"name\": \"main.go\"").expect("main.go entry");
    let scope = entry_scope(&json, pos);
    assert!(
        scope.contains("\"verdict\": \"locus\""),
        "main.go should be locus; scope: {}",
        scope
    );
    assert!(
        scope.contains("\"proposed_locus_name\": \"MainL\""),
        "expected MainL proposal; scope: {}",
        scope
    );
}

// ---- The never-invent-loci property ----

#[test]
fn no_locus_emitted_without_cross_tower_agreement() {
    // Negative assertion: every "locus" verdict must have
    // agreement >= 2. Walk per-file entries and match each
    // entry's verdict against its agreement count. Catches a
    // regression where the threshold is weakened or the
    // agreement count miscomputed.
    //
    // Verdict appears BEFORE agreement in each entry (due to
    // emission order in __per_file_entry), so we scan by
    // splitting the file entries and checking each one.
    let json = run_against("import-graph");
    let mut entries = Vec::new();
    let mut search = 0;
    while let Some(start) = json[search..].find("\"name\":") {
        let abs = search + start;
        // End of entry: next "    }" boundary.
        let end = json[abs..]
            .find("\n    }")
            .map(|e| abs + e + 6)
            .unwrap_or(json.len());
        entries.push(json[abs..end].to_string());
        search = end;
    }
    assert!(!entries.is_empty(), "no entries parsed; output:\n{}", json);
    for entry in &entries {
        if entry.contains("\"verdict\": \"locus\"") {
            // Find "agreement": <n>
            let p = entry
                .find("\"agreement\":")
                .expect("entry has agreement field");
            let tail = &entry[p + 12..];
            let after = tail.trim_start_matches(' ');
            let n: u32 = after
                .chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .unwrap_or(0);
            assert!(
                n >= 2,
                "locus verdict with agreement {} (must be >= 2): {}",
                n, entry
            );
        }
    }
}
