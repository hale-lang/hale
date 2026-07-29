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
        .find(|m| m.contains("@no_block` violated"))
        .unwrap_or_else(|| panic!("expected a @no_block error; got {:?}", ds));
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
        !ds.iter().any(|m| m.contains("@no_block")),
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
        .find(|m| m.contains("@no_recursion` violated"))
        .unwrap_or_else(|| panic!("expected a @no_recursion error; got {:?}", ds));
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
        !ds.iter().any(|m| m.contains("@no_recursion")),
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
        .find(|m| m.contains("@no_ffi` violated"))
        .unwrap_or_else(|| panic!("expected a @no_ffi error; got {:?}", ds));
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
        ds.iter().any(|m| m.contains("@no_block` violated")
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
        .find(|m| m.contains("@no_syscall` violated"))
        .unwrap_or_else(|| panic!("expected @no_syscall error; got {:?}", ds));
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
        !ds.iter().any(|m| m.contains("@no_syscall")),
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
            ds.iter().any(|m| m.contains("@deterministic` violated")
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
        !ds.iter().any(|m| m.contains("@deterministic")),
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
        !ds.iter().any(|m| m.contains("@deterministic")),
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
