//! m76 + m78b: Phase 1 capstone — integration test for
//! examples/io-demo, parameterized via env + argv (m78
//! follow-up).
//!
//! Each test picks a free localhost port and unique /tmp
//! paths, passes them to the example via argv[1] +
//! APERIO_IO_DEMO_{CONFIG,LOG}_PATH. Parallel cargo test
//! threads no longer collide on a hardcoded port — the
//! example consumes std::env / std::str::parse_int from
//! the v1.x stdlib to wire its parameters.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("examples");
    p
}

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "aperio_io_demo_{}_{}_{}.tmp",
        tag,
        std::process::id(),
        nanos
    ));
    p
}

fn wait_until_listening(port: u16) -> bool {
    for _ in 0..50 {
        if let Ok(s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            drop(s);
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

struct Demo {
    port: u16,
    config_path: PathBuf,
    log_path: PathBuf,
    bin: PathBuf,
}

impl Demo {
    fn build(tag: &str) -> Self {
        let mut src_path = examples_dir();
        src_path.push("io-demo");
        src_path.push("main.ap");
        let source = std::fs::read_to_string(&src_path).expect("read source");
        let program = aperio_syntax::parse_source(&source).expect("parse");
        let mut bin = std::env::temp_dir();
        bin.push(format!("aperio_io_demo_bin_{}_{}", std::process::id(), tag));
        build_executable(&program, &bin).expect("build");
        Self {
            port: pick_free_port(),
            config_path: unique_path(&format!("{}_config", tag)),
            log_path: unique_path(&format!("{}_log", tag)),
            bin,
        }
    }

    fn run(&self) -> (String, String, std::process::ExitStatus) {
        let listener_proc = Command::new(&self.bin)
            .arg(self.port.to_string())
            .env("APERIO_IO_DEMO_CONFIG_PATH", &self.config_path)
            .env("APERIO_IO_DEMO_LOG_PATH", &self.log_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn io-demo");

        assert!(
            wait_until_listening(self.port),
            "io-demo never bound to 127.0.0.1:{}",
            self.port
        );

        let mut sock = std::net::TcpStream::connect(("127.0.0.1", self.port))
            .expect("connect to demo");
        let _ = sock.write_all(b"hello\n");
        drop(sock);

        let out = listener_proc.wait_with_output().expect("listener wait");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        (stdout, stderr, out.status)
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.bin);
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_file(&self.log_path);
    }
}

#[test]
fn io_demo_default_config_writes_default_payload() {
    let demo = Demo::build("default");
    // No config file seeded.
    let (stdout, stderr, status) = demo.run();

    assert!(
        status.success(),
        "io-demo exited non-zero: {:?}\nstderr: {}",
        status,
        stderr
    );
    assert!(
        stdout.contains("config: none, using default"),
        "expected default-config message; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains(&format!("io-demo: listening on 127.0.0.1:{}", demo.port)),
        "expected listening diagnostic with picked port; got: {:?}",
        stdout
    );
    let log = std::fs::read_to_string(&demo.log_path)
        .expect("default-cycle: log should exist");
    assert_eq!(log, "default visit\n", "got: {:?}", log);
    demo.cleanup();
}

#[test]
fn io_demo_with_config_writes_config_payload() {
    let demo = Demo::build("with_config");
    std::fs::write(&demo.config_path, "configured visit payload\n")
        .expect("seed config");
    let (stdout, stderr, status) = demo.run();

    assert!(
        status.success(),
        "io-demo exited non-zero: {:?}\nstderr: {}",
        status,
        stderr
    );
    assert!(
        stdout.contains(&format!(
            "config: loaded from {}",
            demo.config_path.to_str().unwrap()
        )),
        "expected loaded-config message; got: {:?}",
        stdout
    );
    let log = std::fs::read_to_string(&demo.log_path)
        .expect("with-config cycle: log should exist");
    assert_eq!(log, "configured visit payload\n", "got: {:?}", log);
    demo.cleanup();
}

#[test]
fn io_demo_falls_back_to_default_port_on_garbage_argv() {
    // Pass a non-numeric argv[1]; parse_int returns 0; the
    // example sees `parsed > 0` is false and keeps the default
    // (9876). To avoid collision with any hardcoded-port test
    // we set the demo's APERIO_IO_DEMO_PORT env... wait, the
    // example doesn't read env for port. Skip this case OR
    // accept that we're testing argv-fallback specifically and
    // it'll bind 9876.
    //
    // Compromise: build with garbage argv[1], let the binary
    // bind its DEFAULT port (9876), connect to that. We
    // serialize this test against the default-port collision
    // by NOT running other 9876-bound tests in this file —
    // the other two tests use unique picked ports.
    let mut src_path = examples_dir();
    src_path.push("io-demo");
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_io_demo_bin_garbage_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");

    let log_path = unique_path("garbage_log");

    let listener_proc = Command::new(&bin)
        .arg("not-a-port")
        .env("APERIO_IO_DEMO_LOG_PATH", &log_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    assert!(
        wait_until_listening(9876),
        "io-demo didn't bind default port 9876"
    );

    let _ = std::net::TcpStream::connect(("127.0.0.1", 9876));
    let out = listener_proc.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&log_path);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("io-demo: listening on 127.0.0.1:9876"),
        "expected default-port fallback; got: {:?}",
        stdout
    );
}
