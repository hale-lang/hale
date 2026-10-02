//! The documents' target statements are rendered from the capability
//! matrix, never edited (design §4, T6): the book's wasm32 statement in
//! `docs/src/systems/webassembly.md` and the spec's table of refused
//! namespaces in `spec/ffi.md`. This test holds the lines between each
//! region's markers byte-equal to `render_markdown`. To update both
//! after changing a cell:
//!
//! ```text
//! HALE_REGEN_CAPABILITY_DOC=1 cargo test -p hale-types --test capability_doc_matches
//! ```
//!
//! The regeneration is gated behind the variable so it is never done
//! reflexively (the `HALE_REGEN_REGISTRY` precedent).

use std::path::PathBuf;

use hale_types::capability::{render_markdown, DocRegion, DOC_REGION_END};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The file split at the region: the text through its opening marker's
/// line, the lines inside, and the text from its closing marker's line.
fn split(text: &str, region: DocRegion) -> Result<(String, String, String), String> {
    let begin = region.begin();
    let open = text
        .find(&format!("{begin}\n"))
        .ok_or_else(|| format!("{} has no line `{begin}`", region.file()))?;
    let inside = open + begin.len() + 1;
    let close = text[inside..]
        .find(&format!("{DOC_REGION_END}\n"))
        .map(|i| inside + i)
        .ok_or_else(|| format!("{}: the region `{begin}` is not closed by `{DOC_REGION_END}`", region.file()))?;
    if text[close + DOC_REGION_END.len()..].contains(&begin) {
        return Err(format!("{} holds the region `{begin}` twice", region.file()));
    }
    Ok((text[..inside].to_string(), text[inside..close].to_string(), text[close..].to_string()))
}

#[test]
fn every_region_is_the_rendered_matrix() {
    let regen = std::env::var("HALE_REGEN_CAPABILITY_DOC").as_deref() == Ok("1");
    let mut drifted = Vec::new();
    for region in DocRegion::ALL {
        let path = repo_root().join(region.file());
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let (head, on_disk, tail) = split(&text, region).unwrap_or_else(|e| panic!("{e}"));
        let rendered = render_markdown(region);
        if on_disk == rendered {
            continue;
        }
        if regen {
            std::fs::write(&path, format!("{head}{rendered}{tail}")).expect("write the region");
            continue;
        }
        let first = head.lines().count()
            + 1
            + on_disk
                .lines()
                .zip(rendered.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(on_disk.lines().count().min(rendered.lines().count()));
        drifted.push(format!("{} (first differing line: {first})", region.file()));
    }
    assert!(
        drifted.is_empty(),
        "a capability-matrix region differs from the rendered matrix:\n  {}\n\
         The region is generated: regenerate it with HALE_REGEN_CAPABILITY_DOC=1 \
         cargo test -p hale-types --test capability_doc_matches, and review the \
         diff like any other spec change.",
        drifted.join("\n  ")
    );
}

/// A file without its markers fails the test rather than passing as
/// nothing to compare.
#[test]
fn a_missing_marker_is_refused() {
    let region = DocRegion::Wasm32StdlibTable;
    let begin = region.begin();
    assert!(split("no region here\n", region).unwrap_err().contains("has no line"));
    assert!(split(&format!("{begin}\n| a |\n"), region).unwrap_err().contains("is not closed"));
    let twice = format!("{begin}\nx\n{DOC_REGION_END}\n{begin}\ny\n{DOC_REGION_END}\n");
    assert!(split(&twice, region).unwrap_err().contains("twice"));
    let (_, inside, _) = split(&format!("a\n{begin}\nx\n{DOC_REGION_END}\nb\n"), region).unwrap();
    assert_eq!(inside, "x\n");
}

/// The rendering reads every reason and substitute from the cells: a
/// refused namespace's row carries its cell's guidance, and the spec
/// table names exactly the namespaces wasm32 refuses.
#[test]
fn the_rendering_reads_the_cells() {
    use hale_types::capability::{derive_capability_matrix, Capability, TargetClass};
    let m = derive_capability_matrix();
    let table = render_markdown(DocRegion::Wasm32StdlibTable);
    for r in m.behaviours {
        let Capability::StdNamespace(ns) = r.capability else { continue };
        let named = table.contains(&format!("`std::{ns}`"));
        let cell = r.cells.get(TargetClass::Wasm32);
        assert_eq!(named, !cell.is_lower(), "std::{ns}: the table and the cell disagree");
        if let Some(g) = cell.refusal().and_then(|f| f.guidance) {
            assert!(table.contains(g), "std::{ns}: the substitute is not the cell's");
        }
    }
}
