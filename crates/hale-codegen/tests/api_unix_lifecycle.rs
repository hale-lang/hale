//! GH #1417 (R2b): the lifecycle of a request over real sockets.
//!
//! One program (`LIFE`) serves three exposures over `unix::Rpc`: `public`
//! and `partner` serve one surface through two instances of one receiver
//! type, each on a pool of its own, and `ops` serves receivers that hold
//! a call, violate, restart and are replaced. Each test starts the program
//! afresh, drives it from a client's side of the sockets, and asserts what
//! a caller or an operator can observe: the reply lines, the state the
//! handlers kept (asked of the program through its own rpc members and
//! printed by it at exit), and whether the process comes to an end.
//!
//! The rows are the handoff's lifecycle table (`spec/api.md` § The request
//! lifecycle, § Receiver failure and generations); `tests/hale/api/` holds
//! the same rows over the in-process transport, and this file is what the
//! socket adds: framing, correlation to a connection, a connection that
//! goes away, and a listener released at the end.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/unix_rpc.rs"]
mod unix_rpc;

use unix_rpc::*;

pub const LIFE: &str = r#"
role operator;

type PlaceOrder { symbol: String; qty: Int; }
type Receipt { order: Int; }
type CancelOrder { order: Int; }
type Cancelled { order: Int; was_open: Bool; }
type OrderError { code: String; reason: String; }
type Ping { n: Int; }
type Pong { n: Int; }
type OrdersStats { entered: Int; next: Int; }
type Req { n: Int; }
type Res { n: Int; }
type LedgerStats { started: Int; finished: Int; }
type Hold { ms: Int; }
type Held { ms: Int; }
type SvcStats { births: Int; ran_slow: Int; ran_fast: Int; }

@unbounded
fn pause(ms: Int) {
    let mut i = 0;
    while i < ms {
        std::time::sleep(1ms);
        i = i + 1;
    }
}

api Public {
    rpc Orders::place;
    rpc Orders::cancel requires: [operator];
    rpc Orders::stats;
}

api Ops {
    rpc Ledger::slow;
    rpc Ledger::boom;
    rpc Ledger::stats;
    rpc Audit::ping;
    rpc Audit::hold;
    rpc Audit::boom;
    rpc Restarter::arm;
    rpc Restarter::slow;
    rpc Restarter::fast;
    rpc Restarter::stats;
    rpc Poisoned::slow;
    rpc Poisoned::fast;
    rpc Poisoned::poison;
    rpc Poisoned::stats;
}

locus Grants {
    params { operator: String = ""; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "operator" { return len(self.operator) > 0 && p.name == self.operator; }
        return false;
    }
}

locus Orders {
    params { next: Int = 41; entered: Int = 0; open: Int = 0; }
    closure position_limit { captures: open; epoch inline; }
    fn place(o: PlaceOrder) -> Receipt fallible(ClosureViolation) {
        self.entered = self.entered + 1;
        if o.qty == 20000 { pause(200); }
        if o.qty > 10000 { violate position_limit; }
        let id = self.next;
        self.next = id + 1;
        self.open = self.open + 1;
        return Receipt { order: id };
    }
    fn cancel(c: CancelOrder) -> Cancelled fallible(OrderError) {
        self.entered = self.entered + 1;
        if c.order < 41 || c.order >= self.next {
            fail OrderError { code: "unknown_order", reason: "no order " + to_string(c.order) };
        }
        return Cancelled { order: c.order, was_open: true };
    }
    fn stats(p: Ping) -> OrdersStats {
        return OrdersStats { entered: self.entered, next: self.next };
    }
}

locus Ledger {
    params { started: Int = 0; finished: Int = 0; }
    closure books { captures: started; epoch inline; }
    fn slow(r: Req) -> Res {
        self.started = self.started + 1;
        pause(r.n);
        self.finished = self.finished + 1;
        return Res { n: r.n };
    }
    fn boom(r: Req) -> Res fallible(ClosureViolation) {
        self.started = self.started + 1;
        pause(r.n);
        violate books;
    }
    fn stats(p: Ping) -> LedgerStats {
        return LedgerStats { started: self.started, finished: self.finished };
    }
}

// on the exposure's own pool
locus Audit {
    params { pings: Int = 0; }
    closure audit_ok { captures: pings; epoch inline; }
    fn ping(p: Ping) -> Pong {
        self.pings = self.pings + 1;
        return Pong { n: p.n + 1 };
    }
    fn hold(h: Hold) -> Held {
        pause(h.ms);
        return Held { ms: h.ms };
    }
    fn boom(p: Ping) -> Pong fallible(ClosureViolation) {
        violate audit_ok;
    }
}

locus Restarter {
    params { births: Int = 0; runs: Int = 0; armed: Bool = false; ran_slow: Int = 0; ran_fast: Int = 0; }
    closure fuse { captures: runs; epoch inline; }
    birth() { self.births = self.births + 1; }
    run() {
        self.runs = self.runs + 1;
        if self.runs == 1 {
            let mut i = 0;
            while !self.armed && i < 20000 { std::time::sleep(1ms); i = i + 1; }
            violate fuse;
        }
    }
    fn arm(r: Req, ctx: std::api::ServedContext) -> Res {
        self.armed = true;
        pause(60);
        return Res { n: ctx.generation };
    }
    fn slow(r: Req) -> Res {
        self.ran_slow = self.ran_slow + 1;
        return Res { n: r.n };
    }
    fn fast(r: Req, ctx: std::api::ServedContext) -> Res {
        self.ran_fast = self.ran_fast + 1;
        return Res { n: ctx.generation };
    }
    fn stats(p: Ping) -> SvcStats {
        return SvcStats { births: self.births, ran_slow: self.ran_slow, ran_fast: self.ran_fast };
    }
}

locus Poisoned {
    params { births: Int = 0; ran_slow: Int = 0; ran_fast: Int = 0; }
    closure bad { captures: ran_fast; epoch inline; }
    birth() { self.births = self.births + 1; }
    fn slow(r: Req) -> Res {
        self.ran_slow = self.ran_slow + 1;
        return Res { n: r.n };
    }
    fn fast(r: Req, ctx: std::api::ServedContext) -> Res {
        self.ran_fast = self.ran_fast + 1;
        return Res { n: ctx.generation };
    }
    fn poison(r: Req) -> Res fallible(ClosureViolation) {
        pause(100);
        violate bad;
    }
    fn stats(p: Ping) -> SvcStats {
        return SvcStats { births: self.births, ran_slow: self.ran_slow, ran_fast: self.ran_fast };
    }
}

main locus Desk {
    params {
        public_roles: Grants = Grants { operator: std::env::var("OPERATOR") };
        partner_roles: Grants = Grants { operator: std::env::var("PARTNER_OPERATOR") };
        orders: Orders = Orders { };
        partner_orders: Orders = Orders { next: 9001 };
        ledger: Ledger = Ledger { };
        audit: Audit = Audit { };
        svc: Restarter = Restarter { };
        poisoned: Poisoned = Poisoned { };
        ops_rpc: std::api::unix::Rpc = std::api::unix::Rpc { path: std::env::var("SOCK3") };
        orders_failed: Int = 0;
        ledger_failed: Int = 0;
        audit_failed: Int = 0;
        svc_failures: Int = 0;
        poisoned_failures: Int = 0;
    }
    placement {
        orders: cooperative(pool = desk) where async_io;
        partner_orders: cooperative(pool = partner) where async_io;
        ledger: cooperative(pool = desk) where async_io;
        svc: cooperative(pool = work) where async_io;
        poisoned: cooperative(pool = work2) where async_io;
    }
    on_failure(o: Orders, err: ClosureViolation) { self.orders_failed = self.orders_failed + 1; }
    on_failure(l: Ledger, err: ClosureViolation) { self.ledger_failed = self.ledger_failed + 1; }
    on_failure(a: Audit, err: ClosureViolation) { self.audit_failed = self.audit_failed + 1; }
    on_failure(r: Restarter, err: ClosureViolation) {
        self.svc_failures = self.svc_failures + 1;
        restart (r) for 3;
    }
    on_failure(p: Poisoned, err: ClosureViolation) {
        self.poisoned_failures = self.poisoned_failures + 1;
        self.poisoned = Poisoned { };
    }

    run() {
        let public = api::serve(Public, unix::Rpc { path: std::env::var("SOCK"), roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 4, on_full: refuse);
        let partner = api::serve(Public, unix::Rpc { path: std::env::var("SOCK2"), roles: self.partner_roles }, as: "partner", receivers: { Orders: self.partner_orders }, bound: 4, on_full: refuse);
        let ops = api::serve(Ops, self.ops_rpc, as: "ops", bound: 3, on_full: refuse);
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {
            std::time::sleep(10ms);
        }
        if std::env::var("MODE") != "scope_exit" {
            ops.stop();
            ops.stop();
            public.stop();
            partner.stop();
            println("stops=", self.ops_rpc.stops);
        }
        println("orders_failed=", self.orders_failed, " ledger_failed=", self.ledger_failed, " audit_failed=", self.audit_failed, " svc_failures=", self.svc_failures, " poisoned_failures=", self.poisoned_failures);
        println("ledger_started=", self.ledger.started, " ledger_finished=", self.ledger.finished, " closes=", self.ops_rpc.closed);
        println("run returned");
    }
}

fn main() { Desk { }; }
"#;

fn build(asan: bool) -> PathBuf {
    static PLAIN: OnceLock<PathBuf> = OnceLock::new();
    static SANITIZED: OnceLock<PathBuf> = OnceLock::new();
    let cell = if asan { &SANITIZED } else { &PLAIN };
    cell.get_or_init(|| {
        let bin = harness::unique_bin(if asan { "api_unix_life_asan" } else { "api_unix_life" });
        if asan {
            harness::build_source_asan(LIFE, &bin);
        } else {
            build_opts::build_source(LIFE, &bin, &build_opts::options()).expect("build the lifecycle program");
        }
        bin
    })
    .clone()
}

fn start(mode: &str) -> Server {
    let (uid, _, _) = me();
    Server::start(
        &build(false),
        &[("OPERATOR", &format!("uid:{uid}")), ("PARTNER_OPERATOR", &format!("uid:{}", uid + 1)), ("MODE", mode)],
    )
}

fn call(member: &str, payload: &str, id: &str) -> String {
    format!("{{\"call\":\"{member}\",\"payload\":{payload},\"id\":\"{id}\"}}")
}

/// An integer member of a reply line.
fn int(line: &str, key: &str) -> i64 {
    let at = line.find(&format!("\"{key}\":")).unwrap_or_else(|| panic!("no `{key}` in {line}")) + key.len() + 3;
    line[at..].chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect::<String>().parse().unwrap()
}

fn is_result(line: &str) -> bool {
    line.contains("\"ok\":true,\"value\":")
}

fn refusal_kind(line: &str) -> String {
    let at = line.find("\"refusal\":{\"kind\":\"").unwrap_or_else(|| panic!("not a refusal: {line}")) + "\"refusal\":{\"kind\":\"".len();
    line[at..].chars().take_while(|c| *c != '"').collect()
}

fn id_of(line: &str) -> String {
    let at = line.find("\"id\":\"").unwrap_or_else(|| panic!("no id in {line}")) + 6;
    line[at..].chars().take_while(|c| *c != '"').collect()
}

fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("never: {what}");
}

fn ledger_stats(c: &mut Conn) -> (i64, i64) {
    let r = c.ask(&call("Ledger::stats", "{\"n\":0}", "ls"));
    assert!(is_result(&r), "{r}");
    (int(&r, "started"), int(&r, "finished"))
}

/// `n` calls that hold the audit receiver for `ms` each, back to back; the
/// replies' ids, in the order they arrive.
fn holds(c: &mut Conn, n: usize, ms: i64) {
    for i in 0..n {
        c.send(&call("Audit::hold", &format!("{{\"ms\":{ms}}}"), &format!("h{i}")));
    }
}

/// How many of the next `n` replies are results and how many are `full`,
/// and that nothing else came.
fn tally(c: &mut Conn, n: usize) -> (usize, usize) {
    let (mut ok, mut full) = (0, 0);
    for _ in 0..n {
        let r = c.line();
        if is_result(&r) {
            ok += 1;
        } else {
            assert_eq!(refusal_kind(&r), "full", "{r}");
            full += 1;
        }
    }
    (ok, full)
}

#[test]
fn two_instances_of_one_receiver_type_reach_their_own_exposures_and_pools() {
    let server = start("stop");
    let mut public = server.connect();
    let mut partner = server.connect2();
    let place = |c: &mut Conn, id: &str| c.ask(&call("Orders::place", "{\"symbol\":\"ACME\",\"qty\":10}", id));

    let a = place(&mut public, "p-1");
    assert!(is_result(&a) && a.contains("\"value\":{\"order\":41}"), "public reached `orders`: {a}");
    let b = place(&mut partner, "q-1");
    assert!(is_result(&b) && b.contains("\"value\":{\"order\":9001}"), "partner reached `partner_orders`: {b}");
    let c = place(&mut public, "p-2");
    assert!(c.contains("\"value\":{\"order\":42}"), "and `orders` kept its own count: {c}");

    let sa = public.ask(&call("Orders::stats", "{\"n\":0}", "s"));
    let sb = partner.ask(&call("Orders::stats", "{\"n\":0}", "s"));
    assert_eq!((int(&sa, "entered"), int(&sa, "next")), (2, 43), "public's receiver: {sa}");
    assert_eq!((int(&sb, "entered"), int(&sb, "next")), (1, 9002), "partner's receiver: {sb}");
    assert!(server.finish().status.success());
}

#[test]
fn refusals_leave_handler_state_untouched_and_run_in_the_specs_order() {
    let server = start("stop");
    let mut public = server.connect();
    let mut partner = server.connect2();
    let mut ops = server.connect3();
    let digest = {
        let d = public.ask("{\"describe\":true}");
        let at = d.find("\"digest\":\"").expect("a description names its digest") + 10;
        d[at..].chars().take_while(|c| *c != '"').collect::<String>()
    };
    let wrong = "fnv1a64:0000000000000000";

    let before = public.ask(&call("Orders::stats", "{\"n\":0}", "b"));
    assert_eq!((int(&before, "entered"), int(&before, "next")), (0, 41));

    // 1. the line is a request
    let r = public.ask("{\"nothing\":true,\"id\":\"a\"}");
    assert_eq!(refusal_kind(&r), "malformed", "{r}");
    // 3 before 4: the digest is looked at before the member is looked up
    let r = public.ask(&format!("{{\"call\":\"Orders::nope\",\"payload\":{{}},\"id\":\"b\",\"digest\":\"{wrong}\"}}"));
    assert_eq!(refusal_kind(&r), "digest_mismatch", "{r}");
    assert!(r.contains(&format!("\"served\":\"{digest}\"")), "the refusal names the served digest: {r}");
    // 4. no such member
    let r = public.ask(&call("Orders::nope", "{}", "c"));
    assert_eq!(refusal_kind(&r), "malformed", "{r}");
    assert!(r.contains("unknown_member"), "{r}");
    // 3 before 5: the digest before the role (partner does not hold `operator`)
    let r = partner.ask(&format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":9001}},\"id\":\"d\",\"digest\":\"{wrong}\"}}"));
    assert_eq!(refusal_kind(&r), "digest_mismatch", "{r}");
    // 5 before 6: the role before the payload is decoded
    let r = partner.ask("{\"call\":\"Orders::cancel\",\"payload\":7,\"id\":\"e\"}");
    assert_eq!(refusal_kind(&r), "unauthorized", "{r}");
    assert!(r.contains("\"requires\":[\"operator\"]"), "{r}");
    // 6. the payload, once the role is held (public's operator is this uid)
    let r = public.ask("{\"call\":\"Orders::cancel\",\"payload\":{\"order\":\"x\"},\"id\":\"f\"}");
    assert_eq!(refusal_kind(&r), "malformed", "{r}");
    assert!(r.contains("wrong_type: order"), "{r}");
    let r = public.ask("{\"call\":\"Orders::place\",\"payload\":{\"symbol\":\"ACME\"},\"id\":\"g\"}");
    assert!(r.contains("missing_field: qty"), "{r}");
    // 7. the bound: three holds fill `ops`, the fourth is refused `full`
    holds(&mut ops, 3, 300);
    ops.send(&call("Ledger::slow", "{\"n\":1}", "w"));
    let r = ops.line();
    assert_eq!((refusal_kind(&r), id_of(&r)), ("full".to_string(), "w".to_string()), "a full exposure refuses at once: {r}");
    assert_eq!(tally(&mut ops, 3), (3, 0), "the three accepted holds are answered");

    // no refusal reached a handler
    let after = public.ask(&call("Orders::stats", "{\"n\":0}", "a"));
    assert_eq!((int(&after, "entered"), int(&after, "next")), (0, 41), "orders untouched: {after}");
    let pa = partner.ask(&call("Orders::stats", "{\"n\":0}", "a"));
    assert_eq!(int(&pa, "entered"), 0, "partner_orders untouched: {pa}");
    assert_eq!(ledger_stats(&mut ops), (0, 0), "the refused call never reached the ledger");
    assert!(server.finish().status.success());
}

#[test]
fn the_outcomes_are_distinguishable_on_the_wire() {
    let server = start("stop");
    let mut public = server.connect();
    let mut partner = server.connect2();

    // result
    let r = public.ask(&call("Orders::place", "{\"symbol\":\"ACME\",\"qty\":10}", "r"));
    assert!(r.contains("\"ok\":true,\"value\":{") && !r.contains("\"refusal\"") && !r.contains("\"error\""), "{r}");
    // handler error: the declared error type, by the codec
    let e = public.ask(&call("Orders::cancel", "{\"order\":999}", "e"));
    assert!(e.contains("\"ok\":false,\"error\":{\"code\":\"unknown_order\",\"reason\":\"no order 999\"}") && !e.contains("\"refusal\""), "{e}");
    // refusal: the request did not run
    let f = partner.ask(&call("Orders::cancel", "{\"order\":41}", "f"));
    assert!(f.contains("\"ok\":false,\"refusal\":{\"kind\":\"unauthorized\"") && !f.contains("\"error\""), "{f}");
    // server error: a violating handler, and nothing of the violation
    let s = public.ask(&call("Orders::place", "{\"symbol\":\"ACME\",\"qty\":99999}", "s"));
    assert!(s.contains("\"ok\":false,\"refusal\":{\"kind\":\"server\"}"), "{s}");
    assert!(!s.contains("position_limit"), "the reply says nothing of the violation: {s}");
    // the four are four: no two share a shape
    let kinds: Vec<&str> = [&r, &e, &f, &s]
        .iter()
        .map(|l| {
            if l.contains("\"ok\":true") {
                "result"
            } else if l.contains("\"error\":") {
                "handler_error"
            } else if l.contains("\"kind\":\"server\"") {
                "server_error"
            } else {
                "refusal"
            }
        })
        .collect();
    assert_eq!(kinds, ["result", "handler_error", "refusal", "server_error"]);
    // (the fifth, a transport failure, is the absence of a reply: the
    // connection ends. `the_executing_call_is_abandoned_at_scope_exit…`.)
    assert!(server.finish().status.success());
}

#[test]
fn a_receiver_violation_with_another_request_queued() {
    let server = start("stop");
    let mut ops = server.connect3();
    // the violating call holds the receiver; the next call to the same
    // receiver waits behind it in the exposure
    ops.send(&call("Ledger::boom", "{\"n\":150}", "v"));
    ops.send(&call("Ledger::stats", "{\"n\":0}", "q"));
    let first = ops.line();
    assert_eq!(id_of(&first), "v", "{first}");
    assert!(first.contains("\"refusal\":{\"kind\":\"server\"}"), "the violating call gets the server error: {first}");
    let second = ops.line();
    assert_eq!((id_of(&second), refusal_kind(&second)), ("q".to_string(), "unavailable".to_string()), "the queued call did not run: {second}");
    assert!(second.contains("the call did not run"), "{second}");

    // the receiver stays unavailable; other receivers of the exposure serve on
    let third = ops.ask(&call("Ledger::slow", "{\"n\":1}", "u"));
    assert_eq!(refusal_kind(&third), "unavailable", "{third}");
    let pong = ops.ask(&call("Audit::ping", "{\"n\":1}", "p"));
    assert!(pong.contains("\"value\":{\"n\":2}"), "{pong}");
    // no deadlock, and the capacity was released once for each of the two:
    // three holds are accepted and a fourth is refused (a unit released
    // twice would accept it; one never released would refuse the third)
    holds(&mut ops, 4, 100);
    assert_eq!(tally(&mut ops, 4), (3, 1), "exactly the bound is accepted");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(done.stdout.contains("ledger_failed=1"), "the owner absorbed the violation once:\n{}", done.stdout);
}

#[test]
fn same_pool_and_cross_pool_execution_never_block_a_scheduler_cycle() {
    let server = start("stop");
    let mut ops = server.connect3();
    let mut other = server.connect3();
    // a call held on the ledger's pool does not stop a receiver on the
    // exposure's own pool from answering
    ops.send(&call("Ledger::slow", "{\"n\":400}", "slow"));
    // (the receiver starts the call at once; asking the ledger itself would wait behind it)
    std::thread::sleep(Duration::from_millis(100));
    let pong = other.ask(&call("Audit::ping", "{\"n\":1}", "ping"));
    assert!(is_result(&pong), "{pong}");
    assert_eq!(ops.recv_within(100), Recv::Timeout, "the held call is still held when the other was answered");
    // a violation on the exposure's own pool: the owner's on_failure runs
    // there too, and the caller gets the server error
    let boom = other.ask(&call("Audit::boom", "{\"n\":1}", "boom"));
    assert!(boom.contains("\"refusal\":{\"kind\":\"server\"}"), "{boom}");
    // the held call completes
    let slow = ops.line();
    assert_eq!(id_of(&slow), "slow");
    assert!(is_result(&slow), "{slow}");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(done.stdout.contains("audit_failed=1"), "{}", done.stdout);
}

#[test]
fn a_client_that_disconnects_after_execution_starts() {
    let server = start("stop");
    let mut gone = server.connect3();
    let mut ops = server.connect3();
    let mut probe = server.connect3();
    gone.send(&call("Ledger::slow", "{\"n\":400}", "lost"));
    std::thread::sleep(Duration::from_millis(100));
    gone.close();
    // the lost call still holds its place in the bound while it executes:
    // two holds and the lost call make three, and the next two are refused
    holds(&mut ops, 4, 500);
    let r = ops.line();
    assert_eq!(refusal_kind(&r), "full", "the lost request holds its place until it completes: {r}");
    let r = ops.line();
    assert_eq!(refusal_kind(&r), "full", "{r}");
    // it completes, once; the work ran once and nobody was written to
    eventually("the lost call completed", || {
        let r = probe.ask(&call("Ledger::stats", "{\"n\":0}", "ls"));
        is_result(&r) && int(&r, "finished") == 1
    });
    // the two holds' results
    assert_eq!(tally(&mut ops, 2), (2, 0));
    assert_eq!(ledger_stats(&mut probe), (1, 1), "the work ran once, to completion");
    // the unit was released once: three holds accepted, a fourth refused
    holds(&mut ops, 4, 80);
    assert_eq!(tally(&mut ops, 4), (3, 1));
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(done.stdout.contains("closes=1"), "the transport was told once that the lost connection is finished with:\n{}", done.stdout);
    assert!(done.stdout.contains("ledger_started=1 ledger_finished=1"), "{}", done.stdout);
}

#[test]
fn stop_with_queued_and_executing_work_then_a_second_stop() {
    let server = start("stop");
    let mut ops = server.connect3();
    ops.send(&call("Ledger::slow", "{\"n\":600}", "run"));
    ops.send(&call("Ledger::slow", "{\"n\":10}", "q1"));
    ops.send(&call("Ledger::slow", "{\"n\":10}", "q2"));
    // (the receiver starts the first at once and holds it for 600 ms)
    std::thread::sleep(Duration::from_millis(200));
    server.trigger();
    // the queued are refused (they did not run) in the order they queued; the
    // executing one is answered; then the connection ends
    let mut got = Vec::new();
    for _ in 0..3 {
        got.push(ops.line());
    }
    let by = |id: &str| got.iter().find(|l| id_of(l) == id).unwrap_or_else(|| panic!("no reply for {id}: {got:?}")).clone();
    assert!(is_result(&by("run")), "the executing call is answered before stop() returns: {}", by("run"));
    for q in ["q1", "q2"] {
        let l = by(q);
        assert_eq!(refusal_kind(&l), "shutting_down", "{l}");
        assert!(l.contains("the call did not run"), "{l}");
    }
    assert!(ops.closed_within(5000), "then the connection is closed");
    let mut server = server;
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(done.stdout.contains("ledger_started=1 ledger_finished=1"), "neither queued call ran, the executing one finished:\n{}", done.stdout);
    assert!(done.stdout.contains("stops=1"), "the second stop() released nothing more:\n{}", done.stdout);
    assert!(!server.has_socket(3) && !server.has_socket(1) && !server.has_socket(2), "every socket is released");
}

#[test]
fn the_executing_call_is_abandoned_at_scope_exit_and_the_sockets_are_released() {
    // `run()` returns without `stop()`: the handles' owner is torn down,
    // which is the same shutdown. A queued call is refused, a call that does
    // not finish within the teardown's wait is abandoned (the caller sees
    // the transport fail, never a refusal), and every socket is released.
    let mut server = start("scope_exit");
    let mut ops = server.connect3();
    ops.send(&call("Ledger::slow", "{\"n\":20000}", "run"));
    ops.send(&call("Ledger::slow", "{\"n\":10}", "queued"));
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();
    server.trigger();
    // A process's teardown stops the pools of the listener and its
    // connections before the exposure's own shutdown runs, so a connection
    // cannot be written to by then: neither call is answered, and each
    // caller sees the transport fail (the connection ends), never a reply
    // that is not true. `stop()` is the orderly path that refuses the queue
    // in words (`stop_with_queued_and_executing_work…`).
    match ops.recv_within(15_000) {
        Recv::Eof => {}
        other => panic!("expected the connection to end without a reply, got {other:?}"),
    }
    let done = server.wait(Duration::from_secs(30));
    assert!(t0.elapsed() < Duration::from_secs(20), "the teardown does not wait for the abandoned call: {:?}", t0.elapsed());
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("run returned"), "{}", done.stdout);
    assert!(done.stdout.contains("ledger_started=1"), "the queued call never ran:\n{}", done.stdout);
    assert!(!server.has_socket(1) && !server.has_socket(2) && !server.has_socket(3), "every socket is released without stop()");
}

#[test]
fn a_restart_and_a_replacement_settle_what_was_queued_and_the_next_call_runs_in_the_next_generation() {
    let server = start("stop");
    let mut ops = server.connect3();

    // ---- the receiver restarts in place ----
    let first = ops.ask(&call("Restarter::fast", "{\"n\":1}", "f1"));
    assert!(first.contains("\"value\":{\"n\":1}"), "the first generation serves: {first}");
    ops.send(&call("Restarter::arm", "{\"n\":2}", "arm"));
    ops.send(&call("Restarter::slow", "{\"n\":3}", "s"));
    ops.send(&call("Restarter::fast", "{\"n\":4}", "f2"));
    let mut got = Vec::new();
    for _ in 0..3 {
        got.push(ops.line());
    }
    let by = |id: &str| got.iter().find(|l| id_of(l) == id).unwrap_or_else(|| panic!("no reply for {id}: {got:?}")).clone();
    assert!(by("arm").contains("\"value\":{\"n\":1}"), "the call executing across the restart completes under the generation that ran it: {}", by("arm"));
    assert_eq!(refusal_kind(&by("s")), "unavailable", "{}", by("s"));
    assert_eq!(refusal_kind(&by("f2")), "unavailable", "{}", by("f2"));
    eventually("the receiver was restarted", || {
        let s = ops.ask(&call("Restarter::stats", "{\"n\":0}", "st"));
        is_result(&s) && int(&s, "births") == 2
    });
    let s = ops.ask(&call("Restarter::stats", "{\"n\":0}", "st"));
    assert_eq!((int(&s, "ran_slow"), int(&s, "ran_fast")), (0, 1), "neither queued call ran, in either generation: {s}");
    let next = ops.ask(&call("Restarter::fast", "{\"n\":5}", "f3"));
    assert!(next.contains("\"value\":{\"n\":2}"), "the next call runs in the second generation: {next}");

    // ---- the receiver is replaced, perhaps at an address it had ----
    let p1 = ops.ask(&call("Poisoned::fast", "{\"n\":1}", "p1"));
    assert!(p1.contains("\"value\":{\"n\":1}"), "{p1}");
    ops.send(&call("Poisoned::poison", "{\"n\":2}", "poison"));
    ops.send(&call("Poisoned::slow", "{\"n\":3}", "ps"));
    ops.send(&call("Poisoned::fast", "{\"n\":4}", "pf"));
    let mut got = Vec::new();
    for _ in 0..3 {
        got.push(ops.line());
    }
    let by = |id: &str| got.iter().find(|l| id_of(l) == id).unwrap_or_else(|| panic!("no reply for {id}: {got:?}")).clone();
    assert!(by("poison").contains("\"refusal\":{\"kind\":\"server\"}"), "{}", by("poison"));
    assert_eq!(refusal_kind(&by("ps")), "unavailable", "{}", by("ps"));
    assert_eq!(refusal_kind(&by("pf")), "unavailable", "{}", by("pf"));
    let mut st = String::new();
    eventually("the replacement answers", || {
        st = ops.ask(&call("Poisoned::stats", "{\"n\":0}", "pst"));
        is_result(&st)
    });
    assert_eq!((int(&st, "births"), int(&st, "ran_slow"), int(&st, "ran_fast")), (1, 0, 0), "the replacement carries nothing across: {st}");
    let p2 = ops.ask(&call("Poisoned::fast", "{\"n\":5}", "p2"));
    assert!(p2.contains("\"value\":{\"n\":2}"), "the replacement serves in the next generation: {p2}");

    // each request was answered once: nothing more arrives for any of them
    assert_eq!(ops.recv_within(300), Recv::Timeout, "no reply was duplicated");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(done.stdout.contains("svc_failures=1") && done.stdout.contains("poisoned_failures=1"), "each owner absorbed its failure once:\n{}", done.stdout);
    // and stop() answered nothing again: the connection simply closed
}

#[test]
fn a_connection_that_breaks_while_the_listener_serves_other_clients() {
    let server = start("stop");
    let mut steady = server.connect();
    let mut breaking = server.connect();
    breaking.send(&call("Orders::place", "{\"symbol\":\"ACME\",\"qty\":20000}", "b"));
    // the breaking client goes away with a violating call executing: its
    // reply has nowhere to go
    breaking.close();
    let r = steady.ask(&call("Orders::stats", "{\"n\":0}", "s"));
    // (the receiver is held by the violating call, so `stats` waits behind
    // it and then finds the receiver unavailable: either way it is answered)
    assert!(r.contains("\"id\":\"s\""), "{r}");
    // other clients are accepted and answered on the other exposure
    let mut partner = server.connect2();
    let p = partner.ask(&call("Orders::place", "{\"symbol\":\"ACME\",\"qty\":10}", "p"));
    assert!(is_result(&p), "{p}");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

#[test]
fn a_description_agrees_with_admission_for_the_same_caller() {
    let server = start("stop");
    let mut public = server.connect();
    let mut partner = server.connect2();
    // public's operator is this uid, partner's another: the same surface,
    // two exposures, two role sources, two descriptions
    let d_public = public.ask("{\"describe\":true,\"id\":\"d\"}");
    let d_partner = partner.ask("{\"describe\":true,\"id\":\"d\"}");
    assert!(d_public.contains("\"exposure\":\"Public@fnv1a64:") && d_public.contains("/public\""), "{d_public}");
    assert!(d_partner.contains("/partner\""), "{d_partner}");
    assert!(d_public.contains("{\"name\":\"Orders::cancel\",\"requires\":[\"operator\"]}"), "the operator may call cancel: {d_public}");
    assert!(d_public.contains("{\"name\":\"Orders::place\",\"requires\":[]}"), "{d_public}");
    assert!(!d_partner.contains("Orders::cancel") && d_partner.contains("Orders::place"), "partner may not: {d_partner}");
    // and admission says the same
    let c1 = public.ask(&call("Orders::cancel", "{\"order\":999}", "c"));
    assert!(c1.contains("\"error\":{\"code\":\"unknown_order\""), "admitted (the handler ran): {c1}");
    let c2 = partner.ask(&call("Orders::cancel", "{\"order\":999}", "c"));
    assert_eq!(refusal_kind(&c2), "unauthorized", "{c2}");
    // a description is a result with the caller's principal
    assert!(d_public.contains("\"ok\":true,\"value\":{\"description\":1") && d_public.contains("\"caller\":{\"mode\":\"unix\""), "{d_public}");
    assert!(server.finish().status.success());
}


#[test]
fn a_request_sent_the_moment_the_socket_appears_is_served_even_on_a_loaded_machine() {
    // The listener binds first, and a client that connects at once has its
    // line read and published: to an exposure that is not subscribed yet
    // unless the listener waits for it. Many programs booting together make
    // the exposure late; every first call is still answered.
    std::thread::scope(|s| {
        for worker in 0..16 {
            s.spawn(move || {
                for round in 0..8 {
                    let server = start("stop");
                    let mut c = server.connect3();
                    c.send(&call("Ledger::slow", "{\"n\":1}", "first"));
                    match c.recv_within(8000) {
                        Recv::Line(l) => assert!(is_result(&l), "worker {worker} round {round}: {l}"),
                        other => panic!("worker {worker} round {round}: the first call got {other:?}"),
                    }
                    assert!(server.finish().status.success());
                }
            });
        }
    });
}

#[test]
fn a_path_that_cannot_be_bound_fails_the_boot() {
    // F.37: a transport that cannot realize its listener is a birth failure
    // of the program, not a program that runs without it
    let (uid, _, _) = me();
    let mut server = Server::start(
        &build(false),
        &[("OPERATOR", &format!("uid:{uid}")), ("PARTNER_OPERATOR", "x"), ("MODE", "stop"), ("SOCK", "/nonexistent-dir-for-hale-tests/s.sock")],
    );
    let done = server.wait(Duration::from_secs(30));
    assert!(!done.status.success(), "the boot is refused: {:?}", done.status);
    assert!(done.stderr.contains("could not listen on /nonexistent-dir-for-hale-tests/s.sock"), "and says why:\n{}", done.stderr);
}

#[test]
fn a_path_held_by_a_live_program_is_not_taken() {
    let first = start("stop");
    let mut held = first.connect();
    let (uid, _, _) = me();
    let first_path = first.sock.to_string_lossy().into_owned();
    let mut second = Server::start(
        &build(false),
        &[("OPERATOR", &format!("uid:{uid}")), ("PARTNER_OPERATOR", "x"), ("MODE", "stop"), ("SOCK", &first_path)],
    );
    let done = second.wait(Duration::from_secs(30));
    assert!(!done.status.success(), "the second program does not boot: {:?}", done.status);
    assert!(done.stderr.contains("could not listen on"), "{}", done.stderr);
    // the first still serves, on the connection it had and on a new one
    let r = held.ask(&call("Orders::stats", "{\"n\":0}", "s"));
    assert!(is_result(&r), "{r}");
    let mut fresh = first.connect();
    assert!(is_result(&fresh.ask(&call("Orders::stats", "{\"n\":0}", "t"))));
    assert!(first.finish().status.success());
}

#[test]
fn an_exposure_given_no_path_listens_nowhere_and_the_program_runs() {
    // a program that finds its path is another process's to hold hands the
    // transport none: that exposure has no socket, the boot is not refused,
    // and the program's other exposures serve
    let (uid, _, _) = me();
    let server = Server::start(&build(false), &[("OPERATOR", &format!("uid:{uid}")), ("PARTNER_OPERATOR", &format!("uid:{}", uid + 1)), ("MODE", "stop"), ("SOCK", "")]);
    let mut other = server.connect2();
    assert!(is_result(&other.ask(&call("Orders::stats", "{\"n\":0}", "s"))), "the other exposure serves");
    assert!(!server.has_socket(1), "and the exposure with no path made no socket");
    let done = server.finish();
    assert!(done.status.success(), "the program ends cleanly: {}", done.stderr);
    assert!(!done.stderr.contains("could not listen"), "{}", done.stderr);
}

#[test]
fn the_lifecycle_program_runs_clean_under_asan() {
    // the same program under AddressSanitizer with the arena's chunk
    // recycling off: a reply that outlives its connection, a request that
    // outlives its connection, a pending record freed once, a listener
    // released: a use after free would be reported here
    let bin = build(true);
    let (uid, _, _) = me();
    let server = Server::start(
        &bin,
        &[("OPERATOR", &format!("uid:{uid}")), ("PARTNER_OPERATOR", &format!("uid:{}", uid + 1)), ("MODE", "stop"), ("LOTUS_NO_CHUNK_POOL", "1")],
    );
    let mut public = server.connect();
    let mut ops = server.connect3();
    let mut other = server.connect3();
    assert!(is_result(&public.ask(&call("Orders::place", "{\"symbol\":\"ACME\",\"qty\":10}", "a"))));
    assert!(public.ask("not json").contains("malformed"));
    assert!(public.closed_within(5000));
    // a violation with a call queued behind it, and a client that leaves with a call executing
    ops.send(&call("Ledger::boom", "{\"n\":50}", "v"));
    ops.send(&call("Ledger::stats", "{\"n\":0}", "q"));
    let _ = (ops.line(), ops.line());
    let mut gone = server.connect3();
    gone.send(&call("Audit::hold", "{\"ms\":150}", "g"));
    gone.close();
    assert!(is_result(&other.ask(&call("Audit::ping", "{\"n\":1}", "p"))));
    let mut cut = server.connect();
    cut.send_raw(b"{\"call\":\"Orders::place\",\"pay");
    cut.close();
    // stop with work queued and executing
    other.send(&call("Audit::hold", "{\"ms\":300}", "h1"));
    other.send(&call("Audit::hold", "{\"ms\":300}", "h2"));
    std::thread::sleep(Duration::from_millis(100));
    let done = server.finish();
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!done.stderr.contains(bad), "{bad} in:\n{}", done.stderr);
    }
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}
