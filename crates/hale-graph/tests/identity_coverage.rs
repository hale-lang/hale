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
            text.contains("hale_graph::identity::stale_hash_paths("),
            "{rel}: the stale hash takes its path list from hale_graph::identity"
        );
    }
    assert!(
        !root().join("crates/hale-codegen/runtime/stdlib").exists(),
        "codegen/runtime/stdlib exists again; stale_hash_paths must say which tree is the stdlib"
    );
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
    let stale = hale_graph::identity::stale_hash_paths(&root.join("crates/hale-codegen"));
    assert!(
        stale.len() > 3 && stale.iter().all(|p| p.is_file()),
        "the stale hash's paths exist: {stale:?}"
    );
}

/// The inputs each identity consumer actually hashes, held against a
/// scratch tree: the build scripts fold `identity_files` (the text
/// check above holds that they do), so a change to any file the
/// selection should cover must move the fold. The cached host is
/// built by the CLI's `build` verb and links the dependencies the
/// lock file and the ts-shim manifest pin, so those move it as much
/// as a graph-core change does.
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
    for name in ["toolchain_digest", "compiler_src_hash"] {
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
    let stale = classes_of(&hale_graph::identity::stale_hash_paths(&root.join("crates/hale-codegen")));
    assert_eq!(
        covered_classes(identity("codegen_src_hash")),
        stale,
        "`codegen_src_hash` folds `stale_hash_paths`: its `covers` must name the classes that list holds"
    );
    // The two build scripts and the stale check call the selection the
    // entry says they fold.
    assert!(read("crates/hale-cli/build.rs").contains("identity_files("));
    assert!(read("crates/hale-iris/build.rs").contains("identity_files("));
    assert!(read("crates/hale-cli/src/shared/stale.rs").contains("stale_hash_paths("));
}

/// The legacy rows an inventory entry's producer still shares: the
/// identities whose own correction (I2 and I3 for `exec_digest`, I4
/// for the stale-binary hash) retires the row. Every other legacy row
/// names a symbol no inventory entry produces.
const LEGACY_STILL_SHARED: &[&str] = &["exec_digest", "compute_codegen_src_hash"];

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
