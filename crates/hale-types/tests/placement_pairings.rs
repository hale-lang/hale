//! The 2026-08-12 placement pairings — check-side rules.
//!
//! Replica keys: `where key == …` filters (Specific AND the new
//! `replica`) require a keyed topic — an unkeyed publish never
//! runs the key match, so a filtered subscriber would silently
//! receive nothing, forever (previously accepted without a word).
//! `replica` additionally needs an Int-family key.
//!
//! Pool affinity: entries naming one pool must agree, and affinity
//! on the main pool has no worker thread to bind.

use hale_types::check_program;

fn errors(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

#[test]
fn key_filters_require_a_keyed_topic() {
    let replica = r#"
        type Job { shard: Int; }
        topic Work { payload: Job; }
        locus W {
            bus { subscribe Work as on_j where key == replica; }
            fn on_j(j: Job) { }
        }
        fn main() { W { }; }
    "#;
    let errs = errors(replica);
    assert!(
        errs.iter().any(|e| e.contains("requires a keyed topic")),
        "replica filter on unkeyed topic rejected: {:?}",
        errs
    );

    // The same rule closes a pre-existing silent trap for Specific
    // filters.
    let specific = r#"
        type Job { shard: Int; }
        topic Work { payload: Job; }
        locus W {
            bus { subscribe Work as on_j where key == 3; }
            fn on_j(j: Job) { }
        }
        fn main() { W { }; }
    "#;
    let errs = errors(specific);
    assert!(
        errs.iter().any(|e| e.contains("requires a keyed topic")),
        "specific filter on unkeyed topic rejected: {:?}",
        errs
    );
}

#[test]
fn replica_filter_needs_an_int_family_key() {
    let src = r#"
        type Job { tag: String; }
        topic Work { payload: Job; keyed_by tag; }
        locus W {
            bus { subscribe Work as on_j where key == replica; }
            fn on_j(j: Job) { }
        }
        fn main() { W { }; }
    "#;
    let errs = errors(src);
    assert!(
        errs.iter().any(|e| e.contains("Int-family key")),
        "replica on a String-keyed topic rejected: {:?}",
        errs
    );

    // Control: a String-keyed SPECIFIC filter stays legal (Gap B).
    let control = r#"
        type Job { tag: String; }
        topic Work { payload: Job; keyed_by tag; }
        locus W {
            params { name: String = "a"; }
            bus { subscribe Work as on_j where key == self.name; }
            fn on_j(j: Job) { }
        }
        fn main() { W { }; }
    "#;
    assert!(
        errors(control).is_empty(),
        "String-keyed specific filters unchanged: {:?}",
        errors(control)
    );
}

#[test]
fn pool_affinity_conflicts_and_main_pool_are_rejected() {
    let conflict = r#"
        locus A { params { n: Int = 0; } }
        locus B { params { n: Int = 0; } }
        main locus App {
            params { a: A = A { }; b: B = B { }; }
            placement {
                a: cooperative(pool = io, cores = 0..=1);
                b: cooperative(pool = io, cores = 2..=3);
            }
        }
        fn main() { App { }; }
    "#;
    let errs = errors(conflict);
    assert!(
        errs.iter().any(|e| e.contains("two different") && e.contains("affinities")),
        "conflicting pool affinity rejected: {:?}",
        errs
    );

    let main_pool = r#"
        locus A { params { n: Int = 0; } }
        main locus App {
            params { a: A = A { }; }
            placement { a: cooperative(core = 3); }
        }
        fn main() { App { }; }
    "#;
    let errs = errors(main_pool);
    assert!(
        errs.iter().any(|e| e.contains("needs a named pool")),
        "affinity on the main pool rejected: {:?}",
        errs
    );

    // Control: one declaring entry + one bare entry naming the
    // pool is fine (the bare entry inherits).
    let ok = r#"
        locus A { params { n: Int = 0; } }
        locus B { params { n: Int = 0; } }
        main locus App {
            params { a: A = A { }; b: B = B { }; }
            placement {
                a: cooperative(pool = io, cores = 0..=1);
                b: cooperative(pool = io);
            }
        }
        fn main() { App { }; }
    "#;
    assert!(
        errors(ok).is_empty(),
        "declaring + inheriting entries coexist: {:?}",
        errors(ok)
    );
}
