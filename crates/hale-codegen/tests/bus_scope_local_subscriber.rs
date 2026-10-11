//! A subscriber born and dissolved inside a method (a `let` or a
//! statement-position literal, owned by nothing beyond the scope) takes
//! the delivery domain of the thread that builds it, as an owned one
//! does (spec/runtime.md § "Subscriptions follow the tower").
//!
//! Its registration used to be tagged with a pool only when the locus was
//! owned beyond its scope, so a scope-local subscriber built on a pool
//! worker registered with no pool: its deliveries went to the global
//! queue and main ran its handlers, while the worker that built it was
//! still using it (the downstream handoff's HTTP connections closed
//! without a response under load, and a process sometimes faulted). A
//! scope-local subscriber on a pinned thread went to the global queue the
//! same way.
//!
//! The handlers report `std::io::tcp::__on_pool()` (1 on a pool's worker,
//! 0 elsewhere); where main runs or blocks tells a pinned thread from main.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

// A host whose method builds a scope-local subscriber, publishes to it
// and sleeps (an `async_io` pool parks the body there; main's queue
// drains there). Main sleeps meanwhile, draining the global queue: a
// handler that reached it runs on main and says so.
const SCOPE_LOCAL: &str = r#"
    type Msg { n: Int; }
    locus Peer {
        params { got: Int = 0; }
        bus { subscribe "peer.msg" as on_msg of type Msg; }
        fn on_msg(m: Msg) {
            self.got = self.got + 1;
            println("handler n=" + to_string(m.n) + " on_pool=" + to_string(std::io::tcp::__on_pool()));
        }
    }
    locus Host {
        bus { publish "peer.msg" of type Msg; }
        fn work() {
            let p = Peer { };
            "peer.msg" <- Msg { n: 1 };
            std::time::sleep(250ms);
            println("work on_pool=" + to_string(std::io::tcp::__on_pool()) + " got=" + to_string(p.got));
        }
        run() { self.work(); }
    }
    main locus App {
        params { h: Host = Host { }; }
        placement { h: PLACEMENT; }
        run() {
            std::time::sleep(600ms);
            println("main done");
        }
    }
    fn main() { App { }; }
"#;

fn run_scope_local(tag: &str, placement: &str) -> String {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(&SCOPE_LOCAL.replace("PLACEMENT", placement), &bin, &build_opts::options())
        .expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "{tag}: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("main done"), "{tag}: {stdout}");
    stdout
}

#[test]
fn an_async_io_pool_serves_its_scope_local_subscriber() {
    // the handler runs on the worker while the body that built the
    // subscriber parks in its sleep
    let out = run_scope_local("scope_local_sub_async", "cooperative(pool = aio) where async_io");
    assert!(out.contains("handler n=1 on_pool=1"), "the handler ran off the pool:\n{out}");
    assert!(out.contains("work on_pool=1 got=1"), "{out}");
    assert!(!out.contains("on_pool=0"), "a handler ran on main:\n{out}");
}

#[test]
fn a_classic_pool_never_hands_its_scope_local_subscriber_to_main() {
    // a classic pool's cells are handler-atomic: the delivery waits for
    // the worker, which the body holds for the subscriber's whole life,
    // and is dropped with it at its dissolve. It is not run on main.
    let out = run_scope_local("scope_local_sub_classic", "cooperative(pool = cls)");
    assert!(!out.contains("handler"), "a handler ran on main:\n{out}");
    assert!(out.contains("work on_pool=1 got=0"), "{out}");
}

#[test]
fn main_serves_its_own_scope_local_subscriber() {
    let out = run_scope_local("scope_local_sub_main", "cooperative(pool = main)");
    assert!(out.contains("handler n=1 on_pool=0"), "{out}");
    assert!(out.contains("work on_pool=0 got=1"), "{out}");
}

// A pinned host (it subscribes itself, so its thread has a mailbox),
// building the same scope-local subscriber while main blocks on stdin:
// only the pinned thread can run the handler before the scope ends.
const PINNED_BLOCKED_MAIN: &str = r#"
    type Msg { n: Int; }
    locus Peer {
        params { got: Int = 0; }
        bus { subscribe "peer.msg" as on_msg of type Msg; }
        fn on_msg(m: Msg) {
            self.got = self.got + 1;
            println("handler n=" + to_string(m.n));
        }
    }
    locus Host {
        bus {
            publish "peer.msg" of type Msg;
            subscribe "host.ping" as on_ping of type Msg;
        }
        fn on_ping(m: Msg) { }
        fn work() {
            let p = Peer { };
            "peer.msg" <- Msg { n: 1 };
            std::time::sleep(250ms);
            println("work got=" + to_string(p.got));
        }
        run() { self.work(); }
    }
    main locus App {
        params { h: Host = Host { }; }
        placement { h: pinned; }
        run() {
            let line = std::io::stdin::read_line();
            println("main got " + line);
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn a_pinned_thread_serves_its_scope_local_subscriber_while_main_blocks() {
    let bin = harness::unique_bin("scope_local_sub_pinned");
    build_opts::build_source(PINNED_BLOCKED_MAIN, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().unwrap();
    let out = child.stdout.take().unwrap();
    let (tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while !seen.iter().any(|l: &String| l.starts_with("work got=")) {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(l) => seen.push(l),
            Err(_) => break,
        }
    }
    // main is still blocked in read_line here
    stdin.write_all(b"bye\n").unwrap();
    drop(stdin);
    let status = child.wait().expect("wait");
    seen.extend(lines.try_iter());
    let mut err = String::new();
    let _ = child.stderr.take().unwrap().read_to_string(&mut err);
    let _ = std::fs::remove_file(&bin);
    assert!(status.success(), "{status:?}\n{err}");
    assert!(seen.iter().any(|l| l == "work got=1"), "the pinned thread did not serve it: {seen:?}");
    let handler = seen.iter().position(|l| l == "handler n=1");
    let main_got = seen.iter().position(|l| l == "main got bye");
    assert!(handler.is_some() && handler < main_got, "{seen:?}");
}

// Many scope-local subscribers born and dissolved on two pools while a
// pinned publisher floods their subject: cells queued for a dissolved one
// are dropped, a handler parked in its sleep holds its subscriber until
// it returns, and no handler runs on main.
const STRESS: &str = r#"
    type Msg { n: Int; }
    type Done { who: String; got: Int; }
    locus Peer {
        params { got: Int = 0; note: String = ""; }
        bus { subscribe "peer.msg" as on_msg of type Msg; }
        fn on_msg(m: Msg) {
            if std::io::tcp::__on_pool() == 0 { println("BAD handler off its pool n=" + to_string(m.n)); }
            self.note = "seen " + to_string(m.n);
            if m.n % 7 == 0 { std::time::sleep(1ms); }
            self.got = self.got + 1;
        }
    }
    locus Host {
        params { label: String = ""; rounds: Int = 0; got: Int = 0; }
        bus { publish "host.done" of type Done; }
        fn once(i: Int) {
            let p = Peer { note: "born " + to_string(i) };
            std::time::sleep(1ms);
            self.got = self.got + p.got;
        }
        run() {
            let mut i = 0;
            while i < self.rounds {
                self.once(i);
                i = i + 1;
            }
            "host.done" <- Done { who: self.label, got: self.got };
        }
    }
    locus Pub {
        params { n: Int = 0; }
        bus { publish "peer.msg" of type Msg; }
        run() {
            let mut i = 0;
            while i < self.n {
                "peer.msg" <- Msg { n: i };
                if i % 4 == 0 { std::time::sleep(1ms); }
                i = i + 1;
            }
        }
    }
    main locus App {
        params {
            a: Host = Host { label: "async_a", rounds: 200 };
            b: Host = Host { label: "async_b", rounds: 200 };
            c: Host = Host { label: "classic", rounds: 200 };
            p: Pub = Pub { n: 2000 };
            done: Int = 0;
        }
        placement {
            a: cooperative(pool = aio) where async_io;
            b: cooperative(pool = aio) where async_io;
            c: cooperative(pool = cls);
            p: pinned;
        }
        bus { subscribe "host.done" as on_done of type Done; }
        fn on_done(d: Done) {
            println(d.who + " got=" + to_string(d.got));
            self.done = self.done + 1;
        }
        run() {
            while self.done < 3 { std::time::sleep(10ms); }
            println("all done");
        }
    }
    fn main() { App { }; }
"#;

fn got(out: &str, who: &str) -> i64 {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{who} got=")))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no `{who} got=` line in:\n{out}"))
}

#[test]
fn scope_local_subscribers_churn_clean_under_asan() {
    // with the chunk pool off, a cell run on a dissolved subscriber's
    // freed arena, or a handler run on main beside the pool, is reported
    let bin = harness::unique_bin("scope_local_sub_stress_asan");
    harness::build_source_asan(STRESS, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    assert!(out.status.success(), "{:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    assert!(stdout.contains("all done"), "{stdout}");
    assert!(!stdout.contains("BAD"), "a handler ran off its pool:\n{stdout}");
    // the async pools serve their subscribers while the bodies park; the
    // classic pool's deliveries are dropped with each subscriber
    assert!(got(&stdout, "async_a") > 0 && got(&stdout, "async_b") > 0, "{stdout}");
    assert_eq!(got(&stdout, "classic"), 0, "{stdout}");
}
