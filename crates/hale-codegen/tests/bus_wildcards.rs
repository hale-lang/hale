//! m94: bus subject wildcards.
//!
//! End-to-end test that a subscription with a trailing `**`
//! pattern receives publishes on every matching concrete
//! subject. Compiles a small Hale program that publishes on
//! three subjects (`log.app`, `log.app.db`, `other.thing`) and
//! has one subscriber on `log.**`; verifies the subscriber
//! prints two events (the `log.*` ones) and not the third.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_buswild_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn wildcard_subscriber_receives_two_of_three_publishes() {
    let src = r#"
        type LogEvent {
            level: Int;
            msg: String;
        }
        type OtherEvent {
            note: String;
        }

        locus LogSinkL {
            bus {
                subscribe "log.**" as on_log of type LogEvent;
            }
            fn on_log(e: LogEvent) {
                println("LOG ", e.level, " ", e.msg);
            }
        }

        locus AppL {
            bus {
                publish "log.app" of type LogEvent;
                publish "log.app.db" of type LogEvent;
                publish "other.thing" of type OtherEvent;
            }
            birth() {
                "log.app" <- LogEvent { level: 1, msg: "starting" };
                "log.app.db" <- LogEvent { level: 1, msg: "connected" };
                "other.thing" <- OtherEvent { note: "ignore me" };
            }
        }

        fn main() {
            LogSinkL { };
            AppL { };
        }
    "#;
    let bin = build_hale("two_of_three", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("LOG 1 starting"),
        "expected log.app delivery; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("LOG 1 connected"),
        "expected log.app.db delivery (cascade); got: {:?}",
        stdout
    );
    // The wildcard pattern should NOT match other.thing — that
    // event has a different type and would type-check fail if
    // delivered to the LogEvent subscriber. Its absence in stdout
    // is the assertion.
    assert!(
        !stdout.contains("ignore me"),
        "wildcard should not match other.thing; got: {:?}",
        stdout
    );
}

#[test]
fn wildcard_matches_root_and_descendants() {
    // m94 semantics: `log.app.**` matches the root subject
    // `log.app` AND any descendant (`log.app.db`,
    // `log.app.db.query`, ...). The cascade-friendly form so
    // a sub-tree subscriber catches the whole branch.
    let src = r#"
        type LogEvent {
            level: Int;
            msg: String;
        }

        locus LogSinkL {
            bus {
                subscribe "log.app.**" as on_log of type LogEvent;
            }
            fn on_log(e: LogEvent) {
                println("CAUGHT ", e.msg);
            }
        }

        locus AppL {
            bus {
                publish "log.app" of type LogEvent;
                publish "log.app.db" of type LogEvent;
                publish "other.thing" of type LogEvent;
            }
            birth() {
                "log.app" <- LogEvent { level: 1, msg: "root" };
                "log.app.db" <- LogEvent { level: 1, msg: "child" };
                "other.thing" <- LogEvent { level: 1, msg: "peer" };
            }
        }

        fn main() {
            LogSinkL { };
            AppL { };
        }
    "#;
    let bin = build_hale("subtree_root", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("CAUGHT root"),
        "log.app.** should match the root log.app; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("CAUGHT child"),
        "log.app.** should match descendants; got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("CAUGHT peer"),
        "log.app.** must not match peer trees; got: {:?}",
        stdout
    );
}

#[test]
fn exact_subscribers_unaffected_by_wildcard_path() {
    // A subject without ** should still go through the fast
    // exact-match path. Both subscribers (one exact, one
    // wildcard) on the same publish subject should fire.
    let src = r#"
        type LogEvent {
            level: Int;
            msg: String;
        }

        locus ExactSinkL {
            bus {
                subscribe "log.app" as on_app of type LogEvent;
            }
            fn on_app(e: LogEvent) {
                println("EXACT ", e.msg);
            }
        }

        locus WildSinkL {
            bus {
                subscribe "log.**" as on_any of type LogEvent;
            }
            fn on_any(e: LogEvent) {
                println("WILD ", e.msg);
            }
        }

        locus AppL {
            bus {
                publish "log.app" of type LogEvent;
            }
            birth() {
                "log.app" <- LogEvent { level: 1, msg: "hi" };
            }
        }

        fn main() {
            ExactSinkL { };
            WildSinkL { };
            AppL { };
        }
    "#;
    let bin = build_hale("both_fire", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("EXACT hi"), "got: {:?}", stdout);
    assert!(stdout.contains("WILD hi"), "got: {:?}", stdout);
}
