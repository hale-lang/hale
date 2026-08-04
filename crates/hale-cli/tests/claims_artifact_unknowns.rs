//! GH #382 follow-up review: the topology artifact must record the
//! untyped-receiver unknown class — it changes claim evaluation
//! (the fail-closed backstop), so leaving it out of the hashed
//! model half reopened the model/hash mismatch: introducing
//! `Bridge { }.hop(n)` could flip a claim while `shape_hash`
//! stayed identical.

use std::process::Command;

fn dump(src: &str, tag: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "hale_artifact_unknowns_{}_{}.hl",
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

const TYPED: &str = r#"
locus B { fn work(n: Int) -> Int { return n * 2; } }
locus Bridge {
    params { b: B = B { }; }
    fn hop(n: Int) -> Int { return self.b.work(n); }
}
locus A {
    params { br: Bridge = Bridge { }; }
    fn go(n: Int) -> Int { return self.br.hop(n); }
}
group a_side = { A };
group b_side = { B };
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side); }
}
fn main() { App { }; }
"#;

fn untyped() -> String {
    // Same program, but A reaches Bridge through a receiver the
    // summarizer cannot type.
    TYPED.replace(
        "params { br: Bridge = Bridge { }; }\n    fn go(n: Int) -> Int { return self.br.hop(n); }",
        "fn go(n: Int) -> Int { return Bridge { }.hop(n); }",
    )
}

fn shape_hash(artifact: &str) -> String {
    artifact
        .lines()
        .find(|l| l.contains("\"shape_hash\""))
        .expect("shape_hash line")
        .to_string()
}

/// The untyped-receiver edge appears in `unknowns` with the callee
/// name, so an outside evaluator can apply the same fail-closed
/// rule.
#[test]
fn the_artifact_records_the_untyped_receiver_unknown() {
    let art = dump(&untyped(), "untyped");
    assert!(
        art.contains("untyped_receiver_call:hop")
            && art.contains("\"A::go\""),
        "the unknown must be recorded with its callee:\n{}",
        art
    );
    let art = dump(TYPED, "typed");
    assert!(
        !art.contains("untyped_receiver_call"),
        "a fully-typed program records no such unknown:\n{}",
        art
    );
}

/// Introducing the untyped edge changes `shape_hash` — the unknown
/// lives inside the hashed model half.
#[test]
fn an_untyped_receiver_edge_changes_shape_hash() {
    let typed = dump(TYPED, "hash_typed");
    let untyped = dump(&untyped(), "hash_untyped");
    assert_ne!(
        shape_hash(&typed),
        shape_hash(&untyped),
        "the unknown class must be part of the shape identity"
    );
}
