//! GH #436: `@sealed locus` — state confinement.
//!
//! A sealed locus's `params` are readable only from inside its own
//! methods. This is the primitive the secrets story rests on: without
//! it a parent reads a child's params directly (`self.child.key`
//! typechecks), so "the key never leaves the locus that owns it" is a
//! property we check rather than one that is true.
//!
//! What sealing does NOT do is make a locus uncallable — that is the
//! whole point, and `sealing_does_not_block_calls` pins it.

use hale_syntax::parse_source;

fn errors(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

const SEALED_SIGNER: &str = r#"
    @sealed locus Signer {
        params { key: Int = 7; }
        fn sign(m: Int) -> Int { return m + self.key; }
    }
"#;

#[test]
fn reading_a_sealed_param_from_outside_is_an_error() {
    let src = format!(
        "{SEALED_SIGNER}
        locus Gateway {{
            params {{ s: Signer = Signer {{ }}; }}
            fn bad() -> Int {{ return self.s.key; }}
        }}
        main locus App {{ params {{ g: Gateway = Gateway {{ }}; }} }}
        fn main() {{ App {{ }}; }}
        "
    );
    let es = errors(&src);
    assert!(
        es.iter().any(|m| m.contains("`Signer` is `@sealed`")
            && m.contains("readable only from inside")),
        "expected a sealed-read error, got {es:?}"
    );
}

#[test]
fn the_diagnostic_names_a_method_to_call_instead() {
    // The point of sealing is that the locus stays usable. The
    // diagnostic has to say so, or it reads as "you cannot use this".
    let src = format!(
        "{SEALED_SIGNER}
        locus Gateway {{
            params {{ s: Signer = Signer {{ }}; }}
            fn bad() -> Int {{ return self.s.key; }}
        }}
        main locus App {{ params {{ g: Gateway = Gateway {{ }}; }} }}
        fn main() {{ App {{ }}; }}
        "
    );
    let es = errors(&src);
    assert!(
        es.iter().any(|m| m.contains("call one of its methods")
            && m.contains("sign")),
        "diagnostic should name `sign` as the way in, got {es:?}"
    );
}

#[test]
fn the_sealed_locus_reads_its_own_params_freely() {
    // `self.key` INSIDE `Signer` has receiver type `Signer` exactly
    // like `self.s.key` outside it does. The rule is about the
    // reader, not the receiver syntax, and this is the case that
    // catches getting that backwards.
    let src = format!(
        "{SEALED_SIGNER}
        main locus App {{ params {{ s: Signer = Signer {{ }}; }} }}
        fn main() {{ App {{ }}; }}
        "
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
}

#[test]
fn sealing_does_not_block_calls() {
    let src = format!(
        "{SEALED_SIGNER}
        locus Gateway {{
            params {{ s: Signer = Signer {{ }}; }}
            fn ok(m: Int) -> Int {{ return self.s.sign(m); }}
        }}
        main locus App {{ params {{ g: Gateway = Gateway {{ }}; }} }}
        fn main() {{ App {{ }}; }}
        "
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
}

#[test]
fn sealing_does_not_block_construction() {
    // Deliberate: a parent writing `Signer { key: … }` already holds
    // the value it passes, so sealing the initializer would cost
    // ordinary configuration and buy nothing. Real secret material
    // should be loaded inside `birth` instead. Pinned because it is
    // a design decision, not an oversight.
    let src = "
        @sealed locus Signer {
            params { key: Int = 0; }
            fn sign(m: Int) -> Int { return m + self.key; }
        }
        main locus App { params { s: Signer = Signer { key: 9 }; } }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn an_unsealed_locus_is_unaffected() {
    // The annotation is opt-in and breaks no existing program.
    let src = "
        locus Signer {
            params { key: Int = 7; }
            fn sign(m: Int) -> Int { return m + self.key; }
        }
        locus Gateway {
            params { s: Signer = Signer { }; }
            fn reads() -> Int { return self.s.key; }
        }
        main locus App { params { g: Gateway = Gateway { }; } }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn a_free_fn_cannot_read_a_sealed_param_either() {
    // The rule is "inside its own methods", and a free fn is not one.
    // Without this the annotation has a hole a helper walks through.
    let src = format!(
        "{SEALED_SIGNER}
        fn peek(s: Signer) -> Int {{ return s.key; }}
        main locus App {{ params {{ s: Signer = Signer {{ }}; }} }}
        fn main() {{ App {{ }}; }}
        "
    );
    let es = errors(&src);
    assert!(
        es.iter().any(|m| m.contains("`@sealed`")),
        "expected the free fn to be rejected, got {es:?}"
    );
}

#[test]
fn sealing_is_per_type_not_per_instance() {
    // A `Signer` method may read another `Signer`'s params. Class-
    // private rather than instance-private, matching the ordinary
    // reading of "inside its own methods" — the two instances share
    // a trust domain because they share a body. Pinned as a decision.
    let src = "
        @sealed locus Signer {
            params { key: Int = 7; }
            fn sign(m: Int) -> Int { return m + self.key; }
            fn peer(o: Signer) -> Int { return o.key; }
        }
        main locus App { params { s: Signer = Signer { }; } }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn sealing_confines_a_secret_the_bus_would_otherwise_carry() {
    // The end-to-end shape: without `@sealed` this program publishes
    // the key and typechecks clean. That is the defect #436 opened on.
    let leaky = "
        type Msg { v: Int; }
        topic Out { payload: Msg; subject: \"app.out\"; }
        LOCUS_KW locus Signer {
            params { key: Int = 7; }
            fn sign(m: Int) -> Int { return m + self.key; }
        }
        locus Gateway {
            params { s: Signer = Signer { }; }
            bus { publish Out; }
            fn go() { Out <- Msg { v: self.s.key }; }
        }
        locus Sink {
            params { n: Int = 0; }
            bus { subscribe Out as on_out; }
            fn on_out(m: Msg) { self.n = m.v; }
        }
        main locus App {
            params { g: Gateway = Gateway { }; k: Sink = Sink { }; }
        }
        fn main() { App { }; }
    ";
    assert!(
        errors(&leaky.replace("LOCUS_KW", "")).is_empty(),
        "unsealed: the key on the bus must still typecheck, or this \
         test is measuring something else"
    );
    let es = errors(&leaky.replace("LOCUS_KW", "@sealed"));
    assert!(
        es.iter().any(|m| m.contains("`@sealed`")),
        "sealed: publishing the key must be rejected, got {es:?}"
    );
}

// ---------------------------------------------------------------
// `@sealed` and `contract { expose … }` are contradictory claims
// about the same boundary.
// ---------------------------------------------------------------

#[test]
fn sealed_plus_expose_is_rejected() {
    // Sealing wins over an expose, so the pair leaves a contract that
    // typechecks as coherent — a matching `consume` binds fine — and
    // is then rejected at every use. A construct that reads as a
    // permission and grants nothing.
    let src = "
        @sealed locus Greeter {
            params { greeting: String = \"hi\"; }
            contract { expose greeting: String; }
            fn hello() -> String { return self.greeting; }
        }
        main locus App { params { g: Greeter = Greeter { }; } }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("cannot grant anything")),
        "the pair must be rejected at the declaration, got {es:?}"
    );
}

#[test]
fn the_consume_side_needs_no_check_of_its_own() {
    // Because a sealed locus cannot declare an `expose`, a
    // coordinator consuming from one falls into the existing
    // "does not expose it" arm. One check covers both directions.
    let src = "
        @sealed locus Greeter {
            params { greeting: String = \"hi\"; }
            fn hello() -> String { return self.greeting; }
        }
        locus Coord {
            params { n: Int = 0; }
            contract { consume greeting: String; }
            accept(g: Greeter) { self.n = 1; }
        }
        main locus App { params { c: Coord = Coord { }; } }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("does not expose it")),
        "expected the existing contract arm to catch it, got {es:?}"
    );
}

#[test]
fn a_contract_on_an_unsealed_locus_is_unaffected() {
    let src = "
        locus Greeter {
            params { greeting: String = \"hi\"; }
            contract { expose greeting: String; }
            fn hello() -> String { return self.greeting; }
        }
        locus Coord {
            params { n: Int = 0; }
            contract { consume greeting: String; }
            accept(g: Greeter) { self.n = len(g.greeting); }
        }
        main locus App { params { c: Coord = Coord { }; } }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn a_sealed_locus_without_a_contract_is_unaffected() {
    let src = "
        @sealed locus Greeter {
            params { greeting: String = \"hi\"; }
            fn hello() -> String { return self.greeting; }
        }
        locus Coord {
            params { n: Int = 0; }
            accept(g: Greeter) { self.n = len(g.hello()); }
        }
        main locus App { params { c: Coord = Coord { }; } }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}
