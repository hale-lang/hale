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

#[path = "support/obs.rs"]
mod obs;
use obs::{
    attach_observer, net_origin_seq, obs_bus_locus, records,
    snapshot_shm, topic_counters, topic_id_and_line,
};

fn compile(tag: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_obsfleet_{}_{}", tag, std::process::id()));
    build_executable(&program, &bin).expect("build");
    bin
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
    attach_observer(pub_pid);
    attach_observer(s1_pid);
    attach_observer(s2_pid);
    std::thread::sleep(Duration::from_millis(1000));
    let pub_seg = snapshot_shm(pub_pid);
    let s1_seg = snapshot_shm(s1_pid);
    let s2_seg = snapshot_shm(s2_pid);

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
            let locus = obs_bus_locus(*w1);
            locus != 0 && births.contains(&locus)
        })
        .count();
    assert!(
        attributed > 0,
        "BUS_PUBLISH must attribute a real birth instance (got loci {:?}, births {:?})",
        publishes.iter().map(|(_, w1)| obs_bus_locus(*w1)).collect::<Vec<_>>(),
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
    attach_observer(pid);
    std::thread::sleep(Duration::from_millis(700));
    let seg = snapshot_shm(pid);
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

/// iris handoff-5 P17 — per-locus attribution on the fleet's real
/// publisher shapes. Petals read BUS_PUBLISH / BUS_DELIVER
/// `w1 locus:20`, which must equal the LOCUS_BIRTH instance id of
/// the publishing/consuming locus in the same segment. The fleet
/// reads 0 fleet-wide; this pins the three shapes it actually has:
///   1. a keyed-topic publish from an accept()-spawned child locus
///      (dynamic spawn, not a param default);
///   2. a plain publish with ZERO local subscribers (all consumers
///      remote — the pure-fanout path);
///   3. BUS_DELIVER stamped with the SUBSCRIBER's locus for a keyed
///      `where key ==` subscription.
#[test]
fn attribution_on_fleet_publisher_shapes() {
    const SRC: &str = r#"
        type Ev { id: Int; v: Int; }
        type Out { v: Int; }
        topic K { payload: Ev; subject: "k.sig"; keyed_by id; }

        locus KeySub {
            params { my_id: Int = 1; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { println("got ", e.v); }
        }

        locus Pusher {
            bus {
                publish K;
                publish "out" of type Out;
            }
            run() {
                std::time::sleep(400ms);
                let mut i = 0;
                while i < 10 {
                    // shape 1: keyed publish from an accept-spawned child
                    K <- Ev { id: 1, v: i };
                    // shape 2: zero local subscribers, remote-bound
                    "out" <- Out { v: i };
                    i = i + 1;
                }
            }
        }

        main locus App {
            params { s: KeySub = KeySub { my_id: 1 }; }
            accept(p: Pusher) { }
            run() {
                Pusher { };
                std::time::sleep(1500ms);
            }
        }
        fn main() { App { }; }
    "#;
    let bin = compile("attrib", SRC);
    let dir = std::env::temp_dir()
        .join(format!("hale_obsattrib_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("bus.conf");
    // "out" has no local subscriber; a connect binding makes it the
    // pure remote-fanout path (no listener needed — attribution is
    // asserted on the publisher's own segment).
    std::fs::write(&cfg, "out = udp://127.0.0.1:57831:connect\n").unwrap();
    let mut child = Command::new(&bin)
        .env("LOTUS_BUS_CONFIG", &cfg)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(25));
        if std::path::Path::new(&format!("/dev/shm/hale-obs-{}", pid))
            .exists()
        {
            break;
        }
    }
    attach_observer(pid);
    std::thread::sleep(Duration::from_millis(1100));
    let seg = snapshot_shm(pid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);
    let seg = seg.expect("segment");

    let births: std::collections::HashSet<u32> =
        records(&seg, 5).iter().map(|(id, _)| *id).collect();
    let (k_id, _) = topic_id_and_line(&seg, b"k.sig").expect("topic k.sig");
    let (out_id, _) = topic_id_and_line(&seg, b"out").expect("topic out");

    // Shape 1: keyed publishes from the accept-spawned Pusher.
    let k_pubs: Vec<u32> = records(&seg, 1)
        .iter()
        .filter(|(id, _)| *id == k_id)
        .map(|(_, w1)| obs_bus_locus(*w1))
        .collect();
    assert!(!k_pubs.is_empty(), "no BUS_PUBLISH records for keyed topic");
    assert!(
        k_pubs.iter().all(|l| *l != 0 && births.contains(l)),
        "P17 shape 1: keyed publish from accept-spawned child must \
         attribute a birth instance; got loci {:?} (births {:?})",
        k_pubs,
        births
    );

    // Shape 2: pure remote-fanout publishes (zero local subscribers).
    let out_pubs: Vec<u32> = records(&seg, 1)
        .iter()
        .filter(|(id, _)| *id == out_id)
        .map(|(_, w1)| obs_bus_locus(*w1))
        .collect();
    assert!(
        !out_pubs.is_empty(),
        "no BUS_PUBLISH records for the zero-local-subscriber publish"
    );
    assert!(
        out_pubs.iter().all(|l| *l != 0 && births.contains(l)),
        "P17 shape 2: pure-fanout publish must attribute a birth \
         instance; got loci {:?} (births {:?})",
        out_pubs,
        births
    );

    // Shape 3: keyed BUS_DELIVER stamped with the subscriber locus.
    let k_dlvs: Vec<u32> = records(&seg, 2)
        .iter()
        .filter(|(id, _)| *id == k_id)
        .map(|(_, w1)| obs_bus_locus(*w1))
        .collect();
    assert!(!k_dlvs.is_empty(), "no BUS_DELIVER records for keyed topic");
    assert!(
        k_dlvs.iter().all(|l| *l != 0 && births.contains(l)),
        "P17 shape 3: keyed BUS_DELIVER must attribute the subscriber \
         locus; got loci {:?} (births {:?})",
        k_dlvs,
        births
    );
    // And the deliver locus differs from the publish locus (it's the
    // SUBSCRIBER, not an echo of the publisher).
    assert!(
        k_dlvs.iter().any(|d| !k_pubs.contains(d)),
        "keyed BUS_DELIVER stamped the publisher, not the subscriber: \
         dlv {:?} vs pub {:?}",
        k_dlvs,
        k_pubs
    );
}

/// iris handoff-5 P17(c) — the fully-devirtualized DIRECT dispatch
/// (single quiet subscriber, same thread: the `lotus_bus_static_
/// direct_*` + baked-handler flavor) was completely probe-less: the
/// topic never registered, counters stayed 0, and no BUS records
/// fired — a subject on this path was invisible to observation.
/// Asserts the flavor now registers, counts, and attributes both
/// sides.
#[test]
fn direct_devirt_flavor_emits_attributed_probes() {
    const SRC: &str = r#"
        type Tick { n: Int; }
        locus Counter {
            params { count: Int = 0; }
            bus { subscribe "t" as on_t of type Tick; }
            fn on_t(t: Tick) { self.count = self.count + 1; }
        }
        locus Pub {
            bus { publish "t" of type Tick; }
            run() {
                std::time::sleep(400ms);
                let mut i = 0;
                while i < 50 { "t" <- Tick { n: i }; i = i + 1; }
            }
        }
        fn main() {
            let c = Counter { };
            Pub { };
            std::time::sleep(300ms);
            println("count=", c.count);
        }
    "#;
    let bin = compile("direct", SRC);
    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(25));
        if std::path::Path::new(&format!("/dev/shm/hale-obs-{}", pid))
            .exists()
        {
            break;
        }
    }
    attach_observer(pid);
    std::thread::sleep(Duration::from_millis(550));
    let seg = snapshot_shm(pid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    let seg = seg.expect("segment");

    let (pub_cnt, dlv_cnt, _) = topic_counters(&seg, b"t")
        .expect("direct-flavor topic must REGISTER (was invisible)");
    assert_eq!(pub_cnt, 50, "direct-flavor published counter");
    assert_eq!(dlv_cnt, 50, "direct-flavor delivered counter");

    let births: std::collections::HashSet<u32> =
        records(&seg, 5).iter().map(|(id, _)| *id).collect();
    let pubs: Vec<u32> = records(&seg, 1)
        .iter()
        .map(|(_, w1)| obs_bus_locus(*w1))
        .collect();
    let dlvs: Vec<u32> = records(&seg, 2)
        .iter()
        .map(|(_, w1)| obs_bus_locus(*w1))
        .collect();
    assert!(!pubs.is_empty(), "direct flavor emitted no BUS_PUBLISH");
    assert!(!dlvs.is_empty(), "direct flavor emitted no BUS_DELIVER");
    assert!(
        pubs.iter().all(|l| *l != 0 && births.contains(l)),
        "direct-flavor publish attribution: {:?} (births {:?})",
        pubs,
        births
    );
    assert!(
        dlvs.iter().all(|l| *l != 0 && births.contains(l)),
        "direct-flavor deliver attribution: {:?} (births {:?})",
        dlvs,
        births
    );
    // publisher and subscriber are distinct loci.
    assert_ne!(pubs[0], dlvs[0], "pub and dlv attribute the same locus");
}

/// iris handoff-5 P18 — the observer-attach birth replay was driven
/// from INSIDE probes, so a probe-quiet process (main parked in a
/// long sleep/read loop, no bus traffic) never replayed its live
/// loci: segment registered, zero records. The obs heartbeat now
/// drives the 0→1 replay within ~250ms of attach even with no
/// probe traffic at all.
#[test]
fn quiet_process_replays_births_via_heartbeat() {
    const SRC: &str = r#"
        locus Quiet {
            params { n: Int = 0; }
        }
        fn main() {
            let q = Quiet { n: 1 };
            // probe-quiet steady state: no publishes, no births
            // after this point — only the heartbeat can replay.
            std::time::sleep(3000ms);
            println("n=", q.n);
        }
    "#;
    let bin = compile("quiet", SRC);
    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(25));
        if std::path::Path::new(&format!("/dev/shm/hale-obs-{}", pid))
            .exists()
        {
            break;
        }
    }
    // Attach AFTER the birth already happened; the process makes no
    // further probes, so only the heartbeat can notice us.
    std::thread::sleep(Duration::from_millis(300));
    attach_observer(pid);
    std::thread::sleep(Duration::from_millis(800));
    let seg = snapshot_shm(pid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    let seg = seg.expect("segment");
    let births = records(&seg, 5).len();
    assert!(
        births >= 1,
        "P18: probe-quiet process replayed no births after observer \
         attach (heartbeat missing?)"
    );
}

/// iris handoff-6 (P19 companion) — the ORDERING case the field has:
/// publishers reach steady-state publish loops FIRST, the observer
/// attaches minutes later. Every prior test attached before the
/// burst. Asserts BUS_PUBLISH records captured AFTER a late attach
/// carry nonzero locus == a birth instance (the fn-entry gate hoist
/// must not have snapshotted a stale dormant flag — `lotus_obs_live`
/// is resolved in a constructor, before any function entry).
#[test]
fn late_attach_still_attributes_publishes() {
    const SRC: &str = r#"
        type Ev { id: Int; v: Int; }
        topic K { payload: Ev; subject: "k.late"; keyed_by id; }
        locus KeySub {
            params { my_id: Int = 1; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { }
        }
        locus Reader {
            bus { publish K; }
            run() {
                let mut i = 0;
                while i < 300 {
                    K <- Ev { id: 1, v: i };
                    std::time::sleep(10ms);
                    i = i + 1;
                }
            }
        }
        main locus App {
            params {
                s: KeySub = KeySub { my_id: 1 };
                r: Reader = Reader { };
            }
            placement { r: pinned(core = 0); }
            run() { std::time::sleep(3500ms); }
        }
        fn main() { App { }; }
    "#;
    let bin = compile("lateattach", SRC);
    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    // Let the publisher run deep into steady state UNOBSERVED.
    std::thread::sleep(Duration::from_millis(1200));
    attach_observer(pid);
    std::thread::sleep(Duration::from_millis(1200));
    let seg = snapshot_shm(pid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    let seg = seg.expect("segment");
    let births: std::collections::HashSet<u32> =
        records(&seg, 5).iter().map(|(id, _)| *id).collect();
    let pubs: Vec<u32> = records(&seg, 1)
        .iter()
        .map(|(_, w1)| obs_bus_locus(*w1))
        .collect();
    assert!(!pubs.is_empty(), "no post-attach BUS_PUBLISH records");
    assert!(
        pubs.iter().all(|l| *l != 0 && births.contains(l)),
        "P19: post-late-attach publishes unattributed: loci {:?} \
         (births {:?})",
        pubs,
        births
    );
}

/// iris handoff-6 — the ADAPTER inbound path (`std::bus::
/// __local_dispatch`, the Hale-owned-wire ingest) is a DELIVERY:
/// it must not stamp locus=0 BUS_PUBLISH records or inflate the
/// published counter (the reader-thread path has marked its
/// re-dispatch since P15; this entry was unmarked). Loopback
/// adapter: 2 genuine publishes, each relayed once → published
/// counter must be exactly 2 and every publish record attributed.
#[test]
fn adapter_inbound_dispatch_is_not_a_publish() {
    const SRC: &str = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }
        locus Loopback {
            fn send(subject: String, bytes: Bytes) {
                std::bus::__local_dispatch(subject, bytes);
            }
        }
        locus Receiver {
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: Tick) { }
        }
        locus Producer {
            bus { publish Beat; }
            run() {
                std::time::sleep(300ms);
                Beat <- Tick { n: 7 };
                Beat <- Tick { n: 42 };
            }
        }
        main locus App {
            bindings { Beat: Loopback { }; }
            // NOTE: App is the main locus and its run() executes
            // INLINE at the `App { }` statement — keep it short or
            // Producer never instantiates until it returns.
            run() { std::time::sleep(50ms); }
        }
        fn main() {
            App { };
            Receiver { };
            Producer { };
            std::time::sleep(1200ms);
        }
    "#;
    let bin = compile("adapterobs", SRC);
    let mut child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(25));
        if std::path::Path::new(&format!("/dev/shm/hale-obs-{}", pid))
            .exists()
        {
            break;
        }
    }
    attach_observer(pid);
    std::thread::sleep(Duration::from_millis(900));
    let seg = snapshot_shm(pid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    let seg = seg.expect("segment");
    let (published, _, _) =
        topic_counters(&seg, b"beat").expect("beat topic");
    assert_eq!(
        published, 2,
        "adapter inbound re-dispatch inflated the published counter \
         (2 genuine publishes, each relayed once)"
    );
    let births: std::collections::HashSet<u32> =
        records(&seg, 5).iter().map(|(id, _)| *id).collect();
    let pubs: Vec<u32> = records(&seg, 1)
        .iter()
        .map(|(_, w1)| obs_bus_locus(*w1))
        .collect();
    assert!(
        pubs.iter().all(|l| *l != 0 && births.contains(l)),
        "adapter inbound stamped locus=0 publish records: {:?}",
        pubs
    );
}

/// iris handoff-8 P21 — the adapter (Hale-owned-wire) ingest path
/// gets the full trio. A producer fans out over the C udp path
/// under LOTUS_OBS_WIRE=1 (headered datagrams); the consumer does
/// NOT use LOTUS_BUS_CONFIG — its own Hale code reads the socket
/// and hands bytes to `std::bus::__local_dispatch` (the adapter
/// ingest). Before this fix that path was dark: no NET_DELIVER, no
/// BUS_DELIVER, and a headered datagram didn't even deserialize.
/// Asserts on the consumer's segment: NET_DELIVER records carrying
/// the producer's wire (origin, seq); BUS_DELIVER attributed to the
/// subscriber's birth instance; published == 0 (ingest is not a
/// publish).
#[test]
fn adapter_ingest_pairs_and_attributes() {
    const PRODUCER: &str = r#"
        type Sig { v: Int; }
        locus Pub {
            bus { publish "sig.plane" of type Sig; }
            run() {
                std::time::sleep(600ms);
                let mut i = 0;
                while i < 20 {
                    "sig.plane" <- Sig { v: i };
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
    const CONSUMER: &str = r#"
        type Sig { v: Int; }
        fn __empty(e: IoError) -> Bytes { return b""; }
        locus Sub {
            params { got: Int = 0; }
            bus { subscribe "sig.plane" as on_s of type Sig; }
            fn on_s(s: Sig) { self.got = self.got + 1; }
        }
        locus Ingest {
            params { port: Int = 0; }
            bus { publish "sig.plane" of type Sig; }
            run() {
                let fd = std::io::udp::bind("127.0.0.1", self.port) or -1;
                if fd < 0 { return; }
                std::io::udp::set_recv_timeout(fd, 200ms) or discard;
                let mut i = 0;
                while i < 200 {
                    let b = std::io::udp::recv(fd, 4096) or __empty(err);
                    if len(b) > 0 {
                        std::bus::__local_dispatch("sig.plane", b);
                    }
                    i = i + 1;
                }
                std::io::udp::close(fd);
            }
        }
        fn main() {
            Sub { };
            Ingest { port: 57845 };
            std::time::sleep(200ms);
        }
    "#;
    let pub_bin = compile("adppub", PRODUCER);
    let sub_bin = compile("adpsub", CONSUMER);
    let dir = std::env::temp_dir()
        .join(format!("hale_obsadp_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("pub.conf");
    std::fs::write(&cfg, "sig.plane = udp://127.0.0.1:57845:connect\n")
        .unwrap();

    // Consumer first (its Ingest loop binds the port), NO bus config
    // — its only ingest is the adapter path.
    let mut sub = Command::new(&sub_bin)
        .env("LOTUS_OBS", "1")
        .env("LOTUS_OBS_WIRE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn consumer");
    std::thread::sleep(Duration::from_millis(200));
    let mut pubc = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &cfg)
        .env("LOTUS_OBS", "1")
        .env("LOTUS_OBS_WIRE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn producer");
    let (pub_pid, sub_pid) = (pubc.id(), sub.id());
    std::thread::sleep(Duration::from_millis(150));
    attach_observer(pub_pid);
    attach_observer(sub_pid);
    // Burst runs pub t+600..800ms; snapshot both while alive.
    std::thread::sleep(Duration::from_millis(1300));
    let pub_seg = snapshot_shm(pub_pid);
    let sub_seg = snapshot_shm(sub_pid);
    let _ = pubc.kill();
    let _ = pubc.wait();
    let _ = sub.kill();
    let _ = sub.wait();
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_dir_all(&dir);
    let pub_seg = pub_seg.expect("producer segment");
    let sub_seg = sub_seg.expect("consumer segment");

    // Producer side: headered NET_SENDs with nonzero origin.
    let sends: std::collections::HashSet<(u32, u64)> = records(&pub_seg, 3)
        .iter()
        .map(|(_, w1)| net_origin_seq(*w1))
        .collect();
    assert!(!sends.is_empty(), "producer emitted no NET_SEND");

    // Consumer side: the adapter ingest trio.
    let delivers: Vec<(u32, u64)> = records(&sub_seg, 4)
        .iter()
        .map(|(_, w1)| net_origin_seq(*w1))
        .collect();
    assert!(
        !delivers.is_empty(),
        "P21: adapter ingest emitted no NET_DELIVER (dark path)"
    );
    let paired = delivers.iter().filter(|d| sends.contains(d)).count();
    assert!(
        paired > 0,
        "P21: adapter NET_DELIVER did not echo the wire (origin, seq): \
         sends {:?} delivers {:?}",
        sends,
        delivers
    );
    let births: std::collections::HashSet<u32> =
        records(&sub_seg, 5).iter().map(|(id, _)| *id).collect();
    let dlvs: Vec<u32> = records(&sub_seg, 2)
        .iter()
        .map(|(_, w1)| obs_bus_locus(*w1))
        .collect();
    assert!(
        !dlvs.is_empty(),
        "P21: adapter ingest produced no BUS_DELIVER"
    );
    assert!(
        dlvs.iter().all(|l| *l != 0 && births.contains(l)),
        "P21: adapter BUS_DELIVER unattributed: {:?} (births {:?})",
        dlvs,
        births
    );
    // Ingest is a delivery, not a publish.
    let (published, delivered, _) =
        topic_counters(&sub_seg, b"sig.plane").expect("topic on consumer");
    assert_eq!(
        published, 0,
        "adapter ingest inflated the published counter"
    );
    assert!(delivered > 0, "delivered counter is 0");
}

/// iris handoff-8 P20 — remote-only publishes MUST count. The field
/// report ("every purely-remote subject shows CT_PUBLISHED = 0")
/// did not reproduce on any of four flavors (adapter binding, udp
/// config, framed transport, keyed); this pins the keyed+udp shape
/// (their signal plane's) so the counter can never regress to the
/// pre-v0.11.15 keyed-probe-gap behavior the report likely carried.
#[test]
fn remote_only_publish_counts() {
    const SRC: &str = r#"
        type Out { id: Int; v: Int; }
        type Keep { n: Int; }
        topic Sig { payload: Out; subject: "sig.keyed"; keyed_by id; }
        locus KeepAlive {
            bus { subscribe "keep" as on_k of type Keep; }
            fn on_k(k: Keep) { }
        }
        locus Pub {
            bus { publish Sig; }
            run() {
                std::time::sleep(100ms);
                let mut i = 0;
                while i < 20 { Sig <- Out { id: 1, v: i }; i = i + 1; }
            }
        }
        fn main() {
            KeepAlive { };
            Pub { };
            std::time::sleep(700ms);
        }
    "#;
    let bin = compile("remoteonly", SRC);
    let dir = std::env::temp_dir()
        .join(format!("hale_obsro_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("bus.conf");
    std::fs::write(&cfg, "sig.keyed = udp://127.0.0.1:57849:connect\n")
        .unwrap();
    let mut child = Command::new(&bin)
        .env("LOTUS_BUS_CONFIG", &cfg)
        .env("LOTUS_OBS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(25));
        if std::path::Path::new(&format!("/dev/shm/hale-obs-{}", pid))
            .exists()
        {
            break;
        }
    }
    attach_observer(pid);
    std::thread::sleep(Duration::from_millis(500));
    let seg = snapshot_shm(pid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);
    let seg = seg.expect("segment");
    let (published, _, _) =
        topic_counters(&seg, b"sig.keyed").expect("remote-only topic");
    assert_eq!(
        published, 20,
        "P20: remote-only keyed publishes must bump CT_PUBLISHED"
    );
}
