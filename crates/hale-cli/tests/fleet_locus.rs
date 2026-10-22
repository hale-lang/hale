//! Phase 4 (C2): the fleet locus. A fleet seed declares `fleet locus
//! NAME { params placement bindings claims }` over other seeds' `main`
//! loci (`tests/fixtures/fleet-locus`); `hale check` judges its laws,
//! `hale build` refuses it, and `hale fleet check <seed>` derives the
//! `deployment` rows (the routes from the instances' artifacts and the
//! bindings) and judges the claims across the instances as one graph.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let dest = to.join(p.file_name().unwrap());
        if p.is_dir() {
            copy_dir(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}

/// The fixture tree, copied: `fleet check` cuts each instance's artifact
/// beside the fleet seed.
fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_fleet_locus_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fleet-locus"),
        &d,
    );
    d
}

/// A fleet seed beside the fixture's: the fixture's fleet with `edits`.
fn variant(d: &Path, name: &str, edits: &[(&str, &str)]) {
    let mut src = std::fs::read_to_string(d.join("prod/fleet.hl")).unwrap();
    for (from, to) in edits {
        assert!(src.contains(from), "the fixture spells `{from}`");
        src = src.replace(from, to);
    }
    std::fs::create_dir_all(d.join(name)).unwrap();
    std::fs::write(d.join(name).join("fleet.hl"), src).unwrap();
}

#[test]
fn routes_are_derived_and_claims_are_judged_across_the_instances() {
    let d = workspace("routes");
    let (ok, out) = hale(&["fleet", "check", "prod"], &d);
    assert!(
        ok,
        "the fixture's fleet composes and its claims hold:\n{out}"
    );
    assert!(
        out.contains("route `t::OrderRequest` (svc.order.request, unix): orders -> gw-0, gw-1"),
        "the route is derived from the instances' artifacts and the binding, one subscriber per replica:\n{out}"
    );
    assert!(
        out.contains("on host `box-a`: orders") && out.contains("2 replicas: gw-0, gw-1"),
        "{out}"
    );
    assert!(
        out.contains("ok: fleet `Prod` composed — 3 instance(s), 1 route(s)"),
        "{out}"
    );

    // The same claims over a deployment that breaks two of them: two
    // order desks, and a claim that orders never reach the gateway.
    variant(
        &d,
        "twice",
        &[
            ("orders: host(\"box-a\");", "orders: replicas(2);"),
            (
                "no_return: forbid reaches(gw, orders);",
                "no_orders: forbid reaches(orders, gw);",
            ),
        ],
    );
    let (ok, out) = hale(&["fleet", "check", "twice"], &d);
    assert!(!ok, "{out}");
    assert!(
        out.contains("fleet claim `one_desk` violated") && out.contains("orders-0, orders-1"),
        "a count is over deployed instances:\n{out}"
    );
    assert!(
        out.contains("fleet claim `no_orders` violated")
            && out.contains("-(route `t::OrderRequest`)->")
            && out.contains("gw-0::Venue::take"),
        "the witness crosses the derived route:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_mis_binding_is_refused() {
    let d = workspace("misbinding");
    // a subscriber whose main does not bind the topic: it never leaves
    // that process, so the route would be fiction
    variant(
        &d,
        "unbound",
        &[(
            "import \"../venue\" as venue;",
            "import \"../venue-unbound\" as venue;",
        )],
    );
    let (ok, out) = hale(&["fleet", "check", "unbound"], &d);
    assert!(!ok && out.contains("instance `gw` subscribes subject `svc.order.request`, but its main binds no transport for it"), "{out}");
    // a subject no instance publishes wires nothing
    variant(
        &d,
        "nothing",
        &[("t::OrderRequest: unix(", "t::Fill: unix(")],
    );
    let (ok, out) = hale(&["fleet", "check", "nothing"], &d);
    assert!(!ok && out.contains("binding `t::Fill` wires nothing: no instance of the fleet publishes subject `svc.fill`"), "{out}");
    // a route over another transport than the instances bind
    variant(
        &d,
        "ring",
        &[(
            "t::OrderRequest: unix(\"/tmp/hale-fleet-locus-orders.sock\");",
            "t::OrderRequest: shm_ring(\"/orders\", slot_count: 8, on_overflow: block);",
        )],
    );
    let (ok, out) = hale(&["fleet", "check", "ring"], &d);
    assert!(
        !ok && out.contains("routes subject `svc.order.request` over `shm_ring`"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_fleet_is_checked_and_never_built() {
    let d = workspace("check");
    let (ok, out) = hale(&["check", "prod"], &d);
    assert!(ok, "{out}");
    let (ok, out) = hale(&["build", "prod"], &d);
    assert!(
        !ok && out.contains("`fleet locus Prod` is checked, never built"),
        "{out}"
    );
    // the laws: an instance is another seed's main, a claim's groups
    // are instances, a fleet never runs
    variant(
        &d,
        "bad",
        &[
            ("gw: venue::Gateway;", "gw: venue::Venue;"),
            (
                "no_return: forbid reaches(gw, orders);",
                "no_return: forbid reaches(gw, desks);",
            ),
        ],
    );
    std::fs::write(d.join("bad/main.hl"), "fn main() { }\n").unwrap();
    let (ok, out) = hale(&["check", "bad"], &d);
    assert!(!ok, "{out}");
    for want in [
        "fleet param `gw` names `venue::Venue`, which is not its seed's `main locus`",
        "fleet claim `no_return` names `desks`, which is not an instance of fleet locus `Prod`",
        "`fn main` beside `fleet locus Prod`: a fleet seed never runs",
    ] {
        assert!(out.contains(want), "missing `{want}`:\n{out}");
    }
    // a fleet holds no code
    variant(
        &d,
        "code",
        &[("    claims {", "    run() {\n    }\n    claims {")],
    );
    let (ok, out) = hale(&["check", "code"], &d);
    assert!(
        !ok && out.contains("a fleet locus holds `params`, `placement`, `bindings` and `claims`"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
