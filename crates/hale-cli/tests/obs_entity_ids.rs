//! GH #476 Change 8 — canonical model entity ids in the live
//! manifest.
//!
//! The observation manifest names entities (`sense.reading`, `W`)
//! and numbers them in registration order. A consumer holding the
//! source-derived model could only join the two by matching strings
//! — a second authority on identity, and the thing this epic
//! removes. Codegen now stamps the model's own entity id into each
//! manifest row's `aux_b` (a field that has been in the ABI since v0
//! and written as 0 by every path, so no consumer's layout moves).
//!
//! Pinned here: the ids appear, they are the ids `hale model dump`
//! reports (+1, since 0 means unstamped), a locus type and a topic
//! both carry them, and a harness-built binary still reads 0.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_obs_ids_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn build(dir: &Path, src: &str, tag: &str) -> PathBuf {
    let seed = dir.join(tag);
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&seed)
        .output()
        .expect("hale build");
    assert!(
        out.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    seed.join(tag)
}

/// One manifest row, as a consumer reads it.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    kind: u8,
    name: String,
    /// Registration-order id (what events reference).
    id: u32,
    /// GH #476 Change 8: canonical model entity id, 0 = unstamped.
    aux_b: u64,
}

/// Run the binary under LOTUS_OBS=1 and read its manifest rows out
/// of the live segment.
fn manifest_rows(bin: &Path) -> Vec<Row> {
    let mut child = Command::new(bin)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    let shm = format!("/dev/shm/hale-obs-{}", pid);
    let mut rows = Vec::new();
    for _ in 0..80 {
        std::thread::sleep(Duration::from_millis(25));
        let Ok(b) = std::fs::read(&shm) else { continue };
        if b.len() < 0x100 {
            continue;
        }
        let u64at = |o: usize| {
            u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
        };
        let u32at = |o: usize| {
            u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
        };
        // obs_hdr_t: manifest_off is the 2nd of the offset run that
        // starts at 0x38 (control_off).
        let manifest_off = u64at(0x40) as usize;
        if manifest_off == 0 || manifest_off + 16 > b.len() {
            continue;
        }
        // obs_mh_t { entry_count, entry_cap, pool_off, pool_used }
        let entry_count = u32at(manifest_off) as usize;
        let pool_off = u32at(manifest_off + 8) as usize;
        if entry_count == 0 {
            continue;
        }
        // obs_me_t is 32 bytes: shape_hash, aux_b, id, name_off,
        // name_len, aux_a, kind, flags, _pad.
        let base = manifest_off + 16;
        let mut out = Vec::new();
        for i in 0..entry_count {
            let e = base + i * 32;
            if e + 32 > b.len() {
                break;
            }
            let aux_b = u64at(e + 8);
            let id = u32at(e + 16);
            let name_off = u32at(e + 20) as usize;
            let name_len = u16::from_le_bytes(
                b[e + 24..e + 26].try_into().unwrap(),
            ) as usize;
            let kind = b[e + 28];
            let ns = manifest_off + pool_off + name_off;
            if ns + name_len > b.len() {
                continue;
            }
            out.push(Row {
                kind,
                name: String::from_utf8_lossy(&b[ns..ns + name_len])
                    .into_owned(),
                id,
                aux_b,
            });
        }
        if !out.is_empty() {
            rows = out;
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    rows
}

const PROG: &str = r#"
type Tick { n: Int = 0; }
topic Ticks { payload: Tick; subject: "obs.tick"; }
// Named so that BIRTH order (Zeta, then Alpha — params field order
// under the root) is not the model's table order (alphabetical).
// If the two coincided, this test could not tell a canonical id
// from a registration-order id.
locus Zeta {
    params { seen: Int = 0; }
    bus { subscribe Ticks as on_t; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}
locus Alpha {
    params { n: Int = 0; }
    fn bump() { self.n = self.n + 1; }
}
main locus App {
    params { z: Zeta = Zeta { }; a: Alpha = Alpha { }; }
    bus { publish Ticks; }
    run() {
        self.a.bump();
        Ticks <- Tick { n: 1 };
        std::time::sleep(600ms);
    }
}
fn main() { App { }; }
"#;

/// The model's own ids for the subject and the locus types, as the
/// dump reports them (the dump prints tables in id order).
fn model_tables(src_dir: &Path) -> (Vec<String>, Vec<String>) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("model")
        .arg("dump")
        .arg(src_dir)
        .output()
        .expect("hale model dump");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let section = |head: &str| -> Vec<String> {
        let mut rows = Vec::new();
        let mut inside = false;
        for line in text.lines() {
            if line.starts_with(head) {
                inside = true;
                continue;
            }
            if !inside {
                continue;
            }
            match line.strip_prefix("  ") {
                Some(r) => rows.push(
                    r.split(" -> ").next().unwrap().trim().to_string(),
                ),
                None => break,
            }
        }
        rows
    };
    (section("subjects ("), section("loci ("))
}

#[test]
fn manifest_rows_carry_canonical_model_entity_ids() {
    let dir = workdir("stamped");
    let bin = build(&dir, PROG, "app");
    let (subjects, loci) = model_tables(&dir.join("app"));
    let rows = manifest_rows(&bin);
    assert!(!rows.is_empty(), "no manifest rows observed");

    // The topic row for the wire subject carries the model's
    // SubjectId + 1 (the manifest fuses publishers by subject, so
    // subject — not topic decl — is the canonical address).
    let topic = rows
        .iter()
        .find(|r| r.kind == 0 && r.name == "obs.tick")
        .unwrap_or_else(|| panic!("no topic row: {:?}", rows));
    let want = subjects
        .iter()
        .position(|s| s == "obs.tick")
        .expect("model names the subject")
        + 1;
    assert_eq!(
        topic.aux_b, want as u64,
        "topic row carries the canonical SubjectId (+1); model \
         subjects = {:?}",
        subjects
    );

    // …and every locus type that registered carries its LocusDeclId.
    let locus_rows: Vec<&Row> =
        rows.iter().filter(|r| r.kind == 1).collect();
    assert!(
        !locus_rows.is_empty(),
        "no locus-type rows registered: {:?}",
        rows
    );
    for r in &locus_rows {
        let want =
            loci.iter().position(|l| *l == r.name).unwrap_or_else(|| {
                panic!("manifest names locus `{}`, model has {:?}", r.name, loci)
            }) + 1;
        assert_eq!(
            r.aux_b, want as u64,
            "locus row `{}` carries the wrong canonical id",
            r.name
        );
    }

    // The registration-order id and the canonical id are different
    // numbers doing different jobs — this is the whole point, so
    // assert the manifest still carries its own ordering id.
    assert!(
        rows.iter().any(|r| r.aux_b != u64::from(r.id)),
        "canonical ids are indistinguishable from registration \
         order in this program — the test proves nothing: {:?}",
        rows
    );
    let _ = std::fs::remove_dir_all(&dir);
}
