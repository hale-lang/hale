//! v1.x Phase 2: hierarchical topics, `subject:` field, `main`
//! locus modifier, and `bindings { }` block. Plus the closed-
//! world intra-locus direct-call optimization.
//!
//! Phase-2 surface (per spec/semantics.md):
//!   topic Events { payload: Event; subject: "events"; }
//!   topic Login : Events { payload: Login; subject: "login"; }
//!   main locus App {
//!     bindings { Login: unix("/tmp/x.sock") : listen; }
//!   }
//!
//! Wire subject for `Login` is `events.login` (parent.own).
//!
//! The intra-locus optimization rewrites
//!   `Foo <- value;` → `self.handler(value);`
//! at desugar time when Foo is published+subscribed only inside
//! the same locus AND has no binding.

use std::process::Command;

use aperio_codegen::build_executable;
use aperio_syntax::{ast::*, parse_source};
use aperio_syntax::desugar::{desugar_intra_locus_topics, desugar_topics};

fn parse(src: &str) -> Program {
    parse_source(src).expect("parse")
}

fn typecheck_diags(src: &str) -> Vec<String> {
    use aperio_types::symbol::Bundle;
    let program = parse(src);
    let mut programs: std::collections::BTreeMap<
        String,
        &aperio_syntax::ast::Program,
    > = std::collections::BTreeMap::new();
    programs.insert("test.ap".to_string(), &program);
    let bundle = Bundle { programs };
    let (scope, mut diags) = aperio_types::resolve::build_top_scope(&bundle);
    diags.extend(aperio_types::check::check_bundle(&bundle, &scope));
    diags.iter().map(|d| d.message.clone()).collect()
}

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = parse(src);
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_phase2_{}", name));
    build_executable(&program, &bin).expect("build");
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
            bindings { Foo: in_memory; }
        }
        fn main() { App { }; }
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
            bindings { Foo: in_memory; }
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
            bindings { NoSuch: in_memory; }
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
                Foo: in_memory;
                Foo: unix("/tmp/x.sock") : listen;
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("already bound")),
        "expected dup-binding diag; got: {:?}",
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
        }
        fn main() { Loop { }; }
    "#);
    desugar_intra_locus_topics(&mut p);
    // Locate Loop.birth's first stmt — should be Stmt::Expr(Call(...self.on_beat...))
    let mut found = false;
    for it in &p.items {
        if let TopDecl::Locus(l) = it {
            if l.name.name != "Loop" {
                continue;
            }
            for m in &l.members {
                if let LocusMember::Lifecycle(lc) = m {
                    if !matches!(lc.kind, LifecycleKind::Birth) {
                        continue;
                    }
                    if let Some(stmt) = lc.body.stmts.first() {
                        if let Stmt::Expr(Expr::Call { callee, .. }) = stmt {
                            if let Expr::Field { receiver, name, .. } = callee.as_ref() {
                                if matches!(receiver.as_ref(), Expr::KwSelf(_))
                                    && name.name == "on_beat"
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
    assert!(found, "expected birth to be rewritten to self.on_beat(...)");
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
    desugar_intra_locus_topics(&mut p);
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
            bindings { Beat: unix("/tmp/x.sock") : listen; }
        }
        fn main() { App { }; Loop { }; }
    "#);
    desugar_intra_locus_topics(&mut p);
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
    desugar_intra_locus_topics(&mut p);
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
    desugar_intra_locus_topics(&mut p);
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
    desugar_intra_locus_topics(&mut p);
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
    // synchronous: birth() runs, the handler increments sum, and
    // by the time fn main reads c.sum the value reflects the
    // synchronous mutation. (Bus dispatch is deferred-cooperative
    // — without the optimization, c.sum would still be 0 right
    // after construction.)
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
            birth() {
                Beat <- Tick { n: 1 };
                Beat <- Tick { n: 2 };
                Beat <- Tick { n: 3 };
            }
        }
        fn main() {
            let c = Counter { };
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
