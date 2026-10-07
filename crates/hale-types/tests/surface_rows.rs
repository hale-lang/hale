//! GH #1417 (R1): the `surface` family's rows and digests. The R0
//! fixture program (`tests/api-contract/program.hl`) produces the rows
//! `tests/api-contract/digest.md` works by hand, and the model holds
//! them with the digests it folds; an `@rpc` handler and an `rpc` line
//! are one row; nothing else about a locus is read.

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_model::surface::digest_text;
use hale_types::surfaces::{Handled, SurfaceRows};

fn fixture() -> Snapshot {
    let path = hale_corpus::repo_root().join("tests/api-contract/program.hl");
    Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(false, false)).ok().expect("the fixture loads")
}

fn source(src: &str) -> Snapshot {
    let program = hale_syntax::parse_source(src).expect("parse");
    Snapshot::from_program(program, Vec::new(), Config::check(false, false)).ok().expect("a snapshot")
}

/// Each row as `member request response error requires`, the hashes as
/// sixteen hex digits or `-`.
fn lines(rows: &SurfaceRows, surface: &str) -> Vec<String> {
    let hex = |t: Option<&hale_types::surfaces::RowTy>| t.map_or_else(|| "-".to_string(), |t| format!("{:016x}", t.hash));
    rows.rows_of(surface)
        .map(|r| {
            let Handled::Fn(h) = &r.handler else { panic!("{} names a handler", r.member) };
            let req: Vec<&str> = r.requires.iter().map(|(n, _)| n.as_str()).collect();
            format!(
                "{} {} {} {} {}",
                r.member,
                hex(h.request.as_ref()),
                hex(h.response.as_ref()),
                hex(h.error.as_ref()),
                if req.is_empty() { "-".to_string() } else { req.join(",") }
            )
        })
        .collect()
}

#[test]
fn the_fixture_programs_surfaces_are_digest_mds_rows() {
    let snap = fixture();
    let rows = snap.demand_surface_rows().expect("rows");
    let names: Vec<(&str, String)> = rows.surfaces.iter().map(|s| (s.name.as_str(), digest_text(s.digest))).collect();
    assert_eq!(
        names,
        vec![("Admin", "fnv1a64:40381db6685c9f75".to_string()), ("Public", "fnv1a64:a8930d6e7998e986".to_string())]
    );
    assert_eq!(
        lines(rows, "Public"),
        vec![
            "Orders::cancel deb8489f34994e5a e1506381a35c8ced db0311924c0e7333 trader",
            "Orders::place cb5775974312c858 bb4f99639cf069af 36c7f0561125943e -",
        ]
    );
    assert_eq!(
        lines(rows, "Admin"),
        vec![
            "Ledger::rebalance 19611780fbd68ecf 3193bf68569ed280 36c7f0561125943e operator",
            "Orders::cancel deb8489f34994e5a e1506381a35c8ced db0311924c0e7333 operator",
        ]
    );
    // The error column's kind: ClosureViolation is the server error.
    let kinds: Vec<(String, bool)> = rows
        .rows
        .iter()
        .map(|r| match &r.handler {
            Handled::Fn(h) => (format!("{}/{}", r.surface, r.member), h.server_error),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("Admin/Ledger::rebalance".to_string(), true),
            ("Admin/Orders::cancel".to_string(), false),
            ("Public/Orders::cancel".to_string(), false),
            ("Public/Orders::place".to_string(), true),
        ]
    );
}

/// The model holds the family: the surfaces with their digests, each row
/// joined to its handler's function row, its pools the placement table's.
#[test]
fn the_model_holds_the_fixtures_rows() {
    let snap = fixture();
    let model = snap.demand_model().expect("the fixture denotes a model");
    model.validate().expect("the model's laws hold, the digest law among them");
    let e = &model.entities;
    let surfaces: Vec<(&str, String)> = e.surfaces.iter().map(|s| (s.name.as_str(), digest_text(s.digest))).collect();
    assert_eq!(
        surfaces,
        vec![("Admin", "fnv1a64:40381db6685c9f75".to_string()), ("Public", "fnv1a64:a8930d6e7998e986".to_string())]
    );
    let rows: Vec<String> = e
        .surface_rows
        .iter()
        .map(|r| {
            format!(
                "{}/{} {} [{}]",
                e.surfaces[r.surface.index()].name,
                r.member,
                r.handler.map_or("-".to_string(), |f| e.functions[f.index()].display.clone()),
                r.pools.join(",")
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            "Admin/Ledger::rebalance Ledger::rebalance [desk]",
            "Admin/Orders::cancel Orders::cancel [desk,partner]",
            "Public/Orders::cancel Orders::cancel [desk,partner]",
            "Public/Orders::place Orders::place [desk,partner]",
        ]
    );
}

/// `@rpc` contributes the row an `rpc` line does, to the seed's default
/// surface named after the seed: the same handler gives the same row
/// under either spelling, so the same digest.
#[test]
fn an_rpc_attribute_and_an_rpc_line_are_one_row() {
    let types = "role trader;
        type CancelOrder { order: Int; }
        type Cancelled { order: Int; was_open: Bool; }";
    let block = source(&format!(
        "{types}
        api desk {{ rpc Orders::cancel requires: [trader]; }}
        locus Orders {{ fn cancel(c: CancelOrder) -> Cancelled {{ return Cancelled {{ order: c.order, was_open: true }}; }} }}
        fn main() {{ }}"
    ));
    let attr = source(&format!(
        "{types}
        locus Orders {{
            @rpc(requires: [trader])
            fn cancel(c: CancelOrder) -> Cancelled {{ return Cancelled {{ order: c.order, was_open: true }}; }}
        }}
        fn main() {{ }}"
    ));
    let b = block.demand_surface_rows().expect("rows");
    let a = attr.demand_surface_rows().expect("rows");
    let seed = a.surfaces[0].name.clone();
    assert!(a.rows.iter().all(|r| r.from_attr && r.surface == seed), "the default surface is the seed's");
    assert_eq!(lines(b, "desk"), lines(a, &seed));
    assert_eq!(b.surfaces[0].digest, a.surfaces[0].digest, "one row, one digest, whatever the surface's name");
}

/// Nothing else about a locus is read: its subscriptions, its `expose`
/// members and a reply type on a bus handler make no row.
#[test]
fn a_locus_without_rows_has_no_surface() {
    let snap = source(
        "type Ping { n: Int; }
        topic Pings { payload: Ping; }
        locus Echo {
            contract { expose n: Int; }
            params { n: Int = 0; }
            bus { subscribe Pings as on_ping; }
            fn on_ping(p: Ping) -> Int { return p.n; }
        }
        fn main() { }",
    );
    let rows = snap.demand_surface_rows().expect("rows");
    assert!(rows.is_empty(), "{rows:?}");
}
