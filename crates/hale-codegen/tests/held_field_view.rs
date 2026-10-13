//! A binding read from a field keeps its value across a write that
//! replaces the field.
//!
//! `let s = self.f.s;` binds the field's own String pointer, not a copy.
//! A callee that replaces `self.f` retires the old String and flushes
//! its retires when it returns, so the next allocation of that size
//! took the block and `s` read it: "NEW-VALUE-2" on v0.21.0, freed memory
//! under ASan once the nested retire (#1542) freed more of them. A
//! same-length String store overwrote the bytes in place, so `let x =
//! self.s; self.s = "NEW";` changed `x` with no call at all.
//!
//! Lowering now binds an owned copy wherever a write that may replace
//! the place comes before a use of the binding (`view_bindings.rs`),
//! and leaves every other binding a view: the copy costs nothing where
//! no such write intervenes.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const PROGRAM: &str = r##"
    type Inner { s: String = ""; }
    type Tag = enum { Named(String), Empty };
    type F {
        n: Int = 0;
        s: String = "";
        inner: Inner = Inner { };
        b: Bytes = b"";
        names: [String; 2] = ["", ""];
        tag: Tag = Tag::Empty;
    }
    fn take(s: String, n: Int) -> String { return s + "!" + to_string(n); }
    locus Kid {
        params { name: String = ""; tag: Tag = Tag::Empty; }
        fn rename(t: String) { self.name = t; }
        fn clear() { self.tag = Tag::Empty; }
        fn tag_it(t: String) { self.tag = Tag::Named(t); }
    }
    // A handle spelled through an alias, returned by a fn.
    type Child = Kid;
    fn child(k: Kid) -> Child { return k; }
    perspective Named {
        fn label() -> String;
        fn rename(t: String);
    }
    locus Badge : serves Named {
        params { name: String = std::str::upper("old-perspective-label"); }
        fn label() -> String { return self.name; }
        fn rename(t: String) { self.name = t; }
    }
    locus H {
        params {
            f: F = F { };
            pair: (String, Int) = ("", 0);
            k: Kid = Kid { };
            badge: perspective(Named) = Badge { };
        }
        fn put(text: String) {
            self.f = F {
                n: 1,
                s: text,
                inner: Inner { s: text },
                b: std::bytes::from_string(text),
                names: [text, text + "2"],
                tag: Tag::Named(text),
            };
            self.pair = (text, 1);
        }
        fn put_inner(text: String) { self.f.inner = Inner { s: text }; }
        // Every replace retires the last one's blocks, and the flush at
        // each `put`'s return hands them to the next.
        fn churn() {
            let mut i = 0;
            while i < 50 { self.put(std::str::upper("new-value-" + to_string(i))); i = i + 1; }
        }
        fn take(s: String) -> String {
            self.churn();
            return s;
        }
        fn held() -> String {
            let s = self.f.s;
            let b = self.f.b;
            let n = self.f.inner.s;
            self.churn();
            return s + "/" + std::str::from_bytes(b) + "/" + n;
        }
        fn field_of_field() -> String {
            let n = self.f.inner.s;
            let mut i = 0;
            while i < 50 { self.put_inner(std::str::upper("inner-" + to_string(i))); i = i + 1; }
            return n;
        }
        fn passed() -> String {
            let s = self.f.s;
            return self.take(s);
        }
        fn passed_direct() -> String {
            return self.take(self.f.s);
        }
        fn each() -> String {
            let mut out = "";
            for x in self.f.names { out = out + x + ","; self.churn(); }
            return out;
        }
        fn matched() -> String {
            return match self.f.tag {
                Tag::Named(s) -> { self.churn(); s },
                Tag::Empty -> "-",
            };
        }
        fn tuple() -> String {
            let (a, n) = self.pair;
            self.churn();
            return a + "#" + to_string(n);
        }
        fn rebound() -> String {
            let mut x = "";
            x = self.f.s;
            self.churn();
            return x;
        }
        fn kid() -> String {
            let s = self.k.name;
            let mut i = 0;
            while i < 50 { self.k.rename(std::str::upper("kid-" + to_string(i))); i = i + 1; }
            return s;
        }
        // A value carried out of a match arm, an `if` and a block, held
        // across the replace (the arm's own binding is used before it).
        fn match_escape() -> String {
            let held = match self.f.tag { Tag::Named(s) -> s, Tag::Empty -> "", };
            self.churn();
            return held;
        }
        fn if_escape() -> String {
            let m = match self.f.tag { Tag::Named(s) -> s, Tag::Empty -> "", };
            let held = if self.f.n > 0 { m } else { "" };
            self.churn();
            return held;
        }
        fn block_escape() -> String {
            let held = { let t = self.f.s; t };
            self.churn();
            return held;
        }
        fn literal_escape() -> String {
            let t = (self.f.s, 1);
            let r = Inner { s: self.f.inner.s };
            self.churn();
            return t.0 + "/" + r.s;
        }
        // An outer local rebound in a loop and read on the next
        // iteration, after the replace the previous one made.
        fn loop_rebound() -> String {
            let mut held = "";
            let mut out = "";
            let mut i = 0;
            while i < 2 { out = out + held + ","; held = self.f.s; self.churn(); i = i + 1; }
            return out;
        }
        fn for_rebound() -> String {
            let mut held = "";
            let mut out = "";
            for k in [1, 2] { out = out + held + ","; held = self.f.s; self.churn(); }
            return out;
        }
        fn nested_rebound() -> String {
            let mut held = "";
            let mut out = "";
            let mut i = 0;
            while i < 2 {
                let mut j = 0;
                while j < 1 { out = out + held + ","; held = self.f.s; self.churn(); j = j + 1; }
                i = i + 1;
            }
            return out;
        }
        // An operand held while a later operand replaces its place: a
        // call's argument, a binary operator's left side, a literal's
        // field.
        fn reset() -> Int {
            self.churn();
            return 0;
        }
        fn operand_arg() -> String {
            return take(match self.f.tag { Tag::Named(s) -> s, Tag::Empty -> "", }, self.reset());
        }
        fn operand_binary() -> String {
            return self.f.s + to_string(self.reset());
        }
        fn operand_literal() -> String {
            let r = Inner { s: self.f.s + to_string(self.reset()) };
            let t = (self.f.inner.s, self.reset());
            return r.s + "/" + t.0;
        }
        fn aliased_handle() -> String {
            let c = child(self.k);
            let held = match c.tag { Tag::Named(s) -> s, Tag::Empty -> "", };
            c.clear();
            let mut i = 0;
            while i < 50 { c.rename(std::str::upper("pad-" + to_string(i))); i = i + 1; }
            return held;
        }
        fn through_perspective() -> String {
            let held = self.badge.label();
            let mut i = 0;
            while i < 50 { self.badge.rename(std::str::upper("new-label-" + to_string(i))); i = i + 1; }
            return held;
        }
        fn in_place() -> String {
            let x = self.f.s;
            self.f.s = std::str::upper("abc");
            return x;
        }
    }
    fn main() {
        let h = H { };
        h.put(std::str::upper("old"));
        println("held=" + h.held());
        h.put(std::str::upper("old"));
        println("field_of_field=" + h.field_of_field());
        h.put(std::str::upper("old"));
        println("passed=" + h.passed());
        h.put(std::str::upper("old"));
        println("passed_direct=" + h.passed_direct());
        h.put(std::str::upper("old"));
        println("each=" + h.each());
        h.put(std::str::upper("old"));
        println("matched=" + h.matched());
        h.put(std::str::upper("old"));
        println("tuple=" + h.tuple());
        h.put(std::str::upper("old"));
        println("rebound=" + h.rebound());
        h.k.rename(std::str::upper("old"));
        println("kid=" + h.kid());
        h.put(std::str::upper("old"));
        println("in_place=" + h.in_place());
        h.k.tag_it(std::str::upper("old-aliased-payload"));
        println("aliased_handle=" + h.aliased_handle());
        println("through_perspective=" + h.through_perspective());
        h.put(std::str::upper("old-argument-payload"));
        println("operand_arg=" + h.operand_arg());
        h.put(std::str::upper("old-argument-payload"));
        println("operand_binary=" + h.operand_binary());
        h.put(std::str::upper("old-argument-payload"));
        println("operand_literal=" + h.operand_literal());
        h.put(std::str::upper("old-matched-payload-text"));
        println("match_escape=" + h.match_escape());
        h.put(std::str::upper("old-matched-payload-text"));
        println("if_escape=" + h.if_escape());
        h.put(std::str::upper("old-block-result-text"));
        println("block_escape=" + h.block_escape());
        h.put(std::str::upper("old-literal-text"));
        println("literal_escape=" + h.literal_escape());
        h.put(std::str::upper("old-loop-binding-text"));
        println("loop_rebound=" + h.loop_rebound());
        h.put(std::str::upper("old-loop-binding-text"));
        println("for_rebound=" + h.for_rebound());
        h.put(std::str::upper("old-loop-binding-text"));
        println("nested_rebound=" + h.nested_rebound());
        println("now=" + h.f.s + "/" + h.k.name);
    }
"##;

/// Every binding of a replaced field's value reads what it was given,
/// under ASan with chunk recycling off: a String, a Bytes and a nested
/// String held across a callee that replaces the field, a nested field
/// replaced on its own, a local and a field passed to the callee, an
/// array field walked by a `for` whose body replaces it, an enum
/// payload matched out of the field, a tuple `let`, a bare local
/// rebound from the field, a child locus's field, and a same-length
/// store in the binding's own method.
#[test]
fn a_binding_held_across_a_replace_reads_the_old_value() {
    let bin = harness::unique_bin("hale_held_field_view");
    harness::build_source_asan(PROGRAM, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    for want in [
        "held=OLD/OLD/OLD",
        "field_of_field=OLD",
        "passed=OLD",
        "passed_direct=OLD",
        "each=OLD,OLD2,",
        "matched=OLD",
        "tuple=OLD#1",
        "rebound=OLD",
        "kid=OLD",
        "in_place=OLD",
        "aliased_handle=OLD-ALIASED-PAYLOAD\n",
        "through_perspective=OLD-PERSPECTIVE-LABEL\n",
        "operand_arg=OLD-ARGUMENT-PAYLOAD!0\n",
        "operand_binary=OLD-ARGUMENT-PAYLOAD0\n",
        "operand_literal=OLD-ARGUMENT-PAYLOAD0/NEW-VALUE-49\n",
        "match_escape=OLD-MATCHED-PAYLOAD-TEXT\n",
        "if_escape=OLD-MATCHED-PAYLOAD-TEXT\n",
        "block_escape=OLD-BLOCK-RESULT-TEXT\n",
        "literal_escape=OLD-LITERAL-TEXT/OLD-LITERAL-TEXT\n",
        "loop_rebound=,OLD-LOOP-BINDING-TEXT,\n",
        "for_rebound=,OLD-LOOP-BINDING-TEXT,\n",
        "nested_rebound=,OLD-LOOP-BINDING-TEXT,\n",
        "now=NEW-VALUE-49/PAD-49",
    ] {
        assert!(stdout.contains(want), "missing {want:?}; stdout: {stdout}; stderr: {stderr}");
    }
}

/// The IR of `H.<name>`.
fn method_ir<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("@H.{name}(");
    let start = ir
        .lines()
        .position(|l| l.starts_with("define") && l.contains(&head))
        .unwrap_or_else(|| panic!("no definition of H.{name} in the IR"));
    let from = ir.lines().take(start).map(|l| l.len() + 1).sum::<usize>();
    let body = &ir[from..];
    &body[..body.find("\n}\n").map(|i| i + 3).unwrap_or(body.len())]
}

/// The copy is only where a write intervenes: a local read from a field
/// and used across a call that writes another field, or before the call
/// that replaces it, stays the field's own pointer, with no clone.
#[test]
fn a_binding_no_write_reaches_stays_a_view() {
    let src = r#"
        type F { s: String = ""; n: Int = 0; }
        locus H {
            params { f: F = F { }; g: F = F { }; count: Int = 0; }
            fn put(t: String) { self.f = F { s: t }; }
            fn bump() { self.count = self.count + 1; }
            fn other() -> String {
                let s = self.f.s;
                self.bump();
                self.g.n = 1;
                return s + "!";
            }
            fn before() -> String {
                let s = self.f.s;
                let out = s + "!";
                self.put("n");
                return out;
            }
            fn across() -> String {
                let s = self.f.s;
                self.put("n");
                return s + "!";
            }
        }
        fn main() {
            let h = H { };
            println(h.other() + h.before() + h.across());
        }
    "#;
    let bin = harness::unique_bin("hale_held_field_view_ir");
    let ir = harness::build_source_ir_text(src, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    for name in ["other", "before"] {
        let body = method_ir(&ir, name);
        assert!(
            !body.contains("_clone_cell_owned"),
            "H.{name} copies a binding no write reaches:\n{body}"
        );
    }
    let across = method_ir(&ir, "across");
    assert!(
        across.contains("call ptr @lotus_str_clone_cell_owned"),
        "H.across holds `s` across the replace without a copy:\n{across}"
    );
}

/// A handle an imported seed hands back, spelled through the seed's
/// alias of its locus (`type Child = Kid;`) — by the seed's own fn and
/// by the consumer's, declared `-> lib::Child`: its payload, matched out
/// and held across the locus's own replace, keeps its value. The
/// analysis classified handles by spelling, so `lib::Child` read as a
/// record value and the payload stayed a view of the retired String.
#[test]
fn an_imported_aliased_handle_keeps_its_value() {
    let lib_src = r#"
        type E = enum { A(String), B };
        locus Kid {
            params { e: E = E::B; }
            fn put(t: String) { self.e = E::A(t); }
            fn clear() { self.e = E::B; }
            fn pad(i: Int) { self.e = E::A(std::str::upper("pad-" + to_string(i))); }
        }
        type Child = Kid;
        fn child(k: Kid) -> Child { return k; }
    "#;
    let consumer_src = r#"
        import "../lib" as lib;
        fn mine(k: lib::Kid) -> lib::Child { return k; }
        locus H {
            params { k: lib::Kid = lib::Kid { }; }
            fn hold() -> String {
                let c = lib::child(self.k);
                let held = match c.e { lib::E::A(s) -> s, lib::E::B -> "", };
                c.clear();
                let mut i = 0;
                while i < 50 { c.pad(i); i = i + 1; }
                return held;
            }
            fn hold_mine() -> String {
                let c = mine(self.k);
                let held = match c.e { lib::E::A(s) -> s, lib::E::B -> "", };
                c.clear();
                let mut i = 0;
                while i < 50 { c.pad(i); i = i + 1; }
                return held;
            }
        }
        fn main() {
            let h = H { };
            h.k.put(std::str::upper("old-imported-payload"));
            println("imported=" + h.hold());
            h.k.put(std::str::upper("old-qualified-alias-payload"));
            println("qualified=" + h.hold_mine());
        }
    "#;
    let dir = harness::unique_dir("hale_held_field_view_import");
    let consumer = dir.join("consumer");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&consumer).expect("create consumer dir");
    std::fs::create_dir_all(&lib).expect("create lib dir");
    std::fs::write(lib.join("thing.hl"), lib_src).expect("write lib");
    let entry = consumer.join("main.hl");
    std::fs::write(&entry, consumer_src).expect("write consumer");
    let bin = harness::unique_bin("hale_held_field_view_import");
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    let built = build_opts::build_seed_dir(&entry, &bin, &options);
    let _ = std::fs::remove_dir_all(&dir);
    built.expect("build consumer + lib");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    for want in ["imported=OLD-IMPORTED-PAYLOAD\n", "qualified=OLD-QUALIFIED-ALIAS-PAYLOAD\n"] {
        assert!(stdout.contains(want), "missing {want:?}; stdout: {stdout}; stderr: {stderr}");
    }
}

/// A payload held across a call through a callback field: a fn-typed
/// field spelled through an alias (`type Callback = fn(H) -> Int;`), an
/// interface-typed field, and a generic locus's `T`-typed field, each
/// running code that replaces the payload's field. The analysis took a
/// callee it could not classify for one that writes nothing — the
/// aliased fn field read as a method named `cb` with an empty summary —
/// so the payload stayed a view of the retired String. A callee it cannot
/// positively identify now writes anything.
#[test]
fn a_binding_held_across_a_callback_reads_the_old_value() {
    let src = r#"
        type E = enum { A(String), B };
        type Callback = fn(H) -> Int;
        interface Resetter { fn fire(h: H) -> Int; }
        locus Hard {
            fn fire(h: H) -> Int {
                h.clear();
                let mut i = 0;
                while i < 50 { h.put(std::str::upper("pad-" + to_string(i))); i = i + 1; }
                return 0;
            }
        }
        locus Hook<T> { params { cb: T; } fn fire(h: H) -> Int { return self.cb(h); } }
        fn reset(h: H) -> Int {
            h.clear();
            let mut i = 0;
            while i < 50 { h.put(std::str::upper("pad-" + to_string(i))); i = i + 1; }
            return 0;
        }
        locus H {
            params {
                e: E = E::A(std::str::upper("old-callback-payload"));
                cb: Callback = reset;
                r: Resetter = Hard { };
            }
            fn clear() { self.e = E::B; }
            fn put(t: String) { self.e = E::A(t); }
            fn aliased() -> String {
                let held = match self.e { E::A(s) -> s, E::B -> "", };
                let n = self.cb(self);
                return held;
            }
            fn through_interface() -> String {
                let held = match self.e { E::A(s) -> s, E::B -> "", };
                let n = self.r.fire(self);
                return held;
            }
            fn generic() -> String {
                let hk: Hook<Callback> = Hook { cb: reset };
                let held = match self.e { E::A(s) -> s, E::B -> "", };
                let n = hk.fire(self);
                return held;
            }
        }
        fn main() {
            let h = H { };
            println("aliased=" + h.aliased());
            h.put(std::str::upper("old-interface-payload"));
            println("interface=" + h.through_interface());
            h.put(std::str::upper("old-generic-payload"));
            println("generic=" + h.generic());
        }
    "#;
    let bin = harness::unique_bin("hale_held_field_view_callback");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    for want in [
        "aliased=OLD-CALLBACK-PAYLOAD\n",
        "interface=OLD-INTERFACE-PAYLOAD\n",
        "generic=OLD-GENERIC-PAYLOAD\n",
    ] {
        assert!(stdout.contains(want), "missing {want:?}; stdout: {stdout}; stderr: {stderr}");
    }
}

/// A record handed to a callee that writes it through its parameter
/// stays the caller's. An argument is passed by reference, so the
/// owned-view copy of an argument the callee may replace is taken only
/// for an immutable leaf (a String, a Bytes): copying the record handed
/// the callee a private copy to fill, and the caller read its own still
/// empty — a Router's captures, filled by the matcher into
/// `cur.params`, came back empty (`hi ` for `hi wire`), once a fn-value
/// call in the matcher's chain made it a callee that may write anything.
#[test]
fn a_record_argument_the_callee_fills_stays_the_callers() {
    let src = r#"
        type P { vals: bounded[String; 4]; }
        type C { p: P = P { }; }
        fn one(x: Int) -> Int { return x; }
        fn make() -> C { return C { }; }
        fn fill(p: P, cb: fn(Int) -> Int) -> Bool {
            let n = cb(1);
            clear(p.vals);
            push(p.vals, std::str::upper("filled")) or raise;
            return true;
        }
        fn hold(mk: fn() -> C) -> Int {
            let cur = mk();
            let ok = fill(cur.p, one);
            return count(cur.p.vals);
        }
        fn main() { println("count=" + to_string(hold(make))); }
    "#;
    let bin = harness::unique_bin("hale_held_field_view_byref");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    assert!(stdout.contains("count=1\n"), "stdout: {stdout}; stderr: {stderr}");
}
