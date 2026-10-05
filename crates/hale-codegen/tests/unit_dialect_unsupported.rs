//! GH #1076, step U1: lowering refuses the unit dialect.
//!
//! The checker refuses a `unit` declaration, a scalar `type` and a
//! quantity literal until the dialect's rows land, so a checked program
//! never brings one to lowering. A build that skips the check (as this
//! harness does) is refused with an error naming the declaration and
//! where it is, never a panic and never a silent skip.

use hale_codegen::{build_executable_with_options, CodegenError};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn refusal(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_unit_dialect_{}_{}", name, std::process::id()));
    let err = build_executable_with_options(&program, &bin, &[], &build_opts::options())
        .expect_err("the unit dialect is not lowered");
    let _ = std::fs::remove_file(&bin);
    match err {
        CodegenError::UnsupportedAt(msg, span) => format!("{msg} @ {}", span.slice(src)),
        other => panic!("{name}: expected a located refusal, got {other}"),
    }
}

#[test]
fn each_form_is_refused_by_name() {
    let cases = [
        ("unit", "unit tick;\nfn main() { println(1); }\n", "unit `tick`", "unit tick;"),
        (
            "scalar",
            "type Money = quantity Int in cent;\nfn main() { println(1); }\n",
            "type `Money`",
            "type Money = quantity Int in cent;",
        ),
        (
            "literal",
            "fn main() {\n    let fee = 3bp;\n    println(1);\n}\n",
            "quantity literal `3bp`",
            "3bp",
        ),
    ];
    for (name, src, what, at) in cases {
        let got = refusal(name, src);
        assert_eq!(
            got,
            format!(
                "{what}: the unit dialect is not lowered yet (GH #1076); `hale check` refuses it @ {at}"
            ),
            "{name}"
        );
    }
}
