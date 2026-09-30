//! v1.x-VIOLATE (F.27) — typecheck rules for inline closures
//! and the `violate` statement.

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn canonical_shape_typechecks_clean() {
    // The error-check-fn pattern from F.27 / styleguide pattern 7.
    let src = r#"
locus L {
    params { last_error: String = ""; }
    closure fatal_io { captures: last_error; epoch inline; }
    fn handle(detail: String) -> Int {
        self.last_error = detail;
        violate fatal_io;
        return 0;
    }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("violate") && !m.contains("captures")),
        "expected clean typecheck, got: {:?}",
        msgs
    );
}

#[test]
fn self_draining_resolves_as_bool() {
    let src = r#"
locus L {
    closure fatal { epoch inline; }
    fn step() {
        if !self.draining {
            let _ = 0;
        }
    }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("draining") && !m.contains("no field")),
        "expected self.draining to typecheck, got: {:?}",
        msgs
    );
}

#[test]
fn violate_in_free_fn_rejected() {
    let src = r#"
fn helper() {
    violate fatal;
}
fn main() { }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("free fns can't use `violate`")),
        "expected free-fn rejection, got: {:?}",
        msgs
    );
}

#[test]
fn violate_in_on_failure_rejected() {
    let src = r#"
locus Child { }
locus Parent {
    closure fatal { epoch inline; }
    accept(c: Child) { }
    on_failure(c: Child, err: ClosureViolation) {
        violate fatal;
    }
}
fn main() { Parent { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("not allowed inside an `on_failure` body")),
        "expected on_failure rejection, got: {:?}",
        msgs
    );
}

#[test]
fn violate_unknown_closure_rejected() {
    let src = r#"
locus L {
    closure fatal { epoch inline; }
    fn step() {
        violate ghost;
    }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("has no closure named `ghost`")),
        "expected unknown-closure rejection, got: {:?}",
        msgs
    );
}

#[test]
fn violate_non_inline_closure_rejected() {
    let src = r#"
locus L {
    params { x: Int = 0; }
    closure check { self.x ~~ self.x within 0; epoch tick; }
    fn step() {
        violate check;
    }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("is not declared `epoch inline`")),
        "expected non-inline rejection, got: {:?}",
        msgs
    );
}

#[test]
fn inline_closure_with_assertion_rejected() {
    let src = r#"
locus L {
    params { x: Int = 0; }
    closure bad { self.x ~~ self.x within 0; epoch inline; }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("must omit the assertion")),
        "expected inline-with-assertion rejection, got: {:?}",
        msgs
    );
}

#[test]
fn captures_on_non_inline_closure_rejected() {
    let src = r#"
locus L {
    params { x: Int = 0; }
    closure check {
        self.x ~~ self.x within 0;
        captures: x;
        epoch tick;
    }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| {
            m.contains("`captures:` is meaningful only on `epoch inline` closures")
        }),
        "expected captures-non-inline rejection, got: {:?}",
        msgs
    );
}

#[test]
fn captures_missing_field_rejected() {
    let src = r#"
locus L {
    params { x: Int = 0; }
    closure fatal { captures: ghost; epoch inline; }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("references field `ghost`")),
        "expected missing-field rejection, got: {:?}",
        msgs
    );
}

#[test]
fn assertion_less_non_inline_rejected() {
    let src = r#"
locus L {
    closure stub { }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("missing assertion")),
        "expected missing-assertion rejection, got: {:?}",
        msgs
    );
}

#[test]
fn violate_with_payload_typechecks() {
    let src = r#"
locus L {
    closure fatal { epoch inline; }
    fn step() {
        violate fatal with 42;
    }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("violate")),
        "expected violate-with-payload to typecheck clean, got: {:?}",
        msgs
    );
}

/// A failing child is routed to its parent's `on_failure` by the child's
/// locus type, and the first handler declared for that type runs. A
/// second handler for the same type can never run — whatever its error
/// param or body says — so it is refused where it stands, and the note
/// points at the handler that runs.
#[test]
fn a_second_on_failure_for_one_child_type_is_refused() {
    let src = r#"
locus Child {
    params { n: Int = 0; }
}
locus Parent {
    params { c: Child = Child { }; }
    on_failure(c: Child, err: ClosureViolation) {
        restart (c);
    }
    on_failure(c: Child, err: ClosureViolation) {
        quarantine (c);
    }
}
fn main() { Parent { }; }
"#;
    let prog = parse_source(src).expect("parse failed");
    let diags: Vec<_> = check_program(&prog)
        .into_iter()
        .filter(|d| d.message.contains("already has an `on_failure` for `Child`"))
        .collect();
    assert_eq!(diags.len(), 1, "one refusal, at the second handler");
    let d = &diags[0];
    assert!(d.is_error(), "an error, not a warning: {}", d.message);
    assert!(d.message.contains("can never run"), "{}", d.message);
    // located at the second handler, pointing at the first
    let second = src.rfind("on_failure(c: Child").unwrap();
    let first = src.find("on_failure(c: Child").unwrap();
    assert_eq!(d.span.start.as_usize(), second, "at the handler that never runs");
    assert!(
        d.related.iter().any(|r| r.span.start.as_usize() == first && r.label.contains("runs")),
        "the note names the handler that runs: {:?}",
        d.related
    );
}

/// Handlers for different child types are each the one that runs for
/// their type.
#[test]
fn on_failure_handlers_for_different_child_types_are_clean() {
    let src = r#"
locus Alpha {
    params { n: Int = 0; }
}
locus Zeta {
    params { n: Int = 0; }
}
locus Parent {
    params { a: Alpha = Alpha { }; z: Zeta = Zeta { }; }
    on_failure(c: Zeta, err: ClosureViolation) {
        restart (c);
    }
    on_failure(c: Alpha, err: ClosureViolation) {
        quarantine (c);
    }
}
fn main() { Parent { }; }
"#;
    let msgs = check(src);
    assert!(msgs.iter().all(|m| !m.contains("on_failure")), "{:?}", msgs);
}

/// A handler the signature rules already refuse is not the one that
/// runs, so it takes no slot: the well-formed handler after it is not
/// reported as a duplicate.
#[test]
fn a_malformed_handler_does_not_make_the_next_a_duplicate() {
    let src = r#"
locus Kid {
    params { n: Int = 0; }
}
locus Parent {
    params { k: Kid = Kid { }; }
    on_failure(x: Kid) { }
    on_failure(x: Kid, err: ClosureViolation) { }
}
fn main() { Parent { }; }
"#;
    let msgs = check(src);
    assert!(msgs.iter().any(|m| m.contains("takes exactly two params")), "the arity error stands: {:?}", msgs);
    assert!(msgs.iter().all(|m| !m.contains("already has an `on_failure`")), "{:?}", msgs);
}

/// A closure has one epoch (F.40 phase 0): `ClosureDecl::epoch` is the
/// one rule the checker and lowering read, and the parser refuses a
/// second clause so "the last clause" and "any clause" never differ.
#[test]
fn a_second_epoch_clause_is_refused_by_the_parser() {
    let src = r#"
locus L {
    closure c { epoch inline; epoch dissolve; }
    fn step() { violate c; }
}
fn main() { L { }; }
"#;
    let errs = hale_syntax::parse_source(src)
        .err()
        .expect("a second epoch clause is a parse error");
    assert!(
        errs.iter()
            .any(|d| d.message.contains("closure `c` declares 2 `epoch` clauses")),
        "got: {:?}",
        errs.iter().map(|d| d.message.clone()).collect::<Vec<_>>()
    );
}
