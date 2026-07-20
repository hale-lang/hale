//! std::http client surface (promoted from pond/http/client,
//! 2026-07-17): get / post / parse_url / ClientRequest / Client.
//!
//! The Rust side plays the server so the client's keep-alive
//! framing paths get exercised against behaviors the stdlib
//! Server doesn't emit: persistent connections (Content-Length
//! framed, connection held open) and Transfer-Encoding: chunked.
//! The pool-reuse assertion counts REQUESTS PER ACCEPTED
//! CONNECTION — two keep-alive gets on one conn proves the pool
//! returned and reused the fd.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_http_client_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

/// Read one request's header block (through \r\n\r\n) plus its
/// Content-Length body from the stream. Returns None on EOF.
fn read_one_request(s: &mut std::net::TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    // headers
    loop {
        match s.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) => buf.push(byte[0]),
            Err(_) => return None,
        }
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if buf.len() > 65536 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let clen: usize = head
        .lines()
        .find(|l| l.to_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; clen];
    if clen > 0 && s.read_exact(&mut body).is_err() {
        return None;
    }
    Some(head + &String::from_utf8_lossy(&body))
}

#[test]
fn keep_alive_client_reuses_one_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let accepts = Arc::new(AtomicUsize::new(0));
    let served = Arc::new(AtomicUsize::new(0));
    let (a2, s2) = (accepts.clone(), served.clone());
    let server = thread::spawn(move || {
        // Keep-alive server: hold each accepted conn open, answer
        // every request on it with a Content-Length-framed 200.
        for stream in listener.incoming() {
            let mut s = match stream {
                Ok(s) => s,
                Err(_) => break,
            };
            a2.fetch_add(1, Ordering::SeqCst);
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut n_on_conn = 0;
            while let Some(_req) = read_one_request(&mut s) {
                n_on_conn += 1;
                s2.fetch_add(1, Ordering::SeqCst);
                let body = format!("resp-{}", n_on_conn);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
                    body.len(),
                    body
                );
                if s.write_all(resp.as_bytes()).is_err() {
                    break;
                }
            }
            if s2.load(Ordering::SeqCst) >= 2 {
                break; // both requests served — stop accepting
            }
        }
    });

    let src = format!(
        r#"
        fn main() {{
            let c = std::http::Client {{ keep_alive: true, max_retries: 0 }};
            let r1 = c.get("http://127.0.0.1:{port}/a") or raise;
            let r2 = c.get("http://127.0.0.1:{port}/b") or raise;
            println("b1=", std::str::from_bytes(r1.body),
                    " b2=", std::str::from_bytes(r2.body));
        }}
    "#
    );
    let (out, status) = build_and_run("keepalive", &src);
    let _ = server.join();
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    // resp-1 then resp-2 ON THE SAME CONNECTION: the server's
    // per-conn counter reaching 2 is only possible via fd reuse.
    assert!(out.contains("b1=resp-1 b2=resp-2"), "got:\n{}", out);
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        1,
        "keep-alive client must reuse one connection (accepts != 1)"
    );
    assert_eq!(served.load(Ordering::SeqCst), 2);
}

#[test]
fn chunked_response_reassembles() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        s.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let _ = read_one_request(&mut s);
        // Three chunks incl. a chunk-extension, then the terminator.
        let resp = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n\
                    5\r\nhello\r\n1;ext=1\r\n-\r\n5\r\nworld\r\n0\r\n\r\n";
        let _ = s.write_all(resp.as_bytes());
        // Hold the conn open briefly: a read-to-close client would
        // hang; the framed reader must stop at the terminator.
        thread::sleep(Duration::from_millis(500));
    });

    let src = format!(
        r#"
        fn main() {{
            let c = std::http::Client {{ keep_alive: true, max_retries: 0 }};
            let r = c.get("http://127.0.0.1:{port}/chunked") or raise;
            println("status=", r.status, " body=[", std::str::from_bytes(r.body), "]");
        }}
    "#
    );
    let (out, status) = build_and_run("chunked", &src);
    let _ = server.join();
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(
        out.contains("status=200 body=[hello-world]"),
        "chunked body must reassemble across chunk boundaries:\n{}",
        out
    );
}

#[test]
fn oneshot_chunked_dechunks_and_rejects_malformed() {
    // The FRICTION-log failure shape: a chunked response with NO
    // Content-Length on the DEFAULT one-shot path (std::http::get,
    // keep_alive off) used to return the raw hex-size framing as
    // the body ("clean run, empty result" after JSON parsing).
    // First accept serves a well-formed chunked response with a
    // chunk-extension and a trailer header; second accept serves
    // malformed chunk framing, which must surface as an error —
    // never as partial/framed body bytes.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let responses = [
            // valid: 3 chunks + chunk-ext + trailer header
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
             6\r\nhello,\r\n1;ext=x\r\n \r\n5\r\nworld\r\n0\r\nX-Trailer: ignored\r\n\r\n",
            // malformed: size line is not hex
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
             zz\r\nbogus\r\n0\r\n\r\n",
        ];
        for resp in responses {
            let (mut s, _) = match listener.accept() {
                Ok(x) => x,
                Err(_) => return,
            };
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let _ = read_one_request(&mut s);
            let _ = s.write_all(resp.as_bytes());
            // close on drop — one-shot clients read to EOF
        }
    });

    let src = format!(
        r#"
        fn main() {{
            let r1 = std::http::get("http://127.0.0.1:{port}/chunked") or raise;
            println("status=", r1.status, " body=[", std::str::from_bytes(r1.body), "]");
            let r2 = std::http::get("http://127.0.0.1:{port}/bad-chunked") or std::http::ClientResponse {{
                status: -1, headers: "", body: b""
            }};
            println("malformed-rejected=", r2.status == -1);
        }}
    "#
    );
    let (out, status) = build_and_run("oneshot_chunked", &src);
    let _ = server.join();
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(
        out.contains("status=200 body=[hello, world]"),
        "one-shot chunked body must be de-chunked (no hex framing):\n{}",
        out
    );
    assert!(
        out.contains("malformed-rejected=true"),
        "malformed chunk framing must error, not return partial data:\n{}",
        out
    );
}

#[test]
fn oneshot_get_post_and_error_channel() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut s, _) = match listener.accept() {
                Ok(x) => x,
                Err(_) => return,
            };
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let req = read_one_request(&mut s).unwrap_or_default();
            let body = if req.starts_with("POST") {
                let b = req.split("\r\n\r\n").nth(1).unwrap_or("");
                format!("echo:{}", b)
            } else {
                "plain".to_string()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });

    let src = format!(
        r#"
        fn main() {{
            let r1 = std::http::get("http://127.0.0.1:{port}/x") or raise;
            println("get=[", std::str::from_bytes(r1.body), "]");
            let r2 = std::http::post("http://127.0.0.1:{port}/x",
                std::bytes::from_string("hi"), "text/plain") or raise;
            println("post=[", std::str::from_bytes(r2.body), "]");
            let bad = std::http::parse_url("not a url") or std::http::Url {{
                scheme: "", host: "", port: 0, path: ""
            }};
            println("bad-url-handled=", len(bad.scheme) == 0);
        }}
    "#
    );
    let (out, status) = build_and_run("oneshot", &src);
    let _ = server.join();
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("get=[plain]"), "got:\n{}", out);
    assert!(out.contains("post=[echo:hi]"), "got:\n{}", out);
    assert!(out.contains("bad-url-handled=true"), "got:\n{}", out);
}
