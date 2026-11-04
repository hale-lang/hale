//! Phase-3 safety net (2026-05-19): hard byte cap on
//! g_bus_payload_arena, the program-lifetime arena a producer with no
//! caller arena falls back to. Reaching the cap stops the process with a
//! report that names the arena and the frames that asked; it used to hand
//! the caller NULL, which most callers never checked, and a capped process
//! died later in a SIGSEGV (hale-lang/voice: an api server, after ~6.5 h).
//!
//! Default cap is 64 MiB; overridable via the
//! LOTUS_BUS_PAYLOAD_ARENA_CAP env var (bytes). The tests set a small cap
//! so a producer depositing once per message reaches it in a few thousand
//! messages instead of hours.

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let bin = harness::unique_bin(&format!("lotus_test_bus_arena_cap_{}", name));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    bin
}

#[test]
fn snapshot_with_tls_routing_skips_global_cap() {
    // 2026-05-19 update: after the __caller_arena threading lands,
    // BytesBuilder snapshot/finish allocations route through the
    // calling fn's arena (via the TLS `lotus_current_caller_arena`)
    // instead of the program-lifetime g_bus_payload_arena. The
    // cap mechanism still exists on the global arena, but the
    // common-case stdlib paths no longer touch it — that's the
    // whole point of the threading work.
    //
    // This test pins that behavior: allocating 100x 1 MiB
    // snapshots with a small global cap should NOT fire the cap
    // diagnostic, because the snapshots land in Parent's
    // (long-lived) locus arena, not the capped global. Parent's
    // arena is unbounded; allocations there are bounded by
    // Parent's lifecycle (still program-lifetime here since
    // Parent IS the program root, but the principle holds at
    // any locus depth).
    let src = r#"
        locus Parent {
            accept(b: std::bytes::BytesBuilder) { }
            on_failure(b: std::bytes::BytesBuilder, err: ClosureViolation) {
                println("absorbed=", err.closure);
            }
            run() {
                let b = std::bytes::BytesBuilder { initial_cap: 1048576 };
                let chunk = std::bytes::from_string(
                    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
                );
                let mut i = 0;
                while i < 50 {
                    b.append(chunk);
                    let snap = b.snapshot() or raise;
                    i = i + 1;
                }
                println("loop done");
            }
        }
        fn main() { Parent { }; }
    "#;
    let bin = build("snapshot_tls", src);
    let output = Command::new(&bin)
        .env("LOTUS_BUS_PAYLOAD_ARENA_CAP", "65536") // 64 KiB
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected clean run — stdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("loop done"),
        "expected loop completion: {:?}",
        stdout
    );
    // The cap diagnostic should NOT fire — TLS routing keeps the
    // allocations out of g_bus_payload_arena.
    assert!(
        !stderr.contains("arena cap hit"),
        "unexpected cap diagnostic — TLS routing should bypass \
         the global arena: {:?}",
        stderr
    );
    assert!(
        !stdout.contains("absorbed=alloc_failed"),
        "no violation expected — TLS-routed snapshot should \
         succeed against Parent's arena: {:?}",
        stdout
    );
}

#[test]
fn default_cap_does_not_fire_for_modest_workload() {
    // The default 64 MiB cap should be ample for any normal
    // test workload. This negative test guards against
    // accidentally setting the default too low and breaking
    // unrelated tests.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 1024 };
            b.append(std::bytes::from_string("hello world"));
            let s = b.finish() or raise;
            println("len=", len(s));
        }
    "#;
    let bin = build("default_cap_safe", src);
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}/{}", stdout, stderr);
    assert!(stdout.contains("len=11"), "got: {:?}", stdout);
    assert!(
        !stderr.contains("arena cap hit"),
        "no cap diagnostic expected: {:?}",
        stderr
    );
}

/// The leak behind hale-lang/voice's crash: a topic bound to a `ws::Hub`
/// carries each payload through the hub's codec, whose `encode` ran in
/// whatever caller arena the publishing thread had published. A free fn
/// that returns leaves none (its scratch, the arena last published, is
/// destroyed, and the TLS cleared with it), so a publish whose payload a
/// free fn built last encoded into the payload arena: one encoded event
/// per publish, never reclaimed, until the cap. The encode now runs in a
/// call arena of its own, freed when the publish's serializer returns.
#[test]
fn a_codec_bound_publish_leaves_nothing_in_the_payload_arena() {
    let src = r#"
        role watcher;

        type Seen { node: String; n: Int; body: String; }
        topic Observed { payload: Seen; subject: "cap.observed"; }

        locus Tokens {
            fn principal(token: String) -> std::api::Principal {
                return std::api::Principal { mode: "bearer", name: "" };
            }
            fn refused() -> String { return "no such token"; }
        }
        locus Grants {
            fn holds(p: std::api::Principal, r: String) -> Bool { return false; }
        }

        fn quoted(s: String) -> String { return "\"" + std::json::escape_string(s) + "\""; }
        fn stamp(t: Time) -> String { return quoted(std::time::iso8601(t)); }

        locus Observer {
            params { seen: Int = 0; }
            bus { publish Observed; }
            fn observe(n: Int) {
                self.seen = self.seen + 1;
                // the free fn is the last call before the publish
                Observed <- Seen {
                    node: "node-1",
                    n: n,
                    body: std::str::repeat("x", 600) + stamp(std::time::from_nanos(1700000000000000000 + n)),
                };
            }
        }

        main locus App {
            params {
                bearer: Tokens = Tokens { };
                grants: Grants = Grants { };
                hub: ws::Hub = ws::Hub { bind: "127.0.0.1:0", principals: self.bearer, roles: self.grants, as: "cap" };
                obs: Observer = Observer { };
            }
            bindings {
                Observed: self.hub requires: [watcher], bound: 8, on_full: drop_old;
            }
            run() {
                let mut i = 0;
                while i < 20000 {
                    self.obs.observe(i);
                    i = i + 1;
                }
                println("observed=", self.obs.seen);
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("codec_encode", src);
    // 20,000 encodes of ~650 bytes: 13 MB, fifty times the cap, had any
    // of them stayed
    let output = Command::new(&bin).env("LOTUS_BUS_PAYLOAD_ARENA_CAP", "262144").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", output.status);
    assert!(stdout.contains("observed=20000"), "stdout:\n{stdout}");
    assert!(!stderr.contains("arena cap hit"), "an encode stayed in the payload arena:\n{stderr}");
}

/// The cap ends the process with its report, at the call that would pass
/// it, rather than handing the caller a NULL it does not check: the
/// program sees no NULL and runs nothing past the cap.
#[test]
fn reaching_the_cap_stops_the_process_with_a_report() {
    let src = r#"
        @ffi("c") fn lotus_bus_payload_arena_deposit_test(n: Int) -> Int;
        fn main() {
            let mut i = 0;
            let mut kept = 0;
            while i < 64 {
                kept = kept + lotus_bus_payload_arena_deposit_test(16384);
                i = i + 1;
            }
            println("past the cap kept=", kept);
        }
    "#;
    let bin = build("cap_stops", src);
    let output = Command::new(&bin).env("LOTUS_BUS_PAYLOAD_ARENA_CAP", "262144").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.signal(),
        Some(6),
        "the cap aborts (SIGABRT), it does not return NULL into a SIGSEGV: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert!(!stdout.contains("past the cap"), "the program ran past the cap:\n{stdout}");
    assert!(
        stderr.contains("lotus: arena cap hit (g_bus_payload_arena): ")
            && stderr.contains("262144 bytes")
            && stderr.contains("the process stops here")
            && stderr.contains("LOTUS_BUS_PAYLOAD_ARENA_CAP"),
        "the report names the arena, its cap and the knob:\n{stderr}"
    );
}
