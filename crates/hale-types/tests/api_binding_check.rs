//! The handler signature rules (GH #1108: an optional `std::api::Context`)
//! and the role vocabulary rules (GH #1109: `role` declarations, `includes`).
//! R4 retired the `api:` entry these files once covered, with its knobs,
//! its one-replier rule, its gate sites and its role source; a
//! surface's `requires` and a serve site's sources are the surface laws'
//! and the serve laws' (`surface_rows.rs`, `serve_laws.rs`).

#[path = "support/entries.rs"]
mod entries;
use entries::check_files;

fn check(src: &str) -> Vec<String> {
    check_files(&[("main.hl", src)]).into_iter().map(|d| d.message).collect()
}

fn role_msgs(src: &str) -> Vec<String> {
    check(src).into_iter().filter(|m| m.contains("role")).collect()
}

fn roles_program(roles: &str) -> String {
    format!("{roles}\nmain locus App {{ }}\nfn main() {{ App {{ }}; }}\n")
}

#[test]
fn a_return_type_on_a_subscribed_handler_is_accepted() {
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
fn a_context_parameter_is_accepted() {
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

// ---- GH #1109: the role vocabulary ------------------------------------------

#[test]
fn declared_roles_are_clean() {
    let msgs = role_msgs(&roles_program("role support;\nrole auditor;\nrole owner includes support, auditor;"));
    assert!(msgs.is_empty(), "{:?}", msgs);
}

#[test]
fn an_undeclared_role_in_includes_is_an_error() {
    let msgs = role_msgs(&roles_program("role owner includes nope;"));
    assert!(msgs.iter().any(|m| m.contains("`role owner includes …`") && m.contains("`nope`")), "{:?}", msgs);
    // `owner` is no longer implicitly declared: nothing in the surface path names it.
    let msgs = role_msgs(&roles_program("role support includes owner;"));
    assert!(msgs.iter().any(|m| m.contains("`owner`") && m.contains("nothing declares")), "{:?}", msgs);
}

#[test]
fn the_vocabulary_is_one_name_once_and_acyclic() {
    let msgs = role_msgs(&roles_program("role support;\nrole support;"));
    assert!(msgs.iter().any(|m| m.contains("role `support` is declared twice")), "{:?}", msgs);
    let msgs = role_msgs(&roles_program("role a includes b;\nrole b includes a;"));
    assert!(msgs.iter().any(|m| m.contains("includes itself")), "{:?}", msgs);
}

#[test]
fn a_surface_row_requires_a_declared_role() {
    let src = r#"
role support;
type Refund { id: Int; }
locus Desk {
    bus { subscribe Refunds as on_refund; }
    @rpc(requires: [suport])
    fn on_refund(r: Refund) { }
}
topic Refunds { payload: Refund; subject: "app.refund"; }
main locus App { params { desk: Desk = Desk { }; } }
fn main() { App { }; }
"#;
    let msgs = role_msgs(src);
    assert!(msgs.iter().any(|m| m.contains("requires `suport`, which no `role` declares")), "{:?}", msgs);
}
