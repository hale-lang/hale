//! 2026-05-30 per-child `terminate`: an accept'd child reclaimed
//! mid-program when it ends itself.
//!
//! `terminate;` inside a locus method sets the `__drain_requested`
//! latch and exits the method; when the run() completes with the
//! latch set, the run-wrapper runs the locus's drain → dissolve →
//! arena reclaim. This is the in-grain "I'm done, reclaim me" signal
//! that fixes the daemon leak (a parent that spawns one child per
//! connection, each ending independently): before this, an accept'd
//! child's arena was reclaimed only at parent dissolve — never, for a
//! daemon — so per-connection arenas accumulated (~8.7KB each,
//! measured 8MB→57MB over 6000). With terminate-reclaim, RSS stays
//! flat.
//!
//! This test asserts the *observable* half: each of N children that
//! `terminate`s runs its dissolve() exactly once — so the reclaim
//! fired N times, not zero (the leak) and not via double-teardown.

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn terminate_reclaims_each_accepted_child() {
    // Manager (async_io pool) accepts a Worker per trigger; each
    // Worker's run() immediately `terminate`s, and its dissolve()
    // prints a token. main fires N triggers then drains. We count
    // the tokens: exactly N means every child was reclaimed once.
    const N: usize = 25;
    let src = format!(
        r#"
        type Trig {{ n: Int = 0; }}
        topic TrigT {{ payload: Trig; subject: "tc.trig"; }}

        locus Worker {{
            params {{ id: Int = 0; }}
            run() {{ terminate; }}
            dissolve() {{ println("RECLAIMED"); }}
        }}

        locus Manager {{
            params {{ spawned: Int = 0; }}
            accept(c: Worker) {{ }}
            bus {{ subscribe TrigT as on_trig; }}
            fn on_trig(t: Trig) {{
                self.spawned = self.spawned + 1;
                Worker {{ id: self.spawned }};
            }}
        }}

        main locus App {{
            params {{ mgr: Manager = Manager {{ }}; }}
            placement {{ mgr: cooperative(pool = ws) where async_io; }}
            bus {{ publish TrigT; }}
            run() {{
                let mut i: Int = 0;
                while i < {N} {{
                    TrigT <- Trig {{ n: i }};
                    i = i + 1;
                }}
                std::time::sleep(400ms);
                println("DONE");
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_terminate_reclaims_child");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "non-zero exit {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let reclaimed = stdout.matches("RECLAIMED").count();
    assert!(
        stdout.contains("DONE"),
        "program didn't reach DONE; stdout: {:?}",
        stdout
    );
    // Each terminating child dissolved exactly once: not 0 (the leak —
    // reclaim never fired) and not 2N (double-teardown).
    assert_eq!(
        reclaimed, N,
        "expected {} child reclamations (one dissolve each), got {}; stdout: {:?}",
        N, reclaimed, stdout
    );
}
