//! A locus returned by a proven-fresh free-fn factory is owned by the
//! binding that names it, and dissolves at that scope's exit
//! (GH #383).
//!
//! Before this, `let m = zeros(n);` leaked: the m90 route places a
//! returned locus in a program-lifetime arena and nothing ever
//! reclaims it — its arena, and any `@form` storage it grew, lived
//! until process exit. In a `fit()`-style loop that is unbounded.
//!
//! Four earlier attempts failed because they had to guess who owned
//! the result. They don't have to any more: since v0.14 a locus
//! value cannot be assigned into a locus-typed field
//! (`check_locus_field_store`), so the binding is the only place a
//! factory result can come to rest. Caller-scoped teardown is then
//! simply correct.
//!
//! Two guards keep it that way, and both are pinned below:
//!
//!  - **Fresh only.** A whitelist analysis
//!    (`compute_fresh_locus_factories`) admits a fn only when every
//!    return is an `L { … }` literal or a single let-binding that is
//!    itself fresh, and that binding never escapes into argument
//!    position or another literal. Fixpointed, so helpers built on
//!    other factories qualify. Anything unrecognized answers NOT
//!    fresh — the old leak, never a double free.
//!  - **Never what the fn hands back.** `compute_returned_bindings`
//!    records, for EVERY free fn, the locals it returns. A fn that
//!    fails the freshness walk can still return a locus it bound
//!    from a factory — `nn::forward` is the real case — and
//!    dissolving that binding hands the caller a dead locus, which
//!    reads back as zeros rather than crashing.
//!
//! Not yet reclaimed, deliberately (see #383): unbound temporaries
//! (`add(matmul(w, a), b)` — the inner result is never named, so a
//! binding-scoped rule cannot reach it) and factories with a second
//! `return <call>` arm.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

use hale_codegen::build_executable;

fn run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

const LIB: &str = r#"
    @form(vec)
    locus Buf { params { n: Int = 0; } capacity { heap data of Float; } }

    fn zeros(n: Int) -> Buf {
        let b = Buf { n: n };
        let mut i = 0;
        while i < n { b.push(0.0); i = i + 1; }
        return b;
    }

    // a helper built on another factory — only qualifies via the
    // fixpoint, which is the point of having one
    fn ramp(n: Int) -> Buf {
        let b = zeros(n);
        let mut i = 0;
        while i < n {
            b.set(i, std::math::int_to_float(i)) or discard;
            i = i + 1;
        }
        return b;
    }
"#;

/// The values must be right, and a long loop must not accumulate.
/// This is the shape that leaked unboundedly.
#[test]
fn factory_results_are_reclaimed_and_still_correct() {
    let src = format!(
        "{LIB}
        locus Engine {{
            params {{ runs: Int = 0; }}
            fn total(n: Int) -> Float {{
                let a = ramp(n);
                let b = zeros(n);
                let mut s = 0.0;
                let mut i = 0;
                while i < n {{
                    let x = a.get(i) or 0.0 - 100.0;
                    let y = b.get(i) or 0.0 - 100.0;
                    s = s + x + y;
                    i = i + 1;
                }}
                self.runs = self.runs + 1;
                return s;
            }}
        }}
        fn main() {{
            let e = Engine {{ }};
            let mut t = 0.0;
            let mut r = 0;
            while r < 50 {{ t = t + e.total(4); r = r + 1; }}
            print(\"t=\"); println(t);
            print(\"runs=\"); println(e.runs);
        }}"
    );
    let (out, st) = run("factory_reclaim_loop", &src);
    assert!(st.success(), "non-zero exit: {:?}\n{}", st, out);
    // ramp(4) sums 0+1+2+3 = 6; zeros contributes 0; 50 rounds.
    assert!(out.contains("t=300"), "values must survive: {:?}", out);
    assert!(out.contains("runs=50"), "got: {:?}", out);
}

/// The guard that cost the most to find: a fn that does NOT qualify
/// as a factory can still hand back a locus it bound from one.
/// Dissolving that binding gives the caller a dead locus — zeros, not
/// a crash, which is why it needs a test rather than a sanitizer.
#[test]
fn a_returned_binding_is_not_dissolved_by_its_own_frame() {
    let src = format!(
        "{LIB}
        // Fails the freshness walk (two different idents returned),
        // but still hands back a locus bound from a factory.
        fn pick(n: Int, which: Int) -> Buf {{
            let lo = zeros(n);
            let hi = ramp(n);
            if which == 0 {{ return lo; }}
            return hi;
        }}
        fn main() {{
            let v = pick(4, 1);
            let a = v.get(3) or 0.0 - 100.0;
            print(\"a=\"); println(a);
            let w = pick(4, 0);
            let b = w.get(3) or 0.0 - 100.0;
            print(\"b=\"); println(b);
        }}"
    );
    let (out, st) = run("factory_reclaim_returned", &src);
    assert!(st.success(), "non-zero exit: {:?}\n{}", st, out);
    assert!(
        out.contains("a=3"),
        "a returned binding must reach the caller alive (ramp(4)[3] = \
         3), not dissolved by the frame that built it: {:?}",
        out
    );
    assert!(out.contains("b=0"), "zeros(4)[3] = 0: {:?}", out);
}

/// A factory result passed as an argument stays alive across the
/// call — the caller still owns it, and using a locus is not
/// transferring it.
#[test]
fn a_factory_result_survives_being_passed_as_an_argument() {
    let src = format!(
        "{LIB}
        locus Reader {{
            params {{ seen: Int = 0; }}
            fn read_arg(v: Buf, i: Int) -> Float {{
                self.seen = self.seen + 1;
                return v.get(i) or 0.0 - 100.0;
            }}
        }}
        fn main() {{
            let r = Reader {{ }};
            let v = ramp(4);
            print(\"one=\"); println(r.read_arg(v, 1));
            print(\"two=\"); println(r.read_arg(v, 2));
            print(\"seen=\"); println(r.seen);
        }}"
    );
    let (out, st) = run("factory_reclaim_arg", &src);
    assert!(st.success(), "non-zero exit: {:?}\n{}", st, out);
    assert!(out.contains("one=1"), "got: {:?}", out);
    assert!(
        out.contains("two=2"),
        "the value must survive the first call: {:?}",
        out
    );
    assert!(out.contains("seen=2"), "got: {:?}", out);
}
