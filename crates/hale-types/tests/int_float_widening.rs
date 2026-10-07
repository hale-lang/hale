//! F.23's Int → Float widening at the let ascription and the
//! data-type field init (a downstream handoff: `let whole_f: Float =
//! whole;` refused).
//!
//! spec/types.md § "Numeric coercion" names four surfaces. Codegen
//! has widened at all four since the rule was made (`sitofp` at the
//! binding and at `populate_user_type_fields`), but the checker's let
//! and struct-literal field checks never did, so `let a: Float = 3;`
//! and `Cfg { timeout: n }` were refused before codegen saw them.
//!
//! The refusals below are the rule's edges and stay refused, each with
//! the message it had: a quantity is not an `Int` (`5ms` is a
//! `Duration`), a range is not one either, `Decimal` never widens,
//! `Float → Int` is explicit, and a locus literal's param override does
//! not widen (codegen has no `sitofp` there, as for a param default).
//! The accepted programs run in tests/hale/float_widening_surfaces_test.hl.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program).into_iter().map(|d| d.message).collect()
}

#[test]
fn an_int_widens_into_a_float_ascription_and_a_float_field() {
    let ds = errs(
        "type Cfg { timeout: Float; }\n\
         fn main() {\n\
             let n = 3;\n\
             let a: Float = 3;\n\
             let e: Float = n;\n\
             let d = Cfg { timeout: n };\n\
             let l = Cfg { timeout: 4 };\n\
             println(to_string(a + e + d.timeout + l.timeout));\n\
         }",
    );
    assert!(ds.is_empty(), "the let and field widenings are legal: {ds:?}");
}

#[test]
fn a_quantity_is_not_an_int_for_the_widening() {
    let ds = errs(
        "type Cfg { timeout: Float; }\n\
         fn main() {\n\
             let f: Float = 5ms;\n\
             let c = Cfg { timeout: 5ms };\n\
             println(to_string(f + c.timeout));\n\
         }",
    );
    assert_eq!(
        ds,
        [
            "let `f`: expected `Float`, got `Duration`",
            "type `Cfg`: field `timeout` expects `Float`, got `Duration`",
        ]
    );
}

#[test]
fn the_widening_stays_one_way_and_int_only() {
    let ds = errs(
        "type Byte = Int { range: 0..256; }\n\
         locus W { params { rate: Float = 1.0; } }\n\
         fn main() {\n\
             let b: Byte = 7;\n\
             let k: Float = b;\n\
             let x: Decimal = 3;\n\
             let y: Int = 2.5;\n\
             let w = W { rate: 3 };\n\
             println(to_string(k) + to_string(x) + to_string(y) + to_string(w.rate));\n\
         }",
    );
    assert_eq!(
        ds,
        [
            "let `k`: expected `Float`, got `Byte`",
            "let `x`: expected `Decimal`, got `Int`",
            "let `y`: expected `Int`, got `Float`",
            "locus `W`: field `rate` expects `Float`, got `Int`",
        ]
    );
}
