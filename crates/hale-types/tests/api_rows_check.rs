//! GH #1417 (R1): the admission law over the surface rows
//! (spec/api.md § Surfaces and their rows), one refusal per law in the
//! contract's wording, law 6's statement of a row, and the R0 fixture
//! program (`tests/api-contract/program.hl`) accepted. The serve sites'
//! laws are R2's: a serve site is named, not checked.

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;

/// Declarations every case shares: two roles, a request and a response,
/// and the handlers the rows name.
const BASE: &str = "
role trader;
role operator;

type CancelOrder { order: Int; }
type Cancelled { order: Int; was_open: Bool; }
type Export { id: Int; raw: Bytes; }

locus Orders {
    params { open: Int = 0; }
    closure stale { captures: open; epoch inline; }
    fn place(c: CancelOrder) -> Cancelled { return Cancelled { order: c.order, was_open: true }; }
    fn cancel(c: CancelOrder) -> Cancelled { return Cancelled { order: c.order, was_open: false }; }
    fn fill(a: CancelOrder, b: CancelOrder) -> Cancelled { return Cancelled { order: a.order + b.order, was_open: true }; }
    fn flush() { if self.open > 3 { violate stale; } }
    run() { }
}

locus Ledger {
    fn dump(c: CancelOrder) -> Export { return Export { id: c.order, raw: bytes(\"\") }; }
    fn load(b: Bytes) -> Int { return len(b); }
}

fn helper() -> Int { return 1; }

fn main() { }
";

/// The surface-law errors a program draws: each `rpc` / `@rpc` message.
fn row_errors(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).unwrap_or_else(|d| panic!("parse: {:?}", d[0].message));
    let snap = Snapshot::from_program(program, Vec::new(), Config::check(false, false)).ok().expect("a snapshot");
    let diags = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    diags
        .iter()
        .filter(|d| d.is_error() && (d.message.starts_with("rpc `") || d.message.starts_with("`@rpc`")))
        .map(|d| d.message.clone())
        .collect()
}

fn with(api: &str) -> Vec<String> {
    row_errors(&format!("{BASE}\n{api}"))
}

#[test]
fn law_1_a_row_names_a_handler() {
    assert_eq!(
        with("api Public { rpc Orders::plcae; }"),
        vec!["rpc `Orders::plcae`: `Orders` declares no fn `plcae`; did you mean `place`?"]
    );
    assert_eq!(
        with("api Public { rpc Ordrs::place; }"),
        vec!["rpc `Ordrs::place`: no locus `Ordrs` is declared; did you mean `Orders`?"]
    );
    assert_eq!(
        with("api Public { rpc Orders::run; }"),
        vec!["rpc `Orders::run`: `run` is a lifecycle method of `Orders`, and a handler is a member fn"]
    );
    let free = row_errors(&BASE.replace("fn helper()", "@rpc\nfn helper()"));
    assert_eq!(free, vec!["`@rpc` on `helper`: a handler is a member fn of a locus, and `helper` is a free fn"]);
}

#[test]
fn law_2_a_handler_takes_one_request() {
    assert_eq!(
        with("api Public { rpc Orders::fill; }"),
        vec![
            "rpc `Orders::fill`: a handler takes its request as one parameter, and `fill` takes two: declare a type \
             for the request"
        ]
    );
}

#[test]
fn law_3_a_member_is_one_row_of_its_surface() {
    assert_eq!(
        with("api Admin { rpc Orders::cancel requires: [operator]; rpc Orders::cancel requires: [operator]; }"),
        vec!["rpc `Orders::cancel` is in `Admin` twice: a member is one row; keep one, with the roles it requires"]
    );
    // One handler in two surfaces is two rows, each its surface's.
    assert!(with("api Public { rpc Orders::cancel requires: [trader]; }\napi Admin { rpc Orders::cancel requires: [operator]; }")
        .is_empty());
}

#[test]
fn law_4_a_required_role_is_declared() {
    assert_eq!(
        with("api Public { rpc Orders::cancel requires: [tradr]; }"),
        vec!["rpc `Orders::cancel` requires `tradr`, which no `role` declares; did you mean `trader`?"]
    );
}

#[test]
fn law_5_every_shape_has_a_codec_form() {
    assert_eq!(
        with("api Public { rpc Ledger::dump; }"),
        vec!["rpc `Ledger::dump`: its response `Export` has a field `raw: Bytes`, which the JSON codec does not carry"]
    );
    assert_eq!(
        with("api Public { rpc Ledger::load; }"),
        vec!["rpc `Ledger::load`: its request is `Bytes`, which the JSON codec does not carry"]
    );
}

/// Law 6 is a statement, not a refusal: the note `hale check --api`
/// prints beside a member whose error type is `ClosureViolation`.
#[test]
fn law_6_the_error_type_decides_the_failure() {
    let snap = fixture();
    let rows = snap.demand_surface_rows().expect("rows");
    let notes: Vec<(String, String, String)> = hale_types::surfaces::server_error_notes(rows);
    assert_eq!(
        notes,
        vec![
            (
                "Admin".to_string(),
                "Ledger::rebalance".to_string(),
                "rpc `Ledger::rebalance`: its error type is `ClosureViolation`, so a failure is the server error; a \
                 description carries no error schema for it"
                    .to_string()
            ),
            (
                "Public".to_string(),
                "Orders::place".to_string(),
                "rpc `Orders::place`: its error type is `ClosureViolation`, so a failure is the server error; a \
                 description carries no error schema for it"
                    .to_string()
            ),
        ]
    );
}

#[test]
fn law_7_a_handler_that_may_violate_is_fallible_closure_violation() {
    assert_eq!(
        with("api Admin { rpc Orders::flush requires: [operator]; }"),
        vec![
            "rpc `Orders::flush` may violate (`violate stale`) and returns nothing: an rpc handler that may violate \
             is `fallible(ClosureViolation)`, so its caller receives the server error instead of a result"
        ]
    );
    // Declared, it is the server error, and admitted.
    let declared = BASE.replace("fn flush() {", "fn flush() fallible(ClosureViolation) {");
    assert!(row_errors(&format!("{declared}\napi Admin {{ rpc Orders::flush requires: [operator]; }}")).is_empty());
}

fn fixture() -> Snapshot {
    let path = hale_corpus::repo_root().join("tests/api-contract/program.hl");
    Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(false, false)).ok().expect("the fixture loads")
}

/// The R0 fixture program is admitted: no error at all, the surface
/// laws' among them.
#[test]
fn the_fixture_program_is_admitted() {
    let snap = fixture();
    let checked = snap.demand_check().ok().expect("the fixture checks");
    let errors: Vec<&str> = checked.diags.iter().filter(|d| d.is_error()).map(|d| d.message.as_str()).collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// T1 (friction sweep 2026-10-12): a role a library seed declares is named
/// through its import alias. `requires: [lib::reader]` on a surface row,
/// an `@rpc` attribute and a hub's stream, and `includes lib::reader` on a
/// role, all reach the library's role (roles are bundle-global vocabulary,
/// so the alias collapses to the role's own name); the hub's stream may
/// carry the library's topic as well. An alias the seed does not import,
/// or a role the library does not declare, is left as written and refused
/// by law 4 in the author's spelling.
fn seed_with_role_library(tag: &str, entry: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("hale_roles_imported_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/hale.toml"), "[deps]\n").unwrap();
    std::fs::write(
        d.join("lib/main.hl"),
        "role reader;
role writer includes reader;
type Tick { n: Int; }
topic Ticks { payload: Tick; subject: \"ticks\"; }
locus Store {
    bus { publish Ticks; }
    fn tick() { Ticks <- Tick { n: 1 }; }
}
locus Toks {
    fn principal(token: String) -> std::api::Principal { return std::api::Principal { mode: \"bearer\", name: token }; }
    fn refused() -> String { return \"no\"; }
}
locus Grants {
    fn holds(p: std::api::Principal, r: String) -> Bool { return true; }
}
",
    )
    .unwrap();
    std::fs::write(d.join("main.hl"), entry).unwrap();
    d
}

fn role_errors_of_seed(dir: &std::path::Path) -> Vec<String> {
    let snap = Snapshot::load(dir, LoadMode::WholeSeed, &Disk, Config::check(false, false)).ok().expect("it loads");
    let diags = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    let out = diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    let _ = std::fs::remove_dir_all(dir);
    out
}

const ROLE_APP: &str = "import \"lib\" as lib;
role boss includes lib::writer;
type Q { n: Int; }
type A { n: Int; }
locus Desk {
    fn look(q: Q) -> A { return A { n: q.n }; }
    @rpc(requires: [lib::writer])
    fn write(q: Q) -> A { return A { n: q.n }; }
}
api Public { rpc Desk::look requires: [lib::reader, boss]; }
main locus App {
    params {
        toks: lib::Toks = lib::Toks { };
        grants: lib::Grants = lib::Grants { };
        store: lib::Store = lib::Store { };
        desk: Desk = Desk { };
        hub: ws::Hub = ws::Hub { bind: \"127.0.0.1:0\", principals: self.toks, roles: self.grants, as: \"stream\" };
    }
    bindings {
        lib::Ticks: self.hub requires: [lib::reader], bound: 8, on_full: drop_old;
    }
    run() { self.hub.stop(); }
}
fn main() { App { }; }
";

#[test]
fn a_role_another_seed_declares_is_named_through_its_alias() {
    let ok = seed_with_role_library("ok", ROLE_APP);
    assert_eq!(role_errors_of_seed(&ok), Vec::<String>::new());
}

#[test]
fn a_qualified_role_nothing_declares_is_refused_as_written() {
    let missing = seed_with_role_library(
        "missing",
        &ROLE_APP.replace("requires: [lib::reader, boss]", "requires: [lib::auditor, boss]"),
    );
    let errors = role_errors_of_seed(&missing);
    assert_eq!(
        errors,
        vec!["rpc `Desk::look` requires `lib::auditor`, which no `role` declares".to_string()],
        "{errors:#?}"
    );
    let unknown_alias = seed_with_role_library(
        "alias",
        &ROLE_APP.replace("requires: [lib::reader, boss]", "requires: [other::reader, boss]"),
    );
    let errors = role_errors_of_seed(&unknown_alias);
    assert!(
        errors.iter().any(|e| e.contains("requires `other::reader`, which no `role` declares")),
        "{errors:#?}"
    );
}
