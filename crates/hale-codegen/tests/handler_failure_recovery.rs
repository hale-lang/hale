//! GH #2028: a child that fails from a bus handler is recovered as one
//! that fails from `run()` is (spec/semantics.md § Recovery primitives,
//! § "A failed child its owner holds is kept (GH #1069)").
//!
//! The parent's publish to its one field child's subscription was
//! rewritten to a direct method call (the intra-locus rewrite), which
//! ran the handler outside its handler wrapper: after the supervisor's
//! `restart(c)` neither `birth()` nor `run()` ran again, and after an
//! absorb the child kept handling the next message. A field child a
//! failure can come from now takes the bus, and its handler wrapper
//! carries out the decision: a restart re-runs `birth()` and `run()`
//! (and puts back the subscriptions the failure's stop set aside), an
//! absorb stops the child. While the child's `run()` is running, its
//! run wrapper decides once `run()` returns, as before.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn program(recovery: &str, placement: &str) -> String {
    format!(
        r#"
type Job {{ n: Int; }}

topic Jobs {{
    payload: Job;
    subject: "jobs";
}}

locus Worker {{
    params {{ births: Int = 0; runs: Int = 0; last: Int = 0; }}
    bus {{ subscribe Jobs as on_job; }}
    closure bad_job {{ captures: last; epoch inline; }}
    birth() {{
        self.births = self.births + 1;
        println("birth ", self.births);
    }}
    run() {{
        self.runs = self.runs + 1;
        println("run ", self.runs);
    }}
    fn on_job(j: Job) {{
        println("job ", j.n);
        self.last = j.n;
        if j.n == 2 {{
            violate bad_job;
        }}
    }}
}}

main locus App {{
    params {{ w: Worker = Worker {{ }}; }}
    {placement}
    bus {{ publish Jobs; }}
    on_failure(c: Worker, err: ClosureViolation) {{
        println("failed ", err.closure);
        {recovery}
    }}
    run() {{
        Jobs <- Job {{ n: 1 }};
        yield;
        std::time::sleep(20ms);
        Jobs <- Job {{ n: 2 }};
        yield;
        std::time::sleep(20ms);
        Jobs <- Job {{ n: 3 }};
        yield;
        std::time::sleep(20ms);
        println("births ", self.w.births, " runs ", self.w.runs);
    }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

fn run(tag: &str, src: &str) -> Vec<String> {
    let bin = harness::unique_bin(&format!("hfr_{tag}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{:?}\n{stdout}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    stdout.lines().map(str::to_string).collect()
}

#[test]
fn a_restart_after_a_handler_failure_reruns_birth_and_run() {
    let out = run("restart", &program("restart(c);", ""));
    assert_eq!(
        out,
        ["birth 1", "run 1", "job 1", "job 2", "failed bad_job", "birth 2", "run 2", "job 3", "births 2 runs 2"],
        "{out:?}"
    );
}

#[test]
fn an_absorbed_handler_failure_stops_the_child() {
    let out = run("absorb", &program("", ""));
    assert_eq!(out, ["birth 1", "run 1", "job 1", "job 2", "failed bad_job", "births 1 runs 1"], "{out:?}");
}

#[test]
fn the_publish_to_a_field_child_that_can_fail_takes_the_bus() {
    // The rewrite still serves a field child no failure can come from;
    // the one that can fail keeps its bus registration (and its wrapper).
    let src = program("restart(c);", "");
    let ir = {
        let bin = harness::unique_bin("hfr_ir");
        let ll = harness::unique_bin("hfr_ir.ll");
        let opts = hale_codegen::BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
        build_opts::build_source(&src, &bin, &opts).expect("build");
        let text = std::fs::read_to_string(&ll).expect("the dumped IR");
        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_file(&ll);
        text
    };
    let app_run = ir.split("define void @App.run(").nth(1).expect("App.run in the IR");
    let app_run = &app_run[..app_run.find("\n}\n").unwrap_or(app_run.len())];
    assert!(!app_run.contains("@Worker.on_job("), "App.run calls the handler directly:\n{app_run}");
}

/// Review of #2052: a locus's publish to its own subscription stays a
/// direct call (it reaches this instance alone), and no handler wrapper
/// surrounds it when an ordinary method its owner calls makes it, after
/// the child's `run()` returned. The decision is carried out after the
/// direct call instead.
fn self_publish(recovery: &str) -> String {
    format!(
        r#"
type Job {{ n: Int; }}

topic Jobs {{
    payload: Job;
    subject: "jobs";
}}

locus Worker {{
    params {{ births: Int = 0; runs: Int = 0; last: Int = 0; }}
    bus {{ subscribe Jobs as on_job; publish Jobs; }}
    closure bad_job {{ captures: last; epoch inline; }}
    birth() {{
        self.births = self.births + 1;
        println("birth ", self.births);
    }}
    run() {{
        self.runs = self.runs + 1;
        println("run ", self.runs);
    }}
    fn send(n: Int) {{
        Jobs <- make_job(n);
    }}
    fn on_job(j: Job) {{
        println("job ", j.n);
        self.last = j.n;
        if j.n == 2 {{
            violate bad_job;
        }}
    }}
}}

main locus App {{
    params {{ w: Worker = Worker {{ }}; }}
    on_failure(c: Worker, err: ClosureViolation) {{
        println("failed ", err.closure);
        {recovery}
    }}
    run() {{
        self.w.send(1);
        self.w.send(2);
        self.w.send(3);
        println("births ", self.w.births, " runs ", self.w.runs);
    }}
}}

fn make_job(n: Int) -> Job {{
    println("make ", n);
    return Job {{ n: n }};
}}

fn main() {{ App {{ }}; }}
"#
    )
}

#[test]
fn a_restart_after_a_self_publishs_handler_failure_reruns_birth_and_run() {
    let out = run("self_restart", &self_publish("restart(c);"));
    assert_eq!(
        out,
        [
            "birth 1", "run 1", "make 1", "job 1", "make 2", "job 2", "failed bad_job", "birth 2", "run 2", "make 3", "job 3",
            "births 2 runs 2"
        ],
        "{out:?}"
    );
}

#[test]
fn an_absorbed_self_publishs_handler_failure_stops_the_child() {
    // The payload of the publish the stopped child makes is still built.
    let out = run("self_absorb", &self_publish(""));
    assert_eq!(
        out,
        ["birth 1", "run 1", "make 1", "job 1", "make 2", "job 2", "failed bad_job", "make 3", "births 1 runs 1"],
        "{out:?}"
    );
}
