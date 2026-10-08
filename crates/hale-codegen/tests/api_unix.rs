//! GH #1417 (R2b): `unix::Rpc`, the first socket transport of `api::serve`.
//!
//! The witness (`tests/api-contract/program.hl`) serves `Admin` over a Unix
//! socket. These tests build that exposure as a program, serve it over a
//! real socket, and drive it with a client process's worth of connections:
//!
//! * the recorded exchanges of `tests/api-contract/wire/unix/` are
//!   replayed: each request line is sent and each reply is held to the
//!   recorded one byte for byte, apart from the request id the exposure
//!   assigns and the peer's own uid, gid and pid (the recording is of a
//!   peer 1000/1000/4242);
//! * a connection that fails (a line that is not an object, an EOF in the
//!   middle of a request) ends that connection, and another connection to
//!   the same listener goes on calling.
//!
//! One recorded exchange cannot be replayed from a client: the
//! `unauthenticated` one, whose peer the kernel will not name. A connected
//! `AF_UNIX` socket always has credentials, so its pieces are held where
//! they can be driven (`tests/hale/api/unix_wire_test.hl`).

use std::path::PathBuf;
use std::sync::OnceLock;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/unix_rpc.rs"]
mod unix_rpc;

use unix_rpc::*;

/// The witness's `Admin` half: the contract program's types, rows, handlers
/// and role source, served over `unix::Rpc` on `$SOCK`, `@BOUND@` requests
/// at a time. `Ledger::rebalance` with the book `slow` holds the call for
/// `$SLOW_MS`. The program stops its handle when `$TRIGGER` appears.
pub const ADMIN: &str = r#"
role operator;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;

type CancelOrder { order: OrderId; }
type Cancelled { order: OrderId; was_open: Bool; }
type OrderError { code: String; reason: String; }
type Rebalance { book: String; }
type Rebalanced { moved: Money; }

api Admin {
    rpc Orders::cancel requires: [operator];
    rpc Ledger::rebalance requires: [operator];
}

locus Grants {
    params {
        operator: String = "";
    }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "operator" { return len(self.operator) > 0 && p.name == self.operator; }
        return false;
    }
}

locus Orders {
    params {
        next: Int = 42;
        open: Int = 0;
    }
    fn cancel(c: CancelOrder, ctx: std::api::Context) -> Cancelled fallible(OrderError) {
        let n = Int(c.order);
        if n < 41 || n >= self.next {
            fail OrderError { code: "unknown_order", reason: "no order " + to_string(n) };
        }
        self.open = self.open - 1;
        return Cancelled { order: c.order, was_open: true };
    }
}

locus Ledger {
    params {
        moved: Int = 0;
    }
    closure books_balanced { captures: moved; epoch inline; }
    fn rebalance(r: Rebalance) -> Rebalanced fallible(ClosureViolation) {
        if len(r.book) == 0 { violate books_balanced; }
        if r.book == "slow" {
            let ms = std::str::parse_int(std::env::var("SLOW_MS")) or 0;
            let mut i = 0;
            while i < ms {
                std::time::sleep(1ms);
                i = i + 1;
            }
        }
        self.moved = self.moved + 1500;
        return Rebalanced { moved: 1500cent };
    }
}

main locus Desk {
    params {
        admin_roles: Grants = Grants { operator: std::env::var("OPERATOR") };
        orders: Orders = Orders { };
        ledger: Ledger = Ledger { };
    }
    placement {
        orders: cooperative(pool = desk) where async_io;
        ledger: cooperative(pool = desk) where async_io;
    }
    on_failure(l: Ledger, err: ClosureViolation) { }
    run() {
        let admin = api::serve(Admin, unix::Rpc { path: std::env::var("SOCK"), roles: self.admin_roles }, as: "admin", bound: @BOUND@, on_full: refuse);
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {
            std::time::sleep(10ms);
        }
        admin.stop();
        println("stopped");
    }
}

fn main() { Desk { }; }
"#;

/// The program with `bound` requests at a time, built once per process.
fn admin(bound: u32) -> PathBuf {
    static BUILT: OnceLock<std::sync::Mutex<std::collections::BTreeMap<u32, PathBuf>>> = OnceLock::new();
    let map = BUILT.get_or_init(Default::default);
    let mut map = map.lock().unwrap();
    map.entry(bound)
        .or_insert_with(|| {
            let bin = harness::unique_bin(&format!("api_unix_admin{bound}"));
            let src = ADMIN.replace("@BOUND@", &bound.to_string());
            build_opts::build_source(&src, &bin, &build_opts::options()).expect("build the Admin program");
            bin
        })
        .clone()
}

/// The text of a recorded exchange, `member` of the object after `key`,
/// whitespace outside strings dropped: the line as the wire carries it.
fn compact_object_after(text: &str, key: &str) -> String {
    let at = text.find(&format!("\"{key}\":")).unwrap_or_else(|| panic!("no `{key}` in the recording"));
    let rest = &text[at + key.len() + 3..];
    let start = rest.find('{').expect("an object");
    let mut depth = 0;
    let mut in_str = false;
    let mut esc = false;
    let mut out = String::new();
    for c in rest[start..].chars() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            c if c.is_whitespace() => {}
            '{' => {
                depth += 1;
                out.push(c);
            }
            '}' => {
                depth -= 1;
                out.push(c);
                if depth == 0 {
                    return out;
                }
            }
            c => out.push(c),
        }
    }
    panic!("an unterminated object in the recording");
}

fn recording(name: &str) -> (String, String, String) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/api-contract/wire/unix").join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let peer = compact_object_after(&text, "peer");
    (compact_object_after(&text, "request"), compact_object_after(&text, "reply"), peer)
}

/// The caller members a recording shows for its peer (`{"uid":1000,
/// "gid":1000,"pid":4242}`).
fn recorded_caller(peer: &str) -> String {
    let field = |k: &str| {
        let at = peer.find(&format!("\"{k}\":")).unwrap() + k.len() + 3;
        peer[at..].chars().take_while(|c| *c != ',' && *c != '}').collect::<String>()
    };
    format!("\"name\":\"uid:{}\",\"uid\":{},\"gid\":{},\"pid\":{}", field("uid"), field("uid"), field("gid"), field("pid"))
}

/// Hold `got` to the recorded `reply`: equal byte for byte once the request
/// id is masked and this test's own uid, gid and pid stand for the
/// recording's.
fn assert_recorded(name: &str, got: &str, reply: &str, peer: &str) {
    let (uid, gid, pid) = me();
    let actual_caller = format!("\"name\":\"uid:{uid}\",\"uid\":{uid},\"gid\":{gid},\"pid\":{pid}");
    assert!(got.contains(&actual_caller), "{name}: the caller is this process's own credentials:\n{got}");
    let masked = mask_request_id(&got.replace(&actual_caller, &recorded_caller(peer)));
    assert_eq!(masked, mask_request_id(reply), "{name}: the reply differs from the recorded exchange");
}

fn me_operator() -> String {
    format!("uid:{}", me().0)
}

/// Replay `names` in order over one connection to a program whose operator
/// is `operator`.
fn replay(server: &Server, names: &[&str]) {
    let mut c = server.connect();
    for name in names {
        let (request, reply, peer) = recording(name);
        let got = c.ask(&request);
        assert_recorded(name, &got, &reply, &peer);
    }
}

#[test]
fn the_recorded_unix_exchanges_replay_byte_for_byte() {
    // a result, the handler's own error, a payload of the wrong shape, a
    // digest that is not the served one, the server error of a violating
    // handler, and the next call to that handler's type: unavailable. (The
    // digest in the requests is the contract's, so the program serves the
    // contract's digest: the types and rows above are `program.hl`'s.)
    let operator = me_operator();
    let server = Server::start(&admin(16), &[("OPERATOR", &operator)]);
    replay(&server, &["result", "handler_error", "refusal_malformed", "refusal_digest_mismatch", "server_error", "refusal_unavailable"]);
    let done = server.finish();
    assert!(done.status.success(), "the program ended cleanly: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("stopped"), "stop() returned:\n{}", done.stdout);
}

#[test]
fn a_peer_who_does_not_hold_the_role_is_refused_as_recorded() {
    // the recording's peer is uid 1001 against an operator uid:1000: any
    // other uid is the same refusal
    let someone_else = format!("uid:{}", me().0 + 1);
    let server = Server::start(&admin(16), &[("OPERATOR", &someone_else)]);
    replay(&server, &["refusal_unauthorized"]);
    assert!(server.finish().status.success());
}

#[test]
fn a_request_over_the_bound_is_refused_full_as_recorded() {
    // bound 1: a slow call holds the only place, and the next request on
    // the same connection is refused while it runs
    let operator = me_operator();
    let server = Server::start(&admin(1), &[("OPERATOR", &operator), ("SLOW_MS", "600")]);
    let mut c = server.connect();
    let digest = "fnv1a64:40381db6685c9f75";
    c.send(&format!("{{\"call\":\"Ledger::rebalance\",\"payload\":{{\"book\":\"slow\"}},\"id\":\"c-6\",\"digest\":\"{digest}\"}}"));
    std::thread::sleep(std::time::Duration::from_millis(150));
    let (request, reply, peer) = recording("refusal_full");
    let got = c.ask(&request);
    assert_recorded("refusal_full", &got, &reply, &peer);
    let slow = c.line();
    assert!(slow.contains("\"id\":\"c-6\",\"ok\":true"), "the slow call completed: {slow}");
    assert!(server.finish().status.success());
}

#[test]
fn a_request_after_stop_began_is_refused_shutting_down_as_recorded() {
    // a slow call is executing when stop() begins; a request that arrives
    // while stop() waits for it is refused, and the executing call is
    // answered before the connection closes
    let operator = me_operator();
    let server = Server::start(&admin(16), &[("OPERATOR", &operator), ("SLOW_MS", "800")]);
    let mut c = server.connect();
    let digest = "fnv1a64:40381db6685c9f75";
    c.send(&format!("{{\"call\":\"Ledger::rebalance\",\"payload\":{{\"book\":\"slow\"}},\"id\":\"c-7\",\"digest\":\"{digest}\"}}"));
    std::thread::sleep(std::time::Duration::from_millis(200));
    server.trigger();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (request, reply, peer) = recording("refusal_shutting_down");
    let got = c.ask(&request);
    assert_recorded("refusal_shutting_down", &got, &reply, &peer);
    let slow = c.line();
    assert!(slow.contains("\"id\":\"c-7\",\"ok\":true"), "the executing call was answered before stop() returned: {slow}");
    assert!(c.closed_within(5000), "and then the connection closes");
    let mut server = server;
    let done = server.wait(std::time::Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

#[test]
fn a_connection_that_fails_ends_that_connection_and_the_listener_serves_on() {
    let operator = me_operator();
    let server = Server::start(&admin(16), &[("OPERATOR", &operator)]);
    let call = |id: &str| {
        format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":41}},\"id\":\"{id}\"}}")
    };
    let mut steady = server.connect();
    let first = steady.ask(&call("s-1"));
    assert!(first.contains("\"id\":\"s-1\",\"ok\":true"), "{first}");

    // a line that is not an object: answered as malformed (request id 0),
    // and this connection is over
    let mut bad = server.connect();
    let reply = bad.ask("this is not json");
    assert!(reply.starts_with("{\"request_id\":0,\"id\":null,\"ok\":false,\"refusal\":{\"kind\":\"malformed\""), "{reply}");
    assert!(bad.closed_within(5000), "a malformed line ends its connection");

    // an EOF in the middle of a line: the connection is gone, nobody is answered
    let mut cut = server.connect();
    cut.send_raw(b"{\"call\":\"Orders::cancel\",\"payl");
    cut.close();

    // a client that vanishes right after a complete request
    let mut gone = server.connect();
    gone.send(&call("g-1"));
    gone.close();

    // the first connection, and a new one, still call
    let second = steady.ask(&call("s-2"));
    assert!(second.contains("\"id\":\"s-2\",\"ok\":true"), "an open connection is untouched: {second}");
    let mut fresh = server.connect();
    let third = fresh.ask(&call("f-1"));
    assert!(third.contains("\"id\":\"f-1\",\"ok\":true"), "the listener accepts a new connection: {third}");

    // request ids increase for the exposure, across its connections
    let (a, b, c) = (request_id_of(&first), request_id_of(&second), request_id_of(&third));
    assert!(0 < a && a < b && b < c, "request ids are unique and increasing: {a} {b} {c}");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}
