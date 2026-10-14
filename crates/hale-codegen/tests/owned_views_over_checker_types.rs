//! The owned-view column reads the checker's types and callees (GH #1554).
//!
//! The pass that decides which bindings take an owned copy used to resolve
//! types and callees itself, by spelling: a method by its name over every
//! locus declaring one, a fn by every minted name sharing its suffix, a
//! field by every declaration of that field name. Each shape below made
//! it copy a binding no write can reach; the column now reads what the
//! checker resolved, so each binding stays the field's own pointer. Every
//! program is also built under AddressSanitizer and run, so a copy the
//! column gave up that a write needed would read freed memory or a
//! replaced value. The controls keep the copies a write does need, and a
//! call through a fn value, whose callee no checker can name, still costs
//! one.

use std::collections::BTreeSet;
use std::process::Command;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::Disk;
use hale_syntax::ast::{Block, Stmt, TopDecl, LocusMember};
use hale_types::view_bindings::OwnedViews;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Every `let`, bare `=` and `for` of `b` the column owns, as
/// `<fn>.<name>` (`fn_name` the enclosing fn or method).
fn owned_in(b: &Block, fn_name: &str, views: &OwnedViews, out: &mut BTreeSet<String>) {
    for s in &b.stmts {
        let (id, name) = match s {
            Stmt::Let { id, name, .. } | Stmt::For { id, name, .. } => (*id, name.name.clone()),
            Stmt::Assign { id, target, .. } if target.tail.is_empty() => (*id, target.head.name.clone()),
            _ => (hale_syntax::ast::NodeId::NONE, String::new()),
        };
        if views.node(id) {
            out.insert(format!("{fn_name}.{name}"));
        }
        match s {
            Stmt::For { body, .. } | Stmt::While { body, .. } | Stmt::Block(body) => owned_in(body, fn_name, views, out),
            Stmt::If(i) => owned_in(&i.then_block, fn_name, views, out),
            _ => {}
        }
    }
}

/// The owned bindings of every fn and locus method `items` declares.
fn owned(items: &[TopDecl], views: &OwnedViews) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for item in hale_syntax::ast::flat_decls(items) {
        match item {
            TopDecl::Fn(f) => owned_in(&f.body, &f.name.name, views, &mut out),
            TopDecl::Locus(l) => {
                for m in &l.members {
                    if let LocusMember::Fn(f) = m {
                        owned_in(&f.body, &f.name.name, views, &mut out);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The owned bindings of `src`'s program, as its harness snapshot's
/// column says.
fn owned_of(src: &str) -> BTreeSet<String> {
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let views = snap.demand_owned_views().expect("the column");
    snap.programs().values().flat_map(|p| owned(&p.items, &views.program)).collect()
}

/// `src` built under ASan and run (the chunk pool off, so a retired block
/// is freed): its stdout.
fn run_asan(tag: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("owned_views_checker_{tag}"));
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{tag}: {:?}; stdout: {stdout}; stderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    stdout
}

/// A method called on a field of another locus's type is that locus's:
/// `self.o.put(..)` runs `Other.put`, which writes `Other`'s field, so a
/// String read from `self.f` stays a view across it. By name, `put` was
/// every locus's `put`, `H.put` (which replaces `self.f`) included, and
/// the binding was copied. The control calls `H.put` itself.
#[test]
fn a_method_is_the_receiver_types() {
    let src = r#"
        type F { s: String = ""; }
        locus Other {
            params { g: F = F { }; }
            fn put(t: String) { self.g = F { s: t }; }
        }
        locus H {
            params { f: F = F { }; o: Other = Other { }; }
            fn put(t: String) { self.f = F { s: t }; }
            fn held() -> String {
                let s = self.f.s;
                self.o.put(std::str::upper("other"));
                return s;
            }
            fn replaced() -> String {
                let s = self.f.s;
                self.put(std::str::upper("new"));
                return s;
            }
        }
        fn main() {
            let h = H { };
            h.put(std::str::upper("kept"));
            println("held=" + h.held());
            println("replaced=" + h.replaced());
        }
    "#;
    assert_eq!(owned_of(src), BTreeSet::from(["replaced.s".to_string()]));
    let out = run_asan("receiver", src);
    assert!(out.contains("held=KEPT\n") && out.contains("replaced=KEPT\n"), "{out}");
}

/// A method named like a fn-typed field elsewhere: `self.sc.cb(3)` is
/// `Scorer.cb`, a method, though `Hooked` declares a field `cb` of a
/// fn-typed alias (`type Callback = fn(Int) -> Int`). By name, any
/// member `cb` of any receiver was a call through a fn value, which may
/// write anything, and the binding was copied. A call through the field
/// itself is still a fn value no checker can name, and keeps its copy.
#[test]
fn a_fn_typed_alias_is_a_value_only_where_it_is_one() {
    let src = r#"
        type F { s: String = ""; }
        type Callback = fn(Int) -> Int;
        fn twice(n: Int) -> Int { return n * 2; }
        locus Scorer {
            params { total: Int = 0; }
            fn cb(n: Int) -> Int { self.total = self.total + n; return self.total; }
        }
        locus H {
            params { f: F = F { }; sc: Scorer = Scorer { }; cb: Callback = twice; }
            fn put(t: String) { self.f = F { s: t }; }
            fn held() -> String {
                let s = self.f.s;
                let n = self.sc.cb(3);
                return s + to_string(n);
            }
            fn through_value() -> String {
                let s = self.f.s;
                let n = self.cb(2);
                return s + to_string(n);
            }
        }
        fn main() {
            let h = H { };
            h.put(std::str::upper("kept"));
            println("held=" + h.held());
            println("value=" + h.through_value());
        }
    "#;
    assert_eq!(owned_of(src), BTreeSet::from(["through_value.s".to_string()]));
    let out = run_asan("fn_alias", src);
    assert!(out.contains("held=KEPT3\n") && out.contains("value=KEPT4\n"), "{out}");
}

/// A write into a part laid out inline is the local's own: `e.slots` is
/// `[Int; 4]` in `Rec`, so the get-modify-set of a map's cell copies
/// nothing. By name, `slots` was every declaration of the field, `Other`'s
/// `[Inner; 2]` (a block of its own) among them, and the write copied the
/// part on write, an allocation the survey reported.
#[test]
fn a_field_is_its_declarations_not_every_field_of_the_name() {
    let src = r#"
        type Inner { k: Int = 0; s: String = ""; }
        type Rec { id: Int = 0; slots: [Int; 4] = [0, 0, 0, 0]; }
        type Other { slots: [Inner; 2] = [Inner { }, Inner { }]; }
        @form(hashmap)
        locus RecMap { capacity { pool items of Rec indexed_by id; } }
        fn bump(m: RecMap, k: Int) -> Int {
            let mut e = m.get(k) or Rec { id: k };
            e.slots[1] = e.slots[1] + 1;
            m.set(e);
            return e.slots[1];
        }
        fn main() {
            let m = RecMap { };
            m.set(Rec { id: 1 });
            let o = Other { };
            println("bump=" + to_string(bump(m, 1)) + " " + to_string(bump(m, 1)) + " " + to_string(o.slots[0].k));
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let views = snap.demand_owned_views().expect("the column");
    let copies: Vec<&str> = views
        .program
        .cell_copies()
        .iter()
        .map(|c| &src[c.span.start.as_usize()..c.span.end.as_usize()])
        .collect();
    assert!(copies.is_empty(), "the inline write copies nothing: {copies:?}");
    let out = run_asan("inline_field", src);
    assert!(out.contains("bump=1 2 0\n"), "{out}");
}

/// A field of an imported seed's locus: `self.k.tick()` runs the seed's
/// `Kid.tick`, which writes `Kid`'s `count`, so a String read from
/// `self.f` stays a view across it, and `self.k.count` is an `Int`. By
/// name, `tick` was every locus's `tick`, the consumer's own (which
/// replaces `self.f`) included.
#[test]
fn an_imported_seeds_locus_is_its_own() {
    let lib_src = r#"
        type F { s: String = ""; }
        locus Kid {
            params { label: F = F { }; count: Int = 0; }
            fn put(t: String) { self.label = F { s: t }; }
            fn tick() { self.count = self.count + 1; }
        }
    "#;
    let app_src = r#"
        import "../lib" as lib;
        type G { s: String = ""; }
        locus H {
            params { f: G = G { }; k: lib::Kid = lib::Kid { }; }
            fn tick() { self.f = G { s: std::str::upper("replaced") }; }
            fn held() -> String {
                let s = self.f.s;
                self.k.tick();
                let n = self.k.count;
                self.k.put(std::str::upper("kid"));
                return s + to_string(n) + self.k.label.s;
            }
            fn own() -> String {
                let s = self.f.s;
                self.tick();
                return s;
            }
        }
        fn main() {
            let h = H { };
            h.f = G { s: std::str::upper("kept") };
            println("held=" + h.held());
            println("own=" + h.own());
        }
    "#;
    let dir = harness::unique_dir("owned_views_checker_import");
    let (app, lib) = (dir.join("app"), dir.join("lib"));
    std::fs::create_dir_all(&app).expect("app dir");
    std::fs::create_dir_all(&lib).expect("lib dir");
    std::fs::write(lib.join("kid.hl"), lib_src).expect("write lib");
    let entry = app.join("main.hl");
    std::fs::write(&entry, app_src).expect("write app");
    let Ok(snap) = Snapshot::load(&entry, LoadMode::WholeSeed, &Disk, Config::check(false, false)) else {
        panic!("the seed does not load");
    };
    let views = snap.demand_owned_views().expect("the column");
    let got: BTreeSet<String> = snap.programs().values().flat_map(|p| owned(&p.items, &views.program)).collect();
    // (`held.n`, an `Int` read through `self.k`, is flagged across
    // `self.k.put`, which may change what `self.k` holds; owning an `Int`
    // copies nothing.)
    assert!(!got.contains("held.s") && got.contains("own.s"), "{got:?}");
    let bin = harness::unique_bin("owned_views_checker_import");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    let built = build_opts::build_seed_dir(&entry, &bin, &options);
    let _ = std::fs::remove_dir_all(&dir);
    built.expect("build app + lib");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{:?}; stdout: {stdout}; stderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("held=KEPT1KID\n") && stdout.contains("own=KEPT\n"), "{stdout}");
}

/// The HTTP router's chain (`__http_run_chain`, the stdlib): the
/// not-found handler is a parameter written as a qualified interface
/// (`std::http::RouteHandler`), so `not_found.handle(cur)` is a call
/// through the interface, and its result binds no copy. A route
/// registered as a fn is called through a fn value (`__route_fn(cur)`):
/// no checker can name which fn it holds, so it may write anything, but
/// its declared signature (`fn(Context) -> Response`) says what it hands
/// back, and a `Response` neither is, holds, nor shares a block with the
/// `Context` it is handed: the response binds no copy either (it was
/// copied once per routed request). A middleware's `before`/`after`, an
/// interface method no locus of the program declares, runs nothing.
#[test]
fn the_route_chain_copies_no_response() {
    let src = "fn main() { println(\"x\"); }\n";
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let views = snap.demand_owned_views().expect("the column");
    let stdlib = hale_types::stdlib_bodies::program().expect("the stdlib parses");
    let chain = hale_syntax::ast::flat_decls(&stdlib.items)
        .find_map(|d| match d {
            TopDecl::Fn(f) if f.name.name == "__http_run_chain" => Some(f),
            _ => None,
        })
        .expect("the route chain");
    let mut got = BTreeSet::new();
    owned_in_all(&chain.body, &views.stdlib, &mut got);
    assert!(!got.iter().any(|g| g.contains("resp") || g.contains("cur")), "no response or context binds a copy: {got:?}");
}

/// What a call through a fn value hands back aliases (a part of) an
/// argument only where the types allow it: the result is, holds, or
/// shares a block with the argument. `take: fn(Wrap) -> In` may hand
/// back `w.inner`, so a write into its result copies the result first
/// (the caller's `self.w` keeps its value); `make: fn(Wrap) -> Out` shares
/// no block with `Wrap` (only a String, which no write into the result
/// reaches), so its result is written in place with no copy. Before, a
/// call through a fn value handed back anything.
#[test]
fn a_fn_values_result_aliases_only_what_its_types_allow() {
    let src = r#"
        type In { k: Int = 0; s: String = ""; }
        type Wrap { inner: In = In { }; }
        type Out { n: Int = 0; s: String = ""; }
        fn pick(w: Wrap) -> In { return w.inner; }
        fn fresh(w: Wrap) -> Out { return Out { n: w.inner.k, s: w.inner.s }; }
        locus H {
            params { w: Wrap = Wrap { }; take: fn(Wrap) -> In = pick; make: fn(Wrap) -> Out = fresh; }
            fn aliasing() -> Int {
                let mut x = self.take(self.w);
                x.k = 5;
                return self.w.inner.k * 10 + x.k;
            }
            fn unrelated() -> String {
                let mut o = self.make(self.w);
                o.n = 9;
                return o.s + to_string(o.n);
            }
        }
        fn main() {
            let h = H { };
            h.w = Wrap { inner: In { k: 1, s: std::str::upper("x") } };
            println("aliasing=" + to_string(h.aliasing()));
            println("unrelated=" + h.unrelated());
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let views = snap.demand_owned_views().expect("the column");
    let copies: Vec<&str> = views
        .program
        .cell_copies()
        .iter()
        .filter(|c| c.span.start.as_usize() < src.find("fn main").expect("main"))
        .map(|c| &src[c.span.start.as_usize()..c.span.end.as_usize()])
        .collect();
    assert_eq!(copies, ["x.k = 5;"], "only the write into what may be `w.inner` copies");
    let out = run_asan("fn_value_alias", src);
    assert!(out.contains("aliasing=15\n") && out.contains("unrelated=X9\n"), "{out}");
}

/// Every owned `let` and bare `=` of `b`, at any depth, as `let <name>`
/// or `assign <name> = <callee>`.
fn owned_in_all(b: &Block, views: &OwnedViews, out: &mut BTreeSet<String>) {
    use hale_syntax::ast::{ElseBranch, Expr};
    for s in &b.stmts {
        match s {
            Stmt::Let { id, name, .. } if views.node(*id) => {
                out.insert(format!("let {}", name.name));
            }
            Stmt::Assign { id, target, value, .. } if target.tail.is_empty() && views.node(*id) => {
                let callee = match value {
                    Expr::Call { callee, .. } => match callee.as_ref() {
                        Expr::Ident(i) => i.name.clone(),
                        Expr::Field { name, .. } => name.name.clone(),
                        _ => "?".to_string(),
                    },
                    _ => "?".to_string(),
                };
                out.insert(format!("assign {} = {callee}", target.head.name));
            }
            _ => {}
        }
        match s {
            Stmt::For { body, .. } | Stmt::While { body, .. } | Stmt::Block(body) => owned_in_all(body, views, out),
            Stmt::If(i) => {
                owned_in_all(&i.then_block, views, out);
                let mut next = i.else_block.as_deref();
                while let Some(e) = next {
                    match e {
                        ElseBranch::Else(b) => {
                            owned_in_all(b, views, out);
                            next = None;
                        }
                        ElseBranch::ElseIf(i2) => {
                            owned_in_all(&i2.then_block, views, out);
                            next = i2.else_block.as_deref();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// A `match` payload is typed by its variant's field (the column's row;
/// the checker's own local stays untyped, its diagnostics unchanged): a
/// `String` payload handed to a fn value holds no block a write could
/// reach, so it binds no copy; a record payload handed to a callee that
/// may write through it is still bound as a copy, so the write never
/// reaches the matched value. Before, every payload was a binding of
/// unknown type, and both were copied.
#[test]
fn a_payload_is_its_variants_field() {
    let src = r#"
        type In { k: Int = 0; }
        type E = enum { A(String), B(In) };
        fn shout(s: String) -> Int { return len(s); }
        fn bump(r: In) -> Int { r.k = r.k + 1; return r.k; }
        locus H {
            params { f: fn(String) -> Int = shout; g: fn(In) -> Int = bump; }
            fn text(v: E) -> Int {
                let n = match v { E::A(s) -> self.f(s), E::B(r) -> 0, };
                return n;
            }
            fn record(v: E) -> Int {
                let n = match v { E::B(r) -> self.g(r), E::A(s) -> 0, };
                let after = match v { E::B(r) -> r.k, E::A(s) -> 0, };
                return n * 10 + after;
            }
        }
        fn main() {
            let h = H { };
            println("text=" + to_string(h.text(E::A(std::str::upper("abc")))));
            println("record=" + to_string(h.record(E::B(In { k: 4 }))));
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let views = snap.demand_owned_views().expect("the column");
    let main_at = src.find("fn main").expect("main");
    let copies: Vec<(&str, &str)> = views
        .program
        .cell_copies()
        .iter()
        .filter(|c| c.span.start.as_usize() < main_at)
        .map(|c| (&src[c.span.start.as_usize()..c.span.end.as_usize()], c.reason))
        .collect();
    assert!(copies.iter().all(|(t, _)| !t.contains("self.f(s)")), "a String payload binds no copy: {copies:?}");
    assert!(copies.iter().any(|(t, _)| t.contains("self.g(r)")), "a record payload handed to a writer is copied: {copies:?}");
    let out = run_asan("payload", src);
    assert!(out.contains("text=3\n") && out.contains("record=54\n"), "{out}");
}

/// A monomorph (`Cfg<Int>` is `Cfg_Int`) is classified through its
/// instantiation: a record, so a `let` of a field holding one is a copy
/// already (a record read from a place), and handing the local to a
/// callee that writes through its parameter (declared `Cfg<Int>`, a
/// record) copies nothing more. As a name no declaration answers it was a
/// type of unknown class, and the binding was copied again for the call.
/// A String read from it and held across a replace of the field is still
/// owned.
#[test]
fn a_monomorph_is_its_instantiation() {
    let src = r#"
        type Cfg<T> { n: T; label: String = ""; }
        fn poke(c: Cfg<Int>) { c.n = 9; }
        locus H {
            params { cfg: Cfg<Int> = Cfg_Int { n: 1 }; }
            fn reset() { self.cfg = Cfg_Int { n: 2, label: std::str::upper("new") }; }
            fn record() -> Int {
                let c = self.cfg;
                poke(c);
                return self.cfg.n * 10 + c.n;
            }
            fn held() -> String {
                let s = self.cfg.label;
                self.reset();
                return s;
            }
        }
        fn main() {
            let h = H { };
            h.cfg = Cfg_Int { n: 1, label: std::str::upper("old") };
            println("record=" + to_string(h.record()));
            println("held=" + h.held());
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let views = snap.demand_owned_views().expect("the column");
    let main_at = src.find("fn main").expect("main");
    let copies: Vec<&str> = views
        .program
        .cell_copies()
        .iter()
        .filter(|c| c.span.start.as_usize() < main_at)
        .map(|c| &src[c.span.start.as_usize()..c.span.end.as_usize()])
        .collect();
    assert!(copies.is_empty(), "a monomorph record bound by `let` is copied nothing more: {copies:?}");
    assert!(owned_of(src).contains("held.s"), "the String held across the replace is owned");
    let out = run_asan("monomorph", src);
    assert!(out.contains("record=19\n") && out.contains("held=OLD\n"), "{out}");
}
