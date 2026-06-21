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

/// `@ffi("js")` host imports — the games-stdlib enabler. A Hale program
/// declares a host import; the wasm build turns it into an `env` import
/// the JS loader provides (here the built-in `console_log`). Proves the
/// whole chain: parse `@ffi("js")` -> lower to a wasm import -> loader
/// wires it -> visible output.
#[test]
fn wasm_ffi_js_host_import() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_ffi_js_host_import: clang/wasm-ld/node not all found");
        return;
    };
    let src = r#"
        target wasm { }
        @ffi("js") fn console_log(msg: String);
        fn main() { console_log("FFIJS_OK"); }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse @ffi(\"js\")");
    let wasm = tmp("ffijs.wasm");
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    let run = Command::new(&node).arg(&loader).output().expect("run node loader");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    assert!(
        run.status.success() && stdout.contains("FFIJS_OK"),
        "@ffi(\"js\") host import should print via the loader:\nstdout: {}\nstderr: {}",
        stdout,
        String::from_utf8_lossy(&run.stderr)
    );
}

/// `String + Int` (and `Int + String`) must format the integer's
/// decimal digits under wasm32, matching native. Regression: the
/// wasm libc shim's `snprintf` was a no-op stub, so `lotus_str_from_int`
/// (the to_string/`+` Int path) emitted nothing — every interpolated
/// Int vanished (`"n=" + 5` → `"n="`), while native was correct. The
/// shim now carries a real minimal `(v)snprintf`. A Float arg
/// (`"f=" + 3.5`) goes through the same `%g` path and is checked too.
#[test]
fn wasm_string_int_concat_formats() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_string_int_concat_formats: toolchain missing");
        return;
    };
    let src = r#"
        target wasm { }
        @ffi("js") fn console_log(msg: String);
        fn main() {
            console_log("n=" + 5);
            console_log("neg=" + (0 - 42));
            console_log(7 + "=seven");
            console_log("{\"kind\":" + 3 + ",\"ref\":" + 11 + "}");
            console_log("f=" + 3.5);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let wasm = tmp("concat.wasm");
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    let run = Command::new(&node).arg(&loader).output().expect("run node loader");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    for expect in [
        "n=5",
        "neg=-42",
        "7=seven",
        "{\"kind\":3,\"ref\":11}",
        "f=3.5",
    ] {
        assert!(
            stdout.contains(expect),
            "wasm String+Int concat dropped a value (expected line {:?}):\n\
             stdout: {}\nstderr: {}",
            expect,
            stdout,
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

/// `@ffi("js")` marshals Hale `Int` as a JS `number` (f64), not a
/// `BigInt` (i64). Before, a host handler `(n) => ...` received a
/// BigInt and had to `Number(n)` everywhere — a recurring sharp edge.
/// Now `typeof n === "number"` on the way out, and an `Int`-returning
/// host import accepts a plain JS number on the way back (f64→i64).
/// `@ffi("c")` keeps i64 (those are linked C symbols). 2^53 caveat.
#[test]
fn wasm_ffi_js_int_marshals_as_number() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_ffi_js_int_marshals_as_number: toolchain missing");
        return;
    };
    let src = r#"
        target wasm { }
        @ffi("js") fn want_number(n: Int);
        @ffi("js") fn next_id() -> Int;
        @export locus M {
            params { _x: Int = 0; }
            birth() { }
            fn go() {
                want_number(42);
                want_number(0 - 7);
                let id = next_id();
                want_number(id + 1);
            }
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let wasm = tmp("ffijs_int.wasm");
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    let harness = wasm.with_extension("harness.mjs");
    let loader_name = loader.file_name().unwrap().to_str().unwrap();
    std::fs::write(
        &harness,
        format!(
            r#"import {{ run }} from "./{loader_name}";
const seen = [];
const inst = await run(() => ({{
  want_number: (n) => seen.push([typeof n, Number(n)]),
  next_id: () => 1000,
}}));
inst.exports.go();
const ok = seen.length === 3 && seen.every(([t]) => t === "number")
  && seen[0][1] === 42 && seen[1][1] === -7 && seen[2][1] === 1001;
console.log(ok ? "FFIJS_NUMBER_OK" : ("FFIJS_NUMBER_FAIL:" + JSON.stringify(seen)));
"#
        ),
    )
    .expect("write harness");
    let run = Command::new(&node).arg(&harness).output().expect("run node harness");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    let _ = std::fs::remove_file(&harness);
    assert!(
        run.status.success() && stdout.contains("FFIJS_NUMBER_OK"),
        "@ffi(\"js\") Int should cross as a JS number both ways:\n\
         stdout: {}\nstderr: {}",
        stdout,
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Decimal (i128 mantissa, scale 9) must format and compute correctly
/// under wasm32. Regression: clang lowers i128 multiply/divide/→double
/// to compiler-rt libcalls (`__multi3` / `__udivti3` / `__umodti3` /
/// `__floatuntidf`), Ubuntu's clang ships no wasm32 builtins archive, so
/// `--allow-undefined` turned them into imports the loader stubbed to 0 —
/// every Decimal came out garbage. The bundled wasm libc now defines
/// those builtins (64-bit-only bodies). Checks formatting, negatives,
/// `*`, `/`, `+`, and `std::decimal::to_float`.
#[test]
fn wasm_decimal_i128_builtins() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_decimal_i128_builtins: toolchain missing");
        return;
    };
    let src = r#"
        target wasm { }
        @ffi("js") fn console_log(msg: String);
        fn main() {
            console_log("a=" + 5.0d);
            console_log("b=" + 19.99d);
            console_log("neg=" + (0.0d - 2.5d));
            console_log("mul=" + (19.99d * 3.0d));
            console_log("div=" + (10.0d / 4.0d));
            console_log("tf=" + std::decimal::to_float(19.99d));
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let wasm = tmp("decimal.wasm");
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    let run = Command::new(&node).arg(&loader).output().expect("run node loader");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    for expect in ["a=5", "b=19.99", "neg=-2.5", "mul=59.97", "div=2.5", "tf=19.99"] {
        assert!(
            stdout.contains(expect),
            "wasm Decimal wrong (expected line {:?}):\nstdout: {}\nstderr: {}",
            expect,
            stdout,
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

/// WASM entry-inversion: `@export fn` + the synthesized `_hale_start` +
/// the runtime state cell give a wasm module PERSISTENT state across
/// separate host calls. A program with only `@export` fns (no `fn main`)
/// builds; the loader auto-calls `_hale_start` (persistent arena) and the
/// host then drives the exports. Here `bump()` increments a counter
/// packed into a Bytes blob parked in the state cell, and `get()` reads
/// it back — three separate `bump()` calls must accumulate to 3, proving
/// the state survived across calls (not reset per call like `main`).
#[test]
fn wasm_export_entry_inversion_persists_state() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_export_entry_inversion_persists_state: toolchain missing");
        return;
    };
    let src = r#"
        target wasm { }
        @ffi("c") fn lotus_wasm_state_set(b: Bytes);
        @ffi("c") fn lotus_wasm_state_get() -> Bytes;
        fn current() -> Int {
            let s = lotus_wasm_state_get();
            if len(s) >= 4 { return std::bytes::read_u32_le(s, 0) or 0; }
            return 0;
        }
        @export fn bump() {
            let n = current() + 1;
            let b = std::bytes::BytesBuilder { };
            b.append_u32_le(n);
            lotus_wasm_state_set(b.snapshot());
        }
        @export fn get() -> Int { return current(); }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse @export program");
    let wasm = tmp("export_ei.wasm");
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    // Drive the exports from a harness: _hale_start runs at load, then
    // three bumps, then read the accumulated counter.
    let harness = wasm.with_extension("harness.mjs");
    let loader_name = loader.file_name().unwrap().to_str().unwrap();
    std::fs::write(
        &harness,
        format!(
            r#"import {{ run }} from "./{loader_name}";
const inst = await run(() => ({{}}));
if (inst.exports.get() !== 0n) {{ console.log("BAD_INIT"); process.exit(1); }}
inst.exports.bump(); inst.exports.bump(); inst.exports.bump();
const v = inst.exports.get();
console.log(v === 3n ? "EXPORT_STATE_OK" : ("EXPORT_STATE_FAIL:" + v));
"#
        ),
    )
    .expect("write harness");
    let run = Command::new(&node).arg(&harness).output().expect("run node harness");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    let _ = std::fs::remove_file(&harness);
    assert!(
        run.status.success() && stdout.contains("EXPORT_STATE_OK"),
        "@export persistent state should accumulate to 3:\nstdout: {}\nstderr: {}",
        stdout,
        String::from_utf8_lossy(&run.stderr)
    );
}

/// WASM entry-inversion, persistent-locus model: `@export locus L`
/// designates a singleton instantiated once by `_hale_start` and never
/// dissolved; its `fn` methods become exports, and state lives in the
/// locus's own fields (not packed Bytes). `bump()` increments a field,
/// `get()` reads it — three `bump()` calls accumulate to 3, proving the
/// singleton (and its `self.n`) persisted across host calls.
#[test]
fn wasm_export_locus_persists_field_state() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_export_locus_persists_field_state: toolchain missing");
        return;
    };
    let src = r#"
        target wasm { }
        @export locus Counter {
            params { n: Int = 0; }
            birth() { }
            fn bump() { self.n = self.n + 1; }
            fn get() -> Int { return self.n; }
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse @export locus");
    let wasm = tmp("export_locus.wasm");
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    let harness = wasm.with_extension("harness.mjs");
    let loader_name = loader.file_name().unwrap().to_str().unwrap();
    std::fs::write(
        &harness,
        format!(
            r#"import {{ run }} from "./{loader_name}";
const inst = await run(() => ({{}}));   // _hale_start instantiates Counter
if (inst.exports.get() !== 0n) {{ console.log("BAD_INIT"); process.exit(1); }}
inst.exports.bump(); inst.exports.bump(); inst.exports.bump();
const v = inst.exports.get();
console.log(v === 3n ? "LOCUS_STATE_OK" : ("LOCUS_STATE_FAIL:" + v));
"#
        ),
    )
    .expect("write harness");
    let run = Command::new(&node).arg(&harness).output().expect("run node harness");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    let _ = std::fs::remove_file(&harness);
    assert!(
        run.status.success() && stdout.contains("LOCUS_STATE_OK"),
        "@export locus field state should accumulate to 3:\nstdout: {}\nstderr: {}",
        stdout,
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Regression: an `@export locus` with MULTIPLE mixed-type fixed-array
/// fields must not alias under wasm. The singleton is instantiated in
/// `_hale_start` but outlives it, so its struct must be heap-allocated
/// in the persistent program arena — a stack alloca dangled, and O2's
/// dead-store elimination then dropped the array-pointer field stores,
/// making the Float arrays read the Int array's bits. Here `fill()`
/// writes distinct values to three Float arrays + one Int array in a
/// loop; reading them back across a separate call must return exactly
/// what was written.
#[test]
fn wasm_export_locus_multiple_array_fields_no_alias() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_export_locus_multiple_array_fields_no_alias: toolchain missing");
        return;
    };
    let src = r#"
        target wasm { }
        @export locus T {
            params {
                ax:[Float;8]=[0.0;8]; ay:[Float;8]=[0.0;8];
                az:[Float;8]=[0.0;8]; owner:[Int;8]=[0;8];
            }
            birth() { }
            fn fill() {
                let mut i = 0;
                while i < 3 {
                    self.ax[i] = 100.0 + std::math::int_to_float(i);
                    self.ay[i] = 200.0 + std::math::int_to_float(i);
                    self.az[i] = 300.0 + std::math::int_to_float(i);
                    self.owner[i] = i * 10;
                    i = i + 1;
                }
            }
            fn rx(k:Int)->Float{return self.ax[k];}
            fn ry(k:Int)->Float{return self.ay[k];}
            fn rz(k:Int)->Float{return self.az[k];}
            fn ro(k:Int)->Int{return self.owner[k];}
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let wasm = tmp("export_locus_arrays.wasm");
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    let harness = wasm.with_extension("harness.mjs");
    let loader_name = loader.file_name().unwrap().to_str().unwrap();
    std::fs::write(
        &harness,
        format!(
            r#"import {{ run }} from "./{loader_name}";
const inst = await run(() => ({{}}));
inst.exports.fill();
let ok = true;
for (let k = 0; k < 3; k++) {{
  const x = inst.exports.rx(BigInt(k)), y = inst.exports.ry(BigInt(k));
  const z = inst.exports.rz(BigInt(k)), o = inst.exports.ro(BigInt(k));
  if (x !== 100+k || y !== 200+k || z !== 300+k || o !== BigInt(k*10)) {{
    ok = false; console.log(`MISMATCH k=${{k}}: x=${{x}} y=${{y}} z=${{z}} o=${{o}}`);
  }}
}}
console.log(ok ? "ARRAYS_OK" : "ARRAYS_FAIL");
"#
        ),
    )
    .expect("write harness");
    let run = Command::new(&node).arg(&harness).output().expect("run node harness");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    let _ = std::fs::remove_file(&harness);
    assert!(
        run.status.success() && stdout.contains("ARRAYS_OK"),
        "multiple mixed-type array fields on an @export locus must not alias \
         under wasm:\nstdout: {}\nstderr: {}",
        stdout,
        String::from_utf8_lossy(&run.stderr)
    );
}

/// `std::math::round` / `trunc` on wasm32 must lower host-free —
/// pure LLVM `fptosi` (+ compare/select shift for round), NOT a
/// libm `round`/`trunc` libcall that would become an undefined
/// `env` import (the way `sin`/`cos` need host glue). The harness
/// instantiates with an EMPTY glue object: if the module imported
/// `round`/`trunc`, instantiation would throw. round(3.7)=4,
/// round(-2.5)=-3 (half away from zero), trunc(9.9)=9.
#[test]
fn wasm_round_trunc_host_free() {
    let (Some(_clang), Some(_wasm_ld), Some(node)) =
        (tool("clang"), tool("wasm-ld"), tool("node"))
    else {
        eprintln!("SKIP wasm_round_trunc_host_free: toolchain missing");
        return;
    };
    let src = r#"
        target wasm { }
        @export locus M {
            params { _x: Int = 0; }
            birth() { }
            fn r_pos() -> Int { return std::math::round(3.7); }
            fn r_neg() -> Int { return std::math::round(0.0 - 2.5); }
            fn t() -> Int { return std::math::trunc(9.9); }
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let wasm = tmp("round_trunc.wasm");
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    let harness = wasm.with_extension("harness.mjs");
    let loader_name = loader.file_name().unwrap().to_str().unwrap();
    std::fs::write(
        &harness,
        format!(
            r#"import {{ run }} from "./{loader_name}";
const inst = await run(() => ({{}}));   // EMPTY glue — no host import allowed
const a = inst.exports.r_pos(), b = inst.exports.r_neg(), c = inst.exports.t();
console.log((a === 4n && b === -3n && c === 9n)
  ? "ROUND_TRUNC_OK" : `ROUND_TRUNC_FAIL:${{a}},${{b}},${{c}}`);
"#
        ),
    )
    .expect("write harness");
    let run = Command::new(&node).arg(&harness).output().expect("run node harness");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    let _ = std::fs::remove_file(&harness);
    assert!(
        run.status.success() && stdout.contains("ROUND_TRUNC_OK"),
        "round/trunc must run host-free on wasm32:\nstdout: {}\nstderr: {}",
        stdout,
        String::from_utf8_lossy(&run.stderr)
    );
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
        fn main() { println("STRUCTSUM=", struct_sum()); }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let wasm = tmp("sc.wasm");
    // The wasm path compiles + links the runtime AND emits a `.mjs` loader.
    build_executable_with_options(&program, &wasm, &[], &wasm_opts())
        .expect("wasm codegen + link");
    let loader = wasm.with_extension("mjs");
    assert!(wasm.exists(), "link_wasm should produce a .wasm");
    assert!(loader.exists(), "link_wasm should emit a .mjs loader beside the .wasm");

    // Run the program through the GENERATED loader (the real CLI artifact):
    // `node sc.mjs` instantiates the module, wires console output, and runs
    // `main`, which prints the struct-computed value via the loader's
    // mini-printf (%lld vararg). Asserts the formatted value reaches stdout.
    let run = Command::new(&node).arg(&loader).output().expect("run node loader");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    assert!(
        run.status.success() && stdout.contains("STRUCTSUM=330"),
        "self-contained wasm run via the generated loader should print STRUCTSUM=330:\n\
         stdout: {}\nstderr: {}",
        stdout,
        String::from_utf8_lossy(&run.stderr)
    );
}
