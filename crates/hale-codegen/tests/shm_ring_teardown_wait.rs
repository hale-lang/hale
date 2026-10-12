//! A shm ring reader whose handler is parked in `or wait` when the
//! program ends does not hang the exit.
//!
//! The readers stop before main's loci dissolve (a handler runs on its
//! reader thread, and must not outlive the locus it belongs to). Main
//! used to join them outright, and a handler can be waiting on main: here
//! the shm subscriber forwards each record, as a String-bearing message,
//! to a main subscriber over a local `bounded(2); on_full: fail;` topic
//! with `or wait`. Once main's run() returns nothing drains that queue
//! but main, so a reader parked on the third forward waited for main
//! while main waited for the reader, forever. Main now reaps the readers
//! without blocking and drains its queue between polls (aborting the
//! waits if draining is not enough), so the program ends; and under
//! AddressSanitizer, with nothing touched after it is freed.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const PROGRAM: &str = r#"
type Tick { n: Int = 0; }
type Note { text: String = ""; }
topic Ticks { payload: Tick; }
topic Notes {
    payload: Note;
    subject: "notes";
    bounded(2);
    on_full: fail;
}
locus Fwd {
    bus {
        subscribe Ticks as on_tick;
        publish Notes;
    }
    fn on_tick(t: Tick) {
        Notes <- Note { text: "tick " + to_string(t.n) } or wait;
    }
}
locus Tally {
    params { seen: Int = 0; }
    bus { subscribe Notes as on_note; }
    fn on_note(n: Note) { self.seen = self.seen + 1; }
    dissolve() { println("notes seen: ", self.seen); }
}
locus Producer {
    bus { publish Ticks; }
    birth() {
        let mut i = 0;
        while i < 20 {
            Ticks <- Tick { n: i };
            i = i + 1;
        }
    }
}
main locus App {
    params {
        tally: Tally = Tally { };
        fwd: Fwd = Fwd { };
    }
    bindings { Ticks: shm_ring("__NAME__", slot_count: 64, on_overflow: drop) where zero_copy; }
    run() {
        Producer { };
        std::time::sleep(20ms);
    }
}
fn main() { App { }; }
"#;

fn shm_name(tag: &str) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("/hale-teardown-wait-{tag}-{}-{nanos}", std::process::id())
}

/// Run `bin`; it must end, cleanly, within the deadline.
fn ends_cleanly(bin: &Path, env: &[(&str, &str)]) {
    let mut child = Command::new(bin).envs(env.iter().copied()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("run");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait") {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the program did not end within 30 s: main and a reader parked in `or wait` wait for each other");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = child.wait_with_output().expect("output");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(status.success(), "{status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    let seen: i64 = stdout
        .lines()
        .find_map(|l| l.strip_prefix("notes seen: "))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("Tally said nothing:\n{stdout}\n{stderr}"));
    assert!(seen > 0, "the forwarded notes reached main's subscriber: {stdout}");
}

#[test]
fn a_reader_parked_in_or_wait_does_not_hang_the_exit() {
    let bin = harness::unique_bin("shm_teardown_wait");
    let src = PROGRAM.replace("__NAME__", &shm_name("plain"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    ends_cleanly(&bin, &[]);
}

#[test]
fn a_reader_parked_in_or_wait_ends_clean_under_asan() {
    let bin = harness::unique_bin("shm_teardown_wait_asan");
    let src = PROGRAM.replace("__NAME__", &shm_name("asan"));
    harness::build_source_asan(&src, &bin);
    ends_cleanly(&bin, &[("LOTUS_NO_CHUNK_POOL", "1")]);
}
