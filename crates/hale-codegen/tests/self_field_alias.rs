//! Gap A (2026-07-17) — single-owner value semantics for self-storage
//! heap fields.
//!
//! Pre-fix, the same-arena clone-skip let two self-storage slots share
//! one String blob: `self.g = self.f` (grow path) and struct literals
//! embedding a `self.<field>` read stored the OTHER slot's pointer
//! instead of a copy. The next in-place overwrite of the source slot
//! then silently mutated the aliased slot too — broken value semantics
//! — and anchor retirement would have turned that staleness into
//! use-after-free. Post-fix every such store force-copies
//! (`lotus_str_copy_owned` / the per-field replace fixup), so slots
//! are pairwise-exclusive owners of their blobs.
//!
//! These programs use CONCAT-BUILT strings deliberately: literals stay
//! in .rodata (immutable, shareable) and never exercise the arena
//! paths. That's exactly how the original probe missed the bug.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_self_field_alias_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        output.status.success(),
        "{} crashed: {:?}\nstderr: {}",
        name,
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn direct_string_field_copy_keeps_value_semantics() {
    // `self.g = self.f` must copy; the later in-place shrink of `f`
    // must not leak through into `g`. Pre-fix this printed g=XY2.
    let out = build_and_run(
        "direct",
        r#"
        locus L {
            params { f: String = ""; g: String = ""; }
            fn go() {
                self.f = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" + 1;
                self.g = self.f;
                self.f = "XY" + 2;
                print("g="); println(self.g);
                print("f="); println(self.f);
            }
            run() { self.go(); std::process::exit(0); }
        }
        main locus App { params { l: L = L { }; } run() { } }
        fn main() { App { }; }
    "#,
    );
    assert!(
        out.contains("g=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1"),
        "self.g must keep its own copy after self.f is overwritten \
         in place; got:\n{}",
        out
    );
    assert!(out.contains("f=XY2"), "unexpected f value:\n{}", out);
}

#[test]
fn struct_store_copies_embedded_self_field_reads() {
    // A struct literal embedding a `self.<field>` read, then a
    // whole-struct cross-slot copy. Replacing the source struct and
    // overwriting the read field must leave the copy untouched.
    // Pre-fix this printed b.s=ZZ3 (aliased through self.other).
    let out = build_and_run(
        "struct",
        r#"
        type Cell { s: String = ""; n: Int = 0; }
        locus L {
            params { a: Cell = Cell { }; b: Cell = Cell { }; other: String = ""; }
            fn go() {
                self.other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" + 1;
                self.a = Cell { s: self.other, n: 1 };
                self.b = self.a;
                self.a = Cell { s: "fresh-value-for-a" + 9, n: 2 };
                self.other = "ZZ" + 3;
                print("b.s="); println(self.b.s);
                print("a.s="); println(self.a.s);
                print("other="); println(self.other);
            }
            run() { self.go(); std::process::exit(0); }
        }
        main locus App { params { l: L = L { }; } run() { } }
        fn main() { App { }; }
    "#,
    );
    assert!(
        out.contains("b.s=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1"),
        "self.b.s must keep its own copy across the source struct's \
         replacement and self.other's in-place overwrite; got:\n{}",
        out
    );
    assert!(out.contains("a.s=fresh-value-for-a9"), "unexpected a.s:\n{}", out);
    assert!(out.contains("other=ZZ3"), "unexpected other:\n{}", out);
}

#[test]
fn rmw_and_aliasing_churn_stays_sound() {
    // 50k iterations mixing every retire-sensitive shape: fresh-clone
    // struct replace, whole-struct cross-slot copy then source
    // replace, literal embedding a self read, same-slot RMW
    // round-trips, and direct String grow churn — with every slot
    // re-read each iteration. Catches freelist corruption /
    // double-retire / dangling-slot regressions at the semantic level
    // (the ASan variant of this shape runs in the sanitizer CI job).
    let out = build_and_run(
        "churn",
        r#"
        type Cell { s: String = ""; t: String = ""; n: Int = 0; }
        locus Rec {
            params { a: Cell = Cell { }; b: Cell = Cell { }; f: String = ""; g: String = ""; sink: Int = 0; }
            fn churn(i: Int) {
                self.a = Cell { s: "value." + (i - (i / 100) * 100), t: "" + i, n: i };
            }
            fn alias_struct(i: Int) {
                self.b = self.a;
                self.a = Cell { s: "repl." + i, t: "x" + i, n: i };
            }
            fn alias_literal(i: Int) {
                self.f = "shared." + (i - (i / 50) * 50);
                self.a = Cell { s: self.f, t: "y" + i, n: i };
            }
            fn rmw() {
                self.a = self.a;
                self.g = self.g;
            }
            fn direct(i: Int) {
                self.g = "grow." + i + ".padpadpadpad";
                self.f = self.g;
            }
            fn check() {
                self.sink = self.sink + len(self.a.s) + len(self.a.t)
                    + len(self.b.s) + len(self.b.t) + len(self.f) + len(self.g);
            }
            run() {
                let mut i = 0;
                while i < 50000 {
                    self.churn(i);
                    self.alias_struct(i);
                    self.alias_literal(i);
                    self.rmw();
                    self.direct(i);
                    self.check();
                    i = i + 1;
                }
                print("b.s="); println(self.b.s);
                print("f="); println(self.f);
                std::process::exit(0);
            }
        }
        main locus App { params { r: Rec = Rec { }; } run() { } }
        fn main() { App { }; }
    "#,
    );
    assert!(
        out.contains("b.s=value.99"),
        "b must hold the copy taken before the source's replacement; got:\n{}",
        out
    );
    assert!(
        out.contains("f=grow.49999.padpadpadpad"),
        "f must hold its own copy of g's final value; got:\n{}",
        out
    );
}
