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

// ── Stage 1: pointer-shaped elements ──

#[test]
fn string_elements_survive_cross_fn_and_return_copy() {
    // Elements pushed from another fn's frame (scratch strings) and
    // a struct returned through the deep-copy path — dangling
    // pointers would print garbage.
    let out = build_and_run(
        "strelem",
        r#"
        type Params {
            n: Int;
            keys: bounded[String; 16];
            vals: bounded[String; 16];
        }
        fn add_pair(p: Params, k: String, v: String) {
            push(p.keys, k) or raise;
            push(p.vals, v) or raise;
        }
        fn build() -> Params {
            let p = Params { n: 3 };
            let mut i = 0;
            while i < 3 {
                add_pair(p, "route_k" + to_string(i), "v" + to_string(i));
                i = i + 1;
            }
            return p;
        }
        fn main() {
            let p = build();
            println("count=", count(p.keys));
            let mut i = 0;
            while i < 3 {
                let k = at(p.keys, i) or "?";
                let v = at(p.vals, i) or "?";
                println(k, "=", v);
                i = i + 1;
            }
        }
    "#,
    );
    assert!(out.contains("count=3"), "got: {:?}", out);
    for want in ["route_k0=v0", "route_k1=v1", "route_k2=v2"] {
        assert!(out.contains(want), "missing {:?} in {:?}", want, out);
    }
}

#[test]
fn struct_elements_in_type_and_locus_field() {
    let out = build_and_run(
        "structelem",
        r#"
        type Msg { role: String; text: String; }
        type Convo { history: bounded[Msg; 64]; }
        fn say(c: Convo, role: String, text: String) {
            push(c.history, Msg { role: role, text: text }) or raise;
        }
        locus Agent {
            params { convo: Convo = Convo { }; }
            fn hear(t: String) -> Int {
                push(self.convo.history, Msg { role: "user", text: t })
                    or raise;
                return count(self.convo.history);
            }
            fn dump() {
                for m in self.convo.history {
                    println(m.role, ": ", m.text);
                }
            }
        }
        fn main() {
            let c = Convo { };
            let mut i = 0;
            while i < 40 {
                say(c, "sys", "msg_" + to_string(i * 7));
                i = i + 1;
            }
            println("c40=", count(c.history));
            let m39 = at(c.history, 39) or raise;
            println("last=", m39.text);
            let a = Agent { };
            println("h1=", a.hear("hello"));
            a.dump();
        }
    "#,
    );
    assert!(out.contains("c40=40"), "got: {:?}", out);
    assert!(out.contains("last=msg_273"), "got: {:?}", out);
    assert!(out.contains("h1=1"), "got: {:?}", out);
    assert!(out.contains("user: hello"), "got: {:?}", out);
}

#[test]
fn scalar_bounded_travels_the_bus() {
    let out = build_and_run(
        "bus",
        r#"
        type Window { id: Int; samples: bounded[Float; 16]; }
        locus Listener {
            params { seen: Int = 0; }
            bus { subscribe "win" as on_win of type Window; }
            fn on_win(w: Window) {
                let mut s = 0.0;
                for x in w.samples {
                    s = s + x;
                }
                println("got id=", w.id, " n=", count(w.samples),
                        " sum=", s);
            }
        }
        locus Sender {
            params { l: Listener = Listener { }; }
            bus { publish "win" of type Window; }
            run() {
                let w = Window { id: 42 };
                push(w.samples, 1.5) or raise;
                push(w.samples, 2.25) or raise;
                push(w.samples, 3.0) or raise;
                "win" <- w;
            }
        }
        fn main() { Sender { }; }
    "#,
    );
    assert!(
        out.contains("got id=42 n=3 sum=6.75"),
        "got: {:?}",
        out
    );
}

#[test]
fn set_truncate_drop_front_idiom() {
    let out = build_and_run(
        "settrunc",
        r#"
        type Buf { vals: bounded[String; 8]; }
        fn main() {
            let b = Buf { };
            let mut i = 0;
            while i < 6 {
                push(b.vals, "m" + to_string(i)) or raise;
                i = i + 1;
            }
            let n = count(b.vals);
            let k = 2;
            let mut j = 0;
            while j < n - k {
                let v = at(b.vals, j + k) or "?";
                set(b.vals, j, v) or raise;
                j = j + 1;
            }
            truncate(b.vals, n - k);
            println("count=", count(b.vals));
            let first = at(b.vals, 0) or "?";
            let last = at(b.vals, 3) or "?";
            println(first, " ", last);
            set(b.vals, 99, "nope")
                or println("oob idx=", err.index);
            println("t0=", truncate(b.vals, 0));
        }
    "#,
    );
    assert!(out.contains("count=4"), "got: {:?}", out);
    assert!(out.contains("m2 m5"), "got: {:?}", out);
    assert!(out.contains("oob idx=99"), "got: {:?}", out);
    assert!(out.contains("t0=0"), "got: {:?}", out);
}
