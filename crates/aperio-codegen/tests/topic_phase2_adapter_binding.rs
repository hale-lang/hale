//! Wave B (bus-transport redesign) — adapter binding end-to-end.
//!
//! Tests the `Topic: MyLocus { ... };` binding form:
//!
//! 1. **Single subject, multiple publishes.** A producer publishes
//!    twice; the adapter's `send` fires twice with the bound
//!    subject.
//! 2. **Adapter carries state.** Adapter params survive registration
//!    (they're allocated in the program-lifetime payload arena via
//!    m90-style routing) and are readable inside `send`.
//! 3. **Two adapters, two subjects.** Independent adapter instances
//!    bound to different topics each see only their own subject's
//!    payloads.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_adapter_binding_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn adapter_send_fires_on_publish() {
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        locus MyAdapter {
            params { label: String = "noname"; }
            fn send(subject: String, bytes: Bytes) {
                println("adapter[" + self.label + "] subject=" + subject);
            }
        }

        locus Producer {
            bus { publish Beat; }
            birth() {
                Beat <- Tick { n: 1 };
                Beat <- Tick { n: 2 };
            }
        }

        main locus App {
            bindings {
                Beat: MyAdapter { label: "T" };
            }
        }

        fn main() {
            App { };
            Producer { };
        }
    "#;
    let (stdout, status) = build_and_run("single", src);
    assert!(status.success(), "non-zero: {:?}", status);
    let count = stdout
        .lines()
        .filter(|l| l.contains("adapter[T] subject=beat"))
        .count();
    assert_eq!(count, 2, "expected 2 send calls; got stdout: {:?}", stdout);
}

#[test]
fn adapter_field_inits_reach_send_body() {
    // The adapter is instantiated with explicit field values; the
    // `send` body reads `self.label` and should see the bound
    // value, not the default. This confirms the field inits flow
    // through to the locus's params via the synthetic Expr::Struct
    // lowering.
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        locus MyAdapter {
            params { label: String = "default"; }
            fn send(subject: String, bytes: Bytes) {
                println("label=" + self.label);
            }
        }

        locus Producer {
            bus { publish Beat; }
            birth() {
                Beat <- Tick { n: 1 };
            }
        }

        main locus App {
            bindings {
                Beat: MyAdapter { label: "explicit-value" };
            }
        }

        fn main() {
            App { };
            Producer { };
        }
    "#;
    let (stdout, status) = build_and_run("field_init", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("label=explicit-value"),
        "expected init value, not default; got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("label=default"),
        "default leaked through: {:?}",
        stdout
    );
}

#[test]
fn two_adapters_two_subjects_route_independently() {
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }
        topic Pulse { payload: Tick; subject: "pulse"; }

        locus AdA {
            params { tag: String = "a"; }
            fn send(subject: String, bytes: Bytes) {
                println("A tag=" + self.tag + " subject=" + subject);
            }
        }

        locus AdB {
            params { tag: String = "b"; }
            fn send(subject: String, bytes: Bytes) {
                println("B tag=" + self.tag + " subject=" + subject);
            }
        }

        locus Producer {
            bus { publish Beat; publish Pulse; }
            birth() {
                Beat <- Tick { n: 1 };
                Pulse <- Tick { n: 2 };
            }
        }

        main locus App {
            bindings {
                Beat: AdA { tag: "alpha" };
                Pulse: AdB { tag: "beta" };
            }
        }

        fn main() {
            App { };
            Producer { };
        }
    "#;
    let (stdout, status) = build_and_run("two_adapters", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("A tag=alpha subject=beat"),
        "A should see beat: {:?}",
        stdout
    );
    assert!(
        stdout.contains("B tag=beta subject=pulse"),
        "B should see pulse: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("A tag=alpha subject=pulse"),
        "A should NOT see pulse: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("B tag=beta subject=beat"),
        "B should NOT see beat: {:?}",
        stdout
    );
}

#[test]
fn typecheck_rejects_non_locus_adapter_head() {
    // `type` head (not a locus) should be rejected with a focused
    // diag at typecheck.
    use aperio_syntax::parse_source;
    use aperio_types::{resolve::build_top_scope, symbol::Bundle, check::check_bundle};

    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        type NotALocus { x: Int; }

        main locus App {
            bindings {
                Beat: NotALocus { x: 1 };
            }
        }
        fn main() { App { }; }
    "#;
    let program = parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("test.ap".to_string(), &program);
    let bundle = Bundle { programs };
    let (scope, mut diags) = build_top_scope(&bundle);
    diags.extend(check_bundle(&bundle, &scope));
    let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("not a locus")),
        "expected `not a locus` diagnostic; got: {:?}",
        msgs
    );
}

#[test]
fn build_rejects_locus_missing_send_method() {
    // The stdlib's `__StdBusAdapter` interface isn't part of the
    // typecheck-visible bundle (stdlib merge happens inside
    // build_executable), so the structural check at typecheck is
    // a no-op. Codegen catches the missing `send` method with a
    // focused diag.
    let program = aperio_syntax::parse_source(r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        locus Empty { }

        main locus App {
            bindings {
                Beat: Empty { };
            }
        }
        fn main() { App { }; }
    "#)
    .expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_adapter_missing_send_{}",
        std::process::id()
    ));
    let err = build_executable(&program, &bin).expect_err("expected codegen err");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("has no `send` method"),
        "expected missing-send diag; got: {:?}",
        msg
    );
}
