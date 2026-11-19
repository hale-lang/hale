//! F.57: the `module NAME { … }` declaration is removed. A module is a
//! directory, reached by `import`, and `module` is an ordinary
//! identifier.
//!
//! A program that still writes the old form at top-level declaration
//! position gets ONE located parse error at the word `module`, and the
//! braced body is skipped whole: nothing inside it reports again, and a
//! declaration after it parses as usual. Everywhere else `module` is a
//! name a program may use: a local, a field, a parameter, a fn.

use hale_syntax::ast::{LocusMember, TopDecl};
use hale_syntax::parse_source;

const REFUSAL: &str = "`module` declarations were removed: a module is a directory, reached by \
     `import \"path\" as name;`. Move the declarations inside `module NAME { }` to the top \
     level, or into their own file or directory and import it (spec/decisions.md F.57)";

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
fn a_module_declaration_is_one_located_error_at_the_word() {
    let src = "type Point { x: Int; }\n\
               \n\
               module geo {\n\
               \x20   type Meters = Int;\n\
               \x20   fn area(p: Point) -> Int { return p.x * p.x; }\n\
               \x20   locus Survey { params { n: Int = 0; } }\n\
               }\n\
               \n\
               fn main() { }\n";
    assert_eq!(diags(src), [format!("3:1 {REFUSAL}")]);
}

#[test]
fn the_body_is_skipped_whole_so_nothing_inside_reports_again() {
    // Nested braces, a nested `module`, a `target` and a body that would
    // not parse as declarations: still the one error, at the outer word.
    let src = "module outer {\n\
               \x20   module inner {\n\
               \x20       target wasm { }\n\
               \x20       fn f() { if true { let x = { 1 }; } }\n\
               \x20   }\n\
               \x20   this is not a declaration at all\n\
               }\n\
               fn main() { }\n";
    assert_eq!(diags(src), [format!("1:1 {REFUSAL}")]);
}

#[test]
fn what_follows_the_module_still_parses_and_reports_its_own_errors() {
    // The declaration after the skipped body is parsed: its own error is
    // reported, at its own place, beside the refusal.
    let src = "module m { fn a() { } }\nfn main() { let = 1; }\n";
    let ds = diags(src);
    assert_eq!(ds.len(), 2, "{ds:?}");
    assert_eq!(ds[0], format!("1:1 {REFUSAL}"));
    assert!(ds[1].starts_with("2:"), "the second error is the fn's own: {ds:?}");
}

#[test]
fn recovery_from_an_earlier_error_stops_at_the_module_word() {
    // A broken declaration before the old form: its own error, then the
    // refusal at the word, and nothing from inside the braces.
    let src = "fn broken( { }\nmodule m {\n    fn f() { let = ; }\n}\nfn main() { }\n";
    let ds = diags(src);
    assert_eq!(ds.len(), 2, "{ds:?}");
    assert!(ds[0].starts_with("1:"), "the broken fn's own error first: {ds:?}");
    assert_eq!(ds[1], format!("2:1 {REFUSAL}"));
}

#[test]
fn two_module_declarations_are_two_errors() {
    let src = "module a { }\nmodule b { fn f() { } }\nfn main() { }\n";
    assert_eq!(diags(src), [format!("1:1 {REFUSAL}"), format!("2:1 {REFUSAL}")]);
}

#[test]
fn an_unclosed_module_is_still_one_error() {
    let src = "module m {\n    fn f() { }\n";
    assert_eq!(diags(src), [format!("1:1 {REFUSAL}")]);
}

#[test]
fn module_is_an_ordinary_name() {
    let src = "type Package { module: String; }\n\
               fn module(x: Int) -> Int { return x + 1; }\n\
               fn uses(module: Int) -> Int { return module; }\n\
               locus Loader {\n\
               \x20   params { module: String = \"core\"; }\n\
               \x20   fn module_name() -> String { return self.module; }\n\
               }\n\
               fn main() {\n\
               \x20   let module = module(1);\n\
               \x20   let p = Package { module: \"m\" };\n\
               \x20   println(p.module);\n\
               \x20   println(uses(module));\n\
               }\n";
    let program = parse_source(src).unwrap_or_else(|ds| panic!("parses: {:?}", diags_of(src, &ds)));
    let fns: Vec<&str> = program
        .items
        .iter()
        .filter_map(|i| match i {
            TopDecl::Fn(f) => Some(f.name.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(fns, ["module", "uses", "main"]);
    let loader = program
        .items
        .iter()
        .find_map(|i| match i {
            TopDecl::Locus(l) => Some(l),
            _ => None,
        })
        .expect("the locus");
    let field = loader
        .members
        .iter()
        .find_map(|m| match m {
            LocusMember::Params(pb) => pb.params.first().map(|p| p.name.name.as_str()),
            _ => None,
        })
        .expect("a params field");
    assert_eq!(field, "module");
}

#[test]
fn module_as_a_local_is_assigned_and_read() {
    // Only `module NAME {` at declaration position is the old form: a
    // statement that starts with the identifier `module` is an ordinary
    // statement.
    let src = "fn main() { let module = 1; module = module + 1; }\n";
    assert_eq!(diags(src), Vec::<String>::new());
}

fn diags_of(src: &str, ds: &[hale_syntax::Diag]) -> Vec<String> {
    ds.iter()
        .map(|d| {
            let (line, col) = d.span.line_col(src);
            format!("{}:{} {}", line, col, d.message)
        })
        .collect()
}
