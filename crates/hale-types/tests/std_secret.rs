//! GH #436: `std::secret` — key material an application never holds.
//!
//! `@sealed` closes the read side but deliberately leaves param
//! *initialization* open, because a parent writing `Signer { key: … }`
//! already holds what it passes. These loci close that last gap by
//! taking the NAME OF A SOURCE rather than the bytes: the key enters
//! the program inside a sealed locus and there is no construction site
//! at which the caller held it.
//!
//! The load-bearing test here is `the_seal_holds_through_the_stdlib_
//! path_rename`. Before the resolver change that accompanies this
//! module, a qualified stdlib path typed as `Ty::Unknown`, the sealed
//! check keys off the resolved type, and so `self.signer.key` read
//! real key bytes while `hale check` passed. The module was written,
//! found unsafe, and held back until the resolver could see it.

use hale_syntax::parse_source;

fn errors(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

const GATEWAY: &str = r#"
    locus Gateway {
        params {
            s: std::secret::Signer =
                std::secret::Signer { env_var: "SK" };
        }
        BODY
    }
    main locus App { params { g: Gateway = Gateway { }; } }
    fn main() { App { }; }
"#;

fn gateway(body: &str) -> String {
    GATEWAY.replace("BODY", body)
}

#[test]
fn the_seal_holds_through_the_stdlib_path_rename() {
    let es = errors(&gateway("fn peek() -> Int { return len(self.s.key); }"));
    assert!(
        es.iter().any(|m| m.contains("`@sealed`")),
        "reading the key from outside must be rejected, got {es:?}"
    );
}

#[test]
fn the_diagnostic_uses_the_spelling_the_author_wrote() {
    // The locus is DECLARED as `__StdSecretSigner`; nobody writes
    // that. A diagnostic naming the mangled form sends the reader
    // looking for a symbol that appears nowhere in their program.
    let es = errors(&gateway("fn peek() -> Int { return len(self.s.key); }"));
    assert!(
        es.iter().any(|m| m.contains("std::secret::Signer")),
        "expected the user-facing path, got {es:?}"
    );
    assert!(
        !es.iter().any(|m| m.contains("__StdSecret")),
        "the mangled name must not leak into a diagnostic: {es:?}"
    );
}

#[test]
fn the_privileged_operations_are_callable() {
    // Sealing confines state; it must not make the locus useless.
    let es = errors(&gateway(
        "fn go(m: Bytes) -> Bytes { return self.s.sign(m); }
         fn ok() -> Bool { return self.s.ready(); }",
    ));
    assert!(es.is_empty(), "{es:?}");
}

#[test]
fn a_credential_is_sealed_too() {
    let src = "
        locus Auth {
            params {
                c: std::secret::Credential =
                    std::secret::Credential { env_var: \"TOKEN\" };
            }
            fn peek() -> Int { return len(self.c.value); }
        }
        main locus App { params { a: Auth = Auth { }; } }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("`@sealed`")),
        "expected the credential value to be confined, got {es:?}"
    );
}

#[test]
fn a_fingerprint_is_publishable_and_the_value_is_not() {
    // The escape hatch that makes confinement usable: a non-reversible
    // handle you CAN log, next to a value you cannot.
    let es = errors(&gateway("fn go() -> Bool { return self.s.ready(); }"));
    assert!(es.is_empty(), "{es:?}");

    let src = "
        locus Auth {
            params {
                c: std::secret::Credential =
                    std::secret::Credential { env_var: \"TOKEN\" };
            }
            fn tag() -> String { return self.c.fingerprint(); }
        }
        main locus App { params { a: Auth = Auth { }; } }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn only_sealed_stdlib_loci_gained_a_resolved_type() {
    // The resolver injection is deliberately narrow: making EVERY
    // qualified stdlib path resolve would switch on field-existence
    // and arity checking across the whole stdlib surface at once.
    // An unsealed stdlib locus must still behave exactly as before —
    // permissive, because its type is still `Unknown`.
    let src = "
        locus L {
            params { n: Int = 0; }
            fn f(s: std::io::tcp::Stream) -> Int {
                return s.no_such_field_at_all;
            }
        }
        main locus App { params { l: L = L { }; } }
        fn main() { App { }; }
    ";
    assert!(
        errors(src).is_empty(),
        "unsealed stdlib paths must stay permissive: {:?}",
        errors(src)
    );
}
