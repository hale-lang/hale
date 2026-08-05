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
        // No `println` in either phase: writing to a stream is a
        // syscall (classified since the frontier gained builtins),
        // and an unrelated syscall violation in both phases would
        // muddy what this test is actually isolating — that `alloc`
        // is legal in birth and illegal in run.
        @phase_effects(birth: {alloc}, run: {})
        locus Engine {
            params { seen: Int = 0; }
            birth() {
                let b = Buf { n: 1 };
                self.seen = b.n;
            }
            run() {
                let bad = Buf { n: 2 };
                self.seen = bad.n;
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

/// An empty set FORBIDS everything; an omitted phase is
/// unconstrained. These are opposite meanings and the difference is
/// the whole feature — `run: {}` is what "no dynamic memory after
/// initialization" actually says.
#[test]
fn empty_set_forbids_where_omission_permits() {
    let body = r#"
        type Buf { n: Int; }
        @phase_effects(PHASES)
        locus A {
            params { seen: Int = 0; }
            run() { let b = Buf { n: 1 }; self.seen = b.n; }
        }
        fn main() { A { }; }
    "#;
    let empty = diags_for(&body.replace("PHASES", "run: {}"));
    assert!(
        empty.iter().any(|m| m.contains("phase `run`") && m.contains("alloc")),
        "`run: {{}}` must forbid every class: {:?}",
        empty
    );
    let omitted = diags_for(&body.replace("PHASES", "birth: {alloc}"));
    assert!(
        !omitted.iter().any(|m| m.contains("phase `run`")),
        "a phase not mentioned is unconstrained: {:?}",
        omitted
    );
}

/// A phase naming nothing on the locus was silently skipped, so a
/// typo produced a contract that was never checked and never
/// reported. Fails closed now, like every other incompleteness in
/// this system.
#[test]
fn unknown_phase_name_is_rejected() {
    let src = r#"
        @phase_effects(not_a_phase: {})
        locus A { params { n: Int = 0; } run() { self.n = 1; } }
        fn main() { A { }; }
    "#;
    let ds = diags_for(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("does not declare"))
        .unwrap_or_else(|| panic!("expected an unknown-phase error: {:?}", ds));
    assert!(
        hit.contains("not_a_phase") && hit.contains("`run`"),
        "the diagnostic must name the bad phase and what IS available: {}",
        hit
    );
}

/// The six lifecycle names stay legal even when the hook is
/// implicit: a locus with only `params` still has a birth, and the
/// canonical `@phase_effects(birth: {alloc}, run: {})` line must not
/// error just because no `birth()` block is written out.
#[test]
fn implicit_lifecycle_phases_are_not_flagged_as_unknown() {
    let src = r#"
        @phase_effects(birth: {alloc}, dissolve: {})
        locus A { params { n: Int = 0; } run() { self.n = 1; } }
        fn main() { A { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("does not declare")),
        "implicit lifecycle hooks are real phases: {:?}",
        ds
    );
}

/// A handler is addressable by name — that is how a per-message
/// contract is written.
#[test]
fn handler_name_works_as_a_phase() {
    let src = r#"
        type Ev { n: Int; }
        topic T { payload: Ev; subject: "t"; }
        @phase_effects(on_ev: {})
        locus A {
            bus { subscribe T as on_ev; }
            fn on_ev(e: Ev) { let n = std::io::fs::file_size("/x") or 0; }
        }
        locus P { bus { publish T; } fn go() { T <- Ev { n: 1 }; } }
        main locus App { params { a: A = A { }; p: P = P { }; } }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("phase `on_ev`") && m.contains("syscall")),
        "a handler named as a phase must be checked: {:?}",
        ds
    );
}

// =====================================================================
// #392 §8 — user classes join the phase contract's closed set
// =====================================================================

/// CANARY: a phase reaching a declared USER-class carrier without
/// listing the class violates — the hardcoded built-in list made
/// the contract blind to the classes a program declares itself.
#[test]
fn a_user_class_carrier_violates_an_unlisted_phase() {
    let src = r#"
        effect money;
        @effects(is: {money})
        fn charge(n: Int) -> Int { return n; }
        @phase_effects(run: {})
        locus Engine {
            params { seen: Int = 0; }
            run() {
                self.seen = charge(1);
            }
        }
        fn main() { Engine { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("phase `run`")
            && m.contains("money")),
        "an unlisted user class reached in the phase must violate: {:?}",
        ds
    );
}

/// CONTROL: listing the user class in the phase's allowed set
/// permits it — and parses (rejecting user classes at parse was
/// the other half of the deficiency).
#[test]
fn a_listed_user_class_is_allowed_in_the_phase() {
    let src = r#"
        effect money;
        @effects(is: {money})
        fn charge(n: Int) -> Int { return n; }
        @phase_effects(run: {money})
        locus Engine {
            params { seen: Int = 0; }
            run() {
                self.seen = charge(1);
            }
        }
        fn main() { Engine { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("phase `run`")
            && m.contains("money")),
        "a listed user class must be allowed in the phase: {:?}",
        ds
    );
}
