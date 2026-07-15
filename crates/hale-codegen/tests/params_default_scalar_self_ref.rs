//! downstream handoff 2026-07-14 finding 4: `self.<scalar>` inside a
//! nested-locus-literal param default ("no field `fd` on locus
//! self" at codegen; typecheck passed). Root cause: `current_self`
//! took precedence over `params_init_self` in the `self.X` field
//! read, so when the declaring locus was instantiated from inside
//! another locus's method body, the override's `self.fd` resolved
//! against the *enclosing method's* locus. The semantics pinned
//! here: `self` in any expression resolves to the locus whose
//! source text lexically contains it — params-default text
//! (including nested-literal field inits written inside a default)
//! resolves to the declaring locus; a locus-literal field init
//! written in a method body resolves to that method's locus (F.4,
//! see sibling_field_forward_ref.rs).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-pdssr-{}-{}-{}.bin",
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
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

// The reported shape, shared by the quadrant tests below.
const CONN_DECLS: &str = r#"
    locus Ws {
        params { conn_fd: Int = 0; }
    }
    locus Conn {
        params {
            fd: Int = -1;
            conn: Ws = Ws { conn_fd: self.fd };
        }
        birth() { println("conn_fd=", self.conn.conn_fd); }
    }
"#;

#[test]
fn quadrant_a_fn_main_context() {
    // Instantiated from fn main(): current_self is None, so
    // params_init_self already resolved correctly pre-fix.
    let src = format!(
        r#"{CONN_DECLS}
        fn main() {{ Conn {{ }}; }}
    "#
    );
    let (stdout, status) = build_and_run("fn_main", &src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("conn_fd=-1"), "got: {:?}", stdout);
}

#[test]
fn quadrant_b_main_params_default() {
    let src = format!(
        r#"{CONN_DECLS}
        main locus App {{
            params {{ c: Conn = Conn {{ }}; }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    );
    let (stdout, status) = build_and_run("main_params", &src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("conn_fd=-1"), "got: {:?}", stdout);
}

#[test]
fn quadrant_c_method_body_context() {
    // The reported shape: the declaring locus is instantiated
    // inside another locus's method body (a per-connection child
    // born in an accept/dispatch handler). Pre-fix: "no field `fd`
    // on locus self" because current_self (Gateway) shadowed
    // params_init_self (Conn).
    let src = format!(
        r#"{CONN_DECLS}
        locus Gateway {{
            params {{ started: Int = 0; }}
            run() {{
                let c = Conn {{ }};
            }}
        }}
        fn main() {{ Gateway {{ }}; }}
    "#
    );
    let (stdout, status) = build_and_run("method_body", &src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("conn_fd=-1"), "got: {:?}", stdout);
}

#[test]
fn quadrant_d_field_reassign_context() {
    // `self.c = Conn { };` in a method — the field-reassign
    // instantiation path, also a method-body context.
    let src = format!(
        r#"{CONN_DECLS}
        locus Holder {{
            params {{ c: Conn = Conn {{ }}; }}
            fn swap() {{ self.c = Conn {{ }}; }}
            run() {{ self.swap(); }}
        }}
        fn main() {{ Holder {{ }}; }}
    "#
    );
    let (stdout, status) = build_and_run("reassign", &src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, stdout);
    // Once at params-init, once at swap().
    assert_eq!(
        stdout.matches("conn_fd=-1").count(),
        2,
        "expected two births with the declared default; got: {:?}",
        stdout
    );
}

#[test]
fn call_site_override_still_resolves_to_caller() {
    // The F.4 rule this fix must NOT regress: an override written
    // at a method-body call site resolves `self.X` against the
    // CALLER (here M.endpoint), not the instantiated locus.
    let src = r#"
        locus Ws {
            params { conn_fd: Int = 0; }
        }
        locus Conn {
            params {
                fd: Int = -1;
                conn: Ws = Ws { conn_fd: self.fd };
            }
            birth() { println("fd=", self.fd, " conn_fd=", self.conn.conn_fd); }
        }
        locus M {
            params { endpoint: Int = 42; }
            run() {
                let c = Conn { fd: self.endpoint };
            }
        }
        fn main() { M { }; }
    "#;
    let (stdout, status) = build_and_run("call_site", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(
        stdout.contains("fd=42 conn_fd=42"),
        "the override must land the caller's endpoint in fd, and the \
         nested default must read it; got: {:?}",
        stdout
    );
}

#[test]
fn default_reading_later_sibling_is_rejected() {
    // Defaults run in declaration order; pre-guard this GEP+loaded
    // an uninitialized slot silently.
    let src = r#"
        locus L {
            params {
                a: Int = self.b;
                b: Int = 1;
            }
        }
        fn main() { L { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("fwd_ref");
    let err = build_executable(&program, &bin)
        .expect_err("forward-ref default must be rejected");
    let _ = std::fs::remove_file(&bin);
    let msg = format!("{err:?}");
    assert!(
        msg.contains("before it is initialized"),
        "expected the declaration-order error; got: {msg}"
    );
}

#[test]
fn two_level_recursion_resolves_innermost() {
    // A default whose literal's locus has its own self-reading
    // default: each default resolves against its OWN declaring
    // locus (innermost), while the outer literal's explicit init
    // resolves against the outer locus.
    let src = r#"
        locus Inner {
            params {
                x: Int = 3;
                y: Int = self.x + 1;
            }
        }
        locus Outer {
            params {
                n: Int = 7;
                inner: Inner = Inner { };
                m: Int = self.n + 100;
            }
            birth() {
                println("inner.y=", self.inner.y, " m=", self.m);
            }
        }
        fn main() { Outer { }; }
    "#;
    let (stdout, status) = build_and_run("recursion", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(
        stdout.contains("inner.y=4 m=107"),
        "inner defaults resolve to Inner, outer to Outer; got: {:?}",
        stdout
    );
}
