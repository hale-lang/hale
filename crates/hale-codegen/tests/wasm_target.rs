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
/// The wasm build path produces a valid, well-formed wasm module via
/// codegen's link_wasm (compile runtime + wasm-ld). Gated on the wasm
/// toolchain (link_wasm shells out to clang + wasm-ld).
#[test]
fn wasm_build_emits_valid_module() {
    if tool("clang").is_none() || tool("wasm-ld").is_none() {
        eprintln!("SKIP wasm_build_emits_valid_module: clang/wasm-ld not found");
        return;
    }
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
        .expect("wasm codegen + link");
    let bytes = std::fs::read(&obj).expect("read wasm module");
    let _ = std::fs::remove_file(&obj);
    // WebAssembly module header: "\0asm" + version 1.
    assert!(bytes.len() > 8, "wasm module too small: {} bytes", bytes.len());
    assert_eq!(&bytes[0..4], b"\0asm", "missing wasm magic");
    assert_eq!(&bytes[4..8], &[1, 0, 0, 0], "unexpected wasm version");
    // Entry-inversion regression guard: the wasm `main` must NOT drag in
    // the cross-process transport/thread startup. Import names live as
    // literal strings in the import section, so a byte search suffices.
    for forbidden in ["socket", "bind", "connect", "pthread_create", "lotus_bus_load_config"] {
        assert!(
            !bytes.windows(forbidden.len()).any(|w| w == forbidden.as_bytes()),
            "entry inversion regressed: wasm module imports `{}` (the socket/thread \
             startup should be gated out of `main` on wasm)",
            forbidden
        );
    }
}

/// End-to-end via the real CLI link path: `build_executable_with_options`
/// with `CompileTarget::Wasm32` now COMPILES + LINKS the self-contained
/// wasm runtime (arena core + bundled libc) into a runnable `.wasm` (the
/// codegen `link_wasm` step — no manual wasm-ld). The module exports
/// `main`; `--gc-sections` strips the unreachable stdlib. We run `main`
/// and capture its `println` output, asserting the allocating program
/// (struct alloc + field GEP) computes 330 on the wasm32 ABI.
///
/// `main`'s startup still references libc-output + (gated-out) IO/thread
/// syscalls as host imports — those become the JS host surface a proper
/// loader will provide; here we stub them (no-ops; printf captures its
/// format string). Skips (passing) when clang/wasm-ld/node aren't found
/// (link_wasm shells out to clang + wasm-ld).
#[test]
fn wasm_struct_runs_self_contained() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_struct_runs_self_contained: clang/wasm-ld/node not all found");
        return;
    };

    // The struct computation runs in wasm; correctness is decided there
    // and signalled with a LITERAL marker string, so capturing printf's
    // format pointer (println formats Int args as varargs we can't read
    // from JS) reliably carries the verdict.
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
        fn main() {
            if struct_sum() == 330 { println("STRUCTSUM_OK"); }
            else { println("STRUCTSUM_BAD"); }
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let linked = tmp("sc.wasm");
    // The wasm path now compiles + links the runtime in codegen (link_wasm).
    build_executable_with_options(&program, &linked, &[], &wasm_opts())
        .expect("wasm codegen + link");
    assert!(linked.exists(), "link_wasm should produce a .wasm at the output path");

    // Run `main`, dynamically stubbing every host import the module
    // declares; printf captures its (literal) format string.
    let js = format!(
        r#"
        import {{ readFileSync }} from 'node:fs';
        const buf = readFileSync({:?});
        const mod = await WebAssembly.compile(buf);
        const holder = {{ inst: null }};
        let out = "";
        const cstr = (p) => {{
            const m = new Uint8Array(holder.inst.exports.memory.buffer);
            let e = p; while (m[e]) e++;
            return new TextDecoder().decode(m.subarray(p, e));
        }};
        const env = {{}};
        const captures = ['puts', 'printf', 'fputs'];   // println(literal) -> puts
        for (const im of WebAssembly.Module.imports(mod)) {{
            if (im.kind !== 'function') continue;
            env[im.name] = captures.includes(im.name)
                ? (p) => {{ out += cstr(p); return 0; }}
                : () => 0;
        }}
        // instantiate(Module, imports) resolves to the Instance directly.
        const instance = await WebAssembly.instantiate(mod, {{ env }});
        holder.inst = instance;
        instance.exports.main(0, 0);
        if (!out.includes('STRUCTSUM_OK')) {{ console.error('FAIL: output was', JSON.stringify(out)); process.exit(1); }}
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
    let _ = std::fs::remove_file(&linked);
    assert!(
        run.status.success(),
        "self-contained wasm run failed (expected main to print 330):\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
