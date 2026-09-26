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
    hale_syntax::api_gen::generate_api(&mut [&mut prog], None);
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

// ---- GH #1108: the handler signature rule ----------------------------------

#[test]
fn a_context_parameter_is_accepted_with_or_without_the_binding() {
    let src = r#"
type Claim { task: Int; }
type Lease { token: String; }
topic Claims { payload: Claim; subject: "t.claim"; }
locus Head {
    bus { subscribe Claims as on_claim; publish Claims; }
    fn on_claim(c: Claim, ctx: std::api::Context) -> Lease {
        return Lease { token: ctx.via + ":" + ctx.caller.name };
    }
}
fn main() { Head { }; }
"#;
    let msgs = check(src);
    assert!(msgs.is_empty(), "{:?}", msgs);
    let with_binding = format!(
        "{}\nmain locus App {{ params {{ head: Head = Head {{ }}; }} bindings {{ api: unix(\"/tmp/t.sock\", bound: 8, on_full: refuse); }} }}\n",
        src.replace("fn main() { Head { }; }", "")
    );
    let msgs = check(&with_binding);
    assert!(!msgs.iter().any(|m| m.contains("error") || m.contains("api binding")), "{:?}", msgs);
}

#[test]
fn a_second_parameter_that_is_not_a_context_is_refused() {
    let src = r#"
type Claim { task: Int; }
topic Claims { payload: Claim; subject: "t.claim"; }
locus Head {
    bus { subscribe Claims as on_claim; publish Claims; }
    fn on_claim(c: Claim, extra: Int) { }
}
fn main() { Head { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("optionally followed by `ctx: std::api::Context`") && m.contains("takes 2 parameters")),
        "{:?}",
        msgs
    );
}

#[test]
fn a_context_handler_on_a_transport_bound_topic_is_refused() {
    let src = r#"
type Claim { task: Int; }
topic Claims { payload: Claim; subject: "t.claim"; }
locus Head {
    bus { subscribe Claims as on_claim; }
    fn on_claim(c: Claim, ctx: std::api::Context) { }
}
main locus App {
    params { head: Head = Head { }; }
    bindings { Claims: unix("/tmp/t.sock", role: listen); }
}
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("bound to a transport") && m.contains("`local` never stands for trust")),
        "{:?}",
        msgs
    );
}

#[test]
fn a_drain_handler_cannot_take_a_context_yet() {
    let src = r#"
type Tick { n: Int; }
topic Ticks { payload: Tick; subject: "t.tick"; }
locus Feed {
    bus { subscribe Ticks as on_ticks; publish Ticks; }
    fn on_ticks(feed: Drain<Tick>, ctx: std::api::Context) { }
}
fn main() { Feed { }; }
"#;
    let msgs = check(src);
    assert!(msgs.iter().any(|m| m.contains("`Drain<T>` batch handler cannot take a `std::api::Context` yet")), "{:?}", msgs);
}

/// The handler stays the subscriber by name: an analysis keyed on the
/// handler (here the unowned-subscriber rule) finds it whether or not
/// it takes a context.
#[test]
fn handler_keyed_analyses_see_a_context_handler() {
    let without = r#"
type Claim { task: Int; }
topic Claims { payload: Claim; subject: "t.claim"; }
locus Worker {
    bus { subscribe Claims as on_claim; }
    fn on_claim(c: Claim) { }
}
locus Head {
    bus { subscribe Claims as on_claim; publish Claims; }
    fn on_claim(c: Claim) { Worker { }; }
}
fn main() { Head { }; }
"#;
    let with = without.replace("fn on_claim(c: Claim) { Worker { }; }", "fn on_claim(c: Claim, ctx: std::api::Context) { Worker { }; }");
    let a = check(without);
    let b = check(&with);
    assert!(a.iter().any(|m| m.contains("unowned inside")), "the rule fires without a context: {:?}", a);
    assert!(b.iter().any(|m| m.contains("unowned inside")), "and with one: {:?}", b);
    assert_eq!(a.len(), b.len(), "the same findings either way:\n{:?}\n{:?}", a, b);
}

// ---- GH #1109: roles and gates ---------------------------------------------

fn gated_program(roles: &str, gate_fn: &str, gate_pub: &str, gate_expose: &str, extra: &str) -> String {
    format!(
        r#"
{roles}
type Refund {{ id: Int; }}
type Ledger {{ n: Int; }}
type Moved {{ n: Int; }}
topic Refunds {{ payload: Refund; subject: "app.refund"; }}
topic Moves {{ payload: Moved; subject: "app.moved"; }}
locus Desk {{
    contract {{ {gate_expose} expose ledger: Ledger; }}
    params {{ ledger: Ledger = Ledger {{ n: 0 }}; }}
    bus {{ subscribe Refunds as on_refund; {gate_pub} publish Moves; }}
    {gate_fn}
    fn on_refund(r: Refund) {{ Moves <- Moved {{ n: r.id }}; }}
    fn plain() {{ }}
}}
{extra}
main locus App {{
    params {{ desk: Desk = Desk {{ }}; }}
    bindings {{ api: unix("/tmp/t.sock", bound: 8, on_full: refuse); }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

fn role_msgs(src: &str) -> Vec<String> {
    check(src).into_iter().filter(|m| m.contains("role") || m.contains("gated") || m.contains("RoleSource")).collect()
}

#[test]
fn a_gated_program_with_declared_roles_is_clean() {
    let msgs = role_msgs(&gated_program(
        "role support;\nrole auditor;\nrole owner includes support, auditor;",
        "@gated(role: support)",
        "@gated(role: support)",
        "@gated(role: auditor)",
        "",
    ));
    assert!(msgs.is_empty(), "{:?}", msgs);
    // `owner` needs no declaration to be named.
    let msgs = role_msgs(&gated_program("", "@gated(role: owner)", "@gated(role: owner)", "@gated(role: owner)", ""));
    assert!(msgs.is_empty(), "{:?}", msgs);
}

#[test]
fn an_undeclared_role_is_an_error_at_every_site() {
    let msgs = role_msgs(&gated_program("role support;", "@gated(role: suport)", "", "", ""));
    assert!(msgs.iter().any(|m| m.contains("names role `suport`, which nothing declares")), "{:?}", msgs);
    let msgs = role_msgs(&gated_program("role support;", "", "@gated(role: nope)", "", ""));
    assert!(msgs.iter().any(|m| m.contains("`nope`") && m.contains("publish")), "{:?}", msgs);
    let msgs = role_msgs(&gated_program("role support;", "", "", "@gated(role: nope)", ""));
    assert!(msgs.iter().any(|m| m.contains("`nope`") && m.contains("expose")), "{:?}", msgs);
    let msgs = role_msgs(&gated_program("role owner includes nope;", "", "", "", ""));
    assert!(msgs.iter().any(|m| m.contains("`role owner includes …`") && m.contains("`nope`")), "{:?}", msgs);
}

#[test]
fn the_vocabulary_is_one_name_once_and_acyclic() {
    let msgs = role_msgs(&gated_program("role support;\nrole support;", "", "", "", ""));
    assert!(msgs.iter().any(|m| m.contains("role `support` is declared twice")), "{:?}", msgs);
    let msgs = role_msgs(&gated_program("role a includes b;\nrole b includes a;", "", "", "", ""));
    assert!(msgs.iter().any(|m| m.contains("includes itself")), "{:?}", msgs);
}

#[test]
fn a_gate_on_a_plain_method_is_refused() {
    let src = gated_program("role support;", "", "", "", "").replace("fn plain() { }", "@gated(role: support)\n    fn plain() { }");
    let msgs = role_msgs(&src);
    assert!(msgs.iter().any(|m| m.contains("`Desk.plain`") && m.contains("no `subscribe` line")), "{:?}", msgs);
}

#[test]
fn every_subscriber_and_publisher_of_one_topic_states_the_same_gate() {
    let extra = r#"
locus Tally {
    params { n: Int = 0; }
    bus { subscribe Refunds as on_refund; publish Moves; }
    fn on_refund(r: Refund) { self.n = self.n + 1; }
}
"#;
    let msgs = role_msgs(&gated_program("role support;", "@gated(role: support)", "@gated(role: support)", "", extra));
    assert!(
        msgs.iter().any(|m| m.contains("topic `Refunds`") && m.contains("Desk.on_refund gated `support`") && m.contains("Tally.on_refund ungated")),
        "subscribers: {:?}",
        msgs
    );
    assert!(msgs.iter().any(|m| m.contains("topic `Moves`") && m.contains("publishes")), "publishers: {:?}", msgs);
    let agreed = extra.replace("fn on_refund", "@gated(role: support)\n    fn on_refund").replace("publish Moves;", "@gated(role: support) publish Moves;");
    let msgs = role_msgs(&gated_program("role support;", "@gated(role: support)", "@gated(role: support)", "", &agreed));
    assert!(msgs.is_empty(), "{:?}", msgs);
}

#[test]
fn a_gated_handler_on_a_transport_bound_topic_is_refused() {
    let src = gated_program("role support;", "@gated(role: support)", "", "", "")
        .replace(r#"bindings { api: unix("/tmp/t.sock", bound: 8, on_full: refuse); }"#, r#"bindings { Refunds: unix("/tmp/r.sock", role: listen); }"#);
    let msgs = role_msgs(&src);
    assert!(msgs.iter().any(|m| m.contains("bound to a transport in `bindings { }` that has no gate")), "{:?}", msgs);
}

#[test]
fn an_app_role_source_must_answer_holds() {
    let good = gated_program("role support;", "@gated(role: support)", "", "", r#"
locus Record {
    params { n: Int = 0; }
    fn holds(p: std::api::Principal, r: String) -> Bool { return p.uid == 7 && r == "support"; }
}
"#).replace(r#"bound: 8, on_full: refuse)"#, r#"bound: 8, on_full: refuse, roles: Record { n: 1 })"#);
    let msgs = role_msgs(&good);
    assert!(msgs.is_empty(), "{:?}", msgs);
    let bad = good.replace("fn holds(p: std::api::Principal, r: String) -> Bool", "fn holds(p: std::api::Principal) -> Bool");
    let msgs = role_msgs(&bad);
    assert!(msgs.iter().any(|m| m.contains("does not satisfy std::api::RoleSource")), "{:?}", msgs);
    let missing = good.replace("roles: Record { n: 1 }", "roles: Nowhere { }");
    let msgs = role_msgs(&missing);
    assert!(msgs.iter().any(|m| m.contains("is no locus of this bundle")), "{:?}", msgs);
    // Review F6: the parameter types are checked, with the fn's span,
    // and the source may be a main param the program built.
    let wrong = good.replace("fn holds(p: std::api::Principal, r: String) -> Bool", "fn holds(p: String, r: Int) -> Bool");
    let msgs = role_msgs(&wrong);
    assert!(msgs.iter().any(|m| m.contains("first parameter is not a `std::api::Principal`") && m.contains("second parameter is not a `String`")), "{:?}", msgs);
    let via_self = good
        .replace("roles: Record { n: 1 }", "roles: self.record")
        .replace("params { desk: Desk = Desk { }; }", "params { desk: Desk = Desk { }; record: Record = Record { n: 2 }; }");
    let msgs = role_msgs(&via_self);
    assert!(msgs.is_empty(), "{:?}", msgs);
    let via_self_wrong = via_self.replace("fn holds(p: std::api::Principal, r: String) -> Bool", "fn holds(p: std::api::Principal) -> Bool");
    let msgs = role_msgs(&via_self_wrong);
    assert!(msgs.iter().any(|m| m.contains("takes 1 parameter(s), not 2")), "{:?}", msgs);
}

#[test]
fn a_gate_on_a_free_fn_is_refused() {
    // Review F3: nothing at top level is reached from the binding, and
    // the role is checked all the same.
    let src = gated_program("role support;", "", "", "", "@gated(role: nope)
fn helper() { }");
    let msgs = role_msgs(&src);
    assert!(msgs.iter().any(|m| m.contains("on the free fn `helper`")), "{:?}", msgs);
    assert!(msgs.iter().any(|m| m.contains("names role `nope`, which nothing declares")), "{:?}", msgs);
}

#[test]
fn one_handler_on_two_topics_is_two_sites() {
    // Review F5: the coherence rule is per topic, so a handler that
    // subscribes two topics cannot hide a disagreement on the second.
    let src = r#"
role support;
type Refund { id: Int; }
type Audit { id: Int; }
topic Refunds { payload: Refund; subject: "app.refund"; }
topic Audits { payload: Audit; subject: "app.audit"; }
locus Desk {
    bus { subscribe Refunds as on_any; subscribe Audits as on_any; }
    @gated(role: support)
    fn on_any(r: Refund) { }
}
locus Tally {
    bus { subscribe Audits as on_audit; }
    fn on_audit(a: Audit) { }
}
main locus App {
    params { desk: Desk = Desk { }; tally: Tally = Tally { }; }
    bindings { api: unix("/tmp/t.sock", bound: 8, on_full: refuse); }
}
fn main() { App { }; }
"#;
    let msgs = role_msgs(src);
    assert!(msgs.iter().any(|m| m.contains("topic `Audits`") && m.contains("Desk.on_any gated `support`") && m.contains("Tally.on_audit ungated")), "{:?}", msgs);
}

#[test]
fn a_stream_follows_its_topics_subscribers_unless_the_publish_says() {
    // Review ruling: a topic both subscribed (gated) and published is a
    // stream under the same gate; a publish member may state its own.
    let src = r#"
role support;
role auditor;
type Refund { id: Int; }
topic Refunds { payload: Refund; subject: "app.refund"; }
locus Desk {
    bus { subscribe Refunds as on_refund; publish Refunds; }
    @gated(role: support)
    fn on_refund(r: Refund) { }
}
main locus App {
    params { desk: Desk = Desk { }; }
    bindings { api: unix("/tmp/t.sock", bound: 8, on_full: refuse); }
}
fn main() { App { }; }
"#;
    let mut prog = parse_source(src).expect("parse failed");
    hale_syntax::json_gen::generate_json_parsers(&mut prog);
    let surface = hale_syntax::api_gen::api_surface(&[&prog]).expect("a surface");
    assert_eq!(surface.streams[0].role.as_deref(), Some("support"), "inherited from the subscribers");
    let own = src.replace("publish Refunds;", "@gated(role: auditor) publish Refunds;");
    let mut prog = parse_source(&own).expect("parse failed");
    hale_syntax::json_gen::generate_json_parsers(&mut prog);
    let surface = hale_syntax::api_gen::api_surface(&[&prog]).expect("a surface");
    assert_eq!(surface.streams[0].role.as_deref(), Some("auditor"), "the publish member's own");
}
