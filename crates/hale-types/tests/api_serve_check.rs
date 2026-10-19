//! GH #1417 (R2a), GH #1565: the laws of a serve site, an exposure param
//! (spec/api.md § Serving), one refusal per law in the contract's wording,
//! the retired body form refused, where a listener may be placed, a site
//! the check admits,
//! and the R0 witness (`tests/api-contract/program.hl`) still accepted:
//! the check reads its serve sites over transports this compiler does
//! not ship (`http::Rpc`, `unix::Rpc`) as it read them in R1, and a
//! build refuses those (`api_serve_build.rs`).

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;

/// Declarations every case shares: a role, the rows' types, two receiver
/// loci and two surfaces, and a serving locus that holds `params`, the
/// fixture transport and then the exposure params `exposures`, and whose
/// `run()` is `body`.
fn program(params: &str, exposures: &str, body: &str) -> String {
    format!(
        "
role trader;

type Req {{ n: Int; }}
type Res {{ n: Int; }}

locus Orders {{
    params {{ count: Int = 0; }}
    fn place(r: Req) -> Res {{ self.count = self.count + 1; return Res {{ n: r.n }}; }}
    run() {{ }}
}}

locus Ledger {{
    fn rebalance(r: Req) -> Res {{ return Res {{ n: r.n }}; }}
    run() {{ }}
}}

api Public {{ rpc Orders::place; }}
api Both {{ rpc Orders::place; rpc Ledger::rebalance requires: [trader]; }}

main locus Desk {{
    params {{
        {params}
        fixture: std::api::test::Rpc = std::api::test::Rpc {{ }};
        {exposures}
    }}
    run() {{
        {body}
    }}
}}

fn main() {{ Desk {{ }}; }}
"
    )
}

const ONE: &str = "orders: Orders = Orders { };";
const TWO: &str = "orders: Orders = Orders { }; partner_orders: Orders = Orders { };";
const LEDGER: &str = "orders: Orders = Orders { }; ledger: Ledger = Ledger { };";

/// The serve-site errors a program draws: every error that is not another
/// law's (`rpc …`), in the order reported.
fn errors(src: &str) -> Vec<String> {
    let parsed = hale_syntax::parse_source(src).unwrap_or_else(|d| panic!("parse: {:?}", d[0].message));
    let snap = Snapshot::from_program(parsed, Vec::new(), Config::check(false, false)).ok().expect("a snapshot");
    let diags = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

const SERVE: &str = "api::serve(Public, self.fixture, as: \"public\", receivers: { Orders: self.orders }, bound: 8, on_full: refuse)";

#[test]
fn a_well_formed_serve_site_is_admitted() {
    assert!(errors(&program(ONE, &format!("public: api::Exposure = {SERVE};"), "self.public.stop();")).is_empty());
    // inferred: the serving locus holds exactly one instance of the type
    assert!(errors(&program(
        ONE,
        "public: api::Exposure = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse);",
        "self.public.stop();"
    ))
    .is_empty());
    // a surface served twice, to two instances, under two names
    assert!(errors(&program(
        TWO,
        "a: api::Exposure = api::serve(Public, self.fixture, as: \"a\", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);
         b: api::Exposure = api::serve(Public, self.fixture, as: \"b\", receivers: { Orders: self.partner_orders }, bound: 8, on_full: refuse);",
        "self.a.stop(); self.b.stop();"
    ))
    .is_empty());
}

/// GH #1565: the serve site in a body is the retired form, refused with the
/// param to write.
#[test]
fn a_serve_site_in_a_body_is_refused_with_the_param_to_write() {
    assert_eq!(
        errors(&program(ONE, "", &format!("let public = {SERVE}; public.stop();"))),
        vec![
            "`api::serve` in a body of `Desk`: an exposure is a param of the locus that holds it, built with it and \
             placed by `placement { }`; declare it in `params { }`, `public: api::Exposure = api::serve(Public, …);`, \
             and use `self.public` where the body used the handle (`self.public.wait()`, `self.public.stop()`) \
             (spec/api.md § Serving)"
        ]
    );
}

#[test]
fn an_exposure_param_is_typed_as_one() {
    assert_eq!(
        errors(&program(ONE, &format!("public: Int = {SERVE};"), "")),
        vec!["exposure param `public`: `api::serve(…)` builds an `api::Exposure`; declare `public: api::Exposure`"]
    );
}

#[test]
fn law_1_an_exposure_is_named_once() {
    let src = program(
        ONE,
        "a: api::Exposure = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse);
         b: api::Exposure = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse);",
        "",
    );
    assert_eq!(errors(&src), vec!["exposure `public` is served twice: `as:` names one exposure; name this one apart"]);
}

#[test]
fn law_2_every_receiver_type_is_bound() {
    // two instances, neither bound
    assert_eq!(
        errors(&program(
            TWO,
            "p: api::Exposure = api::serve(Public, self.fixture, as: \"partner\", bound: 8, on_full: refuse);",
            ""
        )),
        vec![
            "serve of `Public` as `partner`: `Orders` is held twice, as `self.orders` and `self.partner_orders`, and \
             the serve binds neither: name the one that answers (`receivers: { Orders: self.orders }`)"
        ]
    );
    // a type the serving locus does not hold at all
    assert_eq!(
        errors(&program(
            ONE,
            "b: api::Exposure = api::serve(Both, self.fixture, as: \"both\", bound: 8, on_full: refuse);",
            ""
        )),
        vec![
            "serve of `Both` as `both`: `Ledger` is held by no param of `Desk`, and the serve binds none: bind the \
             instance that answers (`receivers: { Ledger: self.<param> }`)"
        ]
    );
}

#[test]
fn law_3_a_receiver_outlives_its_exposure() {
    // a receiver is born before the exposure that binds it: declared after
    // it, it would be born after and drained before
    let src = program("", &format!("public: api::Exposure = {SERVE};"), "").replace(
        "on_full: refuse);\n    }",
        "on_full: refuse);\n        orders: Orders = Orders { };\n    }",
    );
    assert_eq!(
        errors(&src),
        vec![
            "serve of `Public` as `public`: `orders` is declared after `public`, the exposure that binds it; a \
             receiver is born before the exposure that binds it, so declare `orders` first"
        ]
    );
    // a receiver is a param of the holder
    assert_eq!(
        errors(&program(
            ONE,
            "admin: api::Exposure = api::serve(Both, self.fixture, as: \"admin\", receivers: { Ledger: self.ledger }, bound: 8, on_full: refuse);",
            ""
        )),
        vec!["serve of `Both`: `self.ledger` is no param of `Desk`; hold the receiver as a param"]
    );
    // a param the serving locus holds, but built by something other than a
    // literal, cannot carry the number the serve gives its instance
    assert_eq!(
        errors(&program(
            "orders: Orders = make_orders();",
            "public: api::Exposure = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse);",
            ""
        )
        .replace("fn main()", "fn make_orders() -> Orders { return Orders { }; }\nfn main()")),
        vec![
            "serve of `Public`: `orders` is not built by a literal in a param of `Desk`: the serve numbers the \
             instance in the literal that builds it"
        ]
    );
}

#[test]
fn law_4_a_bound_type_is_one_the_rows_name() {
    assert_eq!(
        errors(&program(
            LEDGER,
            "public: api::Exposure = api::serve(Public, self.fixture, as: \"public\", receivers: { Orders: self.orders, Ledger: self.ledger }, bound: 8, on_full: refuse);",
            ""
        )),
        vec!["serve of `Public`: `receivers:` binds `Ledger`, which no row of `Public` names"]
    );
}

#[test]
fn law_5_a_serve_site_states_its_queue() {
    let want = "serve of `Public` as `public`: a serve site states `bound:`, the requests it holds accepted and not yet \
                answered, and `on_full: refuse`, the one policy for a request";
    for site in [
        "api::serve(Public, self.fixture, as: \"public\")",
        "api::serve(Public, self.fixture, as: \"public\", bound: 8)",
        "api::serve(Public, self.fixture, as: \"public\", on_full: refuse)",
        "api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: drop_old)",
    ] {
        assert_eq!(errors(&program(ONE, &format!("public: api::Exposure = {site};"), "")), vec![want], "{site}");
    }
}

#[test]
fn a_site_needs_a_declared_surface_and_a_name() {
    assert_eq!(
        errors(&program(ONE, "p: api::Exposure = api::serve(Pubic, self.fixture, as: \"p\", bound: 8, on_full: refuse);", "")),
        vec!["serve of `Pubic`: no surface `Pubic` is declared; did you mean `Public`?"]
    );
    assert_eq!(
        errors(&program(ONE, "p: api::Exposure = api::serve(Public, self.fixture, bound: 8, on_full: refuse);", "")),
        vec!["serve of `Public`: a serve site names its exposure with `as:`"]
    );
    let free = program(ONE, "", "").replace(
        "fn main()",
        "fn serve_it() { let p = api::serve(Public, std::api::test::Rpc { }, as: \"p\", bound: 8, on_full: refuse); }\nfn main()",
    );
    assert_eq!(
        errors(&free),
        vec![
            "`api::serve` in `serve_it`: an exposure is a param of the locus that holds it and its receivers; declare \
             it in that locus's params, `p: api::Exposure = api::serve(Public, …);` (spec/api.md § Serving)"
        ]
    );
}

// ---- where an exposure's listener runs (GH #1565) ----

const UNIX: &str = "web: api::Exposure = api::serve(Public, unix::Rpc { path: \"/tmp/hale-1565-check.sock\" }, as: \"web\", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);";

/// A row on an exposure the main locus holds places its listener, which
/// parks: only a named pool `where async_io` hosts it; an exposure with no
/// listener of its own has nothing to place.
#[test]
fn an_exposures_row_places_its_listener_on_an_async_io_pool() {
    let placed = |row: &str, exposure: &str| {
        program(ONE, exposure, "")
            .replace("\n    run() {\n        \n    }\n}\n\nfn main", &format!("\n    placement {{ {row} }}\n    run() {{ }}\n}}\n\nfn main"))
    };
    assert_eq!(errors(&placed("web: cooperative(pool = io) where async_io;", UNIX)), Vec::<String>::new());
    let want = "placement entry `web`: an exposure's row places its listener, whose accept and reads park a \
                coroutine, so it runs on a named pool `where async_io`; write `web: cooperative(pool = io) where \
                async_io` (spec/api.md § Serving)";
    assert_eq!(errors(&placed("web: pinned;", UNIX)), vec![want]);
    assert_eq!(errors(&placed("web: cooperative(pool = io);", UNIX)), vec![want]);
    assert_eq!(
        errors(&placed("public: cooperative(pool = io) where async_io;", &format!("public: api::Exposure = {SERVE};"))),
        vec![
            "placement entry `public`: exposure `public` over `std::api::test::Rpc` has no listener of its own to \
             place, and the exposure runs with `Desk`, beside the sources it calls; drop the row"
        ]
    );
}

/// A server locus below the main locus serves; its listener is the main
/// locus's, so the main locus holds the server once, through its params, and
/// can read the address the server's listener binds.
fn server(server_params: &str, main_params: &str) -> String {
    format!(
        "
type Req {{ n: Int; }}
type Res {{ n: Int; }}
locus Orders {{ fn place(r: Req) -> Res {{ return Res {{ n: r.n }}; }} }}
api Public {{ rpc Orders::place; }}
locus Server {{
    params {{
        port: Int = 8080;
        orders: Orders = Orders {{ }};
        {server_params}
    }}
}}
main locus App {{
    params {{
        {main_params}
    }}
    run() {{ }}
}}
fn main() {{ App {{ }}; }}
"
    )
}

const SERVED: &str = "web: api::Exposure = api::serve(Public, unix::Rpc { path: \"/tmp/hale-1565-\" + to_string(self.port) + \".sock\" }, as: \"web\", bound: 8, on_full: refuse);";

#[test]
fn a_server_locus_is_held_once_by_the_placed_tree() {
    assert_eq!(errors(&server(SERVED, "server: Server = Server { };")), Vec::<String>::new());
    assert_eq!(
        errors(&server(SERVED, "count: Int = 0;")),
        vec![
            "exposure `web` is held by `Server`, which no main locus holds through its params: a socket's listener is \
             the main locus's, placed where it places the param that holds the server; hold `Server` as a param of \
             the main locus, or of a locus its params hold (spec/api.md § Serving)"
        ]
    );
    assert_eq!(
        errors(&server(SERVED, "a: Server = Server { }; b: Server = Server { port: 8081 };")),
        vec![
            "exposure `web` is served twice: `Server`, which holds it, is held by `App` as `self.a` and `self.b`; an \
             exposure is served from one place, so hold its server once"
        ]
    );
    let replicated = server(SERVED, "server: Server = Server { };")
        .replace("    run() { }", "    placement { server: pinned(replicas = 2); }\n    run() { }");
    assert_eq!(
        errors(&replicated),
        vec![
            "exposure `web` is served 2 times: `App` places `server`, which holds `Server`, as 2 replicas; an \
             exposure is served from one place, so place its server unreplicated"
        ]
    );
}

#[test]
fn a_server_locus_listener_address_is_read_by_the_main_locus() {
    // the port the main locus's literal gives the server, its default, or a
    // construction's override: the listener reads it off the server as built
    assert_eq!(errors(&server(SERVED, "server: Server = Server { port: 9000 };")), Vec::<String>::new());
    assert_eq!(
        errors(&server(SERVED, "base: Int = 9000; server: Server = Server { port: self.base };")),
        Vec::<String>::new()
    );
    assert_eq!(
        errors(&server(SERVED, "server: Server = Server { };").replace("fn main() { App { }; }", "fn main() { App { server: Server { port: 9001 } }; }")),
        Vec::<String>::new()
    );
    // a method of the server is not a field the main locus can read
    let method = server(&SERVED.replace("to_string(self.port)", "to_string(self.port_of())"), "server: Server = Server { };")
        .replace("\n    }\n}\nmain locus App", "\n    }\n    fn port_of() -> Int { return self.port; }\n}\nmain locus App");
    let errs = errors(&method);
    assert!(
        errs.iter().any(|e| e.contains("exposure `web` held by `Server`: its listener is `App`'s, born after `self.server`")
            && e.contains("`self.port_of(…)`, a method of the holder")),
        "{errs:#?}"
    );
}

/// A server whose listener reads its address off the server is born after
/// the param of the main locus the server sits under, so everything the
/// main locus builds up to it, over its actual construction, must return
/// first (`listener_window`). One row per construction form and expression
/// position, each review witness among them, with its accepted neighbour:
/// `(row, server members, main params, declarations, fn main, refused at)`.
/// `refused at` is a part of the step the refusal names; `None` is admitted.
#[test]
fn what_main_builds_before_a_listener_born_after_its_server_must_return() {
    const HOLDS: &str = "server: Server = Server { };";
    const SPINNER: &str = "locus Spinner { run() { while !self.draining { } } }";
    const WAITER: &str = "locus Waiter {\n    params { web: std::api::Exposure; }\n    run() { self.web.wait(); }\n}";
    const SETTLE: &str = "fn settle() -> Int { std::time::sleep(1s); return 0; }";
    const PRINTED: &str = "fn printed() -> Int { println(\"x\"); return 0; }";
    const CONSUME: &str = "fn consume(s: Spinner) -> Int { return 0; }";
    const APP_N: &str = "n: Int = 0; server: Server = Server { };";
    const APP_N_SPINS: &str = "n: Int = { while true { } 0 }; server: Server = Server { };";
    const LAUNCHER_METHOD_SPINS: &str =
        "locus Launcher {\n    fn start() { App { n: { while true { } 0 } }; }\n}\nfn main() {\n    let l = Launcher { };\n    l.start();\n}";
    const LAUNCHER_METHOD_ZERO: &str =
        "locus Launcher {\n    fn start() { App { n: 0 }; }\n}\nfn main() {\n    let l = Launcher { };\n    l.start();\n}";
    const LAUNCHER_RUN_SPINS: &str = "locus Launcher {\n    run() { App { n: { while true { } 0 } }; }\n}\nfn main() { Launcher { }; }";
    const HOLDER_DEFAULT_SPINS: &str = "locus Holder {\n    params { app: App = App { n: { while true { } 0 } }; }\n}\nfn main() { Holder { }; }";
    const LAUNCH_FN_SPINS: &str = "fn launch() { App { n: { while true { } 0 } }; }\nfn main() { launch(); }";
    const TWO_SITES_ONE_SPINS: &str =
        "fn quick() { App { n: 1 }; }\nfn slow() { App { n: { while true { } 0 } }; }\nfn main() { quick(); slow(); }";
    const TWO_SITES_BOTH_RETURN: &str = "fn quick() { App { n: 1 }; }\nfn other() { App { n: 2 }; }\nfn main() { quick(); other(); }";
    const SETUP_BUILDS: &str = "locus Spinner { run() { while !self.draining { } } }\ntype Setup { n: Int = { Spinner { }; 0 }; }";
    const CFG_SETTLES: &str = "fn settle() -> Int { std::time::sleep(1s); return 0; }\ntype Cfg { n: Int = settle(); }";
    const NESTED_SETTLES: &str =
        "fn settle() -> Int { std::time::sleep(1s); return 0; }\ntype Inner { n: Int = settle(); }\ntype Outer { inner: Inner = Inner { }; }";
    const RECORD_BUILDS_APP: &str = "type Holder { n: Int = { App { n: { while true { } 0 } }; 0 }; }\nfn main() { let h = Holder { }; }";
    const HELPER_SETTLES: &str = "fn settle() -> Int { std::time::sleep(1s); return 0; }\nfn helper(n: Int = settle()) -> Int { return n; }";
    const CONST_SETTLES: &str = "fn settle() -> Int { std::time::sleep(1s); return 0; }\nconst K: Int = settle();";
    const PICK_FREE_SETTLES: &str =
        "fn settle() -> Int { std::time::sleep(1s); return 0; }\nfn pick(n: Int = settle()) -> Int { return n; }";
    const PICK_FREE_RETURNS: &str = "fn settle() -> Int { std::time::sleep(1s); return 0; }\nfn pick(n: Int = 0) -> Int { return n; }";
    // a default typed in each scope that evaluates it, one of them
    // shadowing the const it reads
    const SETUP_READS_K: &str = "fn settle() -> Int { std::time::sleep(1s); return 0; }\nconst K: Int = settle();\ntype Setup { n: Int = K; }";
    const ARG_READS_K: &str = "fn settle() -> Int { std::time::sleep(1s); return 0; }\nconst K: Int = settle();\nfn pick_k(n: Int = K) -> Int { return n; }";
    // a call in a record default on a receiver the caller's scope binds:
    // `s` is whichever locus the literal's scope holds
    const PINGS: &str = "type Ping { id: Int = 0; }\ntopic Pings { payload: Ping; subject: \"t.pings\"; }";
    const SUB_SPINS: &str = "locus Sub {\n    bus { subscribe Pings as on_ping; }\n    fn on_ping(p: Ping) { while !self.draining { } }\n}";
    const SUB_RETURNS: &str = "locus Sub {\n    bus { subscribe Pings as on_ping; }\n    fn on_ping(p: Ping) { println(\"ping\"); }\n}";
    const PUB_BIRTH: &str = "locus Pub {\n    bus { publish Pings; }\n    birth() { Pings <- Ping { id: 1 }; }\n}";
    const SHADOW_FN_LOCAL: &str =
        "fn settle() -> Int { std::time::sleep(1s); return 0; }\nconst K: Int = settle();\nfn quick() -> Int { let K = 0; return K; }";
    const SHADOW_FN_PARAM: &str =
        "fn settle() -> Int { std::time::sleep(1s); return 0; }\nconst K: Int = settle();\nfn twice(K: Int) -> Int { return K + K; }";
    const SHADOW_MATCH: &str =
        "fn settle() -> Int { std::time::sleep(1s); return 0; }\nconst K: Int = settle();\ntype Boxed = enum { Of(Int), Empty };";
    const KEYED_SUB: &str = "fn settle() -> Int { std::time::sleep(1s); return 0; }\nconst K: Int = settle();\ntype Ping { id: Int = 0; }\ntopic Pings { payload: Ping; subject: \"t.pings\"; keyed_by id; }\nlocus Sub {\n    bus { subscribe Pings as on_ping where key == K; }\n    fn on_ping(p: Ping) { }\n}";
    let rows: &[(&str, &str, &str, &str, &str, Option<&str>)] = &[
        // ---- the server's hooks ----
        ("run() waits", "    run() { self.web.wait(); }", HOLDS, "", "", Some("`Server`'s `run()`")),
        ("run() waits through a fn", "    fn hold() { self.web.wait(); }\n    run() { self.hold(); }", HOLDS, "", "", Some("`Server`'s `run()`")),
        ("run() returns", "    run() { println(\"started\"); }", HOLDS, "", "", None),
        ("birth() waits", "    birth() { self.web.wait(); }", HOLDS, "", "", Some("`Server`'s `birth()`")),
        ("birth() returns", "    birth() { println(\"born\"); }", HOLDS, "", "", None),
        // ---- a held child built inline, its run() where it is born ----
        ("held child waits", "", HOLDS, WAITER, "", Some("`Waiter`'s `run()`")),
        // ---- loops in every expression position, in run() and in a fn it calls ----
        ("loop in a let initializer", "    run() { let n = { while !self.draining { } 0 }; }", HOLDS, "", "", Some("a `while` loop")),
        ("loop in a helper's initializer", "    fn spin() -> Int { let n = { while !self.draining { } 0 }; return n; }\n    run() { let n = self.spin(); }", HOLDS, "", "", Some("a `while` loop")),
        ("loop in a block tail", "    run() { let n = { let m = 1; m + { while !self.draining { } 0 } }; }", HOLDS, "", "", Some("a `while` loop")),
        ("loop in an if arm", "    run() { let n = if self.port > 0 { while !self.draining { } 1 } else { 2 }; }", HOLDS, "", "", Some("a `while` loop")),
        ("loop in a match arm", "    run() { let n = match self.port { 0 -> { while !self.draining { } 1 }, _ -> 2, }; }", HOLDS, "", "", Some("a `while` loop")),
        ("loop in a call argument", "    run() { println(to_string({ while !self.draining { } 1 })); }", HOLDS, "", "", Some("a `while` loop")),
        ("loop in a literal field", "    run() { let r = Req { n: { while !self.draining { } 1 } }; }", HOLDS, "", "", Some("a `while` loop")),
        ("finite block initializer", "    run() { let n = { 0 }; println(to_string(n)); }", HOLDS, "", "", None),
        // ---- a main param's default: calls ----
        ("default calls a sleeping fn", "", "n: Int = settle(); server: Server = Server { };", SETTLE, "", Some("the default of `App.n`, through `settle`")),
        ("default calls a printing fn", "", "n: Int = printed(); server: Server = Server { };", PRINTED, "", None),
        ("default calls a builtin", "", "n: Int = len(\"a\"); server: Server = Server { };", "", "", None),
        ("default calls an enum variant", "", "s: Shape = Shape::One(1); server: Server = Server { };", "type Shape = enum { One(Int), Zero };", "", None),
        // ---- a main param's default: a locus built anywhere in it ----
        ("an array of loci", "", "workers: [Spinner; 1] = [Spinner { }]; server: Server = Server { };", SPINNER, "", Some("`Spinner`'s `run()`")),
        ("an array of loci after the server", "", "server: Server = Server { }; workers: [Spinner; 1] = [Spinner { }];", SPINNER, "", None),
        ("a locus in a block default", "", "n: Int = { Spinner { }; 0 }; server: Server = Server { };", SPINNER, "", Some("`Spinner`'s `run()`")),
        ("a locus in a tuple", "", "t: (Spinner, Int) = (Spinner { }, 1); server: Server = Server { };", SPINNER, "", Some("`Spinner`'s `run()`")),
        ("a locus as a call argument", "", "n: Int = consume(Spinner { }); server: Server = Server { };", &format!("{SPINNER}\n{CONSUME}"), "", Some("`Spinner`'s `run()`")),
        ("a locus in an if arm", "", "n: Int = if true { Spinner { }; 1 } else { 0 }; server: Server = Server { };", SPINNER, "", Some("`Spinner`'s `run()`")),
        // ---- a sibling born before the server ----
        ("an unplaced sibling spins", "", "spinner: Spinner = Spinner { }; server: Server = Server { };", SPINNER, "", Some("`Spinner`'s `run()`")),
        // ---- the actual construction: a supplied field's default is not run ----
        ("a blocking default the literal supplies", "        n: Int = settle();", "server: Server = Server { n: 0 };", SETTLE, "", None),
        ("a blocking default the literal leaves", "        n: Int = settle();", HOLDS, SETTLE, "", Some("the default of `Server.n`, through `settle`")),
        ("a blocking default fn main supplies", "        n: Int = settle();", HOLDS, SETTLE, "fn main() { App { server: Server { n: 0 } }; }", None),
        ("a blocking value fn main gives", "", HOLDS, SETTLE, "fn main() { App { server: Server { port: settle() } }; }", Some("the value given of `App.server`")),
        // ---- birth_check runs after birth(), before the listener ----
        ("a birth_check that spins", "    closure stalled { epoch inline; }\n    birth_check { { while !self.draining { } true } } -> violate stalled;", HOLDS, "", "", Some("`Server`'s `birth_check`")),
        ("a birth_check that returns", "    closure stalled { epoch inline; }\n    birth_check { self.port > 0 } -> violate stalled;", HOLDS, "", "", None),
        // ---- where the main locus is built: every place an expression sits ----
        ("built in another locus's method", "", APP_N, "", LAUNCHER_METHOD_SPINS, Some("the value given of `App.n`")),
        ("built in another locus's method, overriding a blocking default", "", APP_N_SPINS, "", LAUNCHER_METHOD_ZERO, None),
        ("built in another locus's run()", "", APP_N, "", LAUNCHER_RUN_SPINS, Some("the value given of `App.n`")),
        ("built in another locus's param default", "", APP_N, "", HOLDER_DEFAULT_SPINS, Some("the value given of `App.n`")),
        ("built in a free fn", "", APP_N, "", LAUNCH_FN_SPINS, Some("the value given of `App.n`")),
        ("built at two sites, one spins", "", APP_N, "", TWO_SITES_ONE_SPINS, Some("the value given of `App.n`")),
        ("built at two sites, both return", "", APP_N, "", TWO_SITES_BOTH_RETURN, None),
        // ---- what a construction evaluates without writing it ----
        // a record literal's omitted fields: their defaults
        ("a record default builds a locus", "", "setup: Setup = Setup { }; server: Server = Server { };", SETUP_BUILDS, "", Some("the default of `Setup.n`")),
        ("a record literal supplies that field", "", "setup: Setup = Setup { n: 0 }; server: Server = Server { };", SETUP_BUILDS, "", None),
        ("a record default calls a sleeping fn", "", "cfg: Cfg = Cfg { }; server: Server = Server { };", CFG_SETTLES, "", Some("the default of `Cfg.n`, through `settle`")),
        ("a record built inside a block default", "", "n: Int = { let c = Cfg { }; c.n }; server: Server = Server { };", CFG_SETTLES, "", Some("the default of `Cfg.n`")),
        ("a record in a server's run()", "    run() { let c = Cfg { }; }", HOLDS, CFG_SETTLES, "", Some("`Server`'s `run()`")),
        ("a record default's own record default", "", "o: Outer = Outer { }; server: Server = Server { };", NESTED_SETTLES, "", Some("the default of `Outer.inner`, through the default of `Inner.n`")),
        ("a record default's record, its field supplied", "", "o: Outer = Outer { inner: Inner { n: 0 } }; server: Server = Server { };", NESTED_SETTLES, "", None),
        // an all-defaulted record param with no value: the `T { }` the compiler builds
        ("a record param with no value", "", "cfg: Cfg; server: Server = Server { };", CFG_SETTLES, "fn main() { }", Some("the default of `App.cfg`, through the default of `Cfg.n`")),
        ("a record param with no value, after the server", "", "server: Server = Server { }; cfg: Cfg;", CFG_SETTLES, "fn main() { }", None),
        // a record default is a place the main locus can be built
        ("built in a record's field default", "", APP_N, "", RECORD_BUILDS_APP, Some("the value given of `App.n`")),
        // a call's omitted arguments: its params' defaults, at the call site
        ("a call leaves a sleeping param default", "", "k: Int = helper(); server: Server = Server { };", HELPER_SETTLES, "", Some("the default of `helper`'s param `n`")),
        ("a call supplies that argument", "", "k: Int = helper(1); server: Server = Server { };", HELPER_SETTLES, "", None),
        ("a method call leaves a sleeping param default", "    fn wait_for(n: Int = settle()) -> Int { return n; }\n    run() { let k = self.wait_for(); }", HOLDS, SETTLE, "", Some("`Server`'s `run()`")),
        // a const that is no literal: evaluated where it is read
        ("a const read by a default", "", "n: Int = K; server: Server = Server { };", CONST_SETTLES, "", Some("the value of the const `K`")),
        ("a literal const read by a default", "", "n: Int = K; server: Server = Server { };", "const K: Int = 5;", "", None),
        // the defaults a call leaves are its callee's as the checker resolves
        // the call (the `omitted_args` column), as lowering expands them; a
        // local fn value named like a fn is the callee (spec/types.md, the
        // binding in scope shadows the fn): a call through a value, whose
        // return the window cannot name, not `helper`'s sleeping default
        ("a call by a fn's name a local fn value also has", "", "n: Int = { let helper = zero; helper() }; server: Server = Server { };", &format!("{HELPER_SETTLES}\nfn zero() -> Int {{ return 0; }}"), "", Some("a call through a value")),
        ("a method call, a free fn of its name sleeping", "    fn pick(n: Int = 0) -> Int { return n; }\n    run() { let k = self.pick(); }", HOLDS, PICK_FREE_SETTLES, "", None),
        ("a method call, the method sleeping", "    fn pick(n: Int = settle()) -> Int { return n; }\n    run() { let k = self.pick(); }", HOLDS, PICK_FREE_RETURNS, "", Some("`Server`'s `run()`")),
        ("a free call in a method, a method of its name sleeping", "    fn pick(n: Int = settle()) -> Int { return n; }\n    run() { let k = pick(); }", HOLDS, PICK_FREE_RETURNS, "", None),
        ("a free call in a default, a method of its name sleeping", "    fn pick(n: Int = settle()) -> Int { return n; }", "k: Int = pick(); server: Server = Server { };", PICK_FREE_RETURNS, "", None),
        // the main locus built inside a record default: its construction is
        // walked on the path the checker typed it on (the Holder literal's)
        ("built in a record default, under a shadowing let", "", APP_N, CONST_SETTLES, "type Holder { n: Int = { let K = 0; App { n: K }; 0 }; }\nfn main() { let h = Holder { }; }", None),
        ("built in a record default, reading the const", "", APP_N, CONST_SETTLES, "type Holder { n: Int = { App { n: K }; 0 }; }\nfn main() { let h = Holder { }; }", Some("the value given of `App.n`, through the value of the const `K`")),
        // a call in a default resolved in each evaluation's scope
        // recursion: the window cannot prove one ends, so it refuses any
        // the step reaches, finite or not
        ("a default calls a fn that calls itself", "", "n: Int = settle(); server: Server = Server { };", "fn settle() -> Int { return settle(); }", "", Some("`settle`")),
        ("a default calls a finite recursion", "", "n: Int = count(3); server: Server = Server { };", "fn count(k: Int) -> Int { if k == 0 { return 0; } return count(k - 1); }", "", Some("`count`")),
        ("a default calls two fns calling each other", "", "n: Int = ping(); server: Server = Server { };", "fn ping() -> Int { return pong(); }\nfn pong() -> Int { return ping(); }", "", Some("`p")),
        ("the server's run() calls a method calling itself", "    fn again() -> Int { return self.again(); }\n    run() { let k = self.again(); }", HOLDS, "", "", Some("`Server`'s `run()`")),
        ("a recursion after the server", "", "server: Server = Server { }; n: Int = settle();", "fn settle() -> Int { return settle(); }", "", None),
        ("a call that returns, no recursion", "", "n: Int = count(3); server: Server = Server { };", "fn count(k: Int) -> Int { return k; }", "", None),
        // a default that reaches its own call again
        ("a param default calling its own fn", "", "k: Int = grow(); server: Server = Server { };", "fn grow(n: Int = grow()) -> Int { return n; }", "", Some("may not return")),
        // a publish runs its subscribers' handlers inside the window, and a
        // `yield` runs whatever main's queue holds
        ("a sibling's birth() publishes to a spinning handler", "", "sub: Sub = Sub { }; p: Pub = Pub { }; server: Server = Server { };", &format!("{PINGS}\n{SUB_SPINS}\n{PUB_BIRTH}"), "", Some("`Pub`'s `birth()`")),
        ("a default calls a method that publishes to a spinning handler", "", "sub: Sub = Sub { }; p: Pub = Pub { }; n: Int = self.p.announce(); server: Server = Server { };", &format!("{PINGS}\n{SUB_SPINS}\nlocus Pub {{\n    bus {{ publish Pings; }}\n    fn announce() -> Int {{ Pings <- Ping {{ id: 1 }}; return 0; }}\n}}"), "", Some("the default of `App.n`, through `Pub::announce`, through `Sub::on_ping`, a handler on the main thread of `t.pings`")),
        ("a sibling's birth() publishes to a returning handler", "", "sub: Sub = Sub { }; p: Pub = Pub { }; server: Server = Server { };", &format!("{PINGS}\n{SUB_RETURNS}\n{PUB_BIRTH}"), "", None),
        ("a publish to a spinning handler after the server", "", "server: Server = Server { }; sub: Sub = Sub { }; p: Pub = Pub { };", &format!("{PINGS}\n{SUB_SPINS}\n{PUB_BIRTH}"), "", None),
        ("a sibling's birth() yields", "", "y: Yielder = Yielder { }; server: Server = Server { };", "locus Yielder { birth() { println(\"y\"); yield; } }", "", Some("a `yield`")),
        ("a sibling's birth() without a yield", "", "y: Yielder = Yielder { }; server: Server = Server { };", "locus Yielder { birth() { println(\"y\"); } }", "", None),
        // a fn used as a value: judged as a call of it
        ("a looping fn passed as a value", "", "n: Int = apply(spin1, 1); server: Server = Server { };", "fn spin1(n: Int) -> Int { while true { } return n; }\nfn apply(f: fn (Int) -> Int, n: Int) -> Int { return f(n); }", "", Some("`spin1`")),
        ("a returning fn passed as a value", "", "n: Int = apply(same, 1); server: Server = Server { };", "fn same(n: Int) -> Int { return n; }\nfn apply(f: fn (Int) -> Int, n: Int) -> Int { return f(n); }", "", None),
        // a const read is the checker's resolution: a binding in scope shadows it
        ("a let shadows the const", "", "n: Int = { let K = 0; K }; server: Server = Server { };", CONST_SETTLES, "", None),
        ("a fn's let shadows the const", "", "n: Int = quick(); server: Server = Server { };", SHADOW_FN_LOCAL, "", None),
        ("a fn's param shadows the const", "", "n: Int = twice(1); server: Server = Server { };", SHADOW_FN_PARAM, "", None),
        ("a match binding shadows the const", "", "n: Int = match Boxed::Of(1) { Boxed::Of(K) -> K, Boxed::Empty -> 0, }; server: Server = Server { };", SHADOW_MATCH, "", None),
        ("a for binding shadows the const", "", "n: Int = { let mut t = 0; for K in 0..3 { t = t + K; } t }; server: Server = Server { };", CONST_SETTLES, "", None),
        // a default reads only its own fields and constants (spec/types.md):
        // a caller binding that would shadow one is refused at the check
        ("a record default reads a sleeping const", "", "n: Int = { let s = Setup { }; s.n }; server: Server = Server { };", &format!("{SETUP_READS_K}\nfn other() -> Int {{ let s = Setup {{ }}; return s.n; }}"), "", Some("the default of `Setup.n`, through the value of the const `K`")),
        ("a param default reads a sleeping const", "", "n: Int = pick_k(); server: Server = Server { };", &format!("{ARG_READS_K}\nfn other() -> Int {{ return pick_k(); }}"), "", Some("the default of `pick_k`'s param `n`, through the value of the const `K`")),
        ("a const read after a block whose let shadowed it", "", "n: Int = { { let K = 0; println(to_string(K)); } K }; server: Server = Server { };", CONST_SETTLES, "", Some("the value of the const `K`")),
        ("a fn's const read after its shadowing block", "", "n: Int = later(); server: Server = Server { };", &format!("{CONST_SETTLES}\nfn later() -> Int {{ {{ let K = 0; println(to_string(K)); }} return K; }}"), "", Some("the default of `App.n`, through the value of the const `K`")),
        // a subscription's key: evaluated where its locus is built
        ("a subscription key reads a sleeping const", "", "sub: Sub = Sub { }; server: Server = Server { };", KEYED_SUB, "", Some("a `where key == …` of `Sub`")),
    ];
    let mut failures = Vec::new();
    for (row, members, params, decls, main_fn, refused_at) in rows {
        let mut src = server(SERVED, "");
        if members.starts_with("        ") {
            // a param of the server
            src = src.replace("        orders: Orders = Orders { };\n", &format!("        orders: Orders = Orders {{ }};\n{members}\n"));
        } else if !members.is_empty() {
            src = src.replace("\n    }\n}\nmain locus App", &format!("\n    }}\n{members}\n}}\nmain locus App"));
        }
        if *decls == WAITER {
            src = src.replace("bound: 8, on_full: refuse);\n    }", "bound: 8, on_full: refuse);\n        waiter: Waiter = Waiter { web: self.web };\n    }");
        }
        src = src.replace("    params {\n        \n    }\n    run() { }", &format!("    params {{\n        {params}\n    }}\n    run() {{ }}"));
        src = src.replace("fn main() { App { }; }", &format!("{decls}\n{}", if main_fn.is_empty() { "fn main() { App { }; }" } else { main_fn }));
        let errs = errors(&src);
        // admitted: no error; refused: the refusal, naming the step, and
        // nothing else
        let ok = match refused_at {
            None => errs.is_empty(),
            Some(at) => {
                errs.len() == 1
                    && errs.iter().any(|e| e.contains("exposure `web` held by `Server`: its address reads `Server`, so its listener (`App`'s) is born after `self.server`")
                        && !e.contains(",,")
                        && e.contains(at)
                        && e.contains("may not return"))
            }
        };
        if !ok {
            failures.push(format!("{row}: want {refused_at:?}: {errs:#?}\n{src}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    // placed: the server's own run() runs on its pool, its birth() is awaited
    let placed = |members: &str| {
        server(SERVED, HOLDS)
            .replace("\n    }\n}\nmain locus App", &format!("\n    }}\n{members}\n}}\nmain locus App"))
            .replacen("    run() { }\n", "    placement { server: cooperative(pool = io) where async_io; }\n    run() { }\n", 1)
    };
    assert_eq!(errors(&placed("    run() { self.web.wait(); }")), Vec::<String>::new());
    assert!(errors(&placed("    birth() { self.web.wait(); }")).iter().any(|e| e.contains("`Server`'s `birth()`")));
    // a pinned sibling runs on its own thread
    let pinned = server(SERVED, "spinner: Spinner = Spinner { }; server: Server = Server { };")
        .replace("fn main()", &format!("{SPINNER}\nfn main()"))
        .replacen("    run() { }\n", "    placement { spinner: pinned; }\n    run() { }\n", 1);
    assert_eq!(errors(&pinned), Vec::<String>::new());
    // an address that reads nothing of the server: the listener is born first
    let constant = server(SERVED, HOLDS)
        .replace("\n    }\n}\nmain locus App", "\n    }\n    birth() { self.web.wait(); }\n}\nmain locus App")
        .replace("to_string(self.port)", "\"8080\"");
    assert_eq!(errors(&constant), Vec::<String>::new());
}

/// A default that calls a fn an imported seed declares (`lib::settle()`)
/// is followed through the import renames to that fn, as the checker
/// resolves it, and refused when it may not return.
#[test]
fn an_imported_fn_called_before_the_listener_is_followed() {
    let d = std::env::temp_dir().join(format!("hale_listener_window_import_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/main.hl"), "fn settle() -> Int { while true { } return 0; }\nfn quick() -> Int { return 1; }\n").unwrap();
    let entry = |n: &str| {
        format!(
            "import \"lib\" as lib;\n{}",
            server(SERVED, &format!("n: Int = lib::{n}(); server: Server = Server {{ }};"))
        )
    };
    std::fs::write(d.join("main.hl"), entry("settle")).unwrap();
    let errs = errors_of_seed(&d);
    assert!(errs.iter().any(|e| e.contains("the default of `App.n`") && e.contains("may not return")), "{errs:#?}");
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/main.hl"), "fn settle() -> Int { while true { } return 0; }\nfn quick() -> Int { return 1; }\n").unwrap();
    std::fs::write(d.join("main.hl"), entry("quick")).unwrap();
    assert_eq!(errors_of_seed(&d), Vec::<String>::new());
    // an imported fn's param default, left by the call: evaluated at the
    // call, as the checker's `omitted_args` column resolves it
    let seed = || {
        std::fs::create_dir_all(d.join("lib")).unwrap();
        std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
        std::fs::write(d.join("lib/hale.toml"), "[deps]\n").unwrap();
    };
    seed();
    std::fs::write(
        d.join("lib/main.hl"),
        "fn settle() -> Int { while true { } return 0; }\nfn helper(n: Int = settle()) -> Int { return n; }\n",
    )
    .unwrap();
    let call = |args: &str| {
        format!(
            "import \"lib\" as lib;\n{}",
            server(SERVED, &format!("n: Int = lib::helper({args}); server: Server = Server {{ }};"))
        )
    };
    std::fs::write(d.join("main.hl"), call("")).unwrap();
    let errs = errors_of_seed(&d);
    assert!(
        errs.iter().any(|e| e.contains("the default of `App.n`") && e.contains("'s param `n`") && e.contains("may not return")),
        "{errs:#?}"
    );
    seed();
    std::fs::write(
        d.join("lib/main.hl"),
        "fn settle() -> Int { while true { } return 0; }\nfn helper(n: Int = settle()) -> Int { return n; }\n",
    )
    .unwrap();
    std::fs::write(d.join("main.hl"), call("1")).unwrap();
    assert_eq!(errors_of_seed(&d), Vec::<String>::new());
}

/// The R0 witness is admitted as R1 admitted it: its serve sites are over
/// `http::Rpc` and `unix::Rpc`, which no check reads as a type, and its
/// receivers are bound as the laws ask.
#[test]
fn the_witness_is_admitted() {
    let path = hale_corpus::repo_root().join("tests/api-contract/program.hl");
    let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(false, false)).ok().expect("it loads");
    let checked = snap.demand_check().ok().expect("it checks");
    let errors: Vec<&str> = checked.diags.iter().filter(|d| d.is_error()).map(|d| d.message.as_str()).collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// The laws read the same sites `hale check --api` names: the witness's
/// serve sites are the ones its inventory lists.
#[test]
fn the_witness_serve_sites_are_the_inventorys() {
    let path = hale_corpus::repo_root().join("tests/api-contract/program.hl");
    let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(false, false)).ok().expect("it loads");
    let rows = snap.demand_surface_rows().expect("rows");
    let names: Vec<&str> = rows.serves.iter().filter_map(|s| s.name.as_deref()).collect();
    assert_eq!(names, vec!["public", "partner", "admin"]);
}

// ---- a main locus the program imports ----

/// A seed whose library declares a `main locus` that serves, and an entry
/// program `entry` (the library is `import "lib" as lib;`).
fn seed_with_serving_library(tag: &str, entry: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("hale_serve_imported_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/hale.toml"), "[deps]\n").unwrap();
    std::fs::write(
        d.join("lib/main.hl"),
        "type Req { n: Int; }
type Res { n: Int; }
locus Echo { fn echo(r: Req) -> Res { return Res { n: r.n }; } }
api Public { rpc Echo::echo; }
main locus LibMain {
    params {
        echo: Echo = Echo { };
        fixture: std::api::test::Rpc = std::api::test::Rpc { };
        public: api::Exposure = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse);
    }
    run() {
        self.public.stop();
    }
}
fn main() { LibMain { }; }
",
    )
    .unwrap();
    std::fs::write(d.join("main.hl"), entry).unwrap();
    d
}

fn errors_of_seed(dir: &std::path::Path) -> Vec<String> {
    let snap = Snapshot::load(dir, LoadMode::WholeSeed, &Disk, Config::check(false, false)).ok().expect("it loads");
    let diags = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    // (the CLI writes an imported name as the program spells it)
    let out = diags.iter().filter(|d| d.is_error()).map(|d| d.message.replace("__lib_lib___main__LibMain", "lib::LibMain")).collect();
    let _ = std::fs::remove_dir_all(dir);
    out
}

const IMPORTED_MAIN_SERVES: &str = "`api::serve` in `lib::LibMain`, a main locus the program imports: only the entry program's main locus serves; serve from the entry's main locus or hold the exposure there (spec/api.md § Serving)";

#[test]
fn a_serve_in_a_main_locus_the_program_builds_but_imports_is_refused() {
    // the program writes the library's main locus as a part of its own main
    // locus: the exposure would never be built, since only the entry's main
    // locus serves
    let as_a_child = seed_with_serving_library(
        "child",
        "import \"lib\" as lib;
main locus App { params { inner: lib::LibMain = lib::LibMain { }; } run() { } }
fn main() { App { }; }
",
    );
    assert_eq!(errors_of_seed(&as_a_child), vec![IMPORTED_MAIN_SERVES.to_string()]);
    // and as the whole program, `fn main() { lib::LibMain { }; }` (a seed whose
    // only main locus is imported has no entry)
    let as_the_program = seed_with_serving_library(
        "program",
        "import \"lib\" as lib;
fn main() { lib::LibMain { }; }
",
    );
    assert_eq!(errors_of_seed(&as_the_program), vec![IMPORTED_MAIN_SERVES.to_string()]);
}

#[test]
fn a_library_whose_main_locus_is_only_imported_for_its_types_is_admitted() {
    // its serve site never runs, and the program says nothing of it: an
    // application's tests import the application this way
    let types_only = seed_with_serving_library(
        "types",
        "import \"lib\" as lib;
main locus App { run() { let r = lib::Req { n: 1 }; println(r.n); } }
fn main() { App { }; }
",
    );
    assert_eq!(errors_of_seed(&types_only), Vec::<String>::new());
}

// ---- where the listeners run, by the placement table (GH #1565) ----

/// A main locus serving five ways: its own exposure placed by its row
/// (`web` on `io`) and one unplaced (`sock`); a server locus placed on an
/// `async_io` pool (`admin` on `io2`); a server one level deeper, under a
/// param placed on `io3`; a server on the main thread (`plain`) and one
/// pinned (`solo`), whose listeners take the transport's shared pool.
const PLACED: &str = r#"
type Req { n: Int; }
type Res { n: Int; }
locus Cfg { fn ask(r: Req) -> Res { return Res { n: r.n }; } }
api Admin { rpc Cfg::ask; }
locus AdminServer {
    params {
        cfg: Cfg = Cfg { };
        api: api::Exposure = api::serve(Admin, unix::Rpc { path: "/tmp/hale-1565-admin.sock" }, as: "admin", receivers: { Cfg: self.cfg }, bound: 8, on_full: refuse);
    }
}
locus DeepServer {
    params {
        cfg: Cfg = Cfg { };
        api: api::Exposure = api::serve(Admin, unix::Rpc { path: "/tmp/hale-1565-deep.sock" }, as: "deep", receivers: { Cfg: self.cfg }, bound: 8, on_full: refuse);
    }
}
locus Wrapper { params { inner: DeepServer = DeepServer { }; } }
locus PlainServer {
    params {
        cfg: Cfg = Cfg { };
        api: api::Exposure = api::serve(Admin, unix::Rpc { path: "/tmp/hale-1565-plain.sock" }, as: "plain", receivers: { Cfg: self.cfg }, bound: 8, on_full: refuse);
    }
}
locus SoloServer {
    params {
        cfg: Cfg = Cfg { };
        api: api::Exposure = api::serve(Admin, unix::Rpc { path: "/tmp/hale-1565-solo.sock" }, as: "solo", receivers: { Cfg: self.cfg }, bound: 8, on_full: refuse);
    }
}
main locus App {
    params {
        cfg: Cfg = Cfg { };
        web: api::Exposure = api::serve(Admin, unix::Rpc { path: "/tmp/hale-1565-web.sock" }, as: "web", receivers: { Cfg: self.cfg }, bound: 8, on_full: refuse);
        sock: api::Exposure = api::serve(Admin, unix::Rpc { path: "/tmp/hale-1565-sock.sock" }, as: "sock", receivers: { Cfg: self.cfg }, bound: 8, on_full: refuse);
        admin: AdminServer = AdminServer { };
        deep: Wrapper = Wrapper { };
        plain: PlainServer = PlainServer { };
        solo: SoloServer = SoloServer { };
    }
    placement {
        web: cooperative(pool = io) where async_io;
        admin: cooperative(pool = io2) where async_io;
        deep: cooperative(pool = io3) where async_io;
        solo: pinned;
    }
    run() { }
}
fn main() { App { }; }
"#;

/// Each instance the placement table places, as `path -> domain`.
fn domains(src: &str) -> std::collections::BTreeMap<String, String> {
    use hale_types::placement::DomainKind;
    let parsed = hale_syntax::parse_source(src).unwrap_or_else(|d| panic!("parse: {:?}", d[0].message));
    let snap = Snapshot::from_program(parsed, Vec::new(), Config::check(false, false)).ok().expect("a snapshot");
    let checked = snap.demand_check().ok().expect("it checks");
    let errors: Vec<&str> = checked.diags.iter().filter(|d| d.is_error()).map(|d| d.message.as_str()).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let table = snap.demand_placement().ok().expect("a placement table");
    table
        .instances
        .iter()
        .filter(|(k, _)| k.replica.is_none())
        .map(|(k, r)| {
            let d = match &table.domain(r.domain).kind {
                DomainKind::Main => "main".to_string(),
                DomainKind::Pool { name, .. } => format!("pool:{name}"),
                DomainKind::Pinned { .. } => "pinned".to_string(),
            };
            (table.path_of(k), d)
        })
        .collect()
}

#[test]
fn a_listener_runs_where_its_exposure_or_its_server_is_placed() {
    let d = domains(PLACED);
    let at = |path: &str| d.get(path).cloned().unwrap_or_else(|| panic!("no instance `{path}` in {d:#?}"));
    // the listeners, numbered by exposure in declaration order: the
    // servers' (1 admin, 2 deep, 3 plain, 4 solo) and then main's own
    // (5 web, 6 sock)
    assert_eq!(at("App.__rpc_u_5"), "pool:io", "main's exposure: its row places its listener");
    assert_eq!(at("App.__rpc_u_6"), "pool:__api_unix", "unplaced: the transport's shared pool");
    assert_eq!(at("App.__rpc_u_1"), "pool:io2", "a server placed on an async_io pool: its listener with it");
    assert_eq!(at("App.__rpc_u_2"), "pool:io3", "a server a level deeper: its nearest placed ancestor's pool");
    assert_eq!(at("App.__rpc_u_3"), "pool:__api_unix", "a server on the main thread: the shared pool");
    assert_eq!(at("App.__rpc_u_4"), "pool:__api_unix", "a pinned server: the shared pool");
    // the servers, and the receivers they hold, where main places them
    assert_eq!(at("App.cfg"), "main");
    assert_eq!(at("App.admin"), "pool:io2");
    assert_eq!(at("App.admin.cfg"), "pool:io2");
    assert_eq!(at("App.deep.inner"), "pool:io3");
    assert_eq!(at("App.deep.inner.cfg"), "pool:io3");
    assert_eq!(at("App.plain.cfg"), "main");
    assert_eq!(at("App.solo.cfg"), "pinned");
}

/// A bare bundle (the program alone, no stdlib program beside it, no
/// scalar type of its own), checked through `check_bundle` as a harness
/// does: a record default is typed at each literal that leaves it, so the
/// window reads each evaluation's resolution of the const in it rather
/// than its spelling. (The field-default walk no longer skips a program
/// with no scalar type; the unit rows carry the stdlib's `Duration` and
/// `Time` into every check, so that skip never fired.)
#[test]
fn a_bundle_with_no_scalar_type_still_types_each_default_evaluation() {
    let errors_bare = |src: &str| -> Vec<String> {
        let program = hale_syntax::parse_source(src).unwrap_or_else(|d| panic!("parse: {:?}", d[0].message));
        let mut programs = std::collections::BTreeMap::new();
        programs.insert("test.hl".to_string(), &program);
        let bundle = hale_types::Bundle::new(programs);
        let (top, mut diags) = hale_types::resolve::build_top_scope(&bundle);
        diags.extend(hale_types::check::check_bundle(&bundle, &top, true));
        diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
    };
    let program = |n: &str| {
        server(SERVED, &format!("n: Int = {n}; server: Server = Server {{ }};")).replace(
            "fn main()",
            "fn settle() -> Int { while true { } return 0; }\nconst K: Int = settle();\ntype Setup { n: Int = K; }\n\
             fn other() -> Int { let s = Setup { }; return s.n; }\nfn main()",
        )
    };
    let refusal = |errs: &[String]| {
        errs.iter().any(|e| e.contains("the default of `Setup.n`, through the value of the const `K`") && e.contains("may not return"))
    };
    let shadowed = errors_bare(&program("{ let K = 0; let s = Setup { }; s.n }"));
    assert!(!refusal(&shadowed), "{shadowed:#?}");
    let read = errors_bare(&program("{ let s = Setup { }; s.n }"));
    assert!(refusal(&read), "{read:#?}");
}

/// What a step does that may not end without a loop of its own: a publish
/// to a topic a transport that may wait carries (a unix socket's write
/// here), and a recovery, which rebuilds a child the window does not
/// follow. Each refused before the listener, with its admitted neighbour.
#[test]
fn a_publish_that_may_wait_and_a_recovery_before_the_listener_are_refused() {
    let refused = |errs: &[String], why: &str| {
        errs.iter().any(|e| {
            e.contains("exposure `web` held by `Server`") && e.contains("`Server`'s `run()`") && e.contains(why)
        })
    };
    let with_run = |run: &str| {
        server(SERVED, "server: Server = Server { };")
            .replace("\n    }\n}\nmain locus App", &format!("\n    }}\n    bus {{ publish Beat; }}\n{run}\n}}\nmain locus App"))
            .replace("fn main()", "type Tick { n: Int = 0; }\ntopic Beat { payload: Tick; subject: \"demo.beat\"; }\nfn main()")
    };
    let publish = with_run("    run() { Beat <- Tick { n: 1 }; }");
    let bound = publish.replacen(
        "    run() { }\n",
        "    bindings { Beat: unix(\"/tmp/hale-1565-beat.sock\", role: listen); }\n    run() { }\n",
        1,
    );
    let errs = errors(&bound);
    assert!(refused(&errs, "a publish to a topic bound to a transport that may wait"), "{errs:#?}\n{bound}");
    // the same publish over the cooperative queue returns
    assert_eq!(errors(&publish), Vec::<String>::new(), "{publish}");
    let recovery = with_run("    run() { restart(self.orders); }");
    let errs = errors(&recovery);
    assert!(refused(&errs, "a recovery"), "{errs:#?}\n{recovery}");
    // placed off the main thread, the server's run() is its own
    let placed = recovery.replacen(
        "    run() { }\n",
        "    placement { server: cooperative(pool = io) where async_io; }\n    run() { }\n",
        1,
    );
    assert_eq!(errors(&placed), Vec::<String>::new(), "{placed}");
}

/// A publish before the listener runs the handlers it can run on the main
/// thread, matched as the runtime matches: a subscription on the topic's
/// subject or a trailing-`**` wildcard over it, and, for a subject built
/// at run time, any main-thread subscription of the published payload's
/// type. A subscriber placed on another pool is a cross-thread enqueue
/// main does not wait for.
#[test]
fn a_publish_before_the_listener_runs_its_main_thread_handlers() {
    let program = |subscriber: &str, publisher_birth: &str, placement: &str| {
        let src = server(SERVED, "s: Spin = Spin { }; p: Pub = Pub { }; server: Server = Server { };").replace(
            "fn main()",
            &format!(
                "type Ping {{ id: Int = 0; }}\ntopic Pings {{ payload: Ping; subject: \"t.pings\"; }}\n{subscriber}\n\
                 locus Pub {{\n    bus {{ publish Pings; publish \"t.**\" of type Ping; }}\n    birth() {{ {publisher_birth} }}\n}}\nfn main()"
            ),
        );
        if placement.is_empty() {
            src
        } else {
            src.replacen("    run() { }\n", &format!("    placement {{ {placement} }}\n    run() {{ }}\n"), 1)
        }
    };
    let refused = |src: &str, at: &str| {
        let errs = errors(src);
        assert!(
            errs.iter().any(|e| e.contains("`Pub`'s `birth()`") && e.contains(at) && e.contains("may not return")),
            "refused at {at}: {errs:#?}\n{src}"
        );
    };
    let admitted = |src: &str| assert_eq!(errors(src), Vec::<String>::new(), "{src}");
    const WILD_SPINS: &str =
        "locus Spin {\n    bus { subscribe \"t.**\" as on_any of type Ping; }\n    fn on_any(p: Ping) { while !self.draining { } }\n}";
    const WILD_RETURNS: &str =
        "locus Spin {\n    bus { subscribe \"t.**\" as on_any of type Ping; }\n    fn on_any(p: Ping) { println(\"any\"); }\n}";
    const EXACT_SPINS: &str =
        "locus Spin {\n    bus { subscribe Pings as on_ping; }\n    fn on_ping(p: Ping) { while !self.draining { } }\n}";
    const EXACT_SLEEPS: &str =
        "locus Spin {\n    bus { subscribe Pings as on_ping; }\n    fn on_ping(p: Ping) { std::time::sleep(2s); }\n}";
    const PUBLISH: &str = "Pings <- Ping { id: 1 };";
    const COMPUTED: &str = "let subj = \"t.\" + \"pings\"; subj <- Ping { id: 1 };";
    // a wildcard subscription on main reaches the topic's publish
    refused(&program(WILD_SPINS, PUBLISH, ""), "`Spin::on_any`, a handler on the main thread of `t.pings`");
    // a subject built at run time reaches every main-thread handler of its payload
    refused(&program(EXACT_SPINS, COMPUTED, ""), "a handler on the main thread a subject built at run time may reach");
    // the same wildcard subscriber returning: admitted
    admitted(&program(WILD_RETURNS, PUBLISH, ""));
    admitted(&program(WILD_RETURNS, COMPUTED, ""));
    // the subscriber on another pool: its handler runs there, main does not wait
    admitted(&program(EXACT_SLEEPS, PUBLISH, "s: cooperative(pool = io);"));
    admitted(&program(EXACT_SPINS, PUBLISH, "s: cooperative(pool = io);"));
    // ... and on main it is refused
    refused(&program(EXACT_SLEEPS, PUBLISH, ""), "`Spin::on_ping`");
}

/// A subscriber (and its publisher) an imported seed declares: its handler
/// on the main thread runs inside the window like the entry's own, judged
/// under the name the merged program gives it.
#[test]
fn an_imported_main_thread_handler_is_judged() {
    let run = |tag: &str, lib: &str, params: &str, entry_decls: &str| -> Vec<String> {
        let d = std::env::temp_dir().join(format!("hale_listener_window_imported_handler_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("lib")).unwrap();
        std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
        std::fs::write(d.join("lib/hale.toml"), "[deps]\n").unwrap();
        std::fs::write(d.join("lib/main.hl"), lib).unwrap();
        let entry = format!(
            "import \"lib\" as lib;\n{}",
            server(SERVED, params).replace("fn main()", &format!("{entry_decls}\nfn main()"))
        );
        std::fs::write(d.join("main.hl"), entry).unwrap();
        errors_of_seed(&d)
    };
    const TOPIC: &str = "type Ping { id: Int = 0; }\ntopic Pings { payload: Ping; subject: \"t.pings\"; }\n";
    let spin = |body: &str| format!("locus Spin {{\n    bus {{ subscribe Pings as on_ping; }}\n    fn on_ping(p: Ping) {{ {body} }}\n}}\n");
    const PUB: &str = "locus Pub {\n    bus { publish Pings; }\n    birth() { Pings <- Ping { id: 1 }; }\n}\n";
    let refused = |errs: &[String]| {
        errs.iter().any(|e| e.contains("a handler on the main thread of `t.pings`") && e.contains("may not return"))
    };
    // subscriber and publisher both imported
    let errs = run(
        "both",
        &format!("{TOPIC}{}{PUB}", spin("while !self.draining { }")),
        "s: lib::Spin = lib::Spin { }; p: lib::Pub = lib::Pub { }; server: Server = Server { };",
        "",
    );
    assert!(refused(&errs), "{errs:#?}");
    // only the subscriber imported: the entry publishes the library's topic
    let errs = run(
        "sub",
        &format!("{TOPIC}{}", spin("while !self.draining { }")),
        "s: lib::Spin = lib::Spin { }; p: Pub = Pub { }; server: Server = Server { };",
        "locus Pub {\n    bus { publish lib::Pings; }\n    birth() { lib::Pings <- lib::Ping { id: 1 }; }\n}",
    );
    assert!(refused(&errs), "{errs:#?}");
    // only the publisher imported: the entry subscribes to the library's topic
    let errs = run(
        "pub",
        &format!("{TOPIC}{PUB}"),
        "s: Spin = Spin { }; p: lib::Pub = lib::Pub { }; server: Server = Server { };",
        "locus Spin {\n    bus { subscribe lib::Pings as on_ping; }\n    fn on_ping(p: lib::Ping) { while !self.draining { } }\n}",
    );
    assert!(refused(&errs), "{errs:#?}");
    // imported, the handler returns: admitted
    let errs = run(
        "returns",
        &format!("{TOPIC}{}{PUB}", spin("println(\"ping\");")),
        "s: lib::Spin = lib::Spin { }; p: lib::Pub = lib::Pub { }; server: Server = Server { };",
        "",
    );
    assert_eq!(errs, Vec::<String>::new());
}
