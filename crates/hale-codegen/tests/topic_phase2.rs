//! v1.x Phase 2: hierarchical topics, `subject:` field, `main`
//! locus modifier, and `bindings { }` block. Plus the closed-
//! world intra-locus direct-call optimization.
//!
//! Phase-2 surface (per spec/semantics.md):
//!   topic Events { payload: Event; subject: "events"; }
//!   topic Login : Events { payload: Login; subject: "login"; }
//!   main locus App {
//!     bindings { Login: unix("/tmp/x.sock"); }
//!   }
//!
//! Wire subject for `Login` is `events.login` (parent.own).
//!
//! The intra-locus optimization rewrites
//!   `Foo <- value;` → `self.handler(value);`
//! at desugar time when Foo is published+subscribed only inside
//! the same locus AND has no binding.

use std::process::Command;

use hale_syntax::{ast::*, parse_source};
use hale_syntax::desugar::{desugar_intra_locus_topics, desugar_topics};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn parse(src: &str) -> Program {
    parse_source(src).expect("parse")
}

fn typecheck_diags(src: &str) -> Vec<String> {
    use hale_types::symbol::Bundle;
    let program = parse(src);
    let mut programs: std::collections::BTreeMap<
        String,
        &hale_syntax::ast::Program,
    > = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let (scope, mut diags) = hale_types::resolve::build_top_scope(&bundle);
    diags.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    diags.iter().map(|d| d.message.clone()).collect()
}

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let bin = harness::unique_bin(&format!("hale_test_phase2_{}", name));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    bin
}

// ---- subject: field ---------------------------------------------------

#[test]
fn topic_subject_field_parses() {
    let src = r#"
        type T { n: Int; }
        topic Events { payload: T; subject: "events"; }
        fn main() { }
    "#;
    let p = parse(src);
    let topic = p.items.iter().find_map(|it| match it {
        TopDecl::Topic(t) => Some(t),
        _ => None,
    }).expect("topic");
    assert_eq!(topic.subject.as_deref(), Some("events"));
}

#[test]
fn duplicate_wire_subject_errors() {
    let src = r#"
        type T { n: Int; }
        topic A { payload: T; subject: "shared"; }
        topic B { payload: T; subject: "shared"; }
        fn main() { }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("shares wire subject") && m.contains("shared")),
        "expected dup-wire-subject diag; got: {:?}",
        diags,
    );
}

// ---- hierarchical topics ----------------------------------------------

#[test]
fn topic_parent_chain_parses() {
    let src = r#"
        type T { n: Int; }
        topic Events { payload: T; subject: "events"; }
        topic Login : Events { payload: T; subject: "login"; }
        fn main() { }
    "#;
    let p = parse(src);
    let login = p.items.iter().find_map(|it| match it {
        TopDecl::Topic(t) if t.name.name == "Login" => Some(t),
        _ => None,
    }).expect("Login topic");
    assert_eq!(login.parent.as_ref().map(|i| i.name.as_str()), Some("Events"));
}

#[test]
fn unknown_parent_errors() {
    let src = r#"
        type T { n: Int; }
        topic Login : NoSuch { payload: T; }
        fn main() { }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("unknown parent topic") && m.contains("NoSuch")),
        "expected unknown-parent diag; got: {:?}",
        diags,
    );
}

#[test]
fn parent_cycle_errors() {
    let src = r#"
        type T { n: Int; }
        topic A : B { payload: T; }
        topic B : A { payload: T; }
        fn main() { }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("cycle")),
        "expected cycle diag; got: {:?}",
        diags,
    );
}

#[test]
fn hierarchical_subject_desugars_to_dot_path() {
    let mut p = parse(r#"
        type T { n: Int; }
        topic Events { payload: T; subject: "events"; }
        topic Login : Events { payload: T; subject: "login"; }
        locus L {
            bus { subscribe Login as h; }
            fn h(t: T) { }
        }
        fn main() { L { }; }
    "#);
    desugar_topics(&mut p);
    // Find L's subscribe; subject should be "events.login".
    let mut got: Option<String> = None;
    for it in &p.items {
        if let TopDecl::Locus(l) = it {
            if l.name.name == "L" {
                for m in &l.members {
                    if let LocusMember::Bus(b) = m {
                        for bm in &b.members {
                            if let BusMember::Subscribe { subject, .. } = bm {
                                if let BusSubject::Literal { subject: s, .. } = subject {
                                    got = Some(s.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(got.as_deref(), Some("events.login"));
}

// ---- main locus + bindings --------------------------------------------

#[test]
fn main_locus_modifier_parses() {
    let src = r#"
        type T { n: Int; }
        topic Foo { payload: T; }
        main locus App {
            bindings { Foo: unix("/tmp/x.sock"); }
        }
        locus Pub { bus { publish Foo; } birth() { Foo <- T { n: 0 }; } }
        fn main() { App { }; Pub { }; }
    "#;
    let p = parse(src);
    let app = p.items.iter().find_map(|it| match it {
        TopDecl::Locus(l) if l.is_main => Some(l),
        _ => None,
    }).expect("main locus");
    assert!(app.is_main);
    assert_eq!(app.name.name, "App");
}

#[test]
fn bindings_in_non_main_locus_rejected_at_parse() {
    let src = r#"
        type T { n: Int; }
        topic Foo { payload: T; }
        locus Other {
            bindings { Foo: unix("/tmp/x.sock"); }
        }
        fn main() { Other { }; }
    "#;
    // Parser should error rather than producing a Bindings entry
    // on a non-main locus.
    let result = parse_source(src);
    assert!(
        result.is_err(),
        "expected parse error for bindings in non-main locus; got program",
    );
}

#[test]
fn binding_to_unknown_topic_errors() {
    let src = r#"
        type T { n: Int; }
        main locus App {
            bindings { NoSuch: unix("/tmp/x.sock"); }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("unknown topic") && m.contains("NoSuch")),
        "expected unknown-topic-in-binding diag; got: {:?}",
        diags,
    );
}

#[test]
fn duplicate_binding_for_same_topic_errors() {
    let src = r#"
        type T { n: Int; }
        topic Foo { payload: T; }
        main locus App {
            bindings {
                Foo: unix("/tmp/a.sock");
                Foo: unix("/tmp/b.sock");
            }
        }
        locus Pub { bus { publish Foo; } birth() { Foo <- T { n: 0 }; } }
        fn main() { App { }; Pub { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("already bound")),
        "expected dup-binding diag; got: {:?}",
        diags,
    );
}

#[test]
fn binding_without_publisher_or_subscriber_errors() {
    // After the v1.x bus-transport refactor: a bindings entry
    // with no publisher AND no subscriber for the topic in the
    // bundle has no role to infer, so the typechecker emits a
    // diagnostic rather than silently producing dead code.
    let src = r#"
        type T { n: Int; }
        topic Foo { payload: T; }
        main locus App {
            bindings { Foo: unix("/tmp/x.sock"); }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("no publisher or subscriber")),
        "expected role-uninferable diag; got: {:?}",
        diags,
    );
}

#[test]
fn binding_with_both_pub_and_sub_without_explicit_role_errors() {
    // Ambiguous case: some locus publishes Foo AND some locus
    // subscribes Foo, but the binding doesn't specify a role.
    // Compile error pointing to the explicit-role override.
    let src = r#"
        type T { n: Int; }
        topic Foo { payload: T; }
        locus P { bus { publish Foo; } birth() { Foo <- T { n: 1 }; } }
        locus S {
            bus { subscribe Foo as on_foo; }
            fn on_foo(t: T) { }
        }
        main locus App {
            bindings { Foo: unix("/tmp/x.sock"); }
        }
        fn main() { App { }; P { }; S { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("ambiguous") && m.contains("role:")),
        "expected ambiguous-role diag; got: {:?}",
        diags,
    );
}

#[test]
fn more_than_one_main_locus_errors() {
    let src = r#"
        type T { n: Int; }
        main locus A { }
        main locus B { }
        fn main() { A { }; B { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("more than one `main` locus")),
        "expected multi-main diag; got: {:?}",
        diags,
    );
}

// ---- intra-locus optimization -----------------------------------------

/// A send to the locus's own subscription keeps its receiver as
/// `self.on_beat(...)`. Lowering guards that receiver's readiness so
/// sends in birth or its helpers wait until birth has completed
/// (decision line 6, F.40 phase 3 L4).
#[test]
fn intra_locus_send_rewrites_to_self_call() {
    let mut p = parse(r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; }
        locus Loop {
            bus {
                publish Beat;
                subscribe Beat as on_beat;
            }
            fn on_beat(t: Tick) { }
            birth() { Beat <- Tick { n: 1 }; }
            run() { Beat <- Tick { n: 2 }; }
        }
        fn main() { Loop { }; }
    "#);
    desugar_intra_locus_topics(&mut p, &Default::default());
    // The first stmt of Loop's `kind` lifecycle method.
    let first = |kind: LifecycleKind| -> Stmt {
        p.items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.name.name == "Loop" => l.members.iter().find_map(|m| match m {
                    LocusMember::Lifecycle(lc) if lc.kind == kind => lc.body.stmts.first().cloned(),
                    _ => None,
                }),
                _ => None,
            })
            .expect("the method has a statement")
    };
    let is_self_call = |stmt: &Stmt| match stmt {
        Stmt::Expr(Expr::Call { callee, .. }) => matches!(
            callee.as_ref(),
            Expr::Field { receiver, name, .. } if matches!(receiver.as_ref(), Expr::KwSelf(_)) && name.name == "on_beat"
        ),
        _ => false,
    };
    assert!(is_self_call(&first(LifecycleKind::Run)), "expected run to be rewritten to self.on_beat(...)");
    assert!(is_self_call(&first(LifecycleKind::Birth)), "birth keeps the receiver identity; lowering guards readiness");
}

#[test]
fn intra_locus_optimization_skipped_when_pub_and_sub_in_different_loci() {
    let mut p = parse(r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; }
        locus Pub { bus { publish Beat; } birth() { Beat <- Tick { n: 1 }; } }
        locus Sub {
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: Tick) { }
        }
        fn main() { Sub { }; Pub { }; }
    "#);
    desugar_intra_locus_topics(&mut p, &Default::default());
    // Pub.birth's first stmt should still be a Stmt::Send (no rewrite).
    let mut still_send = false;
    for it in &p.items {
        if let TopDecl::Locus(l) = it {
            if l.name.name != "Pub" {
                continue;
            }
            for m in &l.members {
                if let LocusMember::Lifecycle(lc) = m {
                    if let Some(Stmt::Send { .. }) = lc.body.stmts.first() {
                        still_send = true;
                    }
                }
            }
        }
    }
    assert!(still_send, "expected cross-locus send to be left alone");
}

#[test]
fn intra_locus_optimization_skipped_when_topic_is_bound() {
    let mut p = parse(r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; }
        locus Loop {
            bus {
                publish Beat;
                subscribe Beat as on_beat;
            }
            fn on_beat(t: Tick) { }
            birth() { Beat <- Tick { n: 1 }; }
        }
        main locus App {
            bindings { Beat: unix("/tmp/x.sock"); }
        }
        fn main() { App { }; Loop { }; }
    "#);
    desugar_intra_locus_topics(&mut p, &Default::default());
    // Bound topic must NOT be optimized — the binding may publish
    // to remote subscribers we can't see at compile time.
    let mut still_send = false;
    for it in &p.items {
        if let TopDecl::Locus(l) = it {
            if l.name.name != "Loop" {
                continue;
            }
            for m in &l.members {
                if let LocusMember::Lifecycle(lc) = m {
                    if let Some(Stmt::Send { .. }) = lc.body.stmts.first() {
                        still_send = true;
                    }
                }
            }
        }
    }
    assert!(still_send, "bound topic must not be optimized");
}

#[test]
fn tower_parent_publishes_child_subscribes_rewrites_to_chained_call() {
    // Parent locus owns a child via params; the child is the
    // sole subscriber; parent publishes. The desugar pass should
    // rewrite `Beat <- t` in parent.birth() to
    // `self.child.on_beat(t)`.
    let mut p = parse(r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; }
        locus Child {
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: Tick) { }
        }
        locus Parent {
            params { child: Child = Child { }; }
            bus { publish Beat; }
            birth() { Beat <- Tick { n: 1 }; }
        }
        fn main() { Parent { }; }
    "#);
    desugar_intra_locus_topics(&mut p, &Default::default());
    let mut found = false;
    for it in &p.items {
        if let TopDecl::Locus(l) = it {
            if l.name.name != "Parent" {
                continue;
            }
            for m in &l.members {
                if let LocusMember::Lifecycle(lc) = m {
                    if !matches!(lc.kind, LifecycleKind::Birth) {
                        continue;
                    }
                    if let Some(Stmt::Expr(Expr::Call { callee, .. })) = lc.body.stmts.first() {
                        if let Expr::Field { receiver, name, .. } = callee.as_ref() {
                            if name.name == "on_beat" {
                                if let Expr::Field { receiver: inner, name: f, .. } = receiver.as_ref() {
                                    if matches!(inner.as_ref(), Expr::KwSelf(_))
                                        && f.name == "child"
                                    {
                                        found = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(found, "expected birth to be rewritten to self.child.on_beat(...)");
}

#[test]
fn tower_optimization_skipped_when_parent_has_two_subscriber_children() {
    // Ambiguity: parent has TWO fields of the subscriber type.
    // The bus would broadcast to both; the desugar pass must not
    // pick one arbitrarily, so the Send stays.
    let mut p = parse(r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; }
        locus Child {
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: Tick) { }
        }
        locus Parent {
            params {
                a: Child = Child { };
                b: Child = Child { };
            }
            bus { publish Beat; }
            birth() { Beat <- Tick { n: 1 }; }
        }
        fn main() { Parent { }; }
    "#);
    desugar_intra_locus_topics(&mut p, &Default::default());
    let mut still_send = false;
    for it in &p.items {
        if let TopDecl::Locus(l) = it {
            if l.name.name != "Parent" {
                continue;
            }
            for m in &l.members {
                if let LocusMember::Lifecycle(lc) = m {
                    if let Some(Stmt::Send { .. }) = lc.body.stmts.first() {
                        still_send = true;
                    }
                }
            }
        }
    }
    assert!(still_send, "expected ambiguous-child case to fall through to bus");
}

#[test]
fn tower_optimization_skipped_when_subscriber_is_two_hops_away() {
    // Multi-hop tower: Outer contains Middle, Middle contains
    // Leaf, Leaf subscribes, Outer publishes. v1 optimization
    // only handles single-hop towers — fall through to bus.
    let mut p = parse(r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; }
        locus Leaf {
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: Tick) { }
        }
        locus Middle {
            params { leaf: Leaf = Leaf { }; }
        }
        locus Outer {
            params { mid: Middle = Middle { }; }
            bus { publish Beat; }
            birth() { Beat <- Tick { n: 1 }; }
        }
        fn main() { Outer { }; }
    "#);
    desugar_intra_locus_topics(&mut p, &Default::default());
    let mut still_send = false;
    for it in &p.items {
        if let TopDecl::Locus(l) = it {
            if l.name.name != "Outer" {
                continue;
            }
            for m in &l.members {
                if let LocusMember::Lifecycle(lc) = m {
                    if let Some(Stmt::Send { .. }) = lc.body.stmts.first() {
                        still_send = true;
                    }
                }
            }
        }
    }
    assert!(still_send, "expected multi-hop tower to fall through to bus");
}

#[test]
fn tower_parent_publishes_child_subscribes_round_trip_end_to_end() {
    // Synchronous-call semantics: the rewrite makes the handler
    // fire inline at the Send site, so a post-construction read
    // of the child's state via the parent must see the
    // accumulated total — without the optimization, the bus
    // dispatch would defer and the post-construct read would
    // see the initial value.
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; }
        locus Counter {
            params { sum: Int = 0; }
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: Tick) { self.sum = self.sum + t.n; }
        }
        locus Driver {
            params { counter: Counter = Counter { }; }
            bus { publish Beat; }
            birth() {
                Beat <- Tick { n: 1 };
                Beat <- Tick { n: 2 };
                Beat <- Tick { n: 3 };
            }
        }
        fn main() {
            let d = Driver { };
            print("sum=");
            println(d.counter.sum);
        }
    "#;
    let bin = build("tower_parent_child_round_trip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sum=6"), "got: {:?}", stdout);
}

#[test]
fn intra_locus_round_trip_end_to_end() {
    // The optimized direct-call path should be observable as
    // synchronous: fire() runs, the handler increments sum, and by
    // the time fn main reads c.sum the value reflects the synchronous
    // mutation. (Bus dispatch is deferred-cooperative — without the
    // optimization, c.sum would still be 0 right after the call.)
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; }
        locus Counter {
            params { sum: Int = 0; }
            bus {
                publish Beat;
                subscribe Beat as on_beat;
            }
            fn on_beat(t: Tick) { self.sum = self.sum + t.n; }
            fn fire() {
                Beat <- Tick { n: 1 };
                Beat <- Tick { n: 2 };
                Beat <- Tick { n: 3 };
            }
        }
        fn main() {
            let c = Counter { };
            c.fire();
            print("sum=");
            println(c.sum);
        }
    "#;
    let bin = build("intra_locus_round_trip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sum=6"), "got: {:?}", stdout);
}

/// Decision line 6 (F.40 phase 3, L4): what birth() publishes to its own
/// subscription is delivered once the birth has completed, in order,
/// never during it and never dropped.
#[test]
fn a_births_own_publishes_are_delivered_after_it_in_order() {
    // An ordinary string, not a raw one: the corpus harvests raw string
    // programs out of test files, and this one belongs to this test.
    let src = "type Tick { n: Int; }
topic Beat { payload: Tick; }
locus Counter {
    params { sum: Int = 0; born: Int = 0; early: Int = 0; order: Int = 0; }
    bus {
        publish Beat;
        subscribe Beat as on_beat;
    }
    fn on_beat(t: Tick) {
        if self.born == 0 { self.early = self.early + 1; }
        self.order = self.order * 10 + t.n;
        self.sum = self.sum + t.n;
    }
    birth() {
        Beat <- Tick { n: 1 };
        Beat <- Tick { n: 2 };
        std::time::sleep(5ms);
        Beat <- Tick { n: 3 };
        self.born = 1;
    }
}
fn main() {
    let c = Counter { };
    std::time::sleep(5ms);
    println(\"sum=\", c.sum, \" early=\", c.early, \" order=\", c.order);
}
";
    for (name, program) in [
        ("direct", src.to_string()),
        ("helper_literal", src.replace(
            "    birth() {",
            "    fn fire(n: Int) { Beat <- Tick { n: n }; }\n    birth() {",
        ).replace("Beat <- Tick { n: 1 };", "self.fire(1);")
            .replace("Beat <- Tick { n: 2 };", "self.fire(2);")
            .replace("Beat <- Tick { n: 3 };", "self.fire(3);")),
        ("helper_variable", src.replace(
            "    birth() {",
            "    fn fire(n: Int) { let t = Tick { n: n }; Beat <- t; }\n    birth() {",
        ).replace("Beat <- Tick { n: 1 };", "self.fire(1);")
            .replace("Beat <- Tick { n: 2 };", "self.fire(2);")
            .replace("Beat <- Tick { n: 3 };", "self.fire(3);")),
    ] {
        let bin = build(&format!("births_own_publishes_after_it_{name}"), &program);
        let out = Command::new(&bin).output().expect("run");
        let _ = std::fs::remove_file(&bin);
        assert!(out.status.success(), "{name}: non-zero: {:?}", out.status);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("sum=6 early=0 order=123"), "{name}: got {stdout:?}");
    }
}

/// The pinned thread remains its mailbox's consumer during birth, so
/// already-born children progress, while its own sends wait for readiness.
/// More sends than ring slots must not make that thread wait on itself.
#[test]
fn a_pinned_birth_keeps_nested_delivery_live_and_can_publish_past_capacity() {
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; }
topic NestedBeat { payload: Tick; }
locus Leaf {
    params { heard: Int = 0; }
    bus { subscribe NestedBeat as on_tick; }
    fn on_tick(t: Tick) { self.heard = self.heard + 1; }
}
locus Counter {
    params { leaf: Leaf = Leaf { }; born: Int = 0; early: Int = 0; heard: Int = 0; }
    bus { publish Beat; publish NestedBeat; subscribe Beat as on_tick; }
    fn on_tick(t: Tick) {
        self.heard = self.heard + 1;
        if self.born == 0 { self.early = self.early + 1; }
    }
    birth() {
        NestedBeat <- Tick { n: 0 };
        std::time::sleep(5ms);
        println("nested=", self.leaf.heard);
        for i in 0..256 { Beat <- Tick { n: i }; }
        self.born = 1;
    }
    run() {
        std::time::sleep(5ms);
        println("heard=", self.heard, " early=", self.early);
    }
}
main locus App {
    params { c: Counter = Counter { }; }
    placement { c: pinned; }
    run() { std::time::sleep(100ms); }
}
fn main() { App { }; }
"#;
    let bin = harness::unique_bin("hale_birth_pinned_full");
    let opts = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).expect("build");
    let mut child = Command::new(&bin)
        .env("LOTUS_BUS_QUEUE_CAP", "64")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn().expect("run");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut timed_out = false;
    while child.try_wait().expect("status").is_none() {
        if std::time::Instant::now() >= deadline {
            timed_out = true;
            child.kill().expect("stop the stalled child");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let out = child.wait_with_output().expect("output");
    let _ = std::fs::remove_file(&bin);
    assert!(!timed_out, "pinned birth blocked on its own delivery");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero: {:?}: {stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("nested=1"), "ready nested subscriber stalled: {stdout}");
    assert!(stdout.contains("heard=256 early=0"), "birth deliveries lost or early: {stdout}");
}

/// Another instance's birth window cannot turn a ready receiver's
/// same-instance call into a broadcast to every instance of its type.
#[test]
fn a_ready_fused_receiver_stays_local_during_another_birth() {
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; }
topic Other { payload: Tick; }
locus Counter {
    params { heard: Int = 0; }
    bus { publish Beat; subscribe Beat as on_tick; }
    fn on_tick(t: Tick) { self.heard = self.heard + 1; }
    fn fire() { Beat <- Tick { n: 1 }; }
}
main locus App {
    params { a: Counter = Counter { }; b: Counter = Counter { }; }
    bus { subscribe Other as on_other; }
    fn on_other(t: Tick) { }
    birth() {
        self.a.fire();
        std::time::sleep(1ms);
        println("a=", self.a.heard, " b=", self.b.heard);
    }
}
fn main() { App { }; }
"#;
    let bin = build("ready_fused_receiver", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a=1 b=0"), "ready receiver was broadcast: {stdout}");
}

/// Deferring a same-instance birth send must keep its receiver, and a
/// payload allocated in the helper must survive the helper's return.
#[test]
fn a_deferred_fused_birth_keeps_its_receiver_and_managed_payload() {
    let src = r#"
type Tick { text: String; }
topic Beat { payload: Tick; }
locus Counter {
    params { heard: Int = 0; born: Int = 0; early: Int = 0; text: String = ""; }
    bus { publish Beat; subscribe Beat as on_tick; }
    fn on_tick(t: Tick) {
        self.heard = self.heard + 1;
        self.text = t.text;
        if self.born == 0 { self.early = self.early + 1; }
    }
    fn fire() { let t = Tick { text: "payload-" + to_string(123) }; Beat <- t; }
    birth() { self.fire(); std::time::sleep(1ms); self.born = 1; }
}
fn main() {
    let a = Counter { };
    let b = Counter { };
    std::time::sleep(5ms);
    println("a=", a.heard, " b=", b.heard, " early=", a.early + b.early);
    println(a.text, " ", b.text);
}
"#;
    let bin = harness::unique_bin("hale_birth_fused_managed");
    let opts = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero: {:?}: {stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a=1 b=1 early=0"), "wrong receiver or readiness: {stdout}");
    assert!(stdout.contains("payload-123 payload-123"), "payload did not survive: {stdout}");
}

/// The PR #1336 review's arrangement: `Pumper`'s handler, on pool
/// `side`'s worker, sends 200 flat cells to `Sub` while `Sub`'s birth
/// (on the instantiating thread) holds its delivery, with another cell
/// queued on `side` behind the handler. `SUBPOOL` places `Sub`. An
/// ordinary string, not a raw one: the corpus harvests raw string
/// programs out of test files, and this one belongs to these tests.
const READINESS_CAP_POOL: &str = "type Ping { n: Int; }
topic Pings { payload: Ping; }
topic Go { payload: Ping; }
topic Other { payload: Ping; }
locus Pumper {
    params { others: Int = 0; took: Int = 0; }
    bus { subscribe Go as on_go; subscribe Other as on_other; publish Pings; }
    fn on_go(g: Ping) {
        let t0 = std::time::monotonic_ns();
        for i in 0..g.n { Pings <- Ping { n: i }; }
        self.took = (std::time::monotonic_ns() - t0) / 1000000;
    }
    fn on_other(o: Ping) { self.others = self.others + 1; }
}
locus Sub {
    params { born: Int = 0; early: Int = 0; heard: Int = 0; next: Int = 0; disorder: Int = 0; }
    bus { subscribe Pings as on_ping; publish Go; publish Other; }
    fn on_ping(p: Ping) {
        if self.born == 0 { self.early = self.early + 1; }
        if p.n != self.next { self.disorder = self.disorder + 1; }
        self.next = p.n + 1;
        self.heard = self.heard + 1;
    }
    birth() {
        Go <- Ping { n: 200 };
        Other <- Ping { n: 0 };
        std::time::sleep(200ms);
        self.born = 1;
    }
}
main locus App {
    params { p: Pumper = Pumper { }; s: Sub = Sub { }; }
    placement { p: cooperative(pool = side); s: cooperative(pool = SUBPOOL); }
    run() {
        std::time::sleep(100ms);
        println(\"heard=\", self.s.heard, \" early=\", self.s.early, \" disorder=\", self.s.disorder, \" others=\", self.p.others);
        if self.p.took >= 100 { println(\"publisher waited\"); } else { println(\"publisher never waited\"); }
    }
}
fn main() { App { }; }
";

/// Build `src` under ASan in one dispatch mode and run it with a
/// 64-cell queue cap; the stdout of a run that finished cleanly.
fn run_at_readiness_cap(name: &str, src: &str, no_bus_devirt: bool) -> String {
    let bin = harness::unique_bin(name);
    let opts = hale_codegen::BuildOptions { asan: true, no_bus_devirt, ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).expect("build");
    let mut child = Command::new(&bin)
        .env("LOTUS_BUS_QUEUE_CAP", "64")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn().expect("run");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut timed_out = false;
    while child.try_wait().expect("status").is_none() {
        if std::time::Instant::now() >= deadline {
            timed_out = true;
            child.kill().expect("stop the stalled child");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let out = child.wait_with_output().expect("output");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mode = if no_bus_devirt { "dispatch" } else { "devirt" };
    assert!(!timed_out, "{name} ({mode}): readiness deadlocked; stdout: {stdout}");
    assert!(out.status.success(), "{name} ({mode}): non-zero: {:?}: {stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{name} ({mode}): {stderr}");
    stdout
}

/// PR #1336 review (P1): a pool's worker publishing past the readiness
/// cap to a held subscriber on its own pool parks instead of waiting,
/// so the readiness step's posts into that pool always drain. Before,
/// the worker waited at the cap and readiness blocked on the ring only
/// that worker drains. Every cell arrives once, in order, after birth.
#[test]
fn a_pool_worker_never_waits_at_the_readiness_cap_for_its_own_pool() {
    let src = READINESS_CAP_POOL.replace("SUBPOOL", "side");
    for no_bus_devirt in [false, true] {
        let stdout = run_at_readiness_cap("hale_readiness_cap_own_pool", &src, no_bus_devirt);
        assert!(stdout.contains("heard=200 early=0 disorder=0 others=1"), "lost, early or reordered: {stdout}");
        assert!(stdout.contains("publisher never waited"), "the pool's own worker waited at the cap: {stdout}");
    }
}

/// The control: a publisher on another domain (`Sub` on pool `other`)
/// keeps the bounded wait. It waits at the cap until `Sub`'s readiness,
/// then every cell arrives once, in order, after birth.
#[test]
fn a_publisher_on_another_domain_still_waits_at_the_readiness_cap() {
    let src = READINESS_CAP_POOL.replace("SUBPOOL", "other");
    for no_bus_devirt in [false, true] {
        let stdout = run_at_readiness_cap("hale_readiness_cap_other_pool", &src, no_bus_devirt);
        assert!(stdout.contains("heard=200 early=0 disorder=0 others=1"), "lost, early or reordered: {stdout}");
        assert!(stdout.contains("publisher waited"), "a cross-domain publisher skipped the cap: {stdout}");
    }
}

/// The pinned twin: a nested `Pumper`'s handler, drained on the pinned
/// `Sub`'s own thread during its birth, sends 200 cells to the held
/// `Sub`, with another cell queued in its mailbox. The thread that
/// drains the mailbox never waits at the cap; readiness posts into it.
/// `Quiet` keeps `Go` and `Other` posted rather than fused to a call.
#[test]
fn a_pinned_thread_never_waits_at_the_readiness_cap_for_its_own_mailbox() {
    let src = "type Ping { n: Int; }
topic Pings { payload: Ping; }
topic Go { payload: Ping; }
topic Other { payload: Ping; }
locus Pumper {
    params { others: Int = 0; took: Int = 0; }
    bus { subscribe Go as on_go; subscribe Other as on_other; publish Pings; }
    fn on_go(g: Ping) {
        let t0 = std::time::monotonic_ns();
        for i in 0..g.n { Pings <- Ping { n: i }; }
        self.took = (std::time::monotonic_ns() - t0) / 1000000;
    }
    fn on_other(o: Ping) { self.others = self.others + 1; }
}
locus Quiet {
    params { seen: Int = 0; }
    bus { subscribe Go as on_go; subscribe Other as on_other; }
    fn on_go(g: Ping) { self.seen = self.seen + 1; }
    fn on_other(o: Ping) { self.seen = self.seen + 1; }
}
locus Sub {
    params { p: Pumper = Pumper { }; q: Quiet = Quiet { }; born: Int = 0; early: Int = 0; heard: Int = 0; next: Int = 0; disorder: Int = 0; }
    bus { subscribe Pings as on_ping; publish Go; publish Other; }
    fn on_ping(p: Ping) {
        if self.born == 0 { self.early = self.early + 1; }
        if p.n != self.next { self.disorder = self.disorder + 1; }
        self.next = p.n + 1;
        self.heard = self.heard + 1;
    }
    birth() {
        Go <- Ping { n: 200 };
        Other <- Ping { n: 0 };
        std::time::sleep(200ms);
        self.born = 1;
    }
    run() {
        std::time::sleep(50ms);
        println(\"heard=\", self.heard, \" early=\", self.early, \" disorder=\", self.disorder, \" others=\", self.p.others, \" seen=\", self.q.seen);
        if self.p.took >= 100 { println(\"publisher waited\"); } else { println(\"publisher never waited\"); }
    }
}
main locus App {
    params { s: Sub = Sub { }; }
    placement { s: pinned; }
    run() { std::time::sleep(400ms); }
}
fn main() { App { }; }
";
    for no_bus_devirt in [false, true] {
        let stdout = run_at_readiness_cap("hale_readiness_cap_pinned", src, no_bus_devirt);
        assert!(stdout.contains("heard=200 early=0 disorder=0 others=1 seen=2"), "lost, early or reordered: {stdout}");
        assert!(stdout.contains("publisher never waited"), "the pinned thread waited at its own cap: {stdout}");
    }
}
