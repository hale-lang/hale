//! Typecheck gates for `terminate;` and `release(c: T)`.
//!
//! - `terminate;` ends the *enclosing locus's* own lifecycle, so it
//!   only has meaning inside a locus method body. In a free function
//!   there is no `self`/locus to terminate — rejected.
//! - `release(c: T)` is the death-side bookend of `accept(c: T)`. A
//!   `release` with no matching `accept` of the same child type can
//!   never fire — rejected.

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
fn terminate_in_free_fn_rejected() {
    let src = r#"
fn helper() {
    terminate;
}
fn main() { }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter()
            .any(|m| m.contains("`terminate` is only valid inside a locus method")),
        "expected free-fn terminate rejection, got: {:?}",
        msgs
    );
}

#[test]
fn terminate_in_locus_method_ok() {
    let src = r#"
locus L {
    params { done: Bool = false; }
    fn step() {
        if self.done {
            terminate;
        }
    }
}
fn main() { L { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("terminate")),
        "expected terminate-in-method to typecheck, got: {:?}",
        msgs
    );
}

#[test]
fn release_without_accept_rejected() {
    let src = r#"
locus Child { params { v: Int = 0; } }
locus Parent {
    release(c: Child) { }
}
fn main() { Parent { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("no matching `accept(c: Child)`")),
        "expected release-without-accept rejection, got: {:?}",
        msgs
    );
}

#[test]
fn release_with_matching_accept_ok() {
    let src = r#"
locus Child { params { v: Int = 0; } }
locus Parent {
    accept(c: Child) { }
    release(c: Child) { }
}
fn main() { Parent { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("release")),
        "expected release-with-accept to typecheck, got: {:?}",
        msgs
    );
}
