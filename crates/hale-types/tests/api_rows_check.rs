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

/// A list crosses as a field of a record: `bounded[T; N]` and `[T; N]` of
/// what a field carries are admitted; an unbounded `[T]`, a list of lists, a
/// list of what the codec does not carry, and a request or response that is
/// itself a list are refused, each named (spec/api.md § Codecs).
#[test]
fn law_5_a_list_is_carried_as_a_field_of_a_record() {
    let lists = "
type Item { id: Int; name: String; }
type Page { items: bounded[Item; 8]; ids: [Int; 3]; }
type Loose { items: [Item]; }
type Nested { grid: bounded[[Int; 2]; 4]; }
type Blobs { raws: bounded[Bytes; 2]; }
locus Shelf {
    fn page(p: Page) -> Page { return p; }
    fn loose(p: Page) -> Loose { return Loose { items: [] }; }
    fn nested(p: Page) -> Nested { return Nested { }; }
    fn blobs(p: Page) -> Blobs { return Blobs { }; }
    fn bare(p: Page) -> [Int; 3] { return p.ids; }
}
";
    let rows = |api: &str| row_errors(&format!("{BASE}\n{lists}\n{api}"));
    assert!(rows("api Shelves { rpc Shelf::page; }").is_empty(), "a bounded list and a fixed array are carried");
    assert_eq!(
        rows("api Shelves { rpc Shelf::loose; }"),
        vec!["rpc `Shelf::loose`: its response `Loose` has a field `items: [Item]`, which the JSON codec does not carry"]
    );
    assert_eq!(
        rows("api Shelves { rpc Shelf::nested; }"),
        vec![
            "rpc `Shelf::nested`: its response `Nested` has a field `grid: bounded[[Int; 2]; 4]`, which the JSON codec \
             does not carry"
        ]
    );
    assert_eq!(
        rows("api Shelves { rpc Shelf::blobs; }"),
        vec!["rpc `Shelf::blobs`: its response `Blobs` has a field `raws: bounded[Bytes; 2]`, which the JSON codec does not carry"]
    );
    assert_eq!(
        rows("api Shelves { rpc Shelf::bare; }"),
        vec![
            "rpc `Shelf::bare`: its response is `[Int; 3]`, a list, which crosses as a field of a record: declare one \
             (`type Page { items: [Int; 3]; }`)"
        ]
    );
}

/// A `Time` and a `Duration` are carried, as a field, as a list's element
/// and as a whole request or response (spec/api.md § Codecs).
#[test]
fn law_5_an_instant_and_a_duration_are_carried() {
    let times = "
type Due = Time;
type Stamp { at: Time; every: Duration; due: Due; seen: [Time; 2]; }
locus Clock {
    fn stamp(s: Stamp) -> Stamp { return s; }
    fn now(d: Duration) -> Time { return std::time::current() + d; }
}
";
    assert!(row_errors(&format!("{BASE}\n{times}\napi Clocks {{ rpc Clock::stamp; rpc Clock::now; }}")).is_empty());
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
/// carry the library's topic as well. The library also declares a fn named
/// `reader`: the role is looked up in the roles' own namespace, so the fn
/// keeps its rename and `lib::reader()` still calls it. An alias the seed does not import,
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
// a fn of a role's name: the role is named in a namespace of its own
fn reader() -> Int { return 7; }
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
    run() {
        let seven = lib::reader();
        println(seven);
        self.hub.stop();
    }
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

/// An enum is carried, as a field, a payload-free one as a list's element,
/// and as a whole request or response; a variant's value that is a list, or
/// an enum with payloads of its own, is refused named by the variant, and so
/// is a list of an enum with payloads (spec/api.md § Codecs).
#[test]
fn law_5_an_enum_is_carried() {
    let enums = "
type Inner { a: Int; }
type Status = enum { Up, Down };
type Shape = enum { Empty, Circle(Float), Rect(Int, Int), Boxed(Inner), Mood(Status), At(Time) };
type Holder { s: Status; shape: Shape; many: bounded[Status; 4]; }
type Listy = enum { Many(bounded[Int; 3]) };
type Deep = enum { Inside(Shape) };
type Shapes { all: bounded[Shape; 2]; }
locus Board {
    fn hold(h: Holder) -> Holder { return h; }
    fn whole(s: Shape) -> Status { return Status::Up; }
    fn listy(h: Holder) -> Listy { return Listy::Many([]); }
    fn deep(h: Holder) -> Deep { return Deep::Inside(Shape::Empty); }
    fn shapes(h: Holder) -> Shapes { return Shapes { }; }
}
";
    let rows = |api: &str| row_errors(&format!("{BASE}\n{enums}\n{api}"));
    assert!(rows("api Boards { rpc Board::hold; rpc Board::whole; }").is_empty(), "an enum is carried");
    assert_eq!(
        rows("api Boards { rpc Board::listy; }"),
        vec!["rpc `Board::listy`: its response `Listy` has a field `Many: bounded[Int; 3]`, which the JSON codec does not carry"]
    );
    assert_eq!(
        rows("api Boards { rpc Board::deep; }"),
        vec!["rpc `Board::deep`: its response `Deep` has a field `Inside: Shape`, which the JSON codec does not carry"]
    );
    assert_eq!(
        rows("api Boards { rpc Board::shapes; }"),
        vec!["rpc `Board::shapes`: its response `Shapes` has a field `all: bounded[Shape; 2]`, which the JSON codec does not carry"]
    );
}
