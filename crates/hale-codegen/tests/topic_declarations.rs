//! v1.x: `topic Foo { payload: T; }` declarations.
//!
//! Topics are typed pub/sub channels declared once at top level.
//! Subscribers + publishers + send sites reference a topic by
//! name, so the payload type travels with the topic decl instead
//! of being repeated at every `subscribe ... of type T` site.
//!
//! Pipeline (per spec):
//!   1. Parser produces `BusSubject::Topic(ident)` for
//!      `subscribe Foo as h;` / `publish Foo;`, and the existing
//!      `Stmt::Send { subject: Expr::Ident("Foo"), ... }` shape
//!      for `Foo <- expr`.
//!   2. Typecheck validates: payload from topic matches
//!      handler-sig, send-payload, etc.
//!   3. Desugar (`hale_syntax::desugar::desugar_topics`)
//!      rewrites topic refs to literal-subject equivalents
//!      before codegen / interpretation.
//!
//! Phase 1 deferred: `transport: external`, `bindings { }` in
//! main, `interface Transport`, user-implementable adapters.

use std::process::Command;

use hale_codegen::build_executable;
use hale_syntax::{parse_source, ast::*};
use hale_syntax::desugar::desugar_topics;

fn parse(src: &str) -> Program {
    parse_source(src).expect("parse")
}

#[test]
fn parses_topic_top_level_decl() {
    let src = r#"
        type Tick { n: Int; }
        topic Ticks { payload: Tick; }
        fn main() { }
    "#;
    let program = parse(src);
    let topic = program
        .items
        .iter()
        .find_map(|it| match it {
            TopDecl::Topic(t) => Some(t),
            _ => None,
        })
        .expect("topic decl present");
    assert_eq!(topic.name.name, "Ticks");
}

#[test]
fn parses_subscribe_and_publish_topic_refs() {
    let src = r#"
        type Tick { n: Int; }
        topic Ticks { payload: Tick; }

        locus Sub {
            bus { subscribe Ticks as on_tick; }
            fn on_tick(t: Tick) { }
        }
        locus Pub {
            bus { publish Ticks; }
            birth() { Ticks <- Tick { n: 1 }; }
        }
        fn main() { Sub { }; Pub { }; }
    "#;
    let program = parse(src);
    // Locate the Sub locus's subscribe AST shape.
    let sub_subj = program
        .items
        .iter()
        .find_map(|it| match it {
            TopDecl::Locus(l) if l.name.name == "Sub" => Some(l),
            _ => None,
        })
        .and_then(|l| {
            l.members.iter().find_map(|m| match m {
                LocusMember::Bus(b) => b.members.first(),
                _ => None,
            })
        })
        .and_then(|bm| match bm {
            BusMember::Subscribe { subject, .. } => Some(subject.clone()),
            _ => None,
        })
        .expect("subscribe present");
    match sub_subj {
        BusSubject::Topic(i) => assert_eq!(i.name, "Ticks"),
        other => panic!("expected topic-ref subject, got {:?}", other),
    }
}

#[test]
fn desugar_rewrites_topic_refs_to_literals() {
    let mut program = parse(r#"
        type Tick { n: Int; }
        topic Ticks { payload: Tick; }

        locus Sub {
            bus { subscribe Ticks as on_tick; }
            fn on_tick(t: Tick) { }
        }
        locus Pub {
            bus { publish Ticks; }
            birth() { Ticks <- Tick { n: 1 }; }
        }
        fn main() { Sub { }; Pub { }; }
    "#);
    desugar_topics(&mut program);

    // After desugar, every BusSubject should be Literal, and
    // every Publish.ty should be Some (the topic's payload).
    for it in &program.items {
        if let TopDecl::Locus(l) = it {
            for m in &l.members {
                if let LocusMember::Bus(b) = m {
                    for bm in &b.members {
                        match bm {
                            BusMember::Subscribe { subject, ty, .. } => {
                                match subject {
                                    BusSubject::Literal { subject: s, .. } => {
                                        assert_eq!(s, "Ticks");
                                    }
                                    BusSubject::Topic(_) => {
                                        panic!("expected literal subject after desugar");
                                    }
                                    BusSubject::QualifiedTopic(_) => {
                                        panic!("expected literal subject after desugar; got qualified");
                                    }
                                }
                                assert!(ty.is_some(), "ty filled after desugar");
                            }
                            BusMember::Publish { subject, ty, .. } => {
                                match subject {
                                    BusSubject::Literal { subject: s, .. } => {
                                        assert_eq!(s, "Ticks");
                                    }
                                    BusSubject::Topic(_) => {
                                        panic!("expected literal subject after desugar");
                                    }
                                    BusSubject::QualifiedTopic(_) => {
                                        panic!("expected literal subject after desugar; got qualified");
                                    }
                                }
                                assert!(ty.is_some(), "publish ty filled after desugar");
                            }
                        }
                    }
                }
            }
        }
    }
}

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = parse(src);
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_topic_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn topic_round_trip_end_to_end() {
    let src = r#"
        type Tick { n: Int; }
        topic Ticks { payload: Tick; }

        locus Counter {
            params { sum: Int = 0; }
            bus { subscribe Ticks as on_tick; }
            fn on_tick(t: Tick) { self.sum = self.sum + t.n; }
        }
        locus Pub {
            params { iters: Int = 4; }
            bus { publish Ticks; }
            run() {
                let mut i = 1;
                while i <= self.iters {
                    Ticks <- Tick { n: i };
                    i = i + 1;
                }
            }
        }

        fn main() {
            let c = Counter { };
            Pub { iters: 4 };
            print("sum=");
            println(c.sum);
        }
    "#;
    let bin = build("round_trip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // 1+2+3+4 = 10
    assert!(stdout.contains("sum=10"), "got: {:?}", stdout);
}

#[test]
fn topic_legacy_string_subjects_still_work() {
    // Mixing the new `topic Ticks` decl with a legacy string
    // subject elsewhere should not cross-contaminate. Two
    // independent channels here.
    let src = r#"
        type Tick { n: Int; }
        topic Ticks { payload: Tick; }

        locus SubA {
            params { got: Int = 0; }
            bus { subscribe Ticks as on_t; }
            fn on_t(t: Tick) { self.got = self.got + t.n; }
        }
        locus SubB {
            params { got: Int = 0; }
            bus { subscribe "legacy.signal" as on_s of type Tick; }
            fn on_s(t: Tick) { self.got = self.got + t.n; }
        }
        locus Pubber {
            bus {
                publish Ticks;
                publish "legacy.signal" of type Tick;
            }
            birth() {
                Ticks <- Tick { n: 10 };
                "legacy.signal" <- Tick { n: 100 };
            }
        }

        fn main() {
            let a = SubA { };
            let b = SubB { };
            Pubber { };
            print("a="); println(a.got);
            print("b="); println(b.got);
        }
    "#;
    let bin = build("legacy_coexists", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a=10"), "got: {:?}", stdout);
    assert!(stdout.contains("b=100"), "got: {:?}", stdout);
}

fn typecheck_diags(src: &str) -> Vec<String> {
    use hale_types::symbol::Bundle;
    let program = parse(src);
    let mut programs: std::collections::BTreeMap<
        String,
        &hale_syntax::ast::Program,
    > = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), &program);
    let bundle = Bundle { programs };
    let (scope, mut diags) = hale_types::resolve::build_top_scope(&bundle);
    diags.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    diags.iter().map(|d| d.message.clone()).collect()
}

#[test]
fn topic_ref_with_stray_of_type_clause_errors() {
    let src = r#"
        type Tick { n: Int; }
        topic Ticks { payload: Tick; }
        locus Bad {
            bus { subscribe Ticks as h of type Int; }
            fn h(t: Tick) { }
        }
        fn main() { Bad { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("of type")
            && m.contains("forbidden")
            && m.contains("Ticks")),
        "expected stray-of-type diag; got: {:?}",
        diags,
    );
}

#[test]
fn unknown_topic_ref_errors() {
    let src = r#"
        type Tick { n: Int; }
        locus Bad {
            bus { subscribe NoSuch as h; }
            fn h(t: Tick) { }
        }
        fn main() { Bad { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("unknown topic")
            && m.contains("NoSuch")),
        "expected unknown-topic diag; got: {:?}",
        diags,
    );
}
