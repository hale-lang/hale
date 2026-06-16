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

/// End-to-end: link the pure-compute symbol and run it under Node on the
/// wasm32 ABI. Skips (passing) when wasm-ld / node aren't installed.
#[test]
fn wasm_struct_runs_in_node() {
    let (Some(wasm_ld), Some(node)) = (tool("wasm-ld"), tool("node")) else {
        eprintln!("SKIP wasm_struct_runs_in_node: wasm-ld and/or node not found");
        return;
    };

    // A struct exercises alloc + field GEP read/write on wasm32.
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
    let obj = tmp("struct_obj.wasm");
    let linked = tmp("struct_linked.wasm");
    build_executable_with_options(&program, &obj, &[], &wasm_opts())
        .expect("wasm codegen");

    // Link only `struct_sum`; --gc-sections strips `main` + stdlib, and
    // --allow-undefined turns the (not-yet-ported) runtime scratch-arena
    // calls into imports the harness stubs out.
    let link = Command::new(&wasm_ld)
        .arg(&obj)
        .arg("--no-entry")
        .arg("--export=struct_sum")
        .arg("--export=__heap_base")
        .arg("--gc-sections")
        .arg("--allow-undefined")
        .arg("-o")
        .arg(&linked)
        .output()
        .expect("run wasm-ld");
    assert!(
        link.status.success(),
        "wasm-ld failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );

    // Node harness: inert runtime stubs + a JS bump-allocator over
    // __heap_base back the scratch arena; Int returns surface as BigInt.
    let js = format!(
        r#"
        import {{ readFileSync }} from 'node:fs';
        const buf = readFileSync({:?});
        let inst = null, bump = 0;
        const num = x => typeof x === 'bigint' ? Number(x) : x;
        const env = {{
          lotus_arena_create_subregion: () => 1,
          lotus_arena_destroy: () => {{}},
          lotus_bus_queue_drain: () => {{}},
          lotus_arena_alloc: (_a, size, align) => {{
            size = num(size); align = num(align) || 8;
            const mem = inst.exports.memory;
            bump = (bump + align - 1) & ~(align - 1);
            const p = bump; bump += size;
            if (bump > mem.buffer.byteLength)
              mem.grow(Math.ceil((bump - mem.buffer.byteLength) / 65536) + 1);
            return p;
          }},
        }};
        const {{ instance }} = await WebAssembly.instantiate(buf, {{ env }});
        inst = instance;
        const hb = instance.exports.__heap_base;
        bump = hb ? Number(hb.value ?? hb) : 1024;
        const got = Number(instance.exports.struct_sum(1));
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
    let _ = std::fs::remove_file(&obj);
    let _ = std::fs::remove_file(&linked);
    assert!(
        run.status.success(),
        "node run failed (expected struct_sum()==330):\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
