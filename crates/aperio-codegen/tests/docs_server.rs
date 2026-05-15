//! m92 — Phase 5 capstone integration test.
//!
//! Sets up a temp directory with two known `.md` files,
//! launches examples/docs-server/main.ap pointed at it,
//! and verifies via real HTTP requests:
//!
//!   - `GET /`            → 200 + HTML index listing both files
//!   - `GET /alpha.md`    → 200 + rendered HTML body
//!   - `GET /missing.md`  → 404
//!   - `GET /../etc/passwd` → 404 (path-traversal rejection)

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

fn build_docs_server() -> PathBuf {
    let src_path = examples_dir().join("docs-server").join("main.ap");
    let src = std::fs::read_to_string(&src_path).expect("read example");
    let program = aperio_syntax::parse_source(&src).expect("parse example");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_docs_server_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    build_executable(&program, &bin).expect("build example");
    bin
}

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "aperio_docs_fixture_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir(&p).expect("mkdir");
    p
}

fn connect_with_retry(port: u16) -> TcpStream {
    for _ in 0..100 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            return s;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("server never came up on port {}", port);
}

fn http_get(port: u16, path: &str) -> String {
    let mut sock = connect_with_retry(port);
    sock.write_all(format!("GET {} HTTP/1.1\r\n\r\n", path).as_bytes())
        .expect("client write");
    let mut buf = Vec::new();
    let _ = sock.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).to_string()
}

#[test]
fn docs_server_serves_index_and_one_doc_and_404s_correctly() {
    // Fixture: a temp docs directory with two markdown files
    // and one non-md file (which the index should skip).
    let dir = unique_dir("served");
    std::fs::write(
        dir.join("alpha.md"),
        "# Alpha\n\nThe first document.\n\n## Section\n\nMore content.\n",
    )
    .expect("write alpha");
    std::fs::write(
        dir.join("beta.md"),
        "# Beta\n\nSecond doc.\n",
    )
    .expect("write beta");
    std::fs::write(dir.join("not-md.txt"), "ignore me\n")
        .expect("write txt");

    let port = pick_free_port();
    let bin = build_docs_server();
    let child = Command::new(&bin)
        .arg(port.to_string())
        .arg(&dir)
        .arg("4") // index + alpha + missing + traversal = 4 requests
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn docs-server");

    // Index request.
    let index = http_get(port, "/");
    assert!(
        index.starts_with("HTTP/1.1 200 OK\r\n"),
        "index status: {:?}",
        index
    );
    assert!(
        index.contains("<h1>Aperio Docs</h1>"),
        "index missing heading; got: {:?}",
        index
    );
    assert!(
        index.contains("href=\"/alpha.md\""),
        "index missing alpha link; got: {:?}",
        index
    );
    assert!(
        index.contains("href=\"/beta.md\""),
        "index missing beta link; got: {:?}",
        index
    );
    assert!(
        !index.contains("not-md.txt"),
        "index leaked non-md file; got: {:?}",
        index
    );

    // One rendered document.
    let alpha = http_get(port, "/alpha.md");
    assert!(
        alpha.starts_with("HTTP/1.1 200 OK\r\n"),
        "alpha status: {:?}",
        alpha
    );
    assert!(
        alpha.contains("<h1>Alpha</h1>"),
        "alpha missing rendered h1; got: {:?}",
        alpha
    );
    assert!(
        alpha.contains("<h2>Section</h2>"),
        "alpha missing rendered h2; got: {:?}",
        alpha
    );
    assert!(
        alpha.contains("<p>The first document.</p>"),
        "alpha missing rendered paragraph; got: {:?}",
        alpha
    );

    // Missing doc → 404.
    let missing = http_get(port, "/nope.md");
    assert!(
        missing.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "missing status: {:?}",
        missing
    );

    // Traversal attempt → 404 (rejected by __safe_path).
    let traversal = http_get(port, "/../../etc/passwd");
    assert!(
        traversal.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "traversal status: {:?}",
        traversal
    );

    let out = child.wait_with_output().expect("wait child");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("docs-server: GET /"),
        "missing request log; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("docs-server: GET /alpha.md"),
        "missing alpha log; got: {:?}",
        stdout
    );
}
