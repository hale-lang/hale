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

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;

fn errors(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
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

// ---------------------------------------------------------------
// The positions the rows reach that a walk's discard used to drop
// (F.40 phase 4's leftovers): a parameter's default, typed at each
// call that leaves it.
// ---------------------------------------------------------------

/// Each error with the source text it is reported at.
fn error_sites(src: &str) -> Vec<(String, String)> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (src[d.span.start.0 as usize..d.span.end.0 as usize].to_string(), d.message))
        .collect()
}

const SEALED_KEY_READ: &str = "`Signer` is `@sealed`: its `params` are readable only from inside its own \
     methods, and `Signer.key` reads one from outside — call one of its methods instead (sign)";

#[test]
fn a_default_reading_a_sealed_param_is_refused_where_it_is_written() {
    // The default is typed at each call that leaves it, in the caller's
    // scope, and that walk's findings are discarded; its accesses are
    // kept, a row per evaluation, and refused once at the default.
    let src = format!(
        "{SEALED_SIGNER}
        locus Gateway {{
            params {{ s: Signer = Signer {{ }}; }}
            fn peek(k: Int = self.s.key) -> Int {{ return k; }}
            fn a() -> Int {{ return self.peek(); }}
            fn b() -> Int {{ return self.peek() + self.peek(); }}
        }}
        main locus App {{ params {{ g: Gateway = Gateway {{ }}; }} }}
        fn main() {{ App {{ }}; }}
        "
    );
    assert_eq!(error_sites(&src), vec![("self.s.key".to_string(), SEALED_KEY_READ.to_string())]);
    // Two declarations evaluating one default: still once.
    let shared = format!(
        "{SEALED_SIGNER}
        fn peek(k: Int = self.s.key) -> Int {{ return k; }}
        locus Gateway {{
            params {{ s: Signer = Signer {{ }}; }}
            fn a() -> Int {{ return peek(); }}
        }}
        locus Other {{
            params {{ s: Signer = Signer {{ }}; }}
            fn a() -> Int {{ return peek() + peek(); }}
        }}
        main locus App {{ params {{ g: Gateway = Gateway {{ }}; o: Other = Other {{ }}; }} }}
        fn main() {{ App {{ }}; }}
        "
    );
    assert_eq!(error_sites(&shared), vec![("self.s.key".to_string(), SEALED_KEY_READ.to_string())]);
    // Unsealed, both read freely.
    assert!(error_sites(&src.replace("@sealed ", "")).is_empty());
    assert!(error_sites(&shared.replace("@sealed ", "")).is_empty());
}

#[test]
fn a_monomorph_receiver_reaches_its_templates_params() {
    // `b: Box<Int>` is typed as the monomorph `Box_Int`, which the scope
    // declares no locus by: the row names the template, whose params
    // `b.v` reaches, and the template is sealed.
    let src = "
        @sealed locus Box<T> {
            params { v: Int = 0; }
            fn get() -> Int { return self.v; }
            fn same(o: Box<Int>) -> Int { return o.v; }
        }
        locus Gateway {
            fn read() -> Int { let b: Box<Int> = Box { }; return b.v; }
            fn write() { let b: Box<Int> = Box { }; b.v = 3; }
        }
        main locus App { params { g: Gateway = Gateway { }; } }
        fn main() { App { }; }
    ";
    let read = "`Box` is `@sealed`: its `params` are readable only from inside its own methods, \
         and `Box.v` reads one from outside — call one of its methods instead (get, same)";
    let write = "`Box` is `@sealed`: its `params` are writable only from inside its own methods, \
         and `Box.v` writes one from outside — call one of its methods instead (get, same)";
    let found = error_sites(src);
    assert_eq!(found, vec![("b.v".to_string(), read.to_string()), ("v".to_string(), write.to_string())]);
    // The write's span is its param segment, in `write`.
    let program = parse_source(src).expect("parse");
    let at = entries::check_program(&program).into_iter().filter(|d| d.is_error()).nth(1).unwrap().span;
    assert_eq!(at.start.0 as usize, src.find("b.v = 3").unwrap() + 2);
    // From inside the template (`same` reads another `Box_Int`), and
    // with the template unsealed, it is clean.
    let inside = src.replace("fn read() -> Int { let b: Box<Int> = Box { }; return b.v; }", "").replace(
        "fn write() { let b: Box<Int> = Box { }; b.v = 3; }",
        "fn call() -> Int { let b: Box<Int> = Box { }; return b.same(b); }",
    );
    assert!(error_sites(&inside).is_empty(), "{:?}", error_sites(&inside));
    // `same`'s read, written outside the template, is refused: the
    // clean read inside is a row, judged from inside.
    let outside = inside.replace("fn call()", "fn peer(o: Box<Int>) -> Int { return o.v; }\n            fn call()");
    assert_eq!(error_sites(&outside), vec![("o.v".to_string(), read.to_string())]);
    assert!(error_sites(&src.replace("@sealed ", "")).is_empty(), "{:?}", error_sites(&src.replace("@sealed ", "")));
}

#[test]
fn a_default_evaluated_inside_the_sealed_locus_is_clean() {
    let src = "
        @sealed locus Signer {
            params { key: Int = 7; }
            fn sign(m: Int = self.key) -> Int { return m; }
            fn twice() -> Int { return self.sign() + self.sign(); }
        }
        main locus App { params { s: Signer = Signer { }; } }
        fn main() { App { }; }
    ";
    assert!(error_sites(src).is_empty(), "{:?}", error_sites(src));
}
