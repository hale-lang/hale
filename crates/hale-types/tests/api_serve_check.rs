//! GH #1417 (R2a): the laws of a serve site (spec/api.md § Serving), one
//! refusal per law in the contract's wording, a site the check admits,
//! and the R0 witness (`tests/api-contract/program.hl`) still accepted:
//! the check reads its serve sites over transports this compiler does
//! not ship (`http::Rpc`, `unix::Rpc`) as it read them in R1, and a
//! build refuses those (`api_serve_build.rs`).

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;

/// Declarations every case shares: a role, the rows' types, two receiver
/// loci and two surfaces, and a serving locus whose `run()` is `body`.
fn program(params: &str, body: &str) -> String {
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
    assert!(errors(&program(ONE, &format!("let public = {SERVE}; public.stop();"))).is_empty());
    // inferred: the serving locus holds exactly one instance of the type
    assert!(errors(&program(
        ONE,
        "let public = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse); public.stop();"
    ))
    .is_empty());
    // a surface served twice, to two instances, under two names
    assert!(errors(&program(
        TWO,
        "let a = api::serve(Public, self.fixture, as: \"a\", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);
         let b = api::serve(Public, self.fixture, as: \"b\", receivers: { Orders: self.partner_orders }, bound: 8, on_full: refuse);
         a.stop(); b.stop();"
    ))
    .is_empty());
}

#[test]
fn law_1_an_exposure_is_named_once() {
    let src = program(
        ONE,
        "let a = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse);
         let b = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse);",
    );
    assert_eq!(errors(&src), vec!["exposure `public` is served twice: `as:` names one exposure; name this one apart"]);
}

#[test]
fn law_2_every_receiver_type_is_bound() {
    // two instances, neither bound
    assert_eq!(
        errors(&program(
            TWO,
            "let p = api::serve(Public, self.fixture, as: \"partner\", bound: 8, on_full: refuse);"
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
            "let b = api::serve(Both, self.fixture, as: \"both\", bound: 8, on_full: refuse);"
        )),
        vec![
            "serve of `Both` as `both`: `Ledger` is held by no param of `Desk`, and the serve binds none: bind the \
             instance that answers (`receivers: { Ledger: self.<param> }`)"
        ]
    );
}

#[test]
fn law_3_a_receiver_outlives_its_exposure() {
    assert_eq!(
        errors(&program(
            ONE,
            "let ledger = Ledger { };
             let admin = api::serve(Both, self.fixture, as: \"admin\", receivers: { Ledger: ledger }, bound: 8, on_full: refuse);
             admin.stop();"
        )),
        vec![
            "serve of `Both`: `ledger` is `let`-bound and dissolves at the end of this block, before `admin.stop()`; \
             hold it as a param"
        ]
    );
    // a param the serving locus holds, but built by something other than a
    // literal, cannot carry the number the serve gives its instance
    assert_eq!(
        errors(&program(
            "orders: Orders = make_orders();",
            "let public = api::serve(Public, self.fixture, as: \"public\", bound: 8, on_full: refuse);"
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
            "let public = api::serve(Public, self.fixture, as: \"public\", receivers: { Orders: self.orders, Ledger: self.ledger }, bound: 8, on_full: refuse);"
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
        assert_eq!(errors(&program(ONE, &format!("let public = {site};"))), vec![want], "{site}");
    }
}

#[test]
fn a_site_needs_a_declared_surface_and_a_name() {
    assert_eq!(
        errors(&program(ONE, "let p = api::serve(Pubic, self.fixture, as: \"p\", bound: 8, on_full: refuse);")),
        vec!["serve of `Pubic`: no surface `Pubic` is declared; did you mean `Public`?"]
    );
    assert_eq!(
        errors(&program(ONE, "let p = api::serve(Public, self.fixture, bound: 8, on_full: refuse);")),
        vec!["serve of `Public`: a serve site names its exposure with `as:`"]
    );
    let free = program(ONE, "").replace(
        "fn main()",
        "fn serve_it() { let p = api::serve(Public, std::api::test::Rpc { }, as: \"p\", bound: 8, on_full: refuse); }\nfn main()",
    );
    assert_eq!(
        errors(&free),
        vec![
            "`api::serve` in `serve_it`: a serve site belongs to a locus's body, since its exposure is a param of the \
             serving locus; serve from the locus that holds the receivers"
        ]
    );
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
