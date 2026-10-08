//! GH #1417 (R2a): a serve site builds. The program is the R0 witness's
//! `Public` surface served over the in-process fixture transport
//! (`std::api::test::Rpc`) instead of `http::Rpc`: the build makes the
//! exposure a param of the serving locus, the rows adapter, the codecs of
//! what the rows carry (an identity and a quantity among them) and the
//! receiver's plumbing, and the binary answers one call of each outcome.
//! (The behaviors the runtime owes, in the language: `tests/hale/api/`.)

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const WITNESS: &str = r#"
role trader;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;

type PlaceOrder { symbol: String; qty: Int; limit: Money; }
type OrderReceipt { order: OrderId; notional: Money; }
type CancelOrder { order: OrderId; }
type Cancelled { order: OrderId; was_open: Bool; }
type OrderError { code: String; reason: String; }

api Public {
    rpc Orders::place;
    rpc Orders::cancel requires: [trader];
}

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-alice" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        if token == "t-bob" { return std::api::Principal { mode: "bearer", name: "bob" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Grants {
    params { trader: String = ""; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "trader" { return len(self.trader) > 0 && p.name == self.trader; }
        return false;
    }
}

locus Orders {
    params { next: Int = 41; open: Int = 0; }
    closure position_limit { captures: open; epoch inline; }

    fn place(o: PlaceOrder) -> OrderReceipt fallible(ClosureViolation) {
        if o.qty > 10000 { violate position_limit; }
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

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        public_roles: Grants = Grants { trader: "alice" };
        orders: Orders = Orders { };
        fixture: std::api::test::Rpc = std::api::test::Rpc { principals: self.bearer, roles: self.public_roles };
    }
    placement { orders: cooperative(pool = op) where async_io; }
    on_failure(o: Orders, err: ClosureViolation) { }
    run() {
        let public = api::serve(Public, self.fixture, as: "public", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);
        let f = self.fixture;
        f.call(1, "Orders::place", "{\"symbol\":\"ACME\",\"qty\":3,\"limit\":125}", "t-alice");
        println("1 ", f.await_outcome(1, 2000));
        f.call(2, "Orders::cancel", "{\"order\":41}", "t-bob");
        println("2 ", f.await_outcome(2, 2000));
        f.call(3, "Orders::cancel", "{\"order\":99}", "t-alice");
        println("3 ", f.await_outcome(3, 2000));
        f.call(4, "Orders::place", "{\"symbol\":\"ACME\",\"qty\":99999,\"limit\":125}", "t-alice");
        println("4 ", f.await_outcome(4, 2000));
        public.stop();
        f.call(5, "Orders::cancel", "{\"order\":41}", "t-alice");
        println("5 ", f.await_outcome(5, 2000));
    }
}

fn main() { Desk { }; }
"#;

#[test]
fn a_serve_site_builds_and_answers_each_outcome() {
    let bin = harness::unique_bin("api_serve_build");
    build_opts::build_source(WITNESS, &bin, &build_opts::options()).expect("the witness over the fixture builds");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "exit {:?}\n{stdout}", out.status);
    for want in [
        "1 result",
        "2 refusal:unauthorized",
        "3 handler_error",
        "4 server_error",
        // the violating handler's receiver is draining; stop() then closes admission
        "5 refusal:shutting_down",
    ] {
        assert!(stdout.contains(want), "no `{want}` in:\n{stdout}");
    }
}
