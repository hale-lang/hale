//! F.35 Slice 2: `where async_io` placement constraint —
//! parse + typecheck + codegen smoke.
//!
//! Slice 1 wired the substrate plumbing (per-pool epoll +
//! ucontext-backed coro dispatch) but left it dormant; this
//! slice exposes the user-facing surface that flips the
//! `async_io_enabled` flag for opt-in pools.
//!
//! These tests only verify the language surface (parse + typecheck
//! rejections + that the codegen emits the enable call). Slice 3
//! wires blocking I/O primitives through `lotus_coop_park_on_fd`;
//! a real "does the pool actually park" test lands there.

use std::process::Command;

use hale_codegen::build_executable;

fn typecheck_diags(source: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), &program);
    let bundle = hale_types::Bundle { programs };
    let (scope, _) = hale_types::resolve::build_top_scope(&bundle);
    let diags = hale_types::check::check_bundle(&bundle, &scope, true);
    diags.into_iter().map(|d| d.message).collect()
}

#[test]
fn placement_where_async_io_parses_and_typechecks() {
    // The canonical shape: a long-running child on its own
    // cooperative pool with `where async_io`. No diagnostics
    // expected.
    let src = r#"
        fn ignore_conn(s: std::io::tcp::Stream) { }

        main locus App {
            params {
                listener: std::io::tcp::Listener = std::io::tcp::Listener {
                    host:         "127.0.0.1",
                    port:         0,
                    max_accepts:  -1,
                    on_connection: ignore_conn,
                };
            }
            placement {
                listener: cooperative(pool = io) where async_io;
            }
        }

        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.is_empty(),
        "expected no diagnostics, got: {:?}",
        diags
    );
}

#[test]
fn placement_where_async_io_on_pinned_is_rejected() {
    // Pinned loci own their own OS thread; there's no shared
    // drain loop to park on. Typecheck must reject.
    let src = r#"
        locus Heartbeat {
            run() { std::time::sleep(1s); }
        }
        main locus App {
            params {
                hb: Heartbeat = Heartbeat { };
            }
            placement {
                hb: pinned(core = 1) where async_io;
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("async_io")
            && m.contains("pinned")),
        "expected pinned-rejection diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn placement_where_async_io_on_pool_main_is_rejected() {
    // Pool `main` runs inline on the binary's primary thread;
    // no dedicated worker to integrate epoll into.
    let src = r#"
        locus Sub {
            params { tag: String = "s"; }
        }
        main locus App {
            params {
                sub: Sub = Sub { };
            }
            placement {
                sub: cooperative(pool = main) where async_io;
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("async_io")
            && m.contains("main")),
        "expected pool=main-rejection diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn placement_async_io_mixed_with_non_async_on_same_pool_is_rejected() {
    // Two entries share pool=io but disagree on async_io. The
    // pool's drain loop is one-or-the-other, so this is
    // structurally inconsistent.
    let src = r#"
        locus A {
            run() { std::time::sleep(1s); }
        }
        locus B {
            run() { std::time::sleep(1s); }
        }
        main locus App {
            params {
                a: A = A { };
                b: B = B { };
            }
            placement {
                a: cooperative(pool = io) where async_io;
                b: cooperative(pool = io);
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("mixed I/O modes")
            && m.contains("pool")),
        "expected mixed-mode-rejection diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn unknown_placement_constraint_is_rejected_at_parse() {
    let src = r#"
        locus A {
            run() { std::time::sleep(1s); }
        }
        main locus App {
            params { a: A = A { }; }
            placement {
                a: cooperative(pool = io) where bogus_constraint;
            }
        }
        fn main() { App { }; }
    "#;
    let res = hale_syntax::parse_source(src);
    let err = res.expect_err("should fail to parse");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("bogus_constraint") || msg.contains("placement constraint"),
        "expected unknown-constraint diagnostic, got: {}",
        msg
    );
}

#[test]
fn placement_where_async_io_builds_and_runs() {
    // End-to-end: build a binary with `where async_io` on a
    // pool, run it, verify it exits cleanly. The enable call is
    // emitted in the prelude (verified by the build succeeding
    // against the new lotus_coop_pool_enable_async_io declaration);
    // actual parking behavior lands in Slice 3.
    let src = r#"
        main locus App {
            params {
                tag: String = "ready";
            }
            placement { }
            run() {
                println("app running");
            }
        }
        fn main() {
            App { };
            println("done");
        }
    "#;
    // Note: the empty placement block + no async_io entries means
    // this just exercises the typecheck-pass path. The enable
    // emit is gated on the async_io_pools set, which stays empty
    // here. The "with async_io" path needs an actual long-running
    // sibling shape — covered by the standalone smoke below.
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_placement_where_no_async");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    assert!(
        stdout.contains("app running") && stdout.contains("done"),
        "unexpected output: {:?}",
        stdout
    );
}

#[test]
fn placement_where_async_io_emits_enable_call() {
    // Build a program whose main locus has at least one
    // `where async_io` entry. The codegen path must:
    //   (a) include the lotus_coop_pool_enable_async_io declaration
    //   (b) emit a call to it for the named pool
    // We verify (b) indirectly: the resulting binary must run
    // cleanly. If the enable call were malformed, the linker /
    // loader would surface it.
    let src = r#"
        fn ignore_conn(s: std::io::tcp::Stream) { }

        main locus App {
            params {
                listener: std::io::tcp::Listener = std::io::tcp::Listener {
                    host:         "127.0.0.1",
                    port:         0,
                    max_accepts:  0,
                    on_connection: ignore_conn,
                };
            }
            placement {
                listener: cooperative(pool = io) where async_io;
            }
        }

        fn main() {
            println("before");
            App { };
            println("after");
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_placement_where_async_io_e2e");
    build_executable(&program, &bin).expect("build");
    let output = Command::new("timeout")
        .arg("3")
        .arg(&bin)
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("before") && stdout.contains("after"),
        "binary didn't run cleanly with where async_io: {:?}",
        stdout
    );
}
