//! GH #1106: the `api:` entry of a `bindings { }` block.

use hale_syntax::ast::{ApiFullPolicy, ApiTransport, LocusMember, ShedPolicy, TopDecl};
use hale_syntax::parse_source;

fn main_with(entries: &str) -> String {
    format!(
        "topic T {{ payload: P; }}\ntype P {{ x: Int; }}\nmain locus App {{ bindings {{ {} }} }}\nfn main() {{ App {{ }}; }}\n",
        entries
    )
}

fn api_of(src: &str) -> hale_syntax::ast::ApiBinding {
    let prog = parse_source(src).expect("parses");
    for item in &prog.items {
        if let TopDecl::Locus(l) = item {
            for m in &l.members {
                if let LocusMember::Bindings(bb) = m {
                    return bb.api.clone().expect("an api entry");
                }
            }
        }
    }
    panic!("no bindings block");
}

#[test]
fn the_entry_parses_with_every_knob() {
    let api = api_of(&main_with(
        r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, watch_bound: 256, on_watch_full: drop_new);"#,
    ));
    let ApiTransport::Unix { path, .. } = &api.transport;
    assert_eq!(path, "/run/app.sock");
    assert_eq!(api.bound.map(|b| b.0), Some(64));
    assert_eq!(api.on_full.map(|p| p.0), Some(ApiFullPolicy::Refuse));
    assert_eq!(api.watch_bound.map(|b| b.0), Some(256));
    assert_eq!(api.on_watch_full.map(|p| p.0), Some(ShedPolicy::DropNew));
}

#[test]
fn the_entry_sits_beside_topic_entries() {
    let src = main_with(r#"T: unix("/tmp/t.sock", role: listen); api: unix("/run/app.sock", bound: 8, on_full: refuse);"#);
    let prog = parse_source(&src).expect("parses");
    let TopDecl::Locus(l) = &prog.items[2] else { panic!() };
    let LocusMember::Bindings(bb) = &l.members[0] else { panic!() };
    assert_eq!(bb.entries.len(), 1, "the api entry is not a topic entry");
    assert!(bb.api.is_some());
}

#[test]
fn a_second_api_entry_is_refused() {
    let src = main_with(r#"api: unix("/a.sock", bound: 8, on_full: refuse); api: unix("/b.sock", bound: 8, on_full: refuse);"#);
    let err = parse_source(&src).expect_err("two entries");
    assert!(err.iter().any(|d| d.message.contains("one `api:` entry")), "{:?}", err);
}

#[test]
fn unknown_knobs_and_policies_are_refused() {
    for (entry, needle) in [
        (r#"api: unix("/a.sock", bounds: 8);"#, "unknown api kwarg `bounds`"),
        (r#"api: unix("/a.sock", bound: 0);"#, "positive integer"),
        (r#"api: unix("/a.sock", on_full: drop);"#, "`on_full:` policy is `refuse`"),
        (r#"api: unix("/a.sock", on_watch_full: refuse);"#, "`drop_old` or `drop_new`"),
        (r#"api: http("127.0.0.1:8080");"#, "expected api transport `unix`"),
    ] {
        let err = parse_source(&main_with(entry)).expect_err(entry);
        assert!(err.iter().any(|d| d.message.contains(needle)), "{}: {:?}", entry, err);
    }
}

#[test]
fn fmt_keeps_the_entry() {
    let src = "main locus App {\n    bindings {\n        api: unix(\"/run/app.sock\", bound: 64, on_full: refuse);\n    }\n}\n\nfn main() {\n    App { };\n}\n";
    let out = hale_syntax::fmt::format_source(src).expect("formats");
    assert_eq!(out, src);
}

#[test]
fn the_flag_injects_the_dev_entry_and_needs_a_main_locus() {
    let mut prog = parse_source("main locus App { }\nfn main() { App { }; }\n").expect("parses");
    hale_syntax::api_gen::inject_api_entry(&mut prog, "unix:/run/app.sock").expect("injects");
    let api = {
        let TopDecl::Locus(l) = &prog.items[0] else { panic!() };
        let Some(LocusMember::Bindings(bb)) = l.members.iter().find(|m| matches!(m, LocusMember::Bindings(_))) else { panic!("no bindings block") };
        bb.api.clone().expect("api entry")
    };
    let ApiTransport::Unix { path, .. } = &api.transport;
    assert_eq!(path, "/run/app.sock");
    assert_eq!(api.bound.map(|b| b.0), Some(hale_syntax::api_gen::DEV_BOUND));
    assert!(api.on_full.is_some());

    let mut bare = parse_source("fn main() { println(\"hi\"); }\n").expect("parses");
    let err = hale_syntax::api_gen::inject_api_entry(&mut bare, "/run/app.sock").expect_err("no main locus");
    assert!(err.contains("`main locus`"), "{}", err);
}

// ---- GH #1109: roles, @gated, and the gate's knobs ------------------------

fn roles_program(extra_top: &str, contract: &str, bus: &str, fn_deco: &str, entry: &str) -> String {
    format!(
        "{extra_top}type P {{ x: Int; }}\ntype L {{ n: Int; }}\ntopic T {{ payload: P; }}\ntopic S {{ payload: P; }}\n\
         locus W {{\n    contract {{ {contract} }}\n    params {{ led: L = L {{ n: 0 }}; }}\n    bus {{ subscribe T as on_t; {bus} }}\n    {fn_deco}fn on_t(p: P) {{ }}\n}}\n\
         main locus App {{ params {{ w: W = W {{ }}; }} bindings {{ {entry} }} }}\nfn main() {{ App {{ }}; }}\n"
    )
}

#[test]
fn roles_are_declared_at_top_level_with_includes() {
    let src = roles_program("role support;\nrole owner includes support, auditor;\nrole auditor;\n", "", "", "", "");
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

#[test]
fn gated_goes_on_a_fn_an_expose_and_a_publish() {
    let src = roles_program(
        "role r;\n",
        "@gated(role: r) expose led: L;",
        "@gated(role: r) publish S;",
        "@gated(role: r)\n    ",
        r#"api: unix("/tmp/t.sock", bound: 8, on_full: refuse);"#,
    );
    let prog = parse_source(&src).expect("parses");
    let TopDecl::Locus(w) = prog.items.iter().find(|i| matches!(i, TopDecl::Locus(l) if l.name.name == "W")).unwrap() else { panic!() };
    let mut seen = 0;
    for m in &w.members {
        match m {
            LocusMember::Fn(f) if f.name.name == "on_t" => {
                assert_eq!(f.gated.as_ref().map(|g| g.name.as_str()), Some("r"));
                assert!(f.decorators.iter().any(|d| d.name == "gated"), "recorded as written: {:?}", f.decorators);
                seen += 1;
            }
            LocusMember::Contract(cb) => {
                let hale_syntax::ast::ContractKind::Members(ms) = &cb.kind else { panic!() };
                assert_eq!(ms[0].gated.as_ref().map(|g| g.name.as_str()), Some("r"));
                seen += 1;
            }
            LocusMember::Bus(bb) => {
                for bm in &bb.members {
                    if let hale_syntax::ast::BusMember::Publish { gated, .. } = bm {
                        assert_eq!(gated.as_ref().map(|g| g.name.as_str()), Some("r"));
                        seen += 1;
                    }
                }
            }
            _ => {}
        }
    }
    assert_eq!(seen, 3);
    let out = hale_syntax::fmt::format_source(&src).expect("formats");
    assert!(out.contains("@gated(role: r) expose led: L;") && out.contains("@gated(role: r) publish S;") && out.contains("role owner") || out.contains("@gated(role: r)"), "fmt keeps the annotations: {}", out);
}

#[test]
fn gated_is_refused_where_it_means_nothing() {
    for (src, needle) in [
        (roles_program("role r;\n", "@gated(role: r) consume led: L;", "", "", ""), "goes on an `expose` member"),
        (roles_program("role r;\n", "", "@gated(role: r) subscribe S as on_s;", "", ""), "not on the `subscribe` line"),
        (roles_program("role r;\n", "", "", "@gated(admin: r)\n    ", ""), "takes `role: <name>`"),
        (roles_program("role r;\n", "", "", "@gated\n    ", ""), "expected"),
    ] {
        let err = parse_source(&src).expect_err(needle);
        assert!(err.iter().any(|d| d.message.contains(needle)), "{}: {:?}", needle, err);
    }
}

#[test]
fn the_entry_takes_a_refusal_policy_and_a_role_source() {
    let api = api_of(&main_with(
        r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, on_unauthorized: drop, roles: Record { store: 1 });"#,
    ));
    assert_eq!(api.on_unauthorized.map(|p| p.0), Some(hale_syntax::ast::ApiUnauthorizedPolicy::Drop));
    let r = api.roles.expect("a role source");
    let hale_syntax::ast::Expr::Struct { path, inits, .. } = &r.expr else { panic!("a locus literal: {:?}", r.expr) };
    assert_eq!(path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Record"]);
    assert_eq!(inits.len(), 1);
    let api = api_of(&main_with(r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, on_unauthorized: refuse);"#));
    assert_eq!(api.on_unauthorized.map(|p| p.0), Some(hale_syntax::ast::ApiUnauthorizedPolicy::Refuse));
    assert!(api.roles.is_none());
    let err = parse_source(&main_with(r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, on_unauthorized: ignore);"#)).expect_err("bad policy");
    assert!(err.iter().any(|d| d.message.contains("unknown on_unauthorized policy")), "{:?}", err);
    // A cross-seed source keeps its qualified path; a main param is an
    // expression on `self` (review F6).
    let api = api_of(&main_with(r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, roles: dna::RecordRoles { });"#));
    let hale_syntax::ast::Expr::Struct { path, .. } = &api.roles.unwrap().expr else { panic!() };
    assert_eq!(path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["dna", "RecordRoles"]);
    let api = api_of(&main_with(r#"api: unix("/run/app.sock", bound: 64, on_full: refuse, roles: self.roles);"#));
    let hale_syntax::ast::Expr::Field { receiver, name, .. } = &api.roles.unwrap().expr else { panic!() };
    assert!(matches!(**receiver, hale_syntax::ast::Expr::KwSelf(_)));
    assert_eq!(name.name, "roles");
}
