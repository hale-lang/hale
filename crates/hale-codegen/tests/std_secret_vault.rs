//! GH #989 (stdlib half): `std::secret::Credential`'s `vault:` source.
//!
//! Unlike `env_var`/`key_file`, which resolve once at `birth` and are
//! pinned by `tests/hale/std_secret_test.hl`, a vault source resolves
//! fresh on every privileged call and needs an env var set on the
//! child process before it starts (`HALE_VAULT_DIR`/`HALE_VAULT_ADDR`/
//! `HALE_VAULT_TOKEN`) — exactly what a `.hl` fixture run through
//! `hale test` cannot arrange for itself. Each outcome below is a
//! separate `Command::env(...)`-scoped child, never the test
//! process's own environment.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
fn main() {
    let c = std::secret::Credential { vault: "the-secret" };
    println(f"ready={c.ready()}");
    println(f"text={c.reveal_text()}");
}
"#;

fn build() -> PathBuf {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin("hale_test_secret_vault");
    build_executable(&program, &bin).expect("build");
    bin
}

fn run(bin: &PathBuf, envs: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(bin);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn local_mode_present_resolves() {
    let bin = build();
    let dir = std::env::temp_dir().join(format!("hale-vault-present-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("the-secret"), b"s3kr1t").unwrap();

    let out = run(&bin, &[("HALE_VAULT_DIR", dir.to_str().unwrap())]);
    assert!(out.contains("ready=true"), "{out}");
    assert!(out.contains("text=s3kr1t"), "{out}");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn local_mode_absent_fails_closed() {
    let bin = build();
    let dir = std::env::temp_dir().join(format!("hale-vault-absent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // no file written for "the-secret"

    let out = run(&bin, &[("HALE_VAULT_DIR", dir.to_str().unwrap())]);
    assert!(out.contains("ready=false"), "{out}");
    assert!(out.contains("text="), "{out}");
    assert!(!out.contains("text=s"), "must not fabricate a value: {out}");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&bin);
}

/// The default (`HALE_VAULT_DIR` unset) must be a per-user cache
/// path any process can write without root — the same
/// `~/.cache/hale/...` root the toolchain's iris/runtime state
/// already uses — never a system path like `/var/lib/hale/vault`
/// that would make every read fail closed on an ordinary machine.
#[test]
fn local_mode_default_dir_is_under_the_user_cache_root() {
    let bin = build();
    let cache_home = std::env::temp_dir().join(format!("hale-vault-xdg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache_home);
    std::fs::create_dir_all(cache_home.join("hale/vault")).unwrap();
    std::fs::write(cache_home.join("hale/vault/the-secret"), b"from-default-dir").unwrap();

    let out = run(&bin, &[("XDG_CACHE_HOME", cache_home.to_str().unwrap())]);
    assert!(out.contains("ready=true"), "{out}");
    assert!(out.contains("text=from-default-dir"), "{out}");

    let _ = std::fs::remove_dir_all(&cache_home);
    let _ = std::fs::remove_file(&bin);
}

/// Review round 1 (SECURITY): `vault: "../../../etc/passwd"` must
/// not escape the vault root — refused before the path is ever
/// built, same fail-closed shape as any other unresolvable name.
#[test]
fn a_traversal_shaped_name_is_refused_not_escaped() {
    let src = r#"
        fn main() {
            let c = std::secret::Credential { vault: "../../../etc/passwd" };
            println(f"ready={c.ready()}");
            println(f"text={c.reveal_text()}");
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_secret_vault_traversal");
    build_executable(&program, &bin).expect("build");

    let dir = std::env::temp_dir().join(format!("hale-vault-traversal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let out = run(&bin, &[("HALE_VAULT_DIR", dir.to_str().unwrap())]);
    assert!(out.contains("ready=false"), "a traversal-shaped name must not resolve: {out}");
    assert!(out.contains("text="), "{out}");
    assert!(!out.contains("root:"), "must never read /etc/passwd: {out}");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn local_mode_missing_directory_fails_closed() {
    let bin = build();
    let dir = std::env::temp_dir().join(format!("hale-vault-nodir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // guaranteed absent

    let out = run(&bin, &[("HALE_VAULT_DIR", dir.to_str().unwrap())]);
    assert!(out.contains("ready=false"), "a missing vault directory is not a crash: {out}");
}

/// A tiny server for one test: `vault:` resolves FRESH on every
/// privileged call, never cached, so a `.hl` program that calls
/// `ready()` and then `reveal_text()` sends TWO requests, not one.
/// Answers up to `n` connections, checking the Authorization header
/// on each (403 if it does not carry `want_token`), then stops. Runs
/// on its own thread so the test can connect while it is "listening".
fn multi_shot_server(want_token: &str, status: u16, body: &'static str, n: usize) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let want_token = want_token.to_string();
    let handle = std::thread::spawn(move || {
        for _ in 0..n {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let read_n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..read_n]);
            let authorized = req.contains(&format!("Bearer {want_token}"));
            let (resp_status, resp_body) = if authorized {
                (status, body)
            } else {
                (403, "")
            };
            let resp = format!(
                "HTTP/1.1 {resp_status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                resp_body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (port, handle)
}

#[test]
fn http_mode_present_resolves() {
    let bin = build();
    let (port, handle) = multi_shot_server("hosttok", 200, "s3kr1t-over-http", 2);

    let out = run(
        &bin,
        &[
            ("HALE_VAULT_ADDR", &format!("http://127.0.0.1:{port}")),
            ("HALE_VAULT_TOKEN", "hosttok"),
        ],
    );
    handle.join().unwrap();
    assert!(out.contains("ready=true"), "{out}");
    assert!(out.contains("text=s3kr1t-over-http"), "{out}");

    let _ = std::fs::remove_file(&bin);
}

#[test]
fn http_mode_wrong_token_fails_closed() {
    let bin = build();
    let (port, handle) = multi_shot_server("hosttok", 200, "s3kr1t-over-http", 2);

    let out = run(
        &bin,
        &[
            ("HALE_VAULT_ADDR", &format!("http://127.0.0.1:{port}")),
            ("HALE_VAULT_TOKEN", "wrong-token"),
        ],
    );
    handle.join().unwrap();
    assert!(out.contains("ready=false"), "a wrong per-host token must not resolve: {out}");

    let _ = std::fs::remove_file(&bin);
}

#[test]
fn http_mode_missing_token_fails_closed_without_a_request() {
    // No server at all — if the client tried to connect despite the
    // missing token, this would hang or error differently than a
    // clean "ready=false".
    let bin = build();
    let out = run(&bin, &[("HALE_VAULT_ADDR", "http://127.0.0.1:1")]);
    assert!(out.contains("ready=false"), "{out}");
    assert!(out.contains("text="), "{out}");
}

#[test]
fn http_mode_unreachable_address_fails_closed() {
    let bin = build();
    // Port 1 on loopback: nothing listens there in a test sandbox.
    let out = run(
        &bin,
        &[
            ("HALE_VAULT_ADDR", "http://127.0.0.1:1"),
            ("HALE_VAULT_TOKEN", "hosttok"),
        ],
    );
    assert!(out.contains("ready=false"), "an unreachable vault is not a crash: {out}");
}

/// `env_var` wins over `vault` when both are set — same precedence
/// `Signer` already documents between `env_var` and `key_file`.
#[test]
fn env_var_wins_over_vault() {
    let src = r#"
        fn main() {
            let c = std::secret::Credential { env_var: "THE_ENV_SECRET", vault: "the-secret" };
            println(f"text={c.reveal_text()}");
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_secret_vault_precedence");
    build_executable(&program, &bin).expect("build");

    let dir = std::env::temp_dir().join(format!("hale-vault-precedence-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("the-secret"), b"from-vault").unwrap();

    let out = run(
        &bin,
        &[
            ("THE_ENV_SECRET", "from-env"),
            ("HALE_VAULT_DIR", dir.to_str().unwrap()),
        ],
    );
    assert!(out.contains("text=from-env"), "env_var must win: {out}");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&bin);
}
