//! GH #1417 (R4): `bindings { api: … }` and `@gated(role: R)` are retired.
//! The parser carries no code to accept either; each is refused with ONE
//! diagnostic that names the replacement (`api NAME { rpc … }` / `@rpc`
//! with `requires`, `api::serve(…)`, a topic binding to a hub).
//! `role` declarations stay: a row's `requires` names them.

use hale_syntax::ast::TopDecl;
use hale_syntax::parse_source;

fn main_with(entries: &str) -> String {
    format!(
        "topic T {{ payload: P; }}\ntype P {{ x: Int; }}\nmain locus App {{ bindings {{ {} }} }}\nfn main() {{ App {{ }}; }}\n",
        entries
    )
}

fn roles_program(extra_top: &str, contract: &str, bus: &str, fn_deco: &str) -> String {
    format!(
        "{extra_top}type P {{ x: Int; }}\ntype L {{ n: Int; }}\ntopic T {{ payload: P; }}\ntopic S {{ payload: P; }}\n\
         locus W {{\n    contract {{ {contract} }}\n    params {{ led: L = L {{ n: 0 }}; }}\n    bus {{ subscribe T as on_t; {bus} }}\n    {fn_deco}fn on_t(p: P) {{ }}\n}}\n\
         main locus App {{ params {{ w: W = W {{ }}; }} }}\nfn main() {{ App {{ }}; }}\n"
    )
}

#[test]
fn the_api_entry_is_refused_in_every_form_with_the_replacement() {
    for entry in [
        r#"api: unix("/run/app.sock", bound: 64, on_full: refuse);"#,
        r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, watch_bound: 256, on_watch_full: drop_new);"#,
        r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, roles: self.roles, on_unauthorized: drop);"#,
        r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, serve: [w]);"#,
        r#"api: unix("/run/app.sock", bound: 64, on_full: refuse), http("0.0.0.0", 8080, principals: self.p);"#,
        r#"T: unix("/run/t.sock"); api: unix("/run/app.sock");"#,
    ] {
        let err = parse_source(&main_with(entry)).expect_err(entry);
        assert_eq!(err.len(), 1, "{entry}: one diagnostic, got {err:?}");
        let m = &err[0].message;
        assert!(m.contains("`bindings { api: … }` is retired"), "{entry}: {m}");
        for named in ["api NAME { rpc", "@rpc", "api::serve(", "unix::Rpc", "http::Rpc", "topic binding to a hub"] {
            assert!(m.contains(named), "{entry}: the diagnostic names `{named}`: {m}");
        }
    }
}

#[test]
fn a_topic_binding_beside_a_hub_binding_still_parses() {
    let src = "topic T { payload: P; }\ntype P { x: Int; }\nmain locus App { bindings { T: unix(\"/run/t.sock\"); } }\nfn main() { App { }; }\n";
    parse_source(src).expect("an ordinary topic binding is not the retired entry");
}

#[test]
fn gated_is_refused_everywhere_with_the_replacement() {
    for (src, place) in [
        (roles_program("role r;\n", "@gated(role: r) expose led: L;", "", ""), "an expose"),
        (roles_program("role r;\n", "", "@gated(role: r) publish S;", ""), "a publish"),
        (roles_program("role r;\n", "", "@gated(role: r) subscribe S as on_s;", ""), "a subscribe line"),
        (roles_program("role r;\n", "@gated(role: r) consume led: L;", "", ""), "a consume"),
        (roles_program("role r;\n", "", "", "@gated(role: r)\n    "), "a locus fn"),
        (roles_program("role r;\n", "", "", "@hot @gated(role: r)\n    "), "a stacked fn decorator"),
        (roles_program("role r;\n@gated(role: r)\nfn free() { }\n", "", "", ""), "a free fn"),
        (
            roles_program("role r;\nperspective V {\n    @gated(role: r)\n    fn f(p: P) -> Int;\n}\n", "", "", ""),
            "a perspective fn",
        ),
    ] {
        let err = parse_source(&src).expect_err(place);
        // The recovery after a refused member may add its own follow-on
        // ("expected top-level declaration"); the refusal is one, first.
        assert_eq!(err.iter().filter(|d| d.message.contains("is retired")).count(), 1, "{place}: {err:?}");
        let m = &err[0].message;
        assert!(m.contains("`@gated` is retired"), "{place}: {m}");
        assert!(m.contains("requires") && m.contains("@rpc") && m.contains("self.hub"), "{place}: {m}");
    }
}

#[test]
fn roles_are_declared_at_top_level_with_includes() {
    let src = roles_program("role support;\nrole owner includes support, auditor;\nrole auditor;\n", "", "", "");
    let prog = parse_source(&src).expect("parses");
    let roles: Vec<(String, Vec<String>)> = prog
        .items
        .iter()
        .filter_map(|i| match i {
            TopDecl::Role(r) => Some((r.name.name.clone(), r.includes.iter().map(|x| x.name.clone()).collect())),
            _ => None,
        })
        .collect();
    assert_eq!(
        roles,
        vec![
            ("support".to_string(), vec![]),
            ("owner".to_string(), vec!["support".to_string(), "auditor".to_string()]),
            ("auditor".to_string(), vec![]),
        ]
    );
    // `role` stays a contextual name elsewhere: a param named `role`.
    let ok = "main locus App { params { role: Int = 1; } }\nfn main() { App { }; }\n";
    parse_source(ok).expect("`role` as a param name still parses");
}
