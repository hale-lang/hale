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

use hale_frontend::snapshot::{Config, Snapshot, Target};

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
        "run() { self.maker.make(); let s = api::serve(Public, self.hub, as: \"desk\", receivers: { Maker: self.maker }, bound: 4, on_full: refuse); s.stop(); }",
    )
    .replace("main locus Desk", "type Req { n: Int; }\ntype Res { n: Int; }\nlocus Svc { fn ask(r: Req) -> Res { return Res { n: r.n }; } }\napi Public { rpc Svc::ask; }\n\nmain locus Desk")
    .replace("receivers: { Maker: self.maker }", "receivers: { Svc: self.svc }")
    .replace("maker: Maker = Maker { };", "maker: Maker = Maker { };\n        svc: Svc = Svc { };");
    let e = errors(&src);
    assert!(
        e.iter().any(|m| m.contains("serve of `Public` over a `udp::Hub`: a request and its reply over datagrams are not framed")),
        "{e:?}"
    );
}
