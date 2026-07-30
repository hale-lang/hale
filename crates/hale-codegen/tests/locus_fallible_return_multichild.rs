//! Locus returned through a fallible context carries its
//! `@form(hashmap)` child loci to the caller without
//! corrupting the latter children's storage.
//!
//! Pre-fix: m90 routed the outer (returning) locus to the
//! payload arena, but its `@form(hashmap)` child loci stayed
//! in `alloca_in_entry_with_nulled_arena` slots on the
//! returning fn's stack frame. After the fn returned, those
//! stack allocas were invalid — the outer struct's slot
//! pointers became dangling. Some children survived because
//! the post-return stack hadn't been overwritten yet; the
//! last few corrupted to garbage / zero `len()`. The repro
//! shape:
//!
//!   fn build() -> Mapper fallible(Err) {
//!       let m = Mapper { a, b, c, d };
//!       // populate ...
//!       return m;
//!   }
//!
//! Post-fix: the outer locus's instantiation sets the
//! `instantiating_into_payload_arena` flag during its
//! params-init loop, so each child literal's
//! `lower_locus_instantiation` also routes to payload arena.
//! Children's storage survives the fn return alongside the
//! outer struct.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-mapper-fallible-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn fallible_return_carries_all_form_hashmap_children() {
    // Four distinct `@form(hashmap)` child loci, returned
    // through `fallible(Err)`. Each must keep its own
    // `len()` across the return.
    let src = r#"
        type Cell { name: String; v: Int = 0; }

        @form(hashmap)
        locus M1 { capacity { pool entries of Cell indexed_by name; } }
        @form(hashmap)
        locus M2 { capacity { pool entries of Cell indexed_by name; } }
        @form(hashmap)
        locus M3 { capacity { pool entries of Cell indexed_by name; } }
        @form(hashmap)
        locus M4 { capacity { pool entries of Cell indexed_by name; } }

        locus Mapper {
            params {
                a: M1;
                b: M2;
                c: M3;
                d: M4;
            }
            fn insert_a(x: Cell) { self.a.set(x); }
            fn insert_b(x: Cell) { self.b.set(x); }
            fn insert_c(x: Cell) { self.c.set(x); }
            fn insert_d(x: Cell) { self.d.set(x); }
            fn len_a() -> Int { return self.a.len(); }
            fn len_b() -> Int { return self.b.len(); }
            fn len_c() -> Int { return self.c.len(); }
            fn len_d() -> Int { return self.d.len(); }
        }

        type Err { msg: String; }

        fn build() -> Mapper fallible(Err) {
            let m = Mapper {
                a: M1 { }, b: M2 { }, c: M3 { }, d: M4 { },
            };
            m.insert_a(Cell { name: "x1", v: 1 });
            m.insert_a(Cell { name: "x2", v: 2 });
            m.insert_b(Cell { name: "y1", v: 10 });
            m.insert_b(Cell { name: "y2", v: 20 });
            m.insert_b(Cell { name: "y3", v: 30 });
            m.insert_c(Cell { name: "z1", v: 100 });
            m.insert_c(Cell { name: "z2", v: 200 });
            m.insert_c(Cell { name: "z3", v: 300 });
            m.insert_c(Cell { name: "z4", v: 400 });
            m.insert_d(Cell { name: "w1", v: 1000 });
            m.insert_d(Cell { name: "w2", v: 2000 });
            return m;
        }

        fn main() {
            let m = build() or raise;
            println("a=", m.len_a(), " b=", m.len_b(),
                    " c=", m.len_c(), " d=", m.len_d());
        }
    "#;
    let (stdout, status) = build_and_run("four-child", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status,
        stdout
    );
    assert!(
        stdout.contains("a=2 b=3 c=4 d=2"),
        "expected all four child counts to survive the fallible \
         return; got: {}",
        stdout
    );
}

#[test]
fn non_fallible_return_carries_form_hashmap_children() {
    // Same shape, plain -> Mapper (no fallible). Should also
    // route children through payload arena.
    let src = r#"
        type Cell { name: String; v: Int = 0; }

        @form(hashmap)
        locus M1 { capacity { pool entries of Cell indexed_by name; } }
        @form(hashmap)
        locus M2 { capacity { pool entries of Cell indexed_by name; } }
        @form(hashmap)
        locus M3 { capacity { pool entries of Cell indexed_by name; } }

        locus Mapper {
            params {
                a: M1;
                b: M2;
                c: M3;
            }
            fn la() -> Int { return self.a.len(); }
            fn lb() -> Int { return self.b.len(); }
            fn lc() -> Int { return self.c.len(); }
        }

        fn build() -> Mapper {
            let m = Mapper { a: M1 { }, b: M2 { }, c: M3 { } };
            m.a.set(Cell { name: "x", v: 1 });
            m.b.set(Cell { name: "y1", v: 10 });
            m.b.set(Cell { name: "y2", v: 20 });
            m.c.set(Cell { name: "z1", v: 100 });
            m.c.set(Cell { name: "z2", v: 200 });
            m.c.set(Cell { name: "z3", v: 300 });
            return m;
        }

        fn main() {
            let m = build();
            println("a=", m.la(), " b=", m.lb(), " c=", m.lc());
        }
    "#;
    let (stdout, status) = build_and_run("nonfallible", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status,
        stdout
    );
    assert!(
        stdout.contains("a=1 b=2 c=3"),
        "expected child counts to survive non-fallible return; \
         got: {}",
        stdout
    );
}
