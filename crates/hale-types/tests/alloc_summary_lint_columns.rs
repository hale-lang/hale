//! What the hot-path lint reads, as columns of the allocation summary's
//! rows (F.40 phase 3, E3a part C, 2 of 3): a fn's `@hot` and whether
//! it is a mode, the order of the declarations, where a site or a call
//! is written (a `return` payload resets `loop_depth`, not `in_loop`),
//! the struct literal written as a whole statement or as the whole
//! right side of a `self.<field> =` replace (the in-place one, which
//! allocates nothing, included), the `let` a call is the value of, how
//! a call is spelled, and the allocating receives (the one list, which
//! `@budget` reads too).

use std::collections::BTreeMap;

use hale_types::alloc_summary::{AllocKind, AllocSite, CallEdge, CallSpelling, EntryKind, FnKey, FnSummary};

const SRC: &str = r#"
locus Child {
    params { n: Int = 0; }
    fn get() -> Int { return self.n; }
}
type P { x: Int = 0; }
fn make(n: Int) -> Child { return Child { n: n }; }
@hot
fn pump(fd: Int, b: std::bytes::BytesBuilder) {
    let mut i = 0;
    while i < 3 {
        let c = make(i);
        let got = std::io::tcp::recv(fd, 8) or raise;
        let s = b.snapshot();
        Child { n: i };
        i = i + 1;
    }
}
fn pick(k: Int) -> Int {
    let mut i = 0;
    while i < 3 { return Child { n: k }.get(); }
    return 0;
}
locus Sink {
    params { last: P = P { }; }
    bus { subscribe "t" as on_t of type Int; }
    fn on_t(v: Int) { self.last = P { x: 1 }; }
    mode bulk() -> Int { return 1; }
}
main locus App {
    params { s: Sink = Sink { }; }
    run() { println(pick(1)); }
}
fn main() { App { }; }
"#;

fn rows() -> BTreeMap<FnKey, FnSummary> {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    summary.fns.into_iter().filter(|(k, _)| !summary.analysis_copy.contains(k)).collect()
}

fn text(span: hale_syntax::Span) -> &'static str {
    &SRC[span.start.as_usize()..span.end.as_usize()]
}

fn call<'a>(f: &'a FnSummary, spelling: CallSpelling) -> &'a CallEdge {
    f.calls.iter().find(|c| c.spelling == spelling).unwrap_or_else(|| panic!("{:?}", spelling))
}

fn literal<'a>(sites: &'a [AllocSite], name: &str) -> &'a AllocSite {
    sites.iter().find(|s| s.kind == AllocKind::StructLit(name.to_string())).unwrap_or_else(|| panic!("{name}"))
}

/// The rows in the order the declarations are written, with `@hot` and
/// the mode marked.
#[test]
fn decl_order_hot_and_mode() {
    let rows = rows();
    let mut order: Vec<(&FnSummary, String)> = rows.values().map(|f| (f, f.key.display())).collect();
    order.sort_by_key(|(f, _)| f.decl_index);
    let order: Vec<(String, bool, bool)> = order.into_iter().map(|(f, k)| (k, f.hot, f.mode)).collect();
    assert_eq!(
        order,
        [
            ("Child::get".to_string(), false, false),
            ("make".to_string(), false, false),
            ("pump".to_string(), true, false),
            ("pick".to_string(), false, false),
            ("Sink::on_t".to_string(), false, false),
            ("Sink::bulk".to_string(), false, true),
            ("App::run".to_string(), false, false),
            ("main".to_string(), false, false),
        ]
    );
}

/// A loop body's calls: the `let`'s factory call, the allocating
/// receive in its path spelling, the method call; and the
/// bare-statement literal.
#[test]
fn calls_and_sites_where_written() {
    let rows = rows();
    let pump = &rows[&FnKey::free_fn("pump")];
    let make = call(pump, CallSpelling::Ident("make".to_string()));
    assert!(make.in_loop);
    assert_eq!(make.let_span.map(text), Some("let c = make(i);"));
    assert_eq!(make.allocating_recv, None);
    let recv = call(pump, CallSpelling::Path("std::io::tcp::recv".to_string()));
    assert_eq!(recv.allocating_recv.as_deref(), Some("std::io::tcp::recv"));
    assert!(recv.in_loop && recv.let_span.is_none(), "the `let`'s value is the `or`, not the call");
    let snap = call(pump, CallSpelling::Method("snapshot".to_string()));
    assert!(snap.in_loop && snap.let_span.is_some() && snap.allocating_recv.is_none());
    let child = literal(&pump.sites, "Child");
    assert!(child.bare_stmt && child.in_loop && child.self_replace.is_none());
    // A `return` payload allocates once per call, and is written in a
    // loop.
    let pick = &rows[&FnKey::free_fn("pick")];
    let child = literal(&pick.sites, "Child");
    assert_eq!((child.loop_depth, child.in_loop, child.bare_stmt), (0, true, false));
}

/// A handler's in-place `self.<field>` replace: no allocation, so not a
/// site, and kept with its statement.
#[test]
fn in_place_self_replace() {
    let rows = rows();
    let on_t = &rows[&FnKey::method("Sink", "on_t")];
    assert_eq!(on_t.entry, Some(EntryKind::BusHandler));
    assert!(on_t.sites.is_empty());
    let p = literal(&on_t.in_place_sites, "P");
    assert_eq!(p.self_replace.map(text), Some("self.last = P { x: 1 };"));
    assert!(!p.in_loop && !p.bare_stmt);
}

/// The allocating receives are one list, by spelling: the path form,
/// and the two method names; a bare call by those names is not one.
#[test]
fn allocating_receives() {
    for (spelling, recv) in [
        (CallSpelling::Path("std::io::udp::recv_with_source".into()), Some("std::io::udp::recv_with_source")),
        (CallSpelling::Path("std::io::tcp::recv_into".into()), None),
        (CallSpelling::Method("recv_bytes".into()), Some("recv_bytes")),
        (CallSpelling::Method("recv".into()), None),
        (CallSpelling::Ident("recv_bytes".into()), None),
        (CallSpelling::PathMethod("recv_bytes".into()), None),
    ] {
        assert_eq!(spelling.allocating_recv().as_deref(), recv, "{:?}", spelling);
    }
}
