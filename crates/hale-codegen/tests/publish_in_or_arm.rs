//! A publish inside an `or { … }` substitute arm, a `let`'s value or
//! an assignment's value reaches its subscriber (downstream handoff).
//!
//! The topic rewrite (`desugar_topics`, `hale-syntax/src/desugar.rs`)
//! turns `Alerts <- …` into a send on the wire subject. Until 2026-11-06
//! its walk stopped at the statement forms (a loop body, an `if` /
//! `match` arm, a nested block), so a send nested in an expression — the
//! `or { … }` arm of `let v = risky(1) or { Alerts <- Alert { … }; 0 };`
//! — kept the bare topic name, which the check accepted and lowering
//! refused as "unknown identifier `Alerts`", where the same publish in an
//! `if` built. The walk is now `each_stmt_mut`'s, over every nested
//! block, and so is the intra-locus rewrite's.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_test_publish_in_or_arm_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const PRELUDE: &str = r#"
    type Alert { msg: String = ""; }
    topic Alerts { payload: Alert; subject: "alerts"; }
    type E { kind: String = ""; }
    fn risky(n: Int) -> Int fallible(E) { if n > 0 { fail E { kind: "pos" }; } return n; }
    locus Watch {
        bus { subscribe Alerts as on_alert; }
        fn on_alert(a: Alert) { println("alert: ", a.msg); }
    }
"#;

/// The or-arm of a `let`: the error's payload is in scope there, and
/// the publish carries it.
#[test]
fn publish_in_the_or_arm_of_a_let_is_delivered() {
    let src = format!(
        "{PRELUDE}
        locus Worker {{
            bus {{ publish Alerts; }}
            run() {{
                let v = risky(1) or {{ Alerts <- Alert {{ msg: err.kind }}; 0 }};
                println(\"v=\", v);
                std::time::sleep(50ms);
            }}
        }}
        fn main() {{ let w = Watch {{ }}; Worker {{ }}; }}
        "
    );
    let out = run("let", &src);
    assert!(out.contains("alert: pos"), "got: {out:?}");
    assert!(out.contains("v=0"), "got: {out:?}");
}

/// The or-arm of an assignment's value, and a call's argument.
#[test]
fn publish_in_the_or_arm_of_an_assignment_and_an_argument_is_delivered() {
    let src = format!(
        "{PRELUDE}
        fn twice(n: Int) -> Int {{ return n * 2; }}
        locus Worker {{
            bus {{ publish Alerts; }}
            run() {{
                let mut v = 0;
                v = risky(1) or {{ Alerts <- Alert {{ msg: \"assigned\" }}; 3 }};
                let d = twice(risky(2) or {{ Alerts <- Alert {{ msg: \"argued\" }}; 4 }});
                println(\"v=\", v, \" d=\", d);
                std::time::sleep(50ms);
            }}
        }}
        fn main() {{ let w = Watch {{ }}; Worker {{ }}; }}
        "
    );
    let out = run("assign", &src);
    assert!(out.contains("alert: assigned"), "got: {out:?}");
    assert!(out.contains("alert: argued"), "got: {out:?}");
    assert!(out.contains("v=3 d=8"), "got: {out:?}");
}

/// The intra-locus rewrite walks the same way: a publisher that holds
/// its one subscriber in a field publishes from an or-arm as a direct
/// call, and the handler runs.
#[test]
fn publish_in_an_or_arm_to_a_held_subscriber_is_delivered() {
    let src = r#"
        type Alert { msg: String = ""; }
        topic Alerts { payload: Alert; subject: "alerts"; }
        type E { kind: String = ""; }
        fn risky(n: Int) -> Int fallible(E) { if n > 0 { fail E { kind: "pos" }; } return n; }
        locus Watch {
            bus { subscribe Alerts as on_alert; }
            fn on_alert(a: Alert) { println("alert: ", a.msg); }
        }
        locus Worker {
            params { w: Watch = Watch { }; }
            bus { publish Alerts; }
            run() {
                let v = risky(1) or { Alerts <- Alert { msg: err.kind }; 0 };
                println("v=", v);
                std::time::sleep(50ms);
            }
        }
        fn main() { Worker { }; }
    "#;
    let out = run("held", src);
    assert!(out.contains("alert: pos"), "got: {out:?}");
    assert!(out.contains("v=0"), "got: {out:?}");
}
