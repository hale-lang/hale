//! RFC #330 — `@effects(depends: {…})`, the backward dual of `causes:`.
//!
//! `causes:` exists because a call graph stops at a publish and the bus
//! graph continues. Nothing walked it the other way, so an
//! independence claim between two parts of a bus graph was
//! unenforceable: a dependence routed through one republishing
//! intermediary is invisible in every declaration on the depending
//! locus, whose `bus {}` block names only the innocent subject.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// The launderer: `Relay` republishes the compute topic under an
/// innocuous name, so `Carry` reads the computed magnitude while
/// naming only `Recalled`.
const LAUNDERED: &str = r#"
    type Act { mag: Float; pos: Int; }
    topic SumLookup   { payload: Act; }
    topic Recalled    { payload: Act; }
    topic VerbalCarry { payload: Act; }

    locus Relay {
        bus { subscribe SumLookup as on_sum; publish Recalled; }
        @effects(publish: {Recalled})
        fn on_sum(a: Act) { Recalled <- Act { mag: a.mag, pos: a.pos }; }
    }

    @effects(depends: {DECLARED})
    locus Carry {
        bus { subscribe Recalled as on_recalled; publish VerbalCarry; }
        params { recalled: Float = 0.0; }
        fn on_recalled(a: Act) { self.recalled = a.mag; }
        @effects(publish: {VerbalCarry})
        fn on_ask(p: Int) {
            VerbalCarry <- Act { mag: self.recalled, pos: p };
        }
    }

    locus Compute {
        bus { publish SumLookup; }
        fn go() { SumLookup <- Act { mag: 7.0, pos: 1 }; }
    }
    locus Sink {
        bus { subscribe VerbalCarry as on_c; }
        params { s: Float = 0.0; }
        fn on_c(a: Act) { self.s = a.mag; }
    }
    main locus App {
        params {
            r: Relay = Relay { }; c: Carry = Carry { };
            k: Compute = Compute { }; z: Sink = Sink { };
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn laundered_dependence_is_caught() {
    let ds = diags(&LAUNDERED.replace("DECLARED", "Recalled"));
    let hit = ds
        .iter()
        .find(|m| m.contains("declared dependency set violated"))
        .unwrap_or_else(|| {
            panic!("a dependence through a republisher must be caught: {:?}", ds)
        });
    assert!(
        hit.contains("SumLookup"),
        "the diagnostic must name the undeclared subject: {}",
        hit
    );
}

/// The verdict alone is nearly useless here — the whole point is that
/// the dependence is not visible locally, so the path is what makes
/// the finding actionable.
#[test]
fn the_diagnostic_names_the_path_through_the_bus() {
    let ds = diags(&LAUNDERED.replace("DECLARED", "Recalled"));
    let hit = ds
        .iter()
        .find(|m| m.contains("declared dependency set violated"))
        .expect("violation");
    assert!(
        hit.contains("Path:") && hit.contains("Relay"),
        "must name the intermediary that launders the value: {}",
        hit
    );
}

#[test]
fn declaring_the_full_closure_passes() {
    let ds = diags(&LAUNDERED.replace("DECLARED", "Recalled, SumLookup"));
    assert!(
        !ds.iter().any(|m| m.contains("dependency set violated")),
        "a complete declaration must be accepted: {:?}",
        ds
    );
}

/// Opt-in: a locus with no `depends:` is unconstrained. The measured
/// justification is that transitivity adds nothing beyond `bus {}` for
/// ~87% of loci in a real application, so a mandatory form would be
/// redundant far more often than informative.
#[test]
fn absent_declaration_constrains_nothing() {
    let src = LAUNDERED.replace("@effects(depends: {DECLARED})\n", "");
    let ds = diags(&src);
    assert!(
        !ds.iter().any(|m| m.contains("dependency set violated")),
        "no annotation must mean no constraint: {:?}",
        ds
    );
}

/// Dependence enters through subscriptions, which are declared
/// per-locus, so a fn-level `depends:` has nothing to mean. It is
/// rejected rather than parsed into silence — an annotation that
/// parses and does nothing is the failure this surface exists to
/// prevent, and it is exactly how the pre-v0.12.0 effect holes read.
#[test]
fn fn_level_depends_is_rejected_not_ignored() {
    let src = "topic A { payload: Int; }\n\
               @effects(depends: {A})\n\
               fn f() -> Int { return 1; }\n\
               fn main() { println(f()); }";
    let errs = parse_source(src).expect_err("must not parse");
    assert!(
        errs.iter().any(|e| e.message.contains("locus-level")),
        "the error should say where it belongs: {:?}",
        errs.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

/// A locus that reads nothing from the bus depends on nothing, and an
/// empty declaration is the way to say so.
#[test]
fn empty_declaration_is_satisfiable_by_a_pure_publisher() {
    let src = r#"
        type Act { mag: Float; }
        topic Out { payload: Act; }
        @effects(depends: {})
        locus Source {
            bus { publish Out; }
            fn go() { Out <- Act { mag: 1.0 }; }
        }
        locus Sink {
            bus { subscribe Out as on_o; }
            params { s: Float = 0.0; }
            fn on_o(a: Act) { self.s = a.mag; }
        }
        main locus App { params { s: Source = Source { }; k: Sink = Sink { }; } }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("dependency set violated")),
        "a locus with no subscriptions depends on nothing: {:?}",
        ds
    );
}

/// A stdlib I/O call must not withdraw a `depends:` certificate for
/// an application subject it provably cannot address.
///
/// `std::io::tcp::Stream` publishes per-op log events to a
/// RUNTIME-CHOSEN subject (`self.log_subject <- ...`), which is a
/// genuine unresolved publish — so any program calling `recv_bytes`
/// carried a publish hole. That hole was consulted program-globally:
/// "a publish whose subject the compiler could not name may address
/// a wire this locus subscribes to", for every subject in the
/// program. One socket read left every `depends:` declaration
/// uncertified, including on loci that touch no I/O (downstream
/// handoff).
///
/// But a computed publish is admitted only under a wildcard
/// declaration — `Stream` declares `publish "io.tcp.**"` — and the
/// publish site enforces it, so the hole cannot name `app.order`.
#[test]
fn a_stdlib_publish_hole_does_not_withdraw_an_unrelated_subject() {
    let src = r#"
        type Order { id: Int = 0; }
        topic OrderT { payload: Order; subject: "app.order"; }

        locus Gw {
            params { n: Int = 0; s: std::io::tcp::Stream = std::io::tcp::Stream { }; }
            bus { publish OrderT; }
            run() {
                let b = self.s.recv_bytes(16) or std::bytes::from_string("");
                self.n = len(b);
                OrderT <- Order { id: self.n };
            }
        }

        @effects(depends: { OrderT })
        locus Ledger {
            params { seen: Int = 0; }
            bus { subscribe OrderT as on_order; }
            fn on_order(o: Order) { self.seen = o.id; }
        }

        main locus App {
            params { g: Gw = Gw { }; l: Ledger = Ledger { }; }
        }
        fn main() { let a = App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|d| d.contains("dependency set cannot be certified")),
        "`app.order` is outside `io.tcp.**`, so the stdlib's publish \
         hole cannot name it: {:?}",
        ds
    );
}

/// The soundness half, and the reason the scoping is by PATTERN
/// rather than by "is it the stdlib": a subscription that really
/// does sit under the declared pattern must still refuse. Without
/// this, the fix above would be a fail-open.
#[test]
fn a_stdlib_publish_hole_still_withdraws_a_subject_it_can_address() {
    let src = r#"
        type Order { id: Int = 0; }
        topic OrderT { payload: Order; subject: "io.tcp.orders"; }

        locus Gw {
            params { n: Int = 0; s: std::io::tcp::Stream = std::io::tcp::Stream { }; }
            bus { publish OrderT; }
            run() {
                let b = self.s.recv_bytes(16) or std::bytes::from_string("");
                self.n = len(b);
                OrderT <- Order { id: self.n };
            }
        }

        @effects(depends: { OrderT })
        locus Ledger {
            params { seen: Int = 0; }
            bus { subscribe OrderT as on_order; }
            fn on_order(o: Order) { self.seen = o.id; }
        }

        main locus App {
            params { g: Gw = Gw { }; l: Ledger = Ledger { }; }
        }
        fn main() { let a = App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|d| d.contains("dependency set cannot be certified")),
        "`io.tcp.orders` IS under `io.tcp.**` — the hole can name it, \
         so the declaration must stay uncertified: {:?}",
        ds
    );
}
