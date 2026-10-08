//! GH #1417 (R1): the surface's spellings parse (spec/api.md § Surfaces
//! and their rows, § Serving, § Streams): an `api` block's `rpc` rows,
//! `@rpc` on a locus fn, a serve site's named arguments and a topic bound
//! to a hub; and each malformed spelling is refused where it goes wrong.

use hale_syntax::ast::{Expr, LocusMember, ServeSite, Stmt, TopDecl};
use hale_syntax::parse_source;

fn first_error(src: &str) -> String {
    match parse_source(src) {
        Ok(_) => panic!("expected a parse error for:\n{src}"),
        Err(diags) => diags[0].message.clone(),
    }
}

#[test]
fn an_api_block_is_a_surface_of_rows() {
    let p = parse_source(
        "api Public {
            rpc Orders::place;
            rpc Orders::cancel requires: [trader];
            rpc lib::Ledger::rebalance requires: [operator, auditor];
            rpc Orders::flush requires: [];
        }",
    )
    .expect("parse");
    let TopDecl::Api(a) = &p.items[0] else { panic!("an api decl") };
    assert_eq!(a.name.name, "Public");
    let rows: Vec<(String, String, String, Vec<String>)> = a
        .rows
        .iter()
        .map(|r| {
            (r.locus.name.clone(), r.written.clone(), r.method.name.clone(), r.requires.iter().map(|i| i.name.clone()).collect())
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Orders".into(), "Orders".into(), "place".into(), vec![]),
            ("Orders".into(), "Orders".into(), "cancel".into(), vec!["trader".into()]),
            ("lib::Ledger".into(), "lib::Ledger".into(), "rebalance".into(), vec!["operator".into(), "auditor".into()]),
            ("Orders".into(), "Orders".into(), "flush".into(), vec![]),
        ]
    );
}

#[test]
fn rpc_on_a_locus_fn_is_a_row_attribute() {
    let p = parse_source(
        "locus Orders {
            @rpc
            fn place(o: Int) -> Int { return o; }
            @rpc(requires: [trader, operator])
            fn cancel(o: Int) -> Int { return o; }
            fn internal() -> Int { return 1; }
        }",
    )
    .expect("parse");
    let TopDecl::Locus(l) = &p.items[0] else { panic!("a locus") };
    let rpcs: Vec<(String, Option<Vec<String>>)> = l
        .members
        .iter()
        .filter_map(|m| match m {
            LocusMember::Fn(f) => {
                Some((f.name.name.clone(), f.rpc.as_ref().map(|r| r.requires.iter().map(|i| i.name.clone()).collect())))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        rpcs,
        vec![
            ("place".into(), Some(vec![])),
            ("cancel".into(), Some(vec!["trader".into(), "operator".into()])),
            ("internal".into(), None),
        ]
    );
}

/// `@rpc` stacks with the other contract decorators in any order.
#[test]
fn rpc_stacks_with_the_contract_decorators() {
    let p = parse_source("locus L { @hot @rpc @no_block fn f(x: Int) -> Int { return x; } }").expect("parse");
    let TopDecl::Locus(l) = &p.items[0] else { panic!("a locus") };
    let LocusMember::Fn(f) = &l.members[0] else { panic!("a fn") };
    assert!(f.rpc.is_some() && f.hot);
}

/// `api` stays an identifier everywhere but the declaration's head: a
/// local, a fn.
#[test]
fn api_is_contextual() {
    parse_source("fn api() -> Int { let api = 1; return api; }").expect("an identifier named api");
}

#[test]
fn a_serve_site_carries_its_named_arguments() {
    let p = parse_source(
        "main locus Desk {
            run() {
                let h = api::serve(Public, http::Rpc { bind: \"127.0.0.1:8080\", codec: json }, as: \"public\", receivers: { Orders: self.orders, lib::Ledger: self.ledger }, bound: 64, on_full: refuse);
                h.stop();
            }
        }",
    )
    .expect("parse");
    let TopDecl::Locus(l) = &p.items[0] else { panic!("a locus") };
    let LocusMember::Lifecycle(run) = &l.members[0] else { panic!("run") };
    let Stmt::Let { value, .. } = &run.body.stmts[0] else { panic!("a let") };
    let site = ServeSite::of(value).expect("a serve site");
    assert!(matches!(site.surface, Expr::Ident(i) if i.name == "Public"));
    assert!(matches!(site.transport, Some(Expr::Struct { .. })));
    assert!(matches!(site.option("as"), Some(Expr::Literal(hale_syntax::ast::Literal::String(s), _)) if s == "public"));
    assert!(matches!(site.option("bound"), Some(Expr::Literal(hale_syntax::ast::Literal::Int(64), _))));
    assert!(matches!(site.option("on_full"), Some(Expr::Ident(i)) if i.name == "refuse"));
    let receivers: Vec<&str> = site.receivers().iter().map(|r| r.name.name.as_str()).collect();
    assert_eq!(receivers, vec!["Orders", "lib::Ledger"]);
    // Any other call keeps its positional arguments and takes no names.
    let ordinary = parse_source("fn main() { std::time::sleep(1ms); }").expect("parse");
    let TopDecl::Fn(f) = &ordinary.items[0] else { panic!("a fn") };
    let Stmt::Expr(e) = &f.body.stmts[0] else { panic!("a call") };
    assert!(ServeSite::of(e).is_none());
}

#[test]
fn a_topic_bound_to_a_hub_is_a_stream_row() {
    let p = parse_source(
        "main locus Desk {
            bindings {
                Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;
                Prices: self.hub;
            }
        }",
    )
    .expect("parse");
    let TopDecl::Locus(l) = &p.items[0] else { panic!("a locus") };
    let LocusMember::Bindings(bb) = &l.members[0] else { panic!("bindings") };
    assert!(bb.entries.is_empty(), "a hub binding is no transport entry");
    let rows: Vec<(String, String, Vec<String>, Option<u64>, Option<String>)> = bb
        .hubs
        .iter()
        .map(|h| {
            (
                h.topic.name.clone(),
                h.instance.name.clone(),
                h.requires.iter().map(|i| i.name.clone()).collect(),
                h.bound.map(|(n, _)| n),
                h.on_full.as_ref().map(|i| i.name.clone()),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Fills".into(), "hub".into(), vec!["operator".into()], Some(64), Some("drop_old".into())),
            ("Prices".into(), "hub".into(), vec![], None, None),
        ]
    );
}

#[test]
fn malformed_surfaces_are_refused_where_they_go_wrong() {
    assert_eq!(
        first_error("api P { rpc place; }"),
        "`rpc place` names no handler: a row names a locus fn as `Locus::fn`"
    );
    assert!(first_error("api P { fn place(); }").starts_with("an `api` block holds `rpc Locus::fn;` rows"));
    assert_eq!(
        first_error("api P { rpc Orders::place requires: trader; }"),
        "`requires:` takes its roles in brackets: `requires: [trader]`"
    );
    assert!(first_error("api P { rpc Orders::place }").contains("expected"), "a row ends with `;`");
    assert_eq!(
        first_error("locus L { @rpc(role: trader) fn f() { } }"),
        "`@rpc` takes `requires: [<role>, …]`, or nothing: `@rpc`"
    );
    assert_eq!(first_error("locus L { @rpc @rpc fn f() { } }"), "a fn takes one `@rpc`");
    assert_eq!(
        first_error("fn main() { api::serve(P, T, at: \"x\"); }"),
        "`api::serve` takes `as:`, `receivers:`, `bound:` and `on_full:`, got `at:`"
    );
    assert_eq!(first_error("fn main() { api::serve(P, T, bound: 1, bound: 2); }"), "`bound:` is written twice");
    assert_eq!(
        first_error("fn main() { api::serve(P, as: \"x\", T); }"),
        "`api::serve`'s named arguments follow its surface and its transport"
    );
    assert_eq!(
        first_error("main locus M { bindings { Fills: self.hub codec: json; } }"),
        "a hub binding takes `requires:`, `bound:` and `on_full:`, got `codec`"
    );
    assert_eq!(
        first_error("main locus M { bindings { Fills: self.hub bound: 1, bound: 2; } }"),
        "`bound:` is written twice"
    );
}
