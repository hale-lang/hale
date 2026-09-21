//! GH #905: the iris session `hale run --observe` starts is bounded by
//! the `hale` process, and never writes to the program's stdout.
//!
//! Both halves are about one descriptor. The session used to inherit
//! `hale`'s stdout, so a caller reading the program's output through a
//! pipe was waiting on the session's copy of the write end as well as
//! the program's — and the session used to survive `hale`, twice over:
//! `hale iris` outlived a SIGKILL'd `hale`, and fuse-hl outlived the
//! `hale iris` that `hale run --observe` kills when the program ends.
//! Either orphan kept the pipe open forever. `timeout 2 hale run
//! --observe x.hl | cat` never returned; nor did a run whose program
//! simply exited.
//!
//! The session's port is fuse-hl's fixed 8787 — `hale run --observe`
//! takes no port, so there is no `free_port()` to hand it. That can
//! only make an attempt VACUOUS (a session that cannot bind exits at
//! once, and then there is no orphan to leak), never falsely red, so
//! each leg asserts it had a live session to kill and retries if it
//! did not, rather than passing on a box where nothing ran.
#![cfg(target_os = "linux")]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Prints, then stays up long enough to be killed mid-run.
const SLEEPER: &str = r#"main locus App {
    run() {
        println("ready");
        std::time::sleep(120s);
    }
}

fn main() {
    App { };
}
"#;

/// Prints and ends: the run whose session outlived it.
const QUICK: &str = r#"main locus App {
    run() {
        println("ready");
    }
}

fn main() {
    App { };
}
"#;

/// The observer cache every test that launches iris shares, so the
/// observer is built once per machine and not once per test
/// (`iris_cli.rs` says the same). Never deleted: toolchain-hashed.
fn cache_root() -> PathBuf {
    let d = std::env::temp_dir().join("hale-tests-iris-cache");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A live process's command line, or `None` if it is gone or a zombie
/// (an orphan reparented to init is reaped within moments of dying).
fn live_cmdline(pid: u32) -> Option<String> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let state = status.lines().find_map(|l| l.strip_prefix("State:"))?;
    if state.trim_start().starts_with('Z') {
        return None;
    }
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(String::from_utf8_lossy(&raw).replace('\0', " "))
}

fn ppid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|l| l.strip_prefix("PPid:")).and_then(|v| v.trim().parse().ok())
}

/// Every live descendant of `root`, by pid and command line. Children
/// are found through `/proc`, never by matching a name pattern: a
/// pattern would blame another checkout's processes for our leak.
fn descendants(root: u32) -> Vec<(u32, String)> {
    let mut all: Vec<(u32, u32)> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    for e in entries.flatten() {
        let Ok(name) = e.file_name().into_string() else { continue };
        let Ok(pid) = name.parse::<u32>() else { continue };
        if let Some(parent) = ppid(pid) {
            all.push((pid, parent));
        }
    }
    let mut found = vec![root];
    let mut out = Vec::new();
    // Bounded by the depth of the tree: hale -> hale iris -> fuse-hl.
    for _ in 0..8 {
        let before = found.len();
        for (pid, parent) in &all {
            if found.contains(parent) && !found.contains(pid) {
                found.push(*pid);
                if let Some(cmd) = live_cmdline(*pid) {
                    out.push((*pid, cmd));
                }
            }
        }
        if found.len() == before {
            break;
        }
    }
    out
}

/// SIGKILL one pid — ours, or one `/proc` named as our descendant.
/// Never a pattern: `pkill -f` would blame another checkout's run.
fn kill(pid: u32) {
    let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).output();
}

fn seed(dir: &Path, name: &str, src: &str) -> PathBuf {
    let d = dir.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), src).unwrap();
    d.join("main.hl")
}

fn observe(prog: &Path, dir: &Path, log: &str) -> Child {
    let err = std::fs::File::create(dir.join(log)).unwrap();
    let runtime = dir.join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["run", "--observe"])
        .arg(prog)
        .env("XDG_CACHE_HOME", cache_root())
        // a private registry: this session discovers no other test's
        // observed processes, whose segments come and go under it
        .env("XDG_RUNTIME_DIR", &runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(err)
        .spawn()
        .expect("spawn hale run --observe")
}

/// Drain a child's stdout on a thread. Returns what was read so far
/// (shared) and a channel that receives once the pipe reaches EOF —
/// which is the property under test, so it must be bounded by the
/// caller rather than waited on with `read_to_end`.
fn drain(out: std::process::ChildStdout) -> (Arc<Mutex<Vec<u8>>>, mpsc::Receiver<()>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mine = Arc::clone(&seen);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = out;
        let mut buf = [0u8; 4096];
        loop {
            match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => mine.lock().unwrap().extend_from_slice(&buf[..n]),
            }
        }
        let _ = tx.send(());
    });
    (seen, rx)
}

/// One attempt at the killed-`hale` leg. `Err` means the session never
/// came up, which proves nothing either way; `Ok` means it was asserted.
fn killed_hale_leaves_nothing(dir: &Path, prog: &Path) -> Result<(), String> {
    let mut hale = observe(prog, dir, "kill-leg.err");
    let pid = hale.id();
    let (seen, eof) = drain(hale.stdout.take().unwrap());

    // Everything is up when the program has printed through the pipe
    // and a descendant is serving on 8787. The first run on a machine
    // builds the observer first, which is why this waits as long as
    // `iris_cli.rs` does.
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut session = Vec::new();
    while Instant::now() < deadline {
        if let Ok(Some(st)) = hale.try_wait() {
            return Err(format!("hale exited {st} before the session was up"));
        }
        session = descendants(pid);
        let running = String::from_utf8_lossy(&seen.lock().unwrap()).contains("ready");
        if running
            && session.iter().any(|(_, c)| c.contains("fuse-hl"))
            && std::net::TcpStream::connect(("127.0.0.1", 8787)).is_ok()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let Some(fuse) = session.iter().find(|(_, c)| c.contains("fuse-hl")).cloned() else {
        for (p, _) in &session {
            kill(*p);
        }
        kill(pid);
        let _ = hale.wait();
        return Err(format!("no fuse-hl under pid {pid} within 240s (port 8787 busy?)"));
    };

    kill(pid);
    let _ = hale.wait();

    let waited = eof.recv_timeout(Duration::from_secs(30));
    let out = String::from_utf8_lossy(&seen.lock().unwrap()).to_string();
    // Clean up before asserting: an assertion unwinds. A recorded pid
    // counts as gone once it is absent, a zombie, or wearing a
    // different command line (a recycled pid is not our leak).
    let mut survivors: Vec<(u32, String)>;
    let gone = Instant::now() + Duration::from_secs(15);
    loop {
        survivors = session
            .iter()
            .filter(|(p, cmd)| live_cmdline(*p).as_deref() == Some(cmd.as_str()))
            .cloned()
            .collect();
        if survivors.is_empty() || Instant::now() > gone {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for (p, _) in &survivors {
        kill(*p);
    }

    assert!(
        waited.is_ok(),
        "the pipe never reached EOF after hale ({pid}) was killed: something it started still \
         holds hale's stdout. Read so far: {out:?}"
    );
    assert!(
        survivors.is_empty(),
        "hale ({pid}) was killed and what it started outlived it: {survivors:?}"
    );
    assert!(
        !out.contains("fuse-hl"),
        "the session wrote to the program's stdout (fuse-hl was pid {}): {out:?}",
        fuse.0
    );
    // Moved, not muted: what the session says still reaches the
    // caller, on the stream diagnostics belong on.
    let said = std::fs::read_to_string(dir.join("kill-leg.err")).unwrap_or_default();
    assert!(
        said.contains("fuse-hl: listening"),
        "the session's own output is relayed to hale's stderr: {said:?}"
    );
    Ok(())
}

/// One attempt at the ordinary-exit leg: the program ends, `hale`
/// reaps the session, and a piped caller sees EOF. fuse-hl used to
/// survive the `hale iris` that `hale run --observe` kills here, and
/// hold the pipe open with nothing left to close it.
fn program_exit_closes_the_pipe(dir: &Path, prog: &Path) -> Result<(), String> {
    let mut hale = observe(prog, dir, "exit-leg.err");
    let pid = hale.id();
    let (seen, eof) = drain(hale.stdout.take().unwrap());
    let saw_session = Arc::new(Mutex::new(Vec::new()));
    {
        // The run is short, so the session is sampled from a thread
        // rather than waited for: what matters is that one existed.
        let saw = Arc::clone(&saw_session);
        std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(240);
            while Instant::now() < until {
                let d = descendants(pid);
                if d.iter().any(|(_, c)| c.contains("fuse-hl")) {
                    *saw.lock().unwrap() = d;
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
    }
    let waited = eof.recv_timeout(Duration::from_secs(240));
    let observed = saw_session.lock().unwrap().clone();
    if waited.is_err() {
        // Nothing closed the pipe: leave nothing behind before failing.
        for (p, _) in &observed {
            kill(*p);
        }
        kill(pid);
        let _ = hale.wait();
        let out = String::from_utf8_lossy(&seen.lock().unwrap()).to_string();
        panic!("`hale run --observe` never closed its stdout after the program ended: {out:?}");
    }
    let _ = hale.wait();
    if observed.is_empty() {
        return Err("no fuse-hl ran under this leg (port 8787 busy?)".into());
    }
    let out = String::from_utf8_lossy(&seen.lock().unwrap()).to_string();
    assert!(out.contains("ready"), "the program's own output reaches the pipe: {out:?}");
    assert!(!out.contains("fuse-hl"), "the session wrote to the program's stdout: {out:?}");
    Ok(())
}

#[test]
fn an_observed_session_dies_with_hale_and_owns_its_stdout() {
    let dir = std::env::temp_dir().join(format!("hale_observe_905_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sleeper = seed(&dir, "sleeper", SLEEPER);
    let quick = seed(&dir, "quick", QUICK);

    for leg in [
        ("hale killed", killed_hale_leaves_nothing as fn(&Path, &Path) -> Result<(), String>, sleeper),
        ("program exited", program_exit_closes_the_pipe, quick),
    ] {
        let (name, run, prog) = leg;
        let mut why = Vec::new();
        let mut ok = false;
        for _ in 0..3 {
            match run(&dir, &prog) {
                Ok(()) => {
                    ok = true;
                    break;
                }
                Err(e) => why.push(e),
            }
        }
        assert!(ok, "`{name}`: no attempt had a session to assert about: {why:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
