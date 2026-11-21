//! F.61: locus modes (`mode bulk|harmonic|resolution`) are removed. A
//! mode compiled exactly like an ordinary method, so a program writes
//! the `fn` it always was.
//!
//! A program that still writes a mode gets ONE located parse error at the
//! word `mode`, naming the `fn` to write, and the member parses on as that
//! `fn`: nothing in it or after it reports again. `mode`, `bulk`,
//! `harmonic` and `resolution` are ordinary identifiers everywhere else,
//! and a locus method may now take one of the three former mode names.

use hale_syntax::ast::{LocusMember, TopDecl};
use hale_syntax::parse_source;

fn refusal(name: &str) -> String {
    format!(
        "locus modes were removed in 0.38: write `fn {name}(...)` (a mode compiled as an \
         ordinary method; spec/decisions.md F.61)"
    )
}

/// Every diagnostic a source produces, as `line:col message`.
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

#[test]
fn each_mode_is_one_located_error_at_the_word() {
    let src = "locus L {\n\
               \x20   params { n: Int = 0; }\n\
               \x20   mode bulk(scale: Int = 2) -> Int { return self.n * scale; }\n\
               \x20   mode harmonic() -> Int { return self.n; }\n\
               \x20   mode resolution() { }\n\
               \x20   run() { println(self.bulk()); }\n\
               }\n\
               fn main() { L { }; }\n";
    assert_eq!(
        diags(src),
        [format!("3:5 {}", refusal("bulk")), format!("4:5 {}", refusal("harmonic")), format!("5:5 {}", refusal("resolution"))]
    );
}

#[test]
fn a_mode_written_without_its_parameter_list_is_refused_the_same_way() {
    let src = "locus L {\n    mode bulk { println(1); }\n    mode harmonic -> Int { return 1; }\n}\nfn main() { }\n";
    assert_eq!(diags(src), [format!("2:5 {}", refusal("bulk")), format!("3:5 {}", refusal("harmonic"))]);
}

#[test]
fn a_decorated_mode_is_refused_at_the_word_too() {
    let src = "locus L {\n    @unbounded\n    mode bulk() { }\n}\nfn main() { }\n";
    assert_eq!(diags(src), [format!("3:5 {}", refusal("bulk"))]);
}

#[test]
fn what_follows_the_mode_still_parses_and_reports_its_own_errors() {
    let src = "locus L {\n    mode bulk() { }\n    fn f() { let = 1; }\n}\nfn main() { }\n";
    let ds = diags(src);
    assert_eq!(ds.len(), 2, "{ds:?}");
    assert_eq!(ds[0], format!("2:5 {}", refusal("bulk")));
    assert!(ds[1].starts_with("3:"), "the second error is the fn's own: {ds:?}");
}

#[test]
fn the_migration_parses_as_a_method() {
    let src = "locus L {\n    params { n: Int = 0; }\n    fn bulk(scale: Int = 2) -> Int { return self.n * scale; }\n    \
               @unbounded fn harmonic() { }\n    fn resolution() -> Int { return self.bulk(); }\n}\nfn main() { }\n";
    let program = parse_source(src).expect("the migration parses");
    let TopDecl::Locus(l) = &program.items[0] else { panic!("a locus") };
    let names: Vec<&str> = l
        .members
        .iter()
        .filter_map(|m| match m {
            LocusMember::Fn(f) => Some(f.name.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["bulk", "harmonic", "resolution"]);
}

#[test]
fn the_words_are_names() {
    let src = "type R { mode: Int; bulk: Int; harmonic: Int; resolution: Int; }\n\
               fn mode(bulk: Int) -> Int { let harmonic = bulk + 1; return harmonic; }\n\
               locus L { params { mode: Int = 0; resolution: Int = 1; } fn get() -> Int { return self.mode + self.resolution; } }\n\
               fn main() { let r = R { mode: 1, bulk: 2, harmonic: 3, resolution: 4 }; println(mode(r.bulk)); }\n";
    assert_eq!(diags(src), Vec::<String>::new());
}
