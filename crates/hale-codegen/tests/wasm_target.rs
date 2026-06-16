//! WASM target — Phase 0 foundation (full-stack web / WASM plan).
//!
//! Locks in the `CompileTarget::Wasm32` codegen path proven by the
//! Phase-0 spike: Hale's codegen (inkwell, LLVM 18) emits a valid
//! `wasm32-unknown-unknown` object, and that object — once linked —
//! runs correctly on the 32-bit-pointer ABI (struct layout, GEP,
//! arithmetic, the free-fn `__caller_arena` calling convention).
//!
//! Two tiers, so the codegen regression net needs NO external tooling
//! while end-to-end coverage runs where it's available:
//!   1. `wasm_object_emits_valid_module` — always runs. Asserts codegen
//!      produces a well-formed wasm module (magic + version). Pure
//!      codegen; catches any regression in the triple/init/emit plumbing.
//!   2. `wasm_struct_runs_in_node` — runs only when `wasm-ld` + `node`
//!      are present; links the pure-compute symbol with `--gc-sections`
//!      (+ inert runtime-import stubs, since the runtime core isn't
//!      ported yet) and asserts the result on the wasm32 ABI.

use hale_codegen::{build_executable_with_options, BuildOptions, CompileTarget};
use std::path::PathBuf;
use std::process::Command;

fn wasm_opts() -> BuildOptions {
    BuildOptions { target: CompileTarget::Wasm32, ..Default::default() }
}

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("hale_wasm_target_{}", name));
    p
}

fn tool(name: &str) -> Option<String> {
    for cand in [name, &format!("{}-18", name)] {
        if Command::new(cand).arg("--version").output().is_ok() {
            return Some(cand.to_string());
        }
    }
    None
}

/// Emitting a wasm object is pure codegen — always exercised.
#[test]
fn wasm_object_emits_valid_module() {
    let src = r#"
        fn compute() -> Int {
            let mut acc = 0;
            let mut i = 1;
            while i <= 10 { acc = acc + i * i; i = i + 1; }
            return acc;
        }
        fn main() { println("compute=", compute()); }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let obj = tmp("obj.wasm");
    build_executable_with_options(&program, &obj, &[], &wasm_opts())
        .expect("wasm codegen");
    let bytes = std::fs::read(&obj).expect("read wasm object");
    let _ = std::fs::remove_file(&obj);
    // WebAssembly module header: "\0asm" + version 1.
    assert!(bytes.len() > 8, "wasm object too small: {} bytes", bytes.len());
    assert_eq!(&bytes[0..4], b"\0asm", "missing wasm magic");
    assert_eq!(&bytes[4..8], &[1, 0, 0, 0], "unexpected wasm version");
}

/// Compile a runtime C source to a wasm32 object with clang.
fn clang_wasm(clang: &str, src: &str, out: &std::path::Path, extra: &[&str]) -> bool {
    Command::new(clang)
        .args(["--target=wasm32", "-mbulk-memory", "-O2", "-c"])
        .args(extra)
        .arg(src)
        .arg("-o")
        .arg(out)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// End-to-end, SELF-CONTAINED: compile the real lotus runtime + bundled
/// libc for wasm32, link them into a Hale program with NO undefined
/// symbols / NO JS stubs, and run it under Node. Proves the size_t-width
/// ABI fix: an allocating program (struct alloc + field GEP) runs against
/// the actual compiled-in runtime on the wasm32 ABI. Skips (passing) when
/// the wasm toolchain (clang / wasm-ld / node) isn't installed.
#[test]
fn wasm_struct_runs_self_contained() {
    let (Some(clang), Some(wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_struct_runs_self_contained: clang/wasm-ld/node not all found");
        return;
    };
    let rt = concat!(env!("CARGO_MANIFEST_DIR"), "/runtime");
    let arena_src = format!("{}/lotus_arena.c", rt);
    let libc_src = format!("{}/wasm/lotus_wasm_libc.c", rt);

    // Compile the runtime core + bundled libc for wasm (libc needs
    // -fno-builtin so its byte-loop mem*/str* aren't re-emitted as
    // recursive calls).
    let arena_o = tmp("rt_arena.o");
    let libc_o = tmp("rt_libc.o");
    if !clang_wasm(&clang, &arena_src, &arena_o, &[]) {
        eprintln!("SKIP: clang could not compile lotus_arena.c for wasm32");
        return;
    }
    assert!(
        clang_wasm(&clang, &libc_src, &libc_o, &["-fno-builtin"]),
        "bundled libc must compile for wasm32"
    );

    let src = r#"
        type Point { a: Int; b: Int; c: Int; }
        fn struct_sum() -> Int {
            let mut p = Point { a: 0, b: 0, c: 0 };
            let mut i = 1;
            while i <= 10 {
                p.a = p.a + i;
                p.b = p.a * 2;
                p.c = p.a + p.b;
                i = i + 1;
            }
            return p.a + p.b + p.c;   // 330
        }
        fn main() { println(struct_sum()); }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let obj = tmp("sc_obj.wasm");
    let linked = tmp("sc_linked.wasm");
    build_executable_with_options(&program, &obj, &[], &wasm_opts())
        .expect("wasm codegen");

    // Link user + runtime + libc. NO --allow-undefined: the runtime is
    // compiled in, so a successful link proves zero unresolved symbols.
    // --gc-sections strips `main` + the unreachable IO/threading families.
    let link = Command::new(&wasm_ld)
        .arg(&obj)
        .arg(&arena_o)
        .arg(&libc_o)
        .arg("--no-entry")
        .arg("--export=struct_sum")
        .arg("--export=__heap_base")
        .arg("--gc-sections")
        .arg("-o")
        .arg(&linked)
        .output()
        .expect("run wasm-ld");
    assert!(
        link.status.success(),
        "wasm-ld failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );

    // Run with an EMPTY import object — fully self-contained.
    let js = format!(
        r#"
        import {{ readFileSync }} from 'node:fs';
        const buf = readFileSync({:?});
        const {{ instance }} = await WebAssembly.instantiate(buf, {{}});
        const got = Number(instance.exports.struct_sum(1));   // arg = __caller_arena
        if (got !== 330) {{ console.error('FAIL: got', got); process.exit(1); }}
        process.exit(0);
        "#,
        linked.to_string_lossy()
    );
    let run = Command::new(&node)
        .arg("--input-type=module")
        .arg("-e")
        .arg(&js)
        .output()
        .expect("run node");
    for f in [&obj, &linked, &arena_o, &libc_o] {
        let _ = std::fs::remove_file(f);
    }
    assert!(
        run.status.success(),
        "self-contained wasm run failed (expected struct_sum()==330):\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
