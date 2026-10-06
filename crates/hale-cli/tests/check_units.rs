//! GH #1076 (U5): `hale check --units`, the unit dialect's witness
//! report. A query over the unit rows and the conversions the check
//! recorded, never a re-check: per scalar declaration its denomination
//! and the declaration that fixed it, its policy, a point's origin, the
//! headroom of its representation and what a declared range fits in; per
//! narrowing its factor (or range), the policy that discharged it and
//! where the policy came from. The text form is stable, so it is pinned
//! whole here for the committed form's example (`units/committed_form.hl`);
//! `--json` carries the same fields.

use std::path::PathBuf;
use std::process::Command;

fn units_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/units")
}

/// `hale check <flags> <file>`, run in `tests/units` so the report names
/// the file as the reader does: its status, stdout and stderr.
fn check(file: &str, flags: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .args(flags)
        .arg(file)
        .current_dir(units_dir())
        .output()
        .expect("hale");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

const COMMITTED_FORM: &str = r#"units: 15 declarations, 7 narrowings

committed_form.hl:33:1  type ByteCount = quantity Int in B
    kind         : quantity
    denomination : B, fixed by its own `in`
    headroom     : ±9223372036854775807 B (8796093022207 MiB)

committed_form.hl:34:1  type Money = quantity Int in cent
    kind         : quantity
    denomination : cent, fixed by its own `in`
    headroom     : ±9223372036854775807 cent (92233720368547758 USD)

committed_form.hl:35:1  type Ratio = quantity Int in bp
    kind         : quantity
    denomination : bp, fixed by its own `in`
    headroom     : ±9223372036854775807 bp (92233720368547758 pct)

committed_form.hl:36:1  type Tick = quantity Int in tick
    kind         : quantity
    denomination : tick, fixed by its own `in`
    headroom     : ±9223372036854775807 tick

committed_form.hl:37:1  type Price = point Tick
    kind         : point of Tick
    denomination : tick, fixed by `type Tick = quantity Int in tick` (committed_form.hl:36:1)
    headroom     : ±9223372036854775807 tick

committed_form.hl:40:1  type TempDelta = quantity Int in mK
    kind         : quantity
    denomination : mK, fixed by its own `in`
    headroom     : ±9223372036854775807 mK

committed_form.hl:41:1  type Kelvin = point TempDelta
    kind         : point of TempDelta
    denomination : mK, fixed by `type TempDelta = quantity Int in mK` (committed_form.hl:40:1)
    headroom     : ±9223372036854775807 mK

committed_form.hl:42:1  type Celsius = point TempDelta { origin: 273_150 mK; }
    kind         : point of TempDelta
    denomination : mK, fixed by `type TempDelta = quantity Int in mK` (committed_form.hl:40:1)
    origin       : 273150 mK
    headroom     : ±9223372036854775807 mK

committed_form.hl:45:1  type OrderId = distinct Int
    kind         : identity

committed_form.hl:46:1  type SeqNo = distinct Int { range: 0..65536; }
    kind         : identity
    range        : 0..65536
    fits in      : u16

committed_form.hl:47:1  type Session = distinct Int { range: 0..64; }
    kind         : identity
    range        : 0..64
    fits in      : u8

committed_form.hl:48:1  type Byte = Int { range: 0..256; }
    kind         : range of Int
    range        : 0..256
    fits in      : u8

committed_form.hl:51:1  type WireStamp = Time in us { round: floor; }
    kind         : point of Time
    denomination : us, fixed by its own `in`
    policy       : floor, from its own `round:`
    headroom     : ±9223372036854775807 us (106751991 day)

committed_form.hl:52:1  type Bucket = quantity Int in 100ms { round: floor; }
    kind         : quantity, a denomination of Duration
    denomination : 100 ms, fixed by its own `in`
    policy       : floor, from its own `round:`
    headroom     : ±922337203685477580700 ms (10675199116730 day)

committed_form.hl:53:1  type Ledger = quantity Int in cent { round: half_even; }
    kind         : quantity, a denomination of Money
    denomination : cent, fixed by its own `in`
    policy       : half_even, from its own `round:`
    headroom     : ±9223372036854775807 cent (92233720368547758 USD)

narrowings:

committed_form.hl:64:17  d.in(s)
    Duration -> Duration in s, factor 1/1000000000
    policy       : or <value>, at the site

committed_form.hl:65:16  d.in(s)
    Duration -> Duration in s, factor 1/1000000000
    policy       : or floor, at the site

committed_form.hl:73:26  a
    Time -> WireStamp, factor 1/1000
    policy       : floor, the `round:` of `type WireStamp = Time in us { round: floor; }` (committed_form.hl:51:1)

committed_form.hl:76:22  spread / 2
    Tick -> Tick, factor 1/2
    policy       : or floor, at the site

committed_form.hl:81:24  odd
    Money in 1/10000 cent -> Ledger, factor 1/10000
    policy       : half_even, the `round:` of `type Ledger = quantity Int in cent { round: half_even; }` (committed_form.hl:53:1)
    headroom     : ±922337203685477 cent (9223372036854 USD), of Money in 1/10000 cent

committed_form.hl:83:16  Session(hdr.session)
    Int -> Session, range 0..64
    policy       : or <value>, at the site

committed_form.hl:84:15  SeqNo(n)
    Int -> SeqNo, range 0..65536
    policy       : or wrap, at the site
"#;

#[test]
fn the_committed_forms_report_is_pinned_whole() {
    let (ok, out, err) = check("committed_form.hl", &["--units"]);
    assert!(ok, "the committed form checks: {err}");
    assert_eq!(out, COMMITTED_FORM, "the whole report, as recorded; the report now reads:\n{out}");
}

#[test]
fn the_report_changes_nothing_the_check_says() {
    let (ok, out, err) = check("committed_form.hl", &[]);
    let (ok_units, _, err_units) = check("committed_form.hl", &["--units"]);
    assert!(ok && ok_units, "{err}");
    assert!(out.is_empty(), "a check prints nothing on stdout: {out}");
    assert_eq!(err, err_units, "the check's own output, with or without the report");
}

#[test]
fn the_json_form_carries_the_same_fields() {
    let (ok, out, err) = check("committed_form.hl", &["--units", "--json"]);
    assert!(ok, "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 1, "one object on one line, and no diagnostic: {out}");
    let v: serde_json::Value = serde_json::from_str(lines[0]).expect("JSON");
    assert_eq!(v["report"], "units");
    let declarations = v["declarations"].as_array().expect("declarations");
    let narrowings = v["narrowings"].as_array().expect("narrowings");
    assert_eq!((declarations.len(), narrowings.len()), (15, 7));
    // Every place the text names, the object names in the same order.
    for (d, at) in declarations.iter().zip(COMMITTED_FORM.lines().filter(|l| l.starts_with("committed_form.hl") && l.contains("  type "))) {
        assert_eq!(format!("{}  {}", d["declared"]["at"].as_str().unwrap(), d["declared"]["text"].as_str().unwrap()), at);
    }
    let ledger = declarations.iter().find(|d| d["name"] == "Ledger").expect("Ledger");
    assert_eq!(ledger["kind"], "quantity");
    assert_eq!(ledger["of"], "Money");
    assert_eq!(ledger["denomination"], "cent");
    assert_eq!(ledger["policy"], "half_even");
    assert_eq!(ledger["headroom"]["count"], "9223372036854775807");
    assert_eq!(ledger["headroom"]["coarse"]["unit"], "USD");
    let session = declarations.iter().find(|d| d["name"] == "Session").expect("Session");
    assert_eq!(session["range"], serde_json::json!(["0", "64"]));
    assert_eq!(session["fits_in"], "u8");
    let celsius = declarations.iter().find(|d| d["name"] == "Celsius").expect("Celsius");
    assert_eq!(celsius["origin"], "273150 mK");
    assert_eq!(celsius["denomination_fixed_by"]["text"], "type TempDelta = quantity Int in mK");
    let paid = &narrowings[4];
    assert_eq!(paid["site"]["text"], "odd");
    assert_eq!(paid["from"], "Money in 1/10000 cent");
    assert_eq!(paid["to"], "Ledger");
    assert_eq!(paid["factor"], "1/10000");
    assert_eq!(paid["policy"], "or half_even");
    assert_eq!(paid["policy_from"]["at"], "committed_form.hl:53:1");
    assert_eq!(paid["headroom"]["count"], "922337203685477");
    assert_eq!(paid["headroom"]["of"], "Money in 1/10000 cent");
    let whole = &narrowings[0];
    assert_eq!((whole["policy"].as_str(), whole["policy_from"].is_null()), (Some("or <value>"), true), "at the site");
    let seq = &narrowings[6];
    assert_eq!((seq["range"].clone(), seq["factor"].is_null()), (serde_json::json!(["0", "65536"]), true));
}

#[test]
fn a_program_with_no_quantity_says_so_in_one_line() {
    let (ok, out, err) = check("no_quantity.hl", &["--units"]);
    assert!(ok, "{err}");
    assert_eq!(out, "units: no quantity is declared\n");
}
