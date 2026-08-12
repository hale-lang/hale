//! Downstream handoff P26 (2026-08-12) — the observation segment
//! header carries the topology model's identity (`shape_hash`,
//! u64 at 0x80, proto 0.2). A consumer joining a live manifest
//! against a source-derived artifact can now establish the RUNNING
//! binary was built from the model it compares against — a fact
//! about the process, not a guess from file digests. The CLI
//! computes the value from the same bundle it typechecks;
//! `shape_hash` excludes provenance, so moving code (or editing a
//! comment) must not move it, while any model change must.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn build(dir: &Path, src: &str, tag: &str) -> PathBuf {
    let seed = dir.join(tag);
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&seed)
        .output()
        .expect("run hale build");
    assert!(
        out.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    seed.join(tag)
}

/// Run the binary under LOTUS_OBS=1 and read the header's
/// model_hash field (u64 at 0x80) from the live segment.
fn segment_model_hash(bin: &Path) -> u64 {
    let mut child = Command::new(bin)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    let shm = format!("/dev/shm/hale-obs-{}", pid);
    let mut bytes = None;
    for _ in 0..60 {
        std::thread::sleep(Duration::from_millis(25));
        if let Ok(b) = std::fs::read(&shm) {
            if b.len() > 0x88 {
                bytes = Some(b);
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let b = bytes.expect("obs segment appeared");
    u64::from_le_bytes(b[0x80..0x88].try_into().unwrap())
}

const BASE: &str = r#"
type Tick { n: Int = 0; }
topic Ticks { payload: Tick; }
locus W {
    bus { subscribe Ticks as on_t; }
    fn on_t(t: Tick) { }
}
locus P {
    bus { publish Ticks; }
    run() { Ticks <- Tick { n: 1 }; }
}
fn main() {
    W { };
    P { };
    std::time::sleep(600ms);
}
"#;

#[test]
fn model_identity_is_stamped_and_tracks_the_model() {
    let dir = std::env::temp_dir()
        .join(format!("hale_obsmh_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let a = build(&dir, BASE, "a");
    let h_a = segment_model_hash(&a);
    assert_ne!(h_a, 0, "the header carries a stamped model identity");

    // Comment-only edit: provenance moves, the model does not.
    let commented = format!("// a comment that moves every span\n{}", BASE);
    let b = build(&dir, &commented, "b");
    let h_b = segment_model_hash(&b);
    assert_eq!(
        h_a, h_b,
        "a comment-only rebuild keeps the model identity"
    );

    // Adding a locus changes the model.
    let grown = format!("{}\nlocus Extra {{ params {{ n: Int = 0; }} }}\n", BASE);
    let c = build(&dir, &grown, "c");
    let h_c = segment_model_hash(&c);
    assert_ne!(h_a, h_c, "adding a locus changes the model identity");

    let _ = std::fs::remove_dir_all(&dir);
}
