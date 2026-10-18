//! bounded[T; N] (2026-07-02): fixed-capacity counted buffers in
//! types and locus params — `{ i64 len, [N x T] }` inline layout,
//! push/at/count/clear intrinsics, iteration, auto-empty init.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_and_run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(name);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
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

/// A whole assignment of a locus's bounded field from a record built in
/// a method, its Strings computed there: the field's elements are
/// anchored in the locus's arena, not the method's scratch, so they are
/// whole after the method returns and its scratch is freed. Under
/// AddressSanitizer (with the chunk pool off, which an ASan build
/// defaults), a slot left in the scratch is a use-after-free at the
/// read. A record local's field and a nested record field of the
/// locus's are the other two places.
#[test]
fn a_whole_assigned_bounded_field_outlives_its_method_under_asan() {
    let src = r#"
        type R { xs: bounded[String; 2]; }
        type Row { name: String = ""; xs: bounded[String; 2]; }
        locus L {
            params { xs: bounded[String; 2]; row: Row = Row { }; }
            fn put(k: Int) {
                let mut r = R { };
                push(r.xs, "first " + to_string(k)) or raise;
                push(r.xs, std::str::upper("second ") + to_string(k)) or raise;
                self.xs = r.xs;
                let mut local = Row { name: "l" };
                local.xs = r.xs;
                self.row.xs = local.xs;
            }
            fn read() -> String {
                return (at(self.xs, 0) or "") + "|" + (at(self.xs, 1) or "") + "|" + (at(self.row.xs, 1) or "");
            }
        }
        fn main() {
            let l = L { };
            let mut k = 0;
            while k < 50 {
                l.put(k);
                k = k + 1;
            }
            println(l.read());
        }
    "#;
    let bin = harness::unique_bin("bounded_whole_assign_asan");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("asan build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && !stderr.contains("AddressSanitizer"), "{:?}: {stderr}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "first 49|SECOND 49|SECOND 49\n");
}

/// A bounded local is a view of the place it was bound from, and the
/// elements a `push` or a `set` stores through it are anchored where that
/// place lives: a locus's own field (`let mut dest = self.xs`), a child's
/// (`let d = self.child.xs`), an alias of a view (`let e = d`), and an
/// inner block's view of another place shadowing an outer one. Each used
/// to anchor in the method's scratch, which its return frees under the
/// field. Under AddressSanitizer (chunk recycling off, as an ASan build
/// defaults), a slot left there is a use-after-free at the read after the
/// methods returned.
#[test]
fn an_element_stored_through_a_bounded_view_outlives_its_method_under_asan() {
    let src = r#"
        locus Child {
            params { xs: bounded[String; 4]; }
            fn first() -> String { return at(self.xs, 0) or ""; }
        }
        locus L {
            params { child: Child = Child { }; xs: bounded[String; 4]; ys: bounded[String; 4]; }
            fn put(k: Int) {
                let mut dest = self.xs;
                clear(dest);
                push(dest, "first " + to_string(k)) or raise;
                push(dest, std::str::upper("second ") + to_string(k)) or raise;
                set(dest, 0, "set " + to_string(k)) or raise;
                let d = self.child.xs;
                clear(d);
                push(d, "child " + to_string(k)) or raise;
                let e = d;
                set(e, 0, "alias " + to_string(k)) or raise;
                let v = self.ys;
                if k > 0 {
                    let v = self.child.xs;
                    push(v, "inner " + to_string(k)) or discard;
                }
                clear(v);
                push(v, "outer " + to_string(k)) or raise;
            }
            fn read() -> String {
                return (at(self.xs, 0) or "") + "|" + (at(self.xs, 1) or "") + "|" + self.child.first() + "|" + (at(self.ys, 0) or "");
            }
        }
        fn main() {
            let l = L { };
            let mut k = 0;
            while k < 50 {
                l.put(k);
                k = k + 1;
            }
            println(l.read());
        }
    "#;
    let bin = harness::unique_bin("bounded_view_anchor_asan");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("asan build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && !stderr.contains("AddressSanitizer"), "{:?}: {stderr}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "set 49|SECOND 49|alias 49|outer 49\n");
}

/// GH #1608 review: a bounded place is evaluated once. A view bound from an
/// indexed place (`let v = self.slots[self.next()].xs;`) calls the index's
/// method once, and the arena the view's elements anchor in is the owner
/// that same evaluation reached: the IR holds one call of `next`, and the
/// element pushed through the view reads whole after the method returned.
#[test]
fn an_indexed_bounded_place_is_evaluated_once() {
    let src = r#"
        locus Slot { params { xs: bounded[String; 4]; } fn first() -> String { return at(self.xs, 0) or ""; } }
        locus Store {
            params { slots: [Slot; 2] = [Slot { }, Slot { }]; calls: Int = 0; }
            fn next() -> Int { self.calls = self.calls + 1; return 0; }
            fn view(k: Int) {
                let v = self.slots[self.next()].xs;
                clear(v);
                push(v, "viewed " + to_string(k)) or raise;
            }
        }
        fn main() {
            let s = Store { };
            s.view(1);
            s.view(2);
            println(s.calls);
            println(s.slots[0].first());
        }
    "#;
    let bin = harness::unique_bin("bounded_indexed_once");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && !stderr.contains("AddressSanitizer"), "{:?}: {stderr}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "2\nviewed 2\n");

    let ir_bin = harness::unique_bin("bounded_indexed_once_ir");
    let ir = harness::build_source_ir_text(src, &ir_bin).expect("ir");
    let _ = std::fs::remove_file(&ir_bin);
    for f in ["Store.view"] {
        let body = ir_fn_body(&ir, f).unwrap_or_else(|| panic!("no `{f}` in the IR"));
        let calls = body.lines().filter(|l| l.contains("call") && l.contains("Store.next")).count();
        assert_eq!(calls, 1, "`{f}` calls `next` once:\n{body}");
    }
}

/// The body of the IR function whose name contains `name`.
fn ir_fn_body(ir: &str, name: &str) -> Option<String> {
    let start = ir.lines().position(|l| l.starts_with("define") && l.contains(&format!("@\"{name}\"")) || l.starts_with("define") && l.contains(&format!("@{name}(")))?;
    let lines: Vec<&str> = ir.lines().skip(start).collect();
    let end = lines.iter().position(|l| *l == "}")?;
    Some(lines[..=end].join("\n"))
}

/// GH #1608 follow-up: the shape the check gives in place of an element
/// stored through a parameter's place: a free fn fills a record of the
/// calling frame, and the method stores it whole into its field, which
/// anchors the elements in the locus's arena. Under AddressSanitizer
/// (chunk recycling off) the field reads whole after the methods returned.
/// The refused member shape is refused by the build too.
#[test]
fn a_record_filled_by_a_free_fn_and_stored_whole_outlives_its_method_under_asan() {
    let src = r#"
        type Row { xs: bounded[String; 4]; }
        fn fill(r: Row, k: Int) {
            clear(r.xs);
            push(r.xs, "free fn " + to_string(k)) or raise;
            push(r.xs, std::str::upper("second ") + to_string(k)) or raise;
        }
        locus L {
            params { row: Row = Row { }; }
            fn put(k: Int) {
                let mut mine = Row { };
                fill(mine, k);
                self.row = mine;
            }
            fn read() -> String { return (at(self.row.xs, 0) or "") + "|" + (at(self.row.xs, 1) or ""); }
        }
        fn main() {
            let l = L { };
            let mut k = 0;
            while k < 50 {
                l.put(k);
                k = k + 1;
            }
            println(l.read());
        }
    "#;
    let bin = harness::unique_bin("bounded_param_place_asan");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("asan build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && !stderr.contains("AddressSanitizer"), "{:?}: {stderr}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "free fn 49|SECOND 49\n");

    let refused = r#"
        type Row { xs: bounded[String; 4]; }
        locus L {
            params { row: Row = Row { }; }
            fn through(r: Row, k: Int) { push(r.xs, "m" + to_string(k)) or raise; }
            fn put(k: Int) { self.through(self.row, k); }
        }
        fn main() { let l = L { }; l.put(1); }
    "#;
    let bin = harness::unique_bin("bounded_param_place_refused");
    let err = build_opts::build_source(refused, &bin, &build_opts::options()).expect_err("the member shape is refused");
    let _ = std::fs::remove_file(&bin);
    assert!(format!("{err:?}").contains("under the member parameter `r`"), "{err:?}");
}

/// Review of #1612: the member-parameter backstop follows the binding, so a
/// `let` that shadows the parameter's name binds a fresh record of the
/// method's own, which builds and runs.
#[test]
fn a_local_shadowing_a_member_parameter_is_its_own_binding() {
    let src = r#"
        type Row { xs: bounded[String; 4]; }
        locus L {
            fn put(r: Row) -> Int {
                let r = Row { };
                push(r.xs, "fresh" + to_string(1)) or raise;
                return count(r.xs);
            }
        }
        fn main() { let l = L { }; println(l.put(Row { })); }
    "#;
    assert_eq!(build_and_run("bounded_member_param_shadow", src), "1\n");
}
