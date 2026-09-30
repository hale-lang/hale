//! `spec/registry.md` is rendered from the registry, never edited.
//!
//! The spec is the contract and the registry is data; the two cannot
//! drift because this test holds them byte-equal. To update the
//! document after changing the table:
//!
//! ```text
//! HALE_REGEN_REGISTRY=1 cargo test -p hale-graph --test registry_matches_spec
//! ```
//!
//! The regeneration is gated behind the variable so it is never done
//! reflexively (the `HALE_REGEN_CLAIM_DIAGS` precedent).

use std::path::PathBuf;

fn spec_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/registry.md")
}

#[test]
fn spec_registry_md_is_the_rendered_registry() {
    let rendered = hale_graph::render_markdown();
    let path = spec_path();
    if std::env::var("HALE_REGEN_REGISTRY").as_deref() == Ok("1") {
        std::fs::write(&path, &rendered).expect("write spec/registry.md");
        return;
    }
    let on_disk =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    if on_disk != rendered {
        let first_diff = on_disk
            .lines()
            .zip(rendered.lines())
            .position(|(a, b)| a != b)
            .map(|i| i + 1)
            .unwrap_or(on_disk.lines().count().min(rendered.lines().count()) + 1);
        panic!(
            "spec/registry.md differs from the rendered registry (first \
             differing line: {first_diff}). It is generated: regenerate it with \
             HALE_REGEN_REGISTRY=1 cargo test -p hale-graph --test \
             registry_matches_spec, and review the diff like any other \
             spec change."
        );
    }
}
