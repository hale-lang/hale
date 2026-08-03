//! A diverging `or` fallback has no type (#353).
//!
//! `v.get(i) or { break; }` was rejected with "fallback type `()` does
//! not match success type `Int`" — but `break` never yields, so there
//! is no value whose type could match. The rule asked callers to
//! invent a substitute that is provably never used.
//!
//! That is a papercut on its own, and a blocker for the recognized-
//! chain lowering: a desugar over a form cannot invent a typed default
//! for an arbitrary element type, so `or { continue; }` is the only
//! shape available to it.
//!
//! Deliberately conservative: only a block whose LAST statement
//! unconditionally transfers control counts. A conditional `break`
//! still requires a substitute, because the block CAN fall through and
//! then genuinely does need a value.

use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

const V: &str = "@form(vec)\nlocus Nums { capacity { heap items of Int; } }\n";

#[test]
fn an_unconditionally_diverging_fallback_needs_no_substitute() {
    for kw in ["break", "continue"] {
        let ds = errs(&format!(
            "{V}fn main() {{\n\
                 let v = Nums {{ }};\n\
                 v.push(1);\n\
                 let mut i = 0;\n\
                 while i < v.len() {{\n\
                     let it = v.get(i) or {{ {kw}; }};\n\
                     println(it);\n\
                     i = i + 1;\n\
                 }}\n\
             }}"
        ));
        assert!(
            !ds.iter().any(|m| m.contains("fallback type")),
            "`or {{ {}; }}` yields no value, so there is no type to \
             match: {:?}",
            kw,
            ds
        );
    }
}

#[test]
fn a_return_fallback_needs_no_substitute() {
    let ds = errs(&format!(
        "{V}fn first(v: Nums) -> Int {{\n\
             let it = v.get(0) or {{ return 0; }};\n\
             return it;\n\
         }}\n\
         fn main() {{ let v = Nums {{ }}; v.push(3); println(first(v)); }}"
    ));
    assert!(
        !ds.iter().any(|m| m.contains("fallback type")),
        "`or {{ return …; }}` diverges: {:?}",
        ds
    );
}

/// The boundary. A block that CAN fall through still needs a value —
/// otherwise the binding would be uninitialised on the falling-through
/// path, which is the thing the original rule was protecting.
#[test]
fn a_conditional_divergence_still_requires_a_substitute() {
    let ds = errs(&format!(
        "{V}fn main() {{\n\
             let v = Nums {{ }};\n\
             v.push(1);\n\
             let mut i = 0;\n\
             while i < v.len() {{\n\
                 let it = v.get(i) or {{ if i > 5 {{ break; }} }};\n\
                 println(it);\n\
                 i = i + 1;\n\
             }}\n\
         }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("fallback type")),
        "a conditional break can fall through and then needs a value: \
         {:?}",
        ds
    );
}

/// An ordinary typed substitute must keep working unchanged.
#[test]
fn a_valued_substitute_still_works() {
    let ds = errs(&format!(
        "{V}fn main() {{\n\
             let v = Nums {{ }};\n\
             println(v.get(0) or 0);\n\
         }}"
    ));
    assert!(
        !ds.iter().any(|m| m.contains("fallback type")),
        "`or 0` is the ordinary form and must be unaffected: {:?}",
        ds
    );
}
