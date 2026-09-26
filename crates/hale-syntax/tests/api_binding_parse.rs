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
