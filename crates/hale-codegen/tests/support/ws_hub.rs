//! A compiled program that serves a stream hub, and the clients that drive
//! its socket (GH #1417, R5).
//!
//! A test builds a program (`build_source`), starts it with [`Server`] and
//! talks to it. The program is driven by files, not clocks: it reads the
//! command file `CTL/cmd`, runs the command it finds (and removes the
//! file), and ends when the command is `stop`, so a test says when each
//! step happens and holds the program's own teardown to account (a program
//! that does not exit within the wait is a hang, and fails the test). The
//! program writes what a test may wait for (`CTL/stats`) after every step.

#![allow(dead_code)]

use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::harness;
use super::ports;

/// What the program printed and how it ended.
pub struct Finished {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// A running program: the port its hub listens on and the directory its
/// commands and statistics live in.
pub struct Server {
    child: Option<Child>,
    pub dir: PathBuf,
    pub port: u16,
}

impl Server {
    /// Run `bin` with `BIND` (the address its hub listens on) and `CTL` (the
    /// directory of its command file) in its environment, plus `env`, on a
    /// port nothing holds (a draw another process took before the program
    /// bound it is drawn again).
    pub fn start(bin: &Path, env: &[(&str, &str)]) -> Server {
        for _ in 0..10 {
            let port = ports::free_port();
            let dir = harness::unique_dir("wshub");
            let mut s = Server::start_on(bin, env, port, dir);
            // the hub binds in the program's birth: it is listening, or it has failed
            let start = Instant::now();
            loop {
                if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    return s;
                }
                if s.exited() || start.elapsed() > Duration::from_secs(30) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(s);
        }
        panic!("the program never listened");
    }

    /// Run `bin` on exactly `port`, in `dir`.
    pub fn start_on(bin: &Path, env: &[(&str, &str)], port: u16, dir: PathBuf) -> Server {
        let mut cmd = Command::new(bin);
        cmd.env("BIND", format!("127.0.0.1:{port}")).env("CTL", &dir).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("start the program");
        Server { child: Some(child), dir, port }
    }

    /// Tell the program to run `command`, and wait for it to have taken it.
    pub fn command(&self, command: &str) {
        let tmp = self.dir.join("cmd.tmp");
        std::fs::write(&tmp, command).expect("write the command");
        std::fs::rename(&tmp, self.dir.join("cmd")).expect("publish the command");
        let start = Instant::now();
        while self.dir.join("cmd").exists() {
            assert!(start.elapsed() < Duration::from_secs(20), "the program never took `{command}`");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The program's statistics (`key=value` lines), as last written.
    pub fn stats(&self) -> Vec<(String, i64)> {
        let text = std::fs::read_to_string(self.dir.join("stats")).unwrap_or_default();
        text.lines()
            .filter_map(|l| l.split_once('='))
            .filter_map(|(k, v)| v.trim().parse().ok().map(|v| (k.to_string(), v)))
            .collect()
    }

    pub fn stat(&self, key: &str) -> Option<i64> {
        self.stats().into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Wait until the statistic `key` reads `want`.
    pub fn await_stat(&self, key: &str, want: i64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            if self.stat(key) == Some(want) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("`{key}` never read {want}; it reads {:?} in {:?}", self.stat(key), self.stats());
    }

    /// Whether the program has exited.
    pub fn exited(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(Some(_))),
            None => true,
        }
    }

    /// Ask the program to stop (its hub's `stop()` runs), wait for it to end,
    /// and hand back what it printed.
    pub fn finish(mut self) -> Finished {
        self.command("stop");
        self.wait(Duration::from_secs(30))
    }

    /// Wait for the program to end on its own.
    pub fn wait(&mut self, limit: Duration) -> Finished {
        let mut child = self.child.take().expect("running");
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("wait") {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut stdout);
                }
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut stderr);
                }
                return Finished { status, stdout, stderr };
            }
            if start.elapsed() > limit {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the program did not exit within {limit:?} of being asked to stop");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
