//! F.42: a value-returning fn that may violate is
//! `fallible(ClosureViolation)`.
//!
//! The stdlib's two such methods, `BytesBuilder.snapshot` and `finish`,
//! carry the declaration, and their callers follow the bare-call law
//! (GH #738) with the violation as the error: the message names
//! `ClosureViolation` (it read `(?)` while the checker resolved the
//! stdlib's signatures without the builtin type), and the `err` an `or`
//! sees at the call is typed.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;
use hale_syntax::Diag;

fn diags(src: &str) -> Vec<Diag> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
}

fn errors(src: &str) -> Vec<String> {
    diags(src).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect()
}

#[test]
fn a_bare_snapshot_names_the_violation_it_can_fail_with() {
    let src = r#"
main locus App {
    run() {
        let b = std::bytes::BytesBuilder { };
        b.append_str("x");
        let s = b.snapshot();
        println(len(s));
    }
}
fn main() { App { }; }
"#;
    let errs = errors(src);
    assert_eq!(errs.len(), 1, "{errs:#?}");
    assert!(
        errs[0].starts_with("`b.snapshot` can fail (ClosureViolation) and this call says nothing about it"),
        "{errs:#?}"
    );
}

#[test]
fn err_at_a_finish_call_is_the_violation() {
    // A handler reads `err.closure` at a `finish()` site: typed, so the
    // read is accepted and a field the record does not have is refused.
    let src = r#"
fn note(e: ClosureViolation) -> Bytes {
    println("builder failed: ", e.closure, " in ", e.locus);
    return std::bytes::from_string("");
}
main locus App {
    run() {
        let b = std::bytes::BytesBuilder { };
        let t = b.finish() or note(err);
        let u = b.snapshot() or { println(err.closure, " ", err.diff); std::bytes::from_string("") };
        let s = b.snapshot() or raise;
        println(len(t) + len(u) + len(s));
    }
}
fn main() { App { }; }
"#;
    assert_eq!(errors(src), Vec::<String>::new());

    let wrong = r#"
main locus App {
    run() {
        let b = std::bytes::BytesBuilder { };
        let u = b.finish() or { println(err.last_error); std::bytes::from_string("") };
        println(len(u));
    }
}
fn main() { App { }; }
"#;
    let errs = errors(wrong);
    assert!(errs.iter().any(|m| m.contains("no field `last_error` on `ClosureViolation`")), "{errs:#?}");
}
