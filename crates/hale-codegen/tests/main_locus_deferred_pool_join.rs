//! A main locus built in a fn other than `main` joins the cooperative
//! pools before its deferred teardown frees its fields (GH #1148).
//!
//! A main locus that subscribes to a topic is deferred to the exit of
//! the fn that instantiates it. When that fn is `main`, main's exit
//! joins the pools before the flush; the eager (non-deferred) dissolve
//! of a main locus does the same. But `fn main() { start(..) }` with
//! the locus built in `start` reached the flush of `start` with no
//! join: the cascade freed the arena of a field placed on
//! `cooperative(pool = ..)` while that field's `run()` was still on the
//! pool's worker, and the worker touched the freed parent arena as the
//! run unwound (heap-use-after-free under ASan; a NULL write in
//! `lotus_arena_init_struct` or a SIGSEGV without it). A downstream
//! handoff's host process died on every stop that way once a metrics
//! endpoint joined its main locus on a pool of its own.
//!
//! The worker's run is still sleeping (and allocating a heap string on
//! every pass, so its per-call scratch hangs off the field's arena) when
//! `App.run` returns: the teardown must wait for it. The run is under
//! ASan with the chunk pool off, so a freed arena chunk is visible
//! (GH #816).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
type Ping { n: Int; }
topic Pinged { payload: Ping; subject: "pinged"; }

locus Worker {
    params { name: String = ""; passes: Int = 0; }
    run() {
        let mut i = 0;
        while i < 15 {
            self.name = std::str::upper("pass-" + to_string(i));
            std::time::sleep(20ms);
            i = i + 1;
        }
        self.passes = i;
    }
}

main locus App {
    params { worker: Worker = Worker { }; }
    placement { worker: cooperative(pool = side); }
    bus { subscribe Pinged as on_ping; publish Pinged; }
    fn on_ping(p: Ping) { println("ping"); }
    run() { println("app ran"); }
}

fn start(label: String) {
    App { };
    println(label);
}

fn main() {
    start("started");
    println("main done");
}
"#;

#[test]
fn a_main_locus_built_outside_main_joins_its_pools_before_teardown() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin("hale_main_locus_deferred_pool_join");
    harness::build_asan(&program, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("AddressSanitizer"),
        "the teardown freed a pooled field's arena under its running \
         worker:\n{stderr}"
    );
    assert!(
        out.status.success(),
        "exit {:?}; stdout={stdout:?} stderr={stderr:?}",
        out.status
    );
    assert_eq!(stdout, "app ran\nstarted\nmain done\n", "stderr={stderr:?}");
}
