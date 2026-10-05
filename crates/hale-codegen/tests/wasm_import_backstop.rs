//! The wasm import backstop (F.40 phase 3, P3 T7;
//! `notes/f40-capability-matrix.md` § 2.3).
//!
//! Every module the wasm tests build imports only what the generated
//! loader's writers supply (read from the loader's own source) plus the
//! program's declared `@ffi("js")` names: each file of this area that
//! builds for wasm32 holds its modules to `wasm_module::backstop`, and
//! [`every_wasm_build_in_the_area_is_held_to_the_backstop`] keeps it
//! so. An import outside that is a symbol that reached the link
//! undefined, which `--allow-undefined` kept and the loader runs as
//! `() => 0`. The backstop catches that and nothing more: it says
//! nothing about semantics, since an inline stub imports nothing (that
//! evidence is the lowering contracts' and probes', § 2.1).
//!
//! What the runtime cannot run on wasm32 is compiled out of it, so the
//! module does not import it. The names still imported by a path that
//! can run, or by generated code, are `wasm_module::KNOWN_OPEN`, each
//! asserted here to be still imported.
//!
//! Skipped, naming what is missing, when clang or wasm-ld is absent (no
//! module is built then); CI's wasm job installs both.

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::{build_executable_with_options, BuildOptions, CompileTarget};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/wasm_module.rs"]
mod wasm_module;

fn tool(name: &str) -> Option<String> {
    [name.to_string(), format!("{name}-18")].into_iter().find(|c| Command::new(c).arg("--version").output().is_ok())
}

fn toolchain(test: &str) -> bool {
    let missing: Vec<&str> = ["clang", "wasm-ld"].into_iter().filter(|t| tool(t).is_none()).collect();
    if !missing.is_empty() {
        eprintln!("SKIP {test}: {} not found", missing.join(", "));
    }
    missing.is_empty()
}

/// Build `src` for wasm32; the module and its loader beside it.
fn build(name: &str, src: &str) -> (hale_syntax::ast::Program, PathBuf) {
    let program = hale_syntax::parse_source(src).unwrap_or_else(|e| panic!("{name}: parse: {e:?}"));
    let wasm = harness::unique_bin(&format!("hale_backstop_{name}")).with_extension("wasm");
    let opts = BuildOptions { target: CompileTarget::Wasm32, ..build_opts::options() };
    build_executable_with_options(&program, &wasm, &[], &opts).unwrap_or_else(|e| panic!("{name}: wasm32 build: {e}"));
    (program, wasm)
}

fn cleanup(wasm: &std::path::Path) {
    let _ = std::fs::remove_file(wasm);
    let _ = std::fs::remove_file(wasm.with_extension("mjs"));
}

/// The control: a module importing a name outside the set fails the
/// backstop, which reports it by name. `@ffi("c")` with no `csrc` is
/// such a name on wasm32: nothing defines it, so it reaches the link
/// undefined. The same name declared `@ffi("js")` is the program's own
/// host import, and passes.
#[test]
fn a_module_importing_outside_the_set_is_reported_by_name() {
    if !toolchain("a_module_importing_outside_the_set_is_reported_by_name") {
        return;
    }
    let call = "fn main() {\n    println(\"n=\", hale_t7_unresolved(1));\n}\n";
    let (program, wasm) = build("control_c", &format!("@ffi(\"c\") fn hale_t7_unresolved(x: Int) -> Int;\n{call}"));
    let held = wasm_module::backstop("control_c", &program, &wasm);
    cleanup(&wasm);
    let err = held.expect_err("an @ffi(\"c\") name nothing defines must fail the backstop");
    assert!(err.contains("env.hale_t7_unresolved (func)"), "the report names the import:\n{err}");
    assert!(err.contains("<- main"), "the report names its caller:\n{err}");

    let (program, wasm) = build("control_js", &format!("@ffi(\"js\") fn hale_t7_unresolved(x: Int) -> Int;\n{call}"));
    let held = wasm_module::backstop("control_js", &program, &wasm);
    cleanup(&wasm);
    held.expect("a declared @ffi(\"js\") name is the program's own host import");
}

/// The loader's writer set is read from the loader the build wrote, not
/// spelled here: it parses to a non-empty set of identifiers.
#[test]
fn the_writer_set_is_read_from_the_loader() {
    if !toolchain("the_writer_set_is_read_from_the_loader") {
        return;
    }
    let (_, wasm) = build("writers", "fn main() {\n    println(\"hi\");\n}\n");
    let loader = std::fs::read_to_string(wasm.with_extension("mjs"));
    cleanup(&wasm);
    let writers = wasm_module::loader_writers(&loader.expect("the loader beside the module"))
        .expect("the loader has a `const writers` object");
    assert!(!writers.is_empty());
    assert!(writers.iter().all(|w| w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')), "{writers:?}");
}

/// The known-open table is current: one program reaching every path
/// that still imports a name outside the set (a locus's birth and
/// dissolve, a publish, readiness, a reclaim, a builder whose append
/// can violate) imports each of them, from the callers the table
/// allows. An entry no longer imported fails here, so the fix that
/// closes it removes the entry.
#[test]
fn every_known_open_import_is_still_imported() {
    if !toolchain("every_known_open_import_is_still_imported") {
        return;
    }
    let src = r#"
        type Ping { n: Int; }
        locus Receiver {
            bus { subscribe "ping" as on_ping of type Ping; }
            fn on_ping(p: Ping) { println("got ", p.n); }
        }
        locus Sender {
            bus { publish "ping" of type Ping; }
            birth() { "ping" <- Ping { n: 1 }; }
        }
        fn main() {
            Receiver { };
            Sender { };
            let b = std::bytes::BytesBuilder { };
            b.append_u32_le(7);
            println("len=", len(b.view()));
        }
    "#;
    let (program, wasm) = build("known_open", src);
    let bytes = std::fs::read(&wasm).expect("the module");
    let held = wasm_module::backstop("known_open", &program, &wasm);
    cleanup(&wasm);
    held.unwrap_or_else(|e| panic!("{e}"));
    let imports: Vec<String> =
        wasm_module::imports(&bytes).expect("a wasm module").into_iter().map(|i| i.name).collect();
    let gone: Vec<&str> =
        wasm_module::KNOWN_OPEN.iter().map(|k| k.name).filter(|n| !imports.iter().any(|i| i == n)).collect();
    assert!(
        gone.is_empty(),
        "known open, but no longer imported (remove the entry from KNOWN_OPEN in support/wasm_module.rs): \
         {gone:?}\nthe module imports: {imports:?}"
    );
}

/// Every file of the wasm area that builds for wasm32 holds the modules
/// it builds to the backstop, so "every module the wasm tests build" is
/// kept as files are added.
#[test]
fn every_wasm_build_in_the_area_is_held_to_the_backstop() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let root = std::fs::read_to_string(dir.join("wasm_ffi.rs")).expect("the area's root");
    let files: Vec<&str> = root
        .lines()
        .filter_map(|l| l.trim().strip_prefix("#[path = \"")?.strip_suffix("\"]"))
        .collect();
    assert!(files.contains(&"wasm_import_backstop.rs"), "the area's root lists its files: {files:?}");
    let bypass: Vec<&str> = files
        .iter()
        .copied()
        .filter(|f| {
            let text = std::fs::read_to_string(dir.join(f)).unwrap_or_default();
            text.contains("CompileTarget::Wasm32") && !text.contains("wasm_module::backstop(")
        })
        .collect();
    assert!(bypass.is_empty(), "these files build wasm32 modules without the import backstop: {bypass:?}");
}
