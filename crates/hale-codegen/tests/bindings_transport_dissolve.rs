//! GH #893 — a `bindings { }` transport is dissolved at main exit.
//!
//! `emit_bindings_prelude` lowers a `bindings { T: unix(...) }` entry
//! as a locus instantiation of `__StdBusUnix{Listen,Connect}Transport`
//! in `fn main`'s prologue, with `current_user_fn_ret` spoofed to
//! `LocusRef(<transport>)` so the struct lands in the program-lifetime
//! payload arena rather than main's subregion. That is the m90
//! `returns_this_locus` path, and it decides two things at once: where
//! the struct lives, AND that nobody owns it — the eager dissolve is
//! suppressed and no deferred-dissolve frame entry is pushed, because
//! a returned locus is the caller's problem. The transport had no
//! caller, so it was nobody's.
//!
//! What that cost, beyond the 216-byte `lotus_arena_t`: the
//! transport's own `dissolve()` — `std::bus::__transport_reclaim`,
//! which interrupts the serve thread, joins it, and destroys the
//! remote entry — never ran at all. The serve thread was reaped only
//! by `lotus_bus_remote_destroy_all`'s fallback join inside
//! `lotus_bus_queue_destroy`, after the global arena had already been
//! destroyed, and the locus's arena was never destroyed by anything.
//!
//! The oracle is `LOTUS_ARENA_RESIDENCY=1`: the runtime's own registry
//! of live top-level arenas, walked by an atexit hook on an ordinary
//! build with no sanitizer in play. Before the fix a `bindings { }`
//! program exits with one live arena labelled after its transport
//! locus; after it, zero. `corpus_oracle.rs`'s ASan pass carries the
//! same regression for the corpus fixture (`85-bindings-unix` came off
//! `LEAKS_UNMASKED_BY_NO_CHUNK_POOL` with this change).
//!
//! Both roles are covered, because both transport loci have a
//! non-empty `dissolve()`. The connect case also asserts the binding
//! still delivers: the transport is torn down at the END of main's
//! reverse-order flush — it is the frame's first entry — so every
//! user locus, including a dissolve-time publisher, has already had
//! its turn on the wire.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Generous enough for a loaded CI box, tight enough that a
/// `pthread_join` on a serve thread nothing ever interrupts trips it
/// instead of hanging the suite.
const DEADLINE: Duration = Duration::from_secs(30);

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_gh893_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn unique_socket_path(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}/hale-893-{}-{}-{}.sock",
        std::env::temp_dir().display(),
        tag,
        std::process::id(),
        nanos
    )
}

/// Threads the process currently has, straight out of procfs. Used to
/// witness that the listen transport's serve thread really was spawned
/// (otherwise "it was joined" would be vacuously true).
fn task_count(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{}/task", pid))
        .map(|d| d.flatten().count())
        .unwrap_or(0)
}

/// Wait for `child` with a deadline; a fix that deadlocks the exit
/// path must fail this test rather than wedge the suite.
fn wait_within(
    child: &mut std::process::Child,
    what: &str,
) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => return status,
            None if start.elapsed() >= DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{what}: still running after {:?} — the transport's \
                     dissolve joined a serve thread nothing woke",
                    DEADLINE
                );
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// `[arena_residency dump] N live arenas, sorted by bytes desc:` —
/// written to stderr by the atexit hook in lotus_arena.c.
fn live_arenas(what: &str, stderr: &str) -> usize {
    let line = stderr
        .lines()
        .find(|l| l.contains("[arena_residency dump]"))
        .unwrap_or_else(|| {
            panic!("{what}: no residency dump\nstderr:\n{stderr}")
        });
    line.split_whitespace()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{what}: unparsable dump line {line:?}"))
}

#[test]
fn a_listen_binding_transport_is_dissolved_at_main_exit() {
    let sock = unique_socket_path("listen");
    let src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "gh893.listen"; }}
        locus Sub {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{ self.seen = self.seen + 1; }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                std::time::sleep(400ms);
                println("served");
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let bin = build("listen", &src);
    let mut child = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");
    // The listen transport's serve loop is C-spawned by the locus's
    // birth(), which runs inline on the boot path — so by the time
    // run()'s sleep is under way the process has a second task.
    std::thread::sleep(Duration::from_millis(200));
    let tasks = task_count(child.id());
    let status = wait_within(&mut child, "listen binding");
    let out = child.wait_with_output().expect("collect output");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        status.success(),
        "listen binding: exit {status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("served"),
        "listen binding: run() did not finish\nstdout:\n{stdout}"
    );
    assert!(
        tasks >= 2,
        "listen binding: expected the transport's serve thread \
         alongside main, saw {tasks} task(s) — the join assertion \
         below would be vacuous"
    );
    let live = live_arenas("listen binding", &stderr);
    assert_eq!(
        live, 0,
        "listen binding: {live} arena(s) still live at exit — the \
         transport locus was never dissolved\nstderr:\n{stderr}"
    );
}

#[test]
fn a_connect_binding_transport_is_dissolved_at_main_exit() {
    let sock = unique_socket_path("connect");
    // The listener is a peer binary, not a raw socket: the transport
    // speaks the runtime's own framing over SOCK_SEQPACKET.
    let listen_src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "gh893.connect"; }}
        locus Sub {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                println("got=", t.n);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.sub.seen < 1 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 150 {{ std::process::exit(3); }}
                }}
                std::time::sleep(200ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let connect_src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "gh893.connect"; }}
        main locus Pub {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                Evt <- T {{ n: 7 }};
                std::time::sleep(200ms);
                println("published");
            }}
        }}
        fn main() {{ Pub {{ }}; }}
    "#,
        sock
    );
    let listen_bin = build("connect_peer", &listen_src);
    let connect_bin = build("connect", &connect_src);

    let mut listener = Command::new(&listen_bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn peer listener");
    // The connect transport realizes synchronously in birth() with
    // its own retry window, so a slow peer boot is absorbed there.
    std::thread::sleep(Duration::from_millis(300));

    let mut publisher = Command::new(&connect_bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn publisher");
    let pub_status = wait_within(&mut publisher, "connect binding");
    let pub_out = publisher.wait_with_output().expect("collect publisher");
    let lis_status = wait_within(&mut listener, "connect binding peer");
    let lis_out = listener.wait_with_output().expect("collect listener");
    let _ = std::fs::remove_file(&listen_bin);
    let _ = std::fs::remove_file(&connect_bin);
    let _ = std::fs::remove_file(&sock);

    let pub_stdout = String::from_utf8_lossy(&pub_out.stdout).to_string();
    let pub_stderr = String::from_utf8_lossy(&pub_out.stderr).to_string();
    let lis_stdout = String::from_utf8_lossy(&lis_out.stdout).to_string();
    let lis_stderr = String::from_utf8_lossy(&lis_out.stderr).to_string();
    assert!(
        pub_status.success(),
        "connect binding: exit {pub_status:?}\n\
         stdout:\n{pub_stdout}\nstderr:\n{pub_stderr}"
    );
    assert!(
        lis_status.success(),
        "connect binding peer: exit {lis_status:?}\n\
         stdout:\n{lis_stdout}\nstderr:\n{lis_stderr}"
    );
    // The transport is torn down last, so tearing it down did not cost
    // the delivery it exists for.
    assert!(
        lis_stdout.contains("got=7"),
        "connect binding: the peer never received the publish\n\
         stdout:\n{lis_stdout}\nstderr:\n{lis_stderr}"
    );
    assert!(
        pub_stdout.contains("published"),
        "connect binding: run() did not finish\nstdout:\n{pub_stdout}"
    );
    let live = live_arenas("connect binding", &pub_stderr);
    assert_eq!(
        live, 0,
        "connect binding: {live} arena(s) still live at exit — the \
         transport locus was never dissolved\nstderr:\n{pub_stderr}"
    );
    let peer_live = live_arenas("connect binding peer", &lis_stderr);
    assert_eq!(
        peer_live, 0,
        "connect binding peer: {peer_live} arena(s) still live at \
         exit\nstderr:\n{lis_stderr}"
    );
}
