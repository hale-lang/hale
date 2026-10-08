//! GH #1417 (R3): the witness's rpc half, end to end, over HTTP and Unix.
//!
//! One program, the contract's own `Desk` (`tests/api-contract/program.hl`
//! less its stream half): `Public` served twice over `http::Rpc` (`public`
//! and `partner`, on two ports, under two role sources, bound to two
//! `Orders` instances on two pools) and `Admin` over `unix::Rpc` (`admin`),
//! in one process. The tests assert what a caller can observe:
//!
//! * every member reaches its own receiver (the order ids each instance
//!   hands out) and its own pool (a call that holds one pool's thread does
//!   not stop the other pool's receiver);
//! * a handler two surfaces share meets each surface's `requires`;
//! * a digest mismatch names the served digest, on each transport;
//! * every outcome, on each transport;
//! * `stop()` of one exposure leaves the others serving;
//! * the lifecycle rows that belong to a socket: a connection that breaks
//!   while the others are served, and a client that goes away after its
//!   call started (the work runs once, the bound is released once).

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;
#[path = "support/http_rpc.rs"]
mod http_rpc;
#[path = "support/unix_rpc.rs"]
mod unix_rpc;

use http_rpc::*;

/// The contract's `Desk`, less the hub. `$BIND`, `$BIND2` and `$SOCK` are
/// the three listeners, `$OPERATOR` the Unix peer who holds `operator`, and
/// `$TRIGGER_PUBLIC`, `$TRIGGER_PARTNER`, `$TRIGGER_ADMIN` and `$TRIGGER`
/// stop one exposure or all that remain. `Orders::place` is held for
/// `$SLOW_MS` when its symbol is `SLOW`, and holds its pool's thread for
/// `$BUSY_MS` when its symbol is `BUSY`.
pub const DESK: &str = r#"
role trader;
role operator;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;

type PlaceOrder { symbol: String; qty: Int; limit: Money; }
type OrderReceipt { order: OrderId; notional: Money; }
type CancelOrder { order: OrderId; }
type Cancelled { order: OrderId; was_open: Bool; }
type OrderError { code: String; reason: String; }
type Rebalance { book: String; }
type Rebalanced { moved: Money; }

api Public {
    rpc Orders::place;
    rpc Orders::cancel requires: [trader];
}

api Admin {
    rpc Orders::cancel requires: [operator];
    rpc Ledger::rebalance requires: [operator];
}

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-alice" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        if token == "t-bob" { return std::api::Principal { mode: "bearer", name: "bob" }; }
        if token == "t-carol" { return std::api::Principal { mode: "bearer", name: "carol" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Grants {
    params {
        trader: String = "";
        operator: String = "";
    }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "trader" { return len(self.trader) > 0 && p.name == self.trader; }
        if r == "operator" { return len(self.operator) > 0 && p.name == self.operator; }
        return false;
    }
}

@unbounded
fn hold_for(ms: Int) {
    let mut i = 0;
    while i < ms {
        std::time::sleep(1ms);
        i = i + 1;
    }
}

// holds the thread of the pool it runs on, as a busy handler does
@unbounded
fn spin_for(ms: Int) {
    let until = std::time::monotonic_ns() + ms * 1000000;
    let mut now = std::time::monotonic_ns();
    while now < until { now = std::time::monotonic_ns(); }
}

locus Orders {
    params {
        next: Int = 41;
        open: Int = 0;
    }
    closure position_limit { captures: open; epoch inline; }

    fn place(o: PlaceOrder) -> OrderReceipt fallible(ClosureViolation) {
        if o.qty > 10000 { violate position_limit; }
        if o.symbol == "SLOW" {
            println("place started");
            hold_for(std::str::parse_int(std::env::var("SLOW_MS")) or 0);
            println("place finished");
        }
        if o.symbol == "BUSY" { spin_for(std::str::parse_int(std::env::var("BUSY_MS")) or 0); }
        let id = OrderId(self.next);
        self.next = self.next + 1;
        self.open = self.open + 1;
        return OrderReceipt { order: id, notional: o.limit * o.qty };
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
    params { moved: Int = 0; }
    closure books_balanced { captures: moved; epoch inline; }

    fn rebalance(r: Rebalance) -> Rebalanced fallible(ClosureViolation) {
        if len(r.book) == 0 { violate books_balanced; }
        self.moved = self.moved + 1500;
        return Rebalanced { moved: 1500cent };
    }
}

fn written(path: String) -> Bool {
    return (std::io::fs::read_file(path) or "") != "";
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        public_roles: Grants = Grants { trader: "alice" };
        partner_roles: Grants = Grants { trader: "carol" };
        admin_roles: Grants = Grants { operator: std::env::var("OPERATOR") };
        orders: Orders = Orders { };
        partner_orders: Orders = Orders { next: 9001 };
        ledger: Ledger = Ledger { };
        orders_failed: Int = 0;
        ledger_failed: Int = 0;
    }
    placement {
        orders: cooperative(pool = desk) where async_io;
        partner_orders: cooperative(pool = partner) where async_io;
        ledger: cooperative(pool = desk) where async_io;
    }
    on_failure(o: Orders, err: ClosureViolation) { self.orders_failed = self.orders_failed + 1; }
    on_failure(l: Ledger, err: ClosureViolation) { self.ledger_failed = self.ledger_failed + 1; }

    run() {
        let public = api::serve(Public, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: @BOUND@, on_full: refuse);
        let partner = api::serve(Public, http::Rpc { bind: std::env::var("BIND2"), codec: json, principals: self.bearer, roles: self.partner_roles }, as: "partner", receivers: { Orders: self.partner_orders }, bound: @BOUND@, on_full: refuse);
        let admin = api::serve(Admin, unix::Rpc { path: std::env::var("SOCK"), roles: self.admin_roles }, as: "admin", receivers: { Orders: self.orders }, bound: 16, on_full: refuse);
        let tp = std::env::var("TRIGGER_PUBLIC");
        let tq = std::env::var("TRIGGER_PARTNER");
        let ta = std::env::var("TRIGGER_ADMIN");
        let all = std::env::var("TRIGGER");
        let mut public_up = true;
        let mut partner_up = true;
        let mut admin_up = true;
        while !self.draining && !written(all) {
            if public_up && written(tp) { public.stop(); public_up = false; println("public stopped"); }
            if partner_up && written(tq) { partner.stop(); partner_up = false; println("partner stopped"); }
            if admin_up && written(ta) { admin.stop(); admin_up = false; println("admin stopped"); }
            std::time::sleep(10ms);
        }
        public.stop();
        partner.stop();
        admin.stop();
        println("stopped");
        println("orders_failed=" + to_string(self.orders_failed));
        println("ledger_failed=" + to_string(self.ledger_failed));
    }
}

fn main() { Desk { }; }
"#;

/// The program with `bound` requests at a time on each HTTP exposure, built
/// once per process.
fn desk(bound: u32) -> PathBuf {
    static BUILT: OnceLock<std::sync::Mutex<std::collections::BTreeMap<u32, PathBuf>>> = OnceLock::new();
    let map = BUILT.get_or_init(Default::default);
    let mut map = map.lock().unwrap();
    map.entry(bound)
        .or_insert_with(|| {
            let bin = harness::unique_bin(&format!("api_http_witness{bound}"));
            let src = DESK.replace("@BOUND@", &bound.to_string());
            build_opts::build_source(&src, &bin, &build_opts::options()).expect("build the Desk program");
            bin
        })
        .clone()
}

const PUBLIC_DIGEST: &str = "fnv1a64:a8930d6e7998e986";
const ADMIN_DIGEST: &str = "fnv1a64:40381db6685c9f75";

fn me_operator() -> String {
    format!("uid:{}", unix_rpc::me().0)
}

fn place(port: u16, token: &str, symbol: &str) -> Response {
    let body = format!("{{\"symbol\":\"{symbol}\",\"qty\":10,\"limit\":12500}}");
    request(port, "POST", "/call/Orders::place", &[bearer(token), h("Hale-Surface-Digest", PUBLIC_DIGEST)], Some(&body))
}

fn cancel(port: u16, token: &str, order: i64) -> Response {
    let body = format!("{{\"order\":{order}}}");
    request(port, "POST", "/call/Orders::cancel", &[bearer(token), h("Hale-Surface-Digest", PUBLIC_DIGEST)], Some(&body))
}

fn admin(server: &Server, line: &str) -> String {
    unix_ask(&server.sock(), line)
}

fn start(bound: u32, env: &[(&str, &str)]) -> Server {
    let operator = me_operator();
    let mut all: Vec<(&str, &str)> = vec![("OPERATOR", &operator)];
    all.extend_from_slice(env);
    let server = Server::start(&desk(bound), &all);
    server.ready();
    wait_listening(server.port2);
    wait_accepting_unix(&server.sock());
    server
}

#[test]
fn every_member_reaches_its_own_receiver_and_its_own_pool() {
    let server = start(64, &[("BUSY_MS", "700")]);
    // two instances of one receiver type, one per exposure: each hands out
    // its own ids
    let a = place(server.port, "t-alice", "ACME");
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(a.body, "{\"order\":41,\"notional\":125000}");
    let c = place(server.port2, "t-carol", "ACME");
    assert_eq!(c.status, 200, "{}", c.body);
    assert_eq!(c.body, "{\"order\":9001,\"notional\":125000}");
    // each exposure's cancel finds only its own instance's orders
    let miss = cancel(server.port, "t-alice", 9001);
    assert_eq!(miss.status, 422, "public's Orders never placed 9001: {}", miss.body);
    assert_eq!(miss.body, "{\"code\":\"unknown_order\",\"reason\":\"no order 9001\"}");
    let hit = cancel(server.port2, "t-carol", 9001);
    assert_eq!(hit.status, 200, "partner's Orders placed 9001: {}", hit.body);
    // the admin exposure is bound to the same instance as `public`: it
    // cancels the order public placed, over the unix socket
    let line = admin(&server, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":41}},\"id\":\"a-1\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"id\":\"a-1\",\"ok\":true,\"value\":{\"order\":41,\"was_open\":true}"), "{line}");
    let line = admin(&server, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":9001}},\"id\":\"a-2\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"ok\":false,\"error\":{\"code\":\"unknown_order\""), "admin's Orders is public's, which never placed 9001: {line}");

    // pools: partner's Orders runs on the pool `partner`. A call that holds
    // that pool's thread for 700 ms does not stop public's receiver (pool
    // `desk`) from answering.
    let busy = send(server.port2, &request_text("POST", "/call/Orders::place", &[bearer("t-carol"), h("Hale-Surface-Digest", PUBLIC_DIGEST)], Some("{\"symbol\":\"BUSY\",\"qty\":10,\"limit\":12500}")));
    std::thread::sleep(Duration::from_millis(150));
    let started = Instant::now();
    let quick = place(server.port, "t-alice", "ACME");
    assert_eq!(quick.status, 200, "{}", quick.body);
    assert!(started.elapsed() < Duration::from_millis(500), "public's pool answered while partner's was held: {:?}", started.elapsed());
    let done = busy.response_within(15_000).expect("the held call is answered");
    assert_eq!(done.status, 200, "{}", done.body);
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

#[test]
fn a_shared_handler_meets_each_surfaces_requires() {
    let server = start(64, &[]);
    let order = place(server.port, "t-alice", "ACME");
    assert_eq!(order.status, 200, "{}", order.body);
    // `Orders::cancel` requires `trader` through `Public` and `operator`
    // through `Admin`, and each exposure reads its own role source
    // alice holds trader under public, not under partner, whose trader is carol
    assert_eq!(cancel(server.port, "t-alice", 41).status, 200);
    let denied = cancel(server.port2, "t-alice", 41);
    assert_eq!(denied.status, 403, "{}", denied.body);
    assert_eq!(denied.body, "{\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"Orders::cancel requires trader\",\"requires\":[\"trader\"]}}");
    let bob = cancel(server.port, "t-bob", 41);
    assert_eq!(bob.status, 403, "{}", bob.body);
    // the unix exposure: the program's operator is this process's uid, who holds no trader
    // grant anywhere and is no bearer; its `requires` is operator
    let line = admin(&server, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":999}},\"id\":\"a-1\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"ok\":false,\"error\":{\"code\":\"unknown_order\""), "the operator is let through to the handler: {line}");
    // the descriptions differ with the exposure's source: carol may cancel through partner, not public
    let d = request(server.port2, "GET", "/.description", &[bearer("t-carol")], None);
    assert!(d.body.contains("\"name\":\"Orders::cancel\""), "{}", d.body);
    let d = request(server.port, "GET", "/.description", &[bearer("t-carol")], None);
    assert!(!d.body.contains("\"name\":\"Orders::cancel\""), "carol holds nothing under public: {}", d.body);
    assert!(d.body.contains("\"roles\":[]"), "{}", d.body);
    assert!(server.finish().status.success());
}

#[test]
fn a_digest_mismatch_names_the_served_digest_on_each_transport() {
    let server = start(64, &[]);
    for (port, served) in [(server.port, PUBLIC_DIGEST), (server.port2, PUBLIC_DIGEST)] {
        let got = request(port, "POST", "/call/Orders::place", &[bearer("t-alice"), h("Hale-Surface-Digest", "fnv1a64:0123456789abcdef")], Some("{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}"));
        assert_eq!(got.status, 409, "{}", got.body);
        assert_eq!(
            got.body,
            format!("{{\"refusal\":{{\"kind\":\"digest_mismatch\",\"reason\":\"the request was generated against fnv1a64:0123456789abcdef\",\"served\":\"{served}\"}}}}")
        );
    }
    let line = admin(&server, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"book\":\"main\"},\"id\":\"m-1\",\"digest\":\"fnv1a64:0123456789abcdef\"}");
    assert!(line.contains(&format!("\"refusal\":{{\"kind\":\"digest_mismatch\",\"reason\":\"the request was generated against fnv1a64:0123456789abcdef\",\"served\":\"{ADMIN_DIGEST}\"}}")), "{line}");
    // the digest is optional: a request that sends none is not refused for it
    let none = request(server.port, "POST", "/call/Orders::place", &[bearer("t-alice")], Some("{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}"));
    assert_eq!(none.status, 200, "{}", none.body);
    assert!(server.finish().status.success());
}

#[test]
fn every_outcome_on_each_transport() {
    let server = start(64, &[]);
    // the HTTP exposures (public is replayed byte for byte in api_http.rs; the
    // second serving of the surface answers the same five outcomes)
    let result = place(server.port2, "t-carol", "ACME");
    assert_eq!((result.status, result.body.as_str()), (200, "{\"order\":9001,\"notional\":125000}"));
    let handler_error = cancel(server.port2, "t-carol", 99999);
    assert_eq!((handler_error.status, handler_error.body.as_str()), (422, "{\"code\":\"unknown_order\",\"reason\":\"no order 99999\"}"));
    let refused = cancel(server.port2, "t-bob", 9001);
    assert_eq!(refused.status, 403, "{}", refused.body);
    let malformed = request(server.port2, "POST", "/call/Orders::place", &[bearer("t-carol")], Some("{\"symbol\":\"ACME\",\"limit\":12500}"));
    assert_eq!((malformed.status, malformed.body.as_str()), (400, "{\"refusal\":{\"kind\":\"malformed\",\"reason\":\"missing_field: qty\"}}"));
    let nobody = place(server.port2, "t-mallory", "ACME");
    assert_eq!((nobody.status, nobody.body.as_str()), (401, "{\"refusal\":{\"kind\":\"unauthenticated\",\"reason\":\"no such token\"}}"));
    let violation = request(server.port2, "POST", "/call/Orders::place", &[bearer("t-carol")], Some("{\"symbol\":\"ACME\",\"qty\":50000,\"limit\":12500}"));
    assert_eq!((violation.status, violation.body.as_str()), (500, "{\"refusal\":{\"kind\":\"server\"}}"));
    let after = place(server.port2, "t-carol", "ACME");
    assert_eq!(after.status, 503, "the receiver that violated is unavailable: {}", after.body);
    assert_eq!(after.body, "{\"refusal\":{\"kind\":\"unavailable\",\"reason\":\"the receiver of Orders::place is unavailable; the call did not run\"}}");
    // ... while public's instance is another receiver, untouched by it
    assert_eq!(place(server.port, "t-alice", "ACME").status, 200);

    // the unix exposure: result, handler error, refusal and server error
    let line = admin(&server, &format!("{{\"call\":\"Ledger::rebalance\",\"payload\":{{\"book\":\"main\"}},\"id\":\"u-1\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"id\":\"u-1\",\"ok\":true,\"value\":{\"moved\":1500}"), "{line}");
    let line = admin(&server, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":999}},\"id\":\"u-2\"}}"));
    assert!(line.contains("\"id\":\"u-2\",\"ok\":false,\"error\":{\"code\":\"unknown_order\",\"reason\":\"no order 999\"}"), "{line}");
    let line = admin(&server, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"nope\":1},\"id\":\"u-3\"}");
    assert!(line.contains("\"id\":\"u-3\",\"ok\":false,\"refusal\":{\"kind\":\"malformed\""), "{line}");
    let line = admin(&server, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"book\":\"\"},\"id\":\"u-4\"}");
    assert!(line.contains("\"id\":\"u-4\",\"ok\":false,\"refusal\":{\"kind\":\"server\"}"), "{line}");
    let line = admin(&server, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"book\":\"main\"},\"id\":\"u-5\"}");
    assert!(line.contains("\"id\":\"u-5\",\"ok\":false,\"refusal\":{\"kind\":\"unavailable\""), "{line}");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("orders_failed=1") && done.stdout.contains("ledger_failed=1"), "each violation reached its owner's on_failure once:\n{}", done.stdout);
}

#[test]
fn stopping_one_exposure_leaves_the_others_serving() {
    let server = start(64, &[]);
    assert_eq!(place(server.port, "t-alice", "ACME").status, 200);
    server.stop_one("public");
    let start = Instant::now();
    while !closed(server.port) {
        assert!(start.elapsed() < Duration::from_secs(10), "public's listener closed after its stop()");
        std::thread::sleep(Duration::from_millis(10));
    }
    // partner and admin go on, with their own state
    let c = place(server.port2, "t-carol", "ACME");
    assert_eq!((c.status, c.body.as_str()), (200, "{\"order\":9001,\"notional\":125000}"));
    let line = admin(&server, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":41}},\"id\":\"a-1\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"ok\":true"), "admin still serves public's Orders: {line}");
    server.stop_one("partner");
    let start = Instant::now();
    while !closed(server.port2) {
        assert!(start.elapsed() < Duration::from_secs(10), "partner's listener closed after its stop()");
        std::thread::sleep(Duration::from_millis(10));
    }
    let line = admin(&server, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"book\":\"main\"},\"id\":\"a-2\"}");
    assert!(line.contains("\"ok\":true"), "admin goes on after both HTTP exposures stopped: {line}");
    server.stop_one("admin");
    let start = Instant::now();
    while server.sock().exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "admin's socket removed after its stop()");
        std::thread::sleep(Duration::from_millis(10));
    }
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    for line in ["public stopped", "partner stopped", "admin stopped"] {
        assert!(done.stdout.contains(line), "{line}:\n{}", done.stdout);
    }
}

#[test]
fn a_connection_that_breaks_while_the_listeners_serve_others() {
    let server = start(64, &[]);
    // cut requests on both HTTP listeners, and a unix client that vanishes
    send(server.port, "POST /call/Orders::place HTTP/1.1\r\nContent-Length: 100\r\n\r\n{\"sym").close();
    send(server.port2, "GET /.desc").close();
    {
        use std::io::Write;
        let mut s = std::os::unix::net::UnixStream::connect(server.sock()).expect("connect");
        s.write_all(b"{\"call\":\"Orders::cancel\",\"payl").expect("write");
    }
    assert_eq!(place(server.port, "t-alice", "ACME").status, 200);
    assert_eq!(place(server.port2, "t-carol", "ACME").status, 200);
    let line = admin(&server, "{\"describe\":true}");
    assert!(line.contains("\"ok\":true"), "{line}");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

#[test]
fn a_client_that_goes_away_after_its_call_started_leaves_the_work_to_finish() {
    // bound 1: the call that was started holds the exposure's only place
    // until the handler returns, though nobody is waiting for it
    let server = start(1, &[("SLOW_MS", "700")]);
    let slow = send(server.port, &request_text("POST", "/call/Orders::place", &[bearer("t-alice"), h("Hale-Surface-Digest", PUBLIC_DIGEST)], Some("{\"symbol\":\"SLOW\",\"qty\":10,\"limit\":12500}")));
    std::thread::sleep(Duration::from_millis(250));
    slow.close();
    std::thread::sleep(Duration::from_millis(100));
    let full = place(server.port, "t-alice", "ACME");
    assert_eq!(full.status, 429, "the lost call still holds its place: {}", full.body);
    // partner is another exposure with its own bound
    assert_eq!(place(server.port2, "t-carol", "ACME").status, 200);
    // once the handler returns, the place is released, once
    let start = Instant::now();
    loop {
        let again = place(server.port, "t-alice", "ACME");
        if again.status == 200 {
            // the lost call ran and took order 41; this one is the next
            assert_eq!(again.body, "{\"order\":42,\"notional\":125000}", "the work ran once");
            break;
        }
        assert_eq!(again.status, 429, "{}", again.body);
        assert!(start.elapsed() < Duration::from_secs(10), "the place was never released");
        std::thread::sleep(Duration::from_millis(50));
    }
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert_eq!(done.stdout.matches("place started").count(), 1, "{}", done.stdout);
    assert_eq!(done.stdout.matches("place finished").count(), 1, "the work ran to completion once:\n{}", done.stdout);
}

#[test]
fn the_desk_program_runs_clean_under_asan() {
    // the same program under AddressSanitizer with the arena's chunk
    // recycling off: a connection that outlives its request, a request
    // whose client went away, a listener released at stop(), a peer locus
    // per connection: a use after free would be reported here
    let bin = harness::unique_bin("api_http_witness_asan");
    harness::build_source_asan(&DESK.replace("@BOUND@", "64"), &bin);
    let operator = me_operator();
    let server = Server::start(&bin, &[("OPERATOR", &operator), ("LOTUS_NO_CHUNK_POOL", "1"), ("SLOW_MS", "300")]);
    server.ready();
    wait_listening(server.port2);
    assert_eq!(place(server.port, "t-alice", "ACME").status, 200);
    assert_eq!(place(server.port2, "t-carol", "ACME").status, 200);
    assert_eq!(request(server.port, "GET", "/.description", &[bearer("t-alice")], None).status, 200);
    assert_eq!(place(server.port, "t-alice", "ACME").status, 200);
    assert_eq!(request(server.port, "POST", "/call/Orders::place", &[bearer("t-alice")], Some("{\"symbol\":\"ACME\",\"qty\":50000,\"limit\":1}")).status, 500);
    send(server.port2, "POST /call/Orders::place HTTP/1.1\r\nContent-Length: 100\r\n\r\n{\"sym").close();
    send(server.port2, &request_text("POST", "/call/Orders::place", &[bearer("t-carol")], Some("{\"symbol\":\"SLOW\",\"qty\":10,\"limit\":12500}"))).close();
    let line = admin(&server, "{\"describe\":true}");
    assert!(line.contains("\"ok\":true"), "{line}");
    let pending = send(server.port2, &request_text("POST", "/call/Orders::place", &[bearer("t-carol")], Some("{\"symbol\":\"SLOW\",\"qty\":10,\"limit\":12500}")));
    std::thread::sleep(Duration::from_millis(100));
    let done = server.finish();
    drop(pending);
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!done.stderr.contains(bad), "{bad} in:\n{}", done.stderr);
    }
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}
