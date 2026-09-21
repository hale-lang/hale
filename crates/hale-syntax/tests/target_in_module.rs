//! GH #901: `target NAME { }` inside a `module { }` is refused where
//! it is written.
//!
//! A module is a namespace and not an analysis boundary, so nearly
//! everything means the same thing one brace deeper (GH #825, GH
//! #884). `target` was the exception that said nothing: it is a
//! program-level build directive, and every consumer of it —
//! `check.rs`'s wasm gate, `desugar`'s wasm-entry detection, the
//! stdlib gating — walks `program.items` only. So `target wasm { }`
//! one brace deeper was silently INERT: the same program that was
//! gated at the top level ("`std::process::pid` unavailable under
//! `target wasm`") reported `ok`, with the declaration sitting there
//! doing nothing.
//!
//! The ruling (2026-09-20, GH #911): refuse it at the parser. A
//! stated directive that does nothing is worse than a refusal, and
//! honouring it at depth would change what the BUILD does rather than
//! what a diagnostic says.
//!
//! The programs here are plain escaped string literals, not raw
//! strings: `hale-corpus` harvests `r#"…"#` literals out of test
//! files into the corpus-wide properties (the parse sweep among
//! them), and these are written to be REFUSED.

use hale_syntax::parse_source;

const MSG: &str =
    "`target` is a program-level declaration; move it to the top level";

/// Every diagnostic, as `line:col message`.
fn diags(src: &str) -> Vec<String> {
    match parse_source(src) {
        Ok(_) => Vec::new(),
        Err(ds) => ds
            .iter()
            .map(|d| {
                let (line, col) = d.span.line_col(src);
                format!("{}:{} {}", line, col, d.message)
            })
            .collect(),
    }
}

fn only_diag(src: &str) -> String {
    let ds = diags(src);
    assert_eq!(
        ds.len(),
        1,
        "expected exactly one diagnostic, got {}:\n{}",
        ds.len(),
        ds.join("\n")
    );
    ds.into_iter().next().unwrap()
}

/// The issue's shape: one brace deeper, refused at the keyword.
#[test]
fn a_module_nested_target_is_refused() {
    let src = "module inner {\n    target wasm { }\n}\n\nfn main() { }\n";
    assert_eq!(only_diag(src), format!("2:5 {}", MSG));
}

/// Nesting depth is not a loophole: the counter is a depth, not a
/// flag.
#[test]
fn a_target_two_modules_deep_is_refused() {
    let src = "module outer {\n\
               \x20   module inner {\n\
               \x20       target browser_js { }\n\
               \x20   }\n\
               }\n\
               \n\
               fn main() { }\n";
    assert_eq!(only_diag(src), format!("3:9 {}", MSG));
}

/// The control, and the behaviour that must not move: a top-level
/// `target` is a declaration like any other, including when a module
/// was parsed before it.
#[test]
fn a_top_level_target_still_parses() {
    let src = "target wasm { }\n\
               \n\
               module inner {\n\
               \x20   fn helper() -> Int { return 1; }\n\
               }\n\
               \n\
               fn main() { }\n";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
    let prog = parse_source(src).expect("parses");
    assert_eq!(prog.items.len(), 3, "target, module, main");
}

/// A module declared AFTER the target is not the interesting order —
/// this one is: the directive comes last, so a depth counter left
/// raised by the module before it would refuse a legal program.
#[test]
fn a_target_after_a_module_still_parses() {
    let src = "module inner {\n\
               \x20   fn helper() -> Int { return 1; }\n\
               }\n\
               \n\
               target wasm { }\n\
               \n\
               fn main() { }\n";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

/// The error path of the same counter. A module item that does not
/// parse is recovered from at the TOP level, so the depth has to be
/// restored on the way out or a later top-level `target` is refused
/// for the wrong reason.
///
/// `recover_to_top_level` resumes at a `locus` / `type` / `const` /
/// `fn` / `module` keyword and `target` is a contextual one, so the
/// shape needs a declaration BETWEEN the broken module and the
/// directive — otherwise recovery skips the `target` itself and the
/// mistake is invisible. With the depth left raised, this program
/// reports two diagnostics and the second one is about a line that
/// is perfectly legal.
#[test]
fn a_failed_module_body_does_not_refuse_a_later_target() {
    let src = "module inner {\n\
               \x20   fn broken() { let x = ; }\n\
               }\n\
               \n\
               type Marker { n: Int = 0; }\n\
               \n\
               target wasm { }\n\
               \n\
               fn main() { }\n";
    let d = only_diag(src);
    assert!(
        d.contains("expected expression"),
        "the `let` is the only mistake here: {d}"
    );
    assert!(!d.contains("program-level"), "{d}");
}
