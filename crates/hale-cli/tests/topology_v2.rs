//! #392 thread 1 — the topology artifact's v2 model export:
//! phases, seeds, derived effects, weights, through-stdlib
//! contraction, and the unhashed provenance half.

use std::process::Command;

fn dump(src: &str, tag: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "hale_topology_v2_{}_{}.hl",
        std::process::id(),
        tag
    ));
    std::fs::write(&path, src).expect("write program");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&path)
        .arg("--dump-topology")
        .output()
        .expect("run hale check");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn shape_hash(artifact: &str) -> String {
    artifact
        .lines()
        .find(|l| l.contains("\"shape_hash\""))
        .expect("shape_hash line")
        .to_string()
}

const BASE: &str = r#"
effect money;
locus B {
    @effects(is: {money})
    fn work(n: Int) -> Int { return n * 2; }
}
locus A {
    params { b: B = B { }; n: Int = 0; }
    birth { self.n = self.b.work(1); }
    fn go(k: Int) -> Int {
        let mut acc = 0;
        for i in 0..k { acc = acc + self.b.work(i); }
        return acc;
    }
}
group a_side = { A };
group b_side = { B };
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side); }
}
fn main() { App { }; }
"#;

/// The v2 hashed sections: the phase relation (hooks vs methods),
/// the derived per-fn effect sets, and call-edge weights.
#[test]
fn the_artifact_exports_phases_effects_and_weights() {
    let art = dump(BASE, "sections");
    assert!(
        art.contains(r#""A::birth": {"phase": "birth", "kind": "hook"}"#),
        "a lifecycle hook is a hook-phase row:\n{}",
        art
    );
    assert!(
        art.contains(r#""A::go": {"phase": "go", "kind": "method"}"#),
        "an ordinary method is a method-phase row:\n{}",
        art
    );
    assert!(
        art.contains(r#""B::work": ["money"]"#),
        "the declared carrier appears in derived effects:\n{}",
        art
    );
    assert!(
        art.contains(
            r#"{"from": "A::go", "to": "B::work", "loop": true, "unbounded": true}"#
        ),
        "the loop-nested (runtime-bounded → unbounded) call edge \
         carries its weights:\n{}",
        art
    );
    assert!(
        art.contains(r#"{"from": "A::birth", "to": "B::work"}"#),
        "the plain call edge stays a plain row:\n{}",
        art
    );
}

/// Provenance is exported — and excluded from the hash: moving code
/// (a comment line at the top) changes every span but must not
/// change the shape identity.
#[test]
fn moving_code_changes_provenance_but_not_shape_hash() {
    let a = dump(BASE, "motion_a");
    let moved = format!("// moved down by one comment line\n{}", BASE);
    let b = dump(&moved, "motion_b");
    assert_eq!(
        shape_hash(&a),
        shape_hash(&b),
        "shape identity must be motion-insensitive"
    );
    let spans_of = |art: &str| -> String {
        art.lines()
            .skip_while(|l| !l.contains("\"provenance\""))
            .take_while(|l| !l.contains("\"claims\""))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_ne!(
        spans_of(&a),
        spans_of(&b),
        "provenance spans must track the moved source"
    );
    assert!(
        a.contains("\"decls\": {")
            && a.contains("\"A\": {\"source\":"),
        "decl spans are exported, resolved to a source file (#408 \
         Phase 0 — a bare offset means nothing outside the process \
         that produced it):\n{}",
        a
    );
}

/// The through-stdlib contraction: a user→user path whose interior
/// is stdlib bodies lands as a `calls_via_stdlib` edge, so
/// reachability replay over the artifact matches the evaluator.
#[test]
fn a_through_stdlib_path_lands_as_a_contracted_edge() {
    let src = r#"
locus Hello {
    fn handle(ctx: std::http::Context) -> std::http::Response {
        return std::http::Response {
            status: 200,
            content_type: "text/plain",
            body: "hi"
        };
    }
}
locus Gate {
    fn probe(r: std::http::Router, req: std::http::Request) -> Int {
        let resp = r.dispatch(req);
        return resp.status;
    }
}
fn main() {
    let r = std::http::Router { };
    r.get("/", Hello { });
    let req = std::http::Request { method: "GET", path: "/", body: "" };
    println(Gate { }.probe(r, req));
}
"#;
    let art = dump(src, "contracted");
    assert!(
        art.contains(
            r#"{"from": "Gate::probe", "to": "Hello::handle", "loop": true}"#
        ),
        "the contracted edge must appear with its conservative loop \
         flag:\n{}",
        art
    );
}

/// #392 §8 — every fn-grained certificate exported as a lowered
/// claim row (one schema of record), with the verdict of the same
/// evaluation that gates the build.
#[test]
fn certificates_appear_as_lowered_claim_rows() {
    let src = r#"
effect money;
@effects(is: {money})
fn charge(n: Int) -> Int { return n; }

type P { a: Int; }

@effects(none: {money})
fn clean(n: Int) -> Int { return n + 1; }

@effects(none: {money})
fn dirty(n: Int) -> Int { return charge(n); }

@budget(alloc_per_call = 2)
fn boxed(n: Int) -> P { return P { a: n }; }

@phase_effects(run: {})
locus Engine {
    params { seen: Int = 0; }
    run() { self.seen = charge(1); }
}

fn main() { Engine { }; }
"#;
    let art = dump(src, "lowered");
    assert!(
        art.contains(
            r#"{"subject": "clean", "form": "forbid reaches({clean}, effects(money))", "result": "holds"}"#
        ),
        "a holding certificate must appear as a lowered row:\n{}",
        art
    );
    assert!(
        art.contains(
            r#"{"subject": "dirty", "form": "forbid reaches({dirty}, effects(money))", "result": "violated"}"#
        ),
        "a violated certificate must appear as a lowered row:\n{}",
        art
    );
    assert!(
        art.contains(
            r#"{"subject": "boxed", "form": "bound alloc <= 2 on paths from {boxed}", "result": "holds"}"#
        ),
        "a budget contract must appear as a lowered bound row:\n{}",
        art
    );
    assert!(
        art.contains(
            r#"{"subject": "Engine", "form": "only effects {} on {Engine} during run", "result": "violated"}"#
        ),
        "a phase contract must appear as a lowered row (violated by \
         the unlisted user class):\n{}",
        art
    );
}

/// #399 — the per-topic observation identity: the artifact's
/// `topics` rows carry the same (subject, shape, hash) the runtime
/// manifest fuses on, so a WAL segment names the checked topology.
/// The hash vector here is the protocol's (PROTOCOL.md §4);
/// changing it is a wire break.
#[test]
fn the_artifact_exports_the_observation_identity() {
    let src = r#"
type Task { id: Int; label: String; }
topic Tasks { payload: Task; }
locus A {
    bus { publish Tasks; }
    fn go(n: Int) { Tasks <- Task { id: n, label: "t" }; }
}
fn main() { A { }; }
"#;
    let art = dump(src, "obs_identity");
    assert!(
        art.contains(
            r#"{"name": "Tasks", "subject": "Tasks", "shape": "id:i;label:s", "payload_hash": "f7d174542aa33437"}"#
        ),
        "the topics row must carry the protocol-vector identity:\n{}",
        art
    );
    // Unhashed: an edit to a payload FIELD changes the topic row
    // but must not change the model shape identity (topology is
    // unchanged).
    let renamed = src
        .replace("label: String", "tag: String")
        .replace("label: \"t\"", "tag: \"t\"");
    let a = dump(src, "obs_identity_a");
    let b = dump(&renamed, "obs_identity_b");
    assert_ne!(
        a.lines().find(|l| l.contains("payload_hash")),
        b.lines().find(|l| l.contains("payload_hash")),
        "a payload field edit must change the observation identity"
    );
    assert_eq!(
        shape_hash(&a),
        shape_hash(&b),
        "a payload field edit must NOT change the model shape_hash"
    );
}
