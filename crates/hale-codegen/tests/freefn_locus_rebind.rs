//! Free-fn locus rebinding must not reclaim live memory
//! (downstream handoff, 2026-08-11; regression shipped in v0.16.0
//! via the GH #402 temporary-reclaim pass).
//!
//! A `let mut` binding of locus type that is REBOUND — from a
//! factory call or from another binding — used to hand its frame a
//! second owner for the same value:
//!
//!   * `a = make(...)` registered the factory result as a
//!     frame-owned temporary (the assignment is neither a `let` RHS
//!     nor a `return` expr, the two owner-already-decided sites the
//!     pass knew), so the frame reclaimed a value the binding (and
//!     then the caller) still held. Returned: the caller received a
//!     locus whose `capacity` heap buffer was gone — params intact,
//!     every `get` failing. Not returned: double-free, SIGTRAP.
//!   * The trigger was static — a rebinding inside `if false { }`
//!     was enough, because the temp's dissolve slot was alloca'd in
//!     the entry block but only STORED where the expression ran, so
//!     a bypassed store left stack garbage for the exit flush.
//!   * `a = nx` aliased two bindings to one value; `nx`'s
//!     scope-exit dissolve then fired on the value `a` returned.
//!
//! The fix: an `=` into a locus-typed slot suppresses the temp
//! registration (the slot owns the RHS), any binding on either side
//! of a bare-local `=` is disqualified from frame-scoped
//! reclamation (conservative: the old leak, never a double-free),
//! and deferred-dissolve slots are NULL-inited at entry so bypassed
//! paths read "nothing to dissolve".
//!
//! The last test pins the second latent shape the fix exposed: an
//! early `fail` that bypasses loop-body `let` bindings whose
//! allocas carry compile-time dissolve registrations — the flush
//! dissolved uninitialized stack garbage (pond `nn::forward` on an
//! empty model).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

use hale_codegen::build_executable;

fn run_src(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

const BUF_AND_MAKE: &str = r#"
    @form(vec)
    locus Buf {
        params { n: Int; }
        capacity { heap data of Float; }
    }

    fn make(n: Int, seed: Float) -> Buf {
        let b = Buf { n: n };
        let mut i = 0;
        while i < n { b.push(seed); i = i + 1; }
        return b;
    }
"#;

#[test]
fn rebind_on_branch_returns_live_capacity() {
    // Repro A: the rebinding branch runs on one call and not the
    // other; BOTH must hand back a live 3-cell buffer (the untaken
    // path used to come back len=0 because the registration is
    // static).
    let src = format!(
        r#"{BUF_AND_MAKE}
        fn rebind(n: Int, take: Bool) -> Buf {{
            let mut a = make(n, 1.0);
            if take {{ a = make(n, 2.0); }}
            return a;
        }}
        fn main() {{
            let f = rebind(3, false);
            println("f len=", to_string(f.len()), " [0]=", to_string(f.get(0) or -999.0));
            let t = rebind(3, true);
            println("t len=", to_string(t.len()), " [0]=", to_string(t.get(0) or -999.0));
        }}
        "#
    );
    let (out, status) = run_src("rebind_branch", &src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("f len=3 [0]=1"), "untaken path: {}", out);
    assert!(out.contains("t len=3 [0]=2"), "taken path: {}", out);
}

#[test]
fn rebind_not_returned_does_not_double_free() {
    // Repro B: same shape, value NOT returned — used to SIGTRAP at
    // frame exit (binding dissolve + temp dissolve on one value).
    let src = format!(
        r#"{BUF_AND_MAKE}
        fn rebind_local(n: Int, take: Bool) -> Int {{
            let mut a = make(n, 1.0);
            if take {{ a = make(n, 2.0); }}
            return a.len();
        }}
        fn main() {{
            println("len=", to_string(rebind_local(3, false)));
            println("len2=", to_string(rebind_local(3, true)));
            println("SURVIVED");
        }}
        "#
    );
    let (out, status) = run_src("rebind_local", &src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("len=3") && out.contains("len2=3"), "{}", out);
    assert!(out.contains("SURVIVED"), "{}", out);
}

#[test]
fn rebind_in_loop_returns_last_value() {
    // The pond `nn::forward` shape: rebound from a factory inside a
    // `while`, returned after the loop.
    let src = format!(
        r#"{BUF_AND_MAKE}
        fn sweep(n: Int, rounds: Int) -> Buf {{
            let mut a = make(n, 1.0);
            let mut r = 0;
            while r < rounds {{
                a = make(n, (a.get(0) or 0.0) + 1.0);
                r = r + 1;
            }}
            return a;
        }}
        fn main() {{
            let s = sweep(3, 4);
            println("len=", to_string(s.len()), " [0]=", to_string(s.get(0) or -999.0));
        }}
        "#
    );
    let (out, status) = run_src("rebind_loop", &src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("len=3 [0]=5"), "{}", out);
}

#[test]
fn rebind_from_ident_survives_both_ways() {
    // `a = nx`: the moved-from binding must not dissolve the value
    // the target returns (returned case) or holds at exit
    // (not-returned case — used to double-free).
    let src = format!(
        r#"{BUF_AND_MAKE}
        fn rebind_ident(n: Int) -> Buf {{
            let mut a = make(n, 1.0);
            let nx = make(n, 5.0);
            a = nx;
            return a;
        }}
        fn rebind_ident_local(n: Int) -> Int {{
            let mut a = make(n, 1.0);
            let nx = make(n, 5.0);
            a = nx;
            return a.len();
        }}
        fn main() {{
            let r = rebind_ident(3);
            println("r len=", to_string(r.len()), " [0]=", to_string(r.get(0) or -999.0));
            println("local len=", to_string(rebind_ident_local(3)));
            println("SURVIVED");
        }}
        "#
    );
    let (out, status) = run_src("rebind_ident", &src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("r len=3 [0]=5"), "{}", out);
    assert!(out.contains("local len=3"), "{}", out);
    assert!(out.contains("SURVIVED"), "{}", out);
}

#[test]
fn dead_branch_unbound_temp_does_not_dissolve_garbage() {
    // An UNBOUND factory temp inside a branch: its dissolve slot is
    // entry-block, its store is branch-local. The untaken path must
    // read the NULL sentinel, not stack garbage.
    let src = format!(
        r#"{BUF_AND_MAKE}
        fn dead_branch(n: Int, take: Bool) -> Int {{
            let mut t = 0;
            if take {{ t = make(n, 3.0).len(); }}
            return t;
        }}
        fn main() {{
            println("f=", to_string(dead_branch(3, false)));
            println("t=", to_string(dead_branch(3, true)));
            println("SURVIVED");
        }}
        "#
    );
    let (out, status) = run_src("dead_branch_temp", &src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("f=0") && out.contains("t=3"), "{}", out);
    assert!(out.contains("SURVIVED"), "{}", out);
}

#[test]
fn early_fail_bypassing_loop_lets_does_not_crash() {
    // The latent second shape: loop-body `let` bindings carry
    // compile-time dissolve registrations on their entry-block
    // allocas; an early `fail` (empty model) bypasses the loop, so
    // the exit flush used to dissolve uninitialized allocas.
    let src = format!(
        r#"{BUF_AND_MAKE}
        type E {{ kind: String; }}

        fn sweep(n: Int, rounds: Int) -> Buf fallible(E) {{
            if rounds <= 0 {{ fail E {{ kind: "neg" }}; }}
            let mut a = make(n, 1.0);
            let mut r = 0;
            while r < rounds {{
                let w = make(n, 2.0);
                a = make(n, (w.get(0) or 0.0) + (a.get(0) or 0.0));
                r = r + 1;
            }}
            return a;
        }}
        fn main() {{
            let ok = sweep(3, 2) or make(3, -1.0);
            println("ok [0]=", to_string(ok.get(0) or -999.0));
            let failed = sweep(3, 0) or make(3, -7.0);
            println("failed [0]=", to_string(failed.get(0) or -999.0));
            println("SURVIVED");
        }}
        "#
    );
    let (out, status) = run_src("fail_bypass_loop_lets", &src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("ok [0]=5"), "{}", out);
    assert!(out.contains("failed [0]=-7"), "{}", out);
    assert!(out.contains("SURVIVED"), "{}", out);
}

#[test]
fn rebind_to_literal_still_works() {
    // Control row from the discriminator matrix: rebinding to a
    // locus LITERAL was never broken and must stay working.
    let src = format!(
        r#"{BUF_AND_MAKE}
        fn rebind_literal(n: Int) -> Buf {{
            let mut a = make(n, 1.0);
            a = Buf {{ n: n }};
            a.push(9.0);
            return a;
        }}
        fn main() {{
            let r = rebind_literal(3);
            println("len=", to_string(r.len()), " [0]=", to_string(r.get(0) or -999.0));
        }}
        "#
    );
    let (out, status) = run_src("rebind_literal", &src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("len=1 [0]=9"), "{}", out);
}
