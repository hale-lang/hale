//! bounded[T; N] (2026-07-02): fixed-capacity counted buffers in
//! types and locus params — `{ i64 len, [N x T] }` inline layout,
//! push/at/count/clear intrinsics, iteration, auto-empty init.

use std::process::Command;

use hale_codegen::build_executable;
use hale_syntax::parse_source;

fn build_and_run(name: &str, src: &str) -> String {
    let program = parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    static NEXT: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    bin.push(format!(
        "hale_bounded_{}_{}_{}",
        name,
        std::process::id(),
        n
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn type_field_full_lifecycle() {
    let out = build_and_run(
        "ty",
        r#"
        type Recent {
            tag: Int;
            vals: bounded[Int; 8];
        }
        fn main() {
            let r = Recent { tag: 7 };
            println("count0=", count(r.vals));
            let mut i = 0;
            while i < 8 {
                push(r.vals, i * 10) or raise;
                i = i + 1;
            }
            println("count8=", count(r.vals));
            push(r.vals, 999)
                or println("full: cap=", err.cap, " count=", err.count);
            let v3 = at(r.vals, 3) or raise;
            println("v3=", v3);
            let oob = at(r.vals, 42) or 0 - 1;
            println("oob=", oob);
            let mut sum = 0;
            for x in r.vals {
                sum = sum + x;
            }
            println("sum=", sum);
            clear(r.vals);
            println("cleared=", count(r.vals));
        }
    "#,
    );
    for want in [
        "count0=0",
        "count8=8",
        "full: cap=8 count=8",
        "v3=30",
        "oob=-1",
        "sum=280",
        "cleared=0",
    ] {
        assert!(out.contains(want), "missing {:?} in {:?}", want, out);
    }
}

#[test]
fn locus_params_field_with_self_receivers() {
    let out = build_and_run(
        "locus",
        r#"
        locus Tracker {
            params { name: String = "t"; recent: bounded[Float; 4]; }
            fn note(v: Float) -> Int {
                push(self.recent, v) or (clear(self.recent));
                return count(self.recent);
            }
            fn total() -> Float {
                let mut s = 0.0;
                for x in self.recent {
                    s = s + x;
                }
                return s;
            }
        }
        fn main() {
            let t = Tracker { };
            println("c1=", t.note(1.5));
            println("c2=", t.note(2.5));
            println("total=", t.total());
        }
    "#,
    );
    assert!(out.contains("c1=1"), "got: {:?}", out);
    assert!(out.contains("c2=2"), "got: {:?}", out);
    assert!(out.contains("total=4"), "got: {:?}", out);
}

#[test]
fn whole_struct_copy_carries_elements() {
    // The bounded storage is inline — a struct copy must carry the
    // live elements and count (deep-correct by construction).
    let out = build_and_run(
        "copy",
        r#"
        type Box { vals: bounded[Int; 4]; }
        fn main() {
            let a = Box { };
            push(a.vals, 5) or raise;
            push(a.vals, 6) or raise;
            let b = a;
            let mut s = 0;
            for x in b.vals {
                s = s + x;
            }
            println("bsum=", s);
            println("bcount=", count(b.vals));
        }
    "#,
    );
    assert!(out.contains("bsum=11"), "got: {:?}", out);
    assert!(out.contains("bcount=2"), "got: {:?}", out);
}

#[test]
fn float_elem_int_widening_on_push() {
    let out = build_and_run(
        "widen",
        r#"
        type W { vals: bounded[Float; 4]; }
        fn main() {
            let w = W { };
            push(w.vals, 2) or raise;
            push(w.vals, 0.5) or raise;
            let mut s = 0.0;
            for x in w.vals {
                s = s + x;
            }
            println("s=", s);
        }
    "#,
    );
    assert!(out.contains("s=2.5"), "got: {:?}", out);
}
