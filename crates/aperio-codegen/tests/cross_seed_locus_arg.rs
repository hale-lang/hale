//! cross-seed-locus-arg-segv — pins the fix shut.
//!
//! A non-fallible free fn whose body constructs a `type` literal
//! and pushes it onto a `@form(vec)` slot on a locus-typed
//! argument used to allocate the literal in the per-call
//! subregion (m49). The push stored a pointer; the subregion's
//! destroy at fn exit then freed the storage; any later
//! `.get(i)` from any caller dereferenced the dangler and
//! segfaulted.
//!
//! Surfaced by pond/agent/tools FRICTION
//! (`cross-seed-locus-arg-segv`). The fix routes free-fn body
//! allocations through the caller arena instead of the per-call
//! subregion (see codegen.rs `current_arena_ptr`).
//!
//! These tests pin the original symptom and a few related shapes
//! that share the underlying lifetime: the cross-seed dimension
//! isn't the cause, just the most-common surface — the same
//! corruption fired for any free fn body that constructs a value
//! and pushes it onto a foreign locus's vec.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_cross_seed_locus_arg_{}_{}",
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
fn free_fn_push_then_get_survives_subregion_destroy() {
    // The minimal repro: a free fn takes a Registry, constructs an
    // Entry inside its body, pushes onto reg.entries. Then main
    // calls .get(0). Before the fix this segfaulted because the
    // Entry was alloc'd in the callee's subregion (destroyed at
    // fn exit) and the vec held the dangling pointer.
    let src = r#"
        type Entry { name: String; value: Int; }

        @form(vec)
        locus EntryList {
            capacity { heap items of Entry; }
        }

        locus Registry {
            params { entries: EntryList = EntryList { }; }
        }

        fn push_via_freefn(reg: Registry, n: String, v: Int) {
            reg.entries.push(Entry { name: n, value: v });
        }

        fn main() {
            let reg = Registry { };
            push_via_freefn(reg, "via-fn", 7);
            println("len=", to_string(reg.entries.len()));
            let e = reg.entries.get(0) or Entry { name: "FB", value: -1 };
            println("name=", e.name, " value=", to_string(e.value));
        }
    "#;
    let (stdout, status) = build_and_run("push_then_get", src);
    assert!(status.success(), "non-zero (segv?): {:?}", status);
    assert!(stdout.contains("len=1"), "stdout: {:?}", stdout);
    assert!(stdout.contains("name=via-fn"), "stdout: {:?}", stdout);
    assert!(stdout.contains("value=7"), "stdout: {:?}", stdout);
}

#[test]
fn interleaved_direct_and_freefn_pushes_both_survive() {
    // Two pushes: one direct from main (alloc'd in main's arena),
    // one through the free fn (was the bug — alloc'd in
    // subregion). After the fix both entries survive `.get`.
    let src = r#"
        type Entry { name: String; value: Int; }

        @form(vec)
        locus EntryList {
            capacity { heap items of Entry; }
        }

        locus Registry {
            params { entries: EntryList = EntryList { }; }
        }

        fn push_via_freefn(reg: Registry, n: String, v: Int) {
            reg.entries.push(Entry { name: n, value: v });
        }

        fn main() {
            let reg = Registry { };
            reg.entries.push(Entry { name: "direct", value: 1 });
            push_via_freefn(reg, "via-fn", 2);
            println("len=", to_string(reg.entries.len()));
            let e0 = reg.entries.get(0) or Entry { name: "FB", value: -1 };
            let e1 = reg.entries.get(1) or Entry { name: "FB", value: -1 };
            println("e0=", e0.name, "/", to_string(e0.value));
            println("e1=", e1.name, "/", to_string(e1.value));
        }
    "#;
    let (stdout, status) = build_and_run("interleaved", src);
    assert!(status.success(), "non-zero (segv?): {:?}", status);
    assert!(stdout.contains("len=2"), "stdout: {:?}", stdout);
    assert!(stdout.contains("e0=direct/1"), "stdout: {:?}", stdout);
    assert!(stdout.contains("e1=via-fn/2"), "stdout: {:?}", stdout);
}

#[test]
fn fn_pointer_field_survives_subregion_destroy() {
    // Matches the pond/agent/tools::register_tool shape: the Entry
    // carries a fn-pointer field, the literal is built inside the
    // callee from individual args, and the receiver's vec is on a
    // sub-locus param. This is the exact shape the FRICTION
    // entry called out; it segfaulted before the fix.
    let src = r#"
        type ToolCall   { name: String; }
        type ToolResult { content: String; }
        type Entry      { name: String;
                          invoke_fn: fn(ToolCall) -> ToolResult; }

        @form(vec)
        locus EntryList {
            capacity { heap items of Entry; }
        }

        locus Registry {
            params { entries: EntryList = EntryList { }; }
        }

        fn register_tool(
            reg: Registry,
            n: String,
            invoke_fn: fn(ToolCall) -> ToolResult
        ) {
            reg.entries.push(Entry { name: n, invoke_fn: invoke_fn });
        }

        fn echo_invoke(c: ToolCall) -> ToolResult {
            return ToolResult { content: "echo:" + c.name };
        }

        fn main() {
            let reg = Registry { };
            register_tool(reg, "echo", echo_invoke);
            let e = reg.entries.get(0) or Entry { name: "FB", invoke_fn: echo_invoke };
            let r = e.invoke_fn(ToolCall { name: e.name });
            println("invoked: ", r.content);
        }
    "#;
    let (stdout, status) = build_and_run("fnptr_field", src);
    assert!(status.success(), "non-zero (segv?): {:?}", status);
    assert!(stdout.contains("invoked: echo:echo"), "stdout: {:?}", stdout);
}
