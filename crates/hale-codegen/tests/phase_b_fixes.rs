//! Phase B follow-up coverage. Each test locks in one of the
//! B-pass fixes from `pond-followup-fixes-phase-bcd-handoff.md`
//! so the relaxed pattern doesn't quietly regress.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_phase_b_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn b1_free_fn_returns_locus_ref() {
    // Free fn returning a locus literal — m90 heap-alloc path
    // routes the instantiation into the lazy global payload
    // arena; the deep-copy epilogue passes the LocusRef
    // pointer through unchanged.
    let src = r#"
        locus Counter {
            params { start: Int = 0; }
            fn show() {
                println("c=" + to_string(self.start));
            }
        }

        fn make(n: Int) -> Counter {
            return Counter { start: n };
        }

        fn main() {
            let c = make(42);
            c.show();
        }
    "#;
    let bin = build("b1_free_fn_locus_ret", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("c=42"), "got: {:?}", stdout);
}

#[test]
fn b2_bytes_literal_lex_roundtrip() {
    // `b"..."` lexes as a bytes literal; codegen lowers via
    // lotus_bytes_from_buf into the caller's arena. Length-
    // safe across embedded NULs (the `\x00` byte is preserved).
    let src = r#"
        fn main() {
            let b = b"ab\x00c";
            println("len=", len(b));
            println("b0=", std::bytes::at(b, 0));
            println("b2=", std::bytes::at(b, 2));
            println("b3=", std::bytes::at(b, 3));
        }
    "#;
    let bin = build("b2_bytes_lit", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=4"), "got: {:?}", stdout);
    assert!(stdout.contains("b0=97"), "got: {:?}", stdout);
    assert!(stdout.contains("b2=0"), "got: {:?}", stdout);
    assert!(stdout.contains("b3=99"), "got: {:?}", stdout);
}

#[test]
fn b2_bytes_literal_full_range_xnn() {
    // The bytes literal accepts the full 0x00..0xFF \xNN range —
    // unlike the string literal which clamps at 0x7f (UTF-8
    // promotion).
    let src = r#"
        fn main() {
            let b = b"\xff\x80\x7f";
            println("b0=", std::bytes::at(b, 0));
            println("b1=", std::bytes::at(b, 1));
            println("b2=", std::bytes::at(b, 2));
        }
    "#;
    let bin = build("b2_bytes_lit_high", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("b0=255"), "got: {:?}", stdout);
    assert!(stdout.contains("b1=128"), "got: {:?}", stdout);
    assert!(stdout.contains("b2=127"), "got: {:?}", stdout);
}

#[test]
fn b3_or_fail_translates_payload() {
    // `or fail X` inside a fallible fn translates a stdlib
    // ParseError into the caller's declared error type.
    let src = r#"
        type AppErr { msg: String; }

        fn parse_to_app(s: String) -> Int fallible(AppErr) {
            return std::str::parse_int(s)
                or fail AppErr { msg: "bad number" };
        }

        fn main() {
            let v = parse_to_app("not-a-number") or -1;
            println("v=", v);
        }
    "#;
    let bin = build("b3_or_fail", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("v=-1"), "got: {:?}", stdout);
}

#[test]
fn b5_bitnot_inverts_int_bits() {
    // `~x` on Int produces ones'-complement (i.e. XOR with -1).
    let src = r#"
        fn main() {
            let a = ~0;
            let b = ~5;
            // `~~` is the closure-approx operator at the lexer
            // level; use a parenthesized inner to force two
            // BitNot ops.
            let c = ~(~7);
            println("a=", a);
            println("b=", b);
            println("c=", c);
        }
    "#;
    let bin = build("b5_bitnot", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a=-1"), "got: {:?}", stdout);
    assert!(stdout.contains("b=-6"), "got: {:?}", stdout);
    assert!(stdout.contains("c=7"), "got: {:?}", stdout);
}

#[test]
fn b7_struct_default_synth_for_all_default_type_field() {
    // A locus param typed as a user struct with every field
    // defaulted gets a synthesized `T { }` default — caller
    // can omit the param at instantiation.
    let src = r#"
        type Cfg {
            host: String = "localhost";
            port: Int = 8080;
        }

        locus Server {
            params { cfg: Cfg; }
            fn show() {
                println("host=" + self.cfg.host);
                println("port=", self.cfg.port);
            }
        }

        fn main() {
            let s = Server { };
            s.show();
        }
    "#;
    let bin = build("b7_default_struct", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("host=localhost"), "got: {:?}", stdout);
    assert!(stdout.contains("port=8080"), "got: {:?}", stdout);
}

#[test]
fn b9_kwself_as_free_fn_arg() {
    // `helper(self, ...)` lowers `self` as the locus self_ptr
    // typed as LocusRef. The free fn receives the locus and
    // reads its field.
    let src = r#"
        locus L {
            params { tag: Int = 7; }
            fn enter() {
                describe(self);
            }
        }

        fn describe(x: L) {
            println("tag=", x.tag);
        }

        fn main() {
            let l = L { };
            l.enter();
        }
    "#;
    let bin = build("b9_kwself_arg", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("tag=7"), "got: {:?}", stdout);
}

#[test]
fn b10_locus_typed_param_back_ref() {
    // Locus-typed locus param — the held locus is caller-
    // owned (stored as a LocusRef ptr). Read-through to the
    // held locus's field works.
    let src = r#"
        locus DB { params { name: String = "db0"; } }

        locus User {
            params {
                db: DB;
                id: Int = 0;
            }
            fn show() {
                println("user=" + to_string(self.id));
                println("db=" + self.db.name);
            }
        }

        fn main() {
            let d = DB { };
            let u = User { db: d, id: 3 };
            u.show();
        }
    "#;
    let bin = build("b10_back_ref", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("user=3"), "got: {:?}", stdout);
    assert!(stdout.contains("db=db0"), "got: {:?}", stdout);
}

#[test]
fn b10_locus_typed_param_forward_ref() {
    // Same as back_ref but the referenced locus is declared
    // AFTER the holder. `pending_locus_names` lets the field
    // type resolve regardless of declaration order.
    let src = r#"
        locus User {
            params {
                db: DB;
                id: Int = 0;
            }
            fn show() {
                println("user=" + to_string(self.id));
                println("db=" + self.db.name);
            }
        }

        locus DB { params { name: String = "db0"; } }

        fn main() {
            let d = DB { };
            let u = User { db: d, id: 9 };
            u.show();
        }
    "#;
    let bin = build("b10_fwd_ref", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("user=9"), "got: {:?}", stdout);
    assert!(stdout.contains("db=db0"), "got: {:?}", stdout);
}

#[test]
fn b11_external_mode_dispatch() {
    // `g.bulk(...)` from outside the locus body dispatches to
    // the locus's `mode bulk { ... }` member. Previously
    // failed because the external-method signature lookup
    // didn't walk LocusMember::Mode.
    let src = r#"
        locus Grinder {
            params { factor: Int = 2; }
            mode bulk {
                println("bulk factor=", self.factor);
            }
        }

        fn main() {
            let g = Grinder { factor: 5 };
            g.bulk();
        }
    "#;
    let bin = build("b11_ext_mode", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bulk factor=5"), "got: {:?}", stdout);
}

#[test]
fn b13_int_float_widens_in_binop() {
    // F.23 Int → Float widening reaches binary-op position.
    // Either side can be the Int that promotes.
    let src = r#"
        fn main() {
            let a = 0.5 + 2;
            let b = 3 * 1.5;
            let c = 7 - 0.25;
            println("a=", a);
            println("b=", b);
            println("c=", c);
        }
    "#;
    let bin = build("b13_widen_binop", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a=2.5"), "got: {:?}", stdout);
    assert!(stdout.contains("b=4.5"), "got: {:?}", stdout);
    assert!(stdout.contains("c=6.75"), "got: {:?}", stdout);
}

#[test]
fn b14_synthetic_field_on_non_self_locus() {
    // `g.draining` (synthetic Bool) reads the same
    // __drain_requested slot as `self.draining`, just from
    // outside the locus body.
    let src = r#"
        locus L {
            params { tag: Int = 1; }
        }

        fn main() {
            let l = L { };
            let d = l.draining;
            println("draining=", d);
        }
    "#;
    let bin = build("b14_synth_field_non_self", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("draining=false"), "got: {:?}", stdout);
}
