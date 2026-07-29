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
            Probe::Unresolved(..) => None,
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
            Probe::Unresolved(..) => None,
        },
    );
    assert!(path.is_none(), "clean must reach no allocation site");
}
