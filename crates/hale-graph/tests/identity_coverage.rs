//! Every compiler identity covers every identity-covered crate, and
//! every workspace member is either covered or excluded for a reason.
//!
//! The build scripts and the stale check take the list and the walk
//! from `hale_graph::identity`, so their coverage is the list's by
//! construction. What a text check still has to hold is that they do
//! take it from there: a script that grew its own list again would
//! drift the day a crate moves.

use std::path::PathBuf;

fn root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

#[test]
fn every_workspace_member_is_covered_or_excluded_with_a_reason() {
    let manifest = read("Cargo.toml");
    let start = manifest.find("members = [").expect("members");
    let end = start + manifest[start..].find(']').expect("members end");
    let members: Vec<String> = manifest[start..end]
        .lines()
        .filter_map(|l| l.trim().strip_prefix("\"crates/"))
        .map(|l| l.trim_end_matches("\",").trim_end_matches('"').to_string())
        .collect();
    assert!(members.len() >= 10, "member scan is vacuous: {members:?}");
    let covered: Vec<&str> = hale_graph::identity::COVERED_CRATES.to_vec();
    let excluded: Vec<&str> = hale_graph::identity::NOT_COVERED
        .iter()
        .map(|(c, _)| *c)
        .collect();
    let unaccounted: Vec<&String> = members
        .iter()
        .filter(|m| !covered.contains(&m.as_str()) && !excluded.contains(&m.as_str()))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "workspace member(s) neither covered by the compiler identities nor excluded with a reason: {unaccounted:?}. \
         Add each to hale_graph::identity::COVERED_CRATES if it shapes a compiled program or a recording, \
         else to NOT_COVERED with the reason."
    );
    for c in covered.iter().chain(excluded.iter()) {
        assert!(
            members.contains(&c.to_string()),
            "`{c}` is listed but is not a workspace member"
        );
    }
    for (c, why) in hale_graph::identity::NOT_COVERED {
        assert!(!why.trim().is_empty(), "`{c}` is excluded without a reason");
    }
}

#[test]
fn the_identities_take_the_list_and_the_walk_from_hale_graph() {
    let cli_build = read("crates/hale-cli/build.rs");
    assert!(
        cli_build.contains("hale_graph::identity::identity_files("),
        "the replay identity must frame hale_graph::identity's selection"
    );
    let iris_build = read("crates/hale-iris/build.rs");
    assert!(
        iris_build.contains("hale_graph::identity::identity_files(")
            && iris_build.contains("hale_graph::identity::fold_files("),
        "the toolchain cache key must fold hale_graph::identity's selection with its fold"
    );
    for rel in [
        "crates/hale-cli/build.rs",
        "crates/hale-cli/src/shared/stale.rs",
    ] {
        let text = read(rel);
        assert!(
            text.contains("hale_graph::identity::identity_files(")
                && text.contains("hale_graph::identity::fold_files("),
            "{rel}: the stale hash folds hale_graph::identity's selection with its fold, at build and at run time"
        );
    }
}

#[test]
fn every_covered_directory_exists_and_every_covered_crate_contributes() {
    let root = root();
    for d in hale_graph::identity::covered_dirs(&root, &[]) {
        assert!(
            d.is_dir(),
            "{} is not a directory (a missing rerun-if-changed path is always stale)",
            d.display()
        );
    }
    for krate in hale_graph::identity::COVERED_CRATES {
        let mut files = Vec::new();
        for d in hale_graph::identity::covered_dirs(&root, &[]) {
            if d.starts_with(root.join("crates").join(krate)) {
                hale_graph::identity::walk_sources(&d, &mut files);
            }
        }
        assert!(
            !files.is_empty(),
            "`{krate}` is covered in name only: it contributes no source file"
        );
    }
    for f in hale_graph::identity::manifest_files(&root) {
        assert!(f.is_file(), "{} vanished", f.display());
    }
    assert_eq!(
        hale_graph::identity::manifest_files(&root).len(),
        hale_graph::identity::MANIFEST_FILES.len(),
        "a manifest file the identity names does not exist"
    );
}

/// The inputs each identity consumer actually hashes, held against a
/// scratch tree: the build scripts fold `identity_files` (the text
/// check above holds that they do), so a change to any file the
/// selection should cover must move the fold. The cached host is
/// built by the CLI's `build` verb and links the dependencies the
/// lock file and the ts-shim manifest pin, so those move it as much
/// as a graph-core change does. The fold is `hale-iris/build.rs`'s
/// whole body, so these are `compiler_src_hash`'s two halves: what the
/// selection holds moves it, what it leaves out keeps it.
#[test]
fn a_change_to_any_hashed_input_moves_the_cache_key() {
    use hale_graph::identity::{fold_files, identity_files, COVERED_CRATES, MANIFEST_FILES};
    let scratch = std::env::temp_dir().join(format!(
        "hale-graph-identity-inputs-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    for krate in COVERED_CRATES {
        let src = scratch.join("crates").join(krate).join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), format!("// {krate}\n")).unwrap();
    }
    for m in MANIFEST_FILES {
        let p = scratch.join(m);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, format!("# {m}\n")).unwrap();
    }
    let key = || fold_files(&scratch, &identity_files(&scratch));
    let hashed: Vec<PathBuf> = identity_files(&scratch);
    for input in [
        "crates/hale-cli/src/lib.rs",
        "crates/hale-graph/src/lib.rs",
        "Cargo.lock",
        "crates/hale-ts-shim/Cargo.toml",
    ] {
        assert!(
            hashed.contains(&scratch.join(input)),
            "{input} is not among the cache key's inputs"
        );
        let before = key();
        let p = scratch.join(input);
        let mut text = std::fs::read_to_string(&p).unwrap();
        text.push_str("// changed\n");
        std::fs::write(&p, text).unwrap();
        assert_ne!(
            before,
            key(),
            "a change to {input} leaves the cache key where it was"
        );
    }
    let before = key();
    std::fs::write(
        scratch.join("crates/hale-cli/src/verb.rs"),
        "fn build() {}\n",
    )
    .unwrap();
    assert_ne!(
        before,
        key(),
        "a CLI source added leaves the cache key where it was"
    );
    // What the selection leaves out keeps the key: a crate no identity
    // covers, a covered crate's tests, a file of no source extension.
    let before = key();
    for (input, text) in [
        ("crates/hale-lsp/src/lib.rs", "// an uncovered crate\n"),
        ("crates/hale-types/tests/probe.rs", "// a covered crate's tests\n"),
        ("crates/hale-types/src/NOTES.md", "not a source\n"),
    ] {
        let p = scratch.join(input);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        assert_eq!(before, key(), "{input} moved the cache key");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

fn identity(name: &str) -> &'static hale_graph::identity::Identity {
    hale_graph::identity::IDENTITIES
        .iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("no inventory entry named `{name}`"))
}

#[test]
fn the_inventory_names_each_identity_once_and_fills_every_column() {
    let mut seen = std::collections::BTreeSet::new();
    for i in hale_graph::identity::IDENTITIES {
        assert!(seen.insert(i.name), "`{}` is inventoried twice", i.name);
        for (column, text) in [
            ("identifies", i.identifies),
            ("computed", i.computed),
            ("producer file", i.producer.0),
            ("producer symbol", i.producer.1),
            ("on_mismatch", i.on_mismatch),
            ("versioned_by", i.versioned_by),
        ] {
            assert!(!text.trim().is_empty(), "`{}`: `{column}` is empty", i.name);
        }
        assert!(!i.covers.is_empty(), "`{}` covers nothing", i.name);
        assert!(!i.consumers.is_empty(), "`{}` has no consumer", i.name);
        for (_, why) in i.leaves_out {
            assert!(!why.trim().is_empty(), "`{}` leaves an input out without a reason", i.name);
        }
        if let Some(f) = i.frozen {
            assert!(!f.trim().is_empty(), "`{}` is frozen by nothing it names", i.name);
        }
    }
}

/// Each identity's two halves (the F.40 exit audit's bar): a test that
/// shows a covered change MOVES the value and one that shows an
/// uncovered change KEEPS it, as `(identity, moves, keeps)`, each a
/// `file::fn` whose name or doc names the identity. One test may show
/// both. "Covered" is the identity's own `covers` in `IDENTITIES`.
const BOTH_HALVES: &[(&str, &str, &str)] = &[
    (
        "shape_hash",
        "crates/hale-cli/tests/claims_artifact_unknowns.rs::an_untyped_receiver_edge_changes_shape_hash",
        "crates/hale-cli/tests/claims_artifact_unknowns.rs::an_untyped_receiver_edge_changes_shape_hash",
    ),
    (
        "model_hash",
        "crates/hale-cli/tests/obs_model_hash.rs::model_identity_is_stamped_and_tracks_the_model",
        "crates/hale-cli/tests/obs_model_hash.rs::model_identity_is_stamped_and_tracks_the_model",
    ),
    (
        "artifact_digest",
        "crates/hale-types/tests/artifact_integrity.rs::a_different_program_produces_a_different_digest",
        "crates/hale-cli/tests/source_map.rs::one_tree_checked_out_at_two_roots_has_one_artifact",
    ),
    (
        "law_digest",
        "crates/hale-types/tests/artifact_law_projection.rs::law_digest_moves_with_a_row_and_keeps_without_one",
        "crates/hale-types/tests/artifact_law_projection.rs::law_digest_moves_with_a_row_and_keeps_without_one",
    ),
    (
        "claim_table_digest",
        "crates/hale-types/tests/judgment_certificates.rs::relowered_table_from_edited_source_is_refused",
        "crates/hale-types/tests/judgment_certificates.rs::relowered_table_from_edited_source_is_refused",
    ),
    (
        "analysis_coverage_digest",
        "crates/hale-types/tests/judgment_certificates.rs::coverage_change_invalidates_evidence_identity",
        "crates/hale-types/tests/judgment_certificates.rs::coverage_change_invalidates_evidence_identity",
    ),
    (
        "dispatch_plan_digest",
        "crates/hale-types/tests/dispatch_plan.rs::the_digest_tracks_the_plan",
        "crates/hale-types/tests/dispatch_plan.rs::the_digest_tracks_the_plan",
    ),
    (
        "exec_digest",
        "crates/hale-cli/tests/replay_cli.rs::same_named_imports_with_their_contents_swapped_are_two_identities",
        "crates/hale-cli/tests/replay_cli.rs::one_program_at_two_roots_has_one_identity",
    ),
    (
        "stale_src_hash",
        "crates/hale-cli/src/shared/stale.rs::an_edit_to_any_covered_source_is_stale_and_an_uncovered_one_is_not",
        "crates/hale-cli/src/shared/stale.rs::an_edit_to_any_covered_source_is_stale_and_an_uncovered_one_is_not",
    ),
    (
        "compiler_src_hash",
        "crates/hale-graph/tests/identity_coverage.rs::a_change_to_any_hashed_input_moves_the_cache_key",
        "crates/hale-graph/tests/identity_coverage.rs::a_change_to_any_hashed_input_moves_the_cache_key",
    ),
    (
        "toolchain_hash",
        "crates/hale-iris/src/lib.rs::a_build_knob_moves_the_key_and_the_same_options_keep_it",
        "crates/hale-cli/src/build_env.rs::the_host_caches_options_are_the_inherited_builds_fingerprint",
    ),
    (
        "embedded_dna_digest",
        "crates/hale-cli/tests/stale_dna_warning.rs::a_tree_the_binary_embeds_raises_nothing_and_an_edited_one_warns",
        "crates/hale-cli/tests/stale_dna_warning.rs::a_tree_the_binary_embeds_raises_nothing_and_an_edited_one_warns",
    ),
    (
        "source_digest",
        "crates/hale-cli/tests/source_map.rs::a_source_digest_tracks_its_contents",
        "crates/hale-cli/tests/source_map.rs::a_source_digest_tracks_its_contents",
    ),
    (
        "obs_entity_id_digest",
        "crates/hale-cli/tests/obs_entity_ids.rs::the_entity_id_table_publishes_its_own_identity",
        "crates/hale-cli/tests/obs_entity_ids.rs::the_entity_id_table_publishes_its_own_identity",
    ),
    (
        "snapshot_key",
        "crates/hale-types/tests/demand_gate.rs::a_snapshot_key_tells_different_loads_apart",
        "crates/hale-types/tests/demand_gate.rs::a_snapshot_key_tells_different_loads_apart",
    ),
    (
        "snapshot_config_digest",
        "crates/hale-frontend/src/snapshot.rs::a_changed_input_is_a_distinct_snapshot_and_shares_no_result",
        "crates/hale-frontend/src/snapshot.rs::a_changed_input_is_a_distinct_snapshot_and_shares_no_result",
    ),
    (
        "snapshot_overlay_digest",
        "crates/hale-frontend/src/snapshot.rs::a_changed_input_is_a_distinct_snapshot_and_shares_no_result",
        "crates/hale-frontend/src/snapshot.rs::a_changed_input_is_a_distinct_snapshot_and_shares_no_result",
    ),
    (
        "snapshot_sources_digest",
        "crates/hale-types/tests/demand_gate.rs::a_snapshot_key_tells_different_loads_apart",
        "crates/hale-types/tests/demand_gate.rs::a_snapshot_key_tells_different_loads_apart",
    ),
    (
        "constitution_digest",
        "crates/hale-types/tests/constitutions.rs::constitution_identity_follows_the_closure_not_the_name",
        "crates/hale-types/tests/constitutions.rs::constitution_identity_follows_the_closure_not_the_name",
    ),
    (
        "fleet_shape_hash",
        "crates/hale-cli/tests/fleet_compose.rs::the_fleet_shape_hash_tracks_the_arrangement_not_provenance",
        "crates/hale-cli/tests/fleet_compose.rs::the_fleet_shape_hash_tracks_the_arrangement_not_provenance",
    ),
    (
        "runtime_object_key",
        "crates/hale-codegen/src/codegen.rs::runtime_object_key_moves_with_source_flags_and_compiler_and_keeps_the_host_clang",
        "crates/hale-codegen/src/codegen.rs::runtime_object_key_moves_with_source_flags_and_compiler_and_keeps_the_host_clang",
    ),
];

/// The identities no test shows both halves of, each with the seam its
/// computation lacks. A gap, not a justification: an entry leaves when
/// the computation takes its inputs as arguments.
const NO_SEAM: &[(&str, &str)] = &[
    (
        "toolchain_digest",
        "a gap: the framing lives in `hale-cli/build.rs`, which prints the value rather than returning it and reads the rustc version and the commit from subprocesses; no test can call it, so neither half is shown (the selection it frames is `compiler_src_hash`'s, shown above)",
    ),
    (
        "analysis_inputs_digest",
        "a gap: `analysis_inputs_digest()` takes no argument and folds only compile-time constants (the semantics version, the stdlib source, the package version, the renames, the surface registry), so no test can make a covered change",
    ),
];

/// A doc names `name` as a whole identifier (`fleet_shape_hash` does not
/// name `shape_hash`).
fn doc_names(doc: &str, name: &str) -> bool {
    doc.match_indices(name).any(|(at, _)| {
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !ident(doc[..at].chars().next_back()) && !ident(doc[at + name.len()..].chars().next())
    })
}

/// A test's fn name names `name` as `_`-separated words of its own.
fn fn_names(func: &str, name: &str) -> bool {
    format!("_{func}_").contains(&format!("_{name}_"))
}

/// The doc lines and attributes directly above `fn NAME(` in `text`, and
/// whether it is there at all.
fn test_fn_header(text: &str, name: &str) -> Option<(String, bool)> {
    let lines: Vec<&str> = text.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.trim_start().strip_prefix("fn ").is_some_and(|r| r.starts_with(&format!("{name}("))))?;
    let mut doc = String::new();
    let mut is_test = false;
    for l in lines[..at].iter().rev() {
        let t = l.trim_start();
        if let Some(d) = t.strip_prefix("///") {
            doc.insert_str(0, &format!("{d}\n"));
        } else if t.starts_with("#[") {
            is_test |= t == "#[test]";
        } else {
            break;
        }
    }
    Some((doc, is_test))
}

#[test]
fn the_identity_name_scan_needs_whole_words() {
    assert!(doc_names(" moves `shape_hash` and", "shape_hash"));
    assert!(!doc_names(" the `fleet_shape_hash` moves", "shape_hash"));
    assert!(!doc_names(" project_shape_hashes", "shape_hash"));
    assert!(fn_names("an_untyped_receiver_edge_changes_shape_hash", "shape_hash"));
    assert!(fn_names("law_digest_moves_with_a_row", "law_digest"));
    assert!(!fn_names("toolchain_hashing_moves", "toolchain_hash"));
}

/// Every identity names a test of each half, or is listed with the seam
/// it lacks; each named test exists, is a `#[test]`, and names the
/// identity in its fn name or its doc.
#[test]
fn every_identity_names_a_test_of_each_half() {
    use std::collections::BTreeSet;
    let names: BTreeSet<&str> = hale_graph::identity::IDENTITIES.iter().map(|i| i.name).collect();
    let paired: Vec<&str> = BOTH_HALVES.iter().map(|(n, _, _)| *n).collect();
    let gapped: Vec<&str> = NO_SEAM.iter().map(|(n, _)| *n).collect();
    let listed: BTreeSet<&str> = paired.iter().chain(gapped.iter()).copied().collect();
    assert_eq!(listed.len(), paired.len() + gapped.len(), "an identity is listed twice");
    let missing: Vec<&&str> = names.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "identities with no test of either half: {missing:?}. Name a `file::fn` that shows a covered \
         change moving the value and one that shows an uncovered change keeping it in BOTH_HALVES, \
         or list the identity in NO_SEAM with the seam its computation lacks."
    );
    let stray: Vec<&&str> = listed.difference(&names).collect();
    assert!(stray.is_empty(), "listed but not inventoried in IDENTITIES: {stray:?}");
    for (name, why) in NO_SEAM {
        assert!(why.starts_with("a gap: "), "`{name}`: a NO_SEAM entry is a gap, and says so");
    }
    for (name, moves, keeps) in BOTH_HALVES {
        for (half, at) in [("moves", moves), ("keeps", keeps)] {
            let (file, func) = at.split_once("::").unwrap_or_else(|| panic!("`{name}`: {at} is not file::fn"));
            let (doc, is_test) = test_fn_header(&read(file), func)
                .unwrap_or_else(|| panic!("`{name}` ({half}): no `fn {func}` in {file}"));
            assert!(is_test, "`{name}` ({half}): {file}::{func} is not a #[test]");
            assert!(
                fn_names(func, name) || doc_names(&doc, name),
                "`{name}` ({half}): {file}::{func} names the identity in neither its fn name nor its doc"
            );
        }
    }
}

/// `fn NAME`, `struct NAME`, `const NAME` or `static NAME` at the start
/// of a line (after `pub`, `pub(crate)` and the like), the name ended
/// by a non-identifier character.
fn defined_in(text: &str, name: &str) -> bool {
    text.lines().any(|line| {
        let mut t = line.trim_start();
        for vis in ["pub(crate) ", "pub(super) ", "pub "] {
            if let Some(r) = t.strip_prefix(vis) {
                t = r;
                break;
            }
        }
        ["fn ", "struct ", "const ", "static "].iter().any(|k| {
            t.strip_prefix(k).is_some_and(|rest| {
                rest.starts_with(name)
                    && !rest[name.len()..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
    })
}

#[test]
fn every_inventory_producer_is_defined_in_its_file() {
    for i in hale_graph::identity::IDENTITIES {
        let (file, symbol) = i.producer;
        let text = read(file);
        assert!(
            defined_in(&text, symbol),
            "`{}`: its producer `{symbol}` is not defined in {file}",
            i.name
        );
    }
}

/// The input classes a walked file belongs to, by extension and name.
fn classes_of(files: &[PathBuf]) -> std::collections::BTreeSet<&'static str> {
    use hale_graph::identity::MANIFEST_FILES;
    files
        .iter()
        .map(|f| {
            let rel = f.strip_prefix(root()).unwrap_or(f).to_string_lossy().replace('\\', "/");
            if MANIFEST_FILES.contains(&rel.as_str()) {
                return "manifests";
            }
            match f.extension().and_then(|s| s.to_str()) {
                Some("rs") => "compiler",
                Some("c") | Some("h") => "runtime",
                Some("hl") => "stdlib",
                other => panic!("{}: a walked file of no class ({other:?})", f.display()),
            }
        })
        .collect()
}

fn covered_classes(i: &hale_graph::identity::Identity) -> std::collections::BTreeSet<&'static str> {
    use hale_graph::identity::Input;
    i.covers
        .iter()
        .filter_map(|c| match c {
            Input::CompilerSources => Some("compiler"),
            Input::RuntimeC => Some("runtime"),
            Input::StdlibSeeds => Some("stdlib"),
            Input::Manifests => Some("manifests"),
            _ => None,
        })
        .collect()
}

#[test]
fn a_file_walking_identity_covers_the_classes_its_selection_holds() {
    let root = root();
    let selected = classes_of(&hale_graph::identity::identity_files(&root));
    for name in ["toolchain_digest", "compiler_src_hash", "stale_src_hash"] {
        assert_eq!(
            covered_classes(identity(name)),
            selected,
            "`{name}` folds `identity_files`: its `covers` must name the classes that selection holds"
        );
    }
    // `exec_digest` takes the compiler half whole, through `toolchain_digest`.
    assert_eq!(
        covered_classes(identity("exec_digest")),
        selected,
        "`exec_digest` frames `toolchain_digest`: it covers what that does"
    );
    // The two build scripts and the stale check call the selection the
    // entry says they fold.
    assert!(read("crates/hale-cli/build.rs").contains("identity_files("));
    assert!(read("crates/hale-iris/build.rs").contains("identity_files("));
    assert!(read("crates/hale-cli/src/shared/stale.rs").contains("identity_files("));
}

/// Where the 64-bit FNV offset basis may be written in a crate's `src`
/// or its build script, each with the reason. The goal is the one
/// fold's home: every FNV identity feeds its bytes to
/// `hale_graph::identity::Fnv64` (F.40 phase 4, I6), so a new
/// hand-rolled digest is either an inventory entry that calls the fold
/// or a non-identity use listed here with its reason.
const FNV_BASIS_ALLOWED: &[(&str, &str)] = &[(
    "crates/hale-graph/src/identity.rs",
    "the one fold (`Fnv64::BASIS`) and its known-answer test",
)];

/// Every file under `dir`, recursively.
fn every_file(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            every_file(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn writes_fnv_basis(line: &str) -> bool {
    !line.trim_start().starts_with("//")
        && line.to_ascii_lowercase().replace('_', "").contains("0xcbf29ce484222325")
}

#[test]
fn the_basis_scan_sees_every_spelling_and_skips_comments() {
    assert!(writes_fnv_basis("    let mut h: u64 = 0xcbf29ce484222325;"));
    assert!(writes_fnv_basis("    let mut h: u64 = 0xcbf2_9ce4_8422_2325;"));
    assert!(writes_fnv_basis("    Digest(0xCBF2_9CE4_8422_2325)"));
    assert!(!writes_fnv_basis("//!  * **hash** — FNV-1a/64 (offset 0xcbf29ce484222325, prime"));
    assert!(!writes_fnv_basis("    let mut h = Fnv64::new();"));
}

/// A seam cannot say this: the registry's matcher counts one literal
/// spelling, and the basis is written with and without digit
/// separators, in either case. So the scan normalizes each line
/// (lower case, no `_`) and skips comments, which may name the
/// constant (the topic hash's doc says which C function it mirrors).
#[test]
fn the_fnv_basis_is_written_only_where_the_one_fold_lives() {
    let root = root();
    let mut files = Vec::new();
    for e in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        let krate = e.path();
        every_file(&krate.join("src"), &mut files);
        if krate.join("build.rs").is_file() {
            files.push(krate.join("build.rs"));
        }
    }
    assert!(files.len() > 200, "the scan is vacuous: {} files", files.len());
    let mut found = std::collections::BTreeSet::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        if text.lines().any(writes_fnv_basis) {
            found.insert(f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
    let allowed: std::collections::BTreeSet<String> =
        FNV_BASIS_ALLOWED.iter().map(|(f, _)| f.to_string()).collect();
    let unlisted: Vec<&String> = found.difference(&allowed).collect();
    assert!(
        unlisted.is_empty(),
        "a hand-rolled FNV fold: {unlisted:?} writes the offset basis. Feed the bytes to \
         hale_graph::identity::Fnv64 (or fnv64) and, if the value is compared to decide that two \
         things are the same, add the identity to IDENTITIES; a use that is no identity goes in \
         FNV_BASIS_ALLOWED with the reason."
    );
    let stale: Vec<&String> = allowed.difference(&found).collect();
    assert!(stale.is_empty(), "FNV_BASIS_ALLOWED lists a file that no longer writes the basis: {stale:?}");
    for (f, why) in FNV_BASIS_ALLOWED {
        assert!(!why.trim().is_empty(), "{f} is allowed without a reason");
    }
}

/// The legacy rows an inventory entry's producer still shares: the
/// identities whose own correction retires the row. None is left since
/// I4 retired the stale-binary hash's; every legacy row names a symbol
/// no inventory entry produces.
const LEGACY_STILL_SHARED: &[&str] = &[];

#[test]
fn every_inventory_producer_is_registered_and_no_legacy_row_duplicates_one() {
    let mut registered = std::collections::BTreeSet::new();
    let mut legacy = std::collections::BTreeSet::new();
    for f in hale_graph::families() {
        if let Some(p) = &f.producer {
            registered.insert((p.path, p.symbol));
        }
        for o in f.owned {
            registered.insert((o.path, o.symbol));
        }
        for l in f.legacy {
            registered.insert((l.site.path, l.site.symbol));
            legacy.insert((l.site.path, l.site.symbol));
        }
    }
    for i in hale_graph::identity::IDENTITIES {
        assert!(
            registered.contains(&i.producer),
            "`{}`: its producer {:?} is not a registered site (the `digests` family's `owned` list)",
            i.name,
            i.producer
        );
        if legacy.contains(&i.producer) {
            assert!(
                LEGACY_STILL_SHARED.contains(&i.producer.1),
                "`{}`: a legacy row names its producer {:?}; an inventory entry is registered through the family's `owned` list, not a legacy row",
                i.name,
                i.producer
            );
        }
    }
}
