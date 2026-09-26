//! GH #1104 piece 5: an imported seed's `main locus` is not a second
//! main, and its bindings are inert. The cross-seed rename pass marks
//! such a locus `imported`; here the mark is set by hand on a merged
//! bundle, which is the shape `hale build` checks.

use hale_syntax::ast::TopDecl;
use hale_syntax::parse_source;
use hale_types::check_program;

fn merged(own: &str, lib: &str) -> hale_syntax::ast::Program {
    let mut prog = parse_source(own).expect("own parses");
    let mut imported = parse_source(lib).expect("lib parses");
    for item in &mut imported.items {
        if let TopDecl::Locus(l) = item {
            l.imported = true;
        }
    }
    prog.items.extend(imported.items);
    prog
}

const LIB: &str = r#"
type Msg { v: Int; }
topic LibTopic { payload: Msg; subject: "lib.msg"; }
main locus LibHead {
    bus { publish LibTopic; }
    bindings { LibTopic: unix("/tmp/lib.sock", role: listen); api: unix("/tmp/lib-api.sock", bound: 8, on_full: refuse); }
}
"#;

#[test]
fn an_imported_main_is_not_a_second_main_and_its_bindings_are_inert() {
    let own = r#"
main locus App {
    bus { subscribe LibTopic as on_msg; }
    fn on_msg(m: Msg) { }
    bindings { LibTopic: unix("/tmp/own.sock", role: connect); }
}
fn main() { App { }; }
"#;
    let msgs: Vec<String> = check_program(&merged(own, LIB)).into_iter().map(|d| d.message).collect();
    assert!(!msgs.iter().any(|m| m.contains("more than one `main` locus")), "{:?}", msgs);
    assert!(!msgs.iter().any(|m| m.contains("already bound")), "the imported main's binding of LibTopic counts toward nothing: {:?}", msgs);
}

#[test]
fn two_mains_of_the_bundles_own_still_error() {
    let own = r#"
main locus App { }
main locus Other { }
fn main() { App { }; }
"#;
    let prog = merged(own, "");
    let msgs: Vec<String> = check_program(&prog).into_iter().map(|d| d.message).collect();
    assert!(msgs.iter().any(|m| m.contains("more than one `main` locus")), "{:?}", msgs);
}
