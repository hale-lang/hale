//! The portable subset (F.40 phase 3, P3 3 of 3;
//! `notes/f40-capability-matrix.md` § 5, item 4): the programs whose
//! stdout agrees byte for byte between the native binary and the wasm32
//! module run under node.
//!
//! Each program is built for both targets and run on both, and the two
//! outputs are compared. A `fn main` program is built for wasm32 as it
//! is (the loader runs `main`), or, where the design names it so,
//! through `--wrap-main`'s rewrite (the playground's examples). The
//! comparison proves agreement for these programs and nothing outside
//! them: no namespace or operation is `Lower` on wasm32 because a
//! program here happens to call it (that is the lowering contracts' and
//! probes' job). Skipped, naming what is missing, when node, clang or
//! wasm-ld is absent; CI installs all three.

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::{build_executable_with_options, BuildOptions, CompileTarget};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn tool(name: &str) -> Option<String> {
    [name.to_string(), format!("{name}-18")].into_iter().find(|c| Command::new(c).arg("--version").output().is_ok())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// How the wasm32 build reaches `main`.
#[derive(Clone, Copy, PartialEq)]
enum Entry {
    /// The module's `main`, which the loader runs.
    Main,
    /// `--wrap-main`: `fn main` becomes the `@export` entry's body.
    Wrapped,
}

/// The design's programs: `crates/hale-codegen/tests/wasm_target.rs`'s
/// struct sum, sum of squares and wrapped main, its string and decimal
/// formatting with `console_log` replaced by `println`,
/// `crates/hale-cli/tests/wasm_link_is_quiet.rs`'s byte and string
/// views, and `crates/hale-types/tests/wasm_target_gating.rs`'s
/// portable stdlib program.
const PROGRAMS: &[(&str, Entry, &str)] = &[
    (
        "struct_sum",
        Entry::Main,
        r#"
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
            return p.a + p.b + p.c;
        }
        fn main() { println("STRUCTSUM=", struct_sum()); }
        "#,
    ),
    (
        "sum_of_squares",
        Entry::Main,
        r#"
        fn compute() -> Int {
            let mut acc = 0;
            let mut i = 1;
            while i <= 10 { acc = acc + i * i; i = i + 1; }
            return acc;
        }
        fn main() { println("compute=", compute()); }
        "#,
    ),
    (
        "wrapped_main",
        Entry::Wrapped,
        r#"
        fn main() {
            let msg: String = "wrapped-main-ran";
            println(msg);
        }
        "#,
    ),
    (
        "string_int_concat",
        Entry::Main,
        r#"
        fn main() {
            println("n=" + 5);
            println("neg=" + (0 - 42));
            println(7 + "=seven");
            println("{\"kind\":" + 3 + ",\"ref\":" + 11 + "}");
            println("f=" + 3.5);
        }
        "#,
    ),
    (
        "decimal",
        Entry::Main,
        r#"
        fn main() {
            println("a=" + 5.0d);
            println("b=" + 19.99d);
            println("neg=" + (0.0d - 2.5d));
            println("mul=" + (19.99d * 3.0d));
            println("div=" + (10.0d / 4.0d));
            println("tf=" + std::decimal::to_float(19.99d));
        }
        "#,
    ),
    (
        "byte_and_string_views",
        Entry::Main,
        r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            let v = b.view();
            println("len=", len(v));
            println("b0=", std::bytes::at(v, 0) or -1);
            let t = b.text_view();
            println(t);
        }
        "#,
    ),
    (
        "portable_stdlib",
        Entry::Main,
        r#"
        fn main() {
            let n = std::str::parse_int("42") or 0;
            let b = std::bytes::BytesBuilder { };
            b.append_u32_le(n);
            println("n=", n);
        }
        "#,
    ),
];

/// The playground's examples, built for wasm32 through `--wrap-main`
/// as `play/build.sh` builds them.
const PLAY: &[&str] = &["collections", "decimal", "closure", "enums", "fallible", "jobqueue"];

fn run_native(name: &str, program: &hale_syntax::ast::Program) -> Result<String, String> {
    let bin = harness::unique_bin(&format!("hale_portable_{name}_native"));
    let opts = BuildOptions { target: CompileTarget::Native, ..build_opts::options() };
    build_executable_with_options(program, &bin, &[], &opts).map_err(|e| format!("native build: {e}"))?;
    let out = Command::new(&bin).output().map_err(|e| format!("run: {e}"))?;
    let _ = std::fs::remove_file(&bin);
    if !out.status.success() {
        return Err(format!("native run exited {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_wasm(name: &str, program: &hale_syntax::ast::Program, entry: Entry, node: &str) -> Result<String, String> {
    let mut program = program.clone();
    if entry == Entry::Wrapped && !hale_syntax::desugar::wrap_main_as_wasm_export(&mut program) {
        return Err("--wrap-main found no `fn main` to wrap".to_string());
    }
    let wasm = harness::unique_bin(&format!("hale_portable_{name}")).with_extension("wasm");
    let opts = BuildOptions { target: CompileTarget::Wasm32, ..build_opts::options() };
    build_executable_with_options(&program, &wasm, &[], &opts).map_err(|e| format!("wasm32 build: {e}"))?;
    let loader = wasm.with_extension("mjs");
    let out = Command::new(node).arg(&loader).output().map_err(|e| format!("node: {e}"))?;
    let _ = std::fs::remove_file(&wasm);
    let _ = std::fs::remove_file(&loader);
    if !out.status.success() {
        return Err(format!("node exited {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn the_portable_subset_prints_the_same_bytes_natively_and_under_node() {
    let missing: Vec<&str> = ["clang", "wasm-ld", "node"].into_iter().filter(|t| tool(t).is_none()).collect();
    if !missing.is_empty() {
        eprintln!("SKIP the portable subset: {} not found", missing.join(", "));
        return;
    }
    let node = tool("node").expect("checked above");
    let mut cases: Vec<(String, Entry, String)> =
        PROGRAMS.iter().map(|(n, e, s)| (n.to_string(), *e, s.to_string())).collect();
    for name in PLAY {
        let path = repo_root().join(format!("play/examples/{name}.hl"));
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        cases.push((format!("play/{name}"), Entry::Wrapped, src));
    }
    let failures: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = cases
            .iter()
            .map(|(name, entry, src)| {
                let node = &node;
                s.spawn(move || {
                    let program = hale_syntax::parse_source(src).map_err(|e| format!("{name}: parse: {e:?}"))?;
                    let tag = name.replace('/', "_");
                    let native = run_native(&tag, &program).map_err(|e| format!("{name}: {e}"))?;
                    let wasm = run_wasm(&tag, &program, *entry, node).map_err(|e| format!("{name}: {e}"))?;
                    if native.is_empty() {
                        return Err(format!("{name}: printed nothing"));
                    }
                    if native != wasm {
                        return Err(format!("{name}: the outputs differ\n--- native\n{native}--- wasm32 under node\n{wasm}"));
                    }
                    Ok(())
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().expect("a case thread").err()).collect()
    });
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
