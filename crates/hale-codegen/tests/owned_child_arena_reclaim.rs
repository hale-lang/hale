//! GH #871 — an owned child's arena is destroyed at its owner's
//! teardown, whatever the field that holds it is DECLARED as.
//!
//! Nine of the corpus fixtures left a locus arena live at process
//! exit. `LOTUS_ARENA_RESIDENCY=1` walks the runtime's own registry
//! of live top-level arenas at exit and reports them on an ordinary
//! build, with no sanitizer in play — which is the oracle here, and
//! the one the issue's dumps were taken with.
//!
//! Two arms, one theme: a locus literal took
//! `lower_locus_instantiation`'s `parent_owns_via_field` branch —
//! no eager dissolve, no deferred-dissolve frame entry, because an
//! owner's cascade is supposed to reach it — and then no cascade
//! did.
//!
//! 1. The field is typed by a CONTRACT, not by the child's locus:
//!    an `interface` slot (`j: Counter = Churner { }`) or a
//!    `perspective(P)` handle (`router: perspective(Router) =
//!    RouterV1 { }`). `emit_locus_field_dissolves` only ever matched
//!    `CodegenTy::LocusRef`, so the whole subtree under such a field
//!    outlived its owner — in `87-temp-locus-receiver`, four
//!    `Churner`s and the four `Rows` `@form(vec)` children under
//!    them.
//!
//! 2. The field cannot hold a locus at all. `Lonely { n: Queries {
//!    ... }.total() }` armed the parent-owned flag for an `Int`
//!    field, the `Queries` literal inside the initializer consumed
//!    it, and nothing owned the result: no field to cascade from, no
//!    frame entry to flush. The same literal as a statement or a
//!    `let` was always reclaimed.
//!
//! The assertions are on stdout as well as on the residency dump: a
//! `dissolve()` that prints makes the cascade's reach observable,
//! and an exact count is what keeps the fix from turning a leak into
//! a double teardown for a child the owner does NOT own.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Compile `src` and run it under `LOTUS_ARENA_RESIDENCY=1`.
/// Returns `(stdout, live_arena_count)`.
fn run(name: &str, src: &str) -> (String, usize) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("lotus_test_gh871_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "{name}: non-zero exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status,
    );
    // `[arena_residency dump] N live arenas, sorted by bytes desc:`
    // — printed to stderr by an atexit hook in lotus_arena.c.
    let line = stderr
        .lines()
        .find(|l| l.contains("[arena_residency dump]"))
        .unwrap_or_else(|| {
            panic!("{name}: no residency dump; is the env var honored?\nstderr:\n{stderr}")
        });
    let live: usize = line
        .split_whitespace()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{name}: unparsable dump line {line:?}"));
    assert_eq!(
        live, 0,
        "{name}: {live} arena(s) still live at exit\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    (stdout, live)
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// An allocating child (its `@form(vec)` grandchild's buffer is a
/// realloc outside the arena) behind an interface contract. Each
/// level prints at dissolve so the cascade's reach is stdout.
const IFACE_TREE: &str = r#"
    type Row { v: Int = 0; }
    @form(vec)
    locus Rows { capacity { heap rows of Row; } }
    interface Counter { fn count() -> Int; }
    locus Churner {
        params { rows: Rows = Rows { }; }
        birth() { self.rows.push(Row { v: 3 }); }
        drain() { println("churner drained"); }
        dissolve() { println("churner dissolved"); }
        fn count() -> Int { return self.rows.len(); }
    }
    locus Queries {
        params { j: Counter = Churner { }; }
        dissolve() { println("queries dissolved"); }
        fn total() -> Int { return self.j.count(); }
    }
    locus Lonely {
        params { n: Int = 0; }
        fn double() -> Int { return self.n * 2; }
    }
"#;

fn iface(body: &str) -> String {
    format!("{IFACE_TREE}\n{body}\n")
}

/// The interface arm, in the three binding shapes. Before the fix
/// every one of them left the `Churner` and its `Rows` alive at
/// exit — the owner's cascade stepped over a field it could not
/// name.
#[test]
fn an_interface_typed_child_is_reclaimed_with_its_owner() {
    let (out, _) = run(
        "iface_default",
        &iface(
            r#"
            fn main() {
                let q = Queries { };
                println("t=", q.total());
            }
        "#,
        ),
    );
    assert!(out.contains("t=1"), "got:\n{out}");
    assert_eq!(count(&out, "churner dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "churner drained"), 1, "got:\n{out}");
    assert_eq!(count(&out, "queries dissolved"), 1, "got:\n{out}");

    // The override spelling, and a temporary receiver — the GH #710
    // shape `87-temp-locus-receiver` is built on.
    let (out, _) = run(
        "iface_override_temp",
        &iface(
            r#"
            fn main() {
                println("a=", Queries { j: Churner { } }.total());
                println("b=", Queries { j: Churner { } }.total());
            }
        "#,
        ),
    );
    assert!(out.contains("a=1") && out.contains("b=1"), "got:\n{out}");
    assert_eq!(count(&out, "churner dissolved"), 2, "got:\n{out}");
    assert_eq!(count(&out, "queries dissolved"), 2, "got:\n{out}");
}

/// The F.29 ownership gate has to survive the new arm. A child
/// handed IN through an interface-typed field is not the holder's
/// to tear down — its own binding is. Exactly once, from one place.
#[test]
fn an_externally_provided_interface_child_dissolves_exactly_once() {
    let (out, _) = run(
        "iface_external",
        &iface(
            r#"
            fn main() {
                let shared = Churner { };
                let q = Queries { j: shared };
                println("t=", q.total());
                println("done");
            }
        "#,
        ),
    );
    assert!(out.contains("t=1"), "got:\n{out}");
    assert_eq!(count(&out, "churner dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "queries dissolved"), 1, "got:\n{out}");
    // Reverse declaration order at fn-scope exit: the holder first,
    // whose cascade skips the borrowed child, then the child by its
    // own binding.
    let lines: Vec<&str> = out.lines().collect();
    let at = |needle: &str| {
        lines
            .iter()
            .position(|l| *l == needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in:\n{out}"))
    };
    assert!(at("done") < at("queries dissolved"), "got:\n{out}");
    assert!(
        at("queries dissolved") < at("churner dissolved"),
        "got:\n{out}"
    );
}

/// A locus literal in the initializer of a field that cannot HOLD a
/// locus. The parent-owned flag used to be armed for every field,
/// and the first literal lowered took it — becoming a child of an
/// owner with nowhere to put it, and so of nobody.
#[test]
fn a_literal_inside_a_non_locus_field_init_is_fn_scope_owned() {
    let (out, _) = run(
        "nested_in_int_field",
        &iface(
            r#"
            fn main() {
                println("n=", Lonely { n: Queries { }.total() }.double());
                println("done");
            }
        "#,
        ),
    );
    assert!(out.contains("n=2"), "got:\n{out}");
    assert_eq!(count(&out, "churner dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "queries dissolved"), 1, "got:\n{out}");
    // Fn-scope owned, like any other expression-position literal
    // (GH #711 / #814): it outlives the statement it was written in.
    let lines: Vec<&str> = out.lines().collect();
    let at = |needle: &str| {
        lines
            .iter()
            .position(|l| *l == needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in:\n{out}"))
    };
    assert!(at("done") < at("queries dissolved"), "got:\n{out}");
}

const PERSP_TREE: &str = r#"
    perspective Router {
        fn route(code: Int) -> Int;
    }
    locus RouterV1 : serves Router {
        dissolve() { println("v1 dissolved"); }
        fn route(code: Int) -> Int { return code + 100; }
    }
    locus RouterV2 : serves Router {
        dissolve() { println("v2 dissolved"); }
        fn route(code: Int) -> Int { return code + 200; }
    }
    locus Gateway {
        params { router: perspective(Router) = RouterV1 { }; }
        dissolve() { println("gateway dissolved"); }
        fn handle(code: Int) -> Int { return self.router.route(code); }
    }
"#;

/// A `perspective(P)` field holds the designated impl for ownership
/// alone — dispatch goes through the program-global slot. Ownership
/// alone is exactly what was never exercised: the impl's arena
/// outlived the holder in all six perspective fixtures.
#[test]
fn a_designated_perspective_impl_is_reclaimed_with_its_holder() {
    let (out, _) = run(
        "persp_default",
        &format!(
            "{PERSP_TREE}\n{}\n",
            r#"
            fn main() {
                let g = Gateway { };
                println("r=", g.handle(7));
            }
        "#
        ),
    );
    assert!(out.contains("r=107"), "got:\n{out}");
    assert_eq!(count(&out, "v1 dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "gateway dissolved"), 1, "got:\n{out}");

    // The ctor-override spelling (`65-perspective-ctor-override`):
    // the impl is chosen at the literal, not at the declaration, so
    // the teardown cannot be keyed on the holder's TYPE.
    let (out, _) = run(
        "persp_override",
        &format!(
            "{PERSP_TREE}\n{}\n",
            r#"
            fn main() {
                let g = Gateway { router: RouterV2 { } };
                println("r=", g.handle(7));
            }
        "#
        ),
    );
    assert!(out.contains("r=207"), "got:\n{out}");
    assert_eq!(count(&out, "v2 dissolved"), 1, "got:\n{out}");
    assert_eq!(count(&out, "v1 dissolved"), 0, "got:\n{out}");
    assert_eq!(count(&out, "gateway dissolved"), 1, "got:\n{out}");
}

/// GH #895 — the same interface-typed field, reached through a
/// FACTORY rather than a literal.
///
/// `Queries { j: make_churner() }` is the GH #836 transfer written
/// against a CONTRACT-typed field, and it was the last shape in this
/// family that nobody owned. Both halves of the decision were already
/// in place: the F.17 gate keeps the enclosing frame out of an
/// `Interface` field's initialiser, and the cascade above tears down
/// whatever `__owned_child_reclaim_<f>` names. Between them sits the
/// mask bit, and the predicate that sets it compares the factory's
/// declared locus with the FIELD's — which an interface field does
/// not have. So the frame stood back, the owner had no bit, and the
/// `Churner` plus the `Rows` `@form(vec)` under it outlived the
/// process.
///
/// The impl's name is what closes it: the factory DECLARES it, so
/// the instantiation can write `__reclaim_<Impl>` into the slot
/// exactly as a literal init does.
const FACTORY_TREE: &str = r#"
    type Row { v: Int = 0; }
    @form(vec)
    locus Rows { capacity { heap rows of Row; } }
    interface Counter { fn count() -> Int; }
    locus Churner {
        params { rows: Rows = Rows { }; }
        birth() { self.rows.push(Row { v: 3 }); }
        drain() { println("churner drained"); }
        dissolve() { println("churner dissolved"); }
        fn count() -> Int { return self.rows.len(); }
    }
    locus Tally {
        dissolve() { println("tally dissolved"); }
        fn count() -> Int { return 7; }
    }
    locus Queries {
        params { j: Counter = Churner { }; }
        dissolve() { println("queries dissolved"); }
        fn total() -> Int { return self.j.count(); }
    }
    locus Defaulted {
        params { j: Counter = make_churner(); }
        dissolve() { println("defaulted dissolved"); }
        fn total() -> Int { return self.j.count(); }
    }
    fn make_churner() -> Churner {
        return Churner { };
    }
    fn make_churner_f(n: Int) -> Churner fallible(String) {
        if n < 0 { fail "negative"; }
        return Churner { };
    }
    fn make_tally() -> Tally {
        return Tally { };
    }
    fn pick(a: Churner, b: Churner, which: Int) -> Churner {
        if which == 0 { return a; }
        return b;
    }
"#;

fn factory(body: &str) -> String {
    format!("{FACTORY_TREE}\n{body}\n")
}

/// The headline, in both spellings. A diverging `or` leaves the
/// factory's result as the only value the field can hold, so it is
/// the same transfer and takes the same bit.
#[test]
fn a_factory_built_interface_child_is_reclaimed_with_its_owner() {
    let (out, _) = run(
        "iface_factory",
        &factory(
            r#"
            fn main() {
                let q = Queries { j: make_churner() };
                println("t=", q.total());
                let r = Queries { j: make_churner_f(1) or raise };
                println("u=", r.total());
                println("done");
            }
        "#,
        ),
    );
    // Exact, because a missing teardown is silence and a double one
    // is the same line twice. Reverse-order flush over the two
    // bindings; within each, the owner's own `dissolve()` body runs
    // before its cascade, and a contract child's whole spine — drain
    // included — runs at that one point (it is not split the way a
    // `LocusRef` field's is, whose type IS known at the first).
    assert_eq!(
        out,
        "t=1\nu=1\ndone\n\
         queries dissolved\nchurner drained\nchurner dissolved\n\
         queries dissolved\nchurner drained\nchurner dissolved\n",
        "a factory-built interface child must be reclaimed by the \
         owner it was built into, once, in either spelling"
    );
}

/// The ctor-override shape (`65-perspective-ctor-override`'s twin
/// for an interface slot): the factory returns an impl that is NOT
/// the one the param's default literal names. The reclaim is chosen
/// per instantiation, so the slot has to carry `__reclaim_Tally` and
/// not the declared default's `__reclaim_Churner` — the wrong one
/// would run a teardown spine for a `Rows` child that was never
/// built.
#[test]
fn a_factory_of_another_impl_writes_that_impls_reclaim() {
    let (out, _) = run(
        "iface_factory_override",
        &factory(
            r#"
            fn main() {
                let q = Queries { j: make_tally() };
                println("t=", q.total());
                println("done");
            }
        "#,
        ),
    );
    assert_eq!(
        out,
        "t=7\ndone\nqueries dissolved\ntally dissolved\n",
        "the owner must reclaim the impl its factory actually built"
    );
}

/// The param DEFAULT site, which is the other half of the same
/// decision (GH #836 pinned it for a locus-typed field): a contract
/// param whose default is a factory call is the same transfer into
/// the same field, and it is the only way to reach the default-init
/// arm.
#[test]
fn a_factory_interface_param_default_dissolves_with_its_owner() {
    let (out, _) = run(
        "iface_factory_default",
        &factory(
            r#"
            fn main() {
                let d = Defaulted { };
                println("t=", d.total());
                println("done");
            }
        "#,
        ),
    );
    assert_eq!(
        out,
        "t=1\ndone\ndefaulted dissolved\n\
         churner drained\nchurner dissolved\n",
        "a contract param whose DEFAULT is a factory call must be \
         reclaimed by the owner it defaulted into"
    );
}

/// The guard, at the interface field. `pick` hands back a locus it
/// did not build — one of its own arguments — so the `let` that
/// built it is still the owner and the field's bit must stay clear.
/// Setting it would dissolve that value twice: once from the owner's
/// cascade and once from the binding's scope. Identical output
/// before and after this fix, on purpose.
#[test]
fn an_accessor_returned_interface_handle_is_left_to_its_owner() {
    let (out, _) = run(
        "iface_accessor",
        &factory(
            r#"
            fn main() {
                let x = Churner { };
                let y = Churner { };
                let q = Queries { j: pick(x, y, 1) };
                println("t=", q.total());
                println("done");
            }
        "#,
        ),
    );
    // Two churners built, two torn down — `y` by its own binding,
    // not a second time by the holder's cascade.
    assert_eq!(
        out,
        "t=1\ndone\nqueries dissolved\n\
         churner drained\nchurner dissolved\n\
         churner drained\nchurner dissolved\n",
        "a call that returns a locus somebody else owns must leave \
         the interface field's mask bit clear"
    );
}
