//! `lotus_bus_queue_idle_wait`: the main queue's bounded wait for work.
//! The C driver pins the contract: a wait with nothing enqueued returns
//! at its deadline, and an enqueue from another thread ends a 1 s wait
//! within a few milliseconds (microseconds on an idle box) with the
//! cell's handler already run. A sleep's slice on the queue
//! (`lotus_bus_queue_sleep`, GH #2091) runs a cell enqueued mid-slice when
//! it arrives and still lasts its whole length.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

#[test]
fn a_foreign_enqueue_ends_the_wait_and_a_quiet_wait_ends_at_its_deadline() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = harness::unique_bin("lotus_bus_idle_wait_driver");
    let status = Command::new("clang")
        .arg(manifest.join("tests").join("bus_idle_wait_driver.c"))
        .arg(manifest.join("runtime").join("lotus_arena.c"))
        .arg("-O2")
        .arg("-lpthread")
        .arg("-lm")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building the idle-wait driver");
    let out = Command::new(&bin).output().expect("run driver");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "idle-wait contract violated.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok "));
}

/// The duration of `std::time::__idle_wait` is evaluated once, whichever
/// thread the call runs on: the main-queue wait and the `sleep` it falls
/// back to take the same value. A duration that counts its calls would
/// otherwise count twice on a pool worker, and sleep for the second value.
#[test]
fn the_duration_is_evaluated_once_for_both_wait_backends() {
    let src = r#"
fn next_delay() -> Duration {
    print("delay asked");
    return 1ms;
}

fn main() {
    std::time::__idle_wait(next_delay());
}
"#;
    let bin = harness::unique_bin("lotus_idle_wait_once");
    let ir = harness::build_source_ir_text(src, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    assert!(ir.contains("lotus_bus_idle_wait_main"), "the call lowers to the idle wait");
    let calls = ir
        .lines()
        .filter(|l| l.contains("call i64 @next_delay("))
        .count();
    assert_eq!(calls, 1, "one call of the duration fn for the one site:\n{ir}");
}

/// GH #2091: a handler on main that sleeps, while main's run() sleeps and
/// another thread posts to main. The run()'s sleep waits on main's queue
/// and runs a delivery when it arrives; the handler's sleep cannot (main's
/// handlers do not nest, so what arrives meanwhile waits for the handler to
/// return) and is a plain sleep. It was a busy loop: its wait woke for the
/// cell, found it still queued at every look after, and spun out each
/// slice: ~0.5 s of CPU for this program's 0.9 s of handler sleeps.
#[test]
fn a_handler_on_main_that_sleeps_while_run_sleeps_waits_without_spinning() {
    let src = r#"
type Tick { n: Int; }
topic Ticks { payload: Tick; subject: "t.tick"; }

@unbounded
fn hold_for(ms: Int) {
    let mut i = 0;
    while i < ms {
        std::time::sleep(1ms);
        i = i + 1;
    }
}

locus Slow {
    params { seen: Int = 0; }
    bus { subscribe Ticks as on_tick; }
    fn on_tick(t: Tick) {
        println("start " + to_string(t.n));
        hold_for(300);
        println("end " + to_string(t.n));
        self.seen = self.seen + 1;
    }
}

locus Feeder {
    bus { publish Ticks; }
    run() {
        std::time::sleep(50ms);
        Ticks <- Tick { n: 1 };
        std::time::sleep(50ms);
        Ticks <- Tick { n: 2 };
        std::time::sleep(50ms);
        Ticks <- Tick { n: 3 };
    }
}

main locus App {
    params {
        slow: Slow = Slow { };
        feeder: Feeder = Feeder { };
    }
    placement { feeder: cooperative(pool = feed) where async_io; }
    run() {
        while self.slow.seen < 3 { std::time::sleep(10ms); }
        println("done");
    }
}

fn main() { App { }; }
"#;
    let bin = harness::unique_bin("lotus_sleep_in_main_handler");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("run the program");
    let pid = child.id() as libc::pid_t;
    // wait4, not Child::wait: the child's own CPU time, whatever else this
    // process runs
    let mut status: libc::c_int = 0;
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let start = std::time::Instant::now();
    loop {
        let r = unsafe { libc::wait4(pid, &mut status, libc::WNOHANG, &mut usage) };
        assert!(r >= 0, "wait4 failed");
        if r == pid {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(30) {
            let _ = child.kill();
            let _ = std::fs::remove_file(&bin);
            panic!("the program did not end within 30 s");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = std::fs::remove_file(&bin);
    let mut out = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().expect("stdout"), &mut out).expect("read stdout");
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "exit status {status}:\n{out}");
    // each cell after the handler before it returned, never inside it
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, ["start 1", "end 1", "start 2", "end 2", "start 3", "end 3", "done"], "{out}");
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    let cpu = tv(usage.ru_utime) + tv(usage.ru_stime);
    assert!(cpu < 0.2, "the handler's sleeps spun: {cpu:.3} s of CPU for 0.9 s of sleeping");
}
