//! #265 phase 1 — `@no_recursion` / `@no_ffi` / `@no_block`.
//!
//! Each assertion: a violating program errors with the WITNESS
//! CHAIN naming the path (the thing `@budget`'s fixpoint couldn't
//! produce), and a clean program passes.

fn diags_for(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn no_block_reports_the_call_chain() {
    let src = r#"
        fn nap() {
            std::time::sleep(50ms);
        }
        fn helper() {
            nap();
        }
        @no_block fn on_tick() {
            helper();
        }
        fn main() { on_tick(); }
    "#;
    let ds = diags_for(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("must not reach `block`"))
        .unwrap_or_else(|| panic!("expected a block-effect error; got {:?}", ds));
    // The witness chain, not just the fn name.
    assert!(
        hit.contains("on_tick -> helper -> nap"),
        "diagnostic must carry the call chain: {}",
        hit
    );
    assert!(hit.contains("sleep"), "and name the leaf: {}", hit);
}

#[test]
fn no_block_clean_program_passes() {
    let src = r#"
        fn pure_math(n: Int) -> Int {
            return n * 2 + 1;
        }
        @no_block fn on_tick() {
            let x = pure_math(3);
            println(x);
        }
        fn main() { on_tick(); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("must not reach `block`")),
        "clean fn must not trip @no_block: {:?}",
        ds
    );
}

#[test]
fn no_recursion_names_the_cycle() {
    let src = r#"
        fn ping(n: Int) -> Int {
            if n <= 0 { return 0; }
            return pong(n - 1);
        }
        fn pong(n: Int) -> Int {
            return ping(n - 1);
        }
        @no_recursion fn entry() -> Int {
            return ping(4);
        }
        fn main() { println(entry()); }
    "#;
    let ds = diags_for(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("reaches a recursive cycle"))
        .unwrap_or_else(|| panic!("expected a recursion error; got {:?}", ds));
    assert!(
        hit.contains("cycle:") && hit.contains("ping") && hit.contains("pong"),
        "must name the cycle members: {}",
        hit
    );
}

#[test]
fn no_recursion_acyclic_passes() {
    let src = r#"
        fn a(n: Int) -> Int { return n + 1; }
        fn b(n: Int) -> Int { return a(n) + a(n); }
        @no_recursion fn entry() -> Int { return b(2); }
        fn main() { println(entry()); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("recursive cycle")),
        "diamond (not a cycle) must pass: {:?}",
        ds
    );
}

#[test]
fn no_ffi_reports_the_chain_to_the_extern() {
    let src = r#"
        @ffi("c") fn c_helper(x: Int) -> Int;
        fn wrapper(x: Int) -> Int {
            return c_helper(x);
        }
        @no_ffi fn managed(x: Int) -> Int {
            return wrapper(x);
        }
        fn main() { println(managed(1)); }
    "#;
    let ds = diags_for(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("must not reach `ffi`"))
        .unwrap_or_else(|| panic!("expected an ffi error; got {:?}", ds));
    assert!(
        hit.contains("managed -> wrapper") && hit.contains("c_helper"),
        "must carry the chain to the extern: {}",
        hit
    );
}

#[test]
fn assertions_stack_with_each_other_and_with_hot() {
    let src = r#"
        fn pure_math(n: Int) -> Int { return n * 2; }
        @no_block @no_recursion @hot fn tick(n: Int) -> Int {
            return pure_math(n);
        }
        fn main() { println(tick(2)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "stacked clean assertions must pass: {:?}",
        ds
    );
}

#[test]
fn assertion_on_a_locus_method_is_checked() {
    let src = r#"
        type Ev { n: Int; }
        fn nap() { std::time::sleep(10ms); }
        locus H {
            bus { subscribe "e" as on_e of type Ev; }
            @no_block fn on_e(e: Ev) {
                nap();
            }
        }
        fn main() { H { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("must not reach `block`")
            && m.contains("H::on_e")),
        "method assertions must be checked and named: {:?}",
        ds
    );
}

// ---- #265 phase 2: registry-driven assertions ----

#[test]
fn no_syscall_reports_the_chain_to_the_io() {
    let src = r#"
        fn persist(path: String, body: String) {
            std::io::fs::write_file(path, body) or discard;
        }
        fn stage(body: String) {
            persist("/tmp/x", body);
        }
        @no_syscall fn compute(body: String) {
            stage(body);
        }
        fn main() { compute("hi"); }
    "#;
    let ds = diags_for(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("must not reach `syscall`"))
        .unwrap_or_else(|| panic!("expected a syscall-effect error; got {:?}", ds));
    assert!(
        hit.contains("compute -> stage -> persist"),
        "witness chain missing: {}",
        hit
    );
    assert!(hit.contains("write_file"), "leaf missing: {}", hit);
}

#[test]
fn no_syscall_pure_computation_passes() {
    let src = r#"
        fn scale(n: Int) -> Int { return n * 3; }
        @no_syscall fn compute(n: Int) -> Int {
            return scale(n) + std::math::float_to_int(2.0);
        }
        fn main() { println(compute(2)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("must not reach `syscall`")),
        "pure computation must pass: {:?}",
        ds
    );
}

#[test]
fn deterministic_rejects_clock_entropy_and_env() {
    for (call, what) in [
        ("std::time::monotonic_ns()", "clock"),
        ("std::rand::next_int(10)", "entropy"),
        ("std::env::args_count()", "env"),
    ] {
        let src = format!(
            r#"
            fn peek() -> Int {{ return {}; }}
            @deterministic fn decide() -> Int {{
                return peek();
            }}
            fn main() {{ println(decide()); }}
        "#,
            call
        );
        let ds = diags_for(&src);
        assert!(
            ds.iter().any(|m| m.contains("must not reach `time`") || m.contains("must not reach `entropy`") || m.contains("must not reach `env`")
                && m.contains("decide -> peek")),
            "{} read must violate @deterministic with a chain: {:?}",
            what,
            ds
        );
    }
}

#[test]
fn deterministic_pure_function_of_inputs_passes() {
    let src = r#"
        fn blend(a: Int, b: Int) -> Int { return a * 31 + b; }
        @deterministic fn decide(seed: Int, n: Int) -> Int {
            return blend(seed, n);
        }
        fn main() { println(decide(7, 2)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("must not reach `time`")),
        "a function of its inputs must pass: {:?}",
        ds
    );
}

/// `time_from_unix` FORMATS a caller-supplied instant — it reads no
/// clock, so it must not trip `@deterministic` (the classification
/// distinguishes reading the clock from formatting a given value).
#[test]
fn deterministic_allows_formatting_a_supplied_instant() {
    let src = r#"
        @deterministic fn render(at: Int) -> Time {
            return std::time::time_from_unix(at);
        }
        fn main() { println(render(0)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("must not reach `time`")),
        "formatting a supplied instant is deterministic: {:?}",
        ds
    );
}

/// The full hot-path certificate from the issue composes.
#[test]
fn full_certificate_composes() {
    let src = r#"
        fn blend(a: Int, b: Int) -> Int { return a * 31 + b; }
        @no_block @no_syscall @deterministic @no_recursion @hot
        @budget(alloc_per_call = 0)
        fn on_tick(a: Int, b: Int) -> Int {
            return blend(a, b);
        }
        fn main() { println(on_tick(1, 2)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("violated") || m.contains("budget")),
        "the stacked certificate must pass on a clean fn: {:?}",
        ds
    );
}

// ---- the general @effects(...) form + the new classes ----

/// The general form expresses contracts the sugar can't name: "no
/// clock, but entropy is fine" (a jittered retry, a fuzzer).
#[test]
fn general_form_expresses_partial_determinism() {
    let src = r#"
        fn jitter() -> Int { return std::rand::next_int(10); }
        fn stamp() -> Int { return std::time::monotonic_ns(); }
        @effects(none: {time}) fn backoff() -> Int {
            return jitter();
        }
        @effects(none: {time}) fn bad() -> Int {
            return stamp();
        }
        fn main() { println(backoff() + bad()); }
    "#;
    let ds = diags_for(src);
    // entropy is allowed under `none: {time}` …
    assert!(
        !ds.iter().any(|m| m.contains("`backoff`")),
        "entropy must be allowed when only time is forbidden: {:?}",
        ds
    );
    // … but the clock read is not.
    assert!(
        ds.iter().any(|m| m.contains("`bad`")
            && m.contains("must not reach `time`")),
        "clock read must violate none: {{time}}: {:?}",
        ds
    );
}

/// The sugar IS the general form — `@no_block` and
/// `@effects(none: {block})` must produce the same diagnostic.
#[test]
fn sugar_and_general_form_agree() {
    let mk = |ann: &str| {
        format!(
            r#"
            fn nap() {{ std::time::sleep(5ms); }}
            {} fn h() {{ nap(); }}
            fn main() {{ h(); }}
        "#,
            ann
        )
    };
    let a = diags_for(&mk("@no_block"));
    let b = diags_for(&mk("@effects(none: {block})"));
    assert_eq!(a, b, "sugar must desugar to the general form exactly");
    assert!(a.iter().any(|m| m.contains("must not reach `block`")), "{:?}", a);
}

/// #265: publishes are syntactic (`Topic <- v`), so the effect
/// engine records them as sites rather than call edges.
#[test]
fn no_publish_catches_a_syntactic_send() {
    let src = r#"
        type Ev { n: Int; }
        topic T { payload: Ev; subject: "t"; }
        locus P {
            bus { publish T; }
            fn emit(n: Int) {
                T <- Ev { n: n };
            }
            @no_publish fn compute(n: Int) {
                self.emit(n);
            }
        }
        fn main() { P { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("must not reach")
            || m.contains("publishes to")),
        "a transitive publish must violate @no_publish: {:?}",
        ds
    );
}

/// The issue's headline positive form: an allowed publish set.
#[test]
fn publish_set_allows_declared_and_rejects_others() {
    let src = r#"
        type Ev { n: Int; }
        topic Ok { payload: Ev; subject: "ok"; }
        topic Nope { payload: Ev; subject: "nope"; }
        locus P {
            bus { publish Ok; publish Nope; }
            @effects(publish: {Ok}) fn good(n: Int) {
                Ok <- Ev { n: n };
            }
            @effects(publish: {Ok}) fn bad(n: Int) {
                Nope <- Ev { n: n };
            }
        }
        fn main() { P { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("`P::good`")),
        "a publish inside the declared set must pass: {:?}",
        ds
    );
    assert!(
        ds.iter().any(|m| m.contains("`P::bad`")
            && m.contains("publish set violated")
            && m.contains("Nope")),
        "a publish outside the set must be reported: {:?}",
        ds
    );
}

/// Locus instantiation is an effect (arena create + possibly a
/// thread/pool post) — the Hale-specific class.
#[test]
fn no_spawn_catches_locus_instantiation() {
    let src = r#"
        locus Child { params { n: Int = 0; } }
        fn make(n: Int) { Child { n: n }; }
        @no_spawn fn handler(n: Int) { make(n); }
        fn main() { handler(1); }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("instantiates locus `Child`")),
        "a transitive locus instantiation must violate @no_spawn: {:?}",
        ds
    );
}

/// #265: **placement-implied contracts** — the check that needs no
/// annotation. A handler on a `where async_io` pool that reaches a
/// blocking call stalls every other locus on that pool; the
/// placement is the assertion, so the compiler says so unprompted.
/// (This is Crumb batch-5's bug as a compile-time finding.)
///
/// The leaf is `std::io::stdin::read_line` and not `sleep`: GH #791
/// took `sleep` out of this advisory's leaf set (it PARKS on an
/// async_io pool), so a sleeping handler is no longer a positive
/// control for it.
#[test]
fn async_io_placement_warns_about_blocking_without_annotation() {
    let src = r#"
        type Ev { n: Int; }
        locus Worker {
            bus { subscribe "e" as on_e of type Ev; }
            fn on_e(e: Ev) {
                println(std::io::stdin::read_line());
            }
        }
        main locus App {
            params { w: Worker = Worker { }; }
            placement { w: cooperative(pool = web) where async_io; }
            run() { std::time::sleep(10ms); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("async_io pool `web`")
            && m.contains("stalls every other locus")),
        "an unannotated blocking handler on an async_io pool must be \
         flagged by placement alone: {:?}",
        ds
    );
}

/// …and an explicit assertion means the author is engaged: the
/// enforced error replaces the advisory warning (no double report).
#[test]
fn explicit_assertion_suppresses_the_placement_advisory() {
    let src = r#"
        type Ev { n: Int; }
        locus Worker {
            bus { subscribe "e" as on_e of type Ev; }
            @no_block fn on_e(e: Ev) {
                println(std::io::stdin::read_line());
            }
        }
        main locus App {
            params { w: Worker = Worker { }; }
            placement { w: cooperative(pool = web) where async_io; }
            run() { std::time::sleep(10ms); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    let advisories =
        ds.iter().filter(|m| m.contains("stalls every other locus")).count();
    assert_eq!(advisories, 0, "annotated fn must not also warn: {:?}", ds);
    assert!(
        ds.iter().any(|m| m.contains("must not reach `block`")),
        "the explicit assertion must still error: {:?}",
        ds
    );
}

// ---- GH #791: the advisory knows what parks on an async_io pool ----

/// GH #791 (the regression). `std::time::sleep` on an async_io pool
/// PARKS — PR #285's timer-only park swaps the coro out and the
/// worker goes on draining — so it stalls nothing and the advisory
/// must be silent. Before the fix every async handler that sleeps
/// carried this warning, and its suggested fix (`@no_block`) is a
/// compile error on the very same program.
#[test]
fn async_io_placement_is_silent_about_a_parking_sleep() {
    let src = r#"
        type Ev { n: Int; }
        locus Worker {
            bus { subscribe "e" as on_e of type Ev; }
            fn on_e(e: Ev) {
                std::time::sleep(400ms);
            }
        }
        main locus App {
            params { w: Worker = Worker { }; }
            placement { w: cooperative(pool = web) where async_io; }
            run() { std::time::sleep(10ms); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("stalls every other locus")),
        "a handler that only parks must not be flagged: {:?}",
        ds
    );
}

/// The park exemption is per-LEAF, not per-handler: a handler that
/// sleeps *and* blocks is still reported, and the witness path names
/// the leaf that actually holds the worker.
#[test]
fn async_io_placement_names_the_blocking_leaf_past_a_park() {
    let src = r#"
        type Ev { n: Int; }
        locus Worker {
            bus { subscribe "e" as on_e of type Ev; }
            fn on_e(e: Ev) {
                std::time::sleep(400ms);
                println(std::io::stdin::read_line());
            }
        }
        main locus App {
            params { w: Worker = Worker { }; }
            placement { w: cooperative(pool = web) where async_io; }
            run() { std::time::sleep(10ms); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("stalls every other locus"))
        .unwrap_or_else(|| panic!("expected the advisory; got {:?}", ds));
    assert!(
        hit.contains("read_line"),
        "the witness must name the leaf that holds the worker: {}",
        hit
    );
    assert!(
        !hit.contains("time::sleep"),
        "…and not the one that parks: {}",
        hit
    );
}

/// The decision GH #791 had to make, pinned. `block` stays a
/// property of the CALL, not of a placement: the same `sleep` on a
/// classic pool really does hold that pool's OS thread, a locus type
/// can be placed per-instance (F.31) on an async_io pool for one
/// field and a classic one for another, and a free fn has no
/// placement at all — so a fn-grained certificate cannot be
/// placement-conditional without becoming ambiguous. `@no_block`
/// therefore still refuses a sleeping handler. The two no longer
/// contradict each other because the advisory no longer *suggests*
/// `@no_block` here: it says nothing at all.
#[test]
fn no_block_still_refuses_a_parking_leaf() {
    let src = r#"
        type Ev { n: Int; }
        locus Worker {
            bus { subscribe "e" as on_e of type Ev; }
            @no_block fn on_e(e: Ev) {
                std::time::sleep(400ms);
            }
        }
        main locus App {
            params { w: Worker = Worker { }; }
            placement { w: cooperative(pool = web) where async_io; }
            run() { std::time::sleep(10ms); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("must not reach `block`")),
        "an explicit @no_block is placement-independent and must still \
         report the sleep: {:?}",
        ds
    );
}

/// The classic-pool half is untouched, and the park exemption is
/// scoped to the async_io advisory alone.
/// `check_cooperative_pool_blocking` keeps its own (narrower)
/// blocking set and its own message: `tcp::recv_into` parks on an
/// async_io pool — it is in the park list — and on a pool that is NOT
/// `where async_io` it holds the pool's OS thread and still warns.
/// `sleep` was never in that set (an event-driven subscriber's sleep
/// loop is how it yields), on any pool, including `main`.
#[test]
fn classic_pool_blocking_warning_is_unchanged() {
    let src = r#"
        locus Worker {
            params {
                fd: Int = 0;
                buf: std::bytes::BytesBuilder =
                    std::bytes::BytesBuilder { initial_cap: 4096 };
            }
            run() {
                let got = std::io::tcp::recv_into(self.fd, self.buf, 2048);
            }
        }
        locus Napper {
            run() {
                std::time::sleep(400ms);
            }
        }
        main locus App {
            params {
                w: Worker = Worker { };
                n: Napper = Napper { };
            }
            placement {
                w: cooperative(pool = web);
                n: cooperative(pool = web);
            }
            run() { std::time::sleep(10ms); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("`std::io::tcp::recv_into`")
            && m.contains("stalling every other locus scheduled on `web`")),
        "a blocking run() on a classic cooperative pool must still \
         warn: {:?}",
        ds
    );
    assert!(
        !ds.iter().any(|m| m.contains("`std::time::sleep`")),
        "…and sleep must stay out of that set, on every pool: {:?}",
        ds
    );
}

/// The park list is a claim about the stdlib frontier, so it has to
/// stay attached to it: every path in it must still be a registry
/// row, and must still carry `block` (a row that lost the bit would
/// make its entry dead weight, silently).
#[test]
fn every_parking_path_is_a_classified_blocking_row() {
    use hale_types::stdlib_surface::{
        effects_for, EffectSet, ASYNC_IO_PARKING,
    };
    for path in ASYNC_IO_PARKING {
        let eff = effects_for(path).unwrap_or_else(|| {
            panic!("{} parks but has no registry row", path.join("::"))
        });
        assert!(
            eff.contains(EffectSet::BLOCK),
            "{} is in the park list but no longer carries `block`",
            path.join("::")
        );
    }
}

// ---- @no_panic: disposition coverage, not leaf reachability ----

#[test]
fn no_panic_flags_an_explicit_violate() {
    let src = r#"
        @no_panic fn risky(n: Int) -> Int {
            if n < 0 { violate bad_input; }
            return n;
        }
        fn main() { println(risky(1)); }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("@no_panic` violated")
            && m.contains("violate")),
        "an explicit violate must trip @no_panic: {:?}",
        ds
    );
}

#[test]
fn no_panic_flags_or_raise() {
    let src = r#"
        @no_panic fn readit(p: String) -> String {
            return std::io::fs::read_file(p) or raise;
        }
        fn main() { println(readit("/tmp/x")); }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("@no_panic` violated")
            && m.contains("or raise")),
        "`or raise` propagates — it must trip @no_panic: {:?}",
        ds
    );
}

#[test]
fn no_panic_accepts_handled_dispositions() {
    let src = r#"
        @no_panic fn readit(p: String) -> String {
            return std::io::fs::read_file(p) or "";
        }
        fn main() { println(readit("/tmp/x")); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("@no_panic")),
        "a substitute disposition handles the failure: {:?}",
        ds
    );
}

/// A publish set must be able to name a QUALIFIED topic (downstream
/// FRICTION). Topics shared between binaries live in a central
/// catalog and are imported under an alias, so accepting only bare
/// idents meant the contract worth having most — "this binary is the
/// only one permitted to publish X" — was the one it could not
/// state. Two halves had to agree: the parser must accept
/// `t::Name` in the set, and the publish SITE must record a
/// qualified subject instead of writing it off as computed.
#[test]
fn publish_set_accepts_and_enforces_a_qualified_topic() {
    let src = r#"
        type Order { n: Int; }
        topic SharedTopic { payload: Order; subject: "shared.topic"; }
        locus Api {
            bus { publish SharedTopic; }
            @effects(publish: {})
            fn go() { SharedTopic <- Order { n: 1 }; }
        }
        locus S { bus { subscribe SharedTopic as on_o; } fn on_o(o: Order) { } }
        main locus App { params { a: Api = Api { }; s: S = S { }; } }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("declared publish set violated")),
        "an undeclared publish must violate: {:?}",
        ds
    );
}

/// The parser half on its own: a qualified name in any `@effects`
/// set must parse. It used to die on the `::`.
#[test]
fn qualified_names_parse_inside_an_effects_set() {
    let src = "@effects(publish: {t::SharedTopic, LocalTopic})\n\
               fn go() -> Int { return 1; }\nfn main() { println(go()); }";
    assert!(
        hale_syntax::parse_source(src).is_ok(),
        "a qualified topic name must parse in a publish set"
    );
}

// === GH #723: decorator stacks =====================================
//
// `@unbounded` and an effect assertion are orthogonal contracts, and
// applying both to one fn used to fail at SYNTAX in either order — so a
// `@unbounded` method needed a free `@no_syscall` wrapper to carry both
// (downstream handoff). Stacking them must weaken neither half, and a
// stack that contradicts itself must say so at the decorator.

fn full_diags(src: &str) -> Vec<hale_syntax::Diag> {
    let program = hale_syntax::parse_source(src).expect("parse");
    hale_types::check_program(&program)
}

/// The assertion half survives the stack: a syscall three frames down
/// still fails, with the witness path.
#[test]
fn stacked_with_unbounded_the_assertion_still_reports_the_witness_path() {
    let src = r#"
        fn deep() { println("a syscall, three frames down"); }
        fn middle() { deep(); }
        @unbounded
        @no_syscall
        fn entry() { middle(); }
        fn main() { entry(); }
    "#;
    let ds = diags_for(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("must not reach `syscall`"))
        .unwrap_or_else(|| panic!("expected a syscall-effect error; got {:?}", ds));
    assert!(
        hit.contains("entry -> middle -> deep"),
        "the witness chain must survive the stack: {}",
        hit
    );
}

/// …and in the other order, which failed to parse symmetrically.
#[test]
fn assertion_before_unbounded_is_enforced_too() {
    let src = r#"
        fn deep() { println("a syscall"); }
        @no_syscall
        @unbounded
        fn entry() { deep(); }
        fn main() { entry(); }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("must not reach `syscall`")),
        "assertion-then-unbounded must still be enforced: {:?}",
        ds
    );
}

/// The `@unbounded` half survives too — and only `@unbounded`
/// suppresses the allocation advisory. The same program twice: the
/// assertion alone warns, the stack does not.
#[test]
fn only_unbounded_suppresses_the_allocation_advisory_in_a_stack() {
    let body = r#"
        locus Thing { params { n: Int = 0; } fn v() -> Int { return self.n; } }
        %DECORATORS%
        fn build(n: Int) -> Int {
            let mut total = 0;
            let mut i = 0;
            while i < n {
                let t = Thing { n: i };
                total = total + t.v();
                i = i + 1;
            }
            return total;
        }
        main locus App { run() { println(build(3)); } }
        fn main() { App { }; }
    "#;
    let advisory = |decorators: &str| -> bool {
        diags_for(&body.replace("%DECORATORS%", decorators))
            .iter()
            .any(|m| m.contains("hot-path allocation"))
    };
    assert!(
        advisory("@no_syscall"),
        "the assertion alone must not silence the advisory"
    );
    assert!(
        !advisory("@unbounded\n@no_syscall"),
        "`@unbounded` in the stack must silence the advisory"
    );
    assert!(
        !advisory("@no_syscall\n@unbounded"),
        "…in either order"
    );
}

/// A stack that contradicts itself is a check-time error pointing AT
/// the decorator, not a parse failure. `@unbounded` acknowledges an
/// allocation without a static bound; `@hot`, an assertion forbidding
/// `alloc`, and `@budget(alloc_per_call = 0)` each say there is none.
#[test]
fn contradictory_decorators_are_diagnosed_at_the_decorator() {
    let cases: [(&str, &str); 3] = [
        ("@unbounded\n@hot\n", "`@unbounded` and `@hot` contradict"),
        (
            "@unbounded\n@effects(none: { alloc })\n",
            "an effect assertion that forbids `alloc` contradict",
        ),
        (
            "@unbounded\n@budget(alloc_per_call = 0)\n",
            "`@budget(alloc_per_call = 0)` contradict",
        ),
    ];
    for (decorators, needle) in cases {
        let src = format!(
            "{}fn grow(n: Int) -> Int {{ return n; }}\n\
             fn main() {{ println(grow(1)); }}",
            decorators
        );
        let ds = full_diags(&src);
        let hit = ds
            .iter()
            .find(|d| d.message.contains(needle))
            .unwrap_or_else(|| {
                panic!(
                    "expected {:?} for {:?}; got {:?}",
                    needle,
                    decorators,
                    ds.iter().map(|d| &d.message).collect::<Vec<_>>()
                )
            });
        assert!(
            src[hit.span.start.as_usize()..].starts_with("@unbounded"),
            "the diagnostic must point at the decorator, got {:?}",
            &src[hit.span.start.as_usize()..]
        );
        assert!(
            !hit.related.is_empty(),
            "and name the decorator it contradicts: {}",
            hit.message
        );
    }
}

/// A ceiling ABOVE zero is not a contradiction: bounded per call,
/// deliberately unbounded in aggregate (a cache insert per call).
#[test]
fn a_nonzero_budget_stacks_with_unbounded() {
    let src = r#"
        @unbounded
        @budget(alloc_per_call = 2)
        fn grow(n: Int) -> Int { return n; }
        fn main() { println(grow(1)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("contradict")),
        "a nonzero per-call ceiling must compose with `@unbounded`: {:?}",
        ds
    );
}

/// The same decorator twice is diagnosed at the SECOND one, with the
/// first as the related location — including when another decorator
/// sits between them (the run is one stack, not two).
#[test]
fn duplicate_decorators_are_diagnosed() {
    let src = "@unbounded\n@unbounded\nfn grow(n: Int) -> Int { return n; }\n\
               fn main() { println(grow(1)); }";
    let ds = full_diags(src);
    let hit = ds
        .iter()
        .find(|d| d.message.contains("duplicate `@unbounded`"))
        .unwrap_or_else(|| {
            panic!(
                "expected a duplicate diagnostic; got {:?}",
                ds.iter().map(|d| &d.message).collect::<Vec<_>>()
            )
        });
    let (line, _) = hit.span.line_col(src);
    assert_eq!(line, 2, "points at the second `@unbounded`: {}", hit.message);
    assert!(!hit.related.is_empty(), "with the first as related");

    let split = "@no_syscall\n@unbounded\n@no_syscall\n\
                 fn grow(n: Int) -> Int { return n; }\n\
                 fn main() { println(grow(1)); }";
    assert!(
        full_diags(split)
            .iter()
            .any(|d| d.message.contains("duplicate `@no_syscall`")),
        "a repeat across an intervening decorator is still a repeat"
    );
}
