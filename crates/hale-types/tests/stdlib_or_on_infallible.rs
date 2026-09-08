//! GH #535 (DNA F.8 / F.9, 2026-09-05): stdlib path calls whose
//! fallibility the checker could not see.
//!
//! F.8: the json flat-object readers had no signature rows, so a call
//! typed Unknown and `find_int_field(..) or 0` slid through `hale
//! check` to fail at build with "`or` over unknown path call". Tabled
//! now: the bare call types precisely and the `or` is refused where
//! it is written, naming the fn.
//!
//! F.9: `write_file_append` was `-> Int fallible(IoError)` to the
//! table while the `or` form lowers through the Unit-success channel
//! (the bare legacy call returns an Int status). The row says Unit
//! now, so `... or 0` is a substitute mismatch at check time and
//! `... or discard` / `or handler(err)` are the admitted shapes.

use hale_syntax::parse_source;
use hale_types::check_program;

fn diags(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn or_on_an_infallible_json_reader_is_refused_by_name() {
    let ds = diags(r#"
fn main() {
    let n = std::json::find_int_field("{\"seq\": 3}", "seq") or 0;
    println(n);
}
"#);
    assert!(
        ds.iter().any(|m| m.contains("`std::json::find_int_field` is not fallible") && m.contains("drop the `or`")),
        "expected the infallible-or refusal, got: {:?}",
        ds
    );
}

#[test]
fn bare_json_reader_types_precisely() {
    let ok = diags(r#"
fn main() {
    let n: Int = std::json::find_int_field("{\"seq\": 3}", "seq");
    let s: String = std::json::find_string_field("{\"k\": \"v\"}", "k");
    let b: Bool = std::json::find_bool_field("{\"f\": true}", "f");
    println(n, s, b);
}
"#);
    assert!(ok.iter().all(|m| !m.contains("type error") && !m.contains("mismatch")), "clean: {:?}", ok);
    let bad = diags(r#"
fn main() {
    let s: String = std::json::find_int_field("{\"seq\": 3}", "seq");
    println(s);
}
"#);
    assert!(
        bad.iter().any(|m| m.contains("Int") && m.contains("String")),
        "the Int reader must not flow into a String binding: {:?}",
        bad
    );
}

#[test]
fn write_file_append_or_value_is_a_substitute_mismatch_at_check_time() {
    let ds = diags(r#"
fn main() {
    let n = std::io::fs::write_file_append("/tmp/x", "y") or 0;
    println(n);
}
"#);
    assert!(
        ds.iter().any(|m| m.contains("or") && (m.contains("()") || m.contains("Unit") || m.contains("unit"))),
        "expected a Unit-vs-Int substitute diagnostic, got: {:?}",
        ds
    );
}

#[test]
fn write_file_append_or_discard_and_or_handler_are_admitted() {
    let ds = diags(r#"
locus W {
    params { errs: Int = 0; }
    fn failed(e: IoError) { self.errs = self.errs + 1; }
    fn put(s: String) {
        std::io::fs::write_file_append("/tmp/x", s) or discard;
        std::io::fs::write_file_append("/tmp/x", s) or self.failed(err);
    }
}
fn main() { let w = W { }; w.put("y"); }
"#);
    assert!(
        ds.iter().all(|m| !m.contains("write_file_append")),
        "statement-position or forms must be clean: {:?}",
        ds
    );
}
