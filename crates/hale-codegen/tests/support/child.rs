//! A test's child process: what it wrote to stderr, and the port it
//! listens on.
//!
//! A program a test serves binds port 0 and the test reads back the port
//! the kernel gave it ([`bound_port`]). A port the test draws
//! (`ports::free_port`) and hands over is free, not reserved: under a
//! parallel run another test can bind it before the program does.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::io::Read;
use std::process::Child;
use std::time::{Duration, Instant};

/// A child's stderr, read on a thread of its own as it is written, so a
/// failure says what the program said.
#[derive(Clone, Default)]
pub struct Stderr {
    text: std::sync::Arc<std::sync::Mutex<String>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Stderr {
    pub fn of(child: &mut Child) -> Stderr {
        let s = Stderr::default();
        if let Some(mut e) = child.stderr.take() {
            let t = s.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n @ 1..) = e.read(&mut buf) {
                    t.text.lock().unwrap().push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                t.done.store(true, std::sync::atomic::Ordering::SeqCst);
            });
        }
        s
    }

    pub fn text(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    /// All of it, once the child has exited (bounded: a grandchild could
    /// hold the pipe).
    pub fn finished(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.done.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.text()
    }
}

/// The port the process listens on: its one listening TCP socket. The
/// programs here bind port 0 and the kernel picks; a port the test drew
/// (`ports::free_port`) and handed over could be bound by another test of
/// the run before the program bound it. Read once the program has said
/// it is up, so the socket is there.
/// Linux: read from `/proc`.
pub fn bound_port(pid: u32, err: &Stderr) -> u16 {
    let ports = listening_ports(pid);
    assert_eq!(ports.len(), 1, "one listening socket: {ports:?}; stderr:\n{}", err.text());
    ports[0]
}

/// The local ports of the process's listening TCP sockets.
pub fn listening_ports(pid: u32) -> Vec<u16> {
    let inodes: BTreeSet<String> = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| std::fs::read_link(e.path()).ok())
                .filter_map(|t| t.to_string_lossy().strip_prefix("socket:[").and_then(|r| r.strip_suffix(']')).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let table = std::fs::read_to_string(format!("/proc/{pid}/net/tcp")).unwrap_or_default();
    table
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() > 9 && f[3] == "0A" && inodes.contains(f[9]) {
                u16::from_str_radix(f[1].rsplit(':').next()?, 16).ok()
            } else {
                None
            }
        })
        .collect()
}
