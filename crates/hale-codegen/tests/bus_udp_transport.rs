//! 2026-05-26 — UDP bus transport (`udp://host:port`) end-to-end.
//! Single URL scheme covers both unicast and multicast: the
//! transport inspects the destination address and joins the
//! multicast group (IP_ADD_MEMBERSHIP) when it lands in
//! 224.0.0.0/4. Same `sendto` on the publisher side either way;
//! the kernel routes via the multicast tree for 224/4
//! destinations, regular path otherwise.
//!
//! Both tests spawn a subscriber binary (sleeps 1s while its
//! reader thread receives datagrams + dispatches them to the
//! local handler), then run a publisher binary that fires once
//! and exits. The subscriber's stdout is checked for the payload.

use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-bus-udp-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ));
    p
}

fn compile(tag: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag, "bin");
    build_executable(&program, &bin).expect("build");
    bin
}

fn subscriber_src() -> &'static str {
    r#"
        type Ping { n: Int; }
        locus Sub {
            bus {
                subscribe "evt" as on_evt of type Ping;
            }
            fn on_evt(p: Ping) {
                println("got n=", p.n);
            }
        }
        fn main() {
            Sub { };
            // Give the udp reader thread time to receive +
            // dispatch the publisher's datagram. The
            // cooperative scheduler drains the queue during
            // sleep ticks.
            std::time::sleep(800ms);
        }
    "#
}

fn publisher_src() -> &'static str {
    r#"
        type Ping { n: Int; }
        locus Pub {
            bus {
                publish "evt" of type Ping;
            }
            birth() {
                "evt" <- Ping { n: 4242 };
            }
        }
        fn main() {
            Pub { };
        }
    "#
}

fn run_pair(sub_bin: &PathBuf, pub_bin: &PathBuf,
            sub_cfg: &PathBuf, pub_cfg: &PathBuf) -> String
{
    // Subscriber spawned first so it binds before the publisher
    // sendto. Loopback delivery is essentially instant; the
    // 800ms sleep on the subscriber side covers the publisher
    // startup + sendto + reader-thread schedule latency.
    let sub = Command::new(sub_bin)
        .env("LOTUS_BUS_CONFIG", sub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");
    // Give the subscriber's reader thread ~100ms to bind the
    // socket + join the group (if multicast) before the
    // publisher sends.
    std::thread::sleep(Duration::from_millis(150));
    let pub_out = Command::new(pub_bin)
        .env("LOTUS_BUS_CONFIG", pub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run publisher");
    assert!(
        pub_out.status.success(),
        "publisher exited non-zero: {:?}\nstderr: {}",
        pub_out.status,
        String::from_utf8_lossy(&pub_out.stderr),
    );
    let sub_out = sub.wait_with_output().expect("wait subscriber");
    assert!(
        sub_out.status.success(),
        "subscriber exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        sub_out.status,
        String::from_utf8_lossy(&sub_out.stdout),
        String::from_utf8_lossy(&sub_out.stderr),
    );
    String::from_utf8_lossy(&sub_out.stdout).to_string()
}

#[test]
fn udp_unicast_delivers_payload_loopback() {
    let sub_bin = compile("uc_sub", subscriber_src());
    let pub_bin = compile("uc_pub", publisher_src());
    // Pick a port unlikely to collide; loopback so the actual
    // port doesn't matter much for routing.
    let port = 57781;
    let sub_cfg = unique_path("uc_sub", "conf");
    let pub_cfg = unique_path("uc_pub", "conf");
    std::fs::write(&sub_cfg, format!("evt = udp://127.0.0.1:{}:listen\n", port))
        .expect("write sub cfg");
    std::fs::write(&pub_cfg, format!("evt = udp://127.0.0.1:{}:connect\n", port))
        .expect("write pub cfg");

    let out = run_pair(&sub_bin, &pub_bin, &sub_cfg, &pub_cfg);

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_cfg);
    let _ = std::fs::remove_file(&pub_cfg);

    assert!(
        out.contains("got n=4242"),
        "subscriber should receive the unicast datagram; \
         stdout:\n{}",
        out
    );
}

#[test]
fn udp_multicast_delivers_payload_loopback() {
    // Multicast group in the administratively-scoped block
    // (239.0.0.0/8) — guaranteed local-scope, won't route off-
    // host even on misconfigured networks. IP_MULTICAST_LOOP
    // defaults to 1 on Linux IPv4 so the sender receives its
    // own packets on loopback.
    let sub_bin = compile("mc_sub", subscriber_src());
    let pub_bin = compile("mc_pub", publisher_src());
    let group = "239.255.77.77";
    let port  = 57783;
    let sub_cfg = unique_path("mc_sub", "conf");
    let pub_cfg = unique_path("mc_pub", "conf");
    std::fs::write(&sub_cfg, format!("evt = udp://{}:{}:listen\n", group, port))
        .expect("write sub cfg");
    std::fs::write(&pub_cfg, format!("evt = udp://{}:{}:connect\n", group, port))
        .expect("write pub cfg");

    let out = run_pair(&sub_bin, &pub_bin, &sub_cfg, &pub_cfg);

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_cfg);
    let _ = std::fs::remove_file(&pub_cfg);

    assert!(
        out.contains("got n=4242"),
        "subscriber should receive the multicast datagram \
         via the joined group; stdout:\n{}",
        out
    );
}

fn large_subscriber_src() -> &'static str {
    r#"
        type Big {
            counts: [Int; 100] = [0; 100];
            tag:    Int        = 0;
        }
        locus Sub {
            bus {
                subscribe "big" as on_big of type Big;
            }
            fn on_big(b: Big) {
                println("big_tag=", b.tag);
                println("big_first=", b.counts[0]);
                println("big_last=", b.counts[99]);
            }
        }
        fn main() {
            Sub { };
            std::time::sleep(800ms);
        }
    "#
}

fn large_publisher_src() -> &'static str {
    r#"
        type Big {
            counts: [Int; 100] = [0; 100];
            tag:    Int        = 0;
        }
        locus Pub {
            bus {
                publish "big" of type Big;
            }
            birth() {
                let mut b = Big { tag: 77 };
                b.counts[0]  = 1234567;
                b.counts[99] = 7654321;
                "big" <- b;
            }
        }
        fn main() {
            Pub { };
        }
    "#
}

fn book_snapshot_subscriber_src() -> &'static str {
    // BookSignalSnapshot-shaped: two Strings (variable-length
    // length-prefixed on the wire) ahead of a long tail of
    // fixed-size Decimals and Decimal arrays. The a downstream app
    // priceview crash report (2026-05-27) said inbound udp
    // datagrams trigger `g_bus_payload_arena` cap-hit on the
    // first message, with the diagnostic numbers matching a
    // single ~64 MB alloc from inside the deserializer.
    // Theory A in the handoff: a length-prefix misread
    // (decoded length taken as garbage from the wire) handed
    // unchecked to `lotus_bus_payload_arena_alloc`.
    //
    // This shape exercises the multi-variable-length-field
    // case that the existing single-Int / single-array UDP
    // tests don't cover.
    r#"
        type Book {
            symbol:        String  = "";
            venue_ts:      String  = "";
            best_bid:      Decimal = 0.0d;
            best_ask:      Decimal = 0.0d;
            bid_qty:       Decimal = 0.0d;
            ask_qty:       Decimal = 0.0d;
            mid:           Decimal = 0.0d;
            spread:        Decimal = 0.0d;
            probe_sizes:   [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            buy_costs:     [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            buy_filled:    [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            sell_proceeds: [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            sell_filled:   [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            vol_proxy:     Int = 0;
            snap_count:    Int = 0;
            delta_count:   Int = 0;
            checksum_ok:   Int = 1;
        }
        locus Sub {
            bus {
                subscribe "book" as on_book of type Book;
            }
            fn on_book(b: Book) {
                println("sym=", b.symbol);
                println("ts=", b.venue_ts);
                println("bid=", b.best_bid);
                println("ask=", b.best_ask);
                println("checksum_ok=", b.checksum_ok);
            }
        }
        fn main() {
            Sub { };
            std::time::sleep(800ms);
        }
    "#
}

fn book_snapshot_publisher_src() -> &'static str {
    r#"
        type Book {
            symbol:        String  = "";
            venue_ts:      String  = "";
            best_bid:      Decimal = 0.0d;
            best_ask:      Decimal = 0.0d;
            bid_qty:       Decimal = 0.0d;
            ask_qty:       Decimal = 0.0d;
            mid:           Decimal = 0.0d;
            spread:        Decimal = 0.0d;
            probe_sizes:   [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            buy_costs:     [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            buy_filled:    [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            sell_proceeds: [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            sell_filled:   [Decimal; 4] = [0.0d, 0.0d, 0.0d, 0.0d];
            vol_proxy:     Int = 0;
            snap_count:    Int = 0;
            delta_count:   Int = 0;
            checksum_ok:   Int = 1;
        }
        locus Pub {
            bus {
                publish "book" of type Book;
            }
            birth() {
                "book" <- Book {
                    symbol:   "BTC-USD",
                    venue_ts: "2026-05-27T10:00:00Z",
                    best_bid: 42000.5d,
                    best_ask: 42001.0d,
                    mid:      42000.75d,
                    spread:   0.5d,
                    checksum_ok: 1,
                };
            }
        }
        fn main() {
            Pub { };
        }
    "#
}

#[test]
fn udp_unicast_delivers_multi_string_payload() {
    // Reproduces the a downstream app priceview crash shape: a payload
    // with two variable-length Strings followed by a tail of
    // fixed-size fields. Without the deserializer length
    // bound-check, the first udp datagram would trigger the
    // g_bus_payload_arena cap-hit and SIGSEGV.
    let sub_bin = compile("book_sub", book_snapshot_subscriber_src());
    let pub_bin = compile("book_pub", book_snapshot_publisher_src());
    let port = 57787;
    let sub_cfg = unique_path("book_sub", "conf");
    let pub_cfg = unique_path("book_pub", "conf");
    std::fs::write(&sub_cfg, format!("book = udp://127.0.0.1:{}:listen\n", port))
        .expect("write sub cfg");
    std::fs::write(&pub_cfg, format!("book = udp://127.0.0.1:{}:connect\n", port))
        .expect("write pub cfg");

    let out = run_pair(&sub_bin, &pub_bin, &sub_cfg, &pub_cfg);

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_cfg);
    let _ = std::fs::remove_file(&pub_cfg);

    assert!(out.contains("sym=BTC-USD"),  "got: {:?}", out);
    assert!(out.contains("ts=2026-05-27T10:00:00Z"), "got: {:?}", out);
    assert!(out.contains("checksum_ok=1"), "got: {:?}", out);
}

#[test]
fn udp_unicast_delivers_payload_over_inline_threshold() {
    // [Int; 100] + Int = 808 bytes, well above
    // LOTUS_PAYLOAD_INLINE (512) so the receiver-side
    // local_dispatch routes through the heap-spill branch in
    // queue_enqueue. End-to-end: serialize → sendto → recvfrom
    // (heap buffer sized at LOTUS_PAYLOAD_MAX) → deserialize →
    // heap-spilled queue cell → drain → handler.
    let sub_bin = compile("uc_big_sub", large_subscriber_src());
    let pub_bin = compile("uc_big_pub", large_publisher_src());
    let port = 57785;
    let sub_cfg = unique_path("uc_big_sub", "conf");
    let pub_cfg = unique_path("uc_big_pub", "conf");
    std::fs::write(&sub_cfg, format!("big = udp://127.0.0.1:{}:listen\n", port))
        .expect("write sub cfg");
    std::fs::write(&pub_cfg, format!("big = udp://127.0.0.1:{}:connect\n", port))
        .expect("write pub cfg");

    let out = run_pair(&sub_bin, &pub_bin, &sub_cfg, &pub_cfg);

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_cfg);
    let _ = std::fs::remove_file(&pub_cfg);

    assert!(out.contains("big_tag=77"),     "got: {:?}", out);
    assert!(out.contains("big_first=1234567"), "got: {:?}", out);
    assert!(out.contains("big_last=7654321"),  "got: {:?}", out);
}

fn multi_listener_subscriber_src() -> &'static str {
    // 2026-05-27 — mirrors the a downstream app priceview shape: a
    // single subscriber binary with FOUR multicast listeners
    // on the same port, different groups, different payload
    // types. The reader threads each call into a different
    // deserialize_fn looked up by subject. If SO_REUSEPORT
    // cross-routes (a multicast packet for group A arrives
    // on the socket that joined group B), the lookup picks
    // the wrong deserializer and tries to interpret the
    // arriving bytes as the wrong type. With multi-String
    // payloads, this manifests as a giant decoded length
    // prefix → the 2026-05-27 deserialize bound-check now
    // rejects this cleanly via the `wire.fail` block.
    r#"
        type BookSnap {
            symbol:   String  = "";
            venue_ts: String  = "";
            mid:      Decimal = 0.0d;
        }
        type TickMsg {
            symbol:   String  = "";
            qty:      Decimal = 0.0d;
        }
        locus Sub {
            bus {
                subscribe "md.book.kraken"   as on_kraken_book   of type BookSnap;
                subscribe "md.book.coinbase" as on_coinbase_book of type BookSnap;
                subscribe "md.tick.kraken"   as on_kraken_tick   of type TickMsg;
                subscribe "md.tick.coinbase" as on_coinbase_tick of type TickMsg;
            }
            fn on_kraken_book(b: BookSnap) {
                println("kraken_book sym=", b.symbol);
            }
            fn on_coinbase_book(b: BookSnap) {
                println("coinbase_book sym=", b.symbol);
            }
            fn on_kraken_tick(t: TickMsg) {
                println("kraken_tick sym=", t.symbol);
            }
            fn on_coinbase_tick(t: TickMsg) {
                println("coinbase_tick sym=", t.symbol);
            }
        }
        fn main() {
            Sub { };
            std::time::sleep(1200ms);
        }
    "#
}

fn multi_listener_publisher_src() -> &'static str {
    r#"
        type BookSnap {
            symbol:   String  = "";
            venue_ts: String  = "";
            mid:      Decimal = 0.0d;
        }
        locus Pub {
            bus {
                publish "md.book.kraken" of type BookSnap;
            }
            birth() {
                "md.book.kraken" <- BookSnap {
                    symbol:   "BTC-USD",
                    venue_ts: "2026-05-27T10:00:00Z",
                    mid:      42000.5d,
                };
            }
        }
        fn main() {
            Pub { };
        }
    "#
}

#[test]
fn udp_multi_listener_same_port_different_groups() {
    // Stress the SO_REUSEPORT + multicast-group-membership
    // case that priceview uses in production. Four reader
    // threads, each bound to the same port and joined to a
    // distinct multicast group. A single publisher fires one
    // payload at the kraken-book group; the subscriber should
    // dispatch exactly one handler (on_kraken_book) and not
    // crash even if SO_REUSEPORT delivers the packet to a
    // socket that hadn't joined that group (which would mean
    // the deserialize lookup picks a deserialize_fn for a
    // different payload type and the bound-check is the only
    // thing standing between us and the 64 MB alloc).
    let sub_bin = compile("multi_sub", multi_listener_subscriber_src());
    let pub_bin = compile("multi_pub", multi_listener_publisher_src());
    let port = 57795;
    let sub_cfg = unique_path("multi_sub", "conf");
    let pub_cfg = unique_path("multi_pub", "conf");
    std::fs::write(&sub_cfg, format!(
        "md.book.kraken   = udp://239.42.1.1:{}:listen\n\
         md.book.coinbase = udp://239.42.1.2:{}:listen\n\
         md.tick.kraken   = udp://239.42.2.1:{}:listen\n\
         md.tick.coinbase = udp://239.42.2.2:{}:listen\n",
        port, port, port, port,
    )).expect("write sub cfg");
    std::fs::write(&pub_cfg, format!(
        "md.book.kraken = udp://239.42.1.1:{}:connect\n",
        port,
    )).expect("write pub cfg");

    let out = run_pair(&sub_bin, &pub_bin, &sub_cfg, &pub_cfg);

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_cfg);
    let _ = std::fs::remove_file(&pub_cfg);

    // Only the kraken_book handler should fire. The other 3
    // listeners are on distinct multicast groups; with the
    // bind-to-group fix (2026-05-27) per-(group, port)
    // endpoints are honored and each socket only receives
    // its own group's datagrams. Pre-fix the
    // bind-to-INADDR_ANY shape would fan this single
    // datagram to all 4 handlers — the crosstalk that
    // a downstream app reported.
    assert!(
        out.contains("kraken_book sym=BTC-USD"),
        "kraken_book handler should fire; stdout:\n{}",
        out
    );
    assert!(
        !out.contains("coinbase_book"),
        "coinbase_book must NOT fire (different group; \
         firing would indicate INADDR_ANY-bind crosstalk); \
         stdout:\n{}",
        out
    );
    assert!(
        !out.contains("kraken_tick"),
        "kraken_tick must NOT fire (different group); \
         stdout:\n{}",
        out
    );
    assert!(
        !out.contains("coinbase_tick"),
        "coinbase_tick must NOT fire (different group); \
         stdout:\n{}",
        out
    );
}

#[test]
fn udp_mixed_listen_connect_survives_realloc() {
    // 2026-05-27 — a downstream app priceview hit a silent SIGSEGV when
    // LOTUS_BUS_CONFIG mixed udp:// listen and udp:// connect
    // roles. Root cause: `g_bus_remote_entries` was an
    // array-of-structs with initial cap = 4. The udp listen
    // reader threads each captured `args->entry = &entries[N]`
    // at spawn time. When a subsequent register_remote call
    // grew the array (>4 entries), realloc could move the
    // storage, and the previously-spawned reader threads'
    // entry pointers became dangling. Matrix: 4 listens alone
    // = stable (fits in cap); 2 connects alone = stable;
    // 4 listens + 2 connects (6 → realloc) = silent SIGSEGV.
    //
    // Fix shipped 2026-05-27: g_bus_remote_entries is an
    // array of POINTERS to individually-malloc'd entries.
    // The array can realloc freely; per-entry addresses stay
    // stable.
    //
    // This test reproduces the priceview shape: more than 4
    // total entries with at least one listen and at least one
    // connect. Without the fix, the subscriber binary
    // segfaults on the first inbound datagram. With the fix,
    // the listen handlers fire normally.
    let sub_bin = compile("mixed_sub", r#"
        type Evt { sym: String = ""; }
        locus Sub {
            bus {
                subscribe "evt.a" as on_a of type Evt;
                subscribe "evt.b" as on_b of type Evt;
                subscribe "evt.c" as on_c of type Evt;
                subscribe "evt.d" as on_d of type Evt;
            }
            fn on_a(e: Evt) { println("a sym=", e.sym); }
            fn on_b(e: Evt) { println("b sym=", e.sym); }
            fn on_c(e: Evt) { println("c sym=", e.sym); }
            fn on_d(e: Evt) { println("d sym=", e.sym); }
        }
        fn main() {
            Sub { };
            println("READY");
            std::time::sleep(800ms);
            println("SURVIVED");
        }
    "#);
    let pub_bin = compile("mixed_pub", r#"
        type Evt { sym: String = ""; }
        locus Pub {
            bus {
                publish "evt.a" of type Evt;
            }
            birth() {
                "evt.a" <- Evt { sym: "FROM-A" };
            }
        }
        fn main() {
            Pub { };
        }
    "#);

    // Subscriber config: 4 udp listens + 2 udp connects = 6
    // remote entries, which forces a realloc past the initial
    // cap of 4. The connects don't actually need to be
    // reachable — just registered, to provoke the realloc.
    let port_a   = 57870;
    let port_b   = 57871;
    let port_c   = 57872;
    let port_d   = 57873;
    let port_out = 57874;
    let sub_cfg = unique_path("mixed_sub", "conf");
    let pub_cfg = unique_path("mixed_pub", "conf");
    std::fs::write(&sub_cfg, format!(
        "evt.a   = udp://127.0.0.1:{}:listen\n\
         evt.b   = udp://127.0.0.1:{}:listen\n\
         evt.c   = udp://127.0.0.1:{}:listen\n\
         evt.d   = udp://127.0.0.1:{}:listen\n\
         out.x   = udp://127.0.0.1:{}:connect\n\
         out.y   = udp://127.0.0.1:{}:connect\n",
        port_a, port_b, port_c, port_d, port_out, port_out + 1,
    )).expect("write sub cfg");
    std::fs::write(&pub_cfg, format!(
        "evt.a = udp://127.0.0.1:{}:connect\n",
        port_a,
    )).expect("write pub cfg");

    let out = run_pair(&sub_bin, &pub_bin, &sub_cfg, &pub_cfg);

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
    let _ = std::fs::remove_file(&sub_cfg);
    let _ = std::fs::remove_file(&pub_cfg);

    assert!(out.contains("READY"),    "stdout: {}", out);
    assert!(out.contains("a sym=FROM-A"),
        "the kraken-shaped listen handler must fire — \
         entry-pointer should survive the realloc that the \
         2 connect entries trigger; stdout:\n{}", out);
    assert!(out.contains("SURVIVED"),
        "subscriber must not segfault after dispatch; \
         stdout:\n{}", out);
}

#[test]
fn udp_corrupt_length_prefix_rejected_cleanly() {
    // 2026-05-27 — proves the deserialize bound-check (added
    // alongside the a downstream app priceview crash report) actually
    // engages. We bypass the publisher binary entirely and
    // send a malformed datagram straight from the test
    // harness: 8 bytes encoding length-prefix = 0x04000000
    // (= 64 MB, exactly the value that triggered priceview's
    // `g_bus_payload_arena` cap-hit symptom).
    //
    // Without the bound-check, the subscriber's deserialize
    // would hand 64 MB to `lotus_bus_payload_arena_alloc`,
    // hit the arena cap, return NULL, then dereference NULL
    // in the subsequent memcpy → SIGSEGV in seconds.
    //
    // With the check, the deserialize compares the decoded
    // length against the wire size (8 bytes), the UGT
    // comparison fires (67108864 > 8), and the deserialize
    // returns -1. The reader thread's `if (struct_size <= 0)
    // continue;` drops the datagram silently. Subscriber
    // stays alive.
    let sub_bin = compile("corrupt_sub", r#"
        type Msg {
            symbol: String  = "";
            mid:    Decimal = 0.0d;
        }
        locus Sub {
            bus {
                subscribe "corrupt" as on_msg of type Msg;
            }
            fn on_msg(m: Msg) {
                // Should NEVER fire — the malformed wire bytes
                // are rejected at deserialize time, before any
                // dispatch happens.
                println("FIRED sym=", m.symbol);
            }
        }
        fn main() {
            Sub { };
            println("READY");
            std::time::sleep(800ms);
            println("SURVIVED");
        }
    "#);

    let port = 57799;
    let sub_cfg = unique_path("corrupt_sub", "conf");
    std::fs::write(&sub_cfg, format!("corrupt = udp://127.0.0.1:{}:listen\n", port))
        .expect("write sub cfg");

    let sub = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");

    // Give the reader thread time to bind.
    std::thread::sleep(Duration::from_millis(200));

    // Send the malformed datagram. The 8 bytes are the LE
    // encoding of i64=67108864 — what priceview observed in
    // the wild.
    let sock = UdpSocket::bind("127.0.0.1:0").expect("test sock");
    let bad = [0x00u8, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00];
    sock.send_to(&bad, format!("127.0.0.1:{}", port))
        .expect("send corrupt datagram");

    let sub_out = sub.wait_with_output().expect("wait subscriber");

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&sub_cfg);

    assert!(
        sub_out.status.success(),
        "subscriber should survive the malformed datagram; \
         status: {:?}\nstdout: {}\nstderr: {}",
        sub_out.status,
        String::from_utf8_lossy(&sub_out.stdout),
        String::from_utf8_lossy(&sub_out.stderr),
    );
    let stdout = String::from_utf8_lossy(&sub_out.stdout);
    assert!(stdout.contains("READY"),    "stdout: {}", stdout);
    assert!(stdout.contains("SURVIVED"), "stdout: {}", stdout);
    assert!(
        !stdout.contains("FIRED"),
        "handler must not fire on a malformed payload; stdout: {}",
        stdout,
    );
}
