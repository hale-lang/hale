//! m76 + m78b: Phase 1 capstone — integration test for
//! examples/io-demo, parameterized via env + argv (m78
//! follow-up).
//!
//! Each test takes a reserved loopback address and unique /tmp
//! paths, passes them to the example via argv[1] +
//! HALE_IO_DEMO_{CONFIG,LOG}_PATH. The example reads a port from
//! argv and binds `127.0.0.1` as written, and a port of 0 means
//! its default — so it cannot bind port 0 for the test to read
//! back. Instead the test holds a [`child::Reserved`] port and
//! builds the example with its host swapped for the reserved one:
//! a port drawn and released could be bound by another test of a
//! parallel run before the example binds it.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "hale_io_demo_{}_{}_{}.tmp",
        tag,
        std::process::id(),
        nanos
    ));
    p
}

fn wait_until_listening(addr: impl std::net::ToSocketAddrs + Copy) -> bool {
    for _ in 0..50 {
        if let Ok(s) = std::net::TcpStream::connect(addr) {
            drop(s);
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

struct Demo {
    /// Where the example listens, held for the test's length.
    at: child::Reserved,
    config_path: PathBuf,
    log_path: PathBuf,
    bin: PathBuf,
}

impl Demo {
    fn build(tag: &str) -> Self {
        let mut src_path = examples_dir();
        src_path.push("io-demo");
        src_path.push("main.hl");
        let source = std::fs::read_to_string(&src_path).expect("read source");
        let at = child::Reserved::new();
        assert!(source.contains("host: \"127.0.0.1\","), "io-demo binds 127.0.0.1 as written");
        let source = source.replace("127.0.0.1", &at.host());
        let bin = harness::unique_bin(&format!("hale_io_demo_bin_{}_{}", std::process::id(), tag));
        build_opts::build_source(&source, &bin, &build_opts::options()).expect("build");
        Self {
            at,
            config_path: unique_path(&format!("{}_config", tag)),
            log_path: unique_path(&format!("{}_log", tag)),
            bin,
        }
    }

    fn run(&self) -> (String, String, std::process::ExitStatus) {
        let listener_proc = Command::new(&self.bin)
            .arg(self.at.port().to_string())
            .env("HALE_IO_DEMO_CONFIG_PATH", &self.config_path)
            .env("HALE_IO_DEMO_LOG_PATH", &self.log_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn io-demo");

        // The Listener's default `max_accepts: 1` means a single
        // accepted connection advances the demo past its accept
        // loop and into the log-write + exit path. `wait_until_listening`'s
        // own probe IS that connection — the demo doesn't read its
        // payload anyway (default `__default_on_connection` just
        // prints the fd and dissolves the Stream). A previous
        // version of this helper did a second connect+write here
        // and raced the listener's exit on slow CI, getting
        // ECONNRESET. The probe alone is sufficient.
        assert!(wait_until_listening(self.at.addr), "io-demo never bound to {}", self.at.addr);

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
        stdout.contains(&format!("io-demo: listening on {}", demo.at.addr)),
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
    // example sees `parsed > 0` is false and keeps its default
    // (9876) — the default is what is under test, so this one
    // binds a fixed port. Nothing else in the suite names 9876,
    // and the kernel never hands it out for a bind of port 0
    // (it is below the ephemeral range).
    let mut src_path = examples_dir();
    src_path.push("io-demo");
    src_path.push("main.hl");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let bin = harness::unique_bin(&format!("hale_io_demo_bin_garbage_{}", std::process::id()));
    build_opts::build_source(&source, &bin, &build_opts::options()).expect("build");

    let log_path = unique_path("garbage_log");

    let listener_proc = Command::new(&bin)
        .arg("not-a-port")
        .env("HALE_IO_DEMO_LOG_PATH", &log_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    assert!(
        wait_until_listening(("127.0.0.1", 9876)),
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
