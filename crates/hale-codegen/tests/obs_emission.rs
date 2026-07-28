//! iris handoff P4 — native observation emission (PROTOCOL v0.1).
//!
//! With LOTUS_OBS=1 the runtime publishes a /hale-obs-<pid> shm
//! segment and its own choke points emit protocol records. This
//! test attaches as a minimal consumer (mmap the segment via
//! /dev/shm, bump observer_count on the control page like a real
//! observer) and asserts:
//!   - header magic/proto/layout sanity,
//!   - the topic manifest entry appears with pub == dlv == N and
//!     the byte count,
//!   - records were emitted to the rings (records_total > 0),
//!   - dormant default: without LOTUS_OBS the segment does not
//!     exist.

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_obs_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

const DEMO: &str = r#"
    type Tick { n: Int = 0; }
    locus Sink {
        params { seen: Int = 0; }
        bus { subscribe "obs.test" as on_t of type Tick; }
        fn on_t(t: Tick) { self.seen = self.seen + 1; }
    }
    locus Pub {
        bus { publish "obs.test" of type Tick; }
        run() {
            std::time::sleep(400ms);
            let mut i = 0;
            while i < 50 {
                "obs.test" <- Tick { n: i };
                i = i + 1;
            }
        }
    }
    main locus App {
        params { s: Sink = Sink { }; p: Pub = Pub { }; }
        placement { p: pinned(core = 0); }
        run() { std::time::sleep(1200ms); }
    }
    fn main() { App { }; }
"#;

fn read_u64(seg: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(seg[off..off + 8].try_into().unwrap())
}
fn read_u32(seg: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(seg[off..off + 4].try_into().unwrap())
}

#[test]
fn emits_protocol_segment_with_matching_counters() {
    let bin = build("emit", DEMO);
    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    let shm_path = format!("/dev/shm/hale-obs-{}", pid);

    // Wait for the segment (lazy init on first probe), then act
    // like an attached observer: bump observer_count so ring
    // emission turns on before the publish burst (which starts
    // at t+400ms).
    let mut seg_file = None;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&shm_path)
        {
            seg_file = Some(f);
            break;
        }
    }
    let seg_file = seg_file.unwrap_or_else(|| {
        let _ = child.kill();
        panic!("segment {} never appeared", shm_path)
    });
    let seg_len = seg_file.metadata().expect("meta").len() as usize;
    let map = unsafe {
        libc_mmap(&seg_file, seg_len)
    };
    let seg = unsafe { std::slice::from_raw_parts(map, seg_len) };

    assert_eq!(read_u64(seg, 0x00), 0x4F42534948414C45, "magic");
    assert_eq!(read_u32(seg, 0x1C).count_ones() >= 1, true, "rings");
    let control_off = read_u64(seg, 0x38) as usize;
    let manifest_off = read_u64(seg, 0x40) as usize;
    let counters_off = read_u64(seg, 0x58) as usize;

    // observer attach (control page is the consumer-writable one).
    unsafe {
        let oc = map.add(control_off) as *mut u32;
        std::ptr::write_volatile(oc, 1);
    }

    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "demo exited nonzero");

    // The emitter tears the segment down at exit; we still hold
    // the mapping (POSIX shm semantics).
    // Manifest: find the obs.test topic entry (kind 0).
    let entry_count = read_u32(seg, manifest_off) as usize;
    let entries = manifest_off + 16;
    let pool_off = read_u32(seg, manifest_off + 8) as usize;
    let mut topic_line: Option<usize> = None;
    let mut line = 0usize;
    for i in 0..entry_count {
        let e = entries + i * 32;
        let kind = seg[e + 28];
        if kind == 0 || kind == 2 {
            line += 1;
            if kind == 0 {
                let name_off = read_u32(seg, e + 20) as usize;
                let name_len = seg[e + 24] as usize
                    | ((seg[e + 25] as usize) << 8);
                let name = &seg
                    [manifest_off + pool_off + name_off
                        ..manifest_off + pool_off + name_off + name_len];
                if name == b"obs.test" {
                    topic_line = Some(line);
                }
            }
        }
    }
    let topic_line = topic_line.expect("obs.test in manifest");

    let cline = counters_off + topic_line * 64;
    let published = read_u64(seg, cline);
    let delivered = read_u64(seg, cline + 8);
    let bytes = read_u64(seg, cline + 16);
    let records_total = read_u64(seg, counters_off + 16);
    assert_eq!(published, 50, "published");
    assert_eq!(delivered, 50, "delivered (one subscriber)");
    assert!(bytes > 0, "payload bytes counted");
    assert!(
        records_total > 100,
        "ring records emitted while observed (got {})",
        records_total
    );
}

/// Walk a ring's slots, collecting decoded (ekind, w0-id, w1)
/// tuples. Consumer-side: read head, scan the live window.
fn drain_ring(
    seg: &[u8],
    rings_off: usize,
    ring_slots: usize,
    ring: usize,
) -> Vec<(u32, u32, u64)> {
    // ring descriptor: {u64 data_off, u64 head, ...} at rings_off
    // + ring * 64 (sizeof rdesc). Descriptors precede data.
    let rdesc = rings_off + ring * 64;
    let data_off = read_u64(seg, rdesc) as usize;
    let head = read_u64(seg, rdesc + 8) as usize;
    let mut out = Vec::new();
    let start = head.saturating_sub(ring_slots);
    for i in start..head {
        let slot = data_off + (i & (ring_slots - 1)) * 16;
        let w0 = read_u64(seg, slot);
        let w1 = read_u64(seg, slot + 8);
        let id = (w0 & 0xFFFFF) as u32;
        let ekind = ((w0 >> 20) & 0x1F) as u32;
        out.push((ekind, id, w1));
    }
    out
}

#[test]
fn births_carry_parentage_and_publishes_attribute_locus() {
    let bin = build("attrib", DEMO);
    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    let shm_path = format!("/dev/shm/hale-obs-{}", pid);
    let mut seg_file = None;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&shm_path)
        {
            seg_file = Some(f);
            break;
        }
    }
    let seg_file = seg_file.unwrap_or_else(|| {
        let _ = child.kill();
        panic!("segment never appeared")
    });
    let seg_len = seg_file.metadata().expect("meta").len() as usize;
    let map = unsafe { libc_mmap(&seg_file, seg_len) };
    let seg = unsafe { std::slice::from_raw_parts(map, seg_len) };
    let control_off = read_u64(seg, 0x38) as usize;
    let rings_off = read_u64(seg, 0x68) as usize;
    let ring_count = read_u32(seg, 0x1C) as usize;
    let ring_slots = read_u32(seg, 0x20) as usize;
    // Attach BEFORE the publish burst (starts at t+400ms).
    unsafe {
        let oc = map.add(control_off) as *mut u32;
        std::ptr::write_volatile(oc, 1);
    }
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "demo exited nonzero");

    let mut births = 0usize;
    let mut parented = 0usize;
    let mut publishes = 0usize;
    let mut attributed = 0usize;
    for r in 0..ring_count {
        for (ekind, _id, w1) in
            drain_ring(seg, rings_off, ring_slots, r)
        {
            match ekind {
                5 => {
                    // LOCUS_BIRTH: w1 = parent:32 | type:20
                    births += 1;
                    if (w1 & 0xFFFFFFFF) != 0 {
                        parented += 1;
                    }
                }
                1 => {
                    // BUS_PUBLISH: w1 = locus:20 | seq:44
                    publishes += 1;
                    if (w1 & 0xFFFFF) != 0 {
                        attributed += 1;
                    }
                }
                _ => {}
            }
        }
    }
    // Sink + Pub (+ App itself) all birth; at least the two
    // children must carry a non-root parent (P9).
    assert!(births >= 2, "expected births, got {}", births);
    assert!(
        parented >= 2,
        "births must carry parentage (P9): {}/{} parented",
        parented,
        births
    );
    // The publisher is the pinned Pub locus — its publishes must
    // attribute it (P10).
    assert!(publishes >= 1, "expected publishes, got {}", publishes);
    assert!(
        attributed >= 1,
        "BUS_PUBLISH must attribute the publishing locus (P10):          {}/{} attributed",
        attributed,
        publishes
    );
    // iris handoff-3 P13: every publish record must attribute its
    // locus, and that id must be a real LOCUS_BIRTH instance (not a
    // stray id space). DEMO publishes only from Pub (a non-first,
    // pinned locus); inbound re-dispatch (which had stamped
    // locus=0) is single-process-absent here — so zero
    // unattributed publishes is the exact contract.
    let mut birth_ids = std::collections::HashSet::new();
    for rr in 0..ring_count {
        for (ekind, id, _w1) in drain_ring(seg, rings_off, ring_slots, rr) {
            if ekind == 5 {
                birth_ids.insert(id);
            }
        }
    }
    for rr in 0..ring_count {
        for (ekind, _id, w1) in drain_ring(seg, rings_off, ring_slots, rr) {
            if ekind == 1 {
                let locus = (w1 & 0xFFFFF) as u32;
                assert!(
                    locus != 0,
                    "P13: a BUS_PUBLISH stamped locus=0 (unattributed \
                     — the reader re-dispatch bug)"
                );
                assert!(
                    birth_ids.contains(&locus),
                    "P13: publish locus {} is not a LOCUS_BIRTH instance",
                    locus
                );
            }
        }
    }
}

#[test]
fn dormant_by_default() {
    let bin = build("dormant", DEMO);
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    std::thread::sleep(std::time::Duration::from_millis(600));
    let exists =
        std::path::Path::new(&format!("/dev/shm/hale-obs-{}", pid))
            .exists();
    let _ = child.kill();
    let _ = child.wait();
    assert!(!exists, "no segment without LOTUS_OBS");
}

unsafe fn libc_mmap(f: &std::fs::File, len: usize) -> *mut u8 {
    use std::os::unix::io::AsRawFd;
    extern "C" {
        fn mmap(
            addr: *mut core::ffi::c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            off: i64,
        ) -> *mut core::ffi::c_void;
    }
    let p = mmap(
        core::ptr::null_mut(),
        len,
        0x1 | 0x2, /* PROT_READ|WRITE */
        0x1,       /* MAP_SHARED */
        f.as_raw_fd(),
        0,
    );
    assert!(p as isize != -1, "mmap failed");
    p as *mut u8
}
