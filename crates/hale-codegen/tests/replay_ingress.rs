//! GH #296 phase 5b — hermetic ingress: recorded wire input is
//! INJECTED under replay, and is a feedable tape under feed mode.
//!
//! What this holds the runtime to:
//!
//!   1. **The recording captures the wire form.** A listen-binding
//!      subscriber records each received message's verbatim wire
//!      bytes (ingress-flagged) — not just struct metadata.
//!   2. **Strict replay is hermetic and equivalent.** Replaying the
//!      subscriber WITHOUT the publisher (and without the socket):
//!      bound transports never open, the injector re-dispatches the
//!      tape with each delivery's RECORDED identity, and the run
//!      produces byte-identical stdout with ZERO divergences — the
//!      per-consumer order enforcement matched every injected
//!      delivery to its recorded consume.
//!   3. **Feed mode re-executes changed code on the same inputs.**
//!      A DIFFERENT program (same subject, different handler) fed
//!      the same tape processes the recorded payload values — the
//!      backtesting contract: same inputs, changed code, live
//!      everything else, and an exit report that says what was fed.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::{build_executable_with_options, BuildOptions};

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(name.to_string(), &program);
    let bundle = hale_types::Bundle::new(programs);
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let bin = harness::unique_bin(&format!("hale_test_ingress_{}", name));
    let options = BuildOptions {
        model_hash: Some(model_hash),
        ..BuildOptions::default()
    };
    build_executable_with_options(&program, &bin, &[], &options)
        .expect("build");
    bin
}

fn unique_socket_path() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}/hale-296-ingress-{}-{}.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    )
}

fn rec_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale_ingress_{}_{}.halerec",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&p);
    p
}

fn sub_src(sock: &str) -> String {
    format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Sub {{
            params {{ seen: Int = 0; total: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                self.total = self.total + t.n;
                println("got=", t.n);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.sub.seen < 3 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 120 {{
                        std::process::exit(3);
                    }}
                }}
                println("total=", self.sub.total);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    )
}

fn pub_src(sock: &str) -> String {
    format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            run() {{
                Evt <- T {{ n: 7 }};
                std::time::sleep(50ms);
                Evt <- T {{ n: 11 }};
                std::time::sleep(50ms);
                Evt <- T {{ n: 23 }};
                std::time::sleep(200ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    )
}

/// Record a listener run fed by a real connecting publisher, and
/// return (recording path, the listener's recorded stdout).
fn record_session(
    tag: &str,
    sub_bin: &std::path::Path,
    pub_bin: &std::path::Path,
) -> (std::path::PathBuf, String) {
    let rec = rec_path(tag);
    let sub = Command::new(sub_bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn recorded subscriber");
    // The publisher's connect-with-retry rides out the listener's
    // boot; run it to completion.
    let p = Command::new(pub_bin).output().expect("run publisher");
    assert!(
        p.status.success(),
        "publisher failed: {}",
        String::from_utf8_lossy(&p.stderr)
    );
    let out = sub.wait_with_output().expect("subscriber exit");
    assert!(
        out.status.success(),
        "recorded subscriber failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("got=7")
            && stdout.contains("got=11")
            && stdout.contains("got=23")
            && stdout.contains("total=41"),
        "recorded run did not see the wire traffic:\n{}",
        stdout
    );
    (rec, stdout)
}

#[test]
fn strict_replay_injects_the_recorded_ingress_hermetically() {
    let sock = unique_socket_path();
    let sub_bin = build("sub", &sub_src(&sock));
    let pub_bin = build("pub", &pub_src(&sock));
    let (rec, recorded_stdout) = record_session("strict", &sub_bin, &pub_bin);

    // Replay with NO publisher and NO socket: the bound transport
    // must never open, and the tape must carry the run.
    let _ = std::fs::remove_file(&sock);
    let replayed = Command::new(&sub_bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay subscriber");
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(
        replayed.status.success(),
        "hermetic replay failed; stderr:\n{}",
        err
    );
    // Byte-identical observable behavior, from the tape alone.
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        recorded_stdout,
        "replay stdout differs from the recorded run's"
    );
    // The wire was hermetic and the tape was injected...
    assert!(
        err.contains("hermetic wire") && err.contains("injected"),
        "expected the hermetic-wire report; stderr:\n{}",
        err
    );
    // ...with recorded identities: every injected delivery matched
    // its recorded consume — zero divergences, not merely "ran".
    assert!(
        err.contains("0 divergences"),
        "expected a divergence-free replay; stderr:\n{}",
        err
    );
    // And the listener socket was never created.
    assert!(
        !std::path::Path::new(&sock).exists(),
        "replay must not open the bound transport"
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
}

#[test]
fn feed_mode_runs_changed_code_on_the_recorded_tape() {
    let sock = unique_socket_path();
    let sub_bin = build("feedsub", &sub_src(&sock));
    let pub_bin = build("feedpub", &pub_src(&sock));
    let (rec, _) = record_session("feed", &sub_bin, &pub_bin);

    // A DIFFERENT program: same subject, different handler logic
    // (scales each payload by 10). Model hash differs — feed mode
    // admits it by design.
    let changed = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Scaler {{
            params {{ seen: Int = 0; total: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                self.total = self.total + t.n * 10;
                println("scaled=", t.n * 10);
            }}
        }}
        main locus App {{
            params {{ s: Scaler = Scaler {{ }}; }}
            bindings {{ Evt: unix("{}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.s.seen < 3 {{
                    std::time::sleep(100ms);
                    waited = waited + 1;
                    if waited > 120 {{
                        std::process::exit(3);
                    }}
                }}
                println("scaled_total=", self.s.total);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let changed_bin = build("feedchanged", &changed);

    let _ = std::fs::remove_file(&sock);
    let fed = Command::new(&changed_bin)
        .env("LOTUS_REPLAY_FEED", &rec)
        .output()
        .expect("feed changed program");
    let out = String::from_utf8_lossy(&fed.stdout);
    let err = String::from_utf8_lossy(&fed.stderr);
    assert!(
        fed.status.success(),
        "feed run failed; stderr:\n{}",
        err
    );
    // The changed code processed the RECORDED inputs.
    assert!(
        out.contains("scaled=70")
            && out.contains("scaled=110")
            && out.contains("scaled=230")
            && out.contains("scaled_total=410"),
        "changed program did not process the tape:\n{}",
        out
    );
    // And the exit report says what was fed.
    assert!(
        err.contains("hale feed:") && err.contains("injected"),
        "expected the feed report; stderr:\n{}",
        err
    );
    assert!(
        !std::path::Path::new(&sock).exists(),
        "feed must not open the bound transport"
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&changed_bin);
}
