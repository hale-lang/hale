//! GH #436: `require attributed(all C)` — every boundary crossing
//! names a purpose.
//!
//! Orthogonal to interposition, which is why it exists as its own
//! form. `forbid reaches(app, effects(syscall)) avoiding gate`
//! constrains WHERE the boundary is crossed and is silent on what any
//! crossing is FOR; this constrains attribution and is silent on
//! location. You can have either without the other:
//!
//!   - interposed, unattributed: all I/O funnels through one
//!     `write(path, bytes)` that everyone calls for everything;
//!   - attributed, not interposed: forty loci each touching the OS,
//!     every one of them naming its purpose.
//!
//! It also closes a coverage hole that an `avoiding` claim scoped to
//! a group necessarily has: this is a universal over the whole closed
//! world, so a locus written next month is covered without anyone
//! editing the claim.

use hale_syntax::parse_source;

fn errors(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

fn program(rogue_attr: &str) -> String {
    format!(
        "effect audit;
         locus Good {{
             params {{ n: Int = 0; }}
             @effects(is: {{ audit }})
             fn record(s: String) {{
                 std::io::fs::write_file(\"/tmp/a.log\", s);
             }}
         }}
         locus Rogue {{
             params {{ n: Int = 0; }}
             {rogue_attr}
             fn sneak(s: String) {{
                 std::io::fs::write_file(\"/tmp/b.log\", s);
             }}
         }}
         main locus App {{
             params {{ g: Good = Good {{ }}; r: Rogue = Rogue {{ }}; }}
             claims {{ io_attributed: require attributed(all syscall); }}
         }}
         fn main() {{ App {{ }}; }}"
    )
}

#[test]
fn an_unattributed_syscall_violates() {
    let es = errors(&program(""));
    assert!(
        es.iter().any(|m| m.contains("io_attributed") && m.contains("violated")),
        "expected a violation, got {es:?}"
    );
}

#[test]
fn the_violation_names_the_site_not_the_group() {
    // Attribution is a per-site property; "something is unattributed"
    // is not actionable in a program with many I/O sites.
    let es = errors(&program(""));
    assert!(
        es.iter().any(|m| m.contains("Rogue::sneak")),
        "expected the offending fn named, got {es:?}"
    );
    assert!(
        !es.iter().any(|m| m.contains("Good::record")),
        "the attributed fn must not be blamed: {es:?}"
    );
}

#[test]
fn attributing_the_site_satisfies_it() {
    let src = program("@effects(is: { audit })");
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
}

#[test]
fn attribution_is_direct_not_transitive() {
    // The load-bearing choice. If it were transitive, every caller
    // downstream of one attributed fn would inherit the label and
    // pass, making the claim nearly vacuous. `Wrapper::go` performs no
    // syscall of its own, so it owes nothing; `Raw::io` does, and owes.
    let src = "
        effect audit;
        locus Raw {
            params { n: Int = 0; }
            fn io(s: String) { std::io::fs::write_file(\"/tmp/x\", s); }
        }
        locus Wrapper {
            params { r: Raw = Raw { }; }
            fn go(s: String) { self.r.io(s); }
        }
        main locus App {
            params { w: Wrapper = Wrapper { }; }
            claims { io_attributed: require attributed(all syscall); }
        }
        fn main() { App { }; }
    ";
    let es = errors(src);
    assert!(
        es.iter().any(|m| m.contains("Raw::io")),
        "the site that crosses the boundary must be named: {es:?}"
    );
    assert!(
        !es.iter().any(|m| m.contains("Wrapper::go")),
        "a caller that performs no syscall itself owes nothing: {es:?}"
    );
}

#[test]
fn a_builtin_in_is_does_not_count_as_attribution() {
    // `@effects(is: {syscall})` restates what the compiler already
    // infers. The claim asks for a purpose the AUTHOR supplied.
    let src = program("@effects(is: { syscall })");
    let es = errors(&src);
    assert!(
        es.iter().any(|m| m.contains("Rogue::sneak")),
        "restating a built-in must not satisfy the claim: {es:?}"
    );
}

#[test]
fn the_class_must_have_countable_direct_sites() {
    // Two rejections with one rationale: the claim can only be
    // evaluated where a DIRECT site exists to attribute.
    //
    // A user class (`audit`) would ask that every site carrying it
    // also carries a user class — trivially true, while reading like
    // a real contract. `ffi` / `spawn` / `recursion` are structural,
    // carried by no registry row, so the evaluator answered them with
    // unconditional success. Validation now uses the same predicate
    // as evaluation, so neither can be accepted.
    for class in ["audit", "ffi", "spawn", "recursion"] {
        let src = format!(
            "effect audit;
             locus L {{ params {{ n: Int = 0; }}
                 fn f() -> Int {{ return 1; }} }}
             main locus App {{
                 params {{ l: L = L {{ }}; }}
                 claims {{ bogus: require attributed(all {class}); }}
             }}
             fn main() {{ App {{ }}; }}"
        );
        let es = errors(&src);
        assert!(
            es.iter().any(|m| m.contains("countable DIRECT sites")),
            "`{class}` must be rejected with the shared-predicate \
             message, got {es:?}"
        );
    }
}

#[test]
fn the_quantifier_must_be_all() {
    let src = "
        locus L { params { n: Int = 0; } fn f() -> Int { return 1; } }
        main locus App {
            params { l: L = L { }; }
            claims { bogus: require attributed(some syscall); }
        }
        fn main() { App { }; }
    ";
    assert!(parse_source(src).is_err(), "`some` must not parse");
}

#[test]
fn a_program_with_no_such_sites_holds() {
    let src = "
        locus L { params { n: Int = 0; } fn f() -> Int { return 1; } }
        main locus App {
            params { l: L = L { }; }
            claims { io_attributed: require attributed(all syscall); }
        }
        fn main() { App { }; }
    ";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}
