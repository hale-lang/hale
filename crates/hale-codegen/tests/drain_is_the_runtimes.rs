//! GH #1858: the drain is the runtime's (spec/semantics.md § "Drain
//! cascade (whole-process)").
//!
//! On SIGTERM the runtime ends every `run()` loop of a locus whose `run()`
//! never reads `draining` at the loop's back-edge — between two
//! iterations, never inside one — and wakes every blocking wait with its
//! closed or empty outcome, so `run()` returns, teardown runs the hooks
//! and the process exits 0. A locus whose `run()` reads `draining` stops
//! by its own code (the check reports a loop it cannot see stop), and the
//! grace stays the backstop for one that does not.
//!
//! Each runtime row of the RFC's table is held here on every placement a
//! `run()` takes: the main locus's own (main thread), a classic
//! cooperative pool, an `async_io` pool and a pinned thread. The stop
//! times are printed (`--nocapture`) for the record.

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

struct Ran {
    status: std::process::ExitStatus,
    stopped_in: Duration,
    stdout: String,
    stderr: String,
}

/// Build `src`, wait for its stdout line `up`, let it settle into what it
/// waits on for `settle`, send SIGTERM and collect how it ended and how
/// long after the signal it took.
fn term_after_up(tag: &str, src: &str, settle: Duration, env: &[(&str, &str)]) -> Ran {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut cmd = Command::new(&bin);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn");
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        let mut seen = String::new();
        let mut line = String::new();
        loop {
            line.clear();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    seen.push_str(&line);
                    if line.trim() == "up" {
                        let _ = tx.send(());
                        break;
                    }
                }
            }
        }
        let mut rest = String::new();
        let _ = r.read_to_string(&mut rest);
        seen + &rest
    });
    if rx.recv_timeout(Duration::from_secs(30)).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("{tag}: the program never printed `up`:\n{}", reader.join().unwrap_or_default());
    }
    std::thread::sleep(settle);
    let sent = Instant::now();
    let killed = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().expect("kill");
    assert!(killed.success(), "sent SIGTERM");
    // Bounded: a program the drain does not end is killed after 20 s.
    let deadline = sent + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{tag}: still running 20 s after SIGTERM:\n{}", reader.join().unwrap_or_default());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stopped_in = sent.elapsed();
    let stdout = reader.join().expect("reader");
    let stderr = err_reader.join().expect("stderr reader");
    let _ = std::fs::remove_file(&bin);
    eprintln!("{tag}: stopped {stopped_in:?} after SIGTERM, {status:?}");
    Ran { status, stopped_in, stdout, stderr }
}

/// The four placements a child's `run()` takes, as the main locus's
/// `placement` block writes them (`None`: the main locus runs the body
/// itself, on main).
const PLACEMENTS: [(&str, Option<&str>); 4] = [
    ("main", None),
    ("classic", Some("cooperative(pool = io)")),
    ("async_io", Some("cooperative(pool = io) where async_io")),
    ("pinned", Some("pinned")),
];

/// A program whose `run()` body is `body` (with `drain()` printing
/// `hook`): the main locus's own on main, or a child `W`'s placed as
/// `placement` under a main locus whose `run()` waits for the drain (a
/// main with no `run()` would raise its tree's drain at once, which a
/// child that reads `draining` sees before any signal).
fn program(placement: Option<&str>, params: &str, body: &str) -> String {
    match placement {
        None => format!(
            "main locus App {{\n    params {{ {params} }}\n    run() {{\n{body}\n    }}\n    drain() {{ println(\"hook\"); }}\n}}\nfn main() {{ App {{ }}; println(\"after\"); }}\n"
        ),
        Some(p) => format!(
            "locus W {{\n    params {{ {params} }}\n    run() {{\n{body}\n    }}\n    drain() {{ println(\"hook\"); }}\n}}\n\
             main locus App {{\n    params {{ w: W = W {{ }}; }}\n    placement {{ w: {p}; }}\n    run() {{ while !self.draining {{ std::time::sleep(10ms); }} }}\n}}\n\
             fn main() {{ App {{ }}; println(\"after\"); }}\n"
        ),
    }
}

fn assert_clean(tag: &str, ran: &Ran) {
    assert_eq!(
        ran.status.code(),
        Some(0),
        "{tag}: exit 0 after the hooks, not the signal ({:?}, {:?}):\nstdout:\n{}\nstderr:\n{}",
        ran.status,
        ran.stopped_in,
        ran.stdout,
        ran.stderr
    );
    assert!(ran.stopped_in < Duration::from_secs(1), "{tag}: stopped in {:?}:\n{}", ran.stopped_in, ran.stdout);
    let lines: Vec<&str> = ran.stdout.lines().collect();
    assert!(lines.contains(&"hook"), "{tag}: the drain() hook ran:\n{}", ran.stdout);
    assert_eq!(lines.last(), Some(&"after"), "{tag}: the program ended the ordinary way:\n{}", ran.stdout);
}

/// A `run()` that never reads `draining` loops for ever, one iteration a
/// `begin`, a 300 ms sleep and an `end`. SIGTERM lands inside the sleep:
/// the sleep (in progress) is cut, the iteration finishes (`end` follows
/// the last `begin`), the loop ends at its back-edge, the hook runs and
/// the process exits 0, well under a second.
#[test]
fn a_run_that_never_reads_draining_stops_between_iterations() {
    for (name, placement) in PLACEMENTS {
        let body = "        println(\"up\");\n        while true {\n            println(\"begin \" + to_string(self.n));\n            std::time::sleep(300ms);\n            println(\"end \" + to_string(self.n));\n            self.n = self.n + 1;\n        }";
        let tag = format!("drain_runtime_iter_{name}");
        let ran = term_after_up(&tag, &program(placement, "n: Int = 0;", body), Duration::from_millis(150), &[]);
        assert_clean(&tag, &ran);
        let begins = ran.stdout.lines().filter(|l| l.starts_with("begin ")).count();
        let ends = ran.stdout.lines().filter(|l| l.starts_with("end ")).count();
        assert_eq!(begins, ends, "{tag}: every iteration begun was finished:\n{}", ran.stdout);
        assert!(begins >= 1, "{tag}:\n{}", ran.stdout);
    }
}

/// A pure spin, `while true` with no wait in it, is stopped at its
/// back-edge on every placement.
#[test]
fn a_pure_spin_is_stopped_at_its_back_edge() {
    for (name, placement) in PLACEMENTS {
        let body = "        println(\"up\");\n        while true { self.n = self.n + 1; }";
        let tag = format!("drain_runtime_spin_{name}");
        let ran = term_after_up(&tag, &program(placement, "n: Int = 0;", body), Duration::from_millis(50), &[]);
        assert_clean(&tag, &ran);
    }
}

/// A `run()` that reads `draining` and stops on it (the shape the check
/// proves) ends by its own code: what it writes after its loop runs.
#[test]
fn a_run_that_reads_draining_ends_by_its_own_loop() {
    for (name, placement) in PLACEMENTS {
        let body = "        println(\"up\");\n        while !self.draining { std::time::sleep(20ms); }\n        println(\"flushed\");";
        let tag = format!("drain_runtime_manual_{name}");
        let ran = term_after_up(&tag, &program(placement, "n: Int = 0;", body), Duration::from_millis(50), &[]);
        assert_clean(&tag, &ran);
        assert!(ran.stdout.contains("flushed"), "{tag}: the author's code after the loop ran:\n{}", ran.stdout);
    }
}

/// A `run()` that reads `draining` and does not stop on it (the check
/// warns at its loop): the runtime leaves the loop to it, and the grace is
/// the backstop — the drain record, then the signal's default action.
#[test]
fn a_run_that_reads_draining_and_ignores_it_is_ended_by_the_grace() {
    for (name, placement) in PLACEMENTS {
        let body = "        if self.draining { return; }\n        println(\"up\");\n        while true { self.n = self.n + 1; }";
        let tag = format!("drain_runtime_unproven_{name}");
        let ran = term_after_up(
            &tag,
            &program(placement, "n: Int = 0;", body),
            Duration::from_millis(50),
            &[("LOTUS_DRAIN_GRACE_MS", "400")],
        );
        assert_eq!(ran.status.signal(), Some(15), "{tag}: ended by the signal after the grace: {:?}\n{}", ran.status, ran.stderr);
        assert!(ran.stopped_in >= Duration::from_millis(350), "{tag}: {:?}", ran.stopped_in);
        assert!(ran.stderr.contains("did not finish within 400 ms"), "{tag}: the grace's record:\n{}", ran.stderr);
    }
}

/// Untimed waits — an `accept` nothing will connect to, a `recv` on a
/// connection that will never send, a UDP `recv` on a quiet socket — wake
/// on the drain with their closed or empty outcome: `-1`, the empty
/// string, the receive's failure. `run()` goes on from there and returns.
#[test]
fn untimed_waits_wake_with_their_closed_outcome() {
    let waits = [
        (
            "accept",
            "        let lfd = std::io::tcp::__listen_socket(\"127.0.0.1\", 0);\n        println(\"up\");\n        let c = std::io::tcp::__accept_one(lfd);\n        println(\"woke \" + to_string(c));",
            "woke -1",
        ),
        (
            "recv",
            "        let lfd = std::io::tcp::__listen_socket(\"127.0.0.1\", 0);\n        let c = std::io::tcp::__connect(\"127.0.0.1\", std::io::tcp::__local_port(lfd));\n        println(\"up\");\n        let got = std::io::tcp::__recv(c, 64);\n        println(\"woke [\" + got + \"]\");",
            "woke []",
        ),
        (
            "udp",
            "        let u = std::io::udp::bind(\"127.0.0.1\", 0) or -1;\n        println(\"up\");\n        let got = std::io::udp::recv(u, 64) or std::bytes::from_string(\"closed\");\n        println(\"woke \" + to_string(std::bytes::at(got, 0) or -1));",
            // the substitute's first byte, `c`: the receive failed
            "woke 99",
        ),
    ];
    for (what, body, woke) in waits {
        for (name, placement) in PLACEMENTS {
            let tag = format!("drain_runtime_{what}_{name}");
            let ran = term_after_up(&tag, &program(placement, "n: Int = 0;", body), Duration::from_millis(150), &[]);
            assert_clean(&tag, &ran);
            assert!(ran.stdout.lines().any(|l| l == woke), "{tag}: the wait woke with `{woke}`:\n{}", ran.stdout);
        }
    }
}

/// `fn main`'s own loop is the program's run: stopped at its back-edge.
#[test]
fn fn_main_s_own_loop_is_stopped() {
    let src = "fn main() {\n    println(\"up\");\n    let mut n = 0;\n    while true { n = n + 1; std::time::sleep(50ms); }\n    println(\"after\");\n}\n";
    let ran = term_after_up("drain_runtime_fn_main", src, Duration::from_millis(100), &[]);
    assert_eq!(ran.status.code(), Some(0), "{:?}\n{}", ran.status, ran.stderr);
    assert!(ran.stopped_in < Duration::from_secs(1), "{:?}", ran.stopped_in);
    assert!(ran.stdout.lines().any(|l| l == "after"), "{}", ran.stdout);
}

/// The back-edge check is in the IR of a `run()` that never reads
/// `draining`, and only there: not in one that does, not in a free fn.
#[test]
fn the_back_edge_check_is_where_the_runtime_stops() {
    let src = "locus Auto { params { n: Int = 0; } run() { while true { self.n = self.n + 1; } } }\n\
               locus Manual { params { n: Int = 0; } run() { while !self.draining { self.n = self.n + 1; } } }\n\
               fn helper() -> Int { let mut i = 0; while i < 10 { i = i + 1; } return i; }\n\
               main locus App { params { a: Auto = Auto { }; m: Manual = Manual { }; } placement { a: pinned; m: pinned; } }\n\
               fn main() { App { }; println(to_string(helper())); }\n";
    let bin = harness::unique_bin("drain_runtime_ir");
    let ll = bin.with_extension("ll");
    let options = hale_codegen::BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("build");
    let ir = std::fs::read_to_string(&ll).expect("the IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    // Each defined function: its `define` line and its body.
    let fns: Vec<(&str, &str)> = ir
        .split("\ndefine ")
        .skip(1)
        .map(|f| {
            let head = f.lines().next().unwrap_or("");
            (head, &f[..f.find("\n}\n").unwrap_or(f.len())])
        })
        .collect();
    let of = |pred: &dyn Fn(&str) -> bool| -> Vec<&str> { fns.iter().filter(|(h, _)| pred(h)).map(|(_, b)| *b).collect() };
    let names: Vec<&str> = fns.iter().map(|(h, _)| *h).collect();
    let auto = of(&|h| h.contains("@Auto") && h.contains("run"));
    let manual = of(&|h| h.contains("@Manual") && h.contains("run"));
    let helper = of(&|h| h.contains("helper"));
    assert!(!auto.is_empty() && !manual.is_empty() && !helper.is_empty(), "the fns:\n{}", names.join("\n"));
    assert!(auto.iter().any(|b| b.contains("loop.drain_stop")), "the stop is in Auto's run():\n{}", auto.join("\n---\n"));
    assert!(manual.iter().all(|b| !b.contains("loop.drain_stop")), "a run() that reads `draining` has none");
    assert!(helper.iter().all(|b| !b.contains("loop.drain_stop")), "a free fn's loop has none");
}
