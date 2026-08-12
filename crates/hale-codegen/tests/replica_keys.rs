//! Replica keys (2026-08-12) — `where key == replica` bridges the
//! placement scale axis and the delivery axis: each instance of a
//! `pinned(..., replicas = K)` fan-out registers its 0-based
//! replica index as its subscription key, so K replicas shard an
//! Int-keyed topic with ONE subscribe line and N spelled once (in
//! the placement entry). Placement stays semantics-free: the
//! filter is written on the subscription; the placement only
//! decides how many indices exist.
//!
//! This is the webserver fan-out shape that motivated it: a
//! listener publishing connections `keyed_by shard`, workers
//! subscribing `where key == replica`.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn run_src(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn replicas_shard_a_keyed_topic_one_index_each() {
    // Shards 0/1/2 get 1/2/3 messages. Broadcast would print
    // 6,6,6; sharded delivery prints 1,2,3.
    let src = r#"
        type Job { shard: Int; v: Int; }
        topic Work { payload: Job; keyed_by shard; }

        locus Worker {
            params { seen: Int = 0; }
            bus { subscribe Work as on_job where key == replica; }
            fn on_job(j: Job) { self.seen = self.seen + 1; }
            run() {
                std::time::sleep(600ms);
                println("seen=", to_string(self.seen));
            }
        }

        locus Pub {
            bus { publish Work; }
            run() {
                std::time::sleep(150ms);
                Work <- Job { shard: 0, v: 1 };
                Work <- Job { shard: 1, v: 1 };
                Work <- Job { shard: 1, v: 1 };
                Work <- Job { shard: 2, v: 1 };
                Work <- Job { shard: 2, v: 1 };
                Work <- Job { shard: 2, v: 1 };
            }
        }

        main locus App {
            params { w: Worker = Worker { }; p: Pub = Pub { }; }
            placement { w: pinned(replicas = 3); }
            run() { std::time::sleep(900ms); }
        }

        fn main() { App { }; }
    "#;
    let (out, st) = run_src("replica_shard", src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    let mut counts: Vec<&str> = out
        .lines()
        .filter_map(|l| l.strip_prefix("seen="))
        .collect();
    counts.sort();
    assert_eq!(
        counts,
        vec!["1", "2", "3"],
        "each replica receives exactly its shard (broadcast would \
         be 6,6,6): {}",
        out
    );
}

#[test]
fn a_non_replicated_instance_is_replica_zero() {
    // The same subscribe line works without a replicas entry: the
    // single instance is replica 0 and receives shard-0 traffic
    // only.
    let src = r#"
        type Job { shard: Int; }
        topic Work { payload: Job; keyed_by shard; }

        locus Worker {
            params { seen: Int = 0; }
            bus { subscribe Work as on_job where key == replica; }
            fn on_job(j: Job) { self.seen = self.seen + 1; }
            run() {
                std::time::sleep(400ms);
                println("seen=", to_string(self.seen));
            }
        }

        locus Pub {
            bus { publish Work; }
            run() {
                std::time::sleep(100ms);
                Work <- Job { shard: 0 };
                Work <- Job { shard: 1 };
                Work <- Job { shard: 2 };
            }
        }

        main locus App {
            params { w: Worker = Worker { }; p: Pub = Pub { }; }
            placement { w: pinned; }
            run() { std::time::sleep(700ms); }
        }

        fn main() { App { }; }
    "#;
    let (out, st) = run_src("replica_zero", src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(
        out.contains("seen=1"),
        "a lone instance is replica 0 and sees only shard 0: {}",
        out
    );
}
