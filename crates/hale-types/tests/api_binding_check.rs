//! GH #1106: the `api:` entry's checker rules. The entry is lowered by
//! `hale_syntax::api_gen` before the checker runs (the CLI does that);
//! these tests run the pass the way the CLI does and read what the
//! checker says about the result, so they cover both halves: the
//! diagnostics the entry itself earns, and that the synthesized code
//! typechecks like the author's.

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let mut prog = parse_source(src).expect("parse failed");
    hale_syntax::json_gen::generate_json_parsers(&mut prog);
    hale_syntax::api_gen::generate_api(&mut [&mut prog]);
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn program(entry: &str, extra: &str) -> String {
    format!(
        r#"
type Verdict {{ review_id: Int; verdict: String; }}
type VerdictResult {{ ok: Bool; note: String; }}
type PriceMoved {{ sym: String; price: Float; }}
type Ledger {{ balance: Int; entries: Int; }}
topic Verdicts {{ payload: Verdict; subject: "app.verdict"; }}
topic Prices {{ payload: PriceMoved; subject: "app.price"; }}
locus Billing {{
    contract {{ expose ledger: Ledger; }}
    params {{ ledger: Ledger = Ledger {{ balance: 100, entries: 0 }}; }}
    bus {{ subscribe Verdicts as on_verdict; publish Prices; }}
    fn on_verdict(v: Verdict) -> VerdictResult {{
        self.ledger.entries = self.ledger.entries + 1;
        return VerdictResult {{ ok: true, note: v.verdict }};
    }}
    run() {{ Prices <- PriceMoved {{ sym: "ABC", price: 1.5 }}; }}
}}
{extra}
main locus App {{
    params {{ billing: Billing = Billing {{ }}; }}
    bindings {{ {entry} }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

#[test]
fn complete_entry_typechecks_with_no_api_diagnostic() {
    let msgs = check(&program(
        r#"api: unix("/tmp/t.sock", bound: 8, on_full: refuse);"#,
        "",
    ));
    let api: Vec<&String> = msgs.iter().filter(|m| m.contains("api")).collect();
    assert!(api.is_empty(), "{:?}", api);
    // The subscribed topic has no in-program publisher; under the
    // binding a caller is its publisher, so the lint stays quiet.
    assert!(
        !msgs.iter().any(|m| m.contains("subscribed but never published")),
        "{:?}",
        msgs
    );
}

#[test]
fn bound_and_policy_are_required() {
    let msgs = check(&program(r#"api: unix("/tmp/t.sock");"#, ""));
    assert!(
        msgs.iter().any(|m| m.contains("`bound:` and `on_full: refuse` are required")
            && m.contains("bound: 64, on_full: refuse")),
        "{:?}",
        msgs
    );
    let msgs = check(&program(r#"api: unix("/tmp/t.sock", bound: 8);"#, ""));
    assert!(
        msgs.iter().any(|m| m.contains("`bound:` and `on_full: refuse` are required")),
        "{:?}",
        msgs
    );
}

#[test]
fn watch_bound_and_its_policy_go_together() {
    let msgs = check(&program(
        r#"api: unix("/tmp/t.sock", bound: 8, on_full: refuse, watch_bound: 16);"#,
        "",
    ));
    assert!(
        msgs.iter().any(|m| m.contains("`watch_bound:` and `on_watch_full:` go together")),
        "{:?}",
        msgs
    );
    let msgs = check(&program(
        r#"api: unix("/tmp/t.sock", bound: 8, on_full: refuse, watch_bound: 16, on_watch_full: drop_new);"#,
        "",
    ));
    assert!(!msgs.iter().any(|m| m.contains("api binding")), "{:?}", msgs);
}

#[test]
fn two_replying_subscribers_are_ambiguous() {
    let msgs = check(&program(
        r#"api: unix("/tmp/t.sock", bound: 8, on_full: refuse);"#,
        r#"
locus Audit {
    bus { subscribe Verdicts as on_verdict; }
    fn on_verdict(v: Verdict) -> VerdictResult { return VerdictResult { ok: false, note: "" }; }
}
"#,
    ));
    assert!(
        msgs.iter().any(|m| m.contains("two subscribers that declare a return type")
            && m.contains("Verdicts")),
        "{:?}",
        msgs
    );
}

#[test]
fn a_payload_without_a_json_form_is_left_out_with_a_warning() {
    let msgs = check(&program(
        r#"api: unix("/tmp/t.sock", bound: 8, on_full: refuse);"#,
        r#"
type Money { amount: Decimal; }
topic Payments { payload: Money; subject: "app.pay"; }
locus Teller {
    bus { subscribe Payments as on_pay; }
    fn on_pay(m: Money) { }
}
"#,
    ));
    assert!(
        msgs.iter().any(|m| m.contains("topic `Payments` is not served through the binding")
            && m.contains("`amount`")
            && m.contains("Decimal")),
        "{:?}",
        msgs
    );
    // The rest of the surface still lowers and typechecks.
    assert!(
        !msgs.iter().any(|m| m.contains("error") || m.contains("unknown")),
        "{:?}",
        msgs
    );
}

#[test]
fn a_return_type_on_a_subscribed_handler_is_accepted_without_the_binding() {
    let src = r#"
type Verdict { review_id: Int; }
type VerdictResult { ok: Bool; }
topic Verdicts { payload: Verdict; subject: "app.verdict"; }
locus Billing {
    bus { subscribe Verdicts as on_verdict; publish Verdicts; }
    fn on_verdict(v: Verdict) -> VerdictResult { return VerdictResult { ok: true }; }
}
fn main() { Billing { }; }
"#;
    let msgs = check(src);
    assert!(msgs.is_empty(), "{:?}", msgs);
}
