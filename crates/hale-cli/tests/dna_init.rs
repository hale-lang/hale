//! GH #528 — `hale dna init` / `upgrade` (Track C, PR 23).
//!
//! What these hold the command to (the issue's acceptance for the
//! existing-application path):
//!   1. The application's locus graph is untouched and it still
//!      passes its previous checks — plus `hale check --matrix` for
//!      the environment init declares, and it still builds and runs.
//!   2. `vendor/dna` is toolchain-owned and pinned in hale.lock;
//!      `dna/` is project-owned; `upgrade` re-materializes the former
//!      without touching the latter.
//!   3. The Journal is seeded from the compiler's model with
//!      `observed` and `inferred` provenance kept distinct, and the
//!      baseline review is requested before any work runs.
//!   4. Re-running is non-destructive: every file is kept.
//!   5. A law the application cannot certify (it has an unresolvable
//!      edge) is deferred with its reason, never silently dropped
//!      and never made to fail the application.

#[path = "support/vault.rs"]
mod vault;
use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = vault::hale().args(args).current_dir(cwd).output().expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_dna_init_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The pinned renderer fixture as an existing application: two
/// loci, two topics, a group, two claims, and one indirect call.
fn pipeline_app(tag: &str) -> PathBuf {
    let d = workdir(tag).join("pipeline");
    std::fs::create_dir_all(&d).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/topology/pipeline.hl");
    std::fs::copy(fixture, d.join("main.hl")).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    d
}

fn journal_rows(root: &Path) -> Vec<serde_json::Value> {
    // the record is the branch refs/dna/journal; its tree holds the events
    let out = std::process::Command::new("git").args(["-C", &root.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .to_string()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("journal line is JSON"))
        .collect()
}

#[test]
fn init_attaches_the_dna_and_the_application_still_checks_builds_and_runs() {
    let app = pipeline_app("attach");
    let before = std::fs::read_to_string(app.join("main.hl")).unwrap();
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out}");
    for f in [
        "vendor/dna/assembly.hl",
        "vendor/dna/topics.hl",
        "dna/org/main.hl",
        "dna/org/law.hl",
        "dna/org/purpose.hl",
        "dna/org/workflows.hl",
        ".hale/dna/baseline.topology",
        "hale.lock",
    ] {
        assert!(app.join(f).exists(), "init creates {f}: {out}");
    }
    // GH #1091: the structure policy is a repository's; an application's
    // record is not born with holes
    assert!(!app.join("dna/org/structure.hl").exists(), "no structure policy for an application: {out}");
    let lock = std::fs::read_to_string(app.join("hale.lock")).unwrap();
    assert!(lock.contains("[dna]") && lock.contains("toolchain = "), "hale.lock pins the toolchain: {lock}");

    // GH #726: what was materialized says which embedded source set it
    // came from — a version does not identify it, and `vendor/dna` is
    // the binary's copy, not the working tree's.
    let digest = hale_dna::EMBEDDED_DIGEST;
    assert!(out.contains(&format!("embedded dna {}", &digest[..16])), "init reports the embedded source it materialized: {out}");
    let vendor_readme = std::fs::read_to_string(app.join("vendor/dna/README.md")).unwrap();
    assert!(vendor_readme.contains(digest), "the vendored core's README names the source set: {vendor_readme}");
    let prov = std::fs::read_to_string(app.join(".hale/dna/embedded.digest")).unwrap();
    assert_eq!(prov, format!("hale {}\nembedded dna: {digest}\n", env!("CARGO_PKG_VERSION")), "and machine-readably, for a fixture or `hale dna status`");
    // NOT in the organization's source: the digest changes with every
    // edit to the DNA, and `dna/org/main.hl` is project-owned source
    // the organism reviews, diffs and (in the recorded acceptance
    // fixture) feeds to a model — a build-varying line there would
    // make every such baseline miss.
    let org_src = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    assert!(!org_src.contains(digest) && !org_src.contains(&digest[..16]), "the scaffold's source carries no build-varying digest");
    let manifest = std::fs::read_to_string(app.join("hale.toml")).unwrap();
    assert!(manifest.contains("no_base = true") && manifest.contains("[environments.local]") && manifest.contains("[environments.org]"), "{manifest}");

    // The application is not touched at all (GH #566 F2): the organization
    // oversees it from dna/org.
    let after = std::fs::read_to_string(app.join("main.hl")).unwrap();
    assert_eq!(after, before, "init leaves the application's source exactly as it was");
    assert!(!app.join("dna_constitution.hl").exists(), "no law is written into the application");
    let org = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    assert!(org.contains("dna::ReviewVerdict: nats::NatsAdapter { }"), "the organization binds its facts to the nerves: {org}");
    // GH #1143: a schedule's occurrence is named by the wall clock in
    // milliseconds, so the loop ticks with it (`now()` is seconds, and a
    // monotonic clock names no occurrence).
    assert!(org.contains("self.core.request_tick(std::time::nanos(std::time::current()) / 1000000)"), "the loop queues the schedules' tick on the wall clock in milliseconds: {org}");

    // …and it still passes its previous checks, plus the matrix, and builds.
    let (ok, out) = hale(&["check", "."], &app);
    assert!(ok, "check: {out}");
    let (ok, out) = hale(&["check", "--matrix", "."], &app);
    assert!(ok, "matrix: {out}");
    let (ok, out) = hale(&["build", "."], &app);
    assert!(ok, "build: {out}");
    let run = Command::new(app.join("pipeline")).current_dir(&app).output().unwrap();
    assert!(run.status.success(), "run: {}", String::from_utf8_lossy(&run.stderr));

    // Re-running keeps everything.
    let (ok, out2) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out2}");
    assert!(out2.contains("kept") && !out2.contains("created "), "second init is non-destructive:\n{out2}");
    assert_eq!(after, std::fs::read_to_string(app.join("main.hl")).unwrap(), "main.hl untouched on re-run");
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn the_journal_is_seeded_from_the_model_with_provenance_kept_distinct() {
    let app = pipeline_app("journal");
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out}");
    let rows = journal_rows(&app);
    let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds[0], "application.attached");
    assert_eq!(kinds.last().copied(), Some("review.requested"), "the baseline review is requested last: {kinds:?}");
    let body = |r: &serde_json::Value| -> serde_json::Value { serde_json::from_str(r["body"].as_str().unwrap()).unwrap() };
    // observed structure: every locus, topic and claim of the fixture
    let observed: Vec<String> = rows
        .iter()
        .filter(|r| r["kind"] == "structure.observed")
        .map(|r| r["entity"].as_str().unwrap().to_string())
        .collect();
    for e in ["locus:App", "locus:Worker", "locus:Store", "topic:Readings", "topic:Cmds", "claim:apart", "claim:one_writer"] {
        assert!(observed.iter().any(|x| x == e), "{e} observed: {observed:?}");
    }
    for r in rows.iter().filter(|r| r["kind"] == "structure.observed") {
        assert_eq!(body(r)["provenance"], "observed");
    }
    // inferred responsibilities: guesses, never ratified
    let proposed: Vec<&serde_json::Value> = rows.iter().filter(|r| r["kind"] == "responsibility.proposed").collect();
    assert_eq!(proposed.len(), 3, "one guess per locus");
    for r in &proposed {
        let b = body(r);
        assert_eq!(b["provenance"], "inferred");
        assert_eq!(b["ratified"], false);
    }
    let worker = proposed.iter().find(|r| r["entity"] == "locus:Worker").unwrap();
    let guess = body(worker)["responsibility"].as_str().unwrap().to_string();
    assert!(guess.contains("Readings") && guess.contains("Cmds"), "the guess is from structure: {guess}");
    // the declared purpose is a proposal like any other (GH #995): its
    // knowledge row and the Board's Review under the group `purpose`,
    // pinning the same digest (the design's own Reviews follow it — GH #596 C)
    let proposal = rows.iter().find(|r| r["kind"] == "knowledge.proposed" && body(r)["kind"] == "purpose").expect("the purpose proposed");
    assert_eq!(body(proposal)["provenance"], "declared");
    let digest = proposal["entity"].as_str().unwrap().to_string();
    assert!(digest.starts_with("sha256:"));
    let review = body(rows.iter().find(|r| r["kind"] == "review.requested" && body(r)["group"] == "purpose").expect("the purpose's Review"));
    assert_eq!(review["subject_digest"].as_str().unwrap(), digest, "the Review pins the proposal's digest");
    assert_eq!(review["question"], "ratify the declared purpose?");
    let main = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    assert!(!main.contains("review_id: \"purpose\"") && main.contains("catalog: workflows()"), "no Review of its own in the organization; it admits from the catalog");
    // the record is ordered (its chain is git's: one commit per event)
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(r["seq"].as_u64().unwrap() as usize, i);
    }
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn upgrade_rematerializes_vendor_without_touching_the_project_seed() {
    let app = pipeline_app("upgrade");
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out}");
    let assembly = app.join("dna/org/main.hl");
    let owned = std::fs::read_to_string(&assembly).unwrap().replace("self.core.request_tick(", "self.core.tick(");
    std::fs::write(&assembly, &owned).unwrap();
    let vendored = app.join("vendor/dna/topics.hl");
    std::fs::write(&vendored, "// tampered\n").unwrap();
    std::fs::remove_file(app.join("vendor/dna/review.hl")).unwrap();
    // GH #726: a tree materialized by another build says so until
    // `upgrade` re-materializes it
    let prov = app.join(".hale/dna/embedded.digest");
    std::fs::write(&prov, "hale 0.0.1\nembedded dna: 0000000000000000000000000000000000000000000000000000000000000000\n").unwrap();
    // GH #1091: a structure policy is regenerated, the roles the project
    // opted into carried over and what else it said named
    let structure = app.join("dna/org/structure.hl");
    std::fs::write(&structure, "fn operational_roles() -> String { return \"support  on-call\"; }\n").unwrap();
    let (ok, out) = hale(&["dna", "upgrade", "."], &app);
    assert!(ok, "{out}");
    assert!(out.contains("use self.core.request_tick with the same millisecond clock"), "upgrade explains how to queue an older scaffold's cadence: {out}");
    assert!(out.contains("2 file(s) rewritten"), "{out}");
    assert!(out.contains(&format!("embedded dna {}", &hale_dna::EMBEDDED_DIGEST[..16])), "upgrade reports the source set it materialized: {out}");
    assert!(std::fs::read_to_string(&prov).unwrap().contains(hale_dna::EMBEDDED_DIGEST), "and refreshes what the tree came from: {}", std::fs::read_to_string(&prov).unwrap());
    assert!(std::fs::read_to_string(&vendored).unwrap().contains("topic ReviewVerdict"), "vendor restored");
    assert!(app.join("vendor/dna/review.hl").exists());
    assert_eq!(std::fs::read_to_string(&assembly).unwrap(), owned, "dna/ is the project's");
    let policy = std::fs::read_to_string(&structure).unwrap();
    assert!(out.contains("rewrote") && out.contains("structure.hl"), "the structure policy is rewritten: {out}");
    assert!(out.contains("it dropped:\n        fn operational_roles() -> String { return \"support  on-call\"; }"), "and says what it dropped: {out}");
    assert!(policy.starts_with("// dna/org/structure.hl") && policy.contains("    return \"support on-call\";\n"), "to the current shape, keeping the roles: {policy}");
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn upgrade_retires_the_owners_map() {
    // GH #1123: the graph is the one org chart. An organization from before
    // the cut has `dna/org/owners` and the generated `ownership:` field;
    // upgrade removes both, and says what the file named
    let app = pipeline_app("owners-cut");
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out}");
    assert!(!app.join("dna/org/owners").exists(), "init writes no owners file: {out}");
    let main = app.join("dna/org/main.hl");
    let text = std::fs::read_to_string(&main).unwrap();
    let anchor = "            budget: dna::Budget { policy: org_budget() },\n";
    assert!(text.contains(anchor), "the generated main.hl has its budget line");
    let old = "            // Owners (GH #664): who admits which position. The map is the\n            // genome's file dna/org/owners — empty while this organization\n            // is the only owner; once the record is shared, every position\n            // names its owner and this body says which it is (dna.owner).\n            ownership: dna::Ownership { path: \"dna/org/owners\" },\n";
    // GH #1143: and the older generator's optimize field and monotonic tick
    let older = text
        .replace(anchor, &format!("{anchor}{old}"))
        .replace(
            "            // (the optimize pass occurs on the cadence a ratified practice\n            // declares, `operating/optimize-cadence`: GH #1143)\n            planned: true\n",
            "            planned: true,\n            // GH #596 O: the optimize pass — the leader walks the machinery\n            // on this cadence, in milliseconds; 0 is never. The Board's to set.\n            optimize_every_ms: 0\n",
        )
        .replace("self.core.request_tick(std::time::nanos(std::time::current()) / 1000000); }", "self.core.request_tick(std::time::monotonic_ns() / 1000000); }")
        // and its Board field, before it was `board`
        .replace("            board: dna::Board { who: \"board\" },\n", "            membrane: dna::Board { who: \"board\" },\n");
    assert!(older.contains("optimize_every_ms: 0") && older.contains("monotonic_ns()") && older.contains("membrane: dna::Board"), "the older generator's main.hl is built");
    std::fs::write(&main, older).unwrap();
    std::fs::write(app.join("dna/org/owners"), "# who admits what\norg = acme\nacme: alice\n").unwrap();
    let (ok, out) = hale(&["dna", "upgrade", "."], &app);
    assert!(ok, "{out}");
    assert!(!app.join("dna/org/owners").exists(), "the owners file is removed: {out}");
    assert!(out.contains("org = acme") && out.contains("acme: alice") && out.contains("holds(position:<p>, organization:<o>)"), "and what it named is said, to be restated as holds edges: {out}");
    assert!(out.contains("the optimize pass is a schedule"), "the cadence's rewrite is said: {out}");
    let upgraded = std::fs::read_to_string(&main).unwrap();
    assert!(!upgraded.contains("optimize_every_ms") && !upgraded.contains("monotonic_ns()"), "the older field and tick are gone: {upgraded}");
    assert!(out.contains("the Board's field is `board`") && upgraded.contains("board: dna::Board") && !upgraded.contains("membrane:"), "and the Board's field is `board`: {out}");
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn init_refuses_a_seed_without_a_main_locus() {
    let d = workdir("nomain");
    std::fs::write(d.join("main.hl"), "fn main() { println(\"hi\"); }\n").unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    let (ok, out) = hale(&["dna", "init", "."], &d);
    assert!(!ok);
    assert!(out.contains("declares no `main locus`"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
