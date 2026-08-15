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
    assert!(
        mmd.contains("n_Store_3a_3aon_5fc"),
        "stores member highlighted"
    );
    assert!(
        mmd.contains("n_Worker_3a_3aon_5fr"),
        "workers member highlighted"
    );
    assert!(
        mmd.contains("claim apart — holds"),
        "card carries name + verdict:\n{}",
        mmd
    );
    // P2 (review round 1): the claim view must NOT hide residue —
    // the pinned fixture has an indirect_call hole, and the very
    // view most likely to become a proof slide says so.
    assert!(
        mmd.contains("1 unresolved"),
        "claim view carries the residue card:\n{}",
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
    // Focus keeps Worker plus its edge-neighbors...
    assert!(mmd.contains("n_Worker_3a_3aon_5fr"));
    assert!(mmd.contains("t_Readings"), "subscribed topic kept");
    // ...and the config title replaced the default.
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

// -----------------------------------------------------------------
// Review round 1 — adapter and identity coverage.
// -----------------------------------------------------------------

/// FNV-1a 64 (the artifact's own tripwire algorithm) — used to craft
/// a digest-valid artifact with hostile CONTENT, so the admission
/// checks past the digest can be exercised.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Re-stamp a (possibly edited) artifact with a fresh valid digest,
/// mirroring the emitter's trailer format.
fn restamp_digest(body_without_trailer: &str) -> String {
    format!(
        "{},\n  \"artifact_digest\": \"{:016x}\"\n}}\n",
        body_without_trailer,
        fnv1a64(body_without_trailer.as_bytes())
    )
}

fn strip_trailer(artifact: &str) -> String {
    let key = ",\n  \"artifact_digest\": \"";
    let i = artifact.rfind(key).expect("artifact has a digest trailer");
    artifact[..i].to_string()
}

#[test]
fn tampered_or_unverifiable_artifacts_are_refused() {
    let dir = workdir("admission");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    let raw = std::fs::read_to_string(&artifact).unwrap();

    // 1. Hand-edited claims section under the ORIGINAL digest: the
    //    exact attack the review named — refused as tampered.
    let edited = raw.replace("\"result\": \"holds\"", "\"result\": \"violated\"");
    assert_ne!(raw, edited, "test premise: the edit landed");
    let tampered = dir.join("tampered.topology");
    std::fs::write(&tampered, &edited).unwrap();
    let out = hale().arg("topology").arg("graph").arg(&tampered).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("artifact_digest"),
        "refusal names the digest"
    );

    // 2. No digest at all (pre-1.3 artifact): refused as unverifiable.
    let undigested = dir.join("undigested.topology");
    std::fs::write(&undigested, strip_trailer(&raw) + "\n}\n").unwrap();
    let out = hale().arg("topology").arg("graph").arg(&undigested).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("artifact_digest"),
        "refusal names the missing digest"
    );

    // 3. Valid digest, WRONG SEMANTICS: rows may mean different
    //    things; refused past the integrity gate.
    let body = strip_trailer(&raw).replace("\"semantics\": 1", "\"semantics\": 999");
    let wrong_sem = dir.join("semantics.topology");
    std::fs::write(&wrong_sem, restamp_digest(&body)).unwrap();
    let out = hale().arg("topology").arg("graph").arg(&wrong_sem).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("semantics"),
        "refusal names the semantics mismatch: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 4. Valid digest, unsupported OLD schema minor (pre-topics /
    //    pre-verdict): refused rather than rendered misleadingly
    //    empty.
    let body = strip_trailer(&raw).replace("\"schema\": \"1.10\"", "\"schema\": \"1.3\"");
    let old_schema = dir.join("oldschema.topology");
    std::fs::write(&old_schema, restamp_digest(&body)).unwrap();
    let out = hale().arg("topology").arg("graph").arg(&old_schema).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("schema"),
        "refusal names the unsupported schema"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Imported locus methods (`p::Store::get`) and imported free fns
/// (`p::helper`) must land in the right boxes — membership by
/// longest declared-locus prefix, never by the first `::`.
#[test]
fn imported_members_keep_their_owners() {
    let dir = workdir("xseed");
    let app = dir.join("app");
    let lib = dir.join("lib/kv");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(dir.join("hale.toml"), "name = \"xseed-graph\"\n").unwrap();
    std::fs::write(
        lib.join("kv.hl"),
        r#"
type Pair { k: Int = 0; v: Int = 0; }
locus Store {
    params { total: Int = 0; }
    fn get(k: Int) -> Int { return self.total + k; }
}
fn helper(v: Int) -> Int { return v + 1; }
"#,
    )
    .unwrap();
    std::fs::write(
        app.join("main.hl"),
        r#"
import "lib/kv" as p;
main locus App {
    params { s: p::Store = p::Store { }; }
    run() {
        let a = self.s.get(1);
        let b = p::helper(a);
        println(b);
    }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dir.join("x.topology");
    let out = hale()
        .arg("check")
        .arg(&app)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cross-seed fixture must check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mmd = render(&artifact, &["--format", "mermaid"]);
    // The imported locus box exists and OWNS its method (the method
    // node appears inside the subgraph, labeled by its short name).
    assert!(
        mmd.contains("subgraph g_p_3a_3aStore"),
        "imported locus box exists:\n{}",
        mmd
    );
    assert!(
        mmd.contains("n_p_3a_3aStore_3a_3aget[\"get"),
        "imported method placed in its locus, short-labeled:\n{}",
        mmd
    );
    // The imported free fn is rendered as free — present, not
    // orphaned into a phantom `p` locus.
    assert!(
        mmd.contains("n_p_3a_3ahelper"),
        "imported free fn is rendered:\n{}",
        mmd
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Literal and wildcard bus subjects have no declared-topic row but
/// are real endpoints: they need visible nodes and edges in every
/// format.
#[test]
fn literal_and_wildcard_subjects_render() {
    let dir = workdir("litsubj");
    let src = dir.join("lit.hl");
    std::fs::write(
        &src,
        r#"
type Event { n: Int = 0; }
locus Audit {
    params { seen: Int = 0; }
    bus { subscribe "orders.**" as on_any of type Event; }
    fn on_any(e: Event) { self.seen = self.seen + 1; }
}
main locus App {
    params { a: Audit = Audit { }; }
    bus { publish "orders.created" of type Event; }
    run() { "orders.created" <- Event { n: 1 }; }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dir.join("lit.topology");
    let out = hale()
        .arg("check")
        .arg(&src)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "literal-subject fixture must check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    for format in ["mermaid", "dot", "svg"] {
        let rendered = render(&artifact, &["--format", format]);
        assert!(
            rendered.contains("orders.created"),
            "{}: literal subject node present:\n{}",
            format,
            rendered
        );
        assert!(
            rendered.contains("orders.**"),
            "{}: wildcard subject node present:\n{}",
            format,
            rendered
        );
        assert!(
            rendered.contains("publish") && rendered.contains("subscribe"),
            "{}: both bus edges survive:\n{}",
            format,
            rendered
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Mermaid-ID collision canary: `A::B` (method), `A__B` (free
/// fn), and a fn named like a prefixed topic must all get distinct
/// injective IDs.
#[test]
fn mermaid_ids_are_injective() {
    let dir = workdir("collide");
    let src = dir.join("collide.hl");
    std::fs::write(
        &src,
        r#"
fn A__B(v: Int) -> Int { return v; }
locus A {
    fn B(v: Int) -> Int { return v + 1; }
}
main locus App {
    params { a: A = A { }; }
    run() { println(self.a.B(A__B(1))); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dir.join("collide.topology");
    let out = hale()
        .arg("check")
        .arg(&src)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "collision fixture must check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mmd = render(&artifact, &["--format", "mermaid"]);
    // Method A::B → n_A_3a_3aB; free fn A__B → n_A_5f_5fB. Distinct.
    assert!(mmd.contains("n_A_3a_3aB"), "method id present:\n{}", mmd);
    assert!(mmd.contains("n_A_5f_5fB"), "free fn id present:\n{}", mmd);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Focus must actually DROP non-neighbors: focusing Store keeps its
/// subscribed topic (Cmds) and drops App and Readings, which share
/// no edge with a focused endpoint.
#[test]
fn focus_drops_non_neighbors() {
    let dir = workdir("focusdrop");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    let cfg = dir.join("focus-store.json");
    std::fs::write(&cfg, r#"{ "focus": ["Store"] }"#).unwrap();
    let mmd = render(
        &artifact,
        &["--format", "mermaid", "--config", cfg.to_str().unwrap()],
    );
    assert!(mmd.contains("g_Store"), "focused box kept:\n{}", mmd);
    assert!(
        mmd.contains("t_Cmds"),
        "edge-neighbor topic kept:\n{}",
        mmd
    );
    assert!(
        !mmd.contains("g_App"),
        "App shares no edge with Store — dropped:\n{}",
        mmd
    );
    assert!(
        !mmd.contains("t_Readings"),
        "Readings shares no edge with Store — dropped:\n{}",
        mmd
    );
    let _ = std::fs::remove_dir_all(&dir);
}
