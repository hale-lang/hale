//! GH #436 follow-up: the negative-control matrix for the confinement
//! and claim fail-opens found in review of the landed work.
//!
//! Every test here was verified to FAIL against the merge commit
//! `d9b0965` before its fix landed. That ordering matters: the point
//! of this file is that each defect was reproduced first, not asserted
//! after the fact against code already changed to satisfy it.
//!
//! The defects share one shape — a stated guarantee that the
//! implementation did not make true:
//!
//!   * `@sealed` stopped reads and permitted writes, so outside code
//!     could REPLACE a confined key rather than read it;
//!   * an unavailable credential authenticated the empty candidate;
//!   * the stdlib's `secret_use` had no identity the application's
//!     claims could name, so the law over `std::secret` was silently
//!     unenforceable;
//!   * `require attributed` reported `holds` for classes its
//!     evaluator never implemented.

use hale_syntax::parse_source;

fn errors(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

// ---------------------------------------------------------------
// P0: `@sealed` must stop WRITES, not only reads.
// ---------------------------------------------------------------

const VAULT: &str = r#"
    @sealed locus Vault {
        params { key: Int = 0; }
        fn use1(m: Int) -> Int { return m + self.key; }
    }
"#;

fn attacker(body: &str) -> String {
    format!(
        "{VAULT}
         locus Attacker {{
             params {{ v: Vault = Vault {{ }}; }}
             {body}
         }}
         main locus App {{ params {{ a: Attacker = Attacker {{ }}; }} }}
         fn main() {{ App {{ }}; }}"
    )
}

#[test]
fn an_outside_write_to_a_sealed_param_is_rejected() {
    // Confinement that stops reads but permits writes is not
    // confinement: for `std::secret` it lets outside code CHOOSE the
    // signing key, which is worse than reading it.
    let es = errors(&attacker("fn replace() { self.v.key = 999; }"));
    assert!(
        es.iter().any(|m| m.contains("`@sealed`")),
        "an outside write must be rejected, got {es:?}"
    );
}

#[test]
fn an_outside_write_is_rejected_by_the_TARGET_not_the_rhs() {
    // Deliberately a clean RHS. `self.v.key = self.v.key;` would pass
    // this test on the pre-fix build, because the READ on the right
    // trips the expression-path check — the target would never be
    // examined and the test would prove nothing.
    let es = errors(&attacker("fn bump(n: Int) { self.v.key = n; }"));
    assert!(
        es.iter().any(|m| m.contains("`@sealed`")),
        "the assignment TARGET must be checked, got {es:?}"
    );
}

#[test]
fn the_locus_still_writes_its_own_params() {
    let src = "
        @sealed locus Vault {
            params { key: Int = 0; }
            fn rotate(k: Int) { self.key = k; }
        }
        main locus App { params { v: Vault = Vault { }; } }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn construction_still_initializes() {
    // A parent writing `Vault { key: … }` already holds what it
    // passes; it is not an LValue mutation of an existing locus, and
    // restricting it would cost ordinary configuration for nothing.
    let src = "
        @sealed locus Vault {
            params { key: Int = 0; }
            fn use1(m: Int) -> Int { return m + self.key; }
        }
        main locus App { params { v: Vault = Vault { key: 5 }; } }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

// ---------------------------------------------------------------
// P0: the stdlib effect class needs an identity claims can name.
// ---------------------------------------------------------------

#[test]
fn a_claim_over_secret_use_sees_the_stdlib_signer() {
    // THE defect that mattered most: `std::secret` was documented as
    // the recommended path and had no enforceable law over it. User
    // effect classes intern per-`Program`, so the stdlib's
    // `secret_use` and the application's were different bits.
    let src = "
        locus Plugin {
            params {
                s: std::secret::Signer =
                    std::secret::Signer { env_var: \"SK\" };
            }
            fn sneak(m: Bytes) -> Bytes { return self.s.sign(m); }
        }
        group plugins = { Plugin };
        main locus App {
            params { p: Plugin = Plugin { }; }
            claims {
                blocked: forbid reaches(plugins, effects(secret_use));
            }
        }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("blocked") && m.contains("violated")),
        "a plugin reaching the stdlib signer must violate: {es:?}"
    );
}

#[test]
fn secret_use_identity_survives_unrelated_effect_declarations() {
    // The aliasing canary: with the application declaring its own
    // classes first, an index-based identity would have the stdlib's
    // bit mean whatever the application declared at that index.
    let src = "
        effect unrelated_one;
        effect unrelated_two;
        locus Plugin {
            params {
                s: std::secret::Signer =
                    std::secret::Signer { env_var: \"SK\" };
            }
            fn sneak(m: Bytes) -> Bytes { return self.s.sign(m); }
        }
        group plugins = { Plugin };
        main locus App {
            params { p: Plugin = Plugin { }; }
            claims {
                blocked: forbid reaches(plugins, effects(secret_use));
                unrelated_is_clean:
                    forbid reaches(plugins, effects(unrelated_one));
            }
        }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("blocked") && m.contains("violated")),
        "`secret_use` must still match regardless of declaration \
         order: {es:?}"
    );
    assert!(
        !es.iter().any(|m| m.contains("unrelated_is_clean")),
        "an unrelated class must NOT alias onto signing: {es:?}"
    );
}

// ---------------------------------------------------------------
// P1: `require attributed` must not hold for classes it cannot check.
// ---------------------------------------------------------------

fn attributed(class: &str) -> String {
    format!(
        "locus L {{
             params {{ n: Int = 0; }}
             fn f() -> Int {{ return 1; }}
         }}
         main locus App {{
             params {{ l: L = L {{ }}; }}
             claims {{ c: require attributed(all {class}); }}
         }}
         fn main() {{ App {{ }}; }}"
    )
}

#[test]
fn structural_classes_are_rejected_rather_than_holding_vacuously() {
    // `ffi` / `spawn` / `recursion` have no registry mask, so the
    // evaluator answered every one with unconditional success while
    // reading like a security baseline.
    for class in ["ffi", "spawn", "recursion"] {
        let es = errors(&attributed(class));
        assert!(
            !es.is_empty(),
            "`require attributed(all {class})` must be rejected, not \
             silently satisfied"
        );
    }
}

#[test]
fn a_direct_allocation_is_visible_to_attributed_alloc() {
    // The evaluator mapped `alloc` to its mask but only ever looked
    // at call edges and publish syntax — never at allocation sites,
    // which is exactly where a direct allocation is recorded.
    let src = "
        type Rec { v: Int; }
        locus L {
            params { n: Int = 0; }
            fn make() -> Int { let r = Rec { v: 1 }; return r.v; }
        }
        main locus App {
            params { l: L = L { }; }
            claims { c: require attributed(all alloc); }
        }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("L::make")),
        "a direct allocation must need attribution: {es:?}"
    );
}

// ---------------------------------------------------------------
// P1: the universals must see declarations inside modules.
// ---------------------------------------------------------------

#[test]
fn require_sealed_sees_a_SEALED_locus_inside_a_module() {
    // The failing direction is the false POSITIVE, not the missed
    // violation: `sealed_loci_of` was top-level-only, so a sealed
    // locus inside a module read as unsealed and the claim reported
    // a violation that is not real. An unsealed-in-module test would
    // pass on the broken build and prove nothing.
    let src = "
        module secrets {
            @sealed locus Vault { params { k: Int = 1; }
                fn f() -> Int { return self.k; } }
        }
        group vaults = { Vault };
        main locus App {
            params { v: Vault = Vault { }; }
            claims { confined: require sealed(all vaults); }
        }
        fn main() { App { }; }
    ";
    assert!(
        errors(src).is_empty(),
        "a sealed locus in a module must satisfy the claim: {:?}",
        errors(src)
    );
}

// ---------------------------------------------------------------
// P1/P2: `require sealed` must not hold over a group with no loci.
// ---------------------------------------------------------------

#[test]
fn require_sealed_over_a_free_fn_group_is_rejected() {
    // The claims system's vacuity discipline: a universal over an
    // empty projection reported `holds`, which is a fail-open wearing
    // formal clothing.
    let src = "
        fn helper() -> Int { return 1; }
        group helpers = { helper };
        main locus App {
            params { n: Int = 0; }
            claims { confined: require sealed(all helpers); }
        }
        fn main() { App { }; }
    ";
    assert!(
        !errors(src).is_empty(),
        "a locus-free group must be rejected, not vacuously satisfied"
    );
}

// ---------------------------------------------------------------
// P1: `--strict-secret` must actually be exhaustive.
// ---------------------------------------------------------------

fn strict(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::frontier::secret_taint_strict(&[&program])
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn strict_sees_a_tuple_destructuring_alias() {
    // `expr_mentions` had a catch-all arm listing six expression
    // forms, so a tuple — and an index, a block tail, an `or`
    // substitute, anything else — carried a secret straight past. It
    // is now exhaustive by construction: no `_` arm, so a new `Expr`
    // variant fails the build rather than opening a laundering route.
    let src = "
        type Msg { v: String; }
        topic Out { payload: Msg; subject: \"app.out\"; }
        locus S {
            params { n: Int = 0; }
            bus { publish Out; }
            fn f(@secret t: String) {
                let (a, b) = (t, 0);
                Out <- Msg { v: a };
            }
        }
        locus K { params { n: Int = 0; }
            bus { subscribe Out as on_out; }
            fn on_out(m: Msg) { self.n = len(m.v); } }
        main locus App {
            params { s: S = S { }; k: K = K { }; }
        }
        fn main() { App { }; }
    ";
    let ms = strict(src);
    assert!(
        ms.iter().any(|m| m.contains("bus publish")),
        "a tuple-destructured alias must not launder: {ms:?}"
    );
}

#[test]
fn strict_sees_a_match_arm_expression() {
    // Arm bodies were walked only when they were BLOCKS, so
    // `match k { 0 -> print(secret), … }` was skipped entirely.
    let src = "
        locus S {
            params { n: Int = 0; }
            fn f(@secret t: String, k: Int) -> Int {
                match k {
                    0 -> len(t),
                    _ -> 0,
                }
            }
        }
        main locus App { params { s: S = S { }; } }
        fn main() { App { }; }
    ";
    let ms = strict(src);
    assert!(
        !ms.is_empty(),
        "an expression arm must be walked, not skipped: {ms:?}"
    );
}

// ---------------------------------------------------------------
// P1: sealing must be visible in the artifact.
// ---------------------------------------------------------------

#[test]
fn sealing_a_locus_moves_the_shape_hash() {
    // A seal changing with no topology diff is precisely the
    // invisible security change the artifact exists to surface.
    // Sealing was a claim INPUT that no model row recorded, so
    // `shape_hash` was byte-identical either way.
    fn hash_of(src: &str) -> String {
        let p = parse_source(src).expect("parse");
        let mut m = std::collections::BTreeMap::new();
        m.insert(String::new(), &p);
        let art = hale_types::topology::dump_topology(
            &hale_types::Bundle::new(m),
        );
        let v: serde_json::Value =
            serde_json::from_str(&art).expect("artifact is json");
        v["shape_hash"].as_str().unwrap().to_string()
    }
    let plain = "
        locus Vault { params { k: Int = 1; }
            fn f() -> Int { return self.k; } }
        main locus App { params { v: Vault = Vault { }; } }
        fn main() { App { }; }
    ";
    let sealed = plain.replace("locus Vault", "@sealed locus Vault");
    assert_ne!(
        hash_of(plain),
        hash_of(&sealed),
        "sealing must change the model identity"
    );
}

// ---------------------------------------------------------------
// P1: attribution must not depend on how an API happens to be
// implemented.
// ---------------------------------------------------------------

#[test]
fn attribution_crosses_a_resolved_stdlib_boundary() {
    // The evaluator ignored EVERY resolved call, so a boundary
    // reached through a Hale-source stdlib body was invisible while
    // the same operation written as a frontier path call was caught.
    // Whether an API is a path call or a locus method is not a
    // stable semantic boundary to hang a security claim on.
    let src = "
        locus L {
            params {
                lg: std::log::Logger = std::log::Logger { name: \"app\" };
            }
            fn via_handle(m: String) { self.lg.info(m); }
        }
        main locus App {
            params { l: L = L { }; }
            claims { c: require attributed(all publish); }
        }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("L::via_handle")),
        "a publish through a stdlib handle must need attribution: {es:?}"
    );
}

#[test]
fn a_bundle_callee_is_still_judged_on_its_own_row() {
    // The counterweight: crossing into code the author does NOT own
    // attributes the caller, but an ordinary application callee is
    // judged where its own site is. Without this, attribution would
    // become transitive and nearly vacuous.
    let src = "
        effect audit;
        locus Raw {
            params { n: Int = 0; }
            @effects(is: { audit })
            fn io(s: String) { std::io::fs::write_file(\"/tmp/x\", s); }
        }
        locus Wrapper {
            params { r: Raw = Raw { }; }
            fn go(s: String) { self.r.io(s); }
        }
        main locus App {
            params { w: Wrapper = Wrapper { }; }
            claims { c: require attributed(all syscall); }
        }
        fn main() { App { }; }
    ";
    assert!(
        errors(src).is_empty(),
        "a wrapper over an attributed bundle fn owes nothing: {:?}",
        errors(src)
    );
}

#[test]
fn a_keyword_named_class_parses() {
    // `publish` is a built-in effect class AND a reserved keyword, so
    // `expect_ident` rejected the one class most worth attributing.
    let src = "
        locus L { params { n: Int = 0; } fn f() -> Int { return 1; } }
        main locus App {
            params { l: L = L { }; }
            claims { c: require attributed(all publish); }
        }
        fn main() { App { }; }
    ";
    assert!(parse_source(src).is_ok(), "`all publish` must parse");
}
