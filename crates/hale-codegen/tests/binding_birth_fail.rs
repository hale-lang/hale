//! GH #227: a bus binding that cannot be realized is a birth
//! failure of the declaring locus — never print-and-continue.
//!
//! The publish contract (spec/semantics.md): `T <- v` succeeding
//! means the broker accepted the message under the bound
//! transport's guarantee. A broker that knows it cannot honor the
//! guarantee (transport never opened) must refuse at birth, not
//! accept-and-drop. Pre-#227 the runtime perror'd to stderr and
//! left a dead entry in the remote table, so every publish
//! "succeeded" while fanout silently skipped the slot.
//!
//! Failure injection is platform-independent: an AF_UNIX socket
//! path longer than sizeof(sun_path) (~108 bytes) fails with
//! ENAMETOOLONG on every platform (no macOS hardware needed —
//! the reviewer's original repro was `socket: Protocol not
//! supported` on Darwin, but any create-path failure exercises
//! the same routing), and a path inside a nonexistent directory
//! fails bind with ENOENT.

use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_binding_birth_fail_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// An AF_UNIX path guaranteed to exceed sun_path (108 bytes incl
/// NUL) — fails addr setup with ENAMETOOLONG before any syscall.
fn overlong_socket_path() -> String {
    format!("/tmp/hale-227-{}", "x".repeat(120))
}

fn unique_socket_path(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}/hale-227-{}-{}-{}.sock",
        std::env::temp_dir().display(),
        tag,
        std::process::id(),
        nanos
    )
}

#[test]
fn connect_role_unrealizable_binding_fails_birth() {
    let src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                Evt <- T {{ n: 1 }};
                println("sent ok");
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        overlong_socket_path()
    );
    let bin = build("connect_overlong", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "binding could not be realized but the process exited 0 \
         (the pre-#227 silent-drop behavior).\nstdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    assert!(
        !stdout.contains("sent ok"),
        "publisher ran (and 'succeeded') past a dead binding.\nstdout: {:?}",
        stdout
    );
    assert!(
        stderr.contains("could not be realized") && stderr.contains("evt"),
        "expected the structural-failure shape naming the subject.\nstderr: {:?}",
        stderr
    );
}

#[test]
fn listen_role_bind_failure_fails_birth_synchronously() {
    // Pre-#227 the LISTEN transport was opened on the reader
    // thread, so a bind failure killed the thread silently and
    // the app ran forever receiving nothing. bind/listen now
    // happen synchronously at registration: a path inside a
    // nonexistent directory must refuse the boot.
    let src = r#"
        type T { n: Int = 0; }
        topic Evt { payload: T; subject: "evt"; }
        locus Sub {
            bus { subscribe Evt as on_evt; }
            fn on_evt(t: T) { }
        }
        main locus App {
            bindings { Evt: unix("/nonexistent-hale-227-dir/x.sock", role: listen); }
            run() {
                println("subscriber alive");
            }
        }
        fn main() { App { }; Sub { }; }
    "#;
    let bin = build("listen_enoent", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "listen-role bind failure did not fail the boot.\nstdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    assert!(
        !stdout.contains("subscriber alive"),
        "app ran past a dead listen binding.\nstdout: {:?}",
        stdout
    );
    assert!(
        stderr.contains("could not be realized"),
        "expected the structural-failure shape.\nstderr: {:?}",
        stderr
    );
}

#[test]
fn listen_role_binding_without_peer_boots_and_exits_cleanly() {
    // The inverse guard: moving bind/listen onto the boot path
    // must NOT reintroduce the pre-m59 boot hang, and teardown
    // must be able to join a reader thread parked in accept()
    // when no peer ever connected (destroy_all now shuts the
    // listener down; pre-#227 this join hung forever).
    let sock = unique_socket_path("noperr");
    let src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Sub {{
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{ }}
        }}
        main locus App {{
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                println("subscriber alive");
            }}
        }}
        fn main() {{ App {{ }}; Sub {{ }}; }}
    "#,
        sock
    );
    let bin = build("listen_no_peer", &src);
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    // Poll with a hard cap so a teardown-join hang fails the
    // test instead of wedging the suite.
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break Some(s),
            None if Instant::now() > deadline => break None,
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_file(&sock);
        panic!(
            "subscriber with an unconnected listen binding failed \
             to exit within 10s — reader-thread join hang at teardown"
        );
    };
    let out = child.wait_with_output().expect("collect output");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        status.success(),
        "clean listen binding must boot fine without a peer.\nstderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("subscriber alive"),
        "run() didn't fire.\nstdout: {:?}",
        stdout
    );
}

#[test]
fn bus_config_route_open_failure_fails_boot() {
    // Same contract for LOTUS_BUS_CONFIG-requested routes: no
    // route that was asked for may silently not exist.
    let src = r#"
        type T { n: Int = 0; }
        topic Evt { payload: T; subject: "evt"; }
        locus Sub {
            bus { subscribe Evt as on_evt; }
            fn on_evt(t: T) { }
        }
        main locus App {
            bus { publish Evt; }
            run() {
                Evt <- T { n: 1 };
                println("sent ok");
            }
        }
        fn main() { App { }; Sub { }; }
    "#;
    let bin = build("config_route", src);
    let mut cfg = std::env::temp_dir();
    cfg.push(format!(
        "hale-227-cfg-{}-{}.conf",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(
        &cfg,
        format!("evt = unix://{} : connect\n", overlong_socket_path()),
    )
    .expect("write config");
    let out = Command::new(&bin)
        .env("LOTUS_BUS_CONFIG", &cfg)
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&cfg);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "config route could not be realized but the process exited 0.\nstdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    assert!(
        !stdout.contains("sent ok"),
        "publisher ran past a dead config route.\nstdout: {:?}",
        stdout
    );
    assert!(
        stderr.contains("could not be realized"),
        "expected the structural-failure shape.\nstderr: {:?}",
        stderr
    );
}

#[test]
fn bus_config_udp_parse_failure_fails_boot() {
    let src = r#"
        type T { n: Int = 0; }
        topic Evt { payload: T; subject: "evt"; }
        locus Sub {
            bus { subscribe Evt as on_evt; }
            fn on_evt(t: T) { }
        }
        main locus App {
            run() { println("subscriber alive"); }
        }
        fn main() { App { }; Sub { }; }
    "#;
    let bin = build("config_udp_bad", src);
    let mut cfg = std::env::temp_dir();
    cfg.push(format!(
        "hale-227-udpcfg-{}-{}.conf",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&cfg, "evt = udp://not.an.ip:9 : listen\n")
        .expect("write config");
    let out = Command::new(&bin)
        .env("LOTUS_BUS_CONFIG", &cfg)
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&cfg);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "unparseable udp route must fail the boot.\nstderr: {:?}",
        stderr
    );
    assert!(
        stderr.contains("could not be realized"),
        "expected the structural-failure shape.\nstderr: {:?}",
        stderr
    );
}
