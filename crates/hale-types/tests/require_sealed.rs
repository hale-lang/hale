//! GH #436: `require sealed(all G)` — confinement as law.
//!
//! `@sealed` on its own is per-locus discipline, which is exactly what
//! a security baseline cannot rest on: one unsealed member of a vault
//! group is the whole hole, and nothing was watching for it. Noticed
//! while writing the spec for #437 — an early draft claimed a
//! constitution could already require sealing across a group, which
//! was false, because no claim form requires an annotation.
//!
//! This is that form. It composes through constitutions like any
//! other claim, so a security baseline can be adopted once.

use hale_syntax::parse_source;

fn errors(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

fn vaults(second_sealed: bool) -> String {
    format!(
        "@sealed locus Vault1 {{ params {{ k: Int = 1; }}
             fn use1() -> Int {{ return self.k; }} }}
         {} locus Vault2 {{ params {{ k: Int = 2; }}
             fn use2() -> Int {{ return self.k; }} }}
         group vaults = {{ Vault1, Vault2 }};
         main locus App {{
             params {{ a: Vault1 = Vault1 {{ }}; b: Vault2 = Vault2 {{ }}; }}
             claims {{ confined: require sealed(all vaults); }}
         }}
         fn main() {{ App {{ }}; }}",
        if second_sealed { "@sealed" } else { "" }
    )
}

#[test]
fn an_unsealed_member_violates() {
    let es = errors(&vaults(false));
    assert!(
        es.iter().any(|m| m.contains("claim `confined` violated")),
        "expected a violation, got {es:?}"
    );
}

#[test]
fn the_violation_names_the_unsealed_locus() {
    // A group can have many members; "something here is unsealed" is
    // not actionable. The name is the whole value of the diagnostic.
    let es = errors(&vaults(false));
    assert!(
        es.iter().any(|m| m.contains("Vault2")),
        "expected the offending member named, got {es:?}"
    );
    assert!(
        !es.iter().any(|m| m.contains("Vault1")),
        "the sealed member must not be blamed: {es:?}"
    );
}

#[test]
fn a_fully_sealed_group_holds() {
    assert!(errors(&vaults(true)).is_empty(), "{:?}", errors(&vaults(true)));
}

#[test]
fn every_unsealed_member_is_reported_at_once() {
    // A universal reports the whole list: a baseline is adopted once
    // and the reader wants everything to fix, not one name per build.
    let src = "
        locus A { params { k: Int = 1; } fn f() -> Int { return self.k; } }
        locus B { params { k: Int = 2; } fn g() -> Int { return self.k; } }
        group vaults = { A, B };
        main locus App {
            params { x: A = A { }; y: B = B { }; }
            claims { confined: require sealed(all vaults); }
        }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("A") && m.contains("B")),
        "expected both members in one diagnostic, got {es:?}"
    );
}

#[test]
fn the_quantifier_must_be_all() {
    // `some` is the existential the other `require` forms use. Taking
    // it here would silently mean the opposite of what it reads.
    let src = "
        @sealed locus V { params { k: Int = 1; }
            fn f() -> Int { return self.k; } }
        group vaults = { V };
        main locus App {
            params { v: V = V { }; }
            claims { confined: require sealed(some vaults); }
        }
        fn main() { App { }; }
    ";
    assert!(
        parse_source(src).is_err(),
        "`require sealed(some G)` must not parse"
    );
}

#[test]
fn it_composes_through_a_constitution() {
    // The whole reason the form exists: a security baseline should be
    // adopted once, not restated per entrypoint. Claimed in the spec
    // and the docs, so it is pinned rather than assumed.
    let src = "
        @sealed locus V1 { params { k: Int = 1; }
            fn f() -> Int { return self.k; } }
        locus V2 { params { k: Int = 2; }
            fn g() -> Int { return self.k; } }
        group vaults = { V1, V2 };
        constitution SecretBaseline {
            vault_confined: require sealed(all vaults);
        }
        main locus App {
            params { a: V1 = V1 { }; b: V2 = V2 { }; }
            claims { adopt SecretBaseline; }
        }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("vault_confined")
            && m.contains("V2")),
        "an adopted constitution must evaluate the form, got {es:?}"
    );
}

#[test]
fn an_unknown_group_is_invalid_not_vacuously_true() {
    // The fail-open shape this whole issue is about: a claim over a
    // group that does not exist must not report `holds`.
    let src = "
        @sealed locus V { params { k: Int = 1; }
            fn f() -> Int { return self.k; } }
        main locus App {
            params { v: V = V { }; }
            claims { confined: require sealed(all nonexistent); }
        }
        fn main() { App { }; }
    ";
    assert!(!errors(src).is_empty(), "an unknown group must be rejected");
}
