//! Crumb batch-3 item 3 — `return f();` in `fn main` must not
//! tear the runtime down before calling `f`.
//!
//! `lower_return`'s in_main path emitted the full teardown (pool
//! shutdown, dissolve flush, arena destroy, bus-queue destroy)
//! BEFORE lowering the return expression. So a main written as
//! `return cmd_run();` — where `cmd_run` instantiates the main
//! locus with a pool-placed subscriber child and publishes to it —
//! executed the whole program inside a torn-down world:
//! `lotus_coop_pool_lookup` returned NULL (the child registered
//! pool-less and its run() was forced onto the synchronous path,
//! Crumb's item 4), and the first bus enqueue wrote into the
//! freed queue (item 3's SIGSEGV — or, in a small heap, a silent
//! drop, which is why minimal repros looked green).
//!
//! Asserts on OUTPUT, not just exit status: the silent-drop
//! flavor of the bug loses the delivery without crashing.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-main-ret-call-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

const SRC: &str = r#"
    type Msg { n: Int; }

    locus Child {
        bus { subscribe "go" as on_go of type Msg; }
        fn on_go(m: Msg) {
            println("child got ", m.n);
        }
    }

    main locus App {
        params { c: Child = Child { }; }
        placement { c: cooperative(pool = w); }
        bus { publish "go" of type Msg; }
        run() {
            "go" <- Msg { n: 7 };
            std::time::sleep(200ms);
        }
    }

    fn wrapped() -> Int {
        App { };
        return 41;
    }

    fn main() {
        return wrapped() + 1;
    }
"#;

#[test]
fn main_return_call_runs_before_teardown() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = unique_path("wrapped");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    // The exit code proves the returned expression's VALUE made it
    // through teardown; the stdout line proves the pool-placed
    // subscriber was alive (registered with a real pool, queue
    // intact) while `wrapped()` ran.
    assert_eq!(
        out.status.code(),
        Some(42),
        "exit code lost through main-return teardown; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("child got 7"),
        "pool-placed subscriber never received the publish made \
         inside a `return f()` call from main (teardown-before-eval \
         regression). stdout: {:?} stderr: {:?}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
}
