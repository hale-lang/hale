//! GH #1104 piece 5: the cross-seed rename pass marks the loci it
//! renamed — the ones another seed declared — `imported`, and leaves
//! the entrypoint's own alone. The api surface reads that mark: an
//! imported `main locus` is inert and an imported locus serves no
//! command (the end-to-end half is `tests/hale/api_roles_xseed_test.hl`,
//! whose library carries an api entry of its own that never binds).

use hale_codegen::mangle::apply_qualified_path_renames;
use hale_syntax::ast::TopDecl;
use hale_syntax::parse_source;

#[test]
fn the_rename_pass_marks_the_other_seeds_loci_imported() {
    // the merged bundle as the CLI hands it over: the library's
    // declarations already carry their mangled names
    let src = r#"
type Ping { n: Int; }
topic __lib_lib_roles_Pings { payload: Ping; subject: "lib.ping"; }
main locus __lib_lib_roles_LibHead {
    bus { subscribe __lib_lib_roles_Pings as on_ping; }
    fn on_ping(p: Ping) { }
    bindings { api: unix("/tmp/lib.sock", bound: 8, on_full: refuse); }
}
locus __lib_lib_roles_TableRoles {
    fn holds(p: std::api::Principal, r: String) -> Bool { return false; }
}
main locus App {
    params { roles: __lib_lib_roles_TableRoles = __lib_lib_roles_TableRoles { }; }
    bindings { api: unix("/tmp/app.sock", bound: 8, on_full: refuse, roles: self.roles); }
}
fn main() { App { }; }
"#;
    let mut prog = parse_source(src).expect("the bundle parses");
    let renames = vec![
        (vec!["lib".to_string(), "Pings".to_string()], "__lib_lib_roles_Pings".to_string()),
        (vec!["lib".to_string(), "LibHead".to_string()], "__lib_lib_roles_LibHead".to_string()),
        (vec!["lib".to_string(), "TableRoles".to_string()], "__lib_lib_roles_TableRoles".to_string()),
    ];
    apply_qualified_path_renames(&mut prog, &renames);
    let mut marks = std::collections::BTreeMap::new();
    for item in &prog.items {
        if let TopDecl::Locus(l) = item {
            marks.insert(l.name.name.clone(), l.imported);
        }
    }
    assert_eq!(marks.get("__lib_lib_roles_LibHead"), Some(&true), "the library's main is marked imported: {marks:?}");
    assert_eq!(marks.get("__lib_lib_roles_TableRoles"), Some(&true), "and so is its source: {marks:?}");
    assert_eq!(marks.get("App"), Some(&false), "the entrypoint's own main is not: {marks:?}");
    let topic_display = prog.items.iter().find_map(|i| match i {
        TopDecl::Topic(t) if t.name.name == "__lib_lib_roles_Pings" => t.display.clone(),
        _ => None,
    });
    assert_eq!(topic_display.as_deref(), Some("lib::Pings"), "the topic keeps the author's spelling for the description");
}

#[test]
fn nothing_renamed_marks_nothing() {
    let mut prog = parse_source("main locus App { }\nfn main() { App { }; }\n").expect("parses");
    apply_qualified_path_renames(&mut prog, &[]);
    let own = prog.items.iter().any(|i| matches!(i, TopDecl::Locus(l) if l.name.name == "App" && !l.imported));
    assert!(own, "an entrypoint with no imports has no imported locus");
}
