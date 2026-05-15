//! v1.x-FRAMEWORK: publish-site payload stack-alloca.
//!
//! `lower_send` recognizes a bare struct-literal value and
//! stack-allocates the payload in the entry block instead of
//! routing through `lower_user_type_instantiation` (which
//! arena-allocs into the publisher's locus arena). The queue
//! cell's inline buffer is the canonical copy; the publisher
//! pointer dies as soon as `lotus_bus_dispatch` returns.
//!
//! Per-event win on event-flood patterns: no `lotus_arena_alloc`
//! per publish, and no matching arena bloat (≈sizeof(Payload)
//! bytes per publish) on long-running subscribers. Locks in the
//! IR shape so a future refactor of `lower_send` doesn't quietly
//! regress the hot path.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-bus-publish-stack-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ));
    p
}

fn dump_ir(src: &str, tag: &str) -> String {
    let bin = unique_path(tag, "bin");
    let ir = bin.with_extension("ll");
    let program = aperio_syntax::parse_source(src).expect("parse");
    std::env::set_var("LOTUS_DUMP_IR", "1");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");
    let ir_text = std::fs::read_to_string(&ir).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);
    ir_text
}

/// Slice the IR text between `define ... @<fn_name>` and the
/// matching closing `\n}` so we can assert against just that fn.
fn fn_body<'a>(ir: &'a str, fn_name: &str) -> &'a str {
    let define_marker = format!(" @{}(", fn_name);
    let start = ir.find(&define_marker).unwrap_or_else(|| {
        panic!("fn @{} not defined in IR", fn_name)
    });
    let body_start = ir[..start].rfind("define").expect("`define` precedes fn");
    let body_end = ir[body_start..]
        .find("\n}")
        .map(|i| body_start + i + 2)
        .unwrap_or(ir.len());
    &ir[body_start..body_end]
}

#[test]
fn publish_loop_uses_stack_alloca_not_arena_alloc() {
    let src = r#"
        type Tick { n: Int; }

        locus Counter {
            params { count: Int = 0; }
            bus { subscribe "bench.tick" as on_tick of type Tick; }
            fn on_tick(t: Tick) { self.count = self.count + 1; }
        }
        locus Pub {
            params { iters: Int = 100; }
            bus { publish "bench.tick" of type Tick; }
            run() {
                let mut i = 0;
                while i < self.iters {
                    "bench.tick" <- Tick { n: i };
                    i = i + 1;
                }
            }
        }
        fn main() {
            Counter { };
            Pub { iters: 100 };
        }
    "#;
    let ir = dump_ir(src, "stack-alloca");
    let pub_run = fn_body(&ir, "Pub.run");

    // The Tick storage should be a stack alloca in the entry,
    // not a per-iter lotus_arena_alloc inside the loop.
    assert!(
        pub_run.contains("alloca %type.Tick"),
        "expected stack-alloca of %type.Tick in Pub.run:\n{}",
        pub_run,
    );
    assert!(
        !pub_run.contains("call ptr @lotus_arena_alloc"),
        "Pub.run must NOT call lotus_arena_alloc per publish:\n{}",
        pub_run,
    );
    // The publish call itself stays.
    assert!(
        pub_run.contains("call void @lotus_bus_dispatch"),
        "Pub.run should still call lotus_bus_dispatch:\n{}",
        pub_run,
    );
}

#[test]
fn non_struct_payload_still_arena_paths() {
    // When the payload value is NOT a bare struct literal (here:
    // a let-bound variable), the fast path doesn't apply and the
    // value goes through whatever its own lowering provided.
    // The point is just that no regression occurs.
    let src = r#"
        type Tick { n: Int; }

        locus Counter {
            params { count: Int = 0; }
            bus { subscribe "bench.tick" as on_tick of type Tick; }
            fn on_tick(t: Tick) { self.count = self.count + 1; }
        }
        locus Pub {
            params { iters: Int = 3; }
            bus { publish "bench.tick" of type Tick; }
            run() {
                let mut i = 0;
                while i < self.iters {
                    let t = Tick { n: i };
                    "bench.tick" <- t;
                    i = i + 1;
                }
            }
        }
        fn main() {
            Counter { };
            Pub { iters: 3 };
        }
    "#;
    let ir = dump_ir(src, "non-struct-value");
    let pub_run = fn_body(&ir, "Pub.run");
    // The let-bound Tick still uses lower_user_type_instantiation
    // (arena_alloc). The send doesn't get the fast path because
    // value is Expr::Ident, not Expr::Struct.
    assert!(
        pub_run.contains("call ptr @lotus_arena_alloc"),
        "let-bound payload should arena-alloc:\n{}",
        pub_run,
    );
    assert!(
        pub_run.contains("call void @lotus_bus_dispatch"),
        "publish call should still fire:\n{}",
        pub_run,
    );
}
