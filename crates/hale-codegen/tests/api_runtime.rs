//! GH #1417 (R2a): what the runtime of `api::serve` owes at the edges of a
//! process, as the process shows it.
//!
//! * The serving locus is the main locus and is never told to stop: it
//!   returns from `run()` with one call executing on a pool of its own and
//!   two queued. Its teardown is the shutdown: the queued two are refused
//!   `shutting_down` (they did not run) and the listener is released once.
//!   The executing call is another matter: the process's teardown shuts the
//!   receiver's pool down under it, so it can no longer be answered, and
//!   its caller is told nothing (it may have run; it is never reported as a
//!   refusal); the shutdown waits for it a bounded while and does not hang.
//!   (A call executing while a serving locus is replaced, whose receivers
//!   outlive the replacement, is answered: `tests/hale/api/teardown_test.hl`
//!   and `lifecycle_test.hl`.) The transport is the program's own, so what
//!   it was asked to deliver is its stdout.
//! * The lifecycle programs of `tests/hale/api/` (a connection lost after
//!   the handler started, `stop()` with queued and executing work, a
//!   receiver restarted, a receiver replaced, a handle dissolved), built
//!   under AddressSanitizer with the arena's chunk recycling off, run
//!   clean: a reply that outlives its handler's scratch, a request that
//!   outlives its connection and a pending record freed once are what the
//!   sanitizer would see go wrong.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const TEARDOWN: &str = r#"
type Req { n: Int; }
type Res { n: Int; }

locus Svc {
    params { started: Int = 0; }
    fn slow(r: Req) -> Res {
        self.started = self.started + 1;
        std::time::sleep(80ms);
        return Res { n: r.n };
    }
}

api S { rpc Svc::slow; }

// the program's own transport: it says what it is asked to do
locus Probe {
    params { exposure: Int = 0; }
    bus { publish "__api.rpc.ingress" of type std::api::RpcIngress; }
    fn attach(id: Int) -> String {
        self.exposure = id;
        return "probe";
    }
    fn frame(raw: Bytes, correlation: Int) -> std::api::Request {
        return std::api::Request {
            kind: 0,
            member: "Svc::slow",
            bytes: raw,
            correlation: correlation,
            peer: std::api::Principal { mode: "unix", name: "uid:1000", uid: 1000, gid: 1000 }
        };
    }
    fn reply(correlation: Int, request_id: Int, caller: std::api::Principal, outcome: std::api::Outcome) {
        match outcome {
            std::api::Outcome::Result(b) -> { println("answered ", correlation, " result"); },
            std::api::Outcome::Refusal(k, r, x) -> { println("answered ", correlation, " refusal:", k); },
            _ -> { println("answered ", correlation, " other"); },
        }
    }
    fn close_connection(correlation: Int) { }
    fn stop_listening() { println("released"); }
    fn push(correlation: Int) {
        "__api.rpc.ingress" <- std::api::RpcIngress { exposure: self.exposure, correlation: correlation, raw: std::bytes::from_string("{\"n\":1}") };
    }
}

main locus App {
    params {
        svc: Svc = Svc { };
        probe: Probe = Probe { };
    }
    placement { svc: cooperative(pool = work) where async_io; }
    run() {
        let h = api::serve(S, self.probe, as: "teardown", receivers: { Svc: self.svc }, bound: 3, on_full: refuse);
        std::time::sleep(20ms);
        self.probe.push(1);
        self.probe.push(2);
        self.probe.push(3);
        let mut i = 0;
        while i < 3000 && self.svc.started < 1 { std::time::sleep(1ms); i = i + 1; }
        println("executing ", self.svc.started);
    }
}

fn main() { App { }; }
"#;

#[test]
fn an_owners_teardown_refuses_the_queue_and_does_not_hang_on_a_call_its_pool_abandoned() {
    let bin = harness::unique_bin("api_runtime_teardown");
    build_opts::build_source(TEARDOWN, &bin, &build_opts::options()).expect("build");
    let started = std::time::Instant::now();
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "exit {:?}\n{stdout}", out.status);
    let lines: Vec<&str> = stdout.lines().collect();
    let at = |want: &str| lines.iter().position(|l| *l == want).unwrap_or_else(|| panic!("no `{want}` in:\n{stdout}"));
    assert_eq!(lines[0], "executing 1", "one call is executing when run() returns:\n{stdout}");
    let (two, three, released) = (at("answered 2 refusal:shutting_down"), at("answered 3 refusal:shutting_down"), at("released"));
    assert!(two < released && three < released, "the queued two are refused before the listener is released:\n{stdout}");
    assert_eq!(released, lines.len() - 1, "the listener is released last, once:\n{stdout}");
    // The receiver's pool is shut down with the process, under the call it
    // was running: that call can no longer be answered, and its caller is not
    // told it was refused (it may have run). The teardown does not wait for
    // it past `teardown_wait_ms`.
    assert!(!lines.iter().any(|l| l.starts_with("answered 1 ")), "the executing call is no refusal:\n{stdout}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "the teardown did not hang on the abandoned call");
}

fn run_asan(name: &str, src: &str) {
    let bin = harness::unique_bin(name);
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{name}: exit {:?}\nstdout: {stdout}\nstderr: {stderr}", out.status);
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{name}: {bad} in:\n{stderr}");
    }
}

#[test]
fn the_disconnect_program_runs_clean_under_asan() {
    run_asan("api_runtime_lifecycle", include_str!("../../../tests/hale/api/lifecycle_test.hl"));
}

#[test]
fn the_restart_program_runs_clean_under_asan() {
    run_asan("api_runtime_restart", include_str!("../../../tests/hale/api/restart_test.hl"));
}

#[test]
fn the_replacement_program_runs_clean_under_asan() {
    run_asan("api_runtime_replace", include_str!("../../../tests/hale/api/replace_test.hl"));
}

#[test]
fn the_dissolution_program_runs_clean_under_asan() {
    run_asan("api_runtime_teardown_asan", include_str!("../../../tests/hale/api/teardown_test.hl"));
}
