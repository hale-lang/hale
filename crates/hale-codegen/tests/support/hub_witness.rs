//! The witness's hub half (GH #1417, R5): the program the hub tests build.
//!
//! The types `Fill`, `Money` and `OrderId`, the topic `Fills`, a bearer
//! source (`Tokens`: dave, bob, erin, and `t-soon`, whose credential
//! expires), a role source (`Grants`: dave and erin hold `operator`, and a
//! revision is announced when a grant is revoked) and a hub named `fills`
//! that binds `Fills` (`requires: [operator], bound: 64, on_full: drop_old`),
//! as `tests/api-contract/program.hl` has them. The program is driven by
//! files (`support/ws_hub.rs`): it publishes, revokes and stops when told to,
//! and writes what a test may wait for. Its hub is a `ws::Hub`; the UDP tests
//! build it with `udp::Hub` in its place.

pub const WITNESS: &str = r#"
role operator;
role trader;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;
type Fill { order: OrderId; qty: Int; price: Money; }
topic Fills { payload: Fill; subject: "desk.fills"; }

// Who a bearer token is. `t-soon` is erin, whose credential expires EXPIRY_MS (1500)
// ms after the source is first asked about it.
locus Tokens {
    params { issued: Int = 0; }
    fn principal(token: String) -> std::api::Principal {
        if token == "t-dave" { return std::api::Principal { mode: "bearer", name: "dave" }; }
        if token == "t-bob" { return std::api::Principal { mode: "bearer", name: "bob" }; }
        if token == "t-erin" || token == "t-soon" { return std::api::Principal { mode: "bearer", name: "erin" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
    fn expiry(token: String) -> Int {
        if token == "t-soon" {
            if self.issued == 0 { self.issued = std::time::nanos(std::time::current()); }
            let ms = std::str::parse_int(std::env::var("EXPIRY_MS")) or 1500;
            return self.issued + ms * 1000000;
        }
        return 0;
    }
}

// Two operators; a revoked grant announces the new revision.
locus Grants {
    params {
        operator: String = "";
        also: String = "";
        revision: Int = 1;
    }
    bus { publish "__api.roles.revision" of type std::api::Revision; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "operator" { return (len(self.operator) > 0 && p.name == self.operator) || (len(self.also) > 0 && p.name == self.also); }
        return false;
    }
    fn grants(p: std::api::Principal) -> std::api::Grants {
        let mut roles = "";
        if self.holds(p, "operator") { roles = "operator"; }
        return std::api::Grants { roles: roles, revision: self.revision };
    }
    fn revoke(who: String) {
        if self.operator == who { self.operator = ""; }
        if self.also == who { self.also = ""; }
        self.revision = self.revision + 1;
        "__api.roles.revision" <- std::api::Revision { source: "hub_roles", revision: self.revision };
    }
}

locus Maker {
    params { made: Int = 0; }
    bus { publish Fills; }
    fn make(n: Int) {
        self.made = self.made + 1;
        Fills <- Fill { order: OrderId(n), qty: n, price: n * 10cent };
    }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        hub_roles: Grants = Grants { operator: "dave", also: "erin" };
        hub: ws::Hub = ws::Hub { bind: std::env::var("BIND"), principals: self.bearer, roles: self.hub_roles, as: "fills" };
        maker: Maker = Maker { };
    }
    bindings {
        Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;
    }
    fn stats(ctl: String) {
        std::io::fs::write_file(ctl + "/stats.tmp", "made=" + to_string(self.maker.made) + "\nevents=" + to_string(self.hub.events()) + "\nsubs=" + to_string(self.hub.subscribers()) + "\nconns=" + to_string(self.hub.connections()) + "\nrevision=" + to_string(self.hub_roles.revision) + "\n") or discard;
        std::io::fs::rename(ctl + "/stats.tmp", ctl + "/stats") or discard;
    }
    // one command is `;`-separated steps: `fill N`, `revoke WHO`, `announce`, `stop`
    fn step(s: String) -> Bool {
        let c = std::str::trim(s);
        if c == "stop" { return true; }
        if std::str::starts_with(c, "fill ") {
            let n = std::str::parse_int(c[5..len(c)]) or 0;
            let mut i = 1;
            while i <= n {
                self.maker.make(i);
                i = i + 1;
            }
        }
        if std::str::starts_with(c, "revoke ") {
            self.hub_roles.revoke(c[7..len(c)]);
        }
        if c == "announce" {
            self.hub.announce(self.hub_roles.revision);
        }
        return false;
    }
    run() {
        let ctl = std::env::var("CTL");
        let mut done = false;
        while !done && !self.draining {
            let cmd = std::str::trim(std::io::fs::read_file(ctl + "/cmd") or "");
            if len(cmd) > 0 {
                std::io::fs::unlink(ctl + "/cmd") or discard;
                let mut rest = cmd;
                while len(rest) > 0 && !done {
                    let semi = std::str::index_of(rest, ";");
                    let mut one = rest;
                    if semi >= 0 {
                        one = rest[0..semi];
                        rest = rest[(semi + 1)..len(rest)];
                    } else {
                        rest = "";
                    }
                    if self.step(one) { done = true; }
                }
            }
            self.stats(ctl);
            std::time::sleep(5ms);
        }
        println("events=", self.hub.events(), " made=", self.maker.made);
        if std::env::var("MODE") != "scope_exit" {
            self.hub.stop();
        }
    }
}

fn main() { Desk { }; }
"#;
