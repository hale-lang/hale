//! GH #265 step 6 — phase-indexed effect contracts.
//!
//! The lifecycle model makes expressible what a function-level
//! effect system cannot say: "no dynamic memory after
//! initialization" (the DO-178 discipline) IS "alloc allowed in
//! birth, forbidden in run and handlers".

fn diags_for(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn alloc_in_birth_allowed_but_not_in_run() {
    let src = r#"
        type Buf { n: Int; }
        @phase_effects(birth: {alloc}, run: {})
        locus Engine {
            params { seen: Int = 0; }
            birth() {
                let b = Buf { n: 1 };
                println(b.n);
            }
            run() {
                let bad = Buf { n: 2 };
                println(bad.n);
            }
        }
        fn main() { Engine { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("Engine::birth")),
        "birth declares {{alloc}} — it must be allowed there: {:?}",
        ds
    );
    assert!(
        ds.iter().any(|m| m.contains("phase `run`")
            && m.contains("Engine::run")
            && m.contains("alloc")),
        "an allocation in run must violate the phase contract: {:?}",
        ds
    );
}

#[test]
fn clean_phases_pass() {
    let src = r#"
        type Buf { n: Int; }
        @phase_effects(birth: {alloc}, run: {})
        locus Engine {
            params { seen: Int = 0; }
            birth() {
                let b = Buf { n: 1 };
                println(b.n);
            }
            run() {
                self.seen = self.seen + 1;
            }
        }
        fn main() { Engine { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("phase contract")
            || m.contains("phase `run`")),
        "a scalar-only run must satisfy run: {{}}: {:?}",
        ds
    );
}

/// A phase may allow some classes and forbid others.
#[test]
fn phase_allows_named_classes_only() {
    let src = r#"
        type Ev { n: Int; }
        topic T { payload: Ev; subject: "t"; }
        @phase_effects(run: {alloc, publish})
        locus Emitter {
            bus { publish T; }
            run() {
                T <- Ev { n: 1 };
            }
        }
        @phase_effects(run: {alloc})
        locus Quiet {
            bus { publish T; }
            run() {
                T <- Ev { n: 2 };
            }
        }
        fn main() { Emitter { }; Quiet { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("Emitter::run")),
        "publish is allowed for Emitter: {:?}",
        ds
    );
    assert!(
        ds.iter().any(|m| m.contains("Quiet::run")
            && m.contains("publish")),
        "publish is NOT allowed for Quiet: {:?}",
        ds
    );
}
