//! iris handoff-4 — the field-shaped acceptance gate.
//!
//! Every prior obs test was a 2-process loopback UNICAST pair, which
//! is why three field regressions slipped through: NET records with a
//! hardcoded topic id 0 (P14 — no join key, so edges are structurally
//! impossible), a published counter that vanished on the fleet (P15 —
//! the keyed dispatch flavors had no publish probe, and P13's TLS gate
//! excluded genuine unattributed publishes), and a subscriber whose
//! inbound re-dispatch inflated its own published counter.
//!
//! This test is shaped like the fleet: 3 processes over a REAL UDP
//! multicast group, LOTUS_OBS=1, asserting the full consumer-visible
//! contract in ONE pass —
//!   * publisher NET_SEND records carry a nonzero topic id (== the
//!     process's manifest topic id) and a nonzero origin,
//!   * the publisher's published counter is nonzero and its
//!     BUS_PUBLISH attributes a real LOCUS_BIRTH instance,
//!   * each subscriber's NET_DELIVER carries the (matching) topic id
//!     and pairs with a publisher NET_SEND on (origin, seq),
//!   * each subscriber's delivered counter is nonzero AND its
//!     published counter is zero (a delivery is not a publish).
//!
//! A second, deterministic single-process test guards the keyed
//! dispatch flavor specifically: a keyed publish must record a
//! BUS_PUBLISH and bump the published counter (the flavor that was
//! silently probe-less).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use hale_codegen::build_executable;

fn compile(tag: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_obsfleet_{}_{}", tag, std::process::id()));
    build_executable(&program, &bin).expect("build");
    bin
}

fn read_u64(seg: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(seg[off..off + 8].try_into().unwrap())
}
fn read_u32(seg: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(seg[off..off + 4].try_into().unwrap())
}

unsafe fn mmap_ro(f: &std::fs::File, len: usize) -> *mut u8 {
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
    let p = mmap(core::ptr::null_mut(), len, 0x1, 0x1, f.as_raw_fd(), 0);
    if p as isize == -1 {
        return core::ptr::null_mut();
    }
    p as *mut u8
}

/// Snapshot a live process's obs segment into an owned buffer (must
/// run before the process exits — teardown shm_unlinks).
unsafe fn snapshot_shm(pid: u32) -> Option<Vec<u8>> {
    let f = std::fs::File::open(format!("/dev/shm/hale-obs-{}", pid))
        .ok()?;
    let len = f.metadata().ok()?.len() as usize;
    let p = mmap_ro(&f, len);
    if p.is_null() {
        return None;
    }
    Some(std::slice::from_raw_parts(p as *const u8, len).to_vec())
}

/// Attach as an observer (bump observer_count so ring emission turns
/// on) — requires a writable map.
unsafe fn attach_observer(pid: u32) {
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
    if let Ok(f) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("/dev/shm/hale-obs-{}", pid))
    {
        let len = f.metadata().unwrap().len() as usize;
        let p = mmap(core::ptr::null_mut(), len, 0x3, 0x1, f.as_raw_fd(), 0);
        if p as isize != -1 {
            let seg = std::slice::from_raw_parts(p as *const u8, len);
            let control_off = read_u64(seg, 0x38) as usize;
            std::ptr::write_volatile(
                (p as *mut u8).add(control_off) as *mut u32,
                1,
            );
        }
    }
}

/// The manifest topic id (join key) + counter line for a subject, or
/// None if the topic never registered in this segment.
fn topic_id_and_line(seg: &[u8], subject: &[u8]) -> Option<(u32, usize)> {
    let manifest_off = read_u64(seg, 0x40) as usize;
    let entry_count = read_u32(seg, manifest_off) as usize;
    let pool_off = read_u32(seg, manifest_off + 8) as usize;
    let entries = manifest_off + 16;
    let mut line = 0usize;
    for i in 0..entry_count {
        let e = entries + i * 32;
        let kind = seg[e + 28];
        // topic (0) and binding (2) entries each occupy a counter line.
        if kind == 0 || kind == 2 {
            line += 1;
            if kind == 0 {
                let name_off = read_u32(seg, e + 20) as usize;
                let name_len = seg[e + 24] as usize
                    | ((seg[e + 25] as usize) << 8);
                let base = manifest_off + pool_off + name_off;
                if &seg[base..base + name_len] == subject {
                    let id = read_u32(seg, e + 16);
                    return Some((id, line));
                }
            }
        }
    }
    None
}

/// (published, delivered, bytes) counters for a subject.
fn topic_counters(seg: &[u8], subject: &[u8]) -> Option<(u64, u64, u64)> {
    let (_, line) = topic_id_and_line(seg, subject)?;
    let counters_off = read_u64(seg, 0x58) as usize;
    let cline = counters_off + line * 64;
    Some((
        read_u64(seg, cline),
        read_u64(seg, cline + 8),
        read_u64(seg, cline + 16),
    ))
}

/// All records of a given ekind as (w0-id, w1). NET (3/4): w0-id is
/// the topic id, w1 = origin:16 | seq:48. BUS_PUBLISH (1): w1 =
/// locus:20 | seq:44. LOCUS_BIRTH (5): w0-id = instance id.
fn records(seg: &[u8], want_ekind: u32) -> Vec<(u32, u64)> {
    let rings_off = read_u64(seg, 0x68) as usize;
    let ring_count = read_u32(seg, 0x1C) as usize;
    let ring_slots = read_u32(seg, 0x20) as usize;
    let mut out = Vec::new();
    for r in 0..ring_count {
        let rdesc = rings_off + r * 64;
        let data_off = read_u64(seg, rdesc) as usize;
        let head = read_u64(seg, rdesc + 8) as usize;
        let start = head.saturating_sub(ring_slots);
        for i in start..head {
            let slot = data_off + (i & (ring_slots - 1)) * 16;
            let w0 = read_u64(seg, slot);
            let w1 = read_u64(seg, slot + 8);
            let id = (w0 & 0xFFFFF) as u32;
            let ekind = ((w0 >> 20) & 0x1F) as u32;
            if ekind == want_ekind {
                out.push((id, w1));
            }
        }
    }
    out
}

fn net_origin_seq(w1: u64) -> (u32, u64) {
    ((w1 & 0xFFFF) as u32, (w1 >> 16) & 0xFFFF_FFFF_FFFF)
}

const SUB: &str = r#"
    type Ping { n: Int; }
    locus Sub {
        bus { subscribe "evt" as on_evt of type Ping; }
        fn on_evt(p: Ping) { println("got n=", p.n); }
    }
    fn main() { Sub { }; std::time::sleep(2500ms); }
"#;

const PUB: &str = r#"
    type Ping { n: Int; }
    locus Pub {
        bus { publish "evt" of type Ping; }
        run() {
            std::time::sleep(400ms);
            let mut i = 0;
            while i < 20 {
                "evt" <- Ping { n: i };
                std::time::sleep(10ms);
                i = i + 1;
            }
        }
    }
    main locus App {
        params { p: Pub = Pub { }; }
        placement { p: pinned(core = 0); }
        run() { std::time::sleep(2500ms); }
    }
    fn main() { App { }; }
"#;

#[test]
fn fleet_multicast_full_contract() {
    let pub_bin = compile("mpub", PUB);
    let sub_bin = compile("msub", SUB);
    let dir = std::env::temp_dir()
        .join(format!("hale_obsfleet_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Admin-scoped multicast group; the bus auto-joins (IP_ADD_MEMBERSHIP)
    // when the listen address is in 224.0.0.0/4, and fans out to it on
    // connect. Loopback delivery relies on IP_MULTICAST_LOOP (default on).
    let group = "239.255.77.12";
    let port = 57821u16;
    let sub_cfg = dir.join("sub.conf");
    let pub_cfg = dir.join("pub.conf");
    std::fs::write(
        &sub_cfg,
        format!("evt = udp://{}:{}:listen\n", group, port),
    )
    .unwrap();
    std::fs::write(
        &pub_cfg,
        format!("evt = udp://{}:{}:connect\n", group, port),
    )
    .unwrap();

    // LOTUS_OBS_WIRE=1 opts the fleet into the observation wire (the
    // (origin, seq) header) — required for cross-process edges (P16).
    let spawn_sub = || {
        Command::new(&sub_bin)
            .env("LOTUS_BUS_CONFIG", &sub_cfg)
            .env("LOTUS_OBS", "1")
            .env("LOTUS_OBS_WIRE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sub")
    };
    let mut sub1 = spawn_sub();
    let mut sub2 = spawn_sub();
    std::thread::sleep(Duration::from_millis(200));
    let mut pubc = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .env("LOTUS_OBS", "1")
        .env("LOTUS_OBS_WIRE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn pub");
    let (pub_pid, s1_pid, s2_pid) = (pubc.id(), sub1.id(), sub2.id());
    // Attach as observer before the burst (starts at pub t+400ms).
    std::thread::sleep(Duration::from_millis(150));
    unsafe {
        attach_observer(pub_pid);
        attach_observer(s1_pid);
        attach_observer(s2_pid);
    }
    std::thread::sleep(Duration::from_millis(1000));
    let pub_seg = unsafe { snapshot_shm(pub_pid) };
    let s1_seg = unsafe { snapshot_shm(s1_pid) };
    let s2_seg = unsafe { snapshot_shm(s2_pid) };

    let _ = pubc.kill();
    let _ = pubc.wait();
    let _ = sub1.kill();
    let _ = sub1.wait();
    let _ = sub2.kill();
    let _ = sub2.wait();
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_dir_all(&dir);

    let pub_seg = pub_seg.expect("publisher segment");
    let s1_seg = s1_seg.expect("sub1 segment");
    let s2_seg = s2_seg.expect("sub2 segment");

    // ---- Publisher side ------------------------------------------
    let (pub_topic_id, _) = topic_id_and_line(&pub_seg, b"evt")
        .expect("publisher registered topic evt");
    let sends = records(&pub_seg, 3); // NET_SEND
    assert!(!sends.is_empty(), "publisher emitted no NET_SEND");
    // P14: the record id field is the topic id, NOT the hardcoded 0.
    assert!(
        sends.iter().all(|(id, _)| *id == pub_topic_id && *id != 0),
        "P14: NET_SEND topic id != manifest topic id {} (got {:?})",
        pub_topic_id,
        sends.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    );
    // origin identifies the sender and must be nonzero.
    assert!(
        sends.iter().all(|(_, w1)| net_origin_seq(*w1).0 != 0),
        "NET_SEND origin is 0 (unattributed sender)"
    );

    // P15: the publisher's published counter must be nonzero.
    let (pub_published, _, pub_bytes) =
        topic_counters(&pub_seg, b"evt").expect("evt counters on pub");
    assert!(
        pub_published > 0,
        "P15: publisher published counter is 0 (the fleet regression)"
    );
    assert!(pub_bytes > 0, "publisher counted zero payload bytes");

    // Attribution: BUS_PUBLISH must name a real LOCUS_BIRTH instance.
    let births: std::collections::HashSet<u32> =
        records(&pub_seg, 5).iter().map(|(id, _)| *id).collect();
    let publishes = records(&pub_seg, 1); // BUS_PUBLISH
    assert!(!publishes.is_empty(), "publisher emitted no BUS_PUBLISH");
    let attributed = publishes
        .iter()
        .filter(|(_, w1)| {
            let locus = (*w1 & 0xFFFFF) as u32;
            locus != 0 && births.contains(&locus)
        })
        .count();
    assert!(
        attributed > 0,
        "BUS_PUBLISH must attribute a real birth instance (got loci {:?}, births {:?})",
        publishes.iter().map(|(_, w1)| (*w1 & 0xFFFFF) as u32).collect::<Vec<_>>(),
        births
    );

    // ---- Subscriber side (each of the two group members) ---------
    let send_pairs: std::collections::HashSet<(u32, u64)> =
        sends.iter().map(|(_, w1)| net_origin_seq(*w1)).collect();
    let mut any_paired = false;
    for (label, seg) in [("sub1", &s1_seg), ("sub2", &s2_seg)] {
        let (sub_topic_id, _) = topic_id_and_line(seg, b"evt")
            .unwrap_or_else(|| panic!("{}: topic evt not registered", label));
        let delivers = records(seg, 4); // NET_DELIVER
        assert!(
            !delivers.is_empty(),
            "{}: emitted no NET_DELIVER (multicast delivery failed?)",
            label
        );
        // P14 on the deliver side: the id is this process's topic id.
        assert!(
            delivers.iter().all(|(id, _)| *id == sub_topic_id && *id != 0),
            "{}: NET_DELIVER topic id != manifest id {} (got {:?})",
            label,
            sub_topic_id,
            delivers.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        );
        // Edge: at least one (origin, seq) pairs with a publisher send.
        let paired = delivers
            .iter()
            .filter(|(_, w1)| send_pairs.contains(&net_origin_seq(*w1)))
            .count();
        assert!(
            paired > 0,
            "{}: no NET_DELIVER pairs a NET_SEND on (origin, seq).\nsends {:?}\ndelivers {:?}",
            label,
            send_pairs,
            delivers.iter().map(|(_, w1)| net_origin_seq(*w1)).collect::<Vec<_>>()
        );
        any_paired = true;

        // P15 negative-marking: a subscriber DELIVERS but never
        // PUBLISHES — its inbound re-dispatch must not inflate the
        // published counter.
        let (s_published, s_delivered, _) = topic_counters(seg, b"evt")
            .expect("evt counters on sub");
        assert!(
            s_delivered > 0,
            "{}: delivered counter is 0",
            label
        );
        assert_eq!(
            s_published, 0,
            "P15: {}: inbound re-dispatch inflated the published counter to {} (must be 0)",
            label, s_published
        );
    }
    assert!(any_paired, "no subscriber produced a cross-process edge");
}

/// P16: LOTUS_OBS=1 WITHOUT LOTUS_OBS_WIRE must not touch the wire. A
/// plain, unobserving receiver (standing in for a pre-header / stale
/// binary) must still receive every datagram from an observed sender —
/// no 16-byte header corrupts its deserialize. This is the regression
/// that silently partitioned a mixed-version fleet.
#[test]
fn obs_without_wire_leaves_udp_pristine() {
    let pub_bin = compile("ppub", PUB);
    let sub_bin = compile("psub", SUB);
    let dir = std::env::temp_dir()
        .join(format!("hale_obspristine_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let group = "239.255.77.13";
    let port = 57823u16;
    let sub_cfg = dir.join("sub.conf");
    let pub_cfg = dir.join("pub.conf");
    std::fs::write(&sub_cfg, format!("evt = udp://{}:{}:listen\n", group, port))
        .unwrap();
    std::fs::write(&pub_cfg, format!("evt = udp://{}:{}:connect\n", group, port))
        .unwrap();

    // Receiver runs with NO observation at all — a pristine peer.
    let sub = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sub");
    std::thread::sleep(Duration::from_millis(200));
    // Sender is OBSERVED (LOTUS_OBS=1) but did NOT opt into the wire.
    let mut pubc = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn pub");
    // Burst runs pub t+400..~700ms; give it margin, then stop pub and
    // collect the receiver's output (SUB exits on its own ~2.5s in).
    std::thread::sleep(Duration::from_millis(1200));
    let _ = pubc.kill();
    let _ = pubc.wait();
    let out = sub.wait_with_output().expect("sub output");
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_dir_all(&dir);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("got n="),
        "P16: a pristine (unobserving) receiver got NO messages from an \
         LOTUS_OBS=1 sender — observation changed the wire without \
         LOTUS_OBS_WIRE. stdout: {:?}",
        stdout
    );
}

/// The keyed dispatch flavor had no BUS_PUBLISH probe at all, so a
/// keyed topic recorded zero publishes and a zero published counter —
/// exactly the shape of a routed market-data feed. Single-process and
/// deterministic: a keyed publish to a matching local subscriber must
/// record a BUS_PUBLISH and bump the published counter.
#[test]
fn keyed_publish_records_bus_publish() {
    const KEYED: &str = r#"
        type Ev { id: Int; payload: Int; }
        topic K { payload: Ev; subject: "k"; keyed_by id; }
        locus Sub {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { println("got id=", e.id); }
        }
        main locus App {
            params { a: Sub = Sub { my_id: 1 }; }
            bus { publish K; }
            run() {
                std::time::sleep(400ms);
                let mut i = 0;
                while i < 15 {
                    K <- Ev { id: 1, payload: i };
                    i = i + 1;
                }
                std::time::sleep(400ms);
            }
        }
        fn main() { App { }; }
    "#;
    let bin = compile("keyed", KEYED);
    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    // Wait for the segment, attach before the burst (t+400ms).
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(25));
        if std::path::Path::new(&format!("/dev/shm/hale-obs-{}", pid))
            .exists()
        {
            break;
        }
    }
    unsafe { attach_observer(pid) };
    std::thread::sleep(Duration::from_millis(700));
    let seg = unsafe { snapshot_shm(pid) };
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);

    let seg = seg.expect("keyed segment");
    let (published, delivered, _) = topic_counters(&seg, b"k")
        .expect("keyed topic k registered");
    assert!(
        published > 0,
        "keyed publish recorded published=0 (the probe-less keyed dispatch flavor)"
    );
    assert!(delivered > 0, "keyed matching subscriber delivered=0");
    let publishes = records(&seg, 1); // BUS_PUBLISH
    assert!(
        !publishes.is_empty(),
        "keyed publish emitted no BUS_PUBLISH record"
    );
}
