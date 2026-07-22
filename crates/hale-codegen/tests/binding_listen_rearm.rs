//! GH #233 step 2: listen-side re-arm. Peer EOF on a listen
//! binding is not connection *loss* — the listener socket is
//! still bound (bound at registration since #232), so the reader
//! loops back into accept() and serves the next peer. This is
//! what makes rolling restarts of the connect-side binary work:
//! publisher restarts, reconnects, and the subscriber keeps
//! receiving. Pre-#233 the reader thread exited at first EOF and
//! inbound silently stopped for the rest of the process.
//!
//! Shape: one subscriber binary (listen role) outlives TWO
//! sequential runs of the same publisher binary (connect role).
//! Each publisher run connects, publishes once, exits (EOF).
//! The subscriber must observe both deliveries.

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_listen_rearm_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn unique_socket_path() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}/hale-233-rearm-{}-{}.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    )
}

#[test]
fn listen_binding_serves_sequential_peers() {
    let sock = unique_socket_path();
    // Sub is a params child of App so it stays born for the whole
    // of App's run() (a statement-position `Sub { };` after
    // `App { };` would only be born after App's lifecycle ends).
    let sub_src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Sub {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                println("got=", self.seen);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                std::time::sleep(4000ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let pub_src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                Evt <- T {{ n: 7 }};
                std::time::sleep(200ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let sub_bin = build("sub", &sub_src);
    let pub_bin = build("pub", &pub_src);

    let mut sub = Command::new(&sub_bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");

    // First peer: connects (retry covers the subscriber's boot),
    // publishes once, exits — EOF on the subscriber side.
    let p1 = Command::new(&pub_bin).output().expect("run publisher 1");
    assert!(
        p1.status.success(),
        "publisher 1 failed: {:?}",
        String::from_utf8_lossy(&p1.stderr)
    );

    // Second peer: the re-armed listener must accept it. The
    // publisher's own connect-with-retry (~1s) absorbs the gap
    // between peer-1 EOF and the subscriber re-reaching accept.
    let p2 = Command::new(&pub_bin).output().expect("run publisher 2");
    assert!(
        p2.status.success(),
        "publisher 2 failed (listener did not re-arm?): {:?}",
        String::from_utf8_lossy(&p2.stderr)
    );

    let out = sub.wait().and_then(|_| {
        use std::io::Read;
        let mut s = String::new();
        sub.stdout.take().unwrap().read_to_string(&mut s)?;
        Ok(s)
    });
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sock);
    let stdout = out.expect("collect subscriber stdout");
    assert!(
        stdout.contains("got=1"),
        "first delivery missing.\nstdout: {:?}",
        stdout
    );
    assert!(
        stdout.contains("got=2"),
        "second delivery missing — listener did not re-arm after \
         peer EOF.\nstdout: {:?}",
        stdout
    );
}
