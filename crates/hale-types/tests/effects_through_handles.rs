//! #265 soundness — effect assertions must see through a call made
//! on a HANDLE, not just a direct path call.
//!
//! The original checker resolved three call shapes: free fns,
//! `self.m()` inside a locus, and `std::ns::fn(...)` path calls.
//! A call through a value — `r.slurp()`, `resolver.get(...)` — was
//! reduced to an unresolved edge carrying only the bare method
//! name, so the callgraph never reached the body and the effect
//! contributed nothing. Every assertion silently passed over it.
//!
//! That mattered far more than it sounds, because the locus-with-
//! methods shape is *the* idiomatic way to do I/O in Hale — the same
//! shape the violation diagnostic recommends as the fix. Moving an
//! effect behind a locus made it invisible, which is not the same
//! thing as making it unreachable.
//!
//! These pin the three ways the analysis can now reach a leaf:
//! through a user locus, through a Hale-source stdlib locus, and
//! through a frontier path with no registry row at all.

fn diags_for(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

fn violation(src: &str, class: &str) -> String {
    let ds = diags_for(src);
    ds.iter()
        .find(|m| m.contains(&format!("must not reach `{}`", class)))
        .cloned()
        .unwrap_or_else(|| {
            panic!("expected a `{}` effect violation; got {:?}", class, ds)
        })
}

#[test]
fn syscall_through_a_user_locus_handle_is_caught() {
    let src = r#"
        locus Reader {
            params { path: String = ""; }
            fn slurp() -> Int {
                return std::io::fs::file_size(self.path) or 0;
            }
        }
        @no_syscall
        fn quiet() -> Int {
            let r = Reader { path: "/etc/hostname" };
            return r.slurp();
        }
        fn main() { println(quiet()); }
    "#;
    let hit = violation(src, "syscall");
    assert!(
        hit.contains("quiet -> Reader::slurp"),
        "the witness must run THROUGH the handle method: {}",
        hit
    );
    assert!(hit.contains("file_size"), "and name the leaf: {}", hit);
}

/// The receiver's type comes from the struct literal, not an
/// annotation — `let r = Reader { … }` is the common shape and an
/// annotated `let` is the exception. Pin the annotated form too so
/// neither path regresses.
#[test]
fn annotated_let_receiver_also_resolves() {
    let src = r#"
        locus Reader {
            params { path: String = ""; }
            fn slurp() -> Int {
                return std::io::fs::file_size(self.path) or 0;
            }
        }
        @no_syscall
        fn quiet() -> Int {
            let r: Reader = Reader { path: "/x" };
            return r.slurp();
        }
        fn main() { println(quiet()); }
    "#;
    assert!(violation(src, "syscall").contains("Reader::slurp"));
}

/// `std::cli::Resolver` is implemented in Hale (`hale-stdlib/hl/
/// cli.hl`) and reads the environment. Those bodies used to live in
/// a const inside `hale-codegen`, downstream of the analyzer, so
/// they were structurally invisible.
#[test]
fn env_through_a_hale_source_stdlib_locus_is_caught() {
    let src = r#"
        @effects(none: {syscall, env})
        fn quiet() -> String {
            let r = std::cli::Resolver { env_prefix: "P_", argv_keys: "dir\n" };
            return r.get("dir", "fallback");
        }
        fn main() { println(quiet()); }
    "#;
    let hit = violation(src, "env");
    assert!(
        hit.contains("std::cli::Resolver::get"),
        "the witness must name the stdlib locus in its PUBLIC spelling, \
         not the mangled `__StdCliResolver`: {}",
        hit
    );
}

/// A path under a namespace with no registry row must fail closed.
/// It used to fail *open*: `effects_for` returned `None`, the `?`
/// short-circuited, and the leaf contributed nothing — so an entire
/// unregistered namespace read as pure.
#[test]
fn unregistered_std_path_fails_closed() {
    let src = r#"
        @no_syscall
        fn quiet() -> String {
            return std::nowhere::at_all("x");
        }
        fn main() { println(quiet()); }
    "#;
    let hit = violation(src, "syscall");
    assert!(
        hit.contains("not in the stdlib effect registry"),
        "an absent frontier row must be reported as uncertifiable, \
         not silently treated as pure: {}",
        hit
    );
}

/// Fail-closed applies to the FRONTIER, not to every unresolved
/// name. A bare method on a receiver whose type can't be inferred is
/// an ordinary unresolved callee; treating those as uncertifiable
/// would make assertions unusable.
#[test]
fn non_std_unresolved_callee_does_not_fail_closed() {
    let src = r#"
        locus Pure {
            params { n: Int = 0; }
            fn double() -> Int { return self.n * 2; }
        }
        @no_syscall
        fn quiet() -> Int {
            let p = Pure { n: 21 };
            return p.double();
        }
        fn main() { println(quiet()); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("must not reach")),
        "a pure handle call must not trip the assertion: {:?}",
        ds
    );
}

/// The negative control for the whole file: if the checker stopped
/// detecting anything, every assertion above would pass vacuously.
#[test]
fn direct_path_call_still_caught() {
    let src = r#"
        @no_syscall
        fn quiet() -> Int {
            return std::io::fs::file_size("/x") or 0;
        }
        fn main() { println(quiet()); }
    "#;
    assert!(violation(src, "syscall").contains("file_size"));
}

/// Writing to a stream is a `write(2)`. The frontier used to see
/// only `std::` paths, so the language builtins slipped through and
/// `@no_syscall` certified a fn that printed — while the very
/// diagnostic it would emit for `std::io::fs::*` described the
/// syscall class as covering "stdio". The surface contradicted
/// itself; this pins the resolution.
#[test]
fn println_is_a_syscall() {
    let src = r#"
        @no_syscall
        fn quiet() { println("hi"); }
        fn main() { quiet(); }
    "#;
    assert!(violation(src, "syscall").contains("println"));
}

#[test]
fn eprintln_is_a_syscall_too() {
    let src = r#"
        @no_syscall
        fn quiet() { eprintln("hi"); }
        fn main() { quiet(); }
    "#;
    assert!(violation(src, "syscall").contains("eprintln"));
}

/// F.20 interface-typed slot (downstream handoff). The declared type
/// of `self.sink` is an INTERFACE, which has no body — so
/// `self.sink.emit()` resolved to nothing and every effect behind the
/// slot was invisible. The concrete locus in the slot's default is
/// what actually runs.
///
/// This is the whole point of a plug-in-implementation tier:
/// consumers see only the
/// abstract type, so a contract on a consumer that reaches a venue
/// surface through a slot was vacuous.
#[test]
fn syscall_through_an_interface_typed_slot_is_caught() {
    let src = r#"
        interface Emitter { fn emit(tag: String) -> Int; }
        locus LoudEmitter {
            params { n: Int = 0; }
            fn emit(tag: String) -> Int { println("loud: ", tag); return 1; }
        }
        locus Manifest {
            params { sink: Emitter = LoudEmitter { }; }
            fn reach(t: String) -> Int { return self.sink.emit(t); }
        }
        @no_syscall
        fn certified(m: Manifest) -> Int { return m.reach("x"); }
        fn main() { let m = Manifest { }; println(certified(m)); }
    "#;
    let hit = violation(src, "syscall");
    assert!(
        hit.contains("Manifest::reach") && hit.contains("LoudEmitter::emit"),
        "the witness must run through the slot into the bound locus: {}",
        hit
    );
}

/// …and a slot bound to a genuinely pure implementation must stay
/// silent, or the check above would just be rejecting all interfaces.
#[test]
fn a_pure_interface_slot_is_not_flagged() {
    let src = r#"
        interface Emitter { fn emit(tag: String) -> Int; }
        locus QuietEmitter {
            params { n: Int = 0; }
            fn emit(tag: String) -> Int { return self.n; }
        }
        locus Manifest {
            params { sink: Emitter = QuietEmitter { }; }
            fn reach(t: String) -> Int { return self.sink.emit(t); }
        }
        @no_syscall
        fn certified(m: Manifest) -> Int { return m.reach("x"); }
        fn main() { let m = Manifest { }; println(certified(m)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("must not reach")),
        "a pure binding must not trip the assertion: {:?}",
        ds
    );
}
