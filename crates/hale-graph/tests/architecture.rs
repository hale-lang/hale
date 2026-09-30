//! F.40 architecture canary: `hale-graph` depends on nothing.
//!
//! The crate boundary is what keeps the AST, the checker and codegen
//! out of the core; a rule in a document would not. The same law
//! holds `hale-model` (its own tests/architecture.rs), and phase 1
//! rebuilds `hale-model` on this crate without loosening either.

#[test]
fn hale_graph_depends_on_nothing() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read Cargo.toml");
    let mut in_deps = false;
    let mut offenders = Vec::new();
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            // `[dependencies]` and every `[target.<cfg>.dependencies]`.
            in_deps = t == "[dependencies]"
                || t.starts_with("[dependencies.")
                || (t.starts_with("[target.") && t.contains(".dependencies"));
            continue;
        }
        if in_deps && !t.is_empty() && !t.starts_with('#') {
            offenders.push(t.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "hale-graph must stay dependency-free (generic mechanics only; \
         row meanings live with their families): {offenders:?}"
    );
}
