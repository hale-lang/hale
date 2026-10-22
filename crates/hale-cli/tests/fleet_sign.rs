//! GH #408 Phase 7, phase 4: a signed fleet.
//!
//! A signature makes the certificate portable — it proves the rows a
//! composition read are the ones a key-holder meant, never that the
//! code behaves. It covers the fleet's `deployment` rows, the EXACT
//! BYTES `hale fleet export <seed>` writes (each instance's seed, host
//! and component shape hash, the derived routes, the claims), which the
//! same source derives the same every time; it lives in `fleet.sig`
//! beside the fleet seed.
//!
//! Trust is strict when declared: passing `--trust` (or listing
//! `[fleet_trust]` keys) makes unsigned or unverifiable rows a refusal.
//! Declaring a trust set and then quietly admitting unsigned rows would
//! be law that looks bound and binds nothing — so these tests care as
//! much about what is REFUSED as what verifies.

#[path = "support/fleet_seed.rs"]
mod fleet_seed;

use fleet_seed::{fleet, hale_in, root, write};
use std::path::{Path, PathBuf};

const TOPICS: &str = r#"
type Tick { n: Int; }
topic Ticks { payload: Tick; subject: "svc.ticks"; }
"#;

const SRC: &str = r#"
import "../lib" as t;
locus Clock {
    params { n: Int = 0; }
    bus { publish t::Ticks; }
    fn tick() { let x = t::Tick { n: 1 }; t::Ticks <- x; }
}
main locus ClockApp { params { c: Clock = Clock { }; } }
fn main() { ClockApp { }; }
"#;

const DST: &str = r#"
import "../lib" as t;
locus Counter {
    params { seen: Int = 0; }
    bus { subscribe t::Ticks as on_tick; }
    fn on_tick(x: t::Tick) { self.seen = self.seen + x.n; }
}
main locus CounterApp { params { c: Counter = Counter { }; } }
fn main() { CounterApp { }; }
"#;

/// Two seeds, a fleet seed `prod/` wiring them, and two keypairs: `ops`
/// (trusted) and `rogue` (not).
fn workspace(tag: &str) -> PathBuf {
    let r = root(&format!("sign_{tag}"));
    write(&r, "lib/topics.hl", TOPICS);
    write(&r, "src/main.hl", SRC);
    write(&r, "dst/main.hl", DST);
    write(&r, "hale.toml", "[deps]\n");
    write(
        &r,
        "prod/fleet.hl",
        &fleet(
            &[("sa", "../src"), ("da", "../dst"), ("t", "../lib")],
            &["clock: sa::ClockApp;", "counter: da::CounterApp;"],
            &[],
            &["t::Ticks: unix(\"/tmp/hale-fleet-sign.sock\");"],
            &[],
        ),
    );
    for k in ["ops", "rogue"] {
        let (out, code) = hale_in(&r, &["fleet", "keygen", k]);
        assert_eq!(code, 0, "keygen {k}: {out}");
    }
    r
}

fn p(r: &Path, rel: &str) -> String {
    r.join(rel).to_string_lossy().into_owned()
}

#[test]
fn signed_rows_verify_and_the_artifact_records_the_key() {
    let r = workspace("ok");
    let (out, code) = hale_in(&r, &["fleet", "sign", "prod", "--key", "ops.pem"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        r.join("prod/fleet.sig").is_file(),
        "the signature lives beside the fleet seed"
    );
    let key_id = out.split("key_id ").nth(1).unwrap_or("").trim().to_string();
    assert!(!key_id.is_empty(), "{out}");
    let (out, code) = hale_in(&r, &["fleet", "check", "prod", "--trust", "ops.pub.pem"]);
    assert_eq!(code, 0, "{out}");
    let (out, code) = hale_in(&r, &["fleet", "dump", "prod", "--trust", "ops.pub.pem"]);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("fleet artifact");
    for c in v["components"].as_array().expect("components") {
        assert_eq!(
            c["signed_by"].as_str(),
            Some(key_id.as_str()),
            "each component is admitted under the rows' key: {c}"
        );
    }
    // without trust roots the same composition records none
    let (out, _) = hale_in(&r, &["fleet", "dump", "prod"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("fleet artifact");
    assert!(v["components"][0]["signed_by"].is_null(), "{out}");
    let _ = std::fs::remove_dir_all(&r);
}

/// A component whose model changes after signing changes the rows: the
/// signature no longer verifies.
#[test]
fn a_changed_component_is_refused_after_signing() {
    let r = workspace("changed");
    let (_, code) = hale_in(&r, &["fleet", "sign", "prod", "--key", "ops.pem"]);
    assert_eq!(code, 0);
    write(&r, "dst/main.hl", &DST.replace("fn on_tick(x: t::Tick) { self.seen = self.seen + x.n; }", "fn on_tick(x: t::Tick) { self.seen = self.seen + x.n; }\n    fn reset() { self.seen = 0; }"));
    let (out, code) = hale_in(&r, &["fleet", "check", "prod", "--trust", "ops.pub.pem"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("does not verify") && out.contains("rows changed after signing"),
        "{out}"
    );
}

#[test]
fn unsigned_rows_are_refused_when_trust_is_declared() {
    let r = workspace("unsigned");
    let (out, code) = hale_in(&r, &["fleet", "check", "prod", "--trust", "ops.pub.pem"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("no signature at") && out.contains("unsigned rows are not admissible"),
        "{out}"
    );
}

#[test]
fn a_signature_from_an_untrusted_key_is_refused() {
    let r = workspace("rogue");
    let (_, code) = hale_in(&r, &["fleet", "sign", "prod", "--key", "rogue.pem"]);
    assert_eq!(code, 0);
    let (out, code) = hale_in(&r, &["fleet", "check", "prod", "--trust", "ops.pub.pem"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("does not verify"), "{out}");
}

/// `[fleet_trust]` in the manifest binds the all-fleets form the same
/// way `--trust` binds a named seed.
#[test]
fn fleet_trust_in_the_manifest_binds_every_declared_fleet() {
    let r = workspace("manifest");
    write(
        &r,
        "hale.toml",
        "[deps]\n\n[fleets]\nsigned = \"prod\"\n\n[fleet_trust]\nkeys = [\"ops.pub.pem\"]\n",
    );
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    assert_eq!(code, 1, "manifest trust must refuse unsigned: {out}");
    assert!(out.contains("no signature at"), "{out}");
    let (_, code) = hale_in(
        &r,
        &["fleet", "sign", &p(&r, "prod"), "--key", &p(&r, "ops.pem")],
    );
    assert_eq!(code, 0);
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 0, "signed fleet passes under manifest trust: {out}");
}
