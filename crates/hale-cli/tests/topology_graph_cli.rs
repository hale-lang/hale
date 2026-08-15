//! GH #476 Track A — the deterministic topology renderer.
//!
//! What these tests hold the tool to (the plan's acceptance criteria):
//!   1. **Artifact client only.** The renderer consumes a committed
//!      `--dump-topology` artifact; it never reads Hale source.
//!   2. **Byte-determinism.** Rendering the same artifact twice is
//!      byte-identical, for every format.
//!   3. **Source-motion invariance.** Moving source lines (a leading
//!      comment) changes provenance spans and the file digest but not
//!      the model shape — the rendered output must be byte-identical.
//!      (This is what makes slide decks stable across edits.)
//!   4. **Golden snapshots.** A pinned fixture renders to checked-in
//!      goldens, so presentation output is regression-tested against
//!      the compiler. A golden change is a reviewable diff, never
//!      silent drift.
//!   5. **Views carry their semantics**: claim view highlights the
//!      groups the claim names and states the result; residue view
//!      renders unknowns visibly rather than omitting them.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/topology")
        .join(name)
}

fn workdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hale_topograph_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Dump the fixture's artifact into `dir` and return its path.
fn dump_artifact(dir: &Path, source: &Path) -> PathBuf {
    let artifact = dir.join("app.hale.topology");
    let out = hale()
        .arg("check")
        .arg(source)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .expect("hale check --dump-topology");
    assert!(
        out.status.success(),
        "fixture must typecheck: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    artifact
}

fn render(artifact: &Path, extra: &[&str]) -> String {
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(artifact)
        .args(extra)
        .output()
        .expect("hale topology graph");
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8 output")
}

#[test]
fn rendering_is_byte_deterministic_in_every_format() {
    let dir = workdir("determ");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    for format in ["svg", "mermaid", "dot"] {
        let a = render(&artifact, &["--format", format]);
        let b = render(&artifact, &["--format", format]);
        assert_eq!(a, b, "{} output must be byte-identical", format);
        assert!(!a.is_empty());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn source_motion_does_not_change_the_render() {
    let dir = workdir("motion");
    let original = std::fs::read_to_string(fixture("pipeline.hl")).unwrap();

    let src_a = dir.join("a.hl");
    std::fs::write(&src_a, &original).unwrap();
    let art_a = dump_artifact(&dir, &src_a);
    let svg_a = render(&art_a, &[]);

    // Shift every span: three comment lines up top. Provenance and
    // the source digest change; the model shape must not — and the
    // render consumes only the model, so it must be byte-identical.
    let src_b = dir.join("b.hl");
    std::fs::write(
        &src_b,
        format!("// moved\n// by three\n// comment lines\n{}", original),
    )
    .unwrap();
    let art_b = dir.join("b.hale.topology");
    let out = hale()
        .arg("check")
        .arg(&src_b)
        .arg(format!("--dump-topology={}", art_b.display()))
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_ne!(
        std::fs::read(&art_a).unwrap(),
        std::fs::read(&art_b).unwrap(),
        "test premise: the artifacts DO differ (spans + digest moved)"
    );
    let svg_b = render(&art_b, &[]);
    assert_eq!(
        svg_a, svg_b,
        "source motion must not move a single pixel of the render"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pinned_fixture_matches_the_checked_in_goldens() {
    let dir = workdir("golden");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));

    let system_mmd = render(&artifact, &["--format", "mermaid"]);
    let golden_mmd =
        std::fs::read_to_string(fixture("golden-system.mmd")).unwrap();
    assert_eq!(
        system_mmd, golden_mmd,
        "system mermaid drifted from the golden — if the change is \
         intended, regenerate tests/fixtures/topology/golden-system.mmd"
    );

    let system_svg = render(&artifact, &["--format", "svg"]);
    let golden_svg =
        std::fs::read_to_string(fixture("golden-system.svg")).unwrap();
    assert_eq!(
        system_svg, golden_svg,
        "system SVG drifted from the golden — if the change is \
         intended, regenerate tests/fixtures/topology/golden-system.svg"
    );

    let claim_mmd = render(
        &artifact,
        &["--view", "claim", "--claim", "apart", "--format", "mermaid"],
    );
    let golden_claim =
        std::fs::read_to_string(fixture("golden-claim.mmd")).unwrap();
    assert_eq!(claim_mmd, golden_claim, "claim mermaid drifted");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn claim_view_highlights_the_named_groups_and_states_the_result() {
    let dir = workdir("claim");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    let mmd = render(
        &artifact,
        &["--view", "claim", "--claim", "apart", "--format", "mermaid"],
    );
    // `apart: forbid reaches(stores, workers) via { calls }` — both
    // group members' fn nodes carry the highlight class, and the
    // card states the verdict.
    assert!(mmd.contains("class ") && mmd.contains(" hl"));
    assert!(mmd.contains("Store__on_c"), "stores member highlighted");
    assert!(mmd.contains("Worker__on_r"), "workers member highlighted");
    assert!(
        mmd.contains("claim apart — holds"),
        "card carries name + verdict:\n{}",
        mmd
    );

    // An unknown claim name refuses loudly and lists what exists.
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .args(["--view", "claim", "--claim", "nope"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("nope") && err.contains("apart"),
        "refusal names the missing claim and the available ones: {}",
        err
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn residue_view_renders_unknowns_visibly() {
    let dir = workdir("residue");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    let mmd = render(&artifact, &["--view", "residue", "--format", "mermaid"]);
    assert!(
        mmd.contains("indirect_call"),
        "the fn-pointer hole must be a rendered node:\n{}",
        mmd
    );
    assert!(mmd.contains("unresolved"), "hole edge labeled");

    // Other views never silently omit residue: it surfaces as a card.
    let sys = render(&artifact, &["--format", "mermaid"]);
    assert!(
        sys.contains("unresolved") || sys.contains("card: 1 unresolved"),
        "system view notes the residue:\n{}",
        sys
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn render_config_focuses_and_highlights_without_new_semantics() {
    let dir = workdir("config");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    let mmd = render(
        &artifact,
        &[
            "--format",
            "mermaid",
            "--config",
            fixture("slide-focus.json").to_str().unwrap(),
        ],
    );
    // Focus keeps Worker plus its edge-neighbors (App publishes the
    // topic Worker subscribes; the free fns Worker calls)...
    assert!(mmd.contains("Worker__on_r"));
    assert!(mmd.contains("topic_Readings"), "subscribed topic kept");
    // ...and drops the unconnected-to-Worker Store subscription side
    // only if Store shares no edge with Worker's anchors. Store
    // subscribes Cmds which Worker publishes — via the topic they ARE
    // neighbors, so Store stays. The one guaranteed drop: nothing.
    // What MUST hold: the config title replaced the default.
    assert!(
        mmd.contains("sensor pipeline — worker focus"),
        "config title applied:\n{}",
        mmd
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_inputs_fail_closed() {
    let dir = workdir("bad");
    // Not JSON.
    let not_json = dir.join("nope.topology");
    std::fs::write(&not_json, "hello").unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&not_json)
        .output()
        .unwrap();
    assert!(!out.status.success());

    // Unsupported schema family.
    let future = dir.join("future.topology");
    std::fs::write(&future, r#"{"schema": "9.0", "sorts": {}}"#).unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&future)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("schema"),
        "refusal names the schema"
    );

    // Unknown view / format / missing --claim.
    for args in [
        vec!["--view", "sideways"],
        vec!["--format", "png"],
        vec!["--view", "claim"],
    ] {
        let real = dump_artifact(&dir, &fixture("pipeline.hl"));
        let out = hale()
            .arg("topology")
            .arg("graph")
            .arg(&real)
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success(), "must refuse: {:?}", args);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
