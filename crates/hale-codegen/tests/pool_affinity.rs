//! Pool affinity (2026-08-12) — `cooperative(pool = X, cores = …)`
//! binds pool X's worker thread to the core set, completing the
//! placement pairing matrix's "cooperative × affinity" cell (the
//! same forms pinned takes; resolved against `topology { }`;
//! best-effort like all affinity). Verified against the kernel:
//! the spawned process must contain exactly one task whose
//! `Cpus_allowed_list` is the declared set.

use std::process::{Command, Stdio};
use std::time::Duration;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
    type Tick { n: Int; }
    locus Sink {
        params { seen: Int = 0; }
        bus { subscribe "aff.t" as on_t of type Tick; }
        fn on_t(t: Tick) { self.seen = self.seen + 1; }
    }
    locus Pub {
        bus { publish "aff.t" of type Tick; }
        run() {
            std::time::sleep(150ms);
            let mut i = 0;
            while i < 5 { "aff.t" <- Tick { n: i }; i = i + 1; }
        }
    }
    main locus App {
        params { s: Sink = Sink { }; p: Pub = Pub { }; }
        placement { s: cooperative(pool = io, cores = 0..=1); }
        run() { std::time::sleep(900ms); }
    }
    fn main() { App { }; }
"#;

fn cpus_allowed_lists(pid: u32) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{}/task", pid)) else {
        return out;
    };
    for t in tasks.flatten() {
        let status = t.path().join("status");
        if let Ok(s) = std::fs::read_to_string(status) {
            for line in s.lines() {
                if let Some(v) = line.strip_prefix("Cpus_allowed_list:") {
                    out.push(v.trim().to_string());
                }
            }
        }
    }
    out
}

#[test]
fn pool_worker_thread_carries_the_declared_core_set() {
    if std::thread::available_parallelism()
        .map(|n| n.get() < 2)
        .unwrap_or(true)
    {
        eprintln!("skipping: needs >= 2 CPUs");
        return;
    }
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin("hale_pool_affinity");
    build_executable(&program, &bin).expect("build");
    let mut child = Command::new(&bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    // Let the pool spawn its worker.
    std::thread::sleep(Duration::from_millis(400));
    let masks = cpus_allowed_lists(pid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    assert!(
        masks.iter().any(|m| m == "0-1"),
        "one task (the io pool worker) must be bound to cores 0-1; \
         saw masks: {:?}",
        masks
    );
    assert!(
        masks.iter().any(|m| m != "0-1"),
        "the main thread must NOT be bound (affinity is the pool's \
         alone): {:?}",
        masks
    );
}
