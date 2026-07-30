//! Observation counters for a topic declared in an IMPORTED seed.
//!
//! A downstream handoff reported `CT_PUBLISHED` always 0 for a
//! cross-seed topic declaration, with deliveries and NET edges
//! unaffected — and correctly identified why the substrate could not
//! see it: **every in-tree obs test declares its topics inline in the
//! test program.** A topic declared in `lib/` and imported under an
//! alias is the only shape a real multi-binary codebase uses, and it
//! was untested.
//!
//! That corpus gap is what this file closes. The bug itself did NOT
//! reproduce at the tree the report measured (`8e1af0f`) in either shape
//! tried here — in-process with a local subscriber, and
//! transport-bound with no local subscriber. Both count correctly.
//! So this is a standing guard rather than a fix: if the counter
//! regresses for an imported topic, it fails here instead of being
//! rediscovered downstream a month later.
//!
//! The inline topic published the same number of times in the same
//! loop is the control. It is what makes a future failure
//! interpretable: if `shared` goes to zero while `inline` holds, the
//! difference is the decl site and nothing else.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Materialize the fixture into a private directory and return its
/// `app/` path.
///
/// `hale build <dir>` writes the binary INTO that directory, so three
/// tests building the shared fixture concurrently race on one output
/// path — the same ETXTBSY class `harness::unique_bin` exists to
/// prevent, which this file reintroduced on its first run. Each test
/// gets its own copy instead.
fn fixture_app() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-obs");
    let dst = std::env::temp_dir().join(format!(
        "hale_xseed_obs_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dst);
    copy_tree(&src, &dst);
    dst.join("app")
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create fixture dir");
    for e in std::fs::read_dir(src).expect("read fixture").flatten() {
        let p = e.path();
        let target = dst.join(e.file_name());
        if p.is_dir() {
            copy_tree(&p, &target);
        } else if p.extension().map(|x| x != "gitignore").unwrap_or(true) {
            let _ = std::fs::copy(&p, &target);
        }
    }
}

/// (published, delivered) for a subject, decoded from the segment.
fn counters(seg: &[u8], subject: &str) -> Option<(u64, u64)> {
    let u32a = |o: usize| -> u32 {
        u32::from_le_bytes(seg[o..o + 4].try_into().unwrap())
    };
    let u64a = |o: usize| -> u64 {
        u64::from_le_bytes(seg[o..o + 8].try_into().unwrap())
    };
    let man = u64a(0x40) as usize;
    let n = u32a(man) as usize;
    let pool = u32a(man + 8) as usize;
    let entries = man + 16;
    let cnt = u64a(0x58) as usize;
    let mut line = 0usize;
    for i in 0..n {
        let e = entries + i * 32;
        let kind = seg[e + 28];
        if kind == 0 || kind == 2 {
            line += 1;
            if kind == 0 {
                let no = u32a(e + 20) as usize;
                let nl = seg[e + 24] as usize | ((seg[e + 25] as usize) << 8);
                let base = man + pool + no;
                if &seg[base..base + nl] == subject.as_bytes() {
                    let c = cnt + line * 64;
                    return Some((u64a(c), u64a(c + 8)));
                }
            }
        }
    }
    None
}

#[test]
fn imported_topic_publishes_are_counted() {
    let app = fixture_app();
    let hale = env!("CARGO_BIN_EXE_hale");
    let build = Command::new(hale)
        .arg("build")
        .arg(&app)
        .output()
        .expect("hale build");
    assert!(
        build.status.success(),
        "building the cross-seed obs fixture failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let bin = app.join("app");
    assert!(bin.is_file(), "expected a binary at {}", bin.display());

    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("run the fixture");
    // The fixture sleeps 300ms, publishes 10 of each, then sleeps
    // again — read while it is alive, since a clean exit unlinks the
    // segment.
    std::thread::sleep(std::time::Duration::from_millis(450));
    let seg_path = format!("/dev/shm/hale-obs-{}", child.id());
    let seg = std::fs::read(Path::new(&seg_path));
    let _ = child.wait();

    let seg = match seg {
        Ok(s) => s,
        Err(e) => panic!("no obs segment at {}: {}", seg_path, e),
    };

    let inline = counters(&seg, "xseed.inline")
        .unwrap_or_else(|| panic!("inline topic missing from the manifest"));
    let shared = counters(&seg, "xseed.shared")
        .unwrap_or_else(|| panic!("imported topic missing from the manifest"));

    // The control first: if this is wrong the whole run is suspect
    // and the cross-seed assertion below would be noise.
    assert_eq!(
        inline.0, 10,
        "the INLINE control should have counted 10 publishes; got {:?} \
         (shared: {:?})",
        inline, shared
    );
    assert_eq!(
        shared.0, 10,
        "a topic declared in an IMPORTED seed must count publishes the \
         same as an inline one — inline counted {:?}, imported {:?}. \
         The decl site is the only difference between them.",
        inline, shared
    );
    assert_eq!(
        shared.1, 10,
        "and its deliveries: {:?}",
        shared
    );
}

/// A SIGKILLed emitter leaks its shm segment: by definition it runs
/// no atexit handler. A downstream fleet measured **442 stale segments, 245 MB
/// of host tmpfs** from a single fleet run, because `docker stop`
/// never reaches `dissolve` and their compose bind-mounts /dev/shm.
///
/// A dead process cannot clean up after itself, so the next observed
/// process to start does it — same uid, already paying init cost, and
/// a restarting fleet therefore sweeps itself.
///
/// (Running the suite on a dev box had accumulated 69 of these before
/// the sweep existed, so this was never purely a downstream problem.)
#[test]
fn a_stale_segment_from_a_dead_pid_is_swept() {
    // A pid that is definitely dead: spawn something trivial and REAP
    // it. Fabricating a number risks colliding with a live process —
    // and an unreaped child is a zombie, which `kill(pid, 0)` reports
    // as alive, so the sweep would rightly skip it. (That is exactly
    // how this test failed on its first run.)
    let mut victim = Command::new("true").spawn().expect("spawn");
    let dead = victim.id();
    victim.wait().expect("reap the victim");
    std::thread::sleep(std::time::Duration::from_millis(50));

    let stale = format!("/dev/shm/hale-obs-{}", dead);
    if std::fs::write(&stale, vec![0u8; 4096]).is_err() {
        eprintln!("skipping: cannot write to /dev/shm");
        return;
    }
    assert!(Path::new(&stale).exists(), "fixture segment not created");

    let app = fixture_app();
    let build = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&app)
        .output()
        .expect("hale build");
    assert!(build.status.success(), "build failed");
    let bin = app.join("app");

    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("run");
    std::thread::sleep(std::time::Duration::from_millis(350));
    let swept = !Path::new(&stale).exists();
    let _ = child.wait();
    let _ = std::fs::remove_file(&stale);

    assert!(
        swept,
        "a segment belonging to dead pid {} should have been swept by \
         the next observed process; it is unbounded tmpfs growth on a \
         long-lived host otherwise",
        dead
    );
}

/// The sweep must never touch a LIVE process's segment. A recycled
/// pid could in principle own a stale one, but blinding a running
/// observer is far worse than leaving a file behind, so the rule is
/// "skip anything alive".
#[test]
fn the_sweep_leaves_a_live_processes_segment_alone() {
    let app = fixture_app();
    let build = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&app)
        .output()
        .expect("hale build");
    assert!(build.status.success(), "build failed");
    let bin = app.join("app");

    let mut a = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("run A");
    std::thread::sleep(std::time::Duration::from_millis(200));
    let a_seg = format!("/dev/shm/hale-obs-{}", a.id());
    let existed = Path::new(&a_seg).exists();

    // B starts and sweeps while A is still alive.
    let mut b = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("run B");
    std::thread::sleep(std::time::Duration::from_millis(150));
    let a_survived = Path::new(&a_seg).exists();
    let _ = b.wait();
    let _ = a.wait();

    if existed {
        assert!(
            a_survived,
            "a live process's segment must survive another process's \
             startup sweep"
        );
    }
}
