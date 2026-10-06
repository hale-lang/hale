//! GH #408 Phase 0: the artifact's source map and semantics version.
//!
//! Spans were bundle-GLOBAL byte offsets — a concatenation artifact,
//! meaningful only inside the process that produced them. A consumer
//! composing artifacts from separately compiled applications cannot
//! turn `[1204, 1231]` into a location, so no cross-artifact witness
//! could say where to look, which is most of what a witness is for.
//!
//! Two properties matter and neither is obvious:
//!
//!  - a span must resolve to a FILE, via a source table;
//!  - the artifact must be REPRODUCIBLE, or two machines checking one
//!    commit disagree about a document that exists to be compared.
//!    That is why paths are workspace-relative rather than absolute,
//!    and why they are canonicalized before being made relative.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_srcmap_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn write(root: &Path, rel: &str, src: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(&p, src).expect("write");
}

/// Run with an explicit working directory — the whole point is that
/// the result must not depend on it.
fn dump_from(cwd: &Path, target: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .current_dir(cwd)
        .args(["check", target, "--dump-topology"])
        .output()
        .expect("run hale");
    String::from_utf8_lossy(&out.stdout).to_string()
}

const LIB: &str = r#"
type Msg { v: Int; }
topic Settled { payload: Msg; subject: "app.settled"; }
locus Billing {
    params { n: Int = 0; }
    bus { publish Settled; }
    fn go() { let m = Msg { v: 1 }; Settled <- m; }
}
"#;

const APP: &str = r#"
import "../../lib" as lb;
main locus App { params { b: lb::Billing = lb::Billing { }; } }
fn main() { App { }; }
"#;

fn workspace(tag: &str) -> PathBuf {
    let r = root(tag);
    write(&r, "lib/lib.hl", LIB);
    write(&r, "apps/api/main.hl", APP);
    write(&r, "hale.toml", "[deps]\n");
    r
}

#[test]
fn every_provenance_span_names_a_source_file() {
    let r = workspace("resolve");
    let out = dump_from(&r, "apps/api");
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("artifact parses");
    let _ = std::fs::remove_dir_all(&r);

    let sources = v["sources"].as_array().expect("sources table");
    assert!(!sources.is_empty(), "a source table is required: {}", out);
    let ids: Vec<i64> =
        sources.iter().map(|s| s["id"].as_i64().unwrap_or(-1)).collect();

    // Every decl row must point at a real entry — a span that cannot
    // be placed is reported as -1 rather than attributed to the wrong
    // file, and none of these should be unplaceable.
    let decls = v["provenance"]["decls"].as_object().expect("decls");
    assert!(!decls.is_empty(), "fixture must declare something");
    for (name, row) in decls {
        let sid = row["source"].as_i64().expect("source id");
        assert!(
            ids.contains(&sid),
            "decl `{}` names source {}, which is not in the table",
            name,
            sid
        );
        let span = row["span"].as_array().expect("span");
        assert_eq!(span.len(), 2, "decl `{}` span", name);
    }
}

/// Spans are now file-LOCAL. A bundle-global offset would exceed the
/// length of the file it claims to be in, which is exactly what made
/// them useless to a consumer.
#[test]
fn every_provenance_span_is_local_at_both_ends() {
    let r = workspace("local_all");
    let out = dump_from(&r, "apps/api");
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("artifact parses");

    let lens: Vec<usize> = v["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|s| {
            let p = r.join(s["path"].as_str().expect("path"));
            std::fs::read_to_string(&p).map(|t| t.len()).unwrap_or(0)
        })
        .collect();
    let _ = std::fs::remove_dir_all(&r);

    // (label, source id, start, end) for every provenance row there
    // is — not just declarations.
    let mut rows: Vec<(String, usize, usize, usize)> = Vec::new();
    let mut push = |label: String, row: &serde_json::Value| {
        rows.push((
            label,
            row["source"].as_i64().expect("source id") as usize,
            row["span"][0].as_u64().expect("start") as usize,
            row["span"][1].as_u64().expect("end") as usize,
        ));
    };
    for sect in ["calls", "publishes", "subscribes"] {
        for (i, row) in v["provenance"][sect]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .enumerate()
        {
            push(format!("{sect}[{i}]"), row);
        }
    }
    for (name, row) in
        v["provenance"]["decls"].as_object().expect("decls")
    {
        push(format!("decl `{name}`"), row);
    }

    assert!(
        rows.iter().any(|(l, ..)| l.starts_with("publishes")),
        "the fixture must contain a publish, or this test cannot see \
         the section it was written for"
    );
    // The publishing locus lives in the LIBRARY, which is not the
    // first source — so its virtual base is nonzero and a
    // bundle-global offset is distinguishable from a file-local one.
    // With the publish in source 0 the two coincide and the bug is
    // invisible, which is how it survived.
    assert!(
        rows.iter()
            .any(|(l, sid, ..)| l.starts_with("publishes") && *sid != 0),
        "the publish must come from a source with a nonzero base"
    );

    for (label, sid, start, end) in &rows {
        let len = lens[*sid];
        assert!(
            *start <= *end && *end <= len,
            "{label}: span [{start}, {end}] in source {sid}, which is \
             only {len} bytes. A row that names a file must resolve \
             INSIDE it at BOTH ends — a file-local start with a \
             bundle-global end is not a coordinate in any single \
             system, and a consumer following it lands outside the \
             file it was told to open"
        );
    }
}

#[test]
fn spans_are_local_to_their_file() {
    let r = workspace("local");
    let out = dump_from(&r, "apps/api");
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("artifact parses");

    let lens: Vec<usize> = v["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|s| {
            let p = r.join(s["path"].as_str().expect("path"));
            std::fs::read_to_string(&p).map(|t| t.len()).unwrap_or(0)
        })
        .collect();
    let _ = std::fs::remove_dir_all(&r);

    for (name, row) in v["provenance"]["decls"].as_object().expect("decls") {
        let sid = row["source"].as_i64().expect("id") as usize;
        let end = row["span"][1].as_u64().expect("end") as usize;
        assert!(
            end <= lens[sid],
            "decl `{}` ends at {} but source {} is only {} bytes — that \
             is a bundle-global offset, not a file-local one",
            name,
            end,
            sid,
            lens[sid]
        );
    }
}

/// The artifact must not depend on where it was produced from. Paths
/// are relative to the workspace (the nearest `hale.toml`), and are
/// canonicalized first — without that the target's own file kept the
/// path as typed on the command line while imported seeds arrived
/// absolute, so the same sources produced two different documents.
#[test]
fn the_artifact_is_reproducible_across_working_directories() {
    let r = workspace("repro");
    let from_root = dump_from(&r, "apps/api");
    let from_apps = dump_from(&r.join("apps"), "api");
    let from_tmp =
        dump_from(Path::new("/"), r.join("apps/api").to_str().expect("utf8"));
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(
        from_root, from_apps,
        "checking from the workspace root and from `apps/` must produce \
         one artifact"
    );
    assert_eq!(
        from_apps, from_tmp,
        "…and so must an absolute target from an unrelated directory"
    );
}

/// Absolute paths would make the artifact machine-specific, which
/// defeats comparing two of them.
#[test]
fn source_paths_are_workspace_relative() {
    let r = workspace("relative");
    let out = dump_from(&r, "apps/api");
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("artifact parses");
    let _ = std::fs::remove_dir_all(&r);

    let paths: Vec<&str> = v["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|s| s["path"].as_str().unwrap_or(""))
        .collect();
    for p in &paths {
        assert!(
            !p.starts_with('/') && !p.contains(".."),
            "`{}` is not workspace-relative — an absolute path makes the \
             artifact differ per machine: {:?}",
            p,
            paths
        );
    }
    // The imported seed lives OUTSIDE the target directory, which is
    // the case that kept paths absolute before the workspace root.
    assert!(
        paths.iter().any(|p| p.starts_with("lib/")),
        "the imported seed must be relativized too: {:?}",
        paths
    );
}

fn source_paths(artifact: &str) -> Vec<String> {
    let v: serde_json::Value =
        serde_json::from_str(artifact).expect("artifact parses");
    v["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|s| s["path"].as_str().unwrap_or("").to_string())
        .collect()
}

/// An application with its own `hale.toml` importing a library beside
/// it, above the manifest's directory.
const APP_ABOVE: &str = r#"
import "../lib" as lb;
main locus App { params { b: lb::Billing = lb::Billing { }; } }
fn main() { App { }; }
"#;

fn app_importing_above(r: &Path) {
    write(r, "lib/lib.hl", LIB);
    write(r, "app/main.hl", APP_ABOVE);
    write(r, "app/hale.toml", "[deps]\n");
}

/// F.40 phase 4, I3: a file outside the `hale.toml` root is named
/// relative to the deepest common ancestor of every source, as every
/// other file of the program is. It used to keep its absolute path, so
/// the artifact differed from machine to machine.
#[test]
fn an_import_above_the_manifest_root_is_named_relatively() {
    let r = root("above");
    app_importing_above(&r);
    let paths = source_paths(&dump_from(&r, "app/main.hl"));
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(
        paths,
        ["app/main.hl", "lib/lib.hl"],
        "every file under one root, the root being the common ancestor \
         when a file sits outside the `hale.toml` directory"
    );
}

/// F.40 phase 4, I3: one tree checked out at two places is one artifact,
/// byte for byte, whether or not a file sits outside its `hale.toml`
/// root. The uncovered change that keeps `artifact_digest`: the
/// absolute root, which no section names.
#[test]
fn one_tree_checked_out_at_two_roots_has_one_artifact() {
    let scratch = root("two_roots");
    let (a, b) = (scratch.join("one"), scratch.join("two/deeper"));
    for r in [&a, &b] {
        app_importing_above(&r.join("above"));
        write(&r.join("under"), "lib/lib.hl", LIB);
        write(&r.join("under"), "apps/api/main.hl", APP);
        write(&r.join("under"), "hale.toml", "[deps]\n");
    }
    let dumps = |r: &Path| {
        (
            dump_from(r, "above/app/main.hl"),
            dump_from(r, "under/apps/api"),
        )
    };
    let (da, db) = (dumps(&a), dumps(&b));
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(da.0.contains("\"sources\""), "an artifact: {}", da.0);
    assert!(da.0.contains("\"artifact_digest\""), "a digested artifact: {}", da.0);
    assert_eq!(
        da.0, db.0,
        "an import above the manifest root: one artifact at two roots"
    );
    assert_eq!(da.1, db.1, "a program under its root: one artifact at two roots");
}

/// F.40 phase 4, I3: a program whose files are all under its `hale.toml`
/// root keeps the paths it had, and so does a program with no manifest,
/// however its target is typed.
#[test]
fn a_program_under_its_root_keeps_its_paths() {
    let r = workspace("kept");
    let under = source_paths(&dump_from(&r, "apps/api"));
    write(&r, "loose/solo/main.hl", "fn main() { let x = 1 + 1; }\n");
    std::fs::remove_file(r.join("hale.toml")).expect("rm manifest");
    let relative = source_paths(&dump_from(&r, "loose/solo/main.hl"));
    let absolute = source_paths(&dump_from(
        Path::new("/"),
        r.join("loose/solo/main.hl").to_str().expect("utf8"),
    ));
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(under, ["apps/api/main.hl", "lib/lib.hl"]);
    assert_eq!(relative, ["main.hl"], "no manifest, a relative target");
    assert_eq!(absolute, ["main.hl"], "no manifest, an absolute target");
}

/// A content digest lets a consumer tell whether two artifacts were
/// built from the same text, and catches a stale artifact paired with
/// edited source — without shipping the source. `source_digest` moves
/// with its file's text and keeps its value when the same text is
/// named by another path.
#[test]
fn a_source_digest_tracks_its_contents() {
    let r = workspace("digest");
    let before = dump_from(&r, "apps/api");
    write(&r, "lib/lib.hl", &format!("{}\n// a comment\n", LIB));
    let after = dump_from(&r, "apps/api");
    std::fs::rename(r.join("apps/api"), r.join("apps/web")).expect("move the app");
    let moved = dump_from(&r, "apps/web");
    let _ = std::fs::remove_dir_all(&r);

    let dig = |s: &str| -> std::collections::BTreeMap<String, String> {
        let v: serde_json::Value = serde_json::from_str(s).expect("parses");
        v["sources"]
            .as_array()
            .expect("sources")
            .iter()
            .map(|x| {
                (
                    x["path"].as_str().unwrap_or("").to_string(),
                    x["digest"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect()
    };
    let (before, after, moved) = (dig(&before), dig(&after), dig(&moved));
    assert_ne!(
        before["lib/lib.hl"], after["lib/lib.hl"],
        "editing a source must change its digest"
    );
    assert_eq!(
        before["apps/api/main.hl"], moved["apps/web/main.hl"],
        "the same text under another path keeps its digest"
    );
}

/// `schema` says a row has these fields; it cannot say what they
/// MEAN. Two compilers agreeing on the schema and disagreeing on the
/// semantics would compose artifacts into a model neither would
/// certify, with nothing in the document revealing it.
#[test]
fn the_artifact_declares_its_model_semantics() {
    let r = workspace("semantics");
    let out = dump_from(&r, "apps/api");
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("artifact parses");
    let _ = std::fs::remove_dir_all(&r);
    assert!(
        v["semantics"].as_u64().is_some(),
        "a semantics version, distinct from `schema`: {}",
        out
    );
}
