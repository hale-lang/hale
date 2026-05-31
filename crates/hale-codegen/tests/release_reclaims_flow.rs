//! 2026-05-30 `release(c)` + run-completion reclaim for flows.
//!
//! Declaring `release(c: Child)` on a parent (the death-side bookend,
//! symmetric to `accept(c: Child)`) marks `Child` a *flow*: when a
//! Child's run() completes, the runtime fires `parent.release(owner,
//! child)` — after the child drains, before it dissolves, so the
//! parent reads the child's final state — then reclaims the child.
//! No explicit `terminate;` needed: a connection child whose run()
//! returns on EOF is reclaimed on that plain return.
//!
//! This is the ergonomic + observable half of the daemon-leak fix.
//! The test asserts: each of N flow children fires release exactly
//! once (reading its id) AND dissolves exactly once, release before
//! dissolve — so the reclaim ran per child via plain run-completion.

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn release_fires_and_reclaims_each_flow_child_on_run_completion() {
    const N: usize = 20;
    let src = format!(
        r#"
        type Trig {{ n: Int = 0; }}
        topic TrigT {{ payload: Trig; subject: "rf.trig"; }}

        locus Worker {{
            params {{ id: Int = 0; }}
            run() {{ let mut _x: Int = 0; }}   // flow: returns, reclaimed on completion
            dissolve() {{ println("DISSOLVE ", self.id); }}
        }}

        locus Manager {{
            params {{ spawned: Int = 0; }}
            accept(c: Worker) {{ }}
            release(c: Worker) {{ println("RELEASE ", c.id); }}
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
    bin.push("hale_test_release_reclaims_flow");
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
    let releases = stdout.matches("RELEASE ").count();
    let dissolves = stdout.matches("DISSOLVE ").count();
    assert!(
        stdout.contains("DONE"),
        "didn't reach DONE; stdout: {:?}",
        stdout
    );
    // Every flow child fired release once and dissolved once — reclaim
    // ran per child on plain run-completion (no `terminate`).
    assert_eq!(
        releases, N,
        "expected {} release() fires, got {}; stdout: {:?}",
        N, releases, stdout
    );
    assert_eq!(
        dissolves, N,
        "expected {} dissolves, got {}; stdout: {:?}",
        N, dissolves, stdout
    );
    // release(c) reads the child's final state (its id) and fires
    // before the child dissolves: for child 1, "RELEASE 1" precedes
    // "DISSOLVE 1".
    let rel1 = stdout.find("RELEASE 1\n");
    let dis1 = stdout.find("DISSOLVE 1\n");
    assert!(
        rel1.is_some() && dis1.is_some() && rel1 < dis1,
        "expected RELEASE 1 before DISSOLVE 1; stdout: {:?}",
        stdout
    );
}
