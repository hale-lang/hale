//! iris handoff-3 P11/P12 — cross-process NET seq pairing.
//!
//! The edge blocker: a UDP publisher's `NET_SEND` and a
//! subscriber's `NET_DELIVER` for the same message must share
//! `(origin, seq)`, so iris's seq matcher pairs them into a
//! cross-process edge. Before the fix the receiver stamped its
//! LOCAL receive counter (which sums across senders), so the
//! send seq never equalled the deliver seq and edges stayed
//! zero. The fix puts the sender's `(origin, seq)` on the UDP
//! wire (self-describing 16-byte header) and the reader echoes
//! it verbatim.
//!
//! This test runs both binaries under LOTUS_OBS=1 over a
//! loopback udp:// binding, then reads BOTH segments as a raw
//! consumer and asserts: the publisher emitted NET_SEND records
//! with a nonzero origin (P12), the subscriber emitted matching
//! NET_DELIVER records, and there is at least one exact
//! (origin, seq) pair across the two segments (P11).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use hale_codegen::build_executable;

#[path = "support/obs.rs"]
mod obs;
use obs::{attach_observer, net_origin_seq, records, snapshot_shm};

fn compile(tag: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_obsnet_{}_{}", tag, std::process::id()));
    build_executable(&program, &bin).expect("build");
    bin
}

/// Collect (origin, seq) for a given ekind (3=NET_SEND, 4=NET_DELIVER).
fn net_pairs(seg: &[u8], want_ekind: u32) -> Vec<(u32, u64)> {
    records(seg, want_ekind)
        .iter()
        .map(|(_, w1)| net_origin_seq(*w1))
        .collect()
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
            std::time::sleep(300ms);
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

/// Same cross-segment (origin, seq) pairing, but over a FRAMED
/// unix transport — the `lotus_transport_send` branch (distinct
/// from the raw-udp branch above). iris handoff-3 field re-test:
/// the transport branch was stamping origin 0 + a local counter;
/// this asserts it now carries the wire (origin, seq).
#[test]
fn framed_transport_net_pairs_on_origin_seq() {
    let sub_bin = compile("fsub", SUB);
    let pub_bin = compile("fpub", PUB);
    let dir = std::env::temp_dir().join(format!(
        "hale_obsnet_f_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("evt.sock");
    let sub_cfg = dir.join("sub.conf");
    let pub_cfg = dir.join("pub.conf");
    std::fs::write(
        &sub_cfg,
        format!("evt = unix://{}:listen
", sock.display()),
    )
    .unwrap();
    std::fs::write(
        &pub_cfg,
        format!("evt = unix://{}:connect
", sock.display()),
    )
    .unwrap();

    let mut sub = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_cfg)
        .env("LOTUS_OBS", "1")
        .env("LOTUS_OBS_WIRE", "1")
        .env("LOTUS_UNIX_STREAM", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sub");
    std::thread::sleep(Duration::from_millis(200));
    let mut pubc = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .env("LOTUS_OBS", "1")
        .env("LOTUS_OBS_WIRE", "1")
        .env("LOTUS_UNIX_STREAM", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn pub");
    let pub_pid = pubc.id();
    let sub_pid = sub.id();
    std::thread::sleep(Duration::from_millis(150));
    attach_observer(pub_pid);
    attach_observer(sub_pid);
    std::thread::sleep(Duration::from_millis(900));
    let pub_seg = snapshot_shm(pub_pid);
    let sub_seg = snapshot_shm(sub_pid);
    let sends = pub_seg.as_ref().map(|s| net_pairs(s, 3)).unwrap_or_default();
    let delivers =
        sub_seg.as_ref().map(|s| net_pairs(s, 4)).unwrap_or_default();
    let _ = pubc.kill();
    let _ = pubc.wait();
    let _ = sub.kill();
    let _ = sub.wait();
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(!sends.is_empty(), "framed publisher emitted no NET_SEND");
    assert!(
        sends.iter().all(|(o, _)| *o != 0),
        "transport-branch NET_SEND origin is 0 (the field bug): {:?}",
        sends
    );
    assert!(!delivers.is_empty(), "framed sub emitted no NET_DELIVER");
    let send_set: std::collections::HashSet<_> =
        sends.iter().copied().collect();
    let paired = delivers.iter().filter(|d| send_set.contains(d)).count();
    assert!(
        paired > 0,
        "transport branch: no NET_SEND/NET_DELIVER pairs on \
         (origin, seq).\nsends: {:?}\ndelivers: {:?}",
        sends,
        delivers
    );
}

#[test]
fn net_send_and_deliver_pair_on_origin_seq() {
    let sub_bin = compile("sub", SUB);
    let pub_bin = compile("pub", PUB);
    let port = 57811u16;
    let dir = std::env::temp_dir().join(format!(
        "hale_obsnet_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let sub_cfg = dir.join("sub.conf");
    let pub_cfg = dir.join("pub.conf");
    std::fs::write(
        &sub_cfg,
        format!("evt = udp://127.0.0.1:{}:listen\n", port),
    )
    .unwrap();
    std::fs::write(
        &pub_cfg,
        format!("evt = udp://127.0.0.1:{}:connect\n", port),
    )
    .unwrap();

    let mut sub = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_cfg)
        .env("LOTUS_OBS", "1")
        .env("LOTUS_OBS_WIRE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sub");
    std::thread::sleep(Duration::from_millis(200));
    let mut pubc = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .env("LOTUS_OBS", "1")
        .env("LOTUS_OBS_WIRE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn pub");
    let pub_pid = pubc.id();
    let sub_pid = sub.id();
    // Attach as observer to both so ring emission is on for the
    // publish burst (starts at pub t+300ms).
    std::thread::sleep(Duration::from_millis(150));
    attach_observer(pub_pid);
    attach_observer(sub_pid);
    // Burst runs pub t+300..500ms; App.run keeps the process alive
    // ~2.5s. Snapshot BOTH segments while alive (teardown
    // shm_unlinks at exit).
    std::thread::sleep(Duration::from_millis(900));
    let pub_seg = snapshot_shm(pub_pid);
    let sub_seg = snapshot_shm(sub_pid);

    let sends = pub_seg
        .as_ref()
        .map(|s| net_pairs(s, 3))
        .unwrap_or_default();
    let delivers = sub_seg
        .as_ref()
        .map(|s| net_pairs(s, 4))
        .unwrap_or_default();

    let _ = pubc.kill();
    let _ = pubc.wait();
    let _ = sub.kill();
    let _ = sub.wait();
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_dir_all(&dir);

    // P12: sends carry a nonzero origin (not the old unknown:0).
    assert!(!sends.is_empty(), "publisher emitted no NET_SEND");
    assert!(
        sends.iter().all(|(o, _)| *o != 0),
        "P12: NET_SEND origin is 0 (unattributed): {:?}",
        sends
    );
    assert!(!delivers.is_empty(), "subscriber emitted no NET_DELIVER");
    // P11: at least one exact (origin, seq) pair across segments.
    let send_set: std::collections::HashSet<_> =
        sends.iter().copied().collect();
    let paired = delivers.iter().filter(|d| send_set.contains(d)).count();
    assert!(
        paired > 0,
        "P11: no NET_SEND/NET_DELIVER pairs on (origin, seq).\n\
         sends: {:?}\ndelivers: {:?}",
        sends,
        delivers
    );
}
