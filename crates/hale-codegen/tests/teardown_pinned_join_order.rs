//! Crumb batch-4 item 1 — the GH #253 teardown delivery contract
//! must hold on the DEFERRED main-locus path too.
//!
//! Shape: main publishes jobs to two pinned workers, which publish
//! results to a cooperative sink field; run() returns with work in
//! flight. The contract ("a dissolving parent joins its own pinned
//! children BEFORE cascading its field children's drain/dissolve")
//! made this correct on the eager path — but adding a single bus
//! SUBSCRIPTION to the main locus made it long-lived, routing it
//! down the deferred-dissolve path, where its pinned children sat
//! BEFORE it in the flush frame: the reverse-order flush tore the
//! parent (and the sink) down first, then joined the workers, whose
//! final publishes drained into a dead subscriber. 0 of 8 delivered,
//! exit 0, silent.
//!
//! The fix re-orders a deferred parent's own pinned entries after
//! its own frame entry (mirroring the eager path's steal), so the
//! flush joins + drains them while every subscriber field is alive.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_count(tag: &str, src: &str) -> usize {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_teardown_join_{}_{}", tag, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("sink got "))
        .count()
}

fn fanout_src(main_subscribes: bool) -> String {
    let (sub_line, handler) = if main_subscribes {
        (
            r#"subscribe "res" as on_res of type Res;"#,
            "fn on_res(r: Res) { }",
        )
    } else {
        ("", "")
    };
    format!(
        r#"
        type Job {{ n: Int; }}
        type Res {{ n: Int; }}

        locus Worker {{
            params {{ id: Int = 0; }}
            bus {{
                subscribe "jobs" as on_job of type Job;
                publish "res" of type Res;
            }}
            fn on_job(j: Job) {{
                if j.n % 2 != self.id {{ return; }}
                "res" <- Res {{ n: j.n }};
            }}
        }}

        locus Sink {{
            params {{ got: Int = 0; }}
            bus {{ subscribe "res" as on_res of type Res; }}
            fn on_res(r: Res) {{
                self.got = self.got + 1;
                println("sink got ", r.n);
            }}
        }}

        main locus App {{
            params {{
                sink: Sink = Sink {{ }};
                w0: Worker = Worker {{ id: 0 }};
                w1: Worker = Worker {{ id: 1 }};
            }}
            placement {{
                w0: pinned(core = 0);
                w1: pinned(core = 1);
            }}
            bus {{
                publish "jobs" of type Job;
                {sub_line}
            }}
            {handler}
            run() {{
                let mut i = 0;
                while i < 8 {{
                    "jobs" <- Job {{ n: i }};
                    i = i + 1;
                }}
                // returns with work in flight — the delivery
                // contract is what makes this correct.
            }}
        }}

        fn main() {{
            App {{ }};
            return 0;
        }}
    "#
    )
}

/// The eager path (no subscription on main) — the GH #253 baseline.
#[test]
fn eager_main_delivers_in_flight_results() {
    assert_eq!(
        build_and_count("eager", &fanout_src(false)),
        8,
        "eager-path teardown lost in-flight pinned results"
    );
}

/// The deferred path (a subscription makes main long-lived) — the
/// Crumb batch-4 regression: was 0 of 8.
#[test]
fn deferred_main_with_subscription_delivers_in_flight_results() {
    assert_eq!(
        build_and_count("deferred", &fanout_src(true)),
        8,
        "a bus subscription on main inverted the teardown join order \
         (parent cascade before pinned join) — in-flight results dropped"
    );
}
