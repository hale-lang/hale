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

/// A server whose `run()` runs on the main thread while the main locus is
/// built (the param that holds it is unplaced) holds back every later param
/// of the main locus, its listener among them when the listener reads its
/// address off the server. A run the blocking analysis says may not return
/// (a wait on the exposure, directly or through a fn it calls; a `while`
/// loop, which the classes cannot judge) is refused, located; one it proves
/// returns is admitted, and so is any run of a placed server or of one
/// whose address reads nothing of it.
#[test]
fn an_unplaced_servers_run_cannot_wait_ahead_of_a_listener_born_after_it() {
    let with_run = |members: &str| {
        server(SERVED, "server: Server = Server { };")
            .replace("\n    }\n}\nmain locus App", &format!("\n    }}\n{members}\n}}\nmain locus App"))
    };
    let refused = |src: &str, why: &str| {
        let errs = errors(src);
        assert!(
            errs.iter().any(|e| e.contains("exposure `web` held by `Server`: its address reads `Server`, so its listener (`App`'s) is born after `self.server`, and `Server`'s `run()` runs on the main thread")
                && e.contains(why)),
            "{why}: {errs:#?}"
        );
    };
    let waiting = with_run("    run() { self.web.wait(); }");
    refused(&waiting, "may not return (it may block");
    // through a fn of its own: the analysis is interprocedural
    refused(&with_run("    fn hold() { self.web.wait(); }\n    run() { self.hold(); }"), "may not return (it may block");
    // a loop that spins without blocking: the classes cannot judge it
    refused(&with_run("    run() { while !self.draining { } }"), "a `while` loop");
    // a run that returns
    assert_eq!(errors(&with_run("    run() { println(\"started\"); }")), Vec::<String>::new());
    let placed = waiting.replace("    run() { }\n}\nfn main", "    placement { server: cooperative(pool = io) where async_io; }\n    run() { }\n}\nfn main");
    assert_eq!(errors(&placed), Vec::<String>::new());
    let constant = waiting.replace("to_string(self.port)", "\"8080\"");
    assert_eq!(errors(&constant), Vec::<String>::new());
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
