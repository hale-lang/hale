//! Every family lowering reads says what lowering does without a row
//! (F.40 phase 3's exit): `Missing::Required`, a row the snapshot always
//! provides, whose absence is a `CodegenError` pinned by a test that
//! removes it from a lowering view; or `Missing::Total`, where absence is
//! itself the fact and the sentence says which. Never `Error` or `Hole`,
//! which say nothing about lowering, and never a default.
//!
//! The families lowering reads are listed here, so a family that gains
//! a codegen consumer fails until it chooses and joins the list, and a
//! family that leaves lowering fails until it leaves the list.

use std::path::PathBuf;

use hale_graph::{families, Family, Kind, Missing, State};

/// The families lowering reads, by name, in the registry's order.
const LOWERING_READS: &[&str] = &[
    "sync_inference",
    "expression_typing",
    "generics",
    "surfaces",
    "forms",
    "entrypoint",
    "ownership",
    "bus_graph",
    "topics",
    "bindings",
    "dispatch",
    "handler_routing",
    "flows",
    "restart",
    "blocking",
    "alloc_summary",
    "placement",
    "target_capability",
    "lifecycle_order",
    "bus_inert",
    "law_backstops",
];

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// A consumer in codegen: its site is in `hale-codegen`'s sources, or it
/// names codegen as the reader. The harness adapter demanding the view
/// from its snapshot (`demand_lowering`, the `demand` family's consumer)
/// reads no row: it is how lowering gets the view, not a read of it.
fn read_by_lowering(f: &Family) -> bool {
    f.consumers.iter().any(|c| {
        c.who.starts_with("codegen")
            || c.site.is_some_and(|s| s.path.starts_with("crates/hale-codegen/src/") && s.symbol != "demand_lowering")
    })
}

/// Why a family codegen consumes has no lowering read to classify: a
/// desugar's or a reserved family's output is the program itself, and a
/// family codegen still derives itself has its legacy producer there as
/// its inventory, not a row lowering reads.
fn exempt(f: &Family) -> Option<&'static str> {
    if f.kind == Kind::Desugar || f.state == State::Reserved {
        return Some("lowering reads the program it produces, not rows");
    }
    if f.state == State::Migrating && f.legacy.iter().any(|l| l.site.path.starts_with("crates/hale-codegen/")) {
        return Some("codegen still derives it; its legacy producer is the inventory");
    }
    None
}

#[test]
fn the_families_lowering_reads_are_the_listed_ones() {
    let read: Vec<&str> = families().iter().filter(|f| read_by_lowering(f) && exempt(f).is_none()).map(|f| f.name).collect();
    assert_eq!(
        read, LOWERING_READS,
        "a family lowering reads chooses `Missing::Required` (with the test that removes its row) or \
         `Missing::Total` (what no row means), and joins LOWERING_READS"
    );
}

#[test]
fn every_family_lowering_reads_is_required_or_total() {
    let root = workspace_root();
    let mut wrong = Vec::new();
    for name in LOWERING_READS {
        let f = hale_graph::family(name).unwrap_or_else(|| panic!("`{name}` is no family"));
        let total = match f.missing {
            Missing::Required { pinned_by, total } => {
                let text = std::fs::read_to_string(root.join(pinned_by.path)).unwrap_or_default();
                if !text.contains(&format!("fn {}(", pinned_by.symbol)) {
                    wrong.push(format!("`{name}`: its pin `{}` is no fn of {}", pinned_by.symbol, pinned_by.path));
                }
                total
            }
            Missing::Total(t) => Some(t),
            other => {
                wrong.push(format!("`{name}`: lowering reads it, and `{other:?}` says nothing about a missing row there"));
                None
            }
        };
        if let Some(t) = total {
            if !t.starts_with("no ") {
                wrong.push(format!("`{name}`: a total answer says what no row means, starting \"no \": {t}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn only_a_family_lowering_reads_is_required_or_total() {
    let misplaced: Vec<&str> = families()
        .iter()
        .filter(|f| !LOWERING_READS.contains(&f.name))
        .filter(|f| matches!(f.missing, Missing::Required { .. } | Missing::Total(_)))
        .map(|f| f.name)
        .collect();
    assert!(misplaced.is_empty(), "classified for lowering, which reads none of them: {misplaced:?}");
}

#[test]
fn the_exempt_families_are_the_known_ones() {
    let exempt: Vec<(&str, &str)> = families()
        .iter()
        .filter(|f| read_by_lowering(f))
        .filter_map(|f| exempt(f).map(|why| (f.name, why)))
        .collect();
    let names: Vec<&str> = exempt.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["desugar_sequence", "stdlib_surface", "closures"], "{exempt:?}");
}
