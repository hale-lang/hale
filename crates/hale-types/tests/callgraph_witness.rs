//! R1 — the path-carrying reachability primitive (#265 step 1).
//!
//! `budget_check`'s fixpoint could tell you a fn allocates; it could
//! not tell you HOW the allocation is reached. `callgraph::
//! witness_path` returns the call chain — the diagnostic shape #265
//! specifies (`on_tick -> format_px -> lotus_str_builder_finish
//! [alloc]`). This pins: a two-hop chain through a helper, the
//! renderer, and the negative (nothing reachable matches).

use hale_types::alloc_summary::{self, FnKey};
use hale_types::callgraph::{self, Probe};

const SRC: &str = r#"
    type P { x: Int; }

    fn leaf_alloc() -> P {
        return P { x: 1 };
    }

    fn mid() -> P {
        return leaf_alloc();
    }

    fn root() -> P {
        return mid();
    }

    fn clean(n: Int) -> Int {
        return n + 1;
    }

    fn main() {
        let p = root();
        let c = clean(2);
        println(p.x + c);
    }
"#;

#[test]
fn witness_path_carries_the_call_chain() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let summary = alloc_summary::summarize_programs(&[&program]);

    let root = FnKey::free_fn("root");
    let path = callgraph::witness_path(
        &summary,
        &root,
        &mut |probe| match probe {
            Probe::Site(_) => Some("alloc".to_string()),
            Probe::Unresolved(..) | Probe::Resolved(..) => None,
        },
    )
    .expect("an alloc is reachable from root");

    // root's body -> mid -> leaf_alloc [alloc]
    let rendered = callgraph::render_witness(&root, &path);
    assert_eq!(rendered, "root -> mid -> leaf_alloc [alloc]");
    // Every interior step names the fn whose body holds the hop.
    assert_eq!(path[0].in_fn, FnKey::free_fn("root"));
    assert_eq!(path[1].in_fn, FnKey::free_fn("mid"));
    assert_eq!(path[2].in_fn, FnKey::free_fn("leaf_alloc"));
}

#[test]
fn witness_path_negative_when_nothing_matches() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let summary = alloc_summary::summarize_programs(&[&program]);
    let root = FnKey::free_fn("clean");
    let path = callgraph::witness_path(
        &summary,
        &root,
        &mut |probe| match probe {
            Probe::Site(_) => Some("alloc".to_string()),
            Probe::Unresolved(..) | Probe::Resolved(..) => None,
        },
    );
    assert!(path.is_none(), "clean must reach no allocation site");
}

/// R2 + #265 phase 2 — the registry's effect column is queryable by
/// path, and the frontier is now CLASSIFIED: every surface entry
/// carries a real effect set, so an assertion can never silently
/// pass an unclassified leaf.
#[test]
fn stdlib_registry_effects_lookup() {
    use hale_types::stdlib_surface::{effects_for, EffectSet, SURFACES};
    // unknown paths stay None (permissive, as before)
    assert_eq!(effects_for(&["std", "crypto", "sha9000"]), None);
    // pure computation
    let sha = effects_for(&["std", "crypto", "sha256"]).expect("sha256");
    assert_eq!(sha, EffectSet::PURE);
    // syscall-class I/O
    let w = effects_for(&["std", "io", "fs", "write_file"]).expect("write_file");
    assert!(w.contains(EffectSet::SYSCALL));
    // sleep is both blocking and a clock effect
    let sl = effects_for(&["std", "time", "sleep"]).expect("sleep");
    assert!(sl.contains(EffectSet::BLOCK) && sl.contains(EffectSet::TIME));
    // nondeterminism classes
    assert!(effects_for(&["std", "rand", "next_int"])
        .expect("rand")
        .contains(EffectSet::ENTROPY));
    assert!(effects_for(&["std", "env", "var"])
        .expect("env")
        .contains(EffectSet::ENV));
    // the whole frontier is classified — no residue
    let unclassified: Vec<&str> = SURFACES
        .iter()
        .flat_map(|s| s.fns.iter())
        .filter(|e| e.effects.is_unclassified())
        .map(|e| e.name)
        .collect();
    assert!(
        unclassified.is_empty(),
        "unclassified stdlib entries remain: {:?}",
        unclassified
    );
}
