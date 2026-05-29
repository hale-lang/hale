//! Phase 3 routing-key end-to-end tests (2026-05-25). Drives
//! the full parser → typecheck → codegen → C-runtime pipeline
//! against small programs that exercise the swallow policy
//! (the v0.1 impl). See `spec/semantics.md` § "Phase 3:
//! routing keys" for the surface.

use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_bus_routing_keys_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// Canonical multi-instance routing: two subscribers, each with
/// a different `where key == self.id` filter. The publisher
/// sends two messages with different `id` fields; each
/// subscriber receives ONLY the message whose key matches its
/// id, never both.
#[test]
fn keyed_subscribe_routes_to_matching_instance_only() {
    let src = r#"
        type Ev { id: Int; payload: Int; }
        topic K {
            payload: Ev;
            subject: "k";
            keyed_by id;
        }
        locus Sub {
            params { my_id: Int = 0; tag: String = "?"; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) {
                println("sub.", self.tag, " got id=", e.id,
                        " payload=", e.payload);
            }
        }
        main locus App {
            params {
                a: Sub = Sub { my_id: 1, tag: "a" };
                b: Sub = Sub { my_id: 2, tag: "b" };
            }
            bus { publish K; }
            run() {
                K <- Ev { id: 1, payload: 100 };
                K <- Ev { id: 2, payload: 200 };
                K <- Ev { id: 1, payload: 101 };
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("multi_instance_routing", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // sub.a (my_id=1) should see id=1 twice; sub.b (my_id=2)
    // should see id=2 once. Neither should see the OTHER key's
    // messages — that's the contract the routing-key primitive
    // is enforcing.
    let a_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("sub.a "))
        .collect();
    let b_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("sub.b "))
        .collect();
    assert_eq!(
        a_lines.len(),
        2,
        "expected 2 lines for sub.a (matched id=1 twice); got {:?}",
        a_lines
    );
    assert_eq!(
        b_lines.len(),
        1,
        "expected 1 line for sub.b (matched id=2 once); got {:?}",
        b_lines
    );
    // No cross-contamination.
    for ln in &a_lines {
        assert!(
            ln.contains("id=1"),
            "sub.a saw a non-1 id: {}",
            ln
        );
    }
    for ln in &b_lines {
        assert!(
            ln.contains("id=2"),
            "sub.b saw a non-2 id: {}",
            ln
        );
    }
}

/// Unmatched-key publishes drop silently (the v0.1 swallow
/// policy). Send a message whose key doesn't match any
/// subscriber — assert no handler fired.
#[test]
fn keyed_publish_swallows_when_no_subscriber_matches() {
    let src = r#"
        type Ev { id: Int; }
        topic K { payload: Ev; subject: "k"; keyed_by id; }
        locus Sub {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { println("got id=", e.id); }
        }
        main locus App {
            params { a: Sub = Sub { my_id: 1 }; }
            bus { publish K; }
            run() {
                K <- Ev { id: 999 };
                println("after publish");
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("swallow_no_match", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("got id="),
        "expected no handler invocations (swallow); got: {:?}",
        stdout
    );
    assert!(stdout.contains("after publish"));
}

/// Decimal-typed routing keys (i128 routing space). Verifies
/// the high-half of the u128 carries through register + dispatch.
#[test]
fn keyed_subscribe_with_decimal_key() {
    let src = r#"
        type Ev { route: Decimal; v: Int; }
        topic K { payload: Ev; subject: "k"; keyed_by route; }
        locus Sub {
            params { r: Decimal = 0.0d; tag: String = "?"; }
            bus { subscribe K as on_k where key == self.r; }
            fn on_k(e: Ev) { println("sub.", self.tag, " v=", e.v); }
        }
        main locus App {
            params {
                a: Sub = Sub { r: 1.5d, tag: "a" };
                b: Sub = Sub { r: 2.5d, tag: "b" };
            }
            bus { publish K; }
            run() {
                K <- Ev { route: 1.5d, v: 10 };
                K <- Ev { route: 2.5d, v: 20 };
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("decimal_key", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let a_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("sub.a "))
        .collect();
    let b_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("sub.b "))
        .collect();
    assert_eq!(a_lines.len(), 1, "sub.a expected 1 line; got {:?}", a_lines);
    assert_eq!(b_lines.len(), 1, "sub.b expected 1 line; got {:?}", b_lines);
    assert!(a_lines[0].contains("v=10"), "got: {}", a_lines[0]);
    assert!(b_lines[0].contains("v=20"), "got: {}", b_lines[0]);
}

/// Unkeyed subscribers (no `where key ==`) on a KEYED topic
/// fire on EVERY keyed publish — they're the "audit-all sink"
/// pattern: a subscriber that wants to see all traffic on the
/// subject regardless of routing key. spec/semantics.md
/// § "Phase 3: routing keys" calls this out explicitly. Both
/// the specific-key sub and the unkeyed sub fire when key=1
/// matches the specific sub's filter.
#[test]
fn keyed_publish_fires_unkeyed_subscribers_as_audit_sinks() {
    let src = r#"
        type Ev { id: Int; }
        topic K { payload: Ev; subject: "k"; keyed_by id; }
        locus Specific {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { println("specific id=", e.id); }
        }
        locus Audit {
            bus { subscribe K as on_k; }
            fn on_k(e: Ev) { println("audit id=", e.id); }
        }
        main locus App {
            params {
                s: Specific = Specific { my_id: 1 };
                u: Audit = Audit { };
            }
            bus { publish K; }
            run() {
                K <- Ev { id: 1 };
                K <- Ev { id: 2 };
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("audit_sink_fires", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Specific (my_id=1) sees only id=1; audit sees both.
    let specific_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("specific "))
        .collect();
    let audit_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("audit "))
        .collect();
    assert_eq!(
        specific_lines.len(),
        1,
        "specific should fire once (key=1 match); got {:?}",
        specific_lines
    );
    assert!(
        specific_lines[0].contains("id=1"),
        "got: {}",
        specific_lines[0]
    );
    assert_eq!(
        audit_lines.len(),
        2,
        "audit-sink should fire twice (every keyed publish); got {:?}",
        audit_lines
    );
}

/// Backward-compat: unkeyed topics (no `keyed_by`) work exactly
/// as today. Unkeyed subscribers receive every publish. Lock-in
/// regression to make sure the codegen's keyed branch only
/// triggers when keyed_by is declared.
#[test]
fn unkeyed_topic_legacy_dispatch_unchanged() {
    let src = r#"
        type Ev { n: Int; }
        topic K { payload: Ev; subject: "k"; }
        locus A {
            bus { subscribe K as on_k; }
            fn on_k(e: Ev) { println("A n=", e.n); }
        }
        locus B {
            bus { subscribe K as on_k; }
            fn on_k(e: Ev) { println("B n=", e.n); }
        }
        main locus App {
            params {
                a: A = A { };
                b: B = B { };
            }
            bus { publish K; }
            run() { K <- Ev { n: 7 }; }
        }
        fn main() { App { }; }
    "#;
    let bin = build("unkeyed_legacy", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("A n=7"), "got: {:?}", stdout);
    assert!(stdout.contains("B n=7"), "got: {:?}", stdout);
}

/// `on_unmatched: fallback` policy. A subscriber declared with
/// `where key == _` catches messages whose key didn't match any
/// specific-key subscriber. When a specific-key subscriber DOES
/// match, the catch-unmatched does not fire.
#[test]
fn fallback_policy_catches_unmatched_keys() {
    let src = r#"
        type Ev { id: Int; tag: String; }
        topic K {
            payload: Ev;
            subject: "k";
            keyed_by id;
            on_unmatched: fallback;
        }
        locus Specific {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) {
                println("specific id=", e.id, " tag=", e.tag);
            }
        }
        locus CatchAll {
            bus { subscribe K as on_unknown where key == _; }
            fn on_unknown(e: Ev) {
                println("catchall id=", e.id, " tag=", e.tag);
            }
        }
        main locus App {
            params {
                s1: Specific = Specific { my_id: 1 };
                s2: Specific = Specific { my_id: 2 };
                catch: CatchAll = CatchAll { };
            }
            bus { publish K; }
            run() {
                K <- Ev { id: 1, tag: "a" };
                K <- Ev { id: 999, tag: "stray" };
                K <- Ev { id: 2, tag: "b" };
                K <- Ev { id: 42, tag: "rare" };
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("fallback_catches_unmatched", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let specific_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("specific "))
        .collect();
    let catchall_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("catchall "))
        .collect();
    // s1 sees id=1, s2 sees id=2 → 2 specific lines total.
    assert_eq!(
        specific_lines.len(),
        2,
        "specific subs should fire on id=1 and id=2; got {:?}",
        specific_lines
    );
    // catchall sees id=999 and id=42 (the unmatched ones).
    assert_eq!(
        catchall_lines.len(),
        2,
        "catchall should fire on the 2 unmatched publishes (id=999, \
         id=42); got {:?}",
        catchall_lines
    );
    // Make sure catchall did NOT fire on the matched publishes.
    for ln in &catchall_lines {
        assert!(
            ln.contains("tag=stray") || ln.contains("tag=rare"),
            "catchall picked up a matched-key publish: {}",
            ln
        );
    }
}

/// `where key == _` on a non-fallback topic is rejected at
/// typecheck. The diag cites the policy mismatch.
#[test]
fn fallback_sentinel_rejected_on_non_fallback_topic() {
    let src = r#"
        type Ev { id: Int; }
        topic K { payload: Ev; subject: "k"; keyed_by id; }
        locus L {
            bus { subscribe K as on_k where key == _; }
            fn on_k(e: Ev) { }
        }
        fn main() { L { }; }
    "#;
    // This goes through the typecheck pipeline. The compiler-
    // level error surface manifests as a build failure with a
    // diagnostic referencing the policy mismatch.
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), &program);
    let bundle = hale_types::Bundle { programs };
    let (scope, _) = hale_types::resolve::build_top_scope(&bundle);
    let diags = hale_types::check::check_bundle(&bundle, &scope, true);
    assert!(
        diags.iter().any(|d| {
            d.message.contains("where key == _")
                && d.message.contains("on_unmatched: fallback")
        }),
        "expected fallback-policy diag, got: {:?}",
        diags
    );
}

/// `on_unmatched: fallback` topic without any `_` subscriber is
/// rejected.
#[test]
fn fallback_topic_without_catchall_rejected() {
    let src = r#"
        type Ev { id: Int; }
        topic K {
            payload: Ev; subject: "k"; keyed_by id;
            on_unmatched: fallback;
        }
        locus L {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { }
        }
        fn main() { L { my_id: 1 }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), &program);
    let bundle = hale_types::Bundle { programs };
    let (scope, _) = hale_types::resolve::build_top_scope(&bundle);
    let diags = hale_types::check::check_bundle(&bundle, &scope, true);
    assert!(
        diags.iter().any(|d| {
            d.message.contains("on_unmatched: fallback")
                && d.message.contains("no subscriber declares")
        }),
        "expected missing-catchall diag, got: {:?}",
        diags
    );
}

/// `on_unmatched: fail` policy with `or raise`. A publish whose
/// routing key matches no specific-key subscriber panics via
/// lotus_root_panic with a BusUnmatchedKey marker; matched
/// publishes proceed normally.
#[test]
fn fail_policy_or_raise_panics_on_no_match() {
    let src = r#"
        type Ev { id: Int; }
        topic K {
            payload: Ev; subject: "k"; keyed_by id;
            on_unmatched: fail;
        }
        locus Sub {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { println("got id=", e.id); }
        }
        main locus App {
            params { a: Sub = Sub { my_id: 1 }; }
            bus { publish K; }
            run() {
                K <- Ev { id: 1 } or raise;
                println("after matched");
                K <- Ev { id: 999 } or raise;
                println("after unmatched");
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("fail_or_raise", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Exit non-zero (panic).
    assert!(
        !out.status.success(),
        "expected panic-exit; got success. stdout={:?}",
        stdout
    );
    // The first publish (id=1) matched, so dispatch_keyed_fallible
    // returned 1 and execution continued — "after matched" prints.
    // (The handler itself is enqueued to the cooperative queue and
    // wouldn't drain until run() returns; the panic on the second
    // publish kills the process before that drain. That's expected
    // semantics for `or raise`.)
    assert!(stdout.contains("after matched"), "stdout={:?}", stdout);
    // The second publish (id=999) should panic — execution must
    // not reach "after unmatched".
    assert!(
        !stdout.contains("after unmatched"),
        "panic must occur before this line; stdout={:?}",
        stdout
    );
    assert!(
        stderr.contains("BusUnmatchedKey"),
        "panic message missing; stderr={:?}",
        stderr
    );
}

/// `on_unmatched: fail` with `or discard` — no-match is silently
/// swallowed (publish-side equivalent of the swallow policy, but
/// per-call rather than per-topic).
#[test]
fn fail_policy_or_discard_swallows() {
    let src = r#"
        type Ev { id: Int; }
        topic K {
            payload: Ev; subject: "k"; keyed_by id;
            on_unmatched: fail;
        }
        locus Sub {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { println("got id=", e.id); }
        }
        main locus App {
            params { a: Sub = Sub { my_id: 1 }; }
            bus { publish K; }
            run() {
                K <- Ev { id: 1 } or discard;
                K <- Ev { id: 999 } or discard;
                println("done");
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("fail_or_discard", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("got id=1"), "got: {:?}", stdout);
    assert!(stdout.contains("done"), "got: {:?}", stdout);
    assert!(!stdout.contains("got id=999"), "got: {:?}", stdout);
}

/// Typecheck: publish to a fail topic without `or` is rejected.
#[test]
fn fail_topic_requires_or_disposition() {
    let src = r#"
        type Ev { id: Int; }
        topic K {
            payload: Ev; subject: "k"; keyed_by id;
            on_unmatched: fail;
        }
        main locus L {
            bus { publish K; }
            run() { K <- Ev { id: 1 }; }
        }
        fn main() { L { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), &program);
    let bundle = hale_types::Bundle { programs };
    let (scope, _) = hale_types::resolve::build_top_scope(&bundle);
    let diags = hale_types::check::check_bundle(&bundle, &scope, true);
    assert!(
        diags.iter().any(|d| {
            d.message.contains("on_unmatched: fail")
                && d.message.contains("or` disposition")
        }),
        "expected fail-requires-or diag, got: {:?}",
        diags
    );
}

/// Typecheck: `or` disposition on a non-fail topic is rejected.
#[test]
fn or_disposition_rejected_on_non_fail_topic() {
    let src = r#"
        type Ev { id: Int; }
        topic K { payload: Ev; subject: "k"; keyed_by id; }
        main locus L {
            bus { publish K; }
            run() { K <- Ev { id: 1 } or raise; }
        }
        fn main() { L { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), &program);
    let bundle = hale_types::Bundle { programs };
    let (scope, _) = hale_types::resolve::build_top_scope(&bundle);
    let diags = hale_types::check::check_bundle(&bundle, &scope, true);
    assert!(
        diags.iter().any(|d| {
            d.message.contains("only legal when the target topic \
                                declares")
                && d.message.contains("fail")
        }),
        "expected or-on-non-fail-topic diag, got: {:?}",
        diags
    );
}

/// v0.2 (2026-05-26): `or handler(err)` on a fail-topic publish.
/// The substitute expression runs on no-match with `err:
/// BusUnmatchedKey` in scope, so the handler can read the subject
/// + key and react (logging, metrics, etc.). The substitute's
/// value is discarded — Send is statement-level.
#[test]
fn fail_policy_or_handler_binds_unmatched_key_err() {
    let src = r#"
        type Ev { id: Int; }
        topic K {
            payload: Ev; subject: "k"; keyed_by id;
            on_unmatched: fail;
        }
        locus Sub {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { println("got id=", e.id); }
        }
        fn log_unmatched(err: BusUnmatchedKey) {
            println("unmatched subj=", err.subject,
                    " key_lo=", err.key_lo,
                    " key_hi=", err.key_hi);
        }
        main locus App {
            params { a: Sub = Sub { my_id: 1 }; }
            bus { publish K; }
            run() {
                K <- Ev { id: 1 } or log_unmatched(err);
                K <- Ev { id: 999 } or log_unmatched(err);
                K <- Ev { id: 42 } or log_unmatched(err);
                println("done");
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("fail_or_handler", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let handler_lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("unmatched "))
        .collect();
    assert_eq!(
        handler_lines.len(),
        2,
        "handler should fire on the 2 unmatched publishes; got {:?}",
        handler_lines
    );
    for ln in &handler_lines {
        assert!(
            ln.contains("subj=k "),
            "handler line missing subject: {}",
            ln
        );
    }
    let saw_999 = handler_lines.iter().any(|ln| ln.contains("key_lo=999 "));
    let saw_42 = handler_lines.iter().any(|ln| ln.contains("key_lo=42 "));
    assert!(saw_999, "missing key=999 in handler lines: {:?}", handler_lines);
    assert!(saw_42, "missing key=42 in handler lines: {:?}", handler_lines);
    assert!(stdout.contains("done"), "got: {:?}", stdout);
}

/// v0.2 (2026-05-26): `or fail <payload>` inside a fallible fn.
/// On no-match, the publish diverts through the fn's err path
/// using the constructed payload, instead of panicking.
#[test]
fn fail_policy_or_fail_propagates_to_enclosing_fn() {
    let src = r#"
        type Ev { id: Int; }
        type RouteErr { reason: String; }
        topic K {
            payload: Ev; subject: "k"; keyed_by id;
            on_unmatched: fail;
        }
        locus Sub {
            params { my_id: Int = 0; }
            bus { subscribe K as on_k where key == self.my_id; }
            fn on_k(e: Ev) { println("got id=", e.id); }
        }
        fn route(id: Int) -> () fallible(RouteErr) {
            K <- Ev { id: id } or fail RouteErr {
                reason: "no subscriber for sym " + err.subject,
            };
        }
        main locus App {
            params { a: Sub = Sub { my_id: 1 }; }
            bus { publish K; }
            run() {
                route(1) or println("err1: ", err.reason);
                route(999) or println("err2: ", err.reason);
                println("done");
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("fail_or_fail_propagates", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("err1:"),
        "route(1) matched; should not divert; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("err2: no subscriber for sym k"),
        "expected fail-divert with subject; got: {:?}",
        stdout
    );
    assert!(stdout.contains("done"), "got: {:?}", stdout);
}

/// Literal-key filter (no self.field involved). `where key == 1`
/// pins the subscriber to that specific value regardless of
/// instance state.
#[test]
fn keyed_subscribe_with_literal_key() {
    let src = r#"
        type Ev { id: Int; }
        topic K { payload: Ev; subject: "k"; keyed_by id; }
        locus Sub {
            params { tag: String = "?"; }
            bus { subscribe K as on_k where key == 42; }
            fn on_k(e: Ev) { println("sub.", self.tag, " id=", e.id); }
        }
        main locus App {
            params { s: Sub = Sub { tag: "a" }; }
            bus { publish K; }
            run() {
                K <- Ev { id: 1 };
                K <- Ev { id: 42 };
                K <- Ev { id: 100 };
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("literal_key", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| ln.starts_with("sub.a "))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "literal-key sub fired wrong number of times: {:?}",
        lines
    );
    assert!(lines[0].contains("id=42"), "got: {}", lines[0]);
}
