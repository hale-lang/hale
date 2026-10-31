//! GH #1417 (R5): the laws of a hub binding (spec/api.md § Streams), one
//! refusal per law in the contract's wording, a binding the check admits
//! (and a build too: the expansion serves it), and the R0 witness's hub
//! still described as its fixtures have it.
//!
//! A stream row states its queue (`bound:`) and a policy a watcher queue
//! has (`drop_old` or `drop_new`, never `refuse`: a subscriber that cannot
//! keep up loses events, not the subscription); it carries a topic some
//! locus publishes; a hub is a param built by a `ws::Hub` literal that
//! states its address and names its exposure, once among the program's.

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::Disk;

/// A program with one topic, a publisher, two role sources and a main
/// locus whose `params` and `bindings` are the case's.
fn program(params: &str, bindings: &str) -> String {
    format!(
        "
role operator;

type Fill {{ qty: Int; }}
topic Fills {{ payload: Fill; subject: \"desk.fills\"; }}
topic Quotes {{ payload: Fill; subject: \"desk.quotes\"; }}

locus Tokens {{
    fn principal(token: String) -> std::api::Principal {{
        return std::api::Principal {{ mode: \"bearer\", name: token }};
    }}
    fn refused() -> String {{ return \"no\"; }}
}}

locus Grants {{
    fn holds(p: std::api::Principal, r: String) -> Bool {{ return p.name == \"dave\"; }}
}}

locus Maker {{
    bus {{ publish Fills; }}
    fn make() {{ Fills <- Fill {{ qty: 1 }}; }}
}}

main locus Desk {{
    params {{
        bearer: Tokens = Tokens {{ }};
        roles: Grants = Grants {{ }};
        maker: Maker = Maker {{ }};
        {params}
    }}
    bindings {{
        {bindings}
    }}
    run() {{ self.maker.make(); }}
}}

fn main() {{ Desk {{ }}; }}
"
    )
}

const HUB: &str = "hub: ws::Hub = ws::Hub { bind: \"127.0.0.1:9000\", principals: self.bearer, roles: self.roles, as: \"fills\" };";
const ROW: &str = "Fills: self.hub requires: [operator], bound: 8, on_full: drop_old;";

/// The errors a program draws from the check, and then from a build's own
/// rules: every message, in the order reported.
fn errors(src: &str) -> Vec<String> {
    let parsed = hale_syntax::parse_source(src).unwrap_or_else(|d| panic!("parse: {:?}", d[0].message));
    let snap = Snapshot::from_program(parsed, Vec::new(), Config::check(false, false)).ok().expect("a snapshot");
    let diags = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

/// What a build refuses (`hale_types::build_rule_diags`).
fn build_errors(src: &str) -> Vec<String> {
    let parsed = hale_syntax::parse_source(src).unwrap_or_else(|d| panic!("parse: {:?}", d[0].message));
    let snap = Snapshot::from_program(parsed, Vec::new(), Config::build(Target::host())).ok().expect("a snapshot");
    let diags = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

#[test]
fn a_well_formed_hub_binding_is_admitted_by_the_check_and_the_build() {
    let src = program(HUB, ROW);
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
    assert!(build_errors(&src).is_empty(), "{:?}", build_errors(&src));
    // a policy the other watcher queue has
    let src = program(HUB, "Fills: self.hub requires: [operator], bound: 8, on_full: drop_new;");
    assert!(build_errors(&src).is_empty(), "{:?}", build_errors(&src));
    // no `requires:` is a stream anyone the sources name may read
    let src = program(HUB, "Fills: self.hub requires: [], bound: 8, on_full: drop_new;");
    assert!(build_errors(&src).is_empty(), "{:?}", build_errors(&src));
}

#[test]
fn a_stream_row_states_its_queue() {
    let src = program(HUB, "Fills: self.hub requires: [operator], on_full: drop_old;");
    let e = errors(&src);
    assert!(
        e.iter().any(|m| m.contains("stream `Fills` of hub `self.hub`: a stream row states `bound:`")),
        "{e:?}"
    );
}

#[test]
fn a_stream_is_never_refused() {
    let src = program(HUB, "Fills: self.hub requires: [operator], bound: 8, on_full: refuse;");
    let e = errors(&src);
    assert!(
        e.iter().any(|m| m.contains("stream `Fills` of hub `self.hub`: a stream row states `on_full: drop_old` or `drop_new`")
            && m.contains("so a stream is not `refuse`d")),
        "{e:?}"
    );
    let src = program(HUB, "Fills: self.hub requires: [operator], bound: 8;");
    assert!(errors(&src).iter().any(|m| m.contains("a stream row states `on_full: drop_old` or `drop_new`")));
}

#[test]
fn a_stream_carries_a_topic_some_locus_publishes() {
    let src = program(HUB, "Quotes: self.hub requires: [operator], bound: 8, on_full: drop_old;");
    let e = errors(&src);
    assert!(
        e.iter().any(|m| m.contains("stream `Quotes` of hub `self.hub`: no locus publishes `Quotes`")),
        "{e:?}"
    );
}

#[test]
fn a_hub_names_its_exposure_and_its_address() {
    let src = program(
        "hub: ws::Hub = ws::Hub { bind: \"127.0.0.1:9000\", principals: self.bearer, roles: self.roles };",
        ROW,
    );
    assert!(errors(&src).iter().any(|m| m.contains("hub `self.hub`: a hub names its exposure with `as:`")));
    let src = program("hub: ws::Hub = ws::Hub { principals: self.bearer, roles: self.roles, as: \"fills\" };", ROW);
    assert!(errors(&src).iter().any(|m| m.contains("hub `self.hub`: a hub states the address it listens on")));
}

#[test]
fn a_hub_is_a_param_built_by_a_literal() {
    let src = program("", ROW);
    let e = errors(&src);
    assert!(e.iter().any(|m| m.contains("hub `self.hub`: a hub is a param built by a `ws::Hub")), "{e:?}");
}

#[test]
fn an_exposure_is_named_once_among_hubs_and_serve_sites() {
    let params = "hub: ws::Hub = ws::Hub { bind: \"127.0.0.1:9000\", principals: self.bearer, roles: self.roles, as: \"fills\" }; \
                  other: ws::Hub = ws::Hub { bind: \"127.0.0.1:9001\", principals: self.bearer, roles: self.roles, as: \"fills\" };";
    let src = program(params, &format!("{ROW}\n        Quotes: self.other requires: [operator], bound: 4, on_full: drop_old;"));
    let e = errors(&src);
    assert!(
        e.iter().any(|m| m.contains("exposure `fills` is served twice: `as:` names one exposure; name this one apart")),
        "{e:?}"
    );
}

const UDP: &str = "hub: udp::Hub = udp::Hub { bind: \"127.0.0.1:9000\", principals: self.bearer, roles: self.roles, as: \"fills\" };";

#[test]
fn a_udp_hub_is_a_hub_and_carries_streams_only() {
    let src = program(UDP, ROW);
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
    assert!(build_errors(&src).is_empty(), "{:?}", build_errors(&src));
    // requests and replies over datagrams are not framed yet: a surface is not served over one
    let src = program(UDP, ROW).replace(
        "run() { self.maker.make(); }",
        "run() { self.maker.make(); self.s.stop(); }",
    )
    .replace("main locus Desk", "type Req { n: Int; }\ntype Res { n: Int; }\nlocus Svc { fn ask(r: Req) -> Res { return Res { n: r.n }; } }\napi Public { rpc Svc::ask; }\n\nmain locus Desk")
    .replace("maker: Maker = Maker { };", "maker: Maker = Maker { };\n        svc: Svc = Svc { };")
    .replace(
        "as: \"fills\" };",
        "as: \"fills\" };\n        s: api::Exposure = api::serve(Public, self.hub, as: \"desk\", receivers: { Svc: self.svc }, bound: 4, on_full: refuse);",
    );
    let e = errors(&src);
    assert!(
        e.iter().any(|m| m.contains("serve of `Public` over a `udp::Hub`: a request and its reply over datagrams are not framed")),
        "{e:?}"
    );
}

#[test]
fn a_construction_cannot_supply_the_hub_a_stream_is_bound_to() {
    // the default is numbered and given its rows; a hub supplied at a
    // construction would be neither, and the listener would wait on a key
    // the hub does not answer on (downstream handoff review of R5)
    let src = program(HUB, ROW).replace(
        "fn main() { Desk { }; }",
        "fn main() { Desk { hub: ws::Hub { bind: \"127.0.0.1:9001\", principals: Tokens { }, roles: Grants { }, name: \"fills\" } }; }",
    );
    let e = errors(&src);
    assert!(
        e.iter().any(|m| m.contains("locus `Desk`: param `hub` is a hub that streams are bound to; its default is the hub, and a construction cannot supply another")),
        "{e:?}"
    );
    assert!(errors(&program(HUB, ROW)).is_empty());
}

// ---- a main locus the program imports (GH #1781) ----

/// A seed whose library `lib` is the hub program with a plain fn beside its
/// main locus, and an entry program `entry` (`import "lib" as lib;`).
fn seed_with_hub_library(tag: &str, entry: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("hale_hub_imported_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/hale.toml"), "[deps]\n").unwrap();
    let lib = format!("{}\nfn price_of(n: Int) -> Int {{ return n * 10; }}\n", program(HUB, ROW));
    std::fs::write(d.join("lib/main.hl"), lib).unwrap();
    std::fs::write(d.join("main.hl"), entry).unwrap();
    d
}

/// The errors the seed at `dir` draws from the check (`build` false) or a
/// build's rules, with the library's main locus written as the program
/// spells it, as the CLI writes it.
fn errors_of_seed(dir: &std::path::Path, build: bool) -> Vec<String> {
    let config = if build { Config::build(Target::host()) } else { Config::check(false, false) };
    let snap = Snapshot::load(dir, LoadMode::WholeSeed, &Disk, config).ok().expect("it loads");
    let diags = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    diags.iter().filter(|d| d.is_error()).map(|d| d.message.replace("__lib_lib___main__Desk", "lib::Desk")).collect()
}

#[test]
fn a_seed_whose_main_locus_holds_a_hub_can_be_imported() {
    // a program's tests import its seed for its fns: the library's stream
    // rows name its topics, which the import renames with the rest of the
    // library, and its hub expands as in the library's own build (it never
    // runs: nothing constructs an imported main locus)
    let d = seed_with_hub_library("fns", "import \"lib\" as lib;\nfn main() { println(lib::price_of(3)); }\n");
    assert_eq!(errors_of_seed(&d, false), Vec::<String>::new());
    assert_eq!(errors_of_seed(&d, true), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&d);
}

const IMPORTED_MAIN_HUB: &str = "hub `self.hub` in `lib::Desk`, a main locus the program imports: only the entry program's main locus binds streams; hold the hub in the entry's main locus and bind the streams there (spec/api.md § Streams)";

#[test]
fn a_hub_in_a_main_locus_the_program_builds_but_imports_is_refused() {
    // only the entry's `bindings { }` bind, so the hub would listen and its
    // streams carry nothing
    let as_a_child = seed_with_hub_library(
        "child",
        "import \"lib\" as lib;\nmain locus App { params { inner: lib::Desk = lib::Desk { }; } run() { } }\nfn main() { App { }; }\n",
    );
    assert_eq!(errors_of_seed(&as_a_child, false), vec![IMPORTED_MAIN_HUB.to_string()]);
    let _ = std::fs::remove_dir_all(&as_a_child);
    let as_the_program = seed_with_hub_library("program", "import \"lib\" as lib;\nfn main() { lib::Desk { }; }\n");
    assert_eq!(errors_of_seed(&as_the_program, false), vec![IMPORTED_MAIN_HUB.to_string()]);
    let _ = std::fs::remove_dir_all(&as_the_program);
}
