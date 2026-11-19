//! httpload — a closed-loop HTTP/1.1 load generator, standard library only.
//!
//! Each of `--conns` threads holds one keep-alive connection and sends
//! `GET <path>` back to back: the next request goes out when the last
//! response is read whole (by its `Content-Length`). A response that says
//! `Connection: close` is followed by a new connection, and that reconnect
//! counts toward the next request's latency (`reconnects`). A request whose
//! connection fails before its response is complete (a reset, an early
//! close, a short body) is a `dropped` attempt: counted, then sent again
//! on a new connection. A response other than 200 is an `error`. The first
//! `--warmup` seconds are not recorded. Prints one JSON object: requests,
//! errors, dropped, reconnects, seconds, requests per second, and latency
//! percentiles in microseconds.
//!
//! Build: `rustc -O --edition 2021 bench/tools/httpload.rs -o <out>`
//! Run:   `<out> --addr 127.0.0.1:8080 --conns 16 --duration 20 --warmup 3`

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

struct Opts {
    addr: String,
    path: String,
    conns: usize,
    duration: f64,
    warmup: f64,
}

fn opts() -> Opts {
    let mut o = Opts { addr: "127.0.0.1:8080".into(), path: "/".into(), conns: 16, duration: 20.0, warmup: 3.0 };
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i + 1 < a.len() {
        let v = a[i + 1].clone();
        match a[i].as_str() {
            "--addr" => o.addr = v,
            "--path" => o.path = v,
            "--conns" => o.conns = v.parse().expect("--conns"),
            "--duration" => o.duration = v.parse().expect("--duration"),
            "--warmup" => o.warmup = v.parse().expect("--warmup"),
            other => panic!("unknown flag {other}"),
        }
        i += 2;
    }
    o
}

/// Reads one response off `s` into `buf`: headers, then the body by its
/// Content-Length. Returns (status, whether the server asked to close),
/// or None when the connection ended first.
fn read_response(s: &mut TcpStream, buf: &mut Vec<u8>) -> Option<(u16, bool)> {
    buf.clear();
    let mut chunk = [0u8; 16384];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        let n = s.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
    let status: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    let mut len = 0usize;
    let mut close = false;
    for line in head.lines() {
        if let Some(v) = line.strip_prefix("content-length:") {
            len = v.trim().parse().ok()?;
        } else if let Some(v) = line.strip_prefix("connection:") {
            close = v.trim() == "close";
        }
    }
    while buf.len() < head_end + len {
        let n = s.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Some((status, close))
}

fn connect(addr: &str) -> TcpStream {
    loop {
        if let Ok(s) = TcpStream::connect(addr) {
            s.set_nodelay(true).ok();
            return s;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn main() {
    let o = opts();
    let req = format!("GET {} HTTP/1.1\r\nHost: {}\r\nConnection: keep-alive\r\n\r\n", o.path, o.addr);
    let stop = Arc::new(AtomicBool::new(false));
    let start = Instant::now();
    let warm = Duration::from_secs_f64(o.warmup);
    let end = Duration::from_secs_f64(o.warmup + o.duration);
    let mut handles = Vec::new();
    for _ in 0..o.conns {
        let (addr, req, stop) = (o.addr.clone(), req.clone(), stop.clone());
        handles.push(thread::spawn(move || {
            let mut lat: Vec<u32> = Vec::with_capacity(1 << 20);
            let (mut errors, mut dropped, mut reconnects) = (0u64, 0u64, 0u64);
            let mut s = connect(&addr);
            let mut buf = Vec::with_capacity(16384);
            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                let mut failed = 0u64;
                let ok = loop {
                    if s.write_all(req.as_bytes()).is_ok() {
                        if let Some((status, close)) = read_response(&mut s, &mut buf) {
                            if close {
                                s = connect(&addr);
                                reconnects += 1;
                            }
                            break status == 200;
                        }
                    }
                    // the connection failed before a whole response: a
                    // dropped attempt; reopen and resend
                    failed += 1;
                    s = connect(&addr);
                    if stop.load(Ordering::Relaxed) {
                        break false;
                    }
                };
                let since = start.elapsed();
                if since >= warm && since < end {
                    dropped += failed;
                    if ok {
                        lat.push(t.elapsed().as_micros().min(u32::MAX as u128) as u32);
                    } else {
                        errors += 1;
                    }
                }
            }
            (lat, errors, dropped, reconnects)
        }));
    }
    thread::sleep(end);
    stop.store(true, Ordering::Relaxed);
    let (mut all, mut errors, mut dropped, mut reconnects) = (Vec::new(), 0, 0, 0);
    for h in handles {
        let (l, e, d, r) = h.join().unwrap();
        all.extend(l);
        errors += e;
        dropped += d;
        reconnects += r;
    }
    all.sort_unstable();
    let pct = |p: f64| -> u32 {
        if all.is_empty() {
            return 0;
        }
        all[((all.len() as f64 * p).ceil() as usize).clamp(1, all.len()) - 1]
    };
    println!(
        "{{\"conns\":{},\"seconds\":{},\"requests\":{},\"errors\":{},\"dropped\":{},\"reconnects\":{},\"rps\":{:.0},\"p50_us\":{},\"p90_us\":{},\"p99_us\":{},\"p999_us\":{},\"max_us\":{}}}",
        o.conns,
        o.duration,
        all.len(),
        errors,
        dropped,
        reconnects,
        all.len() as f64 / o.duration,
        pct(0.50),
        pct(0.90),
        pct(0.99),
        pct(0.999),
        all.last().copied().unwrap_or(0)
    );
}
