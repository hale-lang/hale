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
        a.contains("\"decls\": {") && a.contains("\"A\": ["),
        "decl spans are exported:\n{}",
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
