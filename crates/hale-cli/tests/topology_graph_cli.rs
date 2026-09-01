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
/// Dump an artifact from a program whose only errors are CLAIM
/// errors. A claim error still emits the artifact — the verdict is
/// the thing being recorded — so a law the model refuses to certify
/// is exactly the case this is for.
fn dump_artifact_with_law_errors(
    dir: &Path,
    source: &Path,
) -> (PathBuf, String) {
    let artifact = dir.join("app.hale.topology");
    let out = hale()
        .arg("check")
        .arg(source)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .expect("hale check --dump-topology");
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        artifact.exists(),
        "a claim error still emits the artifact: {}",
        err
    );
    (artifact, err)
}

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
    // Round 5 (#490): the canonical-layout gate runs first, so
    // the minimal crafted document refuses on its layout (no
    // final artifact_digest) — either refusal is fail-closed.
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("schema")
            || err.contains("artifact_digest must be the final"),
        "refusal names the schema or the layout: {}",
        err
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

/// Re-stamp a (possibly edited) artifact with fresh valid digests,
/// mirroring the emitter: `law_digest` recomputes from the
/// canonical-JSON law rows (round 7), then the whole-document
/// trailer. Tamper pins restamp BOTH so each exercises the deep
/// binding it targets, not the digest gate.
fn restamp_digest(body_without_trailer: &str) -> String {
    let mut body = body_without_trailer.to_string();
    // Round 2 (#490): the shape_hash is recomputed at admission,
    // so hashed-half tampers must restamp it too — each pin then
    // exercises the deep binding it targets, not the identity
    // gate. (The stale-identity control below does NOT use this
    // helper.)
    let sk = "\"shape_hash\": \"";
    if let Some(at) = body.find(sk) {
        let start = at + sk.len();
        if let Some(rel) = body[start..].find('"') {
            let claimed_end = start + rel;
            let model_start =
                claimed_end + "\",\n".len();
            if let Some(end_rel) =
                body[model_start..].find(",\n  \"sources\": [")
            {
                let fresh = format!(
                    "{:016x}",
                    fnv1a64(
                        body[model_start
                            ..model_start + end_rel]
                            .as_bytes()
                    )
                );
                body.replace_range(start..claimed_end, &fresh);
            }
        }
    }
    let key = "\"law_digest\": \"";
    if let (Some(at), Ok(v)) = (
        body.find(key),
        serde_json::from_str::<serde_json::Value>(&format!(
            "{}\n}}\n",
            body
        )),
    ) {
        if v["law"]["rows"].is_array() {
            let canon = serde_json::to_string(&serde_json::json!({
                "issues": v["law"]["issues"],
                "rows": v["law"]["rows"],
            }))
            .unwrap();
            let fresh =
                format!("{:016x}", fnv1a64(canon.as_bytes()));
            let start = at + key.len();
            let end = start
                + body[start..].find('"').expect("digest close");
            body.replace_range(start..end, &fresh);
        }
    }
    format!(
        "{},\n  \"artifact_digest\": \"{:016x}\"\n}}\n",
        body,
        fnv1a64(body.as_bytes())
    )
}

/// Remove one JSON row that STARTS with `prefix` (through its
/// closing `}, ` or `}`) — endpoint rows carry provenance tails,
/// so pins match on the stable identity prefix.
fn cut_row(raw: &str, prefix: &str) -> String {
    let at = raw
        .find(prefix)
        .unwrap_or_else(|| panic!("row prefix present: {}", prefix));
    let close = at
        + raw[at..].find('}').expect("row closes")
        + 1;
    let close = if raw[close..].starts_with(", ") {
        close + 2
    } else {
        close
    };
    format!("{}{}", &raw[..at], &raw[close..])
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
    let sem_key = format!(
        "\"semantics\": {}",
        hale_types::topology::MODEL_SEMANTICS
    );
    assert!(raw.contains(&sem_key), "test premise: current semantics");
    let body =
        strip_trailer(&raw).replace(&sem_key, "\"semantics\": 999");
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
    let schema_key = format!(
        "\"schema\": \"{}\"",
        hale_types::topology::TOPOLOGY_SCHEMA
    );
    assert!(raw.contains(&schema_key), "test premise: current schema");
    let body =
        strip_trailer(&raw).replace(&schema_key, "\"schema\": \"1.3\"");
    let old_schema = dir.join("oldschema.topology");
    std::fs::write(&old_schema, restamp_digest(&body)).unwrap();
    let out = hale().arg("topology").arg("graph").arg(&old_schema).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("schema"),
        "refusal names the unsupported schema"
    );

    // 5. Valid digest, DELETED law section (round 2): the claim
    //    view consumes it — a restamped artifact without it must
    //    refuse, never render with silently absent highlights.
    let start = raw.find(",\n  \"law\": {").expect("law section");
    let end = raw
        .find(",\n  \"capabilities\": {")
        .expect("capabilities section");
    let mut lawless = String::new();
    lawless.push_str(&raw[..start]);
    lawless.push_str(&raw[end..]);
    let no_law = dir.join("nolaw.topology");
    std::fs::write(&no_law, restamp_digest(&strip_trailer(&lawless)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&no_law)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a restamped artifact without `law` must refuse"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("law"),
        "the refusal names the missing section: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 6. Round 3: a REAL law replaced by a bare tagged object —
    //    the payload's kind is recognized but its operands are
    //    missing; refused, never rendered without highlights.
    let start =
        raw.find("\"rows\": [\n").expect("law rows") + "\"rows\": [\n".len();
    let row_end = raw[start..]
        .find("\n")
        .map(|i| start + i)
        .expect("first law row line");
    let first_row = &raw[start..row_end];
    let gutted = first_row
        .split("\"law\": {")
        .next()
        .unwrap()
        .to_string()
        + "\"law\": {\"kind\": \"forbid_reaches\"}},";
    let mut bare = String::new();
    bare.push_str(&raw[..start]);
    bare.push_str(&gutted);
    bare.push_str(&raw[row_end..]);
    let bare_path = dir.join("barelaw.topology");
    std::fs::write(
        &bare_path,
        restamp_digest(&strip_trailer(&bare)),
    )
    .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&bare_path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a bare tagged payload must refuse"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("incomplete law payload"),
        "the refusal names the payload: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 7. Round 3: a duplicate same-name law row inserted ahead of
    //    the real one — the contiguous-ordinal law refuses it, so
    //    a masquerading row can never win a name join (which the
    //    renderer no longer performs anyway: it joins by ordinal).
    let dup = {
        let start = raw.find("\"rows\": [\n").expect("law rows")
            + "\"rows\": [\n".len();
        let row_end = raw[start..]
            .find("\n")
            .map(|i| start + i)
            .expect("first row line");
        let first_row = raw[start..row_end].trim_end_matches(',');
        let mut d = String::new();
        d.push_str(&raw[..start]);
        d.push_str(&format!("{},\n", first_row));
        d.push_str(&raw[start..]);
        d
    };
    let dup_path = dir.join("duplaw.topology");
    std::fs::write(
        &dup_path,
        restamp_digest(&strip_trailer(&dup)),
    )
    .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&dup_path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a duplicated law row must refuse"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("contiguous"),
        "the refusal names the broken sequence: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 8. Round 4: an OPERAND SWAP — the typed payload's groups are
    //    replaced while the compatibility form string is kept.
    //    Both referenced groups exist, name and verdict are
    //    unchanged; the canonical re-render is what refuses.
    let swapped = raw.replacen(
        "\"src\": {\"group\": {\"name\": \"stores\", \"display\": \"stores\"",
        "\"src\": {\"group\": {\"name\": \"workers\", \"display\": \"workers\"",
        1,
    );
    assert_ne!(swapped, raw, "test premise: the swap landed");
    {
        let sw = dir.join("swapped.topology");
        std::fs::write(
            &sw,
            restamp_digest(&strip_trailer(&swapped)),
        )
        .unwrap();
        let out = hale()
            .arg("topology")
            .arg("graph")
            .arg(&sw)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "an operand swap under an unchanged form must refuse"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("does not render from the typed law"),
            "the refusal names the form binding: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // 9. Round 4: a CLEAN document verdict over a non-holds law —
    //    the verdict is recomputed during admission, never
    //    trusted.
    let lying = {
        let s = raw.replacen(
            "\"verdict\": \"holds\"",
            "\"verdict\": \"violated\"",
            1,
        );
        assert_ne!(s, raw, "test premise: a law verdict flipped");
        s
    };
    // (the claims row's result must flip too, or the 1:1 check
    // refuses before the verdict recompute — flip it as well so
    // the recompute is what bites)
    let lying = lying.replacen(
        "\"result\": \"holds\", \"ordinal\"",
        "\"result\": \"violated\", \"ordinal\"",
        1,
    );
    let ly = dir.join("lying.topology");
    std::fs::write(&ly, restamp_digest(&strip_trailer(&lying)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&ly)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a clean verdict over a non-holds law must refuse"
    );
    // Round 7 refuses even earlier: a `violated` verdict without
    // its countermodel evidence is not admissible, so the lie
    // fails for lacking the evidence it never had. Either way the
    // law account speaks, not the digest.
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("disagrees with its own law rows")
            || err.contains("retains none of its judgment's"),
        "the refusal names the recompute or the missing          evidence: {}",
        err
    );

    // 10. Round 5: flipping `resolved` to false to dodge the
    //     existence check — a holds row cannot carry an
    //     unresolved operand.
    let unres = raw.replacen(
        "\"name\": \"stores\", \"display\": \"stores\", \"resolved\": true",
        "\"name\": \"ghost_x\", \"display\": \"stores\", \"resolved\": false",
        1,
    );
    assert_ne!(unres, raw, "test premise: the flip landed");
    let p3 = dir.join("unresolved.topology");
    std::fs::write(
        &p3,
        restamp_digest(&strip_trailer(&unres)),
    )
    .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p3)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("only `invalid` is truthful"),
        "the resolution↔verdict binding refuses: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 5: a module-scoped annotation subject resolves against
/// the FULL fn universe (`law.fn_universe`), wider than the legacy
/// `sorts.fns` — the artifact must load through Track A.
#[test]
fn module_scoped_annotation_subject_admits() {
    let dir = workdir("modscope");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
effect money;
module billing {
    @effects(causes: { money })
    fn poke(v: Int) -> Int { return v; }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    // Round 3: a module-scoped subject is outside the
    // analyzable universe, so `causes:` over it is UNCERTIFIED and
    // says so — check is no longer silent about a law it could not
    // certify. The artifact is still emitted and must still admit.
    let (artifact, err) = dump_artifact_with_law_errors(&dir, &src);
    assert!(
        err.contains("outside the analyzable universe")
            || err.contains("cannot be certified"),
        "check explains the refusal: {}",
        err
    );
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .arg("--format")
        .arg("mermaid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "a compiler-emitted artifact with a module-scoped          annotation subject must admit: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 5: an annotation-operand swap — the law's forbidden
/// (`@effects(none:)`) class changes while the certificates keep their original
/// forms. The evidence binding must refuse: a `verdict` is only
/// admissible when the certificates it cites are certificates
/// FOR this law.
#[test]
fn annotation_class_swap_is_refused() {
    let dir = workdir("classswap");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
@effects(none: { block })
fn pure_math(v: Int) -> Int { return v + 1; }
main locus App {
    params { n: Int = 0; }
    run() { println(pure_math(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let cls = raw.replacen("\"class\": \"block\"", "\"class\": \"alloc\"", 1);
    assert_ne!(cls, raw, "test premise: the swap landed");
    let p2 = dir.join("swapped.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&cls))).unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("does not match its typed law")
            || err.contains("certificate"),
        "the certificate↔law binding refuses the swap: {}",
        err
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

/// Round 2: a digest-valid artifact with a MALFORMED relation row
/// (numeric `from`) must be refused naming the row — never rendered
/// with the edge silently absent. The digest is a tripwire, not an
/// authenticator; row shape is validated on its own.
#[test]
fn malformed_rows_are_refused_not_dropped() {
    let dir = workdir("badrow");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    let raw = std::fs::read_to_string(&artifact).unwrap();

    // Corrupt one call row's `from` into a number, restamp so the
    // digest is valid — the exact fail-open the review demonstrated.
    let body = strip_trailer(&raw).replace(
        "{\"from\": \"Worker::on_r\", \"to\": \"call_it\"}",
        "{\"from\": 7, \"to\": \"call_it\"}",
    );
    assert!(body.contains("\"from\": 7"), "test premise: the edit landed");
    let bad = dir.join("badrow.topology");
    std::fs::write(&bad, restamp_digest(&body)).unwrap();
    let out = hale().arg("topology").arg("graph").arg(&bad).output().unwrap();
    assert!(
        !out.status.success(),
        "malformed row must refuse, not render a false absence"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("relations.calls[") && err.contains(".from"),
        "refusal names the row and field: {}",
        err
    );

    // Same for a malformed unknowns row: residue must never be
    // silently droppable either.
    let body = strip_trailer(&raw).replace(
        "{\"fn\": \"call_it\", \"reasons\": [\"indirect_call\"]}",
        "{\"fn\": \"call_it\", \"reasons\": \"indirect_call\"}",
    );
    let bad2 = dir.join("badunknown.topology");
    std::fs::write(&bad2, restamp_digest(&body)).unwrap();
    let out = hale().arg("topology").arg("graph").arg(&bad2).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknowns[0]"),
        "refusal names the unknowns row"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 3: a digest-valid, well-TYPED row naming a ghost endpoint
/// must be refused — the anchored-edge filter would otherwise
/// silently delete the edge, a false absence again.
#[test]
fn referentially_invalid_rows_are_refused() {
    let dir = workdir("ghost");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    let raw = std::fs::read_to_string(&artifact).unwrap();

    // Ghost call endpoint.
    let body = strip_trailer(&raw).replace(
        "{\"from\": \"Worker::on_r\", \"to\": \"call_it\"}",
        "{\"from\": \"Ghost::run\", \"to\": \"call_it\"}",
    );
    assert!(body.contains("Ghost::run"), "test premise: edit landed");
    let bad = dir.join("ghostcall.topology");
    std::fs::write(&bad, restamp_digest(&body)).unwrap();
    let out = hale().arg("topology").arg("graph").arg(&bad).output().unwrap();
    assert!(!out.status.success(), "ghost call endpoint must refuse");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Ghost::run") && err.contains("not in the sorts"),
        "refusal names the ghost: {}",
        err
    );

    // Ghost subscription locus.
    let body = strip_trailer(&raw).replace(
        "{\"subject\": \"Cmds\", \"locus\": \"Store\", \"handler\": \"on_c\"}",
        "{\"subject\": \"Cmds\", \"locus\": \"Phantom\", \"handler\": \"on_c\"}",
    );
    assert!(body.contains("Phantom"), "test premise: edit landed");
    let bad2 = dir.join("ghostsub.topology");
    std::fs::write(&bad2, restamp_digest(&body)).unwrap();
    let out = hale().arg("topology").arg("graph").arg(&bad2).output().unwrap();
    assert!(!out.status.success(), "ghost subscription locus must refuse");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Phantom"),
        "refusal names the phantom locus"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 4 (shared P1): `uninhabited_interface_call:*` rows are
/// known-DEAD closed-world dispatches, not residue — the artifact's
/// own documented distinction. They must render neutrally, never
/// join the unresolved count, and a clean-but-for-dead-sites
/// application must not be visualized as incomplete.
#[test]
fn dead_dispatches_are_not_rendered_as_unresolved() {
    let dir = workdir("dead");
    let artifact = dump_artifact(&dir, &fixture("pipeline.hl"));
    let raw = std::fs::read_to_string(&artifact).unwrap();

    // Add a dead-dispatch row alongside the genuine indirect_call.
    let body = strip_trailer(&raw).replace(
        "{\"fn\": \"call_it\", \"reasons\": [\"indirect_call\"]}",
        "{\"fn\": \"call_it\", \"reasons\": [\"indirect_call\"]}, \
         {\"fn\": \"double\", \"reasons\": [\"uninhabited_interface_call:Notifier.notify\"]}",
    );
    let mixed = dir.join("dead.topology");
    std::fs::write(&mixed, restamp_digest(&body)).unwrap();

    // System view: the unresolved count stays 1 (the genuine hole);
    // the dead site gets its own neutral card stating exactness.
    let mmd = render(&mixed, &["--format", "mermaid"]);
    assert!(
        mmd.contains("1 unresolved"),
        "dead dispatch must not inflate the unresolved count:\n{}",
        mmd
    );
    assert!(
        mmd.contains("1 dead dispatch")
            && mmd.contains("call relation stays exact"),
        "dead dispatch gets its own neutral card:\n{}",
        mmd
    );

    // Residue view: both render, distinctly — the dead site as a
    // dead dispatch, not an unresolved hole.
    let svg = render(&mixed, &["--view", "residue"]);
    assert!(svg.contains("indirect_call"), "genuine hole rendered");
    assert!(
        svg.contains("dead dispatch: Notifier.notify")
            && svg.contains("dead (uninhabited)"),
        "dead site rendered neutrally:\n{}",
        svg
    );
    // The neutral grey, not the residue red, styles the dead node.
    assert!(
        svg.contains("#718096"),
        "dead node uses the neutral palette"
    );

    // A DEAD-ONLY artifact renders as exact: no unresolved card at
    // all, and the residue view says so.
    let body = strip_trailer(&raw).replace(
        "{\"fn\": \"call_it\", \"reasons\": [\"indirect_call\"]}",
        "{\"fn\": \"call_it\", \"reasons\": [\"uninhabited_interface_call:Notifier.notify\"]}",
    );
    let deadonly = dir.join("deadonly.topology");
    std::fs::write(&deadonly, restamp_digest(&body)).unwrap();
    let mmd = render(&deadonly, &["--format", "mermaid"]);
    assert!(
        !mmd.contains("unresolved"),
        "a dead-only application is not incomplete:\n{}",
        mmd
    );
    let residue = render(&deadonly, &["--view", "residue", "--format", "mermaid"]);
    assert!(
        residue.contains("no unresolved residue"),
        "residue view states exactness over dead-only unknowns:\n{}",
        residue
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 5: a BUDGET-operand mutation — `per_call` 4 → 0 in the
/// typed payload while the compatibility `lowered` row keeps the
/// old passing form. The evidence binding must refuse: a budget
/// verdict is only admissible with a lowered row matching the
/// form RE-RENDERED from the typed operands.
#[test]
fn budget_operand_mutation_is_refused() {
    let dir = workdir("budgetswap");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
@budget(alloc_per_call = 4)
fn tight(v: Int) -> Int { return v + 1; }
main locus App {
    params { n: Int = 0; }
    run() { println(tight(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let cut = raw.replacen("\"per_call\": 4", "\"per_call\": 0", 1);
    assert_ne!(cut, raw, "test premise: the mutation landed");
    assert!(
        cut.contains("bound alloc <= 4 on paths from {tight}"),
        "test premise: the lowered evidence row keeps the old form"
    );
    let p2 = dir.join("mutated.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&cut))).unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    // The EXACT error, not a disjunction. While budget rows carried
    // a stale cert-less `(ordinal, None)` expectation, *every*
    // budget artifact failed admission with "has no lowered
    // evidence row" — so accepting that message here let the
    // anti-control pass for a reason unrelated to the mutation.
    assert!(
        err.contains("does not match its typed law")
            || err.contains("certs[0] does not match its typed law"),
        "the budget↔evidence binding refuses the mutation: {}",
        err
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The positive control for the test above, and the regression lock
/// for a downstream handoff: an UNMUTATED `@budget` artifact must
/// pass admission.
///
/// `@budget` certificates used to be appended to `lowered` by the
/// budget engines, keyed by law ordinal alone. Change 5h routed
/// them through the evidence projection like every other
/// certificate — keyed `(ordinal, cert)` — but admission kept
/// registering the old cert-less expectation as well. Nothing emits
/// that row, so the exact lowered↔law bijection could never be
/// satisfied: `hale check` was green and `hale fleet check` refused
/// the very same artifact ("law ordinal N has no lowered evidence
/// row matching `bound alloc <= 0 ...`"). Any binary with a
/// `@budget` contract was inadmissible to a fleet plan.
#[test]
fn budget_artifact_passes_admission_unmutated() {
    let dir = workdir("budgetadmit");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
@budget(alloc_per_call = 4)
fn tight(v: Int) -> Int { return v + 1; }
main locus App {
    params { n: Int = 0; }
    run() { println(tight(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    assert!(
        raw.contains("bound alloc <= 4 on paths from {tight}"),
        "test premise: the budget law lowered an evidence row"
    );
    assert!(
        raw.contains("\"family\": \"budget\""),
        "test premise: the artifact carries a budget law row"
    );
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "a well-formed @budget artifact must be admissible: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 5: the RAW half of a reference is the machine join key.
/// Swapping one occurrence's `name` while keeping its `display`
/// (which is what the catalogs and re-rendered forms check) must
/// not survive — the admission holds raw<->display to one
/// consistent pairing across the whole law section.
#[test]
fn raw_name_swap_under_unchanged_display_is_refused() {
    let dir = workdir("nameswap");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
fn leak(v: Int) -> Int { return v; }
fn safe(v: Int) -> Int { return v; }
group a_side = { safe };
group b_side = { leak };
main locus App {
    params { n: Int = 0; }
    claims {
        iso: forbid reaches(a_side, b_side);
        iso2: forbid reaches(b_side, a_side);
    }
    run() { println(safe(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let needle = "\"name\": \"a_side\", \"display\": \"a_side\", \
                  \"resolved\": true";
    assert!(
        raw.matches(needle).count() >= 2,
        "test premise: the group ref appears in two law rows"
    );
    let swapped = raw.replacen(
        needle,
        "\"name\": \"zz_ghost\", \"display\": \"a_side\", \
         \"resolved\": true",
        1,
    );
    assert_ne!(swapped, raw, "test premise: the swap landed");
    let p2 = dir.join("nameswap.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&swapped)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    // Round 6: the canonical-pair anchor refuses this even for a
    // SINGLETON occurrence — the raw half must match one exact
    // catalog pair, not merely stay self-consistent across rows.
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("does not match any canonical pair")
            || err.contains("names two raw identities"),
        "the raw<->display binding refuses the swap: {}",
        err
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 6: an ANNOTATION-ONLY artifact cannot delete its typed
/// law rows and keep the passing compatibility rows — every
/// lowered row is keyed to (law ordinal, cert ordinal), so gutting
/// `law.rows` orphans the evidence instead of passing vacuously.
#[test]
fn annotation_only_law_deletion_is_refused() {
    let dir = workdir("lawgut");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
@effects(none: { block })
fn pure_math(v: Int) -> Int { return v + 1; }
main locus App {
    params { n: Int = 0; }
    run() { println(pure_math(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    // Empty law.rows by bracket-matching from the `"rows": [`
    // inside the law object; the lowered rows and the `clean`
    // verdict survive untouched.
    let law_at = raw.find("\"law\": {").expect("law section");
    let rows_key = raw[law_at..]
        .find("\"rows\": [")
        .map(|i| law_at + i)
        .expect("law.rows");
    let open = rows_key + "\"rows\": ".len();
    let bytes = raw.as_bytes();
    let (mut depth, mut close, mut in_str, mut esc) =
        (0usize, open, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if esc {
            esc = false;
            continue;
        }
        match b {
            b'\\' if in_str => esc = true,
            b'"' => in_str = !in_str,
            b'[' if !in_str => depth += 1,
            b']' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    assert!(close > open, "bracket match");
    let gutted =
        format!("{}[]{}", &raw[..open], &raw[close + 1..]);
    assert!(
        gutted.contains("\"lowered\""),
        "test premise: the passing lowered rows survive"
    );
    let p2 = dir.join("gutted.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&gutted)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("is not claimed by any typed law row"),
        "the lowered↔law bijection refuses the orphaned \
         evidence: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 6: a bus-selector CANDIDATE swap — the selector keeps its
/// authored name while its machine candidate set names a different
/// existing topic. Admission recomputes the candidates from the
/// catalog with the compiler's own matching rule and refuses the
/// disagreement.
#[test]
fn selector_candidate_swap_is_refused() {
    let dir = workdir("selswap");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
type M { v: Int; }
topic Allowed { payload: M; subject: "app.allowed"; }
topic Denied  { payload: M; subject: "app.denied"; }
locus Sender {
    params { n: Int = 0; }
    bus { publish Allowed; }
    @effects(publish: { Allowed })
    fn send(v: Int) { let m = M { v: v }; Allowed <- m; }
}
locus Sink {
    params { n: Int = 0; }
    bus { subscribe Allowed as on_m; subscribe Denied as on_d; }
    fn on_m(m: M) { self.n = m.v; }
    fn on_d(m: M) { self.n = m.v; }
}
main locus App {
    params { s: Sink = Sink { }; snd: Sender = Sender { }; }
    run() { self.snd.send(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let needle =
        "\"topics\": [{\"name\": \"Allowed\", \"display\": \
         \"Allowed\"}]";
    assert_eq!(
        raw.matches(needle).count(),
        1,
        "test premise: the selector candidate list is unique \
         (catalog rows carry a subject field)"
    );
    let swapped = raw.replacen(
        needle,
        "\"topics\": [{\"name\": \"Denied\", \"display\": \
         \"Denied\"}]",
        1,
    );
    assert_ne!(swapped, raw, "test premise: the swap landed");
    let p2 = dir.join("selswap.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&swapped)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(
            "candidate topics do not match the set recomputed"
        ),
        "the selector binding refuses the candidate swap: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 6: a `causes:` OPERAND swap under a retained legacy
/// verdict — the `law.legacy` report entry is keyed by the
/// fingerprint re-rendered from the typed operands, so changing
/// the class to another declared one orphans the entry.
#[test]
fn causes_operand_swap_is_refused() {
    let dir = workdir("causesswap");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
effect money;
effect spare;
topic Sig { payload: Int; subject: "app.sig"; }
locus P {
    params { n: Int = 0; }
    bus { publish Sig; }
    @effects(causes: { money })
    fn poke(v: Int) { Sig <- v; }
}
locus H {
    params { n: Int = 0; }
    bus { subscribe Sig as on_s; }
    fn on_s(v: Int) { self.n = charge(v); }
}
@effects(is: { money })
fn charge(v: Int) -> Int { return v; }
@effects(is: { spare })
fn side(v: Int) -> Int { return v; }
main locus App {
    params { h: H = H { }; p: P = P { }; }
    run() { self.p.poke(1); println(side(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let needle = "\"class\": \"money\"";
    assert_eq!(
        raw.matches(needle).count(),
        1,
        "test premise: the causes payload is the only class ref"
    );
    let swapped =
        raw.replacen(needle, "\"class\": \"spare\"", 1);
    assert_ne!(swapped, raw, "test premise: the swap landed");
    let p2 = dir.join("causesswap.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&swapped)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("typed payload renders"),
        "the rendered form refuses the operand \
         swap: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 6: an injected FLEET law row — a family the application
/// artifact does not own (Change 7 owns the fleet account) — is
/// refused outright, never excluded from the document verdict.
#[test]
fn fleet_row_injection_is_refused() {
    let dir = workdir("fleetrow");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
@effects(none: { block })
fn pure_math(v: Int) -> Int { return v + 1; }
main locus App {
    params { n: Int = 0; }
    run() { println(pure_math(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    // Prepend the reviewer's exact row shape as row 0, bumping
    // the real row to keep ordinal continuity out of the refusal
    // path.
    let anchor = "\"verdict\": \"clean\"";
    assert!(raw.contains(anchor), "clean fixture");
    let injected = raw.replacen(
        "\"rows\": [\n",
        "\"rows\": [\n      {\"ordinal\": 0, \"name\": \"ghost\", \
         \"origin\": \"fleet\", \"family\": \"fleet\", \
         \"verdict\": \"violated\", \"law\": {\"kind\": \
         \"fleet_forbid_reaches\"}},\n",
        1,
    );
    assert_ne!(injected, raw, "test premise: the row landed");
    // Renumber the original row 0 to keep ordinal continuity out
    // of the refusal path: bump every later ordinal by one.
    let injected = injected.replacen(
        "\"fleet_forbid_reaches\"}},\n      {\"ordinal\": 0,",
        "\"fleet_forbid_reaches\"}},\n      {\"ordinal\": 1,",
        1,
    );
    let p2 = dir.join("fleetrow.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&injected)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("malformed artifact"),
        "the fleet row is refused, not excluded: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("fleet row, which an application artifact"),
        "the refusal names the fleet inadmissibility: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 7: static invalidity DOMINATES the replayed engine
/// result. The semantics-2 counterexample: a cyclic class judges
/// `invalid` while the compatibility certificate preserves the old
/// engine's vacuous `holds` — flipping the row verdict to `holds`
/// (and the document to `clean`) must refuse even though the
/// recomputed certificate severity IS `holds`.
#[test]
fn cyclic_class_verdict_flip_is_refused() {
    let dir = workdir("cycflip");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
effect a = { b };
effect b = { a };
@effects(none: { a })
fn f(v: Int) -> Int { return v; }
main locus App {
    params { n: Int = 0; }
    run() { println(f(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    // A cyclic class is a check ERROR — the artifact still dumps
    // (verdict law_failed), it just fails the build.
    let artifact = dir.join("app.hale.topology");
    let _ = hale()
        .arg("check")
        .arg(&src)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .unwrap();
    let raw = std::fs::read_to_string(&artifact)
        .expect("the cyclic artifact still dumps");
    // The compiler-produced artifact ADMITS as-is (invalid over a
    // cyclic class, old holds preserved in the certs).
    let out =
        hale().arg("topology").arg("graph").arg(&artifact).output().unwrap();
    assert!(
        out.status.success(),
        "the honest cyclic artifact admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let flipped = raw
        .replacen("\"verdict\": \"invalid\"", "\"verdict\": \"holds\"", 1)
        .replacen(
            "\"verdict\": \"law_failed\"",
            "\"verdict\": \"clean\"",
            1,
        );
    assert_ne!(flipped, raw, "test premise: the flip landed");
    let p2 = dir.join("flipped.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&flipped)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("only `invalid` is truthful"),
        "static invalidity dominates the replayed holds: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 7: an `invalid` verdict needs a typed reason — a resolved
/// analyzable law cannot assert `invalid` and discard its
/// certificates.
#[test]
fn bare_invalid_with_discarded_certs_is_refused() {
    let dir = workdir("bareinvalid");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
@no_panic
fn f(v: Int) -> Int { return v; }
main locus App {
    params { n: Int = 0; }
    run() { println(f(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let needle_from = raw
        .find("\"certs\": [")
        .expect("the no_panic row carries its certificate");
    // Delete the certs array (bracket match) and assert invalid.
    let open = needle_from + "\"certs\": ".len();
    let bytes = raw.as_bytes();
    let (mut depth, mut close, mut in_str, mut esc) =
        (0usize, open, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if esc {
            esc = false;
            continue;
        }
        match b {
            b'\\' if in_str => esc = true,
            b'"' => in_str = !in_str,
            b'[' if !in_str => depth += 1,
            b']' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let gutted = format!(
        "{}{}",
        &raw[..needle_from - 2], // also eat the `, ` separator
        &raw[close + 1..]
    );
    let asserted = gutted
        .replacen("\"verdict\": \"holds\"", "\"verdict\": \"invalid\"", 1)
        .replacen(
            "\"verdict\": \"clean\"",
            "\"verdict\": \"law_failed\"",
            1,
        );
    let p2 = dir.join("bareinvalid.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&asserted)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("carries 0 certificates, its law generates"),
        "a bare invalid cannot discard its evidence: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 7: the compiler-produced CYCLIC unmigrated artifacts must
/// admit through their own admission — machine `invalid` rows
/// preserving the old engines' `holds` reports (`law.legacy`, the
/// keyed budget `lowered` row) are bound, not orphaned.
#[test]
fn cyclic_unmigrated_artifacts_self_admit() {
    for (tag, body) in [
        (
            "cyccauses",
            "effect a = { b };\neffect b = { a };\n\
             @effects(causes: { a })\n\
             fn f(v: Int) -> Int { return v; }",
        ),
        (
            "cycbudget",
            "effect a = { b };\neffect b = { a };\n\
             @budget(a = 1)\n\
             fn f(v: Int) -> Int { return v; }",
        ),
    ] {
        let dir = workdir(tag);
        let src = dir.join("app.hl");
        std::fs::write(
            &src,
            format!(
                "{}\nmain locus App {{\n    params {{ n: Int = 0; \
                 }}\n    run() {{ println(f(1)); }}\n}}\nfn main() \
                 {{ App {{ }}; }}\n",
                body
            ),
        )
        .unwrap();
        let artifact = dir.join("app.hale.topology");
        let _ = hale()
            .arg("check")
            .arg(&src)
            .arg(format!(
                "--dump-topology={}",
                artifact.display()
            ))
            .output()
            .unwrap();
        assert!(
            artifact.exists(),
            "{}: the cyclic artifact still dumps",
            tag
        );
        let out = hale()
            .arg("topology")
            .arg("graph")
            .arg(&artifact)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}: the compiler's own cyclic artifact must admit: {}",
            tag,
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Round 7: the catalogs are CLOSED — a ghost topic appended to
/// `law.topics` (with its wire pattern in `law.subjects` and both
/// added to a selector's candidate sets) must refuse; otherwise
/// candidate recomputation is circular.
#[test]
fn ghost_catalog_widening_is_refused() {
    let dir = workdir("ghostcat");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
type M { v: Int; }
topic Allowed { payload: M; subject: "app.allowed"; }
locus Sender {
    params { n: Int = 0; }
    bus { publish Allowed; }
    @effects(publish: { Allowed })
    fn send(v: Int) { let m = M { v: v }; Allowed <- m; }
}
locus Sink {
    params { n: Int = 0; }
    bus { subscribe Allowed as on_m; }
    fn on_m(m: M) { self.n = m.v; }
}
main locus App {
    params { s: Sink = Sink { }; snd: Sender = Sender { }; }
    run() { self.snd.send(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    // Append a ghost topic whose raw name tail-matches the
    // selector.
    let widened = raw.replacen(
        "\"topics\": [{\"name\": \"Allowed\", \"display\": \
         \"Allowed\", \"subject\": \"app.allowed\"}]",
        "\"topics\": [{\"name\": \"Allowed\", \"display\": \
         \"Allowed\", \"subject\": \"app.allowed\"}, {\"name\": \
         \"Ghost::Allowed\", \"display\": \"Ghost::Allowed\", \
         \"subject\": \"ghost.allowed\"}]",
        1,
    );
    assert_ne!(widened, raw, "test premise: the ghost landed");
    let p2 = dir.join("ghost.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&widened)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the catalogs are closed"),
        "the widened catalog refuses: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 7: the digests are recomputed, never shape-checked. A row
/// edit under a STALE law_digest refuses even with a fresh
/// artifact_digest; a foreign inputs_digest refuses outright.
#[test]
fn stale_digests_are_refused() {
    let dir = workdir("staledigest");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
@effects(none: { block })
fn pure_math(v: Int) -> Int { return v + 1; }
main locus App {
    params { n: Int = 0; }
    run() { println(pure_math(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    // 1. Edit a law row, restamp ONLY the document trailer (the
    //    stale law_digest survives).
    fn restamp_doc_only(body: &str) -> String {
        format!(
            "{},\n  \"artifact_digest\": \"{:016x}\"\n}}\n",
            body,
            fnv1a64(body.as_bytes())
        )
    }
    let edited = raw.replacen(
        "\"name\": \"pure_math\", \"origin\": \"annotation\"",
        "\"name\": \"pure_meth\", \"origin\": \"annotation\"",
        1,
    );
    assert_ne!(edited, raw, "test premise: the row edit landed");
    let p2 = dir.join("stale.topology");
    std::fs::write(&p2, restamp_doc_only(&strip_trailer(&edited)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("law_digest does not recompute"),
        "the stale law_digest refuses: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // 2. A foreign inputs_digest refuses: the evidence was
    //    produced under a different analysis snapshot.
    let ik = "\"inputs_digest\": \"";
    let at = raw.find(ik).unwrap() + ik.len();
    let mut foreign = raw.clone();
    foreign.replace_range(at..at + 16, "0000000000000000");
    let p3 = dir.join("foreigninputs.topology");
    std::fs::write(&p3, restamp_digest(&strip_trailer(&foreign)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p3)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("different analysis-inputs snapshot"),
        "the foreign inputs_digest refuses: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 7: a violated claims-tier law cannot delete its
/// countermodel — `violated` means "here is the witness", not a
/// bare label.
#[test]
fn deleted_countermodel_is_refused() {
    let dir = workdir("nocounter");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
fn leak(v: Int) -> Int { return v; }
fn touchy(v: Int) -> Int { return leak(v); }
group a_side = { touchy };
group b_side = { leak };
main locus App {
    params { n: Int = 0; }
    claims { iso: forbid reaches(a_side, b_side); }
    run() { println(touchy(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    // A violated claim still dumps an artifact (check fails but
    // --dump-topology of a violated program is the point: verify
    // whether check refuses; if it does, dump differently).
    let artifact = dir.join("app.hale.topology");
    let out = hale()
        .arg("check")
        .arg(&src)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .unwrap();
    if !artifact.exists() {
        // A violated program refuses to dump — the emitter never
        // produces a violated artifact to strip, so construct the
        // deletion on a HOLDING row instead: flip it to violated
        // WITHOUT evidence (the round-7 rule refuses the bare
        // label).
        let _ = out;
        let src2 = dir.join("app2.hl");
        std::fs::write(
            &src2,
            r#"
fn leak(v: Int) -> Int { return v; }
fn safe(v: Int) -> Int { return v; }
group a_side = { safe };
group b_side = { leak };
main locus App {
    params { n: Int = 0; }
    claims { iso: forbid reaches(a_side, b_side); }
    run() { println(safe(1)); }
}
fn main() { App { }; }
"#,
        )
        .unwrap();
        let artifact = dump_artifact(&dir, &src2);
        let raw = std::fs::read_to_string(&artifact).unwrap();
        let lied = raw
            .replace("\"verdict\": \"holds\"", "\"verdict\": \"violated\"")
            .replace("\"result\": \"holds\"", "\"result\": \"violated\"")
            .replacen(
                "\"verdict\": \"clean\"",
                "\"verdict\": \"law_failed\"",
                1,
            );
        let p2 = dir.join("nocounter.topology");
        std::fs::write(&p2, restamp_digest(&strip_trailer(&lied)))
            .unwrap();
        let out = hale()
            .arg("topology")
            .arg("graph")
            .arg(&p2)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("retains none of its judgment's"),
            "a violated law without its countermodel refuses: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 8: every compiler-emitted Invalid admits. The claims
/// evaluator has legitimate invalidity beyond references and
/// classes — `require attributed(all <declared user class>)` is an
/// operand-domain error the judgment explains in its evidence —
/// and `--dump-topology` deliberately emits despite claim errors
/// so such rows can be replayed externally.
#[test]
fn operand_domain_invalid_admits() {
    let dir = workdir("attrinvalid");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
effect purpose;
main locus App {
    params { n: Int = 0; }
    claims { bad: require attributed(all purpose); }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dir.join("app.hale.topology");
    let _ = hale()
        .arg("check")
        .arg(&src)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .unwrap();
    let raw = std::fs::read_to_string(&artifact)
        .expect("the invalid-claim artifact still dumps");
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "a compiler-emitted operand-domain Invalid admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // …but STRIPPING the judgment's explanation refuses: invalid
    // must retain its reason.
    let stripped = raw.replacen(", \"evidence\": [", ", \"evidence_\": [", 1);
    if stripped != raw {
        // renaming the key would fail only_keys; instead remove
        // the whole evidence field by bracket-matching
        let at = raw.find(", \"evidence\": [").unwrap();
        let open = at + ", \"evidence\": ".len();
        let bytes = raw.as_bytes();
        let (mut depth, mut close, mut in_str, mut esc) =
            (0usize, open, false, false);
        for (i, &b) in bytes.iter().enumerate().skip(open) {
            if esc {
                esc = false;
                continue;
            }
            match b {
                b'\\' if in_str => esc = true,
                b'"' => in_str = !in_str,
                b'[' if !in_str => depth += 1,
                b']' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        close = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let gutted =
            format!("{}{}", &raw[..at], &raw[close + 1..]);
        let p2 = dir.join("noexplain.topology");
        std::fs::write(
            &p2,
            restamp_digest(&strip_trailer(&gutted)),
        )
        .unwrap();
        let out = hale()
            .arg("topology")
            .arg("graph")
            .arg(&p2)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(
                "neither a decodable invalidity nor its \
                 judgment's explanation"
            ),
            "a bare invalid refuses: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 8: an implicit lifecycle phase (`@phase_effects(birth:
/// {})` with no `birth` hook) gets a synthetic Holds certificate —
/// the compiler's own check-clean artifact admits; a module-scoped
/// locus's phase contract judges `uncertified` and admits too.
#[test]
fn implicit_phase_and_module_locus_admit() {
    for (tag, src, expect_verdict) in [
        (
            "implicitphase",
            "@phase_effects(birth: {})\nlocus Worker {\n    \
             params { n: Int = 0; }\n}\nmain locus App {\n    \
             params { w: Worker = Worker { }; }\n    run() { \
             println(1); }\n}\nfn main() { App { }; }\n",
            "holds",
        ),
        // Round 10: a MEMBERLESS module locus is vacuously
        // analyzable — no body to walk means every phase contract
        // holds by absence (and the flag is recomputable, closing
        // the memberless flip in both directions).
        (
            "modlocusphase",
            "module inner {\n    @phase_effects(birth: {})\n    \
             locus Hidden {\n        params { n: Int = 0; }\n    \
             }\n}\nmain locus App {\n    params { n: Int = 0; }\n    \
             run() { println(1); }\n}\nfn main() { App { }; }\n",
            "holds",
        ),
        // A module locus WITH executable members is genuinely
        // unanalyzed: residue, `uncertified`.
        (
            "modlocusmember",
            "module inner {\n    @phase_effects(birth: {})\n    \
             locus Hidden {\n        params { n: Int = 0; }\n        \
             fn poke(v: Int) -> Int { return v; }\n    \
             }\n}\nmain locus App {\n    params { n: Int = 0; }\n    \
             run() { println(1); }\n}\nfn main() { App { }; }\n",
            "uncertified",
        ),
    ] {
        let dir = workdir(tag);
        let src_p = dir.join("app.hl");
        std::fs::write(&src_p, src).unwrap();
        let artifact = dump_artifact(&dir, &src_p);
        let raw = std::fs::read_to_string(&artifact).unwrap();
        assert!(
            raw.contains(&format!(
                "\"verdict\": \"{}\"",
                expect_verdict
            )),
            "{}: expected {}:\n{}",
            tag,
            expect_verdict,
            raw
        );
        let out = hale()
            .arg("topology")
            .arg("graph")
            .arg(&artifact)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}: the compiler's own artifact admits: {}",
            tag,
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Round 8: a declared publisher with NO send site is a real
/// endpoint — the artifact carries it in the typed `endpoints`
/// section and admits; deleting its subject from `law.subjects`
/// (narrowing a selector's recomputed universe) refuses on the
/// exact-equality closure.
#[test]
fn declared_endpoint_without_site_admits_and_closes() {
    let dir = workdir("declendpoint");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
type Msg { n: Int = 0; }
locus Producer {
    params { n: Int = 0; }
    bus { publish "unused.address" of type Msg; }
}
main locus App {
    params { p: Producer = Producer { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    assert!(
        raw.contains(
            "{\"verb\": \"publish\", \"subject\": \
             \"unused.address\", \"via\": \"declaration\", \
             \"locus\": \"Producer\""
        ),
        "the declared endpoint row exists:\n{}",
        raw
    );
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the declared-endpoint artifact admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Narrowing: delete the subject from law.subjects while the
    // endpoint row remains.
    let narrowed =
        raw.replacen("\"subjects\": [\"unused.address\"]", "\"subjects\": []", 1);
    assert_ne!(narrowed, raw, "test premise: the deletion landed");
    let p2 = dir.join("narrowed.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&narrowed)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(
            "does not equal the model's typed endpoint universe"
        ),
        "the narrowed subject universe refuses: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 9: a table-level law-selection failure cannot produce a
/// clean artifact. Two duplicate-name claims that individually
/// hold fail `hale check`; the artifact carries the failure in
/// `law.issues`, its document verdict is `law_failed`, it still
/// ADMITS (the account is honest) — and deleting the issues to
/// dress it up as clean refuses on the recomputable duplicate.
#[test]
fn duplicate_claim_names_cannot_be_clean() {
    let dir = workdir("dupclaims");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
fn leak(v: Int) -> Int { return v; }
fn safe(v: Int) -> Int { return v; }
group a_side = { safe };
group b_side = { leak };
main locus App {
    params { n: Int = 0; }
    claims {
        iso: forbid reaches(a_side, b_side);
        iso: forbid reaches(b_side, a_side);
    }
    run() { println(safe(1)); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dir.join("app.hale.topology");
    let out = hale()
        .arg("check")
        .arg(&src)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "duplicate names fail the check"
    );
    let raw = std::fs::read_to_string(&artifact)
        .expect("the artifact still dumps");
    assert!(
        raw.contains("\"verdict\": \"law_failed\""),
        "no claim error disappears between checking and \
         projection:\n{}",
        raw
    );
    assert!(
        !raw.contains("\"issues\": []"),
        "the law-selection account records the duplicate:\n{}",
        raw
    );
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the honest failing artifact admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Dress it up: empty the issues, flip the verdict, restamp
    // both digests — the duplicate is recomputable from the rows.
    let issues_at = raw.find("\"issues\": [").expect("issues");
    let open = issues_at + "\"issues\": ".len();
    let bytes = raw.as_bytes();
    let (mut depth, mut close, mut in_str, mut esc) =
        (0usize, open, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if esc {
            esc = false;
            continue;
        }
        match b {
            b'\\' if in_str => esc = true,
            b'"' => in_str = !in_str,
            b'[' if !in_str => depth += 1,
            b']' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    close = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let dressed = format!(
        "{}[]{}",
        &raw[..open],
        &raw[close..]
    )
    .replacen("\"verdict\": \"law_failed\"", "\"verdict\": \"clean\"", 1);
    let p2 = dir.join("dressed.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&dressed)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(
            "duplicate claim name `iso` with an empty \
             law-selection account"
        ),
        "the recomputable pre-pass refuses the dressed-up \
         artifact: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 9: the endpoints section is not a second authority — a
/// literal publish's endpoint row cannot be deleted (narrowing the
/// selector universe) while the actual publish relation remains.
#[test]
fn endpoint_narrowing_against_relations_is_refused() {
    let dir = workdir("epnarrow");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
type Msg { n: Int = 0; }
locus Emitter {
    params { n: Int = 0; }
    bus { publish "audit.log" of type Msg; }
    @effects(publish: { "audit.log" })
    fn emit(v: Int) { let m = Msg { n: v }; "audit.log" <- m; }
}
locus Sink {
    params { n: Int = 0; }
    bus { subscribe "audit.log" as on_m of type Msg; }
    fn on_m(m: Msg) { self.n = m.n; }
}
main locus App {
    params { e: Emitter = Emitter { }; s: Sink = Sink { }; }
    run() { self.e.emit(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    // Remove the SITE endpoint row while relations.publishes keeps
    // the actual publish.
    let narrowed = cut_row(
        &raw,
        "{\"verb\": \"publish\", \"subject\": \"audit.log\", \
         \"via\": \"site\", \"fn\": \"Emitter::emit\", \
         \"site\": 0",
    );
    assert_ne!(narrowed, raw, "test premise: the row was deleted");
    let p2 = dir.join("narrowed.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&narrowed)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(
            "does not project from the artifact's relations"
        ),
        "the endpoint recompute refuses the narrowing: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 9: `analyzable` is recomputed from the member account —
/// flipping a module-scoped locus's flag (to dress `uncertified`
/// up as `holds`) contradicts the hashed function universe.
#[test]
fn analyzable_flip_is_refused() {
    let dir = workdir("anaflip");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
module inner {
    @phase_effects(birth: {})
    locus Hidden {
        params { n: Int = 0; }
        fn poke(v: Int) -> Int { return v; }
    }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let flipped = raw.replacen(
        "\"name\": \"Hidden\", \"display\": \"Hidden\", \
         \"analyzable\": false",
        "\"name\": \"Hidden\", \"display\": \"Hidden\", \
         \"analyzable\": true",
        1,
    );
    assert_ne!(flipped, raw, "test premise: the flip landed");
    let p2 = dir.join("flipped.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&flipped)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("its member coverage says"),
        "the member-coverage recompute refuses the flip: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 10: a literal wire address whose text collides with a
/// topic display stays a LITERAL — endpoint identity is typed
/// (`declared_topic` from the model's syntactic fact), never
/// inferred from strings. The compiler's own colliding artifact
/// admits; retagging the literal as topic-covered refuses on the
/// wire-subject disagreement.
#[test]
fn literal_topic_collision_admits_and_binds() {
    // Tag must be unique per test: `workdir` clears the directory on
    // entry and each test removes it on exit, so two tests sharing a
    // tag race on one path under a parallel runner.
    let dir = workdir("litcollide");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
type Msg { n: Int = 0; }
topic Orders { payload: Msg; subject: "wire.orders"; }
locus Emitter {
    params { n: Int = 0; }
    bus { publish Orders; publish "Orders" of type Msg; }
    fn emit(v: Int) {
        let m = Msg { n: v };
        Orders <- m;
        "Orders" <- m;
    }
}
locus Sink {
    params { n: Int = 0; }
    bus { subscribe Orders as on_t; subscribe "Orders" as on_l of type Msg; }
    fn on_t(m: Msg) { self.n = m.n; }
    fn on_l(m: Msg) { self.n = m.n; }
}
main locus App {
    params { e: Emitter = Emitter { }; s: Sink = Sink { }; }
    run() { self.e.emit(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the colliding-literal artifact admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Retag the literal site endpoint as topic-covered: its
    // subject `Orders` disagrees with the topic's wire subject.
    let needle = "\"fn\": \"Emitter::emit\", \"site\": 1, \"file\"";
    assert!(raw.contains(needle), "literal site row exists");
    let retagged = raw.replacen(
        needle,
        "\"fn\": \"Emitter::emit\", \"site\": 1, \"topic\": \
         \"Orders\", \"file\"",
        1,
    );
    let p2 = dir.join("retagged.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&retagged)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("disagrees with topic"),
        "the typed identity refuses the retag: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 10: a top-level locus whose only executable row is an
/// `on_failure` handler is analyzable (the handler is executable
/// but never analyzed — an exempt row in the member coverage);
/// the compiler's own artifact admits.
#[test]
fn on_failure_only_locus_admits() {
    let dir = workdir("onfail");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
@phase_effects(birth: {})
locus Guard {
    params { n: Int = 0; }
    on_failure(e: FailureInfo) { self.n = 1; }
}
main locus App {
    params { g: Guard = Guard { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the on_failure-only locus admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 10: the MEMBERLESS module-locus flip is closed in both
/// directions — a memberless locus is vacuously analyzable, so the
/// honest artifact says analyzable=true/holds, and marking it
/// analyzable=false (to manufacture `uncertified`) refuses on the
/// recompute.
#[test]
fn memberless_locus_flag_is_recomputable() {
    let dir = workdir("memberless");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
module inner {
    @phase_effects(birth: {})
    locus Hidden {
        params { n: Int = 0; }
    }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    assert!(
        raw.contains(
            "\"name\": \"Hidden\", \"display\": \"Hidden\", \
             \"analyzable\": true"
        ) && raw.contains("\"verdict\": \"clean\""),
        "a memberless module locus is vacuously analyzable and \
         holds:\n{}",
        raw
    );
    let flipped = raw
        .replacen(
            "\"name\": \"Hidden\", \"display\": \"Hidden\", \
             \"analyzable\": true",
            "\"name\": \"Hidden\", \"display\": \"Hidden\", \
             \"analyzable\": false",
            1,
        );
    let p2 = dir.join("flipped.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&flipped)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("vacuously analyzable"),
        "the memberless flip refuses on the recompute: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 11: the typed site projection is LOSSLESS — removing the
/// literal `"Orders"` endpoint (plus its declaration row, subject,
/// and candidates) while keeping the topic-covered end cannot hide
/// behind the colliding legacy display: the owner-grain relation
/// tie and the span-grained provenance count both contradict it.
#[test]
fn colliding_literal_narrowing_is_refused() {
    let dir = workdir("losslessnarrow");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
type Msg { n: Int = 0; }
topic Orders { payload: Msg; subject: "wire.orders"; }
locus Emitter {
    params { n: Int = 0; }
    bus { publish Orders; publish "Orders" of type Msg; }
    fn emit(v: Int) {
        let m = Msg { n: v };
        Orders <- m;
        "Orders" <- m;
    }
}
locus Sink {
    params { n: Int = 0; }
    bus { subscribe Orders as on_t; subscribe "Orders" as on_l of type Msg; }
    fn on_t(m: Msg) { self.n = m.n; }
    fn on_l(m: Msg) { self.n = m.n; }
}
main locus App {
    params { e: Emitter = Emitter { }; s: Sink = Sink { }; }
    run() { self.e.emit(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let narrowed = cut_row(
        &raw,
        "{\"verb\": \"publish\", \"subject\": \"Orders\", \
         \"via\": \"site\", \"fn\": \"Emitter::emit\", \
         \"site\": 1",
    );
    let narrowed = cut_row(
        &narrowed,
        "{\"verb\": \"publish\", \"subject\": \"Orders\", \
         \"via\": \"declaration\", \"locus\": \"Emitter\"",
    );
    let narrowed = cut_row(
        &narrowed,
        "{\"verb\": \"subscribe\", \"subject\": \"Orders\", \
         \"via\": \"declaration\", \"fn\": \"Sink::on_l\", \
         \"site\": 1",
    );
    let narrowed = cut_row(
        &narrowed,
        "{\"locus\": \"Emitter\", \"subject\": \"Orders\", \
         \"file\"",
    );
    assert_ne!(narrowed, raw, "test premise: the removals landed");
    let p2 = dir.join("narrowed.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&narrowed)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("does not project from the artifact's")
            || err.contains("provenance site count")
            || err.contains("declares_publish relation"),
        "the lossless projection refuses the narrowing: {}",
        err
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 11: the coverage account is typed, not prefix-inferred —
/// a module-scoped ordinary method named `on_failure_helper` is a
/// real (unanalyzed) member, so the module locus is honestly
/// analyzable=false and its artifact admits.
#[test]
fn on_failure_helper_module_method_admits() {
    let dir = workdir("onfhelper");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
module inner {
    @phase_effects(birth: {})
    locus Hidden {
        params { n: Int = 0; }
        fn on_failure_helper(v: Int) -> Int {
            return v;
        }
    }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    assert!(
        raw.contains("\"kind\": \"method\"")
            && raw.contains(
                "\"name\": \"Hidden\", \"display\": \"Hidden\", \
                 \"analyzable\": false"
            ),
        "the helper is a typed METHOD and the locus honestly \
         unanalyzable:\n{}",
        raw
    );
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the helper-named module method admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 12: a module-scoped locus whose only executable member is
/// an `on_failure` handler is vacuously analyzable — the handler
/// is never analyzed anywhere, so it cannot make the locus
/// unanalyzable. The builder and admission share ONE typed rule;
/// the compiler's own artifact admits.
#[test]
fn module_failure_only_locus_admits() {
    let dir = workdir("failonly");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
module inner {
    @phase_effects(birth: {})
    locus Guard {
        params { n: Int = 0; }
        on_failure(e: FailureInfo) {
            self.n = 1;
        }
    }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    assert!(
        raw.contains(
            "\"name\": \"Guard\", \"display\": \"Guard\", \
             \"analyzable\": true"
        ),
        "one typed rule on both sides:\n{}",
        raw
    );
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the failure-only module locus admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 12: the coverage bit cannot be upgraded — `analyzed` is
/// anchored to the HASHED summary universe (a walked body is
/// summarized), so flipping a module fn's bit to manufacture a
/// Holds certificate contradicts `sorts.fns`.
#[test]
fn coverage_upgrade_is_refused() {
    let dir = workdir("covupgrade");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
module inner {
    @effects(none: { syscall })
    fn f(v: Int) -> Int {
        return v;
    }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let needle = "\"display\": \"f\", \"analyzed\": false, \
                  \"summarized\": false";
    assert!(raw.contains(needle), "module fn coverage:\n{}", raw);
    let upgraded = raw.replacen(
        needle,
        "\"display\": \"f\", \"analyzed\": true, \
         \"summarized\": false",
        1,
    );
    let p2 = dir.join("upgraded.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&upgraded)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(
            "analyzed but not summarized — the walked set is \
             the summary set"
        ),
        "the coverage upgrade refuses on the hashed anchor: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 13: a literal declaration whose text equals a topic's
/// wire subject and the typed topic declaration are DISTINCT
/// endpoint facts — the canonical identity includes declared_topic,
/// so both survive in EITHER declaration order (no first-writer
/// collapse), and the artifact admits.
#[test]
fn colliding_declarations_both_survive() {
    for (tag, bus) in [
        (
            "declorder1",
            "publish \"wire.orders\" of type Msg;\n        \
             publish Orders;",
        ),
        (
            "declorder2",
            "publish Orders;\n        \
             publish \"wire.orders\" of type Msg;",
        ),
    ] {
        let dir = workdir(tag);
        let src = dir.join("app.hl");
        std::fs::write(
            &src,
            format!(
                "type Msg {{ n: Int = 0; }}\n\
                 topic Orders {{ payload: Msg; subject: \
                 \"wire.orders\"; }}\n\
                 locus Producer {{\n    params {{ n: Int = 0; \
                 }}\n    bus {{\n        {}\n    }}\n}}\n\
                 main locus App {{\n    params {{ p: Producer = \
                 Producer {{ }}; }}\n    run() {{ println(1); \
                 }}\n}}\nfn main() {{ App {{ }}; }}\n",
                bus
            ),
        )
        .unwrap();
        let artifact = dump_artifact(&dir, &src);
        let raw = std::fs::read_to_string(&artifact).unwrap();
        // Both typed facts present: the literal (no topic) and
        // the typed topic end.
        assert!(
            raw.contains(
                "{\"locus\": \"Producer\", \"subject\": \
                 \"wire.orders\", \"file\""
            ) && raw.contains(
                "{\"locus\": \"Producer\", \"subject\": \
                 \"wire.orders\", \"topic\": \"Orders\", \
                 \"file\""
            ),
            "{}: both declarations survive:\n{}",
            tag,
            raw
        );
        let out = hale()
            .arg("topology")
            .arg("graph")
            .arg(&artifact)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}: the colliding declarations admit: {}",
            tag,
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Round 13: closures are invisible to the certificate machinery
/// at EVERY scope (a top-level closure-only locus already
/// certifies synthetically), so one membership rule applies on
/// both sides and a module-scoped closure-only locus is vacuously
/// analyzable — the compiler's own artifact admits.
#[test]
fn module_closure_only_locus_admits() {
    let dir = workdir("closuremod");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
module inner {
    @phase_effects(birth: {})
    locus Hidden {
        params { n: Int = 0; }
        closure heartbeat {
            self.n ~~ 0 within 0;
            epoch duration(5ms);
        }
    }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    assert!(
        raw.contains(
            "\"name\": \"Hidden\", \"display\": \"Hidden\", \
             \"analyzable\": true"
        ),
        "one membership rule on both sides:\n{}",
        raw
    );
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the closure-only module locus admits: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 16: the artifact's typed owner is anchored to the entity
/// identity too — retagging `Hidden::poke`'s owner to `App` (with
/// both analyzability flags updated consistently) refuses because
/// the display cannot canonically encode that owner.
#[test]
fn artifact_owner_swap_is_refused() {
    let dir = workdir("ownerswap");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
module inner {
    @phase_effects(birth: {})
    locus Hidden {
        params { n: Int = 0; }
        fn poke(v: Int) -> Int { return v; }
    }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let needle = "\"display\": \"Hidden::poke\", \"analyzed\": \
                  false, \"summarized\": false, \"kind\": \
                  \"method\", \"owner\": \"Hidden\"";
    assert!(raw.contains(needle), "typed owner present:\n{}", raw);
    let swapped = raw
        .replacen(
            needle,
            "\"display\": \"Hidden::poke\", \"analyzed\": false, \
             \"summarized\": false, \"kind\": \"method\", \
             \"owner\": \"App\"",
            1,
        )
        .replacen(
            "\"name\": \"Hidden\", \"display\": \"Hidden\", \
             \"analyzable\": false",
            "\"name\": \"Hidden\", \"display\": \"Hidden\", \
             \"analyzable\": true",
            1,
        )
        .replacen(
            "\"name\": \"App\", \"display\": \"App\", \
             \"analyzable\": true",
            "\"name\": \"App\", \"display\": \"App\", \
             \"analyzable\": false",
            1,
        );
    assert_ne!(swapped, raw, "test premise: the swap landed");
    let p2 = dir.join("ownerswap.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&swapped)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("cannot canonically be owned by"),
        "the identity anchor refuses the owner swap: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Change 7 (schema 1.12): wire identity is part of the SHAPE.
/// The round-12 residual — substituting a colliding literal's
/// content at an unchanged site, with every unhashed section
/// edited consistently — now contradicts the hashed
/// `endpoint_identity` section and refuses. Rewriting the hashed
/// half instead changes the program's identity.
#[test]
fn colliding_content_substitution_is_refused() {
    let dir = workdir("subst");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
type Msg { n: Int = 0; }
topic Orders { payload: Msg; subject: "wire.orders"; }
locus Emitter {
    params { n: Int = 0; }
    bus { publish Orders; publish "Orders" of type Msg; }
    fn emit(v: Int) {
        let m = Msg { n: v };
        Orders <- m;
        "Orders" <- m;
    }
}
locus Sink {
    params { n: Int = 0; }
    bus { subscribe Orders as on_t; subscribe "Orders" as on_l of type Msg; }
    fn on_t(m: Msg) { self.n = m.n; }
    fn on_l(m: Msg) { self.n = m.n; }
}
main locus App {
    params { e: Emitter = Emitter { }; s: Sink = Sink { }; }
    run() { self.e.emit(1); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    // Retag the literal site-1 end as another copy of the
    // topic-covered end — the exact round-12 substitution — in
    // the UNHASHED endpoints section.
    let needle = "\"subject\": \"Orders\", \"via\": \"site\", \
                  \"fn\": \"Emitter::emit\", \"site\": 1";
    assert!(raw.contains(needle), "literal site row:\n{}", raw);
    let swapped = raw.replacen(
        needle,
        "\"subject\": \"wire.orders\", \"via\": \"site\", \
         \"fn\": \"Emitter::emit\", \"site\": 1, \"topic\": \
         \"Orders\"",
        1,
    );
    let p2 = dir.join("subst.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&swapped)))
        .unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(
            "do not agree with the HASHED endpoint identity"
        ),
        "the hashed identity refuses the substitution: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}


/// Review pin (#496 round 2): the rendered form is REQUIRED, not
/// checked-when-present.
///
/// The full forging attempt, with every contradiction the artifact
/// could still raise removed by hand: delete the row's `form`,
/// substitute a different valid declared operand into the typed
/// payload, upgrade the verdict to `holds`, drop the row evidence
/// that a non-holds verdict would have owed, then restamp
/// `shape_hash`, `law_digest` and `artifact_digest` — all of which
/// are recomputed FROM the mutated rows and therefore recover
/// nothing.
///
/// If `form` were optional, this document would be admitted and a
/// `causes:` law would read as holding over a class it never named.
#[test]
fn a_law_row_may_not_omit_its_rendered_form() {
    let dir = workdir("formless");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
effect money;
effect audit;
type T { n: Int = 0; }
topic Settled { payload: T; subject: "settled"; }
locus Ledger {
    bus { subscribe Settled as on_settled; }
    params { n: Int = 0; }
    @effects(is: { money })
    fn on_settled(t: T) { self.n = t.n; }
}
main locus App {
    params { l: Ledger = Ledger { }; }
    bus { publish Settled; }
    @effects(causes: { money })
    fn fire() { Settled <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    assert!(
        raw.contains("causes {money} from {App::fire}"),
        "test premise: the row states its rendered form:\n{}",
        raw
    );

    // 1. delete the form, 2. swap the operand for another DECLARED
    //    class, 3. claim `holds`, 4. drop any row evidence.
    // Anchored on the following key so this removes the LAW ROW's
    // form, not the identically-spelled fingerprint in the
    // `law.legacy` report (which the cutover deletes anyway).
    let mut cut = raw.replacen(
        "\"form\": \"causes {money} from {App::fire}\", \"file\"",
        "\"file\"",
        1,
    );
    assert_ne!(cut, raw, "test premise: the form field was removed");
    // …and the legacy report's own entry goes with it, so the
    // refusal cannot be blamed on an orphaned compatibility row.
    cut = cut.replacen(
        "\"form\": \"causes {money} from {App::fire}\"",
        "\"form\": \"causes {audit} from {App::fire}\"",
        1,
    );
    cut = cut.replacen("\"class\": \"money\"", "\"class\": \"audit\"", 1);
    cut = cut.replacen("\"verdict\": \"violated\"", "\"verdict\": \"holds\"", 1);

    let p2 = dir.join("forged.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&cut))).unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a formless law row was admitted — the operand binding is \
         optional and therefore absent"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("omits its rendered form"),
        "and it is refused for the RIGHT reason: {}",
        err
    );
}

/// Review pin (#497 round 2): a migrated family ships with its
/// adequacy account, and admission requires it.
///
/// Without this the artifact was internally contradictory: a row
/// could declare `"family": "causes"`, the model defined a
/// completeness contract for that family, and the document was
/// structurally forbidden from stating whether the contract was
/// met — the five-family table rejected any artifact that tried.
#[test]
fn a_migrated_family_carries_its_adequacy_entry() {
    let dir = workdir("adequacy_causes");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
effect money;
type T { n: Int = 0; }
topic Settled { payload: T; subject: "settled"; }
locus Ledger {
    bus { subscribe Settled as on_settled; }
    params { n: Int = 0; }
    fn on_settled(t: T) { self.n = t.n; }
}
main locus App {
    params { l: Ledger = Ledger { }; }
    bus { publish Settled; }
    @effects(causes: { publish })
    fn fire() { Settled <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let raw = std::fs::read_to_string(&artifact).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["schema"], "1.17");
    let adequacy =
        v["adequacy"].as_object().expect("adequacy object");
    assert!(
        adequacy.contains_key("causes"),
        "the family a row can declare must have an account: {:?}",
        adequacy
    );
    assert_eq!(
        adequacy.len(),
        8,
        "five older families plus causes, depends and budget: {:?}",
        adequacy
    );
    assert!(adequacy.contains_key("depends"), "{:?}", adequacy);
    assert!(adequacy.contains_key("budget"), "{:?}", adequacy);

    // …and an artifact that omits it is refused, restamped or not.
    let cut = raw.replacen(",\n    \"causes\": \"exact\"", "", 1);
    let cut = if cut == raw {
        raw.replacen(",\n    \"causes\": \"degraded\"", "", 1)
    } else {
        cut
    };
    assert_ne!(cut, raw, "test premise: the entry was removed");
    let p2 = dir.join("thin.topology");
    std::fs::write(&p2, restamp_digest(&strip_trailer(&cut))).unwrap();
    let out = hale()
        .arg("topology")
        .arg("graph")
        .arg(&p2)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("adequacy must carry exactly"),
        "refused for the right reason: {}",
        err
    );
}

/// Every colour the SVG puts in the document is remapped by its own
/// dark theme.
///
/// The renderer emitted a hardcoded light palette, so hale-lang.org
/// re-themed the diagrams with its own `[fill="#ffffff"] { … }`
/// rules — a copy of this palette in a DIFFERENT REPOSITORY, wrong
/// the moment a colour changed here. The stylesheet now ships inside
/// the SVG, generated from the same table the attributes come from.
///
/// This checks the property that matters and that a new colour would
/// quietly break: a hex used in the body but absent from the palette
/// stays light on a dark background, and nothing else would say so.
#[test]
fn every_svg_colour_is_remapped_by_the_dark_theme() {
    let dir = workdir("svgtheme");
    let src = dir.join("app.hl");
    // Exercise the tinted/holed paths too, not just the happy graph:
    // an unresolved call gives the residue view its hole colours.
    std::fs::write(
        &src,
        r#"
type R { v: Int = 0; }
topic T { payload: R; subject: "t"; }
fn apply(f: fn(Int) -> Int, v: Int) -> Int { return f(v); }
fn scale(v: Int) -> Int { return v * 2; }
locus Pub {
    bus { publish T; }
    params { n: Int = 0; }
    fn go() { T <- R { v: apply(scale, self.n) }; }
}
locus Sub {
    bus { subscribe T as on_t; }
    params { seen: Int = 0; }
    fn on_t(r: R) { self.seen = r.v; }
}
main locus App {
    params { p: Pub = Pub { }; s: Sub = Sub { }; }
    claims { one: count publishers(topic T) == 1; }
    run() { self.p.go(); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    for view in ["system", "code", "bus", "claim", "residue"] {
        let mut argv: Vec<&str> =
            vec!["--view", view, "--format", "svg", "--theme", "dark"];
        if view == "claim" {
            argv.extend(["--claim", "one"]);
        }
        let svg = render(&artifact, &argv);
        let (style, body) = svg
            .split_once("</style>")
            .expect("a themed svg carries its stylesheet");
        let mut unthemed: Vec<String> = Vec::new();
        for (attr, _) in [("fill", 0), ("stroke", 0)] {
            let needle = format!("{}=\"#", attr);
            let mut rest = body;
            while let Some(i) = rest.find(&needle) {
                rest = &rest[i + needle.len() - 1..];
                let hex: String =
                    rest.chars().take_while(|c| *c != '"').collect();
                let rule = format!("[{}=\"{}\"]{{", attr, hex);
                if !style.contains(&rule) {
                    unthemed.push(format!("{} {}", attr, hex));
                }
                rest = &rest[1..];
            }
        }
        unthemed.sort();
        unthemed.dedup();
        assert!(
            unthemed.is_empty(),
            "view `{}` uses colours the dark theme does not remap \
             (add them to SVG_PALETTE): {:?}",
            view,
            unthemed
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The stylesheet is scoped to the SVG's own class.
///
/// An SVG is routinely INLINED into an HTML page — hale-lang.org
/// does exactly that with `set:html`. An unscoped `<style>` here
/// would stop being the diagram's theme and become the host page's
/// CSS.
#[test]
fn the_svg_stylesheet_cannot_escape_into_a_host_page() {
    let dir = workdir("svgscope");
    let src = dir.join("app.hl");
    std::fs::write(
        &src,
        r#"
topic T { payload: R; subject: "t"; }
type R { v: Int = 0; }
locus Pub { bus { publish T; } fn go() { T <- R { v: 1 }; } }
locus Sub { bus { subscribe T as on_t; } params { n: Int = 0; }
    fn on_t(r: R) { self.n = r.v; } }
main locus App { params { p: Pub = Pub { }; s: Sub = Sub { }; }
    run() { self.p.go(); } }
fn main() { App { }; }
"#,
    )
    .unwrap();
    let artifact = dump_artifact(&dir, &src);
    let svg = render(
        &artifact,
        &["--view", "bus", "--format", "svg", "--theme", "dark"],
    );
    let style = svg
        .split_once("</style>")
        .expect("themed")
        .0
        .split_once("<style>")
        .expect("open tag")
        .1;
    for line in style.lines().filter(|l| l.contains('{')) {
        assert!(
            line.trim_start().starts_with(".hale-topo "),
            "every selector must be scoped to the diagram's root; \
             found `{}`",
            line
        );
    }
    assert!(
        svg.contains("class=\"hale-topo\""),
        "the root carries the class its own stylesheet selects on"
    );

    // `light` is the pre-theming output: attributes only.
    let light = render(
        &artifact,
        &["--view", "bus", "--format", "svg", "--theme", "light"],
    );
    assert!(
        !light.contains("<style>"),
        "`--theme light` ships no stylesheet: {:?}",
        &light[..light.len().min(200)]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
