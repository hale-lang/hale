//! GH #476 Change 8, control arm: a build that stamps NO canonical
//! entity ids leaves the manifest exactly as it was.
//!
//! The stamps come from the CLI, which derives them from the
//! canonical model. Harness callers (`build_executable`, every
//! codegen test, any embedder) pass none — and their manifest rows
//! must keep reading `aux_b == 0`, the pre-existing "nothing here"
//! value, so a consumer written against the old layout is not
//! silently handed numbers that mean something else.

use std::process::{Command, Stdio};
use std::time::Duration;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

const PROG: &str = r#"
type Tick { n: Int = 0; }
topic Ticks { payload: Tick; subject: "unstamped.tick"; }
locus W {
    params { seen: Int = 0; }
    bus { subscribe Ticks as on_t; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}
main locus App {
    params { w: W = W { }; }
    bus { publish Ticks; }
    run() { Ticks <- Tick { n: 1 }; std::time::sleep(600ms); }
}
fn main() { App { }; }
"#;

#[test]
fn a_harness_build_leaves_manifest_entity_ids_zero() {
    let program = hale_syntax::parse_source(PROG).expect("parse");
    let bin = harness::unique_bin("lotus_test_obs_ids_unstamped");
    build_executable(&program, &bin).expect("build");

    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    let shm = format!("/dev/shm/hale-obs-{}", pid);
    let mut seen = 0usize;
    let mut nonzero = Vec::new();
    for _ in 0..80 {
        std::thread::sleep(Duration::from_millis(25));
        let Ok(b) = std::fs::read(&shm) else { continue };
        if b.len() < 0x100 {
            continue;
        }
        let u64at =
            |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let u32at =
            |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let manifest_off = u64at(0x40) as usize;
        if manifest_off == 0 || manifest_off + 16 > b.len() {
            continue;
        }
        let entry_count = u32at(manifest_off) as usize;
        if entry_count == 0 {
            continue;
        }
        seen = entry_count;
        for i in 0..entry_count {
            let e = manifest_off + 16 + i * 32;
            if e + 32 > b.len() {
                break;
            }
            let aux_b = u64at(e + 8);
            if aux_b != 0 {
                nonzero.push((i, aux_b));
            }
        }
        break;
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);

    assert!(seen > 0, "no manifest rows observed — test proves nothing");
    assert!(
        nonzero.is_empty(),
        "an unstamped build published canonical entity ids: {:?}",
        nonzero
    );
}
