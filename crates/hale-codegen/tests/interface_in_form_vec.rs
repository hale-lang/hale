//! A10 (G20 + F.20 Phase B) — Interface values stored in
//! `@form(vec)` cells, and interface fat-pointer aliasing in
//! locus fields.
//!
//! Two surfaces verified:
//!
//! 1. **Interface as `@form(vec)` cell type.** Before A10, the
//!    `push` / `set` arg-type checks demanded strict equality, so
//!    a `LocusRef` of a satisfying locus couldn't flow into an
//!    `Interface`-typed cell. The fix adds the standard
//!    locus → interface coercion (`coerce_to_interface`) the same
//!    way `lower_user_fn_call` does at regular call sites.
//!
//! 2. **Fat-pointer aliasing through stored interfaces.** A
//!    locus field of interface type stores a pointer to the
//!    fat-pointer struct whose `data` slot is the underlying
//!    locus's `self`. Method dispatch through the interface
//!    reads + writes the underlying locus's region in place
//!    (no copy at the field-store site). The trade/backtest
//!    PnL=0 bug was a workaround target for exactly this surface.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_iface_in_form_vec_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn interface_push_get_dispatches_polymorphically() {
    let src = r#"
        interface Greeter {
            fn greet() -> String;
        }
        locus Hi {
            fn greet() -> String { return "hello"; }
        }
        locus Hey {
            fn greet() -> String { return "hey there"; }
        }
        @form(vec)
        locus Greeters {
            capacity { heap items of Greeter; }
        }
        fn main() {
            let gs = Greeters { };
            gs.push(Hi { });
            gs.push(Hey { });
            let mut i = 0;
            while i < 2 {
                let g = gs.get(i) or raise;
                println(g.greet());
                i = i + 1;
            }
        }
    "#;
    let (stdout, status) = build_and_run("push_get_dispatch", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("hello"), "missing first: {:?}", stdout);
    assert!(stdout.contains("hey there"), "missing second: {:?}", stdout);
}

#[test]
fn interface_field_mutation_aliases_underlying_locus() {
    // A10 part 2: storing an interface in a locus field and
    // mutating through it must write through to the underlying
    // locus's region. Both the field-side read and the direct
    // (locus-ref-side) read should see the same count.
    let src = r#"
        interface Counter {
            fn tick();
            fn current() -> Int;
        }
        locus Cnt {
            params { n: Int = 0; }
            fn tick() { self.n = self.n + 1; }
            fn current() -> Int { return self.n; }
        }
        locus Holder {
            params { c: Counter; }
            fn bump() { self.c.tick(); }
        }
        fn main() {
            let c = Cnt { };
            let h = Holder { c: c };
            h.bump();
            h.bump();
            h.bump();
            println("via field: " + h.c.current());
            println("via direct: " + c.current());
        }
    "#;
    let (stdout, status) = build_and_run("field_alias", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("via field: 3"),
        "field-side should see 3 ticks: {:?}",
        stdout
    );
    assert!(
        stdout.contains("via direct: 3"),
        "locus-side should also see 3 ticks (aliasing): {:?}",
        stdout
    );
}

#[test]
fn interface_in_form_vec_with_mutation_aliases() {
    // A10: combined. @form(vec) of interface-typed cells,
    // mutated through the vec, then read through both the vec
    // and the original locus refs. All four reads should
    // converge on the same count.
    let src = r#"
        interface Counter {
            fn tick();
            fn current() -> Int;
        }
        locus Cnt {
            params { n: Int = 0; }
            fn tick() { self.n = self.n + 1; }
            fn current() -> Int { return self.n; }
        }
        @form(vec)
        locus CntList {
            capacity { heap items of Counter; }
        }
        fn main() {
            let cs = CntList { };
            let c1 = Cnt { };
            let c2 = Cnt { };
            cs.push(c1);
            cs.push(c2);
            let mut i = 0;
            while i < 2 {
                let c = cs.get(i) or raise;
                c.tick();
                c.tick();
                i = i + 1;
            }
            println("c1 direct: " + c1.current());
            println("c2 direct: " + c2.current());
            let cr1 = cs.get(0) or raise;
            let cr2 = cs.get(1) or raise;
            println("c1 via vec: " + cr1.current());
            println("c2 via vec: " + cr2.current());
        }
    "#;
    let (stdout, status) = build_and_run("form_vec_alias", src);
    assert!(status.success(), "non-zero: {:?}", status);
    for needle in [
        "c1 direct: 2",
        "c2 direct: 2",
        "c1 via vec: 2",
        "c2 via vec: 2",
    ] {
        assert!(stdout.contains(needle), "missing `{}`: {:?}", needle, stdout);
    }
}
