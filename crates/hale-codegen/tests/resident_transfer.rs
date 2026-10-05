//! What a bus handler hands a resident it starts (GH #712).
//!
//! A handler builds rows, starts a child that stays (an accepted
//! resident), and returns. Everything the handler built belongs to that
//! dispatch and is reclaimed when it returns; the resident outlives it.
//! The rule is one sentence in `spec/semantics.md` ("A resident and what a
//! handler hands it") and this file pins both sides of it on programs with
//! nothing of DNA in them:
//!
//!  * **Retained by name is refused, with a location.** A `@form(vec)` a
//!    handler built is a locus; handed to a resident's field it is a
//!    borrow, and a resident the handler's frame does not own outlives it.
//!    The diagnostic points at the argument, names GH #712 and gives the
//!    working shape. Passing it through an alias, through a helper method
//!    on `self`, or into a resident's method that keeps it is refused the
//!    same way (`borrow_lifetime.rs` decides all of them from position).
//!  * **Value-shaped data is copied, and reading it later is sound.** A
//!    String, a row, a payload's fields handed to a resident are copied
//!    into the resident's own storage at the store. The resident reads
//!    them after the handler has returned and the arena has churned, under
//!    AddressSanitizer with the chunk pool off, and nothing is reported.
//!  * **Child-local copying is the working shape for a container.** A
//!    resident that reads the handed container only in `birth()` (a
//!    birth-scoped borrow, sound: the instantiation runs inside the frame
//!    that owns the container) copies its rows into a `@form(vec)` of its
//!    own, and reads them long after.
//!
//! This is not #704, which was a cell queued for a resident that had
//! already ended; nothing here ends a resident early.

use std::process::Command;

use hale_syntax::parse_source;

#[path = "support/harness.rs"]
mod harness;

const PRELUDE: &str = r#"
type Row { name: String; n: Int; }
type Go { n: Int; text: String; row: Row; }

@form(vec)
locus Rows { capacity { heap items of Row; } }

fn churn_str(k: Int) -> String { return to_string(k) + "yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy"; }
"#;

/// A resident `Reader` (given by `reader`), the owner that accepts it and
/// starts one from a bus handler (`handler`), and a pinned publisher that
/// sends one message. The reader churns the arena, then prints.
fn program(reader: &str, handler: &str) -> String {
    format!(
        r#"{PRELUDE}
{reader}
locus Owner {{
    params {{ _u: Int = 0; }}
    accept(c: Reader) {{ }}
    bus {{ subscribe "go" as on_go of type Go; }}
    fn on_go(g: Go) {{ let x = g.n; {handler} }}
    run() {{ std::time::sleep(300ms); }}
}}
locus Pub {{
    bus {{ publish "go" of type Go; }}
    run() {{
        std::time::sleep(50ms);
        "go" <- Go {{ n: 7, text: "payload-text-qqqqqqqqqqqqqqqqqqqqqqqq", row: Row {{ name: "payload-row-qqqqqqqqqqqqqqqqqqqq", n: 7 }} }};
        std::time::sleep(600ms);
    }}
}}
main locus Root {{ params {{ o: Owner = Owner {{ }}; p: Pub = Pub {{ }}; }} placement {{ p: pinned; }} }}
fn main() {{ Root {{ }}; }}
"#
    )
}

const CHURN: &str = "let mut k = 0; while k < 400 { let s = churn_str(k); k = k + 1; }";

/// What `hale build` refuses: the whole-program check and the borrow rule.
fn build_diags(program: &hale_syntax::ast::Program) -> Vec<hale_syntax::error::Diag> {
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), program);
    let bundle = hale_types::Bundle::new(programs);
    hale_types::check_bundle_for_build(&bundle, false)
}

fn errors(src: &str) -> Vec<(String, usize, usize)> {
    let program = parse_source(src).expect("parse");
    build_diags(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message.clone(), d.span.start.as_usize(), d.span.end.as_usize()))
        .collect()
}

/// The program is refused, exactly once, at `at` in the source, by a
/// diagnostic naming GH #712 and the handler's remedy.
fn assert_refused_at(src: &str, at: &str) {
    let errs = errors(src);
    assert_eq!(errs.len(), 1, "one diagnostic expected: {errs:#?}");
    let (msg, start, end) = &errs[0];
    assert_eq!(&src[*start..*end], at, "the diagnostic is located at the escaping argument: {msg}");
    for needle in ["would hold a borrow that does not outlive it", "GH #712", "birth()"] {
        assert!(msg.contains(needle), "the diagnostic names `{needle}`: {msg}");
    }
}

/// Build under ASan with the chunk pool off, run, return stdout; a
/// sanitizer report or a crash fails the test.
fn run_asan(tag: &str, src: &str) -> String {
    let program = parse_source(src).expect("parse");
    let clean: Vec<_> = build_diags(&program).into_iter().filter(|d| d.is_error()).collect();
    assert!(clean.is_empty(), "the program must check clean: {clean:?}");
    let bin = harness::unique_bin(tag);
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{tag}: {:?}\n{stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{tag}: {stderr}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn reader_over_rows(field: &str) -> String {
    format!(
        r#"locus Reader {{ params {{ {field}: Rows; }} run() {{ {CHURN} let g = self.{field}.get(0) or Row {{ name: "MISSING", n: -1 }}; println(g.name); }} }}"#
    )
}

const BUILD_ROWS: &str = r#"let rows = Rows { }; rows.push(Row { name: "row-" + to_string(x) + "zzzzzzzzzzzzzzzzzzzzzzzz", n: x });"#;

#[test]
fn a_container_handed_to_a_resident_by_name_is_refused_where_it_escapes() {
    let src = program(&reader_over_rows("rows"), &format!("{BUILD_ROWS} Reader {{ rows: rows }};"));
    assert_refused_at(&src, "rows");
}

#[test]
fn an_alias_of_the_container_is_the_same_escape() {
    let src = program(&reader_over_rows("rows"), &format!("{BUILD_ROWS} let again = rows; Reader {{ rows: again }};"));
    assert_refused_at(&src, "again");
}

#[test]
fn a_helper_method_of_the_owner_that_hands_it_on_is_refused_at_the_call() {
    let reader = reader_over_rows("rows");
    let src = program(&reader, &format!("{BUILD_ROWS} self.start(rows);")).replace(
        "fn on_go(g: Go)",
        "fn start(rows: Rows) { Reader { rows: rows }; }\n    fn on_go(g: Go)",
    );
    let errs = errors(&src);
    assert_eq!(errs.len(), 1, "{errs:#?}");
    assert!(errs[0].0.contains("is a parameter of") && errs[0].0.contains("At the call in `on_go`"), "{}", errs[0].0);
}

#[test]
fn a_string_a_row_and_a_payload_handed_to_a_resident_are_copied_and_read_later() {
    for (tag, field, ty, init, print) in [
        ("string", "label", "String", r#"let s = "label-" + to_string(x) + "zzzzzzzzzzzzzzzzzzzzzzzzzz"; Reader { label: s };"#, "self.label"),
        ("row", "row", "Row", r#"let r = Row { name: "row-" + to_string(x) + "zzzzzzzzzzzzzzzzzzzzzzzzzz", n: x }; Reader { row: r };"#, "self.row.name"),
        ("payload string", "label", "String", "Reader { label: g.text };", "self.label"),
        ("payload row", "row", "Row", "Reader { row: g.row };", "self.row.name"),
    ] {
        let default = if ty == "String" { r#""""# } else { r#"Row { name: "", n: 0 }"# };
        let reader = format!(
            "locus Reader {{ params {{ {field}: {ty} = {default}; }} run() {{ {CHURN} println({print}); }} }}"
        );
        let out = run_asan(&format!("resident_{}", tag.replace(' ', "_")), &program(&reader, init));
        assert!(out.contains("zzzzzzzzzz") || out.contains("qqqqqqqqqq"), "{tag}: the value survives the handler and the churn: {out:?}");
    }
}

#[test]
fn a_resident_that_copies_the_container_in_birth_reads_it_after_the_handler_returned() {
    let reader = format!(
        r#"locus Reader {{
    params {{ src: Rows; mine: Rows = Rows {{ }}; }}
    birth() {{
        let mut i = 0;
        while i < self.src.len() {{
            let r = self.src.get(i) or Row {{ name: "?", n: -1 }};
            self.mine.push(Row {{ name: r.name, n: r.n }});
            i = i + 1;
        }}
    }}
    run() {{ {CHURN} let g = self.mine.get(0) or Row {{ name: "MISSING", n: -1 }}; println(g.name); }}
}}"#
    );
    let out = run_asan("resident_birth_copy", &program(&reader, &format!("{BUILD_ROWS} Reader {{ src: rows }};")));
    assert!(out.contains("row-7zzzzzzzzzz"), "the copied rows are read after the handler returned: {out:?}");
}

/// The safe non-escaping use: the container lives and dies inside the
/// handler that built it, and hands nothing to a resident.
#[test]
fn a_container_that_never_leaves_its_handler_is_fine() {
    let reader = r#"locus Reader { params { label: String = ""; } run() { println(self.label); } }"#;
    let handler = r#"let rows = Rows { }; rows.push(Row { name: "local-" + to_string(x), n: x }); let g = rows.get(0) or Row { name: "?", n: 0 }; Reader { label: g.name };"#;
    let out = run_asan("resident_local_only", &program(reader, handler));
    assert!(out.contains("local-7"), "{out:?}");
}
