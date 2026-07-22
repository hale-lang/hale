//! GH #233 steps 3-4: connection loss on a connect-role binding
//! is structural. Default (no handler): the process exits with
//! the loss diagnostic — never accept-and-drop. With
//! `on_failure(t: std::bus::UnixTransport, err: ClosureViolation)`
//! on the main locus calling `restart (t);`, the runtime re-runs
//! the connect-with-retry and publishing resumes — reconnection
//! as supervision policy, not transport feature (F.37).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_loss_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// transport_driver.c listener: accepts one peer, recvs ONE
/// message, exits — which closes the connection and induces
/// EPIPE on the publisher's next send.
fn build_peer_driver(tag: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_loss_peer_{}", tag));
    let status = Command::new("clang")
        .arg(manifest.join("tests").join("transport_driver.c"))
        .arg(manifest.join("runtime").join("lotus_arena.c"))
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building peer driver");
    bin
}

fn unique_socket_path(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}/hale-233-loss-{}-{}-{}.sock",
        std::env::temp_dir().display(),
        tag,
        std::process::id(),
        nanos
    )
}

#[test]
fn connection_loss_without_handler_is_structural_exit() {
    let sock = unique_socket_path("fatal");
    let src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                Evt <- T {{ n: 1 }};
                std::time::sleep(400ms);
                Evt <- T {{ n: 2 }};
                std::time::sleep(300ms);
                println("survived loss");
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let bin = build("fatal", &src);
    let driver = build_peer_driver("fatal");
    let listener = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn listener peer");
    let out = Command::new(&bin).output().expect("run publisher");
    let _ = listener.wait_with_output();
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "connection loss with no restart policy must exit non-zero.\n\
         stdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    assert!(
        !stdout.contains("survived loss"),
        "program ran past a lost binding with no policy.\nstdout: {:?}",
        stdout
    );
    assert!(
        stderr.contains("lost its connection") && stderr.contains("evt"),
        "expected the loss diagnostic naming the subject.\nstderr: {:?}",
        stderr
    );
}

#[test]
fn on_failure_restart_reconnects_and_resumes() {
    let sock = unique_socket_path("restart");
    let src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            on_failure(t: std::bus::UnixTransport, err: ClosureViolation) {{
                println("[sup] link lost — restarting");
                restart (t);
            }}
            run() {{
                Evt <- T {{ n: 1 }};
                std::time::sleep(500ms);
                Evt <- T {{ n: 2 }};
                std::time::sleep(500ms);
                Evt <- T {{ n: 3 }};
                std::time::sleep(200ms);
                println("recovered");
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let bin = build("restart", &src);
    let driver = build_peer_driver("restart");

    // Peer 1: takes the first message, exits → EPIPE on send 2.
    let peer1 = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn peer 1");
    let mut publisher = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn publisher");
    let _ = peer1.wait_with_output();
    // Peer 2 binds while the publisher is between send 2 (loss)
    // and send 3; the reconnect's connect-with-retry (~1s)
    // absorbs the gap.
    let peer2 = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn peer 2");
    let pub_out = publisher.wait_with_output().expect("publisher output");
    let peer2_out = peer2.wait_with_output().expect("peer 2 output");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&pub_out.stdout);
    let stderr = String::from_utf8_lossy(&pub_out.stderr);
    assert!(
        pub_out.status.success(),
        "publisher with a restart policy must survive the loss.\n\
         stdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("[sup] link lost") && stdout.contains("recovered"),
        "expected the supervision handler + recovery to run.\nstdout: {:?}",
        stdout
    );
    assert!(
        !peer2_out.stdout.is_empty(),
        "peer 2 received nothing — reconnect did not resume delivery.\n\
         publisher stdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
}
